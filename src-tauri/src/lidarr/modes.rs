//! Managed and Protected artists, chosen per artist automatically.
//!
//! - Managed (the default): Lidarr owns the artist's folder, imports,
//!   renames and upgrades in place. Platter requests, watches, and makes
//!   sure Plex shows the result.
//! - Protected: the artist's files are curated. Lidarr only ever downloads
//!   them into the staging folder no Plex library scans, and Platter makes
//!   every change in the library itself.
//!
//! An artist is Protected when any of its files carry a release status other
//! than released (ARCHIVE_*, STATUS/VERSION, Picard's release status or the
//! custom tag, all read by `tags::provenance`), when an album is typed demo,
//! unreleased, leak or bootleg, when the artist is in the library but on
//! Lidarr's import list exclusions, or when the user says so. Protection
//! follows folders and release groups too: an artist whose albums sit inside
//! a protected artist's folder, and a Lidarr artist credited on a release
//! group found there (a collaboration filed under the folder artist), are
//! Protected as well, so they are never Managed and never downloaded into
//! new folders.
//!
//! Artists are matched by MusicBrainz id (or by being the same folder),
//! never by name. This module reads; the user's choices live in the shared
//! store (`artist:<key>`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::paths::NormPath;
use crate::error::AppResult;
use crate::identity::fold::fold_artist;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Managed,
    Protected,
}

/// What the user picked for an artist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModeSetting {
    #[default]
    Auto,
    Managed,
    Protected,
}

/// What better copies aim for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// Lossless 16-bit.
    #[default]
    Lossless,
    /// Hi-res 24-bit.
    HiRes,
}

/// Per-artist choices, shared across devices (`artist:<key>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPrefs {
    #[serde(default)]
    pub mode: ModeSetting,
    /// Add new official albums automatically (Protected). None: ask the
    /// first time.
    #[serde(default)]
    pub auto_add: Option<bool>,
    /// Look for better copies (Protected).
    #[serde(default)]
    pub hunt: bool,
    #[serde(default)]
    pub target: Target,
    /// The artist's name when the choice was made (for Settings).
    #[serde(default)]
    pub name: String,
}

/// One library artist, as the modes need it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryArtist {
    pub artist_id: String,
    pub name: String,
    pub mbid: Option<String>,
    pub files: usize,
    /// Files whose tags say anything but released, by status.
    pub curated: BTreeMap<String, usize>,
    /// Albums typed demo, unreleased, leak or bootleg.
    pub typed_unreleased: Vec<String>,
    /// Each album's folder (server paths), with its title and release group.
    pub albums: Vec<LibraryAlbumDir>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryAlbumDir {
    pub title: String,
    pub folder: String,
    pub release_group: Option<String>,
}

impl LibraryArtist {
    pub fn curated_files(&self) -> usize {
        self.curated.values().sum()
    }

    /// The artist's folder: the folder holding most of its album folders.
    pub fn folder(&self) -> Option<String> {
        let mut counts: BTreeMap<String, (usize, String)> = BTreeMap::new();
        for a in &self.albums {
            let Some(parent) = parent_raw(&a.folder) else { continue };
            let e = counts.entry(NormPath::parse(&parent).key()).or_insert((0, parent));
            e.0 += 1;
        }
        counts.into_values().max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1))).map(|(_, p)| p)
    }
}

/// `/a/b/c` → `/a/b`, keeping the path's own style.
pub fn parent_raw(path: &str) -> Option<String> {
    let t = path.trim_end_matches(['/', '\\']);
    let i = t.rfind(['/', '\\'])?;
    (i > 0).then(|| t[..i].to_string())
}

/// A Lidarr artist, as the modes need it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LidarrArtistRef {
    pub id: i64,
    pub name: String,
    pub mbid: String,
    pub path: String,
}

/// A Lidarr album: which artist, which release group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LidarrAlbumRef {
    pub artist_id: i64,
    pub release_group: String,
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct ModeInputs {
    pub library: Vec<LibraryArtist>,
    pub lidarr_artists: Vec<LidarrArtistRef>,
    pub lidarr_albums: Vec<LidarrAlbumRef>,
    /// MusicBrainz ids on Lidarr's import list exclusions.
    pub exclusions: Vec<String>,
    /// The user's choices by artist key.
    pub prefs: HashMap<String, ArtistPrefs>,
}

