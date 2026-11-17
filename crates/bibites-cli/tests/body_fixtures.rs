use std::path::PathBuf;

use bibites_core::world::World;
use bibites_io::load_world_contents;

fn fixture(name: &str) -> Option<World> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/body")
        .join(format!("{name}.zip"));
    if !path.is_file() {
        eprintln!("skipping body fixture {name}: {} missing", path.display());
        return None;
    }
    let contents = load_world_contents(&path, None).expect("load fixture");
    Some(World::from_contents(contents).expect("world"))
}

#[test]
fn egg_fixture_lays_and_hatches() {
    let Some(mut world) = fixture("egg_lay_hatch") else {
        return;
    };
    let parent = world.bibites.iter().next().unwrap().0;
    let mut hatched = false;
    for _ in 0..200 {
        world.tick();
        if world
            .bibites
            .get(parent)
            .is_some_and(|record| record.egg_layer.n_laid > 0)
            && world.bibites.len() > 1
        {
            hatched = true;
            break;
        }
    }
    assert!(hatched, "egg fixture did not lay+hatch a child");
}

#[test]
fn combat_fixture_deals_damage() {
    let Some(mut world) = fixture("combat_attack") else {
        return;
    };
    let initial: f32 = world
        .bibites
        .iter()
        .map(|(_, record)| record.body.health.amount)
        .sum();
    world.run(180);
    let final_health: f32 = world
        .bibites
        .iter()
        .map(|(_, record)| record.body.health.amount)
        .sum();
    assert!(
        final_health < initial - 1.0,
        "combat fixture did not damage health: {initial} -> {final_health}"
    );
    assert!(
        world
            .bibites
            .iter()
            .all(|(_, record)| record.body.health.amount < 100.0),
        "both combatants must take a first + sustained/second bite (game ends ~98.6 HP)"
    );
}

#[test]
fn fat_fixture_inflates_tail_geometry() {
    let Some(mut world) = fixture("fat_armor_inflate") else {
        return;
    };
    world.run(3);
    let record = world.bibites.iter().next().unwrap().1;
    assert!(record.fat.amount > 0.0);
    assert!(
        record.body.geometry.inflate_factor > 0.0,
        "fat fixture did not inflate the body"
    );
    let tail = record.colliders()[1];
    assert!(tail.offset.y < -4.7 * record.body.geometry.d1_size);
}

#[test]
fn starvation_fixture_dies() {
    let Some(mut world) = fixture("starvation_death") else {
        return;
    };
    world.run(2000);
    assert!(
        world.bibites.iter().all(|(_, record)| record.body.dead),
        "starvation fixture still has a living bibite"
    );
    assert!(world.eggs.is_empty());
}
