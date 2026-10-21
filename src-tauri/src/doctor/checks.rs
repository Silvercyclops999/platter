//! What one album artist's albums and songs say. Read-only: everything
//! comes from the library cache, the file tags and the release status rule.
//!
//! Each check is plain about what it saw, so "Why?" can show it, and errs
//! toward saying nothing: unreleased songs never match anything (the
//! identity rule), editions and builds are never "split", and an
//! unreleased album is never one that "should be matched".

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use super::{Finding, Fix, Kind, OtherAlbum, TagFix, TagRow};
use crate::archive::status;
use crate::db::release::{self, Evidence, ReleaseStatus};
use crate::error::AppResult;
use crate::identity::fold::{fold_album, fold_artist, fold_text, fold_title, is_generic};
use crate::identity::{Context, MatchKind, SongKey, Tier, compare};
use crate::tags::CACHE_VERSION;

/// Raised when the checks change, so every artist is checked again.
pub const CHECKS_VERSION: u32 = 3;

/// A lossy copy at most this bitrate (kbps), and this much below the other
/// copy, counts as low quality next to another lossy copy.
const LOSSY_LOW_KBPS: u32 = 256;
const LOSSY_RATIO: f64 = 0.7;

#[derive(Debug, Clone)]
pub struct AlbumRow {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    pub year: Option<i32>,
    pub mbid: Option<String>,
    pub edition: Option<String>,
    pub version: Option<String>,
    pub group: Option<String>,
    pub status: ReleaseStatus,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone)]
pub struct SongRow {
    pub id: String,
    pub title: String,
    pub album_id: String,
    pub album_title: String,
    pub artist_title: String,
    pub track_artist: Option<String>,
    pub disc: i64,
    pub number: i64,
    pub duration_ms: i64,
    pub codec: Option<String>,
    pub bitrate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub mbid: Option<String>,
    /// Its file's tags were read by a build that keeps album, album artist and year.
    pub tags_read: bool,
    pub isrc: Option<String>,
    pub recording_mbid: Option<String>,
    pub tag_album: Option<String>,
    pub tag_album_artist: Option<String>,
    pub tag_year: Option<i32>,
    pub unreleased: bool,
    /// The folder its file is in, on the server (a disc folder counts as
    /// its album's folder).
    pub folder: Option<String>,
}

/// An album with the same title under another album artist (for the album
/// artist check).
#[derive(Debug, Clone)]
pub struct Elsewhere {
    pub album_id: String,
    pub album: String,
    pub artist: String,
    pub thumb: Option<String>,
}

/// Everything the checks look at for one album artist.
#[derive(Debug, Clone, Default)]
pub struct ArtistData {
    pub albums: Vec<AlbumRow>,
    pub songs: Vec<SongRow>,
    /// By album id: albums of the same name by other artists.
    pub elsewhere: HashMap<String, Vec<Elsewhere>>,
    /// Artist lines that credit several artists ("Ana, Bo"), split into
    /// single names with the library's artists kept whole.
    pub credits: HashMap<String, Vec<String>>,
}

/// The artist line a song credits: its own artist, else its album's.
fn credit_line(s: &SongRow) -> &str {
    s.track_artist.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&s.artist_title)
}

