//! The hexagon: six macro sliders reaching out from a centre, and a magnet
//! cursor that moves their nodes. ATTRACT pulls a node along its slider
//! towards the cursor, REPEL pushes it away, within RANGE and as hard as
//! STRENGTH says. A node the magnet lets go stays put for HOLD, then
//! returns to its start over RELEASE. The magnet works all the time, only
//! while clicked, or only while not clicked.
//!
//! The cursor is the mouse over the panel, or a path while one plays: drawn
//! (points joined by straight or bent lines, each "clicked" or not) or
//! recorded from the mouse. The timeline sets how long one pass takes (TIME
//! × MULTIPLIER, synced or free, or the recording's own length), whether
//! the cursor moves evenly or at the recorded speed, the direction, looping
//! and how the transport restarts it.
//!
//! Hexagon space: x and y in −1…1 from the centre, y down.

use super::{
    Clock, CUR_HOLD, CUR_RANGE, CUR_RELEASE, CUR_STRENGTH, DIVS, HOLD_FOREVER, MAX_POINTS, MAX_REC, MULTIPLIERS, REC_HZ,
    AR_ATTACK, AR_GAIN, AR_RELEASE, AR_THRESHOLD, TIME_DIVS, TIME_S, TL_MULT, TL_OFFSET, TL_TIME, choice, data, log_range,
    on, pick, viz,
};
use crate::audio::modules::ModuleParams;
use crate::audio::params::gain_to_db;

pub const SLIDERS: usize = 6;
/// Where the sliders start and end, as fractions of the hexagon's radius.
pub const INNER: f32 = 0.2;
pub const OUTER: f32 = 0.92;

pub const SELECT: usize = 0;
pub const ATTRACT: usize = 1;
pub const REPEL: usize = 2;
pub const MAGNET_MODES: [(&str, &str); 3] = [
    ("SELECT", "Select: drag the nodes to set where the sliders rest"),
    ("ATTRACT", "Attract: the magnet pulls the nodes towards the cursor"),
    ("REPEL", "Repel: the magnet pushes the nodes away from the cursor"),
];
pub const CONSTANT: usize = 0;
pub const CLICK_ON: usize = 1;
pub const CLICK_OFF: usize = 2;
pub const ACTIVATIONS: [(&str, &str); 3] = [
    ("CONSTANT", "The magnet works whenever the cursor is over the hexagon"),
    ("CLICK ON", "The magnet works while the mouse is clicked"),
    ("CLICK OFF", "The magnet works while the mouse is not clicked"),
];

/// Path kinds, and timeline settings.
pub const DRAWN: usize = 0;
pub const RECORDED: usize = 1;
pub const FIXED: usize = 0;
pub const FREE: usize = 1;
pub const EVEN: usize = 0;
pub const AS_RECORDED: usize = 1;
/// (Retrigger 0 is off: the path runs on its own.)
pub const RETRIG_ON: usize = 1;
pub const RETRIG_SYNC: usize = 2;
pub const FORWARD: usize = 0;
pub const BACKWARD: usize = 1;
pub const PENDULUM: usize = 2;

/// How fast an attracted node closes in at full strength (per second), and
/// how fast a repelled one is pushed (slider lengths per second).
const ATTRACT_RATE: f32 = 14.0;
const REPEL_RATE: f32 = 2.5;

/// Direction of slider `i` (the first points up).
pub fn spoke(i: usize) -> (f32, f32) {
    let a = (-90.0 + 60.0 * i as f32).to_radians();
    (a.cos(), a.sin())
}

/// Where slider `i`'s node sits at value `v`.
pub fn node_pos(i: usize, v: f32) -> (f32, f32) {
    let (dx, dy) = spoke(i);
    let r = INNER + (OUTER - INNER) * v.clamp(0.0, 1.0);
    (dx * r, dy * r)
}

