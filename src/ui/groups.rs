//! Modules (groups of nodes): an expanded module is a frame around its
//! nodes, which stay editable in place; a collapsed one is a single card
//! showing only the ports that link it to the rest of the patch.

use egui::{Align2, CornerRadius, CursorIcon, Id, Painter, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::nodes::HEADER_H;
use super::theme::{self, alpha};
use super::widgets;
use crate::graph::{Graph, Group, PortRef};

/// Margin between a module's frame and its nodes.
pub const PAD: f32 = 14.0;
pub const CARD_W: f32 = 236.0;
const ROW0: f32 = HEADER_H + 13.0;
const ROW_H: f32 = 18.0;
const RADIUS: f32 = 8.0;

/// World rect of a module: its nodes plus margins and a header strip. A
/// collapsed module's card sits at this rect's top-left corner.
pub fn frame_rect(graph: &Graph, g: &Group) -> Option<Rect> {
    let bounds = g
        .members
        .iter()
        .filter_map(|id| graph.node(*id))
        .map(|n| Rect::from_min_size(n.pos, n.size()))
        .reduce(|a, b| a.union(b))?;
    Some(Rect::from_min_max(
        bounds.min - vec2(PAD, PAD + HEADER_H),
        bounds.max + vec2(PAD, PAD),
    ))
}

pub fn card_size(inputs: usize, outputs: usize) -> Vec2 {
    vec2(CARD_W, HEADER_H + 10.0 + ROW_H * inputs.max(outputs).max(1) as f32)
}

pub fn card_port(i: usize, output: bool) -> Vec2 {
    vec2(if output { CARD_W } else { 0.0 }, ROW0 + ROW_H * i as f32)
}

/// What was clicked in a module's header.
pub struct HeaderActions {
    pub toggle_collapse: bool,
    pub ungroup: bool,
    /// Where the title sits (screen space), for editing it in place.
    pub title_rect: Rect,
}

/// Chevron, title, an optional note and the ungroup button along the top
/// `HEADER_H` of `rect` (screen space).
#[allow(clippy::too_many_arguments)]
fn header(
    ui: &Ui,
    p: &Painter,
    g: &Group,
    rect: Rect,
    note: &str,
    led: Option<f32>,
    hovered: bool,
    renaming: bool,
    z: f32,
) -> HeaderActions {
    let cy = rect.top() + HEADER_H * 0.5 * z;

    let fold_rect = Rect::from_center_size(pos2(rect.left() + 12.0 * z, cy), Vec2::splat(16.0 * z));
    let fold = ui
        .interact(fold_rect, Id::unique(("gfold", g.id)), Sense::CLICK)
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(if g.collapsed { "Expand module (C)" } else { "Collapse module (C)" });
    let c = fold_rect.center();
    let s = 3.0 * z;
    let tri = if g.collapsed {
        vec![c + vec2(-0.6 * s, -s), c + vec2(0.9 * s, 0.0), c + vec2(-0.6 * s, s)]
    } else {
        vec![c + vec2(-s, -0.6 * s), c + vec2(s, -0.6 * s), c + vec2(0.0, 0.9 * s)]
    };
    p.add(Shape::convex_polygon(
        tri,
        if fold.hovered() { theme::TEXT } else { theme::TEXT_FAINT },
        Stroke::NONE,
    ));

    let mut x = rect.left() + 24.0 * z;
    if let Some(level) = led {
        p.circle_filled(
            pos2(x + 2.0 * z, cy),
            3.5 * z,
            theme::mix(theme::CONTROL_HI, theme::ACCENT, 0.35 + 0.65 * level.clamp(0.0, 1.0).sqrt()),
        );
        x += 12.0 * z;
    }
    let title_rect = Rect::from_min_max(
        pos2(x - 3.0 * z, rect.top() + 4.0 * z),
        pos2(rect.right() - 28.0 * z, rect.top() + (HEADER_H - 4.0) * z),
    );
    if !renaming {
        let title = p.text(pos2(x, cy), Align2::LEFT_CENTER, &g.title, theme::strong(11.5 * z), theme::TEXT);
        p.text(
            pos2(title.right() + 8.0 * z, cy),
            Align2::LEFT_CENTER,
            note,
            egui::FontId::proportional(9.5 * z),
            theme::TEXT_FAINT,
        );
    }

    let close_rect = Rect::from_center_size(pos2(rect.right() - 13.0 * z, cy), Vec2::splat(16.0 * z));
    let close = ui
        .interact(close_rect, Id::unique(("gclose", g.id)), Sense::CLICK)
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Ungroup (keeps the nodes)");
    if hovered {
        let c = close_rect.center();
        if close.hovered() {
            p.circle_filled(c, 8.0 * z, theme::CONTROL_HI);
        }
        let col = if close.hovered() { theme::TEXT } else { theme::TEXT_FAINT };
        let d = 3.0 * z;
        p.line_segment([c - vec2(d, d), c + vec2(d, d)], Stroke::new(1.2 * z, col));
        p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], Stroke::new(1.2 * z, col));
    }

    HeaderActions {
        toggle_collapse: fold.clicked(),
        ungroup: close.clicked(),
        title_rect,
    }
}

