//! Which old files a better copy may replace. A pure function over what
//! the old files and the new copy say about themselves.
//!
//! A file is replaced only when all of these hold:
//!
//! - it's released: not unreleased by the archive's rules, and no
//!   `ARCHIVE_*` tag in the file itself
//! - it's on a disc the official release has
//! - it and exactly one new file carry the same ISRC or the same
//!   MusicBrainz recording id, in their own tags (names and lengths are
//!   never enough: an alternate mix often runs the same length)
//! - the new file is better
//!
//! Everything else is kept as it is, with the reason.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use super::quality::Rank;
use crate::tags::FileTags;

/// One old file in the album's folders.
#[derive(Debug, Clone, PartialEq)]
pub struct OldSong {
    /// The library song, or empty for a file Plex doesn't list on this album.
    pub track_id: String,
    pub title: String,
    pub disc: u32,
    pub track: u32,
    pub plex_path: String,
    pub rank: Rank,
    pub codec: Option<String>,
    /// By the archive's rules (`db::release`).
    pub unreleased: bool,
    /// Read from the file itself.
    pub tags: FileTags,
}

/// One file of the better copy.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSong {
    pub lidarr_path: String,
    pub medium: i64,
    /// Lidarr's recording id for the track it matched the file to.
    pub lidarr_recording: String,
    pub rank: Rank,
    pub codec: String,
    pub tags: FileTags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Keep {
    Unreleased,
    ArchiveTags,
    ExtraDisc,
    /// A file in the album's folder that isn't one of the album's songs in Plex.
    NotInAlbum,
    /// No ISRC or recording id on one of the two files.
    NoId,
    /// Ids, but no new file has the same.
    NoMatch,
    /// More than one new file, or another old file, has the same id.
    Ambiguous,
    NotBetter,
}

impl Keep {
    pub fn reason(self) -> &'static str {
        match self {
            Keep::Unreleased => "Unreleased",
            Keep::ArchiveTags => "Has archive tags",
            Keep::ExtraDisc => "On a disc the official release doesn't have",
            Keep::NotInAlbum => "Not one of this album's songs in Plex",
            Keep::NoId => "Can't prove it's the same recording",
            Keep::NoMatch => "Not in the new copy",
            Keep::Ambiguous => "More than one possible match",
            Keep::NotBetter => "The new copy isn't better",
        }
    }

    /// Kept for a reason that makes the whole album ask file by file.
    pub fn asks_each(self) -> bool {
        !matches!(self, Keep::NotBetter)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Upgrade { new: usize, by: &'static str },
    Keep(Keep),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub old: usize,
    pub outcome: Outcome,
}

fn ids(t: &FileTags) -> (Option<String>, Option<String>) {
    (t.isrc.as_deref().and_then(crate::tags::clean_isrc), t.recording_mbid.as_deref().and_then(crate::tags::clean_mbid))
}

/// Whether two files carry the same recording by their own tags.
fn same(a: &FileTags, b: &FileTags) -> Option<&'static str> {
    let ((ai, am), (bi, bm)) = (ids(a), ids(b));
    if let (Some(x), Some(y)) = (&am, &bm) {
        // Two different recording ids are never the same recording.
        return (x == y).then_some("recordingId");
    }
    match (ai, bi) {
        (Some(x), Some(y)) if x == y => Some("isrc"),
        _ => None,
    }
}

/// A new file whose own recording id disagrees with the track Lidarr
/// matched it to can't be trusted.
fn trusted(n: &NewSong) -> bool {
    let lidarr = crate::tags::clean_mbid(&n.lidarr_recording);
    match (lidarr, ids(&n.tags).1) {
        (Some(l), Some(t)) => l == t,
        _ => true,
    }
}

