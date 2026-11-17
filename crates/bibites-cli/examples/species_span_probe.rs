//! Does the species span grow with the number of active species, and does that hold the founding of new species down?
//! (`GlobalLineageManager.UpdateEffectiveSpeciesSpan`: `usedSpeciesSpan = speciesGeneticSpan + max(activeSpecies.Count - 20, 0) *
//! speciesSpanScalingFactor`, read by `CheckNewSpecies` at every birth: a newborn stays in its species while its genetic distance to the
//! species' template is below the used span.)
//!
//! It loads a world, optionally overrides `speciesSpanScalingFactor` (`SCALING`; 0 switches the cost off) and `speciesGeneticSpan` (a small base
//! makes a world found species fast, which is what shows the cost at work), runs it, and every `every` ticks
//! prints the active species, the used span the registry holds, the span the live active count would give, and how many species were
//! founded since the line before per 1,000 births. At the start and the end it prints how far the live bibites are from their species
//! as a fraction of the used span (a bibite past the span is a first outlier waiting for the two-step mutant rule, or one that began
//! beyond it).
//!
//! `cargo run --example species_span_probe -- <save.zip> <ticks> <every> [seed] [threads] [SCALING|-] [BASE SPAN|-]`

use std::path::PathBuf;

use bibites_core::world::World;

/// Highest species id ever given out (ids are sequential and never reused): how many species were founded in all.
fn founded(world: &World) -> u32 {
    world.species.records().iter().map(|record| record.id.value()).max().unwrap_or(0)
}

/// The live bibites' distances to their species as fractions of `span`, in bins `< .25, < .5, < .75, < 1, < 1.5, >= 1.5`.
fn distances(world: &World, span: f32) -> ([u32; 6], f32, u32, u32) {
    let (mut bins, mut worst, mut counted, mut flagged) = ([0_u32; 6], 0.0_f32, 0_u32, 0_u32);
    for (_, record) in world.bibites.iter() {
        if record.body.dead {
            continue;
        }
        let Some(species) = record.meta.species else {
            continue;
        };
        let ratio = world.species.distance_to(species, &record.genome, &record.brain, &world.settings) / span;
        let bin = match ratio {
            r if r < 0.25 => 0,
            r if r < 0.5 => 1,
            r if r < 0.75 => 2,
            r if r < 1.0 => 3,
            r if r < 1.5 => 4,
            _ => 5,
        };
        bins[bin] += 1;
        worst = worst.max(ratio);
        counted += 1;
        if ratio >= 1.0 && record.meta.is_mutant {
            flagged += 1;
        }
    }
    (bins, worst, counted, flagged)
}

fn report_distances(label: &str, world: &World) {
    let span = world.species.current_used_span(&world.settings);
    let (bins, worst, counted, flagged) = distances(world, span);
    println!(
        "# {label}: {counted} live bibites, distance / span ({span:.2}): <.25 {} | <.5 {} | <.75 {} | <1 {} | <1.5 {} | >=1.5 {}; worst {worst:.2}; past the span {} of which first-step mutants {flagged}",
        bins[0], bins[1], bins[2], bins[3], bins[4], bins[5], bins[4] + bins[5]
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).expect("ticks").parse().expect("ticks");
    // (At least 1: the loop below takes the tick modulo it.)
    let every: u64 = args.get(3).expect("every").parse::<u64>().expect("every").max(1);
    let seed: u64 = args.get(4).map_or(1, |s| s.parse().expect("seed"));
    let threads: usize = args.get(5).map_or(0, |s| s.parse().expect("threads"));
    let scaling: Option<f32> = args.get(6).filter(|s| s.as_str() != "-").map(|s| s.parse().expect("scaling"));
    let base_span: Option<f32> = args.get(7).filter(|s| s.as_str() != "-").map(|s| s.parse().expect("base span"));
    let mut contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    if let Some(scaling) = scaling {
        world.settings.independents.species_span_scaling_factor = scaling;
    }
    if let Some(base_span) = base_span {
        world.settings.independents.species_genetic_span = base_span;
    }
    let (base, factor) = (world.settings.independents.species_genetic_span, world.settings.independents.species_span_scaling_factor);
    println!(
        "# {}: {} bibites, {} eggs, {} active species, {} recorded; speciesGeneticSpan {base}, speciesSpanScalingFactor {factor}; used span {:.3}",
        path.display(),
        world.bibites.len(),
        world.eggs.len(),
        world.species.active_species().len(),
        world.species.records().len(),
        world.species.current_used_span(&world.settings),
    );
    world.reconcile_species_counts();
    report_distances("start", &world);
    println!("tick,sim_h,bibites,births,active,alive_species,founded,span_held,span_if_recounted,founded_per_1000_births");
    let (start_founded, start_births) = (founded(&world), world.stats().births_total);
    let (mut last_founded, mut last_births) = (start_founded, start_births);
    for tick in 1..=ticks {
        world.tick();
        if tick % every == 0 {
            let births = world.stats().births_total;
            // (The newest species can be pruned from the tree, which lowers the highest id there is: keep the highest seen.)
            let now_founded = founded(&world).max(last_founded);
            let active = world.species.active_species().len();
            let alive = world.species.records().iter().filter(|record| record.stats.count > 0).count();
            let held = world.species.current_used_span(&world.settings);
            let recounted = base + active.saturating_sub(20) as f32 * factor;
            let new_births = births - last_births;
            let rate = if new_births == 0 { 0.0 } else { f64::from(now_founded - last_founded) * 1000.0 / new_births as f64 };
            println!(
                "{tick},{:.2},{},{births},{active},{alive},{now_founded},{held:.3},{recounted:.3},{rate:.2}",
                world.simulated_time() / 3600.0,
                world.bibites.len(),
            );
            last_founded = now_founded;
            last_births = births;
        }
    }
    world.reconcile_species_counts();
    report_distances("end", &world);
    let made = founded(&world).max(last_founded) - start_founded;
    let born = world.stats().births_total - start_births;
    println!("# founded in the run: {made} species in {born} births ({:.2} per 1,000 births)", f64::from(made) * 1000.0 / born.max(1) as f64);
}
