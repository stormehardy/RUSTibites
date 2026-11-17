use std::path::PathBuf;

use bibites_core::body::BibiteSave;
use bibites_core::brain::{Brain, FIRST_OUTPUT, Input, Output, SynapseRecord};
use bibites_core::genome::Gene;
use bibites_core::math::Vec2;
use bibites_io::{load_world_contents, save_world_contents};

fn main() {
    let base = PathBuf::from("testdata/parity/default/game_scenario_seed_600_nospawn.zip");
    let out_dir = PathBuf::from("crates/bibites-cli/tests/fixtures/body");
    std::fs::create_dir_all(&out_dir).expect("fixture dir");
    let mut contents = load_world_contents(&base, None).expect("load base save");
    contents.settings.scenario.prevent_mutations = true;

    let mut egg = contents.clone();
    egg.scene.bibite_spawns.clear();
    egg.eggs.clear();
    egg.pellets.clear();
    egg.pheromones.clear();
    egg.bibites.truncate(1);
    let parent = egg.bibites.first_mut().expect("one bibite");
    prepare_adult(parent, 0.0, 0.0, 0.0);
    parent.genome.set_gene(Gene::LayTime, 1.0);
    parent.genome.set_gene(Gene::HatchTime, 1.0);
    parent.genome.set_gene(Gene::WombWAG, 13.0);
    parent.egg_progress = 1.0;
    parent.energy = 130.0;
    parent.health = 132.0;
    force_output(parent, Output::EggProduction, 10.0);
    force_output(parent, Output::Want2Lay, 10.0);
    save_world_contents(out_dir.join("egg_lay_hatch.zip"), &egg).expect("save egg fixture");

    let mut combat = contents.clone();
    combat.scene.bibite_spawns.clear();
    combat.eggs.clear();
    combat.pellets.clear();
    combat.pheromones.clear();
    combat.settings.scenario.collision_damage_threshold = 200.0;
    combat.bibites.truncate(2);
    {
        let (a, b) = combat.bibites.split_at_mut(1);
        let a = &mut a[0];
        let b = &mut b[0];
        prepare_adult(a, 0.0, 0.0, 0.0);
        prepare_adult(b, 0.0, 2.0, std::f32::consts::PI);
        for bibite in [a, b] {
            bibite.genome.set_gene(Gene::MouthMusclesWAG, 13.0);
            bibite.genome.set_gene(Gene::ArmorWAG, 0.0);
            bibite.health = 132.0;
            bibite.energy = 130.0;
            force_output(bibite, Output::Want2Attack, 10.0);
            force_output(bibite, Output::Want2Eat, 10.0);
        }
    }
    save_world_contents(out_dir.join("combat_attack.zip"), &combat).expect("save combat fixture");

    let mut organs = contents.clone();
    organs.scene.bibite_spawns.clear();
    organs.eggs.clear();
    organs.pellets.clear();
    organs.pheromones.clear();
    organs.bibites.truncate(1);
    let organ = organs.bibites.first_mut().expect("one bibite");
    prepare_adult(organ, 0.0, 0.0, 0.0);
    organ.genome.set_gene(Gene::FatWAG, 20.0);
    organ.genome.set_gene(Gene::ArmorWAG, 10.0);
    organ.genome.set_gene(Gene::WombWAG, 10.0);
    organ.genome.set_gene(Gene::FatStorageThreshold, 0.1);
    organ.genome.set_gene(Gene::FatStorageDeadband, 0.01);
    organ.energy = 100.0;
    let area = organ.d2_size * 80.0;
    let portion = organ.genome.organ_area_portion(Gene::FatWAG);
    organ.fat_amount = area * portion + 0.1 * area * 0.35;
    organ.egg_progress = 0.0;
    save_world_contents(out_dir.join("fat_armor_inflate.zip"), &organs)
        .expect("save organ fixture");

    let mut death = contents.clone();
    death.scene.bibite_spawns.clear();
    death.eggs.clear();
    death.pellets.clear();
    death.pheromones.clear();
    death.bibites.truncate(1);
    let victim = death.bibites.first_mut().expect("one bibite");
    prepare_adult(victim, 0.0, 0.0, 0.0);
    victim.energy = 0.0;
    victim.health = 1.0;
    victim.dead = false;
    victim.dying = false;
    force_output(victim, Output::Want2Heal, -10.0);
    force_output(victim, Output::Want2Eat, -10.0);
    save_world_contents(out_dir.join("starvation_death.zip"), &death).expect("save death fixture");

    println!("wrote body fixtures to {}", out_dir.display());
}

fn prepare_adult(bibite: &mut BibiteSave, x: f32, y: f32, rot: f32) {
    bibite.pos = Vec2::new(x, y);
    bibite.vel = Vec2::ZERO;
    bibite.rot = rot.to_degrees();
    bibite.ang_vel = 0.0;
    bibite.dead = false;
    bibite.dying = false;
    bibite.growth = bibite.genome.growth_at_mature();
    bibite.energy = bibite.energy.max(1.0);
    bibite.health = bibite.health.max(1.0);
    bibite.n_stomach = 0;
    bibite.stomach = Default::default();
    bibite.egg_progress = 0.0;
    bibite.n_eggs_laid = 0;
    bibite.children.clear();
}

fn force_output(bibite: &mut BibiteSave, output: Output, value: f32) {
    let mut nodes = bibite.brain.to_node_records();
    if let Some(node) = nodes.get_mut(FIRST_OUTPUT + output.index()) {
        node.base_activation = value;
    }
    let mut synapses = bibite.brain.to_synapse_records();
    let target = (FIRST_OUTPUT + output.index()) as u32;
    if !synapses.iter().any(|synapse| synapse.node_out == target) {
        let innovation = synapses
            .iter()
            .map(|synapse| synapse.innovation)
            .max()
            .unwrap_or(0)
            + 1;
        synapses.push(SynapseRecord {
            innovation,
            node_in: Input::EnergyRatio.index() as u32,
            node_out: target,
            weight: 100.0,
            enabled: true,
        });
    }
    bibite.brain = Brain::from_records(&nodes, &synapses).expect("rebuild brain");
}
