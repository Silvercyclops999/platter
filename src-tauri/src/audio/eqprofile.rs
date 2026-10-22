//! EQ presets, saved profiles and profiles bound to output devices. Pure
//! functions over [`Settings`]; the commands that call them save the
//! result and pass it to the player.

use serde::{Deserialize, Serialize};

use super::dsp::{CROSSFADE_MAX_S, CROSSFADE_MIN_S, Crossfade, DspSettings, EQ_BANDS, EQ_MAX_DB, PREAMP_MAX_DB};
use super::output::OutputDevice;
use crate::error::{AppError, AppResult};
use crate::settings::{Settings, SettingsPatch};

pub type Bands = [f32; EQ_BANDS];

/// A curve the user saved under a name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqProfile {
    pub name: String,
    pub bands: Bands,
}

/// A preset or profile used whenever this output device plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqOutput {
    pub device_id: String,
    /// Shown in Settings even while the device is unplugged.
    pub device_name: String,
    /// A preset id or `profile:<name>`.
    pub preset: String,
}

/// The EQ choice to go back to when leaving a device with its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqChoice {
    pub preset: String,
    pub bands: Bands,
}

pub const CUSTOM: &str = "custom";
const PROFILE: &str = "profile:";
pub const NAME_MAX: usize = 40;

