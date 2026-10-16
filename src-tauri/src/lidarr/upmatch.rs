//! Which old files a better copy may replace. A pure function over what
//! the old files, the new copy and the confirmed release say.
//!
//! A file is replaced only when it's released (no unreleased status from
//! its tags or the archive's marks), it's on the confirmed release's
//! tracklist, the new file is better, and either:
//!
//! - it and the new file carry the same ISRC, or
//! - all of: a tag says it's the released recording, its provenance note
//!   says it came from an official source, the lengths are within a
//!   second, and the decoded audio correlates at [`MIN_CORRELATION`] or
//!   more (a censored edit can score above that, so it's never enough alone).
//!
//! Everything else is kept as it is, with the reason.

use std::collections::HashMap;

use serde::Serialize;

use super::provenance::{FileStatus, ReleaseStatus, note_is_official};
use super::quality::Rank;
use crate::identity::fold::fold_title;
use crate::tags::FileTags;

pub const MIN_CORRELATION: f64 = 0.99;
pub const MAX_LENGTH_DIFF_MS: u64 = 1_000;

/// One old file in the album's folders.
#[derive(Debug, Clone, PartialEq)]
pub struct OldSong {
    /// The library song, or empty for a file Plex doesn't list on this album.
    pub track_id: String,
    pub title: String,
    pub disc: u32,
    pub track: u32,
    pub duration_ms: u64,
    pub plex_path: String,
    pub rank: Rank,
    pub codec: Option<String>,
    /// By the archive's marks (`db::release`).
    pub unreleased: bool,
    /// Read from the file itself.
    pub tags: FileTags,
    pub status: FileStatus,
}

/// One track of the confirmed release, as Lidarr has it.
#[derive(Debug, Clone, PartialEq)]
pub struct OfficialTrack {
    pub title: String,
    pub recording: String,
    pub medium: i64,
    pub duration_ms: u64,
    /// Its file in the new copy.
    pub new: Option<usize>,
}

/// One file of the better copy.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSong {
    pub lidarr_path: String,
    pub duration_ms: u64,
    pub rank: Rank,
    pub codec: String,
    pub tags: FileTags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Keep {
    Unreleased,
    /// A file in the album's folder that isn't one of the album's songs in Plex.
    NotInAlbum,
    /// Not on the confirmed release's tracklist (a bonus track, an extra disc).
    NotOnRelease,
    /// The release has the song but the new copy has no file for it.
    NotInNewCopy,
    /// No ISRC match, and no tag says it's the released recording.
    NoReleasedTag,
    /// No ISRC match, and its provenance note doesn't say official.
    NoOfficialNote,
    LengthDiffers,
    /// Its length differs from the release's track by more than a second: another version.
    DifferentVersion,
    SoundDiffers,
    /// More than one file could be the same song.
    Ambiguous,
    NotBetter,
}

