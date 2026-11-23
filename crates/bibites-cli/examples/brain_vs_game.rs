//! Is the port's brain MATH exact? Replays every brain tick of the ORIGINAL game in `Brain::step` and compares each node bit for bit.
//!
//! `tools/braincheck/game_diff/game_brain_ticks.py` runs the game one tick at a time and writes the brain of every bibite before the first tick and after each one
//! (`game_0000.json`, `game_0001.json`, ..., `manifest.json`). For each bibite and each pair of neighbouring ticks this builds a `Brain` from the earlier state
//! (`Value` / `LastInput` / `LastOutput` of every node, synapses in the game's order), takes the input nodes' new `Value`s (what `UpdateSenses` wrote, the senses are the
//! game's own here) as the tick's inputs, steps it once with the game's brain period, and compares all three numbers of every node with the later state.
//! The game thinks when `brainTicksCount + 1 >= brainTPS`; a tick where it does not must leave the brain untouched (checked too).
//!
//! The one thing this does not check is where the inputs come from (sensing); that is the port's senses against the game's (`docs/PARITY_REMAINING.md`).
//!
//! `cargo run --release --example brain_vs_game -- <dir of game_NNNN.json> [--show N] [--warm-up N]`   (exit code 1 when any node differs)

use std::collections::BTreeMap;
use std::path::PathBuf;

use bibites_core::brain::{Brain, BrainInputs, Input, NodeRecord, NodeRuntimeState, NodeType, SynapseRecord, N_INPUTS};
use serde_json::Value;

/// A number of a node as the game writes it: Newtonsoft puts a float that is not finite in a string ("Infinity", "-Infinity", "NaN").
fn f(v: &Value, key: &str) -> f32 {
    match v.get(key) {
        Some(Value::String(text)) => match text.as_str() {
            "Infinity" => f32::INFINITY,
            "-Infinity" => f32::NEG_INFINITY,
            _ => f32::NAN,
        },
        Some(number) => number.as_f64().unwrap_or(0.0) as f32,
        None => 0.0,
    }
}

fn records(brain: &Value) -> (Vec<NodeRecord>, Vec<SynapseRecord>) {
    let nodes = brain["Nodes"]
        .as_array()
        .expect("Nodes")
        .iter()
        .enumerate()
        .map(|(i, n)| NodeRecord {
            node_type: NodeType::from_u8(n["Type"].as_u64().expect("Type") as u8).expect("node type"),
            index: n.get("Index").and_then(Value::as_u64).unwrap_or(i as u64) as u32,
            innovation: n.get("Inov").and_then(Value::as_i64).unwrap_or(0),
            desc: None,
            base_activation: f(n, "baseActivation"),
            value: f(n, "Value"),
            last_input: f(n, "LastInput"),
            last_output: f(n, "LastOutput"),
        })
        .collect();
    let synapses = brain["Synapses"]
        .as_array()
        .expect("Synapses")
        .iter()
        .map(|s| SynapseRecord {
            innovation: s.get("Inov").and_then(Value::as_i64).unwrap_or(0),
            node_in: s["NodeIn"].as_u64().expect("NodeIn") as u32,
            node_out: s["NodeOut"].as_u64().expect("NodeOut") as u32,
            weight: f(s, "Weight"),
            enabled: s.get("En").and_then(Value::as_bool).unwrap_or(true),
        })
        .collect();
    (nodes, synapses)
}

/// Same number? A negative zero counts as a zero (the game writes `-0.0f` as `0` in its JSON: old Mono prints no sign, so a saved zero tells nothing about its sign) and any NaN as any NaN (the JSON keeps no payload).
fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a == 0.0 && b == 0.0) || (a.is_nan() && b.is_nan())
}

/// Distance in units of the last place between two floats (u32::MAX when either is NaN or the signs differ over a gap).
fn ulps(a: f32, b: f32) -> u32 {
    if a.is_nan() || b.is_nan() {
        return if a.to_bits() == b.to_bits() { 0 } else { u32::MAX };
    }
    let key = |x: f32| {
        let bits = x.to_bits() as i32;
        if bits < 0 { i32::MIN.wrapping_sub(bits) } else { bits }
    };
    (i64::from(key(a)) - i64::from(key(b))).unsigned_abs().min(u64::from(u32::MAX)) as u32
}

fn load(path: &PathBuf) -> BTreeMap<String, Value> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let v: Value = serde_json::from_str(&text).expect("json");
    v.as_object().expect("object").iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}


