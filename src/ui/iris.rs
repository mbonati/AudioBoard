//! The Iris panel: a screen filling the node, where audio-reactive scenes
//! are drawn in the board's flat, graphic style (line art, dot matrices,
//! coloured outlines, no glows). Everything else stays out of the way: the
//! scene's name, a gear for the settings popup (next to the node, never
//! over the picture) and a corner grip to resize the node show only while
//! the pointer is over it.
//!
//! The processor hands over the sound it listens to (see
//! `audio::modules::iris`); here it is analysed every frame (spectrum,
//! 32 bands, level, onsets) and each scene draws from that.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::f32::consts::{PI, TAU};

use egui::epaint::Mesh;
use egui::{
    Align2, Color32, CursorIcon, Id, Painter, PopupCloseBehavior, Pos2, Rect, RectAlign, RichText, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use super::nodes::{HEADER_H, NodeView};
use super::theme::{self, alpha, mix};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::iris::{self, DETAIL, REACT, RING, SCENES, SMOOTH, SPEED, TRAIL, data, viz};
use crate::graph::ModuleNode;

/// Margin between the node's edge and the screen (the ports sit in it).
const BEZEL: f32 = 10.0;
const BANDS: usize = 32;
const FFT: usize = 2048;

// ---------------------------------------------------------------------------
// Size and readout
// ---------------------------------------------------------------------------

pub fn size(p: &ModuleParams) -> Vec2 {
    let get = |i: usize, d: f32| {
        let v = p.data[i].get();
        if v.is_finite() { v } else { d }
    };
    vec2(
        get(data::W, iris::DEFAULT_SIZE[0]).clamp(iris::MIN_SIZE[0], iris::MAX_SIZE[0]),
        get(data::H, iris::DEFAULT_SIZE[1]).clamp(iris::MIN_SIZE[1], iris::MAX_SIZE[1]),
    )
}

/// No frame: only the picture, as if stuck on the board.
pub fn frameless(p: &ModuleParams) -> bool {
    p.data[data::FRAMELESS].get() > 0.5
}

/// The screen, in node-local units: inside the frame, or the whole node
/// when it has none.
fn screen_local(size: Vec2, frameless: bool) -> Rect {
    if frameless {
        return Rect::from_min_size(Pos2::ZERO, size);
    }
    Rect::from_min_max(pos2(BEZEL, HEADER_H + 6.0), pos2(size.x - BEZEL, size.y - BEZEL))
}

/// Sets the node's size so the screen has the aspect `w / h`, keeping its width.
fn set_aspect(p: &ModuleParams, w: f32, h: f32) {
    let s = size(p);
    let screen = screen_local(s, frameless(p));
    let height = screen.width() * h / w + (s.y - screen.height());
    p.data[data::H].set(height.clamp(iris::MIN_SIZE[1], iris::MAX_SIZE[1]));
}

fn scene_of(p: &ModuleParams) -> usize {
    p.mode().min(SCENES.len() - 1)
}

/// The scene's name as the panel shows it ("Harmonograph").
pub fn scene_name(i: usize) -> String {
    let s = SCENES[i.min(SCENES.len() - 1)];
    let mut out = s[..1].to_owned();
    out.push_str(&s[1..].to_lowercase());
    if out == "Moire" { "Moiré".to_owned() } else { out }
}

pub fn readout(p: &ModuleParams) -> (String, String) {
    // What the processor listens to now (Auto follows the cable).
    let source = if p.viz[viz::FROM_INPUT].get() > 0.5 { "IN" } else { "main mix" };
    (scene_name(scene_of(p)), format!("{} · {}", source, PALETTES[palette_index(p)].name))
}

// ---------------------------------------------------------------------------
// Palettes
// ---------------------------------------------------------------------------

pub struct Palette {
    pub name: &'static str,
    bg: Color32,
    /// The main line colour, then four accents.
    inks: [Color32; 5],
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Saved by index: new ones go at the end.
pub const PALETTES: [Palette; 8] = [
    Palette {
        name: "Board",
        bg: rgb(17, 18, 21),
        inks: [rgb(226, 228, 232), rgb(96, 165, 238), rgb(226, 112, 150), rgb(132, 194, 108), rgb(228, 142, 88)],
    },
    // The node's own colour (filled in when drawing).
    Palette { name: "Mono", bg: rgb(15, 16, 19), inks: [rgb(226, 228, 232); 5] },
    Palette {
        name: "Paper",
        bg: rgb(236, 231, 220),
        inks: [rgb(34, 34, 38), rgb(214, 72, 56), rgb(38, 92, 170), rgb(222, 160, 36), rgb(62, 132, 104)],
    },
    Palette {
        name: "Riso",
        bg: rgb(244, 239, 228),
        inks: [rgb(46, 44, 60), rgb(255, 72, 150), rgb(0, 120, 190), rgb(250, 190, 0), rgb(0, 150, 112)],
    },
    Palette {
        name: "Ember",
        bg: rgb(22, 13, 11),
        inks: [rgb(255, 222, 170), rgb(255, 150, 60), rgb(232, 72, 52), rgb(190, 44, 80), rgb(255, 204, 92)],
    },
    Palette {
        name: "Ice",
        bg: rgb(9, 17, 27),
        inks: [rgb(222, 240, 255), rgb(110, 200, 255), rgb(64, 132, 232), rgb(120, 230, 210), rgb(170, 150, 255)],
    },
    Palette {
        name: "Acid",
        bg: rgb(8, 8, 10),
        inks: [rgb(240, 240, 240), rgb(200, 255, 60), rgb(255, 60, 200), rgb(40, 230, 255), rgb(255, 236, 70)],
    },
    // On white, blue through cyan to green and back (the Halo's own look).
    Palette {
        name: "Lagoon",
        bg: rgb(244, 246, 247),
        inks: [rgb(30, 46, 62), rgb(28, 108, 240), rgb(0, 190, 226), rgb(44, 196, 110), rgb(0, 166, 178)],
    },
];

fn palette_index(p: &ModuleParams) -> usize {
    (p.data[data::PALETTE].get().round().max(0.0) as usize).min(PALETTES.len() - 1)
}

/// A palette ready to draw with.
#[derive(Clone, Copy)]
struct Pal {
    bg: Color32,
    inks: [Color32; 5],
}

impl Pal {
    fn of(i: usize, accent: Color32) -> Self {
        let p = &PALETTES[i];
        if p.name == "Mono" {
            let white = Color32::WHITE;
            return Self {
                bg: p.bg,
                inks: [mix(accent, white, 0.55), accent, mix(accent, white, 0.35), mix(accent, p.bg, 0.35), mix(accent, white, 0.7)],
            };
        }
        Self { bg: p.bg, inks: p.inks }
    }

    fn line(&self) -> Color32 {
        self.inks[0]
    }

    /// Accent `i` (cycling through the four).
    fn ink(&self, i: usize) -> Color32 {
        self.inks[1 + i % 4]
    }

    /// Through the four accents, `t` in 0..1 (cycling).
    fn cyc(&self, t: f32) -> Color32 {
        let x = t.rem_euclid(1.0) * 4.0;
        let i = x.floor() as usize;
        mix(self.ink(i), self.ink(i + 1), x - i as f32)
    }

    /// From the first accent to the last, `t` in 0..1.
    fn grad(&self, t: f32) -> Color32 {
        let x = t.clamp(0.0, 1.0) * 3.0;
        let i = (x.floor() as usize).min(2);
        mix(self.ink(i), self.ink(i + 1), x - i as f32)
    }
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

struct Analysis {
    /// The latest samples, oldest first.
    wave: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,
    /// This frame's bands, 0..1, before REACT and smoothing.
    raw: [f32; BANDS],
    prev: [f32; BANDS],
    /// Smoothed, scaled by REACT.
    bands: [f32; BANDS],
    level: f32,
    bass: f32,
    mid: f32,
    high: f32,
    flux_mean: f32,
    flux_dev: f32,
    /// 1 at an onset, falling.
    beat: f32,
    /// An onset this frame.
    onset: bool,
    since: f32,
    written: Option<u32>,
    stale: f32,
    live: bool,
    sr: f32,
    /// The strongest frequency between 40 Hz and 2 kHz.
    pitch: f32,
    /// Makes the waveform fill the screen.
    gain: f32,
}

impl Analysis {
    fn new() -> Self {
        let window = (0..FFT).map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT as f32).cos()).collect();
        Self {
            wave: vec![0.0; RING],
            re: vec![0.0; FFT],
            im: vec![0.0; FFT],
            window,
            raw: [0.0; BANDS],
            prev: [0.0; BANDS],
            bands: [0.0; BANDS],
            level: 0.0,
            bass: 0.0,
            mid: 0.0,
            high: 0.0,
            flux_mean: 0.0,
            flux_dev: 0.0,
            beat: 0.0,
            onset: false,
            since: 1.0,
            written: None,
            stale: 0.0,
            live: false,
            sr: 48_000.0,
            pitch: 220.0,
            gain: 1.0,
        }
    }

    fn update(&mut self, p: &ModuleParams, dt: f32) {
        let react = p.get(REACT);
        let smooth = p.get(SMOOTH);
        self.sr = p.viz[viz::SR].get().max(8000.0);
        self.live = p.viz[viz::LIVE].get() > 0.5;
        let w = p.viz[viz::WRITTEN].get() as u32;
        if self.written == Some(w) {
            self.stale += dt;
        } else {
            self.stale = 0.0;
        }
        self.written = Some(w);
        if self.live && self.stale < 0.25 {
            // The slot at `w` is the oldest one.
            for (i, s) in self.wave.iter_mut().enumerate() {
                *s = p.viz[viz::RING + (w as usize + i) % RING].get();
            }
        } else {
            // No sound coming (stopped, or IN unpatched): let it fade.
            for s in &mut self.wave {
                *s *= 0.85;
            }
        }

        // Spectrum.
        let start = RING - FFT;
        let mut wsum = 0.0;
        for i in 0..FFT {
            self.re[i] = self.wave[start + i] * self.window[i];
            self.im[i] = 0.0;
            wsum += self.window[i];
        }
        super::monitor::fft(&mut self.re, &mut self.im);
        let bin_hz = self.sr / FFT as f32;
        let amp = |re: &[f32], im: &[f32], k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt() * 2.0 / wsum;
        let (lo, hi) = (30.0f32, 16_000.0f32);
        for b in 0..BANDS {
            let f0 = lo * (hi / lo).powf(b as f32 / BANDS as f32);
            let f1 = lo * (hi / lo).powf((b + 1) as f32 / BANDS as f32);
            // Each bin belongs to one band; a band narrower than a bin
            // (low down) reads the bin nearest its centre.
            let fc = (f0 * f1).sqrt();
            let k0 = (f0 / bin_hz).ceil() as usize;
            let k1 = ((f1 / bin_hz).ceil() as usize).min(FFT / 2);
            let bins = if k1 > k0 { k0..k1 } else { ((fc / bin_hz).round() as usize).max(1)..((fc / bin_hz).round() as usize).max(1) + 1 };
            let mut m = 0.0f32;
            for k in bins {
                m = m.max(amp(&self.re, &self.im, k.max(1)));
            }
            // A gentle tilt (+3 dB an octave around 1 kHz), as music has
            // less energy up high.
            let db = 20.0 * m.max(1e-6).log10() + 3.0 * (fc / 1000.0).log2();
            self.raw[b] = ((db + 66.0) / 54.0).clamp(0.0, 1.0);
        }
        let (mut best, mut best_k) = (0.0f32, 0usize);
        for k in ((40.0 / bin_hz) as usize).max(1)..((2000.0 / bin_hz) as usize).min(FFT / 2) {
            let a = amp(&self.re, &self.im, k);
            if a > best {
                best = a;
                best_k = k;
            }
        }
        if best > 1e-4 {
            let target = best_k as f32 * bin_hz;
            self.pitch += (target - self.pitch) * (dt * 6.0).min(1.0);
        }

        // Smoothing: a fast attack, a release SMOOTH sets.
        let attack = 1.0 - (-dt / 0.012).exp();
        let release = 1.0 - (-dt / (0.04 + smooth * 0.7)).exp();
        let follow = |v: &mut f32, target: f32| {
            let k = if target > *v { attack } else { release };
            *v += (target - *v) * k;
        };
        for b in 0..BANDS {
            follow(&mut self.bands[b], (self.raw[b] * react).min(1.0));
        }
        let tail = &self.wave[RING - 1024..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        let peak = tail.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let level = ((20.0 * rms.max(1e-6).log10() + 54.0) / 48.0).clamp(0.0, 1.0);
        follow(&mut self.level, (level * react).min(1.0));
        let mean = |r: std::ops::Range<usize>| {
            let n = r.len() as f32;
            self.bands[r].iter().sum::<f32>() / n
        };
        self.bass = mean(0..8);
        self.mid = mean(8..20);
        self.high = mean(20..BANDS);
        let target = 0.9 / peak.max(0.03);
        self.gain += (target - self.gain) * (dt * 3.0).min(1.0);

        // Onsets: spectral flux over its own running average.
        let mut flux = 0.0;
        for b in 0..BANDS {
            let weight = if b < 8 { 1.6 } else { 1.0 };
            flux += (self.raw[b] - self.prev[b]).max(0.0) * weight;
        }
        flux /= BANDS as f32;
        self.prev = self.raw;
        let k = (dt / 0.8).min(1.0);
        let over = flux > self.flux_mean + 1.5 * self.flux_dev + 0.012;
        self.flux_mean += (flux - self.flux_mean) * k;
        self.flux_dev += ((flux - self.flux_mean).abs() - self.flux_dev) * k;
        self.onset = self.live && react > 0.0 && over && self.since > 0.14 && level > 0.08;
        if self.onset {
            self.beat = 1.0;
            self.since = 0.0;
        } else {
            self.since += dt;
        }
        self.beat *= (-dt * 5.0).exp();
    }
}

// ---------------------------------------------------------------------------
// Per-node state
// ---------------------------------------------------------------------------

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
}

struct Particle {
    p: Vec2,
    v: Vec2,
    trail: VecDeque<Vec2>,
    age: f32,
    life: f32,
}

#[derive(Default)]
struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<u16>,
    ghost: Vec<f32>,
}

