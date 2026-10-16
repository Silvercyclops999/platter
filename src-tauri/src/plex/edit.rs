//! Metadata edits on a Plex server: artwork, matches, fields with locks,
//! merge, split and refresh (docs/PLEX-API.md "Editing metadata").
//!
//! Ids may carry their server's tag (`crate::sid`); every call takes the tag
//! off with [`PlexClient::raw`], which also refuses an id from another server.
//! Image paths read back get the tag again, so the art cache asks the right
//! server.

use reqwest::{Method, RequestBuilder, Response};
use serde::{Deserialize, Serialize};

use super::client::{map_send_error, status_error, PlexClient};
use super::lenient;
use crate::error::{AppError, AppResult};

/// What a refusal from the server means here: this account can't edit it.
pub const NO_PERMISSION: &str = "Only the server's owner and admins can edit this.";

/// Album poster or artist photo, or the background art behind an artist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageKind {
    Poster,
    Art,
}

impl ImageKind {
    fn list(self) -> &'static str {
        match self {
            ImageKind::Poster => "posters",
            ImageKind::Art => "arts",
        }
    }
    fn one(self) -> &'static str {
        match self {
            ImageKind::Poster => "poster",
            ImageKind::Art => "art",
        }
    }
}

/// Plex metadata type codes for field edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditKind {
    Artist,
    Album,
}

impl EditKind {
    pub fn code(self) -> u8 {
        match self {
            EditKind::Artist => 8,
            EditKind::Album => 9,
        }
    }
}

// ---------------------------------------------------------------- raw shapes

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(rename = "MediaContainer")]
    media_container: T,
}

#[derive(Debug, Default, Deserialize)]
struct ImageList {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<RawImage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawImage {
    #[serde(default, deserialize_with = "lenient::string")]
    key: String,
    #[serde(default, deserialize_with = "lenient::string")]
    rating_key: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    thumb: Option<String>,
    #[serde(default, deserialize_with = "lenient::bool")]
    selected: bool,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MatchList {
    #[serde(default, rename = "SearchResult")]
    results: Vec<RawMatch>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMatch {
    #[serde(default, deserialize_with = "lenient::string")]
    guid: String,
    #[serde(default, deserialize_with = "lenient::string")]
    name: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    parent_name: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    year: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    score: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    area: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ItemList {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<RawItem>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTag {
    #[serde(default, deserialize_with = "lenient::string")]
    tag: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawField {
    #[serde(default, deserialize_with = "lenient::string")]
    name: String,
    #[serde(default, deserialize_with = "lenient::bool")]
    locked: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawItem {
    #[serde(default, deserialize_with = "lenient::string")]
    rating_key: String,
    #[serde(default, rename = "type", deserialize_with = "lenient::string")]
    kind: String,
    #[serde(default, deserialize_with = "lenient::string")]
    title: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    title_sort: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    year: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    originally_available_at: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    studio: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    summary: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    guid: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    thumb: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    art: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    parent_title: Option<String>,
    #[serde(default, rename = "librarySectionID", deserialize_with = "lenient::opt_string")]
    library_section_id: Option<String>,
    #[serde(default, rename = "Genre")]
    genre: Vec<RawTag>,
    #[serde(default, rename = "Style")]
    style: Vec<RawTag>,
    #[serde(default, rename = "Mood")]
    mood: Vec<RawTag>,
    #[serde(default, rename = "Field")]
    field: Vec<RawField>,
}

// ---------------------------------------------------------------- IPC shapes

/// One image Plex offers for an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageChoice {
    /// What selects it (`PUT …/poster?url=`): a link, or `metadata://…`,
    /// `upload://…` for images the server keeps.
    pub key: String,
    /// Art path for a preview (a server path with its tag, or `/ext/<url>`).
    pub preview: Option<String>,
    /// Where it came from (`caa`, `fanarttv`, `embedded`, `upload`, …).
    pub provider: Option<String>,
    /// The one in use now.
    pub selected: bool,
}

/// One candidate from a match search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchCandidate {
    pub guid: String,
    pub name: String,
    /// The album's artist, or an artist's area.
    pub artist: Option<String>,
    pub year: Option<i64>,
    pub score: Option<i64>,
}

/// The fields Platter edits, as the server has them now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub title: String,
    pub sort_title: String,
    /// Albums: `YYYY-MM-DD`, or just the year when that's all there is.
    pub released: String,
    pub studio: String,
    pub summary: String,
    pub genres: Vec<String>,
    pub styles: Vec<String>,
    pub moods: Vec<String>,
}

/// An item as read for editing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableItem {
    /// Tagged id.
    pub id: String,
    pub kind: Option<EditKind>,
    pub fields: Snapshot,
    /// Plex field names that are locked.
    pub locked: Vec<String>,
    pub guid: Option<String>,
    pub thumb: Option<String>,
    pub art: Option<String>,
    pub artist_title: Option<String>,
    pub section_id: Option<String>,
}

impl EditableItem {
    /// Matched to an online entry (not `local://`).
    pub fn matched(&self) -> bool {
        self.guid.as_deref().is_some_and(|g| !g.is_empty() && !g.starts_with("local://"))
    }
}

// ---------------------------------------------------------------- field edits

/// Our names for the fields, and Plex's.
pub const TEXT_FIELDS: [(&str, &str); 4] =
    [("title", "title"), ("sortTitle", "titleSort"), ("studio", "studio"), ("summary", "summary")];
pub const TAG_FIELDS: [(&str, &str); 3] = [("genres", "genre"), ("styles", "style"), ("moods", "mood")];

fn text_of<'a>(s: &'a Snapshot, ours: &str) -> &'a str {
    match ours {
        "title" => &s.title,
        "sortTitle" => &s.sort_title,
        "studio" => &s.studio,
        "summary" => &s.summary,
        "released" => &s.released,
        _ => "",
    }
}

fn tags_of<'a>(s: &'a Snapshot, ours: &str) -> &'a [String] {
    match ours {
        "genres" => &s.genres,
        "styles" => &s.styles,
        "moods" => &s.moods,
        _ => &[],
    }
}

