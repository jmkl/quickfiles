//! GPU texture cache for the pixels `quickfiles-core` decodes.
//!
//! Lookups use [`QuickKey`] (three integers): a thumbnail that already has a texture costs one
//! hash lookup per frame, with no allocation and no lock. Uploads are limited per frame, so a
//! burst of finished thumbnails is spread over several frames instead of stalling one. The
//! pixels arrive premultiplied (converted on a worker), so an upload is a plain copy.
//!
//! While the list is being scrolled fast ([`Tex::set_gate`]) nothing new is requested: icons
//! only, and the thumbnails load as soon as the scroll slows down.

use std::{collections::HashMap, ops::Range, sync::Arc};

use eframe::egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2, vec2};
use quickfiles_core::{Entry, ImageKey, Images, Kind, Pixels, QuickKey, View};

const MAX_TEXTURES: usize = 300;
/// GPU memory held by textures. A 2048 px preview is 16 MB, a thumbnail 37 KB.
const MAX_TEXTURE_BYTES: usize = 192 * 1024 * 1024;
/// Texture uploads per frame. The rest wait for the next frame (a repaint is requested).
/// The large preview is exempt: it is one image and the user is waiting for it.
const MAX_UPLOADS_PER_FRAME: u32 = 12;

struct Slot {
    handle: TextureHandle,
    size: Vec2,
    thumb: bool,
    used: u64,
    bytes: usize,
}

/// What to show for an entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Want {
    Thumb,
    /// A picture of at most this many pixels on the longest side (`quickfiles_core::preview_px`).
    Preview(u32),
    /// Shell icon of an executable, shortcut or drive.
    PathIcon,
}

pub struct Tex {
    map: HashMap<QuickKey, Slot>,
    frame: u64,
    uploads_frame: u32,
    upload_us_frame: u64,
    uploads_total: u64,
    deferred_total: u64,
    bytes: usize,
    gated: bool,
}

/// Texture counters for the F12 overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct TexStats {
    pub textures: usize,
    pub uploads_frame: u32,
    /// Time spent converting and uploading this frame (on the UI thread).
    pub upload_ms_frame: f32,
    pub uploads_total: u64,
    /// Uploads postponed to a later frame because of the per-frame limit.
    pub deferred_total: u64,
    /// Texture memory in MB.
    pub texture_mb: f32,
    /// Image requests are paused because the list is scrolling fast.
    pub gated: bool,
}

pub struct Loaded {
    pub id: TextureId,
    pub size: Vec2,
    /// A real thumbnail / preview (as opposed to a shell icon).
    pub thumb: bool,
}

