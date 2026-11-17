//! Does a world get slower as the process that runs it ages? (Tester report of 2026-11-13: "I restarted the app and a sim with over 1.1k
//! bibites quadrupled performance"; an earlier one said "performance drops over time, reset gives 3x again".) The age runs of the
//! earlier audits went in segments with a save and a reload between them, which also throws away everything that only grows inside
//! one process. This runs ONE world in ONE process, in chunks, and prints what each chunk cost per bibite-tick next to what could
//! explain a change (population, eggs, pellets, how many store slots are in use, how scattered the bibites are in memory).
//!
//! Every `compare_every` chunks it also does the restart: the same world written out and read back (what a save and a reload do,
//! without the file), and then the aged world and the reloaded one tick on alternating slices, so the machine's drift hits both
//! alike. A ratio above 1 says the aged process is slower than a restart of the same world.
//!
//! `cargo run --example age_probe -- <save.zip> <chunks> <ticks per chunk> [seed] [threads] [compare_every]`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bibites_core::world::World;

/// Runs `ticks` ticks; the wall time and the bibite-ticks (the sum over the ticks of the population) it took.
fn run_chunk(world: &mut World, ticks: u64) -> (Duration, u64) {
    let mut bibite_ticks = 0_u64;
    let started = Instant::now();
    for _ in 0..ticks {
        world.tick();
        bibite_ticks += world.bibites.len() as u64;
    }
    (started.elapsed(), bibite_ticks)
}

/// How scattered the live bibites are in memory, walking the store in the order the sweeps do: the slots in use and the mean slot
/// distance between two consecutive bibites as a fraction of them (0 = in order, 0.33 = a random order).
fn scatter(world: &World) -> (usize, f64) {
    let mut last: Option<i64> = None;
    let (mut sum, mut pairs, mut max_slot) = (0.0_f64, 0_u64, 0_u32);
    for (id, _) in world.bibites.iter() {
        let slot = id.handle().slot();
        max_slot = max_slot.max(slot);
        if let Some(previous) = last {
            sum += (i64::from(slot) - previous).unsigned_abs() as f64;
            pairs += 1;
        }
        last = Some(i64::from(slot));
    }
    let used = max_slot as usize + 1;
    (used, if pairs == 0 { 0.0 } else { sum / pairs as f64 / used as f64 })
}

/// The restart test (see the module comment).
fn compare(world: &mut World, threads: usize) {
    let mut fresh = World::from_contents(world.to_contents()).expect("reload the world");
    fresh.set_threads(threads);
    let (fresh_slots, fresh_scatter) = scatter(&fresh);
    let (mut aged_time, mut aged_n, mut fresh_time, mut fresh_n) = (Duration::ZERO, 0_u64, Duration::ZERO, 0_u64);
    for _ in 0..4 {
        let (time, n) = run_chunk(world, 500);
        aged_time += time;
        aged_n += n;
        let (time, n) = run_chunk(&mut fresh, 500);
        fresh_time += time;
        fresh_n += n;
    }
    let aged = aged_time.as_secs_f64() * 1e6 / aged_n as f64;
    let new = fresh_time.as_secs_f64() * 1e6 / fresh_n as f64;
    println!(
        "# restart test at sim {:.0} s: aged {aged:.3} us/bibite-tick, reloaded {new:.3} us/bibite-tick, aged / reloaded {:.2}x (reloaded: {} bibites, {fresh_slots} slots in use, scatter {fresh_scatter:.3})",
        world.simulated_time(),
        aged / new,
        fresh.bibites.len(),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let chunks: u64 = args.get(2).expect("chunks").parse().expect("chunks");
    let per_chunk: u64 = args.get(3).expect("ticks per chunk").parse().expect("ticks per chunk");
    let seed: u64 = args.get(4).map_or(1, |s| s.parse().expect("seed"));
    let threads: usize = args.get(5).map_or(0, |s| s.parse().expect("threads"));
    let compare_every: u64 = args.get(6).map_or(10, |s| s.parse().expect("compare_every"));
    let mut contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    eprintln!("loaded {} bibites, {} pellets, {} eggs", world.bibites.len(), world.pellets.len(), world.eggs.len());
    println!("chunk,tick,sim_s,bibites,eggs,pellets,slots_in_use,scatter,ms_per_tick,us_per_bibite_tick");
    let mut tick = 0_u64;
    for chunk in 1..=chunks {
        let (wall, bibite_ticks) = run_chunk(&mut world, per_chunk);
        tick += per_chunk;
        let (slots, scattered) = scatter(&world);
        println!(
            "{chunk},{tick},{:.0},{},{},{},{slots},{scattered:.3},{:.3},{:.3}",
            world.simulated_time(),
            world.bibites.len(),
            world.eggs.len(),
            world.pellets.len(),
            wall.as_secs_f64() * 1000.0 / per_chunk as f64,
            wall.as_secs_f64() * 1e6 / bibite_ticks.max(1) as f64,
        );
        if compare_every > 0 && chunk % compare_every == 0 {
            compare(&mut world, threads);
        }
    }
}
