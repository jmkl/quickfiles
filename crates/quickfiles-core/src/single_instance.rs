//! Only one quickfiles at a time. Starting it again (`qf.exe .`, a shortcut) while the first is
//! running, possibly hidden in the tray, shows that window instead and hands it the folder.
//!
//! The first instance owns a named mutex. A later one raises the first one's window (found by
//! its title), leaves the folder to open in a small request file, and the caller exits. The
//! running app polls [`take_request`] each frame; showing the window wakes it.

use std::path::{Path, PathBuf};

fn request_file() -> PathBuf {
    crate::sys::data_dir().join("open-request.txt")
}

/// True when this process is the first (or the only way to tell is unavailable). When false the
/// running instance has been brought to the front and, if `folder` is given, asked to open it.
pub fn claim_or_forward(window_title: &str, folder: Option<&Path>) -> bool {
    if !imp::claim() {
        if let Some(dir) = folder {
            let _ = std::fs::create_dir_all(crate::sys::data_dir());
            let _ = std::fs::write(request_file(), dir.to_string_lossy().as_bytes());
        }
        imp::raise_other(window_title);
        return false;
    }
    // A request left behind by an earlier run must not open a folder now.
    let _ = std::fs::remove_file(request_file());
    true
}

/// A folder another launch asked this instance to open (read once, then forgotten).
pub fn take_request() -> Option<PathBuf> {
    let file = request_file();
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    let path = PathBuf::from(text.trim());
    path.is_dir().then_some(path)
}

#[cfg(windows)]
mod imp {
    use windows::{
        Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
            UI::WindowsAndMessaging::FindWindowW,
        },
        core::{PCWSTR, w},
    };

    pub fn claim() -> bool {
        // SAFETY: creating a named mutex; the handle is deliberately kept until the process ends.
        unsafe {
            match CreateMutexW(None, false, w!("Local\\quickfiles-single-instance")) {
                Ok(_handle) => GetLastError() != ERROR_ALREADY_EXISTS,
                // Cannot tell: behave like before and just run.
                Err(_) => true,
            }
        }
    }

    pub fn raise_other(title: &str) {
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let hwnd = unsafe { FindWindowW(None, PCWSTR(wide.as_ptr())) }.unwrap_or_default();
        if !hwnd.0.is_null() {
            crate::global_hotkey::raise(hwnd);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn claim() -> bool {
        true
    }

    pub fn raise_other(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_request_is_none() {
        let _ = std::fs::remove_file(request_file());
        assert!(take_request().is_none());
    }
}
