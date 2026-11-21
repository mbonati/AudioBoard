//! Layout and painting of each node type. Coordinates are in node-local
//! world units and scaled by the canvas zoom through [`NodeView`].

use egui::{Align2, Color32, Id, Pos2, Rect, Shape, Stroke, Vec2, vec2};

use super::modules;
use super::plugin_node;
use super::theme;
use super::widgets::{self, hfader, hmeter, knob, toggle};
use crate::audio::engine::NodeId;
use crate::audio::params::{
    MIXER_CHANNELS, MixerParams, OSC_PARAMS, Waveform, fader_gain, format_db, format_hz,
    gain_to_db, osc,
};

use crate::graph::{MixerNode, Node, NodeKind, NodeType, OscNode, ParamRef};

/// Header height, card radius and the smallest text drawn come from the UI
/// Kit (`audioboard_ui`).
pub use audioboard_ui::card::HEADER_H;
pub use audioboard_ui::view::MIN_TEXT;
pub const OSC_SIZE: Vec2 = vec2(216.0, 268.0);
pub const MIX_SIZE: Vec2 = vec2(380.0, 282.0);

const OSC_PORT_Y: [f32; 2] = [42.0, 60.0];
const MIX_ROW_Y: f32 = 38.0;
const MIX_ROW_H: f32 = 38.0;
/// The master and AUX rows (where OUT and AUX leave).
const MIX_MASTER_Y: f32 = 214.0;
const MIX_AUX_Y: f32 = 252.0;

pub fn type_size(ty: NodeType) -> Vec2 {
    match ty {
        NodeType::Osc => OSC_SIZE,
        NodeType::Mixer => MIX_SIZE,
        NodeType::Console => super::console::size(Default::default()),
        NodeType::Module(k) => modules::size(k.def()),
        NodeType::Plugin => plugin_node::SIZE,
    }
}

pub fn input_offset(ty: NodeType, i: usize) -> Vec2 {
    match ty {
        NodeType::Osc => vec2(0.0, OSC_PORT_Y[i.min(1)]),
        NodeType::Mixer => vec2(0.0, MIX_ROW_Y + MIX_ROW_H * (i as f32 + 0.5)),
        NodeType::Console => super::console::input_offset(Default::default(), i),
        NodeType::Module(_) => modules::input_offset(i),
        NodeType::Plugin => plugin_node::input_offset(i),
    }
}

pub fn output_offset(ty: NodeType, i: usize) -> Vec2 {
    match ty {
        NodeType::Osc => vec2(OSC_SIZE.x, OSC_PORT_Y[0]),
        NodeType::Mixer => vec2(MIX_SIZE.x, if i == 0 { MIX_MASTER_Y } else { MIX_AUX_Y }),
        NodeType::Console => super::console::output_offset(Default::default(), i),
        NodeType::Module(k) => modules::output_offset(k.def(), i),
        NodeType::Plugin => plugin_node::output_offset(i),
    }
}

/// A control drawn this frame, for MIDI mapping (click one to map it).
pub type KnobHit = audioboard_ui::Hit<ParamRef>;

/// Maps node-local coordinates to the screen: the UI Kit's `NodeView`,
/// recording controls by their `ParamRef`.
pub type NodeView<'a> = audioboard_ui::NodeView<'a, ParamRef>;

/// What the user clicked in a node's header.
pub type FrameActions = audioboard_ui::card::CardActions;

/// Card, header (collapse chevron, activity LED, title, close button)
/// shared by every node: the UI Kit's card.
pub fn frame(v: &NodeView, node: &Node, selected: bool) -> FrameActions {
    // No card: only an outline while selected.
    if frameless(node) {
        if selected {
            let s = node.size();
            let stroke = Stroke::new(1.5 * v.z, node.accent);
            v.painter.rect_stroke(
                v.r(0.0, 0.0, s.x, s.y),
                4.0 * v.z,
                stroke,
                egui::StrokeKind::Outside,
            );
        }
        return FrameActions::default();
    }
    let card = audioboard_ui::card::Card {
        size: node.size(),
        title: &node.title,
        accent: node.accent,
        level: node.signal_level(),
        collapsed: node.collapsed,
        selected,
        bypass: node.is_effect().then(|| node.bypassed()),
    };
    audioboard_ui::card::card(v, &card)
}

/// Drawn without its card (an Iris set to show only its picture), unless
/// collapsed.
pub fn frameless(node: &Node) -> bool {
    !node.collapsed
        && matches!(&node.kind, NodeKind::Module(m)
            if m.params.kind == crate::audio::modules::ModuleKind::Iris && super::iris::frameless(&m.params))
}

