//! Panels for the ambient modules. Every module shares one layout: ports
//! and a two-line readout on top, a live display, an optional mode selector,
//! then a grid of knobs. Only the readout and the display are per kind.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{KnobCell, NodeView, knob_cell, note_name, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::midi::note_hz;
use crate::audio::modules::{
    ModuleDef, ModuleKind, ModuleParams, aether, arpeggiator, larsen, looper, vesper, audio_in, automa, colloid, delay, drone, equalizer, filter, frakture, magma, pops, granular, midi_cc, midi_in, midi_note,
    midicer, nullpunkt, pingpong, random, resonator, reverb, sequencer, steps, tape,
};
use crate::audio::params::{
    DIVISIONS, NOTE_NAMES, NOTE_VALUES, format_hz, format_seconds, note_label, sync_division,
};
use crate::graph::{ModuleNode, ParamRef};

const PAD: f32 = 12.0;
const COL_W: f32 = 64.0;
const PORT_Y: f32 = 42.0;
const PORT_DY: f32 = 18.0;
const ROW_H: f32 = 58.0;

fn columns(def: &ModuleDef) -> usize {
    match def.params.len() {
        4 => 4,
        n if n >= 7 => 4,
        _ => 3,
    }
}

/// Node-local top of the display: below the port rows.
fn display_top(def: &ModuleDef) -> f32 {
    let rows = (def.inputs.len() + def.midi_in as usize).max(def.outputs.len() + def.midi_out as usize);
    (PORT_Y + PORT_DY * rows.saturating_sub(1) as f32 + 12.0).max(72.0)
}

fn display_bottom(def: &ModuleDef) -> f32 {
    display_top(def) + def.display_h
}

/// Node-local bottom of the extra controls under the display, if any.
fn controls_bottom(def: &ModuleDef) -> f32 {
    display_bottom(def) + if def.controls_h > 0.0 { 6.0 + def.controls_h } else { 0.0 }
}

/// Node-local top and bottom of the mode selector row.
fn selector_y(def: &ModuleDef) -> [f32; 2] {
    let d = controls_bottom(def);
    [d + 6.0, d + 26.0]
}

/// Node-local y of the first knob row's labels.
fn knobs_top(def: &ModuleDef) -> f32 {
    controls_bottom(def) + if def.modes.is_empty() && !def.map { 18.0 } else { 42.0 }
}

/// A module node's size now: most only depend on their kind, some panels
/// grow and shrink with their settings.
pub fn size_of(p: &ModuleParams) -> Vec2 {
    match p.kind {
        ModuleKind::Pops => super::pops::size(p),
        ModuleKind::Tape => super::tape::size(p),
        kind => size(kind.def()),
    }
}

pub fn size(def: &ModuleDef) -> Vec2 {
    if std::ptr::eq(def, &pops::DEF) {
        return super::pops::SIZE;
    }
    if std::ptr::eq(def, &frakture::DEF) {
        return super::frakture::SIZE;
    }
    if std::ptr::eq(def, &equalizer::DEF) {
        return super::equalizer::SIZE;
    }
    if std::ptr::eq(def, &magma::DEF) {
        return super::magma::SIZE;
    }
    if std::ptr::eq(def, &tape::DEF) {
        return super::tape::SIZE;
    }
    if std::ptr::eq(def, &nullpunkt::DEF) {
        return super::nullpunkt::SIZE;
    }
    if std::ptr::eq(def, &aether::DEF) {
        return super::aether::SIZE;
    }
    if std::ptr::eq(def, &larsen::DEF) {
        return super::larsen::SIZE;
    }
    if std::ptr::eq(def, &vesper::DEF) {
        return super::vesper::SIZE;
    }
    if std::ptr::eq(def, &looper::DEF) {
        return super::looper::SIZE;
    }
    if std::ptr::eq(def, &automa::DEF) {
        return super::automa::SIZE;
    }
    if std::ptr::eq(def, &colloid::DEF) {
        return super::colloid::SIZE;
    }
    let cols = columns(def);
    let rows = def.params.len().div_ceil(cols);
    vec2(
        PAD * 2.0 + COL_W * cols as f32,
        knobs_top(def) + ROW_H * rows as f32 - 4.0,
    )
}

pub fn input_offset(i: usize) -> Vec2 {
    vec2(0.0, PORT_Y + PORT_DY * i as f32)
}

pub fn output_offset(def: &ModuleDef, i: usize) -> Vec2 {
    vec2(size(def).x, PORT_Y + PORT_DY * i as f32)
}

/// What the canvas tells a module panel about MIDI mapping.
#[derive(Default)]
pub struct MapState<'a> {
    /// The control this MIDI node drives ("FILTER 1 · CUTOFF").
    pub target: Option<&'a str>,
    /// Waiting for a click on the control to map.
    pub picking: bool,
}

/// MIDI nodes: the MAP button, if clicked, and where it is.
#[derive(Default)]
pub struct MapButton {
    pub clicked: bool,
    pub rect: Option<Rect>,
}

