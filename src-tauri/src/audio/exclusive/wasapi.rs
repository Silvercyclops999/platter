//! WASAPI exclusive-mode output, event driven. Everything here runs on the
//! output thread that opened it (the COM objects never leave it).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use windows::Win32::Foundation::{CloseHandle, HANDLE, RPC_E_CHANGED_MODE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_DEVICE_INVALIDATED,
    AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED, AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE_EXCLUSIVE,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{HSTRING, PCWSTR};

use super::{Opened, SampleLayout};
use crate::audio::output::{Renderer, StreamCtl};

/// Tried in this order: the widest integer formats first, so 24-bit music
/// keeps every bit.
const LAYOUTS: [SampleLayout; 5] =
    [SampleLayout::I32, SampleLayout::I24In32, SampleLayout::I24, SampleLayout::I16, SampleLayout::F32];

/// Speaker positions: front left and right, or front centre for mono.
fn channel_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,
        _ => 0x3,
    }
}

fn wave_format(layout: SampleLayout, rate: u32, channels: u16) -> WAVEFORMATEXTENSIBLE {
    let bytes = layout.bytes() as u16;
    let block = bytes * channels;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * block as u32,
            nBlockAlign: block,
            wBitsPerSample: bytes * 8,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: layout.valid_bits() },
        dwChannelMask: channel_mask(channels),
        SubFormat: if layout == SampleLayout::F32 { KSDATAFORMAT_SUBTYPE_IEEE_FLOAT } else { KSDATAFORMAT_SUBTYPE_PCM },
    }
}

/// A plain reason for the fallback note.
fn reason(e: &windows::core::Error, rate: u32) -> String {
    let code = e.code();
    if code == AUDCLNT_E_DEVICE_IN_USE {
        "Another app is using the device on its own.".into()
    } else if code == AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED {
        "Exclusive use is turned off for this device in the Windows sound settings.".into()
    } else if code == AUDCLNT_E_UNSUPPORTED_FORMAT {
        format!("The device can't play {} directly.", khz(rate))
    } else {
        format!("The device wouldn't open on its own ({e}).")
    }
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) { format!("{} kHz", rate / 1000) } else { format!("{:.1} kHz", rate as f64 / 1000.0) }
}

/// An exclusive stream, initialised and ready to start.
pub struct OwnStream {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: HANDLE,
    layout: SampleLayout,
    channels: u16,
    frames: u32,
}

fn activate(device: &IMMDevice) -> windows::core::Result<IAudioClient> {
    // SAFETY: plain COM call on this thread's (initialised) apartment.
    unsafe { device.Activate::<IAudioClient>(CLSCTX_ALL, None) }
}

pub(super) fn open(endpoint_id: &str, rate: u32, channels: u16) -> Result<Opened, String> {
    // SAFETY: COM setup and WASAPI calls on this thread only; every pointer
    // handed over points at a local that outlives the call.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            return Err(format!("The device wouldn't open on its own ({hr:?})."));
        }
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| reason(&e, rate))?;
        let id = HSTRING::from(endpoint_id);
        let device = enumerator.GetDevice(PCWSTR(id.as_ptr())).map_err(|e| reason(&e, rate))?;
        let mut client = activate(&device).map_err(|e| reason(&e, rate))?;

        let mut picked = None;
        for layout in LAYOUTS {
            let f = wave_format(layout, rate, channels);
            let hr = client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &f.Format, None);
            if hr.is_ok() && hr.0 == 0 {
                picked = Some((layout, f));
                break;
            }
            if hr == AUDCLNT_E_DEVICE_IN_USE || hr == AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED {
                return Err(reason(&hr.into(), rate));
            }
        }
        let Some((layout, format)) = picked else {
            return Err(format!("The device can't play {} directly.", khz(rate)));
        };

        let mut period = 0i64;
        client.GetDevicePeriod(Some(&mut period), None).map_err(|e| reason(&e, rate))?;
        let init = |c: &IAudioClient, p: i64| {
            c.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, p, p, &format.Format, None)
        };
        if let Err(e) = init(&client, period) {
            if e.code() != AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
                return Err(reason(&e, rate));
            }
            // Ask again with a period that is a whole number of the
            // device's own frames.
            let frames = client.GetBufferSize().map_err(|e| reason(&e, rate))?;
            period = (10_000_000.0 * frames as f64 / rate as f64 + 0.5) as i64;
            client = activate(&device).map_err(|e| reason(&e, rate))?;
            init(&client, period).map_err(|e| reason(&e, rate))?;
        }
        let event = CreateEventW(None, false, false, None).map_err(|e| reason(&e, rate))?;
        let setup = (|| {
            client.SetEventHandle(event)?;
            let frames = client.GetBufferSize()?;
            let render = client.GetService::<IAudioRenderClient>()?;
            Ok::<_, windows::core::Error>((frames, render))
        })();
        let (frames, render) = match setup {
            Ok(x) => x,
            Err(e) => {
                let _ = CloseHandle(event);
                return Err(reason(&e, rate));
            }
        };
        Ok(Opened::Own {
            stream: OwnStream { client, render, event, layout, channels, frames },
            rate,
            channels,
            bits: layout.valid_bits(),
        })
    }
}