/// Built-in presets: (id, name, bands from 31 Hz to 16 kHz).
pub const PRESETS: &[(&str, &str, Bands)] = &[
    ("flat", "Flat", [0.0; EQ_BANDS]),
    ("bassBoost", "Bass boost", [6.0, 5.0, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("trebleBoost", "Treble boost", [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 4.0, 5.0, 6.0]),
    ("vocal", "Vocal", [-2.0, -2.0, -1.0, 0.0, 2.0, 3.5, 3.5, 2.0, 0.0, -1.0]),
    ("loudness", "Loudness", [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 1.5, 3.5, 4.5]),
    ("smallSpeakers", "Small speakers", [-8.0, -4.0, 2.0, 3.0, 1.5, 0.0, 0.0, 1.0, 2.0, 1.0]),
    ("lateNight", "Late night", [-5.0, -4.0, -2.0, -0.5, 0.0, 1.0, 1.5, 1.0, 0.0, -1.0]),
];

pub fn profile_id(name: &str) -> String {
    format!("{PROFILE}{name}")
}

fn find_profile<'a>(s: &'a Settings, name: &str) -> Option<&'a EqProfile> {
    s.eq_profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

/// The bands of a preset or profile id (None: unknown or custom).
pub fn bands_of(s: &Settings, id: &str) -> Option<Bands> {
    if let Some(name) = id.strip_prefix(PROFILE) {
        return find_profile(s, name).map(|p| p.bands);
    }
    PRESETS.iter().find(|p| p.0 == id).map(|p| p.2)
}

fn clamp_bands(b: Bands) -> Bands {
    b.map(|v| if v.is_finite() { (v * 10.0).round() / 10.0 } else { 0.0 }.clamp(-EQ_MAX_DB, EQ_MAX_DB))
}

/// Makes a preset or profile the active curve.
pub fn select(s: &mut Settings, id: &str) -> AppResult<()> {
    if id == CUSTOM {
        s.eq_preset = CUSTOM.into();
        return Ok(());
    }
    let bands = bands_of(s, id).ok_or_else(|| AppError::NotFound(format!("EQ preset {id}")))?;
    s.eq_preset = id.into();
    s.eq_bands = bands;
    Ok(())
}

/// The audio part of a settings patch.
pub fn apply_patch(s: &mut Settings, p: &SettingsPatch) -> AppResult<()> {
    if let Some(m) = p.crossfade {
        s.crossfade = m;
    }
    if let Some(n) = p.crossfade_seconds {
        s.crossfade_seconds = n;
    }
    if let Some(v) = p.crossfade_skip_albums {
        s.crossfade_skip_albums = v;
    }
    if let Some(v) = p.leveling {
        s.leveling = v;
    }
    if let Some(v) = p.preamp_db {
        if !v.is_finite() {
            return Err(AppError::Invalid("preamp must be a number".into()));
        }
        s.preamp_db = v;
    }
    if let Some(v) = p.tune_headphones {
        s.tune_headphones = v;
    }
    if let Some(v) = p.limiter {
        s.limiter = v;
    }
    if let Some(v) = p.eq_enabled {
        s.eq_enabled = v;
    }
    if let Some(id) = &p.eq_preset {
        select(s, id)?;
    }
    // Moving a band makes the curve custom.
    if let Some(b) = p.eq_bands {
        s.eq_bands = clamp_bands(b);
        if bands_of(s, &s.eq_preset) != Some(s.eq_bands) {
            s.eq_preset = CUSTOM.into();
        }
    }
    clamp(s);
    Ok(())
}

/// Keeps audio values in range (also fixes hand-edited stored values).
pub fn clamp(s: &mut Settings) {
    s.crossfade_seconds = s.crossfade_seconds.clamp(CROSSFADE_MIN_S, CROSSFADE_MAX_S);
    s.preamp_db = if s.preamp_db.is_finite() { s.preamp_db.clamp(-PREAMP_MAX_DB, PREAMP_MAX_DB) } else { 0.0 };
    s.eq_bands = clamp_bands(s.eq_bands);
    if s.eq_preset != CUSTOM && bands_of(s, &s.eq_preset).is_none() {
        s.eq_preset = CUSTOM.into();
    }
}

/// What the player's sound stage does with these settings.
pub fn dsp(s: &Settings) -> DspSettings {
    use super::dsp::CrossfadeMode as M;
    DspSettings {
        crossfade: match s.crossfade {
            M::Off => Crossfade::Off,
            M::Smart => Crossfade::Smart,
            M::Fixed => Crossfade::Fixed {
                ms: s.crossfade_seconds as u64 * 1000,
            },
        },
        album_gapless: s.crossfade_skip_albums,
        leveling: s.leveling,
        preamp_db: s.preamp_db,
        limiter: s.limiter,
        eq: s.eq_enabled.then_some(s.eq_bands),
        correction: super::hifi::correction_for(s, s.eq_last_output.as_deref()),
    }
}

fn check_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("Give the profile a name.".into()));
    }
    if name.chars().count() > NAME_MAX {
        return Err(AppError::Invalid(format!("Profile names can be up to {NAME_MAX} characters.")));
    }
    if PRESETS.iter().any(|p| p.1.eq_ignore_ascii_case(name)) || name.eq_ignore_ascii_case("custom") {
        return Err(AppError::Invalid(format!("{name} is a built-in preset. Pick another name.")));
    }
    Ok(name.to_string())
}

/// Saves the current curve as a profile (replacing one with that name)
/// and selects it.
pub fn save_as(s: &mut Settings, name: &str) -> AppResult<()> {
    let name = check_name(name)?;
    let bands = s.eq_bands;
    match s.eq_profiles.iter_mut().find(|p| p.name.eq_ignore_ascii_case(&name)) {
        Some(p) => {
            p.name = name.clone();
            p.bands = bands;
        }
        None => s.eq_profiles.push(EqProfile { name: name.clone(), bands }),
    }
    s.eq_preset = profile_id(&name);
    Ok(())
}

/// Renames a profile everywhere it's used.
pub fn rename(s: &mut Settings, from: &str, to: &str) -> AppResult<()> {
    let to = check_name(to)?;
    let old = find_profile(s, from)
        .map(|p| p.name.clone())
        .ok_or_else(|| AppError::NotFound(format!("EQ profile {from}")))?;
    if !old.eq_ignore_ascii_case(&to) && find_profile(s, &to).is_some() {
        return Err(AppError::Invalid(format!("There's already a profile called {to}.")));
    }
    let (old_id, new_id) = (profile_id(&old), profile_id(&to));
    for p in s.eq_profiles.iter_mut().filter(|p| p.name == old) {
        p.name = to.clone();
    }
    for o in s.eq_outputs.iter_mut().filter(|o| o.preset == old_id) {
        o.preset = new_id.clone();
    }
    if s.eq_preset == old_id {
        s.eq_preset = new_id.clone();
    }
    if let Some(f) = s.eq_fallback.as_mut().filter(|f| f.preset == old_id) {
        f.preset = new_id;
    }
    Ok(())
}

