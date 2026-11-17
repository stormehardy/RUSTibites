//! Seeded evolution benchmark (task-47).
//!
//! Answers "how fast can a population evolve, and does evolution work?" on the
//! current engine: births/deaths, generation growth, species count, mutation
//! counters, population curve and generations per (wall/simulated) hour.
//!
//! The world is the synthetic default ecology with mutation and speciation on,
//! tuned for a sustainable-ish founder population: richer/larger plant zones
//! (`pelletGrowth` at max, 50 zones, bigger pellet energy) and founders spawned
//! **inside** zones so they start near food. The 1k/2k/5k runs still starve down
//! after the first generations (see the population curves); the benchmark
//! reports that honestly rather than hiding it. Everything is deterministic for
//! a fixed seed.
//!
//! Run (release; numbers are only comparable across identical builds/machines):
//!
//! ```powershell
//! cargo run --release -p bibites-core --example evolution_bench
//! # custom runs: population:ticks pairs
//! cargo run --release -p bibites-core --example evolution_bench -- 1000:20000 1000:50000
//! ```

use std::time::Instant;

use bibites_core::body::{BibiteSpawn, SpawnVitals};
use bibites_core::brain::Brain;
use bibites_core::genome::{Genome, GenomeMeta};
use bibites_core::math::Vec2;
use bibites_core::settings::SimSettings;
use bibites_core::sim::state_fingerprint;
use bibites_core::world::{ScenarioData, World};

struct EvolutionReport {
    initial_population: usize,
    ticks: u64,
    seed: u64,
    wall_secs: f64,
    sim_secs: f64,
    ticks_per_second: f64,
    realtime_factor: f64,
    final_population: u32,
    min_population: u32,
    births: u64,
    deaths: u64,
    gen_max: u32,
    gen_mean: f64,
    species_active: usize,
    species_records: usize,
    gene_mutations: u64,
    brain_events: u64,
    synapses_added: u64,
    synapses_removed: u64,
    synapses_toggled: u64,
    nodes_added: u64,
    nodes_removed: u64,
    living_genomes: usize,
    changed_genomes: usize,
    distinct_genomes: usize,
    population_curve: Vec<(u64, u32)>,
    nan_entities: u64,
    fingerprint: u64,
}

fn build_world(initial_population: usize, seed: u64) -> World {
    let mut settings = SimSettings::game_defaults();
    settings.scenario.pellet_energy = 2_000.0;
    settings.scenario.initial_seeding = 2.0;
    settings.independents.biomass_density = 0.02;
    settings.independents.pellet_growth = 0.0005;
    let mut scenario = ScenarioData::with_default_zone_group();
    for group in &mut scenario.zone_groups {
        group.zone_count = 50;
        group.template.radius_relative = 0.1;
        group.template.fertility = 2.0;
        group.template.biomass_density = 2.0;
    }
    let mut world = World::new(settings, scenario, seed).expect("evolution world");
    let _ = world.seed_scenario_world();
    for _ in 0..initial_population {
        let mut rng = world.rng.fork(0xB1B1_7E50);
        // Spawn founders inside a randomly chosen zone so they start near the
        // zone-grown plants (uniform random placement leaves most bibites far
        // from any food). Zones are discs; sample the unit disc by rejection.
        let zone = &world.zones.zones[rng.usize_below(world.zones.zones.len())];
        let radius = zone.radius * 0.9;
        let (dx, dy) = loop {
            let x = rng.f32_range(-1.0, 1.0);
            let y = rng.f32_range(-1.0, 1.0);
            if x * x + y * y <= 1.0 {
                break (x * radius, y * radius);
            }
        };
        let pos = Vec2::new(zone.pos.x + dx, zone.pos.y + dy);
        let spawn = BibiteSpawn {
            pos,
            rot: rng.f32_range(0.0, std::f32::consts::TAU),
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain: Brain::starter(),
            vitals: SpawnVitals::Adult,
            energy: None,
            health: None,
        };
        let id = world.bibites.spawn(spawn).expect("spawn starter bibite");
        world.register_bibite(id);
    }
    world
}

