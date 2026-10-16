//! Every change Platter makes to Lidarr's own settings, profiles, roots and
//! exclusions shows the exact before and after first, waits for a yes, and
//! is kept in the changes log (Settings > Lidarr > Changes) with "Put it
//! back" where that's possible. Writes to albums Platter requested itself
//! (monitoring, freeing a promoted album's staging files) don't ask, since
//! the request was the consent, but they're logged too.
//!
//! A proposal is made from what Lidarr has now. Applying it reads Lidarr
//! again and refuses if the value changed in between, so the change made is
//! always the change shown.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Lidarr;
use super::api::LidarrClient;
use super::check::{ExclusionTarget, FixAction};
use super::paths::{NormPath, inside_any, to_server};
use super::write::{ConfigKind, LidarrWriter, NewRoot, WriteCall};
use crate::error::{AppError, AppResult};

/// How long a shown proposal can be applied.
const PROPOSAL_TTL_S: i64 = 15 * 60;
/// Most entries kept in the log.
const MAX_LOG: usize = 300;
/// The name of the hi-res profile Platter offers to make.
pub const HIRES_PROFILE: &str = "Platter Hi-Res";

/// What putting a change back does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Undo {
    RemoveExclusions { ids: Vec<i64> },
    /// Set one settings value back.
    Config { config: ConfigKind, field: String, value: Value },
    /// Only while no artist lives under it.
    RemoveRootFolder { id: i64, path: String },
    /// Only while no artist uses it.
    RemoveQualityProfile { id: i64 },
    Monitor { album_ids: Vec<i64>, monitored: bool },
}

/// One logged change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeEntry {
    pub id: String,
    /// Unix seconds.
    pub at: i64,
    /// The device that made it.
    pub device: String,
    pub what: String,
    pub before: String,
    pub after: String,
    /// The user said yes to it (false: part of a request they made).
    pub asked: bool,
    #[serde(default)]
    pub undo: Option<Undo>,
    /// When it was put back.
    #[serde(default)]
    pub put_back_at: Option<i64>,
    /// Practice mode: what Platter would have done, not done.
    #[serde(default)]
    pub practice: bool,
}

/// A change waiting for a yes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    pub what: String,
    pub before: String,
    pub after: String,
    /// Can be put back afterwards.
    pub can_put_back: bool,
    #[serde(skip)]
    plan: Plan,
}

#[derive(Debug, Clone, PartialEq)]
enum Plan {
    Exclusions(Vec<ExclusionTarget>),
    Config { config: ConfigKind, field: &'static str, before: Value, after: Value },
    StagingRoot(NewRoot),
    HiResProfile(Value),
}

fn shown(v: &Value) -> String {
    match v {
        Value::String(s) if s.trim().is_empty() => "(none)".into(),
        Value::String(s) => s.clone(),
        Value::Bool(true) => "On".into(),
        Value::Bool(false) => "Off".into(),
        Value::Null => "(none)".into(),
        other => other.to_string(),
    }
}

fn absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with("\\\\") || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// The "Platter Hi-Res" profile: every 24-bit lossless quality allowed and
/// nothing else, cutoff at FLAC 24bit, upgrades on. Lidarr lists FLAC and
/// FLAC 24bit as separate qualities, and a usual Lossless cutoff groups
/// both, so hi-res needs a profile of its own. Built from an existing
/// profile's quality list, which holds every quality Lidarr knows.
pub fn hires_profile(raw: &[Value]) -> AppResult<Value> {
    let src = raw
        .iter()
        .max_by_key(|p| p["items"].as_array().map_or(0, |a| a.len()))
        .ok_or_else(|| AppError::Invalid("Lidarr has no quality profile to start from.".into()))?;
    let is_hires = |q: &Value| q["name"].as_str().is_some_and(|n| n.to_ascii_lowercase().contains("24bit") || n.to_ascii_lowercase().contains("24-bit"));
    let mut cutoff: Option<i64> = None;
    fn walk(items: &mut Value, is_hires: &dyn Fn(&Value) -> bool, cutoff: &mut Option<i64>) {
        let Some(list) = items.as_array_mut() else { return };
        for it in list {
            let on = it.get("quality").is_some_and(|q| !q.is_null() && is_hires(q));
            if on && cutoff.is_none() && it["quality"]["name"].as_str().is_some_and(|n| n.to_ascii_lowercase().starts_with("flac")) {
                *cutoff = it["quality"]["id"].as_i64();
            }
            if let Some(children) = it.get_mut("items") {
                walk(children, is_hires, cutoff);
            }
            let child_on = it["items"].as_array().is_some_and(|c| c.iter().any(|c| c["allowed"] == true));
            it["allowed"] = Value::Bool(on || child_on);
        }
    }
    let mut items = src["items"].clone();
    walk(&mut items, &is_hires, &mut cutoff);
    let cutoff = cutoff.ok_or_else(|| AppError::Invalid("Lidarr doesn't list a FLAC 24bit quality, so there's nothing for a hi-res profile to aim at.".into()))?;
    let mut p = serde_json::json!({
        "name": HIRES_PROFILE,
        "upgradeAllowed": true,
        "cutoff": cutoff,
        "items": items,
    });
    for k in ["minFormatScore", "cutoffFormatScore", "formatItems"] {
        if let Some(v) = src.get(k) {
            p[k] = v.clone();
        }
    }
    Ok(p)
}

/// Shown proposals by id, with when they were made.
pub(super) type Proposals = parking_lot::Mutex<HashMap<String, (i64, Proposal)>>;

impl Lidarr {
    pub(super) async fn connection(&self) -> AppResult<(LidarrClient, LidarrWriter)> {
        let (client, _, route) = Self::connect(self.routes()?).await?;
        Ok((client, LidarrWriter::new(route as std::sync::Arc<dyn super::write::LidarrWrite>)))
    }

