//! Panel of Splitter, flat and graphic: line art with coloured outlines.
//!
//! Along the top, always there: the source (LOAD a WAV or AIFF, its name,
//! length and rate, × to unload), CUT (an equal GRID of 4 to 32, at the
//! TRANSIENTS with their SENS, at RANDOM), the display's ZOOM, DICE (the
//! die, CHANCE, throws for the slices, the rhythms or both, and BACK) and
//! LEVEL.
//!
//! Then the display: the selected slice's source (the sample, or the atom
//! it plays) with every slice of that source outlined in its own colour,
//! the playheads and the grains moving over them, and above it the whole
//! source in a strip, with the part shown. Drag across empty space to cut a
//! new slice, drag a slice's edge to move it, drag inside the selected
//! slice to move it whole; click selects, right-click deletes. Under it the
//! slice keys, 1 to 32, flashing when a slice plays.
//!
//! Three pages below. SLICE: the selected slice's source, how it plays,
//! SOUND (pitch, gain, pan, attack and release, drawn as an envelope),
//! STUTTER (the ratchet, drawn as its strikes), CRUSH (bits and hold, drawn
//! as a stepped sine) and GRAINS (drawn as a cloud). SEQUENCE: the play
//! modes, RUN, LOCK, the rate and length, the knobs that shape every mode,
//! and the mode's own view: the step lane, the probability bars, or a ring
//! per slice for its Euclidean rhythm. MIDI: the channel, CHROMA, MIDI OUT,
//! a keyboard showing (and setting) which note plays which slice, and LEARN.

use std::sync::Arc;

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{MIN_TEXT, NodeView};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::splitter::atoms::{self, ATOM_LEN, ATOMS};
use crate::audio::modules::splitter::dice::{self, Before};
use crate::audio::modules::splitter::voice::{self, nearest_density};
use crate::audio::modules::splitter::{
    self as sp, BITS, FIELDS, MAX_LEN, MAX_SLICES, MODES, PLAYS, RATES, REPEATS, REPEAT_RATES, VOICES, data, slice, slicing, viz,
};
use crate::audio::params::note_label;
use crate::audio::sample::Sample;
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(920.0, 596.0);
const W: f32 = SIZE.x;
const H: f32 = SIZE.y;
const PAD: f32 = 12.0;

// The top.
const SOURCE_X: f32 = 48.0;
const CUT_X: f32 = 318.0;
const DICE_X: [f32; 2] = [606.0, 812.0];
const LEVEL_X: f32 = 852.0;
// The display.
const STRIP_Y: [f32; 2] = [80.0, 94.0];
const DISPLAY_Y: [f32; 2] = [98.0, 250.0];
const KEYS_Y: [f32; 2] = [256.0, 276.0];
const KEYS_X: [f32; 2] = [PAD, 836.0];
const TABS_Y: [f32; 2] = [284.0, 300.0];
/// Where the pages start.
const PAGE_Y: f32 = 308.0;

/// How long a slice key flashes after a hit, and the die tumbles.
const FLASH: f64 = 0.12;
const TUMBLE: f64 = 0.45;
/// How long a message (a load, a cut) stays.
const NOTICE: f64 = 4.0;

/// The slices' colours, in turn.
const COLORS: [Color32; 8] = [
    Color32::from_rgb(255, 196, 87),
    Color32::from_rgb(72, 219, 251),
    Color32::from_rgb(255, 107, 157),
    Color32::from_rgb(126, 231, 135),
    Color32::from_rgb(167, 139, 250),
    Color32::from_rgb(250, 232, 110),
    Color32::from_rgb(255, 140, 105),
    Color32::from_rgb(96, 165, 238),
];
const GREEN: Color32 = Color32::from_rgb(110, 220, 120);
const RED: Color32 = Color32::from_rgb(236, 84, 70);

pub fn color(s: usize) -> Color32 {
    COLORS[s % COLORS.len()]
}

/// A source's short name: SAMPLE or the atom's.
fn source_name(source: usize) -> &'static str {
    if source == 0 { "SAMPLE" } else { ATOMS[(source - 1).min(ATOMS.len() - 1)].name }
}

/// The page shown: 0 SLICE, 1 SEQUENCE, 2 MIDI.
pub fn page(p: &ModuleParams) -> usize {
    sp::int(p, data::TAB, 0, 2) as usize
}

pub fn size(_p: &ModuleParams) -> Vec2 {
    SIZE
}

/// Collapsed nodes' readout: the sample (or the atoms), the slices and the
/// mode.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let main = match p.sample.get() {
        Some(s) => s.name.clone(),
        None => "built-in atoms".to_owned(),
    };
    let playing = if p.viz[viz::STEP].get() >= 0.0 { " · playing" } else { "" };
    (main, format!("{} slices · {} · {}{playing}", sp::count(p), MODES[sp::mode(p)].to_lowercase(), RATES[sp::rate(p)].0))
}

fn short_seconds(s: f32) -> String {
    if s >= 1.0 {
        format!("{s:.2} s")
    } else if s >= 0.01 {
        format!("{:.0} ms", s * 1000.0)
    } else {
        format!("{:.1} ms", s * 1000.0)
    }
}

/// A slice setting as the panel writes it.
fn value_text(k: usize, v: f32, sync: bool, tempo: f32) -> String {
    match k {
        slice::PITCH | slice::RAMP if v > 0.0 => format!("+{v:.0} st"),
        slice::PITCH | slice::RAMP => format!("{v:.0} st"),
        slice::GAIN if v > 0.0 => format!("+{v:.1} dB"),
        slice::GAIN => format!("{v:.1} dB"),
        slice::PAN if v.abs() < 0.02 => "C".to_owned(),
        slice::PAN if v < 0.0 => format!("L{:.0}", -v * 100.0),
        slice::PAN => format!("R{:.0}", v * 100.0),
        slice::ATTACK | slice::RELEASE | slice::SIZE => short_seconds(v * 0.001),
        slice::FALL | slice::PROB => format!("{v:.0}%"),
        slice::POS | slice::SPRAY => format!("{:.0}%", v * 100.0),
        slice::DOWN if v < 1.05 => "off".to_owned(),
        slice::DOWN => format!("×{v:.1}"),
        slice::DENSITY if sync => {
            let per_beat = nearest_density(v / (tempo.max(1.0) / 60.0));
            density_name(per_beat)
        }
        slice::DENSITY => format!("{v:.0}/s"),
        slice::JITTER => format!("±{v:.1} st"),
        _ => format!("{v:.0}"),
    }
}

/// Grains a beat as a note value ("1/16", "1/32T").
fn density_name(per_beat: f32) -> String {
    let quarter = per_beat * 4.0;
    if (per_beat as u32).is_multiple_of(3) {
        format!("1/{:.0}T", quarter / 1.5)
    } else {
        format!("1/{quarter:.0}")
    }
}

fn tip(k: usize) -> &'static str {
    match k {
        slice::PITCH => "Pitch in semitones: a sample slice plays faster and higher (or slower and lower); an atom keeps its length (double-click: 0)",
        slice::GAIN => "The slice's level",
        slice::PAN => "Where the slice sits, left to right (the node's SPREAD scatters around it)",
        slice::ATTACK => "How long the slice takes to rise",
        slice::RELEASE => "How long it takes to fall: after the key or the step's GATE, or as a fade before a ONE slice's end",
        slice::FALL => "How much quieter each repeat is",
        slice::RAMP => "How far each repeat moves in pitch: up a semitone, a fifth, an octave…",
        slice::DOWN => "Sample-and-hold: each value held for this many samples (lo-fi, aliasing)",
        slice::SIZE => "The grains' length",
        slice::DENSITY => "Grains a second (with SYNC: grains on the beat grid, the nearest note value)",
        slice::POS => "Where the playhead starts in the slice, or stands when FROZEN",
        slice::SPRAY => "How far grains scatter from the playhead",
        slice::JITTER => "How far grains scatter in pitch, either way (12: octaves apart)",
        _ => "",
    }
}

/// Knobs over a slice setting: whether they turn on a log scale.
fn log_knob(k: usize) -> bool {
    matches!(k, slice::ATTACK | slice::RELEASE | slice::SIZE | slice::DENSITY | slice::DOWN)
}

fn to_norm(k: usize, v: f32) -> f32 {
    let fl = &FIELDS[k];
    if log_knob(k) {
        let lo = fl.lo.max(0.1);
        ((v.max(lo) / lo).ln() / (fl.hi / lo).ln()).clamp(0.0, 1.0)
    } else {
        ((v - fl.lo) / (fl.hi - fl.lo)).clamp(0.0, 1.0)
    }
}

fn from_norm(k: usize, n: f32) -> f32 {
    let fl = &FIELDS[k];
    let v = if log_knob(k) {
        let lo = fl.lo.max(0.1);
        lo * (fl.hi / lo).powf(n.clamp(0.0, 1.0))
    } else {
        fl.lo + (fl.hi - fl.lo) * n.clamp(0.0, 1.0)
    };
    match k {
        slice::PITCH | slice::RAMP => v.round(),
        slice::GAIN => (v * 2.0).round() / 2.0,
        _ => v,
    }
}

/// What a press on the display is doing.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    /// Moving slice `s`'s start (true) or end.
    Edge(usize, bool),
    /// Moving slice `s` whole, grabbed at this share of the source from its
    /// start.
    Move(usize, f32),
    /// Cutting a new slice from this share.
    Cut(f32),
}

/// An atom's waveform as drawn: the atom, its pitch, the range shown (in
/// millionths).
type AtomKey = (usize, i32, u32, u32);

/// What the panel remembers between frames.
#[derive(Clone, Default)]
struct View {
    /// Per slice: hits seen and when the last came.
    hits: Vec<(f32, f64)>,
    drag: Option<Drag>,
    /// The cut being drawn: from, to (shares).
    cutting: Option<(f32, f32)>,
    face: u8,
    thrown: Option<f64>,
    before: Option<Before>,
    /// A slice's settings, copied.
    clip: Option<Vec<f32>>,
    notice: Option<(String, f64, bool)>,
    /// The notes received, and when the last came.
    notes: (f32, f64),
    /// An atom's waveform for the display, and what it was drawn for.
    atom_wave: Option<(AtomKey, Vec<[f32; 2]>)>,
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
    sample: Option<Arc<Sample>>,
    /// The selected slice (None: there are none).
    s: Option<usize>,
}

