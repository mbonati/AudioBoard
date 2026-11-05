//! Where Vesper's sounds come from. There are no sample files: each recipe
//! ([`super::sounds`]) is rendered into a loop, a few seconds of stereo that
//! then plays like a sample. A family of partials, each widened into a narrow
//! band of noise, is summed in the frequency domain with random phases and
//! brought back by one inverse FFT; every partial lands on a whole number of
//! cycles, so the loop has no seam, and each note beats and drifts by itself
//! at no cost on the audio thread. The real and imaginary parts of that one
//! FFT are the two channels: true stereo, uncorrelated.
//!
//! Rendering takes tenths of a second, so a thread of the node's own does it
//! whenever a sound is picked: the audio thread only ever takes a finished
//! bank from a mailbox, hands the old one back, and never waits.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::sounds::{Recipe, recipes};
use super::SAMPLED;
use crate::audio::midi::note_hz;
use crate::audio::modules::{ModuleParams, Rng};

/// What every loop is scaled to, RMS.
pub const LOOP_RMS: f32 = 0.2;
/// Pad's loops stand on these notes, each played up to ±9 semitones (a note
/// played further than that shows what is left of its top end).
const PAD_NOTES: [f32; 3] = [48.0, 66.0, 84.0];
/// A loop played up by this much at most still has room for its partials.
const HEADROOM: f32 = 1.7;

/// A bin this far above its neighbours' power is a line standing alone.
const LINE: f32 = 64.0;

/// A rendered stereo loop.
pub struct Table {
    pub left: Box<[f32]>,
    pub right: Box<[f32]>,
    pub mask: usize,
    /// The note it plays when read at speed 1.
    pub base: f32,
}

/// A sound, ready to play: its loops, a range of notes each.
pub struct Bank {
    pub sound: usize,
    pub tonal: bool,
    /// How far its voices wander in pitch and level (1: as a pad does).
    pub life: f32,
    pub tables: Box<[Table]>,
}

impl Bank {
    /// The loop nearest a note.
    pub fn pick(&self, note: f32) -> usize {
        let mut best = (0, f32::MAX);
        for (k, t) in self.tables.iter().enumerate() {
            let d = (t.base - note).abs();
            if d < best.1 {
                best = (k, d);
            }
        }
        best.0
    }
}

/// Seconds a layer's loops last at least, and the notes they stand on.
fn layout(layer: usize, recipe: &Recipe) -> (f32, &'static [f32]) {
    match layer {
        0 => (5.0, &PAD_NOTES),
        1 => (2.5, &[36.0]),
        2 => (5.0, &[48.0]),
        _ if recipe.tonal => (10.0, &[52.0]),
        _ => (10.0, &[48.0]),
    }
}

/// Renders a sound, in full.
pub fn render(layer: usize, sound: usize, sr: f32) -> Box<Bank> {
    let list = recipes(layer);
    let recipe = &list[sound.min(list.len() - 1)];
    let (secs, notes) = layout(layer, recipe);
    let n = ((secs * sr) as usize).next_power_of_two();
    let seed = 0x5EED_0000 ^ ((layer as u32) << 8) ^ sound as u32;
    let tables = notes.iter().enumerate().map(|(k, &note)| render_table(recipe, note, n, sr, seed.wrapping_add(k as u32 * 7919)));
    Box::new(Bank { sound, tonal: recipe.tonal, life: recipe.life, tables: tables.collect() })
}

