//! Diagnostic for "pellet spawning seems biased" (2026-11-23): runs a save and logs where pellets are BORN (a pellet id not seen in the tick before),
//! split by what made them (owned by a zone = the zone's production; free = chunks ripped by bites, stomach spills, corpse meat, throws), and tests each group
//! for an even spread over the world with a chi-square.
//!
//! `cargo run --example spawn_births -- <save.zip> <ticks> [seed] [half-size]`

use std::collections::HashSet;
use std::path::PathBuf;

use bibites_core::world::World;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = PathBuf::from(args.get(1).expect("save path"));
    let ticks: u64 = args.get(2).expect("ticks").parse().expect("ticks");
    let seed: u64 = args.get(3).map_or(1, |s| s.parse().expect("seed"));
    let half: f32 = args.get(4).map_or(6000.0, |s| s.parse().expect("half-size"));
    let mut contents = bibites_io::load_world_contents(&path, None).expect("load the save");
    contents.seed = seed;
    let mut world = World::from_contents(contents).expect("world");
    world.set_threads(3);
    let key = |id: bibites_core::ids::PelletId| (u64::from(id.handle().generation()) << 32) | u64::from(id.handle().slot());
    let mut seen: HashSet<u64> = world.pellets.ids().map(key).collect();
    let cells = 8usize;
    // [zone-owned, free] x [plant, meat]
    let mut grids = vec![vec![0u32; cells * cells]; 4];
    let mut totals = [0u64; 4];
    let mut in_pile = [0u64; 4];
    let mut outside = [0u64; 4];
    for _tick in 0..ticks {
        world.tick();
        let mut now: HashSet<u64> = HashSet::with_capacity(seen.len() + 64);
        for id in world.pellets.ids() {
            let k = key(id);
            now.insert(k);
            if !seen.contains(&k) {
                let Some(view) = world.pellets.get(id) else { continue };
                let owned = world.pellets.owner_zone(id).flatten().is_some();
                let meat = !matches!(view.material, bibites_core::pellet::MaterialKind::Plant);
                let group = usize::from(!owned) * 2 + usize::from(meat);
                totals[group] += 1;
                if view.pos.x.abs() >= half || view.pos.y.abs() >= half {
                    outside[group] += 1;
                    continue;
                }
                let cell = |t: f32| (((t + half) / (2.0 * half) * cells as f32) as usize).min(cells - 1);
                grids[group][cell(view.pos.y) * cells + cell(view.pos.x)] += 1;
                // the two clumps of the "deadly pile" save: the centre (250,250, r 700) and the south-west one (-5100,-5500, r 600)
                let near = |x: f32, y: f32, r: f32| ((view.pos.x - x).powi(2) + (view.pos.y - y).powi(2)).sqrt() < r;
                if near(250.0, 250.0, 700.0) || near(-5100.0, -5500.0, 600.0) {
                    in_pile[group] += 1;
                }
            }
        }
        seen = now;
    }
    let names = ["zone-owned plant", "zone-owned meat", "free plant", "free meat"];
    let pile_share = (std::f64::consts::PI * 700.0 * 700.0 + std::f64::consts::PI * 600.0 * 600.0) / f64::from(2.0 * half).powi(2);
    println!("{ticks} ticks, seed {seed}; the two clump discs cover {:.2} % of the field", pile_share * 100.0);
    for (group, name) in names.iter().enumerate() {
        let inside: u64 = grids[group].iter().map(|&c| u64::from(c)).sum();
        let expected = inside as f64 / (cells * cells) as f64;
        let chi: f64 = if expected > 0.0 { grids[group].iter().map(|&c| (f64::from(c) - expected).powi(2) / expected).sum() } else { 0.0 };
        let dof = (cells * cells - 1) as f64;
        println!(
            "{name:<17} born {:>7} (inside the field {inside}, outside {}) | chi-square over {cells}x{cells}: {chi:.1} (even: {dof:.0} +- {:.0}) | in the two clump discs {} = {:.2} %",
            totals[group],
            outside[group],
            (2.0 * dof).sqrt(),
            in_pile[group],
            100.0 * in_pile[group] as f64 / inside.max(1) as f64
        );
    }
}
