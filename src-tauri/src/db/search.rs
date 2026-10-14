//! Local search.
//!
//! - FTS5 over artists, albums (+ album artist), tracks (+ album artist,
//!   album, track artist) and playlists; tokenizer `unicode61
//!   remove_diacritics 2`, so "beyonce" finds "Beyoncé".
//! - Every query token is a prefix match (search as you type).
//! - Typo tolerance: a token of ≥ 5 chars that is not a prefix of any indexed
//!   word gets alternatives within 1 edit (insert / delete / substitute /
//!   transpose) from an in-memory vocabulary built from the FTS vocab tables.
//! - User input never reaches FTS syntax: it is folded, split into word
//!   tokens and each token is double-quoted, so quotes, `*`, `-`, `NEAR`,
//!   `AND`/`OR`/`NOT`, column filters and parentheses are inert.
//! - Ranking: tier first (exact title > title prefix > word prefix > fuzzy),
//!   then boosts for play count and recency, a small kind bias for the top
//!   result (artist > album > track > playlist) and shorter titles first.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::{Connection, params};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::error::AppResult;
use crate::model::{Album, Artist, Playlist, Track};

use super::DbHandle;
use super::popular;
use super::queries::now_s;
use super::store::{ALBUM_COLS, ARTIST_COLS, PLAYLIST_COLS, TRACK_COLS, row_album, row_artist, row_playlist, row_track};
use super::source::RemoteHits;
use super::types::*;

const MAX_QUERY_CHARS: usize = 200;
const MAX_TOKENS: usize = 8;
const MAX_TOKEN_CHARS: usize = 40;
const FUZZY_MIN_CHARS: usize = 5;
const MAX_FUZZY_ALTS: usize = 8;
const TRACK_CANDIDATES: i64 = 300;
const OTHER_CANDIDATES: i64 = 120;
const ALL_SONGS: usize = 4;
const ALL_SHELF: usize = 10;
const FILTER_DEFAULT: usize = 50;
const VOCAB_RELOAD_MIN: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- text