/// `ports` says which audio inputs and outputs are patched, `midi` whether
/// the MIDI input and output are.
pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &ModuleNode,
    (inputs, outputs): (&[bool], &[bool]),
    midi: (bool, bool),
    map: MapState,
) -> MapButton {
    let p = &m.params;
    let kind = p.kind;
    let def = kind.def();
    let z = v.z;
    let w = size(def).x;
    let patched = |i: usize| inputs.get(i).copied().unwrap_or(false);
    if kind == ModuleKind::Frakture {
        super::frakture::draw(v, id, accent, m, (patched(0), outputs.first().copied().unwrap_or(false)));
        return MapButton::default();
    }
    if kind == ModuleKind::Pops {
        let out = outputs.first().copied().unwrap_or(false);
        super::pops::draw(v, id, accent, m, out, midi);
        return MapButton::default();
    }
    if kind == ModuleKind::Magma {
        let out = outputs.first().copied().unwrap_or(false);
        super::magma::draw(v, id, accent, m, [patched(0), patched(1), out], midi.0);
        return MapButton::default();
    }
    if kind == ModuleKind::Nullpunkt {
        super::nullpunkt::draw(v, id, accent, m, outputs.first().copied().unwrap_or(false));
        return MapButton::default();
    }
    if kind == ModuleKind::Aether {
        super::aether::draw(v, id, accent, m, (midi.0, outputs.first().copied().unwrap_or(false)));
        return MapButton::default();
    }
    if kind == ModuleKind::Looper {
        super::looper::draw(v, id, accent, m, midi);
        return MapButton::default();
    }
    if kind == ModuleKind::Larsen {
        super::larsen::draw(v, id, accent, m, (patched(0), midi.0, outputs.first().copied().unwrap_or(false)));
        return MapButton::default();
    }
    if kind == ModuleKind::Vesper {
        super::vesper::draw(v, id, accent, m, (midi.0, midi.1, outputs.first().copied().unwrap_or(false)));
        return MapButton::default();
    }
    if kind == ModuleKind::Automa {
        super::automa::draw(v, id, accent, m, midi.1);
        return MapButton::default();
    }
    if kind == ModuleKind::Colloid {
        super::colloid::draw(v, id, accent, m, midi);
        return MapButton::default();
    }
    if kind == ModuleKind::Tape {
        super::tape::draw(v, id, accent, m, (patched(0), outputs.first().copied().unwrap_or(false)));
        return MapButton::default();
    }
    if kind == ModuleKind::Equalizer {
        let out = outputs.first().copied().unwrap_or(false);
        super::equalizer::draw(v, id, accent, m, [patched(0), patched(1), out]);
        return MapButton::default();
    }
    let label_col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };

    // Ports.
    for (i, label) in def.inputs.iter().enumerate() {
        v.text(PAD, PORT_Y + PORT_DY * i as f32, Align2::LEFT_CENTER, label, v.font(9.0), label_col(patched(i)));
    }
    for (i, label) in def.outputs.iter().enumerate() {
        let on = outputs.get(i).copied().unwrap_or(false);
        v.text(w - PAD, PORT_Y + PORT_DY * i as f32, Align2::RIGHT_CENTER, label, v.font(9.0), label_col(on));
    }
    if def.midi_in {
        let y = PORT_Y + PORT_DY * def.inputs.len() as f32;
        v.text(PAD, y, Align2::LEFT_CENTER, "MIDI", v.font(9.0), label_col(midi.0));
    }
    if def.midi_out {
        let y = PORT_Y + PORT_DY * def.outputs.len() as f32;
        v.text(w - PAD, y, Align2::RIGHT_CENTER, "MIDI", v.font(9.0), label_col(midi.1));
    }

    // Readout.
    let (main, sub) = readout(kind, p, v.tempo);
    let sub = if map.picking {
        "Click a knob or fader…".to_owned()
    } else if let Some(target) = map.target {
        format!("→ {target}")
    } else {
        sub
    };
    v.text(w * 0.5, PORT_Y, Align2::CENTER_CENTER, main, v.mono(13.0), theme::TEXT);
    let sub_col = if map.target.is_some() || map.picking { accent } else { theme::TEXT_DIM };
    // The second line shares its row with the toggle, if there is one; a
    // mapping that does not fit shows just the control's name.
    let (left, right) = (PAD + 30.0, if def.toggle.is_some() { w - PAD - 50.0 } else { w - PAD - 30.0 });
    let fits = |s: &str| v.painter.layout_no_wrap(s.to_owned(), v.font(9.5), sub_col).size().x <= (right - left) * z;
    let sub = match map.target {
        Some(t) if !map.picking && !fits(&sub) => format!("→ {}", t.rsplit(" · ").next().unwrap_or(t)),
        _ => sub,
    };
    let cx = if def.toggle.is_some() { (left + right) * 0.5 } else { w * 0.5 };
    v.text(cx, PORT_Y + PORT_DY, Align2::CENTER_CENTER, sub, v.font(9.5), sub_col);

    if let Some(label) = def.toggle {
        let y = PORT_Y + PORT_DY;
        let rect = v.r(w - PAD - 46.0, y - 8.0, w - PAD, y + 8.0);
        let on = p.toggled();
        if widgets::toggle(v.ui, v.painter, Id::unique(("toggle", id)), rect, label, on, accent, z).clicked() {
            p.set_toggle(!on);
        }
    }

    // Display.
    let rect = v.r(PAD, display_top(def), w - PAD, display_bottom(def));
    v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
    let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
    let screen = Screen {
        p: &painter,
        rect: rect.shrink2(vec2(6.0 * z, 5.0 * z)),
        accent,
        z,
    };
    match kind {
        ModuleKind::Drone => drone_display(&screen, p),
        ModuleKind::AudioIn => audio_in_display(&screen, p),
        ModuleKind::Random | ModuleKind::MidiCc | ModuleKind::MidiNote => trace_display(&screen, m),
        ModuleKind::Granular => granular_display(&screen, p),
        ModuleKind::Frakture | ModuleKind::Equalizer | ModuleKind::Magma | ModuleKind::Pops | ModuleKind::Tape | ModuleKind::Nullpunkt | ModuleKind::Aether | ModuleKind::Larsen | ModuleKind::Vesper | ModuleKind::Automa | ModuleKind::Colloid | ModuleKind::Looper => {}
        ModuleKind::Resonator => resonator_display(&screen, p),
        ModuleKind::Filter => filter_display(&screen, p),
        ModuleKind::Delay => delay_display(&screen, p, echo_seconds(p, v.tempo)),
        ModuleKind::PingPong => pingpong_display(&screen, p, echo_seconds(p, v.tempo)),
        ModuleKind::Reverb => reverb_display(&screen, p),
        ModuleKind::Sequencer => step_grid(v, id, &screen, p),
        ModuleKind::Arpeggiator | ModuleKind::MidiIn => keys_display(&screen, p),
        ModuleKind::Midicer => midicer_display(v, id, &screen, p),
    }
    if kind == ModuleKind::Midicer {
        midicer_controls(v, id, p, accent, display_bottom(def) + 6.0, w);
    }

    // Mode selector, and the MAP button for MIDI nodes.
    let mut button = MapButton::default();
    let mut modes_right = w - PAD;
    if def.map {
        let left = if def.modes.is_empty() { PAD } else { w - PAD - 76.0 };
        let [top, bottom] = selector_y(def);
        let rect = v.r(left, top, w - PAD, bottom);
        let label = if map.picking {
            "PICK…"
        } else if map.target.is_some() {
            "UNMAP"
        } else {
            "MAP"
        };
        let resp = widgets::toggle(v.ui, v.painter, Id::unique(("map", id)), rect, label, map.picking, accent, z)
            .on_hover_text(if map.target.is_some() {
                "Stop driving the mapped control"
            } else {
                "Drive a knob or fader directly: click MAP, then the control"
            });
        button = MapButton {
            clicked: resp.clicked(),
            rect: Some(rect),
        };
        modes_right = left - 4.0;
    }
    if !def.modes.is_empty() {
        let [top, bottom] = selector_y(def);
        let bar = v.r(PAD, top, modes_right, bottom);
        let picked = segmented(v, ("mode", id), bar, def.modes.len(), p.mode(), accent, |i, seg, col| {
            v.painter.text(seg.center(), Align2::CENTER_CENTER, def.modes[i], v.font(8.5), col);
            mode_name(kind, i)
        });
        if let Some(i) = picked {
            p.set_mode(i);
        }
    }

    // Knobs.
    let cols = columns(def);
    let top = knobs_top(def);
    for (k, spec) in def.params.iter().enumerate() {
        let dimmed =
            def.cv_depth.iter().any(|&(param, input)| param == k && !patched(input)) || knob_unused(kind, p, k);
        let cell = KnobCell {
            cx: PAD + COL_W * ((k % cols) as f32 + 0.5),
            row_y: top + ROW_H * (k / cols) as f32,
            label: spec.name,
            dimmed,
            accent,
            param: ParamRef::Module(k),
        };
        if let Some(value) = knob_cell(v, Id::unique(("mknob", id, k)), spec, p.get(k), cell) {
            p.set(k, value);
        }
    }
    button
}

