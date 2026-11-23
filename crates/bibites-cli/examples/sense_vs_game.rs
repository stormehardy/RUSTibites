//! Are the port's SENSES exact? One tick of the port, started from the original game's own saved world, against the game's next tick.
//!
//! `tools/braincheck/game_diff/game_brain_ticks.py` (run with `SAVE_WORLDS=1`) steps the game one tick at a time and writes, after the load and after every tick, the whole
//! world as the game saves it (`world_0000.zip`, ...) next to the brain snapshot (`game_0000.json`, ...) and `manifest.json`. For each pair of neighbouring ticks this loads the
//! EARLIER world into the port, runs ONE `World::tick()` and compares, for every bibite the two worlds share (matched by the game's own `body.id`), the 33 INPUT nodes the
//! brain was given (`Value` and the one-tick-old `LastOutput`) with what the game wrote into its later snapshot, bit for bit. The inputs are what `NEATBrain.UpdateSenses` stores:
//! body state (energy, health, stomach, speed, ...), the field of view (`FieldOfView.ComputeSenses`), the clock and the pheromone sense. The brain's own arithmetic is not under
//! test here (that is `brain_vs_game`); the cadence counters (`brainTicksCount`, `visionLookupCount`, `visionSensingCount`) are compared too.
//!
//! The game's sensing reads the world as it stands at the start of the tick (`FixedUpdate` runs before the physics step), so the port starting from that same saved world has nothing
//! to disagree about but the senses themselves. Two things a save cannot give the port, and what this does about them:
//!
//! * Values the game computes at the END of one tick and reads at the start of the next (the speed and turning speed of the movement organ) are not in a save: they are taken from
//!   the port's own tick before (`--no-prime` leaves them out). The angular velocity is not saved at all, so the turning-speed input cannot be compared.
//! * A save holds a bibite's rotation as degrees rounded to a float, the game's own state is the angle in radians: the port's angle (`save.rot.to_radians()`) can be a float step or two
//!   away from the game's. Every angle the senses compute carries that. `--rot-search N` therefore tries each bibite's rotation moved by -N..=N float steps and keeps, for each bibite
//!   and tick, the step that matches the most field-of-view and pheromone inputs (the table shows both: the plain run and the searched one, and how often each step wins). A formula that
//!   is exact matches with some step on every row; a row that no step matches is a real difference.
//!
//! The game's vision cache (the list `FindSeenEntities` found up to `visionLookupUpdateFactor` brain ticks ago) is not saved either: a bibite whose sensing is due while its lookup is not
//! cannot be compared and is counted and left out. (Run the game with `GAME_SETTINGS` setting both factors to 1 and every brain tick is a full sense.)
//!
//! `cargo run --release --example sense_vs_game -- <dir> [--show N] [--warm-up N] [--first K] [--last K] [--inputs a,b,..] [--rot-search N] [--no-prime] [--csv file]`
//! (exit code 1 when any input differs after the search)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bibites_core::brain::{Input, N_INPUTS};
use bibites_core::ids::BibiteId;
use bibites_core::world::World;
use serde_json::Value;

/// A number as the game writes it: Newtonsoft puts a float that is not finite in a string ("Infinity", "-Infinity", "NaN").
fn num(v: &Value) -> f32 {
    match v {
        Value::String(text) => match text.as_str() {
            "Infinity" => f32::INFINITY,
            "-Infinity" => f32::NEG_INFINITY,
            _ => f32::NAN,
        },
        Value::Null => 0.0,
        other => other.as_f64().unwrap_or(0.0) as f32,
    }
}

/// Same number? A negative zero counts as a zero (the game writes `-0.0f` as `0`) and any NaN as any NaN.
fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a == 0.0 && b == 0.0) || (a.is_nan() && b.is_nan())
}

/// Distance in units of the last place (u32::MAX when either is NaN or the numbers are far apart).
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

/// `Value` / `LastOutput` of input node `i` in a snapshot entry.
fn node(entry: &Value, i: usize, key: &str) -> f32 {
    num(&entry["brain"]["Nodes"][i][key])
}

/// `Deg2Rad` as the game's float constant.
const DEG_TO_RAD: f32 = 0.017_453_292;

