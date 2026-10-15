//! macOS: takes the device in hog mode (no other app can play on it) and
//! sets its nominal sample rate to the music's. cpal then plays on it at
//! that rate. The guard gives the device back.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress, AudioObjectSetPropertyData,
    kAudioDevicePropertyHogMode, kAudioDevicePropertyNominalSampleRate, kAudioHardwarePropertyTranslateUIDToDevice,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioObjectUnknown,
};
use objc2_core_foundation::CFString;

use super::Opened;

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Reads a fixed-size property.
fn get<T: Copy>(object: AudioObjectID, selector: u32, init: T) -> Result<T, i32> {
    let addr = address(selector);
    let mut value = init;
    let mut size = size_of::<T>() as u32;
    // SAFETY: every pointer is to a live local of the size given.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&addr),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    if status == 0 { Ok(value) } else { Err(status) }
}

fn set<T: Copy>(object: AudioObjectID, selector: u32, value: T) -> Result<(), i32> {
    let addr = address(selector);
    // SAFETY: as in `get`.
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            NonNull::from(&addr),
            0,
            std::ptr::null(),
            size_of::<T>() as u32,
            NonNull::from(&value).cast(),
        )
    };
    if status == 0 { Ok(()) } else { Err(status) }
}

fn device_for_uid(uid: &str) -> Option<AudioObjectID> {
    let cf = CFString::from_str(uid);
    let cf_ref: *const CFString = &*cf;
    let addr = address(kAudioHardwarePropertyTranslateUIDToDevice);
    let mut device: AudioObjectID = kAudioObjectUnknown;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: the qualifier is a CFStringRef that lives across the call.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&addr),
            size_of::<*const CFString>() as u32,
            (&cf_ref as *const *const CFString).cast::<c_void>(),
            NonNull::from(&mut size),
            NonNull::from(&mut device).cast(),
        )
    };
    (status == 0 && device != kAudioObjectUnknown).then_some(device)
}

/// Holds the device in hog mode until dropped.
pub struct DeviceGuard {
    device: AudioObjectID,
    old_rate: Option<f64>,
}

impl Drop for DeviceGuard {
    fn drop(&mut self) {
        if let Some(r) = self.old_rate {
            let _ = set(self.device, kAudioDevicePropertyNominalSampleRate, r);
        }
        let _ = set(self.device, kAudioDevicePropertyHogMode, -1i32);
    }
}

pub(super) fn open(uid: &str, rate: u32) -> Result<Opened, String> {
    let device = device_for_uid(uid).ok_or("The device wasn't found.")?;
    let me = std::process::id() as i32;
    let owner = get(device, kAudioDevicePropertyHogMode, -1i32).map_err(|s| format!("The device wouldn't say who holds it ({s})."))?;
    if owner != -1 && owner != me {
        return Err("Another app is using the device on its own.".into());
    }
    if owner != me {
        set(device, kAudioDevicePropertyHogMode, me).map_err(|s| format!("The device wouldn't open on its own ({s})."))?;
        if get(device, kAudioDevicePropertyHogMode, -1i32) != Ok(me) {
            return Err("Another app is using the device on its own.".into());
        }
    }
    let mut guard = DeviceGuard { device, old_rate: None };
    let before = get(device, kAudioDevicePropertyNominalSampleRate, 0f64).unwrap_or(0.0);
    if (before - rate as f64).abs() > 0.5 {
        let unsupported = || format!("The device can't play {} directly.", rate as f64 / 1000.0);
        set(device, kAudioDevicePropertyNominalSampleRate, rate as f64).map_err(|_| unsupported())?;
        guard.old_rate = Some(before);
        // The change lands a moment later.
        let mut now = 0.0;
        for _ in 0..50 {
            now = get(device, kAudioDevicePropertyNominalSampleRate, 0f64).unwrap_or(0.0);
            if (now - rate as f64).abs() <= 0.5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if (now - rate as f64).abs() > 0.5 {
            return Err(unsupported());
        }
    }
    Ok(Opened::Device { guard, rate, bits: 32 })
}
