//! Panels for the ambient modules. Every module shares one layout: ports
//! and a two-line readout on top, a live display, an optional mode selector,
//! then a grid of knobs. Only the readout and the display are per kind.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, Id, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{KnobCell, NodeView, knob_cell, note_name, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::midi::note_hz;
use crate::audio::modules::{
    ModuleDef, ModuleKind, ModuleParams, delay, drone, filter, granular, midi_cc, midi_note, random, resonator,
    reverb,
};
use crate::audio::params::{format_hz, format_seconds};
use crate::graph::{ModuleNode, ParamRef};

const PAD: f32 = 12.0;
const COL_W: f32 = 64.0;
const PORT_Y: f32 = 42.0;
const PORT_DY: f32 = 18.0;
const DISPLAY_Y: [f32; 2] = [72.0, 114.0];
const SELECTOR_Y: [f32; 2] = [120.0, 140.0];
const ROW_H: f32 = 58.0;

fn columns(def: &ModuleDef) -> usize {
    match def.params.len() {
        4 => 4,
        n if n >= 7 => 4,
        _ => 3,
    }
}

/// Node-local y of the first knob row's labels.
fn knobs_top(def: &ModuleDef) -> f32 {
    if def.modes.is_empty() && !def.map { 132.0 } else { 156.0 }
}

pub fn size(def: &ModuleDef) -> Vec2 {
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

pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &ModuleNode,
    inputs: &[bool],
    outputs: &[bool],
    map: MapState,
) -> MapButton {
    let p = &m.params;
    let kind = p.kind;
    let def = kind.def();
    let z = v.z;
    let w = size(def).x;
    let patched = |i: usize| inputs.get(i).copied().unwrap_or(false);
    let label_col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };

    // Ports.
    for (i, label) in def.inputs.iter().enumerate() {
        v.text(PAD, PORT_Y + PORT_DY * i as f32, Align2::LEFT_CENTER, label, v.font(9.0), label_col(patched(i)));
    }
    for (i, label) in def.outputs.iter().enumerate() {
        let on = outputs.get(i).copied().unwrap_or(false);
        v.text(w - PAD, PORT_Y + PORT_DY * i as f32, Align2::RIGHT_CENTER, label, v.font(9.0), label_col(on));
    }

    // Readout.
    let (main, sub) = readout(kind, p);
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
    let rect = v.r(PAD, DISPLAY_Y[0], w - PAD, DISPLAY_Y[1]);
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
        ModuleKind::Random | ModuleKind::MidiCc | ModuleKind::MidiNote => trace_display(&screen, m),
        ModuleKind::Granular => granular_display(&screen, p),
        ModuleKind::Resonator => resonator_display(&screen, p),
        ModuleKind::Filter => filter_display(&screen, p),
        ModuleKind::Delay => delay_display(&screen, p),
        ModuleKind::Reverb => reverb_display(&screen, p),
    }

    // Mode selector, and the MAP button for MIDI nodes.
    let mut button = MapButton::default();
    let mut modes_right = w - PAD;
    if def.map {
        let left = if def.modes.is_empty() { PAD } else { w - PAD - 76.0 };
        let rect = v.r(left, SELECTOR_Y[0], w - PAD, SELECTOR_Y[1]);
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
        let bar = v.r(PAD, SELECTOR_Y[0], modes_right, SELECTOR_Y[1]);
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
        let dimmed = def.cv_depth.iter().any(|&(param, input)| param == k && !patched(input));
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
        _ => kind.def().modes.get(i).copied(),
    }
    .unwrap_or("")
}

/// The main readout line, shown on collapsed nodes.
pub fn summary(p: &ModuleParams) -> String {
    readout(p.kind, p).0
}

fn readout(kind: ModuleKind, p: &ModuleParams) -> (String, String) {
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
            let t = p.get(delay::TIME);
            let fb = delay::loop_gain(p.get(delay::FEEDBACK));
            let tail = if fb >= 0.995 {
                "Endless".to_owned()
            } else if fb < 0.01 {
                "Single echo".to_owned()
            } else {
                // Time for the repeats to fall by 60 dB.
                let repeats = 0.001f32.ln() / fb.ln();
                format!("Tail ≈ {}", format_seconds(t * repeats))
            };
            (format_seconds(t), tail)
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
fn delay_display(s: &Screen, p: &ModuleParams) {
    s.center_line();
    let r = s.rect;
    let span = 4.0;
    let t = p.get(delay::TIME);
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
