//! Sharing a server's music libraries through plex.tv: the server's
//! sections as plex.tv numbers them, sharing them with a Plex user (by
//! username or email), the people a server is shared with, removing a
//! share, the invites this account received, accepting one, and whether
//! the account has Plex Pass (label restrictions need it).
//!
//! These are plex.tv's older `/api` endpoints (docs/PLEX-API.md, "Sharing a
//! server"), which answer XML. Every share Platter makes lists music
//! sections only; [`PlexTv::share_server`] refuses anything else, so a movie
//! or photo library is never shared from here. Tokens in the answers
//! (`accessToken`, `authToken`) are dropped while reading and never kept.
//!
//! Album labels (the server side of "Hide my unreleased music") are read and
//! written on the Plex server, not plex.tv: [`PlexClient::labels`] and
//! [`PlexClient::add_label`].

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Method;
use serde::{Deserialize, Serialize};

use super::auth::PlexTv;
use super::client::{PlexClient, map_send_error, status_error};
use super::token::Token;
use crate::error::{AppError, AppResult};

/// Plex's section type for music.
pub const MUSIC: &str = "artist";

// ---------------------------------------------------------------- XML

/// One element of an answer: its name, attributes and parent (an index).
#[derive(Debug, Clone)]
struct El {
    name: String,
    attrs: HashMap<String, String>,
    parent: Option<usize>,
}

impl El {
    fn get(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(String::as_str).filter(|v| !v.is_empty())
    }
    fn s(&self, k: &str) -> String {
        self.get(k).unwrap_or_default().to_string()
    }
    fn flag(&self, k: &str) -> bool {
        matches!(self.get(k), Some("1") | Some("true"))
    }
    fn num(&self, k: &str) -> Option<i64> {
        self.get(k).and_then(|v| v.parse().ok())
    }
}

/// Attributes that carry a token: never read.
fn secret_attr(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    k.contains("token") || k == "authenticationtoken"
}

/// Every element of an XML answer, in document order.
fn parse(xml: &str) -> AppResult<Vec<El>> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out: Vec<El> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    loop {
        let ev = r.read_event().map_err(|_| AppError::Decode("plex.tv sent unreadable XML".into()))?;
        let (start, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(_) => {
                stack.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let Some(e) = start else { continue };
        let name = e.name().0.to_string();
        let mut attrs = HashMap::new();
        for a in e.attributes().flatten() {
            let k = a.key.0.to_string();
            if secret_attr(&k) {
                continue;
            }
            let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default();
            attrs.insert(k, v);
        }
        out.push(El { name, attrs, parent: stack.last().copied() });
        if !empty {
            stack.push(out.len() - 1);
        }
    }
    Ok(out)
}

fn children<'a>(els: &'a [El], parent: usize, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
    els.iter().filter(move |e| e.parent == Some(parent) && e.name == name)
}

// ---------------------------------------------------------------- shapes

/// A library on a server as plex.tv numbers it. Shares name sections by
/// plex.tv's `id`, not the server's own `key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvSection {
    pub id: String,
    /// The server's section id (`/library/sections/{key}`).
    pub key: String,
    pub kind: String,
    pub title: String,
}

impl TvSection {
    pub fn is_music(&self) -> bool {
        self.kind == MUSIC
    }
}

/// What a new share allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareTerms {
    /// A Plex username or email.
    pub invitee: String,
    /// The server's section keys (music only).
    pub sections: Vec<String>,
    /// "Allow downloads" (Plex's allowSync).
    pub allow_downloads: bool,
    /// Albums with these labels are kept out (`label!=`); needs Plex Pass.
    pub exclude_labels: Vec<String>,
}

/// Someone a server is shared with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedWith {
    /// The share's id (for removing it).
    pub id: String,
    pub user_id: String,
    pub name: String,
    /// The server's section keys they can see.
    pub sections: Vec<String>,
    pub allow_downloads: bool,
    /// The music filter as Plex keeps it (`label!=...`).
    pub filter_music: String,
    /// Still waiting for them to accept.
    pub pending: bool,
}

/// An invite this account received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedInvite {
    pub id: String,
    /// Who sent it (their Plex name).
    pub from: String,
    /// The servers it shares, by name.
    pub servers: Vec<String>,
    pub friend: bool,
    pub home: bool,
    pub server: bool,
    /// Unix seconds.
    pub created_at: Option<i64>,
}