/// Long name of a mode, for tooltips and readouts.
fn mode_name(kind: ModuleKind, i: usize) -> &'static str {
    match kind {
        ModuleKind::Drone => drone::CHORDS.get(i).map(|c| c.0),
        ModuleKind::Resonator => resonator::MATERIALS.get(i).map(|m| m.0),
        ModuleKind::Filter => filter::MODE_NAMES.get(i).copied(),
        ModuleKind::Granular => ["Forward", "Reverse", "Both directions"].get(i).copied(),
        ModuleKind::MidiCc => ["Unipolar 0…1", "Bipolar ±1"].get(i).copied(),
        ModuleKind::Sequencer => sequencer::SCALES.get(i).map(|s| s.0),
        ModuleKind::Midicer => midicer::MODE_NAMES.get(i).copied(),
        ModuleKind::Arpeggiator => arpeggiator::MODE_NAMES.get(i).copied(),
        _ => kind.def().modes.get(i).copied(),
    }
    .unwrap_or("")
}

/// The main readout line, shown on collapsed nodes.
pub fn summary(p: &ModuleParams, tempo: f32) -> String {
    readout(p.kind, p, tempo).0
}

/// A delay's echo time in seconds (TIME, or DIV at `tempo` with SYNC).
/// Delay and Ping Pong share their TIME, DIV and SYNC layout.
fn echo_seconds(p: &ModuleParams, tempo: f32) -> f32 {
    delay::delay_seconds(p.toggled(), p.get(delay::TIME), p.get(delay::DIV), 60.0 / tempo.max(1.0) as f64)
}

/// "1/8D · 375 ms" with SYNC, else "375 ms".
fn echo_label(p: &ModuleParams, tempo: f32) -> String {
    let t = format_seconds(echo_seconds(p, tempo));
    if p.toggled() { format!("{} · {t}", sync_division(p.get(delay::DIV)).0) } else { t }
}

/// "Tail ≈ 4.2 s": how long the repeats take to fall by 60 dB.
fn tail_label(seconds: f32, loop_gain: f32) -> String {
    if loop_gain >= 0.995 {
        "Endless".to_owned()
    } else if loop_gain < 0.01 {
        "Single echo".to_owned()
    } else {
        let repeats = 0.001f32.ln() / loop_gain.ln();
        format!("Tail ≈ {}", format_seconds(seconds * repeats))
    }
}

/// Knobs that do nothing in the current setting, drawn dimmed: a delay's
/// TIME while it is synced, and DIV while it is not.
fn knob_unused(kind: ModuleKind, p: &ModuleParams, k: usize) -> bool {
    matches!(kind, ModuleKind::Delay | ModuleKind::PingPong)
        && ((k == delay::TIME && p.toggled()) || (k == delay::DIV && !p.toggled()))
}