/// The value on slider `i` nearest a point.
pub fn project(i: usize, (x, y): (f32, f32)) -> f32 {
    let (dx, dy) = spoke(i);
    ((x * dx + y * dy - INNER) / (OUTER - INNER)).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct PathPoint {
    pub x: f32,
    pub y: f32,
    /// How far the line to the next point bows sideways (−1…1).
    pub bend: f32,
    /// The mouse counts as clicked along the line to the next point.
    pub clicked: bool,
}

pub fn drawn_count(p: &ModuleParams) -> usize {
    (p.data[data::DRAWN_COUNT].get().max(0.0) as usize).min(MAX_POINTS)
}

/// A drawn path closes on itself (circles do).
pub fn drawn_closed(p: &ModuleParams) -> bool {
    on(p, data::DRAWN_CLOSED) && drawn_count(p) >= 3
}

pub fn drawn_point(p: &ModuleParams, i: usize) -> PathPoint {
    let at = data::DRAWN + i * 4;
    PathPoint {
        x: p.data[at].get(),
        y: p.data[at + 1].get(),
        bend: p.data[at + 2].get(),
        clicked: p.data[at + 3].get() > 0.5,
    }
}

pub fn set_drawn_point(p: &ModuleParams, i: usize, pt: PathPoint) {
    let at = data::DRAWN + i * 4;
    p.data[at].set(pt.x.clamp(-1.0, 1.0));
    p.data[at + 1].set(pt.y.clamp(-1.0, 1.0));
    p.data[at + 2].set(pt.bend.clamp(-1.0, 1.0));
    p.data[at + 3].set(if pt.clicked { 1.0 } else { 0.0 });
}

/// Replaces the drawn path.
pub fn set_drawn(p: &ModuleParams, points: &[PathPoint], closed: bool) {
    let n = points.len().min(MAX_POINTS);
    for (i, pt) in points[..n].iter().enumerate() {
        set_drawn_point(p, i, *pt);
    }
    p.data[data::DRAWN_CLOSED].set(if closed { 1.0 } else { 0.0 });
    p.data[data::DRAWN_COUNT].set(n as f32);
}

pub fn rec_count(p: &ModuleParams) -> usize {
    (p.data[data::REC_COUNT].get().max(0.0) as usize).min(MAX_REC)
}

pub fn rec_point(p: &ModuleParams, i: usize) -> PathPoint {
    let at = data::REC + i * 3;
    PathPoint {
        x: p.data[at].get(),
        y: p.data[at + 1].get(),
        bend: 0.0,
        clicked: p.data[at + 2].get() > 0.5,
    }
}

pub fn set_rec_point(p: &ModuleParams, i: usize, pt: PathPoint) {
    let at = data::REC + i * 3;
    p.data[at].set(pt.x.clamp(-1.0, 1.0));
    p.data[at + 1].set(pt.y.clamp(-1.0, 1.0));
    p.data[at + 2].set(if pt.clicked { 1.0 } else { 0.0 });
}

/// The played part of the recording, as sample indices (START…END).
pub fn rec_span(p: &ModuleParams) -> (f32, f32) {
    let last = rec_count(p).saturating_sub(1) as f32;
    let a = p.data[data::REC_START].get().clamp(0.0, 1.0);
    let b = p.data[data::REC_END].get().clamp(0.0, 1.0);
    (a.min(b) * last, a.max(b) * last)
}

/// The point at `t` (0…1) along the line from `a` to `b`: a quadratic
/// curve whose control point sits off the middle by `a.bend`.
pub fn segment(a: PathPoint, b: PathPoint, t: f32) -> (f32, f32) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (cx, cy) = ((a.x + b.x) * 0.5 - dy * a.bend, (a.y + b.y) * 0.5 + dx * a.bend);
    let u = 1.0 - t;
    (u * u * a.x + 2.0 * u * t * cx + t * t * b.x, u * u * a.y + 2.0 * u * t * cy + t * t * b.y)
}

