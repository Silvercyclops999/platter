//! What a Bridge passes on for a paired desktop, checked against what
//! Lidarr has right now. [`RelayedWrite::check`] only looks at the method,
//! path and that the body is JSON; here every write's body is read field by
//! field, rebuilt from only the fields Platter sends, with the flags Platter
//! always uses, and anything that names a file or folder is looked up in
//! Lidarr first. Reads are limited to the pages Platter asks for, and keys
//! and passwords are taken out of what goes back.

use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::api::{Exclusion, LidarrGet, RootFolder, TrackFile, UNREACHABLE};
use super::changes::HIRES_PROFILE;
use super::paths::NormPath;
use super::write::{AddArtist, ConfigKind, NewRoot, RelayedWrite, StagingFiles, clear_exclusion};
use crate::error::{AppError, AppResult};

/// Most ids one relayed write may name.
const MAX_IDS: usize = 1000;

fn refuse(why: &str) -> AppError {
    AppError::Invalid(format!("{why} The Bridge didn't send it."))
}

/// A path as a list of folder names, or None when it has `..`, a NUL or
/// nothing in it.
pub fn safe_path(p: &str) -> Option<NormPath> {
    if p.contains('\0') || p.split(['/', '\\']).any(|s| s.trim() == "..") {
        return None;
    }
    Some(NormPath::parse(p)).filter(|n| !n.is_empty())
}

async fn fetch<T: DeserializeOwned>(get: &dyn LidarrGet, path: &str, query: &[(&str, &str)]) -> AppResult<T> {
    let resp = get.get(path, query).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
    if !(200..=299).contains(&resp.status) {
        return Err(AppError::Server(format!("Lidarr answered with an error ({}), so the Bridge didn't send the change.", resp.status)));
    }
    serde_json::from_slice(&resp.body).map_err(|_| AppError::Decode("Lidarr's answer couldn't be read, so the Bridge didn't send the change.".into()))
}

fn ids(v: &Value) -> Option<Vec<i64>> {
    let list = v.as_array()?;
    if list.is_empty() || list.len() > MAX_IDS {
        return None;
    }
    list.iter().map(|x| x.as_i64().filter(|n| *n > 0)).collect()
}

fn number_after(path: &str, prefix: &str) -> Option<i64> {
    path.strip_prefix(prefix)?.parse::<i64>().ok().filter(|n| *n > 0)
}

fn config_kind(path: &str) -> Option<(ConfigKind, i64)> {
    [ConfigKind::Naming, ConfigKind::MetadataProvider, ConfigKind::MediaManagement]
        .into_iter()
        .find_map(|k| number_after(path, &format!("{}/", k.path())).map(|id| (k, id)))
}

/// The one value Platter changes on each settings page (and puts back).
fn config_field(kind: ConfigKind) -> &'static str {
    match kind {
        ConfigKind::Naming => "renameTracks",
        ConfigKind::MetadataProvider => "writeAudioTags",
        ConfigKind::MediaManagement => "recycleBin",
    }
}

