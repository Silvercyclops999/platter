//! Other versions of a song, and each song's own release status.
//!
//! Songs are versions of each other when an artist name of each is the same
//! and their titles fold the same (`identity::fold`: version notes, credits,
//! case and punctuation set aside). A bare generic title ("Intro") or a very
//! short one only joins on the same album. Songs on albums sharing an
//! ARCHIVE_GROUP with the same folded title join too, and so do songs with
//! the same recording id from the file's tags or the same track id Plex
//! matched. "Part II" and "Part 2" stay apart (`fold::ROMAN_PART_EQUALS_NUMBER`).
//!
//! Each group gets a key in `versions`; the table is rebuilt after a sync
//! and per artist after tags or marks change ([`refresh_artists`]).
//!
//! A song's own status: released, unreleased (tagged or marked), or for a
//! song on an album that's only probably unreleased, "probably an alternate
//! version" when its group has a released song, else "probably unreleased".
//! An alternate version is still unreleased: nothing here makes anything
//! released.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, ToSql, params, params_from_iter};
use serde::Serialize;

use super::status;
use crate::db::release::{self, AlbumFacts, ReleaseStatus};
use crate::db::types::SearchResults;
use crate::error::AppResult;
use crate::identity::fold;
use crate::model::{SongStatus, Track};

/// Folded titles shorter than this (in characters) only join on the same
/// album, like generic ones.
pub const MIN_TITLE_CHARS: usize = 2;

/// One song as grouping sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Song {
    pub id: String,
    pub artist_id: String,
    pub album_id: String,
    pub title: String,
    pub album_title: String,
    pub album_artist: String,
    pub track_artist: Option<String>,
    /// ARCHIVE_VERSION of the song's file.
    pub version: Option<String>,
    /// The file's status by its release status tags (`tags::provenance`).
    pub release: Option<String>,
    /// ARCHIVE_GROUP of the file, else of the album.
    pub group: Option<String>,
    pub recording_mbid: Option<String>,
    pub plex_mbid: Option<String>,
}

/// The keys a song joins its versions by. The first is its own (the group
/// takes the smallest of its members' first keys as its name).
pub fn keys(s: &Song) -> Vec<String> {
    const SEP: char = '\u{1f}';
    let title = fold::fold_title(&s.title).title;
    let mut out: Vec<String> = Vec::new();
    if !title.is_empty() {
        let aa = fold::fold_artist(&s.album_artist);
        let generic = fold::is_generic(&title) || title.chars().count() < MIN_TITLE_CHARS;
        if generic {
            out.push(format!("g{SEP}{aa}{SEP}{}{SEP}{title}", fold::fold_album(&s.album_title)));
        } else {
            if !aa.is_empty() {
                out.push(format!("t{SEP}{aa}{SEP}{title}"));
            }
            if let Some(ta) = s.track_artist.as_deref().map(fold::fold_artist).filter(|a| !a.is_empty() && *a != aa) {
                out.push(format!("t{SEP}{ta}{SEP}{title}"));
            }
        }
        if let Some(g) = s.group.as_deref().map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty()) {
            out.push(format!("p{SEP}{g}{SEP}{title}"));
        }
    }
    if out.is_empty() {
        out.push(format!("i{SEP}{}", s.id));
    }
    if let Some(m) = s.recording_mbid.as_deref().map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty()) {
        out.push(format!("r{SEP}{m}"));
    }
    if let Some(m) = s.plex_mbid.as_deref().map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty()) {
        out.push(format!("m{SEP}{m}"));
    }
    out
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The featured artists a song credits, from its title and track artist.
fn featured(s: &Song) -> Vec<String> {
    let mut f = fold::featured(&s.title);
    if let Some(ta) = s.track_artist.as_deref() {
        f.extend(fold::featured(ta));
    }
    f.sort();
    f.dedup();
    f
}

/// Two songs with the same name can still be different songs: when both
/// credit featured artists and none are shared ("Home (feat. John Legend)"
/// and "Home (feat. Ant Clemons)"), the name alone doesn't join them. A song
/// with no featured artist (a demo, a solo take) joins either.
fn features_agree(a: &[String], b: &[String]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|x| b.contains(x))
}

