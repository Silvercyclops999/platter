//! One artist under two names. An artist who renamed can sit in Plex as two
//! artists, and plays are counted by name, so stats would list them twice.
//! Two names count as one artist when their library artists share a
//! MusicBrainz artist id (Plex's `mbid://` guid, stored at sync), or when the
//! user said so with "Same artist as..." (kept in the sync store's
//! `artistAliases` namespace, keyed by the artist's stats key, the lower-case
//! name). Their plays add up, and the list shows the name played most.
//!
//! Joining by MusicBrainz id can be switched off (`artistAliasesAuto`), and
//! one pair it joined can be kept apart ("Not the same artist", an entry in
//! the same namespace marked `apart`).

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::Days;
use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;
use crate::sync::store::{ARTIST_ALIASES, Op};

/// "Same artist as...": this artist (the entry's key) counts as `same_as`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistAlias {
    /// The other artist's stats key.
    pub same_as: String,
    /// What Settings shows for each side.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub same_as_name: String,
    /// Unix seconds.
    #[serde(default)]
    pub added_at: i64,
    /// "Not the same artist": `artist` and `same_as` are kept apart even
    /// though their MusicBrainz ids match.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub apart: bool,
    /// The first artist of a kept-apart pair (its entry's key is the pair's).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
}

/// One manual merge (or a pair kept apart), as Settings lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasRow {
    pub key: String,
    pub name: String,
    pub same_as: String,
    pub same_as_name: String,
    pub added_at: i64,
    /// Kept apart rather than joined.
    pub apart: bool,
}

/// Two names joined because their library artists share a MusicBrainz id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPair {
    pub key: String,
    pub name: String,
    pub same_as: String,
    pub same_as_name: String,
}

/// A played artist the picker offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayedArtist {
    pub key: String,
    pub name: String,
    pub plays: i64,
}

/// Most manual merges kept.
pub const MAX_ALIASES: usize = 2_000;

/// Artist key → the key standing for its group (the smallest member), for
/// every artist counted together with another. Empty when none are.
pub type Groups = HashMap<String, String>;

fn find(parent: &mut HashMap<String, String>, k: &str) -> String {
    let mut at = k.to_string();
    while let Some(p) = parent.get(&at).filter(|p| **p != at) {
        at = p.clone();
    }
    // Shorten the path for the next lookup.
    let mut cur = k.to_string();
    while let Some(p) = parent.get(&cur).cloned().filter(|p| *p != at) {
        parent.insert(cur, at.clone());
        cur = p;
    }
    at
}

fn union(parent: &mut HashMap<String, String>, a: &str, b: &str) {
    for k in [a, b] {
        parent.entry(k.to_string()).or_insert_with(|| k.to_string());
    }
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        // The smaller key stands for the group, so every device picks the same.
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent.insert(hi, lo);
    }
}

/// The store key of a kept-apart pair (the two keys in order, hashed so the
/// key stays short).
pub fn apart_key(a: &str, b: &str) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let h = crate::remote::peer::auth::hmac_sha256(b"platter artist apart", format!("{lo}\n{hi}").as_bytes());
    format!("apart:{}", crate::remote::peer::auth::hex(&h[..12]))
}