/// One artist's mode, with why, for Settings and for every decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistMode {
    /// The key choices are kept under: MusicBrainz id, else `name:<folded>`.
    pub key: String,
    pub name: String,
    /// The library artist, when in the library.
    pub artist_id: Option<String>,
    pub mbid: Option<String>,
    pub lidarr_id: Option<i64>,
    /// Lidarr's folder for the artist.
    pub lidarr_path: Option<String>,
    /// The library folder (server path), when known.
    pub folder: Option<String>,
    pub mode: Mode,
    pub setting: ModeSetting,
    /// What Auto would pick.
    pub auto: Mode,
    /// One line: "Protected: 403 files tagged unreleased or leaked".
    pub line: String,
    pub reasons: Vec<String>,
    pub files: usize,
    pub curated_files: usize,
    pub excluded: bool,
    /// The library release groups (Protected artists' groups are protected).
    #[serde(skip)]
    pub release_groups: Vec<String>,
}

impl ArtistMode {
    pub fn protected(&self) -> bool {
        self.mode == Mode::Protected
    }
}

/// The key an artist's choices are kept under.
pub fn artist_key(mbid: Option<&str>, name: &str) -> String {
    match mbid.map(|m| m.trim().to_ascii_lowercase()).filter(|m| crate::artistinfo::is_mbid(m)) {
        Some(m) => m,
        None => format!("name:{}", fold_artist(name)),
    }
}

/// Subformats (Plex's album types) that mark curated material.
const TYPED: [&str; 6] = ["demo", "unreleased", "leak", "leaked", "bootleg", "snippet"];

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn kinds_words(curated: &BTreeMap<String, usize>) -> String {
    let word = |k: &str| match k {
        "unreleased" => "unreleased",
        "leaked" | "leak" => "leaked",
        "alternate" | "demo" => "demo or alternate",
        _ => "with a curated status",
    };
    let order = ["unreleased", "leaked", "demo or alternate", "with a curated status"];
    let words: Vec<&str> = order.into_iter().filter(|w| curated.keys().any(|k| word(k) == *w)).collect();
    match words.len() {
        0 => "curated".into(),
        1 => words[0].into(),
        _ => format!("{} or {}", words[..words.len() - 1].join(", "), words[words.len() - 1]),
    }
}

