//! Probe (ignored): a save's bibites at negative health that are still alive, over time.
//! RUSTIBITES_PROBE_SAVE=<zip> RUSTIBITES_PROBE_SECONDS=<n> cargo test -p bibites-io --test negative_health_probe -- --ignored --nocapture

use bibites_core::world::World;
use bibites_io::load_world_contents;

#[test]
#[ignore]
fn negative_health_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let seconds: u64 = std::env::var("RUSTIBITES_PROBE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 11;
    let mut world = World::from_contents(contents).expect("world");
    let tps = world.settings.sim_tps.max(1) as u64;
    let mut worst_total = 0usize;
    for tick in 0..seconds * tps {
        if tick % tps == 0 {
            let mut neg = 0usize;
            let mut min_health = 0.0_f32;
            for (_, record) in world.bibites.iter() {
                if !record.body.dead && record.body.health.amount < 0.0 {
                    neg += 1;
                    min_health = min_health.min(record.body.health.amount);
                }
            }
            worst_total = worst_total.max(neg);
            if tick % (10 * tps) == 0 || neg > 0 {
                eprintln!("t={:>4}s bibites={} alive_with_negative_health={} min={:.4}", tick / tps, world.bibites.len(), neg, min_health);
            }
        }
        world.tick();
    }
    eprintln!("max alive-with-negative-health at a second's sample: {worst_total}");
}

/// Probe (ignored): the fat state of a save's bibites after some ticks (RUSTIBITES_PROBE_SAVE).
#[test]
#[ignore]
fn fat_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 11;
    let mut world = World::from_contents(contents).expect("world");
    for tick in 0..12 {
        for (_, r) in world.bibites.iter() {
            eprintln!(
                "FAT tick {tick} amount={:.2} max={:.2} base_cap={:.2} max_infl={:.2} inflate={:.3} last={:.3} energy={:.1}/{:.1}",
                r.fat.amount, r.fat.max_amount, r.fat.base_capacity, r.fat.max_inflate_amount, r.fat.inflate_factor, r.fat.last_inflate_factor,
                r.body.energy.amount, r.body.energy.max_amount
            );
        }
        world.tick();
    }
}

/// Probe (ignored): load RUSTIBITES_PROBE_SAVE and write it back to RUSTIBITES_PROBE_OUT (to see what the original game makes of our writer).
#[test]
#[ignore]
fn resave_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let out = std::env::var("RUSTIBITES_PROBE_OUT").expect("RUSTIBITES_PROBE_OUT");
    let contents = load_world_contents(&path, None).expect("load");
    bibites_io::save_world_contents(&out, &contents).expect("save");
}

/// Probe (ignored): positions and speeds of a save's bibites tick by tick (RUSTIBITES_PROBE_SAVE, RUSTIBITES_PROBE_TICKS).
#[test]
#[ignore]
fn track_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let ticks: u64 = std::env::var("RUSTIBITES_PROBE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 1;
    let mut world = World::from_contents(contents).expect("world");
    for tick in 0..ticks {
        let mut line = String::new();
        let mut speeds = String::new();
        let mut hp = String::new();
        let mut rows: Vec<_> = world.bibites.iter().map(|(_, r)| (r.pos, r.vel.length(), r.body.health.amount)).collect();
        rows.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        for (pos, speed, health) in rows {
            line += &format!("{:.4},{:.4},", pos.x, pos.y);
            hp += &format!("{health:.3},");
            speeds += &format!("{speed:.3},");
            let _ = health;
        }
        if tick == 0 {
            for (_, r) in world.bibites.iter() {
                for c in r.colliders().iter() { eprintln!("COLL {:?} off={:?} rot={}", c.shape, c.offset, c.rotation); } eprintln!("MASS d1={} mass={} base_body_area={} lin_damp={} inertia={} armor_thickness={}", r.derived.d1_size, r.derived.mass, r.derived.base_body_area, r.derived.linear_damping, r.derived.inertia, r.body.geometry.armor_thickness);
            }
        }
        let mut rots = String::new(); let mut rr: Vec<_> = world.bibites.iter().map(|(_, r)| (r.pos.x, r.rot.to_degrees(), r.vel.x, r.vel.y, r.ang_vel.to_degrees(), r.pos.y, r.body.energy.amount, r.n_held_targets)).collect(); rr.sort_by(|a, b| a.0.total_cmp(&b.0)); for (px, rot, vx, vy, w, py, en, nh) in rr { rots += &format!(" P=({px:.4},{py:.4}) rot={rot:.3} v=({vx:.4},{vy:.4}) w={w:.1} E={en:.7} nh={nh};"); }
        let mut pl = String::new(); for id in world.pellets.ids() { if let (Some(p), Some(v)) = (world.pellets.pos(id), world.pellets.velocity(id)) { pl += &format!(" ({:.3},{:.3},v{:.2},{:.2},h{})", p.x, p.y, v.x, v.y, world.pellets.is_held(id) as u8); } } eprintln!("PL {tick}{pl}");
        eprintln!("TR {tick} {line}{speeds}{rots}"); eprintln!("HP {tick} {hp}");
        world.tick();
    }
}

