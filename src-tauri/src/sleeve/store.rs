//! The credit index behind credit pages.
//!
//! `tag_credits` lists who each file credits (written when the file's tags
//! are read, so it covers the whole library). `sleeve_credits` keeps what
//! the other sources credited on an album, written each time its sleeve
//! notes are put together. A credit page reads both.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{Connection, params};
use serde::Serialize;

use super::{ROLES, SleeveNotes, Source};
use crate::artistinfo::fold_name;
use crate::error::AppResult;
use crate::model::Album;

/// Replace what the index holds for an album with the notes' credits: each
/// song's, and the album's own from sources that don't credit them on a song.
/// Tag credits aren't repeated here (`tag_credits` has them).
pub fn index_album(c: &Connection, notes: &SleeveNotes) -> AppResult<()> {
    index_facts(c, notes)?;
    c.execute("DELETE FROM sleeve_credits WHERE album_id = ?1", [&notes.album_id])?;
    let mut st = c.prepare_cached(
        "INSERT OR REPLACE INTO sleeve_credits (album_id, track_id, person_key, name, role, detail, sources)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut put = |track: &str, key: &str, name: &str, role: &str, details: &[String], sources: &[Source]| -> AppResult<()> {
        let sources: Vec<&str> = sources.iter().filter(|s| **s != Source::Tags).map(|s| s.key()).collect();
        if sources.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(&sources).unwrap_or_else(|_| "[]".into());
        let detail = details.join(", ");
        st.execute(params![notes.album_id, track, key, name, role, detail, json])?;
        Ok(())
    };
    for t in &notes.tracks {
        for g in &t.groups {
            for p in &g.people {
                put(&t.track_id, &p.key, &p.name, &g.role, &p.details, &p.sources)?;
            }
        }
    }
    for g in &notes.groups {
        for p in &g.people {
            // Sources that credit them on the album itself, not on a song.
            let on_songs: Vec<Source> = notes
                .tracks
                .iter()
                .flat_map(|t| t.groups.iter().filter(|tg| tg.role == g.role))
                .flat_map(|tg| tg.people.iter().filter(|x| x.key == p.key))
                .flat_map(|x| x.sources.iter().copied())
                .collect();
            let album_only: Vec<Source> = p.sources.iter().copied().filter(|s| !on_songs.contains(s)).collect();
            put("", &p.key, &p.name, &g.role, &p.details, &album_only)?;
        }
    }
    Ok(())
}

/// Keep the album's label and studios for crates (`album_facts`), folded
/// for grouping.
pub fn index_facts(c: &Connection, notes: &SleeveNotes) -> AppResult<()> {
    use crate::digging::{label_key, split_labels, studio_key};
    c.execute("DELETE FROM album_facts WHERE album_id = ?1", [&notes.album_id])?;
    let mut st = c.prepare_cached("INSERT OR IGNORE INTO album_facts (album_id, kind, fact_key, name) VALUES (?1, ?2, ?3, ?4)")?;
    if let Some(label) = &notes.label {
        for name in split_labels(&label.value) {
            let key = label_key(&name);
            if !key.is_empty() {
                st.execute(params![notes.album_id, "label", key, name])?;
            }
        }
    }
    for s in &notes.studios {
        let key = studio_key(&s.name);
        if !key.is_empty() {
            st.execute(params![notes.album_id, "studio", key, s.name.trim()])?;
        }
    }
    Ok(())
}

/// Whether the index has anything for an album.
pub fn indexed(c: &Connection, album_id: &str) -> AppResult<bool> {
    Ok(c.prepare_cached("SELECT 1 FROM sleeve_credits WHERE album_id = ?1 LIMIT 1")?.exists([album_id])?)
}

/// A song on a credit page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditSong {
    pub id: String,
    pub title: String,
}

/// One album a person is credited on, in one role.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditAlbum {
    pub album: Album,
    /// Credited on the album itself, or on every song of it.
    pub whole: bool,
    /// The songs, when not the whole album.
    pub songs: Vec<CreditSong>,
    /// Instruments, for performers.
    pub details: Vec<String>,
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditGroup {
    pub role: String,
    pub albums: Vec<CreditAlbum>,
}

/// IPC `CreditPage`: everything in the library a person is credited on, by role.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPage {
    pub key: String,
    pub name: String,
    /// A library artist with this name.
    pub artist_id: Option<String>,
    pub groups: Vec<CreditGroup>,
}

#[derive(Default)]
struct Acc {
    whole: bool,
    songs: BTreeSet<String>,
    details: Vec<String>,
    sources: BTreeSet<Source>,
}

