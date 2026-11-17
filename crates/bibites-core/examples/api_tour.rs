//! `api_tour` — a runnable tour of the `bibites-core` public API.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p bibites-core --example api_tour
//! cargo test -p bibites-core --example api_tour   # same flow as a smoke test
//! ```
//!
//! The tour stays inside `bibites-core` on purpose: file formats live in
//! `bibites-io` (`docs/API.md` shows the io ↔ core load/save flow, which a core
//! example cannot depend on). It walks the layers from the bottom up:
//!
//! 1. deterministic RNG,
//! 2. generational arenas (how entity identity works),
//! 3. genome + brain (the "what" of a bibite),
//! 4. a `World` built from settings + scenario data (the "where"),
//! 5. spawning, ticking and reading statistics (the "how it runs").
//!
//! Every step asserts its own invariants, so a successful run is a behavioural
//! smoke test of the public surface and of the determinism contract in
//! `docs/ARCHITECTURE.md` §5.

use bibites_core::Vec2;
use bibites_core::arena::Arena;
use bibites_core::brain::{Brain, BrainInputs, DEFAULT_BRAIN_PERIOD, Input, Output};
use bibites_core::genome::{Gene, Genome, GenomeMeta};
use bibites_core::rng::Rng;
use bibites_core::settings::{GrowthAtSpawn, RandomizeGenes, SimSettings, Tagging};
use bibites_core::world::{ScenarioData, TemplateSource, World};

fn main() {
    rng_tour();
    arena_tour();
    genome_and_brain_tour();
    world_tour();
    println!("\napi_tour: all checks passed");
}

/// One authoritative random source: same seed ⇒ same stream; derived streams
/// are independent, which is how parallel work stays reproducible.
fn rng_tour() {
    let mut a = Rng::from_seed(42);
    let mut b = Rng::from_seed(42);
    let seq_a: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
    let seq_b: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
    assert_eq!(seq_a, seq_b, "same seed must produce the same stream");

    // Domain-separated streams: per-entity work can be evaluated in any order.
    let mut e1 = Rng::derive(42, 7, 1);
    let mut e2 = Rng::derive(42, 7, 2);
    assert_ne!(
        e1.next_u64(),
        e2.next_u64(),
        "entities need distinct streams"
    );

    // The helpers used by the simulation (all consume the same stream).
    let mut r = Rng::from_seed(7);
    let u = r.f32();
    assert!((0.0..1.0).contains(&u), "f32() is uniform in [0, 1)");
    let n = r.normal();
    assert!(n.is_finite(), "normal() is finite");
    println!("rng: seq[0..2]={:?} u={u:.4} normal={n:.4}", &seq_a[..2]);
}

/// Entity identity is a generational handle, not a slot index: removing an
/// entity invalidates old handles permanently (no ABA problem).
fn arena_tour() {
    let mut arena: Arena<&'static str> = Arena::new();
    let first = arena.insert("bibite-0");
    let second = arena.insert("bibite-1");
    assert_eq!(arena.len(), 2);
    assert_eq!(arena.get(first), Some(&"bibite-0"));

    let removed = arena.remove(first);
    assert_eq!(removed, Some("bibite-0"));
    assert!(!arena.is_live(first), "removed handle must be stale");
    assert!(arena.is_live(second), "other handles survive");

    // Dense, deterministic iteration order (slot order) for stat sums.
    let alive: Vec<&'static str> = arena.values().copied().collect();
    assert_eq!(alive, vec!["bibite-1"]);
    println!("arena: live={} stale_handle_detected=true", arena.len());
}

/// Genome = the 35 genes; Brain = the NEAT network. Both are plain
/// deterministic value types, so tests and tools can build them directly.
fn genome_and_brain_tour() {
    let genome = Genome::starter();
    assert_eq!(genome.gene(Gene::SizeRatio), 1.0);
    assert_eq!(genome.gene(Gene::Diet), 0.2);
    assert!(genome.growth_at_mature() >= 1.0);

    // A brain tick needs 33 inputs and returns 15 outputs. Senses are written
    // by body/world; here we drive one input by hand.
    let mut brain = Brain::starter();
    let mut inputs = BrainInputs::new();
    inputs.set(Input::EnergyRatio, 0.5);
    let outputs = brain.step(&inputs, DEFAULT_BRAIN_PERIOD);
    let accelerate = outputs.get(Output::Accelerate);
    assert!(accelerate.is_finite());

    // Upkeep is derived from topology (hidden neurons + synapses), charged by
    // the body as a per-second rate.
    let upkeep = brain.upkeep();
    assert_eq!(upkeep.hidden, 0, "starter brain has no hidden nodes");
    assert_eq!(upkeep.synapses, 3, "starter brain has 3 synapses");
    let upkeep_per_second = upkeep.energy_per_second(0.00025, 0.001);
    assert!(upkeep_per_second > 0.0);
    println!(
        "genome: size_ratio=1 growth_at_mature={:.3} | brain: nodes={} synapses={} accel={accelerate:.4} upkeep={upkeep_per_second:.6} E/s",
        genome.growth_at_mature(),
        brain.node_count(),
        brain.synapse_count(),
    );
}