fn readout(kind: ModuleKind, p: &ModuleParams, tempo: f32) -> (String, String) {
    let mode = mode_name(kind, p.mode());
    match kind {
        ModuleKind::MidiCc => {
            let (channel, number) = midi_cc::address(p);
            let learning = p.toggled();
            let sub = if learning {
                "Move a controller…".to_owned()
            } else {
                let ch = if channel == 0 { "Omni".to_owned() } else { format!("Ch {channel}") };
                format!("{ch} · {:+.2}", p.viz[midi_cc::VALUE].get())
            };
            (format!("CC {number}"), sub)
        }
        ModuleKind::MidiNote => {
            let note = p.viz[midi_note::NOTE].get();
            let main = if note < 0.0 { "—".to_owned() } else { note_name(note_hz(note)) };
            let channel = midi_note::channel(p);
            let ch = if channel == 0 { "Omni".to_owned() } else { format!("Ch {channel}") };
            (main, format!("{ch} · play a key"))
        }
        ModuleKind::Midicer => {
            use midicer::data;
            let on = |i: usize| p.data[i].get() > 0.5;
            let note = p.viz[midicer::CURRENT].get();
            let main = if on(data::MUTE) {
                "Muted".to_owned()
            } else if note < 0.0 {
                "—".to_owned()
            } else {
                note_name(note_hz(note))
            };
            let (lo, hi) = (p.get(midicer::LOW).round() as i32, p.get(midicer::HIGH).round() as i32);
            let range = format!("{}–{}", note_label(lo.min(hi)), note_label(lo.max(hi)));
            let what = match p.mode() {
                midicer::SEQ => format!("{} · {range}", NOTE_VALUES[(p.get(midicer::NOTE).round().max(0.0) as usize).min(8)].0),
                midicer::SEQ_GATE => format!("gate in · {range}"),
                midicer::QUANT1 => format!("CV2 per beat · {range}"),
                _ => format!("CV2 while held · {range}"),
            };
            let lock = if on(data::LOCK) { " · locked" } else { "" };
            (main, format!("{what}{lock}"))
        }
        ModuleKind::Sequencer => {
            let step = p.viz[sequencer::STEP].get();
            let note = p.viz[sequencer::NOTE].get();
            let main = if step < 0.0 {
                "Stopped".to_owned()
            } else if note < 0.0 {
                "—".to_owned()
            } else {
                note_name(note_hz(note))
            };
            let root = crate::audio::params::NOTE_NAMES[p.get(sequencer::ROOT).round() as usize % 12];
            (main, format!("{root} {} · {}", mode.to_lowercase(), division(p.get(sequencer::RATE))))
        }
        ModuleKind::Arpeggiator => {
            let octaves = p.get(arpeggiator::OCTAVES).round();
            let oct = if octaves > 1.0 { format!(" · {octaves:.0} oct") } else { String::new() };
            (current_note(p), format!("{mode} · {}{oct}", division(p.get(arpeggiator::RATE))))
        }
        ModuleKind::MidiIn => {
            let channel = p.get(midi_in::CHANNEL).round();
            let ch = if channel < 1.0 { "Omni".to_owned() } else { format!("Ch {channel:.0}") };
            let t = p.get(midi_in::TRANSPOSE).round();
            let sub = if t == 0.0 { format!("{ch} · play a key") } else { format!("{ch} · {t:+.0} st") };
            (current_note(p), sub)
        }
        ModuleKind::AudioIn => {
            let (left, right) = audio_in::channels(p);
            let main = if left == right { format!("In {}", left + 1) } else { format!("In {}–{}", left + 1, right + 1) };
            let open = p.viz[audio_in::viz::CHANNELS].get() as usize;
            let sub = if open == 0 { "no input open".to_owned() } else { format!("{} · {:+.1} dB", mode.to_lowercase(), p.get(audio_in::GAIN)) };
            (main, sub)
        }
        ModuleKind::Drone => {
            let f = p.get(drone::PITCH);
            (format_hz(f), format!("{} · {mode}", note_name(f)))
        }
        ModuleKind::Random => {
            let style = if p.get(random::SMOOTH) < 0.05 { "stepped" } else { "smooth" };
            (
                format!("{:+.2}", p.viz[random::VALUE].get()),
                format!("{} · {style}", format_hz(p.get(random::RATE))),
            )
        }
        ModuleKind::Frakture => super::frakture::readout(p),
        ModuleKind::Equalizer => super::equalizer::readout(p),
        ModuleKind::Magma => super::magma::readout(p),
        ModuleKind::Pops => super::pops::readout(p),
        ModuleKind::Tape => super::tape::readout(p),
        ModuleKind::Nullpunkt => super::nullpunkt::readout(p),
        ModuleKind::Aether => super::aether::readout(p),
        ModuleKind::Larsen => super::larsen::readout(p),
        ModuleKind::Vesper => super::vesper::readout(p),
        ModuleKind::Looper => super::looper::readout(p),
        ModuleKind::Automa => super::automa::readout(p),
        ModuleKind::Colloid => super::colloid::readout(p),
        ModuleKind::Granular => {
            let n = p.viz[granular::viz::COUNT].get() as usize;
            let state = if p.toggled() { "Frozen" } else { "Recording" };
            (format!("{n} grains"), state.to_owned())
        }
        ModuleKind::Resonator => {
            let f = p.get(resonator::PITCH);
            (format_hz(f), format!("{} · {mode}", note_name(f)))
        }
        ModuleKind::Filter => (format_hz(p.get(filter::CUTOFF)), mode.to_owned()),
        ModuleKind::Delay => {
            let fb = delay::loop_gain(p.get(delay::FEEDBACK));
            (echo_label(p, tempo), tail_label(echo_seconds(p, tempo), fb))
        }
        ModuleKind::PingPong => {
            let fb = pingpong::loop_gain(p.get(pingpong::FEEDBACK));
            (echo_label(p, tempo), tail_label(echo_seconds(p, tempo), fb))
        }
        ModuleKind::Reverb => {
            let main = format!("RT {}", format_seconds(p.get(reverb::DECAY)));
            if p.toggled() {
                return (main, "Frozen".to_owned());
            }
            let size = p.get(reverb::SIZE);
            let room = if size < 0.35 {
                "Room"
            } else if size < 0.75 {
                "Hall"
            } else {
                "Cathedral"
            };
            (main, room.to_owned())
        }
    }
}

fn division(rate: f32) -> &'static str {
    DIVISIONS[(rate.round().max(0.0) as usize).min(DIVISIONS.len() - 1)].0
}

/// The note a MIDI generator is sounding, or "—".
fn current_note(p: &ModuleParams) -> String {
    let note = p.viz[steps::viz::CURRENT].get();
    if note < 0.0 { "—".to_owned() } else { note_name(note_hz(note)) }
}

// ---------------------------------------------------------------------------
// Displays
// ---------------------------------------------------------------------------

/// The display area of a module, already clipped.
struct Screen<'a> {
    p: &'a Painter,
    rect: Rect,
    accent: Color32,
    z: f32,
}

