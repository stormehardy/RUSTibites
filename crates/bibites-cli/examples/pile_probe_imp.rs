//! Diagnostic for the glitch report of 2026-11-20 ("glitch1234 has pellets that kill anything it touches, two big piles"): runs a save
//! tick by tick and reports (a) the fastest pellets every `period` ticks, (b) every bibite that dies, with the health it had before, its
//! own speed and the pellets within reach of it (their speeds, amounts and distances), so what kills it can be read off.
//!
//! The Improved edition's copy: the sidecar's whirlpool tower is placed (what the viewer does when it loads the save), unless the sixth argument is `off`.
//!
//! `cargo run --example pile_probe_imp -- <save.zip> <ticks> [seed] [period] [threads] [off]`

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
    if args.get(6).map(String::as_str) != Some("off") {
        use bibites_core::force_tower::ForceFalloff;
        use bibites_core::improved::Feature;
        use bibites_core::math::Vec2;
        use bibites_core::tools::Tool;
        use bibites_core::whirlpool::Spin;
        world.improved = world.improved.with(Feature::Whirlpool, true);
        let id = world.place_whirlpool_at(Vec2::new(0.0, 0.0));
        if let Some(Tool::Whirl(tower)) = world.tools.get_mut(id) {
            tower.set_range(4999.998);
            tower.set_speed(50.21834);
            tower.spin = Spin::Counterclockwise;
            tower.falloff = ForceFalloff::Fading;
            tower.bibites = true;
            tower.pellets = true;
            tower.eggs = true;
        }
        eprintln!("whirlpool tower placed at (0,0): range 4999.998, speed 50.21834, counterclockwise, fading");
    }
    eprintln!(
        "loaded {} bibites, {} eggs, {} pellets; seed {seed}, threads {}",
        world.bibites.len(),
        world.eggs.len(),
        world.pellets.len(),
        world.threads()
    );
    if std::env::var("PILE_DROP").is_ok() {
        // Every living bibite is put into the middle of the big pile, at rest and with plenty of energy (so starving does not muddy what the pellets do).
        use bibites_core::math::Vec2;
        let ids: Vec<_> = world.bibites.iter().filter(|(_, record)| !record.is_corpse).map(|(id, _)| id).collect();
        for (index, id) in ids.iter().enumerate() {
            if let Some(record) = world.bibites.get_mut(*id) {
                record.pos = Vec2::new(100.0 + 60.0 * (index % 5) as f32, 300.0 + 60.0 * (index / 5) as f32);
                record.vel = Vec2::ZERO;
                record.body.energy.amount = 100.0;
            }
        }
        eprintln!("dropped {} living bibites into the pile around (100..340, 300..420)", ids.len());
    }
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
    for tick in 1..=ticks {
        world.tick();
        // The pellets, once a tick: speeds and positions.
        let mut pellets: Vec<(f32, [f32; 2], [f32; 2], f32)> = Vec::with_capacity(world.pellets.len());
        for id in world.pellets.ids() {
            if let Some(view) = world.pellets.get(id) {
                let speed = view.velocity.length();
                pellets.push((speed, [view.pos.x, view.pos.y], [view.velocity.x, view.velocity.y], view.amount));
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
                "tick {tick}: {} bibites, {} pellets, fastest {:.1} u/s at ({:.0},{:.0}) amount {:.1}; over 10 u/s: {over10}, over 50 u/s: {over50}; deaths so far {deaths}",
                world.bibites.len(),
                world.pellets.len(),
                fast.first().map_or(0.0, |p| p.0),
                fast.first().map_or(0.0, |p| p.1[0]),
                fast.first().map_or(0.0, |p| p.1[1]),
                fast.first().map_or(0.0, |p| p.3)
            );
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
