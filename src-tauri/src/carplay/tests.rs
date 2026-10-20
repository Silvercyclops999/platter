use std::sync::Arc;

use super::*;
use crate::db::DbHandle;
use crate::db::fake::{self, FakeSource};
use crate::db::source::LibrarySource;

async fn synced(artists: usize, albums_per: usize) -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    let src = Arc::new(FakeSource::new(fake::generated(artists, albums_per, 3)));
    db.sync_once(src as Arc<dyn LibrarySource>, true).await.unwrap();
    (db, dir)
}

#[tokio::test]
async fn library_lists_play_what_they_show_and_stay_short() {
    let (db, _dir) = synced(30, 5).await;
    let albums = db.read(|c| list(c, &CarList::Albums)).unwrap();
    assert_eq!(albums.len(), LIST_MAX, "150 albums are cut to the car's limit");
    for a in &albums {
        assert!(!a.title.is_empty());
        assert!(a.subtitle.as_deref().is_some_and(|s| !s.is_empty()), "albums say who made them");
        assert!(matches!(&a.action, CarAction::Play(p) if p.kind == ContextKind::Album && p.id.is_some()));
    }
    let artists = db.read(|c| list(c, &CarList::Artists)).unwrap();
    assert_eq!(artists.len(), 30);
    assert!(artists.iter().all(|a| matches!(&a.action, CarAction::Open(CarList::Artist { id, name }) if !id.is_empty() && *name == a.title)));
    let added = db.read(|c| list(c, &CarList::RecentlyAdded)).unwrap();
    assert_eq!(added.len(), LIST_MAX);
}

#[tokio::test]
async fn an_empty_library_gives_empty_lists_not_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    for which in [CarList::RecentlyPlayed, CarList::RecentlyAdded, CarList::Playlists, CarList::Albums, CarList::Artists] {
        assert!(db.read(|c| list(c, &which)).unwrap().is_empty(), "{which:?}");
    }
    // With no songs there's nothing to shuffle; likes and the shelves stay.
    let home = db.read(home).unwrap();
    let titles: Vec<_> = home.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Liked songs", "Recently played", "Recently added"]);
}

#[tokio::test]
async fn home_opens_on_shuffle_and_likes_then_what_you_played_then_mixes() {
    let (db, _dir) = synced(4, 3).await;
    db.write(|c| {
        c.execute_batch("UPDATE albums SET last_viewed_at = 1000 + rowid, view_count = 1")?;
        Ok(())
    })
    .unwrap();
    let sections = db.read(home_sections).unwrap();
    let CarRow::Item(shuffle) = &sections[0].rows[0] else { panic!("a row first") };
    assert_eq!(shuffle.title, "Shuffle my library");
    assert_eq!(shuffle.action, CarAction::ShuffleLibrary, "the songs are drawn when it's picked");
    let CarRow::Item(liked) = &sections[0].rows[1] else { panic!("a row second") };
    assert_eq!(liked.title, "Liked songs");
    let ctx = db.read(shuffle_library_context).unwrap();
    assert_eq!(ctx.track_ids.as_ref().map(Vec::len), Some(4 * 3 * 3), "every song (under the cap)");
    let CarRow::Covers { title, items } = &sections[1].rows[0] else { panic!("covers second: {:?}", sections[1]) };
    assert_eq!(title, "Jump back in");
    assert!(!items.is_empty() && items.len() <= COVERS_MAX);
    assert!(items.iter().all(|i| matches!(&i.action, CarAction::Play(_))), "a cover plays in one tap");
    assert_eq!(sections.last().unwrap().header.as_deref(), Some("More"));
}

#[tokio::test]
async fn shuffle_my_library_starts_with_at_most_the_cap() {
    let (db, _dir) = synced(40, 5).await;
    let ctx = db.read(shuffle_library_context).unwrap();
    assert_eq!(ctx.track_ids.as_ref().map(Vec::len), Some(SHUFFLE_MAX));
}

#[test]
fn library_tab_starts_with_liked_songs_then_the_shelves() {
    let tab = library();
    assert_eq!(tab[0].title, "Liked songs");
    assert!(matches!(&tab[0].action, CarAction::Play(p) if p.kind == ContextKind::Liked));
    let opens: Vec<_> = tab[1..].iter().map(|i| i.title.as_str()).collect();
    assert_eq!(opens, ["Playlists", "Albums", "Artists"]);
}

