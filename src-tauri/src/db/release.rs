//! Release type → artist-page section (docs/PLEX-API.md, docs/TAGGING.md).
//!
//! Plex exposes the RELEASETYPE tag as album `Format` ("Album", "EP",
//! "Single") and `Subformat` ("Live", "Compilation", "Soundtrack", "Remix",
//! "Demo", "Mixtape/Street"). Untagged albums have no Format → Albums.

use crate::model::ReleaseSection;

/// Every section, in the order stored ranks follow.
pub const SECTION_ORDER: [ReleaseSection; 9] = [
    ReleaseSection::Albums,
    ReleaseSection::SinglesEps,
    ReleaseSection::Live,
    ReleaseSection::Compilations,
    ReleaseSection::Soundtracks,
    ReleaseSection::Remixes,
    ReleaseSection::Demos,
    ReleaseSection::Mixtapes,
    ReleaseSection::Unreleased,
];

/// The order of the sections (and their chips) on an artist page.
pub const PAGE_ORDER: [ReleaseSection; 9] = [
    ReleaseSection::Albums,
    ReleaseSection::SinglesEps,
    ReleaseSection::Mixtapes,
    ReleaseSection::Unreleased,
    ReleaseSection::Live,
    ReleaseSection::Compilations,
    ReleaseSection::Soundtracks,
    ReleaseSection::Remixes,
    ReleaseSection::Demos,
];

/// Map Plex format/subformats (any case) to a section.
///
/// A secondary type wins over the primary one. When several subformats are
/// present the most specific wins: Demo > Mixtape > Live > Remix >
/// Soundtrack > Compilation (an unreleased live demo belongs with demos).
pub fn release_section<S: AsRef<str>>(format: Option<&str>, subformats: &[S]) -> ReleaseSection {
    let subs: Vec<String> = subformats.iter().map(|s| s.as_ref().to_ascii_lowercase()).collect();
    let has = |needle: &str| subs.iter().any(|s| s.contains(needle));
    if has("demo") {
        return ReleaseSection::Demos;
    }
    if has("mixtape") {
        return ReleaseSection::Mixtapes;
    }
    if has("live") {
        return ReleaseSection::Live;
    }
    if has("remix") {
        return ReleaseSection::Remixes;
    }
    if has("soundtrack") {
        return ReleaseSection::Soundtracks;
    }
    if has("compilation") {
        return ReleaseSection::Compilations;
    }
    match format.map(|f| f.trim().to_ascii_lowercase()).as_deref() {
        Some("ep") | Some("single") => ReleaseSection::SinglesEps,
        _ => ReleaseSection::Albums,
    }
}

/// Whether an album counts as unreleased, so it never gets a public link.
///
/// The one place that decides it: a demo release type (format or any
/// subformat), or a version read from the files that isn't a released one
/// (see [`version_is_unreleased`]). The version is ARCHIVE_VERSION, else the
/// file's status from whichever tags it carries (`tags::provenance`; the
/// SQL forms are noted in `tags::store`). For an album, pass the version
/// most of its files carry, else its status from its files.
///
/// With [`count_unmatched`] on (the default), an album Plex couldn't match
/// counts as probably unreleased too, until it's tagged. `matched` comes
/// from the album's guid; when it isn't known yet (a cache from before the
/// guid was kept), no release type and no label (`studio`) stands in for
/// it. Any release type or ARCHIVE_VERSION is a real tag and wins.
///
/// `mb_found` is what the MusicBrainz check said about an album Plex
/// couldn't match (`album_mb_check.found`, None when not checked): with
/// Some(true) MusicBrainz has the release, so Plex only missed the match
/// and the album counts as released. Ignored while [`use_musicbrainz`] is
/// off, and tags always win over it.
///
/// This form knows nothing of the user's marks; [`album_unreleased`] and
/// [`track_unreleased`] add them.
pub fn is_unreleased<S: AsRef<str>>(
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
    mb_found: Option<bool>,
) -> bool {
    unreleased_reason(format, subformats, matched, studio, archive_version, mb_found).is_some()
}

