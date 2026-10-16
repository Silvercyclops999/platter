//! Re-matching files Lidarr couldn't match under a Managed artist, one
//! album folder at a time, through Lidarr's manual import. Only offered
//! while Rename Tracks is off (with it on, an import could rename files).
//!
//! Measured on a real server, the guard is strict:
//!
//! - A manual import onto a release other than the album's current one
//!   switches the album's edition even with release switching disabled, and
//!   the files linked to the old edition stop counting. So only files whose
//!   proposed release is the current one are imported; anything else is
//!   shown ("this would switch the album's edition and unlink N files") and
//!   never imported.
//! - Every file names its tracks explicitly: the current release's tracks,
//!   matched on disc and track number, each without a file yet.
//! - "Has missing tracks" is a soft rejection (Lidarr groups per disc
//!   folder) and doesn't stop a file that passes the rest; any other
//!   rejection does.
//! - Bonus tracks that aren't on the current release can't be linked without
//!   switching it: they're kept as they are, with no fix offered.
//! - Afterwards the album must have exactly that many more files and still
//!   every file it had; if not, Platter stops and says so.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use serde::Serialize;

use super::Lidarr;
use super::api::{AlbumDetail, ImportProposal, ReleaseTrack, TrackFile};
use super::modes::parent_raw;
use super::paths::NormPath;
use super::write::{ImportFile, WriteCall};
use crate::error::{AppError, AppResult};

/// Looks at the album's files after the import before giving up.
const VERIFY_TRIES: usize = 10;

/// A file left as it is, with why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptFile {
    pub path: String,
    pub why: String,
}

/// What a re-match will do for one album folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RematchPlan {
    pub folder: String,
    pub album_id: Option<i64>,
    pub album: String,
    /// The album's current release (edition).
    pub release: String,
    /// Files that will be linked, with the tracks they go to.
    pub files: Vec<String>,
    pub kept: Vec<KeptFile>,
    /// Files Lidarr would put on another edition: never imported.
    pub other_edition: usize,
    /// Files the album has now (an edition switch would unlink them).
    pub linked_now: usize,
    /// Shown when files are on another edition.
    pub edition_note: Option<String>,
    #[serde(skip)]
    pub imports: Vec<ImportFile>,
}

fn soft(reason: &str) -> bool {
    reason.to_ascii_lowercase().contains("missing tracks")
}

