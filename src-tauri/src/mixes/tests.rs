//! Mix engine tests on fixtures, the service against a temporary cache, and
//! a 100k-song benchmark (ignored; run it explicitly).

use std::collections::{HashMap, HashSet};

use super::engine::{self, Ctx, Generated, Taste};
use super::library::{ArtistRow, Lib, Raw, RawAlbum, RawTrack, TagKind};
use super::radio::{self, Seed};
use super::*;
use super::service::now_ms;

/// 2026-10-14 12:00 UTC, a Wednesday.
const NOW: i64 = 1_791_979_200_000;
const DAY: i64 = 24 * 3600 * 1000;
const HOUR: i64 = 3600 * 1000;

/// Twelve artists in three genres (Rock, Jazz, Hip-Hop), two albums of ten
/// songs each. Artists 0 and 1 of each genre are played a lot (their first
/// eight songs of each album), and rock and jazz sit in separate sessions.
fn raw() -> Raw {
    let genres = ["Rock", "Jazz", "Hip-Hop"];
    let mut raw = Raw::default();
    for (g, genre) in genres.iter().enumerate() {
        for a in 0..4 {
            let artist = format!("{}{a}", genre[..1].to_lowercase());
            raw.artists.push(ArtistRow { id: artist.clone(), title: format!("Artist {artist}"), thumb: Some(format!("/thumb/{artist}")), similar: vec![] });
            for al in 0..2 {
                let album = format!("{artist}-al{al}");
                let mut tags = vec![(TagKind::Genre, genre.to_string())];
                if g == 2 {
                    tags.push((TagKind::Mood, "Night".to_string()));
                }
                raw.albums.push(RawAlbum { id: album.clone(), artist_id: artist.clone(), thumb: Some(format!("/cover/{album}")), added_at: 1_000, tags, unreleased: false });
                for t in 0..10 {
                    let id = format!("{album}-t{t}");
                    let played = a < 2 && t < 8;
                    raw.tracks.push(RawTrack {
                        id: id.clone(),
                        artist_id: artist.clone(),
                        album_id: album.clone(),
                        track_number: t + 1,
                        duration_ms: 200_000,
                        tags: vec![],
                        view_count: if played { 5 } else { 0 },
                        last_viewed_at: played.then_some((NOW - 2 * DAY) / 1000),
                        ..Default::default()
                    });
                }
            }
        }
    }
    // Sessions: rock songs together at 9:00, jazz together at 21:00.
    for d in 1..20i64 {
        for (genre, hour) in [("r", 9), ("j", 21)] {
            for k in 0..6i64 {
                let a = k % 2;
                let id = format!("{genre}{a}-al0-t{}", k % 8);
                raw.listens.push((id, NOW - d * DAY + (hour - 12) * HOUR + k * 4 * 60_000, false));
            }
        }
    }
    raw
}

fn opts() -> MixOptions {
    MixOptions::default()
}

fn lib_of(raw: Raw, o: &MixOptions) -> Lib {
    Lib::build(raw, o, 10.0)
}

fn ctx() -> Ctx {
    Ctx { now_ms: NOW, tz_offset_min: 0, variety: Variety::Balanced, seed: 7, daylist_hours: 3 }
}

fn artist_of(lib: &Lib, t: u32) -> &str {
    &lib.artists[lib.tracks[t as usize].artist as usize].id
}

fn max_run(lib: &Lib, g: &Generated) -> usize {
    let (mut best, mut run) = (0, 0);
    for (i, (t, _)) in g.tracks.iter().enumerate() {
        run = if i > 0 && artist_of(lib, g.tracks[i - 1].0) == artist_of(lib, *t) { run + 1 } else { 1 };
        best = best.max(run);
    }
    best
}

#[test]
fn days_start_at_4am_local_and_weekdays_count_from_monday() {
    assert_eq!(engine::day_string(NOW), "2026-10-14");
    let c = ctx();
    assert_eq!(c.mix_day(), "2026-10-14");
    assert_eq!(c.weekday(NOW), 2, "a Wednesday");
    let early = Ctx { now_ms: NOW - 9 * HOUR, ..ctx() }; // 03:00
    assert_eq!(early.mix_day(), "2026-10-13", "before 4 am is still yesterday's mixes");
    // UTC+10: 22:00 local, same day; UTC-13: 23:00 the day before.
    assert_eq!(Ctx { tz_offset_min: 600, ..ctx() }.hour(NOW), 22);
    assert_eq!(Ctx { tz_offset_min: -13 * 60, ..ctx() }.mix_day(), "2026-10-13");
    assert_eq!(engine::day_string(0), "1970-01-01");
    assert_eq!(engine::day_string(951_782_400_000), "2000-02-29");
}