/// Lower-case, diacritic-free form used for matching and ranking.
pub fn normalize(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// Word tokens the way FTS5 unicode61 splits them (alphanumerics).
pub fn tokens(normalized: &str) -> Vec<String> {
    normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// True when `a` and `b` are within one edit (optimal string alignment:
/// insert, delete, substitute or swap two adjacent chars).
pub fn within_one_edit(a: &[char], b: &[char]) -> bool {
    let (la, lb) = (a.len(), b.len());
    if la.abs_diff(lb) > 1 {
        return false;
    }
    let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    if p == la && p == lb {
        return true;
    }
    let (ra, rb) = (&a[p..], &b[p..]);
    match la.cmp(&lb) {
        Ordering::Equal => {
            ra[1..] == rb[1..] || (ra.len() >= 2 && ra[0] == rb[1] && ra[1] == rb[0] && ra[2..] == rb[2..])
        }
        Ordering::Greater => ra[1..] == *rb,
        Ordering::Less => *ra == rb[1..],
    }
}

// ---------------------------------------------------------------- vocab

struct Vocab {
    generation: u64,
    loaded_at: Instant,
    /// Sorted, deduped terms (already folded by the tokenizer).
    sorted: Vec<String>,
    /// Terms as chars, bucketed by char length.
    by_len: Vec<Vec<Vec<char>>>,
}

impl Vocab {
    fn load(c: &Connection, generation: u64) -> AppResult<Self> {
        let mut sorted = Vec::new();
        for t in ["artists_vocab", "albums_vocab", "tracks_vocab", "playlists_vocab"] {
            let mut st = c.prepare(&format!("SELECT term FROM {t}"))?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            for r in rows {
                sorted.push(r?);
            }
        }
        sorted.sort_unstable();
        sorted.dedup();
        let mut by_len: Vec<Vec<Vec<char>>> = Vec::new();
        for t in &sorted {
            let chars: Vec<char> = t.chars().collect();
            if by_len.len() <= chars.len() {
                by_len.resize_with(chars.len() + 1, Vec::new);
            }
            by_len[chars.len()].push(chars);
        }
        Ok(Self { generation, loaded_at: Instant::now(), sorted, by_len })
    }

    fn has_prefix(&self, token: &str) -> bool {
        let i = self.sorted.partition_point(|t| t.as_str() < token);
        self.sorted.get(i).is_some_and(|t| t.starts_with(token))
    }

    fn fuzzy(&self, token: &str) -> Vec<String> {
        let q: Vec<char> = token.chars().collect();
        let mut out = Vec::new();
        for len in q.len().saturating_sub(1)..=q.len() + 1 {
            let Some(bucket) = self.by_len.get(len) else { continue };
            for cand in bucket {
                if *cand != q && within_one_edit(&q, cand) {
                    out.push(cand.iter().collect());
                    if out.len() >= MAX_FUZZY_ALTS {
                        return out;
                    }
                }
            }
        }
        out
    }
}

#[derive(Default)]
pub(crate) struct VocabCache {
    inner: Mutex<Option<std::sync::Arc<Vocab>>>,
}

impl VocabCache {
    fn get(&self, c: &Connection, generation: u64) -> AppResult<std::sync::Arc<Vocab>> {
        let mut g = self.inner.lock();
        if let Some(v) = g.as_ref() {
            let fresh = v.generation == generation || v.loaded_at.elapsed() < VOCAB_RELOAD_MIN;
            if fresh {
                return Ok(v.clone());
            }
        }
        let v = std::sync::Arc::new(Vocab::load(c, generation)?);
        *g = Some(v.clone());
        Ok(v)
    }

    /// Reload unconditionally (after a sync finished).
    fn force(&self, c: &Connection, generation: u64) -> AppResult<()> {
        let v = std::sync::Arc::new(Vocab::load(c, generation)?);
        *self.inner.lock() = Some(v);
        Ok(())
    }
}

// ---------------------------------------------------------------- query

/// A parsed, sanitized query.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    /// Folded query text (tokens joined by single spaces).
    pub norm: String,
    pub tokens: Vec<String>,
    /// FTS5 MATCH expression; None when there is nothing to search for.
    pub fts: Option<String>,
    /// A lone 1–2 char token (first keystrokes). Matching every word with
    /// that prefix and bm25-ranking tens of thousands of rows is too slow
    /// and not useful, so short queries match titles that *start* with the
    /// token (`title : ^"r"*`) and take the most played candidates.
    pub short: bool,
}

const SHORT_MAX_CHARS: usize = 2;

fn quote(t: &str) -> String {
    format!("\"{}\"", t.replace('"', "\"\""))
}

/// Parse user input into a safe FTS5 expression. `alts(token)` returns
/// typo alternatives for a token (empty when none apply).
pub fn parse_query(raw: &str, mut alts: impl FnMut(&str) -> Vec<String>) -> ParsedQuery {
    let clipped: String = raw.chars().take(MAX_QUERY_CHARS).collect();
    let toks: Vec<String> = tokens(&normalize(&clipped))
        .into_iter()
        .take(MAX_TOKENS)
        .map(|t| t.chars().take(MAX_TOKEN_CHARS).collect())
        .collect();
    if toks.is_empty() {
        return ParsedQuery { norm: String::new(), tokens: toks, fts: None, short: false };
    }
    if toks.len() == 1 && toks[0].chars().count() <= SHORT_MAX_CHARS {
        let fts = format!("title : ^{}*", quote(&toks[0]));
        return ParsedQuery { norm: toks[0].clone(), fts: Some(fts), tokens: toks, short: true };
    }
    let groups: Vec<String> = toks
        .iter()
        .map(|t| {
            let mut parts = vec![format!("{}*", quote(t))];
            parts.extend(alts(t).iter().map(|a| quote(a)));
            if parts.len() == 1 { parts.remove(0) } else { format!("({})", parts.join(" OR ")) }
        })
        .collect();
    ParsedQuery { norm: toks.join(" "), fts: Some(groups.join(" ")), tokens: toks, short: false }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Fuzzy = 0,
    WordPrefix = 1,
    TitlePrefix = 2,
    Exact = 3,
}

/// Match tier of a result given its title and other searchable fields.
pub fn tier(q: &ParsedQuery, title: &str, others: &[Option<&str>]) -> (Tier, bool) {
    let t = normalize(title);
    let t_join = tokens(&t).join(" ");
    if t_join == q.norm {
        return (Tier::Exact, true);
    }
    if t_join.starts_with(&q.norm) {
        return (Tier::TitlePrefix, true);
    }
    let title_toks = tokens(&t);
    let mut all_toks = title_toks.clone();
    for o in others.iter().flatten() {
        all_toks.extend(tokens(&normalize(o)));
    }
    let hit = |toks: &[String]| q.tokens.iter().all(|qt| toks.iter().any(|w| w.starts_with(qt.as_str())));
    if hit(&title_toks) {
        (Tier::WordPrefix, true)
    } else if hit(&all_toks) {
        (Tier::WordPrefix, false)
    } else {
        (Tier::Fuzzy, false)
    }
}

/// Final score: tier dominates (×100), boosts stay below 50.
pub fn score(tier: Tier, in_title: bool, plays: u32, last_viewed: Option<i64>, title_len: usize, now: i64) -> f64 {
    let plays_boost = (6.0 * (1.0 + plays as f64).ln()).min(25.0);
    let recency = match last_viewed {
        Some(t) => {
            let days = ((now - t).max(0) as f64) / 86_400.0;
            if days <= 7.0 { 10.0 } else { (10.0 * (1.0 - (days - 7.0) / 83.0)).max(0.0) }
        }
        None => 0.0,
    };
    let title_bonus = if in_title { 5.0 } else { 0.0 };
    let len_penalty = (title_len as f64 / 20.0).min(4.0);
    tier as i32 as f64 * 100.0 + plays_boost + recency + title_bonus - len_penalty
}

struct Scored<T> {
    item: T,
    score: f64,
}

fn rank<T>(mut v: Vec<Scored<T>>, n: usize) -> Vec<Scored<T>> {
    v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    v.truncate(n);
    v
}

/// Candidate query for one kind. Normal queries take the best bm25 matches;
/// short queries take the most played titles starting with the token.
fn candidates_sql(cols: &str, table: &str, alias: &str, bm25: &str, popular: &str, short: bool) -> String {
    if short {
        format!(
            "SELECT {cols} FROM {table} {alias} WHERE {alias}.pk IN (
                SELECT rowid FROM {table}_fts WHERE {table}_fts MATCH ?1
             ) ORDER BY {popular} LIMIT ?2"
        )
    } else {
        format!(
            "SELECT {cols} FROM {table} {alias} JOIN (
                SELECT rowid AS r FROM {table}_fts WHERE {table}_fts MATCH ?1 ORDER BY {bm25} LIMIT ?2
             ) f ON {alias}.pk = f.r"
        )
    }
}

