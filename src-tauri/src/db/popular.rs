//! An artist's Popular list and the tracks that credit a name.
//!
//! Global popularity is a list of song titles in order, fetched by
//! `artistinfo` (ListenBrainz, or Last.fm top tracks when ListenBrainz has
//! nothing) and cached in the `artist_info` table under [`GLOBAL_ROW`].
//! Library tracks are matched to it by folded title ([`normalize_title`]).
//! Version qualifiers such as "(Interlude)", "(Live)" or "(Radio Sessions)"
//! are part of the folded title, so a variant only matches an entry with
//! the same qualifier; featured artists and remaster notes fold away.
//!
//! Matched tracks come first, in list order, one copy per entry (see
//! [`rank_with`] for which copy). The rest follow by your own plays, then
//! most recent play; variants of a listed song go after other songs. Plex's
//! `ratingCount` only breaks ties among unmatched tracks: it exists for few
//! tracks and isn't on one scale, so it can't rank.
//!
//! Credited artists: names in a track's artist line that have no library
//! artist get a generated page with the id `credit:<name, lower case>`.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::model::{ReleaseSection, Track};

use super::credits;
use super::store::{TRACK_COLS, row_track};

/// Row name in `artist_info` for the global top songs.
pub const GLOBAL_ROW: &str = "listenbrainz";
/// Id prefix of a credited artist's page.
pub const CREDIT_PREFIX: &str = "credit:";
/// Songs kept from a global list.
pub const GLOBAL_KEEP: usize = 100;
/// Songs on the Popular list.
pub const POPULAR_N: usize = 10;
/// Two copies of one song: same title and length within this.
const SAME_SONG_MS: u64 = 2_000;
/// Release names kept per song on a global list.
const RELEASES_KEEP: usize = 4;

/// One song on a global list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSong {
    pub title: String,
    /// Listens (ListenBrainz) or plays (Last.fm), summed over duplicates.
    pub listens: u64,
    /// Releases the counted recordings came out on (ListenBrainz), most
    /// listened first. Picks the library copy when there are several.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub releases: Vec<String>,
}

/// An artist's global top songs, most popular first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GlobalTop {
    /// "listenbrainz" or "lastfm".
    pub from: String,
    pub songs: Vec<GlobalSong>,
}

impl GlobalTop {
    /// Build from raw (title, listens) pairs. See [`GlobalTop::from_entries`].
    pub fn from_pairs(from: &str, pairs: impl IntoIterator<Item = (String, u64)>) -> Self {
        Self::from_entries(from, pairs.into_iter().map(|(t, n)| (t, n, None)))
    }

    /// Build from raw (title, listens, release) entries: copies of a song
    /// (same folded title) merge with their listens summed and their
    /// releases collected, then the list is sorted and cut to
    /// [`GLOBAL_KEEP`].
    pub fn from_entries(from: &str, entries: impl IntoIterator<Item = (String, u64, Option<String>)>) -> Self {
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut songs: Vec<GlobalSong> = Vec::new();
        for (title, listens, release) in entries {
            let key = normalize_title(&title);
            if key.is_empty() {
                continue;
            }
            let i = match by_key.get(&key) {
                Some(&i) => {
                    songs[i].listens = songs[i].listens.saturating_add(listens);
                    i
                }
                None => {
                    by_key.insert(key, songs.len());
                    songs.push(GlobalSong { title, listens, releases: Vec::new() });
                    songs.len() - 1
                }
            };
            if let Some(r) = release.map(|r| r.trim().to_owned()).filter(|r| !r.is_empty())
                && songs[i].releases.len() < RELEASES_KEEP
                && !songs[i].releases.contains(&r)
            {
                songs[i].releases.push(r);
            }
        }
        // Stable: equal counts keep the source's order.
        songs.sort_by_key(|s| std::cmp::Reverse(s.listens));
        songs.truncate(GLOBAL_KEEP);
        Self { from: from.to_owned(), songs }
    }

    fn ranks(&self) -> HashMap<String, usize> {
        let mut out = HashMap::new();
        for (i, s) in self.songs.iter().enumerate() {
            out.entry(normalize_title(&s.title)).or_insert(i);
        }
        out
    }
}

