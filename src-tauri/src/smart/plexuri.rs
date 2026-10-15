//! Rules to and from Plex's smart playlist filter language.
//!
//! A Plex smart playlist is a library query: `/library/sections/{id}/all?
//! type=10&<field><op>=<value>&...` (docs/PLEX-API.md "Smart playlists").
//! The operator sits in the key, percent-encoded: `track.userRating%3E%3E=4`
//! is "rating greater than 4" (Plex counts half stars, 0 to 10). Tag fields
//! (genre, style, mood) take Plex tag ids, so names are looked up first.
//! "Match any" wraps the conditions in `push=1 ... or=1 ... pop=1`.
//! Plex reports a smart playlist's query as `content`:
//! `library://x/directory/<the path, percent-encoded>`.

use std::collections::HashMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use super::rules::{Field, Limit, LimitUnit, Op, Rule, SmartRules, SortBy, Update};
use crate::error::{AppError, AppResult};

/// Plex's library item type for tracks.
pub const TRACK_TYPE: &str = "10";
pub const LIBRARY_IDENTIFIER: &str = "com.plexapp.plugins.library";

/// Everything but unreserved characters.
const ENC: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, ENC).to_string()
}

fn dec(s: &str) -> String {
    percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned()
}

/// Tag ids and names per Plex field (`album.genre` -> [(id, name)]).
pub type TagIds = HashMap<String, Vec<(String, String)>>;

/// The Plex field a rule filters on (None: Plex can't).
pub fn plex_field(r: &Rule) -> Option<String> {
    if let Some(f) = &r.plex_field {
        return Some(f.clone());
    }
    Some(
        match r.field {
            Field::Genre => "album.genre",
            Field::Style => "album.style",
            Field::Mood => "album.mood",
            Field::Artist => "artist.title",
            Field::Album => "album.title",
            Field::Year => "album.year",
            Field::Decade => "album.decade",
            Field::AddedAt => "album.addedAt",
            Field::PlayCount => "track.viewCount",
            Field::LastPlayed => "track.lastViewedAt",
            Field::Rating | Field::Liked => "track.userRating",
            Field::PlayedHere | Field::Duration | Field::ReleaseType | Field::Explicit | Field::Version => return None,
        }
        .to_string(),
    )
}

/// Tag fields these rules need ids for.
pub fn tag_fields(rules: &SmartRules) -> Vec<String> {
    let mut out = Vec::new();
    for r in &rules.rules {
        if matches!(r.field, Field::Genre | Field::Style | Field::Mood)
            && let Some(f) = plex_field(r)
            && !out.contains(&f)
        {
            out.push(f);
        }
    }
    out
}

fn sort_key(s: SortBy) -> Option<&'static str> {
    Some(match s {
        SortBy::None => return None,
        SortBy::Random => "random",
        SortBy::MostPlayed => "viewCount:desc",
        SortBy::LeastPlayed => "viewCount",
        SortBy::RecentlyAdded => "addedAt:desc",
        SortBy::RecentlyPlayed => "lastViewedAt:desc",
        SortBy::Rating => "userRating:desc",
    })
}

fn sort_from(v: &str) -> Option<SortBy> {
    let v = v.strip_prefix("track.").unwrap_or(v);
    Some(match v {
        "random" => SortBy::Random,
        "viewCount:desc" => SortBy::MostPlayed,
        "viewCount" | "viewCount:asc" => SortBy::LeastPlayed,
        "addedAt:desc" => SortBy::RecentlyAdded,
        "lastViewedAt:desc" => SortBy::RecentlyPlayed,
        "userRating:desc" => SortBy::Rating,
        _ => return None,
    })
}

/// "6m" -> "-6mon".
fn plex_span(v: &str) -> String {
    let v = v.trim().to_ascii_lowercase();
    match v.strip_suffix('m') {
        Some(n) => format!("-{n}mon"),
        None => format!("-{v}"),
    }
}

fn span_from(v: &str) -> Option<String> {
    let v = v.trim().strip_prefix('-')?;
    let v = match v.strip_suffix("mon") {
        Some(n) => format!("{n}m"),
        None => v.to_string(),
    };
    super::rules::parse_span(&v).map(|_| v)
}

fn stars_to_plex(v: &str) -> String {
    let n = v.trim().parse::<f64>().unwrap_or(0.0) * 2.0;
    if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") }
}

