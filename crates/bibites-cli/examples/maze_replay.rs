//! Replays a few ticks of the glitch of 2026-11-15 (a bibite that has just grabbed things next to a giant pellet is thrown at 400 u/s)
//! from a checkpoint, so a fix can be tried in seconds instead of after 8,800 ticks.
//!
//! `maze_replay snap <save.zip> <tick> <out.state> [seed]`   runs the save to `tick` and writes the world's checkpoint
//! `maze_replay play <save.zip> <in.state> <ticks> <x> <y> [seed]`   restores it and runs `ticks` more ticks, printing what is within 60
//! units of (x, y) every tick and every bibite or pellet that moves faster than the physics' own limits

use std::path::PathBuf;

use bibites_core::ids::{BibiteId, PelletId};
use bibites_core::math::Vec2;
use bibites_core::world::World;

fn bkey(id: BibiteId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

fn pkey(id: PelletId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

fn load(path: &PathBuf, seed: u64) -> World {
    let mut contents = bibites_io::load_world_contents(path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(1);
    world
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("", String::as_str);
    let save = PathBuf::from(args.get(2).expect("save path"));
    match mode {
        "snap" => {
            let tick: u64 = args.get(3).expect("tick").parse().expect("tick");
            let out = PathBuf::from(args.get(4).expect("out file"));
            let seed: u64 = args.get(5).map_or(1, |s| s.parse().expect("seed"));
            let mut world = load(&save, seed);
            for _ in 0..tick {
                world.tick();
            }
            let snapshot = world.capture_checkpoint_state().expect("capture");
            let bytes = snapshot.encode().expect("encode");
            std::fs::write(&out, &bytes).expect("write");
            println!("tick {tick}: {} bibites, {} pellets; checkpoint of {} bytes written to {}", world.bibites.len(), world.pellets.len(), bytes.len(), out.display());
        }
        "play" => {
            let state = PathBuf::from(args.get(3).expect("state file"));
            let ticks: u64 = args.get(4).expect("ticks").parse().expect("ticks");
            let x: f32 = args.get(5).expect("x").parse().expect("x");
            let y: f32 = args.get(6).expect("y").parse().expect("y");
            let seed: u64 = args.get(7).map_or(1, |s| s.parse().expect("seed"));
            let centre = Vec2::new(x, y);
            let mut world = load(&save, seed);
            let bytes = std::fs::read(&state).expect("read the checkpoint");
            let snapshot = bibites_core::sim::SimSnapshot::decode(&bytes).expect("decode");
            world.restore_checkpoint_state(&snapshot).expect("restore");
            println!("restored: {} bibites, {} pellets", world.bibites.len(), world.pellets.len());
            let mut worst_bibite = 0.0f32;
            let mut worst_spin = 0.0f32;
            for step in 1..=ticks {
                world.tick();
                println!("---- +{step} ticks:");
                for (id, record) in world.bibites.iter() {
                    let d = (record.pos - centre).length();
                    let speed = record.vel.length();
                    let spin = record.ang_vel.abs();
                    if d < 60.0 || speed > 200.0 || spin > 50.0 {
                        println!(
                            "   bibite {:#x} at ({:.1}, {:.1}) vel ({:.1}, {:.1}) speed {:.1} spin {:.2} health {:.1} held {} mouth {} contacts {}",
                            bkey(id),
                            record.pos.x,
                            record.pos.y,
                            record.vel.x,
                            record.vel.y,
                            speed,
                            record.ang_vel,
                            record.body.health.amount,
                            record.n_held_targets,
                            record.n_mouth_targets,
                            record.last_contact_count
                        );
                    }
                    if d < 60.0 && record.n_held_targets > 0 {
                        for tag in &record.held_targets[..record.n_held_targets as usize] {
                            let what = if let Some(pid) = bibites_core::body::pellet_id_from_tag(*tag) {
                                world.pellets.get(pid).map_or("a pellet that is gone".to_owned(), |v| {
                                    format!("{:?} pellet amount {:.2} at ({:.1}, {:.1}) vel ({:.1}, {:.1})", v.material, v.amount, v.pos.x, v.pos.y, v.velocity.x, v.velocity.y)
                                })
                            } else if bibites_core::body::bibite_id_from_tag(*tag).is_some() {
                                "a bibite".to_owned()
                            } else {
                                "something else".to_owned()
                            };
                            println!("      holds tag {tag:#x}: {what}");
                        }
                    }
                    worst_bibite = worst_bibite.max(speed);
                    worst_spin = worst_spin.max(spin);
                }
                for id in world.pellets.ids() {
                    if let (Some(view), Some(pose)) = (world.pellets.get(id), world.pellets.pose(id)) {
                        let d = (view.pos - centre).length();
                        if (d < 40.0 && view.amount <= 5_000.0) || view.velocity.length() > 150.0 {
                            println!(
                                "   pellet {:?} amount {:.2} at ({:.2}, {:.2}) vel ({:.1}, {:.1}) spin {:.2} id {:#x}",
                                view.material,
                                view.amount,
                                view.pos.x,
                                view.pos.y,
                                view.velocity.x,
                                view.velocity.y,
                                pose.angular_velocity,
                                pkey(id)
                            );
                        }
                    }
                }
            }
            println!("worst bibite speed {worst_bibite:.1} u/s, worst bibite spin {worst_spin:.1} rad/s");
        }
        _ => eprintln!("usage: maze_replay snap <save.zip> <tick> <out.state> [seed] | play <save.zip> <in.state> <ticks> <x> <y> [seed]"),
    }
}
