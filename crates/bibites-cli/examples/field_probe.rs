//! Diagnostic for the request of 2026-11-25 (Improved): what do the streams, the force towers and the whirlpool towers cost a tick, and does a change to how they are
//! stepped leave every result as it was?
//!
//! `cargo run --profile perf --example field_probe -- [--synthetic N | --scenario <zip>] [--streams S] [--forces F] [--whirls W] [--range R] [--width W]
//!  [--warm T] [--ticks T] [--reps R] [--threads K] [--seed N]`
//!
//! It builds a world (a synthetic one of N starter bibites at the benchmark density, or a scenario zip), warms it up, times a stretch of ticks without any field,
//! lays S streams, F force towers and W whirlpool towers over the world, times the same stretch again, then times each of the three steps on its own
//! (`step_force_towers`, `step_whirlpools`, `step_streams`, called again on the finished state) and prints a hash of the whole state (every pellet, bibite and egg: position,
//! velocity, rotation) so that two builds of the core can be compared bit for bit.

use std::hash::{Hash, Hasher};
use std::time::Instant;

use bibites_core::body::{BibiteSpawn, SpawnVitals};
use bibites_core::brain::Brain;
use bibites_core::force_tower::ForceTower;
use bibites_core::genome::{Genome, GenomeMeta};
use bibites_core::improved::Feature;
use bibites_core::math::Vec2;
use bibites_core::settings::SimSettings;
use bibites_core::stream::Stream;
use bibites_core::whirlpool::WhirlpoolTower;
use bibites_core::world::{ScenarioData, TemplateSource, World};

struct Options {
    synthetic: usize,
    scenario: Option<String>,
    streams: usize,
    forces: usize,
    whirls: usize,
    range: f32,
    width: f32,
    warm: u64,
    ticks: u64,
    reps: usize,
    threads: usize,
    seed: u64,
}

fn parse() -> Options {
    let mut options = Options {
        synthetic: 0,
        scenario: None,
        streams: 4,
        forces: 4,
        whirls: 4,
        range: 600.0,
        width: 300.0,
        warm: 200,
        ticks: 100,
        reps: 30,
        threads: 1,
        seed: 7,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut at = 0;
    while at < args.len() {
        let value = |at: usize| args.get(at + 1).unwrap_or_else(|| panic!("{} needs a value", args[at])).clone();
        match args[at].as_str() {
            "--synthetic" => options.synthetic = value(at).parse().expect("--synthetic N"),
            "--scenario" => options.scenario = Some(value(at)),
            "--streams" => options.streams = value(at).parse().expect("--streams S"),
            "--forces" => options.forces = value(at).parse().expect("--forces F"),
            "--whirls" => options.whirls = value(at).parse().expect("--whirls W"),
            "--range" => options.range = value(at).parse().expect("--range R"),
            "--width" => options.width = value(at).parse().expect("--width W"),
            "--warm" => options.warm = value(at).parse().expect("--warm T"),
            "--ticks" => options.ticks = value(at).parse().expect("--ticks T"),
            "--reps" => options.reps = value(at).parse().expect("--reps R"),
            "--threads" => options.threads = value(at).parse().expect("--threads K"),
            "--seed" => options.seed = value(at).parse().expect("--seed N"),
            other => panic!("unknown argument {other}"),
        }
        at += 2;
    }
    options
}

/// The synthetic world of `rustibites bench --bibites N` (the benchmark density), spawned the way the CLI does.
fn synthetic_world(count: usize, seed: u64) -> World {
    let mut settings = SimSettings::game_defaults();
    let scale = (count as f32 / 2_000.0).sqrt();
    settings.independents.simulation_size = (2_000.0 * scale).min(20_000.0);
    let mut world = World::new(settings, ScenarioData::with_default_zone_group(), seed).expect("a world");
    let _ = world.seed_scenario_world();
    for index in 0..count {
        let mut rng = world.rng.fork(0xB1B1_7E50 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let min = world.bounds.min();
        let max = world.bounds.max();
        let margin = (max - min).length() * 0.05;
        let (low, high) = (min + Vec2::splat(margin), max - Vec2::splat(margin));
        let mut brain = Brain::starter();
        brain.reset_runtime_state(world.settings.brain_period());
        let spawn = BibiteSpawn {
            pos: Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y)),
            rot: rng.f32_range(0.0, std::f32::consts::TAU),
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain,
            vitals: SpawnVitals::Adult,
            energy: None,
            health: None,
        };
        let id = world.bibites.spawn(spawn).expect("a bibite");
        world.register_bibite(id);
    }
    world
}

/// A scenario zip as the CLI builds it (the templates the zip carries).
fn scenario_world(path: &str, seed: u64) -> World {
    let bundle = bibites_io::load_scenario_bundle(path).expect("a scenario");
    let templates = bundle.templates;
    let mut world = World::new(bundle.settings, bundle.scenario, seed).expect("a world");
    let _ = world.seed_scenario_world();
    let sources: Vec<TemplateSource> = templates
        .iter()
        .map(|(name, loaded)| {
            TemplateSource::new(name.clone(), loaded.genome.clone(), loaded.meta.clone(), loaded.brain.clone())
                .with_species_name(Some(loaded.game_species_name(name)))
        })
        .collect();
    world.set_templates(sources);
    let _ = world.start_fresh_scenario_spawner();
    world
}

