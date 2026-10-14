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
pub fn is_unreleased<S: AsRef<str>>(format: Option<&str>, subformats: &[S], archive_version: Option<&str>) -> bool {
    let demo = |s: &str| s.to_ascii_lowercase().contains("demo");
    format.is_some_and(demo)
        || subformats.iter().any(|s| demo(s.as_ref()))
        || archive_version.is_some_and(version_is_unreleased)
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

    #[test]
    fn demos_are_unreleased() {
        let none: [&str; 0] = [];
        assert!(!is_unreleased(None, &none, None));
        assert!(!is_unreleased(Some("album"), &["live", "compilation"], None));
        assert!(is_unreleased(Some("album"), &["demo"], None));
        assert!(is_unreleased(None, &["Live", "Demo"], None));
        assert!(is_unreleased(Some("Demo"), &none, None));
    }

    #[test]
    fn archive_versions_join_the_unreleased_check() {
        let none: [&str; 0] = [];
        for v in ["leak", "Demo", "snippet", "session", "build", "alternate", "unreleased"] {
            assert!(is_unreleased(Some("album"), &none, Some(v)), "{v}");
        }
        for v in ["released", "remix", "live", ""] {
            assert!(!is_unreleased(Some("album"), &none, Some(v)), "{v}");
        }
        // A demo release type stays unreleased whatever the files say.
        assert!(is_unreleased(Some("album"), &["demo"], Some("released")));
    }

    #[test]
    fn keys_round_trip() {
        for s in SECTION_ORDER {
            assert_eq!(section_from_key(section_key(s)), s);
        }
    }
}