#[tokio::test]
async fn radio_offers_stations_from_whats_playing_then_albums() {
    let (db, _dir) = synced(3, 2).await;
    let none = db.read(|c| radio(c, None)).unwrap();
    assert_eq!(none.len(), 1, "a random album only, while nothing plays (nothing is old yet)");
    assert_eq!(none[0].title, "A random album");
    let playing = Playing { track_id: "t1".into(), title: "Song".into(), artist_id: "a1".into(), artist: "Band".into() };
    let with = db.read(|c| radio(c, Some(&playing))).unwrap();
    assert_eq!(with[0].title, "Radio from Song");
    assert!(matches!(&with[0].action, CarAction::Play(p) if p.kind == ContextKind::Radio && p.id.as_deref() == Some("track:t1")));
    assert!(matches!(&with[1].action, CarAction::Play(p) if p.id.as_deref() == Some("artist:a1")));
}

#[tokio::test]
async fn downloaded_is_empty_with_nothing_kept() {
    let (db, _dir) = synced(2, 2).await;
    assert!(db.read(downloaded).unwrap().is_empty());
}

#[tokio::test]
async fn an_artist_opens_popular_then_their_albums() {
    let (db, _dir) = synced(3, 4).await;
    let artists = db.read(|c| list(c, &CarList::Artists)).unwrap();
    let CarAction::Open(which) = &artists[0].action else { panic!("artists open a list") };
    let CarList::Artist { id, .. } = which else { panic!("an artist's list") };
    assert_eq!(which.title(), artists[0].title, "the list is titled with the artist's name");
    let rows = db.read(|c| list(c, which)).unwrap();
    assert_eq!(rows.len(), 5, "Popular and four albums");
    assert_eq!(rows[0].title, "Popular");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.kind == ContextKind::Artist && p.id.as_ref() == Some(id)));
    let own = db.read(|c| queries::artist_albums(c, id, LIST_MAX)).unwrap();
    for (row, album) in rows[1..].iter().zip(&own) {
        assert_eq!(row.title, album.title);
        assert!(matches!(&row.action, CarAction::Play(p) if p.kind == ContextKind::Album && p.id.as_ref() == Some(&album.id)));
    }
}

#[tokio::test]
async fn an_artist_list_stays_short_and_an_unknown_artist_is_just_popular() {
    let (db, _dir) = synced(1, 150).await;
    let id = db.read(|c| list(c, &CarList::Artists)).unwrap().into_iter().find_map(|a| match a.action {
        CarAction::Open(CarList::Artist { id, .. }) => Some(id),
        _ => None,
    });
    let rows = db.read(|c| list(c, &CarList::Artist { id: id.unwrap(), name: "A".into() })).unwrap();
    assert_eq!(rows.len(), LIST_MAX, "Popular plus albums up to the car's limit");
    let none = db.read(|c| list(c, &CarList::Artist { id: "nobody".into(), name: "Nobody".into() })).unwrap();
    assert_eq!(none.len(), 1);
}

fn health(status: crate::plex::types::HealthStatus, cached_only: bool) -> crate::plex::types::Health {
    crate::plex::types::Health {
        status,
        server_name: None,
        connection: None,
        latency_ms: None,
        server_version: None,
        last_seen_at: None,
        scan: None,
        banner: None,
        files_available: true,
        cached_only,
    }
}

#[test]
fn reach_follows_sign_in_and_the_server() {
    use crate::plex::types::HealthStatus as H;
    assert_eq!(reach(&health(H::NoAuth, false), false), Reach::SignedOut);
    assert_eq!(reach(&health(H::NoAuth, false), true), Reach::Online, "another server is set up");
    assert_eq!(reach(&health(H::NoAuth, true), false), Reach::Unreachable, "the server signed us out: downloads still play");
    assert_eq!(reach(&health(H::Offline, true), false), Reach::Unreachable);
    assert_eq!(reach(&health(H::Local, false), false), Reach::Online);
    assert_eq!(reach(&health(H::Starting, false), false), Reach::Online);
}

