//! Notification-area (system tray) icon with a Show / Exit menu. Windows only; elsewhere it does
//! nothing and `active()` is false, so the window button really closes the app.
//!
//! The icon lives on its own thread with a hidden window. Show and Exit are handled right there
//! (the UI thread sleeps while the window is hidden, so it cannot be asked): Show raises the
//! window found by its title, Exit asks it to close and sets `quitting()` so the UI lets it.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::engine::Wake;

pub struct Tray {
    quitting: Arc<AtomicBool>,
    active: bool,
    title: String,
}

impl Tray {
    /// `window_title` must match the viewport title: the window is found by it.
    pub fn start(window_title: &str, wake: Wake) -> Self {
        let quitting = Arc::new(AtomicBool::new(false));
        let active = imp::start(window_title.to_owned(), wake, quitting.clone());
        Self {
            quitting,
            active,
            title: window_title.to_owned(),
        }
    }

    /// The icon exists, so closing the window may hide it instead.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Hides the window (and its taskbar button) the way minimize would, but to the tray. Done
    /// with plain Win32 so it works every time, whatever the UI thinks the window state is.
    pub fn hide_window(&self) {
        imp::hide(&self.title);
    }

    /// Exit was chosen in the tray menu: the next close request must really close.
    pub fn quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }
}

/// A white magnifying glass on a transparent background, `size` x `size` RGBA pixels.
/// Anti-aliased from distance functions, so it stays crisp at 16 or 32 pixels.
pub fn search_icon_rgba(size: usize) -> Vec<u8> {
    let n = size as f32;
    // Lens: a ring centred up-left; handle: a thick segment towards the bottom-right.
    let (cx, cy, r, thick) = (n * 0.42, n * 0.42, n * 0.28, (n * 0.10).max(1.4));
    let (hx0, hy0) = (cx + r * 0.72, cy + r * 0.72);
    let (hx1, hy1) = (n * 0.90, n * 0.90);
    let mut px = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let ring = ((fx - cx).hypot(fy - cy) - r).abs() - thick * 0.5;
            // Distance to the handle segment.
            let (dx, dy) = (hx1 - hx0, hy1 - hy0);
            let t = (((fx - hx0) * dx + (fy - hy0) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            let handle = (fx - (hx0 + t * dx)).hypot(fy - (hy0 + t * dy)) - thick * 0.6;
            let d = ring.min(handle);
            let a = (0.5 - d).clamp(0.0, 1.0);
            let i = (y * size + x) * 4;
            px[i..i + 4].copy_from_slice(&[255, 255, 255, (a * 255.0).round() as u8]);
        }
    }
    px
}