/// Whether names matched to one MusicBrainz artist are joined on their own
/// (Settings: on unless switched off).
pub fn auto_on(c: &Connection) -> bool {
    crate::db::store::kv_get(c, "app.settings")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("artistAliasesAuto").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// The manual merges, straight from the store's table (stats queries run
/// on a plain connection).
fn manual(c: &Connection) -> AppResult<Vec<(String, ArtistAlias)>> {
    let mut st = c.prepare_cached("SELECT key, value FROM sync_entries WHERE ns = ?1 AND value IS NOT NULL")?;
    let rows = st.query_map([ARTIST_ALIASES.name()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (k, v) = row?;
        if let Ok(a) = serde_json::from_str::<ArtistAlias>(&v) {
            out.push((k, a));
        }
    }
    Ok(out)
}

/// Played artists joined by a shared MusicBrainz id, each id's names most
/// played first. Pairs kept apart are left out (a name kept apart from
/// another of its id isn't joined to any of them).
fn by_mbid(c: &Connection, apart: &HashSet<(String, String)>) -> AppResult<Vec<Vec<String>>> {
    // Library artists' MusicBrainz ids, by id and by lower-case name (plays
    // from other apps carry no id). A name two artists with different ids
    // share says nothing, so it's left out.
    let mut by_name: HashMap<String, Option<String>> = HashMap::new();
    let mut st = c.prepare_cached("SELECT lower(title), mbid FROM artists WHERE mbid IS NOT NULL AND mbid <> ''")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (name, mbid) = row?;
        by_name
            .entry(name)
            .and_modify(|m| {
                if m.as_ref().is_some_and(|x| *x != mbid) {
                    *m = None;
                }
            })
            .or_insert(Some(mbid));
    }
    if by_name.is_empty() {
        return Ok(Vec::new());
    }
    let mut st = c.prepare_cached(
        "SELECT s.artist_key, a.mbid FROM stats_artist s LEFT JOIN artists a ON a.id = s.artist_id AND a.mbid <> ''
         ORDER BY s.plays DESC, s.artist_key",
    )?;
    let mut ids: Vec<String> = Vec::new();
    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))? {
        let (key, mbid) = row?;
        let Some(mbid) = mbid.or_else(|| by_name.get(&key).cloned().flatten()) else { continue };
        if !members.contains_key(&mbid) {
            ids.push(mbid.clone());
        }
        members.entry(mbid).or_default().push(key);
    }
    let kept_apart = |a: &str, b: &str| {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        apart.contains(&(lo.to_string(), hi.to_string()))
    };
    Ok(ids
        .into_iter()
        .filter_map(|id| {
            let m = members.remove(&id)?;
            let joined: Vec<String> = m.iter().filter(|k| !m.iter().any(|o| o != *k && kept_apart(k, o))).cloned().collect();
            (joined.len() > 1).then_some(joined)
        })
        .collect())
}

/// The pairs kept apart, each in key order.
fn apart_pairs(manual: &[(String, ArtistAlias)]) -> HashSet<(String, String)> {
    manual
        .iter()
        .filter(|(_, a)| a.apart && !a.artist.is_empty() && !a.same_as.is_empty())
        .map(|(_, a)| if a.artist <= a.same_as { (a.artist.clone(), a.same_as.clone()) } else { (a.same_as.clone(), a.artist.clone()) })
        .collect()
}

/// Names joined because they share a MusicBrainz id, as Settings lists
/// them (each with the name played most). Empty while that's switched off.
pub fn auto_pairs(c: &Connection) -> AppResult<Vec<AutoPair>> {
    if !auto_on(c) {
        return Ok(Vec::new());
    }
    let m = manual(c)?;
    let name = |k: &str| -> String {
        c.query_row("SELECT name FROM stats_artist WHERE artist_key = ?1", [k], |r| r.get::<_, String>(0)).unwrap_or_else(|_| k.to_string())
    };
    let mut out = Vec::new();
    for g in by_mbid(c, &apart_pairs(&m))? {
        let top = &g[0];
        for k in &g[1..] {
            out.push(AutoPair { key: k.clone(), name: name(k), same_as: top.clone(), same_as_name: name(top) });
        }
    }
    Ok(out)
}

/// Which played artists count as one.
pub fn groups(c: &Connection) -> AppResult<Groups> {
    let mut parent: HashMap<String, String> = HashMap::new();
    let m = manual(c)?;
    if auto_on(c) {
        for g in by_mbid(c, &apart_pairs(&m))? {
            for k in &g[1..] {
                union(&mut parent, &g[0], k);
            }
        }
    }
    for (key, a) in m {
        if !a.apart && !a.same_as.is_empty() && a.same_as != key {
            union(&mut parent, &key, &a.same_as);
        }
    }
    let keys: Vec<String> = parent.keys().cloned().collect();
    let mut out = Groups::new();
    for k in keys {
        let root = find(&mut parent, &k);
        out.insert(k, root);
    }
    // Only artists that share a group with another.
    let mut size: HashMap<&str, usize> = HashMap::new();
    for g in out.values() {
        *size.entry(g.as_str()).or_default() += 1;
    }
    let lone: HashSet<String> = size.into_iter().filter(|(_, n)| *n < 2).map(|(g, _)| g.to_string()).collect();
    out.retain(|_, g| !lone.contains(g));
    Ok(out)
}

