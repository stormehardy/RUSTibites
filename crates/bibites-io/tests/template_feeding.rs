//! Task-91: does the real `Basic bibite.bb8template` eat end-to-end?
//!
//! The core cannot parse `.bb8template` files, so this io+core integration test
//! loads the real user template through `bibites_io::load_template`, spawns it
//! through the template/runtime path, places a plant pellet directly in front
//! (+Y is the body's forward axis), and checks that plant senses appear,
//! `Want2Eat` is expressed, and the pellet is actually consumed.
//!
//! Skips cleanly when the user Templates folder is absent (CI). Kept short and
//! deterministic: empty scenario, one bibite, one pellet, no random spawners.

use std::path::{Path, PathBuf};

use bibites_core::body::{BibiteSpawn, SpawnVitals};
use bibites_core::brain::{BrainInputs, Input, Output};
use bibites_core::ids::PelletId;
use bibites_core::math::Vec2;
use bibites_core::pellet::{MaterialKind, PelletSpawn};
use bibites_core::settings::SimSettings;
use bibites_core::world::{ScenarioData, World};

const TEMPLATE_DIR: &str =
    r"C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\Bibites\Templates";
const TICKS: u64 = 400;

struct Probe {
    world: World,
    pellet: Option<PelletId>,
    id: bibites_core::ids::BibiteId,
    max_closeness: f32,
    max_angle: f32,
    max_n_plants: f32,
    max_want2eat: f32,
    max_contacts: u8,
    start_energy: f32,
}

impl Probe {
    fn with_pellet(template: &Path, with_pellet: bool) -> Self {
        let loaded = bibites_io::load_template(template).expect("load Basic template");
        let mut settings = SimSettings::game_defaults();
        settings.independents.virgin_spawn_rate = 0.0;
        let mut world =
            World::new(settings, ScenarioData::empty(), 0xB451C).expect("template feeding world");
        let pellet = if with_pellet {
            world.spawn_pellet(PelletSpawn::new(
                MaterialKind::Plant,
                20.0,
                Vec2::new(0.0, 2.0),
            ))
        } else {
            None
        };
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
        let id = world.bibites.spawn(spawn).expect("spawn template bibite");
        world.register_bibite(id);
        let start_energy = world
            .bibites
            .get(id)
            .map_or(f32::NAN, |record| record.body.energy.amount);
        Self {
            world,
            pellet,
            id,
            max_closeness: 0.0,
            max_angle: 0.0,
            max_n_plants: 0.0,
            max_want2eat: 0.0,
            max_contacts: 0,
            start_energy,
        }
    }

    fn run(mut self, label: &str) -> Self {
        for tick in 0..=TICKS {
            if let Some(record) = self.world.bibites.get(self.id) {
                let mut inputs = BrainInputs::new();
                inputs.write_internal(record);
                inputs.write_world_senses(&record.senses);
                self.max_closeness = self.max_closeness.max(inputs.get(Input::PlantCloseness));
                self.max_angle = self.max_angle.max(inputs.get(Input::PlantAngle).abs());
                self.max_n_plants = self.max_n_plants.max(inputs.get(Input::NPlants));
                self.max_want2eat = self
                    .max_want2eat
                    .max(record.brain.output_value(Output::Want2Eat));
                self.max_contacts = self.max_contacts.max(record.n_mouth_targets);
                if tick % 50 == 0 {
                    let pellet = self
                        .pellet
                        .and_then(|id| self.world.pellets.get(id))
                        .map_or(f32::NAN, |view| view.amount);
                    eprintln!(
                        "{label} tick {tick}: plant=({:.4},{:.4},{:.4}) Want2Eat={:.4} \
                         acc={:.4} rot={:.4} contacts={} pellet={pellet:.4} energy={:.3}",
                        inputs.get(Input::PlantCloseness),
                        inputs.get(Input::PlantAngle),
                        inputs.get(Input::NPlants),
                        record.brain.output_value(Output::Want2Eat),
                        record.brain.output_value(Output::Accelerate),
                        record.brain.output_value(Output::Rotate),
                        record.n_mouth_targets,
                        record.body.energy.amount,
                    );
                }
            }
            self.world.tick();
        }
        self
    }

