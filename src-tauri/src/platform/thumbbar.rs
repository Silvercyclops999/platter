//! Windows taskbar thumbnail toolbar: previous, play/pause, next under the
//! window's taskbar preview.
//!
//! The buttons are added through `ITaskbarList3` once the taskbar button
//! exists (and again if Explorer restarts), and their clicks arrive as
//! `WM_COMMAND` on the main window, which is subclassed for that. Everything
//! here runs on the main thread. The icons are drawn in code.

use std::cell::RefCell;

use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::watch;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_DISABLED,
    THBF_ENABLED, THBN_CLICKED, THUMBBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, GetSystemMetrics, HICON, RegisterWindowMessageW, SM_CXSMICON, WM_COMMAND,
};
use windows::core::w;

use super::media::OsCommand;
use super::tray::TrayView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

const ID_PREVIOUS: u32 = 1;
const ID_TOGGLE: u32 = 2;
const ID_NEXT: u32 = 3;
const SUBCLASS_ID: usize = 0x504c_4154; // "PLAT"

/// The command for a clicked button id.
pub fn button_command(id: u32) -> Option<OsCommand> {
    match id {
        ID_PREVIOUS => Some(OsCommand::Previous),
        ID_TOGGLE => Some(OsCommand::Toggle),
        ID_NEXT => Some(OsCommand::Next),
        _ => None,
    }
}

fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let s = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1);
    let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn in_rect(p: (f32, f32), x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
}

fn covers(g: Glyph, p: (f32, f32)) -> bool {
    let mirror = |p: (f32, f32)| (1.0 - p.0, p.1);
    match g {
        Glyph::Play => in_triangle(p, (0.28, 0.18), (0.28, 0.82), (0.84, 0.5)),
        Glyph::Pause => in_rect(p, 0.24, 0.18, 0.42, 0.82) || in_rect(p, 0.58, 0.18, 0.76, 0.82),
        Glyph::Next => in_triangle(p, (0.2, 0.2), (0.2, 0.8), (0.66, 0.5)) || in_rect(p, 0.68, 0.2, 0.8, 0.8),
        Glyph::Previous => covers(Glyph::Next, mirror(p)),
    }
}

/// White glyph on transparent, `size`×`size` BGRA rows top to bottom,
/// anti-aliased by 4×4 supersampling.
pub fn glyph_pixels(g: Glyph, size: usize) -> Vec<u8> {
    const SS: usize = 4;
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let mut hit = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = (
                        (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32,
                        (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32,
                    );
                    hit += covers(g, p) as usize;
                }
            }
            let a = (hit * 255 / (SS * SS)) as u8;
            // White, straight alpha.
            out.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    out
}

fn make_icon(g: Glyph, size: usize) -> windows::core::Result<HICON> {
    let bgra = glyph_pixels(g, size);
    // 1-bit mask rows are padded to 16 bits; all clear so alpha decides.
    let and_mask = vec![0u8; size.div_ceil(16) * 2 * size];
    unsafe { CreateIcon(None, size as i32, size as i32, 1, 32, and_mask.as_ptr(), bgra.as_ptr()) }
}

fn tip(s: &str) -> [u16; 260] {
    let mut t = [0u16; 260];
    for (i, c) in s.encode_utf16().take(259).enumerate() {
        t[i] = c;
    }
    t
}

struct Bar {
    hwnd: HWND,
    created_msg: u32,
    list: Option<ITaskbarList3>,
    added: bool,
    icons: [HICON; 4],
    view: TrayView,
    on_command: Box<dyn Fn(OsCommand)>,
}

thread_local! {
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
}