/// The cached global list for a page id (stale or not), when there is one.
/// It is stored already cut to [`GLOBAL_KEEP`] songs, so a page open only
/// reads a few KB of JSON.
pub fn global_top(c: &Connection, id: &str) -> AppResult<Option<GlobalTop>> {
    let data: Option<Option<String>> = c
        .query_row(
            "SELECT data FROM artist_info WHERE artist_id = ?1 AND source = ?2",
            params![id, GLOBAL_ROW],
            |r| r.get(0),
        )
        .optional()?;
    Ok(data.flatten().and_then(|d| serde_json::from_str::<GlobalTop>(&d).ok()).filter(|g| !g.songs.is_empty()))
}

// ------------------------------------------------------------ title folding

/// Words that make a bracketed or " - " part a different version of the
/// song: kept in the folded title.
const QUALIFIERS: &[&str] = &[
    "interlude", "intro", "outro", "live", "session", "sessions", "demo", "remix", "remixed", "mix", "mixed",
    "instrumental", "acapella", "acappella", "cappella", "capella", "edit", "version", "skit", "reprise", "acoustic",
];
/// Parts that name the main recording: they fold away though they contain a
/// qualifier word.
const PLAIN_PARTS: &[&str] = &["album version", "original version", "original mix", "lp version"];
/// Words after " - " that mark a plain edition note, not a subtitle.
const NOTE_WORDS: &[&str] = &["mono", "stereo", "single", "bonus", "track", "explicit", "clean"];
/// Words that start a featured-artist credit.
const FEAT_WORDS: &[&str] = &["feat", "ft", "featuring"];

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// What a bracketed or " - " part of a title is.
enum Part {
    /// A version qualifier: kept.
    Version(String),
    /// A credit or edition note: dropped.
    Note,
    /// Anything else (a subtitle after " - ").
    Other,
}

/// Classify one part (lower case, without its brackets or dash).
fn classify(part: &str) -> Part {
    let part = part.trim();
    // "feat. X", "Remix feat. X", "with X": the credit goes.
    let mut cut = part.len();
    for (i, _) in part.char_indices() {
        let at_word = i == 0 || !part[..i].chars().next_back().is_some_and(char::is_alphanumeric);
        if !at_word {
            continue;
        }
        let w = words(&part[i..]).next().unwrap_or("");
        if FEAT_WORDS.contains(&w) || (i == 0 && w == "with") {
            cut = i;
            break;
        }
    }
    let part = part[..cut].trim();
    if part.is_empty() {
        return Part::Note;
    }
    let flat = words(part).collect::<Vec<_>>().join(" ");
    let remaster = words(part).any(|w| w.starts_with("remaster"));
    if PLAIN_PARTS.contains(&flat.as_str()) {
        return Part::Note;
    }
    // "2011 Remastered Version" is a remaster note, not a version.
    let qualified = words(part).any(|w| QUALIFIERS.contains(&w) && !(remaster && w == "version"));
    if qualified {
        return Part::Version(part.to_owned());
    }
    if remaster || words(part).all(|w| NOTE_WORDS.contains(&w) || w.chars().all(|c| c.is_ascii_digit())) {
        return Part::Note;
    }
    Part::Other
}

/// A title split for matching: the base title and its version qualifiers.
fn split_title(s: &str) -> (String, Vec<String>) {
    use unicode_normalization::UnicodeNormalization;
    let lower: String = s.nfkc().collect::<String>().to_lowercase();
    let mut base = String::with_capacity(lower.len());
    let mut versions = Vec::new();
    let mut depth = 0usize;
    let mut inner = String::new();
    for ch in lower.chars() {
        match ch {
            '(' | '[' | '{' => {
                if depth > 0 {
                    inner.push(' ');
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Part::Version(v) = classify(&inner) {
                        versions.push(v);
                    }
                    inner.clear();
                } else {
                    inner.push(' ');
                }
            }
            ')' | ']' | '}' => {}
            _ if depth == 0 => base.push(ch),
            _ => inner.push(ch),
        }
    }
    // An unclosed bracket: judge what was in it all the same.
    if depth > 0
        && let Part::Version(v) = classify(&inner)
    {
        versions.push(v);
    }
    // " - 2011 Remaster" goes, " - Live" is a version, " - The Return" stays.
    if let Some(i) = base.find(" - ") {
        match classify(&base[i + 3..]) {
            Part::Version(v) => {
                versions.push(v);
                base.truncate(i);
            }
            Part::Note => base.truncate(i),
            Part::Other => {}
        }
    }
    // "Song feat. Someone".
    for sep in [" feat. ", " feat ", " ft. ", " featuring "] {
        if let Some(i) = base.find(sep) {
            base.truncate(i);
        }
    }
    (base, versions)
}