/// The saved world `world_KKKK.zip` in the port, the game's `body.id` of each bibite, and the port's id of each (in the order of the save).
fn load_world(dir: &Path, k: usize) -> (World, Vec<i32>, Vec<BibiteId>) {
    let bundle = bibites_io::load_save_bundle(dir.join(format!("world_{k:04}.zip"))).expect("load the save");
    let mut contents = bibites_io::world_contents_from_save(&bundle).expect("world contents");
    contents.seed = 1;
    let body_ids = bundle.bibite_ids.clone();
    assert_eq!(body_ids.len(), contents.bibites.len(), "one body id per bibite");
    let positions: Vec<_> = contents.bibites.iter().map(|b| b.pos).collect();
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(1);
    let port_ids: Vec<_> = world.bibites.ids().collect();
    assert_eq!(port_ids.len(), body_ids.len(), "the port's bibites in the save's order");
    for (i, id) in port_ids.iter().enumerate() {
        let record = world.bibites.get(*id).expect("record");
        assert!(record.pos == positions[i], "bibite {i} is not where the save has it: the id order is wrong");
    }
    // The physics angle (`rb2d.r`, degrees, not wrapped to 0..360 like `transform.rotation`) is the closest thing to the game's own radians (`extract_rot.py` writes `rot_KKKK.json`).
    if let Ok(text) = std::fs::read_to_string(dir.join(format!("rot_{k:04}.json"))) {
        let degrees: BTreeMap<String, f64> = serde_json::from_str(&text).expect("rot json");
        for (i, id) in port_ids.iter().enumerate() {
            if let Some(r) = degrees.get(&body_ids[i].to_string()) {
                world.bibites.get_mut(*id).expect("record").rot = (*r as f32) * DEG_TO_RAD;
            }
        }
    }
    (world, body_ids, port_ids)
}

/// Every bibite's rotation moved by `steps` floats (the next float up for a positive step, whichever the sign of the angle).
fn shift_rotations(world: &mut World, ids: &[BibiteId], steps: i32) {
    if steps == 0 {
        return;
    }
    for id in ids {
        let record = world.bibites.get_mut(*id).expect("record");
        let bits = i64::from(record.rot.to_bits());
        let moved = if record.rot >= 0.0 { bits + i64::from(steps) } else { bits - i64::from(steps) };
        record.rot = f32::from_bits(moved as u32);
    }
}

/// What the port holds for one bibite after one tick.
#[derive(Clone, Copy)]
struct PortRow {
    value: [f32; N_INPUTS],
    last_output: [f32; N_INPUTS],
    counters: [u32; 3],
}

/// Loads world `k`, moves every rotation by `steps` floats, hands over the end-of-tick values of the tick before (when `prime`), runs one tick and reads the brains.
///
fn run_round(dir: &Path, k: usize, steps: i32, prime: bool, grown: bool) -> (Vec<i32>, Vec<Option<PortRow>>) {
    let (mut world, body_ids, port_ids) = load_world(dir, k);
    shift_rotations(&mut world, &port_ids, steps);
    if grown {
        // experiment: every bibite already has the size it has one tick later (as if the growth step ran before everybody's senses)
        let (later, later_ids, later_port) = load_world(dir, k + 1);
        let by_body: BTreeMap<i32, BibiteId> = later_ids.iter().copied().zip(later_port.iter().copied()).collect();
        for (i, id) in port_ids.iter().enumerate() {
            let Some(from) = by_body.get(&body_ids[i]).and_then(|e| later.bibites.get(*e)) else { continue };
            let (derived, max_health) = (from.derived, from.body.geometry.max_health);
            let record = world.bibites.get_mut(*id).expect("record");
            record.derived.d1_size = derived.d1_size;
            record.derived.d2_size = derived.d2_size;
            record.derived.base_body_area = derived.base_body_area;
            record.body.geometry.max_health = max_health;
        }
    }
    if prime && k > 0 {
        // What the game computed at the END of the previous tick (`UpdateOrgan`: speed, from the pose before that tick's physics step; `PherosenseAround`: the pheromone sense) and reads at
        // the start of this one is not in a save. The port computes it the same way at the end of its body tick: run the tick before in a world of its own and take the values from there.
        let (mut earlier, earlier_ids, earlier_port) = load_world(dir, k - 1);
        shift_rotations(&mut earlier, &earlier_port, steps);
        earlier.tick();
        let by_body: BTreeMap<i32, BibiteId> = earlier_ids.iter().copied().zip(earlier_port.iter().copied()).collect();
        for (i, id) in port_ids.iter().enumerate() {
            let Some(from) = by_body.get(&body_ids[i]).and_then(|e| earlier.bibites.get(*e)) else { continue };
            let (signed, angular, normalized) = (from.propulsion.signed_velocity, from.propulsion.angular_velocity, from.propulsion.normalized_velocity);
            let pending = from.senses.pending_phero;
            let record = world.bibites.get_mut(*id).expect("record");
            // the pheromone sense the tick before computed (the game computes it at the end of every tick and the next brain tick reads it)
            record.senses.pending_phero = pending;
            record.propulsion.signed_velocity = signed;
            record.propulsion.angular_velocity = angular;
            record.propulsion.normalized_velocity = normalized;
        }
    }
    world.tick();
    let rows = port_ids
        .iter()
        .map(|id| {
            let record = world.bibites.get(*id)?;
            let throttle = world.bibites.sense_throttle(*id)?;
            let mut row = PortRow { value: [0.0; N_INPUTS], last_output: [0.0; N_INPUTS], counters: [throttle.brain_counter, throttle.lookup_counter, throttle.sense_counter] };
            for n in 0..N_INPUTS {
                let state = record.brain.node_state(n)?;
                row.value[n] = state.value;
                row.last_output[n] = state.last_output;
            }
            Some(row)
        })
        .collect();
    (body_ids, rows)
}

