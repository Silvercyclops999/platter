//! The library side: who songs are (to send), finding them here (from
//! what others sent), and the plays a taste summary is made from.
//!
//! What goes out is metadata only: title, artist, album, length,
//! MusicBrainz ids and the song's server-qualified id. Never a part key,
//! a path or anything else from the file.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use super::wire::Ident;
use crate::db::merge::norm;
use crate::error::AppResult;
use crate::social::listen::matching;

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(matching::TEXT_MAX).collect::<String>().trim().to_string()
}

fn mbid(s: Option<String>) -> Option<String> {
    s.filter(|m| !m.is_empty() && m.len() <= 64 && m.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
}

/// Who these songs are. Songs not in the cache are left out. An unreleased
/// song (the one release rule; an album that isn't cached counts as
/// unreleased) carries no MusicBrainz ids.
pub fn describe(c: &Connection, ids: &[String]) -> AppResult<HashMap<String, Ident>> {
    let mut out = HashMap::new();
    let mut st = c.prepare_cached(
        "SELECT t.title, coalesce(nullif(t.track_artist, ''), t.artist_title), t.album_title, t.duration_ms, t.mbid, t.album_id, al.mbid
         FROM tracks t LEFT JOIN albums al ON al.id = t.album_id WHERE t.id = ?1",
    )?;
    for id in ids {
        let Some((title, artist, album, d, mb, album_id, rg)) = st
            .query_row([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            })
            .optional()?
        else {
            continue;
        };
        let u = crate::platform::discord::commands::album_unreleased(c, &album_id)?.unwrap_or(true);
        out.insert(
            id.clone(),
            Ident {
                n: clip(&title),
                a: clip(&artist),
                al: Some(clip(&album)).filter(|a| !a.is_empty()),
                d: d.max(0) as u64,
                mb: if u { None } else { mbid(mb) },
                rg: if u { None } else { mbid(rg) },
                k: Some(id.clone()).filter(|k| crate::sid::tag_of(k).is_some() && k.len() <= 64),
                u,
            },
        );
    }
    Ok(out)
}

/// The copy here that plays as `s`: the sender's own copy when this
/// library has it (the same server), else the same song on any server here
/// by the multi-server rules. An unreleased song only as the sender's copy.
pub fn resolve(c: &Connection, s: &Ident) -> AppResult<Option<String>> {
    if let Some(k) = s.k.as_deref()
        && crate::sid::tag_of(k).is_some()
        && matching::has(c, k)?
    {
        return Ok(Some(k.to_string()));
    }
    Ok(matching::find(c, &s.song())?.map(|f| f.id))
}

/// Songs here by an artist (by MusicBrainz artist id when the cache has
/// one, else the name folded), most played and liked first.
pub fn artist_tracks(c: &Connection, name: &str, limit: usize) -> AppResult<Vec<String>> {
    let want = norm(name);
    if want.is_empty() {
        return Ok(Vec::new());
    }
    let mut st = c.prepare_cached(
        "SELECT id, artist_title, track_artist FROM tracks
         WHERE lower(artist_title) = lower(?1) OR lower(track_artist) = lower(?1)
            OR artist_id IN (SELECT id FROM artists WHERE lower(title) = lower(?1))
         ORDER BY view_count DESC, coalesce(user_rating, 0) DESC, pk LIMIT ?2",
    )?;
    let rows = st
        .query_map(params![name.trim(), (limit * 3) as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, a, ta)| norm(a) == want || ta.as_deref().is_some_and(|t| norm(t) == want))
        .map(|(id, _, _)| id)
        .take(limit)
        .collect())
}

/// One play from this computer's own listen log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Play {
    pub track_id: String,
    /// Unix seconds.
    pub at: i64,
}

