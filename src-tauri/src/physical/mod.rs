//! The physical collection: albums the user owns on vinyl, CD, cassette or
//! another format, with notes on each copy (pressing, colour, condition).
//!
//! Marks live in the sync store's `physical` namespace, keyed by album id,
//! so they follow the user through the sync file. A Discogs collection can
//! be brought in with the user's personal token ([`discogs`]): its items are
//! matched to library albums ([`Matcher`]) and become marks; the ones the
//! library doesn't have are listed as "On the shelf, not in your library".
//!
//! - [`commands`]: the service and its Tauri commands (docs/IPC.md
//!   "Physical collection").
//! - [`discogs`]: the Discogs client (paced to 60 requests a minute).

pub mod commands;
pub mod discogs;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_album, fold_artist, is_generic};

/// Compilation titles too common to match on without an artist.
const COMMON_TITLES: &[&str] = &["greatest hits", "best of", "the best of", "hits", "the hits", "soundtrack", "original soundtrack", "compilation", "mixtape", "live", "singles"];
use crate::import::store::{StoreLinks, album_store_links};
use crate::sync::SyncStore;
use crate::sync::store::{Op, PHYSICAL};

/// Most copies one album can have.
pub const MAX_COPIES: usize = 20;
/// Longest note field kept.
pub const NOTE_MAX: usize = 300;
/// Albums on the Stats page's "most played" list.
pub const MOST_PLAYED: usize = 10;
/// Albums on "Missing from the shelf".
pub const MISSING_MAX: usize = 25;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Vinyl,
    Cd,
    Cassette,
    #[default]
    Other,
}

pub const FORMATS: [Format; 4] = [Format::Vinyl, Format::Cd, Format::Cassette, Format::Other];

/// Which Discogs collection item a copy came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscogsRef {
    pub release_id: u64,
    /// The item in the user's collection (the same release can be there twice).
    pub instance_id: u64,
}

/// One copy the user owns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnedCopy {
    pub format: Format,
    /// "LP, Album, Reissue", "2018 Japanese pressing".
    pub pressing: String,
    pub colour: String,
    pub condition: String,
    pub notes: String,
    /// Set when it came from Discogs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discogs: Option<DiscogsRef>,
}

/// What the sync store keeps for an album (keyed by album id).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Owned {
    pub copies: Vec<OwnedCopy>,
    /// Kept so the mark reads even when the album isn't in the library.
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    /// When it was first marked (unix seconds).
    pub at: i64,
}

impl Owned {
    /// Formats it's owned on, each once, in [`FORMATS`] order.
    pub fn formats(&self) -> Vec<Format> {
        let have: HashSet<Format> = self.copies.iter().map(|c| c.format).collect();
        FORMATS.into_iter().filter(|f| have.contains(f)).collect()
    }
}

/// An owned album as the UI gets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedAlbum {
    pub album_id: String,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    pub copies: Vec<OwnedCopy>,
    pub formats: Vec<Format>,
    pub at: i64,
}

impl OwnedAlbum {
    pub fn new(album_id: String, o: Owned) -> Self {
        Self { formats: o.formats(), album_id, title: o.title, artist: o.artist, thumb: o.thumb, copies: o.copies, at: o.at }
    }
}

/// `physical_set` payload: the copies (none takes the mark off), with the
/// album's names for the mark.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SetOwned {
    pub copies: Vec<OwnedCopy>,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
}

fn clip(s: &str) -> String {
    s.trim().chars().take(NOTE_MAX).collect()
}

fn tidy_copy(c: &OwnedCopy) -> OwnedCopy {
    OwnedCopy {
        format: c.format,
        pressing: clip(&c.pressing),
        colour: clip(&c.colour),
        condition: clip(&c.condition),
        notes: clip(&c.notes),
        discogs: c.discogs,
    }
}

// ---------------------------------------------------------------- the marks

pub fn list(store: &SyncStore) -> AppResult<Vec<(String, Owned)>> {
    store.list(&PHYSICAL)
}

pub fn get(store: &SyncStore, album_id: &str) -> AppResult<Option<Owned>> {
    store.get(&PHYSICAL, album_id)
}