/// What the scenes remember from frame to frame.
struct Scenes {
    rng: Rng,
    clock: f32,
    ridges: VecDeque<Vec<f32>>,
    ridge_clock: f32,
    harmo: [f32; 2],
    harmo_target: [f32; 2],
    tunnel_phase: f32,
    tunnel_sides: usize,
    flow: Vec<Particle>,
    life: Grid,
    life_clock: f32,
    ca: VecDeque<(Vec<bool>, usize)>,
    ca_cols: usize,
    ca_rule: usize,
    ca_ink: usize,
    ca_clock: f32,
    truchet: Grid,
    truchet_anim: Vec<f32>,
    truchet_flip: Vec<f32>,
    attractor: usize,
    attractor_mix: f32,
    tau: f32,
    echoes: VecDeque<Vec<Vec2>>,
    peaks: Vec<f32>,
    orbits: Vec<f32>,
    spiro_flip: f32,
    /// Fixed per seed: varies the Halo's rays.
    salt: u32,
    halo_rot: f32,
    halo_spin: f32,
    halo_hue: f32,
    halo_hue_to: f32,
    /// Ripples running out, 0 (inside) to 1 (outside).
    halo_waves: Vec<f32>,
    globe_shape: usize,
    globe_mix: f32,
    globe_yaw: f32,
    globe_spin: f32,
    /// Newest first: a row of heights, and its onset mark.
    terrain: VecDeque<(Vec<f32>, f32)>,
    terrain_clock: f32,
    terrain_n: f32,
    flight_z: f32,
    flight_v: f32,
    flight_roll: f32,
}

impl Scenes {
    fn new(seed: u32) -> Self {
        Self {
            rng: Rng(seed.wrapping_mul(2_654_435_761).max(1)),
            clock: 0.0,
            ridges: VecDeque::new(),
            ridge_clock: 0.0,
            harmo: [2.0, 3.0],
            harmo_target: [2.0, 3.0],
            tunnel_phase: 0.0,
            tunnel_sides: 6,
            flow: Vec::new(),
            life: Grid::default(),
            life_clock: 0.0,
            ca: VecDeque::new(),
            ca_cols: 0,
            ca_rule: 0,
            ca_ink: 0,
            ca_clock: 0.0,
            truchet: Grid::default(),
            truchet_anim: Vec::new(),
            truchet_flip: Vec::new(),
            attractor: 0,
            attractor_mix: 0.0,
            tau: 40.0,
            echoes: VecDeque::new(),
            peaks: Vec::new(),
            orbits: Vec::new(),
            spiro_flip: 0.0,
            salt: seed.wrapping_mul(0x85EB_CA77) | 1,
            halo_rot: 0.0,
            halo_spin: 0.0,
            halo_hue: 0.0,
            halo_hue_to: 0.0,
            halo_waves: Vec::new(),
            globe_shape: 0,
            globe_mix: 0.0,
            globe_yaw: 0.0,
            globe_spin: 0.0,
            terrain: VecDeque::new(),
            terrain_clock: 0.0,
            terrain_n: 0.0,
            flight_z: 0.0,
            flight_v: 1.0,
            flight_roll: 0.0,
        }
    }
}

struct State {
    a: Analysis,
    s: Scenes,
    /// Scene time: runs at SPEED, a little faster with loud sound.
    t: f32,
    last: f64,
    seen: f64,
    /// Seconds left showing the scene's name.
    hud: f32,
    scene: usize,
    seed: f32,
}

impl State {
    fn new(seed: f32) -> Self {
        Self {
            a: Analysis::new(),
            s: Scenes::new(seed.to_bits() ^ 0x9E37_79B9),
            t: 0.0,
            last: f64::NAN,
            seen: 0.0,
            hud: 0.0,
            scene: usize::MAX,
            seed,
        }
    }
}

thread_local! {
    static STATES: RefCell<HashMap<NodeId, State>> = RefCell::new(HashMap::new());
}

/// Tests and snapshots: draw as if the pointer were over the node, with the
/// settings open.
#[cfg(test)]
pub static SHOW_CONTROLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
fn show_controls() -> bool {
    SHOW_CONTROLS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(test))]
fn show_controls() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    p: &'a Painter,
    r: Rect,
    pal: Pal,
    /// Screen points per board unit, times a gentle growth with the
    /// screen's size, so lines keep their weight on a large Iris.
    k: f32,
    t: f32,
    /// This frame, scaled by SPEED.
    dt: f32,
    /// This frame, in real seconds.
    real_dt: f32,
    speed: f32,
    detail: f32,
    trail: f32,
    a: &'a Analysis,
}

impl Ctx<'_> {
    fn c(&self) -> Pos2 {
        self.r.center()
    }

    fn rad(&self) -> f32 {
        self.r.width().min(self.r.height()) * 0.5
    }

    fn w(&self, px: f32) -> f32 {
        (px * self.k).max(0.6)
    }

    /// The bands at `u` in 0..1 (low to high), interpolated.
    fn band(&self, u: f32) -> f32 {
        let x = u.clamp(0.0, 1.0) * (BANDS - 1) as f32;
        let i = (x.floor() as usize).min(BANDS - 2);
        let f = x - i as f32;
        self.a.bands[i] * (1.0 - f) + self.a.bands[i + 1] * f
    }

    fn line(&self, pts: Vec<Pos2>, w: f32, c: Color32) {
        if pts.len() > 1 {
            self.p.add(Shape::line(pts, Stroke::new(w, c)));
        }
    }

    fn closed(&self, pts: Vec<Pos2>, w: f32, c: Color32) {
        if pts.len() > 2 {
            self.p.add(Shape::closed_line(pts, Stroke::new(w, c)));
        }
    }

    /// A polyline in `chunks` pieces, coloured along its length by `col`
    /// (0 at the start, 1 at the end).
    fn gradient_line(&self, pts: &[Pos2], chunks: usize, w: f32, col: impl Fn(f32) -> Color32) {
        if pts.len() < 2 {
            return;
        }
        let chunks = chunks.clamp(1, pts.len() - 1);
        let per = (pts.len() - 1).div_ceil(chunks);
        let mut i = 0;
        while i < pts.len() - 1 {
            let end = (i + per).min(pts.len() - 1);
            self.line(pts[i..=end].to_vec(), w, col(i as f32 / (pts.len() - 1) as f32));
            i = end;
        }
    }

    /// A dashed circle.
    fn dashed_circle(&self, c: Pos2, r: f32, dashes: usize, fill: f32, rot: f32, w: f32, col: Color32) {
        let dashes = dashes.max(3);
        for d in 0..dashes {
            let a0 = rot + d as f32 / dashes as f32 * TAU;
            let a1 = a0 + fill / dashes as f32 * TAU;
            let pts: Vec<Pos2> = (0..=6).map(|k| {
                let a = a0 + (a1 - a0) * k as f32 / 6.0;
                c + vec2(a.cos(), a.sin()) * r
            }).collect();
            self.line(pts, w, col);
        }
    }
}

