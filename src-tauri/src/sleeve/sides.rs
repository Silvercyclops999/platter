//! A record's sides: which songs are on side A, side B and so on.
//!
//! Sides come from the vinyl (or cassette) edition the sleeve notes read:
//! MusicBrainz track numbers as printed ("A1", "B3") and Discogs track
//! positions. When neither has them the user can split the album by hand
//! (the songs where each side after the first starts), kept in the sync
//! store per album. A split by hand with no starts means "no sides" for
//! that album. Sides never change what plays or in which order; they only
//! label the tracklist, play one side, and let the record player pause at
//! the end of a side.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::LibSong;
use super::discogs::DiscogsRelease;
use super::gather::{self, DISCOGS_ROW};
use crate::artistinfo::cache;
use crate::artistinfo::mbdetails::{LibTrack, MbRelease, match_tracks};
use crate::artistinfo::service::album_key;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;
use crate::sync::store::ALBUM_SIDES;

/// Most sides an album can have (a 13 record box set).
pub const MAX_SIDES: usize = 26;

/// What the sync store keeps for an album split by hand (keyed by album id).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManualSides {
    /// The song each side after the first starts on, in album order. Empty:
    /// the album has no sides.
    pub starts: Vec<String>,
}

/// Where an album's sides came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SidesSource {
    Musicbrainz,
    Discogs,
    Manual,
}

/// One side: "A", "B"... and its songs in album order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub label: String,
    pub track_ids: Vec<String>,
}

/// IPC `AlbumSides`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSides {
    pub album_id: String,
    /// Empty when the album has no sides.
    pub sides: Vec<Side>,
    pub source: Option<SidesSource>,
    /// A vinyl edition's sides are known (shown unless split by hand).
    pub detected: bool,
}

/// The side letter of a printed position: "A1" and "B" are sides A and B,
/// "AA" the other side of a single. CD positions ("3", "1-4", "CD1-2") have none.
pub fn side_letter(position: &str) -> Option<String> {
    let p = position.trim();
    let letters: String = p.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rest = &p[letters.len()..];
    let up = letters.to_ascii_uppercase();
    let one_side = up.len() == 1 || (up.len() == 2 && up.as_bytes()[0] == up.as_bytes()[1]);
    // What follows is the track on that side: digits, then maybe a sub-track letter or ".1".
    let tail_ok = rest.is_empty()
        || (rest.starts_with(|c: char| c.is_ascii_digit())
            && rest.trim_start_matches(|c: char| c.is_ascii_digit()).chars().all(|c| c.is_ascii_lowercase() || c == '.' || c.is_ascii_digit()));
    (one_side && tail_ok).then_some(up)
}

/// Label for the nth side (0 is "A").
fn label(i: usize) -> String {
    char::from(b'A' + i as u8).to_string()
}

/// The album's songs grouped into sides from a side key per song (None
/// where the edition didn't name one). Songs without a key go with the side
/// before them (the first side for leading ones). None unless at least half
/// the songs have a key, each side is one run of songs, and there are two
/// or more. Sides are labelled A, B, C... in order, whatever the edition
/// printed (a second record that starts again at A becomes C).
pub fn group(songs: &[LibSong], keys: &HashMap<String, (u32, String)>) -> Option<Vec<Side>> {
    if songs.is_empty() || keys.len() * 2 < songs.len() {
        return None;
    }
    let first = songs.iter().find_map(|s| keys.get(&s.id))?.clone();
    let mut seen: Vec<(u32, String)> = Vec::new();
    let mut sides: Vec<Side> = Vec::new();
    let mut cur = first;
    for s in songs {
        if let Some(k) = keys.get(&s.id) {
            cur = k.clone();
        }
        if seen.last() != Some(&cur) {
            if seen.contains(&cur) {
                return None;
            }
            seen.push(cur.clone());
            sides.push(Side { label: label(sides.len().min(MAX_SIDES - 1)), track_ids: vec![] });
        }
        sides.last_mut()?.track_ids.push(s.id.clone());
    }
    (sides.len() >= 2 && sides.len() <= MAX_SIDES).then_some(sides)
}

/// Sides from a MusicBrainz release whose track numbers name them.
pub fn from_musicbrainz(r: &MbRelease, songs: &[LibSong]) -> Option<Vec<Side>> {
    if !r.tracks.iter().any(|t| t.number.as_deref().and_then(side_letter).is_some()) {
        return None;
    }
    let lib: Vec<LibTrack> = songs.iter().map(|s| LibTrack { id: s.id.clone(), title: s.title.clone(), disc: s.disc, number: s.number }).collect();
    let ids = match_tracks(&r.tracks, &lib);
    let mut keys = HashMap::new();
    for (t, id) in r.tracks.iter().zip(ids) {
        if let (Some(id), Some(l)) = (id, t.number.as_deref().and_then(side_letter)) {
            keys.entry(id).or_insert((t.disc, l));
        }
    }
    group(songs, &keys)
}

/// Sides from a Discogs release whose positions name them, its tracks
/// matched by order when the counts agree, else by title.
pub fn from_discogs(r: &DiscogsRelease, songs: &[LibSong]) -> Option<Vec<Side>> {
    use crate::share::matching::title_key;
    if !r.tracks.iter().any(|t| side_letter(&t.position).is_some()) {
        return None;
    }
    let by_order = r.tracks.len() == songs.len();
    let mut keys = HashMap::new();
    for (i, t) in r.tracks.iter().enumerate() {
        let Some(l) = side_letter(&t.position) else { continue };
        let song = if by_order {
            songs.get(i)
        } else {
            let k = title_key(&t.title);
            songs.iter().find(|s| !k.is_empty() && title_key(&s.title) == k)
        };
        if let Some(s) = song {
            keys.entry(s.id.clone()).or_insert((0, l));
        }
    }
    group(songs, &keys)
}

