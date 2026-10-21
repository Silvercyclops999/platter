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
    let albums = db.read(|c| list(c, &CarList::Albums, 10.0)).unwrap();
    assert_eq!(albums.len(), LIST_MAX, "150 albums are cut to the car's limit");
    for a in &albums {
        assert!(!a.title.is_empty());
        assert!(a.subtitle.as_deref().is_some_and(|s| !s.is_empty()), "albums say who made them");
        assert!(matches!(&a.action, CarAction::Open(CarList::Songs { kind: ContextKind::Album, .. })), "an album opens to its songs");
    }
    let artists = db.read(|c| list(c, &CarList::Artists, 10.0)).unwrap();
    assert_eq!(artists.len(), 30);
    assert!(artists.iter().all(|a| matches!(&a.action, CarAction::Open(CarList::Artist { id, name }) if !id.is_empty() && *name == a.title)));
    let added = db.read(|c| list(c, &CarList::RecentlyAdded, 10.0)).unwrap();
    assert_eq!(added.len(), LIST_MAX);
}

#[tokio::test]
async fn an_empty_library_gives_empty_lists_not_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    for which in [CarList::RecentlyPlayed, CarList::RecentlyAdded, CarList::Playlists, CarList::Albums, CarList::Artists] {
        assert!(db.read(|c| list(c, &which, 10.0)).unwrap().is_empty(), "{which:?}");
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
    assert!(items.iter().all(|i| matches!(&i.action, CarAction::Open(CarList::Songs { .. }))), "a cover opens its songs");
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
    assert!(opens(&tab[0].action, ContextKind::Liked, "liked"));
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
    let artists = db.read(|c| list(c, &CarList::Artists, 10.0)).unwrap();
    let CarAction::Open(which) = &artists[0].action else { panic!("artists open a list") };
    let CarList::Artist { id, .. } = which else { panic!("an artist's list") };
    assert_eq!(which.title(), artists[0].title, "the list is titled with the artist's name");
    let rows = db.read(|c| list(c, which, 10.0)).unwrap();
    assert_eq!(rows.len(), 5, "Popular and four albums");
    assert_eq!(rows[0].title, "Popular");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.kind == ContextKind::Artist && p.id.as_ref() == Some(id)));
    let own = db.read(|c| queries::artist_albums(c, id, LIST_MAX)).unwrap();
    for (row, album) in rows[1..].iter().zip(&own) {
        assert_eq!(row.title, album.title);
        assert!(opens(&row.action, ContextKind::Album, &album.id));
    }
}

#[tokio::test]
async fn an_artist_list_stays_short_and_an_unknown_artist_is_just_popular() {
    let (db, _dir) = synced(1, 150).await;
    let id = db.read(|c| list(c, &CarList::Artists, 10.0)).unwrap().into_iter().find_map(|a| match a.action {
        CarAction::Open(CarList::Artist { id, .. }) => Some(id),
        _ => None,
    });
    let rows = db.read(|c| list(c, &CarList::Artist { id: id.unwrap(), name: "A".into() }, 10.0)).unwrap();
    assert_eq!(rows.len(), LIST_MAX, "Popular plus albums up to the car's limit");
    let none = db.read(|c| list(c, &CarList::Artist { id: "nobody".into(), name: "Nobody".into() }, 10.0)).unwrap();
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
    let now = NowKey { context: Some((ContextKind::Playlist, "p1".into())), album_id: Some("al9".into()), track_id: None };
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
    assert!(opens(&m.action, ContextKind::Mix, "daily-1"));
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
    assert!(opens(&rows[1].action, ContextKind::Playlist, "p1"));
    assert!(opens(&rows[2].action, ContextKind::Liked, "liked"));
    // An album shows as the library has it, with who made it.
    assert_ne!(rows[3].title, "Ignored for albums");
    assert!(rows[3].subtitle.as_deref().is_some_and(|s| !s.is_empty()));
    assert!(opens(&rows[3].action, ContextKind::Album, &album));
}

#[tokio::test]
async fn downloaded_keeps_an_album_the_library_lost() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(1, 1).await;
    db.write(|c| store::set_item(c, &ItemKey::new(ItemKind::Album, "gone"), "Old album", &[], 1)).unwrap();
    let rows = db.read(downloaded).unwrap();
    assert_eq!(rows[0].title, "Old album", "its kept title, not a blank row");
    assert!(opens(&rows[0].action, ContextKind::Album, "gone"));
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