/// The group key of each song, same order. Songs sharing any key are in one
/// group, except that a shared name ("t" key) only joins songs whose
/// featured artists agree ([`features_agree`]).
pub fn group(songs: &[Song]) -> Vec<String> {
    let mut parent: Vec<usize> = (0..songs.len()).collect();
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    let feats: Vec<Vec<String>> = songs.iter().map(featured).collect();
    let mut own: Vec<String> = Vec::with_capacity(songs.len());
    let union = |parent: &mut Vec<usize>, i: usize, j: usize| {
        let (a, b) = (find(parent, i), find(parent, j));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    };
    for (i, s) in songs.iter().enumerate() {
        let ks = keys(s);
        own.push(ks[0].clone());
        for k in ks {
            if k.starts_with('t') {
                let members = by_name.entry(k).or_default();
                for &j in members.iter() {
                    if features_agree(&feats[i], &feats[j]) {
                        union(&mut parent, i, j);
                    }
                }
                members.push(i);
                continue;
            }
            match first.get(&k) {
                Some(&j) => union(&mut parent, i, j),
                None => {
                    first.insert(k, i);
                }
            }
        }
    }
    let mut name: HashMap<usize, String> = HashMap::new();
    for (i, o) in own.iter().enumerate() {
        let r = find(&mut parent, i);
        let e = name.entry(r).or_insert_with(|| o.clone());
        if *o < *e {
            *e = o.clone();
        }
    }
    // Groups the featured-artist rule kept apart can share a name ("Home"
    // twice): those get their smallest song id added, so they stay apart.
    let mut count: HashMap<String, usize> = HashMap::new();
    for n in name.values() {
        *count.entry(n.clone()).or_default() += 1;
    }
    let mut least: HashMap<usize, String> = HashMap::new();
    for (i, s) in songs.iter().enumerate() {
        let r = find(&mut parent, i);
        let e = least.entry(r).or_insert_with(|| s.id.clone());
        if s.id < *e {
            *e = s.id.clone();
        }
    }
    (0..songs.len())
        .map(|i| {
            let r = find(&mut parent, i);
            let n = &name[&r];
            if count[n] > 1 { format!("{n}\u{1f}{}", least[&r]) } else { n.clone() }
        })
        .collect()
}

/// Each song's own status and, for a probable alternate (or an unreleased
/// song with a released version), the released song it's a version of.
/// `facts` are the songs' albums'; `groups` from [`group`].
pub fn song_statuses(songs: &[Song], groups: &[String], facts: &HashMap<String, AlbumFacts>) -> Vec<(SongStatus, Option<String>)> {
    let first: Vec<ReleaseStatus> = songs
        .iter()
        .map(|s| {
            let mut f = facts.get(&s.album_id).cloned().unwrap_or_default();
            // The song's own file tags come before its album's.
            if let Some(v) = s.version.as_deref().filter(|v| !v.trim().is_empty()) {
                f.version = Some(v.to_string());
            } else if let Some(r) = s.release.as_deref() {
                f.version = None;
                f.release = Some(r.to_string());
                f.mixed = false;
            }
            f.mark = release::mark_for(&s.album_id, Some(&s.id));
            release::status_of(&f)
        })
        .collect();
    let mut released_in: HashMap<&str, &str> = HashMap::new();
    for (i, s) in songs.iter().enumerate() {
        if !first[i].is_unreleased() {
            released_in.entry(groups[i].as_str()).or_insert(s.id.as_str());
        }
    }
    songs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let alt = released_in.get(groups[i].as_str()).filter(|id| **id != s.id).map(|id| id.to_string());
            match first[i] {
                ReleaseStatus::Released | ReleaseStatus::PlexMissed => (SongStatus::Released, None),
                ReleaseStatus::TaggedUnreleased | ReleaseStatus::ConfirmedUnreleased => (SongStatus::Unreleased, alt),
                ReleaseStatus::ProbablyUnreleased => match alt {
                    Some(a) => (SongStatus::ProbablyAlternate, Some(a)),
                    None => (SongStatus::ProbablyUnreleased, None),
                },
            }
        })
        .collect()
}

const SONGS: &str = "SELECT t.id, t.artist_id, t.album_id, t.title, t.album_title, t.artist_title, t.track_artist,
            f.version, coalesce(nullif(trim(f.grp), ''), at.grp), f.recording_mbid, t.mbid, f.release
     FROM tracks t
     LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
     LEFT JOIN album_tags at ON at.album_id = t.album_id";

