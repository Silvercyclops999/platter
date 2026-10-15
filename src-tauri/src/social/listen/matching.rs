//! Finding a session's song in this computer's library.
//!
//! For each song a guest is asked to play: the host's own copy when this
//! library has it (the host's server is one this account reaches), else the
//! same song on any server here, by the multi-server rules: the same
//! MusicBrainz track id, else the same artist and title (case and
//! punctuation folded) with lengths within [`SLACK_MS`]. Unreleased songs
//! are never matched; only people with the host's own copy hear them.
//!
//! The host describes its songs ([`describe`]) with metadata only: no path,
//! part key, token or address.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use super::wire::Song;
use crate::db::merge::norm;
use crate::error::AppResult;

/// Copies whose lengths differ by more than this are another version (and
/// a title match must be within it).
pub const SLACK_MS: u64 = 3_000;
/// Longest title or artist sent.
pub const TEXT_MAX: usize = 80;

/// A copy here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub duration_ms: u64,
}

/// A song in this library that might be the one asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cand {
    pub id: String,
    pub title: String,
    /// The album artist, and the song's own artist when it differs.
    pub artist: String,
    pub track_artist: Option<String>,
    pub duration_ms: u64,
    pub mbid: Option<String>,
}

/// The copy that is `song`, if any: a MusicBrainz match first (closest
/// length; any length, a longer cut is still the song), else artist, title
/// and length.
pub fn best(song: &Song, cands: &[Cand]) -> Option<Found> {
    if song.u {
        return None;
    }
    let closest = |it: Vec<&Cand>| it.into_iter().min_by_key(|c| (c.duration_ms.abs_diff(song.d), c.id.clone())).map(|c| Found { id: c.id.clone(), duration_ms: c.duration_ms });
    if let Some(mb) = song.mb.as_deref().filter(|m| !m.is_empty()) {
        let hits: Vec<&Cand> = cands.iter().filter(|c| c.mbid.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(mb))).collect();
        if !hits.is_empty() {
            return closest(hits);
        }
    }
    let (Some(title), Some(artist)) = (song.n.as_deref().map(norm), song.a.as_deref().map(norm)) else { return None };
    if title.is_empty() || artist.is_empty() {
        return None;
    }
    let hits: Vec<&Cand> = cands
        .iter()
        .filter(|c| {
            norm(&c.title) == title
                && (norm(&c.artist) == artist || c.track_artist.as_deref().is_some_and(|a| norm(a) == artist))
                && c.duration_ms.abs_diff(song.d) <= SLACK_MS
        })
        .collect();
    closest(hits)
}

/// Where a copy that runs `here_ms` should be when the host's runs `host_ms`
/// and is at `pos_ms`: the same spot for the same length, the same share of
/// the song for another version.
pub fn map_position(pos_ms: u64, host_ms: u64, here_ms: u64) -> u64 {
    if host_ms == 0 || here_ms == 0 || host_ms.abs_diff(here_ms) <= SLACK_MS {
        return pos_ms;
    }
    ((pos_ms as u128 * here_ms as u128) / host_ms as u128).min(here_ms as u128) as u64
}

