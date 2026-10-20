//! Layout and painting of each node type. Coordinates are in node-local
//! world units and scaled by the canvas zoom through [`NodeView`].

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Painter, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use super::modules;
use super::plugin_node;
use super::theme::{self, alpha};
use super::widgets::{self, hfader, hmeter, knob, toggle};
use crate::audio::engine::NodeId;
use crate::audio::params::{
    MIXER_CHANNELS, OSC_PARAMS, ParamSpec, Waveform, fader_gain, format_db, format_hz, gain_to_db, osc,
};
use std::cell::RefCell;

use crate::graph::{MixerNode, Node, NodeKind, NodeType, OscNode, ParamRef};

pub const HEADER_H: f32 = 26.0;
pub const OSC_SIZE: Vec2 = vec2(216.0, 268.0);
pub const MIX_SIZE: Vec2 = vec2(340.0, 244.0);

const RADIUS: f32 = 6.0;
const OSC_PORT_Y: [f32; 2] = [42.0, 60.0];
const MIX_ROW_Y: f32 = 34.0;
const MIX_ROW_H: f32 = 38.0;

pub fn type_size(ty: NodeType) -> Vec2 {
    match ty {
        NodeType::Osc => OSC_SIZE,
        NodeType::Mixer => MIX_SIZE,
        NodeType::Module(k) => modules::size(k.def()),
        NodeType::Plugin => plugin_node::SIZE,
    }
}

pub fn input_offset(ty: NodeType, i: usize) -> Vec2 {
    match ty {
        NodeType::Osc => vec2(0.0, OSC_PORT_Y[i.min(1)]),
        NodeType::Mixer => vec2(0.0, MIX_ROW_Y + MIX_ROW_H * (i as f32 + 0.5)),
        NodeType::Module(_) => modules::input_offset(i),
        NodeType::Plugin => plugin_node::input_offset(i),
    }
}

pub fn output_offset(ty: NodeType, i: usize) -> Vec2 {
    match ty {
        NodeType::Osc => vec2(OSC_SIZE.x, OSC_PORT_Y[0]),
        NodeType::Mixer => vec2(MIX_SIZE.x, MIX_SIZE.y * 0.5),
        NodeType::Module(k) => modules::output_offset(k.def(), i),
        NodeType::Plugin => plugin_node::output_offset(i),
    }
}

/// A control drawn this frame, for MIDI mapping (click one to map it).
#[derive(Clone, Copy)]
pub struct KnobHit {
    pub rect: Rect,
    pub node: NodeId,
    pub param: ParamRef,
}

/// Maps node-local coordinates to the screen.
pub struct NodeView<'a> {
    pub ui: &'a Ui,
    pub painter: &'a Painter,
    pub min: Pos2,
    pub z: f32,
    pub node: NodeId,
    /// Collects every knob and fader drawn.
    pub hits: &'a RefCell<Vec<KnobHit>>,
}

impl NodeView<'_> {
    pub(super) fn p(&self, x: f32, y: f32) -> Pos2 {
        self.min + vec2(x, y) * self.z
    }

    pub(super) fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }

    /// Records a control for MIDI mapping.
    pub(super) fn hit(&self, rect: Rect, param: ParamRef) {
        self.hits.borrow_mut().push(KnobHit { rect, node: self.node, param });
    }

    pub(super) fn font(&self, size: f32) -> FontId {
        FontId::proportional(size * self.z)
    }

    pub(super) fn mono(&self, size: f32) -> FontId {
        FontId::monospace(size * self.z)
    }

    pub(super) fn text(&self, x: f32, y: f32, align: Align2, text: impl ToString, font: FontId, color: Color32) -> Rect {
        self.painter.text(self.p(x, y), align, text, font, color)
    }
}

fn radius(r: f32, z: f32) -> u8 {
    (r * z).round().clamp(0.0, 255.0) as u8
}

/// What the user clicked in a node's header.
pub struct FrameActions {
    pub close: bool,
    pub toggle_collapse: bool,
}