#[test]
fn top_artists_cluster_by_shared_tags_and_listening() {
    let lib = lib_of(raw(), &opts());
    let taste = Taste::new(&lib);
    let groups = engine::clusters(&lib, &taste);
    let names: Vec<HashSet<String>> =
        groups.iter().map(|g| g.iter().map(|a| lib.artists[*a as usize].id.clone()).collect()).collect();
    let together = |a: &str, b: &str| names.iter().any(|g| g.contains(a) && g.contains(b));
    assert!(together("r0", "r1"), "{names:?}");
    assert!(together("j0", "j1"), "{names:?}");
    assert!(!together("r0", "j0"), "{names:?}");
    assert!(groups.len() <= engine::DAILY_MAX);
}

#[test]
fn daily_mixes_are_mostly_favourites_with_some_fresh_songs() {
    let lib = lib_of(raw(), &opts());
    let mixes = engine::daily_mixes(&lib, &Taste::new(&lib), &ctx(), &HashSet::new());
    assert!(!mixes.is_empty());
    let m = &mixes[0];
    assert_eq!(m.title, "Daily Mix 1");
    let fav = m.tracks.iter().filter(|(_, f)| *f).count();
    let fresh = m.tracks.len() - fav;
    assert!(fav > 0 && fresh > 0, "{fav} favourites, {fresh} fresh");
    for (t, f) in &m.tracks {
        assert_eq!(*f, lib.familiar(*t), "favourites are played songs");
    }
    // Plenty of each: the split follows the setting (70/30 of 50 when balanced).
    let mut big = raw();
    for t in big.tracks.iter_mut() {
        if t.artist_id.starts_with('r') && t.view_count == 0 && t.track_number % 2 == 0 {
            t.view_count = 4;
        }
    }
    for i in 0..60 {
        big.tracks.push(RawTrack { id: format!("rx{i}"), artist_id: "r2".into(), album_id: "r2-al0".into(), duration_ms: 1, ..Default::default() });
        big.tracks.push(RawTrack { id: format!("ry{i}"), artist_id: "r0".into(), album_id: "r0-al1".into(), view_count: 6, duration_ms: 1, ..Default::default() });
    }
    let lib = lib_of(big, &opts());
    let taste = Taste::new(&lib);
    for (variety, want) in [(Variety::Balanced, 35), (Variety::Familiar, 43), (Variety::Adventurous, 25)] {
        let c = Ctx { variety, ..ctx() };
        let rock = engine::daily_mixes(&lib, &taste, &c, &HashSet::new())
            .into_iter()
            .find(|m| m.artist_ids.iter().any(|a| a == "r0"))
            .unwrap();
        assert_eq!(rock.tracks.len(), engine::DAILY_LEN);
        let fav = rock.tracks.iter().filter(|(_, f)| *f).count();
        assert_eq!(fav, want, "{variety:?}");
    }
}

#[test]
fn no_artist_plays_more_than_twice_in_a_row() {
    let lib = lib_of(raw(), &opts());
    // All-Time Top Songs is a ranking, so it keeps its order.
    for m in engine::generate_all(&lib, &ctx()).into_iter().filter(|m| m.kind != MixKind::TopSongs) {
        assert!(max_run(&lib, &m) <= 2, "{} has a run of {}", m.title, max_run(&lib, &m));
    }
    // The spacing pass itself.
    let mut list: Vec<(u32, bool)> = vec![0, 1, 2, 3, 20, 21].into_iter().map(|t| (t, true)).collect();
    engine::space_artists(&lib, &mut list, 2);
    let artists: Vec<&str> = list.iter().map(|(t, _)| artist_of(&lib, *t)).collect();
    assert_eq!(artists, vec!["r0", "r0", "r1", "r0", "r0", "r1"]);
}

#[test]
fn a_song_is_in_one_daily_mix_a_day_at_most() {
    let lib = lib_of(raw(), &opts());
    let mixes = engine::daily_mixes(&lib, &Taste::new(&lib), &ctx(), &HashSet::new());
    assert!(mixes.len() >= 2);
    let mut seen = HashSet::new();
    for m in &mixes {
        for (t, _) in &m.tracks {
            assert!(seen.insert(*t), "{} is in two Daily Mixes", lib.tracks[*t as usize].id);
        }
    }
    // A refresh keeps clear of the others.
    let others: HashSet<u32> = mixes[1..].iter().flat_map(|m| m.tracks.iter().map(|(t, _)| *t)).collect();
    let again = engine::regenerate_one(&lib, &Ctx { seed: 99, ..ctx() }, "daily:1", &others).unwrap();
    assert!(again.tracks.iter().all(|(t, _)| !others.contains(t)));
}