fn plex_to_stars(v: &str) -> Option<String> {
    let n = v.trim().parse::<f64>().ok()? / 2.0;
    Some(if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") })
}

fn unsupported(what: &str) -> AppError {
    AppError::Invalid(format!("This smart playlist uses {what}, which Platter's editor can't show yet. Edit it in Plex."))
}

/// One condition as `key<op>=value`, the key and operator encoded.
fn cond(field: &str, op: &str, value: &str) -> String {
    // The operator without its last "=" goes with the key.
    let prefix = &op[..op.len() - 1];
    format!("{}{}={}", field, enc(prefix), enc(value))
}

/// The query (without `?`) for these rules. `liked_min` is the Plex rating
/// that counts as liked (10 or 8).
pub fn build_query(rules: &SmartRules, liked_min: f64, tags: &TagIds) -> AppResult<String> {
    let gaps = rules.plex_gaps();
    if !gaps.is_empty() {
        return Err(AppError::Invalid(gaps.join("; ")));
    }
    let mut conds = Vec::new();
    for r in &rules.rules {
        let field = plex_field(r).ok_or_else(|| AppError::Invalid(format!("{} isn't something Plex can filter on", r.field.label())))?;
        let v = r.value.trim();
        let c = match r.field {
            Field::Genre | Field::Style | Field::Mood => {
                let id = tags
                    .get(&field)
                    .and_then(|list| list.iter().find(|(_, name)| name.eq_ignore_ascii_case(v)))
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| AppError::Invalid(format!("Plex has no {} called \"{v}\".", r.field.label().to_lowercase())))?;
                cond(&field, if r.op == Op::Is { "=" } else { "!=" }, &id)
            }
            Field::Artist | Field::Album => {
                let op = match r.op {
                    Op::Is => "==",
                    Op::IsNot => "!==",
                    Op::Contains => "=",
                    _ => "!=",
                };
                cond(&field, op, v)
            }
            Field::Year | Field::PlayCount | Field::Decade | Field::Rating => {
                let value = if r.field == Field::Rating { stars_to_plex(v) } else { v.to_string() };
                let op = match r.op {
                    Op::Is => "=",
                    Op::IsNot => "!=",
                    Op::Gt => ">>=",
                    _ => "<<=",
                };
                cond(&field, op, &value)
            }
            Field::AddedAt | Field::LastPlayed => cond(&field, if r.op == Op::InLast { ">>=" } else { "<<=" }, &plex_span(v)),
            Field::Liked => {
                let min = liked_min.round() as i64;
                if v == "true" { cond(&field, ">>=", &(min - 1).to_string()) } else { cond(&field, "<<=", &min.to_string()) }
            }
            _ => unreachable!("checked by plex_gaps"),
        };
        conds.push(c);
    }
    let mut parts = vec![format!("type={TRACK_TYPE}")];
    if !rules.match_all && conds.len() > 1 {
        parts.push("push=1".into());
        parts.push(conds.join("&or=1&"));
        parts.push("pop=1".into());
    } else {
        parts.extend(conds);
    }
    for (k, v) in &rules.extra {
        if k == "sort" && rules.sort != SortBy::None {
            continue;
        }
        parts.push(format!("{}={}", enc(k), enc(v)));
    }
    if let Some(s) = sort_key(rules.sort) {
        parts.push(format!("sort={}", enc(s)));
    }
    if let Some(Limit { unit: LimitUnit::Songs, value }) = rules.limit {
        parts.push(format!("limit={value}"));
    }
    Ok(parts.join("&"))
}

/// The `uri` a smart playlist is created or updated with.
pub fn smart_uri(machine_id: &str, section_id: &str, query: &str) -> String {
    format!("server://{machine_id}/{LIBRARY_IDENTIFIER}/library/sections/{section_id}/all?{query}")
}

/// One condition read back: Plex field, operator, value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cond {
    pub field: String,
    pub op: String,
    pub value: String,
}

/// A smart playlist's query, read from its `content`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub section_id: Option<String>,
    pub any: bool,
    pub conds: Vec<Cond>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub extra: Vec<(String, String)>,
}

const OP_CHARS: &[char] = &['!', '=', '<', '>'];

/// Read `library://x/directory/...` (or a `server://` uri, or the bare path).
pub fn parse_content(content: &str) -> AppResult<Parsed> {
    let path = match content.split_once("/directory/") {
        Some((_, p)) => dec(p),
        None => match content.find("/library/sections/") {
            Some(i) => content[i..].to_string(),
            None => return Err(unsupported("a kind of query")),
        },
    };
    let (path, query) = path.split_once('?').unwrap_or((path.as_str(), ""));
    let section_id = path.strip_prefix("/library/sections/").and_then(|p| p.split('/').next()).map(str::to_owned);
    let mut out = Parsed { section_id, ..Default::default() };
    let mut depth = 0;
    for piece in query.split('&').filter(|p| !p.is_empty()) {
        let (raw_k, raw_v) = piece.split_once('=').unwrap_or((piece, ""));
        let key = dec(raw_k);
        let value = dec(raw_v);
        match key.as_str() {
            "type" => {
                if value != TRACK_TYPE {
                    return Err(unsupported("something other than songs"));
                }
            }
            "push" => {
                depth += 1;
                if depth > 1 {
                    return Err(unsupported("nested rule groups"));
                }
                out.any = true;
            }
            "pop" => depth -= 1,
            "or" => out.any = true,
            "and" => {}
            "sort" => out.sort = Some(value),
            "limit" => out.limit = value.parse().ok(),
            _ => {
                let field = key.trim_end_matches(OP_CHARS);
                let prefix = &key[field.len()..];
                let is_filter = field.contains('.') || matches!(field, "genre" | "mood" | "style" | "userRating" | "viewCount" | "year" | "decade");
                if is_filter {
                    out.conds.push(Cond { field: field.to_string(), op: format!("{prefix}="), value });
                } else {
                    out.extra.push((key, value));
                }
            }
        }
    }
    Ok(out)
}

