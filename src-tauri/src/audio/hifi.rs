//! Headphone correction and bit-perfect output, per output device: what's
//! stored in settings, and the commands that change it (docs/IPC.md).

use serde::{Deserialize, Serialize};
use tauri::State;

use super::autoeq::{self, Entry, Fetcher, Parsed};
use super::dsp::{Correction, PeqFilter};
use super::output::{OutputDevice, OutputState};
use super::player::PlayerHandle;
use crate::error::{AppError, AppResult};
use crate::settings::{Settings, SettingsState};

/// A headphone correction profile used whenever this output device plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCorrection {
    pub device_id: String,
    /// Shown in Settings even while the device is unplugged.
    pub device_name: String,
    /// The headphone, as the index names it.
    pub name: String,
    pub source: String,
    pub form: String,
    /// Its place in the index (used to fetch it again).
    pub path: String,
    pub preamp_db: f32,
    pub filters: Vec<PeqFilter>,
}

/// An output device that plays bit-perfect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BitPerfectOutput {
    pub device_id: String,
    pub device_name: String,
}

/// The correction for `device_id` (None: none set, or no device).
pub fn correction_for(s: &Settings, device_id: Option<&str>) -> Option<Correction> {
    let id = device_id?;
    s.headphone_correction
        .iter()
        .find(|c| c.device_id == id)
        .filter(|c| !c.filters.is_empty())
        .map(|c| Correction::new(c.preamp_db, &c.filters))
}

/// Device ids that play bit-perfect.
pub fn bit_perfect_ids(s: &Settings) -> Vec<String> {
    s.bit_perfect_outputs.iter().map(|b| b.device_id.clone()).collect()
}

/// Binds a profile to `device` (None removes its correction). Returns
/// whether anything changed.
pub fn set_correction(s: &mut Settings, device: &OutputDevice, profile: Option<(Entry, Parsed)>) -> bool {
    let before = s.headphone_correction.clone();
    s.headphone_correction.retain(|c| c.device_id != device.id);
    if let Some((e, p)) = profile {
        s.headphone_correction.push(DeviceCorrection {
            device_id: device.id.clone(),
            device_name: device.name.clone(),
            name: e.name,
            source: e.source,
            form: e.form,
            path: e.path,
            preamp_db: p.preamp_db,
            filters: p.filters,
        });
    }
    s.headphone_correction != before
}

/// Turns bit-perfect output on or off for `device`. Returns whether
/// anything changed.
pub fn set_bit_perfect(s: &mut Settings, device: &OutputDevice, on: bool) -> bool {
    let had = s.bit_perfect_outputs.iter().any(|b| b.device_id == device.id);
    if had == on {
        return false;
    }
    s.bit_perfect_outputs.retain(|b| b.device_id != device.id);
    if on {
        s.bit_perfect_outputs.push(BitPerfectOutput { device_id: device.id.clone(), device_name: device.name.clone() });
    }
    true
}

async fn current_device(player: &PlayerHandle) -> AppResult<OutputDevice> {
    player
        .output_state()
        .await?
        .current
        .filter(|d| !d.id.starts_with(super::netsink::NET_PREFIX))
        .ok_or_else(|| AppError::NotFound("No output device on this computer is in use.".into()))
}

/// Headphones in the bundled index whose name matches `query`.
#[tauri::command]
pub async fn headphone_search(query: String) -> AppResult<Vec<Entry>> {
    Ok(autoeq::search(&query, autoeq::SEARCH_LIMIT))
}

/// Uses the profile at `path` (from `headphone_search`) whenever the
/// output device in use plays; `path: null` turns correction off for it.
/// The profile is fetched the first time and kept on disk.
#[tauri::command]
pub async fn headphone_correction_set(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    fetcher: State<'_, Fetcher>,
    path: Option<String>,
) -> AppResult<Settings> {
    let device = current_device(&player).await?;
    let profile = match path {
        Some(p) => {
            let entry = autoeq::find(&p).ok_or_else(|| AppError::NotFound(format!("headphone {p}")))?;
            let parsed = fetcher.profile(&entry.path).await?;
            Some((entry, parsed))
        }
        None => None,
    };
    let s = match settings.modify(|s| Ok(set_correction(s, &device, profile))).await? {
        Some(s) => s,
        None => settings.get().await?,
    };
    player.set_dsp(s.dsp()).await?;
    Ok(s)
}

