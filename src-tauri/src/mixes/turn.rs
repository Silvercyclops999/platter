//! Turn it: one tap on what's playing moves the next few songs toward a
//! different mood, all from your own library, then the queue carries on as
//! it was. Nothing changes until the listener picks a mood.
//!
//! The moods offered are the library's own mood tags (from the server) that
//! the playing song doesn't already have, nearest first: a mood counts as
//! near when many of its songs share a genre or style with the playing song,
//! so a turn feels like a turn and not a jump. The songs picked for a mood
//! are radio's neighbours of the playing song that carry that mood, topped
//! up from the rest of the mood when there are too few.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::engine::{Taste, jitter, space_artists};
use super::library::{Lib, TagKind};
use super::radio::{self, LEAN_PLAIN};

/// Songs a turn puts up next.
pub const TURN_LEN: usize = 8;
/// Most moods offered.
pub const TURN_CHOICES: usize = 4;
/// A mood needs this many songs to be offered.
pub const TURN_MIN_SONGS: usize = 12;
/// Most songs by one artist in a turn.
const PER_ARTIST: usize = 2;

/// A mood the next songs can turn toward.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnChoice {
    /// The mood's name, as the server tags it ("Mellow").
    pub mood: String,
    /// Songs in the library with it.
    pub songs: usize,
}

fn eligible(lib: &Lib, t: u32) -> bool {
    let tr = &lib.tracks[t as usize];
    tr.eligible && !tr.filler && lib.affinity(t) > -1.0
}

/// The moods to offer for the song `track_id`, nearest first. Empty when the
/// song isn't in the library or the library has no moods to speak of.
pub fn choices(lib: &Lib, track_id: &str) -> Vec<TurnChoice> {
    let Some(&seed) = lib.index.get(track_id) else { return Vec::new() };
    let own: HashSet<u16> = lib.tracks[seed as usize].tags.iter().copied().collect();
    let near_tags: HashSet<u16> = own.iter().copied().filter(|g| lib.tag_kinds[*g as usize] != TagKind::Mood).collect();
    let mut out: Vec<(f32, TurnChoice)> = Vec::new();
    for (g, kind) in lib.tag_kinds.iter().enumerate() {
        let g = g as u16;
        if *kind != TagKind::Mood || own.contains(&g) {
            continue;
        }
        let songs: Vec<u32> = lib.by_tag[g as usize].iter().copied().filter(|t| eligible(lib, *t)).collect();
        if songs.len() < TURN_MIN_SONGS {
            continue;
        }
        let near = songs.iter().filter(|t| lib.tracks[**t as usize].tags.iter().any(|x| near_tags.contains(x))).count();
        // Mostly how many of its songs sit near this one, a little its size.
        let score = near as f32 + 0.1 * songs.len() as f32;
        out.push((score, TurnChoice { mood: lib.tags[g as usize].clone(), songs: songs.len() }));
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.mood.cmp(&b.1.mood)));
    out.into_iter().take(TURN_CHOICES).map(|(_, c)| c).collect()
}

/// The songs to put up next for a turn from `track_id` toward `mood`
/// (matched without case). `avoid`: songs already queued or just played.
pub fn songs(lib: &Lib, taste: &Taste, track_id: &str, mood: &str, avoid: &HashSet<String>, n: usize, salt: u64) -> Vec<u32> {
    let Some(&seed) = lib.index.get(track_id) else { return Vec::new() };
    let Some(g) = lib
        .tags
        .iter()
        .enumerate()
        .position(|(i, name)| lib.tag_kinds[i] == TagKind::Mood && name.eq_ignore_ascii_case(mood))
    else {
        return Vec::new();
    };
    let in_mood: HashSet<u32> = lib.by_tag[g].iter().copied().filter(|t| eligible(lib, *t) && *t != seed).collect();
    let usable = |t: &u32| in_mood.contains(t) && !avoid.contains(&lib.tracks[*t as usize].id);
    // Radio's neighbours of the playing song that carry the mood, best first.
    let mut scored: Vec<(u32, f32)> = radio::score(lib, taste, &[seed], &[], avoid, salt, false, LEAN_PLAIN)
        .into_iter()
        .filter(|(t, _)| usable(t))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    // Too few nearby: the rest of the mood, songs you like first.
    if scored.len() < n {
        let have: HashSet<u32> = scored.iter().map(|x| x.0).collect();
        let mut rest: Vec<(u32, f32)> = in_mood
            .iter()
            .copied()
            .filter(|t| usable(t) && !have.contains(t))
            .map(|t| (t, 0.2 * lib.affinity(t) + 0.3 * jitter(salt, t)))
            .collect();
        rest.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.extend(rest);
    }
    let mut out: Vec<u32> = Vec::with_capacity(n);
    let mut per: HashMap<u32, usize> = HashMap::new();
    for &(t, _) in &scored {
        if out.len() >= n {
            break;
        }
        let c = per.entry(lib.tracks[t as usize].artist).or_default();
        if *c >= PER_ARTIST {
            continue;
        }
        *c += 1;
        out.push(t);
    }
    // A small library: fill past the per-artist limit rather than stop short.
    for &(t, _) in &scored {
        if out.len() >= n {
            break;
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    let mut spaced: Vec<(u32, bool)> = out.into_iter().map(|t| (t, true)).collect();
    space_artists(lib, &mut spaced, 1);
    spaced.into_iter().map(|(t, _)| t).collect()
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
