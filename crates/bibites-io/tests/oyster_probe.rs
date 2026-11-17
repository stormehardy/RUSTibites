//! Probe (ignored): how a template (RUSTIBITES_PROBE_TEMPLATE) grows, lays eggs and spends energy among plant pellets.
//! `cargo test -p bibites-io --test oyster_probe -- --ignored --nocapture`

use bibites_core::body::{BibiteSpawn, SpawnVitals};
use bibites_core::math::Vec2;
use bibites_core::pellet::{MaterialKind, PelletSpawn};
use bibites_core::settings::SimSettings;
use bibites_core::world::{ScenarioData, World};

#[test]
#[ignore]
fn oyster_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_TEMPLATE").expect("RUSTIBITES_PROBE_TEMPLATE");
    let seconds: u64 = std::env::var("RUSTIBITES_PROBE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let vitals = match std::env::var("RUSTIBITES_PROBE_VITALS").as_deref() {
        Ok("adult") => SpawnVitals::Adult,
        Ok("egg") => SpawnVitals::FromEgg,
        _ => SpawnVitals::Baby,
    };
    let loaded = bibites_io::load_template(&path).expect("load template");
    let mut settings = SimSettings::game_defaults();
    settings.independents.virgin_spawn_rate = 0.0;
    let mut world = World::new(settings, ScenarioData::empty(), 7).expect("world");
    for index in 0..(if std::env::var("RUSTIBITES_PROBE_REFILL").is_ok() { 0 } else { 400 }) {
        let a = index as f32 * 2.399;
        let r = 4.0 + 2.2 * (index as f32).sqrt();
        world.spawn_pellet(PelletSpawn::new(MaterialKind::Plant, 20.0, Vec2::new(r * a.cos(), r * a.sin())));
    }
    let spawn = BibiteSpawn {
        pos: Vec2::ZERO,
        rot: 0.0,
        vel: Vec2::ZERO,
        ang_vel: 0.0,
        genome: loaded.genome.clone(),
        meta: loaded.meta.clone(),
        brain: loaded.brain.clone(),
        vitals,
        energy: None,
        health: None,
    };
    let id = world.bibites.spawn(spawn).expect("spawn");
    world.register_bibite(id);
    let tps = world.settings.sim_tps.max(1) as u64;
    let step_ticks = ((std::env::var("RUSTIBITES_PROBE_STEP").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(5.0) * tps as f32).round() as u64).max(1);
    let refill_until: f32 = std::env::var("RUSTIBITES_PROBE_REFILL_UNTIL").ok().and_then(|v| v.parse().ok()).unwrap_or(1e9);
    let refill = std::env::var("RUSTIBITES_PROBE_REFILL").is_ok();
    let mut follow: Option<(bibites_core::ids::BibiteId, u64)> = None;
    for tick in 0..seconds * tps {
        if tick % step_ticks == 0 {
            if let Some(record) = world.bibites.get(id) {
                eprintln!(
                    "t={:>7.2}s growth={:.6} d2={:.6} mature={} energy={:.6}/{:.6} health={:.5}/{:.5} laid={} eggs_in_womb={:.2} w2g={:.2} bibites={} pos=({:.3},{:.3}) vel=({:.3},{:.3}) rot={:.3}",
                    tick as f32 / tps as f32,
                    record.growth.growth,
                    record.body.geometry.d2_size,
                    record.growth.mature,
                    record.body.energy.amount,
                    record.body.energy.max_amount,
                    record.body.health.amount,
                    record.body.health.max_amount,
                    record.egg_layer.n_laid,
                    record.egg_layer.progress,
                    record.brain.output_value(bibites_core::brain::Output::Want2Grow),
                    world.bibites.len(),
                    record.pos.x, record.pos.y, record.vel.x, record.vel.y, record.rot,
                );
            } else {
                eprintln!("t={:>4}s the bibite is gone; bibites={}", tick / tps, world.bibites.len());
            }
        }
        if tick % tps == 0 {
            let mut neg = 0;
            let mut min_health = f32::MAX;
            let mut worst = None;
            for (bid, record) in world.bibites.iter() {
                if record.body.health.amount < 0.0 && !record.body.dead {
                    neg += 1;
                    if record.body.health.amount < min_health {
                        min_health = record.body.health.amount;
                        worst = Some((bid, record.egg_layer.n_laid, record.body.energy.amount, record.egg_layer.energy_consumption, record.growth.growth, record.body.geometry.d2_size));
                    }
                }
            }
            if neg > 0 {
                eprintln!("NEG t={}s count={} min_health={:.4} worst(laid,energy,E/s,growth,d2)={:?}", tick / tps, neg, min_health, worst.map(|w| (w.1, w.2, w.3, w.4, w.5)));
            }
        }
        if follow.is_none() {
            for (bid, record) in world.bibites.iter() {
                if record.body.health.amount < 0.0 && !record.body.dead && tick > 60 {
                    follow = Some((bid, tick));
                    break;
                }
            }
        }
        if let Some((bid, from)) = follow {
            if tick < from + 90 {
                match world.bibites.get(bid) {
                    Some(r) => eprintln!(
                        "F tick {} health={:.5} energy={:.5} dead={} born={} ready={} growth={:.4} mature={} womb={:.3} laid={} E/s={:.4} dying={}",
                        tick - from, r.body.health.amount, r.body.energy.amount, r.body.dead, r.born, r.is_ready, r.growth.growth, r.growth.mature,
                        r.egg_layer.progress, r.egg_layer.n_laid, r.egg_layer.energy_consumption, r.body.dying
                    ),
                    None => eprintln!("F tick {} gone", tick - from),
                }
            }
        }
        if refill && tick % step_ticks == 0 && (tick as f32 / tps as f32) < refill_until {
            if let Some(record) = world.bibites.get_mut(id) {
                record.body.energy.amount = record.body.energy.max_amount;
                record.body.health.amount = record.body.health.max_amount;
            }
        }
        world.tick();
    }
}

