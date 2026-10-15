//! Song lineage: every version of a song on one timeline, from the first
//! demo through sessions, leaks and alternates to the release, remasters
//! and live takes. Built on the Other versions panel's list
//! ([`super::versions::versions_of`]); each version gets a stage and a
//! place in time (ARCHIVE_DATE, else the album's year). Versions with no
//! date at all follow the dated ones, in stage order.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::eras::parse_date;
use super::versions::{VersionItem, versions_of};
use crate::error::AppResult;
use crate::model::SongStatus;

/// Where a version sits in a song's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Demo,
    Session,
    Leak,
    Alternate,
    Unreleased,
    Release,
    Remaster,
    Live,
}

/// One version on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineageNode {
    #[serde(flatten)]
    pub item: VersionItem,
    pub stage: Stage,
    /// Sort key from the date (yyyymmdd, missing parts 0); None: undated.
    pub when: Option<i32>,
    /// Released versions only: unreleased ones never leave this computer.
    pub shareable: bool,
}

/// A song's lineage, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lineage {
    /// The song's name: its released version's title, else the one asked for.
    pub title: String,
    pub artist_id: String,
    pub artist_title: String,
    pub nodes: Vec<LineageNode>,
}

fn has_word(hay: &str, words: &[&str]) -> bool {
    let lower = hay.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| words.contains(&w))
}

/// The stage of a version: its ARCHIVE_VERSION first, then words in its
/// title, album or edition, then its status.
pub fn stage_of(v: &VersionItem, album_live: bool) -> Stage {
    let tag = v.version.as_deref().unwrap_or("");
    let by_tag = match tag {
        "demo" | "snippet" | "rough" | "early" => Some(Stage::Demo),
        "session" | "sessions" | "build" | "stems" | "rehearsal" => Some(Stage::Session),
        "leak" | "leaked" => Some(Stage::Leak),
        "alternate" | "alt" | "remix" | "edit" | "instrumental" | "acoustic" => Some(Stage::Alternate),
        "remaster" | "remastered" => Some(Stage::Remaster),
        "live" => Some(Stage::Live),
        _ => None,
    };
    if let Some(s) = by_tag {
        return s;
    }
    let text = format!("{} {} {}", v.track.title, v.track.album_title, v.edition.as_deref().unwrap_or(""));
    if album_live || has_word(&text, &["live"]) {
        return Stage::Live;
    }
    if has_word(&text, &["remaster", "remastered"]) {
        return Stage::Remaster;
    }
    if has_word(&text, &["demo", "snippet", "rough"]) {
        return Stage::Demo;
    }
    if has_word(&text, &["session", "sessions"]) {
        return Stage::Session;
    }
    if has_word(&text, &["leak", "leaked"]) {
        return Stage::Leak;
    }
    match v.status {
        SongStatus::Released => Stage::Release,
        SongStatus::ProbablyAlternate => Stage::Alternate,
        SongStatus::Unreleased | SongStatus::ProbablyUnreleased => {
            if has_word(&text, &["alt", "alternate", "mix", "version"]) {
                Stage::Alternate
            } else {
                Stage::Unreleased
            }
        }
    }
}

fn album_is_live(c: &Connection, album_id: &str) -> AppResult<bool> {
    let subs: Option<String> = c
        .prepare_cached("SELECT subformats FROM albums WHERE id = ?1")?
        .query_row([album_id], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(subs
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .is_some_and(|v| v.iter().any(|x| x.eq_ignore_ascii_case("live"))))
}

/// Dated versions oldest first, then undated ones by stage.
pub fn order(nodes: &mut [LineageNode]) {
    nodes.sort_by(|a, b| {
        a.when
            .is_none()
            .cmp(&b.when.is_none())
            .then_with(|| a.when.cmp(&b.when))
            .then_with(|| a.stage.cmp(&b.stage))
            .then_with(|| a.item.track.id.cmp(&b.item.track.id))
    });
}

/// The lineage of a song (any of its versions).
pub fn lineage_of(c: &Connection, track_id: &str, show_edits: bool) -> AppResult<Lineage> {
    let items = versions_of(c, track_id, show_edits)?;
    let mut nodes = Vec::with_capacity(items.len());
    for item in items {
        let live = album_is_live(c, &item.track.album_id)?;
        let stage = stage_of(&item, live);
        let when = item.date.as_deref().and_then(parse_date).map(|d| d.key()).or(item.year.map(|y| y * 10_000));
        let shareable = item.status.is_released();
        nodes.push(LineageNode { item, stage, when, shareable });
    }
    order(&mut nodes);
    let face = nodes
        .iter()
        .find(|n| n.stage == Stage::Release)
        .or_else(|| nodes.iter().find(|n| n.item.current))
        .or(nodes.first());
    let (title, artist_id, artist_title) = match face {
        Some(n) => (
            n.item.track.title.clone(),
            n.item.track.artist_id.clone(),
            n.item.track.artist_title.clone(),
        ),
        None => return Err(crate::error::AppError::NotFound("Song".into())),
    };
    Ok(Lineage { title, artist_id, artist_title, nodes })
}
