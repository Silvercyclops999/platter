//! The setup check: a pure function over what was read from Plex and
//! Lidarr. It runs at setup and again before every request.
//!
//! Plex and Lidarr usually run in separate containers, so one folder on
//! disk can be `/data/media/music` to Plex and `/music` to Lidarr. Path
//! strings alone can't show two folders are different, so the check
//! compares contents: the folder names under each Lidarr root folder
//! against the folders Plex's music files sit in, confirmed with Lidarr's
//! own listing of those folders (file names and sizes against Plex's).
//! A Lidarr folder only counts as separate when its contents prove it, or
//! when it's empty and the user has said it's a different folder.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use super::api::{
    Route, Artist, Exclusion, ImportList, Listing, MediaManagementConfig, MetadataProviderConfig, NamingConfig, QualityProfile, RootFolder,
    TrackFile,
};
use super::paths::{NormPath, Relation, fold, inside_any, join_dir, relation, to_server};

/// Matching folder names checked through Lidarr's listing per root folder.
pub const MAX_PROBES: usize = 6;
/// How deep a probe follows matching subfolders looking for files.
const MAX_DEPTH: usize = 3;

/// One Plex music file: its path on the server and size in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexFile {
    pub path: String,
    pub size: Option<i64>,
}

/// What Plex says about its music: the sections' folders and every file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlexFiles {
    pub locations: Vec<String>,
    pub files: Vec<PlexFile>,
}

#[derive(Debug, Default)]
struct DirInfo {
    dirs: BTreeSet<String>,
    /// Folded file name â†’ size.
    files: HashMap<String, Option<i64>>,
}

/// Plex's folders, indexed for the comparisons.
#[derive(Debug, Default)]
pub struct PlexIndex {
    locations: Vec<(String, NormPath)>,
    dirs: HashMap<NormPath, DirInfo>,
    /// Folder name â†’ the folders holding a folder of that name.
    parents_of: HashMap<String, Vec<NormPath>>,
    /// Last two names of every folder â†’ those folders.
    by_last_two: HashMap<(String, String), Vec<NormPath>>,
    files: usize,
}

impl PlexIndex {
    pub fn new(p: &PlexFiles) -> Self {
        let mut ix = PlexIndex {
            locations: p.locations.iter().map(|l| (l.clone(), NormPath::parse(l))).filter(|(_, n)| !n.is_empty()).collect(),
            ..Default::default()
        };
        for f in &p.files {
            let n = NormPath::parse(&f.path);
            let Some(parent) = n.parent() else { continue };
            ix.files += 1;
            ix.dirs.entry(parent.clone()).or_default().files.insert(n.name().unwrap_or_default().to_string(), f.size);
            ix.add_chain(&parent);
        }
        for (_, l) in ix.locations.clone() {
            ix.add_chain(&l);
        }
        for d in ix.dirs.keys() {
            if let (Some(name), Some(parent)) = (d.name(), d.parent()) {
                ix.parents_of.entry(name.to_string()).or_default().push(parent);
            }
            if let Some(pair) = d.last_two() {
                ix.by_last_two.entry(pair).or_default().push(d.clone());
            }
        }
        for v in ix.parents_of.values_mut().chain(ix.by_last_two.values_mut()) {
            v.sort();
            v.dedup();
        }
        ix
    }

    /// Records every folder on the way down to `dir` as holding the next.
    fn add_chain(&mut self, dir: &NormPath) {
        self.dirs.entry(dir.clone()).or_default();
        for i in 1..dir.len() {
            let up = NormPath(dir.0[..i].to_vec());
            self.dirs.entry(up).or_default().dirs.insert(dir.0[i].clone());
        }
    }

    pub fn file_count(&self) -> usize {
        self.files
    }
}

/// Everything read from Lidarr for one check. `listings` maps a folder
/// path (as asked) to its listing, or None when Lidarr couldn't list it.
#[derive(Debug, Clone, Default)]
pub struct LidarrSnapshot {
    pub version: String,
    pub roots: Vec<RootFolder>,
    pub artists: Vec<Artist>,
    pub naming: NamingConfig,
    pub metadata: MetadataProviderConfig,
    pub media: MediaManagementConfig,
    pub profiles: Vec<QualityProfile>,
    pub lists: Vec<ImportList>,
    pub listings: HashMap<String, Option<Listing>>,
    /// Artists Lidarr's import lists exclude (Platter never requests or adds them).
    pub exclusions: Vec<Exclusion>,
    /// Files in Lidarr's root folders it couldn't match to an album.
    pub unmapped: Vec<TrackFile>,
    /// Lidarr's Connect entries.
    pub notifications: Vec<super::api::Notification>,
    /// Managed albums whose folders hold files Lidarr couldn't match.
    pub album_folders: Vec<AlbumFolders>,
}

/// A Managed album in Lidarr: where its files are, and whether Lidarr may
/// switch its edition on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumFolders {
    pub album_id: i64,
    pub title: String,
    /// The artist's folder in Lidarr.
    pub artist_path: String,
    /// "Automatically Switch Release" is on.
    pub any_release_ok: bool,
    /// The folders its imported files are in.
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Red,
    Amber,
    Ok,
}