/// [`is_unreleased`] for an album, with the user's mark on it.
pub fn album_unreleased<S: AsRef<str>>(
    album_id: &str,
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
    mb_found: Option<bool>,
) -> bool {
    let f = facts(format, subformats, matched, studio, archive_version, mb_found, mark_for(album_id, None));
    decide(&f, false).0.is_unreleased()
}

/// [`is_unreleased`] for one song: its own mark, else its album's.
/// `archive_version` is the song's file version, else the album's.
#[allow(clippy::too_many_arguments)]
pub fn track_unreleased<S: AsRef<str>>(
    track_id: &str,
    album_id: &str,
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
    mb_found: Option<bool>,
) -> bool {
    let f = facts(format, subformats, matched, studio, archive_version, mb_found, mark_for(album_id, Some(track_id)));
    decide(&f, false).0.is_unreleased()
}

/// How sure [`is_unreleased`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreleased {
    /// A demo release type or an unreleased ARCHIVE_VERSION says so.
    Tagged,
    /// Plex couldn't match the album and nothing is tagged.
    Probably,
}

/// [`is_unreleased`] with its reason; None when the album counts as released.
pub fn unreleased_reason<S: AsRef<str>>(
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
    mb_found: Option<bool>,
) -> Option<Unreleased> {
    match release_status(format, subformats, matched, studio, archive_version, mb_found).0 {
        ReleaseStatus::TaggedUnreleased | ReleaseStatus::ConfirmedUnreleased => Some(Unreleased::Tagged),
        ReleaseStatus::ProbablyUnreleased => Some(Unreleased::Probably),
        ReleaseStatus::PlexMissed | ReleaseStatus::Released => None,
    }
}

/// An album's release status as the album page shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseStatus {
    /// A demo release type or an unreleased ARCHIVE_VERSION.
    TaggedUnreleased,
    /// The user marked it unreleased (no file tag says otherwise).
    ConfirmedUnreleased,
    /// Plex couldn't match it and nothing is tagged.
    ProbablyUnreleased,
    /// Plex couldn't match it, but MusicBrainz has the release.
    PlexMissed,
    Released,
}

impl ReleaseStatus {
    pub fn is_unreleased(self) -> bool {
        matches!(self, Self::TaggedUnreleased | Self::ConfirmedUnreleased | Self::ProbablyUnreleased)
    }

    /// Stable string stored in SQLite.
    pub fn key(self) -> &'static str {
        match self {
            Self::TaggedUnreleased => "taggedUnreleased",
            Self::ConfirmedUnreleased => "confirmedUnreleased",
            Self::ProbablyUnreleased => "probablyUnreleased",
            Self::PlexMissed => "plexMissed",
            Self::Released => "released",
        }
    }

    pub fn from_key(k: &str) -> Self {
        match k {
            "taggedUnreleased" => Self::TaggedUnreleased,
            "confirmedUnreleased" => Self::ConfirmedUnreleased,
            "probablyUnreleased" => Self::ProbablyUnreleased,
            "plexMissed" => Self::PlexMissed,
            _ => Self::Released,
        }
    }
}

/// The user's answer to "is this unreleased?" on an album or a song. App
/// state (the sync store), never written to files here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    Unreleased,
    Released,
}

/// What a piece of evidence is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceKind {
    /// A release type or ARCHIVE_VERSION in the files.
    Tag,
    /// The user's mark.
    Mark,
    /// The user's mark, which a file tag overrides.
    MarkOverridden,
    /// Plex couldn't match it.
    Unmatched,
    /// No release type in the tags.
    Untyped,
    /// What the MusicBrainz check said.
    Musicbrainz,
    /// MusicBrainz wasn't asked.
    NotChecked,
    /// A folder name on the songs' paths.
    Folder,
    /// Most songs are snippet length.
    Duration,
}

