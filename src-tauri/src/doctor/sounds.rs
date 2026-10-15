//! Untagged or mislabelled files: songs identified by sound (`soundid::`)
//! as a recording other than their tags say. The fix shows the tags the
//! recording suggests next to the file's own; nothing is written.
//!
//! Unreleased songs never get this finding. Sounding like a released song
//! is expected of a demo or a leak, and the app never suggests renaming one
//! as the release: they get a "sounds like" hint instead.

use rusqlite::Connection;

use super::{Finding, Fix, Kind, SuggestedTags};
use crate::error::AppResult;
use crate::identity::fold::{fold_artist, fold_title};
use crate::soundid::store::{Identified, identified};

/// Whether the file's names disagree with the recording it sounds like.
fn mislabelled(s: &Identified) -> bool {
    let rec = &s.sounds;
    if s.tag_recording_mbid.as_deref().is_some_and(|m| m != rec.recording_mbid) {
        return true;
    }
    if s.tag_recording_mbid.as_deref() == Some(rec.recording_mbid.as_str()) {
        return false;
    }
    let title_same = fold_title(&s.title).title == fold_title(&rec.title).title;
    let want = fold_artist(&rec.artist);
    let artist_same = [Some(s.artist.as_str()), s.track_artist.as_deref()].into_iter().flatten().any(|a| {
        let a = fold_artist(a);
        !a.is_empty() && (a == want || want.contains(&a) || a.contains(&want))
    });
    !(title_same && artist_same)
}

/// One finding per identified, released song whose tags don't match what
/// it sounds like.
pub fn findings(c: &Connection) -> AppResult<Vec<Finding>> {
    let mut out = Vec::new();
    for s in identified(c)? {
        if s.unreleased || !mislabelled(&s) {
            continue;
        }
        let rec = &s.sounds;
        let artist_now = s.track_artist.clone().unwrap_or_else(|| s.artist.clone());
        let by = if rec.artist.is_empty() { String::new() } else { format!(" by {}", rec.artist) };
        let mut evidence = vec![
            format!(
                "Its audio fingerprint matched the MusicBrainz recording {}{by} ({}% match, id {}).",
                rec.title, rec.score, rec.recording_mbid
            ),
            format!("The file is tagged as {} by {artist_now}.", s.title),
        ];
        if let Some(m) = &s.tag_recording_mbid {
            evidence.push(format!("Its tags name another MusicBrainz recording ({m})."));
        }
        evidence.push("Only the fingerprint and the file's length were sent to AcoustID to find this.".into());
        out.push(Finding {
            id: format!("soundsLike:{}:{}", s.track_id, rec.recording_mbid),
            kind: Kind::SoundsLike,
            artist_id: s.artist_id.clone(),
            artist: s.artist.clone(),
            album_id: s.album_id.clone(),
            album: s.album.clone(),
            thumb: s.thumb.clone(),
            title: format!("Untagged or mislabelled file, sounds like {}{by}", rec.title),
            text: format!(
                "{} sounds like {}{by}. See the tags that recording suggests next to the file's own; nothing is changed, so a tagger is where to fix them.",
                s.title, rec.title
            ),
            evidence,
            fix: Fix::SuggestedTags,
            tracks: vec![s.track_id.clone()],
            others: Vec::new(),
            table: Vec::new(),
            suggest: Some(SuggestedTags {
                track_id: s.track_id.clone(),
                title: s.title.clone(),
                artist: artist_now,
                album: s.album.clone(),
                new_title: rec.title.clone(),
                new_artist: rec.artist.clone(),
                new_album: rec.album.clone(),
                recording_mbid: rec.recording_mbid.clone(),
                score: rec.score,
            }),
        });
    }
    Ok(out)
}
