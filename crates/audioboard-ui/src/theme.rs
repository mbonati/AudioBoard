//! Colour palette, fonts and global egui styling: AudioBoard's look. Flat
//! graphite surfaces, one accent colour per node, coloured outlines and
//! line art (no glows, no 3D).

use std::sync::Arc;
use std::sync::RwLock;

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub panel: Color32,
    pub panel_border: Color32,
    pub node_body: Color32,
    pub node_header: Color32,
    pub node_border: Color32,
    pub inset: Color32,
    pub control: Color32,
    pub control_hi: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub mixer_accent: Color32,
    pub osc_accents: [Color32; 6],
    pub meter_green: Color32,
    pub meter_yellow: Color32,
    pub meter_red: Color32,
    pub mute: Color32,
    pub solo: Color32,
    pub aux: Color32,
}

impl Palette {
    pub const GRAPHITE: Palette = Palette {
        bg: Color32::from_rgb(20, 21, 24),
        grid_minor: Color32::from_rgb(25, 26, 30),
        grid_major: Color32::from_rgb(31, 32, 37),
        panel: Color32::from_rgb(25, 26, 29),
        panel_border: Color32::from_rgb(38, 40, 45),
        node_body: Color32::from_rgb(31, 32, 36),
        node_header: Color32::from_rgb(36, 37, 42),
        node_border: Color32::from_rgb(50, 52, 58),
        inset: Color32::from_rgb(20, 21, 24),
        control: Color32::from_rgb(46, 48, 54),
        control_hi: Color32::from_rgb(60, 63, 70),
        text: Color32::from_rgb(226, 228, 232),
        text_dim: Color32::from_rgb(148, 152, 160),
        text_faint: Color32::from_rgb(96, 100, 108),
        accent: Color32::from_rgb(96, 165, 238),
        mixer_accent: Color32::from_rgb(214, 166, 92),
        osc_accents: [
            Color32::from_rgb(96, 165, 238),
            Color32::from_rgb(226, 112, 150),
            Color32::from_rgb(132, 194, 108),
            Color32::from_rgb(158, 138, 230),
            Color32::from_rgb(228, 142, 88),
            Color32::from_rgb(86, 188, 172),
        ],
        meter_green: Color32::from_rgb(92, 190, 122),
        meter_yellow: Color32::from_rgb(222, 188, 86),
        meter_red: Color32::from_rgb(226, 84, 84),
        mute: Color32::from_rgb(226, 92, 92),
        solo: Color32::from_rgb(222, 188, 86),
        aux: Color32::from_rgb(96, 196, 200),
    };

    pub fn porcelain() -> Palette {
        Palette {
            bg: Color32::from_rgb(238, 238, 234),
            grid_minor: Color32::from_rgb(232, 232, 227),
            grid_major: Color32::from_rgb(218, 219, 213),
            panel: Color32::from_rgb(242, 242, 238),
            panel_border: Color32::from_rgb(205, 206, 200),
            node_body: Color32::from_rgb(248, 248, 244),
            node_header: Color32::from_rgb(236, 236, 231),
            node_border: Color32::from_rgb(190, 192, 184),
            inset: Color32::from_rgb(226, 227, 221),
            control: Color32::from_rgb(211, 213, 205),
            control_hi: Color32::from_rgb(196, 199, 190),
            text: Color32::from_rgb(44, 46, 42),
            text_dim: Color32::from_rgb(92, 96, 88),
            text_faint: Color32::from_rgb(137, 140, 132),
            accent: Color32::from_rgb(50, 112, 184),
            mixer_accent: Color32::from_rgb(168, 112, 48),
            osc_accents: [
                Color32::from_rgb(50, 112, 184),
                Color32::from_rgb(176, 72, 112),
                Color32::from_rgb(68, 132, 72),
                Color32::from_rgb(118, 88, 168),
                Color32::from_rgb(176, 92, 44),
                Color32::from_rgb(40, 128, 120),
            ],
            meter_green: Color32::from_rgb(55, 142, 78),
            meter_yellow: Color32::from_rgb(176, 138, 44),
            meter_red: Color32::from_rgb(190, 62, 62),
            mute: Color32::from_rgb(190, 62, 62),
            solo: Color32::from_rgb(176, 138, 44),
            aux: Color32::from_rgb(44, 142, 150),
        }
    }

