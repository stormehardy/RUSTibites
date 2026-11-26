//! Second diagnostic for the report of 2026-11-25 (Ringworld.zip: "a bibite with nothing to eat keeps filling its stomach; its digestion energy goes up"). Ticks a save and,
//! for every bibite each tick, sorts the growth of its stomach by source: pellets eaten or bibites swallowed (the consumed counters), meat from biting a bibite (damage
//! dealt), or NOTHING the counters know of. It also looks at every stomach slot after the tick for a digestion that went backwards (a negative digestion rate makes the
//! slot grow) and lists the bibites that grow the most from each source, with their targets and, for a bite, the victim's health and energy before and after.
//!
//! `cargo run --profile perf --example stomach_source_probe -- <save.zip> [ticks] [threads]`

use std::collections::HashMap;
use std::path::PathBuf;

use bibites_core::body::{bibite_id_from_tag, pellet_id_from_tag};
use bibites_core::ids::BibiteId;
use bibites_core::world::World;

#[derive(Clone, Copy, Default)]
struct Counters {
    stomach: f32,
    consumed: f64,
    swallows: u64,
    damage: f64,
}

fn counters(world: &World) -> HashMap<BibiteId, Counters> {
    world
        .bibites
        .iter()
        .map(|(id, r)| {
            let f = &r.feeding;
            (
                id,
                Counters {
                    stomach: r.stomach.total_amount(),
                    consumed: f.plant_consumed_u2 + f.meat_consumed_u2 + f.bibite_consumed_u2,
                    swallows: f.plant_swallow_events + f.meat_swallow_events + f.bibite_swallow_events,
                    damage: f.bibite_damage_dealt,
                },
            )
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    ticks: u32,
    grown: f32,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).map_or(300, |s| s.parse().expect("ticks"));
    let threads: usize = args.get(3).map_or(6, |s| s.parse().expect("threads"));
    let contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    eprintln!("loaded {} bibites, {} eggs, {} pellets", world.bibites.len(), world.eggs.len(), world.pellets.len());
    let mut by_source: HashMap<&'static str, HashMap<BibiteId, Tally>> = HashMap::new();
    let mut negative_digestion: HashMap<BibiteId, (u32, f32)> = HashMap::new();
    let mut unexplained_examples: Vec<String> = Vec::new();
    for tick in 0..ticks {
        let before = counters(&world);
        let targets_before: HashMap<BibiteId, Vec<u64>> = world
            .bibites
            .iter()
            .filter(|(_, r)| r.n_mouth_targets > 0)
            .map(|(id, r)| (id, r.mouth_targets[..r.n_mouth_targets as usize].to_vec()))
            .collect();
        world.tick();
        for (id, r) in world.bibites.iter() {
            for content in r.stomach.contents() {
                if content.digestion_rate < 0.0 || !content.digestion_rate.is_finite() || !content.amount.is_finite() {
                    let entry = negative_digestion.entry(id).or_default();
                    entry.0 += 1;
                    entry.1 = content.digestion_rate;
                }
            }
            let Some(b) = before.get(&id) else {
                continue;
            };
            let grew = r.stomach.total_amount() - b.stomach;
            if grew <= 1e-5 {
                continue;
            }
            let f = &r.feeding;
            let consumed = f.plant_consumed_u2 + f.meat_consumed_u2 + f.bibite_consumed_u2 - b.consumed;
            let swallows = f.plant_swallow_events + f.meat_swallow_events + f.bibite_swallow_events - b.swallows;
            let damage = f.bibite_damage_dealt - b.damage;
            let source = if consumed > 0.0 || swallows > 0 {
                "pellets eaten / bibites swallowed"
            } else if damage > 0.0 {
                "meat from biting a bibite"
            } else {
                "NOTHING the counters know of"
            };
            let tally = by_source.entry(source).or_default().entry(id).or_default();
            tally.ticks += 1;
            tally.grown += grew;
            if source.starts_with("NOTHING") && unexplained_examples.len() < 40 {
                let slots: Vec<String> = r.stomach.contents().map(|c| format!("{:?} amount {:.3} rate {:.4} eff {:.3}", c.material, c.amount, c.digestion_rate, c.efficiency)).collect();
                unexplained_examples.push(format!(
                    "tick {tick}: {:?} grew {:.4} ({:.3} -> {:.3}); corpse {}; targets before {:?}; slots [{}]",
                    id,
                    grew,
                    b.stomach,
                    r.stomach.total_amount(),
                    r.is_corpse,
                    targets_before.get(&id).map(|t| t.iter().map(|tag| if pellet_id_from_tag(*tag).is_some() { "pellet" } else if bibite_id_from_tag(*tag).is_some() { "bibite" } else { "other" }).collect::<Vec<_>>()),
                    slots.join("; ")
                ));
            }
        }
    }
    println!("=== {ticks} ticks");
    let mut sources: Vec<_> = by_source.iter().collect();
    sources.sort_by_key(|(name, _)| *name);
    for (source, per) in sources {
        let total: f32 = per.values().map(|t| t.grown).sum();
        let ticks_sum: u32 = per.values().map(|t| t.ticks).sum();
        println!("-- {source}: {} bibites, {} bibite-ticks, {:.1} u2 in all", per.len(), ticks_sum, total);
        let mut top: Vec<_> = per.iter().collect();
        top.sort_by(|a, b| b.1.grown.total_cmp(&a.1.grown));
        for (id, t) in top.iter().take(8) {
            let alive = world.bibites.get(**id).map_or("gone".to_owned(), |r| {
                format!(
                    "pos ({:.0},{:.0}) corpse {} health {:.2}/{:.2} energy {:.2} stomach {:.2}/{:.2} targets {:?}",
                    r.pos.x,
                    r.pos.y,
                    r.is_corpse,
                    r.body.health.amount,
                    r.body.health.max_amount,
                    r.body.energy.amount,
                    r.stomach.total_amount(),
                    r.stomach.available_space() + r.stomach.total_amount(),
                    r.mouth_targets[..r.n_mouth_targets as usize]
                        .iter()
                        .map(|tag| match (pellet_id_from_tag(*tag), bibite_id_from_tag(*tag)) {
                            (Some(p), _) => format!("pellet {p:?}"),
                            (_, Some(v)) => world.bibites.get(v).map_or(format!("bibite {v:?} gone"), |vr| {
                                format!(
                                    "bibite {:?} corpse {} health {:.2}/{:.2} energy {:.2} dist {:.1}",
                                    v,
                                    vr.is_corpse,
                                    vr.body.health.amount,
                                    vr.body.health.max_amount,
                                    vr.body.energy.amount,
                                    (vr.pos - r.pos).length()
                                )
                            }),
                            _ => "other".to_owned(),
                        })
                        .collect::<Vec<_>>()
                )
            });
            println!("   {:?}: grew in {} ticks, {:.2} u2 :: {}", id, t.ticks, t.grown, alive);
        }
    }
    println!("-- slots digesting backwards (negative or not-a-number rate): {} bibites", negative_digestion.len());
    for (id, (n, rate)) in negative_digestion.iter().take(15) {
        println!("   {id:?}: {n} slot-ticks, last rate {rate}");
    }
    println!("-- examples of growth the counters do not explain:");
    for line in &unexplained_examples {
        println!("   {line}");
    }
}