/// The modes of every library artist and every Lidarr artist.
pub fn assign(inp: &ModeInputs) -> Vec<ArtistMode> {
    let excluded: HashSet<String> = inp.exclusions.iter().map(|m| m.trim().to_ascii_lowercase()).collect();
    let pref = |key: &str| inp.prefs.get(key).map(|p| p.mode).unwrap_or_default();
    let mut out: Vec<ArtistMode> = Vec::new();

    // ---- Library artists, from their own files and choices.
    for a in &inp.library {
        let mbid = a.mbid.as_ref().map(|m| m.trim().to_ascii_lowercase()).filter(|m| crate::artistinfo::is_mbid(m));
        let key = artist_key(mbid.as_deref(), &a.name);
        let folder = a.folder();
        let lidarr = inp.lidarr_artists.iter().find(|l| {
            mbid.as_deref().is_some_and(|m| l.mbid.eq_ignore_ascii_case(m))
                || folder.as_deref().is_some_and(|f| !l.path.trim().is_empty() && NormPath::parse(&l.path) == NormPath::parse(f))
        });
        let is_excluded = mbid.as_deref().is_some_and(|m| excluded.contains(m));
        let mut reasons = Vec::new();
        let n = a.curated_files();
        if n > 0 {
            reasons.push(format!("{} tagged {}", plural(n, "file", "files"), kinds_words(&a.curated)));
        }
        if !a.typed_unreleased.is_empty() {
            reasons.push(format!("{} typed demo, unreleased or leak", plural(a.typed_unreleased.len(), "album", "albums")));
        }
        if is_excluded {
            reasons.push("on Lidarr's import list exclusions".into());
        }
        out.push(ArtistMode {
            key,
            name: a.name.clone(),
            artist_id: Some(a.artist_id.clone()),
            mbid,
            lidarr_id: lidarr.map(|l| l.id),
            lidarr_path: lidarr.map(|l| l.path.clone()).filter(|p| !p.trim().is_empty()),
            folder,
            mode: Mode::Managed,
            setting: ModeSetting::Auto,
            auto: if reasons.is_empty() { Mode::Managed } else { Mode::Protected },
            line: String::new(),
            reasons,
            files: a.files,
            curated_files: n,
            excluded: is_excluded,
            release_groups: a.albums.iter().filter_map(|x| x.release_group.as_ref().map(|r| r.to_ascii_lowercase())).collect(),
        });
    }

    // ---- Lidarr artists the library doesn't have.
    let matched: HashSet<i64> = out.iter().filter_map(|m| m.lidarr_id).collect();
    for l in &inp.lidarr_artists {
        if matched.contains(&l.id) {
            continue;
        }
        let mbid = Some(l.mbid.trim().to_ascii_lowercase()).filter(|m| crate::artistinfo::is_mbid(m));
        out.push(ArtistMode {
            key: artist_key(mbid.as_deref(), &l.name),
            name: l.name.clone(),
            artist_id: None,
            mbid,
            lidarr_id: Some(l.id),
            lidarr_path: Some(l.path.clone()).filter(|p| !p.trim().is_empty()),
            folder: None,
            mode: Mode::Managed,
            setting: ModeSetting::Auto,
            auto: Mode::Managed,
            line: String::new(),
            reasons: Vec::new(),
            files: 0,
            curated_files: 0,
            excluded: false,
            release_groups: Vec::new(),
        });
    }

    // ---- Protection follows folders and release groups. Seeds: artists
    // protected by their own signals or by the user's choice.
    let seed = |m: &ArtistMode| match pref(&m.key) {
        ModeSetting::Protected => true,
        ModeSetting::Managed => false,
        ModeSetting::Auto => m.auto == Mode::Protected,
    };
    let mut protected: Vec<bool> = out.iter().map(seed).collect();
    let library_by_id: HashMap<&str, &LibraryArtist> = inp.library.iter().map(|a| (a.artist_id.as_str(), a)).collect();
    loop {
        let mut changed = false;
        // Protected folders and the release groups found inside them.
        let mut folders: Vec<(NormPath, String)> = Vec::new();
        let mut groups: HashMap<String, (String, String)> = HashMap::new();
        for (i, m) in out.iter().enumerate() {
            if !protected[i] {
                continue;
            }
            if let Some(f) = &m.folder {
                folders.push((NormPath::parse(f), m.name.clone()));
            }
        }
        for a in &inp.library {
            for al in &a.albums {
                let p = NormPath::parse(&al.folder);
                if let (Some(rg), Some((_, owner))) = (&al.release_group, folders.iter().find(|(f, _)| !f.is_empty() && p.starts_with(f))) {
                    groups.entry(rg.to_ascii_lowercase()).or_insert((al.title.clone(), owner.clone()));
                }
            }
        }
        for i in 0..out.len() {
            if protected[i] || pref(&out[i].key) == ModeSetting::Managed {
                continue;
            }
            let m = &out[i];
            let mut why: Option<String> = None;
            if let Some(a) = m.artist_id.as_deref().and_then(|id| library_by_id.get(id)) {
                for al in &a.albums {
                    let p = NormPath::parse(&al.folder);
                    if let Some((_, owner)) = folders.iter().find(|(f, _)| !f.is_empty() && p.starts_with(f)) {
                        why = Some(format!("has {} inside {owner}'s protected folder", al.title));
                        break;
                    }
                }
            }
            if why.is_none() {
                why = m.release_groups.iter().find_map(|rg| groups.get(rg)).map(|(t, owner)| format!("shares {t} with protected {owner}"));
            }
            if why.is_none()
                && let Some(lid) = m.lidarr_id
            {
                why = inp
                    .lidarr_albums
                    .iter()
                    .filter(|x| x.artist_id == lid)
                    .find_map(|x| groups.get(&x.release_group.to_ascii_lowercase()))
                    .map(|(t, owner)| format!("credited on {t} in {owner}'s protected folder"));
            }
            if let Some(w) = why {
                out[i].reasons.push(w);
                out[i].auto = Mode::Protected;
                protected[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for (i, m) in out.iter_mut().enumerate() {
        m.setting = pref(&m.key);
        m.mode = if protected[i] { Mode::Protected } else { Mode::Managed };
        m.line = line(m);
    }
    out.sort_by(|a, b| (b.mode == Mode::Protected).cmp(&(a.mode == Mode::Protected)).then(fold_artist(&a.name).cmp(&fold_artist(&b.name))));
    out
}

fn line(m: &ArtistMode) -> String {
    let first = m.reasons.first().cloned();
    let more = m.reasons.len().saturating_sub(1);
    let with_more = |s: String| if more > 0 { format!("{s} (and {})", plural(more, "more reason", "more reasons")) } else { s };
    match (m.mode, m.setting) {
        (Mode::Protected, ModeSetting::Protected) if m.auto == Mode::Managed => "Protected: you chose Protected".into(),
        (Mode::Protected, ModeSetting::Protected) => format!("Protected: you chose Protected; {}", first.unwrap_or_default()),
        (Mode::Protected, _) => format!("Protected: {}", with_more(first.unwrap_or_else(|| "curated files".into()))),
        (Mode::Managed, ModeSetting::Managed) if m.auto == Mode::Protected => {
            format!("Managed: you chose Managed (Auto would protect it: {})", first.unwrap_or_default())
        }
        (Mode::Managed, ModeSetting::Managed) => "Managed: you chose Managed".into(),
        (Mode::Managed, _) if m.artist_id.is_none() => "Managed: in Lidarr only, nothing of it in your library yet".into(),
        (Mode::Managed, _) => "Managed: all files official, no curated tags".into(),
    }
}

/// The library's artists with their files' statuses, album types and
/// folders (MusicBrainz ids are filled in by the caller).
pub fn library_artists(c: &Connection) -> AppResult<Vec<LibraryArtist>> {
    let mut by_artist: BTreeMap<String, LibraryArtist> = BTreeMap::new();
    let mut album_seen: HashSet<String> = HashSet::new();
    {
        let mut st = c.prepare(
            "SELECT al.artist_id, al.artist_title, al.id, al.title, al.mbid, al.subformats, ast.status
             FROM albums al LEFT JOIN album_status ast ON ast.album_id = al.id",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        for row in rows {
            let (aid, name, alid, title, _mbid, subs, status) = row?;
            let a = by_artist.entry(aid.clone()).or_insert_with(|| LibraryArtist { artist_id: aid, name, ..Default::default() });
            let subs: Vec<String> = subs.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            let typed = subs.iter().any(|s| TYPED.contains(&s.trim().to_ascii_lowercase().as_str()))
                || matches!(status.as_deref(), Some("taggedUnreleased") | Some("confirmedUnreleased"));
            if typed && album_seen.insert(alid) {
                a.typed_unreleased.push(title);
            }
        }
    }
    let mut folders: HashMap<String, BTreeSet<(String, String, Option<String>)>> = HashMap::new();
    let mut st = c.prepare(
        "SELECT al.artist_id, al.title, al.mbid, t.file, coalesce(nullif(trim(ft.version), ''), ft.release)
         FROM tracks t JOIN albums al ON al.id = t.album_id
         LEFT JOIN file_tags ft ON ft.part_key = t.part_key",
    )?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?))
    })?;
    for row in rows {
        let (aid, title, mbid, file, status) = row?;
        let Some(a) = by_artist.get_mut(&aid) else { continue };
        a.files += 1;
        if let Some(s) = status.map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty() && s != "released") {
            *a.curated.entry(s).or_default() += 1;
        }
        if let Some(dir) = file.as_deref().and_then(parent_raw) {
            folders.entry(aid).or_default().insert((dir, title, mbid.filter(|m| crate::artistinfo::is_mbid(m))));
        }
    }
    for (aid, set) in folders {
        if let Some(a) = by_artist.get_mut(&aid) {
            a.albums = set.into_iter().map(|(folder, title, release_group)| LibraryAlbumDir { title, folder, release_group }).collect();
        }
    }
    Ok(by_artist.into_values().collect())
}