impl Screen<'_> {
    /// x for a frequency on a log axis from `lo` to `hi`.
    fn log_x(&self, f: f32, lo: f32, hi: f32) -> f32 {
        self.rect.left() + self.rect.width() * (f / lo).ln() / (hi / lo).ln()
    }

    /// Vertical line from the baseline, `amp` (0..1) of the height.
    fn partial(&self, x: f32, amp: f32, color: Color32) {
        let r = self.rect;
        self.p.line_segment(
            [pos2(x, r.bottom()), pos2(x, r.bottom() - r.height() * amp.clamp(0.0, 1.0))],
            Stroke::new(1.3 * self.z, color),
        );
    }

    /// Polyline with a soft fill down to the baseline.
    fn area(&self, pts: Vec<Pos2>) {
        let base = self.rect.bottom();
        let mut mesh = Mesh::default();
        for (i, pt) in pts.iter().enumerate() {
            mesh.vertices.push(Vertex { pos: *pt, uv: WHITE_UV, color: alpha(self.accent, 0.18) });
            mesh.vertices.push(Vertex { pos: pos2(pt.x, base), uv: WHITE_UV, color: alpha(self.accent, 0.02) });
            if i > 0 {
                let k = (i * 2) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        self.p.add(Shape::mesh(mesh));
        self.p.add(Shape::line(pts, Stroke::new(1.3 * self.z, self.accent)));
    }

    fn center_line(&self) {
        let r = self.rect;
        self.p.hline(r.x_range(), r.center().y, Stroke::new(1.0 * self.z, theme::GRID_MAJOR));
    }
}

/// Chord view: each voice's fundamental, with harmonics as TONE opens.
fn drone_display(s: &Screen, p: &ModuleParams) {
    let chord = drone::CHORDS[p.mode().min(drone::CHORDS.len() - 1)].1;
    let pitch = p.get(drone::PITCH);
    let tone = p.get(drone::TONE);
    let (lo, hi) = (20.0, 12_000.0);
    for semis in chord {
        let f = pitch * (semis / 12.0).exp2();
        for k in 1..=12 {
            let fk = f * k as f32;
            let amp = if k == 1 { 0.9 } else { 0.9 * tone / k as f32 };
            if fk > hi || amp < 0.02 {
                break;
            }
            let a = if k == 1 { 0.85 } else { 0.4 };
            s.partial(s.log_x(fk, lo, hi), amp, alpha(s.accent, a));
        }
    }
}

/// Audio In: a meter per side, labelled with its input; or, with no input
/// open, where to choose one.
fn audio_in_display(s: &Screen, p: &ModuleParams) {
    let r = s.rect;
    let z = s.z;
    let open = p.viz[audio_in::viz::CHANNELS].get() as usize;
    if open == 0 {
        let text = "No audio input · choose one in Settings (Cmd+,)";
        s.p.text(r.center(), Align2::CENTER_CENTER, text, theme::mono(8.5 * z), theme::TEXT_FAINT);
        return;
    }
    let (left, right) = audio_in::channels(p);
    let sides = [(left, p.viz[audio_in::viz::LEFT].get()), (right, p.viz[audio_in::viz::RIGHT].get())];
    let rows = if left == right { &sides[..1] } else { &sides[..] };
    let row_h = r.height() / rows.len() as f32;
    let label_w = 18.0 * z;
    // −60 … 0 dBFS across.
    let x_of = |level: f32| (1.0 + 20.0 * level.max(1e-6).log10() / 60.0).clamp(0.0, 1.0);
    for (k, &(channel, level)) in rows.iter().enumerate() {
        let y = r.top() + row_h * (k as f32 + 0.5);
        let missing = channel >= open;
        let col = if missing { theme::TEXT_FAINT } else { theme::TEXT_DIM };
        s.p.text(pos2(r.left() + label_w * 0.5, y), Align2::CENTER_CENTER, format!("{}", channel + 1), theme::mono(9.0 * z), col);
        let bar = Rect::from_min_max(pos2(r.left() + label_w, y - 3.0 * z), pos2(r.right(), y + 3.0 * z));
        s.p.rect_filled(bar, 1.5 * z, alpha(theme::TEXT, 0.06));
        if missing {
            continue;
        }
        let w = bar.width() * x_of(level);
        let fill = if level >= 0.89 { theme::METER_RED } else { s.accent };
        s.p.rect_filled(Rect::from_min_size(bar.min, vec2(w, bar.height())), 1.5 * z, fill);
        // −18 and −6 dBFS marks.
        for db in [-18.0, -6.0] {
            let x = bar.left() + bar.width() * (1.0 + db / 60.0);
            s.p.vline(x, bar.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.18)));
        }
    }
}

/// Recent output values, newest on the right.
fn trace_display(s: &Screen, m: &ModuleNode) {
    s.center_line();
    let r = s.rect;
    let n = m.trace.len();
    if n < 2 {
        return;
    }
    let step = r.width() / (crate::graph::TRACE_LEN - 1) as f32;
    let x0 = r.right() - step * (n - 1) as f32;
    let pts: Vec<Pos2> = m
        .trace
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(x0 + step * i as f32, r.center().y - v.clamp(-1.0, 1.0) * r.height() * 0.5))
        .collect();
    let last = *pts.last().expect("at least two points");
    s.p.add(Shape::line(pts, Stroke::new(1.3 * s.z, s.accent)));
    s.p.circle_filled(last, 2.5 * s.z, s.accent);
}

/// The recording loop (newest on the right), the POS/SIZE window and the
/// grains currently playing.
fn granular_display(s: &Screen, p: &ModuleParams) {
    use granular::viz;
    let r = s.rect;
    let view = granular::MAX_POSITION + 1.0;
    let x_of = |age: f32| r.right() - r.width() * age / view;
    let loop_s = p.viz[viz::LOOP_SECONDS].get().max(1.0);
    let bin_s = loop_s / granular::OVERVIEW as f32;
    let head = p.viz[viz::HEAD].get() * granular::OVERVIEW as f32;

    let win = Rect::from_x_y_ranges(
        x_of(p.get(granular::POSITION) + p.get(granular::SIZE))..=x_of(p.get(granular::POSITION)),
        r.y_range(),
    );
    s.p.rect_filled(win, 0.0, alpha(s.accent, 0.10));

    let bar = (r.width() * bin_s / view * 0.7).max(1.0);
    let half = r.height() * 0.5;
    for b in 0..granular::OVERVIEW {
        let age = (head - (b as f32 + 0.5)).rem_euclid(granular::OVERVIEW as f32) * bin_s;
        if age > view {
            continue;
        }
        let h = p.viz[viz::OVERVIEW_AT + b].get().min(1.0).sqrt() * half;
        if h < 0.5 {
            continue;
        }
        let x = x_of(age);
        s.p.line_segment(
            [pos2(x, r.center().y - h), pos2(x, r.center().y + h)],
            Stroke::new(bar, alpha(theme::TEXT_FAINT, 0.7)),
        );
    }

    for k in 0..granular::VIZ_GRAINS {
        let age = p.viz[viz::GRAIN_AGE + k].get();
        if age < 0.0 {
            continue;
        }
        let amp = p.viz[viz::GRAIN_AMP + k].get();
        let x = x_of(age);
        s.p.vline(x, r.y_range(), Stroke::new(1.4 * s.z, alpha(s.accent, 0.2 + 0.8 * amp)));
    }
}

/// Partials of the chosen material, sized by BRIGHT.
fn resonator_display(s: &Screen, p: &ModuleParams) {
    let ratios = resonator::MATERIALS[p.mode().min(resonator::MATERIALS.len() - 1)].1;
    let pitch = p.get(resonator::PITCH);
    let bright = p.get(resonator::BRIGHT);
    let (lo, hi) = (20.0, 16_000.0);
    for (k, ratio) in ratios.iter().enumerate() {
        let f = pitch * ratio;
        if f > hi {
            break;
        }
        let (amp, decay) = resonator::mode_shape(k, bright);
        s.partial(s.log_x(f, lo, hi), amp * 0.9, alpha(s.accent, 0.35 + 0.6 * decay));
    }
}

