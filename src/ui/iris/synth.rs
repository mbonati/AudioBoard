//! Iris as a visual synthesizer: up to four layers of scenes, each with its
//! opacity, blend, transform, colour, mirror and echoes (feedback), and the
//! modulators that move them: four LFOs, two envelopes, four audio bands,
//! MIDI notes and controllers, routed by a matrix with bipolar amounts.
//!
//! A layer that is plain (fully opaque, normal blend, no transform, no
//! mirror, no echo) draws straight onto the screen, as Iris always did.
//! Any other is drawn into a layer of its own, tessellated into one mesh,
//! moved and tinted vertex by vertex, then put on the screen: all on the
//! CPU with egui's painter, in the same flat lines and dots.

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;

use egui::epaint::{ClippedShape, Mesh, Tessellator};
use egui::layers::PaintList;
use egui::{Color32, Id, LayerId, Order, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use super::{Analysis, BANDS, Ctx, Pal, Rng, STATES, Scenes, State, draw_scene, heading, scene_name, scene_of};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::iris::data::{self, LAYER_LEN, LAYERS, audio, env, layer, lfo, slot};
use crate::audio::modules::iris::{DETAIL, ENV_TRIGGERS, LFO_SHAPES, MIRRORS, SCENES, SPEED, SYNC_BEATS, SYNC_NAMES, TRAIL, viz};
use crate::ui::theme;

// ---------------------------------------------------------------------------
// Sources and destinations
// ---------------------------------------------------------------------------

/// What the matrix reads. LFOs swing -1..1, everything else 0..1.
pub(super) const SOURCES: [&str; 17] = [
    "—", "LFO 1", "LFO 2", "LFO 3", "LFO 4", "Env 1", "Env 2", "Audio 1", "Audio 2", "Audio 3", "Audio 4", "Beat",
    "Velocity", "Note", "Mod wheel", "CC A", "CC B",
];
const S_LFO: usize = 1;
const S_ENV: usize = 5;
const S_AUDIO: usize = 7;
const S_BEAT: usize = 11;
const S_VELOCITY: usize = 12;
const S_NOTE: usize = 13;
const S_WHEEL: usize = 14;
const S_CC: usize = 15;

const GLOBAL_DESTS: [&str; 3] = ["Speed", "Detail", "Trail"];
const LAYER_DESTS: [&str; 7] = ["Opacity", "Zoom", "Rotate", "X", "Y", "Hue", "Speed"];
/// None, the three global ones, then seven a layer.
pub(super) const DESTS: usize = 1 + GLOBAL_DESTS.len() + LAYERS * LAYER_DESTS.len();

pub(super) fn dest_name(i: usize) -> String {
    match i {
        0 => "—".to_owned(),
        1..=3 => GLOBAL_DESTS[i - 1].to_owned(),
        _ => {
            let j = (i - 4).min(LAYERS * LAYER_DESTS.len() - 1);
            format!("Layer {} {}", j / LAYER_DESTS.len() + 1, LAYER_DESTS[j % LAYER_DESTS.len()])
        }
    }
}

fn get(p: &ModuleParams, i: usize) -> f32 {
    let v = p.data[i].get();
    if v.is_finite() { v } else { 0.0 }
}

fn index(p: &ModuleParams, i: usize, len: usize) -> usize {
    (get(p, i).round().max(0.0) as usize).min(len - 1)
}

// ---------------------------------------------------------------------------
// Modulators
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The modulators' running state, and this frame's sources.
pub(super) struct Mods {
    /// Cycles so far (free LFOs, and synced ones while stopped).
    phase: [f64; data::LFOS],
    hold: [f32; data::LFOS],
    cycle: [i64; data::LFOS],
    env: [(Stage, f32); data::ENVS],
    notes: Option<f32>,
    /// Seconds an onset keeps an envelope's gate open.
    onset_gate: f32,
    rng: Rng,
    pub(super) src: [f32; SOURCES.len()],
}

/// An LFO's value at `ph` (0..1), -1..1.
fn wave(shape: usize, ph: f32, hold: f32) -> f32 {
    match shape {
        0 => (ph * TAU).sin(),
        1 => 1.0 - 4.0 * (ph - 0.5).abs(),
        2 => 2.0 * ph - 1.0,
        3 => {
            if ph < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => hold,
    }
}

impl Mods {
    pub(super) fn new(seed: u32) -> Self {
        Self {
            phase: [0.0; data::LFOS],
            hold: [0.0; data::LFOS],
            cycle: [i64::MIN; data::LFOS],
            env: [(Stage::Idle, 0.0); data::ENVS],
            notes: None,
            onset_gate: 0.0,
            rng: Rng(seed.wrapping_mul(747_796_405) | 1),
            src: [0.0; SOURCES.len()],
        }
    }

    pub(super) fn update(&mut self, p: &ModuleParams, a: &Analysis, dt: f32, tempo: f32) {
        let beat = p.viz[viz::BEAT].get() as f64;
        let playing = p.viz[viz::PLAYING].get() > 0.5;
        for k in 0..data::LFOS {
            let b = data::LFO + k * data::LFO_LEN;
            let cycles = if get(p, b + lfo::SYNC) > 0.5 {
                let period = SYNC_BEATS[index(p, b + lfo::RATE, SYNC_BEATS.len())] as f64;
                if playing {
                    // On the beat, with every synced LFO and the transport.
                    self.phase[k] = beat / period;
                } else {
                    self.phase[k] += dt as f64 * tempo.max(20.0) as f64 / 60.0 / period;
                }
                self.phase[k]
            } else {
                self.phase[k] += dt as f64 * get(p, b + lfo::RATE).clamp(0.01, 20.0) as f64;
                self.phase[k]
            };
            let total = cycles + get(p, b + lfo::PHASE) as f64;
            if total.floor() as i64 != self.cycle[k] {
                self.cycle[k] = total.floor() as i64;
                self.hold[k] = self.rng.unit() * 2.0 - 1.0;
            }
            let shape = index(p, b + lfo::SHAPE, LFO_SHAPES.len());
            self.src[S_LFO + k] = wave(shape, total.rem_euclid(1.0) as f32, self.hold[k]);
        }

        let notes = p.viz[viz::NOTES].get();
        let note_on = self.notes.is_some_and(|n| n != notes);
        self.notes = Some(notes);
        let held = p.viz[viz::HELD].get() > 0.5;
        self.onset_gate = if a.onset { 0.08 } else { (self.onset_gate - dt).max(0.0) };
        for k in 0..data::ENVS {
            let b = data::ENV + k * data::ENV_LEN;
            let trigger = index(p, b + env::TRIGGER, ENV_TRIGGERS.len());
            let (midi, onsets) = (trigger != 1, trigger != 0);
            let fire = (midi && note_on) || (onsets && a.onset);
            let gate = (midi && held) || (onsets && self.onset_gate > 0.0);
            let attack = get(p, b + env::ATTACK).max(0.001);
            let decay = get(p, b + env::DECAY).max(0.001);
            let sustain = get(p, b + env::SUSTAIN).clamp(0.0, 1.0);
            let release = get(p, b + env::RELEASE).max(0.001);
            let (stage, v) = &mut self.env[k];
            if fire {
                *stage = Stage::Attack;
            }
            match *stage {
                Stage::Attack => {
                    *v += dt / attack;
                    if *v >= 1.0 {
                        *v = 1.0;
                        *stage = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    *v -= dt / decay * (1.0 - sustain).max(0.0);
                    if *v <= sustain {
                        *v = sustain;
                        *stage = if gate { Stage::Sustain } else { Stage::Release };
                    }
                }
                Stage::Sustain => {
                    *v = sustain;
                    if !gate {
                        *stage = Stage::Release;
                    }
                }
                Stage::Release => {
                    *v -= dt / release;
                    if *v <= 0.0 {
                        *v = 0.0;
                        *stage = Stage::Idle;
                    }
                }
                Stage::Idle => *v = 0.0,
            }
            self.src[S_ENV + k] = *v;
        }

        for k in 0..data::AUDIO_MODS {
            let b = data::AUDIO + k * data::AUDIO_LEN;
            let (x, y) = (index(p, b + audio::FROM, BANDS), index(p, b + audio::TO, BANDS));
            let (lo, hi) = (x.min(y), x.max(y));
            let mean = a.bands[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32;
            self.src[S_AUDIO + k] = (mean * get(p, b + audio::GAIN).max(0.0)).min(1.0);
        }
        let cc = |n: usize| p.viz[viz::CC + n.min(127)].get().clamp(0.0, 1.0);
        self.src[S_BEAT] = a.beat;
        self.src[S_VELOCITY] = p.viz[viz::VELOCITY].get();
        self.src[S_NOTE] = p.viz[viz::NOTE].get() / 127.0;
        self.src[S_WHEEL] = cc(1);
        for k in 0..data::CCS {
            self.src[S_CC + k] = cc(index(p, data::CC + k, 128));
        }
    }
}

// ---------------------------------------------------------------------------
// The matrix: the settings, moved by the sources
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(super) struct LayerLive {
    pub(super) on: bool,
    pub(super) solo: bool,
    pub(super) scene: usize,
    pub(super) opacity: f32,
    pub(super) add: bool,
    pub(super) zoom: f32,
    /// Turns.
    pub(super) rotate: f32,
    /// Turns a second.
    pub(super) spin: f32,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) hue: f32,
    pub(super) mirror: usize,
    pub(super) echo: usize,
    pub(super) feedback: f32,
    pub(super) speed: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Live {
    pub(super) speed: f32,
    pub(super) detail: f32,
    pub(super) trail: f32,
    pub(super) layers: [LayerLive; LAYERS],
}

/// Every setting the matrix can move, with this frame's sources applied.
pub(super) fn live(p: &ModuleParams, src: &[f32; SOURCES.len()]) -> Live {
    let mut acc = [0.0f32; DESTS];
    for s in 0..data::SLOTS {
        let b = data::MATRIX + s * data::SLOT_LEN;
        let (si, di) = (index(p, b + slot::SOURCE, SOURCES.len()), index(p, b + slot::DEST, DESTS));
        if si > 0 && di > 0 {
            acc[di] += get(p, b + slot::AMOUNT).clamp(-1.0, 1.0) * src[si];
        }
    }
    let layers = std::array::from_fn(|k| {
        let b = data::LAYER + k * LAYER_LEN;
        let m = |j: usize| acc[1 + GLOBAL_DESTS.len() + k * LAYER_DESTS.len() + j];
        LayerLive {
            on: get(p, b + layer::ON) > 0.5,
            solo: get(p, b + layer::SOLO) > 0.5,
            scene: if k == 0 { scene_of(p) } else { index(p, b + layer::SCENE, SCENES.len()) },
            opacity: (get(p, b + layer::OPACITY) + m(0)).clamp(0.0, 1.0),
            add: get(p, b + layer::BLEND) > 0.5,
            zoom: (get(p, b + layer::ZOOM) * (1.5 * m(1)).exp2()).clamp(0.1, 8.0),
            rotate: get(p, b + layer::ROTATE) + 0.5 * m(2),
            spin: get(p, b + layer::SPIN).clamp(-2.0, 2.0),
            x: (get(p, b + layer::X) + m(3)).clamp(-2.0, 2.0),
            y: (get(p, b + layer::Y) + m(4)).clamp(-2.0, 2.0),
            hue: get(p, b + layer::HUE) + 0.5 * m(5),
            mirror: index(p, b + layer::MIRROR, MIRRORS.len()),
            echo: get(p, b + layer::ECHO).round().clamp(0.0, 8.0) as usize,
            feedback: get(p, b + layer::FEEDBACK).clamp(0.8, 1.25),
            speed: (get(p, b + layer::SPEED) * (2.0 * m(6)).exp2()).clamp(0.0, 16.0),
        }
    });
    Live {
        speed: (p.get(SPEED) * (2.0 * acc[1]).exp2()).clamp(0.0, 8.0),
        detail: (p.get(DETAIL) + acc[2]).clamp(0.0, 1.0),
        trail: (p.get(TRAIL) + acc[3]).clamp(0.0, 1.0),
        layers,
    }
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

/// A layer's own scene state, time and echoes.
pub(super) struct Layer {
    pub(super) s: Scenes,
    pub(super) t: f32,
    /// Turned so far by SPIN, in radians.
    spin: f32,
    /// Its past frames, newest first, already moved into place.
    echoes: VecDeque<Arc<Mesh>>,
    echo_clock: f32,
}

impl Layer {
    pub(super) fn new(seed: u32) -> Self {
        Self { s: Scenes::new(seed), t: 0.0, spin: 0.0, echoes: VecDeque::new(), echo_clock: 0.0 }
    }
}

/// How often a layer keeps a frame as an echo.
const ECHO_GAP: f32 = 1.0 / 20.0;
/// At most about this many vertices a layer, echoes and turned copies
/// included: a heavy scene gets fewer echoes rather than a slow frame.
const LAYER_VERTICES: usize = 90_000;

/// Where and how a frame is drawn.
pub(super) struct Frame<'a> {
    pub(super) ctx: &'a egui::Context,
    pub(super) painter: &'a Painter,
    pub(super) screen: Rect,
    pub(super) pal: Pal,
    pub(super) k: f32,
    pub(super) dt: f32,
    pub(super) seed: f32,
    /// Tells this picture's layers apart (the node's, its window's).
    pub(super) key: Id,
    pub(super) visible: bool,
    pub(super) tempo: f32,
}

/// Runs the modulators and every layer for a frame, and draws them when
/// `visible`.
pub(super) fn render(f: &Frame, p: &ModuleParams, st: &mut State) {
    st.mods.update(p, &st.a, f.dt, f.tempo);
    let live = live(p, &st.mods.src);
    let solo = live.layers.iter().any(|l| l.on && l.solo);
    for (k, (lay, l)) in st.layers.iter_mut().zip(live.layers.iter()).enumerate() {
        let speed = live.speed * l.speed;
        lay.t += f.dt * speed * (0.7 + 0.6 * st.a.level);
        lay.spin = (lay.spin + f.dt * l.spin * TAU).rem_euclid(TAU);
        if !l.on || (solo && !l.solo) || !f.visible {
            lay.echoes.clear();
            continue;
        }
        let angle = l.rotate * TAU + lay.spin;
        let turned = angle.rem_euclid(TAU).min(TAU - angle.rem_euclid(TAU)) > 1e-4;
        let plain = l.opacity >= 0.999
            && !l.add
            && (l.zoom - 1.0).abs() < 1e-4
            && !turned
            && l.x.abs() < 1e-4
            && l.y.abs() < 1e-4
            && l.hue.abs() < 1e-4
            && l.mirror == 0
            && l.echo == 0;
        let layer_painter;
        let painter = if plain {
            f.painter
        } else {
            let lid = LayerId::new(Order::Background, f.key.with(("layer", k)));
            layer_painter = Painter::new(f.ctx.clone(), lid, f.screen);
            &layer_painter
        };
        let c = Ctx {
            p: painter,
            r: f.screen,
            pal: f.pal,
            k: f.k,
            t: lay.t,
            dt: f.dt * speed,
            real_dt: f.dt,
            speed,
            detail: live.detail,
            trail: live.trail,
            a: &st.a,
        };
        draw_scene(l.scene, &c, &mut lay.s, f.seed);
        if plain {
            lay.echoes.clear();
        } else {
            composite(f, painter.layer_id(), lay, l, angle);
        }
    }
}

/// Takes what a layer drew off its own layer, moves, echoes, tints and
/// mirrors it, and puts it on the screen.
fn composite(f: &Frame, lid: LayerId, lay: &mut Layer, l: &LayerLive, angle: f32) {
    let shapes: Vec<ClippedShape> = f.ctx.graphics_mut(|g| {
        let list = g.entry(lid);
        let shapes = list.all_entries().cloned().collect();
        *list = PaintList::default();
        shapes
    });
    let options = f.ctx.tessellation_options(|o| *o);
    let font = f.ctx.fonts(|fonts| fonts.font_image_size());
    let mut tess = Tessellator::new(f.ctx.pixels_per_point(), options, font, Vec::new());
    tess.set_clip_rect(Rect::EVERYTHING);
    let mut mesh = Mesh::default();
    for s in shapes {
        tess.tessellate_shape(s.shape, &mut mesh);
    }

    // Zoom and turn round the screen's centre, then move.
    let c0 = f.screen.center();
    let half = f.screen.size() * 0.5;
    let o = c0 + vec2(l.x * half.x, -l.y * half.y);
    let (sn, cs) = angle.sin_cos();
    for v in &mut mesh.vertices {
        let d = (v.pos - c0) * l.zoom;
        v.pos = o + vec2(d.x * cs - d.y * sn, d.x * sn + d.y * cs);
    }

    // Only what can show: the screen, or the part of it a mirror reflects
    // (turned copies need what turns into the screen too).
    let s = f.screen;
    let source = match l.mirror {
        1 => Rect::from_min_max(s.min, pos2(c0.x, s.bottom())),
        2 => Rect::from_min_max(s.min, c0),
        3 | 4 => Rect::from_center_size(c0, Vec2::splat(s.size().length())),
        _ => s,
    };
    let mut scratch = Scratch::default();

    // Echoes: past frames behind it, each grown by FEEDBACK and fading.
    let mesh = Arc::new(mesh);
    // Mirrors reflect only the part they show, so they cost one copy.
    let copies = match l.mirror {
        3 => 3,
        4 => 6,
        _ => 1,
    };
    let n = l.echo.min((LAYER_VERTICES / (mesh.vertices.len() * copies).max(1)).saturating_sub(1));
    let mut out = Mesh::default();
    if n == 0 {
        lay.echoes.clear();
        lay.echo_clock = 0.0;
    } else {
        lay.echo_clock += f.dt;
        if lay.echo_clock >= ECHO_GAP || lay.echoes.is_empty() {
            lay.echo_clock = (lay.echo_clock - ECHO_GAP).clamp(0.0, ECHO_GAP);
            lay.echoes.push_front(mesh.clone());
        }
        lay.echoes.truncate(n);
        let step = lay.echo_clock / ECHO_GAP;
        for (i, m) in lay.echoes.iter().enumerate().rev() {
            let steps = i as f32 + 1.0 + step;
            let grow = l.feedback.powf(steps);
            let fade = (1.0 - steps / (n as f32 + 1.0)).max(0.0) * 0.75;
            append(&mut out, m, |p| o + (p - o) * grow, fade, source, &mut scratch);
        }
    }
    append(&mut out, &mesh, |p| p, 1.0, source, &mut scratch);
    tint(&mut out, l.hue, l.opacity, l.add);

    let clip = s.intersect(f.painter.clip_rect());
    let put = |m: Mesh, r: Rect| {
        let r = r.intersect(clip);
        if r.is_positive() && !m.is_empty() {
            f.painter.with_clip_rect(r).add(Shape::mesh(m));
        }
    };
    let reflect = |m: &Mesh, x: bool, y: bool| {
        let mut m = m.clone();
        for v in &mut m.vertices {
            if x {
                v.pos.x = 2.0 * c0.x - v.pos.x;
            }
            if y {
                v.pos.y = 2.0 * c0.y - v.pos.y;
            }
        }
        m
    };
    match l.mirror {
        // The left half, and its reflection on the right.
        1 => {
            put(reflect(&out, true, false), Rect::from_min_max(pos2(c0.x, s.top()), s.max));
            put(out, source);
        }
        // The top left quarter, reflected into the other three.
        2 => {
            put(reflect(&out, true, false), Rect::from_min_max(pos2(c0.x, s.top()), pos2(s.right(), c0.y)));
            put(reflect(&out, false, true), Rect::from_min_max(pos2(s.left(), c0.y), pos2(c0.x, s.bottom())));
            put(reflect(&out, true, true), Rect::from_min_max(c0, s.max));
            put(out, source);
        }
        // Copies turned round the centre, each only where it shows.
        3 | 4 => {
            let copies = if l.mirror == 3 { 3 } else { 6 };
            for j in 1..copies {
                let (sn, cs) = (j as f32 / copies as f32 * TAU).sin_cos();
                let mut m = Mesh::default();
                let turn = |p: Pos2| {
                    let d = p - c0;
                    c0 + vec2(d.x * cs - d.y * sn, d.x * sn + d.y * cs)
                };
                append(&mut m, &out, turn, 1.0, s, &mut scratch);
                put(m, s);
            }
            let mut m = Mesh::default();
            append(&mut m, &out, |p| p, 1.0, s, &mut scratch);
            put(m, s);
        }
        _ => put(out, s),
    }
}

/// Reused buffers for [`append`].
#[derive(Default)]
struct Scratch {
    map: Vec<u32>,
    pos: Vec<Pos2>,
}

/// Appends the triangles of `m` that touch `area` to `out`, each point
/// moved by `at` and every colour faded.
fn append(out: &mut Mesh, m: &Mesh, at: impl Fn(Pos2) -> Pos2, fade: f32, area: Rect, scratch: &mut Scratch) {
    let Scratch { map, pos } = scratch;
    pos.clear();
    pos.extend(m.vertices.iter().map(|v| at(v.pos)));
    map.clear();
    map.resize(m.vertices.len(), u32::MAX);
    let k = (fade.clamp(0.0, 1.0) * 256.0) as u32;
    out.reserve_triangles(m.indices.len() / 3);
    for tri in m.indices.as_chunks::<3>().0 {
        let [a, b, c] = [pos[tri[0] as usize], pos[tri[1] as usize], pos[tri[2] as usize]];
        if a.x.max(b.x).max(c.x) < area.left()
            || a.x.min(b.x).min(c.x) > area.right()
            || a.y.max(b.y).max(c.y) < area.top()
            || a.y.min(b.y).min(c.y) > area.bottom()
        {
            continue;
        }
        for &i in tri {
            let i = i as usize;
            if map[i] == u32::MAX {
                map[i] = out.vertices.len() as u32;
                let mut v = m.vertices[i];
                v.pos = pos[i];
                if k < 256 {
                    let [r, g, b, a] = v.color.to_array();
                    let f = |x: u8| ((x as u32 * k) >> 8) as u8;
                    v.color = Color32::from_rgba_premultiplied(f(r), f(g), f(b), f(a));
                }
                out.vertices.push(v);
            }
            out.indices.push(map[i]);
        }
    }
}

/// The CSS hue-rotate matrix, for `turns` of the colour wheel.
fn hue_matrix(turns: f32) -> [f32; 9] {
    let (s, c) = (turns * TAU).sin_cos();
    [
        0.213 + 0.787 * c - 0.213 * s,
        0.715 - 0.715 * c - 0.715 * s,
        0.072 - 0.072 * c + 0.928 * s,
        0.213 - 0.213 * c + 0.143 * s,
        0.715 + 0.285 * c + 0.140 * s,
        0.072 - 0.072 * c - 0.283 * s,
        0.213 - 0.213 * c - 0.787 * s,
        0.715 - 0.715 * c + 0.715 * s,
        0.072 + 0.928 * c + 0.072 * s,
    ]
}

/// Turns the colours, sets the opacity, and with `add` makes the light add
/// up (premultiplied colours with no alpha).
fn tint(m: &mut Mesh, hue: f32, opacity: f32, add: bool) {
    let turn = hue.rem_euclid(1.0);
    let matrix = (turn.min(1.0 - turn) > 1e-3).then(|| hue_matrix(turn));
    if matrix.is_none() && opacity >= 0.999 && !add {
        return;
    }
    for v in &mut m.vertices {
        let [r, g, b, a] = v.color.to_array();
        let (mut r, mut g, mut b) = (r as f32, g as f32, b as f32);
        if let Some(m) = &matrix {
            (r, g, b) = (m[0] * r + m[1] * g + m[2] * b, m[3] * r + m[4] * g + m[5] * b, m[6] * r + m[7] * g + m[8] * b);
        }
        let a = a as f32 * opacity;
        // Premultiplied: no channel above the alpha, unless adding.
        let top = if add { 255.0 } else { a };
        let ch = |x: f32| (x * opacity).clamp(0.0, top) as u8;
        v.color = Color32::from_rgba_premultiplied(ch(r), ch(g), ch(b), if add { 0 } else { a as u8 });
    }
}

// ---------------------------------------------------------------------------
// The settings: Layers and Modulation
// ---------------------------------------------------------------------------

fn slider(ui: &mut Ui, p: &ModuleParams, i: usize, range: std::ops::RangeInclusive<f32>, text: impl Fn(f32) -> String + 'static) {
    let mut v = get(p, i);
    let s = egui::Slider::new(&mut v, range).custom_formatter(move |x, _| text(x as f32));
    if ui.add(s).changed() {
        p.data[i].set(v);
    }
}

fn combo(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, p: &ModuleParams, i: usize, names: &[String], width: f32) {
    let cur = index(p, i, names.len());
    egui::ComboBox::from_id_salt(salt).width(width).selected_text(names[cur].as_str()).show_ui(ui, |ui| {
        for (k, name) in names.iter().enumerate() {
            if ui.selectable_label(k == cur, name.as_str()).clicked() {
                p.data[i].set(k as f32);
            }
        }
    });
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn faint(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).small().color(theme::TEXT_FAINT));
}

/// A small bar showing a source now (LFOs from the middle).
fn meter(ui: &mut Ui, v: f32, bipolar: bool, accent: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(34.0, 8.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 2.0, theme::PANEL_BORDER);
    let fill = if bipolar {
        let mid = r.center().x;
        let x = mid + v.clamp(-1.0, 1.0) * r.width() * 0.5;
        Rect::from_min_max(pos2(mid.min(x), r.top()), pos2(mid.max(x), r.bottom()))
    } else {
        Rect::from_min_size(r.min, vec2(r.width() * v.clamp(0.0, 1.0), r.height()))
    };
    p.rect_filled(fill, 2.0, accent);
    p.rect_stroke(r, 2.0, Stroke::new(1.0, theme::PANEL_BORDER), egui::StrokeKind::Inside);
}

/// The sources of the node's picture now, if it is on the board.
fn sources_now(id: NodeId) -> Option<[f32; SOURCES.len()]> {
    STATES.with(|s| s.borrow().get(&id).map(|st| st.mods.src))
}

const LAYER_DEFAULTS: [(usize, f32); 10] = [
    (layer::ZOOM, 1.0),
    (layer::ROTATE, 0.0),
    (layer::SPIN, 0.0),
    (layer::X, 0.0),
    (layer::Y, 0.0),
    (layer::HUE, 0.0),
    (layer::MIRROR, 0.0),
    (layer::ECHO, 0.0),
    (layer::FEEDBACK, 1.06),
    (layer::SPEED, 1.0),
];

/// A layer setting's name, slot, range and how its value reads.
type SliderRow = (&'static str, usize, std::ops::RangeInclusive<f32>, fn(f32) -> String);

pub(super) fn layers_tab(ui: &mut Ui, id: NodeId, p: &ModuleParams) {
    heading(ui, "LAYERS");
    faint(ui, "Drawn from 1 up, 4 on top. Add makes light add up (on dark palettes).");
    let sel_id = Id::unique(("iris-layer", id));
    let mut sel = ui.data(|d| d.get_temp::<usize>(sel_id)).unwrap_or(0).min(LAYERS - 1);
    let scenes: Vec<String> = (0..SCENES.len()).map(scene_name).collect();
    egui::Grid::new(ui.scope_id().with("layers")).num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
        for k in 0..LAYERS {
            let b = data::LAYER + k * LAYER_LEN;
            let mut on = get(p, b + layer::ON) > 0.5;
            if ui.checkbox(&mut on, "").on_hover_text("Drawn").changed() {
                p.data[b + layer::ON].set(on as u8 as f32);
            }
            if ui.selectable_label(sel == k, format!("{}", k + 1)).on_hover_text("Edit this layer").clicked() {
                sel = k;
            }
            if k == 0 {
                // Layer 1's scene is the node's mode.
                let cur = scene_of(p);
                egui::ComboBox::from_id_salt(("iris-layer-scene", id, k)).width(108.0).selected_text(scenes[cur].as_str()).show_ui(ui, |ui| {
                    for (j, name) in scenes.iter().enumerate() {
                        if ui.selectable_label(j == cur, name.as_str()).clicked() {
                            p.set_mode(j);
                        }
                    }
                });
            } else {
                combo(ui, ("iris-layer-scene", id, k), p, b + layer::SCENE, &scenes, 108.0);
            }
            combo(ui, ("iris-layer-blend", id, k), p, b + layer::BLEND, &names(&["Normal", "Add"]), 62.0);
            let mut solo = get(p, b + layer::SOLO) > 0.5;
            if ui.toggle_value(&mut solo, "S").on_hover_text("Solo: only the soloed layers are drawn").changed() {
                p.data[b + layer::SOLO].set(solo as u8 as f32);
            }
            ui.end_row();
        }
    });
    ui.data_mut(|d| d.insert_temp(sel_id, sel));

    let b = data::LAYER + sel * LAYER_LEN;
    ui.horizontal(|ui| {
        heading(ui, &format!("LAYER {}", sel + 1));
        if ui.small_button("Reset").on_hover_text("Its transform, colour, mirror and echoes back to none").clicked() {
            for (j, v) in LAYER_DEFAULTS {
                p.data[b + j].set(v);
            }
        }
    });
    egui::Grid::new(ui.scope_id().with("layer")).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        let rows: [SliderRow; 9] = [
            ("Opacity", layer::OPACITY, 0.0..=1.0, |v| format!("{:.0} %", v * 100.0)),
            ("Zoom", layer::ZOOM, 0.25..=4.0, |v| format!("×{v:.2}")),
            ("Rotate", layer::ROTATE, -0.5..=0.5, |v| format!("{:.0}°", v * 360.0)),
            ("Spin", layer::SPIN, -1.0..=1.0, |v| format!("{:.2} turn/s", v)),
            ("X", layer::X, -1.0..=1.0, |v| format!("{v:+.2}")),
            ("Y", layer::Y, -1.0..=1.0, |v| format!("{v:+.2}")),
            ("Hue", layer::HUE, -0.5..=0.5, |v| format!("{:+.0}°", v * 360.0)),
            ("Speed", layer::SPEED, 0.0..=4.0, |v| format!("×{v:.2}")),
            ("Feedback", layer::FEEDBACK, 0.8..=1.25, |v| format!("×{v:.2}")),
        ];
        for (name, j, range, text) in rows {
            ui.label(name);
            slider(ui, p, b + j, range, text);
            ui.end_row();
        }
        ui.label("Echoes");
        let mut echo = get(p, b + layer::ECHO).round() as i32;
        if ui.add(egui::Slider::new(&mut echo, 0..=8)).on_hover_text("Past frames behind it, each grown by Feedback").changed() {
            p.data[b + layer::ECHO].set(echo as f32);
        }
        ui.end_row();
    });
    ui.horizontal(|ui| {
        ui.label("Mirror");
        let cur = index(p, b + layer::MIRROR, MIRRORS.len());
        for (j, name) in MIRRORS.iter().enumerate() {
            if ui.selectable_label(cur == j, *name).clicked() {
                p.data[b + layer::MIRROR].set(j as f32);
            }
        }
    });
}