/// Fold per-artist counts (most played first) into one row per group: the
/// counts added up, under the key of the member played most.
pub fn fold(rows: &[(String, i64, i64)], g: &Groups) -> Vec<(String, i64, i64)> {
    let mut at: HashMap<&str, usize> = HashMap::new();
    let mut out: Vec<(String, i64, i64, (i64, i64))> = Vec::new();
    for (k, p, m) in rows {
        let group = g.get(k).map(String::as_str).unwrap_or(k);
        match at.get(group) {
            Some(&i) => {
                let row = &mut out[i];
                row.1 += p;
                row.2 += m;
                // The name shown is the one played most.
                if (*p, *m) > row.3 {
                    row.0 = k.clone();
                    row.3 = (*p, *m);
                }
            }
            None => {
                at.insert(group, out.len());
                out.push((k.clone(), *p, *m, (*p, *m)));
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(k, p, m, _)| (k, p, m)).collect()
}

/// Every artist with plays in the range and its counts, most played first.
pub fn artist_counts(c: &Connection, d: Days) -> AppResult<Vec<(String, i64, i64)>> {
    let mut st = if d.all {
        c.prepare_cached("SELECT artist_key, plays, ms FROM stats_artist WHERE plays > 0 ORDER BY plays DESC, ms DESC, artist_key")?
    } else {
        c.prepare_cached(
            "SELECT artist_key, sum(plays) AS p, sum(ms) AS m FROM stats_artist_day WHERE day BETWEEN ?1 AND ?2
             GROUP BY artist_key ORDER BY p DESC, m DESC, artist_key",
        )?
    };
    let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
    let rows = if d.all { st.query_map([], map)?.collect::<Result<Vec<_>, _>>()? } else { st.query_map([d.lo, d.hi], map)?.collect::<Result<Vec<_>, _>>()? };
    Ok(rows)
}

/// How many artists the range has, an artist under two names counted once.
pub fn distinct_artists(c: &Connection, d: Days, g: &Groups) -> AppResult<i64> {
    let rows = artist_counts(c, d)?;
    let set: HashSet<&str> = rows.iter().map(|(k, _, _)| g.get(k).map(String::as_str).unwrap_or(k)).collect();
    Ok(set.len() as i64)
}

/// Artists first played in the range (no plays before it under any of their
/// names), with their counts and first day, most played first.
pub fn new_artists(c: &Connection, d: Days, g: &Groups) -> AppResult<Vec<(String, i64, i64, i64)>> {
    let mut st = c.prepare_cached(
        "SELECT n.artist_key, sum(n.plays) AS p, sum(n.ms) AS m, min(n.day) AS f FROM stats_artist_day n
         WHERE n.day BETWEEN ?1 AND ?2
           AND NOT EXISTS (SELECT 1 FROM stats_artist_day o WHERE o.artist_key = n.artist_key AND o.day < ?1)
         GROUP BY n.artist_key ORDER BY p DESC, m DESC, n.artist_key",
    )?;
    let rows = st
        .query_map([d.lo, d.hi], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    if g.is_empty() {
        return Ok(rows);
    }
    // A group is new only when none of its names was played before.
    let mut before: HashSet<&str> = HashSet::new();
    let mut st = c.prepare_cached("SELECT 1 FROM stats_artist_day WHERE artist_key = ?1 AND day < ?2 LIMIT 1")?;
    for (k, group) in g {
        if st.exists(params![k, d.lo])? {
            before.insert(group.as_str());
        }
    }
    let fresh: Vec<(String, i64, i64, i64)> = rows.into_iter().filter(|(k, ..)| g.get(k).is_none_or(|grp| !before.contains(grp.as_str()))).collect();
    let first: HashMap<&str, i64> = fresh.iter().fold(HashMap::new(), |mut m, (k, _, _, f)| {
        let grp = g.get(k).map(String::as_str).unwrap_or(k);
        let e = m.entry(grp).or_insert(*f);
        *e = (*e).min(*f);
        m
    });
    let counts: Vec<(String, i64, i64)> = fresh.iter().map(|(k, p, m, _)| (k.clone(), *p, *m)).collect();
    Ok(fold(&counts, g)
        .into_iter()
        .map(|(k, p, m)| {
            let f = first[g.get(&k).map(String::as_str).unwrap_or(&k)];
            (k, p, m, f)
        })
        .collect())
}

/// Every name of the artists behind `keys` (their own and the ones they count as).
pub fn members(keys: &[String], g: &Groups) -> Vec<String> {
    let groups: HashSet<&str> = keys.iter().map(|k| g.get(k).map(String::as_str).unwrap_or(k)).collect();
    let mut out: Vec<String> = keys.to_vec();
    for (k, grp) in g {
        if groups.contains(grp.as_str()) && !out.contains(k) {
            out.push(k.clone());
        }
    }
    out
}

/// Played artists whose name has `query` in it (every one for an empty
/// query), most played first, for the "Same artist as..." picker.
pub fn played_artists(c: &Connection, query: &str, limit: u32) -> AppResult<Vec<PlayedArtist>> {
    let q = query.trim().to_lowercase();
    let mut st = c.prepare_cached(
        "SELECT artist_key, name, plays FROM stats_artist WHERE plays > 0 AND instr(artist_key, ?1) > 0
         ORDER BY plays DESC, artist_key LIMIT ?2",
    )?;
    let rows = st.query_map(params![q, limit.clamp(1, 100)], |r| Ok(PlayedArtist { key: r.get(0)?, name: r.get(1)?, plays: r.get(2)? }))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The manual merges, kept in the sync store.
#[derive(Clone)]
pub struct Aliases {
    store: SyncStore,
    clock: std::sync::Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Aliases {
    pub fn new(store: SyncStore) -> Self {
        Self { store, clock: std::sync::Arc::new(crate::db::queries::now_s) }
    }

    pub fn with_clock(store: SyncStore, clock: std::sync::Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self { store, clock }
    }

    /// Newest first.
    pub fn list(&self) -> AppResult<Vec<AliasRow>> {
        let mut out: Vec<AliasRow> = self
            .store
            .list(&ARTIST_ALIASES)?
            .into_iter()
            .map(|(key, a)| AliasRow {
                key: if a.apart { a.artist.clone() } else { key },
                name: a.name,
                same_as: a.same_as,
                same_as_name: a.same_as_name,
                added_at: a.added_at,
                apart: a.apart,
            })
            .collect();
        out.sort_by(|a, b| b.added_at.cmp(&a.added_at).then_with(|| a.key.cmp(&b.key)));
        Ok(out)
    }

    /// Count artist `key` as `same_as` (`on`), or stop (`on` false; `same_as`
    /// unused). Keys are stats keys (lower-case names); names are for Settings.
    pub fn set(&self, key: &str, name: &str, same_as: &str, same_as_name: &str, on: bool) -> AppResult<Vec<AliasRow>> {
        let key = key.trim().to_lowercase();
        if key.is_empty() || key.len() > 190 {
            return Err(AppError::Invalid("Pick an artist.".into()));
        }
        if !on {
            // Both ways: a merge made from either side (here or on another
            // device) stops counting.
            let mut ops = vec![Op::remove(&ARTIST_ALIASES, key.clone())];
            let was = self.store.get(&ARTIST_ALIASES, &key)?.filter(|a| !a.apart).map(|a| a.same_as);
            if let Some(other) = was
                && self.store.get(&ARTIST_ALIASES, &other)?.is_some_and(|a| !a.apart && a.same_as == key)
            {
                ops.push(Op::remove(&ARTIST_ALIASES, other));
            }
            self.store.write_many(ops)?;
            return self.list();
        }
        let same_as = same_as.trim().to_lowercase();
        if same_as.is_empty() || same_as.len() > 190 {
            return Err(AppError::Invalid("Pick the artist they're the same as.".into()));
        }
        if same_as == key {
            return Err(AppError::Invalid("That's the same name.".into()));
        }
        let cur = self.list()?;
        if cur.len() >= MAX_ALIASES && !cur.iter().any(|a| a.key == key) {
            return Err(AppError::Invalid("You've merged as many artists as Platter can keep.".into()));
        }
        let pick = |s: &str, k: &str| if s.trim().is_empty() { k.to_string() } else { s.trim().to_string() };
        let v = ArtistAlias {
            same_as: same_as.clone(),
            name: pick(name, &key),
            same_as_name: pick(same_as_name, &same_as),
            added_at: (self.clock)(),
            ..Default::default()
        };
        self.store.write_many(vec![Op::put(&ARTIST_ALIASES, key, &v)?])?;
        self.list()
    }

    /// "Not the same artist": keep two names apart though their MusicBrainz
    /// ids match (`on`), or join them again (`on` false).
    pub fn apart(&self, key: &str, name: &str, other: &str, other_name: &str, on: bool) -> AppResult<Vec<AliasRow>> {
        let (key, other) = (key.trim().to_lowercase(), other.trim().to_lowercase());
        if key.is_empty() || other.is_empty() || key.len() > 190 || other.len() > 190 || key == other {
            return Err(AppError::Invalid("Pick two different artists.".into()));
        }
        let id = apart_key(&key, &other);
        if !on {
            self.store.write_many(vec![Op::remove(&ARTIST_ALIASES, id)])?;
            return self.list();
        }
        if self.list()?.len() >= MAX_ALIASES {
            return Err(AppError::Invalid("You've merged as many artists as Platter can keep.".into()));
        }
        let pick = |s: &str, k: &str| if s.trim().is_empty() { k.to_string() } else { s.trim().to_string() };
        let v = ArtistAlias {
            same_as: other.clone(),
            name: pick(name, &key),
            same_as_name: pick(other_name, &other),
            added_at: (self.clock)(),
            apart: true,
            artist: key,
        };
        self.store.write_many(vec![Op::put(&ARTIST_ALIASES, id, &v)?])?;
        self.list()
    }
}

// ---- commands -----------------------------------------------------------------

#[tauri::command]
pub async fn artist_aliases(aliases: tauri::State<'_, Aliases>) -> AppResult<Vec<AliasRow>> {
    let a = aliases.inner().clone();
    tokio::task::spawn_blocking(move || a.list()).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn artist_alias_set(
    aliases: tauri::State<'_, Aliases>,
    key: String,
    name: Option<String>,
    same_as: Option<String>,
    same_as_name: Option<String>,
    on: bool,
) -> AppResult<Vec<AliasRow>> {
    let a = aliases.inner().clone();
    tokio::task::spawn_blocking(move || {
        a.set(&key, name.as_deref().unwrap_or(""), same_as.as_deref().unwrap_or(""), same_as_name.as_deref().unwrap_or(""), on)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn artist_alias_apart(
    aliases: tauri::State<'_, Aliases>,
    key: String,
    name: Option<String>,
    other: String,
    other_name: Option<String>,
    on: bool,
) -> AppResult<Vec<AliasRow>> {
    let a = aliases.inner().clone();
    tokio::task::spawn_blocking(move || a.apart(&key, name.as_deref().unwrap_or(""), &other, other_name.as_deref().unwrap_or(""), on))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

/// Names joined because they share a MusicBrainz id.
#[tauri::command]
pub async fn artist_alias_auto_pairs(db: tauri::State<'_, crate::db::DbHandle>) -> AppResult<Vec<AutoPair>> {
    super::commands::fresh(&db).await?;
    db.read_async(auto_pairs).await
}

#[tauri::command]
pub async fn stats_played_artists(db: tauri::State<'_, crate::db::DbHandle>, query: String, limit: Option<u32>) -> AppResult<Vec<PlayedArtist>> {
    super::commands::fresh(&db).await?;
    db.read_async(move |c| played_artists(c, &query, limit.unwrap_or(20))).await
}