/// Mark an album as owned with these copies, or take the mark off when there
/// are none. Keeps when it was first marked.
pub fn set(store: &SyncStore, album_id: &str, s: SetOwned, now: i64) -> AppResult<Option<Owned>> {
    let id = album_id.trim();
    if id.is_empty() || id.len() > 190 {
        return Err(AppError::Invalid("No album to mark.".into()));
    }
    if s.copies.len() > MAX_COPIES {
        return Err(AppError::Invalid(format!("An album can have up to {MAX_COPIES} copies.")));
    }
    if s.copies.is_empty() {
        store.remove(&PHYSICAL, id)?;
        return Ok(None);
    }
    let at = get(store, id)?.map(|o| o.at).filter(|&t| t > 0).unwrap_or(now);
    let o = Owned {
        copies: s.copies.iter().map(tidy_copy).collect(),
        title: clip(&s.title),
        artist: clip(&s.artist),
        thumb: s.thumb.filter(|t| !t.is_empty() && t.len() < 1000),
        at,
    };
    store.put(&PHYSICAL, id, &o)?;
    Ok(Some(o))
}

// ---------------------------------------------------------------- the library

/// A library album, as matching and the stats need it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    pub plays: i64,
    /// Unreleased albums are never matched to a physical copy by name, nor
    /// listed as missing from the shelf.
    pub unreleased: bool,
}

/// Every library album, most played first.
pub fn library(c: &Connection) -> AppResult<Vec<LibAlbum>> {
    use crate::db::release::album_unreleased;
    let mut st = c.prepare_cached(
        "SELECT al.id, al.title, al.artist_title, al.thumb, al.view_count, al.format, al.subformats, al.matched, al.studio, coalesce(at.version, at.release), mc.found
         FROM albums al
         LEFT JOIN album_tags at ON at.album_id = al.id
         LEFT JOIN album_mb_check mc ON mc.album_id = al.id
         ORDER BY al.view_count DESC, al.sort_title COLLATE NOCASE",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            (r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?),
            (
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<bool>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<bool>>(10)?,
            ),
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let ((id, title, artist, thumb, plays), (format, subs, matched, studio, version, found)) = row?;
        let subs: Vec<String> = subs.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let unreleased = album_unreleased(&id, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), found);
        out.push(LibAlbum { id, title, artist, thumb, plays, unreleased });
    }
    Ok(out)
}

/// Discogs adds " (2)" to tell artists with one name apart, and "*" to a
/// name credited differently on the release.
pub fn clean_discogs_artist(name: &str) -> String {
    let mut s = name.trim().trim_end_matches('*').trim().to_string();
    if s.ends_with(')')
        && let Some(i) = s.rfind(" (")
        && s[i + 2..s.len() - 1].chars().all(|c| c.is_ascii_digit())
        && s.len() > i + 3
    {
        s.truncate(i);
    }
    s
}

/// Library albums by Discogs item and by folded artist and title.
#[derive(Debug, Default)]
pub struct Matcher {
    by_release: HashMap<u64, String>,
    by_name: HashMap<(String, String), String>,
}

impl Matcher {
    /// `marks`: albums already marked, whose Discogs copies pin a release to
    /// an album (the user may have matched it by hand).
    pub fn new(library: &[LibAlbum], marks: &[(String, Owned)]) -> Self {
        let present: HashSet<&str> = library.iter().map(|a| a.id.as_str()).collect();
        let mut m = Self::default();
        for (id, o) in marks {
            if !present.contains(id.as_str()) {
                continue;
            }
            for c in &o.copies {
                if let Some(d) = c.discogs {
                    m.by_release.entry(d.release_id).or_insert_with(|| id.clone());
                }
            }
        }
        // Most played first, so the copy played most wins a tie.
        for a in library.iter().filter(|a| !a.unreleased) {
            let key = (fold_artist(&a.artist), fold_album(&a.title));
            if key.1.is_empty() {
                continue;
            }
            m.by_name.entry(key).or_insert_with(|| a.id.clone());
        }
        m
    }

    /// The library album a Discogs item is.
    pub fn album(&self, release_id: u64, artist: &str, title: &str) -> Option<String> {
        if let Some(id) = self.by_release.get(&release_id) {
            return Some(id.clone());
        }
        let a = fold_artist(&clean_discogs_artist(artist));
        let t = fold_album(title);
        // A compilation (no artist) only matches on a title that says something.
        if t.is_empty() || (a.is_empty() && (is_generic(&t) || COMMON_TITLES.contains(&t.as_str()))) {
            return None;
        }
        self.by_name.get(&(a, t)).cloned()
    }