/// An empty Lidarr root folder the user can vouch for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyRoot {
    pub path: String,
    pub confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub level: Level,
    pub title: String,
    pub detail: String,
    /// What to change in Lidarr.
    pub fix: Option<String>,
    /// Set on an empty root folder's finding.
    pub empty_root: Option<EmptyRoot>,
    /// A red finding that only stops work for protected artists (Managed
    /// artists keep working).
    pub protected_only: bool,
    /// A fix Platter can make (it shows the exact change and asks first).
    pub action: Option<FixAction>,
    /// A Lidarr root folder that no Plex library scans, offered as Platter's
    /// staging folder (a Platter setting, not a change in Lidarr).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_staging: Option<String>,
}

impl Finding {
    pub fn new(id: &str, level: Level, title: impl Into<String>, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self { id: id.into(), level, title: title.into(), detail: detail.into(), fix, empty_root: None, protected_only: false, action: None, suggested_staging: None }
    }

    fn protected_only(mut self) -> Self {
        self.protected_only = true;
        self
    }
}

/// An artist to put on Lidarr's import list exclusions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExclusionTarget {
    pub name: String,
    pub mbid: String,
}

/// A one-click fix: each is a write to Lidarr that shows the exact before
/// and after and waits for a yes (`changes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FixAction {
    AddExclusions { artists: Vec<ExclusionTarget> },
    /// Needs a folder from the user.
    SetRecycleBin,
    WriteTagsOff,
    RenameOff,
    AddStagingRoot { path: String },
    /// Re-match unmatched files, one album folder at a time.
    Rematch { folders: Vec<String> },
    /// Turn off "Automatically Switch Release" for these albums.
    ReleaseSwitchOff { albums: Vec<AlbumTarget> },
}

/// A Lidarr album a fix is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumTarget {
    pub id: i64,
    pub title: String,
}

/// A protected artist, as the check needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtectedRef {
    pub name: String,
    pub mbid: Option<String>,
    /// The library folder (server path).
    pub folder: Option<String>,
    pub lidarr_id: Option<i64>,
    /// Lidarr's folder for it, when Lidarr has the artist.
    pub lidarr_path: Option<String>,
}

impl From<&super::modes::ArtistMode> for ProtectedRef {
    fn from(m: &super::modes::ArtistMode) -> Self {
        Self { name: m.name.clone(), mbid: m.mbid.clone(), folder: m.folder.clone(), lidarr_id: m.lidarr_id, lidarr_path: m.lidarr_path.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub version: String,
    /// Red first, then amber, then OK.
    pub findings: Vec<Finding>,
    /// No red findings: work for protected artists may go ahead.
    pub passed: bool,
    /// No red finding that stops Managed artists.
    pub managed_ok: bool,
    /// Protected artists the check was run for.
    pub protected_count: usize,
    pub checked_at: i64,
    /// Which address answered.
    pub route: Option<Route>,
}

/// Inputs besides the two snapshots.
#[derive(Debug, Clone, Default)]
pub struct CheckContext {
    /// Root folders the user said are separate (folded paths).
    pub confirmed_roots: Vec<String>,
    /// Addresses set in Settings that are plain http to a host outside the
    /// home network.
    pub unencrypted: Vec<String>,
    pub route: Option<Route>,
    pub now: i64,
    /// The staging folder for upgrades (Lidarr's path), when set.
    pub staging: Option<String>,
    /// The library has files a tag marks unreleased.
    pub library_has_unreleased: bool,
    /// Protected artists (empty: calm checks for a normal library).
    pub protected: Vec<ProtectedRef>,
    /// The music share mapping: (server, this computer) pairs, so a path
    /// typed as this computer reaches it is compared as the server's.
    pub share: Vec<(String, String)>,
    /// Plex scans for changes on its own (None: not known).
    pub plex_auto_scan: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    /// The same files with the same sizes.
    Confirmed,
    /// Folder names match but the files couldn't be compared.
    Likely,
    /// Lidarr's listing shows different contents.
    Disproved,
    /// No listing yet, or Lidarr couldn't list the folder.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RootVerdict {
    /// The path strings show it's Plex's folder, inside it, or holding it.
    ByPath(Relation, String),
    /// Same contents as a Plex folder.
    Overlap { relation: Option<(Relation, String)>, confirmed: bool, names: Vec<String> },
    /// Its contents prove it's another folder.
    Separate,
    Empty { confirmed: bool },
    Unreadable,
}

struct Eval<'a> {
    plex: &'a PlexIndex,
    lidarr: &'a LidarrSnapshot,
    wanted: Vec<String>,
}

impl Eval<'_> {
    fn listing(&mut self, path: &str) -> Option<Option<&Listing>> {
        match self.lidarr.listings.get(path) {
            Some(l) => Some(l.as_ref()),
            None => {
                if !self.wanted.iter().any(|w| w == path) {
                    self.wanted.push(path.to_string());
                }
                None
            }
        }
    }

