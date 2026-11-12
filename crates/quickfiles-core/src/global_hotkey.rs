//! One system-wide hotkey (default `Ctrl+Alt+Space`) that brings the window to the front, even
//! when it is minimized, hidden or behind other windows. Windows only; elsewhere it does nothing.
//!
//! The app must be running to hear the key: this cannot start a closed app. The window is found
//! by its title, so the title given to `GlobalHotkey::start` must match the viewport title.

use std::sync::{Arc, Mutex};

use crate::engine::Wake;

/// A modifier combination plus one key, e.g. `Ctrl+Alt+Space`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Windows virtual-key code.
    pub vk: u32,
}

fn vk_of(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        return match c {
            b'a'..=b'z' => Some(u32::from(c.to_ascii_uppercase())),
            b'0'..=b'9' => Some(u32::from(c)),
            _ => None,
        };
    }
    if let Some(num) = n.strip_prefix('f').and_then(|d| d.parse::<u32>().ok()) {
        return (1..=24).contains(&num).then(|| 0x70 + num - 1);
    }
    Some(match n.as_str() {
        "space" => 0x20,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        _ => return None,
    })
}

impl Combo {
    /// "Ctrl+Alt+Space" -> combo. At least one modifier is required (a bare key would be taken
    /// away from every other program).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut c = Combo {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk: 0,
        };
        let mut key = None;
        for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => c.ctrl = true,
                "alt" | "option" => c.alt = true,
                "shift" => c.shift = true,
                "win" | "super" | "meta" => c.win = true,
                _ => {
                    let vk = vk_of(part).ok_or_else(|| format!("unknown key \"{part}\""))?;
                    if key.replace(vk).is_some() {
                        return Err("more than one key".to_owned());
                    }
                }
            }
        }
        c.vk = key.ok_or_else(|| "no key".to_owned())?;
        if !(c.ctrl || c.alt || c.shift || c.win) {
            return Err("needs a modifier (Ctrl, Alt, Shift or Win)".to_owned());
        }
        Ok(c)
    }
}

/// Owns the listener thread. Dropping it unregisters the key.
pub struct GlobalHotkey {
    inner: imp::Inner,
    messages: Arc<Mutex<Vec<String>>>,
}

impl GlobalHotkey {
    /// `wake` repaints the UI. The window is found by its title (`window_title`) when the key is
    /// pressed, so nothing about the window has to be known up front.
    pub fn start(window_title: &str, wake: Wake) -> Self {
        let messages = Arc::new(Mutex::new(Vec::new()));
        Self {
            inner: imp::Inner::start(window_title.to_owned(), wake, messages.clone()),
            messages,
        }
    }

    /// Registers `combo` (or removes the hotkey for `None`), replacing the previous one.
    /// Failures (the key belongs to another program) arrive through [`take_messages`](Self::take_messages).
    pub fn set(&self, combo: Option<Combo>) {
        self.inner.set(combo);
    }

    /// Problems reported by the listener since the last call.
    pub fn take_messages(&self) -> Vec<String> {
        std::mem::take(&mut *self.messages.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(windows)]
mod imp {
    use std::{
        sync::{Arc, Mutex, mpsc},
        thread,
    };

    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::{
                HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
                RegisterHotKey, UnregisterHotKey,
            },
            WindowsAndMessaging::{
                BringWindowToTop, FindWindowW, GetForegroundWindow, GetMessageW,
                GetWindowThreadProcessId, IsIconic, IsWindowVisible, MSG, PM_NOREMOVE,
                PeekMessageW, PostThreadMessageW, SW_RESTORE, SW_SHOW, SetForegroundWindow,
                ShowWindow, WM_APP, WM_HOTKEY, WM_QUIT,
            },
        },
    };

    use windows::core::PCWSTR;

    use super::{Combo, Wake};

    const ID: i32 = 0x7166; // arbitrary, unique within this thread
    const WM_SET: u32 = WM_APP + 1;

    pub struct Inner {
        thread_id: u32,
        wanted: Arc<Mutex<Option<Combo>>>,
    }