/// The main entry point: settings + scenario data + seed ⇒ a tickable World.
fn world_tour() {
    // `game_defaults()` is the captured 0.6.3.1 settings (30 TPS, mutation on).
    let settings = SimSettings::game_defaults();
    assert_eq!(settings.sim_tps, 30);
    assert!((settings.fixed_dt() - 1.0 / 30.0).abs() < f32::EPSILON);

    // A scenario with the built-in zone group: 10 plant zones round the world.
    let scenario = ScenarioData::with_default_zone_group();
    let mut world = World::new(settings, scenario, 12345).expect("valid settings/scenario");
    assert_eq!(world.bounds.sim_size(), 2000.0);

    // A new scenario seeds its zones with plants (spec 03 §1.6). A loaded save
    // must NOT re-seed, so this is an explicit call rather than part of `new`.
    let seeded = world
        .seed_scenario_world()
        .expect("initial seeding enabled");
    assert!(seeded.spawned > 0, "zones seeded plants");
    let plant_energy: f64 = world.pellets.counts().plant_biomass;
    assert!(plant_energy > 0.0);

    // Templates are plain core values (`bibites-io` builds these from .bb8
    // files). Spawning from a template is the game's virgin-spawn path.
    let template = TemplateSource::new(
        "Basic bibite",
        Genome::starter(),
        GenomeMeta::default(),
        Brain::starter(),
    );
    world.set_templates(vec![template.clone()]);
    let id = world
        .spawn_bibite_from_template(
            &template,
            GrowthAtSpawn::Adult,
            Tagging::NoTagging,
            "",
            Vec2::ZERO,
            RandomizeGenes::No,
        )
        .expect("spawn a bibite");
    if let Some(record) = world.bibites.get_mut(id) {
        record.vel = Vec2::new(10.0, 0.0);
    }

    // Run the fixed-timestep pipeline (17 phases, spec 03 §9.2).
    world.run(300);
    assert!(
        (world.simulated_time() - 10.0).abs() < 1e-3,
        "300 ticks at 30 TPS"
    );

    // Read per-entity state through the copied `BibiteView` (no borrow of the
    // record stays alive) and through the record itself for organs/genes.
    let view = world.bibites.view(id).expect("live bibite");
    assert!(view.d1_size > 0.0);
    assert!(view.health_ratio > 0.0);
    let mut count = 0;
    for (entity, record) in world.bibites.iter() {
        count += 1;
        let _ = (
            entity,
            record.genome.gene(Gene::Diet),
            record.body.energy_ratio(),
        );
    }
    assert_eq!(count, world.bibites.len());

    // Statistics are sampled explicitly (the game's 60 s cadence is a policy
    // choice for the caller; the harness samples every tick).
    let sample = world.sample_stats();
    assert_eq!(sample.tick, 300);
    assert!(sample.counts.bibites >= 1);
    println!(
        "world: t={:.2}s bibites={} eggs={} plant_count={} plant_biomass={:.1} E mean_energy={:.3} E",
        world.simulated_time(),
        world.bibites.len(),
        world.eggs.len(),
        world.pellets.counts().plant_count,
        world.pellets.counts().plant_biomass,
        sample.vitals.energy_mean,
    );

    // Determinism: two worlds with the same (settings, scenario, seed) and the
    // same inputs must produce the same state fingerprint.
    let mut a = build_seeded_world(9_999);
    let mut b = build_seeded_world(9_999);
    a.run(50);
    b.run(50);
    let fa = bibites_core::sim::state_fingerprint(&a);
    let fb = bibites_core::sim::state_fingerprint(&b);
    assert_eq!(fa, fb, "same seed ⇒ same state (ARCHITECTURE §5)");
    println!("world: state_fingerprint={fa:#018x} (deterministic for seed 9999)");
}

/// Builds the same small configured world used by the determinism check.
fn build_seeded_world(seed: u64) -> World {
    let mut world = World::new(
        SimSettings::game_defaults(),
        ScenarioData::with_default_zone_group(),
        seed,
    )
    .expect("valid settings/scenario");
    world.seed_scenario_world();
    let template = TemplateSource::new(
        "Basic bibite",
        Genome::starter(),
        GenomeMeta::default(),
        Brain::starter(),
    );
    world
        .spawn_bibite_from_template(
            &template,
            GrowthAtSpawn::Adult,
            Tagging::NoTagging,
            "",
            Vec2::ZERO,
            RandomizeGenes::No,
        )
        .expect("spawn a bibite");
    world
}

#[cfg(test)]
mod tests {
    #[test]
    fn tour_runs_end_to_end() {
        super::main();
    }
}