/// One loop of `n` samples (a power of two) of a recipe, on a note.
pub fn render_table(recipe: &Recipe, note: f32, n: usize, sr: f32, seed: u32) -> Table {
    let mut rng = Rng::new(seed.max(1));
    let df = sr / n as f32;
    // The partials stand on whole bins, so the note is the one that fits:
    // even, so that a fifth and a sub land on whole bins too.
    let nominal = if recipe.tonal { note } else { recipe.pitch };
    let bin0 = (2.0 * (note_hz(nominal) / df / 2.0).round()).max(2.0);
    let f0 = bin0 * df;
    let limit = (sr * 0.5 / if recipe.tonal { HEADROOM } else { 1.1 }).min(sr * 0.48);
    let power = spectrum(recipe, f0, df, limit, n, &mut rng);

    // The two sides of the spectrum get phases of their own: one FFT, two
    // uncorrelated channels.
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];
    for (b, &p) in power.iter().enumerate().take(n / 2).skip(1) {
        let a = p.sqrt();
        if a > 0.0 {
            let first = TAU * rng.unit();
            // A line standing alone would come out on the two channels in
            // the proportion of a random angle, perhaps on one only. Its
            // phases are tied so that it stands in the middle, on both.
            let neighbours = power[b - 1] + power.get(b + 1).copied().unwrap_or(0.0);
            let second = if p > LINE * neighbours { std::f32::consts::FRAC_PI_2 - first } else { TAU * rng.unit() };
            let (s, c) = first.sin_cos();
            re[b] = a * c;
            im[b] = a * s;
            let (s, c) = second.sin_cos();
            re[n - b] = a * c;
            im[n - b] = a * s;
        }
    }
    inverse_fft(&mut re, &mut im);
    let mut ch = [re, im];
    scale_to(&mut ch, LOOP_RMS);

    if recipe.grit > 0.0 {
        let g = 1.0 + 14.0 * recipe.grit * recipe.grit;
        for x in ch.iter_mut().flatten() {
            *x = (*x * g).tanh();
        }
        scale_to(&mut ch, LOOP_RMS);
    }
    let (lo, hi, sweeps, mix) = recipe.sweep;
    if mix > 0.0 {
        sweep(&mut ch[0], lo, hi, sweeps, mix, 0.0, sr);
        sweep(&mut ch[1], lo, hi, sweeps, mix, 0.17, sr);
        scale_to(&mut ch, LOOP_RMS);
    }
    if recipe.swell > 0.0 {
        swell(&mut ch[0], recipe.swell, recipe.cycles, &mut rng);
        swell(&mut ch[1], recipe.swell, recipe.cycles, &mut rng);
        scale_to(&mut ch, LOOP_RMS);
    }
    if recipe.dust > 0.0 {
        dust(&mut ch, recipe.dust, sr, &mut rng);
    }
    if recipe.chime > 0.0 {
        chimes(&mut ch, recipe.chime, recipe.ring, f0, sr, &mut rng);
    }
    let [left, right] = ch;
    let base = 69.0 + 12.0 * (f0 / 440.0).log2();
    Table { left: left.into_boxed_slice(), right: right.into_boxed_slice(), mask: n - 1, base }
}

/// The shape over everything, at `f` Hz.
fn envelope(r: &Recipe, f: f32) -> f32 {
    if f <= 0.0 {
        return 0.0;
    }
    let lo = (r.low / f).powi(4);
    let hi = (f / r.high).powi(4);
    let corners = 1.0 / ((1.0 + lo) * (1.0 + hi)).sqrt();
    if r.formants.iter().all(|&(_, _, g)| g <= 0.0) {
        return corners;
    }
    let mut shape = 0.12;
    for &(fc, width, g) in &r.formants {
        if g > 0.0 && fc > 0.0 {
            let x = (f / fc).log2() / width;
            shape += g * (-x * x).exp();
        }
    }
    corners * shape
}