#[test]
fn signed_out_every_tab_says_to_sign_in() {
    for tab in [Tab::Home, Tab::Library, Tab::Downloaded, Tab::Radio] {
        let rows = tab_rows(tab, Reach::SignedOut, || panic!("nothing is read while signed out"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Open Platter on your phone to sign in");
        assert_eq!(rows[0].action, CarAction::Info);
    }
}

#[test]
fn out_of_reach_only_downloaded_lists_its_rows() {
    let kept = || library();
    for tab in [Tab::Home, Tab::Library, Tab::Radio] {
        let rows = tab_rows(tab, Reach::Unreachable, || panic!("{tab:?} is not read"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, UNREACHABLE);
        assert_eq!(rows[0].action, CarAction::Info);
    }
    assert_eq!(tab_rows(Tab::Downloaded, Reach::Unreachable, kept), library());
    assert_eq!(tab_rows(Tab::Home, Reach::Online, kept), library());
}

fn queued(id: &str, title: &str) -> crate::audio::queue::QueueItem {
    let mut track = fake::generated(1, 1, 1).tracks.remove(0);
    track.title = title.into();
    track.track_artist = None;
    crate::audio::queue::QueueItem { item_id: id.into(), track, source: crate::audio::queue::ItemSource::Context }
}

#[test]
fn up_next_lists_the_hand_queued_songs_first_and_jumps() {
    let q = crate::audio::queue::QueueView {
        current: Some(queued("now", "Playing")),
        user_queue: vec![queued("u1", "Queued")],
        next_up: (0..150).map(|i| queued(&format!("n{i}"), &format!("Next {i}"))).collect(),
        history: vec![queued("h", "Earlier")],
        context: None,
    };
    let rows = up_next(&q);
    assert_eq!(rows.len(), LIST_MAX);
    assert_eq!(rows[0].title, "Queued");
    assert_eq!(rows[0].action, CarAction::Jump("u1".into()));
    assert_eq!(rows[1].action, CarAction::Jump("n0".into()));
    assert!(rows[0].subtitle.as_deref().is_some_and(|s| !s.is_empty()), "rows say who it's by");
    assert!(up_next(&crate::audio::queue::QueueView::default()).is_empty());
}

#[test]
fn repeat_button_cycles_off_all_one() {
    use crate::audio::queue::Repeat;
    assert_eq!(next_repeat(Repeat::Off), Repeat::All);
    assert_eq!(next_repeat(Repeat::All), Repeat::One);
    assert_eq!(next_repeat(Repeat::One), Repeat::Off);
}

#[test]
fn rows_playing_what_plays_now_are_marked() {
    let album = |id: &str| CarItem { title: "A".into(), subtitle: None, thumb: None, action: play(ContextKind::Album, id) };
    let playlist = CarItem { title: "P".into(), subtitle: None, thumb: None, action: play(ContextKind::Playlist, "p1") };
    let now = NowKey { context: Some((ContextKind::Playlist, "p1".into())), album_id: Some("al9".into()) };
    assert!(is_playing(&playlist, &now), "the playing playlist");
    assert!(is_playing(&album("al9"), &now), "the album the playing song is on");
    assert!(!is_playing(&album("al1"), &now));
    // The same id under another kind isn't it.
    let mix = CarItem { title: "M".into(), subtitle: None, thumb: None, action: play(ContextKind::Mix, "p1") };
    assert!(!is_playing(&mix, &now));
    // Lists to open, and nothing playing, mark nothing.
    assert!(!is_playing(&open(CarList::Albums), &now));
    assert!(!is_playing(&playlist, &NowKey::default()));
}

#[tokio::test]
async fn home_lists_the_mixes_and_leaves_artists_out_of_the_covers() {
    let (db, _dir) = synced(4, 3).await;
    db.write(|c| {
        // An artist and an album played lately; only the album is a cover.
        c.execute_batch("UPDATE albums SET last_viewed_at = 1000 + rowid, view_count = 1")?;
        let ids: Vec<String> = c
            .prepare("SELECT id FROM tracks LIMIT 30")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mix = crate::mixes::store::Stored {
            id: "daily-1".into(),
            kind: crate::mixes::MixKind::Daily,
            title: "Daily Mix 1".into(),
            description: "Some artists and more".into(),
            color: "#334455".into(),
            artist_ids: vec![],
            tag: None,
            tracks: ids.into_iter().map(|id| (id, false)).collect(),
        };
        crate::mixes::store::save_one(c, &mix, 0, "2026-10-20", 0)?;
        Ok(())
    })
    .unwrap();
    let sections = db.read(home_sections).unwrap();
    let made = sections.iter().find(|s| s.header.as_deref() == Some("Made for you")).expect("a mixes section");
    let CarRow::Item(m) = &made.rows[0] else { panic!("a mix row") };
    assert_eq!(m.title, "Daily Mix 1");
    assert!(matches!(&m.action, CarAction::Play(p) if p.kind == ContextKind::Mix && p.id.as_deref() == Some("daily-1")));
    let CarRow::Covers { items, .. } = &sections[1].rows[0] else { panic!("covers") };
    assert!(items.iter().all(|i| !matches!(i.action, CarAction::Open(CarList::Artist { .. }))), "no artists among the covers");
}

#[test]
fn home_out_of_reach_or_signed_out_says_so() {
    let signed_out = tab_rows(Tab::Home, Reach::SignedOut, Vec::new);
    assert_eq!(signed_out[0].title, SIGN_IN);
    let away = tab_rows(Tab::Home, Reach::Unreachable, Vec::new);
    assert_eq!(away[0].title, UNREACHABLE);
}

#[test]
fn release_lines_say_year_and_kind() {
    assert_eq!(release_line(Some(2011), Some("album")).as_deref(), Some("2011"));
    assert_eq!(release_line(Some(2019), Some("ep")).as_deref(), Some("EP · 2019"));
    assert_eq!(release_line(None, Some("single")).as_deref(), Some("Single"));
    assert_eq!(release_line(None, None), None);
}

#[test]
fn covers_crop_to_a_centred_whole_pixel_square() {
    assert_eq!(square_crop(600, 600), None);
    assert_eq!(square_crop(0, 10), None);
    assert_eq!(square_crop(800, 600), Some((100, 0, 600)));
    assert_eq!(square_crop(600, 901), Some((0, 150, 600)), "odd sizes stay square");
}


fn connect(status: crate::audio::events::PlayerStatus) -> Connect {
    Connect {
        resume_on: true,
        reach: Reach::Online,
        status,
        track: Some(("t1".into(), false)),
        since_disconnect_s: None,
    }
}

#[test]
fn a_car_connecting_plays_on_where_you_left_off() {
    use crate::audio::events::PlayerStatus;
    assert_eq!(on_connect(&connect(PlayerStatus::Paused)), OnConnect::Resume);
    assert_eq!(on_connect(&connect(PlayerStatus::Idle)), OnConnect::Resume, "a fresh start with the saved queue");
    for busy in [PlayerStatus::Playing, PlayerStatus::Loading, PlayerStatus::Buffering, PlayerStatus::Error] {
        assert_eq!(on_connect(&connect(busy)), OnConnect::Nothing, "{busy:?}");
    }
    let off = Connect { resume_on: false, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&off), OnConnect::Nothing, "the setting is off");
    let empty = Connect { track: None, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&empty), OnConnect::Nothing, "nothing new is started");
    let out = Connect { reach: Reach::SignedOut, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&out), OnConnect::Nothing);
}

