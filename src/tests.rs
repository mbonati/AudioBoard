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
        let shape = widgets::cable_shape(from.output_pos(c.from.port), to.input_pos(c.to.port), 1.0);
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

use crate::audio::engine::{Inputs, MidiBuf, ProcessCtx, Processor};

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
        bufs: [[[0.0; 64]; 2]; 8],
        connected: [false; 8],
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
    assert!(shows(&texts, "3"), "a folded category shows its count");
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
        bufs: [[[0.0; 64]; 2]; 8],
        connected: [false; 8],
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
        bufs: [[[0.0; 64]; 2]; 8],
        connected: [false; 8],
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
    let mut inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; 8], connected: [false; 8], midi: MidiBuf::default(), midi_connected: false });
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

/// A Frakture with only the given effects on and nothing mapped.
fn frakture_with(sections: &[usize]) -> Arc<ModuleParams> {
    use crate::audio::modules::frakture::{self, data};
    let p = Arc::new(ModuleParams::new(ModuleKind::Frakture));
    for s in 0..4 {
        p.data[data::SECTION_ON + s].set(if sections.contains(&s) { 1.0 } else { 0.0 });
    }
    for i in 0..frakture::MAPPABLE {
        frakture::set_mapping(&p, i, None);
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
    let mut inputs = Box::new(Inputs { bufs: [[[0.0; 64]; 2]; 8], connected: [false; 8], midi: MidiBuf::default(), midi_connected: false });
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
    let mut inputs = Box::new(Inputs { bufs: [[[0.1; 64]; 2]; 8], connected: [false; 8], midi: MidiBuf::default(), midi_connected: true });
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
