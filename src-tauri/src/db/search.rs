//! Local search.
//!
//! - FTS5 over artists, albums (+ album artist), tracks (+ album artist,
//!   album, track artist) and playlists; tokenizer `unicode61
//!   remove_diacritics 2`, so "beyonce" finds "Beyoncé".
//! - Words are read by [`searchtext`]: spelling variants and shorthand
//!   ("thru" / "through", "pt 2" / "part II"), apostrophes, hyphens and "$"
//!   fold the same way on both sides. Each query word becomes a group of
//!   its spellings; every group must match, in any order, as a prefix.
//! - The library's word list (from the FTS vocab tables, kept in memory)
//!   adds the forms the index splits differently: "kayq" also finds
//!   "Kay-Q", "kay q" also finds "Kayq".
//! - "and" and small words like "the" between other words are not
//!   required (names write them many ways or leave them out).
//! - Fallbacks, ranked below everything found before them:
//!   1. typos, when the words as typed find fewer than [`FUZZY_BELOW`]
//!      results and a word is nowhere in the library (or nothing was
//!      found): each word of 5+ letters also matches library words one
//!      edit away (wrong, missing, extra or swapped letter);
//!   2. when nothing matches every word: words the library has nowhere
//!      are left out, else one word at a time.
//! - Typo corrections are silent: the corrected query comes back in
//!   `corrected` for the search box to offer as a suggestion.
//! - User input never reaches FTS syntax: it is folded into word tokens and
//!   each one is double-quoted, so quotes, `*`, `-`, `NEAR`, `AND`/`OR`/
//!   `NOT`, column filters and parentheses are inert.
//! - Ranking: tier first (exact title, then title or title part prefix,
//!   phrase, every word in the title, words in other fields, typo, missing
//!   word), then boosts for plays, rating and recency, the query's own
//!   spelling over a variant ("tonite" puts "Tonite" above "Tonight"), a
//!   small kind bias for the top result (artist, album, track, playlist)
//!   and shorter titles first. The tiers where every word is in
//!   the title sit close together, so a much played song can pass a never
//!   played exact title; the others stay 100 apart, which plays never
//!   cross.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::{Connection, params};

use crate::error::AppResult;
use crate::model::{Album, Artist, Playlist, Track};

use super::DbHandle;
use super::popular;
use super::queries::now_s;
use super::searchtext::{self, canon, canonical, canonical_of, edits_within, forms_of, max_typos, words};
use super::store::{ALBUM_COLS, ARTIST_COLS, PLAYLIST_COLS, TRACK_COLS, row_album, row_artist, row_playlist, row_track};
use super::source::RemoteHits;
use super::types::*;

pub use searchtext::normalize;

const MAX_QUERY_CHARS: usize = 200;
const MAX_TOKENS: usize = 8;
const MAX_TOKEN_CHARS: usize = 40;
/// The typo pass runs when the words as typed find fewer results than this.
pub const FUZZY_BELOW: usize = 5;
const MAX_FUZZY_ALTS: usize = 4;
const MAX_SPLITS: usize = 3;
const TRACK_CANDIDATES: i64 = 300;
/// Distinct artist lines split into credited names per search.
const CREDITED_LINES: i64 = 40;
const OTHER_CANDIDATES: i64 = 120;
const ALL_SONGS: usize = 4;
const ALL_SHELF: usize = 10;
const FILTER_DEFAULT: usize = 50;
const VOCAB_RELOAD_MIN: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- text

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

/// Every word in the index, with how many items use it.
struct Vocab {
    generation: u64,
    loaded_at: Instant,
    /// Sorted, deduped terms (already folded by the tokenizer).
    sorted: Vec<String>,
    /// Items per term, parallel to `sorted`.
    docs: Vec<u32>,
    /// Each term reversed with its index in `sorted`, sorted: finds terms by
    /// how they end.
    reversed: Vec<(String, u32)>,
}

impl Vocab {
    fn load(c: &Connection, generation: u64) -> AppResult<Self> {
        let mut all: Vec<(String, u32)> = Vec::new();
        for t in ["artists_vocab", "albums_vocab", "tracks_vocab", "playlists_vocab"] {
            let mut st = c.prepare(&format!("SELECT term, doc FROM {t}"))?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
            for r in rows {
                all.push(r?);
            }
        }
        all.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut sorted: Vec<String> = Vec::with_capacity(all.len());
        let mut docs: Vec<u32> = Vec::with_capacity(all.len());
        for (t, d) in all {
            if sorted.last() == Some(&t) {
                let last = docs.last_mut().expect("parallel");
                *last = last.saturating_add(d);
            } else {
                sorted.push(t);
                docs.push(d);
            }
        }
        let mut reversed: Vec<(String, u32)> =
            sorted.iter().enumerate().map(|(i, t)| (t.chars().rev().collect(), i as u32)).collect();
        reversed.sort_unstable();
        Ok(Self { generation, loaded_at: Instant::now(), sorted, docs, reversed })
    }

    fn contains(&self, term: &str) -> bool {
        self.sorted.binary_search_by(|t| t.as_str().cmp(term)).is_ok()
    }

    fn has_prefix(&self, token: &str) -> bool {
        let i = self.sorted.partition_point(|t| t.as_str() < token);
        self.sorted.get(i).is_some_and(|t| t.starts_with(token))
    }

