//! Diagnostic for the glitch report of 2026-11-13: runs a save and stops at the first tick after which a bibite moves in a way no
//! bibite can (speed, spin or distance from the origin far beyond what the arena and the speed limits allow), printing the last
//! ticks of that bibite and of everything near it. The glitch save's NaN bibites were found at 100 million units from the origin.
//!
//! `cargo run --example nan_trace -- <save.zip> <ticks> [seed] [threads] [max_speed] [max_spin] [max_dist]`

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use bibites_core::body::BibiteRecord;
use bibites_core::world::World;

/// (Most fields are only read by the `Debug` print.)
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Snap {
    tick: u64,
    pos: [f32; 2],
    vel: [f32; 2],
    rot: f32,
    ang_vel: f32,
    health: f32,
    energy: f32,
    mass: f32,
    held: u8,
    mouth: u8,
    contacts: u16,
    dead: bool,
    corpse: bool,
    age: f32,
    d2: f32,
}

fn snap(tick: u64, record: &BibiteRecord) -> Snap {
    Snap {
        tick,
        pos: [record.pos.x, record.pos.y],
        vel: [record.vel.x, record.vel.y],
        rot: record.rot,
        ang_vel: record.ang_vel,
        health: record.body.health.amount,
        energy: record.body.energy.amount,
        mass: record.derived.mass,
        held: record.n_held_targets,
        mouth: record.n_mouth_targets,
        contacts: record.last_contact_count,
        dead: record.body.dead,
        corpse: record.is_corpse,
        age: record.clock.time_alive,
        d2: record.body.geometry.d2_size,
    }
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
    let max_speed: f32 = args.get(5).map_or(2000.0, |s| s.parse().expect("max_speed"));
    let max_spin: f32 = args.get(6).map_or(5000.0, |s| s.parse().expect("max_spin"));
    let max_dist: f32 = args.get(7).map_or(100_000.0, |s| s.parse().expect("max_dist"));
    let mut contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    eprintln!("loaded {} bibites; thresholds speed {max_speed} spin {max_spin} dist {max_dist}", world.bibites.len());
    let started = Instant::now();
    const KEEP: usize = 40;
    let mut history: HashMap<u64, VecDeque<Snap>> = HashMap::new();
    for tick in 1..=ticks {
        world.tick();
        let mut trigger: Option<(bibites_core::ids::BibiteId, &'static str)> = None;
        let mut alive_keys: Vec<u64> = Vec::with_capacity(world.bibites.len());
        for (id, record) in world.bibites.iter() {
            let k = key(id);
            alive_keys.push(k);
            let s = snap(tick, record);
            let speed = record.vel.length();
            let dist = record.pos.length();
            if trigger.is_none() {
                if !(speed <= max_speed) {
                    trigger = Some((id, "speed"));
                } else if !(record.ang_vel.abs() <= max_spin) {
                    trigger = Some((id, "spin"));
                } else if !(dist <= max_dist) {
                    trigger = Some((id, "distance"));
                }
            }
            let entry = history.entry(k).or_default();
            entry.push_back(s);
            if entry.len() > KEEP {
                entry.pop_front();
            }
        }
        if tick % 6000 == 0 {
            eprintln!("tick {tick}: {} bibites, sim time {:.0} s, {:.1} ticks/s wall", world.bibites.len(), world.simulated_time(), tick as f64 / started.elapsed().as_secs_f64());
            alive_keys.sort_unstable();
            history.retain(|k, _| alive_keys.binary_search(k).is_ok());
        }
        if let Some((id, what)) = trigger {
            println!("=== tick {tick} (sim time {:.3} s): bibite slot {} gen {} broke the {what} limit ===", world.simulated_time(), id.handle().slot(), id.handle().generation());
            let mine = &history[&key(id)];
            println!("its last {} ticks:", mine.len());
            for s in mine.iter().rev().take(14).rev() {
                println!("  {s:?}");
            }
            // Everything within 150 units of where it was a few ticks ago.
            let anchor = mine.get(mine.len().saturating_sub(4)).map_or([0.0, 0.0], |s| s.pos);
            let mut near: Vec<(f32, u64)> = Vec::new();
            for (id2, record) in world.bibites.iter() {
                if key(id2) == key(id) {
                    continue;
                }
                if let Some(h) = history.get(&key(id2)) {
                    if let Some(s) = h.get(h.len().saturating_sub(4)) {
                        let d = ((s.pos[0] - anchor[0]).powi(2) + (s.pos[1] - anchor[1]).powi(2)).sqrt();
                        if d.is_finite() && d < 150.0 {
                            near.push((d, key(id2)));
                        }
                    }
                }
                let _ = record;
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            println!("{} bibites within 150 units of where it was 4 ticks before; nearest {}:", near.len(), near.len().min(4));
            for (d, k) in near.iter().take(4) {
                println!(" neighbour {k:#x} at distance {d:.1}:");
                for s in history[k].iter().rev().take(8).rev() {
                    println!("    {s:?}");
                }
            }
            return;
        }
    }
    println!("nothing out of the ordinary in {ticks} ticks");
}