/// `out`: the output is patched; `midi`: the MIDI input and output are.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, out: bool, (midi_in, midi_out): (bool, bool)) {
    let p = &m.params;
    p.sample.collect();
    let key = Id::unique(("splitter-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    view.hits.resize(MAX_SLICES, (0.0, f64::NEG_INFINITY));
    for (s, h) in view.hits.iter_mut().enumerate() {
        let n = p.viz[viz::HITS + s].get();
        if n != h.0 {
            *h = (n, now);
        }
    }
    let notes = p.viz[viz::NOTES_IN].get();
    if notes != view.notes.0 {
        view.notes = (notes, now);
    }
    let count = sp::count(p);
    let s = Panel { v, id, p, accent, now, sample: p.sample.get(), s: (count > 0).then(|| sp::selected(p)) };

    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi_in));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(out));
    v.text(W - PAD, 60.0, Align2::RIGHT_CENTER, "MIDI", v.font(9.0), col(midi_out));

    s.source_block(&mut view);
    s.cut_block(&mut view);
    s.dice_block(&mut view);
    s.level();
    s.strip();
    s.display(&mut view);
    s.slice_keys(&mut view);
    s.tabs();
    match page(p) {
        0 => s.slice_page(&mut view),
        1 => s.sequence_page(),
        _ => s.midi_page(&view),
    }
    // PLAY can't be held on another page (or with no slices).
    if (page(p) != 0 || s.s.is_none()) && p.viz[viz::HOLD].get() > 0.5 {
        p.viz[viz::HOLD].set(0.0);
    }
    // The tabs' row: the latest message, or how the display works.
    match &view.notice {
        Some((text, at, error)) if now - at < NOTICE => {
            v.text(W - PAD, TABS_Y[0] + 8.0, Align2::RIGHT_CENTER, text, v.font(7.5), if *error { RED } else { theme::TEXT_DIM });
        }
        _ => {
            let hint = "display: drag to cut a slice · drag an edge or the selected slice to move it · right-click deletes · double-click plays";
            v.text(W - PAD, TABS_Y[0] + 8.0, Align2::RIGHT_CENTER, hint, v.font(6.5), theme::TEXT_FAINT);
        }
    }

    let busy = p.viz[viz::VOICES_ON].get() > 0.0
        || p.viz[viz::STEP].get() >= 0.0
        || view.thrown.is_some_and(|t| now - t < TUMBLE)
        || view.hits.iter().any(|h| now - h.1 < FLASH)
        || view.notice.as_ref().is_some_and(|n| now - n.1 < NOTICE);
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
        Id::unique(("splitter", self.id)).with(salt)
    }

    /// The selected slice's setting `k`.
    fn get(&self, k: usize) -> f32 {
        self.s.map_or(FIELDS[k].default, |s| sp::get(self.p, s, k))
    }

    fn set(&self, k: usize, v: f32) {
        if let Some(s) = self.s
            && v != sp::get(self.p, s, k)
        {
            sp::set(self.p, s, k, v);
        }
    }

    fn source(&self) -> usize {
        self.get(slice::SOURCE).round().max(0.0) as usize
    }

    /// A small button.
    fn chip(&self, salt: impl std::hash::Hash + std::fmt::Debug, rect: Rect, label: &str, lit: bool, color: Color32, tip: &str) -> Response {
        widgets::toggle(self.v.ui, self.v.painter, self.uid(("chip", salt)), rect, label, lit, color, self.z()).on_hover_text(tip)
    }

    /// A row of keys across `rect`, `current` lit in `color`; returns the
    /// key clicked.
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

    /// A value box: drag up or right to change it, double-click to reset.
    #[allow(clippy::too_many_arguments)]
    fn drag_box(&self, salt: &str, rect: Rect, value: f32, (lo, hi, reset, per_px): (f32, f32, f32, f32), shown: &str, tip: &str, on: bool) -> Option<f32> {
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
                out = Some((value + steps * per_px).clamp(lo, hi));
            }
        }
        if resp.double_clicked() {
            out = Some(reset);
        }
        let hot = resp.hovered() || resp.dragged();
        v.painter.rect_filled(rect, 3.0 * z, if hot { theme::CONTROL_HI } else { theme::INSET });
        v.painter.rect_stroke(rect, 3.0 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
        text(v.painter, rect.center(), Align2::CENTER_CENTER, shown, v.mono(7.5), if on { theme::TEXT } else { theme::TEXT_FAINT });
        out
    }

    /// A section's title and the line after it.
    fn title(&self, title: &str, x0: f32, x1: f32, y: f32, color: Color32) {
        let (v, z) = (self.v, self.z());
        let r = v.text(x0, y, Align2::LEFT_CENTER, title, theme::strong(8.5 * z), color);
        let from = (r.right() + 8.0 * z).max(v.p(x0 + 40.0, 0.0).x);
        if from < v.p(x1, 0.0).x {
            v.painter.hline(from..=v.p(x1, 0.0).x, v.p(0.0, y).y, Stroke::new(1.0 * z, alpha(color, 0.3)));
        }
    }

    /// A knob over the selected slice's setting `k` at `cx`: its label at
    /// `y`, the knob, its value.
    fn knob(&self, k: usize, cx: f32, y: f32, r: f32, color: Color32) {
        let (v, z) = (self.v, self.z());
        let idle = self.s.is_none();
        let color = if idle { theme::TEXT_FAINT } else { color };
        v.text(cx, y, Align2::CENTER_CENTER, FIELDS[k].name, v.font(7.5), theme::TEXT_FAINT);
        let top = y + 7.0;
        let value = self.get(k);
        let mut norm = to_norm(k, value);
        let fl = &FIELDS[k];
        let resp = widgets::knob(v.ui, v.painter, self.uid(("knob", k)), v.p(cx, top + r), r * z, &mut norm, to_norm(k, fl.default), fl.lo < 0.0 && fl.hi > 0.0, color, z).on_hover_text(tip(k));
        if resp.dragged() || resp.double_clicked() {
            self.set(k, from_norm(k, norm));
        }
        let hot = resp.dragged() || resp.hovered();
        let sync = self.get(slice::SYNC) > 0.5;
        v.text(cx, top + 2.0 * r + 7.0, Align2::CENTER_CENTER, value_text(k, self.get(k), sync, v.tempo), v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }

    /// A knob over one of the node's own (mappable) knobs.
    fn param_knob(&self, i: usize, cx: f32, y: f32, r: f32, label: &str, shown: String) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = sp::spec(i);
        v.text(cx, y, Align2::CENTER_CENTER, label, v.font(7.5), theme::TEXT_FAINT);
        let top = y + 7.0;
        let mut norm = spec.to_norm(p.get(i));
        let resp = widgets::knob(v.ui, v.painter, self.uid(("param", i)), v.p(cx, top + r), r * z, &mut norm, spec.to_norm(spec.default), spec.bipolar, self.accent, z);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            p.set(i, spec.denorm(norm));
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * r + 7.0, Align2::CENTER_CENTER, shown, v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }

    fn notify(&self, view: &mut View, text: impl Into<String>, error: bool) {
        view.notice = Some((text.into(), self.now, error));
    }

    fn select(&self, s: usize) {
        self.p.data[data::SELECTED].set(s as f32);
    }

    /// Plays the selected slice (PLAY, a double-click).
    fn audition(&self) {
        let at = viz::AUDITION;
        self.p.viz[at].set(self.p.viz[at].get() + 1.0);
    }

    fn seed(&self, salt: u32) -> u32 {
        ((self.now * 1000.0) as u32) ^ self.id.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B)
    }

    // -----------------------------------------------------------------------
    // The top
    // -----------------------------------------------------------------------

    /// LOAD, the sample's name and what it is, × to let it go.
    fn source_block(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let x0 = SOURCE_X;
        let load = self.chip("load", v.r(x0, 34.0, x0 + 52.0, 52.0), "LOAD", false, self.accent, "Load a sample: WAV or AIFF, up to 30 s, kept inside the project");
        if load.clicked() {
            let picked = rfd::FileDialog::new().set_title("Load a sample").add_filter("Audio", &["wav", "wave", "aif", "aiff", "aifc"]).pick_file();
            if let Some(path) = picked {
                match Sample::load(&path) {
                    Ok(sample) => {
                        let trimmed = sample.trimmed;
                        let sample = Arc::new(sample);
                        p.sample.set(Some(sample.clone()));
                        let has_sample_slices = (0..sp::count(p)).any(|s| sp::get(p, s, slice::SOURCE) < 0.5);
                        let mut msg = format!("Loaded {}", sample.name);
                        if !has_sample_slices {
                            let regions = slicing::transients(&sample, p.data[data::SENS].get() / 100.0, 16);
                            slicing::apply(p, &regions, 0);
                            msg = format!("{msg} · cut at {} transients", regions.len());
                        }
                        if trimmed {
                            msg.push_str(" · cut to 30 s");
                        }
                        self.notify(view, msg, false);
                    }
                    Err(e) => self.notify(view, format!("Could not load it: {e}"), true),
                }
            }
        }
        let (name, info) = match &self.sample {
            Some(s) => (s.name.clone(), format!("{} · {:.1} kHz · {}", short_seconds(s.seconds()), s.rate / 1000.0, if s.channels() == 2 { "stereo" } else { "mono" })),
            None => ("no sample".to_owned(), "the eight atoms play".to_owned()),
        };
        let shown = clip_text(&name, 30);
        v.text(x0 + 60.0, 40.0, Align2::LEFT_CENTER, shown, theme::strong(9.0 * z), if self.sample.is_some() { theme::TEXT } else { theme::TEXT_FAINT });
        v.text(x0 + 60.0, 52.0, Align2::LEFT_CENTER, info, v.font(7.0), theme::TEXT_FAINT);
        if self.sample.is_some() && self.chip("unload", v.r(x0, 56.0, x0 + 52.0, 70.0), "EJECT", false, theme::MUTE, "Let the sample go (its slices fall silent until another is loaded)").clicked() {
            p.sample.set(None);
            self.notify(view, "Sample let go", false);
        }
        let counted = sp::count(p);
        v.text(x0 + 60.0, 64.0, Align2::LEFT_CENTER, format!("{counted} of {MAX_SLICES} slices"), v.font(7.0), theme::TEXT_FAINT);
    }

    /// CUT: GRID, TRANSIENTS (with SENS), RANDOM; ZOOM.
    fn cut_block(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let x0 = CUT_X;
        v.text(x0, 42.0, Align2::LEFT_CENTER, "CUT", theme::strong(8.5 * z), theme::TEXT_DIM);
        let has = self.sample.is_some();
        const GRIDS: [usize; 4] = [4, 8, 16, 32];
        if let Some(k) = self.keys("grid", v.r(x0 + 30.0, 34.0, x0 + 146.0, 50.0), 4, None, self.accent, 7.5, |k| format!("÷{}", GRIDS[k]), |k| format!("Cut the sample into {} equal slices", GRIDS[k])) {
            if has {
                slicing::apply(p, &slicing::grid(GRIDS[k]), 0);
                self.notify(view, format!("Cut into {} equal slices", GRIDS[k]), false);
            } else {
                self.notify(view, "Load a sample to cut it", true);
            }
        }
        if self.chip("trans", v.r(x0 + 152.0, 34.0, x0 + 212.0, 50.0), "TRANSIENT", false, self.accent, "Cut the sample where it strikes (SENS: how readily)").clicked() {
            match &self.sample {
                Some(sample) => {
                    let regions = slicing::transients(sample, p.data[data::SENS].get() / 100.0, MAX_SLICES);
                    slicing::apply(p, &regions, 0);
                    self.notify(view, format!("Cut at {} transients", regions.len()), false);
                }
                None => self.notify(view, "Load a sample to cut it", true),
            }
        }
        if self.chip("rand", v.r(x0 + 218.0, 34.0, x0 + 278.0, 50.0), "RANDOM", false, self.accent, "Cut 16 short fragments at random places (8 to 400 ms)").clicked() {
            match &self.sample {
                Some(sample) => {
                    slicing::apply(p, &slicing::random(16, self.seed(7), sample.seconds()), 0);
                    self.notify(view, "Cut 16 random fragments", false);
                }
                None => self.notify(view, "Load a sample to cut it", true),
            }
        }
        let sens = p.data[data::SENS].get();
        if let Some(n) = self.drag_box("sens", v.r(x0 + 152.0, 54.0, x0 + 212.0, 70.0), sens, (0.0, 100.0, 50.0, 1.0), &format!("SENS {sens:.0}%"), "How readily TRANSIENT cuts: higher finds softer strikes (drag; double-click: 50%)", true) {
            p.data[data::SENS].set(n);
        }
        let zoom = sp::on(p, data::ZOOM);
        const ZOOMS: [&str; 2] = ["ALL", "SLICE"];
        if let Some(k) = self.keys("zoom", v.r(x0 + 30.0, 54.0, x0 + 146.0, 70.0), 2, Some(zoom as usize), theme::TEXT_DIM, 7.5, |k| ZOOMS[k].to_owned(), |k| ["Show the whole source", "Zoom on the selected slice"][k].to_owned()) {
            p.data[data::ZOOM].set(k as f32);
        }
        v.text(x0, 62.0, Align2::LEFT_CENTER, "VIEW", v.font(7.0), theme::TEXT_FAINT);
        if self.chip("clear", v.r(x0 + 218.0, 54.0, x0 + 278.0, 70.0), "CLEAR", false, theme::MUTE, "Remove every slice").clicked() {
            view.before = Some(Before::take(p));
            p.data[data::COUNT].set(0.0);
            p.data[data::SELECTED].set(0.0);
            self.notify(view, "Slices cleared · BACK brings them back", false);
        }
    }

    /// DICE: the die, CHANCE, the throws and BACK.
    fn dice_block(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [x0, x1] = DICE_X;
        let tumbling = view.thrown.is_some_and(|t| self.now - t < TUMBLE);
        let face = if tumbling { ((self.now * 30.0) as u64 % 6) as u8 + 1 } else { view.face.max(1) };
        die(v.painter, v.r(x0, 34.0, x0 + 20.0, 54.0), face, if view.thrown.is_some() { self.accent } else { theme::TEXT_DIM }, z);
        v.text(x0 + 26.0, 40.0, Align2::LEFT_CENTER, "DICE", theme::strong(9.5 * z), theme::TEXT);
        v.text(x0 + 26.0, 50.0, Align2::LEFT_CENTER, "throw for new ones", v.font(6.5), theme::TEXT_FAINT);
        let chance = p.data[data::CHANCE].get();
        if let Some(n) = self.drag_box("chance", v.r(x1 - 66.0, 36.0, x1, 52.0), chance, (1.0, 100.0, 50.0, 1.0), &format!("CHANCE {chance:.0}%"), "How far a throw goes: a little nudges what is there, 100% starts afresh (drag; double-click: 50%)", true) {
            p.data[data::CHANCE].set(n);
        }
        let amount = chance.clamp(1.0, 100.0) / 100.0;
        let w = (x1 - x0 - 50.0 - 12.0) / 3.0;
        let shift = v.ui.input(|i| i.modifiers.shift);
        let throws: [(&str, &str); 3] = [
            ("SLICES", "New settings for every slice: pitch by pure intervals, ratchets, crushing, grains (Shift: the selected slice only)"),
            ("RHYTHM", "New rhythms: each slice's Euclidean pattern and chance, and the pattern's steps, made for what each slice sounds like"),
            ("ALL", "New slice settings, then rhythms for them"),
        ];
        let mut thrown = None;
        for (i, (label, tip)) in throws.into_iter().enumerate() {
            let x = x0 + (w + 4.0) * i as f32;
            if self.chip(("dice", i), v.r(x, 56.0, x + w, 70.0), label, false, self.accent, tip).clicked() {
                thrown = Some(i);
            }
        }
        if let Some(i) = thrown {
            let seed = self.seed(i as u32 + 1);
            view.before = Some(Before::take(p));
            let all: Vec<usize> = (0..sp::count(p)).collect();
            let slices = match (shift, self.s) {
                (true, Some(s)) => vec![s],
                _ => all,
            };
            let sample = self.sample.as_deref();
            if i != 1 {
                dice::throw_slices(p, sample, seed, amount, &slices);
            }
            if i != 0 {
                dice::throw_rhythm(p, sample, seed, amount);
            }
            view.face = (seed % 6) as u8 + 1;
            view.thrown = Some(self.now);
        }
        let back = self.chip("back", v.r(x1 - 46.0, 56.0, x1, 70.0), "BACK", view.before.is_some(), theme::TEXT_DIM, "Take back the last throw (or CLEAR)");
        if back.clicked()
            && let Some(before) = view.before.take()
        {
            before.restore(p);
        }
    }

    fn level(&self) {
        let p = self.p;
        self.param_knob(sp::LEVEL, LEVEL_X, 36.0, 12.0, "LEVEL", format!("{:.0}%", p.get(sp::LEVEL) * 100.0));
    }

    // -----------------------------------------------------------------------
    // The display
    // -----------------------------------------------------------------------

    /// The part of the source shown: shares, from and to.
    fn shown_range(&self) -> (f32, f32) {
        if !sp::on(self.p, data::ZOOM) {
            return (0.0, 1.0);
        }
        let Some(_) = self.s else { return (0.0, 1.0) };
        let (a, b) = (self.get(slice::START).min(self.get(slice::END)), self.get(slice::START).max(self.get(slice::END)));
        let pad = ((b - a) * 0.5).max(0.001);
        let (mut lo, mut hi) = (a - pad, b + pad);
        if lo < 0.0 {
            hi -= lo;
            lo = 0.0;
        }
        if hi > 1.0 {
            lo = (lo - (hi - 1.0)).max(0.0);
            hi = 1.0;
        }
        (lo, hi)
    }

    /// The displayed source's columns: per column the lowest and highest
    /// value, across the share range `from`…`to`.
    fn columns(&self, view: &mut View, source: usize, from: f32, to: f32, n: usize) -> Vec<[f32; 2]> {
        if source == 0 {
            let Some(sample) = &self.sample else { return Vec::new() };
            let frames = sample.frames();
            if frames < 2 {
                return Vec::new();
            }
            let (a, b) = ((from * frames as f32) as usize, ((to * frames as f32) as usize).min(frames));
            let per = (b - a) as f32 / n as f32;
            // Far out: from the overview; close in: from the samples.
            if per > frames as f32 / sample.overview.len() as f32 {
                let bins = sample.overview.len();
                return (0..n)
                    .map(|c| {
                        let lo = ((from + (to - from) * c as f32 / n as f32) * bins as f32) as usize;
                        let hi = (((from + (to - from) * (c + 1) as f32 / n as f32) * bins as f32) as usize).max(lo + 1).min(bins);
                        sample.overview[lo.min(bins - 1)..hi].iter().fold([f32::MAX, f32::MIN], |m, x| [m[0].min(x[0]), m[1].max(x[1])])
                    })
                    .collect();
            }
            let right = sample.right();
            return (0..n)
                .map(|c| {
                    let lo = a + (c as f32 * per) as usize;
                    let hi = (a + ((c + 1) as f32 * per) as usize).max(lo + 1).min(frames);
                    let from = lo.min(frames - 1);
                    let mut m = [f32::MAX, f32::MIN];
                    for (l, r) in sample.left[from..hi].iter().zip(&right[from..hi]) {
                        let y = (l + r) * 0.5;
                        m = [m[0].min(y), m[1].max(y)];
                    }
                    if per < 1.0 {
                        // Fewer frames than columns: draw the line between them.
                        let y = sample.left[lo.min(frames - 1)];
                        m = [m[0].min(y), m[1].max(y)];
                    }
                    m
                })
                .collect();
        }
        let atom = (source - 1).min(ATOMS.len() - 1);
        let pitch = (self.get(slice::PITCH) + self.p.get(sp::PITCH)).round() as i32;
        let key = (atom, pitch, (from * 1e6) as u32, (to * 1e6) as u32);
        if let Some((k, cols)) = &view.atom_wave
            && *k == key
            && cols.len() == n
        {
            return cols.clone();
        }
        let hz = atoms::hz(atom, pitch as f32);
        const SUB: usize = 6;
        let cols: Vec<[f32; 2]> = (0..n)
            .map(|c| {
                let mut m = [f32::MAX, f32::MIN];
                for j in 0..=SUB {
                    let share = from + (to - from) * (c as f32 + j as f32 / SUB as f32) / n as f32;
                    let y = atoms::atom(atom, share as f64 * ATOM_LEN, hz, 0x1234);
                    m = [m[0].min(y), m[1].max(y)];
                }
                m
            })
            .collect();
        view.atom_wave = Some((key, cols.clone()));
        cols
    }

    /// The whole source in a strip, the slices as ticks, the part shown.
    fn strip(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(PAD, STRIP_Y[0], W - PAD, STRIP_Y[1]);
        v.painter.rect_filled(rect, 2.0 * z, theme::INSET);
        let source = self.source();
        if source == 0
            && let Some(sample) = &self.sample
        {
            let n = (rect.width() / (2.0 * z)).max(8.0) as usize;
            let bins = sample.overview.len();
            for c in 0..n {
                let lo = c * bins / n;
                let hi = ((c + 1) * bins / n).max(lo + 1).min(bins);
                let m = sample.overview[lo..hi].iter().fold(0.0f32, |m, x| m.max(x[0].abs()).max(x[1].abs()));
                let x = rect.left() + rect.width() * (c as f32 + 0.5) / n as f32;
                let h = (m * rect.height() * 0.45).max(0.5 * z);
                v.painter.vline(x, (rect.center().y - h)..=(rect.center().y + h), Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.6)));
            }
        } else {
            text(v.painter, rect.center(), Align2::CENTER_CENTER, format!("{} · 1 s", source_name(source)), theme::font(6.5 * z), theme::TEXT_FAINT);
        }
        let x_of = |share: f32| rect.left() + rect.width() * share;
        for s in (0..sp::count(p)).filter(|&s| sp::get_int(p, s, slice::SOURCE) as usize == source) {
            let (a, b) = (sp::get(p, s, slice::START).min(sp::get(p, s, slice::END)), sp::get(p, s, slice::START).max(sp::get(p, s, slice::END)));
            let r = Rect::from_min_max(pos2(x_of(a), rect.bottom() - 3.0 * z), pos2(x_of(b).max(x_of(a) + 1.5 * z), rect.bottom()));
            v.painter.rect_filled(r, 0.0, color(s));
        }
        let (lo, hi) = self.shown_range();
        if hi - lo < 0.999 {
            let r = Rect::from_min_max(pos2(x_of(lo), rect.top()), pos2(x_of(hi).max(x_of(lo) + 2.0 * z), rect.bottom()));
            v.painter.rect_stroke(r, 1.5 * z, Stroke::new(1.2 * z, theme::TEXT_DIM), StrokeKind::Inside);
        }
    }

    /// The selected slice's source, its slices, the playheads and grains;
    /// cutting and moving slices.
    fn display(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(PAD, DISPLAY_Y[0], W - PAD, DISPLAY_Y[1]);
        v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let inner = rect.shrink2(vec2(4.0 * z, 6.0 * z));
        let source = self.source();
        let (lo, hi) = self.shown_range();
        let x_of = |share: f32| inner.left() + inner.width() * (share - lo) / (hi - lo);
        let share_at = |x: f32| (lo + (x - inner.left()) / inner.width() * (hi - lo)).clamp(0.0, 1.0);
        let mid = inner.center().y;
        let amp = inner.height() * 0.46;
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));

        // Grid: eighths of what is shown.
        for k in 1..8 {
            let x = inner.left() + inner.width() * k as f32 / 8.0;
            painter.vline(x, inner.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
        }
        painter.hline(inner.x_range(), mid, Stroke::new(1.0 * z, theme::GRID_MAJOR));

        // The waveform: a stroke per column, its outline over it.
        let n = (inner.width() / (1.5 * z)).clamp(32.0, 1400.0) as usize;
        let cols = self.columns(view, source, lo, hi, n);
        let empty = cols.is_empty();
        if empty {
            let msg = if source == 0 { "LOAD a sample (WAV or AIFF) — or give this slice an atom: SINE, PING, CLICK…" } else { "" };
            text(&painter, inner.center(), Align2::CENTER_CENTER, msg, theme::font(8.5 * z), theme::TEXT_FAINT);
        } else {
            let colw = inner.width() / n as f32;
            let ink = alpha(self.accent, 0.35);
            let mut top = Vec::with_capacity(n);
            let mut bottom = Vec::with_capacity(n);
            for (c, m) in cols.iter().enumerate() {
                let x = inner.left() + colw * (c as f32 + 0.5);
                let (y0, y1) = (mid - m[1].clamp(-1.0, 1.0) * amp, mid - m[0].clamp(-1.0, 1.0) * amp);
                painter.vline(x, y0..=y1.max(y0 + 0.6 * z), Stroke::new(colw.max(1.0 * z), ink));
                top.push(pos2(x, y0));
                bottom.push(pos2(x, y1));
            }
            painter.add(Shape::line(top, Stroke::new(1.0 * z, alpha(self.accent, 0.9))));
            painter.add(Shape::line(bottom, Stroke::new(1.0 * z, alpha(self.accent, 0.9))));
        }

        // The slices of this source.
        let count = sp::count(p);
        let ours: Vec<usize> = (0..count).filter(|&s| sp::get_int(p, s, slice::SOURCE) as usize == source).collect();
        let span = |s: usize| (sp::get(p, s, slice::START).min(sp::get(p, s, slice::END)), sp::get(p, s, slice::START).max(sp::get(p, s, slice::END)));
        for (row, &s) in ours.iter().enumerate() {
            let (a, b) = span(s);
            let (xa, xb) = (x_of(a), x_of(b).max(x_of(a) + 2.0 * z));
            let selected = self.s == Some(s);
            let flash = self.now - view.hits.get(s).map_or(f64::NEG_INFINITY, |h| h.1) < FLASH;
            let c = color(s);
            let r = Rect::from_min_max(pos2(xa, inner.top() - 2.0 * z), pos2(xb, inner.bottom() + 2.0 * z));
            painter.rect_filled(r, 2.0 * z, alpha(c, if flash { 0.28 } else if selected { 0.13 } else { 0.06 }));
            painter.rect_stroke(r, 2.0 * z, Stroke::new(if selected { 2.0 } else { 1.0 } * z, if selected { c } else { alpha(c, 0.7) }), StrokeKind::Inside);
            if selected {
                for x in [xa, xb] {
                    painter.rect_filled(Rect::from_center_size(pos2(x, mid), vec2(5.0 * z, 14.0 * z)), 1.5 * z, c);
                }
            }
            // The number, staggered when slices crowd.
            let ty = inner.top() + (6.0 + 10.0 * (row % 3) as f32) * z;
            if xb - xa > 12.0 * z || selected {
                text(&painter, pos2(xa + 3.0 * z, ty), Align2::LEFT_CENTER, (s + 1).to_string(), theme::strong(7.5 * z), c);
            }
        }

        // Playheads and grains.
        for i in 0..VOICES {
            let base = viz::HEADS + 3 * i;
            let sl = p.viz[base].get();
            if sl < 0.0 || sl as usize >= count {
                continue;
            }
            let sl = sl as usize;
            if sp::get_int(p, sl, slice::SOURCE) as usize != source {
                continue;
            }
            let x = x_of(p.viz[base + 1].get());
            let level = p.viz[base + 2].get().clamp(0.0, 1.0);
            painter.vline(x, inner.y_range(), Stroke::new(1.5 * z, alpha(color(sl), 0.5 + 0.5 * level.sqrt())));
        }
        for g in 0..viz::SHOWN_GRAINS {
            let sl = p.viz[viz::GRAINS + 2 * g + 1].get();
            if sl < 0.0 || sl as usize >= count || sp::get_int(p, sl as usize, slice::SOURCE) as usize != source {
                continue;
            }
            let x = x_of(p.viz[viz::GRAINS + 2 * g].get());
            let y = mid + amp * 0.7 * (((g * 7919) % 13) as f32 / 6.0 - 1.0);
            painter.circle_stroke(pos2(x, y), 3.0 * z, Stroke::new(1.2 * z, color(sl as usize)));
        }

        // Pressing on the display.
        let resp = v.ui.interact(rect, self.uid("display"), Sense::click_and_drag());
        let grab = 6.0 * z;
        let edge_at = |q: Pos2| -> Option<(usize, bool)> {
            // The selected slice first, then the others.
            let order: Vec<usize> = self.s.into_iter().filter(|s| ours.contains(s)).chain(ours.iter().copied().filter(|s| Some(*s) != self.s)).collect();
            for s in order {
                let (a, b) = span(s);
                if (q.x - x_of(a)).abs() < grab {
                    return Some((s, true));
                }
                if (q.x - x_of(b)).abs() < grab {
                    return Some((s, false));
                }
            }
            None
        };
        let inside = |q: Pos2| -> Option<usize> {
            let share = share_at(q.x);
            let hits: Vec<usize> = ours.iter().copied().filter(|&s| span(s).0 <= share && share <= span(s).1).collect();
            if let Some(s) = self.s.filter(|s| hits.contains(s)) {
                return Some(s);
            }
            hits.into_iter().min_by(|&x, &y| (span(x).1 - span(x).0).total_cmp(&(span(y).1 - span(y).0)))
        };
        if let Some(q) = resp.hover_pos() {
            let cursor = if edge_at(q).is_some() {
                CursorIcon::ResizeHorizontal
            } else if inside(q).is_some_and(|s| Some(s) == self.s) {
                CursorIcon::Grab
            } else {
                CursorIcon::Crosshair
            };
            v.ui.ctx().set_cursor_icon(cursor);
            painter.vline(q.x, inner.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT_DIM, 0.4)));
            let share = share_at(q.x);
            let label = if source == 0 {
                self.sample.as_ref().map_or(String::new(), |smp| short_seconds(share * smp.seconds()))
            } else {
                short_seconds(share)
            };
            text(&painter, pos2(q.x + 4.0 * z, inner.bottom() - 6.0 * z), Align2::LEFT_CENTER, label, theme::mono(7.0 * z), theme::TEXT_DIM);
        }
        let usable = !empty || source != 0;
        if resp.drag_started()
            && usable
            && let Some(q) = resp.interact_pointer_pos()
        {
            let shift = v.ui.input(|i| i.modifiers.shift);
            view.drag = if shift {
                Some(Drag::Cut(share_at(q.x)))
            } else if let Some((s, start)) = edge_at(q) {
                self.select(s);
                Some(Drag::Edge(s, start))
            } else if let Some(s) = inside(q).filter(|s| Some(*s) == self.s) {
                Some(Drag::Move(s, share_at(q.x) - span(s).0))
            } else {
                Some(Drag::Cut(share_at(q.x)))
            };
        }
        if resp.dragged()
            && let (Some(q), Some(drag)) = (resp.interact_pointer_pos(), view.drag)
        {
            let at = share_at(q.x);
            match drag {
                Drag::Edge(s, start) => {
                    let (a, b) = span(s);
                    if start {
                        sp::set(p, s, slice::START, at.min(b - 0.0001));
                        sp::set(p, s, slice::END, b);
                    } else {
                        sp::set(p, s, slice::START, a);
                        sp::set(p, s, slice::END, at.max(a + 0.0001));
                    }
                }
                Drag::Move(s, grab_at) => {
                    let (a, b) = span(s);
                    let len = b - a;
                    let start = (at - grab_at).clamp(0.0, 1.0 - len);
                    sp::set(p, s, slice::START, start);
                    sp::set(p, s, slice::END, start + len);
                }
                Drag::Cut(from) => view.cutting = Some((from.min(at), from.max(at))),
            }
        }
        if let Some((a, b)) = view.cutting {
            let r = Rect::from_min_max(pos2(x_of(a), inner.top()), pos2(x_of(b), inner.bottom()));
            painter.rect_stroke(r, 2.0 * z, Stroke::new(1.5 * z, theme::TEXT), StrokeKind::Inside);
        }
        if resp.drag_stopped() {
            if let (Some(Drag::Cut(_)), Some((a, b))) = (view.drag, view.cutting)
                && (x_of(b) - x_of(a)) > 3.0 * z
                && sp::add_slice(p, a, b, source).is_none()
            {
                self.notify(view, "All 32 slices are in use", true);
            }
            view.drag = None;
            view.cutting = None;
        }
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && let Some(s) = inside(q)
        {
            self.select(s);
        }
        if resp.double_clicked() && inside(resp.interact_pointer_pos().unwrap_or_default()).is_some() {
            self.audition();
        }
        if resp.secondary_clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && let Some(s) = inside(q)
        {
            view.before = Some(Before::take(p));
            sp::delete_slice(p, s);
            self.notify(view, format!("Slice {} deleted · BACK brings it back", s + 1), false);
        }
        let label = match (source, &self.sample) {
            (0, Some(s)) => format!("SAMPLE · {}", short_seconds(s.seconds() * (hi - lo))),
            (0, None) => "SAMPLE".to_owned(),
            (k, _) => format!("ATOM {} · {}", source_name(k), short_seconds(hi - lo)),
        };
        if 7.0 * z >= MIN_TEXT {
            let galley = v.painter.layout_no_wrap(label, v.font(7.0), theme::TEXT_DIM);
            let at = pos2(rect.left() + 6.0 * z, rect.bottom() - 6.0 * z - galley.size().y);
            v.painter.rect_filled(Rect::from_min_size(at - vec2(3.0 * z, 1.0 * z), galley.size() + vec2(6.0 * z, 2.0 * z)), 2.0 * z, alpha(theme::INSET, 0.85));
            v.painter.galley(at, galley, theme::TEXT_DIM);
        }
    }

    /// The slice keys, 1–32, and + / DEL.
    fn slice_keys(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let count = sp::count(p);
        let [x0, x1] = KEYS_X;
        let [y0, y1] = KEYS_Y;
        let kw = (x1 - x0) / MAX_SLICES as f32;
        for s in 0..MAX_SLICES {
            let r = v.r(x0 + kw * s as f32 + 1.0, y0, x0 + kw * (s + 1) as f32 - 1.0, y1);
            let used = s < count;
            let c = color(s);
            let resp = v.ui.interact(r, self.uid(("key", s)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            let selected = self.s == Some(s);
            let flash = used && self.now - view.hits.get(s).map_or(f64::NEG_INFINITY, |h| h.1) < FLASH;
            if used {
                let muted = sp::flag(p, s, slice::MUTE);
                let fill = if selected {
                    c
                } else if flash {
                    alpha(c, 0.45)
                } else if resp.hovered() {
                    theme::CONTROL_HI
                } else {
                    theme::NODE_BODY
                };
                v.painter.rect_filled(r, 2.5 * z, fill);
                v.painter.rect_stroke(r, 2.5 * z, Stroke::new(1.2 * z, if muted { alpha(c, 0.3) } else { c }), StrokeKind::Inside);
                text(v.painter, r.center(), Align2::CENTER_CENTER, (s + 1).to_string(), theme::font(7.5 * z), if selected { theme::INSET } else if muted { theme::TEXT_FAINT } else { theme::TEXT });
                let tip = format!("Slice {} · {}{} · click: edit · double-click: play", s + 1, source_name(sp::get_int(p, s, slice::SOURCE) as usize), if muted { " · muted" } else { "" });
                let resp = resp.on_hover_text(tip);
                if resp.clicked() {
                    self.select(s);
                }
                if resp.double_clicked() {
                    self.select(s);
                    self.audition();
                }
            } else {
                v.painter.rect_stroke(r, 2.5 * z, Stroke::new(1.0 * z, alpha(theme::CONTROL, 0.6)), StrokeKind::Inside);
            }
        }
        let add = self.chip("add", v.r(x1 + 6.0, y0, x1 + 34.0, y1), "+", false, self.accent, "A new slice: a copy of the selected one (or a SINE)");
        if add.clicked() {
            let made = match self.s {
                Some(s) => sp::add_slice(p, 0.0, 0.25, 0).inspect(|&t| sp::copy_slice(p, s, t)),
                None => sp::add_slice(p, 0.0, 0.25, 1 + atoms::SINE),
            };
            if made.is_none() {
                self.notify(view, "All 32 slices are in use", true);
            }
        }
        if self.chip("del", v.r(x1 + 38.0, y0, W - PAD, y1), "DEL", false, theme::MUTE, "Delete the selected slice").clicked()
            && let Some(s) = self.s
        {
            view.before = Some(Before::take(p));
            sp::delete_slice(p, s);
        }
    }

    fn tabs(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        const PAGES: [&str; 3] = ["SLICE", "SEQUENCE", "MIDI"];
        const TIPS: [&str; 3] = [
            "The selected slice: its source, sound, stutter, crusher and grains",
            "How the slices are played on the board's grid: the modes, the steps, the rhythms",
            "Which notes play which slices, and what goes out",
        ];
        let [y0, y1] = TABS_Y;
        if let Some(t) = self.keys("page", v.r(PAD, y0, 300.0, y1), 3, Some(page(p)), self.accent, 8.0, |t| PAGES[t].to_owned(), |t| TIPS[t].to_owned()) {
            p.data[data::TAB].set(t as f32);
        }
        v.painter.hline(v.r(308.0, 0.0, 340.0, 0.0).x_range(), v.p(0.0, (y0 + y1) * 0.5).y, Stroke::new(1.0 * z, theme::GRID_MAJOR));
    }

    // -----------------------------------------------------------------------
    // SLICE
    // -----------------------------------------------------------------------

    fn slice_page(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let Some(s) = self.s else {
            text(v.painter, v.p(W * 0.5, PAGE_Y + 120.0), Align2::CENTER_CENTER, "No slices: drag across the display to cut one, or press +", theme::font(9.0 * z), theme::TEXT_FAINT);
            return;
        };
        let c = color(s);
        let y = PAGE_Y;
        v.text(PAD, y + 6.0, Align2::LEFT_CENTER, format!("SLICE {}", s + 1), theme::strong(10.0 * z), c);
        let role = slicing::role(p, s, self.sample.as_deref());
        v.text(PAD, y + 17.0, Align2::LEFT_CENTER, format!("sounds like: {}", role.name()), v.font(6.5), theme::TEXT_FAINT);

        // The source.
        let source = self.source();
        if let Some(k) = self.keys(
            "source",
            v.r(84.0, y, 520.0, y + 18.0),
            sp::SOURCES,
            Some(source),
            c,
            7.5,
            |k| source_name(k).to_owned(),
            |k| if k == 0 { "The loaded sample".to_owned() } else { ATOMS[k - 1].tip.to_owned() },
        ) {
            self.set(slice::SOURCE, k as f32);
            if k > 0 && source == 0 {
                // An atom's second: start with its first quarter.
                self.set(slice::START, 0.0);
                self.set(slice::END, 0.25);
            }
        }
        // How it plays, the way round, PLAY.
        let play = self.get(slice::PLAY).round() as usize;
        const PLAY_TIPS: [&str; 3] = ["Plays once, through its release (a note-off changes nothing)", "Plays while held: a key, a step's GATE, PLAY", "Loops while held: micro-loops and stutters"];
        if let Some(k) = self.keys("play", v.r(528.0, y, 648.0, y + 18.0), 3, Some(play.min(2)), c, 7.5, |k| PLAYS[k].to_owned(), |k| PLAY_TIPS[k].to_owned()) {
            self.set(slice::PLAY, k as f32);
        }
        let rev = self.get(slice::REVERSE) > 0.5;
        if self.chip("rev", v.r(654.0, y, 694.0, y + 18.0), "REV", rev, c, "Played backwards").clicked() {
            self.set(slice::REVERSE, if rev { 0.0 } else { 1.0 });
        }
        let audition = self.chip("audition", v.r(700.0, y, 754.0, y + 18.0), "▶ PLAY", false, GREEN, "Play the slice (held: a GATE or LOOP slice keeps playing)");
        if audition.is_pointer_button_down_on() {
            if p.viz[viz::HOLD].get() < 0.5 {
                p.viz[viz::HOLD].set(1.0);
                self.audition();
            }
        } else if p.viz[viz::HOLD].get() > 0.5 {
            p.viz[viz::HOLD].set(0.0);
        }
        let muted = self.get(slice::MUTE) > 0.5;
        if self.chip("mute", v.r(760.0, y, 788.0, y + 18.0), "M", muted, theme::MUTE, "Leave this slice out of the animation (MIDI still plays it)").clicked() {
            self.set(slice::MUTE, if muted { 0.0 } else { 1.0 });
        }
        if self.chip("copy", v.r(794.0, y, 836.0, y + 18.0), "COPY", false, theme::TEXT_DIM, "Copy this slice's settings").clicked() {
            view.clip = Some((0..slice::LEN).map(|k| sp::get(p, s, k)).collect());
        }
        if self.chip("paste", v.r(840.0, y, 882.0, y + 18.0), "PASTE", false, theme::TEXT_DIM, "Paste the copied settings here (the note stays)").clicked()
            && let Some(clip) = &view.clip
        {
            view.before = Some(Before::take(p));
            for (k, val) in clip.iter().enumerate().filter(|(k, _)| *k != slice::NOTE) {
                sp::set(p, s, k, *val);
            }
        }
        if self.chip("init", v.r(886.0, y, W - PAD, y + 18.0), "↺", false, theme::TEXT_DIM, "Back to the plain settings (region, source and note stay)").clicked() {
            view.before = Some(Before::take(p));
            sp::reset_slice(p, s);
        }

        // SOUND.
        let y1 = y + 32.0;
        self.title("SOUND", PAD, 300.0, y1, theme::TEXT_DIM);
        let ky = y1 + 12.0;
        let r = 13.0;
        for (i, k) in [slice::PITCH, slice::GAIN, slice::PAN, slice::ATTACK, slice::RELEASE].into_iter().enumerate() {
            self.knob(k, 36.0 + 56.0 * i as f32, ky, r, c);
        }
        self.region_boxes(s, ky + 52.0);
        self.envelope_picture(v.r(PAD, ky + 74.0, 300.0, ky + 118.0), c);

        // STUTTER.
        let x0 = 314.0;
        let x1 = 600.0;
        self.title("STUTTER", x0, x1, y1, theme::TEXT_DIM);
        let repeat = self.get(slice::REPEAT);
        let current = REPEATS.iter().position(|&r| r == repeat.round());
        if let Some(k) = self.keys("repeat", v.r(x0, ky, x1, ky + 16.0), REPEATS.len(), current, c, 7.5, |k| format!("{}×", REPEATS[k]), |k| if k == 0 { "One strike: no ratchet".to_owned() } else { format!("{} strikes on the grid each time it plays", REPEATS[k]) }) {
            self.set(slice::REPEAT, REPEATS[k]);
        }
        let rate = self.get(slice::REP_RATE).round() as usize;
        if let Some(k) = self.keys("reprate", v.r(x0, ky + 20.0, x1, ky + 36.0), REPEAT_RATES.len(), Some(rate.min(REPEAT_RATES.len() - 1)), alpha(c, 0.8), 7.0, |k| REPEAT_RATES[k].0.to_owned(), |k| format!("Repeats every {} of the board's tempo", REPEAT_RATES[k].0)) {
            self.set(slice::REP_RATE, k as f32);
        }
        self.knob(slice::FALL, x0 + 24.0, ky + 46.0, 12.0, c);
        self.knob(slice::RAMP, x0 + 76.0, ky + 46.0, 12.0, c);
        self.ratchet_picture(v.r(x0 + 110.0, ky + 46.0, x1, ky + 92.0), c);

        // CRUSH.
        let x0 = 614.0;
        let x1 = W - PAD;
        self.title("CRUSH", x0, x1, y1, theme::TEXT_DIM);
        let bits = self.get(slice::BITS).round();
        let current = BITS.iter().position(|&b| b == bits);
        if let Some(k) = self.keys("bits", v.r(x0, ky, x1, ky + 16.0), BITS.len(), current, c, 7.5, |k| if k == 0 { "16".to_owned() } else { format!("{}", BITS[k]) }, |k| if k == 0 { "16 bits: clean".to_owned() } else { format!("{} bits of resolution", BITS[k]) }) {
            self.set(slice::BITS, BITS[k]);
        }
        v.text(x0, ky + 28.0, Align2::LEFT_CENTER, "bits of resolution, then the hold", v.font(6.5), theme::TEXT_FAINT);
        self.knob(slice::DOWN, x0 + 24.0, ky + 46.0, 12.0, c);
        let crush = p.get(sp::CRUSH);
        self.param_knob(sp::CRUSH, x0 + 76.0, ky + 46.0, 12.0, "CRUSH ALL", format!("{:.0}%", crush * 100.0));
        self.crush_picture(v.r(x0 + 110.0, ky + 46.0, x1, ky + 92.0), c);

        // GRAINS.
        let y2 = ky + 132.0;
        self.title("GRAINS", PAD, W - PAD, y2, theme::TEXT_DIM);
        let gy = y2 + 12.0;
        let grain = self.get(slice::GRAIN) > 0.5;
        if self.chip("grain", v.r(PAD, gy, 70.0, gy + 18.0), "GRAIN", grain, c, "Play the slice as a cloud of grains (the playhead moves in real time: the pitch no longer changes the length)").clicked() {
            self.set(slice::GRAIN, if grain { 0.0 } else { 1.0 });
        }
        let freeze = self.get(slice::FREEZE) > 0.5;
        if self.chip("freeze", v.r(PAD, gy + 22.0, 70.0, gy + 40.0), "FREEZE", freeze, c, "The playhead stands still at POS: a frozen sound").clicked() {
            self.set(slice::FREEZE, if freeze { 0.0 } else { 1.0 });
        }
        let sync = self.get(slice::SYNC) > 0.5;
        if self.chip("sync", v.r(PAD, gy + 44.0, 70.0, gy + 62.0), "SYNC", sync, c, "Grains on the board's beat grid: DENSITY snaps to the nearest note value").clicked() {
            self.set(slice::SYNC, if sync { 0.0 } else { 1.0 });
        }
        for (i, k) in [slice::SIZE, slice::DENSITY, slice::POS, slice::SPRAY, slice::JITTER].into_iter().enumerate() {
            self.knob(k, 104.0 + 58.0 * i as f32, gy, 14.0, if grain { c } else { theme::TEXT_FAINT });
        }
        let spread = p.get(sp::SPREAD);
        self.param_knob(sp::SPREAD, 104.0 + 58.0 * 5.0, gy, 14.0, "SPREAD", format!("{:.0}%", spread * 100.0));
        self.cloud_picture(v.r(470.0, gy, W - PAD, H - 12.0), c, grain);
    }

    /// START and END as times, dragged finely.
    fn region_boxes(&self, s: usize, y: f32) {
        let (v, p) = (self.v, self.p);
        let seconds = if sp::get(p, s, slice::SOURCE) < 0.5 { self.sample.as_ref().map_or(1.0, |smp| smp.seconds()) } else { 1.0 };
        let (a, b) = (sp::get(p, s, slice::START), sp::get(p, s, slice::END));
        let step = (0.001 / seconds).max(1e-6);
        let shown = |x: f32| short_seconds(x * seconds);
        if let Some(n) = self.drag_box("start", v.r(PAD, y, 104.0, y + 16.0), a, (0.0, b - step, a, step), &format!("FROM {}", shown(a)), "Where the slice starts (drag: a millisecond a step)", true) {
            sp::set(p, s, slice::START, n);
        }
        if let Some(n) = self.drag_box("end", v.r(110.0, y, 202.0, y + 16.0), b, (a + step, 1.0, b, step), &format!("TO {}", shown(b)), "Where the slice ends (drag: a millisecond a step)", true) {
            sp::set(p, s, slice::END, n);
        }
        v.text(208.0, y + 8.0, Align2::LEFT_CENTER, format!("= {}", shown((b - a).abs())), v.mono(7.5), theme::TEXT_DIM);
    }

    /// The envelope over the slice's length: attack, hold, release.
    fn envelope_picture(&self, r: Rect, c: Color32) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(r, 3.0 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
        let inner = r.shrink(5.0 * z);
        let seconds = {
            let span = (self.get(slice::END) - self.get(slice::START)).abs();
            if self.source() == 0 { span * self.sample.as_ref().map_or(1.0, |s| s.seconds()) } else { span }
        };
        let speed = if self.source() == 0 && self.get(slice::GRAIN) < 0.5 { crate::audio::fastmath::exp2(self.get(slice::PITCH) / 12.0) } else { 1.0 };
        let length = (seconds / speed).max(0.001);
        let play = (self.get(slice::PLAY).round() as usize).min(voice::LOOP);
        let one = play == voice::ONE;
        let a = self.get(slice::ATTACK) * 0.001;
        let rel = self.get(slice::RELEASE) * 0.001;
        let total = if one { length } else { length.max(a + rel) * 1.3 };
        let x = |t: f32| inner.left() + inner.width() * (t / total).clamp(0.0, 1.0);
        let y = |g: f32| inner.bottom() - inner.height() * g;
        let hold_end = if one { (length - rel).max(a.min(length)) } else { total - rel };
        let mut pts = vec![pos2(x(0.0), y(0.0)), pos2(x(a.min(hold_end)), y(1.0)), pos2(x(hold_end), y(1.0))];
        // The release, curved (the level squared).
        for k in 1..=12 {
            let f = k as f32 / 12.0;
            pts.push(pos2(x(hold_end + (total.min(hold_end + rel) - hold_end) * f), y((1.0 - f) * (1.0 - f))));
        }
        v.painter.add(Shape::line(pts, Stroke::new(1.5 * z, c)));
        let what = match play {
            voice::ONE => "ONE: to its end",
            voice::GATE => "GATE: while held",
            _ => "LOOP: while held",
        };
        text(v.painter, pos2(r.right() - 5.0 * z, r.bottom() + 7.0 * z), Align2::RIGHT_CENTER, format!("{what} · {}", short_seconds(length)), theme::font(6.5 * z), theme::TEXT_FAINT);
    }

    /// The ratchet's strikes: a line each, shorter as they FALL, higher or
    /// lower as they RAMP.
    fn ratchet_picture(&self, r: Rect, c: Color32) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(r, 3.0 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
        let inner = r.shrink(5.0 * z);
        let n = self.get(slice::REPEAT).round().max(1.0) as usize;
        let fall = self.get(slice::FALL) * 0.01;
        let ramp = self.get(slice::RAMP);
        let mid = inner.center().y;
        let rate = REPEAT_RATES[(self.get(slice::REP_RATE).round() as usize).min(REPEAT_RATES.len() - 1)].1 as f32;
        // The picture spans a beat: ticks at the sixteenths.
        for t in 1..4 {
            let x = inner.left() + inner.width() * t as f32 / 4.0;
            v.painter.vline(x, (inner.bottom() - 4.0 * z)..=inner.bottom(), Stroke::new(1.0 * z, theme::TEXT_FAINT));
        }
        text(v.painter, pos2(r.right() - 5.0 * z, r.bottom() + 7.0 * z), Align2::RIGHT_CENTER, "a beat", theme::font(6.5 * z), theme::TEXT_FAINT);
        let step = inner.width() / rate;
        for k in 0..n {
            let x = inner.left() + 2.0 * z + step * k as f32;
            if x > inner.right() {
                break;
            }
            let g = (1.0 - fall).max(0.0).powi(k as i32);
            let shift = (ramp * k as f32 / 24.0).clamp(-1.0, 1.0) * inner.height() * 0.3;
            let h = inner.height() * 0.42 * g;
            v.painter.line_segment([pos2(x, mid - shift - h), pos2(x, mid - shift + h)], Stroke::new(2.0 * z, alpha(c, 0.4 + 0.6 * g)));
        }
    }

    /// One period of a sine through the crusher.
    fn crush_picture(&self, r: Rect, c: Color32) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(r, 3.0 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
        let inner = r.shrink(5.0 * z);
        let crush = self.p.get(sp::CRUSH);
        let bits = self.get(slice::BITS).min(16.0 - 13.0 * crush);
        let down = self.get(slice::DOWN) * crate::audio::fastmath::exp2(crush * 3.6);
        let q = 2f32.powf(bits.max(1.0) - 1.0);
        let n = 160;
        let mut held = 0.0;
        let mut phase = f32::MAX;
        let mut pts = Vec::with_capacity(n);
        let mut clean = Vec::with_capacity(n);
        for i in 0..n {
            let y = (std::f32::consts::TAU * i as f32 / n as f32).sin();
            phase += 1.0;
            if phase >= down / 2.0 {
                phase = 0.0;
                held = y;
            }
            let out = if bits < 15.5 { (held * q).round() / q } else { held };
            let x = inner.left() + inner.width() * i as f32 / (n - 1) as f32;
            pts.push(pos2(x, inner.center().y - out * inner.height() * 0.45));
            clean.push(pos2(x, inner.center().y - y * inner.height() * 0.45));
        }
        v.painter.add(Shape::line(clean, Stroke::new(1.0 * z, theme::GRID_MAJOR)));
        v.painter.add(Shape::line(pts, Stroke::new(1.5 * z, c)));
    }

    /// The grains as a cloud: arcs scattered around POS by SPRAY, SIZE wide,
    /// as many as DENSITY, high and low by JITTER; the playhead's way.
    fn cloud_picture(&self, r: Rect, c: Color32, on: bool) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(r, 3.0 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
        let inner = r.shrink(6.0 * z);
        let col = if on { c } else { theme::TEXT_FAINT };
        let seconds = {
            let span = (self.get(slice::END) - self.get(slice::START)).abs();
            if self.source() == 0 { span * self.sample.as_ref().map_or(1.0, |s| s.seconds()) } else { span }
        }
        .max(0.001);
        let size = (self.get(slice::SIZE) * 0.001 / seconds).min(1.0);
        let pos = self.get(slice::POS);
        let spray = self.get(slice::SPRAY);
        let jitter = self.get(slice::JITTER) / 12.0;
        let density = self.get(slice::DENSITY);
        let count = ((density * self.get(slice::SIZE) * 0.001 * 6.0).clamp(3.0, 28.0)) as usize;
        let base = inner.bottom();
        let mut rng = crate::audio::modules::Rng::new(0x5EED_C10D);
        for _ in 0..count {
            let at = (pos + rng.bipolar() * spray * 0.5).rem_euclid(1.0);
            let w = (size * inner.width()).max(3.0 * z);
            let lift = (0.5 + rng.bipolar() * jitter.min(1.0) * 0.5) * inner.height() * 0.35;
            let x0 = inner.left() + at * (inner.width() - w);
            let h = inner.height() * 0.55;
            let pts: Vec<Pos2> = (0..=12)
                .map(|k| {
                    let f = k as f32 / 12.0;
                    let win = 0.5 - 0.5 * (std::f32::consts::TAU * f).cos();
                    pos2(x0 + w * f, base - lift - win * h)
                })
                .collect();
            v.painter.add(Shape::line(pts, Stroke::new(1.0 * z, alpha(col, 0.7))));
        }
        // The playhead's way: from POS to the end, or a point when frozen.
        let x = inner.left() + inner.width() * pos;
        if self.get(slice::FREEZE) > 0.5 {
            v.painter.vline(x, inner.y_range(), Stroke::new(1.5 * z, col));
            text(v.painter, pos2(x + 4.0 * z, inner.top() + 5.0 * z), Align2::LEFT_CENTER, "FROZEN", theme::font(6.5 * z), col);
        } else {
            v.painter.line_segment([pos2(x, inner.top() + 3.0 * z), pos2(inner.right(), inner.top() + 3.0 * z)], Stroke::new(1.0 * z, alpha(col, 0.6)));
            v.painter.circle_filled(pos2(x, inner.top() + 3.0 * z), 2.5 * z, col);
        }
        let state = if !on { "GRAIN is off: the slice plays straight" } else if self.get(slice::SYNC) > 0.5 { "grains on the grid" } else { "grains a second" };
        text(v.painter, pos2(inner.right(), inner.bottom() - 4.0 * z), Align2::RIGHT_CENTER, state, theme::font(6.5 * z), theme::TEXT_FAINT);
    }

    // -----------------------------------------------------------------------
    // SEQUENCE
    // -----------------------------------------------------------------------

    fn sequence_page(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let y = PAGE_Y;
        let mode = sp::mode(p);
        const MODE_TIPS: [&str; 6] = [
            "Each step plays the next slice, in order",
            "Each step plays a slice at random (LOCK: the same ones every LENGTH steps)",
            "Each step plays the slice set in the lane below, or rests",
            "Every step, each slice rolls its own dice (its bar below)",
            "Each slice plays its own Euclidean rhythm, on its own length: they drift against each other",
            "The lane's slices (or the slices in order) rolled into ratchets, now and then the last one held",
        ];
        if let Some(k) = self.keys("mode", v.r(PAD, y, 400.0, y + 18.0), MODES.len(), Some(mode), self.accent, 7.5, |k| MODES[k].to_owned(), |k| MODE_TIPS[k].to_owned()) {
            p.data[data::MODE].set(k as f32);
        }
        let run = sp::on(p, data::RUN);
        if self.chip("run", v.r(408.0, y, 454.0, y + 18.0), if run { "▶ RUN" } else { "RUN" }, run, GREEN, "Play with the board's transport").clicked() {
            sp::toggle(p, data::RUN);
        }
        let lock = sp::on(p, data::LOCK);
        if self.chip("lock", v.r(458.0, y, 504.0, y + 18.0), "LOCK", lock, self.accent, "The random choices repeat every LENGTH steps (RANDOM, PROB, STUTTER, CHAOS)").clicked() {
            sp::toggle(p, data::LOCK);
        }
        if self.chip("reseed", v.r(508.0, y, 540.0, y + 18.0), "NEW", false, theme::TEXT_DIM, "New locked random choices").clicked() {
            p.data[data::SEED].set((self.seed(11) >> 8) as f32);
        }
        let rate = sp::rate(p);
        if let Some(k) = self.keys("rate", v.r(548.0, y, 808.0, y + 18.0), RATES.len(), Some(rate), self.accent, 7.0, |k| RATES[k].0.to_owned(), |k| format!("Steps of {}", RATES[k].0)) {
            p.data[data::RATE].set(k as f32);
        }
        let len = sp::length(p);
        if let Some(n) = self.drag_box("length", v.r(814.0, y, W - PAD, y + 18.0), len as f32, (1.0, MAX_LEN as f32, 16.0, 1.0), &format!("‹ {len} STEPS ›"), "Steps in the pattern (drag; double-click: 16)", true) {
            p.data[data::LENGTH].set(n);
        }

        // The knobs every mode shares.
        let ky = y + 30.0;
        let r = 12.0;
        let shown = |i: usize| format!("{:.0}%", p.get(i) * 100.0);
        self.param_knob(sp::FILL, 36.0, ky, r, "FILL", shown(sp::FILL));
        self.param_knob(sp::CHAOS, 88.0, ky, r, "CHAOS", shown(sp::CHAOS));
        self.param_knob(sp::SWING, 140.0, ky, r, "SWING", shown(sp::SWING));
        let pitch = p.get(sp::PITCH);
        self.param_knob(sp::PITCH, 192.0, ky, r, "PITCH", format!("{pitch:+.0} st"));
        let gate = p.data[data::GATE].get();
        if let Some(n) = self.drag_box("gate", v.r(226.0, ky + 8.0, 306.0, ky + 24.0), gate, (5.0, 100.0, 50.0, 1.0), &format!("GATE {gate:.0}%"), "How long a step holds a GATE or LOOP slice, and its MIDI note (drag; double-click: 50%)", true) {
            p.data[data::GATE].set(n);
        }
        let step = p.viz[viz::STEP].get();
        let state = if !run {
            "off".to_owned()
        } else if step >= 0.0 {
            format!("step {} of {len}", step as usize + 1)
        } else {
            "waiting for the transport".to_owned()
        };
        v.text(226.0, ky + 36.0, Align2::LEFT_CENTER, state, v.font(7.5), if step >= 0.0 { theme::TEXT_DIM } else { theme::TEXT_FAINT });
        v.text(320.0, ky + 8.0, Align2::LEFT_CENTER, MODES[mode], theme::strong(11.0 * z), self.accent);
        v.text(320.0, ky + 24.0, Align2::LEFT_CENTER, MODE_TIPS[mode], v.font(7.5), theme::TEXT_DIM);
        let more = match mode {
            sp::PATTERN | sp::STUTTER => "click a step: the selected slice there (again: a rest) · right-click: rest · drag paints",
            sp::PROB => "drag a bar: the slice's chance each step · click: select",
            sp::EUCLID => "click a ring: select · its HITS, STEPS and ROTATE beside · drag a ring up or down: hits",
            _ => "the lane shows what each step played",
        };
        v.text(320.0, ky + 38.0, Align2::LEFT_CENTER, more, v.font(7.0), theme::TEXT_FAINT);

        let area = v.r(PAD, ky + 56.0, W - PAD, H - 12.0);
        match mode {
            sp::PROB => self.prob_bars(area),
            sp::EUCLID => self.rings(area),
            _ => self.lane(area, mode),
        }
    }

    /// The step lane: per step the slice set (PATTERN, STUTTER) or played.
    fn lane(&self, area: Rect, mode: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let len = sp::length(p);
        let count = sp::count(p);
        let cell = area.width() / MAX_LEN as f32;
        let lane = Rect::from_min_max(area.min, pos2(area.right(), area.top() + area.height() * 0.62));
        let step_now = p.viz[viz::STEP].get();
        let per_beat = RATES[sp::rate(p)].1.max(1.0) as usize;
        let editable = matches!(mode, sp::PATTERN | sp::STUTTER);
        for i in 0..MAX_LEN {
            let r = Rect::from_min_max(pos2(lane.left() + cell * i as f32 + 1.5 * z, lane.top()), pos2(lane.left() + cell * (i + 1) as f32 - 1.5 * z, lane.bottom()));
            let used = i < len;
            let beat = i % (per_beat * 4).max(1) == 0;
            v.painter.rect_stroke(r, 2.0 * z, Stroke::new(1.0 * z, if !used { alpha(theme::CONTROL, 0.4) } else if beat { theme::CONTROL_HI } else { theme::CONTROL }), StrokeKind::Inside);
            let set = sp::step(p, i).filter(|&s| s < count);
            let played = p.viz[viz::STEP_SLICE + i].get();
            let shown = if editable { set } else { (played >= 0.0 && (played as usize) < count).then_some(played as usize) };
            if let Some(s) = shown {
                let c = color(s);
                let block = r.shrink(3.0 * z);
                let ink = if used { 1.0 } else { 0.3 };
                v.painter.rect_stroke(block, 2.0 * z, Stroke::new(1.5 * z, alpha(c, ink)), StrokeKind::Inside);
                let fill_h = block.height() * 0.35;
                v.painter.rect_filled(Rect::from_min_max(pos2(block.left(), block.bottom() - fill_h), block.max), 1.5 * z, alpha(c, 0.25 * ink));
                text(v.painter, block.center_top() + vec2(0.0, 9.0 * z), Align2::CENTER_CENTER, (s + 1).to_string(), theme::strong(8.0 * z), alpha(c, ink));
                // In the edit modes, what actually played over it.
                if editable && played >= 0.0 && played as usize != s && (played as usize) < count {
                    v.painter.circle_filled(block.center_bottom() - vec2(0.0, 5.0 * z), 2.5 * z, color(played as usize));
                }
            } else if used {
                v.painter.circle_stroke(r.center(), 2.0 * z, Stroke::new(1.0 * z, theme::TEXT_FAINT));
            }
            if step_now >= 0.0 && used && step_now as usize == i {
                v.painter.rect_stroke(r.expand(1.0 * z), 2.5 * z, Stroke::new(1.5 * z, theme::TEXT), StrokeKind::Outside);
            }
        }
        // Beat numbers under the lane.
        for i in (0..MAX_LEN).step_by(per_beat.max(1)) {
            let x = lane.left() + cell * (i as f32 + 0.5);
            text(v.painter, pos2(x, lane.bottom() + 8.0 * z), Align2::CENTER_CENTER, (i / per_beat.max(1) + 1).to_string(), theme::font(6.5 * z), theme::TEXT_FAINT);
        }
        if editable {
            let resp = v.ui.interact(lane, self.uid("lane"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
            let paint = self.uid("paint");
            let at = |q: Pos2| {
                let i = ((q.x - lane.left()) / cell).floor();
                (i >= 0.0 && (i as usize) < MAX_LEN).then_some(i as usize)
            };
            if let Some(q) = resp.interact_pointer_pos()
                && let Some(i) = at(q)
            {
                let now = sp::step(p, i);
                if resp.clicked() {
                    sp::set_step(p, i, if now == self.s { None } else { self.s });
                }
                if resp.secondary_clicked() {
                    sp::set_step(p, i, None);
                }
                if resp.drag_started() {
                    let value = if now == self.s { -1i32 } else { self.s.map_or(-1, |s| s as i32) };
                    v.ui.data_mut(|d| d.insert_temp(paint, value));
                }
                if resp.dragged()
                    && let Some(value) = v.ui.data(|d| d.get_temp::<i32>(paint))
                {
                    sp::set_step(p, i, (value >= 0).then_some(value as usize));
                }
            }
            if self.chip("clear-steps", Rect::from_min_size(pos2(area.right() - 60.0 * z, area.bottom() - 18.0 * z), vec2(60.0 * z, 18.0 * z)), "CLEAR", false, theme::MUTE, "Rest on every step").clicked() {
                for i in 0..MAX_LEN {
                    sp::set_step(p, i, None);
                }
            }
            if self.chip("order-steps", Rect::from_min_size(pos2(area.right() - 126.0 * z, area.bottom() - 18.0 * z), vec2(62.0 * z, 18.0 * z)), "IN ORDER", false, theme::TEXT_DIM, "The slices in order, one per step").clicked() {
                for i in 0..MAX_LEN {
                    sp::set_step(p, i, (count > 0).then(|| i % count));
                }
            }
        }
        let note = match mode {
            sp::ORDER => "ORDER plays slices 1, 2, 3… (muted ones skipped)".to_owned(),
            sp::RANDOM => format!("RANDOM{} · FILL leaves steps out", if sp::on(p, data::LOCK) { ", locked" } else { "" }),
            sp::STUTTER => "STUTTER rolls each step's slice into a ratchet · CHAOS: longer rolls, more holds".to_owned(),
            _ => "PATTERN: the lane's slices · FILL leaves steps out".to_owned(),
        };
        text(v.painter, pos2(area.left(), area.bottom() - 9.0 * z), Align2::LEFT_CENTER, note, theme::font(7.0 * z), theme::TEXT_FAINT);
    }

    /// PROB: a bar per slice, its chance each step.
    fn prob_bars(&self, area: Rect) {
        let (v, p, z) = (self.v, self.p, self.z());
        let count = sp::count(p);
        if count == 0 {
            return;
        }
        // Under the slice keys, column for column.
        let (x0, x1) = (v.p(KEYS_X[0], 0.0).x, v.p(KEYS_X[1], 0.0).x);
        let cell = (x1 - x0) / MAX_SLICES as f32;
        let bars = Rect::from_min_max(pos2(x0, area.top()), pos2(x1, area.bottom() - 22.0 * z));
        let fill = p.get(sp::FILL);
        for s in 0..count {
            let r = Rect::from_min_max(pos2(bars.left() + cell * s as f32 + 2.0 * z, bars.top()), pos2(bars.left() + cell * (s + 1) as f32 - 2.0 * z, bars.bottom()));
            let c = color(s);
            let prob = sp::get(p, s, slice::PROB) * 0.01;
            let muted = sp::flag(p, s, slice::MUTE);
            v.painter.rect_stroke(r, 2.0 * z, Stroke::new(1.0 * z, if self.s == Some(s) { c } else { theme::CONTROL }), StrokeKind::Inside);
            let h = r.height() * prob;
            let bar = Rect::from_min_max(pos2(r.left() + 3.0 * z, r.bottom() - h), pos2(r.right() - 3.0 * z, r.bottom()));
            v.painter.rect_stroke(bar, 1.5 * z, Stroke::new(1.5 * z, alpha(c, if muted { 0.3 } else { 1.0 })), StrokeKind::Inside);
            // With FILL, the chance that remains.
            let eff = Rect::from_min_max(pos2(bar.left(), r.bottom() - h * fill), bar.max);
            v.painter.rect_filled(eff, 1.5 * z, alpha(c, if muted { 0.08 } else { 0.25 }));
            text(v.painter, pos2(r.center().x, r.bottom() + 8.0 * z), Align2::CENTER_CENTER, (s + 1).to_string(), theme::font(7.0 * z), if self.s == Some(s) { c } else { theme::TEXT_DIM });
            text(v.painter, pos2(r.center().x, r.top() + 7.0 * z), Align2::CENTER_CENTER, format!("{:.0}", prob * 100.0), theme::mono(6.5 * z), theme::TEXT_FAINT);
        }
        let resp = v.ui.interact(bars, self.uid("prob"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical);
        if let Some(q) = resp.interact_pointer_pos() {
            let s = ((q.x - bars.left()) / cell).floor().max(0.0) as usize;
            if s < count {
                if resp.dragged() || resp.drag_started() {
                    let prob = ((bars.bottom() - q.y) / bars.height()).clamp(0.0, 1.0) * 100.0;
                    sp::set(p, s, slice::PROB, prob.round());
                }
                if resp.clicked() {
                    self.select(s);
                }
            }
        }
    }

    /// EUCLID: a ring per slice, its steps round it, its hits filled, the
    /// step playing marked; the selected slice's numbers beside.
    fn rings(&self, area: Rect) {
        let (v, p, z) = (self.v, self.p, self.z());
        let count = sp::count(p);
        // The selected slice's numbers.
        if let Some(s) = self.s {
            let c = color(s);
            let x = area.left() / z - v.min.x / z;
            let y = area.top() / z - v.min.y / z;
            v.text(x, y + 8.0, Align2::LEFT_CENTER, format!("SLICE {}", s + 1), theme::strong(9.0 * z), c);
            let k = sp::get_int(p, s, slice::HITS);
            let n = sp::get_int(p, s, slice::STEPS);
            let rot = sp::get_int(p, s, slice::ROTATE);
            if let Some(val) = self.drag_box("hits", v.r(x, y + 20.0, x + 76.0, y + 36.0), k as f32, (0.0, n as f32, k as f32, 1.0), &format!("HITS {k}"), "Hits spread over the steps (drag)", true) {
                sp::set(p, s, slice::HITS, val);
            }
            if let Some(val) = self.drag_box("steps", v.r(x, y + 40.0, x + 76.0, y + 56.0), n as f32, (1.0, 32.0, 16.0, 1.0), &format!("STEPS {n}"), "The rhythm's length in steps: odd lengths drift against the bar (drag)", true) {
                sp::set(p, s, slice::STEPS, val);
                sp::set(p, s, slice::HITS, (k as f32).min(val));
            }
            if let Some(val) = self.drag_box("rot", v.r(x, y + 60.0, x + 76.0, y + 76.0), rot as f32, (0.0, (n - 1).max(0) as f32, 0.0, 1.0), &format!("ROTATE {rot}"), "Turns the rhythm round by steps (drag; double-click: 0)", true) {
                sp::set(p, s, slice::ROTATE, val);
            }
        }
        if count == 0 {
            return;
        }
        let left = area.left() + 92.0 * z;
        let per_row = 16usize.min(count.max(1));
        let rows = count.div_ceil(per_row);
        let cw = (area.right() - left) / per_row as f32;
        let ch = area.height() / rows as f32;
        let radius = (cw.min(ch) * 0.5 - 9.0 * z).max(6.0 * z);
        let step_index = p.viz[viz::STEP].get();
        for s in 0..count {
            let (row, col) = (s / per_row, s % per_row);
            let centre = pos2(left + cw * (col as f32 + 0.5), area.top() + ch * (row as f32 + 0.5) - 4.0 * z);
            let c = color(s);
            let k = sp::get_int(p, s, slice::HITS).max(0) as usize;
            let n = sp::get_int(p, s, slice::STEPS).max(1) as usize;
            let rot = sp::get_int(p, s, slice::ROTATE).max(0) as usize;
            let selected = self.s == Some(s);
            let muted = sp::flag(p, s, slice::MUTE);
            let ink = if muted { 0.3 } else { 1.0 };
            v.painter.circle_stroke(centre, radius, Stroke::new(if selected { 1.5 } else { 1.0 } * z, alpha(if selected { c } else { theme::CONTROL_HI }, ink)));
            let mut hits_pts = Vec::new();
            for i in 0..n {
                let a = std::f32::consts::TAU * i as f32 / n as f32 - std::f32::consts::FRAC_PI_2;
                let at = centre + vec2(a.cos(), a.sin()) * radius;
                if sp::euclid(k, n, rot, i) {
                    v.painter.circle_filled(at, 2.6 * z, alpha(c, ink));
                    hits_pts.push(at);
                } else {
                    v.painter.circle_stroke(at, 1.6 * z, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, ink)));
                }
            }
            // The hits joined: the rhythm's polygon.
            if hits_pts.len() >= 2 {
                hits_pts.push(hits_pts[0]);
                v.painter.add(Shape::line(hits_pts, Stroke::new(1.0 * z, alpha(c, 0.45 * ink))));
            }
            if step_index >= 0.0 {
                let i = p.viz[viz::INDEX].get().max(0.0) as usize % n;
                let a = std::f32::consts::TAU * i as f32 / n as f32 - std::f32::consts::FRAC_PI_2;
                let at = centre + vec2(a.cos(), a.sin()) * radius;
                v.painter.circle_stroke(at, 4.2 * z, Stroke::new(1.3 * z, theme::TEXT));
            }
            text(v.painter, centre, Align2::CENTER_CENTER, (s + 1).to_string(), theme::strong(8.0 * z), alpha(c, ink));
            text(v.painter, centre + vec2(0.0, radius + 8.0 * z), Align2::CENTER_CENTER, format!("{k}/{n}"), theme::mono(6.5 * z), theme::TEXT_FAINT);
            let hit = Rect::from_center_size(centre, Vec2::splat(radius * 2.0 + 8.0 * z));
            let resp = v.ui.interact(hit, self.uid(("ring", s)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!("Slice {} · {k} hits over {n} steps · rotate {rot}", s + 1));
            if resp.clicked() || resp.drag_started() {
                self.select(s);
            }
            if resp.dragged() {
                let id = self.uid(("ring-acc", s));
                let dy = resp.drag_delta().y;
                let steps = v.ui.data_mut(|m| {
                    let acc = m.get_temp_mut_or(id, 0.0f32);
                    *acc -= dy / z;
                    let steps = (*acc / 8.0).trunc();
                    *acc -= steps * 8.0;
                    steps
                });
                if steps != 0.0 {
                    sp::set(p, s, slice::HITS, (k as f32 + steps).clamp(0.0, n as f32));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // MIDI
    // -----------------------------------------------------------------------

    fn midi_page(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let y = PAGE_Y;
        let ch = sp::int(p, data::MIDI_CH, 0, 16);
        let shown = if ch == 0 { "CH OMNI".to_owned() } else { format!("CH {ch}") };
        if let Some(n) = self.drag_box("midich", v.r(PAD, y, 90.0, y + 18.0), ch as f32, (0.0, 16.0, 0.0, 1.0), &format!("‹ {shown} ›"), "The MIDI channel heard and sent on (OMNI: every channel heard, sent on 1)", true) {
            p.data[data::MIDI_CH].set(n);
        }
        let chroma = sp::on(p, data::CHROMA);
        if self.chip("chroma", v.r(96.0, y, 166.0, y + 18.0), "CHROMA", chroma, self.accent, "Notes no slice is mapped to play the selected slice in pitch (C4: as it is)").clicked() {
            sp::toggle(p, data::CHROMA);
        }
        let out = sp::on(p, data::MIDI_OUT);
        if self.chip("midiout", v.r(170.0, y, 246.0, y + 18.0), "MIDI OUT", out, self.accent, "The steps' hits go out as notes: each slice's note, or C2 up by slice").clicked() {
            sp::toggle(p, data::MIDI_OUT);
        }
        if self.chip("map", v.r(252.0, y, 330.0, y + 18.0), "MAP FROM C2", false, theme::TEXT_DIM, "Map the slices to notes from C2 up, one each").clicked() {
            sp::map_chromatic(p, 36);
        }
        if self.chip("unmap", v.r(334.0, y, 396.0, y + 18.0), "UNMAP", false, theme::MUTE, "No slice on any note").clicked() {
            for s in 0..sp::count(p) {
                sp::set(p, s, slice::NOTE, -1.0);
            }
        }
        let learn = sp::int(p, data::LEARN, 0, MAX_SLICES as i32);
        if let Some(s) = self.s {
            let armed = learn as usize == s + 1;
            if self.chip("learn", v.r(402.0, y, 470.0, y + 18.0), if armed { "LISTENING" } else { "LEARN" }, armed, RED, "The selected slice takes the next note played").clicked() {
                p.data[data::LEARN].set(if armed { 0.0 } else { (s + 1) as f32 });
            }
        }
        let status = if learn > 0 {
            format!("play a note for slice {}", learn)
        } else {
            let last = p.viz[viz::NOTE_IN].get();
            if view.notes.0 > 0.0 { format!("last note in: {}", note_label(last as i32)) } else { "no notes in yet".to_owned() }
        };
        v.text(478.0, y + 9.0, Align2::LEFT_CENTER, status, v.font(7.5), if learn > 0 { RED } else { theme::TEXT_DIM });

        // The keyboard: C1 to C7, slices shown on their notes.
        let kb = v.r(PAD, y + 30.0, W - PAD, y + 110.0);
        self.keyboard(kb, view);

        // Every slice: its note and LEARN.
        let count = sp::count(p);
        let grid_top = y + 124.0;
        let cw = (W - 2.0 * PAD) / 16.0;
        for s in 0..MAX_SLICES {
            let (row, col) = (s / 16, s % 16);
            let x = PAD + cw * col as f32;
            let yy = grid_top + 44.0 * row as f32;
            let r = v.r(x + 2.0, yy, x + cw - 2.0, yy + 38.0);
            if s >= count {
                v.painter.rect_stroke(r, 3.0 * z, Stroke::new(1.0 * z, alpha(theme::CONTROL, 0.5)), StrokeKind::Inside);
                continue;
            }
            let c = color(s);
            let selected = self.s == Some(s);
            v.painter.rect_stroke(r, 3.0 * z, Stroke::new(if selected { 1.8 } else { 1.0 } * z, c), StrokeKind::Inside);
            let note = sp::note(p, s);
            text(v.painter, r.left_top() + vec2(5.0 * z, 8.0 * z), Align2::LEFT_CENTER, (s + 1).to_string(), theme::strong(7.5 * z), c);
            text(v.painter, r.right_top() + vec2(-5.0 * z, 8.0 * z), Align2::RIGHT_CENTER, note.map_or("—".to_owned(), |n| note_label(n as i32)), theme::mono(7.5 * z), if note.is_some() { theme::TEXT } else { theme::TEXT_FAINT });
            let armed = learn as usize == s + 1;
            let lr = Rect::from_min_max(r.left_bottom() + vec2(4.0 * z, -16.0 * z), r.right_bottom() + vec2(-4.0 * z, -4.0 * z));
            if self.chip(("lrn", s), lr, if armed { "…" } else { "LRN" }, armed, RED, "This slice takes the next note played").clicked() {
                p.data[data::LEARN].set(if armed { 0.0 } else { (s + 1) as f32 });
                self.select(s);
            }
        }
        v.text(PAD, H - 18.0, Align2::LEFT_CENTER, "Click a key: the selected slice plays there (again: off) · a note-off ends GATE and LOOP slices and their repeats", v.font(7.0), theme::TEXT_FAINT);
    }

    fn keyboard(&self, r: Rect, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        const FIRST: i32 = 24;
        const LAST: i32 = 96;
        let white = |n: i32| !matches!(n.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
        let whites = (FIRST..=LAST).filter(|&n| white(n)).count();
        let ww = r.width() / whites as f32;
        let count = sp::count(p);
        let slice_of = |n: i32| (0..count).find(|&s| sp::note(p, s) == Some(n as u8));
        let last_in = p.viz[viz::NOTE_IN].get() as i32;
        let lit = self.now - view.notes.1 < 0.25;
        let mut rects: Vec<(i32, Rect)> = Vec::new();
        let mut wi = 0;
        for n in FIRST..=LAST {
            if white(n) {
                let kr = Rect::from_min_max(pos2(r.left() + ww * wi as f32 + 0.5 * z, r.top()), pos2(r.left() + ww * (wi + 1) as f32 - 0.5 * z, r.bottom()));
                rects.push((n, kr));
                wi += 1;
            }
        }
        let mut blacks: Vec<(i32, Rect)> = Vec::new();
        wi = 0;
        for n in FIRST..=LAST {
            if white(n) {
                wi += 1;
            } else {
                let x = r.left() + ww * wi as f32;
                blacks.push((n, Rect::from_min_max(pos2(x - ww * 0.32, r.top()), pos2(x + ww * 0.32, r.top() + r.height() * 0.6))));
            }
        }
        let draw_key = |n: i32, kr: Rect, black: bool| {
            let mapped = slice_of(n);
            let fill = if lit && n == last_in {
                theme::TEXT_DIM
            } else if black {
                theme::INSET
            } else {
                theme::NODE_BODY
            };
            v.painter.rect_filled(kr, 1.5 * z, fill);
            v.painter.rect_stroke(kr, 1.5 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
            if let Some(s) = mapped {
                let c = color(s);
                v.painter.rect_stroke(kr.shrink(1.0 * z), 1.5 * z, Stroke::new(1.6 * z, c), StrokeKind::Inside);
                let at = if black { kr.center_bottom() - vec2(0.0, 7.0 * z) } else { kr.center_bottom() - vec2(0.0, 9.0 * z) };
                text(v.painter, at, Align2::CENTER_CENTER, (s + 1).to_string(), theme::strong(6.5 * z), c);
            }
            if n % 12 == 0 && !black {
                text(v.painter, kr.center_top() + vec2(0.0, 7.0 * z), Align2::CENTER_CENTER, note_label(n), theme::font(6.0 * z), theme::TEXT_FAINT);
            }
        };
        for &(n, kr) in &rects {
            draw_key(n, kr, false);
        }
        for &(n, kr) in &blacks {
            draw_key(n, kr, true);
        }
        let resp = v.ui.interact(r, self.uid("keyboard"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked()
            && let (Some(q), Some(s)) = (resp.interact_pointer_pos(), self.s)
        {
            let hit = blacks.iter().find(|(_, kr)| kr.contains(q)).or_else(|| rects.iter().find(|(_, kr)| kr.contains(q)));
            if let Some(&(n, _)) = hit {
                if sp::note(p, s) == Some(n as u8) {
                    sp::set(p, s, slice::NOTE, -1.0);
                } else {
                    sp::set(p, s, slice::NOTE, n as f32);
                }
            }
        }
    }
}

/// `s`, cut to `max` characters with an ellipsis.
fn clip_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_owned() } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
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