/// Check a write from a paired desktop against Lidarr (`get`) and the
/// staging folder, and give back the write to send: its body rebuilt from
/// only what Platter sends.
pub async fn vet(w: &RelayedWrite, get: &dyn LidarrGet, staging: &str) -> AppResult<RelayedWrite> {
    let body: Value = if w.body().trim().is_empty() { Value::Null } else { serde_json::from_str(w.body()).map_err(|_| refuse("That change wasn't readable."))? };
    let path = w.path();
    let staging_norm = safe_path(staging);
    let rebuilt = |v: Value| Ok(w.with_body(v.to_string()));
    match (w.method(), path) {
        ("POST", "/artist") => {
            let root_path = body["rootFolderPath"].as_str().and_then(|p| safe_path(p).map(|n| (p, n))).ok_or_else(|| refuse("That artist had no usable folder."))?;
            let roots: Vec<RootFolder> = fetch(get, "/rootfolder", &[]).await?;
            let Some(root) = roots.iter().find(|r| NormPath::parse(&r.path) == root_path.1) else {
                return Err(refuse("That artist's folder isn't one of Lidarr's root folders."));
            };
            let exclusions: Vec<Exclusion> = fetch(get, "/importlistexclusion", &[]).await?;
            let name = body["artistName"].as_str().unwrap_or_default();
            if name.trim().is_empty() || name.len() > 500 {
                return Err(refuse("That artist had no name."));
            }
            let cleared = clear_exclusion(&exclusions, body["foreignArtistId"].as_str().unwrap_or_default(), name)?;
            let (q, m) = (body["qualityProfileId"].as_i64().unwrap_or(0), body["metadataProfileId"].as_i64().unwrap_or(0));
            if q <= 0 || m <= 0 {
                return Err(refuse("That artist had no profiles."));
            }
            let in_staging = staging_norm.as_ref().is_some_and(|s| root_path.1.starts_with(s));
            let add = if in_staging || body["monitored"] != Value::Bool(true) {
                AddArtist::protected(cleared, name, q, m, &root.path)
            } else {
                AddArtist::managed(cleared, name, q, m, &root.path)
            };
            rebuilt(serde_json::to_value(add).unwrap_or_default())
        }
        ("POST", "/importlistexclusion") => {
            let id = body["foreignId"].as_str().map(|s| s.trim().to_ascii_lowercase()).filter(|s| crate::artistinfo::is_mbid(s));
            let name = body["artistName"].as_str().filter(|n| n.len() <= 500);
            match (id, name) {
                (Some(id), Some(name)) => rebuilt(json!({"foreignId": id, "artistName": name})),
                _ => Err(refuse("That exclusion had no MusicBrainz id.")),
            }
        }
        ("POST", "/rootfolder") => {
            let p = body["path"].as_str().unwrap_or_default();
            if staging_norm.is_none() || safe_path(p) != staging_norm {
                return Err(refuse("Platter only ever adds the staging folder as a root folder."));
            }
            let (q, m) = (body["defaultQualityProfileId"].as_i64().unwrap_or(0), body["defaultMetadataProfileId"].as_i64().unwrap_or(0));
            if q <= 0 || m <= 0 {
                return Err(refuse("That root folder had no profiles."));
            }
            rebuilt(serde_json::to_value(NewRoot::staging(p, q, m)).unwrap_or_default())
        }
        ("POST", "/qualityprofile") => {
            if body["name"].as_str() != Some(HIRES_PROFILE) || !body["items"].is_array() {
                return Err(refuse(&format!("Platter only ever adds the {HIRES_PROFILE} profile.")));
            }
            let have: Vec<Value> = fetch(get, "/qualityprofile", &[]).await?;
            if have.iter().any(|p| p["name"].as_str() == Some(HIRES_PROFILE)) {
                return Err(refuse(&format!("Lidarr already has the {HIRES_PROFILE} profile.")));
            }
            let mut b = body.clone();
            if let Some(o) = b.as_object_mut() {
                o.remove("id");
            }
            rebuilt(b)
        }
        ("POST", "/command") => match body["name"].as_str() {
            Some("AlbumSearch") => {
                let album_ids = ids(&body["albumIds"]).ok_or_else(|| refuse("That search named no albums."))?;
                rebuilt(json!({"name": "AlbumSearch", "albumIds": album_ids}))
            }
            Some("ManualImport") => {
                let files = body["files"].as_array().filter(|f| !f.is_empty() && f.len() <= MAX_IDS).ok_or_else(|| refuse("That import named no files."))?;
                // Only files Lidarr itself found and couldn't match, onto
                // the album's current release, never replacing anything.
                let unmapped: Vec<TrackFile> = fetch(get, "/trackfile", &[("unmapped", "true")]).await?;
                let known: std::collections::HashSet<NormPath> = unmapped.iter().map(|f| NormPath::parse(&f.path)).collect();
                let mut out = Vec::new();
                for f in files {
                    let p = f["path"].as_str().unwrap_or_default();
                    if !safe_path(p).is_some_and(|n| known.contains(&n)) {
                        return Err(refuse("That import names a file Lidarr hasn't listed as unmatched."));
                    }
                    let n = |k: &str| f[k].as_i64().filter(|v| *v > 0);
                    let (Some(artist), Some(album), Some(release)) = (n("artistId"), n("albumId"), n("albumReleaseId")) else {
                        return Err(refuse("That import didn't name its album."));
                    };
                    let tracks = ids(&f["trackIds"]).ok_or_else(|| refuse("That import didn't name its tracks."))?;
                    out.push(json!({
                        "path": p, "artistId": artist, "albumId": album, "albumReleaseId": release, "trackIds": tracks,
                        "quality": f["quality"].clone(), "indexerFlags": 0,
                        "downloadId": f["downloadId"].as_str().unwrap_or_default(), "disableReleaseSwitching": true,
                    }));
                }
                rebuilt(json!({"name": "ManualImport", "importMode": "move", "replaceExistingFiles": false, "files": out}))
            }
            _ => Err(refuse("That command isn't one Platter sends.")),
        },
        ("PUT", "/album/monitor") => {
            let album_ids = ids(&body["albumIds"]).ok_or_else(|| refuse("That change named no albums."))?;
            let monitored = body["monitored"].as_bool().ok_or_else(|| refuse("That change didn't say on or off."))?;
            rebuilt(json!({"albumIds": album_ids, "monitored": monitored}))
        }
        ("PUT", _) if w.switch_off_album().is_some() => {
            w.confirm_with(get).await?;
            Ok(w.clone())
        }
        ("PUT", p) => {
            let (kind, id) = config_kind(p).ok_or_else(|| refuse("That isn't a settings page Platter writes."))?;
            let current: Value = fetch(get, kind.path(), &[]).await?;
            let (Some(now), Some(sent)) = (current.as_object(), body.as_object()) else {
                return Err(refuse("That settings page wasn't readable."));
            };
            if now.get("id").and_then(Value::as_i64) != Some(id) {
                return Err(refuse("That settings page isn't the one Lidarr has."));
            }
            let field = config_field(kind);
            let keys: std::collections::BTreeSet<&String> = now.keys().chain(sent.keys()).collect();
            if keys.into_iter().any(|k| k != field && now.get(k) != sent.get(k)) {
                return Err(refuse("That change would touch more of Lidarr's settings than Platter ever changes."));
            }
            let value = sent.get(field).cloned().unwrap_or(Value::Null);
            let fits = match kind {
                ConfigKind::Naming => value.is_boolean(),
                ConfigKind::MetadataProvider => value.as_str().is_some_and(|s| s.len() <= 40),
                ConfigKind::MediaManagement => value.as_str().is_some_and(|s| s.is_empty() || safe_path(s).is_some()),
            };
            if !fits {
                return Err(refuse("That setting's new value isn't one Platter uses."));
            }
            let mut b = current.clone();
            b[field] = value;
            rebuilt(b)
        }
        ("DELETE", "/trackfile/bulk") => {
            let list = ids(&body["trackFileIds"]).ok_or_else(|| refuse("That named no files."))?;
            let mut files = Vec::with_capacity(list.len());
            for id in &list {
                let f: TrackFile = fetch(get, &format!("/trackfile/{id}"), &[]).await?;
                if f.id != *id {
                    return Err(refuse("Lidarr doesn't have that file."));
                }
                files.push(f);
            }
            let ok = StagingFiles::check(&files, staging).map_err(|e| AppError::Invalid(format!("{e} The Bridge didn't send it.")))?;
            rebuilt(json!({"trackFileIds": ok.ids()}))
        }
        ("DELETE", p) if number_after(p, "/importlistexclusion/").is_some() => Ok(w.clone()),
        ("DELETE", p) if number_after(p, "/rootfolder/").is_some() => {
            let id = number_after(p, "/rootfolder/").unwrap_or(0);
            let roots: Vec<RootFolder> = fetch(get, "/rootfolder", &[]).await?;
            let root = roots.iter().find(|r| r.id == id).ok_or_else(|| refuse("Lidarr doesn't have that root folder."))?;
            let rp = NormPath::parse(&root.path);
            if staging_norm.as_ref() != Some(&rp) {
                return Err(refuse("Platter only ever removes the staging root folder."));
            }
            let artists: Vec<super::api::Artist> = fetch(get, "/artist", &[]).await?;
            if artists.iter().any(|a| NormPath::parse(&a.path).starts_with(&rp)) || roots.iter().any(|r| r.id != id && NormPath::parse(&r.path).starts_with(&rp)) {
                return Err(refuse("That root folder still holds artists or another root."));
            }
            Ok(w.clone())
        }
        ("DELETE", p) if number_after(p, "/qualityprofile/").is_some() => {
            let id = number_after(p, "/qualityprofile/").unwrap_or(0);
            let have: Vec<Value> = fetch(get, "/qualityprofile", &[]).await?;
            if !have.iter().any(|q| q["id"].as_i64() == Some(id) && q["name"].as_str() == Some(HIRES_PROFILE)) {
                return Err(refuse(&format!("Platter only ever removes its own {HIRES_PROFILE} profile.")));
            }
            let artists: Vec<super::api::Artist> = fetch(get, "/artist", &[]).await?;
            if artists.iter().any(|a| a.quality_profile_id == Some(id)) {
                return Err(refuse("Lidarr artists use that profile."));
            }
            Ok(w.clone())
        }
        _ => Err(refuse("That isn't one of the changes Platter makes to Lidarr.")),
    }
}

