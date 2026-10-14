//! The Recents page: what was played on this device, as listening sessions.
//!
//! Built from the local listen log. A session is a run of plays from the
//! same context (album, artist, playlist, Liked Songs, or loose songs) with
//! gaps under [`SESSION_GAP_MS`] between one play's end and the next one's
//! start. Coming back to the same album later starts a new session. Skips
//! (under 30 s) aren't plays and are left out.
//!
//! Likes show in the same timeline as "N songs added to Liked Songs", from
//! each liked track's rating time (Plex `lastRatedAt`), grouped the same way.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::error::AppResult;
use crate::model::Track;

use super::store::{ALBUM_COLS, ARTIST_COLS, PLAYLIST_COLS, TRACK_COLS, row_album, row_artist, row_playlist, row_track};
use super::types::HomeItem;

/// Longest pause inside one listening session.
pub const SESSION_GAP_MS: i64 = 30 * 60 * 1000;
/// How far back the page goes.
pub const HISTORY_DAYS: i64 = 90;
/// Most entries the page lists.
pub const HISTORY_MAX: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecentsKind {
    /// Played from an album, artist or playlist (`item` says which).
    Context,
    /// Played from Liked Songs.
    Liked,
    /// Songs played from no single context (search, a song's menu, radio).
    Songs,
    /// Songs added to Liked Songs.
    LikedAdded,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentsEntry {
    pub kind: RecentsKind,
    /// The album, artist or playlist, for `Context`.
    pub item: Option<HomeItem>,
    /// Newest play (or like) in the entry, unix ms.
    pub at: i64,
    /// The songs, newest first. Repeats stay.
    pub tracks: Vec<Track>,
}

/// One play as the grouping sees it.
#[derive(Debug, Clone, PartialEq)]
struct Play {
    key: (String, Option<String>),
    started: i64,
    ended: i64,
    track: Track,
}

fn context_item(c: &Connection, kind: &str, id: &str) -> AppResult<Option<HomeItem>> {
    Ok(match kind {
        "album" => c
            .prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"))?
            .query_row([id], row_album)
            .optional()?
            .map(|album| HomeItem::Album { album }),
        "artist" => c
            .prepare_cached(&format!("SELECT {ARTIST_COLS} FROM artists ar WHERE ar.id = ?1"))?
            .query_row([id], row_artist)
            .optional()?
            .map(|artist| HomeItem::Artist { artist }),
        "playlist" => c
            .prepare_cached(&format!("SELECT {PLAYLIST_COLS} FROM playlists p WHERE p.id = ?1"))?
            .query_row([id], row_playlist)
            .optional()?
            .map(|playlist| HomeItem::Playlist { playlist }),
        _ => None,
    })
}

/// Group time-ordered (oldest first) events into runs: same key and a gap
/// under [`SESSION_GAP_MS`]. Returns the runs oldest first.
fn sessions<T, K: PartialEq>(events: Vec<T>, key: impl Fn(&T) -> K, span: impl Fn(&T) -> (i64, i64)) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    let mut last_end = i64::MIN;
    for e in events {
        let (start, end) = span(&e);
        let joins = out
            .last()
            .and_then(|run| run.last())
            .is_some_and(|prev| key(prev) == key(&e) && start - last_end < SESSION_GAP_MS);
        if joins {
            out.last_mut().unwrap().push(e);
        } else {
            out.push(vec![e]);
        }
        last_end = if joins { last_end.max(end) } else { end.max(start) };
    }
    out
}

