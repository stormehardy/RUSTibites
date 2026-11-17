//! Diagnostic for the glitch report of 2026-11-13 (a long run grew 264,000 bibites whose positions were NaN): runs a save tick by
//! tick and stops at the first tick after which a bibite, an egg or a pellet holds a number that is not finite, printing what the
//! bibite and its neighbours looked like the tick before.
//!
//! `cargo run --example nan_hunt -- <save.zip> <ticks> [seed] [threads]`

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use bibites_core::body::BibiteRecord;
use bibites_core::world::World;

/// What one bibite looked like at the end of a tick.
/// (Most fields are only read by the `Debug` print.)
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Snap {
    pos: [f32; 2],
    vel: [f32; 2],
    rot: f32,
    ang_vel: f32,
    health: f32,
    energy: f32,
    d2: f32,
    mass: f32,
    inertia: f32,
    force: [f32; 2],
    torque: f32,
    held: u8,
    mouth: u8,
    dead: bool,
    age: f32,
    outputs: [f32; 15],
}

fn snap(record: &BibiteRecord) -> Snap {
    Snap {
        pos: [record.pos.x, record.pos.y],
        vel: [record.vel.x, record.vel.y],
        rot: record.rot,
        ang_vel: record.ang_vel,
        health: record.body.health.amount,
        energy: record.body.energy.amount,
        d2: record.body.geometry.d2_size,
        mass: record.derived.mass,
        inertia: record.derived.inertia,
        force: [record.pending_force.x, record.pending_force.y],
        torque: record.pending_torque,
        held: record.n_held_targets,
        mouth: record.n_mouth_targets,
        dead: record.body.dead,
        age: record.clock.time_alive,
        outputs: record.brain.output_values_copy(),
    }
}

fn bad_fields(record: &BibiteRecord) -> Vec<&'static str> {
    let mut bad = Vec::new();
    let mut check = |name: &'static str, value: f32| {
        if !value.is_finite() {
            bad.push(name);
        }
    };
    check("pos.x", record.pos.x);
    check("pos.y", record.pos.y);
    check("vel.x", record.vel.x);
    check("vel.y", record.vel.y);
    check("rot", record.rot);
    check("ang_vel", record.ang_vel);
    check("health", record.body.health.amount);
    check("energy", record.body.energy.amount);
    check("d2_size", record.body.geometry.d2_size);
    check("mass", record.derived.mass);
    check("pending_force.x", record.pending_force.x);
    check("pending_torque", record.pending_torque);
    bad
}

fn key(id: bibites_core::ids::BibiteId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).expect("ticks").parse().expect("ticks");
    let seed: u64 = args.get(3).map_or(1, |s| s.parse().expect("seed"));
    let threads: usize = args.get(4).map_or(0, |s| s.parse().expect("threads"));
    let mut contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    eprintln!(
        "loaded {} bibites, {} eggs, {} pellets; seed {seed}, threads {}",
        world.bibites.len(),
        world.eggs.len(),
        world.pellets.len(),
        world.threads()
    );
    let started = Instant::now();
    let mut previous: HashMap<u64, Snap> = HashMap::new();
    for (id, record) in world.bibites.iter() {
        previous.insert(key(id), snap(record));
    }
    let mut already_bad = 0usize;
    for tick in 1..=ticks {
        world.tick();
        let mut found: Vec<(bibites_core::ids::BibiteId, Vec<&'static str>)> = Vec::new();
        let mut current: HashMap<u64, Snap> = HashMap::with_capacity(world.bibites.len());
        for (id, record) in world.bibites.iter() {
            let bad = bad_fields(record);
            if !bad.is_empty() && !previous.get(&key(id)).is_some_and(|s| !s.pos[0].is_finite()) {
                found.push((id, bad));
            }
            current.insert(key(id), snap(record));
        }
        let mut egg_bad = 0usize;
        for id in world.eggs.ids() {
            if let Some(egg) = world.eggs.get(id) {
                if !egg.pos.is_finite() || !egg.hatch.energy.is_finite() {
                    egg_bad += 1;
                }
            }
        }
        if tick % 3000 == 0 {
            eprintln!(
                "tick {tick}: {} bibites, {} eggs, {} pellets, sim time {:.0} s, {:.1} ticks/s wall, non-finite bibites so far {already_bad}",
                world.bibites.len(),
                world.eggs.len(),
                world.pellets.len(),
                world.simulated_time(),
                tick as f64 / started.elapsed().as_secs_f64()
            );
        }
        if !found.is_empty() || egg_bad > 0 {
            println!("=== tick {tick} (sim time {:.3} s): {} new non-finite bibites, {egg_bad} bad eggs ===", world.simulated_time(), found.len());
            for (id, bad) in found.iter().take(3) {
                println!("bibite slot {} gen {}: non-finite {bad:?}", id.handle().slot(), id.handle().generation());
                match previous.get(&key(*id)) {
                    Some(before) => println!("  the tick before: {before:#?}"),
                    None => println!("  (born this tick: it was not in the previous tick)"),
                }
                if let Some(record) = world.bibites.get(*id) {
                    println!("  now: {:#?}", snap(record));
                    println!(
                        "  genes: size_ratio {} growth_scale {} ; brain nodes {} ; egg layer: progress {} capacity {} ; corpse {}",
                        record.growth_genes.size_ratio,
                        record.growth_genes.growth_scale,
                        record.brain.node_count(),
                        record.egg_layer.progress,
                        record.egg_layer.capacity,
                        record.is_corpse
                    );
                }
                if let Some(before) = previous.get(&key(*id)) {
                    let mut near: Vec<(f32, u64)> = previous
                        .iter()
                        .filter(|(other, _)| **other != key(*id))
                        .map(|(other, s)| (((s.pos[0] - before.pos[0]).powi(2) + (s.pos[1] - before.pos[1]).powi(2)).sqrt(), *other))
                        .filter(|(d, _)| d.is_finite() && *d < 250.0)
                        .collect();
                    near.sort_by(|a, b| a.0.total_cmp(&b.0));
                    println!("  {} neighbours within 250 units the tick before; nearest:", near.len());
                    for (d, other) in near.iter().take(4) {
                        let s = &previous[other];
                        println!("    d {d:.1}: pos {:?} vel {:?} mass {} dead {} held {} mouth {}", s.pos, s.vel, s.mass, s.dead, s.held, s.mouth);
                    }
                }
            }
            return;
        }
        // Bibites that were already non-finite before are counted, not reported again.
        already_bad = current.values().filter(|s| !s.pos[0].is_finite()).count();
        previous = current;
    }
    println!("no non-finite number in {ticks} ticks ({} bibites at the end)", world.bibites.len());
}
