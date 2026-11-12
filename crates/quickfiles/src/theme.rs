//! Single source of truth for the look of the app.
//!
//! UI modules never hard-code a colour or a pixel size: they read them from [`Theme`].
//! To re-skin the app (light theme, different density) only this file changes.
//! All values were measured from the reference screenshot (1152x760, 1.0 scale).

use std::sync::Arc;

use crate::config::{self, Entry, NamedTheme};

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Every colour used by the UI.
///
/// Names say what a colour is *for*. Colours used all over the UI have generic names (`surface`,
/// `border`, `text_muted`, `icon`); a colour used by one widget is named after it
/// (`scrollbar_thumb`, `window_close`). The same names are the keys of `[theme]` in `config.conf`.
#[derive(Clone, Copy)]
pub struct Palette {
    // Surfaces (what things are drawn on)
    /// Title bar; also menus, tooltips and the hotkey panel.
    pub surface: Color32,
    /// Main area: toolbar, file list, search box.
    pub background: Color32,
    /// Side panels: sidebar and preview.
    pub panel: Color32,
    /// Every other row of the file list.
    pub row_alt: Color32,
    /// Row / button under the pointer; key chips.
    pub hover: Color32,
    /// Dividers and outlines.
    pub border: Color32,
    // Accents
    /// Focus ring, selection, healthy drive bar, drop highlight.
    pub accent: Color32,
    /// Warnings: nearly-full drive bar, errors.
    pub danger: Color32,
    // Controls (bars and boxes)
    /// Empty part of a progress / drive bar, small boxes.
    pub control: Color32,
    pub control_border: Color32,
    // Text
    pub text: Color32,
    /// Secondary text: sizes, dates, hints.
    pub text_muted: Color32,
    /// Section headings in the sidebar.
    pub text_secondary: Color32,
    // Icons (toolbar, title bar, row glyphs)
    pub icon: Color32,
    /// Icon being pressed or switched on.
    pub icon_active: Color32,
    /// Quiet icons such as the navigation arrows.
    pub icon_muted: Color32,
    pub icon_disabled: Color32,
    // Scrollbars
    /// The channel the scrollbar runs in.
    pub scrollbar_track: Color32,
    pub scrollbar_thumb: Color32,
    pub scrollbar_thumb_hover: Color32,
    pub scrollbar_thumb_active: Color32,
    // Window buttons
    /// Close button under the pointer.
    pub window_close: Color32,
    // File type glyphs
    pub icon_folder: Color32,
    pub icon_drive: Color32,
    pub icon_drive_system: Color32,
    pub icon_pc: Color32,
    pub icon_file_html: Color32,
    pub icon_file_js: Color32,
    pub icon_file_json: Color32,
    // Coloured file icons (Nerd Font glyphs)
    pub icon_file_code: Color32,
    pub icon_file_script: Color32,
    pub icon_file_image: Color32,
    pub icon_file_media: Color32,
    pub icon_file_archive: Color32,
    pub icon_file_document: Color32,
    pub icon_file_pdf: Color32,
}