    fn mods(c: Combo) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if c.ctrl {
            m |= MOD_CONTROL;
        }
        if c.alt {
            m |= MOD_ALT;
        }
        if c.shift {
            m |= MOD_SHIFT;
        }
        if c.win {
            m |= MOD_WIN;
        }
        m
    }

    /// Restore (if minimized), show (if hidden) and focus the window.
    pub(crate) fn raise(hwnd: HWND) {
        // SAFETY: plain Win32 calls on a window handle; failures are harmless no-ops.
        unsafe {
            // Windows only lets the foreground thread hand out the foreground: borrow its input
            // queue for the duration of the call.
            let me = GetCurrentThreadId();
            let fg = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let attached = fg != 0 && fg != me && AttachThreadInput(me, fg, true).as_bool();
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(me, fg, false);
            }
        }
    }

    impl Inner {
        pub fn start(title: String, wake: Wake, messages: Arc<Mutex<Vec<String>>>) -> Self {
            let wanted: Arc<Mutex<Option<Combo>>> = Arc::new(Mutex::new(None));
            let (tx, rx) = mpsc::channel();
            let w = wanted.clone();
            let spawned = thread::Builder::new()
                .name("qf-global-hotkey".to_owned())
                .spawn(move || {
                    // Touch the message queue so it exists before anyone posts to it.
                    let mut msg = MSG::default();
                    // SAFETY: standard message loop on this thread's own queue.
                    unsafe {
                        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                        let _ = tx.send(GetCurrentThreadId());
                    }
                    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
                    let mut registered = false;
                    // SAFETY: as above.
                    unsafe {
                        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                            match msg.message {
                                WM_HOTKEY => {
                                    // SAFETY: `wide` is NUL-terminated and outlives the call.
                                    let hwnd = FindWindowW(None, PCWSTR(wide.as_ptr()))
                                        .unwrap_or_default();
                                    if !hwnd.0.is_null() {
                                        raise(hwnd);
                                    }
                                    wake();
                                }
                                WM_SET => {
                                    if registered {
                                        let _ = UnregisterHotKey(None, ID);
                                        registered = false;
                                    }
                                    let want = *w.lock().unwrap_or_else(|e| e.into_inner());
                                    if let Some(c) = want {
                                        match RegisterHotKey(None, ID, mods(c), c.vk) {
                                            Ok(()) => registered = true,
                                            Err(e) => messages
                                                .lock()
                                                .unwrap_or_else(|e| e.into_inner())
                                                .push(format!(
                                                    "Global hotkey is taken by another program ({e})"
                                                )),
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if registered {
                            let _ = UnregisterHotKey(None, ID);
                        }
                    }
                });
            let thread_id = if spawned.is_ok() {
                rx.recv().unwrap_or(0)
            } else {
                0
            };
            Self { thread_id, wanted }
        }

        pub fn set(&self, combo: Option<Combo>) {
            *self.wanted.lock().unwrap_or_else(|e| e.into_inner()) = combo;
            if self.thread_id != 0 {
                // SAFETY: posting a private message to our own listener thread.
                unsafe {
                    let _ = PostThreadMessageW(self.thread_id, WM_SET, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            if self.thread_id != 0 {
                // SAFETY: as above; WM_QUIT ends the loop, which unregisters the key.
                unsafe {
                    let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
}

#[cfg(windows)]
pub(crate) use imp::raise;

#[cfg(not(windows))]
mod imp {
    use std::sync::{Arc, Mutex};

    use super::{Combo, Wake};

    pub struct Inner;

    impl Inner {
        pub fn start(_: String, _: Wake, _: Arc<Mutex<Vec<String>>>) -> Self {
            Inner
        }

        pub fn set(&self, _: Option<Combo>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default() {
        let c = Combo::parse("Ctrl+Alt+Space").unwrap();
        assert!(c.ctrl && c.alt && !c.shift && !c.win);
        assert_eq!(c.vk, 0x20);
    }

    #[test]
    fn letters_digits_and_function_keys() {
        assert_eq!(Combo::parse("win+q").unwrap().vk, u32::from(b'Q'));
        assert_eq!(Combo::parse("Ctrl+5").unwrap().vk, u32::from(b'5'));
        assert_eq!(Combo::parse("Alt+F12").unwrap().vk, 0x7B);
        assert!(Combo::parse("Alt+F25").is_err());
    }

    #[test]
    fn rejects_bad_combos() {
        assert!(Combo::parse("Space").is_err(), "no modifier");
        assert!(Combo::parse("Ctrl+").is_err(), "no key");
        assert!(Combo::parse("Ctrl+A+B").is_err(), "two keys");
        assert!(Combo::parse("Ctrl+Banana").is_err());
    }
}
