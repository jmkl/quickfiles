//! Thin OS layer: drives, well-known folders, open / reveal / recycle, finding Everything.
//! Everything here is small and blocking; callers run it on a worker thread.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use crate::model::{DriveInfo, Place};

/// Appends a line to `name` in the data folder (a small file a user can send when something
/// cannot be seen from outside). The file is cut back to nothing once it passes 64 KiB.
pub fn debug_log(name: &str, line: &str) {
    use std::io::Write;
    let path = data_dir().join(name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

pub fn data_dir() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("quickfiles");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Local UTC offset in seconds, used to render modified times.
pub fn tz_offset() -> i32 {
    use chrono::Offset;
    chrono::Local::now().offset().fix().local_minus_utc()
}

pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    let mut add = |label: &str, p: Option<PathBuf>| {
        if let Some(path) = p.filter(|p| p.is_dir()) {
            out.push(Place {
                label: label.to_owned(),
                path,
            });
        }
    };
    add("Home", dirs::home_dir());
    add("Desktop", dirs::desktop_dir());
    add("Documents", dirs::document_dir());
    add("Downloads", dirs::download_dir());
    add("Pictures", dirs::picture_dir());
    add("Music", dirs::audio_dir());
    add("Videos", dirs::video_dir());
    out
}

pub fn open(path: &Path) -> Result<(), String> {
    open::that_detached(path).map_err(|e| e.to_string())
}

/// Show the item selected in Explorer.
pub fn reveal(path: &Path) -> Result<(), String> {
    let arg = format!("/select,{}", path.display());
    Command::new("explorer.exe")
        .arg(arg)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn recycle(paths: &[PathBuf]) -> Result<(), String> {
    trash::delete_all(paths).map_err(|e| e.to_string())
}

/// Where Everything.exe lives, if installed in a usual place.
pub fn find_everything() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Everything 1.5a").join("Everything64.exe"));
            candidates.push(base.join("Everything").join("Everything.exe"));
            candidates.push(base.join("Everything 1.5a").join("Everything.exe"));
            candidates.push(
                base.join("Programs")
                    .join("Everything")
                    .join("Everything.exe"),
            );
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn launch_everything() {
    if let Some(exe) = find_everything() {
        let _ = Command::new(exe).arg("-startup").spawn();
    }
}

pub fn open_url(url: &str) {
    let _ = open::that_detached(url);
}

pub const EVERYTHING_URL: &str = "https://www.voidtools.com/downloads/";

// --- Window size and config files ---------------------------------------------------------

/// Last window state, kept in a tiny text file (the window must be sized before the database
/// thread exists, so this one value does not go through `store`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowState {
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

impl WindowState {
    pub fn to_text(self) -> String {
        format!(
            "{} {} {}",
            self.w.round(),
            self.h.round(),
            u8::from(self.maximized)
        )
    }

    /// Rejects nonsense so a corrupt file can never open a 0x0 or off-screen window.
    pub fn parse(text: &str) -> Option<Self> {
        let mut it = text.split_whitespace();
        let w: f32 = it.next()?.parse().ok()?;
        let h: f32 = it.next()?.parse().ok()?;
        let maximized = it.next().is_some_and(|m| m == "1");
        ((300.0..=10_000.0).contains(&w) && (200.0..=10_000.0).contains(&h)).then_some(Self {
            w,
            h,
            maximized,
        })
    }
}

pub fn load_window() -> Option<WindowState> {
    WindowState::parse(&std::fs::read_to_string(data_dir().join("window.txt")).ok()?)
}

pub fn save_window(state: WindowState) {
    let _ = std::fs::write(data_dir().join("window.txt"), state.to_text());
}

/// User-editable settings live next to the database (e.g. `hotkeys.conf`).
pub fn config_path(name: &str) -> PathBuf {
    data_dir().join(name)
}

pub fn read_config(name: &str) -> Option<String> {
    std::fs::read_to_string(config_path(name)).ok()
}

/// Writes `contents` only when the file does not exist yet (never overwrites user edits).
pub fn write_config_if_missing(name: &str, contents: &str) {
    let p = config_path(name);
    if !p.exists() {
        let _ = std::fs::write(p, contents);
    }
}

// --- File clipboard (CF_HDROP, shared with Explorer) ----------------------------------------

/// Files on the OS clipboard and whether they were cut (move) or copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileClip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

#[cfg(windows)]
mod clip {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::PathBuf,
        thread,
        time::Duration,
    };

    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    use windows::core::w;

    use super::FileClip;

    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;

    fn open() -> bool {
        // Another program may hold the clipboard for a moment.
        for _ in 0..8 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return true;
            }
            thread::sleep(Duration::from_millis(8));
        }
        false
    }

    /// Copies `bytes` into a movable global block and hands it to the clipboard.
    unsafe fn put(format: u32, bytes: &[u8]) -> bool {
        unsafe {
            let Ok(h) = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()) else {
                return false;
            };
            let p = GlobalLock(h);
            if p.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(h);
            SetClipboardData(format, Some(HANDLE(h.0))).is_ok()
        }
    }

    pub fn set_files(paths: &[PathBuf], cut: bool) -> bool {
        // DROPFILES header (20 bytes, wide chars) + double-NUL-terminated list.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(&20u32.to_le_bytes()); // pFiles
        bytes.extend_from_slice(&[0u8; 8]); // pt
        bytes.extend_from_slice(&0u32.to_le_bytes()); // fNC
        bytes.extend_from_slice(&1u32.to_le_bytes()); // fWide
        for p in paths {
            for unit in p.as_os_str().encode_wide() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());

        if !open() {
            return false;
        }
        let ok = unsafe {
            let _ = EmptyClipboard();
            let effect = if cut {
                DROPEFFECT_MOVE
            } else {
                DROPEFFECT_COPY
            };
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            put(CF_HDROP.0 as u32, &bytes) && put(fmt, &effect.to_le_bytes())
        };
        let _ = unsafe { CloseClipboard() };
        ok
    }

    pub fn files() -> Option<FileClip> {
        unsafe {
            if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_err() || !open() {
                return None;
            }
            let result = (|| {
                let handle = GetClipboardData(CF_HDROP.0 as u32).ok()?;
                let drop = HDROP(handle.0);
                let n = DragQueryFileW(drop, u32::MAX, None);
                let mut paths = Vec::new();
                for i in 0..n {
                    let len = DragQueryFileW(drop, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                    paths.push(PathBuf::from(OsString::from_wide(
                        &buf[..got.min(buf.len())],
                    )));
                }
                let mut cut = false;
                let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
                if let Ok(h) = GetClipboardData(fmt) {
                    let g = HGLOBAL(h.0);
                    let p = GlobalLock(g).cast::<u32>();
                    if !p.is_null() {
                        cut = p.read_unaligned() & DROPEFFECT_MOVE != 0;
                        let _ = GlobalUnlock(g);
                    }
                }
                Some(FileClip { paths, cut })
            })();
            let _ = CloseClipboard();
            result
        }
    }

    pub fn clear() {
        if open() {
            unsafe {
                let _ = EmptyClipboard();
                let _ = CloseClipboard();
            }
        }
    }

    pub fn sequence() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }
}

