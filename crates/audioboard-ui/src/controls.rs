//! Panel controls built on [`NodeView`]: a labelled knob with its value
//! ([`knob_cell`]), a segmented selector ([`segmented`]), pills and port
//! labels.

use egui::{Align2, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, vec2};

use audioboard_node::ParamSpec;

use crate::theme::{self, alpha};
use crate::view::NodeView;
use crate::widgets::knob;

/// Segmented control in `rect` (screen space). `item` paints segment `i`
/// in the given colour and returns its tooltip. Returns a clicked segment.
pub fn segmented<P>(
    v: &NodeView<'_, P>,
    salt: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    count: usize,
    selected: usize,
    accent: Color32,
    mut item: impl FnMut(usize, Rect, Color32) -> &'static str,
) -> Option<usize> {
    let z = v.z;
    let p = v.painter;
    p.rect_filled(rect, 4.0 * z, theme::inset());
    let seg_w = rect.width() / count.max(1) as f32;
    let base = Id::unique(salt);
    let mut picked = None;
    for i in 0..count {
        let seg = Rect::from_min_size(
            rect.min + vec2(seg_w * i as f32, 0.0),
            vec2(seg_w, rect.height()),
        );
        let resp =
            v.ui.interact(seg, base.with(i), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand);
        let on = i == selected;
        if on {
            p.rect_filled(seg.shrink(2.0 * z), 3.0 * z, theme::control());
        }
        let col = if on {
            accent
        } else if resp.hovered() {
            theme::text()_DIM
        } else {
            theme::text()_FAINT
        };
        let tip = item(i, seg, col);
        let resp = if tip.is_empty() {
            resp
        } else {
            resp.on_hover_text(tip)
        };
        if resp.clicked() {
            picked = Some(i);
        }
    }
    picked
}

/// Placement and look of one knob cell (label, knob, value).
pub struct KnobCell<'a, P> {
    pub cx: f32,
    /// Node-local y of the label line.
    pub row_y: f32,
    pub label: &'a str,
    /// For modulation-depth knobs whose input is unpatched.
    pub dimmed: bool,
    pub accent: Color32,
    /// The control, for MIDI mapping.
    pub param: P,
}

/// Height of a knob cell, label to value, in node units.
pub const KNOB_CELL_H: f32 = 46.0;

/// Draws a knob cell; returns the new value when it was moved or reset.
pub fn knob_cell<P>(
    v: &NodeView<'_, P>,
    id: Id,
    spec: &ParamSpec,
    value: f32,
    cell: KnobCell<'_, P>,
) -> Option<f32> {
    let z = v.z;
    let KnobCell {
        cx,
        row_y,
        label,
        dimmed,
        accent,
        param,
    } = cell;
    let label_col = if dimmed {
        alpha(theme::text()_FAINT, 0.6)
    } else {
        theme::text()_FAINT
    };
    v.text(
        cx,
        row_y,
        Align2::CENTER_CENTER,
        label,
        v.font(8.5),
        label_col,
    );
    let mut norm = spec.to_norm(value);
    let resp = knob(
        v.ui,
        v.painter,
        id,
        v.p(cx, row_y + 19.0),
        12.0 * z,
        &mut norm,
        spec.to_norm(spec.default),
        spec.bipolar,
        accent,
        z,
    );
    v.hit(resp.rect, param);
    let new = (resp.dragged() || resp.double_clicked()).then(|| spec.denorm(norm));
    let hot = resp.dragged() || resp.hovered();
    v.text(
        cx,
        row_y + 38.0,
        Align2::CENTER_CENTER,
        spec.format(new.unwrap_or(value)),
        v.mono(9.0),
        if hot { theme::text() } else { theme::text()_DIM },
    );
    new
}

/// A small outlined tag ("SYNC", "EXT", "LIVE") centred at `(x, y)` in node
/// units. Filled with `color` when `on`.
pub fn pill<P>(v: &NodeView<'_, P>, x: f32, y: f32, text: &str, color: Color32, on: bool) -> Rect {
    let z = v.z;
    let font = theme::strong(7.5 * z);
    let w = v
        .painter
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x
        / z.max(1e-3)
        + 10.0;
    let rect = v.r(x - w * 0.5, y - 6.0, x + w * 0.5, y + 6.0);
    if on {
        v.painter.rect_filled(rect, 6.0 * z, color);
    } else {
        v.painter.rect_stroke(
            rect,
            6.0 * z,
            Stroke::new(1.0 * z, alpha(color, 0.8)),
            StrokeKind::Inside,
        );
    }
    if font.size >= crate::view::MIN_TEXT {
        v.painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            font,
            if on { theme::inset() } else { color },
        );
    }
    rect
}

/// A column of port labels at `x` from `y0`, `dy` apart: left-aligned for
/// inputs, right-aligned for outputs, brighter when patched.
pub fn port_labels<P>(
    v: &NodeView<'_, P>,
    x: f32,
    y0: f32,
    dy: f32,
    labels: &[&str],
    patched: &[bool],
    output: bool,
) {
    let align = if output {
        Align2::RIGHT_CENTER
    } else {
        Align2::LEFT_CENTER
    };
    for (i, label) in labels.iter().enumerate() {
        let on = patched.get(i).copied().unwrap_or(false);
        let col = if on {
            theme::text()_DIM
        } else {
            theme::text()_FAINT
        };
        v.text(x, y0 + dy * i as f32, align, label, v.font(9.0), col);
    }
}

/// A caption over a group of controls: small, faint, upper case.
pub fn caption<P>(v: &NodeView<'_, P>, x: f32, y: f32, text: &str) {
    v.text(
        x,
        y,
        Align2::LEFT_CENTER,
        text,
        theme::strong(7.5 * v.z),
        theme::text()_FAINT,
    );
}