/// Plex field names a change touches (`year` and `originallyAvailableAt`
/// both for a release date).
pub fn changed_fields(kind: EditKind, from: &Snapshot, to: &Snapshot) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (ours, plex) in TEXT_FIELDS {
        if kind == EditKind::Artist && ours == "studio" {
            continue;
        }
        if text_of(from, ours).trim() != text_of(to, ours).trim() {
            out.push(plex);
        }
    }
    if kind == EditKind::Album && from.released.trim() != to.released.trim() {
        out.push("originallyAvailableAt");
    }
    for (ours, plex) in TAG_FIELDS {
        if norm_tags(tags_of(from, ours)) != norm_tags(tags_of(to, ours)) {
            out.push(plex);
        }
    }
    out
}

fn norm_tags(t: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for x in t {
        let x = x.trim();
        if !x.is_empty() && !seen.iter().any(|s| s.eq_ignore_ascii_case(x)) {
            seen.push(x.to_string());
        }
    }
    seen
}

/// A release date as typed: `YYYY` or `YYYY-MM-DD`. Empty clears it.
pub fn parse_released(s: &str) -> AppResult<(Option<i64>, Option<String>)> {
    let s = s.trim();
    if s.is_empty() {
        return Ok((None, None));
    }
    let bad = || AppError::Invalid("Use a year like 2021 or a date like 2021-06-25.".into());
    let digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [y] if digits(y, 4) => Ok((y.parse().ok(), None)),
        [y, m, d] if digits(y, 4) && digits(m, 2) && digits(d, 2) => {
            let (mm, dd): (u32, u32) = (m.parse().unwrap_or(0), d.parse().unwrap_or(0));
            if !(1..=12).contains(&mm) || !(1..=31).contains(&dd) {
                return Err(bad());
            }
            Ok((y.parse().ok(), Some(s.to_string())))
        }
        _ => Err(bad()),
    }
}