fn search_artists(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize) -> AppResult<Vec<Scored<Artist>>> {
    let sql = candidates_sql(ARTIST_COLS, "artists", "ar", "rank", "ar.view_count DESC", q.short);
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, OTHER_CANDIDATES], row_artist)?;
    let mut v = Vec::new();
    for a in rows {
        let a = a?;
        let (t, in_title) = tier(q, &a.title, &[]);
        let s = score(t, in_title, a.view_count, None, a.title.len(), now) + 3.0;
        v.push(Scored { item: a, score: s });
    }
    Ok(rank(v, n))
}

/// Credited artists (names in track artist lines with no library artist)
/// whose name matches the query, with the plays of the tracks crediting them.
fn search_credited(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize) -> AppResult<Vec<Scored<Artist>>> {
    if q.short || n == 0 {
        return Ok(Vec::new());
    }
    let mut st = c.prepare_cached(
        "SELECT t.track_artist, t.view_count FROM tracks_fts JOIN tracks t ON t.pk = tracks_fts.rowid
         WHERE tracks_fts MATCH ?1 LIMIT ?2",
    )?;
    let rows: Vec<(Option<String>, u32)> = st
        .query_map(params![format!("track_artist : ({fts})"), TRACK_CANDIDATES], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut in_library = c.prepare_cached("SELECT 1 FROM artists WHERE title = ?1 COLLATE NOCASE LIMIT 1")?;
    let mut found: Vec<(String, String, u32, Tier)> = Vec::new();
    for (line, plays) in rows {
        let Some(line) = line else { continue };
        for name in popular::line_credits(c, &line)? {
            let key = name.to_lowercase();
            if let Some(f) = found.iter_mut().find(|f| f.0 == key) {
                f.2 = f.2.saturating_add(plays);
                continue;
            }
            let (t, in_title) = tier(q, &name, &[]);
            if !in_title || in_library.exists([&name])? {
                continue;
            }
            found.push((key, name, plays, t));
        }
    }
    let v = found
        .into_iter()
        .map(|(_, name, plays, t)| {
            let s = score(t, true, plays, None, name.len(), now) + 3.0;
            let artist = Artist {
                id: popular::credit_id(&name),
                sort_title: name.clone(),
                title: name,
                thumb: None,
                art: None,
                view_count: plays,
                summary: None,
            };
            Scored { item: artist, score: s }
        })
        .collect();
    Ok(rank(v, n))
}

fn search_albums(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize) -> AppResult<Vec<Scored<Album>>> {
    let sql = candidates_sql(
        ALBUM_COLS,
        "albums",
        "al",
        "bm25(albums_fts, 10.0, 3.0)",
        "al.view_count DESC, al.last_viewed_at DESC",
        q.short,
    );
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, OTHER_CANDIDATES], row_album)?;
    let mut v = Vec::new();
    for a in rows {
        let a = a?;
        let (t, in_title) = tier(q, &a.title, &[Some(&a.artist_title)]);
        let s = score(t, in_title, a.view_count, a.last_viewed_at, a.title.len(), now) + 2.0;
        v.push(Scored { item: a, score: s });
    }
    Ok(rank(v, n))
}