/// A segment as a quad (no anti-aliasing: for thousands of short strokes).
fn seg(mesh: &mut Mesh, a: Pos2, b: Pos2, w: f32, col: Color32) {
    let d = b - a;
    let len = d.length();
    if len < 1e-3 {
        return;
    }
    let n = vec2(-d.y, d.x) / len * (w * 0.5);
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(a + n, col);
    mesh.colored_vertex(b + n, col);
    mesh.colored_vertex(b - n, col);
    mesh.colored_vertex(a - n, col);
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

/// Pulsar lines, one behind the other: the spectrum scrolls up the screen.
fn ridges(c: &Ctx, s: &mut Scenes) {
    let rows = (14.0 + c.detail * 42.0) as usize;
    let pts = 84;
    s.ridge_clock += c.dt * 22.0;
    while s.ridge_clock >= 1.0 {
        s.ridge_clock -= 1.0;
        let mut row: Vec<f32> = (0..pts)
            .map(|j| {
                let u = j as f32 / (pts - 1) as f32;
                let d = ((u - 0.5).abs() * 2.0).min(1.0);
                let win = (-(d * 2.3).powi(2)).exp();
                let e = c.band(d * 0.85);
                let jitter = s.rng.unit() * (0.06 + 0.3 * c.a.level);
                (e * 0.9 + jitter + 0.03) * win
            })
            .collect();
        for _ in 0..2 {
            let prev = row.clone();
            for j in 1..pts - 1 {
                row[j] = (prev[j - 1] + 2.0 * prev[j] + prev[j + 1]) * 0.25;
            }
        }
        s.ridges.push_back(row);
        while s.ridges.len() > 60 {
            s.ridges.pop_front();
        }
    }
    let n = rows.min(s.ridges.len());
    if n == 0 {
        return;
    }
    let r = c.r.shrink2(vec2(c.r.width() * 0.1, c.r.height() * 0.07));
    let tall = 3.0 + 5.0 * c.trail;
    let gap = r.height() / ((rows - 1) as f32 + 0.85 * tall);
    let amp = gap * tall;
    let start = s.ridges.len() - n;
    for (k, row) in s.ridges.iter().skip(start).enumerate() {
        let base = r.bottom() - gap * (n - 1 - k) as f32;
        let pts: Vec<Pos2> = row
            .iter()
            .enumerate()
            .map(|(j, v)| pos2(r.left() + r.width() * j as f32 / (pts - 1) as f32, base - v * amp))
            .collect();
        // Hide what is behind.
        let mut mesh = Mesh::default();
        for j in 0..pts.len() - 1 {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(pts[j], c.pal.bg);
            mesh.colored_vertex(pts[j + 1], c.pal.bg);
            mesh.colored_vertex(pos2(pts[j + 1].x, base + 1.0), c.pal.bg);
            mesh.colored_vertex(pos2(pts[j].x, base + 1.0), c.pal.bg);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        c.p.add(Shape::mesh(mesh));
        let depth = k as f32 / n.max(2) as f32;
        let mut col = alpha(c.pal.line(), 0.3 + 0.7 * depth);
        if k + 1 == n {
            col = mix(c.pal.line(), c.pal.ink(0), c.a.beat);
        }
        c.line(pts, c.w(1.1), col);
    }
}

/// Rings of petals with N-fold symmetry, each ring a band.
fn mandala(c: &Ctx, _s: &mut Scenes) {
    let sym = 5 + (c.detail * 11.0).round() as usize;
    let rings = 5 + (c.detail * 9.0).round() as usize;
    let rad = c.rad() * 0.95;
    let o = c.c();
    let pts = (sym * 26).clamp(160, 420);
    for k in 0..sym {
        let a = k as f32 / sym as f32 * TAU + c.t * 0.04;
        let d = vec2(a.cos(), a.sin());
        c.line(vec![o + d * rad * 0.08, o + d * rad * 0.98], c.w(0.8), alpha(c.pal.line(), 0.14));
    }
    for i in 0..rings {
        let u = (i as f32 + 0.5) / rings as f32;
        let e = c.band(u);
        let base = rad * (0.1 + 0.76 * u);
        let amp = rad * (0.025 + 0.15 * e) * (0.5 + 0.6 * u);
        let dir = if i % 2 == 0 { 1.0 } else { -1.0 };
        let rot = c.t * (0.1 + 0.22 * u) * dir + i as f32 * 0.37;
        let shape = i % 3;
        let points: Vec<Pos2> = (0..pts)
            .map(|j| {
                let th = j as f32 / pts as f32 * TAU;
                let x = sym as f32 * (th - rot);
                let f = match shape {
                    0 => x.cos(),
                    1 => 1.0 - 2.0 * (x * 0.5).sin().abs(),
                    _ => x.cos().signum() * x.cos().abs().powf(0.35),
                };
                o + vec2(th.cos(), th.sin()) * (base + amp * f)
            })
            .collect();
        c.closed(points, c.w(0.9 + 2.2 * e), c.pal.cyc(u * 0.9 + c.t * 0.01));
    }
    for k in 0..sym {
        let a = k as f32 / sym as f32 * TAU + c.t * 0.04;
        let tip = o + vec2(a.cos(), a.sin()) * rad * 0.97;
        let r = c.w(1.5 + 6.0 * c.a.high);
        c.p.circle_filled(tip, r, c.pal.bg);
        c.p.circle_stroke(tip, r, Stroke::new(c.w(1.0), c.pal.ink(k)));
    }
    if c.a.beat > 0.03 {
        let r = rad * (0.12 + (1.0 - c.a.beat) * 0.86);
        c.dashed_circle(o, r, sym * 3, 0.5, c.t * 0.2, c.w(1.0 + 2.0 * c.a.beat), alpha(c.pal.line(), c.a.beat));
    }
    let core: Vec<Pos2> = (0..sym)
        .map(|k| {
            let a = k as f32 / sym as f32 * TAU - c.t * 0.2;
            o + vec2(a.cos(), a.sin()) * rad * (0.04 + 0.05 * c.a.bass)
        })
        .collect();
    c.closed(core, c.w(1.4), c.pal.line());
}

const RATIOS: [[f32; 2]; 8] = [[2.0, 3.0], [3.0, 4.0], [3.0, 5.0], [4.0, 5.0], [1.0, 2.0], [2.0, 5.0], [5.0, 6.0], [3.0, 7.0]];

/// A pendulum drawing: two damped swings per axis in small ratios, which
/// jump to new ratios on onsets.
fn harmonograph(c: &Ctx, s: &mut Scenes) {
    if c.a.onset && s.rng.unit() < 0.5 {
        s.harmo_target = RATIOS[s.rng.below(RATIOS.len())];
    }
    for i in 0..2 {
        s.harmo[i] += (s.harmo_target[i] - s.harmo[i]) * (c.real_dt * 1.2).min(1.0);
    }
    let [fa, fb] = s.harmo;
    let span = 30.0 + 120.0 * c.trail;
    let n = (900.0 + 2400.0 * c.detail) as usize;
    let damp = 2.4 / span;
    let amp = c.rad() * 0.92 * (0.6 + 0.4 * c.a.level);
    let t = c.t;
    let (p1, p2, p3, p4) = (t * 0.21, t * 0.13 + 1.0, t * 0.17 + 2.0, t * 0.11);
    let detune = 0.004 * (t * 0.3).sin() + 0.012 * c.a.mid;
    let o = c.c();
    let pts: Vec<Pos2> = (0..n)
        .map(|i| {
            let tau = i as f32 / n as f32 * span;
            let e = (-damp * tau).exp();
            let x = (fa * tau + p1).sin() * 0.62 + ((fb + detune) * tau + p2).sin() * 0.38;
            let y = (fb * tau + p3).sin() * 0.62 + ((fa - detune) * tau + p4).sin() * 0.38;
            o + vec2(x, y) * e * amp
        })
        .collect();
    let w = c.w(1.0 + 0.8 * c.a.beat);
    c.gradient_line(&pts, 24, w, |u| alpha(c.pal.grad(u), 1.0 - 0.55 * u));
}

/// Nested spirographs (hypotrochoids), each turning its own way.
fn spiro(c: &Ctx, s: &mut Scenes) {
    if c.a.onset {
        s.spiro_flip = 1.0;
    }
    s.spiro_flip *= (-c.real_dt * 4.0).exp();
    let layers = 2 + (c.detail * 2.0).round() as usize;
    let o = c.c();
    for l in 0..layers {
        let m = 5 + 2 * l + (c.detail * 5.0) as usize;
        let n = 1 + l % 2;
        let r = n as f32 / m as f32;
        let e = c.band(0.1 + 0.75 * l as f32 / layers as f32);
        let d = r * (0.55 + 1.5 * e + 0.25 * (c.t * 0.4 + l as f32).sin());
        let k = (1.0 - r) / r;
        let size = c.rad() * 0.94 * (1.0 - 0.2 * l as f32);
        let scale = size / ((1.0 - r) + d.abs()).max(0.05);
        let rot = c.t * 0.13 * if l % 2 == 0 { 1.0 } else { -1.0 } + s.spiro_flip * 0.3;
        let count = 90 * m;
        let pts: Vec<Pos2> = (0..count)
            .map(|i| {
                let th = i as f32 / count as f32 * TAU * n as f32;
                let x = (1.0 - r) * th.cos() + d * (k * th).cos();
                let y = (1.0 - r) * th.sin() - d * (k * th).sin();
                let (sn, cs) = rot.sin_cos();
                o + vec2(x * cs - y * sn, x * sn + y * cs) * scale
            })
            .collect();
        let w = c.w(0.9 + 1.4 * e + if l == 0 { 1.5 * s.spiro_flip } else { 0.0 });
        c.closed(pts, w, c.pal.ink(l));
    }
}

/// Polygons flying out of a vanishing point, twisting with the mids.
fn tunnel(c: &Ctx, s: &mut Scenes) {
    if c.a.onset && s.rng.unit() < 0.4 {
        s.tunnel_sides = 3 + s.rng.below(6);
    }
    s.tunnel_phase += c.dt * (0.35 + 1.6 * c.a.bass);
    let n = 10 + (c.detail * 22.0) as usize;
    let sides = s.tunnel_sides;
    let rad = c.r.size().length() * 0.55;
    let twist = 0.25 + 1.6 * c.a.mid;
    let fl = s.tunnel_phase.floor();
    let fr = s.tunnel_phase - fl;
    for i in 0..n {
        let depth = (i as f32 + fr) / n as f32;
        let id = i as i64 - fl as i64;
        let scale = depth.powf(2.2) * rad;
        if scale < 1.0 {
            continue;
        }
        let vp = c.c() + vec2((c.t * 0.3).sin(), (c.t * 0.23).cos()) * c.rad() * 0.18 * (1.0 - depth);
        let rot = depth * twist * PI + c.t * 0.08;
        let pts: Vec<Pos2> = (0..sides)
            .map(|k| {
                let a = rot + k as f32 / sides as f32 * TAU;
                vp + vec2(a.cos(), a.sin()) * scale
            })
            .collect();
        let flash = if id.rem_euclid(4) == 0 { c.a.beat } else { 0.0 };
        let col = alpha(c.pal.ink(id.rem_euclid(4) as usize), smoothstep(depth * 3.0));
        c.closed(pts, c.w(0.6 + 3.0 * depth + 3.0 * flash), col);
    }
}

/// Particles drifting on a flow field; onsets burst them outwards.
fn flow(c: &Ctx, s: &mut Scenes, seed: f32) {
    let count = 120 + (c.detail * 780.0) as usize;
    let len = 3 + (c.trail * 18.0) as usize;
    let fresh = |rng: &mut Rng| Particle {
        p: vec2(rng.unit(), rng.unit()),
        v: Vec2::ZERO,
        trail: VecDeque::new(),
        age: 0.0,
        life: 2.0 + 6.0 * rng.unit(),
    };
    while s.flow.len() < count {
        let p = fresh(&mut s.rng);
        s.flow.push(p);
    }
    s.flow.truncate(count);
    let aspect = c.r.height() / c.r.width().max(1.0);
    let speed = 0.05 * (0.25 + 1.6 * c.a.level);
    let t = c.t;
    let mut mesh = Mesh::default();
    for (i, q) in s.flow.iter_mut().enumerate() {
        let (x, y) = (q.p.x, q.p.y * aspect);
        let a = ((x * 3.3 + t * 0.29 + seed).sin() * (y * 2.9 - t * 0.23).cos() + 0.5 * ((x - y) * 4.7 + t * 0.41).sin()) * PI * 1.4;
        if c.a.onset {
            let out = q.p - vec2(0.5, 0.5);
            q.v += out.normalized() * 0.35 * (0.3 + c.a.bass);
        }
        let dir = vec2(a.cos(), a.sin() / aspect.max(0.2));
        q.p += (dir * speed * c.speed + q.v) * c.real_dt;
        q.v *= (-c.real_dt * 2.5).exp();
        q.age += c.real_dt;
        q.trail.push_back(q.p);
        while q.trail.len() > len {
            q.trail.pop_front();
        }
        let out = q.p.x < -0.02 || q.p.x > 1.02 || q.p.y < -0.02 || q.p.y > 1.02;
        if out || q.age > q.life {
            *q = fresh(&mut s.rng);
            continue;
        }
        let to = |v: Vec2| c.r.min + vec2(v.x * c.r.width(), v.y * c.r.height());
        let e = 0.35 + 0.65 * c.band((i % 24) as f32 / 23.0);
        let col = c.pal.ink(i);
        let w = c.w(1.1);
        let m = q.trail.len();
        for k in 1..m {
            let f = k as f32 / m as f32;
            seg(&mut mesh, to(q.trail[k - 1]), to(q.trail[k]), w, alpha(col, f * e));
        }
    }
    c.p.add(Shape::mesh(mesh));
}

const PATTERNS: [&[(i32, i32)]; 4] = [
    &[(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)],
    &[(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)],
    &[(1, 0), (3, 1), (0, 2), (1, 2), (4, 2), (5, 2), (6, 2)],
    &[(1, 0), (4, 0), (0, 1), (0, 2), (4, 2), (0, 3), (1, 3), (2, 3), (3, 3)],
];

fn fit_grid(g: &mut Grid, cols: usize, rows: usize) -> bool {
    if g.cols == cols && g.rows == rows {
        return false;
    }
    *g = Grid { cols, rows, cells: vec![0; cols * rows], ghost: vec![0.0; cols * rows] };
    true
}

/// Conway's Game of Life on a dot matrix; onsets drop gliders and
/// R-pentominoes where the spectrum peaks, highs spark single cells.
fn life(c: &Ctx, s: &mut Scenes) {
    let cols = 36 + (c.detail * 92.0) as usize;
    let rows = ((cols as f32 * c.r.height() / c.r.width().max(1.0)).round() as usize).max(8);
    let g = &mut s.life;
    if fit_grid(g, cols, rows) {
        for _ in 0..(cols * rows / 6) {
            let i = s.rng.below(cols * rows);
            g.cells[i] = 1;
        }
    }
    let at = |x: i32, y: i32| (y.rem_euclid(rows as i32) as usize) * cols + x.rem_euclid(cols as i32) as usize;
    if c.a.onset {
        let peak = (0..BANDS).max_by(|&a, &b| c.a.bands[a].total_cmp(&c.a.bands[b])).unwrap_or(0);
        let x = (peak as f32 / BANDS as f32 * cols as f32) as i32;
        let y = s.rng.below(rows) as i32;
        let pat = PATTERNS[s.rng.below(PATTERNS.len())];
        let flip = s.rng.unit() < 0.5;
        for &(dx, dy) in pat {
            let dx = if flip { -dx } else { dx };
            g.cells[at(x + dx, y + dy)] = 1;
        }
    }
    s.life_clock += c.dt * 12.0;
    let mut steps = 0;
    while s.life_clock >= 1.0 && steps < 4 {
        s.life_clock -= 1.0;
        steps += 1;
        let sparks = (c.a.high * 8.0) as usize;
        for _ in 0..sparks {
            let i = s.rng.below(cols * rows);
            g.cells[i] = 1;
        }
        let mut next = vec![0u16; cols * rows];
        let mut alive = 0;
        for y in 0..rows as i32 {
            for x in 0..cols as i32 {
                let mut nb = 0;
                for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    nb += (g.cells[at(x + dx, y + dy)] > 0) as u8;
                }
                let i = at(x, y);
                let cell = g.cells[i];
                next[i] = match (cell > 0, nb) {
                    (true, 2) | (true, 3) => cell.saturating_add(1),
                    (false, 3) => 1,
                    _ => {
                        if cell > 0 {
                            g.ghost[i] = 1.0;
                        }
                        0
                    }
                };
                alive += (next[i] > 0) as usize;
            }
        }
        g.cells = next;
        // Never let it die out: a patch of noise somewhere.
        if alive < cols * rows / 60 {
            let (x0, y0) = (s.rng.below(cols) as i32, s.rng.below(rows) as i32);
            for _ in 0..40 {
                let i = at(x0 + s.rng.below(12) as i32, y0 + s.rng.below(12) as i32);
                g.cells[i] = 1;
            }
        }
    }
    let fade = (-c.real_dt * (1.2 + 7.0 * (1.0 - c.trail))).exp();
    for v in &mut g.ghost {
        *v *= fade;
    }
    let cs = (c.r.width() / cols as f32).min(c.r.height() / rows as f32);
    let origin = c.c() - vec2(cs * cols as f32, cs * rows as f32) * 0.5;
    let mut mesh = Mesh::default();
    let dot = cs * 0.8;
    for y in 0..rows {
        for x in 0..cols {
            let i = y * cols + x;
            let p = origin + vec2((x as f32 + 0.5) * cs, (y as f32 + 0.5) * cs);
            let cell = g.cells[i];
            if cell > 0 {
                let young = (1.0 - (cell as f32 - 1.0) / 10.0).clamp(0.0, 1.0);
                let col = mix(c.pal.line(), c.pal.ink((x * 4 / cols.max(1)) % 4), young);
                mesh.add_colored_rect(Rect::from_center_size(p, Vec2::splat(dot)), col);
            } else if g.ghost[i] > 0.04 {
                let col = alpha(c.pal.ink(1), g.ghost[i] * 0.45);
                mesh.add_colored_rect(Rect::from_center_size(p, Vec2::splat(dot * 0.55)), col);
            }
        }
    }
    c.p.add(Shape::mesh(mesh));
}

const RULES: [u8; 10] = [30, 90, 110, 150, 45, 105, 73, 54, 18, 126];

/// An elementary cellular automaton scrolling up; the spectrum writes into
/// its newest row and onsets change the rule (and the ink).
fn automaton(c: &Ctx, s: &mut Scenes) {
    let cols = 60 + (c.detail * 160.0) as usize;
    let cs = c.r.width() / cols as f32;
    let rows = (c.r.height() / cs).ceil() as usize + 1;
    if s.ca_cols != cols {
        s.ca_cols = cols;
        s.ca.clear();
        let mut first = vec![false; cols];
        first[cols / 2] = true;
        s.ca.push_back((first, 0));
    }
    if c.a.onset {
        s.ca_rule = (s.ca_rule + 1 + s.rng.below(3)) % RULES.len();
        s.ca_ink = (s.ca_ink + 1) % 4;
    }
    s.ca_clock += c.dt * 24.0;
    let mut steps = 0;
    while s.ca_clock >= 1.0 && steps < 6 {
        s.ca_clock -= 1.0;
        steps += 1;
        let rule = RULES[s.ca_rule];
        let prev = &s.ca.back().map(|r| r.0.clone()).unwrap_or_else(|| vec![false; cols]);
        let mut row: Vec<bool> = (0..cols)
            .map(|x| {
                let l = prev[(x + cols - 1) % cols] as u8;
                let m = prev[x] as u8;
                let r = prev[(x + 1) % cols] as u8;
                (rule >> (l << 2 | m << 1 | r)) & 1 == 1
            })
            .collect();
        for (x, cell) in row.iter_mut().enumerate() {
            let u = x as f32 / cols as f32;
            let e = c.band(u);
            if e > 0.55 && s.rng.unit() < (e - 0.55) * 0.5 {
                *cell = true;
            }
        }
        if row.iter().all(|&v| v) || row.iter().all(|&v| !v) {
            for _ in 0..3 {
                let i = s.rng.below(cols);
                row[i] = !row[i];
            }
        }
        s.ca.push_back((row, s.ca_ink));
        while s.ca.len() > rows {
            s.ca.pop_front();
        }
    }
    let mut mesh = Mesh::default();
    let n = s.ca.len();
    for (k, (row, ink)) in s.ca.iter().enumerate() {
        let y = c.r.bottom() - (n - k) as f32 * cs;
        let age = (n - k) as f32 / n.max(1) as f32;
        let col = alpha(c.pal.ink(*ink), 1.0 - age * (1.0 - c.trail) * 0.8);
        for (x, &on) in row.iter().enumerate() {
            if on {
                let p = pos2(c.r.left() + x as f32 * cs, y);
                mesh.add_colored_rect(Rect::from_min_size(p, Vec2::splat(cs * 0.86)), col);
            }
        }
    }
    c.p.add(Shape::mesh(mesh));
}

/// Truchet tiles: quarter circles that join into endless paths. Onsets
/// send a wave of flips across the grid.
fn truchet(c: &Ctx, s: &mut Scenes) {
    let cols = 5 + (c.detail * 15.0) as usize;
    let ts = c.r.width() / cols as f32;
    let rows = (c.r.height() / ts).ceil() as usize;
    if fit_grid(&mut s.truchet, cols, rows) {
        for v in s.truchet.cells.iter_mut() {
            *v = (s.rng.unit() < 0.5) as u16;
        }
        s.truchet_anim = s.truchet.cells.iter().map(|&v| v as f32).collect();
        s.truchet_flip = vec![-1.0; cols * rows];
    }
    s.clock += c.real_dt;
    let now = s.clock;
    if c.a.onset {
        let (ox, oy) = (s.rng.below(cols) as f32, s.rng.below(rows) as f32);
        for y in 0..rows {
            for x in 0..cols {
                let i = y * cols + x;
                if s.truchet_flip[i] < 0.0 && s.rng.unit() < 0.6 {
                    let d = ((x as f32 - ox).powi(2) + (y as f32 - oy).powi(2)).sqrt();
                    s.truchet_flip[i] = now + d * 0.04;
                }
            }
        }
    }
    let idle = (0.3 + 4.0 * c.a.high) * c.real_dt * c.speed;
    if s.rng.unit() < idle {
        let i = s.rng.below(cols * rows);
        s.truchet_flip[i] = now;
    }
    let follow = (c.real_dt * 9.0 * c.speed.max(0.2)).min(1.0);
    for i in 0..cols * rows {
        if s.truchet_flip[i] >= 0.0 && now >= s.truchet_flip[i] {
            s.truchet.cells[i] ^= 1;
            s.truchet_flip[i] = -1.0;
        }
        let target = s.truchet.cells[i] as f32;
        s.truchet_anim[i] += (target - s.truchet_anim[i]) * follow;
    }
    let g = 0.08 + 0.12 * c.a.bass;
    let radii: &[f32] = if c.detail > 0.35 { &[0.5 - g, 0.5 + g] } else { &[0.5] };
    for y in 0..rows {
        for x in 0..cols {
            let i = y * cols + x;
            let center = pos2(c.r.left() + (x as f32 + 0.5) * ts, c.r.top() + (y as f32 + 0.5) * ts);
            let rot = s.truchet_anim[i] * PI * 0.5;
            let (sn, cs) = rot.sin_cos();
            let e = c.band(x as f32 / cols as f32);
            let col = c.pal.cyc((x + y) as f32 * 0.035 + c.t * 0.02);
            let w = c.w(1.0 + 3.0 * e);
            for (corner, a0) in [(vec2(-0.5, -0.5), 0.0f32), (vec2(0.5, 0.5), PI)] {
                for &r in radii {
                    let pts: Vec<Pos2> = (0..=10)
                        .map(|k| {
                            let a = a0 + k as f32 / 10.0 * PI * 0.5;
                            let v = (corner + vec2(a.cos(), a.sin()) * r) * ts;
                            center + vec2(v.x * cs - v.y * sn, v.x * sn + v.y * cs)
                        })
                        .collect();
                    c.line(pts, w, col);
                }
            }
        }
    }
}

/// Overprinted sets of rings whose centres orbit each other: the
/// interference does the rest.
fn moire(c: &Ctx, _s: &mut Scenes) {
    let rings = 12 + (c.detail * 26.0) as usize;
    let rad = c.r.size().length() * 0.6;
    let spacing = rad / rings as f32;
    let m = c.rad() * (0.08 + 0.4 * c.a.bass + 0.06 * (c.t * 0.3).sin().abs());
    let t = c.t;
    let mut centres = vec![
        c.c() + vec2((t * 0.37).sin(), (t * 0.29).cos()) * m,
        c.c() - vec2((t * 0.31).cos(), (t * 0.41).sin()) * m,
    ];
    if c.detail > 0.4 {
        centres.push(c.c() + vec2((t * 0.23).cos(), (t * 0.19).sin()) * m * 1.5);
    }
    let w = spacing * (0.12 + 0.16 * c.a.level);
    for (k, o) in centres.iter().enumerate() {
        let col = alpha(if k == 0 { c.pal.line() } else { c.pal.ink(k - 1) }, 0.8);
        for i in 1..=rings {
            let wobble = 1.0 + 0.03 * c.a.mid * (t * 2.0 + i as f32 * 0.7).sin();
            c.p.circle_stroke(*o, spacing * i as f32 * wobble, Stroke::new(w, col));
        }
    }
}

const ATTRACTORS: [[f32; 4]; 6] = [
    [-1.4, 1.6, 1.0, 0.7],
    [1.7, 1.7, 0.06, 1.2],
    [-1.7, 1.3, -0.1, -1.21],
    [1.5, -1.8, 1.6, 0.9],
    [-1.8, -2.0, -0.5, -0.9],
    [1.6, -0.6, -1.2, 1.6],
];

/// A Clifford attractor's cloud of points, its constants drifting with the
/// sound; onsets move it on to another shape, the two crossfading.
fn attractor(c: &Ctx, s: &mut Scenes) {
    if c.a.onset && s.attractor_mix <= 0.0 && s.rng.unit() < 0.35 {
        s.attractor_mix = 0.0001;
    }
    if s.attractor_mix > 0.0 {
        s.attractor_mix += c.real_dt / 1.6;
        if s.attractor_mix >= 1.0 {
            s.attractor = (s.attractor + 1) % ATTRACTORS.len();
            s.attractor_mix = 0.0;
        }
    }
    let n = 2500 + (c.detail * 12_500.0) as usize;
    let m = smoothstep(s.attractor_mix);
    let mut mesh = Mesh::default();
    mesh.reserve_vertices(n * 4);
    clifford(c, ATTRACTORS[s.attractor], n, 1.0 - m, &mut mesh);
    if m > 0.0 {
        clifford(c, ATTRACTORS[(s.attractor + 1) % ATTRACTORS.len()], n, m, &mut mesh);
    }
    c.p.add(Shape::mesh(mesh));
}

/// Iterates a Clifford attractor from `base`, nudged by the sound, and adds
/// its points at `opacity`. Some nudges fall into a periodic orbit (a few
/// points instead of a cloud): then it draws `base` as it is, which is
/// chaotic.
fn clifford(c: &Ctx, base: [f32; 4], n: usize, opacity: f32, mesh: &mut Mesh) {
    let t = c.t;
    let nudged = [
        base[0] + 0.05 * (t * 0.13).sin() + 0.12 * c.a.bass,
        base[1] + 0.05 * (t * 0.11).cos() + 0.1 * c.a.mid,
        base[2] + 0.04 * (t * 0.07).sin() + 0.08 * c.a.high,
        base[3] + 0.04 * (t * 0.09).cos(),
    ];
    let orbit = |k: [f32; 4]| -> Vec<(f32, f32, f32)> {
        let [a, b, cc, d] = k;
        let (mut x, mut y) = (0.1f32, 0.1f32);
        let mut pts = Vec::with_capacity(n);
        for i in 0..n + 20 {
            let nx = (a * y).sin() + cc * (a * x).cos();
            let ny = (b * x).sin() + d * (b * y).cos();
            let v = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
            x = nx;
            y = ny;
            if i >= 20 {
                pts.push((x, y, v));
            }
        }
        pts
    };
    // How much of the plane the points cover, on a coarse grid.
    let spread = |pts: &[(f32, f32, f32)], k: [f32; 4]| {
        let (bx, by) = (1.0 + k[2].abs(), 1.0 + k[3].abs());
        let mut cells = [0u64; 40 * 40 / 64 + 1];
        for &(x, y, _) in pts.iter().take(3000) {
            let i = (((x / bx * 0.5 + 0.5) * 39.0) as usize).min(39);
            let j = (((y / by * 0.5 + 0.5) * 39.0) as usize).min(39);
            let cell = j * 40 + i;
            cells[cell / 64] |= 1 << (cell % 64);
        }
        cells.iter().map(|w| w.count_ones()).sum::<u32>()
    };
    let mut k = nudged;
    let mut pts = orbit(k);
    if spread(&pts, k) < 80 {
        k = base;
        pts = orbit(k);
    }
    let (bx, by) = (1.0 + k[2].abs(), 1.0 + k[3].abs());
    let scale = (c.r.width() / (2.0 * bx)).min(c.r.height() / (2.0 * by)) * 0.92;
    let o = c.c();
    let size = c.w(1.0).max(1.0);
    for (x, y, v) in pts {
        let col = alpha(c.pal.grad((v / 2.2).min(1.0)), 0.8 * opacity);
        mesh.add_colored_rect(Rect::from_center_size(o + vec2(x, y) * scale, Vec2::splat(size)), col);
    }
}

/// The sound against itself a quarter period later: a pure tone draws a
/// circle, a chord a knot. Echoes of past frames spiral out behind it.
fn phase(c: &Ctx, s: &mut Scenes) {
    let target = (c.a.sr / c.a.pitch.max(20.0) / 4.0).clamp(3.0, 600.0);
    s.tau += (target - s.tau) * (c.real_dt * 4.0).min(1.0);
    let tau = s.tau.round() as usize;
    let n = 1024usize;
    let base = RING - n - tau - 1;
    let g = (c.a.gain * 0.72).min(30.0);
    let o = c.c();
    let rad = c.rad() * 0.9;
    let point = |i: usize| {
        let x = c.a.wave[base + i] * g;
        let y = c.a.wave[base + i + tau] * g;
        vec2((x - y) * 0.7071, -(x + y) * 0.7071)
    };
    let pts: Vec<Vec2> = (0..n).map(point).collect();
    // Graticule.
    let faint = alpha(c.pal.line(), 0.12);
    for a in [PI * 0.25, PI * 0.75] {
        let d = vec2(a.cos(), a.sin()) * rad;
        c.line(vec![o - d, o + d], c.w(0.7), faint);
    }
    c.p.circle_stroke(o, rad, Stroke::new(c.w(0.7), faint));
    let keep = (c.trail * 14.0) as usize;
    let small: Vec<Vec2> = pts.iter().step_by(4).copied().collect();
    s.echoes.push_front(small);
    while s.echoes.len() > keep + 1 {
        s.echoes.pop_back();
    }
    for (k, e) in s.echoes.iter().enumerate().skip(1).rev() {
        let f = k as f32 / (keep + 1) as f32;
        let grow = 1.0 + 0.07 * k as f32;
        let rot = k as f32 * 0.06 * (1.0 + c.a.level);
        let (sn, cs) = rot.sin_cos();
        let line: Vec<Pos2> = e.iter().map(|v| o + vec2(v.x * cs - v.y * sn, v.x * sn + v.y * cs) * rad * grow).collect();
        c.line(line, c.w(0.8), alpha(c.pal.ink(k), 0.4 * (1.0 - f) * (1.0 - f)));
    }
    let line: Vec<Pos2> = pts.iter().map(|v| o + *v * rad).collect();
    c.gradient_line(&line, 8, c.w(1.2 + c.a.beat), |u| alpha(c.pal.grad(u), 0.25 + 0.75 * u));
}

/// A dot-matrix spectrum with falling peaks.
fn dots(c: &Ctx, s: &mut Scenes) {
    let cols = 16 + (c.detail * 40.0) as usize;
    let pitch = c.r.width() / cols as f32;
    let rows = ((c.r.height() / pitch).floor() as usize).max(4);
    if s.peaks.len() != cols {
        s.peaks = vec![0.0; cols];
    }
    let top = c.r.bottom() - rows as f32 * pitch;
    let dot = pitch * 0.62;
    let mut mesh = Mesh::default();
    let unlit = alpha(c.pal.line(), 0.07);
    for x in 0..cols {
        let u = x as f32 / (cols - 1) as f32;
        let lit = c.band(u) * rows as f32;
        let fall = c.real_dt * rows as f32 * (0.15 + 0.6 * (1.0 - c.trail));
        s.peaks[x] = lit.max(s.peaks[x] - fall);
        let peak = s.peaks[x].floor() as usize;
        for y in 0..rows {
            let p = pos2(c.r.left() + (x as f32 + 0.5) * pitch, top + (rows - 1 - y) as f32 * pitch + pitch * 0.5);
            let rect = Rect::from_center_size(p, Vec2::splat(dot));
            let col = if (y as f32) < lit {
                c.pal.grad(y as f32 / rows as f32)
            } else if y == peak && peak > 0 {
                c.pal.line()
            } else {
                unlit
            };
            mesh.add_colored_rect(rect, col);
        }
    }
    c.p.add(Shape::mesh(mesh));
}

/// Planets on dashed orbits, one orbit a band, turning in a polyrhythm;
/// onsets draw the constellation between them.
fn orbits(c: &Ctx, s: &mut Scenes) {
    let n = 4 + (c.detail * 8.0) as usize;
    if s.orbits.len() != n {
        s.orbits = (0..n).map(|i| i as f32 * 1.3).collect();
    }
    let o = c.c();
    let rad = c.rad() * 0.95;
    let mut firsts = Vec::with_capacity(n);
    for i in 0..n {
        let u = (i as f32 + 0.5) / n as f32;
        let e = c.band(u);
        let r = rad * (0.14 + 0.82 * u);
        let omega = 0.25 + 0.9 * (n - i) as f32 / n as f32;
        s.orbits[i] += c.dt * omega * (0.5 + c.a.level);
        c.dashed_circle(o, r, 18 + i * 6, 0.45, 0.0, c.w(0.7), alpha(c.pal.line(), 0.22));
        let planets = 1 + i % 3;
        let col = c.pal.ink(i);
        let arc = 0.3 + 1.6 * c.trail;
        for k in 0..planets {
            let th = s.orbits[i] + k as f32 / planets as f32 * TAU;
            let p = o + vec2(th.cos(), th.sin()) * r;
            let wake: Vec<Pos2> = (0..=16)
                .map(|j| {
                    let a = th - arc * (1.0 - j as f32 / 16.0);
                    o + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            c.gradient_line(&wake, 6, c.w(1.4), |f| alpha(col, f * 0.8));
            let pr = c.w(2.5 + 9.0 * e);
            c.p.circle_filled(p, pr, c.pal.bg);
            c.p.circle_stroke(p, pr, Stroke::new(c.w(1.3), col));
            if k == 0 {
                firsts.push(p);
            }
        }
    }
    if c.a.beat > 0.03 {
        c.line(firsts, c.w(0.9), alpha(c.pal.line(), 0.2 + 0.7 * c.a.beat));
    }
    let sun = rad * (0.05 + 0.04 * c.a.bass);
    c.p.circle_stroke(o, sun, Stroke::new(c.w(1.6), c.pal.line()));
}

// ---------------------------------------------------------------------------
// Dots and 3D
// ---------------------------------------------------------------------------

/// Round dots in a mesh (no anti-aliasing: for thousands of them), with the
/// unit circle worked out once a frame.
struct Discs {
    six: [Vec2; 6],
    ten: [Vec2; 10],
}

impl Discs {
    fn new() -> Self {
        fn ring<const N: usize>() -> [Vec2; N] {
            std::array::from_fn(|k| {
                let a = k as f32 / N as f32 * TAU;
                vec2(a.cos(), a.sin())
            })
        }
        Self { six: ring(), ten: ring() }
    }

    /// A dot of radius `r`: a square while tiny, a polygon otherwise.
    fn add(&self, mesh: &mut Mesh, p: Pos2, r: f32, col: Color32) {
        if r < 1.1 {
            mesh.add_colored_rect(Rect::from_center_size(p, Vec2::splat(r * 1.8)), col);
            return;
        }
        let rim: &[Vec2] = if r < 3.0 { &self.six } else { &self.ten };
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(p, col);
        for d in rim {
            mesh.colored_vertex(p + *d * r, col);
        }
        let n = rim.len() as u32;
        for k in 0..n {
            mesh.add_triangle(i, i + 1 + k, i + 1 + (k + 1) % n);
        }
    }
}

/// A number in 0..1, always the same for `i` and `salt`.
fn hash(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ salt;
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA77);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE3D);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// A point of the 3D scenes: x to the right, y up, z away from the eye.
#[derive(Clone, Copy)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl std::ops::Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl std::ops::Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, k: f32) -> V3 {
        v3(self.x * k, self.y * k, self.z * k)
    }
}