/// The query for `PUT /library/sections/{s}/all`: the fields that differ
/// from `from` set to `to`, each one locked (`lock`), or put back to its
/// lock state in `locks` when given (undo).
pub fn field_query(
    kind: EditKind,
    id: &str,
    from: &Snapshot,
    to: &Snapshot,
    lock: bool,
    locks: Option<&[String]>,
) -> AppResult<Vec<(String, String)>> {
    let mut q: Vec<(String, String)> = vec![("type".into(), kind.code().to_string()), ("id".into(), id.to_string())];
    let lock_of = |plex: &str| -> &'static str {
        match locks {
            Some(l) => {
                if l.iter().any(|x| x == plex) {
                    "1"
                } else {
                    "0"
                }
            }
            None if lock => "1",
            None => "0",
        }
    };
    let changed = changed_fields(kind, from, to);
    for (ours, plex) in TEXT_FIELDS {
        if !changed.contains(&plex) {
            continue;
        }
        let v = text_of(to, ours).trim();
        if plex == "title" && v.is_empty() {
            return Err(AppError::Invalid("The title can't be empty.".into()));
        }
        q.push((format!("{plex}.value"), v.to_string()));
        q.push((format!("{plex}.locked"), lock_of(plex).into()));
    }
    if changed.contains(&"originallyAvailableAt") {
        let (year, date) = parse_released(&to.released)?;
        q.push(("originallyAvailableAt.value".into(), date.unwrap_or_default()));
        q.push(("originallyAvailableAt.locked".into(), lock_of("originallyAvailableAt").into()));
        q.push(("year.value".into(), year.map(|y| y.to_string()).unwrap_or_default()));
        q.push(("year.locked".into(), lock_of("originallyAvailableAt").into()));
    }
    for (ours, plex) in TAG_FIELDS {
        if !changed.contains(&plex) {
            continue;
        }
        let old = norm_tags(tags_of(from, ours));
        let new = norm_tags(tags_of(to, ours));
        for (i, t) in new.iter().enumerate() {
            q.push((format!("{plex}[{i}].tag.tag"), t.clone()));
        }
        let gone: Vec<&str> = old
            .iter()
            .filter(|o| !new.iter().any(|n| n.eq_ignore_ascii_case(o)))
            .map(String::as_str)
            .collect();
        if !gone.is_empty() {
            q.push((format!("{plex}[].tag.tag-"), gone.join(",")));
        }
        q.push((format!("{plex}.locked"), lock_of(plex).into()));
    }
    Ok(q)
}

// ---------------------------------------------------------------- client

fn rating_key(id: &str) -> AppResult<&str> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(AppError::Invalid(format!("not a Plex id: {id:?}")))
    }
}

/// Map an answer to an edit. 401/403 say this account can't edit the
/// server (not a lost sign-in: reads still work); 413 is an image too big.
pub fn edit_status(resp: Response) -> AppResult<Response> {
    let status = resp.status();
    match status.as_u16() {
        200..=299 => Ok(resp),
        401 | 403 => Err(AppError::Auth(NO_PERMISSION.into())),
        413 => Err(AppError::Invalid("That image is too large for the server.".into())),
        _ => Err(status_error(status, resp.url().path())),
    }
}

impl PlexClient {
    async fn edit_send(&self, rb: RequestBuilder) -> AppResult<Response> {
        let resp = rb.send().await.map_err(map_send_error)?;
        edit_status(resp)
    }

    async fn edit_json<T: serde::de::DeserializeOwned + Default>(&self, path: &str, query: &[(&str, String)]) -> AppResult<T> {
        let rb = self.request(Method::GET, path)?.query(query);
        let body = self.edit_send(rb).await?.bytes().await.map_err(map_send_error)?;
        let env: Envelope<T> =
            serde_json::from_slice(&body).map_err(|e| AppError::Decode(format!("{}: {e}", path.split('?').next().unwrap_or(""))))?;
        Ok(env.media_container)
    }

    /// `GET /library/metadata/{id}`: the fields Platter edits, and locks.
    pub async fn editable(&self, id: &str) -> AppResult<EditableItem> {
        let k = rating_key(self.raw(id)?)?;
        let list: ItemList = self.edit_json(&format!("/library/metadata/{k}"), &[]).await?;
        let m = list.metadata.into_iter().next().ok_or_else(|| AppError::NotFound("That item is gone from the server.".into()))?;
        let tags = |t: Vec<RawTag>| t.into_iter().map(|x| x.tag).filter(|x| !x.trim().is_empty()).collect::<Vec<_>>();
        let released = m
            .originally_available_at
            .clone()
            .filter(|d| !d.trim().is_empty())
            .or_else(|| m.year.map(|y| y.to_string()))
            .unwrap_or_default();
        let kind = match m.kind.as_str() {
            "artist" => Some(EditKind::Artist),
            "album" => Some(EditKind::Album),
            _ => None,
        };
        Ok(EditableItem {
            id: self.qualify(&m.rating_key),
            kind,
            fields: Snapshot {
                title: m.title,
                sort_title: m.title_sort.unwrap_or_default(),
                released,
                studio: m.studio.unwrap_or_default(),
                summary: m.summary.unwrap_or_default(),
                genres: tags(m.genre),
                styles: tags(m.style),
                moods: tags(m.mood),
            },
            locked: m.field.into_iter().filter(|f| f.locked).map(|f| f.name).collect(),
            guid: m.guid,
            thumb: m.thumb.map(|t| self.qualify(&t)),
            art: m.art.map(|t| self.qualify(&t)),
            artist_title: m.parent_title,
            section_id: m.library_section_id,
        })
    }