/// The plan, from what Lidarr has. `unmapped` are Lidarr's unmatched files;
/// only those directly in `folder` are considered.
pub fn plan(
    folder: &str,
    unmapped: &[TrackFile],
    proposals: &[ImportProposal],
    albums: &HashMap<i64, AlbumDetail>,
    release_tracks: &HashMap<i64, Vec<ReleaseTrack>>,
    linked_now: usize,
) -> RematchPlan {
    let dir = NormPath::parse(folder);
    let wanted: HashSet<NormPath> =
        unmapped.iter().filter(|f| parent_raw(&f.path).is_some_and(|p| NormPath::parse(&p) == dir)).map(|f| NormPath::parse(&f.path)).collect();
    let mine: Vec<&ImportProposal> = proposals.iter().filter(|p| wanted.contains(&NormPath::parse(&p.path))).collect();
    // One album at a time: the album most files go to.
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for p in &mine {
        if let Some(a) = p.album.as_ref().filter(|a| a.id > 0) {
            *counts.entry(a.id).or_default() += 1;
        }
    }
    let album_id = counts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(id, _)| *id);
    let detail = album_id.and_then(|id| albums.get(&id));
    let current = detail.and_then(AlbumDetail::current_release);
    let tracks: &[ReleaseTrack] = current.and_then(|r| release_tracks.get(&r.id)).map(Vec::as_slice).unwrap_or(&[]);
    let mut out = RematchPlan {
        folder: folder.to_string(),
        album_id,
        album: detail.map(|d| d.title.clone()).unwrap_or_default(),
        release: current.map(|r| r.title.clone()).unwrap_or_default(),
        files: Vec::new(),
        kept: Vec::new(),
        other_edition: 0,
        linked_now,
        edition_note: None,
        imports: Vec::new(),
    };
    let mut taken: BTreeSet<i64> = BTreeSet::new();
    let keep = |out: &mut RematchPlan, p: &ImportProposal, why: &str| out.kept.push(KeptFile { path: p.path.clone(), why: why.to_string() });
    for p in mine {
        let Some(a) = p.album.as_ref().filter(|a| a.id > 0) else {
            keep(&mut out, p, "Lidarr couldn't tell which album it's from");
            continue;
        };
        if Some(a.id) != album_id {
            keep(&mut out, p, "Lidarr matched it to another album; re-match that album on its own");
            continue;
        }
        let Some(cur) = current else {
            keep(&mut out, p, "Lidarr has no current edition for the album");
            continue;
        };
        if p.album_release_id != Some(cur.id) {
            out.other_edition += 1;
            keep(&mut out, p, "on another edition of the album; importing it would switch the edition");
            continue;
        }
        if let Some(r) = p.rejections.iter().find(|r| !soft(&r.reason)) {
            keep(&mut out, p, &format!("Lidarr won't take it: {}", r.reason));
            continue;
        }
        if p.tracks.is_empty() {
            keep(&mut out, p, "Lidarr couldn't tell which track it is");
            continue;
        }
        let mut ids = Vec::new();
        let mut why: Option<&str> = None;
        for t in &p.tracks {
            match tracks.iter().find(|x| x.medium_number == t.medium_number && x.absolute_track_number == t.absolute_track_number) {
                None => why = Some("not on the album's current edition (a bonus track); kept as it is"),
                Some(x) if x.has_file => why = Some("that track already has a file"),
                Some(x) if taken.contains(&x.id) || ids.contains(&x.id) => why = Some("another file goes to the same track"),
                Some(x) => ids.push(x.id),
            }
            if why.is_some() {
                break;
            }
        }
        if let Some(w) = why {
            keep(&mut out, p, w);
            continue;
        }
        taken.extend(ids.iter().copied());
        out.files.push(p.path.clone());
        out.imports.push(ImportFile {
            path: p.path.clone(),
            artist_id: p.artist.as_ref().map(|x| x.id).unwrap_or(detail.map(|d| d.artist_id).unwrap_or(0)),
            album_id: a.id,
            album_release_id: cur.id,
            track_ids: ids,
            quality: p.quality.clone().unwrap_or(serde_json::Value::Null),
            indexer_flags: 0,
            download_id: p.download_id.clone().unwrap_or_default(),
            disable_release_switching: true,
        });
    }
    if out.other_edition > 0 {
        out.edition_note = Some(format!(
            "{} {} on another edition. Importing {} would switch the album's edition and unlink {} {} it has now, so Platter leaves {} as {}.",
            out.other_edition,
            if out.other_edition == 1 { "file is" } else { "files are" },
            if out.other_edition == 1 { "it" } else { "them" },
            linked_now,
            if linked_now == 1 { "file" } else { "files" },
            if out.other_edition == 1 { "it" } else { "them" },
            if out.other_edition == 1 { "it is" } else { "they are" },
        ));
    }
    out
}

/// After an import: exactly `imported` more files, and every file the album
/// had is still linked.
pub fn verify(before: &[TrackFile], after: &[TrackFile], imported: usize) -> Result<(), String> {
    let now: HashSet<NormPath> = after.iter().map(|f| NormPath::parse(&f.path)).collect();
    let gone: Vec<&str> = before.iter().filter(|f| !now.contains(&NormPath::parse(&f.path))).map(|f| f.path.as_str()).collect();
    if !gone.is_empty() {
        return Err(format!("{} file{} the album had {} no longer linked ({}).", gone.len(), if gone.len() == 1 { "" } else { "s" }, if gone.len() == 1 { "is" } else { "are" }, gone[0]));
    }
    if after.len() != before.len() + imported {
        return Err(format!("The album has {} files now; Platter expected {}.", after.len(), before.len() + imported));
    }
    Ok(())
}

