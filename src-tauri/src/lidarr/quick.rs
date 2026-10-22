//! The quick way to get a song someone tapped: Lidarr's own search for the
//! album's releases (one search of the indexers), the fastest good one
//! picked here, and that one sent to the download client.

use super::api::{QualityProfile, Release};
use super::quality::{Rank, cutoff_rank};
use super::request::{RequestRecord, next_search};
use super::status::RequestState;
use super::write::GrabRelease;
use super::{Checked, Lidarr};
use crate::error::AppResult;

/// What the user sees when the indexers have nothing usable.
pub const NO_COPY: &str = "Your indexers have no copy of this yet. Platter will keep looking and tell you.";

/// Whether Lidarr would take a release at all: approved against the
/// profile, nothing rejected it, the indexer can hand it over, and it's
/// this album rather than a whole discography. A torrent nobody seeds never
/// finishes, so it's left out too.
fn usable(r: &Release) -> bool {
    r.approved && !r.rejected && r.download_allowed && !r.discography && !(is_torrent(r) && r.seeders == Some(0)) && !one_file_image(&r.title, &[&r.album_title, &r.artist_name])
}

/// A whole CD as one file, by the release's name ("image", "img"). Lidarr
/// approves these and then can't import them. A cue sheet alone says
/// nothing (split tracks often come with one), and words in the album's or
/// artist's own name don't count. Most such releases don't say so; those
/// are cleared after the import fails.
pub fn one_file_image(title: &str, names: &[&str]) -> bool {
    let words = |s: &str| s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect::<Vec<_>>();
    let own: Vec<String> = names.iter().flat_map(|n| words(n)).collect();
    words(title).iter().any(|w| ["image", "img", "imgcue", "cueimg"].contains(&w.as_str()) && !own.contains(w))
}

fn is_torrent(r: &Release) -> bool {
    r.protocol.eq_ignore_ascii_case("torrent")
}

/// The release to grab. Lidarr sends its list best first. With `fastest`,
/// the ones at the profile's cutoff quality or better come first when there
/// are any, and among those the one that finishes soonest: usenet before
/// torrents, torrents with more seeders before thin ones, Lidarr's order
/// after that. Without it, Lidarr's own first choice.
pub fn pick<'a>(releases: &'a [Release], profile: Option<&QualityProfile>, fastest: bool) -> Option<&'a Release> {
    let ok: Vec<(usize, &Release)> = releases.iter().enumerate().filter(|(_, r)| usable(r)).collect();
    if !fastest {
        return ok.first().map(|(_, r)| *r);
    }
    let cutoff = profile.and_then(cutoff_rank);
    let good = |r: &Release| cutoff.is_some_and(|c| Rank::of_lidarr(r.quality_name()) >= c);
    let mut pool: Vec<(usize, &Release)> = ok.iter().copied().filter(|(_, r)| good(r)).collect();
    if pool.is_empty() {
        pool = ok;
    }
    pool.into_iter().min_by_key(|(i, r)| (is_torrent(r), std::cmp::Reverse(if is_torrent(r) { r.seeders.unwrap_or(0) } else { 0 }), *i)).map(|(_, r)| r)
}

/// Why nothing could be grabbed, in a few words, when the indexers had
/// copies Lidarr turned down.
fn turned_down(releases: &[Release]) -> Option<String> {
    let why = releases.iter().filter(|r| !usable(r)).flat_map(|r| r.rejections.iter()).find(|w| !w.trim().is_empty())?;
    Some(format!("Every copy your indexers have was turned down (Lidarr said: {}).", why.trim().trim_end_matches('.')))
}

