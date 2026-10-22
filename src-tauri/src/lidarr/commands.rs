//! Lidarr's Tauri commands (docs/IPC.md).

use tauri::State;

use super::check::CheckReport;
use super::mbpick::Picked;
use super::request::{MissingReleases, Preview, RequestInput, RequestRecord};
use super::{Lidarr, LidarrStatus};
use crate::error::{AppError, AppResult};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

/// Sent when a setup check that started by itself finishes, so Get shows.
pub const EVENT_CHANGED: &str = "lidarr://changed";

/// Lidarr's state here. When it can be reached (through the Bridge, say)
/// but no setup check has passed, one starts in the background and
/// `lidarr://changed` follows when it's done.
#[tauri::command]
pub async fn lidarr_status(app: tauri::AppHandle, lidarr: State<'_, Lidarr>) -> AppResult<LidarrStatus> {
    let l = lidarr.inner().clone();
    let (st, start) = blocking(move || {
        let st = l.status();
        let start = l.claim_auto_check(&st);
        Ok((st, start))
    })
    .await?;
    if start {
        let l = lidarr.inner().clone();
        tauri::async_runtime::spawn(async move {
            let r = l.check().await;
            match &r {
                Ok(r) => tracing::info!(passed = r.passed, "Lidarr's setup check ran by itself"),
                Err(e) => tracing::info!("Lidarr's setup check by itself didn't finish: {e}"),
            }
            l.auto_check_done(r.is_ok());
            use tauri::Emitter;
            let _ = app.emit(EVENT_CHANGED, ());
        });
    }
    Ok(st)
}

/// Save (or with null remove) Lidarr's API key in the OS keychain.
#[tauri::command]
pub async fn lidarr_key_set(lidarr: State<'_, Lidarr>, key: Option<String>) -> AppResult<LidarrStatus> {
    let l = lidarr.inner().clone();
    blocking(move || l.set_key(key.as_deref())).await
}

/// Save extra headers for the away address (`Name: value` lines; null or
/// blank removes them), kept in the OS keychain.
#[tauri::command]
pub async fn lidarr_headers_set(lidarr: State<'_, Lidarr>, text: Option<String>) -> AppResult<LidarrStatus> {
    let l = lidarr.inner().clone();
    blocking(move || l.set_headers(text.as_deref())).await
}

/// The read-only setup check.
#[tauri::command]
pub async fn lidarr_check(lidarr: State<'_, Lidarr>) -> AppResult<CheckReport> {
    lidarr.check().await
}

/// The user says an empty root folder is (or isn't) separate from the Plex
/// music folder.
#[tauri::command]
pub async fn lidarr_confirm_root(lidarr: State<'_, Lidarr>, path: String, confirmed: bool) -> AppResult<()> {
    let l = lidarr.inner().clone();
    blocking(move || l.confirm_root(&path, confirmed)).await
}

/// Which release group to request for an album or a song (MusicBrainz).
#[tauri::command]
pub async fn lidarr_pick(
    lidarr: State<'_, Lidarr>,
    artist: String,
    artist_id: Option<String>,
    title: String,
    song: bool,
    isrc: Option<String>,
) -> AppResult<Picked> {
    lidarr.pick(&artist, artist_id.as_deref(), &title, song, isrc.as_deref()).await
}

/// What a request will do and where its files will land. Reads only.
#[tauri::command]
pub async fn lidarr_request_preview(lidarr: State<'_, Lidarr>, input: RequestInput) -> AppResult<Preview> {
    lidarr.preview(&input).await
}

/// Send a request (or keep it waiting when Lidarr can't be reached).
#[tauri::command]
pub async fn lidarr_request(lidarr: State<'_, Lidarr>, input: RequestInput) -> AppResult<RequestRecord> {
    lidarr.request(input).await
}