impl Tex {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            frame: 0,
            uploads_frame: 0,
            upload_us_frame: 0,
            uploads_total: 0,
            deferred_total: 0,
            bytes: 0,
            gated: false,
        }
    }

    pub fn stats(&self) -> TexStats {
        TexStats {
            textures: self.map.len(),
            uploads_frame: self.uploads_frame,
            upload_ms_frame: self.upload_us_frame as f32 / 1000.0,
            uploads_total: self.uploads_total,
            deferred_total: self.deferred_total,
            texture_mb: self.bytes as f32 / (1024.0 * 1024.0),
            gated: self.gated,
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.uploads_frame = 0;
        self.upload_us_frame = 0;
        if self.map.len() > MAX_TEXTURES || self.bytes > MAX_TEXTURE_BYTES {
            let cutoff = self.frame.saturating_sub(2);
            self.map.retain(|_, s| s.used >= cutoff);
            self.bytes = self.map.values().map(|s| s.bytes).sum();
        }
    }

    /// Pause (or resume) requesting new images. Textures that already exist keep drawing.
    pub fn set_gate(&mut self, gated: bool) {
        self.gated = gated;
    }

    /// Low-priority interest in the images of rows just outside the viewport.
    pub fn prefetch_rows(&mut self, images: &Images, view: &View, rows: Range<usize>) {
        if self.gated {
            return;
        }
        for i in rows {
            let Some(e) = view.get(i) else { continue };
            if let Some(quick) = ImageKey::quick_thumb(&e) {
                if !self.map.contains_key(&quick) {
                    if let Some(key) = ImageKey::thumb(&e) {
                        let _ = images.prefetch(&key);
                    }
                }
            }
        }
    }

    /// Low-priority interest in one image (the neighbours of the previewed file).
    pub fn prefetch(&mut self, images: &Images, e: &Entry, want: Want) {
        let (quick, key) = match want {
            Want::Thumb => (ImageKey::quick_thumb(e), ImageKey::thumb(e)),
            Want::Preview(px) => (ImageKey::quick_preview(e, px), ImageKey::preview(e, px)),
            Want::PathIcon => return,
        };
        if let (Some(q), Some(k)) = (quick, key) {
            if !self.map.contains_key(&q) {
                let _ = images.prefetch(&k);
            }
        }
    }

    /// Thumbnail when the type has one, otherwise the shell icon of executables and shortcuts.
    pub fn thumb_or_icon(&mut self, ctx: &Context, images: &Images, e: &Entry) -> Option<Loaded> {
        if e.kind != Kind::File {
            return None;
        }
        if ImageKey::quick_thumb(e).is_some() {
            self.entry(ctx, images, e, Want::Thumb)
        } else if ImageKey::quick_path_icon(e).is_some() {
            self.entry(ctx, images, e, Want::PathIcon)
        } else {
            None
        }
    }

    /// The texture for `e`, uploading it on first use. Records interest with the image service
    /// while it is not ready, so asking every frame is how images get loaded.
    pub fn entry(
        &mut self,
        ctx: &Context,
        images: &Images,
        e: &Entry,
        want: Want,
    ) -> Option<Loaded> {
        let quick = match want {
            Want::Thumb => ImageKey::quick_thumb(e),
            Want::Preview(px) => ImageKey::quick_preview(e, px),
            Want::PathIcon => ImageKey::quick_path_icon(e),
        }?;
        if let Some(slot) = self.map.get_mut(&quick) {
            slot.used = self.frame;
            return Some(Loaded {
                id: slot.handle.id(),
                size: slot.size,
                thumb: slot.thumb,
            });
        }

        // Fast scrolling: draw icons only, ask for nothing. The large preview is exempt.
        if self.gated && !matches!(want, Want::Preview(_)) {
            return None;
        }
        // Miss: build the full key (allocates) and ask the image service.
        let full = match want {
            Want::Thumb => ImageKey::thumb(e)?,
            Want::Preview(px) => ImageKey::preview(e, px)?,
            Want::PathIcon => ImageKey::icon(e),
        };
        let px: Arc<Pixels> = images.want(&full)?;
        if !matches!(want, Want::Preview(_)) && self.uploads_frame >= MAX_UPLOADS_PER_FRAME {
            self.deferred_total += 1;
            ctx.request_repaint();
            return None;
        }

        // The pixels are already premultiplied (done on a worker): this is a copy and an upload.
        let started = std::time::Instant::now();
        let size_px = [px.w as usize, px.h as usize];
        let image = if px.premultiplied {
            ColorImage::from_rgba_premultiplied(size_px, &px.rgba)
        } else {
            ColorImage::from_rgba_unmultiplied(size_px, &px.rgba)
        };
        let handle = ctx.load_texture(format!("{full:?}"), image, TextureOptions::LINEAR);
        self.uploads_frame += 1;
        self.uploads_total += 1;
        self.upload_us_frame += started.elapsed().as_micros() as u64;

        let size = vec2(px.w as f32, px.h as f32);
        let thumb = !matches!(quick, QuickKey::PathIcon(_));
        let loaded = Loaded {
            id: handle.id(),
            size,
            thumb,
        };
        let bytes = px.rgba.len();
        self.bytes += bytes;
        if let Some(old) = self.map.insert(
            quick,
            Slot {
                handle,
                size,
                thumb,
                used: self.frame,
                bytes,
            },
        ) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        Some(loaded)
    }
}

/// Feeds the list's scroll offset (once per frame, after drawing) to the scroll gate and applies
/// the result to `cx.tex` for the next frame. While the scrollbar is dragged through thousands
/// of files only icons are drawn; images load right after the scroll slows down.
pub fn track_scroll(cx: &mut super::Cx<'_>, ctx: &Context, view_id: u64, offset: f32) {
    if cx.state.scroll_gate_view != view_id {
        cx.state.scroll_gate.reset();
        cx.state.scroll_gate_view = view_id;
    }
    cx.state
        .scroll_gate
        .update(offset, std::time::Instant::now());
    let fast = cx.state.scroll_gate.fast();
    cx.tex.set_gate(fast);
    if fast {
        // Frames must keep coming so the gate can notice the scroll has stopped.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// The rows just past the visible ones, in the direction the user is scrolling (`dir` > 0 = down).
pub fn rows_ahead(visible: &Range<usize>, dir: i8, count: usize, total: usize) -> Range<usize> {
    if dir >= 0 {
        visible.end.min(total)..(visible.end + count).min(total)
    } else {
        visible.start.saturating_sub(count)..visible.start.min(total)
    }
}