/// One piece of evidence behind a release status, for "Why?".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub kind: EvidenceKind,
    /// A plain sentence.
    pub text: String,
    /// How far it leans toward unreleased (positive) or released
    /// (negative). Hints are small and never decide alone.
    pub weight: i32,
    /// This piece settled the status.
    pub decides: bool,
}

/// Weights per kind of evidence.
pub const W_TAG: i32 = 100;
pub const W_MARK: i32 = 90;
pub const W_UNMATCHED: i32 = 30;
pub const W_UNTYPED: i32 = 10;
pub const W_MB: i32 = 40;
pub const W_FOLDER: i32 = 10;
pub const W_SHORT: i32 = 5;

/// Songs shorter than this are snippet length.
pub const SNIPPET_MS: u64 = 60_000;

/// Everything the status of an album (or a song on it) is decided from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlbumFacts {
    pub format: Option<String>,
    pub subformats: Vec<String>,
    pub matched: Option<bool>,
    pub studio: Option<String>,
    pub version: Option<String>,
    /// The album's status by its files' release status tags (STATUS,
    /// RELEASESTATUS or a custom tag, `tags::provenance::album_release`).
    /// ARCHIVE_VERSION in `version` comes first.
    pub release: Option<String>,
    /// Its files mix released and other material.
    pub mixed: bool,
    pub mb_found: Option<bool>,
    pub mark: Option<Mark>,
    /// Folder keywords found on the songs' paths ([`folder_hits`]).
    pub folders: Vec<String>,
    /// Songs under [`SNIPPET_MS`], of `songs`.
    pub short_songs: usize,
    pub songs: usize,
}

#[allow(clippy::too_many_arguments)]
fn facts<S: AsRef<str>>(
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    version: Option<&str>,
    mb_found: Option<bool>,
    mark: Option<Mark>,
) -> AlbumFacts {
    AlbumFacts {
        format: format.map(str::to_string),
        subformats: subformats.iter().map(|s| s.as_ref().to_string()).collect(),
        matched,
        studio: studio.map(str::to_string),
        version: version.map(str::to_string),
        mb_found,
        mark,
        ..Default::default()
    }
}

/// [`unreleased_reason`] with the evidence behind it, as plain sentences
/// for the album page's "Why?" (no marks, no hints).
pub fn release_status<S: AsRef<str>>(
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
    mb_found: Option<bool>,
) -> (ReleaseStatus, Vec<String>) {
    let (s, ev) = assess(&facts(format, subformats, matched, studio, archive_version, mb_found, None));
    (s, ev.into_iter().filter(|e| e.weight != 0).map(|e| e.text).collect())
}

/// The status and every piece of evidence, strongest rule first:
///
/// 1. File tags: a demo release type, an ARCHIVE_VERSION, or else the
///    status the files' release status tags give the album (disc 1
///    decides, so an official album with a disc of extras stays official).
///    Final; a mark that disagrees is listed as overridden.
/// 2. The user's mark. Final otherwise.
/// 3. Any other release type: released.
/// 4. Plex matched it: released.
/// 5. Plex couldn't match it: MusicBrainz finding the release makes it
///    released ("Plex missed it"); otherwise probably unreleased while
///    [`count_unmatched`] is on, more surely when MusicBrainz has nothing.
///
/// Folder names and snippet lengths only add weight to the list. They never
/// change the status.
pub fn assess(f: &AlbumFacts) -> (ReleaseStatus, Vec<Evidence>) {
    decide(f, true)
}

/// [`assess`] without the list, for loops over many songs.
pub fn status_of(f: &AlbumFacts) -> ReleaseStatus {
    decide(f, false).0
}

/// Adds a piece of evidence when the caller wants the list.
fn note(ev: &mut Vec<Evidence>, explain: bool, kind: EvidenceKind, text: impl FnOnce() -> String, weight: i32, decides: bool) {
    if explain {
        ev.push(Evidence { kind, text: text(), weight, decides });
    }
}