// ---------------------------------------------------------------- the service

/// How long worked-out modes are reused.
const MODES_TTL_S: i64 = 10 * 60;

impl super::Lidarr {
    pub(crate) fn shared(&self) -> &super::shared::Shared {
        &self.inner.shared
    }

    /// Claim a piece of work for this device among all of the user's
    /// devices (see [`super::shared`]): act only on `Ours`.
    pub fn claim(&self, event: &str) -> crate::error::AppResult<super::shared::ClaimState> {
        self.inner.shared.claim(event)
    }

    /// Every artist's choices, by key.
    pub fn artist_prefs(&self) -> HashMap<String, ArtistPrefs> {
        self.inner.shared.list::<ArtistPrefs>("artist:").into_iter().collect()
    }

    pub fn prefs_for(&self, key: &str) -> ArtistPrefs {
        self.inner.shared.get(&format!("artist:{key}")).unwrap_or_default()
    }

    pub fn save_prefs(&self, key: &str, p: &ArtistPrefs) -> AppResult<()> {
        self.inner.shared.put(&format!("artist:{key}"), p)?;
        *self.modes_cache.lock() = None;
        Ok(())
    }

    /// The modes, worked out from the library, Lidarr's artists, albums and
    /// exclusions, and the user's choices. Reused for a few minutes.
    pub async fn modes_with(&self, client: &super::api::LidarrClient, snap: &super::check::LidarrSnapshot) -> AppResult<Vec<ArtistMode>> {
        let now = (self.inner.clock)();
        if let Some((at, m)) = self.modes_cache.lock().clone()
            && now - at < MODES_TTL_S
        {
            return Ok(m);
        }
        // The Bridge can't read the library's tags; it works from what a
        // desktop worked out.
        if self.bridge_mode() {
            let out = self.shared_modes().ok_or_else(|| {
                crate::error::AppError::Invalid("Waiting for a desktop Platter to share which artists are protected.".into())
            })?;
            *self.modes_cache.lock() = Some((now, out.clone()));
            return Ok(out);
        }
        let mut library = self.inner.db.read_async(library_artists).await?;
        let lookups = futures::future::join_all(library.iter().map(|a| self.inner.artist_mbid.artist_mbid(&a.artist_id, &a.name))).await;
        for (a, m) in library.iter_mut().zip(lookups) {
            a.mbid = m;
        }
        let albums = client.all_albums().await.unwrap_or_default();
        // Editions Lidarr sends with the list are compared with the last seen.
        let editions: Vec<(i64, String, Vec<super::api::AlbumRelease>)> =
            albums.iter().filter(|a| !a.releases.is_empty()).map(|a| (a.id, a.title.clone(), a.releases.clone())).collect();
        if !editions.is_empty() {
            self.observe_releases(&editions);
        }
        let inp = ModeInputs {
            library,
            lidarr_artists: snap
                .artists
                .iter()
                .map(|a| LidarrArtistRef { id: a.id, name: a.artist_name.clone(), mbid: a.foreign_artist_id.clone(), path: a.path.clone() })
                .collect(),
            lidarr_albums: albums
                .iter()
                .map(|a| LidarrAlbumRef { artist_id: a.artist_id, release_group: a.foreign_album_id.clone(), title: a.title.clone() })
                .collect(),
            exclusions: snap.exclusions.iter().map(|e| e.foreign_id.clone()).collect(),
            prefs: self.artist_prefs(),
        };
        let out = assign(&inp);
        self.publish_modes(&out);
        *self.modes_cache.lock() = Some((now, out.clone()));
        Ok(out)
    }
}

