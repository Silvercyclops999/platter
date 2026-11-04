use std::sync::Arc;

use super::*;
use crate::db::fake::{self, FakeSource};
use crate::db::source::LibrarySource;

struct Db {
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn open() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    Db { db, _dir: dir }
}

async fn sync(db: &DbHandle, src: &Arc<FakeSource>, full: bool) {
    db.sync_once(src.clone() as Arc<dyn LibrarySource>, full).await.unwrap();
}

fn gone(db: &DbHandle) -> Vec<GoneSong> {
    db.read(|c| list(c, now_s())).unwrap()
}

fn line(db: &DbHandle) -> Safety {
    db.read(|c| safety(c, now_s())).unwrap()
}

#[tokio::test]
async fn songs_that_leave_the_server_are_listed_until_looked_at() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(3, 2, 5)));
    sync(&t.db, &src, false).await;
    assert_eq!(line(&t.db), Safety::default(), "nothing gone yet");

    let (title, album) = {
        let mut l = src.lib.lock();
        let tr = l.tracks.remove(0);
        (tr.title, tr.album_title)
    };
    sync(&t.db, &src, false).await;
    let g = gone(&t.db);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].title, title);
    assert_eq!(g[0].album_title, album);
    assert!(!g[0].seen);
    assert_eq!(line(&t.db), Safety { songs: 1, playlists: 0 });

    t.db.write(|c| mark_seen(c)).unwrap();
    assert_eq!(line(&t.db), Safety::default(), "looked at: the line is fine again");
    assert_eq!(gone(&t.db).len(), 1, "still listed for 30 days");
    assert!(gone(&t.db)[0].seen);
}

#[tokio::test]
async fn a_full_pass_records_what_it_removes() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(3, 2, 5)));
    sync(&t.db, &src, false).await;
    {
        let mut l = src.lib.lock();
        let al = l.albums.remove(0);
        l.tracks.retain(|x| x.album_id != al.id);
    }
    sync(&t.db, &src, true).await;
    assert_eq!(gone(&t.db).len(), 5);
    assert_eq!(line(&t.db).songs, 5);
}

#[tokio::test]
async fn a_song_replaced_by_a_new_copy_is_not_gone() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(2, 1, 3)));
    sync(&t.db, &src, false).await;
    {
        // A better copy: same song and album, new id (what an upgrade does).
        let mut l = src.lib.lock();
        let mut tr = l.tracks.remove(0);
        tr.id = format!("{}9", tr.id);
        tr.part_key = format!("/library/parts/{}/1/file.flac", tr.id);
        l.updated.insert(tr.id.clone(), now_s());
        l.tracks.push(tr);
    }
    sync(&t.db, &src, false).await;
    assert!(gone(&t.db).is_empty(), "the song is still there");
    assert_eq!(line(&t.db), Safety::default());
}

#[tokio::test]
async fn a_held_back_mass_delete_records_nothing() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(12, 5, 20)));
    sync(&t.db, &src, false).await;
    src.lib.lock().tracks.truncate(100);
    sync(&t.db, &src, false).await;
    assert!(gone(&t.db).is_empty(), "a short answer from the server isn't music gone");
    sync(&t.db, &src, false).await;
    assert_eq!(gone(&t.db).len(), 1100, "the same drop twice is real, and listed");
}

#[test]
fn old_rows_are_dropped_after_30_days() {
    let t = open();
    let now = now_s();
    t.db
        .write(|c| {
            c.execute(
                "INSERT INTO tracks_gone (track_id, title, album_id, album_title, artist_title, gone_at) VALUES ('a', 'Old', 'x', 'Al', 'Ar', ?1), ('b', 'New', 'x', 'Al', 'Ar', ?2)",
                params![now - KEEP_S - 10, now - 10],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(gone(&t.db).len(), 1, "the list hides old rows");
    assert_eq!(t.db.write(|c| reconcile(c, now)).unwrap(), 1);
    assert_eq!(t.db.read(|c| Ok(c.query_row("SELECT count(*) FROM tracks_gone", [], |r| r.get::<_, i64>(0))?)).unwrap(), 1);
}

#[test]
fn only_this_weeks_unseen_count_on_the_line() {
    let t = open();
    let now = now_s();
    t.db
        .write(|c| {
            c.execute(
                "INSERT INTO tracks_gone (track_id, title, album_id, album_title, artist_title, gone_at) VALUES ('a', 'Old', 'x', 'Al', 'Ar', ?1), ('b', 'New', 'x', 'Al', 'Ar', ?2)",
                params![now - FRESH_S - 10, now - 10],
            )?;
            c.execute(
                "INSERT INTO playlist_trash (playlist_id, title, track_ids, gone, deleted_at) VALUES ('p1', 'Road', '[\"1\"]', 1, ?1), ('p2', 'Mine', '[\"1\"]', 0, ?1)",
                [now - 10],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(line(&t.db), Safety { songs: 1, playlists: 1 }, "a playlist deleted here isn't news");
    t.db.write(|c| mark_seen(c)).unwrap();
    assert_eq!(line(&t.db), Safety::default());
}

#[test]
fn a_new_library_forgets_what_the_old_one_lost() {
    let t = open();
    t.db
        .write(|c| {
            c.execute("INSERT INTO tracks_gone (track_id, title, album_id, album_title, artist_title, gone_at) VALUES ('a', 'X', 'x', 'Al', 'Ar', 1)", [])?;
            let tx = c.transaction()?;
            crate::db::store::wipe_library(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    assert_eq!(t.db.read(|c| Ok(c.query_row("SELECT count(*) FROM tracks_gone", [], |r| r.get::<_, i64>(0))?)).unwrap(), 0);
}
