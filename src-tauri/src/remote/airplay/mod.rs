//! AirPlay receivers as Listen on targets, over the original AirPlay audio
//! protocol (RAOP), which AirPlay 1 receivers and most AirPlay 2 speakers
//! still accept.
//!
//! Platter keeps decoding and plays into a network output (see
//! `audio::netsink`): the engine's sound goes to the receiver as Apple
//! Lossless at 44.1 kHz, about two seconds ahead of what's heard, and the
//! position shown is moved back by that delay.
//!
//! Not supported (listed, with the reason, but not playable):
//! - receivers that only take encrypted streams (RSA on the first AirPort
//!   Express, FairPlay or MFi authentication),
//! - receivers that require AirPlay 2 pairing or HomeKit access control
//!   (found out at connect time, they answer 403 or 470),
//! - video and screen AirPlay.
//!
//! - [`rtsp`]: the control connection and password (digest) auth.
//! - [`alac`]: uncompressed Apple Lossless frames.
//! - [`raop`]: the session (RTP audio, sync, timing, resends).
//! - [`link`]: following and controlling it as a [`super::lan::LanLink`].

pub mod alac;
pub mod link;
pub mod raop;
pub mod rtsp;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use super::lan::{MdnsAnswer, lan_player};
use super::types::{PlayerKind, PlayerVia, RemotePlayer};
use crate::audio::netsink::{NET_PREFIX, NetStream, NetTarget};
use crate::audio::output::{OutputDevice, OutputFormat};
use crate::error::AppResult;
use raop::{RaopConfig, RaopShared, RaopStream};

pub use link::AirPlayLink;

/// Keychain entry name for a receiver's password.
pub fn password_key(device_id: &str) -> String {
    format!("airplay-password:{device_id}")
}

/// An AirPlay receiver found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirPlayDevice {
    /// "airplay:<hardware address>".
    pub id: String,
    pub name: String,
    /// Apple model id or the maker's model, when given.
    pub model: Option<String>,
    pub addr: SocketAddr,
    /// The receiver has a password.
    pub password: bool,
    /// Why Platter can't play on it, if it can't.
    pub unsupported: Option<String>,
}

pub const UNSUPPORTED_ENCRYPTION: &str = "Needs AirPlay encryption Platter doesn't support";
pub const UNSUPPORTED_CODEC: &str = "Doesn't take Apple Lossless audio";
pub const UNSUPPORTED_PIN: &str = "Needs AirPlay pairing with a code";

/// Status flag: the receiver wants an on-screen code (pairing).
const SF_PIN: u64 = 0x8;

fn list_has(v: Option<&str>, item: &str) -> Option<bool> {
    v.map(|s| s.split(',').any(|x| x.trim() == item))
}

fn hex_or_dec(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// A `_raop._tcp` answer as a receiver. The instance name is
/// "<hardware address>@<name>".
pub fn from_mdns(a: &MdnsAnswer) -> Option<AirPlayDevice> {
    let addr = a.socket_addr()?;
    let (hw, name) = match a.instance.split_once('@') {
        Some((hw, name)) if !hw.is_empty() && !name.trim().is_empty() => (hw.to_string(), name.trim().to_string()),
        _ => (a.instance.clone(), a.instance.clone()),
    };
    let unsupported = if list_has(a.txt("et"), "0") == Some(false) {
        Some(UNSUPPORTED_ENCRYPTION)
    } else if list_has(a.txt("cn"), "1") == Some(false) {
        Some(UNSUPPORTED_CODEC)
    } else if a.txt("sf").and_then(hex_or_dec).is_some_and(|f| f & SF_PIN != 0) {
        Some(UNSUPPORTED_PIN)
    } else {
        None
    };
    Some(AirPlayDevice {
        id: format!("airplay:{hw}"),
        name,
        model: a.txt("am").map(str::to_string),
        addr,
        password: a.txt("pw").is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1"),
        unsupported: unsupported.map(str::to_string),
    })
}

impl AirPlayDevice {
    pub fn player(&self) -> RemotePlayer {
        let tv = self.model.as_deref().is_some_and(|m| m.to_ascii_lowercase().contains("appletv"));
        let kind = if tv { PlayerKind::Tv } else { PlayerKind::Speaker };
        let mut p = lan_player(self.id.clone(), self.name.clone(), "AirPlay".into(), kind, PlayerVia::AirPlay);
        p.unsupported = self.unsupported.clone();
        p
    }

    /// The output id while the engine plays on this receiver.
    pub fn output_id(&self) -> String {
        format!("{NET_PREFIX}{}", self.id)
    }
}

/// A receiver as a network output target.
pub struct AirPlayTarget {
    pub device: AirPlayDevice,
    cfg: Mutex<RaopConfig>,
    session: Mutex<Option<Arc<RaopShared>>>,
}

impl AirPlayTarget {
    pub fn new(device: AirPlayDevice, password: Option<String>, client_id: String, timeout: Duration, volume: f32) -> Self {
        let cfg = RaopConfig {
            addr: device.addr,
            password,
            timeout,
            client_id,
            volume,
        };
        Self {
            device,
            cfg: Mutex::new(cfg),
            session: Mutex::new(None),
        }
    }

    /// Checks the receiver answers and takes the password (blocking).
    pub fn probe(&self) -> AppResult<()> {
        let cfg = self.cfg.lock().clone();
        let mut r = rtsp::Rtsp::connect(cfg.addr, cfg.timeout, cfg.password, cfg.client_id)?;
        r.request("OPTIONS", "*", &[], None)?;
        Ok(())
    }

    /// The live session, if the output is open on it.
    pub fn session(&self) -> Option<Arc<RaopShared>> {
        self.session.lock().clone().filter(|s| !s.is_closed())
    }

    pub fn volume(&self) -> f32 {
        self.cfg.lock().volume
    }

    /// Receiver volume (blocking when a session is live).
    pub fn set_volume(&self, v: f32) -> AppResult<()> {
        self.cfg.lock().volume = v;
        match self.session() {
            Some(s) => s.set_volume(v),
            None => Ok(()),
        }
    }

    /// Delay between the engine and what's heard.
    pub fn delay(&self) -> Duration {
        self.session().map_or_else(
            || Duration::from_secs_f64(f64::from(raop::LATENCY_FRAMES) / f64::from(raop::SAMPLE_RATE)),
            |s| s.delay(),
        )
    }
}

impl NetTarget for AirPlayTarget {
    fn device(&self) -> OutputDevice {
        OutputDevice {
            id: self.device.output_id(),
            name: self.device.name.clone(),
            is_default: false,
        }
    }
    fn format(&self) -> OutputFormat {
        OutputFormat {
            sample_rate: raop::SAMPLE_RATE,
            channels: 2,
        }
    }
    fn connect(&self) -> AppResult<Box<dyn NetStream>> {
        let cfg = self.cfg.lock().clone();
        let (stream, shared) = RaopStream::open(&cfg)?;
        *self.session.lock() = Some(shared);
        Ok(Box::new(stream))
    }
}
