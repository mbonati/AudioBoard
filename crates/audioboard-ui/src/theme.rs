//! Colour palette, fonts and global egui styling: AudioBoard's look. Flat
//! graphite surfaces, one accent colour per node, coloured outlines and
//! line art (no glows, no 3D).

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};
use egui::epaint::text::{FontTweak, VariationCoords};

// Neutral graphite surfaces, from back to front.
pub const BG: Color32 = Color32::from_rgb(20, 21, 24);
pub const GRID_MINOR: Color32 = Color32::from_rgb(25, 26, 30);
pub const GRID_MAJOR: Color32 = Color32::from_rgb(31, 32, 37);

pub const PANEL: Color32 = Color32::from_rgb(25, 26, 29);
pub const PANEL_BORDER: Color32 = Color32::from_rgb(38, 40, 45);

pub const NODE_BODY: Color32 = Color32::from_rgb(31, 32, 36);
pub const NODE_HEADER: Color32 = Color32::from_rgb(36, 37, 42);
pub const NODE_BORDER: Color32 = Color32::from_rgb(50, 52, 58);
pub const INSET: Color32 = Color32::from_rgb(20, 21, 24);
pub const CONTROL: Color32 = Color32::from_rgb(46, 48, 54);
pub const CONTROL_HI: Color32 = Color32::from_rgb(60, 63, 70);

pub const TEXT: Color32 = Color32::from_rgb(226, 228, 232);
pub const TEXT_DIM: Color32 = Color32::from_rgb(148, 152, 160);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(96, 100, 108);

/// UI accent (selection, focus, analyzers).
pub const ACCENT: Color32 = Color32::from_rgb(96, 165, 238);

pub const MIXER_ACCENT: Color32 = Color32::from_rgb(214, 166, 92);
pub const OSC_ACCENTS: [Color32; 6] = [
    Color32::from_rgb(96, 165, 238),  // blue
    Color32::from_rgb(226, 112, 150), // rose
    Color32::from_rgb(132, 194, 108), // green
    Color32::from_rgb(158, 138, 230), // violet
    Color32::from_rgb(228, 142, 88),  // orange
    Color32::from_rgb(86, 188, 172),  // teal
];

pub const METER_GREEN: Color32 = Color32::from_rgb(92, 190, 122);
pub const METER_YELLOW: Color32 = Color32::from_rgb(222, 188, 86);
pub const METER_RED: Color32 = Color32::from_rgb(226, 84, 84);

pub const MUTE: Color32 = Color32::from_rgb(226, 92, 92);
pub const SOLO: Color32 = Color32::from_rgb(222, 188, 86);
/// Mixers' AUX sends and output.
pub const AUX: Color32 = Color32::from_rgb(96, 196, 200);

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
        ("sf", ui, &[(b"opsz", 17.0), (b"wght", 400.0)][..], FontFamily::Proportional),
        ("sf-strong", ui, &[(b"opsz", 17.0), (b"wght", 600.0)][..], FontFamily::Name("strong".into())),
        ("sf-mono", "/System/Library/Fonts/SFNSMono.ttf", &[(b"wght", 420.0)][..], FontFamily::Monospace),
    ] {
        let Some(data) = load(path, coords) else { continue };
        defs.font_data.insert(name.to_owned(), data);
        match &family {
            FontFamily::Name(_) => strong.insert(0, name.to_owned()),
            f => defs.families.get_mut(f).expect("default family").insert(0, name.to_owned()),
        }
    }
    defs.families.insert(FontFamily::Name("strong".into()), strong);
    ctx.set_fonts(defs);
}

pub fn apply(ctx: &egui::Context) {
    fonts(ctx);

    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = Color32::from_rgb(33, 34, 38);
    v.window_stroke = Stroke::new(1.0, NODE_BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_shadow.blur = 16;
    v.window_shadow.offset = [0, 4];
    v.window_shadow.color = Color32::from_black_alpha(90);
    v.popup_shadow = v.window_shadow;
    v.extreme_bg_color = INSET;
    v.selection.bg_fill = alpha(ACCENT, 0.30);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    // Also used for panel separator lines.
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, PANEL_BORDER);
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.weak_bg_fill = CONTROL;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.weak_bg_fill = CONTROL_HI;
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
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
