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
    Route, Artist, ImportList, Listing, MediaManagementConfig, MetadataProviderConfig, NamingConfig, QualityProfile, RootFolder,
};
use super::paths::{NormPath, Relation, as_dir, fold, join_dir, relation};

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
}

impl Finding {
    fn new(id: &str, level: Level, title: impl Into<String>, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self { id: id.into(), level, title: title.into(), detail: detail.into(), fix, empty_root: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub version: String,
    /// Red first, then amber, then OK.
    pub findings: Vec<Finding>,
    /// No red findings: requests may be sent.
    pub passed: bool,
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

const ROOT_FIX: &str = "In Lidarr, open Settings > Media Management > Root Folders and add a separate, empty folder for downloads (for example a Lidarr folder next to your music, added to the Plex music library as a second folder). Then remove this root folder from Lidarr. Platter never changes Lidarr's folders itself.";

const STAGING_FIX: &str = "Use a staging folder well away from your music, on a separate path (for example a downloads folder elsewhere on the server), that no Plex library includes. Add it in Lidarr as a root folder, then set it in Platter's settings.";

/// The staging root's own red finding, when Plex can see it.
fn staging_scanned(path: &str, v: &RootVerdict) -> Option<Finding> {
    let (how, plex_path) = match v {
        RootVerdict::ByPath(Relation::Contains, p) => ("holds your Plex music folder", Some(p.clone())),
        RootVerdict::ByPath(_, p) => ("is scanned by Plex", Some(p.clone())),
        RootVerdict::Overlap { relation: Some((Relation::Contains, p)), .. } => ("holds your Plex music folder", Some(p.clone())),
        RootVerdict::Overlap { relation, .. } => ("is scanned by Plex", relation.as_ref().map(|(_, p)| p.clone())),
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
        ));
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
    let mut separate = Vec::new();
    let staging = ctx.staging.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let staging_norm = staging.map(NormPath::parse);
    let is_staging = |r: &RootFolder| staging_norm.as_ref().is_some_and(|s| !s.is_empty() && NormPath::parse(&r.path) == *s);
    for (r, v) in &verdicts {
        let path = root_name(r);
        if is_staging(r)
            && let Some(f) = staging_scanned(&path, v)
        {
            red.push(f);
            continue;
        }
        match v {
            RootVerdict::ByPath(rel, plex_path) => {
                let how = match rel {
                    Relation::Same => "is the same folder as",
                    Relation::Inside => "is inside",
                    Relation::Contains => "holds",
                };
                red.push(Finding::new(
                    "rootOverlap",
                    Level::Red,
                    format!("Lidarr's folder {path} {how} your Plex music folder"),
                    format!("Plex keeps your music in {plex_path}. Lidarr renames, retags, upgrades and cleans up the files in its root folders, so it could change your existing library."),
                    Some(ROOT_FIX.into()),
                ));
            }
            RootVerdict::Overlap { relation, confirmed, names } => {
                let how = match relation {
                    Some((Relation::Same, _)) => "is your Plex music folder",
                    Some((Relation::Inside, _)) => "is inside your Plex music folder",
                    Some((Relation::Contains, _)) => "holds your Plex music folder",
                    None => "has the same folders as your Plex music",
                };
                let proof = if *confirmed {
                    "Lidarr sees the same files, with the same sizes, as Plex does"
                } else {
                    "The same folder names are in both"
                };
                let plex_path = relation.as_ref().map(|(_, p)| format!(" ({p})")).unwrap_or_default();
                red.push(Finding::new(
                    "rootOverlap",
                    Level::Red,
                    format!("Lidarr's folder {path} {how}"),
                    format!(
                        "{proof} ({}), so this looks like your Plex music folder{plex_path} seen from Lidarr's side under another name. Lidarr renames, retags, upgrades and cleans up the files in its root folders, so it could change your existing library.",
                        list_names(names, 3)
                    ),
                    Some(ROOT_FIX.into()),
                ));
            }
            RootVerdict::Empty { confirmed: false } => {
                let mut f = Finding::new(
                    "rootEmpty",
                    Level::Red,
                    format!("Lidarr's folder {path} is empty"),
                    "An empty folder has nothing to compare, so Platter can't tell whether it's separate from your Plex music folder. If it's a different folder (not your music folder, and not inside it), say so below. Platter checks it again before every request.",
                    None,
                );
                f.empty_root = Some(EmptyRoot { path: path.clone(), confirmed: false });
                red.push(f);
            }
            RootVerdict::Empty { confirmed: true } => {
                let mut f = Finding::new(
                    "rootEmpty",
                    Level::Ok,
                    format!("You said {path} is a separate folder"),
                    "It's empty, so Platter goes by what you said. Once Lidarr puts music in it, Platter compares its contents with your library before every request.",
                    None,
                );
                f.empty_root = Some(EmptyRoot { path: path.clone(), confirmed: true });
                ok.push(f);
            }
            RootVerdict::Unreadable if plex_known => {
                red.push(Finding::new(
                    "rootUnreadable",
                    Level::Red,
                    format!("Lidarr can't open its folder {path}"),
                    "Platter can't see what's in it, so it can't prove it's separate from your Plex music folder.",
                    Some("In Lidarr, check that the folder exists and that Lidarr can read it (Settings > Media Management > Root Folders shows a warning when it can't).".into()),
                ));
            }
            RootVerdict::Unreadable => {}
            RootVerdict::Separate => separate.push(path),
        }
    }
    if !separate.is_empty() {
        ok.push(Finding::new(
            "rootSeparate",
            Level::Ok,
            "Lidarr's folders are separate from your Plex music",
            format!("Their contents don't match your library: {}.", list_names(&separate, 3)),
            None,
        ));
    }

    // ---- The staging folder for upgrades.
    if let (Some(staging), Some(sn)) = (staging, staging_norm.as_ref()) {
        if !lidarr.roots.iter().any(is_staging) {
            red.push(Finding::new(
                "stagingNotRoot",
                Level::Red,
                format!("The staging folder {staging} isn't one of Lidarr's root folders"),
                "Better copies download into the staging folder, so Lidarr has to have it as a root folder. Until then upgrades can't be requested.",
                Some("In Lidarr, add it in Settings > Media Management > Root Folders, or change the staging folder in Platter's settings. Platter never adds root folders itself.".into()),
            ));
        }
        let beside: Vec<&String> = plex
            .locations
            .iter()
            .filter(|(_, l)| relation(sn, l).is_none() && sn.parent().is_some() && sn.parent() == l.parent())
            .map(|(raw, _)| raw)
            .collect();
        if let Some(music) = beside.first() {
            red.push(Finding::new(
                "stagingBeside",
                Level::Red,
                format!("The staging folder {staging} sits next to your Plex music folder"),
                format!(
                    "It's in the same parent folder as {music}. A folder that close to your library is easy to add to Plex, or to mix up with it, by mistake."
                ),
                Some(STAGING_FIX.into()),
            ));
        }
    }

    // ---- Lidarr artists whose folders are in the Plex music folders.
    if plex_known {
        let mut inside: Vec<String> = Vec::new();
        let roots: Vec<(NormPath, &RootVerdict)> = verdicts.iter().map(|(r, v)| (NormPath::parse(&r.path), v)).collect();
        for a in &lidarr.artists {
            let ap = NormPath::parse(&a.path);
            if ap.is_empty() {
                continue;
            }
            let by_path = plex.locations.iter().any(|(_, l)| ap.starts_with(l));
            let root = roots.iter().find(|(rp, _)| ap.starts_with(rp)).map(|(_, v)| *v);
            let hit = by_path
                || match root {
                    Some(RootVerdict::Separate) | Some(RootVerdict::Empty { .. }) => false,
                    // A root holding the Plex folder: only its artists that
                    // are in Plex's folders.
                    Some(RootVerdict::ByPath(Relation::Contains, _))
                    | Some(RootVerdict::Overlap { relation: Some((Relation::Contains, _)), .. }) => {
                        ap.last_two().is_some_and(|p| plex.by_last_two.contains_key(&p))
                    }
                    // A root that is the Plex folder or inside it: all of them.
                    Some(RootVerdict::ByPath(..)) | Some(RootVerdict::Overlap { .. }) => true,
                    Some(RootVerdict::Unreadable) | None => match ap.last_two().and_then(|p| plex.by_last_two.get(&p)) {
                        Some(dirs) => {
                            let dir = dirs[0].clone();
                            ev.compare(&as_dir(&a.path), &dir, 0) != Evidence::Disproved
                        }
                        None => false,
                    },
                };
            if hit {
                inside.push(if a.artist_name.trim().is_empty() { a.path.clone() } else { a.artist_name.clone() });
            }
        }
        if inside.is_empty() {
            ok.push(Finding::new(
                "artistsInside",
                Level::Ok,
                "No Lidarr artists in your Plex music folder",
                "Lidarr doesn't manage any of the folders your library uses.",
                None,
            ));
        } else {
            let n = inside.len();
            red.push(Finding::new(
                "artistsInside",
                Level::Red,
                if n == 1 { "Lidarr manages an artist in your Plex music folder".to_string() } else { format!("Lidarr manages {n} artists in your Plex music folder") },
                format!(
                    "{} {} folders in your library, so Lidarr may rename, retag or replace those files.",
                    list_names(&inside, 3),
                    if n == 1 { "uses one of the" } else { "use" }
                ),
                Some("In Lidarr, remove these artists (Artist > Delete) and leave the option to delete their files unticked. Platter never removes artists from Lidarr itself.".into()),
            ));
        }
    }

    // ---- Settings that only affect files Lidarr downloads itself.
    let mut setting = |on: bool, id: &str, bad: &str, why: &str, fix: &str, good: &str| {
        if on {
            amber.push(Finding::new(id, Level::Amber, bad, why, Some(fix.into())));
        } else {
            ok.push(Finding::new(id, Level::Ok, good, "", None));
        }
    };
    setting(
        lidarr.naming.rename_tracks,
        "renameTracks",
        "Lidarr renames the files it downloads",
        "Downloads get Lidarr's own file names instead of keeping theirs.",
        "In Lidarr, turn off Settings > Media Management > Rename Tracks if you'd rather keep the original names.",
        "Track renaming is off",
    );
    let tags = lidarr.metadata.write_audio_tags.trim().to_ascii_lowercase();
    if tags == "allfiles" || tags == "sync" {
        setting(
            true,
            "writeAudioTags",
            "Lidarr rewrites tags in files it already has",
            "With this setting Lidarr goes back and retags files it manages, not just new downloads.",
            "In Lidarr, set Settings > Metadata > Write Audio Tags to Never.",
            "",
        );
    } else {
        setting(
            !tags.is_empty() && tags != "no",
            "writeAudioTags",
            "Lidarr writes its own tags into the files it downloads",
            "Downloads get Lidarr's tags instead of keeping their own.",
            "In Lidarr, set Settings > Metadata > Write Audio Tags to Never.",
            "Lidarr doesn't write tags into files",
        );
    }
    setting(
        lidarr.metadata.scrub_audio_tags,
        "scrubAudioTags",
        "Lidarr removes tags it doesn't know",
        "Tags Lidarr doesn't write itself are deleted from files it tags.",
        "In Lidarr, turn off Settings > Metadata > Scrub Existing Tags.",
        "Lidarr keeps tags it doesn't know",
    );
    setting(
        lidarr.media.delete_empty_folders,
        "deleteEmptyFolders",
        "Lidarr deletes empty folders",
        "Folders Lidarr leaves empty after moving files are removed.",
        "In Lidarr, turn off Settings > Media Management > Delete empty folders.",
        "Lidarr leaves empty folders alone",
    );
    setting(
        lidarr.media.recycle_bin.trim().is_empty(),
        "recycleBin",
        "Lidarr deletes files for good",
        "Without a recycling bin, files Lidarr replaces or removes can't be brought back.",
        "In Lidarr, set a folder in Settings > Media Management > Recycling Bin.",
        "Lidarr keeps removed files in a recycling bin",
    );
    let upgrades: Vec<String> = lidarr.profiles.iter().filter(|p| p.upgrade_allowed).map(|p| p.name.clone()).collect();
    if upgrades.is_empty() {
        ok.push(Finding::new("upgradeAllowed", Level::Ok, "Lidarr doesn't replace files with better copies", "", None));
    } else {
        amber.push(Finding::new(
            "upgradeAllowed",
            Level::Amber,
            "Lidarr replaces files with better copies",
            format!("Upgrades are on in {}, so Lidarr may download an album again later and swap the files.", list_names(&upgrades, 3)),
            Some("In Lidarr, open Settings > Profiles and turn off Upgrades Allowed in each quality profile.".into()),
        ));
    }
    let lists: Vec<String> = lidarr.lists.iter().filter(|l| l.adds_artists()).map(|l| l.name.clone()).collect();
    if lists.is_empty() {
        ok.push(Finding::new("importLists", Level::Ok, "No import lists add artists on their own", "", None));
    } else {
        amber.push(Finding::new(
            "importLists",
            Level::Amber,
            "Import lists add artists on their own",
            format!("{} can add artists and albums to Lidarr without you asking.", list_names(&lists, 3)),
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

    let passed = red.is_empty();
    let mut findings = red;
    findings.append(&mut amber);
    findings.append(&mut ok);
    let wanted = ev.wanted;
    (CheckReport { version: lidarr.version.clone(), findings, passed, checked_at: ctx.now, route: ctx.route }, wanted)
}

/// Keeps only the folders a root-folder check would still want (for tests
/// and the fetch loop's cap).
pub fn dedup_wanted(mut wanted: Vec<String>, have: &HashMap<String, Option<Listing>>) -> Vec<String> {
    let mut seen = HashSet::new();
    wanted.retain(|w| !have.contains_key(w) && seen.insert(w.clone()));
    wanted
}