/// Tag fields a parsed query names (to look their ids up).
pub fn parsed_tag_fields(p: &Parsed) -> Vec<String> {
    let mut out = Vec::new();
    for c in &p.conds {
        let tail = c.field.rsplit('.').next().unwrap_or("");
        if matches!(tail, "genre" | "style" | "mood") && !out.contains(&c.field) {
            out.push(c.field.clone());
        }
    }
    out
}

/// The editor's rules for a parsed query, or why they don't map.
pub fn rules_from(p: &Parsed, liked_min: f64, tags: &TagIds) -> AppResult<SmartRules> {
    let mut rules = Vec::new();
    for c in &p.conds {
        let tail = c.field.rsplit('.').next().unwrap_or("");
        let scope = c.field.split('.').next().unwrap_or("");
        let keep = |default: &str| if c.field == default { None } else { Some(c.field.clone()) };
        let rule = match (scope, tail) {
            (_, "genre" | "style" | "mood") => {
                let field = match tail {
                    "genre" => Field::Genre,
                    "style" => Field::Style,
                    _ => Field::Mood,
                };
                let op = match c.op.as_str() {
                    "=" => Op::Is,
                    "!=" => Op::IsNot,
                    _ => return Err(unsupported("a tag comparison")),
                };
                let name = tags
                    .get(&c.field)
                    .and_then(|l| l.iter().find(|(id, _)| *id == c.value))
                    .map(|(_, n)| n.clone())
                    .ok_or_else(|| unsupported("a tag Plex no longer has"))?;
                let default = format!("album.{tail}");
                Rule { field, op, value: name, plex_field: keep(&default) }
            }
            ("artist" | "album", "title") => {
                let field = if scope == "artist" { Field::Artist } else { Field::Album };
                let op = match c.op.as_str() {
                    "==" => Op::Is,
                    "!==" => Op::IsNot,
                    "=" => Op::Contains,
                    "!=" => Op::NotContains,
                    _ => return Err(unsupported("a text comparison")),
                };
                Rule { field, op, value: c.value.clone(), plex_field: None }
            }
            ("album", "year" | "decade") | ("track", "viewCount") | ("track", "userRating") => {
                let field = match tail {
                    "year" => Field::Year,
                    "decade" => Field::Decade,
                    "viewCount" => Field::PlayCount,
                    _ => Field::Rating,
                };
                if field == Field::Rating {
                    let min = liked_min.round() as i64;
                    if c.op == ">>=" && c.value == (min - 1).to_string() {
                        rules.push(Rule { field: Field::Liked, op: Op::Is, value: "true".into(), plex_field: None });
                        continue;
                    }
                    if c.op == "<<=" && c.value == min.to_string() {
                        rules.push(Rule { field: Field::Liked, op: Op::Is, value: "false".into(), plex_field: None });
                        continue;
                    }
                }
                let op = match c.op.as_str() {
                    "=" => Op::Is,
                    "!=" => Op::IsNot,
                    ">>=" => Op::Gt,
                    "<<=" => Op::Lt,
                    _ => return Err(unsupported("a number comparison")),
                };
                let value = if field == Field::Rating {
                    plex_to_stars(&c.value).ok_or_else(|| unsupported("a rating"))?
                } else {
                    c.value.clone()
                };
                Rule { field, op, value, plex_field: None }
            }
            ("album", "addedAt") | ("track", "lastViewedAt") => {
                let field = if tail == "addedAt" { Field::AddedAt } else { Field::LastPlayed };
                let op = match c.op.as_str() {
                    ">>=" => Op::InLast,
                    "<<=" => Op::NotInLast,
                    _ => return Err(unsupported("a date comparison")),
                };
                let value = span_from(&c.value).ok_or_else(|| unsupported("a fixed date"))?;
                Rule { field, op, value, plex_field: None }
            }
            _ => return Err(unsupported(&format!("a rule on {}", c.field))),
        };
        rule.check().map_err(|_| unsupported(&format!("a rule on {}", c.field)))?;
        rules.push(rule);
    }
    let mut extra = p.extra.clone();
    let sort = match p.sort.as_deref() {
        None => SortBy::None,
        Some(s) => match sort_from(s) {
            Some(x) => x,
            None => {
                extra.push(("sort".into(), s.to_string()));
                SortBy::None
            }
        },
    };
    Ok(SmartRules {
        match_all: !p.any,
        rules,
        limit: p.limit.map(|value| Limit { unit: LimitUnit::Songs, value }),
        sort,
        update: Update::Live,
        extra,
    })
}