/// The power in each bin, from 0 up to half the rate: the partials and the
/// noise, each of known total power, so the recipe's levels mean what they
/// say.
fn spectrum(r: &Recipe, f0: f32, df: f32, limit: f32, n: usize, rng: &mut Rng) -> Vec<f32> {
    let mut power = vec![0.0f32; n / 2];
    if r.partials > 0.0 {
        let mut tone = vec![0.0f32; n / 2];
        family(r, f0, 1.0, 1.0, df, limit, &mut tone, rng);
        if r.fifth > 0.0 {
            family(r, f0 * 1.5, r.fifth, 1.0, df, limit, &mut tone, rng);
        }
        if r.sub > 0.0 {
            // Only its lowest partials: a sub isn't a second timbre.
            family(r, f0 * 0.5, r.sub, 0.0, df, (f0 * 2.0).min(limit), &mut tone, rng);
        }
        add_scaled(&mut power, &tone, r.partials);
    }
    if r.noise > 0.0 {
        let mut hiss = vec![0.0f32; n / 2];
        for (b, h) in hiss.iter_mut().enumerate().skip(1) {
            let f = b as f32 * df;
            if f < limit {
                let a = (f / 1000.0).powf(r.slope / 6.02) * envelope(r, f);
                *h = a * a;
            }
        }
        add_scaled(&mut power, &hiss, r.noise);
    }
    power
}

/// Adds `src` scaled to total power `level²`.
fn add_scaled(power: &mut [f32], src: &[f32], level: f32) {
    let total: f64 = src.iter().map(|&p| p as f64).sum();
    if total > 0.0 {
        let k = (level as f64 * level as f64 / total) as f32;
        for (p, s) in power.iter_mut().zip(src) {
            *p += k * s;
        }
    }
}

/// One family of partials on `f0`, added into `tone`. `shaped`: 1 follows the
/// recipe's formants and corners, 0 stands apart from them.
#[allow(clippy::too_many_arguments)]
fn family(r: &Recipe, f0: f32, level: f32, shaped: f32, df: f32, limit: f32, tone: &mut [f32], rng: &mut Rng) {
    let bins = tone.len() as f32;
    let mut k = 1.0f32;
    while k < 1024.0 {
        let place = k.powf(1.0 + r.stretch) * (1.0 + r.scatter * rng.bipolar());
        let f = f0 * place;
        if f >= limit {
            break;
        }
        let mut a = level * k.powf(-r.tilt);
        if r.odd > 0.0 && (k as u32).is_multiple_of(2) {
            a *= 1.0 - r.odd;
        }
        if r.pulse > 0.0 {
            a *= (std::f32::consts::PI * k * r.pulse).sin().abs();
        }
        a *= if shaped > 0.0 { envelope(r, f) } else { 1.0 };
        let cents = (r.width * k.powf(r.grow)).min(400.0);
        let sigma = f * ((cents / 1200.0).exp2() - 1.0) / df;
        let center = f / df;
        if sigma < 0.6 {
            // Narrower than a bin: a single line, on the bin.
            let b = center.round();
            if b >= 1.0 && b < bins {
                tone[b as usize] += a * a;
            }
        } else {
            // A Gaussian band of the partial's power.
            let reach = (3.0 * sigma).ceil();
            let norm = a * a / (sigma * std::f32::consts::PI.sqrt());
            let from = (center - reach).max(1.0) as usize;
            let to = ((center + reach) as usize).min(tone.len() - 1);
            let inv = 1.0 / (sigma * sigma);
            for (b, t) in tone.iter_mut().enumerate().take(to + 1).skip(from) {
                let x = b as f32 - center;
                *t += norm * (-x * x * inv).exp();
            }
        }
        k += 1.0;
    }
}

/// A plain radix-2 inverse FFT, in place (not normalized).
fn inverse_fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let twiddles: Vec<(f32, f32)> = (0..n / 2)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            (a.cos() as f32, a.sin() as f32)
        })
        .collect();
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (wr, wi) = twiddles[k * stride];
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * wr - im[b] * wi;
                let ti = re[b] * wi + im[b] * wr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

fn scale_to(ch: &mut [Vec<f32>; 2], rms: f32) {
    let energy: f64 = ch.iter().flatten().map(|&x| x as f64 * x as f64).sum();
    let count = (ch[0].len() + ch[1].len()) as f64;
    let now = (energy / count).sqrt() as f32;
    if now > 1e-12 {
        let k = rms / now;
        for x in ch.iter_mut().flatten() {
            *x *= k;
        }
    }
}