fn load_songs(c: &Connection, filter: &str, args: &[&dyn ToSql]) -> AppResult<Vec<Song>> {
    let mut st = c.prepare_cached(&format!("{SONGS} {filter}"))?;
    let rows = st.query_map(params_from_iter(args.iter()), |r| {
        Ok(Song {
            id: r.get(0)?,
            artist_id: r.get(1)?,
            album_id: r.get(2)?,
            title: r.get(3)?,
            album_title: r.get(4)?,
            album_artist: r.get(5)?,
            track_artist: r.get(6)?,
            version: r.get(7)?,
            group: r.get(8)?,
            recording_mbid: r.get(9)?,
            plex_mbid: r.get(10)?,
            release: r.get(11)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn write(c: &Connection, songs: &[Song], facts: &HashMap<String, AlbumFacts>) -> AppResult<usize> {
    let groups = group(songs);
    let statuses = song_statuses(songs, &groups, facts);
    let mut ins = c.prepare_cached(
        "INSERT OR REPLACE INTO versions (track_id, group_key, artist_id, status, alt_of) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (i, s) in songs.iter().enumerate() {
        ins.execute(params![s.id, groups[i], s.artist_id, statuses[i].0.key(), statuses[i].1])?;
    }
    Ok(songs.len())
}

fn facts_map(rows: Vec<(String, String, AlbumFacts)>) -> HashMap<String, AlbumFacts> {
    rows.into_iter().map(|(id, _, f)| (id, f)).collect()
}

/// Rebuild the whole table. Returns the songs written.
pub fn rebuild(c: &Connection) -> AppResult<usize> {
    c.execute("DELETE FROM versions", [])?;
    let songs = load_songs(c, "", &[])?;
    let facts = facts_map(status::load_facts(c, "", "", &[])?);
    write(c, &songs, &facts)
}

/// Artists whose songs can join a group with these artists' songs: the
/// same artist name (a song's own artist can be another album artist), a
/// group they already share (recording ids and the like, as of the last
/// build), or albums with the same ARCHIVE_GROUP. Every query runs on an
/// index, so the tag scan can call this after each batch.
const LINKED_ARTISTS: &[&str] = &[
    "SELECT DISTINCT s2.artist_id FROM song_index s1 JOIN song_index s2 ON s2.artist_fold = s1.artist_fold
     WHERE s1.artist_id IN (SELECT value FROM json_each(?1)) AND s1.artist_fold <> ''",
    "SELECT DISTINCT v2.artist_id FROM versions v1 JOIN versions v2 ON v2.group_key = v1.group_key
     WHERE v1.artist_id IN (SELECT value FROM json_each(?1))",
    "SELECT DISTINCT al2.artist_id FROM albums al1
     JOIN album_tags a1 ON a1.album_id = al1.id
     JOIN album_tags a2 ON a2.grp = a1.grp
     JOIN albums al2 ON al2.id = a2.album_id
     WHERE al1.artist_id IN (SELECT value FROM json_each(?1)) AND trim(coalesce(a1.grp, '')) <> ''",
];

/// More artists than this linked together: the whole table is rebuilt.
const MAX_LINKED: usize = 400;

/// Rebuild the rows of these album artists' songs, and of every artist
/// linked to them (followed until no more turn up), so their groups come
/// out whole and named as a full rebuild names them.
pub fn refresh_artists(c: &Connection, artist_ids: &[String]) -> AppResult<usize> {
    let mut ids: Vec<String> = artist_ids.to_vec();
    let mut seen: HashSet<String> = ids.iter().cloned().collect();
    let mut fresh = ids.clone();
    while !fresh.is_empty() {
        let list = serde_json::to_string(&fresh).unwrap_or_else(|_| "[]".into());
        let mut next = Vec::new();
        for sql in LINKED_ARTISTS {
            let mut st = c.prepare_cached(sql)?;
            for id in st.query_map([&list], |r| r.get::<_, String>(0))? {
                let id = id?;
                if seen.insert(id.clone()) {
                    next.push(id);
                }
            }
        }
        ids.extend(next.iter().cloned());
        if ids.len() > MAX_LINKED {
            return rebuild(c);
        }
        fresh = next;
    }
    let all = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into());
    c.execute("DELETE FROM versions WHERE artist_id IN (SELECT value FROM json_each(?1))", [&all])?;
    let songs = load_songs(c, "WHERE t.artist_id IN (SELECT value FROM json_each(?1))", &[&all])?;
    let facts = facts_map(status::load_facts(
        c,
        "WHERE al.artist_id IN (SELECT value FROM json_each(?1))",
        "WHERE t.artist_id IN (SELECT value FROM json_each(?1))",
        &[&all],
    )?);
    write(c, &songs, &facts)
}

/// Members of a group other than `track_id` that count as other versions:
/// fan and AI edits left out unless shown, and copies of the same song on
/// another server (`dupes`) left out.
const OTHERS: &str = "SELECT v2.track_id FROM versions v
     JOIN versions v2 ON v2.group_key = v.group_key AND v2.track_id <> v.track_id
     JOIN tracks t2 ON t2.id = v2.track_id
     LEFT JOIN file_tags f2 ON f2.part_key = t2.part_key AND f2.status = 'ok'
     WHERE v.track_id = ?1
       AND (?2 = 1 OR NOT (lower(coalesce(f2.flags, '')) LIKE '%fanedit%' OR lower(coalesce(f2.flags, '')) LIKE '%aiedit%'))
       AND coalesce((SELECT d.canon FROM dupes d WHERE d.id = v2.track_id), v2.track_id)
           <> coalesce((SELECT d.canon FROM dupes d WHERE d.id = v.track_id), v.track_id)
     ORDER BY v2.track_id";

/// Other versions of a song (ids), one per song across servers.
pub fn other_ids(c: &Connection, track_id: &str, show_edits: bool) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(OTHERS)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for id in st.query_map(params![track_id, show_edits as i64], |r| r.get::<_, String>(0))? {
        let id = id?;
        let canon: Option<String> = c
            .query_row("SELECT canon FROM dupes WHERE id = ?1", [&id], |r| r.get(0))
            .optional()?;
        if seen.insert(canon.unwrap_or_else(|| id.clone())) {
            out.push(id);
        }
    }
    Ok(out)
}

/// A song's own status, when the table has it.
pub fn song_status(c: &Connection, track_id: &str) -> AppResult<Option<(SongStatus, Option<String>)>> {
    Ok(c
        .prepare_cached("SELECT status, alt_of FROM versions WHERE track_id = ?1")?
        .query_row([track_id], |r| Ok((SongStatus::from_key(&r.get::<_, String>(0)?), r.get(1)?)))
        .optional()?)
}

/// Fill in each song's status, what its file's tags say about where it came
/// from, and how many other versions it has.
pub fn annotate(c: &Connection, tracks: &mut [Track], show_edits: bool) -> AppResult<()> {
    for t in tracks.iter_mut() {
        let file = crate::tags::store::track_release(c, &t.id)?;
        t.file_release = (!file.is_empty()).then_some(file);
        if let Some((s, _)) = song_status(c, &t.id)? {
            t.song_status = Some(s);
            let n = other_ids(c, &t.id, show_edits)?.len() as u32;
            t.other_versions = (n > 0).then_some(n);
        }
    }
    Ok(())
}

/// One version in the Other versions panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionItem {
    pub track: Track,
    pub status: SongStatus,
    /// ARCHIVE_VERSION ("demo", "leak").
    pub version: Option<String>,
    /// ARCHIVE_QUALITY ("cdq", "lq").
    pub quality: Option<String>,
    /// ARCHIVE_DATE as tagged.
    pub date: Option<String>,
    pub era: Option<String>,
    /// ARCHIVE_EDITION of the album.
    pub edition: Option<String>,
    pub year: Option<i32>,
    /// The song the panel was opened from.
    pub current: bool,
}

/// The panel: the song and its other versions, released ones first, then
/// the newest (ARCHIVE_DATE, else the album's year).
pub fn versions_of(c: &Connection, track_id: &str, show_edits: bool) -> AppResult<Vec<VersionItem>> {
    let mut ids = vec![track_id.to_string()];
    ids.extend(other_ids(c, track_id, show_edits)?);
    let mut tracks = crate::db::queries::tracks_by_ids(c, &ids)?;
    annotate(c, &mut tracks, show_edits)?;
    let mut st = c.prepare_cached(
        "SELECT f.version, f.quality, f.date, f.era, at.edition, al.year FROM tracks t
         LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         LEFT JOIN album_tags at ON at.album_id = t.album_id
         LEFT JOIN albums al ON al.id = t.album_id WHERE t.id = ?1",
    )?;
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        let (version, quality, date, era, edition, year) = st.query_row([&t.id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i32>>(5)?,
            ))
        })?;
        let clean = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        out.push(VersionItem {
            status: t.song_status.unwrap_or(SongStatus::Released),
            current: t.id == track_id,
            version: clean(version).map(|v| v.to_lowercase()),
            quality: clean(quality).map(|v| v.to_lowercase()),
            date: clean(date),
            era: clean(era),
            edition: clean(edition),
            year,
            track: t,
        });
    }
    let when = |v: &VersionItem| {
        v.date.as_deref().and_then(super::eras::parse_date).map(|d| d.key()).or(v.year.map(|y| y * 10_000)).unwrap_or(i32::MIN)
    };
    out.sort_by(|a, b| {
        (!a.status.is_released())
            .cmp(&!b.status.is_released())
            .then_with(|| when(b).cmp(&when(a)))
            .then_with(|| a.track.id.cmp(&b.track.id))
    });
    Ok(out)
}

