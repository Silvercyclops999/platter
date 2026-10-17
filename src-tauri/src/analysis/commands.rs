//! Tauri commands: analysis progress, playlists with Mix on and their
//! transition picks (kept in the sync file), and what a mixed playlist's
//! rows show (each song's tempo, each gap's transition).

use serde::Serialize;
use tauri::State;

use super::Analyzer;
use super::features::{Analysis, key_name};
use crate::audio::PlayerHandle;
use crate::audio::automix::{self, Feel, MixStyle, Mixed, PlanOptions};
use crate::error::AppResult;
use crate::model::Track;
use crate::settings::SettingsState;
use crate::sync::commands::SyncService;
use crate::sync::mixed;

#[tauri::command]
pub async fn analysis_status(a: State<'_, Analyzer>) -> AppResult<super::Status> {
    a.status().await
}

#[tauri::command]
pub async fn automix_mixed_get(sync: State<'_, SyncService>) -> AppResult<Mixed> {
    sync.store.run(mixed::list).await
}

/// Mix on or off for a playlist (`automix_mix_set`).
#[tauri::command]
pub async fn automix_mix_set(sync: State<'_, SyncService>, player: State<'_, PlayerHandle>, id: String, on: bool) -> AppResult<Mixed> {
    let m = sync.store.run(move |s| mixed::set(s, &id, on)).await?;
    Ok(saved_then_play(m, |m| async move { player.set_mixed(m).await }).await)
}

/// The change is saved: the player is told, and if that fails it's logged
/// (the player picks the change up the next time it's told), not reported,
/// since the setting did change.
async fn saved_then_play<F, Fut>(m: Mixed, tell: F) -> Mixed
where
    F: FnOnce(Mixed) -> Fut,
    Fut: std::future::Future<Output = AppResult<()>>,
{
    if let Err(e) = tell(m.clone()).await {
        tracing::warn!("automix: the player didn't take the mixed playlists: {e}");
    }
    m
}

/// The transition between two songs of a mixed playlist.
#[tauri::command]
pub async fn automix_style_set(
    sync: State<'_, SyncService>,
    player: State<'_, PlayerHandle>,
    id: String,
    from_id: String,
    to_id: String,
    style: MixStyle,
) -> AppResult<Mixed> {
    let m = sync.store.run(move |s| mixed::set_style(s, &id, &from_id, &to_id, style)).await?;
    Ok(saved_then_play(m, |m| async move { player.set_mixed(m).await }).await)
}

/// One song on a mixed playlist.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongMix {
    pub id: String,
    /// Rounded, once analysed.
    pub bpm: Option<u32>,
    pub key: Option<String>,
}

/// The gap between two rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapMix {
    pub from_id: String,
    pub to_id: String,
    /// What it will do: "Can't be mixed" when this is set.
    pub reason: Option<String>,
    pub style: MixStyle,
    /// The songs meet on the beat (both analysed with a steady beat).
    pub beatmatched: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomixView {
    pub songs: Vec<SongMix>,
    pub gaps: Vec<GapMix>,
}

/// What rows and gaps show, from the songs in play order and the analyses.
pub fn view(tracks: &[Track], infos: &std::collections::HashMap<String, Analysis>, styles: Option<&automix::MixedPlaylist>, feel: Feel, fade_ms: u64) -> AutomixView {
    let songs = tracks
        .iter()
        .map(|t| {
            let a = infos.get(&t.id);
            SongMix {
                id: t.id.clone(),
                bpm: a.filter(|a| a.beat_conf >= automix::BEAT_CONF).and_then(|a| a.bpm).map(|b| b.round() as u32),
                key: a.filter(|a| a.key_conf >= 0.3).and_then(|a| a.key).map(key_name),
            }
        })
        .collect();
    let gaps = tracks
        .windows(2)
        .map(|w| {
            let (a, b) = (&w[0], &w[1]);
            let (ai, bi) = (infos.get(&a.id), infos.get(&b.id));
            let style = styles.map_or(MixStyle::Auto, |s| s.style(&a.id, &b.id));
            let reason = automix::unmixable(a, ai, b, bi).map(|r| r.text().to_string());
            let beatmatched = reason.is_none()
                && automix::plan(a, ai, b, bi, &PlanOptions { feel, fade_ms, style: MixStyle::Auto }).is_some_and(|p| p.beatmatched);
            GapMix { from_id: a.id.clone(), to_id: b.id.clone(), reason, style, beatmatched }
        })
        .collect();
    AutomixView { songs, gaps }
}