    /// `GET /library/metadata/{id}/posters` (or `/arts`).
    pub async fn images(&self, id: &str, kind: ImageKind) -> AppResult<Vec<ImageChoice>> {
        let k = rating_key(self.raw(id)?)?;
        let list: ImageList = self.edit_json(&format!("/library/metadata/{k}/{}", kind.list()), &[]).await?;
        Ok(list
            .metadata
            .into_iter()
            .filter(|m| !m.rating_key.is_empty() || !m.key.is_empty())
            .map(|m| {
                let preview = m.thumb.as_deref().or(Some(m.key.as_str())).and_then(|t| {
                    if t.starts_with('/') {
                        Some(self.qualify(t))
                    } else if t.starts_with("https://") {
                        Some(format!("{}{t}", crate::artistinfo::art::EXT_PREFIX))
                    } else {
                        None
                    }
                });
                ImageChoice {
                    key: if m.rating_key.is_empty() { m.key } else { m.rating_key },
                    preview,
                    provider: m.provider,
                    selected: m.selected,
                }
            })
            .collect())
    }

    /// `PUT /library/metadata/{id}/poster?url={key}` (or `/art`): use one of
    /// the listed images.
    pub async fn select_image(&self, id: &str, kind: ImageKind, key: &str) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        if key.trim().is_empty() {
            return Err(AppError::Invalid("No image chosen.".into()));
        }
        let rb = self.request(Method::PUT, &format!("/library/metadata/{k}/{}", kind.one()))?.query(&[("url", key)]);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `POST /library/metadata/{id}/posters` (or `/arts`) with the image as
    /// the body. Plex selects it.
    pub async fn upload_image(&self, id: &str, kind: ImageKind, bytes: Vec<u8>, content_type: &str) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        let rb = self
            .request(Method::POST, &format!("/library/metadata/{k}/{}", kind.list()))?
            .header("content-type", content_type)
            .body(bytes);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `POST /library/metadata/{id}/posters?url={link}` (or `/arts`): the
    /// server fetches the image and selects it.
    pub async fn image_from_link(&self, id: &str, kind: ImageKind, link: &str) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        let rb = self.request(Method::POST, &format!("/library/metadata/{k}/{}", kind.list()))?.query(&[("url", link)]);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `GET /library/metadata/{id}/matches?manual=1&title=&year=`.
    pub async fn matches(&self, id: &str, title: Option<&str>, year: Option<i64>) -> AppResult<Vec<MatchCandidate>> {
        let k = rating_key(self.raw(id)?)?;
        let mut q: Vec<(&str, String)> = vec![("manual", "1".into())];
        if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
            q.push(("title", t.to_string()));
        }
        if let Some(y) = year {
            q.push(("year", y.to_string()));
        }
        let list: MatchList = self.edit_json(&format!("/library/metadata/{k}/matches"), &q).await?;
        Ok(list
            .results
            .into_iter()
            .filter(|m| !m.guid.is_empty())
            .map(|m| MatchCandidate { guid: m.guid, name: m.name, artist: m.parent_name.or(m.area), year: m.year, score: m.score })
            .collect())
    }

    /// `PUT /library/metadata/{id}/match?guid=&name=&year=`.
    pub async fn set_match(&self, id: &str, guid: &str, name: &str, year: Option<i64>) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        if guid.trim().is_empty() {
            return Err(AppError::Invalid("No match chosen.".into()));
        }
        let mut q: Vec<(&str, String)> = vec![("guid", guid.to_string()), ("name", name.to_string())];
        if let Some(y) = year {
            q.push(("year", y.to_string()));
        }
        let rb = self.request(Method::PUT, &format!("/library/metadata/{k}/match"))?.query(&q);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `PUT /library/metadata/{id}/unmatch`.
    pub async fn unmatch(&self, id: &str) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        let rb = self.request(Method::PUT, &format!("/library/metadata/{k}/unmatch"))?;
        self.edit_send(rb).await.map(|_| ())
    }

    /// `PUT /library/sections/{section}/all?type=&id=&<field>.value=&<field>.locked=`.
    pub async fn edit_fields(&self, section: &str, query: &[(String, String)]) -> AppResult<()> {
        let section = rating_key(section)?;
        let rb = self.request(Method::PUT, &format!("/library/sections/{section}/all"))?.query(query);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `PUT /library/metadata/{target}/merge?ids=a,b`: the others become part
    /// of `target`, which keeps its name and photo.
    pub async fn merge(&self, target: &str, others: &[String]) -> AppResult<()> {
        let t = rating_key(self.raw(target)?)?;
        let ids = others
            .iter()
            .map(|o| self.raw(o).and_then(rating_key).map(str::to_string))
            .collect::<AppResult<Vec<_>>>()?;
        if ids.is_empty() || ids.iter().any(|i| i == t) {
            return Err(AppError::Invalid("Pick another artist to merge with.".into()));
        }
        let rb = self.request(Method::PUT, &format!("/library/metadata/{t}/merge"))?.query(&[("ids", ids.join(","))]);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `PUT /library/metadata/{id}/split`.
    pub async fn split(&self, id: &str) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        let rb = self.request(Method::PUT, &format!("/library/metadata/{k}/split"))?;
        self.edit_send(rb).await.map(|_| ())
    }

    /// `GET /library/sections/{id}/refresh?path=`: scan one folder of a
    /// library section (after files in it changed).
    pub async fn scan_folder(&self, section: &str, path: &str) -> AppResult<()> {
        let rb = self.request(Method::GET, &format!("/library/sections/{section}/refresh"))?.query(&[("path", path)]);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `PUT /:/prefs?autoEmptyTrash=`: Plex's "Empty trash automatically
    /// after every scan". The only server setting Platter ever changes, and
    /// only for the length of a move it asked about first.
    pub async fn set_empty_trash(&self, on: bool) -> AppResult<()> {
        let rb = self.request(Method::PUT, "/:/prefs")?.query(&[("autoEmptyTrash", if on { "1" } else { "0" })]);
        self.edit_send(rb).await.map(|_| ())
    }

    /// `PUT /library/metadata/{id}/refresh`: read the agent and files again.
    pub async fn refresh_item(&self, id: &str) -> AppResult<()> {
        let k = rating_key(self.raw(id)?)?;
        let rb = self.request(Method::PUT, &format!("/library/metadata/{k}/refresh"))?;
        self.edit_send(rb).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wiremock::matchers::{body_bytes, header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::plex::identity::ClientIdentity;
    use crate::plex::token::Token;
    use crate::plex::types::ConnectionKind;

    const TAG: &str = "0a1b2c3d";

    fn client(server: &MockServer) -> PlexClient {
        PlexClient::new(&server.uri(), Token::new("tok").unwrap(), Arc::new(ClientIdentity::with_id("cid")), Some(ConnectionKind::Local))
            .unwrap()
    }

    fn ok() -> ResponseTemplate {
        ResponseTemplate::new(200)
    }

    #[tokio::test]
    async fn lists_posters_with_previews() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/5/posters"))
            .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"size": 2, "Metadata": [
                {"key": "/library/metadata/5/file?url=metadata%3A%2F%2Fposters%2Fx", "ratingKey": "metadata://posters/x",
                 "thumb": "/library/metadata/5/file?url=metadata%3A%2F%2Fposters%2Fx", "selected": true, "provider": "embedded"},
                {"key": "https://coverart.example/a.jpg", "ratingKey": "https://coverart.example/a.jpg",
                 "thumb": "https://images.plex.tv/photo?url=a", "selected": false, "provider": "caa"}
            ]}})))
            .mount(&s)
            .await;
        let list = client(&s).images("5", ImageKind::Poster).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].selected);
        assert_eq!(list[0].key, "metadata://posters/x");
        assert!(list[0].preview.as_deref().unwrap().starts_with("/library/metadata/5/file"));
        assert_eq!(list[1].preview.as_deref(), Some("/ext/https://images.plex.tv/photo?url=a"));
        assert_eq!(list[1].provider.as_deref(), Some("caa"));
    }

    #[tokio::test]
    async fn lists_artist_backgrounds_from_arts() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/9/arts"))
            .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"size": 0}})))
            .expect(1)
            .mount(&s)
            .await;
        assert!(client(&s).images("9", ImageKind::Art).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn uploads_selects_and_links_images() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/library/metadata/5/posters"))
            .and(header("content-type", "image/jpeg"))
            .and(body_bytes(vec![1u8, 2, 3]))
            .and(query_param_is_missing("url"))
            .respond_with(ok())
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("PUT"))
            .and(path("/library/metadata/5/poster"))
            .and(query_param("url", "metadata://posters/x"))
            .respond_with(ok())
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/library/metadata/5/arts"))
            .and(query_param("url", "https://example.com/b.png"))
            .respond_with(ok())
            .expect(1)
            .mount(&s)
            .await;
        let c = client(&s);
        c.upload_image("5", ImageKind::Poster, vec![1, 2, 3], "image/jpeg").await.unwrap();
        c.select_image("5", ImageKind::Poster, "metadata://posters/x").await.unwrap();
        c.image_from_link("5", ImageKind::Art, "https://example.com/b.png").await.unwrap();
    }

    #[tokio::test]
    async fn searches_and_sets_matches() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/5/matches"))
            .and(query_param("manual", "1"))
            .and(query_param("title", "Some Title"))
            .and(query_param("year", "2020"))
            .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"size": 1, "SearchResult": [
                {"type": "album", "guid": "mbid://abc", "name": "N", "parentName": "P", "score": "98", "year": 2020,
                 "SearchResult": [{"type": "track", "guid": "mbid://t"}]},
                {"type": "album", "guid": "", "name": "no guid"}
            ]}})))
            .mount(&s)
            .await;
        Mock::given(method("PUT"))
            .and(path("/library/metadata/5/match"))
            .and(query_param("guid", "mbid://abc"))
            .and(query_param("name", "N"))
            .and(query_param("year", "2020"))
            .respond_with(ok())
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("PUT")).and(path("/library/metadata/5/unmatch")).respond_with(ok()).expect(1).mount(&s).await;
        let c = client(&s);
        let m = c.matches("5", Some(" Some Title "), Some(2020)).await.unwrap();
        assert_eq!(
            m,
            vec![MatchCandidate { guid: "mbid://abc".into(), name: "N".into(), artist: Some("P".into()), year: Some(2020), score: Some(98) }]
        );
        c.set_match("5", "mbid://abc", "N", Some(2020)).await.unwrap();
        c.unmatch("5").await.unwrap();
    }

    #[tokio::test]
    async fn merges_splits_and_refreshes() {
        let s = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/library/metadata/7/merge"))
            .and(query_param("ids", "8,9"))
            .respond_with(ok())
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("PUT")).and(path("/library/metadata/7/split")).respond_with(ok()).expect(1).mount(&s).await;
        Mock::given(method("PUT")).and(path("/library/metadata/7/refresh")).respond_with(ok()).expect(1).mount(&s).await;
        let c = client(&s);
        c.merge("7", &["8".into(), "9".into()]).await.unwrap();
        assert!(c.merge("7", &["7".into()]).await.is_err(), "not with itself");
        assert!(c.merge("7", &[]).await.is_err());
        c.split("7").await.unwrap();
        c.refresh_item("7").await.unwrap();
    }

    #[tokio::test]
    async fn edits_fields_with_locks() {
        let s = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/library/sections/7/all"))
            .and(query_param("type", "9"))
            .and(query_param("id", "5"))
            .and(query_param("title.value", "New"))
            .and(query_param("title.locked", "1"))
            .and(query_param("genre[0].tag.tag", "Rock"))
            .and(query_param("genre[].tag.tag-", "Pop"))
            .and(query_param("genre.locked", "1"))
            .respond_with(ok())
            .expect(1)
            .mount(&s)
            .await;
        let from = Snapshot { title: "Old".into(), genres: vec!["Pop".into(), "Rock".into()], ..Default::default() };
        let to = Snapshot { title: "New".into(), genres: vec!["Rock".into()], ..Default::default() };
        let q = field_query(EditKind::Album, "5", &from, &to, true, None).unwrap();
        client(&s).edit_fields("7", &q).await.unwrap();
    }

    #[test]
    fn field_query_sends_only_changes_and_restores_locks() {
        let from = Snapshot { title: "A".into(), summary: "s".into(), released: "2020".into(), ..Default::default() };
        let to = Snapshot { title: "A".into(), summary: "t".into(), released: "2021-06-25".into(), ..Default::default() };
        let q = field_query(EditKind::Album, "5", &from, &to, false, None).unwrap();
        let has = |k: &str, v: &str| q.iter().any(|(a, b)| a == k && b == v);
        assert!(!q.iter().any(|(k, _)| k.starts_with("title")), "unchanged title left alone");
        assert!(has("summary.value", "t") && has("summary.locked", "0"));
        assert!(has("originallyAvailableAt.value", "2021-06-25") && has("year.value", "2021"));
        // Undo puts the old lock state back.
        let undo = field_query(EditKind::Album, "5", &to, &from, true, Some(&["summary".to_string()])).unwrap();
        assert!(undo.iter().any(|(k, v)| k == "summary.locked" && v == "1"));
        assert!(undo.iter().any(|(k, v)| k == "originallyAvailableAt.locked" && v == "0"));
        // Artists have no label or release date.
        let a = changed_fields(EditKind::Artist, &Snapshot::default(), &Snapshot { studio: "x".into(), released: "2020".into(), ..Default::default() });
        assert!(a.is_empty());
        // A title can't be emptied.
        assert!(field_query(EditKind::Album, "5", &from, &Snapshot { title: " ".into(), ..from.clone() }, true, None).is_err());
        assert!(parse_released("21").is_err() && parse_released("2021-13-01").is_err());
        assert_eq!(parse_released("").unwrap(), (None, None));
    }

    #[tokio::test]
    async fn reads_fields_and_locks() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/5"))
            .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [{
                "ratingKey": "5", "type": "album", "title": "T", "year": 2019, "studio": "L", "summary": "S",
                "guid": "local://5", "thumb": "/library/metadata/5/thumb/1", "librarySectionID": 7, "parentTitle": "A",
                "Genre": [{"tag": "Rock"}], "Style": [], "Mood": [{"tag": "Calm"}],
                "Field": [{"name": "title", "locked": true}, {"name": "summary", "locked": false}]
            }]}})))
            .mount(&s)
            .await;
        let it = client(&s).editable("5").await.unwrap();
        assert_eq!(it.kind, Some(EditKind::Album));
        assert_eq!(it.fields.released, "2019");
        assert_eq!(it.fields.genres, vec!["Rock"]);
        assert_eq!(it.locked, vec!["title"]);
        assert_eq!(it.section_id.as_deref(), Some("7"));
        assert!(!it.matched());
    }

    #[tokio::test]
    async fn refusals_and_size_limits_say_what_happened() {
        let s = MockServer::start().await;
        Mock::given(method("PUT")).and(path("/library/metadata/1/unmatch")).respond_with(ResponseTemplate::new(401)).mount(&s).await;
        Mock::given(method("PUT")).and(path("/library/metadata/2/unmatch")).respond_with(ResponseTemplate::new(403)).mount(&s).await;
        Mock::given(method("POST")).and(path("/library/metadata/3/posters")).respond_with(ResponseTemplate::new(413)).mount(&s).await;
        let c = client(&s);
        for id in ["1", "2"] {
            match c.unmatch(id).await {
                Err(AppError::Auth(m)) => assert_eq!(m, NO_PERMISSION),
                other => panic!("{other:?}"),
            }
        }
        assert!(matches!(c.upload_image("3", ImageKind::Poster, vec![0], "image/png").await, Err(AppError::Invalid(m)) if m.contains("too large")));
        // Server gone: a network error.
        let gone = PlexClient::new("http://127.0.0.1:1", Token::new("tok").unwrap(), Arc::new(ClientIdentity::with_id("cid")), None).unwrap();
        let e = gone.unmatch("1").await;
        assert!(matches!(e, Err(AppError::Network(_))), "{e:?}");
    }

    #[tokio::test]
    async fn tagged_ids_go_to_their_server_without_the_tag() {
        let s = MockServer::start().await;
        Mock::given(method("PUT")).and(path("/library/metadata/5/refresh")).respond_with(ok()).expect(1).mount(&s).await;
        let mid = "machine-one";
        let tag = crate::sid::tag(mid);
        let c = client(&s).with_server(mid);
        c.refresh_item(&format!("{tag}:5")).await.unwrap();
        // Another server's id is refused, not sent here.
        assert!(c.refresh_item(&format!("{TAG}:5")).await.is_err() || tag == TAG);
        assert!(c.refresh_item("abc").await.is_err());
    }
}