#[test]
fn quick_skips_count_against_a_song() {
    let mut r = raw();
    for _ in 0..4 {
        r.listens.push(("r0-al0-t0".into(), NOW - HOUR, true));
        r.listens.push(("r2-al0-t0".into(), NOW - HOUR, true));
    }
    let lib = lib_of(r, &opts());
    let skipped = lib.index["r0-al0-t0"];
    let kept = lib.index["r0-al0-t2"];
    assert!(lib.affinity(skipped) < lib.affinity(kept));
    assert_eq!(lib.tracks[skipped as usize].skips, 4);
    assert_eq!(lib.tracks[skipped as usize].plays, lib.tracks[kept as usize].plays, "a skip is not a play");
    // A never played song skipped away isn't offered as something new.
    let fresh_skipped = lib.index["r2-al0-t0"];
    assert!(!lib.fresh(fresh_skipped));
    for m in engine::generate_all(&lib, &ctx()) {
        assert!(m.tracks.iter().all(|(t, _)| *t != fresh_skipped), "{}", m.title);
    }
}

#[test]
fn discover_on_repeat_and_rediscover_keep_to_their_windows() {
    let mut r = raw();
    // Played a lot 7 months ago and not since.
    for t in r.tracks.iter_mut().filter(|t| t.id.starts_with("h3-") || t.id.starts_with("h2-")) {
        t.view_count = 6;
        t.last_viewed_at = Some((NOW - 210 * DAY) / 1000);
    }
    // Old plays of a song outside the On Repeat window.
    for k in 0..5 {
        r.listens.push(("h2-al0-t0".into(), NOW - 40 * DAY - k * HOUR, false));
    }
    let lib = lib_of(r, &opts());
    let taste = Taste::new(&lib);
    let c = ctx();
    let d = engine::discover(&lib, &taste, &c).unwrap();
    assert!(d.tracks.len() <= engine::SHORT_LEN);
    assert!(d.tracks.iter().all(|(t, _)| lib.tracks[*t as usize].plays == 0), "only songs never played");
    let per_artist = d.tracks.iter().fold(HashMap::new(), |mut m: HashMap<&str, usize>, (t, _)| {
        *m.entry(artist_of(&lib, *t)).or_default() += 1;
        m
    });
    assert!(per_artist.values().all(|n| *n <= 2));
    let on = engine::on_repeat(&lib, &c).unwrap();
    let since = NOW - 30 * DAY;
    for (t, _) in &on.tracks {
        let recent = lib.plays.iter().filter(|e| e.track == *t && !e.skipped && e.at_ms >= since).count();
        assert!(recent >= 2);
    }
    assert!(on.tracks.iter().all(|(t, _)| lib.tracks[*t as usize].id != "h2-al0-t0"));
    let re = engine::rediscover(&lib, &c).unwrap();
    assert!(re.tracks.iter().all(|(t, _)| matches!(artist_of(&lib, *t), "h2" | "h3")), "only the long-unplayed songs");
    assert!(re.tracks.iter().all(|(t, _)| lib.tracks[*t as usize].id != "h2-al0-t0"), "played 40 days ago");
}

#[test]
fn new_in_your_library_takes_the_latest_albums() {
    let mut r = raw();
    for a in r.albums.iter_mut().filter(|a| a.id.starts_with("j3")) {
        a.added_at = (NOW - 2 * DAY) / 1000;
    }
    let lib = lib_of(r, &opts());
    let m = engine::new_in_library(&lib, &ctx()).unwrap();
    assert_eq!(m.tracks.len(), 6, "three songs from each of the two new albums");
    assert!(m.tracks.iter().all(|(t, _)| artist_of(&lib, *t) == "j3"));
    assert!(engine::new_in_library(&lib_of(raw(), &opts()), &ctx()).is_none(), "nothing new, no mix");
}

#[test]
fn tags_with_enough_songs_get_genre_and_mood_mixes() {
    let lib = lib_of(raw(), &opts());
    let (genres, moods) = engine::tag_mixes(&lib, &ctx());
    let titles: Vec<&str> = genres.iter().map(|m| m.title.as_str()).collect();
    assert!(titles.contains(&"Rock Mix") && titles.contains(&"Jazz Mix") && titles.contains(&"Hip-Hop Mix"), "{titles:?}");
    assert_eq!(moods.len(), 1);
    assert_eq!(moods[0].title, "Night Mix");
    assert_eq!(moods[0].kind, MixKind::Mood);
    assert!(moods[0].tracks.iter().all(|(t, _)| artist_of(&lib, *t).starts_with('h')));
    // Under 25 songs: no mix.
    let mut r = raw();
    r.albums.push(RawAlbum { id: "x".into(), artist_id: "r0".into(), tags: vec![(TagKind::Genre, "Polka".into())], ..Default::default() });
    for i in 0..24 {
        r.tracks.push(RawTrack { id: format!("x{i}"), artist_id: "r0".into(), album_id: "x".into(), ..Default::default() });
    }
    let (genres, _) = engine::tag_mixes(&lib_of(r, &opts()), &ctx());
    assert!(!genres.iter().any(|m| m.title == "Polka Mix"));
}

