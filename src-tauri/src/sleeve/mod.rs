//! Sleeve notes: an album's liner notes, gathered from every source Platter
//! can read and shown with where each fact came from.
//!
//! Sources, in the order they win when they disagree:
//! - the files' own tags (credits, label, catalogue number, barcode, country,
//!   original date, and the archive's date, source and edition),
//! - Plex (release date, label, and the per-song credits Plex keeps),
//! - MusicBrainz (the release and its recordings' relations),
//! - Discogs (the release's credits, label, notes), only with a token.
//!
//! Agreeing sources add up on one fact; a person's roles keep every source
//! that named them. Online lookups run lazily when the tab opens and are
//! cached on disk, so offline the notes show what was cached.
//!
//! - [`merge`]: the pure merge of what each source says
//! - [`gather`]: reading each source's part from the library cache
//! - [`discogs`]: reading a Discogs release
//! - [`store`]: the credit index behind credit pages
//! - [`service`]: lazy lookups, caching and events
//! - [`commands`]: Tauri commands (docs/IPC.md "Sleeve notes")

pub mod commands;
pub mod discogs;
pub mod gather;
pub mod service;
pub mod store;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::artistinfo::fold_name;

/// Where a fact came from. The order is the precedence: earlier wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Tags,
    Plex,
    Musicbrainz,
    Discogs,
}

impl Source {
    pub fn key(self) -> &'static str {
        match self {
            Source::Tags => "tags",
            Source::Plex => "plex",
            Source::Musicbrainz => "musicbrainz",
            Source::Discogs => "discogs",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        [Source::Tags, Source::Plex, Source::Musicbrainz, Source::Discogs].into_iter().find(|s| s.key() == k)
    }
}

/// Credit roles in the order the notes list them.
pub const ROLES: [&str; 11] =
    ["producer", "writer", "composer", "lyricist", "arranger", "performer", "featured", "conductor", "engineer", "mixer", "mastering"];

fn role_rank(role: &str) -> usize {
    ROLES.iter().position(|r| *r == role).unwrap_or(ROLES.len())
}

/// One fact and the sources that agree on it, the winning source first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub value: String,
    pub sources: Vec<Source>,
}

/// One credit as a source gives it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawCredit {
    pub name: String,
    pub role: String,
    /// The instrument for a performer.
    pub detail: Option<String>,
}

impl RawCredit {
    pub fn new(name: &str, role: &str, detail: Option<&str>) -> Self {
        Self { name: name.trim().to_owned(), role: role.to_owned(), detail: detail.map(|d| d.trim().to_lowercase()).filter(|d| !d.is_empty()) }
    }
}

/// What one source says about a song.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackPart {
    pub credits: Vec<RawCredit>,
    pub recorded: Option<String>,
}

/// What one source says about an album: the input to [`merge`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    /// The first release date.
    pub released: Option<String>,
    /// This edition's date, when it isn't the first release.
    pub edition_date: Option<String>,
    pub recorded: Option<String>,
    pub label: Option<String>,
    pub catalogue: Option<String>,
    pub country: Option<String>,
    pub barcode: Option<String>,
    pub format: Option<String>,
    pub edition: Option<String>,
    /// Where an archive file came from (ARCHIVE_SOURCE).
    pub origin: Option<String>,
    pub notes: Option<String>,
    /// (what was done there: "recorded", "mixed", "mastered"; studio name).
    pub studios: Vec<(String, String)>,
    /// Credits on the album itself.
    pub credits: Vec<RawCredit>,
    /// By library track id.
    pub tracks: HashMap<String, TrackPart>,
    /// The release page at the source.
    pub link: Option<String>,
}

/// One role a person has, with the sources that name it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleCredit {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub sources: Vec<Source>,
}

/// A person in the notes. `key` opens their credit page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub key: String,
    /// A library artist with this name.
    pub artist_id: Option<String>,
    pub roles: Vec<RoleCredit>,
}

/// Everyone credited with one role, for a "Produced by" line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleGroup {
    pub role: String,
    pub people: Vec<GroupPerson>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPerson {
    pub name: String,
    pub key: String,
    pub artist_id: Option<String>,
    /// Instruments, for performers.
    pub details: Vec<String>,
    pub sources: Vec<Source>,
}

/// A studio and what was done there.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Studio {
    pub name: String,
    pub role: String,
    pub sources: Vec<Source>,
}

/// One song's credits.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleeveTrack {
    pub track_id: String,
    pub title: String,
    pub disc: u32,
    pub number: u32,
    pub groups: Vec<RoleGroup>,
    pub recorded: Option<Fact>,
}

/// A link to the release at a source.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub source: Source,
    pub url: String,
}

/// Which lookups are on, so an empty page can say how to get more.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lookups {
    /// "Look up credits online" is on and the app isn't offline.
    pub online: bool,
    pub musicbrainz: bool,
    /// A Discogs token is saved.
    pub discogs: bool,
    /// The album is unreleased: never looked up online.
    pub unreleased: bool,
}