/// The field-of-view inputs (the ones the rotation search is judged on: the pheromone sense is computed at another time in the game, see `docs/PARITY_REMAINING.md`).
fn fov_input(n: usize) -> bool {
    (9..=20).contains(&n)
}

/// The inputs that depend on the bibite's rotation: the field of view and the pheromone sense (the speed too, but that one reads the rotation of the tick before).
fn rotation_input(n: usize) -> bool {
    fov_input(n) || (24..=32).contains(&n)
}

#[derive(Default, Clone)]
struct Tally {
    compared: u64,
    differing: u64,
    max_ulps: u32,
}

impl Tally {
    fn add(&mut self, game: f32, port: f32) -> bool {
        self.compared += 1;
        if same(game, port) {
            return true;
        }
        self.differing += 1;
        self.max_ulps = self.max_ulps.max(ulps(game, port));
        false
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(args.get(1).expect("directory with game_NNNN.json, world_NNNN.zip, manifest.json"));
    let mut show = 12_usize;
    let mut warm_up = 5_usize;
    let mut first: Option<usize> = None;
    let mut last: Option<usize> = None;
    let mut only: Option<Vec<usize>> = None;
    let mut csv_path: Option<PathBuf> = None;
    let mut prime = true;
    let mut search = 0_i32;
    let mut grown = false;
    let mut it = args.iter().skip(2);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--show" => show = it.next().expect("N").parse().expect("N"),
            "--warm-up" => warm_up = it.next().expect("N").parse().expect("N"),
            "--first" => first = Some(it.next().expect("K").parse().expect("K")),
            "--last" => last = Some(it.next().expect("K").parse().expect("K")),
            "--rot-search" => search = it.next().expect("N").parse().expect("N"),
            "--no-prime" => prime = false,
            "--grown" => grown = true,
            "--csv" => csv_path = Some(PathBuf::from(it.next().expect("file"))),
            "--inputs" => {
                let names = it.next().expect("names");
                let list: Vec<usize> = names
                    .split(',')
                    .map(|n| (0..N_INPUTS).find(|i| Input::from_index(*i).is_some_and(|x| x.name().eq_ignore_ascii_case(n))).unwrap_or_else(|| panic!("no input named {n}")))
                    .collect();
                only = Some(list);
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).expect("manifest.json")).expect("manifest json");
    let sim_tps = manifest["simTPS"].as_f64().expect("simTPS");
    let brain_tps = manifest["brainTPS"].as_f64().unwrap_or(2.0) as f32;
    let lookup_factor = manifest["visionLookupUpdateFactor"].as_f64().unwrap_or(10.0) as f32;
    let sense_factor = manifest["visionSenseUpdateFactor"].as_f64().unwrap_or(5.0) as f32;
    let mut n_snaps = 0;
    while dir.join(format!("game_{n_snaps:04}.json")).exists() && dir.join(format!("world_{n_snaps:04}.zip")).exists() {
        n_snaps += 1;
    }
    assert!(n_snaps >= 2, "need game_0000.json + world_0000.zip and the next tick");
    println!(
        "{n_snaps} ticks, simTPS {sim_tps}, brainTPS {brain_tps}, vision lookup every {lookup_factor} / sensing every {sense_factor} brain ticks, warm-up {warm_up}, rotation search +-{search}"
    );

