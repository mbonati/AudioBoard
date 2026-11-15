use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::pos2;

use crate::audio::engine::Engine;
use crate::audio::modules::{ModuleKind, ModuleParams};
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

/// A mixer's main-output peak over a settled stretch.
fn master_peak(e: &mut Engine, p: &MixerParams) -> f32 {
    render(e, 4800);
    p.master_meter.iter().for_each(|m| _ = m.take());
    render(e, 4800);
    p.master_meter.iter().map(|m| m.take()).fold(0.0, f32::max)
}

#[test]
fn mixers_send_on_aux_mute_their_master_and_patch_their_out() {
    use crate::audio::params::FADER_UNITY;
    let (mut g, a, mut e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let dry = g.add_mixer(pos2(0.0, 0.0), &a);
    let wet = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(dry, 0), &a);
    assert!(g.connect(port(dry, 1), port(wet, 0), &a), "AUX into another mixer");
    let (dp, wp) = (mixer_params(&g, dry), mixer_params(&g, wet));
    // No send: the second mixer hears nothing.
    assert!(master_peak(&mut e, &dp) > 0.05);
    assert!(master_peak(&mut e, &wp) < 1e-6);
    // A send: the channel reaches the AUX output, at its own level.
    dp.aux[0].set(FADER_UNITY);
    let wet_level = master_peak(&mut e, &wp);
    assert!(wet_level > 0.05);
    dp.aux_master.set(0.0);
    assert!(master_peak(&mut e, &wp) < 1e-4, "AUX level down");
    dp.aux_master.set(FADER_UNITY);
    // MUTE silences the main output only: the AUX layer plays on.
    dp.master_mute.store(true, Ordering::Relaxed);
    assert!(master_peak(&mut e, &dp) < 1e-4);
    assert!((master_peak(&mut e, &wp) - wet_level).abs() < 0.01);
    assert!(render(&mut e, 4800).0 > 0.05, "heard through the second mixer");
    dp.master_mute.store(false, Ordering::Relaxed);
    dp.aux[0].set(0.0);
    // OUT patched into another mixer: no longer to the device itself, so the
    // device hears it once, through the second mixer (at −6 dB).
    wp.master.set(FADER_UNITY * 0.5f32.cbrt());
    let alone = render(&mut e, 9600).0;
    assert!(g.connect(port(dry, 0), port(wet, 1), &a));
    render(&mut e, 4800);
    let through = render(&mut e, 4800).0;
    assert!(through > 0.05 && through < alone * 0.75, "{through} vs {alone}: wet's master (-6 dB) once, not twice");
}

#[test]
fn the_console_takes_24_channels_either_way_round() {
    use crate::graph::Console;
    let (mut g, a, mut e) = setup();
    let c = g.add_console(pos2(100.0, 100.0), &a);
    let n = g.node(c).unwrap();
    assert_eq!((n.inputs().len(), n.outputs()), (12, &["OUT", "AUX"][..]));
    // Horizontal: the inputs line the top edge.
    assert!(n.inputs_on_top());
    assert_eq!(n.input_pos(0).y, n.pos.y);
    assert!(n.input_pos(1).x > n.input_pos(0).x);
    // Vertical, 24 channels: down the left edge.
    g.set_console(c, Console { channels: 24, horizontal: false }, &a);
    let n = g.node(c).unwrap();
    assert_eq!(n.inputs().len(), 24);
    assert!(!n.inputs_on_top());
    assert_eq!(n.input_pos(23).x, n.pos.x);
    assert!(n.input_pos(23).y > n.input_pos(0).y && n.input_pos(23).y < n.pos.y + n.size().y);
    // Channel 20 plays.
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    assert!(g.connect(port(x, 0), port(c, 19), &a));
    assert!(render(&mut e, 4800).0 > 0.05);
    // Back to 12 unplugs it.
    let twelve = Console { channels: 12, horizontal: true };
    assert_eq!(g.console_losses(c, twelve), 1);
    g.set_console(c, twelve, &a);
    assert!(g.cables.is_empty());
    render(&mut e, 4800);
    assert!(render(&mut e, 4800).0 < 1e-6);
    // The plain Mixer can't be reshaped.
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.set_console(m, twelve, &a);
    assert!(g.node(m).unwrap().console().is_none());
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

// ---------------------------------------------------------------------------
// Ambient modules
// ---------------------------------------------------------------------------

fn module_params(g: &Graph, id: u32) -> Arc<ModuleParams> {
    match &g.node(id).unwrap().kind {
        NodeKind::Module(m) => m.params.clone(),
        _ => panic!("not a module"),
    }
}

/// Renders `frames` stereo frames and returns them interleaved.
fn render_buf(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut buf = vec![0.0f32; frames * 2];
    for chunk in buf.chunks_mut(2 * 300) {
        engine.render(chunk, 2);
    }
    buf
}

fn peak_of(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

fn rms_of(buf: &[f32]) -> f32 {
    (buf.iter().map(|s| s * s).sum::<f32>() / buf.len().max(1) as f32).sqrt()
}

/// A noisy, harmonically rich source: saw at 220 Hz.
fn source(g: &mut Graph, a: &AudioHandle) -> u32 {
    let x = g.add_osc(pos2(0.0, 0.0), a);
    let p = osc_params(g, x);
    p.set_waveform(Waveform::Saw);
    p.set(osc::FREQ, 220.0);
    p.set(osc::LEVEL, 0.5);
    x
}

/// Source → module → mixer; returns (module id, its params).
fn patch(g: &mut Graph, a: &AudioHandle, kind: ModuleKind) -> (u32, Arc<ModuleParams>) {
    let src = source(g, a);
    let m = g.add_module(kind, pos2(0.0, 0.0), a);
    let mix = g.add_mixer(pos2(0.0, 0.0), a);
    if !kind.def().inputs.is_empty() {
        g.connect(port(src, 0), port(m, 0), a);
    }
    g.connect(port(m, 0), port(mix, 0), a);
    (m, module_params(g, m))
}

/// Removes the cable feeding the module's audio input.
fn unplug_input(g: &mut Graph, a: &AudioHandle, m: u32) {
    let cable = *g.cables.iter().find(|c| c.to == port(m, 0)).expect("input cable");
    g.disconnect(cable, a);
}

#[test]
fn every_module_stays_finite_across_settings() {
    // MIDI generators make no sound of their own (tested below).
    for kind in ModuleKind::ALL.into_iter().filter(|k| !k.def().outputs.is_empty()) {
        let def = kind.def();
        let (mut g, a, mut e) = setup();
        let (m, p) = patch(&mut g, &a, kind);
        // Modulate every CV input with a fast LFO.
        let lfo = g.add_osc(pos2(0.0, 0.0), &a);
        osc_params(&g, lfo).set(osc::FREQ, 7.0);
        osc_params(&g, lfo).set(osc::LEVEL, 1.0);
        for input in 1..def.inputs.len() {
            g.connect(port(lfo, 0), port(m, input), &a);
        }

        for setting in [0.0, 0.5, 1.0] {
            for (i, spec) in def.params.iter().enumerate() {
                p.set(i, spec.denorm(setting));
            }
            for mode in 0..def.modes.len().max(1) {
                for toggle in [false, true] {
                    p.set_mode(mode);
                    p.set_toggle(toggle);
                    let buf = render_buf(&mut e, 4800);
                    assert!(
                        buf.iter().all(|s| s.is_finite()),
                        "{kind:?} at {setting} mode {mode} toggle {toggle} produced NaN/inf"
                    );
                    assert!(p.meter.take() <= 1.0, "{kind:?} left its output range");
                }
            }
        }
        // Its sound comes from the interface (see audio_in_node_reaches_the_board).
        if kind == ModuleKind::AudioIn {
            continue;
        }
        // Back to defaults it still makes sound.
        let fresh = ModuleParams::new(kind);
        for i in 0..def.params.len() {
            p.set(i, fresh.get(i));
        }
        p.set_mode(0);
        p.set_toggle(false);
        // MIDI nodes need something to play: the mod wheel and a key.
        a.midi.handle(&[0xB0, 1, 100]);
        a.midi.handle(&[0x90, 60, 100]);
        render_buf(&mut e, 48_000);
        let buf = render_buf(&mut e, 24_000);
        assert!(rms_of(&buf) > 0.005, "{kind:?} is silent at its defaults");
    }
}

#[test]
fn reverb_tail_decays_and_freeze_holds_it() {
    use crate::audio::modules::reverb;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Reverb);
    p.set(reverb::MIX, 1.0);
    p.set(reverb::DECAY, 3.0);
    render_buf(&mut e, 48_000);
    unplug_input(&mut g, &a, m);

    let early = rms_of(&render_buf(&mut e, 12_000));
    let late = rms_of(&render_buf(&mut e, 96_000)[96_000..]);
    assert!(early > 0.01, "no tail: {early}");
    assert!(late < early * 0.3, "tail does not decay: {early} → {late}");

    // Freeze while it rings: the level holds instead of decaying.
    g.connect(port(1, 0), port(m, 0), &a);
    render_buf(&mut e, 48_000);
    p.set_toggle(true);
    unplug_input(&mut g, &a, m);
    let held_a = rms_of(&render_buf(&mut e, 24_000));
    render_buf(&mut e, 96_000);
    let held_b = rms_of(&render_buf(&mut e, 24_000));
    assert!(held_b > held_a * 0.7, "freeze lost energy: {held_a} → {held_b}");
}

#[test]
fn granular_freeze_keeps_the_cloud_playing() {
    use crate::audio::modules::granular;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Granular);
    p.set(granular::MIX, 1.0);
    p.set(granular::PITCH, 7.0);
    render_buf(&mut e, 96_000);
    assert!(p.viz[granular::viz::COUNT].get() > 0.0, "no grains playing");

    p.set_toggle(true);
    unplug_input(&mut g, &a, m);
    render_buf(&mut e, 48_000);
    let frozen = rms_of(&render_buf(&mut e, 48_000));
    assert!(frozen > 0.01, "frozen cloud is silent: {frozen}");

    // Unfrozen with no input, the loop fills with silence.
    p.set_toggle(false);
    render_buf(&mut e, 48_000 * 9);
    let empty = rms_of(&render_buf(&mut e, 24_000));
    assert!(empty < 1e-3, "cloud should fade with no input: {empty}");
}

#[test]
fn delay_echoes_outlast_the_input() {
    use crate::audio::modules::delay;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Delay);
    p.set(delay::TIME, 0.25);
    p.set(delay::FEEDBACK, 0.5);
    p.set(delay::MIX, 1.0);
    render_buf(&mut e, 24_000);
    unplug_input(&mut g, &a, m);
    let echoes = rms_of(&render_buf(&mut e, 24_000));
    assert!(echoes > 0.01, "no echoes: {echoes}");
    render_buf(&mut e, 48_000 * 4);
    let gone = rms_of(&render_buf(&mut e, 24_000));
    assert!(gone < echoes * 0.05, "echoes do not die away: {echoes} → {gone}");
}

#[test]
fn filter_modes_shape_the_spectrum() {
    use crate::audio::modules::filter;
    let (mut g, a, mut e) = setup();
    let (_, p) = patch(&mut g, &a, ModuleKind::Filter);
    osc_params(&g, 1).set(osc::FREQ, 6000.0);
    p.set(filter::CUTOFF, 300.0);
    p.set(filter::RESO, 0.0);
    p.set_mode(filter::LOWPASS);
    render_buf(&mut e, 4800);
    let low = rms_of(&render_buf(&mut e, 9600));
    p.set_mode(filter::HIGHPASS);
    render_buf(&mut e, 4800);
    let high = rms_of(&render_buf(&mut e, 9600));
    assert!(low < high * 0.05, "6 kHz through a 300 Hz low-pass: {low} vs high-pass {high}");
}

#[test]
fn random_wanders_within_range() {
    use crate::audio::modules::random;
    let (mut g, a, mut e) = setup();
    let (_, p) = patch(&mut g, &a, ModuleKind::Random);
    p.set(random::RATE, 20.0);
    let mut seen = Vec::new();
    for _ in 0..40 {
        render_buf(&mut e, 600);
        seen.push(p.viz[random::VALUE].get());
    }
    let (lo, hi) = seen.iter().fold((1.0f32, -1.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    assert!(lo >= -1.0 && hi <= 1.0);
    assert!(hi - lo > 0.5, "barely moves: {lo}..{hi}");
}

#[test]
fn resonator_rings_after_the_excitation_stops() {
    use crate::audio::modules::resonator;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Resonator);
    p.set(resonator::DECAY, 4.0);
    render_buf(&mut e, 24_000);
    unplug_input(&mut g, &a, m);
    let ring = rms_of(&render_buf(&mut e, 24_000));
    assert!(ring > 0.005, "no ringing: {ring}");
}

#[test]
fn duplicating_a_module_copies_its_settings() {
    use crate::audio::modules::reverb;
    let (mut g, a, _e) = setup();
    let m = g.add_module(ModuleKind::Reverb, pos2(0.0, 0.0), &a);
    let p = module_params(&g, m);
    p.set(reverb::DECAY, 33.0);
    p.set_toggle(true);
    let copy = g.duplicate(m, &a).unwrap();
    let q = module_params(&g, copy);
    assert_eq!(q.get(reverb::DECAY), 33.0);
    assert!(q.toggled());
    assert_eq!(g.node(copy).unwrap().title, "REVERB 2");
}

#[test]
fn cables_carry_stereo_through_the_mixer() {
    let (mut g, a, mut e) = setup();
    let x = source(&mut g, &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    mixer_params(&g, m).pan[0].set(-1.0);
    render_buf(&mut e, 4800);
    let buf = render_buf(&mut e, 4800);
    let left: Vec<f32> = buf.iter().step_by(2).copied().collect();
    let right: Vec<f32> = buf.iter().skip(1).step_by(2).copied().collect();
    assert!(peak_of(&left) > 0.1);
    assert!(peak_of(&right) < 1e-3, "hard-left pan leaks right: {}", peak_of(&right));
}

#[test]
fn every_node_type_draws_without_panicking() {
    let (mut g, a, mut e) = setup();
    let mut editor = crate::ui::canvas::Editor::new();
    for (_, types) in crate::graph::NodeType::MENU {
        for &ty in types {
            let id = g.add(ty, pos2(0.0, 0.0), &a);
            // Patch the previous node in, so connected states are drawn too.
            if id > 1 && !g.node(id).unwrap().inputs().is_empty() {
                g.connect(port(id - 1, 0), port(id, 0), &a);
            }
        }
    }
    // The first three nodes form a module.
    let first: Vec<u32> = g.nodes.iter().take(3).map(|n| n.id).collect();
    let gid = g.create_group(&first).unwrap();
    render_buf(&mut e, 4800);
    let ctx = egui::Context::default();
    for frame in 0..6 {
        // Expanded, then the module folded, then everything folded.
        g.set_all_collapsed(frame >= 4);
        g.set_group_collapsed(gid, frame >= 2);
        for node in &mut g.nodes {
            node.update_meters(1.0 / 60.0);
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| editor.show(ui, &mut g, &a));
        out.textures_delta.clear(); // no renderer here to upload them
    }
}

#[test]
fn collapsing_shrinks_a_node_and_keeps_it_patchable() {
    let (mut g, a, mut e) = setup();
    let x = source(&mut g, &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 2), &a);

    let full = g.node(m).unwrap().size();
    let input = g.node(m).unwrap().input_pos(2);
    g.toggle_collapsed(m);
    let small = g.node(m).unwrap().size();
    assert!(small.x < full.x && small.y < full.y, "{full:?} → {small:?}");
    assert_ne!(g.node(m).unwrap().input_pos(2), input, "ports follow the compact layout");
    assert!(g.node(m).unwrap().input_pos(3).y < g.node(m).unwrap().pos.y + small.y);

    // Collapsing is visual only: the audio keeps flowing.
    render_buf(&mut e, 4800);
    assert!(rms_of(&render_buf(&mut e, 4800)) > 0.01);

    let copy = g.duplicate(m, &a).unwrap();
    assert!(g.node(copy).unwrap().collapsed, "duplicates keep the collapsed state");
    g.set_all_collapsed(false);
    assert!(g.nodes.iter().all(|n| !n.collapsed));
    assert_eq!(g.node(m).unwrap().size(), full);
}

/// Random → Filter (CUT), Drone → Filter → Reverb → Mixer.
fn ambient_chain(g: &mut Graph, a: &AudioHandle) -> [u32; 5] {
    let random = g.add_module(ModuleKind::Random, pos2(0.0, 0.0), a);
    let drone = g.add_module(ModuleKind::Drone, pos2(0.0, 300.0), a);
    let filter = g.add_module(ModuleKind::Filter, pos2(340.0, 150.0), a);
    let reverb = g.add_module(ModuleKind::Reverb, pos2(700.0, 150.0), a);
    let mixer = g.add_mixer(pos2(1000.0, 150.0), a);
    g.connect(port(random, 0), port(filter, 1), a);
    g.connect(port(drone, 0), port(filter, 0), a);
    g.connect(port(filter, 0), port(reverb, 0), a);
    g.connect(port(reverb, 0), port(mixer, 0), a);
    [random, drone, filter, reverb, mixer]
}

#[test]
fn modules_expose_only_their_outer_ports() {
    let (mut g, a, _e) = setup();
    let [random, drone, filter, reverb, _mixer] = ambient_chain(&mut g, &a);
    let gid = g.create_group(&[random, drone, filter, reverb]).unwrap();

    let (ins, outs) = g.exposed_ports(g.group(gid).unwrap());
    // Only the drone's free CV inputs and the reverb feeding the mixer.
    assert_eq!(ins, vec![port(drone, 0), port(drone, 1)]);
    assert_eq!(outs, vec![port(reverb, 0)]);

    // Tapping the filter from outside exposes its output as well.
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    g.connect(port(filter, 0), port(x, 0), &a);
    let (_, outs) = g.exposed_ports(g.group(gid).unwrap());
    assert_eq!(outs, vec![port(filter, 0), port(reverb, 0)]);
}

#[test]
fn modules_are_created_renamed_and_dissolved() {
    let (mut g, a, mut e) = setup();
    let [random, drone, filter, reverb, mixer] = ambient_chain(&mut g, &a);
    let first = g.create_group(&[random, drone]).unwrap();
    assert_eq!(g.group(first).unwrap().title, "MODULE 1");

    // A node joins at most one module: it leaves the old one.
    let second = g.create_group(&[drone, filter]).unwrap();
    assert_eq!(g.group(second).unwrap().title, "MODULE 2");
    assert_eq!(g.group(first).unwrap().members, vec![random]);
    assert_eq!(g.group_of(drone).unwrap().id, second);

    g.rename_group(second, "Pad");
    assert_eq!(g.group(second).unwrap().title, "Pad");

    // Grouping is visual: the sound is the same before and after.
    render_buf(&mut e, 9600);
    let grouped = rms_of(&render_buf(&mut e, 9600));
    g.ungroup(second);
    assert!(g.group_of(drone).is_none());
    assert_eq!(g.nodes.len(), 5, "ungrouping keeps the nodes");
    assert!(grouped > 0.005);

    // Deleting its last node removes a module.
    g.remove_node(random, &a);
    assert!(g.group(first).is_none());
    let _ = (reverb, mixer);
}

#[test]
fn duplicating_a_module_copies_its_inner_cables() {
    let (mut g, a, _e) = setup();
    let [random, drone, filter, _reverb, _mixer] = ambient_chain(&mut g, &a);
    let gid = g.create_group(&[random, drone, filter]).unwrap();
    g.set_group_collapsed(gid, true);
    let cables_before = g.cables.len();

    let copies = g.duplicate_nodes(&[random, drone, filter], &a);
    assert_eq!(copies.len(), 3);
    // Random → Filter and Drone → Filter are recreated between the copies;
    // the cable leaving the module (Filter → Reverb) is not.
    assert_eq!(g.cables.len(), cables_before + 2);
    let copy = g.group_of(copies[0]).expect("the copies form a module");
    assert_ne!(copy.id, gid);
    assert_eq!(copy.members.len(), 3);
    assert!(copy.collapsed);
}

#[test]
fn keyboard_makes_and_folds_a_module() {
    let (mut g, a, _e) = setup();
    let [random, drone, filter, _reverb, _mixer] = ambient_chain(&mut g, &a);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let frame = |editor: &mut crate::ui::canvas::Editor, g: &mut Graph, key: Option<(egui::Key, egui::Modifiers)>| {
        let mut input = egui::RawInput::default();
        if let Some((key, modifiers)) = key {
            input.events.push(egui::Event::ModifiersChanged(modifiers));
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, &a));
        out.textures_delta.clear();
    };

    frame(&mut editor, &mut g, None);
    editor.selected = [random, drone, filter].into_iter().collect();
    frame(&mut editor, &mut g, Some((egui::Key::G, egui::Modifiers::COMMAND)));
    assert_eq!(g.groups.len(), 1);
    let gid = g.groups[0].id;
    assert_eq!(editor.selected_group(&g).map(|m| m.id), Some(gid));

    frame(&mut editor, &mut g, Some((egui::Key::C, egui::Modifiers::NONE)));
    assert!(g.group(gid).unwrap().collapsed, "C folds a selected module");
    frame(&mut editor, &mut g, Some((egui::Key::C, egui::Modifiers::NONE)));
    assert!(!g.group(gid).unwrap().collapsed);

    frame(&mut editor, &mut g, Some((egui::Key::G, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)));
    assert!(g.groups.is_empty(), "Cmd+Shift+G ungroups");
}

#[test]
fn right_click_opens_a_menu_on_anything() {
    let (mut g, a, _e) = setup();
    let x = source(&mut g, &a);
    let y = g.add_osc(pos2(500.0, 0.0), &a);
    g.connect(port(x, 0), port(y, 0), &a);
    let pair = [g.add_osc(pos2(0.0, 400.0), &a), g.add_osc(pos2(300.0, 400.0), &a)];
    let gid = g.create_group(&pair).unwrap();
    g.set_group_collapsed(gid, true);

    let mut editor = crate::ui::canvas::Editor::new();
    // The canvas menu lists library modules too.
    editor.library = vec![crate::project::preset_from_group(&g, gid).unwrap()];
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    let run = |editor: &mut crate::ui::canvas::Editor, g: &mut Graph, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, &a));
        out.textures_delta.clear();
    };
    run(&mut editor, &mut g, vec![]);
    let to_screen = |e: &crate::ui::canvas::Editor, p: egui::Pos2| (e.pan + p.to_vec2() * e.zoom).to_pos2();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };

    let knob = to_screen(&editor, g.node(x).unwrap().pos + egui::vec2(44.0, 175.0));
    let card = to_screen(&editor, crate::ui::groups::frame_rect(&g, g.group(gid).unwrap()).unwrap().min)
        + egui::vec2(120.0, 40.0) * editor.zoom;
    let mid_cable = {
        let (a_, b_) = (g.node(x).unwrap().output_pos(0), g.node(y).unwrap().input_pos(0));
        to_screen(&editor, a_ + (b_ - a_) * 0.5)
    };
    let empty = egui::pos2(1380.0, 880.0);
    for (what, p, selects) in [
        ("a knob", knob, vec![x]),
        ("a module card", card, pair.to_vec()),
        ("a cable", mid_cable, vec![]),
        ("empty canvas", empty, vec![]),
    ] {
        editor.selected.clear();
        run(&mut editor, &mut g, vec![egui::Event::PointerMoved(p)]);
        run(&mut editor, &mut g, vec![button(p, true)]);
        run(&mut editor, &mut g, vec![button(p, false)]);
        run(&mut editor, &mut g, vec![]);
        assert!(egui::Popup::is_any_open(&ctx), "right-click on {what} opened no menu");
        let mut got: Vec<u32> = editor.selected.iter().copied().collect();
        got.sort();
        assert_eq!(got, selects, "right-click on {what} selects it");
        run(&mut editor, &mut g, vec![escape.clone()]);
        run(&mut editor, &mut g, vec![]);
        assert!(!egui::Popup::is_any_open(&ctx));
    }
}

// ---------------------------------------------------------------------------
// Saving and the module library
// ---------------------------------------------------------------------------

/// A fresh folder under the system temp dir, removed on drop.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("audioboard-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn projects_round_trip_through_a_file() {
    use crate::audio::modules::{drone, reverb};
    use crate::project;
    let (mut g, a, _e) = setup();
    let [random, drone_id, filter, reverb_id, mixer] = ambient_chain(&mut g, &a);
    let x = source(&mut g, &a);
    module_params(&g, drone_id).set(drone::PITCH, 73.4);
    module_params(&g, drone_id).set_mode(3);
    module_params(&g, reverb_id).set(reverb::DECAY, 21.0);
    module_params(&g, reverb_id).set_toggle(true);
    mixer_params(&g, mixer).pan[0].set(-0.4);
    mixer_params(&g, mixer).mute[2].store(true, Ordering::Relaxed);
    mixer_params(&g, mixer).aux[1].set(0.5);
    mixer_params(&g, mixer).aux_master.set(0.4);
    let console = g.add_console(pos2(0.0, 600.0), &a);
    g.set_console(console, crate::graph::Console { channels: 24, horizontal: false }, &a);
    mixer_params(&g, console).master_mute.store(true, Ordering::Relaxed);
    mixer_params(&g, console).aux[20].set(0.7);
    g.node(reverb_id).unwrap().set_bypass(true);
    osc_params(&g, x).set_waveform(Waveform::Square);
    g.set_collapsed(filter, true);
    let gid = g.create_group(&[random, drone_id]).unwrap();
    g.rename_group(gid, "Source");
    g.set_group_collapsed(gid, true);

    let dir = TempDir::new("project");
    let path = dir.0.join("pad.audioboard");
    let saved = project::snapshot(&g);
    project::save_project(&path, &saved).unwrap();

    // Load into a patch that already has other nodes: they are replaced.
    let (mut h, b, mut e2) = setup();
    source(&mut h, &b);
    let loaded = project::load_project(&path).unwrap();
    assert_eq!(project::restore(&mut h, &loaded, &b), 0);
    assert_eq!(project::snapshot(&h), saved, "everything survives the round trip");
    assert_eq!(h.groups[0].title, "Source");

    // And it plays.
    render_buf(&mut e2, 48_000);
    assert!(rms_of(&render_buf(&mut e2, 24_000)) > 0.005);
}

#[test]
fn library_modules_are_inserted_with_their_cables() {
    use crate::audio::modules::filter;
    use crate::project;
    let (mut g, a, _e) = setup();
    let [random, drone, filter_id, _reverb, _mixer] = ambient_chain(&mut g, &a);
    module_params(&g, filter_id).set(filter::CUTOFF, 333.0);
    let gid = g.create_group(&[random, drone, filter_id]).unwrap();
    g.rename_group(gid, "Wobble pad");
    let preset = project::preset_from_group(&g, gid).unwrap();
    assert_eq!(preset.name, "Wobble pad");
    assert_eq!(preset.node_count(), 3);

    let (mut h, b, _e2) = setup();
    let nodes = project::insert_preset(&mut h, &preset, pos2(500.0, 200.0), &b);
    assert_eq!(nodes.len(), 3);
    assert_eq!(h.cables.len(), 2, "random → filter and drone → filter");
    let group = h.group_of(nodes[0]).expect("inserted as a module");
    assert_eq!(group.title, "Wobble pad");
    assert!(group.collapsed, "library modules arrive folded");
    let frame = crate::ui::groups::frame_rect(&h, group).unwrap();
    assert!((frame.min - pos2(500.0, 200.0)).length() < 1e-3, "frame at the drop point: {frame:?}");
    let f = h.nodes.iter().find(|n| n.title == "FILTER 1").unwrap().id;
    assert_eq!(module_params(&h, f).get(filter::CUTOFF), 333.0);
}

#[test]
fn library_stores_presets_on_disk() {
    use crate::project::{self, Library};
    let (mut g, a, _e) = setup();
    let [random, drone, ..] = ambient_chain(&mut g, &a);
    let gid = g.create_group(&[random, drone]).unwrap();
    g.rename_group(gid, "Drift / pad");
    let preset = project::preset_from_group(&g, gid).unwrap();

    let dir = TempDir::new("library");
    let mut lib = Library::open(dir.0.join("Library"));
    assert!(lib.presets.is_empty());
    lib.save(&preset).unwrap();
    assert!(lib.contains("Drift / pad"));

    // A fresh library sees the file; saving again replaces it.
    let mut again = Library::open(dir.0.join("Library"));
    assert_eq!(again.presets, vec![preset.clone()]);
    again.save(&preset).unwrap();
    assert_eq!(again.presets.len(), 1);

    again.remove(0).unwrap();
    assert!(Library::open(dir.0.join("Library")).presets.is_empty());
}

#[test]
fn foreign_files_are_rejected() {
    use crate::project;
    let dir = TempDir::new("foreign");
    let path = dir.0.join("x.audioboard");
    std::fs::write(&path, "{\"hello\": 1}").unwrap();
    assert!(project::load_project(&path).is_err());
    std::fs::write(&path, r#"{"format":"other","version":1,"nodes":[],"cables":[]}"#).unwrap();
    assert!(project::load_project(&path).is_err());
}

// ---------------------------------------------------------------------------
// MIDI, renaming, palette
// ---------------------------------------------------------------------------

/// Runs editor frames with the given input events.
struct Ui {
    ctx: egui::Context,
    editor: crate::ui::canvas::Editor,
    /// Simulated clock, 50 ms per frame.
    time: f64,
}

impl Ui {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            editor: crate::ui::canvas::Editor::new(),
            time: 0.0,
        }
    }

    fn frame(&mut self, g: &mut Graph, a: &AudioHandle, events: Vec<egui::Event>) {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let editor = &mut self.editor;
        let mut out = self.ctx.run_ui(input, |ui| editor.show(ui, g, a));
        out.textures_delta.clear();
    }

    fn key(&mut self, g: &mut Graph, a: &AudioHandle, key: egui::Key) {
        self.key_with(g, a, key, egui::Modifiers::NONE);
    }

    fn key_with(&mut self, g: &mut Graph, a: &AudioHandle, key: egui::Key, modifiers: egui::Modifiers) {
        let event = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        self.frame(g, a, vec![egui::Event::ModifiersChanged(modifiers), event]);
        self.frame(g, a, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    }

    /// Lets animations finish.
    fn settle(&mut self, g: &mut Graph, a: &AudioHandle) {
        for _ in 0..16 {
            self.frame(g, a, vec![]);
        }
    }
}

#[test]
fn midi_state_tracks_controllers_and_the_last_key() {
    use crate::audio::midi::MidiState;
    let m = MidiState::new();
    m.handle(&[0xB2, 74, 127]); // CC 74 on channel 3
    let (value, seq) = m.cc(3, 74);
    assert_eq!((value, seq), (1.0, 1));
    assert_eq!(m.cc(0, 74).0, 1.0, "omni sees every channel");
    assert_eq!(m.cc(1, 74).1, 0);
    assert_eq!(m.last_cc(), (1, 3, 74));

    m.handle(&[0x90, 60, 100]);
    m.handle(&[0x90, 67, 50]);
    assert_eq!(m.note(1).note, 67);
    m.handle(&[0x80, 67, 0]); // back to the key still held
    let held = m.note(1);
    assert_eq!((held.note, held.gate), (60, true));
    m.handle(&[0x90, 60, 0]); // note-on with velocity 0 is a note-off
    assert!(!m.note(1).gate);
    m.handle(&[0x90, 50, 90]);
    m.handle(&[0xB0, 123, 0]); // all notes off
    assert!(!m.note(0).gate);
    m.handle(&[0xF8]); // clock: ignored
}

#[test]
fn midi_learn_and_mapping_drive_knobs() {
    use crate::audio::modules::{drone, filter, midi_cc};
    use crate::graph::{ParamRef, Target};
    let (mut g, a, mut e) = setup();
    let [_random, drone_id, filter_id, _reverb, mixer] = ambient_chain(&mut g, &a);
    let cc = g.add_module(ModuleKind::MidiCc, pos2(0.0, 0.0), &a);
    let note = g.add_module(ModuleKind::MidiNote, pos2(0.0, 0.0), &a);

    // LEARN takes the next controller moved.
    module_params(&g, cc).set_toggle(true);
    g.update_midi(&a.midi);
    a.midi.handle(&[0xB4, 21, 10]); // CC 21, channel 5
    g.update_midi(&a.midi);
    let p = module_params(&g, cc);
    assert_eq!((p.get(midi_cc::CC), p.get(midi_cc::CHANNEL)), (21.0, 5.0));
    assert!(!p.toggled(), "learn switches itself off");

    // Mapping does not jump the knob; the next message moves it.
    let cutoff = Target { node: filter_id, param: ParamRef::Module(filter::CUTOFF) };
    g.set_target(cc, Some(cutoff));
    let before = module_params(&g, filter_id).get(filter::CUTOFF);
    g.update_midi(&a.midi);
    assert_eq!(module_params(&g, filter_id).get(filter::CUTOFF), before);
    a.midi.handle(&[0xB4, 21, 127]);
    g.update_midi(&a.midi);
    assert_eq!(module_params(&g, filter_id).get(filter::CUTOFF), 20_000.0);
    assert_eq!(g.target_label(cutoff).unwrap(), "FILTER 1 · CUTOFF");

    // A key sets a pitch knob to its frequency.
    g.set_target(note, Some(Target { node: drone_id, param: ParamRef::Module(drone::PITCH) }));
    g.update_midi(&a.midi);
    a.midi.handle(&[0x90, 45, 100]); // A2
    g.update_midi(&a.midi);
    assert!((module_params(&g, drone_id).get(drone::PITCH) - 110.0).abs() < 0.01);

    // Controllers can ride mixer faders too.
    g.set_target(cc, Some(Target { node: mixer, param: ParamRef::Fader(1) }));
    g.update_midi(&a.midi);
    a.midi.handle(&[0xB4, 21, 0]);
    g.update_midi(&a.midi);
    assert_eq!(mixer_params(&g, mixer).fader[1].get(), 0.0);

    // The MIDI note node plays an envelope while the key is held.
    render_buf(&mut e, 24_000);
    assert!(module_params(&g, note).viz[crate::audio::modules::midi_note::ENV].get() > 0.9);

    // Deleting the mapped node unmaps it; mappings survive saving.
    g.remove_node(mixer, &a);
    assert!(g.target_of(cc).is_none());
    g.set_target(cc, Some(cutoff));
    let saved = crate::project::snapshot(&g);
    let (mut h, b, _e2) = setup();
    crate::project::restore(&mut h, &crate::project::snapshot(&g), &b);
    assert_eq!(crate::project::snapshot(&h), saved);
    let cc_copy = h.nodes.iter().find(|n| n.title == "CC 1").unwrap().id;
    assert_eq!(h.target_label(h.target_of(cc_copy).unwrap()).unwrap(), "FILTER 1 · CUTOFF");
}

#[test]
fn modules_are_renamed_in_place() {
    let (mut g, a, _e) = setup();
    let [random, drone, ..] = ambient_chain(&mut g, &a);
    let gid = g.create_group(&[random, drone]).unwrap();
    let mut ui = Ui::new();
    ui.frame(&mut g, &a, vec![]);
    ui.editor.selected = [random, drone].into_iter().collect();

    // Enter opens the title for editing, with the old name selected.
    ui.key(&mut g, &a, egui::Key::Enter);
    ui.frame(&mut g, &a, vec![]); // (a person types a few frames later)
    ui.frame(&mut g, &a, vec![egui::Event::Text("Pad source".into())]);
    ui.key(&mut g, &a, egui::Key::Enter);
    assert_eq!(g.group(gid).unwrap().title, "Pad source");

    // Esc cancels.
    ui.key(&mut g, &a, egui::Key::F2);
    ui.frame(&mut g, &a, vec![]);
    ui.frame(&mut g, &a, vec![egui::Event::Text("Nope".into())]);
    ui.key(&mut g, &a, egui::Key::Escape);
    assert_eq!(g.group(gid).unwrap().title, "Pad source");
}

#[test]
fn palette_entries_drop_where_released() {
    use crate::ui::palette::Spawn;
    let (mut g, a, _e) = setup();
    g.add_mixer(pos2(0.0, 0.0), &a);
    let mut ui = Ui::new();
    ui.frame(&mut g, &a, vec![]);

    let p = egui::pos2(300.0, 200.0);
    egui::DragAndDrop::set_payload(&ui.ctx, Spawn::Node(crate::graph::NodeType::Module(ModuleKind::Reverb)));
    let release = egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    };
    ui.frame(&mut g, &a, vec![egui::Event::PointerMoved(p), release]);
    let reverb = g.nodes.iter().find(|n| n.title == "REVERB 1").expect("dropped");
    // Its header sits under the pointer.
    let e = &ui.editor;
    let screen = (e.pan + reverb.pos.to_vec2() * e.zoom).to_pos2();
    assert!((screen.y - p.y).abs() < 20.0 && screen.x < p.x, "{screen:?}");
    assert!(egui::DragAndDrop::payload::<Spawn>(&ui.ctx).is_none());
}

// ---------------------------------------------------------------------------
// Auto layout and zoom
// ---------------------------------------------------------------------------

fn rect_of(g: &Graph, id: u32) -> egui::Rect {
    let n = g.node(id).unwrap();
    egui::Rect::from_min_size(n.pos, n.size())
}

fn assert_no_overlaps(g: &Graph, ids: &[u32]) {
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            let (ra, rb) = (rect_of(g, a), rect_of(g, b));
            assert!(!ra.shrink(1.0).intersects(rb.shrink(1.0)), "{a} and {b} overlap: {ra:?} {rb:?}");
        }
    }
}

#[test]
fn arrange_puts_signal_flow_in_columns_with_level_ports() {
    use crate::ui::layout::{GAP_X, Link, arrange};
    let size = egui::vec2(200.0, 100.0);
    // 0 → 1 → 2, and modulator 3 → 2 (its port lower on 2).
    let links = [
        Link { from: 0, to: 1, from_dy: 20.0, to_dy: 20.0 },
        Link { from: 1, to: 2, from_dy: 20.0, to_dy: 20.0 },
        Link { from: 3, to: 2, from_dy: 20.0, to_dy: 60.0 },
    ];
    let start = [pos2(500.0, 0.0), pos2(0.0, 300.0), pos2(250.0, 900.0), pos2(0.0, 0.0)];
    let p = arrange(&[size; 4], &start, &links);
    assert_eq!(p[0].x, 0.0);
    assert_eq!(p[1].x, size.x + GAP_X);
    assert_eq!(p[2].x, 2.0 * (size.x + GAP_X));
    assert_eq!(p[3].x, p[1].x, "the modulator sits right before what it feeds");
    // Two blocks in one column never overlap, and a source lines up with
    // what it feeds even when its neighbours have to compromise.
    assert!((p[1].y - p[3].y).abs() >= size.y + crate::ui::layout::GAP_Y - 0.5);
    assert!((p[0].y - p[1].y).abs() < 0.5);

    // A plain chain comes out perfectly straight.
    let chain = arrange(&[size; 3], &start[..3], &links[..2]);
    assert!(chain.iter().all(|q| q.y == chain[0].y), "{chain:?}");
}

#[test]
fn auto_layout_arranges_the_patch_left_to_right() {
    use crate::ui::layout;
    let (mut g, a, _e) = setup();
    let ids = ambient_chain(&mut g, &a);
    let [random, drone, filter, reverb, mixer] = ids;
    // Scramble.
    for (i, id) in ids.iter().enumerate() {
        g.node_mut(*id).unwrap().pos = pos2(900.0 - 200.0 * i as f32, 50.0 * (i % 2) as f32);
    }
    let gid = g.create_group(&[random, drone]).unwrap();
    for (id, to) in layout::plan(&g, None) {
        g.node_mut(id).unwrap().pos = to;
    }
    assert_no_overlaps(&g, &ids);
    let x = |id| g.node(id).unwrap().pos.x;
    assert!(x(drone) < x(filter) && x(filter) < x(reverb) && x(reverb) < x(mixer));
    assert!(x(random) < x(filter));
    // Every cable runs left to right.
    for c in &g.cables {
        assert!(g.node(c.from.node).unwrap().output_pos(c.from.port).x < g.node(c.to.node).unwrap().input_pos(c.to.port).x);
    }
    // The module moved as one block: its frame does not cover other nodes.
    let frame = crate::ui::groups::frame_rect(&g, g.group(gid).unwrap()).unwrap();
    for id in [filter, reverb, mixer] {
        assert!(!frame.intersects(rect_of(&g, id)), "module frame covers {id}");
    }
}

#[test]
fn layout_glides_into_place_and_undoes() {
    use crate::ui::layout;
    let (mut g, a, _e) = setup();
    let ids = ambient_chain(&mut g, &a);
    for (i, id) in ids.iter().enumerate() {
        g.node_mut(*id).unwrap().pos = pos2(1000.0 - 250.0 * i as f32, 0.0);
    }
    let before: Vec<egui::Pos2> = ids.iter().map(|id| g.node(*id).unwrap().pos).collect();
    let planned = layout::plan(&g, None);

    let mut ui = Ui::new();
    ui.frame(&mut g, &a, vec![]);
    ui.key(&mut g, &a, egui::Key::L);
    ui.settle(&mut g, &a);
    for (id, to) in &planned {
        assert!((g.node(*id).unwrap().pos - *to).length() < 0.01);
    }
    assert!(ui.editor.notices.iter().any(|n| n.contains("Cmd+Z")));

    ui.key_with(&mut g, &a, egui::Key::Z, egui::Modifiers::COMMAND);
    ui.settle(&mut g, &a);
    let after: Vec<egui::Pos2> = ids.iter().map(|id| g.node(*id).unwrap().pos).collect();
    assert_eq!(after, before, "Cmd+Z puts everything back");
}

#[test]
fn auto_fit_follows_the_patch_until_you_zoom() {
    let (mut g, a, _e) = setup();
    ambient_chain(&mut g, &a);
    let mut ui = Ui::new();
    ui.frame(&mut g, &a, vec![]);
    ui.key(&mut g, &a, egui::Key::A);
    assert!(ui.editor.auto_fit);
    ui.settle(&mut g, &a);
    let fitted = ui.editor.zoom;

    // A node far away: the view zooms out to keep everything in sight.
    g.add_osc(pos2(5000.0, 3000.0), &a);
    ui.settle(&mut g, &a);
    assert!(ui.editor.zoom < fitted * 0.7, "{} vs {fitted}", ui.editor.zoom);

    // Zooming by hand turns auto-fit off.
    let z = ui.editor.zoom;
    ui.key_with(&mut g, &a, egui::Key::Equals, egui::Modifiers::COMMAND);
    ui.settle(&mut g, &a);
    assert!(!ui.editor.auto_fit);
    assert!((ui.editor.zoom - z * 1.25).abs() < 1e-3);
}

#[test]
fn after_auto_layout_no_cable_runs_behind_a_node() {
    use crate::ui::{layout, widgets};
    let (mut g, a, _e) = setup();
    // The demo patch: its reverb → mixer cable skips the delay's column.
    crate::ui::app::demo_patch(&mut g, &a);
    for (id, to) in layout::plan(&g, None) {
        g.node_mut(id).unwrap().pos = to;
    }

    for c in &g.cables {
        let (from, to) = (g.node(c.from.node).unwrap(), g.node(c.to.node).unwrap());
        let shape = widgets::cable_shape(from.output_pos(c.from.port), to.input_pos(c.to.port), false, 1.0);
        for n in g.nodes.iter().filter(|n| n.id != from.id && n.id != to.id) {
            let body = egui::Rect::from_min_size(n.pos, n.size()).shrink(4.0);
            for i in 0..=64 {
                let p = shape.sample(i as f32 / 64.0);
                assert!(!body.contains(p), "{} → {} passes behind {}", from.title, to.title, n.title);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// VST3 plugins
// ---------------------------------------------------------------------------

#[test]
fn base64_round_trips() {
    use crate::project::{base64_decode, base64_encode};
    for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"\x00\xff\x10binary\x80"] {
        let text = base64_encode(data);
        assert_eq!(base64_decode(&text).unwrap(), data, "{text}");
    }
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    assert!(base64_decode("not*base64").is_none());
}

#[test]
fn midi_stream_reaches_every_reader() {
    use crate::audio::midi::MidiState;
    let m = MidiState::new();
    let mut a = m.stream_head();
    m.handle(&[0x90, 60, 100]);
    m.handle(&[0xE0, 0, 64]); // pitch bend: kept in the stream though not in the state
    let mut b = m.stream_head(); // a late reader sees only newer messages
    m.handle(&[0x80, 60, 0]);
    let mut seen_a = Vec::new();
    m.read_stream(&mut a, |msg| seen_a.push(msg));
    assert_eq!(seen_a, vec![[0x90, 60, 100], [0xE0, 0, 64], [0x80, 60, 0]]);
    let mut seen_b = Vec::new();
    m.read_stream(&mut b, |msg| seen_b.push(msg));
    assert_eq!(seen_b, vec![[0x80, 60, 0]]);
    // A reader far behind skips to what is still kept.
    let mut late = m.stream_head();
    for i in 0..3000u32 {
        m.handle(&[0xB0, 1, (i % 128) as u8]);
    }
    let mut count = 0;
    m.read_stream(&mut late, |_| count += 1);
    assert_eq!(count, 1024);
}

/// A free effect plugin, if installed (these checks need a real VST3).
fn installed_effect() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from("/Library/Audio/Plug-Ins/VST3/ValhallaFreqEcho.vst3");
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipping: {} is not installed", path.display());
        None
    }
}

#[test]
fn vst3_plugins_work_as_nodes() {
    use crate::graph::{NodeKind, ParamRef, Target};
    let Some(path) = installed_effect() else { return };
    let (mut g, a, mut e) = setup();
    let src = source(&mut g, &a);
    let fx = g.add_plugin(&path, pos2(300.0, 0.0), &a).expect("loads");
    let mix = g.add_mixer(pos2(600.0, 0.0), &a);
    g.connect(port(src, 0), port(fx, 0), &a);
    g.connect(port(fx, 0), port(mix, 0), &a);
    let node = g.node(fx).unwrap();
    assert_eq!(node.title, "VALHALLAFREQECHO 1");
    assert_eq!(node.inputs(), &["IN"]);

    // Audio flows through it.
    render_buf(&mut e, 24_000);
    let buf = render_buf(&mut e, 24_000);
    assert!(buf.iter().all(|s| s.is_finite()));
    assert!(rms_of(&buf) > 0.01, "plugin output is silent");

    // A knob moved in the UI reaches the plugin on the audio thread.
    let (index, id) = match &g.node(fx).unwrap().kind {
        NodeKind::Plugin(p) => (0, p.params[0].id),
        _ => unreachable!(),
    };
    if let Some(NodeKind::Plugin(p)) = g.node_mut(fx).map(|n| &mut n.kind) {
        p.set_param(index, 0.25, 0.0);
    }
    render_buf(&mut e, 4800);
    let plugin_value = |g: &Graph| match &g.node(fx).unwrap().kind {
        NodeKind::Plugin(p) => p.shared.plugin.lock().unwrap().get_parameter(id).unwrap(),
        _ => unreachable!(),
    };
    assert!((plugin_value(&g) - 0.25).abs() < 1e-3);

    // MIDI CC can drive a plugin parameter too.
    let cc = g.add_module(ModuleKind::MidiCc, pos2(0.0, 300.0), &a);
    g.set_target(cc, Some(Target { node: fx, param: ParamRef::Plugin(id) }));
    g.update_midi(&a.midi);
    a.midi.handle(&[0xB0, 1, 127]);
    g.update_midi(&a.midi);
    render_buf(&mut e, 4800);
    assert!((plugin_value(&g) - 1.0).abs() < 1e-3);

    // Saved and reopened: same plugin, same state, same knobs.
    let saved = crate::project::snapshot(&g);
    let (mut h, b, mut e2) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    let copy = h.nodes.iter().find(|n| matches!(n.kind, NodeKind::Plugin(_))).expect("plugin restored").id;
    render_buf(&mut e2, 4800);
    assert!((plugin_value(&h) - 1.0).abs() < 1e-3, "state restored");
    let _ = copy;
    assert_eq!(crate::project::snapshot(&h), saved);

    // Duplicates carry the state; removal tears down off the audio thread.
    let dup = g.duplicate(fx, &a).expect("duplicates");
    assert_eq!(g.node(dup).unwrap().title, "VALHALLAFREQECHO 2");
    g.remove_node(fx, &a);
    g.remove_node(dup, &a);
    render_buf(&mut e, 4800); // the engine lets go of them
    g.service_plugins(0.0);
    assert!(rms_of(&render_buf(&mut e, 4800)) < 1e-3, "nothing left reaches the mixer");
}

// ---------------------------------------------------------------------------
// MIDI sequencing
// ---------------------------------------------------------------------------

use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx, Processor};

/// Runs a MIDI generator for `frames` at 120 BPM and returns every event
/// it sent as (absolute frame, message). `feed` may add input events for
/// each block (given its first frame).
fn run_generator(
    proc: &mut dyn Processor,
    frames: usize,
    mut feed: impl FnMut(usize, &mut MidiBuf),
) -> Vec<(usize, [u8; 3])> {
    const SR: f32 = 48_000.0;
    let mut ctx = ProcessCtx { sample_rate: SR, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / SR as f64, playing: true, restart: false };
    let mut inputs = Box::new(Inputs {
        bufs: [[[0.0; 64]; 2]; MAX_INPUTS],
        connected: [false; MAX_INPUTS],
        midi: MidiBuf::default(),
        midi_connected: false,
    });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let mut events = Vec::new();
    let mut done = 0;
    while done < frames {
        inputs.midi.clear();
        feed(done, &mut inputs.midi);
        inputs.midi_connected = !inputs.midi.events().is_empty() || inputs.midi_connected;
        proc.process(&ctx, &inputs, &mut outputs, 64);
        for e in proc.midi_out().expect("sends MIDI").events() {
            events.push((done + e.frame as usize, e.data));
        }
        ctx.beat += ctx.beats_per_sample * 64.0;
        done += 64;
    }
    events
}

fn note_ons(events: &[(usize, [u8; 3])]) -> Vec<(usize, u8)> {
    events.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90 && m[2] > 0).map(|(f, m)| (*f, m[1])).collect()
}

#[test]
fn sequencer_plays_its_steps_in_time() {
    use crate::audio::modules::sequencer::{self, Sequencer};
    let p = Arc::new(ModuleParams::new(ModuleKind::Sequencer));
    for step in 0..sequencer::LEN {
        sequencer::set_on(&p, step, true);
    }
    // 120 BPM, sixteenths: a step every 6000 frames, half of it sounding.
    let events = run_generator(&mut Sequencer::new(p.clone()), 48_000, |_, _| {});
    let ons = note_ons(&events);
    let frames: Vec<usize> = ons.iter().map(|o| o.0).collect();
    assert_eq!(frames, [0, 6000, 12000, 18000, 24000, 30000, 36000, 42000]);
    // C3 major, degrees 0 2 4 2 5 4 2 1.
    let notes: Vec<u8> = ons.iter().map(|o| o.1).collect();
    assert_eq!(notes, [48, 52, 55, 52, 57, 55, 52, 50]);
    let offs: Vec<usize> = events.iter().filter(|(_, m)| m[0] & 0xF0 == 0x80).map(|e| e.0).collect();
    assert_eq!(offs[..3], [3000, 9000, 15000], "GATE 50% ends notes half way");
    assert!(events.iter().all(|(_, m)| m[0] & 0x0F == 0), "channel 1");

    // Swing pushes the off-beats late; switched-off steps rest.
    p.set(sequencer::SWING, 1.0);
    sequencer::set_on(&p, 2, false);
    let ons = note_ons(&run_generator(&mut Sequencer::new(p.clone()), 24_000, |_, _| {}));
    assert_eq!(ons.iter().map(|o| o.0).collect::<Vec<_>>(), [0, 9000, 21000]);

    // Stopping it ends the note that is sounding.
    p.set(sequencer::SWING, 0.0);
    p.set(sequencer::GATE, 1.0);
    let mut seq = Sequencer::new(p.clone());
    run_generator(&mut seq, 1_000, |_, _| {});
    p.set_toggle(false);
    let events = run_generator(&mut seq, 64, |_, _| {});
    assert_eq!(events, [(0, [0x80, 48, 0])]);
}

#[test]
fn arpeggiator_walks_the_held_chord() {
    use crate::audio::modules::arpeggiator::{self, Arpeggiator};
    let (_g, a, _e) = setup();
    let p = Arc::new(ModuleParams::new(ModuleKind::Arpeggiator));
    p.set(arpeggiator::OCTAVES, 2.0);
    let mut arp = Arpeggiator::new(p.clone(), a.midi.clone());
    // A chord arrives on the cable (played out of order), held for a while.
    let chord = |at: usize, buf: &mut MidiBuf| {
        if at == 0 {
            for key in [67, 60, 64] {
                buf.push(0, [0x90, key, 90]);
            }
        }
    };
    let notes: Vec<u8> = note_ons(&run_generator(&mut arp, 48_000, chord)).iter().map(|o| o.1).collect();
    assert_eq!(notes, [60, 64, 67, 72, 76, 79, 60, 64]);

    // Down, as played.
    p.set_mode(arpeggiator::DOWN);
    let mut arp = Arpeggiator::new(p.clone(), a.midi.clone());
    let notes: Vec<u8> = note_ons(&run_generator(&mut arp, 24_000, chord)).iter().map(|o| o.1).collect();
    assert_eq!(notes, [79, 76, 72, 67]);

    // Letting go stops it, unless LATCH holds the chord.
    for latch in [false, true] {
        p.set_mode(arpeggiator::UP);
        p.set_toggle(latch);
        let mut arp = Arpeggiator::new(p.clone(), a.midi.clone());
        let play_and_release = |at: usize, buf: &mut MidiBuf| {
            chord(at, buf);
            if at == 12_032 {
                for key in [60, 64, 67] {
                    buf.push(0, [0x80, key, 0]);
                }
            }
        };
        let ons = note_ons(&run_generator(&mut arp, 48_000, play_and_release));
        let late = ons.iter().filter(|o| o.0 > 13_000).count();
        assert_eq!(late > 0, latch, "latch {latch}: {ons:?}");
    }
}

#[test]
fn midi_cables_only_join_midi_ports() {
    use crate::graph::MIDI_PORT;
    let (mut g, a, _e) = setup();
    let seq = g.add_module(ModuleKind::Sequencer, pos2(0.0, 0.0), &a);
    let arp = g.add_module(ModuleKind::Arpeggiator, pos2(300.0, 0.0), &a);
    let osc = g.add_osc(pos2(600.0, 0.0), &a);
    let filter = g.add_module(ModuleKind::Filter, pos2(600.0, 300.0), &a);
    let node = |id| g.node(id).unwrap();
    assert_eq!(node(seq).output_ports(), [MIDI_PORT]);
    assert_eq!(node(arp).input_ports(), [MIDI_PORT]);
    assert_eq!(node(osc).input_ports(), [0, 1], "no MIDI on an oscillator");

    assert!(!g.connect(port(seq, MIDI_PORT), port(osc, 0), &a), "MIDI into audio");
    assert!(!g.connect(port(osc, 0), port(arp, MIDI_PORT), &a), "audio into MIDI");
    assert!(!g.connect(port(seq, MIDI_PORT), port(filter, MIDI_PORT), &a), "no such port");
    assert!(!g.connect(port(seq, 0), port(filter, 0), &a), "the sequencer has no audio out");
    assert!(g.connect(port(seq, MIDI_PORT), port(arp, MIDI_PORT), &a));
    assert_eq!(g.cables.len(), 1);
}

#[test]
fn a_sequencer_plays_a_midi_note_node_through_the_engine() {
    use crate::audio::modules::{midi_note, sequencer};
    use crate::graph::MIDI_PORT;
    let (mut g, a, mut e) = setup();
    let seq = g.add_module(ModuleKind::Sequencer, pos2(0.0, 0.0), &a);
    let note = g.add_module(ModuleKind::MidiNote, pos2(300.0, 0.0), &a);
    let mix = g.add_mixer(pos2(600.0, 0.0), &a);
    assert!(g.connect(port(seq, MIDI_PORT), port(note, MIDI_PORT), &a));
    g.connect(port(note, 0), port(mix, 0), &a);
    let sp = module_params(&g, seq);
    let np = module_params(&g, note);
    np.set(midi_note::ATTACK, 0.002);

    render_buf(&mut e, 24_000);
    assert_eq!(np.viz[midi_note::CABLE].get(), 1.0);
    assert!(np.viz[midi_note::SEQ].get() >= 2.0, "notes arrive on the cable");
    let played = np.viz[midi_note::NOTE].get() as u8;
    assert!((48..=72).contains(&played), "a note of the scale: {played}");
    assert!(sp.viz[sequencer::STEP].get() >= 0.0);
    // Its envelope follows the steps: the output moves.
    let buf = render_buf(&mut e, 24_000);
    assert!(rms_of(&buf) > 0.05);

    // A stopped sequencer leaves the node silent once it releases.
    sp.set_toggle(false);
    np.set(midi_note::RELEASE, 0.01);
    render_buf(&mut e, 12_000);
    assert!(np.viz[midi_note::ENV].get() < 0.01);
}

#[test]
fn tempo_and_steps_are_saved() {
    use crate::audio::modules::sequencer;
    use crate::graph::MIDI_PORT;
    let (mut g, a, _e) = setup();
    g.tempo = 128.5;
    let seq = g.add_module(ModuleKind::Sequencer, pos2(0.0, 0.0), &a);
    let note = g.add_module(ModuleKind::MidiNote, pos2(300.0, 0.0), &a);
    g.connect(port(seq, MIDI_PORT), port(note, MIDI_PORT), &a);
    let p = module_params(&g, seq);
    sequencer::set_degree(&p, 3, 11);
    sequencer::set_on(&p, 5, false);
    let saved = crate::project::snapshot(&g);

    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    assert_eq!(h.tempo, 128.5);
    assert_eq!(h.cables.len(), 1, "the MIDI cable is kept");
    let q = module_params(&h, h.nodes[0].id);
    assert_eq!(sequencer::degree(&q, 3), 11);
    assert!(!sequencer::is_on(&q, 5) && sequencer::is_on(&q, 4));
    assert_eq!(crate::project::snapshot(&h), saved);
}

/// An instrument plugin, if installed. Its samples load in the background
/// on the app's run loop, so tests see silence: they check the wiring.
fn installed_instrument() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from("/Library/Audio/Plug-Ins/VST3/Originals - Cinematic Pads.vst3");
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipping: {} is not installed", path.display());
        None
    }
}

#[test]
fn vst3_instruments_take_a_midi_cable() {
    use crate::graph::MIDI_PORT;
    let Some(path) = installed_instrument() else { return };
    let (mut g, a, mut e) = setup();
    let seq = g.add_module(ModuleKind::Sequencer, pos2(0.0, 0.0), &a);
    let synth = g.add_plugin(&path, pos2(300.0, 0.0), &a).expect("loads");
    let mix = g.add_mixer(pos2(600.0, 0.0), &a);
    let node = g.node(synth).unwrap();
    assert!(node.midi_in());
    assert_eq!(node.input_ports(), [MIDI_PORT], "an instrument: MIDI in, no audio in");
    assert!(g.connect(port(seq, MIDI_PORT), port(synth, MIDI_PORT), &a));
    assert!(g.connect(port(synth, 0), port(mix, 0), &a));
    let buf = render_buf(&mut e, 24_000);
    assert!(buf.iter().all(|s| s.is_finite()));

    // Unplugging mid-note is safe (the plugin gets note-offs).
    let cable = *g.cables.iter().find(|c| c.to == port(synth, MIDI_PORT)).unwrap();
    g.disconnect(cable, &a);
    let buf = render_buf(&mut e, 4_800);
    assert!(buf.iter().all(|s| s.is_finite()));

    // The MIDI cable is saved with the project.
    g.connect(port(seq, MIDI_PORT), port(synth, MIDI_PORT), &a);
    let saved = crate::project::snapshot(&g);
    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    assert!(h.cables.iter().any(|c| c.to.port == MIDI_PORT));
}

#[test]
fn resting_on_a_node_or_module_shows_its_type() {
    let (mut g, a, _e) = setup();
    let reverb = g.add_module(ModuleKind::Reverb, pos2(0.0, 300.0), &a);
    g.node_mut(reverb).unwrap().title = "MY SPACE".to_owned();
    let pair = [
        g.add_module(ModuleKind::Drone, pos2(0.0, 0.0), &a),
        g.add_module(ModuleKind::Filter, pos2(300.0, 0.0), &a),
    ];
    let gid = g.create_group(&pair).unwrap();
    g.rename_group(gid, "Pad voice");
    g.set_group_collapsed(gid, true);

    let ctx = egui::Context::default();
    let mut editor = crate::ui::canvas::Editor::new();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    // Runs a frame at `time` and returns every text drawn.
    let run = |editor: &mut crate::ui::canvas::Editor, g: &mut Graph, time: f64, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, &a));
        out.textures_delta.clear();
        fn texts(shape: &egui::Shape, into: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => into.push(t.galley.job.text.clone()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, into)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut found));
        found
    };
    run(&mut editor, &mut g, 0.0, vec![]);
    let to_screen = |e: &crate::ui::canvas::Editor, p: egui::Pos2| (e.pan + p.to_vec2() * e.zoom).to_pos2();
    let on_reverb = to_screen(&editor, g.node(reverb).unwrap().pos + egui::vec2(100.0, 90.0));
    let on_card = to_screen(&editor, crate::ui::groups::frame_rect(&g, g.group(gid).unwrap()).unwrap().min)
        + egui::vec2(100.0, 12.0) * editor.zoom;
    let label = |texts: &[String], what: &str| texts.iter().any(|t| t.contains(what));

    // Only once the pointer has rested a moment.
    let texts = run(&mut editor, &mut g, 1.0, vec![egui::Event::PointerMoved(on_reverb)]);
    assert!(!label(&texts, "hall · shimmer"), "no label straight away");
    let texts = run(&mut editor, &mut g, 1.6, vec![]);
    assert!(label(&texts, "Reverb") && label(&texts, "hall · shimmer · freeze"), "{texts:?}");

    // A renamed module says what it holds.
    run(&mut editor, &mut g, 2.0, vec![egui::Event::PointerMoved(on_card)]);
    let texts = run(&mut editor, &mut g, 2.6, vec![]);
    assert!(label(&texts, "Module") && label(&texts, "Drone · Filter"), "{texts:?}");

    // Not while dragging.
    let press = egui::Event::PointerButton {
        pos: on_card,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    };
    let texts = run(&mut editor, &mut g, 2.7, vec![press]);
    assert!(!label(&texts, "Drone · Filter"));
}

#[test]
fn palette_categories_fold() {
    use crate::ui::palette::Palette;
    let ctx = egui::Context::default();
    let mut palette = Palette::new();
    let mut time = 0.0;
    // Runs a frame; returns each text drawn and where.
    let mut run = |palette: &mut Palette, events: Vec<egui::Event>| {
        time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut requests = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            palette.show(ui, &[], &[], &mut requests);
        });
        out.textures_delta.clear();
        let mut texts = Vec::new();
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape {
                texts.push((t.galley.job.text.clone(), t.pos + t.galley.rect.center().to_vec2()));
            }
        }
        texts
    };
    let shows = |texts: &[(String, egui::Pos2)], what: &str| texts.iter().any(|(t, _)| t == what);
    let click = |p: egui::Pos2| {
        let button = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(p), button(true), button(false)]
    };

    let texts = run(&mut palette, vec![]);
    assert!(shows(&texts, "Oscillator") && shows(&texts, "Drone"), "open at first");
    let heading = texts.iter().find(|(t, _)| t == "SOURCES").unwrap().1;
    run(&mut palette, click(heading));
    let mut texts = Vec::new();
    for _ in 0..10 {
        texts = run(&mut palette, vec![]);
    }
    assert!(!shows(&texts, "Oscillator") && shows(&texts, "SOURCES"), "folded");
    assert!(shows(&texts, "10"), "a folded category shows its count");
    assert!(shows(&texts, "Reverb"), "the others stay open");

    // Searching opens what matches; clearing it folds it again.
    let field = egui::pos2(100.0, 14.0);
    run(&mut palette, click(field));
    let texts = run(&mut palette, vec![egui::Event::Text("drone".into())]);
    assert!(shows(&texts, "Drone") && !shows(&texts, "Reverb"));
    run(&mut palette, vec![egui::Event::Key {
        key: egui::Key::Backspace,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { command: true, mac_cmd: true, ..Default::default() },
    }]);
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = run(&mut palette, vec![]);
    }
    assert!(!shows(&texts, "Drone") && shows(&texts, "Reverb"), "{:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------
// MIDIcer
// ---------------------------------------------------------------------------

fn midicer() -> (Arc<ModuleParams>, crate::audio::modules::midicer::Midicer) {
    let (_g, a, _e) = setup();
    let p = Arc::new(ModuleParams::new(ModuleKind::Midicer));
    let m = crate::audio::modules::midicer::Midicer::new(p.clone(), a.midi.clone());
    (p, m)
}

#[test]
fn midicer_plays_the_manuals_basic_pattern() {
    use crate::audio::modules::midicer::data;
    let (p, mut m) = midicer();
    // 120 BPM, 1/16 notes: one every 6000 frames, C3 or C4 only.
    let events = run_generator(&mut m, 2 * 96_000, |_, _| {});
    let ons = note_ons(&events);
    assert_eq!(ons.len(), 32, "two bars of sixteenths");
    assert!(ons.iter().all(|o| o.0 % 6000 == 0 && (o.1 == 48 || o.1 == 60)), "{ons:?}");
    // DICE: the second bar repeats the first.
    let bar = |b: usize| ons[b * 16..(b + 1) * 16].iter().map(|o| o.1).collect::<Vec<_>>();
    assert_eq!(bar(0), bar(1));
    assert!(bar(0).contains(&48) && bar(0).contains(&60), "both octaves: {:?}", bar(0));
    // GATE 80%: each note ends before the next.
    let offs: Vec<usize> = events.iter().filter(|(_, m)| m[0] & 0xF0 == 0x80).map(|e| e.0).collect();
    assert_eq!(offs[0], 4800);

    // Different dice, a different pattern.
    p.data[data::SEED_M].set(777.0);
    let (_, mut m2) = (p.clone(), crate::audio::modules::midicer::Midicer::new(p.clone(), setup().1.midi.clone()));
    let other: Vec<u8> = note_ons(&run_generator(&mut m2, 96_000, |_, _| {})).iter().map(|o| o.1).collect();
    assert_ne!(other, bar(0));
}

#[test]
fn midicer_live_mode_never_repeats() {
    use crate::audio::modules::midicer::{Midicer, data};
    let (p, _) = midicer();
    for pc in 0..12 {
        p.data[data::WEIGHTS + pc].set(1.0);
    }
    let midi = setup().1.midi.clone();
    let first_bar = |p: &Arc<ModuleParams>| -> Vec<u8> {
        let mut m = Midicer::new(p.clone(), midi.clone());
        note_ons(&run_generator(&mut m, 96_000, |_, _| {})).iter().map(|o| o.1).collect()
    };
    let kept = first_bar(&p);

    p.data[data::DICE_M].set(0.0);
    let mut m = Midicer::new(p.clone(), midi.clone());
    let ons = note_ons(&run_generator(&mut m, 2 * 96_000, |_, _| {}));
    let bar = |b: usize| ons[b * 16..(b + 1) * 16].iter().map(|o| o.1).collect::<Vec<_>>();
    assert_ne!(bar(0), bar(1), "LIVE: no repeating pattern");

    // Back to DICE: the kept values return.
    p.data[data::DICE_M].set(1.0);
    assert_eq!(first_bar(&p), kept);
}

#[test]
fn midicer_rests_legato_and_mute() {
    use crate::audio::modules::midicer::{LEGATO, REST, data};
    let (p, mut m) = midicer();
    p.set(REST, 1.0);
    assert!(note_ons(&run_generator(&mut m, 96_000, |_, _| {})).is_empty(), "all rests");

    let (p, mut m) = midicer();
    p.set(LEGATO, 1.0);
    let events = run_generator(&mut m, 96_000, |_, _| {});
    let ons = note_ons(&events);
    // Every note runs into the next: ties merge repeated pitches, and a new
    // pitch starts before the old one stops.
    assert!(ons.len() < 16, "ties: {}", ons.len());
    for w in events.windows(2) {
        if w[1].1[0] & 0xF0 == 0x80 {
            assert_eq!(w[0].0, w[1].0, "a note-off only comes with the next note-on");
        }
    }

    let (p, mut m) = midicer();
    p.data[data::MUTE].set(1.0);
    assert!(note_ons(&run_generator(&mut m, 96_000, |_, _| {})).is_empty(), "muted");
}

#[test]
fn midicer_pattern_length_and_variation() {
    use crate::audio::modules::midicer::{FIRST, LAST, NOTE, VARIATION, data};
    let (p, mut m) = midicer();
    // One beat long (steps 1…4): the dice repeat every beat.
    p.set(FIRST, 1.0);
    p.set(LAST, 4.0);
    for pc in 0..12 {
        p.data[data::WEIGHTS + pc].set(1.0);
    }
    let ons = note_ons(&run_generator(&mut m, 96_000, |_, _| {}));
    let beat = |b: usize| ons[b * 4..(b + 1) * 4].iter().map(|o| o.1).collect::<Vec<_>>();
    assert_eq!(beat(0), beat(1));
    assert_eq!(beat(1), beat(3));

    // VARIATION to the right adds shorter notes (1/32 allowed).
    let (p, mut m) = midicer();
    p.set(NOTE, 4.0); // 1/8: 12000 frames
    p.set(VARIATION, 1.0);
    p.data[data::THIRTY_SECOND].set(1.0);
    let ons = note_ons(&run_generator(&mut m, 96_000, |_, _| {}));
    let gaps: Vec<usize> = ons.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(gaps.iter().all(|&g| [12_000, 6000, 3000].contains(&g)), "{gaps:?}");
    assert!(gaps.iter().any(|&g| g < 12_000), "some shorter notes");
    assert!(ons.len() > 8);
}

#[test]
fn midicer_follows_gates_and_quantizes() {
    use crate::audio::modules::midicer::{QUANT1, SEQ_GATE};
    // SEQ+G: a note for each note on the MIDI input, ended by its note-off.
    let (p, mut m) = midicer();
    p.set_mode(SEQ_GATE);
    let gates = |at: usize, buf: &mut MidiBuf| match at {
        3200 => buf.push(10, [0x90, 70, 90]),
        6400 => buf.push(0, [0x80, 70, 0]),
        9600 => buf.push(5, [0x90, 72, 90]),
        _ => {}
    };
    let events = run_generator(&mut m, 12_800, gates);
    let ons = note_ons(&events);
    assert_eq!(ons.iter().map(|o| o.0).collect::<Vec<_>>(), [3210, 9605]);
    assert!(ons.iter().all(|o| o.1 == 48 || o.1 == 60));
    assert!(events.iter().any(|(f, m)| *f == 6400 && m[0] & 0xF0 == 0x80));

    // QUANT1: CV2 → the enabled notes, once a beat. With only C enabled and
    // CV2 at +1 (the top of LOW…HIGH), that is C4 every beat.
    let (p, mut m) = midicer();
    p.set_mode(QUANT1);
    const SR: f32 = 48_000.0;
    let mut ctx = ProcessCtx { sample_rate: SR, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / SR as f64, playing: true, restart: false };
    let mut inputs = Box::new(Inputs {
        bufs: [[[0.0; 64]; 2]; MAX_INPUTS],
        connected: [false; MAX_INPUTS],
        midi: MidiBuf::default(),
        midi_connected: false,
    });
    inputs.connected[1] = true;
    inputs.bufs[1] = [[1.0; 64]; 2];
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let mut ons = Vec::new();
    for block in 0..1500 {
        m.process(&ctx, &inputs, &mut outputs, 64);
        for e in m.midi_out().unwrap().events() {
            if e.data[0] & 0xF0 == 0x90 {
                ons.push((block * 64 + e.frame as usize, e.data[1]));
            }
        }
        ctx.beat += ctx.beats_per_sample * 64.0;
    }
    assert_eq!(ons, [(0, 60), (24_000, 60), (48_000, 60), (72_000, 60)]);
}

#[test]
fn stop_rewinds_and_start_plays_from_the_top() {
    use crate::audio::modules::{midi_note, sequencer};
    use crate::graph::MIDI_PORT;
    let shared = Arc::new(EngineShared::new());
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let seq = g.add_module(ModuleKind::Sequencer, pos2(0.0, 0.0), &a);
    let note = g.add_module(ModuleKind::MidiNote, pos2(300.0, 0.0), &a);
    g.connect(port(seq, MIDI_PORT), port(note, MIDI_PORT), &a);
    let sp = module_params(&g, seq);
    let np = module_params(&g, note);
    for step in 0..sequencer::LEN {
        sequencer::set_on(&sp, step, true);
    }

    render_buf(&mut e, 62_400); // 2.6 beats
    assert_eq!(sp.viz[sequencer::STEP].get(), 10.0);

    // Stop: silence, and back to the start.
    shared.rewinds.fetch_add(1, Ordering::Relaxed);
    shared.running.store(false, Ordering::Relaxed);
    let silence = render_buf(&mut e, 24_000);
    assert!(silence.iter().all(|&s| s == 0.0));

    // Start: from step 1 and its note (C3), on beat 0.
    shared.running.store(true, Ordering::Relaxed);
    render_buf(&mut e, 512);
    assert_eq!(sp.viz[sequencer::STEP].get(), 0.0);
    assert_eq!(np.viz[midi_note::NOTE].get(), 48.0);
    assert!(shared.beat.get() < 0.05, "the transport starts at 0: {}", shared.beat.get());
}

// ---------------------------------------------------------------------------
// Delays
// ---------------------------------------------------------------------------

/// Runs an effect on a single click at frame 0 (120 BPM) and returns the
/// left and right outputs.
fn click_response(proc: &mut dyn Processor, frames: usize) -> (Vec<f32>, Vec<f32>) {
    const SR: f32 = 48_000.0;
    let mut ctx = ProcessCtx { sample_rate: SR, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / SR as f64, playing: true, restart: false };
    let mut inputs = Box::new(Inputs {
        bufs: [[[0.0; 64]; 2]; MAX_INPUTS],
        connected: [false; MAX_INPUTS],
        midi: MidiBuf::default(),
        midi_connected: false,
    });
    inputs.connected[0] = true;
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for block in 0..frames / 64 {
        let click = if block == 0 { 1.0 } else { 0.0 };
        inputs.bufs[0] = [[0.0; 64]; 2];
        inputs.bufs[0][0][0] = click;
        inputs.bufs[0][1][0] = click;
        proc.process(&ctx, &inputs, &mut outputs, 64);
        l.extend_from_slice(&outputs[0][0]);
        r.extend_from_slice(&outputs[0][1]);
        ctx.beat += ctx.beats_per_sample * 64.0;
    }
    (l, r)
}

/// Where the loudest sample is between `from` and `to`, and how loud.
fn loudest(x: &[f32], from: usize, to: usize) -> (usize, f32) {
    (from..to.min(x.len())).map(|i| (i, x[i].abs())).fold((from, 0.0), |a, b| if b.1 > a.1 { b } else { a })
}

#[test]
fn delay_syncs_to_the_tempo() {
    use crate::audio::modules::delay::{self, Delay};
    let p = Arc::new(ModuleParams::new(ModuleKind::Delay));
    p.set(delay::MIX, 1.0);
    p.set(delay::FEEDBACK, 0.0);
    p.set(delay::PING, 0.0);
    p.set(delay::WOW, 0.0);
    p.set_toggle(true);
    // 1/8D at 120 BPM: 0.75 beat = 375 ms = 18000 frames.
    p.set(delay::DIV, 7.0);
    let (l, _) = click_response(&mut Delay::new(p.clone(), 48_000.0), 48_000);
    let (at, level) = loudest(&l, 1000, 48_000);
    assert!(level > 0.1 && at.abs_diff(18_000) <= 2, "echo at {at} ({level})");
    // Off sync: TIME rules.
    p.set_toggle(false);
    p.set(delay::TIME, 0.25);
    let (l, _) = click_response(&mut Delay::new(p.clone(), 48_000.0), 48_000);
    assert!(loudest(&l, 1000, 48_000).0.abs_diff(12_000) <= 2);
}

#[test]
fn ping_pong_echoes_alternate_sides() {
    use crate::audio::modules::pingpong::{self, PingPong};
    let p = Arc::new(ModuleParams::new(ModuleKind::PingPong));
    p.set(pingpong::MIX, 1.0);
    p.set(pingpong::FEEDBACK, 0.8);
    p.set(pingpong::TONE, 1.0);
    // Synced 1/8 at 120 BPM: an echo every 250 ms (12000 frames).
    p.set(pingpong::DIV, 5.0);
    let d = 12_000;
    let (l, r) = click_response(&mut PingPong::new(p.clone(), 48_000.0), 4 * d + 2000);
    let near = |x: &[f32], k: usize| loudest(x, k * d - 300, k * d + 300).1;
    // Left, right, left: each echo on one side only (WIDTH 100%).
    assert!(near(&l, 1) > 0.2 && near(&r, 1) < 0.01, "1st: L {} R {}", near(&l, 1), near(&r, 1));
    assert!(near(&r, 2) > 0.1 && near(&l, 2) < 0.01, "2nd: L {} R {}", near(&l, 2), near(&r, 2));
    assert!(near(&l, 3) > 0.05 && near(&r, 3) < 0.01);
    assert!(near(&l, 3) < near(&l, 1), "they fade");

    // R → L starts on the right; WIDTH 0 puts every echo in the middle.
    p.set_mode(pingpong::RIGHT_FIRST);
    let (l, r) = click_response(&mut PingPong::new(p.clone(), 48_000.0), 2 * d + 2000);
    assert!(near(&r, 1) > 0.2 && near(&l, 1) < 0.01);
    p.set(pingpong::WIDTH, 0.0);
    let (l, r) = click_response(&mut PingPong::new(p.clone(), 48_000.0), 2 * d + 2000);
    assert!((near(&l, 1) - near(&r, 1)).abs() < 1e-3);
}

#[test]
fn ping_pong_ducks_under_the_input() {
    use crate::audio::modules::pingpong;
    let mut levels = Vec::new();
    for duck in [0.0, 1.0] {
        let (mut g, a, mut e) = setup();
        let (m, p) = patch(&mut g, &a, ModuleKind::PingPong);
        let _ = m;
        p.set(pingpong::MIX, 1.0);
        p.set(pingpong::DUCK, duck);
        render_buf(&mut e, 48_000);
        levels.push(rms_of(&render_buf(&mut e, 24_000)));
    }
    assert!(levels[1] < levels[0] * 0.3, "ducked {} vs {}", levels[1], levels[0]);
}

#[test]
fn stop_silences_sources_but_tails_ring_on() {
    use crate::audio::modules::reverb;
    let shared = Arc::new(EngineShared::new());
    let (a, mut e) = audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let src = source(&mut g, &a);
    let rev = g.add_module(ModuleKind::Reverb, pos2(300.0, 0.0), &a);
    let mix = g.add_mixer(pos2(600.0, 0.0), &a);
    g.connect(port(src, 0), port(rev, 0), &a);
    g.connect(port(rev, 0), port(mix, 0), &a);
    let rp = module_params(&g, rev);
    rp.set(reverb::MIX, 0.5);
    rp.set(reverb::DECAY, 6.0);
    render_buf(&mut e, 48_000);

    // Stop: the oscillator fades out within a few tens of ms, while the
    // reverb's tail goes on.
    shared.rewinds.fetch_add(1, Ordering::Relaxed);
    shared.running.store(false, Ordering::Relaxed);
    let after = render_buf(&mut e, 48_000);
    assert!(rms_of(&after[..96]) > 0.05, "no cut: it fades");
    assert!(rms_of(&after[40_000..]) > 0.005, "the tail rings on: {}", rms_of(&after[40_000..]));
    // The source itself is silent once faded.
    let dry = g.add_mixer(pos2(600.0, 300.0), &a);
    g.connect(port(src, 0), port(dry, 0), &a);
    rp.set(reverb::MIX, 0.0);
    render_buf(&mut e, 4_800);
    assert!(rms_of(&render_buf(&mut e, 4_800)) < 1e-4);

    // Start: it sounds again.
    shared.running.store(true, Ordering::Relaxed);
    render_buf(&mut e, 4_800);
    assert!(rms_of(&render_buf(&mut e, 4_800)) > 0.05);
}

#[test]
fn stop_ends_the_notes_of_sequencers() {
    use crate::audio::modules::{midi_note, sequencer};
    use crate::graph::MIDI_PORT;
    let shared = Arc::new(EngineShared::new());
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let seq = g.add_module(ModuleKind::Sequencer, pos2(0.0, 0.0), &a);
    let note = g.add_module(ModuleKind::MidiNote, pos2(300.0, 0.0), &a);
    g.connect(port(seq, MIDI_PORT), port(note, MIDI_PORT), &a);
    let sp = module_params(&g, seq);
    let np = module_params(&g, note);
    sp.set(sequencer::GATE, 1.0);
    np.set(midi_note::RELEASE, 0.05);
    for step in 0..sequencer::LEN {
        sequencer::set_on(&sp, step, true);
    }
    render_buf(&mut e, 24_000);
    assert!(np.viz[midi_note::ENV].get() > 0.5, "a note is held");
    shared.running.store(false, Ordering::Relaxed);
    render_buf(&mut e, 24_000);
    assert!(np.viz[midi_note::ENV].get() < 0.01, "Stop ended it: the envelope released");
    assert_eq!(sp.viz[sequencer::STEP].get(), -1.0);
}

#[test]
fn idle_effects_rest_and_wake_up() {
    use crate::audio::modules::reverb;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Reverb);
    p.set(reverb::DECAY, 0.3);
    p.set(reverb::MIX, 1.0);
    render_buf(&mut e, 24_000);
    // Input gone: the tail dies away, then the reverb rests (exact silence).
    unplug_input(&mut g, &a, m);
    assert!(rms_of(&render_buf(&mut e, 4_800)) > 1e-3, "the tail first");
    render_buf(&mut e, 96_000);
    assert!(render_buf(&mut e, 4_800).iter().all(|&s| s == 0.0), "resting");
    // Input back: it sounds at once.
    let src = g.nodes.iter().find(|n| matches!(n.kind, NodeKind::Osc(_))).unwrap().id;
    g.connect(port(src, 0), port(m, 0), &a);
    assert!(rms_of(&render_buf(&mut e, 4_800)) > 1e-3, "awake");
}

#[test]
fn the_output_is_limited_not_clipped() {
    let shared = Arc::new(EngineShared::new());
    let (a, mut e) = audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let mix = g.add_mixer(pos2(600.0, 0.0), &a);
    let m = mixer_params(&g, mix);
    m.master.set(1.0);
    for ch in 0..3 {
        let x = source(&mut g, &a);
        osc_params(&g, x).set(osc::LEVEL, 1.0);
        osc_params(&g, x).set(osc::FREQ, 110.0 * (ch + 1) as f32);
        m.fader[ch].set(1.0);
        g.connect(port(x, 0), port(mix, ch), &a);
    }
    render_buf(&mut e, 24_000);
    let out = render_buf(&mut e, 24_000);
    let peak = out.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    assert!(peak <= 0.967, "kept under the ceiling: {peak}");
    // No flat tops: hard clipping would hold full scale for many samples.
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let at_top = left.windows(2).filter(|w| w[0].abs() > 0.96 && w[0] == w[1]).count();
    assert_eq!(at_top, 0);
    assert!(shared.limiting.take() > 3.0, "the limiter worked");
}

#[test]
fn removed_nodes_are_freed_off_the_audio_thread() {
    use crate::audio::engine::{Command, Slot};
    use std::sync::atomic::AtomicBool;
    struct Probe(Arc<AtomicBool>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    impl Processor for Probe {
        fn process(&mut self, _: &ProcessCtx, _: &Inputs, _: &mut [crate::audio::engine::Buf], _: usize) {}
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let mut e = Engine::new(rx, Arc::new(EngineShared::new()), 48_000.0);
    let trash = e.take_trash().unwrap();
    let freed = Arc::new(AtomicBool::new(false));
    tx.send(Command::AddNode(1, Slot::new(Box::new(Probe(freed.clone())), 1))).unwrap();
    render_buf(&mut e, 512);
    tx.send(Command::RemoveNode(1)).unwrap();
    render_buf(&mut e, 512);
    assert!(!freed.load(Ordering::Relaxed), "the engine did not free it");
    drop(trash.try_recv().expect("handed back"));
    assert!(freed.load(Ordering::Relaxed), "freed where it was received");
}

/// Runs a processor at 120 BPM, transport playing, on `input(frame)` in
/// both channels.
fn drive(proc: &mut dyn Processor, frames: usize, from: usize, input: impl Fn(usize) -> f32) -> (Vec<f32>, Vec<f32>) {
    const SR: f32 = 48_000.0;
    let mut ctx = ProcessCtx {
        sample_rate: SR,
        smooth: 0.01,
        beat: from as f64 * 2.0 / SR as f64,
        beats_per_sample: 2.0 / SR as f64,
        playing: true,
        restart: false,
    };
    let mut inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    inputs.connected[0] = true;
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for block in 0..frames / 64 {
        for i in 0..64 {
            let x = input(from + block * 64 + i);
            inputs.bufs[0][0][i] = x;
            inputs.bufs[0][1][i] = x;
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        l.extend_from_slice(&outputs[0][0]);
        r.extend_from_slice(&outputs[0][1]);
        ctx.beat += ctx.beats_per_sample * 64.0;
    }
    (l, r)
}

/// A Frakture with only the given effects on, nothing mapped and nothing
/// mutated.
fn frakture_with(sections: &[usize]) -> Arc<ModuleParams> {
    use crate::audio::modules::frakture::{self, data};
    let p = Arc::new(ModuleParams::new(ModuleKind::Frakture));
    for s in 0..4 {
        p.data[data::SECTION_ON + s].set(if sections.contains(&s) { 1.0 } else { 0.0 });
    }
    for i in 0..frakture::MAPPABLE {
        frakture::set_mapping(&p, i, None);
    }
    for i in [frakture::FEEDBACK, frakture::SWARM, frakture::GLIDE, frakture::SHIMMER, frakture::RESONATE, frakture::BLUR, frakture::ERODE] {
        p.set(i, 0.0);
    }
    p
}

/// A saw whose period doesn't divide the slice lengths used below.
fn saw(frame: usize) -> f32 {
    0.4 * ((frame as f64 * 233.0 / 48_000.0).fract() as f32 * 2.0 - 1.0)
}

#[test]
fn frakture_gate_drops_the_off_steps() {
    use crate::audio::modules::frakture::{self, Frakture, data};
    let p = frakture_with(&[frakture::GATE]);
    p.set(frakture::GATE_STEPS, 2.0);
    p.set(frakture::GATE_RATE, 10.0); // 1/4: a beat, 24000 frames at 120 BPM
    p.set(frakture::GATE_ATTACK, 0.0);
    p.set(frakture::GATE_RELEASE, 0.0);
    p.data[data::GATE_PATTERN].set(1.0);
    p.data[data::GATE_PATTERN + 1].set(0.0);
    let (l, _) = drive(&mut Frakture::new(p, 48_000.0), 48_000, 0, |_| 0.5);
    assert!(l[2_000..22_000].iter().all(|s| (s - 0.5).abs() < 0.01), "the on step passes");
    assert!(l[26_000..46_000].iter().all(|s| s.abs() < 0.01), "the off step is silent");
}

#[test]
fn frakture_repeat_plays_the_slice_again() {
    use crate::audio::modules::frakture::{self, Frakture, REPEAT_DIVS, data, div_norm};
    let p = frakture_with(&[frakture::REPEAT]);
    p.set(frakture::REP_AMOUNT, 4.0);
    p.set(frakture::REP_TRIGGER, 15.0); // every bar: 96000 frames
    p.set(frakture::REP_OFFSET, 0.0);
    p.set(frakture::REP_DUCK, 1.0);
    p.data[data::REPEAT_SYNC].set(1.0);
    p.set(frakture::REP_RATE, div_norm(4, REPEAT_DIVS)); // 1/16: 6000 frames
    let (l, _) = drive(&mut Frakture::new(p, 48_000.0), 130_000, 0, saw);
    // A slice taken on the bar (96000…102000), then played again 4 times
    // (the dry signal ducks out over the first few milliseconds).
    let slice = 6_000;
    for n in 108_200..113_800 {
        assert!((l[n] - l[n + slice]).abs() < 1e-3, "repeat differs at {n}: {} vs {}", l[n], l[n + slice]);
        assert!((l[n] - saw(n - 12_000)).abs() < 1e-3, "not the slice at {n}");
    }
    assert!((108_200..113_800).any(|n| (l[n] - saw(n)).abs() > 0.1), "not just the input");
    // After the fourth repeat the input is back.
    assert!((128_000..129_900).all(|n| (l[n] - saw(n)).abs() < 0.01), "the input is back");
}

#[test]
fn frakture_freeze_keeps_the_grains_playing() {
    use crate::audio::modules::frakture::{self, Frakture, data, layer, lp};
    let p = frakture_with(&[frakture::GRAINS]);
    p.data[data::PLAYHEAD_MODE].set(frakture::MANUAL as f32);
    p.set(lp(0, layer::PLAYHEAD), 0.5);
    p.set(lp(0, layer::DRY), 0.001);
    p.set(lp(0, layer::WET), 1.0);
    let mut fx = Frakture::new(p.clone(), 48_000.0);
    drive(&mut fx, 96_000, 0, saw);
    p.data[data::FREEZE].set(1.0);
    let (held, _) = drive(&mut fx, 192_000, 96_000, |_| 0.0);
    assert!(rms_of(&held[96_000..]) > 0.02, "frozen grains fell silent: {}", rms_of(&held[96_000..]));
    // Recording again: silence fills the buffer and the grains die out.
    p.data[data::FREEZE].set(0.0);
    drive(&mut fx, 144_000, 288_000, |_| 0.0);
    let (after, _) = drive(&mut fx, 48_000, 432_000, |_| 0.0);
    assert!(peak_of(&after) < 1e-4, "still sounding: {}", peak_of(&after));
}

#[test]
fn frakture_hexagon_moves_mapped_knobs() {
    use crate::audio::modules::frakture::{self, Frakture, data, viz};
    let p = frakture_with(&[]);
    frakture::set_mapping(&p, frakture::SPACE_MIX, Some((2, 0.2, 0.8)));
    p.data[data::SLIDER_START + 1].set(1.0);
    p.data[data::MAGNET].set(frakture::motion::SELECT as f32);
    let mut fx = Frakture::new(p.clone(), 48_000.0);
    drive(&mut fx, 640, 0, |_| 0.0);
    assert!((p.viz[viz::EFFECTIVE + frakture::SPACE_MIX].get() - 0.8).abs() < 1e-4);
    assert_eq!(p.viz[viz::EFFECTIVE + frakture::SPACE_LENGTH].get(), -1.0, "not mapped");
}

#[test]
fn frakture_paths_and_mappings_are_saved() {
    use crate::audio::modules::frakture::{self, data, motion};
    let (mut g, a, _e) = setup();
    let fr = g.add_module(ModuleKind::Frakture, pos2(0.0, 0.0), &a);
    let p = module_params(&g, fr);
    frakture::set_mapping(&p, frakture::REP_DUCK, Some((3, 0.1, 0.9)));
    p.set(frakture::lp(1, frakture::layer::PITCH), -7.0);
    p.data[data::ROUTING].set(3.0);
    p.data[data::ROUTING + 3].set(0.0);
    let pts = [
        motion::PathPoint { x: -0.5, y: 0.2, bend: 0.3, clicked: true },
        motion::PathPoint { x: 0.4, y: -0.1, bend: 0.0, clicked: false },
    ];
    motion::set_drawn(&p, &pts, false);
    motion::set_rec_point(&p, 0, motion::PathPoint { x: 0.25, y: 0.5, bend: 0.0, clicked: true });
    p.data[data::REC_COUNT].set(1.0);
    let saved = crate::project::snapshot(&g);

    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    let q = module_params(&h, h.nodes[0].id);
    assert_eq!(frakture::mapping(&q, frakture::REP_DUCK), Some((3, 0.1, 0.9)));
    assert_eq!(q.get(frakture::lp(1, frakture::layer::PITCH)), -7.0);
    assert_eq!(frakture::routing(&q), [frakture::SPACE, frakture::GATE, frakture::REPEAT, frakture::GRAINS]);
    assert_eq!(motion::drawn_count(&q), 2);
    assert_eq!(motion::drawn_point(&q, 0), pts[0]);
    assert_eq!(motion::drawn_point(&q, 1), pts[1]);
    assert_eq!(motion::rec_point(&q, 0).x, 0.25);
    assert_eq!(crate::project::snapshot(&h), saved);
}

#[test]
fn projects_with_the_old_fractal_name_still_load() {
    let (mut g, a, _e) = setup();
    g.add_module(ModuleKind::Frakture, pos2(0.0, 0.0), &a);
    let json = serde_json::to_string(&crate::project::snapshot(&g)).unwrap();
    let saved: crate::project::Project = serde_json::from_str(&json.replace("\"Frakture\"", "\"Fractal\"")).unwrap();
    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    assert_eq!(module_params(&h, h.nodes[0].id).kind, ModuleKind::Frakture);
}

/// Runs a processor at 48 kHz on `main(frame)` (L, R), with `side` on the
/// second input if given.
fn drive_stereo(
    proc: &mut dyn Processor,
    frames: usize,
    main: impl Fn(usize) -> (f32, f32),
    side: Option<&dyn Fn(usize) -> (f32, f32)>,
) -> (Vec<f32>, Vec<f32>) {
    let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
    let mut inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    inputs.connected[0] = true;
    inputs.connected[1] = side.is_some();
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for block in 0..frames / 64 {
        for i in 0..64 {
            let n = block * 64 + i;
            (inputs.bufs[0][0][i], inputs.bufs[0][1][i]) = main(n);
            if let Some(sc) = side {
                (inputs.bufs[1][0][i], inputs.bufs[1][1][i]) = sc(n);
            }
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        l.extend_from_slice(&outputs[0][0]);
        r.extend_from_slice(&outputs[0][1]);
    }
    (l, r)
}

fn sine(hz: f32, amp: f32) -> impl Fn(usize) -> (f32, f32) {
    move |n| {
        let v = amp * (std::f32::consts::TAU * hz * n as f32 / 48_000.0).sin();
        (v, v)
    }
}

/// Level of the second half, in dB relative to `amp`'s sine.
fn level_db(x: &[f32], amp: f32) -> f32 {
    let tail = &x[x.len() / 2..];
    20.0 * (rms_of(tail) / (amp / 2f32.sqrt())).log10()
}

fn eq_params() -> Arc<ModuleParams> {
    Arc::new(ModuleParams::new(ModuleKind::Equalizer))
}

#[test]
fn equalizer_bands_shape_the_spectrum() {
    use crate::audio::modules::equalizer::{Equalizer, band, bp};
    let p = eq_params();
    p.set(bp(3, band::FREQ), 1000.0);
    p.set(bp(3, band::GAIN), 6.0);
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(1000.0, 0.25), None);
    assert!((level_db(&l, 0.25) - 6.0).abs() < 0.3, "boost at 1 kHz: {}", level_db(&l, 0.25));
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(100.0, 0.25), None);
    assert!(level_db(&l, 0.25).abs() < 0.3, "100 Hz untouched: {}", level_db(&l, 0.25));
    // Flat by default: the input passes as it is.
    let flat = eq_params();
    let (l, _) = drive_stereo(&mut Equalizer::new(flat, 48_000.0), 4_800, sine(440.0, 0.3), None);
    assert!((0..4_800).all(|n| (l[n] - sine(440.0, 0.3)(n).0).abs() < 1e-5));
}

#[test]
fn equalizer_dynamics_act_above_the_threshold() {
    use crate::audio::modules::equalizer::{Equalizer, band, bp};
    let p = eq_params();
    p.set(bp(3, band::FREQ), 1000.0);
    p.set(bp(3, band::RANGE), -12.0);
    p.set(bp(3, band::THRESHOLD), -30.0);
    p.set(bp(3, band::ATTACK), 0.001);
    // Loud: the band cuts by its full range.
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(1000.0, 0.5), None);
    assert!((level_db(&l, 0.5) + 12.0).abs() < 1.0, "compressed: {}", level_db(&l, 0.5));
    // Quiet: below the threshold nothing happens.
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(1000.0, 0.005), None);
    assert!(level_db(&l, 0.005).abs() < 0.3, "untouched: {}", level_db(&l, 0.005));
    // Positive range expands.
    p.set(bp(3, band::RANGE), 6.0);
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(1000.0, 0.25), None);
    assert!((level_db(&l, 0.25) - 6.0).abs() < 1.0, "expanded: {}", level_db(&l, 0.25));
}

#[test]
fn equalizer_external_side_chain_ducks() {
    use crate::audio::modules::equalizer::{Equalizer, band, bp, data};
    let p = eq_params();
    p.set(bp(3, band::FREQ), 1000.0);
    p.set(bp(3, band::RANGE), -9.0);
    p.set(bp(3, band::THRESHOLD), -20.0);
    p.data[data::SC_EXT + 3].set(1.0);
    p.data[data::SC_WIDE + 3].set(1.0);
    let quiet = sine(1000.0, 0.01);
    // A loud key on SC (another frequency: WIDE hears it) ducks the band.
    let key = |n: usize| sine(100.0, 0.5)(n);
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, &quiet, Some(&key));
    assert!((level_db(&l, 0.01) + 9.0).abs() < 1.0, "ducked: {}", level_db(&l, 0.01));
    // In SPLIT the band only hears its own frequencies: 100 Hz, 20 dB down
    // there, stays under the threshold.
    p.data[data::SC_WIDE + 3].set(0.0);
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, &quiet, Some(&key));
    assert!(level_db(&l, 0.01) > -4.0, "split: {}", level_db(&l, 0.01));
    // Without a key, nothing.
    let silent = |_: usize| (0.0, 0.0);
    let (l, _) = drive_stereo(&mut Equalizer::new(p, 48_000.0), 24_000, &quiet, Some(&silent));
    assert!(level_db(&l, 0.01).abs() < 0.3);
}

#[test]
fn equalizer_filters_mid_side_mix_and_solo() {
    use crate::audio::modules::equalizer::{self, Equalizer, band, bp, data};
    // HP 24 dB/octave at 1 kHz: two octaves below, −48 dB.
    let p = eq_params();
    p.data[data::HP_ON].set(1.0);
    p.data[data::HP_SLOPE].set(3.0);
    p.set(equalizer::HP_FREQ, 1000.0);
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(250.0, 0.5), None);
    assert!((level_db(&l, 0.5) + 48.0).abs() < 1.5, "HP: {}", level_db(&l, 0.5));
    // MIX 0: the dry signal.
    p.set(equalizer::MIX, 0.0);
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(250.0, 0.5), None);
    assert!(level_db(&l, 0.5).abs() < 0.1, "dry: {}", level_db(&l, 0.5));

    // A side-only band leaves a mono signal alone and works on the sides.
    let p = eq_params();
    p.set(bp(2, band::FREQ), 1000.0);
    p.set(bp(2, band::GAIN), 12.0);
    p.data[data::BAND_MODE + 2].set(equalizer::SIDE as f32);
    let (l, r) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(1000.0, 0.2), None);
    assert!(level_db(&l, 0.2).abs() < 0.1 && level_db(&r, 0.2).abs() < 0.1, "mono untouched");
    let wide = |n: usize| {
        let v = sine(1000.0, 0.2)(n).0;
        (v, -v)
    };
    let (l, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, wide, None);
    assert!((level_db(&l, 0.2) - 12.0).abs() < 0.5, "sides boosted: {}", level_db(&l, 0.2));

    // Solo plays the band's side chain: around its frequency only.
    let p = eq_params();
    p.set(bp(3, band::FREQ), 1000.0);
    p.data[data::SOLO].set(3.0);
    let (near, _) = drive_stereo(&mut Equalizer::new(p.clone(), 48_000.0), 24_000, sine(1000.0, 0.3), None);
    let (far, _) = drive_stereo(&mut Equalizer::new(p, 48_000.0), 24_000, sine(100.0, 0.3), None);
    assert!(level_db(&near, 0.3).abs() < 0.5 && level_db(&far, 0.3) < -15.0, "solo: {} / {}", level_db(&near, 0.3), level_db(&far, 0.3));
}

fn magma_params() -> Arc<ModuleParams> {
    Arc::new(ModuleParams::new(ModuleKind::Magma))
}

fn magma(p: &Arc<ModuleParams>) -> crate::audio::modules::magma::Magma {
    crate::audio::modules::magma::Magma::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0)
}

/// Filter 1 alone, clean, of `shape` at `freq`.
fn magma_clean(shape: usize, slope: usize, freq: f32, peak: f32) -> Arc<ModuleParams> {
    use crate::audio::modules::magma::{data, filt, fp};
    let p = magma_params();
    p.data[data::SHAPE].set(shape as f32);
    p.data[data::SLOPE].set(slope as f32);
    p.data[data::STYLE].set(10.0);
    p.set(fp(0, filt::FREQ), freq);
    p.set(fp(0, filt::PEAK), peak);
    p
}

#[test]
fn magma_filters_match_their_curves() {
    use crate::audio::modules::magma::{self, BELL, HIGH_PASS, LOW_PASS, NOTCH};
    for (shape, slope, freq, peak, probe) in [
        (LOW_PASS, 2, 1000.0, 0.0, 250.0),
        (LOW_PASS, 2, 1000.0, 0.0, 4000.0),
        (HIGH_PASS, 1, 1000.0, 0.0, 500.0),
        (BELL, 1, 1000.0, 0.5, 1000.0),
        (NOTCH, 1, 1000.0, 0.0, 1300.0),
        (LOW_PASS, 3, 2000.0, 0.6, 1000.0),
    ] {
        let p = magma_clean(shape, slope, freq, peak);
        let (l, _) = drive_stereo(&mut magma(&p), 24_000, sine(probe, 0.1), None);
        let measured = level_db(&l, 0.1);
        let vals = magma::base_values(&p);
        let expected = magma::db(magma::total_response(&p, &vals, 0, probe, 48_000.0));
        assert!((measured - expected).abs() < 0.6, "shape {shape} slope {slope} at {probe} Hz: {measured:.2} vs {expected:.2}");
    }
}

#[test]
fn magma_self_oscillates_and_auto_mutes() {
    use crate::audio::modules::magma::{LOW_PASS, data, filt, fp};
    let p = magma_clean(LOW_PASS, 1, 800.0, 1.0);
    p.data[data::STYLE].set(0.0); // Classic
    p.data[data::AUTO_MUTE].set(0.0);
    let mut fx = magma(&p);
    let burst = |n: usize| if n < 480 { sine(800.0, 0.5)(n) } else { (0.0, 0.0) };
    let (l, _) = drive_stereo(&mut fx, 48_000, burst, None);
    let tail = &l[24_000..];
    assert!(rms_of(tail) > 0.05, "keeps singing on its own: {}", rms_of(tail));
    let crossings = tail.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32 * 2.0;
    assert!((crossings - 800.0).abs() < 120.0, "near the cutoff: {crossings} Hz");
    // Auto-mute stops it once the input has gone silent.
    p.data[data::AUTO_MUTE].set(1.0);
    let (l, _) = drive_stereo(&mut magma(&p), 48_000, burst, None);
    assert!(peak_of(&l[24_000..]) < 1e-3, "muted: {}", peak_of(&l[24_000..]));
    // Clean never oscillates.
    p.data[data::STYLE].set(10.0);
    p.data[data::AUTO_MUTE].set(0.0);
    let (l, _) = drive_stereo(&mut magma(&p), 48_000, burst, None);
    assert!(peak_of(&l[24_000..]) < 1e-3);
    let _ = fp(0, filt::PEAK);
}

#[test]
fn magma_routing_and_mid_side() {
    use crate::audio::modules::magma::{self, HIGH_PASS, LOW_PASS, MID_SIDE, data, filt, fp};
    // LP 300 Hz and HP 3 kHz in parallel pass both ends, not the middle.
    let p = magma_clean(LOW_PASS, 2, 300.0, 0.0);
    p.data[data::FILTER_ON + 1].set(1.0);
    p.data[data::SHAPE + 1].set(HIGH_PASS as f32);
    p.data[data::SLOPE + 1].set(2.0);
    p.data[data::STYLE + 1].set(10.0);
    p.set(fp(1, filt::FREQ), 3000.0);
    p.set(fp(1, filt::PEAK), 0.0);
    p.data[data::ROUTING].set(1.0);
    let level = |hz: f32| level_db(&drive_stereo(&mut magma(&p), 24_000, sine(hz, 0.1), None).0, 0.1);
    assert!(level(80.0).abs() < 1.0 && level(10_000.0).abs() < 1.0, "ends pass");
    assert!(level(950.0) < -12.0, "middle cut: {}", level(950.0));
    // In series they cut everything.
    p.data[data::ROUTING].set(0.0);
    assert!(level(80.0) < -30.0 && level(10_000.0) < -30.0);

    // M/S: filter 1 (a low pass) on the mid only leaves the sides alone.
    let p = magma_clean(LOW_PASS, 2, 200.0, 0.0);
    p.data[data::ROUTING_MODE].set(MID_SIDE as f32);
    let sides = |n: usize| {
        let v = sine(4000.0, 0.1)(n).0;
        (v, -v)
    };
    let (l, _) = drive_stereo(&mut magma(&p), 24_000, sides, None);
    assert!(level_db(&l, 0.1).abs() < 0.3, "sides untouched");
    let (l, _) = drive_stereo(&mut magma(&p), 24_000, sine(4000.0, 0.1), None);
    assert!(level_db(&l, 0.1) < -40.0, "mid filtered");
    let _ = magma::FILTERS;
}

#[test]
fn magma_high_quality_keeps_the_sound() {
    use crate::audio::modules::magma::{LOW_PASS, data};
    let p = magma_clean(LOW_PASS, 1, 2000.0, 0.3);
    let plain = level_db(&drive_stereo(&mut magma(&p), 24_000, sine(1000.0, 0.1), None).0, 0.1);
    p.data[data::HQ].set(1.0);
    let hq = level_db(&drive_stereo(&mut magma(&p), 24_000, sine(1000.0, 0.1), None).0, 0.1);
    assert!((plain - hq).abs() < 0.3, "{plain} vs {hq}");
    let high = level_db(&drive_stereo(&mut magma(&magma_clean(LOW_PASS, 0, 20_000.0, 0.0)), 24_000, sine(15_000.0, 0.1), None).0, 0.1);
    p.set(crate::audio::modules::magma::fp(0, 1), 20_000.0);
    assert!(high > -4.0, "the oversampler passes the top: {high}");
}

#[test]
fn magma_modulation_moves_the_filter() {
    use crate::audio::modules::magma::{self, data, filt, fp, modulation, viz, xlfo, xp};
    let p = magma_params();
    magma::add_slot(&p, modulation::SRC_XLFO, fp(0, filt::FREQ)).expect("a slot");
    p.set(xp(0, xlfo::RATE), 5.0);
    let mut fx = magma(&p);
    let mut seen = Vec::new();
    for _ in 0..20 {
        drive_stereo(&mut fx, 960, sine(440.0, 0.1), None);
        seen.push(p.viz[viz::EFFECTIVE + fp(0, filt::FREQ)].get());
    }
    let (lo, hi) = seen.iter().fold((1.0f32, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(lo >= 0.0 && hi - lo > 0.2, "swept: {lo}…{hi}");
    // A slot on another slot's level: the XLFO's depth follows XY 1.
    magma::add_slot(&p, modulation::SRC_XY, magma::SLOT0).expect("a second slot");
    p.set(magma::XY0, -1.0);
    p.set(magma::SLOT0 + 1, 1.0);
    drive_stereo(&mut fx, 960, sine(440.0, 0.1), None);
    assert!(p.viz[viz::EFFECTIVE + magma::SLOT0].get() < 0.5, "slot level pulled down");
    let _ = data::SLOT_SOURCE;
}

#[test]
fn magma_envelope_follows_midi_notes() {
    use crate::audio::modules::magma::{self, data, modulation, viz};
    let p = magma_params();
    p.data[data::EG].set(2.0); // MIDI trigger
    p.data[data::SOURCE_SHOWN].set(2.0); // shown in the panel: worked out
    let mut fx = magma(&p);
    let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
    let mut inputs = Box::new(Inputs { bufs: [[[0.1; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: true });
    inputs.connected[0] = true;
    let mut out = vec![[[0.0f32; 64]; 2]; 1];
    let mut run = |inputs: &mut Inputs, blocks: usize, first: Option<[u8; 3]>| {
        for b in 0..blocks {
            inputs.midi.clear();
            if b == 0 && let Some(msg) = first {
                inputs.midi.push(0, msg);
            }
            fx.process(&ctx, inputs, &mut out, 64);
        }
    };
    run(&mut inputs, 10, None);
    let eg = viz::SOURCES_AT + modulation::SRC_EG - 1;
    assert_eq!(p.viz[eg].get(), 0.0);
    run(&mut inputs, 400, Some([0x90, 60, 100]));
    assert!((p.viz[eg].get() - 0.5).abs() < 0.05, "sustain: {}", p.viz[eg].get());
    run(&mut inputs, 750, Some([0x80, 60, 0]));
    assert!(p.viz[eg].get() < 0.01, "released: {}", p.viz[eg].get());
    let _ = magma::FILTERS;
}

#[test]
fn magma_envelope_holds_on_a_steady_tone() {
    use crate::audio::modules::magma::{data, modulation, viz};
    let p = magma_params();
    p.data[data::SOURCE_SHOWN].set(2.0);
    let mut fx = magma(&p);
    // A low saw crosses zero every period: the gate must stay open.
    let low = |n: usize| {
        let v = 0.5 * ((n as f32 * 55.0 / 48_000.0).fract() * 2.0 - 1.0);
        (v, v)
    };
    drive_stereo(&mut fx, 96_000, low, None);
    let eg = p.viz[viz::SOURCES_AT + modulation::SRC_EG - 1].get();
    assert!((eg - 0.5).abs() < 0.02, "sustaining: {eg}");
}

#[test]
fn magma_live_values_are_only_for_modulated_knobs() {
    use crate::audio::modules::magma::{self, filt, fp, modulation, viz};
    let p = magma_params();
    let mut fx = magma(&p);
    let freq = viz::EFFECTIVE + fp(0, filt::FREQ);
    assert_eq!(p.viz[freq].get(), -1.0, "nothing modulated from the start");
    // Silent long enough to rest: still nothing claims to be modulated.
    drive_stereo(&mut fx, 48_000, |_| (0.0, 0.0), None);
    assert_eq!(p.viz[freq].get(), -1.0);
    // A slot while resting: its target's live value is published.
    magma::add_slot(&p, modulation::SRC_XY, fp(0, filt::FREQ));
    drive_stereo(&mut fx, 640, |_| (0.0, 0.0), None);
    assert!(p.viz[freq].get() >= 0.0, "live while resting");
}

/// Runs a Pops 66 at 120 BPM (transport playing unless `stopped`), with
/// `midi` fed at given (block, message); returns the left channel and the
/// MIDI it sent, with absolute frames.
fn run_pops(
    p: &Arc<ModuleParams>,
    blocks: usize,
    playing: bool,
    midi: &[(usize, [u8; 3])],
) -> (Vec<f32>, Vec<(usize, [u8; 3])>) {
    use crate::audio::modules::pops::Pops;
    let mut proc = Pops::new(p.clone(), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing, restart: false };
    let mut inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: true });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let (mut left, mut sent) = (Vec::new(), Vec::new());
    for b in 0..blocks {
        inputs.midi.clear();
        for (_, msg) in midi.iter().filter(|(at, _)| *at == b) {
            inputs.midi.push(0, *msg);
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        left.extend_from_slice(&outputs[0][0]);
        if let Some(out) = proc.midi_out() {
            sent.extend(out.events().iter().map(|e| (b * 64 + e.frame as usize, e.data)));
        }
        if playing {
            ctx.beat += ctx.beats_per_sample * 64.0;
        }
    }
    (left, sent)
}

fn pops_params() -> Arc<ModuleParams> {
    Arc::new(ModuleParams::new(ModuleKind::Pops))
}

#[test]
fn pops_rhythms_are_well_formed_and_combine() {
    use crate::audio::modules::pops::rhythms::{RHYTHMS, combine};
    use crate::audio::modules::pops::voices::{BASE, HIHAT};
    for r in &RHYTHMS {
        assert!(r.len >= 6 && r.len <= 32 && matches!(r.per_beat, 3 | 4), "{}", r.name);
        assert!(r.rows.iter().any(|&b| b != 0), "{} plays something", r.name);
        assert_eq!(r.len % r.per_beat, 0, "{}: whole beats", r.name);
    }
    // ROCK 1 + ROCK 2: ROCK 1's grid, ROCK 2's sixteenth hats added.
    let both = combine(&[6, 16]).unwrap();
    assert_eq!(both.rows[HIHAT].count_ones(), 32);
    assert_eq!(both.rows[BASE], RHYTHMS[6].rows[BASE] | RHYTHMS[16].rows[BASE]);
    assert!(combine(&[]).is_none());
}

#[test]
fn pops_follows_the_board_tempo() {
    use crate::audio::modules::pops::{data, voices::BASE, INSTRUMENTS};
    let p = pops_params(); // ROCK 1, SYNC on
    let (left, sent) = run_pops(&p, 1500, true, &[]);
    // At 120 BPM a sixteenth is 6000 frames: bass drum on steps 0, 8, 16, 24.
    let kicks: Vec<usize> = sent.iter().filter(|(_, m)| m[0] == 0x99 && m[1] == INSTRUMENTS[BASE].note).map(|(f, _)| *f).collect();
    assert_eq!(kicks, [0, 48_000]);
    assert!(rms_of(&left) > 0.01, "makes sound");
    // Transport stopped: nothing.
    let (left, sent) = run_pops(&p, 400, false, &[]);
    assert!(sent.is_empty() && peak_of(&left) == 0.0);
    // Its own clock: SYNC off, PLAY, 60 BPM → a sixteenth every 12000 frames.
    p.data[data::SYNC].set(0.0);
    p.data[data::RUNNING].set(1.0);
    p.set(crate::audio::modules::pops::TEMPO, 60.0);
    let (_, sent) = run_pops(&p, 1600, false, &[]);
    let kicks: Vec<usize> = sent.iter().filter(|(_, m)| m[0] == 0x99 && m[1] == 36).map(|(f, _)| *f).collect();
    assert_eq!(kicks, [0, 96_000]);
}

#[test]
fn pops_plays_midi_notes_user_patterns_and_mutes() {
    use crate::audio::modules::pops::{self, data, voices::{BASE, SNARE1}, viz};
    let p = pops_params();
    p.data[data::POWER].set(0.0);
    // A MIDI note plays its instrument, rhythm off.
    let (left, sent) = run_pops(&p, 200, true, &[(10, [0x99, 38, 120])]);
    assert!(sent.is_empty(), "notes played in are not sent out");
    assert!(peak_of(&left[..640]) == 0.0 && rms_of(&left[640..640 + 9600]) > 0.01, "snare from MIDI");
    assert_eq!(p.viz[viz::HITS + SNARE1].get(), 1.0);
    // A user pattern: one snare on step 4 of 8.
    p.data[data::POWER].set(1.0);
    p.data[data::SOURCE].set(1.0);
    p.data[data::PATTERN].set(7.0);
    pops::set_shape(&p, 7, 8, false);
    pops::set_hit(&p, 7, SNARE1, 4, true);
    let (_, sent) = run_pops(&p, 1600, true, &[]);
    let snares: Vec<usize> = sent.iter().filter(|(_, m)| m[0] == 0x99 && m[1] == 38).map(|(f, _)| *f).collect();
    assert_eq!(snares, [24_000, 72_000]);
    // Mute it.
    p.data[data::MUTE + SNARE1].set(1.0);
    let (_, sent) = run_pops(&p, 1600, true, &[]);
    assert!(sent.iter().all(|(_, m)| m[1] != 38));
    let _ = BASE;
}

#[test]
fn pops_every_instrument_sounds() {
    use crate::audio::modules::pops::{INSTRUMENTS, data};
    for info in &INSTRUMENTS {
        let p = pops_params();
        p.data[data::POWER].set(0.0);
        let note = info.note;
        let (left, _) = run_pops(&p, 400, true, &[(0, [0x99, note, 127]), (50, [0x89, note, 0])]);
        assert!(left.iter().all(|s| s.is_finite()));
        assert!(rms_of(&left[..12_000]) > 0.002, "{} is silent: {}", info.name, rms_of(&left[..12_000]));
        assert!(peak_of(&left) <= 1.0);
    }
}

#[test]
fn pops_waveforms_can_be_reshaped() {
    use crate::audio::modules::pops::voices::{BASE, COUNT, HIHAT, shape};
    use crate::audio::modules::pops::{self, ip, knob};
    // WAVE: a sine, a triangle at the middle, a square at the end.
    assert!((shape(0.25, 0.0) - 1.0).abs() < 1e-3);
    assert!((shape(0.125, 0.5) - 0.5).abs() < 1e-6);
    assert!((shape(0.125, 1.0) - 1.0).abs() < 1e-6 && (shape(0.625, 1.0) + 1.0).abs() < 1e-6);
    let render = |p: &ModuleParams, inst: usize, n: usize| {
        let mut out = vec![0.0; n];
        pops::render(p, inst, 48_000.0, &mut out);
        out
    };
    let upward = |s: &[f32]| s.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    let p = pops_params();
    let sine = render(&p, BASE, 19_200);
    // A square: more energy under the same peak.
    p.set(ip(BASE, knob::WAVE), 1.0);
    let square = render(&p, BASE, 19_200);
    assert!(rms_of(&square) > rms_of(&sine) * 1.2 && peak_of(&square) <= 1.2, "{} {} {} {}", rms_of(&square), rms_of(&sine), peak_of(&square), peak_of(&sine));
    p.set(ip(BASE, knob::WAVE), 0.0);
    // BEND up: the hit starts two octaves higher and glides back to TUNE.
    p.set(ip(BASE, knob::BEND), 1.0);
    let bent = render(&p, BASE, 19_200);
    assert!(upward(&bent[..960]) > upward(&sine[..960]) + 1, "{} vs {}", upward(&bent[..960]), upward(&sine[..960]));
    assert!(upward(&bent[9600..14_400]).abs_diff(upward(&sine[9600..14_400])) <= 1);
    p.set(ip(BASE, knob::BEND), 0.0);
    // DRIVE: flattened, louder, never over the top.
    p.set(ip(BASE, knob::DRIVE), 1.0);
    let driven = render(&p, BASE, 19_200);
    assert!(rms_of(&driven) > rms_of(&sine) * 1.3 && peak_of(&driven) <= 1.01);
    // The hihat's noise turns metallic.
    let noise = render(&p, HIHAT, 2400);
    p.set(ip(HIHAT, knob::WAVE), 1.0);
    let metal = render(&p, HIHAT, 2400);
    assert!(rms_of(&metal) > 0.01 && metal.iter().zip(&noise).any(|(a, b)| (a - b).abs() > 0.01));
    // Every instrument, at the extremes, still sounds and stays in bounds.
    for inst in 0..COUNT {
        for (bend, drive) in [(1.0, 0.0), (-1.0, 1.0)] {
            let p = pops_params();
            p.set(ip(inst, knob::WAVE), 1.0);
            p.set(ip(inst, knob::BEND), bend);
            p.set(ip(inst, knob::DRIVE), drive);
            let hit = render(&p, inst, (pops::hit_seconds(&p, inst) * 48_000.0) as usize);
            assert!(hit.iter().all(|y| y.is_finite()), "{inst}");
            assert!(rms_of(&hit) > 0.001 && peak_of(&hit) < 4.0, "{inst}: {} {}", rms_of(&hit), peak_of(&hit));
        }
    }
}

fn tape_params() -> Arc<ModuleParams> {
    let p = Arc::new(ModuleParams::new(ModuleKind::Tape));
    p.set(crate::audio::modules::tape::WOW, 0.0);
    p.set(crate::audio::modules::tape::FLUTTER, 0.0);
    p
}

/// The amplitude of the `hz` component of `buf` (a whole number of cycles
/// at 48 kHz).
fn tone_amp(buf: &[f32], hz: f32) -> f32 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &x) in buf.iter().enumerate() {
        let a = std::f64::consts::TAU * hz as f64 * n as f64 / 48_000.0;
        re += x as f64 * a.cos();
        im += x as f64 * a.sin();
    }
    (2.0 * (re * re + im * im).sqrt() / buf.len() as f64) as f32
}

#[test]
fn tape_is_clean_when_quiet_and_saturates_when_driven() {
    use crate::audio::modules::tape::{INPUT, Tape, viz};
    let p = tape_params();
    let mut t = Tape::new(p.clone(), 48_000.0);
    let (l, _) = drive_stereo(&mut t, 24_000, sine(1000.0, 0.05), None);
    let tail = &l[14_400..24_000];
    let fundamental = tone_amp(tail, 1000.0);
    assert!((fundamental / 0.05 - 1.0).abs() < 0.12, "unity when quiet: {fundamental}");
    assert!(tone_amp(tail, 3000.0) < fundamental * 0.003, "clean when quiet");
    // Driven 18 dB (AUTO keeps the level): squeezed, with harmonics.
    p.set(INPUT, 18.0);
    let mut t = Tape::new(p.clone(), 48_000.0);
    let (l, _) = drive_stereo(&mut t, 24_000, sine(1000.0, 0.25), None);
    let tail = &l[14_400..24_000];
    let fundamental = tone_amp(tail, 1000.0);
    assert!(fundamental < 0.25 * 0.7, "squeezed: {fundamental}");
    assert!(tone_amp(tail, 3000.0) > fundamental * 0.03, "third harmonic");
    assert!(p.viz[viz::SAT].get() < -1.0, "the panel sees it: {}", p.viz[viz::SAT].get());
}

#[test]
fn tape_lines_up_dry_and_wet() {
    use crate::audio::modules::tape::{MIX, Tape};
    let click = |n: usize| if n == 4800 { (0.5, 0.5) } else { (0.0, 0.0) };
    let peak_at = |mix: f32| {
        let p = tape_params();
        p.set(MIX, mix);
        let mut t = Tape::new(p, 48_000.0);
        let (l, _) = drive_stereo(&mut t, 9600, click, None);
        l.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0
    };
    let (dry, wet) = (peak_at(0.0), peak_at(1.0));
    assert!(dry.abs_diff(wet) <= 3, "dry {dry}, wet {wet}");
}

#[test]
fn tape_stops_and_spins_back_up() {
    use crate::audio::modules::tape::{Tape, data, viz};
    let p = tape_params();
    let mut t = Tape::new(p.clone(), 48_000.0);
    let tone = sine(220.0, 0.2);
    let (running, _) = drive_stereo(&mut t, 24_000, &tone, None);
    let level = rms_of(&running[12_000..]);
    p.data[data::RUNNING].set(0.0);
    let (stopping, _) = drive_stereo(&mut t, 72_000, &tone, None);
    assert!(rms_of(&stopping[57_600..]) < 1e-4, "stopped");
    assert_eq!(p.viz[viz::SPEED].get(), 0.0);
    // Slowing down: the pitch falls (fewer crossings late than early).
    let ups = |s: &[f32]| s.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    assert!(ups(&stopping[24_000..33_600]) < ups(&stopping[..9600]));
    p.data[data::RUNNING].set(1.0);
    let (starting, _) = drive_stereo(&mut t, 48_000, &tone, None);
    assert_eq!(p.viz[viz::SPEED].get(), 1.0);
    let back = rms_of(&starting[33_600..]);
    assert!((back / level - 1.0).abs() < 0.1, "back to {back} from {level}");
    assert!(starting.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
}

#[test]
fn tape_wow_and_flutter_bend_the_pitch() {
    use crate::audio::modules::tape::{FLUTTER, Tape, WOW, data};
    let spread = |wow: f32| {
        let p = tape_params();
        p.data[data::MACHINE].set(2.0); // cassette: the most wobble
        p.set(WOW, wow);
        p.set(FLUTTER, wow);
        let mut t = Tape::new(p, 48_000.0);
        let (l, _) = drive_stereo(&mut t, 96_000, sine(1000.0, 0.1), None);
        // Periods between upward crossings (interpolated), after settling.
        let mut at = Vec::new();
        for (n, w) in l.windows(2).enumerate().skip(4800) {
            if w[0] < 0.0 && w[1] >= 0.0 {
                at.push(n as f32 + w[0] / (w[0] - w[1]));
            }
        }
        let periods: Vec<f32> = at.windows(2).map(|w| w[1] - w[0]).collect();
        let lo = periods.iter().copied().fold(f32::MAX, f32::min);
        let hi = periods.iter().copied().fold(0.0, f32::max);
        (hi - lo) / 48.0
    };
    assert!(spread(0.0) < 0.002, "steady: {}", spread(0.0));
    assert!(spread(1.0) > 0.01, "wobbling: {}", spread(1.0));
}

#[test]
fn tape_hisses_only_when_asked_and_then_rests() {
    use crate::audio::modules::tape::{HISS, Tape};
    let p = tape_params();
    let silent = |_: usize| (0.0, 0.0);
    let mut t = Tape::new(p.clone(), 48_000.0);
    let (l, _) = drive_stereo(&mut t, 48_000, silent, None);
    assert!(l.iter().all(|s| *s == 0.0), "no hiss by default");
    p.set(HISS, 1.0);
    let (l, _) = drive_stereo(&mut t, 48_000, silent, None);
    let db = 20.0 * rms_of(&l[24_000..]).log10();
    assert!((-75.0..-40.0).contains(&db), "hiss at {db} dB");
}

#[test]
fn every_tape_setting_stays_finite_and_bounded() {
    use crate::audio::modules::tape::{AGE, BIAS, HISS, INPUT, Tape, data};
    for machine in 0..3 {
        for speed in 0..3 {
            for tape in 0..3 {
                for eq in 0..2 {
                    let p = tape_params();
                    for (i, v) in [(data::MACHINE, machine), (data::SPEED, speed), (data::TAPE, tape), (data::EQ, eq)] {
                        p.data[i].set(v as f32);
                    }
                    p.set(INPUT, 18.0);
                    p.set(BIAS, if eq == 0 { -1.0 } else { 1.0 });
                    p.set(AGE, 1.0);
                    p.set(HISS, 1.0);
                    let mut t = Tape::new(p, 48_000.0);
                    let (l, r) = drive_stereo(&mut t, 9600, sine(80.0, 0.9), None);
                    assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() <= 1.0));
                    assert!(rms_of(&l[4800..]) > 0.01, "{machine} {speed} {tape} {eq}");
                }
            }
        }
    }
}

#[test]
fn effects_bypass_without_clicks() {
    use crate::audio::modules::filter;
    let (mut g, a, mut e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    osc_params(&g, x).set(osc::FREQ, 4000.0);
    let f = g.add_module(ModuleKind::Filter, pos2(0.0, 0.0), &a);
    module_params(&g, f).set(filter::CUTOFF, 80.0);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(f, 0), &a);
    g.connect(port(f, 0), port(m, 0), &a);
    render_buf(&mut e, 9600);
    let filtered = rms_of(&render_buf(&mut e, 9600));
    // Bypassed: the oscillator straight through, faded in.
    g.node(f).unwrap().set_bypass(true);
    let fade = render_buf(&mut e, 9600);
    let dry = rms_of(&render_buf(&mut e, 9600));
    assert!(dry > filtered * 10.0, "bypassed {dry}, filtered {filtered}");
    // No jumps bigger than the sine itself makes (4 kHz at this level).
    let steepest = fade.chunks(2).collect::<Vec<_>>().windows(2).map(|w| (w[1][0] - w[0][0]).abs()).fold(0.0, f32::max);
    let sine_step = dry * std::f32::consts::SQRT_2 * std::f32::consts::TAU * 4000.0 / 48_000.0;
    assert!(steepest <= sine_step * 1.1, "click: {steepest} > {sine_step}");
    // And back.
    g.node(f).unwrap().set_bypass(false);
    render_buf(&mut e, 9600);
    assert!((rms_of(&render_buf(&mut e, 9600)) - filtered).abs() < filtered * 0.2 + 1e-4);
    // Only effects bypass.
    g.node(x).unwrap().set_bypass(true);
    assert!(!g.node(x).unwrap().bypassed());
    assert!(g.node(f).unwrap().is_effect() && !g.node(m).unwrap().is_effect());
}

/// Runs Nullpunkt for `blocks` at 120 BPM (playing or not); returns the
/// left channel.
fn run_nullpunkt(p: &Arc<ModuleParams>, blocks: usize, playing: bool) -> Vec<f32> {
    use crate::audio::modules::nullpunkt::Nullpunkt;
    let mut proc = Nullpunkt::new(p.clone(), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing, restart: false };
    let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let mut left = Vec::new();
    for _ in 0..blocks {
        proc.process(&ctx, &inputs, &mut outputs, 64);
        left.extend_from_slice(&outputs[0][0]);
        if playing {
            ctx.beat += ctx.beats_per_sample * 64.0;
        }
    }
    left
}

/// Nullpunkt with nothing on: no voices, no field, no glitches.
fn quiet_nullpunkt() -> Arc<ModuleParams> {
    use crate::audio::modules::nullpunkt::{CUT, JITTER, MUTATE, SKIP, STUTTER, data, knob, voice, vp};
    let p = Arc::new(ModuleParams::new(ModuleKind::Nullpunkt));
    for v in 0..voice::COUNT {
        p.set(vp(v, knob::LEVEL), 0.0);
        p.data[data::ROT + v].set(0.0);
    }
    for i in [SKIP, CUT, MUTATE, STUTTER, JITTER] {
        p.set(i, 0.0);
    }
    p
}

#[test]
fn nullpunkt_patterns_are_euclidean() {
    use crate::audio::modules::nullpunkt::euclid;
    let hits = |k: usize, n: usize| (0..n).filter(|&s| euclid(k, n, s)).collect::<Vec<_>>();
    assert_eq!(hits(3, 8), [0, 3, 6]);
    assert_eq!(hits(4, 16), [0, 4, 8, 12]);
    for n in 1..=32 {
        for k in 0..=n {
            assert_eq!(hits(k, n).len(), k, "{k} of {n}");
        }
    }
    assert_eq!(hits(40, 8).len(), 8, "no more hits than steps");
}

#[test]
fn nullpunkt_voices_play_on_the_board_clock() {
    use crate::audio::modules::nullpunkt::{knob, regenerate, viz, voice, vp};
    for v in 0..voice::COUNT {
        let p = quiet_nullpunkt();
        p.set(vp(v, knob::LEVEL), 0.8);
        p.set(vp(v, knob::HITS), 4.0);
        p.set(vp(v, knob::LEN), 16.0);
        regenerate(&p, v);
        // 120 BPM, sixteenths: a step every 6000 frames, a hit every fourth.
        let left = run_nullpunkt(&p, 1500, true);
        assert_eq!(p.viz[viz::HITS + v].get(), 4.0, "{}: steps 0, 4, 8, 12", voice::NAMES[v]);
        assert!(left.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(peak_of(&left[..3000]) > 0.01, "{} sounds on step 0", voice::NAMES[v]);
        assert!(peak_of(&left[12_100..23_900]) < peak_of(&left[..3000]) * 0.5, "{} quieter between hits", voice::NAMES[v]);
        // Stopped: nothing.
        let p2 = quiet_nullpunkt();
        p2.set(vp(v, knob::LEVEL), 0.8);
        assert_eq!(peak_of(&run_nullpunkt(&p2, 400, false)), 0.0);
    }
}

#[test]
fn nullpunkt_cuts_steps_and_survives_every_glitch() {
    use crate::audio::modules::nullpunkt::{CRUSH, CUT, MUTATE, RATE, REVERSE, SKIP, STATIC, WARP, knob, regenerate, voice, vp};
    // DATA on every step, ringing through it; CUT at full: about half the
    // steps fall silent.
    let p = quiet_nullpunkt();
    let data = |k| vp(voice::DATA, k);
    p.set(data(knob::LEVEL), 0.8);
    p.set(data(knob::HITS), 16.0);
    p.set(data(knob::LEN), 16.0);
    p.set(data(knob::DECAY), 0.4);
    regenerate(&p, voice::DATA);
    p.set(CUT, 1.0);
    let left = run_nullpunkt(&p, 3000, true);
    let silent = left.chunks(6000).skip(1).filter(|step| rms_of(&step[200..5800]) < 1e-3).count();
    assert!((6..26).contains(&silent), "{silent} of 31 steps cut");
    // Everything at once stays finite and bounded.
    let p = Arc::new(ModuleParams::new(ModuleKind::Nullpunkt));
    use crate::audio::modules::nullpunkt::{GRAIN, JITTER, SCATTER, STUTTER, STUTTER_LEN};
    for (i, v) in [
        (SKIP, 1.0),
        (CUT, 1.0),
        (REVERSE, 1.0),
        (WARP, 1.0),
        (STATIC, 1.0),
        (CRUSH, 1.0),
        (RATE, 1.0),
        (MUTATE, 1.0),
        (STUTTER, 0.6),
        (STUTTER_LEN, 8.0),
        (GRAIN, 1.0),
        (SCATTER, 1.0),
        (JITTER, 1.0),
    ] {
        p.set(i, v);
    }
    let left = run_nullpunkt(&p, 3000, true);
    assert!(left.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(rms_of(&left) > 0.005);
}

#[test]
fn nullpunkt_steps_can_be_edited_rotated_and_muted() {
    use crate::audio::modules::nullpunkt::{data, knob, rotate, set_step, set_steps, steps, viz, voice, vp};
    let p = quiet_nullpunkt();
    let sub = voice::SUB;
    p.set(vp(sub, knob::LEVEL), 0.8);
    p.set(vp(sub, knob::LEN), 8.0);
    // One hit, on step 3 of 8: steps are 6000 frames at 120 BPM.
    set_steps(&p, sub, 0);
    set_step(&p, sub, 3, true);
    let left = run_nullpunkt(&p, 375, true);
    assert_eq!(p.viz[viz::HITS + sub].get(), 1.0);
    assert!(peak_of(&left[..17_900]) < 1e-6 && peak_of(&left[18_000..21_000]) > 0.05, "struck on step 3");
    // Rotating turns it a step later, within the length.
    rotate(&p, sub, 1);
    assert_eq!(steps(&p, sub), 1 << 4);
    rotate(&p, sub, 5);
    assert_eq!(steps(&p, sub), 1 << 1, "wraps round at LEN");
    // Muted: silent.
    p.data[data::MUTE + sub].set(1.0);
    assert_eq!(peak_of(&run_nullpunkt(&p, 375, true)), 0.0);
}

#[test]
fn nullpunkt_randomizes_what_is_not_muted() {
    use crate::audio::modules::nullpunkt::{data, knob, pattern, randomize, steps, voice, vp};
    let p = Arc::new(ModuleParams::new(ModuleKind::Nullpunkt));
    p.data[data::MUTE + voice::SINE].set(1.0);
    let sine = (steps(&p, voice::SINE), p.get(vp(voice::SINE, knob::TONE)));
    let before: Vec<u32> = (0..voice::COUNT).map(|v| steps(&p, v)).collect();
    randomize(&p, 1234, true);
    let after: Vec<u32> = (0..voice::COUNT).map(|v| steps(&p, v)).collect();
    assert_ne!(before, after);
    assert_eq!((steps(&p, voice::SINE), p.get(vp(voice::SINE, knob::TONE))), sine, "muted voices stay");
    for v in 0..voice::COUNT {
        let (_, len, _) = pattern(&p, v);
        assert!(steps(&p, v) & ((1u64 << len) - 1) as u32 != 0, "something to play");
    }
    // The same seed, the same scene.
    let q = Arc::new(ModuleParams::new(ModuleKind::Nullpunkt));
    q.data[data::MUTE + voice::SINE].set(1.0);
    randomize(&q, 1234, true);
    assert_eq!(after, (0..voice::COUNT).map(|v| steps(&q, v)).collect::<Vec<_>>());
}

#[test]
fn nullpunkt_scenes_save_as_presets() {
    use crate::audio::modules::nullpunkt::{FACTORY, SKIP, data, load_factory, set_step, steps, voice};
    use crate::presets::{self, ModulePreset};
    let p = Arc::new(ModuleParams::new(ModuleKind::Nullpunkt));
    set_step(&p, voice::CLICK, 1, true);
    p.data[data::MUTE + voice::DATA].set(1.0);
    p.set(SKIP, 0.42);
    let dir = TempDir::new("presets");
    let mut list = presets::load(&dir.0, ModuleKind::Nullpunkt);
    assert!(list.is_empty());
    presets::store(&mut list, ModulePreset::capture(&p, "Mine"));
    presets::save(&dir.0, ModuleKind::Nullpunkt, &list).unwrap();
    let loaded = presets::load(&dir.0, ModuleKind::Nullpunkt);
    assert_eq!(loaded, list);
    // Into a fresh node: the same scene.
    let q = Arc::new(ModuleParams::new(ModuleKind::Nullpunkt));
    loaded[0].apply(&q);
    assert_eq!(steps(&q, voice::CLICK), steps(&p, voice::CLICK));
    assert_eq!(q.get(SKIP), 0.42);
    assert!(q.data[data::MUTE + voice::DATA].get() > 0.5);
    // Saving under the same name replaces it.
    presets::store(&mut list, ModulePreset::capture(&q, "Mine"));
    assert_eq!(list.len(), 1);
    // The built-in scenes all load and play.
    for (k, (name, _)) in FACTORY.iter().enumerate() {
        load_factory(&q, k);
        let left = run_nullpunkt(&q, 1500, true);
        assert!(left.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{name}");
        assert!(rms_of(&left) > 0.001, "{name} plays");
    }
}

#[test]
fn nullpunkt_plays_triplets_and_probabilities_on_the_grid() {
    use crate::audio::modules::nullpunkt::{data, knob, set_steps, viz, voice, vp};
    let p = quiet_nullpunkt();
    let sub = voice::SUB;
    p.set(vp(sub, knob::LEVEL), 0.8);
    p.set(vp(sub, knob::LEN), 6.0);
    set_steps(&p, sub, 0b1001);
    // Eighth-note triplets at 120 BPM: a step every 8000 frames; hits on
    // steps 0 and 3.
    p.data[data::DIV].set(2.0);
    let left = run_nullpunkt(&p, 750, true);
    assert_eq!(p.viz[viz::HITS + sub].get(), 2.0);
    assert!(peak_of(&left[23_900..24_000]) < peak_of(&left[24_000..25_000]) * 0.2, "the second hit on frame 24000");
    // PROB: no hits at 0; about half at 0.5.
    set_steps(&p, sub, 0b111111);
    p.set(vp(sub, knob::PROB), 0.0);
    run_nullpunkt(&p, 3000, true);
    assert_eq!(p.viz[viz::HITS + sub].get(), 0.0);
    p.set(vp(sub, knob::PROB), 0.5);
    run_nullpunkt(&p, 6000, true);
    let played = p.viz[viz::HITS + sub].get();
    assert!((15.0..=33.0).contains(&played), "{played} of 48");
}

#[test]
fn nullpunkt_stutters_in_time() {
    use crate::audio::modules::nullpunkt::{GRAIN, SCATTER, STUTTER, STUTTER_LEN, knob, set_steps, voice, vp};
    let p = quiet_nullpunkt();
    let sub = voice::SUB;
    p.set(vp(sub, knob::LEVEL), 0.8);
    p.set(vp(sub, knob::LEN), 1.0);
    set_steps(&p, sub, 1);
    // Every step stutters in halves (GRAIN 0), nothing scattered: the
    // second half of each step repeats the first, exactly a half-step on.
    p.set(STUTTER, 1.0);
    p.set(STUTTER_LEN, 1.0);
    p.set(GRAIN, 0.0);
    p.set(SCATTER, 0.0);
    let left = run_nullpunkt(&p, 1500, true);
    for step in 1..15 {
        let at = step * 6000;
        let worst = (64..2900).map(|u| (left[at + 3000 + u] - left[at + u]).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-3, "step {step}: off by {worst}");
    }
}

/// What a run of Aether gives: both channels and the glitches it fired
/// (frame, frequency, kind).
type AetherRun = (Vec<f32>, Vec<f32>, Vec<(usize, f32, u8)>);

/// Aether alone for `blocks` blocks at 120 BPM, its MIDI cable carrying
/// `midi` (block, message) if any.
fn run_aether(p: &Arc<ModuleParams>, blocks: usize, playing: bool, midi: &[(usize, [u8; 3])]) -> AetherRun {
    use crate::audio::modules::aether::{Aether, viz};
    let mut proc = Aether::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing, restart: false };
    let mut inputs =
        Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: !midi.is_empty() });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let (mut left, mut right, mut events) = (Vec::new(), Vec::new(), Vec::new());
    let mut seen = p.viz[viz::FIRED].get() as u32;
    for b in 0..blocks {
        inputs.midi.clear();
        for (_, msg) in midi.iter().filter(|(at, _)| *at == b) {
            inputs.midi.push(0, *msg);
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        left.extend_from_slice(&outputs[0][0]);
        right.extend_from_slice(&outputs[0][1]);
        let fired = p.viz[viz::FIRED].get() as u32;
        for n in seen + 1..=fired {
            let at = viz::EVENT + 3 * (n as usize % viz::EVENTS);
            events.push((b * 64, p.viz[at + 1].get(), p.viz[at + 2].get() as u8));
        }
        seen = fired;
        if playing {
            ctx.beat += ctx.beats_per_sample * 64.0;
        }
    }
    (left, right, events)
}

/// Brightness: how much the signal changes from sample to sample, for its
/// level (rises with the share of high frequencies).
fn brightness(x: &[f32]) -> f32 {
    let diff = (x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / x.len() as f32).sqrt();
    diff / rms_of(x).max(1e-9)
}

#[test]
fn aether_sounds_warm_finite_and_under_full_scale() {
    use crate::audio::modules::aether::{DRIVE, LEVEL, SPACE, TONE};
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    let (l, r, _) = run_aether(&p, 3000, true, &[]);
    let all = || l.iter().chain(&r);
    assert!(all().all(|s| s.is_finite()));
    let peak = all().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak < 0.6, "headroom at the defaults: {peak}");
    let body = &l[48_000..];
    assert!(rms_of(body) > 0.04, "it sounds: {}", rms_of(body));
    let warm = brightness(body);
    assert!(warm < 0.1, "warm, not bright: {warm}");
    // Wide: the sides differ.
    let corr = l[48_000..].iter().zip(&r[48_000..]).map(|(a, b)| a * b).sum::<f32>() / (rms_of(&l[48_000..]) * rms_of(&r[48_000..]) * (l.len() - 48_000) as f32);
    assert!(corr < 0.95, "stereo, not mono: {corr}");

    // TONE opens it up.
    p.set(TONE, 1.0);
    let (bright, _, _) = run_aether(&p, 2000, true, &[]);
    assert!(brightness(&bright[48_000..]) > warm * 1.5, "TONE brightens: {} vs {warm}", brightness(&bright[48_000..]));

    // Everything up, everything down: never past full scale.
    for setting in [0.0, 1.0] {
        let def = ModuleKind::Aether.def();
        for (i, spec) in def.params.iter().enumerate() {
            p.set(i, spec.denorm(setting));
        }
        p.set(LEVEL, 1.0);
        p.set(DRIVE, setting);
        p.set(SPACE, setting);
        for mode in [0, 6] {
            p.set_mode(mode);
            let (l, r, _) = run_aether(&p, 1500, true, &[]);
            let peak = l.iter().chain(&r).fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(l.iter().chain(&r).all(|s| s.is_finite()) && peak < 1.0, "setting {setting} mode {mode}: {peak}");
        }
    }
}

#[test]
fn aether_evolves_slowly_and_the_same_from_start() {
    use crate::audio::modules::aether::{EVOLVE, GLITCH, viz};
    use crate::audio::modules::aether::fractal::PARTIALS;
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    p.set(GLITCH, 0.0);
    p.set(EVOLVE, 0.0); // a bar: 2 s at 120 BPM
    let snapshot = |p: &ModuleParams| -> Vec<f32> { (0..5 * PARTIALS).map(|k| p.viz[viz::AMP + k].get()).collect() };
    // The partials' levels move while the knobs stand still…
    let mut shots = Vec::new();
    let mut colors = Vec::new();
    for _ in 0..6 {
        run_aether(&p, 375, true, &[]); // half a second each
        shots.push(snapshot(&p));
        colors.push(p.viz[viz::COLOR].get());
    }
    let change = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.iter().sum::<f32>().max(1e-9);
    // (each run starts from Start: the same moment every time)
    assert!(change(&shots[0], &shots[5]) < 1e-6, "from Start it plays the same");

    // Over one long run, sampled as it goes.
    let mut proc_shots = Vec::new();
    {
        use crate::audio::modules::aether::Aether;
        let mut proc = Aether::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0);
        let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
        let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
        let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
        for b in 0..750 * 8 {
            proc.process(&ctx, &inputs, &mut outputs, 64);
            ctx.beat += ctx.beats_per_sample * 64.0;
            if b % 375 == 374 {
                proc_shots.push((snapshot(&p), p.viz[viz::COLOR].get(), p.viz[viz::POS].get()));
            }
        }
    }
    // Slowly: little changes in a 20th of a second…
    let steps: Vec<f32> = proc_shots.windows(2).map(|w| change(&w[0].0, &w[1].0)).collect();
    let far = change(&proc_shots[0].0, &proc_shots[proc_shots.len() - 1].0);
    assert!(far > 0.1, "the timbre has moved over 8 s: {far}");
    let colors: Vec<f32> = proc_shots.iter().map(|s| s.1).collect();
    let spread = colors.iter().fold(f32::MIN, |m, c| m.max(*c)) - colors.iter().fold(f32::MAX, |m, c| m.min(*c));
    assert!(spread > 0.02, "COLOR wanders: {colors:?}");
    assert!(steps.iter().all(|s| s.is_finite()));
    // Synced, the motion is on the bars: a period is a bar of 4 beats.
    let (_, _, pos) = proc_shots[3];
    let beats = 375.0 * 4.0 * 64.0 * 2.0 / 48_000.0;
    assert!((pos - beats / 4.0).abs() < 0.01, "position {pos} after {beats} beats");

    // RESEED: another evolution.
    let before = colors.clone();
    crate::audio::modules::aether::reseed(&p, 777);
    run_aether(&p, 375 * 3, true, &[]);
    assert!((p.viz[viz::COLOR].get() - before[2]).abs() > 1e-4, "another seed, another path");
}

/// Whether `hz` is a harmonic of one of `notes`, an octave or more down
/// allowed (pings fold into a warm range), within 15 cents.
fn on_a_harmonic(hz: f32, notes: &[f32], octaves_up: bool) -> bool {
    use crate::audio::modules::aether::glitch::HARMONICS;
    notes.iter().any(|&n| {
        let f = crate::audio::midi::note_hz(n);
        HARMONICS.iter().chain(&[1]).any(|&h| {
            (-6..=if octaves_up { 6 } else { 0 }).any(|k| {
                let want = f * h as f32 * 2f32.powi(k);
                (1200.0 * (hz / want).log2()).abs() < 15.0
            })
        })
    })
}

#[test]
fn aether_glitches_ring_on_harmonics_in_time() {
    use crate::audio::modules::aether::glitch::{PING, SKIP as SKIPPED};
    use crate::audio::modules::aether::{DECAY, GLITCH, HARM, MORPH, REPEAT, SKIP, SPARK, data, knob_chord};
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    p.set(MORPH, 0.0); // the voices barely drift
    p.set(GLITCH, 1.0);
    p.set(REPEAT, 0.0);
    p.set(SKIP, 0.0);
    p.data[data::DIV].set(0.0); // quarter notes: every 24 000 frames
    let notes = knob_chord(&p);
    let (with, _, events) = run_aether(&p, 3000, true, &[]);
    assert!(events.len() >= 7, "one a beat: {}", events.len());
    for &(frame, hz, kind) in &events {
        assert_eq!(kind, PING);
        assert!(frame % 24_000 < 64, "on the beat: frame {frame}");
        assert!(on_a_harmonic(hz, &notes, false), "{hz} Hz is not a harmonic of {notes:?}");
    }
    // In tune, not noise: the glitches alone (with them minus without)
    // ring at their pitch and not a quarter tone off.
    p.set(GLITCH, 0.0);
    let (without, _, _) = run_aether(&p, 3000, true, &[]);
    let alone: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    for &(frame, hz, _) in events.iter().skip(1).take(4) {
        let len = 19_200;
        let window: Vec<f32> = alone[frame + 240..frame + 240 + len]
            .iter()
            .enumerate()
            .map(|(i, x)| x * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / len as f32).cos()))
            .collect();
        let tuned = tone_amp(&window, hz);
        let off = tone_amp(&window, hz * 2f32.powf(1.0 / 12.0)).max(tone_amp(&window, hz * 2f32.powf(-1.0 / 12.0)));
        assert!(tuned > 4.0 * off, "{hz} Hz: {tuned} on pitch, {off} a semitone off");
    }

    // HARM keeps them low: at 2, only the octave (folded down or not).
    p.set(GLITCH, 1.0);
    p.set(HARM, 2.0);
    let (_, _, events) = run_aether(&p, 1500, true, &[]);
    for &(_, hz, _) in &events {
        let ok = notes.iter().any(|&n| {
            let ratio = hz / crate::audio::midi::note_hz(n);
            (0..6).any(|k| (1200.0 * (ratio / (2.0 / (1 << k) as f32)).log2()).abs() < 15.0)
        });
        assert!(ok, "{hz} Hz is an octave of a chord tone");
    }

    // Skips loop in tune: an octave of a harmonic of the root.
    p.set(HARM, 16.0);
    p.set(SKIP, 1.0);
    p.set(DECAY, 0.2);
    let (l, _, events) = run_aether(&p, 3000, true, &[]);
    assert!(!events.is_empty() && events.iter().all(|e| e.2 == SKIPPED));
    for &(_, hz, _) in &events {
        assert!(on_a_harmonic(hz, &notes[..1], true), "skip at {hz} Hz");
    }
    assert!(l.iter().all(|s| s.is_finite() && s.abs() < 1.0));

    // REPEAT ratchets them in subdivisions of the step.
    p.set(SKIP, 0.0);
    p.set(REPEAT, 1.0);
    p.set(SPARK, 1.0);
    let (_, _, events) = run_aether(&p, 3000, true, &[]);
    assert!(events.len() > 12, "ratchets: {} glitches in 8 beats", events.len());
    let subdivided = events.iter().filter(|e| e.0 % 24_000 >= 64).count();
    assert!(subdivided > 0);
    for &(frame, _, _) in &events {
        // Every ratchet sits on a 2, 3, 4, 6 or 8-way split of the beat.
        let within = (frame % 24_000) as f32 / 24_000.0;
        let on_grid = [2.0, 3.0, 4.0, 6.0, 8.0].iter().any(|d: &f32| ((within * d).round() - within * d).abs() * 24_000.0 / d < 64.0);
        assert!(on_grid || within < 0.003, "frame {frame} off the grid");
    }

    // GLITCH at 0: none.
    p.set(GLITCH, 0.0);
    assert!(run_aether(&p, 1500, true, &[]).2.is_empty());
}

#[test]
fn aether_plays_the_keys_latches_and_gates() {
    use crate::audio::modules::aether::{CHORDS, GLITCH, MORPH, data, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    p.set(GLITCH, 0.0);
    p.set(MORPH, 0.0);
    let notes = |p: &ModuleParams| -> Vec<i32> { (0..5).map(|v| p.viz[viz::NOTE + v].get().round() as i32).collect() };
    // One key: the chord shape on it.
    let one = [(10, [0x90, 48, 100])];
    run_aether(&p, 400, true, &one);
    assert_eq!(p.viz[viz::SOURCE].get(), 1.0, "keys held");
    let shape = CHORDS[0].2;
    assert_eq!(notes(&p), shape.iter().map(|s| 48 + s).collect::<Vec<_>>());
    // Three keys: themselves, doubled an octave up.
    let three = [(10, [0x90, 50, 100]), (10, [0x90, 53, 100]), (10, [0x90, 57, 100])];
    run_aether(&p, 400, true, &three);
    assert_eq!(notes(&p), vec![50, 53, 57, 62, 65]);
    // Let go: latched, still sounding while the transport plays.
    let tap = [(10, [0x90, 50, 100]), (20, [0x80, 50, 0])];
    let (l, _, _) = run_aether(&p, 1500, true, &tap);
    assert_eq!(p.viz[viz::SOURCE].get(), 2.0, "latched");
    assert!(rms_of(&l[l.len() - 24_000..]) > 0.03);
    // GATE: once the key is up, it fades away, even while playing.
    p.data[data::KEYS].set(1.0);
    p.set(crate::audio::modules::aether::SPACE, 0.0);
    let second = [(10, [0x90, 50, 100]), (760, [0x80, 50, 0])];
    let (l, _, _) = run_aether(&p, 2300, true, &second);
    let held = rms_of(&l[38_000..48_000]);
    assert!(held > 0.03, "it sounds while held");
    assert!(rms_of(&l[l.len() - 4_800..]) < held * 0.1, "it fades after the key");
    // And with the transport stopped, a held key still plays.
    p.data[data::KEYS].set(0.0);
    let held = [(10, [0x90, 45, 100])];
    let (l, _, _) = run_aether(&p, 1200, false, &held);
    assert!(rms_of(&l[48_000..]) > 0.03, "keys play while stopped");
    let (l, _, _) = run_aether(&p, 1200, false, &[]);
    assert!(l.iter().all(|&s| s == 0.0), "stopped and no keys: silent");
}

#[test]
fn aether_swells_in_fades_out_and_rests() {
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    use crate::audio::modules::aether::Aether;
    let mut proc = Aether::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
    let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let mut run = |ctx: &mut ProcessCtx, blocks: usize| -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..blocks {
            proc.process(ctx, &inputs, &mut outputs, 64);
            out.extend_from_slice(&outputs[0][0]);
            ctx.beat += ctx.beats_per_sample * 64.0;
        }
        out
    };
    let start = run(&mut ctx, 1500);
    assert!(rms_of(&start[..2_400]) < rms_of(&start[48_000..]) * 0.5, "it swells in");
    assert!(start.windows(2).all(|w| (w[1] - w[0]).abs() < 0.1), "no clicks");
    ctx.playing = false;
    let stop = run(&mut ctx, 750);
    assert!(rms_of(&stop[..4_800]) > 0.01, "no cut: it fades");
    // The tail rings out, then it rests in exact silence.
    let mut rest = Vec::new();
    for _ in 0..40 {
        rest = run(&mut ctx, 750);
        if rest.iter().all(|&s| s == 0.0) {
            break;
        }
    }
    assert!(rest.iter().all(|&s| s == 0.0), "resting");
    ctx.playing = true;
    let again = run(&mut ctx, 1500);
    assert!(rms_of(&again[48_000..]) > 0.04, "Start: it sounds again");
}

#[test]
fn aether_settings_are_saved() {
    use crate::audio::modules::aether::{COLOR, EVOLVE, HARM, data, glides, reseed, seed};
    let (mut g, a, _e) = setup();
    let id = g.add_module(ModuleKind::Aether, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    p.set_mode(4);
    p.set_toggle(false);
    p.set(COLOR, 0.8);
    p.set(HARM, 5.0);
    p.set(EVOLVE, 0.25);
    p.data[data::DIV].set(5.0);
    p.data[data::KEYS].set(1.0);
    p.data[data::GLIDE].set(1.0);
    reseed(&p, 4321);
    let saved = crate::project::snapshot(&g);

    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    let q = module_params(&h, h.nodes[0].id);
    assert_eq!(q.mode(), 4);
    assert!(!q.toggled());
    assert_eq!((q.get(COLOR), q.get(HARM), q.get(EVOLVE)), (0.8, 5.0, 0.25));
    assert_eq!(q.data[data::DIV].get(), 5.0);
    assert_eq!(q.data[data::KEYS].get(), 1.0);
    assert!(glides(&q));
    assert_eq!(seed(&q), 4321);
    assert_eq!(crate::project::snapshot(&h), saved);
}

#[test]
fn aether_panel_draws_while_it_plays() {
    use crate::audio::modules::aether::{GLITCH, REPEAT, SKIP, viz};
    let shared = Arc::new(EngineShared::new());
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Aether, pos2(0.0, 0.0), &a);
    let mix = g.add_mixer(pos2(900.0, 0.0), &a);
    g.connect(port(id, 0), port(mix, 0), &a);
    let p = module_params(&g, id);
    p.set(GLITCH, 0.8);
    p.set(REPEAT, 0.6);
    p.set(SKIP, 0.4);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for chord in 0..crate::audio::modules::aether::CHORDS.len() {
        p.set_mode(chord);
        p.set_toggle(chord % 2 == 0);
        for _ in 0..3 {
            render_buf(&mut e, 4_800);
            time += 0.1;
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[viz::FIRED].get() > 0.0, "it glitched while drawn");
    assert!(p.viz[viz::RUN].get() > 0.5, "and sounded");
}

#[test]
fn aether_free_notes_impose_nothing() {
    use crate::audio::modules::aether::{VOICES, free_notes};
    // No keys: ROOT alone, on every voice.
    assert_eq!(free_notes(&[], 45.2), ([45.0; VOICES], VOICES));
    // The keys as they are: each once, low to high, nothing added.
    let (notes, n) = free_notes(&[64, 60, 64, 61], 45.0);
    assert_eq!(&notes[..n], &[60.0, 61.0, 64.0]);
    // More keys than voices: spread evenly, the lowest and highest kept.
    let (notes, n) = free_notes(&[50, 52, 53, 55, 57, 59, 60, 62], 45.0);
    assert_eq!(n, VOICES);
    assert_eq!((notes[0], notes[VOICES - 1]), (50.0, 62.0));
    assert!(notes.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn aether_free_plays_the_keys_as_a_plain_pad() {
    use crate::audio::midi::note_hz;
    use crate::audio::modules::aether::glitch::SKIP as SKIPPED;
    use crate::audio::modules::aether::{FREE, GLITCH, MORPH, data, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    p.set(GLITCH, 0.0);
    p.set(MORPH, 0.0);
    // The notes of the voices that sound (faded in), low to high.
    let sounding = |p: &ModuleParams| -> Vec<i32> {
        let mut notes: Vec<i32> =
            (0..5).filter(|&v| p.viz[viz::LIVE + v].get() > 0.5).map(|v| p.viz[viz::NOTE + v].get().round() as i32).collect();
        notes.sort_unstable();
        notes
    };
    // The fifth over ROOT (E3 over A2), for ROOT's level, over a second.
    let fifth = |l: &[f32]| {
        let len = 48_000;
        let w: Vec<f32> =
            l[96_000..96_000 + len].iter().enumerate().map(|(i, x)| x * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / len as f32).cos())).collect();
        tone_amp(&w, note_hz(52.0)) / tone_amp(&w, note_hz(45.0))
    };
    let (l, _, _) = run_aether(&p, 2300, true, &[]);
    let shaped = fifth(&l);
    // FREE adds nothing to ROOT: it sounds alone, in unison on every voice.
    p.set_mode(FREE);
    let (l, _, _) = run_aether(&p, 2300, true, &[]);
    assert_eq!(sounding(&p), vec![45; 5]);
    assert!(fifth(&l) < shaped * 0.05, "no fifth: {} against {shaped}", fifth(&l));
    assert!(rms_of(&l[96_000..]) > 0.02, "and it sounds");

    // One key: that note alone, on one voice; the others fade out.
    run_aether(&p, 1500, true, &[(10, [0x90, 62, 100])]);
    assert_eq!(sounding(&p), vec![62]);
    assert_eq!(p.viz[viz::SOURCE].get(), 1.0);
    // Three keys: those three, nothing doubled or added.
    let three = [(10, [0x90, 50, 100]), (10, [0x90, 53, 100]), (10, [0x90, 58, 100])];
    run_aether(&p, 1500, true, &three);
    assert_eq!(sounding(&p), vec![50, 53, 58]);
    // LATCH: letting go of the keys one by one keeps the chord whole (a
    // voice let go would have faded below 0.95 by the end).
    let letting_go = [three[0], three[1], three[2], (900, [0x80, 58, 0]), (1000, [0x80, 53, 0])];
    run_aether(&p, 1050, true, &letting_go);
    assert_eq!(sounding(&p), vec![50, 53, 58]);
    let lives: Vec<f32> = (0..5).map(|v| p.viz[viz::LIVE + v].get()).filter(|&l| l > 0.5).collect();
    assert!(lives.iter().all(|&l| l > 0.95), "none let go: {lives:?}");
    // Five keys at once stay under full scale.
    let five: Vec<(usize, [u8; 3])> = [45, 52, 57, 61, 64].iter().map(|&k| (0, [0x90, k, 100])).collect();
    let (l, r, _) = run_aether(&p, 1500, true, &five);
    let peak = peak_of(&l).max(peak_of(&r));
    assert!(peak < 0.9, "five keys: peak {peak}");
    // GATE: a key let go fades out on its own note while the rest play on.
    p.data[data::KEYS].set(1.0);
    let gated = [(10, [0x90, 50, 100]), (10, [0x90, 57, 100]), (600, [0x80, 57, 0])];
    let (l, _, _) = run_aether(&p, 1500, true, &gated);
    assert_eq!(sounding(&p), vec![50]);
    let fading = (0..5).find(|&v| p.viz[viz::NOTE + v].get().round() as i32 == 57).expect("it keeps its note");
    let live = p.viz[viz::LIVE + fading].get();
    assert!(live > 0.0 && live < 0.5, "fading: {live}");
    assert!(l.windows(2).all(|w| (w[1] - w[0]).abs() < 0.1), "no clicks");

    // Glitches ring on harmonics of the keys played.
    p.data[data::KEYS].set(0.0);
    p.set(GLITCH, 1.0);
    p.data[data::DIV].set(0.0);
    let chord = [(0, [0x90, 50, 100]), (0, [0x90, 57, 100])];
    let (_, _, events) = run_aether(&p, 3000, true, &chord);
    assert!(events.len() >= 5, "{} glitches", events.len());
    for &(_, hz, kind) in &events {
        assert!(on_a_harmonic(hz, &[50.0, 57.0], kind == SKIPPED), "{hz} Hz is not a harmonic of the keys");
    }
}

#[test]
fn aether_glides_only_with_glide() {
    use crate::audio::modules::aether::{GLITCH, MORPH, data, glides, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    p.set(GLITCH, 0.0);
    p.set(MORPH, 0.0);
    // C3 latched, then G3: the lowest voice goes from 48 to 55.
    let change = [(10, [0x90, 48, 100]), (300, [0x80, 48, 0]), (400, [0x90, 55, 100])];
    let lowest = |p: &ModuleParams| p.viz[viz::NOTE].get();
    // Off at first: the voices step there, without a click.
    assert!(!glides(&p));
    let (l, r, _) = run_aether(&p, 403, true, &change);
    assert!((lowest(&p) - 55.0).abs() < 0.1, "stepped: {}", lowest(&p));
    assert!(l.windows(2).chain(r.windows(2)).all(|w| (w[1] - w[0]).abs() < 0.1), "no clicks");
    // With GLIDE they are on their way (from wherever the glide from ROOT
    // to C3 had got to), and get there.
    p.data[data::GLIDE].set(1.0);
    run_aether(&p, 400, true, &change);
    let before = lowest(&p);
    run_aether(&p, 403, true, &change);
    let after = lowest(&p);
    assert!(after > before + 0.1 && after < 50.0, "gliding: {before} to {after}");
    run_aether(&p, 1300, true, &change);
    assert!((lowest(&p) - 55.0).abs() < 0.1, "there: {}", lowest(&p));
}

#[test]
fn aether_randomizes_within_warm_ranges() {
    use crate::audio::modules::aether::{CHORDS, DECAY, FREE, GLITCH, HARM, LEVEL, REPEAT, ROOT, SKIP, SPARK, data, randomize, seed, spec};
    let fresh = || {
        let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
        p.set(ROOT, 50.0);
        p.set(LEVEL, 0.5);
        p.set_toggle(false);
        p.data[data::KEYS].set(1.0);
        p
    };
    let knobs = |p: &ModuleParams| -> Vec<f32> { (0..20).map(|i| p.get(i)).collect() };
    let p = fresh();
    let before = knobs(&p);
    randomize(&p, 99, false);
    let after = knobs(&p);
    assert!(before.iter().zip(&after).filter(|(a, b)| a != b).count() >= 15, "a new sound");
    assert_eq!((p.get(ROOT), p.get(LEVEL), p.toggled(), p.data[data::KEYS].get()), (50.0, 0.5, false, 1.0), "the player's settings stay");
    for (i, v) in after.iter().enumerate() {
        assert!((spec(i).min..=spec(i).max).contains(v), "{} at {v}", spec(i).name);
    }
    assert!(p.get(GLITCH) <= 0.4 && p.get(HARM).fract() == 0.0);
    assert!(p.mode() < FREE, "a chord shape");
    // The same seed, the same sound.
    let q = fresh();
    randomize(&q, 99, false);
    assert_eq!((knobs(&q), q.mode(), seed(&q)), (after, p.mode(), seed(&p)));

    // FREE stays FREE; the timbre alone keeps the chord, the seed and the
    // glitches.
    let glitches = |p: &ModuleParams| -> Vec<f32> { [GLITCH, HARM, REPEAT, SKIP, DECAY, SPARK].iter().map(|&i| p.get(i)).collect() };
    p.set_mode(FREE);
    let kept = (glitches(&p), seed(&p), p.data[data::DIV].get());
    randomize(&p, 7, true);
    assert_eq!(p.mode(), FREE);
    assert_eq!((glitches(&p), seed(&p), p.data[data::DIV].get()), kept);
    randomize(&p, 8, false);
    assert_eq!(p.mode(), FREE);

    // Whatever comes out stays warm: finite, under full scale, sounding.
    p.data[data::KEYS].set(0.0);
    p.set(LEVEL, crate::audio::modules::aether::spec(LEVEL).default);
    for k in 0..8u32 {
        randomize(&p, 1000 + k, false);
        p.set_mode(k as usize % CHORDS.len());
        let (l, r, _) = run_aether(&p, 1500, true, &[]);
        let peak = peak_of(&l).max(peak_of(&r));
        assert!(l.iter().chain(&r).all(|s| s.is_finite()));
        assert!(peak < 0.9, "seed {k}: peak {peak}");
        assert!(rms_of(&l[48_000..]) > 0.01, "seed {k} sounds");
    }
}

#[test]
fn aether_sounds_save_as_presets() {
    use crate::audio::modules::aether::{COLOR, FACTORY, FREE, LEVEL, ROOT, data, load_factory, reseed, seed};
    use crate::presets::{self, ModulePreset};
    let p = Arc::new(ModuleParams::new(ModuleKind::Aether));
    p.set_mode(FREE);
    p.set_toggle(false);
    p.set(COLOR, 0.77);
    p.data[data::DIV].set(5.0);
    reseed(&p, 2468);
    let dir = TempDir::new("aether-presets");
    let mut list = presets::load(&dir.0, ModuleKind::Aether);
    assert!(list.is_empty());
    presets::store(&mut list, ModulePreset::capture(&p, "Mine"));
    presets::save(&dir.0, ModuleKind::Aether, &list).unwrap();
    let loaded = presets::load(&dir.0, ModuleKind::Aether);
    assert_eq!(loaded, list);
    assert_eq!((loaded[0].mode.as_deref(), loaded[0].toggle), (Some("FREE"), Some(false)));
    // Into a fresh node: the same sound, the chord shape and SYNC included.
    let q = Arc::new(ModuleParams::new(ModuleKind::Aether));
    loaded[0].apply(&q);
    assert_eq!((q.mode(), q.toggled()), (FREE, false));
    assert_eq!((q.get(COLOR), q.data[data::DIV].get(), seed(&q)), (0.77, 5.0, 2468));
    // A preset without a mode or toggle (older ones, other kinds) leaves
    // them be.
    let old: ModulePreset = serde_json::from_str(r#"{"name":"Old","params":{"COLOR":0.1}}"#).unwrap();
    old.apply(&q);
    assert_eq!((q.mode(), q.toggled(), q.get(COLOR)), (FREE, false, 0.1));

    // The built-in sounds keep the player's ROOT, LEVEL, GLIDE and keys, and
    // all play warm.
    q.set(ROOT, 52.0);
    q.set(LEVEL, 0.5);
    q.data[data::KEYS].set(1.0);
    q.data[data::GLIDE].set(1.0);
    load_factory(&q, 1);
    assert_eq!((q.get(ROOT), q.get(LEVEL), q.data[data::KEYS].get(), q.data[data::GLIDE].get()), (52.0, 0.5, 1.0, 1.0));
    assert_eq!(q.mode(), FREE, "{}", FACTORY[1].0);
    q.data[data::KEYS].set(0.0);
    q.set(LEVEL, crate::audio::modules::aether::spec(LEVEL).default);
    for (k, (name, ..)) in FACTORY.iter().enumerate() {
        load_factory(&q, k);
        let (l, r, _) = run_aether(&q, 1500, true, &[]);
        let peak = peak_of(&l).max(peak_of(&r));
        assert!(l.iter().chain(&r).all(|s| s.is_finite()), "{name}");
        assert!(peak < 0.9, "{name}: peak {peak}");
        assert!(rms_of(&l[48_000..]) > 0.01, "{name} plays");
    }
}

/// Runs a Larsen for `blocks` blocks: MIDI as (block, message), an input
/// on IN (a function of the sample), and `each` called before every block
/// (to turn knobs on the way). Left and right.
fn run_larsen_with(
    p: &Arc<ModuleParams>,
    blocks: usize,
    playing: impl Fn(usize) -> bool,
    midi: &[(usize, [u8; 3])],
    input: Option<&dyn Fn(usize) -> f32>,
    each: &dyn Fn(usize),
) -> (Vec<f32>, Vec<f32>) {
    use crate::audio::modules::larsen::Larsen;
    let mut proc = Larsen::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
    let mut inputs =
        Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: !midi.is_empty() });
    inputs.connected[0] = input.is_some();
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for b in 0..blocks {
        each(b);
        ctx.playing = playing(b);
        inputs.midi.clear();
        for (_, msg) in midi.iter().filter(|(at, _)| *at == b) {
            inputs.midi.push(0, *msg);
        }
        if let Some(f) = input {
            for i in 0..64 {
                let x = f(b * 64 + i);
                (inputs.bufs[0][0][i], inputs.bufs[0][1][i]) = (x, x);
            }
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        left.extend_from_slice(&outputs[0][0]);
        right.extend_from_slice(&outputs[0][1]);
    }
    (left, right)
}

fn run_larsen(p: &Arc<ModuleParams>, blocks: usize, midi: &[(usize, [u8; 3])]) -> (Vec<f32>, Vec<f32>) {
    run_larsen_with(p, blocks, |_| true, midi, None, &|_| {})
}

/// The strongest pitch in `buf` between `lo` and `hi` Hz, to a tenth of a
/// semitone.
fn strongest_hz(buf: &[f32], lo: f32, hi: f32) -> f32 {
    let mut best = (0.0, lo);
    let mut hz = lo;
    while hz < hi {
        let a = tone_amp(buf, hz);
        if a > best.0 {
            best = (a, hz);
        }
        hz *= 2f32.powf(1.0 / 120.0);
    }
    best.1
}

fn cents(got: f32, want: f32) -> f32 {
    1200.0 * (got / want).log2()
}

/// Blocks in `secs` seconds.
fn blocks(secs: f32) -> usize {
    (secs * 750.0) as usize
}

#[test]
fn larsen_drones_on_its_own_and_stays_in_range() {
    use crate::audio::modules::larsen::FEEDBACK;
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    let (l, r) = run_larsen(&p, blocks(6.0), &[]);
    // From a floor of noise to a drone within the first second, on both
    // sides (two strings, not one copied).
    assert!(rms_of(&l[24_000..48_000]) > 0.03, "it starts: {}", rms_of(&l[24_000..48_000]));
    assert!(rms_of(&r[96_000..]) > 0.03 && rms_of(&l[96_000..]) > 0.03);
    assert!(l != r, "two strings");
    assert!(l.iter().chain(&r).all(|s| s.is_finite()));
    let peak = peak_of(&l).max(peak_of(&r));
    assert!(peak < 0.9, "peak {peak}");
    // Too little FEEDBACK and the loop dies away; with more it sustains.
    p.set(FEEDBACK, -36.0);
    let (l, _) = run_larsen(&p, blocks(3.0), &[]);
    assert!(rms_of(&l[96_000..]) < 1e-3, "closed: {}", rms_of(&l[96_000..]));
    p.set(FEEDBACK, 12.0);
    let (l, r) = run_larsen(&p, blocks(3.0), &[]);
    assert!(rms_of(&l[96_000..]) > 0.05);
    assert!(peak_of(&l).max(peak_of(&r)) < 0.9, "saturating, not running away");
}

#[test]
fn larsen_body_moves_the_drone_between_partials() {
    use crate::audio::modules::larsen::BODY;
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    let mut pitches = Vec::new();
    for body in [0.002, 0.01, 0.05] {
        p.set(BODY, body);
        let (l, _) = run_larsen(&p, blocks(3.0), &[]);
        pitches.push(strongest_hz(&l[96_000..96_000 + 16_384], 40.0, 3000.0));
    }
    pitches.dedup_by(|a, b| cents(*a, *b).abs() < 50.0);
    assert!(pitches.len() >= 2, "BODY moved it: {pitches:?}");
}

#[test]
fn larsen_pitch_follows_the_keys_and_the_knob_takes_it_back() {
    use crate::audio::midi::note_hz;
    use crate::audio::modules::larsen::{FREQ, data, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    p.data[data::PITCH].set(1.0);
    for note in [36u8, 48, 60, 72] {
        let (l, _) = run_larsen(&p, blocks(3.0), &[(5, [0x90, note, 100])]);
        let want = note_hz(note as f32);
        let got = strongest_hz(&l[96_000..96_000 + 32_768], want * 0.7, want * 1.4);
        assert!(cents(got, want).abs() < 30.0, "note {note}: {got} Hz for {want}");
        assert!(p.viz[viz::MIDI].get() > 0.5 && p.viz[viz::TUNED].get() > 0.5);
        // BODY snapped to a whole number of the note's periods (less the
        // filters' lag, a fraction of one).
        let periods = p.viz[viz::BODY].get() * want;
        assert!((periods - periods.round()).abs() < 0.35 && periods.round() >= 2.0, "{periods} periods");
    }
    // Bend: up two semitones at most.
    let bent = [(5, [0x90, 60, 100]), (5, [0xE0, 0x7F, 0x7F])];
    let (l, _) = run_larsen(&p, blocks(3.0), &bent);
    let got = strongest_hz(&l[96_000..96_000 + 32_768], 250.0, 340.0);
    assert!(cents(got, note_hz(62.0)).abs() < 30.0, "bent to {got}");
    // Let go: the note stays; FREQ moved with no key down takes it back.
    let tap = [(5, [0x90, 60, 100]), (100, [0x80, 60, 0])];
    let freq = p.get(FREQ);
    run_larsen_with(&p, 400, |_| true, &tap, None, &|b| {
        if b == 300 {
            p.set(FREQ, freq + 1.0);
        }
        if b == 200 {
            assert!(p.viz[viz::MIDI].get() > 0.5, "the note stays after its key");
        }
    });
    assert!(p.viz[viz::MIDI].get() < 0.5, "the knob has it back");
    assert!((p.viz[viz::NOTE].get() - (freq + 1.0)).abs() < 0.5);
}

#[test]
fn larsen_gate_opens_the_loop_only_while_a_key_is_held() {
    use crate::audio::modules::larsen::{data, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    p.data[data::PITCH].set(1.0);
    p.data[data::GATE].set(1.0);
    let held = [(750, [0x90, 48, 100]), (2250, [0x80, 48, 0])];
    let (l, _) = run_larsen(&p, blocks(6.0), &held);
    assert!(rms_of(&l[..48_000]) < 1e-3, "silent until a key: {}", rms_of(&l[..48_000]));
    let sounding = rms_of(&l[96_000..144_000]);
    assert!(sounding > 0.03, "it swells on the key: {sounding}");
    assert!(rms_of(&l[l.len() - 24_000..]) < sounding * 0.1, "and dies away after");
    assert_eq!(p.viz[viz::OPEN].get(), 0.0);
}

#[test]
fn larsen_three_strings_play_three_notes() {
    use crate::audio::midi::note_hz;
    use crate::audio::modules::larsen::{FACTORY, FREQ, load_factory, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    let find = |name: &str| FACTORY.iter().position(|s| s.0 == name).unwrap();
    // From the knobs, LOCK on: FREQ, a fifth and an octave up.
    load_factory(&p, find("Fifths"));
    let (l, r) = run_larsen(&p, blocks(4.0), &[]);
    let root = p.get(FREQ);
    for st in [0.0, 7.0, 12.0] {
        let want = note_hz(root + st);
        let got = strongest_hz(&l[144_000..144_000 + 32_768], want * 0.97, want * 1.03);
        assert!(cents(got, want).abs() < 30.0, "+{st}: {got} Hz for {want}");
        assert!(tone_amp(&l[144_000..144_000 + 32_768], got) > 0.01, "+{st} sounds");
    }
    assert!(peak_of(&l).max(peak_of(&r)) < 0.9);
    // From three keys, coupled.
    load_factory(&p, find("Chord keys"));
    let chord = [(5, [0x90, 48, 100]), (5, [0x90, 52, 100]), (5, [0x90, 55, 100])];
    let (l, _) = run_larsen(&p, blocks(3.0), &chord);
    for note in [48.0, 52.0, 55.0] {
        let want = note_hz(note);
        let got = strongest_hz(&l[96_000..96_000 + 32_768], want * 0.97, want * 1.03);
        assert!(cents(got, want).abs() < 30.0, "{note}: {got} Hz for {want}");
        assert!(tone_amp(&l[96_000..96_000 + 32_768], got) > 0.01, "{note} sounds");
    }
    // One key sounds one string: those never played stay closed.
    run_larsen(&p, blocks(1.0), &[(5, [0x90, 50, 100])]);
    let open: Vec<f32> = (0..3).map(|s| p.viz[viz::OPEN + s].get()).collect();
    assert_eq!(open, vec![1.0, 0.0, 0.0]);
}

#[test]
fn larsen_echo_repeats_warps_and_pongs() {
    use crate::audio::modules::larsen::{FACTORY, REPEATS, SEND, TIME, data, load_factory, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    // The loop shut (Init): a burst on IN, and its repeats.
    load_factory(&p, FACTORY.iter().position(|s| s.0 == "Init").unwrap());
    p.set(SEND, 1.0);
    p.set(REPEATS, 0.6);
    p.set(TIME, 0.5);
    let burst = |i: usize| if i < 480 { 0.5 * (i as f32 * 0.3).sin() } else { 0.0 };
    // Around repeat `k`, and just before it (once the string has stopped
    // ringing).
    let window = |x: &[f32], k: usize| rms_of(&x[k * 24_000 - 3000..k * 24_000 + 3000]);
    let before = |x: &[f32], k: usize| rms_of(&x[k * 24_000 - 7000..k * 24_000 - 4000]);
    let (l, r) = run_larsen_with(&p, blocks(2.0), |_| true, &[], Some(&burst), &|_| {});
    // CLASSIC: every 0.5 s, on both sides.
    for k in 1..=3 {
        assert!(window(&l, k) > 1e-4 && window(&r, k) > 1e-4, "repeat {k}");
        assert!(window(&l, k) > 10.0 * before(&l, k), "repeat {k}: {} against {}", window(&l, k), before(&l, k));
    }
    // PONG: the first on the left, the next on the right, and so on.
    p.data[data::PONG].set(1.0);
    let (l, r) = run_larsen_with(&p, blocks(2.0), |_| true, &[], Some(&burst), &|_| {});
    assert!(window(&l, 1) > 3.0 * window(&r, 1), "first left");
    assert!(window(&r, 2) > 3.0 * window(&l, 2), "then right");
    assert!(window(&l, 3) > 3.0 * window(&r, 3), "then left");
    // WARP: a quarter of the time above the shortest.
    p.data[data::WARP].set(1.0);
    run_larsen(&p, blocks(1.0), &[]);
    assert!((p.viz[viz::ECHO].get() - (0.05 + 0.45 / 4.0)).abs() < 1e-3, "{}", p.viz[viz::ECHO].get());
}

#[test]
fn larsen_rests_once_stopped_and_needs_input_as_a_resonator() {
    use crate::audio::modules::larsen::{FACTORY, SEND, load_factory};
    let p = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    // Stopped, it is silent; stopped after playing, it dies away to
    // nothing and rests.
    let (l, _) = run_larsen_with(&p, blocks(2.0), |_| false, &[], None, &|_| {});
    assert!(l.iter().all(|&s| s == 0.0));
    // (No echo: its repeats would take their time.)
    p.set(SEND, 0.0);
    let (l, _) = run_larsen_with(&p, blocks(16.0), |b| b < blocks(3.0), &[], None, &|_| {});
    assert!(rms_of(&l[96_000..144_000]) > 0.03);
    assert!(l[l.len() - 48_000..].iter().all(|&s| s == 0.0), "rests");
    // The resonator plays only what comes in.
    load_factory(&p, FACTORY.iter().position(|s| s.0 == "Resonator (IN)").unwrap());
    let (l, _) = run_larsen(&p, blocks(3.0), &[]);
    assert!(rms_of(&l) < 1e-3, "nothing in, nothing out: {}", rms_of(&l));
    let pulses = |i: usize| if i % 24_000 < 200 { 0.3 * (i as f32 * 0.37).sin() } else { 0.0 };
    let (l, r) = run_larsen_with(&p, blocks(3.0), |_| true, &[], Some(&pulses), &|_| {});
    assert!(rms_of(&l) > 0.005, "it rings with the input: {}", rms_of(&l));
    assert!(peak_of(&l).max(peak_of(&r)) < 0.9);
}

#[test]
fn larsen_settings_and_sounds_are_saved() {
    use crate::audio::modules::larsen::{BODY, FACTORY, FEEDBACK, LEVEL, data, flag, load_factory, spec};
    use crate::presets::{self, ModulePreset};
    let (mut g, a, _e) = setup();
    let id = g.add_module(ModuleKind::Larsen, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    p.set(FEEDBACK, 3.0);
    p.set(BODY, 0.042);
    for d in [data::PONG, data::POLY, data::GATE] {
        p.data[d].set(1.0);
    }
    p.data[data::LOCK].set(0.0);
    let saved = crate::project::snapshot(&g);
    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    let q = module_params(&h, h.nodes[0].id);
    assert_eq!((q.get(FEEDBACK), q.get(BODY)), (3.0, 0.042));
    assert!(flag(&q, data::PONG) && flag(&q, data::POLY) && flag(&q, data::GATE) && !flag(&q, data::LOCK) && !flag(&q, data::PITCH));
    assert_eq!(crate::project::snapshot(&h), saved);

    // A preset holds the switches too.
    let dir = TempDir::new("larsen-presets");
    presets::save(&dir.0, ModuleKind::Larsen, &[ModulePreset::capture(&q, "Mine")]).unwrap();
    let loaded = presets::load(&dir.0, ModuleKind::Larsen);
    let fresh = Arc::new(ModuleParams::new(ModuleKind::Larsen));
    loaded[0].apply(&fresh);
    assert_eq!(fresh.get(BODY), 0.042);
    assert!(flag(&fresh, data::POLY) && !flag(&fresh, data::LOCK));

    // The built-in sounds keep the player's LEVEL; those that drone on
    // their own do, and all stay in range.
    fresh.set(LEVEL, 0.3);
    for (k, (name, ..)) in FACTORY.iter().enumerate() {
        load_factory(&fresh, k);
        assert_eq!(fresh.get(LEVEL), 0.3, "{name}");
        fresh.set(LEVEL, spec(LEVEL).default);
        let (l, r) = run_larsen(&fresh, blocks(4.0), &[]);
        assert!(l.iter().chain(&r).all(|s| s.is_finite()), "{name}");
        let peak = peak_of(&l).max(peak_of(&r));
        assert!(peak < 0.9, "{name}: peak {peak}");
        let waits = matches!(*name, "Keys" | "Chord keys" | "Resonator (IN)" | "Init");
        let rms = rms_of(&l[96_000..]);
        assert!(if waits { rms < 1e-3 } else { rms > 0.03 }, "{name}: {rms}");
        fresh.set(LEVEL, 0.3);
    }
}

#[test]
fn larsen_panel_draws_while_it_plays() {
    use crate::audio::modules::larsen::{data, viz};
    let shared = Arc::new(EngineShared::new());
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Larsen, pos2(0.0, 0.0), &a);
    let mix = g.add_mixer(pos2(900.0, 0.0), &a);
    g.connect(port(id, 0), port(mix, 0), &a);
    let p = module_params(&g, id);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for round in 0..8 {
        p.data[data::POLY].set((round % 2) as f32);
        p.data[data::PONG].set((round / 2 % 2) as f32);
        p.data[data::PITCH].set((round / 4) as f32);
        for _ in 0..3 {
            render_buf(&mut e, 4_800);
            time += 0.1;
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[viz::LEVEL].get() > 0.01, "it sounded while drawn");
}

/// Runs a MIDI Looper at 120 BPM (a beat is 24 000 samples): MIDI on its
/// cable as (sample, message), the board's transport `playing`, and `each`
/// called before every block. What comes out, as (sample, message).
fn run_looper(p: &Arc<ModuleParams>, blocks: usize, playing: bool, midi: &[(usize, [u8; 3])], each: &dyn Fn(usize)) -> Vec<(usize, [u8; 3])> {
    use crate::audio::modules::looper::Looper;
    let mut proc = Looper::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing, restart: playing };
    let mut inputs =
        Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: true });
    let mut outputs: Vec<crate::audio::engine::Buf> = Vec::new();
    let mut out = Vec::new();
    for b in 0..blocks {
        each(b);
        inputs.midi.clear();
        for &(at, msg) in midi.iter().filter(|(at, _)| at / 64 == b) {
            inputs.midi.push(at % 64, msg);
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        for e in proc.midi_out().map(|m| m.events()).unwrap_or(&[]) {
            out.push((b * 64 + e.frame as usize, e.data));
        }
        if playing {
            ctx.beat += ctx.beats_per_sample * 64.0;
        }
        ctx.restart = false;
    }
    out
}

/// Samples a beat at 120 BPM.
const BEAT: usize = 24_000;

/// When `key` starts, in samples.
fn looper_ons(out: &[(usize, [u8; 3])], key: u8) -> Vec<usize> {
    out.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90 && m[2] > 0 && m[1] == key).map(|&(at, _)| at).collect()
}

fn near(at: usize, want: usize) -> bool {
    at.abs_diff(want) <= 64
}

fn looper_with(notes: &[(usize, u8, u32, u32)]) -> Arc<ModuleParams> {
    use crate::audio::modules::looper::{Note, add_note, set_bars};
    let p = Arc::new(ModuleParams::new(ModuleKind::Looper));
    for &(scene, key, at, len) in notes {
        set_bars(&p, scene, 1);
        add_note(&p, scene, Note { at, len, key, velocity: 100, channel: 0 });
    }
    p
}

#[test]
fn looper_records_its_bars_and_plays_them_back() {
    use crate::audio::modules::looper::{ask, count, data, note, request, set_bars, state, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Looper));
    set_bars(&p, 0, 1);
    let arm = |b: usize| {
        if b == 0 {
            request(&p, ask::REC, 0.0);
        }
    };
    // Armed, it records from the next bar (beat 0): two notes in a bar.
    let played = [
        (BEAT / 2, [0x90, 60, 90]),
        (BEAT, [0x80, 60, 0]),
        (2 * BEAT, [0x90, 64, 110]),
        (5 * BEAT / 2, [0x80, 64, 0]),
    ];
    let out = run_looper(&p, 3 * 4 * BEAT / 64, true, &played, &arm);
    assert_eq!(count(&p, 0), 2);
    assert_eq!(p.viz[viz::STATE].get() as usize, state::PLAYING);
    let n = note(&p, 0, 0);
    assert_eq!((n.key, n.at, n.len, n.velocity), (60, 48, 48, 90));
    // Heard as played (THRU), then from the loop on every pass after.
    let ons = looper_ons(&out, 60);
    assert_eq!(ons.len(), 3, "{ons:?}");
    assert!(near(ons[0], BEAT / 2) && near(ons[1], 4 * BEAT + BEAT / 2) && near(ons[2], 8 * BEAT + BEAT / 2), "{ons:?}");
    assert!(looper_ons(&out, 64).iter().any(|&at| near(at, 6 * BEAT)));
    // Each note ends as long as it was held.
    let off = out.iter().find(|(at, m)| *at > 4 * BEAT && m[0] & 0xF0 == 0x80 && m[1] == 60).unwrap().0;
    assert!(near(off, 5 * BEAT), "{off}");
    // Without THRU, only the loop is heard.
    p.data[data::THRU].set(0.0);
    let out = run_looper(&p, 4 * BEAT / 64, true, &played, &|_| {});
    assert_eq!(looper_ons(&out, 60).len(), 1);
}

#[test]
fn looper_starts_on_the_first_note_overdubs_and_undoes() {
    use crate::audio::modules::looper::{ON_NOTE, ask, count, data, note, request, set_bars, state, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Looper));
    set_bars(&p, 0, 1);
    p.data[data::TRIGGER].set(ON_NOTE as f32);
    // Waits, armed, until a note: the bar it falls in is the loop's start.
    let played = [
        (4 * BEAT + BEAT / 4, [0x90, 60, 100]),
        (4 * BEAT + BEAT / 2, [0x80, 60, 0]),
        // Overdubbed in the next pass, at the loop's second beat.
        (9 * BEAT, [0x90, 67, 100]),
        (9 * BEAT + BEAT / 2, [0x80, 67, 0]),
    ];
    let dub_at = (8 * BEAT + BEAT / 2) / 64;
    let undo_at = (13 * BEAT + BEAT / 2) / 64;
    let out = run_looper(&p, 14 * BEAT / 64, true, &played, &|b| {
        if b == 0 {
            request(&p, ask::REC, 0.0);
        }
        if b == 4 * BEAT / 64 - 10 {
            assert_eq!(p.viz[viz::STATE].get() as usize, state::ARMED, "still armed");
        }
        if b == dub_at {
            request(&p, ask::DUB, 0.0);
        }
        if b == undo_at {
            assert_eq!(count(&p, 0), 2, "the overdub joined the loop");
            request(&p, ask::UNDO, 0.0);
        }
    });
    assert_eq!(note(&p, 0, 0).at, 24, "a quarter beat into its bar");
    // The overdubbed note plays from the pass after it was played.
    assert!(looper_ons(&out, 67).iter().any(|&at| near(at, 13 * BEAT)), "{:?}", looper_ons(&out, 67));
    assert_eq!(count(&p, 0), 1, "UNDO took the overdub back");

    // Played an eighth early, a note opens the next bar.
    let early = Arc::new(ModuleParams::new(ModuleKind::Looper));
    set_bars(&early, 0, 1);
    early.data[data::TRIGGER].set(ON_NOTE as f32);
    let played = [(4 * BEAT - BEAT / 5, [0x90, 62, 100]), (4 * BEAT + BEAT / 2, [0x80, 62, 0])];
    run_looper(&early, 9 * BEAT / 64, true, &played, &|b| {
        if b == 0 {
            request(&early, ask::REC, 0.0);
        }
    });
    assert_eq!(count(&early, 0), 1);
    assert_eq!(note(&early, 0, 0).at, 0);
}

#[test]
fn looper_plays_reversed_and_picks_at_random() {
    use crate::audio::modules::looper::{GRID, RANDOM, REVERSE, set_direction};
    let notes: Vec<(usize, u8, u32, u32)> = [60u8, 62, 64, 65].iter().enumerate().map(|(k, &key)| (0, key, k as u32 * 96, 48)).collect();
    let p = looper_with(&notes);
    let first = |out: &[(usize, [u8; 3])], key: u8| looper_ons(out, key)[0];
    let out = run_looper(&p, 4 * BEAT / 64, true, &[], &|_| {});
    assert!(near(first(&out, 60), 0) && near(first(&out, 65), 3 * BEAT));
    // Reversed: each note ends where it started, from the end back.
    set_direction(&p, 0, REVERSE);
    let out = run_looper(&p, 4 * BEAT / 64, true, &[], &|_| {});
    for (key, at) in [(65, BEAT / 2), (64, 3 * BEAT / 2), (62, 5 * BEAT / 2), (60, 7 * BEAT / 2)] {
        assert!(near(first(&out, key), at), "{key}: {}", first(&out, key));
    }
    // RANDOM: a beat's slice at a time (GRID 1/4), each picked anew.
    set_direction(&p, 0, RANDOM);
    p.set(GRID, 0.0);
    let out = run_looper(&p, 32 * BEAT / 64, true, &[], &|_| {});
    let ons: Vec<(usize, u8)> = out.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90 && m[2] > 0).map(|&(at, m)| (at, m[1])).collect();
    assert_eq!(ons.len(), 32, "a note every beat");
    assert!(ons.iter().all(|&(at, _)| near(at, (at + BEAT / 2) / BEAT * BEAT)), "on the beats");
    let keys: Vec<u8> = ons.iter().map(|&(_, k)| k).collect();
    let forward: Vec<u8> = (0..32).map(|k| [60, 62, 64, 65][k % 4]).collect();
    assert_ne!(keys, forward, "not in order");
    for key in [60, 62, 64, 65] {
        assert!(keys.contains(&key), "{key} picked");
    }
}

#[test]
fn looper_switches_scenes_on_launch_and_follows_on() {
    use crate::audio::modules::looper::{HALT, NEXT, ask, data, request, scene, set_follow, set_repeats, state, viz};
    let p = looper_with(&[(0, 60, 0, 48), (1, 72, 0, 48), (2, 84, 0, 48)]);
    // Scene 1 twice, then the next: scene 2, for good.
    set_repeats(&p, 0, 2);
    set_follow(&p, 0, NEXT);
    // Asked for mid-bar, scene 1 comes back on the next bar line.
    let ask_at = (13 * BEAT) / 64;
    let out = run_looper(&p, 20 * BEAT / 64, true, &[], &|b| {
        if b == ask_at {
            request(&p, ask::SCENE, 0.0);
            assert!(p.viz[viz::QUEUED].get() < 0.0 || p.viz[viz::QUEUED].get() >= 0.0);
        }
    });
    let ons = looper_ons(&out, 60);
    assert!(near(ons[0], 0) && near(ons[1], 4 * BEAT) && near(ons[2], 16 * BEAT), "{ons:?}");
    let ons = looper_ons(&out, 72);
    assert!(near(ons[0], 8 * BEAT) && near(ons[1], 12 * BEAT) && ons.len() == 2, "{ons:?}");
    // Program Change 3 picks scene 3 the same way.
    p.data[data::SCENE].set(0.0);
    set_repeats(&p, 0, 0);
    let out = run_looper(&p, 9 * BEAT / 64, true, &[(BEAT, [0xC0, 2, 0])], &|_| {});
    assert!(looper_ons(&out, 84).iter().any(|&at| near(at, 4 * BEAT)));
    assert_eq!(scene(&p), 2);
    // STOP after the last repeat.
    p.data[data::SCENE].set(0.0);
    set_repeats(&p, 0, 1);
    set_follow(&p, 0, HALT);
    let out = run_looper(&p, 12 * BEAT / 64, true, &[], &|_| {});
    assert_eq!(looper_ons(&out, 60).len(), 1);
    assert_eq!(p.viz[viz::STATE].get() as usize, state::STOPPED);
}

#[test]
fn looper_follows_a_midi_clock() {
    use crate::audio::midi::MidiState;
    use crate::audio::modules::looper::{data, viz};
    let p = looper_with(&[(0, 60, 96, 48)]);
    p.data[data::SYNC].set(1.0);
    // Start, then 24 ticks a beat at 120 BPM, then Stop in the second bar.
    let mut clock = vec![(0, [0xFA, 0, 0])];
    clock.extend((0..6 * 24).map(|k| (k * 1000, [0xF8, 0, 0])));
    clock.push((6 * BEAT + 10, [0xFC, 0, 0]));
    clock.sort_by_key(|&(at, m)| (at, m[0] != 0xFA));
    // The board stands still: only the clock moves it.
    let out = run_looper(&p, 8 * BEAT / 64, false, &clock, &|b| {
        if b == 3 * BEAT / 64 {
            assert!((p.viz[viz::TEMPO].get() - 120.0).abs() < 1.0, "{}", p.viz[viz::TEMPO].get());
        }
    });
    let ons = looper_ons(&out, 60);
    assert_eq!(ons.len(), 2, "{ons:?}");
    assert!(near(ons[0], BEAT) && near(ons[1], 5 * BEAT), "{ons:?}");
    assert!(out.iter().filter(|(at, _)| *at > 6 * BEAT + 64).all(|(_, m)| m[0] & 0xF0 != 0x90), "nothing after Stop");

    // The clock reaches the stream (for the looper), but is no activity.
    let midi = MidiState::new();
    let (cursor, events) = (midi.stream_head(), midi.events());
    midi.handle(&[0xF8]);
    midi.handle(&[0xFA]);
    let mut got = Vec::new();
    midi.read_stream(&mut { cursor }, |m| got.push(m[0]));
    assert_eq!(got, vec![0xF8, 0xFA]);
    assert_eq!(midi.events(), events);
}

#[test]
fn looper_step_records_and_takes_commands_over_midi() {
    use crate::audio::modules::looper::{count, data, note, scene, set_bars, state, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Looper));
    set_bars(&p, 0, 1);
    p.data[data::STEP].set(1.0);
    // A note, a chord, a note: three steps (a sixteenth each).
    let played = [
        (0, [0x90, 60, 100]),
        (100, [0x80, 60, 0]),
        (200, [0x90, 64, 80]),
        (210, [0x90, 67, 80]),
        (300, [0x80, 64, 0]),
        (310, [0x80, 67, 0]),
        (400, [0x90, 72, 100]),
        (500, [0x80, 72, 0]),
        // CTRL channel 16: C2 arms REC (not heard, not recorded).
        (600, [0x9F, 36, 100]),
        (700, [0x8F, 36, 0]),
        // Program Change 2: scene 2.
        (800, [0xC0, 1, 0]),
    ];
    let out = run_looper(&p, 20, true, &played, &|_| {});
    let notes: Vec<(u32, u8, u32)> = (0..count(&p, 0)).map(|i| note(&p, 0, i)).map(|n| (n.at, n.key, n.len)).collect();
    assert_eq!(notes, vec![(0, 60, 24), (24, 64, 24), (24, 67, 24), (48, 72, 24)]);
    assert_eq!(p.viz[viz::CURSOR].get(), 72.0);
    assert_eq!(p.viz[viz::STATE].get() as usize, state::ARMED);
    assert!(out.iter().all(|(_, m)| m[1] != 36 || m[0] & 0x0F != 15), "commands are not music");
    assert_eq!(scene(&p), 1);
}

#[test]
fn looper_scenes_are_saved() {
    use crate::audio::modules::looper::{
        Note, REVERSE, add_note, bars, count, data, direction, follow, note, repeats, scene, set_bars, set_direction, set_follow, set_repeats,
    };
    let (mut g, a, _e) = setup();
    let id = g.add_module(ModuleKind::Looper, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    set_bars(&p, 5, 12);
    set_repeats(&p, 5, 4);
    set_direction(&p, 5, REVERSE);
    set_follow(&p, 5, 1);
    let n = Note { at: 4000, len: 300, key: 91, velocity: 77, channel: 9 };
    add_note(&p, 5, n);
    p.data[data::SCENE].set(5.0);
    p.data[data::SYNC].set(1.0);
    let saved = crate::project::snapshot(&g);
    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    let q = module_params(&h, h.nodes[0].id);
    assert_eq!((bars(&q, 5), repeats(&q, 5), direction(&q, 5), follow(&q, 5), count(&q, 5)), (12, 4, REVERSE, 1, 1));
    assert_eq!(note(&q, 5, 0), n);
    assert_eq!(scene(&q), 5);
    assert_eq!(crate::project::snapshot(&h), saved);
}

#[test]
fn looper_panel_draws_while_it_plays() {
    use crate::audio::modules::looper::{RANDOM, REVERSE, ask, request, set_direction, viz};
    let shared = Arc::new(EngineShared::new());
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Looper, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    for s in 0..3 {
        for k in 0..8u32 {
            crate::audio::modules::looper::add_note(&p, s, crate::audio::modules::looper::Note { at: k * 48, len: 40, key: 50 + (k * 5 % 30) as u8, velocity: 100, channel: 0 });
        }
    }
    crate::audio::modules::looper::set_bars(&p, 1, 9);
    set_direction(&p, 1, REVERSE);
    set_direction(&p, 2, RANDOM);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for round in 0..9 {
        if round % 3 == 0 {
            request(&p, ask::SCENE, (round / 3) as f32);
        }
        if round == 7 {
            request(&p, ask::REC, 0.0);
        }
        for _ in 0..3 {
            render_buf(&mut e, 9_600);
            time += 0.1;
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[viz::RUNNING].get() > 0.5, "it ran while drawn");
}

/// Vesper on a test bench: at 120 BPM (a bar is 96 000 samples), its sounds
/// already made. MIDI goes in on its cable; what comes out is kept.
struct VesperRig {
    proc: crate::audio::modules::vesper::Vesper,
    ctx: ProcessCtx,
    inputs: Box<Inputs>,
    outputs: Vec<crate::audio::engine::Buf>,
    p: Arc<ModuleParams>,
    at: usize,
}

/// What a run gave: left, right, and the MIDI that came out.
type VesperTake = (Vec<f32>, Vec<f32>, Vec<(usize, [u8; 3])>);

/// Samples in a bar, and blocks in a bar.
const VESPER_BAR: usize = 96_000;
const VESPER_BAR_BLOCKS: usize = VESPER_BAR / 64;

impl VesperRig {
    fn new(p: &Arc<ModuleParams>) -> Self {
        use crate::audio::modules::vesper::Vesper;
        let proc = Vesper::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0);
        let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: false, restart: false };
        let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: true });
        let mut rig = Self { proc, ctx, inputs, outputs: vec![[[0.0f32; 64]; 2]; 1], p: p.clone(), at: 0 };
        rig.wait_for_sounds();
        rig
    }

    /// The sounds picked have arrived.
    fn wait_for_sounds(&mut self) {
        use crate::audio::modules::vesper::{SAMPLED, sound, viz};
        let playing = self.ctx.playing;
        self.ctx.playing = false;
        for _ in 0..4000 {
            self.proc.process(&self.ctx, &self.inputs, &mut self.outputs, 64);
            if (0..SAMPLED).all(|l| self.p.viz[viz::LOADED + l].get() == sound(&self.p, l) as f32) {
                self.ctx.playing = playing;
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the sounds never arrived");
    }

    /// Runs `blocks`; `keys` are MIDI messages to send at a block from now.
    fn run(&mut self, blocks: usize, playing: bool, keys: &[(usize, [u8; 3])]) -> VesperTake {
        let (mut l, mut r, mut midi) = (Vec::new(), Vec::new(), Vec::new());
        for b in 0..blocks {
            self.ctx.playing = playing;
            self.inputs.midi.clear();
            for (_, msg) in keys.iter().filter(|(at, _)| *at == b) {
                self.inputs.midi.push(0, *msg);
            }
            self.proc.process(&self.ctx, &self.inputs, &mut self.outputs, 64);
            l.extend_from_slice(&self.outputs[0][0]);
            r.extend_from_slice(&self.outputs[0][1]);
            for e in self.proc.midi_out().map(|m| m.events()).unwrap_or(&[]) {
                midi.push((self.at + e.frame as usize, e.data));
            }
            if playing {
                self.ctx.beat += self.ctx.beats_per_sample * 64.0;
            }
            self.at += 64;
            self.ctx.restart = false;
        }
        (l, r, midi)
    }
}

/// Only `layer` sounds, dry and unmoving.
fn vesper_solo(p: &ModuleParams, layer: usize) {
    use crate::audio::modules::vesper::{LAYERS, data, k, knob};
    for l in 0..LAYERS {
        p.set(knob(l, k::LEVEL), if l == layer { 0.8 } else { 0.0 });
        crate::audio::modules::vesper::set_flag(p, data::MUTE + l, l != layer);
        p.set(knob(l, k::REVERB), 0.0);
        p.set(knob(l, k::ECHO), 0.0);
        p.set(knob(l, k::CHORUS), 0.0);
        p.set(knob(l, k::DRIVE), 0.0);
        p.set(knob(l, k::MOVE1), 0.0);
        p.set(knob(l, k::MOVE2), 0.0);
        p.set(knob(l, k::FREQ1), 20_000.0);
        p.set(knob(l, k::FREQ2), 20.0);
    }
}

/// The notes sounding on a channel, at each bar line, from what came out.
fn vesper_sounding(midi: &[(usize, [u8; 3])], channel: u8, at: usize) -> Vec<u8> {
    let mut on = Vec::new();
    for &(_, msg) in midi.iter().filter(|(when, _)| *when <= at) {
        if msg[0] & 0x0F != channel {
            continue;
        }
        match msg[0] & 0xF0 {
            0x90 => on.push(msg[1]),
            0x80 => on.retain(|&n| n != msg[1]),
            _ => {}
        }
    }
    on.sort_unstable();
    on
}

#[test]
fn vesper_every_sound_renders() {
    use crate::audio::modules::vesper::library::{LOOP_RMS, render_table};
    use crate::audio::modules::vesper::sounds::{PINGS, recipes};
    use crate::audio::modules::vesper::{SAMPLED, sound_count, sound_name};
    assert!(PINGS.len() >= 12);
    for layer in 0..SAMPLED {
        let list = recipes(layer);
        assert!(list.len() >= 40, "at least forty sounds a layer: {layer} has {}", list.len());
        for (k, r) in list.iter().enumerate() {
            let t = render_table(r, 48.0, 1 << 15, 48_000.0, 77 + k as u32);
            assert!(t.left.iter().chain(t.right.iter()).all(|x| x.is_finite()), "{}: finite", r.name);
            let rms = rms_of(&t.left);
            assert!(rms > 0.3 * LOOP_RMS && rms < 3.0 * LOOP_RMS, "{}: rms {rms}", r.name);
            assert_ne!(t.left, t.right, "{}: two channels", r.name);
        }
        let mut names: Vec<&str> = (0..sound_count(layer)).map(|s| sound_name(layer, s)).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), sound_count(layer), "the names of layer {layer} are all different");
    }
}

#[test]
fn vesper_loops_have_no_seam() {
    use crate::audio::modules::vesper::library::render_table;
    use crate::audio::modules::vesper::sounds::recipes;
    use crate::audio::modules::vesper::{NOISE, PAD, find_sound};
    // A tone, a tone with swells and a sweep, a wind.
    for (layer, name) in [(PAD, "Slow Silk"), (PAD, "Tidal Sweep"), (NOISE, "Mountain Wind")] {
        let r = &recipes(layer)[find_sound(layer, name)];
        let t = render_table(r, 48.0, 1 << 16, 48_000.0, 5);
        let n = t.left.len();
        let biggest = t.left.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        let seam = (t.left[0] - t.left[n - 1]).abs();
        assert!(seam <= biggest * 1.01, "{name}: the seam jumps {seam}, the loop at most {biggest}");
    }
}

#[test]
fn vesper_chords_follow_the_rules() {
    use crate::audio::modules::vesper::harmony::{MOODS, Rules, Step, allowed, fold, label, progression, voice};
    assert_eq!(MOODS.len(), 16, "sixteen moods");
    for mood in 0..MOODS.len() {
        for seed in 1..40u32 {
            for variation in 1..=7 {
                let rules = Rules { root: (seed % 12) as i32, mood, variation, complexity: 3 };
                let (degrees, len) = allowed(seed, &rules);
                assert_eq!(len, variation);
                assert_eq!(degrees[0], 0, "the tonic is always allowed");
                let more = Rules { variation: (variation + 1).min(7), ..rules };
                let (wider, wider_len) = allowed(seed, &more);
                assert!(degrees[..len].iter().all(|d| wider[..wider_len].contains(d)), "a wider choice keeps the narrower");
                let prog = progression(seed, 8, &rules);
                for s in &prog {
                    assert!(degrees[..len].contains(&s.degree), "mood {mood}, variation {variation}: degree {}", s.degree);
                }
                if variation > 1 {
                    let (a, b) = (prog[0], prog[7]);
                    assert_ne!(a.degree, b.degree, "the last chord doesn't land on the first");
                    assert!(prog.windows(2).all(|w| w[0].degree != w[1].degree), "no chord follows itself");
                }
            }
            // Each chord has its notes, in the scale, laid out low to high,
            // moving little from the one before.
            for complexity in 2..=4 {
                let rules = Rules { root: 9, mood, variation: 7, complexity };
                let mut prev: Vec<i32> = Vec::new();
                let mut moved = 0;
                for &step in &progression(seed, 8, &rules) {
                    let mut notes = [0; 4];
                    let n = voice(&rules, step, &prev, 60, &mut notes);
                    assert_eq!(n, complexity);
                    assert!(notes[..n].windows(2).all(|w| w[0] < w[1]), "{:?}", &notes[..n]);
                    for &m in &notes[..n] {
                        assert!(rules.in_scale(m), "mood {mood}: {m} is outside the scale");
                        assert!((40..=84).contains(&m), "{m} out of register");
                    }
                    let (tones, _) = rules.tones(step);
                    let pcs: Vec<i32> = tones[..n].iter().map(|t| (rules.root + t).rem_euclid(12)).collect();
                    assert!(notes[..n].iter().all(|m| pcs.contains(&m.rem_euclid(12))), "the chord's own notes");
                    if !prev.is_empty() {
                        moved += notes[..n].iter().map(|m| prev.iter().map(|q| (m - q).abs()).min().unwrap()).sum::<i32>();
                    }
                    prev = notes[..n].to_vec();
                    assert!(!label(&rules, step).is_empty());
                }
                assert!(moved <= 7 * complexity as i32 * 5, "voices move little: {moved}");
            }
        }
    }
    // Names, spelled for the key.
    let am = Rules { root: 9, mood: 4, variation: 7, complexity: 4 };
    assert_eq!(label(&am, Step { degree: 0, flavor: 0 }), "Am7");
    assert_eq!(label(&am, Step { degree: 2, flavor: 0 }), "Cmaj7");
    let triad = Rules { complexity: 3, ..am };
    assert_eq!(label(&triad, Step { degree: 5, flavor: 0 }), "F");
    assert_eq!(label(&triad, Step { degree: 0, flavor: 2 }), "Asus4");
    let open = Rules { complexity: 2, ..am };
    assert_eq!(label(&open, Step { degree: 0, flavor: 0 }), "A5");
    assert_eq!(fold(60, 33), 36);
}

#[test]
fn vesper_loop_repeats_on_the_bars_and_sends_its_score() {
    use crate::audio::modules::vesper::{CHORDS, COMPLEXITY, DURATION, MOOD, ROOT, VARIATION, generate, rules};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    p.set(DURATION, 1.0); // a bar a chord
    p.set(CHORDS, 3.0);
    p.set(COMPLEXITY, 4.0);
    p.set(VARIATION, 6.0);
    p.set(MOOD, 5.0);
    p.set(ROOT, 2.0);
    let mut rig = VesperRig::new(&p);
    let (_, _, midi) = rig.run(VESPER_BAR_BLOCKS * 9, true, &[]);
    let rules_now = rules(&p);
    // Chord on channel 1, four notes in the scale; the same every three bars
    // (after its first time round, which starts from nothing).
    let at = |bar: usize| vesper_sounding(&midi, 0, bar * VESPER_BAR + 10);
    for bar in 0..9 {
        let notes = at(bar);
        assert_eq!(notes.len(), 4, "bar {bar}: {notes:?}");
        assert!(notes.iter().all(|&n| rules_now.in_scale(n as i32)), "bar {bar}: {notes:?}");
    }
    assert_eq!(at(3), at(6));
    assert_eq!(at(4), at(7));
    assert_eq!(at(5), at(8));
    assert!(at(3) != at(4) || at(4) != at(5), "the chords change");
    // They change on the bar lines and only there.
    for &(when, msg) in midi.iter().filter(|(_, m)| m[0] & 0x0F == 0) {
        assert_eq!(when % VESPER_BAR, 0, "{msg:?} at {when}");
    }
    // The bass, on channel 2, one note at a time, the chord's root.
    for bar in 0..9 {
        let bass = vesper_sounding(&midi, 1, bar * VESPER_BAR + 10);
        assert_eq!(bass.len(), 1, "bar {bar}");
        assert!(rules_now.in_scale(bass[0] as i32));
    }
    // GENERATE writes something else, on the same rules.
    let before: Vec<Vec<u8>> = (3..6).map(at).collect();
    generate(&p, 123_456);
    let (_, _, midi2) = rig.run(VESPER_BAR_BLOCKS * 7, true, &[]);
    let mut all = midi.clone();
    all.extend_from_slice(&midi2);
    let after: Vec<Vec<u8>> = (12..15).map(|bar| vesper_sounding(&all, 0, bar * VESPER_BAR + 10)).collect();
    assert_ne!(before, after, "GENERATE writes new chords");
    assert!(after.iter().flatten().all(|&n| rules_now.in_scale(n as i32)));
}

#[test]
fn vesper_evolve_keeps_writing_within_the_rules() {
    use crate::audio::modules::vesper::{DURATION, EVOLVE, VARIATION, rules, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    p.set_mode(EVOLVE);
    p.set(DURATION, 0.0); // half a bar a chord
    p.set(VARIATION, 7.0);
    let mut rig = VesperRig::new(&p);
    let bars = 16;
    let (_, _, midi) = rig.run(VESPER_BAR_BLOCKS * bars, true, &[]);
    let chords: Vec<Vec<u8>> = (0..bars * 2).map(|c| vesper_sounding(&midi, 0, c * VESPER_BAR / 2 + 10)).collect();
    let rules_now = rules(&p);
    assert!(chords.iter().flatten().all(|&n| rules_now.in_scale(n as i32)));
    assert!(chords.windows(2).filter(|w| w[0] != w[1]).count() >= chords.len() / 2, "it moves on");
    let mut distinct = chords.clone();
    distinct.sort();
    distinct.dedup();
    assert!(distinct.len() >= 7, "it doesn't just go round: {} different chords", distinct.len());
    assert_eq!(p.viz[viz::CARDS].get() as usize, 8, "it shows what it has written, and what's next");
}

#[test]
fn vesper_each_layer_sounds_and_all_stay_in_range() {
    use crate::audio::modules::vesper::{LAYERS, MIXTYPE, RATE, data, k, knob, set_flag};
    for layer in 0..LAYERS {
        let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
        vesper_solo(&p, layer);
        p.set(RATE, 6.0);
        let mut rig = VesperRig::new(&p);
        let (l, r, _) = rig.run(blocks(6.0), true, &[]);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()), "layer {layer}");
        let rms = rms_of(&l[144_000..]);
        assert!(rms > 0.004, "layer {layer} sounds alone: {rms}");
        assert_ne!(l[200_000..200_500], r[200_000..200_500], "layer {layer} is stereo");
    }
    // Everything up, in every MixType.
    for mix in 0..4 {
        let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
        p.set(MIXTYPE, mix as f32);
        p.set(RATE, 6.0);
        for l in 0..LAYERS {
            p.set(knob(l, k::LEVEL), 1.0);
            p.set(knob(l, k::DRIVE), 1.0);
            p.set(knob(l, k::REVERB), 1.0);
            p.set(knob(l, k::ECHO), 1.0);
            p.set(knob(l, k::CHORUS), 1.0);
            set_flag(&p, data::MUTE + l, false);
        }
        let mut rig = VesperRig::new(&p);
        let (l, r, _) = rig.run(blocks(10.0), true, &[]);
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() <= 1.0), "mix {mix}");
        assert!(rms_of(&l[240_000..]) > 0.05);
    }
}

#[test]
fn vesper_plays_in_tune_from_the_keys() {
    use crate::audio::modules::vesper::{BASS, PAD, find_sound, set_sound};
    // The Pad plays the key.
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    p.set_toggle(false);
    vesper_solo(&p, PAD);
    set_sound(&p, PAD, find_sound(PAD, "Felt Organ"));
    let mut rig = VesperRig::new(&p);
    let (l, _, _) = rig.run(blocks(6.0), false, &[(0, [0x90, 69, 100])]);
    let hz = strongest_hz(&l[192_000..], 300.0, 600.0);
    assert!(cents(hz, 440.0).abs() < 12.0, "the Pad on A4: {hz} Hz");

    // The Bass plays the lowest key, folded into its octave: C4 is C2 there.
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    p.set_toggle(false);
    vesper_solo(&p, BASS);
    set_sound(&p, BASS, find_sound(BASS, "Pure Sub"));
    let mut rig = VesperRig::new(&p);
    let (l, _, _) = rig.run(blocks(6.0), false, &[(0, [0x90, 67, 100]), (0, [0x90, 60, 100])]);
    let hz = strongest_hz(&l[192_000..], 40.0, 120.0);
    assert!(cents(hz, 65.406).abs() < 6.0, "the Bass on C2: {hz} Hz");

    // In every register a Pad's loops stay on the note.
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    p.set_toggle(false);
    vesper_solo(&p, PAD);
    set_sound(&p, PAD, find_sound(PAD, "Felt Organ"));
    let mut rig = VesperRig::new(&p);
    for note in [40u8, 57, 75, 88] {
        let (l, _, _) = rig.run(blocks(4.0), false, &[(0, [0x90, note, 100])]);
        let want = crate::audio::midi::note_hz(note as f32);
        let hz = strongest_hz(&l[120_000..], want * 0.8, want * 1.2);
        assert!(cents(hz, want).abs() < 12.0, "note {note}: {hz} Hz against {want} Hz");
        rig.run(blocks(6.0), false, &[(0, [0x80, note, 0])]);
    }
}

#[test]
fn vesper_keys_play_only_with_play_off() {
    use crate::audio::modules::vesper::{LAYERS, k, knob};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    for l in 0..LAYERS {
        p.set(knob(l, k::ATTACK), 0.05);
        p.set(knob(l, k::RELEASE), 0.1);
        p.set(knob(l, k::REVERB), 0.0);
        p.set(knob(l, k::ECHO), 0.0);
    }
    let mut rig = VesperRig::new(&p);
    // PLAY on, transport stopped: keys do nothing.
    let (l, _, _) = rig.run(blocks(1.0), false, &[(0, [0x90, 60, 100])]);
    assert!(rms_of(&l) < 1e-6, "with PLAY on the keys are off: {}", rms_of(&l));
    rig.run(1, false, &[(0, [0x80, 60, 0])]);
    // PLAY off: they play, and the transport alone does not.
    p.set_toggle(false);
    let (l, _, midi) = rig.run(blocks(1.0), true, &[]);
    assert!(rms_of(&l[24_000..]) < 1e-4, "PLAY off and no key: silence, whatever the transport: {}", rms_of(&l[24_000..]));
    assert!(midi.iter().all(|(_, m)| m[0] & 0xF0 != 0x90), "and no score");
    let (l, _, _) = rig.run(blocks(1.0), false, &[(0, [0x90, 60, 100])]);
    assert!(rms_of(&l[24_000..]) > 0.01, "a key plays it: {}", rms_of(&l[24_000..]));
    let (l, _, _) = rig.run(blocks(1.5), false, &[(0, [0x80, 60, 0])]);
    assert!(rms_of(&l[48_000..]) < 1e-3, "and letting go ends it: {}", rms_of(&l[48_000..]));
}

#[test]
fn vesper_shuffle_respects_the_locks_and_generate_keeps_the_sounds() {
    use crate::audio::modules::vesper::{BASS, LAYERS, PAD, data, generate, seed, set_flag, shuffle, sound};
    let p = ModuleParams::new(ModuleKind::Vesper);
    set_flag(&p, data::LOCK + PAD, true);
    set_flag(&p, data::LOCK + BASS, true);
    let before: Vec<usize> = (0..LAYERS).map(|l| sound(&p, l)).collect();
    let chords = seed(&p);
    for s in 1..30 {
        shuffle(&p, s * 7919);
        assert_eq!(sound(&p, PAD), before[PAD], "a locked layer stays");
        assert_eq!(sound(&p, BASS), before[BASS], "a locked layer stays");
        assert_eq!(seed(&p), chords, "SHUFFLE leaves the chords");
    }
    assert!((2..LAYERS).all(|l| sound(&p, l) != before[l] || l == LAYERS), "the others have moved");
    let sounds: Vec<usize> = (0..LAYERS).map(|l| sound(&p, l)).collect();
    generate(&p, 4242);
    assert_ne!(seed(&p), chords);
    assert_eq!(sounds, (0..LAYERS).map(|l| sound(&p, l)).collect::<Vec<_>>(), "GENERATE leaves the sounds");
    // SHUFFLE always moves an unlocked layer on.
    for s in 1..200 {
        let was = sound(&p, 2);
        shuffle(&p, s);
        assert_ne!(sound(&p, 2), was);
    }
}

#[test]
fn vesper_scenes_and_settings_are_saved() {
    use crate::audio::modules::vesper::{FACTORY, LAYERS, LEVEL, MOOD, ROOT, data, k, knob, load_factory, set_sound, sound, sound_count, sound_name, spec};
    use crate::presets::ModulePreset;
    let names: std::collections::HashSet<&str> = ModuleKind::Vesper.def().params.iter().map(|s| s.name).collect();
    assert_eq!(names.len(), ModuleKind::Vesper.def().params.len(), "knobs are saved by name: no two the same");
    // Every scene's sounds exist; a scene sets them, and leaves ROOT and LEVEL.
    let p = ModuleParams::new(ModuleKind::Vesper);
    p.set(ROOT, 3.0);
    p.set(LEVEL, 0.5);
    for (n, scene) in FACTORY.iter().enumerate() {
        for layer in 0..LAYERS {
            assert!((0..sound_count(layer)).any(|s| sound_name(layer, s) == scene.sounds[layer]), "{}: no {} on layer {layer}", scene.name, scene.sounds[layer]);
        }
        for &(i, v) in scene.knobs {
            let s = spec(i);
            assert!(v >= s.min && v <= s.max, "{}: {} is {v}", scene.name, s.name);
        }
        load_factory(&p, n);
        for layer in 0..LAYERS {
            assert_eq!(sound_name(layer, sound(&p, layer)), scene.sounds[layer]);
        }
        assert_eq!(p.get(MOOD), scene.mood as f32);
        assert_eq!(p.get(ROOT), 3.0);
        assert_eq!(p.get(LEVEL), 0.5);
    }
    // A scene overrides what the one before set.
    let brass = FACTORY.iter().position(|s| s.name == "Folded City").unwrap();
    load_factory(&p, brass);
    assert!(p.get(knob(0, k::HOLD)) < 10.0);
    load_factory(&p, 0);
    assert_eq!(p.get(knob(0, k::HOLD)), spec(knob(0, k::HOLD)).default);

    // A preset brings everything back.
    set_sound(&p, 0, 33);
    p.set(knob(3, k::REVERB), 0.123);
    p.data[data::KIND2 + 1].set(3.0);
    p.set_mode(1);
    p.set_toggle(false);
    let preset = ModulePreset::capture(&p, "Mine");
    let q = ModuleParams::new(ModuleKind::Vesper);
    preset.apply(&q);
    assert_eq!(sound(&q, 0), 33);
    assert_eq!(q.get(knob(3, k::REVERB)), 0.123);
    assert_eq!(q.data[data::KIND2 + 1].get(), 3.0);
    assert_eq!(q.mode(), 1);
    assert!(!q.toggled());
}

#[test]
fn vesper_a_new_sound_comes_in_without_a_jump() {
    use crate::audio::modules::vesper::{PAD, find_sound, set_sound, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    let mut rig = VesperRig::new(&p);
    rig.run(blocks(4.0), true, &[]);
    let wanted = find_sound(PAD, "Bowed Glass");
    set_sound(&p, PAD, wanted);
    let mut biggest = 0.0f32;
    let mut arrived = false;
    let mut last = 0.0f32;
    for _ in 0..4000 {
        let (l, _, _) = rig.run(64, true, &[]);
        for x in l {
            biggest = biggest.max((x - last).abs());
            last = x;
        }
        if p.viz[viz::LOADED + PAD].get() == wanted as f32 {
            arrived = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
        if arrived {
            break;
        }
    }
    assert!(arrived, "the new sound arrived");
    let (l, _, _) = rig.run(blocks(3.0), true, &[]);
    for x in &l {
        biggest = biggest.max((x - last).abs());
        last = *x;
    }
    assert!(biggest < 0.2, "no click on the way: {biggest}");
    assert!(rms_of(&l) > 0.01, "and it plays");
}

#[test]
fn vesper_rests_once_stopped_and_ends_its_notes() {
    use crate::audio::modules::vesper::{ECHO, LAYERS, REPEATS, SPACE, k, knob};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    p.set(SPACE, 0.0);
    p.set(REPEATS, 0.0);
    p.set(ECHO, 0.0);
    for l in 0..LAYERS {
        p.set(knob(l, k::RELEASE), 0.3);
    }
    let mut rig = VesperRig::new(&p);
    let (l, _, midi) = rig.run(blocks(5.0), true, &[]);
    assert!(rms_of(&l[120_000..]) > 0.01);
    let ons = midi.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90 && m[0] & 0x0F < 2).count();
    assert!(ons >= 4, "chord and bass: {ons}");
    let (l, _, midi) = rig.run(blocks(15.0), false, &[]);
    let offs = midi.iter().filter(|(_, m)| m[0] & 0xF0 == 0x80 && m[0] & 0x0F < 2).count();
    assert_eq!(offs, 3 + 1, "the score's notes end when it stops");
    assert_eq!(peak_of(&l[640_000..]), 0.0, "and it falls silent, doing nothing");
    // And starts again.
    let (l, _, _) = rig.run(blocks(4.0), true, &[]);
    assert!(rms_of(&l[96_000..]) > 0.01);
}

#[test]
fn vesper_hold_lets_a_note_go_and_the_next_chord_strikes_again() {
    use crate::audio::modules::vesper::{DURATION, PAD, VARIATION, k, knob};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    vesper_solo(&p, PAD);
    p.set(DURATION, 1.0); // two seconds a chord
    p.set(VARIATION, 1.0); // always the same chord: no note changes
    p.set(knob(PAD, k::ATTACK), 0.02);
    p.set(knob(PAD, k::HOLD), 0.3);
    p.set(knob(PAD, k::RELEASE), 0.2);
    let mut rig = VesperRig::new(&p);
    let (l, _, _) = rig.run(VESPER_BAR_BLOCKS * 3, true, &[]);
    let bit = |bar: f32, secs: f32| {
        let at = (bar * VESPER_BAR as f32 + secs * 48_000.0) as usize;
        rms_of(&l[at..at + 4800])
    };
    for bar in [0.0, 1.0, 2.0] {
        assert!(bit(bar, 0.1) > 0.01, "a stab at the start of bar {bar}: {}", bit(bar, 0.1));
        assert!(bit(bar, 1.5) < bit(bar, 0.1) * 0.02, "gone before the next: {} after {}", bit(bar, 1.5), bit(bar, 0.1));
    }
}

#[test]
fn vesper_ostinato_plays_the_chord_in_time() {
    use crate::audio::modules::vesper::{COMPLEXITY, DURATION, PULSE, data, set_flag};
    let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
    set_flag(&p, data::OSTINATO, true);
    p.set(PULSE, 5.0); // eighths: 12 000 samples at 120 BPM
    p.set(DURATION, 4.0);
    p.set(COMPLEXITY, 3.0);
    let mut rig = VesperRig::new(&p);
    let (_, _, midi) = rig.run(VESPER_BAR_BLOCKS, true, &[]);
    let ons: Vec<(usize, u8)> = midi.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90 && m[0] & 0x0F == 2).map(|&(at, m)| (at, m[1])).collect();
    assert_eq!(ons.len(), 8, "eight eighths in a bar: {ons:?}");
    for (n, &(at, _)) in ons.iter().enumerate() {
        assert_eq!(at, n * 12_000, "on the grid: {ons:?}");
    }
    // Up over two octaves of the chord, then back down.
    let notes: Vec<i32> = ons.iter().map(|&(_, m)| m as i32).collect();
    assert!(notes[..6].windows(2).all(|w| w[0] < w[1]), "up: {notes:?}");
    assert!(notes[5..8].windows(2).all(|w| w[0] > w[1]), "and down: {notes:?}");
    assert_eq!(notes[3], notes[0] + 12);
    assert_eq!(notes[6], notes[4]);
    // Each has a note-off.
    let offs = midi.iter().filter(|(_, m)| m[0] & 0xF0 == 0x80 && m[0] & 0x0F == 2).count();
    assert_eq!(offs, 8);
}

#[test]
fn vesper_big_screen_sounds_have_their_shape() {
    use crate::audio::modules::vesper::library::render_table;
    use crate::audio::modules::vesper::sounds::recipes;
    use crate::audio::modules::vesper::{FACTORY, MIXTYPE, PAD, find_sound, load_factory};
    let amp = |buf: &[f32], hz: f32| tone_amp(buf, hz);
    // An organ: pure and steady, even and odd harmonics up high, a sub below.
    let organ = &recipes(PAD)[find_sound(PAD, "Temple Organ")];
    let t = render_table(organ, 48.0, 1 << 18, 48_000.0, 3);
    let f0 = crate::audio::midi::note_hz(t.base);
    let a1 = amp(&t.left, f0);
    assert!(amp(&t.left, f0 * 2.0) > 0.1 * a1, "the octave stop");
    assert!(amp(&t.left, f0 * 1.5) > 0.03 * a1, "the quint");
    assert!(amp(&t.left, f0 * 0.5) > 0.05 * a1, "the sixteen-foot");
    // Brass: saturated and heavy in the lows, with top end to bite.
    let brass = &recipes(PAD)[find_sound(PAD, "Low Brass Wall")];
    let t = render_table(brass, 48.0, 1 << 18, 48_000.0, 3);
    let crest = peak_of(&t.left) / rms_of(&t.left);
    assert!(crest < 3.2, "saturated: a crest of {crest}");
    // The brass and the organ turn slowly through i–VI–III–VII.
    let folded = FACTORY.iter().find(|s| s.name == "Folded City").unwrap();
    let rules = crate::audio::modules::vesper::harmony::Rules { root: 9, mood: folded.mood, variation: folded.variation as usize, complexity: folded.complexity as usize };
    let degrees = crate::audio::modules::vesper::harmony::progression(folded.seed as u32, folded.chords as usize, &rules).map(|s| s.degree);
    assert_eq!(degrees[..4], [0, 5, 2, 6]);
    // Each scene plays, loud enough and in range, in every mix it asks for.
    for (n, scene) in FACTORY.iter().enumerate().skip(20) {
        let p = Arc::new(ModuleParams::new(ModuleKind::Vesper));
        load_factory(&p, n);
        assert!(p.get(MIXTYPE) < 4.0);
        let mut rig = VesperRig::new(&p);
        let (l, r, _) = rig.run(blocks(8.0), true, &[]);
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() <= 1.0), "{}", scene.name);
        let rms = rms_of(&l[96_000..]);
        assert!(rms > 0.03, "{} is audible: {rms}", scene.name);
    }
}

#[test]
fn vesper_panel_draws_while_it_plays() {
    use crate::audio::modules::vesper::{EVOLVE, LAYERS, data, viz};
    let shared = Arc::new(EngineShared::new());
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Vesper, pos2(0.0, 0.0), &a);
    let mix = g.add_mixer(pos2(1000.0, 0.0), &a);
    g.connect(port(id, 0), port(mix, 0), &a);
    let p = module_params(&g, id);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for round in 0..10 {
        p.data[data::SELECTED].set((round % LAYERS) as f32);
        p.set_mode(if round % 2 == 1 { EVOLVE } else { 0 });
        p.set_toggle(round != 7);
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            render_buf(&mut e, 4_800);
            time += 0.1;
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[viz::CARDS].get() >= 2.0, "it showed its chords while drawn");
}

/// Records `seconds` of an oscillator through a mixer with the offline
/// engine: silence, `sound` seconds of tone, silence.
fn record_take(silence: f32, sound: f32) -> std::sync::Arc<crate::audio::record::Take> {
    use crate::audio::params::EngineShared;
    use crate::audio::record::Recorder;
    let shared = Arc::new(EngineShared::new());
    let (handle, mut engine) = crate::audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let x = g.add_osc(pos2(0.0, 0.0), &handle);
    let m = g.add_mixer(pos2(0.0, 0.0), &handle);
    let mut recorder = Recorder::new(shared.clone());
    recorder.start(48_000).unwrap();
    render(&mut engine, (silence * 48_000.0) as usize);
    g.connect(port(x, 0), port(m, 0), &handle);
    render(&mut engine, (sound * 48_000.0) as usize);
    g.disconnect(g.cables[0], &handle);
    render(&mut engine, (silence * 48_000.0) as usize);
    recorder.stop().unwrap()
}

#[test]
fn the_board_records_to_a_take() {
    let take = record_take(0.0, 1.0);
    assert_eq!(take.frames, 48_000);
    assert_eq!(take.dropped, 0);
    assert!(take.peak > 0.05 && take.peak <= 1.0, "peak {}", take.peak);
    assert_eq!(std::fs::metadata(&take.path).unwrap().len(), 48_000 * 8);
    let path = take.path.clone();
    drop(take);
    assert!(!path.exists(), "a dropped take cleans up after itself");
}

#[test]
fn takes_export_to_wav_and_aiff() {
    use crate::audio::export::{Format, Options, export};
    let take = record_take(0.25, 0.5);
    let dir = TempDir::new("export");
    let raw = std::fs::read(&take.path).unwrap();
    let sample = |frame: usize| f32::from_le_bytes(raw[frame * 8..frame * 8 + 4].try_into().unwrap());
    let all = Options { format: Format::Wav24, normalize: false, trim: false };

    // WAV, 24-bit: a plain header, then every frame.
    let wav = dir.0.join("t.wav");
    export(&take, all, &wav).unwrap();
    let bytes = std::fs::read(&wav).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2, "stereo");
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 48_000);
    assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 24);
    assert_eq!(bytes.len() as u64, 44 + take.frames * 6);
    let at = 44 + 18_000 * 6;
    let v = i32::from_le_bytes([0, bytes[at], bytes[at + 1], bytes[at + 2]]) >> 8;
    assert!((v as f32 / 8_388_607.0 - sample(18_000)).abs() < 1e-6, "the same samples");

    // 32-bit float.
    let float = dir.0.join("f.wav");
    export(&take, Options { format: Format::WavFloat, ..all }, &float).unwrap();
    let bytes = std::fs::read(&float).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 3, "IEEE float");
    assert_eq!(f32::from_le_bytes(bytes[44 + 18_000 * 8..44 + 18_000 * 8 + 4].try_into().unwrap()), sample(18_000));

    // AIFF, 16-bit: big-endian, the rate as an 80-bit float.
    let aiff = dir.0.join("t.aiff");
    export(&take, Options { format: Format::Aiff16, ..all }, &aiff).unwrap();
    let bytes = std::fs::read(&aiff).unwrap();
    assert_eq!(&bytes[..4], b"FORM");
    assert_eq!(&bytes[8..16], b"AIFFCOMM");
    assert_eq!(u32::from_be_bytes(bytes[22..26].try_into().unwrap()) as u64, take.frames);
    assert_eq!(&bytes[28..38], &[0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0], "48000 Hz");
    assert_eq!(bytes.len() as u64, 54 + take.frames * 4);

    // Trimmed: the silence goes (10 ms kept each side); normalized: the
    // peak at −1 dBFS.
    let trimmed = dir.0.join("trim.wav");
    export(&take, Options { format: Format::WavFloat, normalize: true, trim: true }, &trimmed).unwrap();
    let bytes = std::fs::read(&trimmed).unwrap();
    let frames = (bytes.len() - 44) / 8;
    assert!((24_000..=25_500).contains(&frames), "{frames} frames of sound");
    let peak = bytes[44..].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b).abs()).fold(0.0, f32::max);
    assert!((peak - 0.891).abs() < 0.002, "peak {peak}");
}

#[test]
fn takes_export_through_the_system_encoders() {
    use crate::audio::export::{Format, Options, export};
    if Format::Flac.missing().is_some() {
        return; // not on macOS
    }
    let take = record_take(0.0, 0.5);
    let dir = TempDir::new("encode");
    for (format, magic, at) in [(Format::Flac, &b"fLaC"[..], 0), (Format::Alac, &b"ftyp"[..], 4), (Format::Aac, &b"ftyp"[..], 4)] {
        let path = dir.0.join(format!("t.{}", format.extension()));
        export(&take, Options { format, normalize: false, trim: false }, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[at..at + 4], magic, "{}", format.label());
    }
}

#[test]
fn a_centred_channel_keeps_its_level_and_the_output_sets_the_board() {
    use crate::audio::dsp::balance;
    use crate::audio::params::EngineShared;
    // Balance: unity both sides in the centre; turning fades the far side.
    assert_eq!(balance(0.0), (1.0, 1.0));
    assert_eq!(balance(-1.0).0, 1.0);
    assert!(balance(-1.0).1 < 1e-6 && balance(1.0).0 < 1e-6);
    assert!((balance(0.5).0 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    // A saw at −6 dB through a fresh mixer reaches the device at −6 dB.
    let shared = Arc::new(EngineShared::new());
    let (a, mut e) = crate::audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let x = source(&mut g, &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    render(&mut e, 9600);
    let level = render(&mut e, 9600).0;
    assert!((level - 0.5).abs() < 0.03, "−6 dBFS through the mixer: {level}");
    // OUTPUT +3 dB: louder by as much, under the limiter's ceiling.
    shared.output.set(10f32.powf(3.0 / 20.0));
    render(&mut e, 9600);
    let louder = render(&mut e, 9600).0;
    assert!((louder / level - 1.413).abs() < 0.05, "{louder} vs {level}");
}

#[test]
fn tape_saturation_adds_harmonics_at_a_steady_level() {
    use crate::audio::modules::tape::{SATURATION, Tape};
    // A sine at −12 dBFS, the tape clean and then pushed.
    let run = |sat: f32| {
        let p = tape_params();
        p.set(SATURATION, sat);
        let mut t = Tape::new(p, 48_000.0);
        let (l, _) = drive_stereo(&mut t, 24_000, sine(1000.0, 0.25), None);
        let tail = &l[14_400..24_000];
        let f = tone_amp(tail, 1000.0);
        (f, (tone_amp(tail, 2000.0) + tone_amp(tail, 3000.0)) / f)
    };
    let (clean, clean_h) = run(0.0);
    let (default, default_h) = run(0.35);
    let (full, full_h) = run(1.0);
    assert!(default_h > 3.0 * clean_h && full_h > 3.0 * default_h, "harmonics grow: {clean_h} {default_h} {full_h}");
    assert!(default_h > 0.03, "characterful by default: {default_h}");
    for (level, what) in [(default, "default"), (full, "full")] {
        let db = 20.0 * (level / clean).log10();
        assert!((-3.5..1.0).contains(&db), "{what} keeps the level: {db:.1} dB");
    }
}

/// Grains only: layer 1 alone and fully wet (no dry signal), mutations off.
fn grains_only() -> Arc<ModuleParams> {
    use crate::audio::modules::frakture::{self, data, layer, lp};
    let p = frakture_with(&[frakture::GRAINS]);
    p.data[data::LAYER_ON + 1].set(0.0);
    p.set(lp(0, layer::DRY), 0.001);
    p.set(lp(0, layer::WET), 1.0);
    p
}

/// Steady white noise, the same every run.
fn noise(frame: usize) -> f32 {
    let h = (frame as u32).wrapping_mul(2_654_435_761) ^ 0x5bd1_e995;
    let h = h.wrapping_mul(0x2c1b_3c6d) ^ (h >> 15);
    0.3 * ((h >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
}

#[test]
fn frakture_mutations_reshape_the_grains() {
    use crate::audio::modules::frakture::{self, Frakture, GRAIN_VIZ, VIZ_GRAINS, grain_kind, viz};
    // FEEDBACK: after the input stops, the grains keep regenerating.
    let tail = |feedback: f32| {
        let p = grains_only();
        p.set(frakture::FEEDBACK, feedback);
        let mut fx = Frakture::new(p, 48_000.0);
        drive(&mut fx, 96_000, 0, saw);
        let (after, _) = drive(&mut fx, 192_000, 96_000, |_| 0.0);
        rms_of(&after[144_000..])
    };
    let (dry_tail, fed_tail) = (tail(0.0), tail(0.9));
    assert!(fed_tail > 0.01 && fed_tail > 10.0 * dry_tail, "feedback tail {fed_tail} vs {dry_tail}");

    // SWARM: clouds, many more grains at once.
    let grains = |swarm: f32| {
        let p = grains_only();
        p.set(frakture::SWARM, swarm);
        let mut fx = Frakture::new(p.clone(), 48_000.0);
        drive(&mut fx, 96_000, 0, saw);
        p.viz[viz::GRAIN_COUNT].get()
    };
    let (alone, cloud) = (grains(0.0), grains(1.0));
    assert!(cloud >= 3.0 * alone.max(1.0), "{cloud} grains in a swarm, {alone} without");

    // GLIDE and SHIMMER: grains end up away from their starting pitch, and
    // some jump up; ERODE marks some as eroded.
    let p = grains_only();
    p.set(frakture::GLIDE, 1.0);
    p.set(frakture::SHIMMER, 1.0);
    p.set(frakture::ERODE, 1.0);
    p.set(frakture::SWARM, 1.0);
    let mut fx = Frakture::new(p.clone(), 48_000.0);
    drive(&mut fx, 96_000, 0, saw);
    let shown: Vec<(f32, f32, u32)> = (0..VIZ_GRAINS)
        .map(|g| viz::GRAINS + g * GRAIN_VIZ)
        .filter(|&at| p.viz[at + 2].get() >= 0.0)
        .map(|at| (p.viz[at + 1].get(), p.viz[at + 4].get(), p.viz[at + 3].get() as u32))
        .collect();
    assert!(shown.iter().any(|(now, start, _)| now - start > 3.0), "no grain glided: {shown:?}");
    assert!(shown.iter().all(|(_, _, kind)| kind & grain_kind::SHIMMER != 0), "shimmer at full jumps every grain");
    assert!(shown.iter().all(|(_, _, kind)| kind & grain_kind::ERODED != 0), "erode at full wears every grain");

    // RESONATE: noise comes out pitched, on the harmonics of KEY.
    let peaks = |resonate: f32| {
        let p = grains_only();
        p.set(frakture::RESONATE, resonate);
        p.set(frakture::KEY, 220.0);
        let mut fx = Frakture::new(p, 48_000.0);
        let (l, _) = drive(&mut fx, 144_000, 0, noise);
        let tail = &l[48_000..];
        let on: f32 = [220.0, 440.0, 660.0, 880.0].iter().map(|&f| tone_amp(tail, f)).sum();
        let off: f32 = [330.0, 550.0, 770.0, 990.0].iter().map(|&f| tone_amp(tail, f)).sum();
        on / off.max(1e-9)
    };
    let (flat, rung) = (peaks(0.0), peaks(1.0));
    assert!(rung > 3.0 * flat, "harmonics of KEY stand out: {rung:.2} vs {flat:.2}");
}

#[test]
fn frakture_mutated_to_the_extreme_stays_in_bounds() {
    use crate::audio::modules::frakture::{self, Frakture};
    let p = Arc::new(ModuleParams::new(ModuleKind::Frakture));
    for (i, v) in [
        (frakture::FEEDBACK, 1.0),
        (frakture::SWARM, 1.0),
        (frakture::GLIDE, -1.0),
        (frakture::SHIMMER, 1.0),
        (frakture::RESONATE, 1.0),
        (frakture::BLUR, 1.0),
        (frakture::ERODE, 1.0),
    ] {
        p.set(i, v);
    }
    let (l, r) = drive(&mut Frakture::new(p, 48_000.0), 192_000, 0, saw);
    assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(rms_of(&l[96_000..]) > 0.01, "still sounding");
}

#[test]
fn frakture_scan_sync_is_locked_to_the_transport() {
    use crate::audio::modules::frakture::{self, Frakture, SCAN, SCAN_RANGE, data, div_norm, layer, lp, viz};
    // A 1-bar buffer (4 beats, 96000 frames at 120 BPM): at 1× the playhead
    // makes one pass per bar, at −½× half a pass backwards, wherever the
    // transport starts.
    for (ratio, at) in [(6, 1.0), (2, -0.5)] {
        for from in [0, 30_000] {
            let p = grains_only();
            p.data[data::PLAYHEAD_MODE].set(SCAN as f32);
            p.data[data::SCAN_SYNC].set(1.0);
            p.set(lp(0, layer::PLAYHEAD), div_norm(ratio, SCAN_RANGE));
            let mut fx = Frakture::new(p.clone(), 48_000.0);
            // The display is published every 4th block: end on one.
            for frames in [6_464, 64_064] {
                drive(&mut fx, frames, from, saw);
                let beat = (from + frames - 1) as f64 / 24_000.0;
                let want = (beat / 4.0 * at).rem_euclid(1.0) as f32;
                let got = p.viz[viz::PLAYHEADS].get();
                let off = (got - want).abs();
                assert!(off.min(1.0 - off) < 0.002, "{at}× from {from}: playhead {got}, want {want}");
                fx = Frakture::new(p.clone(), 48_000.0);
            }
        }
    }
    let p = grains_only();
    p.data[data::PLAYHEAD_MODE].set(SCAN as f32);
    p.data[data::SCAN_SYNC].set(1.0);
    assert_eq!(frakture::format(&p, lp(0, layer::PLAYHEAD), div_norm(6, SCAN_RANGE)), "1×");
}

#[test]
fn frakture_synced_swarm_rolls_in_time() {
    use crate::audio::modules::frakture::{self, DENSITY_DIVS, Frakture, SIZE_DIVS, data, div_norm, layer, lp, viz};
    // A cloud every half note, grains a quarter long; with SYNC its grains
    // start one after another, a 1/32 (3000 frames) apart.
    let run = |sync: bool| {
        let p = grains_only();
        p.set(frakture::SWARM, 1.0);
        p.data[data::SWARM_SYNC].set(if sync { 1.0 } else { 0.0 });
        p.set(frakture::SWARM_RATE, 2.0);
        p.set(lp(0, layer::DENSITY), div_norm(13, DENSITY_DIVS));
        p.data[data::SIZE_SYNC].set(1.0);
        p.set(lp(0, layer::SIZE), div_norm(10, SIZE_DIVS));
        let mut fx = Frakture::new(p.clone(), 48_000.0);
        let (mut last, mut most, mut starts) = (0.0, 0.0f32, Vec::new());
        for block in 0..4 * 48_000 / 64 {
            drive(&mut fx, 64, block * 64, saw);
            // Published every 4th block: 256 frames at a time.
            if block % 4 != 0 {
                continue;
            }
            let count = p.viz[viz::GRAIN_COUNT].get();
            if count > last {
                starts.push((block, count - last));
            }
            (last, most) = (count, most.max(count));
        }
        (starts, most)
    };
    let (starts, most) = run(true);
    assert!(most >= 3.0, "still a cloud: {most} grains at once");
    for &(block, added) in &starts {
        assert_eq!(added, 1.0, "one grain at a time, block {block}");
        let into = (block * 64 + 63) % 3_000;
        assert!(into < 256 + 2, "block {block} isn't on the 1/32 grid ({into} frames past it)");
    }
    let (free, _) = run(false);
    assert!(free.iter().any(|&(_, added)| added > 1.0), "unsynced, a cloud starts all at once");
}

#[test]
fn frakture_synced_glide_bends_in_a_note_value() {
    use crate::audio::modules::frakture::{self, Frakture, GRAIN_VIZ, SIZE_DIVS, VIZ_GRAINS, data, div_norm, layer, lp, viz};
    // Whole-note grains, a new one every 1/16, each bending 12…36 st: synced
    // to 1/16, nearly all of them are past 12 st; free, the bend lasts the
    // whole grain and fewer than half are.
    let bent = |sync: bool| {
        let p = grains_only();
        p.set(frakture::GLIDE, 1.0);
        p.data[data::GLIDE_SYNC].set(if sync { 1.0 } else { 0.0 });
        p.set(frakture::GLIDE_TIME, 4.0);
        p.data[data::SIZE_SYNC].set(1.0);
        p.set(lp(0, layer::SIZE), div_norm(15, SIZE_DIVS));
        let mut fx = Frakture::new(p.clone(), 48_000.0);
        let (mut done, mut all) = (0, 0);
        for k in 0..12 {
            drive(&mut fx, 9_600, 96_000 + k * 9_600, saw);
            if k < 2 {
                continue;
            }
            for g in 0..VIZ_GRAINS {
                let at = viz::GRAINS + g * GRAIN_VIZ;
                if p.viz[at + 2].get() >= 0.0 {
                    all += 1;
                    done += ((p.viz[at + 1].get() - p.viz[at + 4].get()).abs() >= 11.9) as usize;
                }
            }
        }
        done as f32 / all.max(1) as f32
    };
    let (synced, free) = (bent(true), bent(false));
    assert!(synced > 0.85 && free < 0.65, "bent past 12 st: {synced:.2} synced, {free:.2} free");
}

#[test]
fn frakture_synced_feedback_echoes_a_note_value_later() {
    use crate::audio::modules::frakture::{self, Frakture, layer, lp};
    // The default layer 1 plays a quarter note (24000 frames) behind, here
    // without scatter: a short burst comes back at 24000, then fed back
    // straight away at 48000, 72000… or, a half note later, at 96000.
    let tail = |sync: bool| {
        let p = grains_only();
        p.set(lp(0, layer::RANGE), 0.0);
        p.set(lp(0, layer::CHAOS), 0.0);
        p.set(frakture::FEEDBACK, 0.9);
        p.data[frakture::data::FEEDBACK_SYNC].set(if sync { 1.0 } else { 0.0 });
        p.set(frakture::FEEDBACK_TIME, 13.0);
        let (l, _) = drive(&mut Frakture::new(p, 48_000.0), 120_000, 0, |f| if f < 4_800 { saw(f) } else { 0.0 });
        let w = |from: usize| rms_of(&l[from..from + 8_000]);
        (w(24_500), w(48_500), w(72_500), w(96_500))
    };
    let (first, early, later, late) = tail(true);
    assert!(first > 0.01 && late > 0.1 * first, "the echo comes back: {first} then {late}");
    assert!(early < 0.05 * late && later < 0.05 * late, "nothing in between: {early}, {later} vs {late}");
    let (_, free, _, _) = tail(false);
    assert!(free > 20.0 * early, "unsynced it's straight back: {free} vs {early}");
}

#[test]
fn frakture_synced_space_shows_note_values() {
    use crate::audio::modules::frakture::{self, SPACE_DIVS, data, div_norm};
    let p = Arc::new(ModuleParams::new(ModuleKind::Frakture));
    let spec = &frakture::DEF.params[frakture::SPACE_LENGTH];
    let v = spec.denorm(div_norm(13, SPACE_DIVS));
    p.data[data::SPACE_SYNC].set(1.0);
    assert_eq!(frakture::format(&p, frakture::SPACE_LENGTH, v), "1/2");
    p.data[data::SPACE_SYNC].set(0.0);
    assert!(frakture::format(&p, frakture::SPACE_LENGTH, v).ends_with('s'));
    for (i, label) in [(frakture::SWARM_RATE, "1/32"), (frakture::GLIDE_TIME, "1/4"), (frakture::FEEDBACK_TIME, "1/4")] {
        assert_eq!(frakture::format(&p, i, spec_default(i)), label);
    }
}

fn spec_default(i: usize) -> f32 {
    crate::audio::modules::frakture::DEF.params[i].default
}

// ---------------------------------------------------------------------------
// Audio input and devices
// ---------------------------------------------------------------------------

/// A 4-channel input whose samples say where they come from: channel c of
/// frame f is c + f / 100000.
fn input_sample(frame: usize, channel: usize) -> f32 {
    channel as f32 + frame as f32 / 100_000.0
}

/// Pushes `frames` of the 4-channel test input from `from`, as a device
/// callback would.
fn push_input(shared: &EngineShared, from: usize, frames: usize, channels: usize) {
    let data: Vec<f32> = (from..from + frames).flat_map(|f| (0..channels).map(move |c| input_sample(f, c))).collect();
    shared.input.push(&data, channels);
}

/// An Audio In node's processor on `shared`'s input, with INPUT and mode.
fn audio_in(shared: &Arc<EngineShared>, input: f32, mono: bool) -> crate::audio::modules::audio_in::AudioIn {
    use crate::audio::modules::audio_in::{self, AudioIn};
    let p = Arc::new(ModuleParams::new(ModuleKind::AudioIn));
    p.set(audio_in::INPUT, input);
    p.set_mode(if mono { audio_in::MONO } else { 0 });
    AudioIn::new(p, shared.clone(), 48_000.0)
}

/// One output buffer of `frames`: the engine takes the input, the node
/// plays it block by block.
fn play_input(shared: &EngineShared, node: &mut dyn crate::audio::engine::Processor, frames: usize) -> (Vec<f32>, Vec<f32>) {
    use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx};
    let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
    let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    shared.input.begin_render(frames);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for _ in 0..frames / 64 {
        node.process(&ctx, &inputs, &mut outputs, 64);
        l.extend_from_slice(&outputs[0][0]);
        r.extend_from_slice(&outputs[0][1]);
    }
    (l, r)
}

#[test]
fn audio_in_plays_the_chosen_input_channels_a_buffer_late() {
    let shared = Arc::new(EngineShared::new());
    shared.input.open(4);
    // INPUT 3, stereo: channels 3 and 4 (2 and 3 counting from 0).
    let mut node = audio_in(&shared, 3.0, false);
    // The engine notices the new input (and drops anything from before).
    play_input(&shared, &mut node, 256);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for k in 0..12 {
        push_input(&shared, k * 256, 256, 4);
        let (a, b) = play_input(&shared, &mut node, 256);
        l.extend(a);
        r.extend(b);
    }
    // The first buffer waits for the cushion (one device buffer), then
    // every frame comes through, 256 frames late, nothing lost.
    assert!(l[..256].iter().chain(&r[..256]).all(|&s| s == 0.0), "silent while the cushion fills");
    for f in 256..l.len() {
        assert_eq!((l[f], r[f]), (input_sample(f - 256, 2), input_sample(f - 256, 3)), "frame {f}");
    }
    assert_eq!(shared.input.dropouts.load(Ordering::Relaxed), 0);

    // MONO: INPUT 2 on both sides.
    let mut node = audio_in(&shared, 2.0, true);
    push_input(&shared, 12 * 256, 256, 4);
    let (l, r) = play_input(&shared, &mut node, 256);
    assert_eq!((l[10], r[10]), (input_sample(11 * 256 + 10, 1), input_sample(11 * 256 + 10, 1)));
}

#[test]
fn audio_in_copes_with_mono_inputs_dropouts_and_drift() {
    // A one-channel input (a laptop's microphone): STEREO puts it on both
    // sides; a channel it doesn't have is silence.
    let shared = Arc::new(EngineShared::new());
    shared.input.open(1);
    let mut node = audio_in(&shared, 1.0, false);
    let mut missing = audio_in(&shared, 2.0, true);
    for k in 0..3 {
        push_input(&shared, k * 128, 128, 1);
        let (l, r) = play_input(&shared, &mut node, 128);
        assert_eq!(l, r);
    }
    let (l, _) = play_input(&shared, &mut missing, 128);
    assert!(l.iter().all(|&s| s == 0.0), "no channel 2 on a mono input");

    // The input stops delivering: one dropout, then silence until the
    // cushion is back.
    let shared = Arc::new(EngineShared::new());
    shared.input.open(2);
    let mut node = audio_in(&shared, 1.0, false);
    for k in 0..4 {
        push_input(&shared, k * 256, 256, 2);
        play_input(&shared, &mut node, 256);
    }
    play_input(&shared, &mut node, 256); // the cushion
    let (l, _) = play_input(&shared, &mut node, 256);
    assert!(l.iter().all(|&s| s == 0.0));
    assert_eq!(shared.input.dropouts.load(Ordering::Relaxed), 1);
    push_input(&shared, 4 * 256, 256, 2);
    let (l, _) = play_input(&shared, &mut node, 256);
    assert!(l.iter().all(|&s| s == 0.0), "waits for a whole cushion");
    push_input(&shared, 5 * 256, 256, 2);
    let (l, _) = play_input(&shared, &mut node, 256);
    assert_eq!(l[0], input_sample(4 * 256, 0), "then plays on from where it stopped");

    // An input whose clock runs fast piles up frames: some are skipped, so
    // the delay stays short.
    let shared = Arc::new(EngineShared::new());
    shared.input.open(2);
    let mut node = audio_in(&shared, 1.0, false);
    let mut pushed = 0;
    let mut last = Vec::new();
    for _ in 0..200 {
        push_input(&shared, pushed, 300, 2);
        pushed += 300;
        last = play_input(&shared, &mut node, 256).0;
    }
    assert!(shared.input.skips.load(Ordering::Relaxed) > 0);
    let behind = pushed as f32 - (last[255] * 100_000.0).round();
    assert!(behind <= (2 * 256 + 4 * 300) as f32, "{behind} frames behind");

    // Closed: silence, and the node says so.
    shared.input.close();
    let (l, _) = play_input(&shared, &mut node, 256);
    assert!(l.iter().all(|&s| s == 0.0));
}

#[test]
fn audio_in_node_reaches_the_board() {
    let shared = Arc::new(EngineShared::new());
    let (a, mut e) = audio::offline(shared.clone(), 48_000);
    let mut g = Graph::default();
    let input = g.add_module(ModuleKind::AudioIn, pos2(0.0, 0.0), &a);
    let mixer = g.add_mixer(pos2(300.0, 0.0), &a);
    g.connect(port(input, 0), port(mixer, 0), &a);
    shared.input.open(2);
    let tone = |f: usize| 0.3 * (f as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin();
    let mut out = Vec::new();
    for k in 0..40 {
        let data: Vec<f32> = (k * 600..(k + 1) * 600).flat_map(|f| [tone(f), tone(f)]).collect();
        shared.input.push(&data, 2);
        out.extend(render_buf(&mut e, 600));
    }
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).skip(4800).collect();
    assert!(tone_amp(&left, 440.0) > 0.05, "the input is heard: {}", tone_amp(&left, 440.0));
    let p = module_params(&g, input);
    assert_eq!(p.viz[crate::audio::modules::audio_in::viz::CHANNELS].get(), 2.0);
}

#[test]
fn the_board_plays_on_the_chosen_output_pair() {
    let (mut g, a, mut e) = setup();
    let osc = g.add_osc(pos2(0.0, 0.0), &a);
    let mixer = g.add_mixer(pos2(300.0, 0.0), &a);
    g.connect(port(osc, 0), port(mixer, 0), &a);
    let mut buf = vec![0.0f32; 4 * 4800];
    for chunk in buf.chunks_mut(4 * 300) {
        e.render_at(chunk, 4, 2);
    }
    let channel = |c: usize| buf.chunks(4).map(|f| f[c].abs()).fold(0.0f32, f32::max);
    assert_eq!((channel(0), channel(1)), (0.0, 0.0), "1–2 silent");
    assert!(channel(2) > 0.01 && channel(3) > 0.01, "3–4 playing");
    // A pair past the last channels: the last two.
    let mut buf = vec![0.0f32; 2 * 600];
    e.render_at(&mut buf, 2, 4);
    assert!(buf.iter().any(|s| s.abs() > 0.01));
}

#[test]
fn audio_settings_survive_and_older_files_still_load() {
    use crate::audio::device::{self, AudioConfig, InputChoice};
    use crate::ui::app::AppSettings;
    // Before the audio settings there was only the buffer size.
    let old = AppSettings::parse(r#"{ "buffer_size": 512, "output_db": 3.0 }"#).unwrap();
    assert_eq!((old.audio.buffer_size, old.output_db), (Some(512), 3.0));
    assert_eq!(old.audio.input, InputChoice::SystemDefault, "the input follows the system by default");

    let mut settings = AppSettings::default();
    settings.audio = AudioConfig {
        host: Some("CoreAudio".into()),
        output: Some("coreaudio:BuiltInSpeakerDevice".into()),
        output_channel: 2,
        input: InputChoice::Device("coreaudio:Scarlett".into()),
        sample_rate: Some(96_000),
        buffer_size: Some(128),
    };
    let text = serde_json::to_string(&settings).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(json.get("buffer_size").is_none(), "the old field isn't written again");
    assert_eq!(AppSettings::parse(&text).unwrap(), settings);

    assert_eq!(device::pairs(2), [(0, "1–2".to_owned())]);
    assert_eq!(device::pairs(5).iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), ["1–2", "3–4", "5"]);
    assert_eq!(device::pairs(1), [(0, "1".to_owned())]);
}

#[test]
fn audio_in_feeds_effects_and_texture_nodes() {
    // The interface's input through each processing node into a mixer: it
    // comes out, with the transport playing or stopped (a live input is not
    // a sequenced source).
    let kinds = [
        ModuleKind::Filter,
        ModuleKind::Magma,
        ModuleKind::Equalizer,
        ModuleKind::Tape,
        ModuleKind::Delay,
        ModuleKind::PingPong,
        ModuleKind::Reverb,
        ModuleKind::Frakture,
        ModuleKind::Granular,
        ModuleKind::Resonator,
    ];
    for playing in [true, false] {
        for kind in kinds {
            let shared = Arc::new(EngineShared::new());
            shared.running.store(playing, Ordering::Relaxed);
            let (a, mut e) = audio::offline(shared.clone(), 48_000);
            let mut g = Graph::default();
            let input = g.add_module(ModuleKind::AudioIn, pos2(0.0, 0.0), &a);
            let fx = g.add_module(kind, pos2(300.0, 0.0), &a);
            let mixer = g.add_mixer(pos2(600.0, 0.0), &a);
            g.connect(port(input, 0), port(fx, 0), &a);
            g.connect(port(fx, 0), port(mixer, 0), &a);
            shared.input.open(2);
            let tone = |f: usize| 0.3 * (f as f32 * std::f32::consts::TAU * 220.0 / 48_000.0).sin();
            let mut out = Vec::new();
            for k in 0..80 {
                let data: Vec<f32> = (k * 600..(k + 1) * 600).flat_map(|f| [tone(f), tone(f)]).collect();
                shared.input.push(&data, 2);
                out.extend(render_buf(&mut e, 600));
            }
            let level = rms_of(&out[out.len() / 2..]);
            assert!(level > 0.01, "{kind:?} (playing {playing}) passes the input on: {level}");
        }
    }
}

// ---------------------------------------------------------------------------
// Automa
// ---------------------------------------------------------------------------

/// Runs Automa from frame `from` of a 120 BPM grid for `frames`, returning
/// its events at absolute frames. `restart`: the transport was just
/// rewound (Start).
fn run_automa(
    a: &mut crate::audio::modules::automa::Automa,
    from: usize,
    frames: usize,
    playing: bool,
    restart: bool,
) -> Vec<(usize, [u8; 3])> {
    const SR: f64 = 48_000.0;
    let bps = 2.0 / SR;
    let inputs = Box::new(Inputs {
        bufs: [[[0.0; 64]; 2]; MAX_INPUTS],
        connected: [false; MAX_INPUTS],
        midi: MidiBuf::default(),
        midi_connected: false,
    });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let mut events = Vec::new();
    for done in (0..frames).step_by(64) {
        let at = from + done;
        let ctx = ProcessCtx {
            sample_rate: SR as f32,
            smooth: 0.01,
            beat: at as f64 * bps,
            beats_per_sample: bps,
            playing,
            restart: restart && done == 0,
        };
        a.process(&ctx, &inputs, &mut outputs, 64);
        for e in a.midi_out().expect("sends MIDI").events() {
            events.push((at + e.frame as usize, e.data));
        }
    }
    events
}

/// Every note-on matched by a note-off.
fn every_note_ends(events: &[(usize, [u8; 3])]) -> bool {
    let mut held = std::collections::HashMap::<(u8, u8), i32>::new();
    for (_, m) in events {
        let key = (m[0] & 0x0F, m[1]);
        match m[0] & 0xF0 {
            0x90 if m[2] > 0 => *held.entry(key).or_default() += 1,
            0x80 | 0x90 => *held.entry(key).or_default() -= 1,
            _ => {}
        }
        if held.get(&key).is_some_and(|&n| !(0..=1).contains(&n)) {
            return false;
        }
    }
    held.values().all(|&n| n == 0)
}

const BAR: usize = 96_000;
const SIXTEENTH: usize = 6_000;

#[test]
fn automa_melodies_stay_in_key_on_the_chords_and_in_time() {
    use crate::audio::modules::automa::{self, Automa, data};
    let scale = [0, 2, 4, 5, 7, 9, 11];
    for engine in 0..4 {
        for (tension, voices) in [(0.0, 1.0), (0.4, 1.0), (1.0, 3.0)] {
            let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
            p.set_mode(engine);
            p.set(automa::TENSION, tension);
            p.set(automa::VOICES, voices);
            p.data[data::PAD].set(1.0);
            let mut a = Automa::new(p.clone());
            let events = run_automa(&mut a, 0, 8 * BAR, true, true);
            let (h, prog) = (automa::harmony(&p), automa::progression(&p));
            let chord_at = |frame: usize| h.chord(prog.degree_at(frame as f64 / 24_000.0));
            let ons = note_ons(&events);
            // The melody from middle C up (OCTAVE 4), the pad under it.
            let (melody, pad): (Vec<_>, Vec<_>) = ons.iter().partition(|o| o.1 >= 60);
            assert!(melody.len() > 40, "engine {engine} plays: {} notes", melody.len());
            for &(frame, note) in &melody {
                assert_eq!(frame % SIXTEENTH, 0, "on the sixteenths of the global grid");
                assert!(scale.contains(&(note % 12)), "engine {engine}: {note} is in C major");
                // Every chord begins on one of its tones; with no TENSION
                // there is nothing else.
                if frame % BAR == 0 || tension == 0.0 {
                    let chord = chord_at(frame);
                    assert!(chord.contains(note % 12), "engine {engine}: {note} at frame {frame} is a tone of {}", chord.name(&h));
                }
            }
            if voices > 1.0 {
                let chords = melody.iter().filter(|a| melody.iter().filter(|b| b.0 == a.0).count() > 1).count();
                assert!(chords > 0, "engine {engine} plays in parts");
            }
            // The pad: the chord's three tones on every bar line.
            assert_eq!(pad.len(), 8 * 3, "engine {engine}");
            for &(frame, note) in &pad {
                assert_eq!(frame % BAR, 0);
                assert!(chord_at(frame).contains(note % 12));
            }
        }
    }
}

#[test]
fn automa_plays_the_same_from_start_and_stop_ends_its_notes() {
    use crate::audio::modules::automa::{self, Automa, data};
    for engine in 0..4 {
        let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
        p.set_mode(engine);
        p.set(automa::VOICES, 3.0);
        p.set(automa::GATE, 1.0);
        p.data[data::PAD].set(1.0);
        let mut a = Automa::new(p.clone());
        let mut first = run_automa(&mut a, 0, 4 * BAR, true, true);
        let stop = run_automa(&mut a, 4 * BAR, 64, false, false);
        assert!(!stop.is_empty() && stop.iter().all(|(_, m)| m[0] & 0xF0 == 0x80), "Stop only ends notes");
        first.extend(&stop);
        assert!(every_note_ends(&first), "engine {engine}: no note hangs");
        assert_eq!(p.viz[automa::viz::STEP].get(), -1.0);
        let again = run_automa(&mut a, 0, 4 * BAR, true, true);
        assert_eq!(note_ons(&first), note_ons(&again), "engine {engine}: Start plays the same music again");

        // A new seed, other music.
        automa::reseed(&p, 777);
        let other = run_automa(&mut a, 0, 4 * BAR, true, true);
        assert_ne!(note_ons(&other), note_ons(&again), "engine {engine}");
    }
}

#[test]
fn automa_follows_rate_swing_and_the_chord_length() {
    use crate::audio::modules::automa::{self, Automa, data};
    let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
    p.set_mode(automa::CASCADE);
    p.set(automa::DENSITY, 1.0);
    p.set(automa::RATE, 1.0); // eighths
    p.set(automa::CHANGE, 2.0); // two bars a chord
    p.data[data::PAD].set(1.0);
    let ons = note_ons(&run_automa(&mut Automa::new(p.clone()), 0, 4 * BAR, true, true));
    let melody: Vec<usize> = ons.iter().filter(|o| o.1 >= 60).map(|o| o.0).collect();
    assert_eq!(melody, (0..32).map(|k| k * 12_000).collect::<Vec<_>>(), "every eighth (CASCADE always offers)");
    let pad: Vec<usize> = ons.iter().filter(|o| o.1 < 60).map(|o| o.0).collect();
    assert_eq!(pad, [0, 0, 0, 2 * BAR, 2 * BAR, 2 * BAR]);
    // Swing pushes the off-beats late.
    p.set(automa::SWING, 1.0);
    let ons = note_ons(&run_automa(&mut Automa::new(p.clone()), 0, BAR / 2, true, true));
    let firsts: Vec<usize> = ons.iter().filter(|o| o.1 >= 60).map(|o| o.0).collect();
    assert_eq!(firsts, [0, 18_000, 24_000, 42_000]);
}

#[test]
fn automa_freeze_loops_the_last_bar() {
    use crate::audio::modules::automa::{self, Automa, data, viz};
    // TREE over one chord: the frozen bar is the last one, again and again.
    let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
    p.set_mode(automa::TREE);
    p.set(automa::MUTATE, 0.0);
    automa::set_progression(&p, &[0]);
    let mut a = Automa::new(p.clone());
    let free = note_ons(&run_automa(&mut a, 0, 2 * BAR, true, true));
    p.data[data::FREEZE].set(1.0);
    let frozen = note_ons(&run_automa(&mut a, 2 * BAR, 3 * BAR, true, false));
    let bar = |ons: &[(usize, u8)], k: usize| ons.iter().filter(|o| o.0 / BAR == k).map(|o| (o.0 % BAR, o.1)).collect::<Vec<_>>();
    assert!(!bar(&free, 1).is_empty());
    for k in 2..5 {
        assert_eq!(bar(&frozen, k), bar(&free, 1), "frozen bar {k}");
    }
    // Let go, it carries on through the series.
    p.data[data::FREEZE].set(0.0);
    let after = note_ons(&run_automa(&mut a, 5 * BAR, BAR, true, false));
    assert_ne!(bar(&after, 5), bar(&free, 1));

    // Frozen, LIFE stops evolving (and keeps playing its grid).
    let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
    p.set(automa::LIFE_GEN, 1.0);
    let mut a = Automa::new(p.clone());
    run_automa(&mut a, 0, BAR, true, true);
    let generation = p.viz[viz::GENERATION].get();
    assert!(generation > 10.0);
    p.data[data::FREEZE].set(1.0);
    let ons = note_ons(&run_automa(&mut a, BAR, 2 * BAR, true, false));
    assert!(p.viz[viz::GENERATION].get() <= generation + 1.0, "it held still");
    assert!(!ons.is_empty());
}

#[test]
fn automa_plays_cells_drawn_on_its_grid() {
    use crate::audio::modules::automa::{self, Automa, data, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
    automa::set_pattern(&p, &[0; 16]);
    p.set(automa::MUTATE, 0.0);
    p.set(automa::DENSITY, 1.0);
    let mut a = Automa::new(p.clone());
    run_automa(&mut a, 0, 64, false, false);
    assert!((0..256).all(|i| p.viz[viz::CELLS + i].get() == 0.0), "an empty world");
    // A blinker drawn on row 5, columns 4–6.
    for x in 4..7 {
        p.viz[viz::EDITS + 5 * 16 + x].set(1.0);
    }
    p.viz[viz::EDITED].set(1.0);
    run_automa(&mut a, 0, 64, false, false);
    let alive: Vec<usize> = (0..256).filter(|&i| p.viz[viz::CELLS + i].get() > 0.0).collect();
    assert_eq!(alive, [84, 85, 86]);
    // Playing (frozen, so it stays put), exactly those columns sound.
    p.data[data::FREEZE].set(1.0);
    let ons = note_ons(&run_automa(&mut a, 0, BAR, true, false));
    assert_eq!(ons.iter().map(|o| o.0).collect::<Vec<_>>(), [4 * SIXTEENTH, 5 * SIXTEENTH, 6 * SIXTEENTH]);
    // Rubbed out.
    p.viz[viz::EDITS + 5 * 16 + 5].set(-1.0);
    p.viz[viz::EDITED].set(1.0);
    run_automa(&mut a, BAR, 64, true, false);
    assert_eq!(p.viz[viz::CELLS + 85].get(), 0.0);
}

#[test]
fn automa_survives_every_setting() {
    use crate::audio::modules::automa::{Automa, data};
    let def = ModuleKind::Automa.def();
    for setting in [0.0, 0.5, 1.0] {
        for mode in 0..4 {
            for flags in [0.0, 1.0] {
                let p = Arc::new(ModuleParams::new(ModuleKind::Automa));
                for (i, spec) in def.params.iter().enumerate() {
                    p.set(i, spec.denorm(setting));
                }
                p.set_mode(mode);
                for i in [data::FREEZE, data::SEVENTHS, data::PAD, data::WANDER] {
                    p.data[i].set(flags);
                }
                p.data[data::CHORDS].set(setting * 8.0);
                let mut a = Automa::new(p.clone());
                let mut events = run_automa(&mut a, 0, 2 * BAR, true, true);
                events.extend(run_automa(&mut a, 2 * BAR, 64, false, false));
                assert!(every_note_ends(&events), "setting {setting} mode {mode} flags {flags}");
                assert!(events.iter().all(|(_, m)| m[1] < 128 && m[2] < 128));
            }
        }
    }
}

#[test]
fn automa_settings_progression_and_pattern_are_saved() {
    use crate::audio::modules::automa::{self, data, engines};
    let (mut g, a, _e) = setup();
    let id = g.add_module(ModuleKind::Automa, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    p.set_mode(automa::JULIA);
    p.set(automa::SHAPE, 0.6);
    automa::set_progression(&p, &[5, 3, 0, 4, 1]);
    automa::reseed(&p, 1234);
    p.data[data::SEVENTHS].set(1.0);
    let saved = crate::project::snapshot(&g);

    let (mut h, b, _f) = setup();
    assert_eq!(crate::project::restore(&mut h, &saved, &b), 0);
    let q = module_params(&h, h.nodes[0].id);
    assert_eq!(q.mode(), automa::JULIA);
    assert_eq!(q.get(automa::SHAPE), 0.6);
    let prog = automa::progression(&q);
    assert_eq!((prog.len, &prog.degrees[..5]), (5, &[5, 3, 0, 4, 1][..]));
    assert_eq!(automa::seed(&q), 1234);
    assert_eq!(automa::pattern(&q), engines::soup(1234));
    assert!(automa::harmony(&q).sevenths);
    assert_eq!(crate::project::snapshot(&h), saved);
}

#[test]
fn automa_panel_draws_every_generator() {
    use crate::audio::modules::automa::data;
    let shared = Arc::new(EngineShared::new());
    shared.running.store(true, Ordering::Relaxed);
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Automa, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    p.data[data::PAD].set(1.0);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for engine in 0..4 {
        p.set_mode(engine);
        for _ in 0..4 {
            render_buf(&mut e, 4_800);
            time += 0.1;
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[crate::audio::modules::automa::viz::EVENT_SEQ].get() > 0.0, "it played while drawn");
}

// ---------------------------------------------------------------------------
// Colloid
// ---------------------------------------------------------------------------

/// Runs Colloid from frame `from` of a 120 BPM grid for `frames`, with
/// `keys` arriving on its MIDI cable (absolute frames), returning its MIDI.
fn run_colloid(
    c: &mut crate::audio::modules::colloid::Colloid,
    from: usize,
    frames: usize,
    playing: bool,
    restart: bool,
    keys: &[(usize, [u8; 3])],
) -> Vec<(usize, [u8; 3])> {
    use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx, Processor};
    const SR: f64 = 48_000.0;
    let bps = 2.0 / SR;
    let mut inputs = Box::new(Inputs {
        bufs: [[[0.0; 64]; 2]; MAX_INPUTS],
        connected: [false; MAX_INPUTS],
        midi: MidiBuf::default(),
        midi_connected: true,
    });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 1];
    let mut events = Vec::new();
    for done in (0..frames).step_by(64) {
        let at = from + done;
        inputs.midi.clear();
        for (f, m) in keys.iter().filter(|(f, _)| (at..at + 64).contains(f)) {
            inputs.midi.push(f - at, *m);
        }
        let ctx = ProcessCtx {
            sample_rate: SR as f32,
            smooth: 0.01,
            beat: at as f64 * bps,
            beats_per_sample: bps,
            playing,
            restart: restart && done == 0,
        };
        c.process(&ctx, &inputs, &mut outputs, 64);
        for e in c.midi_out().expect("sends MIDI").events() {
            events.push((at + e.frame as usize, e.data));
        }
    }
    events
}

fn colloid(p: &Arc<ModuleParams>) -> crate::audio::modules::colloid::Colloid {
    crate::audio::modules::colloid::Colloid::new(p.clone(), Arc::new(crate::audio::midi::MidiState::new()), 48_000.0)
}

/// The pitch classes Colloid says it plays.
fn colloid_pcs(p: &ModuleParams) -> u16 {
    use crate::audio::modules::colloid::viz;
    p.viz[viz::MASK].get() as u16 | p.viz[viz::COLOURS].get() as u16
}

/// Each note's length in frames.
fn note_lengths(events: &[(usize, [u8; 3])]) -> Vec<usize> {
    let mut on = std::collections::HashMap::<u8, usize>::new();
    let mut lengths = Vec::new();
    for &(f, m) in events {
        match m[0] & 0xF0 {
            0x90 if m[2] > 0 => {
                on.insert(m[1], f);
            }
            0x80 | 0x90 => {
                if let Some(start) = on.remove(&m[1]) {
                    lengths.push(f - start);
                }
            }
            _ => {}
        }
    }
    lengths
}

#[test]
fn colloid_strikes_play_the_chords_notes_and_every_note_ends() {
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    let mut c = colloid(&p);
    let mut events = run_colloid(&mut c, 0, 20 * 48_000, true, true, &[]);
    events.extend(run_colloid(&mut c, 20 * 48_000, 4_800, false, false, &[]));
    let ons = note_ons(&events);
    eprintln!("default patch: {} notes in 20 s", ons.len());
    assert!(ons.len() > 20, "the atoms strike: {} notes", ons.len());
    let pcs = colloid_pcs(&p);
    assert!(ons.iter().all(|&(_, n)| pcs >> (n % 12) & 1 == 1), "every note is in the chord");
    assert!(events.iter().all(|(_, m)| m[0] & 0x0F == 0), "on channel 1");
    assert!(events.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90).all(|(_, m)| (1..=127).contains(&m[2])));
    assert!(every_note_ends(&events), "stopping ends the notes");
    // Velocities follow the strikes: not all the same.
    let vels: std::collections::BTreeSet<u8> = events.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90).map(|(_, m)| m[2]).collect();
    assert!(vels.len() > 5, "dynamics from the physics: {vels:?}");
}

#[test]
fn colloid_is_silent_while_stopped_or_paused() {
    use crate::audio::modules::colloid::viz;
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    let mut c = colloid(&p);
    assert!(run_colloid(&mut c, 0, 4 * 48_000, false, false, &[]).is_empty());
    assert_eq!(p.viz[viz::ATOMS].get(), 0.0, "no atoms while stopped");
    p.set_toggle(false);
    assert!(run_colloid(&mut c, 0, 4 * 48_000, true, true, &[]).is_empty());
    p.set_toggle(true);
    let events = run_colloid(&mut c, 4 * 48_000, 4 * 48_000, true, false, &[]);
    assert!(!note_ons(&events).is_empty());
    assert_eq!(p.viz[viz::ATOMS].get(), 6.0);
}

#[test]
fn colloid_takes_the_chord_from_the_keys() {
    use crate::audio::modules::colloid::chords::FIRST_CHORD;
    use crate::audio::modules::colloid::{COLOR, MOLECULES, viz};
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    p.set_mode(FIRST_CHORD); // major
    p.set(COLOR, 0.0);
    p.set(MOLECULES, 9.0);
    let mut c = colloid(&p);
    let c_major = [(100, [0x90, 60, 90]), (300, [0x90, 64, 90]), (400, [0x90, 67, 90]), (9_000, [0x80, 60, 0]), (9_000, [0x80, 64, 0]), (9_000, [0x80, 67, 0])];
    let events = run_colloid(&mut c, 0, 10 * 48_000, true, true, &c_major);
    assert_eq!(p.viz[viz::SOURCE].get(), 2.0);
    assert_eq!(p.viz[viz::MASK].get() as u16, 1 | 1 << 4 | 1 << 7);
    let late: Vec<_> = note_ons(&events).into_iter().filter(|&(f, _)| f > 4_800).collect();
    assert!(!late.is_empty());
    assert!(late.iter().all(|&(_, n)| [0, 4, 7].contains(&(n % 12))), "C E G only: {late:?}");
    // The molecules hold the chord's notes, each a different one.
    let notes: Vec<u8> = (0..9).map(|k| p.viz[viz::MOLECULE + k * viz::MOLECULE_LEN + 5].get() as u8).collect();
    assert!(notes.iter().all(|n| [0, 4, 7].contains(&(n % 12))));
    assert_eq!(notes.iter().collect::<std::collections::BTreeSet<_>>().len(), 9);

    // One key: a new root for the panel's shape (major): A C# E.
    let from = 10 * 48_000;
    let a = [(from + 100, [0x90, 69, 90]), (from + 4_000, [0x80, 69, 0])];
    let events = run_colloid(&mut c, from, 6 * 48_000, true, false, &a);
    assert_eq!(p.viz[viz::SOURCE].get(), 1.0);
    let late: Vec<_> = note_ons(&events).into_iter().filter(|&(f, _)| f > from + 4_800).collect();
    assert!(late.iter().all(|&(_, n)| [9, 1, 4].contains(&(n % 12))), "A C# E only: {late:?}");

    // Program Change picks the shape; letting go goes back to ROOT (D).
    run_colloid(&mut c, from, 640, true, false, &[(from + 10, [0xC0, FIRST_CHORD as u8 + 1, 0])]);
    assert_eq!(p.mode(), FIRST_CHORD + 1);
    p.viz[viz::UNLATCH].set(1.0);
    run_colloid(&mut c, from, 640, true, false, &[]);
    assert_eq!(p.viz[viz::SOURCE].get(), 0.0);
    assert_eq!(p.viz[viz::MASK].get() as u16, 1 << 2 | 1 << 5 | 1 << 9, "D minor");
}

#[test]
fn colloid_quantized_notes_land_on_the_grid() {
    use crate::audio::modules::colloid::QUANT;
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    p.set(QUANT, 4.0); // 1/16
    let mut c = colloid(&p);
    let mut events = run_colloid(&mut c, 0, 10 * 48_000, true, true, &[]);
    events.extend(run_colloid(&mut c, 10 * 48_000, 4_800, false, false, &[]));
    let ons = note_ons(&events);
    assert!(ons.len() > 10);
    assert!(ons.iter().all(|&(f, _)| f % SIXTEENTH == 0), "on sixteenths: {:?}", &ons[..ons.len().min(12)]);
    assert!(every_note_ends(&events));
}

#[test]
fn colloid_physics_shapes_the_notes() {
    use crate::audio::modules::colloid::{ATOMS, LENGTH, MOLECULES, VISCOSITY, viz};
    let take = |set: &dyn Fn(&ModuleParams)| {
        let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
        set(&p);
        let mut c = colloid(&p);
        let mut events = run_colloid(&mut c, 0, 20 * 48_000, true, true, &[]);
        events.extend(run_colloid(&mut c, 20 * 48_000, 9_600, false, false, &[]));
        (p, events)
    };
    let (_, few) = take(&|p| p.set(ATOMS, 3.0));
    let (_, many) = take(&|p| p.set(ATOMS, 40.0));
    eprintln!("3 atoms: {} notes, 40 atoms: {} notes", note_ons(&few).len(), note_ons(&many).len());
    assert!(note_ons(&many).len() > 2 * note_ons(&few).len(), "more atoms, more strikes");

    let mean = |e: &[(usize, [u8; 3])]| {
        let l = note_lengths(e);
        l.iter().sum::<usize>() as f32 / l.len().max(1) as f32 / 48_000.0
    };
    let (_, thin) = take(&|p| p.set(VISCOSITY, 0.0));
    let (_, thick) = take(&|p| p.set(VISCOSITY, 1.0));
    eprintln!("mean note: thin {:.3} s, thick {:.3} s", mean(&thin), mean(&thick));
    assert!(mean(&thick) < mean(&thin), "a thick liquid damps the rings");
    let (_, short) = take(&|p| p.set(LENGTH, 0.05));
    let (_, long) = take(&|p| p.set(LENGTH, 3.0));
    assert!(mean(&long) > 3.0 * mean(&short));

    let (p, events) = take(&|p| p.set(MOLECULES, 4.0));
    assert_eq!(p.viz[viz::MOLECULES].get(), 4.0);
    let distinct: std::collections::BTreeSet<u8> = note_ons(&events).into_iter().map(|(_, n)| n).collect();
    assert!(distinct.len() <= 4, "one note per molecule: {distinct:?}");
}

#[test]
fn colloid_panel_draws() {
    use crate::audio::modules::colloid::{ATOMS, viz};
    let shared = Arc::new(EngineShared::new());
    shared.running.store(true, Ordering::Relaxed);
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Colloid, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    p.set(ATOMS, 30.0);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for shape in 0..12 {
        p.set_mode(shape);
        for _ in 0..4 {
            render_buf(&mut e, 4_800);
            time += 0.1;
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[viz::EVENT_SEQ].get() > 0.0, "the atoms struck while drawn");
}

/// A stepped knob (ROOT: 12 notes) dragged slowly, a pixel a frame, with the
/// panel rounding what it sets: it steps on all the same (it used to round
/// each frame's movement away and stay put), and a slow drag down brings it
/// back.
#[test]
fn stepped_knobs_follow_slow_drags() {
    use crate::audio::params::{ParamSpec, Unit};
    use egui::{Event, Modifiers, PointerButton};
    let spec = ParamSpec::lin("ROOT", 0.0, 11.0, 0.0, Unit::NoteName);
    let ctx = egui::Context::default();
    let id = egui::Id::unique("stepped-knob");
    let c = pos2(100.0, 100.0);
    let mut value = 0.0f32;
    let mut seen = Vec::new();
    let frame = |events: Vec<Event>, t: f64, value: &mut f32| {
        let input = egui::RawInput { time: Some(t), events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let painter = ui.painter().clone();
            let mut norm = spec.to_norm(*value);
            let resp = crate::ui::widgets::knob(ui, &painter, id, c, 10.0, &mut norm, 0.0, false, egui::Color32::WHITE, 1.0);
            if resp.dragged() {
                *value = spec.denorm(norm).round();
            }
        });
        out.textures_delta.clear();
    };
    let mut t = 0.0;
    frame(vec![Event::PointerMoved(c)], t, &mut value);
    t += 1.0 / 60.0;
    frame(vec![Event::PointerButton { pos: c, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }], t, &mut value);
    let mut at = c;
    for _ in 0..120 {
        at.y -= 1.0;
        t += 1.0 / 60.0;
        frame(vec![Event::PointerMoved(at)], t, &mut value);
        seen.push(value);
    }
    assert!(value >= 5.0, "a slow drag up steps on: {value}");
    assert!(seen.windows(2).all(|w| w[1] - w[0] <= 1.0), "one step at a time");
    for _ in 0..60 {
        at.y += 1.0;
        t += 1.0 / 60.0;
        frame(vec![Event::PointerMoved(at)], t, &mut value);
    }
    let back = value;
    assert!(back < seen[seen.len() - 1], "and back down: {back}");
}


#[test]
fn colloid_plays_handpan_layouts() {
    use crate::audio::modules::colloid::{MOLECULES, viz};
    // D Kurd by default: D3 | A3 Bb3 C4 D4 E4 F4 G4 A4 C5.
    let kurd = [50, 57, 58, 60, 62, 64, 65, 67, 69, 72];
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    p.set(MOLECULES, 10.0);
    let mut c = colloid(&p);
    let events = run_colloid(&mut c, 0, 10 * 48_000, true, true, &[]);
    assert_eq!(p.viz[viz::LAYOUT].get(), 0.0);
    let notes: Vec<u8> = (0..10).map(|k| p.viz[viz::MOLECULE + k * viz::MOLECULE_LEN + 5].get() as u8).collect();
    let mut sorted = notes.clone();
    sorted.sort();
    assert_eq!(sorted, kurd, "one molecule per note of the layout");
    assert!(note_ons(&events).iter().all(|(_, n)| kurd.contains(n)));
    // A key moves the ding: A Kurd.
    let from = 10 * 48_000;
    let events = run_colloid(&mut c, from, 6 * 48_000, true, false, &[(from + 100, [0x90, 57, 90]), (from + 2_000, [0x80, 57, 0])]);
    let a_kurd: Vec<u8> = kurd.iter().map(|n| n + 7 - 12).collect();
    let late: Vec<_> = note_ons(&events).into_iter().filter(|&(f, _)| f > from + 4_800).collect();
    assert!(!late.is_empty());
    assert!(late.iter().all(|(_, n)| a_kurd.iter().any(|k| k % 12 == n % 12)), "A Kurd: {late:?}");
}

/// The CC events sent on lane `k`'s controller: (frame, value).
fn ccs(events: &[(usize, [u8; 3])], number: u8) -> Vec<(usize, u8)> {
    events.iter().filter(|(_, m)| m[0] & 0xF0 == 0xB0 && m[1] == number).map(|(f, m)| (*f, m[2])).collect()
}

#[test]
fn colloid_pedal_catches_each_chord_and_lets_go_on_stop() {
    use crate::audio::modules::colloid::{ROOT, cc};
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    p.data[cc::at(0, cc::ON)].set(1.0); // CC 64 from the pedal
    let mut c = colloid(&p);
    let mut events = run_colloid(&mut c, 0, 48_000, true, true, &[]);
    p.set(ROOT, 7.0); // a new chord a second in
    events.extend(run_colloid(&mut c, 48_000, 48_000, true, false, &[]));
    events.extend(run_colloid(&mut c, 96_000, 4_800, false, false, &[]));
    let pedal = ccs(&events, 64);
    assert_eq!(pedal.first(), Some(&(0, 127)), "down on Start");
    let lift = pedal.iter().position(|&(f, v)| f >= 48_000 && v == 0).expect("lifted at the new chord");
    let (at, _) = pedal[lift];
    assert!(at < 48_000 + 128);
    let (back, v) = pedal[lift + 1];
    assert_eq!(v, 127);
    assert!((back - at) as f32 / 48_000.0 > 0.06 && (back - at) as f32 / 48_000.0 < 0.15, "down again soon: {}", back - at);
    assert_eq!(pedal.last().map(|e| e.1), Some(0), "up on Stop");
    assert!(events.iter().all(|(_, m)| m[0] & 0x0F == 0));
}

#[test]
fn colloid_passes_the_keys_controllers_through_and_sends_the_tanks() {
    use crate::audio::modules::colloid::cc;
    let p = Arc::new(ModuleParams::new(ModuleKind::Colloid));
    // Lane 0: the keys' sustain, inverted into 100…20. Lane 1: the energy
    // on CC 1, into 10…90.
    for (i, v) in [(cc::ON, 1.0), (cc::SOURCE, cc::KEYS as f32), (cc::LOW, 100.0), (cc::HIGH, 20.0)] {
        p.data[cc::at(0, i)].set(v);
    }
    for (i, v) in [(cc::ON, 1.0), (cc::LOW, 10.0), (cc::HIGH, 90.0)] {
        p.data[cc::at(1, i)].set(v);
    }
    let mut c = colloid(&p);
    let keys = [(1_000, [0xB0, 64, 127]), (30_000, [0xB0, 64, 0]), (40_000, [0xB0, 7, 99])];
    let events = run_colloid(&mut c, 0, 6 * 48_000, true, true, &keys);
    assert_eq!(ccs(&events, 64), vec![(960, 20), (29_952, 100)], "passed through (in the block they came in), mapped");
    assert!(ccs(&events, 7).is_empty(), "only its own controller");
    let energy = ccs(&events, 1);
    assert!(energy.len() > 10, "the energy moves");
    assert!(energy.iter().all(|&(_, v)| (10..=90).contains(&v)));
    assert!(energy.windows(2).all(|w| w[1].0 - w[0].0 >= 448), "at most every 10 ms");
    // Stopped, the tank's lanes are quiet.
    let stopped = run_colloid(&mut c, 6 * 48_000, 48_000, false, false, &[]);
    assert!(ccs(&stopped, 1).is_empty());
}


// ---------------------------------------------------------------------------
// Fjord
// ---------------------------------------------------------------------------

fn fjord_params() -> Arc<ModuleParams> {
    Arc::new(ModuleParams::new(ModuleKind::Fjord))
}

/// What a Fjord run produced: the main output (left), each channel's
/// output, and the MIDI it sent (absolute frames).
struct FjordRun {
    out: Vec<f32>,
    channels: [Vec<f32>; 4],
    sent: Vec<(usize, [u8; 3])>,
}

/// Runs Fjord at 120 BPM for `blocks`, with `midi` arriving at (frame,
/// message) and `trig` (if any) feeding TRIG 1 sample by sample.
fn run_fjord(p: &Arc<ModuleParams>, blocks: usize, playing: bool, midi: &[(usize, [u8; 3])], trig: Option<&dyn Fn(usize) -> f32>) -> FjordRun {
    use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx, Processor};
    use crate::audio::modules::fjord::Fjord;
    let mut proc = Fjord::new(p.clone(), 48_000.0);
    let mut ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing, restart: false };
    let mut inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: true });
    inputs.connected[0] = trig.is_some();
    let mut outputs = vec![[[0.0f32; 64]; 2]; 5];
    let mut run = FjordRun { out: Vec::new(), channels: Default::default(), sent: Vec::new() };
    for b in 0..blocks {
        let at = b * 64;
        inputs.midi.clear();
        for (f, m) in midi.iter().filter(|(f, _)| (at..at + 64).contains(f)) {
            inputs.midi.push(f - at, *m);
        }
        if let Some(trig) = trig {
            for i in 0..64 {
                let x = trig(at + i);
                inputs.bufs[0][0][i] = x;
                inputs.bufs[0][1][i] = x;
            }
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        run.out.extend_from_slice(&outputs[0][0]);
        for c in 0..4 {
            run.channels[c].extend_from_slice(&outputs[1 + c][0]);
        }
        run.sent.extend(proc.midi_out().unwrap().events().iter().map(|e| (at + e.frame as usize, e.data)));
        if playing {
            ctx.beat += ctx.beats_per_sample * 64.0;
        }
    }
    run
}

/// A Fjord with the sequencer off.
fn fjord_quiet() -> Arc<ModuleParams> {
    use crate::audio::modules::fjord::data;
    let p = fjord_params();
    p.data[data::RUN].set(0.0);
    p
}

/// Rising zero crossings per second over `buf`.
fn crossings_per_second(buf: &[f32]) -> f32 {
    buf.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32 * 48_000.0 / buf.len() as f32
}

/// The last sample louder than `share` of the peak, in ms.
fn length_ms(buf: &[f32], share: f32) -> f32 {
    let peak = peak_of(buf);
    buf.iter().rposition(|y| y.abs() > peak * share).unwrap_or(0) as f32 / 48.0
}

#[test]
fn fjord_programs_pack_load_store_and_name() {
    use crate::audio::modules::fjord::bank::{FACTORY, INIT, PROGRAMS};
    use crate::audio::modules::fjord::{self as fj, ch, data, knob};
    let p = fjord_params();
    // The knobs start as P1, unedited.
    assert_eq!(fj::program(&p), 0);
    assert!(!fj::edited(&p));
    for (k, prog) in FACTORY.iter().enumerate() {
        fj::load_program(&p, k);
        for c in 0..4 {
            assert_eq!(fj::sound(&p, c), prog.channels[c], "P{} {} ch{} survives packing", k + 1, prog.name, c + 1);
        }
        assert!(!fj::edited(&p));
        assert_eq!(fj::program_name(&p, k), prog.name);
    }
    assert_eq!(fj::program_name(&p, PROGRAMS - 1), "Empty");
    fj::load_program(&p, 40);
    assert_eq!(fj::sound(&p, 2), INIT);
    // An edit shows; Memory Protect keeps it from being stored.
    p.set(ch(1, knob::PITCH), 60.5);
    assert!(fj::edited(&p));
    assert!(!fj::store_program(&p, 40));
    p.data[data::PROTECT].set(0.0);
    assert!(fj::store_program(&p, 60));
    assert_eq!(fj::program(&p), 60);
    assert!(!fj::edited(&p));
    assert_eq!(fj::program_name(&p, 60), "User");
    assert_eq!(fj::stored(&p, 60)[1][knob::PITCH], 60.5, "half semitones are kept");
    // Sound Init and the extremes of every range survive packing.
    fj::init_channel(&p, 0);
    assert_eq!(fj::sound(&p, 0), INIT);
    for (k, &(_, lo, hi, _)) in fj::RANGES.iter().enumerate() {
        for v in [lo, hi] {
            p.set(ch(3, k), v);
            fj::store_program(&p, 98);
            assert_eq!(fj::stored(&p, 98)[3][k], v, "knob {k} at {v}");
        }
    }
}

#[test]
fn fjord_sequencer_plays_its_lanes_on_the_board_grid() {
    use crate::audio::modules::fjord::{self as fj, data, viz};
    let p = fjord_params();
    // 120 BPM, sixteenths: a step every 6000 frames. The kick lane starts
    // 3 0 0 0 2 0 0 0 3 0 0 1 2 …, on MIDI channel 10, note 36.
    let run = run_fjord(&p, 1500, true, &[], None);
    let kicks: Vec<(usize, u8)> = run.sent.iter().filter(|(_, m)| m[0] == 0x99 && m[1] == 36).map(|(f, m)| (*f, m[2])).collect();
    assert_eq!(kicks, [(0, 127), (24_000, 91), (48_000, 127), (66_000, 51), (72_000, 91)]);
    assert!(run.sent.iter().any(|(_, m)| m[0] == 0x89 && m[1] == 36), "notes end");
    assert!(rms_of(&run.out) > 0.02);
    assert!(rms_of(&run.channels[0]) > 0.02 && rms_of(&run.channels[1]) > 0.005, "each channel on its own output");
    assert!(p.viz[viz::STEP].get() >= 0.0);
    // Stopped: no steps.
    let run = run_fjord(&p, 400, false, &[], None);
    assert!(run.sent.is_empty() && peak_of(&run.out) == 0.0);
    assert_eq!(p.viz[viz::STEP].get(), -1.0);
    // Eighths, 6 steps, a mute: the kick every 12000 frames on steps 0 and 4.
    p.data[data::RATE].set(1.0);
    p.data[data::LENGTH].set(6.0);
    p.data[data::MUTE + 3].set(1.0);
    let run = run_fjord(&p, 1500, true, &[], None);
    let kicks: Vec<usize> = run.sent.iter().filter(|(_, m)| m[0] == 0x99 && m[1] == 36).map(|(f, _)| *f).collect();
    assert_eq!(kicks, [0, 48_000, 72_000]);
    assert!(run.sent.iter().all(|(_, m)| m[1] != 47), "lane 4 muted");
    // Edited steps.
    fj::set_step(&p, 2, 1, 3);
    let run = run_fjord(&p, 200, true, &[], None);
    assert!(run.sent.iter().any(|&(f, m)| f == 12_000 && m == [0x99, 50, 127]));
    // RUN off: silent.
    p.data[data::RUN].set(0.0);
    assert!(run_fjord(&p, 400, true, &[], None).sent.is_empty());
}

#[test]
fn fjord_midi_notes_strike_their_channels() {
    use crate::audio::modules::fjord::{data, viz};
    let p = fjord_quiet();
    // Note 38 on channel 10 strikes channel 2 only; played in, not sent out.
    let run = run_fjord(&p, 300, true, &[(640, [0x99, 38, 110])], None);
    assert!(run.sent.is_empty());
    assert!(peak_of(&run.out[..640]) == 0.0 && rms_of(&run.channels[1]) > 0.005);
    assert!(peak_of(&run.channels[0]) == 0.0);
    assert_eq!(p.viz[viz::HITS + 1].get(), 1.0);
    // Another MIDI channel, or Off: ignored.
    assert!(peak_of(&run_fjord(&p, 100, true, &[(0, [0x90, 38, 110])], None).out) == 0.0);
    p.data[data::MIDI_CH].set(0.0);
    assert!(peak_of(&run_fjord(&p, 100, true, &[(0, [0x99, 38, 110])], None).out) == 0.0);
    // Learn the channel from the next message, then a note for channel 4.
    p.data[data::LEARN].set(1.0);
    let run = run_fjord(&p, 100, true, &[(0, [0x92, 36, 100])], None);
    assert_eq!(p.data[data::MIDI_CH].get(), 3.0);
    assert!(rms_of(&run.channels[0]) > 0.005);
    p.data[data::LEARN].set(5.0);
    let run = run_fjord(&p, 100, true, &[(0, [0x92, 64, 100])], None);
    assert_eq!(p.data[data::NOTE + 3].get(), 64.0);
    assert_eq!(p.data[data::LEARN].get(), 0.0);
    assert!(rms_of(&run.channels[3]) > 0.001);
    // FOLLOW: the pad played picks the channel to edit.
    assert_eq!(p.data[data::CHANNEL].get(), 0.0);
    p.data[data::FOLLOW].set(1.0);
    run_fjord(&p, 10, true, &[(0, [0x92, 64, 100])], None);
    assert_eq!(p.data[data::CHANNEL].get(), 3.0);
    // Velocity: softer is quieter; Dn2 makes it hotter.
    let loud = rms_of(&run_fjord(&p, 100, true, &[(0, [0x92, 36, 127])], None).channels[0]);
    let soft = rms_of(&run_fjord(&p, 100, true, &[(0, [0x92, 36, 40])], None).channels[0]);
    p.data[data::DYN].set(1.0);
    let hot = rms_of(&run_fjord(&p, 100, true, &[(0, [0x92, 36, 40])], None).channels[0]);
    assert!(soft < loud * 0.5 && hot > soft * 1.5, "{soft} {loud} {hot}");
}

#[test]
fn fjord_program_changes_go_both_ways() {
    use crate::audio::modules::fjord::bank::FACTORY;
    use crate::audio::modules::fjord::{self as fj, data};
    let p = fjord_quiet();
    // Received (the default): program 7 loads P8.
    run_fjord(&p, 4, true, &[(0, [0xC9, 7, 0])], None);
    assert_eq!(fj::program(&p), 7);
    assert_eq!(fj::sound(&p, 0), FACTORY[7].channels[0]);
    // Not received when set to Send only; sent when the panel picks one.
    p.data[data::PROG_CHANGE].set(1.0);
    run_fjord(&p, 4, true, &[(0, [0xC9, 2, 0])], None);
    assert_eq!(fj::program(&p), 7);
    use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx, Processor};
    let mut proc = crate::audio::modules::fjord::Fjord::new(p.clone(), 48_000.0);
    let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: true, restart: false };
    let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 5];
    proc.process(&ctx, &inputs, &mut outputs, 64);
    assert!(proc.midi_out().unwrap().events().is_empty());
    fj::load_program(&p, 11);
    proc.process(&ctx, &inputs, &mut outputs, 64);
    assert_eq!(proc.midi_out().unwrap().events()[0].data, [0xC9, 11, 0]);
}

#[test]
fn fjord_local_off_sends_without_sounding() {
    use crate::audio::modules::fjord::data;
    let p = fjord_params();
    p.data[data::LOCAL].set(0.0);
    let run = run_fjord(&p, 800, true, &[], None);
    assert!(run.sent.iter().any(|(_, m)| m[0] == 0x99 && m[1] == 36), "the pattern still goes out");
    assert_eq!(peak_of(&run.out), 0.0);
    // MIDI coming back still plays.
    let run = run_fjord(&p, 100, false, &[(0, [0x99, 36, 100])], None);
    assert!(rms_of(&run.out) > 0.01);
}

#[test]
fn fjord_panel_trig_plays_the_edited_channel_at_its_velocity() {
    use crate::audio::modules::fjord::{data, viz};
    use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx, Processor};
    let p = fjord_quiet();
    p.data[data::CHANNEL].set(2.0);
    let mut proc = crate::audio::modules::fjord::Fjord::new(p.clone(), 48_000.0);
    let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: false, restart: false };
    let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 5];
    // The panel bumps the edited channel's counter.
    p.viz[viz::AUDITION + 2].set(1.0);
    let (mut sound, mut sent) = (Vec::new(), Vec::new());
    for _ in 0..200 {
        proc.process(&ctx, &inputs, &mut outputs, 64);
        sound.extend_from_slice(&outputs[3][0]);
        sent.extend(proc.midi_out().unwrap().events().iter().map(|e| e.data));
    }
    assert!(rms_of(&sound) > 0.01);
    assert!(sent.contains(&[0x99, 50, 100]), "TRIG goes out too, at the TRIG velocity: {sent:?}");
}

#[test]
fn fjord_trig_inputs_find_hits_and_skip_the_ringing() {
    use crate::audio::modules::fjord::{data, trigger, viz};
    let p = fjord_quiet();
    // A decaying 200 Hz "drum" every 12000 frames, the second softer.
    let drum = |f: usize| {
        let (k, t) = (f / 12_000, (f % 12_000) as f32 / 48_000.0);
        let amp = if k % 2 == 0 { 0.8 } else { 0.1 };
        amp * (-t * 30.0).exp() * (std::f32::consts::TAU * 200.0 * t).sin()
    };
    let run = run_fjord(&p, 750, false, &[], Some(&drum));
    let notes: Vec<(usize, u8)> = run.sent.iter().filter(|(_, m)| m[0] == 0x99).map(|(f, m)| (*f, m[2])).collect();
    assert_eq!(notes.len(), 4, "one note per hit, none from its ringing: {notes:?}");
    assert!(notes.iter().all(|&(f, _)| f % 12_000 < 300), "{notes:?}");
    assert!(notes[0].1 == 127 && notes[1].1 < 70, "hard and soft: {notes:?}");
    assert!(p.viz[viz::STRENGTH].get() > 0.0);
    assert!(rms_of(&run.channels[0]) > 0.01);
    // A higher threshold drops the soft hits.
    p.data[data::THRES].set(85.0);
    let run = run_fjord(&p, 750, false, &[], Some(&drum));
    assert_eq!(run.sent.iter().filter(|(_, m)| m[0] == 0x99).count(), 2);
    // Sens and Thres in dB.
    assert_eq!(trigger::full_db(0.0), 0.0);
    assert!(trigger::threshold_db(99.0).abs() < 1e-6);
}

#[test]
fn fjord_engine_follows_its_parameters() {
    use crate::audio::modules::fjord::bank::INIT;
    use crate::audio::modules::fjord::knob::*;
    use crate::audio::modules::fjord::voice::{Sound, WAVES, pitch_hz, render};
    let hit = |s: &Sound, n: usize| {
        let parts = render(s, 1.0, 0.1, 48_000.0, n);
        (0..n).map(|i| parts[0][i] + parts[1][i] + parts[2][i]).collect::<Vec<f32>>()
    };
    let with = |set: &[(usize, f32)]| {
        let mut s = INIT;
        for &(k, v) in set {
            s[k] = v;
        }
        s
    };
    // Pitch in MIDI notes: 69 is 440 Hz, 57.5 a semitone and a half under A3.
    for note in [45.0, 57.5, 69.0] {
        let s = with(&[(PITCH, note), (T_DECAY, 40.0)]);
        let f = crossings_per_second(&hit(&s, 24_000)[2400..]);
        assert!((f / pitch_hz(note) - 1.0).abs() < 0.03, "{note}: {f} vs {}", pitch_hz(note));
    }
    // Bend: positive starts above and falls; negative starts below.
    let plain = with(&[(PITCH, 57.0), (T_DECAY, 40.0)]);
    let down = with(&[(PITCH, 57.0), (T_DECAY, 40.0), (BEND, 24.0), (BEND_TIME, 4.0)]);
    let up = with(&[(PITCH, 57.0), (T_DECAY, 40.0), (BEND, -9.0), (BEND_TIME, 4.0)]);
    let early = |s: &Sound| crossings_per_second(&hit(s, 4800)[..2400]);
    assert!(early(&down) > early(&plain) * 1.5 && early(&up) < early(&plain) * 0.9, "{} {} {}", early(&down), early(&plain), early(&up));
    // Decay longer → longer; the gate G holds then cuts, S holds longer
    // than n.
    let short = length_ms(&hit(&with(&[(T_DECAY, 20.0)]), 48_000), 0.1);
    let long = length_ms(&hit(&with(&[(T_DECAY, 35.0)]), 48_000), 0.1);
    assert!(long > short * 4.0, "{short} {long}");
    let half = |s: &Sound| length_ms(&hit(s, 96_000), 0.5);
    let (n, soft, gated) = (half(&with(&[(T_DECAY, 35.0)])), half(&with(&[(T_DECAY, 35.0), (T_GATE, 1.0)])), half(&with(&[(T_DECAY, 35.0), (T_GATE, 2.0)])));
    assert!(soft > n * 2.0 && gated > n * 1.5, "half level at n {n} S {soft} G {gated}");
    let g = hit(&with(&[(T_DECAY, 35.0), (T_GATE, 2.0)]), 96_000);
    let cut = length_ms(&g, 0.4) - length_ms(&g, 0.01);
    assert!(cut.abs() < 60.0, "G falls fast after the hold: {cut} ms");
    // The tone filter darkens a sawtooth; its sweep brightens the strike.
    let saw = |set: &[(usize, f32)]| {
        let mut all = vec![(T_WAVE, 2.0), (PITCH, 45.0), (T_DECAY, 40.0)];
        all.extend_from_slice(set);
        hit(&with(&all), 9600)
    };
    let bright = |b: &[f32]| rms_of(&b.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>()) / rms_of(b);
    let open = bright(&saw(&[])[2400..]);
    let dark = bright(&saw(&[(T_FILTER, 40.0)])[2400..]);
    let swept = bright(&saw(&[(T_FILTER, 40.0), (T_SWEEP, 10.0), (T_TIME, 9.0)])[..2400]);
    assert!(dark < open * 0.5 && swept > dark * 1.5, "{open} {dark} {swept}");
    // Noise: COLOR below 0 darker, above 0 brighter; the filter darker still.
    let noise = |set: &[(usize, f32)]| {
        let mut all = vec![(TONE, 0.0), (NOISE, 10.0), (N_FILTER, 99.0), (N_DECAY, 40.0)];
        all.extend_from_slice(set);
        crossings_per_second(&hit(&with(&all), 9600))
    };
    let (dull, flat, thin) = (noise(&[(N_COLOR, -8.0)]), noise(&[]), noise(&[(N_COLOR, 8.0)]));
    assert!(dull < flat * 0.5 && thin > flat * 1.1, "{dull} {flat} {thin}");
    assert!(noise(&[(N_FILTER, 40.0)]) < flat * 0.3);
    // Every waveform, filter type and click type sounds and stays in range,
    // with the punch and the sweeps up.
    for w in 0..WAVES.len() {
        let b = hit(&with(&[(T_WAVE, w as f32), (T_PUNCH, 9.0), (T_SWEEP, 10.0), (BEND, 30.0), (PITCH, 98.5)]), 9600);
        assert!(b.iter().all(|y| y.is_finite()) && rms_of(&b) > 0.02 && peak_of(&b) < 3.0, "wave {w}: {} {}", rms_of(&b), peak_of(&b));
    }
    for t in 1..=7 {
        let b = hit(&with(&[(TONE, 0.0), (NOISE, 10.0), (N_TYPE, t as f32), (N_FILTER, 0.0), (N_SWEEP, 10.0), (N_DECAY, 30.0)]), 9600);
        assert!(b.iter().all(|y| y.is_finite()) && rms_of(&b) > 0.005 && peak_of(&b) < 3.0, "type {t}: {}", rms_of(&b));
    }
    for c in 0..27 {
        let b = hit(&with(&[(TONE, 0.0), (CLICK, 10.0), (C_TYPE, c as f32), (C_DECAY, 6.0)]), 960);
        assert!(b.iter().all(|y| y.is_finite()) && rms_of(&b) > 0.01 && peak_of(&b) < 3.0, "click {c}: {}", rms_of(&b));
    }
}

#[test]
fn fjord_solo_edit_keeps_one_component_of_the_edited_channel() {
    use crate::audio::modules::fjord::{ch, data, knob};
    let p = fjord_quiet();
    // Channel 2 (the snare) with only its noise soloed: no tone left.
    p.set(ch(1, knob::NOISE), 0.0);
    p.data[data::CHANNEL].set(1.0);
    p.data[data::SOLO].set(2.0);
    let run = run_fjord(&p, 200, false, &[(0, [0x99, 38, 120])], None);
    assert!(peak_of(&run.channels[1]) < 1e-4, "{}", peak_of(&run.channels[1]));
    p.data[data::SOLO].set(1.0);
    let run = run_fjord(&p, 200, false, &[(0, [0x99, 38, 120])], None);
    assert!(rms_of(&run.channels[1]) > 0.01);
}

/// The panel draws through the canvas with each channel edited, each
/// solo, a few programs, the sequencer playing and the display hovered.
#[test]
fn fjord_panel_draws() {
    use crate::audio::modules::fjord::{self as fj, data};
    let shared = Arc::new(EngineShared::new());
    shared.running.store(true, Ordering::Relaxed);
    shared.tempo.set(120.0);
    let (a, mut e) = audio::offline(shared, 48_000);
    let mut g = Graph::default();
    let id = g.add_module(ModuleKind::Fjord, pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for (k, prog) in [0usize, 12, 19, 32, 40, 33].into_iter().enumerate() {
        fj::load_program(&p, prog);
        p.data[data::CHANNEL].set((k % 4) as f32);
        p.data[data::SOLO].set((k % 4) as f32);
        p.data[data::TAB].set((k % 2) as f32);
        assert_eq!(g.node(id).unwrap().size().y, crate::ui::fjord::size(&p).y, "the node takes its page's size");
        for frame in 0..4 {
            render_buf(&mut e, 4_800);
            time += 0.1;
            let pointer = egui::Event::PointerMoved(pos2(200.0 + 120.0 * frame as f32, 120.0 + 60.0 * k as f32));
            let input = egui::RawInput { time: Some(time), events: vec![pointer], ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
            out.textures_delta.clear();
        }
    }
    assert!(p.viz[crate::audio::modules::fjord::viz::STEP].get() >= 0.0, "the steps played while drawn");
}

#[test]
fn fjord_tone_adsrs_shape_the_level_and_the_filter() {
    use crate::audio::modules::fjord::bank::INIT;
    use crate::audio::modules::fjord::knob::*;
    use crate::audio::modules::fjord::voice::{Sound, render};
    let with = |set: &[(usize, f32)]| {
        let mut s = INIT;
        for &(k, v) in set {
            s[k] = v;
        }
        s
    };
    let tone = |s: &Sound, gate: f32, n: usize| render(s, 1.0, gate, 48_000.0, n)[0].clone();
    let peak_at = |b: &[f32]| b.iter().enumerate().fold((0, 0.0f32), |(at, m), (i, y)| if y.abs() > m { (i, y.abs()) } else { (at, m) }).0;
    let level = |b: &[f32], ms: usize| peak_of(&b[ms * 48..(ms + 20) * 48]);
    // A slow attack (30: 126 ms) rises where DECAY strikes at once.
    let decay = tone(&with(&[(T_DECAY, 40.0)]), 0.5, 48_000);
    let swell = tone(&with(&[(AMP_ENV, 1.0), (AMP_A, 30.0), (AMP_D, 30.0), (AMP_S, 99.0), (AMP_R, 20.0)]), 0.5, 48_000);
    assert!(peak_at(&decay) < 480 && peak_at(&swell) > 4_800, "{} {}", peak_at(&decay), peak_at(&swell));
    // Held at the sustain while the note is, then released.
    assert!((level(&swell, 300) / level(&swell, 420) - 1.0).abs() < 0.1, "sustains: {} {}", level(&swell, 300), level(&swell, 420));
    assert!(level(&swell, 700) < level(&swell, 420) * 0.05, "released: {}", level(&swell, 700));
    // A longer note sustains longer; with no sustain it decays all the same.
    let long = tone(&with(&[(AMP_ENV, 1.0), (AMP_A, 30.0), (AMP_D, 30.0), (AMP_S, 99.0), (AMP_R, 20.0)]), 0.9, 48_000);
    assert!(level(&long, 700) > level(&long, 420) * 0.8);
    let no_sustain = tone(&with(&[(AMP_ENV, 1.0), (AMP_D, 20.0), (AMP_S, 0.0)]), 0.9, 48_000);
    assert!(level(&no_sustain, 500) < 1e-3, "{}", level(&no_sustain, 500));
    // The filter's ADSR opens a closed sawtooth at the strike and lets it
    // close again; a negative AMOUNT closes an open one.
    let bright = |b: &[f32]| rms_of(&b.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>()) / rms_of(b).max(1e-9);
    let saw = |set: &[(usize, f32)]| {
        let mut all = vec![(T_WAVE, 2.0), (PITCH, 45.0), (T_DECAY, 45.0), (T_FILTER, 30.0)];
        all.extend_from_slice(set);
        tone(&with(&all), 0.5, 24_000)
    };
    let plain = saw(&[]);
    let enveloped = saw(&[(FLT_AMT, 10.0), (FLT_D, 30.0), (FLT_S, 0.0)]);
    assert!(bright(&enveloped[480..2400]) > bright(&plain[480..2400]) * 2.0, "opens at the strike");
    assert!(bright(&enveloped[19_200..24_000]) < bright(&enveloped[480..2400]) * 0.5, "and closes again");
    let open = saw(&[(T_FILTER, 99.0)]);
    let closing = saw(&[(T_FILTER, 99.0), (FLT_AMT, -10.0), (FLT_A, 0.0), (FLT_D, 40.0), (FLT_S, 99.0)]);
    assert!(bright(&closing[2400..9600]) < bright(&open[2400..9600]) * 0.5, "{} {}", bright(&closing[2400..9600]), bright(&open[2400..9600]));
}

#[test]
fn fjord_notes_are_held_by_keys_trig_and_steps() {
    use crate::audio::modules::fjord::{ch, data, knob, viz};
    let p = fjord_quiet();
    // Channel 1: an organ-like tone, held at full sustain, quick release.
    for (k, v) in [(knob::AMP_ENV, 1.0), (knob::AMP_A, 5.0), (knob::AMP_D, 20.0), (knob::AMP_S, 99.0), (knob::AMP_R, 10.0), (knob::NOISE, 0.0), (knob::CLICK, 0.0)] {
        p.set(ch(0, k), v);
    }
    // A MIDI key holds it until its note-off.
    let held = run_fjord(&p, 1000, false, &[(0, [0x99, 36, 120])], None);
    let released = run_fjord(&p, 1000, false, &[(0, [0x99, 36, 120]), (24_000, [0x89, 36, 0])], None);
    let window = |b: &[f32], from: usize| rms_of(&b[from..from + 4800]);
    assert!(window(&held.channels[0], 40_000) > 0.05, "held: {}", window(&held.channels[0], 40_000));
    assert!(window(&released.channels[0], 19_000) > 0.05 && window(&released.channels[0], 40_000) < 1e-3, "let go");
    // A note-off for another note leaves it alone.
    let other = run_fjord(&p, 1000, false, &[(0, [0x99, 36, 120]), (24_000, [0x89, 38, 0])], None);
    assert!(window(&other.channels[0], 40_000) > 0.05);

    // A step holds it for its GATE: 50% of a sixteenth at 120 BPM, 3000
    // frames, and the MIDI note sent lasts as long.
    p.data[data::RUN].set(1.0);
    let run = run_fjord(&p, 400, true, &[], None);
    let on = run.sent.iter().find(|(_, m)| m[0] == 0x99 && m[1] == 36).unwrap().0;
    let off = run.sent.iter().find(|(_, m)| m[0] == 0x89 && m[1] == 36).unwrap().0;
    assert_eq!(off - on, 3000);
    assert!(rms_of(&run.channels[0][1000..2800]) > 0.05 && rms_of(&run.channels[0][5000..5900]) < 1e-3, "released after the gate");
    p.data[data::RUN].set(0.0);

    // TRIG held down holds it; let go, it is released.
    use crate::audio::engine::{Inputs, MAX_INPUTS, MidiBuf, ProcessCtx, Processor};
    let mut proc = crate::audio::modules::fjord::Fjord::new(p.clone(), 48_000.0);
    let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 2.0 / 48_000.0, playing: false, restart: false };
    let inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; MAX_INPUTS], connected: [false; MAX_INPUTS], midi: MidiBuf::default(), midi_connected: false });
    let mut outputs = vec![[[0.0f32; 64]; 2]; 5];
    p.viz[viz::AUDITION].set(p.viz[viz::AUDITION].get() + 1.0);
    p.viz[viz::HOLD].set(1.0);
    let mut sound = Vec::new();
    for b in 0..1000 {
        if b == 500 {
            p.viz[viz::HOLD].set(0.0);
        }
        proc.process(&ctx, &inputs, &mut outputs, 64);
        sound.extend_from_slice(&outputs[1][0]);
    }
    assert!(window(&sound, 20_000) > 0.05, "held while TRIG is down");
    assert!(window(&sound, 40_000) < 1e-3, "released when it is let go");
}

#[test]
fn fjord_alea_throws_breed_sounds_and_grooves_and_take_them_back() {
    use crate::audio::modules::fjord::alea::{self, Before, Role};
    use crate::audio::modules::fjord::voice::render;
    use crate::audio::modules::fjord::{self as fj, CHANNELS, MAX_LEN, RANGES, data, knob};
    let p = fjord_quiet();
    // A full throw of the sounds: every channel changes, every knob stays
    // on its grid and in its range, and every channel is heard.
    for seed in 1..=12u32 {
        fj::load_program(&p, 0);
        let before: Vec<_> = (0..CHANNELS).map(|c| fj::sound(&p, c)).collect();
        alea::throw_sound(&p, seed, 1.0, &[0, 1, 2, 3]);
        for (c, was) in before.iter().enumerate() {
            let s = fj::sound(&p, c);
            assert_ne!(&s, was, "seed {seed} ch {c} changed");
            for (k, &(_, lo, hi, _)) in RANGES.iter().enumerate() {
                assert!(s[k] >= lo && s[k] <= hi && fj::snap(k, s[k]) == s[k], "seed {seed} ch {c} knob {k}: {}", s[k]);
            }
            let parts = render(&s, 0.8, 0.2, 48_000.0, 24_000);
            let mix: Vec<f32> = (0..24_000).map(|i| parts[0][i] + parts[1][i] + parts[2][i]).collect();
            assert!(mix.iter().all(|y| y.is_finite()) && peak_of(&mix) > 0.02, "seed {seed} ch {c} is heard: {}", peak_of(&mix));
        }
    }
    // A small CHANCE nudges: no knob goes more than a tenth of the way.
    fj::load_program(&p, 0);
    let before = fj::sound(&p, 1);
    alea::throw_sound(&p, 7, 0.05, &[1]);
    let after = fj::sound(&p, 1);
    let moved = (0..knob::COUNT).filter(|&k| after[k] != before[k]).count();
    assert!(moved > 0 && (0..knob::COUNT).all(|k| (after[k] - before[k]).abs() <= (RANGES[k].2 - RANGES[k].1) * 0.15 + 1.0), "{before:?} → {after:?}");
    // Only the channels asked for.
    assert_eq!(fj::sound(&p, 0), fj::stored(&p, 0)[0]);

    // Grooves: whatever the role, length and grid, in range; a kick always
    // on the bar's first step, a snare always on the backbeats.
    let mut rng = crate::audio::modules::Rng::new(3);
    for role in [Role::Kick, Role::Snare, Role::Hat, Role::Tom] {
        for len in 1..=MAX_LEN {
            for per_beat in [1, 2, 4, 6, 8] {
                let g = alea::groove(&mut rng, role, len, per_beat);
                assert!(g.iter().all(|&v| v <= 3) && g[len..].iter().all(|&v| v == 0));
            }
        }
        let g = alea::groove(&mut rng, role, 16, 4);
        match role {
            Role::Kick => assert_eq!(g[0], 3),
            Role::Snare => assert!(g[4] >= 2 && g[12] >= 2, "{g:?}"),
            _ => {}
        }
    }
    // First Light's channels are taken for kick, snare, tom, hihat.
    fj::load_program(&p, 0);
    let roles: Vec<Role> = (0..CHANNELS).map(|c| alea::role(&fj::sound(&p, c))).collect();
    assert_eq!(roles, [Role::Kick, Role::Snare, Role::Tom, Role::Hat]);

    // A throw of the steps, all the way: the kick lane starts its bars.
    let steps = |p: &ModuleParams| (0..CHANNELS * MAX_LEN).map(|i| p.data[data::STEPS + i].get()).collect::<Vec<_>>();
    let original = steps(&p);
    alea::throw_steps(&p, 99, 1.0);
    assert_eq!(fj::step(&p, 0, 0), 3);
    assert_ne!(steps(&p), original);
    // BACK: everything as it was before the throw.
    let sounds = |p: &ModuleParams| (0..CHANNELS).map(|c| fj::sound(p, c)).collect::<Vec<_>>();
    let (s0, st0) = (sounds(&p), steps(&p));
    let before = Before::take(&p);
    alea::throw_sound(&p, 5, 1.0, &[0, 1, 2, 3]);
    alea::throw_steps(&p, 5, 1.0);
    assert!(sounds(&p) != s0 || steps(&p) != st0);
    before.restore(&p);
    assert_eq!((sounds(&p), steps(&p)), (s0, st0));
}

// ---------------------------------------------------------------------------
// Prism (after Pro-Q 4)
// ---------------------------------------------------------------------------

fn prism_params() -> Arc<ModuleParams> {
    Arc::new(ModuleParams::new(ModuleKind::Prism))
}

fn prism() -> fn(&Arc<ModuleParams>) -> crate::audio::modules::prism::Prism {
    |p| crate::audio::modules::prism::Prism::new(p.clone(), 48_000.0)
}

/// A tone's level in the second half of `x` (dB relative to `amp`) and
/// its phase (radians), by correlation over whole cycles.
fn tone(x: &[f32], hz: f32, amp: f32) -> (f32, f32) {
    let start = x.len() / 2;
    let period = 48_000.0 / hz;
    let cycles = ((x.len() - start) as f32 / period).floor();
    let len = (cycles * period).round() as usize;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x[start..start + len].iter().enumerate() {
        let a = std::f64::consts::TAU * hz as f64 * (start + i) as f64 / 48_000.0;
        re += v as f64 * a.sin();
        im += v as f64 * a.cos();
    }
    let mag = 2.0 * (re * re + im * im).sqrt() / len as f64;
    (20.0 * (mag / amp as f64).log10() as f32, im.atan2(re) as f32)
}

fn two_tones(a: (f32, f32), b: (f32, f32)) -> impl Fn(usize) -> (f32, f32) {
    move |n| {
        let t = n as f32 / 48_000.0;
        let v = a.1 * (std::f32::consts::TAU * a.0 * t).sin() + b.1 * (std::f32::consts::TAU * b.0 * t).sin();
        (v, v)
    }
}

#[test]
fn prism_bands_shape_the_sound_in_every_mode() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 1000.0, 6.0, 1.0);
    pq::create_band(&p, 1, LOW_CUT, 60.0, 0.0, 1.0);
    // Linear phase at HIGH: steep low curves need fine frames.
    p.data[data::RESOLUTION].set(2.0);
    for mode in [pq::ZERO_LATENCY, pq::NATURAL_PHASE, pq::LINEAR_PHASE] {
        p.data[data::MODE].set(mode as f32);
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.2), None);
        assert!((tone(&l, 1000.0, 0.2).0 - 6.0).abs() < 0.3, "mode {mode}: bell {}", tone(&l, 1000.0, 0.2).0);
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(250.0, 0.2), None);
        let mid = Analog::design(&Shape { shape: BELL, slope: 1, freq: 1000.0, gain: 6.0, q: 1.0 }).db(250.0) as f32;
        assert!((tone(&l, 250.0, 0.2).0 - mid).abs() < 0.3, "mode {mode}: skirt");
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(30.0, 0.2), None);
        let cut = Analog::design(&Shape { shape: LOW_CUT, slope: 3, freq: 60.0, gain: 0.0, q: 1.0 }).db(30.0) as f32;
        assert!((tone(&l, 30.0, 0.2).0 - cut).abs() < 1.5, "mode {mode}: cut {} vs {cut}", tone(&l, 30.0, 0.2).0);
    }
    // A bypassed band does nothing; an empty Prism passes the input.
    p.data[data::MODE].set(0.0);
    p.data[data::ON].set(0.0);
    p.data[data::USED + 1].set(0.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 9600, sine(1000.0, 0.2), None);
    assert!(tone(&l, 1000.0, 0.2).0.abs() < 0.05);
}

#[test]
fn prism_slopes_and_the_brickwall_cut_as_set() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, HIGH_CUT, 1000.0, 0.0, 1.0);
    for (slope, f, want) in [(1usize, 4000.0, -24.1), (3, 2000.0, -24.1), (8, 1414.0, -48.0)] {
        p.data[data::SLOPE].set(slope as f32);
        let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(f, 0.5), None);
        assert!((tone(&l, f, 0.5).0 - want).abs() < 1.0, "slope {slope}: {}", tone(&l, f, 0.5).0);
    }
    p.data[data::SLOPE].set(BRICKWALL as f32);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(1500.0, 0.5), None);
    assert!(tone(&l, 1500.0, 0.5).0 < -80.0, "brickwall stop band: {}", tone(&l, 1500.0, 0.5).0);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(600.0, 0.5), None);
    assert!(tone(&l, 600.0, 0.5).0.abs() < 0.1, "brickwall pass band");
}

#[test]
fn prism_places_bands_on_stereo_left_right_mid_or_side() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 1000.0, -12.0, 1.0);
    for mode in [pq::ZERO_LATENCY, pq::LINEAR_PHASE] {
        p.data[data::MODE].set(mode as f32);
        p.data[data::PLACE].set(pq::LEFT as f32);
        let (l, r) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.2), None);
        assert!((tone(&l, 1000.0, 0.2).0 + 12.0).abs() < 0.3 && tone(&r, 1000.0, 0.2).0.abs() < 0.2, "mode {mode}: left only");
        p.data[data::PLACE].set(pq::SIDE as f32);
        // A mono signal has no side: untouched.
        let (l, r) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.2), None);
        assert!(tone(&l, 1000.0, 0.2).0.abs() < 0.2 && tone(&r, 1000.0, 0.2).0.abs() < 0.2, "mode {mode}: side leaves mono");
        p.data[data::PLACE].set(pq::MID as f32);
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.2), None);
        assert!((tone(&l, 1000.0, 0.2).0 + 12.0).abs() < 0.3, "mode {mode}: mid");
        // Opposite signals are all side: the mid band leaves them.
        let anti = |n: usize| {
            let v = 0.2 * (std::f32::consts::TAU * 1000.0 * n as f32 / 48_000.0).sin();
            (v, -v)
        };
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, anti, None);
        assert!(tone(&l, 1000.0, 0.2).0.abs() < 0.2, "mode {mode}: mid leaves side");
    }
}

#[test]
fn prism_dynamic_bands_act_above_the_threshold() {
    use crate::audio::modules::prism::{self as pq, band, bp, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 1000.0, 0.0, 1.0);
    p.set(bp(0, band::RANGE), -9.0);
    p.data[data::AUTO].set(0.0);
    p.set(bp(0, band::THRESHOLD), -40.0);
    for mode in [pq::ZERO_LATENCY, pq::NATURAL_PHASE, pq::LINEAR_PHASE] {
        p.data[data::MODE].set(mode as f32);
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.5), None);
        assert!((tone(&l, 1000.0, 0.5).0 + 9.0).abs() < 0.6, "mode {mode}: loud is turned down: {}", tone(&l, 1000.0, 0.5).0);
        let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.002), None);
        assert!(tone(&l, 1000.0, 0.002).0.abs() < 0.3, "mode {mode}: quiet passes");
    }
    // Expansion lifts what crosses.
    p.data[data::MODE].set(0.0);
    p.set(bp(0, band::RANGE), 6.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.2), None);
    assert!((tone(&l, 1000.0, 0.2).0 - 6.0).abs() < 0.6);
    // A tone outside the band's range doesn't trigger it.
    p.set(bp(0, band::RANGE), -9.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 48_000, two_tones((100.0, 0.5), (1000.0, 0.002)), None);
    assert!(tone(&l, 1000.0, 0.002).0.abs() < 1.0, "the band listens to its own range");
    // Automatic threshold: a steady tone settles, a burst over it is caught.
    p.data[data::AUTO].set(1.0);
    let burst = |n: usize| {
        let amp = if n < 72_000 { 0.05 } else { 0.4 };
        let v = amp * (std::f32::consts::TAU * 1000.0 * n as f32 / 48_000.0).sin();
        (v, v)
    };
    let mut pr = prism()(&p);
    let (l, _) = drive_stereo(&mut pr, 81_600, burst, None);
    let steady = tone(&l[48_000..72_000], 1000.0, 0.05).0;
    assert!(steady > -2.5, "a steady tone is mostly left alone: {steady}");
    let caught = tone(&l[72_000..81_600], 1000.0, 0.4).0;
    assert!(caught < -4.0, "the burst is turned down: {caught}");
    assert!(p.viz[pq::viz::DYN].get() < -4.0);
}

#[test]
fn prism_external_side_chain_triggers_the_band() {
    use crate::audio::modules::prism::{self as pq, band, bp, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, LOW_SHELF, 200.0, 0.0, 1.0);
    p.set(bp(0, band::RANGE), -10.0);
    p.data[data::AUTO].set(0.0);
    p.set(bp(0, band::THRESHOLD), -30.0);
    p.data[data::SC_EXT].set(1.0);
    let key = sine(80.0, 0.5);
    let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(60.0, 0.01), Some(&key));
    assert!(tone(&l, 60.0, 0.01).0 < -7.0, "ducked by the key: {}", tone(&l, 60.0, 0.01).0);
    let silent = |_: usize| (0.0, 0.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(60.0, 0.01), Some(&silent));
    assert!(tone(&l, 60.0, 0.01).0 > -0.5);
    // Free range: a trigger range that misses the key leaves the band.
    p.data[data::SC_FREE].set(1.0);
    p.set(bp(0, band::SC_LO), 2000.0);
    p.set(bp(0, band::SC_HI), 8000.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(60.0, 0.01), Some(&key));
    assert!(tone(&l, 60.0, 0.01).0 > -0.5, "free range ignores the key");
}

#[test]
fn prism_spectral_bands_turn_down_only_what_crosses() {
    use crate::audio::modules::prism::{self as pq, band, bp, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 3500.0, 0.0, 0.5);
    p.set(bp(0, band::RANGE), -12.0);
    p.data[data::AUTO].set(0.0);
    p.set(bp(0, band::THRESHOLD), -30.0);
    p.data[data::SPEC_TILT].set(0.0);
    p.set(bp(0, band::DENSITY), 1.0);
    let input = two_tones((3000.0, 0.5), (4200.0, 0.003));
    // An ordinary dynamic band turns the whole band down.
    let (l, _) = drive_stereo(&mut prism()(&p), 72_000, &input, None);
    let (loud, quiet) = (tone(&l, 3000.0, 0.5).0, tone(&l, 4200.0, 0.003).0);
    assert!(loud < -8.0 && quiet < -8.0, "dynamic: {loud} {quiet}");
    // A spectral one only the loud tone.
    p.data[data::SPECTRAL].set(1.0);
    let mut pr = prism()(&p);
    let (l, _) = drive_stereo(&mut pr, 72_000, &input, None);
    let (loud, quiet) = (tone(&l, 3000.0, 0.5).0, tone(&l, 4200.0, 0.003).0);
    assert!(loud < -8.0, "spectral turns the loud tone down: {loud}");
    assert!(quiet.abs() < 1.5, "and leaves the quiet one: {quiet}");
    assert!(p.viz[pq::viz::LATENCY].get() > 1000.0, "spectral bands bring the stage's latency");
    // Its threshold moves live: above the tone, it lets go.
    p.set(bp(0, band::THRESHOLD), 0.0);
    let (l, _) = drive_stereo(&mut pr, 72_000, &input, None);
    assert!(tone(&l, 3000.0, 0.5).0 > -1.5, "threshold raised: {}", tone(&l, 3000.0, 0.5).0);
}

#[test]
fn prism_linear_phase_is_symmetric_and_natural_phase_is_close_to_analog() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 2000.0, 9.0, 2.0);
    p.data[data::MODE].set(pq::LINEAR_PHASE as f32);
    p.data[data::RESOLUTION].set(0.0);
    let mut pr = prism()(&p);
    let (l, _) = drive_stereo(&mut pr, 19_200, |n| if n == 4800 { (1.0, 1.0) } else { (0.0, 0.0) }, None);
    let lat = p.viz[pq::viz::LATENCY].get() as usize;
    let c = 4800 + lat;
    let peak = l.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
    assert_eq!(peak, c, "the impulse lands after the stated latency");
    let asym = (1..1500).map(|k| (l[c + k] - l[c - k]).abs()).fold(0.0f32, f32::max);
    assert!(asym < 2e-3 * l[c].abs(), "linear phase: a symmetric response ({asym})");

    // Phase near the top: Natural Phase follows the analog curve closer
    // than Zero Latency, where the bell's skirt runs past Nyquist.
    p.data[data::USED].set(0.0);
    pq::create_band(&p, 1, BELL, 15_000.0, 12.0, 1.0);
    let analog = Analog::design(&Shape { shape: BELL, slope: 1, freq: 15_000.0, gain: 12.0, q: 1.0 });
    let f = 19_000.0;
    let want = analog.phase(f as f64) as f32;
    let wrap = |x: f32| (x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let mut err = [0.0f32; 2];
    for (k, mode) in [pq::ZERO_LATENCY, pq::NATURAL_PHASE].into_iter().enumerate() {
        p.data[data::MODE].set(mode as f32);
        let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(f, 0.1), None);
        let (gain, ph) = tone(&l, f, 0.1);
        let (_, ph_in) = tone(&(0..24_000).map(|n| sine(f, 0.1)(n).0).collect::<Vec<_>>(), f, 0.1);
        let delay = if mode == pq::NATURAL_PHASE { pq::NATURAL_LATENCY } else { 0.0 };
        let measured = wrap(ph - ph_in + std::f32::consts::TAU * f * delay / 48_000.0);
        err[k] = wrap(measured - want).abs();
        assert!((gain - analog.db(f as f64) as f32).abs() < 1.0, "mode {mode}: magnitude {gain}");
    }
    assert!(err[1] < 0.15, "natural phase: {:.3} rad off (zero latency {:.3})", err[1], err[0]);
    assert!(err[1] < err[0], "natural phase is closer than zero latency ({:.3} vs {:.3})", err[1], err[0]);
}

#[test]
fn prism_output_auto_gain_scale_pan_and_polarity() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 1000.0, 6.0, 1.0);
    p.set(pq::SCALE, 0.5);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(1000.0, 0.2), None);
    assert!((tone(&l, 1000.0, 0.2).0 - 3.0).abs() < 0.3, "gain scale halves the gain");
    p.set(pq::SCALE, 1.0);
    p.set(pq::OUTPUT, -6.0);
    p.data[data::INVERT].set(1.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(1000.0, 0.2), None);
    let (db, ph) = tone(&l, 1000.0, 0.2);
    assert!(db.abs() < 0.3, "+6 band, −6 out: {db}");
    let (_, ph_in) = tone(&(0..24_000).map(|n| sine(1000.0, 0.2)(n).0).collect::<Vec<_>>(), 1000.0, 0.2);
    let bell_phase = Analog::design(&Shape { shape: BELL, slope: 1, freq: 1000.0, gain: 6.0, q: 1.0 }).phase(1000.0) as f32;
    let d = (ph - ph_in - bell_phase).rem_euclid(std::f32::consts::TAU);
    assert!((d - std::f32::consts::PI).abs() < 0.1, "inverted: {d}");
    p.set(pq::OUTPUT, 0.0);
    p.data[data::INVERT].set(0.0);
    // Auto gain takes back what a broad boost adds.
    p.data[data::USED].set(0.0);
    pq::create_band(&p, 1, HIGH_SHELF, 100.0, 6.0, 1.0);
    p.data[data::AUTO_GAIN].set(1.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 48_000, sine(1000.0, 0.2), None);
    let ag = p.viz[pq::viz::AUTO_GAIN].get();
    assert!(ag < -4.0 && ag > -6.5, "auto gain: {ag}");
    assert!((tone(&l, 1000.0, 0.2).0 - (6.0 + ag)).abs() < 0.4);
    // Pan: right, left/right.
    p.data[data::AUTO_GAIN].set(0.0);
    p.set(pq::PAN, 1.0);
    let (l, r) = drive_stereo(&mut prism()(&p), 24_000, sine(1000.0, 0.2), None);
    assert!(tone(&l, 1000.0, 0.2).0 < -60.0 && (tone(&r, 1000.0, 0.2).0 - 6.0).abs() < 0.3);
}

#[test]
fn prism_solo_plays_what_a_band_works_on() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 1000.0, 6.0, 4.0);
    p.data[data::SOLO].set(0.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, two_tones((1000.0, 0.2), (150.0, 0.2)), None);
    assert!(tone(&l, 1000.0, 0.2).0.abs() < 0.5, "the band's frequency passes");
    assert!(tone(&l, 150.0, 0.2).0 < -25.0, "the rest is gone: {}", tone(&l, 150.0, 0.2).0);
    // A low cut's solo plays what it removes.
    p.data[data::USED].set(0.0);
    pq::create_band(&p, 1, LOW_CUT, 500.0, 0.0, 1.0);
    p.data[data::SOLO].set(1.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, two_tones((5000.0, 0.2), (100.0, 0.2)), None);
    assert!(tone(&l, 100.0, 0.2).0.abs() < 0.5 && tone(&l, 5000.0, 0.2).0 < -30.0);
}

#[test]
fn prism_character_adds_harmonics_only_when_asked() {
    use crate::audio::modules::prism::{self as pq, data, design::*};
    let p = prism_params();
    pq::create_band(&p, 0, BELL, 1000.0, 3.0, 1.0);
    let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(500.0, 0.5), None);
    assert!(tone(&l, 1000.0, 0.5).0 < -100.0, "clean");
    p.data[data::CHARACTER].set(2.0);
    for mode in [pq::ZERO_LATENCY, pq::NATURAL_PHASE] {
        p.data[data::MODE].set(mode as f32);
        let (l, _) = drive_stereo(&mut prism()(&p), 24_000, sine(500.0, 0.5), None);
        let h2 = tone(&l, 1000.0, 0.5).0;
        assert!(h2 > -60.0 && h2 < -15.0, "mode {mode}: warm: second harmonic at {h2} dB");
        let bell = Analog::design(&Shape { shape: BELL, slope: 1, freq: 1000.0, gain: 3.0, q: 1.0 }).db(500.0) as f32;
        // Warm rounds the peaks: a dB or so of compression at −5 dBFS.
        assert!((tone(&l, 500.0, 0.5).0 - bell).abs() < 1.5, "the level stays: {} vs {bell}", tone(&l, 500.0, 0.5).0);
        let tail = &l[12_000..];
        let dc = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(dc.abs() < 1e-3, "mode {mode}: no offset ({dc})");
    }
}

#[test]
fn prism_every_shape_slope_and_mode_stays_finite() {
    use crate::audio::modules::prism::{self as pq, band, bp, data, design::*};
    let p = prism_params();
    let noise = |n: usize| {
        let x = ((n as u32).wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
        (x * 0.5, -x * 0.3)
    };
    for mode in [pq::ZERO_LATENCY, pq::NATURAL_PHASE, pq::LINEAR_PHASE] {
        p.data[data::MODE].set(mode as f32);
        for sh in 0..SHAPES {
            for slope in (0..=BRICKWALL).filter(|&s| slope_ok(sh, s)) {
                for (i, f) in [12.0, 900.0, 29_000.0].into_iter().enumerate() {
                    pq::create_band(&p, i, sh, f, 18.0, 30.0);
                    p.data[data::SLOPE + i].set(slope as f32);
                    p.data[data::PLACE + i].set((i * 2 % 5) as f32);
                }
                p.set(bp(0, band::RANGE), -20.0);
                let (l, r) = drive_stereo(&mut prism()(&p), 6_400, noise, None);
                assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 100.0), "mode {mode} shape {sh} slope {slope}");
            }
        }
    }
}

#[test]
fn prism_draws_with_every_kind_of_band() {
    use crate::audio::modules::prism::{self as pq, band, bp, data, design::*};
    use crate::graph::NodeType;
    let (mut g, a, mut e) = setup();
    let id = g.add(NodeType::Module(ModuleKind::Prism), pos2(0.0, 0.0), &a);
    let p = module_params(&g, id);
    for sh in 0..SHAPES {
        pq::create_band(&p, sh, sh, 50.0 * (sh + 1) as f32, 4.0, 1.0);
    }
    p.set(bp(0, band::RANGE), -6.0);
    p.set(bp(1, band::RANGE), 6.0);
    p.data[data::SPECTRAL + 1].set(1.0);
    p.data[data::PLACE + 2].set(pq::SIDE as f32);
    p.data[data::SELECTED].set(3.0);
    p.data[data::PIANO].set(1.0);
    render_buf(&mut e, 4800);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| editor.show(ui, &mut g, &a));
        out.textures_delta.clear();
    }
    let (main, sub) = crate::ui::prism::readout(&p);
    assert_eq!(main, "10 bands");
    assert!(sub.contains("1 dynamic") && sub.contains("1 spectral"), "{sub}");
}
