//! What Lidarr got for a request once it's imported: the quality and the
//! edition ("FLAC, 13 tracks", "MP3 320, Newsletter Edit"), and, when the
//! copy is lossy under a profile that keeps upgrading, that Lidarr will
//! replace it. Reads only; kept on this device next to the requests.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::api::{QualityProfile, TrackFile};
use super::editions::SeenRelease;
use super::quality::{Rank, cutoff_rank, cutoff_reachable};
use super::request::RequestRecord;
use super::status::RequestState;
use crate::db::store::{kv_get, kv_set};
use crate::error::AppResult;

const KEY: &str = "lidarr.grabbed";
/// Most imported requests looked at in one refresh.
const MAX_LOOKS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grabbed {
    /// "FLAC, 13 tracks".
    pub line: String,
    /// "Lidarr will replace it with FLAC when one turns up."
    pub upgrade_note: Option<String>,
}

/// A request with what Lidarr got for it, for the requests list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    #[serde(flatten)]
    pub record: RequestRecord,
    pub grabbed: Option<Grabbed>,
}

/// "MP3-320" → "MP3 320", "FLAC 24bit" → "FLAC 24-bit".
pub fn quality_words(name: &str) -> String {
    let n = name.trim().replace('-', " ");
    match n.strip_suffix("bit").filter(|s| s.ends_with(|c: char| c.is_ascii_digit())) {
        Some(s) => format!("{s}-bit"),
        None => n,
    }
}

/// The quality most of the files have, by Lidarr's name for it.
fn main_quality(files: &[TrackFile]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for f in files {
        if let Some(n) = f.quality.as_ref().and_then(|q| q["quality"]["name"].as_str()).map(str::trim).filter(|n| !n.is_empty()) {
            *counts.entry(n.to_string()).or_default() += 1;
        }
    }
    counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(n, _)| n)
}

/// What Lidarr got, from its files, the album's current edition and the
/// artist's quality profile.
pub fn describe(files: &[TrackFile], edition: Option<&SeenRelease>, profile: Option<&QualityProfile>) -> Option<Grabbed> {
    let name = main_quality(files)?;
    let edition = match edition {
        Some(r) if !r.disambiguation.trim().is_empty() => r.disambiguation.trim().to_string(),
        Some(r) if r.tracks > 0 => format!("{} tracks", r.tracks),
        _ => format!("{} tracks", files.len()),
    };
    let rank = Rank::of_lidarr(&name);
    let upgrade_note = profile.filter(|p| p.upgrade_allowed && cutoff_reachable(p)).and_then(cutoff_rank).filter(|c| !rank.is_lossless() && *c > rank).map(|c| {
        let to = if c.is_lossless() { c.label(Some("FLAC")) } else { "a better copy".to_string() };
        format!("Lidarr will replace it with {to} when one turns up.")
    });
    Some(Grabbed { line: format!("{}, {edition}", quality_words(&name)), upgrade_note })
}

impl Lidarr {
    fn grabbed(&self) -> HashMap<String, Grabbed> {
        self.inner.db.read(|c| kv_get(c, KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// The requests, newest first, each with what Lidarr got.
    pub fn request_views(&self) -> Vec<RequestView> {
        let g = self.grabbed();
        self.requests().into_iter().map(|r| RequestView { grabbed: g.get(&r.id).filter(|_| r.state == RequestState::Imported).cloned(), record: r }).collect()
    }

    /// Look at imported requests Lidarr may still change (new ones, and
    /// lossy ones it keeps upgrading). Reads only.
    pub async fn note_grabs(&self) -> AppResult<()> {
        let mut have = self.grabbed();
        let todo: Vec<RequestRecord> = self
            .requests()
            .into_iter()
            .filter(|r| r.state == RequestState::Imported && r.lidarr_album_id.is_some())
            .filter(|r| have.get(&r.id).is_none_or(|g| g.upgrade_note.is_some()))
            .take(MAX_LOOKS)
            .collect();
        if todo.is_empty() {
            return Ok(());
        }
        let (client, _) = self.connection().await?;
        let profiles = client.quality_profiles().await.unwrap_or_default();
        let mut artist_profile: HashMap<i64, Option<i64>> = HashMap::new();
        for r in todo {
            let Some(album_id) = r.lidarr_album_id else { continue };
            let Ok((detail, edition)) = self.release_now(&client, album_id).await else { continue };
            let Ok(files) = client.track_files(album_id).await else { continue };
            let pid = match artist_profile.get(&detail.artist_id) {
                Some(p) => *p,
                None => {
                    let p = client.artist(detail.artist_id).await.ok().and_then(|a| a.quality_profile_id);
                    artist_profile.insert(detail.artist_id, p);
                    p
                }
            };
            let profile = pid.and_then(|id| profiles.iter().find(|p| p.id == id));
            if let Some(g) = describe(&files, edition.as_ref(), profile) {
                have.insert(r.id.clone(), g);
            }
        }
        let ids: std::collections::HashSet<String> = self.requests().into_iter().map(|r| r.id).collect();
        have.retain(|k, _| ids.contains(k));
        if let Ok(json) = serde_json::to_string(&have) {
            self.inner.db.write(|c| kv_set(c, KEY, &json))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::{ProfileItem, QualityRef};
    use super::*;

    fn tf(q: &str) -> TrackFile {
        TrackFile { quality: Some(serde_json::json!({"quality": {"id": 1, "name": q}})), ..Default::default() }
    }

    fn item(id: i64, name: &str, allowed: bool) -> ProfileItem {
        ProfileItem { quality: Some(QualityRef { id, name: name.into() }), allowed, ..Default::default() }
    }

    #[test]
    fn quality_and_edition_in_plain_words() {
        let rel = SeenRelease { id: 1, title: "Panes".into(), tracks: 13, disambiguation: String::new() };
        let g = describe(&[tf("FLAC"), tf("FLAC")], Some(&rel), None).unwrap();
        assert_eq!(g.line, "FLAC, 13 tracks");
        assert_eq!(g.upgrade_note, None);
        let named = SeenRelease { disambiguation: "Newsletter Edit".into(), ..rel.clone() };
        assert_eq!(describe(&[tf("MP3-320")], Some(&named), None).unwrap().line, "MP3 320, Newsletter Edit");
        assert_eq!(quality_words("FLAC 24bit"), "FLAC 24-bit");
        assert!(describe(&[], Some(&rel), None).is_none(), "nothing imported, nothing said");
    }

    #[test]
    fn lossy_under_an_upgrading_profile_says_lidarr_keeps_looking() {
        let rel = SeenRelease { id: 1, title: "Panes".into(), tracks: 13, disambiguation: String::new() };
        let up = QualityProfile {
            id: 4,
            name: "FLAC + MP3 320".into(),
            upgrade_allowed: true,
            cutoff: Some(6),
            items: vec![item(4, "MP3-320", true), item(6, "FLAC", true)],
        };
        let g = describe(&[tf("MP3-320")], Some(&rel), Some(&up)).unwrap();
        assert_eq!(g.upgrade_note.as_deref(), Some("Lidarr will replace it with FLAC when one turns up."));
        // Already lossless, or a profile that keeps the first copy: no note.
        assert_eq!(describe(&[tf("FLAC")], Some(&rel), Some(&up)).unwrap().upgrade_note, None);
        let keep = QualityProfile { upgrade_allowed: false, ..up.clone() };
        assert_eq!(describe(&[tf("MP3-320")], Some(&rel), Some(&keep)).unwrap().upgrade_note, None);
    }
}
