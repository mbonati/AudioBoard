//! Signal processors for each node type.

use std::f32::consts::{FRAC_PI_4, TAU};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::engine::{BLOCK, Buf, Inputs, ProcessCtx, Processor};
use super::fastmath;
use super::params::{MIXER_CHANNELS, MixerParams, OscParams, Waveform, fader_gain, osc};

/// Band-limited step correction (PolyBLEP) for the discontinuities of
/// saw and pulse waves; removes most of the audible aliasing.
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[inline]
pub fn drive(x: f32, amount: f32) -> f32 {
    if amount < 0.001 {
        x
    } else {
        let k = 1.0 + amount * 9.0;
        fastmath::tanh(k * x) / fastmath::tanh(k)
    }
}

/// Naive (non band-limited) waveform used by the UI for previews.
pub fn preview_sample(wave: Waveform, shape: f32, t: f32, noise_seed: u32) -> f32 {
    let t = t - t.floor();
    match wave {
        Waveform::Sine => drive((t * TAU).sin(), shape),
        Waveform::Triangle => triangle(t, shape),
        Waveform::Saw => drive(2.0 * t - 1.0, shape),
        Waveform::Square => {
            if t < pulse_width(shape) {
                1.0
            } else {
                -1.0
            }
        }
        Waveform::Noise => {
            let mut x = ((t * 97.0) as u32)
                .wrapping_mul(2_654_435_761)
                .wrapping_add(noise_seed.wrapping_mul(40_503));
            x ^= x >> 15;
            x = x.wrapping_mul(2_246_822_519);
            x ^= x >> 13;
            let white = (x & 0xffff) as f32 / 32_767.5 - 1.0;
            white * (1.0 - 0.6 * shape)
        }
    }
}

fn triangle(t: f32, shape: f32) -> f32 {
    let peak = 0.5 + 0.49 * shape;
    if t < peak {
        -1.0 + 2.0 * t / peak
    } else {
        1.0 - 2.0 * (t - peak) / (1.0 - peak)
    }
}

fn pulse_width(shape: f32) -> f32 {
    0.5 - 0.45 * shape
}

pub struct Oscillator {
    params: Arc<OscParams>,
    phase: f64,
    freq: f32,
    level: f32,
    rng: u32,
    noise_lp: f32,
    /// Fades the output in on Start and out on Stop.
    run: f32,
}

impl Oscillator {
    pub fn new(params: Arc<OscParams>) -> Self {
        let freq = params.get(osc::FREQ);
        let level = params.get(osc::LEVEL);
        Self {
            params,
            phase: 0.0,
            freq,
            level,
            rng: 0x9E37_79B9,
            noise_lp: 0.0,
            run: 0.0,
        }
    }

    fn white(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn sample(&mut self, wave: Waveform, shape: f32, dt: f32) -> f32 {
        let t = self.phase as f32;
        let adt = dt.abs().max(1e-9);
        match wave {
            Waveform::Sine => drive(fastmath::sin_turns(t), shape),
            Waveform::Triangle => triangle(t, shape),
            Waveform::Saw => drive(2.0 * t - 1.0 - poly_blep(t, adt), shape),
            Waveform::Square => {
                let pw = pulse_width(shape);
                let naive = if t < pw { 1.0 } else { -1.0 };
                let t2 = (t - pw + 1.0).fract();
                naive + poly_blep(t, adt) - poly_blep(t2, adt)
            }
            Waveform::Noise => {
                // One-pole low-pass for "colour", with rough RMS compensation.
                let a = 1.0 - shape * 0.985;
                let w = self.white();
                self.noise_lp += a * (w - self.noise_lp);
                (self.noise_lp * ((2.0 - a) / a).sqrt() * 0.6).clamp(-1.0, 1.0)
            }
        }
    }
}

impl Processor for Oscillator {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let wave = p.waveform();
        let freq_target = p.get(osc::FREQ) * (p.get(osc::FINE) / 1200.0).exp2();
        let level_target = p.get(osc::LEVEL);
        let shape = p.get(osc::SHAPE);
        let fm_amount = p.get(osc::FM);
        let am_amount = p.get(osc::AM);
        let fm_in = inputs.cv(0);
        let am_in = inputs.cv(1);
        let sr = ctx.sample_rate;
        // Sources sound only while the transport plays.
        let run_t = if ctx.playing { 1.0 } else { 0.0 };

