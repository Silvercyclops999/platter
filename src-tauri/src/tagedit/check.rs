//! The tag check: what's worth fixing in an album's tags, each with why it
//! matters and a fix through the editor. It reads; it never writes.
//!
//! Each album's own style is learned from most of its files (how ARTIST
//! joins names, where featured artists go, how files are named), and files
//! are compared with it. Nothing about a particular artist is built in.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::Values;
use super::fields::{self, Field, Scheme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rule {
    MissingAlbumArtist,
    AlbumTagsDiffer,
    ReleaseTypeHalf,
    LonelyGroup,
    SplitAlbum,
    UnknownArchiveValue,
    DiscTags,
    StatusMissing,
    NoteMissing,
    ArtistStyle,
    FeatPlacement,
    NumberGaps,
    DuplicateTitle,
    FilenamePattern,
    NumberVsFilename,
    TitleVsFilename,
    StrayTags,
    ShortTrack,
    LowBitrate,
    MissingIsrc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// "Fix" opens the editor on these songs, at this field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub album_id: String,
    pub track_ids: Vec<String>,
    pub field: Option<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable, for "This is fine".
    pub id: String,
    pub rule: Rule,
    pub severity: Severity,
    pub album_id: String,
    pub album_title: String,
    pub track_ids: Vec<String>,
    pub what: String,
    pub why: String,
    /// None without a music folder (the editor can't save).
    pub fix: Option<Fix>,
}

/// One file as the check sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckFile {
    pub track_id: String,
    pub file_name: String,
    /// The folder on the server.
    pub folder: String,
    pub values: Values,
    /// Tag names, when the file itself was read (empty from the cache).
    pub keys: Vec<String>,
    pub duration_ms: i64,
    pub codec: Option<String>,
    pub bitrate: Option<i64>,
    /// Plex's disc number, for albums whose files don't say.
    pub plex_disc: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckAlbum {
    pub album_id: String,
    pub title: String,
    pub files: Vec<CheckFile>,
}

/// What the check needs from the rest of the library.
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// ARCHIVE_GROUP value -> the albums carrying it.
    pub groups: HashMap<String, BTreeSet<String>>,
    /// Server folder -> the Plex albums with files in it.
    pub folders: HashMap<String, BTreeSet<String>>,
    pub titles: HashMap<String, String>,
    /// A custom status tag is set in Settings.
    pub custom_status: bool,
    /// The music folder is set, so fixes can be offered.
    pub can_fix: bool,
}

/// An album's style, learned from most of its files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    pub album_id: String,
    /// "This album's style: ..."
    pub text: String,
}

// ---------------------------------------------------------------- helpers

/// How ARTIST joins several names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Joiner {
    Comma,
    Ampersand,
    Semicolon,
    Slash,
    X,
    And,
    Feat,
}

impl Joiner {
    fn label(self) -> &'static str {
        match self {
            Joiner::Comma => "names joined with commas",
            Joiner::Ampersand => "names joined with &",
            Joiner::Semicolon => "names joined with ;",
            Joiner::Slash => "names joined with /",
            Joiner::X => "names joined with x",
            Joiner::And => "names joined with and",
            Joiner::Feat => "featured artists after feat.",
        }
    }
}

/// The joiners an ARTIST value uses.
pub fn joiners(artist: &str) -> BTreeSet<Joiner> {
    let a = format!(" {} ", artist.to_lowercase());
    let mut out = BTreeSet::new();
    for (pat, j) in [
        (", ", Joiner::Comma),
        (" & ", Joiner::Ampersand),
        ("; ", Joiner::Semicolon),
        (" / ", Joiner::Slash),
        (" x ", Joiner::X),
        (" and ", Joiner::And),
        (" feat. ", Joiner::Feat),
        (" feat ", Joiner::Feat),
        (" ft. ", Joiner::Feat),
        (" featuring ", Joiner::Feat),
    ] {
        if a.contains(pat) {
            out.insert(j);
        }
    }
    out
}

/// Whether a title names featured artists.
pub fn feat_in_title(title: &str) -> bool {
    let t = title.to_lowercase();
    ["(feat.", "(ft.", "(featuring", "[feat.", "[ft.", " feat. ", " ft. "].iter().any(|p| t.contains(p))
}

