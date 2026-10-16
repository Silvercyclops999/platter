//! Plex for the tag editor: rescanning one folder, the size of a track's
//! file, and the read-only check of the music library's settings.

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use super::editor::PlexSide;
use crate::error::AppResult;
use crate::plex::client::LibrarySection;
use crate::plex::{PlexClient, PlexState};

/// `GET /library/sections/{section}/refresh?path={folder}`: Plex scans only
/// that folder.
pub async fn refresh_folder_on(client: &PlexClient, section: &str, folder: &str) -> AppResult<()> {
    let section: String = section.chars().filter(char::is_ascii_alphanumeric).collect();
    client.get_bytes(&format!("/library/sections/{section}/refresh"), &[("path", folder.to_string())]).await.map(|_| ())
}

pub struct AppPlex(pub PlexState);

#[async_trait]
impl PlexSide for AppPlex {
    async fn refresh_folder(&self, track_id: &str, section: &str, folder: &str) -> AppResult<()> {
        let client = self.0.client_for(track_id).await?;
        refresh_folder_on(&client, section, folder).await
    }

    async fn part_sizes(&self, track_ids: &[String]) -> AppResult<Vec<(String, Option<u64>)>> {
        let mut out = Vec::new();
        for id in track_ids {
            let client = self.0.client_for(id).await?;
            let size = client
                .metadata_many(std::slice::from_ref(id))
                .await?
                .into_iter()
                .next()
                .and_then(|m| m.media.into_iter().next())
                .and_then(|m| m.parts.into_iter().next())
                .and_then(|p| p.size)
                .and_then(|s| u64::try_from(s).ok());
            out.push((id.clone(), size));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- library settings

/// One setting the check looked at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefItem {
    pub label: String,
    /// None when Plex doesn't say.
    pub ok: Option<bool>,
    /// What it means, and the steps in Plex when it's off.
    pub help: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionCheck {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub items: Vec<PrefItem>,
    /// Everything that can be checked is on.
    pub ok: bool,
}

/// The Personal Media agents' ids (artists, albums).
const PERSONAL_MEDIA: &[&str] = &["com.plexapp.agents.none", "tv.plex.agents.none"];

/// Agent ids from `GET /system/agents`.
pub fn parse_agents(body: &[u8]) -> Vec<String> {
    let v: Value = serde_json::from_slice(body).unwrap_or_default();
    let list = v.pointer("/MediaContainer/Agent").and_then(Value::as_array).cloned().unwrap_or_default();
    list.iter().filter_map(|a| a.get("identifier").and_then(Value::as_str).map(str::to_string)).collect()
}

fn pref_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn agent_name(id: &str) -> String {
    match id {
        "tv.plex.agents.music" => "Plex Music".into(),
        "com.plexapp.agents.lastfm" => "Last.fm (legacy)".into(),
        "com.plexapp.agents.plexmusic" => "Plex Premium Music (legacy)".into(),
        a if PERSONAL_MEDIA.contains(&a) => "Personal Media".into(),
        "" => "Unknown".into(),
        a => a.to_string(),
    }
}

/// How one music library is set up for tags (read only).
pub fn assess(section: &LibrarySection, prefs: &[(String, Value)], agents: &[String]) -> SectionCheck {
    let find = |id: &str| prefs.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).and_then(|(_, v)| pref_bool(v));
    let agent = section.agent.clone().unwrap_or_default();
    let personal_library = PERSONAL_MEDIA.contains(&agent.as_str());
    let local = find("respectTags");
    let mut items = vec![PrefItem {
        label: "Local metadata preferred".into(),
        ok: if personal_library { Some(true) } else { local },
        help: match (personal_library, local) {
            (true, _) => "This library uses Personal Media, so Plex shows the tags in your files as they are.".into(),
            (_, Some(true)) => "Plex shows the album artist, album and titles from your tags rather than its own.".into(),
            _ => format!(
                "Plex may replace your tags with its own matches. In Plex, open {}, choose Edit Library, then Advanced, turn on Prefer local metadata and save.",
                section.title
            ),
        },
    }];
    // Legacy agents have their own "use embedded tags" setting; Plex Music
    // always reads them.
    let embedded = prefs.iter().find(|(k, _)| k.to_ascii_lowercase().contains("embedded")).and_then(|(_, v)| pref_bool(v));
    items.push(PrefItem {
        label: "Embedded tags read".into(),
        ok: embedded.or(if agent == "tv.plex.agents.music" || personal_library { Some(true) } else { None }),
        help: match embedded {
            Some(false) => format!(
                "Plex ignores the tags in your files. In Plex, open {}, choose Edit Library, then Advanced, and turn on Use embedded tags.",
                section.title
            ),
            _ => "Plex reads the tags inside your files when it scans them.".into(),
        },
    });
    let personal = agents.iter().any(|a| PERSONAL_MEDIA.contains(&a.as_str()));
    items.push(PrefItem {
        label: "Personal Media agent available".into(),
        ok: Some(personal || personal_library),
        help: if personal || personal_library {
            "For an album MusicBrainz doesn't know, choose Fix Match in Plex and pick Personal Media, so a wrong match never overwrites an unreleased album.".into()
        } else {
            "Plex didn't list its Personal Media agent. Update Plex Media Server: newer versions include it.".into()
        },
    });
    let ok = items.iter().all(|i| i.ok != Some(false));
    SectionCheck { id: section.id.clone(), title: section.title.clone(), agent: agent_name(&agent), items, ok }
}

/// Every music library on the connected server.
pub async fn library_check(client: &PlexClient) -> AppResult<Vec<SectionCheck>> {
    let sections = client.music_sections().await?;
    let agents = match client.get_bytes("/system/agents", &[("mediaType", "9".to_string())]).await {
        Ok(b) => parse_agents(&b),
        Err(e) => {
            tracing::debug!("agents: {e}");
            Vec::new()
        }
    };
    let mut out = Vec::new();
    for s in sections {
        let prefs = client.section_prefs(&s.id).await.map(|p| p.all).unwrap_or_default();
        out.push(assess(&s, &prefs, &agents));
    }
    Ok(out)
}