/// Deletes a profile. Outputs using it lose their binding; if it's the
/// active curve, the curve stays as custom.
pub fn delete(s: &mut Settings, name: &str) -> AppResult<()> {
    let old = find_profile(s, name)
        .map(|p| p.name.clone())
        .ok_or_else(|| AppError::NotFound(format!("EQ profile {name}")))?;
    let id = profile_id(&old);
    s.eq_profiles.retain(|p| p.name != old);
    s.eq_outputs.retain(|o| o.preset != id);
    if s.eq_preset == id {
        s.eq_preset = CUSTOM.into();
    }
    if let Some(f) = s.eq_fallback.as_mut().filter(|f| f.preset == id) {
        f.preset = CUSTOM.into();
    }
    Ok(())
}

/// Binds the active preset or profile to `device` (or unbinds it).
pub fn use_for_output(s: &mut Settings, device: &OutputDevice, on: bool) -> AppResult<()> {
    s.eq_outputs.retain(|o| o.device_id != device.id);
    if !on {
        return Ok(());
    }
    if s.eq_preset == CUSTOM {
        return Err(AppError::Invalid("Save this curve as a profile first.".into()));
    }
    s.eq_outputs.push(EqOutput {
        device_id: device.id.clone(),
        device_name: device.name.clone(),
        preset: s.eq_preset.clone(),
    });
    Ok(())
}

fn bound_preset(s: &Settings, device_id: &str) -> Option<String> {
    s.eq_outputs
        .iter()
        .find(|o| o.device_id == device_id)
        .map(|o| o.preset.clone())
        .filter(|id| bands_of(s, id).is_some())
}