fn segment_length(a: PathPoint, b: PathPoint) -> f32 {
    let mut len = 0.0;
    let mut prev = (a.x, a.y);
    for k in 1..=8 {
        let q = segment(a, b, k as f32 / 8.0);
        len += (q.0 - prev.0).hypot(q.1 - prev.1);
        prev = q;
    }
    len
}

/// Scratch space for walking a path by its length.
pub struct PathCache {
    lengths: Box<[f32]>,
    points: [PathPoint; MAX_POINTS],
}

impl Default for PathCache {
    fn default() -> Self {
        Self {
            lengths: vec![0.0; MAX_REC + 1].into_boxed_slice(),
            points: [PathPoint::default(); MAX_POINTS],
        }
    }
}

impl PathCache {
    /// The path's cursor at `u` (0…1 along it): where, and whether the
    /// mouse counts as clicked there. None without a path.
    pub fn at(&mut self, p: &ModuleParams, u: f32) -> Option<PathPoint> {
        let u = u.clamp(0.0, 1.0);
        if choice(p, data::PATH_KIND, 2) == RECORDED {
            return self.recorded_at(p, u);
        }
        let n = drawn_count(p);
        if n < 2 {
            return None;
        }
        let closed = drawn_closed(p);
        let segments = if closed { n } else { n - 1 };
        for i in 0..n {
            self.points[i] = drawn_point(p, i);
        }
        let mut total = 0.0;
        for s in 0..segments {
            self.lengths[s] = total;
            total += segment_length(self.points[s], self.points[(s + 1) % n]);
        }
        self.lengths[segments] = total;
        let d = u * total;
        let s = (0..segments).rev().find(|&s| self.lengths[s] <= d).unwrap_or(0);
        let span = (self.lengths[s + 1] - self.lengths[s]).max(1e-6);
        let a = self.points[s];
        let (x, y) = segment(a, self.points[(s + 1) % n], ((d - self.lengths[s]) / span).clamp(0.0, 1.0));
        Some(PathPoint { x, y, bend: 0.0, clicked: a.clicked })
    }

    fn recorded_at(&mut self, p: &ModuleParams, u: f32) -> Option<PathPoint> {
        if rec_count(p) < 2 {
            return None;
        }
        let (a, b) = rec_span(p);
        let index = if choice(p, data::SPEED, 2) == AS_RECORDED {
            a + (b - a) * u
        } else {
            // Even speed: walk the samples by distance.
            let (first, last) = (a.floor() as usize, (b.ceil() as usize).max(a.floor() as usize + 1));
            let mut total = 0.0;
            let mut prev = rec_point(p, first);
            self.lengths[0] = 0.0;
            for k in first + 1..=last {
                let q = rec_point(p, k);
                total += (q.x - prev.x).hypot(q.y - prev.y);
                self.lengths[k - first] = total;
                prev = q;
            }
            if total < 1e-6 {
                a + (b - a) * u
            } else {
                let d = u * total;
                let lens = &self.lengths[..=last - first];
                let k = lens.partition_point(|&l| l <= d).clamp(1, lens.len() - 1);
                let f = (d - lens[k - 1]) / (lens[k] - lens[k - 1]).max(1e-6);
                (first + k - 1) as f32 + f.clamp(0.0, 1.0)
            }
        };
        let last = rec_count(p) - 1;
        let i = (index.max(0.0) as usize).min(last);
        let j = (i + 1).min(last);
        let f = index - i as f32;
        let (pa, pb) = (rec_point(p, i), rec_point(p, j));
        Some(PathPoint {
            x: pa.x + (pb.x - pa.x) * f,
            y: pa.y + (pb.y - pa.y) * f,
            bend: 0.0,
            clicked: pa.clicked,
        })
    }
}