    pub fn sand() -> Palette {
        Palette {
            bg: Color32::from_rgb(226, 219, 203),
            grid_minor: Color32::from_rgb(218, 211, 196),
            grid_major: Color32::from_rgb(204, 196, 180),
            panel: Color32::from_rgb(232, 226, 212),
            panel_border: Color32::from_rgb(188, 179, 159),
            node_body: Color32::from_rgb(238, 232, 218),
            node_header: Color32::from_rgb(220, 213, 196),
            node_border: Color32::from_rgb(174, 165, 145),
            inset: Color32::from_rgb(210, 202, 184),
            control: Color32::from_rgb(198, 189, 168),
            control_hi: Color32::from_rgb(184, 174, 151),
            text: Color32::from_rgb(58, 50, 40),
            text_dim: Color32::from_rgb(102, 92, 76),
            text_faint: Color32::from_rgb(142, 132, 112),
            accent: Color32::from_rgb(154, 88, 42),
            mixer_accent: Color32::from_rgb(138, 102, 56),
            osc_accents: [
                Color32::from_rgb(154, 88, 42),
                Color32::from_rgb(154, 72, 98),
                Color32::from_rgb(86, 126, 78),
                Color32::from_rgb(118, 92, 150),
                Color32::from_rgb(184, 118, 52),
                Color32::from_rgb(54, 128, 132),
            ],
            meter_green: Color32::from_rgb(72, 132, 70),
            meter_yellow: Color32::from_rgb(170, 132, 48),
            meter_red: Color32::from_rgb(182, 74, 58),
            mute: Color32::from_rgb(182, 74, 58),
            solo: Color32::from_rgb(170, 132, 48),
            aux: Color32::from_rgb(54, 128, 132),
        }
    }

    pub fn midnight() -> Palette {
        Palette {
            bg: Color32::from_rgb(10, 13, 24),
            grid_minor: Color32::from_rgb(14, 18, 32),
            grid_major: Color32::from_rgb(20, 25, 44),
            panel: Color32::from_rgb(13, 17, 31),
            panel_border: Color32::from_rgb(30, 38, 64),
            node_body: Color32::from_rgb(18, 23, 40),
            node_header: Color32::from_rgb(24, 30, 52),
            node_border: Color32::from_rgb(42, 52, 82),
            inset: Color32::from_rgb(8, 11, 21),
            control: Color32::from_rgb(38, 48, 76),
            control_hi: Color32::from_rgb(52, 64, 96),
            text: Color32::from_rgb(224, 230, 246),
            text_dim: Color32::from_rgb(145, 158, 188),
            text_faint: Color32::from_rgb(91, 105, 138),
            accent: Color32::from_rgb(108, 154, 242),
            mixer_accent: Color32::from_rgb(214, 178, 104),
            osc_accents: [
                Color32::from_rgb(108, 154, 242),
                Color32::from_rgb(224, 112, 164),
                Color32::from_rgb(112, 194, 138),
                Color32::from_rgb(166, 142, 234),
                Color32::from_rgb(232, 150, 92),
                Color32::from_rgb(92, 188, 184),
            ],
            meter_green: Color32::from_rgb(92, 190, 122),
            meter_yellow: Color32::from_rgb(222, 188, 86),
            meter_red: Color32::from_rgb(226, 84, 84),
            mute: Color32::from_rgb(226, 92, 92),
            solo: Color32::from_rgb(222, 188, 86),
            aux: Color32::from_rgb(96, 196, 200),
        }
    }

    pub fn forest() -> Palette {
        Palette {
            bg: Color32::from_rgb(14, 22, 18),
            grid_minor: Color32::from_rgb(18, 28, 22),
            grid_major: Color32::from_rgb(25, 38, 30),
            panel: Color32::from_rgb(17, 27, 21),
            panel_border: Color32::from_rgb(36, 52, 43),
            node_body: Color32::from_rgb(23, 35, 28),
            node_header: Color32::from_rgb(29, 43, 35),
            node_border: Color32::from_rgb(49, 70, 58),
            inset: Color32::from_rgb(12, 19, 15),
            control: Color32::from_rgb(43, 62, 51),
            control_hi: Color32::from_rgb(58, 78, 66),
            text: Color32::from_rgb(226, 232, 222),
            text_dim: Color32::from_rgb(150, 166, 152),
            text_faint: Color32::from_rgb(100, 122, 107),
            accent: Color32::from_rgb(112, 188, 136),
            mixer_accent: Color32::from_rgb(210, 172, 104),
            osc_accents: [
                Color32::from_rgb(112, 188, 136),
                Color32::from_rgb(214, 132, 150),
                Color32::from_rgb(154, 194, 104),
                Color32::from_rgb(150, 148, 210),
                Color32::from_rgb(214, 148, 92),
                Color32::from_rgb(96, 184, 174),
            ],
            meter_green: Color32::from_rgb(112, 194, 138),
            meter_yellow: Color32::from_rgb(214, 184, 96),
            meter_red: Color32::from_rgb(218, 96, 92),
            mute: Color32::from_rgb(218, 96, 92),
            solo: Color32::from_rgb(214, 184, 96),
            aux: Color32::from_rgb(96, 184, 174),
        }
    }
}

static ACTIVE_PALETTE: RwLock<Palette> = RwLock::new(Palette::GRAPHITE);

pub fn set_palette(palette: Palette) {
    *ACTIVE_PALETTE.write().expect("theme palette lock") = palette;
}

pub fn palette() -> Palette {
    *ACTIVE_PALETTE.read().expect("theme palette lock")
}