#[test]
fn a_car_that_only_blinked_changes_nothing() {
    use crate::audio::events::PlayerStatus;
    let blink = Connect { since_disconnect_s: Some(RECONNECT_GRACE_S - 1), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&blink), OnConnect::Nothing, "paused in the car, the cable wobbled");
    let later = Connect { since_disconnect_s: Some(RECONNECT_GRACE_S), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&later), OnConnect::Resume, "a new drive");
}

#[test]
fn the_blink_line_is_thirty_seconds() {
    use crate::audio::events::PlayerStatus;
    assert_eq!(RECONNECT_GRACE_S, 30);
    let at = |s| on_connect(&Connect { since_disconnect_s: Some(s), ..connect(PlayerStatus::Paused) });
    assert_eq!(at(29), OnConnect::Nothing);
    assert_eq!(at(30), OnConnect::Resume);
    assert_eq!(at(0), OnConnect::Nothing);
}

#[test]
fn time_since_the_car_left_counts_the_phones_sleep() {
    assert_eq!(since_left(5, Some(5)), 5, "a blink on both clocks");
    assert_eq!(since_left(5, Some(8 * 3600)), 8 * 3600, "a short drive, then a night asleep");
    assert_eq!(since_left(40, Some(3)), 40, "the wall clock set back a little");
    assert_eq!(since_left(5, None), u64::MAX, "a wall clock that went backwards resumes");
    use crate::audio::events::PlayerStatus;
    let slept = Connect { since_disconnect_s: Some(since_left(5, Some(8 * 3600))), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&slept), OnConnect::Resume);
}

