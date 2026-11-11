//! Decoding. JPEGs (the common case) are decoded at a reduced size through DCT scaling, or not
//! decoded at all when the camera embedded a thumbnail; everything else goes through the `image`
//! crate, and types it cannot read go to the Windows shell (see [`route`]).
//!
//! Every entry point takes a `cancel` callback that is polled between the steps (read, decode,
//! resize), so a preview the user has already moved away from stops early.

use std::{io::Read, path::Path};

use super::Pixels;

/// Files larger than this are not decoded in-process (the shell handles them).
pub const MAX_FILE: u64 = 96 * 1024 * 1024;
/// Pixel limit for in-process decoding, so a 200 MP image cannot exhaust memory.
pub const MAX_PIXELS: u64 = 100_000_000;
/// The EXIF block lives in the first segments of a JPEG; this much is enough to find it.
const EXIF_HEAD: usize = 70 * 1024;

/// Why no pixels came back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// The caller no longer wants the result. Not a property of the file: do not remember it.
    Cancelled,
    /// Unreadable, unsupported or over the limits.
    Failed,
}

pub type Decoded = Result<Pixels, Fail>;

/// Which decoder handles a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    /// JPEG: scaled decode / embedded EXIF thumbnail.
    Jpeg,
    /// PNG, GIF, WebP, BMP through the `image` crate.
    Image,
    /// Everything else (RAW, HEIC, PDF, video, Office, oversized files): the shell's thumbnails.
    Shell,
}

pub fn route(ext: &str, len: u64) -> Route {
    if len > MAX_FILE {
        return Route::Shell;
    }
    match ext {
        "jpg" | "jpeg" | "jpe" | "jfif" => Route::Jpeg,
        "png" | "gif" | "webp" | "bmp" => Route::Image,
        _ => Route::Shell,
    }
}

pub fn is_decodable(ext: &str) -> bool {
    route(ext, 0) != Route::Shell
}

fn check(cancel: &dyn Fn() -> bool) -> Result<(), Fail> {
    if cancel() {
        Err(Fail::Cancelled)
    } else {
        Ok(())
    }
}

fn read_all(path: &Path) -> Result<Vec<u8>, Fail> {
    std::fs::read(path).map_err(|_| Fail::Failed)
}

fn read_head(path: &Path, n: usize) -> Result<Vec<u8>, Fail> {
    let f = std::fs::File::open(path).map_err(|_| Fail::Failed)?;
    let mut buf = Vec::new();
    f.take(n as u64)
        .read_to_end(&mut buf)
        .map_err(|_| Fail::Failed)?;
    Ok(buf)
}

/// The size that fits `(w, h)` into `max_side` on the longest side, never upscaling.
pub fn fit(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let long = w.max(h);
    if long == 0 || long <= max_side {
        return (w, h);
    }
    let k = max_side as f64 / long as f64;
    (
        ((w as f64 * k).round() as u32).max(1),
        ((h as f64 * k).round() as u32).max(1),
    )
}

// --- Entry points ---------------------------------------------------------------------------

/// A thumbnail of at most `px` on the longest side.
pub fn thumbnail(path: &Path, ext: &str, len: u64, px: u32, cancel: &dyn Fn() -> bool) -> Decoded {
    match route(ext, len) {
        Route::Jpeg => {
            // 1. The embedded EXIF thumbnail: reads 70 KB instead of the whole file.
            let head = read_head(path, EXIF_HEAD)?;
            if let Some(t) = exif_thumbnail(&head)
                && let Ok(p) = jpeg_scaled(t, px, Some(px), cancel)
            {
                return Ok(p);
            }
            check(cancel)?;
            // 2. Scaled decode of the real image.
            jpeg_scaled(&read_all(path)?, px, None, cancel).or_else(|f| {
                if f == Fail::Cancelled {
                    Err(f)
                } else {
                    generic(path, px, cancel)
                }
            })
        }
        Route::Image => generic(path, px, cancel),
        Route::Shell => Err(Fail::Failed),
    }
}