impl V3 {
    fn lerp(self, o: V3, t: f32) -> V3 {
        self * (1.0 - t) + o * t
    }

    fn cross(self, o: V3) -> V3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    fn normalized(self) -> V3 {
        self * (1.0 / (self.x * self.x + self.y * self.y + self.z * self.z).sqrt().max(1e-6))
    }

    /// Turned by `a` round the vertical axis.
    fn yaw(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        v3(self.x * c + self.z * s, self.y, self.z * c - self.x * s)
    }

    /// Tipped by `a` round the horizontal axis.
    fn pitch(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        v3(self.x, self.y * c - self.z * s, self.y * s + self.z * c)
    }
}

/// A halftone halo: rays of dots round an empty centre, biggest in a band
/// half way out and shrinking to specks inside and out, with faint streaks
/// along the rays. It turns, swells with the bass, sends a ripple out on
/// onsets, and its colours travel round it.
fn halo(c: &Ctx, s: &mut Scenes) {
    let rays = 96 + (c.detail * 144.0) as usize;
    let per = 16 + (c.detail * 18.0) as usize;
    if c.a.onset {
        s.halo_waves.push(0.0);
        s.halo_hue_to += 0.1 + 0.12 * s.rng.unit();
        s.halo_spin += 0.5;
    }
    s.halo_hue_to += c.dt * 0.02;
    s.halo_hue += (s.halo_hue_to - s.halo_hue) * (c.real_dt * 2.5).min(1.0);
    s.halo_spin *= (-c.real_dt * 2.0).exp();
    s.halo_rot += c.dt * (0.05 + 0.25 * c.a.level) + c.real_dt * s.halo_spin * 0.4;
    for w in &mut s.halo_waves {
        *w += c.real_dt * (0.6 + 0.3 * c.speed.min(2.0));
    }
    s.halo_waves.retain(|w| *w < 1.3);
    let o = c.c();
    let rad = c.rad() * 0.97;
    let (r_in, r_out) = (rad * 0.14, rad);
    let dr = (r_out - r_in) / (per - 1) as f32;
    let rc = rad * (0.55 + 0.07 * c.a.bass + 0.05 * c.a.beat);
    let wd = rad * (0.12 + 0.07 * c.a.level);
    let size = (dr * 0.5).min(TAU * rc / rays as f32 * 0.5) * (0.9 + 0.3 * c.a.beat);
    let discs = Discs::new();
    let mut streaks = Mesh::default();
    let mut dots = Mesh::default();
    for i in 0..rays {
        let f = i as f32 / rays as f32;
        let th = f * TAU + s.halo_rot;
        let dir = vec2(th.cos(), th.sin());
        // The spectrum round the ring, mirrored so it joins up: the bass on
        // one side, the highs opposite.
        let e = c.band((1.0 - (2.0 * f - 1.0).abs()) * 0.85);
        let reach = rc + wd * (1.0 + 2.2 * e + 0.9 * hash(i as u32, s.salt));
        let col = c.pal.cyc(f + s.halo_hue);
        let from = (rc - wd * 1.6).max(r_in);
        seg(&mut streaks, o + dir * from, o + dir * reach.min(r_out), c.w(0.7) + size * 0.5, alpha(col, 0.08 + 0.2 * e));
        let stagger = if i % 2 == 1 { 0.5 * dr } else { 0.0 };
        for j in 0..per {
            let r = r_in + j as f32 * dr + stagger;
            if r > r_out {
                break;
            }
            let mut k = if r < rc {
                (-((rc - r) / wd).powi(2)).exp()
            } else {
                (-((r - rc) / (reach - rc).max(1.0) * 1.7).powi(2)).exp()
            };
            for w in &s.halo_waves {
                let rw = r_in + w * (r_out - r_in);
                k += 0.7 * (1.0 - w / 1.3) * (-((r - rw) / (dr * 1.3)).powi(2)).exp();
            }
            let k = k.min(1.25);
            let dot = size * k;
            if dot < 0.3 {
                continue;
            }
            let col = c.pal.cyc(f + s.halo_hue + (r / rad - 0.55) * 0.12);
            discs.add(&mut dots, o + dir * r, dot, alpha(col, 0.35 + 0.65 * k.min(1.0)));
        }
    }
    c.p.add(Shape::mesh(streaks));
    c.p.add(Shape::mesh(dots));
}