impl Keep {
    pub fn reason(self) -> &'static str {
        match self {
            Keep::Unreleased => "Unreleased",
            Keep::NotInAlbum => "Not one of this album's songs in Plex",
            Keep::NotOnRelease => "Not on the confirmed release",
            Keep::NotInNewCopy => "Not in the new copy",
            Keep::NoReleasedTag => "Can't prove it's the same recording: no tag says it's the released one",
            Keep::NoOfficialNote => "Can't prove it's the same recording: its source note doesn't say official",
            Keep::LengthDiffers => "Can't prove it's the same recording: the lengths differ",
            Keep::DifferentVersion => "A different version: its length differs from the release's",
            Keep::SoundDiffers => "Can't prove it's the same recording: it doesn't sound the same",
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

fn isrc(t: &FileTags) -> Option<String> {
    t.isrc.as_deref().and_then(crate::tags::clean_isrc)
}

fn mbid(t: &FileTags) -> Option<String> {
    t.recording_mbid.as_deref().and_then(crate::tags::clean_mbid)
}

fn title_key(t: &str) -> String {
    let f = fold_title(t);
    format!("{}\u{1f}{}", f.title, f.version)
}

/// The tracklist entry an old file is, by recording id, else by its title
/// when exactly one track has it.
fn on_release(o: &OldSong, tracks: &[OfficialTrack]) -> Option<usize> {
    if let Some(m) = mbid(&o.tags)
        && let Some(i) = tracks.iter().position(|t| crate::tags::clean_mbid(&t.recording).as_deref() == Some(m.as_str()))
    {
        return Some(i);
    }
    let key = title_key(&o.title);
    let hits: Vec<usize> = tracks.iter().enumerate().filter(|(_, t)| title_key(&t.title) == key).map(|(i, _)| i).collect();
    match hits.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// What one old file would match, before checking others.
fn candidate(o: &OldSong, tracks: &[OfficialTrack], new: &[NewSong], prefix: &str, corr: &mut dyn FnMut(usize) -> Option<f64>) -> Result<(usize, &'static str), Keep> {
    if o.track_id.is_empty() {
        return Err(Keep::NotInAlbum);
    }
    if o.unreleased || o.status.status == ReleaseStatus::Unreleased {
        return Err(Keep::Unreleased);
    }
    // ISRC on both files decides.
    if let Some(i) = isrc(&o.tags) {
        let same: Vec<usize> = new.iter().enumerate().filter(|(_, n)| isrc(&n.tags).as_deref() == Some(i.as_str())).map(|(k, _)| k).collect();
        match same.as_slice() {
            [one] => {
                let Some(t) = tracks.iter().find(|t| t.new == Some(*one)) else { return Err(Keep::NotOnRelease) };
                return if differs(o.duration_ms, t.duration_ms) { Err(Keep::DifferentVersion) } else { Ok((*one, "isrc")) };
            }
            [] => {}
            _ => return Err(Keep::Ambiguous),
        }
    }
    let Some(t) = on_release(o, tracks) else { return Err(Keep::NotOnRelease) };
    if differs(o.duration_ms, tracks[t].duration_ms) {
        return Err(Keep::DifferentVersion);
    }
    let Some(n) = tracks[t].new else { return Err(Keep::NotInNewCopy) };
    if o.status.status != ReleaseStatus::Released {
        return Err(Keep::NoReleasedTag);
    }
    if !note_is_official(o.status.note.as_deref(), prefix) {
        return Err(Keep::NoOfficialNote);
    }
    let (a, b) = (o.duration_ms, new[n].duration_ms);
    if a == 0 || b == 0 || a.abs_diff(b) > MAX_LENGTH_DIFF_MS {
        return Err(Keep::LengthDiffers);
    }
    match corr(n) {
        Some(c) if c >= MIN_CORRELATION => Ok((n, "sound")),
        _ => Err(Keep::SoundDiffers),
    }
}

/// Both lengths known and more than a second apart.
fn differs(a: u64, b: u64) -> bool {
    a > 0 && b > 0 && a.abs_diff(b) > MAX_LENGTH_DIFF_MS
}

/// One decision per old file, in order. `corr(old, new)` decodes and
/// correlates the two files; it's only asked when every other key holds.
pub fn decide(old: &[OldSong], tracks: &[OfficialTrack], new: &[NewSong], prefix: &str, corr: &mut dyn FnMut(usize, usize) -> Option<f64>) -> Vec<Decision> {
    let cands: Vec<Result<(usize, &'static str), Keep>> =
        old.iter().enumerate().map(|(i, o)| candidate(o, tracks, new, prefix, &mut |n| corr(i, n))).collect();
    let mut claims: HashMap<usize, usize> = HashMap::new();
    for (n, _) in cands.iter().flatten() {
        *claims.entry(*n).or_insert(0) += 1;
    }
    cands
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let outcome = match c {
                Err(k) => Outcome::Keep(k),
                Ok((n, _)) if claims.get(&n).copied().unwrap_or(0) > 1 => Outcome::Keep(Keep::Ambiguous),
                Ok((n, _)) if new[n].rank <= old[i].rank => Outcome::Keep(Keep::NotBetter),
                Ok((n, by)) => Outcome::Upgrade { new: n, by },
            };
            Decision { old: i, outcome }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::provenance::NoteTag;
    use super::super::quality::CD;
    use super::*;

    const R1: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const R2: &str = "aaaaaaaa-0000-0000-0000-000000000002";

    fn status(s: ReleaseStatus, note: Option<&str>) -> FileStatus {
        FileStatus { status: s, note: note.map(str::to_string), note_tag: NoteTag::ArchiveSource }
    }

    fn released() -> FileStatus {
        status(ReleaseStatus::Released, Some("Official CD"))
    }

    fn old(id: &str, title: &str, isrc: Option<&str>, st: FileStatus) -> OldSong {
        OldSong {
            track_id: id.into(),
            title: title.into(),
            disc: 1,
            track: 1,
            duration_ms: 200_000,
            plex_path: format!("/music/A/{title}.mp3"),
            rank: Rank(320),
            codec: Some("mp3".into()),
            unreleased: false,
            tags: FileTags { isrc: isrc.map(str::to_string), ..Default::default() },
            status: st,
        }
    }

    fn new(isrc: Option<&str>) -> NewSong {
        NewSong { lidarr_path: "/up/x.flac".into(), duration_ms: 200_400, rank: CD, codec: "flac".into(), tags: FileTags { isrc: isrc.map(str::to_string), ..Default::default() } }
    }

    fn track(title: &str, rec: &str, new: Option<usize>) -> OfficialTrack {
        OfficialTrack { title: title.into(), recording: rec.into(), medium: 1, duration_ms: 200_000, new }
    }

    fn run(o: &[OldSong], t: &[OfficialTrack], n: &[NewSong], c: f64) -> Vec<Outcome> {
        decide(o, t, n, "Official", &mut |_, _| Some(c)).into_iter().map(|d| d.outcome).collect()
    }

    #[test]
    fn an_isrc_on_both_files_is_enough() {
        let t = [track("Dawn", R1, Some(0))];
        let o = [old("t1", "Dawn", Some("US-RC1-76-07839"), status(ReleaseStatus::Unknown, None))];
        assert_eq!(run(&o, &t, &[new(Some("USRC17607839"))], 0.0), vec![Outcome::Upgrade { new: 0, by: "isrc" }]);
    }

    #[test]
    fn without_an_isrc_every_key_must_hold() {
        let t = [track("Dawn", R1, Some(0))];
        let n = [new(None)];
        let ok = old("t1", "Dawn", None, released());
        assert_eq!(run(std::slice::from_ref(&ok), &t, &n, 0.995), vec![Outcome::Upgrade { new: 0, by: "sound" }]);
        // A censored edit scores 0.9907: it passes the sound key, but the other keys still have to hold.
        assert_eq!(run(&[old("t1", "Dawn", None, status(ReleaseStatus::Unknown, Some("Official CD")))], &t, &n, 0.9907), vec![Outcome::Keep(Keep::NoReleasedTag)]);
        assert_eq!(run(&[old("t1", "Dawn", None, status(ReleaseStatus::Released, Some("Web rip")))], &t, &n, 0.995), vec![Outcome::Keep(Keep::NoOfficialNote)]);
        assert_eq!(run(std::slice::from_ref(&ok), &t, &n, 0.98), vec![Outcome::Keep(Keep::SoundDiffers)]);
        // The old file is off the release's length: another version.
        let mut long = ok.clone();
        long.duration_ms = 201_500;
        assert_eq!(run(&[long], &t, &n, 0.999), vec![Outcome::Keep(Keep::DifferentVersion)]);
        // The new file is off the old one's length.
        let mut off = new(None);
        off.duration_ms = 198_500;
        assert_eq!(run(std::slice::from_ref(&ok), &t, &[off], 0.999), vec![Outcome::Keep(Keep::LengthDiffers)]);
        // The release has the song, the new copy doesn't.
        assert_eq!(run(std::slice::from_ref(&ok), &[track("Dawn", R1, None)], &n, 0.999), vec![Outcome::Keep(Keep::NotInNewCopy)]);
    }

    #[test]
    fn the_confirmed_tracklist_decides_what_is_extra_not_the_disc_number() {
        let t = [track("Dawn", R1, Some(0))];
        let mut bonus = old("t2", "Dawn (Demo)", None, released());
        bonus.disc = 1;
        assert_eq!(run(&[bonus], &t, &[new(None)], 0.999), vec![Outcome::Keep(Keep::NotOnRelease)]);
        // Disc 2 in the folder, but on the release: fine.
        let mut d2 = old("t3", "Dawn", None, released());
        d2.disc = 2;
        assert_eq!(run(&[d2], &t, &[new(None)], 0.999), vec![Outcome::Upgrade { new: 0, by: "sound" }]);
        // An ISRC that points at a file outside the release.
        let o = [old("t1", "Dawn", Some("USRC17607839"), released())];
        assert_eq!(run(&o, &[track("Dawn", R1, None)], &[new(Some("USRC17607839"))], 0.0), vec![Outcome::Keep(Keep::NotOnRelease)]);
    }

    #[test]
    fn unreleased_stray_ambiguous_and_worse_files_are_kept() {
        let t = [track("Dawn", R1, Some(0)), track("Noon", R2, Some(1))];
        let n = [new(None), new(None)];
        let mut by_mark = old("t1", "Dawn", None, released());
        by_mark.unreleased = true;
        let by_tag = old("t2", "Noon", None, status(ReleaseStatus::Unreleased, Some("Official CD")));
        let stray = old("", "leak.mp3", None, released());
        assert_eq!(
            run(&[by_mark, by_tag, stray], &t, &n, 0.999),
            vec![Outcome::Keep(Keep::Unreleased), Outcome::Keep(Keep::Unreleased), Outcome::Keep(Keep::NotInAlbum)]
        );
        // Two files claim one track.
        let two = [old("t1", "Dawn", None, released()), old("t9", "Dawn", None, released())];
        assert_eq!(run(&two, &t, &n, 0.999), vec![Outcome::Keep(Keep::Ambiguous), Outcome::Keep(Keep::Ambiguous)]);
        // A new copy that isn't better.
        let mut worse = new(Some("USRC17607839"));
        worse.rank = Rank(256);
        let o = [old("t1", "Dawn", Some("USRC17607839"), released())];
        assert_eq!(run(&o, &[track("Dawn", R1, Some(0))], &[worse], 0.0), vec![Outcome::Keep(Keep::NotBetter)]);
        assert!(Keep::NotOnRelease.asks_each() && !Keep::NotBetter.asks_each());
    }

    #[test]
    fn a_length_off_the_release_is_a_different_version_even_with_an_isrc() {
        let mut t = track("Dawn", R1, Some(0));
        t.duration_ms = 215_000; // the release's edit runs 15 s longer
        let o = [old("t1", "Dawn", Some("USRC17607839"), released())];
        assert_eq!(run(&o, std::slice::from_ref(&t), &[new(Some("USRC17607839"))], 0.999), vec![Outcome::Keep(Keep::DifferentVersion)]);
        let o = [old("t1", "Dawn", None, released())];
        assert_eq!(run(&o, &[t], &[new(None)], 0.999), vec![Outcome::Keep(Keep::DifferentVersion)]);
    }

    #[test]
    fn the_sound_is_only_compared_when_everything_else_holds() {
        let t = [track("Dawn", R1, Some(0))];
        let mut asked = 0;
        let o = [old("t1", "Dawn", None, status(ReleaseStatus::Unknown, None))];
        decide(&o, &t, &[new(None)], "Official", &mut |_, _| {
            asked += 1;
            Some(1.0)
        });
        assert_eq!(asked, 0);
    }
}