fn opens(a: &CarAction, kind: ContextKind, id: &str) -> bool {
    matches!(a, CarAction::Open(CarList::Songs { kind: k, id: i, .. }) if *k == kind && i == id)
}

#[tokio::test]
async fn an_album_opens_to_play_shuffle_then_its_songs() {
    let (db, _dir) = synced(1, 1).await;
    let albums = db.read(|c| list(c, &CarList::Albums, 10.0)).unwrap();
    let CarAction::Open(which) = &albums[0].action else { panic!("opens") };
    assert_eq!(which.title(), albums[0].title, "titled with the album");
    let rows = db.read(|c| list(c, which, 10.0)).unwrap();
    assert_eq!(rows[0].title, "Play");
    assert_eq!(rows[0].subtitle.as_deref(), Some("3 songs"));
    assert!(matches!(&rows[0].action, CarAction::Start { context, shuffle: false } if context.kind == ContextKind::Album));
    assert_eq!(rows[1].title, "Shuffle");
    assert!(matches!(&rows[1].action, CarAction::Start { context, shuffle: true } if context.kind == ContextKind::Album));
    assert_eq!(rows.len(), 2 + 3, "then its three songs");
    for (n, r) in rows[2..].iter().enumerate() {
        assert!(
            matches!(&r.action, CarAction::PlayFrom { context, track_id, index } if context.kind == ContextKind::Album && !track_id.is_empty() && *index == n),
            "a song plays the album from its own place"
        );
        assert!(r.thumb.is_none(), "an album's songs share its cover");
    }
}

#[test]
fn an_opened_album_is_marked_while_it_plays() {
    let row = CarItem { title: "A".into(), subtitle: None, thumb: None, action: songs(ContextKind::Album, "al1", "A") };
    let now = NowKey { context: Some((ContextKind::Album, "al1".into())), album_id: None, track_id: Some("t2".into()) };
    assert!(is_playing(&row, &now));
    assert!(is_playing(&row, &NowKey { album_id: Some("al1".into()), ..Default::default() }));
    assert!(!is_playing(&row, &NowKey::default()));
    // Inside the opened album: the playing song is marked, Play and Shuffle aren't.
    let ctx = PlayContext { kind: ContextKind::Album, id: Some("al1".into()), track_ids: None };
    let song = |t: &str| CarItem { title: t.into(), subtitle: None, thumb: None, action: CarAction::PlayFrom { context: ctx.clone(), track_id: t.into(), index: 0 } };
    assert!(is_playing(&song("t2"), &now));
    assert!(!is_playing(&song("t1"), &now));
    let start = CarItem { title: "Play".into(), subtitle: None, thumb: None, action: CarAction::Start { context: ctx.clone(), shuffle: false } };
    assert!(!is_playing(&start, &now));
    // The same song playing from another list isn't this row.
    let elsewhere = NowKey { context: Some((ContextKind::Playlist, "p1".into())), track_id: Some("t2".into()), ..Default::default() };
    assert!(!is_playing(&song("t2"), &elsewhere));
}

#[test]
fn offline_a_kept_album_opens_and_its_kept_songs_play() {
    let mut kept = Kept::default();
    kept.items.insert((crate::downloads::store::ItemKind::Album, "al1".into()));
    kept.tracks.insert("t1".into());
    assert!(kept.plays(&songs(ContextKind::Album, "al1", "A")));
    assert!(!kept.plays(&songs(ContextKind::Album, "al2", "B")));
    let ctx = PlayContext { kind: ContextKind::Album, id: Some("al1".into()), track_ids: None };
    assert!(kept.plays(&CarAction::PlayFrom { context: ctx.clone(), track_id: "t1".into(), index: 0 }));
    assert!(!kept.plays(&CarAction::PlayFrom { context: ctx, track_id: "t2".into(), index: 1 }));
    // Kept playlists, Liked songs and mixes open with no signal too.
    use crate::downloads::store::ItemKind;
    kept.items.insert((ItemKind::Playlist, "p1".into()));
    kept.items.insert((ItemKind::Liked, String::new()));
    kept.items.insert((ItemKind::Mix, "m1".into()));
    assert!(kept.plays(&songs(ContextKind::Playlist, "p1", "P")));
    assert!(kept.plays(&songs(ContextKind::Liked, "liked", "Liked songs")));
    assert!(kept.plays(&songs(ContextKind::Mix, "m1", "M")));
    assert!(!kept.plays(&songs(ContextKind::Playlist, "p2", "Q")));
}

