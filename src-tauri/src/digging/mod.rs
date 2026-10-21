//! Crates: the library browsed like record crates, by label, producer,
//! studio, era, year and genre.
//!
//! Everything comes from what the library already keeps: the album's label
//! (its sleeve notes' label, else the files' `LABEL` tags, else Plex's), the
//! producers credited in the files' tags and in sleeve notes, the studios
//! sleeve notes found, the album year and Plex's genres. Nothing goes to the
//! network; online facts count only once a sleeve notes lookup cached them.
//!
//! A crate holds at least [`MIN_ALBUMS`] albums; a crate of one is no crate.
//! Names are folded so spellings of one label, producer or studio share a
//! crate ("Low Tide Records" and "Low Tide"), and each crate shows the
//! spelling used most.
//!
//! - [`members`]: who belongs where, read from the library cache
//! - [`group`]: the pure grouping into crates
//! - [`commands`]: Tauri commands (docs/IPC.md "Crates")

pub mod commands;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::artistinfo::fold_name;
use crate::error::AppResult;

/// The fewest albums a crate holds.
pub const MIN_ALBUMS: usize = 2;
/// Sleeves sent with each crate on the Crates page.
pub const PREVIEW: usize = 12;
/// Albums on an album page's crate shelf.
pub const SHELF_N: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrateKind {
    Label,
    Producer,
    Studio,
    Era,
    Year,
    Genre,
}

impl CrateKind {
    pub const ALL: [CrateKind; 6] = [CrateKind::Label, CrateKind::Producer, CrateKind::Studio, CrateKind::Era, CrateKind::Year, CrateKind::Genre];

    pub fn key(self) -> &'static str {
        match self {
            CrateKind::Label => "label",
            CrateKind::Producer => "producer",
            CrateKind::Studio => "studio",
            CrateKind::Era => "era",
            CrateKind::Year => "year",
            CrateKind::Genre => "genre",
        }
    }
}

// ---------------------------------------------------------------- folding

/// Words a label's name may end with that don't make it another label.
const LABEL_TAIL: &[&str] = &["records", "recordings", "record", "music", "musique", "entertainment", "inc", "ltd", "llc", "limited", "co", "label", "group"];
/// The same for a studio.
const STUDIO_TAIL: &[&str] = &["studios", "studio", "recording", "recordings", "inc", "ltd", "llc"];

fn fold_with_tail(name: &str, tail: &[&str]) -> String {
    let folded = fold_name(name);
    let mut words: Vec<&str> = folded
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.first() == Some(&"the") && words.len() > 1 {
        words.remove(0);
    }
    while words.len() > 1 && words.last().is_some_and(|w| tail.contains(w)) {
        words.pop();
    }
    words.join(" ")
}

/// A label's folded name: case, accents' forms, punctuation, a leading
/// "The" and endings like "Records" or "Ltd." don't count.
pub fn label_key(name: &str) -> String {
    fold_with_tail(name, LABEL_TAIL)
}

/// A studio's folded name ("Studios", "Recording" endings don't count).
pub fn studio_key(name: &str) -> String {
    fold_with_tail(name, STUDIO_TAIL)
}

/// A genre's folded name: case, spaces, hyphens and dots don't count, so
/// "Hip-Hop", "Hip Hop" and "hiphop" are one genre.
pub fn genre_key(name: &str) -> String {
    fold_name(name).chars().filter(|ch| !ch.is_whitespace() && !matches!(ch, '-' | '\u{2010}' | '\u{2011}' | '_' | '.')).collect()
}

/// The labels in one label field: "A / B" and "A; B" name two.
pub fn split_labels(s: &str) -> Vec<String> {
    s.split(['/', ';', '|'])
        .map(str::trim)
        .filter(|x| !x.is_empty() && !matches!(x.to_lowercase().as_str(), "[no label]" | "no label" | "not on label" | "self-released" | "none"))
        .map(str::to_owned)
        .collect()
}

/// The decade a year falls in (1994 -> 1990). None for no year.
pub fn decade(year: Option<i32>) -> Option<i32> {
    year.filter(|y| *y > 0).map(|y| y - y.rem_euclid(10))
}

/// The era crate's name: "1990s".
pub fn era_name(decade: i32) -> String {
    format!("{decade}s")
}

// ---------------------------------------------------------------- grouping

/// One album in one crate, as read from the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub kind: CrateKind,
    pub key: String,
    /// The name as this source spells it.
    pub name: String,
    pub album_id: String,
}

impl Member {
    pub fn new(kind: CrateKind, key: String, name: &str, album_id: &str) -> Self {
        Self { kind, key, name: name.trim().to_owned(), album_id: album_id.to_owned() }
    }
}

/// How albums sort inside a crate: by year (none last, as `i32::MAX`), then artist and title.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Order {
    pub year: i32,
    pub artist: String,
    pub title: String,
}

/// A crate: its albums in [`Order`].
#[derive(Debug, Clone, PartialEq)]
pub struct CrateEntry {
    pub kind: CrateKind,
    pub key: String,
    pub name: String,
    pub album_ids: Vec<String>,
    /// First and last year among its albums.
    pub years: Option<(i32, i32)>,
    /// Distinct album artists.
    pub artists: usize,
}