/// How far the bands push a Globe shape's dots out: less on the thinner
/// ones, which would otherwise close up.
const GLOBE_PUSH: [f32; 3] = [0.3, 0.16, 0.07];

/// A point of a Globe shape and its normal, from `u` and `v` in 0..1: a
/// sphere (`u` the latitude), a torus (`u` round the tube, 0.5 outside) or
/// a trefoil knot (`u` round the tube, `v` along it).
fn globe_point(shape: usize, u: f32, v: f32) -> (V3, V3) {
    match shape {
        0 => {
            let th = (1.0 - 2.0 * u).clamp(-1.0, 1.0).acos();
            let ph = TAU * v;
            let n = v3(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
            (n, n)
        }
        1 => {
            let (big, small) = (TAU * v, TAU * u + PI);
            let n = v3(small.cos() * big.cos(), small.sin(), small.cos() * big.sin());
            (v3(big.cos() * 0.68, 0.0, big.sin() * 0.68) + n * 0.32, n)
        }
        _ => {
            let a = TAU * v;
            let k = 1.0 / 3.1;
            let p = v3(a.sin() + 2.0 * (2.0 * a).sin(), a.cos() - 2.0 * (2.0 * a).cos(), -(3.0 * a).sin()) * k;
            let t = v3(a.cos() + 4.0 * (2.0 * a).cos(), -a.sin() + 4.0 * (2.0 * a).sin(), -3.0 * (3.0 * a).cos());
            // The tangent never points along z, so this frame never folds.
            let nn = t.cross(v3(0.0, 0.0, 1.0)).normalized();
            let b = t.normalized().cross(nn);
            let n = nn * (TAU * u).cos() + b * (TAU * u).sin();
            (p + n * 0.12, n)
        }
    }
}

/// A cloud of dots on a sphere, a torus or a trefoil knot, turning in
/// space. Each dot is pushed out by its band (the bass round the middle,
/// the highs at the ends); onsets give it a spin and now and then morph it
/// into the next shape.
fn globe(c: &Ctx, s: &mut Scenes) {
    if c.a.onset {
        s.globe_spin += 1.0;
        if s.globe_mix <= 0.0 && s.rng.unit() < 0.15 {
            s.globe_mix = 0.0001;
        }
    }
    if s.globe_mix > 0.0 {
        s.globe_mix += c.real_dt;
        if s.globe_mix >= 1.0 {
            s.globe_shape = (s.globe_shape + 1) % 3;
            s.globe_mix = 0.0;
        }
    }
    s.globe_spin *= (-c.real_dt * 1.5).exp();
    s.globe_yaw += c.dt * (0.3 + 0.5 * c.a.level) + c.real_dt * s.globe_spin;
    let pitch = 0.45 + 0.3 * (c.t * 0.11).sin();
    let n = 900 + (c.detail * 3100.0) as usize;
    let m = smoothstep(s.globe_mix);
    let (from, to) = (s.globe_shape, (s.globe_shape + 1) % 3);
    let dist = 3.2;
    let f = c.rad() * 2.3;
    let o = c.c();
    let swell = 1.0 + 0.1 * c.a.beat;
    let bounds = c.r.expand(12.0);
    let mut pts: Vec<(f32, Pos2, f32, Color32)> = Vec::with_capacity(n);
    for i in 0..n {
        let u = (i as f32 + 0.5) / n as f32;
        let v = (i as f32 * 0.618_034).fract();
        let e = c.band((2.0 * u - 1.0).abs() * 0.9);
        let (mut p, mut nrm) = globe_point(from, u, v);
        if m > 0.0 {
            let (p2, n2) = globe_point(to, u, v);
            p = p.lerp(p2, m);
            nrm = nrm.lerp(n2, m);
        }
        let push = GLOBE_PUSH[from] + (GLOBE_PUSH[to] - GLOBE_PUSH[from]) * m;
        let q = ((p + nrm * (push * e)) * swell).yaw(s.globe_yaw).pitch(pitch);
        let z = q.z + dist;
        let sp = o + vec2(q.x, -q.y) * (f / z);
        if !bounds.contains(sp) {
            continue;
        }
        let depth = ((z - dist + 1.3) / 2.6).clamp(0.0, 1.0);
        let col = mix(c.pal.cyc(u * 0.5 + c.t * 0.015), c.pal.bg, 0.72 * depth);
        pts.push((z, sp, c.w(0.8 + 2.0 * e) * (dist / z), col));
    }
    // Far dots first, so the near ones cover them.
    pts.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let discs = Discs::new();
    let mut mesh = Mesh::default();
    for (_, p, r, col) in pts {
        discs.add(&mut mesh, p, r, col);
    }
    c.p.add(Shape::mesh(mesh));
}

/// A new row of the Terrain: rolling hills, plus the spectrum when `live`
/// (the low bands either side of a flat middle, the highs out at the edges).
fn terrain_row(c: &Ctx, s: &mut Scenes, cols: usize, live: bool) -> Vec<f32> {
    s.terrain_n += 1.0;
    let z = s.terrain_n * 0.23;
    let mut row: Vec<f32> = (0..=cols)
        .map(|j| {
            let u = j as f32 / cols as f32;
            let d = (2.0 * u - 1.0).abs();
            let side = smoothstep((d - 0.08) * 2.6);
            let hills = 0.5 + 0.5 * ((u * 9.0 + z).sin() * (u * 4.0 - z * 0.7).cos());
            let (e, jitter) = if live { (c.band(d * 0.88), s.rng.unit() * 0.05 * c.a.level) } else { (0.0, 0.0) };
            (0.05 + 0.12 * hills + 0.85 * e + jitter) * side
        })
        .collect();
    let prev = row.clone();
    for j in 1..cols {
        row[j] = (prev[j - 1] + 2.0 * prev[j] + prev[j + 1]) * 0.25;
    }
    row
}

/// The spectrum as a landscape: each new row is the sound now, in front,
/// and the rows move away into the distance, drawn as a wireframe that
/// hides what is behind it, under a sun in outline. Onsets mark their row
/// with a bright line.
fn terrain(c: &Ctx, s: &mut Scenes) {
    let cols = 24 + (c.detail * 40.0) as usize;
    let rows = 18 + (c.detail * 26.0) as usize;
    if s.terrain.front().is_none_or(|r| r.0.len() != cols + 1) {
        s.terrain.clear();
        for _ in 0..=rows {
            let row = terrain_row(c, s, cols, false);
            s.terrain.push_back((row, 0.0));
        }
    }
    s.terrain_clock += c.dt * (5.0 + 6.0 * c.a.level);
    let mut steps = 0;
    while s.terrain_clock >= 1.0 {
        s.terrain_clock -= 1.0;
        steps += 1;
        if steps > 4 {
            s.terrain_clock = s.terrain_clock.fract();
            break;
        }
        let row = terrain_row(c, s, cols, true);
        s.terrain.push_front((row, 0.0));
        s.terrain.truncate(80);
    }
    if c.a.onset
        && let Some(row) = s.terrain.front_mut()
    {
        row.1 = 1.0;
    }
    let n = rows.min(s.terrain.len() - 1);
    let r = c.r;
    let (eye, near, half, amp) = (1.0f32, 1.0f32, 1.8f32, 0.9f32);
    let dz = 9.0 / rows as f32;
    let hy = r.top() + r.height() * 0.3;
    let f = (0.62 * r.height() * near / eye).max(0.55 * r.width() * near / half);
    let frac = s.terrain_clock;

    // Behind it: the horizon, and a sun in outline above it.
    c.line(vec![pos2(r.left(), hy), pos2(r.right(), hy)], c.w(0.8), alpha(c.pal.line(), 0.25));
    let sky = c.p.with_clip_rect(Rect::from_min_max(r.min, pos2(r.right(), hy)).intersect(c.p.clip_rect()));
    let sun_r = r.height() * (0.15 + 0.04 * c.a.bass);
    let sun = pos2(r.center().x, hy - sun_r * 0.55);
    let sun_col = mix(c.pal.ink(1), c.pal.line(), c.a.beat * 0.6);
    sky.circle_stroke(sun, sun_r, Stroke::new(c.w(1.3), sun_col));
    for k in 0..5 {
        let y = hy - sun_r * 0.16 * (k as f32 + 1.0 - (c.t * 0.3).fract());
        let dx = (sun_r * sun_r - (y - sun.y).powi(2)).max(0.0).sqrt();
        sky.line_segment([pos2(sun.x - dx, y), pos2(sun.x + dx, y)], Stroke::new(c.w(1.0), alpha(sun_col, 0.8)));
    }

    let pts: Vec<Vec<Pos2>> = (0..=n)
        .map(|k| {
            let z = near + (k as f32 + frac) * dz;
            s.terrain[k]
                .0
                .iter()
                .enumerate()
                .map(|(j, h)| {
                    let x = (j as f32 / cols as f32 * 2.0 - 1.0) * half;
                    pos2(r.center().x + x * f / z, hy + (eye - h * amp) * f / z)
                })
                .collect()
        })
        .collect();
    // Far to near: each strip hides what is behind it, then draws its lines.
    for k in (0..n).rev() {
        let depth = (k as f32 + frac) / n as f32;
        let fog = depth.powf(1.2) * 0.9;
        let w = c.w(0.6 + 1.0 * (1.0 - depth));
        let (far, front) = (&pts[k + 1], &pts[k]);
        let (hf, hn) = (&s.terrain[k + 1].0, &s.terrain[k].0);
        let mut mesh = Mesh::default();
        // Just under the far line, so it keeps its whole width.
        let under = vec2(0.0, w * 0.5);
        for j in 0..cols {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(far[j] + under, c.pal.bg);
            mesh.colored_vertex(far[j + 1] + under, c.pal.bg);
            mesh.colored_vertex(front[j + 1], c.pal.bg);
            mesh.colored_vertex(front[j], c.pal.bg);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        for j in 0..=cols {
            let col = mix(c.pal.grad((hf[j] + hn[j]) * 0.6), c.pal.bg, fog);
            seg(&mut mesh, far[j], front[j], w, col);
        }
        let mark = s.terrain[k].1;
        for j in 0..cols {
            let col = mix(c.pal.grad((hn[j] + hn[j + 1]) * 0.6), c.pal.line(), mark);
            seg(&mut mesh, front[j], front[j + 1], w * (1.0 + mark), mix(col, c.pal.bg, fog));
        }
        c.p.add(Shape::mesh(mesh));
    }
    for row in s.terrain.iter_mut() {
        row.1 *= (-c.real_dt * 0.4).exp();
    }
}

/// Flying through a lattice of dots: the bass and the level set the speed,
/// onsets kick it, the mids roll the view. Each layer has its colour, each
/// dot swells with its band (the bass in the middle of the view, the highs
/// at the edges) and streaks as it rushes past.
fn flight(c: &Ctx, s: &mut Scenes) {
    let depth = 10 + (c.detail * 6.0) as i32;
    let target = 1.0 + 5.0 * c.a.bass + 2.0 * c.a.level;
    if c.a.onset {
        s.flight_v += 4.0;
    }
    s.flight_v += (target - s.flight_v) * (c.real_dt * 1.5).min(1.0);
    s.flight_z += c.dt * s.flight_v;
    s.flight_roll += c.dt * (0.06 + 0.4 * c.a.mid);
    let (sn, cs) = s.flight_roll.sin_cos();
    // The eye weaves about, a little off the middle of a cell.
    let eye = vec2(0.5 + 0.3 * (c.t * 0.19).sin(), 0.5 + 0.3 * (c.t * 0.15).cos());
    let fl = s.flight_z.floor();
    let fr = s.flight_z - fl;
    let f = c.rad() * 1.15;
    let o = c.c();
    let bounds = c.r.expand(6.0);
    // How far behind each dot its streak starts.
    let tail = (s.flight_v * 0.025 * (0.4 + c.trail)).min(0.6);
    let keep = 0.35 + 0.35 * c.detail;
    let reach = c.r.size().length() * 0.5 / f;
    let rad = c.rad();
    let discs = Discs::new();
    let mut mesh = Mesh::default();
    for iz in (1..=depth).rev() {
        let z = iz as f32 - fr;
        if z < 0.12 {
            continue;
        }
        let layer = iz as f32 + fl;
        let id = (layer as i64).rem_euclid(1 << 20) as u32;
        let fog = (z / depth as f32).powf(1.3);
        let flash = c.a.beat * (1.0 - z / 4.0).max(0.0);
        let col = mix(mix(c.pal.cyc(layer * 0.06 + c.t * 0.01), c.pal.line(), flash), c.pal.bg, fog * 0.9);
        let k = (reach * z).ceil() as i32 + 1;
        let scale = f / z;
        let back = f / (z + tail);
        for iy in -k..=k {
            for ix in -k..=k {
                // Somewhere in its cell, the same every time it comes round.
                let cell = id.wrapping_mul(0x01F3_5A67) ^ ((ix + 512) as u32).wrapping_mul(0x9E37) ^ ((iy + 512) as u32).wrapping_mul(0x632B_E5AB);
                if hash(cell, s.salt) > keep {
                    continue;
                }
                let jx = hash(cell ^ 0x5555, s.salt) - 0.5;
                let jy = hash(cell ^ 0xAAAA, s.salt) - 0.5;
                let (x, y) = (ix as f32 + jx - eye.x, iy as f32 + jy - eye.y);
                let d = vec2(x * cs - y * sn, x * sn + y * cs);
                let p = o + d * scale;
                if !bounds.contains(p) {
                    continue;
                }
                let e = c.band(((p - o).length() / (rad * 1.6)).min(1.0));
                let dot = (0.045 * scale * (0.55 + 1.1 * e)).min(rad * 0.05).max(0.5);
                let q = o + d * back;
                if (p - q).length_sq() > 1.0 {
                    seg(&mut mesh, q, p, dot * 0.6, alpha(col, 0.35));
                }
                discs.add(&mut mesh, p, dot, col);
            }
        }
    }
    c.p.add(Shape::mesh(mesh));
}

fn draw_scene(i: usize, c: &Ctx, s: &mut Scenes, seed: f32) {
    match i {
        0 => ridges(c, s),
        1 => mandala(c, s),
        2 => harmonograph(c, s),
        3 => spiro(c, s),
        4 => tunnel(c, s),
        5 => flow(c, s, seed),
        6 => life(c, s),
        7 => automaton(c, s),
        8 => truchet(c, s),
        9 => moire(c, s),
        10 => attractor(c, s),
        11 => phase(c, s),
        12 => dots(c, s),
        13 => orbits(c, s),
        14 => halo(c, s),
        15 => globe(c, s),
        16 => terrain(c, s),
        _ => flight(c, s),
    }
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, (input, _thru): (bool, bool)) {
    let p = &m.params;
    let z = v.z;
    let size = size(p);
    let local = screen_local(size, frameless(p));
    let screen = v.r(local.min.x, local.min.y, local.max.x, local.max.y);
    let now = v.ui.input(|i| i.time);
    let pal = Pal::of(palette_index(p), accent);
    let scene = scene_of(p);
    let listens_in = iris::listens_to_input(p, input);
    let hovered = v.ui.rect_contains_pointer(v.r(0.0, 0.0, size.x, size.y)) || show_controls();

    // The screen and its picture.
    v.painter.rect_filled(screen, 4.0 * z, pal.bg);
    let clip = screen.intersect(v.painter.clip_rect());
    let visible = clip.is_positive();
    let painter = v.painter.with_clip_rect(clip);
    let mut gear_rect = None;
    STATES.with(|states| {
        let mut states = states.borrow_mut();
        states.retain(|_, s| now - s.seen < 30.0 || now < s.seen);
        let seed = p.data[data::SEED].get();
        let st = states.entry(id).or_insert_with(|| State::new(seed));
        if st.seed != seed {
            *st = State::new(seed);
        }
        let dt = if st.last.is_finite() { ((now - st.last) as f32).clamp(0.0, 0.1) } else { 1.0 / 60.0 };
        st.last = now;
        st.seen = now;
        if st.scene != scene {
            if st.scene != usize::MAX {
                st.hud = 1.6;
            }
            st.scene = scene;
        }
        st.hud = (st.hud - dt).max(0.0);
        st.a.update(p, dt);
        let speed = p.get(SPEED);
        st.t += dt * speed * (0.7 + 0.6 * st.a.level);

        if visible {
            let ctx = Ctx {
                p: &painter,
                r: screen,
                pal,
                k: z * (local.height() / 300.0).sqrt().clamp(0.85, 2.2),
                t: st.t,
                dt: dt * speed,
                real_dt: dt,
                speed,
                detail: p.get(DETAIL),
                trail: p.get(TRAIL),
                a: &st.a,
            };
            draw_scene(scene, &ctx, &mut st.s, seed);
        }

        // The scene's name, while the pointer is over the node or just
        // after the scene changed.
        let show = hovered || st.hud > 0.0;
        if show && z > 0.3 {
            let source = if listens_in { "IN" } else { "MAIN" };
            let text = format!("{}  ·  {}", SCENES[scene], source);
            let font = theme::mono(9.0 * z);
            let galley = v.painter.layout_no_wrap(text, font, theme::TEXT);
            let pill = Rect::from_min_size(screen.min + vec2(8.0, 8.0) * z, galley.size() + vec2(14.0, 8.0) * z);
            let a = if hovered { 1.0 } else { (st.hud / 0.4).min(1.0) };
            painter.rect_filled(pill, 9.0 * z, alpha(Color32::from_rgb(14, 15, 18), 0.86 * a));
            painter.rect_stroke(pill, 9.0 * z, Stroke::new(1.0, alpha(accent, 0.7 * a)), StrokeKind::Inside);
            painter.galley_with_override_text_color(pill.min + vec2(7.0, 4.0) * z, galley, alpha(theme::TEXT, a));
        }
    });

    // Nothing to listen to.
    if listens_in && !input && z > 0.3 {
        v.painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            "Patch a sound into IN, or listen to the main mix (⚙)",
            theme::font(10.0 * z),
            alpha(pal.line(), 0.55),
        );
    }

    // The gear: the settings popup, next to the node.
    let popup_id = Id::unique(("iris-settings", id));
    if show_controls() && !egui::Popup::is_id_open(v.ui.ctx(), popup_id) {
        egui::Popup::open_id(v.ui.ctx(), popup_id);
    }
    let open = egui::Popup::is_id_open(v.ui.ctx(), popup_id);
    if hovered || open {
        let c = screen.right_top() + vec2(-16.0, 16.0) * z;
        let r = Rect::from_center_size(c, Vec2::splat(22.0 * z));
        gear_rect = Some(r);
        let resp = v.ui.interact(r, Id::unique(("iris-gear", id)), Sense::click());
        let hot = resp.hovered() || open;
        v.painter.circle_filled(c, 11.0 * z, alpha(Color32::from_rgb(14, 15, 18), 0.86));
        v.painter.circle_stroke(c, 11.0 * z, Stroke::new(1.0, if hot { accent } else { alpha(theme::TEXT_DIM, 0.6) }));
        gear(v.painter, c, z, if hot { accent } else { theme::TEXT });
        let resp = resp.on_hover_text("Iris settings");
        if resp.hovered() {
            v.ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let node_rect = v.r(0.0, 0.0, size.x, size.y);
        egui::Popup::from_toggle_button_response(&resp)
            .id(popup_id)
            .anchor(node_rect)
            .align(RectAlign::RIGHT_START)
            .align_alternatives(&[RectAlign::LEFT_START, RectAlign::BOTTOM_START, RectAlign::TOP_START])
            .gap(10.0)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .width(330.0)
            .show(|ui| settings(ui, id, p, accent, input));
    }

    // The grip: drag to resize (Shift keeps the shape).
    if hovered && z > 0.2 {
        let c = screen.right_bottom() - vec2(9.0, 9.0) * z;
        let r = Rect::from_center_size(c, Vec2::splat(18.0 * z));
        let resp = v.ui.interact(r, Id::unique(("iris-grip", id)), Sense::drag());
        let hot = resp.hovered() || resp.dragged();
        let col = if hot { accent } else { alpha(theme::TEXT, 0.7) };
        for k in 1..=3 {
            let d = k as f32 * 4.0 * z;
            v.painter.line_segment([c + vec2(6.0 * z - d, 6.0 * z), c + vec2(6.0 * z, 6.0 * z - d)], Stroke::new(1.4 * z, col));
        }
        if hot {
            v.ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
        }
        if resp.dragged() {
            let d = resp.drag_delta() / z;
            let (w, h) = (size.x + d.x, size.y + d.y);
            let (w, h) = if v.ui.input(|i| i.modifiers.shift) {
                let k = (w / size.x).max(h / size.y);
                (size.x * k, size.y * k)
            } else {
                (w, h)
            };
            p.data[data::W].set(w.clamp(iris::MIN_SIZE[0], iris::MAX_SIZE[0]));
            p.data[data::H].set(h.clamp(iris::MIN_SIZE[1], iris::MAX_SIZE[1]));
        }
    }
    let _ = gear_rect;

    if visible {
        v.ui.ctx().request_repaint();
    }
}

/// A small gear: a ring and eight teeth.
fn gear(p: &Painter, c: Pos2, z: f32, col: Color32) {
    let s = Stroke::new(1.4 * z, col);
    p.circle_stroke(c, 3.2 * z, s);
    for k in 0..8 {
        let a = k as f32 / 8.0 * TAU;
        let d = vec2(a.cos(), a.sin());
        p.line_segment([c + d * 4.6 * z, c + d * 6.6 * z], Stroke::new(1.8 * z, col));
    }
}

// ---------------------------------------------------------------------------
// Every frame: CYCLE and the window
// ---------------------------------------------------------------------------

/// What runs every frame for an Iris, drawn or not.
struct Ticker {
    scene: usize,
    on_scene: f32,
    last: f64,
    /// The window's own picture: the same sound, its own generative state.
    window: Option<State>,
    /// When the pointer last moved in the window (for its hint).
    moved: f64,
}

thread_local! {
    static TICKERS: RefCell<HashMap<NodeId, Ticker>> = RefCell::new(HashMap::new());
    static OPEN: RefCell<std::collections::HashSet<NodeId>> = RefCell::new(Default::default());
}

fn window_open(id: NodeId) -> bool {
    OPEN.with(|o| o.borrow().contains(&id))
}

fn set_window(id: NodeId, open: bool) {
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        if open {
            o.insert(id);
        } else {
            o.remove(&id);
        }
    });
}