/// Card, header (collapse chevron, activity LED, title, close button)
/// shared by every node.
pub fn frame(v: &NodeView, node: &Node, selected: bool) -> FrameActions {
    let z = v.z;
    let size = node.size();
    let rect = v.r(0.0, 0.0, size.x, size.y);
    let p = v.painter;

    widgets::shadow(p, rect, RADIUS * z, z);
    p.rect_filled(rect, RADIUS * z, theme::NODE_BODY);
    let top = CornerRadius {
        nw: radius(RADIUS, z),
        ne: radius(RADIUS, z),
        sw: 0,
        se: 0,
    };
    p.rect_filled(v.r(0.0, 0.0, size.x, HEADER_H), top, theme::NODE_HEADER);
    p.line_segment(
        [v.p(0.0, HEADER_H), v.p(size.x, HEADER_H)],
        Stroke::new(1.0 * z, theme::NODE_BORDER),
    );

    // Collapse chevron: points down when expanded, right when collapsed.
    let fold_rect = Rect::from_center_size(v.p(12.0, HEADER_H * 0.5), Vec2::splat(16.0 * z));
    let fold = v
        .ui
        .interact(fold_rect, Id::unique(("fold", node.id)), Sense::CLICK)
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(if node.collapsed { "Expand (C)" } else { "Collapse (C)" });
    let c = fold_rect.center();
    let s = 3.0 * z;
    let tri = if node.collapsed {
        vec![c + vec2(-0.6 * s, -s), c + vec2(0.9 * s, 0.0), c + vec2(-0.6 * s, s)]
    } else {
        vec![c + vec2(-s, -0.6 * s), c + vec2(s, -0.6 * s), c + vec2(0.0, 0.9 * s)]
    };
    let fold_col = if fold.hovered() { theme::TEXT } else { theme::TEXT_FAINT };
    p.add(Shape::convex_polygon(tri, fold_col, Stroke::NONE));

    // Activity LED in the node's colour, lit by its output level.
    let level = node.signal_level().clamp(0.0, 1.0).sqrt();
    p.circle_filled(
        v.p(26.0, HEADER_H * 0.5),
        3.5 * z,
        theme::mix(theme::CONTROL_HI, node.accent, 0.35 + 0.65 * level),
    );
    v.text(36.0, HEADER_H * 0.5, Align2::LEFT_CENTER, &node.title, theme::strong(11.5 * z), theme::TEXT);

    // Close button, only shown while the node is hovered or selected.
    let close_rect = Rect::from_center_size(v.p(size.x - 13.0, HEADER_H * 0.5), Vec2::splat(16.0 * z));
    let close = v
        .ui
        .interact(close_rect, Id::unique(("close", node.id)), Sense::CLICK)
        .on_hover_cursor(CursorIcon::PointingHand);
    if selected || v.ui.rect_contains_pointer(rect) {
        let c = close_rect.center();
        if close.hovered() {
            p.circle_filled(c, 8.0 * z, theme::CONTROL_HI);
        }
        let col = if close.hovered() { theme::TEXT } else { theme::TEXT_FAINT };
        let d = 3.0 * z;
        p.line_segment([c - vec2(d, d), c + vec2(d, d)], Stroke::new(1.2 * z, col));
        p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], Stroke::new(1.2 * z, col));
    }

    if selected {
        p.rect_stroke(rect, RADIUS * z, Stroke::new(1.5 * z, node.accent), StrokeKind::Outside);
    } else {
        p.rect_stroke(rect, RADIUS * z, Stroke::new(1.0 * z, theme::NODE_BORDER), StrokeKind::Inside);
    }
    FrameActions {
        close: close.clicked(),
        toggle_collapse: fold.clicked(),
    }
}

// ---------------------------------------------------------------------------
// Collapsed nodes
// ---------------------------------------------------------------------------

pub const COLLAPSED_W: f32 = 168.0;
const COLLAPSED_ROW0: f32 = HEADER_H + 13.0;
const COLLAPSED_ROW_H: f32 = 18.0;

