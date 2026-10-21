//! Prompted playlists: a description typed by the user becomes the same
//! rule set the editor shows, plus a ranked pick of songs, using the AI
//! provider chosen in Settings (none by default).
//!
//! Two requests: the first gets the description and the names of the
//! library's genres, styles and moods (and its years) and answers with
//! rules; the second gets up to [`MAX_CANDIDATES`] songs those rules found
//! (title, artist, album, year) and answers with the best ones in order.
//! Unreleased music is never sent. The key lives in the OS keychain.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::eval::Row;
use super::rules::{Field, Limit, LimitUnit, Op, Rule, SmartRules, SortBy};
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::web::{Web, WebRequest};

pub const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_KEY_ACCOUNT: &str = "anthropic-api-key";
/// The model the setting starts with.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Most songs sent for ranking.
pub const MAX_CANDIDATES: usize = 150;
/// Most picks kept.
pub const MAX_PICKS: usize = 50;
const MAX_TAGS: usize = 200;
const MAX_PROMPT: usize = 2000;

/// Models that take the server-side fallback on a declined request.
const FALLBACK_MODELS: &[&str] = &["claude-opus-5-5", "claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiProvider {
    #[default]
    None,
    Anthropic,
}

/// What the library has, for the first request.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Vocab {
    pub genres: Vec<String>,
    pub styles: Vec<String>,
    pub moods: Vec<String>,
    pub years: Option<(i32, i32)>,
}

fn top(lists: impl Iterator<Item = Vec<String>>) -> Vec<String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for l in lists {
        for t in l {
            *count.entry(t).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().take(MAX_TAGS).map(|(t, _)| t).collect()
}

/// The library's most used tags (released music only).
pub fn vocab(rows: &[Row]) -> Vocab {
    let released: Vec<&Row> = rows.iter().filter(|r| !r.unreleased).collect();
    let years: Vec<i32> = released.iter().filter_map(|r| r.year).filter(|y| *y > 0).collect();
    Vocab {
        genres: top(released.iter().map(|r| r.genres.clone())),
        styles: top(released.iter().map(|r| r.styles.clone())),
        moods: top(released.iter().map(|r| r.moods.clone())),
        years: years.iter().min().zip(years.iter().max()).map(|(a, b)| (*a, *b)),
    }
}

const FIELDS: &[&str] = &[
    "genre", "style", "mood", "artist", "album", "year", "decade", "addedAt", "playCount", "lastPlayed", "playedHere", "rating", "liked", "duration", "releaseType",
];
const OPS: &[&str] = &["is", "isNot", "contains", "notContains", "gt", "lt", "inLast", "notInLast"];
const SORTS: &[&str] = &["none", "random", "mostPlayed", "leastPlayed", "recentlyAdded", "recentlyPlayed", "rating"];

fn rules_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "matchAll", "rules", "sort", "limitSongs"],
        "properties": {
            "title": { "type": "string" },
            "matchAll": { "type": "boolean" },
            "rules": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["field", "op", "value"],
                    "properties": {
                        "field": { "type": "string", "enum": FIELDS },
                        "op": { "type": "string", "enum": OPS },
                        "value": { "type": "string" }
                    }
                }
            },
            "sort": { "type": "string", "enum": SORTS },
            "limitSongs": { "type": "integer" }
        }
    })
}

fn picks_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["picks"],
        "properties": { "picks": { "type": "array", "items": { "type": "integer" } } }
    })
}

const RULES_SYSTEM: &str = "You turn a description of a playlist into filter rules over the user's music library. \
Fields: genre, style, mood (use names from the lists given, exactly), artist, album (text), year, playCount, \
rating (stars 1-5), duration (minutes) take is/isNot/gt/lt with a number; decade takes is/isNot with a year like 1990; \
addedAt, lastPlayed, playedHere take inLast/notInLast with a span like 30d, 2w, 6m or 1y; liked takes is with true or false; \
releaseType takes is/isNot with one of albums, singlesEps, live, compilations, soundtracks, remixes, mixtapes. \
artist and album also take contains/notContains. Prefer a few broad rules; use matchAll false when any rule may match. \
limitSongs is how many songs the playlist should hold (0 for no limit). The title is short, no quotes.";

const PICKS_SYSTEM: &str = "You pick and order songs for a playlist from a numbered list. \
Answer with the numbers of the songs that fit the description best, best first, at most 50, only numbers from the list.";

#[derive(Deserialize)]
struct AiRule {
    field: String,
    op: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiRules {
    title: String,
    match_all: bool,
    rules: Vec<AiRule>,
    sort: String,
    limit_songs: i64,
}

#[derive(Deserialize)]
struct AiPicks {
    picks: Vec<i64>,
}

/// What the editor fills in from a prompt.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompted {
    pub title: String,
    pub rules: SmartRules,
    /// Song ids, best first.
    pub picks: Vec<String>,
}

pub struct Anthropic {
    web: Arc<dyn Web>,
    keys: Arc<dyn TokenStore>,
}

impl Anthropic {
    pub fn new(web: Arc<dyn Web>, keys: Arc<dyn TokenStore>) -> Self {
        Self { web, keys }
    }

    pub fn has_key(&self) -> bool {
        self.keys.get(ANTHROPIC_KEY_ACCOUNT).ok().flatten().is_some()
    }