#[test]
fn signed_out_never_resumes_even_a_kept_song() {
    use crate::audio::events::PlayerStatus;
    let c = Connect { reach: Reach::SignedOut, track: Some(("t1".into(), true)), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&c), OnConnect::Nothing);
    let idle = Connect { status: PlayerStatus::Idle, ..c };
    assert_eq!(on_connect(&idle), OnConnect::Nothing);
}

#[test]
fn out_of_reach_only_a_kept_song_resumes() {
    use crate::audio::events::PlayerStatus;
    let streamed = Connect { reach: Reach::Unreachable, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&streamed), OnConnect::Nothing, "it would only fail");
    let kept = Connect { track: Some(("t1".into(), true)), ..streamed };
    assert_eq!(on_connect(&kept), OnConnect::Resume);
}

fn row(title: &str, action: CarAction) -> CarItem {
    CarItem { title: title.into(), subtitle: None, thumb: None, action }
}

fn ctx(kind: ContextKind, id: &str) -> CarAction {
    CarAction::Play(PlayContext { kind, id: Some(id.into()), track_ids: None })
}

fn kept_album_a1() -> Kept {
    Kept {
        items: [(crate::downloads::store::ItemKind::Album, "a1".to_string())].into(),
        tracks: ["t1".to_string()].into(),
    }
}

#[test]
fn kept_knows_what_plays_with_no_signal() {
    let k = kept_album_a1();
    assert!(k.plays(&ctx(ContextKind::Album, "a1")));
    assert!(!k.plays(&ctx(ContextKind::Album, "a2")));
    assert!(!k.plays(&ctx(ContextKind::Playlist, "a1")), "kinds don't mix");
    assert!(!k.plays(&ctx(ContextKind::Liked, "liked")));
    assert!(!k.plays(&ctx(ContextKind::Artist, "x")), "Popular comes from the server");
    assert!(!k.plays(&ctx(ContextKind::Radio, "track:t1")));
    let some = CarAction::Play(PlayContext {
        kind: ContextKind::Tracks,
        id: None,
        track_ids: Some(vec!["t9".into(), "t1".into()]),
    });
    assert!(k.plays(&some), "a list with a kept song plays it");
    assert!(k.plays(&CarAction::ShuffleLibrary));
    assert!(!Kept::default().plays(&CarAction::ShuffleLibrary), "nothing kept, nothing to shuffle");
    for always in [CarAction::Open(CarList::Albums), CarAction::Jump("q".into()), CarAction::Info] {
        assert!(Kept::default().plays(&always), "{always:?}");
    }
    let liked = Kept { items: [(crate::downloads::store::ItemKind::Liked, String::new())].into(), ..Kept::default() };
    assert!(liked.plays(&ctx(ContextKind::Liked, "liked")), "Liked songs is kept under an empty id");
}

#[test]
fn offline_rows_hide_or_say_what_needs_a_connection() {
    let k = kept_album_a1();
    let rows = || {
        vec![
            row("Kept", ctx(ContextKind::Album, "a1")),
            row("Streamed", ctx(ContextKind::Album, "a2")),
            row("Albums", CarAction::Open(CarList::Albums)),
        ]
    };
    let only: Vec<_> = offline_rows(rows(), &k, true).into_iter().map(|r| r.title).collect();
    assert_eq!(only, ["Kept", "Albums"], "lists stay: they're filtered when opened");
    let all = offline_rows(rows(), &k, false);
    assert_eq!(all.len(), 3);
    assert_eq!(all[0], rows()[0], "a kept row is untouched");
    assert_eq!(all[1].subtitle.as_deref(), Some(NEEDS_CONNECTION));
    assert_eq!(all[1].action, CarAction::Info, "picking it does nothing");
}