/// Called by the canvas every frame for every Iris on the board, whether
/// it is drawn or not (off screen, collapsed, inside a collapsed module):
/// moves on to the next scene every CYCLE bars, and shows the node's
/// window while it is open.
pub fn tick(ctx: &egui::Context, id: NodeId, title: &str, accent: Color32, p: &ModuleParams, tempo: f32) {
    let now = ctx.input(|i| i.time);
    let open = window_open(id);
    let mut close = false;
    TICKERS.with(|t| {
        let mut tickers = t.borrow_mut();
        let tk = tickers.entry(id).or_insert(Ticker { scene: usize::MAX, on_scene: 0.0, last: now, window: None, moved: now });
        let dt = ((now - tk.last) as f32).clamp(0.0, 0.1);
        tk.last = now;
        let scene = scene_of(p);
        if tk.scene != scene {
            tk.scene = scene;
            tk.on_scene = 0.0;
        }
        tk.on_scene += dt;
        let bars = p.data[data::CYCLE].get().round();
        if bars >= 1.0 && tk.on_scene >= bars * 4.0 * 60.0 / tempo.max(20.0) {
            p.set_mode((scene + 1) % SCENES.len());
        }
        if !open {
            tk.window = None;
            return;
        }
        let seed = p.data[data::SEED].get();
        let st = tk.window.get_or_insert_with(|| State::new(seed));
        let moved = &mut tk.moved;
        let vid = egui::ViewportId::from_hash_of(("iris-window", id));
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("{title} · Iris"))
            .with_inner_size([960.0, 540.0])
            .with_min_inner_size([240.0, 160.0]);
        ctx.show_viewport_immediate(vid, builder, |ui, _class| {
            let rect = ui.max_rect();
            let pal = Pal::of(palette_index(p), accent);
            ui.painter().rect_filled(rect, 0.0, pal.bg);
            let now = ui.input(|i| i.time);
            let dt = if st.last.is_finite() { ((now - st.last) as f32).clamp(0.0, 0.1) } else { 1.0 / 60.0 };
            st.last = now;
            if st.seed != seed {
                *st = State::new(seed);
            }
            st.a.update(p, dt);
            let speed = p.get(SPEED);
            st.t += dt * speed * (0.7 + 0.6 * st.a.level);
            let painter = ui.painter().with_clip_rect(rect);
            let c = Ctx {
                p: &painter,
                r: rect,
                pal,
                k: (rect.height() / 300.0).sqrt().clamp(1.0, 3.0),
                t: st.t,
                dt: dt * speed,
                real_dt: dt,
                speed,
                detail: p.get(DETAIL),
                trail: p.get(TRAIL),
                a: &st.a,
            };
            draw_scene(scene_of(p), &c, &mut st.s, seed);

            let resp = ui.interact(rect, Id::unique(("iris-window-area", id)), Sense::click());
            let fullscreen = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if resp.double_clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                if fullscreen {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                } else {
                    close = true;
                }
            }
            if ui.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                *moved = now;
            }
            // A hint for a moment after the pointer moves; the cursor hides
            // while it rests in full screen.
            let shown = (now - *moved) as f32;
            if shown < 2.5 {
                let a = (2.5 - shown).min(1.0);
                let text = if fullscreen { "Double-click or Esc: leave full screen" } else { "Double-click: full screen  ·  Esc: close" };
                ui.painter().text(
                    rect.center_bottom() - vec2(0.0, 22.0),
                    Align2::CENTER_CENTER,
                    text,
                    theme::font(11.0),
                    alpha(pal.line(), 0.6 * a),
                );
            } else if fullscreen {
                ui.ctx().set_cursor_icon(CursorIcon::None);
            }
            ui.ctx().request_repaint();
        });
    });
    if close {
        set_window(id, false);
    }
}