/// Probe (ignored): the template's growth with the brain's tick phase 0 and 1 (the game draws it at random: `Random.Range(0, brainTickFactor)`),
/// energy and health refilled every 5 s, no food. `RUSTIBITES_PROBE_TEMPLATE`, `RUSTIBITES_PROBE_SECONDS`.
#[test]
#[ignore]
fn oyster_phase_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_TEMPLATE").expect("RUSTIBITES_PROBE_TEMPLATE");
    let seconds: u64 = std::env::var("RUSTIBITES_PROBE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let loaded = bibites_io::load_template(&path).expect("load template");
    let mut settings = SimSettings::game_defaults();
    settings.independents.virgin_spawn_rate = 0.0;
    let mut world = World::new(settings, ScenarioData::empty(), 7).expect("world");
    let mut ids = Vec::new();
    for phase in 0..2u32 {
        let spawn = BibiteSpawn {
            pos: Vec2::new(phase as f32 * 400.0 - 2000.0, 0.0),
            rot: 0.0,
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            genome: loaded.genome.clone(),
            meta: loaded.meta.clone(),
            brain: loaded.brain.clone(),
            vitals: SpawnVitals::Baby,
            energy: None,
            health: None,
        };
        let id = world.bibites.spawn(spawn).expect("spawn");
        world.register_bibite(id);
        world.bibites.get_mut(id).unwrap().throttle.brain_counter = phase;
        ids.push(id);
    }
    let tps = world.settings.sim_tps.max(1) as u64;
    for tick in 0..=seconds * tps {
        if tick % (5 * tps) == 0 {
            let mut line = format!("{:>3}", tick / tps);
            for id in &ids {
                if let Some(record) = world.bibites.get_mut(*id) {
                    line += &format!(" {:.3}", record.growth.growth);
                    record.body.energy.amount = record.body.energy.max_amount;
                    record.body.health.amount = record.body.health.max_amount;
                }
            }
            eprintln!("PH {line}");
        }
        world.tick();
    }
}