impl Lidarr {
    async fn rematch_plan(&self, client: &super::api::LidarrClient, folder: &str) -> AppResult<(RematchPlan, Vec<TrackFile>)> {
        if client.naming().await?.rename_tracks {
            return Err(AppError::Invalid("Rename Tracks is on in Lidarr, so a re-match could rename files. Platter only re-matches with it off.".into()));
        }
        let unmapped = client.unmapped_files().await?;
        let proposals = client.manual_import(folder).await?;
        let mut albums = HashMap::new();
        let mut tracks = HashMap::new();
        for id in proposals.iter().filter_map(|p| p.album.as_ref().map(|a| a.id)).filter(|id| *id > 0).collect::<BTreeSet<_>>() {
            let d = client.album_detail(id).await?;
            if let Some(r) = d.current_release() {
                tracks.insert(r.id, client.release_tracks(r.id).await?);
            }
            albums.insert(id, d);
        }
        let first = plan(folder, &unmapped, &proposals, &albums, &tracks, 0);
        let before = match first.album_id {
            Some(id) => client.track_files(id).await?,
            None => Vec::new(),
        };
        Ok((plan(folder, &unmapped, &proposals, &albums, &tracks, before.len()), before))
    }

    /// What re-matching an album folder would do. Reads only.
    pub async fn rematch_preview(&self, folder: &str) -> AppResult<RematchPlan> {
        let (client, _) = self.connection().await?;
        Ok(self.rematch_plan(&client, folder).await?.0)
    }