        let mut peak = 0.0f32;
        for i in 0..n {
            self.freq += (freq_target - self.freq) * ctx.smooth;
            self.level += (level_target - self.level) * ctx.smooth;

            // Linear (through-zero) FM: the modulator scales the frequency.
            let mut f = self.freq;
            if let Some(fm) = &fm_in {
                f *= 1.0 + fm_amount * 4.0 * fm[i];
            }
            let dt = (f / sr).clamp(-0.49, 0.49);

            let s = self.sample(wave, shape, dt);
            self.phase += dt as f64;
            self.phase -= self.phase.floor();

            let mut gain = self.level;
            if let Some(am) = &am_in {
                gain *= 1.0 - am_amount * 0.5 * (1.0 - am[i].clamp(-1.0, 1.0));
            }

            self.run += (run_t - self.run) * ctx.smooth;
            let y = s * gain * self.run;
            outputs[0][0][i] = y;
            outputs[0][1][i] = y;
            peak = peak.max(y.abs());
        }
        self.params.meter.fetch_max(peak);
    }
}

pub struct Mixer {
    params: Arc<MixerParams>,
    gain: [f32; MIXER_CHANNELS],
    pan_l: [f32; MIXER_CHANNELS],
    pan_r: [f32; MIXER_CHANNELS],
    master: f32,
}

impl Mixer {
    pub fn new(params: Arc<MixerParams>) -> Self {
        Self {
            params,
            gain: [0.0; MIXER_CHANNELS],
            pan_l: [FRAC_PI_4.cos(); MIXER_CHANNELS],
            pan_r: [FRAC_PI_4.sin(); MIXER_CHANNELS],
            master: 0.0,
        }
    }
}

impl Processor for Mixer {
    #[allow(clippy::needless_range_loop)] // indexes several per-channel arrays in lockstep
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let any_solo = p.solo.iter().any(|s| s.load(Ordering::Relaxed));

        let mut gain_t = [0.0; MIXER_CHANNELS];
        let mut pan_lt = [0.0; MIXER_CHANNELS];
        let mut pan_rt = [0.0; MIXER_CHANNELS];
        for ch in 0..MIXER_CHANNELS {
            let muted = p.mute[ch].load(Ordering::Relaxed);
            let soloed = p.solo[ch].load(Ordering::Relaxed);
            let audible = !muted && (!any_solo || soloed);
            gain_t[ch] = if audible { fader_gain(p.fader[ch].get()) } else { 0.0 };
            // Constant-power pan law (-3 dB at centre).
            let theta = (p.pan[ch].get().clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
            pan_lt[ch] = theta.cos();
            pan_rt[ch] = theta.sin();
        }
        let master_t = fader_gain(p.master.get());

        let mut ch_peak = [0.0f32; MIXER_CHANNELS];
        let mut out_peak = [0.0f32; 2];
        let k = ctx.smooth;

        // One channel at a time across the block; unpatched channels are
        // skipped and their smoothing jumps to its target.
        let mut sum = [[0.0f32; BLOCK]; 2];
        for ch in 0..MIXER_CHANNELS {
            let Some([xl, xr]) = inputs.get(ch) else {
                self.gain[ch] = gain_t[ch];
                self.pan_l[ch] = pan_lt[ch];
                self.pan_r[ch] = pan_rt[ch];
                continue;
            };
            let (mut gain, mut pan_l, mut pan_r) = (self.gain[ch], self.pan_l[ch], self.pan_r[ch]);
            let mut peak = 0.0f32;
            for i in 0..n {
                gain += (gain_t[ch] - gain) * k;
                pan_l += (pan_lt[ch] - pan_l) * k;
                pan_r += (pan_rt[ch] - pan_r) * k;
                // Stereo inputs: the pan acts as a balance control.
                let (l, r) = (xl[i] * gain, xr[i] * gain);
                peak = peak.max(l.abs()).max(r.abs());
                sum[0][i] += l * pan_l;
                sum[1][i] += r * pan_r;
            }
            (self.gain[ch], self.pan_l[ch], self.pan_r[ch]) = (gain, pan_l, pan_r);
            ch_peak[ch] = peak;
        }
        for i in 0..n {
            self.master += (master_t - self.master) * k;
            let (l, r) = (sum[0][i] * self.master, sum[1][i] * self.master);
            outputs[0][0][i] = l;
            outputs[0][1][i] = r;
            out_peak[0] = out_peak[0].max(l.abs());
            out_peak[1] = out_peak[1].max(r.abs());
        }

        for (meter, peak) in p.ch_meter.iter().zip(ch_peak) {
            meter.fetch_max(peak);
        }
        for (meter, peak) in p.master_meter.iter().zip(out_peak) {
            meter.fetch_max(peak);
        }
    }

    fn is_sink(&self) -> bool {
        true
    }
}
