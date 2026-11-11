//! Vector icons drawn with the egui painter, so the app needs no image assets or icon font.
//! Every icon is authored on a 16x16 grid centred on the given point.
//! Coloured icons (folder, drives, file badges) take their colours from the palette and
//! ignore `tint`.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};

use quickfiles_core::Category;

use crate::theme::Palette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    // Chrome
    Menu,
    PanelRight,
    Search,
    Close,
    Minimize,
    Maximize,
    Kebab,
    // Arrows
    ChevronDown,
    ChevronRight,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    // Toolbar / status
    Lock,
    Bookmark,
    Clock,
    Filter,
    Storage,
    Pin,
    // Coloured glyphs
    Folder,
    FileHtml,
    FileJs,
    FileJson,
    FileGeneric,
    Drive,
    DriveWindows,
    ThisPc,
}

impl Icon {
    pub fn for_category(category: Category) -> Self {
        match category {
            Category::Folder => Self::Folder,
            Category::Drive => Self::Drive,
            Category::Html => Self::FileHtml,
            Category::Script | Category::Code => Self::FileJs,
            Category::Json => Self::FileJson,
            _ => Self::FileGeneric,
        }
    }
}

pub fn paint(p: &Painter, pal: &Palette, icon: Icon, c: Pos2, tint: Color32) {
    paint_scaled(p, pal, icon, c, tint, 1.0);
}

