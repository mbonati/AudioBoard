//! Panel of the MIDI Looper. Up top the transport (REC, DUB, PLAY, UNDO,
//! CLEAR), what it follows (the board or a MIDI clock), what starts a
//! recording, THRU and STEP. Then the eight scenes, and the settings of the
//! one selected: its bars, repeats, direction and what follows. The roll
//! shows its notes over two octaves, four bars at a time, with the
//! playhead; click to add a note a GRID step long (drag to stretch it),
//! click a note to take it away. With STEP, a click on the ruler moves the
//! step cursor. Then the knobs, and the CONTROL channel's notes.

use egui::{
    Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::looper::{
    self as lo, BAR, CHANNEL, CONTROL, DIRECTIONS, FOLLOWS, GRID, LAUNCH, Note, PPQ, QUANTIZE,
    REPEATS, TRANSPOSE, TRIGGERS, ask, data, state, viz,
};
use crate::audio::params::note_label;
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 474.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
const PADS: [f32; 2] = [58.0, 88.0];
const SETTINGS: f32 = 104.0;
/// What the looper is doing, and what is under the pointer.
const STATUS: f32 = 122.0;
/// The roll: its ruler, the grid, and the keys beside it.
const RULER: [f32; 2] = [130.0, 142.0];
const GRID_Y: [f32; 2] = [144.0, 390.0];
const KEYS_X: f32 = 40.0;
const GRID_R: f32 = 722.0;
const ROWS: u8 = 24;
const KNOBS: f32 = 404.0;
/// Bars shown at once.
const PAGE: usize = 4;
const REC_RED: Color32 = theme::MUTE;

const KNOB_ROW: [(usize, &str, &str); 6] = [
    (
        GRID,
        "GRID",
        "The step: for the roll, STEP, QUANTIZE and RANDOM's slices",
    ),
    (
        QUANTIZE,
        "QUANTIZE",
        "How far recorded notes are pulled onto the GRID",
    ),
    (
        LAUNCH,
        "LAUNCH",
        "When a scene asked for starts: on the next beat, the next bar, or when the loop playing ends",
    ),
    (
        TRANSPOSE,
        "TRANSPOSE",
        "Shifts what plays back (not what passes THRU)",
    ),
    (
        CHANNEL,
        "OUT CH",
        "The channel notes play on: as they were played, or one for all",
    ),
    (
        CONTROL,
        "CTRL CH",
        "The channel whose notes are commands, not music (off: none)",
    ),
];

/// The node's two summary lines: the scene and what it does, then its
/// size.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let s = lo::scene(p);
    let what = match p.viz[viz::STATE].get() as usize {
        state::ARMED => "armed",
        state::RECORDING => "recording",
        state::PLAYING if p.viz[viz::DUB].get() > 0.5 => "overdub",
        state::PLAYING => "playing",
        _ => "stopped",
    };
    (
        format!("SCENE {} · {what}", s + 1),
        format!("{} bars · {} notes", lo::bars(p, s), lo::count(p, s)),
    )
}

/// What the roll remembers: its lowest key, the page, and a note being
/// stretched.
#[derive(Clone, Copy, Default)]
struct View {
    scene: Option<usize>,
    low: u8,
    page: usize,
    /// Paged by hand until then (else it follows the playhead).
    manual: f64,
    stretching: Option<(usize, u32)>,
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
}