/// The most common value and how often it's there.
fn majority<T: Ord + Clone>(items: impl IntoIterator<Item = T>) -> Option<(T, usize)> {
    let mut n: BTreeMap<T, usize> = BTreeMap::new();
    for i in items {
        *n.entry(i).or_default() += 1;
    }
    n.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
}

/// How a file is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NamePattern {
    /// "1-01 - Title"
    DiscTrack,
    /// "01 - Title"
    TrackDash,
    /// "01. Title"
    TrackDot,
    /// "01 Title"
    TrackSpace,
    Other,
}

impl NamePattern {
    fn label(self) -> &'static str {
        match self {
            NamePattern::DiscTrack => "\"1-01 - Title\"",
            NamePattern::TrackDash => "\"01 - Title\"",
            NamePattern::TrackDot => "\"01. Title\"",
            NamePattern::TrackSpace => "\"01 Title\"",
            NamePattern::Other => "no number",
        }
    }
}

/// A file name's pattern, disc, track and title part.
pub type ParsedName = (NamePattern, Option<u32>, Option<u32>, String);

pub fn parse_name(file_name: &str) -> ParsedName {
    let stem = match file_name.rfind('.') {
        Some(i) if i > 0 => &file_name[..i],
        _ => file_name,
    };
    let digits = |s: &str| -> (String, usize) {
        let d: String = s.chars().take_while(char::is_ascii_digit).collect();
        let n = d.len();
        (d, n)
    };
    let (a, n) = digits(stem);
    if n == 0 || n > 3 {
        return (NamePattern::Other, None, None, stem.trim().to_string());
    }
    let rest = &stem[n..];
    // Disc and track: "1-01 - Title" or "1-01 Title".
    if let Some(r) = rest.strip_prefix('-') {
        let (b, m) = digits(r);
        if (1..=3).contains(&m) && r[m..].starts_with([' ', '.', '-']) {
            let title = r[m..].trim_start_matches([' ', '-', '.']).trim();
            return (NamePattern::DiscTrack, a.parse().ok(), b.parse().ok(), title.to_string());
        }
    }
    let track = a.parse().ok();
    if let Some(r) = rest.strip_prefix(" - ").or_else(|| rest.strip_prefix(" -")) {
        return (NamePattern::TrackDash, None, track, r.trim().to_string());
    }
    if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix('.')) {
        return (NamePattern::TrackDot, None, track, r.trim().to_string());
    }
    if let Some(r) = rest.strip_prefix(' ') {
        return (NamePattern::TrackSpace, None, track, r.trim_start_matches(['-', ' ']).trim().to_string());
    }
    (NamePattern::Other, None, None, stem.trim().to_string())
}

/// A title as a file name can hold it: characters file systems refuse
/// dropped, " - " for "/", any case, one space between words.
fn name_fold(s: &str) -> String {
    let s = s.replace('/', " - ");
    let kept: String = s.chars().filter(|c| !matches!(c, ':' | '\\' | '?' | '*' | '"' | '<' | '>' | '|')).collect();
    kept.replace(" - ", " ").replace('_', " ").to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Tags that come from rippers and players, not from the library's style.
fn stray(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    let k = k.strip_prefix("TXXX:").or_else(|| k.strip_prefix("----:")).unwrap_or(&k).to_string();
    k.starts_with("REPLAYGAIN")
        || matches!(
            k.as_str(),
            "ITUNNORM" | "ITUNSMPB" | "ITUNES_CDDB_1" | "ENCODER" | "ENCODED_BY" | "ENCODEDBY" | "ENCODING" | "TSSE" | "TENC" | "BPM" | "TBPM"
                | "WXXX" | "PRIV" | "TOOL" | "\u{a9}TOO" | "RIPPING TOOL" | "RIPDATE" | "RIP DATE" | "SOURCEMEDIA" | "LOG"
        )
}

// ---------------------------------------------------------------- the check

struct Out<'a> {
    album: &'a CheckAlbum,
    can_fix: bool,
    findings: Vec<Finding>,
}

