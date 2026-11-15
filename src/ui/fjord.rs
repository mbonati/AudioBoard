//! Panel of Fjord, flat and graphic, laid out like the Nord Drum's panel
//! grown into a node.
//!
//! Along the top, always there: the programs (the current one's number,
//! which can be dragged like the hardware's dial, its name, STORE and
//! Memory Protect, and the whole bank as a map of 99 cells, outlined in
//! each program's style colour: click one to load it, or, after STORE, to
//! store to it); the four channel keys with their trigger lights (green on
//! a hit, red for a hard one, longer for the hardest), TRIG and its
//! velocity, COPY, PASTE, INIT; SOLO and MASTER; and ALEA, the dice: a
//! die that shows the last throw, CHANCE, throws for the sounds, the steps
//! or both, and BACK.
//!
//! Under it two pages, and the node takes the size of the one shown.
//! SOUND: the edited channel's strike, drawn as the outlines of its tone,
//! noise and click with the bend's pitch over them (a line runs through it
//! on each hit); its sections, NOISE, TONE, CLICK and MIXER, the red
//! parameters (filter type, sweep times, gates, punch, bend time) in red
//! as on the hardware, the choices (waveform, filter type, click type,
//! gates) as rows of keys; and the tone's envelopes, AMP ENV and FILTER
//! ENV, each ADSR drawn beside its knobs. SEQUENCER · MIDI: the step lanes,
//! one per channel, then SYSTEM: the MIDI and Trig input settings.

use std::sync::Arc;

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{MIN_TEXT, NodeView};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::fjord::alea::{self, Before};
use crate::audio::modules::fjord::bank::{FACTORY, PROGRAMS};
use crate::audio::modules::fjord::trigger::{self, CURVES, TYPES};
use crate::audio::modules::fjord::voice::{self, CLICK_FAMILIES, GATES, NOISE_Q, Sound, WAVES};
use crate::audio::modules::fjord::{self as fj, CHANNELS, MAX_LEN, RANGES, ch, data, knob, viz};
use crate::audio::params::{DIVISIONS, note_label};
use crate::graph::{ModuleNode, ParamRef};

/// The panel's height on each page: SOUND, SEQUENCER · MIDI (the node
/// resizes with the page).
const HEIGHTS: [f32; 2] = [470.0, 394.0];
pub const SIZE: Vec2 = vec2(900.0, HEIGHTS[0]);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;

/// The pages' tabs, under the top.
const TABS_Y: [f32; 2] = [130.0, 148.0];

// The SOUND page.
/// The display of the edited channel's strike.
const DISPLAY: [f32; 2] = [156.0, 248.0];
/// The sections: their titles, the row of keys, the knobs, the red row.
const EDIT_Y: f32 = 260.0;
const SEL: [f32; 2] = [270.0, 286.0];
const KNOB_Y: f32 = 294.0;
const RED_Y: f32 = 342.0;
/// The tone's envelopes (amplitude and filter ADSRs).
const ENV_Y: f32 = 394.0;
const ENV_KNOBS: f32 = 406.0;
const AMP_X: [f32; 2] = [PAD, 428.0];
const FLT_X: [f32; 2] = [446.0, W - PAD];

// The SEQUENCER · MIDI page.
const SEQ_Y: f32 = 164.0;
const LANES_TOP: f32 = 180.0;
const LANE_H: f32 = 34.0;
const SYS_Y: f32 = 332.0;
const SYS_ROWS: [f32; 2] = [342.0, 364.0];

/// The ALEA block, beside MASTER.
const ALEA_X: [f32; 2] = [676.0, 830.0];
/// How long the die tumbles after a throw.
const TUMBLE: f64 = 0.45;

/// The sections' columns.
const NOISE_X: [f32; 2] = [PAD, 240.0];
const TONE_X: [f32; 2] = [252.0, 576.0];
const CLICK_X: [f32; 2] = [588.0, 724.0];
const MIXER_X: [f32; 2] = [736.0, W - PAD];

/// Tone, noise, click: their colours (outlines, keys, knobs).
const TONE_COL: Color32 = Color32::from_rgb(255, 196, 87);
const NOISE_COL: Color32 = Color32::from_rgb(72, 219, 251);
const CLICK_COL: Color32 = Color32::from_rgb(255, 107, 157);
/// The red parameters' labels (and STORE).
const RED: Color32 = Color32::from_rgb(236, 84, 70);
const GREEN: Color32 = Color32::from_rgb(110, 220, 120);

/// How long a trigger light stays lit, a hard hit's red, the hardest's.
const FLASH: f64 = 0.12;
const RED_SHORT: f64 = 0.1;
const RED_LONG: f64 = 0.45;
/// How long a refused or done Store is reported.
const PROTECTED: f64 = 1.5;

/// The program block and its bank map: 20 columns (a gap between the two
/// halves, to count by tens), five rows.
const PROG_X: [f32; 2] = [70.0, 330.0];
const MAP_Y: [f32; 2] = [64.0, 120.0];
const BANK_COLS: usize = 20;
const DECADE_GAP: f32 = 4.0;
/// The channel keys and the SOLO / MASTER block.
const KEYS_X: f32 = 342.0;
const SOLO_X: f32 = 628.0;
/// The factory styles' colours (programs of your own are white, empty
/// locations faint).
const STYLES: [(&str, Color32); 4] = [
    ("Real", Color32::from_rgb(255, 196, 87)),
    ("Retro", Color32::from_rgb(72, 219, 251)),
    ("Ethno", Color32::from_rgb(126, 231, 135)),
    ("FX", Color32::from_rgb(255, 107, 157)),
];

/// The waveform preview's rate and columns.
const PREVIEW_SR: f32 = 24_000.0;
const COLUMNS: usize = 300;
const PREVIEW_MAX: f32 = 4.0;
/// How long the preview holds its note (for the ADSRs).
const PREVIEW_GATE: f32 = 0.4;

fn tip(k: usize) -> &'static str {
    match k {
        knob::N_COLOR => "Colour of the noise: below 0 more bass, above 0 less",
        knob::N_FILTER => "Noise filter, a low-pass: lower is duller",
        knob::N_TYPE => "Filter type (red): seven types, each with more resonance",
        knob::N_SWEEP => "How far the filter opens at the strike (velocity sensitive)",
        knob::N_TIME | knob::T_TIME => "Sweep time (red), a share of the decay: 9 lasts as long as the decay",
        knob::N_DECAY => "Length of the noise",
        knob::N_GATE | knob::T_GATE | knob::C_GATE => "Gate (red): n decays gradually, S holds with a soft knee, G is gated (held, then cut)",
        knob::T_WAVE => "The waveform: An analog, b drum-like, c percussive",
        knob::T_FILTER => "Tone filter, a low-pass: 99 is fully open",
        knob::T_PUNCH => "Punch (red): a short, velocity-controlled attack",
        knob::T_SWEEP => "How far the tone filter opens at the strike (velocity sensitive)",
        knob::T_DECAY => "Length of the tone",
        knob::PITCH => "Pitch in half semitones, as MIDI note numbers: 69.0 is A 440 Hz",
        knob::BEND => "Bend: positive starts above the pitch and falls to it, negative rises to it (velocity sensitive)",
        knob::BEND_TIME => "Bend time (red), a share of the tone's decay: 9 lasts as long as the decay",
        knob::C_TYPE => "Click type: n noise, c noise with an attack, t pitched",
        knob::C_DECAY => "Length of the click",
        knob::TONE | knob::NOISE | knob::CLICK => "Level of this component in the channel",
        knob::AMP_ENV => "What shapes the tone's level: its DECAY and GATE (the Nord Drum's), or this ADSR, held as long as the note",
        knob::AMP_A | knob::FLT_A => "Attack: how long it takes to rise",
        knob::AMP_D | knob::FLT_D => "Decay: how long it takes to fall to the sustain level (with the amp ADSR it also times the sweep, the bend and the partials)",
        knob::AMP_S => "Sustain: the level held while the note is (a MIDI key, a step's GATE, TRIG held down)",
        knob::FLT_S => "Sustain: how much of AMOUNT is held while the note is",
        knob::AMP_R | knob::FLT_R => "Release: how long it takes to fall once the note is let go",
        knob::FLT_AMT => "How far the ADSR moves the tone filter: up to 8 octaves up, or down when negative (velocity sensitive)",
        _ => "The channel's overall level",
    }
}

/// A knob's value as the panel writes it under the knob.
fn value_text(k: usize, v: f32) -> String {
    match k {
        knob::N_COLOR | knob::BEND if v > 0.0 => format!("+{v:.0}"),
        knob::N_FILTER | knob::T_FILTER if v >= 99.0 => "99 · open".to_owned(),
        knob::N_FILTER | knob::T_FILTER => format!("{v:.0} · {}", short_hz(voice::filter_hz(v))),
        knob::N_DECAY | knob::T_DECAY => format!("{v:.0} · {}", short_seconds(voice::decay_seconds(v))),
        knob::C_DECAY => format!("{v:.0} · {}", short_seconds(voice::click_seconds(v))),
        knob::PITCH => format!("{v:.1} · {}", note_label(v.floor() as i32) + if v.fract() > 0.25 { "+½" } else { "" }),
        knob::T_WAVE => WAVES[(v.round().max(0.0) as usize).min(16)].name.to_owned(),
        knob::C_TYPE => voice::click_name(v.round().max(0.0) as usize),
        knob::N_GATE | knob::T_GATE | knob::C_GATE => GATES[(v.round().max(0.0) as usize).min(2)].to_owned(),
        knob::AMP_A | knob::FLT_A => short_seconds(voice::attack_seconds(v)),
        knob::AMP_D | knob::FLT_D | knob::AMP_R | knob::FLT_R => short_seconds(voice::decay_seconds(v)),
        knob::AMP_S | knob::FLT_S => format!("{:.0}%", v / 99.0 * 100.0),
        knob::FLT_AMT if v > 0.0 => format!("+{v:.0} · {:.1} oct", v * voice::AMOUNT_OCTAVES),
        knob::FLT_AMT => format!("{v:.0} · {:.1} oct", v * voice::AMOUNT_OCTAVES),
        _ => format!("{v:.0}"),
    }
}