/// Load one album artist's albums and songs.
pub fn load(c: &Connection, artist_id: &str) -> AppResult<ArtistData> {
    let assessed: HashMap<String, (ReleaseStatus, Vec<Evidence>)> =
        status::load_facts(c, "WHERE al.artist_id = ?1", "WHERE t.artist_id = ?1", &[&artist_id])?
            .into_iter()
            .map(|(id, _, f)| (id, release::assess(&f)))
            .collect();
    let mut albums = Vec::new();
    {
        let mut st = c.prepare_cached(
            "SELECT al.id, al.title, al.artist_title, al.thumb, al.year, al.mbid, at.edition, at.version, at.grp
             FROM albums al LEFT JOIN album_tags at ON at.album_id = al.id
             WHERE al.artist_id = ?1 ORDER BY al.id",
        )?;
        let mut rows = st.query([artist_id])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let (status, evidence) = assessed.get(&id).cloned().unwrap_or((ReleaseStatus::Released, Vec::new()));
            albums.push(AlbumRow {
                title: r.get(1)?,
                artist: r.get(2)?,
                thumb: r.get(3)?,
                year: r.get(4)?,
                mbid: r.get(5)?,
                edition: r.get(6)?,
                version: r.get(7)?,
                group: r.get(8)?,
                status,
                evidence,
                id,
            });
        }
    }
    let unreleased_album: HashMap<&str, bool> = albums.iter().map(|a| (a.id.as_str(), a.status.is_unreleased())).collect();
    let mut songs = Vec::new();
    {
        let mut st = c.prepare_cached(
            "SELECT t.id, t.title, t.album_id, t.album_title, t.artist_title, t.track_artist, t.disc_number, t.track_number,
                    t.duration_ms, t.codec, t.bitrate, t.bit_depth, t.sample_rate, t.mbid,
                    f.status, f.ver, f.isrc, f.recording_mbid, f.album, f.album_artist, f.year, v.status, t.file
             FROM tracks t
             LEFT JOIN file_tags f ON f.part_key = t.part_key
             LEFT JOIN versions v ON v.track_id = t.id
             WHERE t.artist_id = ?1
             ORDER BY t.album_id, t.disc_number, t.track_number, t.id",
        )?;
        let mut rows = st.query([artist_id])?;
        while let Some(r) = rows.next()? {
            let album_id: String = r.get(2)?;
            let ok = r.get::<_, Option<String>>(14)?.as_deref() == Some("ok");
            let ver: i64 = r.get::<_, Option<i64>>(15)?.unwrap_or(0);
            let song_status: Option<String> = r.get(21)?;
            let unreleased = match song_status.as_deref() {
                Some(s) => s != "released",
                None => unreleased_album.get(album_id.as_str()).copied().unwrap_or(false),
            };
            let num = |i: usize| -> rusqlite::Result<Option<u32>> { Ok(r.get::<_, Option<i64>>(i)?.and_then(|v| u32::try_from(v).ok())) };
            songs.push(SongRow {
                id: r.get(0)?,
                title: r.get(1)?,
                album_title: r.get(3)?,
                artist_title: r.get(4)?,
                track_artist: r.get(5)?,
                disc: r.get(6)?,
                number: r.get(7)?,
                duration_ms: r.get(8)?,
                codec: r.get(9)?,
                bitrate: num(10)?,
                bit_depth: num(11)?,
                sample_rate: num(12)?,
                mbid: r.get(13)?,
                tags_read: ok && ver >= CACHE_VERSION,
                isrc: if ok { r.get(16)? } else { None },
                recording_mbid: if ok { r.get(17)? } else { None },
                tag_album: if ok { r.get(18)? } else { None },
                tag_album_artist: if ok { r.get(19)? } else { None },
                tag_year: if ok { r.get(20)? } else { None },
                unreleased,
                folder: r.get::<_, Option<String>>(22)?.as_deref().and_then(album_folder),
                album_id,
            });
        }
    }
    // Albums of the same name filed under another album artist: a sign the
    // album was split across artists for want of an album artist tag.
    let mut elsewhere: HashMap<String, Vec<Elsewhere>> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT id, title, artist_title, thumb FROM albums WHERE title = ?1 COLLATE NOCASE AND artist_id <> ?2 ORDER BY id LIMIT 5",
        )?;
        for a in &albums {
            let rows = st
                .query_map([a.title.as_str(), artist_id], |r| {
                    Ok(Elsewhere { album_id: r.get(0)?, album: r.get(1)?, artist: r.get(2)?, thumb: r.get(3)? })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if !rows.is_empty() {
                elsewhere.insert(a.id.clone(), rows);
            }
        }
    }
    let mut credits: HashMap<String, Vec<String>> = HashMap::new();
    for s in &songs {
        let line = credit_line(s);
        if !credits.contains_key(line) && crate::db::credits::has_separator(line) {
            credits.insert(line.to_string(), crate::db::popular::line_credits(c, line)?);
        }
    }
    Ok(ArtistData { albums, songs, elsewhere, credits })
}

/// A short, stable signature of what the checks look at.
pub fn signature(d: &ArtistData) -> String {
    let mut h = Fnv::default();
    h.add(&CHECKS_VERSION.to_string());
    for a in &d.albums {
        h.add(&format!(
            "a|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{:?}",
            a.id,
            a.title,
            a.thumb,
            a.year,
            a.mbid,
            a.edition,
            a.version,
            a.group,
            a.status.key(),
            a.evidence.iter().map(|e| &e.text).collect::<Vec<_>>()
        ));
    }
    for s in &d.songs {
        h.add(&format!(
            "s|{}|{}|{}|{:?}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}",
            s.id,
            s.title,
            s.album_id,
            s.track_artist,
            s.disc,
            s.number,
            s.duration_ms,
            s.codec,
            s.bitrate,
            s.bit_depth,
            s.sample_rate,
            s.mbid,
            s.tags_read,
            s.isrc,
            s.recording_mbid,
            s.tag_album,
            s.tag_album_artist,
            s.tag_year,
            s.unreleased
        ));
        if let Some(f) = &s.folder {
            h.add(f);
        }
    }
    let mut keys: Vec<_> = d.elsewhere.iter().collect();
    keys.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in keys {
        h.add(&format!("e|{k}|{:?}", v.iter().map(|e| &e.album_id).collect::<Vec<_>>()));
    }
    format!("{:016x}", h.0)
}