/// Slow swells, a few sines on whole cycles of the loop: no seam.
fn swell(x: &mut [f32], depth: f32, cycles: f32, rng: &mut Rng) {
    let n = x.len() as f32;
    let rates = [cycles.max(1.0).round(), (cycles * 2.0).max(2.0).round() + 1.0, (cycles * 3.0).max(3.0).round() + 2.0];
    let phases = [rng.unit(), rng.unit(), rng.unit()];
    for (i, s) in x.iter_mut().enumerate() {
        let t = i as f32 / n;
        let mut w = 0.0;
        for (k, gain) in [0.6, 0.27, 0.13].into_iter().enumerate() {
            w += gain * (TAU * (rates[k] * t + phases[k])).sin();
        }
        *s *= 1.0 - depth * (0.5 + 0.5 * w);
    }
}

/// A band sweeping to and fro, on whole cycles of the loop. A filter's state
/// isn't periodic until it has been round once, so it runs round twice and
/// only the second pass is kept: no seam.
fn sweep(x: &mut [f32], lo: f32, hi: f32, sweeps: f32, mix: f32, phase: f32, sr: f32) {
    let n = x.len();
    let (mut ic1, mut ic2) = (0.0f32, 0.0f32);
    let (mut a1, mut a2, mut a3) = (0.0, 0.0, 0.0);
    let k = 0.8;
    let octaves = (hi / lo).log2();
    let dry: Vec<f32> = x.to_vec();
    for pass in 0..2 {
        for (i, &d) in dry.iter().enumerate() {
            if i % 64 == 0 {
                let t = i as f32 / n as f32;
                let m = 0.5 + 0.5 * (TAU * (sweeps.round().max(1.0) * t + phase)).sin();
                let g = (std::f32::consts::PI * (lo * (m * octaves).exp2() / sr).min(0.23)).tan();
                a1 = 1.0 / (1.0 + g * (g + k));
                a2 = g * a1;
                a3 = g * a2;
            }
            let v3 = d - ic2;
            let v1 = a1 * ic1 + a2 * v3;
            let v2 = ic2 + a2 * ic1 + a3 * v3;
            ic1 = 2.0 * v1 - ic1;
            ic2 = 2.0 * v2 - ic2;
            if pass == 1 {
                x[i] = d * (1.0 - mix) + v1 * mix;
            }
        }
    }
}

/// Clicks and crackles: short, bright, wrapping round the loop.
fn dust(ch: &mut [Vec<f32>; 2], rate: f32, sr: f32, rng: &mut Rng) {
    let n = ch[0].len();
    let count = (rate * n as f32 / sr) as usize;
    for _ in 0..count {
        let at = (rng.unit() * n as f32) as usize;
        let amp = 1.8 * rng.unit().powi(3) + 0.05;
        let tau = (0.0003 + 0.002 * rng.unit()) * sr;
        let pan = rng.unit();
        let len = (tau * 5.0) as usize + 1;
        for i in 0..len {
            let s = rng.bipolar() * (-(i as f32) / tau).exp() * amp;
            let j = (at + i) & (n - 1);
            ch[0][j] += s * (1.0 - pan).sqrt();
            ch[1][j] += s * pan.sqrt();
        }
    }
}