/// Whether a name is written all in capitals ("LOW TIDE").
fn shouting(s: &str) -> bool {
    s.chars().any(char::is_alphabetic) && !s.chars().any(char::is_lowercase)
}

/// The spelling used most. On a tie: not all capitals, then the shortest
/// ("Low Tide" over "Low Tide Records"), then the first alphabetically.
fn best_spelling(counts: &BTreeMap<&str, usize>) -> Option<String> {
    counts
        .iter()
        .min_by(|(a, x), (b, y)| y.cmp(x).then_with(|| shouting(a).cmp(&shouting(b))).then_with(|| a.chars().count().cmp(&b.chars().count())).then_with(|| a.cmp(b)))
        .map(|(s, _)| (*s).to_owned())
}

/// Groups members into crates: one per (kind, folded key), each album once,
/// named by the spelling used most ([`best_spelling`]).
/// Crates under [`MIN_ALBUMS`] and albums missing from `order` are left out.
pub fn group(members: &[Member], order: &HashMap<String, Order>) -> Vec<CrateEntry> {
    struct Acc<'a> {
        names: BTreeMap<&'a str, usize>,
        albums: HashSet<&'a str>,
    }
    let mut accs: BTreeMap<(CrateKind, &str), Acc> = BTreeMap::new();
    for m in members {
        if m.key.is_empty() || !order.contains_key(&m.album_id) {
            continue;
        }
        let a = accs.entry((m.kind, m.key.as_str())).or_insert_with(|| Acc { names: BTreeMap::new(), albums: HashSet::new() });
        if !m.name.is_empty() {
            *a.names.entry(m.name.as_str()).or_default() += 1;
        }
        a.albums.insert(m.album_id.as_str());
    }
    let mut out = Vec::new();
    for ((kind, key), acc) in accs {
        if acc.albums.len() < MIN_ALBUMS {
            continue;
        }
        let name = best_spelling(&acc.names).unwrap_or_else(|| key.to_owned());
        let mut ids: Vec<&str> = acc.albums.into_iter().collect();
        ids.sort_by(|a, b| order[*a].cmp(&order[*b]).then_with(|| a.cmp(b)));
        let years: Vec<i32> = ids.iter().map(|id| order[*id].year).filter(|y| *y > 0 && *y < i32::MAX).collect();
        let artists: HashSet<&str> = ids.iter().map(|id| order[*id].artist.as_str()).collect();
        out.push(CrateEntry {
            kind,
            key: key.to_owned(),
            name,
            years: years.iter().min().zip(years.iter().max()).map(|(a, b)| (*a, *b)),
            artists: artists.len(),
            album_ids: ids.into_iter().map(str::to_owned).collect(),
        });
    }
    out
}

/// Every crate, and which crates each album is in.
#[derive(Debug, Clone, Default)]
pub struct Index {
    pub crates: Vec<CrateEntry>,
    /// Album id -> indexes into `crates`.
    pub by_album: HashMap<String, Vec<usize>>,
}

impl Index {
    pub fn new(crates: Vec<CrateEntry>) -> Self {
        let mut by_album: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, c) in crates.iter().enumerate() {
            for id in &c.album_ids {
                by_album.entry(id.clone()).or_default().push(i);
            }
        }
        Self { crates, by_album }
    }

    pub fn of_kind(&self, kind: CrateKind) -> impl Iterator<Item = &CrateEntry> {
        self.crates.iter().filter(move |c| c.kind == kind)
    }

    pub fn find(&self, kind: CrateKind, key: &str) -> Option<&CrateEntry> {
        self.crates.iter().find(|c| c.kind == kind && c.key == key)
    }

    /// The label, producer and studio crates an album is in, in that order
    /// (biggest first within a kind).
    pub fn digging_from(&self, album_id: &str) -> Vec<&CrateEntry> {
        let mut out: Vec<&CrateEntry> = self
            .by_album
            .get(album_id)
            .map(|ix| ix.iter().map(|i| &self.crates[*i]).filter(|c| matches!(c.kind, CrateKind::Label | CrateKind::Producer | CrateKind::Studio)).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| b.album_ids.len().cmp(&a.album_ids.len())).then_with(|| a.name.cmp(&b.name)));
        out
    }
}

// ---------------------------------------------------------------- reading the library

/// The label most of an album's files name (folded), in the spelling they
/// use most.
fn most_common(values: &[String]) -> Option<String> {
    let mut groups: Vec<(String, BTreeMap<&str, usize>, usize)> = Vec::new();
    for v in values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        let k = label_key(v);
        let i = match groups.iter().position(|(x, _, _)| *x == k) {
            Some(i) => i,
            None => {
                groups.push((k, BTreeMap::new(), 0));
                groups.len() - 1
            }
        };
        *groups[i].1.entry(v).or_default() += 1;
        groups[i].2 += 1;
    }
    let best = groups.iter().map(|(_, _, n)| *n).max()?;
    groups.into_iter().find(|(_, _, n)| *n == best).and_then(|(_, spellings, _)| best_spelling(&spellings))
}