/// Probe (ignored): the bibites that hold something and how their velocity changes tick to tick (RUSTIBITES_PROBE_SAVE, RUSTIBITES_PROBE_TICKS).
#[test]
#[ignore]
fn holder_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let ticks: u64 = std::env::var("RUSTIBITES_PROBE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 1;
    let mut world = World::from_contents(contents).expect("world");
    let mut last: std::collections::HashMap<u64, (f32, f32, u8)> = std::collections::HashMap::new();
    let (mut spikes, mut holder_ticks, mut changes) = (0u32, 0u32, 0u32);
    for tick in 0..ticks {
        let mut seen = Vec::new();
        for (id, r) in world.bibites.iter() {
            let key = bibites_core::body::bibite_tag(id);
            let speed = r.vel.length();
            if let Some(&(vx, vy, held)) = last.get(&key) {
                let dv = ((r.vel.x - vx).powi(2) + (r.vel.y - vy).powi(2)).sqrt();
                if r.n_held_targets != held { changes += 1; }
                if r.n_held_targets > 0 || held > 0 {
                    holder_ticks += 1;
                    if dv > 8.0 {
                        spikes += 1;
                        if spikes <= 4000 && tick >= 15 {
                            eprintln!("SPIKE tick {tick} id {key:x} dv {dv:.1} speed {speed:.1} held {}->{} pos ({:.1},{:.1}) w {:.1}", held, r.n_held_targets, r.pos.x, r.pos.y, r.ang_vel);
                        }
                    }
                }
            }
            seen.push((key, (r.vel.x, r.vel.y, r.n_held_targets)));
        }
        for (k, v) in seen { last.insert(k, v); }
        if tick % 100 == 0 {
            let holding = world.bibites.iter().filter(|(_, r)| r.n_held_targets > 0).count();
            eprintln!("T {tick} bibites {} holders {holding}", world.bibites.len());
        }
        world.tick();
    }
    eprintln!("SUMMARY holder_ticks {holder_ticks} spikes {spikes} held_changes {changes}");
}