#[test]
fn the_daylist_follows_the_hour_and_names_the_part_of_the_day() {
    let lib = lib_of(raw(), &opts());
    // 21:30 local: the jazz played around 21:00 leads.
    let evening = Ctx { now_ms: NOW + 9 * HOUR + 30 * 60_000, ..ctx() };
    let d = engine::daylist(&lib, &evening).unwrap();
    assert_eq!(d.title, "jazz wednesday night");
    assert_eq!(d.description, "Here's some jazz, inspired by your listening on Wednesdays at night.");
    assert_eq!(d.tracks.len(), engine::DAYLIST_LEN.min(d.tracks.len()));
    let fav: Vec<u32> = d.tracks.iter().filter(|(_, f)| *f).map(|(t, _)| *t).collect();
    assert!(!fav.is_empty() && fav.iter().all(|t| artist_of(&lib, *t).starts_with('j')));
    // 9:00 local the same day: rock. The same moment in UTC+12 is 21:00 there.
    let morning = Ctx { now_ms: NOW - 3 * HOUR, ..ctx() };
    assert_eq!(engine::daylist(&lib, &morning).unwrap().title, "rock wednesday morning");
    let tz = Ctx { now_ms: NOW - 3 * HOUR, tz_offset_min: 12 * 60, ..ctx() };
    assert_eq!(engine::daylist(&lib, &tz).unwrap().title, "rock wednesday night", "plays are placed by local hour too");
    // 21:30 with 3-hour blocks: the next one starts at midnight local.
    assert_eq!(evening.next_daylist_update(), NOW + 12 * HOUR);
    assert_eq!(Ctx { daylist_hours: 1, ..evening.clone() }.next_daylist_update(), NOW + 10 * HOUR);
    assert_eq!(Ctx { tz_offset_min: 60, ..morning.clone() }.next_daylist_update(), NOW - HOUR, "10:00 local, next block at 12:00 local");
    assert_eq!(engine::part_of_day(2), "late night");
    assert_ne!(evening.daylist_slot(), morning.daylist_slot());
    let soon = Ctx { now_ms: evening.now_ms + 20 * 60_000, ..evening.clone() };
    assert_eq!(evening.daylist_slot(), soon.daylist_slot(), "same block of hours, same daylist");
}

#[test]
fn radio_refills_without_repeating_and_starts_with_its_song() {
    let lib = lib_of(raw(), &opts());
    let taste = Taste::new(&lib);
    let seed = Seed::Track("r0-al0-t0".into());
    let first = radio::batch(&lib, &taste, &seed, &HashSet::new(), radio::RADIO_BATCH, 1, true, radio::LEAN_PLAIN);
    assert_eq!(lib.tracks[first[0] as usize].id, "r0-al0-t0");
    assert_eq!(first.len(), radio::RADIO_BATCH);
    let rock = first.iter().filter(|t| artist_of(&lib, **t).starts_with('r')).count();
    assert!(rock * 2 > first.len(), "mostly like the seed: {rock} of {}", first.len());
    let mut avoid: HashSet<String> = first.iter().map(|t| lib.tracks[*t as usize].id.clone()).collect();
    let next = radio::batch(&lib, &taste, &seed, &avoid, radio::RADIO_BATCH, 2, false, radio::LEAN_PLAIN);
    assert!(!next.is_empty());
    for t in &next {
        assert!(avoid.insert(lib.tracks[*t as usize].id.clone()), "repeated {}", lib.tracks[*t as usize].id);
    }
    let seed_artist = next.iter().filter(|t| artist_of(&lib, **t) == "r0").count();
    assert!(seed_artist <= radio::RADIO_BATCH / 3);
    // Artist radio and similar music after a context.
    assert!(!radio::batch(&lib, &taste, &Seed::Artist("j1".into()), &HashSet::new(), 10, 3, true, radio::LEAN_PLAIN).is_empty());
    let after = radio::batch(&lib, &taste, &Seed::Tracks(vec!["h0-al0-t0".into()]), &HashSet::new(), 10, 4, false, radio::LEAN_PLAIN);
    assert!(after.iter().all(|t| lib.tracks[*t as usize].id != "h0-al0-t0"));
    assert_eq!(Seed::parse("artist:5"), Some(Seed::Artist("5".into())));
    assert_eq!(Seed::parse("album:5"), None);
}