/// "Request these": up to 10 albums confirmed at once, each with the full
/// check and every guard. One that fails doesn't stop the rest.
#[tauri::command]
pub async fn lidarr_request_batch(lidarr: State<'_, Lidarr>, inputs: Vec<RequestInput>) -> AppResult<Vec<super::request::BatchItem>> {
    lidarr.request_batch(inputs).await
}

/// Requests on this device, newest first, with their latest state.
#[tauri::command]
pub async fn lidarr_requests(lidarr: State<'_, Lidarr>, refresh: bool) -> AppResult<Vec<super::grabbed::RequestView>> {
    if refresh {
        let _ = lidarr.refresh_statuses().await;
        if let Err(e) = lidarr.note_grabs().await {
            tracing::debug!("lidarr grabs: {e}");
        }
    }
    Ok(lidarr.request_views())
}

/// Cancel a waiting request, or clear a finished one.
#[tauri::command]
pub async fn lidarr_request_cancel(lidarr: State<'_, Lidarr>, id: String) -> AppResult<Vec<super::grabbed::RequestView>> {
    let l = lidarr.inner().clone();
    l.cancel(&id).await?;
    blocking(move || Ok(l.request_views())).await
}

/// An artist's official releases the library lacks, for the artist page.
#[tauri::command]
pub async fn lidarr_missing_releases(lidarr: State<'_, Lidarr>, artist_id: String, artist: String) -> AppResult<MissingReleases> {
    lidarr.missing_releases(&artist_id, &artist).await
}

// ---------------------------------------------------------------- upgrades

use super::upgrade::{AlbumUpgrade, BulkResult, Candidates, Confirmed, Offer, SwapResult, Under, UpgradeRecord, Upgrades};
use crate::musicshare::ShareMap;

/// Whether an album's menu offers "Look for a better copy", and its upgrade.
#[tauri::command]
pub async fn lidarr_upgrade_album(upgrades: State<'_, Upgrades>, album_id: String) -> AppResult<AlbumUpgrade> {
    upgrades.for_album(&album_id).await
}

/// Ask Lidarr for a better copy of an album, into the staging folder.
#[tauri::command]
pub async fn lidarr_upgrade_request(upgrades: State<'_, Upgrades>, album_id: String) -> AppResult<UpgradeRecord> {
    upgrades.request(&album_id).await
}

/// Ask for better copies of every released album under a quality.
#[tauri::command]
pub async fn lidarr_upgrade_bulk(upgrades: State<'_, Upgrades>, under: Under) -> AppResult<BulkResult> {
    upgrades.bulk(under).await
}

/// Upgrades on this computer, newest first; refresh follows them in Lidarr.
#[tauri::command]
pub async fn lidarr_upgrades(upgrades: State<'_, Upgrades>, refresh: bool) -> AppResult<Vec<UpgradeRecord>> {
    if refresh {
        let _ = upgrades.refresh().await;
    }
    Ok(upgrades.records())
}

/// What swapping would do now: songs that upgrade and songs kept, with why.
#[tauri::command]
pub async fn lidarr_upgrade_offer(upgrades: State<'_, Upgrades>, id: String) -> AppResult<Offer> {
    upgrades.offer(&id).await
}

/// Swap the better copies in (null: every song that upgrades).
#[tauri::command]
pub async fn lidarr_upgrade_swap(upgrades: State<'_, Upgrades>, id: String, track_ids: Option<Vec<String>>) -> AppResult<SwapResult> {
    upgrades.swap(&id, track_ids).await
}

/// Put the old files of the last swap back exactly.
#[tauri::command]
pub async fn lidarr_upgrade_undo(upgrades: State<'_, Upgrades>, id: String) -> AppResult<UpgradeRecord> {
    upgrades.undo(&id).await
}

/// Clear an upgrade from the list.
#[tauri::command]
pub async fn lidarr_upgrade_clear(upgrades: State<'_, Upgrades>, id: String) -> AppResult<Vec<UpgradeRecord>> {
    upgrades.clear(&id).await
}

