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
    /// Picked by Platter when the headphones connected, not by hand.
    #[serde(default)]
    pub auto: bool,
}

/// An output device that plays bit-perfect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BitPerfectOutput {
    pub device_id: String,
    pub device_name: String,
}

/// The correction `device_id` uses (None: none set, or no device). One
/// Platter picked by itself counts only while Tune for my headphones is on.
pub fn binding_for<'a>(s: &'a Settings, device_id: Option<&str>) -> Option<&'a DeviceCorrection> {
    let id = device_id?;
    s.headphone_correction.iter().find(|c| c.device_id == id && (!c.auto || s.tune_headphones))
}

/// The correction for `device_id` (None: none set, or no device).
pub fn correction_for(s: &Settings, device_id: Option<&str>) -> Option<Correction> {
    binding_for(s, device_id)
        .filter(|c| !c.filters.is_empty())
        .map(|c| Correction::new(c.preamp_db, &c.filters))
}

/// Device ids that play bit-perfect.
pub fn bit_perfect_ids(s: &Settings) -> Vec<String> {
    s.bit_perfect_outputs.iter().map(|b| b.device_id.clone()).collect()
}

/// Binds a profile to `device` by hand (None removes its correction).
/// Turning it off also stops Platter picking one for that device again;
/// picking one lets it. Returns whether anything changed.
pub fn set_correction(s: &mut Settings, device: &OutputDevice, profile: Option<(Entry, Parsed)>) -> bool {
    let declined_before = s.headphone_declined.clone();
    s.headphone_declined.retain(|id| *id != device.id);
    if profile.is_none() {
        s.headphone_declined.push(device.id.clone());
    }
    bind(s, device, profile, false) || s.headphone_declined != declined_before
}

fn bind(s: &mut Settings, device: &OutputDevice, profile: Option<(Entry, Parsed)>, auto: bool) -> bool {
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
            auto,
        });
    }
    s.headphone_correction != before
}

/// Whether Platter may tune `device` by itself: headphones (by the phone's
/// route, or by a computer output's name) never set by hand and never
/// turned off there. The phone's speaker, the car, AirPlay and the like
/// never are.
pub fn may_tune(s: &Settings, device: &OutputDevice) -> bool {
    if !s.tune_headphones || device.id.starts_with(super::netsink::NET_PREFIX) {
        return false;
    }
    if let Some(rest) = device.id.strip_prefix(ROUTE_PREFIX) {
        let kind = rest.split(':').next().unwrap_or("");
        if !TUNED_ROUTES.contains(&kind) {
            return false;
        }
    }
    // Bit-perfect plays the file untouched: a correction there would do nothing.
    !s.headphone_correction.iter().any(|c| c.device_id == device.id)
        && !s.headphone_declined.contains(&device.id)
        && !s.bit_perfect_outputs.iter().any(|b| b.device_id == device.id)
}

/// Phone routes that can be headphones (a Bluetooth speaker or car stereo
/// isn't in the headphone list, so its name finds nothing).
const TUNED_ROUTES: [&str; 3] = ["headphones", "bluetooth", "usb"];

/// Ids of the phone's audio routes: `route:<kind>:<the port's own id>`.
pub const ROUTE_PREFIX: &str = "route:";

/// The headphones `device` is, when Platter may tune it and knows the model.
pub fn auto_pick(s: &Settings, device: &OutputDevice) -> Option<Entry> {
    if !may_tune(s, device) {
        return None;
    }
    autoeq::model_match(&headphone_query(&device.name))
}

/// Binds a profile Platter picked, unless meanwhile the sound moved to
/// another output, the switch went off, or one was set by hand. Returns
/// whether it was bound.
pub fn bind_auto(s: &mut Settings, device: &OutputDevice, profile: (Entry, Parsed)) -> bool {
    if s.eq_last_output.as_deref() != Some(device.id.as_str()) || !may_tune(s, device) {
        return false;
    }
    bind(s, device, Some(profile), true)
}