#[test]
fn smart_shuffle_suggests_other_artists_in_the_playlists_style() {
    let mut r = raw();
    // r2's songs are liked: they come after songs that aren't.
    for t in r.tracks.iter_mut().filter(|t| t.artist_id == "r2") {
        t.rating = Some(10.0);
    }
    // r3 is listed as similar to r0 by Plex.
    r.artists.iter_mut().find(|a| a.id == "r0").unwrap().similar = vec!["Artist r3".into()];
    let lib = lib_of(r, &opts());
    let taste = Taste::new(&lib);
    let playlist: Vec<String> = (0..6).map(|t| format!("r0-al0-t{t}")).collect();
    let avoid: HashSet<String> = playlist.iter().cloned().collect();
    let artists: HashSet<String> = ["r0".to_string()].into();
    let picks = radio::suggestions(&lib, &taste, &playlist, &avoid, &artists, 6, 1);
    assert_eq!(picks.len(), 6);
    let ids: Vec<&str> = picks.iter().map(|t| lib.tracks[*t as usize].id.as_str()).collect();
    assert!(ids.iter().all(|id| !id.starts_with("r0-")), "no songs by the playlist's own artist: {ids:?}");
    assert!(ids.iter().all(|id| !avoid.contains(*id)));
    let rock = ids.iter().filter(|id| id.starts_with('r')).count();
    assert!(rock * 2 > ids.len(), "mostly in the playlist's style: {ids:?}");
    assert!(ids.iter().all(|id| !id.starts_with("r2-")), "not-liked songs first: {ids:?}");
    let mut per: HashMap<&str, usize> = HashMap::new();
    for t in &picks {
        *per.entry(artist_of(&lib, *t)).or_default() += 1;
    }
    assert!(per.values().all(|n| *n <= 3), "a few per artist: {per:?}");
    // Each rebuild picks a fresh set.
    let again = radio::suggestions(&lib, &taste, &playlist, &avoid, &artists, 6, 99);
    assert_ne!(again, picks);
    // Hidden artists and the unreleased setting keep songs out.
    let mut hidden = crate::hidden::HiddenSet::default();
    hidden.artists.insert("r1".into());
    let mut r = raw();
    r.albums.iter_mut().find(|a| a.id == "r3-al0").unwrap().unreleased = true;
    let o = MixOptions { hidden: std::sync::Arc::new(hidden), include_unreleased: false, ..opts() };
    let lib = lib_of(r, &o);
    let taste = Taste::new(&lib);
    for t in radio::suggestions(&lib, &taste, &playlist, &avoid, &artists, 40, 2) {
        let id = &lib.tracks[t as usize].id;
        assert!(!id.starts_with("r1-") && !id.starts_with("r3-al0") && !id.starts_with("r0-"), "{id}");
    }
}

#[test]
fn hidden_songs_and_artists_stay_out_of_mixes_and_radio() {
    let mut hidden = crate::hidden::HiddenSet::default();
    hidden.artists.insert("r1".into());
    hidden.tracks.insert("r0-al0-t3".into());
    let o = MixOptions { hidden: std::sync::Arc::new(hidden), ..opts() };
    let lib = lib_of(raw(), &o);
    let banned = |id: &str| id.starts_with("r1-") || id == "r0-al0-t3";
    for t in &lib.tracks {
        assert_eq!(t.eligible, !banned(&t.id), "{}", t.id);
    }
    // Radio and "keep playing similar music" (a radio of the last songs).
    let taste = Taste::new(&lib);
    for seed in [Seed::Track("r0-al0-t0".into()), Seed::Artist("r0".into()), Seed::Tracks(vec!["r0-al0-t0".into()])] {
        let b = radio::batch(&lib, &taste, &seed, &HashSet::new(), 40, 5, true, radio::LEAN_PLAIN);
        assert!(!b.is_empty());
        assert!(b.iter().all(|t| !banned(&lib.tracks[*t as usize].id)));
    }
    // Hiding doesn't change the fingerprint (no remaking every mix).
    assert_eq!(o.fingerprint(), opts().fingerprint());
}

#[tokio::test]
async fn a_song_hidden_after_a_mix_was_made_leaves_its_page() {
    let (s, _db, _d) = service();
    s.ensure_fresh_blocking(NOW).unwrap();
    let ids = crate::hidden::HiddenIds::default();
    s.set_hidden_ids(ids.clone());
    let before = s.page("daily:1".into(), None).await.unwrap();
    let gone = before.tracks[0].id.clone();
    let mut h = crate::hidden::HiddenSet { version: 1, ..Default::default() };
    h.tracks.insert(gone.clone());
    ids.set(h);
    let after = s.page("daily:1".into(), None).await.unwrap();
    assert_eq!(after.tracks.len() + 1, before.tracks.len());
    assert_eq!(after.familiar.len(), after.tracks.len());
    assert!(after.tracks.iter().all(|t| t.id != gone));
    assert_eq!(after.mix.track_count as usize, after.tracks.len());
    // Mixes made from now on leave it out too.
    let lib = s.lib().unwrap();
    assert!(!lib.tracks[lib.index[&gone] as usize].eligible);
}