/// Magnitude response, −30 to +24 dB.
fn filter_display(s: &Screen, p: &ModuleParams) {
    let r = s.rect;
    let fc = p.get(filter::CUTOFF);
    let q = filter::q_of(p.get(filter::RESO));
    let mode = p.mode();
    let (db_lo, db_hi) = (-30.0f32, 24.0f32);
    let y_of = |db: f32| r.bottom() - r.height() * ((db - db_lo) / (db_hi - db_lo)).clamp(0.0, 1.0);
    s.p.hline(r.x_range(), y_of(0.0), Stroke::new(1.0 * s.z, theme::GRID_MAJOR));

    let n = (r.width() / 3.0).max(16.0) as usize;
    let pts = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let w = 20.0 * 1000f32.powf(t) / fc;
            let den = (1.0 - w * w).powi(2) + (w / q).powi(2);
            let num = match mode {
                filter::BANDPASS => (w / q).powi(2),
                filter::HIGHPASS => w.powi(4),
                filter::NOTCH => (1.0 - w * w).powi(2),
                _ => 1.0,
            };
            let db = 10.0 * (num / den).max(1e-9).log10();
            pos2(r.left() + r.width() * t, y_of(db))
        })
        .collect();
    s.area(pts);
}

/// Echoes over 4 seconds: left channel up, right channel down.
fn delay_display(s: &Screen, p: &ModuleParams, t: f32) {
    s.center_line();
    let r = s.rect;
    let span = 4.0;
    let fb = delay::loop_gain(p.get(delay::FEEDBACK)).min(1.0);
    let ping = p.get(delay::PING);
    let half = r.height() * 0.5;
    let stroke = |a: f32| Stroke::new(2.0 * s.z, alpha(s.accent, 0.3 + 0.7 * a.min(1.0)));
    // Energy per channel for a centred input.
    let mut e = [1.0f32, 1.0 - ping];
    for k in 1..=64 {
        let x = r.left() + r.width() * k as f32 * t / span;
        if x > r.right() || e[0].max(e[1]) < 0.02 {
            break;
        }
        let (l, rr) = (e[0].min(1.0), e[1].min(1.0));
        s.p.line_segment([pos2(x, r.center().y), pos2(x, r.center().y - l * half)], stroke(l));
        s.p.line_segment([pos2(x, r.center().y), pos2(x, r.center().y + rr * half)], stroke(rr));
        e = [
            fb * ((1.0 - ping) * e[0] + ping * e[1]),
            fb * ((1.0 - ping) * e[1] + ping * e[0]),
        ];
    }
}

/// Ping Pong's echoes over 4 seconds, alternating sides: left up, right
/// down, the first on the side the mode picks; WIDTH pulls them together.
fn pingpong_display(s: &Screen, p: &ModuleParams, t: f32) {
    s.center_line();
    let r = s.rect;
    let span = 4.0;
    let fb = pingpong::loop_gain(p.get(pingpong::FEEDBACK));
    let width = p.get(pingpong::WIDTH);
    let first_right = p.mode() == pingpong::RIGHT_FIRST;
    let half = r.height() * 0.5;
    let stroke = |a: f32| Stroke::new(2.0 * s.z, alpha(s.accent, 0.3 + 0.7 * a.min(1.0)));
    let mut level = 1.0f32;
    for k in 1..=64 {
        let x = r.left() + r.width() * k as f32 * t / span;
        if x > r.right() || level < 0.02 {
            break;
        }
        // Odd echoes on the first side; the other side gets what WIDTH leaks.
        let right = (k % 2 == 1) == first_right;
        let (main, leak) = (level * (0.5 + 0.5 * width), level * (0.5 - 0.5 * width));
        let (l, rr) = if right { (leak, main) } else { (main, leak) };
        s.p.line_segment([pos2(x, r.center().y), pos2(x, r.center().y - l * half)], stroke(l));
        s.p.line_segment([pos2(x, r.center().y), pos2(x, r.center().y + rr * half)], stroke(rr));
        level *= fb;
    }
}

/// Decay envelope over 12 seconds (flat when frozen).
fn reverb_display(s: &Screen, p: &ModuleParams) {
    let r = s.rect;
    let span = 12.0;
    let decay = p.get(reverb::DECAY);
    let frozen = p.toggled();
    let n = (r.width() / 3.0).max(16.0) as usize;
    let pts = (0..=n)
        .map(|i| {
            let t = span * i as f32 / n as f32;
            let env = if frozen { 1.0 } else { 10f32.powf(-3.0 * t / decay) };
            // Envelope in dB over a 60 dB range, so long tails read clearly.
            let level = (1.0 + (20.0 * env.max(1e-6).log10()) / 60.0).clamp(0.0, 1.0);
            pos2(r.left() + r.width() * i as f32 / n as f32, r.bottom() - r.height() * 0.9 * level)
        })
        .collect();
    s.area(pts);
}

/// The sequencer's steps: one column each, the lit cell at its degree.
/// Click a cell to move the step there (or, on its lit cell, to switch it
/// on or off); drag across to draw a melody.
fn step_grid(v: &NodeView, id: NodeId, s: &Screen, p: &ModuleParams) {
    use sequencer::{DEGREE_MAX, LEN};
    let r = s.rect;
    let z = s.z;
    let col_w = r.width() / LEN as f32;
    let cell_h = r.height() / (DEGREE_MAX + 1) as f32;
    let cell_at = |pos: Pos2| {
        let step = (((pos.x - r.left()) / col_w).max(0.0) as usize).min(LEN - 1);
        let degree = (((r.bottom() - pos.y) / cell_h).max(0.0) as usize).min(DEGREE_MAX);
        (step, degree)
    };
    let resp = v
        .ui
        .interact(r, Id::unique(("steps", id)), Sense::CLICK | Sense::DRAG)
        .on_hover_cursor(CursorIcon::Crosshair);
    if let Some(pos) = resp.interact_pointer_pos() {
        let (step, degree) = cell_at(pos);
        if resp.clicked() && degree == sequencer::degree(p, step) {
            sequencer::set_on(p, step, !sequencer::is_on(p, step));
        } else if resp.clicked() || resp.dragged() {
            sequencer::set_degree(p, step, degree);
            sequencer::set_on(p, step, true);
        }
    }

    // Rows of the root note, one per octave of the scale.
    let scale_len = sequencer::SCALES[p.mode().min(sequencer::SCALES.len() - 1)].1.len();
    for degree in (0..=DEGREE_MAX).step_by(scale_len) {
        let y = r.bottom() - cell_h * (degree as f32 + 0.5);
        s.p.hline(r.x_range(), y, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.18)));
    }
    let len = sequencer::length(p);
    let playing = p.viz[sequencer::STEP].get();
    for i in 0..LEN {
        let x0 = r.left() + col_w * i as f32;
        if i % 4 == 0 && i > 0 {
            s.p.vline(x0, r.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
        }
        let column = Rect::from_x_y_ranges(x0..=x0 + col_w, r.y_range());
        let now = playing >= 0.0 && playing.round() as usize == i;
        if now {
            s.p.rect_filled(column, 0.0, alpha(theme::TEXT, 0.07));
        }
        let degree = sequencer::degree(p, i);
        let cell = Rect::from_x_y_ranges(
            x0 + 1.5 * z..=x0 + col_w - 1.5 * z,
            r.bottom() - cell_h * (degree + 1) as f32 + 0.5 * z..=r.bottom() - cell_h * degree as f32 - 0.5 * z,
        );
        let active = if i < len { 1.0 } else { 0.3 };
        if sequencer::is_on(p, i) {
            let stem = Rect::from_x_y_ranges(cell.x_range(), cell.bottom()..=r.bottom());
            s.p.rect_filled(stem, 0.0, alpha(s.accent, 0.12 * active));
            let color = if now { theme::mix(s.accent, Color32::WHITE, 0.45) } else { alpha(s.accent, 0.9 * active) };
            s.p.rect_filled(cell, 1.0 * z, color);
        } else {
            s.p.rect_stroke(cell, 1.0 * z, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.8 * active)), StrokeKind::Inside);
        }
    }

    // The note under the pointer.
    if let Some(pos) = resp.hover_pos() {
        let (step, degree) = cell_at(pos);
        let label = format!("{} · {}", step + 1, note_name(note_hz(sequencer::note_of(p, degree) as f32)));
        s.p.text(
            r.left_top() + vec2(2.0, 1.0) * z,
            Align2::LEFT_TOP,
            label,
            theme::mono(9.0 * z),
            theme::TEXT,
        );
    }
}

