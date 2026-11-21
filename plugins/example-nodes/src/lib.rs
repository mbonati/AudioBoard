//! An example AudioBoard node plugin: three nodes written against the node
//! SDK (`audioboard-node`) and exported from one dynamic library.
//!
//! - **Glass** (source): FM voices played from the MIDI input.
//! - **Haze** (effect): a stereo chorus with feedback.
//! - **Pulse** (MIDI): a Euclidean rhythm on the MIDI output.
//!
//! Build and install (macOS):
//!
//! ```text
//! cargo build --release -p audioboard-example-nodes
//! mkdir -p ~/Library/Application\ Support/AudioBoard/Nodes
//! cp target/release/libaudioboard_example_nodes.dylib ~/Library/Application\ Support/AudioBoard/Nodes/
//! ```
//!
//! Then Rescan in the palette's Plugin nodes section (or restart).

mod glass;
mod haze;
mod pulse;

pub use glass::Glass;
pub use haze::Haze;
pub use pulse::{Pulse, hit};

audioboard_node::export_nodes! {
    name: "Example nodes",
    maker: "AudioBoard Examples",
    version: env!("CARGO_PKG_VERSION"),
    nodes: [Glass, Haze, Pulse],
}

#[cfg(test)]
mod tests {
    use super::*;
    use audioboard_node::node::{Init, Io};
    use audioboard_node::{BLOCK, Buf, MidiMsg, Node, Params, ProcessCtx, Shared};

    fn ctx(beat: f64, playing: bool) -> ProcessCtx {
        ProcessCtx {
            sample_rate: 48_000.0,
            smooth: 0.01,
            beat,
            beats_per_sample: 2.0 / 48_000.0,
            playing,
            restart: false,
        }
    }

    fn make<N: Node>(params: &Params) -> N {
        // SAFETY: `params` outlives the node in each test.
        N::new(&Init {
            sample_rate: 48_000.0,
            params: unsafe { Shared::from_raw(params.raw()) },
        })
    }

    #[test]
    fn descriptions_are_valid() {
        for d in [Glass::DESCRIPTOR, Haze::DESCRIPTOR, Pulse::DESCRIPTOR] {
            assert_eq!(d.problem(), None, "{}", d.id);
        }
    }

    #[test]
    fn euclid_spreads_hits() {
        let pattern: Vec<bool> = (0..8).map(|i| hit(i, 8, 3)).collect();
        assert_eq!(pattern.iter().filter(|h| **h).count(), 3);
        assert!(!hit(0, 8, 0));
        assert_eq!((0..16).filter(|&i| hit(i, 16, 16)).count(), 16);
    }

    #[test]
    fn glass_plays_a_note() {
        let params = Params::new(&Glass::DESCRIPTOR.def);
        let mut g: Glass = make(&params);
        let mut out: [Buf; 1] = [[[0.0; BLOCK]; 2]];
        let midi = [MidiMsg {
            frame: 0,
            data: [0x90, 69, 100],
        }];
        let mut room = [MidiMsg::default(); 8];
        let mut peak = 0.0f32;
        for b in 0..40 {
            let input: &[MidiMsg] = if b == 0 { &midi } else { &[] };
            let mut io = Io::new(BLOCK, &[], &[], &mut out, input, &mut room);
            g.process(&ctx(0.0, true), &mut io);
            peak = out[0][0].iter().fold(peak, |m, s| m.max(s.abs()));
        }
        assert!(peak > 0.05 && peak <= 1.0, "peak {peak}");
        assert_eq!(params.viz[32].get(), 69.0);
    }

    #[test]
    fn haze_passes_and_colours_audio() {
        let params = Params::new(&Haze::DESCRIPTOR.def);
        let mut h: Haze = make(&params);
        let input: [Buf; 1] = [[[0.5; BLOCK]; 2]];
        let mut out: [Buf; 1] = [[[0.0; BLOCK]; 2]];
        let mut room = [];
        for _ in 0..100 {
            let mut io = Io::new(BLOCK, &input, &[true], &mut out, &[], &mut room);
            h.process(&ctx(0.0, true), &mut io);
        }
        assert!(out[0][0].iter().all(|s| s.is_finite() && s.abs() < 2.0));
        assert!(out[0][0][10] > 0.2);
    }

    #[test]
    fn pulse_sends_notes_on_the_beat() {
        let params = Params::new(&Pulse::DESCRIPTOR.def);
        let mut p: Pulse = make(&params);
        let mut out: [Buf; 0] = [];
        let mut ons = 0;
        let mut offs = 0;
        let mut beat = 0.0;
        // Two beats of 1/16 steps: 8 steps, 3 hits.
        for _ in 0..(48_000 / BLOCK) {
            let mut room = [MidiMsg::default(); 16];
            let mut io = Io::new(BLOCK, &[], &[], &mut out, &[], &mut room);
            p.process(&ctx(beat, true), &mut io);
            ons += io
                .midi_sent()
                .iter()
                .filter(|m| m.note_on().is_some())
                .count();
            offs += io
                .midi_sent()
                .iter()
                .filter(|m| m.note_off().is_some())
                .count();
            beat += 2.0 / 48_000.0 * BLOCK as f64;
        }
        assert_eq!(ons, 3, "one bar of 8 sixteenths with 3 hits");
        assert!(offs >= 2);
    }
}