fn decide(f: &AlbumFacts, explain: bool) -> (ReleaseStatus, Vec<Evidence>) {
    let mut ev: Vec<Evidence> = Vec::new();
    let demo = |s: &str| s.to_ascii_lowercase().contains("demo");
    let blank = |s: Option<&str>| s.is_none_or(|s| s.trim().is_empty());
    let overridden = |unreleased: bool| f.mark.is_some_and(|m| (m == Mark::Unreleased) != unreleased);
    const OVERRIDDEN: &str = "Your mark is overridden by the file's tags.";
    let status = if f.format.as_deref().is_some_and(demo) || f.subformats.iter().any(|s| demo(s)) {
        note(&mut ev, explain, EvidenceKind::Tag, || "Its release type is tagged as a demo.".into(), W_TAG, true);
        if overridden(true) {
            note(&mut ev, explain, EvidenceKind::MarkOverridden, || OVERRIDDEN.into(), 0, false);
        }
        ReleaseStatus::TaggedUnreleased
    } else if let Some(v) = f.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let unreleased = version_is_unreleased(v);
        let w = if unreleased { W_TAG } else { -W_TAG };
        note(&mut ev, explain, EvidenceKind::Tag, || format!("Its files are tagged as version \"{v}\"."), w, true);
        if overridden(unreleased) {
            note(&mut ev, explain, EvidenceKind::MarkOverridden, || OVERRIDDEN.into(), 0, false);
        }
        if unreleased { ReleaseStatus::TaggedUnreleased } else { ReleaseStatus::Released }
    } else if let Some(r) = f.release.as_deref().and_then(crate::tags::provenance::FileRelease::from_key) {
        let unreleased = !r.is_released();
        let text = || {
            if unreleased {
                format!("Its files are tagged as {}.", r.label())
            } else if f.mixed {
                "Most songs on its first disc are tagged as released.".to_string()
            } else {
                "Its files are tagged as released.".to_string()
            }
        };
        note(&mut ev, explain, EvidenceKind::Tag, text, if unreleased { W_TAG } else { -W_TAG }, true);
        if overridden(unreleased) {
            note(&mut ev, explain, EvidenceKind::MarkOverridden, || OVERRIDDEN.into(), 0, false);
        }
        if unreleased { ReleaseStatus::TaggedUnreleased } else { ReleaseStatus::Released }
    } else if let Some(m) = f.mark {
        match m {
            Mark::Unreleased => {
                note(&mut ev, explain, EvidenceKind::Mark, || "You confirmed it's unreleased.".into(), W_MARK, true);
                ReleaseStatus::ConfirmedUnreleased
            }
            Mark::Released => {
                note(&mut ev, explain, EvidenceKind::Mark, || "You marked it as released.".into(), -W_MARK, true);
                ReleaseStatus::Released
            }
        }
    } else {
        let typed = !blank(f.format.as_deref()) || f.subformats.iter().any(|s| !s.trim().is_empty());
        let unmatched = !typed
            && match f.matched {
                Some(m) => !m,
                None => blank(f.studio.as_deref()),
            };
        if !unmatched {
            ReleaseStatus::Released
        } else {
            let mb = if use_musicbrainz() { f.mb_found } else { None };
            if mb != Some(true) && !count_unmatched() {
                ReleaseStatus::Released
            } else {
                note(&mut ev, explain, EvidenceKind::Unmatched, || "Plex couldn't match it to a known release.".into(), W_UNMATCHED, false);
                note(&mut ev, explain, EvidenceKind::Untyped, || "Nothing in its tags says what kind of release it is.".into(), W_UNTYPED, false);
                match mb {
                    Some(true) => {
                        note(
                            &mut ev,
                            explain,
                            EvidenceKind::Musicbrainz,
                            || "MusicBrainz has an official release with the same artist, title and about as many songs.".into(),
                            -W_MB,
                            true,
                        );
                        ReleaseStatus::PlexMissed
                    }
                    Some(false) => {
                        note(&mut ev, explain, EvidenceKind::Musicbrainz, || "MusicBrainz has no release by this artist with this title.".into(), W_MB, false);
                        ReleaseStatus::ProbablyUnreleased
                    }
                    None => {
                        note(&mut ev, explain, EvidenceKind::NotChecked, || "MusicBrainz not checked.".into(), 0, false);
                        ReleaseStatus::ProbablyUnreleased
                    }
                }
            }
        }
    };
    if explain {
        if f.mixed {
            ev.push(Evidence {
                kind: EvidenceKind::Tag,
                text: "Some of its songs are tagged as released and some as unreleased, leaked or alternate.".into(),
                weight: 0,
                decides: false,
            });
        }
        for k in &f.folders {
            ev.push(Evidence { kind: EvidenceKind::Folder, text: format!("Folder name: {k}"), weight: W_FOLDER, decides: false });
        }
        if f.songs > 0 && f.short_songs * 2 > f.songs {
            let text = if f.songs == 1 { "It's under a minute long." } else { "Most of its songs are under a minute long." };
            ev.push(Evidence { kind: EvidenceKind::Duration, text: text.into(), weight: W_SHORT, decides: false });
        }
    }
    (status, ev)
}