/// When each friend last used a server (`/api/users`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastSeen {
    pub user_id: String,
    pub machine_id: String,
    /// Unix seconds.
    pub at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSubscription {
    #[serde(default)]
    active: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RawUserPass {
    #[serde(default)]
    subscription: Option<RawSubscription>,
    #[serde(default)]
    roles: Vec<serde_json::Value>,
}

/// The music filter that keeps albums with these labels out.
pub fn exclude_filter(labels: &[String]) -> String {
    let vals: Vec<String> = labels.iter().map(|l| url::form_urlencoded::byte_serialize(l.as_bytes()).collect()).collect();
    if vals.is_empty() { String::new() } else { format!("label!={}", vals.join("%2C")) }
}

fn ok_machine(m: &str) -> bool {
    !m.is_empty() && m.len() <= 100 && m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn ok_num(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Why plex.tv turned a share down, in words for the host.
fn share_refusal(code: u16, body: &str) -> AppError {
    let low = body.to_ascii_lowercase();
    if low.contains("already") {
        return AppError::Invalid("ALREADY_SHARED".into());
    }
    match code {
        401 | 403 => AppError::Auth("Plex rejected the sign-in. Please sign in again.".into()),
        404 => AppError::NotFound("Plex doesn't know that username or email.".into()),
        400 | 409 | 422 => AppError::Invalid(if low.contains("user") || low.contains("email") {
            "Plex doesn't know that username or email.".into()
        } else {
            "Plex turned the invite down.".into()
        }),
        500..=599 => AppError::Server("Plex isn't answering right now. Try again in a while.".into()),
        _ => AppError::Invalid("Plex turned the invite down.".into()),
    }
}

/// Whether an error is plex.tv saying the person already has access.
pub fn is_already_shared(e: &AppError) -> bool {
    matches!(e, AppError::Invalid(m) if m == "ALREADY_SHARED")
}

impl PlexTv {
    async fn tv_xml(&self, method: Method, path: &str, token: &Token, query: &[(&str, String)]) -> AppResult<Vec<El>> {
        let resp = self
            .http()
            .request(method, self.url_for(path)?)
            .query(query)
            .header("x-plex-token", token.header_value()?)
            .header("accept", "application/xml")
            .send()
            .await
            .map_err(map_send_error)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(status_error(status, ""));
        }
        let text = resp.text().await.map_err(map_send_error)?;
        if text.trim().is_empty() { Ok(Vec::new()) } else { parse(&text) }
    }

    /// `GET /api/v2/user`: whether the account has Plex Pass (an active
    /// subscription, or the `plexpass` role).
    pub async fn plex_pass(&self, token: &Token) -> AppResult<bool> {
        let resp = self
            .http()
            .get(self.url_for("/api/v2/user")?)
            .header("x-plex-token", token.header_value()?)
            .send()
            .await
            .map_err(map_send_error)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(status_error(status, ""));
        }
        let u: RawUserPass = resp.json().await.map_err(map_send_error)?;
        let role = u.roles.iter().any(|r| r.get("id").and_then(|v| v.as_str()).or(r.as_str()) == Some("plexpass"));
        Ok(u.subscription.is_some_and(|s| s.active) || role)
    }

    /// `GET /api/servers/{machine}`: the server's libraries with plex.tv's
    /// ids for them.
    pub async fn server_sections(&self, token: &Token, machine: &str) -> AppResult<Vec<TvSection>> {
        if !ok_machine(machine) {
            return Err(AppError::Invalid("Unknown server.".into()));
        }
        let els = self.tv_xml(Method::GET, &format!("/api/servers/{machine}"), token, &[]).await?;
        Ok(els
            .iter()
            .filter(|e| e.name == "Section")
            .map(|e| TvSection { id: e.s("id"), key: e.s("key"), kind: e.s("type"), title: e.s("title") })
            .filter(|s| !s.id.is_empty() && !s.key.is_empty())
            .collect())
    }

    /// `POST /api/servers/{machine}/shared_servers`: share these music
    /// sections with a Plex user. Anything but music is refused before
    /// plex.tv is asked. plex.tv emails the invite too; it shows in their
    /// Platter as a [`ReceivedInvite`].
    pub async fn share_server(&self, token: &Token, machine: &str, terms: &ShareTerms) -> AppResult<()> {
        let invitee = terms.invitee.trim();
        if invitee.is_empty() || invitee.len() > 200 || invitee.chars().any(char::is_control) {
            return Err(AppError::Invalid("Type their Plex username or email.".into()));
        }
        if terms.sections.is_empty() {
            return Err(AppError::Invalid("Pick a music library to share.".into()));
        }
        let all = self.server_sections(token, machine).await?;
        let mut ids = Vec::new();
        for key in &terms.sections {
            let s = all.iter().find(|s| &s.key == key).ok_or_else(|| AppError::NotFound("That library isn't on your server anymore.".into()))?;
            if !s.is_music() {
                return Err(AppError::Invalid("Only music libraries can be shared from Platter.".into()));
            }
            ids.push(s.id.parse::<i64>().map_err(|_| AppError::Decode("plex.tv sent an odd library id".into()))?);
        }
        let body = serde_json::json!({
            "server_id": machine,
            "shared_server": { "library_section_ids": ids, "invited_email": invitee },
            "sharing_settings": {
                "allowSync": if terms.allow_downloads { "1" } else { "0" },
                "allowCameraUpload": "0",
                "allowChannels": "0",
                "filterMovies": "",
                "filterTelevision": "",
                "filterMusic": exclude_filter(&terms.exclude_labels),
            },
        });
        let resp = self
            .http()
            .post(self.url_for(&format!("/api/servers/{machine}/shared_servers"))?)
            .header("x-plex-token", token.header_value()?)
            .json(&body)
            .send()
            .await
            .map_err(map_send_error)?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(share_refusal(status.as_u16(), &text))
    }

    /// `GET /api/servers/{machine}/shared_servers`: who the server is shared
    /// with and what each one sees.
    pub async fn shares(&self, token: &Token, machine: &str) -> AppResult<Vec<SharedWith>> {
        if !ok_machine(machine) {
            return Err(AppError::Invalid("Unknown server.".into()));
        }
        let els = self.tv_xml(Method::GET, &format!("/api/servers/{machine}/shared_servers"), token, &[]).await?;
        Ok(els
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name == "SharedServer")
            .map(|(i, e)| {
                let name = [e.get("username"), e.get("title"), e.get("email")].into_iter().flatten().find(|s| !s.trim().is_empty()).unwrap_or("Plex user");
                SharedWith {
                    id: e.s("id"),
                    user_id: e.s("userID"),
                    name: name.to_string(),
                    sections: children(&els, i, "Section").filter(|s| s.flag("shared")).map(|s| s.s("key")).collect(),
                    allow_downloads: e.flag("allowSync"),
                    filter_music: e.s("filterMusic"),
                    pending: e.get("acceptedAt").is_none_or(|a| a == "0"),
                }
            })
            .filter(|s| ok_num(&s.id))
            .collect())
    }

    /// `DELETE /api/servers/{machine}/shared_servers/{id}`: they lose access
    /// to the server (they stay a Plex friend).
    pub async fn unshare(&self, token: &Token, machine: &str, share_id: &str) -> AppResult<()> {
        if !ok_machine(machine) || !ok_num(share_id) {
            return Err(AppError::Invalid("Unknown share.".into()));
        }
        match self.tv_xml(Method::DELETE, &format!("/api/servers/{machine}/shared_servers/{share_id}"), token, &[]).await {
            Ok(_) | Err(AppError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `GET /api/users`: when each friend last used each server.
    pub async fn last_seen(&self, token: &Token) -> AppResult<Vec<LastSeen>> {
        let els = self.tv_xml(Method::GET, "/api/users", token, &[]).await?;
        let mut out = Vec::new();
        for (i, u) in els.iter().enumerate().filter(|(_, e)| e.name == "User") {
            for s in children(&els, i, "Server") {
                out.push(LastSeen { user_id: u.s("id"), machine_id: s.s("machineIdentifier"), at: s.num("lastSeenAt").filter(|t| *t > 0) });
            }
        }
        Ok(out)
    }

    /// `GET /api/invites/requests`: invites this account received.
    pub async fn invites_received(&self, token: &Token) -> AppResult<Vec<ReceivedInvite>> {
        let els = self.tv_xml(Method::GET, "/api/invites/requests", token, &[]).await?;
        Ok(els
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name == "Invite")
            .map(|(i, e)| {
                let from = [e.get("friendlyName"), e.get("username"), e.get("title"), e.get("email")]
                    .into_iter()
                    .flatten()
                    .find(|s| !s.trim().is_empty())
                    .unwrap_or("Someone");
                ReceivedInvite {
                    id: e.s("id"),
                    from: from.chars().take(60).collect(),
                    servers: children(&els, i, "Server").map(|s| s.s("name")).filter(|n| !n.is_empty()).collect(),
                    friend: e.flag("friend"),
                    home: e.flag("home"),
                    server: e.flag("server"),
                    created_at: e.num("createdAt"),
                }
            })
            .filter(|i| ok_num(&i.id))
            .collect())
    }

    /// `PUT /api/invites/requests/{id}?friend=&home=&server=`: accept it.
    pub async fn accept_invite(&self, token: &Token, invite: &ReceivedInvite) -> AppResult<()> {
        if !ok_num(&invite.id) {
            return Err(AppError::Invalid("Unknown invite.".into()));
        }
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        let q = [("friend", b(invite.friend)), ("home", b(invite.home)), ("server", b(invite.server))];
        match self.tv_xml(Method::PUT, &format!("/api/invites/requests/{}", invite.id), token, &q).await {
            Ok(_) => Ok(()),
            Err(AppError::NotFound(_)) => Err(AppError::NotFound("That invite is gone. It may have been taken back.".into())),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------- labels

#[derive(Debug, Default, Deserialize)]
struct RawLabels {
    #[serde(rename = "MediaContainer", default)]
    mc: RawLabelsMc,
}

#[derive(Debug, Default, Deserialize)]
struct RawLabelsMc {
    #[serde(rename = "Metadata", default)]
    metadata: Vec<RawLabelItem>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLabelItem {
    #[serde(rename = "Label", default)]
    label: Vec<RawTagName>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTagName {
    #[serde(default)]
    tag: String,
}

impl PlexClient {
    /// `GET /library/metadata/{id}`: the album's labels.
    pub async fn labels(&self, album_id: &str) -> AppResult<Vec<String>> {
        let k = crate::sid::key(album_id);
        if !ok_num(k) {
            return Err(AppError::Invalid("Not a Plex album.".into()));
        }
        let resp = self.request(Method::GET, &format!("/library/metadata/{k}"))?.send().await.map_err(map_send_error)?;
        let resp = super::edit::edit_status(resp)?;
        let raw: RawLabels = resp.json().await.map_err(map_send_error)?;
        Ok(raw.mc.metadata.into_iter().next().map(|m| m.label.into_iter().map(|t| t.tag).filter(|t| !t.trim().is_empty()).collect()).unwrap_or_default())
    }

    /// Add a label to an album, keeping the ones it has:
    /// `PUT /library/sections/{s}/all?type=9&id={k}&label[N].tag.tag=...&label.locked=1`.
    /// Nothing is sent when it already has the label.
    pub async fn add_label(&self, section: &str, album_id: &str, label: &str) -> AppResult<bool> {
        let k = crate::sid::key(album_id);
        if !ok_num(k) || !ok_num(section) {
            return Err(AppError::Invalid("Not a Plex album.".into()));
        }
        let mut have = self.labels(album_id).await?;
        if have.iter().any(|l| l.eq_ignore_ascii_case(label)) {
            return Ok(false);
        }
        have.push(label.to_string());
        let mut q: Vec<(String, String)> = vec![("type".into(), "9".into()), ("id".into(), k.to_string())];
        for (i, t) in have.iter().enumerate() {
            q.push((format!("label[{i}].tag.tag"), t.clone()));
        }
        q.push(("label.locked".into(), "1".into()));
        let resp = self.request(Method::PUT, &format!("/library/sections/{section}/all"))?.query(&q).send().await.map_err(map_send_error)?;
        super::edit::edit_status(resp)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::plex::identity::ClientIdentity;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "fake-account-token";
    const MACHINE: &str = "0123456789abcdef0123456789abcdef01234567";

    fn tv(server: &MockServer) -> PlexTv {
        PlexTv::with_base(Arc::new(ClientIdentity::with_id("cid")), &server.uri()).unwrap()
    }

    fn token() -> Token {
        Token::new(TOKEN).unwrap()
    }

    pub(crate) const SERVER_XML: &str = r#"<MediaContainer friendlyName="myPlex" size="1">
  <Server name="Home" machineIdentifier="0123456789abcdef0123456789abcdef01234567" accessToken="fake-server-token">
    <Section id="901" key="3" type="artist" title="Music"/>
    <Section id="902" key="4" type="movie" title="Films"/>
    <Section id="903" key="7" type="artist" title="Archive"/>
    <Section id="904" key="9" type="photo" title="Photos"/>
  </Server>
</MediaContainer>"#;

    async fn sections(s: &MockServer) {
        Mock::given(method("GET"))
            .and(path(format!("/api/servers/{MACHINE}")))
            .and(header("x-plex-token", TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_string(SERVER_XML))
            .mount(s)
            .await;
    }

    fn terms(sections: &[&str], labels: &[&str]) -> ShareTerms {
        ShareTerms {
            invitee: "robin_listens".into(),
            sections: sections.iter().map(|s| s.to_string()).collect(),
            allow_downloads: false,
            exclude_labels: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn sections_are_read_without_the_servers_token() {
        let s = MockServer::start().await;
        sections(&s).await;
        let got = tv(&s).server_sections(&token(), MACHINE).await.unwrap();
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], TvSection { id: "901".into(), key: "3".into(), kind: "artist".into(), title: "Music".into() });
        assert!(!format!("{got:?}").contains("fake-server-token"));
        let els = parse(SERVER_XML).unwrap();
        assert!(els.iter().all(|e| !e.attrs.contains_key("accessToken")), "tokens are dropped while reading");
    }

    #[tokio::test]
    async fn a_share_names_plex_tv_section_ids_and_the_label_filter() {
        let s = MockServer::start().await;
        sections(&s).await;
        Mock::given(method("POST"))
            .and(path(format!("/api/servers/{MACHINE}/shared_servers")))
            .and(header("x-plex-token", TOKEN))
            .and(body_json(serde_json::json!({
                "server_id": MACHINE,
                "shared_server": { "library_section_ids": [901, 903], "invited_email": "robin_listens" },
                "sharing_settings": {
                    "allowSync": "0", "allowCameraUpload": "0", "allowChannels": "0",
                    "filterMovies": "", "filterTelevision": "",
                    "filterMusic": "label!=platter-unreleased",
                },
            })))
            .respond_with(ResponseTemplate::new(200).set_body_string("<MediaContainer/>"))
            .expect(1)
            .mount(&s)
            .await;
        tv(&s).share_server(&token(), MACHINE, &terms(&["3", "7"], &["platter-unreleased"])).await.unwrap();
    }

    #[tokio::test]
    async fn only_music_libraries_are_ever_shared() {
        let s = MockServer::start().await;
        sections(&s).await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&s).await;
        for bad in [&["3", "4"][..], &["9"], &[]] {
            let e = tv(&s).share_server(&token(), MACHINE, &terms(bad, &[])).await.unwrap_err();
            assert_eq!(e.kind(), "invalid", "{bad:?}: {e}");
        }
        let e = tv(&s).share_server(&token(), MACHINE, &terms(&["55"], &[])).await.unwrap_err();
        assert_eq!(e.kind(), "notFound");
    }

    #[tokio::test]
    async fn refused_already_shared_and_plex_down_are_told_apart() {
        for (code, body, kind, already) in [
            (400, "<Response code=\"400\" status=\"User is already sharing this server\"/>", "invalid", true),
            (422, "<Response status=\"Invalid user\"/>", "invalid", false),
            (404, "", "notFound", false),
            (503, "", "server", false),
        ] {
            let s = MockServer::start().await;
            sections(&s).await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(code).set_body_string(body)).mount(&s).await;
            let e = tv(&s).share_server(&token(), MACHINE, &terms(&["3"], &[])).await.unwrap_err();
            assert_eq!((e.kind(), is_already_shared(&e)), (kind, already), "{code}");
        }
        // plex.tv not there at all.
        let tv = PlexTv::with_base(Arc::new(ClientIdentity::with_id("cid")), "http://127.0.0.1:9").unwrap();
        assert_eq!(tv.share_server(&token(), MACHINE, &terms(&["3"], &[])).await.unwrap_err().kind(), "network");
    }

    #[tokio::test]
    async fn shares_list_who_sees_what_and_removing_one_deletes_it() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/servers/{MACHINE}/shared_servers")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer size="2">
  <SharedServer id="5001" username="robin_listens" email="robin@example.com" userID="71" accessToken="fake-friend-token" name="Home" acceptedAt="1760000000" invitedAt="1759990000" allowSync="0" filterMusic="label!=platter-unreleased">
    <Section id="901" key="3" title="Music" type="artist" shared="1"/>
    <Section id="903" key="7" title="Archive" type="artist" shared="0"/>
  </SharedServer>
  <SharedServer id="5002" username="" email="jules@example.com" userID="72" acceptedAt="" allowSync="1">
    <Section id="901" key="3" title="Music" type="artist" shared="1"/>
  </SharedServer>
</MediaContainer>"#,
            ))
            .mount(&s)
            .await;
        Mock::given(method("DELETE")).and(path(format!("/api/servers/{MACHINE}/shared_servers/5001"))).respond_with(ResponseTemplate::new(200)).expect(1).mount(&s).await;
        let got = tv(&s).shares(&token(), MACHINE).await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "robin_listens");
        assert_eq!(got[0].sections, vec!["3".to_string()]);
        assert!(!got[0].pending && got[1].pending && got[1].allow_downloads);
        assert_eq!(got[1].name, "jules@example.com");
        assert!(!format!("{got:?}").contains("fake-friend-token"));
        tv(&s).unshare(&token(), MACHINE, "5001").await.unwrap();
        assert!(tv(&s).unshare(&token(), MACHINE, "../x").await.is_err());
    }

    #[tokio::test]
    async fn received_invites_are_listed_and_accepted_with_their_flags() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/invites/requests"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer size="1"><Invite id="8001" createdAt="1760000000" friend="1" home="0" server="1" username="alex_hosts" friendlyName="Alex" email="alex@example.com"><Server name="Alex's Music" numLibraries="1"/></Invite></MediaContainer>"#,
            ))
            .mount(&s)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/invites/requests/8001"))
            .and(query_param("friend", "1"))
            .and(query_param("home", "0"))
            .and(query_param("server", "1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&s)
            .await;
        let got = tv(&s).invites_received(&token()).await.unwrap();
        assert_eq!(got, vec![ReceivedInvite { id: "8001".into(), from: "Alex".into(), servers: vec!["Alex's Music".into()], friend: true, home: false, server: true, created_at: Some(1_760_000_000) }]);
        tv(&s).accept_invite(&token(), &got[0]).await.unwrap();
    }

    #[tokio::test]
    async fn plex_pass_comes_from_the_subscription_or_the_role() {
        for (body, want) in [
            (serde_json::json!({"subscription": {"active": true, "status": "Active"}}), true),
            (serde_json::json!({"subscription": {"active": false}, "roles": [{"id": "plexpass"}]}), true),
            (serde_json::json!({"subscription": {"active": false, "status": "Inactive"}}), false),
            (serde_json::json!({}), false),
        ] {
            let s = MockServer::start().await;
            Mock::given(method("GET")).and(path("/api/v2/user")).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(&s).await;
            assert_eq!(tv(&s).plex_pass(&token()).await.unwrap(), want);
        }
    }

    #[tokio::test]
    async fn last_seen_is_read_per_server() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/users"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"<MediaContainer><User id="71" username="robin_listens"><Server id="1" machineIdentifier="{MACHINE}" lastSeenAt="1760100000" numLibraries="1"/></User><User id="72"><Server machineIdentifier="{MACHINE}" lastSeenAt=""/></User></MediaContainer>"#
            )))
            .mount(&s)
            .await;
        let got = tv(&s).last_seen(&token()).await.unwrap();
        assert_eq!(got[0], LastSeen { user_id: "71".into(), machine_id: MACHINE.into(), at: Some(1_760_100_000) });
        assert_eq!(got[1].at, None);
    }

    #[tokio::test]
    async fn a_label_is_added_next_to_the_ones_an_album_has() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/77"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [{"ratingKey": "77", "Label": [{"tag": "Vinyl"}]}]}})))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/78"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [{"ratingKey": "78", "Label": [{"tag": "platter-unreleased"}]}]}})))
            .mount(&s)
            .await;
        Mock::given(method("PUT"))
            .and(path("/library/sections/3/all"))
            .and(query_param("type", "9"))
            .and(query_param("id", "77"))
            .and(query_param("label[0].tag.tag", "Vinyl"))
            .and(query_param("label[1].tag.tag", "platter-unreleased"))
            .and(query_param("label.locked", "1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&s)
            .await;
        let c = PlexClient::new(&s.uri(), token(), Arc::new(ClientIdentity::with_id("cid")), None).unwrap();
        assert!(c.add_label("3", "77", "platter-unreleased").await.unwrap());
        assert!(!c.add_label("3", "78", "platter-unreleased").await.unwrap(), "already labelled: nothing written");
    }

    #[test]
    fn the_exclusion_filter_is_plexs_label_not_equal() {
        assert_eq!(exclude_filter(&["platter-unreleased".into()]), "label!=platter-unreleased");
        assert_eq!(exclude_filter(&["a b".into(), "c".into()]), "label!=a+b%2Cc");
        assert_eq!(exclude_filter(&[]), "");
    }
}