/// Every album's crate memberships, and how albums sort, for the albums
/// `shows` keeps. A handful of whole-table reads, each on an index or a
/// small table.
pub fn members(c: &Connection, shows: &dyn Fn(&str) -> bool) -> AppResult<(Vec<Member>, HashMap<String, Order>)> {
    let mut order: HashMap<String, Order> = HashMap::new();
    let mut out: Vec<Member> = Vec::new();
    // Plex's label, kept to fall back on.
    let mut plex_label: HashMap<String, String> = HashMap::new();
    {
        let mut st = c.prepare("SELECT id, year, sort_title, artist_title, studio, genres FROM albums")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i32>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        for row in rows {
            let (id, year, sort, artist, studio, genres) = row?;
            if !shows(&id) {
                continue;
            }
            if let Some(d) = decade(year) {
                out.push(Member::new(CrateKind::Era, d.to_string(), &era_name(d), &id));
                let y = year.unwrap_or_default();
                out.push(Member::new(CrateKind::Year, y.to_string(), &y.to_string(), &id));
            }
            let genres: Vec<String> = serde_json::from_str(&genres).unwrap_or_default();
            for g in genres {
                out.push(Member::new(CrateKind::Genre, genre_key(&g), &g, &id));
            }
            if let Some(s) = studio.filter(|s| !s.trim().is_empty()) {
                plex_label.insert(id.clone(), s);
            }
            order.insert(id, Order { year: year.filter(|y| *y > 0).unwrap_or(i32::MAX), artist: artist.to_lowercase(), title: sort.to_lowercase() });
        }
    }

    // Labels: the sleeve notes' label, else the files' tags, else Plex's.
    let mut noted: HashMap<String, Vec<String>> = HashMap::new();
    let mut studios: Vec<(String, String)> = Vec::new();
    {
        let mut st = c.prepare_cached("SELECT album_id, kind, name FROM album_facts")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (album, kind, name) = row?;
            match kind.as_str() {
                "label" => noted.entry(album).or_default().push(name),
                "studio" => studios.push((album, name)),
                _ => {}
            }
        }
    }
    let mut tagged: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT t.album_id, json_extract(f.sleeve, '$.label') FROM file_tags f JOIN tracks t ON t.part_key = f.part_key
             WHERE f.status = 'ok' AND f.sleeve IS NOT NULL AND json_extract(f.sleeve, '$.label') IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (album, label) = row?;
            tagged.entry(album).or_default().push(label);
        }
    }
    for id in order.keys() {
        let labels: Vec<String> = if let Some(n) = noted.get(id) {
            n.clone()
        } else if let Some(l) = tagged.get(id).and_then(|v| most_common(v)) {
            split_labels(&l)
        } else if let Some(l) = plex_label.get(id) {
            split_labels(l)
        } else {
            vec![]
        };
        for l in labels {
            out.push(Member::new(CrateKind::Label, label_key(&l), &l, id));
        }
    }
    for (album, name) in studios {
        out.push(Member::new(CrateKind::Studio, studio_key(&name), &name, &album));
    }

    // Producers: the files' credit tags and what sleeve notes credited.
    {
        let mut st = c.prepare_cached(
            "SELECT tc.person_key, tc.name, t.album_id FROM tag_credits tc JOIN tracks t ON t.part_key = tc.part_key
             WHERE tc.role = 'producer'",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (key, name, album) = row?;
            out.push(Member::new(CrateKind::Producer, key, &name, &album));
        }
        let mut st = c.prepare_cached("SELECT person_key, name, album_id FROM sleeve_credits WHERE role = 'producer'")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (key, name, album) = row?;
            out.push(Member::new(CrateKind::Producer, key, &name, &album));
        }
    }
    Ok((out, order))
}

/// Every crate in the library, for the albums `shows` keeps.
pub fn build(c: &Connection, shows: &dyn Fn(&str) -> bool) -> AppResult<Index> {
    let (members, order) = members(c, shows)?;
    Ok(Index::new(group(&members, &order)))
}

/// Changes whenever anything crates read changes.
pub type Stamp = (i64, i64, i64, i64, i64, i64, i64);

pub fn stamp(c: &Connection) -> AppResult<Stamp> {
    Ok(c.query_row(
        "SELECT (SELECT count(*) FROM albums), (SELECT coalesce(max(pk), 0) FROM albums),
                (SELECT coalesce(sum(length(coalesce(studio, '')) * 31 + length(genres) + coalesce(year, 0)), 0) FROM albums),
                (SELECT count(*) FROM album_facts), (SELECT count(*) FROM tag_credits), (SELECT count(*) FROM sleeve_credits),
                (SELECT count(*) FROM file_tags WHERE sleeve IS NOT NULL)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
    )?)
}

/// Whether any of an album's songs has a place on the sound map.
pub fn on_map(c: &Connection, album_id: &str) -> AppResult<bool> {
    Ok(c
        .prepare_cached("SELECT 1 FROM tracks t JOIN sound_map sm ON sm.track_id = t.id WHERE t.album_id = ?1 LIMIT 1")?
        .exists([album_id])?)
}
