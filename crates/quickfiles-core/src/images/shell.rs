//! Windows shell icons and thumbnails (COM). Everything returns straight-alpha RGBA.

use super::Pixels;

#[cfg(windows)]
mod imp {
    use super::Pixels;
    use std::{ffi::c_void, mem::size_of};
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
        GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
    };
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize, IBindCtx,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SHFILEINFOW, SHGFI_ICON,
        SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SIIGBF_BIGGERSIZEOK,
        SIIGBF_THUMBNAILONLY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
    use windows::core::PCWSTR;

    /// COM must be initialised on every thread that creates shell items.
    pub struct ComGuard;
    impl ComGuard {
        pub fn new() -> Self {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            ComGuard
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// HBITMAP -> (w, h, top-down BGRA)
    unsafe fn bitmap_bgra(hbm: HBITMAP) -> Option<(usize, usize, Vec<u8>)> {
        let mut bm = BITMAP::default();
        let n = unsafe {
            GetObjectW(
                HGDIOBJ(hbm.0),
                size_of::<BITMAP>() as i32,
                Some(&mut bm as *mut BITMAP as *mut c_void),
            )
        };
        if n == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        };
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let hdc = unsafe { GetDC(None) };
        let got = unsafe {
            GetDIBits(
                hdc,
                hbm,
                0,
                h as u32,
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut bi,
                DIB_RGB_COLORS,
            )
        };
        unsafe { ReleaseDC(None, hdc) };
        (got != 0).then_some((w as usize, h as usize, buf))
    }

    unsafe fn hicon_to_pixels(hicon: HICON) -> Option<Pixels> {
        let mut ii = ICONINFO::default();
        unsafe { GetIconInfo(hicon, &mut ii).ok()? };
        let color = unsafe { bitmap_bgra(ii.hbmColor) };
        let mask = unsafe { bitmap_bgra(ii.hbmMask) };
        unsafe {
            if !ii.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
            }
            if !ii.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
            }
        }
        let (w, h, mut px) = color?;
        if !px.chunks_exact(4).any(|p| p[3] != 0) {
            // Old-style icon without alpha: the AND mask decides (white = transparent).
            match mask {
                Some((mw, mh, m)) if mw == w && mh == h => {
                    for (p, mp) in px.chunks_exact_mut(4).zip(m.chunks_exact(4)) {
                        p[3] = if mp[0] == 0 { 255 } else { 0 };
                    }
                }
                _ => px.chunks_exact_mut(4).for_each(|p| p[3] = 255),
            }
        }
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2); // BGRA -> RGBA
        }
        Some(Pixels {
            premultiplied: false,
            w: w as u32,
            h: h as u32,
            rgba: px,
        })
    }

    /// 32x32 shell icon. `by_attrs` resolves it from the extension alone (no disk access);
    /// otherwise `lookup` must be a real path (exe, lnk, drives...).
    pub fn icon(lookup: &str, folder: bool, by_attrs: bool) -> Option<Pixels> {
        unsafe {
            let w = wide(lookup);
            let mut sfi = SHFILEINFOW::default();
            let mut flags = SHGFI_ICON | SHGFI_LARGEICON;
            let attrs = if by_attrs {
                flags |= SHGFI_USEFILEATTRIBUTES;
                if folder {
                    FILE_ATTRIBUTE_DIRECTORY
                } else {
                    FILE_ATTRIBUTE_NORMAL
                }
            } else {
                FILE_FLAGS_AND_ATTRIBUTES(0)
            };
            let r = SHGetFileInfoW(
                PCWSTR(w.as_ptr()),
                attrs,
                Some(&mut sfi as *mut SHFILEINFOW),
                size_of::<SHFILEINFOW>() as u32,
                flags,
            );
            if r == 0 || sfi.hIcon.is_invalid() {
                return None;
            }
            let img = hicon_to_pixels(sfi.hIcon);
            let _ = DestroyIcon(sfi.hIcon);
            img
        }
    }

    /// Shell thumbnail (video, pdf, office...). `None` when the type has no thumbnail handler.
    pub fn thumb(path: &str, px: i32) -> Option<Pixels> {
        unsafe {
            let w = wide(path);
            let f: IShellItemImageFactory =
                SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None::<&IBindCtx>).ok()?;
            let hbm = f
                .GetImage(
                    SIZE { cx: px, cy: px },
                    SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
                )
                .ok()?;
            let r = bitmap_bgra(hbm);
            let _ = DeleteObject(HGDIOBJ(hbm.0));
            let (w, h, mut p) = r?;
            // Premultiplied ARGB from the shell; some handlers leave alpha at 0.
            let any_alpha = p.chunks_exact(4).any(|c| c[3] != 0);
            for c in p.chunks_exact_mut(4) {
                c.swap(0, 2);
                if !any_alpha {
                    c[3] = 255;
                } else if c[3] != 0 && c[3] != 255 {
                    let a = c[3] as u32;
                    for v in &mut c[..3] {
                        *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            Some(Pixels {
                premultiplied: false,
                w: w as u32,
                h: h as u32,
                rgba: p,
            })
        }
    }
}

#[cfg(windows)]
pub use imp::{ComGuard, icon, thumb};

#[cfg(not(windows))]
pub struct ComGuard;
#[cfg(not(windows))]
impl ComGuard {
    pub fn new() -> Self {
        ComGuard
    }
}
#[cfg(not(windows))]
pub fn icon(_: &str, _: bool, _: bool) -> Option<Pixels> {
    None
}
#[cfg(not(windows))]
pub fn thumb(_: &str, _: i32) -> Option<Pixels> {
    None
}