/// Lower case, apostrophes removed, "&" spelled out, other punctuation
/// turned into single spaces.
fn fold(s: &str) -> String {
    let mut folded = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\'' || ch == '\u{2019}' || ch == '`' {
            continue;
        }
        if ch == '&' {
            folded.push_str(" and ");
        } else if ch.is_alphanumeric() {
            folded.push(ch);
        } else {
            folded.push(' ');
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A title folded for matching: lower case, featured artists and edition
/// notes ("2011 Remaster", "[Bonus Track]", "Album Version") dropped,
/// version qualifiers ("(Interlude)", "- Live", "(Radio Sessions)") kept,
/// apostrophes removed and other punctuation turned into single spaces.
pub fn normalize_title(s: &str) -> String {
    let (base, versions) = split_title(s);
    let mut all = base;
    for v in versions {
        all.push(' ');
        all.push_str(&v);
    }
    fold(&all)
}

/// A title with every bracketed or " - " version part dropped too: the song
/// whatever the version. Also folds album and release names.
pub fn loose_title(s: &str) -> String {
    fold(&split_title(s).0)
}

// ------------------------------------------------------------ ranking

/// What picks between library copies of one listed song.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CopyInfo {
    pub year: Option<i32>,
    /// None counts as an album.
    pub section: Option<ReleaseSection>,
    /// [`super::release::is_unreleased`].
    pub unreleased: bool,
}

/// A track's album info for picking a copy. The file's ARCHIVE_VERSION wins
/// over the album's. Asked only for tracks that match a list entry.
pub fn copy_info(c: &Connection, t: &Track) -> AppResult<CopyInfo> {
    let row = c
        .prepare_cached(
            "SELECT al.year, al.format, al.subformats, al.section,
                    coalesce((SELECT ft.version FROM file_tags ft WHERE ft.part_key = ?2),
                             (SELECT at.version FROM album_tags at WHERE at.album_id = ?1)),
                    al.studio, al.matched
             FROM albums al WHERE al.id = ?1",
        )?
        .query_row(params![t.album_id, t.part_key], |r| {
            let format: Option<String> = r.get(1)?;
            let subs: Option<String> = r.get(2)?;
            let section: Option<String> = r.get(3)?;
            let version: Option<String> = r.get(4)?;
            let studio: Option<String> = r.get(5)?;
            let matched: Option<bool> = r.get(6)?;
            let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
            Ok(CopyInfo {
                year: r.get(0)?,
                section: section.as_deref().map(super::release::section_from_key),
                unreleased: super::release::is_unreleased(format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref()),
            })
        })
        .optional()?;
    Ok(row.unwrap_or_default())
}

/// Studio albums first, demos last.
fn section_pref(s: Option<ReleaseSection>) -> u8 {
    match s.unwrap_or(ReleaseSection::Albums) {
        ReleaseSection::Albums => 0,
        ReleaseSection::SinglesEps => 1,
        ReleaseSection::Compilations => 2,
        ReleaseSection::Soundtracks => 3,
        ReleaseSection::Remixes => 4,
        ReleaseSection::Live => 5,
        ReleaseSection::Mixtapes => 6,
        ReleaseSection::Demos => 7,
    }
}

fn lossless(t: &Track) -> bool {
    let c = t.codec.as_deref().unwrap_or("").to_ascii_lowercase();
    matches!(c.as_str(), "flac" | "alac" | "wav" | "pcm" | "aiff" | "ape" | "wavpack" | "wv" | "tta")
}

/// Rank candidates for Popular without album info (plain tests and callers
/// that have none). See [`rank_with`].
pub fn rank(cands: Vec<Track>, global: Option<&GlobalTop>, n: usize) -> Vec<Track> {
    rank_with(cands, global, n, |_| Ok(CopyInfo::default())).unwrap_or_default()
}

/// Rank candidates for Popular (see the module docs). `global` None ranks
/// by your plays only. `info` is asked only for tracks that match a list
/// entry. When several copies match one entry, one shows, picked by:
/// album named as the entry's release, released over unreleased, studio
/// album over other release types, earliest year, lossless then bitrate,
/// most played.
pub fn rank_with(
    mut cands: Vec<Track>,
    global: Option<&GlobalTop>,
    n: usize,
    mut info: impl FnMut(&Track) -> AppResult<CopyInfo>,
) -> AppResult<Vec<Track>> {
    let ranks = global.map(GlobalTop::ranks).unwrap_or_default();
    let keyed: Vec<(Option<usize>, String)> = cands
        .iter()
        .map(|t| {
            let k = normalize_title(&t.title);
            (ranks.get(&k).copied(), k)
        })
        .collect();
    // Songs on the list that the library has: their other versions wait.
    let listed: HashSet<String> = keyed
        .iter()
        .zip(&cands)
        .filter(|((r, _), _)| r.is_some())
        .map(|(_, t)| loose_title(&t.title))
        .collect();
    let variant: Vec<bool> =
        keyed.iter().zip(&cands).map(|((r, _), t)| r.is_none() && listed.contains(&loose_title(&t.title))).collect();
    // Copy preference for matched tracks (lower is better).
    type Pref = (bool, bool, u8, i32, bool, std::cmp::Reverse<u32>, std::cmp::Reverse<u32>);
    let mut prefs: Vec<Option<Pref>> = vec![None; cands.len()];
    for (i, t) in cands.iter().enumerate() {
        let Some(r) = keyed[i].0 else { continue };
        let ci = info(t)?;
        let releases = &global.expect("ranked by a list").songs[r].releases;
        let album = loose_title(&t.album_title);
        let named = !album.is_empty() && releases.iter().any(|rel| loose_title(rel) == album);
        prefs[i] = Some((
            !named,
            ci.unreleased,
            section_pref(ci.section),
            ci.year.filter(|y| *y > 0).unwrap_or(i32::MAX),
            !lossless(t),
            std::cmp::Reverse(t.bitrate.unwrap_or(0)),
            std::cmp::Reverse(t.view_count),
        ));
    }
    let mut order: Vec<usize> = (0..cands.len()).collect();
    // Stable sort keeps library order for full ties.
    order.sort_by(|&a, &b| {
        let (ta, tb) = (&cands[a], &cands[b]);
        let (ra, rb) = (keyed[a].0, keyed[b].0);
        match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(&y).then(prefs[a].cmp(&prefs[b])),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => variant[a]
                .cmp(&variant[b])
                .then(tb.view_count.cmp(&ta.view_count))
                .then(tb.last_viewed_at.cmp(&ta.last_viewed_at))
                .then(tb.rating_count.cmp(&ta.rating_count)),
        }
    });
    let mut picked: Vec<(usize, Option<usize>)> = Vec::with_capacity(n);
    for i in order {
        let t = &cands[i];
        let dupe = picked.iter().any(|&(j, r)| {
            let o = &cands[j];
            o.id == t.id
                || (r.is_some() && r == keyed[i].0)
                || (keyed[j].1 == keyed[i].1 && o.duration_ms.abs_diff(t.duration_ms) <= SAME_SONG_MS)
        });
        if !dupe {
            picked.push((i, keyed[i].0));
            if picked.len() == n {
                break;
            }
        }
    }
    let mut slots: Vec<Option<Track>> = cands.drain(..).map(Some).collect();
    Ok(picked.into_iter().filter_map(|(i, _)| slots[i].take()).collect())
}