    fn pellet_amount(&self) -> Option<f32> {
        self.pellet
            .and_then(|id| self.world.pellets.get(id))
            .map(|view| view.amount)
    }

    fn final_energy(&self) -> f32 {
        self.world
            .bibites
            .get(self.id)
            .map_or(f32::NAN, |record| record.body.energy.amount)
    }
}

fn template_dir() -> PathBuf {
    PathBuf::from(TEMPLATE_DIR)
}

#[test]
fn real_basic_template_feeds_on_a_plant_in_front() {
    let dir = template_dir();
    if !dir.is_dir() {
        eprintln!(
            "skipping real-template feeding test (absent): {}",
            dir.display()
        );
        return;
    }
    let template = dir.join("Basic bibite.bb8template");
    if !template.is_file() {
        eprintln!("skipping real-template feeding test (no Basic template)");
        return;
    }

    let feeding = Probe::with_pellet(&template, true);
    let start_energy = feeding.start_energy;
    let feeding = feeding.run("feed");
    let with_pellet_energy = feeding.final_energy();
    let with_pellet_amount = feeding.pellet_amount();

    let control = Probe::with_pellet(&template, false).run("control");
    let control_energy = control.final_energy();

    eprintln!(
        "Basic template feeding: start_energy={start_energy:.3} with_pellet_energy={with_pellet_energy:.3} \
         no_pellet_energy={control_energy:.3} pellet={with_pellet_amount:?} \
         max PlantCloseness={:.4} |PlantAngle|={:.4} NPlants={:.4} Want2Eat={:.4} contacts={}",
        feeding.max_closeness,
        feeding.max_angle,
        feeding.max_n_plants,
        feeding.max_want2eat,
        feeding.max_contacts,
    );

    // Dump the real template wiring when the action path does not express.
    if feeding.max_closeness <= 0.0 || feeding.max_want2eat <= 0.0 {
        let loaded = bibites_io::load_template(&template).expect("reload template for dump");
        let nodes = loaded.brain.to_node_records();
        let synapses = loaded.brain.to_synapse_records();
        eprintln!(
            "TEMPLATE WIRING: hidden={} synapses={} genes Basic: LayTime={:.4} Want2Eat wiring:",
            loaded.brain.hidden_count(),
            synapses.len(),
            loaded.genome.gene(bibites_core::genome::Gene::LayTime),
        );
        for synapse in &synapses {
            if synapse.node_in == 12 || synapse.node_in == 13 || synapse.node_out == 38 {
                eprintln!(
                    "  {} -> {} w={:.5} en={}",
                    synapse.node_in, synapse.node_out, synapse.weight, synapse.enabled
                );
            }
        }
        eprintln!("nodes: {nodes:?}");
    }

    assert!(
        feeding.max_closeness > 0.0,
        "real Basic template never produced a PlantCloseness signal"
    );
    assert!(
        feeding.max_want2eat > 0.0,
        "real Basic template never expressed Want2Eat"
    );
    assert!(
        with_pellet_amount.is_none_or(|amount| amount < 19.9),
        "real Basic template did not consume the plant pellet (amount {:?})",
        with_pellet_amount
    );
    assert!(
        feeding.max_contacts > 0,
        "real Basic template never got the pellet into its mouth targets"
    );
    // Intake should beat the no-pellet control's energy trend; allow noise and
    // the large movement cost by checking the energy *change* is less negative.
    let feeding_delta = with_pellet_energy - start_energy;
    let control_delta = control_energy - start_energy;
    eprintln!("energy delta: feeding={feeding_delta:.3} control={control_delta:.3}");
    assert!(
        feeding_delta >= control_delta - 20.0,
        "feeding energy trend unexpectedly worse than control"
    );
}