/// A bypassed effect's body, dimmed over its controls.
pub fn bypass_veil(v: &NodeView, node: &Node) {
    audioboard_ui::card::veil(v, node.size());
}

// ---------------------------------------------------------------------------
// Collapsed nodes
// ---------------------------------------------------------------------------

pub const COLLAPSED_W: f32 = 168.0;
const COLLAPSED_ROW0: f32 = HEADER_H + 13.0;
const COLLAPSED_ROW_H: f32 = 18.0;

/// Size of a collapsed node with `rows` port rows.
pub fn collapsed_size(rows: usize) -> Vec2 {
    vec2(
        COLLAPSED_W,
        HEADER_H + 10.0 + COLLAPSED_ROW_H * rows.max(1) as f32,
    )
}

pub fn collapsed_port(i: usize, output: bool) -> Vec2 {
    let x = if output { COLLAPSED_W } else { 0.0 };
    vec2(x, COLLAPSED_ROW0 + COLLAPSED_ROW_H * i as f32)
}

/// The node's main value, shown when it is collapsed.
pub(super) fn summary(node: &Node, tempo: f32) -> String {
    match &node.kind {
        NodeKind::Osc(o) => {
            format_hz(o.params.get(osc::FREQ) * (o.params.get(osc::FINE) / 1200.0).exp2())
        }
        NodeKind::Mixer(m) => format_db(gain_to_db(fader_gain(m.params.master.get()))),
        NodeKind::Module(m) => modules::summary(&m.params, tempo),
        NodeKind::Plugin(p) => p.meta.vendor.clone(),
    }
}

/// Body of a collapsed node: port labels and the main value. `inputs` and
/// `outputs` say which of [`Node::input_ports`] / [`Node::output_ports`]
/// are patched.
pub fn collapsed(v: &NodeView, node: &Node, inputs: &[bool], outputs: &[bool]) {
    let row_y = |i: usize| COLLAPSED_ROW0 + COLLAPSED_ROW_H * i as f32;
    let label_col = |on: bool| {
        if on {
            theme::TEXT_DIM
        } else {
            theme::TEXT_FAINT
        }
    };
    for (i, port) in node.input_ports().into_iter().enumerate() {
        let on = inputs.get(i).copied().unwrap_or(false);
        v.text(
            12.0,
            row_y(i),
            Align2::LEFT_CENTER,
            node.port_label(port, false),
            v.font(9.0),
            label_col(on),
        );
    }
    for (i, port) in node.output_ports().into_iter().enumerate() {
        let on = outputs.get(i).copied().unwrap_or(false);
        let label = node.port_label(port, true);
        v.text(
            COLLAPSED_W - 12.0,
            row_y(i),
            Align2::RIGHT_CENTER,
            label,
            v.font(9.0),
            label_col(on),
        );
    }
    v.text(
        COLLAPSED_W * 0.5,
        row_y(0),
        Align2::CENTER_CENTER,
        summary(node, v.tempo),
        v.mono(10.5),
        theme::TEXT,
    );
    // The mixer is the output: keep its master meters in view.
    if let NodeKind::Mixer(m) = &node.kind {
        for (i, meter) in m.master.iter().enumerate() {
            let y = row_y(1) + 6.0 * i as f32;
            hmeter(
                v.painter,
                v.r(40.0, y, COLLAPSED_W - 40.0, y + 3.0),
                meter,
                v.z,
            );
        }
    }
}