    pub fn set_key(&self, key: Option<&str>) -> AppResult<()> {
        match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => self.keys.set(ANTHROPIC_KEY_ACCOUNT, &crate::plex::token::Token::new(k)?),
            None => self.keys.delete(ANTHROPIC_KEY_ACCOUNT),
        }
    }

    /// One request with a JSON answer in `schema`'s shape.
    async fn json(&self, model: &str, system: &str, user: String, schema: serde_json::Value) -> AppResult<serde_json::Value> {
        let key = self
            .keys
            .get(ANTHROPIC_KEY_ACCOUNT)?
            .ok_or_else(|| AppError::Invalid("Add your Anthropic API key in Settings first.".into()))?;
        let mut body = json!({
            "model": model,
            "max_tokens": 16000,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
            "output_config": { "format": { "type": "json_schema", "schema": schema } }
        });
        let mut req = WebRequest::post_json(ANTHROPIC_URL, String::new())
            .header("x-api-key", key.expose())
            .header("anthropic-version", "2023-06-01")
            .timeout(Duration::from_secs(120));
        if FALLBACK_MODELS.contains(&model) {
            body["fallbacks"] = json!("default");
            req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
        }
        req.body = crate::web::Body::Json(body.to_string());
        let resp = self.web.send(req).await?;
        match resp.status {
            200..=299 => {}
            401 | 403 => return Err(AppError::Auth("Anthropic didn't accept your API key.".into())),
            404 => return Err(AppError::Invalid(format!("Anthropic doesn't know the model \"{model}\". Check it in Settings."))),
            429 | 529 => return Err(AppError::Server("Anthropic is busy. Try again in a moment.".into())),
            s if s >= 500 => return Err(AppError::Server("Anthropic is having trouble. Try again in a moment.".into())),
            _ => return Err(AppError::Invalid("Anthropic couldn't make a playlist from that. Try describing it differently.".into())),
        }
        let v: serde_json::Value = resp.json()?;
        if v["stop_reason"] == "refusal" {
            return Err(AppError::Invalid("The AI provider declined this one. Try describing it differently.".into()));
        }
        let text = v["content"]
            .as_array()
            .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
            .and_then(|b| b["text"].as_str())
            .ok_or_else(|| AppError::Decode("no answer from Anthropic".into()))?;
        serde_json::from_str(text).map_err(|e| AppError::Decode(format!("Anthropic answer: {e}")))
    }
}

fn field_of(s: &str) -> Option<Field> {
    serde_json::from_value(json!(s)).ok()
}

fn op_of(s: &str) -> Option<Op> {
    serde_json::from_value(json!(s)).ok()
}

fn sort_of(s: &str) -> SortBy {
    serde_json::from_value(json!(s)).unwrap_or_default()
}

/// The first answer as rules (rules that don't check out are left out).
fn to_rules(a: AiRules) -> (String, SmartRules) {
    let rules: Vec<Rule> = a
        .rules
        .into_iter()
        .filter_map(|r| {
            let rule = Rule { field: field_of(&r.field)?, op: op_of(&r.op)?, value: r.value, plex_field: None };
            rule.check().ok().map(|_| rule)
        })
        .take(super::rules::MAX_RULES)
        .collect();
    let limit = (a.limit_songs > 0).then(|| Limit { unit: LimitUnit::Songs, value: (a.limit_songs as u32).min(super::rules::MAX_LIMIT_SONGS) });
    let title: String = a.title.trim().trim_matches('"').chars().take(100).collect();
    (title, SmartRules { match_all: a.match_all, rules, limit, sort: sort_of(&a.sort), ..SmartRules::default() })
}

/// Turn a description into rules and picks.
pub async fn generate(ai: &Anthropic, model: &str, prompt: &str, rows: &[Row], now: i64, liked_min: f64) -> AppResult<Prompted> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::Invalid("Describe the playlist first.".into()));
    }
    if prompt.chars().count() > MAX_PROMPT {
        return Err(AppError::Invalid(format!("Descriptions can be up to {MAX_PROMPT} characters.")));
    }
    let v = vocab(rows);
    let user = format!(
        "Description: {prompt}\n\nGenres: {}\nStyles: {}\nMoods: {}\nYears: {}",
        v.genres.join(", "),
        v.styles.join(", "),
        v.moods.join(", "),
        v.years.map(|(a, b)| format!("{a} to {b}")).unwrap_or_default()
    );
    let a: AiRules = serde_json::from_value(ai.json(model, RULES_SYSTEM, user, rules_schema()).await?)
        .map_err(|e| AppError::Decode(format!("Anthropic rules: {e}")))?;
    let (title, rules) = to_rules(a);

    // Songs the rules find (released only), for the ranking.
    let released: Vec<Row> = rows.iter().filter(|r| !r.unreleased).cloned().collect();
    let mut wide = rules.clone();
    wide.limit = None;
    wide.sort = SortBy::MostPlayed;
    let found: Vec<&Row> = super::eval::evaluate(&released, &wide, now, liked_min, 0).into_iter().take(MAX_CANDIDATES).collect();
    if found.is_empty() {
        return Ok(Prompted { title, rules, picks: Vec::new() });
    }
    let list: Vec<String> = found
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let year = r.year.map(|y| format!(", {y}")).unwrap_or_default();
            format!("{}. {} by {} ({}{year})", i + 1, r.title, r.track_artist.as_deref().unwrap_or(&r.artist), r.album)
        })
        .collect();
    let user = format!("Description: {prompt}\n\nSongs:\n{}", list.join("\n"));
    let p: AiPicks = serde_json::from_value(ai.json(model, PICKS_SYSTEM, user, picks_schema()).await?)
        .map_err(|e| AppError::Decode(format!("Anthropic picks: {e}")))?;
    let mut picks: Vec<String> = Vec::new();
    for n in p.picks {
        if let Some(r) = usize::try_from(n).ok().and_then(|n| n.checked_sub(1)).and_then(|i| found.get(i))
            && !picks.contains(&r.id)
        {
            picks.push(r.id.clone());
        }
        if picks.len() >= MAX_PICKS {
            break;
        }
    }
    Ok(Prompted { title, rules, picks })
}
