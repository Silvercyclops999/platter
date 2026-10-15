//! More than one library (Settings, Library).
//!
//! The connected server's main library is always in. Any other music
//! library the account can reach, on the same server or another one, owned
//! or shared, can be included too; Settings keeps them as
//! `<machine id>/<section id>`. Each included library syncs into the cache
//! with its own `src` (db::sync), its ids carrying its server (crate::sid),
//! and db::merge finds what's on more than one. With more than one server
//! included, the sidebar shows a switcher: All servers (merged) or one.
//!
//! Requests for an item go to its server (`PlexState::client_for`). How each
//! server is reached right now (local, remote, relay) is kept fresh here for
//! choosing between copies of a song.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tauri::State;

use crate::db::DbHandle;
use crate::db::merge::{Rank, ServerView};
use crate::db::store;
use crate::db::sync::ExtraLibrary;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::plex::types::ConnectionKind;

/// Most libraries that can be included besides the main one.
pub const MAX_LIBRARIES: usize = 32;
/// How often the connection to each included server is looked at.
const RANK_EVERY: Duration = Duration::from_secs(90);

/// A saved entry: the server's machine id and the section id.
pub fn parse_entry(e: &str) -> Option<(&str, &str)> {
    let (m, s) = e.split_once('/')?;
    let machine_ok = !m.is_empty() && m.len() <= 100 && m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let section_ok = !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit());
    (machine_ok && section_ok).then_some((m, s))
}

/// The entries as saved: checked, without repeats, at most [`MAX_LIBRARIES`].
pub fn tidy(v: &[String]) -> AppResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in v {
        let e = e.trim();
        if parse_entry(e).is_none() {
            return Err(AppError::Invalid("That isn't a music library on your servers.".into()));
        }
        if seen.insert(e.to_string()) {
            out.push(e.to_string());
        }
    }
    if out.len() > MAX_LIBRARIES {
        return Err(AppError::Invalid(format!("Up to {MAX_LIBRARIES} libraries can be included.")));
    }
    Ok(out)
}

/// The sync sources for the saved entries.
pub fn extra_libraries(plex: &PlexState, entries: &[String]) -> Vec<ExtraLibrary> {
    entries
        .iter()
        .filter_map(|e| parse_entry(e))
        .map(|(machine, section)| {
            let tag = crate::sid::tag(machine);
            ExtraLibrary {
                src: store::src_of(&tag, section),
                source: Arc::new(crate::bridge::PlexLibrary::other(plex.clone(), &tag, section)),
                tag,
            }
        })
        .collect()
}

// ---------------------------------------------------------------- listing

/// A server on the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEntry {
    pub machine_id: String,
    pub name: String,
    pub owned: bool,
}

/// A music library on a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: String,
    pub title: String,
}

/// A server's music libraries and how it's reached now.
pub type Sections = (Vec<Section>, Option<ConnectionKind>);

/// The account's servers as Plex has them. The app uses [`PlexDirectory`];
/// tests use a fake.
#[async_trait]
pub trait Directory: Send + Sync {
    async fn servers(&self) -> Vec<ServerEntry>;
    /// The connected server's tag.
    async fn current_tag(&self) -> Option<String>;
    /// A server's music libraries and how it's reached now.
    async fn sections(&self, tag: &str) -> AppResult<Sections>;
}

pub struct PlexDirectory(pub PlexState);

