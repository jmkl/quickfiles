//! Splits the window into the regions of the layout. Everything is derived from the window
//! rect and the (animated) side-panel widths, so the layout follows resizes and slides.

use eframe::egui::{Rect, pos2, vec2};

use crate::theme::Metrics;

/// The file list never gets narrower than this; the preview panel gives way first.
pub const MIN_LIST_W: f32 = 340.0;

/// A side panel's configured width and how far it is slid in (0 = hidden, 1 = fully shown).
#[derive(Clone, Copy)]
pub struct Slide {
    pub width: f32,
    pub t: f32,
}

/// A side panel while it slides: `visible` is the part on screen (use it to clip), `layout` is
/// the full-width rect the panel draws itself into, shifted so it slides rather than squashes.
#[derive(Clone, Copy)]
pub struct Panel {
    pub visible: Rect,
    pub layout: Rect,
}

pub struct Regions {
    pub title: Rect,
    /// Zero-height unless there are several tabs.
    pub tabs: Rect,
    pub sidebar: Option<Panel>,
    pub toolbar: Rect,
    pub header: Rect,
    pub list: Rect,
    pub preview: Option<Panel>,
}

impl Regions {
    pub fn compute(full: Rect, m: &Metrics, left: Slide, right: Slide, tabs_h: f32) -> Self {
        let title = Rect::from_min_size(full.min, vec2(full.width(), m.title_h));
        let body_top = title.bottom();
        let body_h = full.bottom() - body_top;

        let left_vis = (left.width * left.t).round();
        let right_vis = (right.width * right.t)
            .min((full.width() - left_vis - MIN_LIST_W).max(0.0))
            .round();
        let main_left = full.left() + left_vis;
        let main_right = full.right() - right_vis;

        let sidebar = (left_vis >= 1.0).then(|| Panel {
            visible: Rect::from_min_max(
                pos2(full.left(), body_top),
                pos2(main_left, full.bottom()),
            ),
            layout: Rect::from_min_size(
                pos2(main_left - left.width, body_top),
                vec2(left.width, body_h),
            ),
        });
        let preview = (right_vis >= 1.0).then(|| Panel {
            visible: Rect::from_min_max(pos2(main_right, body_top), full.right_bottom()),
            layout: Rect::from_min_size(pos2(main_right, body_top), vec2(right.width, body_h)),
        });

        let tabs = Rect::from_min_max(
            pos2(main_left, body_top),
            pos2(main_right, body_top + tabs_h),
        );
        let toolbar = Rect::from_min_max(
            pos2(main_left, tabs.bottom()),
            pos2(main_right, tabs.bottom() + m.toolbar_h),
        );
        let header = Rect::from_min_max(
            pos2(main_left, toolbar.bottom()),
            pos2(main_right, toolbar.bottom() + m.header_h),
        );
        let list = Rect::from_min_max(
            pos2(main_left, header.bottom()),
            pos2(main_right, full.bottom()),
        );

        Self {
            title,
            tabs,
            sidebar,
            toolbar,
            header,
            list,
            preview,
        }
    }
}
