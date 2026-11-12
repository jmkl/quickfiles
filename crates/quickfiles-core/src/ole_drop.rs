//! Receiving drags from other programs (Windows OLE `IDropTarget`).
//!
//! winit registers its own drop target on the window, which only understands `CF_HDROP` (files
//! from Explorer). A browser drag carries the image address in other formats, so [`DropTarget`]
//! replaces it: it takes both kinds, so files from Explorer keep working through the same path.
//! The callbacks run on the UI thread; they only decode the data object (`dropdata.rs`) and put
//! the result in a queue that the app polls once per frame. Nothing here touches the disk or the
//! network: copying and downloading are jobs (`jobs.rs`).
//!
//! Elsewhere (and when registration fails) [`DropTarget::register`] returns `None` and egui's
//! own dropped-files events are used instead.

use std::sync::{Arc, Mutex};

use crate::{dropdata::Dropped, engine::Wake};

/// What is being dragged over the window right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hover {
    /// This many files / folders from Explorer.
    Files(usize),
    /// An image from a web page.
    Image,
    /// Something we cannot use (the banner says so).
    Other,
}

pub fn hover_of(d: &Dropped) -> Hover {
    match d {
        Dropped::Files(f) => Hover::Files(f.len()),
        Dropped::Image(_) => Hover::Image,
        Dropped::Unsupported(_) => Hover::Other,
    }
}

#[derive(Default)]
struct Shared {
    hover: Mutex<Option<Hover>>,
    /// Pointer position in window-client pixels while a drag is over the window.
    pos: Mutex<Option<(i32, i32)>>,
    drops: Mutex<Vec<Dropped>>,
}

/// Keeps the drop target registered; dropping it unregisters.
pub struct DropTarget {
    shared: Arc<Shared>,
    _imp: imp::Registration,
}