#[async_trait]
impl Directory for PlexDirectory {
    async fn servers(&self) -> Vec<ServerEntry> {
        self.0
            .account_servers()
            .await
            .into_iter()
            .map(|s| ServerEntry { machine_id: s.machine_id, name: s.name, owned: s.owned })
            .collect()
    }
    async fn current_tag(&self) -> Option<String> {
        self.0.tag().await
    }
    async fn sections(&self, tag: &str) -> AppResult<Sections> {
        let c = self.0.client_for_tag(tag).await?;
        let list = c.music_sections().await?.into_iter().map(|s| Section { id: s.id, title: s.title }).collect();
        Ok((list, c.connection()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLibrary {
    pub id: String,
    pub title: String,
    pub included: bool,
    /// The connected server's main library: always included.
    pub main: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceServer {
    pub machine_id: String,
    pub tag: String,
    pub name: String,
    pub owned: bool,
    /// The server Platter is connected to.
    pub current: bool,
    pub online: bool,
    pub connection: Option<ConnectionKind>,
    pub libraries: Vec<SourceLibrary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySources {
    pub servers: Vec<SourceServer>,
}

/// Every server and music library the account can reach, which are
/// included, and how each server is reached. Servers are asked at once; one
/// that doesn't answer is listed offline with the libraries included on it.
pub async fn sources(dir: &dyn Directory, included: &[String], main_section: Option<&str>) -> (LibrarySources, Vec<(String, Rank)>) {
    let current = dir.current_tag().await;
    let servers = dir.servers().await;
    let asks = servers.iter().map(|s| {
        let tag = crate::sid::tag(&s.machine_id);
        async move { dir.sections(&tag).await }
    });
    let answers = futures::future::join_all(asks).await;
    let mut out = Vec::new();
    let mut ranks = Vec::new();
    for (s, answer) in servers.into_iter().zip(answers) {
        let tag = crate::sid::tag(&s.machine_id);
        let is_current = current.as_deref() == Some(tag.as_str());
        let entry = |section: &str| format!("{}/{section}", s.machine_id);
        let (online, connection, sections) = match answer {
            Ok((list, conn)) => (true, conn, list),
            Err(e) => {
                tracing::debug!(server = %s.name, "libraries not listed: {e}");
                // Keep what's included visible so it can be left out.
                let kept = included
                    .iter()
                    .filter_map(|e| parse_entry(e))
                    .filter(|(m, _)| *m == s.machine_id)
                    .map(|(_, id)| Section { id: id.to_string(), title: "Music".into() })
                    .collect();
                (false, None, kept)
            }
        };
        ranks.push((tag.clone(), if online { Rank::of(connection) } else { Rank::Unknown }));
        let libraries = sections
            .into_iter()
            .map(|sec| {
                let main = is_current && main_section == Some(sec.id.as_str());
                SourceLibrary { included: main || included.contains(&entry(&sec.id)), main, id: sec.id, title: sec.title }
            })
            .collect();
        out.push(SourceServer {
            machine_id: s.machine_id,
            tag,
            name: s.name,
            owned: s.owned,
            current: is_current,
            online,
            connection,
            libraries,
        });
    }
    // The connected server first, then owned ones, then by name.
    out.sort_by(|a, b| b.current.cmp(&a.current).then(b.owned.cmp(&a.owned)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    (LibrarySources { servers: out }, ranks)
}

/// The servers the library has music from: the connected one and every
/// server with an included library, by tag and name, for the switcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryServers {
    pub servers: Vec<NamedServer>,
    /// "all" or a server tag.
    pub view: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedServer {
    pub tag: String,
    pub name: String,
}

pub fn included_servers(all: &[ServerEntry], current: Option<&str>, included: &[String], view: &ServerView) -> LibraryServers {
    let mut tags: Vec<String> = current.map(str::to_string).into_iter().collect();
    for (m, _) in included.iter().filter_map(|e| parse_entry(e)) {
        let t = crate::sid::tag(m);
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    let servers: Vec<NamedServer> = tags
        .into_iter()
        .map(|tag| {
            let name = all
                .iter()
                .find(|s| crate::sid::tag(&s.machine_id) == tag)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Plex server".into());
            NamedServer { tag, name }
        })
        .collect();
    // A view of a server no longer included falls back to all of them.
    let view = match view {
        ServerView::Server(t) if servers.len() > 1 && servers.iter().any(|s| &s.tag == t) => t.clone(),
        _ => "all".into(),
    };
    LibraryServers { servers, view }
}

/// Keep how each included server is reached up to date (for choosing
/// between copies of a song). Only does anything with other libraries
/// included.
pub fn spawn_ranks(plex: PlexState, db: DbHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let extras = db.extra_libraries();
            if !extras.is_empty() {
                let mut tags: Vec<String> = plex.tag().await.into_iter().collect();
                for l in extras {
                    if !tags.contains(&l.tag) {
                        tags.push(l.tag);
                    }
                }
                for t in tags {
                    let rank = match plex.client_for_tag(&t).await {
                        Ok(c) => Rank::of(c.connection()),
                        Err(_) => Rank::Unknown,
                    };
                    db.set_source_rank(&t, Some(rank));
                }
            }
            tokio::time::sleep(RANK_EVERY).await;
        }
    });
}

// ---------------------------------------------------------------- commands

fn main_section(db: &DbHandle) -> Option<String> {
    db.read(store::sync_state).ok().flatten().map(|s| s.section_id).or_else(|| crate::db::sync::selected_section(db))
}

/// Settings, Library: every server and music library, which are included.
#[tauri::command]
pub async fn library_sources(
    plex: State<'_, PlexState>,
    db: State<'_, DbHandle>,
    settings: State<'_, crate::settings::SettingsState>,
) -> AppResult<LibrarySources> {
    let s = settings.get().await?;
    let main = main_section(db.inner());
    let (out, ranks) = sources(&PlexDirectory(plex.inner().clone()), &s.libraries, main.as_deref()).await;
    for (tag, rank) in ranks {
        db.set_source_rank(&tag, Some(rank));
    }
    Ok(out)
}

/// The sidebar switcher: the servers the library has music from.
#[tauri::command]
pub async fn library_servers(
    plex: State<'_, PlexState>,
    db: State<'_, DbHandle>,
    settings: State<'_, crate::settings::SettingsState>,
) -> AppResult<LibraryServers> {
    let s = settings.get().await?;
    let all = PlexDirectory(plex.inner().clone()).servers().await;
    Ok(included_servers(&all, plex.tag().await.as_deref(), &s.libraries, &db.server_view()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeDir {
        servers: Vec<ServerEntry>,
        current: Option<String>,
        sections: HashMap<String, AppResult<Sections>>,
    }

    #[async_trait]
    impl Directory for FakeDir {
        async fn servers(&self) -> Vec<ServerEntry> {
            self.servers.clone()
        }
        async fn current_tag(&self) -> Option<String> {
            self.current.clone()
        }
        async fn sections(&self, tag: &str) -> AppResult<Sections> {
            match self.sections.get(tag) {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(_)) | None => Err(AppError::Network("offline".into())),
            }
        }
    }

    fn sec(id: &str, title: &str) -> Section {
        Section { id: id.into(), title: title.into() }
    }

    fn dir() -> FakeDir {
        let t = crate::sid::tag;
        FakeDir {
            servers: vec![
                ServerEntry { machine_id: "zed-friend".into(), name: "Zed's".into(), owned: false },
                ServerEntry { machine_id: "home-box".into(), name: "Home".into(), owned: true },
                ServerEntry { machine_id: "cabin".into(), name: "Cabin".into(), owned: true },
            ],
            current: Some(t("home-box")),
            sections: [
                (t("home-box"), Ok((vec![sec("1", "Music"), sec("4", "Archive")], Some(ConnectionKind::Local)))),
                (t("zed-friend"), Ok((vec![sec("7", "Zed music")], Some(ConnectionKind::Relay)))),
                (t("cabin"), Err(AppError::Network("x".into()))),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[tokio::test]
    async fn lists_owned_and_shared_libraries_with_what_is_included() {
        let included = vec!["zed-friend/7".to_string(), "cabin/3".to_string()];
        let (out, ranks) = sources(&dir(), &included, Some("1")).await;
        let names: Vec<&str> = out.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Home", "Cabin", "Zed's"], "connected first, then owned, then shared");
        let home = &out.servers[0];
        assert!(home.current && home.owned && home.online);
        assert_eq!(home.connection, Some(ConnectionKind::Local));
        assert_eq!(
            home.libraries,
            vec![
                SourceLibrary { id: "1".into(), title: "Music".into(), included: true, main: true },
                SourceLibrary { id: "4".into(), title: "Archive".into(), included: false, main: false },
            ]
        );
        let zed = &out.servers[2];
        assert!(!zed.owned && zed.libraries[0].included);
        // Offline, but what's included on it still shows so it can be left out.
        let cabin = &out.servers[1];
        assert!(!cabin.online);
        assert_eq!(cabin.libraries.len(), 1);
        assert!(cabin.libraries[0].included);
        let rank: HashMap<String, Rank> = ranks.into_iter().collect();
        assert_eq!(rank[&crate::sid::tag("home-box")], Rank::Local);
        assert_eq!(rank[&crate::sid::tag("zed-friend")], Rank::Relay);
        assert_eq!(rank[&crate::sid::tag("cabin")], Rank::Unknown);
    }

    #[test]
    fn entries_are_checked_and_kept_once() {
        assert_eq!(parse_entry("abc-1/12"), Some(("abc-1", "12")));
        for bad in ["", "abc", "/1", "abc/", "abc/x", "a b/1", "abc/1/2"] {
            assert_eq!(parse_entry(bad), None, "{bad}");
        }
        assert_eq!(tidy(&[" m/1 ".into(), "m/1".into(), "n/2".into()]).unwrap(), vec!["m/1", "n/2"]);
        assert!(tidy(&["m/one".into()]).is_err());
        let many: Vec<String> = (0..=MAX_LIBRARIES).map(|i| format!("m/{i}")).collect();
        assert!(tidy(&many).is_err());
    }

    #[test]
    fn the_switcher_lists_servers_with_music_and_falls_back_to_all() {
        let d = dir();
        let home = crate::sid::tag("home-box");
        let zed = crate::sid::tag("zed-friend");
        // Only the main library: one server, nothing to switch.
        let one = included_servers(&d.servers, Some(&home), &[], &ServerView::Server(home.clone()));
        assert_eq!(one.servers.len(), 1);
        assert_eq!(one.view, "all");
        // Another library on the same server: still one server.
        let same = included_servers(&d.servers, Some(&home), &["home-box/4".into()], &ServerView::All);
        assert_eq!(same.servers.len(), 1);
        // A shared server's library: two, and a chosen one sticks.
        let two = included_servers(&d.servers, Some(&home), &["zed-friend/7".into()], &ServerView::Server(zed.clone()));
        assert_eq!(two.servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Home", "Zed's"]);
        assert_eq!(two.view, zed);
        // A server no longer included: back to all.
        let gone = included_servers(&d.servers, Some(&home), &["zed-friend/7".into()], &ServerView::Server(crate::sid::tag("cabin")));
        assert_eq!(gone.view, "all");
    }
}