    /// Re-match an album folder the user confirmed: the plan is made again
    /// from what Lidarr has now, imported, then checked.
    pub async fn rematch_apply(&self, folder: &str) -> AppResult<RematchPlan> {
        let _one = self.lock.lock().await;
        let (client, writer) = self.connection().await?;
        let (p, before) = self.rematch_plan(&client, folder).await?;
        let Some(album_id) = p.album_id.filter(|_| !p.imports.is_empty()) else {
            return Err(AppError::Invalid("Nothing in this folder can be re-matched safely.".into()));
        };
        writer.send_call(WriteCall::ManualImport(p.imports.clone())).await?;
        let mut last = String::new();
        for i in 0..VERIFY_TRIES {
            let after = client.track_files(album_id).await?;
            match verify(&before, &after, p.imports.len()) {
                Ok(()) => {
                    self.log_change(
                        &format!("Re-match {} in Lidarr", p.album),
                        &format!("{} files not matched in {}", p.imports.len(), p.folder),
                        &format!("Linked to {} ({}), renaming nothing", p.album, p.release),
                        true,
                        None,
                    )?;
                    return Ok(p);
                }
                Err(e) => last = e,
            }
            if i + 1 < VERIFY_TRIES {
                tokio::time::sleep(self.inner.album_wait.max(Duration::from_millis(1))).await;
            }
        }
        self.log_change(&format!("Re-match {} in Lidarr: stopped", p.album), "", &last, true, None)?;
        Err(AppError::Server(format!("Lidarr's import didn't come out as expected: {last} Platter stopped; check the album in Lidarr.")))
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::{AlbumRelease, IdRef, Rejection};
    use super::*;

    const DIR: &str = "/data/media/Music/Lidarr/Northlight/First Light (2019)";

    fn tf(id: i64, p: &str) -> TrackFile {
        TrackFile { id, path: p.into(), size: None }
    }

    fn rt(id: i64, disc: i64, n: i64, has: bool) -> ReleaseTrack {
        ReleaseTrack { id, medium_number: disc, absolute_track_number: n, has_file: has, title: String::new() }
    }

    fn prop(file: &str, release: i64, tracks: Vec<(i64, i64)>) -> ImportProposal {
        ImportProposal {
            path: format!("{DIR}/{file}"),
            artist: Some(IdRef { id: 3, title: String::new() }),
            album: Some(IdRef { id: 10, title: "First Light".into() }),
            album_release_id: Some(release),
            tracks: tracks.into_iter().map(|(d, n)| rt(0, d, n, false)).collect(),
            ..Default::default()
        }
    }

    type World = (Vec<TrackFile>, HashMap<i64, AlbumDetail>, HashMap<i64, Vec<ReleaseTrack>>);

    fn world() -> World {
        let unmapped = vec![tf(0, &format!("{DIR}/1-02 Noon.flac")), tf(0, &format!("{DIR}/2-01 Dusk.flac")), tf(0, &format!("{DIR}/3-01 Bonus.flac"))];
        let album = AlbumDetail {
            id: 10,
            title: "First Light".into(),
            artist_id: 3,
            releases: vec![
                AlbumRelease { id: 100, title: "CD".into(), monitored: true, track_count: 3 },
                AlbumRelease { id: 101, title: "Deluxe".into(), monitored: false, track_count: 5 },
            ],
        };
        let tracks = HashMap::from([(100, vec![rt(501, 1, 1, true), rt(502, 1, 2, false), rt(503, 2, 1, false)])]);
        (unmapped, HashMap::from([(10, album)]), tracks)
    }

    #[test]
    fn only_the_current_edition_with_explicit_tracks_is_imported() {
        let (unmapped, albums, tracks) = world();
        let mut noon = prop("1-02 Noon.flac", 100, vec![(1, 2)]);
        // Lidarr's proposal carries its own track ids; Platter uses the current release's.
        noon.tracks[0].id = 999;
        noon.rejections = vec![Rejection { reason: "Has missing tracks".into(), kind: "permanent".into() }];
        let props = vec![noon, prop("2-01 Dusk.flac", 101, vec![(2, 1)]), prop("3-01 Bonus.flac", 100, vec![(3, 1)])];
        let p = plan(DIR, &unmapped, &props, &albums, &tracks, 1);
        assert_eq!(p.imports.len(), 1, "{p:?}");
        let i = &p.imports[0];
        assert_eq!((i.album_release_id, i.track_ids.clone(), i.disable_release_switching), (100, vec![502], true));
        // Another edition: never imported, explained.
        assert_eq!(p.other_edition, 1);
        assert!(p.edition_note.as_deref().unwrap().contains("switch the album's edition and unlink 1 file"), "{:?}", p.edition_note);
        // A bonus track not on the current edition is kept, no fix.
        assert!(p.kept.iter().any(|k| k.path.ends_with("3-01 Bonus.flac") && k.why.contains("bonus track")), "{:?}", p.kept);
    }

    #[test]
    fn hard_rejections_tracks_with_files_and_other_folders_are_left_alone() {
        let (mut unmapped, albums, tracks) = world();
        unmapped.push(tf(0, "/data/media/Music/Lidarr/Northlight/Other/01 x.flac"));
        let mut bad = prop("1-02 Noon.flac", 100, vec![(1, 2)]);
        bad.rejections = vec![Rejection { reason: "Not an upgrade for existing track file(s)".into(), kind: "permanent".into() }];
        let has = prop("2-01 Dusk.flac", 100, vec![(1, 1)]);
        let elsewhere = ImportProposal { path: "/data/media/Music/Lidarr/Northlight/Other/01 x.flac".into(), ..prop("x", 100, vec![(2, 1)]) };
        let p = plan(DIR, &unmapped, &[bad, has, elsewhere], &albums, &tracks, 1);
        assert!(p.imports.is_empty(), "{p:?}");
        assert_eq!(p.kept.len(), 2, "the other folder isn't part of this plan");
        assert!(p.kept.iter().any(|k| k.why.contains("Lidarr won't take it")));
        assert!(p.kept.iter().any(|k| k.why.contains("already has a file")));
    }

    #[test]
    fn the_count_check_catches_a_drop_or_a_short_import() {
        let before = vec![tf(1, "/a/1.flac"), tf(2, "/a/2.flac")];
        assert!(verify(&before, &[tf(1, "/a/1.flac"), tf(2, "/a/2.flac"), tf(3, "/a/3.flac")], 1).is_ok());
        let dropped = verify(&before, &[tf(1, "/a/1.flac"), tf(3, "/a/3.flac"), tf(4, "/a/4.flac")], 2).unwrap_err();
        assert!(dropped.contains("no longer linked"), "{dropped}");
        assert!(verify(&before, &before, 1).unwrap_err().contains("expected 3"));
    }
}