pub fn bg() -> Color32 {
    palette().bg
}
pub fn grid_minor() -> Color32 {
    palette().grid_minor
}
pub fn grid_major() -> Color32 {
    palette().grid_major
}
pub fn panel() -> Color32 {
    palette().panel
}
pub fn panel_border() -> Color32 {
    palette().panel_border
}
pub fn node_body() -> Color32 {
    palette().node_body
}
pub fn node_header() -> Color32 {
    palette().node_header
}
pub fn node_border() -> Color32 {
    palette().node_border
}
pub fn inset() -> Color32 {
    palette().inset
}
pub fn control() -> Color32 {
    palette().control
}
pub fn control_hi() -> Color32 {
    palette().control_hi
}
pub fn text() -> Color32 {
    palette().text
}
pub fn text_dim() -> Color32 {
    palette().text_dim
}
pub fn text_faint() -> Color32 {
    palette().text_faint
}
pub fn accent() -> Color32 {
    palette().accent
}
pub fn mixer_accent() -> Color32 {
    palette().mixer_accent
}
pub fn osc_accents() -> [Color32; 6] {
    palette().osc_accents
}
pub fn meter_green() -> Color32 {
    palette().meter_green
}
pub fn meter_yellow() -> Color32 {
    palette().meter_yellow
}
pub fn meter_red() -> Color32 {
    palette().meter_red
}
pub fn mute() -> Color32 {
    palette().mute
}
pub fn solo() -> Color32 {
    palette().solo
}
pub fn aux() -> Color32 {
    palette().aux
}

/// Same hue with reduced opacity (premultiplied).
pub fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// Lerp between two opaque colours.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Snaps a font size to half points. Text on the canvas is sized by the
/// zoom; snapping lets zooming reuse a few dozen glyph sizes instead of
/// laying out and rasterizing text at a new size every frame.
pub fn snap(size: f32) -> f32 {
    (size * 2.0).round().max(1.0) / 2.0
}

/// Proportional font at a (zoomed) size.
pub fn font(size: f32) -> FontId {
    FontId::proportional(snap(size))
}

/// Monospace font at a (zoomed) size.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(snap(size))
}

/// Semibold proportional font, for titles.
pub fn strong(size: f32) -> FontId {
    FontId::new(snap(size), FontFamily::Name("strong".into()))
}

/// Uses the system UI fonts (SF Pro / SF Mono) when available, falling back
/// to egui's bundled fonts for anything they don't cover.
fn fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let load = |path: &str, coords: &[(&[u8; 4], f32)]| {
        let bytes = std::fs::read(path).ok()?;
        let tweak = FontTweak {
            coords: VariationCoords::new(coords.iter().copied()),
            ..Default::default()
        };
        Some(Arc::new(FontData::from_owned(bytes).tweak(tweak)))
    };
    let ui = "/System/Library/Fonts/SFNS.ttf";
    let fallback = defs.families[&FontFamily::Proportional].clone();
    let mut strong = fallback.clone();
    for (name, path, coords, family) in [
        (
            "sf",
            ui,
            &[(b"opsz", 17.0), (b"wght", 400.0)][..],
            FontFamily::Proportional,
        ),
        (
            "sf-strong",
            ui,
            &[(b"opsz", 17.0), (b"wght", 600.0)][..],
            FontFamily::Name("strong".into()),
        ),
        (
            "sf-mono",
            "/System/Library/Fonts/SFNSMono.ttf",
            &[(b"wght", 420.0)][..],
            FontFamily::Monospace,
        ),
    ] {
        let Some(data) = load(path, coords) else {
            continue;
        };
        defs.font_data.insert(name.to_owned(), data);
        match &family {
            FontFamily::Name(_) => strong.insert(0, name.to_owned()),
            f => defs
                .families
                .get_mut(f)
                .expect("default family")
                .insert(0, name.to_owned()),
        }
    }
    defs.families
        .insert(FontFamily::Name("strong".into()), strong);
    ctx.set_fonts(defs);
}

pub fn apply(ctx: &egui::Context) {
    fonts(ctx);

    let p = palette();
    let mut v = Visuals::dark();
    v.panel_fill = p.panel;
    v.window_fill = p.node_header;
    v.window_stroke = Stroke::new(1.0, p.node_border);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow.blur = 16;
    v.window_shadow.offset = [0, 4];
    v.window_shadow.color = Color32::from_black_alpha(90);
    v.popup_shadow = v.window_shadow;
    v.extreme_bg_color = p.inset;
    v.selection.bg_fill = alpha(p.accent, 0.30);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_dim);
    // Also used for panel separator lines.
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.panel_border);
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.weak_bg_fill = p.control;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.weak_bg_fill = p.control_hi;
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.corner_radius = CornerRadius::same(4);
    }
    // Always dark, whatever the OS appearance (menus and separators use these).
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    // Cmd +/- zoom the canvas, not the whole interface.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    ctx.all_styles_mut(|s| {
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.menu_margin = egui::Margin::same(4);
        s.spacing.item_spacing = egui::vec2(6.0, 2.0);
        s.interaction.tooltip_delay = 0.25;
    });
}