fn short_hz(hz: f32) -> String {
    if hz >= 1000.0 { format!("{:.1}k", hz / 1000.0) } else { format!("{hz:.0}") }
}

fn short_seconds(s: f32) -> String {
    if s >= 1.0 { format!("{s:.1}s") } else { format!("{:.0}ms", s * 1000.0) }
}

/// The page shown: 0 SOUND, 1 SEQUENCER · MIDI.
pub fn page(p: &ModuleParams) -> usize {
    fj::int(p, data::TAB, 0, 1) as usize
}

/// The node's size now: it follows the page.
pub fn size(p: &ModuleParams) -> Vec2 {
    vec2(W, HEIGHTS[page(p)])
}

/// Collapsed nodes' readout: the program, its name and state.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let k = fj::program(p);
    let name = fj::program_name(p, k);
    let state = if fj::edited(p) { " · edited" } else { "" };
    let seq = if p.viz[viz::STEP].get() >= 0.0 { " · playing" } else { "" };
    (format!("P{} {name}", k + 1), format!("{}{state}{seq}", category(p, k)))
}

/// "Retro · Kit" for a factory program, "your program" otherwise.
fn category(p: &ModuleParams, k: usize) -> String {
    match FACTORY.get(k) {
        Some(prog) if fj::program_name(p, k) == prog.name => format!("{} · {}", prog.style, prog.category),
        _ if fj::program_name(p, k) == "Empty" => "empty location".to_owned(),
        _ => "your program".to_owned(),
    }
}

/// A program's colour in the bank: its factory style, white for one of your
/// own, faint for an empty location.
fn style_color(k: usize, name: &str) -> Color32 {
    match FACTORY.get(k) {
        Some(prog) if prog.name == name => STYLES.iter().find(|(s, _)| *s == prog.style).map_or(theme::TEXT, |(_, c)| *c),
        _ if name == "Empty" => theme::TEXT_FAINT,
        _ => theme::TEXT,
    }
}

/// The edited channel's strike, for the display.
struct Preview {
    /// Per component, the loudest sample per column (1 = the loudest).
    columns: [Vec<f32>; 3],
    seconds: f32,
}

impl Preview {
    fn new(s: &Sound, vel: f32) -> Self {
        let seconds = voice::hit_seconds(s, PREVIEW_GATE).min(PREVIEW_MAX);
        let n = ((seconds * PREVIEW_SR) as usize).max(COLUMNS);
        let parts = voice::render(s, vel, PREVIEW_GATE, PREVIEW_SR, n);
        let top = parts.iter().flat_map(|p| p.iter()).fold(1e-6f32, |m, y| m.max(y.abs()));
        // Each column's peak over a window: two periods of the tone (so
        // the outline follows its envelope, not its waveform), a few
        // milliseconds of the noise and the click.
        let period = 1.0 / voice::pitch_hz(s[knob::PITCH]);
        let windows = [(2.0 * period).clamp(0.002, 0.06), 0.006, 0.0015];
        let columns = std::array::from_fn(|c| {
            let half = ((windows[c] * PREVIEW_SR) as usize / 2).max(n / COLUMNS / 2 + 1);
            (0..COLUMNS)
                .map(|i| {
                    let mid = (2 * i + 1) * n / (2 * COLUMNS);
                    parts[c][mid.saturating_sub(half)..(mid + half).min(n)].iter().fold(0.0f32, |m, y| m.max(y.abs())) / top
                })
                .collect()
        });
        Preview { columns, seconds }
    }
}

/// What the panel remembers between frames: the lights and the playhead.
#[derive(Clone, Default)]
struct View {
    /// Per channel: hits seen, when the last came, its strength.
    hits: [(f32, f64, f32); CHANNELS],
    /// Picking the location to store to.
    storing: bool,
    /// The last ALEA throw: the die's face, when, and what it changed.
    face: u8,
    thrown: Option<f64>,
    before: Option<Before>,
    /// When a Store was refused (Memory Protect), and the last one done.
    refused: Option<f64>,
    stored: Option<(f64, usize)>,
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
    /// The edited channel.
    c: usize,
}

