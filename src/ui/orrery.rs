//! Panel of Orrery, drawn flat and graphic: the rings are orbits round a
//! hub, the outer one first, each in a colour of its own. A ring's steps sit
//! round it: a small dot for a rest, a coloured outline for a hit, a
//! bullseye for an accent, and its hits are joined into the rhythm's
//! polygon. A notch marks the ring's first step, so its turn shows. A
//! planet travels each orbit and plays the hits it passes: they flash and
//! send out a ring. In CYCLE mode the planets line up on one hand that
//! sweeps the dial, its beats round the bezel.
//!
//! Under the header, the pattern buttons: click one to recall it (it comes
//! in as LAUNCH says), Shift-click or STORE to keep the rings in it,
//! right-click to name, clear or fill it from the library (PATTERNS ▾).
//!
//! Click a step to switch it, right-click (or Shift-click) to accent it,
//! drag a ring round to turn it (by whole steps; with Shift, freely). On the
//! right, a row per ring (ON, STEPS, PULSES, ROT, NOTE, CH and its pattern),
//! the selected ring's steps in a strip with its VELOCITY, GATE and CHANCE,
//! and the clock: RATE, CYCLE, SWING, ACCENT and TWIST.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use egui::{
    Align2, Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
    pos2, vec2,
};

use super::nodes::{MIN_TEXT, NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::orrery::{
    self as or, ACCENT, ACCENTED, CYCLE, CYCLE_MODE, OFF, ON, Pattern, RATE, RINGS, SLOTS, SWING,
    TWIST, ring, slot, viz,
};
use crate::audio::params::note_label as midi_note_label;
use crate::graph::{ModuleNode, ParamRef};
use crate::presets::{self, ModulePreset};

/// "C2" for 36.
fn note_label(note: u8) -> String {
    midi_note_label(note as i32)
}

pub const SIZE: Vec2 = vec2(860.0, 544.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The pattern buttons' row.
const SLOTS_Y: [f32; 2] = [58.0, 84.0];
const SLOT_W: f32 = 70.0;
/// Everything under the pattern buttons is this much lower.
const DY: f32 = 34.0;
/// The orbits' square, and the rings' radii (outer, inner).
const DISPLAY: [f32; 4] = [PAD, 58.0 + DY, PAD + 440.0, 498.0 + DY];
const R_OUT: f32 = 184.0;
const R_IN: f32 = 62.0;
const BEZEL: f32 = 197.0;
/// The column on the right: the rings' table, the selected ring, the clock.
const X0: f32 = 468.0;
const X1: f32 = W - PAD;
const TABLE_Y: f32 = 64.0 + DY;
const ROW0: f32 = 84.0 + DY;
const ROW_H: f32 = 23.0;
const RING_Y: f32 = 280.0 + DY;
const STRIP: [f32; 2] = [291.0 + DY, 315.0 + DY];
const BUTTONS: [f32; 2] = [322.0 + DY, 338.0 + DY];
const RING_KNOBS_Y: f32 = 350.0 + DY;
const CLOCK_Y: f32 = 414.0 + DY;
/// The table's columns.
const COL_NUM: [f32; 2] = [X0, X0 + 22.0];
const COL_ON: [f32; 2] = [X0 + 26.0, X0 + 52.0];
const COL_STEPS: [f32; 2] = [X0 + 56.0, X0 + 90.0];
const COL_PULSES: [f32; 2] = [X0 + 94.0, X0 + 128.0];
const COL_ROT: [f32; 2] = [X0 + 132.0, X0 + 170.0];
const COL_NOTE: [f32; 2] = [X0 + 174.0, X0 + 210.0];
const COL_CH: [f32; 2] = [X0 + 214.0, X0 + 240.0];
const COL_PATTERN: [f32; 2] = [X0 + 248.0, X1];

/// The rings' colours, from the outside in.
const COLORS: [Color32; RINGS] = [
    Color32::from_rgb(255, 196, 87),
    Color32::from_rgb(255, 107, 157),
    Color32::from_rgb(72, 219, 251),
    Color32::from_rgb(162, 155, 254),
    Color32::from_rgb(126, 231, 135),
    Color32::from_rgb(255, 145, 100),
    Color32::from_rgb(100, 160, 255),
    Color32::from_rgb(235, 225, 110),
];

/// How long a hit's ring spreads, and its flash.
const PING: f64 = 0.5;
const FLASH: f64 = 0.14;

/// A ring's settings with a knob: (value, label, low, high, default, tip).
const RING_KNOBS: [(usize, &str, f32, f32, f32, &str); 3] = [
    (
        ring::VELOCITY,
        "VELOCITY",
        1.0,
        127.0,
        100.0,
        "The ring's velocity (accents play louder, as ACCENT says)",
    ),
    (
        ring::GATE,
        "GATE",
        0.05,
        1.0,
        0.5,
        "How long the ring's notes last, as a part of its step",
    ),
    (
        ring::CHANCE,
        "CHANCE",
        0.0,
        1.0,
        1.0,
        "The chance each hit plays: under 100% the rhythm thins out at random",
    ),
];

const TIPS: [(usize, &str); 5] = [
    (
        RATE,
        "GRID: the step every ring moves on, on the board's grid",
    ),
    (
        CYCLE,
        "CYCLE: how many beats each ring takes to go round once; its steps share them out",
    ),
    (SWING, "GRID: delays every other step of the grid"),
    (
        ACCENT,
        "How much louder accented steps play: from the ring's VELOCITY up to 127",
    ),
    (
        TWIST,
        "Turns each ring this many steps further than the one outside it: the rings fall out of phase, the inner ones the most",
    ),
];

/// The node's two summary lines.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let on = (0..RINGS).filter(|&r| or::is_on(p, r)).count();
    let mode = if p.mode() == CYCLE_MODE {
        "cycle"
    } else {
        "grid"
    };
    let hits: usize = (0..RINGS)
        .filter(|&r| or::is_on(p, r))
        .map(|r| or::hits(p, r))
        .sum();
    (
        format!("{on} rings · {}", or::clock_label(p)),
        format!("{mode} · {hits} hits"),
    )
}

/// A knob's value as the panel shows it.
fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    match i {
        RATE => or::spec(RATE).format(v),
        CYCLE => or::beats_label(v.round().max(1.0) as u32),
        TWIST if v.abs() < 0.005 => "0".to_owned(),
        TWIST => format!("{v:+.2}"),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// "2" or "2.35": a turn in steps.
fn turn_label(steps: f32) -> String {
    if (steps - steps.round()).abs() < 0.005 {
        format!("{}", steps.round())
    } else {
        format!("{steps:.2}")
    }
}

/// A ring being turned by hand: the pointer's last angle and the turn so
/// far (not snapped to steps).
#[derive(Clone, Copy)]
struct Turning {
    ring: usize,
    angle: f32,
    steps: f32,
}

#[derive(Clone, Copy)]
struct Ping {
    ring: usize,
    step: usize,
    at: f64,
}

/// What the panel remembers between frames.
#[derive(Clone, Default)]
struct View {
    time: f64,
    seen: Option<[f32; RINGS]>,
    pings: Vec<Ping>,
    turning: Option<Turning>,
    /// The strip being painted: the value it paints.
    paint: Option<u8>,
    /// STORE is armed: the next button clicked keeps the rings.
    storing: bool,
    /// The processor's last tick and when it changed.
    tick: f32,
    tick_at: f64,
}

impl View {
    /// New hits since the last frame become pings.
    fn update(&mut self, p: &ModuleParams, now: f64) {
        let hits: [f32; RINGS] = std::array::from_fn(|r| p.viz[viz::HITS + r].get());
        if let Some(seen) = self.seen {
            for r in 0..RINGS {
                if hits[r] != seen[r] {
                    let step = p.viz[viz::HIT_STEP + r].get().max(0.0) as usize;
                    self.pings.push(Ping {
                        ring: r,
                        step,
                        at: now,
                    });
                }
            }
        }
        self.seen = Some(hits);
        // A recall waits for the processor; one that rests (nothing patched
        // from it) never takes it, so the panel does.
        let tick = p.viz[viz::TICK].get();
        if tick != self.tick {
            self.tick = tick;
            self.tick_at = now;
        }
        if let Some(k) = or::pending(p)
            && now - self.tick_at > 0.3
        {
            or::take(p, k);
        }
        self.pings.retain(|g| now - g.at < PING);
        if self.pings.len() > 64 {
            let extra = self.pings.len() - 64;
            self.pings.drain(..extra);
        }
    }

    /// How bright step `i` of ring `r` flashes now (0…1).
    fn flash(&self, r: usize, i: usize, now: f64) -> f32 {
        self.pings
            .iter()
            .filter(|g| g.ring == r && g.step == i)
            .map(|g| (1.0 - ((now - g.at) / FLASH) as f32).clamp(0.0, 1.0))
            .fold(0.0, f32::max)
    }
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
    live: bool,
}

/// The orbits on the screen: their centre and the zoom.
#[derive(Clone, Copy)]
struct Orbits {
    c: Pos2,
    z: f32,
}

impl Orbits {
    /// Ring `r`'s radius, on the screen.
    fn radius(&self, r: usize) -> f32 {
        radius(r) * self.z
    }

    /// The point at `angle` on a circle of `radius` (screen units).
    fn at(&self, radius: f32, angle: f32) -> Pos2 {
        self.c + Vec2::angled(angle) * radius
    }

    /// The ring under a point, if any.
    fn ring_at(&self, q: Pos2) -> Option<usize> {
        let d = (q - self.c).length();
        let gap = (R_OUT - R_IN) / (RINGS - 1) as f32 * self.z;
        let k = ((R_OUT * self.z - d) / gap).round();
        if !(0.0..RINGS as f32).contains(&k) {
            return None;
        }
        let r = k as usize;
        ((d - self.radius(r)).abs() <= gap * 0.5 + 2.0 * self.z).then_some(r)
    }

    /// The pointer's angle round the centre.
    fn angle_of(&self, q: Pos2) -> f32 {
        (q - self.c).angle()
    }
}

/// Ring `r`'s radius, in panel units.
fn radius(r: usize) -> f32 {
    R_OUT - (R_OUT - R_IN) * r as f32 / (RINGS - 1) as f32
}

/// The angle of a place `pos` steps round a ring of `n` steps, from the top,
/// clockwise.
fn angle(pos: f32, n: usize) -> f32 {
    -FRAC_PI_2 + pos / n.max(1) as f32 * TAU
}

/// `(midi_in, midi_out)`: the MIDI input and output are patched.
pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &ModuleNode,
    (midi_in, midi_out): (bool, bool),
) {
    let p = &m.params;
    let key = Id::unique(("orrery-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    view.time = now;
    view.update(p, now);
    let live = p.viz[viz::LIVE].get() > 0.5;
    let s = Panel {
        v,
        id,
        p,
        accent,
        now,
        live,
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
    s.patterns(&mut view);
    s.orbits(&mut view);
    s.table();
    s.selected(&mut view);
    s.clock();
    if live
        || !view.pings.is_empty()
        || view.turning.is_some()
        || view.storing
        || or::pending(p).is_some()
    {
        v.ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    v.ui.data_mut(|d| d.insert_temp(key, view));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("orrery", self.id)).with(salt)
    }

    fn cycle(&self) -> bool {
        self.p.mode() == CYCLE_MODE
    }

    /// GRID or CYCLE, DICE, PLAY and what the rings are doing.
    fn header(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        const MODES: [(&str, &str); 2] = [
            (
                "GRID",
                "Every ring steps on the board's grid (RATE): rings of different lengths drift against each other (polymeter)",
            ),
            (
                "CYCLE",
                "Every ring goes round once in CYCLE beats, its steps sharing them out: 3 against 4 against 5 (polyrhythm)",
            ),
        ];
        let picked = segmented(
            v,
            self.uid("mode"),
            v.r(46.0, 34.0, 166.0, 50.0),
            2,
            p.mode().min(1),
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    MODES[k].0,
                    v.font(7.5),
                    col,
                );
                MODES[k].1
            },
        );
        if let Some(k) = picked {
            p.set_mode(if k == 0 { or::GRID } else { CYCLE_MODE });
        }
        let dice = widgets::toggle(v.ui, v.painter, self.uid("dice"), v.r(174.0, 34.0, 220.0, 50.0), "DICE", false, self.accent, z)
            .on_hover_text("New rhythms on the rings that play: their pulses and turns (steps, notes and channels stay).\nShift-click: only the selected ring");
        if dice.clicked() {
            let seed = (self.now * 1000.0) as u32 ^ self.id.wrapping_mul(0x9E37_79B9);
            let only = v.ui.input(|i| i.modifiers.shift).then(|| or::selected(p));
            or::dice(p, seed, only);
        }
        let play = widgets::toggle(
            v.ui,
            v.painter,
            self.uid("play"),
            v.r(226.0, 34.0, 268.0, 50.0),
            "PLAY",
            p.toggled(),
            self.accent,
            z,
        )
        .on_hover_text("Play the rings while the transport plays");
        if play.clicked() {
            p.set_toggle(!p.toggled());
        }
        let text = if !p.toggled() {
            "paused".to_owned()
        } else if !self.live {
            "waiting for Start".to_owned()
        } else if self.cycle() {
            format!("one turn every {}", or::clock_label(p))
        } else {
            format!("a step every {}", or::clock_label(p))
        };
        v.text(
            276.0,
            42.0,
            Align2::LEFT_CENTER,
            text,
            v.font(7.5),
            if self.live {
                theme::TEXT_DIM
            } else {
                theme::TEXT_FAINT
            },
        );
    }

    // -----------------------------------------------------------------------
    // The pattern buttons
    // -----------------------------------------------------------------------

    /// Eight buttons of patterns, STORE, LAUNCH and the library.
    fn patterns(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [y0, y1] = SLOTS_Y;
        let shift = v.ui.input(|i| i.modifiers.shift);
        let (active, pending, edited) = (or::active(p), or::pending(p), or::edited(p));
        for k in 0..SLOTS {
            let x0 = PAD + k as f32 * (SLOT_W + 4.0);
            let rect = v.r(x0, y0, x0 + SLOT_W, y1);
            let used = or::slot_used(p, k);
            let s = or::slot(p, k);
            let name = or::name_of(&s);
            let tip = if used {
                format!(
                    "{} · {}\nClick: recall ({}) · Shift-click: keep the rings here · right-click: name, clear, library",
                    self.slot_title(k, &name),
                    self.slot_summary(&s),
                    LAUNCH_TIPS[or::launch(p)]
                )
            } else {
                format!(
                    "Button {} · empty\nShift-click (or STORE, then click): keep the rings here · right-click: fill it from the library",
                    k + 1
                )
            };
            let resp =
                v.ui.interact(rect, self.uid(("slot", k)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(tip);
            let hot = resp.hovered();
            let is_active = active == Some(k);
            let waiting = pending == Some(k);
            let blink = ((self.now * 4.0) as i64) % 2 == 0;
            let fill = if waiting && blink {
                alpha(self.accent, 0.35)
            } else if hot {
                theme::CONTROL_HI
            } else if used {
                theme::CONTROL
            } else {
                theme::INSET
            };
            v.painter.rect_filled(rect, 4.0 * z, fill);
            if view.storing {
                let pulse = 0.5 + 0.5 * (self.now as f32 * 6.0).sin();
                v.painter.rect_stroke(
                    rect,
                    4.0 * z,
                    Stroke::new(1.5 * z, alpha(theme::MUTE, 0.5 + 0.5 * pulse)),
                    StrokeKind::Inside,
                );
            } else if is_active {
                v.painter.rect_stroke(
                    rect,
                    4.0 * z,
                    Stroke::new(1.5 * z, self.accent),
                    StrokeKind::Inside,
                );
            } else if !used {
                v.painter.rect_stroke(
                    rect,
                    4.0 * z,
                    Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)),
                    StrokeKind::Inside,
                );
            }
            let c = v.p(x0 + 14.0, (y0 + y1) * 0.5);
            if used {
                glyph(v.painter, c, 10.0 * z, &s, z);
            } else {
                v.painter.circle_stroke(
                    c,
                    9.0 * z,
                    Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.4)),
                );
            }
            let title = self.slot_title(k, &name);
            let ink = if used {
                if is_active {
                    theme::TEXT
                } else {
                    theme::TEXT_DIM
                }
            } else {
                theme::TEXT_FAINT
            };
            let shown = if is_active && edited {
                format!("{title} •")
            } else {
                title
            };
            label(
                v.painter,
                v.p(x0 + 27.0, y0 + 9.0),
                Align2::LEFT_CENTER,
                shown,
                v.font(7.5),
                ink,
            );
            let sub = if waiting {
                "waiting…".to_owned()
            } else if used {
                if s[slot::MODE] > 0.5 {
                    "CYCLE".to_owned()
                } else {
                    "GRID".to_owned()
                }
            } else {
                format!("{} · empty", k + 1)
            };
            label(
                v.painter,
                v.p(x0 + 27.0, y0 + 19.0),
                Align2::LEFT_CENTER,
                sub,
                v.font(6.5),
                if waiting {
                    self.accent
                } else {
                    theme::TEXT_FAINT
                },
            );

            if resp.clicked() {
                if view.storing || shift {
                    or::store(p, k);
                    view.storing = false;
                } else if used {
                    or::recall(p, k);
                }
            }
            resp.context_menu(|ui| self.slot_menu(ui, k));
        }

        let x = PAD + SLOTS as f32 * (SLOT_W + 4.0) + 4.0;
        let store = widgets::toggle(v.ui, v.painter, self.uid("store"), v.r(x, y0 + 4.0, x + 46.0, y1 - 4.0), "STORE", view.storing, theme::MUTE, z)
            .on_hover_text("Keep the rings in a button: press STORE, then click the button (or Shift-click a button)");
        if store.clicked() {
            view.storing = !view.storing;
        }
        const LAUNCHES: [&str; 3] = ["NOW", "BEAT", "BAR"];
        let picked = segmented(
            v,
            self.uid("launch"),
            v.r(x + 52.0, y0 + 4.0, x + 160.0, y1 - 4.0),
            3,
            or::launch(p),
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    LAUNCHES[k],
                    v.font(7.0),
                    col,
                );
                LAUNCH_TIPS[k]
            },
        );
        if let Some(k) = picked {
            p.data[or::data::LAUNCH].set(k as f32);
        }
        let menu_rect = v.r(x + 166.0, y0 + 4.0, X1, y1 - 4.0);
        let menu = widgets::toggle(v.ui, v.painter, self.uid("library"), menu_rect, "PATTERNS ▾", false, self.accent, z)
            .on_hover_text("The library of named patterns, kept for every project: save the rings, load a pattern or put it on a button");
        egui::Popup::menu(&menu)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.library(ui));
        if v.ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            view.storing = false;
        }
    }

    /// "Groove", or "P3" for a button with no name.
    fn slot_title(&self, k: usize, name: &str) -> String {
        if name.is_empty() {
            format!("P{}", k + 1)
        } else {
            name.to_owned()
        }
    }

    /// "5 rings · GRID 1/16" for a pattern.
    fn slot_summary(&self, s: &Pattern) -> String {
        let on = (0..RINGS)
            .filter(|&r| or::pattern_ring(s, r, slot::ON) > 0.5)
            .count();
        let clock = if s[slot::MODE] > 0.5 {
            format!(
                "CYCLE {}",
                or::beats_label(s[slot::CYCLE].round().max(1.0) as u32)
            )
        } else {
            format!("GRID {}", or::spec(RATE).format(s[slot::RATE]))
        };
        format!("{on} rings · {clock}")
    }

    /// A button's right-click menu: keep the rings in it, name it, clear it,
    /// or fill it from the library.
    fn slot_menu(&self, ui: &mut egui::Ui, k: usize) {
        let p = self.p;
        ui.set_min_width(210.0);
        ui.weak(format!("Button {}", k + 1));
        if ui.button("Keep the rings here").clicked() {
            or::store(p, k);
            ui.close();
        }
        if or::slot_used(p, k) {
            ui.horizontal(|ui| {
                ui.label("Name");
                let mut name = or::slot_name(p, k);
                let field = egui::TextEdit::singleline(&mut name)
                    .char_limit(slot::NAME_CHARS)
                    .desired_width(130.0)
                    .hint_text(format!("P{}", k + 1));
                if ui.add(field).changed() {
                    or::rename_slot(p, k, &name);
                }
            });
            if ui
                .button("Recall now")
                .on_hover_text("Bring it in at once, whatever LAUNCH says")
                .clicked()
            {
                or::take(p, k);
                ui.close();
            }
            if ui.button("Clear").clicked() {
                or::clear_slot(p, k);
                ui.close();
            }
        }
        ui.separator();
        ui.weak("Fill from the library");
        for (i, name) in or::FACTORY_NAMES.iter().enumerate() {
            if ui.button(*name).clicked() {
                or::set_slot(p, k, &or::factory(i));
                ui.close();
            }
        }
        let saved = saved_patterns(ui);
        if !saved.is_empty() {
            ui.separator();
            for preset in saved.iter() {
                if ui.button(&preset.name).clicked()
                    && let Some(s) = pattern_of(preset)
                {
                    or::set_slot(p, k, &s);
                    ui.close();
                }
            }
        }
    }

    /// PATTERNS ▾: save the rings under a name; load a pattern into the
    /// rings (as LAUNCH says) or put it on a button; delete a saved one.
    fn library(&self, ui: &mut egui::Ui) {
        let p = self.p;
        let name_id = self.uid("library-name");
        let error_id = self.uid("library-error");
        ui.set_min_width(330.0);
        ui.label("Save the rings as");
        let mut name: String = ui
            .data(|d| d.get_temp(name_id))
            .unwrap_or_else(|| "My pattern".to_owned());
        let mut save = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(slot::NAME_CHARS)
                    .desired_width(180.0),
            );
            save = ui.button("Save").clicked()
                || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        });
        ui.data_mut(|d| d.insert_temp(name_id, name.clone()));
        if save && !name.trim().is_empty() {
            let mut s = or::capture(p);
            or::set_name(&mut s, name.trim());
            let mut list = (*saved_patterns(ui)).clone();
            presets::store(
                &mut list,
                ModulePreset {
                    name: or::name_of(&s),
                    params: Default::default(),
                    data: s.to_vec(),
                    mode: None,
                    toggle: None,
                },
            );
            let result = keep_patterns(ui, list);
            ui.data_mut(|d| d.insert_temp(error_id, result.err().unwrap_or_default()));
        }
        if let Some(error) = ui
            .data(|d| d.get_temp::<String>(error_id))
            .filter(|e| !e.is_empty())
        {
            ui.colored_label(theme::MUTE, format!("Could not save: {error}"));
        }
        ui.separator();
        ui.weak("Click a name to load it into the rings; 1–8 put it on a button");
        ui.weak("Built in");
        for (i, name) in or::FACTORY_NAMES.iter().enumerate() {
            self.library_row(ui, name, &or::factory(i), None);
        }
        let saved = saved_patterns(ui);
        if !saved.is_empty() {
            ui.separator();
            ui.weak("Saved");
            let mut remove = None;
            for (i, preset) in saved.iter().enumerate() {
                if let Some(s) = pattern_of(preset) {
                    self.library_row(ui, &preset.name, &s, Some((i, &mut remove)));
                }
            }
            if let Some(i) = remove {
                let mut list = (*saved).clone();
                list.remove(i);
                let result = keep_patterns(ui, list);
                ui.data_mut(|d| d.insert_temp(error_id, result.err().unwrap_or_default()));
            }
        }
    }

    /// A pattern of the library: its name (load it), a button per slot (put
    /// it there), and × for a saved one.
    fn library_row(
        &self,
        ui: &mut egui::Ui,
        name: &str,
        s: &Pattern,
        delete: Option<(usize, &mut Option<usize>)>,
    ) {
        let p = self.p;
        ui.horizontal(|ui| {
            let load = ui
                .add_sized([110.0, 18.0], egui::Button::new(name))
                .on_hover_text(self.slot_summary(s));
            if load.clicked() {
                or::cue(p, s);
                ui.close();
            }
            for k in 0..SLOTS {
                let taken = or::slot_used(p, k);
                let button = egui::Button::new(egui::RichText::new(format!("{}", k + 1)).small())
                    .min_size(egui::vec2(18.0, 18.0));
                let tip = if taken {
                    format!(
                        "Put it on button {} (now {})",
                        k + 1,
                        self.slot_title(k, &or::slot_name(p, k))
                    )
                } else {
                    format!("Put it on button {}", k + 1)
                };
                if ui.add(button).on_hover_text(tip).clicked() {
                    or::set_slot(p, k, s);
                }
            }
            if let Some((i, remove)) = delete
                && ui
                    .small_button("×")
                    .on_hover_text("Delete this pattern")
                    .clicked()
            {
                *remove = Some(i);
            }
        });
    }

    // -----------------------------------------------------------------------
    // The orbits
    // -----------------------------------------------------------------------

    fn orbits(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(DISPLAY[0], DISPLAY[1], DISPLAY[2], DISPLAY[3]);
        v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let painter = v
            .painter
            .with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let o = Orbits {
            c: rect.center(),
            z,
        };

        // Hover: the ring and step under the pointer.
        let resp =
            v.ui.interact(rect, self.uid("orbits"), Sense::click_and_drag());
        let hover = resp.hover_pos().and_then(|q| o.ring_at(q).map(|r| (r, q)));
        let near = hover.and_then(|(r, q)| self.step_near(o, r, q).map(|i| (r, i)));

        self.backdrop(&painter, o);
        let sel = or::selected(p);
        for r in 0..RINGS {
            self.ring(&painter, o, r, r == sel, view);
        }
        self.hand(&painter, o);
        self.pings(&painter, o, view);
        if let Some((r, i)) = near {
            let pos = self.step_pos(o, r, i);
            painter.circle_stroke(pos, 7.0 * z, Stroke::new(1.2 * z, theme::TEXT));
        } else if let Some((r, _)) = hover {
            painter.circle_stroke(
                o.c,
                o.radius(r),
                Stroke::new(3.0 * z, alpha(COLORS[r], 0.12)),
            );
        }
        self.hub(&painter, o);
        painter.rect_stroke(
            rect,
            4.0 * z,
            Stroke::new(1.0 * z, alpha(theme::NODE_BORDER, 0.9)),
            StrokeKind::Inside,
        );

        // Clicks switch and accent steps, drags turn rings.
        let tip = match (near, hover) {
            (Some((r, i)), _) => {
                let state = match or::step(p, r, i) {
                    OFF => "rest",
                    ON => "hit",
                    _ => "accent",
                };
                format!(
                    "Ring {} · step {} · {state}\nClick: hit or rest · right-click: accent · drag round: turn the ring (Shift: freely)",
                    r + 1,
                    i + 1
                )
            }
            (None, Some((r, _))) => format!(
                "Ring {} · {} · turned {}\nDrag round to turn it (Shift: freely); click a step to switch it",
                r + 1,
                or::euclid_name(p, r),
                turn_label(or::rotation(p, r))
            ),
            _ => "Drag a ring round to turn it; click its steps to switch them".to_owned(),
        };
        let cursor = if view.turning.is_some() {
            CursorIcon::Grabbing
        } else if near.is_some() {
            CursorIcon::PointingHand
        } else if hover.is_some() {
            CursorIcon::Grab
        } else {
            CursorIcon::Default
        };
        let resp = resp.on_hover_cursor(cursor).on_hover_text(tip);
        let shift = v.ui.input(|i| i.modifiers.shift);
        if resp.clicked() || resp.secondary_clicked() {
            if let Some((r, _)) = hover {
                or::select(p, r);
            }
            if let Some((r, i)) = near {
                let now = or::step(p, r, i);
                let accent = resp.secondary_clicked() || shift;
                let next = match (accent, now) {
                    (false, OFF) => ON,
                    (false, _) => OFF,
                    (true, ACCENTED) => ON,
                    (true, _) => ACCENTED,
                };
                or::set_step(p, r, i, next);
            }
        }
        if resp.drag_started() {
            let origin =
                v.ui.input(|i| i.pointer.press_origin())
                    .or(resp.interact_pointer_pos());
            if let Some(q) = origin
                && let Some(r) = o.ring_at(q)
            {
                or::select(p, r);
                view.turning = Some(Turning {
                    ring: r,
                    angle: o.angle_of(q),
                    steps: or::get(p, r, ring::ROTATE),
                });
            }
        }
        if resp.dragged()
            && let (Some(t), Some(q)) = (view.turning.as_mut(), resp.interact_pointer_pos())
        {
            let a = o.angle_of(q);
            let mut d = a - t.angle;
            if d > PI {
                d -= TAU;
            } else if d < -PI {
                d += TAU;
            }
            t.angle = a;
            let n = or::steps(p, t.ring) as f32;
            t.steps += d / TAU * n;
            let value = if shift { t.steps } else { t.steps.round() };
            if (value.rem_euclid(n) - or::rotation(p, t.ring)).abs() > 1e-4 {
                or::set_rotation(p, t.ring, value);
            }
        }
        if !resp.dragged() {
            view.turning = None;
        }
    }

    /// Where step `i` of ring `r` sits, turned.
    fn step_pos(&self, o: Orbits, r: usize, i: usize) -> Pos2 {
        let n = or::steps(self.p, r);
        o.at(o.radius(r), angle(i as f32 + or::turn(self.p, r), n))
    }

    /// The step of ring `r` near a point, if one is close enough to click.
    fn step_near(&self, o: Orbits, r: usize, q: Pos2) -> Option<usize> {
        let n = or::steps(self.p, r);
        let a = (o.angle_of(q) + FRAC_PI_2).rem_euclid(TAU);
        let u = a / TAU * n as f32 - or::turn(self.p, r);
        let i = (u.round() as i64).rem_euclid(n as i64) as usize;
        let reach = (7.0 * self.z()).max(o.radius(r) * TAU / n as f32 * 0.45);
        ((self.step_pos(o, r, i) - q).length() <= reach.min(9.0 * self.z())).then_some(i)
    }

    /// A dot matrix, the quarters, and the bezel: the cycle's beats (CYCLE)
    /// or the selected ring's steps (GRID).
    fn backdrop(&self, painter: &Painter, o: Orbits) {
        let (p, z) = (self.p, self.z());
        let rect = painter.clip_rect();
        let step = 20.0 * z;
        let mut x = o.c.x - (((o.c.x - rect.left()) / step).floor()) * step;
        while x < rect.right() {
            let mut y = o.c.y - (((o.c.y - rect.top()) / step).floor()) * step;
            while y < rect.bottom() {
                let d = (pos2(x, y) - o.c).length();
                if d > (BEZEL + 8.0) * z || d < (R_IN - 26.0) * z {
                    painter.circle_filled(pos2(x, y), 0.8 * z, alpha(theme::TEXT_FAINT, 0.35));
                }
                y += step;
            }
            x += step;
        }
        for k in 0..4 {
            let a = k as f32 * FRAC_PI_2;
            dashed_line(
                painter,
                o.at((R_IN - 14.0) * z, a),
                o.at((R_OUT + 6.0) * z, a),
                2.0 * z,
                5.0 * z,
                Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.25)),
            );
        }
        painter.circle_stroke(
            o.c,
            BEZEL * z,
            Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.35)),
        );
        if self.cycle() {
            let beats = or::cycle_beats(p) as usize;
            for b in 0..beats {
                let a = angle(b as f32, beats);
                let bar = b % 4 == 0;
                let len = if bar { 7.0 } else { 4.0 };
                painter.line_segment(
                    [o.at((BEZEL - len) * z, a), o.at(BEZEL * z, a)],
                    Stroke::new(
                        if bar { 1.5 } else { 1.0 } * z,
                        alpha(theme::TEXT_DIM, if bar { 0.8 } else { 0.5 }),
                    ),
                );
                if beats <= 16 {
                    label(
                        painter,
                        o.at((BEZEL + 9.0) * z, a),
                        Align2::CENTER_CENTER,
                        b + 1,
                        theme::mono(7.0 * z),
                        theme::TEXT_FAINT,
                    );
                }
            }
        } else {
            let r = or::selected(p);
            let n = or::steps(p, r);
            for i in 0..n {
                let a = angle(i as f32, n);
                let major = i % 4 == 0;
                let len = if major { 6.0 } else { 3.5 };
                painter.line_segment(
                    [o.at((BEZEL - len) * z, a), o.at(BEZEL * z, a)],
                    Stroke::new(1.0 * z, alpha(COLORS[r], if major { 0.7 } else { 0.4 })),
                );
            }
            label(
                painter,
                o.at((BEZEL + 9.0) * z, -FRAC_PI_2),
                Align2::CENTER_CENTER,
                "1",
                theme::mono(7.0 * z),
                theme::TEXT_FAINT,
            );
        }
    }

    /// Ring `r`: its track, the rhythm's polygon, the steps, the notch on
    /// its first step and its planet.
    fn ring(&self, painter: &Painter, o: Orbits, r: usize, selected: bool, view: &View) {
        let (p, z) = (self.p, self.z());
        let color = COLORS[r];
        let on = or::is_on(p, r);
        let ink = if on { 1.0 } else { 0.32 };
        let rad = o.radius(r);
        let n = or::steps(p, r);
        let turned = or::turn(p, r);
        let track = Stroke::new(
            1.0 * z,
            alpha(color, if selected { 0.45 } else { 0.2 } * ink),
        );
        if on {
            painter.circle_stroke(o.c, rad, track);
        } else {
            dashed_circle(
                painter,
                o.c,
                rad,
                (rad / (6.0 * z)).round().max(8.0) as usize,
                0.0,
                track,
            );
        }

        // The polygon of the hits.
        let pts: Vec<Pos2> = (0..n)
            .filter(|&i| or::step(p, r, i) != OFF)
            .map(|i| o.at(rad, angle(i as f32 + turned, n)))
            .collect();
        if pts.len() >= 2 {
            let mut closed = pts.clone();
            closed.push(pts[0]);
            let stroke = Stroke::new(
                if selected { 1.4 } else { 0.9 } * z,
                alpha(color, if selected { 0.75 } else { 0.35 } * ink),
            );
            painter.add(Shape::line(closed, stroke));
        }

        // The notch on the first step: it turns with the ring.
        let a0 = angle(turned, n);
        let tip = o.at(rad + 3.5 * z, a0);
        let side = Vec2::angled(a0 + FRAC_PI_2) * 3.0 * z;
        let back = o.at(rad + 8.5 * z, a0);
        painter.add(Shape::convex_polygon(
            vec![tip, back + side, back - side],
            alpha(color, 0.85 * ink),
            Stroke::NONE,
        ));

        // The steps.
        let live = self.live && on;
        for i in 0..n {
            let at = o.at(rad, angle(i as f32 + turned, n));
            let flash = if live {
                view.flash(r, i, self.now)
            } else {
                0.0
            };
            match or::step(p, r, i) {
                OFF => {
                    painter.circle_filled(
                        at,
                        1.3 * z,
                        alpha(theme::TEXT_FAINT, 0.7 * ink.max(0.6)),
                    );
                }
                value => {
                    let size = if value == ACCENTED { 4.6 } else { 3.6 } * z;
                    let width = (1.5 + 1.2 * flash) * z;
                    painter.circle_filled(at, size, theme::INSET);
                    if flash > 0.0 {
                        painter.circle_filled(at, size, alpha(color, 0.9 * flash));
                    }
                    painter.circle_stroke(
                        at,
                        size,
                        Stroke::new(width, alpha(mix(color, theme::TEXT, 0.6 * flash), ink)),
                    );
                    if value == ACCENTED {
                        painter.circle_filled(at, 1.6 * z, alpha(color, ink));
                    }
                }
            }
        }

        // The planet, with a wake of dots behind it.
        let phase = p.viz[viz::PHASE + r].get();
        let a = angle(phase, n);
        if self.live {
            let gap = 0.045;
            for k in 1..=6 {
                let q = o.at(rad, a - gap * k as f32);
                painter.circle_filled(
                    q,
                    (2.2 - 0.25 * k as f32).max(0.6) * z,
                    alpha(theme::TEXT, (0.5 - 0.075 * k as f32) * ink),
                );
            }
        }
        let planet = o.at(rad, a);
        let size = if selected { 4.8 } else { 4.0 } * z;
        painter.circle_filled(planet, size, theme::INSET);
        painter.circle_stroke(
            planet,
            size,
            Stroke::new(
                1.5 * z,
                alpha(
                    if self.live {
                        theme::TEXT
                    } else {
                        theme::TEXT_FAINT
                    },
                    ink,
                ),
            ),
        );
    }

    /// CYCLE: one hand sweeps every ring, a fan of fading lines behind it.
    fn hand(&self, painter: &Painter, o: Orbits) {
        if !self.cycle() {
            return;
        }
        let (p, z) = (self.p, self.z());
        let n = or::steps(p, 0);
        let a = angle(p.viz[viz::PHASE].get(), n);
        let (inner, outer) = ((R_IN - 16.0) * z, (BEZEL - 2.0) * z);
        if self.live {
            for k in 1..=10 {
                let b = a - k as f32 * 0.012;
                painter.line_segment(
                    [o.at(inner, b), o.at(outer, b)],
                    Stroke::new(1.0 * z, alpha(self.accent, 0.16 * (1.0 - k as f32 / 11.0))),
                );
            }
        }
        let color = if self.live {
            theme::TEXT
        } else {
            theme::TEXT_FAINT
        };
        painter.line_segment(
            [o.at(inner, a), o.at(outer, a)],
            Stroke::new(1.2 * z, alpha(color, 0.6)),
        );
        let tip = o.at(BEZEL * z + 1.0 * z, a);
        let side = Vec2::angled(a + FRAC_PI_2) * 3.5 * z;
        let back = o.at((BEZEL + 8.0) * z, a);
        painter.add(Shape::convex_polygon(
            vec![tip, back + side, back - side],
            color,
            Stroke::NONE,
        ));
    }

    /// Each hit sends a ring out from its step.
    fn pings(&self, painter: &Painter, o: Orbits, view: &View) {
        let z = self.z();
        for g in &view.pings {
            if g.step >= or::steps(self.p, g.ring) {
                continue;
            }
            let t = ((self.now - g.at) / PING) as f32;
            let at = self.step_pos(o, g.ring, g.step);
            let ease = 1.0 - (1.0 - t).powi(2);
            painter.circle_stroke(
                at,
                (5.0 + 13.0 * ease) * z,
                Stroke::new(1.3 * z, alpha(COLORS[g.ring], 0.8 * (1.0 - t))),
            );
        }
    }

    /// The hub: the selected ring's Euclidean name and its turn.
    fn hub(&self, painter: &Painter, o: Orbits) {
        let (p, z) = (self.p, self.z());
        let r = or::selected(p);
        let color = COLORS[r];
        let spin = (p.viz[viz::PHASE].get() / or::steps(p, 0) as f32) * TAU * 0.25;
        painter.circle_filled(o.c, (R_IN - 20.0) * z, theme::INSET);
        dashed_circle(
            painter,
            o.c,
            (R_IN - 20.0) * z,
            20,
            spin,
            Stroke::new(1.0 * z, alpha(color, 0.5)),
        );
        label(
            painter,
            o.c + vec2(0.0, -15.0 * z),
            Align2::CENTER_CENTER,
            format!("RING {}", r + 1),
            theme::strong(7.0 * z),
            color,
        );
        label(
            painter,
            o.c + vec2(0.0, 1.0 * z),
            Align2::CENTER_CENTER,
            or::euclid_name(p, r),
            theme::mono(11.0 * z),
            theme::TEXT,
        );
        let turned = or::turn(p, r);
        let sub = if turned.abs() < 0.005 {
            note_label(or::note(p, r))
        } else {
            format!("↻ {}", turn_label(turned))
        };
        label(
            painter,
            o.c + vec2(0.0, 16.0 * z),
            Align2::CENTER_CENTER,
            sub,
            theme::mono(7.0 * z),
            theme::TEXT_DIM,
        );
    }

    // -----------------------------------------------------------------------
    // The rings' table
    // -----------------------------------------------------------------------

    #[allow(clippy::needless_range_loop)] // a row per ring, by its index
    fn table(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        self.group_title("RINGS", X0, X1, TABLE_Y, theme::TEXT_DIM);
        let head = TABLE_Y + 12.0;
        for (cols, name) in [
            (COL_ON, "ON"),
            (COL_STEPS, "STEPS"),
            (COL_PULSES, "PULSES"),
            (COL_ROT, "ROT"),
            (COL_NOTE, "NOTE"),
            (COL_CH, "CH"),
        ] {
            v.text(
                (cols[0] + cols[1]) * 0.5,
                head,
                Align2::CENTER_CENTER,
                name,
                v.font(6.5),
                theme::TEXT_FAINT,
            );
        }
        v.text(
            COL_PATTERN[0],
            head,
            Align2::LEFT_CENTER,
            "PATTERN",
            v.font(6.5),
            theme::TEXT_FAINT,
        );
        let sel = or::selected(p);
        for r in 0..RINGS {
            let (y0, y1) = (
                ROW0 + ROW_H * r as f32,
                ROW0 + ROW_H * r as f32 + ROW_H - 3.0,
            );
            let color = COLORS[r];
            let on = or::is_on(p, r);
            if r == sel {
                v.painter.rect_filled(
                    v.r(X0 - 4.0, y0 - 1.0, X1 + 2.0, y1 + 1.0),
                    3.0 * z,
                    theme::CONTROL,
                );
                v.painter
                    .rect_filled(v.r(X0 - 4.0, y0 - 1.0, X0 - 2.0, y1 + 1.0), 1.0 * z, color);
            }
            // The ring's number: click to select it.
            let num = v.r(COL_NUM[0], y0, COL_NUM[1], y1);
            let resp =
                v.ui.interact(num, self.uid(("num", r)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(format!("Ring {}: select it", r + 1));
            if resp.clicked() {
                or::select(p, r);
            }
            let c = num.center();
            v.painter.circle_stroke(
                c,
                7.5 * z,
                Stroke::new(1.3 * z, alpha(color, if on { 1.0 } else { 0.4 })),
            );
            label(
                v.painter,
                c,
                Align2::CENTER_CENTER,
                r + 1,
                theme::strong(7.0 * z),
                if on { color } else { alpha(color, 0.5) },
            );

            if self.switch(
                ("on", r),
                v.r(COL_ON[0], y0 + 2.0, COL_ON[1], y1 - 2.0),
                on,
                color,
                "Play this ring (off: it keeps turning, silent)",
            ) {
                or::set(p, r, ring::ON, if on { 0.0 } else { 1.0 });
            }
            let n = or::steps(p, r);
            let k = or::pulses(p, r);
            if let Some(value) = self.drag_box(
                ("steps", r),
                v.r(COL_STEPS[0], y0, COL_STEPS[1], y1),
                n as f32,
                (1.0, 32.0, 16.0, 1.0),
                &n.to_string(),
                "Steps round the ring, 1–32 (drag; double-click: 16). The rhythm is laid again",
                on,
            ) {
                or::set_steps(p, r, value as usize);
                or::select(p, r);
            }
            if let Some(value) = self.drag_box(("pulses", r), v.r(COL_PULSES[0], y0, COL_PULSES[1], y1), k as f32, (0.0, n as f32, (n as f32 / 4.0).round(), 1.0), &k.to_string(), "Hits spread over the steps as evenly as they go (drag; double-click: a quarter). Lays the rhythm again, over your edits", on) {
                or::set_pulses(p, r, value as usize);
                or::select(p, r);
            }
            let rot = or::rotation(p, r);
            if let Some(value) = self.drag_box(("rot", r), v.r(COL_ROT[0], y0, COL_ROT[1], y1), rot.round(), (-1.0, n as f32, 0.0, 1.0), &turn_label(rot), "How far the ring is turned, in steps: its hits come that much later (drag; double-click: 0).\nDrag the ring itself to turn it, with Shift by parts of a step", on) {
                or::set_rotation(p, r, value);
                or::select(p, r);
            }
            let note = or::note(p, r);
            let tip = match or::drum_name(note) {
                Some(drum) => format!(
                    "The ring's note: {} (General MIDI: {drum}) · drag; double-click: C2",
                    note_label(note)
                ),
                None => format!(
                    "The ring's note: {} · drag; double-click: C2",
                    note_label(note)
                ),
            };
            if let Some(value) = self.drag_box(
                ("note", r),
                v.r(COL_NOTE[0], y0, COL_NOTE[1], y1),
                note as f32,
                (0.0, 127.0, 36.0, 1.0),
                &note_label(note),
                &tip,
                on,
            ) {
                or::set(p, r, ring::NOTE, value);
            }
            let ch = or::channel(p, r);
            if let Some(value) = self.drag_box(
                ("ch", r),
                v.r(COL_CH[0], y0, COL_CH[1], y1),
                ch as f32,
                (1.0, 16.0, 10.0, 1.0),
                &ch.to_string(),
                "The ring's MIDI channel, 1–16 (drag; double-click: 10, drums)",
                on,
            ) {
                or::set(p, r, ring::CHANNEL, value);
            }
            self.mini(r, v.r(COL_PATTERN[0], y0 + 3.0, COL_PATTERN[1], y1 - 3.0));
        }
    }

    /// A ring's pattern laid out flat, the step playing marked. Click it to
    /// select the ring.
    fn mini(&self, r: usize, rect: Rect) {
        let (v, p, z) = (self.v, self.p, self.z());
        let color = COLORS[r];
        let on = or::is_on(p, r);
        let ink = if on { 1.0 } else { 0.4 };
        let n = or::steps(p, r);
        let cw = rect.width() / n as f32;
        let playing = p.viz[viz::STEP + r].get();
        for i in 0..n {
            let x = rect.left() + cw * (i as f32 + 0.5);
            let value = or::step(p, r, i);
            if value == OFF {
                v.painter.circle_filled(
                    pos2(x, rect.center().y),
                    0.9 * z,
                    alpha(theme::TEXT_FAINT, 0.8),
                );
            } else {
                let h = if value == ACCENTED {
                    rect.height()
                } else {
                    rect.height() * 0.62
                };
                let bar = Rect::from_center_size(
                    pos2(x, rect.center().y),
                    vec2((cw - 1.2 * z).clamp(1.0 * z, 4.0 * z), h),
                );
                v.painter.rect_filled(bar, 1.0 * z, alpha(color, ink));
            }
            if self.live && on && playing >= 0.0 && i == playing as usize {
                v.painter.hline(
                    (x - cw * 0.5)..=(x + cw * 0.5),
                    rect.bottom() + 2.0 * z,
                    Stroke::new(1.5 * z, theme::TEXT),
                );
            }
        }
        let resp =
            v.ui.interact(rect.expand(3.0 * z), self.uid(("mini", r)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
        if resp
            .on_hover_text(format!(
                "Ring {} · {} · {} hits",
                r + 1,
                or::euclid_name(p, r),
                or::hits(p, r)
            ))
            .clicked()
        {
            or::select(p, r);
        }
    }

    // -----------------------------------------------------------------------
    // The selected ring
    // -----------------------------------------------------------------------

    fn selected(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let r = or::selected(p);
        let color = COLORS[r];
        let title = v.text(
            X0,
            RING_Y,
            Align2::LEFT_CENTER,
            format!("RING {}", r + 1),
            theme::strong(8.5 * z),
            color,
        );
        let note = or::note(p, r);
        let what = match or::drum_name(note) {
            Some(drum) => format!(
                "{} · {} {drum} · ch {}",
                or::euclid_name(p, r),
                note_label(note),
                or::channel(p, r)
            ),
            None => format!(
                "{} · {} · ch {}",
                or::euclid_name(p, r),
                note_label(note),
                or::channel(p, r)
            ),
        };
        let x = (title.right() - v.min.x) / z + 8.0;
        let detail = v.text(
            x,
            RING_Y,
            Align2::LEFT_CENTER,
            what,
            v.font(7.5),
            theme::TEXT_DIM,
        );
        let from = detail.right() + 8.0 * z;
        if from < v.p(X1, 0.0).x {
            v.painter.hline(
                from..=v.p(X1, 0.0).x,
                v.p(0.0, RING_Y).y,
                Stroke::new(1.0 * z, alpha(color, 0.3)),
            );
        }
        self.strip(r, view);
        self.buttons(r);
        let cw = (X1 - X0) / 5.0;
        for (k, &(i, name, lo, hi, default, tip)) in RING_KNOBS.iter().enumerate() {
            self.ring_knob(
                r,
                i,
                name,
                (lo, hi, default),
                tip,
                X0 + cw * (k as f32 + 0.5),
                RING_KNOBS_Y,
            );
        }
        // How full the ring is.
        let (hits, n) = (or::hits(p, r), or::steps(p, r));
        let cx = X0 + cw * 4.0;
        v.text(
            cx,
            RING_KNOBS_Y + 10.0,
            Align2::CENTER_CENTER,
            format!("{hits}/{n}"),
            v.mono(14.0),
            theme::TEXT,
        );
        v.text(
            cx,
            RING_KNOBS_Y + 26.0,
            Align2::CENTER_CENTER,
            format!("hits · {:.0}%", 100.0 * hits as f32 / n as f32),
            v.font(7.0),
            theme::TEXT_DIM,
        );
        let turned = or::turn(p, r);
        let twist = r as f32 * p.get(TWIST);
        let line = if twist.abs() < 0.005 {
            format!("turned {}", turn_label(turned))
        } else {
            format!("turned {} ({:+.2} twist)", turn_label(turned), twist)
        };
        v.text(
            cx,
            RING_KNOBS_Y + 38.0,
            Align2::CENTER_CENTER,
            line,
            v.font(6.5),
            theme::TEXT_FAINT,
        );
    }

    /// The selected ring's steps in a row: click to switch, right-click
    /// (or Shift-click) to accent, drag to paint.
    fn strip(&self, r: usize, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let color = COLORS[r];
        let n = or::steps(p, r);
        let [y0, y1] = STRIP;
        let cw = (X1 - X0) / n as f32;
        let playing = p.viz[viz::STEP + r].get();
        let on = or::is_on(p, r);
        let rect = v.r(X0, y0, X1, y1);
        for i in 0..n {
            let cell = v.r(
                X0 + cw * i as f32 + 1.0,
                y0,
                X0 + cw * (i + 1) as f32 - 1.0,
                y1,
            );
            let value = or::step(p, r, i);
            let flash = if self.live && on {
                view.flash(r, i, self.now)
            } else {
                0.0
            };
            v.painter.rect_filled(cell, 2.0 * z, theme::INSET);
            match value {
                OFF => {
                    v.painter.rect_stroke(
                        cell,
                        2.0 * z,
                        Stroke::new(1.0 * z, alpha(theme::CONTROL_HI, 0.8)),
                        StrokeKind::Inside,
                    );
                }
                _ => {
                    if flash > 0.0 {
                        v.painter
                            .rect_filled(cell, 2.0 * z, alpha(color, 0.8 * flash));
                    }
                    v.painter.rect_stroke(
                        cell,
                        2.0 * z,
                        Stroke::new(1.5 * z, color),
                        StrokeKind::Inside,
                    );
                    if value == ACCENTED {
                        v.painter.rect_filled(
                            cell.shrink(cell.width().min(cell.height()) * 0.3),
                            1.0 * z,
                            color,
                        );
                    }
                }
            }
            if i % 4 == 0 {
                v.painter.hline(
                    cell.x_range(),
                    cell.bottom() + 2.5 * z,
                    Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.6)),
                );
            }
            if self.live && on && playing >= 0.0 && i == playing as usize {
                v.painter.rect_stroke(
                    cell.expand(1.0 * z),
                    2.5 * z,
                    Stroke::new(1.2 * z, theme::TEXT),
                    StrokeKind::Outside,
                );
            }
        }
        let resp =
            v.ui.interact(rect, self.uid("strip"), Sense::click_and_drag())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(
                    "Click: hit or rest · right-click (or Shift-click): accent · drag: paint",
                );
        let cell_at = |q: Pos2| {
            let i = ((q.x - rect.left()) / (cw * z)).floor();
            (0.0..n as f32).contains(&i).then_some(i as usize)
        };
        let shift = v.ui.input(|i| i.modifiers.shift);
        if (resp.clicked() || resp.secondary_clicked())
            && let Some(i) = resp.interact_pointer_pos().and_then(cell_at)
        {
            let now = or::step(p, r, i);
            let next = match (resp.secondary_clicked() || shift, now) {
                (false, OFF) => ON,
                (false, _) => OFF,
                (true, ACCENTED) => ON,
                (true, _) => ACCENTED,
            };
            or::set_step(p, r, i, next);
        }
        if resp.drag_started() {
            let origin = v.ui.input(|i| i.pointer.press_origin());
            view.paint = origin
                .and_then(cell_at)
                .map(|i| if or::step(p, r, i) == OFF { ON } else { OFF });
        }
        if resp.dragged()
            && let (Some(value), Some(i)) =
                (view.paint, resp.interact_pointer_pos().and_then(cell_at))
        {
            let now = or::step(p, r, i);
            if (value == OFF) != (now == OFF) {
                or::set_step(p, r, i, value);
            }
        }
        if !resp.dragged() {
            view.paint = None;
        }
    }

    /// Turn the ring a step either way, lay its rhythm again, invert or
    /// clear it, or throw new dice for it.
    fn buttons(&self, r: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [y0, y1] = BUTTONS;
        let color = COLORS[r];
        let mut x = X0;
        let mut button = |label: &str, width: f32, tip: &str| {
            let rect = v.r(x, y0, x + width, y1);
            x += width + 4.0;
            widgets::toggle(
                v.ui,
                v.painter,
                self.uid(("btn", label)),
                rect,
                label,
                false,
                color,
                z,
            )
            .on_hover_text(tip)
            .clicked()
        };
        if button(
            "◀",
            22.0,
            "Turn the ring back a step: its hits come a step earlier",
        ) {
            or::set_rotation(p, r, or::rotation(p, r) - 1.0);
        }
        if button(
            "▶",
            22.0,
            "Turn the ring on a step: its hits come a step later",
        ) {
            or::set_rotation(p, r, or::rotation(p, r) + 1.0);
        }
        if button("0", 20.0, "Back to no turn") {
            or::set_rotation(p, r, 0.0);
        }
        if button(
            "EUCLID",
            50.0,
            "Lay the Euclidean rhythm of PULSES over STEPS again (over your edits; accents on hits stay)",
        ) {
            or::lay(p, r);
        }
        if button("INVERT", 48.0, "Hits become rests and rests hits") {
            or::invert(p, r);
        }
        if button("CLEAR", 44.0, "Every step a rest") {
            or::clear(p, r);
        }
        if button("DICE", 40.0, "New pulses and turn for this ring") {
            let seed =
                (self.now * 1000.0) as u32 ^ (r as u32 + 1).wrapping_mul(0x85EB_CA6B) ^ self.id;
            or::dice(p, seed, Some(r));
        }
    }

    /// A knob for one of ring `r`'s settings.
    #[allow(clippy::too_many_arguments)]
    fn ring_knob(
        &self,
        r: usize,
        i: usize,
        name: &str,
        (lo, hi, default): (f32, f32, f32),
        tip: &str,
        cx: f32,
        y: f32,
    ) {
        let (v, p, z) = (self.v, self.p, self.z());
        const R: f32 = 10.0;
        v.text(
            cx,
            y,
            Align2::CENTER_CENTER,
            name,
            v.font(7.5),
            theme::TEXT_FAINT,
        );
        let top = y + 8.0;
        let value = or::get(p, r, i).clamp(lo, hi);
        let mut norm = (value - lo) / (hi - lo);
        let resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("rknob", r, i)),
            v.p(cx, top + R),
            R * z,
            &mut norm,
            (default - lo) / (hi - lo),
            false,
            COLORS[r],
            z,
        )
        .on_hover_text(tip);
        if resp.dragged() || resp.double_clicked() {
            let mut new = lo + norm.clamp(0.0, 1.0) * (hi - lo);
            if i == ring::VELOCITY {
                new = new.round();
            }
            or::set(p, r, i, new);
        }
        let shown = if i == ring::VELOCITY {
            format!("{}", value.round())
        } else {
            format!("{:.0}%", value * 100.0)
        };
        let hot = resp.dragged() || resp.hovered();
        v.text(
            cx,
            top + 2.0 * R + 8.0,
            Align2::CENTER_CENTER,
            shown,
            v.mono(7.0),
            if hot { theme::TEXT } else { theme::TEXT_DIM },
        );
    }

    // -----------------------------------------------------------------------
    // The clock
    // -----------------------------------------------------------------------

    fn clock(&self) {
        let cw = (X1 - X0) / 5.0;
        self.group_title("CLOCK", X0, X0 + cw * 3.0 - 8.0, CLOCK_Y, theme::TEXT_DIM);
        self.group_title("FEEL", X0 + cw * 3.0, X1, CLOCK_Y, theme::TEXT_DIM);
        for (k, (i, name)) in [
            (RATE, "RATE"),
            (CYCLE, "CYCLE"),
            (SWING, "SWING"),
            (ACCENT, "ACCENT"),
            (TWIST, "TWIST"),
        ]
        .into_iter()
        .enumerate()
        {
            self.knob(i, name, X0 + cw * (k as f32 + 0.5), CLOCK_Y + 14.0);
        }
    }

    fn group_title(&self, title: &str, x0: f32, x1: f32, y: f32, color: Color32) {
        let (v, z) = (self.v, self.z());
        let r = v.text(
            x0,
            y,
            Align2::LEFT_CENTER,
            title,
            theme::strong(8.5 * z),
            color,
        );
        let from = r.right() + 8.0 * z;
        if from < v.p(x1, 0.0).x {
            v.painter.hline(
                from..=v.p(x1, 0.0).x,
                v.p(0.0, y).y,
                Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)),
            );
        }
    }

    /// A knob for one of the node's own settings (it can be mapped to MIDI).
    fn knob(&self, i: usize, name: &str, cx: f32, y: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        const R: f32 = 10.0;
        let spec = or::spec(i);
        let idle = match i {
            RATE | SWING => self.cycle(),
            CYCLE => !self.cycle(),
            _ => false,
        };
        let faint = |c: Color32| if idle { alpha(c, 0.45) } else { c };
        v.text(
            cx,
            y,
            Align2::CENTER_CENTER,
            name,
            v.font(7.5),
            faint(theme::TEXT_FAINT),
        );
        let top = y + 8.0;
        let mut norm = spec.to_norm(p.get(i));
        let color = if idle { theme::TEXT_FAINT } else { self.accent };
        let resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("knob", i)),
            v.p(cx, top + R),
            R * z,
            &mut norm,
            spec.to_norm(spec.default),
            spec.bipolar,
            color,
            z,
        );
        let tip = TIPS.iter().find(|(k, _)| *k == i).map_or("", |(_, t)| *t);
        let resp = resp.on_hover_text(tip);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let mut value = spec.denorm(norm);
            if matches!(i, RATE | CYCLE) {
                value = value.round();
            }
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(
            cx,
            top + 2.0 * R + 8.0,
            Align2::CENTER_CENTER,
            format(p, i),
            v.mono(7.0),
            faint(if hot { theme::TEXT } else { theme::TEXT_DIM }),
        );
    }

    /// A ring's ON switch, drawn as an outline in its colour: clicked?
    fn switch(
        &self,
        salt: impl std::hash::Hash + std::fmt::Debug,
        rect: Rect,
        on: bool,
        color: Color32,
        tip: &str,
    ) -> bool {
        let (v, z) = (self.v, self.z());
        let resp =
            v.ui.interact(rect, self.uid(salt), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tip);
        let hot = resp.hovered();
        v.painter.rect_filled(
            rect,
            3.0 * z,
            if hot { theme::CONTROL_HI } else { theme::INSET },
        );
        let edge = if on {
            color
        } else {
            alpha(theme::TEXT_FAINT, 0.6)
        };
        v.painter.rect_stroke(
            rect,
            3.0 * z,
            Stroke::new(if on { 1.3 } else { 1.0 } * z, edge),
            StrokeKind::Inside,
        );
        label(
            v.painter,
            rect.center(),
            Align2::CENTER_CENTER,
            if on { "ON" } else { "OFF" },
            v.font(6.5),
            if on { color } else { theme::TEXT_FAINT },
        );
        resp.clicked()
    }

    /// A number to drag (up or right raises it, a step every few pixels;
    /// double-click resets it).
    #[allow(clippy::too_many_arguments)]
    fn drag_box(
        &self,
        salt: impl std::hash::Hash + std::fmt::Debug,
        rect: Rect,
        value: f32,
        (lo, hi, reset, per_step): (f32, f32, f32, f32),
        shown: &str,
        tip: &str,
        on: bool,
    ) -> Option<f32> {
        let (v, z) = (self.v, self.z());
        let id = self.uid(("box", salt));
        let resp =
            v.ui.interact(rect, id, Sense::click_and_drag())
                .on_hover_cursor(CursorIcon::ResizeVertical)
                .on_hover_text(tip);
        let mut out = None;
        if resp.dragged() {
            let d = resp.drag_delta();
            let steps = v.ui.data_mut(|m| {
                let acc = m.get_temp_mut_or(id.with("acc"), 0.0f32);
                *acc += (d.x - d.y) / z;
                let steps = (*acc / 5.0).trunc();
                *acc -= steps * 5.0;
                steps
            });
            if steps != 0.0 {
                out = Some((value + steps * per_step).clamp(lo, hi));
            }
        }
        if resp.double_clicked() {
            out = Some(reset);
        }
        let hot = resp.hovered() || resp.dragged();
        v.painter.rect_filled(
            rect,
            3.0 * z,
            if hot { theme::CONTROL_HI } else { theme::INSET },
        );
        label(
            v.painter,
            rect.center(),
            Align2::CENTER_CENTER,
            shown,
            v.mono(7.5),
            if on { theme::TEXT } else { theme::TEXT_FAINT },
        );
        out
    }
}

