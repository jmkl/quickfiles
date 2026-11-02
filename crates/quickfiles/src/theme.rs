//! Single source of truth for the look of the app.
//!
//! UI modules never hard-code a colour or a pixel size: they read them from [`Theme`].
//! To re-skin the app (light theme, different density) only this file changes.
//! All values were measured from the reference screenshot (1152x760, 1.0 scale).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Every colour used by the UI.
#[derive(Clone, Copy)]
pub struct Palette {
    // Surfaces
    pub chrome: Color32,  // title bar
    pub main: Color32,    // toolbar / list background, search box
    pub sidebar: Color32, // sidebar and preview panel
    pub stripe: Color32,  // zebra rows
    pub hover: Color32,
    pub border: Color32,
    // Accents
    pub accent: Color32, // focus ring, healthy drive bar
    pub danger: Color32, // nearly-full drive bar
    pub track: Color32,  // drive bar track, "0%" box
    pub track_border: Color32,
    // Text
    pub text: Color32,
    pub text_dim: Color32,
    pub text_section: Color32,
    // Icons
    pub icon: Color32,
    pub icon_strong: Color32,
    pub icon_nav: Color32,
    pub icon_disabled: Color32,
    pub close_hover: Color32,
    // Coloured glyphs
    pub folder: Color32,
    pub drive: Color32,
    pub drive_win: Color32,
    pub pc: Color32,
    pub file_html: Color32,
    pub file_js: Color32,
    pub file_json: Color32,
}

impl Palette {
    pub const DARK: Self = Self {
        chrome: rgb(0x27, 0x2a, 0x2b),
        main: rgb(0x19, 0x1b, 0x1c),
        sidebar: rgb(0x1f, 0x22, 0x23),
        stripe: rgb(0x1f, 0x22, 0x24),
        hover: rgb(0x2a, 0x2d, 0x2f),
        border: rgb(0x34, 0x3a, 0x3b),
        accent: rgb(0x00, 0x79, 0xa6),
        danger: rgb(0xff, 0x66, 0x66),
        track: rgb(0x38, 0x3c, 0x3d),
        track_border: rgb(0x46, 0x4a, 0x4d),
        text: rgb(0xdd, 0xde, 0xe0),
        text_dim: rgb(0x6b, 0x70, 0x78),
        text_section: rgb(0xa3, 0xa5, 0xa6),
        icon: rgb(0xaf, 0xb1, 0xb3),
        icon_strong: rgb(0xdd, 0xde, 0xe0),
        icon_nav: rgb(0x85, 0x86, 0x88),
        icon_disabled: rgb(0x46, 0x48, 0x49),
        close_hover: rgb(0xc4, 0x2b, 0x1c),
        folder: rgb(0xff, 0xdc, 0x79),
        drive: rgb(0xe0, 0xe2, 0xe5),
        drive_win: rgb(0x12, 0x90, 0xdd),
        pc: rgb(0x22, 0xa4, 0xd1),
        file_html: rgb(0xec, 0xef, 0xf1),
        file_js: rgb(0x1b, 0x1c, 0x1e),
        file_json: rgb(0xe8, 0x8a, 0x2e),
    };
}

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

    /// Push the theme into egui (visuals, spacing, text styles, fonts). Call once at start-up.
    pub fn apply(&self, ctx: &egui::Context) {
        install_fonts(ctx);
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
        v.panel_fill = p.main;
        v.window_fill = p.main;
        v.extreme_bg_color = p.sidebar;
        v.faint_bg_color = p.stripe;
        v.override_text_color = Some(p.text);
        v.selection.bg_fill = p.accent;
        v.selection.stroke = Stroke::new(1.0, p.text);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        v
    }
}

/// Use Segoe UI when available (Windows) so metrics match the reference; otherwise egui's default.
fn install_fonts(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") else {
        return;
    };
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("segoe-ui".to_owned(), Arc::new(FontData::from_owned(bytes)));
    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, "segoe-ui".to_owned());
    }
    ctx.set_fonts(fonts);
}