fn search_tracks(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize) -> AppResult<Vec<Scored<Track>>> {
    let sql = candidates_sql(
        TRACK_COLS,
        "tracks",
        "t",
        "bm25(tracks_fts, 10.0, 3.0, 2.0, 3.0)",
        "t.view_count DESC, t.last_viewed_at DESC",
        q.short,
    );
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, TRACK_CANDIDATES], row_track)?;
    let mut v = Vec::new();
    for tr in rows {
        let tr = tr?;
        let (t, in_title) = tier(
            q,
            &tr.title,
            &[Some(&tr.artist_title), Some(&tr.album_title), tr.track_artist.as_deref()],
        );
        let s = score(t, in_title, tr.view_count, tr.last_viewed_at, tr.title.len(), now) + 1.0;
        v.push(Scored { item: tr, score: s });
    }
    Ok(rank(v, n))
}

fn search_playlists(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize) -> AppResult<Vec<Scored<Playlist>>> {
    let sql = candidates_sql(PLAYLIST_COLS, "playlists", "p", "rank", "p.title COLLATE NOCASE", q.short);
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, OTHER_CANDIDATES], row_playlist)?;
    let mut v = Vec::new();
    for p in rows {
        let p = p?;
        let (t, in_title) = tier(q, &p.title, &[]);
        let s = score(t, in_title, 0, None, p.title.len(), now);
        v.push(Scored { item: p, score: s });
    }
    Ok(rank(v, n))
}

/// How many results of `kind` a search with `filter`/`limit` returns.
fn kind_limit(filter: SearchFilter, limit: Option<u32>, kind: SearchFilter, all_n: usize) -> usize {
    match (filter, limit) {
        // An explicit limit applies to every kind; without one, All gives the
        // short lists (four songs).
        (SearchFilter::All, Some(l)) => (l as usize).clamp(1, 50),
        (SearchFilter::All, _) => all_n,
        (f, l) if f == kind => l.map(|l| l as usize).unwrap_or(FILTER_DEFAULT).clamp(1, 200),
        _ => 0,
    }
}

