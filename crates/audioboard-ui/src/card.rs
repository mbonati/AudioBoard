//! The card every node is drawn on: body, header (collapse chevron,
//! activity LED, title, bypass switch, close button) and outline.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Vec2, vec2,
};

use crate::theme::{self, alpha};
use crate::view::NodeView;
use crate::widgets;

/// Height of a node's header, in node units.
pub const HEADER_H: f32 = 26.0;
/// Corner radius of a node's card, in node units.
pub const RADIUS: f32 = 6.0;

/// A corner radius in node units, at zoom `z`.
pub fn radius(r: f32, z: f32) -> u8 {
    (r * z).round().clamp(0.0, 255.0) as u8
}

/// What the card shows.
pub struct Card<'a> {
    pub size: Vec2,
    pub title: &'a str,
    pub accent: Color32,
    /// Output level (0…1+), lighting the header's LED.
    pub level: f32,
    pub collapsed: bool,
    pub selected: bool,
    /// An effect: Some(bypassed) shows the bypass switch.
    pub bypass: Option<bool>,
}

/// What the user clicked in the header.
#[derive(Default)]
pub struct CardActions {
    pub close: bool,
    pub toggle_collapse: bool,
    /// The bypass switch (effects only) was clicked.
    pub bypass: bool,
}

/// Draws the card and its header; returns what was clicked.
pub fn card<P>(v: &NodeView<'_, P>, c: &Card<'_>) -> CardActions {
    let z = v.z;
    let size = c.size;
    let rect = v.r(0.0, 0.0, size.x, size.y);
    let p = v.painter;

    widgets::shadow(p, rect, RADIUS * z, z);
    p.rect_filled(rect, RADIUS * z, theme::node_body());
    let top = CornerRadius {
        nw: radius(RADIUS, z),
        ne: radius(RADIUS, z),
        sw: 0,
        se: 0,
    };
    p.rect_filled(v.r(0.0, 0.0, size.x, HEADER_H), top, theme::node_header());
    p.line_segment(
        [v.p(0.0, HEADER_H), v.p(size.x, HEADER_H)],
        Stroke::new(1.0 * z, theme::node_border()),
    );

    // Collapse chevron: points down when expanded, right when collapsed.
    let fold_rect = Rect::from_center_size(v.p(12.0, HEADER_H * 0.5), Vec2::splat(16.0 * z));
    let fold =
        v.ui.interact(fold_rect, Id::unique(("fold", v.node)), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(if c.collapsed {
                "Expand (C)"
            } else {
                "Collapse (C)"
            });
    let center = fold_rect.center();
    let s = 3.0 * z;
    let tri = if c.collapsed {
        vec![
            center + vec2(-0.6 * s, -s),
            center + vec2(0.9 * s, 0.0),
            center + vec2(-0.6 * s, s),
        ]
    } else {
        vec![
            center + vec2(-s, -0.6 * s),
            center + vec2(s, -0.6 * s),
            center + vec2(0.0, 0.9 * s),
        ]
    };
    let fold_col = if fold.hovered() {
        theme::text()
    } else {
        theme::text()_FAINT
    };
    p.add(Shape::convex_polygon(tri, fold_col, Stroke::NONE));

    // Activity LED in the node's colour, lit by its output level.
    let level = c.level.clamp(0.0, 1.0).sqrt();
    p.circle_filled(
        v.p(26.0, HEADER_H * 0.5),
        3.5 * z,
        theme::mix(theme::control()_HI, c.accent, 0.35 + 0.65 * level),
    );
    v.text(
        36.0,
        HEADER_H * 0.5,
        Align2::LEFT_CENTER,
        c.title,
        theme::strong(11.5 * z),
        theme::text(),
    );

    // Close button, only shown while the node is hovered or selected.
    let close_rect =
        Rect::from_center_size(v.p(size.x - 13.0, HEADER_H * 0.5), Vec2::splat(16.0 * z));
    let close =
        v.ui.interact(close_rect, Id::unique(("close", v.node)), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand);
    if c.selected || v.ui.rect_contains_pointer(rect) {
        let center = close_rect.center();
        if close.hovered() {
            p.circle_filled(center, 8.0 * z, theme::control()_HI);
        }
        let col = if close.hovered() {
            theme::text()
        } else {
            theme::text()_FAINT
        };
        let d = 3.0 * z;
        p.line_segment(
            [center - vec2(d, d), center + vec2(d, d)],
            Stroke::new(1.2 * z, col),
        );
        p.line_segment(
            [center + vec2(-d, d), center + vec2(d, -d)],
            Stroke::new(1.2 * z, col),
        );
    }

    // Bypass: a power switch, lit while the effect is on.
    let mut bypass = false;
    if let Some(off) = c.bypass {
        let bp_rect =
            Rect::from_center_size(v.p(size.x - 34.0, HEADER_H * 0.5), Vec2::splat(16.0 * z));
        let resp =
            v.ui.interact(bp_rect, Id::unique(("bypass", v.node)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if off {
                    "Bypassed: click to switch the effect back on (B)"
                } else {
                    "Bypass: hear the dry signal (B)"
                });
        if resp.hovered() {
            p.circle_filled(bp_rect.center(), 8.0 * z, theme::control()_HI);
        }
        let col = if off { theme::meter_yellow() } else { c.accent };
        power_icon(p, bp_rect.center(), 4.0 * z, Stroke::new(1.3 * z, col));
        if off {
            v.text(
                size.x - 46.0,
                HEADER_H * 0.5,
                Align2::RIGHT_CENTER,
                "BYPASS",
                theme::strong(8.0 * z),
                theme::meter_yellow(),
            );
        }
        bypass = resp.clicked();
    }

    if c.selected {
        p.rect_stroke(
            rect,
            RADIUS * z,
            Stroke::new(1.5 * z, c.accent),
            StrokeKind::Outside,
        );
    } else {
        p.rect_stroke(
            rect,
            RADIUS * z,
            Stroke::new(1.0 * z, theme::node_border()),
            StrokeKind::Inside,
        );
    }
    CardActions {
        close: close.clicked(),
        toggle_collapse: fold.clicked(),
        bypass,
    }
}

/// A power symbol: a ring open at the top, and a bar through the gap.
pub fn power_icon(p: &Painter, c: Pos2, r: f32, stroke: Stroke) {
    let arc: Vec<Pos2> = (0..=20)
        .map(|k| {
            let a = -std::f32::consts::FRAC_PI_2
                + 0.7
                + (std::f32::consts::TAU - 1.4) * k as f32 / 20.0;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::line(arc, stroke));
    p.line_segment([c + vec2(0.0, -r * 1.25), c + vec2(0.0, -r * 0.2)], stroke);
}

/// A bypassed effect's body, dimmed over its controls.
pub fn veil<P>(v: &NodeView<'_, P>, size: Vec2) {
    let rect = v.r(0.0, HEADER_H, size.x, size.y);
    let bottom = CornerRadius {
        nw: 0,
        ne: 0,
        sw: radius(RADIUS, v.z),
        se: radius(RADIUS, v.z),
    };
    v.painter
        .rect_filled(rect, bottom, alpha(theme::node_body(), 0.55));
}