/// One pass of the path, in seconds.
pub fn duration(p: &ModuleParams, seconds_per_beat: f64) -> f64 {
    if choice(p, data::DURATION, 2) == FREE && choice(p, data::PATH_KIND, 2) == RECORDED && rec_count(p) >= 2 {
        let (a, b) = rec_span(p);
        return ((b - a) / REC_HZ).max(0.05) as f64;
    }
    let time = p.get(TL_TIME);
    let base = if on(p, data::TIME_SYNC) {
        DIVS[pick(time, TIME_DIVS)].1 * seconds_per_beat
    } else {
        log_range(time, TIME_S) as f64
    };
    let mult = MULTIPLIERS[(p.get(TL_MULT).round().max(0.0) as usize).min(MULTIPLIERS.len() - 1)].1;
    (base * mult).max(0.05)
}

#[derive(Clone, Copy)]
struct Cursor {
    x: f32,
    y: f32,
    clicked: bool,
}

pub struct Motion {
    /// The sliders' live values (0…1).
    pub values: [f32; SLIDERS],
    /// Seconds since the magnet last moved each node.
    idle: [f32; SLIDERS],
    /// The input-level follower (0…1).
    pub env: f32,
    /// Seconds the path has been playing.
    time: f64,
    playing: bool,
    /// The panel's frame counter, and for how long it hasn't moved.
    stamp: f32,
    stale: f32,
    cache: PathCache,
}

impl Motion {
    pub fn new(p: &ModuleParams) -> Self {
        Self {
            values: std::array::from_fn(|i| p.data[data::SLIDER_START + i].get().clamp(0.0, 1.0)),
            idle: [0.0; SLIDERS],
            env: 0.0,
            time: 0.0,
            playing: false,
            stamp: 0.0,
            stale: 1.0,
            cache: PathCache::default(),
        }
    }

    /// One block of `dt` seconds; `level` is the input's peak.
    pub fn update(&mut self, p: &ModuleParams, clock: &Clock, level: f32, dt: f32) {
        self.follow(p, level, dt);
        let cursor = match self.path(p, clock, dt) {
            Some(c) => Some(c),
            None => self.mouse(p, dt),
        };
        let mode = choice(p, data::MAGNET, 3);
        let activation = choice(p, data::ACTIVATION, 3);
        let pulling = cursor.filter(|c| {
            mode != SELECT
                && match activation {
                    CLICK_ON => c.clicked,
                    CLICK_OFF => !c.clicked,
                    _ => true,
                }
        });
        let (range, strength) = (p.get(CUR_RANGE).max(0.01), p.get(CUR_STRENGTH));
        let (hold, release) = (p.get(CUR_HOLD), p.get(CUR_RELEASE).max(0.001));
        for i in 0..SLIDERS {
            let start = p.data[data::SLIDER_START + i].get().clamp(0.0, 1.0);
            let v = &mut self.values[i];
            if mode == SELECT {
                *v = start;
                self.idle[i] = 0.0;
                continue;
            }
            let mut touched = false;
            if let Some(c) = pulling {
                let (nx, ny) = node_pos(i, *v);
                let d = (c.x - nx).hypot(c.y - ny);
                let w = strength * (1.0 - d / range).max(0.0).powi(2);
                if w > 1e-4 {
                    touched = true;
                    let t = project(i, (c.x, c.y));
                    match mode {
                        ATTRACT => *v += (t - *v) * (1.0 - (-w * ATTRACT_RATE * dt).exp()),
                        REPEL => {
                            let away = if *v < t { -1.0 } else { 1.0 };
                            *v = (*v + away * w * REPEL_RATE * dt).clamp(0.0, 1.0);
                        }
                        _ => {}
                    }
                }
            }
            if touched {
                self.idle[i] = 0.0;
            } else {
                self.idle[i] += dt;
                if hold < HOLD_FOREVER && self.idle[i] > hold {
                    *v += (start - *v) * (1.0 - (-dt * 4.6 / release).exp());
                }
            }
        }

        let viz = &p.viz;
        for (i, v) in self.values.iter().enumerate() {
            viz[viz::SLIDERS + i].set(*v);
        }
        viz[viz::AUDIO_ENV].set(self.env);
        match cursor {
            Some(c) => {
                viz[viz::CURSOR_X].set(c.x);
                viz[viz::CURSOR_Y].set(c.y);
                viz[viz::CURSOR_STATE].set(if pulling.is_some() { 2.0 } else { 1.0 });
            }
            None => viz[viz::CURSOR_STATE].set(0.0),
        }
    }