    /// Two library words that run together into `text` ("kayq" → "kay q";
    /// a "$" read as "s": "risky" → "ri ky"), as FTS phrases.
    fn splits(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        if chars.len() < 4 {
            return out;
        }
        for i in 1..chars.len() {
            let left: String = chars[..i].iter().collect();
            if !self.contains(&left) {
                continue;
            }
            let right: String = chars[i..].iter().collect();
            if self.contains(&right) {
                out.push(format!("{left} {right}"));
            }
            if chars[i] == 's' && i + 1 < chars.len() {
                let right: String = chars[i + 1..].iter().collect();
                if self.contains(&right) {
                    out.push(format!("{left} {right}"));
                }
            }
            if out.len() >= MAX_SPLITS {
                break;
            }
        }
        out
    }

    /// Library words one edit from `token` (wrong, missing, extra or
    /// swapped letter), most used first. A word one edit away keeps either
    /// the token's first half or its last half as typed, so the candidates
    /// are the terms starting with the one or ending with the other: two
    /// binary searches, not a pass over every word.
    fn fuzzy(&self, token: &str) -> Vec<String> {
        let q: Vec<char> = token.chars().collect();
        let l = q.len();
        if max_typos(l) == 0 {
            return Vec::new();
        }
        let a = (l - 1) / 2;
        let head: String = q[..a].iter().collect();
        let tail: String = q[a + 1..].iter().rev().collect();
        let mut cands: Vec<u32> = Vec::new();
        let from = self.sorted.partition_point(|t| t.as_str() < head.as_str());
        cands.extend(
            self.sorted[from..].iter().take_while(|t| t.starts_with(head.as_str())).enumerate().map(|(k, _)| (from + k) as u32),
        );
        let from = self.reversed.partition_point(|(t, _)| t.as_str() < tail.as_str());
        cands.extend(self.reversed[from..].iter().take_while(|(t, _)| t.starts_with(tail.as_str())).map(|(_, i)| *i));
        cands.sort_unstable();
        cands.dedup();
        let mut found: Vec<(u32, u32)> = Vec::new();
        for i in cands {
            let term = &self.sorted[i as usize];
            let n = term.chars().count();
            if n + 1 < l || n > l + 1 || term == token {
                continue;
            }
            let c: Vec<char> = term.chars().collect();
            if within_one_edit(&q, &c) {
                found.push((self.docs[i as usize], i));
            }
        }
        found.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        found.truncate(MAX_FUZZY_ALTS);
        found.into_iter().map(|(_, i)| self.sorted[i as usize].clone()).collect()
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

/// One query word.
#[derive(Debug, Clone, PartialEq)]
pub struct QToken {
    /// Folded and joined ("don't" → "dont").
    pub text: String,
    /// The parts the index stores for it ("don", "t").
    pub pieces: Vec<String>,
    /// "and" (or "&", "+") or a small word like "the" between other words:
    /// names write them many ways or leave them out, so they are not
    /// required.
    pub optional: bool,
}

/// A parsed, sanitized query.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    /// Canonical query text (canonical words joined by single spaces).
    pub norm: String,
    /// The folded words as typed.
    pub tokens: Vec<String>,
    pub toks: Vec<QToken>,
    /// FTS5 MATCH expression for the words as typed and their spellings;
    /// None when there is nothing to search for.
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

/// Parse user input into words and a safe FTS5 expression.
pub fn parse_query(raw: &str) -> ParsedQuery {
    let clipped: String = raw.chars().take(MAX_QUERY_CHARS).collect();
    let mut toks: Vec<QToken> = words(&clipped)
        .into_iter()
        .take(MAX_TOKENS)
        .map(|w| QToken {
            text: w.text.chars().take(MAX_TOKEN_CHARS).collect(),
            pieces: w.pieces.into_iter().map(|p| p.chars().take(MAX_TOKEN_CHARS).collect()).collect(),
            optional: false,
        })
        .collect();
    let n = toks.len();
    for (i, t) in toks.iter_mut().enumerate() {
        // "and" typed as a word may be the start of "android" while it is
        // the last word; a symbol never is.
        let small = canon(&t.text) == "and" || searchtext::STOPWORDS.contains(&t.text.as_str());
        t.optional = small && (t.pieces.is_empty() || i + 1 < n);
    }
    let tokens: Vec<String> = toks.iter().map(|t| t.text.clone()).collect();
    let norm = tokens.iter().map(|t| canon(t)).collect::<Vec<_>>().join(" ");
    if toks.is_empty() {
        return ParsedQuery { norm, tokens, toks, fts: None, short: false };
    }
    if toks.len() == 1 && toks[0].text.chars().count() <= SHORT_MAX_CHARS && !toks[0].pieces.is_empty() {
        let fts = format!("title : ^{}*", quote(&toks[0].pieces.join(" ")));
        return ParsedQuery { norm, tokens, toks, fts: Some(fts), short: true };
    }
    let mut q = ParsedQuery { norm, tokens, toks, fts: None, short: false };
    q.fts = build_fts(&q, &Extra::default());
    q
}

/// What the library's word list adds to a query.
#[derive(Debug, Clone, Default)]
pub struct Extra {
    /// More spellings per word (FTS phrases, matched whole).
    pub alts: Vec<Vec<String>>,
    /// `joins[i]`: words i and i+1 also match run together as one word.
    pub joins: Vec<bool>,
    /// Words left out (the missing-word fallback).
    pub skip: Vec<usize>,
}

/// One word's FTS group: as typed (prefix), its parts as a phrase, its
/// spellings and any extra forms.
fn group_expr(t: &QToken, extra: &[String]) -> String {
    let mut parts = vec![format!("{}*", quote(&t.text))];
    if !t.pieces.is_empty() && t.pieces != [t.text.clone()] {
        parts.push(format!("{}*", quote(&t.pieces.join(" "))));
    }
    parts.extend(searchtext::variants(&t.text).map(quote));
    parts.extend(extra.iter().map(|e| quote(e)));
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| seen.insert(p.clone()));
    if parts.len() == 1 { parts.remove(0) } else { format!("({})", parts.join(" OR ")) }
}

/// The FTS5 expression for a parsed query plus extra forms.
pub fn build_fts(q: &ParsedQuery, extra: &Extra) -> Option<String> {
    let n = q.toks.len();
    let kept = |i: &usize| !extra.skip.contains(i);
    let mut idx: Vec<usize> = (0..n).filter(kept).filter(|i| !q.toks[*i].optional).collect();
    if idx.is_empty() {
        idx = (0..n).filter(kept).filter(|i| !q.toks[*i].pieces.is_empty()).collect();
    }
    if idx.is_empty() {
        return None;
    }
    let none: Vec<String> = Vec::new();
    let g = |i: usize| group_expr(&q.toks[i], extra.alts.get(i).unwrap_or(&none));
    let mut parts = Vec::new();
    let mut k = 0;
    while k < idx.len() {
        let i = idx[k];
        if k + 1 < idx.len() && idx[k + 1] == i + 1 && extra.joins.get(i) == Some(&true) {
            let joined = format!("{}{}", q.toks[i].text, q.toks[i + 1].text);
            parts.push(format!("(({} AND {}) OR {}*)", g(i), g(i + 1), quote(&joined)));
            k += 2;
        } else {
            parts.push(g(i));
            k += 1;
        }
    }
    // Explicit AND: FTS5 only joins bare phrases implicitly.
    Some(parts.join(" AND "))
}

/// Every word but one, for each word that can be left out.
fn drop_one_fts(q: &ParsedQuery, extra: &Extra) -> Option<String> {
    let required: Vec<usize> = (0..q.toks.len()).filter(|i| !q.toks[*i].optional).collect();
    if required.len() < 2 {
        return None;
    }
    let ors: Vec<String> = required
        .iter()
        .filter_map(|&i| build_fts(q, &Extra { skip: vec![i], ..extra.clone() }))
        .map(|e| format!("({e})"))
        .collect();
    Some(ors.join(" OR "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Found with a word left out.
    Partial = 0,
    /// Found through a typo.
    Fuzzy = 1,
    /// Every word starts a word somewhere in the name or its fields.
    WordPrefix = 2,
    /// The words in a row inside the title.
    Phrase = 3,
    /// The title, or one part of a title like "A / B", starts with the query.
    TitlePrefix = 4,
    Exact = 5,
}

/// Separators that split a title into parts ("Side A / Side B", "Song -
/// Live", "One + Two").
const PART_SEPARATORS: &[&str] = &[" / ", "/", " - ", " – ", " — ", " + ", " | ", "; "];

fn title_parts(title: &str) -> Vec<String> {
    let mut parts = vec![title.to_string()];
    for sep in PART_SEPARATORS {
        parts = parts.iter().flat_map(|p| p.split(sep).map(str::to_string).collect::<Vec<_>>()).collect();
    }
    parts.retain(|p| !p.trim().is_empty());
    parts
}

/// True when the query's canonical words appear in a row in `words`, the
/// last one as a prefix.
fn has_phrase(words: &[&str], q: &[&str]) -> bool {
    let n = q.len();
    if n == 0 || words.len() < n {
        return false;
    }
    words.windows(n).any(|w| w[..n - 1] == q[..n - 1] && w[n - 1].starts_with(q[n - 1]))
}

/// How each query word matched one item.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Hit {
    Missing,
    Typo,
    Word,
}

fn word_hits(q: &ParsedQuery, forms: &[String], typos: bool) -> Vec<Hit> {
    let n = q.toks.len();
    let mut hits = vec![Hit::Missing; n];
    for (i, t) in q.toks.iter().enumerate() {
        if t.optional {
            hits[i] = Hit::Word;
            continue;
        }
        let c = canon(&t.text);
        if forms.iter().any(|f| f.starts_with(c) || f.starts_with(t.text.as_str())) {
            hits[i] = Hit::Word;
        }
    }
    // "kay q" against "Kayq": two words run together.
    for i in 0..n.saturating_sub(1) {
        if hits[i] == Hit::Word && hits[i + 1] == Hit::Word {
            continue;
        }
        let joined = format!("{}{}", q.toks[i].text, q.toks[i + 1].text);
        if forms.iter().any(|f| f.starts_with(&joined)) {
            hits[i] = Hit::Word;
            hits[i + 1] = Hit::Word;
        }
    }
    if typos {
        for (i, t) in q.toks.iter().enumerate() {
            if hits[i] != Hit::Missing {
                continue;
            }
            let qc: Vec<char> = t.text.chars().collect();
            let max = max_typos(qc.len());
            if max > 0 && forms.iter().any(|f| edits_within(&qc, &f.chars().collect::<Vec<_>>(), max).is_some()) {
                hits[i] = Hit::Typo;
            }
        }
    }
    hits
}

/// Match tier of a result given its title and other searchable fields, and
/// whether the title alone holds the match.
pub fn tier(q: &ParsedQuery, title: &str, others: &[Option<&str>]) -> (Tier, bool) {
    let (t, in_title, _) = rate(q, title, others);
    (t, in_title)
}

/// True when a title may have parts ("A / B", "A - Live").
fn may_have_parts(title: &str) -> bool {
    title.contains(['/', '-', '\u{2013}', '\u{2014}', '+', '|', ';'])
}

fn judge(q: &ParsedQuery, title: &str, tw: &[searchtext::Word], others: &[Vec<searchtext::Word>]) -> (Tier, bool) {
    let t = canonical_of(tw);
    if t == q.norm {
        return (Tier::Exact, true);
    }
    if !q.norm.is_empty() && t.starts_with(&q.norm) {
        return (Tier::TitlePrefix, true);
    }
    if may_have_parts(title) {
        let parts = title_parts(title);
        if parts.len() > 1 && parts.iter().any(|p| canonical(p).starts_with(&q.norm)) {
            return (Tier::TitlePrefix, true);
        }
    }
    let t_words: Vec<&str> = t.split(' ').collect();
    let q_words: Vec<&str> = q.norm.split(' ').collect();
    if q_words.len() > 1 && has_phrase(&t_words, &q_words) {
        return (Tier::Phrase, true);
    }
    let title_forms = forms_of(tw);
    if word_hits(q, &title_forms, false).iter().all(|h| *h == Hit::Word) {
        return (Tier::WordPrefix, true);
    }
    let mut all_forms = title_forms.clone();
    for o in others {
        all_forms.extend(forms_of(o));
    }
    let hits = word_hits(q, &all_forms, true);
    let tier = match hits.iter().copied().min().unwrap_or(Hit::Word) {
        Hit::Word => return (Tier::WordPrefix, false),
        Hit::Typo => Tier::Fuzzy,
        Hit::Missing => Tier::Partial,
    };
    let title_holds = word_hits(q, &title_forms, true).iter().all(|h| *h != Hit::Missing);
    (tier, title_holds)
}

/// Score of each tier, before boosts. Where every word is in the title the
/// tiers sit close, so plays can carry a song past a never played exact
/// title; elsewhere they stay 100 apart and boosts (under 60) never cross.
fn tier_base(tier: Tier, in_title: bool) -> f64 {
    let title = if in_title { 5.0 } else { 0.0 };
    match tier {
        Tier::Exact => 355.0,
        Tier::TitlePrefix => 330.0,
        Tier::Phrase => 320.0,
        Tier::WordPrefix if in_title => 310.0,
        Tier::WordPrefix => 200.0,
        Tier::Fuzzy => 100.0 + title,
        Tier::Partial => title,
    }
}

/// Final score: the tier's base plus boosts for plays, recency and short
/// titles.
pub fn score(tier: Tier, in_title: bool, plays: u32, last_viewed: Option<i64>, title_len: usize, now: i64) -> f64 {
    let plays_boost = (6.0 * (1.0 + plays as f64).ln()).min(25.0);
    let recency = match last_viewed {
        Some(t) => {
            let days = ((now - t).max(0) as f64) / 86_400.0;
            if days <= 7.0 { 10.0 } else { (10.0 * (1.0 - (days - 7.0) / 83.0)).max(0.0) }
        }
        None => 0.0,
    };
    let len_penalty = (title_len as f64 / 20.0).min(4.0);
    tier_base(tier, in_title) + plays_boost + recency - len_penalty
}

/// Boost for a name holding every query word as typed, not only a variant
/// of it: "tonite" ranks "Tonite" above "Tonight".
const SPELLED_BONUS: f64 = 15.0;

/// Up to 5 for a rated song (Plex ratings run 0 to 10).
fn rating_boost(r: Option<f32>) -> f64 {
    f64::from(r.unwrap_or(0.0).clamp(0.0, 10.0)) / 2.0
}

/// Tier, whether the title holds the match, and the spelling bonus. Each
/// name is split into words once.
fn rate(q: &ParsedQuery, title: &str, others: &[Option<&str>]) -> (Tier, bool, f64) {
    let tw = words(title);
    let ow: Vec<Vec<searchtext::Word>> = others.iter().flatten().map(|o| words(o)).collect();
    let (t, in_title) = judge(q, title, &tw, &ow);
    if t < Tier::WordPrefix {
        return (t, in_title, 0.0);
    }
    let spelled = q.toks.iter().filter(|t| !t.optional).all(|t| {
        tw.iter().chain(ow.iter().flatten()).any(|w| {
            w.text.starts_with(t.text.as_str()) || w.pieces.iter().any(|p| p.starts_with(t.text.as_str()))
        })
    });
    (t, in_title, if spelled { SPELLED_BONUS } else { 0.0 })
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

/// Add a later pass's results: one row per item (its best score), best `n`.
fn merge<T>(into: &mut Vec<Scored<T>>, more: Vec<Scored<T>>, id: fn(&T) -> &str, n: usize) {
    for s in more {
        match into.iter_mut().find(|x| id(&x.item) == id(&s.item)) {
            Some(x) => x.score = x.score.max(s.score),
            None => into.push(s),
        }
    }
    let v = std::mem::take(into);
    *into = rank(v, n);
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

fn search_artists(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize, div: i64) -> AppResult<Vec<Scored<Artist>>> {
    let sql = candidates_sql(ARTIST_COLS, "artists", "ar", "rank", "ar.view_count DESC", q.short);
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, OTHER_CANDIDATES / div], row_artist)?;
    let mut v = Vec::new();
    for a in rows {
        let a = a?;
        let (t, in_title, bonus) = rate(q, &a.title, &[]);
        let s = score(t, in_title, a.view_count, None, a.title.len(), now) + bonus + 3.0;
        v.push(Scored { item: a, score: s });
    }
    Ok(rank(v, n))
}

/// Credited artists (names in track artist lines with no library artist)
/// whose name matches the query, with the plays of the tracks crediting them.
fn search_credited(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize, div: i64) -> AppResult<Vec<Scored<Artist>>> {
    if q.short || n == 0 {
        return Ok(Vec::new());
    }
    // Splitting a line into names checks it against every artist, so only
    // the most played distinct lines are split, in library order (a name is
    // written as on the first track that credits it).
    let mut st = c.prepare_cached(
        "SELECT line, plays FROM (
            SELECT t.track_artist AS line, sum(t.view_count) AS plays, min(t.pk) AS first FROM (
                SELECT rowid AS r FROM tracks_fts WHERE tracks_fts MATCH ?1 ORDER BY rank LIMIT ?2
            ) f JOIN tracks t ON t.pk = f.r WHERE t.track_artist IS NOT NULL
            GROUP BY t.track_artist ORDER BY plays DESC LIMIT ?3
         ) ORDER BY first",
    )?;
    let rows: Vec<(Option<String>, u32)> = st
        .query_map(params![format!("track_artist : ({fts})"), TRACK_CANDIDATES / div, CREDITED_LINES / div], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
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
            if !in_title || t == Tier::Partial || in_library.exists([&name])? {
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

fn search_albums(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize, div: i64) -> AppResult<Vec<Scored<Album>>> {
    let sql = candidates_sql(
        ALBUM_COLS,
        "albums",
        "al",
        "bm25(albums_fts, 10.0, 3.0)",
        "al.view_count DESC, al.last_viewed_at DESC",
        q.short,
    );
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, OTHER_CANDIDATES / div], row_album)?;
    let mut v = Vec::new();
    for a in rows {
        let a = a?;
        let (t, in_title, bonus) = rate(q, &a.title, &[Some(&a.artist_title)]);
        let s = score(t, in_title, a.view_count, a.last_viewed_at, a.title.len(), now) + bonus + 2.0;
        v.push(Scored { item: a, score: s });
    }
    Ok(rank(v, n))
}

fn search_tracks(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize, div: i64) -> AppResult<Vec<Scored<Track>>> {
    let sql = candidates_sql(
        TRACK_COLS,
        "tracks",
        "t",
        "bm25(tracks_fts, 10.0, 3.0, 2.0, 3.0)",
        "t.view_count DESC, t.last_viewed_at DESC",
        q.short,
    );
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, TRACK_CANDIDATES / div], row_track)?;
    let mut v = Vec::new();
    for tr in rows {
        let tr = tr?;
        let (t, in_title, bonus) = rate(
            q,
            &tr.title,
            &[Some(&tr.artist_title), Some(&tr.album_title), tr.track_artist.as_deref()],
        );
        let s = score(t, in_title, tr.view_count, tr.last_viewed_at, tr.title.len(), now) + bonus + rating_boost(tr.user_rating) + 1.0;
        v.push(Scored { item: tr, score: s });
    }
    Ok(rank(v, n))
}