#[test]
fn offline_home_keeps_covers_that_play_and_drops_empty_sections() {
    let k = kept_album_a1();
    let sections = vec![
        CarSection { header: None, rows: vec![CarRow::Item(row("Shuffle my library", CarAction::ShuffleLibrary))] },
        CarSection {
            header: None,
            rows: vec![CarRow::Covers {
                title: "Jump back in".into(),
                items: vec![row("a1", ctx(ContextKind::Album, "a1")), row("a2", ctx(ContextKind::Album, "a2"))],
            }],
        },
        CarSection { header: Some("Made for you".into()), rows: vec![CarRow::Item(row("Daily Mix 1", ctx(ContextKind::Mix, "m1")))] },
    ];
    let out = offline_sections(sections.clone(), &k, true);
    assert_eq!(out.len(), 2, "the mixes section had nothing kept");
    let CarRow::Covers { items, .. } = &out[1].rows[0] else { panic!("covers stay") };
    assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["a1"]);
    // Without the setting the mix stays and says why it won't play; covers
    // still only show what plays.
    let out = offline_sections(sections, &k, false);
    assert_eq!(out.len(), 3);
    let CarRow::Item(mix) = &out[2].rows[0] else { panic!() };
    assert_eq!(mix.subtitle.as_deref(), Some(NEEDS_CONNECTION));
    let CarRow::Covers { items, .. } = &out[1].rows[0] else { panic!() };
    assert_eq!(items.len(), 1);
}

#[test]
fn offline_tab_with_nothing_playable_says_so() {
    let rows = vec![row("Streamed", ctx(ContextKind::Album, "a2"))];
    for only in [true, false] {
        let out = offline_tab(rows.clone(), &Kept::default(), only);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, UNREACHABLE);
    }
    let out = offline_tab(rows, &kept_album_a1(), true);
    assert_eq!(out[0].title, UNREACHABLE, "a2 isn't kept");
    let out = offline_tab(vec![row("Kept", ctx(ContextKind::Album, "a1"))], &kept_album_a1(), true);
    assert_eq!(out[0].title, "Kept");
}

#[tokio::test]
async fn kept_reads_finished_downloads_and_shuffle_draws_from_them() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(2, 2).await;
    let (album, tracks): (String, Vec<String>) = db
        .read(|c| {
            let album: String = c.query_row("SELECT id FROM albums ORDER BY id LIMIT 1", [], |r| r.get(0))?;
            let mut st = c.prepare("SELECT id FROM tracks WHERE album_id = ?1 ORDER BY id")?;
            let t = st.query_map([&album], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok((album, t))
        })
        .unwrap();
    db.write(|c| {
        let key = ItemKey::new(ItemKind::Album, &album);
        store::set_item(c, &key, "A", &tracks, 1)?;
        store::reconcile(c)?;
        Ok(())
    })
    .unwrap();
    let k = db.read(Kept::read).unwrap();
    assert!(k.items.is_empty() && k.tracks.is_empty(), "queued, not finished");
    assert!(db.read(shuffle_downloads_context).unwrap().track_ids.unwrap().is_empty());
    db.write(|c| {
        store::mark_done(c, &tracks[0], "/p", "original", "f0.flac", 10, 2)?;
        Ok(())
    })
    .unwrap();
    let k = db.read(Kept::read).unwrap();
    assert!(k.plays(&CarAction::Play(PlayContext { kind: ContextKind::Album, id: Some(album), track_ids: None })));
    assert_eq!(k.tracks, [tracks[0].clone()].into());
    assert_eq!(db.read(shuffle_downloads_context).unwrap().track_ids, Some(vec![tracks[0].clone()]));
    // Shuffle draws exactly the songs Kept counts: finished with a file.
    db.write(|c| {
        for t in &tracks[1..] {
            store::mark_done(c, t, "/p", "original", &format!("{t}.flac"), 10, 3)?;
        }
        Ok(())
    })
    .unwrap();
    let k = db.read(Kept::read).unwrap();
    let drawn: std::collections::HashSet<String> =
        db.read(shuffle_downloads_context).unwrap().track_ids.unwrap().into_iter().collect();
    assert_eq!(drawn, k.tracks);
}