impl DropTarget {
    /// Registers on the top-level window titled `window_title`. Call on the UI thread.
    pub fn register(window_title: &str, wake: Wake) -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let imp = imp::Registration::new(window_title, shared.clone(), wake)?;
        Some(Self { shared, _imp: imp })
    }

    /// The drag currently over the window, if any.
    pub fn hover(&self) -> Option<Hover> {
        self.shared
            .hover
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Where the pointer is, in window pixels (divide by the scale factor for egui points).
    pub fn hover_pos(&self) -> Option<(i32, i32)> {
        *self.shared.pos.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drops completed since the last call.
    pub fn take_drops(&self) -> Vec<Dropped> {
        std::mem::take(&mut *self.shared.drops.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::Arc;

    use windows::{
        Win32::{
            Foundation::{HWND, POINT, POINTL},
            Graphics::Gdi::ScreenToClient,
            System::{
                Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL},
                DataExchange::RegisterClipboardFormatW,
                Memory::{GlobalLock, GlobalSize, GlobalUnlock},
                Ole::{
                    DROPEFFECT, DROPEFFECT_COPY, IDropTarget, IDropTarget_Impl, RegisterDragDrop,
                    ReleaseStgMedium, RevokeDragDrop,
                },
                SystemServices::MODIFIERKEYS_FLAGS,
            },
            UI::WindowsAndMessaging::FindWindowW,
        },
        core::{PCWSTR, Ref, implement, w},
    };

    use super::{Shared, hover_of};
    use crate::{
        dropdata::{self, Dropped, Formats, Html},
        engine::Wake,
    };

    const CF_UNICODETEXT: u16 = 13;
    const CF_HDROP: u16 = 15;

    pub struct Registration {
        hwnd: HWND,
        _target: IDropTarget,
    }

    impl Registration {
        pub fn new(title: &str, shared: Arc<Shared>, wake: Wake) -> Option<Self> {
            let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` is NUL-terminated and outlives the call.
            let hwnd = unsafe { FindWindowW(None, PCWSTR(wide.as_ptr())) }.ok()?;
            let target: IDropTarget = Target { shared, wake, hwnd }.into();
            // SAFETY: plain OLE calls on the window this thread created. winit's own target must
            // go first: a window has exactly one.
            unsafe {
                let _ = RevokeDragDrop(hwnd);
                RegisterDragDrop(hwnd, &target).ok()?;
            }
            Some(Self {
                hwnd,
                _target: target,
            })
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            // SAFETY: unregisters what `new` registered.
            unsafe {
                let _ = RevokeDragDrop(self.hwnd);
            }
        }
    }

    #[implement(IDropTarget)]
    struct Target {
        shared: Arc<Shared>,
        wake: Wake,
        hwnd: HWND,
    }

    impl Target {
        /// Remembers the pointer (screen coordinates from OLE -> window pixels); repaints when
        /// it moved, so the folder under it can light up.
        fn set_pos(&self, pt: &POINTL) {
            let mut p = POINT { x: pt.x, y: pt.y };
            // SAFETY: converts a point for our own window.
            let ok = unsafe { ScreenToClient(self.hwnd, &mut p) }.as_bool();
            let new = ok.then_some((p.x, p.y));
            let mut cur = self.shared.pos.lock().unwrap_or_else(|e| e.into_inner());
            if *cur != new {
                *cur = new;
                drop(cur);
                (self.wake)();
            }
        }

        fn set_hover(&self, h: Option<super::Hover>) {
            *self.shared.hover.lock().unwrap_or_else(|e| e.into_inner()) = h;
            (self.wake)();
        }
    }

    #[allow(non_snake_case)]
    impl IDropTarget_Impl for Target_Impl {
        fn DragEnter(
            &self,
            pdataobj: Ref<'_, IDataObject>,
            _keys: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            self.set_pos(pt);
            // SAFETY: the data object is valid for the duration of the callback.
            let hover = pdataobj
                .as_ref()
                .map(|d| hover_of(&dropdata::classify(&unsafe { gather(d) })));
            self.set_hover(hover);
            set_effect(pdweffect);
            Ok(())
        }

        fn DragOver(
            &self,
            _keys: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            self.set_pos(pt);
            set_effect(pdweffect);
            Ok(())
        }

        fn DragLeave(&self) -> windows::core::Result<()> {
            *self.shared.pos.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.set_hover(None);
            Ok(())
        }

        fn Drop(
            &self,
            pdataobj: Ref<'_, IDataObject>,
            _keys: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            self.set_pos(pt);
            // SAFETY: as in DragEnter.
            let dropped = pdataobj
                .as_ref()
                .map(|d| dropdata::classify(&unsafe { gather(d) }))
                .unwrap_or_else(|| Dropped::Unsupported("the drop carried no data".to_owned()));
            self.shared
                .drops
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(dropped);
            set_effect(pdweffect);
            self.set_hover(None);
            Ok(())
        }
    }

    /// Everything is copied (we never move out of the other program).
    fn set_effect(effect: *mut DROPEFFECT) {
        if !effect.is_null() {
            // SAFETY: OLE passes a valid pointer to the effect.
            unsafe { *effect = DROPEFFECT_COPY };
        }
    }

    fn format(name: PCWSTR) -> u16 {
        // SAFETY: `name` is a NUL-terminated literal. Registered ids always fit 16 bits.
        unsafe { RegisterClipboardFormatW(name) as u16 }
    }

    /// The bytes of one clipboard format in global memory, if the source offers it.
    unsafe fn read(data: &IDataObject, cf: u16) -> Option<Vec<u8>> {
        let fe = FORMATETC {
            cfFormat: cf,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0 as u32,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        unsafe {
            if data.QueryGetData(&fe).is_err() {
                return None;
            }
            let mut stg = data.GetData(&fe).ok()?;
            let h = stg.u.hGlobal;
            let size = GlobalSize(h);
            let p = GlobalLock(h).cast::<u8>();
            let bytes = (!p.is_null()).then(|| std::slice::from_raw_parts(p, size).to_vec());
            let _ = GlobalUnlock(h);
            ReleaseStgMedium(&mut stg);
            bytes
        }
    }

    /// Reads every format we understand.
    unsafe fn gather(data: &IDataObject) -> Formats {
        unsafe {
            let html = read(data, format(w!("HTML Format")))
                .map(|b| dropdata::parse_cf_html(&b))
                // Firefox: plain UTF-16 HTML without the header.
                .or_else(|| {
                    read(data, format(w!("text/html"))).map(|b| Html {
                        source_url: None,
                        fragment: dropdata::utf16_text(&b),
                    })
                });
            Formats {
                files: read(data, CF_HDROP)
                    .map(|b| dropdata::parse_dropfiles(&b))
                    .unwrap_or_default(),
                html,
                url: read(data, format(w!("UniformResourceLocatorW")))
                    .map(|b| dropdata::utf16_text(&b))
                    .or_else(|| {
                        read(data, format(w!("UniformResourceLocator")))
                            .map(|b| dropdata::ansi_text(&b))
                    }),
                moz_url: read(data, format(w!("text/x-moz-url"))).map(|b| dropdata::utf16_text(&b)),
                moz_image_url: read(data, format(w!("application/x-moz-file-promise-url")))
                    .map(|b| dropdata::utf16_text(&b)),
                text: read(data, CF_UNICODETEXT).map(|b| dropdata::utf16_text(&b)),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::Arc;

    use super::{Shared, Wake};

    pub struct Registration;

    impl Registration {
        pub fn new(_: &str, _: Arc<Shared>, _: Wake) -> Option<Self> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dropdata::ImageSource;

    #[test]
    fn hover_summarises_a_drop() {
        assert_eq!(
            hover_of(&Dropped::Files(vec!["a".into(), "b".into()])),
            Hover::Files(2)
        );
        assert_eq!(
            hover_of(&Dropped::Image(ImageSource {
                url: "https://a/b.png".into(),
                referer: None
            })),
            Hover::Image
        );
        assert_eq!(hover_of(&Dropped::Unsupported("x".into())), Hover::Other);
    }
}