/// Run a search on a read connection. `generation` keys the vocab cache.
// The read connection, its cache keys and the request: one call site.
#[allow(clippy::too_many_arguments)]
pub(crate) fn search_conn(
    c: &Connection,
    vocab: &VocabCache,
    generation: u64,
    query: &str,
    filter: SearchFilter,
    limit: Option<u32>,
    now: i64,
    credited: bool,
) -> AppResult<SearchResults> {
    let start = Instant::now();
    let mut out = SearchResults::default();

    // Typo alternatives only for tokens that match nothing as typed.
    let mut vocab_ref = None;
    let mut vocab_err = None;
    let q = parse_query(query, |t| {
        if t.chars().count() < FUZZY_MIN_CHARS {
            return Vec::new();
        }
        if vocab_ref.is_none() {
            match vocab.get(c, generation) {
                Ok(v) => vocab_ref = Some(v),
                Err(e) => {
                    vocab_err = Some(e);
                    return Vec::new();
                }
            }
        }
        let v = vocab_ref.as_ref().expect("vocab loaded");
        if v.has_prefix(t) { Vec::new() } else { v.fuzzy(t) }
    });
    if let Some(e) = vocab_err {
        tracing::warn!("search vocab: {e}");
    }
    let Some(fts) = q.fts.as_deref() else {
        out.took_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Ok(out);
    };

    let n_for = |kind: SearchFilter, all_n: usize| kind_limit(filter, limit, kind, all_n);

    let mut top: Option<(f64, SearchHit)> = None;
    let mut consider = |score: f64, hit: SearchHit| {
        if top.as_ref().is_none_or(|(s, _)| score > *s) {
            top = Some((score, hit));
        }
    };

    let n = n_for(SearchFilter::Artists, ALL_SHELF);
    if n > 0 {
        let mut v = search_artists(c, &q, fts, now, n)?;
        if credited {
            v.extend(search_credited(c, &q, fts, now, n)?);
            v = rank(v, n);
        }
        if let Some(f) = v.first() {
            consider(f.score, SearchHit { kind: SearchKind::Artist, artist: Some(f.item.clone()), album: None, track: None, playlist: None });
        }
        out.artists = v.into_iter().map(|s| s.item).collect();
    }
    let n = n_for(SearchFilter::Albums, ALL_SHELF);
    if n > 0 {
        let v = search_albums(c, &q, fts, now, n)?;
        if let Some(f) = v.first() {
            consider(f.score, SearchHit { kind: SearchKind::Album, artist: None, album: Some(f.item.clone()), track: None, playlist: None });
        }
        out.albums = v.into_iter().map(|s| s.item).collect();
    }
    let n = n_for(SearchFilter::Songs, ALL_SONGS);
    if n > 0 {
        let v = search_tracks(c, &q, fts, now, n)?;
        if let Some(f) = v.first() {
            consider(f.score, SearchHit { kind: SearchKind::Track, artist: None, album: None, track: Some(f.item.clone()), playlist: None });
        }
        out.songs = v.into_iter().map(|s| s.item).collect();
    }
    let n = n_for(SearchFilter::Playlists, ALL_SHELF);
    if n > 0 {
        let v = search_playlists(c, &q, fts, now, n)?;
        if let Some(f) = v.first() {
            consider(f.score, SearchHit { kind: SearchKind::Playlist, artist: None, album: None, track: None, playlist: Some(f.item.clone()) });
        }
        out.playlists = v.into_iter().map(|s| s.item).collect();
    }
    if filter == SearchFilter::All {
        out.top = top.map(|(_, h)| h);
    }
    out.took_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(out)
}

/// Load the typo vocabulary for the current library generation.
pub(crate) fn warm(db: &DbHandle, c: &Connection) -> AppResult<()> {
    db.inner.vocab.force(c, db.generation())
}

/// Search the local index.
pub fn search(db: &DbHandle, query: &str, filter: SearchFilter, limit: Option<u32>) -> AppResult<SearchResults> {
    let generation = db.generation();
    let credited = db.link_featured();
    db.read(|c| search_conn(c, &db.inner.vocab, generation, query, filter, limit, now_s(), credited))
}

// ---------------------------------------------------------------- server fallback

/// How long to wait for the server before answering from the cache.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(4);
/// Items per kind asked of the server.
const REMOTE_LIMIT: u32 = 30;

