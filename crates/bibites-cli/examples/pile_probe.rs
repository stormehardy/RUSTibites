//! Diagnostic for the glitch report of 2026-11-20 ("glitch1234 has pellets that kill anything it touches, two big piles"): runs a save
//! tick by tick and reports (a) the fastest pellets every `period` ticks, (b) every bibite that dies, with the health it had before, its
//! own speed and the pellets within reach of it (their speeds, amounts and distances), so what kills it can be read off.
//!
//! `cargo run --example pile_probe -- <save.zip> <ticks> [seed] [period] [threads]`

use std::collections::HashMap;
use std::path::PathBuf;

use bibites_core::world::World;

#[derive(Clone, Copy)]
struct Before {
    pos: [f32; 2],
    vel: [f32; 2],
    health: f32,
    energy: f32,
    length: f32,
    mass: f32,
    corpse: bool,
}

fn key(id: bibites_core::ids::BibiteId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).expect("ticks").parse().expect("ticks");
    let seed: u64 = args.get(3).map_or(1, |s| s.parse().expect("seed"));
    let period: u64 = args.get(4).map_or(50, |s| s.parse().expect("period"));
    let threads: usize = args.get(5).map_or(1, |s| s.parse().expect("threads"));
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
    let mut previous: HashMap<u64, Before> = HashMap::new();
    for (id, record) in world.bibites.iter() {
        previous.insert(
            key(id),
            Before {
                pos: [record.pos.x, record.pos.y],
                vel: [record.vel.x, record.vel.y],
                health: record.body.health.amount,
                energy: record.body.energy.amount,
                length: record.derived.body_length,
                mass: record.derived.mass,
                corpse: record.is_corpse,
            },
        );
    }
    let mut deaths = 0usize;
    let (mut fast_pellet_lines, mut fast_bibite_lines) = (0usize, 0usize);
    let mut last_pellet_pos: HashMap<u64, [f32; 2]> = HashMap::new();
    for tick in 1..=ticks {
        world.tick();
        // The pellets, once a tick: speeds and positions.
        let mut pellets: Vec<(f32, [f32; 2], [f32; 2], f32)> = Vec::with_capacity(world.pellets.len());
        let mut held_total = 0usize;
        // (speed, pos, velocity, amount, held, moved since the last tick) of the pellets over 100 u/s.
        let mut fast_now: Vec<(f32, [f32; 2], [f32; 2], f32, bool, f32)> = Vec::new();
        let mut pos_now: HashMap<u64, [f32; 2]> = HashMap::with_capacity(world.pellets.len());
        for id in world.pellets.ids() {
            if let Some(view) = world.pellets.get(id) {
                if view.held {
                    held_total += 1;
                }
                let speed = view.velocity.length();
                let handle = (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot());
                if speed > 100.0 {
                    let moved = last_pellet_pos
                        .get(&handle)
                        .map_or(-1.0, |before| ((before[0] - view.pos.x).powi(2) + (before[1] - view.pos.y).powi(2)).sqrt());
                    fast_now.push((speed, [view.pos.x, view.pos.y], [view.velocity.x, view.velocity.y], view.amount, view.held, moved));
                }
                pos_now.insert(handle, [view.pos.x, view.pos.y]);
                pellets.push((speed, [view.pos.x, view.pos.y], [view.velocity.x, view.velocity.y], view.amount));
            }
        }
        last_pellet_pos = pos_now;
        // `TRACE="first-last@x,y,r"`: every body inside the disc, every tick of the window (what a runaway is made of).
        if let Ok(spec) = std::env::var("TRACE") {
            if let Some((window, disc)) = spec.split_once('@') {
                let w: Vec<u64> = window.split('-').filter_map(|x| x.trim().parse().ok()).collect();
                let d: Vec<f32> = disc.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if w.len() == 2 && d.len() == 3 && tick >= w[0] && tick <= w[1] {
                    let inside = |x: f32, y: f32| ((x - d[0]).powi(2) + (y - d[1]).powi(2)).sqrt() < d[2];
                    println!("--- trace tick {tick}");
                    for (id, record) in world.bibites.iter() {
                        if inside(record.pos.x, record.pos.y) {
                            println!(
                                "  B{} pos ({:.2},{:.2}) vel ({:.1},{:.1}) rot {:.1} spin {:.2} mass {:.3} len {:.2} corpse {} held {} mouth {} force ({:.2},{:.2}) torque {:.2}",
                                id.handle().slot(), record.pos.x, record.pos.y, record.vel.x, record.vel.y, record.rot, record.ang_vel, record.derived.mass, record.derived.body_length,
                                record.is_corpse, record.n_held_targets, record.n_mouth_targets, record.pending_force.x, record.pending_force.y, record.pending_torque
                            );
                        }
                    }
                    for id in world.pellets.ids() {
                        if let Some(view) = world.pellets.get(id) {
                            if inside(view.pos.x, view.pos.y) && (view.velocity.length() > 30.0 || view.held) {
                                println!(
                                    "  P{} pos ({:.2},{:.2}) vel ({:.1},{:.1}) spin {:.2} rot {:.2} amount {:.2} radius {:.2} held {}",
                                    id.handle().slot(), view.pos.x, view.pos.y, view.velocity.x, view.velocity.y,
                                    world.pellets.angular_velocity(id).unwrap_or(0.0), world.pellets.rotation(id).unwrap_or(0.0), view.amount, view.radius, view.held
                                );
                            }
                        }
                    }
                }
            }
        }
        for f in &fast_now {
            if fast_pellet_lines < 60 {
                fast_pellet_lines += 1;
                let neighbours = pellets.iter().filter(|p| ((p.1[0] - f.1[0]).powi(2) + (p.1[1] - f.1[1]).powi(2)).sqrt() < 12.0).count() - 1;
                let mut near_bibite = String::from("none within 60");
                let mut best = 60.0f32;
                for (_, record) in world.bibites.iter() {
                    let d = ((record.pos.x - f.1[0]).powi(2) + (record.pos.y - f.1[1]).powi(2)).sqrt();
                    if d < best {
                        best = d;
                        near_bibite = format!("d {d:.1} speed {:.1} corpse {} holding {}", record.vel.length(), record.is_corpse, record.n_held_targets);
                    }
                }
                println!(
                    "FASTPELLET tick {tick} at ({:.0},{:.0}) speed {:.0} vel ({:.0},{:.0}) amount {:.1} held {} moved {:.2} u this tick, {neighbours} pellets within 12 u, nearest bibite: {near_bibite}",
                    f.1[0], f.1[1], f.0, f.2[0], f.2[1], f.3, f.4, f.5
                );
            }
        }
        let mut current: HashMap<u64, Before> = HashMap::with_capacity(world.bibites.len());
        let mut dead_now: Vec<(u64, Before, Option<Before>)> = Vec::new();
        for (id, record) in world.bibites.iter() {
            let now = Before {
                pos: [record.pos.x, record.pos.y],
                vel: [record.vel.x, record.vel.y],
                health: record.body.health.amount,
                energy: record.body.energy.amount,
                length: record.derived.body_length,
                mass: record.derived.mass,
                corpse: record.is_corpse,
            };
            let own_speed = (now.vel[0].powi(2) + now.vel[1].powi(2)).sqrt();
            if own_speed > 150.0 && fast_bibite_lines < 40 {
                fast_bibite_lines += 1;
                println!("FASTBIBITE tick {tick} at ({:.0},{:.0}) speed {:.0} corpse {} mass {:.2}", now.pos[0], now.pos[1], own_speed, now.corpse, now.mass);
            }
            if let Some(before) = previous.get(&key(id)) {
                if now.corpse && !before.corpse {
                    dead_now.push((key(id), *before, Some(now)));
                }
                if !before.corpse && !now.corpse && (now.health - before.health < -1.0 || now.energy - before.energy < -3.0) {
                    let reach = before.length * 0.5 + 12.0;
                    let touching: Vec<&(f32, [f32; 2], [f32; 2], f32)> = pellets
                        .iter()
                        .filter(|p| ((p.1[0] - now.pos[0]).powi(2) + (p.1[1] - now.pos[1]).powi(2)).sqrt() < reach)
                        .collect();
                    let fastest = touching.iter().map(|p| p.0).fold(0.0f32, f32::max);
                    let own = (before.vel[0].powi(2) + before.vel[1].powi(2)).sqrt();
                    println!(
                        "HURT tick {tick}: at ({:.0},{:.0}) health {:.2} -> {:.2}, energy {:.2} -> {:.2}, own speed {:.1}, {} pellets within {:.0} units (fastest {:.1} u/s)",
                        now.pos[0], now.pos[1], before.health, now.health, before.energy, now.energy, own, touching.len(), reach, fastest
                    );
                }
            }
            current.insert(key(id), now);
        }
        for (k, before) in &previous {
            if !current.contains_key(k) && !before.corpse {
                dead_now.push((*k, *before, None));
            }
        }
        if tick % period == 0 || tick == 1 {
            let mut fast: Vec<&(f32, [f32; 2], [f32; 2], f32)> = pellets.iter().collect();
            fast.sort_by(|a, b| b.0.total_cmp(&a.0));
            let over50 = pellets.iter().filter(|p| p.0 > 50.0).count();
            let over10 = pellets.iter().filter(|p| p.0 > 10.0).count();
            println!(
                "tick {tick}: {} bibites, {} pellets, fastest {:.1} u/s at ({:.0},{:.0}) amount {:.1}; over 10 u/s: {over10}, over 50 u/s: {over50}; held pellets {held_total}; deaths so far {deaths}",
                world.bibites.len(),
                world.pellets.len(),
                fast.first().map_or(0.0, |p| p.0),
                fast.first().map_or(0.0, |p| p.1[0]),
                fast.first().map_or(0.0, |p| p.1[1]),
                fast.first().map_or(0.0, |p| p.3)
            );
        }
        if (tick % period == 0 || tick == 1) && std::env::var("PILE_AREAS").is_ok() {
            // `PILE_AREAS="x,y,r;x,y,r"`: what is inside each area now (pellets and their mean speed, live bibites, corpses).
            let spec = std::env::var("PILE_AREAS").unwrap_or_default();
            for (index, area) in spec.split(';').enumerate() {
                let v: Vec<f32> = area.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 3 {
                    continue;
                }
                let inside = |x: f32, y: f32| ((x - v[0]).powi(2) + (y - v[1]).powi(2)).sqrt() < v[2];
                let in_pellets: Vec<&(f32, [f32; 2], [f32; 2], f32)> = pellets.iter().filter(|p| inside(p.1[0], p.1[1])).collect();
                let mean_speed = in_pellets.iter().map(|p| p.0).sum::<f32>() / in_pellets.len().max(1) as f32;
                let amount: f32 = in_pellets.iter().map(|p| p.3).sum();
                let (mut alive, mut corpses) = (0, 0);
                for (_, record) in world.bibites.iter() {
                    if inside(record.pos.x, record.pos.y) {
                        if record.is_corpse {
                            corpses += 1;
                        } else {
                            alive += 1;
                        }
                    }
                }
                println!(
                    "  area {index} ({:.0},{:.0},r {:.0}): {} pellets (amount {:.0}, mean speed {:.2}), {alive} alive, {corpses} corpses",
                    v[0], v[1], v[2], in_pellets.len(), amount, mean_speed
                );
            }
        }
        for (_, before, after) in &dead_now {
            deaths += 1;
            let speed = (before.vel[0].powi(2) + before.vel[1].powi(2)).sqrt();
            println!(
                "DEATH tick {tick}: at ({:.1},{:.1}) health before {:.2} mass {:.3} own speed {:.1} ({}); then:",
                before.pos[0],
                before.pos[1],
                before.health,
                before.mass,
                speed,
                if after.is_some() { "left a corpse" } else { "removed" }
            );
            let mut near: Vec<(f32, &(f32, [f32; 2], [f32; 2], f32))> = pellets
                .iter()
                .map(|p| (((p.1[0] - before.pos[0]).powi(2) + (p.1[1] - before.pos[1]).powi(2)).sqrt(), p))
                .filter(|(d, _)| *d < 40.0)
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            println!("   {} pellets within 40 units; nearest:", near.len());
            for (d, p) in near.iter().take(4) {
                println!("   d {d:.1}: speed {:.1} vel ({:.1},{:.1}) amount {:.1}", p.0, p.2[0], p.2[1], p.3);
            }
        }
        previous = current;
    }
    println!("{ticks} ticks done: {} bibites at the end, {deaths} bibites died", world.bibites.len());
}