/// The bands from `lo` to `hi`, in Hz.
fn band_hz(b: usize) -> f32 {
    30.0 * (16_000.0f32 / 30.0).powf(b as f32 / BANDS as f32)
}

fn hz(v: f32) -> String {
    if v >= 1000.0 { format!("{:.1} kHz", v / 1000.0) } else { format!("{v:.0} Hz") }
}

pub(super) fn mod_tab(ui: &mut Ui, id: NodeId, p: &ModuleParams, accent: Color32) {
    let now = sources_now(id).unwrap_or([0.0; SOURCES.len()]);
    let sources = names(&SOURCES);
    let dests: Vec<String> = (0..DESTS).map(dest_name).collect();

    heading(ui, "MATRIX");
    faint(ui, "Any source to any setting, by a bipolar amount.");
    // The slots in use, and one free one to add another.
    let used: Vec<usize> = (0..data::SLOTS)
        .filter(|&s| {
            let b = data::MATRIX + s * data::SLOT_LEN;
            get(p, b + slot::SOURCE) > 0.5 || get(p, b + slot::DEST) > 0.5
        })
        .collect();
    let free = (0..data::SLOTS).find(|s| !used.contains(s));
    egui::Grid::new(ui.scope_id().with("matrix")).num_columns(4).spacing([5.0, 4.0]).show(ui, |ui| {
        for s in used.iter().copied().chain(free) {
            let b = data::MATRIX + s * data::SLOT_LEN;
            combo(ui, ("iris-src", id, s), p, b + slot::SOURCE, &sources, 78.0);
            combo(ui, ("iris-dst", id, s), p, b + slot::DEST, &dests, 112.0);
            ui.spacing_mut().slider_width = 58.0;
            slider(ui, p, b + slot::AMOUNT, -1.0..=1.0, |v| format!("{:+.0} %", v * 100.0));
            if used.contains(&s) {
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    for j in 0..data::SLOT_LEN {
                        p.data[b + j].set(0.0);
                    }
                }
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    faint(ui, format!("{} of {} slots", used.len(), data::SLOTS));

    heading(ui, "LFOS");
    egui::Grid::new(ui.scope_id().with("lfos")).num_columns(5).spacing([5.0, 4.0]).show(ui, |ui| {
        for k in 0..data::LFOS {
            let b = data::LFO + k * data::LFO_LEN;
            ui.horizontal(|ui| {
                ui.label(format!("{}", k + 1));
                meter(ui, now[S_LFO + k], true, accent);
            });
            combo(ui, ("iris-lfo-shape", id, k), p, b + lfo::SHAPE, &names(&LFO_SHAPES), 70.0);
            let synced = get(p, b + lfo::SYNC) > 0.5;
            if ui.selectable_label(synced, "Sync").on_hover_text("To the transport's beat (free: in Hz)").clicked() {
                p.data[b + lfo::SYNC].set(if synced { 0.0 } else { 1.0 });
                // A sensible rate for the new kind.
                p.data[b + lfo::RATE].set(if synced { 0.5 } else { 4.0 });
            }
            ui.spacing_mut().slider_width = 58.0;
            if synced {
                combo(ui, ("iris-lfo-rate", id, k), p, b + lfo::RATE, &names(&SYNC_NAMES), 64.0);
            } else {
                let mut v = get(p, b + lfo::RATE).clamp(0.02, 10.0);
                let s = egui::Slider::new(&mut v, 0.02..=10.0).logarithmic(true).custom_formatter(|x, _| format!("{x:.2} Hz"));
                if ui.add(s).changed() {
                    p.data[b + lfo::RATE].set(v);
                }
            }
            slider(ui, p, b + lfo::PHASE, 0.0..=1.0, |v| format!("{:.0}°", v * 360.0));
            ui.end_row();
        }
    });

    heading(ui, "ENVELOPES");
    for k in 0..data::ENVS {
        let b = data::ENV + k * data::ENV_LEN;
        ui.horizontal(|ui| {
            ui.label(format!("{}", k + 1));
            meter(ui, now[S_ENV + k], false, accent);
            combo(ui, ("iris-env-trig", id, k), p, b + env::TRIGGER, &names(&ENV_TRIGGERS), 92.0);
        });
        egui::Grid::new(ui.scope_id().with(("env", k))).num_columns(4).spacing([6.0, 3.0]).show(ui, |ui| {
            ui.spacing_mut().slider_width = 62.0;
            let secs = |v: f32| if v < 1.0 { format!("{:.0} ms", v * 1000.0) } else { format!("{v:.1} s") };
            ui.label("A");
            log_slider(ui, p, b + env::ATTACK, 0.001..=4.0, secs);
            ui.label("D");
            log_slider(ui, p, b + env::DECAY, 0.01..=4.0, secs);
            ui.end_row();
            ui.label("S");
            slider(ui, p, b + env::SUSTAIN, 0.0..=1.0, |v| format!("{:.0} %", v * 100.0));
            ui.label("R");
            log_slider(ui, p, b + env::RELEASE, 0.01..=8.0, secs);
            ui.end_row();
        });
    }

    heading(ui, "AUDIO");
    egui::Grid::new(ui.scope_id().with("audio")).num_columns(4).spacing([5.0, 4.0]).show(ui, |ui| {
        for k in 0..data::AUDIO_MODS {
            let b = data::AUDIO + k * data::AUDIO_LEN;
            ui.horizontal(|ui| {
                ui.label(format!("{}", k + 1));
                meter(ui, now[S_AUDIO + k], false, accent);
            });
            let mut lo = index(p, b + audio::FROM, BANDS) as i32;
            let mut hi = index(p, b + audio::TO, BANDS) as i32;
            let r = ui.add(egui::DragValue::new(&mut lo).range(0..=BANDS as i32 - 1).custom_formatter(|v, _| hz(band_hz(v as usize))));
            if r.changed() {
                p.data[b + audio::FROM].set(lo as f32);
            }
            let r = ui.add(egui::DragValue::new(&mut hi).range(0..=BANDS as i32 - 1).custom_formatter(|v, _| hz(band_hz(v as usize + 1))));
            if r.changed() {
                p.data[b + audio::TO].set(hi as f32);
            }
            ui.spacing_mut().slider_width = 60.0;
            slider(ui, p, b + audio::GAIN, 0.0..=4.0, |v| format!("×{v:.1}"));
            ui.end_row();
        }
    });

    heading(ui, "MIDI");
    faint(ui, "Notes and CCs from the MIDI input, or from the keyboards while it is free.");
    ui.horizontal(|ui| {
        for k in 0..data::CCS {
            ui.label(if k == 0 { "CC A" } else { "CC B" });
            let mut n = index(p, data::CC + k, 128) as i32;
            if ui.add(egui::DragValue::new(&mut n).range(0..=127)).changed() {
                p.data[data::CC + k].set(n as f32);
            }
            meter(ui, now[S_CC + k], false, accent);
        }
    });
    ui.horizontal(|ui| {
        faint(ui, "Velocity");
        meter(ui, now[S_VELOCITY], false, accent);
        faint(ui, "Note");
        meter(ui, now[S_NOTE], false, accent);
        faint(ui, "Mod wheel");
        meter(ui, now[S_WHEEL], false, accent);
    });
}

fn log_slider(ui: &mut Ui, p: &ModuleParams, i: usize, range: std::ops::RangeInclusive<f32>, text: impl Fn(f32) -> String + 'static) {
    let mut v = get(p, i).clamp(*range.start(), *range.end());
    let s = egui::Slider::new(&mut v, range).logarithmic(true).custom_formatter(move |x, _| text(x as f32));
    if ui.add(s).changed() {
        p.data[i].set(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::modules::ModuleKind;

    #[test]
    fn a_new_iris_draws_layer_one_as_it_is() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let l = live(&p, &[0.5; SOURCES.len()]);
        assert!(l.layers[0].on && !l.layers[1].on && !l.layers[2].on && !l.layers[3].on);
        let one = l.layers[0];
        assert_eq!((one.opacity, one.add, one.zoom, one.rotate, one.x, one.y, one.hue), (1.0, false, 1.0, 0.0, 0.0, 0.0, 0.0));
        assert_eq!((one.mirror, one.echo, one.speed), (0, 0, 1.0));
        assert_eq!(l.speed, p.get(SPEED));
    }

    #[test]
    fn the_matrix_moves_its_destination_both_ways() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let b = data::MATRIX;
        p.data[b + slot::SOURCE].set(S_LFO as f32);
        // Layer 2's opacity.
        let dest = (0..DESTS).find(|&d| dest_name(d) == "Layer 2 Opacity").unwrap();
        p.data[b + slot::DEST].set(dest as f32);
        p.data[b + slot::AMOUNT].set(-0.5);
        let mut src = [0.0; SOURCES.len()];
        src[S_LFO] = 1.0;
        assert!((live(&p, &src).layers[1].opacity - 0.3).abs() < 1e-5);
        src[S_LFO] = -1.0;
        assert!((live(&p, &src).layers[1].opacity - 1.0).abs() < 1e-5, "clamped");
    }

    #[test]
    fn lfo_waves_swing_both_ways() {
        for shape in 0..4 {
            let vals: Vec<f32> = (0..100).map(|i| wave(shape, i as f32 / 100.0, 0.0)).collect();
            let (lo, hi) = vals.iter().fold((1.0f32, -1.0f32), |(a, b), &v| (a.min(v), b.max(v)));
            assert!(lo < -0.9 && hi > 0.9, "shape {shape}: {lo}..{hi}");
        }
    }

    #[test]
    fn a_note_runs_the_envelope_through_its_stages() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let a = Analysis::new();
        let mut m = Mods::new(1);
        let b = data::ENV + data::ENV_LEN;
        p.data[b + env::TRIGGER].set(0.0);
        p.data[b + env::ATTACK].set(0.1);
        p.data[b + env::DECAY].set(0.1);
        p.data[b + env::SUSTAIN].set(0.5);
        p.data[b + env::RELEASE].set(0.2);
        let step = |m: &mut Mods, n: usize| {
            for _ in 0..n {
                m.update(&p, &a, 0.01, 120.0);
            }
        };
        step(&mut m, 2);
        assert_eq!(m.src[S_ENV + 1], 0.0);
        // Note on, held.
        p.viz[viz::NOTES].set(1.0);
        p.viz[viz::HELD].set(1.0);
        step(&mut m, 9);
        assert!(m.src[S_ENV + 1] > 0.8, "attack: {}", m.src[S_ENV + 1]);
        step(&mut m, 40);
        assert!((m.src[S_ENV + 1] - 0.5).abs() < 1e-3, "sustain: {}", m.src[S_ENV + 1]);
        // Note off: it lets go.
        p.viz[viz::HELD].set(0.0);
        step(&mut m, 40);
        assert_eq!(m.src[S_ENV + 1], 0.0);
    }

    #[test]
    fn synced_lfos_follow_the_transport() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let a = Analysis::new();
        let mut m = Mods::new(1);
        let b = data::LFO;
        p.data[b + lfo::SHAPE].set(2.0);
        p.data[b + lfo::SYNC].set(1.0);
        // One beat.
        p.data[b + lfo::RATE].set(2.0);
        p.viz[viz::PLAYING].set(1.0);
        p.viz[viz::BEAT].set(10.25);
        m.update(&p, &a, 0.01, 120.0);
        assert!((m.src[S_LFO] - (-0.5)).abs() < 1e-4, "a quarter into the saw: {}", m.src[S_LFO]);
    }

    #[test]
    fn hue_and_add_keep_colours_in_range() {
        let mut mesh = Mesh::default();
        for c in [Color32::from_rgb(255, 0, 0), Color32::from_rgba_premultiplied(40, 80, 120, 128), Color32::WHITE] {
            mesh.colored_vertex(Pos2::ZERO, c);
        }
        let mut normal = mesh.clone();
        tint(&mut normal, 0.33, 0.7, false);
        for v in &normal.vertices {
            let [r, g, b, a] = v.color.to_array();
            assert!(r <= a && g <= a && b <= a, "premultiplied: {:?}", v.color);
        }
        tint(&mut mesh, 0.0, 1.0, true);
        assert!(mesh.vertices.iter().all(|v| v.color.a() == 0), "adds up");
        assert_eq!(mesh.vertices[0].color.r(), 255);
    }

    #[test]
    fn hue_matrix_turns_back_to_itself() {
        let m = hue_matrix(0.0);
        for (i, v) in m.iter().enumerate() {
            let want = if i % 4 == 0 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-3, "{i}: {v}");
        }
    }
}
