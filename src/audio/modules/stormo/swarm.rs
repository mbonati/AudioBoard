//! The swarm's environment: where a voice's nodes are and how they move.
//!
//! Every node is a point in a square field (−1..1 on both axes) with a
//! velocity. Forces act on it each control tick: the FIELD (an orbit round
//! the attractor, a flock, a drifting current or a storm), SPIN round the
//! attractor, a TILT pulling the whole field one way, CHAOS (random kicks),
//! a little personal space between neighbours, and DAMPING. The BOUNDARY
//! keeps them in: walls they bounce off (EDGE), a round wall (CIRCLE) or a
//! field that wraps round (WRAP). A node that hits a wall, or crosses the
//! orbit's ring, is flagged (TRIGGER: HIT replucks it).

use std::f32::consts::{PI, TAU};

use super::super::Rng;

/// Most nodes a voice holds.
pub const MAX_NODES: usize = 48;

pub const FIELDS: [&str; 4] = ["ORBIT", "FLOCK", "DRIFT", "STORM"];
pub const BOUNDS: [&str; 3] = ["EDGE", "CIRCLE", "WRAP"];
pub const SHAPES: [&str; 6] = ["CLUSTER", "RING", "LINE", "GRID", "SPIRAL", "SCATTER"];

/// One node: where it is, how fast it goes, who it is.
#[derive(Clone, Copy, Default)]
pub struct Node {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Its personal-space push, worked out every few ticks.
    sx: f32,
    sy: f32,
    /// Distance from the attractor last tick (ring crossings).
    last_r: f32,
    /// A fixed random number of its own, 0..1.
    pub u: f32,
    /// A second one (detune).
    pub w: f32,
}

/// The environment, as the knobs set it.
#[derive(Clone, Copy)]
pub struct Field {
    pub mode: usize,
    pub gravity: f32,
    pub radius: f32,
    pub cohesion: f32,
    pub speed: f32,
    pub chaos: f32,
    pub damping: f32,
    pub spin: f32,
    pub tilt_angle: f32,
    pub tilt: f32,
    pub bound_on: bool,
    pub bound: usize,
    pub bounce: f32,
    pub size: f32,
    /// The attractor (the panel's ✕).
    pub ax: f32,
    pub ay: f32,
}

/// A voice's nodes.
pub struct Swarm {
    pub nodes: [Node; MAX_NODES],
    /// The flags of the last tick: node `i` hit something.
    pub hits: [bool; MAX_NODES],
    tick: u32,
    /// The storm's wandering attractor and the drift's time.
    t: f32,
}

impl Swarm {
    pub fn new(rng: &mut Rng) -> Self {
        let mut nodes = [Node::default(); MAX_NODES];
        for n in nodes.iter_mut() {
            n.u = rng.unit();
            n.w = rng.bipolar();
        }
        Self {
            nodes,
            hits: [false; MAX_NODES],
            tick: 0,
            t: rng.unit() * 100.0,
        }
    }

    /// Lays the nodes out in a SHAPE round the attractor, at rest but for a
    /// nudge along the orbit.
    pub fn spawn(&mut self, shape: usize, count: usize, f: &Field, rng: &mut Rng) {
        let count = count.clamp(1, MAX_NODES);
        let size = 0.15 + 0.75 * f.size;
        let cols = (count as f32).sqrt().ceil() as usize;
        for (i, n) in self.nodes.iter_mut().enumerate().take(count) {
            let k = i as f32 / count as f32;
            let (x, y) = match shape {
                // A tight blob.
                0 => {
                    let a = rng.unit() * TAU;
                    let r = rng.unit().sqrt() * size * 0.35;
                    (r * a.cos(), r * a.sin())
                }
                1 => {
                    let a = k * TAU;
                    (size * a.cos(), size * a.sin())
                }
                2 => ((k * 2.0 - 1.0 + 1.0 / count as f32) * size, 0.0),
                3 => {
                    let (c, r) = (i % cols, i / cols);
                    let step = 2.0 * size / cols.max(1) as f32;
                    (
                        -size + step * (c as f32 + 0.5),
                        -size + step * (r as f32 + 0.5),
                    )
                }
                4 => {
                    let a = k * TAU * 2.5;
                    let r = size * (0.1 + 0.9 * k);
                    (r * a.cos(), r * a.sin())
                }
                _ => (rng.bipolar() * size, rng.bipolar() * size),
            };
            n.x = (f.ax + x).clamp(-1.0, 1.0);
            n.y = (f.ay + y).clamp(-1.0, 1.0);
            // Along the orbit, the way SPIN turns.
            let (dx, dy) = (n.x - f.ax, n.y - f.ay);
            let s = 0.6 * f.spin;
            n.vx = -dy * s + rng.bipolar() * 0.02;
            n.vy = dx * s + rng.bipolar() * 0.02;
            n.last_r = (dx * dx + dy * dy).sqrt();
        }
    }

