use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::pos2;

use crate::audio::engine::Engine;
use crate::audio::params::{EngineShared, MixerParams, OscParams, Waveform, osc};
use crate::audio::{self, AudioHandle};
use crate::graph::{Graph, NodeKind, PortRef};

fn setup() -> (Graph, AudioHandle, Engine) {
    let (handle, engine) = audio::offline(Arc::new(EngineShared::new()), 48_000);
    (Graph::default(), handle, engine)
}

fn port(node: u32, port: usize) -> PortRef {
    PortRef { node, port }
}

fn osc_params(g: &Graph, id: u32) -> Arc<OscParams> {
    match &g.node(id).unwrap().kind {
        NodeKind::Osc(o) => o.params.clone(),
        _ => panic!("not an oscillator"),
    }
}

fn mixer_params(g: &Graph, id: u32) -> Arc<MixerParams> {
    match &g.node(id).unwrap().kind {
        NodeKind::Mixer(m) => m.params.clone(),
        _ => panic!("not a mixer"),
    }
}

/// Renders `frames` stereo frames and returns (peak, all samples finite).
fn render(engine: &mut Engine, frames: usize) -> (f32, bool) {
    let mut buf = vec![0.0f32; frames * 2];
    // Odd-sized chunks exercise the block splitting.
    for chunk in buf.chunks_mut(2 * 300) {
        engine.render(chunk, 2);
    }
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    (peak, buf.iter().all(|s| s.is_finite()))
}

#[test]
fn rejects_feedback_loops() {
    let (mut g, a, _e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let y = g.add_osc(pos2(0.0, 0.0), &a);
    assert!(g.connect(port(x, 0), port(y, 0), &a));
    assert!(!g.connect(port(y, 0), port(x, 0), &a), "y -> x would close a loop");
    assert!(!g.connect(port(x, 0), port(x, 1), &a), "self-patching is a loop");
    assert_eq!(g.cables.len(), 1);
}

#[test]
fn an_input_holds_one_cable() {
    let (mut g, a, _e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let y = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    g.connect(port(y, 0), port(m, 0), &a);
    assert_eq!(g.cables.len(), 1);
    assert_eq!(g.cables[0].from.node, y);
}

#[test]
fn removing_a_node_drops_its_cables() {
    let (mut g, a, mut e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    g.remove_node(x, &a);
    assert!(g.cables.is_empty());
    let (peak, finite) = render(&mut e, 4800);
    assert!(finite);
    assert_eq!(peak, 0.0);
}

#[test]
fn titles_reuse_free_numbers() {
    let (mut g, a, _e) = setup();
    let first = g.add_osc(pos2(0.0, 0.0), &a);
    g.rename(first, "LFO");
    let second = g.add_osc(pos2(0.0, 0.0), &a);
    assert_eq!(g.node(second).unwrap().title, "OSC 1");
}

#[test]
fn oscillator_through_mixer_reaches_the_output() {
    let (mut g, a, mut e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);

    // Unpatched: silence.
    assert_eq!(render(&mut e, 4800).0, 0.0);

    g.connect(port(x, 0), port(m, 0), &a);
    let (peak, finite) = render(&mut e, 4800);
    assert!(finite);
    assert!(peak > 0.05 && peak <= 1.0, "peak = {peak}");

    // Mute the channel: after parameter smoothing the output is silent.
    mixer_params(&g, m).mute[0].store(true, Ordering::Relaxed);
    render(&mut e, 4800);
    let (peak, _) = render(&mut e, 4800);
    assert!(peak < 1e-3, "muted peak = {peak}");
}

#[test]
fn every_waveform_with_modulation_stays_finite_and_bounded() {
    let (mut g, a, mut e) = setup();
    let lfo = g.add_osc(pos2(0.0, 0.0), &a);
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(lfo, 0), port(x, 0), &a); // FM
    g.connect(port(lfo, 0), port(x, 1), &a); // AM
    g.connect(port(x, 0), port(m, 0), &a);

    let lp = osc_params(&g, lfo);
    lp.set(osc::FREQ, 3.0);
    lp.set(osc::LEVEL, 1.0);
    let xp = osc_params(&g, x);
    xp.set(osc::LEVEL, 1.0);
    xp.set(osc::FM, 1.0);
    xp.set(osc::AM, 1.0);

    for wave in Waveform::ALL {
        for shape in [0.0, 0.5, 1.0] {
            xp.set_waveform(wave);
            xp.set(osc::SHAPE, shape);
            xp.set(osc::FREQ, 12_000.0);
            let (peak, finite) = render(&mut e, 9600);
            assert!(finite, "{wave:?} shape {shape} produced NaN/inf");
            assert!(peak <= 1.0);
            assert!(peak > 0.0, "{wave:?} shape {shape} is silent");
        }
    }
}