/// What each LAUNCH does, for the tooltips.
const LAUNCH_TIPS: [&str; 3] = [
    "now: a recalled pattern comes in at once",
    "on the next beat",
    "on the next bar (CYCLE: on the next turn)",
];

/// A pattern as a little orrery: its rings that play, their hits as dots.
fn glyph(painter: &Painter, c: Pos2, radius: f32, s: &Pattern, z: f32) {
    let on: Vec<usize> = (0..RINGS)
        .filter(|&r| or::pattern_ring(s, r, slot::ON) > 0.5)
        .collect();
    for (r, &color) in COLORS.iter().enumerate() {
        let rad = radius * (1.0 - 0.72 * r as f32 / (RINGS - 1) as f32);
        let lit = on.contains(&r);
        painter.circle_stroke(
            c,
            rad,
            Stroke::new(0.6 * z, alpha(color, if lit { 0.4 } else { 0.08 })),
        );
        if !lit {
            continue;
        }
        let n = (or::pattern_ring(s, r, slot::STEPS).round() as usize).clamp(1, or::MAX_STEPS);
        let turned = or::pattern_ring(s, r, slot::ROTATE) + r as f32 * s[slot::TWIST];
        for i in 0..n {
            if or::pattern_step(s, r, i) != OFF {
                painter.circle_filled(
                    c + Vec2::angled(angle(i as f32 + turned, n)) * rad,
                    0.9 * z,
                    color,
                );
            }
        }
    }
}