/// Size of a collapsed node with `rows` port rows.
pub fn collapsed_size(rows: usize) -> Vec2 {
    vec2(COLLAPSED_W, HEADER_H + 10.0 + COLLAPSED_ROW_H * rows.max(1) as f32)
}

pub fn collapsed_port(i: usize, output: bool) -> Vec2 {
    let x = if output { COLLAPSED_W } else { 0.0 };
    vec2(x, COLLAPSED_ROW0 + COLLAPSED_ROW_H * i as f32)
}

/// The node's main value, shown when it is collapsed.
fn summary(node: &Node) -> String {
    match &node.kind {
        NodeKind::Osc(o) => format_hz(o.params.get(osc::FREQ) * (o.params.get(osc::FINE) / 1200.0).exp2()),
        NodeKind::Mixer(m) => format_db(gain_to_db(fader_gain(m.params.master.get()))),
        NodeKind::Module(m) => modules::summary(&m.params),
        NodeKind::Plugin(p) => p.meta.vendor.clone(),
    }
}

/// Body of a collapsed node: port labels and the main value.
pub fn collapsed(v: &NodeView, node: &Node, inputs: &[bool], output: bool) {
    let row_y = |i: usize| COLLAPSED_ROW0 + COLLAPSED_ROW_H * i as f32;
    let label_col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    for (i, label) in node.inputs().iter().enumerate() {
        let on = inputs.get(i).copied().unwrap_or(false);
        v.text(12.0, row_y(i), Align2::LEFT_CENTER, label, v.font(9.0), label_col(on));
    }
    for (i, label) in node.outputs().iter().enumerate() {
        v.text(COLLAPSED_W - 12.0, row_y(i), Align2::RIGHT_CENTER, label, v.font(9.0), label_col(output));
    }
    v.text(COLLAPSED_W * 0.5, row_y(0), Align2::CENTER_CENTER, summary(node), v.mono(10.5), theme::TEXT);
    // The mixer is the output: keep its master meters in view.
    if let NodeKind::Mixer(m) = &node.kind {
        for (i, meter) in m.master.iter().enumerate() {
            let y = row_y(1) + 6.0 * i as f32;
            hmeter(v.painter, v.r(40.0, y, COLLAPSED_W - 40.0, y + 3.0), meter, v.z);
        }
    }
}

pub(super) fn note_name(freq: f32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let midi = 69.0 + 12.0 * (freq / 440.0).log2();
    let nearest = midi.round();
    let cents = ((midi - nearest) * 100.0).round() as i32;
    let n = nearest as i32;
    let name = NAMES[n.rem_euclid(12) as usize];
    let octave = n.div_euclid(12) - 1;
    if cents == 0 {
        format!("{name}{octave}")
    } else {
        format!("{name}{octave} {cents:+}ct")
    }
}

// ---------------------------------------------------------------------------
// Oscillator
// ---------------------------------------------------------------------------