/// Where this computer reaches the music share (this device only).
#[tauri::command]
pub async fn music_share_get(lidarr: State<'_, Lidarr>) -> AppResult<ShareMap> {
    let db = lidarr.inner.db.clone();
    blocking(move || Ok(crate::musicshare::load(&db))).await
}

#[tauri::command]
pub async fn music_share_set(lidarr: State<'_, Lidarr>, map: ShareMap) -> AppResult<ShareMap> {
    let db = lidarr.inner.db.clone();
    blocking(move || crate::musicshare::save(&db, &map)).await
}

/// Plex's music folders, as Plex sees them (to fill in the share mapping).
#[tauri::command]
pub async fn music_share_plex_folders(lidarr: State<'_, Lidarr>) -> AppResult<Vec<String>> {
    let mut l = lidarr.inner.plex.music_files().await?.locations;
    l.sort();
    l.dedup();
    Ok(l)
}

/// The album's songs and the releases it could be, to confirm one.
#[tauri::command]
pub async fn lidarr_upgrade_candidates(upgrades: State<'_, Upgrades>, album_id: String) -> AppResult<Candidates> {
    upgrades.candidates(&album_id).await
}

/// Confirm (or with null forget) which release group an album is.
#[tauri::command]
pub async fn lidarr_upgrade_confirm(upgrades: State<'_, Upgrades>, album_id: String, release_group_id: Option<String>) -> AppResult<Option<Confirmed>> {
    upgrades.confirm(&album_id, release_group_id.as_deref()).await
}

/// The release groups an artist is limited to (empty: no limit).
#[tauri::command]
pub async fn lidarr_allowlist(lidarr: State<'_, Lidarr>, artist: String, artist_id: Option<String>) -> AppResult<Vec<String>> {
    Ok(lidarr.allowlist(&artist, artist_id.as_deref()).await)
}

/// Limit an artist to these release groups (empty removes the limit).
#[tauri::command]
pub async fn lidarr_allowlist_set(lidarr: State<'_, Lidarr>, artist: String, artist_id: Option<String>, release_group_ids: Vec<String>) -> AppResult<Vec<String>> {
    lidarr.set_allowlist(&artist, artist_id.as_deref(), release_group_ids).await
}

// ---------------------------------------------------------------- modes and work

/// Every artist's mode, with why.
#[tauri::command]
pub async fn lidarr_modes(lidarr: State<'_, Lidarr>) -> AppResult<Vec<super::modes::ArtistMode>> {
    lidarr.modes_now().await
}

/// Set an artist's mode; Managed needs `confirmations` of 2.
#[tauri::command]
pub async fn lidarr_mode_set(
    lidarr: State<'_, Lidarr>,
    key: String,
    name: String,
    mode: super::modes::ModeSetting,
    confirmations: u8,
) -> AppResult<super::modes::ArtistPrefs> {
    let l = lidarr.inner().clone();
    blocking(move || l.set_mode(&key, &name, mode, confirmations)).await
}

/// An artist's choices: add new albums on their own, look for better copies.
#[tauri::command]
pub async fn lidarr_artist_choices_set(
    lidarr: State<'_, Lidarr>,
    key: String,
    name: String,
    auto_add: Option<bool>,
    hunt: bool,
    target: super::modes::Target,
) -> AppResult<super::modes::ArtistPrefs> {
    let l = lidarr.inner().clone();
    blocking(move || l.set_artist_choices(&key, &name, auto_add, hunt, target)).await
}

/// Every artist's choices, by key.
#[tauri::command]
pub async fn lidarr_artist_choices(lidarr: State<'_, Lidarr>) -> AppResult<std::collections::HashMap<String, super::modes::ArtistPrefs>> {
    let l = lidarr.inner().clone();
    blocking(move || Ok(l.artist_prefs())).await
}

/// Show what a fix would change (reads only).
#[tauri::command]
pub async fn lidarr_change_preview(lidarr: State<'_, Lidarr>, action: super::check::FixAction, input: Option<String>) -> AppResult<super::changes::Proposal> {
    lidarr.propose(action, input).await
}