    /// Compare Lidarr's folder `path` with Plex's folder `dir`.
    fn compare(&mut self, path: &str, dir: &NormPath, depth: usize) -> Evidence {
        let plex = self.plex;
        let Some(info) = plex.dirs.get(dir) else { return Evidence::Unknown };
        let (files, dirs): (Vec<(String, Option<i64>)>, Vec<String>) = match self.listing(path) {
            None | Some(None) => return Evidence::Unknown,
            Some(Some(l)) => (
                l.files.iter().map(|f| (fold(&f.name), f.size)).collect(),
                l.directories.iter().map(|d| d.name.clone()).collect(),
            ),
        };
        let same_file = files.iter().any(|(name, size)| match (info.files.get(name), size) {
            (Some(Some(a)), Some(b)) => a == b,
            (Some(_), _) => true,
            (None, _) => false,
        });
        if same_file {
            return Evidence::Confirmed;
        }
        let mut common: Vec<&String> = dirs.iter().filter(|d| info.dirs.contains(&fold(d))).collect();
        common.sort();
        let Some(first) = common.first() else { return Evidence::Disproved };
        if depth >= MAX_DEPTH {
            return Evidence::Likely;
        }
        match self.compare(&join_dir(path, first), &dir.child(first), depth + 1) {
            Evidence::Confirmed => Evidence::Confirmed,
            Evidence::Disproved => Evidence::Disproved,
            Evidence::Likely | Evidence::Unknown => Evidence::Likely,
        }
    }

    fn root(&mut self, r: &RootFolder, ctx: &CheckContext) -> RootVerdict {
        let (plex, lidarr) = (self.plex, self.lidarr);
        let rp = NormPath::parse(&r.path);
        for (raw, l) in &plex.locations {
            if let Some(rel) = relation(&rp, l) {
                return RootVerdict::ByPath(rel, raw.clone());
            }
        }
        if !r.accessible {
            return RootVerdict::Unreadable;
        }
        // The folders under the root: unmapped ones and Lidarr's artists.
        let mut children: BTreeSet<String> = BTreeSet::new();
        for u in &r.unmapped_folders {
            let n = if u.name.trim().is_empty() { NormPath::parse(&u.path).name().unwrap_or_default().to_string() } else { fold(&u.name) };
            if !n.is_empty() {
                children.insert(n);
            }
        }
        for a in &lidarr.artists {
            let ap = NormPath::parse(&a.path);
            if let (true, Some(n)) = (ap.parent().as_ref() == Some(&rp), ap.name()) {
                children.insert(n.to_string());
            }
        }
        if children.is_empty() {
            return RootVerdict::Empty { confirmed: ctx.confirmed_roots.contains(&rp.key()) };
        }
        // The Plex folder holding the most of the same names.
        let mut counts: HashMap<&NormPath, usize> = HashMap::new();
        for c in &children {
            for p in plex.parents_of.get(c).into_iter().flatten() {
                *counts.entry(p).or_default() += 1;
            }
        }
        let Some(best) = counts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.len().cmp(&a.0.len())).then(b.0.cmp(a.0))).map(|(p, _)| (*p).clone())
        else {
            return RootVerdict::Separate;
        };
        let info_dirs = plex.dirs.get(&best).map(|d| d.dirs.clone()).unwrap_or_default();
        let names: Vec<String> = children.iter().filter(|c| info_dirs.contains(*c)).cloned().collect();
        // Lidarr's own names (not folded) for listing paths.
        let mut originals: HashMap<String, String> = HashMap::new();
        for u in &r.unmapped_folders {
            originals.entry(fold(&u.name)).or_insert_with(|| u.name.clone());
        }
        for a in &lidarr.artists {
            let ap = NormPath::parse(&a.path);
            if ap.parent().as_ref() == Some(&rp) {
                let raw = a.path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default().to_string();
                originals.entry(fold(&raw)).or_insert(raw);
            }
        }
        let mut evidence = Vec::new();
        for n in names.iter().take(MAX_PROBES) {
            let raw = originals.get(n).cloned().unwrap_or_else(|| n.clone());
            evidence.push(self.compare(&join_dir(&r.path, &raw), &best.child(n), 0));
        }
        // Every probed name has different contents (up to MAX_PROBES of
        // them; the same folder would match on the first).
        if !evidence.is_empty() && evidence.iter().all(|e| *e == Evidence::Disproved) {
            return RootVerdict::Separate;
        }
        let relation = plex.locations.iter().find_map(|(raw, l)| relation(&best, l).map(|rel| (rel, raw.clone())));
        RootVerdict::Overlap {
            relation,
            confirmed: evidence.contains(&Evidence::Confirmed),
            names: names.iter().map(|n| originals.get(n).cloned().unwrap_or_else(|| n.clone())).collect(),
        }
    }
}

fn root_name(r: &RootFolder) -> String {
    r.path.trim().to_string()
}

const STAGING_FIX: &str = "Use a staging folder well away from your music, on a separate path (for example a downloads folder elsewhere on the server), that no Plex library includes. Add it in Lidarr as a root folder, then set it in Platter's settings.";

/// The staging root's own red finding, when Plex can see it.
fn staging_scanned(path: &str, v: &RootVerdict) -> Option<Finding> {
    let (how, plex_path) = match v {
        RootVerdict::ByPath(Relation::Contains, p) => ("holds your Plex music folder", Some(p.clone())),
        RootVerdict::ByPath(_, p) => ("is scanned by Plex", Some(p.clone())),
        // Only proof counts (the same files with the same sizes): staging
        // folders hold the same artist names as the library by design.
        RootVerdict::Overlap { relation: Some((Relation::Contains, p)), confirmed: true, .. } => ("holds your Plex music folder", Some(p.clone())),
        RootVerdict::Overlap { relation, confirmed: true, .. } => ("is scanned by Plex", relation.as_ref().map(|(_, p)| p.clone())),
        _ => return None,
    };
    let where_ = plex_path.map(|p| format!(" (Plex reads {p})")).unwrap_or_default();
    Some(Finding::new(
        "stagingScanned",
        Level::Red,
        format!("The staging folder {path} {how}"),
        format!("Better copies wait in the staging folder until you swap them in. If Plex reads it{where_}, half-finished downloads show up in your library next to the albums they're meant to replace."),
        Some(STAGING_FIX.into()),
    ))
}

