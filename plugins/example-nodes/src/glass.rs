//! Glass: a small FM instrument played from its MIDI input. Eight voices,
//! each a sine carrier (detuned left and right) phase-modulated by one
//! sine, with a struck envelope. The display draws one cycle of the
//! current timbre; the readout names the last note played.

use audioboard_node::prelude::*;

const TONE: usize = 0;
const RATIO: usize = 1;
const DECAY: usize = 2;
const DETUNE: usize = 3;
const LEVEL: usize = 4;

const PARAMS: &[ParamSpec] = &[
    ParamSpec::lin("TONE", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("RATIO", 0.5, 8.0, 2.0, Unit::Plain),
    ParamSpec::log("DECAY", 0.05, 8.0, 1.2, Unit::Seconds),
    ParamSpec::lin("DETUNE", 0.0, 50.0, 6.0, Unit::Cents),
    ParamSpec::lin("LEVEL", 0.0, 1.0, 0.6, Unit::Decibel),
];

const MODES: &[&str] = &["SINE", "BELL", "REED"];

/// Display values: one cycle of the timbre, then the last note.
const CYCLE: usize = 32;
const LAST_NOTE: usize = CYCLE;

const VOICES: usize = 8;

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    hz: f32,
    /// Carrier phases (left, right) and the modulator's, in turns.
    carrier: [f32; 2],
    modulator: f32,
    env: f32,
    held: bool,
}

pub struct Glass {
    params: Shared,
    sample_rate: f32,
    voices: [Voice; VOICES],
    level: Smoothed,
}

/// (modulation index scale, ratio offset) for each mode.
fn mode_shape(mode: usize) -> (f32, f32) {
    match mode {
        1 => (3.0, 0.414),
        2 => (6.0, -0.5),
        _ => (1.0, 0.0),
    }
}

impl Glass {
    fn note_on(&mut self, note: u8, velocity: u8) {
        // A free voice, else the same note again, else the quietest.
        let i = self
            .voices
            .iter()
            .position(|v| v.env < 1e-4)
            .or_else(|| self.voices.iter().position(|v| v.note == note))
            .unwrap_or_else(|| {
                (0..VOICES)
                    .min_by(|&a, &b| self.voices[a].env.total_cmp(&self.voices[b].env))
                    .unwrap_or(0)
            });
        self.voices[i] = Voice {
            note,
            hz: dsp::note_hz(note as f32),
            env: velocity as f32 / 127.0,
            held: true,
            ..Voice::default()
        };
        self.params.set_viz(LAST_NOTE, note as f32);
    }

    fn note_off(&mut self, note: u8) {
        for v in self.voices.iter_mut().filter(|v| v.note == note) {
            v.held = false;
        }
    }
}

impl Node for Glass {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "dev.audioboard.example.glass",
        maker: "AudioBoard Examples",
        def: NodeDef {
            name: "Glass",
            prefix: "GLASS",
            hint: "FM voices · MIDI · plugin",
            outputs: &["OUT"],
            params: PARAMS,
            modes: MODES,
            midi_in: true,
            display_h: 44.0,
            viz: CYCLE + 1,
            data: &[],
            ..NodeDef::EMPTY
        },
        panel: Panel {
            display: Display::Curve {
                from: 0,
                count: CYCLE,
            },
            readout: Readout::Viz {
                index: LAST_NOTE,
                unit: Unit::Note,
            },
        },
    };

    fn new(init: &Init) -> Self {
        init.params.set_viz(LAST_NOTE, 60.0);
        Glass {
            params: init.params,
            sample_rate: init.sample_rate,
            voices: [Voice::default(); VOICES],
            level: Smoothed::new(0.0),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let p = self.params;
        let (scale, offset) = mode_shape(p.mode());
        let index = p.get(TONE) * 4.0 * scale;
        let ratio = (p.get(RATIO) + offset).max(0.25);
        let decay = dsp::time_coeff(p.get(DECAY), self.sample_rate);
        let release = dsp::time_coeff(0.15, self.sample_rate);
        let spread = fastmath::exp2(p.get(DETUNE) / 2400.0);
        let level = p.get(LEVEL);

        // Events at their frames: render up to each, then apply it.
        let mut events = [MidiMsg::default(); 128];
        let count = io.midi().len().min(events.len());
        events[..count].copy_from_slice(&io.midi()[..count]);
        let mut next = 0;
        let out = io.output(0);
        for i in 0..n {
            while next < count && events[next].frame as usize <= i {
                let e = events[next];
                if let Some((note, velocity)) = e.note_on() {
                    self.note_on(note, velocity);
                } else if let Some(note) = e.note_off() {
                    self.note_off(note);
                }
                next += 1;
            }
            let g = self.level.next(level, ctx.smooth);
            let mut lr = [0.0f32; 2];
            for v in self.voices.iter_mut().filter(|v| v.env > 1e-5) {
                let m = fastmath::sin_turns(v.modulator) * index;
                v.modulator = (v.modulator + v.hz * ratio / self.sample_rate).fract();
                for (c, side) in lr.iter_mut().enumerate() {
                    let detune = if c == 0 { 1.0 / spread } else { spread };
                    *side += fastmath::sin_turns(v.carrier[c] + m * 0.159) * v.env;
                    v.carrier[c] = (v.carrier[c] + v.hz * detune / self.sample_rate).fract();
                }
                v.env -= v.env * if v.held { decay } else { release.max(decay) };
            }
            out[0][i] = dsp::soft_limit(lr[0] * 0.3 * g);
            out[1][i] = dsp::soft_limit(lr[1] * 0.3 * g);
        }
        for e in &events[next..count] {
            if let Some((note, velocity)) = e.note_on() {
                self.note_on(note, velocity);
            } else if let Some(note) = e.note_off() {
                self.note_off(note);
            }
        }

        // One cycle of the timbre, for the display.
        for k in 0..CYCLE {
            let t = k as f32 / CYCLE as f32;
            let m = fastmath::sin_turns(t * ratio) * index;
            p.set_viz(k, fastmath::sin_turns(t + m * 0.159));
        }
    }
}
