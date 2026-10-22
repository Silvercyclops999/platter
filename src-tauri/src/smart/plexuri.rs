//! Rules to and from Plex's smart playlist filter language.
//!
//! A Plex smart playlist is a library query: `/library/sections/{id}/all?
//! type=10&<field><op>=<value>&...` (docs/PLEX-API.md "Smart playlists").
//! The operator sits in the key, percent-encoded: `track.userRating%3E%3E=4`
//! is "rating greater than 4" (Plex counts half stars, 0 to 10). Tag fields
//! (genre, style, mood) take Plex tag ids, so names are looked up first.
//! "Match any" wraps the conditions in `push=1 ... or=1 ... pop=1`; a group
//! of rules is its own `push=1 ... pop=1` joined by `and=1` or `or=1`, and a
//! number between two others is a group of "more than" and "less than".
//! Plex reports a smart playlist's query as `content`:
//! `library://x/directory/<the path, percent-encoded>`.

use std::collections::HashMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use super::rules::{Field, Limit, LimitUnit, Op, Rule, RuleGroup, SmartRules, SortBy, Update, parse_number_range};
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
            Field::Title => "track.title",
            Field::Year => "album.year",
            Field::Decade => "album.decade",
            Field::AddedAt => "album.addedAt",
            Field::PlayCount => "track.viewCount",
            Field::LastPlayed => "track.lastViewedAt",
            Field::Rating | Field::Liked => "track.userRating",
            // Filled on this computer only (Field::local_only).
            _ => return None,
        }
        .to_string(),
    )
}