/// A keyboard with the held notes lit and the one sounding brightest.
fn keys_display(s: &Screen, p: &ModuleParams) {
    use steps::viz;
    let r = s.rect;
    let z = s.z;
    let count = (p.viz[viz::COUNT].get() as usize).min(viz::SHOWN);
    let held: Vec<i32> = p.viz[viz::NOTES..viz::NOTES + count].iter().map(|n| n.get() as i32).collect();
    let current = p.viz[viz::CURRENT].get();
    let current = (current >= 0.0).then_some(current as i32);
    // Whole octaves around the notes: three at least, five at most.
    let lowest = held.iter().chain(&current).copied().min().unwrap_or(48);
    let highest = held.iter().chain(&current).copied().max().unwrap_or(48);
    let lo = lowest.div_euclid(12) * 12;
    let hi = (lo + 36).max((highest.div_euclid(12) + 1) * 12).min(lo + 60);
    let is_black = |n: i32| matches!(n.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
    let whites = (lo..hi).filter(|&n| !is_black(n)).count().max(1);
    let key_w = r.width() / whites as f32;
    let fill = |n: i32, base: Color32| {
        if current == Some(n) {
            s.accent
        } else if held.contains(&n) {
            alpha(s.accent, 0.45)
        } else {
            base
        }
    };
    let mut x = r.left();
    for n in lo..hi {
        if is_black(n) {
            continue;
        }
        let key = Rect::from_x_y_ranges(x + 0.5 * z..=x + key_w - 0.5 * z, r.y_range());
        s.p.rect_filled(key, 1.0 * z, fill(n, theme::CONTROL_HI));
        if n % 12 == 0 && key_w > 6.0 * z {
            let octave = n / 12 - 1;
            s.p.text(
                pos2(key.center().x, key.bottom() - 1.0 * z),
                Align2::CENTER_BOTTOM,
                format!("{octave}"),
                theme::font(7.0 * z),
                theme::INSET,
            );
        }
        x += key_w;
    }
    let mut x = r.left();
    for n in lo..hi {
        if !is_black(n) {
            x += key_w;
            continue;
        }
        let key = Rect::from_center_size(pos2(x, r.top() + r.height() * 0.3), vec2(key_w * 0.6, r.height() * 0.6));
        s.p.rect_filled(key, 1.0 * z, fill(n, theme::INSET));
    }
}

/// MIDIcer's display: a probability fader for each semitone (click or drag
/// to set it; down to the bottom switches it off), lit while its note
/// plays; under it the pattern's sixteen steps, FIRST…LAST bright and the
/// step playing lit.
fn midicer_display(v: &NodeView, id: NodeId, s: &Screen, p: &ModuleParams) {
    use midicer::data;
    let r = s.rect;
    let z = s.z;
    let strip = Rect::from_x_y_ranges(r.x_range(), r.bottom() - 6.0 * z..=r.bottom());
    let faders = Rect::from_x_y_ranges(r.x_range(), r.top()..=strip.top() - 5.0 * z);
    let label_h = 11.0 * z;
    let bars = Rect::from_x_y_ranges(faders.x_range(), faders.top()..=faders.bottom() - label_h);
    let col_w = faders.width() / 12.0;

    let resp = v
        .ui
        .interact(faders, Id::unique(("midicer_faders", id)), Sense::CLICK | Sense::DRAG)
        .on_hover_cursor(CursorIcon::ResizeVertical);
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged())
    {
        let i = (((pos.x - faders.left()) / col_w).max(0.0) as usize).min(11);
        let value = ((bars.bottom() - pos.y) / bars.height()).clamp(0.0, 1.0);
        p.data[data::WEIGHTS + i].set(if value < 0.04 { 0.0 } else { value });
    }

    let current = p.viz[midicer::CURRENT].get();
    let lit = (current >= 0.0).then_some(current as usize % 12);
    for (i, name) in NOTE_NAMES.iter().enumerate() {
        let x0 = faders.left() + col_w * i as f32;
        let column = Rect::from_x_y_ranges(x0 + 1.5 * z..=x0 + col_w - 1.5 * z, bars.y_range());
        let black = matches!(i, 1 | 3 | 6 | 8 | 10);
        s.p.rect_filled(column, 1.5 * z, if black { theme::GRID_MINOR } else { alpha(theme::CONTROL, 0.6) });
        let w = p.data[data::WEIGHTS + i].get().clamp(0.0, 1.0);
        if w > 0.0 {
            let bar = Rect::from_x_y_ranges(column.x_range(), column.bottom() - column.height() * w..=column.bottom());
            let color = if lit == Some(i) { theme::mix(s.accent, Color32::WHITE, 0.45) } else { alpha(s.accent, 0.35 + 0.5 * w) };
            s.p.rect_filled(bar, 1.5 * z, color);
        }
        let label_col = if lit == Some(i) { theme::TEXT } else if w > 0.0 { theme::TEXT_DIM } else { theme::TEXT_FAINT };
        let font = theme::font(8.0 * z);
        if font.size >= super::nodes::MIN_TEXT {
            s.p.text(pos2(column.center().x, faders.bottom()), Align2::CENTER_BOTTOM, *name, font, label_col);
        }
    }

    // The pattern: sixteen sixteenths, FIRST…LAST in use.
    let first = (p.get(midicer::FIRST).round() as usize).clamp(1, 16) - 1;
    let last = (p.get(midicer::LAST).round() as usize).clamp(first + 1, 16) - 1;
    let step = p.viz[midicer::STEP].get();
    let cell = strip.width() / 16.0;
    for i in 0..16 {
        let x0 = strip.left() + cell * i as f32;
        let c = Rect::from_x_y_ranges(x0 + 1.0 * z..=x0 + cell - 1.0 * z, strip.y_range());
        let color = if step >= 0.0 && step as usize == i {
            s.accent
        } else if (first..=last).contains(&i) {
            if i % 4 == 0 { theme::TEXT_FAINT } else { theme::CONTROL_HI }
        } else {
            theme::GRID_MINOR
        };
        s.p.rect_filled(c, 1.0 * z, color);
    }
}

