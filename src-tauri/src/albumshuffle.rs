//! Shuffle albums: whole records in a random order, each one start to
//! finish. The window asks for the songs and plays them as one list with
//! shuffle off, so songs of an album stay in order and gapless.

use std::collections::HashSet;

use rand::seq::SliceRandom;
use rusqlite::{Connection, OptionalExtension};
use tauri::State;

use crate::db::{DbHandle, merge::ServerView, queries};
use crate::error::{AppError, AppResult};
use crate::hidden::HiddenSet;
use crate::tags::TagView;

/// Albums one press plays.
pub const MAX_ALBUMS: usize = 50;
/// The `tracks` context id the window plays them under, and the queue's
/// name for it.
pub const CONTEXT_ID: &str = "albums";
pub const TITLE: &str = "Albums on shuffle";
/// What a press with nothing to play says.
pub const NOTHING: &str = "There are no albums to play here.";

/// Whether an album shows in this tag and server view.
fn visible(c: &Connection, id: &str, view: TagView, server: &ServerView) -> AppResult<bool> {
    let sql = format!(
        "SELECT 1 FROM albums al WHERE al.id = ?1 AND {} AND {}",
        crate::tags::store::visible_clause(view),
        server.clause("al", "album")
    );
    Ok(c.prepare_cached(&sql)?.query_row([id], |_| Ok(())).optional()?.is_some())
}