pub fn osc(v: &NodeView, id: NodeId, accent: Color32, o: &mut OscNode, inputs: &[bool], output: bool) {
    let z = v.z;
    let p = v.painter;
    let w = OSC_SIZE.x;
    let params = &o.params;
    let wave = params.waveform();

    // Port labels.
    for (i, label) in ["FM", "AM"].iter().enumerate() {
        let col = if inputs[i] { theme::TEXT_DIM } else { theme::TEXT_FAINT };
        v.text(12.0, OSC_PORT_Y[i], Align2::LEFT_CENTER, label, v.font(9.0), col);
    }
    v.text(
        w - 12.0,
        OSC_PORT_Y[0],
        Align2::RIGHT_CENTER,
        "OUT",
        v.font(9.0),
        if output { theme::TEXT_DIM } else { theme::TEXT_FAINT },
    );

    // Frequency readout.
    let freq = params.get(osc::FREQ) * (params.get(osc::FINE) / 1200.0).exp2();
    v.text(w * 0.5, OSC_PORT_Y[0], Align2::CENTER_CENTER, format_hz(freq), v.mono(13.0), theme::TEXT);
    let sub = if wave == Waveform::Noise {
        "random".to_owned()
    } else if freq < 20.0 {
        format!("LFO · {:.2} s", 1.0 / freq)
    } else {
        note_name(freq)
    };
    v.text(w * 0.5, OSC_PORT_Y[1], Align2::CENTER_CENTER, sub, v.font(9.5), theme::TEXT_DIM);

    // Waveform display (static: shape and level only).
    let display = v.r(12.0, 72.0, w - 12.0, 114.0);
    p.rect_filled(display, 4.0 * z, theme::INSET);
    p.line_segment(
        [display.left_center(), display.right_center()],
        Stroke::new(1.0 * z, theme::GRID_MAJOR),
    );
    let level = params.get(osc::LEVEL).max(0.03);
    let pts = widgets::wave_points(
        display.shrink2(vec2(6.0 * z, 7.0 * z)),
        wave,
        params.get(osc::SHAPE),
        2.0,
        level,
        0.0,
        1,
    );
    p.add(Shape::line(pts, Stroke::new(1.4 * z, accent)));

    // Waveform selector.
    let bar = v.r(12.0, 120.0, w - 12.0, 140.0);
    let picked = segmented(v, ("wave", id), bar, Waveform::ALL.len(), wave as usize, accent, |i, rect, col| {
        let wf = Waveform::ALL[i];
        let glyph = widgets::wave_points(rect.shrink2(vec2(11.0 * z, 6.0 * z)), wf, 0.0, 1.0, 1.0, 0.0, 7);
        p.add(Shape::line(glyph, Stroke::new(1.3 * z, col)));
        wf.name()
    });
    if let Some(i) = picked {
        params.set_waveform(Waveform::ALL[i]);
    }

    // Knobs: two rows of three.
    let order = [osc::FREQ, osc::FINE, osc::LEVEL, osc::SHAPE, osc::FM, osc::AM];
    let col_w = (w - 24.0) / 3.0;
    for (k, &pi) in order.iter().enumerate() {
        let spec = &OSC_PARAMS[pi];
        let cx = 12.0 + col_w * ((k % 3) as f32 + 0.5);
        let row_y = 156.0 + 58.0 * (k / 3) as f32;
        let unpatched = (pi == osc::FM && !inputs[0]) || (pi == osc::AM && !inputs[1]);
        let label = if pi == osc::SHAPE { wave.shape_label() } else { spec.name };
        let cell = KnobCell {
            cx,
            row_y,
            label,
            dimmed: unpatched,
            accent,
            param: ParamRef::Osc(pi),
        };
        if let Some(value) = knob_cell(v, Id::unique(("knob", id, pi)), spec, params.get(pi), cell) {
            params.set(pi, value);
        }
    }
}

/// Segmented control in `rect` (screen space). `item` paints segment `i`
/// in the given colour and returns its tooltip. Returns a clicked segment.
pub(super) fn segmented(
    v: &NodeView,
    salt: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    count: usize,
    selected: usize,
    accent: Color32,
    mut item: impl FnMut(usize, Rect, Color32) -> &'static str,
) -> Option<usize> {
    let z = v.z;
    let p = v.painter;
    p.rect_filled(rect, 4.0 * z, theme::INSET);
    let seg_w = rect.width() / count as f32;
    let base = Id::unique(salt);
    let mut picked = None;
    for i in 0..count {
        let seg = Rect::from_min_size(rect.min + vec2(seg_w * i as f32, 0.0), vec2(seg_w, rect.height()));
        let resp = v
            .ui
            .interact(seg, base.with(i), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand);
        let on = i == selected;
        if on {
            p.rect_filled(seg.shrink(2.0 * z), 3.0 * z, theme::CONTROL);
        }
        let col = if on {
            accent
        } else if resp.hovered() {
            theme::TEXT_DIM
        } else {
            theme::TEXT_FAINT
        };
        let tip = item(i, seg, col);
        if resp.on_hover_text(tip).clicked() {
            picked = Some(i);
        }
    }
    picked
}