/// Where the library of patterns is kept.
fn library_file() -> Option<std::path::PathBuf> {
    presets::dir().map(|d| d.join("Orrery patterns.json"))
}

fn library_id() -> Id {
    Id::unique("orrery-pattern-library")
}

/// The saved patterns, read once and kept for the session.
fn saved_patterns(ui: &egui::Ui) -> std::sync::Arc<Vec<ModulePreset>> {
    if let Some(list) = ui.data(|d| d.get_temp::<std::sync::Arc<Vec<ModulePreset>>>(library_id())) {
        return list;
    }
    let list = std::sync::Arc::new(
        library_file()
            .map(|f| presets::load_file(&f))
            .unwrap_or_default(),
    );
    ui.data_mut(|d| d.insert_temp(library_id(), list.clone()));
    list
}

/// Writes the saved patterns back, and keeps them for the session.
fn keep_patterns(ui: &egui::Ui, list: Vec<ModulePreset>) -> Result<(), String> {
    let file = library_file().ok_or("no folder for presets")?;
    presets::save_file(&file, &list)?;
    ui.data_mut(|d| d.insert_temp(library_id(), std::sync::Arc::new(list)));
    Ok(())
}

/// A saved pattern's values (none if the file holds something else).
fn pattern_of(preset: &ModulePreset) -> Option<Pattern> {
    let s: Pattern = preset.data.as_slice().try_into().ok()?;
    let mut s = s;
    s[slot::USED] = 1.0;
    or::set_name(&mut s, &preset.name);
    Some(s)
}

