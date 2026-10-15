//! Radio: an endless run of songs like a seed (a song, an artist, or what
//! was just playing), 20 at a time. Neighbours come from shared tags, artist
//! similarity (tags, co-listening, Plex similar artists) and songs played
//! together. Songs already in the queue or played lately are left out, and
//! the seed artist keeps to about a third of each batch.

use std::collections::{HashMap, HashSet};

use super::engine::{Taste, jitter, space_artists};
use super::library::Lib;

/// Songs added per refill.
pub const RADIO_BATCH: usize = 20;
/// Songs on a radio page.
pub const RADIO_PAGE_LEN: usize = 50;
/// How much favourites count for: as is, and with a steering chip.
pub const LEAN_PLAIN: f32 = 0.15;
pub const LEAN_FAMILIAR: f32 = 0.7;
pub const LEAN_NEW: f32 = -0.4;
/// Best candidates a batch is drawn from (so batches differ).
const RADIO_POOL: usize = 300;
/// Most songs by one artist in a batch (the seed artist may have more).
const PER_ARTIST: usize = 3;

/// What a radio starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seed {
    Track(String),
    Artist(String),
    /// Songs that were playing (keep playing similar music).
    Tracks(Vec<String>),
}

impl Seed {
    /// From a radio context id: `track:<id>` or `artist:<id>`.
    pub fn parse(id: &str) -> Option<Seed> {
        if let Some(t) = id.strip_prefix("track:") {
            return Some(Seed::Track(t.to_string()));
        }
        id.strip_prefix("artist:").map(|a| Seed::Artist(a.to_string()))
    }
}

/// The next batch. `avoid`: track ids already queued or just played.
/// `first`: this starts the radio (a song radio opens with its song).
/// `lean`: what being a favourite adds to a song's score.
#[allow(clippy::too_many_arguments)]
pub fn batch(lib: &Lib, taste: &Taste, seed: &Seed, avoid: &HashSet<String>, n: usize, salt: u64, first: bool, lean: f32) -> Vec<u32> {
    let seeds: Vec<u32> = match seed {
        Seed::Track(id) => lib.index.get(id).copied().into_iter().collect(),
        Seed::Artist(id) => lib
            .artists
            .iter()
            .position(|a| a.id == *id)
            .map(|a| {
                let mut ts = lib.by_artist[a].clone();
                ts.sort_by(|x, y| lib.affinity(*y).total_cmp(&lib.affinity(*x)).then(x.cmp(y)));
                ts.truncate(10);
                ts
            })
            .unwrap_or_default(),
        Seed::Tracks(ids) => ids.iter().filter_map(|i| lib.index.get(i).copied()).collect(),
    };
    if seeds.is_empty() {
        return Vec::new();
    }
    let seed_artists: HashSet<u32> = seeds.iter().map(|t| lib.tracks[*t as usize].artist).collect();
    let mut tags: HashMap<u16, f32> = HashMap::new();
    for t in &seeds {
        for g in &lib.tracks[*t as usize].tags {
            *tags.entry(*g).or_default() += 1.0;
        }
    }
    let norm = tags.values().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    let mut co: HashMap<u32, f32> = HashMap::new();
    for t in &seeds {
        for (o, w) in lib.co_tracks.get(t).map(Vec::as_slice).unwrap_or(&[]) {
            *co.entry(*o).or_default() += w;
        }
    }
    // Artist closeness to the seed, once per artist.
    let mut artist_close: HashMap<u32, f32> = HashMap::new();
    let mut closeness = |a: u32| -> f32 {
        *artist_close
            .entry(a)
            .or_insert_with(|| seed_artists.iter().map(|s| taste.similarity(lib, a, *s)).fold(0.0f32, f32::max))
    };
    // Candidates: songs sharing a tag with the seed, by its artists, or played with it.
    let mut cand: HashSet<u32> = HashSet::new();
    for g in tags.keys() {
        cand.extend(lib.by_tag[*g as usize].iter().copied());
    }
    for a in &seed_artists {
        cand.extend(lib.by_artist[*a as usize].iter().copied());
    }
    cand.extend(co.keys().copied());
    let seed_set: HashSet<u32> = seeds.iter().copied().collect();
    let mut scored: Vec<(u32, f32)> = cand
        .into_iter()
        .filter(|t| {
            let tr = &lib.tracks[*t as usize];
            tr.eligible && !avoid.contains(&tr.id) && !(seed_set.contains(t) && !first) && lib.affinity(*t) > -1.0
        })
        .map(|t| {
            let tr = &lib.tracks[t as usize];
            let shared: f32 = tr.tags.iter().map(|g| tags.get(g).copied().unwrap_or(0.0)).sum::<f32>()
                / (norm * (tr.tags.len() as f32).sqrt().max(1.0));
            let close = closeness(tr.artist);
            let together = (co.get(&t).copied().unwrap_or(0.0) / 2.0).min(1.0);
            let known = if lib.familiar(t) { lean } else { 0.0 };
            (t, 0.45 * shared + 0.35 * close + 0.25 * together + known + 0.3 * jitter(salt, t))
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(RADIO_POOL);
    let mut out: Vec<u32> = Vec::with_capacity(n);
    if first && let Seed::Track(_) = seed {
        out.push(seeds[0]);
    }
    let seed_cap = (n / 3).max(1);
    let mut per: HashMap<u32, usize> = HashMap::new();
    for &(t, _) in &scored {
        if out.len() >= n {
            break;
        }
        if out.contains(&t) {
            continue;
        }
        let a = lib.tracks[t as usize].artist;
        let cap = if seed_artists.contains(&a) { seed_cap } else { PER_ARTIST };
        let c = per.entry(a).or_default();
        if *c >= cap {
            continue;
        }
        *c += 1;
        out.push(t);
    }
    // A small library: fill up past the per-artist limits rather than stop short.
    for &(t, _) in &scored {
        if out.len() >= n {
            break;
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    let mut spaced: Vec<(u32, bool)> = out.into_iter().map(|t| (t, true)).collect();
    // The seed song stays first.
    let start = usize::from(first && matches!(seed, Seed::Track(_)));
    space_artists(lib, &mut spaced[start..], 2);
    spaced.into_iter().map(|(t, _)| t).collect()
}