/// How strongly the evidence leans: the sum of the weights.
pub fn score(ev: &[Evidence]) -> i32 {
    ev.iter().map(|e| e.weight).sum()
}

/// Folder keywords that appear as a word in a folder name on `path` (the
/// file name itself doesn't count), any case, singular or plural ("leak"
/// finds "Leaks"). Each keyword once, as written in `keywords`.
pub fn folder_hits<S: AsRef<str>>(path: &str, keywords: &[S]) -> Vec<String> {
    let mut segs: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    segs.pop();
    let stem = |w: &str| w.strip_suffix('s').filter(|x| !x.is_empty()).unwrap_or(w).to_string();
    let words: Vec<String> = segs
        .iter()
        .flat_map(|s| {
            crate::identity::fold::fold_text(s)
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(stem)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    for k in keywords {
        let k = k.as_ref().trim();
        let kf = stem(&crate::identity::fold::fold_text(k));
        if kf.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(k)) {
            continue;
        }
        if words.contains(&kf) {
            out.push(k.to_string());
        }
    }
    out
}

// The user's marks (album:<id> and track:<id>, from the sync store) and the
// folder keywords setting. Tests keep them per thread.
type MarkMap = std::collections::HashMap<String, Mark>;
#[cfg(not(test))]
static MARKS: std::sync::LazyLock<parking_lot::RwLock<MarkMap>> = std::sync::LazyLock::new(Default::default);
#[cfg(not(test))]
static FOLDER_KEYWORDS: std::sync::LazyLock<parking_lot::RwLock<Vec<String>>> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(default_folder_keywords()));
#[cfg(test)]
thread_local! {
    static MARKS: std::cell::RefCell<MarkMap> = Default::default();
    static FOLDER_KEYWORDS: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(default_folder_keywords());
}

/// Folder names that hint at unreleased material, until the user changes them.
pub fn default_folder_keywords() -> Vec<String> {
    ["Unreleased", "Leaks", "Demos", "Sessions", "Snippets"].iter().map(|s| s.to_string()).collect()
}

/// The key a mark is kept under: `album:<id>` or `track:<id>`.
pub fn mark_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// Replace every mark (from the sync store).
pub fn set_marks(m: MarkMap) {
    #[cfg(not(test))]
    {
        *MARKS.write() = m;
    }
    #[cfg(test)]
    MARKS.with(|x| *x.borrow_mut() = m);
}

pub fn mark_of(key: &str) -> Option<Mark> {
    #[cfg(not(test))]
    return MARKS.read().get(key).copied();
    #[cfg(test)]
    return MARKS.with(|x| x.borrow().get(key).copied());
}

/// A song's own mark, else its album's.
pub fn mark_for(album_id: &str, track_id: Option<&str>) -> Option<Mark> {
    track_id.and_then(|t| mark_of(&mark_key("track", t))).or_else(|| mark_of(&mark_key("album", album_id)))
}

pub fn folder_keywords() -> Vec<String> {
    #[cfg(not(test))]
    return FOLDER_KEYWORDS.read().clone();
    #[cfg(test)]
    return FOLDER_KEYWORDS.with(|x| x.borrow().clone());
}

