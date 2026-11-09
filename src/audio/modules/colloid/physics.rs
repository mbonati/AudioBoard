//! The tank: molecules floating in a liquid and atoms falling towards them.
//!
//! The tank is `ASPECT` wide and 1 tall, y growing downwards. Molecules
//! float at a height set by their note (low notes are heavy and sink), held
//! apart and loosely together by a Lennard-Jones-like pull, wandering with
//! the liquid. Atoms are pulled by every molecule (gravity, softened inside
//! a molecule and fading past RANGE), slowed by the liquid's viscosity,
//! shaken by heat and carried by its currents. When an atom strikes a
//! molecule it bounces (BOUNCE is the restitution), sets the molecule
//! ringing and spinning, and picks up a charge that pushes it away from
//! that molecule for a while (CHARGE): the strike is a note.

use super::super::Rng;

pub const MAX_MOLECULES: usize = 16;
pub const MAX_ATOMS: usize = 48;
/// The tank's width (its height is 1).
pub const ASPECT: f32 = 2.6;
/// Fastest an atom moves, in tank heights per second.
const MAX_SPEED: f32 = 3.2;
/// Slowest strike that sounds.
pub const MIN_STRIKE: f32 = 0.12;
/// A molecule's ring below this has died away.
pub const SILENT: f32 = 0.04;

