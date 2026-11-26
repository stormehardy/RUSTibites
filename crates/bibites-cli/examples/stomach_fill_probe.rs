//! Diagnostic for the report of 2026-11-25: "a bibite with nothing to eat keeps filling its stomach with more pellets, not visual only, as its digestion energy goes up"
//! (the owner's save `Ringworld.zip`). Loads a save, ticks it, and for every bibite whose stomach holds more after a tick than before, says what it could have eaten
//! from: its mouth targets before the tick (pellets, bibites, eggs: where they are, how far from the bibite, whether a pellet is held and by whom, whether it still
//! exists after the tick), its held targets, and what its feeding counters say it took. A stomach that grows with no target near, or with no feeding counted, is
//! reported in full.
//!
//! `cargo run --profile perf --example stomach_fill_probe -- <save.zip> [ticks] [threads] [report limit]`

use std::collections::HashMap;
use std::path::PathBuf;

use bibites_core::body::{bibite_id_from_tag, egg_id_from_tag, pellet_id_from_tag};
use bibites_core::ids::BibiteId;
use bibites_core::math::Vec2;
use bibites_core::world::World;

#[derive(Clone, Debug)]
struct Before {
    stomach: f32,
    pos: Vec2,
    rot: f32,
    corpse: bool,
    targets: Vec<u64>,
    held: Vec<u64>,
    consumed: f64,
    ripped: f64,
    swallows: u64,
    bites: u64,
}

fn snapshot(world: &World) -> HashMap<BibiteId, Before> {
    world
        .bibites
        .iter()
        .map(|(id, record)| {
            let f = &record.feeding;
            (
                id,
                Before {
                    stomach: record.stomach.total_amount(),
                    pos: record.pos,
                    rot: record.rot,
                    corpse: record.is_corpse,
                    targets: record.mouth_targets[..record.n_mouth_targets as usize].to_vec(),
                    held: record.held_targets[..record.n_held_targets as usize].to_vec(),
                    consumed: f.plant_consumed_u2 + f.meat_consumed_u2 + f.bibite_consumed_u2,
                    ripped: f.plant_ripped_u2 + f.meat_ripped_u2,
                    swallows: f.plant_swallow_events + f.meat_swallow_events + f.bibite_swallow_events,
                    bites: f.plant_bite_events + f.meat_bite_events + f.bibite_bite_events,
                },
            )
        })
        .collect()
}

/// What a target tag is, where it is and how far from `from` (before the tick).
fn describe(world: &World, tag: u64, from: Vec2) -> (String, f32) {
    if let Some(id) = pellet_id_from_tag(tag) {
        return match world.pellets.get(id) {
            Some(view) => (
                format!(
                    "pellet {:?} amount {:.1} r {:.1} at ({:.1},{:.1}) held {} vel ({:.2},{:.2})",
                    id, view.amount, view.radius, view.pos.x, view.pos.y, view.held, view.velocity.x, view.velocity.y
                ),
                (view.pos - from).length() - view.radius,
            ),
            None => (format!("pellet {id:?} GONE"), f32::NAN),
        };
    }
    if let Some(id) = bibite_id_from_tag(tag) {
        return match world.bibites.get(id) {
            Some(r) => (format!("bibite {:?} at ({:.1},{:.1}) corpse {}", id, r.pos.x, r.pos.y, r.is_corpse), (r.pos - from).length()),
            None => (format!("bibite {id:?} GONE"), f32::NAN),
        };
    }
    if let Some(id) = egg_id_from_tag(tag) {
        return match world.eggs.get(id) {
            Some(e) => (format!("egg {:?} at ({:.1},{:.1})", id, e.pos.x, e.pos.y), (e.pos - from).length()),
            None => (format!("egg {id:?} GONE"), f32::NAN),
        };
    }
    (format!("tag {tag:#x} (not a pellet, bibite or egg)"), f32::NAN)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).map_or(20, |s| s.parse().expect("ticks"));
    let threads: usize = args.get(3).map_or(6, |s| s.parse().expect("threads"));
    let limit: usize = args.get(4).map_or(25, |s| s.parse().expect("limit"));
    let contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    eprintln!(
        "loaded {} bibites, {} eggs, {} pellets, threads {}, time {:.0} s",
        world.bibites.len(),
        world.eggs.len(),
        world.pellets.len(),
        world.threads(),
        world.clock.time_secs()
    );
    let mut reported = 0usize;
    let mut totals: HashMap<&'static str, usize> = HashMap::new();
    let mut repeat: HashMap<BibiteId, u32> = HashMap::new();
    for tick in 0..ticks {
        let before = snapshot(&world);
        // Where every target was before the tick (the tick moves and eats them).
        let mut seen: HashMap<BibiteId, Vec<(String, f32)>> = HashMap::new();
        for (id, b) in &before {
            if !b.targets.is_empty() || !b.held.is_empty() {
                let mut list: Vec<(String, f32)> = b.targets.iter().map(|tag| describe(&world, *tag, b.pos)).collect();
                list.extend(b.held.iter().map(|tag| {
                    let (text, d) = describe(&world, *tag, b.pos);
                    (format!("HELD {text}"), d)
                }));
                seen.insert(*id, list);
            }
        }
        world.tick();
        for (id, record) in world.bibites.iter() {
            let Some(b) = before.get(&id) else {
                continue;
            };
            let after = record.stomach.total_amount();
            let grew = after - b.stomach;
            if grew <= 1e-4 {
                continue;
            }
            let f = &record.feeding;
            let consumed = f.plant_consumed_u2 + f.meat_consumed_u2 + f.bibite_consumed_u2 - b.consumed;
            let swallows = f.plant_swallow_events + f.meat_swallow_events + f.bibite_swallow_events - b.swallows;
            let bites = f.plant_bite_events + f.meat_bite_events + f.bibite_bite_events - b.bites;
            let targets = seen.get(&id).cloned().unwrap_or_default();
            let nearest = targets.iter().map(|(_, d)| *d).filter(|d| d.is_finite()).fold(f32::INFINITY, f32::min);
            let class: &'static str = if targets.is_empty() {
                "grew with NO target"
            } else if consumed <= 0.0 && swallows == 0 {
                "grew with NO feeding counted"
            } else if nearest > 40.0 {
                "grew from a FAR target (> 40 u)"
            } else {
                "normal"
            };
            *totals.entry(class).or_default() += 1;
            if class != "normal" {
                *repeat.entry(id).or_default() += 1;
                if reported < limit {
                    reported += 1;
                    println!(
                        "tick {tick}: {:?} [{class}] stomach {:.2} -> {:.2} (+{:.3}); counted consumed {:.3}, swallows {}, bites {}; corpse {} -> {}; pos ({:.1},{:.1}) rot {:.2}",
                        id, b.stomach, after, grew, consumed, swallows, bites, b.corpse, record.is_corpse, b.pos.x, b.pos.y, b.rot
                    );
                    for (text, d) in &targets {
                        println!("      target: {text}  (distance {d:.1})");
                    }
                }
            }
        }
    }
    println!("--- classes over {ticks} ticks:");
    let mut list: Vec<_> = totals.into_iter().collect();
    list.sort();
    for (class, count) in list {
        println!("  {class}: {count}");
    }
    let mut worst: Vec<_> = repeat.into_iter().collect();
    worst.sort_by(|a, b| b.1.cmp(&a.1));
    println!("--- bibites flagged most often: {:?}", &worst[..worst.len().min(15)]);
}