/// Rank server matches with the same rules as the local index and shape
/// them like a local result (`source: server`).
pub fn rank_remote(
    query: &str,
    filter: SearchFilter,
    limit: Option<u32>,
    hits: RemoteHits,
    now: i64,
) -> SearchResults {
    let start = Instant::now();
    let q = parse_query(query, |_| Vec::new());
    let mut out = SearchResults { source: SearchSource::Server, ..Default::default() };
    if q.fts.is_none() {
        return out;
    }
    let n_for = |kind: SearchFilter, all_n: usize| kind_limit(filter, limit, kind, all_n);
    let mut top: Option<(f64, SearchHit)> = None;
    let mut consider = |score: f64, hit: SearchHit| {
        if top.as_ref().is_none_or(|(s, _)| score > *s) {
            top = Some((score, hit));
        }
    };
    let hit = |kind| SearchHit { kind, artist: None, album: None, track: None, playlist: None };

    let artists = rank(
        hits.artists
            .into_iter()
            .map(|a| {
                let (t, in_title) = tier(&q, &a.title, &[]);
                let score = score(t, in_title, a.view_count, None, a.title.len(), now) + 3.0;
                Scored { item: a, score }
            })
            .collect(),
        n_for(SearchFilter::Artists, ALL_SHELF),
    );
    if let Some(f) = artists.first() {
        consider(f.score, SearchHit { artist: Some(f.item.clone()), ..hit(SearchKind::Artist) });
    }
    let albums = rank(
        hits.albums
            .into_iter()
            .map(|a| {
                let (t, in_title) = tier(&q, &a.title, &[Some(&a.artist_title)]);
                let score = score(t, in_title, a.view_count, a.last_viewed_at, a.title.len(), now) + 2.0;
                Scored { item: a, score }
            })
            .collect(),
        n_for(SearchFilter::Albums, ALL_SHELF),
    );
    if let Some(f) = albums.first() {
        consider(f.score, SearchHit { album: Some(f.item.clone()), ..hit(SearchKind::Album) });
    }
    let songs = rank(
        hits.tracks
            .into_iter()
            .map(|tr| {
                let (t, in_title) = tier(
                    &q,
                    &tr.title,
                    &[Some(&tr.artist_title), Some(&tr.album_title), tr.track_artist.as_deref()],
                );
                let score = score(t, in_title, tr.view_count, tr.last_viewed_at, tr.title.len(), now) + 1.0;
                Scored { item: tr, score }
            })
            .collect(),
        n_for(SearchFilter::Songs, ALL_SONGS),
    );
    if let Some(f) = songs.first() {
        consider(f.score, SearchHit { track: Some(f.item.clone()), ..hit(SearchKind::Track) });
    }
    let playlists = rank(
        hits.playlists
            .into_iter()
            .map(|p| {
                let (t, in_title) = tier(&q, &p.title, &[]);
                let score = score(t, in_title, 0, None, p.title.len(), now);
                Scored { item: p, score }
            })
            .collect(),
        n_for(SearchFilter::Playlists, ALL_SHELF),
    );
    if let Some(f) = playlists.first() {
        consider(f.score, SearchHit { playlist: Some(f.item.clone()), ..hit(SearchKind::Playlist) });
    }
    out.artists = artists.into_iter().map(|s| s.item).collect();
    out.albums = albums.into_iter().map(|s| s.item).collect();
    out.songs = songs.into_iter().map(|s| s.item).collect();
    out.playlists = playlists.into_iter().map(|s| s.item).collect();
    if filter == SearchFilter::All {
        out.top = top.map(|(_, h)| h);
    }
    out.took_ms = start.elapsed().as_secs_f64() * 1000.0;
    out
}

/// Search the local index, or the server until the first library sync has
/// finished (the cache may be missing most of the library until then). Any server failure (offline, slow, error) falls back to the
/// cache, so search always answers.
pub async fn search_with_fallback(
    db: &DbHandle,
    query: &str,
    filter: SearchFilter,
    limit: Option<u32>,
) -> AppResult<SearchResults> {
    let local = |db: DbHandle, query: String| async move {
        tokio::task::spawn_blocking(move || search(&db, &query, filter, limit))
            .await
            .map_err(|e| crate::error::AppError::Internal(e.to_string()))?
    };
    let has_terms = parse_query(query, |_| Vec::new()).fts.is_some();
    let remote = db.remote_search().filter(|_| has_terms);
    let Some(remote) = remote else {
        return local(db.clone(), query.to_string()).await;
    };
    let sync_state = db.read_async(super::store::sync_state).await?;
    if sync_state.as_ref().is_some_and(|s| s.last_sync_at.is_some()) {
        return local(db.clone(), query.to_string()).await;
    }
    let started = Instant::now();
    let section = sync_state.map(|s| s.section_id);
    let clipped: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    match tokio::time::timeout(REMOTE_TIMEOUT, remote.search(&clipped, section.as_deref(), REMOTE_LIMIT)).await {
        Ok(Ok(hits)) => {
            let mut r = rank_remote(query, filter, limit, hits, now_s());
            r.took_ms = started.elapsed().as_secs_f64() * 1000.0;
            Ok(r)
        }
        Ok(Err(e)) => {
            tracing::debug!("server search failed, using cache: {}", e.kind());
            local(db.clone(), query.to_string()).await
        }
        Err(_) => {
            tracing::debug!("server search timed out, using cache");
            local(db.clone(), query.to_string()).await
        }
    }
}

