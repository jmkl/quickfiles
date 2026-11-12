//! Dragging files out of the window into other programs (Windows OLE drag source).
//!
//! The data object comes from the shell (`IShellItemArray` -> `BHID_DataObject`), so Explorer,
//! browsers, chat apps and editors see the same formats they get from an Explorer drag
//! (`CF_HDROP` and the shell ones). [`start`] runs the drag loop and returns when the button is
//! released.

use std::{path::PathBuf, thread};

/// Appends one line to `drag-out.log` in the data folder: a freeze here is hard to see from
/// outside, so every step is written down (the file is truncated at each new drag).
fn log(line: &str) {
    use std::io::Write;
    let path = crate::sys::data_dir().join("drag-out.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() % 1_000_000);
        let _ = writeln!(
            f,
            "{t:>6} {} {line}",
            thread::current().name().unwrap_or("?")
        );
    }
}

/// How a drag out ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Copied,
    Moved,
    Linked,
    /// Escape, or dropped where nothing accepts it.
    Cancelled,
}

/// Runs a drag of `paths` and returns when the left mouse button is released. Call on the UI
/// thread: the drag loop reads the button state of the thread it runs on, so on any other thread
/// it sees the button as already up and ends at once. The window is unresponsive meanwhile.
pub fn start(paths: &[PathBuf]) -> Result<Outcome, String> {
    let _ = std::fs::remove_file(crate::sys::data_dir().join("drag-out.log"));
    log(&format!("start: {} path(s)", paths.len()));
    if paths.is_empty() {
        return Err("nothing to drag".to_owned());
    }
    let result = imp::start(paths);
    log(&format!("finished: {result:?}"));
    result
}

#[cfg(windows)]
mod imp {
    use std::{ffi::c_void, path::PathBuf};

    use windows::{
        Win32::{
            Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK},
            System::{
                Com::{CoTaskMemFree, IBindCtx, IDataObject},
                Ole::{
                    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
                    DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize,
                },
                SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
            },
            UI::Shell::{
                BHID_DataObject, Common::ITEMIDLIST, IShellItemArray,
                SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
            },
        },
        core::{BOOL, HRESULT, PCWSTR, implement},
    };

    use super::{Outcome, log};

    #[implement(IDropSource)]
    struct Source;

    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool() {
                super::log("escape pressed");
                DRAGDROP_S_CANCEL
            } else if keys.0 & MK_LBUTTON.0 == 0 {
                super::log("button released");
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// Frees the shell's item ids when the drag is over.
    struct Pidls(Vec<*mut ITEMIDLIST>);

    impl Drop for Pidls {
        fn drop(&mut self) {
            for p in &self.0 {
                // SAFETY: allocated by SHParseDisplayName, freed once.
                unsafe { CoTaskMemFree(Some(*p as *const c_void)) };
            }
        }
    }

    pub fn start(paths: &[PathBuf]) -> Result<Outcome, String> {
        // SAFETY: balanced below; this thread only exists for the drag.
        unsafe {
            let _ = OleInitialize(None);
        }
        log("ole initialised");
        let result = run(paths);
        // SAFETY: matches the OleInitialize above.
        unsafe { OleUninitialize() };
        result
    }

    fn run(paths: &[PathBuf]) -> Result<Outcome, String> {
        let mut pidls = Pidls(Vec::new());
        for path in paths {
            let wide: Vec<u16> = path
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            // SAFETY: `wide` is NUL-terminated; `pidl` receives a shell allocation.
            let parsed = unsafe {
                SHParseDisplayName(PCWSTR(wide.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None)
            };
            if parsed.is_ok() && !pidl.is_null() {
                pidls.0.push(pidl);
            }
        }
        log(&format!("{} item id(s) parsed", pidls.0.len()));
        if pidls.0.is_empty() {
            return Err("those items cannot be dragged".to_owned());
        }
        let list: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|p| *p as *const _).collect();
        // SAFETY: the item ids stay alive (in `pidls`) for the whole drag.
        let data: IDataObject = unsafe {
            let array: IShellItemArray =
                SHCreateShellItemArrayFromIDLists(&list).map_err(|e| e.to_string())?;
            array
                .BindToHandler(None::<&IBindCtx>, &BHID_DataObject)
                .map_err(|e| e.to_string())?
        };
        log("data object ready");
        let source: IDropSource = Source.into();
        let mut effect = DROPEFFECT_NONE;
        // SAFETY: blocks in the OLE drag loop on this (UI) thread; both objects outlive it.
        log("entering DoDragDrop");
        let result = unsafe {
            DoDragDrop(
                &data,
                &source,
                DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
                &mut effect,
            )
        };
        log(&format!(
            "DoDragDrop returned {result:?}, effect {}",
            effect.0
        ));
        if result == DRAGDROP_S_DROP {
            Ok(if effect == DROPEFFECT_MOVE {
                Outcome::Moved
            } else if effect == DROPEFFECT_COPY {
                Outcome::Copied
            } else if effect == DROPEFFECT_LINK {
                Outcome::Linked
            } else {
                Outcome::Cancelled
            })
        } else if result == DRAGDROP_S_CANCEL {
            Ok(Outcome::Cancelled)
        } else {
            Err(format!("drag failed ({result:?})"))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    use super::Outcome;

    pub fn start(_: &[PathBuf]) -> Result<Outcome, String> {
        Err("dragging out is only available on Windows".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_drag_is_an_error() {
        assert!(start(&[]).is_err());
    }
}