    // per input: [plain run (the rotation as the save gives it), searched (the best rotation step of each bibite and tick), not thinking (the inputs must stay as they were)]
    let mut plain: Vec<Tally> = vec![Tally::default(); N_INPUTS];
    let mut searched: Vec<Tally> = vec![Tally::default(); N_INPUTS];
    let mut kept: Vec<Tally> = vec![Tally::default(); N_INPUTS];
    let mut last_output = Tally::default();
    let (mut brain_ticks, mut idle_ticks, mut stale_skipped, mut multi_step, mut gone, mut dead_skipped) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    let mut counters = [Tally::default(), Tally::default(), Tally::default()];
    // rows (a brain tick of a bibite) whose rotation-dependent inputs all match: with the rotation as saved, with the best step; and how often each step wins
    let (mut rows_total, mut rows_plain_exact, mut rows_searched_exact) = (0_u64, 0_u64, 0_u64);
    let mut step_wins: BTreeMap<i32, u64> = BTreeMap::new();
    let mut shown = 0_usize;
    let mut csv = String::new();
    let k_first = first.unwrap_or(warm_up);
    let k_last = last.unwrap_or(n_snaps - 2).min(n_snaps - 2);
    for k in k_first..=k_last {
        let before = load(&dir.join(format!("game_{k:04}.json")));
        let after = load(&dir.join(format!("game_{:04}.json", k + 1)));
        // the round with the rotation as saved first, then the others
        let mut steps: Vec<i32> = vec![0];
        for s in 1..=search {
            steps.push(-s);
            steps.push(s);
        }
        let mut body_ids: Vec<i32> = Vec::new();
        let mut rounds: Vec<(i32, Vec<Option<PortRow>>)> = Vec::new();
        for &s in &steps {
            let (ids, rows) = run_round(&dir, k, s, prime, grown);
            body_ids = ids;
            rounds.push((s, rows));
        }
        for (i, body_id) in body_ids.iter().enumerate() {
            let key = body_id.to_string();
            let (Some(b), Some(a)) = (before.get(&key), after.get(&key)) else {
                gone += 1;
                continue;
            };
            if b["dead"].as_bool().unwrap_or(false) || a["dead"].as_bool().unwrap_or(false) {
                dead_skipped += 1;
                continue;
            }
            // exactly one tick between the two snapshots?
            let ticks = (num(&a["timeAlive"]) - num(&b["timeAlive"])) as f64 * sim_tps;
            if (ticks - 1.0).abs() > 0.05 {
                multi_step += 1;
                continue;
            }
            let Some(plain_row) = rounds[0].1[i] else {
                gone += 1;
                continue;
            };
            let count = |e: &Value, name: &str| -> f32 { num(&e[name]) };
            let (bc, lc, sc) = (count(b, "brainTicksCount"), count(b, "visionLookupCount"), count(b, "visionSensingCount"));
            let thinks = bc + 1.0 >= brain_tps;
            let lookup_due = thinks && lc + 1.0 >= lookup_factor;
            let sense_due = thinks && sc + 1.0 >= sense_factor;
            let stale = sense_due && !lookup_due;
            counters[0].add(count(a, "brainTicksCount"), plain_row.counters[0] as f32);
            counters[1].add(count(a, "visionLookupCount"), plain_row.counters[1] as f32);
            counters[2].add(count(a, "visionSensingCount"), plain_row.counters[2] as f32);
            if thinks {
                brain_ticks += 1;
            } else {
                idle_ticks += 1;
            }
            if stale {
                stale_skipped += 1;
            }
            let game: Vec<f32> = (0..N_INPUTS).map(|n| node(a, n, "Value")).collect();
            let wanted = |n: usize| only.as_ref().is_none_or(|list| list.contains(&n));
            if thinks && !stale {
                // which rotation step matches the most rotation-dependent inputs
                let mismatches = |row: &PortRow| (0..N_INPUTS).filter(|&n| fov_input(n) && wanted(n) && !same(game[n], row.value[n])).count();
                let mut best = (usize::MAX, 0_i32, plain_row);
                for (s, rows) in &rounds {
                    let Some(row) = rows[i] else { continue };
                    let m = mismatches(&row);
                    if m < best.0 || (m == best.0 && s.abs() < best.1.abs()) {
                        best = (m, *s, row);
                    }
                }
                rows_total += 1;
                if mismatches(&plain_row) == 0 {
                    rows_plain_exact += 1;
                }
                if best.0 == 0 {
                    rows_searched_exact += 1;
                }
                *step_wins.entry(if best.0 == 0 { best.1 } else { i32::MAX }).or_insert(0) += 1;
                let chosen = best.2;
                if csv_path.is_some() {
                    use std::fmt::Write as _;
                    let _ = write!(csv, "{k},{key},{},{},{},{}", best.1, u8::from(lookup_due), u8::from(sense_due), best.0);
                    for n in 0..N_INPUTS {
                        let _ = write!(csv, ",{:08x},{:08x},{:08x}", game[n].to_bits(), chosen.value[n].to_bits(), node(b, n, "Value").to_bits());
                    }
                    csv.push('\n');
                }
                for n in 0..N_INPUTS {
                    if !wanted(n) {
                        continue;
                    }
                    plain[n].add(game[n], plain_row.value[n]);
                    // the rotation-dependent inputs come from the best step; the others (body state, clock) are independent of it
                    let port = if rotation_input(n) { chosen.value[n] } else { plain_row.value[n] };
                    let ok = searched[n].add(game[n], port);
                    last_output.add(node(a, n, "LastOutput"), plain_row.last_output[n]);
                    if !ok && shown < show {
                        shown += 1;
                        println!(
                            "  tick {k}->{}: bibite {key} input {n} ({}): game {:e} ({:08x}) port {port:e} ({:08x}) before {:e}; step {} lookup_due {lookup_due} sense_due {sense_due} counters {bc}/{lc}/{sc}",
                            k + 1,
                            Input::from_index(n).map_or("?", |x| x.name()),
                            game[n],
                            game[n].to_bits(),
                            port.to_bits(),
                            node(b, n, "Value"),
                            best.1,
                        );
                    }
                }
            } else if !thinks {
                for n in 0..N_INPUTS {
                    if wanted(n) {
                        kept[n].add(game[n], plain_row.value[n]);
                    }
                }
            }
        }
    }
    if let Some(path) = &csv_path {
        std::fs::write(path, csv).expect("write csv");
    }
    println!(
        "\nbibite-ticks compared: {brain_ticks} brain ticks + {idle_ticks} idle ticks ({stale_skipped} brain ticks whose vision cache the port cannot have, {multi_step} not exactly one tick apart, {dead_skipped} dead, {gone} gone)"
    );
    println!(
        "brain ticks whose {} field-of-view inputs ALL match: {rows_plain_exact} of {rows_total} with the rotation as saved, {rows_searched_exact} with the best rotation step",
        (0..N_INPUTS).filter(|&n| fov_input(n)).count()
    );
    if search > 0 {
        let wins: Vec<String> = step_wins.iter().map(|(s, c)| if *s == i32::MAX { format!("no step: {c}") } else { format!("step {s:+}: {c}") }).collect();
        println!("winning rotation step per brain tick: {}", wins.join(", "));
    }
    println!("\n{:<18} {:>8} {:>8} {:>6}   {:>8} {:>8} {:>6}   {:>8} {:>8} {:>6}", "input", "thinking", "plain", "ulps", "searched", "differ", "ulps", "kept", "differ", "ulps");
    let mut total_differ = 0_u64;
    for n in 0..N_INPUTS {
        let (p, s, kp) = (&plain[n], &searched[n], &kept[n]);
        if p.compared + s.compared + kp.compared == 0 {
            continue;
        }
        total_differ += s.differing + kp.differing;
        println!(
            "{:<18} {:>8} {:>8} {:>6}   {:>8} {:>8} {:>6}   {:>8} {:>8} {:>6}",
            Input::from_index(n).map_or("?", |x| x.name()),
            p.compared,
            p.differing,
            p.max_ulps,
            s.compared,
            s.differing,
            s.max_ulps,
            kp.compared,
            kp.differing,
            kp.max_ulps,
        );
    }
    total_differ += last_output.differing;
    println!("LastOutput of the inputs (the previous Value): {} compared, {} differ", last_output.compared, last_output.differing);
    let names = ["brainTicksCount", "visionLookupCount", "visionSensingCount"];
    for (t, name) in counters.iter().zip(names) {
        total_differ += t.differing;
        println!("counter {name}: {} compared, {} differ", t.compared, t.differing);
    }
    if total_differ == 0 {
        println!("\nEXACT: every compared input and counter is bit-identical to the game's");
    } else {
        println!("\n{total_differ} numbers differ");
        std::process::exit(1);
    }
}