fn run_evolution(initial_population: usize, ticks: u64, seed: u64) -> EvolutionReport {
    let mut world = build_world(initial_population, seed);
    bibites_core::genome::mutation_counters::reset();
    bibites_core::brain::mutation_counters::reset();
    let births_start = world.stats().births_total;
    let deaths_start = world.stats().deaths_total;

    let sample_every = (ticks / 20).max(1);
    let initial_sample = world.sample_stats();
    let mut population_curve = vec![(0_u64, initial_sample.counts.bibites)];
    let mut nan_entities = 0_u64;
    let start = Instant::now();
    for tick in 1..=ticks {
        world.tick();
        if tick % sample_every == 0 {
            let sample = world.sample_stats();
            population_curve.push((tick, sample.counts.bibites));
        }
        for (_, record) in world.bibites.iter() {
            if record.is_corpse || record.body.dead {
                continue;
            }
            let bad = [
                record.pos.x,
                record.pos.y,
                record.vel.x,
                record.vel.y,
                record.rot,
                record.ang_vel,
                record.body.energy.amount,
                record.body.health.amount,
            ]
            .iter()
            .any(|value| !value.is_finite());
            if bad {
                nan_entities += 1;
            }
        }
    }
    let wall_secs = start.elapsed().as_secs_f64();
    let final_sample = world.sample_stats();
    let sim_secs = ticks as f64 / f64::from(world.settings.sim_tps);

    let starter = Genome::starter();
    let mut living_genomes = 0_usize;
    let mut changed_genomes = 0_usize;
    let mut distinct = std::collections::BTreeSet::new();
    for (_, record) in world.bibites.iter() {
        if record.is_corpse || record.body.dead {
            continue;
        }
        living_genomes += 1;
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut changed = false;
        for (index, &gene) in record.genome.values().iter().enumerate() {
            if gene.to_bits() != starter.genes[index].to_bits() {
                changed = true;
            }
            hash ^= u64::from(gene.to_bits());
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        distinct.insert(hash);
        if changed {
            changed_genomes += 1;
        }
    }
    let brain = bibites_core::brain::mutation_counters::snapshot();
    EvolutionReport {
        initial_population,
        ticks,
        seed,
        wall_secs,
        sim_secs,
        ticks_per_second: ticks as f64 / wall_secs.max(f64::MIN_POSITIVE),
        realtime_factor: sim_secs / wall_secs.max(f64::MIN_POSITIVE),
        final_population: final_sample.counts.bibites,
        min_population: population_curve.iter().map(|(_, n)| *n).min().unwrap_or(0),
        births: world.stats().births_total - births_start,
        deaths: world.stats().deaths_total - deaths_start,
        gen_max: final_sample.generation.gen_max,
        gen_mean: f64::from(final_sample.generation.gen_mean),
        species_active: world.species.active_species_count(),
        species_records: world.species.records().len(),
        gene_mutations: bibites_core::genome::mutation_counters::gene_mutations(),
        brain_events: brain.0,
        synapses_added: brain.1,
        synapses_removed: brain.2,
        synapses_toggled: brain.3,
        nodes_added: brain.4,
        nodes_removed: brain.5,
        living_genomes,
        changed_genomes,
        distinct_genomes: distinct.len(),
        population_curve,
        nan_entities,
        fingerprint: state_fingerprint(&world),
    }
}

fn main() {
    let runs: Vec<(usize, u64)> = std::env::args()
        .skip(1)
        .filter_map(|arg| {
            let (population, ticks) = arg.split_once(':')?;
            Some((population.parse().ok()?, ticks.parse().ok()?))
        })
        .collect();
    let runs = if runs.is_empty() {
        vec![
            (1_000_usize, 20_000_u64),
            (2_000, 20_000),
            (5_000, 20_000),
            (1_000, 50_000),
        ]
    } else {
        runs
    };
    println!("evolution benchmark (synthetic default world, mutation+speciation on, release)");
    println!(
        "  ticks   pop   wall   realtime | population          | generation    | lifecycle            | species | mutations                                    | genomes                    | speed"
    );
    for (index, (population, ticks)) in runs.into_iter().enumerate() {
        let seed = 0xE701_1234 + index as u64 * 1_003 + population as u64;
        let report = run_evolution(population, ticks, seed);
        let gens_per_sim_hour = f64::from(report.gen_max) * 3600.0 / report.sim_secs.max(1e-9);
        let gens_per_wall_hour = f64::from(report.gen_max) * 3600.0 / report.wall_secs.max(1e-9);
        println!(
            "  {:>5} {:>5} {:>6.1}s {:>8.1}x ({:>6.0} tps, seed {:#x}) | pop {:>4}->{:>4} (min {:>3}) | gen {:>2} mean {:>5.2} | births {:>5} deaths {:>5} | {:>2}/{:>2}   | g{:>6} b{:>6} (+{}s -{}s ~{}t +{}n -{}n) | {}/{} changed, {} distinct | gen/sim-h {gens_per_sim_hour:>7.1} gen/wall-h {gens_per_wall_hour:>7.1} | fp {:#x}",
            report.ticks,
            report.initial_population,
            report.wall_secs,
            report.realtime_factor,
            report.ticks_per_second,
            report.seed,
            report.initial_population,
            report.final_population,
            report.min_population,
            report.gen_max,
            report.gen_mean,
            report.births,
            report.deaths,
            report.species_active,
            report.species_records,
            report.gene_mutations,
            report.brain_events,
            report.synapses_added,
            report.synapses_removed,
            report.synapses_toggled,
            report.nodes_added,
            report.nodes_removed,
            report.changed_genomes,
            report.living_genomes,
            report.distinct_genomes,
            report.fingerprint,
        );
        println!("    population curve: {:?}", report.population_curve);
        assert_eq!(report.nan_entities, 0, "non-finite simulation state");
    }
}
