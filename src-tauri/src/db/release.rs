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
pub fn is_unreleased<S: AsRef<str>>(
    format: Option<&str>,
    subformats: &[S],
    matched: Option<bool>,
    studio: Option<&str>,
    archive_version: Option<&str>,
) -> bool {
    unreleased_reason(format, subformats, matched, studio, archive_version).is_some()
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
) -> Option<Unreleased> {
    let demo = |s: &str| s.to_ascii_lowercase().contains("demo");
    let blank = |s: Option<&str>| s.is_none_or(|s| s.trim().is_empty());
    if format.is_some_and(demo) || subformats.iter().any(|s| demo(s.as_ref())) {
        return Some(Unreleased::Tagged);
    }
    if !blank(archive_version) {
        return archive_version.is_some_and(version_is_unreleased).then_some(Unreleased::Tagged);
    }
    let typed = !blank(format) || subformats.iter().any(|s| !s.as_ref().trim().is_empty());
    if typed || !count_unmatched() {
        return None;
    }
    let unmatched = match matched {
        Some(m) => !m,
        None => blank(studio),
    };
    unmatched.then_some(Unreleased::Probably)
}

// The "count albums Plex couldn't match as unreleased" setting. Tests keep
// it per thread so they can't see each other's changes.
#[cfg(not(test))]
static COUNT_UNMATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[cfg(test)]
thread_local! {
    static COUNT_UNMATCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
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
        assert!(!is_unreleased(Some("album"), &NONE, MATCHED, LABEL, None));
        assert!(!is_unreleased(Some("album"), &["live", "compilation"], MATCHED, None, None));
        assert!(is_unreleased(Some("album"), &["demo"], MATCHED, LABEL, None));
        assert!(is_unreleased(None, &["Live", "Demo"], MATCHED, LABEL, None));
        assert!(is_unreleased(Some("Demo"), &NONE, MATCHED, LABEL, None));
        assert_eq!(unreleased_reason(Some("Demo"), &NONE, MATCHED, LABEL, None), Some(Unreleased::Tagged));
    }

    #[test]
    fn archive_versions_join_the_unreleased_check() {
        for v in ["leak", "Demo", "snippet", "session", "build", "alternate", "unreleased"] {
            assert!(is_unreleased(Some("album"), &NONE, MATCHED, None, Some(v)), "{v}");
            assert_eq!(unreleased_reason(None, &NONE, LOCAL, None, Some(v)), Some(Unreleased::Tagged), "{v}");
        }
        for v in ["released", "remix", "live", ""] {
            assert!(!is_unreleased(Some("album"), &NONE, MATCHED, None, Some(v)), "{v}");
        }
        // A demo release type stays unreleased whatever the files say.
        assert!(is_unreleased(Some("album"), &["demo"], MATCHED, None, Some("released")));
    }

    #[test]
    fn albums_plex_couldnt_match_are_probably_unreleased() {
        // local:// guid, nothing tagged.
        assert_eq!(unreleased_reason(None, &NONE, LOCAL, None, None), Some(Unreleased::Probably));
        assert_eq!(unreleased_reason(None, &NONE, LOCAL, LABEL, None), Some(Unreleased::Probably));
        assert!(is_unreleased(Some(" "), &[""], LOCAL, Some(""), Some("")));
        // Matched by Plex: released even with no type and no label.
        assert!(!is_unreleased(None, &NONE, MATCHED, None, None));
        // A release type means someone tagged it.
        assert!(!is_unreleased(Some("Album"), &NONE, LOCAL, None, None));
        assert!(!is_unreleased(None, &["Live"], LOCAL, None, None));
        // A real version tag wins over the guess.
        assert!(!is_unreleased(None, &NONE, LOCAL, None, Some("released")));
        assert!(is_unreleased(None, &["Demo"], LOCAL, None, Some("released")));
    }

    #[test]
    fn without_a_guid_no_type_and_no_label_stands_in() {
        // No format and no studio: Plex found no match.
        assert_eq!(unreleased_reason(None, &NONE, None, None, None), Some(Unreleased::Probably));
        // Format only, studio only: matched.
        assert!(!is_unreleased(Some("album"), &NONE, None, None, None));
        assert!(!is_unreleased(None, &NONE, None, LABEL, None));
        // Version tags still win.
        assert!(!is_unreleased(None, &NONE, None, None, Some("released")));
        assert!(is_unreleased(None, &NONE, None, None, Some("leak")));
        assert!(is_unreleased(None, &["demo"], None, LABEL, None));
    }

    #[test]
    fn the_setting_off_restores_the_tag_only_rule() {
        set_count_unmatched(false);
        assert!(!count_unmatched());
        assert!(!is_unreleased(None, &NONE, LOCAL, None, None));
        assert!(!is_unreleased(None, &NONE, None, None, None));
        assert!(is_unreleased(None, &["demo"], LOCAL, None, None));
        assert!(is_unreleased(None, &NONE, LOCAL, None, Some("leak")));
        set_count_unmatched(true);
        assert!(is_unreleased(None, &NONE, LOCAL, None, None));
    }

    #[test]
    fn keys_round_trip() {
        for s in SECTION_ORDER {
            assert_eq!(section_from_key(section_key(s)), s);
        }
    }
}