/// A person's credit page from the index. None when nothing credits them.
pub fn credit_page(c: &Connection, name_or_key: &str) -> AppResult<Option<CreditPage>> {
    let key = fold_name(name_or_key);
    if key.is_empty() {
        return Ok(None);
    }
    // role -> album -> what's credited.
    let mut by_role: HashMap<String, HashMap<String, Acc>> = HashMap::new();
    let mut names: HashMap<String, usize> = HashMap::new();
    let add_detail = |a: &mut Acc, d: &str| {
        for d in d.split(", ").map(str::trim).filter(|d| !d.is_empty()) {
            if !a.details.iter().any(|x| x == d) {
                a.details.push(d.to_owned());
            }
        }
    };
    {
        let mut st = c.prepare_cached(
            "SELECT t.album_id, t.id, tc.name, tc.role, tc.detail FROM tag_credits tc JOIN tracks t ON t.part_key = tc.part_key
             WHERE tc.person_key = ?1",
        )?;
        let rows = st.query_map([&key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })?;
        for row in rows {
            let (album, track, name, role, detail) = row?;
            *names.entry(name).or_default() += 1;
            let a = by_role.entry(role).or_default().entry(album).or_default();
            a.songs.insert(track);
            a.sources.insert(Source::Tags);
            add_detail(a, &detail);
        }
    }
    {
        let mut st = c.prepare_cached(
            "SELECT album_id, track_id, name, role, detail, sources FROM sleeve_credits WHERE person_key = ?1",
        )?;
        let rows = st.query_map([&key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        for row in rows {
            let (album, track, name, role, detail, sources) = row?;
            *names.entry(name).or_default() += 1;
            let a = by_role.entry(role).or_default().entry(album).or_default();
            if track.is_empty() {
                a.whole = true;
            } else {
                a.songs.insert(track);
            }
            let srcs: Vec<String> = serde_json::from_str(&sources).unwrap_or_default();
            a.sources.extend(srcs.iter().filter_map(|s| Source::from_key(s)));
            add_detail(a, &detail);
        }
    }
    if by_role.is_empty() {
        return Ok(None);
    }
    // The spelling used most, the first alphabetically on a tie.
    let mut spellings: Vec<(String, usize)> = names.into_iter().collect();
    spellings.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let name = spellings.first().map(|(n, _)| n.clone()).unwrap_or_else(|| name_or_key.to_owned());
    let artist_id = super::gather::library_ids(c, std::slice::from_ref(&name))?.remove(&key);

    let album_ids: Vec<String> = {
        let mut all: BTreeSet<String> = BTreeSet::new();
        for albums in by_role.values() {
            all.extend(albums.keys().cloned());
        }
        all.into_iter().collect()
    };
    let albums: HashMap<String, Album> = crate::db::queries::albums_by_ids(c, &album_ids)?.into_iter().map(|a| (a.id.clone(), a)).collect();
    let mut song_titles = c.prepare_cached("SELECT title FROM tracks WHERE id = ?1")?;
    let mut counts = c.prepare_cached("SELECT count(*) FROM tracks WHERE album_id = ?1")?;
    let mut order = c.prepare_cached("SELECT disc_number * 10000 + track_number FROM tracks WHERE id = ?1")?;

    let mut roles: Vec<String> = by_role.keys().cloned().collect();
    roles.sort_by_key(|r| (ROLES.iter().position(|x| x == r).unwrap_or(ROLES.len()), r.clone()));
    let mut groups = Vec::new();
    for role in roles {
        let mut out: Vec<CreditAlbum> = Vec::new();
        for (album_id, acc) in by_role.remove(&role).unwrap_or_default() {
            // Albums that left the library aren't listed.
            let Some(album) = albums.get(&album_id).cloned() else { continue };
            let total: i64 = counts.query_row([&album_id], |r| r.get(0))?;
            let whole = acc.whole || (total > 0 && acc.songs.len() as i64 >= total);
            let mut songs: Vec<(i64, CreditSong)> = Vec::new();
            if !whole {
                for id in &acc.songs {
                    use rusqlite::OptionalExtension;
                    let Some(title) = song_titles.query_row([id], |r| r.get::<_, String>(0)).optional()? else { continue };
                    let n: i64 = order.query_row([id], |r| r.get(0)).optional()?.unwrap_or(0);
                    songs.push((n, CreditSong { id: id.clone(), title }));
                }
                songs.sort_by_key(|(n, _)| *n);
                if songs.is_empty() {
                    continue;
                }
            }
            out.push(CreditAlbum {
                album,
                whole,
                songs: songs.into_iter().map(|(_, s)| s).collect(),
                details: acc.details,
                sources: acc.sources.into_iter().collect(),
            });
        }
        // Newest first, then by title.
        out.sort_by(|a, b| b.album.year.cmp(&a.album.year).then_with(|| a.album.title.cmp(&b.album.title)));
        if !out.is_empty() {
            groups.push(CreditGroup { role, albums: out });
        }
    }
    if groups.is_empty() {
        return Ok(None);
    }
    Ok(Some(CreditPage { key, name, artist_id, groups }))
}