/// Sides from a split by hand: a new side at each start (starts that
/// aren't the album's songs, or are its first song, are left out).
pub fn from_manual(m: &ManualSides, songs: &[LibSong]) -> Vec<Side> {
    if m.starts.is_empty() || songs.is_empty() {
        return vec![];
    }
    let mut sides = vec![Side { label: label(0), track_ids: vec![] }];
    for (i, s) in songs.iter().enumerate() {
        if i > 0 && sides.len() < MAX_SIDES && m.starts.contains(&s.id) {
            sides.push(Side { label: label(sides.len()), track_ids: vec![] });
        }
        if let Some(last) = sides.last_mut() {
            last.track_ids.push(s.id.clone());
        }
    }
    if sides.len() < 2 { vec![] } else { sides }
}

/// An album's sides: a split by hand wins, then MusicBrainz, then Discogs.
pub fn resolve(album_id: &str, songs: &[LibSong], mb: Option<&MbRelease>, discogs: Option<&DiscogsRelease>, manual: Option<&ManualSides>) -> AlbumSides {
    let detected = mb
        .and_then(|r| from_musicbrainz(r, songs).map(|s| (SidesSource::Musicbrainz, s)))
        .or_else(|| discogs.and_then(|r| from_discogs(r, songs).map(|s| (SidesSource::Discogs, s))));
    let has_detected = detected.is_some();
    let (source, sides) = match (manual, detected) {
        (Some(m), _) => {
            let s = from_manual(m, songs);
            (Some(SidesSource::Manual), s)
        }
        (None, Some((src, s))) => (Some(src), s),
        (None, None) => (None, vec![]),
    };
    AlbumSides { album_id: album_id.to_owned(), sides, source, detected: has_detected }
}

/// Checks a split by hand against the album's songs and puts it in album
/// order: every start is one of its songs, never the first.
pub fn clean_starts(starts: &[String], songs: &[LibSong]) -> AppResult<ManualSides> {
    if let Some(bad) = starts.iter().find(|id| !songs.iter().any(|s| &s.id == *id)) {
        return Err(AppError::Invalid(format!("That song isn't on this album ({bad}).")));
    }
    if songs.first().is_some_and(|f| starts.contains(&f.id)) {
        return Err(AppError::Invalid("Side A starts on the first song.".into()));
    }
    let ordered: Vec<String> = songs.iter().filter(|s| starts.contains(&s.id)).map(|s| s.id.clone()).collect();
    if ordered.len() + 1 > MAX_SIDES {
        return Err(AppError::Invalid(format!("An album can have up to {MAX_SIDES} sides.")));
    }
    Ok(ManualSides { starts: ordered })
}

/// Album sides: the library cache for the editions, the sync store for
/// splits by hand. Cheap to clone.
#[derive(Clone)]
pub struct Sides {
    db: DbHandle,
    store: SyncStore,
}

impl Sides {
    pub fn new(db: DbHandle, store: SyncStore) -> Self {
        Self { db, store }
    }

    async fn songs_and_editions(&self, album_id: &str) -> AppResult<(Vec<LibSong>, Option<MbRelease>, Option<DiscogsRelease>)> {
        let id = album_id.to_owned();
        self.db
            .read_async(move |c| {
                let Some(b) = gather::basics(c, &id)? else { return Err(AppError::NotFound(format!("album {id}"))) };
                let mb = gather::cached_mb(c, &id)?;
                let discogs = cache::get(c, &album_key(&id), DISCOGS_ROW)?.and_then(|r| r.parse::<DiscogsRelease>());
                Ok((b.songs, mb, discogs))
            })
            .await
    }

    /// An album's sides from what's cached; never looks anything up.
    pub async fn get(&self, album_id: &str) -> AppResult<AlbumSides> {
        let (songs, mb, discogs) = self.songs_and_editions(album_id).await?;
        let key = album_id.to_owned();
        let manual = self.store.run(move |s| s.get(&ALBUM_SIDES, &key)).await?;
        Ok(resolve(album_id, &songs, mb.as_ref(), discogs.as_ref(), manual.as_ref()))
    }

    /// Split an album by hand (`starts`: where each side after the first
    /// starts; empty for no sides), or None to go back to the edition's sides.
    pub async fn set(&self, album_id: &str, starts: Option<Vec<String>>) -> AppResult<AlbumSides> {
        let (songs, _, _) = self.songs_and_editions(album_id).await?;
        let key = album_id.to_owned();
        match starts {
            Some(st) => {
                let m = clean_starts(&st, &songs)?;
                self.store.run(move |s| s.put(&ALBUM_SIDES, &key, &m)).await?;
            }
            None => {
                self.store.run(move |s| s.remove(&ALBUM_SIDES, &key)).await?;
            }
        }
        self.get(album_id).await
    }
}

/// An album's sides (`album_sides`).
#[tauri::command]
pub async fn album_sides(sides: tauri::State<'_, Sides>, id: String) -> AppResult<AlbumSides> {
    sides.get(&id).await
}

/// Split an album into sides by hand, or go back to its edition's sides (`album_sides_set`).
#[tauri::command]
pub async fn album_sides_set(sides: tauri::State<'_, Sides>, id: String, starts: Option<Vec<String>>) -> AppResult<AlbumSides> {
    sides.set(&id, starts).await
}