// ---------------------------------------------------------------------------
// The settings popup
// ---------------------------------------------------------------------------

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).small().strong().color(theme::TEXT_FAINT));
}

fn settings(ui: &mut Ui, id: NodeId, p: &ModuleParams, accent: Color32, input: bool) {
    ui.spacing_mut().item_spacing = vec2(6.0, 5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Iris").strong().color(theme::TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let open = window_open(id);
            let label = if open { "Close window" } else { "Open in a window" };
            if ui.button(label).on_hover_text("The picture in a window of its own: double-click it for full screen").clicked() {
                set_window(id, !open);
            }
        });
    });

    heading(ui, "SCENE");
    let scene = scene_of(p);
    egui::Grid::new(ui.scope_id().with("scenes")).num_columns(3).spacing([5.0, 5.0]).show(ui, |ui| {
        for (k, _) in SCENES.iter().enumerate() {
            let label = RichText::new(scene_name(k)).size(11.0);
            let button = egui::Button::selectable(k == scene, label);
            if ui.add_sized([100.0, 24.0], button).clicked() {
                p.set_mode(k);
            }
            if k % 3 == 2 {
                ui.end_row();
            }
        }
    });

    heading(ui, "REACTS TO");
    let source = p.data[data::SOURCE].get().round();
    ui.horizontal(|ui| {
        for (value, label, tip) in [
            (iris::AUTO, "Auto", "The node patched into IN; the main mix while IN is free"),
            (iris::INPUT, "IN", "Only what is patched into IN"),
            (iris::MAIN, "Main mix", "Everything the board plays, even with IN patched"),
        ] {
            if ui.selectable_label(source == value, label).on_hover_text(tip).clicked() {
                p.data[data::SOURCE].set(value);
            }
        }
    });
    let now = match (iris::listens_to_input(p, input), input) {
        (true, true) => "Listening to the node on IN",
        (true, false) => "Nothing patched into IN",
        (false, true) => "Listening to the main mix (IN only goes on to THRU)",
        (false, false) => "Listening to the main mix",
    };
    ui.label(RichText::new(now).small().color(theme::TEXT_FAINT));

    heading(ui, "PALETTE");
    let current = palette_index(p);
    ui.horizontal(|ui| {
        for (k, pal) in PALETTES.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(33.0, 24.0), Sense::click());
            let colors = Pal::of(k, accent);
            let painter = ui.painter();
            painter.rect_filled(rect, 4.0, colors.bg);
            for (j, ink) in colors.inks.iter().enumerate() {
                let x = rect.left() + 5.0 + j as f32 * 5.0;
                painter.line_segment([pos2(x, rect.top() + 6.0), pos2(x + 4.0, rect.bottom() - 6.0)], Stroke::new(2.0, *ink));
            }
            let stroke = if k == current { Stroke::new(2.0, accent) } else { Stroke::new(1.0, theme::PANEL_BORDER) };
            painter.rect_stroke(rect, 4.0, stroke, StrokeKind::Inside);
            if resp.on_hover_text(pal.name).clicked() {
                p.data[data::PALETTE].set(k as f32);
            }
        }
    });

    heading(ui, "MOTION");
    let names = [(REACT, "React", 2.0), (SPEED, "Speed", 2.0), (DETAIL, "Detail", 1.0), (SMOOTH, "Smooth", 1.0), (TRAIL, "Trail", 1.0)];
    egui::Grid::new(ui.scope_id().with("knobs")).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        for (i, name, max) in names {
            ui.label(name);
            let mut val = p.get(i);
            let slider = egui::Slider::new(&mut val, 0.0..=max).custom_formatter(|v, _| format!("{:.0} %", v * 100.0));
            if ui.add(slider).changed() {
                p.set(i, val);
            }
            ui.end_row();
        }
    });

    heading(ui, "NEXT SCENE EVERY");
    let cycle = p.data[data::CYCLE].get().round() as u32;
    ui.horizontal(|ui| {
        for (bars, label) in [(0, "Never"), (1, "1 bar"), (2, "2"), (4, "4"), (8, "8"), (16, "16")] {
            if ui.selectable_label(cycle == bars, label).clicked() {
                p.data[data::CYCLE].set(bars as f32);
            }
        }
    });

    heading(ui, "ON THE BOARD");
    ui.horizontal(|ui| {
        let bare = frameless(p);
        if ui.selectable_label(!bare, "Frame").on_hover_text("The node's card: title bar, border and bezel").clicked() {
            p.data[data::FRAMELESS].set(0.0);
        }
        let tip = "Only the picture, as if stuck on the board; the scene's name, gear and grip still show under the pointer";
        if ui.selectable_label(bare, "No frame").on_hover_text(tip).clicked() {
            p.data[data::FRAMELESS].set(1.0);
        }
    });

    heading(ui, "SIZE");
    ui.horizontal(|ui| {
        for (label, w, h) in [("16:9", 16.0, 9.0), ("4:3", 4.0, 3.0), ("1:1", 1.0, 1.0), ("9:16", 9.0, 16.0)] {
            if ui.button(label).clicked() {
                set_aspect(p, w, h);
            }
        }
        if ui.button("Reset").clicked() {
            p.data[data::W].set(iris::DEFAULT_SIZE[0]);
            p.data[data::H].set(iris::DEFAULT_SIZE[1]);
        }
        let s = size(p);
        ui.label(RichText::new(format!("{:.0} × {:.0}", s.x, s.y)).small().color(theme::TEXT_FAINT));
    });
    ui.horizontal(|ui| {
        if ui.button("New seed").on_hover_text("Starts the generative scenes afresh").clicked() {
            let seed = (p.data[data::SEED].get() + 1.0) % 1000.0;
            p.data[data::SEED].set(seed);
        }
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("Drag the corner of the screen to resize (Shift keeps the shape); drag the picture to move the node.")
            .small()
            .color(theme::TEXT_FAINT),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::modules::ModuleKind;

    /// Writes `samples` into the ring as the processor would.
    fn feed(p: &ModuleParams, samples: impl Iterator<Item = f32>) {
        let mut w = p.viz[viz::WRITTEN].get() as u32;
        for s in samples {
            p.viz[viz::RING + w as usize % RING].set(s);
            w = (w + 1) % iris::COUNT;
        }
        p.viz[viz::WRITTEN].set(w as f32);
        p.viz[viz::LIVE].set(1.0);
        p.viz[viz::SR].set(48_000.0);
    }

    #[test]
    fn a_tone_lights_its_band_and_its_pitch() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let mut a = Analysis::new();
        let mut n = 0usize;
        for _ in 0..30 {
            feed(&p, (0..800).map(|_| {
                n += 1;
                0.05 * (TAU * 440.0 * n as f32 / 48_000.0).sin()
            }));
            a.update(&p, 1.0 / 60.0);
        }
        assert!((a.pitch - 440.0).abs() < 25.0, "pitch {}", a.pitch);
        let loudest = (0..BANDS).max_by(|&x, &y| a.bands[x].total_cmp(&a.bands[y])).unwrap();
        let f0 = 30.0 * (16_000.0f32 / 30.0).powf(loudest as f32 / BANDS as f32);
        let f1 = 30.0 * (16_000.0f32 / 30.0).powf((loudest + 1) as f32 / BANDS as f32);
        assert!(f0 <= 460.0 && f1 >= 420.0, "440 Hz lands in band {loudest} ({f0}–{f1} Hz)");
        assert!(a.level > 0.3, "level {}", a.level);
        assert!(a.bands[BANDS - 1] < 0.3, "nothing up high: {}", a.bands[BANDS - 1]);
    }

    #[test]
    fn hits_are_onsets_and_silence_is_not() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let mut a = Analysis::new();
        let mut onsets = 0;
        let mut n = 0usize;
        // Two seconds of a click every half second over quiet noise.
        let mut rng = Rng(7);
        for _ in 0..120 {
            feed(&p, (0..800).map(|_| {
                n += 1;
                let hit = n % 24_000 < 600;
                let env = if hit { 1.0 - (n % 24_000) as f32 / 600.0 } else { 0.0 };
                env * (rng.unit() * 2.0 - 1.0) * 0.9 + (rng.unit() - 0.5) * 0.002
            }));
            a.update(&p, 1.0 / 60.0);
            onsets += a.onset as usize;
        }
        assert!((3..=5).contains(&onsets), "{onsets} onsets for 4 hits");
        // Silence: no onsets, and everything falls back.
        let mut late = 0;
        for _ in 0..90 {
            feed(&p, (0..800).map(|_| 0.0));
            a.update(&p, 1.0 / 60.0);
            late += a.onset as usize;
        }
        assert_eq!(late, 0);
        assert!(a.level < 0.05 && a.beat < 0.05);
    }

    /// What each scene costs the UI per frame at full detail, drawn and
    /// tessellated: `cargo test --release iris_frame_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn iris_frame_cost() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let mut a = Analysis::new();
        let mut n = 0usize;
        let ctx = egui::Context::default();
        for (w, h) in [(460.0, 290.0), (1920.0, 1080.0)] {
            for scene in 0..SCENES.len() {
                let mut s = Scenes::new(1);
                let mut total = std::time::Duration::ZERO;
                let frames = 120;
                for f in 0..frames {
                    feed(&p, (0..800).map(|_| {
                        n += 1;
                        let t = n as f32 / 48_000.0;
                        (TAU * 110.0 * t).sin() * 0.5 + if n % 24_000 < 400 { 0.8 } else { 0.0 }
                    }));
                    a.update(&p, 1.0 / 60.0);
                    let start = std::time::Instant::now();
                    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                        let c = Ctx {
                            p: ui.painter(),
                            r: Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h)),
                            pal: Pal::of(0, Color32::from_rgb(96, 165, 238)),
                            k: (h / 300.0).sqrt(),
                            t: f as f32 / 60.0,
                            dt: 1.0 / 60.0,
                            real_dt: 1.0 / 60.0,
                            speed: 1.0,
                            detail: 1.0,
                            trail: 1.0,
                            a: &a,
                        };
                        draw_scene(scene, &c, &mut s, 1.0);
                    });
                    let _ = ctx.tessellate(std::mem::take(&mut out.shapes), 2.0);
                    out.textures_delta.clear();
                    total += start.elapsed();
                }
                eprintln!("{:>4}×{:<4} {:<13} {:>6.2} ms a frame", w, h, SCENES[scene], total.as_secs_f64() * 1000.0 / frames as f64);
            }
        }
    }

    #[test]
    fn frameless_fills_the_node_and_keeps_the_shape() {
        let p = ModuleParams::new(ModuleKind::Iris);
        assert!(!frameless(&p), "a new Iris has its frame");
        assert!(screen_local(size(&p), false).width() < size(&p).x);
        p.data[data::FRAMELESS].set(1.0);
        assert!(frameless(&p));
        assert_eq!(screen_local(size(&p), true), Rect::from_min_size(Pos2::ZERO, size(&p)));
        set_aspect(&p, 16.0, 9.0);
        let s = size(&p);
        assert!((s.x / s.y - 16.0 / 9.0).abs() < 0.01, "{s:?}");
    }

    #[test]
    fn every_attractor_shape_is_a_cloud_not_an_orbit() {
        // The fallback when the sound nudges a shape into a periodic orbit.
        for k in ATTRACTORS {
            let [a, b, c, d] = k;
            let (mut x, mut y) = (0.1f32, 0.1f32);
            let mut cells = std::collections::HashSet::new();
            for i in 0..3020 {
                let nx = (a * y).sin() + c * (a * x).cos();
                let ny = (b * x).sin() + d * (b * y).cos();
                x = nx;
                y = ny;
                if i >= 20 {
                    let cx = ((x / (1.0 + c.abs()) * 0.5 + 0.5) * 39.0) as i32;
                    let cy = ((y / (1.0 + d.abs()) * 0.5 + 0.5) * 39.0) as i32;
                    cells.insert((cx, cy));
                }
            }
            assert!(cells.len() >= 150, "{k:?} covers only {} cells", cells.len());
        }
    }

    #[test]
    fn every_scene_draws_on_every_palette_with_extremes() {
        let p = ModuleParams::new(ModuleKind::Iris);
        let mut a = Analysis::new();
        let mut n = 0usize;
        feed(&p, (0..4096).map(|_| {
            n += 1;
            (TAU * 110.0 * n as f32 / 48_000.0).sin() * 0.7
        }));
        a.update(&p, 1.0 / 60.0);
        a.onset = true;
        a.beat = 1.0;
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (pi, _) in PALETTES.iter().enumerate() {
                for detail in [0.0, 1.0] {
                    for scene in 0..SCENES.len() {
                        let mut s = Scenes::new(pi as u32);
                        for r in [Rect::from_min_size(pos2(0.0, 0.0), vec2(220.0, 140.0)), Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 300.0))] {
                            let c = Ctx {
                                p: ui.painter(),
                                r,
                                pal: Pal::of(pi, Color32::from_rgb(96, 165, 238)),
                                k: 1.0,
                                t: 3.0,
                                dt: 0.1,
                                real_dt: 0.1,
                                speed: 2.0,
                                detail,
                                trail: detail,
                                a: &a,
                            };
                            draw_scene(scene, &c, &mut s, 1.0);
                        }
                    }
                }
            }
        });
        out.textures_delta.clear();
    }
}