/// Recents entries since `since_ms`, newest first, at most `limit`.
/// `liked_min` is the rating that counts as liked.
pub fn recents(c: &Connection, since_ms: i64, liked_min: f64, limit: usize) -> AppResult<Vec<RecentsEntry>> {
    let mut items: HashMap<(String, String), Option<HomeItem>> = HashMap::new();
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS}, l.started_at, coalesce(l.ended_at, l.started_at + l.duration_ms),
                l.context_kind, l.context_id
         FROM listen_log l JOIN tracks t ON t.id = l.rating_key
         WHERE l.started_at >= ?1 AND l.skipped = 0
         ORDER BY l.started_at, l.id"
    ))?;
    let n = super::store::TRACK_NCOLS;
    let rows = st
        .query_map([since_ms], |r| {
            Ok((
                row_track(r)?,
                r.get::<_, i64>(n)?,
                r.get::<_, i64>(n + 1)?,
                r.get::<_, Option<String>>(n + 2)?,
                r.get::<_, Option<String>>(n + 3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut plays = Vec::with_capacity(rows.len());
    for (track, started, ended, kind, id) in rows {
        let key = match (kind.as_deref(), id) {
            (Some("liked"), _) => ("liked".to_string(), None),
            (Some(k @ ("album" | "artist" | "playlist")), Some(id)) => {
                let found = match items.get(&(k.to_string(), id.clone())) {
                    Some(v) => v.is_some(),
                    None => {
                        let item = context_item(c, k, &id)?;
                        let ok = item.is_some();
                        items.insert((k.to_string(), id.clone()), item);
                        ok
                    }
                };
                // A context no longer in the library shows as loose songs.
                if found { (k.to_string(), Some(id)) } else { ("songs".to_string(), None) }
            }
            _ => ("songs".to_string(), None),
        };
        plays.push(Play { key, started, ended, track });
    }

    let mut out = Vec::new();
    for run in sessions(plays, |p| p.key.clone(), |p| (p.started, p.ended)) {
        let (kind, id) = run[0].key.clone();
        let at = run.iter().map(|p| p.started).max().unwrap_or(0);
        let tracks = run.into_iter().rev().map(|p| p.track).collect();
        let (kind, item) = match (kind.as_str(), id) {
            ("liked", _) => (RecentsKind::Liked, None),
            ("songs", _) | (_, None) => (RecentsKind::Songs, None),
            (k, Some(id)) => (RecentsKind::Context, items.get(&(k.to_string(), id)).cloned().flatten()),
        };
        out.push(RecentsEntry { kind, item, at, tracks });
    }

    // Likes: rating times are unix seconds.
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS}, t.rated_at FROM tracks t
         WHERE t.user_rating >= ?1 AND t.rated_at IS NOT NULL AND t.rated_at * 1000 >= ?2
         ORDER BY t.rated_at, t.pk"
    ))?;
    let likes = st
        .query_map(rusqlite::params![liked_min, since_ms], |r| Ok((row_track(r)?, r.get::<_, i64>(n)? * 1000)))?
        .collect::<Result<Vec<_>, _>>()?;
    for run in sessions(likes, |_| (), |(_, at)| (*at, *at)) {
        let at = run.iter().map(|(_, at)| *at).max().unwrap_or(0);
        let tracks = run.into_iter().rev().map(|(t, _)| t).collect();
        out.push(RecentsEntry { kind: RecentsKind::LikedAdded, item: None, at, tracks });
    }

    out.sort_by_key(|e| std::cmp::Reverse(e.at));
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> (String, Option<String>) {
        (s.to_string(), None)
    }

    #[test]
    fn runs_split_on_context_change_and_long_gaps() {
        let min = 60_000;
        let ev = vec![
            (k("a"), 0, 3 * min),
            (k("a"), 3 * min, 6 * min),
            (k("b"), 7 * min, 10 * min),
            (k("b"), 10 * min + SESSION_GAP_MS - 1, 13 * min + SESSION_GAP_MS),
            (k("b"), 13 * min + 2 * SESSION_GAP_MS + 1, 16 * min + 2 * SESSION_GAP_MS),
            (k("a"), 17 * min + 2 * SESSION_GAP_MS, 20 * min + 2 * SESSION_GAP_MS),
        ];
        let runs = sessions(ev, |e| e.0.clone(), |e| (e.1, e.2));
        let sizes: Vec<_> = runs.iter().map(|r| (r[0].0.0.clone(), r.len())).collect();
        assert_eq!(
            sizes,
            vec![("a".into(), 2), ("b".into(), 2), ("b".into(), 1), ("a".into(), 1)],
            "a pause just under the gap stays in, a longer one starts over, and coming back to a is new",
        );
    }
}