/// A small deterministic generator for the layout of the fields.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32) / ((1_u64 << 24) as f32)
    }
}

fn lay_fields(world: &mut World, options: &Options) {
    let mut rng = Lcg(0x5EED_F1E1_D5 ^ options.seed);
    let (min, max) = (world.bounds.min(), world.bounds.max());
    let mut point = |rng: &mut Lcg| Vec2::new(min.x + (max.x - min.x) * (0.1 + 0.8 * rng.next()), min.y + (max.y - min.y) * (0.1 + 0.8 * rng.next()));
    world.improved.set(Feature::ForceTower, true);
    world.improved.set(Feature::Whirlpool, true);
    world.improved.set(Feature::Streams, true);
    for _ in 0..options.forces {
        let mut tower = ForceTower::new(world.tools.next_save_id(), point(&mut rng));
        tower.set_range(options.range);
        let _ = world.tools.add_force_tower(tower);
    }
    for _ in 0..options.whirls {
        let mut tower = WhirlpoolTower::new(world.tools.next_save_id(), point(&mut rng));
        tower.set_range(options.range);
        let _ = world.tools.add_whirlpool(tower);
    }
    for _ in 0..options.streams {
        let start = point(&mut rng);
        let end = start + Vec2::new(rng.next() - 0.5, rng.next() - 0.5) * ((max.x - min.x) * 0.4);
        let mut stream = Stream::new(start, end);
        stream.set_width(options.width);
        world.streams.push(stream);
    }
}

/// A hash of everything that moves: pellets, bibites, eggs (position, velocity, rotation as bits).
fn full_hash(world: &World) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut put = |hasher: &mut std::collections::hash_map::DefaultHasher, values: &[f32]| {
        for value in values {
            value.to_bits().hash(hasher);
        }
    };
    world.pellets.len().hash(&mut hasher);
    for id in world.pellets.ids() {
        if let Some(pose) = world.pellets.pose(id) {
            put(&mut hasher, &[pose.pos.x, pose.pos.y, pose.velocity.x, pose.velocity.y, pose.rotation, pose.angular_velocity]);
        }
    }
    world.bibites.len().hash(&mut hasher);
    for (_, record) in world.bibites.iter() {
        put(&mut hasher, &[record.pos.x, record.pos.y, record.vel.x, record.vel.y, record.rot]);
    }
    world.eggs.len().hash(&mut hasher);
    for id in world.eggs.ids() {
        if let Some(egg) = world.eggs.get(id) {
            put(&mut hasher, &[egg.pos.x, egg.pos.y, egg.vel.x, egg.vel.y]);
        }
    }
    hasher.finish()
}

fn ms_per_tick(world: &mut World, ticks: u64) -> f64 {
    let start = Instant::now();
    world.run(ticks);
    start.elapsed().as_secs_f64() * 1000.0 / ticks as f64
}

/// Calls `f` `reps` times; returns (mean, best) in microseconds.
fn time_step(reps: usize, mut f: impl FnMut()) -> (f64, f64) {
    let mut total = 0.0;
    let mut best = f64::MAX;
    for _ in 0..reps {
        let start = Instant::now();
        f();
        let micros = start.elapsed().as_secs_f64() * 1e6;
        total += micros;
        best = best.min(micros);
    }
    (total / reps as f64, best)
}

fn main() {
    let options = parse();
    let mut world = match (&options.scenario, options.synthetic) {
        (Some(path), _) => scenario_world(path, options.seed),
        (None, count) if count > 0 => synthetic_world(count, options.seed),
        _ => panic!("give --synthetic N or --scenario <zip>"),
    };
    world.set_threads(options.threads);
    world.run(options.warm);
    println!(
        "world: {} bibites, {} pellets, {} eggs, threads {}",
        world.bibites.len(),
        world.pellets.len(),
        world.eggs.len(),
        world.threads()
    );
    let plain = ms_per_tick(&mut world, options.ticks);
    println!("ticks without a field: {plain:.4} ms/tick");
    lay_fields(&mut world, &options);
    println!("fields: {} streams, {} force towers, {} whirlpool towers (range {}, width {})", options.streams, options.forces, options.whirls, options.range, options.width);
    let with = ms_per_tick(&mut world, options.ticks);
    println!("ticks with the fields:  {with:.4} ms/tick ({:+.4})", with - plain);
    let dt = world.settings.fixed_dt();
    let (mean, best) = time_step(options.reps, || world.step_force_towers(dt));
    println!("step_force_towers: mean {mean:9.1} us, best {best:9.1} us");
    let (mean, best) = time_step(options.reps, || world.step_whirlpools(dt));
    println!("step_whirlpools:   mean {mean:9.1} us, best {best:9.1} us");
    let (mean, best) = time_step(options.reps, || world.step_streams(dt));
    println!("step_streams:      mean {mean:9.1} us, best {best:9.1} us");
    let affected: Vec<u32> = world
        .tools
        .tools()
        .iter()
        .filter_map(|tool| match tool {
            bibites_core::tools::Tool::Force(tower) => Some(tower.affected),
            bibites_core::tools::Tool::Whirl(tower) => Some(tower.affected),
            _ => None,
        })
        .collect();
    println!("bodies moved per tower (last step): {affected:?}");
    // The state, bit for bit: run a fixed stretch more and hash everything.
    world.run(50);
    println!("state hash after 50 more ticks: {:016x}", full_hash(&world));
}