#[cfg(windows)]
mod imp {
    use std::{
        sync::{
            Arc, OnceLock,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Shell::{
                    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                    Shell_NotifyIconW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
                    DestroyMenu, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, HICON,
                    IDI_APPLICATION, LoadIconW, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
                    RegisterClassW, RegisterWindowMessageW, SW_HIDE, SetForegroundWindow,
                    ShowWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                    TranslateMessage, WM_APP, WM_CLOSE, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL,
                    WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
                },
            },
        },
        core::{PCWSTR, w},
    };

    use crate::engine::Wake;

    const WM_TRAY: u32 = WM_APP + 2;
    const ID_SHOW: usize = 1;
    const ID_EXIT: usize = 2;
    const ICON_ID: u32 = 1;

    struct Shared {
        title: Vec<u16>,
        wake: Wake,
        quitting: Arc<AtomicBool>,
        /// Broadcast by Explorer after it restarts: the icon has to be added again.
        created: u32,
    }

    static SHARED: OnceLock<Shared> = OnceLock::new();

    fn app_window(s: &Shared) -> HWND {
        // SAFETY: `title` is NUL-terminated and outlives the call.
        unsafe { FindWindowW(None, PCWSTR(s.title.as_ptr())) }.unwrap_or_default()
    }

    fn show(s: &Shared) {
        let hwnd = app_window(s);
        if !hwnd.0.is_null() {
            crate::global_hotkey::raise(hwnd);
        }
        (s.wake)();
    }

    fn exit(s: &Shared) {
        s.quitting.store(true, Ordering::SeqCst);
        let hwnd = app_window(s);
        if !hwnd.0.is_null() {
            // SAFETY: posting a message to a window handle; failure is harmless.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        (s.wake)();
        // A hidden window may never get to run its close handling: do not hang around.
        thread::spawn(|| {
            thread::sleep(Duration::from_millis(1500));
            std::process::exit(0);
        });
    }

    /// The generated search icon as an HICON (32 bit, with alpha); the stock icon if that fails.
    fn search_icon() -> Option<HICON> {
        const SIZE: usize = 32;
        let rgba = super::search_icon_rgba(SIZE);
        // Windows wants BGRA for the colour bits, and a (here all-zero = opaque) AND mask.
        let mut bgra = rgba.clone();
        for p in bgra.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        let mask = vec![0u8; SIZE * SIZE / 8];
        // SAFETY: both buffers outlive the call and have the sizes CreateIcon reads.
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            CreateIcon(
                Some(module.into()),
                SIZE as i32,
                SIZE as i32,
                1,
                32,
                mask.as_ptr(),
                bgra.as_ptr(),
            )
            .ok()
            .or_else(|| LoadIconW(None, IDI_APPLICATION).ok())
        }
    }

    pub fn hide(title: &str) {
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        unsafe {
            if let Ok(hwnd) = FindWindowW(None, PCWSTR(wide.as_ptr())) {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            ..Default::default()
        };
        nid.hIcon = search_icon().unwrap_or_default();
        for (i, c) in "quickfiles".encode_utf16().enumerate() {
            nid.szTip[i] = c;
        }
        nid
    }

    fn menu(hwnd: HWND) {
        // SAFETY: standard popup menu on our own hidden window.
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let _ = AppendMenuW(menu, MF_STRING, ID_SHOW, w!("Show quickfiles"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"));
            let mut at = POINT::default();
            let _ = GetCursorPos(&mut at);
            // Required, or the menu does not close when the user clicks elsewhere.
            let _ = SetForegroundWindow(hwnd);
            let picked = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                at.x,
                at.y,
                None,
                hwnd,
                None,
            );
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            if let Some(s) = SHARED.get() {
                match picked.0 as usize {
                    ID_SHOW => show(s),
                    ID_EXIT => exit(s),
                    _ => {}
                }
            }
        }
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_TRAY {
            match lp.0 as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    if let Some(s) = SHARED.get() {
                        show(s);
                    }
                }
                WM_RBUTTONUP => menu(hwnd),
                _ => {}
            }
            return LRESULT(0);
        }
        if SHARED.get().is_some_and(|s| s.created == msg) {
            let nid = icon_data(hwnd);
            // SAFETY: `nid` is fully initialised.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            }
            return LRESULT(0);
        }
        // SAFETY: default handling for everything else.
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    pub fn start(title: String, wake: Wake, quitting: Arc<AtomicBool>) -> bool {
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("qf-tray".to_owned())
            .spawn(move || {
                // SAFETY: a hidden top-level window with its own message loop on this thread.
                unsafe {
                    let created = RegisterWindowMessageW(w!("TaskbarCreated"));
                    let _ = SHARED.set(Shared {
                        title: title.encode_utf16().chain(Some(0)).collect(),
                        wake,
                        quitting,
                        created,
                    });
                    let class = w!("quickfiles_tray");
                    let wc = WNDCLASSW {
                        lpfnWndProc: Some(proc),
                        lpszClassName: class,
                        ..Default::default()
                    };
                    RegisterClassW(&wc);
                    let hwnd = CreateWindowExW(
                        WS_EX_TOOLWINDOW,
                        class,
                        w!("quickfiles tray"),
                        WS_POPUP,
                        0,
                        0,
                        0,
                        0,
                        None,
                        None,
                        None,
                        None,
                    );
                    let Ok(hwnd) = hwnd else {
                        let _ = tx.send(false);
                        return;
                    };
                    let nid = icon_data(hwnd);
                    let added = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
                    let _ = tx.send(added);
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                }
            });
        spawned.is_ok() && rx.recv().unwrap_or(false)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::{Arc, atomic::AtomicBool};

    use crate::engine::Wake;

    pub fn start(_: String, _: Wake, _: Arc<AtomicBool>) -> bool {
        false
    }

    pub fn hide(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_white_and_not_blank_or_full() {
        let px = search_icon_rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        let opaque = px.chunks(4).filter(|p| p[3] == 255).count();
        assert!(opaque > 40 && opaque < 500, "{opaque}");
        assert!(px.chunks(4).all(|p| p[..3] == [255, 255, 255]));
        // Corners stay empty (top-right and bottom-left have no glass or handle).
        assert_eq!(px[(31 * 4) + 3], 0);
        assert_eq!(px[(31 * 32 * 4) + 3], 0);
    }
}
