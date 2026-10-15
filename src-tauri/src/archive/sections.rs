//! Artist page sections: release types, unreleased material, and sections
//! of the user's own.
//!
//! An album counts as unreleased by the one rule (`db::release`, through
//! `album_status`), whatever its release type says, and goes to Unreleased.
//! When an artist has both demos (ARCHIVE_VERSION "demo") and other
//! unreleased material, the demos get a Demos section of their own. An
//! album the user moved to a section of their own shows there and nowhere
//! else. Sections follow `db::release::PAGE_ORDER`, then the user's own by
//! name. Empty sections are left out.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use crate::db::release::{PAGE_ORDER, ReleaseStatus};
use crate::db::types::{ArtistPage, ArtistSection};
use crate::error::AppResult;
use crate::model::{Album, ReleaseSection};

/// One album with what places it.
#[derive(Debug, Clone)]
pub struct Placed {
    pub album: Album,
    pub status: ReleaseStatus,
    /// The album's ARCHIVE_VERSION (most of its files).
    pub version: Option<String>,
    /// The user's own section for it.
    pub custom: Option<String>,
}

fn is_demo(v: Option<&str>) -> bool {
    v.is_some_and(|v| v.trim().eq_ignore_ascii_case("demo"))
}

/// The sections for these albums, in page order. Each album's `section`
/// is set to where it shows.
pub fn classify(albums: Vec<Placed>) -> Vec<ArtistSection> {
    let unreleased = |p: &Placed| p.custom.is_none() && p.status.is_unreleased();
    let demos = albums.iter().filter(|p| unreleased(p) && is_demo(p.version.as_deref())).count();
    let others = albums.iter().filter(|p| unreleased(p) && !is_demo(p.version.as_deref())).count();
    let split_demos = demos > 0 && others > 0;
    let mut by: HashMap<ReleaseSection, Vec<Album>> = HashMap::new();
    let mut custom: Vec<(String, Vec<Album>)> = Vec::new();
    for p in albums {
        let mut a = p.album;
        if let Some(name) = p.custom {
            match custom.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                Some((_, list)) => list.push(a),
                None => custom.push((name, vec![a])),
            }
            continue;
        }
        if p.status.is_unreleased() {
            a.section = if split_demos && is_demo(p.version.as_deref()) { ReleaseSection::Demos } else { ReleaseSection::Unreleased };
        } else if matches!(a.section, ReleaseSection::Demos | ReleaseSection::Unreleased) {
            // A demo type that a released version tag or mark overrides.
            a.section = ReleaseSection::Albums;
        }
        by.entry(a.section).or_default().push(a);
    }
    let mut out: Vec<ArtistSection> = PAGE_ORDER
        .iter()
        .filter_map(|s| by.remove(s).map(|albums| ArtistSection { section: *s, albums, custom: None }))
        .collect();
    custom.sort_by_key(|c| c.0.to_lowercase());
    out.extend(custom.into_iter().map(|(name, albums)| ArtistSection { section: ReleaseSection::Albums, albums, custom: Some(name) }));
    out
}

/// Regroup an artist page's sections by [`classify`]. `custom` maps album
/// ids to the user's section names.
pub fn apply(c: &Connection, page: &mut ArtistPage, custom: &HashMap<String, String>) -> AppResult<()> {
    if page.sections.is_empty() {
        return Ok(());
    }
    let mut seen = std::collections::HashSet::new();
    let albums: Vec<Album> = std::mem::take(&mut page.sections)
        .into_iter()
        .flat_map(|s| s.albums)
        .filter(|a| seen.insert(a.id.clone()))
        .collect();
    let ids: Vec<String> = albums.iter().map(|a| a.id.clone()).collect();
    let statuses = super::status::for_artist(c, &page.artist.id, &ids)?;
    let mut version_of = c.prepare_cached("SELECT version FROM album_tags WHERE album_id = ?1")?;
    let mut placed = Vec::with_capacity(albums.len());
    for album in albums {
        let version: Option<String> = version_of.query_row([&album.id], |r| r.get(0)).optional()?.flatten();
        placed.push(Placed {
            status: statuses.get(&album.id).copied().unwrap_or(ReleaseStatus::Released),
            custom: custom.get(&album.id).cloned(),
            version,
            album,
        });
    }
    page.sections = classify(placed);
    Ok(())
}