/// Tag fields these rules need ids for.
pub fn tag_fields(rules: &SmartRules) -> Vec<String> {
    let mut out = Vec::new();
    for r in rules.all_rules() {
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

/// Whether Plex can hold a "between" rule: whole numbers in Plex's own units
/// (half stars for ratings), since Plex compares "more than" and "less than".
pub fn whole_range(r: &Rule) -> bool {
    let scale = if r.field == Field::Rating { 2.0 } else { 1.0 };
    parse_number_range(&r.value).is_some_and(|(a, b)| (a * scale).fract() == 0.0 && (b * scale).fract() == 0.0)
}

fn number_text(n: f64) -> String {
    if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") }
}

/// One rule as Plex conditions (a "between" is a group of two).
fn rule_query(r: &Rule, liked_min: f64, tags: &TagIds) -> AppResult<String> {
    let field = plex_field(r).ok_or_else(|| AppError::Invalid(format!("{} isn't something Plex can filter on", r.field.label())))?;
    let v = r.value.trim();
    Ok(match r.field {
        Field::Genre | Field::Style | Field::Mood => {
            let id = tags
                .get(&field)
                .and_then(|list| list.iter().find(|(_, name)| name.eq_ignore_ascii_case(v)))
                .map(|(id, _)| id.clone())
                .ok_or_else(|| AppError::Invalid(format!("Plex has no {} called \"{v}\".", r.field.label().to_lowercase())))?;
            cond(&field, if r.op == Op::Is { "=" } else { "!=" }, &id)
        }
        Field::Artist | Field::Album | Field::Title => {
            let op = match r.op {
                Op::Is => "==",
                Op::IsNot => "!==",
                Op::Contains => "=",
                Op::BeginsWith => "<=",
                Op::EndsWith => ">=",
                _ => "!=",
            };
            cond(&field, op, v)
        }
        Field::Year | Field::PlayCount | Field::Decade | Field::Rating if r.op == Op::Between => {
            let (a, b) = parse_number_range(v).ok_or_else(|| AppError::Invalid(format!("Check the value for {}.", r.field.label())))?;
            let scale = if r.field == Field::Rating { 2.0 } else { 1.0 };
            let below = cond(&field, "<<=", &number_text(b * scale + 1.0));
            // From nothing up: "less than" alone, so songs Plex has no number for still count.
            if a <= 0.0 {
                return Ok(below);
            }
            // Plex's "more than" and "less than" leave the ends out, so step past them.
            format!(
                "push=1&{}&and=1&{}&pop=1",
                cond(&field, ">>=", &number_text(a * scale - 1.0)),
                below
            )
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
    })
}

/// Conditions joined: all of them, or any one (a group of their own).
fn joined(items: Vec<String>, all: bool, wrap: bool) -> String {
    if items.len() == 1 {
        return items.into_iter().next().unwrap_or_default();
    }
    let join = if all { "&and=1&" } else { "&or=1&" };
    if wrap { format!("push=1&{}&pop=1", items.join(join)) } else { items.join(join) }
}

/// The query (without `?`) for these rules. `liked_min` is the Plex rating
/// that counts as liked (10 or 8).
pub fn build_query(rules: &SmartRules, liked_min: f64, tags: &TagIds) -> AppResult<String> {
    let gaps = rules.plex_gaps();
    if !gaps.is_empty() {
        return Err(AppError::Invalid(gaps.join("; ")));
    }
    let mut items = Vec::new();
    for r in &rules.rules {
        items.push(rule_query(r, liked_min, tags)?);
    }
    for g in &rules.groups {
        let inner = g.rules.iter().map(|r| rule_query(r, liked_min, tags)).collect::<AppResult<Vec<_>>>()?;
        // Always a group of its own, so it reads back as one.
        items.push(if inner.len() == 1 { format!("push=1&{}&pop=1", inner[0]) } else { joined(inner, g.match_all, true) });
    }
    let mut parts = vec![format!("type={TRACK_TYPE}")];
    if !items.is_empty() {
        if !rules.match_all && items.len() > 1 {
            parts.push(joined(items, false, true));
        } else {
            // All of them: Plex reads conditions side by side as "and".
            parts.push(items.join("&"));
        }
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

/// Conditions in `push=1 ... pop=1`, or the whole query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Group {
    /// Joined by `or=1` (otherwise `and=1`, or side by side).
    pub any: bool,
    pub items: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Cond(Cond),
    Group(Group),
}

/// A smart playlist's query, read from its `content`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub section_id: Option<String>,
    pub root: Group,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub extra: Vec<(String, String)>,
}

const OP_CHARS: &[char] = &['!', '=', '<', '>'];
/// Groups inside groups Plex can nest before the query is refused as too deep.
const MAX_DEPTH: usize = 8;

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
    // Open groups, each with whether it has seen "and" and "or".
    let mut stack: Vec<(Group, bool, bool)> = vec![(Group::default(), false, false)];
    for piece in query.split('&').filter(|p| !p.is_empty()) {
        let (raw_k, raw_v) = piece.split_once('=').unwrap_or((piece, ""));
        let key = dec(raw_k);
        let value = dec(raw_v);
        let top = stack.len() - 1;
        match key.as_str() {
            "type" => {
                if value != TRACK_TYPE {
                    return Err(unsupported("something other than songs"));
                }
            }
            "push" => {
                if stack.len() > MAX_DEPTH {
                    return Err(unsupported("rule groups nested that deep"));
                }
                stack.push((Group::default(), false, false));
            }
            "pop" => {
                if stack.len() < 2 {
                    return Err(unsupported("a kind of query"));
                }
                let (g, _, _) = stack.pop().unwrap_or_default();
                let top = stack.len() - 1;
                stack[top].0.items.push(Node::Group(g));
            }
            "or" => {
                stack[top].2 = true;
                stack[top].0.any = true;
            }
            "and" => stack[top].1 = true,
            "sort" => out.sort = Some(value),
            "limit" => out.limit = value.parse().ok(),
            _ => {
                let field = key.trim_end_matches(OP_CHARS);
                let prefix = &key[field.len()..];
                let is_filter = field.contains('.') || matches!(field, "genre" | "mood" | "style" | "userRating" | "viewCount" | "year" | "decade");
                if is_filter {
                    stack[top].0.items.push(Node::Cond(Cond { field: field.to_string(), op: format!("{prefix}="), value }));
                } else {
                    out.extra.push((key, value));
                }
            }
        }
        if stack.iter().any(|(_, and, or)| *and && *or) {
            return Err(unsupported("\"and\" and \"or\" mixed in one group"));
        }
    }
    if stack.len() != 1 {
        return Err(unsupported("a kind of query"));
    }
    out.root = stack.pop().map(|(g, _, _)| g).unwrap_or_default();
    Ok(out)
}

fn walk<'a>(g: &'a Group, out: &mut Vec<&'a Cond>) {
    for n in &g.items {
        match n {
            Node::Cond(c) => out.push(c),
            Node::Group(inner) => walk(inner, out),
        }
    }
}

/// Every condition in a parsed query.
pub fn parsed_conds(p: &Parsed) -> Vec<&Cond> {
    let mut out = Vec::new();
    walk(&p.root, &mut out);
    out
}

/// Tag fields a parsed query names (to look their ids up).
pub fn parsed_tag_fields(p: &Parsed) -> Vec<String> {
    let mut out = Vec::new();
    for c in parsed_conds(p) {
        let tail = c.field.rsplit('.').next().unwrap_or("");
        if matches!(tail, "genre" | "style" | "mood") && !out.contains(&c.field) {
            out.push(c.field.clone());
        }
    }
    out
}

/// The editor's field for a Plex number field.
fn number_field(scope: &str, tail: &str) -> Option<Field> {
    Some(match (scope, tail) {
        ("album", "year") => Field::Year,
        ("album", "decade") => Field::Decade,
        ("track", "viewCount") => Field::PlayCount,
        ("track", "userRating") => Field::Rating,
        _ => return None,
    })
}

/// "More than a and less than b" on one number field: a "between" rule.
fn between(g: &Group) -> Option<Rule> {
    let [Node::Cond(x), Node::Cond(y)] = g.items.as_slice() else { return None };
    if g.any || x.field != y.field {
        return None;
    }
    let (lo, hi) = match (x.op.as_str(), y.op.as_str()) {
        (">>=", "<<=") => (x, y),
        ("<<=", ">>=") => (y, x),
        _ => return None,
    };
    let field = number_field(x.field.split('.').next().unwrap_or(""), x.field.rsplit('.').next().unwrap_or(""))?;
    let scale = if field == Field::Rating { 2.0 } else { 1.0 };
    let a = (lo.value.trim().parse::<f64>().ok()? + 1.0) / scale;
    let b = (hi.value.trim().parse::<f64>().ok()? - 1.0) / scale;
    let r = Rule { field, op: Op::Between, value: format!("{}..{}", number_text(a), number_text(b)), plex_field: None };
    r.check().ok().map(|_| r)
}

/// One condition as the editor's rule.
fn cond_rule(c: &Cond, liked_min: f64, tags: &TagIds) -> AppResult<Rule> {
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
        ("artist" | "album" | "track", "title") => {
            let field = match scope {
                "artist" => Field::Artist,
                "album" => Field::Album,
                _ => Field::Title,
            };
            let op = match c.op.as_str() {
                "==" => Op::Is,
                "!==" => Op::IsNot,
                "=" => Op::Contains,
                "!=" => Op::NotContains,
                "<=" => Op::BeginsWith,
                ">=" => Op::EndsWith,
                _ => return Err(unsupported("a text comparison")),
            };
            Rule { field, op, value: c.value.clone(), plex_field: None }
        }
        ("album", "year" | "decade") | ("track", "viewCount") | ("track", "userRating") => {
            let field = number_field(scope, tail).unwrap_or(Field::Year);
            if field == Field::Rating {
                let min = liked_min.round() as i64;
                if c.op == ">>=" && c.value == (min - 1).to_string() {
                    return Ok(Rule { field: Field::Liked, op: Op::Is, value: "true".into(), plex_field: None });
                }
                if c.op == "<<=" && c.value == min.to_string() {
                    return Ok(Rule { field: Field::Liked, op: Op::Is, value: "false".into(), plex_field: None });
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
    Ok(rule)
}

/// A rule from a condition, or from a "between" group.
fn node_rule(n: &Node, liked_min: f64, tags: &TagIds) -> AppResult<Option<Rule>> {
    match n {
        Node::Cond(c) => cond_rule(c, liked_min, tags).map(Some),
        Node::Group(g) => Ok(between(g)),
    }
}

/// The editor's rules for a parsed query, or why they don't map.
pub fn rules_from(p: &Parsed, liked_min: f64, tags: &TagIds) -> AppResult<SmartRules> {
    // "Match any" wraps everything in one group of its own.
    let mut root = &p.root;
    if let [Node::Group(only)] = root.items.as_slice()
        && between(only).is_none()
    {
        root = only;
    }
    let mut rules = Vec::new();
    let mut groups = Vec::new();
    for n in &root.items {
        if let Some(r) = node_rule(n, liked_min, tags)? {
            rules.push(r);
            continue;
        }
        let Node::Group(g) = n else { continue };
        let mut inner = Vec::new();
        for m in &g.items {
            match node_rule(m, liked_min, tags)? {
                Some(r) => inner.push(r),
                None => return Err(unsupported("rule groups inside groups")),
            }
        }
        if !inner.is_empty() {
            groups.push(RuleGroup { match_all: !g.any, rules: inner });
        }
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
    let out = SmartRules {
        match_all: !root.any,
        rules,
        groups,
        limit: p.limit.map(|value| Limit { unit: LimitUnit::Songs, value }),
        sort,
        update: Update::Live,
        extra,
    };
    if out.groups.len() > super::rules::MAX_GROUPS || out.all_rules().count() > super::rules::MAX_RULES {
        return Err(unsupported("more rules than the editor holds"));
    }
    Ok(out)
}