/// Whether this library has the song with this id.
pub fn has(c: &Connection, id: &str) -> AppResult<bool> {
    Ok(c.query_row("SELECT 1 FROM tracks WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

/// [`best`] over this library's songs.
pub fn find(c: &Connection, song: &Song) -> AppResult<Option<Found>> {
    if song.u {
        return Ok(None);
    }
    let mut cands = Vec::new();
    let row = |r: &rusqlite::Row<'_>| {
        Ok(Cand {
            id: r.get(0)?,
            title: r.get(1)?,
            artist: r.get(2)?,
            track_artist: r.get(3)?,
            duration_ms: r.get::<_, i64>(4)?.max(0) as u64,
            mbid: r.get::<_, Option<String>>(5)?.filter(|m| !m.is_empty()),
        })
    };
    const COLS: &str = "SELECT id, title, artist_title, track_artist, duration_ms, mbid FROM tracks";
    if let Some(mb) = song.mb.as_deref().filter(|m| !m.is_empty()) {
        let mut st = c.prepare_cached(&format!("{COLS} WHERE lower(mbid) = lower(?1)"))?;
        cands.extend(st.query_map([mb], row)?.collect::<Result<Vec<_>, _>>()?);
    }
    if cands.is_empty() && song.n.is_some() && song.a.is_some() {
        let lo = song.d.saturating_sub(SLACK_MS) as i64;
        let hi = song.d.saturating_add(SLACK_MS).min(i64::MAX as u64) as i64;
        let mut st = c.prepare_cached(&format!("{COLS} WHERE duration_ms BETWEEN ?1 AND ?2"))?;
        cands.extend(st.query_map(params![lo, hi], row)?.collect::<Result<Vec<_>, _>>()?);
    }
    Ok(best(song, &cands))
}

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(TEXT_MAX).collect::<String>().trim().to_string()
}

/// Who these songs are, from this library (the host's side). Songs not in
/// the cache are left out. Unreleased songs (the one release rule; a song
/// whose album isn't cached counts as unreleased) carry no MusicBrainz id.
pub fn describe(c: &Connection, ids: &[String]) -> AppResult<HashMap<String, Song>> {
    let mut out = HashMap::new();
    let mut st = c.prepare_cached("SELECT title, coalesce(nullif(track_artist, ''), artist_title), duration_ms, mbid, album_id FROM tracks WHERE id = ?1")?;
    for id in ids {
        let Some((title, artist, d, mbid, album)) = st
            .query_row([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?)))
            .optional()?
        else {
            continue;
        };
        let u = crate::platform::discord::commands::album_unreleased(c, &album)?.unwrap_or(true);
        let mbid = mbid.filter(|m| !m.is_empty() && m.len() <= 64 && m.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
        out.insert(id.clone(), Song { mb: if u { None } else { mbid }, n: Some(clip(&title)), a: Some(clip(&artist)), d: d.max(0) as u64, u });
    }
    Ok(out)
}

/// The song as guests get it: an unreleased one's title and artist go only
/// when the host doesn't hide them.
pub fn shown(mut s: Song, hide_unreleased: bool) -> Song {
    if s.u {
        s.mb = None;
        if hide_unreleased {
            s.n = None;
            s.a = None;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
        schema::migrate(&mut c).unwrap();
        (dir, c)
    }

    /// An album (released unless `demo`) with one song.
    fn song(c: &Connection, id: &str, title: &str, artist: &str, ms: i64, mbid: Option<&str>, demo: bool) {
        let album = format!("al{id}");
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, format, matched)
             VALUES (?1, 'Album', 'Album', 'ar', ?2, 1, 's', ?3, 1)",
            params![album, artist, if demo { "Demo" } else { "Album" }],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src, mbid)
             VALUES (?1, ?2, ?3, 'Album', 'ar', ?4, ?5, '/library/parts/1/file.flac', 's', ?6)",
            params![id, title, album, artist, ms, mbid],
        )
        .unwrap();
    }

    const MB: &str = "0f6a1b2c-3d4e-4f50-8a9b-0c1d2e3f4a5b";

    fn want(mb: Option<&str>, n: &str, a: &str, d: u64) -> Song {
        Song { mb: mb.map(str::to_string), n: Some(n.into()), a: Some(a.into()), d, u: false }
    }

    #[test]
    fn a_copy_is_found_by_musicbrainz_id_then_by_artist_title_and_length() {
        let (_d, c) = db();
        song(&c, "bbbbbbbb:1", "Other Song", "Someone", 300_000, Some(&MB.to_uppercase()), false);
        song(&c, "bbbbbbbb:2", "Night Drive (Remastered)", "The Band", 241_000, None, false);
        song(&c, "bbbbbbbb:3", "night drive remastered", "The Band", 250_000, None, false);
        // By MusicBrainz id, whatever the title says and however long.
        assert_eq!(find(&c, &want(Some(MB), "x", "y", 200_000)).unwrap(), Some(Found { id: "bbbbbbbb:1".into(), duration_ms: 300_000 }));
        // By text: case and punctuation fold, the length is within 3 s.
        assert_eq!(find(&c, &want(None, "NIGHT DRIVE - remastered", "the band", 243_500)).unwrap().map(|f| f.id), Some("bbbbbbbb:2".into()));
        // An unknown MusicBrainz id falls back to the text.
        assert_eq!(find(&c, &want(Some("ffffffff-0000-0000-0000-000000000000"), "Night Drive (Remastered)", "The Band", 250_500)).unwrap().map(|f| f.id), Some("bbbbbbbb:3".into()));
        // Too far apart, another artist, or no title: nothing.
        assert_eq!(find(&c, &want(None, "Night Drive (Remastered)", "The Band", 245_000)).unwrap(), None);
        assert_eq!(find(&c, &want(None, "Night Drive (Remastered)", "Someone Else", 241_000)).unwrap(), None);
        assert_eq!(find(&c, &Song { d: 241_000, ..Default::default() }).unwrap(), None);
        assert!(has(&c, "bbbbbbbb:2").unwrap() && !has(&c, "aaaaaaaa:2").unwrap());
    }

    #[test]
    fn unreleased_songs_are_never_matched() {
        let (_d, c) = db();
        song(&c, "bbbbbbbb:1", "Demo Song", "The Band", 200_000, Some(MB), false);
        let mut s = want(Some(MB), "Demo Song", "The Band", 200_000);
        assert!(find(&c, &s).unwrap().is_some());
        s.u = true;
        assert_eq!(find(&c, &s).unwrap(), None);
        assert_eq!(best(&s, &[Cand { id: "x".into(), title: "Demo Song".into(), artist: "The Band".into(), track_artist: None, duration_ms: 200_000, mbid: Some(MB.into()) }]), None);
    }

    #[test]
    fn the_host_sends_metadata_only_and_can_hide_unreleased_titles() {
        let (_d, c) = db();
        song(&c, "aaaaaaaa:1", "Released", "The Band", 200_000, Some(MB), false);
        song(&c, "aaaaaaaa:2", "From The Vault", "The Band", 180_000, Some(MB), true);
        let got = describe(&c, &["aaaaaaaa:1".into(), "aaaaaaaa:2".into(), "aaaaaaaa:9".into()]).unwrap();
        assert_eq!(got.len(), 2, "songs not in the cache are left out");
        let rel = &got["aaaaaaaa:1"];
        assert_eq!(rel, &Song { mb: Some(MB.into()), n: Some("Released".into()), a: Some("The Band".into()), d: 200_000, u: false });
        let unrel = &got["aaaaaaaa:2"];
        assert!(unrel.u && unrel.mb.is_none());
        let text = serde_json::to_string(&got).unwrap();
        assert!(!text.contains("/library") && !text.contains("file.flac"), "{text}");
        // Shown as is, or without the title and artist when hidden.
        assert_eq!(shown(unrel.clone(), false).n.as_deref(), Some("From The Vault"));
        let hidden = shown(unrel.clone(), true);
        assert_eq!((hidden.n, hidden.a, hidden.u, hidden.d), (None, None, true, 180_000));
        assert_eq!(shown(rel.clone(), true), rel.clone(), "released songs are never hidden");
    }

    #[test]
    fn another_version_plays_from_the_same_share_of_the_song() {
        // Same length (within 3 s): the same spot.
        assert_eq!(map_position(60_000, 200_000, 202_000), 60_000);
        // A longer cut: the same share.
        assert_eq!(map_position(60_000, 200_000, 300_000), 90_000);
        assert_eq!(map_position(150_000, 200_000, 100_000), 75_000);
        // Nothing known: as is.
        assert_eq!(map_position(5_000, 0, 100_000), 5_000);
    }
}