/// The pages a Bridge reads for a desktop: exactly what Platter asks for.
const READS: &[&str] = &[
    "/system/status",
    "/rootfolder",
    "/artist",
    "/artist/lookup",
    "/album",
    "/album/lookup",
    "/config/naming",
    "/config/metadataprovider",
    "/config/mediamanagement",
    "/qualityprofile",
    "/metadataprofile",
    "/importlist",
    "/importlistexclusion",
    "/notification",
    "/command",
    "/queue",
    "/queue/details",
    "/history",
    "/history/since",
    "/track",
    "/trackfile",
];

/// The query names those reads use.
const READ_QUERY: &[&str] = &[
    "artistId",
    "albumId",
    "albumIds",
    "albumReleaseId",
    "foreignAlbumId",
    "term",
    "page",
    "pageSize",
    "sortKey",
    "sortDirection",
    "date",
    "unmapped",
    "folder",
    "filterExistingFiles",
    "path",
    "includeFiles",
    "allowFoldersWithoutTrailingSlashes",
];

/// Whether a read may be passed on. Folder reads (`/filesystem`,
/// `/manualimport`) also need [`folder_read_ok`].
pub fn read_allowed(path: &str, query: &[(String, String)]) -> bool {
    let page = READS.contains(&path)
        || number_after(path, "/artist/").is_some()
        || number_after(path, "/album/").is_some()
        || number_after(path, "/trackfile/").is_some()
        || path == "/filesystem"
        || path == "/manualimport";
    // A name given twice is refused: the folder check reads the first one,
    // and Lidarr might read another.
    let mut seen = std::collections::HashSet::new();
    page && query.len() <= 20
        && query.iter().all(|(k, v)| READ_QUERY.contains(&k.as_str()) && v.len() <= 1000 && !v.contains('\0') && seen.insert(k.to_ascii_lowercase()))
}