/// Text, unless it is too small to read (zoomed far out).
fn label(
    painter: &Painter,
    at: Pos2,
    align: Align2,
    text: impl ToString,
    font: egui::FontId,
    color: Color32,
) -> Rect {
    if font.size < MIN_TEXT {
        return Rect::from_center_size(at, Vec2::ZERO);
    }
    painter.text(at, align, text.to_string(), font, color)
}

fn dashed_circle(painter: &Painter, c: Pos2, r: f32, dashes: usize, turn: f32, stroke: Stroke) {
    if r < 2.0 {
        return;
    }
    for i in 0..dashes {
        let a0 = turn + i as f32 / dashes as f32 * TAU;
        let a1 = a0 + TAU / dashes as f32 * 0.5;
        let pts: Vec<Pos2> = (0..=3)
            .map(|s| c + Vec2::angled(a0 + (a1 - a0) * s as f32 / 3.0) * r)
            .collect();
        painter.add(Shape::line(pts, stroke));
    }
}

fn dashed_line(painter: &Painter, a: Pos2, b: Pos2, dash: f32, gap: f32, stroke: Stroke) {
    let len = (b - a).length();
    if len < 1.0 {
        return;
    }
    let dir = (b - a) / len;
    let mut t = 0.0;
    while t < len {
        let e = (t + dash).min(len);
        painter.line_segment([a + dir * t, a + dir * e], stroke);
        t += dash + gap;
    }
}