impl Out<'_> {
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, rule: Rule, severity: Severity, tracks: Vec<String>, key: &str, field: Option<Field>, what: String, why: &str) {
        let id = format!("{rule:?}:{}:{key}", self.album.album_id);
        let fix = self.can_fix.then(|| Fix { album_id: self.album.album_id.clone(), track_ids: tracks.clone(), field });
        self.findings.push(Finding {
            id,
            rule,
            severity,
            album_id: self.album.album_id.clone(),
            album_title: self.album.title.clone(),
            track_ids: tracks,
            what,
            why: why.to_string(),
            fix,
        });
    }
}

fn get(f: &CheckFile, k: Field) -> Option<&str> {
    f.values.get(&k).map(String::as_str).filter(|v| !v.trim().is_empty())
}

fn songs(n: usize) -> String {
    if n == 1 { "1 song".into() } else { format!("{n} songs") }
}

/// Checks one album. Returns its findings and its style line.
pub fn check_album(album: &CheckAlbum, lib: &Library) -> (Vec<Finding>, Option<Style>) {
    let mut out = Out { album, can_fix: lib.can_fix, findings: Vec::new() };
    let files = &album.files;
    if files.is_empty() {
        return (Vec::new(), None);
    }
    let ids = |pred: &dyn Fn(&CheckFile) -> bool| -> Vec<String> { files.iter().filter(|f| pred(f)).map(|f| f.track_id.clone()).collect() };

    // Album artist on every file.
    let missing = ids(&|f| get(f, Field::AlbumArtist).is_none());
    if !missing.is_empty() {
        out.add(
            Rule::MissingAlbumArtist,
            Severity::Error,
            missing.clone(),
            "",
            Some(Field::AlbumArtist),
            format!("{} without an album artist", songs(missing.len())),
            "Plex decides the artist page by the album artist. Without one, it treats the album as Various Artists.",
        );
    }

    // Album-level values that differ.
    for (f, label) in [(Field::AlbumArtist, "album artist"), (Field::Album, "album name"), (Field::Year, "year")] {
        let vals: BTreeSet<&str> = files.iter().filter_map(|x| get(x, f)).collect();
        let year_of = |v: &str| crate::tags::year_of(v);
        let differ = if f == Field::Year { vals.iter().filter_map(|v| year_of(v)).collect::<BTreeSet<_>>().len() > 1 } else { vals.len() > 1 };
        if differ {
            let list: Vec<&str> = vals.iter().take(3).copied().collect();
            out.add(
                Rule::AlbumTagsDiffer,
                Severity::Warning,
                files.iter().map(|x| x.track_id.clone()).collect(),
                label,
                Some(f),
                format!("The {label} differs between songs: {}", list.join(", ")),
                "Plex keeps one value for the whole album, so the album can split in two or show the wrong one.",
            );
        }
    }

    // Release type with only its second part.
    let half = ids(&|f| {
        get(f, Field::ReleaseType).is_some_and(|v| {
            let first = v.split([';', ',']).next().unwrap_or("").trim().to_ascii_lowercase();
            !first.is_empty() && !fields::PRIMARY_TYPES.contains(&first.as_str())
        })
    });
    if !half.is_empty() {
        let v = files.iter().find_map(|f| get(f, Field::ReleaseType)).unwrap_or_default().to_string();
        out.add(
            Rule::ReleaseTypeHalf,
            Severity::Warning,
            half,
            "",
            Some(Field::ReleaseType),
            format!("Release type \"{v}\" is missing its first part"),
            "Secondary types need both parts, like album;demo or album;live, or Plex files the album in the wrong section.",
        );
    }

    // A group of one.
    let groups: BTreeSet<&str> = files.iter().filter_map(|f| get(f, Field::Group)).collect();
    for g in groups {
        if lib.groups.get(g).is_none_or(|a| a.len() <= 1) {
            out.add(
                Rule::LonelyGroup,
                Severity::Warning,
                ids(&|f| get(f, Field::Group) == Some(g)),
                g,
                Some(Field::Group),
                format!("ARCHIVE_GROUP \"{g}\" is only on this album"),
                "A group joins editions of one album into one card. With one member it does nothing, which is often a typo in one edition's group.",
            );
        }
    }

    // One folder in two Plex albums.
    let folders: BTreeSet<&str> = files.iter().map(|f| f.folder.as_str()).filter(|f| !f.is_empty()).collect();
    for folder in folders {
        if let Some(albums) = lib.folders.get(folder).filter(|a| a.len() > 1) {
            let others: Vec<String> =
                albums.iter().filter(|a| **a != album.album_id).map(|a| lib.titles.get(a).cloned().unwrap_or_else(|| a.clone())).collect();
            out.add(
                Rule::SplitAlbum,
                Severity::Error,
                files.iter().filter(|f| f.folder == folder).map(|f| f.track_id.clone()).collect(),
                folder,
                Some(Field::Album),
                format!("Plex shows this folder as more than one album (also \"{}\")", others.join("\", \"")),
                "Songs in one folder that Plex splits usually differ in album name, album artist or year. Make them match and Plex joins them.",
            );
        }
    }

    // ARCHIVE_* values Platter doesn't know.
    let mut unknown: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in files {
        let mut bad = |what: String| unknown.entry(what).or_default().push(f.track_id.clone());
        if let Some(v) = get(f, Field::ArchiveVersion).filter(|v| !fields::ARCHIVE_VERSIONS.contains(&v.to_lowercase().as_str())) {
            bad(format!("ARCHIVE_VERSION \"{v}\""));
        }
        if let Some(v) = get(f, Field::Quality).filter(|v| !fields::ARCHIVE_QUALITIES.contains(&v.to_lowercase().as_str())) {
            bad(format!("ARCHIVE_QUALITY \"{v}\""));
        }
        if let Some(v) = get(f, Field::Flags) {
            for flag in v.split(',').map(str::trim).filter(|x| !x.is_empty() && !fields::ARCHIVE_FLAGS.contains(x)) {
                bad(format!("ARCHIVE_FLAGS \"{flag}\""));
            }
        }
        if let Some(v) = get(f, Field::ArchiveDate).filter(|v| !fields::archive_date_ok(v)) {
            bad(format!("ARCHIVE_DATE \"{v}\""));
        }
        for k in &f.keys {
            let bare = k.strip_prefix("TXXX:").or_else(|| k.strip_prefix("----:")).unwrap_or(k);
            if bare.starts_with("ARCHIVE_") && !fields::ARCHIVE_KEYS.contains(&bare) {
                bad(format!("the tag {bare}"));
            }
        }
    }
    for (what, tracks) in unknown {
        out.add(
            Rule::UnknownArchiveValue,
            Severity::Warning,
            tracks,
            &what,
            None,
            format!("Platter doesn't know {what}"),
            "Archive tags only work with the values in the tagging guide. An unknown one is ignored, so the badge or filter it was meant for doesn't show.",
        );
    }

    // Disc tags on albums with more than one disc.
    let discs: BTreeSet<i64> = files
        .iter()
        .map(|f| get(f, Field::DiscNumber).and_then(|d| d.parse().ok()).unwrap_or(f.plex_disc.max(1)))
        .collect();
    let total_many = files.iter().any(|f| get(f, Field::DiscTotal).and_then(|d| d.parse::<i64>().ok()).is_some_and(|d| d > 1));
    if discs.len() > 1 || total_many {
        let missing = ids(&|f| get(f, Field::DiscNumber).is_none() || get(f, Field::DiscTotal).is_none());
        if !missing.is_empty() {
            out.add(
                Rule::DiscTags,
                Severity::Warning,
                missing.clone(),
                "",
                Some(Field::DiscNumber),
                format!("{} on this multi-disc album without a disc number or disc total", songs(missing.len())),
                "Without them, songs from different discs sort together and players can't show the discs.",
            );
        }
    }

    // Release status and provenance where the album's other files have them.
    let scheme = fields::detect_scheme(files.iter().map(|f| &f.values), lib.custom_status);
    let sf = scheme.field();
    let with: Vec<&CheckFile> = files.iter().filter(|f| get(f, sf).is_some()).collect();
    if !with.is_empty() && with.len() < files.len() {
        let missing = ids(&|f| get(f, sf).is_none());
        out.add(
            Rule::StatusMissing,
            Severity::Warning,
            missing.clone(),
            "",
            Some(sf),
            format!("{} without a release status, while the album's other songs have one", songs(missing.len())),
            "Songs without a status count as unknown, so they're left out of what's shown as released or unreleased.",
        );
    }
    let note = |f: &CheckFile| get(f, Field::VersionNote).is_some() || get(f, Field::Source).is_some();
    let noted = files.iter().filter(|f| note(f)).count();
    if noted * 2 > files.len() && noted < files.len() {
        let missing = ids(&|f| !note(f));
        out.add(
            Rule::NoteMissing,
            Severity::Info,
            missing.clone(),
            "",
            Some(if scheme == Scheme::Archive { Field::Source } else { Field::VersionNote }),
            format!("{} without a provenance note, while most of the album has one", songs(missing.len())),
            "The note says where a file came from. A gap makes it hard to tell later which copy this is.",
        );
    }

    // How ARTIST joins names, and where features go.
    let mut style_parts: Vec<String> = Vec::new();
    let joined: Vec<(&CheckFile, BTreeSet<Joiner>)> =
        files.iter().filter_map(|f| get(f, Field::Artist).map(|a| (f, joiners(a)))).filter(|(_, j)| !j.is_empty()).collect();
    let main = majority(joined.iter().flat_map(|(_, j)| j.iter().copied().filter(|j| *j != Joiner::Feat)));
    if let Some((m, _)) = main {
        style_parts.push(format!("ARTIST {}", m.label()));
        let odd: Vec<String> = joined
            .iter()
            .filter(|(_, j)| j.iter().any(|x| *x != Joiner::Feat && *x != m))
            .map(|(f, _)| f.track_id.clone())
            .collect();
        if !odd.is_empty() {
            out.add(
                Rule::ArtistStyle,
                Severity::Warning,
                odd.clone(),
                "",
                Some(Field::Artist),
                format!("{} join artist names differently from the rest of the album", songs(odd.len())),
                "Plex splits artists only on the separators it knows, so mixed ones give some songs a combined artist that has its own page.",
            );
        }
    }
    let in_title = files.iter().filter(|f| get(f, Field::Title).is_some_and(feat_in_title)).count();
    let in_artist = joined.iter().filter(|(_, j)| j.contains(&Joiner::Feat)).count();
    if in_title > 0 || in_artist > 0 {
        style_parts.push(if in_title >= in_artist { "features in the title as (feat. ...)".into() } else { "features in ARTIST after feat.".into() });
    }
    if in_title > 0 && in_artist > 0 {
        let odd: Vec<String> = if in_title >= in_artist {
            joined.iter().filter(|(_, j)| j.contains(&Joiner::Feat)).map(|(f, _)| f.track_id.clone()).collect()
        } else {
            ids(&|f| get(f, Field::Title).is_some_and(feat_in_title))
        };
        out.add(
            Rule::FeatPlacement,
            Severity::Warning,
            odd.clone(),
            "",
            Some(if in_title >= in_artist { Field::Artist } else { Field::Title }),
            format!("{} put featured artists in a different place from the rest of the album", songs(odd.len())),
            "Mixed placement makes the same song look different in lists and search, and versions of a song stop matching.",
        );
    }

    // Numbering per disc: gaps and repeats.
    let mut by_disc: BTreeMap<i64, Vec<(i64, String)>> = BTreeMap::new();
    for f in files {
        let d = get(f, Field::DiscNumber).and_then(|d| d.parse().ok()).unwrap_or(f.plex_disc.max(1));
        if let Some(t) = get(f, Field::TrackNumber).and_then(|t| t.parse::<i64>().ok()) {
            by_disc.entry(d).or_default().push((t, f.track_id.clone()));
        }
    }
    for (d, mut ts) in by_disc {
        ts.sort();
        let nums: Vec<i64> = ts.iter().map(|x| x.0).collect();
        let repeated: BTreeSet<i64> = nums.windows(2).filter(|w| w[0] == w[1]).map(|w| w[0]).collect();
        let max = nums.last().copied().unwrap_or(0);
        let gaps: Vec<i64> = (1..=max).filter(|n| !nums.contains(n)).collect();
        let disc = if discs.len() > 1 { format!(" on disc {d}") } else { String::new() };
        if !repeated.is_empty() {
            let r: Vec<String> = repeated.iter().map(i64::to_string).collect();
            out.add(
                Rule::NumberGaps,
                Severity::Warning,
                ts.iter().filter(|x| repeated.contains(&x.0)).map(|x| x.1.clone()).collect(),
                &format!("repeat:{d}"),
                Some(Field::TrackNumber),
                format!("Track number {} is used twice{disc}", r.join(", ")),
                "Two songs with one number play in an unpredictable order.",
            );
        }
        if !gaps.is_empty() && gaps.len() < max as usize {
            let g: Vec<String> = gaps.iter().take(5).map(i64::to_string).collect();
            out.add(
                Rule::NumberGaps,
                Severity::Info,
                Vec::new(),
                &format!("gap:{d}"),
                Some(Field::TrackNumber),
                format!("No track {}{disc}", g.join(", ")),
                "A gap in the numbers usually means a song is missing from the folder, or a number is wrong.",
            );
        }
    }

    // The same title twice.
    let mut titles: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in files {
        if let Some(t) = get(f, Field::Title) {
            titles.entry(t.trim().to_lowercase()).or_default().push(f.track_id.clone());
        }
    }
    for (t, tr) in titles.into_iter().filter(|(_, v)| v.len() > 1) {
        out.add(
            Rule::DuplicateTitle,
            Severity::Info,
            tr.clone(),
            &t,
            Some(Field::Title),
            format!("\"{}\" is on the album {} times", files.iter().find_map(|f| get(f, Field::Title).filter(|x| x.trim().to_lowercase() == t)).unwrap_or(&t), tr.len()),
            "It may be a duplicate file, or two versions that need a version note to tell apart.",
        );
    }

    // File names: one pattern, numbers and titles that agree with the tags.
    let named: Vec<(&CheckFile, ParsedName)> = files.iter().map(|f| (f, parse_name(&f.file_name))).collect();
    if let Some((pat, n)) = majority(named.iter().map(|(_, p)| p.0)) {
        if n * 2 > named.len() {
            style_parts.push(format!("files named {}", pat.label()));
        }
        let odd: Vec<String> = named.iter().filter(|(_, p)| p.0 != pat).map(|(f, _)| f.track_id.clone()).collect();
        if !odd.is_empty() {
            out.add(
                Rule::FilenamePattern,
                Severity::Info,
                odd.clone(),
                "",
                None,
                format!("{} named differently from the rest of the album, which uses {}", songs(odd.len()), pat.label()),
                "Platter never renames files. Mixed names are a hint that a song came from another copy of the album.",
            );
        }
    }
    for (f, (pat, disc, track, title)) in &named {
        if *pat == NamePattern::Other {
            continue;
        }
        let tag_track = get(f, Field::TrackNumber).and_then(|t| t.parse::<u32>().ok());
        let tag_disc = get(f, Field::DiscNumber).and_then(|t| t.parse::<u32>().ok());
        let track_off = track.is_some() && tag_track.is_some() && *track != tag_track;
        let disc_off = disc.is_some() && tag_disc.is_some() && *disc != tag_disc;
        if track_off || disc_off {
            out.add(
                Rule::NumberVsFilename,
                Severity::Warning,
                vec![f.track_id.clone()],
                &f.track_id,
                Some(if disc_off { Field::DiscNumber } else { Field::TrackNumber }),
                format!("{} says {} in its name but {} in its tags", f.file_name, if disc_off { format!("disc {}", disc.unwrap_or(0)) } else { format!("track {}", track.unwrap_or(0)) }, if disc_off { format!("disc {}", tag_disc.unwrap_or(0)) } else { format!("track {}", tag_track.unwrap_or(0)) }),
                "Players sort by the tags, so the album plays in a different order than the folder shows.",
            );
        }
        if let Some(t) = get(f, Field::Title)
            && !title.is_empty()
            && name_fold(t) != name_fold(title)
        {
            out.add(
                Rule::TitleVsFilename,
                Severity::Info,
                vec![f.track_id.clone()],
                &format!("title:{}", f.track_id),
                Some(Field::Title),
                format!("The title \"{t}\" isn't the one in the file name \"{}\"", f.file_name),
                "One of them may be out of date. Platter shows the tag, and never renames the file.",
            );
        }
    }

    // Tags from rippers and players that most of the album doesn't carry.
    let read: Vec<&CheckFile> = files.iter().filter(|f| !f.keys.is_empty()).collect();
    if !read.is_empty() {
        let mut count: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for f in &read {
            for k in f.keys.iter().filter(|k| stray(k)) {
                count.entry(k.as_str()).or_default().push(f.track_id.clone());
            }
        }
        for (k, tr) in count {
            if tr.len() * 2 <= read.len() {
                out.add(
                    Rule::StrayTags,
                    Severity::Info,
                    tr.clone(),
                    k,
                    None,
                    format!("{} {} the tag {k}, which the rest of the album doesn't use", songs(tr.len()), if tr.len() == 1 { "has" } else { "have" }),
                    "Left over from a ripper or player. Harmless, but a sign the file came from somewhere else. The editor never adds tags like it.",
                );
            }
        }
    }

    // Short songs and low-bitrate MP3s: worth knowing, nothing wrong.
    let short = ids(&|f| f.duration_ms > 0 && f.duration_ms < 60_000);
    if !short.is_empty() && short.len() < files.len() {
        out.add(Rule::ShortTrack, Severity::Info, short.clone(), "", None, format!("{} under a minute long", songs(short.len())), "Often an intro or skit, sometimes a snippet or a cut-off file.");
    }
    let low = ids(&|f| f.codec.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("mp3")) && f.bitrate.is_some_and(|b| b > 0 && b < 192));
    if !low.is_empty() {
        out.add(Rule::LowBitrate, Severity::Info, low.clone(), "", Some(Field::Quality), format!("{} in MP3 under 192 kbps", songs(low.len())), "A low-bitrate copy. If a better one turns up, it can replace this file.");
    }

    // Released songs without an ISRC: info only.
    let released = |f: &CheckFile| {
        let t = to_file_tags(&f.values);
        crate::tags::provenance::resolve(&t, &crate::tags::provenance::config()).status == Some(crate::tags::provenance::FileRelease::Released)
    };
    let no_isrc = ids(&|f| released(f) && get(f, Field::Isrc).is_none());
    if !no_isrc.is_empty() {
        out.add(
            Rule::MissingIsrc,
            Severity::Info,
            no_isrc.clone(),
            "",
            Some(Field::Isrc),
            format!("{} released without an ISRC", songs(no_isrc.len())),
            "An ISRC lets other services find the exact recording. Look up ISRCs can fill them from free sources.",
        );
    }

    let style = (!style_parts.is_empty()).then(|| Style { album_id: album.album_id.clone(), text: format!("This album's style: {}", style_parts.join(", ")) });
    (out.findings, style)
}