    /// Every logged change, newest first.
    pub fn changes(&self) -> Vec<ChangeEntry> {
        let mut l: Vec<ChangeEntry> = self.inner.shared.list::<ChangeEntry>("change:").into_iter().map(|(_, e)| e).collect();
        l.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
        l
    }

    /// Log a change (asked or part of a request).
    pub fn log_change(&self, what: &str, before: &str, after: &str, asked: bool, undo: Option<Undo>) -> AppResult<ChangeEntry> {
        self.log_entry(what, before, after, asked, undo, false)
    }

    /// Log what practice mode would have done.
    pub fn log_practice(&self, what: &str, before: &str, after: &str) -> AppResult<ChangeEntry> {
        self.log_entry(what, before, after, false, None, true)
    }

    fn log_entry(&self, what: &str, before: &str, after: &str, asked: bool, undo: Option<Undo>, practice: bool) -> AppResult<ChangeEntry> {
        let at = (self.inner.clock)();
        let e = ChangeEntry {
            id: format!("{at:012}-{}", uuid::Uuid::new_v4().simple()),
            at,
            device: self.inner.shared.label(),
            what: what.into(),
            before: before.into(),
            after: after.into(),
            asked,
            undo,
            put_back_at: None,
            practice,
        };
        self.inner.shared.put(&format!("change:{}", e.id), &e)?;
        let all = self.changes();
        for old in all.iter().skip(MAX_LOG) {
            let _ = self.inner.shared.remove(&format!("change:{}", old.id));
        }
        tracing::info!(what, asked, practice, "lidarr change logged");
        Ok(e)
    }