/// The group key of a song.
fn group_of(c: &Connection, track_id: &str) -> AppResult<Option<String>> {
    Ok(c
        .prepare_cached("SELECT group_key FROM versions WHERE track_id = ?1")?
        .query_row([track_id], |r| r.get(0))
        .optional()?)
}

/// The released song a group is shown as: a released hit, else the most
/// played released song in the group. None when nothing in it is released.
fn released_face(c: &Connection, group: &str, hits: &[Track]) -> AppResult<Option<String>> {
    let mut st = c.prepare_cached(
        "SELECT v.track_id FROM versions v JOIN tracks t ON t.id = v.track_id
         WHERE v.group_key = ?1 AND v.status = 'released' ORDER BY t.view_count DESC, t.id",
    )?;
    let released: Vec<String> = st.query_map([group], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(hits
        .iter()
        .map(|t| &t.id)
        .find(|id| released.contains(id))
        .cloned()
        .or_else(|| released.into_iter().next()))
}

/// Search: one row per song, its other versions under it. A group shows as
/// its released version when it has one (never the newest or a demo, so an
/// unreleased version can't stand in for the song), with the rest in
/// `versions` under that song's id. The top result follows the same rule.
pub fn group_search(c: &Connection, out: &mut SearchResults, show_edits: bool) -> AppResult<()> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut songs: Vec<Track> = Vec::with_capacity(out.songs.len());
    let hits = std::mem::take(&mut out.songs);
    let mut face_of: HashMap<String, Track> = HashMap::new();
    for t in &hits {
        let Some(g) = group_of(c, &t.id)? else {
            songs.push(t.clone());
            continue;
        };
        if !seen.insert(g.clone()) {
            continue;
        }
        let face = match released_face(c, &g, &hits)? {
            Some(id) if id == t.id => t.clone(),
            Some(id) => match crate::db::queries::tracks_by_ids(c, &[id])?.into_iter().next() {
                Some(x) => x,
                None => t.clone(),
            },
            None => t.clone(),
        };
        let others = other_ids(c, &face.id, show_edits)?;
        let mut face = face;
        if !others.is_empty() {
            let mut list = crate::db::queries::tracks_by_ids(c, &others)?;
            annotate(c, &mut list, show_edits)?;
            out.versions.insert(face.id.clone(), list);
        }
        annotate(c, std::slice::from_mut(&mut face), show_edits)?;
        face_of.insert(g, face.clone());
        songs.push(face);
    }
    out.songs = songs;
    if let Some(top) = out.top.as_mut()
        && let Some(t) = top.track.as_ref()
        && let Some(g) = group_of(c, &t.id)?
    {
        let face = match face_of.get(&g) {
            Some(f) => Some(f.clone()),
            None => match released_face(c, &g, std::slice::from_ref(t))? {
                Some(id) => crate::db::queries::tracks_by_ids(c, &[id])?.into_iter().next(),
                None => None,
            },
        };
        if let Some(f) = face {
            top.track = Some(f);
        }
    }
    Ok(())
}