/// The device in use changed: switch to its bound preset or profile, or to
/// the choice for devices without one. Leaving a device without one keeps
/// what was in use there as that choice. Returns whether the EQ changed.
pub fn output_changed(s: &mut Settings, device: Option<&OutputDevice>) -> bool {
    let id = device.map(|d| d.id.clone());
    if s.eq_last_output.is_some() && s.eq_last_output == id {
        return false;
    }
    let left_unbound = s.eq_last_output.as_deref().is_none_or(|old| bound_preset(s, old).is_none());
    if left_unbound {
        s.eq_fallback = Some(EqChoice {
            preset: s.eq_preset.clone(),
            bands: s.eq_bands,
        });
    }
    s.eq_last_output = id;
    let (preset, bands) = (s.eq_preset.clone(), s.eq_bands);
    match device.and_then(|d| bound_preset(s, &d.id)) {
        Some(p) => {
            let _ = select(s, &p);
        }
        None => {
            if let Some(f) = s.eq_fallback.clone() {
                s.eq_preset = f.preset;
                s.eq_bands = f.bands;
            }
        }
    }
    s.eq_preset != preset || s.eq_bands != bands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str) -> OutputDevice {
        OutputDevice {
            id: id.into(),
            name: format!("Device {id}"),
            is_default: false,
        }
    }

    #[test]
    fn presets_select_and_moving_a_band_makes_it_custom() {
        let mut s = Settings::default();
        assert_eq!(s.eq_preset, "flat");
        select(&mut s, "bassBoost").unwrap();
        assert_eq!(s.eq_bands[0], 6.0);
        let mut b = s.eq_bands;
        b[3] = 30.0;
        apply_patch(&mut s, &SettingsPatch { eq_bands: Some(b), ..Default::default() }).unwrap();
        assert_eq!(s.eq_preset, CUSTOM);
        assert_eq!(s.eq_bands[3], 12.0, "clamped");
        assert!(select(&mut s, "nope").is_err());
        // Every preset stays in range and is our own named set.
        for (_, _, b) in PRESETS {
            assert!(b.iter().all(|v| v.abs() <= EQ_MAX_DB));
        }
    }

    #[test]
    fn profiles_save_rename_and_delete_everywhere() {
        let mut s = Settings {
            eq_bands: [1.0; EQ_BANDS],
            eq_preset: CUSTOM.into(),
            ..Default::default()
        };
        save_as(&mut s, " Desk ").unwrap();
        assert_eq!(s.eq_preset, "profile:Desk");
        assert!(save_as(&mut s, "Flat").is_err(), "built-in names are taken");
        assert!(save_as(&mut s, "").is_err());
        use_for_output(&mut s, &dev("usb"), true).unwrap();
        rename(&mut s, "desk", "Studio").unwrap();
        assert_eq!(s.eq_preset, "profile:Studio");
        assert_eq!(s.eq_outputs[0].preset, "profile:Studio");
        save_as(&mut s, "Other").unwrap();
        assert!(rename(&mut s, "Other", "studio").is_err());
        delete(&mut s, "Studio").unwrap();
        assert!(s.eq_outputs.is_empty());
        assert_eq!(s.eq_profiles.len(), 1);
        assert!(delete(&mut s, "Studio").is_err());
    }

    #[test]
    fn custom_curves_must_be_saved_before_binding() {
        let mut s = Settings {
            eq_preset: CUSTOM.into(),
            ..Default::default()
        };
        assert!(use_for_output(&mut s, &dev("usb"), true).is_err());
    }

    #[test]
    fn switching_output_switches_profile_and_back() {
        let mut s = Settings {
            eq_bands: [2.0; EQ_BANDS],
            ..Default::default()
        };
        save_as(&mut s, "Headphones").unwrap();
        // Vocal on the speakers.
        select(&mut s, "vocal").unwrap();
        assert!(!output_changed(&mut s, Some(&dev("speakers"))));
        // Over on the headphones the user picks their profile and binds it.
        assert!(!output_changed(&mut s, Some(&dev("hp"))));
        select(&mut s, "profile:Headphones").unwrap();
        use_for_output(&mut s, &dev("hp"), true).unwrap();
        // Back on the speakers: Vocal again.
        assert!(output_changed(&mut s, Some(&dev("speakers"))));
        assert_eq!(s.eq_preset, "vocal");
        // Changing it there is remembered for unbound devices.
        select(&mut s, "lateNight").unwrap();
        assert!(output_changed(&mut s, Some(&dev("hp"))));
        assert_eq!(s.eq_preset, "profile:Headphones");
        assert_eq!(s.eq_bands, [2.0; EQ_BANDS]);
        assert!(!output_changed(&mut s, Some(&dev("hp"))), "already on it");
        // No device known counts as unbound.
        assert!(output_changed(&mut s, None));
        assert_eq!(s.eq_preset, "lateNight");
        assert!(!output_changed(&mut s, Some(&dev("usb"))));
    }

    #[test]
    fn dsp_follows_settings() {
        let mut s = Settings::default();
        let d = dsp(&s);
        assert_eq!(d.crossfade, Crossfade::Smart);
        assert!(d.limiter && d.album_gapless && d.eq.is_none());
        apply_patch(
            &mut s,
            &SettingsPatch {
                crossfade: Some(super::super::dsp::CrossfadeMode::Fixed),
                crossfade_seconds: Some(40),
                preamp_db: Some(-30.0),
                eq_enabled: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let d = dsp(&s);
        assert_eq!(d.crossfade, Crossfade::Fixed { ms: 12_000 });
        assert_eq!(d.preamp_db, -12.0);
        assert_eq!(d.eq, Some([0.0; EQ_BANDS]));
        assert!(apply_patch(&mut s, &SettingsPatch { preamp_db: Some(f32::NAN), ..Default::default() }).is_err());
    }
}