    /// [`Matcher::album`] for a collection item: the whole credit, then its
    /// first artist ("A & B" is often filed under A).
    pub fn item(&self, it: &DiscogsItem) -> Option<String> {
        self.album(it.release_id, &it.artist, &it.title).or_else(|| {
            (!it.first_artist.is_empty() && it.first_artist != it.artist).then(|| self.album(it.release_id, &it.first_artist, &it.title)).flatten()
        })
    }
}

// ---------------------------------------------------------------- importing

/// One item of the user's Discogs collection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscogsItem {
    pub release_id: u64,
    pub instance_id: u64,
    /// Every credited artist, as Discogs joins them.
    pub artist: String,
    /// The first credited artist.
    pub first_artist: String,
    pub title: String,
    pub year: Option<i32>,
    pub thumb: Option<String>,
    pub format: Format,
    pub pressing: String,
    pub colour: String,
    pub condition: String,
    pub notes: String,
}

impl DiscogsItem {
    fn reference(&self) -> DiscogsRef {
        DiscogsRef { release_id: self.release_id, instance_id: self.instance_id }
    }

    fn copy(&self) -> OwnedCopy {
        tidy_copy(&OwnedCopy {
            format: self.format,
            pressing: self.pressing.clone(),
            colour: self.colour.clone(),
            condition: self.condition.clone(),
            notes: self.notes.clone(),
            discogs: Some(self.reference()),
        })
    }
}

/// A Discogs item the library doesn't have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfOnly {
    pub release_id: u64,
    pub artist: String,
    pub title: String,
    pub year: Option<i32>,
    pub format: Format,
    pub thumb: Option<String>,
    #[serde(skip_deserializing, default = "no_links")]
    pub links: StoreLinks,
}

fn no_links() -> StoreLinks {
    album_store_links("", "")
}

/// What the last Discogs import found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscogsReport {
    pub username: String,
    /// Unix seconds.
    pub imported_at: i64,
    /// Items in the collection.
    pub total: usize,
    /// Items matched to a library album.
    pub matched: usize,
    /// Albums marked owned for the first time.
    pub added: usize,
    /// Items the library doesn't have, by artist then title.
    pub unmatched: Vec<ShelfOnly>,
}

impl DiscogsReport {
    /// Store links are worked out when read, not kept.
    pub fn with_links(mut self) -> Self {
        for u in &mut self.unmatched {
            u.links = album_store_links(&u.artist, &u.title);
        }
        self
    }
}

/// The writes an import makes and its report.
pub struct ImportPlan {
    pub ops: Vec<(String, Option<Owned>)>,
    pub report: DiscogsReport,
}

/// Work out an import: matched items become copies (a copy already there
/// for the same collection item keeps the user's edits), copies of items
/// no longer in the collection go, copies added by hand stay.
pub fn plan_import(library: &[LibAlbum], marks: &[(String, Owned)], items: &[DiscogsItem], username: &str, now: i64) -> ImportPlan {
    let matcher = Matcher::new(library, marks);
    let lib: HashMap<&str, &LibAlbum> = library.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut groups: HashMap<String, Vec<&DiscogsItem>> = HashMap::new();
    let mut unmatched = Vec::new();
    let mut matched = 0;
    for it in items {
        match matcher.item(it) {
            Some(id) => {
                matched += 1;
                groups.entry(id).or_default().push(it);
            }
            None => unmatched.push(ShelfOnly {
                release_id: it.release_id,
                artist: clean_discogs_artist(&it.artist),
                title: it.title.trim().to_string(),
                year: it.year,
                format: it.format,
                thumb: it.thumb.clone(),
                links: no_links(),
            }),
        }
    }
    let existing: HashMap<&str, &Owned> = marks.iter().map(|(k, o)| (k.as_str(), o)).collect();
    // Marks on albums the library doesn't list right now (another server
    // that's offline) are left as they are.
    let mut ids: Vec<String> = marks
        .iter()
        .filter(|(k, o)| lib.contains_key(k.as_str()) && o.copies.iter().any(|c| c.discogs.is_some()))
        .map(|(k, _)| k.clone())
        .collect();
    ids.extend(groups.keys().cloned());
    ids.sort();
    ids.dedup();
    let mut ops = Vec::new();
    let mut added = 0;
    for id in ids {
        let old = existing.get(id.as_str()).copied();
        let from = groups.get(&id).map(Vec::as_slice).unwrap_or(&[]);
        let mut copies: Vec<OwnedCopy> = old.map(|o| o.copies.iter().filter(|c| c.discogs.is_none()).cloned().collect()).unwrap_or_default();
        for it in from {
            let kept = old.and_then(|o| o.copies.iter().find(|c| c.discogs.is_some_and(|d| d.instance_id == it.instance_id)));
            copies.push(kept.cloned().unwrap_or_else(|| it.copy()));
        }
        copies.truncate(MAX_COPIES);
        if old.is_some_and(|o| o.copies == copies) {
            continue;
        }
        if copies.is_empty() {
            ops.push((id, None));
            continue;
        }
        if old.is_none() {
            added += 1;
        }
        let la = lib.get(id.as_str());
        let o = Owned {
            copies,
            title: old.map(|o| o.title.clone()).filter(|t| !t.is_empty()).or_else(|| la.map(|a| a.title.clone())).unwrap_or_default(),
            artist: old.map(|o| o.artist.clone()).filter(|t| !t.is_empty()).or_else(|| la.map(|a| a.artist.clone())).unwrap_or_default(),
            thumb: old.and_then(|o| o.thumb.clone()).or_else(|| la.and_then(|a| a.thumb.clone())),
            at: old.map(|o| o.at).unwrap_or(now),
        };
        ops.push((id, Some(o)));
    }
    unmatched.sort_by(|a, b| {
        let k = |s: &ShelfOnly| (s.artist.to_lowercase(), s.title.to_lowercase(), s.release_id);
        k(a).cmp(&k(b))
    });
    unmatched.dedup_by_key(|u| u.release_id);
    ImportPlan {
        ops,
        report: DiscogsReport { username: username.to_string(), imported_at: now, total: items.len(), matched, added, unmatched },
    }
}