impl Palette {
    pub const DARK: Self = Self {
        surface: rgb(0x27, 0x2a, 0x2b),
        background: rgb(0x19, 0x1b, 0x1c),
        panel: rgb(0x1f, 0x22, 0x23),
        row_alt: rgb(0x1f, 0x22, 0x24),
        hover: rgb(0x2a, 0x2d, 0x2f),
        border: rgb(0x34, 0x3a, 0x3b),
        accent: rgb(0x00, 0x79, 0xa6),
        danger: rgb(0xff, 0x66, 0x66),
        control: rgb(0x38, 0x3c, 0x3d),
        control_border: rgb(0x46, 0x4a, 0x4d),
        text: rgb(0xdd, 0xde, 0xe0),
        text_muted: rgb(0x6b, 0x70, 0x78),
        text_secondary: rgb(0xa3, 0xa5, 0xa6),
        icon: rgb(0xaf, 0xb1, 0xb3),
        icon_active: rgb(0xdd, 0xde, 0xe0),
        icon_muted: rgb(0x85, 0x86, 0x88),
        icon_disabled: rgb(0x46, 0x48, 0x49),
        // The track was the panel colour and the thumb egui's default greys before these
        // became settings.
        scrollbar_track: rgb(0x1f, 0x22, 0x23),
        scrollbar_thumb: rgb(0x96, 0x96, 0x96),
        scrollbar_thumb_hover: rgb(0xf0, 0xf0, 0xf0),
        scrollbar_thumb_active: rgb(0xff, 0xff, 0xff),
        window_close: rgb(0xc4, 0x2b, 0x1c),
        icon_folder: rgb(0xff, 0xdc, 0x79),
        icon_drive: rgb(0xe0, 0xe2, 0xe5),
        icon_drive_system: rgb(0x12, 0x90, 0xdd),
        icon_pc: rgb(0x22, 0xa4, 0xd1),
        icon_file_html: rgb(0xec, 0xef, 0xf1),
        icon_file_js: rgb(0x1b, 0x1c, 0x1e),
        icon_file_json: rgb(0xe8, 0x8a, 0x2e),
        icon_file_code: rgb(0x6c, 0xb6, 0xff),
        icon_file_script: rgb(0xf0, 0xd0, 0x60),
        icon_file_image: rgb(0xc7, 0x92, 0xea),
        icon_file_media: rgb(0xf7, 0x8c, 0x6c),
        icon_file_archive: rgb(0xd1, 0x9a, 0x66),
        icon_file_document: rgb(0x5b, 0x9b, 0xd5),
        icon_file_pdf: rgb(0xe0, 0x5d, 0x5d),
    };
}

/// Names `config.conf` used before the colours were renamed; still accepted.
const OLD_NAMES: &[(&str, &str)] = &[
    ("chrome", "surface"),
    ("main", "background"),
    ("sidebar", "panel"),
    ("stripe", "row_alt"),
    ("track", "control"),
    ("track_border", "control_border"),
    ("text_dim", "text_muted"),
    ("text_section", "text_secondary"),
    ("icon_strong", "icon_active"),
    ("icon_nav", "icon_muted"),
    ("close_hover", "window_close"),
    ("folder", "icon_folder"),
    ("drive", "icon_drive"),
    ("drive_win", "icon_drive_system"),
    ("pc", "icon_pc"),
    ("file_html", "icon_file_html"),
    ("file_js", "icon_file_js"),
    ("file_json", "icon_file_json"),
];

/// Name-based access to the palette, so `config.conf` can override any colour.
macro_rules! palette_fields {
    ($($f:ident),* $(,)?) => {
        impl Palette {
            /// (name, colour) for every field, in declaration order.
            pub fn entries(&self) -> Vec<(&'static str, Color32)> {
                vec![$((stringify!($f), self.$f)),*]
            }

            /// Sets the colour called `name` (or its pre-rename name); false when there is none.
            pub fn set(&mut self, name: &str, c: Color32) -> bool {
                let name = OLD_NAMES
                    .iter()
                    .find(|(old, _)| *old == name)
                    .map_or(name, |(_, new)| *new);
                match name {
                    $(stringify!($f) => self.$f = c,)*
                    _ => return false,
                }
                true
            }
        }
    };
}

palette_fields!(
    surface,
    background,
    panel,
    row_alt,
    hover,
    border,
    accent,
    danger,
    control,
    control_border,
    text,
    text_muted,
    text_secondary,
    icon,
    icon_active,
    icon_muted,
    icon_disabled,
    scrollbar_track,
    scrollbar_thumb,
    scrollbar_thumb_hover,
    scrollbar_thumb_active,
    window_close,
    icon_folder,
    icon_drive,
    icon_drive_system,
    icon_pc,
    icon_file_html,
    icon_file_js,
    icon_file_json,
    icon_file_code,
    icon_file_script,
    icon_file_image,
    icon_file_media,
    icon_file_archive,
    icon_file_document,
    icon_file_pdf,
);

/// Every size / offset used by the UI (logical pixels).
#[derive(Clone, Copy)]
pub struct Metrics {
    // Window regions
    pub title_h: f32,
    pub sidebar_w: f32, // includes the 1px divider
    pub toolbar_h: f32,
    pub header_h: f32,
    pub preview_w: f32, // includes the 1px divider
    pub search_h: f32,
    // Title bar
    pub win_btn_w: f32,
    // Lists
    pub row_pitch: f32,
    pub row_h: f32,
    pub list_pad: f32,
    pub radius: f32,
    // Sidebar
    pub side_row_h: f32,
    pub side_drive_h: f32,
    pub side_section_gap: f32,
    pub side_text_x: f32,
    pub side_icon_x: f32,
    // File list columns: distance of each column's left edge from the list's right edge.
    pub col_type: f32,
    pub col_date: f32,
    pub col_folders: f32,
    pub col_files: f32,
    pub col_size: f32,
    pub col_text_pad: f32,
}