/// Every album in view, in random order.
fn all_albums(c: &Connection, view: TagView, server: &ServerView) -> AppResult<Vec<String>> {
    let sql = format!(
        "SELECT al.id FROM albums al WHERE {} AND {} ORDER BY random()",
        crate::tags::store::visible_clause(view),
        server.clause("al", "album")
    );
    let mut st = c.prepare(&sql)?;
    Ok(st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

/// The songs to play: `album_ids` (every album in view when empty) in a
/// random order, each in disc and track order, hidden songs and artists
/// left out, at most [`MAX_ALBUMS`] albums with songs left. While only
/// music on this computer can play, `playable` holds what can, and albums
/// with none of it are passed over (the player skips the rest and says so).
pub fn songs(
    c: &Connection,
    album_ids: &[String],
    view: TagView,
    server: &ServerView,
    hidden: &HiddenSet,
    playable: Option<&HashSet<String>>,
    rng: &mut impl rand::Rng,
) -> AppResult<Vec<String>> {
    let order = if album_ids.is_empty() {
        all_albums(c, view, server)?
    } else {
        let mut seen = HashSet::new();
        let mut ids: Vec<String> =
            album_ids.iter().map(|i| crate::sid::resolve(i)).filter(|i| seen.insert(i.clone())).collect();
        ids.shuffle(rng);
        ids
    };
    let mut out = Vec::new();
    let mut albums = 0;
    for id in order {
        if albums == MAX_ALBUMS {
            break;
        }
        if !album_ids.is_empty() && !visible(c, &id, view, server)? {
            continue;
        }
        let mut tracks = queries::album_tracks(c, &id)?;
        tracks.retain(|t| !hidden.hides(&t.id, &t.artist_id));
        crate::tags::store::drop_hidden(c, &mut tracks, view)?;
        if tracks.is_empty() || playable.is_some_and(|ok| !tracks.iter().any(|t| ok.contains(&t.id))) {
            continue;
        }
        albums += 1;
        out.extend(tracks.into_iter().map(|t| t.id));
    }
    Ok(out)
}

/// `album_shuffle_tracks`: the song ids for Shuffle albums.
#[tauri::command]
pub async fn album_shuffle_tracks(
    db: State<'_, DbHandle>,
    hidden: State<'_, crate::hidden::Hidden>,
    backends: State<'_, crate::backend::Backends>,
    cache: State<'_, crate::monitor::cached::AudioCacheDir>,
    downloads: State<'_, crate::downloads::Downloads>,
    album_ids: Vec<String>,
) -> AppResult<Vec<String>> {
    let view = db.tag_view();
    let server = db.server_view();
    let hidden = hidden.ids().get();
    let local = crate::audio::source::LocalFiles(std::sync::Arc::new(downloads.inner().clone()));
    let playable = crate::bridge::OfflinePlay::new(cache.0.clone())
        .with_downloads(local)
        .playable(backends.cached_only())
        .await;
    let ids = db
        .read_async(move |c| songs(c, &album_ids, view, &server, &hidden, playable.as_ref(), &mut rand::rng()))
        .await?;
    if ids.is_empty() {
        return Err(AppError::Invalid(NOTHING.into()));
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rusqlite::params;

    /// Albums a1..a6 by artists x and y, each with three songs `{id}-n`
    /// stored out of order, and one empty album.
    fn db() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&mut c).unwrap();
        for (i, artist) in ["x", "y", "x", "y", "x", "y"].iter().enumerate() {
            let id = format!("a{}", i + 1);
            c.execute(
                "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count) VALUES (?1, ?1, ?1, ?2, ?2, 3)",
                params![id, artist],
            )
            .unwrap();
            for n in [3, 1, 2] {
                c.execute(
                    "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, track_number)
                     VALUES (?1, ?1, ?2, ?2, ?3, ?3, ?4, 1000, ?5)",
                    params![format!("{id}-{n}"), id, artist, format!("/p/{id}-{n}"), n],
                )
                .unwrap();
            }
        }
        c.execute("INSERT INTO albums (id, title, sort_title, artist_id, artist_title) VALUES ('empty', 'e', 'e', 'x', 'x')", [])
            .unwrap();
        c
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn run(c: &Connection, albums: &[String], hidden: &HiddenSet, seed: u64) -> Vec<String> {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        songs(c, albums, TagView::default(), &ServerView::All, hidden, None, &mut rng).unwrap()
    }

    /// The albums in play order, one entry per run of songs.
    fn runs(songs: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in songs {
            let a = s.split('-').next().unwrap().to_string();
            if out.last() != Some(&a) {
                out.push(a);
            }
        }
        out
    }

    #[test]
    fn whole_albums_in_track_order_each_once() {
        let c = db();
        let all = ids(&["a1", "a2", "a3", "a4", "a5", "a6"]);
        let got = run(&c, &all, &HiddenSet::default(), 7);
        let order = runs(&got);
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, all, "each album once, in one piece");
        for a in &order {
            let mine: Vec<&String> = got.iter().filter(|s| s.starts_with(&format!("{a}-"))).collect();
            assert_eq!(mine, [&format!("{a}-1"), &format!("{a}-2"), &format!("{a}-3")]);
        }
    }

    #[test]
    fn the_order_changes_between_presses() {
        let c = db();
        let all = ids(&["a1", "a2", "a3", "a4", "a5", "a6"]);
        let orders: HashSet<Vec<String>> = (0..8).map(|s| runs(&run(&c, &all, &HiddenSet::default(), s))).collect();
        assert!(orders.len() > 1);
    }

    #[test]
    fn no_albums_named_means_every_album_in_view() {
        let c = db();
        let mut order = runs(&run(&c, &[], &HiddenSet::default(), 1));
        order.sort();
        assert_eq!(order, ids(&["a1", "a2", "a3", "a4", "a5", "a6"]), "the empty album is passed over");
    }

    #[test]
    fn hidden_songs_and_artists_stay_out() {
        let c = db();
        let mut hidden = HiddenSet::default();
        hidden.tracks.insert("a1-2".into());
        hidden.artists.insert("y".into());
        let got = run(&c, &ids(&["a1", "a2", "a3"]), &hidden, 3);
        let mut order = runs(&got);
        order.sort();
        assert_eq!(order, ids(&["a1", "a3"]));
        assert!(!got.contains(&"a1-2".to_string()));
    }

    #[test]
    fn unknown_empty_and_repeated_albums_are_passed_over() {
        let c = db();
        let got = run(&c, &ids(&["a2", "nope", "a2", "empty"]), &HiddenSet::default(), 2);
        assert_eq!(got, ids(&["a2-1", "a2-2", "a2-3"]));
        assert!(run(&c, &ids(&["nope", "empty"]), &HiddenSet::default(), 2).is_empty());
    }

    #[test]
    fn one_server_view_keeps_to_its_albums() {
        let c = db();
        let tag = "0a1b2c3d";
        c.execute("UPDATE albums SET id = ?1 || ':' || id WHERE id IN ('a1', 'a2')", [tag]).unwrap();
        c.execute("UPDATE tracks SET album_id = ?1 || ':' || album_id WHERE album_id IN ('a1', 'a2')", [tag]).unwrap();
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        let got = songs(&c, &[], TagView::default(), &ServerView::Server(tag.into()), &HiddenSet::default(), None, &mut rng).unwrap();
        let mut order = runs(&got);
        order.sort();
        assert_eq!(order, ids(&["a1", "a2"]));
    }

    #[test]
    fn at_most_the_cap() {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&mut c).unwrap();
        let mut all = Vec::new();
        for i in 0..(MAX_ALBUMS + 5) {
            let id = format!("b{i}");
            c.execute("INSERT INTO albums (id, title, sort_title, artist_id, artist_title) VALUES (?1, ?1, ?1, 'x', 'x')", [&id])
                .unwrap();
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, track_number)
                 VALUES (?1, ?1, ?2, ?2, 'x', 'x', ?1, 1000, 1)",
                params![format!("{id}-1"), id],
            )
            .unwrap();
            all.push(id);
        }
        assert_eq!(runs(&run(&c, &all, &HiddenSet::default(), 9)).len(), MAX_ALBUMS);
        assert_eq!(runs(&run(&c, &[], &HiddenSet::default(), 9)).len(), MAX_ALBUMS);
    }

    #[test]
    fn offline_plays_only_albums_on_this_computer() {
        let c = db();
        // a2 has one song stored (the player skips the other two and says
        // so); a5 has all three; the rest have none.
        let ok: HashSet<String> = ids(&["a2-3", "a5-1", "a5-2", "a5-3"]).into_iter().collect();
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let got = songs(&c, &[], TagView::default(), &ServerView::All, &HiddenSet::default(), Some(&ok), &mut rng).unwrap();
        let mut order = runs(&got);
        order.sort();
        assert_eq!(order, ids(&["a2", "a5"]));
        // Nothing stored: nothing to play.
        let none = HashSet::new();
        assert!(songs(&c, &[], TagView::default(), &ServerView::All, &HiddenSet::default(), Some(&none), &mut rng).unwrap().is_empty());
    }
}