/// Show the "Platter Hi-Res" profile Platter would add (reads only).
#[tauri::command]
pub async fn lidarr_hires_preview(lidarr: State<'_, Lidarr>) -> AppResult<super::changes::Proposal> {
    lidarr.propose_hires_profile().await
}

/// Make a change the user said yes to.
#[tauri::command]
pub async fn lidarr_change_apply(lidarr: State<'_, Lidarr>, id: String) -> AppResult<super::changes::ChangeEntry> {
    lidarr.apply(&id).await
}

/// The changes log, newest first.
#[tauri::command]
pub async fn lidarr_changes(lidarr: State<'_, Lidarr>) -> AppResult<Vec<super::changes::ChangeEntry>> {
    let l = lidarr.inner().clone();
    blocking(move || Ok(l.changes())).await
}

/// Where getting albums is at, for the top of the Changes log.
#[tauri::command]
pub async fn lidarr_stage(waves: State<'_, super::waves::Waves>, bring: State<'_, crate::import::bring::Bring>) -> AppResult<Vec<String>> {
    let looking_up = bring.looking_up().await.unwrap_or(0);
    let w = waves.inner().clone();
    blocking(move || w.stage(looking_up)).await
}

/// Put a logged change back.
#[tauri::command]
pub async fn lidarr_change_put_back(lidarr: State<'_, Lidarr>, id: String) -> AppResult<super::changes::ChangeEntry> {
    lidarr.put_back(&id).await
}

/// What re-matching an album folder would do (reads only).
#[tauri::command]
pub async fn lidarr_rematch_preview(lidarr: State<'_, Lidarr>, folder: String) -> AppResult<super::rematch::RematchPlan> {
    lidarr.rematch_preview(&folder).await
}

/// Re-match an album folder the user confirmed.
#[tauri::command]
pub async fn lidarr_rematch_apply(lidarr: State<'_, Lidarr>, folder: String, release_id: Option<i64>) -> AppResult<super::rematch::RematchPlan> {
    lidarr.rematch_apply(&folder, release_id).await
}

/// What the Lidarr work is doing: who did it last, practice mode, albums
/// waiting in staging.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LidarrWork {
    pub catch_up: super::catchup::CatchUpStatus,
    pub practice_until: Option<i64>,
    pub practice: bool,
    pub staged: Vec<super::promote::Staged>,
}

#[tauri::command]
pub async fn lidarr_work(lidarr: State<'_, Lidarr>) -> AppResult<LidarrWork> {
    let l = lidarr.inner().clone();
    blocking(move || {
        Ok(LidarrWork { catch_up: l.catch_up_status(), practice_until: l.practice_until(), practice: l.practice_active(), staged: l.staged() })
    })
    .await
}

/// Catch up with Lidarr now.
#[tauri::command]
pub async fn lidarr_catch_up(lidarr: State<'_, Lidarr>) -> AppResult<super::catchup::CatchUpReport> {
    lidarr.catch_up().await
}

/// Resume protected work after a pause, keeping Lidarr's settings as they are.
#[tauri::command]
pub async fn lidarr_resume_protected(lidarr: State<'_, Lidarr>) -> AppResult<()> {
    let l = lidarr.inner().clone();
    blocking(move || l.resume_protected()).await
}

/// Add an album waiting in staging to the library; `always` adds the
/// artist's later albums on their own.
#[tauri::command]
pub async fn lidarr_staged_add(lidarr: State<'_, Lidarr>, id: String, always: bool) -> AppResult<super::promote::Staged> {
    lidarr.promotion_approve(&id, always).await
}

/// Take an added album out again; `trash_ok` lets Platter keep Plex's trash for the move.
#[tauri::command]
pub async fn lidarr_staged_undo(lidarr: State<'_, Lidarr>, id: String, trash_ok: bool) -> AppResult<super::promote::Staged> {
    lidarr.promotion_undo(&id, trash_ok).await
}