#[tokio::test]
async fn downloaded_lists_what_is_kept_newest_first_and_plays_each() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(2, 2).await;
    let (album, tracks): (String, Vec<String>) = db
        .read(|c| {
            let album: String = c.query_row("SELECT id FROM albums ORDER BY id LIMIT 1", [], |r| r.get(0))?;
            let t = c.prepare("SELECT id FROM tracks ORDER BY id LIMIT 4")?.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok((album, t))
        })
        .unwrap();
    db.write(|c| {
        store::set_item(c, &ItemKey::new(ItemKind::Album, &album), "Ignored for albums", &tracks[..1], 1)?;
        store::set_item(c, &ItemKey::new(ItemKind::Liked, "liked"), "Liked", &tracks[1..2], 2)?;
        store::set_item(c, &ItemKey::new(ItemKind::Playlist, "p1"), "Road trip", &tracks[2..3], 3)?;
        store::set_item(c, &ItemKey::new(ItemKind::Mix, "daily-1"), "Daily Mix 1", &tracks[2..4], 4)?;
        Ok(())
    })
    .unwrap();
    let rows = db.read(downloaded).unwrap();
    let titles: Vec<_> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(&titles[..3], ["Daily Mix 1", "Road trip", "Liked songs"], "newest first");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.kind == ContextKind::Tracks && p.track_ids.as_deref() == Some(&tracks[2..4])), "a mix plays the songs it was kept with");
    assert_eq!(rows[1].subtitle.as_deref(), Some("Playlist"));
    assert!(matches!(&rows[1].action, CarAction::Play(p) if p.kind == ContextKind::Playlist && p.id.as_deref() == Some("p1")));
    assert!(matches!(&rows[2].action, CarAction::Play(p) if p.kind == ContextKind::Liked));
    // An album shows as the library has it, with who made it.
    assert_ne!(rows[3].title, "Ignored for albums");
    assert!(rows[3].subtitle.as_deref().is_some_and(|s| !s.is_empty()));
    assert!(matches!(&rows[3].action, CarAction::Play(p) if p.kind == ContextKind::Album && p.id.as_deref() == Some(album.as_str())));
}

#[tokio::test]
async fn downloaded_keeps_an_album_the_library_lost() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(1, 1).await;
    db.write(|c| store::set_item(c, &ItemKey::new(ItemKind::Album, "gone"), "Old album", &[], 1)).unwrap();
    let rows = db.read(downloaded).unwrap();
    assert_eq!(rows[0].title, "Old album", "its kept title, not a blank row");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.id.as_deref() == Some("gone")));
}

#[tokio::test]
async fn jump_back_in_holds_one_row_of_covers() {
    let (db, _dir) = synced(10, 3).await;
    db.write(|c| Ok(c.execute_batch("UPDATE albums SET last_viewed_at = 1000 + rowid, view_count = 1")?)).unwrap();
    let sections = db.read(home_sections).unwrap();
    let CarRow::Covers { items, .. } = &sections[1].rows[0] else { panic!("covers") };
    assert_eq!(items.len(), COVERS_MAX, "30 played albums, one row");
}

#[tokio::test]
async fn made_for_you_shows_at_most_the_cap_in_shelf_order() {
    let (db, _dir) = synced(6, 3).await;
    db.write(|c| {
        let ids: Vec<String> = c.prepare("SELECT id FROM tracks")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for i in 0..MIXES_MAX + 3 {
            let mix = crate::mixes::store::Stored {
                id: format!("daily-{i}"),
                kind: crate::mixes::MixKind::Daily,
                title: format!("Daily Mix {i}"),
                description: String::new(),
                color: "#334455".into(),
                artist_ids: vec![],
                tag: None,
                tracks: ids.iter().map(|id| (id.clone(), false)).collect(),
            };
            crate::mixes::store::save_one(c, &mix, i, "2026-10-20", 0)?;
        }
        Ok(())
    })
    .unwrap();
    let sections = db.read(home_sections).unwrap();
    let made = sections.iter().find(|s| s.header.as_deref() == Some("Made for you")).expect("mixes");
    assert_eq!(made.rows.len(), MIXES_MAX);
    let CarRow::Item(first) = &made.rows[0] else { panic!() };
    assert_eq!(first.title, "Daily Mix 0");
    assert_eq!(first.subtitle, None, "no empty line under a mix without a description");
}

#[tokio::test]
async fn radio_offers_an_album_not_played_in_a_while() {
    let (db, _dir) = synced(3, 2).await;
    db.write(|c| Ok(c.execute_batch("UPDATE albums SET view_count = 1, last_viewed_at = 1000 WHERE rowid = 1")?)).unwrap();
    let rows = db.read(|c| radio(c, None)).unwrap();
    let old = rows.iter().find(|r| r.title == "One you haven't played in a while").expect("the forgotten album");
    assert!(old.subtitle.as_deref().is_some_and(|s| s.contains(" by ")));
    assert!(matches!(&old.action, CarAction::Play(p) if p.kind == ContextKind::Album));
}