impl Metrics {
    pub const DEFAULT: Self = Self {
        title_h: 33.0,
        sidebar_w: 249.0,
        toolbar_h: 32.0,
        header_h: 32.0,
        preview_w: 300.0,
        search_h: 24.0,
        win_btn_w: 41.0,
        row_pitch: 27.0,
        row_h: 26.0,
        list_pad: 14.0,
        radius: 4.0,
        side_row_h: 27.0,
        side_drive_h: 29.0,
        side_section_gap: 12.5,
        side_text_x: 59.0,
        side_icon_x: 43.5,
        col_type: 481.0,
        col_date: 350.0,
        col_folders: 219.0,
        col_files: 160.0,
        col_size: 87.0,
        col_text_pad: 18.0,
    };
}

#[derive(Clone)]
pub struct Theme {
    pub palette: Palette,
    pub metrics: Metrics,
    pub font: FontId,
    pub mono: FontId,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            palette: Palette::DARK,
            metrics: Metrics::DEFAULT,
            font: FontId::proportional(13.0),
            mono: FontId::monospace(12.0),
        }
    }

    /// The dark theme, then the `[[themes]]` block named by `active_theme`, then the `[theme]`
    /// entries as personal tweaks. Bad entries are skipped and reported, one message each.
    pub fn configured(entries: &[Entry], themes: &[NamedTheme]) -> (Self, Vec<String>) {
        let mut theme = Self::dark();
        let mut problems = Vec::new();

        let wanted = entries.iter().rev().find(|e| e.key == "active_theme");
        if let Some(w) = wanted.filter(|w| !w.value.is_empty()) {
            match themes
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&w.value))
            {
                Some(t) => theme.apply_entries(&t.entries, &mut problems),
                None => {
                    let known: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
                    problems.push(format!(
                        "config line {}: no theme named \"{}\" (have: {})",
                        w.line,
                        w.value,
                        if known.is_empty() {
                            "none".to_owned()
                        } else {
                            known.join(", ")
                        }
                    ));
                }
            }
        }
        for t in themes.iter().filter(|t| t.name.is_empty()) {
            problems.push(format!(
                "config: a [[themes]] block has no name{}",
                t.entries
                    .first()
                    .map_or(String::new(), |e| format!(" (near line {})", e.line))
            ));
        }

        let own: Vec<Entry> = entries
            .iter()
            .filter(|e| e.key != "active_theme")
            .cloned()
            .collect();
        theme.apply_entries(&own, &mut problems);
        (theme, problems)
    }

    fn apply_entries(&mut self, entries: &[Entry], problems: &mut Vec<String>) {
        let theme = self;
        for e in entries {
            let at = format!("config line {}", e.line);
            match e.key.as_str() {
                "font_size" | "mono_size" => match e.value.parse::<f32>() {
                    Ok(s) if (8.0..=32.0).contains(&s) => {
                        if e.key == "font_size" {
                            theme.font = FontId::proportional(s);
                        } else {
                            theme.mono = FontId::monospace(s);
                        }
                    }
                    _ => problems.push(format!("{at}: {} must be a number from 8 to 32", e.key)),
                },
                name => match config::parse_color(&e.value) {
                    Some([r, g, b, a]) => {
                        let c = Color32::from_rgba_unmultiplied(r, g, b, a);
                        if !theme.palette.set(name, c) {
                            problems.push(format!("{at}: unknown theme key \"{name}\""));
                        }
                    }
                    None => {
                        problems.push(format!("{at}: \"{}\" is not a colour (#rrggbb)", e.value))
                    }
                },
            }
        }
    }

    /// Push the theme into egui (visuals, spacing, text styles, fonts). Call once at start-up.
    pub fn apply(&self, ctx: &egui::Context) {
        install_fonts(ctx);
        self.restyle(ctx);
    }

    /// Everything [`apply`](Self::apply) does except loading fonts; used when the config reloads.
    pub fn restyle(&self, ctx: &egui::Context) {
        ctx.set_theme(egui::Theme::Dark);
        ctx.set_visuals(self.visuals());

        let font = self.font.clone();
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            // We lay everything out by hand; zero spacing keeps row maths exact.
            style.spacing.item_spacing = egui::Vec2::ZERO;
            style.spacing.scroll = egui::style::ScrollStyle::thin();
            style.interaction.selectable_labels = false;
            for text_style in [TextStyle::Body, TextStyle::Button, TextStyle::Small] {
                style.text_styles.insert(text_style, font.clone());
            }
        });
    }

    fn visuals(&self) -> Visuals {
        let p = &self.palette;
        let mut v = Visuals::dark();
        v.panel_fill = p.background;
        v.window_fill = p.background;
        // egui draws the scrollbar channel with `extreme_bg_color` (our text boxes have no frame).
        v.extreme_bg_color = p.scrollbar_track;
        v.faint_bg_color = p.row_alt;
        v.override_text_color = Some(p.text);
        v.selection.bg_fill = p.accent;
        v.selection.stroke = Stroke::new(1.0, p.text);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        // The scrollbar thumb takes its colour from the widget states; everything else is drawn
        // by hand, so nothing else is affected.
        for (state, c) in [
            (&mut v.widgets.inactive, p.scrollbar_thumb),
            (&mut v.widgets.hovered, p.scrollbar_thumb_hover),
            (&mut v.widgets.active, p.scrollbar_thumb_active),
        ] {
            state.bg_fill = c;
            state.fg_stroke.color = c;
        }
        v
    }
}