/// What switching an artist to Managed needs: the user said yes twice.
pub const MANAGED_ASKS: u8 = 2;

impl super::Lidarr {
    /// Every artist's mode now (reads Lidarr and the library).
    pub async fn modes_now(&self) -> AppResult<Vec<ArtistMode>> {
        let (client, status, _) = Self::connect(self.routes()?).await?;
        let snap = Self::snapshot(&client, status).await?;
        self.modes_with(&client, &snap).await
    }

    /// Set an artist's mode. Managed hands the artist's folder to Lidarr, so
    /// it takes two yeses (`confirmations`), each after the explanation.
    pub fn set_mode(&self, key: &str, name: &str, mode: ModeSetting, confirmations: u8) -> AppResult<ArtistPrefs> {
        if mode == ModeSetting::Managed && confirmations < MANAGED_ASKS {
            return Err(crate::error::AppError::Invalid(
                "Managed lets Lidarr rename, retag and replace this artist's files. Confirm twice to switch.".into(),
            ));
        }
        let mut p = self.prefs_for(key);
        p.mode = mode;
        p.name = name.to_string();
        self.save_prefs(key, &p)?;
        self.log_change(&format!("Set {name} to {}", match mode {
            ModeSetting::Auto => "Auto",
            ModeSetting::Managed => "Managed",
            ModeSetting::Protected => "Protected",
        }), "", "", true, None)?;
        Ok(p)
    }

    /// An artist's other choices (adding new albums on their own, better copies).
    pub fn set_artist_choices(&self, key: &str, name: &str, auto_add: Option<bool>, hunt: bool, target: Target) -> AppResult<ArtistPrefs> {
        let mut p = self.prefs_for(key);
        p.auto_add = auto_add;
        p.hunt = hunt;
        p.target = target;
        p.name = name.to_string();
        self.save_prefs(key, &p)?;
        Ok(p)
    }
}