impl Lidarr {
    /// After the album is monitored and the check passed again: one search
    /// of the indexers, and the fastest good release grabbed.
    pub(super) async fn grab_step(&self, c: &Checked, mut rec: RequestRecord, album_id: i64, artist_id: i64, now: i64) -> AppResult<RequestRecord> {
        let started = tokio::time::Instant::now();
        let releases = c.client.releases(album_id).await?;
        let fastest = crate::settings::load(&self.inner.db).lidarr_prefer_fastest;
        let profile = c.snap.artists.iter().find(|a| a.id == artist_id).and_then(|a| c.snap.profiles.iter().find(|p| Some(p.id) == a.quality_profile_id));
        // The search went out: anything Lidarr does from here (a later search
        // on the usual schedule) counts from now.
        rec.searched_at = Some(now);
        rec.searches = 1;
        rec.next_search_at = next_search(now, 1);
        rec.updated_at = now;
        let Some(r) = pick(&releases, profile, fastest) else {
            tracing::info!(album = album_id, found = releases.len(), ms = started.elapsed().as_millis() as u64, "lidarr quick get: nothing usable on the indexers");
            rec.state = RequestState::Searching;
            rec.message = Some(match turned_down(&releases) {
                Some(why) => format!("{why} {NO_COPY}"),
                None => NO_COPY.into(),
            });
            return Ok(rec);
        };
        let grab = GrabRelease::of(r)?;
        let what = format!("{} ({}, {})", r.title, r.quality_name(), r.protocol);
        if let Err(e) = c.writer.grab(grab).await {
            if e.kind() == "network" {
                return Err(e);
            }
            tracing::info!(album = album_id, release = %what, error = %e, "lidarr quick get: the download client didn't take it");
            rec.state = RequestState::Failed;
            rec.message = Some(format!("Your download client didn't take it. {}", e.message()));
            return Ok(rec);
        }
        tracing::info!(album = album_id, release = %what, of = releases.len(), ms = started.elapsed().as_millis() as u64, "lidarr quick get: grabbed");
        rec.notes.push(format!("Grabbed {what}."));
        rec.state = RequestState::Downloading;
        rec.progress = Some(0);
        rec.message = None;
        Ok(rec)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_file_cd_images_are_told_by_their_names() {
        for t in ["Artist-Album-CD-FLAC-IMAGE-2021", "Artist - Album [img+cue]", "Artist_Album_FLAC_img", "Artist - Album (2021) [FLAC image+cue]"] {
            assert!(super::one_file_image(t, &["Album", "Artist"]), "{t}");
        }
        for t in ["Northlight-Second_Wind-CD-FLAC-2021-GRP", "Artist - Album (2021) [FLAC+CUE]", "Artist - Album [tracks+cue]", "Artist - Imagine (2021) [FLAC]", "Cuesta - Album"] {
            assert!(!super::one_file_image(t, &["Album", "Artist"]), "{t}");
        }
        // The album's own name isn't a sign.
        assert!(!super::one_file_image("Northlight - Image (2021) [FLAC]", &["Image", "Northlight"]));
        assert!(super::one_file_image("Northlight - Image (2021) [FLAC] [img]", &["Image", "Northlight"]));
    }

    use super::*;
    use crate::lidarr::api::{ProfileItem, QualityRef, ReleaseQuality};

    fn rel(guid: &str, protocol: &str, quality: &str, seeders: Option<i64>) -> Release {
        Release {
            guid: guid.into(),
            indexer_id: 1,
            title: guid.into(),
            protocol: protocol.into(),
            seeders,
            quality: Some(ReleaseQuality { quality: Some(QualityRef { id: 0, name: quality.into() }) }),
            approved: true,
            download_allowed: true,
            ..Default::default()
        }
    }

    fn lossless_cutoff() -> QualityProfile {
        let q = |id: i64, name: &str| ProfileItem { quality: Some(QualityRef { id, name: name.into() }), allowed: true, ..Default::default() };
        QualityProfile { id: 2, name: "Any".into(), cutoff: Some(6), items: vec![q(4, "MP3-320"), q(6, "FLAC")], ..Default::default() }
    }

    #[test]
    fn the_fastest_good_copy_wins_usenet_then_well_seeded_torrents() {
        let p = lossless_cutoff();
        let list = vec![rel("t-thin", "torrent", "FLAC", Some(2)), rel("t-busy", "torrent", "FLAC", Some(80)), rel("nzb", "usenet", "FLAC", None)];
        assert_eq!(pick(&list, Some(&p), true).unwrap().guid, "nzb");
        assert_eq!(pick(&list[..2], Some(&p), true).unwrap().guid, "t-busy");
        assert_eq!(pick(&list, Some(&p), false).unwrap().guid, "t-thin", "setting off: Lidarr's own first choice");
    }

    #[test]
    fn a_fast_copy_below_the_profiles_cutoff_loses_to_a_good_one() {
        let p = lossless_cutoff();
        let list = vec![rel("nzb-mp3", "usenet", "MP3-320", None), rel("t-flac", "torrent", "FLAC", Some(10))];
        assert_eq!(pick(&list, Some(&p), true).unwrap().guid, "t-flac");
        // Only lower ones there: the fastest of those.
        let low = vec![rel("t-mp3", "torrent", "MP3-320", Some(10)), rel("nzb-mp3", "usenet", "MP3-320", None)];
        assert_eq!(pick(&low, Some(&p), true).unwrap().guid, "nzb-mp3");
    }

    #[test]
    fn rejected_unapproved_unseeded_and_discography_copies_are_never_picked() {
        let p = lossless_cutoff();
        let list = vec![
            Release { rejected: true, rejections: vec!["Not enough tracks".into()], ..rel("a", "usenet", "FLAC", None) },
            Release { approved: false, ..rel("b", "usenet", "FLAC", None) },
            Release { discography: true, ..rel("c", "usenet", "FLAC", None) },
            Release { download_allowed: false, ..rel("d", "usenet", "FLAC", None) },
            rel("e", "torrent", "FLAC", Some(0)),
        ];
        assert!(pick(&list, Some(&p), true).is_none());
        assert!(pick(&list, Some(&p), false).is_none());
        assert!(turned_down(&list).unwrap().contains("Not enough tracks"));
        assert!(turned_down(&[]).is_none());
    }
}