/// A/B: plays without the headphone correction while `bypass` is true.
/// Not saved; the Settings row turns it back off when it closes.
#[tauri::command]
pub async fn headphone_correction_compare(player: State<'_, PlayerHandle>, bypass: bool) -> AppResult<()> {
    player.set_correction_bypass(bypass).await
}

/// Bit-perfect output on or off for the output device in use. The stream
/// reopens at once, keeping position and play state.
#[tauri::command]
pub async fn bit_perfect_set(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    on: bool,
) -> AppResult<Settings> {
    let device = current_device(&player).await?;
    let s = match settings.modify(|s| Ok(set_bit_perfect(s, &device, on))).await? {
        Some(s) => s,
        None => settings.get().await?,
    };
    player.set_exclusive_outputs(bit_perfect_ids(&s)).await?;
    Ok(s)
}

/// After the output device changed: its correction and bit-perfect choice.
pub async fn apply_for_output(player: &PlayerHandle, s: &Settings) -> AppResult<OutputState> {
    player.set_dsp(s.dsp()).await?;
    player.set_exclusive_outputs(bit_perfect_ids(s)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::FilterKind;

    fn dev(id: &str) -> OutputDevice {
        OutputDevice { id: id.into(), name: format!("{id} name"), is_default: false }
    }

    fn profile(name: &str, gain: f32) -> (Entry, Parsed) {
        (
            Entry { name: name.into(), source: "lab".into(), form: "over-ear".into(), path: format!("lab/over-ear/{name}") },
            Parsed {
                preamp_db: -gain,
                filters: vec![PeqFilter { kind: FilterKind::Peaking, freq: 3000.0, gain_db: gain, q: 1.0 }],
            },
        )
    }

    #[test]
    fn each_device_has_its_own_correction() {
        let mut s = Settings::default();
        assert!(set_correction(&mut s, &dev("hp"), Some(profile("Phones A", 4.0))));
        assert!(set_correction(&mut s, &dev("usb"), Some(profile("Phones B", 2.0))));
        assert_eq!(correction_for(&s, Some("hp")).unwrap().preamp_db, -4.0);
        assert_eq!(correction_for(&s, Some("usb")).unwrap().preamp_db, -2.0);
        assert_eq!(correction_for(&s, Some("spk")), None);
        assert_eq!(correction_for(&s, None), None);
        // Picking another profile replaces it; off removes it.
        assert!(set_correction(&mut s, &dev("hp"), Some(profile("Phones C", 3.0))));
        assert_eq!(s.headphone_correction.iter().filter(|c| c.device_id == "hp").count(), 1);
        assert!(set_correction(&mut s, &dev("hp"), None));
        assert!(!set_correction(&mut s, &dev("hp"), None));
        assert_eq!(correction_for(&s, Some("hp")), None);
    }

    #[test]
    fn the_player_gets_the_correction_of_the_device_in_use() {
        let mut s = Settings::default();
        set_correction(&mut s, &dev("hp"), Some(profile("Phones A", 4.0)));
        s.eq_last_output = Some("hp".into());
        assert!(s.dsp().correction.is_some());
        s.eq_last_output = Some("spk".into());
        assert!(s.dsp().correction.is_none());
    }

    #[test]
    fn bit_perfect_is_per_device() {
        let mut s = Settings::default();
        assert!(bit_perfect_ids(&s).is_empty());
        assert!(set_bit_perfect(&mut s, &dev("dac"), true));
        assert!(!set_bit_perfect(&mut s, &dev("dac"), true));
        assert!(set_bit_perfect(&mut s, &dev("hp"), true));
        assert_eq!(bit_perfect_ids(&s), ["dac", "hp"]);
        assert!(set_bit_perfect(&mut s, &dev("dac"), false));
        assert_eq!(bit_perfect_ids(&s), ["hp"]);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let mut s = Settings::default();
        set_correction(&mut s, &dev("hp"), Some(profile("Phones A", 4.0)));
        set_bit_perfect(&mut s, &dev("dac"), true);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["headphoneCorrection"][0]["filters"][0]["kind"], "peaking");
        assert_eq!(v["bitPerfectOutputs"][0]["deviceId"], "dac");
        let back: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
