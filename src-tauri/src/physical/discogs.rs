//! The Discogs client: who a personal token belongs to, and the items of
//! that user's collection.
//!
//! The token goes only in the `Authorization` header (flagged sensitive by
//! the real client), never in a URL, an error or the log. Requests are paced
//! to one a second, inside Discogs' 60 a minute for signed-in requests, and a
//! 429 pauses them. The user agent names the app, as Discogs asks.

use std::sync::Arc;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;

use super::{DiscogsItem, Format};
use crate::artistinfo::art::EXT_PREFIX;
use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::limit::Limiter;
use crate::error::{AppError, AppResult};
use crate::plex::token::Token;

pub const API: &str = "https://api.discogs.com";
/// Items asked for per page (Discogs' largest).
pub const PER_PAGE: usize = 100;
/// Most pages read (10,000 items).
pub const MAX_PAGES: usize = 100;
/// Collection notes Discogs gives every user: media and sleeve condition, notes.
const FIELD_MEDIA: u64 = 1;
const FIELD_SLEEVE: u64 = 2;
const FIELD_NOTES: u64 = 3;

pub const REFUSED: &str = "Discogs didn't accept that token. Copy a new one from your Discogs developer settings.";
pub const BUSY: &str = "Discogs is busy. Try again in a minute.";

pub struct DiscogsClient {
    pub http: Arc<dyn InfoHttp>,
    pub limiter: Arc<Limiter>,
}

#[derive(Deserialize)]
struct Identity {
    username: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Page {
    pagination: Pagination,
    releases: Vec<Release>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Pagination {
    pages: usize,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Release {
    id: u64,
    instance_id: u64,
    basic_information: Basic,
    notes: Vec<Note>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Basic {
    title: String,
    year: Option<i32>,
    thumb: Option<String>,
    cover_image: Option<String>,
    formats: Vec<DFormat>,
    artists: Vec<DArtist>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DFormat {
    name: String,
    descriptions: Vec<String>,
    text: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DArtist {
    name: String,
    join: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Note {
    field_id: u64,
    value: String,
}

/// The format Platter files a Discogs format name under.
pub fn format_of(name: &str) -> Format {
    match name.trim().to_ascii_lowercase().as_str() {
        "vinyl" | "lathe cut" | "acetate" | "flexi-disc" | "shellac" => Format::Vinyl,
        "cd" | "cdr" | "sacd" | "hybrid" | "minidisc" | "cdv" => Format::Cd,
        "cassette" | "microcassette" | "8-track cartridge" => Format::Cassette,
        _ => Format::Other,
    }
}

/// A Discogs image as an art path the art cache fetches (only from
/// i.discogs.com).
fn art(url: Option<&str>) -> Option<String> {
    let u = url::Url::parse(url?.trim()).ok()?;
    (u.scheme() == "https" && u.host_str() == Some("i.discogs.com")).then(|| format!("{EXT_PREFIX}{u}"))
}

fn item(r: Release) -> DiscogsItem {
    let b = r.basic_information;
    // A box set with a record and a CD is filed under the first format that says which.
    let main = b.formats.iter().find(|f| format_of(&f.name) != Format::Other).or(b.formats.first());
    let mut artist = String::new();
    for (i, a) in b.artists.iter().enumerate() {
        artist.push_str(&super::clean_discogs_artist(&a.name));
        if i + 1 < b.artists.len() {
            match a.join.trim() {
                "" | "," => artist.push_str(", "),
                j => artist.push_str(&format!(" {j} ")),
            }
        }
    }
    let note = |id: u64| r.notes.iter().find(|n| n.field_id == id).map(|n| n.value.trim().to_string()).filter(|v| !v.is_empty());
    let condition = match (note(FIELD_MEDIA), note(FIELD_SLEEVE)) {
        (Some(m), Some(s)) => format!("{m}, sleeve {s}"),
        (Some(m), None) => m,
        (None, Some(s)) => format!("Sleeve {s}"),
        (None, None) => String::new(),
    };
    DiscogsItem {
        release_id: r.id,
        instance_id: r.instance_id,
        first_artist: b.artists.first().map(|a| super::clean_discogs_artist(&a.name)).unwrap_or_default(),
        artist,
        title: b.title.trim().to_string(),
        year: b.year.filter(|&y| y > 0),
        thumb: art(b.thumb.as_deref()).or_else(|| art(b.cover_image.as_deref())),
        format: main.map(|f| format_of(&f.name)).unwrap_or_default(),
        pressing: main.map(|f| f.descriptions.join(", ")).unwrap_or_default(),
        colour: main.and_then(|f| f.text.clone()).unwrap_or_default().trim().to_string(),
        condition,
        notes: note(FIELD_NOTES).unwrap_or_default(),
    }
}

impl DiscogsClient {
    fn headers(token: &Token) -> Vec<(&'static str, String)> {
        vec![("Authorization", format!("Discogs token={}", token.expose())), ("Accept", "application/vnd.discogs.v2.discogs+json".into())]
    }

    async fn get(&self, url: &str, token: &Token) -> AppResult<crate::artistinfo::http::HttpResponse> {
        let resp = match self.limiter.get(&*self.http, url, &Self::headers(token)).await {
            Ok(r) => r,
            Err(AppError::Server(_)) => return Err(AppError::Server(BUSY.into())),
            Err(AppError::Network(_)) => return Err(AppError::Network("Couldn't reach Discogs. Check your connection.".into())),
            Err(e) => return Err(e),
        };
        match resp.status {
            401 | 403 => Err(AppError::Invalid(REFUSED.into())),
            404 => Err(AppError::NotFound("Discogs has no collection for that user.".into())),
            s if !(200..300).contains(&s) => Err(AppError::Server(format!("Discogs answered {s}."))),
            _ => Ok(resp),
        }
    }

    /// The Discogs user a token belongs to.
    pub async fn identity(&self, token: &Token) -> AppResult<String> {
        let resp = self.get(&format!("{API}/oauth/identity"), token).await?;
        let id: Identity = resp.json()?;
        let name = id.username.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Invalid(REFUSED.into()));
        }
        Ok(name)
    }

    /// Every item in the user's collection (the "All" folder).
    pub async fn collection(&self, token: &Token, username: &str) -> AppResult<Vec<DiscogsItem>> {
        let user = utf8_percent_encode(username, NON_ALPHANUMERIC).to_string();
        let mut out = Vec::new();
        let mut page = 1;
        loop {
            let url = format!("{API}/users/{user}/collection/folders/0/releases?per_page={PER_PAGE}&page={page}&sort=artist");
            let p: Page = self.get(&url, token).await?.json()?;
            out.extend(p.releases.into_iter().filter(|r| r.id > 0).map(item));
            if page >= p.pagination.pages.min(MAX_PAGES) {
                break;
            }
            page += 1;
        }
        Ok(out)
    }
}
