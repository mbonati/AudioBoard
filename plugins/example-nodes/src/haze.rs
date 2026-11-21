//! Haze: a stereo chorus with feedback. Two delay lines swept by one LFO
//! (a quarter turn apart with WIDE), a tone filter in the feedback path,
//! and an equal-power dry/wet mix. Rests while its input and tail are
//! silent.

use audioboard_node::prelude::*;

const RATE: usize = 0;
const DEPTH: usize = 1;
const TIME: usize = 2;
const FEEDBACK: usize = 3;
const TONE: usize = 4;
const MIX: usize = 5;

const PARAMS: &[ParamSpec] = &[
    ParamSpec::log("RATE", 0.05, 8.0, 0.4, Unit::Hz),
    ParamSpec::lin("DEPTH", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::log("TIME", 0.003, 0.05, 0.012, Unit::Seconds),
    ParamSpec::lin("FEEDBACK", 0.0, 0.95, 0.3, Unit::Percent),
    ParamSpec::log("TONE", 500.0, 18_000.0, 6000.0, Unit::Hz),
    ParamSpec::lin("MIX", 0.0, 1.0, 0.5, Unit::Percent),
];

/// Longest delay: TIME at its top, swept up by DEPTH.
const MAX_SECONDS: f32 = 0.05 * 1.95 + 0.01;

pub struct Haze {
    params: Shared,
    sample_rate: f32,
    lines: [DelayLine; 2],
    tone: [OnePole; 2],
    phase: f32,
    smooth: [Smoothed; 6],
    quiet: Quiet,
}

impl Node for Haze {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "dev.audioboard.example.haze",
        maker: "AudioBoard Examples",
        def: NodeDef {
            name: "Haze",
            prefix: "HAZE",
            hint: "chorus · feedback · plugin",
            inputs: &["IN"],
            outputs: &["OUT"],
            params: PARAMS,
            toggle: Some("WIDE"),
            toggle_default: true,
            display_h: 44.0,
            viz: 1,
            ..NodeDef::EMPTY
        },
        panel: Panel {
            display: Display::Trace { index: 0 },
            readout: Readout::Param(RATE),
        },
    };

    fn new(init: &Init) -> Self {
        let len = (MAX_SECONDS * init.sample_rate) as usize + 4;
        let p = init.params;
        Haze {
            params: p,
            sample_rate: init.sample_rate,
            lines: [DelayLine::new(len), DelayLine::new(len)],
            tone: [OnePole::default(); 2],
            phase: 0.0,
            smooth: std::array::from_fn(|i| Smoothed::new(p.get(i))),
            quiet: Quiet::new(len),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let p = self.params;
        let silent = Quiet::silent(io.input(0), n);
        if silent && self.quiet.resting() {
            io.silence();
            return;
        }
        let wide = if p.toggled() { 0.25 } else { 0.0 };
        let sr = self.sample_rate;
        let k = ctx.smooth;
        let t: [f32; 6] = std::array::from_fn(|i| p.get(i));
        let (input, out) = io.through(0, 0);
        let mut peak = 0.0f32;
        for i in 0..n {
            let s = &mut self.smooth;
            let rate = s[RATE].next(t[RATE], k);
            let depth = s[DEPTH].next(t[DEPTH], k);
            let time = s[TIME].next(t[TIME], k);
            let feedback = s[FEEDBACK].next(t[FEEDBACK], k);
            let tone = s[TONE].next(t[TONE], k);
            let mix = s[MIX].next(t[MIX], k);
            self.phase = (self.phase + rate / sr).fract();
            let tone_k = OnePole::coeff(tone, sr);
            let (dry_g, wet_g) = dsp::mix_gains(mix);
            for c in 0..2 {
                let lfo = fastmath::sin_turns(self.phase + wide * c as f32);
                let d = time * (1.0 + 0.95 * depth * lfo) * sr;
                let wet = self.lines[c].tap(d - 1.0);
                let back = self.tone[c].lowpass(wet, tone_k) * feedback;
                let x = input.map_or(0.0, |b| b[c][i]);
                self.lines[c].push(dsp::soft_ceiling(x + back));
                let y = x * dry_g + wet * wet_g;
                out[c][i] = y;
                peak = peak.max(wet.abs());
            }
        }
        self.quiet.update(silent, peak, n);
        p.set_viz(0, fastmath::sin_turns(self.phase));
    }
}