impl Bar {
    fn buttons(&self) -> [THUMBBUTTON; 3] {
        let flags = if self.view.has_track { THBF_ENABLED } else { THBF_DISABLED };
        let b = |id, icon, text: &str| THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: id,
            hIcon: icon,
            szTip: tip(text),
            dwFlags: flags,
            ..Default::default()
        };
        let (toggle_icon, toggle_tip) = if self.view.playing { (self.icons[2], "Pause") } else { (self.icons[1], "Play") };
        [b(ID_PREVIOUS, self.icons[0], "Previous"), b(ID_TOGGLE, toggle_icon, toggle_tip), b(ID_NEXT, self.icons[3], "Next")]
    }

    fn add(&mut self) {
        if self.list.is_none() {
            let list: windows::core::Result<ITaskbarList3> =
                unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) };
            match list.and_then(|l| unsafe { l.HrInit() }.map(|_| l)) {
                Ok(l) => self.list = Some(l),
                Err(e) => {
                    tracing::debug!("taskbar list: {e}");
                    return;
                }
            }
        }
        let buttons = self.buttons();
        if let Some(l) = &self.list {
            // Fails until the taskbar button exists; we try again when it does.
            self.added = unsafe { l.ThumbBarAddButtons(self.hwnd, &buttons) }.is_ok();
        }
    }

    fn refresh(&self) {
        if let (true, Some(l)) = (self.added, &self.list) {
            let _ = unsafe { l.ThumbBarUpdateButtons(self.hwnd, &self.buttons()) };
        }
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let mut cmd = None;
    BAR.with(|b| {
        if let Ok(mut b) = b.try_borrow_mut()
            && let Some(bar) = b.as_mut()
        {
            if msg == bar.created_msg {
                bar.added = false;
                bar.add();
            } else if msg == WM_COMMAND && ((wparam.0 >> 16) & 0xffff) as u32 == THBN_CLICKED {
                cmd = button_command((wparam.0 & 0xffff) as u32);
            }
        }
    });
    if let Some(c) = cmd {
        BAR.with(|b| {
            if let Ok(b) = b.try_borrow()
                && let Some(bar) = b.as_ref()
            {
                (bar.on_command)(c)
            }
        });
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Add the toolbar to the main window and keep it in step with playback.
pub fn install<R: Runtime>(
    app: &AppHandle<R>,
    on_command: impl Fn(OsCommand) + Send + Sync + Clone + 'static,
    mut view: watch::Receiver<TrayView>,
) {
    let Some(window) = app.get_webview_window("main") else { return };
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    let initial = view.borrow().clone();
    let r = app.run_on_main_thread(move || {
        let hwnd = HWND(hwnd as *mut _);
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as usize;
        let icons = [Glyph::Previous, Glyph::Play, Glyph::Pause, Glyph::Next].map(|g| make_icon(g, size));
        let Ok(icons) = icons.into_iter().collect::<windows::core::Result<Vec<_>>>() else {
            tracing::debug!("thumbnail toolbar icons failed");
            return;
        };
        let created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
        let mut bar = Bar {
            hwnd,
            created_msg,
            list: None,
            added: false,
            icons: [icons[0], icons[1], icons[2], icons[3]],
            view: initial,
            on_command: Box::new(on_command),
        };
        if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) }.as_bool() {
            tracing::debug!("thumbnail toolbar: subclass failed");
            return;
        }
        bar.add();
        BAR.with(|b| *b.borrow_mut() = Some(bar));
    });
    if r.is_err() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while view.changed().await.is_ok() {
            let v = view.borrow_and_update().clone();
            let _ = app.run_on_main_thread(move || {
                BAR.with(|b| {
                    if let Ok(mut b) = b.try_borrow_mut()
                        && let Some(bar) = b.as_mut()
                        && bar.view != v
                    {
                        bar.view = v;
                        bar.refresh();
                    }
                });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_map_to_commands() {
        assert_eq!(button_command(ID_PREVIOUS), Some(OsCommand::Previous));
        assert_eq!(button_command(ID_TOGGLE), Some(OsCommand::Toggle));
        assert_eq!(button_command(ID_NEXT), Some(OsCommand::Next));
        assert_eq!(button_command(99), None);
    }

    #[test]
    fn glyphs_are_drawn_and_distinct() {
        let size = 16;
        let alpha = |g| glyph_pixels(g, size).chunks(4).map(|p| p[3] as u32).collect::<Vec<_>>();
        let (prev, play, pause, next) = (alpha(Glyph::Previous), alpha(Glyph::Play), alpha(Glyph::Pause), alpha(Glyph::Next));
        assert_eq!(play.len(), size * size);
        for g in [&prev, &play, &pause, &next] {
            let ink: u32 = g.iter().sum();
            assert!(ink > 255 * 30 && ink < 255 * 200, "{ink}");
            // Corners stay clear.
            assert_eq!(g[0], 0);
        }
        assert_ne!(play, pause);
        // Previous is next mirrored.
        for y in 0..size {
            for x in 0..size {
                assert_eq!(prev[y * size + x], next[y * size + (size - 1 - x)]);
            }
        }
    }
}