fn search_playlists(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, n: usize, div: i64) -> AppResult<Vec<Scored<Playlist>>> {
    let sql = candidates_sql(PLAYLIST_COLS, "playlists", "p", "rank", "p.title COLLATE NOCASE", q.short);
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![fts, OTHER_CANDIDATES / div], row_playlist)?;
    let mut v = Vec::new();
    for p in rows {
        let p = p?;
        let (t, in_title, bonus) = rate(q, &p.title, &[]);
        let s = score(t, in_title, 0, None, p.title.len(), now) + bonus;
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

/// One pass's results per kind.
#[derive(Default)]
struct Found {
    artists: Vec<Scored<Artist>>,
    albums: Vec<Scored<Album>>,
    songs: Vec<Scored<Track>>,
    playlists: Vec<Scored<Playlist>>,
}

impl Found {
    fn count(&self) -> usize {
        self.artists.len() + self.albums.len() + self.songs.len() + self.playlists.len()
    }
}

/// Per-kind result counts for one search.
#[derive(Clone, Copy)]
struct Limits {
    artists: usize,
    albums: usize,
    songs: usize,
    playlists: usize,
    credited: bool,
    /// Candidates read per kind, as a share of the usual pool (1 or 3).
    pool_div: i64,
}

impl Limits {
    /// The fallback passes rank below everything before them, so they read
    /// a third of the candidates.
    fn fallback(self) -> Self {
        Self { pool_div: 3, ..self }
    }
}

fn run(c: &Connection, q: &ParsedQuery, fts: &str, now: i64, l: Limits) -> AppResult<Found> {
    let mut f = Found::default();
    if l.artists > 0 {
        let mut v = search_artists(c, q, fts, now, l.artists, l.pool_div)?;
        if l.credited {
            v.extend(search_credited(c, q, fts, now, l.artists, l.pool_div)?);
            v = rank(v, l.artists);
        }
        f.artists = v;
    }
    if l.albums > 0 {
        f.albums = search_albums(c, q, fts, now, l.albums, l.pool_div)?;
    }
    if l.songs > 0 {
        f.songs = search_tracks(c, q, fts, now, l.songs, l.pool_div)?;
    }
    if l.playlists > 0 {
        f.playlists = search_playlists(c, q, fts, now, l.playlists, l.pool_div)?;
    }
    Ok(f)
}

fn merge_found(into: &mut Found, more: Found, l: Limits) {
    merge(&mut into.artists, more.artists, |a| a.id.as_str(), l.artists);
    merge(&mut into.albums, more.albums, |a| a.id.as_str(), l.albums);
    merge(&mut into.songs, more.songs, |t| t.id.as_str(), l.songs);
    merge(&mut into.playlists, more.playlists, |p| p.id.as_str(), l.playlists);
}

/// True when word `i` can match something in the library: as typed, in a
/// spelling, or through `extra` (split forms; typos once that pass ran).
fn in_library(q: &ParsedQuery, i: usize, v: &Vocab, extra: &Extra) -> bool {
    let t = &q.toks[i];
    v.has_prefix(&t.text)
        || searchtext::variants(&t.text).any(|s| v.contains(s))
        || (t.pieces.len() > 1 && t.pieces.last().is_some_and(|p| v.has_prefix(p)))
        || extra.alts.get(i).is_some_and(|a| !a.is_empty())
}

/// The query with each word that matches nothing in the library replaced
/// by its closest library word; None when no word changed.
fn correction(q: &ParsedQuery, v: &Vocab, fuzzy: &[Vec<String>], extra: &Extra) -> Option<String> {
    let mut changed = false;
    let words: Vec<String> = q
        .toks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            match fuzzy.get(i).and_then(|f| f.first()) {
                Some(best) if !t.optional && !in_library(q, i, v, extra) => {
                    changed = true;
                    best.clone()
                }
                _ => t.text.clone(),
            }
        })
        .collect();
    changed.then(|| words.join(" "))
}