#[test]
fn exclusions_and_filters_keep_songs_out() {
    let mut r = raw();
    r.tracks.iter_mut().find(|t| t.id == "r0-al0-t1").unwrap().explicit = true;
    r.tracks.iter_mut().find(|t| t.id == "r0-al0-t2").unwrap().edit = true;
    r.albums.iter_mut().find(|a| a.id == "r1-al0").unwrap().unreleased = true;
    let o = MixOptions {
        exclude_artists: vec!["j0".into()],
        exclude_albums: vec!["r0-al1".into()],
        exclude_genres: vec!["hip-hop".into()],
        include_unreleased: false,
        tag_view: crate::tags::TagView { filter_explicit: true, show_edits: false, group_editions: true },
        ..opts()
    };
    let lib = lib_of(r, &o);
    let banned = |id: &str| {
        id.starts_with("j0-") || id.starts_with("r0-al1") || id.starts_with('h') || id.starts_with("r1-al0") || id == "r0-al0-t1" || id == "r0-al0-t2"
    };
    let all = engine::generate_all(&lib, &ctx());
    assert!(!all.is_empty());
    for m in &all {
        for (t, _) in &m.tracks {
            assert!(!banned(&lib.tracks[*t as usize].id), "{} has {}", m.title, lib.tracks[*t as usize].id);
        }
    }
    let taste = Taste::new(&lib);
    for t in radio::batch(&lib, &taste, &Seed::Track("r0-al0-t0".into()), &HashSet::new(), 20, 5, true, radio::LEAN_PLAIN) {
        assert!(!banned(&lib.tracks[t as usize].id));
    }
    // Shown again when allowed.
    let lib = lib_of(raw(), &opts());
    assert!(lib.tracks.iter().all(|t| t.eligible));
}

#[test]
fn plex_history_counts_but_not_twice_for_a_reported_play() {
    let mut r = raw();
    r.listens.push(("j3-al0-t0".into(), NOW - HOUR, false));
    r.history.push(("j3-al0-t0".into(), NOW - HOUR + 60_000));
    r.history.push(("j3-al0-t0".into(), NOW - 5 * DAY));
    let lib = lib_of(r, &opts());
    assert_eq!(lib.tracks[lib.index["j3-al0-t0"] as usize].plays, 2);
}

#[test]
fn all_time_top_songs_rank_by_plays() {
    let mut r = raw();
    r.tracks.iter_mut().find(|t| t.id == "j3-al1-t9").unwrap().view_count = 50;
    let lib = lib_of(r, &opts());
    let top = engine::top_songs(&lib).unwrap();
    assert_eq!(lib.tracks[top.tracks[0].0 as usize].id, "j3-al1-t9");
    let plays: Vec<u32> = top.tracks.iter().map(|(t, _)| lib.tracks[*t as usize].plays).collect();
    assert!(plays.windows(2).all(|w| w[0] >= w[1]));
    assert!(plays.iter().all(|p| *p > 0));
    assert!(top.tracks.len() <= engine::TOP_SONGS_LEN);
}

#[test]
fn the_same_day_gives_the_same_mixes() {
    let lib = lib_of(raw(), &opts());
    assert_eq!(engine::generate_all(&lib, &ctx()), engine::generate_all(&lib, &ctx()));
}

// ------------------------------------------------------------------ cache