#[tokio::test]
async fn a_long_list_says_how_many_it_leaves_out() {
    let (db, _dir) = synced(1, 1).await;
    let ids: Vec<String> = db.read(|c| Ok(c.prepare("SELECT id FROM tracks")?.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?)).unwrap();
    let many: Vec<String> = ids.iter().cycle().take(150).cloned().collect();
    let mix = crate::mixes::store::Stored {
        id: "big".into(),
        kind: crate::mixes::MixKind::Daily,
        title: "Big".into(),
        description: String::new(),
        color: "#000000".into(),
        artist_ids: vec![],
        tag: None,
        tracks: many.into_iter().map(|id| (id, false)).collect(),
    };
    db.write(|c| crate::mixes::store::save_one(c, &mix, 0, "2026-10-20", 0)).unwrap();
    let rows = db.read(|c| list(c, &CarList::Songs { kind: ContextKind::Mix, id: "big".into(), title: "Big".into() }, 10.0)).unwrap();
    assert_eq!(rows.len(), LIST_MAX, "stays within the car's limit");
    let last = rows.last().unwrap();
    assert!(last.title.starts_with("Showing ") && last.title.ends_with(" songs"), "{}", last.title);
    assert_eq!(last.action, CarAction::Info);
}


mod pictures {
    use super::super::pictures::{draw, drawn, mix_artists, picture_for, CarPicture};
    use super::*;

    fn decoded(png: &[u8]) -> image::RgbImage {
        image::load_from_memory(png).expect("a PNG").to_rgb8()
    }

    /// How many of the picture's pixels differ from its corner (the ground).
    fn marked(img: &image::RgbImage) -> usize {
        let ground = *img.get_pixel(img.width() - 1, img.height() - 1);
        img.pixels().filter(|p| p.0.iter().zip(ground.0).any(|(a, b)| a.abs_diff(b) > 24)).count()
    }

    fn mix_item(title: &str, description: &str) -> CarItem {
        CarItem {
            title: title.into(),
            subtitle: Some(description.into()),
            thumb: None,
            action: songs(ContextKind::Mix, "daily-1", title),
        }
    }

    #[test]
    fn each_row_kind_gets_its_own_picture() {
        assert_eq!(picture_for(&shuffle_library()), Some(CarPicture::Shuffle));
        assert_eq!(picture_for(&liked_item()), Some(CarPicture::Liked));
        assert_eq!(picture_for(&library()[0]), Some(CarPicture::Liked), "Liked songs on the Library tab too");
        assert_eq!(
            picture_for(&mix_item("Daily Mix 1", "Motte, Saltglass, Odessa Wren, Kesto Marrow and more")),
            Some(CarPicture::Mix { title: "Daily Mix 1".into(), artists: vec!["Motte".into(), "Saltglass".into(), "Odessa Wren".into()] })
        );
        // Lists to open and covers draw nothing of their own.
        assert_eq!(picture_for(&open(CarList::Albums)), None);
        let album = CarItem { title: "A".into(), subtitle: None, thumb: Some("/t".into()), action: play(ContextKind::Album, "a1") };
        assert_eq!(picture_for(&album), None);
    }

    #[test]
    fn mix_lines_give_their_artists() {
        assert_eq!(mix_artists("with Motte, Saltglass and more"), vec!["Motte", "Saltglass"]);
        assert_eq!(mix_artists("Motte and more"), vec!["Motte"]);
        assert!(mix_artists("Songs you haven't played in a while").is_empty());
        assert!(mix_artists("").is_empty());
    }