/// The font family of the file / folder icons (Nerd Font symbols).
pub fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

/// Fonts: the Nerd Font symbols for file icons (always), and Segoe UI for text when available
/// (Windows) so metrics match the reference; otherwise egui's default.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "nerd-symbols".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/SymbolsNerdFontMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .insert(icon_family(), vec!["nerd-symbols".to_owned()]);
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts
            .font_data
            .insert("segoe-ui".to_owned(), Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "segoe-ui".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_can_be_set_by_its_name_and_names_are_unique() {
        let mut p = Palette::DARK;
        let names: Vec<&str> = p.entries().iter().map(|(n, _)| *n).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate colour name");
        for (i, name) in names.iter().enumerate() {
            let c = Color32::from_rgb(i as u8, 7, 9);
            assert!(p.set(name, c), "{name} cannot be set");
            assert_eq!(p.entries()[i], (*name, c), "{name} changed another field");
        }
        assert!(!p.set("no_such_colour", Color32::WHITE));
    }

    #[test]
    fn old_names_still_work() {
        for (old, new) in OLD_NAMES {
            let mut p = Palette::DARK;
            let c = Color32::from_rgb(1, 2, 3);
            assert!(p.set(old, c), "{old}");
            assert_eq!(
                p.entries().iter().find(|(n, _)| n == new).unwrap().1,
                c,
                "{old} -> {new}"
            );
        }
    }

    #[test]
    fn the_scrollbar_is_themeable() {
        let mut t = Theme::dark();
        t.palette.scrollbar_thumb = Color32::from_rgb(10, 20, 30);
        t.palette.scrollbar_track = Color32::from_rgb(40, 50, 60);
        let v = t.visuals();
        assert_eq!(
            v.widgets.inactive.fg_stroke.color,
            Color32::from_rgb(10, 20, 30)
        );
        assert_eq!(v.widgets.inactive.bg_fill, Color32::from_rgb(10, 20, 30));
        assert_eq!(v.extreme_bg_color, Color32::from_rgb(40, 50, 60));
    }

    #[test]
    fn active_theme_then_personal_tweaks() {
        let e = |line, key: &str, value: &str| Entry {
            line,
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let themes = [NamedTheme {
            name: "Night".to_owned(),
            entries: vec![e(5, "accent", "#010203"), e(6, "hover", "#040506")],
        }];
        let own = [e(2, "active_theme", "night"), e(3, "hover", "#0a0b0c")];
        let (t, problems) = Theme::configured(&own, &themes);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(t.palette.accent, Color32::from_rgb(1, 2, 3));
        assert_eq!(t.palette.hover, Color32::from_rgb(10, 11, 12), "tweak wins");

        let (t, problems) = Theme::configured(&[e(2, "active_theme", "nope")], &themes);
        assert_eq!(problems.len(), 1);
        assert_eq!(t.palette.accent, Palette::DARK.accent);
    }
}