/// Lower-cased names that split credits must keep whole: library artists and
/// learned names (`credit_names`) that appear in `line`.
pub fn known_in(c: &Connection, line: &str) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(
        "SELECT lower(title) FROM artists WHERE instr(lower(?1), lower(title)) > 0
         UNION SELECT key FROM credit_names WHERE instr(lower(?1), key) > 0",
    )?;
    let rows = st.query_map([line], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The credited names of a track's artist line, split with the known names.
pub fn line_credits(c: &Connection, line: &str) -> AppResult<Vec<String>> {
    let known = known_in(c, line)?;
    Ok(credits::split_credits(line, |k| known.iter().any(|n| n == k)).into_iter().map(str::to_owned).collect())
}

/// Tracks whose artist line credits `name` as a whole name (any case),
/// leaving out tracks by `exclude_artist`. The search index finds lines that
/// mention the name without a table scan; names with no letters or digits
/// ("$") fall back to a scan.
pub fn credited_tracks(c: &Connection, name: &str, exclude_artist: Option<&str>) -> AppResult<Vec<Track>> {
    let me = name.trim().to_lowercase();
    if me.is_empty() {
        return Ok(Vec::new());
    }
    let exclude = exclude_artist.unwrap_or("");
    let mentioned: Vec<Track> = if me.chars().any(char::is_alphanumeric) {
        let phrase = format!("track_artist : \"{}\"", me.replace('"', "\"\""));
        let mut st = c.prepare_cached(&format!(
            "SELECT {TRACK_COLS} FROM tracks_fts JOIN tracks t ON t.pk = tracks_fts.rowid
             WHERE tracks_fts MATCH ?1 AND t.artist_id <> ?2 ORDER BY t.pk"
        ))?;
        st.query_map(params![phrase, exclude], row_track)?.collect::<Result<_, _>>()?
    } else {
        let mut st = c.prepare_cached(&format!(
            "SELECT {TRACK_COLS} FROM tracks t
             WHERE t.track_artist IS NOT NULL AND instr(lower(t.track_artist), ?1) > 0 AND t.artist_id <> ?2
             ORDER BY t.pk"
        ))?;
        st.query_map(params![me, exclude], row_track)?.collect::<Result<_, _>>()?
    };
    let mut out = Vec::new();
    for t in mentioned {
        let line = t.track_artist.as_deref().unwrap_or_default();
        if line_credits(c, line)?.iter().any(|n| n.to_lowercase() == me) {
            out.push(t);
        }
    }
    Ok(out)
}

/// The credited name as written on the first track that credits it.
pub fn credited_display_name(c: &Connection, key: &str) -> AppResult<Option<String>> {
    let tracks = credited_tracks(c, key, None)?;
    for t in &tracks {
        if let Some(line) = &t.track_artist
            && let Some(n) = line_credits(c, line)?.into_iter().find(|n| n.to_lowercase() == key)
        {
            return Ok(Some(n));
        }
    }
    Ok(None)
}

/// `credit:<name>` for a credited name.
pub fn credit_id(name: &str) -> String {
    format!("{CREDIT_PREFIX}{}", name.trim().to_lowercase())
}

/// The lower-cased name in a `credit:` id.
pub fn credit_key(id: &str) -> Option<&str> {
    id.strip_prefix(CREDIT_PREFIX).map(str::trim).filter(|k| !k.is_empty())
}

/// Remember names that contain a separator ("Tyler, The Maker") so splits
/// keep them whole. Returns how many were new.
pub fn learn_names<'a>(c: &Connection, names: impl IntoIterator<Item = &'a str>) -> AppResult<usize> {
    let mut st = c.prepare_cached("INSERT OR IGNORE INTO credit_names (key, name) VALUES (?1, ?2)")?;
    let mut n = 0;
    for name in names {
        let name = name.trim();
        if name.is_empty() || !credits::has_separator(name) {
            continue;
        }
        n += st.execute(params![name.to_lowercase(), name])?;
    }
    Ok(n)
}