/// A folder read is only for a folder inside one of Lidarr's root folders
/// or the staging folder.
pub async fn folder_read_ok(get: &dyn LidarrGet, path: &str, query: &[(String, String)], staging: &str) -> AppResult<bool> {
    let key = match path {
        "/filesystem" => "path",
        "/manualimport" => "folder",
        _ => return Ok(true),
    };
    let Some(folder) = query.iter().find(|(k, _)| k == key).and_then(|(_, v)| safe_path(v)) else { return Ok(false) };
    if safe_path(staging).is_some_and(|s| folder.starts_with(&s)) {
        return Ok(true);
    }
    let roots: Vec<RootFolder> = fetch(get, "/rootfolder", &[]).await?;
    Ok(roots.iter().any(|r| folder.starts_with(&NormPath::parse(&r.path))))
}

fn secret_name(k: &str) -> bool {
    let k = k.to_ascii_lowercase().replace(['_', '-'], "");
    ["apikey", "password", "passphrase", "secret", "token", "cookie", "authorization", "consumerkey", "accesskey", "privatekey"].iter().any(|s| k.contains(s))
}

fn strip(v: &mut Value) {
    match v {
        Value::Object(m) => {
            // Lidarr's settings fields: {"name": "apiKey", "privacy": "apiKey", "value": ...}
            let private = m.get("privacy").and_then(Value::as_str).is_some_and(|p| !p.eq_ignore_ascii_case("normal"))
                || m.get("name").and_then(Value::as_str).is_some_and(secret_name);
            if private && m.contains_key("value") {
                m.insert("value".into(), Value::String(String::new()));
            }
            for (k, x) in m.iter_mut() {
                if secret_name(k) && !x.is_object() && !x.is_array() {
                    *x = Value::String(String::new());
                } else {
                    strip(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip),
        _ => {}
    }
}

/// Lidarr's answer with keys, passwords and tokens blanked. An answer that
/// isn't JSON goes back empty.
pub fn strip_secrets(body: &[u8]) -> Bytes {
    match serde_json::from_slice::<Value>(body) {
        Ok(mut v) => {
            strip(&mut v);
            Bytes::from(v.to_string())
        }
        Err(_) => Bytes::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artistinfo::http::HttpResponse;
    use crate::lidarr::api::Route;
    use async_trait::async_trait;
    use std::collections::HashMap;

    const STAGING: &str = "/data/staging/music";

    struct Fake(HashMap<String, Value>);

    #[async_trait]
    impl LidarrGet for Fake {
        async fn get(&self, path: &str, _q: &[(&str, &str)]) -> AppResult<HttpResponse> {
            match self.0.get(path) {
                Some(v) => Ok(HttpResponse { status: 200, body: Bytes::from(v.to_string()), ..Default::default() }),
                None => Ok(HttpResponse { status: 404, ..Default::default() }),
            }
        }
        fn route(&self) -> Route {
            Route::Home
        }
    }

    fn lidarr() -> Fake {
        let mut m = HashMap::new();
        m.insert("/rootfolder".into(), json!([{"id": 1, "path": "/data/media/Music"}, {"id": 2, "path": STAGING}]));
        m.insert("/importlistexclusion".into(), json!([{"foreignId": "b0000000-0000-0000-0000-000000000002", "artistName": "Quiet Hours"}]));
        m.insert("/artist".into(), json!([{"id": 5, "artistName": "Northlight", "path": "/data/media/Music/Northlight", "qualityProfileId": 2}]));
        m.insert("/qualityprofile".into(), json!([{"id": 2, "name": "Lossless"}, {"id": 7, "name": HIRES_PROFILE}]));
        m.insert("/trackfile/11".into(), json!({"id": 11, "path": "/data/staging/music/A/B/01.flac"}));
        m.insert("/trackfile/12".into(), json!({"id": 12, "path": "/data/media/Music/A/B/01.flac"}));
        m.insert("/trackfile".into(), json!([{"id": 0, "path": "/data/media/Music/Northlight/First/03.flac"}]));
        m.insert("/config/naming".into(), json!({"id": 1, "renameTracks": true, "standardTrackFormat": "{Track Title}"}));
        Fake(m)
    }

    async fn vetted(m: &str, p: &str, b: Value) -> AppResult<Value> {
        let body = if b.is_null() { String::new() } else { b.to_string() };
        let w = RelayedWrite::check(m, p, &body)?;
        let out = vet(&w, &lidarr(), STAGING).await?;
        Ok(if out.body().is_empty() { Value::Null } else { serde_json::from_str(out.body()).unwrap() })
    }

    #[tokio::test]
    async fn forged_artist_adds_are_refused_or_made_safe() {
        let good = json!({"artistName": "Northlight", "foreignArtistId": "a0000000-0000-0000-0000-000000000001", "qualityProfileId": 2,
            "metadataProfileId": 3, "rootFolderPath": STAGING, "monitored": true, "monitorNewItems": "all",
            "addOptions": {"monitor": "all", "searchForMissingAlbums": true}, "tags": [1]});
        let v = vetted("POST", "/artist", good.clone()).await.unwrap();
        assert_eq!(v["monitored"], false, "in staging: never monitored");
        assert_eq!(v["monitorNewItems"], "none");
        assert_eq!(v["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));
        assert!(v.get("tags").is_none(), "only the fields Platter sends");
        let mut lib = good.clone();
        lib["rootFolderPath"] = json!("/data/media/Music");
        assert_eq!(vetted("POST", "/artist", lib).await.unwrap()["addOptions"]["searchForMissingAlbums"], false);
        for root in ["/etc", "/data/staging/music/../../etc", "/data/media/Music/Sub"] {
            let mut bad = good.clone();
            bad["rootFolderPath"] = json!(root);
            assert!(vetted("POST", "/artist", bad).await.is_err(), "{root}");
        }
        let mut excluded = good.clone();
        excluded["foreignArtistId"] = json!("b0000000-0000-0000-0000-000000000002");
        assert!(vetted("POST", "/artist", excluded).await.is_err(), "exclusions respected");
    }

    #[tokio::test]
    async fn forged_deletes_imports_and_settings_are_refused() {
        assert_eq!(vetted("DELETE", "/trackfile/bulk", json!({"trackFileIds": [11]})).await.unwrap(), json!({"trackFileIds": [11]}));
        assert!(vetted("DELETE", "/trackfile/bulk", json!({"trackFileIds": [11, 12]})).await.is_err(), "a library file");
        assert!(vetted("DELETE", "/trackfile/bulk", json!({"trackFileIds": [99]})).await.is_err(), "unknown id");
        assert!(vetted("DELETE", "/rootfolder/1", Value::Null).await.is_err(), "the library root");
        assert!(vetted("DELETE", "/rootfolder/2", Value::Null).await.is_ok(), "the empty staging root");
        assert!(vetted("DELETE", "/qualityprofile/2", Value::Null).await.is_err(), "not Platter's profile");
        assert!(vetted("DELETE", "/qualityprofile/7", Value::Null).await.is_ok());
        assert!(vetted("POST", "/rootfolder", json!({"path": "/", "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1})).await.is_err());
        let r = vetted("POST", "/rootfolder", json!({"path": STAGING, "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1, "defaultMonitorOption": "all"})).await.unwrap();
        assert_eq!(r["defaultMonitorOption"], "none");
        assert!(vetted("POST", "/qualityprofile", json!({"name": "Anything", "items": []})).await.is_err());
        let file = |p: &str| json!({"path": p, "artistId": 5, "albumId": 9, "albumReleaseId": 90, "trackIds": [3]});
        let ok = vetted("POST", "/command", json!({"name": "ManualImport", "importMode": "copy", "replaceExistingFiles": true,
            "files": [file("/data/media/Music/Northlight/First/03.flac")]})).await.unwrap();
        assert_eq!((ok["importMode"].as_str(), ok["replaceExistingFiles"].as_bool()), (Some("move"), Some(false)));
        assert_eq!(ok["files"][0]["disableReleaseSwitching"], true);
        assert!(vetted("POST", "/command", json!({"name": "ManualImport", "files": [file("/etc/passwd")]})).await.is_err());
        // Settings: only the one value Platter changes.
        let page = json!({"id": 1, "renameTracks": false, "standardTrackFormat": "{Track Title}"});
        assert_eq!(vetted("PUT", "/config/naming/1", page).await.unwrap()["renameTracks"], false);
        let sneaky = json!({"id": 1, "renameTracks": false, "standardTrackFormat": "../{Track Title}"});
        assert!(vetted("PUT", "/config/naming/1", sneaky).await.is_err());
        assert!(vetted("PUT", "/config/naming/1", json!({"id": 1, "renameTracks": "yes", "standardTrackFormat": "{Track Title}"})).await.is_err());
        let m = vetted("PUT", "/album/monitor", json!({"albumIds": [1], "monitored": true, "extra": 1})).await.unwrap();
        assert_eq!(m, json!({"albumIds": [1], "monitored": true}));
        assert!(vetted("PUT", "/album/monitor", json!({"albumIds": [-1], "monitored": true})).await.is_err());
    }

    #[tokio::test]
    async fn reads_are_limited_and_keys_taken_out() {
        let q = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert!(read_allowed("/artist", &[]));
        assert!(read_allowed("/album/12", &[]));
        assert!(read_allowed("/trackfile", &q("albumId", "3")));
        for p in ["/config/host", "/system/backup", "/log/file", "/indexer", "/downloadclient", "/config/ui"] {
            assert!(!read_allowed(p, &[]), "{p}");
        }
        assert!(!read_allowed("/artist", &q("apikey", "x")));
        // A name given twice: the folder check and Lidarr could read different ones.
        let two = |a: (&str, &str), b: (&str, &str)| vec![(a.0.to_string(), a.1.to_string()), (b.0.to_string(), b.1.to_string())];
        assert!(!read_allowed("/filesystem", &two(("path", "/data/media/Music/A"), ("path", "/config"))));
        assert!(!read_allowed("/track", &two(("albumId", "1"), ("AlbumId", "2"))));
        assert!(read_allowed("/track", &two(("albumId", "1"), ("artistId", "2"))));
        let l = lidarr();
        assert!(folder_read_ok(&l, "/filesystem", &q("path", "/data/media/Music/A"), STAGING).await.unwrap());
        assert!(folder_read_ok(&l, "/manualimport", &q("folder", "/data/staging/music/A"), STAGING).await.unwrap());
        assert!(!folder_read_ok(&l, "/filesystem", &q("path", "/config"), STAGING).await.unwrap());
        assert!(!folder_read_ok(&l, "/filesystem", &q("path", "/data/media/Music/../../config"), STAGING).await.unwrap());
        let body = json!([{"id": 1, "name": "Plex", "apiKey": "k1", "fields": [{"name": "authToken", "value": "t1"}, {"name": "host", "value": "h"},
            {"name": "x", "privacy": "password", "value": "p1"}]}]);
        let out = String::from_utf8(strip_secrets(body.to_string().as_bytes()).to_vec()).unwrap();
        for s in ["k1", "t1", "p1"] {
            assert!(!out.contains(s), "{s} in {out}");
        }
        assert!(out.contains("\"h\""));
        assert!(strip_secrets(b"<html>apiKey=1</html>").is_empty());
    }
}