/// Set from the settings. Returns whether they changed.
pub fn set_folder_keywords(k: Vec<String>) -> bool {
    #[cfg(not(test))]
    {
        let mut cur = FOLDER_KEYWORDS.write();
        let changed = *cur != k;
        *cur = k;
        changed
    }
    #[cfg(test)]
    FOLDER_KEYWORDS.with(|x| {
        let changed = *x.borrow() != k;
        *x.borrow_mut() = k;
        changed
    })
}

// The "count albums Plex couldn't match as unreleased" and "check
// MusicBrainz for albums Plex couldn't match" settings. Tests keep them per
// thread so they can't see each other's changes.
#[cfg(not(test))]
static COUNT_UNMATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[cfg(not(test))]
static USE_MUSICBRAINZ: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[cfg(test)]
thread_local! {
    static COUNT_UNMATCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static USE_MUSICBRAINZ: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether albums Plex couldn't match count as unreleased.
pub fn count_unmatched() -> bool {
    #[cfg(not(test))]
    return COUNT_UNMATCHED.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    return COUNT_UNMATCHED.with(|c| c.get());
}

/// Set from the settings.
pub fn set_count_unmatched(on: bool) {
    #[cfg(not(test))]
    COUNT_UNMATCHED.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    COUNT_UNMATCHED.with(|c| c.set(on));
}

/// Whether the MusicBrainz check's answers count: its setting, with
/// MusicBrainz on.
pub fn use_musicbrainz() -> bool {
    #[cfg(not(test))]
    return USE_MUSICBRAINZ.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    return USE_MUSICBRAINZ.with(|c| c.get());
}

/// Set from the settings.
pub fn set_use_musicbrainz(on: bool) {
    #[cfg(not(test))]
    USE_MUSICBRAINZ.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    USE_MUSICBRAINZ.with(|c| c.set(on));
}
/// ARCHIVE_VERSION values that mean the music was never released, and the
/// file statuses `tags::provenance` gives ("leaked" among them). Remix and
/// live versions can be released ones, so they don't count on their own.
pub fn version_is_unreleased(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "unreleased" | "leak" | "leaked" | "demo" | "snippet" | "session" | "build" | "alternate"
    )
}

/// Stable string stored in SQLite.
pub fn section_key(s: ReleaseSection) -> &'static str {
    match s {
        ReleaseSection::Albums => "albums",
        ReleaseSection::SinglesEps => "singlesEps",
        ReleaseSection::Live => "live",
        ReleaseSection::Compilations => "compilations",
        ReleaseSection::Soundtracks => "soundtracks",
        ReleaseSection::Remixes => "remixes",
        ReleaseSection::Demos => "demos",
        ReleaseSection::Mixtapes => "mixtapes",
        ReleaseSection::Unreleased => "unreleased",
    }
}

pub fn section_from_key(k: &str) -> ReleaseSection {
    SECTION_ORDER
        .iter()
        .copied()
        .find(|s| section_key(*s) == k)
        .unwrap_or(ReleaseSection::Albums)
}

