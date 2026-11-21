//! Pulse: a Euclidean rhythm on the MIDI output. HITS notes spread as evenly
//! as possible over STEPS steps, turned by ROTATE, on the transport's beat.
//! The display shows the pattern with the step playing now.

use audioboard_node::prelude::*;

const STEPS: usize = 0;
const HITS: usize = 1;
const ROTATE: usize = 2;
const NOTE: usize = 3;
const VELOCITY: usize = 4;
const GATE: usize = 5;

const PARAMS: &[ParamSpec] = &[
    ParamSpec::lin("STEPS", 2.0, 16.0, 8.0, Unit::Integer),
    ParamSpec::lin("HITS", 0.0, 16.0, 3.0, Unit::Integer),
    ParamSpec::lin("ROTATE", 0.0, 15.0, 0.0, Unit::Integer),
    ParamSpec::lin("NOTE", 24.0, 96.0, 36.0, Unit::Note),
    ParamSpec::lin("VELOCITY", 1.0, 127.0, 100.0, Unit::Integer),
    ParamSpec::lin("GATE", 0.05, 1.0, 0.5, Unit::Percent),
];

/// Step lengths: steps per beat.
const MODES: &[&str] = &["1/16", "1/8", "1/4"];
const STEPS_PER_BEAT: [f64; 3] = [4.0, 2.0, 1.0];

const MAX_STEPS: usize = 16;

/// Euclidean pattern: is step `i` of `steps` a hit?
pub fn hit(i: usize, steps: usize, hits: usize) -> bool {
    let hits = hits.min(steps);
    hits > 0 && (i * hits) % steps < hits
}

pub struct Pulse {
    params: Shared,
    /// The step last started (−1: none yet).
    step: i64,
    /// The note sounding, and the beat it ends on.
    held: Option<(u8, f64)>,
}

impl Node for Pulse {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "dev.audioboard.example.pulse",
        maker: "AudioBoard Examples",
        def: NodeDef {
            name: "Pulse",
            prefix: "PULSE",
            hint: "euclidean rhythm · plugin",
            params: PARAMS,
            modes: MODES,
            midi_out: true,
            display_h: 36.0,
            viz: MAX_STEPS,
            ..NodeDef::EMPTY
        },
        panel: Panel {
            display: Display::Bars {
                from: 0,
                count: MAX_STEPS,
            },
            readout: Readout::Param(NOTE),
        },
    };

    fn new(init: &Init) -> Self {
        Pulse {
            params: init.params,
            step: -1,
            held: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let p = self.params;
        let steps = (p.get(STEPS).round() as usize).clamp(2, MAX_STEPS);
        let hits = p.get(HITS).round().max(0.0) as usize;
        let rotate = p.get(ROTATE).round().max(0.0) as usize;
        let note = p.get(NOTE).round().clamp(0.0, 127.0) as u8;
        let velocity = p.get(VELOCITY).round().clamp(1.0, 127.0) as u8;
        let step_beats = 1.0 / STEPS_PER_BEAT[p.mode().min(MODES.len() - 1)];
        let gate = p.get(GATE) as f64 * step_beats;

        if ctx.restart || !ctx.playing {
            if let Some((held, _)) = self.held.take() {
                io.send_midi(0, [0x80, held, 0]);
            }
            self.step = -1;
        }
        if ctx.playing {
            for i in 0..n {
                let beat = ctx.beat_at(i);
                if let Some((held, end)) = self.held
                    && beat >= end
                {
                    io.send_midi(i, [0x80, held, 0]);
                    self.held = None;
                }
                let step = (beat / step_beats).floor() as i64;
                if step != self.step {
                    self.step = step;
                    let at =
                        (step.rem_euclid(steps as i64) as usize + steps - rotate % steps) % steps;
                    if hit(at, steps, hits) {
                        if let Some((held, _)) = self.held.take() {
                            io.send_midi(i, [0x80, held, 0]);
                        }
                        io.send_midi(i, [0x90, note, velocity]);
                        self.held = Some((note, step as f64 * step_beats + gate));
                    }
                }
            }
        }

        // The pattern: rests low, hits higher, the current step full.
        let now = if self.step >= 0 {
            self.step.rem_euclid(steps as i64) as usize
        } else {
            usize::MAX
        };
        for k in 0..MAX_STEPS {
            let at = (k + steps - rotate % steps) % steps;
            let v = if k >= steps {
                0.0
            } else if hit(at, steps, hits) {
                if k == now { 1.0 } else { 0.6 }
            } else if k == now {
                0.35
            } else {
                0.12
            };
            p.set_viz(k, v);
        }
    }
}
