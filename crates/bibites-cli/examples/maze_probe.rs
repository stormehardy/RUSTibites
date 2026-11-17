//! Diagnostic for the report of 2026-11-15 ("bibites who die from getting stuck against an inf-mass pellet spawn meat that immediately
//! kills bibites upon touch"): runs a save and prints
//! - every bibite that disappears (a death: its last health and energy, whether it had moved in the last 100 ticks, how far its body is
//!   from the nearest giant pellet's rim) and the meat that appears near it in the next ticks (place, speed at birth, depth inside a giant),
//! - every pellet that moves faster than a pellet can,
//! - every bibite that loses health in one tick with no other bibite near enough to bite it (so something else did it), and the
//!   pellets it touches.
//!
//! `cargo run --example maze_probe -- <save.zip> <ticks> [seed] [threads] [fast_speed] [health_drop]`

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bibites_core::ids::{BibiteId, PelletId};
use bibites_core::math::Vec2;
use bibites_core::pellet::MaterialKind;
use bibites_core::world::World;

fn bkey(id: BibiteId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

fn pkey(id: PelletId) -> u64 {
    (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot())
}

/// The radius of a pellet's collider (`build_pellet_physics_body`).
fn collider_radius(amount: f32) -> f32 {
    bibites_core::physics::prefab::PELLET_CIRCLE_RADIUS * (amount / 20.0).max(0.0).sqrt()
}

#[derive(Clone, Copy)]
struct Giant {
    pos: Vec2,
    radius: f32,
}

/// Signed distance from `p` to the rim of the nearest giant: negative inside it.
fn nearest_giant(giants: &[Giant], p: Vec2) -> Option<f32> {
    giants
        .iter()
        .map(|g| (p - g.pos).length() - g.radius)
        .min_by(f32::total_cmp)
}

/// What the probe remembers of a bibite from the tick before.
#[derive(Clone, Copy)]
struct Seen {
    health: f32,
    energy: f32,
    pos: Vec2,
    vel: Vec2,
    /// Where it was and when, to tell whether it moves.
    anchor: (u64, Vec2),
    /// How far it went in the last 100 ticks or more.
    moved: f32,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).expect("ticks").parse().expect("ticks");
    let seed: u64 = args.get(3).map_or(1, |s| s.parse().expect("seed"));
    let threads: usize = args.get(4).map_or(1, |s| s.parse().expect("threads"));
    let fast: f32 = args.get(5).map_or(120.0, |s| s.parse().expect("fast_speed"));
    let drop: f32 = args.get(6).map_or(10.0, |s| s.parse().expect("health_drop"));
    let trace: Option<(u64, Vec2)> = match (args.get(7), args.get(8), args.get(9)) {
        (Some(t), Some(x), Some(y)) => Some((t.parse().expect("trace_tick"), Vec2::new(x.parse().expect("x"), y.parse().expect("y")))),
        _ => None,
    };
    let mut contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(threads);
    let giants: Vec<Giant> = world
        .pellets
        .ids()
        .filter_map(|id| {
            let view = world.pellets.get(id)?;
            (view.amount > 5_000.0).then(|| Giant {
                pos: view.pos,
                radius: collider_radius(view.amount),
            })
        })
        .collect();
    println!(
        "loaded {} bibites, {} pellets ({} giants), {} eggs; sim {} TPS",
        world.bibites.len(),
        world.pellets.len(),
        giants.len(),
        world.eggs.len(),
        world.settings.sim_tps
    );

    let mut seen: HashMap<u64, Seen> = HashMap::new();
    let mut known_pellets: HashSet<u64> = world.pellets.ids().map(pkey).collect();
    // Pellets that have moved: their last speeds, and where and when the meat of a death was born (tick, rim gap, amount).
    let mut speed_hist: HashMap<u64, Vec<(u64, f32, f32)>> = HashMap::new();
    let mut born: HashMap<u64, (u64, f32, f32)> = HashMap::new();
    // Pellets inside a giant at the start.
    for id in world.pellets.ids() {
        if let Some(view) = world.pellets.get(id) {
            if view.amount <= 5_000.0 {
                if let Some(gap) = nearest_giant(&giants, view.pos) {
                    if gap < 0.0 {
                        println!(
                            "start: {:?} pellet amount {:.2} at ({:.0}, {:.0}) is {:.1} units INSIDE a giant (collider radius {:.1})",
                            view.material,
                            view.amount,
                            view.pos.x,
                            view.pos.y,
                            -gap,
                            collider_radius(view.amount)
                        );
                        born.insert(pkey(id), (0, gap, view.amount));
                    }
                }
            }
        }
    }
    let mut death_sites: Vec<(u64, Vec2)> = Vec::new();
    let (mut deaths, mut deaths_stuck, mut deaths_by_wall_energy) = (0u32, 0u32, 0u32);
    let (mut deaths_shown, mut meat_shown, mut fast_shown, mut hurt_shown, mut killed_shown) = (0, 0, 0, 0, 0);
    let (mut new_meat, mut new_meat_fast, mut new_meat_inside) = (0u32, 0u32, 0u32);
    let (mut fast_ticks, mut max_pellet_speed) = (0u32, 0.0f32);
    let (mut spin_shown, mut max_rim_speed, mut max_small_rim_speed, mut max_small_spin) = (0, 0.0f32, 0.0f32, 0.0f32);
    let (mut hurt_no_bite, mut hurt_no_bite_with_pellet) = (0u32, 0u32);
    let (mut max_bibite_speed, mut max_bibite_spin, mut thrown) = (0.0f32, 0.0f32, 0u32);
    for tick in 1..=ticks {
        world.tick();
        // The bibites now: the ones that lost health, and the ones that are gone.
        let mut now: HashMap<u64, (Vec2, f32, bool)> = HashMap::new();
        let mut hurt: Vec<(BibiteId, f32, f32, Vec2, Vec2)> = Vec::new();
        for (id, record) in world.bibites.iter() {
            let k = bkey(id);
            let health = record.body.health.amount;
            now.insert(k, (record.pos, health, record.body.dead));
            // A bibite that moves or spins as no bibite can (the physics limits are 400 u/s and 6.3 rad/s).
            let (speed, spin) = (record.vel.length(), record.ang_vel.abs());
            max_bibite_speed = max_bibite_speed.max(speed);
            max_bibite_spin = max_bibite_spin.max(spin);
            if speed > 300.0 || spin > 30.0 {
                thrown += 1;
                if thrown <= 20 {
                    println!(
                        "tick {tick}: THROWN bibite {k:#x} at ({:.0}, {:.0}) speed {speed:.1} spin {spin:.1} held {} rim gap {:?}",
                        record.pos.x,
                        record.pos.y,
                        record.n_held_targets,
                        nearest_giant(&giants, record.pos)
                    );
                }
            }
            let entry = seen.entry(k).or_insert(Seen {
                health,
                energy: record.body.energy.amount,
                pos: record.pos,
                vel: record.vel,
                anchor: (tick, record.pos),
                moved: f32::MAX,
            });
            if entry.health - health >= drop && !record.body.dead {
                hurt.push((id, entry.health, health, record.pos, record.vel));
            }
            entry.health = health;
            entry.energy = record.body.energy.amount;
            entry.pos = record.pos;
            entry.vel = record.vel;
            if tick - entry.anchor.0 >= 100 {
                entry.moved = (record.pos - entry.anchor.1).length();
                entry.anchor = (tick, record.pos);
            }
        }
        let gone: Vec<u64> = seen.keys().filter(|k| !now.contains_key(k)).copied().collect();
        for k in gone {
            let s = seen.remove(&k).expect("seen");
            deaths += 1;
            death_sites.push((tick, s.pos));
            let gap = nearest_giant(&giants, s.pos);
            let stuck = s.moved < 15.0 && gap.is_some_and(|g| g < 25.0);
            if stuck {
                deaths_stuck += 1;
            }
            if s.energy <= 1.0 && stuck {
                deaths_by_wall_energy += 1;
            }
            // A death with energy left was killed by damage: who and what was next to it?
            if s.energy > 2.0 {
                let nearest_bibite = now
                    .values()
                    .map(|(p, _, _)| (*p - s.pos).length())
                    .fold(f32::MAX, f32::min);
                let mut touching: Vec<String> = Vec::new();
                for pid in world.pellets.ids() {
                    if let Some(view) = world.pellets.get(pid) {
                        let d = (view.pos - s.pos).length() - collider_radius(view.amount);
                        if d < 15.0 {
                            touching.push(format!(
                                "{:?} amount {:.1} speed {:.1} gap {d:.1}{}{}",
                                view.material,
                                view.amount,
                                view.velocity.length(),
                                if known_pellets.contains(&pkey(pid)) { "" } else { " (born this tick)" },
                                born.get(&pkey(pid)).map_or(String::new(), |b| format!(" born at tick {} gap {:.1}", b.0, b.1))
                            ));
                        }
                    }
                }
                if killed_shown < 40 {
                    killed_shown += 1;
                    println!(
                        "tick {tick}: KILLED bibite {k:#x} (energy {:.1} left, health {:.1}, speed {:.1}) at ({:.0}, {:.0}); nearest other bibite {nearest_bibite:.1} units; pellets within 15 units: {}",
                        s.energy,
                        s.health,
                        s.vel.length(),
                        s.pos.x,
                        s.pos.y,
                        touching.len()
                    );
                    for text in touching.iter().take(5) {
                        println!("      {text}");
                    }
                }
            }
            if deaths_shown < 40 {
                deaths_shown += 1;
                println!(
                    "tick {tick}: GONE bibite {k:#x} at ({:.0}, {:.0}) last health {:.1} energy {:.1} speed {:.1}, moved {:.0} in its last 100 ticks, rim gap {:?}{}",
                    s.pos.x,
                    s.pos.y,
                    s.health,
                    s.energy,
                    s.vel.length(),
                    s.moved,
                    gap,
                    if stuck { "  <- STUCK AT A WALL" } else { "" }
                );
            }
        }
        death_sites.retain(|(t, _)| tick - t <= 40);
        // Pellets: new meat near a death, and fast ones.
        let mut pellets_now: HashSet<u64> = HashSet::with_capacity(known_pellets.len());
        let (mut fastest_now, mut fast_now) = (0.0f32, 0u32);
        for id in world.pellets.ids() {
            let k = pkey(id);
            pellets_now.insert(k);
            let Some(view) = world.pellets.get(id) else { continue };
            let spin = world.pellets.pose(id).map_or(0.0, |pose| pose.angular_velocity);
            // How fast the rim of the pellet moves: what a bibite that touches it meets, on top of the pellet's own speed.
            let rim_speed = spin.abs() * collider_radius(view.amount);
            if !(rim_speed <= 100.0) && spin_shown < 40 {
                spin_shown += 1;
                println!(
                    "tick {tick}: SPINNING PELLET {:?} amount {:.2} at ({:.0}, {:.0}) spin {spin:.1} rad/s = rim speed {rim_speed:.1} u/s, linear speed {:.1}, rim gap {:?}; born {:?}",
                    view.material,
                    view.amount,
                    view.pos.x,
                    view.pos.y,
                    view.velocity.length(),
                    nearest_giant(&giants, view.pos),
                    born.get(&k)
                );
            }
            if rim_speed.is_finite() {
                max_rim_speed = max_rim_speed.max(rim_speed);
                if view.amount <= 5_000.0 {
                    max_small_rim_speed = max_small_rim_speed.max(rim_speed);
                    max_small_spin = max_small_spin.max(spin.abs());
                }
            }
            let speed = view.velocity.length();
            if speed.is_finite() {
                fastest_now = fastest_now.max(speed);
            }
            if speed > 25.0 || speed_hist.contains_key(&k) {
                let history = speed_hist.entry(k).or_default();
                history.push((tick, speed, nearest_giant(&giants, view.pos).unwrap_or(f32::NAN)));
                if history.len() > 12 {
                    history.remove(0);
                }
            }
            if !(speed <= fast) {
                fast_now += 1;
                if fast_shown < 40 {
                    fast_shown += 1;
                    println!(
                        "tick {tick}: FAST PELLET {:?} amount {:.2} at ({:.0}, {:.0}) speed {speed:.1} rim gap {:?}; born {:?} (tick, rim gap at birth, amount); its last ticks (tick, speed, rim gap): {:?}",
                        view.material,
                        view.amount,
                        view.pos.x,
                        view.pos.y,
                        nearest_giant(&giants, view.pos),
                        born.get(&k),
                        speed_hist.get(&k)
                    );
                }
            }
            if !known_pellets.contains(&k) && view.material == MaterialKind::Meat {
                if death_sites.iter().any(|(_, p)| (*p - view.pos).length() < 80.0) {
                    new_meat += 1;
                    let gap = nearest_giant(&giants, view.pos);
                    born.insert(k, (tick, gap.unwrap_or(f32::NAN), view.amount));
                    if speed > 50.0 {
                        new_meat_fast += 1;
                    }
                    if gap.is_some_and(|g| g < 0.0) {
                        new_meat_inside += 1;
                        println!(
                            "tick {tick}: meat born INSIDE a giant: amount {:.2} at ({:.0}, {:.0}), {:.1} units deep, speed {speed:.1}",
                            view.amount,
                            view.pos.x,
                            view.pos.y,
                            -gap.unwrap_or(0.0)
                        );
                    }
                    if meat_shown < 30 {
                        meat_shown += 1;
                        println!(
                            "tick {tick}: NEW MEAT near a death: amount {:.2} at ({:.0}, {:.0}) speed {speed:.1} rim gap {gap:?}",
                            view.amount, view.pos.x, view.pos.y
                        );
                    }
                }
            }
        }
        known_pellets = pellets_now;
        max_pellet_speed = max_pellet_speed.max(fastest_now);
        if fast_now > 0 {
            fast_ticks += 1;
        }
        // Who lost health with no other bibite near enough to bite it.
        for (id, before, after, pos, vel) in hurt {
            let nearest_bibite = world
                .bibites
                .iter()
                .filter(|(other, _)| *other != id)
                .map(|(_, r)| (r.pos - pos).length())
                .fold(f32::MAX, f32::min);
            let mut near: Vec<(f32, String)> = Vec::new();
            let mut fast_touch = false;
            for pid in world.pellets.ids() {
                if let Some(view) = world.pellets.get(pid) {
                    let d = (view.pos - pos).length() - collider_radius(view.amount);
                    if d < 25.0 {
                        let speed = view.velocity.length();
                        fast_touch |= speed > 30.0 && d < 12.0;
                        near.push((
                            d,
                            format!("{:?} amount {:.1} speed {speed:.1} gap {d:.1} born {:?}", view.material, view.amount, born.get(&pkey(pid))),
                        ));
                    }
                }
            }
            if nearest_bibite < 45.0 && after > 0.0 && !fast_touch {
                continue; // a bite is possible, and nothing fast is touching it
            }
            hurt_no_bite += 1;
            if !near.is_empty() {
                hurt_no_bite_with_pellet += 1;
            }
            if hurt_shown < 40 {
                hurt_shown += 1;
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                println!(
                    "tick {tick}: HURT with no bibite within 45 units (nearest {nearest_bibite:.0}): {:#x} {before:.1} -> {after:.1} at ({:.0}, {:.0}) speed {:.1}; pellets within 25 units: {}",
                    bkey(id),
                    pos.x,
                    pos.y,
                    vel.length(),
                    near.len()
                );
                for (_, text) in near.iter().take(4) {
                    println!("      {text}");
                }
            }
        }
        // A closer look around one place and tick (`trace_tick trace_x trace_y` after the other arguments).
        if let Some((from, centre)) = trace {
            if tick + 3 >= from && tick <= from + 3 {
                println!("---- trace tick {tick} around ({:.0}, {:.0}):", centre.x, centre.y);
                for (id, record) in world.bibites.iter() {
                    let d = (record.pos - centre).length();
                    if d < 90.0 {
                        println!(
                            "   bibite {:#x} at ({:.1}, {:.1}) d {d:.1} vel ({:.1}, {:.1}) speed {:.1} spin {:.2} health {:.1} energy {:.1} dead {} contacts {} held {} mouth targets {} rim gap {:?}",
                            bkey(id),
                            record.pos.x,
                            record.pos.y,
                            record.vel.x,
                            record.vel.y,
                            record.vel.length(),
                            record.ang_vel,
                            record.body.health.amount,
                            record.body.energy.amount,
                            record.body.dead,
                            record.last_contact_count,
                            record.n_held_targets,
                            record.n_mouth_targets,
                            nearest_giant(&giants, record.pos)
                        );
                    }
                }
                for id in world.pellets.ids() {
                    if let (Some(view), Some(pose)) = (world.pellets.get(id), world.pellets.pose(id)) {
                        let d = (view.pos - centre).length();
                        if d < 45.0 && view.amount <= 5_000.0 {
                            println!(
                                "   pellet {:?} amount {:.2} at ({:.2}, {:.2}) d {d:.1} vel ({:.1}, {:.1}) spin {:.2} rim gap {:?} id {:#x}",
                                view.material,
                                view.amount,
                                view.pos.x,
                                view.pos.y,
                                view.velocity.x,
                                view.velocity.y,
                                pose.angular_velocity,
                                nearest_giant(&giants, view.pos),
                                pkey(id)
                            );
                        }
                    }
                }
            }
        }
        if tick % 1000 == 0 {
            println!(
                "-- tick {tick}: {} bibites, {} pellets, {deaths} gone so far ({deaths_stuck} stuck at a wall, {deaths_by_wall_energy} of them out of energy), fastest pellet now {fastest_now:.1}",
                world.bibites.len(),
                world.pellets.len()
            );
        }
    }
    println!("== done: {ticks} ticks, {deaths} bibites gone ({deaths_stuck} stuck at a wall: not moved 15 units in 100 ticks and within 25 of a rim)");
    println!(
        "new meat within 80 units of a death in the 40 ticks after it: {new_meat} ({new_meat_fast} born faster than 50 u/s, {new_meat_inside} born inside a giant); ticks with a pellet faster than {fast}: {fast_ticks}; fastest pellet ever {max_pellet_speed:.1}"
    );
    println!("health drops of {drop}+ with no bibite within 45 units: {hurt_no_bite} ({hurt_no_bite_with_pellet} with a pellet touching)");
    println!("fastest bibite ever {max_bibite_speed:.1} u/s, fastest spin {max_bibite_spin:.1} rad/s; bibite-ticks thrown (faster than 300 u/s or spinning over 30 rad/s): {thrown}");
    println!("fastest pellet rim ever {max_rim_speed:.1} u/s (small pellets: {max_small_rim_speed:.1} u/s, spin up to {max_small_spin:.1} rad/s)");
}