/// Tempo and transitions for a playlist's songs (in the order shown). On a
/// mixed playlist the songs not analysed yet go to the front of the queue.
#[tauri::command]
pub async fn automix_view(
    a: State<'_, Analyzer>,
    sync: State<'_, SyncService>,
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    playlist_id: Option<String>,
    track_ids: Vec<String>,
) -> AppResult<AutomixView> {
    let ids: Vec<String> = track_ids.into_iter().take(2000).collect();
    let tracks = player.lookup_tracks(&ids).await?;
    let lookup = ids.clone();
    let infos = a.db().read_async(move |c| super::store::get_many(c, &lookup)).await?;
    let mixed = sync.store.run(mixed::list).await?;
    let styles = playlist_id.as_ref().and_then(|id| mixed.get(id));
    if styles.is_some() {
        let missing: Vec<String> = ids.iter().filter(|id| !infos.contains_key(*id)).cloned().collect();
        a.want(&missing);
    }
    let s = settings.get().await?;
    Ok(view(&tracks, &infos, styles, s.automix_style, s.automix_fade_ms()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::testutil::track;

    #[tokio::test]
    async fn a_saved_mix_change_succeeds_even_when_the_player_fails() {
        let mut m = Mixed::new();
        m.insert("pl".into(), automix::MixedPlaylist::default());
        let got = saved_then_play(m.clone(), |_| async { Err(crate::error::AppError::Internal("player gone".into())) }).await;
        assert_eq!(got.keys().collect::<Vec<_>>(), vec!["pl"]);
        let told = std::sync::Arc::new(parking_lot::Mutex::new(None));
        let t = told.clone();
        saved_then_play(m, |m| async move {
            *t.lock() = Some(m.len());
            Ok(())
        })
        .await;
        assert_eq!(*told.lock(), Some(1));
    }

    #[test]
    fn rows_show_tempo_and_gaps_say_why_they_cant_mix() {
        let mut a = track("a", "x", "/a");
        a.album_id = "al".into();
        a.track_number = 1;
        a.duration_ms = 200_000;
        let mut b = a.clone();
        b.id = "b".into();
        b.track_number = 2;
        let mut c = a.clone();
        c.id = "c".into();
        c.album_id = "other".into();
        let mut infos = std::collections::HashMap::new();
        let mut an = crate::analysis::features::Analysis {
            duration_ms: 200_000,
            bpm: Some(127.6),
            beat_conf: 0.9,
            intro: None,
            outro: None,
            key: Some(21),
            key_conf: 0.8,
            energy: 0.5,
            loudness_db: -10.0,
            cue_in_ms: 0,
            cue_out_ms: 199_000,
            intro_end_ms: 0,
            outro_start_ms: 0,
            starts_hot: false,
            ends_hot: false,
            sections: vec![],
            features: vec![],
        };
        infos.insert("b".to_string(), an.clone());
        an.beat_conf = 0.1;
        infos.insert("c".to_string(), an);
        let mut styles = automix::MixedPlaylist::default();
        styles.styles.insert(automix::gap_key("b", "c"), MixStyle::Echo);
        let v = view(&[a, b, c], &infos, Some(&styles), Feel::Smooth, 0);
        assert_eq!(v.songs[0].bpm, None, "not analysed");
        assert_eq!(v.songs[1].bpm, Some(128));
        assert_eq!(v.songs[1].key.as_deref(), Some("A minor"));
        assert_eq!(v.songs[2].bpm, None, "no steady beat");
        assert_eq!(v.gaps[0].reason.as_deref(), Some("Songs from one album, in order"));
        assert_eq!(v.gaps[1].reason, None);
        assert_eq!(v.gaps[1].style, MixStyle::Echo);
    }
}