/// The file's values as the tag reader keeps them, for its release status.
pub fn to_file_tags(v: &Values) -> crate::tags::FileTags {
    let mut t = crate::tags::FileTags::for_read();
    for (f, val) in v {
        let key = match f {
            Field::CustomStatus => crate::tags::provenance::custom_key(),
            _ => f.vorbis_key().map(str::to_string),
        };
        if let Some(k) = key {
            t.set(&k, val);
        }
    }
    t
}

/// The check over many albums, worst first, without the ones marked fine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub findings: Vec<Finding>,
    pub styles: Vec<Style>,
    pub albums: usize,
    pub files: usize,
    /// Findings left out because they were marked "This is fine".
    pub ignored: usize,
    /// Read from the files themselves (the music folder is set), else from
    /// what Plex and the tag cache have.
    pub from_files: bool,
}

pub fn report(albums: &[CheckAlbum], lib: &Library, ignored: &BTreeSet<String>, from_files: bool) -> Report {
    let mut r = Report { albums: albums.len(), files: albums.iter().map(|a| a.files.len()).sum(), from_files, ..Default::default() };
    for a in albums {
        let (f, s) = check_album(a, lib);
        for x in f {
            if ignored.contains(&x.id) {
                r.ignored += 1;
            } else {
                r.findings.push(x);
            }
        }
        r.styles.extend(s);
    }
    r.findings.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.album_title.cmp(&b.album_title)));
    r
}