/// `--sums`: which way of adding up a node's incoming `weight * lastOutput` terms gives the game's `LastInput` (the full activation) bit for bit? Every node that has
/// incoming synapses is summed from the game's own numbers (a source that runs earlier in the same tick counts with the value the game gave it) in four ways.
static DEBUG_MULT: std::sync::Mutex<u32> = std::sync::Mutex::new(0);

fn probe_sums(before: &Value, after: &Value, herding_enabled: bool, tally: &mut BTreeMap<(u8, &'static str), (u64, u64)>) {
    let (nodes_b, synapses) = records(&before["brain"]);
    let (nodes_a, _) = records(&after["brain"]);
    let mut order: Vec<u32> = Vec::new();
    for s in &synapses {
        if s.enabled && (herding_enabled || s.node_out != 2) && !order.contains(&s.node_out) {
            order.push(s.node_out);
        }
    }
    let mut done = vec![false; nodes_b.len()];
    for &target in &order {
        let t = target as usize;
        let lo = |i: usize, done: &Vec<bool>| -> f32 {
            if done[i] { nodes_a[i].value } else if i < N_INPUTS { nodes_b[i].value } else { nodes_b[i].last_output }
        };
        let kind = nodes_b[t].node_type;
        let bias = nodes_b[t].base_activation;
        let mut s0 = if kind == NodeType::Mult { 1.0_f32 } else { 0.0 };
        let mut s1 = s0;
        let mut s2 = f64::from(s0);
        let mut s3 = s0;
        for syn in synapses.iter().filter(|s| s.enabled && s.node_out == target) {
            let l = lo(syn.node_in as usize, &done);
            let term = syn.weight * l;
            if kind == NodeType::Mult {
                s0 *= term;
                s1 *= term;
                s2 *= f64::from(syn.weight) * f64::from(l);
                s3 *= term;
            } else {
                s0 += term;
                s1 = syn.weight.mul_add(l, s1);
                s2 += f64::from(syn.weight) * f64::from(l);
                s3 = ((f64::from(s3) + f64::from(syn.weight) * f64::from(l)) as f32);
            }
        }
        let with_bias = |x: f32| -> f32 {
            match kind {
                NodeType::Mult => x * bias,
                NodeType::Integrator | NodeType::Inhibitory | NodeType::SoftLatch => x,
                _ => x + bias,
            }
        };
        let theirs = nodes_a[t].last_input;
        if kind == NodeType::Mult && !same(with_bias(s0), theirs) && *DEBUG_MULT.lock().expect("lock") < 3 {
            *DEBUG_MULT.lock().expect("lock") += 1;
            println!("  Mult node {t} bias {:e} (bits {:08x}): port x {:e} ({:08x}) game x {:e} ({:08x}); terms:", bias, bias.to_bits(), with_bias(s0), with_bias(s0).to_bits(), theirs, theirs.to_bits());
            for syn in synapses.iter().filter(|s| s.enabled && s.node_out == target) {
                let l = lo(syn.node_in as usize, &done);
                println!("      from {:>3} weight {:e} x lastOutput {:e} (bits {:08x}) = {:e}", syn.node_in, syn.weight, l, l.to_bits(), syn.weight * l);
            }
        }
        for (name, x) in [("f32 sequential", with_bias(s0)), ("fused multiply-add", with_bias(s1)), ("f64 sum, one rounding", with_bias(s2 as f32)), ("f64 each step, f32 store", with_bias(s3))] {
            let e = tally.entry((kind as u8, name)).or_default();
            e.0 += 1;
            if same(x, theirs) {
                e.1 += 1;
            }
        }
        done[t] = true;
    }
}


/// `--acts`: the activation functions alone. Every node that ran is given the game's own sum (its `LastInput`, bias included) and its own previous state, and the output
/// is compared with the game's `Value`; nothing computed earlier in the tick is used, so a difference here is the function's, not a cascade.
fn probe_acts(before: &Value, after: &Value, period: f32, herding_enabled: bool, tally: &mut BTreeMap<u8, (u64, u64, u32)>, shown: &mut usize, show: usize, dump: &mut Option<std::fs::File>) {
    let (nodes_b, synapses) = records(&before["brain"]);
    let (nodes_a, _) = records(&after["brain"]);
    let mut order: Vec<u32> = Vec::new();
    for s in &synapses {
        if s.enabled && (herding_enabled || s.node_out != 2) && !order.contains(&s.node_out) {
            order.push(s.node_out);
        }
    }
    for &target in &order {
        let t = target as usize;
        let kind = nodes_b[t].node_type;
        let bias = nodes_b[t].base_activation;
        // `execute_neuron` takes the sum before the bias; hand it the game's full activation with the bias taken off again where it is added.
        let x_game = nodes_a[t].last_input;
        let (out, x) = bibites_core::brain::execute_neuron_from_activation(kind, bias, x_game, nodes_b[t].last_output, nodes_b[t].last_input, period);
        let theirs = nodes_a[t].value;
        if let Some(file) = dump.as_mut() {
            use std::io::Write;
            // raw sum before the bias goes along, so a rule that rounds the bias add differently can be tried too
            let _ = writeln!(file, "{{\"kind\":{},\"bias\":{},\"x\":{},\"prev_out\":{},\"prev_in\":{},\"out\":{},\"period\":{}}}", kind as u8, nodes_b[t].base_activation.to_bits(), x_game.to_bits(), nodes_b[t].last_output.to_bits(), nodes_b[t].last_input.to_bits(), theirs.to_bits(), period.to_bits());
        }
        let e = tally.entry(kind as u8).or_default();
        e.0 += 1;
        if same(out, theirs) && same(x, x_game) {
            e.1 += 1;
        } else {
            e.2 = e.2.max(ulps(out, theirs));
            if *shown < show {
                *shown += 1;
                println!(
                    "  {:?} bias {:e} x {:e} (bits {:08x}) prev out {:e} prev in {:e}: port {:e} ({:08x})  game {:e} ({:08x})  ulps {}",
                    kind, bias, x_game, x_game.to_bits(), nodes_b[t].last_output, nodes_b[t].last_input, out, out.to_bits(), theirs, theirs.to_bits(), ulps(out, theirs)
                );
            }
        }
    }
}

#[derive(Default)]
struct TypeStats {
    compared: u64,
    wrong: u64,
    max_ulps: u32,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().expect("usage: brain_vs_game <dir> [--show N]"));
    let show: usize = args.iter().position(|a| a == "--show").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(8);
    // The first steps after a load run with another time step than the settings' (two FixedUpdates of 1/30 s where one of 1/15 s was asked for): left out.
    let warm_up: usize = args.iter().position(|a| a == "--warm-up").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(5);
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).expect("manifest.json")).expect("manifest json");
    let sim_tps = manifest["simTPS"].as_f64().expect("simTPS") as f32;
    let brain_tps = manifest["brainTPS"].as_f64().expect("brainTPS") as f32;
    let herding_enabled = !manifest["disableHerding"].as_bool().unwrap_or(false);
    let period = brain_tps / sim_tps;
    println!("simTPS {sim_tps}  brainTPS {brain_tps}  period {period}  herding enabled {herding_enabled}");

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("game_") && n.ends_with(".json")))
        .collect();
    files.sort();

    let acts = args.iter().any(|a| a == "--acts");
    let mut act_tally: BTreeMap<u8, (u64, u64, u32)> = BTreeMap::new();
    let mut act_shown = 0_usize;
    let mut dump: Option<std::fs::File> = args.iter().position(|a| a == "--dump").and_then(|i| args.get(i + 1)).map(|p| std::fs::File::create(p).expect("dump file"));
    let sums = args.iter().any(|a| a == "--sums");
    let mut sum_tally: BTreeMap<(u8, &'static str), (u64, u64)> = BTreeMap::new();
    let mut by_type: BTreeMap<u8, TypeStats> = BTreeMap::new();
    let (mut steps, mut steps_wrong, mut idle, mut idle_wrong, mut cadence_wrong, mut skipped, mut multi_step) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    let mut shown = 0_usize;
    let mut previous = load(&files[0]);
    for (tick, file) in files.iter().enumerate().skip(1) {
        let current = load(file);
        if tick <= warm_up {
            previous = current;
            continue;
        }
        for (id, before) in &previous {
            let Some(after) = current.get(id) else { skipped += 1; continue };
            if before["dead"].as_bool().unwrap_or(false) || after["dead"].as_bool().unwrap_or(false) {
                skipped += 1;
                continue;
            }
            // The harness asks for one tick; the game sometimes runs two (`run_start` stops on a time, not a tick): only single steps are compared.
            let lived = (after["timeAlive"].as_f64().unwrap_or(0.0) - before["timeAlive"].as_f64().unwrap_or(0.0)) * f64::from(sim_tps);
            if (lived - 1.0).abs() > 0.02 {
                multi_step += 1;
                continue;
            }
            let (nodes_b, synapses_b) = records(&before["brain"]);
            let (nodes_a, _) = records(&after["brain"]);
            if nodes_b.len() != nodes_a.len() {
                skipped += 1;
                continue;
            }
            // Does the game think in this tick? (`brainTicksCount += 1; if >= brainTickFactor: Thinker()`)
            let counter_before = before["brainTicksCount"].as_f64().unwrap_or(0.0) as f32;
            let counter_after = after["brainTicksCount"].as_f64().unwrap_or(0.0) as f32;
            let thinks = counter_before + 1.0 >= brain_tps;
            let expect_after = if thinks { counter_before + 1.0 - brain_tps } else { counter_before + 1.0 };
            if (counter_after - expect_after).abs() > 1e-4 {
                cadence_wrong += 1;
            }
            if acts && thinks {
                probe_acts(before, after, period, herding_enabled, &mut act_tally, &mut act_shown, show, &mut dump);
            }
            if sums && thinks {
                probe_sums(before, after, herding_enabled, &mut sum_tally);
            }
            let mut brain = Brain::from_records(&nodes_b, &synapses_b).expect("brain");
            brain.set_herding_enabled(herding_enabled);
            brain.rebuild();
            if thinks {
                let mut inputs = BrainInputs::new();
                for i in 0..N_INPUTS {
                    inputs.set(Input::from_index(i).expect("input"), nodes_a[i].value);
                }
                let _ = brain.step(&inputs, period);
                steps += 1;
            } else {
                idle += 1;
            }
            let mut wrong_nodes = Vec::new();
            for (i, node) in nodes_a.iter().enumerate() {
                let state = brain.node_state(i).expect("node state");
                let kind = nodes_b[i].node_type as u8;
                let stats = by_type.entry(kind).or_default();
                let pairs = [(state.value, node.value), (state.last_input, node.last_input), (state.last_output, node.last_output)];
                let mut wrong = false;
                for (mine, theirs) in pairs {
                    stats.compared += 1;
                    if !same(mine, theirs) {
                        stats.wrong += 1;
                        stats.max_ulps = stats.max_ulps.max(ulps(mine, theirs));
                        wrong = true;
                    }
                }
                if wrong {
                    wrong_nodes.push((i, kind, state, NodeRuntimeState { value: node.value, last_input: node.last_input, last_output: node.last_output }));
                }
            }
            if !wrong_nodes.is_empty() {
                if thinks { steps_wrong += 1 } else { idle_wrong += 1 }
                if shown < show {
                    shown += 1;
                    println!("tick {tick} bibite {id} ({}): {} node(s) differ", if thinks { "thought" } else { "did not think" }, wrong_nodes.len());
                    for (i, kind, mine, theirs) in wrong_nodes.iter().take(6) {
                        println!(
                            "    node {i} {:?} bias {:e}: port value {:e} lastIn {:e} lastOut {:e}  game value {:e} lastIn {:e} lastOut {:e}   (before: value {:e} lastIn {:e} lastOut {:e})",
                            NodeType::from_u8(*kind).expect("kind"),
                            nodes_b[*i].base_activation,
                            mine.value, mine.last_input, mine.last_output,
                            theirs.value, theirs.last_input, theirs.last_output,
                            nodes_b[*i].value, nodes_b[*i].last_input, nodes_b[*i].last_output,
                        );
                    }
                }
            }
        }
        previous = current;
    }

    println!("\nbrain ticks replayed: {steps}  (wrong: {steps_wrong})   ticks without a brain tick checked: {idle} (wrong: {idle_wrong})   counter schedule wrong: {cadence_wrong}   skipped: {skipped} (dead, born or gone)   left out, the game ran more than one tick: {multi_step}");
    println!("{:<14} {:>12} {:>10} {:>10}", "node type", "numbers", "differ", "max ulps");
    for (kind, s) in &by_type {
        println!("{:<14} {:>12} {:>10} {:>10}", NodeType::from_u8(*kind).map_or("?", NodeType::name), s.compared, s.wrong, s.max_ulps);
    }
    if acts {
        println!("
the activation functions alone, from the game's own sums (matches / nodes, largest difference in ulps):");
        for (kind, (n, ok, max)) in &act_tally {
            println!("  {:<12} {:>7} / {:<7} {:>10}", NodeType::from_u8(*kind).map_or("?", NodeType::name), ok, n, max);
        }
    }
    if sums {
        println!("
the game's LastInput (the full activation) reproduced by each way of summing (matches / nodes):");
        for ((kind, name), (n, ok)) in &sum_tally {
            println!("  {:<12} {:<26} {:>7} / {:<7}", NodeType::from_u8(*kind).map_or("?", NodeType::name), name, ok, n);
        }
    }
    if steps_wrong + idle_wrong > 0 {
        std::process::exit(1);
    }
}