/// IPC `SleeveNotes`: the album page's Sleeve notes tab.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleeveNotes {
    pub album_id: String,
    /// Online lookups are running; `sleeve://changed` follows.
    pub looking: bool,
    pub lookups: Lookups,
    pub released: Option<Fact>,
    pub edition_date: Option<Fact>,
    pub recorded: Option<Fact>,
    pub label: Option<Fact>,
    pub catalogue: Option<Fact>,
    pub country: Option<Fact>,
    pub barcode: Option<Fact>,
    pub format: Option<Fact>,
    pub edition: Option<Fact>,
    /// The files' release status ("released", "unreleased", "leaked", "alternate").
    pub status: Option<Fact>,
    pub origin: Option<Fact>,
    /// The release's own notes.
    pub notes: Option<Fact>,
    pub studios: Vec<Studio>,
    /// How this edition differs from the library's other editions.
    pub edition_notes: Vec<Fact>,
    /// Everyone credited on the album, by role (featured artists stay with their songs).
    pub groups: Vec<RoleGroup>,
    pub tracks: Vec<SleeveTrack>,
    pub links: Vec<Link>,
}

impl SleeveNotes {
    /// Nothing at all is known.
    pub fn is_empty(&self) -> bool {
        [&self.released, &self.edition_date, &self.recorded, &self.label, &self.catalogue, &self.country, &self.barcode, &self.format, &self.edition, &self.origin, &self.notes]
            .iter()
            .all(|f| f.is_none())
            && self.studios.is_empty()
            && self.groups.is_empty()
            && self.tracks.iter().all(|t| t.groups.is_empty() && t.recorded.is_none())
    }
}

/// A library song, for the notes' song list.
#[derive(Debug, Clone, PartialEq)]
pub struct LibSong {
    pub id: String,
    pub title: String,
    pub disc: u32,
    pub number: u32,
}

// ---------------------------------------------------------------- merging

/// Folded value for comparing facts.
fn fact_key(v: &str) -> String {
    fold_name(v).replace(['-', ' ', '.', '/'], "")
}

/// Whether two dates agree, one being a less precise form of the other
/// ("1997" and "1997-05-21").
fn same_date(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a.starts_with(b) || b.starts_with(a)
}

/// Merges one fact from each source in precedence order. Dates that agree
/// keep the most precise form.
fn merge_fact(values: &[(Source, Option<&str>)], dates: bool) -> Option<Fact> {
    let mut out: Option<Fact> = None;
    let mut sorted: Vec<(Source, &str)> =
        values.iter().filter_map(|(s, v)| v.map(str::trim).filter(|v| !v.is_empty()).map(|v| (*s, v))).collect();
    sorted.sort_by_key(|(s, _)| *s);
    for (src, v) in sorted {
        match &mut out {
            None => out = Some(Fact { value: v.to_owned(), sources: vec![src] }),
            Some(f) => {
                let agree = if dates { same_date(&f.value, v) } else { fact_key(&f.value) == fact_key(v) };
                if agree {
                    if dates && v.len() > f.value.len() {
                        f.value = v.to_owned();
                    }
                    if !f.sources.contains(&src) {
                        f.sources.push(src);
                    }
                }
            }
        }
    }
    out
}

/// Merges credits from each source: one person per folded name, one role
/// per (role, instrument), each with the sources that name it.
pub fn merge_credits(parts: &[(Source, &[RawCredit])], library: &HashMap<String, String>) -> Vec<Person> {
    let mut sorted: Vec<&(Source, &[RawCredit])> = parts.iter().collect();
    sorted.sort_by_key(|(s, _)| *s);
    let mut people: Vec<Person> = Vec::new();
    for (src, credits) in sorted {
        for c in credits.iter() {
            let key = fold_name(&c.name);
            if key.is_empty() || c.role.is_empty() {
                continue;
            }
            let i = match people.iter().position(|p| p.key == key) {
                Some(i) => i,
                None => {
                    people.push(Person { name: c.name.clone(), artist_id: library.get(&key).cloned(), key, roles: vec![] });
                    people.len() - 1
                }
            };
            let p = &mut people[i];
            match p.roles.iter_mut().find(|r| r.role == c.role && r.detail == c.detail) {
                Some(r) if !r.sources.contains(src) => r.sources.push(*src),
                Some(_) => {}
                None => p.roles.push(RoleCredit { role: c.role.clone(), detail: c.detail.clone(), sources: vec![*src] }),
            }
        }
    }
    for p in &mut people {
        p.roles.sort_by_key(|r| role_rank(&r.role));
    }
    // By each person's first role; the sources' order otherwise.
    people.sort_by_key(|p| p.roles.first().map(|r| role_rank(&r.role)).unwrap_or(ROLES.len()));
    people
}