/// FNV-1a, so signatures stay the same from one build to the next.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn add(&mut self, s: &str) {
        for b in s.bytes().chain(std::iter::once(0xff)) {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Check one album artist when what the checks look at changed since the
/// last check: the new signature and findings, or None when nothing did.
pub fn check_if_changed(c: &Connection, artist_id: &str) -> AppResult<Option<(String, Vec<Finding>)>> {
    let d = load(c, artist_id)?;
    let sig = signature(&d);
    let old: Option<String> =
        c.query_row("SELECT sig FROM doctor_artists WHERE artist_id = ?1", [artist_id], |r| r.get(0)).optional()?;
    if old.as_deref() == Some(sig.as_str()) {
        return Ok(None);
    }
    Ok(Some((sig, run_checks(artist_id, &d))))
}

/// Every check over one album artist's data.
pub fn run_checks(artist_id: &str, d: &ArtistData) -> Vec<Finding> {
    let mut out = Vec::new();
    let by_album = songs_by_album(d);
    out.extend(duplicates(artist_id, d, &by_album));
    out.extend(album_tags(artist_id, d, &by_album));
    out.extend(album_artist(artist_id, d, &by_album));
    out.extend(split_albums(artist_id, d, &by_album));
    out.extend(low_quality(artist_id, d));
    out.extend(unmatched(artist_id, d));
    out
}

fn songs_by_album(d: &ArtistData) -> HashMap<&str, Vec<&SongRow>> {
    let mut m: HashMap<&str, Vec<&SongRow>> = HashMap::new();
    for s in &d.songs {
        m.entry(s.album_id.as_str()).or_default().push(s);
    }
    m
}

/// What the matcher knows a song by (the same rules as a synced track).
pub fn song_key(s: &SongRow) -> SongKey {
    let album_artist = fold_artist(&s.artist_title);
    let own = s.track_artist.as_deref().map(fold_artist).filter(|a| !a.is_empty());
    let (artist_fold, album_artist_fold) = match own {
        Some(a) if a != album_artist => (a, Some(album_artist).filter(|x| !x.is_empty())),
        Some(a) => (a, None),
        None => (album_artist, None),
    };
    let t = fold_title(&s.title);
    SongKey {
        isrc: s.isrc.as_deref().and_then(crate::tags::clean_isrc),
        recording_mbid: s.recording_mbid.as_deref().and_then(crate::tags::clean_mbid),
        plex_track_mbid: s.mbid.as_deref().map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty()),
        artist_fold,
        album_artist_fold,
        title_fold: t.title,
        version_fold: t.version,
        album_fold: fold_album(&s.album_title),
        duration_ms: s.duration_ms.max(0) as u64,
        unreleased: s.unreleased,
        sound_mbid: None,
    }
}

/// "3:45".
pub fn length(ms: i64) -> String {
    let s = (ms.max(0) + 500) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// "FLAC 24-bit/96 kHz", "MP3 320 kbps", or "unknown quality".
pub fn quality_label(s: &SongRow) -> String {
    let codec = s.codec.as_deref().map(str::to_ascii_uppercase).filter(|c| !c.is_empty());
    let lossless = crate::db::merge::quality(s.codec.as_deref(), None, None, None).0 == 1;
    let detail = if lossless {
        match (s.bit_depth, s.sample_rate) {
            (Some(b), Some(r)) if b > 0 && r > 0 => Some(format!("{b}-bit/{} kHz", khz(r))),
            _ => None,
        }
    } else {
        s.bitrate.filter(|b| *b > 0).map(|b| format!("{b} kbps"))
    };
    match (codec, detail) {
        (Some(c), Some(d)) => format!("{c} {d}"),
        (Some(c), None) => c,
        (None, Some(d)) => d,
        (None, None) => "unknown quality".into(),
    }
}

fn khz(hz: u32) -> String {
    let k = hz as f64 / 1000.0;
    if (k - k.round()).abs() < 0.05 { format!("{}", k.round() as u32) } else { format!("{k:.1}") }
}

fn quality(s: &SongRow) -> (u8, u32, u32, u32) {
    crate::db::merge::quality(s.codec.as_deref(), s.bit_depth, s.sample_rate, s.bitrate)
}

/// `a` is a clearly worse copy than `b`: lossy next to lossless, or a low
/// lossy bitrate well under the other lossy copy's.
pub fn clearly_worse(a: &SongRow, b: &SongRow) -> bool {
    let (qa, qb) = (quality(a), quality(b));
    if qa.0 != qb.0 {
        return qa.0 < qb.0;
    }
    if qa.0 == 1 {
        return false;
    }
    match (a.bitrate, b.bitrate) {
        (Some(x), Some(y)) if x > 0 && y > 0 => x <= LOSSY_LOW_KBPS && (x as f64) < (y as f64) * LOSSY_RATIO,
        _ => false,
    }
}

fn why_same(t: Tier) -> &'static str {
    match t {
        Tier::Isrc => "the same ISRC",
        Tier::RecordingMbid => "the same MusicBrainz recording",
        Tier::PlexTrackMbid => "the same Plex match",
        Tier::Fingerprint => "the same sound (an audio fingerprint)",
        Tier::Text | Tier::TextOtherVersion => "the same title and length",
    }
}

fn times(n: usize) -> String {
    match n {
        2 => "twice".into(),
        n => format!("{n} times"),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn album_of<'a>(d: &'a ArtistData, id: &str) -> Option<&'a AlbumRow> {
    d.albums.iter().find(|a| a.id == id)
}

