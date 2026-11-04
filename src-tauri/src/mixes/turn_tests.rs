use std::collections::HashSet;

use super::*;
use crate::mixes::MixOptions;
use crate::mixes::library::{ArtistRow, Raw, RawAlbum, RawTrack};

/// Rock: four artists, an Upbeat album and a Mellow album each. Jazz: four
/// artists, one Mellow album and one Dark album each. Folk: one artist, a
/// Sad album of five songs (too few to offer).
fn raw() -> Raw {
    let mut raw = Raw::default();
    let mut add = |artist: &str, album: &str, genre: &str, mood: &str, n: u32| {
        if !raw.artists.iter().any(|a| a.id == artist) {
            raw.artists.push(ArtistRow { id: artist.into(), title: artist.into(), thumb: None, similar: vec![] });
        }
        raw.albums.push(RawAlbum {
            id: album.into(),
            artist_id: artist.into(),
            thumb: None,
            added_at: 1_000,
            tags: vec![(TagKind::Genre, genre.into()), (TagKind::Mood, mood.into())],
            unreleased: false,
        });
        for t in 0..n {
            raw.tracks.push(RawTrack {
                id: format!("{album}-t{t}"),
                artist_id: artist.into(),
                album_id: album.into(),
                track_number: t + 1,
                duration_ms: 200_000,
                ..Default::default()
            });
        }
    };
    for a in 0..4 {
        add(&format!("rock{a}"), &format!("rock{a}-up"), "Rock", "Upbeat", 6);
        add(&format!("rock{a}"), &format!("rock{a}-mellow"), "Rock", "Mellow", 6);
        add(&format!("jazz{a}"), &format!("jazz{a}-mellow"), "Jazz", "Mellow", 6);
        add(&format!("jazz{a}"), &format!("jazz{a}-dark"), "Jazz", "Dark", 6);
    }
    add("folk", "folk-sad", "Folk", "Sad", 5);
    raw
}

fn lib() -> Lib {
    Lib::build(raw(), &MixOptions::default(), 10.0)
}

const SEED: &str = "rock0-up-t0";

fn mood_of(lib: &Lib, t: u32) -> Vec<String> {
    lib.tracks[t as usize].tags.iter().filter(|g| lib.tag_kinds[**g as usize] == TagKind::Mood).map(|g| lib.tags[*g as usize].clone()).collect()
}

#[test]
fn offers_other_moods_nearest_first_and_skips_small_ones() {
    let lib = lib();
    let c = choices(&lib, SEED);
    let names: Vec<&str> = c.iter().map(|x| x.mood.as_str()).collect();
    assert_eq!(names, ["Mellow", "Dark"], "its own mood isn't offered, Mellow shares Rock so it's nearer, Sad is too small");
    assert_eq!(c[0].songs, 48);
}

#[test]
fn no_choices_for_a_song_not_in_the_library() {
    assert!(choices(&lib(), "nope").is_empty());
}

#[test]
fn a_turn_picks_songs_in_the_mood_near_the_playing_song() {
    let lib = lib();
    let taste = Taste::new(&lib);
    let picked = songs(&lib, &taste, SEED, "mellow", &HashSet::new(), TURN_LEN, 7);
    assert_eq!(picked.len(), TURN_LEN);
    assert!(picked.iter().all(|t| mood_of(&lib, *t) == ["Mellow"]), "every song is Mellow");
    let rock = picked.iter().filter(|t| lib.tracks[**t as usize].id.starts_with("rock")).count();
    assert!(rock >= TURN_LEN / 2, "mostly the same genre: {rock} of {TURN_LEN}");
    let mut per = std::collections::HashMap::new();
    for t in &picked {
        *per.entry(lib.tracks[*t as usize].artist).or_insert(0) += 1;
    }
    assert!(per.values().all(|n| *n <= 2), "two songs per artist at most");
    assert!(!picked.contains(&lib.index[SEED]));
}

#[test]
fn a_turn_leaves_out_what_is_already_queued() {
    let lib = lib();
    let taste = Taste::new(&lib);
    let first = songs(&lib, &taste, SEED, "Mellow", &HashSet::new(), TURN_LEN, 7);
    let avoid: HashSet<String> = first.iter().map(|t| lib.tracks[*t as usize].id.clone()).collect();
    let second = songs(&lib, &taste, SEED, "Mellow", &avoid, TURN_LEN, 7);
    assert_eq!(second.len(), TURN_LEN);
    assert!(second.iter().all(|t| !avoid.contains(&lib.tracks[*t as usize].id)));
}

#[test]
fn a_mood_the_library_lacks_gives_nothing() {
    let lib = lib();
    let taste = Taste::new(&lib);
    assert!(songs(&lib, &taste, SEED, "Furious", &HashSet::new(), TURN_LEN, 7).is_empty());
    assert!(songs(&lib, &taste, "nope", "Mellow", &HashSet::new(), TURN_LEN, 7).is_empty());
}

#[test]
fn a_small_mood_still_fills_from_far_away() {
    let lib = lib();
    let taste = Taste::new(&lib);
    // Dark shares nothing with the rock song but its mood: still eight songs.
    let picked = songs(&lib, &taste, SEED, "Dark", &HashSet::new(), TURN_LEN, 7);
    assert_eq!(picked.len(), TURN_LEN);
    assert!(picked.iter().all(|t| mood_of(&lib, *t) == ["Dark"]));
}
