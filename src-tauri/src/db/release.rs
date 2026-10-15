//! Release type → artist-page section (docs/PLEX-API.md, docs/TAGGING.md).
//!
//! Plex exposes the RELEASETYPE tag as album `Format` ("Album", "EP",
//! "Single") and `Subformat` ("Live", "Compilation", "Soundtrack", "Remix",
//! "Demo", "Mixtape/Street"). Untagged albums have no Format → Albums.

use crate::model::ReleaseSection;

/// Fixed display order of sections on an artist page.
pub const SECTION_ORDER: [ReleaseSection; 8] = [
    ReleaseSection::Albums,
    ReleaseSection::SinglesEps,
    ReleaseSection::Live,
    ReleaseSection::Compilations,
    ReleaseSection::Soundtracks,
    ReleaseSection::Remixes,
    ReleaseSection::Demos,
    ReleaseSection::Mixtapes,
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
/// subformat), or an ARCHIVE_VERSION read from the files that isn't a
/// released one (see [`version_is_unreleased`]). For an album, pass the
/// version most of its files carry.
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
        ReleaseStatus::TaggedUnreleased => Some(Unreleased::Tagged),
        ReleaseStatus::ProbablyUnreleased => Some(Unreleased::Probably),
        ReleaseStatus::PlexMissed | ReleaseStatus::Released => None,
    }
}

/// An album's release status as the album page shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseStatus {
    /// A demo release type or an unreleased ARCHIVE_VERSION.
    TaggedUnreleased,
    /// Plex couldn't match it and nothing is tagged.
    ProbablyUnreleased,
    /// Plex couldn't match it, but MusicBrainz has the release.
    PlexMissed,
    Released,
}

/// [`unreleased_reason`] with the evidence behind it, as plain sentences
/// for the album page's "Why?".
pub fn release_status<S: AsRef<str>>(
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
    mb_found: Option<bool>,
) -> (ReleaseStatus, Vec<String>) {
    let demo = |s: &str| s.to_ascii_lowercase().contains("demo");
    let blank = |s: Option<&str>| s.is_none_or(|s| s.trim().is_empty());
    if format.is_some_and(demo) || subformats.iter().any(|s| demo(s.as_ref())) {
        return (ReleaseStatus::TaggedUnreleased, vec!["Its release type is tagged as a demo.".into()]);
    }
    if let Some(v) = archive_version.map(str::trim).filter(|v| !v.is_empty()) {
        let why = vec![format!("Its files are tagged as version \"{v}\".")];
        return if version_is_unreleased(v) { (ReleaseStatus::TaggedUnreleased, why) } else { (ReleaseStatus::Released, why) };
    }
    let typed = !blank(format) || subformats.iter().any(|s| !s.as_ref().trim().is_empty());
    if typed {
        return (ReleaseStatus::Released, Vec::new());
    }
    let unmatched = match matched {
        Some(m) => !m,
        None => blank(studio),
    };
    if !unmatched {
        return (ReleaseStatus::Released, Vec::new());
    }
    let mut why = vec![
        "Plex couldn't match it to a known release.".to_string(),
        "Nothing in its tags says what kind of release it is.".to_string(),
    ];
    let mb = if use_musicbrainz() { mb_found } else { None };
    if mb == Some(true) {
        why.push("MusicBrainz has an official release with the same artist, title and about as many songs.".into());
        return (ReleaseStatus::PlexMissed, why);
    }
    if !count_unmatched() {
        return (ReleaseStatus::Released, Vec::new());
    }
    if mb == Some(false) {
        why.push("MusicBrainz has no release by this artist with this title.".into());
    }
    (ReleaseStatus::ProbablyUnreleased, why)
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
/// ARCHIVE_VERSION values that mean the music was never released. Remix and
/// live versions can be released ones, so they don't count on their own.
pub fn version_is_unreleased(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "unreleased" | "leak" | "demo" | "snippet" | "session" | "build" | "alternate"
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