// ---------------------------------------------------------------- completions

/// Query completions shown above the direct results while typing.
pub const COMPLETIONS_MAX: usize = 4;
/// Names read per kind before de-duplicating.
const COMPLETION_CANDIDATES: i64 = 24;

/// Whole artist, album and track names that start with the typed words,
/// most played first, de-duplicated case- and diacritic-insensitively.
/// Playlist names are left out: a completion runs a search, and a playlist
/// name rarely finds more than the playlist itself.
pub fn completions(c: &Connection, query: &str, limit: usize) -> AppResult<Vec<String>> {
    let q = parse_query(query, |_| Vec::new());
    if q.fts.is_none() || limit == 0 {
        return Ok(Vec::new());
    }
    // A phrase anchored at the start of the title, its last word a prefix.
    let fts = format!("title : ^{}*", quote(&q.tokens.join(" ")));
    let mut cands: Vec<(String, u32)> = Vec::new();
    for table in ["artists", "albums", "tracks"] {
        let sql = format!(
            "SELECT title, view_count FROM {table} WHERE pk IN (
                SELECT rowid FROM {table}_fts WHERE {table}_fts MATCH ?1
             ) ORDER BY view_count DESC LIMIT ?2"
        );
        let mut st = c.prepare_cached(&sql)?;
        let rows = st.query_map(params![fts, COMPLETION_CANDIDATES], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
        for r in rows {
            cands.push(r?);
        }
    }
    // Stable sort: on equal plays, shorter names first, then artists before
    // albums before tracks.
    cands.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.chars().count().cmp(&b.0.chars().count())));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (title, _) in cands {
        let norm = tokens(&normalize(&title)).join(" ");
        if !norm.starts_with(&q.norm) || !seen.insert(norm) {
            continue;
        }
        out.push(title);
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- recents

/// Items opened from search that are remembered.
pub const RECENTS_MAX: i64 = 20;

fn kind_str(k: SearchKind) -> &'static str {
    match k {
        SearchKind::Artist => "artist",
        SearchKind::Album => "album",
        SearchKind::Track => "track",
        SearchKind::Playlist => "playlist",
    }
}

/// Remember an item opened from search: newest first, one entry per item,
/// capped at [`RECENTS_MAX`].
pub fn recent_add(c: &Connection, kind: SearchKind, id: &str) -> AppResult<()> {
    if id.is_empty() {
        return Ok(());
    }
    // Delete and insert (not upsert) so a reopened item also gets a new
    // rowid, which breaks ties between opens in the same millisecond.
    recent_remove(c, kind, id)?;
    c.execute(
        "INSERT INTO search_recents (kind, id, opened_at) VALUES (?1, ?2, ?3)",
        params![kind_str(kind), id, now_ms()],
    )?;
    c.execute(
        "DELETE FROM search_recents WHERE rowid NOT IN (
            SELECT rowid FROM search_recents ORDER BY opened_at DESC, rowid DESC LIMIT ?1)",
        [RECENTS_MAX],
    )?;
    Ok(())
}

pub fn recent_remove(c: &Connection, kind: SearchKind, id: &str) -> AppResult<()> {
    c.execute("DELETE FROM search_recents WHERE kind = ?1 AND id = ?2", params![kind_str(kind), id])?;
    Ok(())
}