/// `(midi in, midi out)`: what is patched.
pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &ModuleNode,
    (midi_in, midi_out): (bool, bool),
) {
    let p = &m.params;
    let key = Id::unique(("looper-view", id));
    let mut view: View = v.ui.data(|d| d.get_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    let s = Panel {
        v,
        id,
        p,
        accent,
        now,
    };
    let col = |on: bool| {
        if on {
            theme::TEXT_DIM
        } else {
            theme::TEXT_FAINT
        }
    };
    v.text(
        PAD,
        42.0,
        Align2::LEFT_CENTER,
        "MIDI",
        v.font(9.0),
        col(midi_in),
    );
    v.text(
        W - PAD,
        42.0,
        Align2::RIGHT_CENTER,
        "MIDI",
        v.font(9.0),
        col(midi_out),
    );
    s.header();
    s.scenes();
    s.settings();
    s.roll(&mut view);
    s.knobs();
    v.ui.data_mut(|d| d.insert_temp(key, view));
    let busy = p.viz[viz::RUNNING].get() > 0.5
        || p.viz[viz::STATE].get() as usize == state::ARMED
        || p.viz[viz::QUEUED].get() >= 0.0;
    if busy {
        v.ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("looper", self.id)).with(salt)
    }

    fn state(&self) -> usize {
        self.p.viz[viz::STATE].get() as usize
    }

    fn blink(&self) -> bool {
        (self.now * 2.0).fract() < 0.5
    }

    fn button(
        &self,
        salt: &str,
        rect: Rect,
        label: &str,
        on: bool,
        color: Color32,
        tip: &str,
    ) -> bool {
        widgets::toggle(
            self.v.ui,
            self.v.painter,
            self.uid(salt),
            rect,
            label,
            on,
            color,
            self.z(),
        )
        .on_hover_text(tip)
        .clicked()
    }

    /// The transport, SYNC, what starts a recording, THRU and STEP.
    fn header(&self) {
        let (v, p) = (self.v, self.p);
        let st = self.state();
        let dub = p.viz[viz::DUB].get() > 0.5;
        let queued = p.viz[viz::QUEUED].get() >= 0.0;
        let rec_on = st == state::RECORDING || (st == state::ARMED && self.blink());
        let rect = |x: f32| v.r(x, 34.0, x + 42.0, 50.0);
        if self.button("rec", rect(50.0), "REC", rec_on, REC_RED, "Record the scene: it starts as REC ON says and runs for its bars, then plays.\nAgain while armed: cancel; while recording: stop recording") {
            lo::request(p, ask::REC, 0.0);
        }
        if self.button(
            "dub",
            rect(96.0),
            "DUB",
            dub,
            REC_RED,
            "Overdub: play over the loop and it joins in at the end of each pass",
        ) {
            lo::request(p, ask::DUB, 0.0);
        }
        let running = st != state::STOPPED || p.viz[viz::PLAYING].get() >= 0.0;
        let (label, lit) = if running {
            ("STOP", false)
        } else {
            ("PLAY", queued && self.blink())
        };
        if self.button(
            "play",
            rect(142.0),
            label,
            lit,
            self.accent,
            "Play the scene (from the next LAUNCH point), or stop",
        ) {
            lo::request(p, ask::PLAY, 0.0);
        }
        if self.button(
            "undo",
            rect(188.0),
            "UNDO",
            false,
            self.accent,
            "Take back the last recording, overdub, step session or clear (again: redo)",
        ) {
            lo::request(p, ask::UNDO, 0.0);
        }
        if self.button(
            "clear",
            rect(234.0),
            "CLEAR",
            false,
            self.accent,
            "Empty the scene (UNDO brings it back)",
        ) {
            lo::request(p, ask::CLEAR, 0.0);
        }

        v.text(
            290.0,
            42.0,
            Align2::LEFT_CENTER,
            "SYNC",
            v.font(7.0),
            theme::TEXT_FAINT,
        );
        let tempo = p.viz[viz::TEMPO].get();
        let clock_tip: &'static str = if tempo > 0.0 {
            "Follow the MIDI clock on the MIDI input (Start, Stop, Continue and song position too)"
        } else {
            "Follow a MIDI clock on the MIDI input: none is coming in now"
        };
        let sync = lo::flag(p, data::SYNC) as usize;
        let picked = segmented(
            v,
            self.uid("sync"),
            v.r(314.0, 34.0, 414.0, 50.0),
            2,
            sync,
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    ["BOARD", "CLOCK"][k],
                    v.font(7.0),
                    col,
                );
                ["Follow the board's tempo and transport", clock_tip][k]
            },
        );
        if let Some(k) = picked {
            lo::set_flag(p, data::SYNC, k == 1);
        }
        v.text(
            424.0,
            42.0,
            Align2::LEFT_CENTER,
            "REC ON",
            v.font(7.0),
            theme::TEXT_FAINT,
        );
        const TRIGGER_TIPS: [&str; 3] = [
            "Recording starts on the next bar",
            "Recording starts with the first note you play (on its bar; up to an eighth early, on the next)",
            "Recording starts on a Start: the board's (Stop, then Start) or the MIDI clock's",
        ];
        let trigger = lo::choice(p, data::TRIGGER, TRIGGERS.len());
        let picked = segmented(
            v,
            self.uid("trigger"),
            v.r(462.0, 34.0, 590.0, 50.0),
            TRIGGERS.len(),
            trigger,
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    TRIGGERS[k],
                    v.font(7.0),
                    col,
                );
                TRIGGER_TIPS[k]
            },
        );
        if let Some(k) = picked {
            p.data[data::TRIGGER].set(k as f32);
        }
        let thru = lo::flag(p, data::THRU);
        if self.button(
            "thru",
            v.r(600.0, 34.0, 642.0, 50.0),
            "THRU",
            thru,
            self.accent,
            "What comes in also goes out, so you hear what you play",
        ) {
            lo::set_flag(p, data::THRU, !thru);
        }
        let step = lo::flag(p, data::STEP);
        if self.button(
            "step",
            v.r(646.0, 34.0, 688.0, 50.0),
            "STEP",
            step,
            self.accent,
            "Step recording: each note (or chord) you play goes in at the cursor, a GRID step long, and the cursor moves on.\nClick the ruler to move the cursor",
        ) {
            lo::set_flag(p, data::STEP, !step);
        }
    }

    /// The eight scenes.
    fn scenes(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let selected = lo::scene(p);
        let playing = p.viz[viz::PLAYING].get();
        let queued = p.viz[viz::QUEUED].get();
        let recording = self.state() == state::RECORDING;
        let w = (W - 2.0 * PAD - 7.0 * 6.0) / data::SCENES as f32;
        for s in 0..data::SCENES {
            let x = PAD + s as f32 * (w + 6.0);
            let rect = v.r(x, PADS[0], x + w, PADS[1]);
            let resp =
                v.ui.interact(rect, self.uid(("scene", s)), Sense::CLICK)
                    .on_hover_cursor(CursorIcon::PointingHand);
            let resp = resp.on_hover_text(format!(
                "Scene {}: click to play it (at the LAUNCH point), or to pick it while stopped.\nProgram Change {} or {} on the CTRL channel do the same",
                s + 1,
                s + 1,
                note_label((lo::SCENE_NOTES as usize + s) as i32)
            ));
            if resp.clicked() {
                lo::request(p, ask::SCENE, s as f32);
            }
            let notes = lo::count(p, s);
            let is_playing = playing.round() as i32 == s as i32 && playing >= 0.0;
            let color = if is_playing && recording {
                REC_RED
            } else {
                self.accent
            };
            let fill = if is_playing {
                alpha(color, 0.3)
            } else if resp.hovered() {
                theme::CONTROL_HI
            } else {
                theme::CONTROL
            };
            v.painter.rect_filled(rect, 4.0 * z, fill);
            if s == selected {
                v.painter.rect_stroke(
                    rect,
                    4.0 * z,
                    Stroke::new(1.5 * z, color),
                    StrokeKind::Inside,
                );
            }
            if queued.round() as i32 == s as i32 && queued >= 0.0 && self.blink() {
                v.painter.rect_stroke(
                    rect.expand(1.5 * z),
                    5.0 * z,
                    Stroke::new(1.5 * z, mix(color, theme::TEXT, 0.4)),
                    StrokeKind::Outside,
                );
            }
            let ink = if notes > 0 {
                theme::TEXT
            } else {
                theme::TEXT_FAINT
            };
            v.painter.text(
                rect.left_center() + vec2(8.0 * z, -4.0 * z),
                Align2::LEFT_CENTER,
                format!("{}", s + 1),
                theme::strong(11.0 * z),
                ink,
            );
            let what = if notes > 0 {
                format!("{} bars · {notes}", lo::bars(p, s))
            } else {
                format!("{} bars · empty", lo::bars(p, s))
            };
            v.painter.text(
                rect.left_center() + vec2(8.0 * z, 8.0 * z),
                Align2::LEFT_CENTER,
                what,
                theme::font(6.5 * z),
                theme::TEXT_DIM,
            );
            let flags = match lo::direction(p, s) {
                lo::REVERSE => "◀",
                lo::RANDOM => "⁂",
                _ => "",
            };
            v.painter.text(
                rect.right_top() + vec2(-6.0, 5.0) * z,
                Align2::RIGHT_TOP,
                flags,
                theme::font(8.0 * z),
                theme::TEXT_DIM,
            );
            // How far through its loop.
            let pos = p.viz[viz::POS].get();
            if is_playing && pos >= 0.0 {
                let f = (pos as f64 / (lo::bars(p, s) as f64 * BAR)).clamp(0.0, 1.0) as f32;
                let bar = Rect::from_min_max(
                    rect.left_bottom() + vec2(3.0, -4.0) * z,
                    pos2(
                        rect.left() + 3.0 * z + (rect.width() - 6.0 * z) * f,
                        rect.bottom() - 2.0 * z,
                    ),
                );
                v.painter.rect_filled(bar, 1.0 * z, color);
            }
        }
    }

    /// A small − value + control.
    fn stepper(&self, salt: &str, x: f32, label: &str, value: &str, tip: &str) -> i32 {
        let (v, z) = (self.v, self.z());
        let y = SETTINGS;
        v.text(
            x,
            y,
            Align2::LEFT_CENTER,
            label,
            v.font(7.0),
            theme::TEXT_FAINT,
        );
        let lx = x + 40.0;
        let minus = widgets::toggle(
            v.ui,
            v.painter,
            self.uid((salt, 0)),
            v.r(lx, y - 8.0, lx + 16.0, y + 8.0),
            "−",
            false,
            self.accent,
            z,
        )
        .on_hover_text(tip);
        v.text(
            lx + 34.0,
            y,
            Align2::CENTER_CENTER,
            value,
            v.mono(8.0),
            theme::TEXT,
        );
        let plus = widgets::toggle(
            v.ui,
            v.painter,
            self.uid((salt, 1)),
            v.r(lx + 52.0, y - 8.0, lx + 68.0, y + 8.0),
            "+",
            false,
            self.accent,
            z,
        )
        .on_hover_text(tip);
        if minus.clicked() {
            -1
        } else if plus.clicked() {
            1
        } else {
            0
        }
    }

    /// The selected scene's bars, repeats, direction, what follows, COPY;
    /// and what the looper is doing.
    fn settings(&self) {
        let (v, p) = (self.v, self.p);
        let s = lo::scene(p);
        v.text(
            PAD,
            SETTINGS,
            Align2::LEFT_CENTER,
            format!("SCENE {}", s + 1),
            theme::strong(8.5 * self.z()),
            theme::TEXT_DIM,
        );
        let bars = lo::bars(p, s);
        let d = self.stepper(
            "bars",
            74.0,
            "BARS",
            &bars.to_string(),
            "The scene's length, in bars (1–16)",
        );
        if d != 0 {
            lo::set_bars(p, s, (bars as i32 + d).max(1) as usize);
        }
        let repeats = lo::repeats(p, s);
        let k = REPEATS.iter().position(|&r| r == repeats).unwrap_or(0) as i32;
        let shown = if repeats == 0 {
            "∞".to_owned()
        } else {
            format!("×{repeats}")
        };
        let d = self.stepper(
            "repeats",
            194.0,
            "REPEATS",
            &shown,
            "How many times the scene plays before what follows (∞: forever)",
        );
        if d != 0 {
            lo::set_repeats(
                p,
                s,
                REPEATS[(k + d).clamp(0, REPEATS.len() as i32 - 1) as usize],
            );
        }
        const DIR_TIPS: [&str; 3] = [
            "Forward",
            "Reversed: from the end back to the start",
            "A random pick: every GRID step plays a slice of the loop chosen at random",
        ];
        let picked = segmented(
            v,
            self.uid(("dir", s)),
            v.r(330.0, SETTINGS - 8.0, 470.0, SETTINGS + 8.0),
            DIRECTIONS.len(),
            lo::direction(p, s),
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    DIRECTIONS[k],
                    v.font(7.0),
                    col,
                );
                DIR_TIPS[k]
            },
        );
        if let Some(k) = picked {
            lo::set_direction(p, s, k);
        }
        v.text(
            482.0,
            SETTINGS,
            Align2::LEFT_CENTER,
            "THEN",
            v.font(7.0),
            theme::TEXT_FAINT,
        );
        const FOLLOW_TIPS: [&str; 3] = [
            "After its repeats: the next scene that has notes",
            "After its repeats: another scene with notes, at random",
            "After its repeats: stop",
        ];
        let endless = repeats == 0;
        let accent = if endless {
            alpha(self.accent, 0.4)
        } else {
            self.accent
        };
        let picked = segmented(
            v,
            self.uid(("follow", s)),
            v.r(510.0, SETTINGS - 8.0, 640.0, SETTINGS + 8.0),
            FOLLOWS.len(),
            lo::follow(p, s),
            accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    FOLLOWS[k],
                    v.font(7.0),
                    if endless { alpha(col, 0.5) } else { col },
                );
                FOLLOW_TIPS[k]
            },
        );
        if let Some(k) = picked {
            lo::set_follow(p, s, k);
        }
        let target = (1..data::SCENES)
            .map(|k| (s + k) % data::SCENES)
            .find(|&t| lo::count(p, t) == 0);
        let tip = match target {
            Some(t) => format!("Copy this scene into scene {}, the next empty one", t + 1),
            None => "No empty scene to copy into".to_owned(),
        };
        if self.button(
            "copy",
            v.r(650.0, SETTINGS - 8.0, 700.0, SETTINGS + 8.0),
            "COPY →",
            false,
            self.accent,
            &tip,
        ) && let Some(t) = target
        {
            lo::copy_scene(p, s, t);
        }
    }

    /// What the looper is doing, in words.
    fn status(&self) -> String {
        let p = self.p;
        let s = lo::scene(p);
        let pos = p.viz[viz::POS].get();
        let bar = |pos: f32| (pos as f64 / BAR).floor() as usize + 1;
        let queued = p.viz[viz::QUEUED].get();
        let next = if queued >= 0.0 {
            format!(" · scene {} next", queued as usize + 1)
        } else {
            String::new()
        };
        let clock = if lo::flag(p, data::SYNC) {
            match p.viz[viz::TEMPO].get() {
                t if t > 0.0 => format!(" · clock {t:.1} BPM"),
                _ => " · no clock".to_owned(),
            }
        } else {
            String::new()
        };
        let what = match self.state() {
            state::ARMED => match lo::choice(p, data::TRIGGER, TRIGGERS.len()) {
                lo::ON_NOTE => "armed: play to record".to_owned(),
                lo::ON_START => "armed: waiting for a Start".to_owned(),
                _ => "armed: records from the next bar".to_owned(),
            },
            state::RECORDING if pos >= 0.0 => {
                format!("recording bar {} of {}", bar(pos), lo::bars(p, s))
            }
            state::PLAYING if pos >= 0.0 => {
                let pass = p.viz[viz::PASS].get() as u32 + 1;
                let of = match lo::repeats(p, s) {
                    0 => String::new(),
                    r => format!(" of {r}"),
                };
                let dub = if p.viz[viz::DUB].get() > 0.5 {
                    " · overdub"
                } else {
                    ""
                };
                format!("bar {} · pass {pass}{of}{dub}", bar(pos))
            }
            state::PLAYING => "waiting for the transport".to_owned(),
            _ if queued >= 0.0 => "starts at the LAUNCH point".to_owned(),
            _ => "stopped".to_owned(),
        };
        format!("{what}{next}{clock}")
    }

    /// The notes of the selected scene, four bars at a time.
    fn roll(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let s = lo::scene(p);
        let bars = lo::bars(p, s);
        let len = lo::length(p, s);
        let step = lo::step(p);
        let notes: Vec<(usize, Note)> = (0..lo::count(p, s))
            .map(|i| (i, lo::note(p, s, i)))
            .collect();
        if view.scene != Some(s) {
            view.scene = Some(s);
            view.page = 0;
            view.low = notes
                .iter()
                .map(|(_, n)| n.key)
                .min()
                .map_or(48, |k| (k / 12) * 12)
                .min(127 - ROWS);
        }
        let pages = bars.div_ceil(PAGE);
        let playing = p.viz[viz::PLAYING].get().round() as i32 == s as i32
            && p.viz[viz::CONTENT].get() >= 0.0;
        let content = p.viz[viz::CONTENT].get() as f64 * PPQ as f64;
        if playing && self.now > view.manual {
            view.page = ((content / (PAGE as f64 * BAR * PPQ as f64)) as usize).min(pages - 1);
        }
        view.page = view.page.min(pages - 1);
        let page_ticks = (bars.min(PAGE) as u32) * BAR as u32 * PPQ;
        let first = view.page as u32 * PAGE as u32 * BAR as u32 * PPQ;

        let grid = v.r(KEYS_X, GRID_Y[0], GRID_R, GRID_Y[1]);
        let ruler = v.r(KEYS_X, RULER[0], GRID_R, RULER[1]);
        let row_h = grid.height() / ROWS as f32;
        let x_of = |tick: u32| {
            grid.left() + grid.width() * (tick as f32 - first as f32) / page_ticks as f32
        };
        let tick_at = |x: f32| first as f32 + (x - grid.left()) / grid.width() * page_ticks as f32;
        let y_of = |key: u8| grid.bottom() - (key as f32 - view.low as f32 + 1.0) * row_h;

        v.painter
            .rect_filled(grid.union(ruler), 4.0 * z, theme::INSET);
        // The keys: black ones shaded, a name on every C.
        for r in 0..ROWS {
            let key = view.low + r;
            let y = y_of(key);
            let row = Rect::from_min_max(pos2(grid.left(), y), pos2(grid.right(), y + row_h));
            if matches!(key % 12, 1 | 3 | 6 | 8 | 10) {
                v.painter.rect_filled(row, 0.0, alpha(Color32::BLACK, 0.25));
            }
            if key.is_multiple_of(12) {
                v.painter.hline(
                    grid.x_range(),
                    y + row_h,
                    Stroke::new(0.8 * z, alpha(theme::TEXT_FAINT, 0.3)),
                );
                v.painter.text(
                    pos2(grid.left() - 4.0 * z, y + row_h * 0.5),
                    Align2::RIGHT_CENTER,
                    note_label(key as i32),
                    theme::font(6.5 * z),
                    theme::TEXT_FAINT,
                );
            }
        }
        // Steps, beats and bars.
        let mut t = first;
        while t <= first + page_ticks {
            let x = x_of(t);
            let (a, w) = if t.is_multiple_of(BAR as u32 * PPQ) {
                (0.45, 1.0)
            } else if t.is_multiple_of(PPQ) {
                (0.22, 0.8)
            } else {
                (0.08, 0.6)
            };
            v.painter.vline(
                x,
                grid.y_range(),
                Stroke::new(w * z, alpha(theme::TEXT_FAINT, a)),
            );
            if t.is_multiple_of(BAR as u32 * PPQ) && t < len {
                let bar = t / (BAR as u32 * PPQ) + 1;
                v.painter.text(
                    pos2(x + 3.0 * z, ruler.center().y),
                    Align2::LEFT_CENTER,
                    bar.to_string(),
                    theme::mono(6.5 * z),
                    theme::TEXT_DIM,
                );
            }
            t += step;
        }
        // Beyond the loop's end (the last page may be short).
        if first + page_ticks > len {
            let past = Rect::from_min_max(pos2(x_of(len), grid.top()), grid.right_bottom());
            v.painter.rect_filled(past, 0.0, alpha(Color32::BLACK, 0.4));
        }

        // The step cursor.
        if lo::flag(p, data::STEP) {
            let c = p.viz[viz::CURSOR].get() as u32;
            if (first..first + page_ticks).contains(&c) {
                let col = Rect::from_min_max(
                    pos2(x_of(c), ruler.top()),
                    pos2(x_of(c + step), grid.bottom()),
                );
                v.painter.rect_filled(col, 0.0, alpha(self.accent, 0.14));
            }
        }

        // The notes; those above or below the rows as marks on the edge.
        for &(_, n) in &notes {
            if n.at >= len || n.at + n.len <= first || n.at >= first + page_ticks {
                continue;
            }
            let (x0, x1) = (
                x_of(n.at).max(grid.left()),
                x_of((n.at + n.len).min(first + page_ticks)).min(grid.right()),
            );
            if n.key < view.low || n.key >= view.low + ROWS {
                let y = if n.key < view.low {
                    grid.bottom() - 2.0 * z
                } else {
                    grid.top() + 2.0 * z
                };
                v.painter.line_segment(
                    [pos2(x0, y), pos2(x0 + 4.0 * z, y)],
                    Stroke::new(2.0 * z, alpha(self.accent, 0.6)),
                );
                continue;
            }
            let y = y_of(n.key);
            let rect = Rect::from_min_max(
                pos2(x0 + 0.5 * z, y + 1.0 * z),
                pos2((x1 - 0.5 * z).max(x0 + 2.0 * z), y + row_h - 1.0 * z),
            );
            let a = 0.45 + 0.55 * n.velocity as f32 / 127.0;
            v.painter.rect_filled(rect, 1.5 * z, alpha(self.accent, a));
        }

        // The playhead (where in the scene: reversed, or the random slice).
        if playing && (first as f64..(first + page_ticks) as f64).contains(&content) {
            let x = x_of(content as u32);
            v.painter
                .vline(x, grid.y_range(), Stroke::new(1.5 * z, theme::TEXT));
        }

        // Editing: not while this scene is being recorded into.
        let st = self.state();
        let busy = playing && (st == state::RECORDING || p.viz[viz::DUB].get() > 0.5);
        let resp =
            v.ui.interact(grid, self.uid("roll"), Sense::click_and_drag())
                .on_hover_cursor(if busy {
                    CursorIcon::NotAllowed
                } else {
                    CursorIcon::Crosshair
                });
        let under = |pos: Pos2| {
            let tick = tick_at(pos.x).max(0.0) as u32;
            let key = (view.low as f32 + ((grid.bottom() - pos.y) / row_h).floor())
                .clamp(0.0, 127.0) as u8;
            let snapped = tick / step * step;
            let hit = notes
                .iter()
                .find(|(_, n)| n.key == key && n.at <= tick && tick < n.at + n.len.max(1))
                .map(|&(i, _)| i);
            (snapped, key, hit)
        };
        if !busy && let Some(pos) = resp.interact_pointer_pos() {
            let (at, key, hit) = under(pos);
            if resp.clicked() {
                match hit {
                    Some(i) => lo::remove_note(p, s, i),
                    None if at < len => {
                        lo::add_note(
                            p,
                            s,
                            Note {
                                at,
                                len: step,
                                key,
                                velocity: 100,
                                channel: 0,
                            },
                        );
                    }
                    None => {}
                }
            } else if resp.drag_started() && hit.is_none() && at < len {
                if lo::add_note(
                    p,
                    s,
                    Note {
                        at,
                        len: step,
                        key,
                        velocity: 100,
                        channel: 0,
                    },
                ) {
                    view.stretching = Some((lo::count(p, s) - 1, at));
                }
            } else if resp.dragged()
                && let Some((i, start)) = view.stretching
                && i < lo::count(p, s)
            {
                let end = (tick_at(pos.x).max(0.0) as u32).div_ceil(step) * step;
                let mut n = lo::note(p, s, i);
                n.len = end.saturating_sub(start).max(step).min(len - start);
                lo::set_note(p, s, i, n);
            }
        }
        if resp.drag_stopped() || !(resp.dragged() || resp.drag_started()) {
            view.stretching = None;
        }
        if busy {
            v.painter.text(
                grid.center_top() + vec2(0.0, 10.0 * z),
                Align2::CENTER_CENTER,
                "recording into this scene",
                theme::font(7.0 * z),
                alpha(REC_RED, 0.8),
            );
        }
        if let Some(pos) = resp.hover_pos() {
            let (at, key, _) = under(pos);
            let beat = at as f64 / PPQ as f64;
            let text = format!(
                "{} · bar {} beat {:.2}",
                note_label(key as i32),
                (beat / BAR).floor() as usize + 1,
                beat % BAR + 1.0
            );
            v.text(
                GRID_R,
                STATUS,
                Align2::RIGHT_CENTER,
                text,
                v.mono(6.5),
                theme::TEXT_DIM,
            );
        }

        // STEP: the ruler moves the cursor.
        let ruler_resp = v.ui.interact(ruler, self.uid("ruler"), Sense::CLICK);
        if ruler_resp.clicked()
            && let Some(pos) = ruler_resp.interact_pointer_pos()
        {
            let at = (tick_at(pos.x).max(0.0) as u32) / step * step;
            if at < len {
                lo::request(p, ask::CURSOR, at as f32);
            }
        }

        // Octaves and pages.
        let up = v.r(GRID_R + 4.0, GRID_Y[0], W - PAD, GRID_Y[0] + 16.0);
        let down = v.r(GRID_R + 4.0, GRID_Y[0] + 20.0, W - PAD, GRID_Y[0] + 36.0);
        if self.button("up", up, "▲", false, self.accent, "An octave up") {
            view.low = (view.low + 12).min(127 - ROWS);
        }
        if self.button("down", down, "▼", false, self.accent, "An octave down") {
            view.low = view.low.saturating_sub(12);
        }
        if pages > 1 {
            let (prev, next) = (
                v.r(GRID_R + 4.0, GRID_Y[1] - 36.0, W - PAD, GRID_Y[1] - 20.0),
                v.r(GRID_R + 4.0, GRID_Y[1] - 16.0, W - PAD, GRID_Y[1]),
            );
            if self.button("prev", prev, "‹", false, self.accent, "Earlier bars") {
                view.page = view.page.saturating_sub(1);
                view.manual = self.now + 4.0;
            }
            if self.button("next", next, "›", false, self.accent, "Later bars") {
                view.page = (view.page + 1).min(pages - 1);
                view.manual = self.now + 4.0;
            }
            v.text(
                GRID_R + 4.0 + (W - PAD - GRID_R - 4.0) * 0.5,
                GRID_Y[1] - 46.0,
                Align2::CENTER_CENTER,
                format!("{}/{}", view.page + 1, pages),
                v.mono(6.5),
                theme::TEXT_FAINT,
            );
        }

        // What it is doing.
        let lit = matches!(st, state::RECORDING | state::ARMED);
        v.text(
            PAD,
            STATUS,
            Align2::LEFT_CENTER,
            self.status(),
            v.font(7.5),
            if lit { REC_RED } else { theme::TEXT_DIM },
        );
    }

    fn knobs(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.text(
            PAD,
            KNOBS,
            Align2::LEFT_CENTER,
            "LOOPER",
            theme::strong(8.5 * z),
            theme::TEXT_DIM,
        );
        v.painter.hline(
            v.p(PAD + 62.0, 0.0).x..=v.p(500.0, 0.0).x,
            v.p(0.0, KNOBS).y,
            Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)),
        );
        let cw = (500.0 - PAD) / KNOB_ROW.len() as f32;
        for (k, &(i, label, tip)) in KNOB_ROW.iter().enumerate() {
            self.knob(i, PAD + cw * (k as f32 + 0.5), KNOBS + 14.0, label, tip);
        }
        let ctrl = p.get(CONTROL).round() as usize;
        let head = if ctrl > 0 {
            format!("CTRL CH {ctrl}: its notes are commands")
        } else {
            "CTRL CH off: no note commands".to_owned()
        };
        let lines = [
            head.as_str(),
            "C2 REC · C#2 DUB · D2 PLAY · D#2 UNDO · E2 CLEAR",
            "F2 reverse · F#2 random pick · C3–G3 scenes 1–8",
            "Program Change 1–8, on any channel: scenes",
        ];
        for (k, line) in lines.iter().enumerate() {
            let col = if k == 0 {
                theme::TEXT_DIM
            } else {
                theme::TEXT_FAINT
            };
            v.text(
                516.0,
                KNOBS + 2.0 + 13.0 * k as f32,
                Align2::LEFT_CENTER,
                *line,
                v.font(7.0),
                col,
            );
        }
    }

    fn knob(&self, i: usize, cx: f32, y: f32, label: &str, tip: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = lo::spec(i);
        let r = 10.0;
        v.text(
            cx,
            y,
            Align2::CENTER_CENTER,
            label,
            v.font(7.5),
            theme::TEXT_FAINT,
        );
        let top = y + 8.0;
        let mut norm = spec.to_norm(p.get(i));
        let resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("knob", i)),
            v.p(cx, top + r),
            r * z,
            &mut norm,
            spec.to_norm(spec.default),
            spec.bipolar,
            self.accent,
            z,
        )
        .on_hover_text(tip);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let value = if i == QUANTIZE { value } else { value.round() };
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(
            cx,
            top + 2.0 * r + 8.0,
            Align2::CENTER_CENTER,
            lo::format(p, i),
            v.mono(7.0),
            if hot { theme::TEXT } else { theme::TEXT_DIM },
        );
    }
}