/// Placement and look of one knob cell (label, knob, value).
pub(super) struct KnobCell<'a> {
    pub cx: f32,
    /// Node-local y of the label line.
    pub row_y: f32,
    pub label: &'a str,
    /// For modulation-depth knobs whose input is unpatched.
    pub dimmed: bool,
    pub accent: Color32,
    pub param: ParamRef,
}

/// Draws a knob cell; returns the new value when it was moved or reset.
pub(super) fn knob_cell(v: &NodeView, id: Id, spec: &ParamSpec, value: f32, cell: KnobCell) -> Option<f32> {
    let z = v.z;
    let KnobCell { cx, row_y, label, dimmed, accent, param } = cell;
    let label_col = if dimmed { alpha(theme::TEXT_FAINT, 0.6) } else { theme::TEXT_FAINT };
    v.text(cx, row_y, Align2::CENTER_CENTER, label, v.font(8.5), label_col);
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
        if hot { theme::TEXT } else { theme::TEXT_DIM },
    );
    new
}

// ---------------------------------------------------------------------------
// Mixer
// ---------------------------------------------------------------------------

fn pan_label(pan: f32) -> String {
    if pan.abs() < 0.01 {
        "C".to_owned()
    } else if pan < 0.0 {
        format!("L{:.0}", -pan * 100.0)
    } else {
        format!("R{:.0}", pan * 100.0)
    }
}

/// Fader column, shared by the channel strips and the master bus.
const FADER_X: [f32; 2] = [110.0, 262.0];