/// One decision per old file, in order. `mediums` are the official
/// release's disc numbers.
pub fn decide(old: &[OldSong], new: &[NewSong], mediums: &BTreeSet<i64>) -> Vec<Decision> {
    // Candidate new files per old file.
    let mut cands: Vec<Vec<(usize, &'static str)>> = Vec::with_capacity(old.len());
    for o in old {
        let c: Vec<(usize, &'static str)> =
            new.iter().enumerate().filter(|(_, n)| trusted(n)).filter_map(|(i, n)| same(&o.tags, &n.tags).map(|by| (i, by))).collect();
        cands.push(c);
    }
    // How many eligible old files point at each new file.
    let mut claims: HashMap<usize, usize> = HashMap::new();
    for (o, c) in old.iter().zip(&cands) {
        if pre_keep(o, mediums).is_none()
            && let [(n, _)] = c.as_slice()
        {
            *claims.entry(*n).or_insert(0) += 1;
        }
    }
    old.iter()
        .enumerate()
        .map(|(i, o)| {
            let outcome = match pre_keep(o, mediums) {
                Some(k) => Outcome::Keep(k),
                None => {
                    let (oi, om) = ids(&o.tags);
                    match cands[i].as_slice() {
                        [] if oi.is_none() && om.is_none() => Outcome::Keep(Keep::NoId),
                        [] if new.iter().any(|n| trusted(n) && {
                            let (ni, nm) = ids(&n.tags);
                            (oi.is_some() && ni.is_some()) || (om.is_some() && nm.is_some())
                        }) => Outcome::Keep(Keep::NoMatch),
                        [] => Outcome::Keep(Keep::NoId),
                        [(n, by)] if claims.get(n).copied().unwrap_or(0) == 1 => {
                            if new[*n].rank > o.rank { Outcome::Upgrade { new: *n, by } } else { Outcome::Keep(Keep::NotBetter) }
                        }
                        _ => Outcome::Keep(Keep::Ambiguous),
                    }
                }
            };
            Decision { old: i, outcome }
        })
        .collect()
}

fn pre_keep(o: &OldSong, mediums: &BTreeSet<i64>) -> Option<Keep> {
    if o.track_id.is_empty() {
        return Some(Keep::NotInAlbum);
    }
    if o.unreleased {
        return Some(Keep::Unreleased);
    }
    if o.tags.has_archive() {
        return Some(Keep::ArchiveTags);
    }
    if !mediums.contains(&i64::from(o.disc.max(1))) {
        return Some(Keep::ExtraDisc);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const R1: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const R2: &str = "aaaaaaaa-0000-0000-0000-000000000002";
    const R3: &str = "aaaaaaaa-0000-0000-0000-000000000003";

    fn tags(isrc: Option<&str>, mbid: Option<&str>) -> FileTags {
        FileTags { isrc: isrc.map(str::to_string), recording_mbid: mbid.map(str::to_string), ..Default::default() }
    }

    fn old(id: &str, disc: u32, t: FileTags) -> OldSong {
        OldSong {
            track_id: id.into(),
            title: id.into(),
            disc,
            track: 1,
            plex_path: format!("/music/A/{id}.mp3"),
            rank: Rank(320),
            codec: Some("mp3".into()),
            unreleased: false,
            tags: t,
        }
    }

    fn new(medium: i64, rec: &str, t: FileTags) -> NewSong {
        NewSong { lidarr_path: format!("/up/A/{rec}.flac"), medium, lidarr_recording: rec.into(), rank: super::super::quality::CD, codec: "flac".into(), tags: t }
    }

    fn one() -> BTreeSet<i64> {
        BTreeSet::from([1])
    }

    fn outcome(o: &[OldSong], n: &[NewSong]) -> Vec<Outcome> {
        decide(o, n, &one()).into_iter().map(|d| d.outcome).collect()
    }

    #[test]
    fn isrc_or_recording_id_on_both_files_matches() {
        let n = [new(1, R1, tags(Some("USRC17607839"), Some(R1))), new(1, R2, tags(Some("GBAYE0601498"), None))];
        let o = [old("t1", 1, tags(None, Some(R1))), old("t2", 1, tags(Some("gb-aye-06-01498"), None))];
        assert_eq!(outcome(&o, &n), vec![Outcome::Upgrade { new: 0, by: "recordingId" }, Outcome::Upgrade { new: 1, by: "isrc" }]);
    }

    #[test]
    fn names_and_lengths_are_never_enough() {
        // Same title, same length, no ids: kept.
        let n = [new(1, R1, tags(None, None))];
        let o = [old("t1", 1, tags(None, None))];
        assert_eq!(outcome(&o, &n), vec![Outcome::Keep(Keep::NoId)]);
        // The old file has an id, the new one doesn't (only Lidarr's own match): kept.
        let n = [new(1, R1, tags(None, None))];
        let o = [old("t1", 1, tags(None, Some(R1)))];
        assert_eq!(outcome(&o, &n), vec![Outcome::Keep(Keep::NoId)]);
        assert_eq!(Keep::NoId.reason(), "Can't prove it's the same recording");
    }

    #[test]
    fn different_recording_ids_never_match_even_with_one_isrc() {
        let n = [new(1, R2, tags(Some("USRC17607839"), Some(R2)))];
        let o = [old("t1", 1, tags(Some("USRC17607839"), Some(R1)))];
        assert_eq!(outcome(&o, &n), vec![Outcome::Keep(Keep::NoMatch)]);
    }

    #[test]
    fn a_new_file_that_disagrees_with_lidarr_is_not_trusted() {
        let mut n = new(1, R1, tags(None, Some(R3)));
        n.lidarr_recording = R1.into();
        let o = [old("t1", 1, tags(None, Some(R3)))];
        assert_eq!(outcome(&o, &[n]), vec![Outcome::Keep(Keep::NoId)]);
    }

    #[test]
    fn unreleased_archive_tagged_extra_disc_and_stray_files_are_kept() {
        let n = [new(1, R1, tags(None, Some(R1)))];
        let mut unreleased = old("t1", 1, tags(None, Some(R1)));
        unreleased.unreleased = true;
        let mut archived = old("t2", 1, tags(None, Some(R1)));
        archived.tags.version = Some("released".into());
        let extra = old("t3", 2, tags(None, Some(R1)));
        let mut stray = old("", 1, tags(None, Some(R1)));
        stray.title = "leak.mp3".into();
        let got = outcome(&[unreleased, archived, extra, stray], &n);
        assert_eq!(
            got,
            vec![Outcome::Keep(Keep::Unreleased), Outcome::Keep(Keep::ArchiveTags), Outcome::Keep(Keep::ExtraDisc), Outcome::Keep(Keep::NotInAlbum)]
        );
        // Any ARCHIVE_* tag counts, even one that says released.
        assert!(FileTags { era: Some("2019".into()), ..Default::default() }.has_archive());
        assert!(!FileTags::default().has_archive());
        assert!(Keep::ExtraDisc.asks_each() && !Keep::NotBetter.asks_each());
    }

    #[test]
    fn two_claims_on_one_file_or_two_files_for_one_are_ambiguous() {
        let n = [new(1, R1, tags(None, Some(R1)))];
        let o = [old("t1", 1, tags(None, Some(R1))), old("t2", 1, tags(None, Some(R1)))];
        assert_eq!(outcome(&o, &n), vec![Outcome::Keep(Keep::Ambiguous), Outcome::Keep(Keep::Ambiguous)]);
        let n = [new(1, R1, tags(Some("USRC17607839"), None)), new(1, R2, tags(Some("USRC17607839"), None))];
        let o = [old("t1", 1, tags(Some("USRC17607839"), None))];
        assert_eq!(outcome(&o, &n), vec![Outcome::Keep(Keep::Ambiguous)]);
    }

    #[test]
    fn only_a_better_file_replaces() {
        let mut n = new(1, R1, tags(None, Some(R1)));
        n.rank = Rank(256);
        let o = [old("t1", 1, tags(None, Some(R1)))];
        assert_eq!(outcome(&o, &[n]), vec![Outcome::Keep(Keep::NotBetter)]);
    }
}
