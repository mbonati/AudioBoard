//! Automa's four generators. On every step each one offers the scale
//! degrees it would play (none: a rest), each with a weight, and how much
//! energy it has; the node picks among them and fits them to the chord.
//!
//! - LIFE: a Life-like cellular automaton on a 16 × 16 torus, read column
//!   by column like a step sequencer: each row is a scale degree.
//! - CASCADE: an elementary (one-dimensional) automaton: each generation is
//!   a row of sixteen degrees, and the generations pour down the display
//!   (rule 90 draws Sierpiński's triangle).
//! - TREE: Per Nørgård's infinity series, a melody that holds itself at
//!   every scale: every second note plays it upside down, every fourth plays
//!   it again.
//! - JULIA: the orbit of a point under z → z² + c, the pitch its angle round
//!   the fixed point. Inside the Mandelbrot set's main cardioid the orbit
//!   spirals in (a phrase that settles), on its edge it circles (a figure
//!   that repeats), past it it may escape (a rest).

use std::f32::consts::TAU;

use super::super::Rng;
use super::hash01;

/// LIFE's columns, and the cells of a CASCADE generation.
pub const W: usize = 16;
/// LIFE's rows: scale degrees, the lowest at the bottom.
pub const H: usize = 16;
/// The degree in the middle of the range.
pub const CENTRE: i32 = 7;

/// What a generator offers on one step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Offer {
    pub degrees: [i8; H],
    pub weights: [f32; H],
    pub len: usize,
    /// How strongly it is sounding, 0…1: drives the dynamics.
    pub energy: f32,
    /// The cells that offered (LIFE's rows, CASCADE's columns).
    pub cells: u16,
}

impl Offer {
    fn push(&mut self, degree: i32, weight: f32) {
        if self.len < H {
            self.degrees[self.len] = degree.clamp(-64, 63) as i8;
            self.weights[self.len] = weight;
            self.len += 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn degrees(&self) -> impl Iterator<Item = (i32, f32)> + '_ {
        self.degrees[..self.len].iter().zip(&self.weights[..self.len]).map(|(&d, &w)| (d as i32, w))
    }
}

// ---------------------------------------------------------------------------
// LIFE
// ---------------------------------------------------------------------------

/// Life-like rules: (name, what it does, born with, survives with), the
/// last two as bit masks of neighbour counts.
pub const LIFE_RULES: [(&str, &str, u16, u16); 6] = [
    ("LIFE", "B3/S23 · Conway's Game of Life: gliders, blinkers, still lifes", 1 << 3, 1 << 2 | 1 << 3),
    ("HIGHLIFE", "B36/S23 · Life with replicators", 1 << 3 | 1 << 6, 1 << 2 | 1 << 3),
    ("DAY&NIGHT", "B3678/S34678 · blobs that grow and melt", 1 << 3 | 1 << 6 | 1 << 7 | 1 << 8, 1 << 3 | 1 << 4 | 1 << 6 | 1 << 7 | 1 << 8),
    ("AMOEBA", "B357/S1358 · restless, spreading shapes", 1 << 3 | 1 << 5 | 1 << 7, 1 << 1 | 1 << 3 | 1 << 5 | 1 << 8),
    ("MAZE", "B3/S12345 · grows into corridors, then holds", 1 << 3, 0b11_1110),
    ("SEEDS", "B2/S · every cell lives one generation: sparks", 1 << 2, 0),
];

/// Shapes sown into a world that died or settled: (x, y) cells.
const SHAPES: [&[(usize, usize)]; 4] = [
    &[(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)],         // glider
    &[(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)],         // R-pentomino
    &[(0, 0), (1, 0), (2, 0), (1, 1)],                 // T: grows into traffic lights
    &[(0, 1), (1, 0), (1, 1), (1, 2), (2, 2), (3, 1)], // a scrap that wanders
];

pub struct Life {
    /// One bit per column.
    pub rows: [u16; H],
    /// Generations each cell has been alive (0: dead).
    pub age: [[u8; W]; H],
    pub generation: u32,
    /// Recent states, to notice when the world settles into a loop.
    recent: [u64; 8],
    /// Generations in a row that repeated a recent one.
    stale: u32,
}

impl Life {
    pub fn new(rows: [u16; H]) -> Self {
        let mut life = Self { rows: [0; H], age: [[0; W]; H], generation: 0, recent: [u64::MAX; 8], stale: 0 };
        life.load(rows);
        life
    }