/// People grouped by role in [`ROLES`] order, each person once per role
/// with their instruments and the sources that name them in it.
pub fn group_by_role(people: &[Person]) -> Vec<RoleGroup> {
    let mut groups: Vec<RoleGroup> = Vec::new();
    for role in ROLES.iter().copied().chain(people.iter().flat_map(|p| p.roles.iter().map(|r| r.role.as_str())).filter(|r| !ROLES.contains(r))) {
        if groups.iter().any(|g| g.role == role) {
            continue;
        }
        let mut members: Vec<GroupPerson> = Vec::new();
        for p in people {
            let rs: Vec<&RoleCredit> = p.roles.iter().filter(|r| r.role == role).collect();
            if rs.is_empty() {
                continue;
            }
            let mut sources: Vec<Source> = rs.iter().flat_map(|r| r.sources.iter().copied()).collect();
            sources.sort();
            sources.dedup();
            let mut details: Vec<String> = Vec::new();
            for d in rs.iter().filter_map(|r| r.detail.clone()) {
                if !details.contains(&d) {
                    details.push(d);
                }
            }
            members.push(GroupPerson { name: p.name.clone(), key: p.key.clone(), artist_id: p.artist_id.clone(), details, sources });
        }
        if !members.is_empty() {
            groups.push(RoleGroup { role: role.to_owned(), people: members });
        }
    }
    groups
}

fn studios(parts: &[(Source, &Part)]) -> Vec<Studio> {
    let mut out: Vec<Studio> = Vec::new();
    let mut sorted: Vec<&(Source, &Part)> = parts.iter().collect();
    sorted.sort_by_key(|(s, _)| *s);
    for (src, p) in sorted {
        for (role, name) in &p.studios {
            match out.iter_mut().find(|s| s.role == *role && fact_key(&s.name) == fact_key(name)) {
                Some(s) if !s.sources.contains(src) => s.sources.push(*src),
                Some(_) => {}
                None => out.push(Studio { name: name.clone(), role: role.clone(), sources: vec![*src] }),
            }
        }
    }
    let rank = |r: &str| ["recorded", "mixed", "mastered"].iter().position(|x| *x == r).unwrap_or(3);
    out.sort_by_key(|s| rank(&s.role));
    out
}

/// Merges what each source says into the notes. `library` maps folded
/// names to library artist ids. Lookup state is the caller's to fill.
pub fn merge(album_id: &str, parts: &[(Source, Part)], songs: &[LibSong], library: &HashMap<String, String>) -> SleeveNotes {
    let refs: Vec<(Source, &Part)> = parts.iter().map(|(s, p)| (*s, p)).collect();
    let fact = |get: fn(&Part) -> Option<&str>, dates: bool| {
        let values: Vec<(Source, Option<&str>)> = refs.iter().map(|(s, p)| (*s, get(p))).collect();
        merge_fact(&values, dates)
    };
    // The album's people: its own credits and every song's, but featured
    // artists stay with their songs.
    let album_credits: Vec<Vec<RawCredit>> = refs
        .iter()
        .map(|(_, p)| {
            let mut all = p.credits.clone();
            let mut ids: Vec<&String> = p.tracks.keys().collect();
            ids.sort();
            for id in ids {
                all.extend(p.tracks[id].credits.iter().filter(|c| c.role != "featured").cloned());
            }
            all
        })
        .collect();
    let album_parts: Vec<(Source, &[RawCredit])> = refs.iter().zip(&album_credits).map(|((s, _), c)| (*s, c.as_slice())).collect();
    let people = merge_credits(&album_parts, library);
    let tracks = songs
        .iter()
        .map(|song| {
            let per: Vec<(Source, &[RawCredit])> =
                refs.iter().filter_map(|(s, p)| p.tracks.get(&song.id).map(|t| (*s, t.credits.as_slice()))).collect();
            let rec: Vec<(Source, Option<&str>)> =
                refs.iter().map(|(s, p)| (*s, p.tracks.get(&song.id).and_then(|t| t.recorded.as_deref()))).collect();
            SleeveTrack {
                track_id: song.id.clone(),
                title: song.title.clone(),
                disc: song.disc,
                number: song.number,
                groups: group_by_role(&merge_credits(&per, library)),
                recorded: merge_fact(&rec, true),
            }
        })
        .collect();
    let mut links: Vec<Link> = refs.iter().filter_map(|(s, p)| p.link.clone().map(|url| Link { source: *s, url })).collect();
    links.sort_by_key(|l| l.source);
    SleeveNotes {
        album_id: album_id.to_owned(),
        looking: false,
        lookups: Lookups::default(),
        released: fact(|p| p.released.as_deref(), true),
        edition_date: fact(|p| p.edition_date.as_deref(), true),
        recorded: fact(|p| p.recorded.as_deref(), true),
        label: fact(|p| p.label.as_deref(), false),
        catalogue: fact(|p| p.catalogue.as_deref(), false),
        country: fact(|p| p.country.as_deref(), false),
        barcode: fact(|p| p.barcode.as_deref(), false),
        format: fact(|p| p.format.as_deref(), false),
        edition: fact(|p| p.edition.as_deref(), false),
        status: None,
        origin: fact(|p| p.origin.as_deref(), false),
        notes: fact(|p| p.notes.as_deref(), false),
        studios: studios(&refs),
        edition_notes: vec![],
        groups: group_by_role(&people),
        tracks,
        links,
    }
}