    /// Show what a fix would change. Reads only.
    pub async fn propose(&self, action: FixAction, input: Option<String>) -> AppResult<Proposal> {
        let (client, _) = self.connection().await?;
        let config = |config: ConfigKind, field: &'static str, after: Value, what: &str| {
            let client = client.clone();
            let what = what.to_string();
            async move {
                let raw = client.get_raw(config.path()).await?;
                let before = raw.get(field).cloned().unwrap_or(Value::Null);
                if before == after {
                    return Err(AppError::Invalid("Lidarr already has that setting.".into()));
                }
                Ok::<_, AppError>((what, shown(&before), shown(&after), Plan::Config { config, field, before, after }))
            }
        };
        let (what, before, after, plan) = match action {
            FixAction::AddExclusions { artists } => {
                let have: Vec<String> = client.exclusions().await?.into_iter().map(|e| e.foreign_id.to_ascii_lowercase()).collect();
                let add: Vec<ExclusionTarget> =
                    artists.into_iter().filter(|a| crate::artistinfo::is_mbid(&a.mbid) && !have.contains(&a.mbid.to_ascii_lowercase())).collect();
                if add.is_empty() {
                    return Err(AppError::Invalid("They're already on Lidarr's import list exclusions.".into()));
                }
                let names: Vec<&str> = add.iter().map(|a| a.name.as_str()).collect();
                (
                    "Add protected artists to Lidarr's import list exclusions".to_string(),
                    format!("Not on the list: {}", names.join(", ")),
                    format!("On the list: {}. Lidarr's import lists never add them.", names.join(", ")),
                    Plan::Exclusions(add),
                )
            }
            FixAction::SetRecycleBin => {
                let path = input.map(|p| p.trim().trim_end_matches(['/', '\\']).to_string()).filter(|p| !p.is_empty());
                let path = path.ok_or_else(|| AppError::Invalid("Write the recycling bin folder as Lidarr sees it, like /data/lidarr-recycle.".into()))?;
                if !absolute(&path) || path.split(['/', '\\']).any(|s| s == "..") {
                    return Err(AppError::Invalid("Write the recycling bin as a full folder path.".into()));
                }
                let plex = self.inner.plex.music_files().await?.locations;
                let share = crate::musicshare::load(&self.inner.db);
                let pairs: Vec<(String, String)> = share.music.iter().map(|p| (p.server.clone(), p.local.clone())).collect();
                let server = to_server(&path, &pairs);
                if let Some(l) = inside_any(&server, plex.iter().map(String::as_str)) {
                    return Err(AppError::Invalid(format!("{path} is inside your Plex music folder {l}, so deleted files would show up in Plex. Pick a folder outside it.")));
                }
                let roots: Vec<String> = client.root_folders().await?.into_iter().map(|r| r.path).collect();
                if let Some(r) = inside_any(&server, roots.iter().map(String::as_str)) {
                    return Err(AppError::Invalid(format!("{path} is inside Lidarr's root folder {r}. Pick a folder outside every root folder.")));
                }
                config(ConfigKind::MediaManagement, "recycleBin", Value::String(path), "Set Lidarr's recycling bin").await?
            }
            FixAction::WriteTagsOff => config(ConfigKind::MetadataProvider, "writeAudioTags", Value::String("no".into()), "Turn off Write Audio Tags").await?,
            FixAction::RenameOff => config(ConfigKind::Naming, "renameTracks", Value::Bool(false), "Turn off Rename Tracks").await?,
            FixAction::AddStagingRoot { path } => {
                let plex = self.inner.plex.music_files().await?.locations;
                if let Some(l) = inside_any(&path, plex.iter().map(String::as_str)) {
                    return Err(AppError::Invalid(format!("{path} is inside your Plex music folder {l}. A staging folder must be somewhere Plex doesn't scan.")));
                }
                let roots = client.root_folders().await?;
                let p = NormPath::parse(&path);
                if let Some(r) = roots.iter().find(|r| {
                    let rp = NormPath::parse(&r.path);
                    p.starts_with(&rp) || rp.starts_with(&p)
                }) {
                    return Err(AppError::Invalid(format!(
                        "{path} and Lidarr's root folder {} are one inside the other. Pick a staging folder apart from every root folder.",
                        r.path
                    )));
                }
                let quality = roots.iter().find_map(|r| r.default_quality_profile_id).or(client.quality_profiles().await?.first().map(|q| q.id));
                let metadata = roots.iter().find_map(|r| r.default_metadata_profile_id).or(client.metadata_profiles().await?.first().map(|m| m.id));
                let (Some(q), Some(m)) = (quality, metadata) else { return Err(AppError::Invalid("Lidarr has no profiles to give the staging folder.".into())) };
                (
                    "Add the staging folder to Lidarr".to_string(),
                    "Not a root folder".to_string(),
                    format!("Root folder {path}, monitoring nothing by default"),
                    Plan::StagingRoot(NewRoot::staging(&path, q, m)),
                )
            }
            FixAction::Rematch { .. } => return Err(AppError::Invalid("Re-match an album from its own row.".into())),
        };
        Ok(self.keep_proposal(what, before, after, plan))
    }

    /// Show the "Platter Hi-Res" profile Platter would add. Reads only.
    pub async fn propose_hires_profile(&self) -> AppResult<Proposal> {
        let (client, _) = self.connection().await?;
        let profiles = client.quality_profiles().await?;
        if let Some(p) = profiles.iter().find(|p| p.name.eq_ignore_ascii_case(HIRES_PROFILE)) {
            return Err(AppError::Invalid(format!("Lidarr already has the {} profile.", p.name)));
        }
        let raw: Vec<Value> = serde_json::from_value(client.get_raw("/qualityprofile").await?).unwrap_or_default();
        let body = hires_profile(&raw)?;
        Ok(self.keep_proposal(
            format!("Add the {HIRES_PROFILE} quality profile"),
            "No hi-res profile".into(),
            format!("{HIRES_PROFILE}: 24-bit lossless only, upgrades until FLAC 24bit"),
            Plan::HiResProfile(body),
        ))
    }

    fn keep_proposal(&self, what: String, before: String, after: String, plan: Plan) -> Proposal {
        let p = Proposal { id: uuid::Uuid::new_v4().to_string(), what, before, after, can_put_back: true, plan };
        let now = (self.inner.clock)();
        let mut all = self.proposals.lock();
        all.retain(|_, (at, _)| now - *at < PROPOSAL_TTL_S);
        all.insert(p.id.clone(), (now, p.clone()));
        p
    }