/// Learned names with no library artist of the same name.
pub fn learned_names(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(
        "SELECT name FROM credit_names cn
         WHERE NOT EXISTS (SELECT 1 FROM artists ar WHERE lower(ar.title) = cn.key) ORDER BY key",
    )?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: &str, title: &str, plays: u32, last: Option<i64>, count: Option<u32>) -> Track {
        let al = crate::db::fake::album("al", "LP", &crate::db::fake::artist("a", "A"), 2020, None, &[]);
        let mut t = crate::db::fake::track(id, title, &al, 1, 1);
        t.view_count = plays;
        t.last_viewed_at = last;
        t.rating_count = count;
        t
    }

    fn ids(v: &[Track]) -> Vec<&str> {
        v.iter().map(|t| t.id.as_str()).collect()
    }

    /// A track on its own album, for copy picking.
    fn on(id: &str, title: &str, album: &str) -> Track {
        let mut t = tr(id, title, 0, None, None);
        t.album_id = format!("al-{album}");
        t.album_title = album.to_owned();
        t
    }

    fn info(year: i32, section: ReleaseSection, unreleased: bool) -> CopyInfo {
        CopyInfo { year: Some(year), section: Some(section), unreleased }
    }

    /// Rank with album info looked up by album title.
    fn rank_by(cands: Vec<Track>, g: &GlobalTop, albums: &[(&str, CopyInfo)]) -> Vec<String> {
        let infos: HashMap<String, CopyInfo> = albums.iter().map(|(a, i)| (a.to_string(), i.clone())).collect();
        rank_with(cands, Some(g), POPULAR_N, |t| Ok(infos.get(&t.album_title).cloned().unwrap_or_default()))
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    }

    #[test]
    fn normalizes_titles() {
        for (a, b) in [
            ("Song Title", "song title"),
            ("Song Title (feat. Someone)", "song title"),
            ("Song Title [feat. Someone & Other]", "song title"),
            ("Song Title (with Someone)", "song title"),
            ("Song Title [Bonus Track]", "song title"),
            ("Song Title - 2011 Remaster", "song title"),
            ("Song Title (2011 Remaster)", "song title"),
            ("Song Title - Remastered 2009", "song title"),
            ("Song Title - 2009 Remastered Version", "song title"),
            ("Song Title - Album Version", "song title"),
            ("Song Title - Mono", "song title"),
            ("Song Title feat. Someone", "song title"),
            ("Don't Stop", "dont stop"),
            ("Don\u{2019}t  Stop!", "dont stop"),
            ("Rock & Roll", "rock and roll"),
            ("Ｗｉｄｅ", "wide"),
            // The same qualifier, written either way, matches.
            ("Song Title (Live)", "Song Title - Live"),
            ("Song Title (Remix) [feat. Someone]", "Song Title - Remix"),
            ("Song Title (Remix feat. Someone)", "Song Title (Remix)"),
        ] {
            assert_eq!(normalize_title(a), normalize_title(b), "{a} vs {b}");
        }
        // Version qualifiers stay: a variant is a different song.
        for v in [
            "Song Title (Interlude)",
            "Song Title (Intro)",
            "Song Title - Outro",
            "Song Title (Live)",
            "Song Title (Live at the Hall)",
            "Song Title (Radio Sessions)",
            "Song Title [Demo]",
            "Song Title (Club Remix)",
            "Song Title (Extended Mix)",
            "Song Title (Instrumental)",
            "Song Title (A Cappella)",
            "Song Title (Acapella)",
            "Song Title (Radio Edit)",
            "Song Title (Single Version)",
            "Song Title (Skit)",
            "Song Title (Reprise)",
            "Song Title - Acoustic",
        ] {
            assert_ne!(normalize_title(v), normalize_title("Song Title"), "{v}");
            assert_eq!(loose_title(v), "song title", "{v}");
        }
        // A real subtitle after " - " stays.
        assert_ne!(normalize_title("Part One - The Return"), normalize_title("Part One"));
        assert_eq!(normalize_title("Ünïcode"), "ünïcode");
        // Words inside a title are not qualifiers.
        assert_eq!(normalize_title("Live Forever"), "live forever");
        assert_eq!(normalize_title("Intro"), "intro");
    }

    #[test]
    fn duplicate_recordings_merge_with_listens_and_releases() {
        let g = GlobalTop::from_entries(
            "listenbrainz",
            [
                ("Hit".into(), 100, Some("First LP".into())),
                ("Other".into(), 150, None),
                ("Hit - 2011 Remaster".into(), 80, Some("First LP (Remastered)".into())),
                ("hit (Live)".into(), 1, Some("Live Album".into())),
            ],
        );
        let got: Vec<_> = g.songs.iter().map(|s| (s.title.as_str(), s.listens, s.releases.clone())).collect();
        assert_eq!(
            got,
            [
                ("Hit", 180, vec!["First LP".to_string(), "First LP (Remastered)".to_string()]),
                ("Other", 150, vec![]),
                ("hit (Live)", 1, vec!["Live Album".to_string()]),
            ]
        );
        // Lists cached before releases were kept still read.
        let old: GlobalTop = serde_json::from_str(r#"{"from":"listenbrainz","songs":[{"title":"Hit","listens":3}]}"#).unwrap();
        assert!(old.songs[0].releases.is_empty());
    }

    #[test]
    fn keeps_the_top_hundred() {
        let g = GlobalTop::from_pairs("listenbrainz", (0..250).map(|i| (format!("Song {i}"), 1000 - i as u64)));
        assert_eq!(g.songs.len(), GLOBAL_KEEP);
        assert_eq!(g.songs[0].title, "Song 0");
    }

    #[test]
    fn the_global_list_wins_over_rating_counts_and_plays() {
        // One album with huge rating counts; the global list ranks the others.
        let cands = vec![
            tr("new1", "Deep Cut", 0, None, Some(318_000)),
            tr("new2", "Filler", 0, None, Some(133_000)),
            tr("old1", "Famous Single", 2, None, Some(618)),
            tr("old2", "Fan Favourite", 0, None, None),
            tr("mine", "My Favourite", 50, Some(9), None),
        ];
        let g = GlobalTop::from_pairs("listenbrainz", [("Famous Single".into(), 900), ("Fan Favourite - Remastered".into(), 500), ("Deep Cut".into(), 10)]);
        assert_eq!(ids(&rank(cands.clone(), Some(&g), 10)), ["old1", "old2", "new1", "mine", "new2"]);
        // No list: plays, then last play; rating count only breaks ties.
        assert_eq!(ids(&rank(cands, None, 10)), ["mine", "old1", "new1", "new2", "old2"]);
    }

    #[test]
    fn a_short_interlude_does_not_take_the_songs_place() {
        // The library has the song and a short interlude of the same name;
        // the interlude is played more.
        let mut song = on("song", "Bright Lights", "Big Record");
        song.duration_ms = 300_000;
        let mut inter = on("inter", "Bright Lights (Interlude)", "Big Record");
        inter.duration_ms = 60_000;
        inter.view_count = 40;
        let other = on("other", "Second Hit", "Big Record");
        let mut mine = on("mine", "Played Often", "Big Record");
        mine.view_count = 10;
        let g = GlobalTop::from_pairs("listenbrainz", [("Bright Lights".into(), 900), ("Second Hit".into(), 500)]);
        // Matches first; the interlude, a variant of a listed song, after
        // other unmatched songs even though it has more plays.
        assert_eq!(rank_by(vec![inter, mine, other, song], &g, &[]), ["song", "other", "mine", "inter"]);

        // Only the interlude in the library: it doesn't stand in for the song.
        let mut inter = on("inter", "Bright Lights (Interlude)", "Big Record");
        inter.view_count = 40;
        let other = on("other", "Second Hit", "Big Record");
        assert_eq!(rank_by(vec![inter, other], &g, &[]), ["other", "inter"]);

        // A listed interlude does match.
        let g = GlobalTop::from_pairs("listenbrainz", [("Bright Lights (Interlude)".into(), 9)]);
        let inter = on("inter", "Bright Lights (Interlude)", "Big Record");
        let song = on("song", "Bright Lights", "Big Record");
        assert_eq!(rank_by(vec![song, inter], &g, &[]), ["inter", "song"]);
    }

    #[test]
    fn released_album_copy_beats_an_unreleased_one() {
        let boot = on("boot", "Power Song", "Leaked Tapes");
        let studio = on("studio", "Power Song", "Fifth Album");
        let g = GlobalTop::from_pairs("listenbrainz", [("Power Song".into(), 900)]);
        let albums = [
            ("Leaked Tapes", info(2009, ReleaseSection::Albums, true)),
            ("Fifth Album", info(2010, ReleaseSection::Albums, false)),
        ];
        assert_eq!(rank_by(vec![boot.clone(), studio.clone()], &g, &albums), ["studio"]);
        // Also when the other copy is on a mixtape with more plays.
        let mut boot = boot;
        boot.view_count = 99;
        let albums = [
            ("Leaked Tapes", info(2009, ReleaseSection::Mixtapes, false)),
            ("Fifth Album", info(2010, ReleaseSection::Albums, false)),
        ];
        assert_eq!(rank_by(vec![boot, studio], &g, &albums), ["studio"]);
    }

    #[test]
    fn a_session_take_on_a_compilation_does_not_match_the_song() {
        let take = on("take", "Gold Song (Radio Sessions)", "Session Collection");
        let song = on("song", "Gold Song", "Second Album");
        let g = GlobalTop::from_pairs("listenbrainz", [("Gold Song".into(), 900)]);
        let albums = [
            ("Session Collection", info(2006, ReleaseSection::Compilations, false)),
            ("Second Album", info(2005, ReleaseSection::Albums, false)),
        ];
        assert_eq!(rank_by(vec![take, song], &g, &albums), ["song", "take"]);
        // The same title on a compilation and the album: the album copy.
        let comp = on("comp", "Gold Song", "Session Collection");
        let song = on("song", "Gold Song", "Second Album");
        assert_eq!(rank_by(vec![comp, song], &g, &albums), ["song"]);
    }

    #[test]
    fn the_studio_album_copy_beats_a_live_album() {
        let live = on("live", "Walking Song", "Live with Strings");
        let debut = on("debut", "Walking Song", "First Album");
        let g = GlobalTop::from_pairs("listenbrainz", [("Walking Song".into(), 900)]);
        let albums = [
            ("Live with Strings", info(2006, ReleaseSection::Live, false)),
            ("First Album", info(2004, ReleaseSection::Albums, false)),
        ];
        assert_eq!(rank_by(vec![live.clone(), debut.clone()], &g, &albums), ["debut"]);
        // An untagged live album: the earliest year still picks the debut.
        let albums = [
            ("Live with Strings", info(2006, ReleaseSection::Albums, false)),
            ("First Album", info(2004, ReleaseSection::Albums, false)),
        ];
        assert_eq!(rank_by(vec![live, debut], &g, &albums), ["debut"]);
    }

    #[test]
    fn the_release_named_by_the_list_wins_then_quality_then_plays() {
        let g = GlobalTop::from_entries("listenbrainz", [("Song".to_string(), 9, Some("Greatest Hits".to_string()))]);
        let hits = on("hits", "Song", "Greatest Hits (Deluxe)");
        let lp = on("lp", "Song", "The Album");
        let albums = [
            ("Greatest Hits (Deluxe)", info(2015, ReleaseSection::Compilations, false)),
            ("The Album", info(2001, ReleaseSection::Albums, false)),
        ];
        assert_eq!(rank_by(vec![lp, hits], &g, &albums), ["hits"]);

        // Same album twice: lossless, then bitrate, then plays.
        let g = GlobalTop::from_pairs("listenbrainz", [("Song".into(), 9)]);
        let mut mp3 = on("mp3", "Song", "The Album");
        mp3.codec = Some("mp3".into());
        mp3.bitrate = Some(320);
        mp3.view_count = 50;
        let mut flac = on("flac", "Song", "The Album");
        flac.bitrate = Some(900);
        let mut flac_hi = on("flac_hi", "Song", "The Album");
        flac_hi.bitrate = Some(2_000);
        let albums = [("The Album", info(2001, ReleaseSection::Albums, false))];
        assert_eq!(rank_by(vec![mp3.clone(), flac.clone(), flac_hi], &g, &albums), ["flac_hi"]);
        assert_eq!(rank_by(vec![mp3, flac.clone()], &g, &albums), ["flac"]);
        let mut flac2 = flac.clone();
        flac2.id = "flac2".into();
        flac2.view_count = 3;
        assert_eq!(rank_by(vec![flac, flac2], &g, &albums), ["flac2"]);
    }

    #[test]
    fn feat_and_remaster_copies_still_match() {
        let g = GlobalTop::from_pairs("listenbrainz", [("Duet Song".into(), 9), ("Old Song".into(), 5)]);
        let duet = on("duet", "Duet Song (feat. Guest Singer)", "The Album");
        let old = on("old", "Old Song - 2011 Remaster", "The Album");
        let mut mine = on("mine", "Mine", "The Album");
        mine.view_count = 80;
        assert_eq!(rank_by(vec![mine, old, duet], &g, &[]), ["duet", "old", "mine"]);
    }

    #[test]
    fn copies_of_a_matched_song_show_once() {
        let mut a = tr("a", "Hit", 0, None, None);
        a.duration_ms = 200_000;
        let mut b = tr("b", "Hit (Remastered)", 3, None, None);
        b.duration_ms = 180_000;
        let c = tr("c", "Other", 0, None, None);
        let g = GlobalTop::from_pairs("lastfm", [("Hit".into(), 9), ("Other".into(), 1)]);
        assert_eq!(ids(&rank(vec![a, b, c], Some(&g), 10)), ["b", "c"]);
    }

    #[test]
    fn credit_ids_round_trip() {
        assert_eq!(credit_id(" Some Guest "), "credit:some guest");
        assert_eq!(credit_key("credit:some guest"), Some("some guest"));
        assert_eq!(credit_key("credit:"), None);
        assert_eq!(credit_key("123"), None);
    }
}