/// Write an import's marks in one go.
pub fn apply_plan(store: &SyncStore, plan: &ImportPlan) -> AppResult<()> {
    let mut ops = Vec::new();
    for (id, o) in &plan.ops {
        ops.push(match o {
            Some(o) => Op::put(&PHYSICAL, id, o)?,
            None => Op::remove(&PHYSICAL, id.clone()),
        });
    }
    if ops.is_empty() {
        return Ok(());
    }
    store.write_many(ops)
}

// ---------------------------------------------------------------- stats

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCount {
    pub format: Format,
    /// Albums owned on it.
    pub albums: usize,
    pub copies: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfAlbum {
    pub album_id: String,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    pub plays: i64,
    pub formats: Vec<Format>,
}

/// The shelf on the Stats page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfStats {
    /// Owned albums that are in the library.
    pub albums: usize,
    pub copies: usize,
    /// Formats with at least one copy, in [`FORMATS`] order.
    pub by_format: Vec<FormatCount>,
    /// Owned albums played most.
    pub most_played: Vec<ShelfAlbum>,
    /// Albums played most that aren't owned; None while the setting is off.
    pub missing: Option<Vec<ShelfAlbum>>,
}

pub fn stats(library: &[LibAlbum], marks: &[(String, Owned)], missing_on: bool) -> ShelfStats {
    let owned: HashMap<&str, &Owned> = marks.iter().map(|(k, o)| (k.as_str(), o)).collect();
    let mut by: HashMap<Format, (usize, usize)> = HashMap::new();
    let mut albums = 0;
    let mut copies = 0;
    let mut most = Vec::new();
    for a in library {
        let Some(o) = owned.get(a.id.as_str()) else { continue };
        albums += 1;
        copies += o.copies.len();
        for f in o.formats() {
            by.entry(f).or_default().0 += 1;
        }
        for c in &o.copies {
            by.entry(c.format).or_default().1 += 1;
        }
        if most.len() < MOST_PLAYED && a.plays > 0 {
            most.push(shelf_album(a, o.formats()));
        }
    }
    let missing = missing_on.then(|| {
        library
            .iter()
            .filter(|a| a.plays > 0 && !a.unreleased && !owned.contains_key(a.id.as_str()))
            .take(MISSING_MAX)
            .map(|a| shelf_album(a, Vec::new()))
            .collect()
    });
    ShelfStats {
        albums,
        copies,
        by_format: FORMATS
            .into_iter()
            .filter_map(|f| by.get(&f).map(|&(albums, copies)| FormatCount { format: f, albums, copies }))
            .collect(),
        most_played: most,
        missing,
    }
}

fn shelf_album(a: &LibAlbum, formats: Vec<Format>) -> ShelfAlbum {
    ShelfAlbum { album_id: a.id.clone(), title: a.title.clone(), artist: a.artist.clone(), thumb: a.thumb.clone(), plays: a.plays, formats }
}