/// A preview of at most `max_side` on the longest side.
pub fn preview(
    path: &Path,
    ext: &str,
    len: u64,
    max_side: u32,
    cancel: &dyn Fn() -> bool,
) -> Decoded {
    match route(ext, len) {
        Route::Jpeg => jpeg_scaled(&read_all(path)?, max_side, None, cancel).or_else(|f| {
            if f == Fail::Cancelled {
                Err(f)
            } else {
                generic(path, max_side, cancel)
            }
        }),
        Route::Image => generic(path, max_side, cancel),
        Route::Shell => Err(Fail::Failed),
    }
}

// --- JPEG -----------------------------------------------------------------------------------

/// Decode a JPEG at the smallest DCT scale that still covers `max_side`, then area-average the
/// rest of the way down. With `min_embedded` set (EXIF thumbnails) the result is refused when
/// the picture is smaller than that, so a tiny embedded thumbnail is not stretched.
fn jpeg_scaled(
    data: &[u8],
    max_side: u32,
    min_embedded: Option<u32>,
    cancel: &dyn Fn() -> bool,
) -> Decoded {
    check(cancel)?;
    let mut d = jpeg_decoder::Decoder::new(std::io::Cursor::new(data));
    let req = max_side.clamp(1, u16::MAX as u32) as u16;
    let (sw, sh) = d.scale(req, req).map_err(|_| Fail::Failed)?;
    let (sw, sh) = (sw as u32, sh as u32);
    if sw as u64 * sh as u64 > MAX_PIXELS {
        return Err(Fail::Failed);
    }
    if let Some(min) = min_embedded {
        // `scale` never goes below the full size of a small image, so this is the real size.
        if sw.max(sh) < min {
            return Err(Fail::Failed);
        }
    }
    check(cancel)?;
    let buf = d.decode().map_err(|_| Fail::Failed)?;
    let info = d.info().ok_or(Fail::Failed)?;
    let ch = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => 3,
        jpeg_decoder::PixelFormat::L8 => 1,
        // CMYK and 16-bit grey: leave them to the `image` crate.
        _ => return Err(Fail::Failed),
    };
    if buf.len() != sw as usize * sh as usize * ch {
        return Err(Fail::Failed);
    }
    check(cancel)?;
    let (dw, dh) = fit(sw, sh, max_side);
    Ok(Pixels::new(dw, dh, area_resize(&buf, sw, sh, ch, dw, dh)))
}

/// Area-average resize of a tightly packed 1- or 3-channel image to RGBA. Two separable passes
/// with exact fractional coverage: a good filter for large reductions, and fast because the
/// input is already small after DCT scaling. `dw`/`dh` must not exceed `sw`/`sh`.
pub fn area_resize(src: &[u8], sw: u32, sh: u32, ch: usize, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let px = |s: &[u8], i: usize, c: usize| -> f32 { s[i * ch + c.min(ch - 1)] as f32 };

    // Horizontal pass: dw x sh, `ch` channels, 8 bit.
    let xs = spans(sw, dw);
    let mut mid = vec![0u8; dw * sh * ch];
    for y in 0..sh {
        let row = &src[y * sw * ch..(y + 1) * sw * ch];
        for (x, sp) in xs.iter().enumerate() {
            for c in 0..ch {
                let mut acc = 0.0;
                for (k, w) in sp.weights.iter().enumerate() {
                    acc += px(row, sp.start + k, c) * w;
                }
                mid[(y * dw + x) * ch + c] = (acc + 0.5) as u8;
            }
        }
    }

    // Vertical pass into RGBA.
    let ys = spans(sh, dh);
    let mut out = vec![255u8; dw * dh * 4];
    for (y, sp) in ys.iter().enumerate() {
        for x in 0..dw {
            let mut rgb = [0.0f32; 3];
            for (k, w) in sp.weights.iter().enumerate() {
                let i = (sp.start + k) * dw + x;
                for (c, v) in rgb.iter_mut().enumerate() {
                    *v += mid[i * ch + c.min(ch - 1)] as f32 * w;
                }
            }
            let o = (y * dw + x) * 4;
            for c in 0..3 {
                out[o + c] = (rgb[c] + 0.5).min(255.0) as u8;
            }
        }
    }
    out
}

struct Span {
    start: usize,
    weights: Vec<f32>,
}