/// An expanded module: a translucent frame behind its nodes.
pub fn draw_frame(ui: &Ui, p: &Painter, g: &Group, rect: Rect, selected: bool, renaming: bool, z: f32) -> HeaderActions {
    let r = RADIUS * z;
    p.rect_filled(rect, r, alpha(theme::TEXT, 0.025));
    let top = CornerRadius {
        nw: (r.round() as u8),
        ne: (r.round() as u8),
        sw: 0,
        se: 0,
    };
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H * z));
    p.rect_filled(head, top, alpha(theme::TEXT, 0.04));
    let border = if selected {
        Stroke::new(1.5 * z, theme::ACCENT)
    } else {
        Stroke::new(1.0 * z, alpha(theme::NODE_BORDER, 0.9))
    };
    p.rect_stroke(rect, r, border, StrokeKind::Inside);

    let n = g.members.len();
    let note = if n == 1 { "1 node".to_owned() } else { format!("{n} nodes") };
    header(ui, p, g, rect, &note, None, ui.rect_contains_pointer(head) || selected, renaming, z)
}

/// A collapsed module: one card with the module's outer ports.
#[allow(clippy::too_many_arguments)]
pub fn draw_card(
    ui: &Ui,
    p: &Painter,
    graph: &Graph,
    g: &Group,
    rect: Rect,
    ports: (&[PortRef], &[PortRef]),
    connected: impl Fn(PortRef, bool) -> bool,
    selected: bool,
    renaming: bool,
    z: f32,
) -> HeaderActions {
    let r = 6.0 * z;
    widgets::shadow(p, rect, r, z);
    p.rect_filled(rect, r, theme::NODE_BODY);
    let top = CornerRadius {
        nw: (r.round() as u8),
        ne: (r.round() as u8),
        sw: 0,
        se: 0,
    };
    p.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H * z)), top, theme::NODE_HEADER);
    p.hline(rect.x_range(), rect.top() + HEADER_H * z, Stroke::new(1.0 * z, theme::NODE_BORDER));
    // Stacked-card edge: this node holds several.
    p.hline(
        (rect.left() + 6.0 * z)..=(rect.right() - 6.0 * z),
        rect.bottom() + 3.0 * z,
        Stroke::new(1.0 * z, theme::NODE_BORDER),
    );

    let level = g
        .members
        .iter()
        .filter_map(|id| graph.node(*id))
        .map(|n| n.signal_level())
        .fold(0.0, f32::max);
    let n = g.members.len();
    let note = if n == 1 { "1 node".to_owned() } else { format!("{n} nodes") };
    let hovered = ui.rect_contains_pointer(rect) || selected;
    let actions = header(ui, p, g, rect, &note, Some(level), hovered, renaming, z);

    // Port labels: "DRONE 1 · PITCH".
    let label = |pr: PortRef, input: bool| {
        let node = graph.node(pr.node);
        let title = node.map_or("?", |n| n.title.as_str());
        let port = node.map_or("?", |n| n.port_label(pr.port, !input));
        format!("{title} · {port}")
    };
    let font = egui::FontId::proportional(8.5 * z);
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    let (ins, outs) = ports;
    for (i, pr) in ins.iter().enumerate() {
        let pos = rect.min + card_port(i, false) * z + vec2(12.0 * z, 0.0);
        p.text(pos, Align2::LEFT_CENTER, label(*pr, true), font.clone(), col(connected(*pr, true)));
    }
    for (i, pr) in outs.iter().enumerate() {
        let pos = rect.min + card_port(i, true) * z - vec2(12.0 * z, 0.0);
        p.text(pos, Align2::RIGHT_CENTER, label(*pr, false), font.clone(), col(connected(*pr, false)));
    }

    let border = if selected {
        Stroke::new(1.5 * z, theme::ACCENT)
    } else {
        Stroke::new(1.0 * z, theme::NODE_BORDER)
    };
    p.rect_stroke(
        rect,
        r,
        border,
        if selected { StrokeKind::Outside } else { StrokeKind::Inside },
    );
    actions
}

/// World rect and outer ports of a collapsed module's card.
pub fn card_layout(graph: &Graph, g: &Group) -> Option<(Rect, Vec<PortRef>, Vec<PortRef>)> {
    let frame = frame_rect(graph, g)?;
    let (ins, outs) = graph.exposed_ports(g);
    let rect = Rect::from_min_size(frame.min, card_size(ins.len(), outs.len()));
    Some((rect, ins, outs))
}