    /// One control tick of `dt` seconds for the first `count` nodes.
    pub fn step(&mut self, count: usize, f: &Field, dt: f32, rng: &mut Rng) {
        let count = count.clamp(1, MAX_NODES);
        let dt = dt * f.speed;
        self.t += dt;
        self.tick = self.tick.wrapping_add(1);
        self.hits = [false; MAX_NODES];

        // Personal space: every fourth tick (it is the costly part).
        let space = 0.06 + 0.22 * f.size;
        if self.tick.is_multiple_of(4) {
            for i in 0..count {
                let (mut sx, mut sy) = (0.0, 0.0);
                let (xi, yi) = (self.nodes[i].x, self.nodes[i].y);
                for j in 0..count {
                    if i == j {
                        continue;
                    }
                    let dx = xi - self.nodes[j].x;
                    let dy = yi - self.nodes[j].y;
                    let d2 = dx * dx + dy * dy;
                    if d2 < space * space && d2 > 1e-8 {
                        let d = d2.sqrt();
                        let push = (space - d) / (space * d);
                        sx += dx * push;
                        sy += dy * push;
                    }
                }
                self.nodes[i].sx = sx;
                self.nodes[i].sy = sy;
            }
        }

        // The flock's centre and heading.
        let (mut cx, mut cy, mut mvx, mut mvy) = (0.0, 0.0, 0.0, 0.0);
        for n in &self.nodes[..count] {
            cx += n.x;
            cy += n.y;
            mvx += n.vx;
            mvy += n.vy;
        }
        let inv = 1.0 / count as f32;
        let (cx, cy, mvx, mvy) = (cx * inv, cy * inv, mvx * inv, mvy * inv);

        // The storm drags the attractor round a Lissajous path.
        let (ax, ay) = if f.mode == 3 {
            let t = self.t * 0.35;
            (
                (f.ax + 0.55 * (t * 1.3).sin()).clamp(-0.9, 0.9),
                (f.ay + 0.55 * (t * 0.9 + 1.0).cos()).clamp(-0.9, 0.9),
            )
        } else {
            (f.ax, f.ay)
        };
        let radius = 0.08 + 0.8 * f.radius;
        let g = 0.5 + 9.0 * f.gravity;
        let spin = f.spin * 3.0;
        let (tx, ty) = (
            f.tilt * 2.5 * f.tilt_angle.cos(),
            f.tilt * 2.5 * f.tilt_angle.sin(),
        );
        let kick = f.chaos * f.chaos * 9.0 * dt.max(1e-6).sqrt() / dt.max(1e-6);
        let damp = (-dt * (0.15 + 5.0 * f.damping * f.damping)).exp();

        for (i, n) in self.nodes[..count].iter_mut().enumerate() {
            let (dx, dy) = (n.x - ax, n.y - ay);
            let r = (dx * dx + dy * dy).sqrt().max(1e-4);
            let (ux, uy) = (dx / r, dy / r);
            let (mut fx, mut fy) = (tx, ty);
            // Turning round the attractor.
            fx += -uy * spin * r.min(0.6) / 0.6;
            fy += ux * spin * r.min(0.6) / 0.6;
            match f.mode {
                // ORBIT: held on a ring of RADIUS.
                0 => {
                    let pull = -g * (r - radius);
                    fx += ux * pull;
                    fy += uy * pull;
                }
                // FLOCK: together, the same way, loosely round the attractor.
                1 => {
                    let c = 0.5 + 6.0 * f.cohesion;
                    fx += (cx - n.x) * c + (mvx - n.vx) * c * 0.6;
                    fy += (cy - n.y) * c + (mvy - n.vy) * c * 0.6;
                    let pull = -g * 0.35 * (r - radius).max(0.0);
                    fx += ux * pull;
                    fy += uy * pull;
                }
                // DRIFT: carried by a slow current, gently held.
                2 => {
                    let t = self.t * 0.4;
                    let flow = 0.6 + 1.6 * f.cohesion;
                    fx += flow * (PI * 1.3 * n.y + t).sin() - g * 0.12 * dx;
                    fy += flow * (PI * 1.1 * n.x - t * 0.8).cos() - g * 0.12 * dy;
                }
                // STORM: chasing a wandering attractor, hard.
                _ => {
                    let pull = -g * 1.4 * (r - radius * 0.5);
                    fx += ux * pull + (cx - n.x) * f.cohesion * 2.0;
                    fy += uy * pull + (cy - n.y) * f.cohesion * 2.0;
                }
            }
            // Personal space, chaos.
            let push = 1.5 + 3.0 * (1.0 - f.cohesion);
            fx += n.sx * push + rng.bipolar() * kick;
            fy += n.sy * push + rng.bipolar() * kick;
            n.vx = (n.vx + fx * dt) * damp;
            n.vy = (n.vy + fy * dt) * damp;
            let v2 = n.vx * n.vx + n.vy * n.vy;
            if v2 > 16.0 {
                let s = 4.0 / v2.sqrt();
                n.vx *= s;
                n.vy *= s;
            }
            n.x += n.vx * dt;
            n.y += n.vy * dt;

            // The boundary.
            let mut hit = false;
            if f.bound_on {
                let e = f.bounce;
                match f.bound {
                    0 => {
                        if n.x.abs() > 1.0 {
                            n.x = n.x.signum() * (2.0 - n.x.abs()).max(0.0);
                            n.vx = -n.vx * e;
                            hit = true;
                        }
                        if n.y.abs() > 1.0 {
                            n.y = n.y.signum() * (2.0 - n.y.abs()).max(0.0);
                            n.vy = -n.vy * e;
                            hit = true;
                        }
                    }
                    1 => {
                        let d = (n.x * n.x + n.y * n.y).sqrt();
                        if d > 1.0 {
                            let (nx, ny) = (n.x / d, n.y / d);
                            let vn = n.vx * nx + n.vy * ny;
                            if vn > 0.0 {
                                n.vx -= (1.0 + e) * vn * nx;
                                n.vy -= (1.0 + e) * vn * ny;
                            }
                            n.x = nx * (2.0 - d).max(0.0);
                            n.y = ny * (2.0 - d).max(0.0);
                            hit = true;
                        }
                    }
                    _ => {
                        if n.x.abs() > 1.0 {
                            n.x -= 2.0 * n.x.signum();
                            hit = true;
                        }
                        if n.y.abs() > 1.0 {
                            n.y -= 2.0 * n.y.signum();
                            hit = true;
                        }
                    }
                }
            } else {
                // No wall: a soft pull back from far away.
                let d = (n.x * n.x + n.y * n.y).sqrt();
                if d > 1.3 {
                    let k = (d - 1.3) * 6.0 * dt;
                    n.vx -= n.x / d * k * 10.0;
                    n.vy -= n.y / d * k * 10.0;
                }
                n.x = n.x.clamp(-2.0, 2.0);
                n.y = n.y.clamp(-2.0, 2.0);
            }
            // Crossing the orbit's ring, outwards.
            let r_now = ((n.x - ax).powi(2) + (n.y - ay).powi(2)).sqrt();
            if n.last_r < radius && r_now >= radius {
                hit = true;
            }
            n.last_r = r_now;
            self.hits[i] = hit;
        }
    }

    /// The attractor as it is now (it wanders in a STORM).
    pub fn attractor(&self, f: &Field) -> (f32, f32) {
        if f.mode == 3 {
            let t = self.t * 0.35;
            (
                (f.ax + 0.55 * (t * 1.3).sin()).clamp(-0.9, 0.9),
                (f.ay + 0.55 * (t * 0.9 + 1.0).cos()).clamp(-0.9, 0.9),
            )
        } else {
            (f.ax, f.ay)
        }
    }
}