/// Struck, ringing notes on harmonics of the pitch, so that they stay in tune
/// when the loop follows a chord.
fn chimes(ch: &mut [Vec<f32>; 2], rate: f32, ring: f32, f0: f32, sr: f32, rng: &mut Rng) {
    const HARMONICS: [f32; 8] = [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0];
    let n = ch[0].len();
    let count = ((rate * n as f32 / sr).round() as usize).max(1);
    let top = sr * 0.3;
    for _ in 0..count {
        let at = (rng.unit() * n as f32) as usize;
        let mut hz = f0 * HARMONICS[(rng.unit() * 8.0) as usize % 8];
        while hz > top {
            hz *= 0.5;
        }
        let tau = ring * (0.4 + rng.unit()) * sr;
        let amp = 0.25 + 0.5 * rng.unit() * rng.unit();
        let pan = rng.unit();
        let (gl, gr) = ((1.0 - pan).sqrt(), pan.sqrt());
        let len = ((tau * 6.0) as usize).min(n);
        let rise = (0.002 * sr).max(1.0);
        let (rs, rc) = (TAU * hz / sr).sin_cos();
        let (mut s, mut c) = (0.0f32, 1.0f32);
        let fall = (-1.0 / tau).exp();
        let mut env = amp;
        for i in 0..len {
            let y = s * env * (i as f32 / rise).min(1.0);
            let j = (at + i) & (n - 1);
            ch[0][j] += y * gl;
            ch[1][j] += y * gr;
            (s, c) = (s * rc + c * rs, c * rc - s * rs);
            env *= fall;
        }
    }
}

/// A bank on its way, or on its way out.
type Slot = Mutex<Option<Box<Bank>>>;

struct Shared {
    stop: AtomicBool,
    /// A finished bank is waiting in `ready` (checked first: cheaper than a
    /// lock, every block).
    fresh: [AtomicBool; SAMPLED],
    ready: [Slot; SAMPLED],
    retired: [Slot; SAMPLED],
}

/// The node's rendering thread, and its mailboxes.
pub struct Library {
    shared: Arc<Shared>,
    thread: Option<std::thread::Thread>,
}

impl Library {
    /// Starts the thread, which renders what the node's settings ask for.
    pub fn start(params: Arc<ModuleParams>, sr: f32) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            fresh: std::array::from_fn(|_| AtomicBool::new(false)),
            ready: std::array::from_fn(|_| Mutex::new(None)),
            retired: std::array::from_fn(|_| Mutex::new(None)),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new().name("vesper-sounds".into()).spawn(move || run(&worker, &params, sr)).ok().map(|h| h.thread().clone());
        Self { shared, thread }
    }

    /// A finished bank for a layer, if there is one. Never waits.
    pub fn take(&self, layer: usize) -> Option<Box<Bank>> {
        if !self.shared.fresh[layer].load(Ordering::Acquire) {
            return None;
        }
        // Under the lock, so that a bank arriving now isn't marked as taken.
        let mut slot = self.shared.ready[layer].try_lock().ok()?;
        self.shared.fresh[layer].store(false, Ordering::Release);
        slot.take()
    }

    /// Hands a bank back to be freed off the audio thread. Never waits: if
    /// the mailbox is busy, the caller keeps it and tries again.
    pub fn retire(&self, layer: usize, bank: Box<Bank>) -> Result<(), Box<Bank>> {
        match self.shared.retired[layer].try_lock() {
            Ok(mut slot) if slot.is_none() => {
                *slot = Some(bank);
                Ok(())
            }
            _ => Err(bank),
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(t) = &self.thread {
            t.unpark();
        }
    }
}

#[allow(clippy::needless_range_loop)] // a layer's slot in several arrays
fn run(shared: &Shared, params: &ModuleParams, sr: f32) {
    let mut rendered = [usize::MAX; SAMPLED];
    while !shared.stop.load(Ordering::Acquire) {
        for layer in 0..SAMPLED {
            if shared.stop.load(Ordering::Acquire) {
                return;
            }
            let wanted = super::sound(params, layer);
            if wanted != rendered[layer] {
                let bank = render(layer, wanted, sr);
                rendered[layer] = wanted;
                // An older bank nobody took goes when this scope ends, past
                // the lock.
                let _old = shared.ready[layer].lock().map(|mut slot| {
                    shared.fresh[layer].store(true, Ordering::Release);
                    slot.replace(bank)
                });
            }
            let _gone = shared.retired[layer].lock().map(|mut slot| slot.take());
        }
        std::thread::park_timeout(Duration::from_millis(25));
    }
}