/// For each destination pixel, the source pixels it covers and their normalised weights.
fn spans(s: usize, d: usize) -> Vec<Span> {
    let scale = s as f64 / d as f64;
    (0..d)
        .map(|i| {
            let (a, b) = (i as f64 * scale, ((i + 1) as f64 * scale).min(s as f64));
            let first = a.floor() as usize;
            let last = ((b.ceil() as usize).max(first + 1)).min(s);
            let mut weights: Vec<f32> = (first..last)
                .map(|j| ((b.min(j as f64 + 1.0)) - a.max(j as f64)).max(0.0) as f32)
                .collect();
            let total: f32 = weights.iter().sum();
            if total > 0.0 {
                weights.iter_mut().for_each(|w| *w /= total);
            } else {
                weights = vec![1.0];
            }
            Span {
                start: first,
                weights,
            }
        })
        .collect()
}

// --- EXIF thumbnail -------------------------------------------------------------------------

/// The JPEG thumbnail embedded in an Exif block (IFD1), if there is one. `jpeg` only needs to
/// be the first part of the file.
pub fn exif_thumbnail(jpeg: &[u8]) -> Option<&[u8]> {
    if jpeg.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    while i + 4 <= jpeg.len() {
        if jpeg[i] != 0xFF {
            return None;
        }
        let m = jpeg[i + 1];
        if m == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            return None; // end of image / start of scan: no Exif before the pixels
        }
        if m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2; // markers without a length
            continue;
        }
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let end = i + 2 + len;
        let seg = jpeg.get(i + 4..end)?;
        if m == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return tiff_thumbnail(&seg[6..]);
        }
        i = end;
    }
    None
}