/// Look for better copies for protected artists that asked for it.
#[tauri::command]
pub async fn lidarr_hunt(upgrades: State<'_, Upgrades>) -> AppResult<Vec<super::hunt::HuntItem>> {
    upgrades.hunt().await
}

/// Better copies: the practice week, and whether swaps are paused today.
#[tauri::command]
pub fn better_copies_status(upgrades: State<'_, Upgrades>) -> super::better::BetterStatus {
    upgrades.better_status()
}

/// What's New and a notification for an album Lidarr brought in.
pub struct AppAnnouncer {
    pub app: tauri::AppHandle,
    pub plex: crate::plex::PlexState,
}

impl super::catchup::Announcer for AppAnnouncer {
    fn album_added(&self, artist: &str, album: &str) {
        use tauri_plugin_notification::NotificationExt;
        let text = if artist.is_empty() { format!("{album} is in your library") } else { format!("{album} by {artist} is in your library") };
        self.plex.toast("info", text.clone());
        let _ = self.app.notification().builder().title("New in your library").body(text).show();
    }
}

// ---------------------------------------------------------------- bring in my listening

use super::waves::{Waves, WavesStatus};

/// "Bring in my listening": the waves, today's count, why it's paused,
/// what needs the user and what practice mode listed.
#[tauri::command]
pub async fn bring_in_status(waves: State<'_, Waves>) -> AppResult<WavesStatus> {
    let w = waves.inner().clone();
    blocking(move || w.status()).await
}

/// Pause it, or carry on (a pass runs soon).
#[tauri::command]
pub async fn bring_in_pause(waves: State<'_, Waves>, paused: bool) -> AppResult<WavesStatus> {
    let w = waves.inner().clone();
    blocking(move || w.set_paused(paused)).await
}

/// A "Needs you" album: try it again (`retry`), or leave it be.
#[tauri::command]
pub async fn bring_in_needs_you(waves: State<'_, Waves>, key: String, retry: bool) -> AppResult<WavesStatus> {
    let w = waves.inner().clone();
    blocking(move || w.needs_you(&key, retry)).await
}

/// One-click Get: the request with every check, or why it needs the user
/// (the first request ever, or a check that says no).
#[tauri::command]
pub async fn lidarr_get(lidarr: State<'_, Lidarr>, input: RequestInput) -> AppResult<super::request::GetOutcome> {
    lidarr.get(input).await
}

/// Tap a song the library doesn't have: it's got the quick way and plays
/// as soon as it's here (`lidarr::tap`), or why it needs the user.
#[tauri::command]
pub async fn lidarr_tap(taps: State<'_, super::tap::Taps>, input: super::tap::TapInput) -> AppResult<super::tap::TapOutcome> {
    taps.tap(input).await
}

/// Taps on this device, newest first.
#[tauri::command]
pub async fn lidarr_taps(taps: State<'_, super::tap::Taps>) -> AppResult<Vec<super::tap::Tap>> {
    Ok(taps.list())
}

/// How long the last get took, step by step.
#[tauri::command]
pub async fn lidarr_last_get(lidarr: State<'_, Lidarr>) -> AppResult<Option<super::tap::LastGet>> {
    Ok(lidarr.last_get())
}

/// "Search again" for an album nothing turned up for: one AlbumSearch for
/// it now, and the searches again start over.
#[tauri::command]
pub async fn lidarr_search_again(lidarr: State<'_, Lidarr>, id: String) -> AppResult<RequestRecord> {
    lidarr.search_again(&id).await
}

/// Take albums off the "Get albums" queue (all of them when `releaseGroupIds` is empty).
#[tauri::command]
pub async fn get_albums_unqueue(waves: State<'_, Waves>, release_group_ids: Vec<String>) -> AppResult<WavesStatus> {
    let w = waves.inner().clone();
    blocking(move || w.unqueue(&release_group_ids)).await
}