pub(crate) fn fill_db(db: &crate::db::DbHandle, raw: &Raw) {
    db.write(|c| {
        let tx = c.transaction()?;
        {
            let mut ar = tx.prepare("INSERT INTO artists (id, title, sort_title, thumb) VALUES (?1, ?2, ?2, ?3)")?;
            for a in &raw.artists {
                ar.execute(rusqlite::params![a.id, a.title, a.thumb])?;
            }
            let mut al = tx.prepare(
                "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb, added_at, genres, moods)
                 VALUES (?1, ?1, ?1, ?2, '', ?3, ?4, ?5, ?6)",
            )?;
            for a in &raw.albums {
                let pick = |k: TagKind| serde_json::to_string(&a.tags.iter().filter(|t| t.0 == k).map(|t| &t.1).collect::<Vec<_>>()).unwrap();
                al.execute(rusqlite::params![a.id, a.artist_id, a.thumb, a.added_at, pick(TagKind::Genre), pick(TagKind::Mood)])?;
            }
            let mut tr = tx.prepare(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, duration_ms,
                   part_key, view_count, last_viewed_at, thumb)
                 VALUES (?1, ?1, ?2, ?2, ?3, '', ?4, ?5, '/p/' || ?1, ?6, ?7, '/cover/' || ?2)",
            )?;
            for t in &raw.tracks {
                tr.execute(rusqlite::params![t.id, t.album_id, t.artist_id, t.track_number, t.duration_ms as i64, t.view_count, t.last_viewed_at])?;
            }
            let mut ll = tx.prepare("INSERT INTO listen_log (rating_key, started_at, skipped, completed) VALUES (?1, ?2, ?3, 1)")?;
            for (id, at, skipped) in &raw.listens {
                ll.execute(rusqlite::params![id, at, skipped])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .unwrap();
}

fn service() -> (MixService, crate::db::DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::db::DbHandle::open(dir.path().join("m.db")).unwrap();
    fill_db(&db, &raw());
    (MixService::new(db.clone()), db, dir)
}

fn stored_day(db: &crate::db::DbHandle) -> Option<String> {
    db.read(|c| store::get_kv(c, store::KEY_DAY)).unwrap()
}

#[test]
fn mixes_are_made_once_a_day_and_the_daylist_by_its_block_of_hours() {
    let (s, db, _d) = service();
    assert!(s.ensure_fresh_blocking(NOW).unwrap());
    assert_eq!(stored_day(&db).as_deref(), Some("2026-10-14"));
    let n = db.read(store::count).unwrap();
    assert!(n > 4, "{n} mixes");
    assert!(!s.ensure_fresh_blocking(NOW + 30 * 60_000).unwrap(), "nothing due yet");
    // Next daylist block: only the daylist is made again.
    let before = db.read(|c| store::tracks(c, "daily:1")).unwrap();
    assert!(s.ensure_fresh_blocking(NOW + 3 * HOUR).unwrap());
    assert_eq!(db.read(|c| store::tracks(c, "daily:1")).unwrap(), before);
    assert_eq!(stored_day(&db).as_deref(), Some("2026-10-14"));
    // After 4 am the next day: all of them.
    assert!(s.ensure_fresh_blocking(NOW + 17 * HOUR).unwrap());
    assert_eq!(stored_day(&db).as_deref(), Some("2026-10-15"));
    // New settings: made again.
    s.set_options(MixOptions { exclude_artists: vec!["r0".into()], ..MixOptions::default() });
    assert!(s.ensure_fresh_blocking(NOW + 17 * HOUR).unwrap());
    let all = db.read(store::all).unwrap();
    assert!(all.iter().all(|(_, ts, _)| ts.iter().all(|t| t.artist_id != "r0")));
}

#[tokio::test]
async fn shelves_pages_steering_and_refresh() {
    let (s, db, _d) = service();
    s.ensure_fresh_blocking(NOW).unwrap();
    let shelves = db.read(store::all).unwrap();
    let first = &shelves[0].0;
    assert_eq!(first.id, "daily:1");
    assert_eq!(first.art.as_deref(), Some(format!("/thumb/{}", first.artist_ids[0]).as_str()), "the leading artist's photo");
    assert!(first.description.ends_with("and more"));
    assert_eq!(first.track_count as usize, shelves[0].1.len());
    let page = s.page("daily:1".into(), None).await.unwrap();
    assert_eq!(page.tracks.len(), page.familiar.len());
    let fam = |p: &MixPage| p.familiar.iter().filter(|f| **f).count();
    let more = s.page("daily:1".into(), Some(MixSteer::Familiar)).await.unwrap();
    let less = s.page("daily:1".into(), Some(MixSteer::New)).await.unwrap();
    assert!(fam(&more) >= fam(&less), "{} vs {}", fam(&more), fam(&less));
    assert_eq!(more.steer, Some(MixSteer::Familiar));
    // Steering doesn't change the stored mix.
    assert_eq!(s.page("daily:1".into(), None).await.unwrap().tracks, page.tracks);
    let shelves = s.shelves().await.unwrap();
    assert!(shelves.made_for_you.iter().any(|m| m.kind == MixKind::Daily));
    assert!(shelves.moods.iter().all(|m| m.kind == MixKind::Mood));
    assert!(shelves.genres.iter().any(|m| m.title == "Rock Mix"));
    assert!(!shelves.made_for_you.iter().any(|m| m.kind == MixKind::TopSongs), "Top Songs stays off Made for you");
    let daylist = shelves.made_for_you.iter().find(|m| m.kind == MixKind::Daylist).unwrap();
    assert!(daylist.next_update_at.is_some_and(|t| t > now_ms()));
    let radio = s.radio_page("track:r0-al0-t0".into(), None, Some(1)).await.unwrap();
    assert_eq!(radio.mix.title, "r0-al0-t0 Radio");
    assert_eq!(radio.tracks[0].id, "r0-al0-t0", "starts with its song");
    assert!(radio.mix.description.starts_with("with Artist r0, "), "{}", radio.mix.description);
    assert_eq!(radio.mix.covers.len(), 3, "the song's cover and two artists");
    assert_eq!(radio.mix.kind, MixKind::Radio);
    let fam = |p: &MixPage| p.familiar.iter().filter(|f| **f).count();
    let lean_new = s.radio_page("track:r0-al0-t0".into(), Some(MixSteer::New), Some(1)).await.unwrap();
    let lean_fam = s.radio_page("track:r0-al0-t0".into(), Some(MixSteer::Familiar), Some(1)).await.unwrap();
    assert!(fam(&lean_fam) >= fam(&lean_new));
    let artist = s.radio_page("artist:j1".into(), None, Some(1)).await.unwrap();
    assert_eq!(artist.mix.title, "Artist j1 Radio");
    assert!(s.radio_page("album:1".into(), None, Some(1)).await.is_err());
    let refreshed = s.refresh("genre:rock".into()).await.unwrap();
    assert_eq!(refreshed.mix.id, "genre:rock");
    assert!(s.page("nope".into(), None).await.is_err());
}

#[tokio::test]
async fn mix_and_radio_contexts_resolve_through_the_cache() {
    use crate::audio::source::{ContextKind, PlayContext, TrackLookup};
    let (s, db, _d) = service();
    s.ensure_fresh_blocking(NOW).unwrap();
    let lookup = crate::bridge::CacheLookup(db.clone(), None, Default::default(), Some(s.clone()));
    let mix = lookup
        .context(&PlayContext { kind: ContextKind::Mix, id: Some("daily:1".into()), track_ids: None })
        .await
        .unwrap();
    assert_eq!(mix.title.as_deref(), Some("Daily Mix 1"));
    assert_eq!(mix.tracks, db.read(|c| store::tracks(c, "daily:1")).unwrap().0);
    let steered = lookup
        .context(&PlayContext { kind: ContextKind::Mix, id: Some("daily:1".into()), track_ids: Some(vec!["j0-al0-t0".into()]) })
        .await
        .unwrap();
    assert_eq!(steered.tracks.len(), 1);
    let r = lookup
        .context(&PlayContext { kind: ContextKind::Radio, id: Some("track:r0-al0-t0".into()), track_ids: None })
        .await
        .unwrap();
    assert_eq!(r.title.as_deref(), Some("r0-al0-t0 Radio"));
    assert_eq!(r.tracks[0].id, "r0-al0-t0");
    let ar = lookup
        .context(&PlayContext { kind: ContextKind::Radio, id: Some("artist:j1".into()), track_ids: None })
        .await
        .unwrap();
    assert_eq!(ar.title.as_deref(), Some("Artist j1 Radio"));
    let more = lookup
        .more(&crate::audio::source::MoreRequest {
            kind: ContextKind::Radio,
            id: Some("track:r0-al0-t0".into()),
            seed_ids: vec![],
            avoid: r.tracks.iter().map(|t| t.id.clone()).collect(),
            n: 10,
            smart: false,
            avoid_artists: vec![],
        })
        .await
        .unwrap();
    assert!(!more.is_empty());
    assert!(more.iter().all(|t| !r.tracks.iter().any(|o| o.id == t.id)));
}

// ------------------------------------------------------------------ benchmark

/// 100k songs, 5k artists, 10k albums, 40 genres and 30 moods, 30k plays.
pub(crate) fn big_raw() -> Raw {
    let mut raw = Raw::default();
    let mut x: u64 = 42;
    let mut rnd = move |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    for a in 0..5_000 {
        raw.artists.push(ArtistRow { id: format!("a{a}"), title: format!("Artist {a}"), thumb: None, similar: vec![] });
    }
    for al in 0..10_000u64 {
        let artist = al / 2;
        let g = artist % 40;
        let tags = vec![
            (TagKind::Genre, format!("Genre {g}")),
            (TagKind::Style, format!("Style {}", (g * 3 + rnd(3)) % 90)),
            (TagKind::Mood, format!("Mood {}", rnd(30))),
        ];
        raw.albums.push(RawAlbum { id: format!("al{al}"), artist_id: format!("a{artist}"), thumb: None, added_at: (NOW / 1000) - rnd(400) as i64 * 86_400, tags, unreleased: false });
        for t in 0..10 {
            let id = format!("t{al}-{t}");
            raw.tracks.push(RawTrack {
                id,
                artist_id: format!("a{artist}"),
                album_id: format!("al{al}"),
                track_number: t + 1,
                duration_ms: 200_000,
                view_count: if rnd(5) == 0 { rnd(20) as u32 } else { 0 },
                rating: (rnd(50) == 0).then_some(10.0),
                ..Default::default()
            });
        }
    }
    for k in 0..30_000i64 {
        let al = rnd(600);
        let id = format!("t{al}-{}", rnd(10));
        raw.listens.push((id, NOW - 200 * DAY + k * 9 * 60_000, rnd(8) == 0));
    }
    raw
}

#[test]
#[ignore = "benchmark: 100k songs; run with --ignored --nocapture"]
fn bench_daily_regeneration_on_100k_songs() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::db::DbHandle::open(dir.path().join("big.db")).unwrap();
    fill_db(&db, &big_raw());
    let s = MixService::new(db.clone());
    let t = std::time::Instant::now();
    assert!(s.ensure_fresh_blocking(NOW).unwrap());
    let full = t.elapsed();
    let n = db.read(store::count).unwrap();
    // A radio batch on the cached snapshot.
    let t = std::time::Instant::now();
    let ids = s.radio_ids(&Seed::Track("t5-0".into()), &HashSet::new(), 20, true).unwrap();
    let radio = t.elapsed();
    println!("daily regeneration (load + make + store) of {n} mixes on 100k songs: {full:?}; radio batch: {radio:?}");
    assert_eq!(ids.len(), 20);
    assert!(full < std::time::Duration::from_secs(2), "took {full:?}");
}