/// Remembered items, newest first. Items no longer in the library are
/// skipped (they come back if the item does).
pub fn recent_list(c: &Connection) -> AppResult<Vec<SearchHit>> {
    let mut st = c.prepare_cached("SELECT kind, id FROM search_recents ORDER BY opened_at DESC, rowid DESC")?;
    let keys = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let hit = |kind| SearchHit { kind, artist: None, album: None, track: None, playlist: None };
    let mut out = Vec::new();
    for (kind, id) in keys {
        let found = match kind.as_str() {
            "artist" => one(c, &format!("SELECT {ARTIST_COLS} FROM artists ar WHERE ar.id = ?1"), &id, row_artist)?
                .map(|a| SearchHit { artist: Some(a), ..hit(SearchKind::Artist) }),
            "album" => one(c, &format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"), &id, row_album)?
                .map(|a| SearchHit { album: Some(a), ..hit(SearchKind::Album) }),
            "track" => one(c, &format!("SELECT {TRACK_COLS} FROM tracks t WHERE t.id = ?1"), &id, row_track)?
                .map(|t| SearchHit { track: Some(t), ..hit(SearchKind::Track) }),
            "playlist" => one(c, &format!("SELECT {PLAYLIST_COLS} FROM playlists p WHERE p.id = ?1"), &id, row_playlist)?
                .map(|p| SearchHit { playlist: Some(p), ..hit(SearchKind::Playlist) }),
            _ => None,
        };
        out.extend(found);
    }
    Ok(out)
}

fn one<T>(
    c: &Connection,
    sql: &str,
    id: &str,
    row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> AppResult<Option<T>> {
    let mut st = c.prepare_cached(sql)?;
    let mut rows = st.query_map([id], row)?;
    Ok(rows.next().transpose()?)
}

pub fn recent_clear(c: &Connection) -> AppResult<()> {
    c.execute("DELETE FROM search_recents", [])?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn edit_distance_one() {
        assert!(within_one_edit(&chars("beatles"), &chars("beatls")));
        assert!(within_one_edit(&chars("beatles"), &chars("beatlles")));
        assert!(within_one_edit(&chars("beatles"), &chars("beatlez")));
        assert!(within_one_edit(&chars("radiohead"), &chars("raidohead")));
        assert!(within_one_edit(&chars("abc"), &chars("abc")));
        assert!(!within_one_edit(&chars("beatles"), &chars("btales")));
        assert!(!within_one_edit(&chars("abc"), &chars("abcde")));
    }

    #[test]
    fn normalizes_diacritics_and_case() {
        assert_eq!(normalize("Beyoncé"), "beyonce");
        assert_eq!(normalize("Sigur Rós"), "sigur ros");
        assert_eq!(tokens(&normalize("AC/DC — Back in Black!")), vec!["ac", "dc", "back", "in", "black"]);
    }

    #[test]
    fn parse_neutralizes_fts_syntax() {
        let none = |_: &str| Vec::new();
        for raw in ["\"", "*", "-", "NEAR(a b)", "a OR b", "title:x", "(((", "^a", "a + b", "\"unterminated", "NOT", "--"] {
            let q = parse_query(raw, none);
            if q.short {
                assert_eq!(q.fts.as_deref(), Some(format!("title : ^\"{}\"*", q.tokens[0]).as_str()));
            } else if let Some(f) = &q.fts {
                // Every token is quoted; only our own syntax remains.
                for g in f.split(' ') {
                    assert!(g.starts_with('"') && g.ends_with("\"*"), "{raw} → {f}");
                }
            }
        }
        assert_eq!(parse_query("", none).fts, None);
        assert_eq!(parse_query("   --- ", none).fts, None);
        let q = parse_query("Beyoncé halo", none);
        assert_eq!(q.fts.as_deref(), Some("\"beyonce\"* \"halo\"*"));
        let q = parse_query("beatls", |_| vec!["beatles".into()]);
        assert_eq!(q.fts.as_deref(), Some("(\"beatls\"* OR \"beatles\")"));
        let q = parse_query("Ré", none);
        assert!(q.short);
        assert_eq!(q.fts.as_deref(), Some("title : ^\"re\"*"));
        assert!(!parse_query("ab cd", none).short);
        let long = "a".repeat(10_000);
        let q = parse_query(&long, none);
        assert!(q.tokens[0].chars().count() <= MAX_TOKEN_CHARS);
    }

    #[test]
    fn score_tiers_dominate_boosts() {
        let now = 1_000_000_000;
        let fuzzy_hot = score(Tier::Fuzzy, true, 100_000, Some(now), 1, now);
        let prefix_cold = score(Tier::WordPrefix, false, 0, None, 200, now);
        assert!(prefix_cold > fuzzy_hot);
        let exact = score(Tier::Exact, true, 0, None, 50, now);
        let title_prefix_hot = score(Tier::TitlePrefix, true, 100_000, Some(now), 1, now);
        assert!(exact > title_prefix_hot);
        assert!(score(Tier::Exact, true, 50, None, 5, now) > score(Tier::Exact, true, 0, None, 5, now));
    }
}