pub(super) fn note_name(freq: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
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

pub fn osc(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    o: &mut OscNode,
    inputs: &[bool],
    output: bool,
) {
    let z = v.z;
    let p = v.painter;
    let w = OSC_SIZE.x;
    let params = &o.params;
    let wave = params.waveform();

    // Port labels.
    for (i, label) in ["FM", "AM"].iter().enumerate() {
        let col = if inputs[i] {
            theme::TEXT_DIM
        } else {
            theme::TEXT_FAINT
        };
        v.text(
            12.0,
            OSC_PORT_Y[i],
            Align2::LEFT_CENTER,
            label,
            v.font(9.0),
            col,
        );
    }
    v.text(
        w - 12.0,
        OSC_PORT_Y[0],
        Align2::RIGHT_CENTER,
        "OUT",
        v.font(9.0),
        if output {
            theme::TEXT_DIM
        } else {
            theme::TEXT_FAINT
        },
    );

    // Frequency readout.
    let freq = params.get(osc::FREQ) * (params.get(osc::FINE) / 1200.0).exp2();
    v.text(
        w * 0.5,
        OSC_PORT_Y[0],
        Align2::CENTER_CENTER,
        format_hz(freq),
        v.mono(13.0),
        theme::TEXT,
    );
    let sub = if wave == Waveform::Noise {
        "random".to_owned()
    } else if freq < 20.0 {
        format!("LFO · {:.2} s", 1.0 / freq)
    } else {
        note_name(freq)
    };
    v.text(
        w * 0.5,
        OSC_PORT_Y[1],
        Align2::CENTER_CENTER,
        sub,
        v.font(9.5),
        theme::TEXT_DIM,
    );

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
    let picked = segmented(
        v,
        ("wave", id),
        bar,
        Waveform::ALL.len(),
        wave as usize,
        accent,
        |i, rect, col| {
            let wf = Waveform::ALL[i];
            let glyph = widgets::wave_points(
                rect.shrink2(vec2(11.0 * z, 6.0 * z)),
                wf,
                0.0,
                1.0,
                1.0,
                0.0,
                7,
            );
            p.add(Shape::line(glyph, Stroke::new(1.3 * z, col)));
            wf.name()
        },
    );
    if let Some(i) = picked {
        params.set_waveform(Waveform::ALL[i]);
    }

    // Knobs: two rows of three.
    let order = [
        osc::FREQ,
        osc::FINE,
        osc::LEVEL,
        osc::SHAPE,
        osc::FM,
        osc::AM,
    ];
    let col_w = (w - 24.0) / 3.0;
    for (k, &pi) in order.iter().enumerate() {
        let spec = &OSC_PARAMS[pi];
        let cx = 12.0 + col_w * ((k % 3) as f32 + 0.5);
        let row_y = 156.0 + 58.0 * (k / 3) as f32;
        let unpatched = (pi == osc::FM && !inputs[0]) || (pi == osc::AM && !inputs[1]);
        let label = if pi == osc::SHAPE {
            wave.shape_label()
        } else {
            spec.name
        };
        let cell = KnobCell {
            cx,
            row_y,
            label,
            dimmed: unpatched,
            accent,
            param: ParamRef::Osc(pi),
        };
        if let Some(value) = knob_cell(v, Id::unique(("knob", id, pi)), spec, params.get(pi), cell)
        {
            params.set(pi, value);
        }
    }
}

/// Placement and look of one knob cell (label, knob, value).
pub type KnobCell<'a> = audioboard_ui::KnobCell<'a, ParamRef>;

/// Segmented selectors and knob cells come from the UI Kit.
pub use audioboard_ui::{knob_cell, segmented};

// ---------------------------------------------------------------------------
// Mixer
// ---------------------------------------------------------------------------

pub(super) fn pan_label(pan: f32) -> String {
    if pan.abs() < 0.01 {
        "C".to_owned()
    } else if pan < 0.0 {
        format!("L{:.0}", -pan * 100.0)
    } else {
        format!("R{:.0}", pan * 100.0)
    }
}

/// A channel's PAN (or, `aux`, AUX send) knob centred at `c` (node units),
/// radius `r`: MIDI can map it.
#[allow(clippy::too_many_arguments)]
pub(super) fn channel_knob(
    v: &NodeView,
    id: NodeId,
    params: &MixerParams,
    ch: usize,
    aux: bool,
    c: Pos2,
    r: f32,
    color: Color32,
) {
    let (mut norm, default) = if aux {
        (params.aux[ch].get(), 0.0)
    } else {
        ((params.pan[ch].get() + 1.0) * 0.5, 0.5)
    };
    let salt = if aux { "aux" } else { "pan" };
    let resp = knob(
        v.ui,
        v.painter,
        Id::unique((salt, id, ch)),
        v.p(c.x, c.y),
        r * v.z,
        &mut norm,
        default,
        !aux,
        color,
        v.z,
    );
    v.hit(
        resp.rect,
        if aux {
            ParamRef::Aux(ch)
        } else {
            ParamRef::Pan(ch)
        },
    );
    if resp.dragged() || resp.double_clicked() {
        if aux {
            params.aux[ch].set(norm);
        } else {
            params.pan[ch].set(norm * 2.0 - 1.0);
        }
    }
}

/// An AUX send's value: "off", or its gain.
pub(super) fn send_label(pos: f32) -> String {
    if pos <= 0.001 {
        "off".to_owned()
    } else {
        format_db(gain_to_db(fader_gain(pos)))
    }
}

/// MASTER's or AUX's fader (`aux`) in `rect` (screen): MIDI can map it.
pub(super) fn bus_fader(
    v: &NodeView,
    id: NodeId,
    params: &MixerParams,
    aux: bool,
    rect: Rect,
    vertical: bool,
) {
    let muted = !aux
        && params
            .master_mute
            .load(std::sync::atomic::Ordering::Relaxed);
    let (value, color) = if aux {
        (&params.aux_master, theme::AUX)
    } else {
        (&params.master, theme::TEXT_DIM)
    };
    let mut pos = value.get();
    let fid = Id::unique((if aux { "aux-master" } else { "master" }, id));
    let fr = if vertical {
        widgets::vfader(v.ui, v.painter, fid, rect, &mut pos, color, !muted, v.z)
    } else {
        hfader(v.ui, v.painter, fid, rect, &mut pos, color, !muted, v.z)
    };
    v.hit(
        fr.rect,
        if aux {
            ParamRef::AuxMaster
        } else {
            ParamRef::Master
        },
    );
    if fr.dragged() || fr.double_clicked() {
        value.set(pos);
    }
}

/// The master's MUTE button.
pub(super) fn master_mute(v: &NodeView, id: NodeId, params: &MixerParams, rect: Rect, label: &str) {
    use std::sync::atomic::Ordering::Relaxed;
    let muted = params.master_mute.load(Relaxed);
    let resp = toggle(
        v.ui,
        v.painter,
        Id::unique(("master-mute", id)),
        rect,
        label,
        muted,
        theme::MUTE,
        v.z,
    )
    .on_hover_text("Mute the main output (AUX keeps sending)");
    if resp.clicked() {
        params.master_mute.store(!muted, Relaxed);
    }
}

/// Where the main mix goes, at (`x`, `y`): the audio device (a speaker
/// that lights with the level), or the OUT cable when one is patched.
pub(super) fn main_out(
    v: &NodeView,
    x: f32,
    y: f32,
    m: &MixerNode,
    accent: Color32,
    patched: bool,
) {
    let z = v.z;
    let level = m.master[0]
        .level
        .max(m.master[1].level)
        .clamp(0.0, 1.0)
        .sqrt();
    let col = theme::mix(theme::TEXT_FAINT, accent, level);
    if patched {
        v.text(x, y, Align2::LEFT_CENTER, "Out by cable", v.font(8.5), col);
        return;
    }
    let sp = v.p(x + 4.0, y);
    v.painter.add(Shape::convex_polygon(
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
    v.text(
        x + 12.0,
        y,
        Align2::LEFT_CENTER,
        "Main out",
        v.font(8.5),
        col,
    );
}

/// Fader column, shared by the channel strips and the master bus.
const FADER_X: [f32; 2] = [140.0, 292.0];

/// `sources[ch]` is the colour and name of what's plugged into channel `ch`;
/// `outs`: OUT and AUX are patched.
pub fn mixer(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &mut MixerNode,
    sources: &[Option<(Color32, String)>],
    outs: (bool, bool),
) {
    use std::sync::atomic::Ordering::Relaxed;
    let z = v.z;
    let p = v.painter;
    let w = MIX_SIZE.x;
    let [fx0, fx1] = FADER_X;
    let params = &m.params;
    let any_solo = params.solo.iter().any(|s| s.load(Relaxed));

    v.text(
        88.0,
        31.0,
        Align2::CENTER_CENTER,
        "PAN",
        v.font(7.0),
        theme::TEXT_FAINT,
    );
    v.text(
        118.0,
        31.0,
        Align2::CENTER_CENTER,
        "AUX",
        v.font(7.0),
        theme::AUX,
    );
    for ch in 0..MIXER_CHANNELS {
        let cy = MIX_ROW_Y + MIX_ROW_H * (ch as f32 + 0.5);
        let source = sources.get(ch).and_then(|s| s.as_ref());
        let src_col = source.map(|s| s.0);

        v.text(
            17.0,
            cy,
            Align2::CENTER_CENTER,
            ch + 1,
            v.mono(10.0),
            src_col.unwrap_or(theme::TEXT_FAINT),
        );

        // Mute / solo.
        let muted = params.mute[ch].load(Relaxed);
        let soloed = params.solo[ch].load(Relaxed);
        if toggle(
            v.ui,
            p,
            Id::unique(("mute", id, ch)),
            v.r(30.0, cy - 8.0, 46.0, cy + 8.0),
            "M",
            muted,
            theme::MUTE,
            z,
        )
        .clicked()
        {
            params.mute[ch].store(!muted, Relaxed);
        }
        if toggle(
            v.ui,
            p,
            Id::unique(("solo", id, ch)),
            v.r(50.0, cy - 8.0, 66.0, cy + 8.0),
            "S",
            soloed,
            theme::SOLO,
            z,
        )
        .clicked()
        {
            params.solo[ch].store(!soloed, Relaxed);
        }
        let audible = !muted && (!any_solo || soloed);

        // Pan and AUX send.
        channel_knob(
            v,
            id,
            params,
            ch,
            false,
            Pos2::new(88.0, cy - 3.0),
            9.0,
            src_col.unwrap_or(theme::TEXT_FAINT),
        );
        v.text(
            88.0,
            cy + 13.0,
            Align2::CENTER_CENTER,
            pan_label(params.pan[ch].get()),
            v.mono(7.5),
            theme::TEXT_FAINT,
        );
        channel_knob(
            v,
            id,
            params,
            ch,
            true,
            Pos2::new(118.0, cy - 3.0),
            9.0,
            theme::AUX,
        );
        v.text(
            118.0,
            cy + 13.0,
            Align2::CENTER_CENTER,
            send_label(params.aux[ch].get()),
            v.mono(7.5),
            theme::TEXT_FAINT,
        );

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
            if audible {
                theme::TEXT
            } else {
                theme::TEXT_FAINT
            },
        );
        let (name, name_col) = match source {
            Some((_, n)) => (n.as_str(), theme::TEXT_DIM),
            None => ("No input", theme::TEXT_FAINT),
        };
        v.text(
            w - 12.0,
            cy + 9.0,
            Align2::RIGHT_CENTER,
            name,
            v.font(8.5),
            name_col,
        );
    }

    // Master and AUX.
    let sep_y = MIX_ROW_Y + MIX_ROW_H * MIXER_CHANNELS as f32 + 4.0;
    p.line_segment(
        [v.p(12.0, sep_y), v.p(w - 12.0, sep_y)],
        Stroke::new(1.0 * z, theme::NODE_BORDER),
    );
    let muted = params.master_mute.load(Relaxed);
    for (aux, cy) in [(false, MIX_MASTER_Y), (true, MIX_AUX_Y)] {
        let (label, col) = if aux {
            ("AUX", theme::AUX)
        } else {
            ("MASTER", theme::TEXT_DIM)
        };
        v.text(
            12.0,
            cy - 5.0,
            Align2::LEFT_CENTER,
            label,
            theme::strong(9.5 * z),
            col,
        );
        bus_fader(v, id, params, aux, v.r(fx0, cy - 6.0, fx1, cy + 6.0), false);
        let (value, meters) = if aux {
            (params.aux_master.get(), &m.aux)
        } else {
            (params.master.get(), &m.master)
        };
        let off = !aux && muted;
        v.text(
            w - 12.0,
            cy - 3.0,
            Align2::RIGHT_CENTER,
            if off {
                "muted".to_owned()
            } else {
                format_db(gain_to_db(fader_gain(value)))
            },
            v.mono(10.0),
            if off { theme::MUTE } else { theme::TEXT },
        );
        for (i, meter) in meters.iter().enumerate() {
            let y = cy + 9.0 + i as f32 * 6.0;
            v.text(
                fx0 - 8.0,
                y + 1.5,
                Align2::CENTER_CENTER,
                ["L", "R"][i],
                v.font(7.0),
                theme::TEXT_FAINT,
            );
            hmeter(p, v.r(fx0, y, fx1, y + 3.0), meter, z);
        }
    }
    master_mute(
        v,
        id,
        params,
        v.r(96.0, MIX_MASTER_Y - 9.0, 128.0, MIX_MASTER_Y + 7.0),
        "MUTE",
    );
    main_out(v, 12.0, MIX_MASTER_Y + 10.0, m, accent, outs.0);
    v.text(
        12.0,
        MIX_AUX_Y + 10.0,
        Align2::LEFT_CENTER,
        if outs.1 {
            "Sends by cable"
        } else {
            "Not patched"
        },
        v.font(8.5),
        theme::TEXT_FAINT,
    );

    let peak = m.master[0].peak.max(m.master[1].peak);
    let clip = peak >= 0.999;
    let clip_rect = v.r(w - 40.0, MIX_MASTER_Y + 7.0, w - 12.0, MIX_MASTER_Y + 19.0);
    p.rect_filled(
        clip_rect,
        2.0 * z,
        if clip {
            theme::METER_RED
        } else {
            theme::CONTROL
        },
    );
    p.text(
        clip_rect.center(),
        Align2::CENTER_CENTER,
        "CLIP",
        v.font(7.5),
        if clip {
            theme::INSET
        } else {
            theme::TEXT_FAINT
        },
    );
}
