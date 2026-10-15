//! What the library already has, for folding catalogue results into it: a
//! catalogue artist is a library artist when their MusicBrainz ids match or
//! their names fold the same (`identity::fold`).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::artistinfo::is_mbid;
use crate::error::AppResult;
use crate::identity::fold::{fold_album, fold_artist, fold_title};

#[derive(Debug, Clone, Default)]
pub struct LibraryIndex {
    by_mbid: HashMap<String, String>,
    by_fold: HashMap<String, String>,
}

impl LibraryIndex {
    /// Every library artist by folded name, and by MusicBrainz id where the
    /// artist info cache knows it.
    pub fn load(c: &Connection) -> AppResult<Self> {
        let mut ix = Self::default();
        let mut st = c.prepare_cached("SELECT id, title FROM artists ORDER BY view_count DESC")?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, title) = row?;
            let f = fold_artist(&title);
            if !f.is_empty() {
                ix.by_fold.entry(f).or_insert(id);
            }
        }
        let mut st = c.prepare_cached(
            "SELECT artist_id, data FROM artist_info WHERE source = ?1 AND data IS NOT NULL AND artist_id NOT LIKE 'album:%'",
        )?;
        for row in st.query_map([crate::artistinfo::cache::MBID_ROW], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, data) = row?;
            if let Ok(m) = serde_json::from_str::<String>(&data) {
                let m = m.to_ascii_lowercase();
                if is_mbid(&m) {
                    ix.by_mbid.entry(m).or_insert(id);
                }
            }
        }
        Ok(ix)
    }

    /// The library artist a catalogue artist is, if any.
    pub fn artist(&self, mbid: Option<&str>, name: &str) -> Option<String> {
        if let Some(m) = mbid
            && let Some(id) = self.by_mbid.get(&m.to_ascii_lowercase())
        {
            return Some(id.clone());
        }
        let f = fold_artist(name);
        if f.is_empty() { None } else { self.by_fold.get(&f).cloned() }
    }

    #[cfg(test)]
    pub fn with(artists: &[(&str, &str, Option<&str>)]) -> Self {
        let mut ix = Self::default();
        for (id, name, mbid) in artists {
            ix.by_fold.insert(fold_artist(name), id.to_string());
            if let Some(m) = mbid {
                ix.by_mbid.insert(m.to_string(), id.to_string());
            }
        }
        ix
    }
}

/// A library album by a library artist with the same folded title.
pub fn album_of(c: &Connection, artist_id: &str, title: &str) -> AppResult<Option<String>> {
    let want = fold_album(title);
    if want.is_empty() {
        return Ok(None);
    }
    let mut st = c.prepare_cached("SELECT id, title FROM albums WHERE artist_id = ?1")?;
    for row in st.query_map([artist_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, t) = row?;
        if fold_album(&t) == want {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Whether the library has a song by that artist with that title (any
/// version).
pub fn has_song(c: &Connection, artist: &str, title: &str) -> AppResult<bool> {
    let (a, t) = (fold_artist(artist), fold_title(title).title);
    if a.is_empty() || t.is_empty() {
        return Ok(false);
    }
    Ok(c.query_row("SELECT 1 FROM song_index WHERE artist_fold = ?1 AND title_fold = ?2 LIMIT 1", params![a, t], |_| Ok(()))
        .optional()?
        .is_some())
}