fn list_names(names: &[String], max: usize) -> String {
    let shown: Vec<&str> = names.iter().take(max).map(String::as_str).collect();
    let more = names.len().saturating_sub(max);
    if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

/// The check over both snapshots. Also returns Lidarr folders whose
/// listing would settle a comparison and hasn't been read yet.
pub fn evaluate(plex: &PlexIndex, lidarr: &LidarrSnapshot, ctx: &CheckContext) -> (CheckReport, Vec<String>) {
    let mut ev = Eval { plex, lidarr, wanted: Vec::new() };
    let mut red = Vec::new();
    let mut amber = Vec::new();
    let mut ok = Vec::new();

    // ---- Lidarr's root folders against Plex's music folders.
    let plex_known = plex.file_count() > 0 && !plex.locations.is_empty();
    let mut verdicts: Vec<(&RootFolder, RootVerdict)> = Vec::new();
    if !plex_known {
        red.push(Finding::new(
            "plexUnknown",
            Level::Red,
            "Platter couldn't see your Plex music files",
            "Without the folders and files Plex has, Platter can't prove Lidarr's folders are separate from your music.",
            Some("Make sure Platter is connected to the Plex server that has your music, and that its music library has finished scanning. Then check again.".into()),
        ).protected_only());
    }
    if lidarr.roots.is_empty() {
        red.push(Finding::new(
            "noRoots",
            Level::Red,
            "Lidarr has no root folder",
            "Lidarr needs a folder to put downloads in before anything can be requested.",
            Some("In Lidarr, open Settings > Media Management > Root Folders and add a separate, empty folder that isn't inside your Plex music folder.".into()),
        ));
    }
    for r in &lidarr.roots {
        let v = if plex_known { ev.root(r, ctx) } else { RootVerdict::Unreadable };
        verdicts.push((r, v));
    }
    let protected = !ctx.protected.is_empty();
    let plex_locs: Vec<&str> = plex.locations.iter().map(|(raw, _)| raw.as_str()).collect();
    let root_paths: Vec<&str> = lidarr.roots.iter().map(|r| r.path.as_str()).collect();
    let staging = ctx.staging.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| to_server(s, &ctx.share));
    let staging_norm = staging.as_deref().map(NormPath::parse);
    let is_staging = |r: &RootFolder| staging_norm.as_ref().is_some_and(|s| !s.is_empty() && NormPath::parse(&r.path) == *s);

    // ---- Roots Plex and Lidarr name differently: the path tests can't be
    // trusted for protected artists.
    let mut separate = Vec::new();
    for (r, v) in &verdicts {
        let path = root_name(r);
        if is_staging(r)
            && let Some(f) = staging_scanned(&path, v)
        {
            red.push(f.protected_only());
            continue;
        }
        match v {
            RootVerdict::Overlap { confirmed: true, names, .. } if protected => {
                let proof = "Lidarr sees the same files, with the same sizes, as Plex does";
                red.push(
                    Finding::new(
                        "pathsDiffer",
                        Level::Red,
                        format!("Lidarr and Plex name the folder {path} differently"),
                        format!(
                            "{proof} ({}), but under another path. Platter decides where protected artists are by their paths, so it can't keep them safe until both name the folder the same way.",
                            list_names(names, 3)
                        ),
                        Some("Give Lidarr and Plex the same path for your music (in Docker, mount it at the same place in both containers), then check again. Managed artists keep working meanwhile.".into()),
                    )
                    .protected_only(),
                );
            }
            RootVerdict::Unreadable if plex_known => {
                ok.push(Finding::new(
                    "rootUnreadable",
                    Level::Ok,
                    format!("Lidarr can't open its folder {path}"),
                    "Lidarr shows a warning for it in Settings > Media Management > Root Folders.",
                    None,
                ));
            }
            RootVerdict::Separate => separate.push(path),
            _ => {}
        }
    }
    if !separate.is_empty() {
        ok.push(Finding::new(
            "rootSeparate",
            Level::Ok,
            "Some of Lidarr's folders are outside your Plex music",
            format!("{} {} nothing Plex has.", list_names(&separate, 3), if separate.len() == 1 { "holds" } else { "hold" }),
            None,
        ));
    }

    // ---- Protected artists: never in Lidarr with a library path, never
    // under a Lidarr root while Lidarr has them, never retagged.
    let mut in_plex: Vec<String> = Vec::new();
    let mut under_root: Vec<String> = Vec::new();
    let mut any_under_root: Vec<String> = Vec::new();
    for p in &ctx.protected {
        let lidarr_path = p.lidarr_path.as_deref().filter(|x| !x.trim().is_empty());
        if let Some(lp) = lidarr_path
            && let Some(loc) = inside_any(lp, plex_locs.iter().copied())
        {
            in_plex.push(format!("{} ({lp}, inside {loc})", p.name));
        }
        if let Some(f) = p.folder.as_deref()
            && let Some(root) = inside_any(&to_server(f, &ctx.share), root_paths.iter().copied())
        {
            any_under_root.push(p.name.clone());
            if lidarr_path.is_some() {
                under_root.push(format!("{} ({f}, under {root})", p.name));
            }
        }
    }
    if !in_plex.is_empty() {
        red.push(
            Finding::new(
                "protectedInPlex",
                Level::Red,
                if in_plex.len() == 1 { "Lidarr has a protected artist in your Plex music".to_string() } else { format!("Lidarr has {} protected artists in your Plex music", in_plex.len()) },
                format!(
                    "{}. Lidarr imports, renames and upgrades in that folder, so curated files could change. Protected artists only ever live in Lidarr's staging folder.",
                    list_names(&in_plex, 3)
                ),
                Some("In Lidarr, edit each artist and set its path to a folder inside the staging root, or remove the artist (Artist > Delete, leave the files). Platter never moves or removes artists itself.".into()),
            )
            .protected_only(),
        );
    }
    if !under_root.is_empty() {
        red.push(
            Finding::new(
                "protectedUnderRoot",
                Level::Red,
                if under_root.len() == 1 { "A protected artist's folder is inside a Lidarr root folder".to_string() } else { format!("{} protected artists' folders are inside Lidarr root folders", under_root.len()) },
                format!(
                    "{}. Lidarr has these artists, so a rescan could match, rename or replace their curated files.",
                    list_names(&under_root, 3)
                ),
                Some("Move the folder outside every Lidarr root folder, into a folder your Plex music library still scans (for example next to the Lidarr folder, not inside it). Platter never moves it for you.".into()),
            )
            .protected_only(),
        );
    }
    let tags = lidarr.metadata.write_audio_tags.trim().to_ascii_lowercase();
    let writes_tags = !tags.is_empty() && tags != "no";
    if writes_tags && !any_under_root.is_empty() {
        let mut f = Finding::new(
            "writeAudioTags",
            Level::Red,
            "Lidarr writes tags while protected artists are under its root folders",
            format!(
                "With Write Audio Tags on, Lidarr changes the tags of files under its root folders, and {} {} there. Their tags carry release status and sources.",
                list_names(&any_under_root, 3),
                if any_under_root.len() == 1 { "is" } else { "are" }
            ),
            Some("In Lidarr, set Settings > Metadata > Write Audio Tags to Never.".into()),
        )
        .protected_only();
        f.action = Some(FixAction::WriteTagsOff);
        red.push(f);
    } else if writes_tags {
        let mut f = Finding::new(
            "writeAudioTags",
            Level::Ok,
            "Lidarr writes tags into the files it manages",
            "That's fine for Managed artists. Platter checks again before any protected work.",
            None,
        );
        f.action = Some(FixAction::WriteTagsOff);
        ok.push(f);
    } else {
        ok.push(Finding::new("writeAudioTags", Level::Ok, "Lidarr doesn't write tags into files", "", None));
    }

    // ---- The staging folder.
    if let Some(st) = staging.as_deref() {
        if let Some(loc) = inside_any(st, plex_locs.iter().copied())
            && !red.iter().any(|f| f.id == "stagingScanned")
        {
            red.push(
                Finding::new(
                    "stagingScanned",
                    Level::Red,
                    format!("The staging folder {st} is inside your Plex music folder {loc}"),
                    "Downloads for protected artists wait in the staging folder. If Plex scans it, they show up in your library before you've added them.",
                    Some(STAGING_FIX.into()),
                )
                .protected_only(),
            );
        }
        if !lidarr.roots.iter().any(is_staging) {
            let mut f = Finding::new(
                "stagingNotRoot",
                Level::Red,
                format!("The staging folder {st} isn't one of Lidarr's root folders"),
                "Downloads for protected artists land in the staging folder, so Lidarr has to have it as a root folder.",
                Some("Add it in Lidarr's Settings > Media Management > Root Folders, or let Platter add it.".into()),
            )
            .protected_only();
            f.action = Some(FixAction::AddStagingRoot { path: st.to_string() });
            red.push(f);
        } else {
            ok.push(Finding::new("staging", Level::Ok, format!("Staging folder: {st}"), "Outside every Plex music folder.", None));
        }
    } else if protected {
        // A Lidarr root that no Plex library scans (and that holds no Plex
        // folder) is almost certainly the staging folder: offer it.
        let candidates: Vec<&str> = lidarr
            .roots
            .iter()
            .map(|r| r.path.as_str())
            .filter(|p| inside_any(p, plex_locs.iter().copied()).is_none())
            .filter(|p| !plex_locs.iter().any(|l| inside_any(l, [*p]).is_some()))
            .collect();
        let mut f = Finding::new(
            "stagingMissing",
            Level::Red,
            "No staging folder for protected artists",
            "Lidarr only ever downloads protected artists into a staging folder no Plex library scans. Until one is set, Platter won't request anything for them.",
            Some(STAGING_FIX.into()),
        )
        .protected_only();
        if let [only] = candidates.as_slice() {
            f.fix = Some(format!("{only} is a Lidarr root folder that no Plex library scans. Use it as the staging folder?"));
            f.suggested_staging = Some((*only).to_string());
        }
        red.push(f);
    }

    // ---- Exclusions: protected artists stay out of Lidarr's import lists
    // even while Platter is closed.
    let excluded: HashSet<String> = lidarr.exclusions.iter().map(|e| e.foreign_id.trim().to_ascii_lowercase()).collect();
    let not_excluded: Vec<(String, String)> = ctx
        .protected
        .iter()
        .filter_map(|p| p.mbid.as_ref().map(|m| (p.name.clone(), m.to_ascii_lowercase())))
        .filter(|(_, m)| !excluded.contains(m))
        .collect();
    if !not_excluded.is_empty() {
        let names: Vec<String> = not_excluded.iter().map(|(n, _)| n.clone()).collect();
        let mut f = Finding::new(
            "protectedNotExcluded",
            Level::Amber,
            if names.len() == 1 { "A protected artist isn't on Lidarr's import list exclusions".to_string() } else { format!("{} protected artists aren't on Lidarr's import list exclusions", names.len()) },
            format!("{}. On the exclusion list, Lidarr's import lists never add them, even while Platter is closed.", list_names(&names, 3)),
            Some("Platter can add them for you (it shows each change first), or add them in Lidarr's Settings > Import Lists > Import List Exclusions.".into()),
        );
        f.action = Some(FixAction::AddExclusions { artists: not_excluded.into_iter().map(|(name, mbid)| ExclusionTarget { name, mbid }).collect() });
        amber.push(f);
    }

    // ---- Settings that matter for protected artists, calm otherwise.
    let level = |unsafe_: bool| if unsafe_ && protected { Level::Amber } else { Level::Ok };
    let no_bin = lidarr.media.recycle_bin.trim().is_empty();
    {
        let mut f = Finding::new(
            "recycleBin",
            level(no_bin),
            if no_bin { "Lidarr deletes files for good" } else { "Lidarr keeps removed files in a recycling bin" },
            if no_bin {
                "Freeing up staging space and replaced files can't be brought back without a recycling bin."
            } else {
                "Files freed from staging go there first; the space comes back after Lidarr's recycling bin cleanup."
            },
            no_bin.then(|| "Set a folder in Lidarr's Settings > Media Management > Recycling Bin, or let Platter set it.".to_string()),
        );
        if no_bin {
            f.action = Some(FixAction::SetRecycleBin);
        }
        push(&mut amber, &mut ok, f);
    }
    {
        // Renaming only touches files under Lidarr's root folders, so it
        // matters only when a protected artist's folder is under one.
        let on = lidarr.naming.rename_tracks;
        let risky = on && !any_under_root.is_empty();
        let mut f = Finding::new(
            "renameTracks",
            if risky { Level::Amber } else { Level::Ok },
            if on { "Lidarr renames the files it imports" } else { "Track renaming is off" },
            if risky {
                format!(
                    "{} {} under a Lidarr root folder, so Lidarr could rename curated files there. Platter also doesn't re-match unmatched files while renaming is on.",
                    list_names(&any_under_root, 3),
                    if any_under_root.len() == 1 { "is" } else { "are" }
                )
            } else if on {
                "That's fine: no protected artist's folder is under a Lidarr root folder, so only Managed albums get Lidarr's names. Platter doesn't re-match unmatched files while renaming is on.".to_string()
            } else {
                String::new()
            },
            risky.then(|| "In Lidarr, turn off Settings > Media Management > Rename Tracks, or let Platter do it.".to_string()),
        );
        if risky {
            f.action = Some(FixAction::RenameOff);
        }
        push(&mut amber, &mut ok, f);
    }
    push(
        &mut amber,
        &mut ok,
        Finding::new(
            "scrubAudioTags",
            level(lidarr.metadata.scrub_audio_tags && writes_tags),
            if lidarr.metadata.scrub_audio_tags { "Lidarr removes tags it doesn't know" } else { "Lidarr keeps tags it doesn't know" },
            "",
            lidarr.metadata.scrub_audio_tags.then(|| "In Lidarr, turn off Settings > Metadata > Scrub Existing Tags.".to_string()),
        ),
    );
    if lidarr.media.delete_empty_folders {
        ok.push(Finding::new("deleteEmptyFolders", Level::Ok, "Lidarr deletes empty folders", "Folders Lidarr leaves empty after moving files are removed.", None));
    }
    let monitor_all: Vec<String> = lidarr
        .roots
        .iter()
        .filter(|r| {
            [&r.default_monitor_option, &r.default_new_item_monitor_option].iter().any(|o| o.as_deref().is_some_and(|v| v.trim().eq_ignore_ascii_case("all")))
        })
        .map(root_name)
        .collect();
    if !monitor_all.is_empty() {
        amber.push(Finding::new(
            "rootMonitorAll",
            Level::Amber,
            "Lidarr monitors everything by default",
            format!("New artists in {} start with all their albums, and new ones, marked for download.", list_names(&monitor_all, 3)),
            Some("In Lidarr, edit the root folder (Settings > Media Management > Root Folders) and set its default monitoring to None.".into()),
        ));
    }
    let lists: Vec<String> = lidarr.lists.iter().filter(|l| l.adds_artists()).map(|l| l.name.clone()).collect();
    if !lists.is_empty() {
        amber.push(Finding::new(
            "importLists",
            Level::Amber,
            "Import lists add artists on their own",
            format!("{} can add artists and albums to Lidarr without you asking. Protected artists on the exclusion list are never added.", list_names(&lists, 3)),
            Some("In Lidarr, open Settings > Import Lists and turn off automatic adding, or remove the list.".into()),
        ));
    }
    if !ctx.unencrypted.is_empty() {
        amber.push(Finding::new(
            "plainHttp",
            Level::Amber,
            "Your API key would travel unencrypted",
            format!(
                "{} starts with http:// and isn't on your home network, so the key and everything Lidarr answers cross the internet in plain text.",
                list_names(&ctx.unencrypted, 2)
            ),
            Some("Reach Lidarr from away through https (your reverse proxy or tunnel can add it), then use the https address here.".into()),
        ));
    }

    // ---- Managed artists: Lidarr's profiles upgrade them in place.
    for p in managed_profiles(lidarr, ctx) {
        let reach = super::quality::cutoff_reachable(p);
        let (lvl, title, detail) = match (p.upgrade_allowed, p.cutoff.is_some(), reach) {
            (false, _, _) => (Level::Ok, format!("{} doesn't look for better copies", p.name), "Upgrades Allowed is off in this profile, so Lidarr keeps the first copy it finds.".to_string()),
            (true, false, _) | (true, true, false) => (
                Level::Amber,
                format!("{} can't reach its cutoff", p.name),
                "Upgrades are on, but the cutoff quality isn't one the profile allows, so Lidarr may keep downloading the same album.".to_string(),
            ),
            (true, true, true) => (Level::Ok, format!("{} upgrades Managed artists in place", p.name), "Lidarr looks for better copies until the cutoff is met.".to_string()),
        };
        push(&mut amber, &mut ok, Finding::new("managedProfile", lvl, title, detail, (lvl == Level::Amber).then(|| "In Lidarr, open Settings > Profiles and set the cutoff to a quality the profile allows.".to_string())));
    }

    // ---- Plex picks up Managed imports on its own while Platter is closed.
    let notifies_plex = lidarr.notifications.iter().any(|n| n.implementation.eq_ignore_ascii_case("PlexServer"));
    match ctx.plex_auto_scan {
        Some(true) => ok.push(Finding::new("plexAutoScan", Level::Ok, "Plex scans for changes on its own", "Managed albums appear in Plex even while Platter is closed.", None)),
        Some(false) if !notifies_plex => amber.push(Finding::new(
            "plexAutoScan",
            Level::Amber,
            "Plex doesn't scan for changes on its own",
            "Lidarr's imports only reach Plex while Platter is open (it asks Plex to scan just the new folder). Turn on automatic scanning so they show up anyway.",
            Some("In Plex, open Settings > Library and turn on \"Scan my library automatically\" (or \"Scan my library periodically\").".into()),
        )),
        _ => {}
    }
    if notifies_plex {
        ok.push(Finding::new("plexConnect", Level::Ok, "Lidarr tells Plex about imports", "Lidarr's Plex Media Server connection refreshes Plex after each import.", None));
    } else {
        ok.push(Finding::new(
            "plexConnect",
            Level::Ok,
            "Platter asks Plex to scan new albums",
            "While Platter is open it scans just the new folder. Lidarr's own Plex connection is an option (Settings > Connect), but it needs a Plex token inside Lidarr.",
            None,
        ));
    }

    // ---- Files Lidarr couldn't match under Managed artists: re-matched one
    // album at a time, only with renaming off.
    if !lidarr.unmapped.is_empty() {
        let managed: Vec<&str> = lidarr
            .artists
            .iter()
            .filter(|a| !ctx.protected.iter().any(|p| p.lidarr_id == Some(a.id)))
            .map(|a| a.path.as_str())
            .filter(|p| !p.trim().is_empty())
            .collect();
        let mut folders: BTreeSet<String> = BTreeSet::new();
        for f in &lidarr.unmapped {
            if inside_any(&f.path, managed.iter().copied()).is_some()
                && let Some(d) = super::modes::parent_raw(&f.path)
            {
                folders.insert(d);
            }
        }
        if !folders.is_empty() {
            let list: Vec<String> = folders.into_iter().collect();
            let rename = lidarr.naming.rename_tracks;
            let mut f = Finding::new(
                "unmappedManaged",
                Level::Ok,
                format!("Lidarr couldn't match files in {}", if list.len() == 1 { "1 album folder".to_string() } else { format!("{} album folders", list.len()) }),
                if rename {
                    format!("{}. Re-matching them would let Lidarr rename them too while Rename Tracks is on, so Platter doesn't offer it.", list_names(&list, 2))
                } else {
                    format!("{}. Platter can re-match them one album at a time; you confirm each, and it's logged.", list_names(&list, 2))
                },
                None,
            );
            if !rename {
                f.action = Some(FixAction::Rematch { folders: list });
            }
            ok.push(f);
        }
    }

    // ---- Managed albums with files on another edition (bonus discs, extra
    // tracks Lidarr doesn't track) that Lidarr may switch on its own: a
    // switch detaches the files it does track.
    let switching = switching_albums(lidarr, ctx);
    if !switching.is_empty() {
        let names: Vec<String> = switching.iter().map(|a| a.title.clone()).collect();
        let mut f = Finding::new(
            "releaseSwitch",
            Level::Amber,
            if names.len() == 1 {
                format!("Lidarr may switch {} to another edition", names[0])
            } else {
                format!("Lidarr may switch {} albums to other editions", names.len())
            },
            format!(
                "{} {} files Lidarr doesn't track (extra discs or bonus tracks) and Automatically Switch Release on. At its daily refresh Lidarr can move {} to another edition, and the files it tracks now stop counting. Nothing changes on disk.",
                list_names(&names, 3),
                if names.len() == 1 { "has" } else { "have" },
                if names.len() == 1 { "it" } else { "them" }
            ),
            Some("Turn off Automatically Switch Release for this album (in Lidarr, Edit album), or let Platter do it. The edition it's on now stays.".into()),
        );
        f.action = Some(FixAction::ReleaseSwitchOff { albums: switching.iter().map(|a| AlbumTarget { id: a.album_id, title: a.title.clone() }).collect() });
        amber.push(f);
    }

    // ---- A root folder inside another: removing the outer one in Lidarr
    // detaches every file under the inner one.
    let mut nested: Vec<String> = Vec::new();
    for a in &lidarr.roots {
        for b in &lidarr.roots {
            let (pa, pb) = (NormPath::parse(&a.path), NormPath::parse(&b.path));
            if a.id != b.id && pa != pb && pa.starts_with(&pb) {
                let holds = lidarr.artists.iter().any(|x| NormPath::parse(&x.path).starts_with(&pa));
                nested.push(format!("{} is inside {}{}", root_name(a), root_name(b), if holds { " and holds artists" } else { "" }));
            }
        }
    }
    if !nested.is_empty() {
        amber.push(Finding::new(
            "nestedRoots",
            Level::Amber,
            "One of Lidarr's root folders is inside another",
            format!(
                "{}. Never remove the outer folder from Lidarr: Lidarr would let go of every file under the inner one and may match them to other releases when it scans again.",
                list_names(&nested, 2)
            ),
            Some("Keep both root folders, or move the inner one somewhere else first. Platter never removes root folders.".into()),
        ));
    }

    if !lidarr.exclusions.is_empty() {
        let names: Vec<String> = lidarr.exclusions.iter().map(|e| if e.artist_name.trim().is_empty() { e.foreign_id.clone() } else { e.artist_name.clone() }).collect();
        ok.push(Finding::new(
            "importExclusions",
            Level::Ok,
            "Platter respects Lidarr's import list exclusions",
            format!("It never adds {}.", list_names(&names, 3)),
            None,
        ));
    }

    let passed = red.is_empty();
    let managed_ok = !red.iter().any(|f| !f.protected_only);
    let mut findings = red;
    findings.append(&mut amber);
    findings.append(&mut ok);
    let wanted = ev.wanted;
    (
        CheckReport {
            version: lidarr.version.clone(),
            findings,
            passed,
            managed_ok,
            protected_count: ctx.protected.len(),
            checked_at: ctx.now,
            route: ctx.route,
        },
        wanted,
    )
}