    /// The input-level follower: 0 at THRESHOLD, 1 at full scale (after
    /// GAIN), with its own attack and release.
    fn follow(&mut self, p: &ModuleParams, level: f32, dt: f32) {
        let threshold = gain_to_db(p.get(AR_THRESHOLD)).min(-1.0);
        let db = gain_to_db(level * p.get(AR_GAIN));
        let target = ((db - threshold) / -threshold).clamp(0.0, 1.0);
        let tau = if target > self.env { p.get(AR_ATTACK) } else { p.get(AR_RELEASE) };
        self.env += (target - self.env) * (1.0 - (-dt / tau.max(1e-4)).exp());
    }

    /// The mouse over the panel's hexagon, if the panel is drawing it.
    fn mouse(&mut self, p: &ModuleParams, dt: f32) -> Option<Cursor> {
        let stamp = p.viz[viz::UI_STAMP].get();
        if stamp != self.stamp {
            self.stamp = stamp;
            self.stale = 0.0;
        } else {
            self.stale += dt;
        }
        (self.stale < 0.25 && p.viz[viz::UI_IN].get() > 0.5).then(|| Cursor {
            x: p.viz[viz::UI_X].get(),
            y: p.viz[viz::UI_Y].get(),
            clicked: p.viz[viz::UI_DOWN].get() > 0.5,
        })
    }