    /// Make a change the user said yes to, then log it.
    pub async fn apply(&self, id: &str) -> AppResult<ChangeEntry> {
        let _one = self.lock.lock().await;
        let now = (self.inner.clock)();
        let p = self
            .proposals
            .lock()
            .remove(id)
            .filter(|(at, _)| now - *at < PROPOSAL_TTL_S)
            .map(|(_, p)| p)
            .ok_or_else(|| AppError::Invalid("That change was shown a while ago. Look at it again first.".into()))?;
        let (client, writer) = self.connection().await?;
        let undo = match &p.plan {
            Plan::Exclusions(list) => {
                let mut ids = Vec::new();
                for a in list {
                    let id = writer.send_call(WriteCall::AddExclusion { foreign_id: a.mbid.clone(), artist_name: a.name.clone() }).await?;
                    if id > 0 {
                        ids.push(id);
                    }
                }
                Some(Undo::RemoveExclusions { ids })
            }
            Plan::Config { config, field, before, after } => {
                let mut raw = client.get_raw(config.path()).await?;
                if raw.get(*field).unwrap_or(&Value::Null) != before {
                    return Err(AppError::Invalid("Lidarr's setting changed since Platter showed it. Look at it again first. Nothing was changed.".into()));
                }
                raw[*field] = after.clone();
                let cid = raw["id"].as_i64().unwrap_or(1);
                writer.send_call(WriteCall::PutConfig { kind: *config, id: cid, body: raw }).await?;
                Some(Undo::Config { config: *config, field: field.to_string(), value: before.clone() })
            }
            Plan::StagingRoot(root) => {
                let id = writer.send_call(WriteCall::AddRootFolder(root.clone())).await?;
                (id > 0).then(|| Undo::RemoveRootFolder { id, path: root.path.clone() })
            }
            Plan::HiResProfile(body) => {
                let id = writer.send_call(WriteCall::AddQualityProfile(body.clone())).await?;
                (id > 0).then_some(Undo::RemoveQualityProfile { id })
            }
        };
        *self.modes_cache.lock() = None;
        self.log_change(&p.what, &p.before, &p.after, true, undo)
    }

    /// Put a logged change back (the user confirmed it in the log).
    pub async fn put_back(&self, id: &str) -> AppResult<ChangeEntry> {
        let _one = self.lock.lock().await;
        let key = format!("change:{id}");
        let mut e: ChangeEntry = self.inner.shared.get(&key).ok_or_else(|| AppError::NotFound("That change isn't in the log.".into()))?;
        if e.put_back_at.is_some() {
            return Err(AppError::Invalid("That change was already put back.".into()));
        }
        let undo = e.undo.clone().ok_or_else(|| AppError::Invalid("This change can't be put back from Platter.".into()))?;
        let (client, writer) = self.connection().await?;
        match undo {
            Undo::RemoveExclusions { ids } => {
                for id in ids {
                    writer.send_call(WriteCall::RemoveExclusion { id }).await?;
                }
            }
            Undo::Config { config, field, value } => {
                let mut raw = client.get_raw(config.path()).await?;
                raw[field.as_str()] = value;
                let cid = raw["id"].as_i64().unwrap_or(1);
                writer.send_call(WriteCall::PutConfig { kind: config, id: cid, body: raw }).await?;
            }
            Undo::RemoveRootFolder { id, path } => {
                // Never a root that holds artists or another root: removing it
                // would let go of their files.
                let root = NormPath::parse(&path);
                if client.artists().await?.iter().any(|a| NormPath::parse(&a.path).starts_with(&root)) {
                    return Err(AppError::Invalid(format!("Lidarr has artists in {path} now, so Platter won't remove it. Nothing was changed.")));
                }
                if client.root_folders().await?.iter().any(|r| r.id != id && NormPath::parse(&r.path).starts_with(&root)) {
                    return Err(AppError::Invalid(format!("Another root folder is inside {path}, so Platter won't remove it. Nothing was changed.")));
                }
                writer.send_call(WriteCall::RemoveRootFolder { id }).await?;
            }
            Undo::RemoveQualityProfile { id } => {
                if client.artists().await?.iter().any(|a| a.quality_profile_id == Some(id)) {
                    return Err(AppError::Invalid("Lidarr artists use this profile now, so Platter won't remove it. Nothing was changed.".into()));
                }
                writer.send_call(WriteCall::RemoveQualityProfile { id }).await?;
            }
            Undo::Monitor { album_ids, monitored } => {
                writer.monitor(album_ids, monitored).await?;
            }
        }
        e.put_back_at = Some((self.inner.clock)());
        self.inner.shared.put(&key, &e)?;
        *self.modes_cache.lock() = None;
        Ok(e)
    }
}