/// Probe (ignored): every bibite's position, speed and hold count per tick, as JSON lines in RUSTIBITES_PROBE_OUT (RUSTIBITES_PROBE_SAVE, RUSTIBITES_PROBE_TICKS).
#[test]
#[ignore]
fn series_probe() {
    use std::io::Write;
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let out = std::env::var("RUSTIBITES_PROBE_OUT").expect("RUSTIBITES_PROBE_OUT");
    let ticks: u64 = std::env::var("RUSTIBITES_PROBE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 1;
    let mut world = World::from_contents(contents).expect("world");
    let mut file = std::fs::File::create(out).unwrap();
    for _ in 0..ticks {
        let mut line = String::from("[");
        for (i, (id, r)) in world.bibites.iter().enumerate() {
            if i > 0 { line.push(','); }
            line += &format!("[{},{:.4},{:.4},{:.4},{}]", bibites_core::body::bibite_tag(id) & 0xFFFF_FFFF, r.pos.x, r.pos.y, r.vel.length(), r.n_held_targets);
        }
        line.push(']');
        writeln!(file, "{line}").unwrap();
        world.tick();
    }
}

/// Probe (ignored): one bibite (RUSTIBITES_PROBE_ID, hex tag) tick by tick with what it holds (RUSTIBITES_PROBE_SAVE, RUSTIBITES_PROBE_TICKS).
#[test]
#[ignore]
fn one_holder_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let want = u64::from_str_radix(&std::env::var("RUSTIBITES_PROBE_ID").expect("RUSTIBITES_PROBE_ID"), 16).unwrap();
    let ticks: u64 = std::env::var("RUSTIBITES_PROBE_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 1;
    let mut world = World::from_contents(contents).expect("world");
    for tick in 0..ticks {
        for (id, r) in world.bibites.iter() {
            if bibites_core::body::bibite_tag(id) != want { continue; }
            let mut held = String::new();
            for t in &r.held_targets[..r.n_held_targets as usize] {
                if let Some(pid) = bibites_core::body::pellet_id_from_tag(*t) {
                    held += &format!(" PEL[{:?} amt {:.0} pos ({:.1},{:.1}) vel ({:.1},{:.1})]", pid, world.pellets.get(pid).map_or(0.0, |v| v.amount), world.pellets.pos(pid).map_or(0.0, |p| p.x), world.pellets.pos(pid).map_or(0.0, |p| p.y), world.pellets.velocity(pid).map_or(0.0, |v| v.x), world.pellets.velocity(pid).map_or(0.0, |v| v.y));
                } else if let Some(bid) = bibites_core::body::bibite_id_from_tag(*t) {
                    if let Some(o) = world.bibites.get(bid) { held += &format!(" BIB[pos ({:.1},{:.1}) vel ({:.1},{:.1}) size {:.2}]", o.pos.x, o.pos.y, o.vel.x, o.vel.y, o.body.geometry.d1_size); }
                }
            }
            eprintln!("H {tick} pos ({:.2},{:.2}) rot {:.2} vel ({:.2},{:.2}) w {:.2} grab {:.2} n_held {} mass {:.2}{held}", r.pos.x, r.pos.y, r.rot, r.vel.x, r.vel.y, r.ang_vel, r.mouth.desire_grab, r.n_held_targets, r.derived.mass);
        }
        world.tick();
    }
}

/// Probe (ignored): do bibites born after the load see other bibites? (RUSTIBITES_PROBE_SAVE, RUSTIBITES_PROBE_SECONDS)
#[test]
#[ignore]
fn newborn_vision_probe() {
    use bibites_core::genome::Gene;
    use bibites_core::math::Vec2;
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let seconds: u64 = std::env::var("RUSTIBITES_PROBE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 11;
    let mut world = World::from_contents(contents).expect("world");
    let initial: std::collections::HashSet<_> = world.bibites.iter().map(|(id, _)| id).collect();
    let tps = world.settings.sim_tps.max(1) as u64;
    for _ in 0..seconds * tps {
        world.tick();
    }
    let all: Vec<(Vec2, bool)> = world.bibites.iter().filter(|(_, r)| !r.body.dead).map(|(_, r)| (r.pos, true)).collect();
    for (label, want_new) in [("initial", false), ("newborn", true)] {
        let (mut n, mut in_view, mut sensed_of_in_view, mut sensed_closeness, mut cache_nonempty) = (0, 0, 0, 0, 0);
        for (id, r) in world.bibites.iter() {
            if r.body.dead || initial.contains(&id) == want_new || (want_new && r.clock.time_alive < 15.0) || r.clock.time_alive < 15.0 {
                continue;
            }
            n += 1;
            let radius = r.genome.gene(Gene::ViewRadius) * r.derived.d1_size;
            let half = r.genome.gene(Gene::ViewAngle) / 2.0;
            let up = Vec2::Y.rotate(r.rot);
            let mut seen = false;
            for (pos, _) in &all {
                let d = *pos - r.pos;
                let dist = d.length();
                if dist < 1.0 || dist > radius * 0.8 {
                    continue;
                }
                let cos = (d.x * up.x + d.y * up.y) / dist;
                if cos.clamp(-1.0, 1.0).acos().to_degrees() < half * 0.8 {
                    seen = true;
                    break;
                }
            }
            let sensed = r.senses.bibite_closeness != 0.0 || r.senses.n_bibites != 0.0;
            let input = r.brain.input_value(bibites_core::brain::Input::BibiteCloseness);
            in_view += usize::from(seen);
            if seen {
                sensed_of_in_view += usize::from(sensed);
                sensed_closeness += usize::from(input != 0.0);
                cache_nonempty += usize::from(!r.fov_seen.bibites.is_empty());
            }
        }
        eprintln!("{label}: n={n} clearly-in-view={in_view} senses-nonzero-of-those={sensed_of_in_view} brain-input-nonzero-of-those={sensed_closeness} fov_seen.bibites-nonempty={cache_nonempty}");
    }
}

/// Probe (ignored): the fraction of bibites whose bibite / plant vision inputs are nonzero over time (RUSTIBITES_PROBE_SAVE).
#[test]
#[ignore]
fn vision_fraction_probe() {
    use bibites_core::brain::Input;
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 11;
    let mut world = World::from_contents(contents).expect("world");
    let tps = world.settings.sim_tps.max(1) as u64;
    for tick in 0..=60 * tps {
        if tick == 0 || tick == 1 || tick == 5 || tick == 10 || tick == 20 || tick == 40 || tick % (10 * tps) == 0 {
            let (mut n, mut b, mut p) = (0, 0, 0);
            for (_, r) in world.bibites.iter() {
                if r.body.dead {
                    continue;
                }
                n += 1;
                b += usize::from(r.brain.input_value(Input::BibiteCloseness) != 0.0 || r.brain.input_value(Input::NBibites) != 0.0);
                p += usize::from(r.brain.input_value(Input::PlantCloseness) != 0.0);
            }
            eprintln!("tick {tick:>5}: n={n} bibite-input-nonzero={b} plant-input-nonzero={p}");
        }
        world.tick();
    }
}

/// Probe (ignored): how far the spatial hash's bibite entries are from the bibites (RUSTIBITES_PROBE_SAVE).
#[test]
#[ignore]
fn hash_staleness_probe() {
    use bibites_core::world::SpatialClass;
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 11;
    let mut world = World::from_contents(contents).expect("world");
    let tps = world.settings.sim_tps.max(1) as u64;
    for tick in 0..=20 * tps {
        if tick % (5 * tps) == 0 {
            let mut worst = 0.0_f32;
            let mut sum = 0.0_f32;
            let mut n = 0;
            for (id, r) in world.bibites.iter() {
                if let Some(p) = world.hash.pos_of(id.handle(), SpatialClass::Bibite) {
                    let d = (p - r.pos).length();
                    worst = worst.max(d);
                    sum += d;
                    n += 1;
                }
            }
            eprintln!("tick {tick}: {n} bibites in hash, mean offset {:.2}, worst {:.2}", sum / n.max(1) as f32, worst);
        }
        world.tick();
    }
}

/// Probe (ignored): the conservation terms of the pellets after some seconds (RUSTIBITES_PROBE_SAVE, RUSTIBITES_PROBE_SECONDS).
#[test]
#[ignore]
fn accounting_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let seconds: u64 = std::env::var("RUSTIBITES_PROBE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    let mut contents = load_world_contents(&path, None).expect("load");
    contents.seed = 11;
    let mut world = World::from_contents(contents).expect("world");
    let tps = world.settings.sim_tps.max(1) as u64;
    for tick in 0..=seconds * tps {
        if tick == 30 * tps || tick == seconds * tps {
            eprintln!("t={}s {:#?}", tick / tps, world.conservation_totals());
            let mut f = bibites_core::body::FeedingCounters::default();
            for (_, r) in world.bibites.iter() {
                let c = &r.feeding;
                f.meat_bite_events += c.meat_bite_events;
                f.meat_swallow_events += c.meat_swallow_events;
                f.bibite_swallow_events += c.bibite_swallow_events;
                f.plant_bite_events += c.plant_bite_events;
                f.plant_swallow_events += c.plant_swallow_events;
                f.meat_consumed_u2 += c.meat_consumed_u2;
                f.plant_consumed_u2 += c.plant_consumed_u2;
                f.bibite_consumed_u2 += c.bibite_consumed_u2;
                f.meat_ripped_u2 += c.meat_ripped_u2;
                f.plant_ripped_u2 += c.plant_ripped_u2;
                f.stomach_digested_u2 += c.stomach_digested_u2;
                f.stomach_energy_gain_e += c.stomach_energy_gain_e;
                f.energy_gained_e += c.energy_gained_e;
            }
            eprintln!("feeding sums (living bibites only): {f:#?}");
        }
        world.tick();
    }
}

/// Probe (ignored): the logged history of a save's species - how many points, and the count and energy of the biggest ones over time
/// (RUSTIBITES_PROBE_SAVE). What the lineage mesh's edges are made of.
#[test]
#[ignore]
fn species_log_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_SAVE").expect("RUSTIBITES_PROBE_SAVE");
    let contents = load_world_contents(&path, None).expect("load");
    let world = World::from_contents(contents).expect("world");
    let registry = &world.species;
    eprintln!("species {} max_energy {} records {}", registry.records().len(), registry.max_energy(), registry.n_records());
    let mut ranked: Vec<_> = registry.records().iter().collect();
    ranked.sort_by_key(|record| std::cmp::Reverse(record.stats.log.n_points_present()));
    for record in ranked.iter().take(6) {
        let log = &record.stats.log;
        let present = log.n_points_present() as i32;
        let first = log.total_index_of_apparition();
        let last = log.total_index_of_disappearance();
        eprintln!("species {} present {} apparition {} disappearance {}", record.id.value(), present, first, last);
        let mut line = String::new();
        let top = first.min(40);
        for i in (last.max(0)..=top).rev().step_by(1).take(40) {
            let p = log.at(i as usize);
            line += &format!("[{} {:.0}] ", p.count, p.energy);
        }
        eprintln!("   newest->older: {line}");
    }
}

/// Probe (ignored): a New Game world of a given size (units) and biomass density (E/u^2) - pellets, ticks and how long they take
/// (`RUSTIBITES_PROBE_SIZE`, `RUSTIBITES_PROBE_DENSITY`, `RUSTIBITES_PROBE_SECONDS`); watch the process's memory from outside.
#[test]
#[ignore]
fn big_world_probe() {
    let size: f32 = std::env::var("RUSTIBITES_PROBE_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(10000.0);
    let density: f32 = std::env::var("RUSTIBITES_PROBE_DENSITY").ok().and_then(|v| v.parse().ok()).unwrap_or(0.1);
    let seconds: u64 = std::env::var("RUSTIBITES_PROBE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    use bibites_core::settings::SettingValue;
    let scenario = std::env::var("RUSTIBITES_PROBE_SCENARIO").unwrap_or_else(|_| "testdata/real/Default.zip".to_owned());
    let bundle = bibites_io::load_scenario_bundle(&scenario).expect("scenario");
    let mut settings = bibites_io::world::scenario_effective_settings(&bundle, None);
    settings.independents.set_raw("SimulationSize", SettingValue::Float(size));
    settings.independents.set_raw("biomassDensity", SettingValue::Float(density));
    let templates = bundle
        .templates
        .iter()
        .map(|(name, loaded)| {
            bibites_core::world::TemplateSource::new(name.clone(), loaded.genome.clone(), loaded.meta.clone(), loaded.brain.clone())
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let mut world = World::new_fresh_scenario(settings, bundle.scenario.clone(), 12_345, templates).expect("world");
    let counts = world.pellets.counts();
    eprintln!("created in {:.1} s: plant {} meat {} bibites {}", started.elapsed().as_secs_f64(), counts.plant_count, counts.meat_count, world.bibites.len());
    let tps = world.settings.sim_tps.max(1) as u64;
    let begin = std::time::Instant::now();
    let mut ticks = 0u64;
    while begin.elapsed().as_secs() < seconds {
        world.tick();
        ticks += 1;
        if ticks % (tps * 5) == 0 {
            let counts = world.pellets.counts();
            eprintln!("{:>4.0} s wall: tick {} plant {} bibites {}", begin.elapsed().as_secs_f64(), ticks, counts.plant_count, world.bibites.len());
        }
    }
}