    /// The path's cursor while it plays.
    fn path(&mut self, p: &ModuleParams, clock: &Clock, dt: f32) -> Option<Cursor> {
        if !on(p, data::PATH_PLAYING) {
            self.playing = false;
            p.viz[viz::PATH_POS].set(-1.0);
            p.viz[viz::PATH_DONE].set(0.0);
            return None;
        }
        if !self.playing {
            // Play starts from the top.
            self.playing = true;
            self.time = 0.0;
        }
        let retrigger = choice(p, data::RETRIGGER, 3);
        if retrigger == RETRIG_ON && clock.started {
            self.time = 0.0;
        }
        let seconds = duration(p, clock.seconds_per_beat);
        let passes = if retrigger == RETRIG_SYNC && clock.playing {
            clock.beat0 * clock.seconds_per_beat / seconds
        } else {
            self.time / seconds
        } + p.get(TL_OFFSET) as f64;
        self.time += dt as f64;

        let direction = choice(p, data::DIRECTION, 3);
        let lap = if direction == PENDULUM { 2.0 } else { 1.0 };
        let done = !on(p, data::LOOP) && passes >= lap;
        let phase = if done { lap - 1e-6 } else { passes.rem_euclid(lap) } as f32;
        let u = match direction {
            BACKWARD => 1.0 - phase,
            PENDULUM if phase > 1.0 => 2.0 - phase,
            _ => phase,
        };
        p.viz[viz::PATH_POS].set(u);
        p.viz[viz::PATH_DONE].set(if done { 1.0 } else { 0.0 });
        self.cache.at(p, u).map(|c| Cursor { x: c.x, y: c.y, clicked: c.clicked })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::modules::ModuleKind;
    use crate::audio::modules::frakture::{CIRCLE_R, CUR_HOLD, CUR_RELEASE, CUR_STRENGTH};

    const DT: f32 = 64.0 / 48_000.0;

    fn clock() -> Clock {
        Clock { beat0: 0.0, bps: 2.0 / 48_000.0, samples_per_beat: 24_000.0, seconds_per_beat: 0.5, playing: false, started: false }
    }

    /// Runs `seconds` of blocks, moving the mouse to `at` (None: away).
    fn run(m: &mut Motion, p: &ModuleParams, at: Option<(f32, f32)>, seconds: f32) {
        for _ in 0..(seconds / DT) as usize {
            p.viz[viz::UI_STAMP].set(p.viz[viz::UI_STAMP].get() + 1.0);
            p.viz[viz::UI_IN].set(if at.is_some() { 1.0 } else { 0.0 });
            if let Some((x, y)) = at {
                p.viz[viz::UI_X].set(x);
                p.viz[viz::UI_Y].set(y);
            }
            m.update(p, &clock(), 0.0, DT);
        }
    }

    #[test]
    fn magnet_pulls_holds_and_releases() {
        let p = ModuleParams::new(ModuleKind::Frakture);
        p.set(CUR_STRENGTH, 1.0);
        p.set(CUR_HOLD, 0.5);
        p.set(CUR_RELEASE, 0.5);
        let mut m = Motion::new(&p);
        // The cursor just beyond slider 1's node, towards its end.
        run(&mut m, &p, Some(node_pos(0, 0.75)), 1.0);
        assert!(m.values[0] > 0.7, "attracted: {}", m.values[0]);
        // The magnet lets go: the node holds, then returns to its start.
        let pulled = m.values[0];
        run(&mut m, &p, None, 0.4);
        assert!((m.values[0] - pulled).abs() < 1e-6, "held");
        run(&mut m, &p, None, 3.0);
        assert!((m.values[0] - 0.5).abs() < 0.02, "released: {}", m.values[0]);
        // Repel pushes the node away from the cursor.
        p.data[data::MAGNET].set(REPEL as f32);
        run(&mut m, &p, Some(node_pos(0, 0.6)), 0.3);
        assert!(m.values[0] < 0.45, "repelled: {}", m.values[0]);
        // Click on: nothing moves until the mouse is clicked.
        p.data[data::MAGNET].set(ATTRACT as f32);
        p.data[data::ACTIVATION].set(CLICK_ON as f32);
        let before = m.values[3];
        run(&mut m, &p, Some(node_pos(3, 1.0)), 0.3);
        assert!((m.values[3] - before).abs() < 1e-6);
    }

    #[test]
    fn the_path_drives_the_cursor() {
        let p = ModuleParams::new(ModuleKind::Frakture);
        p.data[data::PATH_PLAYING].set(1.0);
        let mut m = Motion::new(&p);
        let mut seen = Vec::new();
        for _ in 0..200 {
            m.update(&p, &clock(), 0.0, DT);
            assert_eq!(p.viz[viz::CURSOR_STATE].get(), 2.0, "the magnet follows the path");
            let (x, y) = (p.viz[viz::CURSOR_X].get(), p.viz[viz::CURSOR_Y].get());
            assert!((x.hypot(y) - CIRCLE_R).abs() < 0.05, "on the circle: {x}, {y}");
            seen.push(p.viz[viz::PATH_POS].get());
        }
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "moves forwards");
        // Recorded path, played at its own speed.
        p.data[data::PATH_KIND].set(RECORDED as f32);
        for i in 0..50 {
            set_rec_point(&p, i, PathPoint { x: -0.5 + i as f32 / 49.0, y: 0.0, bend: 0.0, clicked: true });
        }
        p.data[data::REC_COUNT].set(50.0);
        let mut cache = PathCache::default();
        let mid = cache.at(&p, 0.5).expect("a path");
        assert!(mid.x.abs() < 0.02 && mid.y == 0.0);
        assert!(duration(&p, 0.5) > 0.0);
    }

    #[test]
    fn sliders_and_projection_agree() {
        for i in 0..SLIDERS {
            for v in [0.0, 0.3, 1.0] {
                assert!((project(i, node_pos(i, v)) - v).abs() < 1e-5);
            }
        }
    }
}