impl OwnStream {
    /// Fills one device buffer. Returns false when the device went away.
    fn fill(&self, scratch: &mut [f32], renderer: &dyn Renderer) -> windows::core::Result<()> {
        let n = self.frames as usize * self.channels as usize;
        let samples = &mut scratch[..n];
        renderer.render(samples);
        // SAFETY: GetBuffer hands out `frames` frames of device memory until
        // ReleaseBuffer; the slice covers exactly that.
        unsafe {
            let ptr = self.render.GetBuffer(self.frames)?;
            let out = std::slice::from_raw_parts_mut(ptr, n * self.layout.bytes());
            self.layout.write(samples, out);
            self.render.ReleaseBuffer(self.frames, 0)
        }
    }

    fn silence(&self) -> windows::core::Result<()> {
        // SAFETY: as in `fill`; the buffer is zeroed before release.
        unsafe {
            let ptr = self.render.GetBuffer(self.frames)?;
            std::ptr::write_bytes(ptr, 0, self.frames as usize * self.channels as usize * self.layout.bytes());
            self.render.ReleaseBuffer(self.frames, 0)
        }
    }

    /// Plays until told to stop. While idle the stream is stopped and the
    /// thread sleeps on the control channel, so nothing wakes the app.
    pub(in crate::audio) fn run(self, renderer: Arc<dyn Renderer>, ctl: Receiver<StreamCtl>, lost: Box<dyn FnOnce(String) + Send>) {
        let mut scratch = vec![0.0f32; self.frames as usize * self.channels as usize];
        let start = |s: &Self| -> windows::core::Result<()> {
            s.silence()?;
            // SAFETY: the client was initialised on this thread.
            unsafe { s.client.Start() }
        };
        let mut failure = start(&self).err();
        while failure.is_none() {
            match ctl.try_recv() {
                Ok(StreamCtl::Idle(true)) => {
                    // SAFETY: as above.
                    let _ = unsafe { self.client.Stop() };
                    let mut resume = false;
                    while let Ok(c) = ctl.recv() {
                        if let StreamCtl::Idle(false) = c {
                            resume = true;
                            break;
                        }
                        if let StreamCtl::Stop = c {
                            break;
                        }
                    }
                    if !resume {
                        break;
                    }
                    failure = start(&self).err();
                    continue;
                }
                Ok(StreamCtl::Idle(false)) | Err(TryRecvError::Empty) => {}
                Ok(StreamCtl::Stop) | Err(TryRecvError::Disconnected) => break,
            }
            // SAFETY: the event handle lives as long as `self`.
            if unsafe { WaitForSingleObject(self.event, 2000) } != WAIT_OBJECT_0 {
                continue;
            }
            failure = self.fill(&mut scratch, renderer.as_ref()).err();
        }
        // SAFETY: as above.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
        if let Some(e) = failure {
            let why = if e.code() == AUDCLNT_E_DEVICE_INVALIDATED { "the device went away".to_string() } else { e.to_string() };
            lost(why);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_describe_the_layout() {
        let f = wave_format(SampleLayout::I24, 96_000, 2);
        assert_eq!({ f.Format.nBlockAlign }, 6);
        assert_eq!({ f.Format.nAvgBytesPerSec }, 96_000 * 6);
        assert_eq!({ f.Format.wBitsPerSample }, 24);
        // SAFETY: reading a plain union field just written.
        assert_eq!(unsafe { f.Samples.wValidBitsPerSample }, 24);
        let f = wave_format(SampleLayout::I24In32, 44_100, 2);
        assert_eq!({ f.Format.wBitsPerSample }, 32);
        assert_eq!(unsafe { f.Samples.wValidBitsPerSample }, 24);
        assert_eq!({ wave_format(SampleLayout::F32, 48_000, 2).SubFormat }, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
    }

    #[test]
    fn rates_read_plainly() {
        assert_eq!(khz(96_000), "96 kHz");
        assert_eq!(khz(44_100), "44.1 kHz");
    }

    /// Opens the default output exclusively for a moment. Needs a real
    /// device nothing else holds: `cargo test -- --ignored exclusive_open`.
    #[test]
    #[ignore]
    fn exclusive_open_on_the_default_device() {
        use cpal::traits::{DeviceTrait, HostTrait};
        let id = cpal::default_host().default_output_device().and_then(|d| d.id().ok()).expect("a device").to_string();
        let system = id.split_once(':').map_or(id.as_str(), |(_, s)| s).to_string();
        match open(&system, 48_000, 2) {
            Ok(Opened::Own { bits, .. }) => assert!(bits >= 16),
            Ok(_) => unreachable!(),
            Err(reason) => panic!("fell back: {reason}"),
        }
    }
}