fn base(kind: Kind, id: String, artist_id: &str, a: &AlbumRow, title: String, text: String, fix: Fix) -> Finding {
    Finding {
        id,
        kind,
        artist_id: artist_id.to_string(),
        artist: a.artist.clone(),
        album_id: a.id.clone(),
        album: a.title.clone(),
        thumb: a.thumb.clone(),
        title,
        text,
        evidence: Vec::new(),
        fix,
        tracks: Vec::new(),
        others: Vec::new(),
        table: Vec::new(),
        suggest: None,
        set: None,
        album_note: None,
    }
}

/// Union-find over indexes.
fn find(p: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while p[r] != r {
        r = p[r];
    }
    let mut i = i;
    while p[i] != r {
        let n = p[i];
        p[i] = r;
        i = n;
    }
    r
}

// ------------------------------------------------------------------ duplicates

/// The same recording more than once on one album.
pub fn duplicates(artist_id: &str, d: &ArtistData, by_album: &HashMap<&str, Vec<&SongRow>>) -> Vec<Finding> {
    let mut out = Vec::new();
    for a in &d.albums {
        let Some(songs) = by_album.get(a.id.as_str()) else { continue };
        let keys: Vec<SongKey> = songs.iter().map(|s| song_key(s)).collect();
        let mut parent: Vec<usize> = (0..songs.len()).collect();
        let mut tier: HashMap<usize, Tier> = HashMap::new();
        for i in 0..songs.len() {
            for j in i + 1..songs.len() {
                let Some(m) = compare(&keys[i], &keys[j], Context::MatchedAlbum) else { continue };
                if m.kind != MatchKind::SameRecording {
                    continue;
                }
                // Two "Intro"s on one album are only the same file when they sit in the same place.
                let same_slot = songs[i].number > 0 && songs[i].disc == songs[j].disc && songs[i].number == songs[j].number;
                if m.tier == Tier::Text && is_generic(&keys[i].title_fold) && !same_slot {
                    continue;
                }
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[rj] = ri;
                }
                let t = tier.entry(i).or_insert(m.tier);
                *t = (*t).min(m.tier);
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..songs.len() {
            let r = find(&mut parent, i);
            groups.entry(r).or_default().push(i);
        }
        let groups: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
        if groups.is_empty() {
            continue;
        }
        let first = songs[groups[0][0]];
        let title = if groups.len() == 1 {
            format!("\u{201c}{}\u{201d} is on this album {}", first.title, times(groups[0].len()))
        } else {
            format!("{} are on this album more than once", plural(groups.len(), "song", "songs"))
        };
        let mut f = base(
            Kind::Duplicate,
            format!("duplicate:{}", a.id),
            artist_id,
            a,
            title,
            "Your library has more than one file of the same recording here, so it plays twice. Open the album to compare the copies, then remove the one you don't want from your music folder.".into(),
            Fix::OpenAlbum,
        );
        for g in &groups {
            let why = g.iter().filter_map(|i| tier.get(i)).min().copied().unwrap_or(Tier::Text);
            let copies: Vec<String> =
                g.iter().map(|&i| format!("{}, {}", length(songs[i].duration_ms), quality_label(songs[i]))).collect();
            f.evidence.push(format!(
                "\u{201c}{}\u{201d} is here {} with {}: {}.",
                songs[g[0]].title,
                times(g.len()),
                why_same(why),
                copies.join("; ")
            ));
            f.tracks.extend(g.iter().map(|&i| songs[i].id.clone()));
        }
        out.push(f);
    }
    out
}

// ------------------------------------------------------------------ album tags

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn table(songs: &[&SongRow]) -> Vec<TagRow> {
    songs
        .iter()
        .map(|s| TagRow {
            track_id: s.id.clone(),
            title: s.title.clone(),
            album: s.tag_album.clone(),
            album_artist: s.tag_album_artist.clone(),
            year: s.tag_year,
        })
        .collect()
}

/// "“Title” on 8 songs, “Title (Deluxe)” on 2".
fn spread<T: Clone + Ord + std::fmt::Display>(values: &[T]) -> String {
    let mut counts: BTreeMap<T, usize> = BTreeMap::new();
    for v in values {
        *counts.entry(v.clone()).or_default() += 1;
    }
    let mut parts: Vec<(T, usize)> = counts.into_iter().collect();
    parts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    parts
        .iter()
        .map(|(v, n)| format!("\u{201c}{v}\u{201d} on {}", plural(*n, "song", "songs")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Distinct values (folded for comparing) in the order first seen, as written.
fn differing(values: impl Iterator<Item = String>) -> Option<Vec<String>> {
    let all: Vec<String> = values.collect();
    let distinct: HashSet<String> = all.iter().map(|v| norm(v)).collect();
    (distinct.len() > 1).then_some(all)
}

/// Album name, album artist or year differ between one album's files.
pub fn album_tags(artist_id: &str, d: &ArtistData, by_album: &HashMap<&str, Vec<&SongRow>>) -> Vec<Finding> {
    let mut out = Vec::new();
    for a in &d.albums {
        let Some(songs) = by_album.get(a.id.as_str()) else { continue };
        let read: Vec<&SongRow> = songs.iter().copied().filter(|s| s.tags_read).collect();
        if read.len() < 2 {
            continue;
        }
        let mut evidence = Vec::new();
        let mut fields = Vec::new();
        let mut set = TagFix::default();
        if let Some(v) = differing(read.iter().filter_map(|s| s.tag_album.clone())) {
            evidence.push(format!("Album name: {}.", spread(&v)));
            fields.push("album name");
            set.album = most_common(&v);
        }
        if let Some(v) = differing(read.iter().filter_map(|s| s.tag_album_artist.clone())) {
            evidence.push(format!("Album artist: {}.", spread(&v)));
            fields.push("album artist");
            set.album_artist = most_common(&v);
        }
        let years: Vec<i32> = read.iter().filter_map(|s| s.tag_year).collect();
        if years.iter().collect::<HashSet<_>>().len() > 1 {
            evidence.push(format!("Year: {}.", spread(&years)));
            fields.push("year");
            set.year = most_common_year(&years);
        }
        if fields.is_empty() {
            continue;
        }
        let what = match fields.as_slice() {
            [one] => one.to_string(),
            [a, b] => format!("{a} and {b}"),
            _ => "album name, album artist and year".into(),
        };
        let mut f = base(
            Kind::AlbumTags,
            format!("albumTags:{}", a.id),
            artist_id,
            a,
            format!("The songs don't agree on the {what}"),
            format!(
                "The files on this album have different {}s, which can make Plex show the album wrong or split it. Fix the tags gives every song the same {what}, and shows each change before it's saved.",
                if fields.len() == 1 { fields[0] } else { "tag" }
            ),
            Fix::Tags,
        );
        f.evidence = evidence;
        f.evidence.push(format!("Read from {} of the album's {} files.", read.len(), songs.len()));
        f.table = table(songs);
        f.set = Some(set);
        out.push(f);
    }
    out
}

/// The value most songs have, case and spacing aside, as most of them
/// spell it. A tie goes to the one the earliest song has.
fn most_common(values: &[String]) -> Option<String> {
    let mut groups: Vec<(String, Vec<&String>)> = Vec::new();
    for v in values.iter().filter(|v| !v.trim().is_empty()) {
        let k = norm(v);
        match groups.iter_mut().find(|(g, _)| *g == k) {
            Some((_, list)) => list.push(v),
            None => groups.push((k, vec![v])),
        }
    }
    // Stable sort: equal counts keep the order the songs come in.
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    let (_, list) = groups.into_iter().next()?;
    let mut spellings: Vec<(&String, usize)> = Vec::new();
    for v in list {
        match spellings.iter_mut().find(|(s, _)| *s == v) {
            Some((_, n)) => *n += 1,
            None => spellings.push((v, 1)),
        }
    }
    spellings.sort_by(|a, b| b.1.cmp(&a.1));
    spellings.first().map(|(s, _)| s.trim().to_string())
}

fn most_common_year(years: &[i32]) -> Option<i32> {
    let as_text: Vec<String> = years.iter().map(i32::to_string).collect();
    most_common(&as_text)?.parse().ok()
}

// ------------------------------------------------------------------ album artist

/// Files without an album artist on an album whose songs credit several
/// artists, or that has the same name under another artist.
pub fn album_artist(artist_id: &str, d: &ArtistData, by_album: &HashMap<&str, Vec<&SongRow>>) -> Vec<Finding> {
    let mut out = Vec::new();
    for a in &d.albums {
        let Some(songs) = by_album.get(a.id.as_str()) else { continue };
        let read: Vec<&SongRow> = songs.iter().copied().filter(|s| s.tags_read).collect();
        let lacking = read.iter().filter(|s| s.tag_album_artist.as_deref().is_none_or(|x| x.trim().is_empty())).count();
        if read.is_empty() || lacking == 0 {
            continue;
        }
        // Each artist once, however many songs or credit lines name them.
        let mut credits: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        // How many songs each artist leads.
        let mut leads: Vec<(String, usize)> = Vec::new();
        for s in songs {
            let line = credit_line(s);
            let one = [line.trim().to_string()];
            let names = d.credits.get(line).map(Vec::as_slice).unwrap_or(&one);
            for (i, name) in names.iter().enumerate() {
                let k = fold_artist(name);
                if k.is_empty() {
                    continue;
                }
                if i == 0 {
                    match leads.iter_mut().find(|(n, _)| fold_artist(n) == k) {
                        Some((_, c)) => *c += 1,
                        None => leads.push((name.to_string(), 1)),
                    }
                }
                if seen.insert(k) {
                    credits.push(name.to_string());
                }
            }
        }
        let filed_under_none = fold_artist(&a.artist).is_empty();
        let elsewhere = d.elsewhere.get(&a.id).cloned().unwrap_or_default();
        if credits.len() < 2 && !filed_under_none && elsewhere.is_empty() {
            continue;
        }
        let title = if lacking == read.len() {
            "No album artist on any song".to_string()
        } else {
            format!("No album artist on {} of {} songs", lacking, read.len())
        };
        let mut f = base(
            Kind::AlbumArtist,
            format!("albumArtist:{}", a.id),
            artist_id,
            a,
            title,
            "Without an album artist, Plex files each song under its own artist and can break the album up. Fix the tags sets one album artist on every song, and shows each change before it's saved.".into(),
            Fix::Tags,
        );
        f.evidence.push(format!(
            "{} of the {} files read {} no album artist tag.",
            lacking,
            read.len(),
            if lacking == 1 { "has" } else { "have" }
        ));
        if credits.len() >= 2 {
            let shown: Vec<&str> = credits.iter().take(4).map(String::as_str).collect();
            let more = credits.len().saturating_sub(shown.len());
            f.evidence.push(if more > 0 {
                format!("The songs credit {} and {}.", shown.join(", "), plural(more, "other artist", "other artists"))
            } else {
                format!("The songs credit {}.", join_and(&shown))
            });
        }
        if filed_under_none {
            f.evidence.push(format!("Plex files the album under \u{201c}{}\u{201d}.", a.artist));
        }
        for e in &elsewhere {
            f.evidence.push(format!("An album called \u{201c}{}\u{201d} is also under {}.", e.album, e.artist));
            f.others.push(OtherAlbum {
                id: e.album_id.clone(),
                title: format!("{} ({})", e.album, e.artist),
                thumb: e.thumb.clone(),
                note: None,
            });
        }
        f.table = table(songs);
        f.set = Some(TagFix { album_artist: suggest_album_artist(a, &read, &leads, songs.len()), ..TagFix::default() });
        out.push(f);
    }
    out
}

/// The album artist to suggest: what the songs that have one say; else the
/// artist who leads at least half the songs; else "Various Artists" when
/// several do. The user can change it before anything is written.
fn suggest_album_artist(a: &AlbumRow, read: &[&SongRow], leads: &[(String, usize)], songs: usize) -> Option<String> {
    let tagged: Vec<String> = read.iter().filter_map(|s| s.tag_album_artist.clone()).filter(|v| !v.trim().is_empty()).collect();
    if let Some(v) = most_common(&tagged) {
        return Some(v);
    }
    let mut leads = leads.to_vec();
    leads.sort_by(|x, y| y.1.cmp(&x.1));
    if let Some((name, n)) = leads.first()
        && n * 2 >= songs
    {
        return Some(name.clone());
    }
    if leads.len() >= 2 {
        return Some("Various Artists".into());
    }
    Some(a.artist.trim().to_string()).filter(|n| !n.is_empty())
}

fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [a] => a.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// ------------------------------------------------------------------ split albums

fn same_opt(a: &Option<String>, b: &Option<String>) -> bool {
    let n = |v: &Option<String>| v.as_deref().map(norm).filter(|x| !x.is_empty());
    n(a) == n(b)
}

/// Whether two same-titled albums look like halves of one album.
fn halves(a: &AlbumRow, b: &AlbumRow, sa: &[&SongRow], sb: &[&SongRow]) -> bool {
    if a.status.is_unreleased() != b.status.is_unreleased() {
        return false;
    }
    // Editions, builds and versions are kept apart on purpose.
    if !same_opt(&a.edition, &b.edition) || !same_opt(&a.version, &b.version) || !same_opt(&a.group, &b.group) {
        return false;
    }
    if let (Some(x), Some(y)) = (a.year, b.year)
        && x != y
    {
        return false;
    }
    if let (Some(x), Some(y)) = (a.mbid.as_deref(), b.mbid.as_deref())
        && !x.eq_ignore_ascii_case(y)
    {
        return false;
    }
    if sa.is_empty() || sb.is_empty() {
        return false;
    }
    // Halves don't share songs; two copies of one album do (that's a duplicate).
    let slots = |s: &[&SongRow]| -> HashSet<(i64, i64)> { s.iter().filter(|x| x.number > 0).map(|x| (x.disc, x.number)).collect() };
    if !slots(sa).is_disjoint(&slots(sb)) {
        return false;
    }
    let titles = |s: &[&SongRow]| -> HashSet<String> { s.iter().map(|x| fold_title(&x.title).title).filter(|t| !t.is_empty()).collect() };
    titles(sa).is_disjoint(&titles(sb))
}

/// What tells one part of a split album apart: "4 songs, from “Intro”".
fn part_note(songs: &[&SongRow]) -> String {
    let first = songs.iter().min_by_key(|s| (s.disc, s.number));
    match first {
        Some(s) if !s.title.trim().is_empty() => format!("{}, from \u{201c}{}\u{201d}", plural(songs.len(), "song", "songs"), s.title.trim()),
        _ => plural(songs.len(), "song", "songs"),
    }
}

fn range_text(songs: &[&SongRow]) -> String {
    let mut n: Vec<i64> = songs.iter().map(|s| s.number).filter(|n| *n > 0).collect();
    n.sort_unstable();
    match (n.first(), n.last()) {
        (Some(a), Some(b)) if a == b => format!("song {a}"),
        (Some(a), Some(b)) => format!("songs {a} to {b}"),
        _ => plural(songs.len(), "song", "songs"),
    }
}

/// The album folder a file is in: its folder, or the folder above a disc
/// folder ("Disc 2", "CD2"), folded for comparing.
pub fn album_folder(file: &str) -> Option<String> {
    let parts: Vec<&str> = file.split(['/', '\\']).collect();
    if parts.len() < 2 {
        return None;
    }
    let mut dir = &parts[..parts.len() - 1];
    if let Some(last) = dir.last() {
        let l = last.trim().to_lowercase();
        let rest = l.trim_start_matches("disc").trim_start_matches("disk").trim_start_matches("cd").trim();
        if rest.len() < l.len() && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) && dir.len() > 1 {
            dir = &dir[..dir.len() - 1];
        }
    }
    let f = dir.join("/");
    (!f.trim_matches('/').is_empty()).then_some(f)
}

/// The one folder all of an album's files are in, if they share one.
fn only_folder(songs: &[&SongRow]) -> Option<String> {
    let mut it = songs.iter().map(|s| s.folder.clone());
    let first = it.next()??;
    it.all(|f| f.as_deref() == Some(first.as_str())).then_some(first)
}

/// Two albums whose files share one folder: Plex shows one album in two
/// (a bonus track tagged a little differently, say), whatever their titles.
fn same_folder_halves(a: &AlbumRow, b: &AlbumRow, sa: &[&SongRow], sb: &[&SongRow]) -> bool {
    if a.status.is_unreleased() != b.status.is_unreleased() {
        return false;
    }
    if !same_opt(&a.edition, &b.edition) || !same_opt(&a.version, &b.version) || !same_opt(&a.group, &b.group) {
        return false;
    }
    let (Some(fa), Some(fb)) = (only_folder(sa), only_folder(sb)) else { return false };
    if fa != fb {
        return false;
    }
    // Two copies of the same songs in one folder are duplicates, not halves.
    let titles = |s: &[&SongRow]| -> HashSet<String> { s.iter().map(|x| fold_title(&x.title).title).filter(|t| !t.is_empty()).collect() };
    titles(sa).is_disjoint(&titles(sb))
}

/// Albums with the same title and album artist that look like one album in
/// two, and albums whose files share one folder.
pub fn split_albums(artist_id: &str, d: &ArtistData, by_album: &HashMap<&str, Vec<&SongRow>>) -> Vec<Finding> {
    let empty: Vec<&SongRow> = Vec::new();
    let songs_of = |id: &str| by_album.get(id).unwrap_or(&empty);
    // Albums group by title, and albums sharing a folder join one group.
    let n = d.albums.len();
    let mut group_of: Vec<usize> = (0..n).collect();
    {
        let mut first_by_title: HashMap<String, usize> = HashMap::new();
        let mut first_by_folder: HashMap<String, usize> = HashMap::new();
        for (i, a) in d.albums.iter().enumerate() {
            let k = fold_text(&a.title);
            if !k.is_empty() {
                let g = *first_by_title.entry(k).or_insert(i);
                let (ri, rg) = (find(&mut group_of, i), find(&mut group_of, g));
                group_of[ri] = rg;
            }
            if let Some(f) = only_folder(songs_of(&a.id)) {
                let g = *first_by_folder.entry(f).or_insert(i);
                let (ri, rg) = (find(&mut group_of, i), find(&mut group_of, g));
                group_of[ri] = rg;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<&AlbumRow>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut group_of, i);
        groups.entry(r).or_default().push(&d.albums[i]);
    }
    let mut out = Vec::new();
    for group in groups.values().filter(|g| g.len() > 1) {
        let mut parent: Vec<usize> = (0..group.len()).collect();
        for i in 0..group.len() {
            for j in i + 1..group.len() {
                let (si, sj) = (songs_of(&group[i].id), songs_of(&group[j].id));
                let titled = fold_text(&group[i].title) == fold_text(&group[j].title) && halves(group[i], group[j], si, sj);
                if titled || same_folder_halves(group[i], group[j], si, sj) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[rj] = ri;
                    }
                }
            }
        }
        let mut clusters: BTreeMap<usize, Vec<&AlbumRow>> = BTreeMap::new();
        for (i, a) in group.iter().enumerate() {
            let r = find(&mut parent, i);
            clusters.entry(r).or_default().push(a);
        }
        for mut parts in clusters.into_values().filter(|c| c.len() > 1) {
            // The biggest part keeps its place; the others merge into it.
            parts.sort_by(|a, b| songs_of(&b.id).len().cmp(&songs_of(&a.id).len()).then_with(|| a.id.cmp(&b.id)));
            let main = parts[0];
            let mut ids: Vec<&str> = parts.iter().map(|a| a.id.as_str()).collect();
            ids.sort_unstable();
            let n = parts.len();
            let mut f = base(
                Kind::SplitAlbum,
                format!("splitAlbum:{}", ids.join("+")),
                artist_id,
                main,
                if n == 2 { format!("{} is split in two", main.title) } else { format!("{} is split in {n}", main.title) },
                format!(
                    "Plex shows one album as {n}, with the songs divided between them. Merging puts them back together as one album, and you can split it again later."
                ),
                Fix::MergeAlbums,
            );
            let one_title = parts.iter().all(|p| fold_text(&p.title) == fold_text(&main.title));
            if one_title {
                f.evidence.push(format!("{} albums are called \u{201c}{}\u{201d} by {}.", n, main.title, main.artist));
                for p in &parts {
                    f.evidence.push(format!("One has {}.", range_text(songs_of(&p.id))));
                }
            } else {
                f.evidence.push(format!("Plex shows {} albums by {}:", n, main.artist));
                for p in &parts {
                    f.evidence.push(format!("\u{201c}{}\u{201d} has {}.", p.title, range_text(songs_of(&p.id))));
                }
            }
            let folders: Vec<Option<String>> = parts.iter().map(|p| only_folder(songs_of(&p.id))).collect();
            let shared = folders.first().cloned().flatten().filter(|f0| folders.iter().all(|f| f.as_deref() == Some(f0.as_str())));
            match &shared {
                Some(folder) => f.evidence.push(format!("Their files are all in one folder ({folder}), so they're one album on disk.")),
                None => f.evidence.push("They share no songs, and none is tagged as another edition or version.".into()),
            }
            f.album_note = Some(part_note(songs_of(&main.id)));
            f.others = parts[1..]
                .iter()
                .map(|p| OtherAlbum { id: p.id.clone(), title: p.title.clone(), thumb: p.thumb.clone(), note: Some(part_note(songs_of(&p.id))) })
                .collect();
            out.push(f);
        }
    }
    out
}

// ------------------------------------------------------------------ low quality

/// Songs with a clearly better copy of the same recording on another album.
pub fn low_quality(artist_id: &str, d: &ArtistData) -> Vec<Finding> {
    let mut buckets: HashMap<String, Vec<(&SongRow, SongKey)>> = HashMap::new();
    for s in &d.songs {
        let k = song_key(s);
        if !k.title_fold.is_empty() && !s.unreleased {
            buckets.entry(k.title_fold.clone()).or_default().push((s, k));
        }
    }
    // (worse album, better album) → (worse song, better song).
    let mut pairs: BTreeMap<(String, String), Vec<(&SongRow, &SongRow)>> = BTreeMap::new();
    for list in buckets.values().filter(|l| l.len() > 1) {
        for (i, (a, ka)) in list.iter().enumerate() {
            for (b, kb) in list.iter().skip(i + 1) {
                if a.album_id == b.album_id {
                    continue;
                }
                let Some(m) = compare(ka, kb, Context::Single) else { continue };
                if m.kind != MatchKind::SameRecording {
                    continue;
                }
                let (worse, better) = if clearly_worse(a, b) {
                    (*a, *b)
                } else if clearly_worse(b, a) {
                    (*b, *a)
                } else {
                    continue;
                };
                let list = pairs.entry((worse.album_id.clone(), better.album_id.clone())).or_default();
                if !list.iter().any(|(w, _)| w.id == worse.id) {
                    list.push((worse, better));
                }
            }
        }
    }
    let mut out = Vec::new();
    for ((worse_id, better_id), mut songs) in pairs {
        let (Some(wa), Some(ba)) = (album_of(d, &worse_id), album_of(d, &better_id)) else { continue };
        songs.sort_by(|x, y| (x.0.disc, x.0.number, &x.0.id).cmp(&(y.0.disc, y.0.number, &y.0.id)));
        let title = if songs.len() == 1 {
            format!("\u{201c}{}\u{201d} has a better copy on {}", songs[0].0.title, ba.title)
        } else {
            format!("{} have a better copy on {}", plural(songs.len(), "song", "songs"), ba.title)
        };
        let mut f = base(
            Kind::LowQuality,
            format!("lowQuality:{worse_id}>{better_id}"),
            artist_id,
            wa,
            title,
            format!(
                "Your library has {} in better quality on {}. Play that copy instead, or look for a better copy of this album.",
                if songs.len() == 1 { "this song" } else { "these songs" },
                ba.title
            ),
            Fix::OpenAlbum,
        );
        for (w, b) in songs.iter().take(12) {
            f.evidence.push(format!("\u{201c}{}\u{201d}: {} here, {} on {}.", w.title, quality_label(w), quality_label(b), ba.title));
        }
        if songs.len() > 12 {
            f.evidence.push(format!("And {} more.", songs.len() - 12));
        }
        f.tracks = songs.iter().map(|(w, _)| w.id.clone()).collect();
        f.others = vec![OtherAlbum { id: ba.id.clone(), title: ba.title.clone(), thumb: ba.thumb.clone(), note: None }];
        out.push(f);
    }
    out
}

// ------------------------------------------------------------------ unmatched

/// Albums Plex couldn't match that MusicBrainz knows. Never an unreleased one.
pub fn unmatched(artist_id: &str, d: &ArtistData) -> Vec<Finding> {
    let mut out = Vec::new();
    for a in &d.albums {
        if a.status != ReleaseStatus::PlexMissed || a.status.is_unreleased() {
            continue;
        }
        let mut f = base(
            Kind::Unmatched,
            format!("unmatched:{}", a.id),
            artist_id,
            a,
            format!("Plex didn't match {}", a.title),
            "MusicBrainz has this release, but your server couldn't match it, so it's missing details like its cover choices and release date. Fix match lets you pick the right release.".into(),
            Fix::Match,
        );
        f.evidence = a.evidence.iter().map(|e| e.text.clone()).collect();
        out.push(f);
    }
    out
}