pub fn paint_scaled(p: &Painter, pal: &Palette, icon: Icon, c: Pos2, tint: Color32, k: f32) {
    let pt = |x: f32, y: f32| c + vec2(x, y) * k;
    let r4 = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pt(x0, y0), pt(x1, y1));
    let line = |pts: &[(f32, f32)], st: Stroke| {
        p.add(Shape::line(
            pts.iter().map(|&(x, y)| pt(x, y)).collect(),
            st,
        ));
    };
    let poly = |pts: &[(f32, f32)], st: Stroke| {
        p.add(Shape::closed_line(
            pts.iter().map(|&(x, y)| pt(x, y)).collect(),
            st,
        ));
    };
    let fill = |r: Rect, radius: f32, col: Color32| {
        p.rect_filled(r, radius, col);
    };
    let outline = |r: Rect, radius: f32, st: Stroke| {
        p.rect_stroke(r, radius, st, StrokeKind::Inside);
    };

    let s = Stroke::new(1.3, tint);
    let thin = Stroke::new(1.0, tint);

    match icon {
        Icon::Menu => {
            for y in [-5.0, 0.0, 5.0] {
                line(&[(-6.0, y), (6.0, y)], s);
            }
        }
        Icon::PanelRight => {
            outline(r4(-7.5, -6.5, 7.5, 6.5), 2.0, Stroke::new(1.2, tint));
            line(&[(2.5, -6.0), (2.5, 6.0)], Stroke::new(1.2, tint));
        }
        Icon::Search => {
            p.circle_stroke(pt(-1.5, -1.5), 4.5 * k, s);
            line(&[(1.8, 1.8), (6.0, 6.0)], s);
        }
        Icon::Close => {
            line(&[(-5.0, -5.0), (5.0, 5.0)], thin);
            line(&[(-5.0, 5.0), (5.0, -5.0)], thin);
        }
        Icon::Minimize => line(&[(-5.0, 0.0), (5.0, 0.0)], thin),
        Icon::Maximize => outline(r4(-5.0, -5.0, 5.0, 5.0), 0.0, thin),
        Icon::Kebab => {
            for y in [-5.0, 0.0, 5.0] {
                p.circle_filled(pt(0.0, y), 1.4 * k, tint);
            }
        }

        Icon::ChevronDown => line(&[(-4.0, -2.0), (0.0, 2.0), (4.0, -2.0)], s),
        Icon::ChevronRight => line(&[(-2.0, -4.0), (2.0, 0.0), (-2.0, 4.0)], s),
        Icon::ArrowLeft => {
            line(&[(-6.0, 0.0), (6.0, 0.0)], s);
            line(&[(-1.0, -5.0), (-6.0, 0.0), (-1.0, 5.0)], s);
        }
        Icon::ArrowRight => {
            line(&[(-6.0, 0.0), (6.0, 0.0)], s);
            line(&[(1.0, -5.0), (6.0, 0.0), (1.0, 5.0)], s);
        }
        Icon::ArrowUp => {
            line(&[(0.0, -6.0), (0.0, 6.0)], s);
            line(&[(-5.0, -1.0), (0.0, -6.0), (5.0, -1.0)], s);
        }
        Icon::ArrowDown => {
            line(&[(0.0, -6.0), (0.0, 6.0)], s);
            line(&[(-5.0, 1.0), (0.0, 6.0), (5.0, 1.0)], s);
        }

        Icon::Lock => {
            outline(r4(-5.5, -1.0, 5.5, 7.0), 1.5, s);
            line(
                &[
                    (-3.0, -1.0),
                    (-3.0, -4.0),
                    (-1.5, -6.0),
                    (1.5, -6.0),
                    (3.0, -4.0),
                    (3.0, -1.0),
                ],
                s,
            );
        }
        Icon::Bookmark => poly(
            &[
                (-4.5, -6.0),
                (4.5, -6.0),
                (4.5, 6.0),
                (0.0, 2.5),
                (-4.5, 6.0),
            ],
            s,
        ),
        Icon::Clock => {
            p.circle_stroke(c, 6.0 * k, s);
            line(&[(0.0, 0.0), (0.0, -3.5)], s);
            line(&[(0.0, 0.0), (2.5, 1.5)], s);
        }
        Icon::Filter => poly(
            &[
                (-6.0, -5.0),
                (6.0, -5.0),
                (1.5, 0.0),
                (1.5, 5.5),
                (-1.5, 4.0),
                (-1.5, 0.0),
            ],
            s,
        ),
        Icon::Storage => {
            outline(r4(-7.0, -4.0, 7.0, 5.0), 2.0, s);
            line(&[(-7.0, 0.5), (7.0, 0.5)], s);
            p.circle_filled(pt(4.0, 3.0), 0.9 * k, tint);
        }
        Icon::Pin => {
            p.circle_stroke(pt(0.0, -2.0), 4.0 * k, s);
            line(&[(-3.0, 1.0), (0.0, 7.0), (3.0, 1.0)], s);
        }

        Icon::Folder => {
            fill(
                r4(-8.0, -6.0, -1.0, -3.0),
                1.5,
                pal.folder.gamma_multiply(0.85),
            );
            fill(r4(-8.0, -4.5, 8.0, 6.0), 2.0, pal.folder);
        }
        Icon::FileHtml => {
            fill(r4(-6.0, -8.0, 6.0, 8.0), 1.5, pal.file_html);
            p.circle_filled(pt(0.0, 1.0), 3.0 * k, pal.accent);
        }
        Icon::FileJs => {
            fill(r4(-8.0, -8.0, 8.0, 8.0), 2.0, pal.file_js);
            line(
                &[(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)],
                Stroke::new(1.3, Color32::WHITE),
            );
        }
        Icon::FileJson => {
            fill(r4(-8.0, -8.0, 8.0, 8.0), 2.0, pal.file_json);
            line(
                &[
                    (3.0, -3.5),
                    (-2.0, -3.5),
                    (-2.0, 0.0),
                    (2.0, 0.0),
                    (2.0, 3.5),
                    (-3.0, 3.5),
                ],
                Stroke::new(1.3, pal.file_js),
            );
        }
        Icon::FileGeneric => {
            fill(r4(-6.0, -8.0, 6.0, 8.0), 1.5, pal.icon_nav);
            line(&[(-3.0, -2.0), (3.0, -2.0)], Stroke::new(1.0, pal.main));
            line(&[(-3.0, 2.0), (3.0, 2.0)], Stroke::new(1.0, pal.main));
        }
        Icon::Drive => {
            fill(
                r4(-5.0, -6.0, 5.0, -2.0),
                1.0,
                pal.drive.gamma_multiply(0.6),
            );
            fill(r4(-7.0, -2.0, 7.0, 6.0), 2.0, pal.drive);
            p.circle_filled(pt(4.0, 3.0), 0.9 * k, pal.sidebar);
        }
        Icon::DriveWindows => {
            for (x, y) in [(-7.0, -7.0), (0.5, -7.0), (-7.0, 0.5), (0.5, 0.5)] {
                fill(r4(x, y, x + 6.5, y + 6.5), 0.5, pal.drive_win);
            }
        }
        Icon::ThisPc => {
            fill(r4(-8.0, -6.0, 8.0, 4.0), 1.5, pal.pc);
            fill(r4(-1.5, 4.0, 1.5, 6.5), 0.0, pal.pc);
            line(&[(-4.0, 7.0), (4.0, 7.0)], Stroke::new(1.5, pal.pc));
        }
    }
}