/// Probe (ignored): per tick, the first second: the brain's EnergyRatio input and Want2Grow output, the growth and the womb.
#[test]
#[ignore]
fn oyster_tick_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_TEMPLATE").expect("RUSTIBITES_PROBE_TEMPLATE");
    let loaded = bibites_io::load_template(&path).expect("load template");
    let mut settings = SimSettings::game_defaults();
    settings.independents.virgin_spawn_rate = 0.0;
    let mut world = World::new(settings, ScenarioData::empty(), 7).expect("world");
    let spawn = BibiteSpawn {
        pos: Vec2::ZERO,
        rot: 0.0,
        vel: Vec2::ZERO,
        ang_vel: 0.0,
        genome: loaded.genome.clone(),
        meta: loaded.meta.clone(),
        brain: loaded.brain.clone(),
        vitals: SpawnVitals::Baby,
        energy: None,
        health: None,
    };
    let id = world.bibites.spawn(spawn).expect("spawn");
    world.register_bibite(id);
    for tick in 0..16 {
        let record = world.bibites.get(id).unwrap();
        eprintln!(
            "TK {tick:>2} growth={:.6} energy={:.6}/{:.6} ratio={:.3} in={:.3} w2g={:.3} upd={:.3} brain_counter={}",
            record.growth.growth,
            record.body.energy.amount,
            record.body.energy.max_amount,
            record.body.energy_ratio(),
            record.brain.input_value(bibites_core::brain::Input::EnergyRatio),
            record.brain.output_value(bibites_core::brain::Output::Want2Grow),
            record.growth.update_period,
            record.throttle.brain_counter,
        );
        world.tick();
    }
}

/// Probe (ignored): the template at SizeRatio 1 starts on a pellet of RUSTIBITES_PROBE_PELLET (0: none); speed and position every 0.2 s.
#[test]
#[ignore]
fn pellet_overlap_probe() {
    let path = std::env::var("RUSTIBITES_PROBE_TEMPLATE").expect("RUSTIBITES_PROBE_TEMPLATE");
    let amount: f32 = std::env::var("RUSTIBITES_PROBE_PELLET").ok().and_then(|v| v.parse().ok()).unwrap_or(200.0);
    let mut loaded = bibites_io::load_template(&path).expect("load template");
    loaded.genome.set_gene(bibites_core::genome::Gene::SizeRatio, 1.0);
    let mut settings = SimSettings::game_defaults();
    settings.independents.virgin_spawn_rate = 0.0;
    let mut world = World::new(settings, ScenarioData::empty(), 7).expect("world");
    if amount > 0.0 {
        world.spawn_pellet(PelletSpawn::new(MaterialKind::Plant, amount, Vec2::ZERO));
    }
    let spawn = BibiteSpawn {
        pos: Vec2::ZERO,
        rot: 0.0,
        vel: Vec2::ZERO,
        ang_vel: 0.0,
        genome: loaded.genome.clone(),
        meta: loaded.meta.clone(),
        brain: loaded.brain.clone(),
        vitals: SpawnVitals::Adult,
        energy: None,
        health: None,
    };
    let id = world.bibites.spawn(spawn).expect("spawn");
    world.register_bibite(id);
    let tps = world.settings.sim_tps.max(1) as u64;
    for tick in 0..=(6.0 * tps as f32) as u64 {
        if tick % 6 == 0 {
            let r = world.bibites.get(id).unwrap();
            eprintln!(
                "PP t={:.2} pos=({:.3},{:.3}) vel=({:.3},{:.3}) speed={:.3} energy={:.2} d2={:.3} pellets={} held={}",
                tick as f32 / tps as f32, r.pos.x, r.pos.y, r.vel.x, r.vel.y, r.vel.length(), r.body.energy.amount, r.body.geometry.d2_size, world.pellets.counts().plant_count, r.n_held_targets
            );
        }
        let r = world.bibites.get_mut(id).unwrap();
        r.body.energy.amount = r.body.energy.max_amount;
        r.body.health.amount = r.body.health.max_amount;
        world.tick();
    }
}