/// The liquid's currents: a stream function's curl, so they swirl without
/// gathering anywhere. The panel draws the same field.
pub fn current(x: f32, y: f32, t: f32) -> (f32, f32) {
    // ψ = sin(a)·sin(b) + 0.5·sin(c), with a, b, c linear in x, y, t.
    let a = 1.9 * x + 0.23 * t;
    let b = 2.6 * y - 0.17 * t;
    let c = 2.9 * x - 2.1 * y + 0.31 * t;
    let dpsi_dx = 1.9 * a.cos() * b.sin() + 0.5 * 2.9 * c.cos();
    let dpsi_dy = 2.6 * a.sin() * b.cos() - 0.5 * 2.1 * c.cos();
    (dpsi_dy * 0.5, -dpsi_dx * 0.5)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Molecule {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Radius, growing towards `size` (a new molecule condenses).
    pub r: f32,
    pub size: f32,
    pub mass: f32,
    /// Its turn, and how fast it turns (strikes off-centre spin it).
    pub angle: f32,
    pub spin: f32,
    /// Its own slow turn, which the spin settles back to.
    pub idle_spin: f32,
    /// How hard it rings, 0…1: the note sounds while it rings.
    pub ring: f32,
    /// Seconds a ring takes to fall by e.
    pub decay: f32,
    /// The height its note makes it float at, and the place across the
    /// tank it drifts round (loosely: clusters pull it off).
    pub home: f32,
    pub home_x: f32,
    pub note: u8,
    pub role: u8,
    /// Phases of its wandering.
    pub wander: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Atom {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// 1 just after a strike, falling to 0: pushed from `from` meanwhile.
    pub charge: f32,
    pub from: u8,
    /// Seconds since it appeared.
    pub age: f32,
}

/// The physics, from the knobs.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Gravity's strength.
    pub pull: f32,
    /// How far it reaches.
    pub reach: f32,
    /// Drag on the atoms, per second.
    pub drag: f32,
    /// Brownian kicks.
    pub heat: f32,
    /// How strongly the currents carry.
    pub flow: f32,
    /// Restitution of a strike.
    pub bounce: f32,
    /// Seconds an atom stays charged after a strike (0: never).
    pub charge: f32,
    /// How much the molecules wander.
    pub drift: f32,
    /// An atom's mass and radius.
    pub atom_mass: f32,
    pub atom_r: f32,
    /// How long a strike rings, in seconds (for the loudest one).
    pub length: f32,
    /// Viscosity 0…1, which also damps the rings.
    pub viscosity: f32,
}

impl Settings {
    /// From the knobs' values, each 0…1 (`length` in seconds).
    #[allow(clippy::too_many_arguments)]
    pub fn new(attract: f32, range: f32, viscosity: f32, heat: f32, flow: f32, bounce: f32, charge: f32, drift: f32, mass: f32, length: f32) -> Self {
        let c = |v: f32| v.clamp(0.0, 1.0);
        Self {
            pull: 1.1 * c(attract).powf(1.6),
            reach: 0.08 + 1.5 * c(range) * c(range),
            drag: 4.5 * c(viscosity) * c(viscosity),
            heat: 1.6 * c(heat) * c(heat),
            flow: 1.4 * c(flow) * c(flow),
            bounce: 0.15 + c(bounce),
            charge: 1.4 * c(charge),
            drift: c(drift),
            atom_mass: 0.3 + 1.7 * c(mass) * c(mass),
            atom_r: 0.008 + 0.01 * c(mass),
            length: length.max(0.01),
            viscosity: c(viscosity),
        }
    }
}

/// An atom striking a molecule.
#[derive(Clone, Copy, Debug, Default)]
pub struct Strike {
    pub molecule: usize,
    pub atom: usize,
    /// Where on the molecule's surface.
    pub x: f32,
    pub y: f32,
    /// How hard it was, 0…1 (speed and mass).
    pub force: f32,
}

/// Strikes a step can report, at most.
pub const MAX_STRIKES: usize = 32;

pub struct Tank {
    pub molecules: [Molecule; MAX_MOLECULES],
    pub molecule_count: usize,
    pub atoms: [Atom; MAX_ATOMS],
    pub atom_count: usize,
    /// Seconds since the tank was made.
    pub time: f32,
}

/// A normal-ish random value (mean 0, deviation about 1).
fn gauss(rng: &mut Rng) -> f32 {
    (rng.unit() + rng.unit() + rng.unit() + rng.unit() - 2.0) * 1.732
}

impl Tank {
    pub fn new() -> Self {
        Self {
            molecules: [Molecule::default(); MAX_MOLECULES],
            molecule_count: 0,
            atoms: [Atom::default(); MAX_ATOMS],
            atom_count: 0,
            time: 0.0,
        }
    }

    /// Scatters every molecule anew round its home height.
    pub fn scatter(&mut self, rng: &mut Rng) {
        for m in &mut self.molecules {
            m.home_x = 0.2 + rng.unit() * (ASPECT - 0.4);
            m.x = (m.home_x + rng.bipolar() * 0.15).clamp(0.15, ASPECT - 0.15);
            m.y = (m.home + rng.bipolar() * 0.12).clamp(0.12, 0.88);
            m.vx = 0.0;
            m.vy = 0.0;
            m.idle_spin = rng.bipolar() * 0.35;
            m.spin = m.idle_spin;
            m.angle = rng.unit() * std::f32::consts::TAU;
            m.wander = std::array::from_fn(|_| rng.unit() * std::f32::consts::TAU);
        }
    }

    /// Molecule `k` has a new note: its size, weight and height follow it.
    /// `t` is where the note sits in the register (0 lowest, 1 highest).
    pub fn tune(&mut self, k: usize, note: u8, role: u8, t: f32, count: usize) {
        let m = &mut self.molecules[k];
        let base = (0.078 * (8.0 / (count as f32 + 2.0)).sqrt()).clamp(0.045, 0.11);
        m.size = base * (1.3 - 0.55 * t.clamp(0.0, 1.0));
        m.mass = 4.0 * (m.size / 0.07).powi(2);
        m.home = if count <= 1 { 0.5 } else { 0.84 - 0.68 * t.clamp(0.0, 1.0) };
        m.note = note;
        m.role = role;
    }

    /// A molecule appears (it condenses: its radius grows from nothing).
    pub fn condense(&mut self, k: usize, rng: &mut Rng) {
        let m = &mut self.molecules[k];
        m.home_x = 0.2 + rng.unit() * (ASPECT - 0.4);
        m.x = m.home_x;
        m.y = (m.home + rng.bipolar() * 0.1).clamp(0.12, 0.88);
        m.vx = 0.0;
        m.vy = 0.0;
        m.r = 0.0;
        m.ring = 0.0;
        m.idle_spin = rng.bipolar() * 0.35;
        m.spin = m.idle_spin;
        m.angle = rng.unit() * std::f32::consts::TAU;
        m.wander = std::array::from_fn(|_| rng.unit() * std::f32::consts::TAU);
    }

    /// An atom enters from an edge, heading in.
    pub fn spawn(&mut self, rng: &mut Rng) {
        if self.atom_count >= MAX_ATOMS {
            return;
        }
        let edge = (rng.unit() * 4.0) as u32;
        let along = rng.unit();
        let (x, y) = match edge {
            0 => (along * ASPECT, 0.03),
            1 => (ASPECT - 0.03, along),
            2 => (along * ASPECT, 0.97),
            _ => (0.03, along),
        };
        // Towards a point near the middle, at a lively speed.
        let (tx, ty) = (ASPECT * (0.3 + 0.4 * rng.unit()), 0.3 + 0.4 * rng.unit());
        let (dx, dy) = (tx - x, ty - y);
        let d = (dx * dx + dy * dy).sqrt().max(1e-3);
        let speed = 0.5 + 0.6 * rng.unit();
        self.atoms[self.atom_count] = Atom { x, y, vx: dx / d * speed, vy: dy / d * speed, charge: 0.0, from: 0, age: 0.0 };
        self.atom_count += 1;
    }

    /// The liquid stirred at `x`, `y`: what is near moves with the stir's
    /// velocity `vx`, `vy` or, if it has none, is pushed outwards.
    pub fn stir(&mut self, x: f32, y: f32, vx: f32, vy: f32) {
        const REACH: f32 = 0.3;
        let (vx, vy) = (vx.clamp(-4.0, 4.0), vy.clamp(-4.0, 4.0));
        let push = vx == 0.0 && vy == 0.0;
        let shove = |px: f32, py: f32, gain: f32| -> Option<(f32, f32)> {
            let (dx, dy) = (px - x, py - y);
            let d = (dx * dx + dy * dy).sqrt();
            if d >= REACH {
                return None;
            }
            let w = (1.0 - d / REACH) * gain;
            Some(if push { (dx / d.max(1e-3) * 1.8 * w, dy / d.max(1e-3) * 1.8 * w) } else { (vx * 0.7 * w, vy * 0.7 * w) })
        };
        for a in &mut self.atoms[..self.atom_count] {
            if let Some((dx, dy)) = shove(a.x, a.y, 1.0) {
                a.vx += dx;
                a.vy += dy;
            }
        }
        for m in &mut self.molecules[..self.molecule_count] {
            if let Some((dx, dy)) = shove(m.x, m.y, 0.3) {
                m.vx += dx;
                m.vy += dy;
            }
        }
    }

    /// Moves everything on by `dt` seconds. Strikes go to `strikes` (as many
    /// as fit); returns how many.
    pub fn step(&mut self, s: &Settings, dt: f32, rng: &mut Rng, strikes: &mut [Strike; MAX_STRIKES]) -> usize {
        self.time += dt;
        let t = self.time;
        self.move_molecules(s, dt, t);
        let mut count = 0;
        let n = self.molecule_count;
        let ra = s.atom_r;
        let kick = s.heat * dt.sqrt();
        let drag = (-s.drag * dt).exp();
        for (ai, a) in self.atoms[..self.atom_count].iter_mut().enumerate() {
            a.age += dt;
            if s.charge > 0.0 {
                a.charge = (a.charge - dt / s.charge).max(0.0);
            } else {
                a.charge = 0.0;
            }
            // Gravity from every molecule.
            let (mut ax, mut ay) = (0.0, 0.0);
            for (j, m) in self.molecules[..n].iter().enumerate() {
                let (dx, dy) = (m.x - a.x, m.y - a.y);
                let d2 = dx * dx + dy * dy;
                let d = d2.sqrt().max(1e-4);
                let q = d2 / (s.reach * s.reach);
                let mut f = s.pull * m.mass * 0.25 / (d2 + m.r * m.r + 1e-4) / (1.0 + q * q);
                if a.charge > 0.0 && a.from as usize == j {
                    // Charged by its last strike: pushed off.
                    f *= 1.0 - 2.4 * a.charge;
                }
                ax += f * dx / d;
                ay += f * dy / d;
            }
            if s.flow > 0.0 {
                let (ux, uy) = current(a.x, a.y, t);
                ax += s.flow * ux;
                ay += s.flow * uy;
            }
            a.vx = (a.vx + ax * dt) * drag + kick * gauss(rng);
            a.vy = (a.vy + ay * dt) * drag + kick * gauss(rng);
            let speed = (a.vx * a.vx + a.vy * a.vy).sqrt();
            if speed > MAX_SPEED {
                a.vx *= MAX_SPEED / speed;
                a.vy *= MAX_SPEED / speed;
            }
            a.x += a.vx * dt;
            a.y += a.vy * dt;
            // The tank's walls.
            if a.x < ra {
                a.x = ra;
                a.vx = a.vx.abs() * 0.85;
            } else if a.x > ASPECT - ra {
                a.x = ASPECT - ra;
                a.vx = -a.vx.abs() * 0.85;
            }
            if a.y < ra {
                a.y = ra;
                a.vy = a.vy.abs() * 0.85;
            } else if a.y > 1.0 - ra {
                a.y = 1.0 - ra;
                a.vy = -a.vy.abs() * 0.85;
            }
            // Strikes.
            for (j, m) in self.molecules[..n].iter_mut().enumerate() {
                let reach = m.r + ra;
                let (dx, dy) = (a.x - m.x, a.y - m.y);
                let d2 = dx * dx + dy * dy;
                if d2 >= reach * reach || m.r < 0.3 * m.size {
                    continue;
                }
                let d = d2.sqrt().max(1e-5);
                let (nx, ny) = (dx / d, dy / d);
                let (rvx, rvy) = (a.vx - m.vx, a.vy - m.vy);
                let closing = rvx * nx + rvy * ny;
                // Out of the molecule, onto its surface.
                a.x = m.x + nx * (reach + 1e-4);
                a.y = m.y + ny * (reach + 1e-4);
                if closing >= 0.0 {
                    continue;
                }
                let impulse = -(1.0 + s.bounce) * closing / (1.0 / s.atom_mass + 1.0 / m.mass);
                a.vx += impulse / s.atom_mass * nx;
                a.vy += impulse / s.atom_mass * ny;
                m.vx -= impulse / m.mass * nx;
                m.vy -= impulse / m.mass * ny;
                // Off-centre strikes spin it.
                let tangential = -rvx * ny + rvy * nx;
                m.spin += tangential * s.atom_mass / m.mass * 6.0;
                a.charge = if s.charge > 0.0 { 1.0 } else { 0.0 };
                a.from = j as u8;
                let speed = -closing;
                if speed >= MIN_STRIKE {
                    let force = 1.0 - (-speed * (0.45 + 0.55 * s.atom_mass.sqrt()) / 1.5).exp();
                    m.ring = m.ring.max(force);
                    if count < MAX_STRIKES {
                        strikes[count] = Strike { molecule: j, atom: ai, x: m.x + nx * m.r, y: m.y + ny * m.r, force };
                        count += 1;
                    }
                }
            }
        }
        count
    }

    fn move_molecules(&mut self, s: &Settings, dt: f32, t: f32) {
        let n = self.molecule_count;
        let mut force = [(0.0f32, 0.0f32); MAX_MOLECULES];
        // Held apart up close, drawn loosely together a little further out:
        // they gather into clusters that keep shifting.
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&self.molecules[i], &self.molecules[j]);
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let d = (dx * dx + dy * dy).sqrt().max(1e-4);
                let near = (a.size + b.size) * 1.35;
                let far = near * 2.2;
                let f = if d < near {
                    -6.0 * (near - d) / near
                } else if d < far {
                    0.18 * ((d - near) / (far - near) * std::f32::consts::PI).sin()
                } else {
                    0.0
                };
                let (fx, fy) = (f * dx / d, f * dy / d);
                force[i].0 += fx;
                force[i].1 += fy;
                force[j].0 -= fx;
                force[j].1 -= fy;
            }
        }
        let drag = (-(1.6 + 3.0 * s.viscosity) * dt).exp();
        for (m, f) in self.molecules[..n].iter_mut().zip(force) {
            m.r += (m.size - m.r) * (1.0 - (-dt / 0.35).exp());
            // Buoyancy: towards the height its note floats at.
            let mut ax = f.0 + 0.5 * (m.home_x - m.x);
            let mut ay = f.1 + 2.2 * (m.home - m.y);
            // Wandering, two slow waves each way.
            let w = s.drift * 0.55;
            ax += w * ((t * 0.21 + m.wander[0]).sin() + 0.6 * (t * 0.47 + m.wander[1]).sin());
            ay += w * 0.6 * ((t * 0.17 + m.wander[2]).sin() + 0.6 * (t * 0.39 + m.wander[3]).sin());
            if s.flow > 0.0 {
                let (ux, uy) = current(m.x, m.y, t);
                ax += 0.3 * s.flow * ux;
                ay += 0.3 * s.flow * uy;
            }
            // The walls push back softly.
            let margin = m.size * 1.2;
            ax += 8.0 * ((margin - m.x).max(0.0) - (m.x - (ASPECT - margin)).max(0.0));
            ay += 8.0 * ((margin - m.y).max(0.0) - (m.y - (1.0 - margin)).max(0.0));
            m.vx = (m.vx + ax * dt) * drag;
            m.vy = (m.vy + ay * dt) * drag;
            let edge = m.size * 0.9;
            m.x = (m.x + m.vx * dt).clamp(edge, ASPECT - edge);
            m.y = (m.y + m.vy * dt).clamp(edge, 1.0 - edge);
            m.spin += (m.idle_spin - m.spin) * (1.0 - (-dt * 0.8).exp());
            m.spin = m.spin.clamp(-14.0, 14.0);
            m.angle = (m.angle + m.spin * dt).rem_euclid(std::f32::consts::TAU);
            // The ring dies away: sooner in a thick liquid, later in a big
            // molecule.
            m.decay = s.length / 3.0 * (m.size / 0.07).clamp(0.6, 1.6) / (1.0 + 1.5 * s.viscosity);
            let fade = (-dt / m.decay.max(0.005)).exp();
            m.ring *= fade;
        }
    }
}