/// Woken when Tune for my headphones is turned on, so the output in use is
/// looked at again.
pub static RETUNE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Said when Platter tuned the output in use for its headphones.
pub const EV_TUNED: &str = "audio://tuned";

/// The `audio://tuned` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tuned {
    pub device_id: String,
    /// The headphone, as the index names it.
    pub name: String,
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

/// What the output is called, cut down to the headphones' own name: an
/// owner in front ("Adam's AirPods Pro") and the type of connection
/// ("Bluetooth", "Stereo") don't name a model.
pub fn headphone_query(output_name: &str) -> String {
    let name = output_name.replace('\u{2019}', "'");
    let name = match name.find("'s ") {
        Some(i) if !name[..i].contains(' ') => &name[i + 3..],
        _ => &name[..],
    };
    // Windows says "Headphones (2- WH-1000XM5 Stereo)": the brackets and
    // the "2-" go, and so does a "(2)" for a second one.
    const NOISE: [&str; 6] = ["bluetooth", "stereo", "hands-free", "headphones", "(2)", "(3)"];
    let numbered = |w: &str| w.strip_suffix('-').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let noise = |w: &str| NOISE.contains(&w.to_ascii_lowercase().as_str());
    name.split_whitespace()
        .filter(|w| !noise(w))
        .map(|w| w.trim_matches(['(', ')']))
        .filter(|w| !w.is_empty() && !numbered(w) && !noise(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The output playing now and the headphones it most likely is.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadphoneGuess {
    /// The output's own name ("Adam's AirPods Pro").
    pub output_name: String,
    /// Best matches first; empty for a speaker or an unknown model.
    pub matches: Vec<Entry>,
}

/// Finds the headphones in use: on a phone from the system's audio route
/// (only headphones and Bluetooth count, not the speaker), on a computer
/// from the output device's name.
#[tauri::command]
pub async fn headphone_detect(player: State<'_, PlayerHandle>) -> AppResult<HeadphoneGuess> {
    #[cfg(target_os = "ios")]
    let name = {
        let _ = &player;
        crate::platform::audio_session::headphones_name().unwrap_or_default()
    };
    #[cfg(not(target_os = "ios"))]
    let name = current_device(&player).await.map(|d| d.name).unwrap_or_default();
    let query = headphone_query(&name);
    let matches = if query.is_empty() { Vec::new() } else { autoeq::search(&query, 5) };
    Ok(HeadphoneGuess { output_name: name, matches })
}

/// Headphones in the bundled index whose name matches `query`.
#[tauri::command]
pub async fn headphone_search(query: String) -> AppResult<Vec<Entry>> {
    Ok(autoeq::search(&query, autoeq::SEARCH_LIMIT))
}

/// Uses the profile at `path` (from `headphone_search`) whenever the
/// output device in use plays; `path: null` turns correction off for it.
/// The profile is fetched the first time and kept on disk. With
/// `device_id` (Undo on "Tuned for"), only while that device is in use.
#[tauri::command]
pub async fn headphone_correction_set(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    fetcher: State<'_, Fetcher>,
    path: Option<String>,
    device_id: Option<String>,
) -> AppResult<Settings> {
    let device = current_device(&player).await?;
    if device_id.is_some_and(|id| id != device.id) {
        return Err(AppError::Invalid("Those headphones aren't playing any more.".into()));
    }
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

    fn named(id: &str, name: &str) -> OutputDevice {
        OutputDevice { id: id.into(), name: name.into(), is_default: true }
    }

    fn playing_on(s: &mut Settings, d: &OutputDevice) {
        s.eq_last_output = Some(d.id.clone());
    }

    #[test]
    fn headphones_are_known_by_their_model() {
        let s = Settings::default();
        let pods = named("route:bluetooth:AA-tacl", "Adam\u{2019}s AirPods Max");
        assert_eq!(auto_pick(&s, &pods).unwrap().name, "Apple AirPods Max");
        let xm5 = named("{0.0.0.00000000}.{x}", "Headphones (2- WH-1000XM5 Stereo)");
        assert_eq!(auto_pick(&s, &xm5).unwrap().name, "Sony WH-1000XM5");
    }

    #[test]
    fn the_most_trusted_measurement_wins() {
        // HD 650 was measured by several people; oratory1990 comes first.
        let e = autoeq::model_match("Sennheiser HD 650").unwrap();
        assert!(e.source.starts_with("oratory1990"), "{}", e.source);
    }

    #[test]
    fn an_unknown_or_blank_name_tunes_nothing() {
        let s = Settings::default();
        for name in ["", "Adam's Phones", "Speakers (Realtek(R) Audio)", "MacBook Pro Speakers", "Living Room"] {
            assert_eq!(auto_pick(&s, &named("dev", name)), None, "{name}");
        }
    }

    #[test]
    fn a_name_that_fits_several_models_tunes_nothing() {
        // "AirPods" starts many models but is none of them exactly.
        assert_eq!(autoeq::model_match("AirPods"), None);
        assert_eq!(autoeq::model_match("Apple"), None);
        // The phone calls AirPods Pro 2 "AirPods Pro" too: which one can't be told.
        assert_eq!(autoeq::model_match("AirPods Pro"), None);
        assert_eq!(autoeq::model_match("AirPods Pro 2").unwrap().name.to_lowercase(), "apple airpods pro 2");
        assert!(!autoeq::search("AirPods", 5).is_empty());
    }

    #[test]
    fn the_speaker_the_car_and_airplay_are_never_tuned() {
        let s = Settings::default();
        for id in ["route:speaker:Speaker", "route:car:c1", "route:airplay:tv", "route:receiver:r", "net:cast:1"] {
            assert!(!may_tune(&s, &named(id, "AirPods Pro")), "{id}");
            assert_eq!(auto_pick(&s, &named(id, "AirPods Pro")), None, "{id}");
        }
        for id in ["route:bluetooth:b", "route:headphones:w", "route:usb:u", "cpal-device"] {
            assert!(may_tune(&s, &named(id, "AirPods Pro")), "{id}");
        }
    }

    #[test]
    fn a_picked_correction_plays_only_on_its_headphones() {
        let mut s = Settings::default();
        let pods = named("route:bluetooth:AA", "AirPods Pro");
        let speaker = named("route:speaker:Speaker", "Speaker");
        playing_on(&mut s, &pods);
        assert!(bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0)));
        assert!(s.headphone_correction[0].auto);
        assert!(s.dsp().correction.is_some());
        playing_on(&mut s, &speaker);
        assert!(s.dsp().correction.is_none());
        // Once picked, it isn't picked again.
        playing_on(&mut s, &pods);
        assert!(!may_tune(&s, &pods));
        assert!(!bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0)));
    }

    #[test]
    fn a_profile_landing_after_the_sound_moved_is_dropped() {
        let mut s = Settings::default();
        let pods = named("route:bluetooth:AA", "AirPods Pro");
        playing_on(&mut s, &named("route:speaker:Speaker", "Speaker"));
        assert!(!bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0)));
        assert!(s.headphone_correction.is_empty());
    }

    #[test]
    fn bit_perfect_headphones_are_left_alone() {
        let mut s = Settings::default();
        let dac = named("usb-dac", "Sennheiser HD 650");
        assert!(may_tune(&s, &dac));
        set_bit_perfect(&mut s, &dac, true);
        assert!(!may_tune(&s, &dac));
    }

    #[tokio::test]
    async fn a_failed_fetch_leaves_it_to_try_again() {
        let s = Settings::default();
        let max = named("route:bluetooth:AA", "Adam's AirPods Max");
        let entry = auto_pick(&s, &max).unwrap();
        // Offline (here: a host profiles never come from), so nothing is fetched.
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_base(dir.path().into(), "https://10.1.2.3/", autoeq::ALLOWED_HOSTS);
        assert!(fetcher.profile(&entry.path).await.is_err());
        // Nothing was bound or declined, so the next time they play it's picked again.
        assert!(s.headphone_correction.is_empty() && s.headphone_declined.is_empty());
        assert_eq!(auto_pick(&s, &max), Some(entry));
    }

    #[test]
    fn turning_it_off_by_hand_is_kept() {
        let mut s = Settings::default();
        let pods = named("route:bluetooth:AA", "AirPods Pro");
        playing_on(&mut s, &pods);
        bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0));
        // Undo, or the switch off in Settings.
        assert!(set_correction(&mut s, &pods, None));
        assert!(s.headphone_correction.is_empty());
        assert_eq!(auto_pick(&s, &pods), None);
        assert!(!bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0)));
        // Picking one by hand lets it be theirs (and not Platter's).
        assert!(set_correction(&mut s, &pods, Some(profile("Apple AirPods Pro 2", 2.0))));
        assert!(s.headphone_declined.is_empty());
        assert!(!s.headphone_correction[0].auto);
    }

    #[test]
    fn a_hand_picked_correction_is_never_replaced() {
        let mut s = Settings::default();
        let pods = named("route:bluetooth:AA", "AirPods Pro");
        playing_on(&mut s, &pods);
        set_correction(&mut s, &pods, Some(profile("Apple AirPods Pro 2", 2.0)));
        assert_eq!(auto_pick(&s, &pods), None);
        assert!(!bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0)));
        assert_eq!(s.headphone_correction[0].name, "Apple AirPods Pro 2");
    }

    #[test]
    fn the_switch_off_stops_picked_ones_but_not_hand_picked_ones() {
        let mut s = Settings::default();
        let pods = named("route:bluetooth:AA", "AirPods Pro");
        let hd = named("usb-dac", "HD 650");
        playing_on(&mut s, &pods);
        bind_auto(&mut s, &pods, profile("Apple AirPods Pro", 3.0));
        set_correction(&mut s, &hd, Some(profile("Sennheiser HD 650", 2.0)));
        s.tune_headphones = false;
        assert!(s.dsp().correction.is_none());
        assert_eq!(auto_pick(&s, &named("route:bluetooth:BB", "AirPods Max")), None);
        playing_on(&mut s, &hd);
        assert!(s.dsp().correction.is_some());
        // On again: the picked one comes back.
        s.tune_headphones = true;
        playing_on(&mut s, &pods);
        assert!(s.dsp().correction.is_some());
    }

    #[test]
    fn older_saved_corrections_read_as_hand_picked() {
        let mut s = Settings::default();
        set_correction(&mut s, &dev("hp"), Some(profile("Phones A", 4.0)));
        let mut v = serde_json::to_value(&s).unwrap();
        v["headphoneCorrection"][0].as_object_mut().unwrap().remove("auto");
        let back: Settings = serde_json::from_value(v).unwrap();
        assert!(!back.headphone_correction[0].auto);
        assert!(back.tune_headphones);
    }

    #[test]
    fn output_names_come_down_to_the_model() {
        assert_eq!(headphone_query("Adam\u{2019}s AirPods Pro"), "AirPods Pro");
        assert_eq!(headphone_query("Adam's AirPods Max"), "AirPods Max");
        assert_eq!(headphone_query("WH-1000XM5 Stereo"), "WH-1000XM5");
        assert_eq!(headphone_query("Sennheiser HD 650"), "Sennheiser HD 650");
        assert_eq!(headphone_query("Living Room's Speaker Set"), "Living Room's Speaker Set");
        assert_eq!(headphone_query(""), "");
        assert_eq!(headphone_query("Headphones (2- WH-1000XM5 Stereo)"), "WH-1000XM5");
        assert_eq!(headphone_query("AirPods Pro (2)"), "AirPods Pro");
        assert!(super::autoeq::search(&headphone_query("Adam's AirPods Pro"), 5).iter().any(|e| e.name.contains("AirPods Pro")));
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