/// `ports`: which inputs and outputs are patched; `midi`: the MIDI input
/// and output.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, (inputs, outputs): (&[bool], &[bool]), (midi_in, midi_out): (bool, bool)) {
    let p = &m.params;
    let key = Id::unique(("fjord-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    for c in 0..CHANNELS {
        let hits = p.viz[viz::HITS + c].get();
        if hits != view.hits[c].0 {
            view.hits[c] = (hits, now, p.viz[viz::STRENGTH + c].get());
        }
    }
    let s = Panel { v, id, p, accent, now, c: fj::selected(p) };

    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    let patched = |list: &[bool], i: usize| list.get(i).copied().unwrap_or(false);
    for (i, label) in fj::DEF.inputs.iter().enumerate() {
        v.text(PAD, 42.0 + 18.0 * i as f32, Align2::LEFT_CENTER, label, v.font(9.0), col(patched(inputs, i)));
    }
    v.text(PAD, 42.0 + 18.0 * 4.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi_in));
    for (i, label) in fj::DEF.outputs.iter().enumerate() {
        v.text(W - PAD, 42.0 + 18.0 * i as f32, Align2::RIGHT_CENTER, label, v.font(9.0), col(patched(outputs, i)));
    }
    v.text(W - PAD, 42.0 + 18.0 * 5.0, Align2::RIGHT_CENTER, "MIDI", v.font(9.0), col(midi_out));

    s.programs(&mut view);
    s.channel_keys(&view);
    s.solo_master();
    s.alea(&mut view);
    s.tabs();
    if page(p) == 0 {
        s.display(&view);
        s.noise();
        s.tone();
        s.click();
        s.mixer();
        s.envelopes();
    } else {
        s.sequencer();
        s.system();
    }

    let busy = p.viz[viz::STEP].get() >= 0.0
        || view.hits.iter().any(|h| now - h.1 < RED_LONG.max(PREVIEW_MAX as f64))
        || view.refused.is_some_and(|t| now - t < PROTECTED)
        || view.stored.is_some_and(|(t, _)| now - t < PROTECTED)
        || view.thrown.is_some_and(|t| now - t < TUMBLE);
    if busy {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    v.ui.data_mut(|d| d.insert_temp(key, view));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("fjord", self.id)).with(salt)
    }

    /// The edited channel's knob `k`.
    fn get(&self, k: usize) -> f32 {
        self.p.get(ch(self.c, k))
    }

    fn set(&self, k: usize, v: f32) {
        let v = fj::snap(k, v);
        if v != self.get(k) {
            self.p.set(ch(self.c, k), v);
        }
    }

    /// A small button.
    fn chip(&self, salt: impl std::hash::Hash + std::fmt::Debug, rect: Rect, label: &str, lit: bool, color: Color32, tip: &str) -> Response {
        widgets::toggle(self.v.ui, self.v.painter, self.uid(("chip", salt)), rect, label, lit, color, self.z()).on_hover_text(tip)
    }

    /// A row of keys across `rect`, `current` lit in `color`; returns the
    /// key clicked. `label` names each key, `tip` describes it.
    #[allow(clippy::too_many_arguments)]
    fn keys(&self, salt: &str, rect: Rect, count: usize, current: Option<usize>, color: Color32, size: f32, label: impl Fn(usize) -> String, tip: impl Fn(usize) -> String) -> Option<usize> {
        let (v, z) = (self.v, self.z());
        let kw = rect.width() / count as f32;
        let mut picked = None;
        for k in 0..count {
            let r = Rect::from_min_max(pos2(rect.left() + kw * k as f32 + 1.0 * z, rect.top()), pos2(rect.left() + kw * (k + 1) as f32 - 1.0 * z, rect.bottom()));
            let resp = v.ui.interact(r, self.uid((salt, k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            let on = current == Some(k);
            let fill = if on {
                color
            } else if resp.hovered() {
                theme::CONTROL_HI
            } else {
                theme::CONTROL
            };
            v.painter.rect_filled(r, 2.5 * z, fill);
            text(v.painter, r.center(), Align2::CENTER_CENTER, label(k), theme::font(size * z), if on { theme::INSET } else { theme::TEXT_DIM });
            let resp = if resp.hovered() { resp.on_hover_text(tip(k)) } else { resp };
            if resp.clicked() {
                picked = Some(k);
            }
        }
        picked
    }

    /// A value box: drag up or right to change it (a step every few
    /// pixels), double-click to reset.
    #[allow(clippy::too_many_arguments)]
    fn drag_box(&self, salt: &str, rect: Rect, value: f32, (lo, hi, reset): (f32, f32, f32), shown: &str, tip: &str, on: bool) -> Option<f32> {
        let (v, z) = (self.v, self.z());
        let id = self.uid(("box", salt));
        let resp = v.ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical).on_hover_text(tip);
        let mut out = None;
        if resp.dragged() {
            let d = resp.drag_delta();
            let steps = v.ui.data_mut(|m| {
                let acc = m.get_temp_mut_or(id.with("acc"), 0.0f32);
                *acc += (d.x - d.y) / z;
                let steps = (*acc / 4.0).trunc();
                *acc -= steps * 4.0;
                steps
            });
            if steps != 0.0 {
                out = Some((value + steps).clamp(lo, hi));
            }
        }
        if resp.double_clicked() {
            out = Some(reset);
        }
        let hot = resp.hovered() || resp.dragged();
        v.painter.rect_filled(rect, 3.0 * z, if hot { theme::CONTROL_HI } else { theme::NODE_BODY });
        text(v.painter, rect.center(), Align2::CENTER_CENTER, shown, v.mono(7.5), if on { theme::TEXT } else { theme::TEXT_FAINT });
        out
    }

    /// A section's title and the line after it.
    fn title(&self, title: &str, x0: f32, x1: f32, y: f32, color: Color32) {
        let (v, z) = (self.v, self.z());
        let r = v.text(x0, y, Align2::LEFT_CENTER, title, theme::strong(8.5 * z), color);
        let from = (r.right() + 8.0 * z).max(v.p(x0 + 40.0, 0.0).x);
        v.painter.hline(from..=v.p(x1, 0.0).x, v.p(0.0, y).y, Stroke::new(1.0 * z, alpha(color, 0.3)));
    }

    /// The edited channel's knob `k` at `cx`: its label at `y` (red for
    /// the red parameters), the knob, its value. `r`: the radius.
    fn knob(&self, k: usize, cx: f32, y: f32, r: f32, color: Color32, red: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let i = ch(self.c, k);
        let spec = fj::spec(i);
        let idle = self.idle(k);
        let color = if idle { theme::TEXT_FAINT } else { color };
        let faint = |c: Color32| if idle { alpha(c, 0.5) } else { c };
        v.text(cx, y, Align2::CENTER_CENTER, RANGES[k].0, v.font(7.5), faint(if red { RED } else { theme::TEXT_FAINT }));
        let top = y + 7.0;
        let mut norm = spec.to_norm(p.get(i));
        let stored = fj::stored(p, fj::program(p))[self.c][k];
        let resp = widgets::knob(v.ui, v.painter, self.uid(("knob", i)), v.p(cx, top + r), r * z, &mut norm, spec.to_norm(stored), spec.bipolar, color, z).on_hover_text(tip(k));
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            self.set(k, spec.denorm(norm));
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * r + 7.0, Align2::CENTER_CENTER, value_text(k, p.get(i)), v.mono(7.0), faint(if hot { theme::TEXT } else { theme::TEXT_DIM }));
    }

    /// The amp ADSR shapes the tone (DECAY and GATE stand aside).
    fn adsr(&self) -> bool {
        self.get(knob::AMP_ENV) > 0.5
    }

    /// A knob that does nothing as things are set: DECAY and GATE of the
    /// tone under its ADSR, the ADSR without it, the filter's ADSR with no
    /// AMOUNT.
    fn idle(&self, k: usize) -> bool {
        match k {
            knob::T_DECAY | knob::T_GATE => self.adsr(),
            knob::AMP_A | knob::AMP_D | knob::AMP_S | knob::AMP_R => !self.adsr(),
            knob::FLT_A | knob::FLT_D | knob::FLT_S | knob::FLT_R => self.get(knob::FLT_AMT) == 0.0,
            _ => false,
        }
    }

    /// A gate (n, S, G) as three keys under a red label.
    fn gate(&self, k: usize, x0: f32, x1: f32, y: f32) {
        let v = self.v;
        let idle = self.idle(k);
        v.text((x0 + x1) * 0.5, y, Align2::CENTER_CENTER, "GATE", v.font(7.5), if idle { alpha(RED, 0.4) } else { RED });
        let current = Some((self.get(k).round().max(0.0) as usize).min(2));
        const LONG: [&str; 3] = ["n: decays gradually", "S: a soft knee, held then falling", "G: gated, held then cut"];
        let color = if idle { theme::TEXT_FAINT } else { RED };
        if let Some(g) = self.keys(&format!("gate{k}"), v.r(x0, y + 8.0, x1, y + 24.0), 3, current, color, 8.0, |g| GATES[g].to_owned(), |g| LONG[g].to_owned()) {
            self.set(k, g as f32);
        }
    }

    // -----------------------------------------------------------------------
    // The top: programs, channels, solo and master
    // -----------------------------------------------------------------------

    /// The programs: the current one's number (drag it up or down to step
    /// through them, as the hardware's dial does) and name, STORE and
    /// PROTECT, and the bank: all 99 as a map of cells, outlined in their
    /// style's colour, the current one filled. Click a cell to load it; after
    /// STORE, click the cell to store to.
    fn programs(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let k = fj::program(p);
        let edited = fj::edited(p);
        let protect = fj::on(p, data::PROTECT);
        if view.storing && v.ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            view.storing = false;
        }
        let names: [&str; PROGRAMS] = std::array::from_fn(|n| fj::program_name(p, n));
        let color_of = |n: usize| style_color(n, names[n]);
        let [x0, x1] = PROG_X;

        // The number.
        let pill = v.r(x0, 32.0, x0 + 50.0, 58.0);
        let id = self.uid("number");
        let resp = v
            .ui
            .interact(pill, id, Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::ResizeVertical)
            .on_hover_text("Drag up or down to step through the programs (edits are lost unless stored)");
        if resp.dragged() {
            let dy = resp.drag_delta().y;
            let steps = v.ui.data_mut(|m| {
                let acc = m.get_temp_mut_or(id.with("acc"), 0.0f32);
                *acc -= dy / z;
                let steps = (*acc / 6.0).trunc();
                *acc -= steps * 6.0;
                steps as i32
            });
            if steps != 0 {
                fj::load_program(p, (k as i32 + steps).rem_euclid(PROGRAMS as i32) as usize);
            }
        }
        let hot = resp.hovered() || resp.dragged();
        let ring = color_of(k);
        v.painter.rect_filled(pill, 5.0 * z, if hot { theme::CONTROL } else { theme::INSET });
        v.painter.rect_stroke(pill, 5.0 * z, Stroke::new(1.5 * z, ring), StrokeKind::Inside);
        text(v.painter, pill.center() + vec2(0.0, 1.0 * z), Align2::CENTER_CENTER, format!("P{}", k + 1), theme::strong(15.0 * z), theme::TEXT);
        if edited {
            v.painter.circle_filled(pill.right_top() + vec2(-6.0, 6.0) * z, 2.5 * z, RED);
        }

        // The bank.
        let map = v.r(x0, MAP_Y[0], x1, MAP_Y[1]);
        let cw = (map.width() - DECADE_GAP * z) / BANK_COLS as f32;
        let rows = PROGRAMS.div_ceil(BANK_COLS);
        let rh = map.height() / rows as f32;
        let cell_rect = |n: usize| {
            let (row, col) = (n / BANK_COLS, n % BANK_COLS);
            let x = map.left() + cw * col as f32 + if col >= BANK_COLS / 2 { DECADE_GAP * z } else { 0.0 };
            Rect::from_min_size(pos2(x, map.top() + rh * row as f32), vec2(cw, rh))
        };
        let cell_at = |q: Pos2| (0..PROGRAMS).find(|&n| cell_rect(n).contains(q));
        let resp = v.ui.interact(map, self.uid("bank"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let hovered = resp.hover_pos().and_then(cell_at);
        for row in 0..rows {
            let r = cell_rect(row * BANK_COLS);
            text(v.painter, pos2(map.left() - 4.0 * z, r.center().y), Align2::RIGHT_CENTER, row * BANK_COLS + 1, theme::mono(6.0 * z), theme::TEXT_FAINT);
        }
        for (n, name) in names.iter().enumerate() {
            let r = cell_rect(n).shrink2(vec2(1.5, 1.5) * z);
            let color = color_of(n);
            let empty = *name == "Empty";
            if n == k {
                v.painter.rect_filled(r.expand(0.5 * z), 2.0 * z, color);
                if edited {
                    v.painter.circle_filled(r.center(), 1.6 * z, theme::INSET);
                }
            } else {
                if hovered == Some(n) {
                    v.painter.rect_filled(r, 2.0 * z, alpha(color, 0.3));
                }
                v.painter.rect_stroke(r, 2.0 * z, Stroke::new(1.1 * z, alpha(color, if empty { 0.3 } else { 0.85 })), StrokeKind::Inside);
            }
            if view.storing && hovered == Some(n) {
                v.painter.rect_stroke(r.expand(1.5 * z), 2.5 * z, Stroke::new(1.5 * z, RED), StrokeKind::Outside);
            }
        }
        if view.storing {
            let frame = map.expand(3.0 * z);
            let edge = [frame.left_top(), frame.right_top(), frame.right_bottom(), frame.left_bottom(), frame.left_top()];
            v.painter.extend(Shape::dashed_line(&edge, Stroke::new(1.0 * z, RED), 4.0 * z, 3.0 * z));
        }
        if let Some(n) = hovered {
            resp.clone().on_hover_text(format!("P{} · {} · {}", n + 1, names[n], category(p, n)));
        }
        if resp.clicked()
            && let Some(n) = hovered
        {
            if view.storing {
                view.storing = false;
                if fj::store_program(p, n) {
                    view.stored = Some((self.now, n));
                } else {
                    view.refused = Some(self.now);
                }
            } else {
                fj::load_program(p, n);
            }
        }

        // STORE and PROTECT.
        let store = self.chip("store", v.r(x1 - 62.0, 32.0, x1, 44.0), if view.storing { "CANCEL" } else { "STORE" }, view.storing, RED, "Store the four channels: then click the location in the bank (Esc cancels)");
        if store.clicked() {
            if protect {
                view.refused = Some(self.now);
            } else {
                view.storing = !view.storing;
            }
        }
        let lock = self.chip("protect", v.r(x1 - 62.0, 47.0, x1, 59.0), if protect { "PROTECT" } else { "UNLOCKED" }, protect, theme::TEXT_DIM, "Memory Protect: on, programs can't be overwritten");
        if lock.clicked() {
            p.data[data::PROTECT].set(if protect { 0.0 } else { 1.0 });
        }

        // The name, and what is going on.
        let (nx, recent) = (x0 + 58.0, |t: Option<f64>| t.is_some_and(|t| self.now - t < PROTECTED));
        let (title, title_col, line, line_col) = if view.storing {
            let to = hovered.map_or("pick a location · Esc cancels".to_owned(), |n| format!("to P{} · {}", n + 1, names[n]));
            ("STORE".to_owned(), RED, to, RED)
        } else if let Some(n) = hovered.filter(|&n| n != k) {
            (names[n].to_owned(), theme::TEXT_DIM, format!("P{} · {}", n + 1, category(p, n)), alpha(color_of(n), 0.8))
        } else if recent(view.refused) {
            (names[k].to_owned(), theme::TEXT, "Memory Protect is on".to_owned(), RED)
        } else if let Some((_, n)) = view.stored.filter(|&(t, _)| self.now - t < PROTECTED) {
            (names[k].to_owned(), theme::TEXT, format!("stored to P{}", n + 1), GREEN)
        } else if edited {
            (names[k].to_owned(), theme::TEXT, "edited · STORE to keep".to_owned(), RED)
        } else {
            (names[k].to_owned(), theme::TEXT, category(p, k), ring)
        };
        let clip = v.painter.with_clip_rect(v.r(nx, 30.0, x1 - 66.0, 62.0).intersect(v.painter.clip_rect()));
        text(&clip, v.p(nx, 40.0), Align2::LEFT_CENTER, title, theme::strong(10.0 * z), title_col);
        text(&clip, v.p(nx, 53.0), Align2::LEFT_CENTER, line, theme::font(7.5 * z), line_col);
    }

    fn channel_keys(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        for c in 0..CHANNELS {
            let x = KEYS_X + 70.0 * c as f32;
            let rect = v.r(x, 32.0, x + 64.0, 86.0);
            let resp = v.ui.interact(rect, self.uid(("channel", c)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!(
                "Channel {}: click to edit it (TRIG plays it); double-click to play it",
                c + 1
            ));
            let selected = c == self.c;
            v.painter.rect_filled(rect, 4.0 * z, if resp.hovered() { theme::CONTROL } else { theme::NODE_BODY });
            v.painter.rect_stroke(rect, 4.0 * z, Stroke::new(if selected { 2.0 } else { 1.0 } * z, if selected { self.accent } else { theme::CONTROL_HI }), StrokeKind::Inside);
            text(v.painter, rect.left_top() + vec2(8.0, 4.0) * z, Align2::LEFT_TOP, format!("{}", c + 1), theme::strong(16.0 * z), if selected { self.accent } else { theme::TEXT_DIM });
            let s = fj::sound(p, c);
            let wave = WAVES[voice::int(&s, knob::T_WAVE, 0, 16) as usize].name;
            let summary = format!("{wave} · {:.1}", s[knob::PITCH]);
            text(v.painter, rect.left_bottom() + vec2(8.0, -5.0) * z, Align2::LEFT_BOTTOM, summary, v.mono(7.0), theme::TEXT_FAINT);
            // The lights: green on a hit, red for a hard one.
            let (_, at, strength) = view.hits[c];
            let age = self.now - at;
            let green = (1.0 - (age / FLASH) as f32).clamp(0.0, 1.0);
            let red_for = if strength >= 1.0 { RED_LONG } else if strength >= 0.5 { RED_SHORT } else { 0.0 };
            let red = if age < red_for { 1.0 } else { 0.0 };
            let led = |dx: f32, color: Color32, lit: f32| {
                let ctr = rect.right_top() + vec2(-dx, 10.0) * z;
                v.painter.circle_filled(ctr, 3.5 * z, theme::INSET);
                v.painter.circle_stroke(ctr, 3.5 * z, Stroke::new(1.0 * z, alpha(color, 0.5)));
                if lit > 0.0 {
                    v.painter.circle_filled(ctr, 2.5 * z, alpha(color, lit));
                }
            };
            led(10.0, GREEN, green);
            led(21.0, RED, red);
            if resp.clicked() {
                self.pick(c);
            }
            if resp.double_clicked() {
                self.trig(c);
            }
        }
        // TRIG and its velocity, the channel's copy, paste and init.
        let y = [94.0, 110.0];
        let trig = self.chip("trig", v.r(KEYS_X, y[0], KEYS_X + 48.0, y[1]), "TRIG", false, GREEN, "Play the edited channel at the TRIG velocity; hold it down to hold the note (the ADSRs' sustain)");
        let down = trig.is_pointer_button_down_on();
        let was_id = self.uid("trig-down");
        let was = v.ui.data(|d| d.get_temp::<bool>(was_id)).unwrap_or(false);
        if down && !was {
            self.trig(self.c);
        }
        if down != was {
            v.ui.data_mut(|d| d.insert_temp(was_id, down));
        }
        for c in 0..CHANNELS {
            p.viz[viz::HOLD + c].set(if down && c == self.c { 1.0 } else { 0.0 });
        }
        if down {
            v.painter.rect_filled(trig.rect, 3.0 * z, GREEN);
            text(v.painter, trig.rect.center(), Align2::CENTER_CENTER, "TRIG", theme::font(9.0 * z), theme::INSET);
        }
        let vel = p.data[data::TRIG_VEL].get();
        if let Some(n) = self.drag_box("vel", v.r(KEYS_X + 52.0, y[0], KEYS_X + 100.0, y[1]), vel, (1.0, 127.0, 100.0), &format!("VEL {vel:.0}"), "TRIG's velocity (drag; double-click: 100)", true) {
            p.data[data::TRIG_VEL].set(n);
        }
        let clip_id = Id::unique("fjord-clipboard");
        if self.chip("copy", v.r(KEYS_X + 106.0, y[0], KEYS_X + 152.0, y[1]), "COPY", false, self.accent, "Copy the edited channel's sound (to paste in any channel, any program, any Fjord)").clicked() {
            let s = fj::sound(p, self.c);
            v.ui.data_mut(|d| d.insert_temp(clip_id, s));
        }
        let clip: Option<Sound> = v.ui.data(|d| d.get_temp(clip_id));
        let paste = self.chip("paste", v.r(KEYS_X + 156.0, y[0], KEYS_X + 204.0, y[1]), "PASTE", false, self.accent, if clip.is_some() { "Paste the copied sound into the edited channel" } else { "Nothing copied yet" });
        if paste.clicked()
            && let Some(s) = clip
        {
            fj::set_sound(p, self.c, &s);
        }
        if self.chip("init", v.r(KEYS_X + 208.0, y[0], KEYS_X + 274.0, y[1]), "INIT", false, RED, "Sound Init: the edited channel back to a plain starting sound").clicked() {
            fj::init_channel(p, self.c);
        }
    }

    /// Picks the channel to edit by hand: pads stop picking it.
    fn pick(&self, c: usize) {
        self.p.data[data::CHANNEL].set(c as f32);
        self.p.data[data::FOLLOW].set(0.0);
    }

    fn trig(&self, c: usize) {
        let at = viz::AUDITION + c;
        self.p.viz[at].set(self.p.viz[at].get() + 1.0);
    }

    fn solo_master(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let solo = fj::int(p, data::SOLO, 0, 3) as usize;
        v.text(SOLO_X, 38.0, Align2::LEFT_CENTER, "SOLO EDIT", v.font(7.5), if solo > 0 { theme::SOLO } else { theme::TEXT_FAINT });
        const ITEMS: [&str; 4] = ["OFF", "TONE", "NOISE", "CLICK"];
        const TIPS: [&str; 4] = ["Hear the whole channel", "Hear only the edited channel's tone", "Hear only the edited channel's noise", "Hear only the edited channel's click"];
        let colors = [theme::TEXT_DIM, TONE_COL, NOISE_COL, CLICK_COL];
        if let Some(k) = self.keys("solo", v.r(SOLO_X, 46.0, 830.0, 62.0), 4, Some(solo), colors[solo], 7.5, |k| ITEMS[k].to_owned(), |k| TIPS[k].to_owned()) {
            p.data[data::SOLO].set(k as f32);
        }
        // MASTER: a knob of its own (not stored in programs).
        let spec = fj::spec(fj::MASTER);
        let (cx, top, r) = (SOLO_X + 20.0, 80.0, 11.0);
        v.text(cx, 72.0, Align2::CENTER_CENTER, "MASTER", v.font(7.5), theme::TEXT_FAINT);
        let mut norm = spec.to_norm(p.get(fj::MASTER));
        let resp = widgets::knob(v.ui, v.painter, self.uid("master"), v.p(cx, top + r), r * z, &mut norm, spec.to_norm(spec.default), false, self.accent, z)
            .on_hover_text("Overall level (not stored in the programs)");
        v.hit(resp.rect, ParamRef::Module(fj::MASTER));
        if resp.dragged() || resp.double_clicked() {
            p.set(fj::MASTER, spec.denorm(norm));
        }
        let level = format!("{:.0}", p.get(fj::MASTER) * 99.0);
        v.text(cx, top + 2.0 * r + 8.0, Align2::CENTER_CENTER, level, v.mono(7.0), theme::TEXT_DIM);
    }

    /// The two pages' tabs, and a rule after them.
    fn tabs(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        const PAGES: [&str; 2] = ["SOUND", "SEQUENCER · MIDI"];
        const TIPS: [&str; 2] = ["The edited channel's sound: tone, noise, click, mixer, envelopes", "The step lanes, and how the channels are played: MIDI and the Trig inputs"];
        let [y0, y1] = TABS_Y;
        if let Some(t) = self.keys("page", v.r(PAD, y0, 300.0, y1), 2, Some(page(p)), self.accent, 8.0, |t| PAGES[t].to_owned(), |t| TIPS[t].to_owned()) {
            p.data[data::TAB].set(t as f32);
        }
        v.painter.hline(v.r(308.0, 0.0, 830.0, 0.0).x_range(), v.p(0.0, (y0 + y1) * 0.5).y, Stroke::new(1.0 * z, theme::GRID_MAJOR));
    }

    // -----------------------------------------------------------------------
    // ALEA: throws of the dice
    // -----------------------------------------------------------------------

    /// The die (its face from the last throw, tumbling for a moment after
    /// it), CHANCE, the three throws and BACK.
    fn alea(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [x0, x1] = ALEA_X;
        let tumbling = view.thrown.is_some_and(|t| self.now - t < TUMBLE);
        let face = if tumbling { ((self.now * 30.0) as u64 % 6) as u8 + 1 } else { view.face.max(1) };
        die(v.painter, v.r(x0, 68.0, x0 + 20.0, 88.0), face, if view.thrown.is_some() { self.accent } else { theme::TEXT_DIM }, z);
        v.text(x0 + 26.0, 74.0, Align2::LEFT_CENTER, "ALEA", theme::strong(9.5 * z), theme::TEXT);
        v.text(x0 + 26.0, 84.0, Align2::LEFT_CENTER, "throw the dice", v.font(6.5), theme::TEXT_FAINT);
        let chance = p.data[data::CHANCE].get();
        if let Some(n) = self.drag_box(
            "chance",
            v.r(x1 - 62.0, 70.0, x1, 86.0),
            chance,
            (1.0, 100.0, 60.0),
            &format!("CHANCE {chance:.0}%"),
            "How far a throw goes: a little nudges what is there, 100% starts afresh (drag; double-click: 60%)",
            true,
        ) {
            p.data[data::CHANCE].set(n);
        }
        let amount = chance.clamp(1.0, 100.0) / 100.0;
        let y = [92.0, 106.0];
        let w = (x1 - x0 - 8.0) / 3.0;
        let shift = v.ui.input(|i| i.modifiers.shift);
        let throws: [(&str, &str); 3] = [
            ("SOUND", "New sounds for the four channels, bred from the factory programs (Shift: the edited channel only)"),
            ("STEPS", "New steps, each lane's groove made for what its channel sounds like"),
            ("ALL", "New sounds, then steps for them"),
        ];
        let mut thrown = None;
        for (i, (label, tip)) in throws.into_iter().enumerate() {
            let x = x0 + (w + 4.0) * i as f32;
            if self.chip(("alea", i), v.r(x, y[0], x + w, y[1]), label, false, self.accent, tip).clicked() {
                thrown = Some(i);
            }
        }
        if let Some(i) = thrown {
            let seed = ((self.now * 1000.0) as u32) ^ self.id.wrapping_mul(0x9E37_79B9) ^ (i as u32 + 1).wrapping_mul(0x85EB_CA6B);
            view.before = Some(Before::take(p));
            let all: Vec<usize> = (0..CHANNELS).collect();
            let channels = if shift { vec![self.c] } else { all };
            if i != 1 {
                alea::throw_sound(p, seed, amount, &channels);
            }
            if i != 0 {
                alea::throw_steps(p, seed, amount);
            }
            view.face = (seed % 6) as u8 + 1;
            view.thrown = Some(self.now);
        }
        let back = self.chip("alea-back", v.r(x0, 110.0, x0 + 56.0, 122.0), "BACK", false, theme::TEXT_DIM, "Take back the last throw");
        if back.clicked()
            && let Some(before) = view.before.take()
        {
            before.restore(p);
        }
        let note = if view.before.is_some() { "BACK takes it back" } else { "Shift-SOUND: this channel" };
        v.text(x0 + 62.0, 116.0, Align2::LEFT_CENTER, note, v.font(6.5), theme::TEXT_FAINT);
    }

    // -----------------------------------------------------------------------
    // The display
    // -----------------------------------------------------------------------

    fn display(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(PAD, DISPLAY[0], W - PAD, DISPLAY[1]);
        v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let s = fj::sound(p, self.c);
        let vel = fj::int(p, data::TRIG_VEL, 1, 127) as f32 / 127.0;
        let key: [u32; knob::COUNT + 1] = std::array::from_fn(|k| if k < knob::COUNT { s[k].to_bits() } else { vel.to_bits() });
        let id = self.uid("preview");
        let preview = match v.ui.data(|d| d.get_temp::<([u32; knob::COUNT + 1], Arc<Preview>)>(id)) {
            Some((k, preview)) if k == key => preview,
            _ => {
                let preview = Arc::new(Preview::new(&s, vel));
                v.ui.data_mut(|d| d.insert_temp(id, (key, preview.clone())));
                preview
            }
        };
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let inner = rect.shrink2(vec2(8.0 * z, 6.0 * z));
        let (mid, h) = (inner.center().y + 4.0 * z, inner.height() * 0.42);
        // Time grid.
        let step = [0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0].into_iter().find(|&st| preview.seconds / st <= 10.0).unwrap_or(1.0);
        let mut t = step;
        while t < preview.seconds {
            let x = inner.left() + inner.width() * t / preview.seconds;
            painter.vline(x, inner.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
            text(&painter, pos2(x + 2.0 * z, inner.bottom()), Align2::LEFT_BOTTOM, short_seconds(t), theme::mono(6.5 * z), theme::TEXT_FAINT);
            t += step;
        }
        painter.hline(inner.x_range(), mid, Stroke::new(1.0 * z, theme::GRID_MAJOR));
        // Each component's outline, mirrored: noise under tone under click.
        let w = inner.width() / (COLUMNS - 1) as f32;
        for (c, color) in [(1usize, NOISE_COL), (0, TONE_COL), (2, CLICK_COL)] {
            let cols = &preview.columns[c];
            if cols.iter().all(|&a| a < 0.002) {
                continue;
            }
            let top: Vec<Pos2> = cols.iter().enumerate().map(|(i, &a)| pos2(inner.left() + w * i as f32, mid - a * h)).collect();
            let bottom: Vec<Pos2> = cols.iter().enumerate().map(|(i, &a)| pos2(inner.left() + w * i as f32, mid + a * h)).collect();
            painter.add(Shape::line(top, Stroke::new(1.5 * z, color)));
            painter.add(Shape::line(bottom, Stroke::new(1.0 * z, alpha(color, 0.55))));
        }
        // Where the preview lets go of its note, when an ADSR listens.
        if (s[knob::AMP_ENV] > 0.5 || s[knob::FLT_AMT] != 0.0) && PREVIEW_GATE < preview.seconds {
            let x = inner.left() + inner.width() * PREVIEW_GATE / preview.seconds;
            painter.extend(Shape::dashed_line(&[pos2(x, inner.top() + 10.0 * z), pos2(x, inner.bottom())], Stroke::new(1.0 * z, alpha(TONE_COL, 0.6)), 3.0 * z, 3.0 * z));
            text(&painter, pos2(x + 3.0 * z, inner.top() + 14.0 * z), Align2::LEFT_CENTER, format!("note off · {}", short_seconds(PREVIEW_GATE)), theme::mono(6.5 * z), alpha(TONE_COL, 0.8));
        }
        // The bend: the pitch over the strike, dashed, from where it starts.
        let bend = s[knob::BEND];
        if bend.abs() > 0.0 && s[knob::TONE] > 0.0 {
            let decay = voice::decay_seconds(s[knob::T_DECAY]) * voice::time_share(s[knob::BEND_TIME]);
            let start = bend * voice::BEND_STEP * (0.3 + 0.7 * vel);
            let y_of = |st: f32| mid - (st / 30.0) * h * 0.95;
            let points: Vec<Pos2> = (0..COLUMNS)
                .map(|i| {
                    let t = preview.seconds * i as f32 / (COLUMNS - 1) as f32;
                    pos2(inner.left() + w * i as f32, y_of(start * (-6.9 * t / decay.max(1e-4)).exp()))
                })
                .collect();
            painter.extend(Shape::dashed_line(&points, Stroke::new(1.2 * z, theme::TEXT_DIM), 4.0 * z, 3.0 * z));
            text(&painter, pos2(inner.left() + 24.0 * z, y_of(start) + if start > 0.0 { -2.0 } else { 6.0 } * z), Align2::LEFT_CENTER, format!("bend {:+.0} st", start), theme::mono(6.5 * z), theme::TEXT_DIM);
        }
        // Tags: what each component is.
        let gate = |k: usize| GATES[voice::int(&s, k, 0, 2) as usize];
        let tags = [
            (TONE_COL, knob::TONE, {
                let wave = &WAVES[voice::int(&s, knob::T_WAVE, 0, 16) as usize];
                let what = wave.long.split(':').next().unwrap_or(wave.long);
                let shape = if s[knob::AMP_ENV] > 0.5 { "ADSR" } else { gate(knob::T_GATE) };
                format!("TONE {} {what} · {} · {shape}", wave.name, note_label(s[knob::PITCH].round() as i32))
            }),
            (NOISE_COL, knob::NOISE, format!("NOISE · type {} · {}", voice::int(&s, knob::N_TYPE, 1, 7), gate(knob::N_GATE))),
            (CLICK_COL, knob::CLICK, format!("CLICK {} · {}", voice::click_name(voice::int(&s, knob::C_TYPE, 0, 26) as usize), gate(knob::C_GATE))),
        ];
        let mut x = inner.left();
        for (color, k, label) in tags {
            let on = s[k] > 0.0;
            let r = text(&painter, pos2(x, inner.top() + 4.0 * z), Align2::LEFT_CENTER, label, theme::font(7.0 * z), if on { color } else { alpha(color, 0.35) });
            x = r.right() + 14.0 * z;
        }
        text(&painter, inner.right_top() + vec2(0.0, 4.0 * z), Align2::RIGHT_CENTER, format!("CH {} · {}", self.c + 1, short_seconds(voice::hit_seconds(&s, PREVIEW_GATE))), theme::mono(7.0 * z), theme::TEXT_FAINT);
        // A line runs through on each hit of the edited channel.
        let age = (self.now - view.hits[self.c].1) as f32;
        if view.hits[self.c].0 > 0.0 && age < preview.seconds {
            let x = inner.left() + inner.width() * age / preview.seconds;
            painter.vline(x, inner.y_range(), Stroke::new(1.5 * z, alpha(theme::TEXT, 0.7)));
        }
        let resp = v.ui.interact(rect, self.uid("display"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Click to play the edited channel");
        if resp.clicked() {
            self.trig(self.c);
        }
    }

    // -----------------------------------------------------------------------
    // The sections
    // -----------------------------------------------------------------------

    fn noise(&self) {
        let v = self.v;
        let [x0, x1] = NOISE_X;
        self.title("NOISE", x0, x1, EDIT_Y, NOISE_COL);
        v.text(x0, (SEL[0] + SEL[1]) * 0.5, Align2::LEFT_CENTER, "TYPE", v.font(7.5), RED);
        let current = Some((self.get(knob::N_TYPE).round().max(1.0) as usize).min(7) - 1);
        let rect = v.r(x0 + 32.0, SEL[0], x1, SEL[1]);
        if let Some(t) = self.keys("ntype", rect, 7, current, NOISE_COL, 8.0, |t| format!("{}", t + 1), |t| format!("Filter type {} (red): resonance Q {:.2}", t + 1, NOISE_Q[t])) {
            self.set(knob::N_TYPE, t as f32 + 1.0);
        }
        let cw = (x1 - x0) / 4.0;
        let cx = |i: usize| x0 + cw * (i as f32 + 0.5);
        for (i, k) in [knob::N_COLOR, knob::N_FILTER, knob::N_SWEEP, knob::N_DECAY].into_iter().enumerate() {
            self.knob(k, cx(i), KNOB_Y, 10.0, NOISE_COL, false);
        }
        self.knob(knob::N_TIME, cx(2), RED_Y, 8.0, NOISE_COL, true);
        self.gate(knob::N_GATE, cx(3) - cw * 0.5 + 2.0, x1, RED_Y);
        self.filter_curve(v.r(x0 + 4.0, RED_Y - 4.0, cx(1) + cw * 0.5 - 6.0, RED_Y + 32.0), self.get(knob::N_FILTER), NOISE_Q[(self.get(knob::N_TYPE).round().max(1.0) as usize).min(7) - 1], NOISE_COL);
    }

    /// A small picture of a low-pass response: cutoff and resonance.
    fn filter_curve(&self, rect: Rect, filter: f32, q: f32, color: Color32) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(rect, 3.0 * z, Stroke::new(1.0 * z, alpha(color, 0.2)), StrokeKind::Inside);
        let inner = rect.shrink(3.0 * z);
        let fc = voice::filter_hz(filter).min(20_000.0);
        let points: Vec<Pos2> = (0..48)
            .map(|i| {
                let f = 20.0 * 2f32.powf(10.0 * i as f32 / 47.0);
                let r = f / fc;
                let mag = 1.0 / ((1.0 - r * r).powi(2) + (r / q).powi(2)).sqrt();
                let db = (20.0 * mag.log10()).clamp(-36.0, 24.0);
                pos2(inner.left() + inner.width() * i as f32 / 47.0, inner.top() + inner.height() * (24.0 - db) / 60.0)
            })
            .collect();
        v.painter.add(Shape::line(points, Stroke::new(1.3 * z, color)));
    }

    fn tone(&self) {
        let v = self.v;
        let [x0, x1] = TONE_X;
        self.title("TONE", x0, x1, EDIT_Y, TONE_COL);
        let current = Some((self.get(knob::T_WAVE).round().max(0.0) as usize).min(16));
        let rect = v.r(x0, SEL[0], x1, SEL[1]);
        if let Some(w) = self.keys("wave", rect, WAVES.len(), current, TONE_COL, 6.5, |w| WAVES[w].name.to_owned(), |w| format!("{}: {}", WAVES[w].name, WAVES[w].long)) {
            self.set(knob::T_WAVE, w as f32);
        }
        let cw = (x1 - x0) / 5.0;
        let cx = |i: usize| x0 + cw * (i as f32 + 0.5);
        for (i, k) in [knob::T_FILTER, knob::T_SWEEP, knob::T_DECAY, knob::PITCH, knob::BEND].into_iter().enumerate() {
            self.knob(k, cx(i), KNOB_Y, 10.0, TONE_COL, false);
        }
        self.knob(knob::T_PUNCH, cx(0), RED_Y, 8.0, TONE_COL, true);
        self.knob(knob::T_TIME, cx(1), RED_Y, 8.0, TONE_COL, true);
        self.gate(knob::T_GATE, cx(2) - cw * 0.5 + 4.0, cx(2) + cw * 0.5 - 4.0, RED_Y);
        // Under PITCH: what it is in Hz.
        let pitch = self.get(knob::PITCH);
        v.text(cx(3), RED_Y + 10.0, Align2::CENTER_CENTER, format!("{:.1} Hz", voice::pitch_hz(pitch)), v.mono(7.0), theme::TEXT_FAINT);
        v.text(cx(3), RED_Y + 22.0, Align2::CENTER_CENTER, "69.0 = A 440", v.font(6.5), theme::TEXT_FAINT);
        self.knob(knob::BEND_TIME, cx(4), RED_Y, 8.0, TONE_COL, true);
    }

    fn click(&self) {
        let v = self.v;
        let [x0, x1] = CLICK_X;
        self.title("CLICK", x0, x1, EDIT_Y, CLICK_COL);
        let kind = (self.get(knob::C_TYPE).round().max(0.0) as usize).min(26);
        let (family, index) = (kind / 9, kind % 9);
        const LONG: [&str; 3] = ["n: noise, nine characters", "c: noise with an added attack", "t: pitched ticks"];
        let rect = v.r(x0, SEL[0], x1, SEL[1]);
        if let Some(f) = self.keys("cfamily", rect, 3, Some(family), CLICK_COL, 8.0, |f| CLICK_FAMILIES[f].to_owned(), |f| LONG[f].to_owned()) {
            self.set(knob::C_TYPE, (f * 9 + index) as f32);
        }
        // The nine of the family, as a keypad.
        for row in 0..3 {
            let y0 = KNOB_Y - 4.0 + 15.0 * row as f32;
            let r = v.r(x0, y0, x1, y0 + 13.0);
            let current = (index / 3 == row).then_some(index % 3);
            if let Some(k) = self.keys(&format!("cindex{row}"), r, 3, current, CLICK_COL, 7.5, |k| format!("{}{}", CLICK_FAMILIES[family], row * 3 + k + 1), |k| format!("Click {}{}", CLICK_FAMILIES[family], row * 3 + k + 1)) {
                self.set(knob::C_TYPE, (family * 9 + row * 3 + k) as f32);
            }
        }
        let mid = (x0 + x1) * 0.5;
        self.knob(knob::C_DECAY, x0 + 26.0, RED_Y, 8.0, CLICK_COL, false);
        self.gate(knob::C_GATE, mid - 6.0, x1, RED_Y);
    }

    fn mixer(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [x0, x1] = MIXER_X;
        self.title("MIXER", x0, x1, EDIT_Y, theme::TEXT_DIM);
        let cw = (x1 - x0) / 4.0;
        let faders = [(knob::TONE, TONE_COL), (knob::NOISE, NOISE_COL), (knob::CLICK, CLICK_COL), (knob::LEVEL, self.accent)];
        for (i, (k, color)) in faders.into_iter().enumerate() {
            let cx = x0 + cw * (i as f32 + 0.5);
            let idx = ch(self.c, k);
            let spec = fj::spec(idx);
            v.text(cx, (SEL[0] + SEL[1]) * 0.5, Align2::CENTER_CENTER, RANGES[k].0, v.font(7.0), theme::TEXT_FAINT);
            let rect = v.r(cx - 6.0, SEL[1] + 8.0, cx + 6.0, RED_Y + 18.0);
            let mut pos = spec.to_norm(p.get(idx));
            let resp = widgets::vfader(v.ui, v.painter, self.uid(("fader", idx)), rect, &mut pos, color, true, z).on_hover_text(tip(k));
            v.hit(resp.rect, ParamRef::Module(idx));
            if resp.dragged() {
                self.set(k, spec.denorm(pos));
            }
            if resp.double_clicked() {
                self.set(k, fj::stored(p, fj::program(p))[self.c][k]);
            }
            let hot = resp.hovered() || resp.dragged();
            v.text(cx, RED_Y + 30.0, Align2::CENTER_CENTER, format!("{:.0}", p.get(idx)), v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
        }
    }

    // -----------------------------------------------------------------------
    // The tone's envelopes
    // -----------------------------------------------------------------------

    /// AMP ENV: DECAY (the Nord Drum's) or ADSR, the ADSR's knobs and its
    /// shape; FILTER ENV: its ADSR, AMOUNT and its shape.
    fn envelopes(&self) {
        let v = self.v;
        let [a0, a1] = AMP_X;
        self.title("AMP ENV", a0, a1, ENV_Y, TONE_COL);
        let adsr = self.adsr();
        const MODES: [&str; 2] = ["DECAY", "ADSR"];
        const MODE_TIPS: [&str; 2] = ["The tone's level follows DECAY and GATE, as on the Nord Drum", "The tone's level follows this ADSR, held as long as the note"];
        if let Some(m) = self.keys("ampenv", v.r(a0, ENV_KNOBS - 2.0, a0 + 100.0, ENV_KNOBS + 14.0), 2, Some(usize::from(adsr)), TONE_COL, 7.5, |m| MODES[m].to_owned(), |m| MODE_TIPS[m].to_owned()) {
            self.set(knob::AMP_ENV, m as f32);
        }
        let s = fj::sound(self.p, self.c);
        let amp = [voice::attack_seconds(s[knob::AMP_A]), voice::decay_seconds(s[knob::AMP_D]), voice::sustain_gain(s[knob::AMP_S]), voice::decay_seconds(s[knob::AMP_R])];
        self.adsr_shape(v.r(a0, ENV_KNOBS + 18.0, a0 + 100.0, ENV_KNOBS + 50.0), amp, 1.0, adsr);
        let cw = (a1 - a0 - 112.0) / 4.0;
        for (i, k) in [knob::AMP_A, knob::AMP_D, knob::AMP_S, knob::AMP_R].into_iter().enumerate() {
            self.knob(k, a0 + 112.0 + cw * (i as f32 + 0.5), ENV_KNOBS, 9.0, TONE_COL, false);
        }

        let [f0, f1] = FLT_X;
        self.title("FILTER ENV", f0, f1, ENV_Y, TONE_COL);
        let amount = s[knob::FLT_AMT];
        let flt = [voice::attack_seconds(s[knob::FLT_A]), voice::decay_seconds(s[knob::FLT_D]), s[knob::FLT_S] / 99.0, voice::decay_seconds(s[knob::FLT_R])];
        v.text(f0, ENV_KNOBS + 6.0, Align2::LEFT_CENTER, if amount == 0.0 { "AMOUNT 0: off" } else if amount > 0.0 { "opens the filter" } else { "closes the filter" }, v.font(7.0), if amount == 0.0 { theme::TEXT_FAINT } else { theme::TEXT_DIM });
        self.adsr_shape(v.r(f0, ENV_KNOBS + 18.0, f0 + 100.0, ENV_KNOBS + 50.0), flt, amount.signum(), amount != 0.0);
        let cw = (f1 - f0 - 112.0) / 5.0;
        for (i, k) in [knob::FLT_AMT, knob::FLT_A, knob::FLT_D, knob::FLT_S, knob::FLT_R].into_iter().enumerate() {
            self.knob(k, f0 + 112.0 + cw * (i as f32 + 0.5), ENV_KNOBS, 9.0, TONE_COL, false);
        }
    }

    /// An ADSR's shape as line art: attack, decay to the sustain, a held
    /// stretch, release; each stage as wide as its time on a log scale.
    /// `sign` −1 draws it hanging from the top (a filter closing).
    fn adsr_shape(&self, rect: Rect, [a, d, sustain, r]: [f32; 4], sign: f32, on: bool) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(rect, 3.0 * z, Stroke::new(1.0 * z, alpha(TONE_COL, if on { 0.25 } else { 0.1 })), StrokeKind::Inside);
        let inner = rect.shrink2(vec2(5.0, 4.0) * z);
        let width = |t: f32| 0.25 + (t.max(0.0005) / 0.0005).ln() / (10_000f32).ln();
        let (wa, wd, ws, wr) = (width(a), width(d), 0.9, width(r));
        let total = wa + wd + ws + wr;
        let x_at = |u: f32| inner.left() + inner.width() * u / total;
        let y_at = |level: f32| if sign < 0.0 { inner.top() + level * inner.height() } else { inner.bottom() - level * inner.height() };
        let mut points = vec![pos2(x_at(0.0), y_at(0.0)), pos2(x_at(wa), y_at(1.0))];
        for i in 1..=12 {
            let u = i as f32 / 12.0;
            points.push(pos2(x_at(wa + wd * u), y_at(sustain + (1.0 - sustain) * (-6.9 * u).exp())));
        }
        points.push(pos2(x_at(wa + wd + ws), y_at(sustain)));
        for i in 1..=12 {
            let u = i as f32 / 12.0;
            points.push(pos2(x_at(wa + wd + ws + wr * u), y_at(sustain * (-6.9 * u).exp())));
        }
        let color = if on { TONE_COL } else { alpha(theme::TEXT_FAINT, 0.6) };
        v.painter.add(Shape::line(points, Stroke::new(1.4 * z, color)));
        // Where the note is let go.
        let x = x_at(wa + wd + ws);
        v.painter.vline(x, inner.y_range(), Stroke::new(1.0 * z, alpha(color, 0.35)));
    }

    // -----------------------------------------------------------------------
    // SYSTEM: MIDI and the Trig inputs
    // -----------------------------------------------------------------------

    fn system(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        self.title("SYSTEM", PAD, W - PAD, SYS_Y, theme::TEXT_DIM);
        let c = self.c;
        let learn = fj::int(p, data::LEARN, 0, 5);
        let [y0, y1] = [SYS_ROWS[0], SYS_ROWS[0] + 16.0];
        let mid = (y0 + y1) * 0.5;
        // The edited channel's MIDI note and Trig input.
        v.text(PAD, mid, Align2::LEFT_CENTER, format!("CH {}", c + 1), theme::strong(8.0 * z), self.accent);
        v.text(48.0, mid, Align2::LEFT_CENTER, "NOTE", v.font(7.5), theme::TEXT_FAINT);
        let note = fj::note(p, c);
        let shown = if learn == c as i32 + 2 { "Lrn…".to_owned() } else { format!("{note} {}", note_label(note as i32)) };
        if let Some(n) = self.drag_box("note", v.r(76.0, y0, 130.0, y1), note as f32, (0.0, 127.0, fj::NOTES[c] as f32), &shown, "The MIDI note that plays this channel (drag; double-click: its General MIDI note)", true) {
            p.data[data::NOTE + c].set(n);
        }
        let learning = learn == c as i32 + 2;
        if self.chip("learn-note", v.r(134.0, y0, 166.0, y1), "LRN", learning, RED, "Learn: the next note played in sets this channel's note").clicked() {
            p.data[data::LEARN].set(if learning { 0.0 } else { c as f32 + 2.0 });
        }
        v.text(180.0, mid, Align2::LEFT_CENTER, "TRIG TYPE", v.font(7.5), theme::TEXT_FAINT);
        let kind = fj::int(p, data::TRIG_TYPE + c, 0, 5) as usize;
        if let Some(t) = self.keys("trigtype", v.r(230.0, y0, 446.0, y1), TYPES.len(), Some(kind), self.accent, 7.5, |t| TYPES[t].0.to_owned(), |t| format!("{}: {}", TYPES[t].0, TYPES[t].1)) {
            p.data[data::TRIG_TYPE + c].set(t as f32);
        }
        let curve = fj::int(p, data::DYN + c, 0, 1) as usize;
        const CURVE_TIPS: [&str; 2] = ["Dn1: velocity follows the strike linearly", "Dn2: a hotter response, soft strikes play louder (also for MIDI notes)"];
        if let Some(d) = self.keys("dyn", v.r(452.0, y0, 524.0, y1), 2, Some(curve), self.accent, 7.5, |d| CURVES[d].to_owned(), |d| CURVE_TIPS[d].to_owned()) {
            p.data[data::DYN + c].set(d as f32);
        }
        let thres = p.data[data::THRES + c].get();
        v.text(536.0, mid, Align2::LEFT_CENTER, "THRES", v.font(7.5), theme::TEXT_FAINT);
        let tip_thres = format!("Threshold: hits quieter than {:.0} dB under the hardest are ignored (drag; double-click: 10)", -trigger::threshold_db(thres));
        if let Some(n) = self.drag_box("thres", v.r(568.0, y0, 600.0, y1), thres, (0.0, 99.0, 10.0), &format!("{thres:.0}"), &tip_thres, true) {
            p.data[data::THRES + c].set(n);
        }
        let sens = p.data[data::SENS + c].get();
        v.text(612.0, mid, Align2::LEFT_CENTER, "SENS", v.font(7.5), theme::TEXT_FAINT);
        let tip_sens = format!("Sensitivity: an input at {:.0} dBFS plays the hardest hit (drag; double-click: 30)", trigger::full_db(sens));
        if let Some(n) = self.drag_box("sens", v.r(640.0, y0, 672.0, y1), sens, (0.0, 99.0, 30.0), &format!("{sens:.0}"), &tip_sens, true) {
            p.data[data::SENS + c].set(n);
        }
        v.text(684.0, mid, Align2::LEFT_CENTER, format!("TRIG {} in → CH {}", c + 1, c + 1), v.font(7.0), theme::TEXT_FAINT);

        // The whole node: MIDI channel, Local, program change.
        let [y0, y1] = [SYS_ROWS[1], SYS_ROWS[1] + 16.0];
        let mid = (y0 + y1) * 0.5;
        v.text(PAD, mid, Align2::LEFT_CENTER, "MIDI CH", v.font(7.5), theme::TEXT_FAINT);
        let mc = p.data[data::MIDI_CH].get();
        let shown = if learn == 1 {
            "Lrn…".to_owned()
        } else if mc < 0.5 {
            "Off".to_owned()
        } else {
            format!("{mc:.0}")
        };
        if let Some(n) = self.drag_box("midich", v.r(56.0, y0, 92.0, y1), mc, (0.0, 16.0, 10.0), &shown, "Channel received and sent (0: Off; drag; double-click: 10)", true) {
            p.data[data::MIDI_CH].set(n);
        }
        let learning = learn == 1;
        if self.chip("learn-ch", v.r(96.0, y0, 128.0, y1), "LRN", learning, RED, "Learn: the next message played in sets the channel").clicked() {
            p.data[data::LEARN].set(if learning { 0.0 } else { 1.0 });
        }
        let local = fj::on(p, data::LOCAL);
        if self.chip("local", v.r(140.0, y0, 210.0, y1), if local { "LOCAL ON" } else { "LOCAL OFF" }, local, self.accent, "Local Off: the panel, the Trig inputs and the steps only go out as MIDI notes (for a sequencer echoing them back)").clicked() {
            p.data[data::LOCAL].set(if local { 0.0 } else { 1.0 });
        }
        v.text(224.0, mid, Align2::LEFT_CENTER, "PROG CHANGE", v.font(7.5), theme::TEXT_FAINT);
        const PC: [&str; 4] = ["OFF", "SEND", "RECV", "BOTH"];
        const PC_TIPS: [&str; 4] = ["Program changes: none", "Send a program change when a program is picked here", "Receive program changes: they load programs", "Send and receive"];
        let pc = fj::int(p, data::PROG_CHANGE, 0, 3) as usize;
        if let Some(k) = self.keys("pc", v.r(290.0, y0, 446.0, y1), 4, Some(pc), self.accent, 7.5, |k| PC[k].to_owned(), |k| PC_TIPS[k].to_owned()) {
            p.data[data::PROG_CHANGE].set(k as f32);
        }
        let follow = fj::on(p, data::FOLLOW);
        if self.chip("follow", v.r(458.0, y0, 552.0, y1), "FOLLOW PADS", follow, self.accent, "MIDI notes and Trig input hits pick the channel to edit (picking one on the panel turns this off)").clicked() {
            p.data[data::FOLLOW].set(if follow { 0.0 } else { 1.0 });
        }
        v.text(564.0, mid, Align2::LEFT_CENTER, "Hits from TRIG, the Trig inputs and the steps go out as notes.", v.font(7.0), theme::TEXT_FAINT);
    }

    // -----------------------------------------------------------------------
    // STEPS: the sequencer
    // -----------------------------------------------------------------------

    fn sequencer(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.text(PAD, SEQ_Y, Align2::LEFT_CENTER, "STEPS", theme::strong(8.5 * z), theme::TEXT_DIM);
        let [y0, y1] = [SEQ_Y - 8.0, SEQ_Y + 8.0];
        let run = fj::on(p, data::RUN);
        let playing = p.viz[viz::STEP].get() >= 0.0;
        if self.chip("run", v.r(56.0, y0, 104.0, y1), if run { "▶ RUN" } else { "RUN" }, run, GREEN, "Play the lanes with the board's transport").clicked() {
            p.data[data::RUN].set(if run { 0.0 } else { 1.0 });
        }
        let rate = fj::int(p, data::RATE, 0, DIVISIONS.len() as i32 - 1) as usize;
        let current = fj::RATES.iter().position(|&r| r == rate);
        if let Some(k) = self.keys("rate", v.r(112.0, y0, 272.0, y1), fj::RATES.len(), current, self.accent, 7.5, |k| DIVISIONS[fj::RATES[k]].0.to_owned(), |k| format!("Steps of {}", DIVISIONS[fj::RATES[k]].0)) {
            p.data[data::RATE].set(fj::RATES[k] as f32);
        }
        let len = fj::length(p);
        if let Some(n) = self.drag_box("length", v.r(280.0, y0, 352.0, y1), len as f32, (1.0, MAX_LEN as f32, 16.0), &format!("‹ {len} STEPS ›"), "Steps in the pattern (drag; double-click: 16)", true) {
            p.data[data::LENGTH].set(n);
        }
        let swing = p.get(fj::SWING);
        if let Some(n) = self.drag_box("swing", v.r(358.0, y0, 430.0, y1), (swing * 100.0).round(), (0.0, 100.0, 0.0), &format!("SWING {:.0}%", swing * 100.0), "Swing: every other step late, up to half a step (drag; double-click: 0)", true) {
            p.set(fj::SWING, n / 100.0);
        }
        v.hit(v.r(358.0, y0, 430.0, y1), ParamRef::Module(fj::SWING));
        let gate = p.data[data::GATE].get();
        if let Some(n) = self.drag_box("gate", v.r(436.0, y0, 500.0, y1), gate, (5.0, 100.0, 50.0), &format!("GATE {gate:.0}%"), "How long a step's note is held, in % of the step: the ADSRs' sustain, and the MIDI notes sent (drag; double-click: 50%)", true) {
            p.data[data::GATE].set(n);
        }
        if self.chip("clear", v.r(506.0, y0, 554.0, y1), "CLEAR", false, theme::MUTE, "Clear every step").clicked() {
            for c in 0..CHANNELS {
                for s in 0..MAX_LEN {
                    fj::set_step(p, c, s, 0);
                }
            }
        }
        let state = if !run {
            "off".to_owned()
        } else if playing {
            format!("step {} of {len}", p.viz[viz::STEP].get() as usize + 1)
        } else {
            "waiting for the transport".to_owned()
        };
        v.text(564.0, SEQ_Y, Align2::LEFT_CENTER, state, v.font(7.5), if playing { theme::TEXT_DIM } else { theme::TEXT_FAINT });
        v.text(W - PAD, SEQ_Y, Align2::RIGHT_CENTER, "click on/off · right-click soft, medium, accent · drag paints", v.font(7.0), theme::TEXT_FAINT);

        // The lanes.
        let x0 = 92.0;
        let cell = (W - PAD - x0) / MAX_LEN as f32;
        let step = p.viz[viz::STEP].get();
        let per_beat = DIVISIONS[rate].1.round().max(1.0) as usize;
        for c in 0..CHANNELS {
            let ry = LANES_TOP + LANE_H * c as f32;
            let lane = v.r(PAD, ry + 2.0, 60.0, ry + LANE_H - 2.0);
            let resp = v
                .ui
                .interact(lane, self.uid(("lane", c)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Click: edit this channel; double-click: play it. Below: what ALEA takes it for");
            let selected = c == self.c;
            text(v.painter, lane.left_center() - vec2(0.0, 5.0 * z), Align2::LEFT_CENTER, format!("CH {}", c + 1), theme::font(8.0 * z), if selected { self.accent } else { theme::TEXT_DIM });
            text(v.painter, lane.left_center() + vec2(0.0, 6.0 * z), Align2::LEFT_CENTER, alea::role(&fj::sound(p, c)).name(), theme::font(6.5 * z), theme::TEXT_FAINT);
            if resp.clicked() {
                self.pick(c);
            }
            if resp.double_clicked() {
                self.trig(c);
            }
            let muted = fj::on(p, data::MUTE + c);
            if self.chip(("mute", c), v.r(62.0, ry + 9.0, 86.0, ry + LANE_H - 9.0), "M", muted, theme::MUTE, "Mute this lane").clicked() {
                p.data[data::MUTE + c].set(if muted { 0.0 } else { 1.0 });
            }
            for s in 0..MAX_LEN {
                let x = x0 + cell * s as f32;
                let r = v.r(x + 1.5, ry + 3.0, x + cell - 1.5, ry + LANE_H - 3.0);
                let used = s < len;
                let level = fj::step(p, c, s);
                let color = if muted { theme::TEXT_FAINT } else { self.accent };
                let ink = |a: f32| if used { alpha(color, a) } else { alpha(color, a * 0.3) };
                let beat = s % (per_beat * 4).max(1) == 0;
                v.painter.rect_stroke(r, 2.0 * z, Stroke::new(1.0 * z, if used { if beat { theme::CONTROL_HI } else { theme::CONTROL } } else { alpha(theme::CONTROL, 0.4) }), StrokeKind::Inside);
                if level > 0 {
                    let h = r.height() * level as f32 / 3.0;
                    let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - h), r.max).shrink(2.0 * z);
                    v.painter.rect_filled(bar, 1.5 * z, ink(1.0));
                }
                if step >= 0.0 && used && step as usize == s {
                    v.painter.rect_stroke(r.expand(1.0 * z), 2.5 * z, Stroke::new(1.5 * z, theme::TEXT), StrokeKind::Outside);
                }
            }
        }
        // Beat marks over the lanes.
        for s in (0..MAX_LEN).step_by(per_beat.max(1)) {
            let x = x0 + cell * s as f32;
            v.painter.vline(v.p(x, 0.0).x, v.r(0.0, LANES_TOP - 2.0, 0.0, LANES_TOP + 1.0).y_range(), Stroke::new(1.0 * z, theme::TEXT_FAINT));
        }
        // Click a step to switch it, right-click for its level, drag to
        // paint.
        let grid = v.r(x0, LANES_TOP, x0 + cell * MAX_LEN as f32, LANES_TOP + LANE_H * CHANNELS as f32);
        let resp = v.ui.interact(grid, self.uid("grid"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
        let paint_id = self.uid("paint");
        let cell_at = |q: Pos2| {
            let s = ((q.x - grid.left()) / (cell * z)).floor();
            let c = ((q.y - grid.top()) / (LANE_H * z)).floor();
            (s >= 0.0 && c >= 0.0 && (s as usize) < MAX_LEN && (c as usize) < CHANNELS).then_some((c as usize, s as usize))
        };
        if let Some(q) = resp.interact_pointer_pos()
            && let Some((c, s)) = cell_at(q)
        {
            let level = fj::step(p, c, s);
            let shift = v.ui.input(|i| i.modifiers.shift);
            if resp.clicked() && !shift {
                fj::set_step(p, c, s, if level > 0 { 0 } else { 2 });
            }
            if resp.secondary_clicked() || (resp.clicked() && shift) {
                fj::set_step(p, c, s, level % 3 + 1);
            }
            if resp.drag_started() {
                v.ui.data_mut(|d| d.insert_temp(paint_id, if level > 0 { 0usize } else { 2 }));
            }
            if resp.dragged()
                && let Some(value) = v.ui.data(|d| d.get_temp::<usize>(paint_id))
            {
                fj::set_step(p, c, s, value);
            }
        }
        if let Some(q) = resp.hover_pos()
            && let Some((c, s)) = cell_at(q)
        {
            const LEVELS: [&str; 4] = ["off", "soft", "medium", "accent"];
            resp.on_hover_text(format!("CH {} · step {} · {}", c + 1, s + 1, LEVELS[fj::step(p, c, s)]));
        }
    }
}

/// Text, unless it is too small to read (zoomed far out).
fn text(painter: &egui::Painter, at: Pos2, align: Align2, s: impl ToString, font: egui::FontId, color: Color32) -> Rect {
    if font.size < MIN_TEXT {
        return Rect::from_center_size(at, Vec2::ZERO);
    }
    painter.text(at, align, s, font, color)
}

/// A die as line art: a rounded square and the pips of `face` (1…6).
fn die(painter: &egui::Painter, r: Rect, face: u8, color: Color32, z: f32) {
    painter.rect_stroke(r, 4.0 * z, Stroke::new(1.5 * z, color), StrokeKind::Inside);
    let (c, d) = (r.center(), r.width() * 0.26);
    let at = |dx: f32, dy: f32| pos2(c.x + dx * d, c.y + dy * d);
    let pips: &[(f32, f32)] = match face {
        1 => &[(0.0, 0.0)],
        2 => &[(-1.0, -1.0), (1.0, 1.0)],
        3 => &[(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)],
        4 => &[(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)],
        5 => &[(-1.0, -1.0), (1.0, -1.0), (0.0, 0.0), (-1.0, 1.0), (1.0, 1.0)],
        _ => &[(-1.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (1.0, 1.0)],
    };
    for &(dx, dy) in pips {
        painter.circle_filled(at(dx, dy), 1.7 * z, color);
    }
}