/// Puts files on the OS clipboard (Explorer can paste them). `false` when it is unavailable.
pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> bool {
    #[cfg(windows)]
    {
        clip::set_files(paths, cut)
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, cut);
        false
    }
}

/// Files currently on the OS clipboard, if any.
pub fn clipboard_files() -> Option<FileClip> {
    #[cfg(windows)]
    {
        clip::files().filter(|c| !c.paths.is_empty())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn clipboard_clear() {
    #[cfg(windows)]
    clip::clear();
}

/// Changes whenever any program writes to the clipboard (0 where unsupported).
pub fn clipboard_sequence() -> u32 {
    #[cfg(windows)]
    {
        clip::sequence()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

// --- Drives ---------------------------------------------------------------------------------

#[cfg(windows)]
pub fn drives() -> Arc<[DriveInfo]> {
    use windows::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    use windows::core::PCWSTR;

    const DRIVE_REMOTE: u32 = 4;
    let mask = unsafe { GetLogicalDrives() };
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };

        // Network drives can hang for seconds: ask on a helper thread and give up quickly.
        let query = move || {
            let wide: Vec<u16> = root_wide(letter);
            let (mut total, mut free, mut avail) = (0u64, 0u64, 0u64);
            let ok = unsafe {
                GetDiskFreeSpaceExW(
                    PCWSTR(wide.as_ptr()),
                    Some(&mut avail),
                    Some(&mut total),
                    Some(&mut free),
                )
            }
            .is_ok();
            let mut name = [0u16; 261];
            let mut fs_buf = [0u16; 64];
            let named = unsafe {
                GetVolumeInformationW(
                    PCWSTR(wide.as_ptr()),
                    Some(&mut name),
                    None,
                    None,
                    None,
                    Some(&mut fs_buf),
                )
            }
            .is_ok();
            let fs = if named {
                let n = fs_buf.iter().position(|&c| c == 0).unwrap_or(fs_buf.len());
                String::from_utf16_lossy(&fs_buf[..n])
            } else {
                String::new()
            };
            let label = if named {
                let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                String::from_utf16_lossy(&name[..n])
            } else {
                String::new()
            };
            (ok, total, free, label, fs)
        };
        let (ok, total, free, label, fs) = if kind == DRIVE_REMOTE {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(query());
            });
            rx.recv_timeout(Duration::from_millis(800)).unwrap_or((
                false,
                0,
                0,
                String::new(),
                String::new(),
            ))
        } else {
            query()
        };
        let name = if label.is_empty() {
            "Local Disk".to_owned()
        } else {
            label
        };
        let used = if ok && total > 0 {
            (total - free.min(total)) as f32 / total as f32
        } else {
            0.0
        };
        out.push(DriveInfo {
            label: format!("{name} ({letter}:)"),
            root: PathBuf::from(format!("{letter}:\\")),
            total,
            free,
            used,
            system: format!("{letter}:").eq_ignore_ascii_case(&system),
            fs,
        });
    }
    Arc::from(out)
}

#[cfg(windows)]
fn root_wide(letter: char) -> Vec<u16> {
    format!("{letter}:\\")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(not(windows))]
pub fn drives() -> Arc<[DriveInfo]> {
    Arc::from(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_state_round_trip_and_validation() {
        let w = WindowState {
            w: 1152.0,
            h: 760.0,
            maximized: true,
        };
        assert_eq!(WindowState::parse(&w.to_text()), Some(w));
        assert_eq!(WindowState::parse("0 0 0"), None);
        assert_eq!(WindowState::parse("99999 700 0"), None);
        assert_eq!(WindowState::parse("garbage"), None);
        assert_eq!(
            WindowState::parse("800 600").map(|s| s.maximized),
            Some(false)
        );
    }
}
