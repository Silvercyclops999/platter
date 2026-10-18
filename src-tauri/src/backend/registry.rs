//! The servers Platter knows, kept in `servers.json` next to the library
//! cache. Records hold no secrets: tokens, API keys and passwords live in
//! the keychain. A Plex server's record is written from what plex.tv lists;
//! other kinds are added by the user.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const SERVERS_FILE: &str = "servers.json";

/// What kind of server a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendKind {
    Plex,
    Jellyfin,
    Subsonic,
}

impl BackendKind {
    /// The tag for this kind of server with its own id (`crate::sid`).
    pub fn tag(self, server_id: &str) -> String {
        use crate::sid::{TagInput, tag_for};
        match self {
            BackendKind::Plex => tag_for(TagInput::Plex(server_id)),
            BackendKind::Jellyfin => tag_for(TagInput::Jellyfin(server_id)),
            BackendKind::Subsonic => tag_for(TagInput::Subsonic(server_id)),
        }
    }
}

/// One server. `url` is the address the user typed (none for Plex, which
/// is found through plex.tv); `user` the account name shown in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRecord {
    pub tag: String,
    pub kind: BackendKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The account's id on the server (not a secret; the token is in the
    /// keychain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// The music libraries included from this server; None: all of them.
    /// (Plex libraries are kept in Settings.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libraries: Option<Vec<String>>,
    /// The server software as it names itself (an OpenSubsonic server's
    /// `type`, such as navidrome), to tell it's the same server at a new
    /// address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// What the server said it can do when last asked (a Subsonic server:
    /// whether it's an OpenSubsonic server, and its extensions), so a start
    /// while it's out of reach isn't left with the plain feature set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_subsonic: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

impl ServerRecord {
    pub fn new(tag: &str, kind: BackendKind, name: &str) -> Self {
        Self {
            tag: tag.into(),
            kind,
            name: name.into(),
            url: None,
            user: None,
            user_id: None,
            libraries: None,
            product: None,
            open_subsonic: None,
            extensions: Vec::new(),
        }
    }
}

/// The file. Fields this build doesn't know are dropped on the next save,
/// so a secret written by mistake doesn't stay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServersFile {
    #[serde(default)]
    pub servers: Vec<ServerRecord>,
    /// Tag of the primary server (the main library).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
}

impl ServersFile {
    /// The saved list; empty when there's none or it can't be read.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(SERVERS_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("{SERVERS_FILE} not read: {e}");
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("{SERVERS_FILE} not read: {e}");
                Self::default()
            }
        }
    }

    pub fn save(&self, dir: &Path) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| AppError::Internal(format!("{SERVERS_FILE}: {e}")))?;
        let tmp = dir.join(format!("{SERVERS_FILE}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, dir.join(SERVERS_FILE))?;
        Ok(())
    }

    pub fn get(&self, tag: &str) -> Option<&ServerRecord> {
        self.servers.iter().find(|s| s.tag == tag)
    }

    /// Add or replace the record with this tag. True when anything changed.
    pub fn upsert(&mut self, r: ServerRecord) -> bool {
        match self.servers.iter_mut().find(|s| s.tag == r.tag) {
            Some(s) if *s == r => false,
            Some(s) => {
                *s = r;
                true
            }
            None => {
                if self.primary.is_none() {
                    self.primary = Some(r.tag.clone());
                }
                self.servers.push(r);
                true
            }
        }
    }

    /// Make `tag` the primary server. True when it changed.
    pub fn set_primary(&mut self, tag: &str) -> bool {
        if self.primary.as_deref() == Some(tag) || self.get(tag).is_none() {
            return false;
        }
        self.primary = Some(tag.to_string());
        true
    }

    /// Drop a server. The first one left becomes primary if it was.
    pub fn remove(&mut self, tag: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|s| s.tag != tag);
        if self.primary.as_deref() == Some(tag) {
            self.primary = self.servers.first().map(|s| s.tag.clone());
        }
        before != self.servers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plex(machine: &str, name: &str) -> ServerRecord {
        ServerRecord::new(&BackendKind::Plex.tag(machine), BackendKind::Plex, name)
    }

    #[test]
    fn kind_tags_match_sid() {
        assert_eq!(BackendKind::Plex.tag("m1"), crate::sid::tag("m1"));
        assert_ne!(BackendKind::Jellyfin.tag("m1"), crate::sid::tag("m1"));
        assert_eq!(
            BackendKind::Subsonic.tag("http://h.example/rest/"),
            BackendKind::Subsonic.tag("http://h.example")
        );
    }

    #[test]
    fn saves_and_loads_without_secrets() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(ServersFile::load(d.path()), ServersFile::default(), "nothing saved yet");
        let mut f = ServersFile::default();
        assert!(f.upsert(plex("m1", "Home")));
        assert!(!f.upsert(plex("m1", "Home")), "same record");
        assert!(f.upsert(plex("m1", "Home server")), "renamed");
        let jf = ServerRecord {
            tag: BackendKind::Jellyfin.tag("abc"),
            kind: BackendKind::Jellyfin,
            name: "Den".into(),
            url: Some("http://den.example:8096".into()),
            user: Some("me".into()),
            user_id: Some("u1".into()),
            libraries: None,
            product: None,
            open_subsonic: None,
            extensions: Vec::new(),
        };
        assert!(f.upsert(jf.clone()));
        assert_eq!(f.primary.as_deref(), Some(crate::sid::tag("m1").as_str()), "the first added");
        f.save(d.path()).unwrap();
        assert_eq!(ServersFile::load(d.path()), f);
        let text = std::fs::read_to_string(d.path().join(SERVERS_FILE)).unwrap();
        assert!(text.contains("\"kind\": \"jellyfin\""));

        // A secret written into the file by hand is dropped on the next save.
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["servers"][1]["token"] = "secret-token".into();
        std::fs::write(d.path().join(SERVERS_FILE), v.to_string()).unwrap();
        let back = ServersFile::load(d.path());
        assert_eq!(back, f);
        back.save(d.path()).unwrap();
        assert!(!std::fs::read_to_string(d.path().join(SERVERS_FILE)).unwrap().contains("secret-token"));
    }

    #[test]
    fn primary_follows_removals() {
        let mut f = ServersFile::default();
        f.upsert(plex("m1", "A"));
        f.upsert(plex("m2", "B"));
        assert!(f.set_primary(&crate::sid::tag("m2")));
        assert!(!f.set_primary("ffffffff"), "unknown tag");
        assert!(f.remove(&crate::sid::tag("m2")));
        assert_eq!(f.primary.as_deref(), Some(crate::sid::tag("m1").as_str()));
        assert!(!f.remove("ffffffff"));
    }

    #[test]
    fn an_unreadable_file_is_an_empty_list() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(SERVERS_FILE), b"{not json").unwrap();
        assert_eq!(ServersFile::load(d.path()), ServersFile::default());
    }
}