/// `sources[ch]` is the colour and name of what's plugged into channel `ch`.
pub fn mixer(v: &NodeView, id: NodeId, accent: Color32, m: &mut MixerNode, sources: &[Option<(Color32, String)>]) {
    use std::sync::atomic::Ordering::Relaxed;
    let z = v.z;
    let p = v.painter;
    let w = MIX_SIZE.x;
    let [fx0, fx1] = FADER_X;
    let params = &m.params;
    let any_solo = params.solo.iter().any(|s| s.load(Relaxed));

    for ch in 0..MIXER_CHANNELS {
        let cy = MIX_ROW_Y + MIX_ROW_H * (ch as f32 + 0.5);
        let source = sources.get(ch).and_then(|s| s.as_ref());
        let src_col = source.map(|s| s.0);

        v.text(17.0, cy, Align2::CENTER_CENTER, ch + 1, v.mono(10.0), src_col.unwrap_or(theme::TEXT_FAINT));

        // Mute / solo.
        let muted = params.mute[ch].load(Relaxed);
        let soloed = params.solo[ch].load(Relaxed);
        if toggle(v.ui, p, Id::unique(("mute", id, ch)), v.r(30.0, cy - 8.0, 46.0, cy + 8.0), "M", muted, theme::MUTE, z)
            .clicked()
        {
            params.mute[ch].store(!muted, Relaxed);
        }
        if toggle(v.ui, p, Id::unique(("solo", id, ch)), v.r(50.0, cy - 8.0, 66.0, cy + 8.0), "S", soloed, theme::SOLO, z)
            .clicked()
        {
            params.solo[ch].store(!soloed, Relaxed);
        }
        let audible = !muted && (!any_solo || soloed);

        // Pan.
        let pan = params.pan[ch].get();
        let mut norm = (pan + 1.0) * 0.5;
        let r = knob(
            v.ui,
            p,
            Id::unique(("pan", id, ch)),
            v.p(88.0, cy - 3.0),
            9.0 * z,
            &mut norm,
            0.5,
            true,
            src_col.unwrap_or(theme::TEXT_FAINT),
            z,
        );
        v.hit(r.rect, ParamRef::Pan(ch));
        if r.dragged() || r.double_clicked() {
            params.pan[ch].set(norm * 2.0 - 1.0);
        }
        v.text(88.0, cy + 13.0, Align2::CENTER_CENTER, pan_label(params.pan[ch].get()), v.mono(7.5), theme::TEXT_FAINT);

        // Fader + meter.
        let mut pos = params.fader[ch].get();
        let fr = hfader(
            v.ui,
            p,
            Id::unique(("fader", id, ch)),
            v.r(fx0, cy - 10.0, fx1, cy + 2.0),
            &mut pos,
            src_col.unwrap_or(theme::TEXT_FAINT),
            audible,
            z,
        );
        v.hit(fr.rect, ParamRef::Fader(ch));
        if fr.dragged() || fr.double_clicked() {
            params.fader[ch].set(pos);
        }
        hmeter(p, v.r(fx0, cy + 6.0, fx1, cy + 9.0), &m.channels[ch], z);

        let db = gain_to_db(fader_gain(params.fader[ch].get()));
        v.text(
            w - 12.0,
            cy - 4.0,
            Align2::RIGHT_CENTER,
            format_db(db),
            v.mono(10.0),
            if audible { theme::TEXT } else { theme::TEXT_FAINT },
        );
        let (name, name_col) = match source {
            Some((_, n)) => (n.as_str(), theme::TEXT_DIM),
            None => ("No input", theme::TEXT_FAINT),
        };
        v.text(w - 12.0, cy + 9.0, Align2::RIGHT_CENTER, name, v.font(8.5), name_col);
    }

    // Master section.
    let sep_y = MIX_ROW_Y + MIX_ROW_H * MIXER_CHANNELS as f32 + 4.0;
    p.line_segment([v.p(12.0, sep_y), v.p(w - 12.0, sep_y)], Stroke::new(1.0 * z, theme::NODE_BORDER));

    let cy = sep_y + 18.0;
    v.text(12.0, cy, Align2::LEFT_CENTER, "MASTER", theme::strong(9.5 * z), theme::TEXT_DIM);

    let mut pos = params.master.get();
    let fr = hfader(
        v.ui,
        p,
        Id::unique(("master", id)),
        v.r(fx0, cy - 6.0, fx1, cy + 6.0),
        &mut pos,
        theme::TEXT_DIM,
        true,
        z,
    );
    v.hit(fr.rect, ParamRef::Master);
    if fr.dragged() || fr.double_clicked() {
        params.master.set(pos);
    }
    v.text(
        w - 12.0,
        cy,
        Align2::RIGHT_CENTER,
        format_db(gain_to_db(fader_gain(params.master.get()))),
        v.mono(10.0),
        theme::TEXT,
    );

    let my = cy + 14.0;
    for (i, label) in ["L", "R"].iter().enumerate() {
        let y = my + i as f32 * 6.0;
        v.text(fx0 - 8.0, y + 1.5, Align2::CENTER_CENTER, label, v.font(7.0), theme::TEXT_FAINT);
        hmeter(p, v.r(fx0, y, fx1, y + 3.0), &m.master[i], z);
    }

    let peak = m.master[0].peak.max(m.master[1].peak);
    let clip = peak >= 0.999;
    let clip_rect = v.r(w - 40.0, my - 2.0, w - 12.0, my + 11.0);
    p.rect_filled(clip_rect, 2.0 * z, if clip { theme::METER_RED } else { theme::CONTROL });
    p.text(
        clip_rect.center(),
        Align2::CENTER_CENTER,
        "CLIP",
        v.font(7.5),
        if clip { theme::INSET } else { theme::TEXT_FAINT },
    );

    // Main-out indicator: this node feeds the audio device.
    let level = m.master[0].level.max(m.master[1].level).clamp(0.0, 1.0).sqrt();
    let col = theme::mix(theme::TEXT_FAINT, accent, level);
    let sp = v.p(16.0, my + 4.5);
    p.add(Shape::convex_polygon(
        vec![
            sp + vec2(-4.0, -2.0) * z,
            sp + vec2(-1.5, -2.0) * z,
            sp + vec2(2.0, -5.0) * z,
            sp + vec2(2.0, 5.0) * z,
            sp + vec2(-1.5, 2.0) * z,
            sp + vec2(-4.0, 2.0) * z,
        ],
        col,
        Stroke::NONE,
    ));
    v.text(24.0, my + 4.5, Align2::LEFT_CENTER, "Main out", v.font(8.5), col);
}