/// Run a search on a read connection. `generation` keys the vocab cache;
/// `typos` allows the typo fallback.
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
    typos: bool,
) -> AppResult<SearchResults> {
    let start = Instant::now();
    let mut out = SearchResults::default();
    let q = parse_query(query);
    let Some(typed) = q.fts.clone() else {
        out.took_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Ok(out);
    };
    let l = Limits {
        artists: kind_limit(filter, limit, SearchFilter::Artists, ALL_SHELF),
        albums: kind_limit(filter, limit, SearchFilter::Albums, ALL_SHELF),
        songs: kind_limit(filter, limit, SearchFilter::Songs, ALL_SONGS),
        playlists: kind_limit(filter, limit, SearchFilter::Playlists, ALL_SHELF),
        credited,
        pool_div: 1,
    };

    let vocab = if q.short {
        None
    } else {
        vocab.get(c, generation).map_err(|e| tracing::warn!("search vocab: {e}")).ok()
    };
    // Pass 1: the words as typed, their spellings, and the forms the index
    // splits or joins differently.
    let mut extra = Extra::default();
    if let Some(v) = &vocab {
        extra.alts = q.toks.iter().map(|t| if t.pieces.len() > 1 { Vec::new() } else { v.splits(&t.text) }).collect();
        extra.joins = q.toks.windows(2).map(|w| v.has_prefix(&format!("{}{}", w[0].text, w[1].text))).collect();
    }
    let fts = if q.short { typed } else { build_fts(&q, &extra).unwrap_or(typed) };
    let mut found = run(c, &q, &fts, now, l)?;

    // Pass 2: typos, only when the words as typed found little and one of
    // them is nowhere in the library (or nothing at all was found). A query
    // of real library words is not "corrected", and it keeps typing fast.
    let unknown_word = |v: &Vocab| (0..q.toks.len()).any(|i| !q.toks[i].optional && !in_library(&q, i, v, &extra));
    let want_typos = |v: &Vocab| found.count() == 0 || unknown_word(v);
    if let (true, Some(v)) = (typos && found.count() < FUZZY_BELOW && !q.short, &vocab)
        && want_typos(v)
    {
        let fuzzy: Vec<Vec<String>> = q
            .toks
            .iter()
            .map(|t| if t.optional { Vec::new() } else { v.fuzzy(&t.text) })
            .collect();
        if fuzzy.iter().any(|f| !f.is_empty()) {
            let mut wide = extra.clone();
            for (i, f) in fuzzy.iter().enumerate() {
                if let Some(a) = wide.alts.get_mut(i) {
                    a.extend(f.iter().cloned());
                }
            }
            let before = found.count();
            if let Some(e) = build_fts(&q, &wide) {
                let more = run(c, &q, &e, now, l.fallback())?;
                merge_found(&mut found, more, l);
            }
            if found.count() > before {
                out.corrected = correction(&q, v, &fuzzy, &extra);
            }
            extra = wide;
        }
    }

    // Pass 3, when nothing matched every word: leave out the words the
    // library has nowhere (in any spelling); failing that, one word at a time.
    if found.count() == 0 && !q.short {
        let unknown: Vec<usize> = match &vocab {
            Some(v) => (0..q.toks.len()).filter(|&i| !q.toks[i].optional && !in_library(&q, i, v, &extra)).collect(),
            None => Vec::new(),
        };
        let some_left = q.toks.iter().enumerate().any(|(i, t)| !t.optional && !unknown.contains(&i));
        let e = if !unknown.is_empty() && some_left {
            build_fts(&q, &Extra { skip: unknown, ..extra.clone() })
        } else {
            drop_one_fts(&q, &extra)
        };
        if let Some(e) = e {
            let more = run(c, &q, &e, now, l.fallback())?;
            merge_found(&mut found, more, l);
        }
    }

    let mut top: Option<(f64, SearchHit)> = None;
    let mut consider = |score: f64, hit: SearchHit| {
        if top.as_ref().is_none_or(|(s, _)| score > *s) {
            top = Some((score, hit));
        }
    };
    let hit = |kind| SearchHit { kind, artist: None, album: None, track: None, playlist: None };
    if let Some(f) = found.artists.first() {
        consider(f.score, SearchHit { artist: Some(f.item.clone()), ..hit(SearchKind::Artist) });
    }
    if let Some(f) = found.albums.first() {
        consider(f.score, SearchHit { album: Some(f.item.clone()), ..hit(SearchKind::Album) });
    }
    if let Some(f) = found.songs.first() {
        consider(f.score, SearchHit { track: Some(f.item.clone()), ..hit(SearchKind::Track) });
    }
    if let Some(f) = found.playlists.first() {
        consider(f.score, SearchHit { playlist: Some(f.item.clone()), ..hit(SearchKind::Playlist) });
    }
    out.artists = found.artists.into_iter().map(|s| s.item).collect();
    out.albums = found.albums.into_iter().map(|s| s.item).collect();
    out.songs = found.songs.into_iter().map(|s| s.item).collect();
    out.playlists = found.playlists.into_iter().map(|s| s.item).collect();
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
    let typos = db.search_typos();
    db.read(|c| search_conn(c, &db.inner.vocab, generation, query, filter, limit, now_s(), credited, typos))
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
    let q = parse_query(query);
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
                let (t, in_title, bonus) = rate(&q, &a.title, &[]);
                let score = score(t, in_title, a.view_count, None, a.title.len(), now) + bonus + 3.0;
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
                let (t, in_title, bonus) = rate(&q, &a.title, &[Some(&a.artist_title)]);
                let score = score(t, in_title, a.view_count, a.last_viewed_at, a.title.len(), now) + bonus + 2.0;
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
                let (t, in_title, bonus) = rate(
                    &q,
                    &tr.title,
                    &[Some(&tr.artist_title), Some(&tr.album_title), tr.track_artist.as_deref()],
                );
                let score = score(t, in_title, tr.view_count, tr.last_viewed_at, tr.title.len(), now) + bonus + rating_boost(tr.user_rating) + 1.0;
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
                let (t, in_title, bonus) = rate(&q, &p.title, &[]);
                let score = score(t, in_title, 0, None, p.title.len(), now) + bonus;
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
    let has_terms = parse_query(query).fts.is_some();
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
    let q = parse_query(query);
    let pieces: Vec<&str> = q.toks.iter().flat_map(|t| t.pieces.iter().map(String::as_str)).collect();
    if q.fts.is_none() || pieces.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    // A phrase anchored at the start of the title, its last word a prefix.
    let fts = format!("title : ^{}*", quote(&pieces.join(" ")));
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
        let norm = canonical(&title);
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
        assert_eq!(tokens(&normalize("XY/ZW, Back in Blue!")), vec!["xy", "zw", "back", "in", "blue"]);
    }

    #[test]
    fn parse_neutralizes_fts_syntax() {
        // Outside the quotes only our own syntax is left: parentheses, `*`
        // and OR; inside, only folded word characters.
        fn safe(f: &str) -> bool {
            let mut outside = String::new();
            for (i, part) in f.split('"').enumerate() {
                if i % 2 == 1 {
                    if !part.chars().all(|c| c.is_alphanumeric() || c == ' ') {
                        return false;
                    }
                } else {
                    outside.push_str(part);
                }
            }
            outside.replace("OR", "").replace("AND", "").chars().all(|c| matches!(c, '(' | ')' | '*' | ' '))
        }
        for raw in ["\"", "*", "-", "NEAR(a b)", "a OR b", "title:x", "(((", "^a", "a + b", "\"unterminated", "NOT", "--", "x & y"] {
            let q = parse_query(raw);
            if q.short {
                assert_eq!(q.fts.as_deref(), Some(format!("title : ^\"{}\"*", q.tokens[0]).as_str()));
            } else if let Some(f) = &q.fts {
                assert!(safe(f), "{raw} → {f}");
            }
        }
        assert_eq!(parse_query("").fts, None);
        assert_eq!(parse_query("   --- ").fts, None);
        assert_eq!(parse_query(" & ").fts, None);
        let q = parse_query("Beyoncé halo");
        assert_eq!(q.fts.as_deref(), Some("\"beyonce\"* AND \"halo\"*"));
        let q = parse_query("thru the nite");
        assert_eq!(q.fts.as_deref(), Some("(\"thru\"* OR \"through\") AND (\"nite\"* OR \"night\")"));
        assert_eq!(q.norm, "through the night");
        let q = parse_query("don't");
        assert_eq!(q.fts.as_deref(), Some("(\"dont\"* OR \"don t\"*)"));
        // "and" (or "&") between words is not required; as the last word it
        // may be the start of another word.
        let q = parse_query("salt & pepper");
        assert_eq!(q.fts.as_deref(), Some("\"salt\"* AND \"pepper\"*"));
        assert!(parse_query("rock and").fts.unwrap().contains("\"and\"*"));
        let q = parse_query("beatls");
        let e = Extra { alts: vec![vec!["beatles".into()]], ..Default::default() };
        assert_eq!(build_fts(&q, &e).as_deref(), Some("(\"beatls\"* OR \"beatles\")"));
        let q = parse_query("kay q");
        let e = Extra { joins: vec![true], ..Default::default() };
        assert_eq!(build_fts(&q, &e).as_deref(), Some("((\"kay\"* AND \"q\"*) OR \"kayq\"*)"));
        let q = parse_query("red sky blue");
        assert_eq!(drop_one_fts(&q, &Extra::default()).as_deref(), Some("(\"sky\"* AND \"blue\"*) OR (\"red\"* AND \"blue\"*) OR (\"red\"* AND \"sky\"*)"));
        // Small words between others are not required.
        assert_eq!(parse_query("the sky of blue").fts.as_deref(), Some("\"sky\"* AND \"blue\"*"));
        let q = parse_query("Ré");
        assert!(q.short);
        assert_eq!(q.fts.as_deref(), Some("title : ^\"re\"*"));
        assert!(!parse_query("ab cd").short);
        let long = "a".repeat(10_000);
        let q = parse_query(&long);
        assert!(q.tokens[0].chars().count() <= MAX_TOKEN_CHARS);
    }

    #[test]
    fn tiers_follow_spellings_parts_and_phrases() {
        let q = parse_query("thru the high");
        assert_eq!(tier(&q, "Through the High Grass", &[]).0, Tier::TitlePrefix);
        assert_eq!(tier(&q, "Through The High", &[]).0, Tier::Exact);
        let q = parse_query("this one here");
        assert_eq!(tier(&q, "And They Came Back / This One Here", &[]).0, Tier::TitlePrefix);
        assert_eq!(tier(&q, "Was This the One Here", &[]).0, Tier::WordPrefix);
        assert_eq!(tier(&q, "Oh This One Here Again", &[]).0, Tier::Phrase);
        let q = parse_query("hold lemonaed");
        assert_eq!(tier(&q, "Hold Lemonade", &[]).0, Tier::Fuzzy);
        let q = parse_query("hold nothing");
        assert_eq!(tier(&q, "Hold Up", &[]).0, Tier::Partial);
        let q = parse_query("kay q");
        assert_eq!(tier(&q, "Kayq", &[]).0, Tier::WordPrefix);
        let q = parse_query("risky");
        assert_eq!(tier(&q, "Ri$ky", &[]).0, Tier::Exact);
        assert!(Tier::Exact > Tier::TitlePrefix && Tier::TitlePrefix > Tier::Phrase && Tier::Phrase > Tier::WordPrefix);
        assert!(Tier::WordPrefix > Tier::Fuzzy && Tier::Fuzzy > Tier::Partial);
    }

    #[test]
    fn score_tiers_dominate_boosts() {
        let now = 1_000_000_000;
        let fuzzy_hot = score(Tier::Fuzzy, true, 100_000, Some(now), 1, now);
        let prefix_cold = score(Tier::WordPrefix, false, 0, None, 200, now);
        assert!(prefix_cold > fuzzy_hot);
        let exact = score(Tier::Exact, true, 0, None, 50, now);
        let title_prefix_warm = score(Tier::TitlePrefix, true, 40, None, 50, now);
        assert!(exact > title_prefix_warm);
        // Where every word is in the title, a much played song can pass a
        // never played exact title.
        let title_prefix_hot = score(Tier::TitlePrefix, true, 100_000, Some(now), 50, now);
        assert!(title_prefix_hot > exact);
        let elsewhere_hot = score(Tier::WordPrefix, false, 100_000, Some(now), 1, now) + SPELLED_BONUS + 5.0 + 3.0;
        assert!(score(Tier::WordPrefix, true, 0, None, 200, now) > elsewhere_hot);
        assert!(score(Tier::Exact, true, 50, None, 5, now) > score(Tier::Exact, true, 0, None, 5, now));
    }
}