/// Plays on this computer since `since_s` that count (at least half the
/// song or 30 s, not skipped), from the local listen log only: a private
/// session writes nothing there, so its plays never count.
pub fn plays(c: &Connection, since_s: i64) -> AppResult<Vec<Play>> {
    let mut st = c.prepare_cached(
        "SELECT rating_key, started_at FROM listen_log
         WHERE started_at >= ?1 AND skipped = 0 AND (completed = 1 OR ms_listened >= 30000 OR ms_listened * 2 >= duration_ms)
         ORDER BY started_at",
    )?;
    let since_ms = since_s.saturating_mul(1000);
    let rows = st.query_map([since_ms], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    // started_at is ms in the log.
    Ok(rows.into_iter().map(|(track_id, at)| Play { track_id, at: at / 1000 }).collect())
}

/// Liked songs, newest like first.
pub fn favourites(c: &Connection, liked_min: f64, limit: usize) -> AppResult<Vec<String>> {
    let mut ids = crate::db::likes::liked_ids(c, liked_min)?;
    ids.truncate(limit);
    Ok(ids)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::schema;

    pub fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
        schema::migrate(&mut c).unwrap();
        (dir, c)
    }

    /// A song on its own album (released unless `demo`).
    pub fn song(c: &Connection, id: &str, title: &str, artist: &str, ms: i64, mbid: Option<&str>, demo: bool) {
        let album = format!("al{id}");
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, format, matched, mbid)
             VALUES (?1, 'Album', 'Album', 'ar', ?2, 1, 's', ?3, 1, '11111111-2222-3333-4444-555555555555')",
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

    #[test]
    fn songs_go_out_as_metadata_only() {
        let (_d, c) = db();
        song(&c, "aaaaaaaa:1", "Night Drive", "The Band", 200_000, Some(MB), false);
        song(&c, "aaaaaaaa:2", "Vault Take", "The Band", 180_000, Some(MB), true);
        let got = describe(&c, &["aaaaaaaa:1".into(), "aaaaaaaa:2".into(), "aaaaaaaa:9".into()]).unwrap();
        assert_eq!(got.len(), 2);
        let s = &got["aaaaaaaa:1"];
        assert_eq!((s.n.as_str(), s.a.as_str(), s.al.as_deref(), s.d), ("Night Drive", "The Band", Some("Album"), 200_000));
        assert_eq!((s.mb.as_deref(), s.rg.as_deref(), s.k.as_deref()), (Some(MB), Some("11111111-2222-3333-4444-555555555555"), Some("aaaaaaaa:1")));
        let u = &got["aaaaaaaa:2"];
        assert!(u.u && u.mb.is_none() && u.rg.is_none());
        let text = serde_json::to_string(&got).unwrap();
        assert!(!text.contains("/library") && !text.contains("file.flac"), "{text}");
    }

    #[test]
    fn a_song_resolves_to_the_same_copy_or_another_server_s() {
        let (_d, c) = db();
        song(&c, "bbbbbbbb:7", "Night Drive", "The Band", 201_000, None, false);
        let mut s = Ident { n: "night drive".into(), a: "the band".into(), d: 200_000, ..Default::default() };
        assert_eq!(resolve(&c, &s).unwrap().as_deref(), Some("bbbbbbbb:7"));
        // The sender's own copy, when it's here.
        song(&c, "aaaaaaaa:1", "Night Drive", "The Band", 200_000, None, false);
        s.k = Some("aaaaaaaa:1".into());
        assert_eq!(resolve(&c, &s).unwrap().as_deref(), Some("aaaaaaaa:1"));
        // Unreleased: only the sender's own copy.
        s.u = true;
        s.k = Some("cccccccc:1".into());
        assert_eq!(resolve(&c, &s).unwrap(), None);
        s.k = Some("aaaaaaaa:1".into());
        assert_eq!(resolve(&c, &s).unwrap().as_deref(), Some("aaaaaaaa:1"));
    }

    #[test]
    fn an_artist_s_songs_most_played_first() {
        let (_d, c) = db();
        song(&c, "aaaaaaaa:1", "One", "The Band", 200_000, None, false);
        song(&c, "aaaaaaaa:2", "Two", "The Band", 200_000, None, false);
        song(&c, "aaaaaaaa:3", "Three", "Someone", 200_000, None, false);
        c.execute("UPDATE tracks SET view_count = 9 WHERE id = 'aaaaaaaa:2'", []).unwrap();
        assert_eq!(artist_tracks(&c, "the band", 5).unwrap(), vec!["aaaaaaaa:2".to_string(), "aaaaaaaa:1".into()]);
        assert!(artist_tracks(&c, "", 5).unwrap().is_empty());
    }

    #[test]
    fn plays_that_count_come_from_the_local_log() {
        let (_d, c) = db();
        let ins = |k: &str, at: i64, ms: i64, skipped: bool| {
            c.execute(
                "INSERT INTO listen_log (rating_key, started_at, duration_ms, ms_listened, skipped) VALUES (?1, ?2, 200000, ?3, ?4)",
                params![k, at * 1000, ms, skipped],
            )
            .unwrap();
        };
        ins("aaaaaaaa:1", 1000, 150_000, false);
        ins("aaaaaaaa:2", 1000, 5_000, false);
        ins("aaaaaaaa:3", 1000, 150_000, true);
        ins("aaaaaaaa:4", 10, 150_000, false);
        assert_eq!(plays(&c, 500).unwrap(), vec![Play { track_id: "aaaaaaaa:1".into(), at: 1000 }]);
    }
}
