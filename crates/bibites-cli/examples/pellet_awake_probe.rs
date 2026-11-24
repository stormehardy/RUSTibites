//! Diagnostic for the pellet-speed request of 2026-11-23: why do hundreds of pellets stay awake (never settle into the resting layer) in
//! worlds that have been running for thousands of ticks? Loads a save (or a scenario), runs `warm` ticks, then looks at the pellets one tick
//! at a time: which of them change pose bit for bit, how fast they are, whether they are held, and what separates the ones that keep moving
//! from the ones that rest.
//!
//! `cargo run --example pellet_awake_probe -- <save.zip> [warm ticks] [look ticks] [threads]`

use std::collections::HashMap;
use std::path::PathBuf;

use bibites_core::world::World;

fn key(id: bibites_core::ids::PelletId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

fn bucket(speed: f32) -> usize {
    // 0: exactly 0; 1: < 1e-30; 2: < 1e-10; 3: < 1e-4; 4: < 1e-2; 5: < 1; 6: < 10; 7: >= 10
    if speed == 0.0 {
        0
    } else if speed < 1e-30 {
        1
    } else if speed < 1e-10 {
        2
    } else if speed < 1e-4 {
        3
    } else if speed < 1e-2 {
        4
    } else if speed < 1.0 {
        5
    } else if speed < 10.0 {
        6
    } else {
        7
    }
}

const BUCKETS: [&str; 8] = ["0", "<1e-30", "<1e-10", "<1e-4", "<1e-2", "<1", "<10", ">=10"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let warm: u64 = args.get(2).map_or(10, |s| s.parse().expect("warm"));
    let look: u64 = args.get(3).map_or(5, |s| s.parse().expect("look"));
    let threads: usize = args.get(4).map_or(1, |s| s.parse().expect("threads"));
    let contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    eprintln!(
        "loaded {} bibites, {} eggs, {} pellets, threads {}",
        world.bibites.len(),
        world.eggs.len(),
        world.pellets.len(),
        world.threads()
    );
    for _ in 0..warm {
        world.tick();
    }
    let stats = world.pellet_sleep_stats();
    eprintln!("after {warm} ticks: asleep {} awake {}", stats.asleep, stats.awake);
    // Follow three pellets whose only change is the rotation, for a few ticks: the values themselves.
    {
        let mut first: HashMap<u64, [u32; 6]> = HashMap::new();
        for id in world.pellets.ids() {
            if let Some(pose) = world.pellets.pose(id) {
                first.insert(
                    key(id),
                    [
                        pose.pos.x.to_bits(),
                        pose.pos.y.to_bits(),
                        pose.velocity.x.to_bits(),
                        pose.velocity.y.to_bits(),
                        pose.rotation.to_bits(),
                        pose.angular_velocity.to_bits(),
                    ],
                );
            }
        }
        world.tick();
        let mut picked: Vec<bibites_core::ids::PelletId> = Vec::new();
        for id in world.pellets.ids() {
            let (Some(pose), Some(old)) = (world.pellets.pose(id), first.get(&key(id))) else {
                continue;
            };
            let same_pos = pose.pos.x.to_bits() == old[0] && pose.pos.y.to_bits() == old[1];
            let same_vel = pose.velocity.x.to_bits() == old[2] && pose.velocity.y.to_bits() == old[3];
            if same_pos && same_vel && pose.rotation.to_bits() != old[4] && picked.len() < 3 {
                picked.push(id);
            }
        }
        for id in &picked {
            let pose = world.pellets.pose(*id).expect("pose");
            println!(
                "follow {:?}: amount {:?} pos ({}, {}) rot {:e} ang_vel {:e} (bits rot {:08x} ang {:08x})",
                key(*id),
                world.pellets.amount(*id),
                pose.pos.x,
                pose.pos.y,
                pose.rotation,
                pose.angular_velocity,
                pose.rotation.to_bits(),
                pose.angular_velocity.to_bits()
            );
        }
        for step in 0..8 {
            world.tick();
            let line: Vec<String> = picked
                .iter()
                .map(|id| {
                    let pose = world.pellets.pose(*id).expect("pose");
                    format!(
                        "rot {:e} ({:08x}) ang {:e} vel ({:e},{:e})",
                        pose.rotation,
                        pose.rotation.to_bits(),
                        pose.angular_velocity,
                        pose.velocity.x,
                        pose.velocity.y
                    )
                })
                .collect();
            println!("  step {step}: {}", line.join(" | "));
        }
    }
    for round in 0..look {
        let mut before: HashMap<u64, [u32; 6]> = HashMap::with_capacity(world.pellets.len());
        for id in world.pellets.ids() {
            if let Some(pose) = world.pellets.pose(id) {
                before.insert(
                    key(id),
                    [
                        pose.pos.x.to_bits(),
                        pose.pos.y.to_bits(),
                        pose.velocity.x.to_bits(),
                        pose.velocity.y.to_bits(),
                        pose.rotation.to_bits(),
                        pose.angular_velocity.to_bits(),
                    ],
                );
            }
        }
        world.tick();
        let mut changed = 0usize;
        let mut by_speed = [0usize; 8];
        let mut moved_pos = 0usize;
        let mut vel_only = 0usize;
        let mut rot_only = 0usize;
        let mut held = 0usize;
        let mut ang_nonzero_still = 0usize;
        let mut total = 0usize;
        let mut kinds: HashMap<&'static str, usize> = HashMap::new();
        for id in world.pellets.ids() {
            total += 1;
            let Some(pose) = world.pellets.pose(id) else {
                continue;
            };
            let Some(old) = before.get(&key(id)) else {
                continue;
            };
            let now = [
                pose.pos.x.to_bits(),
                pose.pos.y.to_bits(),
                pose.velocity.x.to_bits(),
                pose.velocity.y.to_bits(),
                pose.rotation.to_bits(),
                pose.angular_velocity.to_bits(),
            ];
            if now == *old {
                continue;
            }
            changed += 1;
            by_speed[bucket(pose.velocity.length())] += 1;
            let pos_same = now[0] == old[0] && now[1] == old[1];
            if !pos_same {
                moved_pos += 1;
            } else if now[4] == old[4] && now[5] == old[5] {
                vel_only += 1;
            } else {
                rot_only += 1;
            }
            if world.pellets.is_held(id) {
                held += 1;
            }
            if pose.velocity.length() == 0.0 && pose.angular_velocity != 0.0 {
                ang_nonzero_still += 1;
            }
            let kind = match world.pellets.material(id) {
                Some(bibites_core::pellet::MaterialKind::Plant) => "plant",
                Some(bibites_core::pellet::MaterialKind::Meat) => "meat",
                _ => "other",
            };
            *kinds.entry(kind).or_default() += 1;
        }
        println!(
            "round {round}: {changed} of {total} pellets changed: position moved {moved_pos}, only velocity {vel_only}, only rotation {rot_only}, held {held}, still-but-spinning {ang_nonzero_still}; by speed after the step {}; kinds {:?}",
            BUCKETS
                .iter()
                .zip(by_speed)
                .map(|(name, count)| format!("{name}:{count}"))
                .collect::<Vec<_>>()
                .join(" "),
            kinds
        );
    }
}