    #[test]
    fn every_picture_renders_at_each_size_the_car_asks_for() {
        let all = [
            CarPicture::Shuffle,
            CarPicture::Liked,
            CarPicture::Mix { title: "Daily Mix 1".into(), artists: vec!["Motte".into(), "Saltglass".into()] },
            CarPicture::Mix { title: "Discover".into(), artists: vec![] },
            CarPicture::Placeholder,
        ];
        for px in [120, 240] {
            for p in &all {
                let img = decoded(&draw(p, px));
                assert_eq!((img.width(), img.height()), (px, px), "{p:?} at {px}");
                let share = marked(&img) as f32 / (px * px) as f32;
                assert!(share > 0.02 && share < 0.6, "{p:?} at {px} draws something, not a flood: {share}");
            }
        }
    }

    #[test]
    fn the_sleeves_are_paper_and_shuffle_is_graphite() {
        let corner = |p: &CarPicture| *decoded(&draw(p, 120)).get_pixel(2, 117);
        for p in [CarPicture::Liked, CarPicture::Placeholder, CarPicture::Mix { title: "M".into(), artists: vec![] }] {
            let c = corner(&p);
            assert!(c.0.iter().all(|&v| v > 200), "{p:?} on paper: {c:?}");
        }
        let g = corner(&CarPicture::Shuffle);
        assert!(g.0.iter().all(|&v| v < 60), "Shuffle on graphite: {g:?}");
    }

    #[test]
    fn a_bigger_sleeve_lists_the_artists_and_a_small_one_keeps_its_name() {
        let with = CarPicture::Mix { title: "Daily Mix 1".into(), artists: vec!["Motte".into(), "Saltglass".into(), "Odessa Wren".into()] };
        let without = CarPicture::Mix { title: "Daily Mix 1".into(), artists: vec![] };
        // At 240 the names under the rule add ink; at 120 they're left off.
        assert!(marked(&decoded(&draw(&with, 240))) > marked(&decoded(&draw(&without, 240))));
        assert_eq!(draw(&with, 120), draw(&without, 120));
    }

    #[test]
    fn long_names_and_odd_letters_still_draw() {
        let p = CarPicture::Mix {
            title: "Supercalifragilisticexpialidocious Radio and a very long name indeed".into(),
            artists: vec!["Sigur Rós".into(), "Łona".into(), "日本".into()],
        };
        // A name the type has no letters for is left off rather than printed as boxes.
        let CarPicture::Mix { artists, .. } = drawn(&p, 240) else { panic!("a sleeve") };
        assert_eq!(artists, vec!["Sigur Rós", "Łona"]);
        let img = decoded(&draw(&p, 240));
        assert!(marked(&img) > 0);
        // Nothing runs into the sleeve's right margin: a long word is cut, not clipped.
        let edge = (img.width() * 96 / 100..img.width()).flat_map(|x| (0..img.height()).map(move |y| (x, y)));
        let ground = *img.get_pixel(img.width() - 1, img.height() - 1);
        assert!(edge.into_iter().all(|(x, y)| img.get_pixel(x, y).0.iter().zip(ground.0).all(|(a, b)| a.abs_diff(b) <= 24)));
    }

    #[test]
    fn a_small_sleeve_keys_like_any_other_with_its_name() {
        let a = CarPicture::Mix { title: "Daily Mix 1".into(), artists: vec!["Motte".into()] };
        let b = CarPicture::Mix { title: "Daily Mix 1".into(), artists: vec!["Saltglass".into()] };
        assert_eq!(drawn(&a, 120), drawn(&b, 120));
        assert_ne!(drawn(&a, 240), drawn(&b, 240));
    }

    #[test]
    fn mixes_on_home_show_their_sleeve_not_a_photo() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (db, _dir) = rt.block_on(synced(2, 2));
        db.write(|c| {
            let ids: Vec<String> = c.prepare("SELECT id FROM tracks LIMIT 10")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            let mix = crate::mixes::store::Stored {
                id: "daily-1".into(),
                kind: crate::mixes::MixKind::Daily,
                title: "Daily Mix 1".into(),
                description: "Motte, Saltglass and more".into(),
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
        let made = sections.iter().find(|s| s.header.as_deref() == Some("Made for you")).expect("mixes");
        let CarRow::Item(m) = &made.rows[0] else { panic!("a mix row") };
        assert_eq!(m.thumb, None);
        assert!(matches!(picture_for(m), Some(CarPicture::Mix { ref artists, .. }) if artists == &["Motte", "Saltglass"]));
    }
}