    /// Starts over from `rows`.
    pub fn load(&mut self, rows: [u16; H]) {
        self.rows = rows;
        self.generation = 0;
        self.recent = [u64::MAX; 8];
        self.stale = 0;
        for y in 0..H {
            for x in 0..W {
                self.age[y][x] = self.alive(x, y) as u8;
            }
        }
    }

    pub fn alive(&self, x: usize, y: usize) -> bool {
        self.rows[y % H] >> (x % W) & 1 == 1
    }

    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        let (x, y) = (x % W, y % H);
        if on {
            self.rows[y] |= 1 << x;
            self.age[y][x] = self.age[y][x].max(1);
        } else {
            self.rows[y] &= !(1 << x);
            self.age[y][x] = 0;
        }
    }

    pub fn population(&self) -> u32 {
        self.rows.iter().map(|r| r.count_ones()).sum()
    }

    fn neighbours(&self, x: usize, y: usize) -> u32 {
        let mut n = 0;
        for dy in [H - 1, 0, 1] {
            for dx in [W - 1, 0, 1] {
                if (dx, dy) != (0, 0) && self.alive(x + dx, y + dy) {
                    n += 1;
                }
            }
        }
        n
    }

    /// The next generation under `rule` (an index into [`LIFE_RULES`]).
    pub fn step(&mut self, rule: usize) {
        let (_, _, born, survive) = LIFE_RULES[rule.min(LIFE_RULES.len() - 1)];
        let mut next = [0u16; H];
        for (y, row) in next.iter_mut().enumerate() {
            for x in 0..W {
                let mask = if self.alive(x, y) { survive } else { born };
                if mask >> self.neighbours(x, y) & 1 == 1 {
                    *row |= 1 << x;
                }
            }
        }
        for (y, row) in next.iter().enumerate() {
            for x in 0..W {
                let age = &mut self.age[y][x];
                *age = if row >> x & 1 == 1 { age.saturating_add(1) } else { 0 };
            }
        }
        self.rows = next;
        self.generation = self.generation.wrapping_add(1);
        let print = fingerprint(&self.rows);
        self.stale = if self.recent.contains(&print) { self.stale + 1 } else { 0 };
        self.recent[self.generation as usize % self.recent.len()] = print;
    }

    /// Keeps the world going: with MUTATE (`amount` > 0) a few cells are
    /// born next to living ones now and then, and a world that died or has
    /// looped for a while gets a new shape sown into it.
    pub fn mutate(&mut self, rng: &mut Rng, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        if self.population() == 0 || self.stale >= 4 {
            self.sow(rng);
            self.stale = 0;
        }
        if rng.unit() < amount {
            let births = 1 + (rng.unit() * 3.0 * amount) as usize;
            for _ in 0..births {
                let (x, y) = self.near_life(rng);
                self.set(x, y, true);
            }
        }
    }

    /// A cell next to a living one (anywhere, if none lives).
    fn near_life(&self, rng: &mut Rng) -> (usize, usize) {
        let pick = |rng: &mut Rng, n: usize| (rng.unit() * n as f32) as usize % n;
        let population = self.population() as usize;
        if population == 0 {
            return (pick(rng, W), pick(rng, H));
        }
        let mut k = pick(rng, population);
        for y in 0..H {
            for x in 0..W {
                if self.alive(x, y) {
                    if k == 0 {
                        return ((x + W - 1 + pick(rng, 3)) % W, (y + H - 1 + pick(rng, 3)) % H);
                    }
                    k -= 1;
                }
            }
        }
        (0, 0)
    }

    fn sow(&mut self, rng: &mut Rng) {
        let shape = SHAPES[(rng.unit() * SHAPES.len() as f32) as usize % SHAPES.len()];
        let (ox, oy) = ((rng.unit() * W as f32) as usize, (rng.unit() * H as f32) as usize);
        let (flip_x, flip_y) = (rng.unit() < 0.5, rng.unit() < 0.5);
        for &(x, y) in shape {
            let x = if flip_x { 3 - x } else { x };
            let y = if flip_y { 3 - y } else { y };
            self.set(ox + x, oy + y, true);
        }
    }