/// MIDIcer's buttons: DICE and LIVE for rhythm and melody, LOCK, MUTE,
/// RESTART, and SETUP for the settings meloDICER keeps in its edit menu.
fn midicer_controls(v: &NodeView, id: NodeId, p: &ModuleParams, accent: Color32, top: f32, w: f32) {
    use midicer::data;
    let z = v.z;
    let on = |i: usize| p.data[i].get() > 0.5;
    let flip = |i: usize| p.data[i].set(if on(i) { 0.0 } else { 1.0 });
    let gap = 4.0;
    let bw = (w - 2.0 * PAD - 3.0 * gap) / 4.0;
    let slot = |row: usize, k: usize| {
        let x = PAD + k as f32 * (bw + gap);
        let y = top + row as f32 * 24.0;
        v.r(x, y, x + bw, y + 20.0)
    };
    let button = |row: usize, k: usize, label: &str, lit: bool, tip: &str| {
        widgets::toggle(v.ui, v.painter, Id::unique(("midicer", id, label)), slot(row, k), label, lit, accent, z)
            .on_hover_text(tip)
    };
    let now = v.ui.input(|i| i.time);
    for (k, (dice, seed, name)) in [(data::DICE_R, data::SEED_R, "R"), (data::DICE_M, data::SEED_M, "M")].into_iter().enumerate() {
        let what = if name == "R" { "rhythm" } else { "melody" };
        let d = button(0, 2 * k, &format!("DICE {name}"), on(dice), &format!(
            "The {what} repeats: click for new random values\n(in LIVE, click to come back to the kept ones)"
        ));
        if d.clicked() {
            if on(dice) {
                p.data[seed].set(midicer::new_seed((now * 1e6) as u32 ^ seed as u32));
            } else {
                p.data[dice].set(1.0);
            }
        }
        let l = button(0, 2 * k + 1, &format!("LIVE {name}"), !on(dice), &format!(
            "New random {what} values all the time: no repeating pattern"
        ));
        if l.clicked() {
            p.data[dice].set(0.0);
        }
    }
    if button(1, 0, "LOCK", on(data::LOCK), "Keep playing with the current settings while you change them;\nclick again to apply").clicked() {
        flip(data::LOCK);
    }
    if button(1, 1, "MUTE", on(data::MUTE), "Silence the notes; the pattern keeps running").clicked() {
        flip(data::MUTE);
    }
    if button(1, 2, "RESTART", false, "Start the pattern again from FIRST on the next beat").clicked() {
        p.data[data::RESTART].set(p.data[data::RESTART].get() + 1.0);
    }
    let setup = button(1, 3, "SETUP", false, "Note variation, mute, CV routing, MIDI commands");
    egui::Popup::menu(&setup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| midicer_setup(ui, p));
}

/// The settings meloDICER keeps in its edit menu.
fn midicer_setup(ui: &mut egui::Ui, p: &ModuleParams) {
    use midicer::data;
    ui.set_width(270.0);
    let heading = |ui: &mut egui::Ui, text: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).size(9.5).color(theme::TEXT_FAINT));
    };
    let check = |ui: &mut egui::Ui, i: usize, label: &str| {
        let mut on = p.data[i].get() > 0.5;
        if ui.checkbox(&mut on, label).changed() {
            p.data[i].set(if on { 1.0 } else { 0.0 });
        }
    };
    heading(ui, "VARIATION MAY PICK");
    check(ui, data::TRIPLETS, "Triplets");
    check(ui, data::THIRTY_SECOND, "1/32 notes");
    heading(ui, "MUTE");
    check(ui, data::UNMUTE_RESTART, "Unmuting restarts the pattern on the next beat");
    heading(ui, "CV1 · PITCH");
    let mut pitch = p.data[data::CV1_PITCH].get().round() as i32;
    let before = pitch;
    ui.horizontal(|ui| {
        ui.radio_value(&mut pitch, 0, "Off");
        ui.radio_value(&mut pitch, 1, "Bend").on_hover_text("Pitch bend, unquantized (like ADD SEQ)");
        ui.radio_value(&mut pitch, 2, "Transpose").on_hover_text("Moves the notes ±24 semitones (like TRANS SEQ)");
    });
    if pitch != before {
        p.data[data::CV1_PITCH].set(pitch as f32);
    }
    check(ui, data::CV1_LOW, "Moves LOW (±24 semitones)");
    check(ui, data::CV1_HIGH, "Moves HIGH (±24 semitones)");
    heading(ui, "CV2 · RHYTHM (ADDED TO THE KNOBS)");
    check(ui, data::CV2_NOTE, "NOTE value");
    check(ui, data::CV2_VAR, "VARIATION");
    check(ui, data::CV2_LEGATO, "LEGATO");
    check(ui, data::CV2_REST, "REST");
    heading(ui, "MIDI IN · COMMANDS IN SEQ MODE");
    ui.label(
        egui::RichText::new("C restart · D dice rhythm · E dice melody\nF / G rhythm / melody DICE ⇄ LIVE · A mute while held")
            .size(10.5)
            .color(theme::TEXT_DIM),
    );
}