/// Position in [`SECTION_ORDER`].
pub fn section_rank(s: ReleaseSection) -> i64 {
    SECTION_ORDER.iter().position(|x| *x == s).unwrap_or(0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_formats_and_subformats() {
        let none: [&str; 0] = [];
        assert_eq!(release_section(None, &none), ReleaseSection::Albums);
        assert_eq!(release_section(Some("Album"), &none), ReleaseSection::Albums);
        assert_eq!(release_section(Some("EP"), &none), ReleaseSection::SinglesEps);
        assert_eq!(release_section(Some("single"), &none), ReleaseSection::SinglesEps);
        assert_eq!(release_section(Some("Album"), &["Live"]), ReleaseSection::Live);
        assert_eq!(release_section(Some("Album"), &["Compilation"]), ReleaseSection::Compilations);
        assert_eq!(release_section(Some("Album"), &["Soundtrack"]), ReleaseSection::Soundtracks);
        assert_eq!(release_section(Some("Album"), &["Remix"]), ReleaseSection::Remixes);
        assert_eq!(release_section(Some("Album"), &["Demo"]), ReleaseSection::Demos);
        assert_eq!(release_section(Some("Album"), &["Mixtape/Street"]), ReleaseSection::Mixtapes);
        assert_eq!(release_section(Some("EP"), &["Live"]), ReleaseSection::Live);
        assert_eq!(release_section(None, &["Live", "Demo"]), ReleaseSection::Demos);
    }

    const NONE: [&str; 0] = [];
    const LABEL: Option<&str> = Some("Night Records");
    const MATCHED: Option<bool> = Some(true);
    const LOCAL: Option<bool> = Some(false);

    #[test]
    fn demos_are_unreleased() {
        assert!(!is_unreleased(Some("album"), &NONE, MATCHED, LABEL, None, None));
        assert!(!is_unreleased(Some("album"), &["live", "compilation"], MATCHED, None, None, None));
        assert!(is_unreleased(Some("album"), &["demo"], MATCHED, LABEL, None, None));
        assert!(is_unreleased(None, &["Live", "Demo"], MATCHED, LABEL, None, None));
        assert!(is_unreleased(Some("Demo"), &NONE, MATCHED, LABEL, None, None));
        assert_eq!(unreleased_reason(Some("Demo"), &NONE, MATCHED, LABEL, None, None), Some(Unreleased::Tagged));
    }

    #[test]
    fn archive_versions_join_the_unreleased_check() {
        for v in ["leak", "Demo", "snippet", "session", "build", "alternate", "unreleased"] {
            assert!(is_unreleased(Some("album"), &NONE, MATCHED, None, Some(v), None), "{v}");
            assert_eq!(unreleased_reason(None, &NONE, LOCAL, None, Some(v), None), Some(Unreleased::Tagged), "{v}");
        }
        for v in ["released", "remix", "live", ""] {
            assert!(!is_unreleased(Some("album"), &NONE, MATCHED, None, Some(v), None), "{v}");
        }
        // A demo release type stays unreleased whatever the files say.
        assert!(is_unreleased(Some("album"), &["demo"], MATCHED, None, Some("released"), None));
    }

    #[test]
    fn albums_plex_couldnt_match_are_probably_unreleased() {
        // local:// guid, nothing tagged.
        assert_eq!(unreleased_reason(None, &NONE, LOCAL, None, None, None), Some(Unreleased::Probably));
        assert_eq!(unreleased_reason(None, &NONE, LOCAL, LABEL, None, None), Some(Unreleased::Probably));
        assert!(is_unreleased(Some(" "), &[""], LOCAL, Some(""), Some(""), None));
        // Matched by Plex: released even with no type and no label.
        assert!(!is_unreleased(None, &NONE, MATCHED, None, None, None));
        // A release type means someone tagged it.
        assert!(!is_unreleased(Some("Album"), &NONE, LOCAL, None, None, None));
        assert!(!is_unreleased(None, &["Live"], LOCAL, None, None, None));
        // A real version tag wins over the guess.
        assert!(!is_unreleased(None, &NONE, LOCAL, None, Some("released"), None));
        assert!(is_unreleased(None, &["Demo"], LOCAL, None, Some("released"), None));
    }

    #[test]
    fn without_a_guid_no_type_and_no_label_stands_in() {
        // No format and no studio: Plex found no match.
        assert_eq!(unreleased_reason(None, &NONE, None, None, None, None), Some(Unreleased::Probably));
        // Format only, studio only: matched.
        assert!(!is_unreleased(Some("album"), &NONE, None, None, None, None));
        assert!(!is_unreleased(None, &NONE, None, LABEL, None, None));
        // Version tags still win.
        assert!(!is_unreleased(None, &NONE, None, None, Some("released"), None));
        assert!(is_unreleased(None, &NONE, None, None, Some("leak"), None));
        assert!(is_unreleased(None, &["demo"], None, LABEL, None, None));
    }

    #[test]
    fn the_setting_off_restores_the_tag_only_rule() {
        set_count_unmatched(false);
        assert!(!count_unmatched());
        assert!(!is_unreleased(None, &NONE, LOCAL, None, None, None));
        assert!(!is_unreleased(None, &NONE, None, None, None, None));
        assert!(is_unreleased(None, &["demo"], LOCAL, None, None, None));
        assert!(is_unreleased(None, &NONE, LOCAL, None, Some("leak"), None));
        set_count_unmatched(true);
        assert!(is_unreleased(None, &NONE, LOCAL, None, None, None));
    }

    #[test]
    fn musicbrainz_finding_the_release_means_plex_missed_it() {
        let (s, why) = release_status(None, &NONE, LOCAL, None, None, Some(true));
        assert_eq!(s, ReleaseStatus::PlexMissed);
        assert!(why.iter().any(|w| w.contains("MusicBrainz has an official release")));
        assert!(!is_unreleased(None, &NONE, LOCAL, None, None, Some(true)));
        // Still released with the unmatched guess off.
        set_count_unmatched(false);
        assert_eq!(release_status(None, &NONE, LOCAL, None, None, Some(true)).0, ReleaseStatus::PlexMissed);
        set_count_unmatched(true);
    }

    #[test]
    fn musicbrainz_finding_nothing_makes_the_guess_stronger() {
        let (s, why) = release_status(None, &NONE, LOCAL, None, None, Some(false));
        assert_eq!(s, ReleaseStatus::ProbablyUnreleased);
        assert_eq!(why.len(), 3);
        assert!(why[2].contains("MusicBrainz has no release"));
        let (_, unchecked) = release_status(None, &NONE, LOCAL, None, None, None);
        assert_eq!(unchecked.len(), 2);
        assert_eq!(unreleased_reason(None, &NONE, LOCAL, None, None, Some(false)), Some(Unreleased::Probably));
    }

    #[test]
    fn tags_beat_musicbrainz() {
        // A demo type or an unreleased version stays unreleased whatever MusicBrainz says.
        assert_eq!(release_status(None, &["Demo"], LOCAL, None, None, Some(true)).0, ReleaseStatus::TaggedUnreleased);
        assert_eq!(release_status(None, &NONE, LOCAL, None, Some("leak"), Some(true)).0, ReleaseStatus::TaggedUnreleased);
        assert_eq!(unreleased_reason(None, &NONE, LOCAL, None, Some("leak"), Some(true)), Some(Unreleased::Tagged));
        // A released tag stays plain released.
        assert_eq!(release_status(None, &NONE, LOCAL, None, Some("released"), Some(false)).0, ReleaseStatus::Released);
        assert_eq!(release_status(Some("Album"), &NONE, LOCAL, None, None, Some(false)).0, ReleaseStatus::Released);
        // Matched by Plex: MusicBrainz's answer doesn't matter.
        assert_eq!(release_status(None, &NONE, MATCHED, None, None, Some(false)).0, ReleaseStatus::Released);
        let (_, why) = release_status(None, &NONE, LOCAL, None, Some("leak"), None);
        assert_eq!(why, ["Its files are tagged as version \"leak\"."]);
    }

    #[test]
    fn the_musicbrainz_setting_off_ignores_its_answers() {
        set_use_musicbrainz(false);
        assert_eq!(release_status(None, &NONE, LOCAL, None, None, Some(true)).0, ReleaseStatus::ProbablyUnreleased);
        assert_eq!(release_status(None, &NONE, LOCAL, None, None, Some(false)).1.len(), 2);
        set_use_musicbrainz(true);
        assert!(use_musicbrainz());
        assert_eq!(release_status(None, &NONE, LOCAL, None, None, Some(true)).0, ReleaseStatus::PlexMissed);
    }

    #[test]
    fn keys_round_trip() {
        for s in SECTION_ORDER {
            assert_eq!(section_from_key(section_key(s)), s);
        }
    }
}