    /// The living cells of column `x`: newborn ones count more.
    pub fn column(&self, x: usize) -> Offer {
        let mut offer = Offer::default();
        let mut young = 0;
        for y in 0..H {
            if self.alive(x, y) {
                let age = self.age[y][x];
                young += (age <= 1) as usize;
                offer.push(y as i32, if age <= 1 { 1.0 } else { 0.6 });
                offer.cells |= 1 << y;
            }
        }
        if offer.len > 0 {
            offer.energy = 0.6 * (offer.len as f32 / 5.0).min(1.0) + 0.4 * young as f32 / offer.len as f32;
        }
        offer
    }
}

fn fingerprint(rows: &[u16; H]) -> u64 {
    rows.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &r| (h ^ r as u64).wrapping_mul(0x0100_0000_01B3))
}

/// A random start for LIFE: about a fifth of the cells alive.
pub fn soup(seed: u32) -> [u16; H] {
    let mut rows = [0u16; H];
    for (y, row) in rows.iter_mut().enumerate() {
        for x in 0..W {
            if hash01(seed, (y * W + x) as u64, 0x50_0B) < 0.21 {
                *row |= 1 << x;
            }
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// CASCADE
// ---------------------------------------------------------------------------

/// Elementary automaton rules (Wolfram's numbering).
pub const CASCADE_RULES: [(u8, &str); 8] = [
    (90, "Sierpiński's triangle: each cell the sum of its neighbours"),
    (30, "chaos from order: a rule used to make random numbers"),
    (110, "structures that move and collide (it can compute anything)"),
    (150, "a denser fractal, symmetric"),
    (45, "noisy stripes"),
    (73, "walls with chatter between them"),
    (105, "a lattice of nested triangles"),
    (22, "a sparse Sierpiński"),
];

/// Generations the waterfall shows.
pub const HISTORY: usize = 16;

pub struct Cascade {
    /// The last generations, `head` the newest.
    rows: [u16; HISTORY],
    head: usize,
    start: u16,
    pub generation: u32,
}

impl Cascade {
    pub fn new(start: u16) -> Self {
        let mut cascade = Self { rows: [0; HISTORY], head: 0, start, generation: 0 };
        cascade.reset(start);
        cascade
    }

    pub fn reset(&mut self, start: u16) {
        self.start = start;
        self.rows = [0; HISTORY];
        self.rows[0] = start;
        self.head = 0;
        self.generation = 0;
    }

    /// The generation `age` back (0: the newest).
    pub fn row(&self, age: usize) -> u16 {
        self.rows[(self.head + HISTORY - age % HISTORY) % HISTORY]
    }

    /// The next generation under `rule` (an index into [`CASCADE_RULES`]).
    /// A row that dies out (or fills up) starts again: from the start with
    /// no MUTATE, from a random cell with it; MUTATE also flips a cell now
    /// and then.
    pub fn step(&mut self, rule: usize, rng: &mut Rng, mutate: f32) {
        let rule = CASCADE_RULES[rule.min(CASCADE_RULES.len() - 1)].0;
        let now = self.row(0);
        let bit = |x: usize| (now >> (x % W) & 1) as u8;
        let mut next = 0u16;
        for x in 0..W {
            let pattern = bit(x + W - 1) << 2 | bit(x) << 1 | bit(x + 1);
            if rule >> pattern & 1 == 1 {
                next |= 1 << x;
            }
        }
        let random_cell = |rng: &mut Rng| 1u16 << ((rng.unit() * W as f32) as usize % W);
        if next == 0 || next == u16::MAX {
            next = if mutate > 0.0 { random_cell(rng) } else { self.start };
        }
        if mutate > 0.0 && rng.unit() < mutate * 0.5 {
            next ^= random_cell(rng);
        }
        self.head = (self.head + 1) % HISTORY;
        self.rows[self.head] = next;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The newest generation's cells.
    pub fn offer(&self) -> Offer {
        let row = self.row(0);
        let mut offer = Offer { cells: row, ..Default::default() };
        for x in 0..W {
            if row >> x & 1 == 1 {
                // Cells that just changed state count more.
                let born = self.row(1) >> x & 1 == 0;
                offer.push(x as i32, if born { 1.0 } else { 0.7 });
            }
        }
        offer.energy = (row.count_ones() as f32 / 6.0).min(1.0);
        offer
    }
}

/// CASCADE's first row: a single cell, somewhere in the middle.
pub fn first_row(seed: u32) -> u16 {
    1 << (4 + seed % 8)
}

// ---------------------------------------------------------------------------
// TREE
// ---------------------------------------------------------------------------

/// Nørgård's infinity series: a(0) = 0, a(2n) = −a(n), a(2n + 1) = a(n) + 1.
/// Read from the top bit down, a 1 steps up and a 0 turns the melody over.
pub fn series(n: u64) -> i32 {
    let bits = u64::BITS - n.leading_zeros();
    (0..bits).rev().fold(0, |v, k| if n >> k & 1 == 1 { v + 1 } else { -v })
}

/// `d` reflected into `lo..=hi`.
pub fn fold(d: i32, lo: i32, hi: i32) -> i32 {
    let span = (hi - lo).max(1);
    let m = (d - lo).rem_euclid(2 * span);
    lo + if m > span { 2 * span - m } else { m }
}

/// The series is played from 0 to here, then from 0 again (it stays exact
/// in the display values).
const TREE_WRAP: u64 = 1 << 24;

pub struct Tree {
    /// The term playing next, and the last one played.
    pub n: u64,
    pub played: u64,
}

impl Tree {
    pub fn new(start: u64) -> Self {
        Self { n: start % TREE_WRAP, played: start % TREE_WRAP }
    }

    pub fn reset(&mut self, start: u64) {
        *self = Self::new(start);
    }

    /// The term `n` as a degree: INTERVAL degrees per step of the series,
    /// folded into a range LEAP wide round the middle.
    pub fn offer(&self, interval: i32, span: i32) -> Offer {
        let mut offer = Offer::default();
        let half = (span / 2).max(1);
        let d = fold(CENTRE + series(self.n) * interval, CENTRE - half, CENTRE + half);
        offer.push(d, 1.0);
        // Terms deeper in the tree (n divisible by more twos) are accents.
        offer.energy = 0.45 + 0.55 * (self.n.trailing_zeros().min(6) as f32 / 6.0);
        offer
    }

    /// On by STRIDE terms; with MUTATE it sometimes jumps elsewhere in the
    /// series (a branch it had not taken).
    pub fn advance(&mut self, stride: u64, rng: &mut Rng, mutate: f32) {
        let jump = if mutate > 0.0 && rng.unit() < mutate * 0.25 { 1 + (rng.unit() * 31.0) as u64 } else { 1 };
        self.played = self.n;
        self.n = (self.n + stride.max(1) * jump) % TREE_WRAP;
    }
}

pub fn tree_start(seed: u32) -> u64 {
    (hash01(seed, 0, 0x7EE) * 512.0) as u64
}

// ---------------------------------------------------------------------------
// JULIA
// ---------------------------------------------------------------------------

pub type Complex = (f32, f32);

fn mul(a: Complex, b: Complex) -> Complex {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn sqrt(a: Complex) -> Complex {
    let r = a.0.hypot(a.1);
    let re = ((r + a.0) * 0.5).max(0.0).sqrt();
    let im = ((r - a.0) * 0.5).max(0.0).sqrt();
    (re, if a.1 < 0.0 { -im } else { im })
}

/// The c of SHAPE (an angle round the main cardioid's edge), CHAOS (from
/// inside it, 0, through its edge, ½, to past it, 1) and MORPH (c drifting
/// round with the beat: at 1, once every sixteen bars).
pub fn julia_c(shape: f32, chaos: f32, morph: f32, beat: f64) -> Complex {
    let turns = (shape as f64 + beat * morph as f64 / 64.0).rem_euclid(1.0) as f32;
    let a = turns * TAU;
    let r = 0.9 + 0.2 * chaos.clamp(0.0, 1.0);
    // The cardioid's edge: e^{ia}/2 − e^{2ia}/4.
    (r * (0.5 * a.cos() - 0.25 * (2.0 * a).cos()), r * (0.5 * a.sin() - 0.25 * (2.0 * a).sin()))
}

/// The fixed point of z → z² + c that orbits circle round (attracting
/// inside the cardioid).
pub fn fixed_point(c: Complex) -> Complex {
    let s = sqrt((1.0 - 4.0 * c.0, -4.0 * c.1));
    ((1.0 - s.0) * 0.5, -s.1 * 0.5)
}

/// Orbit points the display keeps.
pub const TRAIL: usize = 32;

pub struct Julia {
    pub z: Complex,
    pub c: Complex,
    pub fix: Complex,
    /// Phrases so far: each starts the orbit from a new point.
    pub phrase: u32,
    pub trail: [Complex; TRAIL],
    /// Where the next point goes in `trail`.
    pub trail_pos: usize,
    seed: u32,
    /// The orbit settled (or escaped): the next step starts a phrase.
    done: bool,
}

impl Julia {
    pub fn new(seed: u32) -> Self {
        let mut julia = Self { z: (0.0, 0.0), c: (0.0, 0.0), fix: (0.0, 0.0), phrase: 0, trail: [(0.0, 0.0); TRAIL], trail_pos: 0, seed, done: false };
        julia.reset(seed);
        julia
    }

    pub fn reset(&mut self, seed: u32) {
        self.seed = seed;
        self.phrase = 0;
        self.begin();
        self.trail = [self.z; TRAIL];
    }

    /// A phrase starts near 0, the critical point, whose orbit shows the
    /// set's character best.
    fn begin(&mut self) {
        let a = hash01(self.seed, self.phrase as u64, 0x1A) * TAU;
        self.z = (0.12 * a.cos(), 0.12 * a.sin());
        self.done = false;
    }

    pub fn set_c(&mut self, c: Complex) {
        self.c = c;
        self.fix = fixed_point(c);
    }

    /// One iteration. The angle round the fixed point is the degree, in a
    /// range `span` degrees wide; the distance from it is the energy. An
    /// orbit that escapes rests and starts a new phrase, one that settles
    /// starts one after its last note.
    pub fn step(&mut self, span: i32, rng: &mut Rng, mutate: f32) -> Offer {
        if self.done {
            self.phrase = self.phrase.wrapping_add(1);
            self.begin();
        }
        let before = self.z;
        let mut z = mul(self.z, self.z);
        z = (z.0 + self.c.0, z.1 + self.c.1);
        if mutate > 0.0 {
            z.0 += rng.bipolar() * 0.04 * mutate;
            z.1 += rng.bipolar() * 0.04 * mutate;
        }
        let mut offer = Offer::default();
        let r2 = z.0 * z.0 + z.1 * z.1;
        // Escaped (or lost to NaN).
        if r2.is_nan() || r2 > 4.0 {
            self.done = true;
            return offer;
        }
        self.z = z;
        self.trail[self.trail_pos] = z;
        self.trail_pos = (self.trail_pos + 1) % TRAIL;
        let (dx, dy) = (z.0 - self.fix.0, z.1 - self.fix.1);
        let h = dy.atan2(dx) / TAU + 0.5;
        offer.push(CENTRE + ((h - 0.5) * span as f32).round() as i32, 1.0);
        offer.energy = (dx.hypot(dy) * 1.2).clamp(0.15, 1.0);
        if (z.0 - before.0).hypot(z.1 - before.1) < 1e-3 {
            self.done = true;
        }
        offer
    }
}

/// Escape-time of `z` under z² + c, 0…1 (1: it never left within `limit`),
/// smoothed between iterations. For drawing the set.
pub fn escape(mut z: Complex, c: Complex, limit: u32) -> f32 {
    for i in 0..limit {
        let r2 = z.0 * z.0 + z.1 * z.1;
        if r2 > 16.0 {
            let smooth = i as f32 + 1.0 - (r2.ln() * 0.5).ln() / std::f32::consts::LN_2;
            return (smooth / limit as f32).clamp(0.0, 0.999);
        }
        z = mul(z, z);
        z = (z.0 + c.0, z.1 + c.1);
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(cells: &[(usize, usize)]) -> [u16; H] {
        let mut rows = [0u16; H];
        for &(x, y) in cells {
            rows[y] |= 1 << x;
        }
        rows
    }

    #[test]
    fn life_blinks_and_glides() {
        let mut life = Life::new(rows(&[(4, 5), (5, 5), (6, 5)]));
        life.step(0);
        assert_eq!(life.rows, rows(&[(5, 4), (5, 5), (5, 6)]), "a blinker turns");
        life.step(0);
        assert_eq!(life.rows, rows(&[(4, 5), (5, 5), (6, 5)]));
        assert_eq!(life.age[5][5], 3, "its middle never died");

        // A glider comes back to its shape four generations later, one cell
        // along each way (and wraps round the torus).
        let glider = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
        let mut life = Life::new(rows(&glider));
        for _ in 0..4 {
            life.step(0);
        }
        let moved: Vec<(usize, usize)> = glider.iter().map(|&(x, y)| (x + 1, y + 1)).collect();
        assert_eq!(life.rows, rows(&moved));
        for _ in 0..4 * 15 {
            life.step(0);
        }
        assert_eq!(life.rows, rows(&glider), "round the torus and home");
    }

    #[test]
    fn life_offers_its_column_and_keeps_alive() {
        let life = Life::new(rows(&[(3, 2), (3, 9), (4, 1)]));
        let offer = life.column(3);
        assert_eq!(offer.degrees().map(|d| d.0).collect::<Vec<_>>(), [2, 9]);
        assert_eq!(offer.cells, 1 << 2 | 1 << 9);
        assert!(life.column(0).is_empty());

        // A still life is sown into after a few generations with MUTATE,
        // never without it.
        let block = rows(&[(5, 5), (6, 5), (5, 6), (6, 6)]);
        let mut rng = Rng::new(3);
        let mut life = Life::new(block);
        for _ in 0..12 {
            life.step(0);
            life.mutate(&mut rng, 0.0);
        }
        assert_eq!(life.rows, block);
        let mut grew = false;
        for _ in 0..12 {
            life.step(0);
            life.mutate(&mut rng, 0.01);
            grew |= life.population() > 4;
        }
        assert!(grew);
    }

    #[test]
    fn rule_90_draws_sierpinski() {
        let mut cascade = Cascade::new(1 << 8);
        let mut rng = Rng::new(1);
        let mut seen = Vec::new();
        for _ in 0..4 {
            cascade.step(0, &mut rng, 0.0);
            seen.push(cascade.row(0));
        }
        assert_eq!(seen, [0b101 << 7, 0b1_0001 << 6, 0b101_0101 << 5, 0b1_0000_0001 << 4]);
        assert_eq!(cascade.row(4), 1 << 8, "the history goes back to the start");
        let offer = cascade.offer();
        assert_eq!(offer.degrees().map(|d| d.0).collect::<Vec<_>>(), [4, 12]);
        // On sixteen cells it dies after eight generations, and starts again.
        for _ in 0..4 {
            cascade.step(0, &mut rng, 0.0);
        }
        assert_eq!(cascade.row(0), 1 << 8);
    }

    #[test]
    fn the_infinity_series() {
        let terms: Vec<i32> = (0..16).map(series).collect();
        assert_eq!(terms, [0, 1, -1, 2, 1, 0, -2, 3, -1, 2, 0, 1, 2, -1, -3, 4]);
        // It holds itself: every second term upside down, every fourth as is.
        for n in 0..500 {
            assert_eq!(series(2 * n), -series(n));
            assert_eq!(series(4 * n), series(n));
        }
        assert_eq!(fold(18, 0, 15), 12);
        assert_eq!(fold(-3, 0, 15), 3);
        assert_eq!(fold(7, 0, 15), 7);
    }

    #[test]
    fn julia_orbits_spiral_cycle_and_escape() {
        let mut rng = Rng::new(1);
        // Inside the cardioid the orbit settles on the fixed point, then a
        // new phrase begins.
        let mut julia = Julia::new(5);
        julia.set_c(julia_c(0.3, 0.0, 0.0, 0.0));
        let fix = julia.fix;
        let mut notes = 0;
        for _ in 0..400 {
            notes += !julia.step(12, &mut rng, 0.0).is_empty() as usize;
        }
        assert_eq!(notes, 400, "it never escapes");
        assert!(julia.phrase > 0, "it settled, more than once");
        let z = mul(fix, fix);
        assert!((z.0 + julia.c.0 - fix.0).abs() < 1e-5 && (z.1 + julia.c.1 - fix.1).abs() < 1e-5, "z* = z*² + c");

        // Past the edge, away from the bulbs, it escapes: rests.
        let mut julia = Julia::new(5);
        julia.set_c((0.45, 0.1));
        let rests = (0..40).filter(|_| julia.step(12, &mut rng, 0.0).is_empty()).count();
        assert!(rests > 3);
        assert!(escape((0.0, 0.0), (0.45, 0.1), 40) < 1.0);
        assert_eq!(escape((0.0, 0.0), (-0.1, 0.1), 40), 1.0);
    }
}