/// Managed albums with unmatched files in their folders (or in a disc
/// folder next to theirs) and "Automatically Switch Release" on.
fn switching_albums<'a>(l: &'a LidarrSnapshot, ctx: &CheckContext) -> Vec<&'a AlbumFolders> {
    let protected: Vec<&str> =
        l.artists.iter().filter(|a| ctx.protected.iter().any(|p| p.lidarr_id == Some(a.id))).map(|a| a.path.as_str()).collect();
    let unmapped: Vec<NormPath> = l
        .unmapped
        .iter()
        .filter(|f| inside_any(&f.path, protected.iter().copied()).is_none())
        .filter_map(|f| NormPath::parse(&f.path).parent())
        .collect();
    let mut out: Vec<&AlbumFolders> = l
        .album_folders
        .iter()
        .filter(|a| a.any_release_ok && inside_any(&a.artist_path, protected.iter().copied()).is_none())
        .filter(|a| {
            let artist = NormPath::parse(&a.artist_path);
            let dirs: Vec<NormPath> = a.folders.iter().map(|d| NormPath::parse(d)).collect();
            // A disc folder's parent is the album's folder (never the artist's).
            let homes: Vec<NormPath> = dirs.iter().filter_map(NormPath::parent).filter(|p| *p != artist && p.starts_with(&artist)).collect();
            unmapped.iter().any(|u| dirs.contains(u) || u.parent().is_some_and(|p| homes.contains(&p)) || homes.contains(u))
        })
        .collect();
    out.sort_by(|a, b| a.title.cmp(&b.title).then(a.album_id.cmp(&b.album_id)));
    out.dedup_by_key(|a| a.album_id);
    out
}

/// Amber findings go with the amber ones, the rest with the OK ones.
fn push(amber: &mut Vec<Finding>, ok: &mut Vec<Finding>, f: Finding) {
    if f.level == Level::Amber { amber.push(f) } else { ok.push(f) }
}

/// The quality profiles Managed artists use (their own, else the roots' defaults).
fn managed_profiles<'a>(l: &'a LidarrSnapshot, ctx: &CheckContext) -> Vec<&'a QualityProfile> {
    let mut ids: BTreeSet<i64> = l
        .artists
        .iter()
        .filter(|a| !ctx.protected.iter().any(|p| p.lidarr_id == Some(a.id)))
        .filter_map(|a| a.quality_profile_id)
        .collect();
    ids.extend(l.roots.iter().filter_map(|r| r.default_quality_profile_id));
    l.profiles.iter().filter(|p| ids.contains(&p.id)).collect()
}

/// Keeps only the folders a root-folder check would still want (for tests
/// and the fetch loop's cap).
pub fn dedup_wanted(mut wanted: Vec<String>, have: &HashMap<String, Option<Listing>>) -> Vec<String> {
    let mut seen = HashSet::new();
    wanted.retain(|w| !have.contains_key(w) && seen.insert(w.clone()));
    wanted
}