fn tiff_thumbnail(t: &[u8]) -> Option<&[u8]> {
    let be = match t.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u32> {
        let b: [u8; 2] = t.get(o..o + 2)?.try_into().ok()?;
        Some(if be {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        } as u32)
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b: [u8; 4] = t.get(o..o + 4)?.try_into().ok()?;
        Some(if be {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd0 = u32_at(4)? as usize;
    let n0 = u16_at(ifd0)? as usize;
    let ifd1 = u32_at(ifd0.checked_add(2 + n0 * 12)?)? as usize;
    if ifd1 == 0 {
        return None;
    }
    let n1 = u16_at(ifd1)? as usize;
    let (mut offset, mut length) = (None, None);
    for e in 0..n1.min(64) {
        let at = ifd1 + 2 + e * 12;
        let value = || match u16_at(at + 2)? {
            3 => u16_at(at + 8),
            _ => u32_at(at + 8),
        };
        match u16_at(at)? {
            0x0201 => offset = value(),
            0x0202 => length = value(),
            _ => {}
        }
    }
    let (o, l) = (offset? as usize, length? as usize);
    let jpg = t.get(o..o.checked_add(l)?)?;
    (jpg.get(0..2)? == [0xFF, 0xD8]).then_some(jpg)
}

// --- Generic (`image` crate) ----------------------------------------------------------------

fn generic(path: &Path, max_side: u32, cancel: &dyn Fn() -> bool) -> Decoded {
    check(cancel)?;
    let bytes = read_all(path)?;
    check(cancel)?;
    let reader = || {
        image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|_| Fail::Failed)
    };
    let (w, h) = reader()?.into_dimensions().map_err(|_| Fail::Failed)?;
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(Fail::Failed);
    }
    let img = reader()?.decode().map_err(|_| Fail::Failed)?;
    check(cancel)?;
    let img = if img.width().max(img.height()) > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Ok(Pixels::new(rgba.width(), rgba.height(), rgba.into_raw()))
}

// --- PNG (kept for tests and tools; the disk cache stores raw pixels) ------------------------

pub fn encode_png(p: &Pixels) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let img = image::RgbaImage::from_raw(p.w, p.h, p.rgba.clone())?;
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_by_type_and_size() {
        assert_eq!(route("jpg", 10), Route::Jpeg);
        assert_eq!(route("jpeg", 10), Route::Jpeg);
        assert_eq!(route("png", 10), Route::Image);
        assert_eq!(route("webp", 10), Route::Image);
        for shell in ["heic", "cr2", "nef", "pdf", "mp4", "docx", "psd", ""] {
            assert_eq!(route(shell, 10), Route::Shell, "{shell}");
        }
        assert_eq!(route("jpg", MAX_FILE + 1), Route::Shell);
        assert!(is_decodable("png") && !is_decodable("pdf"));
    }

    #[test]
    fn fit_never_upscales_and_keeps_the_aspect() {
        assert_eq!(fit(6000, 4000, 96), (96, 64));
        assert_eq!(fit(4000, 6000, 96), (64, 96));
        assert_eq!(fit(50, 40, 96), (50, 40));
        assert_eq!(fit(10000, 1, 96), (96, 1));
    }

    #[test]
    fn area_resize_averages_a_checkerboard() {
        // 4x4 grey checkerboard of 0 / 200 reduced to 2x2: every cell is the mean (100).
        let mut src = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                src.push(if (x + y) % 2 == 0 { 0 } else { 200 });
            }
        }
        let out = area_resize(&src, 4, 4, 1, 2, 2);
        assert_eq!(out.len(), 16);
        for px in out.chunks(4) {
            assert_eq!(px, [100, 100, 100, 255]);
        }
    }

    #[test]
    fn area_resize_handles_fractional_ratios_and_colour() {
        // 3x1 RGB: red, green, blue -> 2x1. Left covers 1.5 px (red + half green).
        let src = [255, 0, 0, 0, 255, 0, 0, 0, 255];
        let out = area_resize(&src, 3, 1, 3, 2, 1);
        assert_eq!(&out[0..4], [170, 85, 0, 255]);
        assert_eq!(&out[4..8], [0, 85, 170, 255]);
    }

    #[test]
    fn area_resize_keeps_a_flat_colour_flat() {
        let src = vec![77u8; 10 * 7 * 3];
        let out = area_resize(&src, 10, 7, 3, 3, 2);
        assert!(out.chunks(4).all(|p| p == [77, 77, 77, 255]));
    }

    /// A minimal Exif APP1 whose IFD1 points at `thumb`.
    fn jpeg_with_exif(thumb: &[u8], big_endian: bool) -> Vec<u8> {
        let w16 = |v: u16| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let w32 = |v: u32| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut t = Vec::new();
        t.extend_from_slice(if big_endian { b"MM" } else { b"II" });
        t.extend_from_slice(&w16(42));
        t.extend_from_slice(&w32(8)); // IFD0 at 8
        t.extend_from_slice(&w16(0)); // IFD0: no entries
        t.extend_from_slice(&w32(14)); // next IFD (IFD1) at 14
        t.extend_from_slice(&w16(2)); // IFD1: two entries
        let thumb_at = 14 + 2 + 2 * 12 + 4;
        for (tag, ty, val) in [
            (0x0201u16, 4u16, thumb_at as u32),
            (0x0202, 4, thumb.len() as u32),
        ] {
            t.extend_from_slice(&w16(tag));
            t.extend_from_slice(&w16(ty));
            t.extend_from_slice(&w32(1));
            t.extend_from_slice(&w32(val));
        }
        t.extend_from_slice(&w32(0)); // no IFD2
        t.extend_from_slice(thumb);
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend_from_slice(&t);
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46]; // SOI + a tiny APP0
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&seg);
        out.extend_from_slice(&[0xFF, 0xDA, 0, 2]); // start of scan
        out
    }

    #[test]
    fn finds_the_embedded_thumbnail_in_both_byte_orders() {
        let thumb = [0xFF, 0xD8, 1, 2, 3, 4, 0xFF, 0xD9];
        for be in [false, true] {
            let jpg = jpeg_with_exif(&thumb, be);
            assert_eq!(exif_thumbnail(&jpg), Some(&thumb[..]), "big_endian={be}");
        }
    }

    #[test]
    fn exif_parser_rejects_garbage_without_panicking() {
        assert_eq!(exif_thumbnail(b""), None);
        assert_eq!(exif_thumbnail(b"not a jpeg"), None);
        assert_eq!(exif_thumbnail(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), None);
        let thumb = [0xFF, 0xD8, 9, 9, 0xFF, 0xD9];
        let jpg = jpeg_with_exif(&thumb, false);
        // Every truncation must be handled (None or a valid slice), never a panic.
        for n in 0..jpg.len() {
            let _ = exif_thumbnail(&jpg[..n]);
        }
        // A thumbnail that does not start with SOI is refused.
        let bad = jpeg_with_exif(&[1, 2, 3, 4], false);
        assert_eq!(exif_thumbnail(&bad), None);
    }
}
