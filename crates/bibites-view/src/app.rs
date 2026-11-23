//! Viewer application state: world loading, deterministic fixed-step playback,
//! input handling and the data shown in the HUD/selection panel.
//!
//! The app never touches the simulation's randomness: it only calls
//! [`World::tick`] at the world's own fixed timestep, so watching a world in the
//! viewer yields bit-identical results to running it headless with the same seed
//! and tick count (there is a regression test for exactly this).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bibites_core::body::{BibiteSpawn, SpawnVitals};
use bibites_core::brain::{ALL_OUTPUTS, Brain, N_OUTPUTS};
use bibites_core::genome::{Gene, Genome, GenomeMeta, N_GENES};
use bibites_core::ids::{BibiteId, SpeciesId};
use bibites_core::math::Vec2;
use bibites_core::settings::{GrowthAtSpawn, SimSettings, Tagging};
use bibites_core::world::{ScenarioData, TemplateSource, World};
use bibites_io::storage::StoragePaths;

mod animator_actions;
mod force_tower_actions;
mod gene_tower_actions;
mod hidden_objects;
mod map_zone_labels;
mod stream_actions;
mod time_tag_actions;
mod vent_tower_actions;
mod whirlpool_actions;
mod zone_color_actions;

pub use time_tag_actions::TimeTagEdit;

use crate::assets::GameAssets;
use crate::render::{Camera, Scene, SceneOptions, build_scene, pigment_rgb, render_scene};
use crate::tail_mesh::TailState;
use crate::ui::{InspectorTab, OverlayKind, UiAction, UiInteraction};

/// Screen margin (logical px) kept around the world by camera fit.
pub const VIEW_MARGIN_PX: f32 = 24.0;
/// Minimum world-space extent of the default "fit bibites" view. A single
/// bibite is only ~13 world units long, so without a floor the camera would
/// zoom in absurdly far; 200 units frames a readable patch of world.
pub const MIN_VIEW_EXTENT_WORLD: f32 = 200.0;
/// The default view always shows at least this many body lengths of context.
pub const VIEW_BODY_LENGTH_FACTOR: f32 = 4.0;
/// Maximum world-space extent of the default view. Populations spread across a
/// huge scenario world (e.g. Ring's 100k-unit square) would otherwise produce a
/// whole-world fit where bibites are sub-pixel dots; beyond this extent the
/// default view anchors on a bibite instead.
pub const MAX_DEFAULT_VIEW_EXTENT_WORLD: f32 = 500.0;
/// Maximum species rows shown in the HUD legend.
pub const MAX_HUD_SPECIES: usize = 8;
/// Catch-up cap: at most this many simulation ticks per frame. When the viewer
/// cannot keep up (slow machine, huge population) it drops backlog instead of
/// spiralling; the simulation itself is unaffected.
pub const MAX_CATCHUP_TICKS: u32 = 32;

/// `RUSTIBITES_FRAME_PAD_MS=N`: every frame of the window takes N milliseconds longer to be shown (a model of a big window on a slow graphics card, to look at the
/// frame pacing: the window sleeps before it presents). Unset or 0: nothing.
#[must_use]
pub fn frame_pad() -> Option<std::time::Duration> {
    static PAD: std::sync::OnceLock<Option<std::time::Duration>> = std::sync::OnceLock::new();
    *PAD.get_or_init(|| {
        std::env::var("RUSTIBITES_FRAME_PAD_MS")
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|ms| *ms > 0.0)
            .map(|ms| std::time::Duration::from_secs_f64(ms / 1000.0))
    })
}

/// `RUSTIBITES_LEGACY_PACING=1`: the frame pacing of the builds before 2026-11-23 (the ticks' room is a frame of the minimum FPS whatever the drawing takes, and the
/// minimum-FPS rule takes the time scale down whenever the frames are slow, even when they are slow for the drawing and not for the ticks), to compare with.
fn legacy_pacing() -> bool {
    static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LEGACY.get_or_init(|| std::env::var("RUSTIBITES_LEGACY_PACING").is_ok_and(|value| value != "0"))
}
/// Time-speed range of `TimeController.targetTimeScale` (0.1..100); keys `1`..`8`
/// select presets inside it.
pub const MAX_SPEED: f32 = 100.0;
/// Lower bound of the time-speed slider.
pub const MIN_SPEED: f32 = 0.1;
/// Seconds between HUD population refreshes (the stats scan is O(population)).
pub const HUD_REFRESH_SECS: f32 = 0.2;
/// Frames in a row that only watch before the world moves to the simulation thread (so that a click or a hotkey that follows
/// a moment of watching does not pay for the move and the move back).
pub const LIGHT_FRAMES_BEFORE_AWAY: u32 = 20;
/// The most auto-panner steps made at once when the world comes back from the simulation thread (the auto-panner moves with
/// every tick, and the ticks of the time away are made up for; a step costs about a hundred nanoseconds).
const AUTOPAN_CATCH_UP_LIMIT: u64 = 2_000_000;

// ---------------------------------------------------------------------------
// World loading
// ---------------------------------------------------------------------------

/// A world ready to be shown, plus a human-readable source label.
pub struct LoadedWorld {
    /// Simulation world.
    pub world: World,
    /// Source description shown in the HUD (scenario/save/genome name).
    pub label: String,
    /// Number of bibites spawned while preparing the world.
    pub spawned: usize,
}

impl LoadedWorld {
    /// Loads a world from a save/scenario `.zip` or a `.bb8`/`.bb8template`
    /// genome.
    ///
    /// * `input = None` builds a generated starter world with `fallback_bibites`
    ///   adult starter bibites (used for the "just run it" experience).
    /// * `.zip` files are tried as scenarios first, then as saves, mirroring the
    ///   CLI's `run` command.
    /// * `.bb8`/`.bb8template` files spawn a single bibite in a default world,
    ///   exactly like `rustibites run <genome>`.
    ///
    /// `threads` follows [`World::set_threads`] semantics (`0` = automatic).
    pub fn load(
        input: Option<&Path>,
        seed: u64,
        fallback_bibites: usize,
        threads: usize,
    ) -> Result<Self, String> {
        let mut loaded = match input {
            None => starter_world(seed, fallback_bibites)?,
            Some(path) => {
                let extension = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                match extension.as_str() {
                    "zip" => load_zip(path, seed)?,
                    "bb8" | "bb8template" => load_genome_world(path, seed)?,
                    other => {
                        return Err(format!(
                            "unsupported file type `.{other}` for {}: expected a `.zip` \
                             world/save/scenario or a `.bb8`/`.bb8template` genome",
                            path.display()
                        ));
                    }
                }
            }
        };
        loaded.world.set_threads(threads);
        Ok(loaded)
    }
}

impl LoadedWorld {
    /// Builds the world of `ScenarioSelectorPanel.StartChallenge`: the scenario with the champion's row added (tagged `Champion`,
    /// `LoadForChampion`), plus the challenge's setup when it is started as a challenge. A scenario that is not a challenge, or a
    /// challenge started without a champion (its button stays disabled in the game), is an error.
    pub fn load_challenge_start(
        start: &crate::challenge_run::ChallengeStart,
        seed: u64,
        threads: usize,
    ) -> Result<(Self, Option<crate::challenge_run::ChallengeSetup>), String> {
        let bundle =
            bibites_io::load_scenario_bundle(&start.scenario).map_err(|error| error.to_string())?;
        let challenge = bundle.settings_file.challenge.clone();
        if start.as_challenge && challenge.is_none() {
            return Err(format!("{} is not a challenge", start.scenario.display()));
        }
        if start.as_challenge && start.champion.is_none() {
            return Err("a challenge needs a champion".to_owned());
        }
        // `BibiteSettings.LoadForChampion`: the champion is always tagged `Champion` (custom tagging).
        let mut parameters =
            challenge.unwrap_or_else(bibites_core::settings::ChallengeParameters::game_defaults);
        let extra = start.champion.as_ref().map(|champion| {
            let mut row = parameters.champion.clone();
            let file_name = champion
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            row.path = file_name;
            row.is_external = true;
            row.tagging = Tagging::CustomTagging;
            row.tag = "Champion".to_owned();
            row.total_spawned = 0;
            (row, champion.path.as_path())
        });
        if let Some((row, _)) = extra.as_ref() {
            parameters.champion = row.clone();
        }
        let mut loaded = load_scenario_with(&start.scenario, seed, extra)?;
        loaded.world.set_threads(threads);
        let setup = match (&start.champion, start.as_challenge) {
            (Some(champion), true) => Some(crate::challenge_run::ChallengeSetup {
                scenario: start.scenario.clone(),
                name: bundle.info.name.clone(),
                description: bundle.info.description.clone(),
                stars: [0, 1, 2].map(|index| bundle.info.stars[index].clone().unwrap_or_default()),
                parameters,
                champion: champion.clone(),
            }),
            _ => None,
        };
        Ok((loaded, setup))
    }
}

/// Builds a default world seeded with `count` starter adult bibites.
fn starter_world(seed: u64, count: usize) -> Result<LoadedWorld, String> {
    let settings = SimSettings::game_defaults();
    let scenario = ScenarioData::with_default_zone_group();
    let mut world = World::new(settings, scenario, seed)
        .map_err(|error| format!("could not build starter world: {error}"))?;
    let _ = world.seed_scenario_world();
    let count = count.max(1);
    for index in 0..count {
        spawn_one(
            &mut world,
            &Genome::starter(),
            &GenomeMeta::default(),
            &Brain::starter(),
            SpawnVitals::Adult,
            index as u64,
        )?;
    }
    Ok(LoadedWorld {
        world,
        label: format!("generated starter world ({count} bibites)"),
        spawned: count,
    })
}

/// Loads a `.zip` as a scenario, falling back to the save format.
fn load_zip(path: &Path, seed: u64) -> Result<LoadedWorld, String> {
    match load_scenario(path, seed) {
        Ok(loaded) => Ok(loaded),
        Err(scenario_error) => load_save(path, seed).map_err(|save_error| {
            format!(
                "{} is neither a scenario ({scenario_error}) nor a save ({save_error})",
                path.display()
            )
        }),
    }
}

/// Builds a fresh world from a scenario bundle and spawns its starting rows.
///
/// This mirrors `bibites-cli`'s `run <scenario.zip>` path (the viewer must not
/// edit the CLI, so the wiring is repeated here deliberately).
fn load_scenario(path: &Path, seed: u64) -> Result<LoadedWorld, String> {
    load_scenario_with(path, seed, None)
}

/// [`load_scenario`] with one more spawn row and its template file, added to the scenario's own rows before the world is built
/// (a challenge's champion: `ScenarioSelectorPanel.StartChallenge` adds `championSettings` to `ScenarioSettings.bibites`, so the
/// spawner's first pass already spawns it).
fn load_scenario_with(
    path: &Path,
    seed: u64,
    extra: Option<(bibites_core::settings::BibiteSpawn, &Path)>,
) -> Result<LoadedWorld, String> {
    let mut bundle = bibites_io::load_scenario_bundle(path).map_err(|error| error.to_string())?;
    let label = format!(
        "scenario {} ({})",
        bundle.info.name, bundle.info.version_text
    );
    let mut templates = bundle.templates;
    if let Some(dir) = default_templates_dir() {
        let external = load_template_dir(&dir)?;
        if !external.is_empty() {
            eprintln!(
                "loaded {} template(s) from {}",
                external.len(),
                dir.display()
            );
        }
        templates.extend(external);
    }
    if let Some((row, template_file)) = extra {
        // The champion's file is registered under its own file name when the Library folder did not already bring it
        // (a template of the Saved tab, or one from another folder).
        let file_name = template_file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !templates.iter().any(|(name, _)| *name == file_name) {
            let loaded = bibites_io::load_template(template_file)
                .or_else(|_| bibites_io::load_genome(template_file))
                .map_err(|error| format!("champion {}: {error}", template_file.display()))?;
            templates.push((file_name, loaded));
        }
        bundle.scenario.bibite_spawns.push(row);
    }
    let mut world = World::new(bundle.settings, bundle.scenario, seed)
        .map_err(|error| format!("could not build scenario world: {error}"))?;
    let _ = world.seed_scenario_world();
    // Same fresh-scenario spawner as `rustibites run <scenario.zip>`: one bibite
    // immediately, then `virginSpawnRate` (default 1 Hz) until each spawn row's
    // minimum is met. (The old up-front mass spawn made the viewer start with the
    // full population, unlike the game and the CLI.)
    let template_sources: Vec<TemplateSource> = templates
        .iter()
        .map(|(name, loaded)| {
            TemplateSource::new(
                name.clone(),
                loaded.genome.clone(),
                loaded.meta.clone(),
                loaded.brain.clone(),
            )
            .with_species_name(Some(loaded.game_species_name(name)))
        })
        .collect();
    world.set_templates(template_sources);
    let spawned = world.start_fresh_scenario_spawner();
    Ok(LoadedWorld {
        world,
        label,
        spawned,
    })
}

/// `SaveSystem.LoadWorld` -> `SimulationManager.ResumeSpawners` -> `BibiteSpawner.StartSpawner`: a loaded world runs its default-bibite spawner like a new one (a save whose
/// scenario has a default bibite below its minimum spawns more of it). The templates the rows name are read from the player's folders (a save's own are put there on load).
fn resume_spawner_of_loaded_world(world: &mut World, storage: &bibites_io::storage::StoragePaths) {
    if world.bibite_spawns.is_empty() {
        return;
    }
    let mut sources: Vec<TemplateSource> = world.templates.clone();
    for row in world.bibite_spawns.clone() {
        let file_name = row.path.rsplit(['/', '\\']).next().unwrap_or(&row.path).to_owned();
        if file_name.is_empty() || sources.iter().any(|source| source.name == file_name || source.name == row.path) {
            continue;
        }
        let candidates = [PathBuf::from(&row.path), storage.templates_dir().join(&file_name), storage.bibites_dir().join(&file_name)];
        let Some(path) = candidates.iter().find(|path| path.is_file()) else {
            continue;
        };
        if let Ok(loaded) = bibites_io::load_template(path).or_else(|_| bibites_io::load_genome(path)) {
            let species_name = loaded.game_species_name(&file_name);
            sources.push(
                TemplateSource::new(file_name, loaded.genome, loaded.meta, loaded.brain).with_species_name(Some(species_name)),
            );
        }
    }
    world.set_templates(sources);
    let _ = world.resume_spawner_after_load();
}

/// Builds a world from a save `.zip`.
fn load_save(path: &Path, seed: u64) -> Result<LoadedWorld, String> {
    let mut contents =
        bibites_io::load_world_contents(path, None).map_err(|error| error.to_string())?;
    contents.seed = seed;
    let label = format!("save {} ({})", contents.scene.name, contents.scene.version);
    let spawned = contents.bibites.len();
    let mut world = World::from_contents(contents)
        .map_err(|error| format!("could not build world from save: {error}"))?;
    resume_spawner_of_loaded_world(&mut world, &bibites_io::storage::StoragePaths::default_content());
    Ok(LoadedWorld {
        world,
        label,
        spawned,
    })
}

/// Builds a default world with a single bibite loaded from a genome file.
fn load_genome_world(path: &Path, seed: u64) -> Result<LoadedWorld, String> {
    let is_template = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8template"));
    let loaded = if is_template {
        bibites_io::load_template(path)
    } else {
        bibites_io::load_genome(path)
    }
    .map_err(|error| error.to_string())?;

    let mut world = World::new(
        SimSettings::game_defaults(),
        ScenarioData::with_default_zone_group(),
        seed,
    )
    .map_err(|error| format!("could not build world: {error}"))?;
    let _ = world.seed_scenario_world();
    spawn_one(
        &mut world,
        &loaded.genome,
        &loaded.meta,
        &loaded.brain,
        SpawnVitals::Adult,
        0,
    )?;
    Ok(LoadedWorld {
        world,
        label: format!("genome {}", path.display()),
        spawned: 1,
    })
}

/// Spawns one bibite at a deterministic random position inside the world bounds.
///
/// `spawn_index` gives each spawn its own RNG stream: `Rng::fork` is
/// position-independent, so re-forking the same tag would stack every bibite at
/// the same spot.
fn spawn_one(
    world: &mut World,
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
    vitals: SpawnVitals,
    spawn_index: u64,
) -> Result<(), String> {
    let mut rng = world
        .rng
        .fork(0xB1B1_7E50 ^ spawn_index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let min = world.bounds.min();
    let max = world.bounds.max();
    let margin = (max - min).length() * 0.05;
    let low = min + Vec2::splat(margin);
    let high = max - Vec2::splat(margin);
    let pos = Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y));
    let spawn = BibiteSpawn {
        pos,
        rot: rng.f32_range(0.0, std::f32::consts::TAU),
        vel: Vec2::ZERO,
        ang_vel: 0.0,
        genome: genome.clone(),
        meta: meta.clone(),
        // `CopyBrain` + `FinalizeEditing`: a spawned bibite's nodes start fresh, whatever values the template's brain was saved with.
        brain: {
            let mut fresh = brain.clone();
            fresh.reset_runtime_state(world.settings.brain_period());
            fresh
        },
        vitals,
        energy: None,
        health: None,
    };
    let id = world
        .bibites
        .spawn(spawn)
        .map_err(|error| format!("could not spawn bibite: {error}"))?;
    world.register_bibite(id);
    Ok(())
}

/// The installed game's user template folder, when present.
fn default_templates_dir() -> Option<PathBuf> {
    let user_profile = std::env::var_os("USERPROFILE")?;
    let dir = PathBuf::from(user_profile)
        .join("AppData")
        .join("LocalLow")
        .join("The Bibites")
        .join("The Bibites")
        .join("Bibites")
        .join("Templates");
    dir.is_dir().then_some(dir)
}

/// Loads every `.bb8template` in a directory (sorted by file name).
fn load_template_dir(dir: &Path) -> Result<Vec<(String, bibites_io::LoadedGenome)>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|error| {
            format!(
                "could not read template directory {}: {error}",
                dir.display()
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8template"))
        })
        .collect();
    paths.sort();
    let mut templates = Vec::with_capacity(paths.len());
    for path in paths {
        match bibites_io::load_template(&path) {
            Ok(loaded) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                templates.push((name, loaded));
            }
            // User template folders can contain broken files; one bad file must
            // not stop the viewer from opening a world.
            Err(error) => eprintln!(
                "warning: skipping unreadable template {}: {error}",
                path.display()
            ),
        }
    }
    Ok(templates)
}

// ---------------------------------------------------------------------------
// Overlay data
// ---------------------------------------------------------------------------

/// One active species row in the HUD legend.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesLegendEntry {
    /// Persistent species id.
    pub id: u32,
    /// Display name (`genus + epithet`).
    pub name: String,
    /// Living members + incubating eggs (game `Species.count`).
    pub count: u32,
    /// Swatch colour derived from the founder's skin-pigment genes.
    pub color: [u8; 3],
}

/// Aggregated world statistics shown in the HUD.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudInfo {
    /// Living bibites (corpses excluded).
    pub population: u32,
    /// Corpse bodies still in the body store.
    pub corpses: u32,
    /// Eggs incubating.
    pub eggs: u32,
    /// Active species count (game `activeSpeciesList`).
    pub species: u32,
    /// Mean generation of living bibites.
    pub generation_mean: f32,
    /// Mean age of living bibites, in simulated hours.
    pub age_mean_hours: f32,
    /// Live plant pellets.
    pub plant_pellets: u32,
    /// Live meat pellets.
    pub meat_pellets: u32,
    /// Plant pellet energy (E).
    pub plant_biomass: f32,
    /// Meat pellet energy (E).
    pub meat_biomass: f32,
    /// Active species legend (largest species first, capped at
    /// [`MAX_HUD_SPECIES`]).
    pub species_legend: Vec<SpeciesLegendEntry>,
}

impl HudInfo {
    /// Scans the world stores. O(population); call at [`HUD_REFRESH_SECS`].
    #[must_use]
    pub fn collect(world: &World) -> Self {
        let mut info = Self {
            eggs: world.eggs.len() as u32,
            species: world.species.active_species_count() as u32,
            ..Self::default()
        };
        let counts = world.pellets.counts();
        info.plant_pellets = counts.plant_count;
        info.meat_pellets = counts.meat_count;
        info.plant_biomass = counts.plant_biomass as f32;
        info.meat_biomass = counts.meat_biomass as f32;

        let mut generation_sum = 0.0_f64;
        let mut age_sum = 0.0_f64;
        let mut generation_n = 0_u32;
        for (_, record) in world.bibites.iter() {
            if record.is_corpse {
                info.corpses += 1;
                continue;
            }
            info.population += 1;
            generation_sum += f64::from(record.meta.generation);
            age_sum += f64::from(record.clock.time_alive);
            generation_n += 1;
        }
        if generation_n > 0 {
            info.generation_mean = (generation_sum / f64::from(generation_n)) as f32;
            info.age_mean_hours = (age_sum / f64::from(generation_n) / 3600.0) as f32;
        }

        // Species legend: the game's template snapshot carries the founder's
        // genome, so its ColorR/G/B genes are the best available species swatch.
        let mut legend: Vec<SpeciesLegendEntry> = world
            .species
            .active_species()
            .iter()
            .filter_map(|species_id| {
                let record = world.species.get(*species_id)?;
                // `GenesToColor` of the founder's pigment genes: the colour its lineage mesh uses.
                let color = pigment_rgb([
                    record.template.genome.gene(Gene::ColorR),
                    record.template.genome.gene(Gene::ColorG),
                    record.template.genome.gene(Gene::ColorB),
                ]);
                Some(SpeciesLegendEntry {
                    id: species_id.value(),
                    name: record.name(),
                    count: record.stats.count,
                    color,
                })
            })
            .collect();
        legend.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
        legend.truncate(MAX_HUD_SPECIES);
        info.species_legend = legend;
        info
    }
}

/// What the inspector panels are about: the game's `GUIManager.target` of type `Bibite` (a living bibite or its
/// corpse) or `Egg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionTarget {
    /// A bibite (`BibiteBody`).
    Bibite(BibiteId),
    /// An incubating egg (`EggHatching`).
    Egg(bibites_core::ids::EggId),
}

impl std::fmt::Display for SelectionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bibite(id) => write!(f, "{id}"),
            Self::Egg(id) => write!(f, "egg {id}"),
        }
    }
}

/// Snapshot of the selected bibite or egg for the inspector panels.
///
/// For an egg the species, generation, genes, brain and colour are the egg's own (`BibiteGenes` and `NEATBrain` sit on the
/// egg object too, which is what the Genes and Brain panels read), [`SelectionPanel::egg`] holds what its Stats panel
/// shows, and the fields that only a living bibite has (organs, energy, health, age, travelled distance...) are zero.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionPanel {
    /// The bibite or egg this is a snapshot of, for display.
    pub target: SelectionTarget,
    /// The egg section of the Stats panel (`BibiteStatsPanel.UpdateEggStats`); `None` for a bibite.
    pub egg: Option<bibites_core::body::EggStatsView>,
    /// Assigned species id, when the world has one.
    pub species_id: Option<u32>,
    /// Species display name (`genus + epithet`), when assigned.
    pub species_name: String,
    /// Generation number.
    pub generation: u32,
    /// Age in simulated seconds.
    pub age_seconds: f32,
    /// Active energy (E).
    pub energy: f32,
    /// Maximum energy (E).
    pub max_energy: f32,
    /// Health (HP).
    pub health: f32,
    /// Maximum health (HP).
    pub max_health: f32,
    /// Body 1-D size.
    pub size: f32,
    /// Diet gene (0 herbivore .. 1 carnivore).
    pub diet: f32,
    /// Brain node count.
    pub brain_nodes: u32,
    /// Brain synapse count.
    pub brain_synapses: u32,
    /// Stomach content (u²).
    pub stomach: f32,
    /// Stomach capacity (u²).
    pub stomach_capacity: f32,
    /// Stored fat (u²).
    pub fat: f32,
    /// Maximum stored fat (u²).
    pub fat_max: f32,
    /// Fat inflation factor (0..=1).
    pub fat_inflate: f32,
    /// Egg-organ progress (integers are completed eggs).
    pub egg_progress: f32,
    /// Egg capacity (`floor(area / egg_size)`).
    pub egg_capacity: u32,
    /// `SizeRatio` gene (body size).
    pub size_gene: f32,
    /// `SpeedRatio` gene (metabolism speed).
    pub speed_gene: f32,
    /// The 15 brain outputs in enum order.
    pub brain_outputs: [f32; N_OUTPUTS],
    /// Corpse flag.
    pub is_corpse: bool,
    /// Speciation outlier flag.
    pub is_mutant: bool,
    /// Skin colour (0..=1): `GenesToColor` of the pigment genes, what the game's materials get.
    pub color: [f32; 3],
    /// Body mass (g, body + organs).
    pub mass: f32,
    /// Brain energy upkeep (E/s).
    pub brain_upkeep: f32,
    /// Base metabolism (E/s).
    pub metabolism: f32,
    /// Body area (u²).
    pub body_area: f32,
    /// Armor shell thickness (u).
    pub armor_thickness: f32,
    /// Armor shell strength.
    pub armor_strength: f32,
    /// All 35 gene values (genes tab).
    pub genes: [f32; N_GENES],
    /// Body length (`bodyLength`, u).
    pub body_length: f32,
    /// Real body area incl. inflation (`realBodyArea`, u2).
    pub real_body_area: f32,
    /// Signed forward velocity (`SignedVelocity`).
    pub signed_velocity: f32,
    /// Total distance travelled (`totalTravel`).
    pub travelled: f32,
    /// Eggs laid (`nEggsLaid`).
    pub eggs_laid: u32,
    /// Bibites bitten (`bibitesBitten`).
    pub bibites_bitten: u32,
    /// Total damage dealt by bites (`totalDamageDealt`).
    pub damage_dealt: f32,
    /// Species tag (`BibiteGenes.speciesTag`), if any.
    pub species_tag: Option<String>,
    /// Gene values of the bibite's species template (for the Genes panel's
    /// difference column).
    pub species_genes: Option<[f32; N_GENES]>,
    /// Gene / brain distance to the species template divided by the used species
    /// span (`GenesPanel.cs:66`, `BrainPanel.cs:38`), as fractions.
    pub gene_distance: Option<f32>,
    /// See [`SelectionPanel::gene_distance`].
    pub brain_distance: Option<f32>,
    /// Living children and unhatched eggs of this bibite (`eggLayer.children.Count`).
    pub alive_children: u32,
    /// Times attacked (`timesAttacked`).
    pub times_attacked: u32,
    /// Total damage suffered (`totalDamageSuffered`).
    pub damage_received: f32,
    /// Biology tab rows (`BiologyPanel`): label and formatted value.
    pub biology: Vec<(&'static str, String)>,
    /// What the Internals picture of the Biology tab is drawn from (a bibite only).
    pub internals: Option<crate::internals::Params>,
    /// Whether the bibite is dead (`BiologyPanel.CheckAndSetAliveOrDead`).
    pub dead: bool,
    /// `BibiteGrowth.maturity`, the Biology panel's Growth bar.
    pub maturity: f32,
    /// `BibiteBody.age` (hours), the Age bar.
    pub age_hours: f32,
    /// `BibiteBody.agingThreshold` (hours), the Age bar's marker.
    pub aging_threshold: f32,
    /// `BibiteBody.EnergyFlowRate` (E/s): the Biology panel's Flow bar.
    pub energy_flow: f32,
    /// What the Biology panel's organ pop-ups show (a bibite only).
    pub organs: Option<crate::biology_popups::OrganInfo>,
    /// The stomach as the Biology panel shows it (a bibite only).
    pub stomach_info: Option<crate::internals::StomachInfo>,
    /// Brain tab rows: network size, every output then every input with its live value.
    pub brain_rows: Vec<(&'static str, String)>,
}

impl SelectionPanel {
    /// Looks up a bibite and snapshots its vitals.
    #[must_use]
    pub fn collect(world: &World, id: BibiteId) -> Option<Self> {
        let record = world.bibites.get(id)?;
        let species_id = record.meta.species;
        let species_name = species_name_of(world, species_id);
        let mut brain_outputs = [0.0; N_OUTPUTS];
        for output in ALL_OUTPUTS {
            brain_outputs[output.index()] = record.brain.output_value(output);
        }
        Some(Self {
            target: SelectionTarget::Bibite(id),
            egg: None,
            species_id: species_id.map(SpeciesId::value),
            species_name,
            generation: record.meta.generation,
            age_seconds: record.clock.time_alive,
            energy: record.body.energy.amount,
            max_energy: record.body.energy.max_amount,
            health: record.body.health.amount,
            max_health: record.body.health.max_amount,
            size: record.body.geometry.d1_size,
            diet: record.genome.gene(Gene::Diet),
            brain_nodes: record.brain.node_count() as u32,
            brain_synapses: record.brain.synapse_count() as u32,
            stomach: record.stomach.total_content,
            stomach_capacity: record.stomach.area,
            fat: record.fat.amount,
            fat_max: record.fat.max_amount,
            fat_inflate: record.fat.inflate_factor,
            egg_progress: record.egg_layer.progress,
            egg_capacity: record.egg_layer.capacity,
            size_gene: record.genome.gene(Gene::SizeRatio),
            speed_gene: record.genome.gene(Gene::SpeedRatio),
            brain_outputs,
            is_corpse: record.is_corpse,
            is_mutant: record.meta.is_mutant,
            color: crate::assets::genes_to_color(record.color[0], record.color[1], record.color[2]),
            mass: record.derived.mass,
            brain_upkeep: record.derived.brain_upkeep,
            metabolism: record.body.metabolism,
            body_area: record.body.geometry.base_body_area,
            armor_thickness: record.armor.thickness,
            armor_strength: record.armor.strength,
            genes: *record.genome.values(),
            body_length: record.body.geometry.body_length,
            real_body_area: record.body.geometry.real_body_area,
            signed_velocity: record.propulsion.signed_velocity,
            travelled: record.propulsion.total_travel,
            eggs_laid: record.egg_layer.n_laid,
            bibites_bitten: record.mouth.bibites_bitten,
            damage_dealt: record.mouth.total_damage_dealt,
            species_tag: record
                .meta
                .species_tag
                .clone()
                .filter(|tag| !tag.is_empty()),
            species_genes: species_id
                .and_then(|species| world.species.get(species))
                .map(|species| *species.template.genome.values()),
            gene_distance: species_distance(world, species_id, |template| {
                template.gene_distance(&record.genome)
            }),
            brain_distance: species_distance(world, species_id, |template| {
                template.brain_distance(&record.brain, &world.settings)
            }),
            alive_children: alive_children(world, id),
            times_attacked: record.body.times_attacked,
            damage_received: record.body.total_damage_suffered,
            biology: biology_rows(record),
            internals: Some(crate::internals::params_of(record)),
            stomach_info: Some(crate::internals::stomach_info_of(record)),
            organs: Some(crate::biology_popups::organ_info_of(record, &world.settings)),
            energy_flow: record.energy_flow_rate(),
            dead: record.body.dead,
            maturity: record.growth.maturity(&record.growth_genes),
            age_hours: record.clock.time_alive / 3600.0,
            aging_threshold: world.settings.scenario.ageing_threshold / record.body.params.metabolic_rate,
            brain_rows: brain_rows(record),
        })
    }

    /// Looks up an egg and snapshots what the inspector panels show for it (`BibiteStatsPanel.FillPanel` with an
    /// `EggHatching` target). `BibiteGenes` and `NEATBrain` are components of the egg, so the Genes and Brain panels work
    /// on the egg's own genes and brain; the Biology panel does not exist for an egg.
    #[must_use]
    pub fn collect_egg(world: &World, id: bibites_core::ids::EggId) -> Option<Self> {
        let record = world.eggs.get(id)?;
        let species_id = record.meta.species;
        let genes = *record.genome.values();
        Some(Self {
            target: SelectionTarget::Egg(id),
            egg: Some(record.stats_view(&world.settings)),
            species_id: species_id.map(SpeciesId::value),
            species_name: species_name_of(world, species_id),
            generation: record.meta.generation,
            age_seconds: 0.0,
            energy: record.hatch.energy,
            max_energy: record.hatch.energy,
            health: 0.0,
            max_health: 0.0,
            size: record.size_gene,
            diet: record.genome.gene(Gene::Diet),
            brain_nodes: record.brain.node_count() as u32,
            brain_synapses: record.brain.synapse_count() as u32,
            stomach: 0.0,
            stomach_capacity: 0.0,
            fat: 0.0,
            fat_max: 0.0,
            fat_inflate: 0.0,
            egg_progress: 0.0,
            egg_capacity: 0,
            size_gene: record.genome.gene(Gene::SizeRatio),
            speed_gene: record.genome.gene(Gene::SpeedRatio),
            brain_outputs: [0.0; N_OUTPUTS],
            is_corpse: false,
            is_mutant: record.meta.is_mutant,
            color: crate::assets::genes_to_color(genes[5], genes[6], genes[7]),
            mass: 0.0,
            brain_upkeep: 0.0,
            metabolism: 0.0,
            body_area: 0.0,
            armor_thickness: 0.0,
            armor_strength: 0.0,
            genes,
            body_length: 0.0,
            real_body_area: 0.0,
            signed_velocity: 0.0,
            travelled: 0.0,
            eggs_laid: 0,
            bibites_bitten: 0,
            damage_dealt: 0.0,
            species_tag: record
                .meta
                .species_tag
                .clone()
                .filter(|tag| !tag.is_empty()),
            species_genes: species_id
                .and_then(|species| world.species.get(species))
                .map(|species| *species.template.genome.values()),
            gene_distance: species_distance(world, species_id, |template| {
                template.gene_distance(&record.genome)
            }),
            brain_distance: species_distance(world, species_id, |template| {
                template.brain_distance(&record.brain, &world.settings)
            }),
            alive_children: 0,
            times_attacked: 0,
            damage_received: 0.0,
            biology: Vec::new(),
            internals: None,
            stomach_info: None,
            organs: None,
            energy_flow: 0.0,
            dead: false,
            maturity: 0.0,
            age_hours: 0.0,
            aging_threshold: 1.0,
            brain_rows: Vec::new(),
        })
    }
}

/// A species' display name; "unassigned" for a bibite that belongs to none.
fn species_name_of(world: &World, species: Option<SpeciesId>) -> String {
    species
        .and_then(|species| world.species.get(species))
        .map(|species| species.name())
        .unwrap_or_else(|| "unassigned".to_owned())
}

/// Brain tab rows: node/synapse counts, the 15 outputs and the 30 inputs with
/// their live values, labelled with the game's node names.
fn brain_rows(record: &bibites_core::body::BibiteRecord) -> Vec<(&'static str, String)> {
    use bibites_core::brain::{ALL_INPUTS, ALL_OUTPUTS};
    let mut rows = vec![(
        "network",
        format!(
            "{} nodes / {} synapses",
            record.brain.node_count(),
            record.brain.synapse_count()
        ),
    )];
    for output in ALL_OUTPUTS {
        rows.push((
            output.name(),
            format!("{:+.3}", record.brain.output_value(output)),
        ));
    }
    for input in ALL_INPUTS {
        rows.push((
            input.name(),
            format!("{:+.3}", record.brain.input_value(input)),
        ));
    }
    rows
}

/// One edit of the selected editor connection.
#[derive(Clone, Copy)]
enum SynapseEdit {
    /// Add to the weight.
    Weight(f32),
    /// Flip `enabled`.
    Toggle,
    /// Delete the connection.
    Remove,
}

/// `distance(template) / usedSpeciesSpan` for the bibite's species.
fn species_distance(
    world: &World,
    species: Option<SpeciesId>,
    distance: impl Fn(&bibites_core::species::TemplateSnapshot) -> f32,
) -> Option<f32> {
    let record = world.species.get(species?)?;
    // `GlobalLineageManager.usedSpeciesSpan` as the registry holds it: the game's panels read that static, which is refreshed when a species is
    // created, not each time a species dies out (a recount of the active list here showed a ratio the simulation was not judging by).
    let span = world.species.current_used_span(&world.settings);
    (span > 0.0).then(|| distance(&record.template) / span)
}

/// Living bibites and incubating eggs whose first parent is `id`.
fn alive_children(world: &World, id: BibiteId) -> u32 {
    let bibites = world
        .bibites
        .iter()
        .filter(|(_, record)| !record.body.dead && record.meta.parent1 == Some(id))
        .count();
    let eggs = world
        .eggs
        .ids()
        .filter_map(|egg| world.eggs.get(egg))
        .filter(|egg| egg.meta.parent1 == Some(id))
        .count();
    (bibites + eggs) as u32
}

/// Game version stamped into saves (`Application.version`, as shown on the title screen).
pub const GAME_VERSION: &str = "0.6.3.1";

/// UTC calendar fields `(year, month, day, hour, minute, second)` from the system clock.
fn utc_now_parts() -> (i64, i64, i64, i64, i64, i64) {
    calendar_parts(unix_now())
}

/// Seconds since the Unix epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs()) as i64
}

/// Calendar fields `(year, month, day, hour, minute, second)` of `secs` since the Unix epoch.
fn calendar_parts(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day, rem / 3_600, (rem % 3_600) / 60, rem % 60)
}

/// `yyyyMMddHHmmss` of the machine's local time (`DateTime.Now` in the game's
/// `autosave_<stamp>.zip` names, so the files sort with the game's own autosaves). Falls back to
/// UTC when the local offset is unknown (no PowerShell reader).
fn autosave_timestamp() -> String {
    let offset = bibites_io::prefs::GamePrefs::global()
        .utc_offset_minutes()
        .unwrap_or(0);
    autosave_timestamp_at(unix_now(), offset)
}

/// [`autosave_timestamp`] for `secs` since the epoch at `offset_minutes` from UTC.
fn autosave_timestamp_at(secs: i64, offset_minutes: i64) -> String {
    let (year, month, day, hour, minute, second) = calendar_parts(secs + offset_minutes * 60);
    format!("{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}")
}

/// ISO-8601 UTC time (`DateTime.ToString("O")` in the game's score records).
fn utc_iso8601() -> String {
    let (year, month, day, hour, minute, second) = utc_now_parts();
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Organ and energy readout for the Biology tab. The game's `BiologyPanel`
/// (cs:637-1078) shows ~90 live fields; these are the ones the core records
/// carry, formatted with the game's `ScientificFormat`.
fn biology_rows(record: &bibites_core::body::BibiteRecord) -> Vec<(&'static str, String)> {
    use bibites_core::format::FloatValueFormat;
    let fmt = |units: &'static str| FloatValueFormat {
        units,
        precision: 3,
        precision_is_si: true,
        ..FloatValueFormat::default()
    };
    let geometry = &record.body.geometry;
    vec![
        (
            "health",
            format!(
                "{} / {}",
                fmt("HP").format(record.body.health.amount),
                fmt("HP").format(record.body.health.max_amount)
            ),
        ),
        (
            "energy",
            format!(
                "{} / {}",
                fmt("E").format(record.body.energy.amount),
                fmt("E").format(record.body.energy.max_amount)
            ),
        ),
        ("metabolism", fmt("E/s").format(record.body.metabolism)),
        (
            "brain upkeep",
            fmt("E/s").format(record.derived.brain_upkeep),
        ),
        ("d1 size", fmt("u").format(geometry.d1_size)),
        ("d2 size", fmt("u").format(geometry.d2_size)),
        ("body length", fmt("u").format(geometry.body_length)),
        ("body area", fmt("u2").format(geometry.real_body_area)),
        ("mass", fmt("g").format(record.derived.mass)),
        (
            "stomach",
            format!(
                "{} / {}",
                fmt("u2").format(record.stomach.total_content),
                fmt("u2").format(record.stomach.area)
            ),
        ),
        (
            "fat",
            format!(
                "{} / {}",
                fmt("u2").format(record.fat.amount),
                fmt("u2").format(record.fat.max_amount)
            ),
        ),
        ("fat inflate", fmt("").format(record.fat.inflate_factor)),
        (
            "womb",
            format!(
                "{:.2} / {} eggs",
                record.egg_layer.progress, record.egg_layer.capacity
            ),
        ),
        ("egg energy", fmt("E").format(record.egg_layer.egg_energy)),
        (
            "jaw strength",
            fmt("").format(record.mouth_derived.jaw_strength),
        ),
        (
            "bite period",
            fmt("s").format(record.mouth_derived.bite_period),
        ),
        ("bite progress", fmt("s").format(record.mouth.bite_progress)),
        (
            "throat width",
            fmt("u").format(record.mouth_derived.throat_width),
        ),
        (
            "move strength",
            fmt("").format(record.propulsion_derived.muscle_move_strength),
        ),
        (
            "turn strength",
            fmt("").format(record.propulsion_derived.muscle_turn_strength),
        ),
        ("armor thickness", fmt("u").format(record.armor.thickness)),
        ("armor strength", fmt("").format(record.armor.strength)),
        (
            "brain",
            format!(
                "{} nodes, {} synapses",
                record.brain.node_count(),
                record.brain.synapse_count()
            ),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// One frame of user input, produced by the window loop (or by tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInput {
    /// Cursor position in logical screen pixels.
    pub cursor: Vec2,
    /// Primary (left) mouse button currently held.
    pub primary_down: bool,
    /// Primary button pressed this frame.
    pub primary_pressed: bool,
    /// Primary button released this frame.
    pub primary_released: bool,
    /// Wheel notches (positive = zoom in).
    pub wheel: f32,
    /// Space pressed: toggle pause.
    pub toggle_pause: bool,
    /// Right arrow pressed: advance exactly one tick (and pause).
    pub step: bool,
    /// `F` pressed: fit the whole world.
    pub fit: bool,
    /// `B` pressed: fit the bibite bounding box (the default view).
    pub fit_bibites: bool,
    /// `R` pressed: reset the camera to the default bibite view.
    pub reset_view: bool,
    /// Digit key 1-4 pressed (`GUIManager.Update`: opens the inspector panels).
    pub digit: Option<u32>,
    /// Digit key 1 is held (`BibiteBrainEditor.UpdatePanel`: the brain simulator steps by its period while it is).
    pub digit1_down: bool,
    /// `G` pressed: toggle the background grid.
    pub toggle_grid: bool,
    /// `Z` pressed: toggle zone overlays.
    pub toggle_zones: bool,
    /// `P` pressed: toggle pellets.
    pub toggle_pellets: bool,
    /// `O` pressed: toggle pheromone spots.
    pub toggle_pheromones: bool,
    /// `H` pressed: toggle the HUD.
    pub toggle_hud: bool,
    /// RUSTibites Improved: the Hide Objects hotkey went down (its key is the player's: `improved_keys`).
    pub toggle_objects: bool,
    /// Escape pressed: pop a panel, deselect, or open the pause menu.
    pub quit: bool,
    /// `R`: select a random bibite (`UserControl.SelectRandomBibite`).
    pub select_random_bibite: bool,
    /// `G`: select the highest-generation bibite.
    pub select_high_generation: bool,
    /// `O`: select the oldest bibite.
    pub select_oldest: bool,
    /// `E`: select a random egg.
    pub select_random_egg: bool,
    /// `Z`: select a random zone (`UserControl.Update`; Ctrl does not matter, so Ctrl + Z also
    /// undoes).
    pub select_random_zone: bool,
    /// `K`: make the auto-panner the target (`UserControl.Update`).
    pub select_autopanner: bool,
    /// `F5`: quick save.
    pub quick_save: bool,
    /// F1, F3 and F4 went down this frame (`BrainPanelCommon.UpdatePanel`: sort the brain diagram once, 6 and 8
    /// times).
    pub brain_sort_keys: [bool; 3],
    /// F2 is held (`SortHiddenOnce` every frame).
    pub brain_hidden_down: bool,
    /// The right Control key is held (`Input.GetKey(305)`: the hidden sort's scalar becomes 0).
    pub right_ctrl_down: bool,
    /// `Insert`: lay an egg with the selected bibite.
    pub lay_egg: bool,
    /// `Delete`: kill / explode the selected bibite.
    pub kill_target: bool,
    /// `Home`: deselect and reset the camera to the origin.
    pub camera_home: bool,
    /// `=` / `-` / numpad: keyboard zoom ratio (`+0.25` / `-0.2`), else 0.
    pub zoom_key: f32,
    /// Right mouse button held (`CameraManager` pans on right-drag).
    pub secondary_down: bool,
    /// Ctrl held (right button + Ctrl slides the camera instead of dragging).
    pub ctrl_down: bool,
    /// Shift held (the lineage tree scrolls sideways with the wheel).
    pub shift_down: bool,
    /// Middle mouse button held / pressed this frame (drag-zoom).
    pub middle_down: bool,
    /// Middle mouse button pressed this frame.
    pub middle_pressed: bool,
    /// `GetAxisRaw("Horizontal"/"Vertical")`: arrow keys + WASD, each -1..1.
    pub move_axis: Vec2,
    /// PageUp (+1) / PageDown or Keypad1 (-1): camera speed scale step.
    pub speed_scale_step: i8,
    /// Ctrl + Z (`UINavigationManager.RevertOneAction`).
    pub undo: bool,
    /// Ctrl + Shift + Z (`UINavigationManager.RedoOneAction`).
    pub redo: bool,
    /// Ctrl + C (`GUIManager.CopyTagOfTarget`).
    pub copy_tag: bool,
    /// Ctrl + V (`GUIManager.TagTarget`, `SelectionPanel.TagTargets`, or pasting into the tag field).
    pub paste_tag: bool,
    /// Backspace (edits the species tag field).
    pub backspace: bool,
    /// Left / Right / Home / End / Delete (the caret of a text field).
    pub key_left: bool,
    /// See `key_left`.
    pub key_right: bool,
    /// See `key_left`.
    pub key_home: bool,
    /// See `key_left`.
    pub key_end: bool,
    /// See `key_left`.
    pub key_delete: bool,
    /// Ctrl + A (select all in a text field).
    pub select_all_key: bool,
    /// Enter (confirms the species tag field).
    pub enter: bool,
    /// Real frame time in seconds (`Time.unscaledDeltaTime`).
    pub dt: f32,
}

/// Current camera behaviour (drives automatic re-fit on window resize).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Frame the living bibite population (default, `B`/`R`).
    #[default]
    Bibites,
    /// Fit the whole simulation square (`F`).
    World,
    /// Explicit centre/zoom (`--center`/`--zoom` or manual panning has no effect;
    /// resize keeps the user's framing).
    Manual,
}

/// `OnHoverShow` of a bottom-left toolbar button: the panel opens once the pointer has rested on the
/// button for [`Self::OPEN_DELAY`] and closes [`Self::CLOSE_DELAY`] after it left the button and
/// the panel; pressing the button opens it at once (`ShowElement`). Both delays are the prefab's
/// 0.5 s, in real time (`Time.unscaledDeltaTime`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HoverPanel {
    /// The panel shows.
    pub open: bool,
    hover: bool,
    pending: bool,
    timer: f32,
}

impl HoverPanel {
    /// `OnHoverShow.openDelay` of the toolbar buttons.
    pub const OPEN_DELAY: f32 = 0.5;
    /// `OnHoverShow.closeDelay` of the toolbar buttons.
    pub const CLOSE_DELAY: f32 = 0.5;

    /// One frame: `over` says whether the pointer is on the button or its panel.
    pub fn update(&mut self, over: bool, dt: f32) {
        if over != self.hover {
            // `OnPointerEnter` / `OnPointerExit` start the delay.
            self.hover = over;
            self.pending = true;
            self.timer = 0.0;
        }
        if self.pending {
            self.timer += dt.max(0.0);
            let delay = if self.hover {
                Self::OPEN_DELAY
            } else {
                Self::CLOSE_DELAY
            };
            if self.timer >= delay {
                self.pending = false;
                self.open = self.hover;
            }
        }
    }

    /// `OnHoverShow.ShowElement` (the button's click).
    pub fn show(&mut self) {
        self.open = true;
    }
}

/// What `UserControl.toRepeatOnDeath` repeats once the selected target is gone (Ctrl + R / G / O).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reselect {
    /// `SelectRandomBibite`.
    Random,
    /// `SelectHighGenBibite`.
    HighestGeneration,
    /// `SelectOldestBibite`.
    Oldest,
    /// `SelectRandomBibiteOfSpecies` (a Ctrl click on a species row of the Information panel).
    Species(SpeciesId),
    /// `SelectRandomBibiteOfTag` (a Ctrl click on a tag row), the tag in `App::reselect_tag`.
    Tag,
}

/// A body the left button grabbed (`DraggableObject`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    /// A bibite or a corpse.
    Bibite(BibiteId),
    /// An egg.
    Egg(bibites_core::ids::EggId),
    /// A free pellet.
    Pellet(bibites_core::ids::PelletId),
    /// A placed tower or colour killer (by save id).
    Tool(u64),
}

/// `DraggableObject.speed`: a dragged body heads for the cursor at `speed * distance` (per
/// time-scaled second); the same number scales the Ctrl + drag turning.
const DRAG_SPEED: f32 = 10.0;

/// The left-button gesture on the world: a click or a drag rectangle
/// (`UserControl.EnableMultiSelect`).
#[derive(Clone, Copy, Debug)]
pub struct DragState {
    /// Cursor position at the press (screen pixels).
    pub start: Vec2,
    /// Latest cursor position.
    pub cursor: Vec2,
    /// Seconds the button has been held (unscaled time).
    pub held: f32,
    /// A bibite or egg body was pressed: its own click handling applies (a quick release selects
    /// it) and no rectangle can start from it.
    pub on_entity: Option<crate::selection::Entity>,
    /// A tower or colour killer (save id) was pressed with no body above it: its own click
    /// handler (`TargetObjectOnClick`) selects it, and no rectangle can start from it either.
    pub on_tool: Option<u64>,
    /// The body under the press, which a held button drags around.
    pub grab: Option<Grab>,
}

impl DragState {
    /// Whether the press has become a selection rectangle (held past the click time).
    #[must_use]
    pub fn is_rectangle(&self) -> bool {
        self.on_entity.is_none()
            && self.on_tool.is_none()
            && self.held >= crate::selection::CLICK_MAX_SECONDS
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// What a template picked in the Library overlay is for
/// (`BibiteTemplateSelectorPanel.OpenWithCallBacks` hands it to the caller's callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateUse {
    /// A new default bibite of the world (`DefaultBibitesPanel.AddBibiteButtonPressed`).
    DefaultBibite,
    /// The template `BibitePlacer` places (`ButtonFromTemplatePressed`).
    PlaceBibite,
}

/// `BibitePlacer` while it is in placement mode: bibites are placed from this template.
#[derive(Clone, Debug)]
pub struct BibitePlacer {
    /// The template placed.
    pub template: TemplateSource,
}

/// Interactive tool property values (pellet placer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolSettings {
    /// Placer pellet material (`PelletPlacer.material`: Plant or Meat).
    pub material: bibites_core::pellet::MaterialKind,
    /// Placer pellets per placement (1..=10).
    pub pellet_count: usize,
    /// Pellet size factor (`PelletPlacer.pelletSize`, 0.1..=1000); the pellet
    /// energy is `scenario.pelletEnergy * pellet_energy`.
    pub pellet_energy: f32,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            material: bibites_core::pellet::MaterialKind::Plant,
            pellet_count: 1,
            pellet_energy: 1.0,
        }
    }
}

/// Open captured dropdown with its option labels and origin rect.
#[derive(Clone, Debug, PartialEq)]
pub struct DropdownState {
    /// Visible option labels.
    pub options: Vec<String>,
    /// Origin control rect `[x, y, w, h]` in logical pixels.
    pub rect: [f32; 4],
    /// Captured session.
    pub session: String,
    /// Captured node path.
    pub path: String,
    /// Visible control label.
    pub label: String,
}

/// One step of the Ctrl + Z history.
#[derive(Clone)]
enum UndoAction {
    /// The world settings the dynamic panel's rows edit: before and after.
    Settings(Box<(SimSettings, SimSettings)>),
    /// One zone's settings (a control of the Zones tab), found again by its persistent id.
    Zone {
        id: Option<u32>,
        before: Box<bibites_core::settings::ZoneSettings>,
        after: Box<bibites_core::settings::ZoneSettings>,
    },
}

/// `UINavigationManager`'s revertable-action stack: each edit of the dynamic settings panel (the
/// scenario settings, the materials, a zone item's controls) keeps what it changed before and after
/// (a slider drag counts once). Ctrl + Z steps back, Ctrl + Shift + Z forward; a new edit drops what
/// could still be redone; 60 edits are kept.
#[derive(Clone, Default)]
pub struct UndoStack {
    actions: Vec<UndoAction>,
    /// Index of the last applied action, `-1` before the first (`revertableStackIndex`).
    index: i32,
}

impl UndoStack {
    /// `UINavigationManager.revertableActionStack` capacity.
    const DEPTH: i32 = 60;

    fn new() -> Self {
        Self {
            actions: Vec::new(),
            index: -1,
        }
    }

    /// `AddRevertableToStack`.
    fn push(&mut self, action: UndoAction) {
        let count = self.actions.len() as i32;
        if self.index < count - 1 {
            self.actions.truncate((self.index + 1) as usize);
        }
        self.actions.push(action);
        if self.index >= Self::DEPTH - 1 {
            self.actions.remove(0);
        } else {
            self.index += 1;
        }
    }

    /// `RevertOneAction`: the step to take back.
    fn revert(&mut self) -> Option<UndoAction> {
        if self.index < 0 {
            return None;
        }
        let action = self.actions[self.index as usize].clone();
        self.index -= 1;
        Some(action)
    }

    /// `RedoOneAction`: the step to do again.
    fn redo(&mut self) -> Option<UndoAction> {
        if self.index >= self.actions.len() as i32 - 1 {
            return None;
        }
        self.index += 1;
        Some(self.actions[self.index as usize].clone())
    }

    /// Drops the steps that edited zones: deleting a zone renumbers the others, so the persistent
    /// ids those steps hold no longer say which zone they meant. The other steps stay in order.
    fn drop_zone_steps(&mut self) {
        let applied = (self.index + 1).max(0) as usize;
        let mut kept = Vec::new();
        let mut kept_applied = 0;
        for (position, action) in self.actions.drain(..).enumerate() {
            if matches!(action, UndoAction::Zone { .. }) {
                continue;
            }
            if position < applied {
                kept_applied += 1;
            }
            kept.push(action);
        }
        self.actions = kept;
        self.index = kept_applied - 1;
    }

    /// `ClearRevertable` (a new scene).
    fn clear(&mut self) {
        self.actions.clear();
        self.index = -1;
    }

    /// How many edits can still be undone.
    #[must_use]
    pub fn undoable(&self) -> usize {
        (self.index + 1).max(0) as usize
    }
}

/// The species tag is cut to this many characters when pasted (`systemCopyBuffer.Substring(0, 100)`).
const TAG_MAX_CHARS: usize = 100;

/// The rows of the fallback Load browser whose zip metadata (version, population, time) is read; the others only have their file's facts.
const SAVE_BROWSER_ROWS: usize = 12;

/// Pending confirmation dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    /// Kill the selected bibite.
    KillSelected,
    /// Kill half of the living population.
    KillHalf,
    /// Remove half of all pellets (`UserActions.RemoveHalfPelletsPressed`).
    RemoveHalfPellets,
    /// Delete every default bibite (`DefaultBibitesPanel.DeleteAllBibites`).
    ClearDefaultBibites,
    /// Delete zone `.0` of the world (`ZoneSettingsHandle.DeleteZone`).
    DeleteZone(usize),
    /// Delete every zone of the world (`ZonesEditorPanel.DeleteAllZones`).
    ClearZones,
    /// Delete settings animator `.0` (`SettingsChangerHandle.DeleteChanger`).
    DeleteAnimator(usize),
    /// Delete every settings animator (`SettingsChangersEditor.DeleteAllChangers`).
    ClearAnimators,
    /// RUSTibites Improved: delete every stream of the Zones tab's Streams list.
    ClearStreams,
    /// Overwrite an existing save name.
    OverwriteSave(String),
    /// Overwrite an existing scenario (the scenario saver's name is taken).
    OverwriteScenario(String),
    /// Delete a save file.
    DeleteSave(PathBuf),
    /// Restart the running challenge (`ChallengeTrackerPanel` Retry).
    RestartChallenge,
    /// Prune species `.0` from the lineage tree (`SpeciesInfoPanel.PruneSpecies`).
    PruneSpecies(bibites_core::ids::SpeciesId),
}

impl ConfirmKind {
    /// The action warning behind the dialog (`PopupManager.DisplayActionWarning`): its popup has
    /// the "do not show again" checkbox, and is skipped once that was ticked. The other dialogs
    /// are plain choice dialogs (`DisplayChoiceDialog`).
    #[must_use]
    pub fn warning(&self) -> Option<crate::warnings::ActionWarning> {
        match self {
            Self::DeleteSave(_) => Some(crate::warnings::ActionWarning::DeleteSaveFile),
            Self::PruneSpecies(_) => Some(crate::warnings::ActionWarning::PruneSpecies),
            _ => None,
        }
    }
}

/// Viewer state: the world, camera, playback controls and overlay data.
pub struct App {
    /// Simulation world (the single source of truth).
    pub world: World,
    /// Active camera.
    pub camera: Camera,
    /// Reused per-frame display list.
    pub scene: Scene,
    /// Human-readable world source label.
    pub label: String,
    /// RNG seed the world was built with (shown in the HUD).
    pub seed: u64,
    /// Paused flag (space toggles).
    pub paused: bool,
    /// Time-speed multiplier the slider asks for (`targetTimeScale`, 0.1..=100).
    pub speed: f32,
    /// Which fraction of [`App::speed`] the simulation runs at (`engineTimeScale / targetTimeScale`, 1 unless the minimum-FPS
    /// governor slowed it down; see [`crate::time_keeper::min_fps_scale`]).
    pub engine_fraction: f32,
    /// Seconds the ticks of the last [`App::advance`] took (the frame pacing's measure of what the rest of a frame costs).
    last_tick_secs: f64,
    /// What a tick costs, averaged over the recent ones (the pacing's expectation of the next).
    tick_cost: f64,
    /// Tests: every tick costs this many seconds (a model machine), instead of the time it took.
    #[cfg(test)]
    model_tick_secs: Option<f64>,
    /// The window runs each frame's ticks on a helper thread while it draws and presents the frame (see [`crate::tick_overlap`]).
    /// Off for tests and the headless runs (their `advance` runs its ticks before it returns).
    pub overlap_ticks: bool,
    /// The helper thread of the overlapped ticks (started by the first frame that needs it).
    tick_worker: Option<crate::tick_overlap::TickWorker>,
    /// The real world is on the tick helper for the frame's ticks; `world` holds a husk (nothing may read it until
    /// [`App::join_overlapped_ticks`]).
    ticks_in_flight: bool,
    /// The empty world that stands in for the real one while it is on the helper (kept, so that no frame builds one).
    world_husk: Option<World>,
    /// `advance` left this frame's ticks owed for [`App::dispatch_overlapped_ticks`] (they stay in the accumulator).
    ticks_deferred: bool,
    /// When the window began the frame: what the frame did before the ticks started is what the ticks' room is short of.
    frame_started: Option<std::time::Instant>,
    /// Ticks the last join brought in (the frame's challenge and autosave clocks count them).
    joined_ticks: u32,
    /// When the last batch of ticks was handed to the helper: the drawing and the wait for the display have run beside it since.
    overlap_dispatched: Option<std::time::Instant>,
    /// How long the drawing and the wait for the display take beside the ticks (an average over the recent frames, seconds): the ticks may run that long
    /// without holding the frame up. A big window on a modest graphics card makes it long, and the ticks' room must not stay a frame of the minimum FPS.
    overlap_hidden: f64,
    /// Seconds the frames of the current measurement of the frame rate waited for ticks that outlasted the drawing, and the batches of ticks of the window.
    overlap_wait_secs: f64,
    overlap_batches: u32,
    /// `wall_time` at which the current measurement of the frame rate began.
    govern_since: f64,
    /// Tests: the drawing and the display of every frame take this many seconds beside the ticks (a model machine, with `model_tick_secs`).
    #[cfg(test)]
    model_beside_secs: Option<f64>,
    /// The simulation thread, when `--sim-thread` asked for one (see [`crate::sim_thread`]).
    pub sim: Option<crate::sim_thread::SimThread>,
    /// RUSTibites Improved's simulation switches for this run (see [`crate::improved_switches`]). Whatever world the app holds gets them before it ticks.
    pub improved: bibites_core::improved::ImprovedFeatures,
    /// The real world is on the simulation thread; `world` holds a husk with its settings.
    world_away: bool,
    /// Frames in a row that only watched (could have gone on without the world).
    light_streak: u32,
    /// The newest snapshot of the world that is away.
    snapshot: Option<std::sync::Arc<crate::sim_thread::Snapshot>>,
    /// Ticks of the simulation thread that have been counted.
    sim_ticks_seen: u64,
    /// Ticks the world ran on the simulation thread that the auto-panner has not made its steps for yet.
    autopan_owed: u64,
    /// The next frame needs the world (an autosave is due).
    world_wanted: bool,
    /// Simulation ticks advanced by the viewer (for the HUD).
    pub sim_ticks: u64,
    /// Currently selected bibite.
    pub selected: Option<BibiteId>,
    /// Cached population statistics.
    pub hud: HudInfo,
    /// Selected bibite panel snapshot (`None` when nothing is selected).
    pub selection: Option<SelectionPanel>,
    /// Window FPS as the game's time panel measures it (0 until the first half second has passed).
    pub fps: i32,
    /// `TimeKeeper`: the measured frames and ticks per second of the time panel.
    pub time_keeper: crate::time_keeper::TimeKeeper,
    /// Real seconds since the viewer started (`Time.realtimeSinceStartup`): the sum of the frame times given to
    /// [`App::advance`], stalls included.
    wall_time: f64,
    /// Current camera behaviour (automatic re-fit on resize when not manual).
    pub view_mode: ViewMode,
    /// Optional scene layers (grid/zones/pellets/pheromones).
    pub scene_options: SceneOptions,
    /// Whether the HUD overlay is drawn.
    pub show_hud: bool,
    /// The inspector panel last chosen (`GUIManager.curMenu`); it shows while `inspector_open`.
    pub inspector_tab: InspectorTab,
    /// Edits of the dynamic settings panel that Ctrl + Z can take back.
    pub undo: UndoStack,
    /// The edit a slider drag is making: what it started from and what it has now.
    undo_drag: Option<UndoAction>,
    /// The species tag being typed in the Stats panel (`BibiteStatsPanel.tagInputField`); the
    /// keyboard hotkeys are blocked meanwhile (`SetKeyboardBlockFromSource("SpeciesTagEdit")`).
    pub tag_edit: Option<String>,
    /// Characters typed this frame (filled by the window loop).
    pub typed: String,
    /// The OS clipboard text, supplied by the window loop on the frame Ctrl + V is pressed.
    pub clipboard_in: Option<String>,
    /// Text for the OS clipboard (Ctrl + C), taken by the window loop.
    pub clipboard_out: Option<String>,
    /// A web address for the window loop to open in the browser (`Application.OpenURL`).
    pub pending_url: Option<String>,
    /// The Library overlay is open to pick a template for this purpose.
    pub template_pick: Option<TemplateUse>,
    /// `BibitePlacer`: set while bibites are placed with the left button.
    pub placer: Option<BibitePlacer>,
    /// The placer's choices (`BibitePlacer.settings` is static: they stay between uses). Only
    /// the tagging, randomize genes, growth at spawn and custom tag fields are used.
    pub placer_options: bibites_core::settings::BibiteSpawn,
    /// The pause the template selector holds (`TogglePauseGame("bibitePlacer")`): the state the
    /// game returns to when it closes.
    placer_pause: Option<bool>,
    /// The tag field being typed into is the placer's custom tag, not the selected bibite's.
    tag_edit_is_placer: bool,
    /// The Save Bibite dialog while it is open (`SaveBibiteDialogHandle`).
    pub save_dialog: Option<crate::save_dialog::SaveDialog>,
    /// The scenario saver while it is open (`ScenarioSaver`, opened by the settings panel's Save button).
    pub scenario_saver: Option<crate::scenario_saver::ScenarioSaver>,
    /// `UserSettings.SaveBibiteAsTemplate`: the dialog's checkbox starts from it.
    pub save_as_template: bool,
    /// `UserSettings.FormatBibiteJSON`: the dialog's checkbox starts from it.
    pub format_bibite_json: bool,
    /// Where the dialog writes its two checkboxes back to (the viewer's user settings file); `None`
    /// keeps them in memory (tests, headless runs).
    pub user_settings_path: Option<PathBuf>,
    /// `(file name, display name)` of the player's templates, read once for the list's names.
    template_names: std::cell::OnceCell<Vec<(String, String)>>,
    /// `UserControl.toRepeatOnDeath`: selects the next bibite when the target is gone.
    pub reselect: Option<Reselect>,
    /// The tag [`Reselect::Tag`] repeats.
    reselect_tag: Option<String>,
    /// Ctrl was held at the last click on a control (`Input.GetKey(LeftControl)`).
    pub ctrl_held: bool,
    /// Shift is held (a text field's selection grows with it).
    pub shift_held: bool,
    /// The Information panel: the section shown, the numbers of the last refresh, the tags list's scroll.
    pub information: crate::information_panel::InfoState,
    /// Seconds (of game time, `WaitForSeconds`) until the repeat fires.
    reselect_timer: Option<f32>,
    /// The target was picked by `SelectBibiteTarget` (R / G / O): its death ends the selection.
    corpse_listener: bool,
    /// `UserSettings.ReselectOnCorpses`.
    pub reselect_on_corpses: bool,
    /// `UserSettings.ReselectionDelay` (seconds).
    pub reselection_delay: f32,
    /// `UserSettings.allowArrowKeyCameraControl`.
    pub allow_arrow_keys: bool,
    /// Whether a bibite panel is chosen (`curMenu != None`). It starts closed: selecting a bibite
    /// shows only the tab buttons until one is pressed. It survives deselecting, so the next
    /// selection reopens the same panel.
    pub inspector_open: bool,
    /// The Expanded Brain View (`ExpandedBrainPanel`) while it is open.
    pub expanded_brain: Option<crate::expanded_brain_ui::ExpandedSession>,
    /// `ExpandedBrainPanel.graphType`: it stays when the view closes.
    pub expanded_graph: crate::expanded_brain::GraphType,
    /// `BrainHolder.anchoredPosition`: it stays when the view closes (the next brain opens on the same canvas place).
    pub expanded_pan: [f32; 2],
    /// The settings panel was open when the view closed: it is a child of the view with a flag of its own, so it is there again.
    pub expanded_settings_open: bool,
    /// The achievements: what is unlocked, the popup, and the counters the triggers use.
    pub achievements: crate::achievements::Achievements,
    /// Pointer interaction state (UI-first input routing).
    #[allow(dead_code)] // window-loop wiring lands next
    pub interaction: UiInteraction,
    /// Open panels/modals/view stack for the functional UI.
    pub panels: crate::menu::PanelState,
    /// Active world-editing tool (`Placer`/tower/killer/inspect).
    pub tool: crate::ui::ToolKind,
    /// The placed tower or colour killer the player selected (`UserControl.target` when it is
    /// one), by save id.
    pub selected_tool: Option<u64>,
    /// The zone the Z key selected (`UserControl.target` when it is one), by index in the world's
    /// zone list; the Zones tab shows its item open. Indices shift when zones are deleted.
    pub selected_zone: Option<usize>,
    /// `UserSettings.OpenZoneEditorOnSelect` (default on): selecting a zone opens the Zones tab.
    pub open_zone_editor_on_select: bool,
    /// The zone whose name line is being typed into and the text so far (`TextLineHandle`).
    pub zone_name_edit: Option<(usize, String)>,
    /// The settings animator editor's state: the creator panel (`SettingsChangerCreator`), the picked and dragged points, the graphs' frames.
    pub animator: crate::animator_session::Session,
    /// The edit field a slider row opened (`SettingSlider.OpenEditPanel`); the keyboard hotkeys are
    /// blocked meanwhile (`SetKeyboardBlockFromSource("SliderEdit")`).
    pub value_edit: Option<crate::value_edit::ValueEdit>,
    /// The caret of the tag / zone-name text field being typed into (which of them: `edit_caret_owner`).
    pub edit_caret: crate::text_field::Caret,
    pub(crate) edit_caret_owner: u8,
    /// A typed value is being applied: it joins no history (`SubmitEditValue` is not revertable
    /// unless the slider is a `LogSettingSlider`).
    suppress_history: bool,
    /// The scene's auto-panner (`Utility.AutoPanner`), which drifts between zone positions.
    pub autopan: crate::autopanner::AutoPanner,
    /// Random numbers of the auto-panner (its own stream, so the UI picks stay as they were).
    autopan_rng: u64,
    /// Whether the auto-panner is the target (`UserControl.SelectTarget(autoPanner)`, the K key).
    pub selected_autopan: bool,
    /// Whether the Zones tab's Preview panel is shown (`TogglePreview`; hidden at first).
    pub zone_preview_open: bool,
    /// Whether the selected tool's panel is open: Escape closes the panel first, a second Escape
    /// deselects (a colour killer deselects at once, `ColorKillerPanel.Escape`).
    pub tool_panel_open: bool,
    /// A tower panel slider being dragged: field and track x range (reference px).
    pub tool_drag: Option<(u8, u16, u16)>,
    /// A pellet placer slider being dragged: field and track x range (reference px).
    pub pellet_drag: Option<(u8, u16, u16)>,
    /// The hover timer of the tooltips (`TooltipTrigger`, `PelletTooltipTrigger`).
    pub tooltip: crate::tooltip::State,
    /// Cursor of the latest frame (logical pixels): the placement ghost follows it.
    pub cursor: Vec2,
    /// The bottom-left toolbar panels (0 placer, 1 user actions, 2 view).
    pub toolbar: [HoverPanel; 3],
    /// "Not enough data collected" dialog of the species (lineage) panel.
    pub lineage_gate_popup: bool,
    /// Local save/scenario storage roots (fmt-dev `bibites_io::storage`).
    pub storage: StoragePaths,
    /// Last UI command status (shown in tests/logs).
    pub last_action: String,
    /// Tool property values used by viewport placement.
    pub tool_settings: ToolSettings,
    /// Pending confirmation dialog (`KILL`/`KillHalf`).
    pub confirm_dialog: Option<ConfirmKind>,
    /// The error popups waiting in the world (`PopupManager.DisplayError`: source, text), the first one showing.
    pub error_popups: Vec<(String, String)>,
    /// The Biology panel's bar pop-ups with their 0.3 s open / close delays (`OnHoverShow`).
    pub bar_hover: crate::biology_popups::BarHover,
    /// The "do not show again" checkbox of the pending action warning is ticked
    /// (`WarningHandle.doNotShowAgain.isOn`; every popup starts unticked).
    pub confirm_do_not_show: bool,
    /// The action warnings switched off in this run when there is no user settings file to keep
    /// them in (`user_settings_path`).
    warnings_off: std::collections::BTreeSet<crate::warnings::ActionWarning>,
    /// Selected bibite's editable species tag (TAG control).
    pub tag_draft: String,
    /// Selected egg (TAG/SAVE inspector target).
    pub selected_egg: Option<bibites_core::ids::EggId>,
    /// Egg tag draft.
    pub egg_tag_draft: String,
    /// Cached load/save browser rows.
    pub save_entries: Vec<bibites_io::storage::SaveListEntry>,
    /// The open Save Game panel (`SaveGamePanel`), while `PanelKind::SaveGame` is on the stack.
    pub save_panel: Option<crate::save_panel::SavePanel>,
    /// The primary button is held (the buttons of the panels darken while pressed).
    pub pointer_down: bool,
    /// Selected dynamic save row.
    pub save_selected: usize,
    /// Currently open dropdown (captured runtime UI).
    pub dropdown: Option<DropdownState>,
    /// The open dropdown list of a dropdown control (`TMP_Dropdown.Show`).
    pub dropdown_open: Option<crate::dropdown::Open>,
    /// The control the last click landed on (reference px: `x, y, w, h`), where a dropdown opened by
    /// that click hangs its list.
    pub click_rect: [f32; 4],
    /// Scenario editor working copy of settings.
    pub editor_settings: Option<SimSettings>,
    /// Scenario editor dirty flag.
    pub editor_dirty: bool,
    /// Source scenario path (enables Save in the editor).
    pub source_scenario: Option<PathBuf>,
    /// Genome editor working copy.
    pub editor_genome: Option<Genome>,
    /// Selected genome row index.
    pub editor_gene: usize,
    /// Genome editor page (14 rows per page).
    pub editor_gene_page: usize,
    /// Brain editor working copy.
    pub editor_brain: Option<Brain>,
    /// Selected brain node index.
    pub editor_node: usize,
    /// Node selected before `editor_node` (source of the next synapse).
    pub editor_prev_node: usize,
    /// Brain editor validation message.
    pub editor_validation: String,
    /// Brain editor node anchors keyed by innovation.
    pub editor_anchors: std::collections::BTreeMap<i64, (f32, f32)>,
    /// Running local challenge session.
    pub challenge: Option<bibites_core::world::ChallengeSession>,
    /// What the running challenge was made from (kept for "Retry").
    pub challenge_setup: Option<crate::challenge_run::ChallengeSetup>,
    /// The pause state before the challenge's end paused the game (`TogglePauseGame("ChallengeEnd")`).
    pub challenge_end_pause: Option<bool>,
    /// Challenge parameters embedded in the loaded scenario (fallback: defaults).
    pub challenge_params: Option<bibites_core::settings::ChallengeParameters>,
    /// Draft parameters edited in the challenge parameters panel.
    pub challenge_draft: Option<bibites_core::settings::ChallengeParameters>,
    /// Selected timer preset index.
    pub challenge_timer_index: usize,
    /// Selected star threshold preset indices (1/2/3 stars).
    pub challenge_star_indices: [usize; 3],
    /// Local Workshop entries `(name, path, kind)`.
    pub workshop: Vec<(String, PathBuf, String)>,
    /// Challenge scenario candidates `(name, path)`.
    pub challenge_scenarios: Vec<(String, PathBuf)>,
    /// Champion template candidates `(name, path)`.
    pub champion_templates: Vec<(String, PathBuf)>,
    /// Selected challenge scenario/champion indices.
    pub challenge_scenario_selected: usize,
    pub champion_selected: usize,
    /// Selected Workshop row.
    pub workshop_selected: usize,
    /// Generic list scroll offset for native ScrollRect controls.
    pub list_scroll: i32,
    /// Local Workshop rename draft.
    pub workshop_name_draft: String,
    /// Hotkey action awaiting the next key press.
    pub hotkey_listening: Option<usize>,
    /// High score of the last finished challenge.
    pub challenge_high: Option<bibites_core::world::ChallengeScoreInfo>,
    /// Outcome of recording the finished challenge against the stored best
    /// (`ChallengesProgress.TrySetHighScoreOfChallenge`).
    pub challenge_record: Option<bibites_io::storage::ChallengeRecordOutcome>,
    /// Whether the end popup is shown.
    pub challenge_end_shown: bool,
    /// Viewer/gameplay preferences bound to the Settings pages.
    pub prefs: bibites_io::storage::UserPreferences,
    /// Active Settings page (0 graphics, 1 gameplay, 2 hotkeys).
    pub settings_page: u8,
    /// Cached lineage tree while the graph panel is open.
    pub lineage: Option<bibites_core::species::LineageTree>,
    /// Selected lineage node index.
    pub lineage_selected: Option<usize>,
    /// The lineage tree's window: the laid out tree, the zoom, the position (`LineageWindow`).
    pub lineage_view: crate::lineage_view::View,
    /// The right button was down last frame (a drag of the tree starts on the press).
    pub lineage_right_was_down: bool,
    /// The frame's pointer state as the window read it, before the loop took the wheel and the buttons away over the UI: the Lineage panel is
    /// a full-screen panel (its backdrop is a widget everywhere), so without this its wheel zoom / scroll and its scroll bars never saw a thing.
    /// Set by `route_ui_frame` while the panel is open, used (once) by `lineage_input`.
    pub lineage_raw: Option<FrameInput>,
    /// The Lineage panel's Settings panel is open (`LineagePanelSettingsManager`).
    pub lineage_settings_open: bool,
    /// How far its column is scrolled up.
    pub lineage_settings_scroll: f32,
    /// Its scroll bar's handle is held: where in the handle it was grabbed (0..1).
    pub lineage_settings_grab: Option<f32>,
    /// The tree's options pinned by a headless run or a test (else they are the user settings).
    pub lineage_options: Option<crate::lineage_tree::TreeSettings>,
    /// Species list for the lineage species panel.
    pub lineage_species: Vec<bibites_core::species::SpeciesListEntry>,
    /// Selected species list index.
    pub species_selected: usize,
    /// Species panel tab (0 info, 1 genes, 2 brain, 3 position: `SpeciesTabManager`).
    pub lineage_tab: u8,
    /// The species whose panels show in the lineage panel (`SpeciesTabManager.targetSpecies`).
    pub species_info: Option<bibites_core::ids::SpeciesId>,
    /// The species panel's text fields and Prune button.
    pub species_ui: crate::species_panel::State,
    /// The species the Genes (and Brain) tab compares the shown one with: its parent, or itself
    /// (`SpeciesTabManager.SelectToCompareSpecies`).
    pub species_compared: Option<bibites_core::ids::SpeciesId>,
    /// The Position tab's time slider value (`SpeciesDistributionPanel.timeSlider`), kept from species to species.
    pub position_value: i32,
    /// What the Position tab was filled with when the species was selected or the slider moved.
    pub position: Option<crate::species_panel::position::State>,
    /// The Position tab's slider is held.
    pub position_drag: Option<crate::species_panel::position::Drag>,
    /// The Brain tab's layout of the shown species' brain (`SpeciesBrainPanel.FillPanel`).
    pub brain: Option<crate::species_panel::brain::State>,
    /// The width the Brain tab's holder has (`holderRT.rect.width`): it only ever grows.
    pub brain_holder_width: f32,
    /// The positions the Save pos. button kept (`template.nodeAnchors`), by species.
    pub brain_anchors: crate::species_panel::brain::Anchors,
    /// A neuron of the Brain tab is held.
    pub brain_drag: Option<crate::species_panel::brain::Drag>,
    /// `UserSettings.showBrainDifferences` (the "Highlight differences" toggle; off by default).
    pub show_brain_differences: bool,
    /// `UserSettings.SortBiologyPanelEnergyDetails`: the Flow details pop-up lists its rows by value (off by default).
    pub sort_energy_details: bool,
    /// RUSTibites Improved: the Species Distribution window writes each zone's name over its map (the "Zone names" check box; on, as the game has it).
    pub map_zone_names: bool,
    /// RUSTibites Improved: and each zone's biomass and pellet count (the "Zone biomass" check box; on).
    pub map_zone_biomass: bool,
    /// How many times a diagram was sorted: every sort draws other random numbers.
    pub brain_sorts: u64,
    /// The inspector's Brain tab's layout of the target's brain (`BrainPanel.FillPanel`).
    pub inspector_brain: Option<crate::inspector_brain::State>,
    /// The width that tab's holder has: it only ever grows.
    pub inspector_holder_width: f32,
    /// A neuron of that tab is held.
    pub inspector_brain_drag: Option<crate::inspector_brain::Drag>,
    /// Graphs panel: index into `DataLogger::groups`.
    pub graphs_stream: usize,
    /// Graphs panel: history scale (0 = finest).
    pub graphs_scale: usize,
    /// Graphs panel: which curves of each group show (the `state` of the group's
    /// `DataStreamDescription`s, all on to begin with), indexed like `DataLogger::groups`.
    pub graphs_curves: Vec<Vec<bool>>,
    /// Graphs panel: the legend shows while the pointer rests on its "?" button or on the legend
    /// (`OnHoverShow`, `OnHoverKeep`).
    pub graphs_legend_open: bool,
    /// RUSTibites Improved: the time tags of the world (`crate::time_tags`), the same in every panel that has a time axis.
    pub time_tags: crate::time_tags::TimeTags,
    /// RUSTibites Improved: in which panels the time tags show (each panel's own button; kept in `time_tag_view.json`).
    pub time_tag_view: bibites_io::storage::TimeTagViewFile,
    /// RUSTibites Improved: the name field of a time tag while the player types it.
    pub time_tag_edit: Option<TimeTagEdit>,
    /// RUSTibites Improved: the save name the world's time tags are stored under (`None`: the world has not been saved or loaded by a name yet).
    pub(crate) time_tags_key: Option<String>,
    /// RUSTibites Improved: the colour pop-up of a zone, while it is open (`crate::zone_color_ui`).
    pub zone_color_popup: Option<crate::zone_color_ui::ZoneColorPopup>,
    /// The Bibite Editor, while it is open.
    pub bibite_editor: Option<crate::template_editor_ui::EditorSession>,
    /// The editor closed and the Library under it must fill its lists again (`AfterEditorClose`: `Some(None)`; `AfterEditorSubmit`:
    /// `Some(Some(name))`, the template to select). The window loop hands it to the Library and clears it.
    pub library_refill: Option<Option<String>>,
    /// Graphs panel: the Gene Analysis page shows instead of the data graphs (`GraphsPanel.activeGraph == 1`).
    pub graphs_genes: bool,
    /// Gene Analysis page: the gene whose distribution shows (`BarGraph.dataSelected`; the dropdown starts on the 17th, Diet).
    pub graphs_gene: usize,
    /// Gene Analysis page: the time slider's value (`-1` is now, the others count back through the genes stream).
    pub graphs_gene_time: i32,
    /// Gene Analysis page: the slider is held.
    pub graphs_gene_drag: Option<crate::species_panel::position::Drag>,
    /// `UserSettings.ShowRuler` (default on).
    pub show_ruler: bool,
    /// `UserSettings.ShowStatusBars` (default off): the health / energy bars of a selected bibite.
    pub show_status_bars: bool,
    /// `UserSettings.AllowEasterEggs` (default off) and `EasterEggsProbability` (default 500 000): the world's children may be born
    /// radioactive, rainbow, producer or reaper bibites. They are the player's, so they are handed to whichever world is running.
    pub allow_easter_eggs: bool,
    /// See [`App::allow_easter_eggs`].
    pub easter_eggs_period: u32,
    /// `StatusBarsReference.targetScale`: the scale of the target when it was selected (`transform.localScale.x`: a bibite's
    /// `d1Size`, an egg's size gene), which the bars' size follows.
    pub status_bar_scale: f32,
    /// `UserSettings.ShowBibiteTargets` (default off): the target weights of a selected bibite.
    pub show_targets: bool,
    /// Current ruler length in world units (`RulerHandle.targetSize`).
    pub ruler_target: f32,
    /// Ruler step size (`RulerHandle.scale`).
    ruler_scale: f32,
    /// First visible gene row in the inspector's Genes tab.
    pub genes_scroll: usize,
    /// Pause state to restore when the Escape panel closes
    /// (`EscapePanel.SetRestrictions`).
    escape_pause: Option<bool>,
    /// Last pellet-placer position (hold-to-paint spacing).
    pellet_last_placement: Option<Vec2>,
    /// xorshift state for the random-selection hotkeys (UI only; never touches
    /// the simulation RNG).
    ui_rng: u64,
    /// Pause state to restore when the Graphs / Lineage panel closes
    /// (`TogglePauseGame("GraphsPanel" | "SpeciesPanel")`).
    panel_pause: Option<bool>,
    /// Seconds accumulated towards the next autosave.
    autosave_progress: f32,
    /// The write of the autosave in progress (it runs on its own thread): sends the message to show when it is done.
    autosave_result: Option<std::sync::mpsc::Receiver<String>>,
    /// Previous cursor position while the right button pans the camera.
    pan_last: Option<Vec2>,
    /// `CameraManager.gradualInput`: smoothed arrow/WASD input.
    gradual_input: Vec2,
    /// `CameraManager.speedScale` (PageUp x1.25 / PageDown /1.25).
    camera_speed_scale: f32,
    /// Cursor height where the middle-drag zoom began.
    scroll_drag_start_y: f32,
    /// Whether autosaves are listed in the browser.
    pub include_autosaves: bool,
    /// Optional extracted game assets (game-faithful sprite mode).
    pub assets: Option<GameAssets>,
    /// The procedural bibite pictures made so far (the species panels, `preview.rs`).
    pub previews: crate::preview::PreviewCache,
    /// `UserSettings.ProceduralSize`: the size the previews pick their sprites by.
    pub procedural_size: crate::preview::ProceduralSize,
    /// Per-bibite procedural tail state (render-only; stepped once per tick).
    pub tail_states: HashMap<BibiteId, TailState>,
    /// Smoothed-zoom target (pixels per world unit); `None` when idle.
    pub zoom_target: Option<f32>,
    /// Screen anchor for wheel zoom (kept fixed while smoothing).
    pub zoom_anchor: Vec2,
    /// Set by Escape; the window loop exits when true.
    pub quit: bool,
    /// Set by the Escape panel's "Back to Menu"; the window loop returns to the title menu.
    pub return_to_menu: bool,
    /// Dynamic settings panel state (tab, scroll, open sub-panels, slider drag).
    pub dyn_panel: crate::dyn_panel::DynState,
    /// Time-speed slider being dragged: track x range (reference px).
    pub speed_drag: Option<(u16, u16)>,
    /// How far from the handle's middle (reference px) the press that holds the time-speed slider or a settings row's slider took it: the
    /// handle stays where the pointer found it (`Slider.OnPointerDown` keeps the offset of a press on the handle), 0 for a press on the track,
    /// which moves the handle under the pointer. Set by every such press ([`App::speed_slider_grab`], [`App::dyn_slider_grab`]).
    pub slider_grab: f32,
    /// Where the pointer was (screen px) when a press took a slider by its handle: the slider changes nothing until the pointer leaves that
    /// place (`Slider.OnPointerDown` only notes the offset), so a press on the handle that does not move leaves the value as it was.
    slider_press_x: Option<f32>,
    /// The player's TPS defaults (default pointers of the TPS sliders).
    pub tps_defaults: [i32; 5],
    pub(crate) accumulator: f64,
    hud_timer: f32,
    /// The left-button press on the world, while it is held.
    pub drag: Option<DragState>,
    /// The entities picked by a drag rectangle; the selection panel is open while this is `Some`
    /// (`UserControl.multiSelection`).
    pub multi: Option<crate::selection::MultiSelection>,
    /// `UserSettings.SelectionRadius`: click radius as a fraction of the half screen height.
    pub selection_radius: f32,
    /// `CameraManager.LateUpdate`: the camera tracks the selected target until
    /// the user pans away.
    follow_selected: bool,
}

/// Straight-behind initial tail state for every live bibite at load time.
fn initial_tail_states(world: &World) -> HashMap<BibiteId, TailState> {
    world
        .bibites
        .iter()
        .map(|(id, record)| {
            let up = Vec2::Y.rotate(record.rot);
            let d1 = record.body.geometry.d1_size;
            (id, TailState::new(record.pos, up, d1))
        })
        .collect()
}

/// World-space bounding box of living bibites plus the longest body length.
///
/// Returns `None` when the world has no living (non-corpse) bibite, in which
/// case callers fall back to a whole-world fit.
fn bibite_bounds(world: &World) -> Option<(Vec2, Vec2, f32)> {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut longest_body = 0.0_f32;
    let mut found = false;
    for (_, record) in world.bibites.iter() {
        if record.is_corpse || !record.pos.is_finite() {
            continue;
        }
        min = min.min(record.pos);
        max = max.max(record.pos);
        longest_body = longest_body.max(record.body.geometry.body_length);
        found = true;
    }
    found.then_some((min, max, longest_body))
}

/// The default camera: frame the bibite population, never closer than
/// [`MIN_VIEW_EXTENT_WORLD`] world units and never further than
/// [`MAX_DEFAULT_VIEW_EXTENT_WORLD`]. When the population is spread wider than
/// the maximum, the view anchors on the bibite nearest the population centre so
/// the frame always contains at least one body.
fn bibite_view_camera(world: &World, screen: Vec2) -> Camera {
    let Some((min_corner, max_corner, longest_body)) = bibite_bounds(world) else {
        return Camera::fit(
            world.bounds.min(),
            world.bounds.max(),
            screen,
            VIEW_MARGIN_PX,
        );
    };
    let bbox_center = (min_corner + max_corner) * 0.5;
    let bbox_extent = max_corner - min_corner;
    let (center, extent) = if bbox_extent.x.max(bbox_extent.y) > MAX_DEFAULT_VIEW_EXTENT_WORLD {
        (
            densest_bibite(world),
            Vec2::splat(MAX_DEFAULT_VIEW_EXTENT_WORLD),
        )
    } else {
        let minimum = MIN_VIEW_EXTENT_WORLD.max(longest_body * VIEW_BODY_LENGTH_FACTOR);
        (bbox_center, bbox_extent.max(Vec2::splat(minimum)))
    };
    let half = extent * 0.5;
    Camera::fit(center - half, center + half, screen, VIEW_MARGIN_PX)
}

/// Position of the living bibite with the most neighbours within 300 world
/// units (a deterministic "densest cluster" anchor used when the population is
/// spread wider than the default view cap). Ties break on store order.
fn densest_bibite(world: &World) -> Vec2 {
    let positions: Vec<Vec2> = world
        .bibites
        .iter()
        .filter(|(_, record)| !record.is_corpse && record.pos.is_finite())
        .map(|(_, record)| record.pos)
        .collect();
    if positions.is_empty() {
        return Vec2::ZERO;
    }
    let mut best = positions[0];
    let mut best_count = 0_usize;
    for &candidate in &positions {
        let count = positions
            .iter()
            .filter(|&&other| candidate.distance_sq(other) < 300.0 * 300.0)
            .count();
        if count > best_count {
            best_count = count;
            best = candidate;
        }
    }
    best
}

impl App {
    /// Creates the app for `loaded` (logical-pixel `viewport`).
    ///
    /// The default camera frames the bibite population (see
    /// [`App::fit_bibites`]) rather than the whole world: real scenarios can be
    /// tens of thousands of units wide, where a full-world fit makes bibites
    /// sub-pixel dots.
    #[must_use]
    pub fn new(loaded: LoadedWorld, viewport: Vec2, assets: Option<GameAssets>) -> Self {
        let viewport = Vec2::new(viewport.x.max(1.0), viewport.y.max(1.0));
        let seed = loaded.world.seed;
        let camera = bibite_view_camera(&loaded.world, viewport);
        let hud = HudInfo::collect(&loaded.world);
        let tail_states = initial_tail_states(&loaded.world);
        let graphs_curves = loaded
            .world
            .data_logger
            .groups
            .iter()
            .map(|group| vec![true; group.streams.len()])
            .collect();
        // The real game starts with its grid disabled (`UserSettings.ShowGrid`
        // default = false); keep the viewer's grid off in game-asset mode so
        // reference comparisons are 1:1.
        let scene_options = SceneOptions {
            grid: false,
            ..SceneOptions::default()
        };
        Self {
            world: loaded.world,
            camera,
            scene: Scene::new(),
            label: loaded.label,
            seed,
            paused: false,
            speed: 1.0,
            engine_fraction: 1.0,
            last_tick_secs: 0.0,
            tick_cost: 0.0,
            #[cfg(test)]
            model_tick_secs: None,
            overlap_ticks: false,
            tick_worker: None,
            ticks_in_flight: false,
            world_husk: None,
            ticks_deferred: false,
            frame_started: None,
            joined_ticks: 0,
            overlap_dispatched: None,
            overlap_hidden: 0.0,
            overlap_wait_secs: 0.0,
            overlap_batches: 0,
            govern_since: 0.0,
            #[cfg(test)]
            model_beside_secs: None,
            sim: None,
            improved: crate::improved_switches::configured(),
            world_away: false,
            light_streak: 0,
            snapshot: None,
            sim_ticks_seen: 0,
            autopan_owed: 0,
            world_wanted: false,
            sim_ticks: 0,
            selected: None,
            hud,
            selection: None,
            fps: 0,
            time_keeper: crate::time_keeper::TimeKeeper::new(0.0),
            wall_time: 0.0,
            view_mode: ViewMode::Bibites,
            scene_options,
            show_hud: true,
            inspector_tab: InspectorTab::Stats,
            inspector_open: false,
            expanded_brain: None,
            expanded_graph: crate::expanded_brain::GraphType::Linear,
            expanded_pan: [0.0, 0.0],
            expanded_settings_open: false,
            achievements: crate::achievements::Achievements::default(),
            undo: UndoStack::new(),
            undo_drag: None,
            tag_edit: None,
            typed: String::new(),
            clipboard_in: None,
            clipboard_out: None,
            pending_url: None,
            template_pick: None,
            placer: None,
            placer_options: bibites_core::settings::BibiteSpawn::game_defaults(),
            placer_pause: None,
            tag_edit_is_placer: false,
            save_dialog: None,
            scenario_saver: None,
            save_as_template: false,
            format_bibite_json: false,
            user_settings_path: None,
            template_names: std::cell::OnceCell::new(),
            reselect: None,
            reselect_tag: None,
            ctrl_held: false,
            shift_held: false,
            information: crate::information_panel::InfoState::default(),
            reselect_timer: None,
            corpse_listener: false,
            reselect_on_corpses: true,
            reselection_delay: 5.0,
            allow_arrow_keys: true,
            interaction: UiInteraction::default(),
            panels: crate::menu::PanelState::default(),
            tool: crate::ui::ToolKind::Inspect,
            selected_tool: None,
            selected_zone: None,
            open_zone_editor_on_select: true,
            zone_name_edit: None,
            animator: crate::animator_session::Session::default(),
            value_edit: None,
            edit_caret: crate::text_field::Caret::default(),
            edit_caret_owner: 0,
            suppress_history: false,
            autopan: crate::autopanner::AutoPanner::new(crate::autopanner::MIN_WAIT),
            autopan_rng: 0x9E37_79B9_7F4A_7C15,
            selected_autopan: false,
            zone_preview_open: false,
            tool_panel_open: false,
            tool_drag: None,
            pellet_drag: None,
            tooltip: crate::tooltip::State::default(),
            cursor: Vec2::ZERO,
            toolbar: [HoverPanel::default(); 3],
            lineage_gate_popup: false,
            storage: StoragePaths::default_local(),
            last_action: String::new(),
            tool_settings: ToolSettings::default(),
            confirm_dialog: None,
            error_popups: Vec::new(),
            bar_hover: crate::biology_popups::BarHover::default(),
            confirm_do_not_show: false,
            warnings_off: std::collections::BTreeSet::new(),
            tag_draft: String::new(),
            selected_egg: None,
            egg_tag_draft: String::new(),
            save_entries: Vec::new(),
            save_panel: None,
            pointer_down: false,
            save_selected: 0,
            dropdown: None,
            dropdown_open: None,
            click_rect: [0.0; 4],
            editor_settings: None,
            editor_dirty: false,
            source_scenario: None,
            editor_genome: None,
            editor_gene: 0,
            editor_gene_page: 0,
            editor_brain: None,
            editor_node: 0,
            editor_prev_node: 0,
            editor_validation: String::new(),
            editor_anchors: std::collections::BTreeMap::new(),
            challenge: None,
            challenge_setup: None,
            challenge_end_pause: None,
            challenge_params: None,
            challenge_draft: None,
            challenge_timer_index: 4,
            challenge_star_indices: [2, 2, 2],
            workshop: Vec::new(),
            challenge_scenarios: Vec::new(),
            champion_templates: Vec::new(),
            challenge_scenario_selected: 0,
            champion_selected: 0,
            workshop_selected: 0,
            list_scroll: 0,
            workshop_name_draft: String::new(),
            hotkey_listening: None,
            challenge_high: None,
            challenge_record: None,
            challenge_end_shown: false,
            prefs: bibites_io::storage::load_user_preferences(&StoragePaths::default_local())
                .unwrap_or_default(),
            settings_page: 0,
            lineage: None,
            lineage_selected: None,
            lineage_view: crate::lineage_view::View::default(),
            lineage_right_was_down: false,
            lineage_raw: None,
            lineage_settings_open: false,
            lineage_settings_scroll: 0.0,
            lineage_settings_grab: None,
            lineage_options: None,
            lineage_species: Vec::new(),
            species_selected: 0,
            lineage_tab: 0,
            species_info: None,
            species_ui: crate::species_panel::State::default(),
            species_compared: None,
            position_value: 0,
            position: None,
            position_drag: None,
            brain: None,
            brain_holder_width: crate::brain_layout::START_HOLDER_WIDTH,
            brain_anchors: crate::species_panel::brain::Anchors::new(),
            brain_drag: None,
            show_brain_differences: false,
            map_zone_names: true,
            map_zone_biomass: true,
            sort_energy_details: false,
            brain_sorts: 0,
            inspector_brain: None,
            inspector_holder_width: crate::inspector_brain::START_HOLDER_WIDTH,
            inspector_brain_drag: None,
            graphs_stream: 0,
            graphs_curves,
            graphs_legend_open: false,
            time_tags: crate::time_tags::TimeTags::new(),
            time_tag_view: bibites_io::storage::TimeTagViewFile::default(),
            time_tag_edit: None,
            time_tags_key: None,
            zone_color_popup: None,
            bibite_editor: None,
            library_refill: None,
            graphs_genes: false,
            graphs_gene: crate::gene_analysis::DEFAULT_GENE,
            graphs_gene_time: -1,
            graphs_gene_drag: None,
            graphs_scale: 0,
            escape_pause: None,
            genes_scroll: 0,
            show_ruler: true,
            show_status_bars: false,
            status_bar_scale: 1.0,
            allow_easter_eggs: false,
            easter_eggs_period: 500_000,
            show_targets: false,
            ruler_target: 100.0,
            ruler_scale: 100.0,
            pellet_last_placement: None,
            ui_rng: 0x9E37_79B9_7F4A_7C15,
            pan_last: None,
            gradual_input: Vec2::ZERO,
            camera_speed_scale: 1.0,
            scroll_drag_start_y: 0.0,
            autosave_progress: 0.0,
            autosave_result: None,
            panel_pause: None,
            include_autosaves: false,
            assets,
            previews: crate::preview::PreviewCache::default(),
            procedural_size: crate::preview::ProceduralSize::Both,
            tail_states,
            zoom_target: None,
            zoom_anchor: Vec2::ZERO,
            quit: false,
            return_to_menu: false,
            dyn_panel: crate::dyn_panel::DynState::default(),
            speed_drag: None,
            slider_grab: 0.0,
            slider_press_x: None,
            tps_defaults: [40, 2, 2, 10, 5],
            accumulator: 0.0,
            hud_timer: 0.0,
            drag: None,
            multi: None,
            selection_radius: crate::selection::DEFAULT_SELECTION_RADIUS,
            follow_selected: false,
        }
    }

    /// Updates the viewport size (window resize / DPI change).
    ///
    /// When the camera is in an automatic fit mode (the default) it is re-fitted
    /// for the new aspect ratio; a manual centre/zoom is preserved.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        let screen = Vec2::new(width.max(1.0), height.max(1.0));
        if (self.camera.screen - screen).length_sq() < 1e-6 {
            return;
        }
        if self.view_mode != ViewMode::Manual {
            // Fitting the view to the bibites or the world reads the world.
            self.ensure_world_home();
        }
        // The game's camera is orthographic: `orthographicSize` (half the window's height, in world units) stays when the window changes size, so a
        // taller window (fullscreen, a resize) shows the same part of the world bigger. The zoom here is pixels per unit: it follows the height.
        let previous_height = self.camera.screen.y;
        self.camera.set_screen(screen);
        match self.view_mode {
            ViewMode::Bibites => self.fit_bibites(),
            ViewMode::World => self.fit_world(),
            ViewMode::Manual => {
                if previous_height > 0.0 {
                    let ratio = screen.y / previous_height;
                    self.camera.zoom = (self.camera.zoom * ratio).clamp(Camera::MIN_ZOOM, Camera::MAX_ZOOM);
                    self.zoom_target = self.zoom_target.map(|target| (target * ratio).clamp(Camera::MIN_ZOOM, Camera::MAX_ZOOM));
                }
            }
        }
    }

    /// Fits the whole world into the current viewport (`F`).
    pub fn fit_world(&mut self) {
        self.camera = Camera::fit(
            self.world.bounds.min(),
            self.world.bounds.max(),
            self.camera.screen,
            VIEW_MARGIN_PX,
        );
        self.view_mode = ViewMode::World;
    }

    /// Frames the living bibites with a readable margin (`B`, `R`, and the
    /// default view). Falls back to a world fit when the population is empty.
    pub fn fit_bibites(&mut self) {
        self.camera = bibite_view_camera(&self.world, self.camera.screen);
        self.view_mode = ViewMode::Bibites;
    }

    /// Sets the camera to an exact Unity orthographic size (--ortho-size).
    ///
    /// Unity's Camera.orthographicSize is half the visible world height, so
    /// zoom = viewport_height / (2 * ortho_size) maps world units to pixels
    /// exactly like the game camera at that size (used for reference-frame
    /// comparison screenshots).
    pub fn set_ortho_size(&mut self, ortho_size: f32, viewport_height: f32) {
        self.zoom_target = None;
        if !(ortho_size.is_finite() && ortho_size > 0.0) {
            return;
        }
        let screen_height = viewport_height.max(1.0);
        self.camera.zoom =
            (screen_height / (2.0 * ortho_size)).clamp(Camera::MIN_ZOOM, Camera::MAX_ZOOM);
        self.view_mode = ViewMode::Manual;
    }

    /// Overrides the camera centre and/or zoom (CLI `--center`/`--zoom`).
    ///
    /// Both are optional so a demo can set only the zoom (keeping the default
    /// bibite centre) or only the centre. Either option switches the camera to
    /// [`ViewMode::Manual`] so window resizes do not throw the framing away.
    pub fn set_view(&mut self, center: Option<Vec2>, zoom: Option<f32>) {
        if let Some(center) = center {
            self.camera.center = center;
        }
        if let Some(zoom) = zoom {
            self.camera.zoom = zoom.clamp(Camera::MIN_ZOOM, Camera::MAX_ZOOM);
        }
        if center.is_some() || zoom.is_some() {
            self.view_mode = ViewMode::Manual;
        }
    }

    /// Handles one frame of input. Never advances the simulation (see
    /// [`App::advance`]), so it is deterministic and testable.
    pub fn handle_input(&mut self, input: &FrameInput) {
        // The world must be home (the window joins the helper's ticks right after the frame is presented).
        self.join_overlapped_ticks();
        self.cursor = input.cursor;
        self.pointer_down = input.primary_down;
        self.lineage_input(input);
        self.graphs_legend_open = self.panels.is_open(crate::ui::PanelKind::Graphs)
            && !self.graphs_genes
            && crate::hud::graphs_legend_hovered(
                self,
                input.cursor,
                self.graphs_legend_open,
                self.camera.screen.x,
            );
        self.sync_escape_stack();
        self.sync_save_panel();
        if input.quit {
            self.escape_pressed();
        }
        if self.dyn_panel.drag.is_none()
            && self.dyn_panel.material_drag.is_none()
            && self.dyn_panel.zone_drag.is_none()
        {
            self.flush_undo_drag();
        }
        if (input.undo || input.redo) && self.value_edit.is_none() {
            self.undo_or_redo(input.redo);
        }
        // Space is a keyboard control: ignored while the pause menu / a dialog is
        // open or a hotkey is being rebound (`EscapePanel` blocks the keyboard).
        if input.toggle_pause
            && self.hotkey_listening.is_none()
            && self.confirm_dialog.is_none()
            && self.value_edit.is_none()
            // RUSTibites Improved: a space typed in a time tag's name is a letter, not the pause key (and the colour pop-up is modal).
            && self.time_tag_edit.is_none()
            && self.zone_color_popup.is_none()
            && self.bibite_editor.is_none()
            && self.expanded_brain.is_none()
            && self.scenario_saver.is_none()
            // `UserControl.SetKeyboardBlockFromSource("Challenge", true)` when the challenge ends.
            && !self.challenge_end_shown
            && !self.panels.is_open(crate::ui::PanelKind::Escape)
            // `SaveGamePanel.OpenPanel`: `AllowControl = false`.
            && !self.panels.is_open(crate::ui::PanelKind::SaveGame)
        {
            self.paused = !self.paused;
            self.accumulator = 0.0;
        }
        // `UserControl.Update` / `CameraManager.Update` hotkeys. They are ignored while
        // a key is being rebound or a dialog / the pause menu is open, and Insert,
        // Delete and F5 are disabled during challenges (`UserControl.cs:114-180`).
        let hotkeys_live = self.hotkey_listening.is_none()
            && self.confirm_dialog.is_none()
            && self.dropdown_open.is_none()
            && self.save_dialog.is_none()
            && self.scenario_saver.is_none()
            && self.tag_edit.is_none()
            && self.zone_name_edit.is_none()
            && self.value_edit.is_none()
            && self.time_tag_edit.is_none()
            && self.zone_color_popup.is_none()
            && !self.species_ui.typing()
            && self.bibite_editor.is_none()
            && self.expanded_brain.is_none()
            && !self.challenge_end_shown
            && !self.panels.is_open(crate::ui::PanelKind::Escape)
            && !self.panels.is_open(crate::ui::PanelKind::SaveGame);
        // `UserControl.Update` returns early without `AllowSelection` (the placer turns it off):
        // the random selections, Insert, Delete and F5 stop with it; the camera keys do not.
        let selection_live =
            hotkeys_live && self.placer.is_none() && self.tool != crate::ui::ToolKind::Pellet;
        let edit_keys_live = selection_live && self.challenge.is_none();
        let raw_input = *input;
        let input = &FrameInput {
            select_random_bibite: input.select_random_bibite && selection_live,
            select_high_generation: input.select_high_generation && selection_live,
            select_oldest: input.select_oldest && selection_live,
            select_random_egg: input.select_random_egg && selection_live,
            select_random_zone: input.select_random_zone && selection_live,
            select_autopanner: input.select_autopanner && selection_live,
            camera_home: input.camera_home && hotkeys_live,
            zoom_key: if hotkeys_live { input.zoom_key } else { 0.0 },
            move_axis: if hotkeys_live {
                input.move_axis
            } else {
                Vec2::ZERO
            },
            speed_scale_step: if hotkeys_live {
                input.speed_scale_step
            } else {
                0
            },
            digit: if hotkeys_live { input.digit } else { None },
            toggle_pause: input.toggle_pause && hotkeys_live,
            toggle_hud: input.toggle_hud && hotkeys_live,
            toggle_objects: input.toggle_objects && hotkeys_live,
            copy_tag: input.copy_tag && hotkeys_live,
            paste_tag: input.paste_tag && hotkeys_live,
            step: input.step && hotkeys_live,
            quick_save: input.quick_save && edit_keys_live,
            lay_egg: input.lay_egg && edit_keys_live,
            kill_target: input.kill_target && edit_keys_live,
            ..*input
        };
        // `UserControl.Update`: with Ctrl held the pick repeats whenever the target is gone.
        // The dialog's fields take the typed characters first (the tag field drops them when it is
        // not being edited).
        self.edit_save_dialog(&raw_input);
        self.edit_scenario_saver(&raw_input);
        self.edit_save_name(&raw_input);
        self.edit_editor_text(&raw_input);
        // The Expanded Brain View's force simulation runs with the real frame time.
        self.expanded_brain_frame(raw_input.dt);
        self.edit_zone_name(&raw_input);
        self.edit_value(&raw_input);
        self.edit_time_tag(&raw_input);
        self.edit_zone_color_hex(&raw_input);
        self.species_edit_typing(&raw_input);
        self.edit_species_tag(&raw_input);
        if self.tag_edit.is_none() && self.zone_name_edit.is_none() && !self.species_ui.typing() && self.bibite_editor.as_ref().is_none_or(|session| session.view.focus.is_none() && !session.view.brain.label_focus && !session.view.brain.filter_focus) {
            self.edit_caret_owner = 0;
        }
        crate::text_field::set_display_caret(if self.edit_caret_owner == 0 { crate::text_field::Caret { pos: usize::MAX, anchor: None } } else { self.edit_caret });
        if input.copy_tag && self.target_is_bibite_or_egg() {
            // `GUIManager.CopyTagOfTarget`.
            self.clipboard_out = Some(self.target_tag().unwrap_or_default());
        }
        if input.paste_tag {
            self.paste_tag();
        }
        if input.select_random_bibite {
            if input.ctrl_down {
                self.reselect = Some(Reselect::Random);
            }
            self.select_random_bibite();
        }
        if input.select_high_generation {
            if input.ctrl_down {
                self.reselect = Some(Reselect::HighestGeneration);
            }
            self.select_highest_generation();
        }
        if input.select_oldest {
            if input.ctrl_down {
                self.reselect = Some(Reselect::Oldest);
            }
            self.select_oldest();
        }
        if input.select_random_egg {
            self.select_random_egg();
        }
        if input.select_random_zone {
            self.select_random_zone();
        }
        if input.select_autopanner {
            self.select_autopanner();
        }
        if input.quick_save {
            self.save_world(true);
        }
        // `BrainPanelCommon.UpdatePanel`: the sort keys of the open Brain tab.
        if self.panels.is_open(crate::ui::PanelKind::Lineage)
            && self.species_info.is_some()
            && self.lineage_tab == 2
            && !self.species_ui.typing()
        {
            for (key, times) in input.brain_sort_keys.iter().zip([1, 6, 8]) {
                if *key {
                    self.brain_sort_n(times);
                }
            }
            if input.brain_hidden_down {
                self.brain_sort_hidden(input.dt, if input.right_ctrl_down { 0.0 } else { 1.0 });
            }
        }
        // The same keys sort the inspector's Brain tab while it is open (each `BrainPanelCommon.UpdatePanel` reads them).
        if self.inspector_visible()
            && self.inspector_tab == InspectorTab::Brain
            && self.tag_edit.is_none()
        {
            for (key, times) in input.brain_sort_keys.iter().zip([1, 6, 8]) {
                if *key {
                    self.inspector_brain_sort_n(times);
                }
            }
            if input.brain_hidden_down {
                self.inspector_brain_sort_hidden(
                    input.dt,
                    if input.right_ctrl_down { 0.0 } else { 1.0 },
                );
            }
        }
        if input.lay_egg {
            self.invoke_command(crate::ui::ControlCommand::LayEggs);
        }
        if input.kill_target {
            if self.multi.is_some() {
                // `UserControl.Update`: Delete acts on the multi-selection when there is one.
                self.kill_multi_selection();
            } else {
                self.kill_selected();
            }
        }
        if input.camera_home {
            self.clear_target();
            self.camera.center = Vec2::ZERO;
            self.gradual_input = Vec2::ZERO;
            self.view_mode = ViewMode::Manual;
        }
        // `CameraManager.Update`/`LateUpdate`: PageUp/PageDown rescale the camera
        // speed; arrows/WASD move it with a smoothed input; Ctrl+right-drag
        // slides towards the cursor; middle-drag zooms.
        if input.speed_scale_step > 0 {
            self.camera_speed_scale *= 1.25;
        } else if input.speed_scale_step < 0 {
            self.camera_speed_scale /= 1.25;
        }
        let ortho = crate::render::zoom_to_ortho(self.camera.zoom, self.camera.screen.y.max(1.0));
        let cam_speed = ortho * self.camera_speed_scale;
        let dt = input.dt.clamp(0.0, 0.25);
        // `CameraManager.LateUpdate`: `ArrowsMove` only runs with the setting on.
        let move_axis = if self.allow_arrow_keys {
            input.move_axis
        } else {
            Vec2::ZERO
        };
        if move_axis == Vec2::ZERO {
            self.gradual_input = Vec2::ZERO;
        } else {
            self.gradual_input += 2.0 * (move_axis - self.gradual_input) * dt;
            self.camera.center += self.gradual_input * cam_speed * dt;
            self.view_mode = ViewMode::Manual;
            self.follow_selected = false;
        }
        // `EffectTower.Update`: the right button cancels a tower that is still being placed.
        if input.secondary_down
            && matches!(
                self.tool,
                crate::ui::ToolKind::Tower
                    | crate::ui::ToolKind::PheroTower
                    | crate::ui::ToolKind::Killer
                    | crate::ui::ToolKind::GeneSelector
                    | crate::ui::ToolKind::Vent
                    | crate::ui::ToolKind::Force
                    | crate::ui::ToolKind::Whirl
            )
        {
            self.tool = crate::ui::ToolKind::Inspect;
        }
        // `CameraManager.MouseDrag` / `MouseSlide`: holding the right button deselects the target.
        if input.secondary_down && self.has_target() {
            self.clear_target();
        }
        if input.secondary_down && input.ctrl_down {
            self.pan_last = None;
            self.follow_selected = false;
            let screen = self.camera.screen;
            let offset = 2.0 * (input.cursor - screen * 0.5) / screen.y.max(1.0);
            let slide = offset * offset.length() * cam_speed * dt;
            // Screen y grows downwards, world y upwards.
            self.camera.center += Vec2::new(slide.x, -slide.y);
            self.view_mode = ViewMode::Manual;
        }
        if input.middle_down {
            if input.middle_pressed {
                self.scroll_drag_start_y = input.cursor.y;
            }
            // `MouseDragScroll`: screen y grows downwards while Unity's grows up.
            let amount = (self.scroll_drag_start_y - input.cursor.y)
                / self.camera.screen.y.max(1.0)
                * self.camera_speed_scale
                * dt;
            if amount != 0.0 {
                let height = self.camera.screen.y.max(1.0);
                let target = crate::render::clamp_ortho_size(
                    ortho * (1.0 - amount),
                    self.world.settings.independents.simulation_size,
                );
                self.camera.zoom = crate::render::ortho_to_zoom(target, height);
                self.zoom_target = None;
                self.view_mode = ViewMode::Manual;
            }
        }
        // Right-drag pans the camera and drops the follow target.
        if input.secondary_down && !input.ctrl_down {
            if let Some(last) = self.pan_last {
                let delta = input.cursor - last;
                if delta.length_sq() > 0.0 {
                    self.camera.pan_screen(delta);
                    self.view_mode = ViewMode::Manual;
                    self.follow_selected = false;
                }
            }
            self.pan_last = Some(input.cursor);
        } else {
            self.pan_last = None;
        }
        if input.zoom_key != 0.0 {
            // `CameraManager.CameraZoom(ratio)`: camSize -= camSize * ratio.
            let height = self.camera.screen.y.max(1.0);
            let ortho = crate::render::zoom_to_ortho(self.camera.zoom, height);
            let target = crate::render::clamp_ortho_size(
                ortho * (1.0 - input.zoom_key),
                self.world.settings.independents.simulation_size,
            );
            self.camera.zoom = crate::render::ortho_to_zoom(target, height);
            self.zoom_target = None;
            self.view_mode = ViewMode::Manual;
        }
        if let Some(key) = input.digit
            && self.target_is_bibite_or_egg()
        {
            // `GUIManager.Update` (cs:180-195): with a bibite or egg selected the digits 1-4 open
            // the Stats / Genes / Biology / Brain panels (an egg has no Biology panel: its 3 is
            // the Brain and 4 does nothing). Without a target the digits do nothing.
            let egg = self.selected.is_none();
            let tab = match (key, egg) {
                (1, _) => Some(InspectorTab::Stats),
                (2, _) => Some(InspectorTab::Genes),
                (3, false) => Some(InspectorTab::Biology),
                (3, true) | (4, false) => Some(InspectorTab::Brain),
                _ => None,
            };
            if let Some(tab) = tab {
                self.open_bibite_panel(tab, true);
            }
        }
        if input.step {
            self.paused = true;
            self.tick_world();
            self.step_tails();
            self.sim_ticks += 1;
            self.accumulator = 0.0;
            self.refresh_overlay(0.0);
        }
        if input.fit {
            self.fit_world();
        }
        if input.fit_bibites || input.reset_view {
            self.fit_bibites();
        }
        if input.toggle_grid {
            self.scene_options.grid = !self.scene_options.grid;
        }
        if input.toggle_zones {
            self.scene_options.zones = !self.scene_options.zones;
        }
        if input.toggle_pellets {
            self.scene_options.pellets = !self.scene_options.pellets;
        }
        if input.toggle_pheromones {
            self.scene_options.pheromones = !self.scene_options.pheromones;
        }
        if input.toggle_hud {
            self.show_hud = !self.show_hud;
        }
        if input.toggle_objects {
            self.toggle_objects();
        }
        // The mouse wheel scrolls the inspector's gene list when the cursor is
        // over the selection panel (the game ignores scroll over UI for zoom).
        let over_inspector = self.inspector_visible()
            && matches!(
                self.inspector_tab,
                InspectorTab::Genes | InspectorTab::Biology | InspectorTab::Brain
            )
            && input.cursor.x
                > self.camera.screen.x - 450.0 * (self.camera.screen.y / 1080.0).max(0.5)
            && input.cursor.y < 460.0 * (self.camera.screen.y / 1080.0).max(0.5);
        if input.wheel.abs() > 0.0 && over_inspector {
            let rows = match (self.inspector_tab, self.selection.as_ref()) {
                (InspectorTab::Biology, Some(panel)) => panel.biology.len(),
                (InspectorTab::Brain, Some(panel)) => panel.brain_rows.len(),
                _ => 35,
            };
            let max_start = rows.saturating_sub(12);
            if input.wheel > 0.0 {
                self.genes_scroll = self.genes_scroll.saturating_sub(2);
            } else {
                self.genes_scroll = (self.genes_scroll + 2).min(max_start);
            }
        } else if input.wheel.abs() > 0.0 {
            // Game `CameraManager.CameraZoom`: camSize -= camSize*ratio with
            // ratio = delta/10 * scrollSpeed, clamped to [5, max(250, 2*size)]. It sets the
            // orthographic size at once, so the zoom is about the screen centre (the camera
            // does not move) and there is no easing.
            // Live value + formula: assets/game/reference/camera/live_camera_controls.json
            // (CameraManager.scrollSpeed = 1.0, speedScale = 1.0, camSize = 400).
            const SCROLL_SPEED: f32 = 1.0;
            let height = self.camera.screen.y.max(1.0);
            let current_ortho = crate::render::zoom_to_ortho(self.camera.zoom, height);
            let ratio = crate::render::wheel_zoom_ratio(input.wheel, SCROLL_SPEED);
            let target_ortho = crate::render::clamp_ortho_size(
                current_ortho * (1.0 - ratio),
                self.world.settings.independents.simulation_size,
            );
            self.camera.zoom = crate::render::ortho_to_zoom(target_ortho, height);
            self.zoom_target = None;
            self.view_mode = ViewMode::Manual;
        }
        if self.placer.is_some() {
            // `BibitePlacer.UpdatePanel`: every left press off the UI places a bibite; clicks
            // select nothing meanwhile (`AllowSelection` is off).
            if input.primary_pressed {
                // A press on the world takes the focus off the custom tag field first.
                if self.tag_edit.is_some() {
                    self.commit_species_tag();
                }
                let world = self.camera.screen_to_world(input.cursor);
                self.place_bibite(world);
            }
        } else if input.primary_pressed && self.tool != crate::ui::ToolKind::Inspect {
            let world = self.camera.screen_to_world(input.cursor);
            self.apply_tool(world);
        } else if input.primary_down && self.tool == crate::ui::ToolKind::Pellet {
            // `PelletPlacer.UpdatePanel`: holding the button keeps placing a
            // pile every 1.5 pile radii of cursor travel.
            let world = self.camera.screen_to_world(input.cursor);
            let pile = self.world.pellet_placer_pile_radius(
                self.tool_settings.material,
                self.tool_settings.pellet_count,
                self.tool_settings.pellet_energy,
            );
            if self
                .pellet_last_placement
                .is_none_or(|last| last.distance(world) > 1.5 * pile)
            {
                self.apply_tool(world);
            }
        } else {
            // A press on the world takes the focus off the tag field, which confirms it
            // (`TMP_InputField.onEndEdit`), before the click can change the target; a zone's
            // name field just closes (`onDeselect`).
            if input.primary_pressed && self.tag_edit.is_some() {
                self.commit_species_tag();
            }
            if input.primary_pressed {
                self.zone_name_edit = None;
                self.value_edit = None;
            }
            self.handle_drag(input);
        }
    }

    /// The left button on the world (`UserControl.EnableMultiSelect`): a press shorter than
    /// [`crate::selection::CLICK_MAX_SECONDS`] is a click, a longer one draws a selection
    /// rectangle (the left button never pans: the camera pans with the right button).
    fn handle_drag(&mut self, input: &FrameInput) {
        if input.primary_pressed {
            let world = self.camera.screen_to_world(input.cursor);
            let on_entity = crate::selection::entity_at(&self.world, world);
            let on_pellet = if on_entity.is_none() {
                crate::selection::pellet_at(&self.world, world)
            } else {
                None
            };
            // The towers are drawn below the bibites, eggs and pellets: they take the press only
            // where none of those is.
            let on_tool = if on_entity.is_none() && on_pellet.is_none() {
                crate::selection::tool_at(&self.world, world)
            } else {
                None
            };
            let grab = match on_entity {
                Some(crate::selection::Entity::Bibite(id)) => Some(Grab::Bibite(id)),
                Some(crate::selection::Entity::Egg(id)) => Some(Grab::Egg(id)),
                None => on_pellet
                    .map(Grab::Pellet)
                    .or_else(|| on_tool.map(Grab::Tool)),
            };
            self.drag = Some(DragState {
                start: input.cursor,
                cursor: input.cursor,
                held: 0.0,
                on_entity,
                on_tool,
                grab,
            });
        }
        if !input.primary_down && !input.primary_released {
            // The release was missed (focus lost mid-drag): drop the gesture.
            self.drag = None;
        }
        if input.primary_down
            && let Some(drag) = &mut self.drag
        {
            drag.cursor = input.cursor;
            drag.held += input.dt.max(0.0);
        }
        if input.primary_down
            && let Some(drag) = self.drag
            && let Some(grab) = drag.grab
        {
            self.drag_body(grab, drag.held, input);
        }
        if input.primary_released
            && let Some(drag) = self.drag.take()
        {
            if drag.held < crate::selection::CLICK_MAX_SECONDS {
                self.click_select(drag.on_entity, drag.on_tool, input.cursor);
            } else if drag.on_entity.is_none() && drag.on_tool.is_none() {
                self.rectangle_select(drag.start, input.cursor);
            }
        }
    }

    /// `DraggableObject.OnMouseDrag`: a body held by the button follows the cursor. Bibites and eggs
    /// (targetable) only after the click time and never while selected; pellets at once. Ctrl turns
    /// the body towards the cursor instead. Nothing moves in a challenge or while the pause menu
    /// has the controls.
    fn drag_body(&mut self, grab: Grab, held: f32, input: &FrameInput) {
        if self.challenge_is_running() || self.panels.is_open(crate::ui::PanelKind::Escape) {
            return;
        }
        let targetable = !matches!(grab, Grab::Pellet(_));
        if targetable && held < crate::selection::CLICK_MAX_SECONDS {
            return;
        }
        let mouse = self.camera.screen_to_world(input.cursor);
        let paused = self.paused;
        let speed = self.engine_speed().max(MIN_SPEED);
        let (pos, rot) = match grab {
            Grab::Bibite(id) => match self.world.bibites.get(id) {
                Some(record) => (record.pos, Some(record.rot)),
                None => return,
            },
            Grab::Egg(id) => match self.world.eggs.get(id) {
                Some(egg) => (egg.pos, Some(egg.rot)),
                None => return,
            },
            Grab::Pellet(id) => match self.world.pellets.pos(id) {
                Some(pos) => (pos, None),
                None => return,
            },
            // A tower or colour killer has no rigid body: it jumps to the cursor. The selected one
            // stays put and one that follows a zone is put back every tick (`set_tool_position`).
            Grab::Tool(save_id) => {
                if self.selected_tool == Some(save_id) {
                    return;
                }
                if let Some(id) = self.tool_id_of(save_id)
                    && self.world.set_tool_position(id, mouse)
                {
                    self.refresh_overlay(1.0);
                }
                return;
            }
        };
        let offset = mouse - pos;
        if input.ctrl_down {
            // Turn: `up += (1 - |mouse from centre|) * speed / 2 * dir * dt`; paused, snap to the cursor.
            let Some(rot) = rot else {
                return;
            };
            let up = Vec2::Y.rotate(rot);
            let direction = offset.normalize_or(Vec2::ZERO);
            let new_up = if paused {
                direction
            } else {
                let screen = self.camera.screen;
                let from_centre =
                    (input.cursor - screen * 0.5) / screen.x.max(screen.y).max(1.0) * 2.0;
                let steer = (1.0 - from_centre.length()) * DRAG_SPEED / 2.0;
                (up + direction * steer * input.dt.max(0.0)).normalize_or(Vec2::ZERO)
            };
            if new_up != Vec2::ZERO {
                let new_rot = (-new_up.x).atan2(new_up.y);
                match grab {
                    Grab::Bibite(id) => {
                        if let Some(record) = self.world.bibites.get_mut(id) {
                            record.rot = new_rot;
                        }
                    }
                    Grab::Egg(id) => {
                        if let Some(egg) = self.world.eggs.get_mut(id) {
                            egg.rot = new_rot;
                        }
                    }
                    Grab::Pellet(_) | Grab::Tool(_) => {}
                }
            }
            return;
        }
        // The selected body is not dragged (`controller.CompareTarget`).
        let is_target = match grab {
            Grab::Bibite(id) => self.selected == Some(id),
            Grab::Egg(id) => self.selected_egg == Some(id),
            Grab::Pellet(_) | Grab::Tool(_) => false,
        };
        if is_target {
            return;
        }
        // Running: velocity `speed * offset / timeScale`; paused: jump to the cursor.
        let velocity = if paused {
            Vec2::ZERO
        } else {
            offset * (DRAG_SPEED / speed)
        };
        match grab {
            Grab::Bibite(id) => {
                if let Some(record) = self.world.bibites.get_mut(id) {
                    record.vel = velocity;
                    if paused {
                        record.pos = mouse;
                    }
                }
            }
            Grab::Egg(id) => {
                if let Some(egg) = self.world.eggs.get_mut(id) {
                    egg.vel = velocity;
                    if paused {
                        egg.pos = mouse;
                    }
                }
            }
            Grab::Pellet(id) => {
                if paused {
                    self.world.move_pellet(id, mouse);
                } else {
                    self.world.pellets.set_velocity(id, velocity);
                }
            }
            Grab::Tool(_) => {}
        }
    }

    /// Drops the multi-selection and the single target (`UserControl.SelectTarget(null)`), which
    /// also cancels a pending Ctrl-repeat.
    fn clear_target(&mut self) {
        self.reselect = None;
        self.reselect_tag = None;
        self.reselect_timer = None;
        self.corpse_listener = false;
        self.multi = None;
        self.selected = None;
        self.selected_egg = None;
        self.selection = None;
        self.follow_selected = false;
        self.selected_tool = None;
        self.selected_zone = None;
        self.selected_autopan = false;
        self.tool_panel_open = false;
        self.tool_drag = None;
    }

    /// Whether the tag field being typed into is the placer's custom tag.
    #[must_use]
    pub fn tag_edit_is_placer(&self) -> bool {
        self.tag_edit_is_placer
    }

    /// `BibitePlacer.ButtonFromTemplatePressed`: the custom tag resets, the template selector
    /// opens and the game pauses while it is open.
    fn begin_bibite_placer(&mut self) {
        self.placer_options.tag.clear();
        self.template_pick = Some(TemplateUse::PlaceBibite);
        if self.placer_pause.is_none() {
            self.placer_pause = Some(self.paused);
            self.paused = true;
            self.accumulator = 0.0;
        }
        self.panels.open(crate::ui::PanelKind::Library);
    }

    /// The template selector closed without a choice (`onCancelAction`), or was left for any other
    /// reason: the pick ends and the game goes back to its previous pause.
    pub fn cancel_template_pick(&mut self) {
        self.template_pick = None;
        self.restore_placer_pause();
    }

    /// `BibitePlacer.RemoveSelectorListeners`: `TogglePauseGame("bibitePlacer", isUnpause: true)`.
    fn restore_placer_pause(&mut self) {
        if let Some(previous) = self.placer_pause.take() {
            self.paused = previous;
            self.accumulator = 0.0;
        }
    }

    /// `BibitePlacer.PlaceBibitesFromTemplate`: the picked template becomes the one placed. The
    /// selector's pause ends, the target is dropped, selection is disabled (`AllowSelection`) and
    /// the options panel opens.
    pub fn start_bibite_placer(&mut self, path: &Path) {
        self.template_pick = None;
        self.restore_placer_pause();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        match bibites_io::load_template(path) {
            Ok(loaded) => {
                let template = TemplateSource::new(
                    file_name.to_owned(),
                    loaded.genome,
                    loaded.meta,
                    loaded.brain,
                );
                self.clear_target();
                self.placer = Some(BibitePlacer { template });
                self.last_action = format!("placing {file_name}");
            }
            Err(error) => self.last_action = format!("could not read {file_name}: {error}"),
        }
    }

    /// `BibitePlacer.ClosePanel`: placement mode ends and selection works again.
    fn close_placer(&mut self) {
        self.placer = None;
        if self.tag_edit_is_placer {
            self.tag_edit = None;
            self.tag_edit_is_placer = false;
        }
    }

    /// `BibitePlacer.UpdatePanel`: a left press on the world spawns the template there with the
    /// panel's choices (`WorldObjectsSpawner.SpawnBibiteFromTemplate`); an egg takes the randomize-genes
    /// choice as well (`SpawnEggFromTemplate`).
    fn place_bibite(&mut self, pos: Vec2) {
        let Some(placer) = &self.placer else {
            return;
        };
        let options = &self.placer_options;
        let tag = options.tag.clone();
        let result = if options.growth_at_spawn == GrowthAtSpawn::Egg {
            self.world
                .spawn_template_at_randomized(
                    &placer.template,
                    pos,
                    None,
                    GrowthAtSpawn::Egg,
                    options.tagging,
                    &tag,
                    options.randomize_genes,
                )
                .map(|_| ())
        } else {
            self.world
                .spawn_bibite_from_template(
                    &placer.template,
                    options.growth_at_spawn,
                    options.tagging,
                    &tag,
                    pos,
                    options.randomize_genes,
                )
                .map(|_| ())
        };
        match result {
            Ok(()) => {
                self.last_action = format!("placed {}", placer.template.name);
                self.refresh_overlay(1.0);
            }
            Err(error) => self.last_action = format!("placing failed: {error}"),
        }
    }

    /// The tower or colour killer with this save id, in the tool table.
    fn tool_id_of(&self, save_id: u64) -> Option<bibites_core::ids::ToolId> {
        self.world
            .tools
            .tools()
            .iter()
            .position(|tool| tool.save_id() == save_id)
            .and_then(bibites_core::tools::ToolStore::tool_id_at)
    }

    /// The selected tower or colour killer, in the tool table.
    #[must_use]
    pub fn selected_tool_id(&self) -> Option<bibites_core::ids::ToolId> {
        self.selected_tool
            .and_then(|save_id| self.tool_id_of(save_id))
    }

    /// `UserControl.target != null`: a bibite, an egg, a tower or a colour killer is selected.
    fn has_target(&self) -> bool {
        self.target_is_bibite_or_egg()
            || self.selected_tool.is_some()
            || self.selected_zone.is_some()
            || self.selected_autopan
    }

    /// `UserControl.SelectTarget(tower)` and `GUIManager.OnTargetChanged`: the previous target and
    /// the multi-selection go, the bibite panels close and the tower's panel opens. Selecting the
    /// tower that is already the target does nothing.
    fn select_tool(&mut self, save_id: u64) {
        if self.selected_tool == Some(save_id) || self.tool_id_of(save_id).is_none() {
            return;
        }
        self.clear_target();
        self.selected_tool = Some(save_id);
        self.tool_panel_open = true;
        // The camera follows the target, whatever it is (`CameraManager.SetFollowTarget`).
        self.follow_selected = true;
    }

    /// `EffectTower.Place`: a placed tower selects itself.
    pub(crate) fn select_placed_tool(&mut self, id: bibites_core::ids::ToolId) {
        if let Some(save_id) = self
            .world
            .tools
            .get(id)
            .map(bibites_core::tools::Tool::save_id)
        {
            self.select_tool(save_id);
        }
    }

    /// The tower panel's Remove button and the Delete key on a colour killer
    /// (`PheroTowerPanel.DestroyTarget`, `GUIManager.TryKillTarget`): the tool is destroyed and
    /// the target cleared.
    fn remove_selected_tool(&mut self) {
        let Some(id) = self.selected_tool_id() else {
            return;
        };
        if self.world.remove_tool(id) {
            self.last_action = "removed tower".to_owned();
        }
        self.clear_target();
        self.refresh_overlay(1.0);
    }

    /// A tower panel slider follows the cursor (`SettingSlider.SetValueFromSlider`).
    pub fn tool_slide(&mut self, field: u8, track: (f32, f32), cursor_x: f32) {
        let Some(id) = self.selected_tool_id() else {
            return;
        };
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        if span <= 0.0 {
            return;
        }
        crate::tower_panel::slide(
            &mut self.world,
            id,
            field,
            (cursor_x / scale - track.0) / span,
        );
    }

    /// What the scene draws for the towers: the placement ghost following the cursor and the
    /// selected tool.
    fn tool_view(&self) -> crate::render::ToolView {
        use crate::render::GhostKind;
        let ghost = match self.tool {
            crate::ui::ToolKind::Tower => Some(GhostKind::Rad),
            crate::ui::ToolKind::PheroTower => Some(GhostKind::Phero),
            crate::ui::ToolKind::Killer => Some(GhostKind::Killer),
            crate::ui::ToolKind::GeneSelector => Some(GhostKind::Gene),
            crate::ui::ToolKind::Vent => Some(GhostKind::Vent),
            crate::ui::ToolKind::Force => Some(GhostKind::Force),
            crate::ui::ToolKind::Whirl => Some(GhostKind::Whirl),
            _ => None,
        };
        crate::render::ToolView {
            ghost: ghost.map(|kind| (kind, self.camera.screen_to_world(self.cursor))),
            selected: self.selected_tool,
        }
    }

    /// A short click (`SelectClosestBibiteAroundCursor` or the pressed body's own click handler).
    /// Clearing the multi-selection happens on every click; clicking empty space leaves the
    /// current target selected.
    fn click_select(
        &mut self,
        hit: Option<crate::selection::Entity>,
        tool: Option<u64>,
        cursor: Vec2,
    ) {
        self.multi = None;
        match hit {
            Some(crate::selection::Entity::Bibite(id)) => {
                self.select_bibite(id);
                return;
            }
            Some(crate::selection::Entity::Egg(id)) => {
                self.select_egg(id);
                return;
            }
            None => {}
        }
        // `TargetObjectOnClick` selects the tower first; `SelectClosestBibiteAroundCursor`, which
        // runs after it in the same frame, then replaces it when a bibite is within reach.
        if let Some(save_id) = tool {
            self.select_tool(save_id);
        }
        let height = self.camera.screen.y.max(1.0);
        let radius = crate::render::zoom_to_ortho(self.camera.zoom, height) * self.selection_radius;
        let center = self.camera.screen_to_world(cursor);
        let camera = &self.camera;
        let screen = camera.screen;
        let on_screen = |point: Vec2| {
            let at = camera.world_to_screen(point);
            at.x >= 0.0 && at.x <= screen.x && at.y >= 0.0 && at.y <= screen.y
        };
        if let Some(id) =
            crate::selection::closest_bibite_in_circle(&self.world, center, radius, on_screen)
        {
            self.select_bibite(id);
        }
    }

    /// A released drag (`UserControl.SelectObjectsInRectangle`).
    fn rectangle_select(&mut self, start: Vec2, end: Vec2) {
        let camera = &self.camera;
        let picked = crate::selection::select_in_rectangle(
            &self.world,
            |point| camera.world_to_screen(point),
            camera.screen,
            start,
            end,
        );
        let has_target = self.selected.is_some() || self.selected_egg.is_some();
        if picked.is_empty() {
            // Nothing inside: the panel closes and the current target is dropped.
            self.clear_target();
        } else if picked.len() == 1 && picked.bibites.len() == 1 {
            // A lone bibite becomes the ordinary selection (no panel).
            let id = picked.bibites[0];
            self.multi = None;
            self.select_bibite(id);
        } else if picked.len() == 1 {
            // A lone egg or pellet opens the panel; the target stays.
            self.multi = Some(picked);
        } else {
            // The game opens the panel, then deselects the target, and that deselection closes
            // the panel again: with something selected a drag only clears the target.
            self.multi = Some(picked);
            if has_target {
                self.clear_target();
            }
        }
    }

    /// `SelectionPanel.KillSelection` (Delete key and the panel's button); closes the panel.
    fn kill_multi_selection(&mut self) {
        let Some(multi) = self.multi.take() else {
            return;
        };
        let affected = multi.kill(&mut self.world);
        self.last_action = format!("deleted {affected} entities");
        self.refresh_overlay(1.0);
    }

    /// Applies actions emitted by the themed HUD (UI-first input routing).
    pub fn apply_ui_actions(&mut self, actions: &[UiAction]) {
        for action in actions {
            // Pressing anything but one of the Brain tab's own text fields takes the keyboard off them (the event system deselects
            // the field on any other press; its text stays, as `OnDeselect` is not Escape).
            if !matches!(
                action,
                UiAction::Editor(crate::template_editor_ui::EditorAction::Brain(
                    crate::brain_editor_ui::BrainAction::FocusLabel
                        | crate::brain_editor_ui::BrainAction::FocusFilter
                ))
            ) && let Some(session) = self.bibite_editor.as_mut()
            {
                session.view.brain.label_focus = false;
                session.view.brain.filter_focus = false;
            }
            // Pressing any other control takes the focus off a species text field, which ends the
            // edit (`TMP_InputField.onEndEdit`) before the control acts.
            if self.species_ui.typing()
                && !matches!(
                    action,
                    UiAction::SpeciesNameEdit | UiAction::SpeciesNotes | UiAction::Consume
                )
            {
                self.species_commit_edit();
            }
            // RUSTibites Improved: a zone's colour pop-up closes (the zone keeps its colour) on a press on anything but itself.
            if self.zone_color_popup.is_some()
                && !matches!(
                    action,
                    UiAction::ZoneColorOpen(_)
                        | UiAction::ZoneColorWheel
                        | UiAction::ZoneColorValueBar
                        | UiAction::ZoneColorHexField
                        | UiAction::ZoneColorClear(_)
                        | UiAction::ZoneColorSpread
                        | UiAction::ZoneColorClose
                        | UiAction::Consume
                )
            {
                self.zone_color_close();
            }
            // RUSTibites Improved: the same for a time tag's name field (its own field and the Tag time buttons are the exceptions).
            if self.time_tag_edit.is_some() && !matches!(action, UiAction::TimeTagField | UiAction::TimeTagStart(_) | UiAction::TimeTagPlace(..)) {
                self.time_tag_edit = None;
            }
            // Pressing anything but the edit field takes the focus off it (`onDeselect`): the field
            // closes without applying its text.
            if self.value_edit.is_some()
                && !matches!(
                    action,
                    UiAction::ValueEditInside | UiAction::DynEditValue(_)
                )
            {
                self.value_edit = None;
            }
            match *action {
                UiAction::TogglePause => {
                    self.paused = !self.paused;
                    self.accumulator = 0.0;
                }
                UiAction::Step => {
                    self.paused = true;
                    self.tick_world();
                    self.step_tails();
                    self.sim_ticks += 1;
                    self.accumulator = 0.0;
                    self.refresh_overlay(0.0);
                }
                UiAction::SetSpeed(speed) => self.set_target_speed(speed),
                UiAction::ToggleOverlay(kind) => match kind {
                    OverlayKind::Grid => self.scene_options.grid = !self.scene_options.grid,
                    OverlayKind::Ruler => self.show_ruler = !self.show_ruler,
                    OverlayKind::Zones => self.scene_options.zones = !self.scene_options.zones,
                    OverlayKind::ZoneColors => self.toggle_zone_colors(),
                    OverlayKind::Pellets => {
                        self.scene_options.pellets = !self.scene_options.pellets
                    }
                    OverlayKind::Pheromones => {
                        self.scene_options.pheromones = !self.scene_options.pheromones;
                    }
                    OverlayKind::Hud => self.show_hud = !self.show_hud,
                    OverlayKind::StatusBars => self.show_status_bars = !self.show_status_bars,
                    OverlayKind::FieldOfView => {
                        self.scene_options.fov = !self.scene_options.fov;
                    }
                    OverlayKind::Targets => self.show_targets = !self.show_targets,
                },
                UiAction::Consume => {}
                // The tab buttons: `GUIManager.OpenBibitePanel(panel)` (pressing the open one closes it).
                UiAction::SetTab(tab) => self.open_bibite_panel(tab, true),
                UiAction::CloseInspector => {
                    self.selected = None;
                    self.selected_egg = None;
                    self.selection = None;
                }
                UiAction::DismissPopup => self.lineage_gate_popup = false,
                // Title-menu commands are handled by the menu loop, not the world.
                UiAction::Menu(_) => {}
                UiAction::ToggleScenarioSetting(index) => {
                    self.toggle_scenario_setting(usize::from(index))
                }
                // Handled continuously while the button is held (`speed_slide`).
                UiAction::SpeedSlide(..) => {}
                UiAction::DynTab(tab) => {
                    self.dyn_panel.tab = tab.min(4);
                    self.dyn_panel.scroll = 0.0;
                }
                UiAction::DynToggle(index) => self.dyn_toggle(usize::from(index)),
                // Handled continuously while the button is held (`dyn_drag`).
                UiAction::DynSlide(..) => {}
                // The Position tab's slider follows the cursor while it is held (`position_slide`).
                UiAction::PositionSlide(..) | UiAction::PositionSlideHandle(..) => {}
                // A held neuron of the Brain tab follows the cursor (`brain_drag_to`).
                UiAction::BrainNode(_) | UiAction::InspectorBrainNode(_) => {}
                UiAction::BrainSort => self.brain_sort(),
                UiAction::InspectorBrainSort => self.inspector_brain_sort_n(1),
                UiAction::BrainSavePositions => self.brain_save_positions(),
                UiAction::BrainResetPositions => self.brain_reset_positions(),
                UiAction::BrainToggleDifferences => self.brain_toggle_differences(),
                UiAction::ToggleEnergySort => self.sort_energy_details = !self.sort_energy_details,
                UiAction::ToggleMapZoneNames => self.toggle_map_zone_names(),
                UiAction::ToggleMapZoneBiomass => self.toggle_map_zone_biomass(),
                UiAction::DynEditValue(field) => self.open_value_edit(field),
                UiAction::ValueEditInside => {}
                UiAction::DynReset(index) => self.dyn_reset(&[usize::from(index)]),
                UiAction::DynSub(group) => self.dyn_panel.toggle_sub(group),
                UiAction::DynResetGroup(group) => {
                    let keys = crate::dyn_panel::sub_group_keys(self.dyn_panel.tab, group);
                    let indices: Vec<usize> = keys
                        .iter()
                        .filter_map(|key| crate::dyn_panel::ui_of(key).map(|(index, _)| index))
                        .collect();
                    self.dyn_reset(&indices);
                }
                // Scenario-editor controls: only the title-menu editor handles them.
                // A tower panel slider drag is followed by the window loop (`tool_slide`).
                UiAction::ToolSlide(..) => {}
                UiAction::SaveDialogFocus(which) => {
                    if let Some(dialog) = self.save_dialog.as_mut() {
                        dialog.focus_field(if which == 0 {
                            crate::save_dialog::Field::Name
                        } else {
                            crate::save_dialog::Field::Description
                        });
                    }
                }
                UiAction::SaveDialogToggle(which) => self.toggle_save_dialog_option(which),
                UiAction::ScenarioSaver(code) => self.scenario_saver_action(code),
                // A dropdown control was clicked: its list opens under it (`TMP_Dropdown.Show`).
                UiAction::DropdownOpen(target) => {
                    let anchor = self.click_rect;
                    self.open_dropdown_list(target, anchor);
                }
                UiAction::DropdownPick(index) => self.pick_dropdown_list(usize::from(index)),
                UiAction::DropdownClose => {
                    self.dropdown_open = None;
                    self.dropdown = None;
                }
                // `BibitePlacer`'s dropdowns (`SettingDropdown`: the next choice).
                UiAction::PlacerCycle(field) => {
                    crate::bibite_editor::cycle(&mut self.placer_options, field, &[]);
                }
                UiAction::ToolZone => {
                    if let Some(id) = self.selected_tool_id() {
                        crate::tower_panel::cycle_zone(&mut self.world, id);
                        self.refresh_overlay(1.0);
                    }
                }
                UiAction::ToolSwitch(field) => {
                    if let Some(id) = self.selected_tool_id() {
                        crate::tower_panel::flip_switch(&mut self.world, id, field);
                    }
                }
                // The world's default bibites: a slider drag is followed by the window loop.
                UiAction::BibiteSlide(..) => {}
                UiAction::BibiteCycle(index, field) => {
                    let zones: Vec<_> = self
                        .world
                        .zones
                        .zones
                        .iter()
                        .map(|zone| zone.config.0.clone())
                        .collect();
                    if let Some(row) = self.world.bibite_spawns.get_mut(usize::from(index)) {
                        crate::bibite_editor::cycle(row, field, &zones);
                    }
                }
                UiAction::BibiteReset(index, field) => {
                    if let Some(row) = self.world.bibite_spawns.get_mut(usize::from(index)) {
                        crate::bibite_editor::reset_value(row, field);
                    }
                }
                // `DefaultBibitesPanel.ItemClicked`: one item is open at a time; a click selects its item, and the open one is never closed by a click
                // (`BibiteSettingsHandle.OnPointerClick` only reacts while the edit section is closed).
                UiAction::ZoneOpen(2, index) => {
                    self.dyn_panel.open_items.retain(|open| open.0 != 2);
                    self.dyn_panel.open_items.push((2, index));
                }
                // The world's zones (`ZoneSettingsHandle`): a slider drag is followed by the
                // window loop, the other controls change the live zone at once.
                UiAction::ZoneSlide(..) => {}
                UiAction::ZoneToggle(index, field) => {
                    if let crate::zone_editor::ZoneRef::Animator(changer) = crate::zone_editor::zone_ref(index) {
                        if field == crate::zone_editor::ANIM_REPEAT {
                            self.animator_toggle_repeat(changer);
                        }
                    } else if let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(index) {
                        if field == crate::zone_editor::GROUP_SCALE {
                            self.edit_group(group, |group| group.scale_with_sim = !group.scale_with_sim);
                        }
                    } else {
                        let size = self.world.settings.independents.simulation_size;
                        self.edit_zone(usize::from(index), |zone, _| {
                            crate::zone_editor::toggle(zone, field);
                            crate::zone_editor::cap_position(zone, size);
                        });
                    }
                }
                UiAction::ZoneCycle(index, field) => {
                    let size = self.world.settings.independents.simulation_size;
                    self.edit_zone(usize::from(index), |zone, zones| {
                        crate::zone_editor::cycle(zone, field, zones, usize::from(index));
                        crate::zone_editor::cap_position(zone, size);
                    });
                }
                UiAction::ZoneReset(index, field) => {
                    if matches!(crate::zone_editor::zone_ref(index), crate::zone_editor::ZoneRef::Animator(_)) {
                        // The Start Time slider's reset button.
                        if let Some((info, _)) = self.animator_slider(index, field) {
                            self.animator_set_value(index, field, info.default, true);
                        }
                    } else {
                        let size = self.world.settings.independents.simulation_size;
                        self.edit_zone(usize::from(index), |zone, _| {
                            crate::zone_editor::reset(zone, field, size);
                        });
                    }
                }
                // `TextLineHandle.OpenEditSection`: the name becomes a text field.
                UiAction::ZoneNameEdit(index) => {
                    if let crate::zone_editor::ZoneRef::Animator(changer) = crate::zone_editor::zone_ref(index) {
                        if let Some(changer) = self.world.settings_changers.get(changer) {
                            self.zone_name_edit = Some((usize::from(index), changer.name.clone()));
                        }
                    } else if let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(index) {
                        if let Some(group) = self.world.zone_groups.get(group) {
                            self.zone_name_edit = Some((usize::from(index), group.name.clone()));
                        }
                    } else if let Some(zone) = self.world.zones.zones.get(usize::from(index)) {
                        self.zone_name_edit =
                            Some((usize::from(index), zone.config.0.name.clone()));
                    }
                }
                // `ZoneSettingsHandle`: an item opens and closes on its own (the bar closes it, a
                // click on a closed item opens it); several can be open.
                // A zone group's card (`ZoneGroupHandle.ToggleBody`) opens and closes on its own bar as well.
                UiAction::ZoneOpen(list @ (0 | 1 | 3 | 4), index) => {
                    let item = (list, index);
                    if let Some(at) = self.dyn_panel.open_items.iter().position(|o| *o == item) {
                        self.dyn_panel.open_items.remove(at);
                    } else {
                        self.dyn_panel.open_items.push(item);
                    }
                }
                UiAction::ZoneOpen(..) => {}
                // `ReorderableItem`: the card swaps places with its neighbour in the list as it is shown (the data keeps its order).
                UiAction::ListMove(list, index, up) => {
                    let len = match list {
                        0 => self.world.zones.explicit_count(),
                        1 => self.world.zone_groups.len(),
                        _ => self.world.settings_changers.len(),
                    };
                    self.dyn_panel.move_card(list, len, usize::from(index), up);
                }
                // The curve's graph: the window loop follows the press (a drag).
                UiAction::AnimGraph(..) => {}
                // The Energy tab's material blocks (`MatterMaterialSettingsHandle`): a slider drag
                // is followed by the window loop, the other controls act at once.
                UiAction::MaterialSlide(..) => {}
                UiAction::MaterialToggle(kind) => {
                    self.edit_material(kind, |slot, _| slot.decay = !slot.decay);
                }
                UiAction::MaterialReset(kind, field) => {
                    self.edit_material(kind, |slot, material| {
                        crate::zone_editor::reset_material_field(slot, material, field);
                    });
                }
                UiAction::MaterialResetAll(kind) => {
                    self.edit_material(kind, |slot, material| {
                        *slot = *bibites_core::settings::MaterialSettings::game_defaults()
                            .get(material);
                    });
                }
                // `DynamicSettingsPanel.OpenScenarioSaver`.
                UiAction::DynSaveScenario => self.open_scenario_saver(),
                UiAction::ShowToolbar(which) => {
                    if let Some(panel) = self.toolbar.get_mut(usize::from(which)) {
                        panel.show();
                    }
                }
                // `scaleButtons[k].interactable = stream.DepthHasData(k)`: a scale the stream has
                // no data for cannot be picked.
                UiAction::SetGraphsScale(index) => {
                    if self.graphs_depth_has_data(index) {
                        self.graphs_scale = index;
                    }
                }
                // `GraphsPanel.OpenGeneAnalysis` / `OpenHistoricalGraph`.
                UiAction::GraphsPage(genes) => self.graphs_genes = genes,
                UiAction::TimeTagStart(panel) => self.time_tag_open(panel),
                UiAction::TimeTagField => {}
                UiAction::TimeTagPlace(panel, seconds) => self.time_tag_place(panel, seconds),
                UiAction::ZoneColorOpen(zone) => self.zone_color_open(usize::from(zone)),
                // (The wheel and the bar are held by the window loop: `zone_color_press` / `zone_color_drag_to`.)
                UiAction::ZoneColorWheel | UiAction::ZoneColorValueBar => {}
                UiAction::ZoneColorHexField => self.zone_color_focus_hex(),
                UiAction::ZoneColorClear(zone) => self.zone_color_clear(usize::from(zone)),
                UiAction::ZoneColorSpread => self.zone_colors_spread(),
                UiAction::ZoneColorClose => self.zone_color_close(),
                UiAction::TimeTagRemove(id) => self.time_tag_remove(id),
                UiAction::TimeTagsShow(panel) => self.time_tags_toggle(panel),
                UiAction::Editor(action) => self.apply_editor_action(action),
                UiAction::ExpandedBrain(action) => self.apply_expanded_action(action),
                UiAction::SavePanel(action) => self.save_panel_action(action),
                // The Gene Analysis slider follows the cursor while it is held (`graphs_gene_slide`).
                UiAction::GenesTimeSlide(..) | UiAction::GenesTimeSlideHandle(..) => {}
                // Any positive target time scale forces play (`TimeController.SetTarget`).
                UiAction::SetSpeedSlider(speed) => {
                    self.set_target_speed(speed);
                    self.paused = false;
                    self.accumulator = 0.0;
                }
                // The lineage panel has the species-log gate and needs a data refresh.
                UiAction::OpenPanel(crate::ui::PanelKind::Lineage) => {
                    self.invoke_command(crate::ui::ControlCommand::OpenLineage);
                }
                UiAction::OpenPanel(panel) => self.panels.open(panel),
                UiAction::ClosePanel => {
                    self.panels.close_top();
                }
                UiAction::TogglePanel(panel) => self.panels.toggle(panel),
                UiAction::SetValue(_value) => {
                    // Bound by the owning panel state machine (settings/inspector).
                }
                UiAction::Invoke(command) => self.invoke_command(command),
                UiAction::MenuRow(_row) => {
                    // In-game menu rows are handled by the menu state machine.
                }
                UiAction::ActivateControl(_id) => {
                    // Unmapped audited runtime control: registry binding lands with
                    // bench's control registry follow-up.
                }
                UiAction::Quit => self.quit = true,
                UiAction::ToggleHelp => self.panels.toggle(crate::ui::PanelKind::Help),
                UiAction::SelectSaveRow(row) => {
                    self.save_selected = row as usize;
                    self.refresh_save_entries();
                }
                UiAction::ToggleAutosaves => {
                    self.include_autosaves = !self.include_autosaves;
                    // The checkbox is the user setting `IncludeAutoSavesInLoadPanel`.
                    if let Some(path) = self.user_settings_path.clone() {
                        let mut settings = crate::user_settings::UserSettings::load(Some(path));
                        if settings.flag("Include autosaves") != self.include_autosaves
                            && let Some(index) = crate::user_settings::index_of("Include autosaves")
                        {
                            settings.toggle(index);
                        }
                    }
                    self.refresh_save_entries();
                }
                // `PelletPlacer.ChangeMaterial`: Plant and Meat are the two physical materials.
                UiAction::PelletMaterial => {
                    self.tool_settings.material = match self.tool_settings.material {
                        bibites_core::pellet::MaterialKind::Plant => {
                            bibites_core::pellet::MaterialKind::Meat
                        }
                        _ => bibites_core::pellet::MaterialKind::Plant,
                    };
                }
                // A pellet placer slider drag is followed by the window loop (`pellet_slide`).
                UiAction::PelletSlide(..) => {}
                // The tags list's scroll bar is followed by the window loop (`InfoState::drag_bar`).
                UiAction::InformationScroll(..) => {}
                UiAction::PickDropdownChoice(index) => self.pick_dropdown_choice(index),
                UiAction::SelectGene(index) => {
                    self.editor_gene = index as usize;
                }
                UiAction::AdjustGene(delta) => self.adjust_editor_gene(delta),
                UiAction::ScrollGene(delta) => {
                    let page = self.editor_gene_page as i32 + delta;
                    self.editor_gene_page = page.max(0) as usize;
                    self.editor_gene = self.editor_gene_page * 14;
                }
                UiAction::SelectBrainNode(index) => {
                    self.editor_prev_node = self.editor_node;
                    self.editor_node = index as usize;
                }
                UiAction::SelectSpecies(index) => self.select_species(index as usize),
                UiAction::SetLineageTab(tab) => self.lineage_tab = tab.min(3) as u8,
                // `SpeciesInfoPanel.EditSpeciesName`: the name becomes a text field.
                UiAction::SpeciesNameEdit => {
                    if let Some(name) = self.species_info_entry().map(|entry| entry.name.clone()) {
                        self.species_commit_edit();
                        self.species_ui.name = Some(name);
                    }
                }
                // A click into the notes field.
                UiAction::SpeciesNotes => {
                    if self.species_ui.notes.is_none()
                        && let Some(text) = self
                            .species_info_entry()
                            .map(|entry| entry.description.clone())
                    {
                        self.species_commit_edit();
                        self.species_ui.notes = Some(text);
                    }
                }
                UiAction::SpeciesParent => self.species_select_parent(),
                UiAction::SpeciesRoot => self.species_select_root(),
                UiAction::SpeciesSave => self.species_save(),
                UiAction::SpeciesPlace => self.species_place(),
                UiAction::SpeciesPrune => self.species_prune(),
                // `SpeciesPanel.EditBibite` opens the Library's Bibite Editor on the species' template.
                UiAction::SpeciesEdit => self.species_edit(),
                UiAction::SpeciesClose => self.select_species_info(None),
                UiAction::RebindHotkey(index) => {
                    self.hotkey_listening = Some(index as usize);
                    self.last_action = "press a key to bind".to_owned();
                }
                UiAction::ResetHotkeys => self.reset_hotkeys(),
                UiAction::AdjustChallengeParam(index, delta) => {
                    self.adjust_challenge_param(index as usize, delta);
                }
                UiAction::SetSettingsPage(page) => self.settings_page = page.min(2) as u8,
                UiAction::SelectWorkshopRow(row) => {
                    self.workshop_selected = row as usize;
                }
                UiAction::SelectChallengeRow(row) => {
                    self.challenge_scenario_selected = row as usize;
                }
                UiAction::SelectChampionRow(row) => {
                    self.champion_selected = row as usize;
                }
                UiAction::ToggleLineageSettings => {
                    self.lineage_settings_open = !self.lineage_settings_open;
                    self.lineage_settings_grab = None;
                }
                UiAction::LineageOption(index) => {
                    if let Some(option) = crate::lineage_settings::LineageOption::from_index(index)
                    {
                        self.lineage_toggle_option(option);
                    }
                }
                // `SpeciesPanel.SelectPanel(tree)`: the Mesh and Tree buttons of the top bar.
                UiAction::SelectLineagePanel(tree) => self.lineage_select_panel(tree),
                // `LineageSpeciesNode.OnPointerClick`: the species of the node is selected.
                UiAction::SelectLineageNode(species) => {
                    let id = self
                        .lineage_species
                        .iter()
                        .find(|entry| entry.id.value() == species)
                        .map(|entry| entry.id);
                    if let Some(id) = id {
                        self.select_species_info(Some(id));
                        self.last_action = format!("lineage species {species}");
                    }
                }
            }
        }
        // The Information panel shows the numbers of the moment it opens (an `OpenPanel` of any route).
        self.information.frame(
            self.panels.is_open(crate::ui::PanelKind::Information),
            &self.world,
            0.0,
        );
    }

    /// Starts the bibite placer with `template` (`BibitePlacer.PlaceBibitesFromTemplate`).
    pub fn start_bibite_placer_with(&mut self, template: TemplateSource) {
        self.template_pick = None;
        self.restore_placer_pause();
        self.clear_target();
        self.last_action = format!("placing {}", template.name);
        self.placer = Some(BibitePlacer { template });
    }

    /// Applies a named UI command.
    pub(crate) fn invoke_command(&mut self, command: crate::ui::ControlCommand) {
        use crate::ui::ControlCommand;
        match command {
            ControlCommand::TogglePause => {
                self.paused = !self.paused;
                self.accumulator = 0.0;
            }
            ControlCommand::Step => {
                self.paused = true;
                self.tick_world();
                self.sim_ticks += 1;
                self.accumulator = 0.0;
            }
            // The pause menu stays open under Settings / Save / Load / the Library: `UserSettingsManager.OpenPanel`,
            // `SaveGamePanel.OpenPanel`, `LoadGamePanel.OpenPanel` and `BibiteTemplateSelectorPanel.OpenPanel` (its `panelToHide`
            // is not set in either scene) leave `EscapePanel` active under them, and closing them shows it again.
            ControlCommand::OpenSettings => self.panels.open(crate::ui::PanelKind::Settings),
            ControlCommand::OpenMenu => self.open_escape_menu(),
            ControlCommand::ReturnToMenu => {
                self.close_pause_menu_for_good();
                self.return_to_menu = true;
            }
            ControlCommand::OpenWiki => {
                let url = "https://the-bibites.fandom.com/wiki/";
                let _ = if cfg!(windows) {
                    std::process::Command::new("cmd")
                        .args(["/C", "start", "", url])
                        .spawn()
                } else {
                    std::process::Command::new("xdg-open").arg(url).spawn()
                };
            }
            ControlCommand::OpenLibrary => self.panels.open(crate::ui::PanelKind::Library),
            ControlCommand::OpenAchievements => {
                self.panels.open(crate::ui::PanelKind::Achievements);
            }
            ControlCommand::OpenTools => self.panels.open(crate::ui::PanelKind::Tools),
            ControlCommand::OpenCredits => self.panels.open(crate::ui::PanelKind::Credits),
            ControlCommand::OpenLineage => {
                // `SpeciesPanel.OpenPanel` (cs:63): needs at least 5 species-log records.
                if self.world.species.n_records() < 5 {
                    self.last_action =
                        "Not enough data collected, try again in a few minutes".to_owned();
                    self.lineage_gate_popup = true;
                    return;
                }
                // `SpeciesPanel.OpenPanel` starts with `lineageSettingsPanel.ClosePanel()`.
                self.lineage_settings_open = false;
                // ... and calls `GlobalLineageManager.UpdateEffectiveSpeciesSpan()` (cs:74): the used span takes the species that are active now,
                // and the births after the panel closes are judged by it.
                self.world.species.refresh_span();
                self.refresh_lineage();
                // `SpeciesPanel.OpenPanel`: a tree or a mesh with nothing to draw (every species left out) says the
                // same and closes again.
                if !self.lineage_view.built() {
                    self.last_action =
                        "Not enough data collected, try again in a few minutes".to_owned();
                    self.lineage_gate_popup = true;
                    return;
                }
                // `speciesTabManager.SelectSpecies(null)`.
                self.select_species_info(None);
                self.panels.open(crate::ui::PanelKind::Lineage);
            }
            ControlCommand::OpenGraphs => self.panels.open(crate::ui::PanelKind::Graphs),
            ControlCommand::OpenInformation => {
                self.panels.open(crate::ui::PanelKind::Information);
            }
            ControlCommand::SelectParent(first) => {
                // `BibiteStatsPanel.SelectParent`: `UserControl.SelectTarget(genes.parent1 / parent2)`; a parent that is gone does nothing.
                let parents =
                    if let Some(record) = self.selected.and_then(|id| self.world.bibites.get(id)) {
                        Some((record.meta.parent1, record.meta.parent2))
                    } else {
                        self.selected_egg
                            .and_then(|id| self.world.eggs.get(id))
                            .map(|egg| (egg.meta.parent1, egg.meta.parent2))
                    };
                let parent = parents.and_then(|(one, two)| if first { one } else { two });
                if let Some(parent) = parent
                    && self.world.bibites.get(parent).is_some()
                {
                    self.select_bibite(parent);
                }
            }
            ControlCommand::InformationClose => {
                self.panels.close(crate::ui::PanelKind::Information);
            }
            ControlCommand::InformationToggleMode => self.information.toggle_mode(),
            ControlCommand::InformationSelectSpecies(row) => {
                let species = self
                    .information
                    .snapshot()
                    .and_then(|snapshot| snapshot.species.get(usize::from(row)))
                    .map(|row| row.id);
                if let Some(species) = species {
                    self.select_random_of_species(species);
                    if self.ctrl_held {
                        self.reselect = Some(Reselect::Species(species));
                    }
                }
            }
            ControlCommand::InformationSelectTag(index) => {
                let tag = self
                    .information
                    .snapshot()
                    .and_then(|snapshot| snapshot.tags.get(index as usize))
                    .map(|row| row.name.clone());
                if let Some(tag) = tag {
                    self.select_random_of_tag(&tag);
                    if self.ctrl_held {
                        self.reselect = Some(Reselect::Tag);
                        self.reselect_tag = Some(tag);
                    }
                }
            }
            ControlCommand::GraphsScaleDown => {
                self.graphs_scale = self.graphs_scale.saturating_sub(1);
            }
            ControlCommand::GraphsScaleUp => {
                self.graphs_scale = (self.graphs_scale + 1).min(self.deepest_graphs_scale());
            }
            ControlCommand::OpenSpecies => self.panels.open(crate::ui::PanelKind::Species),
            ControlCommand::OpenSavePanel => self.open_save_panel(),
            ControlCommand::OpenLoadPanel => {
                self.refresh_save_entries();
                self.panels.open(crate::ui::PanelKind::LoadGame);
            }
            ControlCommand::OpenScenarioPicker => {
                self.panels.open(crate::ui::PanelKind::ScenarioPicker);
            }
            ControlCommand::OpenScenarioEditor => self.open_scenario_editor(),
            ControlCommand::EditorApply => self.apply_editor_settings(),
            ControlCommand::EditorSave => self.save_editor_scenario(),
            ControlCommand::EditorCancel => {
                self.editor_settings = None;
                self.editor_dirty = false;
                self.panels.close(crate::ui::PanelKind::ScenarioEditor);
            }
            ControlCommand::EditorAddZone => self.editor_add_zone(),
            ControlCommand::EditorRemoveZone => self.editor_remove_zone(),
            ControlCommand::OpenGeneEditor => self.open_gene_editor(),
            ControlCommand::OpenBrainEditor => self.open_brain_editor(),
            ControlCommand::EditorRandomize => self.editor_randomize_genome(),
            ControlCommand::EditorMutate => self.editor_mutate_gene(),
            ControlCommand::EditorAddNode => self.editor_add_node(),
            ControlCommand::EditorRemoveNode => self.editor_remove_node(),
            ControlCommand::EditorAddSynapse => self.editor_add_synapse(),
            ControlCommand::SpeciesToggleFavorite => self.species_toggle_favorite(),
            ControlCommand::SpeciesSelectRandomMember => self.species_select_random_member(),
            ControlCommand::EditorCycleNodeType => self.editor_cycle_node_type(),
            ControlCommand::EditorWeightUp => self.editor_edit_synapse(SynapseEdit::Weight(0.1)),
            ControlCommand::EditorWeightDown => self.editor_edit_synapse(SynapseEdit::Weight(-0.1)),
            ControlCommand::EditorToggleSynapse => self.editor_edit_synapse(SynapseEdit::Toggle),
            ControlCommand::EditorRemoveSynapse => self.editor_edit_synapse(SynapseEdit::Remove),
            ControlCommand::EditorBiasUp => self.editor_adjust_bias(1.0),
            ControlCommand::EditorBiasDown => self.editor_adjust_bias(-1.0),
            ControlCommand::EditorValidate => self.editor_validate_brain(),
            ControlCommand::EditorSaveTemplate => self.editor_save_template(),
            ControlCommand::EditorLoadTemplate => self.editor_load_template(),
            ControlCommand::EditorAnchorNode => self.editor_anchor_node(),
            ControlCommand::EditorClearAnchors => self.editor_anchors.clear(),
            // `ChallengeTrackerPanel.Retry` (cs:91-100) asks before throwing away a
            // running challenge's progress.
            ControlCommand::ChallengeRestart
                if self.challenge.as_ref().is_some_and(|session| {
                    session.state != bibites_core::world::ChallengeState::Ended
                }) =>
            {
                self.confirm_dialog = Some(ConfirmKind::RestartChallenge);
                self.last_action = "confirm: restart challenge?".to_owned();
            }
            ControlCommand::ChallengeRestart if self.challenge_setup.is_some() => {
                self.restart_challenge();
            }
            ControlCommand::StartChallenge | ControlCommand::ChallengeRestart => {
                if self.challenge_scenarios.is_empty() && self.champion_templates.is_empty() {
                    self.open_challenge_picker();
                } else {
                    self.start_local_challenge();
                }
            }
            ControlCommand::OpenChallengePicker => self.open_challenge_picker(),
            ControlCommand::OpenWorkshop => {
                self.refresh_workshop();
                self.panels.open(crate::ui::PanelKind::Workshop);
            }
            ControlCommand::WorkshopRefresh => self.refresh_workshop(),
            ControlCommand::WorkshopOnline => {
                self.last_action =
                    "Steam/online Workshop is not available in this build".to_owned();
            }
            ControlCommand::WorkshopRename => self.rename_workshop_selected(),
            ControlCommand::WorkshopExport => self.export_workshop_selected(),
            ControlCommand::WorkshopDelete => self.delete_workshop_selected(),
            ControlCommand::ChallengeSaveParams => self.save_challenge_params(),
            ControlCommand::UnsupportedControl => {
                self.last_action = "control is not available in this build".to_owned();
            }
            ControlCommand::OpenFolder => {
                let path = self.storage.scenarios_dir();
                #[cfg(not(test))]
                {
                    let _ = std::process::Command::new("explorer").arg(&path).spawn();
                }
                self.last_action = format!("opened {}", path.display());
            }
            ControlCommand::ScrollList => {
                self.list_scroll = self.list_scroll.wrapping_add(1);
                self.last_action = format!("list scroll {}", self.list_scroll);
            }
            ControlCommand::ChallengeTrackerClose => {
                self.panels.close(crate::ui::PanelKind::ChallengeTracker);
            }
            ControlCommand::ChallengeContinue => self.continue_as_scenario(),
            ControlCommand::ChallengeReturnToMenu => self.challenge_return_to_menu(),
            ControlCommand::SaveWorld => self.save_world(false),
            ControlCommand::QuickSave => self.save_world(true),
            ControlCommand::QuickLoad => self.load_quick_save(),
            ControlCommand::SpawnBibite => self.spawn_bibite_at_cursor(),
            // `BibiteStatsPanel.KillCurrentBibite` does nothing in a challenge.
            ControlCommand::KillSelected if self.challenge_is_running() => {}
            ControlCommand::KillSelected => self.kill_selected(),
            ControlCommand::RequestKill => self.request_kill(ConfirmKind::KillSelected),
            ControlCommand::RequestKillHalf => self.request_kill(ConfirmKind::KillHalf),
            ControlCommand::SelectRandomChild => self.select_random_child(),
            ControlCommand::AddDefaultBibite => {
                // `DefaultBibitesPanel.AddBibiteButtonPressed`: the template selector opens; the
                // window loop turns the Library overlay into a picker and hands the file back.
                self.template_pick = Some(TemplateUse::DefaultBibite);
                self.panels.open(crate::ui::PanelKind::Library);
            }
            ControlCommand::LoadBibite => self.begin_bibite_placer(),
            ControlCommand::ClosePlacer => self.close_placer(),
            ControlCommand::EditPlacerTag => {
                if self.placer.is_some() && self.placer_options.tagging == Tagging::CustomTagging {
                    self.tag_edit = Some(self.placer_options.tag.clone());
                    self.tag_edit_is_placer = true;
                }
            }
            ControlCommand::RequestClearDefaultBibites => {
                // The button is disabled while the list is empty.
                if !self.world.bibite_spawns.is_empty() {
                    self.confirm_dialog = Some(ConfirmKind::ClearDefaultBibites);
                }
            }
            ControlCommand::DeleteDefaultBibite(index) => {
                self.delete_default_bibite(usize::from(index));
            }
            ControlCommand::ToggleZonePreview => self.zone_preview_open = !self.zone_preview_open,
            ControlCommand::AddZone => self.add_zone(),
            ControlCommand::RequestClearZones => {
                // The button is disabled while the list is empty.
                if self.world.zones.explicit_count() > 0 {
                    self.confirm_dialog = Some(ConfirmKind::ClearZones);
                }
            }
            ControlCommand::DeleteZone(index) => {
                if usize::from(index) < self.world.zones.explicit_count() {
                    self.confirm_dialog = Some(ConfirmKind::DeleteZone(usize::from(index)));
                }
            }
            ControlCommand::AddZoneGroup => self.add_zone_group(),
            // The game wires this button to `ZonesEditorPanel.DeleteAllZoneGroups`, a method that does not exist: nothing happens there either.
            ControlCommand::ClearZoneGroups => {
                self.last_action = "Remove All of the Zone Groups list does nothing in the game".to_owned();
            }
            ControlCommand::DeleteZoneGroup(index) => self.delete_zone_group(usize::from(index)),
            ControlCommand::RerollZoneGroup(index) => self.reroll_zone_group(usize::from(index)),
            ControlCommand::AddStream => self.add_stream(),
            ControlCommand::RequestClearStreams => {
                // The button is disabled while the list is empty.
                if !self.world.streams.is_empty() {
                    self.confirm_dialog = Some(ConfirmKind::ClearStreams);
                }
            }
            ControlCommand::DeleteStream(index) => self.delete_stream(usize::from(index)),
            ControlCommand::OpenAnimatorCreator => self.open_animator_creator(),
            ControlCommand::SubmitAnimatorCreator => self.submit_animator_creator(),
            ControlCommand::CloseAnimatorCreator => self.close_animator_creator(),
            ControlCommand::RequestClearAnimators => {
                // The button is disabled while the list is empty.
                if !self.world.settings_changers.is_empty() {
                    self.confirm_dialog = Some(ConfirmKind::ClearAnimators);
                }
            }
            ControlCommand::DeleteAnimator(index) => {
                if usize::from(index) < self.world.settings_changers.len() {
                    self.confirm_dialog = Some(ConfirmKind::DeleteAnimator(usize::from(index)));
                }
            }
            ControlCommand::RemoveAnimatorBase(changer, base) => {
                self.remove_animator_base(usize::from(changer), usize::from(base));
            }
            ControlCommand::RemoveAnimatorPoint(changer, point) => {
                self.animator_remove_point(usize::from(changer), usize::from(point));
            }
            ControlCommand::RemoveTool => self.remove_selected_tool(),
            // `GUIManager.OpenBibitePanel(ExpendedBrainPanel)`.
            ControlCommand::OpenExpandedBrain => self.open_expanded_brain(),
            ControlCommand::OpenBrainHelp => {
                // `GameManager.brainHelp`.
                self.pending_url = Some(
                    "https://drive.google.com/open?id=1-6AyPj3MMdWPcam4Uq-2ofupDO9vKrZP".to_owned(),
                );
            }
            ControlCommand::OpenLineageOfSelected => {
                // `SpeciesPanel.OpenAndSelectSpecies`: opens the species panel (with its
                // "not enough data" gate) and picks the selected bibite's species.
                let species = self.selection.as_ref().and_then(|panel| panel.species_id);
                self.invoke_command(ControlCommand::OpenLineage);
                if self.panels.is_open(crate::ui::PanelKind::Lineage)
                    && let Some(species) = species
                    && let Some(index) = self
                        .lineage_species
                        .iter()
                        .position(|entry| entry.id.value() == species)
                {
                    self.select_species(index);
                }
            }
            ControlCommand::EditSpeciesTag => {
                // `BibiteStatsPanel.EditSpeciesTag` (not in a challenge).
                if !self.challenge_is_running() && self.target_is_bibite_or_egg() {
                    self.tag_edit = Some(self.target_tag().unwrap_or_default());
                }
            }
            ControlCommand::OpenEscapeMenu => {
                if !self.panels.is_open(crate::ui::PanelKind::Escape) {
                    self.open_escape_menu();
                }
            }
            ControlCommand::RequestRemoveHalfPellets => {
                self.confirm_dialog = Some(ConfirmKind::RemoveHalfPellets);
                self.last_action = "confirm: remove half of the pellets?".to_owned();
            }
            ControlCommand::Confirm => self.confirm_pending(),
            // `WarningHandle.Cancel`: nothing is remembered, whatever the checkbox said.
            ControlCommand::Cancel => {
                self.confirm_dialog = None;
                self.confirm_do_not_show = false;
            }
            ControlCommand::DismissError => {
                if !self.error_popups.is_empty() {
                    self.error_popups.remove(0);
                }
            }
            ControlCommand::ToggleDoNotShowAgain => {
                if self
                    .confirm_dialog
                    .as_ref()
                    .is_some_and(|kind| kind.warning().is_some())
                {
                    self.confirm_do_not_show = !self.confirm_do_not_show;
                }
            }
            ControlCommand::ResetWarnings => self.reset_warnings(),
            // An egg saves through the same dialog (`SaveBibiteOrEgg`).
            ControlCommand::SaveEggs => self.open_save_dialog(),
            ControlCommand::AbortEggs => {
                let ids: Vec<_> = self.world.eggs.ids().collect();
                let mut aborted = 0;
                for id in ids {
                    if self.world.abort_egg(id) {
                        aborted += 1;
                    }
                }
                self.last_action = format!("aborted {aborted} egg(s)");
                self.refresh_overlay(1.0);
            }
            ControlCommand::SetTag => {
                let tag = self.tag_draft.clone();
                self.set_selected_tag(&tag);
            }
            ControlCommand::ClearTag => {
                self.tag_draft.clear();
                self.egg_tag_draft.clear();
                self.set_selected_tag("");
            }
            ControlCommand::SaveAs => self.save_panel_save(),
            ControlCommand::LoadSelectedSave => self.load_selected_save(),
            ControlCommand::DeleteSave => self.delete_selected_save(),
            ControlCommand::SelectTool(tool) => {
                // `PelletPlacer.OpenPanel`: the target is dropped and selection stops while it is
                // open.
                if tool == crate::ui::ToolKind::Pellet {
                    self.clear_target();
                }
                self.tool = tool;
                self.last_action = format!("tool: {tool:?}");
            }
            ControlCommand::ClosePellets => self.tool = crate::ui::ToolKind::Inspect,
            ControlCommand::ApplySettings => {
                self.apply_user_preferences();
                self.persist_settings(false);
            }
            ControlCommand::ResetSettings => {
                self.prefs = bibites_io::storage::UserPreferences::default();
                self.apply_user_preferences();
                self.persist_settings(true);
            }
            ControlCommand::SelectionClose => self.multi = None,
            ControlCommand::SelectionKill => self.kill_multi_selection(),
            ControlCommand::SelectionLay => {
                if let Some(multi) = self.multi.as_ref() {
                    let laid = multi.lay_eggs(&mut self.world);
                    self.last_action = format!("laid {laid} egg(s)");
                    self.refresh_overlay(1.0);
                }
            }
            ControlCommand::SelectionRemoveKind(index) => {
                if let (Some(multi), Some(kind)) = (
                    self.multi.as_mut(),
                    crate::selection::Kind::from_index(index),
                ) {
                    multi.remove_kind(kind, &self.world);
                }
            }
            ControlCommand::SelectionRemoveSpecies(raw) => {
                if let Some(multi) = self.multi.as_mut() {
                    multi.remove_species(SpeciesId::new(raw), &self.world);
                }
            }
            // `LayEggFromCurrentBibite` / `ReviveCorpse` are disabled in a challenge.
            ControlCommand::LayEggs | ControlCommand::ReviveSelected
                if self.challenge_is_running() => {}
            ControlCommand::LayEggs => {
                if self.multi.is_some() {
                    // `UserControl.Update`: Insert lays on the multi-selection when there is one.
                    self.invoke_command(ControlCommand::SelectionLay);
                } else if let Some(id) = self.selected
                    && self.world.bibites.get(id).is_some_and(|r| !r.body.dead)
                {
                    // `GUIManager.LayEggIfBibite`: a living bibite only.
                    let laid = self.world.lay_egg_once(id);
                    self.last_action = format!("laid {laid} egg(s)");
                    self.refresh_overlay(1.0);
                }
            }
            ControlCommand::ReviveSelected => {
                if let Some(id) = self.selected {
                    let revived = self.world.revive_bibite(id);
                    self.last_action = format!("revive {revived}");
                    self.refresh_overlay(1.0);
                }
            }
            ControlCommand::SaveSelected => self.open_save_dialog(),
            ControlCommand::SaveDialogConfirm => self.confirm_save_dialog(),
            ControlCommand::SaveDialogCancel => self.close_save_dialog(),
            // `BibiteStatsPanel` EDIT opens the bibite editor on the selection (it
            // does not save a template; SAVE does that).
            ControlCommand::EditSelected => {
                if self.selected.is_some() {
                    self.open_template_editor_for_selection();
                } else {
                    self.last_action = "edit: no selection".to_owned();
                }
            }
            ControlCommand::Quit => self.quit = true,
        }
    }

    /// World units per screen pixel at the current zoom.
    fn world_units_per_pixel(&self) -> f32 {
        let height = self.camera.screen.y.max(1.0);
        crate::render::zoom_to_ortho(self.camera.zoom, height) / (height / 2.0)
    }

    /// `RulerHandle.UpdateSize` (cs:68-90): keeps the bar between 100 and 200
    /// pixels by stepping its world length one decade-aware step per update.
    fn update_ruler(&mut self) {
        let per_pixel = self.world_units_per_pixel();
        if !per_pixel.is_finite() || per_pixel <= 0.0 {
            return;
        }
        let width_px = self.ruler_target / per_pixel;
        let leading = (f64::from(self.ruler_target)
            / 10.0_f64.powi(f64::from(self.ruler_target).log10().floor() as i32))
            as i32;
        if width_px > 200.0 {
            if leading <= 1 {
                self.ruler_scale /= 10.0;
            }
            self.ruler_target -= self.ruler_scale;
        } else if width_px < 100.0 {
            self.ruler_target += self.ruler_scale;
            if leading >= 9 {
                self.ruler_scale *= 10.0;
            }
        }
        self.ruler_target = self.ruler_target.max(1e-6);
    }

    /// Ruler bar length in screen pixels.
    #[must_use]
    pub fn ruler_pixels(&self) -> f32 {
        let per_pixel = self.world_units_per_pixel();
        if per_pixel > 0.0 && per_pixel.is_finite() {
            (self.ruler_target / per_pixel).clamp(0.0, 4_000.0)
        } else {
            0.0
        }
    }

    fn next_ui_random(&mut self) -> u64 {
        let mut x = self.ui_rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.ui_rng = x;
        x
    }

    /// `UserControl.SelectRandomBibite`.
    fn select_random_bibite(&mut self) {
        let ids = self.world.live_bibite_ids();
        if ids.is_empty() {
            return;
        }
        let pick = ids[(self.next_ui_random() % ids.len() as u64) as usize];
        self.select_bibite_target(pick);
    }

    /// `UserControl.SelectRandomBibiteOfSpecies`: a random living member of the species, an egg of it when
    /// it has none.
    fn select_random_of_species(&mut self, species: SpeciesId) {
        let members: Vec<BibiteId> = self
            .world
            .bibites
            .iter()
            .filter(|(_, record)| !record.body.dead && record.meta.species == Some(species))
            .map(|(id, _)| id)
            .collect();
        if !members.is_empty() {
            let pick = members[(self.next_ui_random() % members.len() as u64) as usize];
            self.select_bibite_target(pick);
            return;
        }
        let eggs: Vec<bibites_core::ids::EggId> = self
            .world
            .eggs
            .ids()
            .filter(|id| {
                self.world
                    .eggs
                    .get(*id)
                    .is_some_and(|egg| egg.meta.species == Some(species))
            })
            .collect();
        if !eggs.is_empty() {
            let pick = eggs[(self.next_ui_random() % eggs.len() as u64) as usize];
            self.select_egg(pick);
        }
    }

    /// `UserControl.SelectRandomBibiteOfTag`: a random living bibite or egg carrying the tag (`"Untagged"`
    /// meaning none).
    fn select_random_of_tag(&mut self, tag: &str) {
        let wanted = if tag == crate::information_panel::UNTAGGED {
            ""
        } else {
            tag
        };
        let bibites: Vec<BibiteId> = self
            .world
            .bibites
            .iter()
            .filter(|(_, record)| {
                !record.body.dead && record.meta.species_tag.as_deref().unwrap_or("") == wanted
            })
            .map(|(id, _)| id)
            .collect();
        let eggs: Vec<bibites_core::ids::EggId> = self
            .world
            .eggs
            .ids()
            .filter(|id| {
                self.world
                    .eggs
                    .get(*id)
                    .is_some_and(|egg| egg.meta.species_tag.as_deref().unwrap_or("") == wanted)
            })
            .collect();
        let total = bibites.len() + eggs.len();
        if total == 0 {
            return;
        }
        let pick = (self.next_ui_random() % total as u64) as usize;
        if let Some(id) = bibites.get(pick) {
            self.select_bibite_target(*id);
        } else {
            self.select_egg(eggs[pick - bibites.len()]);
        }
    }

    /// `UserControl.SelectHighGenBibite` (needs a living bibite).
    fn select_highest_generation(&mut self) {
        if !self.world.live_bibite_ids().is_empty() {
            self.select_by_key(|record| record.meta.generation as f32);
        }
    }

    /// `UserControl.SelectOldestBibite`.
    fn select_oldest(&mut self) {
        self.select_by_key(|record| record.clock.time_alive);
    }

    /// Selects the live bibite with the largest `key` (`SelectHighGenBibite`,
    /// `SelectOldestBibite`).
    fn select_by_key(&mut self, key: impl Fn(&bibites_core::body::BibiteRecord) -> f32) {
        let mut best: Option<(f32, BibiteId)> = None;
        for (id, record) in self.world.bibites.iter() {
            if record.body.dead {
                continue;
            }
            let value = key(record);
            if best.is_none_or(|(top, _)| value > top) {
                best = Some((value, id));
            }
        }
        if let Some((_, id)) = best {
            self.select_bibite_target(id);
        }
    }

    /// `UserControl.SelectRandomEgg`.
    fn select_random_egg(&mut self) {
        let ids: Vec<_> = self
            .world
            .eggs
            .ids()
            .filter(|id| {
                self.world
                    .eggs
                    .get(*id)
                    .is_some_and(|egg| !egg.hatch.aborting)
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let pick = ids[(self.next_ui_random() % ids.len() as u64) as usize];
        self.select_egg(pick);
    }

    /// `UserControl.Update` (K): `SelectTarget(autoPanner)`: the camera follows the auto-panner (and
    /// Escape deselects it). Selecting it again does nothing.
    fn select_autopanner(&mut self) {
        if self.selected_autopan {
            return;
        }
        self.clear_target();
        self.selected_autopan = true;
        self.follow_selected = true;
        self.last_action = "auto-panner selected".to_owned();
    }

    /// A number of the auto-panner's own random stream in `[0, 1)` (`UnityEngine.Random.value`).
    fn autopan_roll(&mut self) -> f32 {
        let mut x = self.autopan_rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.autopan_rng = x;
        ((x >> 40) as f32) / ((1_u64 << 24) as f32)
    }

    /// `ZoneManager.GetRandomPositionInZone`: a random zone's spawn location (the origin without
    /// zones). The location is drawn on a copy of the zone with a stream of its own, so the
    /// simulation's random numbers are not touched.
    fn random_zone_position(&mut self) -> Vec2 {
        let count = self.world.zones.zones().len();
        if count == 0 {
            return Vec2::ZERO;
        }
        let index = ((self.autopan_roll() * count as f32) as usize).min(count - 1);
        let seed = (u64::from((self.autopan_roll() * 16_777_216.0) as u32) << 32)
            | u64::from((self.autopan_roll() * 16_777_216.0) as u32);
        let zone = &self.world.zones.zones()[index];
        let mut probe = bibites_core::zone::ZoneData::new(
            zone.config.clone(),
            index,
            0,
            &self.world.bounds,
            &self.world.zones.globals,
            seed,
        );
        probe.pos = zone.pos;
        probe.spawn_location()
    }

    /// One physics step of the auto-panner (`AutoPanner.FixedUpdate` and its coroutine): it only
    /// moves while the simulation runs.
    fn step_autopan(&mut self, dt: f32) {
        let needs = self.autopan.step(dt);
        if needs.destination {
            self.autopan.dest = self.random_zone_position();
        }
        if needs.wait {
            let span = crate::autopanner::MAX_WAIT - crate::autopanner::MIN_WAIT;
            self.autopan.wait = crate::autopanner::MIN_WAIT + span * self.autopan_roll();
        }
    }

    /// `UserControl.Update` (Z): `SelectTarget(ZoneManager.instance.zones.RandomElement())`.
    fn select_random_zone(&mut self) {
        let count = self.world.zones.zones.len();
        if count == 0 {
            return;
        }
        let pick = (self.next_ui_random() % count as u64) as usize;
        self.select_zone(pick);
    }

    /// `UserControl.SelectTarget(zone)` and `GUIManager.OnTargetChanged`: the zone becomes the
    /// target (the previous one goes; Escape deselects it) and, with `OpenZoneEditorOnSelect`, the
    /// panels close and the right-hand Dynamic panel opens on its Zones tab with this zone's item
    /// open (`ZonesEditorPanel.SelectItemOfZone`, which opens the item and leaves the others).
    /// Selecting the zone that already is the target does nothing.
    pub fn select_zone(&mut self, index: usize) {
        if self.selected_zone == Some(index) || index >= self.world.zones.zones.len() {
            return;
        }
        self.clear_target();
        self.selected_zone = Some(index);
        self.follow_selected = true;
        // The target's Escape entry goes under the panel's (`SelectTarget` pushes its action before
        // `onTargetChange` opens the panel).
        self.sync_escape_stack();
        if self.open_zone_editor_on_select {
            self.inspector_open = false;
            // (The Zones list holds the explicit zones: a zone a group generated has no item to open.)
            let item = (0, index as u16);
            if index < self.world.zones.explicit_count() && !self.dyn_panel.open_items.contains(&item) {
                self.dyn_panel.open_items.push(item);
            }
            if self.dyn_panel.tab != 1 {
                self.dyn_panel.tab = 1;
                self.dyn_panel.scroll = 0.0;
            }
            self.panels.open(crate::ui::PanelKind::WorldDynamics);
        }
        self.last_action = format!("zone {index} selected");
    }

    /// A different world replaced the old one: the Zones tab starts again (the game refills the
    /// panel with every item closed) and no zone is the target.
    fn forget_zone_editor(&mut self) {
        self.selected_zone = None;
        self.selected_autopan = false;
        self.dyn_panel.open_items.clear();
        self.dyn_panel.zone_drag = None;
        self.dyn_panel.bibite_drag = None;
        self.dyn_panel.material_drag = None;
        self.zone_name_edit = None;
        self.animator.creator.open = false;
        self.animator.point = None;
        self.animator.drag = None;
        self.value_edit = None;
    }

    /// The live zones' settings, in list order (what the Zones tab lists and edits).
    fn zone_settings_list(&self) -> Vec<bibites_core::settings::ZoneSettings> {
        self.world
            .zones
            .zones
            .iter()
            .map(|zone| zone.config.0.clone())
            .collect()
    }

    /// A control of zone `index` in the Zones tab changed a setting: `edit` changes a copy of the
    /// zone's settings (it also gets every zone's settings, for the target names) and the zone
    /// takes the result at once, as the game's `ZoneSettings` do through their change events.
    fn edit_zone(
        &mut self,
        index: usize,
        edit: impl FnOnce(
            &mut bibites_core::settings::ZoneSettings,
            &[bibites_core::settings::ZoneSettings],
        ),
    ) {
        // The template of a zone group edits the group (its zones follow, `ApplyChangesToAllZones`).
        if let Ok(reference) = u16::try_from(index) {
            match crate::zone_editor::zone_ref(reference) {
                crate::zone_editor::ZoneRef::Template(group) => {
                    let zones = self.zone_settings_list();
                    self.edit_group(group, |group| edit(&mut group.template, &zones));
                    return;
                }
                crate::zone_editor::ZoneRef::Group(_)
                | crate::zone_editor::ZoneRef::Animator(_)
                | crate::zone_editor::ZoneRef::Creator => return,
                crate::zone_editor::ZoneRef::Zone(_) => {}
            }
        }
        let Some(id) = self.world.zones.zone_id_at(index) else {
            return;
        };
        let zones = self.zone_settings_list();
        let before = zones[index].clone();
        let mut item = before.clone();
        edit(&mut item, &zones);
        match self.world.apply_zone_settings(id, item) {
            Ok(()) => {
                self.refresh_overlay(0.0);
                // What the world kept (a position or a target it capped) is what the history holds.
                if let Some(after) = self.zone_settings_list().get(index).cloned() {
                    self.record_zone_edit(before, after);
                }
            }
            Err(error) => self.last_action = format!("zone setting rejected: {error}"),
        }
    }

    /// The settings a zone index of an action addresses: a zone of the world, or a zone group's template.
    fn zone_like(&self, index: u16) -> Option<bibites_core::settings::ZoneSettings> {
        match crate::zone_editor::zone_ref(index) {
            crate::zone_editor::ZoneRef::Zone(zone) => {
                self.world.zones.zones.get(zone).map(|zone| zone.config.0.clone())
            }
            crate::zone_editor::ZoneRef::Template(group) => {
                self.world.zone_groups.get(group).map(|group| group.template.clone())
            }
            crate::zone_editor::ZoneRef::Group(_)
            | crate::zone_editor::ZoneRef::Animator(_)
            | crate::zone_editor::ZoneRef::Creator => None,
        }
    }

    /// Something about zone group `group` changed: the world follows as the game's group handle makes it (the zones are renamed, generated
    /// again when their number changed, or take the template's new settings).
    fn edit_group(&mut self, group: usize, edit: impl FnOnce(&mut bibites_core::settings::ZoneGroupSettings)) {
        match self.world.edit_zone_group(group, edit) {
            Ok(Some(mapping)) => self.remap_zone_state(&mapping),
            Ok(None) => {}
            Err(error) => self.last_action = format!("zone group setting rejected: {error}"),
        }
        self.refresh_overlay(0.0);
    }

    /// The list of zones changed: `mapping` says where each zone of the old list is now. The selected zone follows (the target ends when its zone
    /// went) and what was typed or recorded about zones by their place is dropped.
    fn remap_zone_state(&mut self, mapping: &[Option<usize>]) {
        self.drop_zone_history();
        if self.zone_name_edit.as_ref().is_some_and(|(index, _)| *index < usize::from(crate::zone_editor::TEMPLATE_BASE)) {
            self.zone_name_edit = None;
        }
        if let Some(selected) = self.selected_zone {
            match mapping.get(selected).copied().flatten() {
                Some(now) => self.selected_zone = Some(now),
                None => self.clear_target(),
            }
        }
    }

    /// `ZonesEditorPanel.AddZoneGroup`: a group "Pockets<n>" (n: the groups there are) with the game's defaults generates its zones and its card
    /// opens.
    fn add_zone_group(&mut self) {
        let mut group = bibites_core::settings::ZoneGroupSettings::game_defaults();
        group.name = format!("Pockets{}", self.world.zone_groups.len());
        match self.world.add_zone_group(group) {
            Ok(index) => {
                self.dyn_panel.open_items.push((1, index as u16));
                self.last_action = format!("added zone group {index}");
                self.refresh_overlay(0.0);
            }
            Err(error) => self.last_action = format!("zone group not added: {error}"),
        }
    }

    /// `ZonesEditorPanel.GroupDeleted`: the group and its zones go at once (no question is asked).
    fn delete_zone_group(&mut self, index: usize) {
        match self.world.remove_zone_group(index) {
            Ok(mapping) => {
                let at = index as u16;
                self.dyn_panel.open_items.retain(|item| *item != (1, at));
                for item in &mut self.dyn_panel.open_items {
                    if item.0 == 1 && item.1 > at {
                        item.1 -= 1;
                    }
                }
                self.dyn_panel.card_removed(1, index);
                self.remap_zone_state(&mapping);
                self.last_action = format!("deleted zone group {index}");
                self.refresh_overlay(0.0);
            }
            Err(error) => self.last_action = format!("zone group not deleted: {error}"),
        }
    }

    /// `ZoneGroupHandle.GenerateZones` (the reroll button): the group's zones are generated again.
    fn reroll_zone_group(&mut self, index: usize) {
        match self.world.generate_zone_group(index) {
            Ok(mapping) => {
                self.remap_zone_state(&mapping);
                self.last_action = format!("rolled zone group {index} again");
                self.refresh_overlay(0.0);
            }
            Err(error) => self.last_action = format!("zone group not rolled: {error}"),
        }
    }

    /// A control of material block `kind` (0 Plant, 1 Meat, 2 Fat, 3 Armor) of the Energy tab
    /// changed a parameter: the world takes the new table at once (`MatterMaterialManager`).
    fn edit_material(
        &mut self,
        kind: u8,
        edit: impl FnOnce(
            &mut bibites_core::settings::MaterialSetting,
            bibites_core::settings::MaterialKind,
        ),
    ) {
        let material = crate::zone_editor::material_kind(kind);
        // The material table is part of the settings: the edit joins the Ctrl + Z history.
        self.dyn_edit("material", |settings| {
            edit(settings.materials.get_mut(material), material);
            true
        });
        self.refresh_overlay(0.0);
    }

    /// A material slider of the Energy tab follows the cursor.
    pub fn material_slide(&mut self, kind: u8, field: u8, track: (f32, f32), cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        if span <= 0.0 {
            return;
        }
        let fraction = (cursor_x / scale - track.0) / span;
        self.edit_material(kind, |slot, material| {
            if let Some(info) = crate::zone_editor::material_slider_info(material, field) {
                let value = crate::dyn_panel::value_range(
                    info.mode,
                    info.min,
                    info.max,
                    info.default,
                    fraction,
                );
                crate::zone_editor::set_material_value(slot, material, field, value);
            }
        });
    }

    /// Feeds the pointer to the bar rows of the Biology panel (`OnHoverShow`, 0.3 s each way).
    pub fn update_bar_hover(&mut self, screen: Vec2, dt: f32) {
        let s = crate::ui::UiFrame::scaler(screen.x, screen.y);
        let reference = Vec2::new(self.cursor.x / s, self.cursor.y / s);
        let active = self.inspector_open
            && self.inspector_tab == InspectorTab::Biology
            && self.selection.as_ref().is_some_and(|panel| panel.organs.is_some())
            && self.confirm_dialog.is_none();
        let dead = self.selection.as_ref().is_some_and(|panel| panel.dead);
        self.bar_hover.update(reference, dead, active, dt);
    }

    /// Feeds the pointer to the tooltips: a captured control with a `TooltipTrigger` under it, or
    /// else (with no UI under it) a pellet that no body covers (`PelletTooltipTrigger.OnMouseEnter`:
    /// the mouse-over goes to the topmost collider). A dialog's screen blocker hides both.
    pub fn update_tooltip(
        &mut self,
        frame: &crate::ui::UiFrame,
        cursor: Vec2,
        over_ui: bool,
        dt: f32,
    ) {
        use crate::tooltip::Target;
        let blocked = self.confirm_dialog.is_some() || self.save_dialog.is_some() || self.scenario_saver.is_some();
        let control = frame.tips.iter().rev().find(|rect| rect.contains(cursor));
        // A live tooltip (the lineage mesh's: its path starts with '@') belongs to the rectangle its shape registered as
        // a hit: with a panel drawn over it, the pointer is not on the shape and the tooltip does not show.
        let control = control.filter(|rect| {
            !rect.tip.path.starts_with('@')
                || frame.widget_at(cursor).is_some_and(|id| {
                    (crate::lineage_graph::HIT_PART..crate::lineage_graph::HIT_PART + 9_999)
                        .contains(&id)
                })
        });
        let target = match control {
            // `isUITooltip` false: `TooltipTrigger.Update` hides the box while the pointer is over
            // the UI, so it never shows on a UI element.
            Some(rect) if !blocked => rect.tip.ui.then_some(Target::Control(rect.tip)),
            Some(_) => None,
            None if !over_ui && !blocked && self.scene_options.pellets && self.scene_options.objects => self
                .pellet_target_at(self.camera.screen_to_world(cursor))
                .map(Target::Pellet),
            None => None,
        };
        self.tooltip.update(target, dt);
    }

    /// The pellet the pointer rests on at `point` (`PelletTooltipTrigger`). With the world on the simulation thread the
    /// thread answers, for the point the window asked about last: an answer for another point (the pointer or the camera
    /// has moved since) says nothing.
    fn pellet_target_at(&self, point: Vec2) -> Option<bibites_core::ids::PelletId> {
        if !self.world_away {
            return crate::tooltip::pellet_tooltip_target(&self.world, point);
        }
        let hover = self.snapshot.as_ref()?.hover.as_ref()?;
        if (hover.point - point).length_sq() > 1e-6 {
            return None;
        }
        hover.pellet.as_ref().map(|(id, _)| *id)
    }

    /// The text of the box of the pellet `id` (from the thread's answer while the world is away).
    #[must_use]
    pub fn pellet_tooltip_body(
        &self,
        id: bibites_core::ids::PelletId,
    ) -> Option<crate::tooltip::Body> {
        if !self.world_away {
            return crate::tooltip::pellet_body(&self.world, id);
        }
        let hover = self.snapshot.as_ref()?.hover.as_ref()?;
        hover
            .pellet
            .as_ref()
            .filter(|(shown, _)| *shown == id)
            .map(|(_, body)| body.clone())
    }

    /// The caret keys of this frame (the arrows extend the selection while Shift is held).
    pub(crate) fn edit_keys(&self, input: &FrameInput) -> crate::text_field::EditKeys {
        crate::text_field::EditKeys {
            backspace: input.backspace,
            delete: input.key_delete,
            left: input.key_left,
            right: input.key_right,
            home: input.key_home,
            end: input.key_end,
            select_all: input.select_all_key,
            shift: self.shift_held,
        }
    }

    /// Typing into a zone's name field: characters and Backspace edit it, Ctrl + V pastes, Enter
    /// submits (`TextLineHandle.FieldSubmit`: the zone is renamed at once).
    fn edit_zone_name(&mut self, input: &FrameInput) {
        if self.zone_name_edit.is_none() {
            return;
        }
        let mut typed = std::mem::take(&mut self.typed);
        if input.paste_tag {
            typed.push_str(&self.clipboard_in.take().unwrap_or_default());
        }
        let keys = self.edit_keys(input);
        if self.edit_caret_owner != 1 {
            self.edit_caret_owner = 1;
            self.edit_caret = crate::text_field::Caret::at_end(self.zone_name_edit.as_ref().map_or("", |(_, text)| text));
        }
        let Some((_, text)) = self.zone_name_edit.as_mut() else {
            return;
        };
        crate::text_field::edit(text, &mut self.edit_caret, &typed, TAG_MAX_CHARS, keys);
        if input.enter {
            let Some((index, name)) = self.zone_name_edit.take() else {
                return;
            };
            if let Ok(reference) = u16::try_from(index)
                && let crate::zone_editor::ZoneRef::Animator(changer) = crate::zone_editor::zone_ref(reference)
            {
                let _ = self.world.edit_settings_changer(changer, |changer| changer.name = name.clone());
            } else if let Ok(reference) = u16::try_from(index)
                && let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(reference)
            {
                self.edit_group(group, |group| group.name = name.clone());
            } else {
                self.edit_zone(index, |zone, _| zone.name = name.clone());
            }
        }
    }

    /// What typing a value into slider row `field` follows (the setting's `factor`, ends and kind) and
    /// the value it holds now; `None` when the row has no slider any more.
    pub(crate) fn typed_rules(
        &self,
        field: crate::dyn_panel::FieldRef,
    ) -> Option<(crate::value_edit::Rules, f32)> {
        use crate::dyn_panel::{FieldRef, info_rules, setting_rules};
        match field {
            FieldRef::Setting(index) => {
                let row = crate::setting_ui_data::SETTING_UI.get(usize::from(index))?;
                let (mode, _) = crate::dyn_panel::slider_info_or_default(row);
                Some((
                    setting_rules(row, mode),
                    crate::dyn_panel::read_value(&self.world.settings, row),
                ))
            }
            FieldRef::Zone(index, field) => {
                let size = self.world.settings.independents.simulation_size;
                if let Some(stream) = crate::zone_editor::stream_index(index) {
                    return self.stream_typed_rules(stream, field);
                }
                if matches!(
                    crate::zone_editor::zone_ref(index),
                    crate::zone_editor::ZoneRef::Animator(_) | crate::zone_editor::ZoneRef::Creator
                ) {
                    let (info, value) = self.animator_slider(index, field)?;
                    return Some((info_rules(&info, false), value));
                }
                if let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(index) {
                    let group = self.world.zone_groups.get(group)?;
                    let info = crate::zone_editor::group_slider_info(field)?;
                    return Some((
                        info_rules(&info, field == crate::zone_editor::GROUP_COUNT),
                        crate::zone_editor::group_value_of(group, field),
                    ));
                }
                let zone = self.zone_like(index)?;
                let info = crate::zone_editor::slider_info(&zone, field, size)?;
                let mut rules = info_rules(&info, false);
                // `canGoOutOfBounds == false` for the position, the radius, the inside radius, the width and the height; the fertility, the
                // biomass density, the pellet size and the speed factor keep what is typed past their slider's ends.
                rules.out_of_bounds = matches!(
                    field,
                    crate::zone_editor::FERTILITY
                        | crate::zone_editor::BIOMASS
                        | crate::zone_editor::PELLET_SIZE
                        | crate::zone_editor::SPEED
                );
                Some((rules, crate::zone_editor::value_of(&zone, field)))
            }
            FieldRef::Material(kind, field) => {
                let material = crate::zone_editor::material_kind(kind);
                let info = crate::zone_editor::material_slider_info(material, field)?;
                let current = *self.world.settings.materials.get(material);
                Some((
                    info_rules(&info, false),
                    crate::zone_editor::material_value(&current, field),
                ))
            }
            FieldRef::Bibite(index, field) => {
                let row = self.world.bibite_spawns.get(usize::from(index))?;
                let info = crate::bibite_editor::slider_info(field)?;
                Some((
                    info_rules(&info, field == crate::bibite_editor::MINIMUM),
                    crate::bibite_editor::value_of(row, field),
                ))
            }
            // A gene slider never goes out of its range (`canGoOutOfBounds` is off for the gene settings).
            FieldRef::Gene(gene) => {
                let session = self.bibite_editor.as_ref()?;
                let gene = usize::from(gene).min(bibites_core::genome::N_GENES - 1);
                let def = &bibites_core::genome::GENE_DEFS[gene];
                let info = crate::gene_analysis::gene_info(gene);
                Some((
                    crate::value_edit::Rules {
                        factor: info.factor,
                        min: def.min,
                        max: def.max,
                        out_of_bounds: false,
                        integer: false,
                        log: false,
                    },
                    session.model.genes()[gene],
                ))
            }
            // The Brain tab's sliders: the open node's bias or value, the open connection's weight.
            FieldRef::BrainBias | FieldRef::BrainAct | FieldRef::BrainWeight => {
                self.editor_brain_typed_rules(field)
            }
            // A parameter of the Expanded Brain View.
            FieldRef::Expanded(_) => self.expanded_typed_rules(field),
            // `TimeController.targetTimeScale`: 0.1 to 100, `canGoOutOfBounds`, on a log slider (base 5) that has its own `SubmitEditValue`.
            FieldRef::TimeSpeed => Some((
                crate::value_edit::Rules {
                    factor: 1.0,
                    min: MIN_SPEED,
                    max: MAX_SPEED,
                    out_of_bounds: true,
                    integer: false,
                    log: true,
                },
                self.speed,
            )),
        }
    }

    /// `SettingSlider.OpenEditPanel`: the value text of a slider row becomes an edit field holding
    /// the value times the factor, all selected.
    fn open_value_edit(&mut self, field: crate::dyn_panel::FieldRef) {
        use crate::value_edit::{Content, ValueEdit};
        let Some((rules, current)) = self.typed_rules(field) else {
            return;
        };
        // The press took the focus off the other text fields.
        self.zone_name_edit = None;
        let content = if rules.integer {
            Content::Integer
        } else {
            Content::Decimal
        };
        self.value_edit = Some(ValueEdit::open(field, current * rules.factor, content));
    }

    /// Typing into the edit field: characters, Backspace and Ctrl + V edit it, Enter applies the
    /// number (`SubmitEditValue`); Escape and a press elsewhere close it without applying.
    fn edit_value(&mut self, input: &FrameInput) {
        let Some(edit) = self.value_edit.as_mut() else {
            return;
        };
        edit.idle += input.dt;
        let typed = std::mem::take(&mut self.typed);
        for character in typed.chars() {
            edit.type_char(character);
        }
        if input.paste_tag
            && let Some(text) = self.clipboard_in.take()
        {
            edit.paste(&text);
        }
        if input.backspace {
            edit.backspace();
        }
        {
            use crate::value_edit::CaretKey;
            if input.key_delete {
                edit.delete_forward();
            }
            for (pressed, key) in [
                (input.key_left, CaretKey::Left),
                (input.key_right, CaretKey::Right),
                (input.key_home, CaretKey::Home),
                (input.key_end, CaretKey::End),
            ] {
                if pressed {
                    edit.move_caret(key, self.shift_held);
                }
            }
            if input.select_all_key {
                edit.move_caret(CaretKey::SelectAll, false);
            }
        }
        if input.enter {
            self.submit_value_edit();
        }
    }

    /// Enter in the edit field: the setting takes the typed number and the field closes. A text
    /// that does not parse changes nothing (the C# logs it). Only a `LogSettingSlider` puts the
    /// change on the Ctrl + Z history.
    fn submit_value_edit(&mut self) {
        let Some(edit) = self.value_edit.take() else {
            return;
        };
        let Some((rules, _)) = self.typed_rules(edit.field) else {
            return;
        };
        let Some(value) = rules.submitted(&edit.text) else {
            return;
        };
        self.suppress_history = !rules.log;
        self.apply_typed_value(edit.field, value);
        self.suppress_history = false;
    }

    /// Writes a typed value (already divided by the factor) into the row's setting as it is.
    fn apply_typed_value(&mut self, field: crate::dyn_panel::FieldRef, value: f32) {
        use crate::dyn_panel::FieldRef;
        match field {
            FieldRef::Setting(index) => {
                let index = usize::from(index);
                if self.setting_is_animated(index) {
                    return;
                }
                let label = crate::setting_ui_data::SETTING_UI
                    .get(index)
                    .map_or("setting", |row| row.name);
                self.dyn_edit(label, |settings| {
                    crate::dyn_panel::set_value_balanced(settings, index, value)
                });
            }
            FieldRef::Zone(index, field) => {
                let size = self.world.settings.independents.simulation_size;
                if let Some(stream) = crate::zone_editor::stream_index(index) {
                    self.stream_set(stream, field, value);
                    return;
                }
                if matches!(
                    crate::zone_editor::zone_ref(index),
                    crate::zone_editor::ZoneRef::Animator(_) | crate::zone_editor::ZoneRef::Creator
                ) {
                    self.animator_set_value(index, field, value, false);
                    return;
                }
                if let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(index) {
                    self.edit_group(group, |group| {
                        crate::zone_editor::set_group_value_unclamped(group, field, value);
                    });
                    return;
                }
                self.edit_zone(usize::from(index), |zone, _| {
                    crate::zone_editor::set_value_unclamped(zone, field, value, size);
                });
            }
            FieldRef::Material(kind, field) => {
                self.edit_material(kind, |slot, _| {
                    crate::zone_editor::set_material_value_unclamped(slot, field, value);
                });
            }
            FieldRef::Bibite(index, field) => {
                if let Some(row) = self.world.bibite_spawns.get_mut(usize::from(index)) {
                    crate::bibite_editor::set_value_unclamped(row, field, value);
                }
            }
            FieldRef::Gene(gene) => {
                if let Some(session) = self.bibite_editor.as_mut() {
                    session.model.set_gene(
                        usize::from(gene).min(bibites_core::genome::N_GENES - 1),
                        value,
                    );
                }
            }
            FieldRef::BrainBias | FieldRef::BrainAct | FieldRef::BrainWeight => {
                self.editor_brain_apply_value(field, value);
            }
            FieldRef::Expanded(_) => self.expanded_apply_value(field, value),
            // `LogSettingSlider.SubmitEditValue` sets the target time scale: any positive value forces play (`TimeController.SetTarget`), and a zero is a pause
            // (`engineTimeScale` 0: `Time.timeScale = 0`). The game keeps what is typed past the slider's ends; Unity itself holds the time scale to 100.
            FieldRef::TimeSpeed => {
                if value > 0.0 {
                    self.set_target_speed(value);
                    self.paused = false;
                    self.accumulator = 0.0;
                } else {
                    self.paused = true;
                }
            }
        }
    }

    /// A zone slider of the Zones tab follows the cursor (`SettingSlider.SetValueFromSlider`).
    pub fn zone_slide(&mut self, index: usize, field: u8, track: (f32, f32), cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        if span <= 0.0 {
            return;
        }
        let fraction = (cursor_x / scale - track.0) / span;
        let size = self.world.settings.independents.simulation_size;
        if let Ok(reference) = u16::try_from(index)
            && let Some(stream) = crate::zone_editor::stream_index(reference)
        {
            self.stream_slide(stream, field, fraction);
            return;
        }
        if let Ok(reference) = u16::try_from(index)
            && matches!(
                crate::zone_editor::zone_ref(reference),
                crate::zone_editor::ZoneRef::Animator(_) | crate::zone_editor::ZoneRef::Creator
            )
        {
            self.animator_slide(reference, field, track, cursor_x);
            return;
        }
        if let Ok(reference) = u16::try_from(index)
            && let crate::zone_editor::ZoneRef::Group(group) = crate::zone_editor::zone_ref(reference)
        {
            if let Some(info) = crate::zone_editor::group_slider_info(field) {
                let value = crate::dyn_panel::value_range(info.mode, info.min, info.max, info.default, fraction);
                self.edit_group(group, |group| crate::zone_editor::set_group_value(group, field, value));
            }
            return;
        }
        self.edit_zone(index, |zone, _| {
            if let Some(info) = crate::zone_editor::slider_info(zone, field, size) {
                let value = crate::dyn_panel::value_range(
                    info.mode,
                    info.min,
                    info.max,
                    info.default,
                    fraction,
                );
                crate::zone_editor::set_value(zone, field, value, size);
            }
        });
    }

    /// `ZonesEditorPanel.AddZone`: a Plant zone with the game's defaults named "Zone n" (n = the
    /// number of zones before) joins the world and its item opens.
    fn add_zone(&mut self) {
        use bibites_core::settings::{MaterialKind, ZoneSettings};
        let mut zone = ZoneSettings::game_defaults();
        zone.material = Some(MaterialKind::Plant);
        // (`Zone {zoneHandles.activeCount}`: the zones of the list, not the groups' ones.)
        zone.name = format!("Zone {}", self.world.zones.explicit_count());
        match self.world.add_zone(zone) {
            Ok(_) => {
                let index = self.world.zones.explicit_count() - 1;
                self.dyn_panel.open_items.push((0, index as u16));
                self.last_action = format!("added zone {index}");
                self.refresh_overlay(0.0);
            }
            Err(error) => self.last_action = format!("zone not added: {error}"),
        }
    }

    /// `ZonesEditorPanel.ZoneDeleted`: zone `index` leaves the world (its pellets go with it).
    /// Zones after it move up one place; when it was the target the target ends
    /// (`TargetableObject.OnDestroy`).
    fn delete_zone(&mut self, index: usize) {
        let Some(id) = self.world.zones.zone_id_at(index) else {
            return;
        };
        if self.world.remove_zone(id).is_err() {
            return;
        }
        self.drop_zone_history();
        self.zone_name_edit = None;
        let at = index as u16;
        self.dyn_panel.open_items.retain(|item| *item != (0, at));
        for item in &mut self.dyn_panel.open_items {
            if item.0 == 0 && item.1 > at {
                item.1 -= 1;
            }
        }
        self.dyn_panel.card_removed(0, index);
        match self.selected_zone {
            Some(selected) if selected == index => self.clear_target(),
            Some(selected) if selected > index => self.selected_zone = Some(selected - 1),
            _ => {}
        }
        self.last_action = format!("deleted zone {index}");
        self.refresh_overlay(0.0);
    }

    /// `ZonesEditorPanel.ActuallyDeleteAllZones`: every zone leaves the world.
    fn clear_zones(&mut self) {
        // From the last one, so the others are not renumbered over and over. The zones of the groups stay (`ScenarioSettings.RemoveAllZones`
        // clears `zones` only).
        while let Some(last) = self.world.zones.explicit_count().checked_sub(1) {
            let Some(id) = self.world.zones.zone_id_at(last) else {
                break;
            };
            if self.world.remove_zone(id).is_err() {
                break;
            }
        }
        self.dyn_panel.open_items.retain(|item| item.0 != 0);
        self.dyn_panel.cards_cleared(0);
        self.drop_zone_history();
        self.zone_name_edit = None;
        if self.selected_zone.is_some() {
            self.clear_target();
        }
        self.last_action = "removed all zones".to_owned();
        self.refresh_overlay(0.0);
    }

    /// Feeds the pointer to the toolbar buttons' `OnHoverShow` (`zones` are the screen rectangles of
    /// each button together with its open panel).
    pub fn update_toolbar_hover(
        &mut self,
        zones: &[Option<(Vec2, Vec2)>; 3],
        cursor: Vec2,
        dt: f32,
    ) {
        for (panel, zone) in self.toolbar.iter_mut().zip(zones) {
            let over = zone.is_some_and(|(pos, size)| {
                cursor.x >= pos.x
                    && cursor.y >= pos.y
                    && cursor.x < pos.x + size.x
                    && cursor.y < pos.y + size.y
            });
            panel.update(over, dt);
        }
    }

    /// The species tag of the selected bibite or egg.
    fn target_tag(&self) -> Option<String> {
        if let Some(id) = self.selected {
            return self.world.bibites.get(id)?.meta.species_tag.clone();
        }
        let id = self.selected_egg?;
        self.world.eggs.get(id)?.meta.species_tag.clone()
    }

    /// Typing into the Stats panel's tag field: characters and Backspace edit it, Enter confirms
    /// (`BibiteStatsPanel.UpdateSpeciesTag`), Ctrl + V pastes the clipboard.
    fn edit_species_tag(&mut self, input: &FrameInput) {
        let mut typed = std::mem::take(&mut self.typed);
        if self.tag_edit.is_none() {
            self.edit_caret_owner = 0;
            return;
        }
        if input.paste_tag {
            typed.push_str(&self.clipboard_in.take().unwrap_or_default());
        }
        let keys = self.edit_keys(input);
        if self.edit_caret_owner != 2 {
            self.edit_caret_owner = 2;
            self.edit_caret = crate::text_field::Caret::at_end(self.tag_edit.as_deref().unwrap_or(""));
        }
        let Some(text) = self.tag_edit.as_mut() else {
            return;
        };
        crate::text_field::edit(text, &mut self.edit_caret, &typed, TAG_MAX_CHARS, keys);
        if input.enter {
            self.commit_species_tag();
        }
    }

    /// `BibiteStatsPanel.UpdateSpeciesTag`: a typed tag is added to (an empty one removed from) the
    /// selected living bibite or egg; corpses keep theirs.
    fn commit_species_tag(&mut self) {
        let Some(text) = self.tag_edit.take() else {
            return;
        };
        // The placer's custom tag field (`StringFieldHandle`): the text becomes the tag.
        if std::mem::take(&mut self.tag_edit_is_placer) {
            self.placer_options.tag = text;
            return;
        }
        if self.challenge_is_running() {
            return;
        }
        let alive = match (self.selected, self.selected_egg) {
            (Some(id), _) => self
                .world
                .bibites
                .get(id)
                .is_some_and(|record| !record.body.dead),
            (None, Some(_)) => true,
            (None, None) => false,
        };
        if alive {
            self.set_selected_tag(&text);
        }
    }

    /// Ctrl + V: the clipboard (at most 100 characters) becomes the species tag of the multi
    /// selection (`SelectionPanel.TagTargets`) or of the target (`GUIManager.TagTarget`).
    fn paste_tag(&mut self) {
        let text: String = self
            .clipboard_in
            .take()
            .unwrap_or_default()
            .chars()
            .take(TAG_MAX_CHARS)
            .collect();
        let tag = Some(text.as_str());
        if let Some(multi) = &self.multi {
            for id in multi.bibites.clone() {
                self.world.set_bibite_species_tag(id, tag);
            }
            for id in multi.eggs.clone() {
                self.world.set_egg_species_tag(id, tag);
            }
        } else if let Some(id) = self.selected {
            self.world.set_bibite_species_tag(id, tag);
            self.tag_draft = text;
        } else if let Some(id) = self.selected_egg {
            self.world.set_egg_species_tag(id, tag);
            self.egg_tag_draft = text;
        } else {
            return;
        }
        self.refresh_overlay(0.0);
    }

    /// `BibiteStatsPanel.SelectRandomChild`: selects one of the selected bibite's living children
    /// (bibites and eggs whose first parent it is); nothing happens without any.
    fn select_random_child(&mut self) {
        use crate::selection::Entity;
        let Some(parent) = self.selected else {
            return;
        };
        let mut children: Vec<Entity> = self
            .world
            .bibites
            .iter()
            .filter(|(_, record)| !record.body.dead && record.meta.parent1 == Some(parent))
            .map(|(id, _)| Entity::Bibite(id))
            .collect();
        children.extend(
            self.world
                .eggs
                .ids()
                .filter(|egg| {
                    self.world
                        .eggs
                        .get(*egg)
                        .is_some_and(|record| record.meta.parent1 == Some(parent))
                })
                .map(Entity::Egg),
        );
        if children.is_empty() {
            return;
        }
        match children[(self.next_ui_random() % children.len() as u64) as usize] {
            Entity::Bibite(id) => self.select_bibite(id),
            Entity::Egg(id) => {
                self.select_egg(id);
            }
        }
    }

    /// What the default bibites list shows for each spawn row of the world.
    #[must_use]
    pub fn default_bibite_names(&self) -> Vec<String> {
        let library = self.template_names.get_or_init(|| {
            crate::menu::list_template_entries()
                .into_iter()
                .filter_map(|entry| {
                    let file = entry.path.file_name()?.to_str()?.to_owned();
                    Some((file, entry.name))
                })
                .collect()
        });
        self.world
            .bibite_spawns
            .iter()
            .map(|row| crate::bibite_editor::display_name(&row.path, library))
            .collect()
    }

    /// `DefaultBibitesPanel.AddBibite`: a new default bibite from the template file `path`; the
    /// world's spawner then keeps its lineage at the row's target count.
    pub fn add_default_bibite(&mut self, path: &std::path::Path) {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        // The spawner finds templates by name: make sure this one is known to the world.
        let stem = |name: &str| {
            name.trim_end_matches(".bb8template")
                .trim_end_matches(".bb8")
                .to_owned()
        };
        if !self
            .world
            .templates
            .iter()
            .any(|template| stem(&template.name) == stem(file_name))
        {
            match bibites_io::load_template(path) {
                Ok(loaded) => {
                    let species_name = loaded.game_species_name(file_name);
                    self.world.templates.push(
                        TemplateSource::new(file_name.to_owned(), loaded.genome, loaded.meta, loaded.brain)
                            .with_species_name(Some(species_name)),
                    );
                }
                Err(error) => {
                    self.last_action = format!("could not read {file_name}: {error}");
                    return;
                }
            }
        }
        let mut rows = self.world.bibite_spawns.clone();
        rows.push(crate::bibite_editor::new_row(file_name));
        match self.world.set_bibite_spawns(rows) {
            Ok(()) => {
                // `ItemClicked`: the new item opens, the others close.
                self.dyn_panel.open_items.retain(|item| item.0 != 2);
                let last = self.world.bibite_spawns.len() - 1;
                self.dyn_panel.open_items.push((2, last as u16));
                self.last_action = format!("added default bibite {file_name}");
            }
            Err(error) => self.last_action = format!("could not add {file_name}: {error}"),
        }
    }

    /// `DefaultBibitesPanel.BibiteDeleted`: removes default bibite `index` at once.
    fn delete_default_bibite(&mut self, index: usize) {
        let mut rows = self.world.bibite_spawns.clone();
        if index >= rows.len() {
            return;
        }
        rows.remove(index);
        if self.world.set_bibite_spawns(rows).is_ok() {
            self.dyn_panel
                .open_items
                .retain(|item| *item != (2, index as u16));
            for item in &mut self.dyn_panel.open_items {
                if item.0 == 2 && usize::from(item.1) > index {
                    item.1 -= 1;
                }
            }
        }
    }

    /// Moves slider `field` of default bibite `index` to the cursor (`x` in screen px).
    pub fn bibite_slide(&mut self, index: usize, field: u8, track: (f32, f32), cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        let Some(info) = crate::bibite_editor::slider_info(field) else {
            return;
        };
        let Some(row) = self.world.bibite_spawns.get_mut(index) else {
            return;
        };
        if span <= 0.0 {
            return;
        }
        let value = crate::dyn_panel::value_range(
            info.mode,
            info.min,
            info.max,
            info.default,
            (cursor_x / scale - track.0) / span,
        );
        crate::bibite_editor::set_value(row, field, value);
    }

    /// `ChallengeManager.isChallenge`: a challenge is running.
    pub(crate) fn challenge_is_running(&self) -> bool {
        self.challenge.is_some() || self.challenge_params.is_some()
    }

    /// `GUIManager.targetIsBibiteOrEgg`.
    fn target_is_bibite_or_egg(&self) -> bool {
        self.selected.is_some() || self.selected_egg.is_some()
    }

    /// Whether an inspector panel shows: a bibite is selected and a panel is chosen
    /// (`GUIManager.curMenu`).
    #[must_use]
    pub fn inspector_visible(&self) -> bool {
        self.inspector_open && self.selection.is_some()
    }

    /// Whether `Right Panels/QuickStatsHolder` is inactive: `DynamicSettingsPanel.onOpen` deactivates it and `onClose` activates it again (the
    /// wiring of the world scene). With it go the Information panel (whether it was open or not: it shows again when the settings panel closes),
    /// the challenge's tracker, and the `OpenButtons` (the "Info +" button and the Challenge tab). Nothing of it is drawn or clicked meanwhile; the
    /// settings panel's creator and its OK button used to lie under the Information panel.
    #[must_use]
    pub fn quick_stats_hidden(&self) -> bool {
        self.panels.is_open(crate::ui::PanelKind::WorldDynamics)
    }

    /// `GUIManager.OpenBibitePanel(panel, toggle)`: shows `tab` and hides the other panels, or hides
    /// it again when it is the open one and `toggle` is set (the tab buttons and the digit keys).
    pub fn open_bibite_panel(&mut self, tab: InspectorTab, toggle: bool) {
        // An egg has no Biology panel (`OpenBiologyPanelButton` is inactive for it, key 3 is its Brain).
        if tab == InspectorTab::Biology && self.selected_egg.is_some() {
            return;
        }
        let open_now = self.inspector_open && self.inspector_tab == tab;
        if !open_now {
            self.inspector_tab = tab;
            self.inspector_open = true;
            self.genes_scroll = 0;
            // The panel that closes leaves the Escape stack and the new one joins on top.
            self.panels.close(crate::ui::PanelKind::Inspector);
            if self.target_is_bibite_or_egg() {
                self.panels.open_once(crate::ui::PanelKind::Inspector);
            }
        } else if toggle {
            self.inspector_open = false;
            self.panels.close(crate::ui::PanelKind::Inspector);
        }
    }

    /// Keeps the Escape stack in step with the state (`UINavigationManager`): a selected target,
    /// an open inspector panel and a multi-selection panel each hold one entry, in the order they
    /// appeared, so Escape peels them off newest first.
    pub(crate) fn sync_escape_stack(&mut self) {
        use crate::ui::PanelKind;
        let wanted = [
            (PanelKind::Target, self.has_target()),
            (PanelKind::Inspector, self.inspector_visible()),
            (
                PanelKind::ToolPanel,
                self.selected_tool.is_some() && self.tool_panel_open,
            ),
            (PanelKind::Multi, self.multi.is_some()),
            (PanelKind::BibitePlacer, self.placer.is_some()),
            (
                PanelKind::PelletPlacer,
                self.tool == crate::ui::ToolKind::Pellet,
            ),
        ];
        for (kind, want) in wanted {
            match (want, self.panels.is_open(kind)) {
                (true, false) => self.panels.open_once(kind),
                (false, true) => self.panels.close(kind),
                _ => {}
            }
        }
    }

    /// Escape: `UINavigationManager` pops the newest entry; with an empty stack the pause menu
    /// (`EscapePanel`) opens (`GUIManager.Update`).
    fn escape_pressed(&mut self) {
        use crate::ui::PanelKind;
        // An open dropdown list closes first (`DropdownItem.OnCancel`).
        if self.dropdown_open.take().is_some() {
            self.dropdown = None;
            return;
        }
        // The dialog's `EscapableAction` is the newest entry of the stack.
        if self.save_dialog.is_some() {
            self.close_save_dialog();
            return;
        }
        // The overwrite question over the scenario saver closes first, then the saver itself (its exit button).
        if self.scenario_saver.is_some() {
            if matches!(self.confirm_dialog, Some(ConfirmKind::OverwriteScenario(_))) {
                self.confirm_dialog = None;
            } else {
                self.close_scenario_saver();
            }
            return;
        }
        // Escape leaves the editor's text field first, then the editor closes (`UIPanel.Escape`).
        if let Some(session) = self.bibite_editor.as_mut()
            && session.view.focus.take().is_some()
        {
            return;
        }
        // A slider row's edit field closes before anything of the Brain tab does (it has the keyboard).
        if self.bibite_editor.is_some() && self.value_edit.take().is_some() {
            return;
        }
        // The Brain tab's own entries (the help, a field, the selection, a new connection, the add-node panel) are newer than the
        // editor's: the newest is undone first.
        if self.editor_brain_escape() {
            return;
        }
        // RUSTibites Improved: Escape closes a time tag's name field without setting the tag, and a zone's colour pop-up without a colour.
        if self.time_tag_cancel() || self.zone_color_cancel() {
            return;
        }
        // Escape closes a zone's name field without renaming it (`TextLineHandle.editField`).
        if self.zone_name_edit.take().is_some() {
            return;
        }
        // ... and a slider row's edit field without applying it (`closeEditPanel`).
        if self.value_edit.take().is_some() {
            return;
        }
        // The Expanded Brain View: its settings panel is the newest entry of the stack, then the view itself.
        if self.expanded_brain_escape() {
            return;
        }
        // A species text field gives up its edit, then the species panels close
        // (`CloseSpeciesPanels` is the newest entry of the stack), then the lineage panel.
        if self.panels.is_open(PanelKind::Lineage) {
            // The settings panel is a `UIPanel` of its own: Escape closes it.
            if self.lineage_settings_open {
                self.lineage_settings_open = false;
                return;
            }
            if self.species_cancel_edit() {
                return;
            }
            if self.species_info.is_some() {
                self.select_species_info(None);
                return;
            }
        }
        // Escape leaves the tag field without changing the tag.
        if self.tag_edit.take().is_some() {
            self.tag_edit_is_placer = false;
            return;
        }
        self.sync_escape_stack();
        match self.panels.pop_escape() {
            // `UserControl.bibiteSelectedAction`: `SelectTarget(null)`.
            Some(PanelKind::Target) => self.clear_target(),
            // `BibitePanel.OnPanelEscape`: `ResetPanelToNone`, then the panel closes.
            Some(PanelKind::Inspector) => self.inspector_open = false,
            Some(PanelKind::Multi) => self.multi = None,
            Some(PanelKind::BibiteEditor) => self.close_template_editor(),
            Some(PanelKind::BibitePlacer) => self.close_placer(),
            // `PelletPlacer.ClosePanel`.
            Some(PanelKind::PelletPlacer) => self.tool = crate::ui::ToolKind::Inspect,
            // `UIPanel.Escape` closes a tower's panel and keeps it selected; the colour killer's
            // panel deselects it instead (`ColorKillerPanel.Escape`).
            Some(PanelKind::ToolPanel) => {
                let killer = self.selected_tool_id().is_some_and(|id| {
                    matches!(
                        self.world.tools.get(id),
                        Some(bibites_core::tools::Tool::Killer(_))
                    )
                });
                if killer {
                    self.clear_target();
                } else {
                    self.tool_panel_open = false;
                }
            }
            // A panel closed; nothing else leaves with it.
            Some(_) => {}
            // The keyboard is blocked by the challenge's end (`GUIManager.Update` needs `AllowKeyboardControl`): no pause menu.
            None if self.challenge_end_shown => {}
            None => self.open_escape_menu(),
        }
    }

    /// `UserControl.SelectBibiteTarget`: like [`Self::select_bibite`], and with
    /// `ReselectOnCorpses` the target's death ends the selection.
    fn select_bibite_target(&mut self, id: BibiteId) {
        self.select_bibite(id);
        self.corpse_listener = true;
    }

    /// `UserControl.TargetDied`: without a repeat the selection ends (an egg's newborn takes over,
    /// see [`Self::update_target`]); with one the next pick comes after `ReselectionDelay`.
    fn target_died(&mut self) {
        if self.reselect.is_none() {
            self.clear_target();
        } else {
            self.reselect_timer = Some(self.reselection_delay.max(0.0));
        }
    }

    /// The target's life cycle (`TargetableObject.onDestroy` / `onDeathStatusChange`), checked
    /// after the world advanced by `game_dt` seconds of game time.
    fn update_target(&mut self, game_dt: f32) {
        // An egg that hatched hands the selection to its newborn; an aborted one just goes.
        if let Some(egg) = self.selected_egg
            && self.world.eggs.get(egg).is_none()
        {
            let child = self
                .world
                .take_hatched_children()
                .into_iter()
                .find(|(hatched, _)| *hatched == egg)
                .map(|(_, child)| child);
            match (child, self.reselect) {
                (Some(child), None) => self.select_bibite(child),
                _ => {
                    self.selected_egg = None;
                    self.selection = None;
                    self.target_died();
                }
            }
        }
        if let Some(id) = self.selected {
            match self.world.bibites.get(id) {
                // The corpse turned to meat: the object is destroyed.
                None => {
                    self.selected = None;
                    self.selection = None;
                    self.target_died();
                }
                // `OnSelectedBibiteCorpse`: a bibite picked with R / G / O ends the selection when it dies.
                Some(record) if record.body.dead && self.corpse_listener => {
                    self.corpse_listener = false;
                    if self.reselect_on_corpses {
                        self.target_died();
                    }
                }
                Some(_) => {}
            }
        }
        // `SelectNewTarget`: the delay ran out, so the repeat fires.
        if let Some(left) = self.reselect_timer {
            let left = left - game_dt;
            if left > 0.0 {
                self.reselect_timer = Some(left);
            } else {
                self.reselect_timer = None;
                match self.reselect {
                    Some(Reselect::Random) => self.select_random_bibite(),
                    Some(Reselect::HighestGeneration) => self.select_highest_generation(),
                    Some(Reselect::Oldest) => self.select_oldest(),
                    Some(Reselect::Species(species)) => self.select_random_of_species(species),
                    Some(Reselect::Tag) => {
                        if let Some(tag) = self.reselect_tag.clone() {
                            self.select_random_of_tag(&tag);
                        }
                    }
                    None => {}
                }
            }
        }
        // The data of an egg that hatched while nothing watched it must not pile up.
        if self.selected_egg.is_none() {
            let _ = self.world.take_hatched_children();
        }
    }

    /// Selects `id`, refreshes the inspector snapshot and follows it.
    pub(crate) fn select_bibite(&mut self, id: BibiteId) {
        self.corpse_listener = false;
        self.multi = None;
        self.selected_tool = None;
        self.selected_zone = None;
        self.selected_autopan = false;
        self.tool_panel_open = false;
        self.selected = Some(id);
        self.selected_egg = None;
        self.selection = SelectionPanel::collect(&self.world, id);
        self.status_bar_scale = self.selection.as_ref().map_or(1.0, |panel| panel.size);
        self.follow_selected = true;
    }

    /// `SaveController` autosave: every `AutoSavePeriod` minutes of wall-clock
    /// (`AutoSaveRealTime`) or simulated time, write
    /// `Autosaves/autosave_<yyyyMMddHHmmss>.zip` and keep `AutoSaveMaxFiles`.
    pub fn update_autosave(&mut self, wall_dt: f32, sim_seconds: f32) {
        // The write of the last autosave reports when it is done.
        if let Some(receiver) = &self.autosave_result {
            match receiver.try_recv() {
                Ok(message) => {
                    self.last_action = message;
                    self.autosave_result = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.autosave_result = None,
            }
        }
        // `AutoSave()` skips challenges (`!ScenarioSettings.Instance.isChallenge`).
        if !self.prefs.autosave || self.challenge_params.is_some() || self.challenge.is_some() {
            self.autosave_progress = 0.0;
            return;
        }
        let schedule = bibites_io::storage::AutosaveSchedule::from_preferences(&self.prefs);
        self.autosave_progress += if schedule.real_time {
            wall_dt
        } else {
            sim_seconds
        };
        if self.autosave_progress < schedule.interval().as_secs_f32() {
            return;
        }
        // The previous autosave is still being written: this one starts as soon as it is done.
        if self.autosave_result.is_some() {
            return;
        }
        // The world is on the simulation thread: the next frame gets it back and writes the save.
        if self.world_away {
            self.world_wanted = true;
            return;
        }
        self.autosave_progress = 0.0;
        if self.storage.ensure_layout().is_err() {
            return;
        }
        let mut contents = self.world.to_contents();
        contents.scene.version = GAME_VERSION.to_owned();
        if contents.scene.name.trim().is_empty() {
            contents.scene.name = "RUSTibites viewer".to_owned();
        }
        // An autosave carries the picture too (`SaveSystem` makes it for every save).
        contents.img_png = self.save_picture();
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        let stamp = autosave_timestamp();
        self.time_tags_beside_autosave(&stamp);
        // Serialising and compressing a world with tens of thousands of pellets takes seconds: it happens on a thread of its
        // own, from a copy of the world's contents, so that the window does not freeze.
        let (sender, receiver) = std::sync::mpsc::channel();
        let write = move || {
            let message = match browser.autosave_file(&stamp, &contents, schedule) {
                Ok(path) => format!("autosaved {}", path.display()),
                Err(error) => format!("autosave failed: {error}"),
            };
            let _ = sender.send(message);
        };
        match std::thread::Builder::new()
            .name("autosave".to_owned())
            .spawn(write)
        {
            Ok(_) => self.autosave_result = Some(receiver),
            Err(error) => self.last_action = format!("autosave failed: {error}"),
        }
    }

    /// Whether the write of an autosave is still running (the tests wait for it).
    #[cfg(test)]
    #[must_use]
    pub fn autosave_in_progress(&self) -> bool {
        self.autosave_result.is_some()
    }

    /// Closes the pause menu without giving the pause back: the scene it belonged to is left ("Back to Menu") or rebuilt (a
    /// load: `UINavigationManager.OnSceneChange` empties the Escape stack), so its pause source is not released into the old state.
    fn close_pause_menu_for_good(&mut self) {
        if self.panels.is_open(crate::ui::PanelKind::Escape) {
            self.panels.close(crate::ui::PanelKind::Escape);
            self.escape_pause = None;
            self.paused = true;
        }
    }

    /// Opens the pause menu, pausing the simulation until it closes.
    fn open_escape_menu(&mut self) {
        if self.escape_pause.is_none() {
            self.escape_pause = Some(self.paused);
        }
        self.paused = true;
        self.accumulator = 0.0;
        self.panels.open(crate::ui::PanelKind::Escape);
    }

    /// Opens the KILL confirmation dialog.
    fn request_kill(&mut self, kind: ConfirmKind) {
        if self.world.live_bibite_ids().is_empty() {
            self.last_action = "kill: no living bibites".to_owned();
            return;
        }
        self.confirm_dialog = Some(kind);
        self.last_action = "confirm: kill?".to_owned();
    }

    /// The options, the current choice and the look of the list of the world's dropdown `target`.
    #[must_use]
    pub fn dropdown_choices(
        &self,
        target: crate::ui::DropdownTarget,
    ) -> Option<(Vec<crate::dropdown::Choice>, usize, crate::dropdown::Style)> {
        use crate::dropdown::{COMPARED, Choice, GRAPHS, LIBERATION, ROBOTO};
        use crate::ui::DropdownTarget;
        let zone_configs = || -> Vec<_> {
            self.world
                .zones
                .zones
                .iter()
                .map(|zone| zone.config.0.clone())
                .collect()
        };
        if let Some((choices, current)) = self.animator_dropdown_choices(target) {
            return Some((choices, current, ROBOTO));
        }
        match target {
            DropdownTarget::AnimatorPreset
            | DropdownTarget::AnimatorBase(_)
            | DropdownTarget::AnimatorInterpolation(_) => None,
            DropdownTarget::Zone(index, field) => {
                let zones = zone_configs();
                let zone = self.zone_like(index)?;
                Some((
                    crate::zone_editor::choices(field, &zones),
                    crate::zone_editor::current_choice(&zone, field, &zones),
                    ROBOTO,
                ))
            }
            DropdownTarget::Bibite(index, field) => {
                let zones = zone_configs();
                let row = self.world.bibite_spawns.get(usize::from(index))?;
                Some((
                    crate::bibite_editor::choices(field, &zones),
                    crate::bibite_editor::current_choice(row, field, &zones),
                    ROBOTO,
                ))
            }
            DropdownTarget::Placer(field) => Some((
                crate::bibite_editor::choices(field, &[]),
                crate::bibite_editor::current_choice(&self.placer_options, field, &[]),
                ROBOTO,
            )),
            // `PelletPlacer.InitPanel`: the physical materials, plant then meat.
            DropdownTarget::PelletMaterial => Some((
                vec![Choice::plain("Plant"), Choice::plain("Meat")],
                usize::from(matches!(
                    self.tool_settings.material,
                    bibites_core::pellet::MaterialKind::Meat
                )),
                ROBOTO,
            )),
            // RUSTibites Improved: the gene selector's gene.
            DropdownTarget::ToolGene => {
                let id = self.selected_tool_id()?;
                let Some(bibites_core::tools::Tool::Gene(selector)) = self.world.tools.get(id).copied() else {
                    return None;
                };
                Some((
                    bibites_core::genome::GENE_DEFS.iter().map(|def| Choice::plain(def.display_name)).collect(),
                    usize::from(selector.gene),
                    LIBERATION,
                ))
            }
            DropdownTarget::ToolZone => {
                let id = self.selected_tool_id()?;
                let tool = *self.world.tools.get(id)?;
                let zones = crate::tower_panel::zone_choices(&self.world, &tool);
                Some((
                    zones
                        .labels
                        .iter()
                        .map(|label| Choice::plain(label.clone()))
                        .collect(),
                    zones.current,
                    LIBERATION,
                ))
            }
            // `GraphsPanel.Start`: one option per stream group, with its description.
            DropdownTarget::GraphsData => Some((
                self.world
                    .data_logger
                    .groups
                    .iter()
                    .map(|group| Choice::described(group.title, group.description))
                    .collect(),
                self.graphs_stream_index(),
                GRAPHS,
            )),
            // `GraphsPanel.Start`: one option per gene (`geneSettingsOrdered`), with its help text.
            DropdownTarget::GraphsGene => Some((
                (0..bibites_core::genome::N_GENES)
                    .map(|gene| {
                        let info = crate::gene_analysis::gene_info(gene);
                        Choice::described(info.name, info.help)
                    })
                    .collect(),
                self.graphs_gene.min(bibites_core::genome::N_GENES - 1),
                crate::dropdown::GENES,
            )),
            // `GraphsPanel.ChangeSelectedHistoricalData`: one option per curve of the group.
            DropdownTarget::GraphsCurves => Some((
                self.world.data_logger.groups[self.graphs_stream_index()]
                    .streams
                    .iter()
                    .map(|stream| Choice::described(stream.label, stream.description))
                    .collect(),
                0,
                GRAPHS,
            )),
            // `SpeciesTabManager.RefillComparableSpecies`: the species of the graph, by name.
            DropdownTarget::SpeciesCompared => {
                let displayed = self.species_displayed();
                let selected = displayed
                    .iter()
                    .position(|(id, _)| Some(*id) == self.species_compared)
                    .unwrap_or(0);
                Some((
                    displayed
                        .into_iter()
                        .map(|(_, name)| Choice::plain(name))
                        .collect(),
                    selected,
                    COMPARED,
                ))
            }
            // `NodeEditPanel.functionDropdown`: the 13 functions a hidden node can have, with their tooltips.
            DropdownTarget::NodeFunction => {
                let (choices, current) = self.editor_brain_function_choices()?;
                Some((choices, current, ROBOTO))
            }
            DropdownTarget::Setting(_) | DropdownTarget::Modifier(_) | DropdownTarget::Captured => {
                None
            }
        }
    }

    /// The group the Graphs panel plots (`graphs_stream`, kept inside the groups).
    #[must_use]
    pub fn graphs_stream_index(&self) -> usize {
        self.graphs_stream
            .min(self.world.data_logger.groups.len().saturating_sub(1))
    }

    /// Whether each curve of the plotted group shows (`DataStreamDescription.state`).
    #[must_use]
    pub fn graphs_curve_marks(&self) -> Vec<bool> {
        let group = self.graphs_stream_index();
        let count = self.world.data_logger.groups[group].streams.len();
        let mut marks = self.graphs_curves.get(group).cloned().unwrap_or_default();
        marks.resize(count, true);
        marks
    }

    /// The Curves dropdown's caption (`TogglesDropdown.UpdateHandles`).
    #[must_use]
    pub fn graphs_curves_label(&self) -> String {
        let marks = self.graphs_curve_marks();
        let on = marks.iter().filter(|mark| **mark).count();
        if on == 0 {
            "None Selected".to_owned()
        } else if on == marks.len() {
            "All".to_owned()
        } else {
            format!("{on} Selected")
        }
    }

    /// `GraphsPanel.ChangeSelectedHistoricalData`: plots group `index`. A scale it has no data for
    /// falls back to the deepest one it has (`DataStreamLinesGraph.SetDataStream`).
    pub fn set_graphs_stream(&mut self, index: usize) {
        self.graphs_stream = index.min(self.world.data_logger.groups.len().saturating_sub(1));
        self.graphs_scale = self.graphs_scale.min(self.deepest_graphs_scale());
    }

    /// The deepest history scale of the plotted group that has data (0 when only the finest has).
    #[must_use]
    pub fn deepest_graphs_scale(&self) -> usize {
        let history = &self.world.data_logger.groups[self.graphs_stream_index()].history;
        (1..history.n_scales())
            .rev()
            .find(|depth| history.scale_has_data(*depth))
            .unwrap_or(0)
    }

    /// `scaleButtons[depth].interactable`: the finest scale always is, the others once the plotted
    /// group has data at that depth.
    #[must_use]
    pub fn graphs_depth_has_data(&self, depth: usize) -> bool {
        let history = &self.world.data_logger.groups[self.graphs_stream_index()].history;
        depth < history.n_scales() && (depth == 0 || history.scale_has_data(depth))
    }

    /// `TogglesDropdown.OnChange`: curve `index` of the plotted group switches on or off.
    pub fn toggle_graphs_curve(&mut self, index: usize) {
        let group = self.graphs_stream_index();
        let mut marks = self.graphs_curve_marks();
        let Some(mark) = marks.get_mut(index) else {
            return;
        };
        *mark = !*mark;
        if self.graphs_curves.len() <= group {
            self.graphs_curves.resize(group + 1, Vec::new());
        }
        self.graphs_curves[group] = marks;
    }

    /// `MyDropdown.OnPointerExit`: the list of a custom dropdown closes when the pointer is over
    /// neither the dropdown nor its list (`cursor` and `screen` in px at UI scale `scale`).
    pub fn close_dropdown_on_leave(&mut self, cursor: Vec2, screen: Vec2, scale: f32) {
        if self
            .dropdown_open
            .as_ref()
            .is_some_and(|open| open.leave_closes && !open.contains(cursor, screen, scale))
        {
            self.dropdown_open = None;
        }
    }

    /// `TMP_Dropdown.Show`: the list of `target` hangs under the dropdown at `anchor` (reference px).
    pub fn open_dropdown_list(&mut self, target: crate::ui::DropdownTarget, anchor: [f32; 4]) {
        // A second click on the dropdown closes its list (`Show` while shown is `Hide`); a custom
        // dropdown only keeps its template open (`MyDropdown.OnPointerClick`).
        if let Some(open) = self.dropdown_open.as_ref()
            && open.target == target
        {
            if !open.leave_closes {
                self.dropdown_open = None;
            }
            return;
        }
        self.dropdown_open = self
            .dropdown_choices(target)
            .map(|(choices, selected, style)| {
                let open = crate::dropdown::Open::new(target, anchor, choices, selected, style);
                match target {
                    crate::ui::DropdownTarget::GraphsData
                    | crate::ui::DropdownTarget::GraphsGene => open.leaving(),
                    crate::ui::DropdownTarget::GraphsCurves => {
                        open.with_checks(self.graphs_curve_marks())
                    }
                    _ => open,
                }
            });
    }

    /// A pick in the open list: the control takes the option and the list closes
    /// (`DropdownItem.OnPointerClick` -> `OnSelectItem`).
    pub fn pick_dropdown_list(&mut self, index: usize) {
        use crate::ui::DropdownTarget;
        // `TogglesDropdown`: an item switches its curve and the list stays open (the item stays
        // the selected one until the pointer leaves).
        if let Some(open) = self.dropdown_open.as_ref()
            && open.target == DropdownTarget::GraphsCurves
            && index < open.choices.len()
        {
            self.toggle_graphs_curve(index);
            let marks = self.graphs_curve_marks();
            if let Some(open) = self.dropdown_open.as_mut() {
                open.checked = Some(marks);
                open.tint = Some(index);
            }
            return;
        }
        let Some(open) = self.dropdown_open.take() else {
            return;
        };
        if index >= open.choices.len() {
            return;
        }
        if self.animator_dropdown_pick(open.target, index) {
            return;
        }
        match open.target {
            DropdownTarget::AnimatorPreset
            | DropdownTarget::AnimatorBase(_)
            | DropdownTarget::AnimatorInterpolation(_) => {}
            DropdownTarget::Zone(zone_index, field) => {
                let size = self.world.settings.independents.simulation_size;
                self.edit_zone(usize::from(zone_index), |zone, zones| {
                    crate::zone_editor::set_choice(zone, field, index, zones);
                    crate::zone_editor::cap_position(zone, size);
                });
            }
            DropdownTarget::Bibite(row_index, field) => {
                let zones: Vec<_> = self
                    .world
                    .zones
                    .zones
                    .iter()
                    .map(|zone| zone.config.0.clone())
                    .collect();
                if let Some(row) = self.world.bibite_spawns.get_mut(usize::from(row_index)) {
                    crate::bibite_editor::set_choice(row, field, index, &zones);
                }
            }
            DropdownTarget::Placer(field) => {
                crate::bibite_editor::set_choice(&mut self.placer_options, field, index, &[]);
            }
            DropdownTarget::PelletMaterial => {
                self.tool_settings.material = if index == 1 {
                    bibites_core::pellet::MaterialKind::Meat
                } else {
                    bibites_core::pellet::MaterialKind::Plant
                };
            }
            DropdownTarget::ToolZone => {
                if let Some(id) = self.selected_tool_id() {
                    crate::tower_panel::set_zone(&mut self.world, id, index);
                    self.refresh_overlay(1.0);
                }
            }
            DropdownTarget::ToolGene => {
                if let Some(id) = self.selected_tool_id() {
                    crate::tower_panel::set_gene(&mut self.world, id, index);
                    self.refresh_overlay(1.0);
                }
            }
            DropdownTarget::GraphsData => self.set_graphs_stream(index),
            // `BarGraph.UpdateSelectedData`.
            DropdownTarget::GraphsGene => {
                self.graphs_gene = index.min(bibites_core::genome::N_GENES - 1);
            }
            // `SpeciesTabManager.SelectComparedSpeciesFromDropdown`.
            DropdownTarget::SpeciesCompared => {
                if let Some((id, _)) = self.species_displayed().get(index) {
                    self.species_compared = Some(*id);
                    // `UpdateCompared`: the Brain tab marks the differences with it again.
                    self.brain_update_marks();
                }
            }
            DropdownTarget::Captured => self.pick_dropdown_choice(index as u32),
            // `NodeEditPanel.EditFunction`.
            DropdownTarget::NodeFunction => self.editor_brain_pick_function(index),
            DropdownTarget::Setting(_)
            | DropdownTarget::Modifier(_)
            | DropdownTarget::GraphsCurves => {}
        }
    }

    /// Whether the player switched the action warning off with its "do not show again" checkbox
    /// (`ActionWarning.doNotShowAgain`): kept in the user settings file, else for this run only.
    #[must_use]
    pub fn warning_off(&self, warning: crate::warnings::ActionWarning) -> bool {
        match &self.user_settings_path {
            Some(path) => {
                crate::user_settings::UserSettings::load(Some(path.clone())).warning_off(warning)
            }
            None => self.warnings_off.contains(&warning),
        }
    }

    /// `WarningHandle.Dismiss` with the box ticked: the warning is not shown again.
    fn switch_warning_off(&mut self, warning: crate::warnings::ActionWarning) {
        match self.user_settings_path.clone() {
            Some(path) => {
                crate::user_settings::UserSettings::load(Some(path)).set_warning_off(warning, true);
            }
            None => {
                self.warnings_off.insert(warning);
            }
        }
    }

    /// "Reset Warnings" (`UserSettingsManager.ResetAllWarnings`): the warnings of
    /// `ActionWarnings.allInfoShown` are shown again.
    fn reset_warnings(&mut self) {
        match self.user_settings_path.clone() {
            Some(path) => crate::user_settings::UserSettings::load(Some(path)).reset_warnings(),
            None => {
                for warning in crate::warnings::ALL_INFO_SHOWN {
                    self.warnings_off.remove(&warning);
                }
            }
        }
        self.last_action = "warnings reset".to_owned();
    }

    /// Executes the pending KILL confirmation.
    fn confirm_pending(&mut self) {
        let Some(kind) = self.confirm_dialog.take() else {
            return;
        };
        // `WarningHandle.Dismiss`: a ticked box switches the warning off before its action runs.
        let ticked = std::mem::take(&mut self.confirm_do_not_show);
        if ticked && let Some(warning) = kind.warning() {
            self.switch_warning_off(warning);
        }
        match kind {
            ConfirmKind::KillSelected => self.kill_selected(),
            // `ChallengeTrackerPanel.RetryChallenge`: the same scenario and champion again.
            ConfirmKind::RestartChallenge if self.challenge_setup.is_some() => {
                self.restart_challenge();
            }
            ConfirmKind::RestartChallenge => self.start_local_challenge(),
            ConfirmKind::OverwriteSave(name) => self.save_as_to(&name),
            ConfirmKind::OverwriteScenario(name) => self.scenario_saver_write(&name),
            ConfirmKind::DeleteSave(path) => {
                self.delete_save_path(&path);
            }
            ConfirmKind::PruneSpecies(id) => self.species_actually_prune(id),
            ConfirmKind::KillHalf => {
                // The selection is left alone: a selected bibite that dies stays as a corpse, an
                // aborted egg ends it (`update_target`).
                let (killed, aborted) = self.world.kill_random_half();
                self.last_action =
                    format!("killed {killed} bibites, aborted {aborted} eggs (half)");
                self.refresh_overlay(1.0);
                // `UserActions.KillHalfBibites`: once a session (`triggeredThanos`).
                if !self.achievements.triggered_snap {
                    self.achievements.triggered_snap = true;
                    self.trigger_achievement("ACH_USER_SNAP");
                }
            }
            ConfirmKind::ClearDefaultBibites => {
                // `ActuallyDeleteAllBibites`: the rows go; bibites already spawned stay.
                if self.world.set_bibite_spawns(Vec::new()).is_ok() {
                    self.dyn_panel.open_items.retain(|item| item.0 != 2);
                    self.last_action = "removed all default bibites".to_owned();
                }
            }
            ConfirmKind::DeleteZone(index) => self.delete_zone(index),
            ConfirmKind::ClearZones => self.clear_zones(),
            ConfirmKind::DeleteAnimator(index) => self.delete_animator(index),
            ConfirmKind::ClearAnimators => self.clear_animators(),
            ConfirmKind::ClearStreams => self.clear_streams(),
            ConfirmKind::RemoveHalfPellets => {
                let removed = self.world.remove_random_half_pellets();
                self.last_action = format!("removed {removed} pellets (half)");
                self.refresh_overlay(1.0);
            }
        }
    }

    /// Sets the selected bibite's or egg's species tag (TAG editor control).
    pub fn set_selected_tag(&mut self, tag: &str) -> bool {
        if let Some(id) = self.selected_egg {
            let value = Some(tag).filter(|tag| !tag.is_empty());
            let applied = self.world.set_egg_species_tag(id, value);
            self.egg_tag_draft = tag.to_owned();
            self.last_action = format!("egg tag: {tag}");
            self.refresh_overlay(1.0);
            return applied;
        }
        let Some(id) = self.selected else {
            return false;
        };
        let applied = self.world.set_bibite_species_tag(id, Some(tag));
        self.tag_draft = tag.to_owned();
        self.last_action = format!("tag: {tag}");
        self.refresh_overlay(1.0);
        applied
    }

    /// Selects an egg (inspector/TAG target).
    pub fn select_egg(&mut self, id: bibites_core::ids::EggId) {
        if let Some(record) = self.world.eggs.get(id) {
            self.corpse_listener = false;
            self.multi = None;
            self.selected_tool = None;
            self.selected_zone = None;
            self.selected_autopan = false;
            self.tool_panel_open = false;
            self.selected_egg = Some(id);
            self.selected = None;
            self.selection = SelectionPanel::collect_egg(&self.world, id);
            self.status_bar_scale = self.selection.as_ref().map_or(1.0, |panel| panel.size);
            self.follow_selected = true;
            self.egg_tag_draft = record.meta.species_tag.clone().unwrap_or_default();
            self.last_action = format!("egg {id} selected");
            // `BiologyPanel.FillPanel`: an egg has no Biology panel; when that one is open it hands over to the Stats
            // panel (`OpenBibitePanel(StatsPanel)`).
            if self.inspector_open && self.inspector_tab == InspectorTab::Biology {
                self.open_bibite_panel(InspectorTab::Stats, false);
            }
        }
    }

    /// SAVE inspector button for eggs: writes the first egg entity.
    #[allow(dead_code)] // the dialog replaced the direct save
    fn save_eggs(&mut self) {
        let egg = self
            .world
            .eggs
            .ids()
            .find_map(|id| self.world.egg_save(id).map(|save| (id, save)));
        let Some((id, save)) = egg else {
            self.last_action = "save egg: no eggs".to_owned();
            return;
        };
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("save egg failed: {error}");
            return;
        }
        // `SaveSystem.savedBibitePath`: single bibites and eggs go to `Bibites/`, not `Savefiles/`.
        let path = self.storage.bibites_dir().join(format!("egg-{id}.bb8"));
        match bibites_io::save_egg_entity(&path, &save) {
            Ok(()) => self.last_action = format!("saved egg to {}", path.display()),
            Err(error) => self.last_action = format!("save egg failed: {error}"),
        }
    }

    /// `SaveController.SaveBibiteOrEgg`: the Save Bibite dialog opens for the selected bibite or
    /// egg, named after its species; the game pauses and every control is blocked.
    fn open_save_dialog(&mut self) {
        use crate::save_dialog::{DEFAULT_NAME, Target};
        let (target, species) = match (self.selected, self.selected_egg) {
            (Some(id), _) => (
                Target::Bibite(id),
                self.world.bibites.get(id).and_then(|r| r.meta.species),
            ),
            (None, Some(egg)) => (
                Target::Egg(egg),
                self.world.eggs.get(egg).and_then(|e| e.meta.species),
            ),
            (None, None) => {
                self.last_action = "save: no selection".to_owned();
                return;
            }
        };
        let (name, description) = species
            .and_then(|species| self.world.species.get(species))
            .map_or((DEFAULT_NAME.to_owned(), String::new()), |record| {
                (record.name(), record.description.clone())
            });
        self.begin_save_dialog(target, name, description, true);
    }

    /// `SaveBibiteDialogHandle.OpenDialog(defaultText, defaultDescription, allowSaved)`: the dialog opens, the game pauses and every control is blocked. Without
    /// `allowSaved` (a species' template, `SaveController.SaveTemplateOfBibite`) "Save as template" is switched on and cannot be changed; switching it on
    /// stores the player's choice like a click on it does (`saveAsTemplate.onValueChanged`).
    pub(crate) fn begin_save_dialog(&mut self, target: crate::save_dialog::Target, name: String, description: String, allow_saved: bool) {
        use crate::save_dialog::{Field, SaveDialog};
        if !allow_saved && !self.save_as_template {
            self.save_as_template = true;
            self.store_save_option("Save as template", true);
        }
        self.save_dialog = Some(SaveDialog {
            target,
            name,
            description,
            as_template: self.save_as_template,
            allow_saved,
            format_json: self.format_bibite_json,
            focus: Field::Name,
            previous_pause: self.paused,
            caret: crate::text_field::Caret { pos: usize::MAX, anchor: None },
        });
        self.paused = true;
        self.accumulator = 0.0;
    }

    /// `SaveBibiteDialogHandle.CloseDialog`: the pause the dialog held ends.
    fn close_save_dialog(&mut self) {
        if let Some(dialog) = self.save_dialog.take() {
            self.paused = dialog.previous_pause;
            self.accumulator = 0.0;
        }
    }

    /// The dialog's checkboxes keep the player's choice (`UserSettings.SaveBibiteAsTemplate`,
    /// `FormatBibiteJSON`): it persists to the viewer's user settings.
    fn toggle_save_dialog_option(&mut self, which: u8) {
        let Some(dialog) = self.save_dialog.as_mut() else {
            return;
        };
        // A species' template is always saved as a template: its box is not interactable (`OpenDialog(allowSaved: false)`).
        if which == 0 && !dialog.allow_saved {
            return;
        }
        let (label, value) = if which == 0 {
            dialog.as_template = !dialog.as_template;
            self.save_as_template = dialog.as_template;
            ("Save as template", dialog.as_template)
        } else {
            dialog.format_json = !dialog.format_json;
            self.format_bibite_json = dialog.format_json;
            ("Format JSON", dialog.format_json)
        };
        self.store_save_option(label, value);
    }

    /// The player's choice of one of the dialog's checkboxes goes to the user settings file (`UserSettings.SaveBibiteAsTemplate`, `FormatBibiteJSON`).
    fn store_save_option(&self, label: &str, value: bool) {
        if let Some(path) = self.user_settings_path.clone() {
            let mut settings = crate::user_settings::UserSettings::load(Some(path));
            if settings.flag(label) != value
                && let Some(index) = crate::user_settings::index_of(label)
            {
                settings.toggle(index);
            }
        }
    }

    /// Typing into the dialog: characters and Backspace edit the focused field, Enter breaks a line
    /// of the description, Ctrl + V pastes.
    fn edit_save_dialog(&mut self, input: &FrameInput) {
        // Only an open dialog takes the typed characters and the clipboard.
        if self.save_dialog.is_none() {
            return;
        }
        let typed = std::mem::take(&mut self.typed);
        let pasted = if input.paste_tag {
            self.clipboard_in.take().unwrap_or_default()
        } else {
            String::new()
        };
        let keys = self.edit_keys(input);
        let Some(dialog) = self.save_dialog.as_mut() else {
            return;
        };
        dialog.type_text(&typed);
        dialog.type_text(&pasted);
        if input.backspace {
            dialog.backspace();
        }
        dialog.keys(crate::text_field::EditKeys { backspace: false, ..keys });
        if input.enter {
            dialog.enter();
        }
    }

    /// `SaveController.SaveDialogConfirmed`: a template goes to the templates folder (named and
    /// described by the dialog), a plain bibite or egg to the saved bibites folder with the version
    /// and description; the JSON is indented with "Format JSON".
    fn confirm_save_dialog(&mut self) {
        use crate::save_dialog::Target;
        let Some(dialog) = self.save_dialog.clone() else {
            return;
        };
        if !dialog.can_save() {
            return;
        }
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("save failed: {error}");
            return;
        }
        let path = dialog.path(&self.storage.bibites_dir(), &self.storage.templates_dir());
        let result = if dialog.as_template {
            let source = match dialog.target {
                Target::Bibite(id) => self.world.bibite_template_source(id),
                Target::Species(id) => self.species_save_source(id),
                Target::Egg(id) => self.world.eggs.get(id).map(|egg| {
                    TemplateSource::new(
                        dialog.name.clone(),
                        egg.genome.clone(),
                        egg.meta.clone(),
                        egg.brain.clone(),
                    )
                }),
            };
            match source {
                Some(mut source) => {
                    source.name = dialog.name.clone();
                    bibites_io::save_template_source_described(
                        &path,
                        &source,
                        &dialog.description,
                        dialog.format_json,
                    )
                }
                None => Err(bibites_core::Error::invalid_data("nothing to save")),
            }
        } else {
            match dialog.target {
                Target::Bibite(id) => match self.world.bibite_save(id) {
                    Some(save) => bibites_io::save_bibite_entity_described(
                        &path,
                        &save,
                        GAME_VERSION,
                        &dialog.description,
                        dialog.format_json,
                    ),
                    None => Err(bibites_core::Error::invalid_data("nothing to save")),
                },
                // A species' template is only ever saved as a template (its box is locked on).
                Target::Species(_) => Err(bibites_core::Error::invalid_data("nothing to save")),
                Target::Egg(id) => match self.world.egg_save(id) {
                    Some(save) => bibites_io::save_egg_entity_described(
                        &path,
                        &save,
                        GAME_VERSION,
                        &dialog.description,
                        dialog.format_json,
                    ),
                    None => Err(bibites_core::Error::invalid_data("nothing to save")),
                },
            }
        };
        match result {
            Ok(()) => {
                self.last_action = format!("saved {}", path.display());
                if dialog.as_template {
                    // The Library and the template selector list it from now on.
                    self.template_names = std::cell::OnceCell::new();
                }
            }
            Err(error) => self.last_action = format!("save failed: {error}"),
        }
        self.close_save_dialog();
    }

    /// SAVE inspector button: entity save of the selected bibite.
    #[allow(dead_code)] // the dialog replaced the direct save
    fn save_selected_entity(&mut self) {
        let Some(id) = self.selected else {
            self.last_action = "save: no selection".to_owned();
            return;
        };
        let Some(save) = self.world.bibite_save(id) else {
            self.last_action = "save: no entity".to_owned();
            return;
        };
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("save failed: {error}");
            return;
        }
        let path = self.storage.bibites_dir().join(format!("bibite-{id}.bb8"));
        match bibites_io::save_bibite_entity(&path, &save) {
            Ok(()) => self.last_action = format!("saved bibite to {}", path.display()),
            Err(error) => self.last_action = format!("save failed: {error}"),
        }
    }

    /// EDIT inspector button: export the selected bibite as a template.
    #[allow(dead_code)] // kept for the template-export path
    fn save_selected_template(&mut self) {
        let Some(id) = self.selected else {
            self.last_action = "edit: no selection".to_owned();
            return;
        };
        let Some(template) = self.world.bibite_template_source(id) else {
            self.last_action = "edit: no template".to_owned();
            return;
        };
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("edit failed: {error}");
            return;
        }
        let path = self
            .storage
            .templates_dir()
            .join(format!("viewer-bibite-{id}.bb8template"));
        match bibites_io::save_template_source(&path, &template) {
            Ok(()) => self.last_action = format!("exported template {}", path.display()),
            Err(error) => self.last_action = format!("edit failed: {error}"),
        }
    }

    /// Saves the current world into the local storage saves directory.
    fn save_world(&mut self, quick: bool) {
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("save failed: {error}");
            return;
        }
        let mut contents = self.world.to_contents();
        // `SaveSystem` writes `Application.version` into every scene.
        contents.scene.version = GAME_VERSION.to_owned();
        if contents.scene.name.trim().is_empty() {
            contents.scene.name = "RUSTibites viewer".to_owned();
        }
        // `SceneScreenShotHandler.SaveScreenshotToZip`: every save carries a picture of the world as it is.
        contents.img_png = self.save_picture();
        if quick {
            match browser.quick_save(&contents) {
                Ok(path) => {
                    self.last_action = format!("quick saved {}", path.display());
                    self.time_tags_after_save(&path);
                }
                Err(error) => self.last_action = format!("save failed: {error}"),
            }
            self.refresh_save_entries();
            return;
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let name = format!("viewer-{stamp}");
        match browser.save_world_with_list(&name, &contents, true, false) {
            Ok((path, rows)) => {
                self.save_entries = rows;
                self.last_action = format!("saved {}", path.display());
                self.time_tags_after_save(&path);
            }
            Err(error) => self.last_action = format!("save failed: {error}"),
        }
    }

    /// `SaveGamePanel.Save` (also the overwrite dialog's OK): the world and its picture go to `Savefiles/<name>.zip`.
    pub(crate) fn save_as_to(&mut self, name: &str) {
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        let mut contents = self.world.to_contents();
        contents.scene.version = GAME_VERSION.to_owned();
        // `SceneScreenShotHandler.SaveScreenshotToZip`: every save carries a picture of the world as it is.
        contents.img_png = self.save_picture();
        match browser.save_world_with_list(name, &contents, true, false) {
            Ok((path, rows)) => {
                self.save_entries = rows;
                self.last_action = format!("saved as {}", path.display());
                self.time_tags_after_save(&path);
                // `SaveGamePanel.Save` (also the overwrite dialog's OK): `SaveWorld`, then `ClosePanel` - back to what was under it.
                self.panels.close(crate::ui::PanelKind::SaveGame);
            }
            Err(error) => self.last_action = format!("save-as failed: {error}"),
        }
    }

    fn load_selected_save(&mut self) {
        let Some(entry) = self.save_entries.get(self.save_selected).cloned() else {
            self.last_action = "load: no rows".to_owned();
            return;
        };
        let path = entry.path.clone();
        self.load_save_path(&path);
    }

    fn delete_selected_save(&mut self) {
        let Some(entry) = self.save_entries.get(self.save_selected).cloned() else {
            self.last_action = "delete: no rows".to_owned();
            return;
        };
        // `PopupManager.DisplayActionWarning`: once "do not show again" was ticked the action
        // runs at once.
        if self.warning_off(crate::warnings::ActionWarning::DeleteSaveFile) {
            self.delete_save_path(&entry.path);
            return;
        }
        self.confirm_do_not_show = false;
        self.confirm_dialog = Some(ConfirmKind::DeleteSave(entry.path));
        self.last_action = "confirm: delete save?".to_owned();
    }

    /// A pellet placer slider follows the cursor: "Pellet Size Factor" is a logarithmic slider from
    /// 0.1 to 1000 (default 1) and "Pellet Number" a whole-number one from 1 to 10
    /// (`PelletPlacer.pelletSize`, `pelletCount`).
    pub fn pellet_slide(&mut self, field: u8, track: (f32, f32), cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        if span <= 0.0 {
            return;
        }
        let fraction = ((cursor_x / scale - track.0) / span).clamp(0.0, 1.0);
        match field {
            crate::placer_panel::PELLET_SIZE => {
                self.tool_settings.pellet_energy = crate::dyn_panel::value_range(
                    crate::dyn_panel::log(10.0, None),
                    0.1,
                    1000.0,
                    1.0,
                    fraction,
                )
                .clamp(0.1, 1000.0);
            }
            crate::placer_panel::PELLET_COUNT => {
                let count = crate::dyn_panel::value_range(
                    crate::dyn_panel::SliderMode::Int,
                    1.0,
                    10.0,
                    1.0,
                    fraction,
                );
                self.tool_settings.pellet_count = (count.round() as usize).clamp(1, 10);
            }
            _ => {}
        }
    }

    /// Applies a captured in-world Dynamics/Settings control to safe world
    /// settings (structural fields are staged with a status message).
    pub fn apply_world_captured_setting(
        &mut self,
        _key: &str,
        label: &str,
        value: crate::ui::ControlValue,
    ) {
        use crate::ui::ControlValue;
        use bibites_core::settings::{IndependentSettings, ScenarioSettings, SettingValue};
        let number = match value {
            ControlValue::Float(float) => Some(float),
            ControlValue::Integer(int) => Some(int as f32),
            ControlValue::Choice(choice) => Some(choice as f32),
            _ => None,
        };
        let flag = match value {
            ControlValue::Flag(flag) => Some(flag),
            ControlValue::Choice(choice) => Some(choice != 0),
            _ => None,
        };
        let label_lower = label.to_ascii_lowercase();
        let mut candidates: Vec<String> = Vec::new();
        for meta in ScenarioSettings::SETTINGS
            .iter()
            .chain(IndependentSettings::SETTINGS.iter())
        {
            let name = meta.label.to_ascii_lowercase();
            if label_lower.contains(&name) || name.contains(&label_lower) {
                candidates.push(meta.key.to_owned());
            }
        }
        if candidates.is_empty() {
            for (needle, key) in [
                ("biomass", "biomassDensity"),
                ("fertility", "pelletGrowth"),
                ("limit bibite", "limitBibiteBirth"),
                ("bibite limit", "bibiteLimit"),
                ("mutation", "backgroundMutationChance"),
            ] {
                if label_lower.contains(needle) {
                    candidates.push(key.to_owned());
                }
            }
        }
        if candidates.is_empty() {
            self.last_action = format!("no world setting matched: {label}");
            return;
        }
        let mut settings = self.world.settings.clone();
        let mut applied = false;
        for key in candidates {
            let attempts: Vec<SettingValue> = flag
                .map(SettingValue::Bool)
                .into_iter()
                .chain(number.map(SettingValue::Float))
                .chain(number.map(|value| SettingValue::Int(value as i32)))
                .collect();
            for attempt in attempts {
                if settings.independents.set_by_key(&key, attempt).is_some()
                    || settings.scenario.set_by_key(&key, attempt).is_some()
                {
                    applied = true;
                    break;
                }
            }
            if applied {
                break;
            }
        }
        if !applied {
            self.last_action = format!("setting rejected: {label}");
            return;
        }
        match self.world.apply_settings(settings) {
            Ok(()) => {
                self.last_action = format!("world setting applied: {label}");
                self.refresh_overlay(1.0);
            }
            Err(error) => self.last_action = format!("structural setting staged: {error}"),
        }
    }

    /// Starts the simulation thread (`--sim-thread`). From then on a frame that only watches runs without the world: the world
    /// moves to the thread, which ticks it and publishes what the window draws (see [`crate::sim_thread`]).
    pub fn enable_sim_thread(&mut self) {
        let root = self.assets.as_ref().map(|assets| assets.root.clone());
        self.sim = Some(crate::sim_thread::SimThread::spawn(root));
    }

    /// Whether the real world is on the simulation thread (`world` is a husk with its settings).
    #[must_use]
    pub fn world_away(&self) -> bool {
        self.world_away
    }

    /// Whether the app is in a state where frames can go on without the world: it only watches. Anything that looks at or
    /// changes the world's content (a panel, a selection, a tool, a placer, a challenge, a pause...) needs the world.
    fn watching_only(&self) -> bool {
        self.sim.is_some()
            && !self.paused
            && self.panels.top().is_none()
            && self.tool == crate::ui::ToolKind::Inspect
            && self.placer.is_none()
            && self.selected.is_none()
            && self.selected_egg.is_none()
            && self.selected_tool.is_none()
            && self.selected_zone.is_none()
            && !self.selected_autopan
            && !self.follow_selected
            && self.multi.is_none()
            && self.challenge.is_none()
            && self.challenge_params.is_none()
            && self.dropdown_open.is_none()
            && self.confirm_dialog.is_none()
            && self.save_dialog.is_none()
            && self.scenario_saver.is_none()
            && self.value_edit.is_none()
            && self.tag_edit.is_none()
            && self.zone_name_edit.is_none()
            && self.hotkey_listening.is_none()
            && self.template_pick.is_none()
            && self.reselect.is_none()
            && self.drag.is_none()
            && self.panel_pause.is_none()
            && self.escape_pause.is_none()
            && !self.inspector_visible()
            // The toolbar's hover panels list the world's zones, groups and spawns.
            && !self.toolbar.iter().any(|panel| panel.open)
            && !self.lineage_gate_popup
    }

    /// Whether this frame's input can be handled without the world: the camera's keys, wheel and drags, and the toggles of what
    /// is drawn. A click, a typed key, a hotkey that acts on a bibite or the world needs it.
    #[must_use]
    pub fn light_input(raw: &FrameInput) -> bool {
        !raw.primary_down
            && !raw.primary_pressed
            && !raw.primary_released
            && !raw.toggle_pause
            && !raw.step
            && !raw.fit
            && !raw.fit_bibites
            && !raw.reset_view
            && raw.digit.is_none()
            && !raw.quit
            && !raw.select_random_bibite
            && !raw.select_high_generation
            && !raw.select_oldest
            && !raw.select_random_egg
            && !raw.select_random_zone
            && !raw.select_autopanner
            && !raw.quick_save
            && !raw.brain_sort_keys.iter().any(|down| *down)
            && !raw.brain_hidden_down
            && !raw.lay_egg
            && !raw.kill_target
            && !raw.camera_home
            && !raw.undo
            && !raw.redo
            && !raw.copy_tag
            && !raw.paste_tag
            && !raw.backspace
            && !raw.enter
    }

    /// Decides where the world is for this frame: on the simulation thread after a few frames that only watched, back in the
    /// app (waiting for the tick in progress, at most) as soon as one frame needs it. Call before the frame's UI and input.
    pub fn frame_world(&mut self, raw: &FrameInput) {
        if self.sim.is_none() {
            return;
        }
        let light = self.watching_only() && Self::light_input(raw) && !self.world_wanted;
        if !light {
            self.light_streak = 0;
            self.ensure_world_home();
            return;
        }
        self.light_streak = self.light_streak.saturating_add(1);
        if !self.world_away
            && self.light_streak >= LIGHT_FRAMES_BEFORE_AWAY
            && self.sim.as_mut().is_some_and(|sim| sim.is_ready())
        {
            self.send_world_away();
        }
    }

    /// Runs one tick of the world the app holds, with the run's simulation switches on it (a world that was loaded, restored or made a moment ago
    /// has the default, every switch off, until it is given them here; every in-process tick goes through this).
    fn tick_world(&mut self) {
        self.world.improved = self.improved;
        self.world.tick();
    }

    /// Hands the world to the simulation thread; `world` becomes a husk with its settings.
    fn send_world_away(&mut self) {
        self.world.improved = self.improved;
        let husk = crate::sim_thread::husk_like(&self.world);
        let world = std::mem::replace(&mut self.world, husk);
        self.accumulator = 0.0;
        // Until the thread publishes its first snapshot the window shows what it drew last, with the numbers of the world as
        // it leaves (the husk would read as an empty world with the clock at zero).
        let seed = std::sync::Arc::new(crate::sim_thread::Snapshot {
            seq: 0,
            scene: self.scene.clone(),
            sim_time: world.simulated_time(),
            tick: world.clock.tick(),
            hud: self.hud.clone(),
            hover: self
                .scene_request()
                .hover
                .map(|point| crate::sim_thread::hover_answer(&world, point)),
            pellets: world.pellets.len(),
            bibites: world.bibites.len(),
        });
        let request = self.scene_request();
        let engine = self.engine_speed();
        let Some(sim) = self.sim.as_mut() else {
            self.world = world;
            return;
        };
        if sim.is_running() {
            self.world = world;
            return;
        }
        sim.set_engine(engine);
        sim.set_request(request);
        sim.clear_snapshot();
        self.sim_ticks_seen = sim.ticks();
        sim.run(Box::new(world));
        self.world_away = true;
        self.snapshot = Some(seed);
    }

    /// After the frame's input is handled: a state that needs the world (a panel that opened under the pointer, a selection...)
    /// brings it home before the frame is simulated and drawn.
    pub fn settle_world(&mut self) {
        if self.world_away && !self.watching_only() {
            self.light_streak = 0;
            self.ensure_world_home();
        }
    }

    /// Takes the world back from the simulation thread (it finishes the tick it is in) if it has it.
    pub fn ensure_world_home(&mut self) {
        self.join_overlapped_ticks();
        if !self.world_away {
            return;
        }
        let Some(sim) = self.sim.as_mut() else {
            return;
        };
        let world = sim.reclaim();
        self.world = *world;
        self.world_away = false;
        self.world_wanted = false;
        self.snapshot = None;
        self.accumulator = 0.0;
        self.count_worker_ticks();
        // The auto-panner moves with every tick of the world (`AutoPanner.FixedUpdate`), the ones of the time away included;
        // it needs the world's zones, so it takes its steps now.
        let owed = std::mem::take(&mut self.autopan_owed).min(AUTOPAN_CATCH_UP_LIMIT);
        let fixed = self.world.settings.fixed_dt();
        for _ in 0..owed {
            self.step_autopan(fixed);
        }
        self.hud = HudInfo::collect(&self.world);
    }

    /// Counts the ticks the simulation thread has run since the last count (for the ticks per second and the autosave).
    fn count_worker_ticks(&mut self) -> u64 {
        let Some(sim) = self.sim.as_ref() else {
            return 0;
        };
        let total = sim.ticks();
        let delta = total.saturating_sub(self.sim_ticks_seen);
        self.sim_ticks_seen = total;
        self.sim_ticks += delta;
        self.autopan_owed += delta;
        let fixed = self.world.settings.fixed_dt();
        for _ in 0..delta.min(10_000) {
            self.time_keeper.fixed_tick(self.wall_time, fixed);
        }
        delta
    }

    /// What the simulation thread draws for the window.
    fn scene_request(&self) -> crate::sim_thread::SceneRequest {
        crate::sim_thread::SceneRequest {
            camera: self.camera,
            selected: self.selected,
            options: self.draw_options(),
            tool_view: self.tool_view(),
            hover: Some(self.camera.screen_to_world(self.cursor)),
        }
    }

    /// The frame of an app whose world is on the simulation thread: the camera, the numbers of the time panel, the HUD from the
    /// newest snapshot. Nothing here reads the world.
    fn advance_light(&mut self, frame_dt: f32) {
        // A tick that panicked on the thread took the world with it: end the window as a tick in the loop would have.
        assert!(
            !self.sim.as_ref().is_some_and(|sim| sim.has_died()),
            "the simulation thread died (a tick panicked, see the message above)"
        );
        self.wall_time += f64::from(frame_dt.max(0.0));
        let dt = frame_dt.clamp(0.0, 0.25);
        self.update_ruler();
        self.update_zoom(dt);
        self.count_worker_ticks();
        let engine = self.engine_speed();
        let request = self.scene_request();
        if let Some(sim) = self.sim.as_ref() {
            sim.set_engine(engine);
            sim.set_request(request);
            if let Some(snapshot) = sim.latest()
                && self
                    .snapshot
                    .as_ref()
                    .is_none_or(|old| old.seq != snapshot.seq)
            {
                self.hud = snapshot.hud.clone();
                self.snapshot = Some(snapshot);
            }
        }
        self.last_tick_secs = 0.0;
        // `TimeKeeper.Update` hands each measured frame rate to `TimeController.CheckMinFPS`.
        if let Some(present_fps) = self.time_keeper.render_frame(self.wall_time, false) {
            self.check_min_fps(present_fps);
        }
        self.fps = self.time_keeper.fps().round() as i32;
    }

    /// The display list to draw this frame: the simulation thread's newest while the world is away, the app's own otherwise.
    #[must_use]
    pub fn drawn_scene(&self) -> &Scene {
        match (&self.snapshot, self.world_away()) {
            (Some(snapshot), true) => &snapshot.scene,
            _ => &self.scene,
        }
    }

    /// Live pellets and bibites, from the snapshot while the world is away.
    #[must_use]
    pub fn frame_counts(&self) -> (usize, usize) {
        match (&self.snapshot, self.world_away()) {
            (Some(snapshot), true) => (snapshot.pellets, snapshot.bibites),
            _ => (self.world.pellets.len(), self.world.bibites.len()),
        }
    }

    /// The time scale the simulation really runs at (`engineTimeScale`, what Unity's `Time.timeScale` is): the slider's target,
    /// slowed down by the minimum-FPS governor when the frame rate fell under the player's minimum.
    #[must_use]
    pub fn engine_speed(&self) -> f32 {
        self.speed * self.engine_fraction
    }

    /// Sets the time slider's target (`TimeController.SetTarget`): the engine scale follows at once when it is lowered or
    /// when there is no minimum FPS, and is left to the governor when it is raised.
    pub fn set_target_speed(&mut self, speed: f32) {
        let engine = self.engine_speed();
        let target = speed.clamp(MIN_SPEED, MAX_SPEED);
        self.speed = target;
        let engine = crate::time_keeper::engine_scale_after_target(
            engine,
            target,
            self.prefs.min_fps,
            self.time_keeper.first_update(),
            self.time_keeper.fps(),
            self.paused,
        );
        self.engine_fraction = (engine / target).clamp(0.0, 1.0);
    }

    /// `TimeController.CheckMinFPS` after a measurement of the frame rate.
    fn check_min_fps(&mut self, present_fps: f32) {
        let present_fps = self.frame_rate_for_governor(present_fps);
        if let Some(engine) = crate::time_keeper::min_fps_scale(
            self.engine_speed(),
            self.speed,
            present_fps,
            self.prefs.min_fps,
            self.paused,
        ) {
            self.engine_fraction = (engine / self.speed).clamp(0.0, 1.0);
        }
    }

    /// Time-speed slider position `0..=1` of `speed` (`LogFloatSettingSlider(targetTimeScale, 5)`:
    /// default 1, range 0.1..100, so the slider space is `log5(v)`).
    #[must_use]
    pub fn speed_fraction(speed: f32) -> f32 {
        let (min, max) = (
            (0.1_f32).ln() / 5.0_f32.ln(),
            (100.0_f32).ln() / 5.0_f32.ln(),
        );
        let position = if speed > 0.0 {
            speed.ln() / 5.0_f32.ln()
        } else {
            min
        };
        ((position - min) / (max - min)).clamp(0.0, 1.0)
    }

    /// Moves the time-speed slider to the cursor (`x` in screen px); moving it resumes play
    /// (`TimeController.SetTarget` -> `ForcePlay`).
    pub fn speed_slide(&mut self, track: (f32, f32), cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let span = track.1 - track.0;
        if span <= 0.0 {
            return;
        }
        if !self.slider_moved(cursor_x) {
            return;
        }
        let fraction = ((cursor_x / scale - self.slider_grab - track.0) / span).clamp(0.0, 1.0);
        let (min, max) = (
            (0.1_f32).ln() / 5.0_f32.ln(),
            (100.0_f32).ln() / 5.0_f32.ln(),
        );
        let position = min + fraction * (max - min);
        let speed = if fraction <= 0.0 {
            MIN_SPEED
        } else {
            5.0_f32.powf(position)
        };
        self.set_target_speed(speed);
        self.paused = false;
    }

    /// Applies an edit of the dynamic panel to a copy of the world settings and installs it.
    fn dyn_edit(
        &mut self,
        label: &str,
        edit: impl FnOnce(&mut bibites_core::settings::SimSettings) -> bool,
    ) {
        let before = self.world.settings.clone();
        let mut settings = before.clone();
        if !edit(&mut settings) {
            return;
        }
        match self.world.apply_settings(settings) {
            Ok(()) => {
                self.last_action = format!("{label} updated");
                if self.suppress_history {
                    return;
                }
                let after = self.world.settings.clone();
                if self.dyn_panel.drag.is_some() || self.dyn_panel.material_drag.is_some() {
                    // A drag is one action: keep the settings it started from.
                    match &mut self.undo_drag {
                        Some(UndoAction::Settings(pair)) => pair.1 = after,
                        _ => {
                            self.flush_undo_drag();
                            self.undo_drag = Some(UndoAction::Settings(Box::new((before, after))));
                        }
                    }
                } else {
                    self.flush_undo_drag();
                    self.undo
                        .push(UndoAction::Settings(Box::new((before, after))));
                }
            }
            Err(error) => self.last_action = format!("{label}: {error}"),
        }
    }

    /// Closes the action a finished slider drag made.
    fn flush_undo_drag(&mut self) {
        if let Some(action) = self.undo_drag.take() {
            self.undo.push(action);
        }
    }

    /// A control of a zone item changed the zone's settings: the edit joins the history (a slider
    /// drag counts once, as the game's `ChangeSettingHandleAction` does for the zone's settings).
    fn record_zone_edit(
        &mut self,
        before: bibites_core::settings::ZoneSettings,
        after: bibites_core::settings::ZoneSettings,
    ) {
        if before == after || self.suppress_history {
            return;
        }
        let id = after.zone_id;
        if self.dyn_panel.zone_drag.is_some() {
            match &mut self.undo_drag {
                Some(UndoAction::Zone {
                    id: held,
                    after: latest,
                    ..
                }) if *held == id => **latest = after,
                _ => {
                    self.flush_undo_drag();
                    self.undo_drag = Some(UndoAction::Zone {
                        id,
                        before: Box::new(before),
                        after: Box::new(after),
                    });
                }
            }
        } else {
            self.flush_undo_drag();
            self.undo.push(UndoAction::Zone {
                id,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
    }

    /// The zones were renumbered by a removal: the steps that edited zones are forgotten.
    fn drop_zone_history(&mut self) {
        self.flush_undo_drag();
        self.undo.drop_zone_steps();
    }

    /// Puts a zone's settings back (or forward) as the history says: the zone is found by its
    /// persistent id, and a zone that is gone leaves the step without effect.
    fn apply_zone_history(
        &mut self,
        id: Option<u32>,
        settings: bibites_core::settings::ZoneSettings,
    ) {
        let zones = self.zone_settings_list();
        let Some(index) = zones.iter().position(|zone| zone.zone_id == id) else {
            return;
        };
        let Some(zone) = self.world.zones.zone_id_at(index) else {
            return;
        };
        match self.world.apply_zone_settings(zone, settings) {
            Ok(()) => self.refresh_overlay(0.0),
            Err(error) => self.last_action = format!("zone history rejected: {error}"),
        }
    }

    /// Ctrl + Z / Ctrl + Shift + Z (`UINavigationManager.Update`): only with something open on the
    /// Escape stack, exactly like the game (`emptyEscapeStack` returns first).
    fn undo_or_redo(&mut self, redo: bool) {
        if self.panels.is_empty() {
            return;
        }
        self.flush_undo_drag();
        let target = if redo {
            self.undo.redo()
        } else {
            self.undo.revert()
        };
        match target {
            Some(UndoAction::Settings(pair)) => {
                let settings = if redo { pair.1 } else { pair.0 };
                match self.world.apply_settings(settings) {
                    Ok(()) => {
                        self.last_action = if redo { "redo" } else { "undo" }.to_owned();
                    }
                    Err(error) => self.last_action = format!("undo failed: {error}"),
                }
            }
            Some(UndoAction::Zone { id, before, after }) => {
                self.apply_zone_history(id, if redo { *after } else { *before });
                self.last_action = if redo { "redo" } else { "undo" }.to_owned();
            }
            None => {}
        }
    }

    /// Flips a toggle of the dynamic panel.
    fn dyn_toggle(&mut self, index: usize) {
        if self.setting_is_animated(index) {
            return;
        }
        let label = crate::setting_ui_data::SETTING_UI
            .get(index)
            .map_or("setting", |row| row.name);
        self.dyn_edit(label, |settings| {
            crate::dyn_panel::toggle_value(settings, index)
        });
    }

    /// Resets dynamic-panel settings to their defaults.
    fn dyn_reset(&mut self, indices: &[usize]) {
        let indices: Vec<usize> = indices.iter().copied().filter(|index| !self.setting_is_animated(*index)).collect();
        let indices = &indices[..];
        let tps = self.tps_defaults;
        self.dyn_edit("settings", |settings| {
            crate::dyn_panel::reset_values(settings, indices, tps)
        });
    }

    /// Moves a dynamic-panel slider to the cursor (`x` in screen px).
    pub fn dyn_slide(&mut self, index: usize, track: (f32, f32), cursor_x: f32) {
        if self.setting_is_animated(index) {
            return;
        }
        let label = crate::setting_ui_data::SETTING_UI
            .get(index)
            .map_or("setting", |row| row.name);
        if !self.slider_moved(cursor_x) {
            return;
        }
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let x = cursor_x / scale - self.slider_grab;
        let tps = self.tps_defaults;
        self.dyn_edit(label, |settings| {
            crate::dyn_panel::slide_value(settings, index, track, x, tps)
        });
    }

    /// `Slider.OnPointerDown` for the time-speed slider: how far from its handle's middle (reference px) a press at screen `cursor_x` is, when it is
    /// on the handle (the handle then keeps its place and the speed does not jump), `None` on the track (the handle goes under the pointer).
    #[must_use]
    pub fn speed_slider_grab(&self, track: (f32, f32), cursor_x: f32) -> Option<f32> {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let handle = track.0 + Self::speed_fraction(self.speed) * (track.1 - track.0);
        crate::menu::slider_grab_on(handle, cursor_x / scale, 1.0)
    }

    /// The same for the slider of setting row `index` (a dynamic settings panel row): the handle is at the setting's value.
    #[must_use]
    pub fn dyn_slider_grab(&self, index: usize, track: (f32, f32), cursor_x: f32) -> Option<f32> {
        let row = crate::setting_ui_data::SETTING_UI.get(index)?;
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let (mode, _) = crate::dyn_panel::slider_info_or_default(row);
        let default = crate::dyn_panel::default_of(row, self.tps_defaults);
        let value = crate::dyn_panel::read_value(&self.world.settings, row);
        let handle = track.0 + crate::dyn_panel::fraction_of(mode, row, default, value) * (track.1 - track.0);
        crate::menu::slider_grab_on(handle, cursor_x / scale, 1.0)
    }

    /// A press has taken a slider at screen `cursor_x`: `grab` is [`Self::speed_slider_grab`] / [`Self::dyn_slider_grab`]'s answer. On the handle
    /// the slider keeps the offset and waits for the pointer to move; on the track it follows the pointer from the first call.
    pub fn begin_slider_hold(&mut self, grab: Option<f32>, cursor_x: f32) {
        self.slider_grab = grab.unwrap_or(0.0);
        self.slider_press_x = grab.map(|_| cursor_x);
    }

    /// Whether the held slider is to follow the pointer at screen `cursor_x`: not while a press on its handle has not moved yet.
    fn slider_moved(&mut self, cursor_x: f32) -> bool {
        match self.slider_press_x {
            Some(at) if (cursor_x - at).abs() < 0.5 => false,
            Some(_) => {
                self.slider_press_x = None;
                true
            }
            None => true,
        }
    }

    /// Whether a settings animator animates the setting of row `index`: it cannot be edited by hand meanwhile (`SetInteractable(false)`).
    #[must_use]
    pub fn setting_is_animated(&self, index: usize) -> bool {
        crate::setting_ui_data::SETTING_UI
            .get(index)
            .is_some_and(|row| self.world.is_animated(row.field))
    }

    /// Flips a boolean scenario setting (dynamic-settings checkbox).
    fn toggle_scenario_setting(&mut self, index: usize) {
        use bibites_core::settings::{ScenarioSettings, SettingValue};
        let Some(meta) = ScenarioSettings::SETTINGS.get(index) else {
            return;
        };
        let Some(SettingValue::Bool(current)) = self.world.settings.scenario.get_by_key(meta.key)
        else {
            return;
        };
        let mut settings = self.world.settings.clone();
        if settings
            .scenario
            .set_by_key(meta.key, SettingValue::Bool(!current))
            .is_none()
        {
            return;
        }
        match self.world.apply_settings(settings) {
            Ok(()) => {
                self.last_action = format!("{} = {}", meta.label, !current);
                self.refresh_overlay(1.0);
            }
            Err(error) => self.last_action = format!("setting rejected: {error}"),
        }
    }

    /// Local achievement progress `(name, achieved, progress text)`.
    #[must_use]
    pub fn achievement_progress(&self) -> Vec<(String, bool, String)> {
        let days = self.world.simulated_time() / 86_400.0;
        let population = self.hud.population;
        let generation = self.hud.generation_mean;
        vec![
            (
                "Bibite 100 children".to_owned(),
                population >= 100,
                format!("population {population}/100"),
            ),
            (
                "Bibite 1 day old".to_owned(),
                self.hud.age_mean_hours >= 24.0,
                format!("mean age {:.1}/24 h", self.hud.age_mean_hours),
            ),
            (
                "Bibite Editor 100".to_owned(),
                self.editor_genome.is_some(),
                "open the genome editor".to_owned(),
            ),
            (
                "Bibite Einstein".to_owned(),
                generation >= 10.0,
                format!("gen {generation:.1}/10"),
            ),
            (
                "Simulation 1 FPS".to_owned(),
                self.fps <= 1 && self.fps > 0,
                format!("fps {}", self.fps),
            ),
            (
                "Simulation 7 days".to_owned(),
                days >= 7.0,
                format!("{days:.2}/7 days"),
            ),
            (
                "User snapshot".to_owned(),
                self.save_entries
                    .iter()
                    .any(|entry| entry.name.contains("viewer")),
                "save a world".to_owned(),
            ),
        ]
    }

    /// Refreshes challenge/champion candidates and opens the picker.
    pub fn open_challenge_picker(&mut self) {
        let _ = self.storage.ensure_layout();
        let all_scenarios: Vec<(String, PathBuf)> =
            bibites_io::storage::list_scenarios(&self.storage)
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| (entry.name, entry.path))
                        .collect()
                })
                .unwrap_or_default();
        // `ScenarioSelectorPanel` lists challenges separately: only scenarios that define
        // a challenge. Without any, fall back to every scenario so a custom challenge can
        // still be started locally.
        let challenge_only: Vec<(String, PathBuf)> = all_scenarios
            .iter()
            .filter(|(_, path)| {
                bibites_io::load_scenario_bundle(path)
                    .is_ok_and(|bundle| bundle.settings_file.challenge.is_some())
            })
            .cloned()
            .collect();
        self.challenge_scenarios = if challenge_only.is_empty() {
            all_scenarios
        } else {
            challenge_only
        };
        self.champion_templates = bibites_io::storage::list_template_entries(&self.storage)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| (entry.name, entry.path))
                    .collect()
            })
            .unwrap_or_default();
        self.challenge_scenario_selected = self
            .challenge_scenario_selected
            .min(self.challenge_scenarios.len().saturating_sub(1));
        self.champion_selected = self
            .champion_selected
            .min(self.champion_templates.len().saturating_sub(1));
        if self.challenge_draft.is_none() {
            self.challenge_draft = Some(
                self.challenge_params
                    .clone()
                    .unwrap_or_else(bibites_core::settings::ChallengeParameters::game_defaults),
            );
        }
        self.panels.open(crate::ui::PanelKind::ChallengePicker);
        self.last_action = format!(
            "{} challenge(s), {} champion template(s)",
            self.challenge_scenarios.len(),
            self.champion_templates.len()
        );
    }

    /// Rebuilds the draft parameters from the selected timer/star presets.
    fn rebuild_challenge_draft(&mut self) {
        use bibites_core::settings::{
            ChallengeParameters, Comparator, ConditionGroup, SimCondition, SimMetric,
        };
        const TIMERS: [&str; 6] = ["300", "600", "1200", "1800", "3600", "7200"];
        const STARS: [&str; 6] = ["10", "25", "50", "100", "200", "400"];
        let tag_condition = |value: &'static str| {
            ConditionGroup::single(SimCondition::new(
                SimMetric::TagCount,
                "Champion",
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                value,
            ))
        };
        let mut params = ChallengeParameters::game_defaults();
        params.exit_condition = ConditionGroup::single(SimCondition::new(
            SimMetric::Time,
            "",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            TIMERS[self.challenge_timer_index.min(TIMERS.len() - 1)],
        ));
        params.one_star_condition = tag_condition(STARS[self.challenge_star_indices[0].min(5)]);
        params.two_star_condition = tag_condition(STARS[self.challenge_star_indices[1].min(5)]);
        params.three_star_condition = tag_condition(STARS[self.challenge_star_indices[2].min(5)]);
        self.challenge_draft = Some(params);
    }

    /// Adjusts timer/stars in the challenge parameters panel.
    pub fn adjust_challenge_param(&mut self, index: usize, delta: i32) {
        match index {
            0 => {
                self.challenge_timer_index =
                    (self.challenge_timer_index as i32 + delta).clamp(0, 5) as usize;
            }
            star @ 1..=3 => {
                let slot = star - 1;
                self.challenge_star_indices[slot] =
                    (self.challenge_star_indices[slot] as i32 + delta).clamp(0, 5) as usize;
            }
            _ => return,
        }
        self.rebuild_challenge_draft();
        self.last_action = "challenge parameters updated".to_owned();
    }

    /// Persists draft challenge parameters into the source scenario.
    pub fn save_challenge_params(&mut self) {
        let Some(path) = self.source_scenario.clone() else {
            self.last_action = "challenge save: no source scenario".to_owned();
            return;
        };
        let Some(draft) = self.challenge_draft.clone() else {
            return;
        };
        let result = (|| {
            let mut bundle =
                bibites_io::load_scenario_bundle(&path).map_err(|error| error.to_string())?;
            bundle.settings_file.challenge = Some(draft);
            bibites_io::storage::update_scenario_settings(&path, &bundle.settings_file)
                .map_err(|error| error.to_string())
        })();
        match result {
            Ok(()) => self.last_action = format!("challenge saved {}", path.display()),
            Err(error) => self.last_action = format!("challenge save failed: {error}"),
        }
    }

    /// Puts a freshly built world in the place of the running one (what loading a scenario or a save does to the app).
    pub(crate) fn adopt_world(&mut self, loaded: LoadedWorld) {
        self.world = loaded.world;
        self.time_tags_reset();
        self.undo.clear();
        self.undo_drag = None;
        self.tail_states = initial_tail_states(&self.world);
        self.selected = None;
        self.selected_egg = None;
        self.selection = None;
        self.follow_selected = false;
        self.multi = None;
        self.selected_tool = None;
        self.tool_panel_open = false;
        self.tool_drag = None;
        self.placer = None;
        self.template_pick = None;
        self.placer_pause = None;
        self.forget_zone_editor();
        self.accumulator = 0.0;
        self.sim_ticks = 0;
        self.fit_bibites();
    }

    /// Starts the local scored challenge (game-default parameters, local only).
    pub fn start_local_challenge(&mut self) {
        use bibites_core::settings::ChallengeParameters;
        // A challenge scenario with a champion picked: the game's own start (`StartChallenge(true)`).
        if let (Some((_, scenario)), Some((champion_name, champion_path))) = (
            self.challenge_scenarios
                .get(self.challenge_scenario_selected)
                .cloned(),
            self.champion_templates.get(self.champion_selected).cloned(),
        ) {
            let start = crate::challenge_run::ChallengeStart {
                scenario,
                champion: Some(crate::challenge_run::Champion {
                    path: champion_path,
                    name: champion_name,
                }),
                as_challenge: true,
            };
            let seed = self.world.seed ^ 0x5EED_C0DE;
            if let Ok((loaded, Some(setup))) =
                LoadedWorld::load_challenge_start(&start, seed, self.world.threads())
            {
                self.adopt_world(loaded);
                self.begin_challenge(setup);
                self.last_action = "challenge started".to_owned();
                return;
            }
        }
        let mut parameters = self
            .challenge_draft
            .clone()
            .or_else(|| self.challenge_params.clone())
            .unwrap_or_else(ChallengeParameters::game_defaults);
        // `ChallengeManager` / `ScenarioSelectorPanel.StartChallenge`: a challenge runs in
        // a *fresh* world built from its scenario (so the `Time` metric starts at 0 and
        // pre-existing "Champion" tags cannot count). Fall back to the running world only
        // when the scenario cannot be loaded.
        if let Some((_, path)) = self
            .challenge_scenarios
            .get(self.challenge_scenario_selected)
            .cloned()
        {
            // The challenge definition (exit condition, scoring, star conditions) is part
            // of the scenario file; the local draft only applies to scenarios without one.
            if let Some(embedded) = bibites_io::load_scenario_bundle(&path)
                .ok()
                .and_then(|bundle| bundle.settings_file.challenge.clone())
            {
                parameters = embedded;
            }
            match LoadedWorld::load(Some(&path), 12_345, 1, self.world.threads()) {
                Ok(loaded) => {
                    self.world = loaded.world;
                    self.time_tags_reset();
                    self.undo.clear();
                    self.undo_drag = None;
                    self.tail_states = initial_tail_states(&self.world);
                    self.selected = None;
                    self.selected_egg = None;
                    self.selection = None;
                    self.follow_selected = false;
                    self.multi = None;
                    self.selected_tool = None;
                    self.tool_panel_open = false;
                    self.tool_drag = None;
                    self.placer = None;
                    self.template_pick = None;
                    self.placer_pause = None;
                    self.forget_zone_editor();
                    self.accumulator = 0.0;
                    self.sim_ticks = 0;
                    self.fit_bibites();
                }
                Err(error) => {
                    self.last_action = format!("challenge scenario not loaded: {error}");
                }
            }
        }
        let started = self.world.simulated_time();
        let scenario_name = self
            .challenge_scenarios
            .get(self.challenge_scenario_selected)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| "Local Challenge".to_owned());
        let champion_name = self
            .champion_templates
            .get(self.champion_selected)
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        let champion_path = self
            .champion_templates
            .get(self.champion_selected)
            .map(|(_, path)| path.display().to_string())
            .unwrap_or_default();
        self.challenge_high = self.challenge.as_ref().map(|session| session.score_info());
        // Insert the champion spawn row, then spawn the champion immediately so
        // the challenge starts with it on the field.
        if let Some((_, champion_path)) =
            self.champion_templates.get(self.champion_selected).cloned()
        {
            parameters.champion.path = champion_path.display().to_string();
            parameters.champion.is_external = true;
            parameters.champion.tagging = bibites_core::settings::Tagging::CustomTagging;
            parameters.champion.tag = "Champion".to_owned();
            // The spawn row is what spawns the champions: `minimum` of them at the spawner's next pass (the game's rows do the
            // same; spawning one more at the camera was this port's own addition).
            if let Err(error) = self.world.add_bibite_spawn(parameters.champion.clone()) {
                self.last_action = format!("champion spawn row rejected: {error}");
            }
        } else if !parameters.champion.path.is_empty() {
            // Scenario-embedded challenge champion row (no picker override).
            if let Err(error) = self.world.add_bibite_spawn(parameters.champion.clone()) {
                self.last_action = format!("champion spawn row rejected: {error}");
            } else {
                self.last_action = "champion spawned".to_owned();
            }
        }
        self.challenge = Some(bibites_core::world::ChallengeSession::new(
            parameters,
            scenario_name,
            "Survive and grow the Champion line (local score)",
            champion_name,
            champion_path,
            started,
        ));
        self.challenge_end_shown = false;
        self.challenge_record = None;
        self.last_action = "challenge started".to_owned();
    }

    /// Advances the challenge session; called once per viewer frame after ticks.
    pub fn update_challenge(&mut self, sim_dt: f32) {
        let Some(session) = self.challenge.as_mut() else {
            return;
        };
        if session.tick(&self.world, sim_dt) {
            let info = session.score_info();
            let high_is_better = session.parameters.high_score_is_better;
            self.challenge_high = Some(info.clone());
            self.challenge_record = match bibites_io::storage::record_challenge_result_over(
                &self.storage,
                &info,
                high_is_better,
                &utc_iso8601(),
                Some(bibites_io::prefs::GamePrefs::global()),
            ) {
                Ok(outcome) => Some(outcome),
                Err(error) => {
                    self.last_action = format!("could not store challenge result: {error}");
                    None
                }
            };
            self.challenge_stars_achievements(&info.challenge_name, info.star_attained);
            self.challenge_end_shown = true;
            // `ChallengeManager.RunChallenge` pauses the game ("ChallengeEnd")
            // before showing the end popup (cs:84-85); `ContinueAsScenario` gives that pause back.
            self.challenge_end_pause = Some(self.paused);
            self.paused = true;
            self.last_action = "challenge ended".to_owned();
        }
    }

    /// Current challenge status line for HUD/tests.
    #[allow(dead_code)] // TODO(wire): HUD consumes session fields directly
    #[must_use]
    pub fn challenge_status(&self) -> Option<(u32, u8, f64, bool)> {
        let session = self.challenge.as_ref()?;
        Some((
            session.stars as u32,
            session.stars,
            session.elapsed_s,
            session.state == bibites_core::world::ChallengeState::Ended,
        ))
    }

    /// Hotkey action names in display order (prefs default keys).
    #[must_use]
    pub fn hotkey_actions() -> [&'static str; 8] {
        [
            "pause",
            "step",
            "fit",
            "fit_bibites",
            "reset_view",
            "toggle_grid",
            "toggle_pheromones",
            "screenshot",
        ]
    }

    /// Assigns a captured key string to a hotkey action (UserPreferences).
    pub fn assign_hotkey(&mut self, index: usize, key: &str) {
        let actions = Self::hotkey_actions();
        let Some(action) = actions.get(index) else {
            return;
        };
        self.prefs
            .hotkeys
            .insert((*action).to_owned(), key.to_owned());
        self.hotkey_listening = None;
        if let Err(error) = bibites_io::storage::save_user_preferences(&self.storage, &self.prefs) {
            self.last_action = format!("hotkey save failed: {error}");
        } else {
            self.last_action = format!("hotkey {action} = {key}");
        }
    }

    /// Resets hotkeys to the game defaults (UserPreferences).
    pub fn reset_hotkeys(&mut self) {
        self.prefs.hotkeys = bibites_io::storage::default_hotkeys();
        if let Err(error) = bibites_io::storage::save_user_preferences(&self.storage, &self.prefs) {
            self.last_action = format!("hotkey reset failed: {error}");
        } else {
            self.last_action = "hotkeys reset".to_owned();
        }
    }

    /// Renames the selected local Workshop item to the draft name.
    pub fn rename_workshop_selected(&mut self) {
        let Some((_name, path, _kind)) = self.workshop.get(self.workshop_selected).cloned() else {
            self.last_action = "workshop rename: no selection".to_owned();
            return;
        };
        let name = self.workshop_name_draft.trim().to_owned();
        if name.is_empty() {
            self.last_action = "workshop rename: empty name".to_owned();
            return;
        }
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        match browser.rename_workshop(&path, &name) {
            Ok(_) => {
                self.refresh_workshop();
                self.last_action = format!("renamed workshop item to {name}");
            }
            Err(error) => self.last_action = format!("workshop rename failed: {error}"),
        }
    }

    /// Refreshes the local Workshop browser entries.
    pub fn refresh_workshop(&mut self) {
        self.workshop.clear();
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("workshop layout failed: {error}");
            return;
        }
        match bibites_io::storage::list_workshop_entries(&self.storage) {
            Ok(entries) => {
                self.workshop = entries
                    .into_iter()
                    .map(|entry| (entry.name, entry.path, entry.kind))
                    .collect();
                self.workshop_selected = self
                    .workshop_selected
                    .min(self.workshop.len().saturating_sub(1));
                self.last_action = format!("{} workshop item(s)", self.workshop.len());
            }
            Err(error) => self.last_action = format!("workshop list failed: {error}"),
        }
    }

    /// Exports the selected Workshop entry into `target/workshop-export/`.
    pub fn export_workshop_selected(&mut self) {
        let Some((name, path, _kind)) = self.workshop.get(self.workshop_selected).cloned() else {
            self.last_action = "workshop export: no selection".to_owned();
            return;
        };
        let dest_dir = Path::new("target").join("workshop-export");
        if let Err(error) = std::fs::create_dir_all(&dest_dir) {
            self.last_action = format!("export failed: {error}");
            return;
        }
        let dest = dest_dir.join(&name);
        match bibites_io::storage::export_workshop_item(&path, &dest) {
            Ok(()) => self.last_action = format!("exported {}", dest.display()),
            Err(error) => self.last_action = format!("export failed: {error}"),
        }
    }

    /// Deletes the selected Workshop entry.
    pub fn delete_workshop_selected(&mut self) {
        let Some((name, path, _kind)) = self.workshop.get(self.workshop_selected).cloned() else {
            self.last_action = "workshop delete: no selection".to_owned();
            return;
        };
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        match browser.delete_workshop(&path) {
            Ok(_rows) => {
                self.refresh_workshop();
                self.last_action = format!("deleted workshop item {name}");
            }
            Err(error) => self.last_action = format!("delete failed: {error}"),
        }
    }

    /// Applies viewer preferences to live scene state.
    pub fn apply_user_preferences(&mut self) {
        self.scene_options.grid = self.prefs.show_grid;
        self.scene_options.pheromones = self.prefs.show_pheromones;
        if let Err(error) = bibites_io::storage::save_user_preferences(&self.storage, &self.prefs) {
            self.last_action = format!("prefs save failed: {error}");
        } else {
            self.last_action = "preferences applied".to_owned();
        }
    }

    /// Maps a captured settings control to a viewer preference.
    pub fn set_pref_by_key(&mut self, key: &str, value: crate::ui::ControlValue) -> bool {
        use crate::ui::ControlValue;
        let flag = match value {
            ControlValue::Flag(flag) => Some(flag),
            ControlValue::Choice(choice) => Some(choice != 0),
            _ => None,
        };
        let number = match value {
            ControlValue::Float(float) => Some(float),
            ControlValue::Integer(int) => Some(int as f32),
            ControlValue::Choice(choice) => Some(choice as f32),
            _ => None,
        };
        let applied = if key.contains("fullscreen") {
            if let Some(flag) = flag {
                self.prefs.fullscreen = flag;
            }
            true
        } else if key.contains("resolution") || key.contains("screen size") {
            if let Some(choice) = number {
                let presets = [
                    (1280, 720),
                    (1600, 900),
                    (1920, 1080),
                    (2560, 1440),
                    (3840, 2160),
                ];
                let (width, height) = presets[(choice as usize).min(presets.len() - 1)];
                self.prefs.resolution_width = width;
                self.prefs.resolution_height = height;
            }
            true
        } else if key.contains("ui scale") || key.contains("uiscale") {
            if let Some(number) = number {
                self.prefs.ui_scale = number.clamp(0.5, 3.0);
            }
            true
        } else if key.contains("pheromone") && !key.contains("production") {
            if let Some(flag) = flag {
                self.prefs.show_pheromones = flag;
                self.scene_options.pheromones = flag;
            } else if let Some(number) = number {
                self.prefs.pheromone_opacity = number.clamp(0.0, 1.0);
            }
            true
        } else if key.contains("background") {
            self.prefs.background_mode = format!("{value:?}");
            true
        } else if key.contains("grid") {
            if let Some(flag) = flag {
                self.prefs.show_grid = flag;
                self.scene_options.grid = flag;
            }
            true
        } else if key.contains("node name") {
            if let Some(flag) = flag {
                self.prefs.show_node_names = flag;
            }
            true
        } else if key.contains("autosave") && key.contains("period") {
            if let Some(number) = number {
                self.prefs.autosave_period_minutes = number.max(0.0) as u32;
            }
            true
        } else if key.contains("max") && key.contains("save") {
            if let Some(number) = number {
                self.prefs.autosave_max_files = number.max(0.0) as u32;
            }
            true
        } else if key.contains("pause after") {
            if let Some(flag) = flag {
                self.prefs.pause_after_load = flag;
            }
            true
        } else if key.contains("reload after") {
            if let Some(flag) = flag {
                self.prefs.reload_after_autosaves = flag;
            }
            true
        } else if key.contains("minimum fps") || key.contains("min fps") {
            if let Some(number) = number {
                self.prefs.min_fps = number.max(0.0) as u32;
            }
            true
        } else if key.contains("selection") {
            if let Some(flag) = flag {
                self.prefs.selection_outline = flag;
            }
            true
        } else if key.contains("camera") {
            if let Some(number) = number {
                self.prefs.camera_pan_speed = number.max(0.1);
            }
            true
        } else {
            false
        };
        if applied {
            self.last_action = format!("pref {key}");
        }
        applied
    }

    /// `BibiteStatsPanel.EditBibite`: `new BibiteTemplate(body)` (named after its species, or its tag, with its genes and its brain),
    /// then `BibiteTemplateSelectorPanel.OpenForBibiteEditor`: the Library opens and the Bibite Editor on the template over it.
    pub fn open_template_editor_for_selection(&mut self) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(record) = self.world.bibites.get(id) else {
            return;
        };
        let name = record
            .meta
            .species
            .and_then(|species| self.world.species.get(species))
            .map(|species| species.name())
            .or_else(|| record.meta.species_tag.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| crate::template_editor::DEFAULT_NAME.to_owned());
        let model = self.make_template_editor(
            record.genome.genes,
            name,
            String::new(),
            record.brain.clone(),
        );
        self.open_library_for_editor(model);
    }

    /// `SpeciesPanel.EditBibite`: `new BibiteTemplate(selectedSpecies.template)` (named after the species, with its genes and its
    /// brain) in the Bibite Editor over the Library.
    pub fn species_edit(&mut self) {
        let Some(id) = self.species_info else {
            return;
        };
        let Some(record) = self.world.species.get(id) else {
            return;
        };
        let model = self.make_template_editor(
            record.template.genome.genes,
            record.name(),
            String::new(),
            record.template.brain.clone(),
        );
        self.open_library_for_editor(model);
    }

    /// A model for the Bibite Editor on a template's parts, with the player's template folder behind its name check.
    #[must_use]
    pub fn make_template_editor(
        &self,
        genes: [f32; bibites_core::genome::N_GENES],
        name: String,
        description: String,
        brain: Brain,
    ) -> crate::template_editor::TemplateEditor {
        let folder = self.storage.templates_dir();
        let official = crate::template_editor::official_names(&folder);
        crate::template_editor::TemplateEditor::from_parts(
            genes,
            name,
            description,
            brain,
            std::collections::BTreeMap::new(),
            official,
            move |name| folder.join(format!("{name}.bb8template")).is_file(),
        )
    }

    /// `BibiteTemplateSelectorPanel.OpenForBibiteEditor(template)`: the Library opens (it pauses the game while it is open) and the
    /// editor opens on the template over it, with "Save and submit" (`AfterEditorSubmit`).
    pub fn open_library_for_editor(&mut self, model: crate::template_editor::TemplateEditor) {
        self.sync_escape_stack();
        self.panels.open_once(crate::ui::PanelKind::Library);
        self.open_template_editor(model, true);
    }

    /// The Library's buttons `EditSelectedBibite` (the selected template or saved bibite, `path`) and `OpenEditorForNewBibite`
    /// (`path` is `None`: `EditTemplate(null)`). `submit` is whether the editor has "Save and submit".
    pub fn open_editor_from_library(&mut self, path: Option<&Path>, submit: bool) {
        let folder = self.storage.templates_dir();
        let official = crate::template_editor::official_names(&folder);
        let exists = move |name: &str| folder.join(format!("{name}.bb8template")).is_file();
        let model = match path {
            Some(path) => {
                match crate::template_editor::TemplateEditor::from_file(path, official, exists) {
                    Ok(model) => model,
                    Err(error) => {
                        self.last_action = format!("could not read {}: {error}", path.display());
                        return;
                    }
                }
            }
            None => crate::template_editor::TemplateEditor::new_empty(
                crate::template_editor::blank_brain(),
                official,
                exists,
            ),
        };
        self.open_template_editor(model, submit);
    }

    /// `BibiteEditorPanel.EditTemplate`: the editor shows over everything, on its Genes tab. `submit` is whether it has the
    /// "Save and submit" button (`actionOnSubmit != null`).
    pub fn open_template_editor(
        &mut self,
        model: crate::template_editor::TemplateEditor,
        submit: bool,
    ) {
        self.value_edit = None;
        self.library_refill = None;
        // `BibiteBrainEditor.FillEditor`: the brain editor takes the template's nodes, connections and kept positions, and sizes its
        // canvas by the window.
        let mut session = crate::template_editor_ui::EditorSession::for_screen(
            model,
            [self.camera.screen.x, self.camera.screen.y],
        );
        session.submit = submit;
        self.bibite_editor = Some(session);
        // The target's and the inspector's Escape entries come first, so the editor is the newest (`UINavigationManager`).
        self.sync_escape_stack();
        self.panels.open_once(crate::ui::PanelKind::BibiteEditor);
        self.last_action = "Bibite Editor opened".to_owned();
    }

    /// `BibiteEditorPanel.ClosePanel`: the Library under it fills its lists again (`AfterEditorClose`).
    pub fn close_template_editor(&mut self) {
        self.bibite_editor = None;
        self.value_edit = None;
        self.panels.close(crate::ui::PanelKind::BibiteEditor);
        self.library_refill = Some(None);
    }

    /// A control of the Bibite Editor was clicked.
    pub fn apply_editor_action(&mut self, action: crate::template_editor_ui::EditorAction) {
        use crate::template_editor_ui::{EditorAction, VISIBLE_ROWS, max_scroll_rows};
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        match action {
            EditorAction::Tab(tab) => {
                session.view.tab = tab;
                session.view.focus = None;
            }
            EditorAction::Save => self.editor_save(false),
            EditorAction::Submit => self.editor_save(true),
            EditorAction::Back => self.close_template_editor(),
            // `BibiteEditorPreview.ToggleOpenPreview`.
            EditorAction::Info => {
                session.view.info_open = !session.view.info_open;
                session.view.focus = None;
            }
            // `SpriteGenerator.SaveSpriteOfPreview` asks for a file with the system's save dialog and builds the picture from the
            // previewer's sprites: neither is available here.
            EditorAction::SaveSprite => self.editor_save_sprite(),
            // A slider follows the cursor while it is held (`editor_gene_slide`).
            EditorAction::GeneSlide(..) => {}
            EditorAction::GeneReset(gene) => {
                let gene = usize::from(gene).min(bibites_core::genome::N_GENES - 1);
                session.model.reset_gene(gene);
            }
            EditorAction::Focus(field) => {
                session.view.focus = Some(field);
                session.view.idle = 0.0;
                // The keyboard is the field's now: the Brain tab's label and filter fields let go of it.
                session.view.brain.label_focus = false;
                session.view.brain.filter_focus = false;
            }
            EditorAction::Page(up) => {
                let rows = &mut session.view.scroll_rows;
                *rows = if up {
                    rows.saturating_sub(VISIBLE_ROWS)
                } else {
                    (*rows + VISIBLE_ROWS).min(max_scroll_rows())
                };
            }
            EditorAction::Brain(action) => self.editor_brain_action(action),
        }
    }

    /// `SaveSpriteOfBibite`: the previewer's sprites on one sheet (see `sprite_export.rs`), as `bibite_<time>.png` in the data folder (the game asks with the
    /// system's save dialog).
    fn editor_save_sprite(&mut self) {
        let Some(session) = self.bibite_editor.as_ref() else {
            return;
        };
        let Some(assets) = self.assets.as_ref() else {
            if let Some(session) = self.bibite_editor.as_mut() {
                session.view.status = "The sprites of the bibite are not available".to_owned();
            }
            return;
        };
        let genome = session.model.genome();
        let size = crate::preview::editor_size(&genome, self.procedural_size);
        let numbers = crate::preview::scale_of(assets, &genome, size, &crate::preview::EDITOR, crate::preview::ProceduralSize::Both);
        let Some(bytes) = crate::sprite_export::png(assets, &genome, numbers.size_index, crate::sprite_export::PIXEL_RATIO) else {
            if let Some(session) = self.bibite_editor.as_mut() {
                session.view.status = "The sprites of the bibite are not available".to_owned();
            }
            return;
        };
        let path = self.storage.root.join(format!("bibite_{}.png", crate::menu::file_stamp(std::time::SystemTime::now())));
        let status = match std::fs::write(&path, bytes) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(error) => format!("System.IO: {error}"),
        };
        self.last_action.clone_from(&status);
        if let Some(session) = self.bibite_editor.as_mut() {
            session.view.status = status;
        }
    }

    /// `SaveAndContinue` / `SaveAndSubmit`: the template goes to the templates folder; a submit closes the editor.
    fn editor_save(&mut self, close: bool) {
        let folder = self.storage.templates_dir();
        let pretty = self.format_bibite_json;
        // The buttons work while the name and the brain are valid (`UpdateValidity`).
        if !self
            .bibite_editor
            .as_ref()
            .is_some_and(crate::template_editor_ui::EditorSession::can_save)
        {
            return;
        }
        // `SaveTemplate`: `brainEditor.PrepareForTemplateSave()` first, then the template takes its nodes, connections and positions.
        if let Err(error) = self.editor_brain_into_template() {
            self.last_action = format!("could not save the template: {error}");
            if let Some(session) = self.bibite_editor.as_mut() {
                session.view.status = format!("Could not save: {error}");
            }
            return;
        }
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        // `anyChange || brainEditor.anyChange` (the brain's was handed to the model above).
        let changed = session.model.any_change;
        match session.model.save(&folder, pretty) {
            Ok((path, _source)) => {
                session.view.status = format!("Saved {}", path.display());
                self.last_action = format!("saved {}", path.display());
                if changed {
                    self.editor_saved_achievements();
                }
                let Some(session) = self.bibite_editor.as_mut() else {
                    return;
                };
                if close && session.submit {
                    // `submitAction(SaveTemplate())` is `AfterEditorSubmit`: the Library fills its lists again and selects the
                    // template that was saved; `ClosePanel` then finds no close listener left.
                    let name = session.model.name().to_owned();
                    self.close_template_editor();
                    self.library_refill = Some(Some(name));
                }
            }
            Err(error) => {
                session.view.status = format!("Could not save: {error}");
                self.last_action = format!("could not save the template: {error}");
            }
        }
    }

    /// The picture of the editor's template: its procedural bibite, `size` canvas units across, rendered at `pixels_per_unit` screen
    /// pixels per unit; `None` without the game's sprites.
    #[must_use]
    pub fn editor_picture(
        &self,
        size: [f32; 2],
        pixels_per_unit: f32,
    ) -> Option<std::sync::Arc<crate::ui::UiTexture>> {
        let assets = self.assets.as_ref()?;
        let session = self.bibite_editor.as_ref()?;
        let genome = session.model.genome();
        // `ProceduralGenePreviewer.UpdatePreview`: its own size, and the sprite tier is always the one closest to it.
        let request = crate::preview::PreviewRequest {
            genome: &genome,
            size: Some(crate::preview::editor_size(&genome, self.procedural_size)),
            style: crate::preview::EDITOR,
            procedural: crate::preview::ProceduralSize::Both,
            picture: size,
            pixels_per_unit,
        };
        self.previews.get(assets, &request)
    }

    /// A gene slider of the editor was pressed on its track (`gene`, the track's ends in reference px).
    pub fn editor_gene_press(&mut self, gene: u8, track: (f32, f32), cursor_x: f32) {
        if let Some(session) = self.bibite_editor.as_mut() {
            session.view.drag = Some((gene, track.0 as u16, track.1 as u16));
            session.view.focus = None;
        }
        self.editor_gene_slide(cursor_x);
    }

    /// The held slider follows the cursor (`Slider.UpdateDrag`).
    pub fn editor_gene_slide(&mut self, cursor_x: f32) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        let Some((gene, x0, x1)) = session.view.drag else {
            return;
        };
        let gene = usize::from(gene).min(bibites_core::genome::N_GENES - 1);
        let value = crate::template_editor_ui::value_at(
            gene,
            f32::from(x0),
            f32::from(x1),
            cursor_x / scale,
        );
        session.model.set_gene(gene, value);
    }

    /// Whether a gene slider of the editor is held.
    #[must_use]
    pub fn editor_gene_dragging(&self) -> bool {
        self.bibite_editor
            .as_ref()
            .is_some_and(|session| session.view.drag.is_some())
    }

    /// The held slider was let go.
    pub fn editor_gene_release(&mut self) {
        if let Some(session) = self.bibite_editor.as_mut() {
            session.view.drag = None;
        }
    }

    /// The mouse wheel over the editor scrolls the genes list a row a notch (up is positive); over the Brain tab it zooms the canvas
    /// and scrolls its lists (`cursor` in window px).
    pub fn editor_wheel(&mut self, cursor: Vec2, notches: f32) {
        use crate::template_editor_ui::{Tab, max_scroll_rows};
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        if session.view.tab == Tab::Brain {
            self.editor_brain_wheel(cursor, notches);
            return;
        }
        if session.view.tab != Tab::Genes || notches == 0.0 {
            return;
        }
        let rows = &mut session.view.scroll_rows;
        *rows = if notches > 0.0 {
            rows.saturating_sub(1)
        } else {
            (*rows + 1).min(max_scroll_rows())
        };
    }

    /// Typing into the editor's focused text field: characters and Backspace edit it, Ctrl + V pastes, Enter leaves the name field
    /// and breaks a line in the description.
    fn edit_editor_text(&mut self, input: &FrameInput) {
        use crate::template_editor_ui::{TextField, type_into};
        // The Brain tab's keys and clock, and what is typed into its label and filter fields.
        self.editor_brain_frame(input);
        if self.editor_brain_type(input) {
            return;
        }
        let keys = self.edit_keys(input);
        let Some(session) = self.bibite_editor.as_mut() else {
            return;
        };
        session.view.idle += input.dt;
        // With nothing focused the typed characters are left to the others (and dropped at the end of the frame).
        let Some(field) = session.view.focus else {
            return;
        };
        let typed = std::mem::take(&mut self.typed);
        let pasted = if input.paste_tag {
            self.clipboard_in.take().unwrap_or_default()
        } else {
            String::new()
        };
        let folder = self.storage.templates_dir();
        let exists = move |name: &str| folder.join(format!("{name}.bb8template")).is_file();
        let (model, view) = (&mut session.model, &mut session.view);
        let mut text = match field {
            TextField::Name => model.name().to_owned(),
            TextField::Description => model.description.clone(),
        };
        let before = text.clone();
        let owner = if field == TextField::Name { 4 } else { 5 };
        if self.edit_caret_owner != owner {
            self.edit_caret_owner = owner;
            self.edit_caret = crate::text_field::Caret::at_end(&text);
        }
        self.edit_caret.clamp(&text);
        // `type_into` decides which characters the field takes; the taken one goes in at the caret.
        let put = |text: &mut String, caret: &mut crate::text_field::Caret, character: char| {
            let mut probe = text.clone();
            type_into(&mut probe, field, character);
            if probe.chars().count() > text.chars().count()
                && let Some(taken) = probe.chars().last()
            {
                if taken == '\n' {
                    caret.insert_newline(text);
                } else {
                    caret.insert(text, &taken.to_string());
                }
            }
        };
        for character in typed.chars().chain(pasted.chars()) {
            put(&mut text, &mut self.edit_caret, character);
        }
        crate::text_field::edit(&mut text, &mut self.edit_caret, "", usize::MAX, keys);
        if input.enter {
            match field {
                TextField::Name => view.focus = None,
                TextField::Description => put(&mut text, &mut self.edit_caret, '\n'),
            }
        }
        if text != before {
            view.idle = 0.0;
            match field {
                TextField::Name => model.set_name(text, exists),
                TextField::Description => {
                    model.description = text;
                    model.any_change = true;
                }
            }
        }
    }

    /// Opens the genome editor on the selected bibite (or a starter genome).
    pub fn open_gene_editor(&mut self) {
        let genome = self
            .selected
            .and_then(|id| self.world.bibites.get(id))
            .map(|record| record.genome.clone())
            .unwrap_or_else(Genome::starter);
        self.editor_genome = Some(genome);
        self.editor_gene = 0;
        self.panels.open(crate::ui::PanelKind::GeneEditor);
        self.last_action = "genome editor opened".to_owned();
    }

    /// Opens the brain editor on the selected bibite (or a starter brain).
    pub fn open_brain_editor(&mut self) {
        let brain = self
            .selected
            .and_then(|id| self.world.bibites.get(id))
            .map(|record| record.brain.clone())
            .unwrap_or_else(Brain::starter);
        self.editor_node = 0;
        self.editor_prev_node = 0;
        self.editor_validation.clear();
        self.editor_brain = Some(brain);
        self.panels.open(crate::ui::PanelKind::BrainEditor);
        self.last_action = "brain editor opened".to_owned();
    }

    /// Rows `(name, value)` for the genome editor list.
    #[must_use]
    pub fn gene_editor_rows(&self) -> Vec<(String, f32)> {
        let Some(genome) = &self.editor_genome else {
            return Vec::new();
        };
        bibites_core::genome::ALL_GENES
            .iter()
            .enumerate()
            .map(|(index, gene)| (gene.def().display_name.to_owned(), genome.values()[index]))
            .collect()
    }

    fn adjust_editor_gene(&mut self, delta: i32) {
        let Some(genome) = self.editor_genome.as_mut() else {
            return;
        };
        let genes = bibites_core::genome::ALL_GENES;
        let index = self.editor_gene.min(genes.len() - 1);
        let gene = genes[index];
        let def = gene.def();
        let step = (def.max - def.min) * 0.01 * f32::from(delta as i16);
        let value = (genome.values()[index] + step).clamp(def.min, def.max);
        genome.set_gene(gene, value);
        self.last_action = format!("gene {} = {value:.3}", def.display_name);
    }

    fn editor_randomize_genome(&mut self) {
        if let Some(genome) = self.editor_genome.as_mut() {
            let mut rng = bibites_core::rng::Rng::from_seed(0x5EED_0001);
            genome.randomize_in_place(&mut rng);
            self.last_action = "genome randomized".to_owned();
        }
    }

    fn editor_mutate_gene(&mut self) {
        let Some(genome) = self.editor_genome.as_mut() else {
            return;
        };
        let genes = bibites_core::genome::ALL_GENES;
        let index = self.editor_gene.min(genes.len() - 1);
        let gene = genes[index];
        let mut rng = bibites_core::rng::Rng::from_seed(0x5EED_0002);
        let factor = bibites_core::genome::MutationFactor::default();
        let value = genome.mutate_gene(gene, &mut rng, &self.world.settings, &factor);
        self.last_action = format!("gene {} mutated -> {value:.3}", gene.def().display_name);
    }

    fn editor_add_node(&mut self) {
        if let Some(brain) = self.editor_brain.as_mut()
            && let Some(index) = brain.add_hidden_node(bibites_core::brain::NodeType::Sigmoid, None)
        {
            self.editor_node = index;
            self.last_action = format!("brain node added {index}");
        }
    }

    fn editor_remove_node(&mut self) {
        if let Some(brain) = self.editor_brain.as_mut()
            && brain.remove_hidden_node(self.editor_node)
        {
            // Indices shift after a removal; restart the selection.
            self.editor_node = 0;
            self.editor_prev_node = 0;
            self.last_action = "brain node removed".to_owned();
        }
    }

    fn editor_add_synapse(&mut self) {
        let Some(brain) = self.editor_brain.as_mut() else {
            return;
        };
        // Connect the previously selected node to the selected one (the game
        // adds a synapse from a node's panel to another clicked node).
        let (from, to) = (self.editor_prev_node, self.editor_node);
        if from == to {
            self.last_action = "select two different nodes first".to_owned();
            return;
        }
        match brain.add_synapse_connection(from, to, 0.5, true) {
            Some(index) => self.last_action = format!("synapse {from}->{to} added ({index})"),
            None => self.last_action = format!("cannot connect {from}->{to}"),
        }
    }

    /// Toggles the favourite flag of the species of the panel.
    fn species_toggle_favorite(&mut self) {
        self.species_set_favorite();
    }

    /// `SpeciesInfoPanel.SelectRandomOfSpecies`: closes the lineage panel and
    /// selects a random living member (needs at least one).
    fn species_select_random_member(&mut self) {
        let Some(entry) = self.species_info_entry() else {
            return;
        };
        let species = entry.id;
        let members: Vec<BibiteId> = self
            .world
            .bibites
            .iter()
            .filter(|(_, record)| !record.body.dead && record.meta.species == Some(species))
            .map(|(id, _)| id)
            .collect();
        if members.is_empty() {
            self.last_action = "species has no living members".to_owned();
            return;
        }
        let pick = members[(self.next_ui_random() % members.len() as u64) as usize];
        self.panels.close(crate::ui::PanelKind::Lineage);
        self.select_bibite(pick);
    }

    /// Edits the connection from the previously selected node to the selected
    /// one (`ConnectionEditPanel`: weight, enabled toggle, remove).
    fn editor_edit_synapse(&mut self, edit: SynapseEdit) {
        let (from, to) = (self.editor_prev_node, self.editor_node);
        let Some(brain) = self.editor_brain.as_mut() else {
            return;
        };
        let found = brain
            .graph_view()
            .synapses
            .into_iter()
            .find(|synapse| synapse.node_in == from && synapse.node_out == to);
        let Some(synapse) = found else {
            self.last_action = format!("no connection {from}->{to}");
            return;
        };
        match edit {
            SynapseEdit::Weight(delta) => {
                let weight = (synapse.weight + delta).clamp(-5.0, 5.0);
                if brain.set_synapse_weight(synapse.index, weight) {
                    self.last_action = format!("connection {from}->{to} weight {weight:.2}");
                }
            }
            SynapseEdit::Toggle => {
                if brain.set_synapse_enabled(synapse.index, !synapse.enabled) {
                    self.last_action = format!(
                        "connection {from}->{to} {}",
                        if synapse.enabled {
                            "disabled"
                        } else {
                            "enabled"
                        }
                    );
                }
            }
            SynapseEdit::Remove => {
                if brain.remove_synapse_at(synapse.index) {
                    self.last_action = format!("connection {from}->{to} removed");
                }
            }
        }
    }

    /// Cycles the selected hidden node through the activation functions
    /// (`NodeEditPanel` function dropdown).
    fn editor_cycle_node_type(&mut self) {
        let Some(brain) = self.editor_brain.as_mut() else {
            return;
        };
        let Some(current) = brain.node_type(self.editor_node) else {
            return;
        };
        let mut next = current as u8;
        loop {
            next = if next >= 13 { 1 } else { next + 1 };
            if let Some(kind) = bibites_core::brain::NodeType::from_u8(next) {
                if brain.set_hidden_node_type(self.editor_node, kind) {
                    self.last_action = format!("node {} -> {}", self.editor_node, kind.name());
                } else {
                    self.last_action = "only hidden nodes can change function".to_owned();
                }
                break;
            }
        }
    }

    /// Adjusts the selected node's bias by +/-0.25 within the editor's bounds
    /// (`NodeEditPanel`: Inhibitory 0.0001..10, SoftLatch 0.0001..1000,
    /// everything else -5..5).
    fn editor_adjust_bias(&mut self, direction: f32) {
        use bibites_core::brain::NodeType;
        let Some(brain) = self.editor_brain.as_mut() else {
            return;
        };
        let (Some(kind), Some(records)) = (
            brain.node_type(self.editor_node),
            Some(brain.to_node_records()),
        ) else {
            return;
        };
        let Some(record) = records.get(self.editor_node) else {
            return;
        };
        let (low, high) = match kind {
            NodeType::Inhibitory => (0.0001, 10.0),
            NodeType::SoftLatch => (0.0001, 1000.0),
            _ => (-5.0, 5.0),
        };
        let bias = (record.base_activation + 0.25 * direction).clamp(low, high);
        if brain.set_node_bias_at(self.editor_node, bias) {
            self.last_action = format!("node {} bias {bias:.2}", self.editor_node);
        }
    }

    fn editor_validate_brain(&mut self) {
        if let Some(brain) = &self.editor_brain {
            let validation = brain.validate_editor();
            self.editor_validation = if validation.errors.is_empty() {
                format!("valid ({} floating)", validation.floating_hidden.len())
            } else {
                validation.errors.join("; ")
            };
            self.last_action = self.editor_validation.clone();
        }
    }

    fn editor_save_template(&mut self) {
        let Some(brain) = self.editor_brain.clone() else {
            return;
        };
        let genome = self.editor_genome.clone().unwrap_or_else(Genome::starter);
        let mut loaded = bibites_io::LoadedGenome::starter_template("viewer-editor");
        loaded.genome = genome;
        loaded.brain = brain;
        for (innovation, (x, y)) in &self.editor_anchors {
            loaded.template.set_node_anchor(*innovation, *x, *y);
        }
        if let Err(error) = self.storage.ensure_layout() {
            self.last_action = format!("template save failed: {error}");
            return;
        }
        let path = self
            .storage
            .templates_dir()
            .join("viewer-editor.bb8template");
        let result = loaded
            .to_json()
            .map_err(|error| error.to_string())
            .and_then(|json| std::fs::write(&path, json).map_err(|error| error.to_string()));
        match result {
            Ok(()) => self.last_action = format!("template saved {}", path.display()),
            Err(error) => self.last_action = format!("template save failed: {error}"),
        }
    }

    fn editor_load_template(&mut self) {
        let path = self
            .storage
            .templates_dir()
            .join("viewer-editor.bb8template");
        match bibites_io::load_template(&path) {
            Ok(loaded) => {
                self.editor_genome = Some(loaded.genome);
                self.editor_brain = Some(loaded.brain);
                self.editor_anchors = loaded
                    .template
                    .node_anchors
                    .iter()
                    .map(|(innovation, anchor)| {
                        let (x, y) = anchor.position();
                        (*innovation, (x, y))
                    })
                    .collect();
                self.last_action = format!("template loaded {}", path.display());
            }
            Err(error) => self.last_action = format!("template load failed: {error}"),
        }
    }

    /// Anchors the selected brain node at a deterministic editor position.
    pub fn editor_anchor_node(&mut self) {
        let Some(brain) = &self.editor_brain else {
            return;
        };
        let graph = brain.graph_view();
        let Some(node) = graph.nodes.get(self.editor_node) else {
            return;
        };
        let innovation = node.innovation;
        let x = 30.0 + (self.editor_node % 14) as f32 * 55.0;
        let y = 60.0 + (self.editor_node / 14) as f32 * 44.0;
        self.editor_anchors.insert(innovation, (x, y));
        self.last_action = format!("anchored node {innovation}");
    }

    /// Opens the scenario editor with a working copy of the settings.
    pub fn open_scenario_editor(&mut self) {
        self.editor_settings = Some(self.world.settings.clone());
        self.editor_dirty = false;
        self.panels.open(crate::ui::PanelKind::ScenarioEditor);
        self.last_action = "scenario editor opened".to_owned();
    }

    /// Edits one setting key in the editor working copy.
    pub fn editor_set(&mut self, key: &str, value: bibites_core::settings::SettingValue) -> bool {
        let Some(settings) = self.editor_settings.as_mut() else {
            return false;
        };
        let applied = settings
            .independents
            .set_by_key(key, value)
            .or_else(|| settings.scenario.set_by_key(key, value));
        self.editor_dirty = applied.is_some();
        applied.is_some()
    }

    /// Applies the working copy; structural changes are staged until Save.
    pub fn apply_editor_settings(&mut self) {
        let Some(settings) = self.editor_settings.clone() else {
            return;
        };
        match self.world.apply_settings(settings) {
            Ok(()) => {
                self.editor_dirty = false;
                self.last_action = "editor settings applied".to_owned();
            }
            Err(error) => {
                self.last_action = format!("structural settings staged: {error}");
            }
        }
    }

    /// Persists the edited settings into the source scenario (in place).
    pub fn save_editor_scenario(&mut self) {
        let Some(path) = self.source_scenario.clone() else {
            self.last_action = "editor save: no source scenario".to_owned();
            return;
        };
        let Some(settings) = self.editor_settings.clone() else {
            return;
        };
        let result = (|| {
            let mut bundle =
                bibites_io::load_scenario_bundle(&path).map_err(|error| error.to_string())?;
            bundle.settings_file.scenario = settings.scenario.clone();
            bundle.settings_file.recommended = Some(settings.independents.clone());
            bundle.settings_file.materials = settings.materials;
            bibites_io::storage::update_scenario_settings(&path, &bundle.settings_file)
                .map_err(|error| error.to_string())
        })();
        match result {
            Ok(()) => {
                self.editor_dirty = false;
                self.last_action = format!("scenario saved {}", path.display());
            }
            Err(error) => self.last_action = format!("editor save failed: {error}"),
        }
    }

    /// Adds a default-ish zone to the world (scenario editor).
    pub fn editor_add_zone(&mut self) {
        let zone = bibites_core::settings::ZoneSettings {
            fertility: 1.0,
            biomass_density: 1.0,
            ..Default::default()
        };
        match self.world.add_zone(zone) {
            Ok(id) => {
                self.editor_dirty = true;
                self.last_action = format!("zone added {id}");
            }
            Err(error) => self.last_action = format!("add zone failed: {error}"),
        }
    }

    /// Removes the last zone (scenario editor); ids become stale afterwards.
    pub fn editor_remove_zone(&mut self) {
        let Some(zone) = self.world.zones.zones.last() else {
            return;
        };
        let id = zone.id;
        match self.world.remove_zone(id) {
            Ok(removed) => {
                self.editor_dirty = true;
                self.last_action = format!("zone removed ({removed} pellets cleared)");
            }
            Err(error) => self.last_action = format!("remove zone failed: {error}"),
        }
    }

    /// Applies fertility/biomass to a zone by list index (re-read after edits).
    #[allow(dead_code)] // TODO(wire): curated zone editor row uses this next
    pub fn editor_apply_zone(&mut self, index: usize, fertility: f32, density: f32) -> bool {
        let Some(zone) = self.world.zones.zones.get(index) else {
            return false;
        };
        let id = zone.id;
        let mut config = zone.config.0.clone();
        config.fertility = fertility.clamp(0.001, 100.0);
        config.biomass_density = density.clamp(0.001, 100.0);
        match self.world.apply_zone_settings(id, config) {
            Ok(()) => {
                self.editor_dirty = true;
                self.last_action = format!("zone {id} updated");
                true
            }
            Err(error) => {
                self.last_action = format!("zone update failed: {error}");
                false
            }
        }
    }

    /// Rebuilds the lineage tree snapshot used by the graph panel.
    pub fn refresh_lineage(&mut self) {
        // The members are counted again first: the registry keeps its counts at the tick boundary, and a world that was
        // just loaded (or is paused) has not had one, so every species would say it has none.
        self.world.reconcile_species_counts();
        self.lineage = Some(self.world.species.lineage_tree());
        self.lineage_species = self.world.species.species_list();
        self.species_selected = self
            .species_selected
            .min(self.lineage_species.len().saturating_sub(1));
        self.lineage_selected = None;
        // `LineageTreePanel.BuildTree`: the tree is laid out afresh and fits the view (or the mesh is made again).
        self.rebuild_lineage_view();
        self.last_action = "lineage refreshed".to_owned();
    }

    /// Selects a species list row and mirrors it into the lineage graph.
    pub fn select_species(&mut self, index: usize) {
        if index >= self.lineage_species.len() {
            return;
        }
        let id = self.lineage_species[index].id;
        self.select_species_info(Some(id));
        self.species_selected = index;
        self.last_action = format!("species {}", self.lineage_species[index].name);
    }

    /// Selected species entry, when available.
    #[cfg(test)]
    #[must_use]
    pub fn selected_species_entry(&self) -> Option<&bibites_core::species::SpeciesListEntry> {
        self.lineage_species.get(self.species_selected)
    }

    /// Selected lineage record (`species`, parents/children counts).
    #[cfg(test)]
    #[must_use]
    pub fn selected_lineage(&self) -> Option<bibites_core::species::LineageSelection> {
        let tree = self.lineage.as_ref()?;
        let index = self.lineage_selected?;
        let id = tree.nodes.get(index)?.id;
        self.world.species.selected_lineage(id)
    }

    /// Refreshes the cached save/load browser rows.
    pub fn refresh_save_entries(&mut self) {
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        // The fallback browser shows the metadata of its first rows only; a folder of a thousand autosaves must not be opened zip by zip on
        // every refresh (a freeze of seconds on F5, on Load Game and after a delete).
        match browser.refresh_save_entries_head(self.include_autosaves, SAVE_BROWSER_ROWS) {
            Ok(rows) => {
                self.save_entries = rows;
                self.save_selected = self
                    .save_selected
                    .min(self.save_entries.len().saturating_sub(1));
                self.last_action = format!("{} save(s)", self.save_entries.len());
            }
            Err(error) => self.last_action = format!("refresh saves failed: {error}"),
        }
    }

    /// Loads a save from the browser (panel row activation).
    pub fn load_save_path(&mut self, path: &std::path::Path) -> bool {
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        // `CheckTemplatesOfArchive`: the templates the save brings that the player does not have go to their folders.
        let added = browser.restore_missing_templates(path);
        // `LoadWorld`: a save older than the game can update from is refused with an error popup; one that needs updating is loaded with a warning.
        let version = bibites_io::storage::save_version(path);
        let compat = version.map(bibites_io::version::save_compat);
        if let (Some(version), Some(bibites_io::version::SaveCompat::Incompatible)) = (version, compat) {
            self.error_popups.push((
                "Save System".to_owned(),
                format!(
                    "This save file is from an earlier version. ({version}) that is incompatible with the present version ({}).",
                    bibites_io::version::PRESENT
                ),
            ));
            self.last_action = format!("load failed: save version {version} is incompatible");
            return false;
        }
        let contents = match browser.load_world(path) {
            Ok(contents) => contents,
            Err(error) => {
                self.last_action = format!("load failed: {error}");
                return false;
            }
        };
        match World::from_contents(contents) {
            Ok(mut world) => {
                // A loaded world starts serial: it keeps the worker count of the one it replaces.
                world.set_threads(self.world.threads());
                resume_spawner_of_loaded_world(&mut world, &self.storage);
                self.world = world;
                self.undo.clear();
                self.undo_drag = None;
                self.tail_states = initial_tail_states(&self.world);
                self.selected = None;
                self.selected_egg = None;
                self.selection = None;
                self.follow_selected = false;
                self.multi = None;
                self.selected_tool = None;
                self.tool_panel_open = false;
                self.tool_drag = None;
                self.placer = None;
                self.template_pick = None;
                self.placer_pause = None;
                self.forget_zone_editor();
                self.after_world_loaded();
                self.time_tags_after_load(path);
                self.refresh_overlay(1.0);
                self.last_action = format!("loaded {}", path.display());
                if let (Some(version), Some(bibites_io::version::SaveCompat::Updated)) = (version, compat) {
                    // The game raises a dialog: "...changes were made to the save files structure since this version. However it seems like it might be compatible...".
                    self.last_action = format!(
                        "{}; saved by {version}: the files' structure changed since, so it was updated and might still run into issues",
                        self.last_action
                    );
                }
                if !added.is_empty() {
                    // The game raises a dialog here ("This file contained one or more bibite templates that you did not posses...").
                    self.last_action = format!("{}; templates added to your folder: {}", self.last_action, added.join(", "));
                }
                true
            }
            Err(error) => {
                self.last_action = format!("load failed: {error}");
                false
            }
        }
    }

    /// `SimulationManager` (cs:69): `PauseAfterLoad` pauses the freshly loaded
    /// world.
    fn after_world_loaded(&mut self) {
        self.autosave_progress = 0.0;
        // The scene is built again: the Expanded Brain View is closed (its canvas place and graph type are the old scene's).
        self.close_expanded_brain();
        self.expanded_pan = [0.0, 0.0];
        self.expanded_graph = crate::expanded_brain::GraphType::Linear;
        self.expanded_settings_open = false;
        self.panels.close(crate::ui::PanelKind::ExpandedSettings);
        // A load issued from the pause menu (it stays open under the Load Game panel) ends both: the scene is built again and
        // `UINavigationManager.OnSceneChange` empties the Escape stack, so the menu's pause (a fresh `TimeController` runs) is not
        // given back to the old state when it closes.
        if self.panels.is_open(crate::ui::PanelKind::Escape) {
            self.panels.close(crate::ui::PanelKind::Escape);
            self.paused = false;
            self.accumulator = 0.0;
        }
        self.panels.close(crate::ui::PanelKind::LoadGame);
        self.escape_pause = None;
        if self.prefs.pause_after_load {
            self.paused = true;
            self.accumulator = 0.0;
        }
    }

    /// Deletes a save and refreshes the browser rows.
    pub fn delete_save_path(&mut self, path: &std::path::Path) -> bool {
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        match browser.delete_save(path, false) {
            Ok(_rows) => {
                self.refresh_save_entries();
                // The Save Game panel's list follows (`SaveGamePanel.DeleteSelectedSave`).
                self.save_panel_after_delete(path);
                self.last_action = "save deleted".to_owned();
                true
            }
            Err(error) => {
                self.last_action = format!("delete failed: {error}");
                false
            }
        }
    }

    /// Applies a value emitted by a captured toggle/slider/dropdown.
    pub fn apply_control_value(
        &mut self,
        control: &crate::ui_runtime::RuntimeControl,
        value: crate::ui::ControlValue,
    ) {
        self.apply_control_by_key(&control.session, &control.path, &control.label, value);
    }

    /// Opens a captured dropdown for item selection: its list hangs under the control (`scale` is
    /// the UI scale the control's rectangle is in px of).
    pub fn open_dropdown(&mut self, control: &crate::ui_runtime::RuntimeControl, scale: f32) {
        let options = control.option_labels().to_vec();
        if options.is_empty() {
            return;
        }
        let selected = control
            .value
            .as_ref()
            .and_then(|meta| meta.selected_index)
            .map_or(0, |index| index as usize);
        let scale = scale.max(f32::EPSILON);
        let anchor = control.rect.map(|value| value / scale);
        self.dropdown_open = Some(crate::dropdown::Open::new(
            crate::ui::DropdownTarget::Captured,
            anchor,
            options
                .iter()
                .map(|label| crate::dropdown::Choice::plain(label.clone()))
                .collect(),
            selected,
            crate::dropdown::ROBOTO,
        ));
        self.dropdown = Some(DropdownState {
            options,
            rect: control.rect,
            session: control.session.clone(),
            path: control.path.clone(),
            label: control.label.clone(),
        });
    }

    /// Picks an option from the open dropdown.
    pub fn pick_dropdown_choice(&mut self, index: u32) {
        let Some(dropdown) = self.dropdown.take() else {
            return;
        };
        self.apply_control_by_key(
            &dropdown.session,
            &dropdown.path,
            &dropdown.label,
            crate::ui::ControlValue::Choice(index),
        );
    }

    fn apply_control_by_key(
        &mut self,
        session: &str,
        path: &str,
        label: &str,
        value: crate::ui::ControlValue,
    ) {
        use crate::ui::ControlValue;
        let key = format!("{session} {path} {label}").to_ascii_lowercase();
        let flag = match value {
            ControlValue::Flag(flag) => Some(flag),
            ControlValue::Choice(choice) => Some(choice != 0),
            ControlValue::Integer(int) => Some(int != 0),
            _ => None,
        };
        let number = match value {
            ControlValue::Float(float) => Some(float),
            ControlValue::Integer(int) => Some(int as f32),
            ControlValue::Choice(choice) => Some(choice as f32),
            _ => None,
        };
        if key.contains("world_settings") || key.contains("dynamic_parameters") {
            self.apply_world_captured_setting(&key, label, value);
            return;
        }
        if key.contains("settings") && self.set_pref_by_key(&key, value) {
            return;
        }
        if self.editor_settings.is_some() {
            let structural =
                key.contains("simulationsize") || key.contains("size of the simulation");
            let editor_key = if key.contains("biomass") {
                Some("biomassDensity")
            } else if key.contains("fertility") || key.contains("pelletgrowth") {
                Some("pelletGrowth")
            } else if structural {
                Some("SimulationSize")
            } else {
                None
            };
            if let Some(editor_key) = editor_key {
                let number = match value {
                    ControlValue::Float(float) => Some(float),
                    ControlValue::Integer(int) => Some(int as f32),
                    ControlValue::Choice(choice) => Some(choice as f32),
                    _ => None,
                };
                if let Some(number) = number {
                    self.editor_set(
                        editor_key,
                        bibites_core::settings::SettingValue::Float(number),
                    );
                }
                self.last_action = format!("editor {editor_key} = {value:?}");
                return;
            }
        }
        if key.contains("grid") {
            if let Some(flag) = flag {
                self.scene_options.grid = flag;
            }
        } else if key.contains("pheromone") {
            if let Some(flag) = flag {
                self.scene_options.pheromones = flag;
            }
        } else if key.contains("pellet") {
            if let Some(flag) = flag {
                self.scene_options.pellets = flag;
            }
        } else if key.contains("zone") || key.contains("fertility") {
            if let Some(flag) = flag {
                self.scene_options.zones = flag;
            }
        } else if key.contains("hud") {
            if let Some(flag) = flag {
                self.show_hud = flag;
            }
        } else if key.contains("tps") || key.contains("speed") {
            if let Some(number) = number {
                // Through the world, which changes the tick's length with the setting (the clock and the bodies must agree).
                if let Err(error) = self.world.set_sim_tps(number.round().clamp(1.0, 1000.0) as u32) {
                    self.last_action = format!("{label}: {error}");
                    return;
                }
            }
        }
        self.last_action = format!("set {label} = {value:?}");
        self.refresh_overlay(1.0);
    }

    /// Quick-saves and restores through the local storage `saves` directory.
    #[allow(dead_code)] // TODO(wire): used by the save-browser tests
    fn quick_save_path(&self) -> PathBuf {
        self.storage.saves_dir().join("quick.zip")
    }

    fn load_quick_save(&mut self) {
        let browser = bibites_io::storage::SaveBrowser::new(self.storage.clone());
        match browser.quick_load() {
            Ok(contents) => match World::from_contents(contents) {
                Ok(mut world) => {
                    world.set_threads(self.world.threads());
                    resume_spawner_of_loaded_world(&mut world, &self.storage);
                    self.world = world;
                    self.undo.clear();
                    self.undo_drag = None;
                    self.tail_states = initial_tail_states(&self.world);
                    self.selected = None;
                    self.selected_egg = None;
                    self.selection = None;
                    self.follow_selected = false;
                    self.multi = None;
                    self.selected_tool = None;
                    self.tool_panel_open = false;
                    self.tool_drag = None;
                    self.placer = None;
                    self.template_pick = None;
                    self.placer_pause = None;
                    self.forget_zone_editor();
                    self.after_world_loaded();
                    let quick = self.quick_save_path();
                    self.time_tags_after_load(&quick);
                    self.refresh_overlay(1.0);
                    self.last_action = "loaded quick save".to_owned();
                }
                Err(error) => self.last_action = format!("load failed: {error}"),
            },
            Err(error) => self.last_action = format!("load failed: {error}"),
        }
    }

    /// Spawns a starter-template bibite at the camera centre (toolbar action).
    fn spawn_bibite_at_cursor(&mut self) {
        let pos = self.camera.center;
        let spawned = self
            .world
            .spawn_bibite_by_template_path(
                "starter",
                GrowthAtSpawn::Adult,
                Tagging::NoTagging,
                "",
                pos,
            )
            .or_else(|_| {
                // No named template in this world: clone the first living
                // bibite's template source (game "spawn from current" behaviour).
                let source = self
                    .world
                    .live_bibite_ids()
                    .first()
                    .copied()
                    .and_then(|id| self.world.bibite_template_source(id));
                match source {
                    Some(template) => self
                        .world
                        .spawn_template_at(
                            &template,
                            pos,
                            None,
                            GrowthAtSpawn::Adult,
                            Tagging::NoTagging,
                            "",
                        )
                        .map(|spawned| match spawned {
                            bibites_core::world::TemplateSpawn::Bibite(id) => id,
                            bibites_core::world::TemplateSpawn::Egg(_) => {
                                self.world.live_bibite_ids()[0]
                            }
                        }),
                    None => Err(bibites_core::Error::InvalidData(
                        "no local template or living bibite to copy".to_owned(),
                    )),
                }
            });
        match spawned {
            Ok(id) => {
                self.last_action = format!("spawned bibite {id}");
                self.refresh_overlay(1.0);
            }
            Err(error) => self.last_action = format!("spawn failed: {error}"),
        }
    }

    /// KILL inspector/toolbar action.
    fn kill_selected(&mut self) {
        // `GUIManager.TryKillTarget`: a selected colour killer is destroyed (the two towers have
        // no case there).
        if let Some(id) = self.selected_tool_id()
            && matches!(
                self.world.tools.get(id),
                Some(bibites_core::tools::Tool::Killer(_))
            )
        {
            self.remove_selected_tool();
            return;
        }
        // `GUIManager.TryKillTarget` / `BibiteStatsPanel.KillCurrentBibite`: a selected
        // egg is aborted; a living bibite dies and stays selected as a corpse (a
        // second kill explodes it into meat).
        if let Some(egg) = self.selected_egg
            && self.world.abort_egg(egg)
        {
            self.last_action = format!("aborted egg {egg}");
            self.refresh_overlay(1.0);
            return;
        }
        if let Some(id) = self.selected
            && self.world.kill_or_explode_bibite(id)
        {
            if self.world.bibites.get(id).is_none() {
                self.selected = None;
                self.selection = None;
                self.follow_selected = false;
            }
            self.last_action = format!("killed {id}");
            self.refresh_overlay(1.0);
        }
    }

    /// Applies/resets viewer+sim settings and persists them for the menu.
    fn persist_settings(&mut self, reset: bool) {
        if reset {
            self.scene_options = SceneOptions {
                grid: false,
                ..SceneOptions::default()
            };
            self.show_hud = true;
            // Through the world: the tick's length follows the setting.
            let _ = self.world.set_sim_tps(30);
        }
        let settings = crate::menu::MenuSettings {
            grid: self.scene_options.grid,
            zones: self.scene_options.zones,
            pellets: self.scene_options.pellets,
            pheromones: self.scene_options.pheromones,
            hud: self.show_hud,
            sim_tps: self.world.settings.sim_tps,
        };
        if let Some(parent) = Path::new("target").parent() {
            let _ = parent;
        }
        let settings_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("viewer-menu-settings.txt");
        match crate::menu::save_settings(&settings_path, settings) {
            Ok(()) => {
                self.last_action = if reset {
                    "settings reset"
                } else {
                    "settings applied"
                }
                .to_owned()
            }
            Err(error) => self.last_action = format!("settings save failed: {error}"),
        }
    }

    /// Applies the active tool at a world position (placer/tower/killer).
    pub fn apply_tool(&mut self, pos: Vec2) {
        match self.tool {
            crate::ui::ToolKind::Pellet => {
                let spawned = self.world.place_pellets_like_game(
                    self.tool_settings.material,
                    pos,
                    self.tool_settings.pellet_count,
                    self.tool_settings.pellet_energy,
                );
                self.pellet_last_placement = Some(pos);
                self.last_action = format!("placed {} pellet(s)", spawned.len());
            }
            // `EffectTower.Place`: the tower goes down where the cursor is, follows no zone (its
            // panel's dropdown sets one), is placed once (unlike the pellet painter) and becomes
            // the selected target, which opens its panel.
            crate::ui::ToolKind::Tower => {
                let id = self.world.place_rad_tower_at(pos, None);
                self.last_action = "placed radiation tower".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
                self.select_placed_tool(id);
            }
            crate::ui::ToolKind::PheroTower => {
                let id = self.world.place_phero_tower_at(pos, None);
                self.last_action = "placed pheromone tower".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
                self.select_placed_tool(id);
            }
            // `ColorKiller.Place` does not select the killer: its panel opens when it is clicked.
            crate::ui::ToolKind::Killer => {
                self.world.place_color_killer_at(pos, None);
                self.last_action = "placed colour killer".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
            }
            // RUSTibites Improved: a gene selector goes down where the cursor is and is selected, which opens its panel (a new one kills nobody until its range is narrowed).
            crate::ui::ToolKind::GeneSelector => {
                let id = self.world.place_gene_selector_at(pos);
                self.last_action = "placed gene selector".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
                self.select_placed_tool(id);
            }
            // RUSTibites Improved: a hydrothermal vent goes down where the cursor is and is selected, which opens its panel.
            crate::ui::ToolKind::Vent => {
                let id = self.world.place_vent_at(pos);
                self.last_action = "placed hydrothermal vent".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
                self.select_placed_tool(id);
            }
            // RUSTibites Improved: a force tower goes down where the cursor is and is selected, which opens its panel.
            crate::ui::ToolKind::Force => {
                let id = self.world.place_force_tower_at(pos);
                self.last_action = "placed force tower".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
                self.select_placed_tool(id);
            }
            // RUSTibites Improved: a whirlpool tower goes down where the cursor is and is selected, which opens its panel.
            crate::ui::ToolKind::Whirl => {
                let id = self.world.place_whirlpool_at(pos);
                self.last_action = "placed whirlpool tower".to_owned();
                self.tool = crate::ui::ToolKind::Inspect;
                self.select_placed_tool(id);
            }
            crate::ui::ToolKind::Inspect => {}
        }
        self.refresh_overlay(1.0);
    }

    /// Centres the camera on the selected bibite (selecting the first living
    /// one when nothing is selected). Used by `--center-on-selected`/`C`.
    pub fn center_on_selected(&mut self) -> bool {
        if self.selected.is_none() {
            self.select_first();
        }
        let Some(id) = self.selected else {
            return false;
        };
        let Some(record) = self.world.bibites.get(id) else {
            return false;
        };
        self.camera.center = record.pos;
        self.view_mode = ViewMode::Manual;
        true
    }

    /// Selects the most recently placed tower or colour killer and opens its panel (headless
    /// verification helper, `--select-tool`).
    pub fn select_last_tool(&mut self) -> bool {
        let Some(save_id) = self
            .world
            .tools
            .tools()
            .last()
            .map(bibites_core::tools::Tool::save_id)
        else {
            return false;
        };
        self.select_tool(save_id);
        self.selected_tool == Some(save_id)
    }

    /// Selects the first living bibite (headless demo/verification helper).
    pub fn select_first(&mut self) -> bool {
        self.multi = None;
        self.selected_egg = None;
        let id = self
            .world
            .bibites
            .iter()
            .find(|(_, record)| !record.is_corpse)
            .map(|(id, _)| id);
        self.selected = id;
        self.selection = id.and_then(|id| SelectionPanel::collect(&self.world, id));
        self.status_bar_scale = self.selection.as_ref().map_or(1.0, |panel| panel.size);
        id.is_some()
    }

    /// Advances the simulation by at most `frame_dt` seconds of wall time,
    /// scaled by the speed multiplier, using the world's fixed timestep.
    ///
    /// This is the *only* place the viewer ticks the world, and it calls
    /// [`World::tick`] with no other state changes, so playback is
    /// bit-identical to a headless `world.run(n)`.
    /// Smoothly approaches the wheel-zoom target while keeping the cursor anchor.
    fn update_zoom(&mut self, dt: f32) {
        let Some(target) = self.zoom_target else {
            return;
        };
        let current = self.camera.zoom;
        if !(current.is_finite() && current > 0.0 && target.is_finite() && target > 0.0) {
            self.zoom_target = None;
            return;
        }
        let next = crate::render::smooth_zoom(current, target, dt, 0.12);
        let settled = (next - target).abs() <= target * 1e-3;
        let applied = if settled { target } else { next };
        let factor = applied / current;
        self.camera.zoom_at(self.zoom_anchor, factor);
        if settled {
            self.zoom_target = None;
        }
    }

    pub fn advance(&mut self, frame_dt: f32) {
        // The ticks of the last frame may still be running on the helper (the window joins them before it gets here).
        self.join_overlapped_ticks();
        self.ticks_deferred = false;
        if self.world_away {
            self.advance_light(frame_dt);
            return;
        }
        // The player's easter egg settings reach whichever world is running (a loaded one has the defaults).
        self.world.settings.allow_easter_eggs = self.allow_easter_eggs;
        self.world.settings.easter_eggs_period = self.easter_eggs_period;
        // Real time passes whatever is simulated of it (`TimeKeeper` measures against it).
        self.wall_time += f64::from(frame_dt.max(0.0));
        // The pheromone particles follow the game's clock, frame by frame (the scene build does it too, for a world that runs elsewhere).
        crate::phero_particles::advance(&self.world, self.scene_options.pheromones, self.prefs.pheromone_opacity);
        crate::blood_particles::advance(&self.world);
        // `ZoneManager.CountBiomass(1f)`: the Information panel's numbers, once a second of real time.
        self.information.frame(
            self.panels.is_open(crate::ui::PanelKind::Information),
            &self.world,
            frame_dt,
        );
        // Clamp huge frame times (e.g. after dragging the window) so a single
        // stall cannot make the viewer simulate minutes in one frame.
        let dt = frame_dt.clamp(0.0, 0.25);
        // A selected tower that is gone (another world was loaded) leaves the selection.
        if self.selected_tool.is_some() && self.selected_tool_id().is_none() {
            self.clear_target();
        }
        self.update_ruler();
        // The Graphs and Lineage (species) panels pause the game while open, and so does the Library (`TogglePauseGame("BibiteLibrary")`;
        // while it picks a template for the placer the placer's own pause holds) with the Bibite Editor over it.
        let panel_wants_pause = self.panels.is_open(crate::ui::PanelKind::Graphs)
            || self.panels.is_open(crate::ui::PanelKind::Lineage)
            || (self.panels.is_open(crate::ui::PanelKind::Library)
                && self.template_pick != Some(TemplateUse::PlaceBibite))
            || self.panels.is_open(crate::ui::PanelKind::BibiteEditor)
            // `ExpandedBrainPanel.OpenPanel`: `TogglePauseGame("ExpendedBrainPanel")`.
            || self.expanded_brain.is_some();
        if panel_wants_pause && self.panel_pause.is_none() {
            self.panel_pause = Some(self.paused);
            self.paused = true;
            self.accumulator = 0.0;
        } else if !panel_wants_pause && let Some(previous) = self.panel_pause.take() {
            self.paused = previous;
            self.accumulator = 0.0;
        }
        // Closing the Escape panel (by any route) restores the previous pause.
        if self.escape_pause.is_some() && !self.panels.is_open(crate::ui::PanelKind::Escape) {
            if let Some(previous) = self.escape_pause.take() {
                self.paused = previous;
                self.accumulator = 0.0;
            }
        }
        self.update_zoom(dt);
        self.hud_timer += dt;
        if self.hud_timer >= HUD_REFRESH_SECS {
            self.hud_timer = 0.0;
            self.hud = HudInfo::collect(&self.world);
            self.selection = self.collect_selection();
        }
        self.sync_inspector_brain();
        if self.paused {
            // `WaitForSeconds` does not advance while the game is paused.
            self.update_target(0.0);
            // A body held by the pointer or placed by a tool moves while paused: the camera still follows it.
            self.follow_target_camera();
            // `TimeKeeper.Update` notes the pause and keeps the numbers it has.
            self.time_keeper.render_frame(self.wall_time, true);
            self.achievements_frame(frame_dt, false);
            return;
        }
        let fixed = f64::from(self.world.settings.fixed_dt());
        let engine = self.engine_speed();
        self.accumulator += f64::from(dt) * f64::from(engine);
        if self.overlap_worth_it(fixed) {
            // The owed ticks run on the helper thread while the window draws and presents this frame
            // ([`App::dispatch_overlapped_ticks`]); they stay in the accumulator until then. What follows in this method is about
            // the world as it is now, which is the world this frame draws.
            self.ticks_deferred = true;
        } else {
            // The player's minimum FPS allows a frame `1 / min_fps` seconds; what the last frame spent besides its ticks (drawing,
            // the UI, waiting for the display) is spent again, and what is left is the room for this frame's ticks. The game keeps
            // its frames by running at most one tick a frame (`maximumDeltaTime = fixedDeltaTime`) and by lowering the time scale;
            // here several ticks may run in a frame, and without this a world whose ticks are slow would owe more ticks after every
            // long frame (the frame time is what is added to the accumulator) and the frames would grow without bound.
            let room = (self.prefs.min_fps >= 1).then(|| {
                let rest = (f64::from(frame_dt.max(0.0)) - self.last_tick_secs).max(0.0);
                (1.0 / f64::from(self.prefs.min_fps) - rest).max(0.0)
            });
            let mut spent = 0.0_f64;
            let mut steps = 0_u32;
            while self.accumulator >= fixed && steps < MAX_CATCHUP_TICKS {
                // One tick always runs when one is owed; more only while the next is expected to fit the room.
                if steps >= 1 && room.is_some_and(|room| spent + self.tick_cost > room) {
                    break;
                }
                let started = std::time::Instant::now();
                self.tick_world();
                self.step_autopan(self.world.settings.fixed_dt());
                self.accumulator -= fixed;
                self.sim_ticks += 1;
                // `TimeKeeper.FixedUpdate`: every tick that really ran counts towards the ticks per second.
                self.time_keeper
                    .fixed_tick(self.wall_time, self.world.settings.fixed_dt());
                steps += 1;
                let took = self.tick_seconds(started.elapsed());
                spent += took;
                self.tick_cost = if self.tick_cost == 0.0 {
                    took
                } else {
                    0.7 * self.tick_cost + 0.3 * took
                };
            }
            self.last_tick_secs = spent;
            if self.accumulator >= fixed {
                // The cap or the room is used up: drop the backlog instead of spiralling.
                self.accumulator = 0.0;
            }
        }
        self.update_target(dt * engine);
        // `CameraManager.LateUpdate`: after the frame's physics ticks, on the target as they left it.
        self.follow_target_camera();
        // `TimeKeeper.Update` hands each measured frame rate to `TimeController.CheckMinFPS`.
        if let Some(present_fps) = self.time_keeper.render_frame(self.wall_time, false) {
            self.check_min_fps(present_fps);
        }
        self.fps = self.time_keeper.fps().round() as i32;
        self.achievements_frame(frame_dt, true);
    }

    /// Whether this frame's owed ticks go to the tick helper: always, whatever the size of the world (a world of a few bibites
    /// ticks in microseconds and gains nothing from running beside the drawing, but the hand-off costs next to nothing either),
    /// unless nothing else may have the world, none is owed, or no tick has been timed yet (the first frame runs its ticks
    /// itself: the frame pacing needs what a tick costs to tell how many fit the room).
    fn overlap_worth_it(&self, fixed: f64) -> bool {
        self.overlap_ticks
            && self.sim.is_none()
            && !self.world_away
            && !self.ticks_in_flight
            && self.accumulator >= fixed
            && self.tick_cost > 0.0
    }

    /// Tests: the cost every tick counts as in the frame pacing (a model machine), if one was set.
    fn model_tick(&self) -> Option<f64> {
        #[cfg(test)]
        {
            self.model_tick_secs
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    /// The window calls this when it begins a frame (after [`App::join_overlapped_ticks`]).
    pub fn begin_frame(&mut self) {
        self.frame_started = Some(std::time::Instant::now());
    }

    /// Hands this frame's owed ticks to the helper thread (see [`crate::tick_overlap`]), to run while the frame is drawn and
    /// presented. Call it after everything that reads the world in the frame (the display list, the UI) and before the drawing;
    /// [`App::join_overlapped_ticks`] takes the world back after the frame is presented, and nothing may read `world` in between.
    /// Does nothing when `advance` ran the ticks itself or none are owed.
    pub fn dispatch_overlapped_ticks(&mut self) {
        if !std::mem::take(&mut self.ticks_deferred) || self.ticks_in_flight {
            return;
        }
        let fixed = f64::from(self.world.settings.fixed_dt());
        let owed = ((self.accumulator / fixed).floor().max(0.0) as u32).min(MAX_CATCHUP_TICKS);
        if owed == 0 {
            return;
        }
        // The player's minimum FPS allows a frame `1 / min_fps` seconds. What the frame did before this point (input, the UI's
        // routing, the display list, the UI) is serial with the ticks; the drawing and the wait for the display run beside them,
        // so only the first is taken off the room.
        let before = self
            .frame_started
            .map_or(0.0, |started| started.elapsed().as_secs_f64());
        // A frame whose drawing and display take longer than that (`overlap_hidden`) has the ticks for as long as they last: they run beside it and
        // hold nothing up, and a window that is slow to draw (4K) must not get the simulation less than a small one.
        let room = (self.prefs.min_fps >= 1).then(|| {
            (1.0 / f64::from(self.prefs.min_fps) - before)
                .max(if legacy_pacing() { 0.0 } else { self.overlap_hidden })
                .max(0.0)
        });
        let husk = self
            .world_husk
            .take()
            .unwrap_or_else(|| crate::sim_thread::husk_like(&self.world));
        let real = std::mem::replace(&mut self.world, husk);
        let batch = crate::tick_overlap::Batch {
            world: Box::new(real),
            owed,
            room,
            tick_cost: self.tick_cost,
            model_tick_secs: self.model_tick(),
        };
        self.tick_worker
            .get_or_insert_with(crate::tick_overlap::TickWorker::spawn)
            .start(batch);
        self.ticks_in_flight = true;
        self.overlap_dispatched = Some(std::time::Instant::now());
    }

    /// Takes the world back from the tick helper, waiting for the ticks it is running, and does what `advance` does for every tick
    /// that ran (the counters, the time keeper's ticks per second, the auto-panner, the accumulator). Does nothing when no ticks
    /// are out. Returns the ticks that ran. The window calls it right after `next_frame`; every method that reads the world calls
    /// it first as well, so that a world on the helper can never be read.
    pub fn join_overlapped_ticks(&mut self) -> u32 {
        if !self.ticks_in_flight {
            return 0;
        }
        let beside = self
            .overlap_dispatched
            .take()
            .map_or(0.0, |at| at.elapsed().as_secs_f64());
        let waiting_since = std::time::Instant::now();
        let finished = self
            .tick_worker
            .as_mut()
            .expect("ticks are out, so the helper exists")
            .finish();
        let waited = waiting_since.elapsed().as_secs_f64();
        let (beside, waited) = self
            .model_overlap(finished.spent)
            .unwrap_or((beside, waited));
        self.note_overlap(beside, waited);
        self.ticks_in_flight = false;
        let husk = std::mem::replace(&mut self.world, *finished.world);
        self.world_husk = Some(husk);
        let fixed = self.world.settings.fixed_dt();
        for _ in 0..finished.ticks {
            self.step_autopan(fixed);
            self.sim_ticks += 1;
            // `TimeKeeper.FixedUpdate`: every tick that really ran counts towards the ticks per second.
            self.time_keeper.fixed_tick(self.wall_time, fixed);
        }
        self.accumulator -= f64::from(finished.ticks) * f64::from(fixed);
        if self.accumulator >= f64::from(fixed) {
            // The cap or the room is used up: drop the backlog instead of spiralling.
            self.accumulator = 0.0;
        }
        self.last_tick_secs = finished.spent;
        self.tick_cost = finished.tick_cost;
        self.joined_ticks += finished.ticks;
        finished.ticks
    }

    /// What a batch of ticks that ran beside `beside` seconds of drawing and display, and made the frame wait `waited` seconds for them, tells the pacing.
    fn note_overlap(&mut self, beside: f64, waited: f64) {
        // (One stall is not what the display usually takes.)
        let beside = beside.min(0.5);
        self.overlap_hidden = if self.overlap_hidden == 0.0 {
            beside
        } else if beside > self.overlap_hidden {
            0.5 * self.overlap_hidden + 0.5 * beside
        } else {
            0.8 * self.overlap_hidden + 0.2 * beside
        };
        self.overlap_wait_secs += waited;
        self.overlap_batches += 1;
    }

    /// Tests: a model machine's `(beside, waited)` for a batch of ticks that took `spent` seconds, if the machine has a model display.
    fn model_overlap(&self, spent: f64) -> Option<(f64, f64)> {
        #[cfg(test)]
        {
            if self.model_tick_secs.is_some() {
                if let Some(beside) = self.model_beside_secs {
                    return Some((beside, (spent - beside).max(0.0)));
                }
            }
        }
        let _ = spent;
        None
    }

    /// The frame rate `TimeController.CheckMinFPS` is asked about after a measurement of `present_fps`. The game slows the simulation when the frames fall under
    /// the minimum because its ticks and its drawing take turns, so fewer ticks mean quicker frames. Here the ticks run beside the drawing: when the frames of the
    /// window did not wait for them (the drawing and the display held the frame up, a window of 4K on a modest graphics card), fewer ticks would not make the
    /// frames quicker and only slowed the simulation down to a tenth, so the rule is asked as if the frame rate were twice the minimum: it takes nothing off the
    /// time scale and raises it towards the target again. When the frames did wait for the ticks the measured frame rate goes to the rule as it is.
    fn frame_rate_for_governor(&mut self, present_fps: f32) -> f32 {
        let window = (self.wall_time - self.govern_since).max(1e-3);
        let batches = std::mem::take(&mut self.overlap_batches);
        let waited = std::mem::take(&mut self.overlap_wait_secs);
        self.govern_since = self.wall_time;
        let minimum = self.prefs.min_fps as f32;
        if !legacy_pacing() && batches > 0 && minimum >= 1.0 && present_fps < minimum && waited < 0.05 * window {
            return 2.0 * minimum;
        }
        present_fps
    }

    /// The ticks the joins have brought in since the last call (the frame's challenge and autosave clocks count them).
    pub fn take_joined_ticks(&mut self) -> u32 {
        std::mem::take(&mut self.joined_ticks)
    }

    /// `CameraManager.LateUpdate`: while a target is followed the camera is put on its position, whatever it is (a bibite, an egg, a tool, a zone, the
    /// auto-panner). Unity runs `LateUpdate` after the frame's `FixedUpdate`s (the ticks) and before the picture is drawn, so the camera ends every frame on
    /// the position the target has in the picture. (The follow used to run BEFORE the frame's ticks: on every frame that ran a tick the target was drawn one
    /// tick's movement off the centre and on the frames without one exactly on it, which showed as a vibration of the locked bibite, the tester's "vibrates
    /// when you lock onto a bibite".)
    fn follow_target_camera(&mut self) {
        if !self.follow_selected || self.zoom_target.is_some() {
            return;
        }
        let target = self
            .selected
            .and_then(|id| self.world.bibites.get(id).map(|record| record.pos))
            .or_else(|| {
                self.selected_egg
                    .and_then(|id| self.world.eggs.get(id).map(|egg| egg.pos))
            })
            .or_else(|| {
                self.selected_tool_id()
                    .and_then(|id| self.world.tools.get(id))
                    .map(|tool| tool.base().pos)
            })
            .or_else(|| {
                self.selected_zone
                    .and_then(|index| self.world.zones.zones.get(index))
                    .map(|zone| zone.pos)
            })
            .or_else(|| self.selected_autopan.then_some(self.autopan.pos));
        match target {
            Some(pos) => {
                self.camera.center = pos;
                self.view_mode = ViewMode::Manual;
            }
            None => self.follow_selected = false,
        }
    }

    /// `AchievementManager.Trigger(id)`: unlocks an achievement that is not unlocked yet and shows its popup.
    pub fn trigger_achievement(&mut self, id: &str) {
        // The popup is the notice; `last_action` is the action's own text.
        self.achievements.trigger(id);
    }

    /// One frame of the achievements: the popup's life (real time, also while paused; the pointer on it keeps it) and, while the game
    /// runs, what `TimeKeeper.Update` checks (a minute below 1.5 frames a second, 168 hours of simulated time) and what the bibites'
    /// own updates check (a day of age, 100 children, a newborn with 100 hidden nodes: looked at once a second here).
    pub(crate) fn achievements_frame(&mut self, real_dt: f32, running: bool) {
        let scale = crate::ui::UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let (x, y) = (self.cursor.x / scale, self.cursor.y / scale);
        let hovered = self
            .achievements
            .shown_at
            .get()
            .is_some_and(|rect| x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3]);
        self.achievements.tick(real_dt, hovered);
        if !running {
            return;
        }
        // `TimeKeeper.Update`: `timeScale > 0 && fps < 1.5 && !firstUpdate` counts real time; any other frame starts over.
        if self.time_keeper.fps() < 1.5 && !self.time_keeper.first_update() {
            self.achievements.seconds_below_one_fps += real_dt;
            self.achievements.frames_below_one_fps += 1;
            if self.achievements.seconds_below_one_fps >= 60.0
                && self.achievements.frames_below_one_fps > 10
            {
                self.trigger_achievement("ACH_SIM_1FPS");
            }
        } else {
            self.achievements.seconds_below_one_fps = 0.0;
            self.achievements.frames_below_one_fps = 0;
        }
        if !self.achievements.triggered_seven_days {
            let hours = crate::time_keeper::parse_time(self.simulated_time())[0];
            if (168..170).contains(&hours) {
                self.achievements.triggered_seven_days = true;
                self.trigger_achievement("ACH_SIM_7DAYS");
            }
        }
        if self.world_away {
            return;
        }
        self.achievements.scan_timer += real_dt;
        if self.achievements.scan_timer < 1.0 {
            return;
        }
        self.achievements.scan_timer = 0.0;
        // `InternalClock`: `timeAlive` of a living bibite past 86 400 s; `BibiteEggLayingOrgan`: 100 children.
        let (mut elder, mut matriarch) = (false, false);
        for (_, record) in self.world.bibites.iter() {
            if record.body.dead {
                continue;
            }
            elder |= record.clock.time_alive >= 86_400.0;
            matriarch |= record.children.len() >= 100;
            if elder && matriarch {
                break;
            }
        }
        if elder {
            self.trigger_achievement("ACH_BIBITE_1DAY");
        }
        if matriarch {
            self.trigger_achievement("ACH_BIBITE_100CHILDREN");
        }
        // `BibiteBody.Born`: a natural birth with 100 hidden nodes.
        if self.world.take_einstein_births() > 0 {
            self.trigger_achievement("ACH_BIBITE_EINSTEIN");
        }
    }

    /// What `BibiteEditorPanel.SaveTemplate` triggers after a save with a change: the first one, and with the hundredth (`savedBibiteCount.Trigger()
    /// > 100`) the second.
    pub(crate) fn editor_saved_achievements(&mut self) {
        self.trigger_achievement("ACH_BIBITE_EDITOR");
        let count = self.achievements.count("BibiteEditorSaved");
        if count > 100 {
            self.trigger_achievement("ACH_BIBITE_EDITOR_100");
        }
    }

    /// What a challenge's result earns (`ChallengesProgress.TrySetHighScoreOfChallenge`: an achievement for every star attained).
    pub(crate) fn challenge_stars_achievements(&mut self, challenge: &str, stars: u8) {
        for level in 1..=stars.min(3) {
            self.trigger_achievement(&crate::achievements::challenge_id(challenge, level));
        }
    }

    /// What a tick that took `elapsed` counts as in the frame pacing (tests may give every tick a model cost).
    fn tick_seconds(&self, elapsed: std::time::Duration) -> f64 {
        #[cfg(test)]
        if let Some(model) = self.model_tick_secs {
            return model;
        }
        elapsed.as_secs_f64()
    }

    /// Refreshes overlay data without advancing the simulation (used after a
    /// single-step and before a screenshot).
    pub fn refresh_overlay(&mut self, dt: f32) {
        self.join_overlapped_ticks();
        self.hud_timer += dt;
        self.hud = HudInfo::collect(&self.world);
        self.selection = self.collect_selection();
        self.sync_inspector_brain();
    }

    /// The inspector snapshot of the current bibite or egg target (`None` without one).
    fn collect_selection(&self) -> Option<SelectionPanel> {
        match (self.selected, self.selected_egg) {
            (Some(id), _) => SelectionPanel::collect(&self.world, id),
            (None, Some(egg)) => SelectionPanel::collect_egg(&self.world, egg),
            (None, None) => None,
        }
    }

    /// Rebuilds the display list from the current world/camera state.
    /// Advances the render-only tails exactly once per simulated world tick.
    fn step_tails(&mut self) {
        let dt = self.world.settings.fixed_dt();
        let world = &self.world;
        let states = &mut self.tail_states;
        states.retain(|id, _| world.bibites.get(*id).is_some());
        for (id, record) in world.bibites.iter() {
            let up = Vec2::Y.rotate(record.rot);
            let d1 = record.body.geometry.d1_size;
            let state = states
                .entry(id)
                .or_insert_with(|| TailState::new(record.pos, up, d1));
            state.step(record.pos, up, d1, dt);
            // `BibiteBody.Hurting` / `CorpseUpdate` / `Update`: a hit that took `dmg` of `health` sets the flash to `min(2 dmg / health, 1)` (it
            // only ever goes up), a corpse flashes by how little of it is left, and the flash falls by 2 a second.
            let health = record.body.health.amount;
            if state.last_health > 0.0 && health < state.last_health {
                let taken = state.last_health - health;
                state.hurt = state.hurt.max((2.0 * taken / state.last_health).min(1.0));
            }
            state.last_health = health;
            if record.body.dead {
                state.hurt = state.hurt.max((1.0 - record.body.health_ratio()).clamp(0.0, 1.0).powi(8));
            }
            state.hurt = (state.hurt - dt / 0.5).max(0.0);
        }
    }

    pub fn rebuild_scene(&mut self) {
        self.join_overlapped_ticks();
        if self.panels.is_open(crate::ui::PanelKind::Lineage) {
            self.sync_lineage_canvas();
        }
        if self.world_away {
            // The simulation thread builds it (see `drawn_scene`).
            return;
        }
        self.scene.tool_view = self.tool_view();
        let options = self.draw_options();
        build_scene(
            &mut self.scene,
            &self.world,
            &self.camera,
            self.selected,
            &options,
            self.assets.as_ref(),
            Some(&self.tail_states),
        );
    }

    /// The overlay switches the scene is built with: the view switches of `scene_options` and the "Show Targets" one (`show_targets`).
    fn draw_options(&self) -> SceneOptions {
        SceneOptions {
            targets: self.show_targets,
            pheromone_strength: self.prefs.pheromone_opacity,
            ..self.scene_options
        }
    }

    /// Renders the current frame with the software rasteriser and writes a PNG.
    ///
    /// Used by the `F12` screenshot hotkey (window mode) and by the CLI
    /// `--screenshot` path. When `path` is `None` the file is
    /// `target/viewer-<unix-seconds>.png`. Returns the written path.
    pub fn save_screenshot(&mut self, path: Option<&Path>) -> Result<PathBuf, String> {
        let width = self.camera.screen.x.round().max(1.0) as u32;
        let height = self.camera.screen.y.round().max(1.0) as u32;
        let path = path.map(Path::to_path_buf).unwrap_or_else(|| {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            PathBuf::from("target").join(format!("viewer-{seconds}.png"))
        });
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        self.rebuild_scene();
        let atlases: &[crate::assets::Atlas] = self
            .assets
            .as_ref()
            .map_or(&[], |assets| assets.atlases.as_slice());
        let image = render_scene(&self.scene, &self.camera, width, height, 2, atlases);
        image
            .write_png(&path)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok(path)
    }

    /// Simulation time in seconds.
    #[must_use]
    pub fn simulated_time(&self) -> f64 {
        match (&self.snapshot, self.world_away()) {
            (Some(snapshot), true) => snapshot.sim_time,
            _ => self.world.simulated_time(),
        }
    }

    /// Current simulation tick number (world clock).
    #[must_use]
    pub fn world_tick(&self) -> u64 {
        match (&self.snapshot, self.world_away()) {
            (Some(snapshot), true) => snapshot.tick,
            _ => self.world.clock.tick(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::sim::state_fingerprint;

    fn synthetic(seed: u64, count: usize) -> LoadedWorld {
        LoadedWorld::load(None, seed, count, 1).expect("starter world")
    }

    #[test]
    fn app_playback_matches_headless_execution() {
        // Two identical worlds: one driven through the viewer's accumulator, the
        // other ticked directly. Bit-identical fingerprints prove the viewer
        // never perturbs the simulation (no RNG use, no dt changes).
        let mut app = App::new(synthetic(9_001, 24), Vec2::new(1280.0, 800.0), None);
        app.speed = 3.0;
        for _ in 0..120 {
            app.advance(1.0 / 60.0);
        }
        let ticks = app.sim_ticks;
        assert!(ticks > 0, "the viewer must have simulated something");

        let loaded = synthetic(9_001, 24);
        let mut headless = loaded.world;
        headless.run(ticks);

        assert_eq!(app.simulated_time(), headless.simulated_time());
        assert_eq!(
            state_fingerprint(&app.world),
            state_fingerprint(&headless),
            "viewer playback diverged from headless execution"
        );
    }

    #[test]
    fn selecting_a_bibite_and_opening_its_panels_leaves_the_simulation_bit_identical() {
        // The suspicion: "every bibite I open the panels for has an increased energy cost, so it starves quickly". Two identical worlds
        // run through the viewer's frame loop (the input, then the frame's ticks): one left alone, the other with its first bibite
        // selected (the camera follows it), each of the four inspector panels opened in turn and the pointer wandering over the world and
        // the panels. Nothing the viewer does for a selection may reach the world: the selected bibite's energy, and every other
        // number of the state fingerprint, are the same bits in both worlds at every check.
        let size = Vec2::new(1280.0, 800.0);
        let mut plain = App::new(synthetic(9_201, 24), size, None);
        let mut watched = App::new(synthetic(9_201, 24), size, None);
        plain.speed = 3.0;
        watched.speed = 3.0;
        assert!(watched.select_first());
        let target = watched.selected.expect("a selected bibite");
        let energy_of = |app: &App| app.world.bibites.get(target).map(|record| record.body.energy.amount.to_bits());
        assert!(energy_of(&plain).is_some(), "the same bibite lives in both worlds");
        for frame in 0..800_u32 {
            if frame % 100 == 0 {
                watched.open_bibite_panel(InspectorTab::ALL[(frame / 100) as usize % 4], false);
            }
            let phase = frame as f32 * 0.05;
            let cursor = Vec2::new(640.0 + 500.0 * phase.cos(), 400.0 + 300.0 * (phase * 1.3).sin());
            for app in [&mut plain, &mut watched] {
                app.handle_input(&FrameInput {
                    cursor,
                    dt: 1.0 / 60.0,
                    ..FrameInput::default()
                });
                app.advance(1.0 / 60.0);
            }
            if frame % 50 == 0 {
                assert_eq!(plain.sim_ticks, watched.sim_ticks, "frame {frame}");
                assert_eq!(energy_of(&plain), energy_of(&watched), "the selected bibite's energy, frame {frame}");
                assert_eq!(state_fingerprint(&plain.world), state_fingerprint(&watched.world), "the world, frame {frame}");
            }
        }
        assert!(plain.sim_ticks > 500, "the worlds ran: {} ticks", plain.sim_ticks);
        assert_eq!(state_fingerprint(&plain.world), state_fingerprint(&watched.world));
    }

    #[test]
    fn a_press_on_a_sliders_handle_keeps_the_handle_where_it_was_and_one_on_the_track_moves_it_there() {
        // The game's sliders are Unity `Slider`s: `OnPointerDown` on the handle only records the pointer's offset from it (nothing moves until the
        // pointer does, then the handle follows the pointer's travel), and anywhere else on the track the handle jumps under the pointer. A 1080p
        // window: one screen px is one reference px.
        let mut app = App::new(synthetic(9_301, 6), Vec2::new(1920.0, 1080.0), None);
        // The time-speed slider: its track is 845..1075 reference px (a log scale).
        let track = (845.0, 1075.0);
        let span = track.1 - track.0;
        app.speed = 1.0;
        let handle = track.0 + App::speed_fraction(1.0) * span;
        let press = handle + 8.0;
        app.begin_slider_hold(app.speed_slider_grab(track, press), press);
        assert!((app.slider_grab - 8.0).abs() < 1e-3, "{}", app.slider_grab);
        app.speed_slide(track, press);
        assert_eq!(app.speed, 1.0, "the press itself moves nothing");
        app.speed_slide(track, press + 23.0);
        let followed = (App::speed_fraction(app.speed) - (handle + 23.0 - track.0) / span).abs();
        assert!(followed < 1e-3, "the handle moves by the pointer's travel, whatever the offset was: {followed}");
        // A press far from the handle is on the track: the handle goes under the pointer from the first call.
        let far = handle + 150.0;
        assert_eq!(app.speed_slider_grab(track, far), None);
        app.begin_slider_hold(app.speed_slider_grab(track, far), far);
        assert_eq!(app.slider_grab, 0.0);
        app.speed_slide(track, far);
        let jumped = (App::speed_fraction(app.speed) - (far - track.0) / span).abs();
        assert!(jumped < 1e-3, "{jumped}");

        // A settings row's slider the same way: the handle is where the setting's value puts it on the row's track.
        use crate::setting_ui_data::{SETTING_UI, SettingUiKind};
        let index = (0..SETTING_UI.len())
            .find(|&index| {
                SETTING_UI[index].kind != SettingUiKind::Bool
                    && crate::dyn_panel::slide_value(&mut app.world.settings.clone(), index, (0.0, 100.0), 50.0, app.tps_defaults)
            })
            .expect("a slider row");
        let track = (0.0, 100.0);
        assert!(crate::dyn_panel::slide_value(&mut app.world.settings, index, track, 40.0, app.tps_defaults));
        let original = app.world.settings.clone();
        let row = &SETTING_UI[index];
        let (mode, _) = crate::dyn_panel::slider_info_or_default(row);
        let handle = 100.0
            * crate::dyn_panel::fraction_of(
                mode,
                row,
                crate::dyn_panel::default_of(row, app.tps_defaults),
                crate::dyn_panel::read_value(&app.world.settings, row),
            );
        let press = handle + 6.0;
        app.begin_slider_hold(app.dyn_slider_grab(index, track, press), press);
        app.dyn_slide(index, track, press);
        assert_eq!(app.world.settings, original, "a press on the handle moves nothing");
        app.dyn_slide(index, track, press + 20.0);
        let mut expected = original.clone();
        assert!(crate::dyn_panel::slide_value(&mut expected, index, track, handle + 20.0, app.tps_defaults));
        let (got, want) = (
            crate::dyn_panel::read_value(&app.world.settings, row),
            crate::dyn_panel::read_value(&expected, row),
        );
        assert!(
            (got - want).abs() <= 1e-3 * want.abs().max(1.0),
            "the setting follows the handle, which followed the pointer's travel: {got} against {want}"
        );
        assert_ne!(got, crate::dyn_panel::read_value(&original, row), "and it did move");
    }

    #[test]
    fn a_hit_flashes_the_skin_and_the_flash_falls_two_a_second() {
        let mut app = App::new(synthetic(9_004, 6), Vec2::new(1280.0, 800.0), None);
        app.step_tails();
        let id = app.world.bibites.iter().map(|(id, _)| id).next().expect("a bibite");
        assert_eq!(app.tail_states[&id].hurt, 0.0);
        // A hit that takes a quarter of the health: the flash is twice that, then it falls by dt / 0.5 a tick.
        let health = app.world.bibites.get(id).expect("record").body.health.amount;
        app.world.bibites.get_mut(id).expect("record").body.health.amount = health * 0.75;
        app.step_tails();
        let dt = app.world.settings.fixed_dt();
        let flash = app.tail_states[&id].hurt;
        assert!((flash - (0.5 - dt / 0.5)).abs() < 1e-4, "{flash}");
        for _ in 0..40 {
            app.step_tails();
        }
        assert_eq!(app.tail_states[&id].hurt, 0.0, "it has fallen away");
    }

    #[test]
    fn the_ticks_per_second_can_be_edited_in_the_middle_of_a_simulation() {
        let mut app = App::new(synthetic(9_006, 12), Vec2::new(1280.0, 800.0), None);
        let index = crate::setting_ui_data::SETTING_UI
            .iter()
            .position(|row| row.field == "simTPS")
            .expect("the Ticks Per Second row");
        let field = crate::dyn_panel::FieldRef::Setting(u16::try_from(index).expect("a small index"));
        assert_eq!(app.world.clock.tps(), 30);
        // Two seconds at the world's 30 ticks a second, speed 1, 60 frames a second.
        for _ in 0..120 {
            app.advance(1.0 / 60.0);
        }
        assert!((app.time_keeper.tps() - 30.0).abs() < 2.5, "tps {}", app.time_keeper.tps());
        let (tick, time) = (app.world.clock.tick(), app.world.simulated_time());
        // The number typed into the row (the game's slider has a number box too): 60.
        app.apply_typed_value(field, 60.0);
        assert!(app.last_action.contains("updated"), "{}", app.last_action);
        assert_eq!((app.world.clock.tps(), app.world.settings.sim_tps), (60, 60));
        assert_eq!((app.world.clock.tick(), app.world.simulated_time()), (tick, time), "the edit itself moves nothing");
        // Two more seconds: twice the ticks a second and still one simulated second per real second (the time panel's `sim/real`).
        for _ in 0..120 {
            app.advance(1.0 / 60.0);
        }
        let keeper = &app.time_keeper;
        assert!((keeper.tps() - 60.0).abs() < 4.0, "tps {}", keeper.tps());
        assert!((keeper.sim_seconds_per_second() - 1.0).abs() < 0.1, "sim/real {}", keeper.sim_seconds_per_second());
        let made = app.world.simulated_time() - time;
        assert!((made - 2.0).abs() < 0.1, "{made} simulated seconds in two real ones");
        // Down to 15 (a slider drag or a reset lands in the same place): half the ticks a second.
        app.apply_typed_value(field, 15.0);
        for _ in 0..120 {
            app.advance(1.0 / 60.0);
        }
        assert_eq!(app.world.clock.tps(), 15);
        assert!((app.time_keeper.tps() - 15.0).abs() < 2.0, "tps {}", app.time_keeper.tps());
    }

    #[test]
    fn the_time_panel_shows_the_ticks_and_frames_that_really_ran() {
        let mut app = App::new(synthetic(9_002, 24), Vec2::new(1280.0, 800.0), None);
        let sim_tps = app.world.settings.sim_tps as f32;
        // Speed 1 at 60 frames per second for three seconds: the simulation's own rate, one simulated second per real second.
        for _ in 0..180 {
            app.advance(1.0 / 60.0);
        }
        let keeper = &app.time_keeper;
        assert!((keeper.fps() - 60.0).abs() < 2.5, "fps {}", keeper.fps());
        assert!(
            (keeper.tps() - sim_tps).abs() < 2.5,
            "tps {} of {sim_tps}",
            keeper.tps()
        );
        assert!(
            (keeper.sim_seconds_per_second() - 1.0).abs() < 0.1,
            "sim/real {}",
            keeper.sim_seconds_per_second()
        );
        assert_eq!(app.fps, keeper.fps().round() as i32);
        // Speed 3: three times the ticks, three simulated seconds per real second.
        app.speed = 3.0;
        for _ in 0..180 {
            app.advance(1.0 / 60.0);
        }
        let keeper = &app.time_keeper;
        assert!(
            (keeper.tps() - 3.0 * sim_tps).abs() < 6.0,
            "tps {} of {}",
            keeper.tps(),
            3.0 * sim_tps
        );
        assert!(
            (keeper.sim_seconds_per_second() - 3.0).abs() < 0.25,
            "sim/real {}",
            keeper.sim_seconds_per_second()
        );
        // Paused: the numbers stay what they were (the game does not zero them).
        let before = (keeper.fps(), keeper.tps(), keeper.sim_seconds_per_second());
        app.paused = true;
        for _ in 0..120 {
            app.advance(1.0 / 60.0);
        }
        let keeper = &app.time_keeper;
        assert_eq!(
            before,
            (keeper.fps(), keeper.tps(), keeper.sim_seconds_per_second())
        );
        // The text of the panel carries them.
        let text = crate::time_keeper::time_panel_text(app.simulated_time(), keeper);
        assert!(text.contains(&format!("{:.0} tps", keeper.tps())), "{text}");
    }

    #[test]
    fn the_time_panel_shows_the_rate_the_viewer_manages_not_the_speed_asked_for() {
        // Speed 100 in frames of a tenth of a second asks for 300 ticks a frame; the catch-up cap lets 32 run, 320 a second:
        // the panel says so (and not 100 x 30 = 3000, nor the setting of 30).
        let mut app = App::new(synthetic(9_003, 8), Vec2::new(800.0, 600.0), None);
        // Without a minimum FPS only the catch-up cap limits the ticks of a frame.
        app.prefs.min_fps = 0;
        app.speed = 100.0;
        for _ in 0..60 {
            app.advance(0.1);
        }
        let keeper = &app.time_keeper;
        assert!((keeper.tps() - 320.0).abs() < 20.0, "tps {}", keeper.tps());
        let expected = keeper.tps() * app.world.settings.fixed_dt();
        assert!((keeper.sim_seconds_per_second() - expected).abs() < 1e-3);
    }

    #[test]
    fn a_world_too_slow_for_the_minimum_fps_is_slowed_down_and_the_frames_are_kept() {
        let mut app = App::new(synthetic(9_004, 8), Vec2::new(800.0, 600.0), None);
        app.prefs.min_fps = 15;
        // The machine of the model: a tick costs 20 ms (at most 50 a second, 1.7 simulated seconds a second at 30 ticks a
        // second) and the rest of a frame 5 ms, against the speed 10 asked for.
        app.model_tick_secs = Some(0.020);
        app.set_target_speed(10.0);
        let mut dt = 0.005_f32;
        let mut frames = Vec::new();
        for frame in 0..900 {
            let before = app.sim_ticks;
            app.advance(dt);
            let ticks = (app.sim_ticks - before) as f32;
            dt = 0.005 + 0.020 * ticks;
            if frame >= 600 {
                frames.push(dt);
            }
        }
        let fps = app.time_keeper.fps();
        assert!(
            (12.0..=19.0).contains(&fps),
            "fps {fps}: the frames should be held near the minimum of 15"
        );
        let tps = app.time_keeper.tps();
        assert!((30.0..=50.0).contains(&tps), "tps {tps}");
        assert_eq!(app.speed, 10.0, "the slider keeps what was asked for");
        // What the panel says: the simulated seconds per real second the machine really manages, nowhere near the 10 asked for.
        let measured = app.time_keeper.sim_seconds_per_second();
        assert!((0.9..=1.8).contains(&measured), "sim/real {measured}");
        // Over the last stretch no frame was much longer than the minimum frame time.
        assert!(frames.iter().all(|dt| *dt < 0.1), "frame times {frames:?}");
    }

    #[test]
    fn a_fast_machine_runs_at_the_speed_asked_for() {
        let mut app = App::new(synthetic(9_005, 8), Vec2::new(800.0, 600.0), None);
        app.prefs.min_fps = 15;
        app.set_target_speed(3.0);
        for _ in 0..300 {
            app.advance(1.0 / 60.0);
        }
        assert_eq!(app.engine_speed(), 3.0);
        // Without a minimum nothing is ever slowed, whatever the frames.
        let mut slow = App::new(synthetic(9_006, 8), Vec2::new(800.0, 600.0), None);
        slow.prefs.min_fps = 0;
        slow.set_target_speed(5.0);
        for _ in 0..40 {
            slow.advance(0.2);
        }
        assert_eq!(slow.engine_speed(), 5.0);
    }

    /// An app on a model machine (a tick costs `tick_secs`) that runs its frames' ticks beside the drawing when asked to.
    fn overlap_app(seed: u64, bibites: usize, overlap: bool, tick_secs: f64, min_fps: u32) -> App {
        let mut app = App::new(synthetic(seed, bibites), Vec2::new(800.0, 600.0), None);
        app.prefs.min_fps = min_fps;
        app.model_tick_secs = Some(tick_secs);
        app.overlap_ticks = overlap;
        app
    }

    #[test]
    fn ticks_run_beside_the_drawing_are_the_ticks_the_loop_would_have_run() {
        let mut plain = overlap_app(9_010, 40, false, 0.010, 0);
        let mut beside = overlap_app(9_010, 40, true, 0.010, 0);
        plain.set_target_speed(3.0);
        beside.set_target_speed(3.0);
        let mut handed = 0;
        for frame in 0..60 {
            plain.advance(0.05);
            beside.advance(0.05);
            beside.begin_frame();
            beside.dispatch_overlapped_ticks();
            handed += u32::from(beside.ticks_in_flight);
            beside.join_overlapped_ticks();
            assert_eq!(beside.sim_ticks, plain.sim_ticks, "frame {frame}");
        }
        assert!(handed > 50, "only {handed} of the frames' ticks went to the helper");
        assert!(beside.sim_ticks > 100, "{} ticks", beside.sim_ticks);
        // The same ticks, one after the other: the same world.
        assert_eq!(
            state_fingerprint(&beside.world),
            state_fingerprint(&plain.world)
        );
        assert_eq!(beside.world_tick(), plain.world_tick());
        assert!((beside.accumulator - plain.accumulator).abs() < 1e-9);
        // The time panel counted every tick that ran.
        assert!((beside.time_keeper.tps() - plain.time_keeper.tps()).abs() < 1e-3);
    }

    #[test]
    fn a_world_on_the_helper_is_home_before_anything_reads_it() {
        let mut app = overlap_app(9_011, 40, true, 0.010, 0);
        app.set_target_speed(3.0);
        // The first frame has no cost to go by and runs its ticks itself.
        app.advance(0.05);
        assert!(!app.ticks_deferred);
        app.advance(0.05);
        assert!(app.ticks_deferred, "the owed ticks wait for the dispatch");
        let ticks = app.sim_ticks;
        app.dispatch_overlapped_ticks();
        assert!(app.ticks_in_flight);
        assert_eq!(app.world.bibites.len(), 0, "a husk stands in while the ticks run");
        // Whatever reads the world brings it home first: the scene build, the input, the next frame.
        app.rebuild_scene();
        assert!(!app.ticks_in_flight);
        assert!(app.world.bibites.len() > 0);
        assert!(app.sim_ticks > ticks);
        for read in 0..3 {
            app.advance(0.05);
            app.dispatch_overlapped_ticks();
            assert!(app.ticks_in_flight);
            match read {
                0 => app.handle_input(&FrameInput::default()),
                1 => app.refresh_overlay(0.0),
                _ => app.advance(0.05),
            }
            assert!(!app.ticks_in_flight, "read {read}");
            assert!(app.world.bibites.len() > 0, "read {read}");
        }
    }

    #[test]
    fn the_ticks_beside_the_drawing_get_the_frames_room_less_what_came_before_them() {
        use std::time::{Duration, Instant};
        // A tick costs 20 ms; the minimum of 15 frames a second gives a frame 66.7 ms.
        let mut app = overlap_app(9_012, 40, true, 0.020, 15);
        app.set_target_speed(100.0);
        app.advance(0.25);
        assert!(!app.ticks_deferred, "no cost to go by yet");
        // Nothing came before the ticks: 3 ticks fit (20, 40, 60 ms).
        app.advance(0.25);
        app.frame_started = Some(Instant::now());
        let before = app.sim_ticks;
        app.dispatch_overlapped_ticks();
        app.join_overlapped_ticks();
        assert_eq!(app.sim_ticks - before, 3);
        // The frame spent 40 ms before the ticks started: 26.7 ms are left, one tick (the first always runs).
        app.advance(0.25);
        app.frame_started = Instant::now().checked_sub(Duration::from_millis(40));
        let before = app.sim_ticks;
        app.dispatch_overlapped_ticks();
        app.join_overlapped_ticks();
        assert_eq!(app.sim_ticks - before, 1);
        // Only what came before the ticks is taken off the room (the drawing and the wait for the display run beside them): a frame
        // that spent 5 ms before them leaves room for three.
        app.advance(0.25);
        app.frame_started = Instant::now().checked_sub(Duration::from_millis(5));
        let before = app.sim_ticks;
        app.dispatch_overlapped_ticks();
        app.join_overlapped_ticks();
        assert_eq!(app.sim_ticks - before, 3);
    }

    #[test]
    fn a_cheap_world_hands_its_ticks_over_as_well() {
        // Multithreading always runs: the ticks of a world of eight bibites (20 microseconds each) go to the helper too, from the
        // second frame on (the first one has no cost to go by).
        let mut app = overlap_app(9_013, 8, true, 0.000_02, 0);
        app.set_target_speed(3.0);
        app.advance(0.05);
        assert!(!app.ticks_deferred, "no cost to go by yet");
        let mut handed = 0;
        for frame in 0..20 {
            app.advance(0.05);
            assert!(app.ticks_deferred, "frame {frame}");
            app.dispatch_overlapped_ticks();
            handed += u32::from(app.ticks_in_flight);
            app.join_overlapped_ticks();
        }
        assert_eq!(handed, 20);
        assert!(app.sim_ticks > 50);
    }

    /// One frame of a model machine in the window: the frame's owed ticks, the hand-over, the drawing and the display taking `beside` seconds beside the ticks (a
    /// big window on a modest graphics card), the join. Returns the frame's length (nothing came before the ticks in the model): the longer of the two.
    fn model_window_frame(app: &mut App, frame_dt: f64, beside: f64) -> f64 {
        app.model_beside_secs = Some(beside);
        app.advance(frame_dt as f32);
        app.begin_frame();
        app.dispatch_overlapped_ticks();
        app.join_overlapped_ticks();
        beside.max(app.last_tick_secs)
    }

    /// Runs `frames` frames of a model machine and returns (simulated seconds per real second over the second half, the frame rate over it).
    fn model_window_speed(app: &mut App, frames: usize, beside: f64) -> (f64, f64) {
        let (mut dt, mut time) = (0.01_f64, 0.0_f64);
        let (mut ticks_then, mut time_then) = (0_u64, 0.0_f64);
        for frame in 0..frames {
            if frame == frames / 2 {
                (ticks_then, time_then) = (app.sim_ticks, time);
            }
            dt = model_window_frame(app, dt, beside);
            time += dt;
        }
        let fixed = f64::from(app.world.settings.fixed_dt());
        let seconds = time - time_then;
        ((app.sim_ticks - ticks_then) as f64 * fixed / seconds, (frames - frames / 2) as f64 / seconds)
    }

    /// The testers' "at 4K it does not let you go fast, but as soon as the window is smaller it goes super fast (4,200 bibites at 3x)": the ticks run beside the drawing
    /// and the display, so a window whose drawing takes long (a 4K window on a modest graphics card: 10 frames a second and less) costs the simulation nothing as long as
    /// the ticks fit in the drawing. The minimum-FPS rule used to slow the simulation to a tenth below 13 frames a second as if the ticks held the frames up (they did
    /// in the game, where ticks and drawing take turns), and the ticks' room was a frame of the minimum's length whatever the frame really took.
    #[test]
    fn a_slow_display_does_not_slow_the_simulation_down() {
        for beside in [0.005_f64, 0.04, 0.08, 0.2] {
            // A tick takes 4 ms: 3x of 30 ticks a second is 90 ticks, 0.36 s of every second.
            let mut app = overlap_app(9_020, 40, true, 0.004, 15);
            app.set_target_speed(3.0);
            let (speed, fps) = model_window_speed(&mut app, 1_200, beside);
            assert!(speed > 2.85, "drawing and display {} ms ({fps:.1} frames a second): the simulation ran at x{speed:.2}", beside * 1000.0);
            assert_eq!(app.engine_speed(), 3.0, "drawing and display {} ms: the time scale was lowered", beside * 1000.0);
        }
    }

    /// The rule still works when the ticks ARE what holds the frames up: 20 ms a tick, a quick display and 10x asked for.
    #[test]
    fn ticks_that_outlast_the_drawing_still_hold_the_frame_rate_near_the_minimum() {
        let mut app = overlap_app(9_021, 40, true, 0.020, 15);
        app.set_target_speed(10.0);
        let (speed, fps) = model_window_speed(&mut app, 1_200, 0.005);
        assert!((12.0..=19.0).contains(&fps), "frames a second {fps:.1}: held near the minimum of 15");
        // 20 ms a tick is at most 50 ticks a second, 1.7 simulated seconds a second at 30; the asked-for 10x is far out of reach.
        assert!((0.9..=1.8).contains(&speed), "simulated seconds per second {speed:.2}");
        assert_eq!(app.speed, 10.0, "the slider keeps what was asked for");
    }

    /// A time scale the rule lowered while the ticks held the frames up comes back when they no longer do (a slow display, ticks that fit in its drawing).
    #[test]
    fn a_time_scale_lowered_for_slow_ticks_recovers_under_a_slow_display() {
        let mut app = overlap_app(9_022, 40, true, 0.004, 15);
        app.set_target_speed(3.0);
        app.engine_fraction = 0.2;
        let (speed, _) = model_window_speed(&mut app, 1_200, 0.1);
        assert!(speed > 2.85, "x{speed:.2}");
        assert_eq!(app.engine_speed(), 3.0);
    }

    #[test]
    fn pause_and_single_step_advance_exactly_one_tick() {
        let mut app = App::new(synthetic(5, 8), Vec2::new(800.0, 600.0), None);
        app.handle_input(&FrameInput {
            toggle_pause: true,
            ..FrameInput::default()
        });
        assert!(app.paused);
        let tick_before = app.world_tick();
        app.handle_input(&FrameInput {
            step: true,
            ..FrameInput::default()
        });
        assert_eq!(app.world_tick(), tick_before + 1);
        app.advance(1.0);
        assert_eq!(app.world_tick(), tick_before + 1, "paused world advanced");
    }

    #[test]
    fn catch_up_is_capped() {
        let mut app = App::new(synthetic(6, 8), Vec2::new(800.0, 600.0), None);
        // Without a minimum FPS only the cap limits the ticks of a frame (with one, the frame's room does too).
        app.prefs.min_fps = 0;
        app.speed = 8.0;
        // A 250 ms frame at 8× speed would be 60 ticks; the cap is 32.
        app.advance(0.25);
        assert!(app.sim_ticks <= u64::from(MAX_CATCHUP_TICKS));
        assert!(app.sim_ticks > 0);
    }

    #[test]
    fn clicking_selects_the_nearest_bibite() {
        let mut app = App::new(synthetic(7, 8), Vec2::new(800.0, 600.0), None);
        let (id, record) = app.world.bibites.iter().next().expect("spawned bibites");
        let screen = app.camera.world_to_screen(record.pos);
        app.handle_input(&FrameInput {
            cursor: screen,
            primary_pressed: true,
            primary_down: true,
            primary_released: true,
            ..FrameInput::default()
        });
        assert_eq!(app.selected, Some(id));
        let panel = app.selection.as_ref().expect("selection panel");
        assert_eq!(panel.target, SelectionTarget::Bibite(id));
        assert!(panel.brain_nodes > 0);
    }

    #[test]
    fn scene_culls_and_lods_distant_bodies() {
        use crate::render::{GRID_AXIS, GRID_LINE, Primitive, SceneOptions};

        let mut app = App::new(synthetic(3, 120), Vec2::new(1280.0, 800.0), None);
        let anchor = app
            .world
            .bibites
            .iter()
            .next()
            .expect("spawned bibite")
            .1
            .pos;

        // Zoomed in hard on one bibite: bodies outside the view are culled.
        app.camera = Camera::new(anchor, 8.0, Vec2::new(1280.0, 800.0));
        app.rebuild_scene();
        let visible_capsules = app
            .scene
            .primitives
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Capsule { .. }))
            .count();
        assert!(visible_capsules >= 1, "anchor bibite must be in view");
        assert!(
            visible_capsules < app.world.bibites.len(),
            "culling did not remove off-screen bibites ({} of {})",
            visible_capsules,
            app.world.bibites.len()
        );

        // Grid on/off changes the primitive count (and grid lines are lines).
        let counts = |grid: bool| {
            let mut scene = Scene::new();
            build_scene(
                &mut scene,
                &app.world,
                &app.camera,
                None,
                &SceneOptions {
                    grid,
                    ..SceneOptions::default()
                },
                None,
                None,
            );
            scene.primitives
        };
        let with_grid = counts(true);
        let without_grid = counts(false);
        assert!(
            with_grid.len() > without_grid.len(),
            "grid toggle did not add primitives"
        );
        assert!(with_grid.iter().any(|primitive| matches!(
            primitive,
            Primitive::Line { color, .. } if *color == GRID_LINE || *color == GRID_AXIS
        )));

        // Zoomed all the way out: organ detail is LOD'd away, so no mouth bars
        // (grid disabled, so the scene contains no line primitives at all).
        let mut far = Scene::new();
        build_scene(
            &mut far,
            &app.world,
            &Camera::new(anchor, 0.05, Vec2::new(1280.0, 800.0)),
            None,
            &SceneOptions {
                zones: false,
                zone_colors: false,
                pellets: false,
                pheromones: false,
                grid: false,
                fov: false,
                targets: false,
                pheromone_strength: 0.1,
                objects: true,
            },
            None,
            None,
        );
        assert!(
            !far.primitives
                .iter()
                .any(|primitive| matches!(primitive, Primitive::Line { .. })),
            "tiny bodies should not draw organ detail"
        );
    }

    #[test]
    fn screenshot_renders_a_non_trivial_world() {
        // Headless proof that rendering works in CI: build a starter world, run
        // it for two simulated seconds, rasterise one frame without a display and
        // assert the image contains real content (bodies, plants, zone tints).
        let mut app = App::new(synthetic(12_345, 64), Vec2::new(960.0, 600.0), None);
        for _ in 0..120 {
            app.advance(1.0 / 60.0);
        }
        app.refresh_overlay(0.0);
        app.rebuild_scene();
        assert!(
            app.scene.len() > 8,
            "scene should contain world/zone/sprite content, got {}",
            app.scene.len()
        );

        let image = crate::render::render_scene(&app.scene, &app.camera, 960, 600, 2, &[]);
        let stats = image.stats();
        assert_eq!((stats.width, stats.height), (960, 600));
        assert!(
            stats.non_background > 150_000,
            "image looks blank: {stats:?}"
        );
        assert!(stats.distinct_colors > 20, "image is flat: {stats:?}");
        assert!(stats.green > 300, "no plant/zone greens: {stats:?}");
        assert!(stats.bright > 200, "no bright bodies/eggs: {stats:?}");
        assert!(stats.mean_luma > 4.0, "image is too dark: {stats:?}");

        let path = screenshot_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create target dir");
        }
        image.write_png(&path).expect("write screenshot PNG");
        let bytes = std::fs::metadata(&path).expect("screenshot exists").len();
        assert!(bytes > 10_000, "PNG is suspiciously small: {bytes} bytes");
        eprintln!("viewer screenshot written to {}", path.display());
    }

    /// `target/viewer-screenshot.png`, resolved from the crate directory (where
    /// `cargo test` runs).
    fn screenshot_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("viewer-screenshot.png")
    }
}

#[cfg(test)]
mod sprite_renderer_tests {
    //! Headless regression test for the game-asset sprite path.
    //!
    //! Game artwork never enters the repository, so this test synthesises a
    //! minimal `assets/game`-shaped asset set under `target/` (white masks,
    //! game-format CSVs and manifest), loads it through the real
    //! [`crate::assets::GameAssets`] loader and renders a frame. It proves the
    //! whole pipeline (manifest → CSV slicing → gene→sprite indices → layered
    //! composition → both raster backends) works in CI with no game data.

    use std::path::{Path, PathBuf};

    use crate::app::{
        App, ConfirmKind, DropdownState, FrameInput, LoadedWorld, Scene, SelectionTarget,
    };
    use crate::ui::{InspectorTab, UiAction};
    use bibites_core::ids::SpeciesId;
    use bibites_core::math::Vec2;
    use bibites_io::storage::StoragePaths;

    fn synthetic(seed: u64, count: usize) -> LoadedWorld {
        LoadedWorld::load(None, seed, count, 1).expect("starter world")
    }

    use crate::assets::{GameAssets, PART_PPU_DIVISOR};
    use crate::render::{BACKGROUND, Camera, Primitive, SceneOptions, build_scene, render_scene};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("viewer-sprite-test-assets")
    }

    fn write_text(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    /// Writes a solid RGBA PNG (the synthetic sheets are plain masks).
    fn write_png(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let file = std::fs::File::create(path).unwrap();
        let writer = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let pixels = color.repeat((width * height) as usize);
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }

    fn build_fixture() -> GameAssets {
        let root = fixture_root();
        let _ = std::fs::remove_dir_all(&root);
        // Text assets live in `text/` to avoid the Windows case collision
        // between the game's `Bibites/parts` TextAsset and `Bibites/Parts/`
        // sprite folder (documented for the extraction script).
        write_text(
            &root.join("text/bibites_sizes.csv"),
            "name;pixelWidth\nsmall;100\n",
        );
        write_text(
            &root.join("text/egg_sizes.csv"),
            "name;pixelWidth\nsmall;110\n",
        );
        write_text(
            &root.join("text/pellet_sizes.csv"),
            "name;pixelWidth\nsmall;100\n",
        );
        write_text(
            &root.join("text/bibites_parts.csv"),
            "name;nRows;nColumns\n\
             small_body;2;1\nsmall_mouth;1;2\nsmall_eyes;2;2\nsmall_eyes_dead;2;2\n\
             small_arms;1;2\nsmall_exoskeleton;1;2\n",
        );
        write_text(
            &root.join("text/parameters.csv"),
            "name;value\nInitialHue;0.55\nVminAtS0;0\nVmaxAtS0;1\n",
        );
        let sheet = |name: &str, width: u32, height: u32| {
            let path = root.join("Bibites/Parts").join(format!("{name}.png"));
            write_png(&path, width, height, [255, 255, 255, 255]);
            format!("Bibites/Parts/{name}.png")
        };
        let body = sheet("small_body", 100, 200);
        let mouth = sheet("small_mouth", 200, 100);
        let eyes = sheet("small_eyes", 200, 200);
        let eyes_dead = sheet("small_eyes_dead", 200, 200);
        let arms = sheet("small_arms", 200, 100);
        let exo = sheet("small_exoskeleton", 200, 100);
        for (name, color) in [
            ("small_egg", [245, 235, 190, 255]),
            ("small_plant", [70, 190, 90, 255]),
            ("small_meat", [200, 70, 60, 255]),
        ] {
            write_png(
                &root.join("Objects/Sprites").join(format!("{name}.png")),
                100,
                100,
                color,
            );
        }
        let manifest = serde_json::json!({
            "schema": "rustibites.viewer.assets.v1",
            "text_files": {
                "Bibites/sizes": "text/bibites_sizes.csv",
                "Bibites/parts": "text/bibites_parts.csv",
                "Objects/egg_sizes": "text/egg_sizes.csv",
                "Objects/pellet_sizes": "text/pellet_sizes.csv",
                "parameters": "text/parameters.csv",
            },
            "textures": {
                "small_body": body,
                "small_mouth": mouth,
                "small_eyes": eyes,
                "small_eyes_dead": eyes_dead,
                "small_arms": arms,
                "small_exoskeleton": exo,
                "small_egg": "Objects/Sprites/small_egg.png",
                "small_plant": "Objects/Sprites/small_plant.png",
                "small_meat": "Objects/Sprites/small_meat.png",
            },
            "prefabs": { "bibite": { "layers": [
                {"name": "Left Arm", "sorting_order": -2},
                {"name": "Body", "sorting_order": -1},
                {"name": "RightArm", "sorting_order": 0},
                {"name": "Exoskelton", "sorting_order": 1},
                {"name": "Mouth", "sorting_order": 2},
                {"name": "Eyes", "sorting_order": 3},
            ]}},
            "camera": {"orthographic_size": 500.0, "background_color": [0.0, 0.0, 0.19, 1.0]},
            "tint_mode": "multiply",
        });
        write_text(&root.join("manifest.json"), &manifest.to_string());
        GameAssets::load(&root).expect("synthetic game assets load")
    }

    #[test]
    fn sprite_mode_composes_layers_and_renders_headlessly() {
        let assets = build_fixture();
        assert_eq!(assets.part_sizes[0].pixel_width, 100);
        assert!(
            (assets.part_sizes[0].pixel_width as f32 / PART_PPU_DIVISOR - 100.0 / PART_PPU_DIVISOR)
                .abs()
                < 1e-6
        );

        let loaded = LoadedWorld::load(None, 4_242, 8, 1).expect("starter world");
        let bibites = loaded.world.bibites.len();
        let mut app = App::new(loaded, Vec2::new(640.0, 480.0), Some(assets));
        // Frame the first bibite closely so its composed layers are visible.
        let centre = app
            .world
            .bibites
            .iter()
            .next()
            .expect("spawned bibite")
            .1
            .pos;
        app.camera = Camera::new(centre, 2.0, Vec2::new(640.0, 480.0));
        app.rebuild_scene();

        let sprites = app
            .scene
            .primitives
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Sprite { .. }))
            .count();
        // Every visible bibite contributes several layers (body, mouth, eyes,
        // two arms, exoskeleton); the population is culled to the camera.
        assert!(
            sprites >= 5,
            "expected layered sprites, got {sprites} for {bibites} bibites"
        );
        // Asset mode uses the game's camera background colour, not the
        // viewer's procedural palette.
        assert_ne!(app.scene.background, BACKGROUND);

        let atlases = app.assets.as_ref().unwrap().atlases.as_slice();
        let image = render_scene(&app.scene, &app.camera, 640, 480, 2, atlases);
        let stats = image.stats();
        assert!(
            stats.non_background > 20_000,
            "blank sprite frame: {stats:?}"
        );
        assert!(stats.distinct_colors > 8, "flat sprite frame: {stats:?}");

        // Also prove the scene builder itself reports sprite primitives when
        // assets are supplied directly (no App state involved).
        let mut scene = Scene::new();
        build_scene(
            &mut scene,
            &app.world,
            &app.camera,
            None,
            &SceneOptions::default(),
            app.assets.as_ref(),
            None,
        );
        assert!(
            scene
                .primitives
                .iter()
                .any(|primitive| matches!(primitive, Primitive::Sprite { .. }))
        );
    }

    /// The display list of `world` seen at `zoom` pixels per world unit from `centre`, with `assets` (no pellets unless `pellets`).
    fn lod_scene(world: &bibites_core::world::World, assets: &GameAssets, centre: Vec2, zoom: f32, selected: Option<bibites_core::ids::BibiteId>, pellets: bool) -> Scene {
        let camera = Camera::new(centre, zoom, Vec2::new(640.0, 480.0));
        let mut scene = Scene::new();
        let options = SceneOptions { pellets, grid: false, zones: false, fov: false, ..SceneOptions::default() };
        build_scene(&mut scene, world, &camera, selected, &options, Some(assets), None);
        scene
    }

    fn sprite_count(scene: &Scene) -> usize {
        scene.primitives.iter().filter(|primitive| matches!(primitive, Primitive::Sprite { .. })).count()
    }

    /// How much of a bibite is drawn follows how long it is on the screen: its parts when it is big enough for them to show, its body alone when it is a few
    /// pixels, one dot when it is a couple. (Its mouth, eyes and arms were a draw each from any distance, and a world zoomed out showed thousands of bibites.)
    #[test]
    fn a_bibite_a_few_pixels_long_is_drawn_as_its_body_alone_and_a_tiny_one_as_a_dot() {
        let assets = build_fixture();
        let loaded = LoadedWorld::load(None, 4_242, 1, 1).expect("starter world");
        let (id, record) = loaded.world.bibites.iter().next().expect("one bibite");
        let (centre, length) = (record.pos, crate::render::body_length(record).expect("a body"));
        let expected_colour = {
            let tint = crate::sprites::skin_tint(record);
            [tint[0], tint[1], tint[2]].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        // 12 pixels long: all its parts.
        let near = lod_scene(&loaded.world, &assets, centre, 12.0 / length, None, false);
        assert!(sprite_count(&near) >= 5, "{} sprites", sprite_count(&near));
        assert!(near.dots.is_empty());
        // 4 pixels: the body sprite alone.
        let middle = lod_scene(&loaded.world, &assets, centre, 4.0 / length, None, false);
        assert_eq!(sprite_count(&middle), 1);
        assert!(middle.dots.is_empty());
        // 1.5 pixels: no sprite, one dot at the body's middle in the bibite's colour (the art of the fixture is white, so the skin multiplies it).
        let far = lod_scene(&loaded.world, &assets, centre, 1.5 / length, None, false);
        assert_eq!(sprite_count(&far), 0);
        assert_eq!(far.dots.len(), 1);
        assert_eq!(far.dots[0].color[..3], expected_colour, "{:?}", far.dots[0]);
        assert!(far.dots[0].center.distance(centre) < length, "the dot is on the bibite");
        // The dot is in a run of the display list, and the run reaches it.
        let runs: Vec<(u32, u32)> = far.primitives.iter().filter_map(|primitive| match primitive {
            Primitive::Dots { first, count } => Some((*first, *count)),
            _ => None,
        }).collect();
        assert_eq!(runs.iter().map(|run| run.1).sum::<u32>(), 1, "{runs:?}");
        assert_eq!(far.dots_of(0, 5).len(), 1, "a run that reaches past the list gets what there is");
        // The selected bibite keeps all its parts, whatever its size.
        let selected = lod_scene(&loaded.world, &assets, centre, 1.5 / length, Some(id), false);
        assert!(sprite_count(&selected) >= 5);
        assert!(selected.dots.is_empty());
    }

    /// A pellet that would be a pixel is a dot of its material's colour, not a sprite primitive: the sprite path wrote a hundred bytes per pellet for the same pixel.
    #[test]
    fn a_pellet_a_pixel_across_is_a_dot_and_a_bigger_one_a_sprite() {
        use bibites_core::pellet::{MaterialKind, PelletSpawn};
        let assets = build_fixture();
        let mut loaded = LoadedWorld::load(None, 4_243, 1, 1).expect("starter world");
        let ids: Vec<_> = loaded.world.pellets.ids().collect();
        for id in ids {
            loaded.world.pellets.remove(id);
        }
        for (x, material) in [(0.0, MaterialKind::Plant), (10.0, MaterialKind::Meat), (20.0, MaterialKind::Plant)] {
            loaded.world.pellets.spawn(PelletSpawn::new(material, 20.0, Vec2::new(x, 500.0)));
        }
        let centre = Vec2::new(10.0, 500.0);
        // (A bibite's sprites are in the scene too, if it is in view: only the pellets' atlases are counted.)
        let atlases = [assets.plant_sprite(1.0).expect("a plant sprite").atlas, assets.meat_sprite(1.0).expect("a meat sprite").atlas];
        let pellet_sprites = |scene: &Scene| {
            scene.primitives.iter().filter(|primitive| matches!(primitive, Primitive::Sprite { sprite, .. } if atlases.contains(&sprite.atlas))).count()
        };
        // 20 pixels per 5 units across: sprites.
        let near = lod_scene(&loaded.world, &assets, centre, 4.0, None, true);
        assert_eq!(pellet_sprites(&near), 3);
        assert!(near.dots.iter().all(|dot| dot.color != [70, 190, 90, 255] && dot.color != [200, 70, 60, 255]), "{:?}", near.dots);
        // A third of a pixel per unit: 1.7 pixels across (the fixture's pellets are 5 units across at 20 u2).
        let far = lod_scene(&loaded.world, &assets, centre, 0.34, None, true);
        assert_eq!(pellet_sprites(&far), 0);
        let plant = far.dots.iter().filter(|dot| dot.color == [70, 190, 90, 255]).count();
        let meat = far.dots.iter().filter(|dot| dot.color == [200, 70, 60, 255]).count();
        assert_eq!((plant, meat), (2, 1), "{:?}", far.dots);
        // A scene is rebuilt from nothing: the dots of the last one are gone with it.
        let mut again = far.clone();
        again.clear();
        assert!(again.dots.is_empty() && again.primitives.is_empty());
    }

    /// The software rasteriser draws a dot a pixel across at the least, on the pixel its centre is in.
    #[test]
    fn the_software_rasteriser_draws_a_dot_as_at_least_one_pixel() {
        let mut scene = Scene::new();
        scene.background = [0, 0, 0, 255];
        let run = scene.open_dots();
        scene.dots.push(crate::render::Dot { center: Vec2::new(0.2, -0.3), half: 0.0, color: [200, 100, 50, 255] });
        scene.dots.push(crate::render::Dot { center: Vec2::new(40.0, 40.0), half: 1.0, color: [1, 2, 3, 255] });
        scene.close_dots(run);
        let camera = Camera::new(Vec2::ZERO, 1.0, Vec2::new(8.0, 8.0));
        let image = render_scene(&scene, &camera, 8, 8, 1, &[]);
        let lit: Vec<(usize, usize)> = (0..8).flat_map(|y| (0..8).map(move |x| (x, y))).filter(|&(x, y)| image.rgba[(y * 8 + x) * 4..(y * 8 + x) * 4 + 3] != [0, 0, 0]).collect();
        assert_eq!(lit, vec![(4, 4)], "one pixel, where the centre is (the second dot is off the picture)");
        assert_eq!(image.rgba[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 3], [200, 100, 50]);
        // Supersampled it is still a pixel of the output.
        let image = render_scene(&scene, &camera, 8, 8, 2, &[]);
        let lit = (0..8 * 8).filter(|pixel| image.rgba[pixel * 4..pixel * 4 + 3] != [0, 0, 0]).count();
        assert_eq!(lit, 1);
    }

    #[test]
    fn tool_placement_spawns_pellets_and_kills_with_killer() {
        let mut app = App::new(synthetic(31, 12), Vec2::new(1280.0, 800.0), None);
        let before = app.world.pellets.len();
        app.tool = crate::ui::ToolKind::Pellet;
        app.apply_tool(Vec2::new(10.0, -10.0));
        assert_eq!(
            app.world.pellets.len(),
            before + 1,
            "placer did not spawn a pellet"
        );

        let target = app.world.live_bibite_ids()[0];
        let pos = app.world.bibites.get(target).expect("target").pos;
        let tools_before = app.world.tools.len();
        app.tool = crate::ui::ToolKind::Killer;
        app.apply_tool(pos);
        assert_eq!(app.world.tools.len(), tools_before + 1);
        assert!(app.last_action.contains("colour killer"));
    }

    #[test]
    fn the_three_placer_buttons_make_three_kinds_of_tower_one_at_a_time() {
        use bibites_core::tools::Tool;
        let mut app = App::new(synthetic(74, 6), Vec2::new(1280.0, 800.0), None);
        let pos = Vec2::new(30.0, 40.0);
        let kinds = |app: &App| {
            let mut rad = 0;
            let mut phero = 0;
            let mut killer = 0;
            for tool in app.world.tools.tools() {
                match tool {
                    Tool::Rad(_) => rad += 1,
                    Tool::Phero(_) => phero += 1,
                    Tool::Killer(_) => killer += 1,
                    // RUSTibites Improved: the placer test of the game's three tools does not count the gene selector.
                    Tool::Gene(_) | Tool::Vent(_) | Tool::Force(_) | Tool::Whirl(_) => {}
                }
            }
            (rad, phero, killer)
        };
        for (tool, expect) in [
            (crate::ui::ToolKind::PheroTower, (0, 1, 0)),
            (crate::ui::ToolKind::Tower, (1, 0, 0)),
            (crate::ui::ToolKind::Killer, (0, 0, 1)),
        ] {
            let before = kinds(&app);
            app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SelectTool(
                tool,
            ))]);
            assert_eq!(app.tool, tool);
            app.apply_tool(pos);
            let after = kinds(&app);
            assert_eq!(
                (after.0 - before.0, after.1 - before.1, after.2 - before.2),
                expect,
                "{tool:?}"
            );
            // One tower per press of the button.
            assert_eq!(app.tool, crate::ui::ToolKind::Inspect);
        }
        // The right button cancels a tower that has not been placed yet.
        app.tool = crate::ui::ToolKind::PheroTower;
        app.handle_input(&FrameInput {
            secondary_down: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(app.tool, crate::ui::ToolKind::Inspect);
    }

    /// Two named zones on opposite sides of the origin, so a tower has something to follow.
    fn two_zones(app: &mut App) -> (bibites_core::ids::ZoneId, bibites_core::ids::ZoneId) {
        let mut zone = |name: &str, x: f32| {
            let mut settings = bibites_core::settings::ZoneSettings::game_defaults();
            settings.name = name.to_owned();
            settings.pos_x = x;
            settings.pos_y = 0.0;
            app.world.add_zone(settings).expect("zone")
        };
        (zone("Alpha", 0.5), zone("Beta", -0.5))
    }

    /// A short click (press and release) at the screen position of the world point `at`.
    fn click_world(app: &mut App, at: Vec2) {
        let screen = app.camera.world_to_screen(at);
        app.handle_input(&press(screen, true, true, false, 0.016));
        app.handle_input(&press(screen, false, false, true, 0.016));
    }

    #[test]
    fn a_placed_tower_selects_itself_and_opens_its_panel_but_a_colour_killer_does_not() {
        use bibites_core::tools::Tool;
        let mut app = App::new(synthetic(80, 0), Vec2::new(1280.0, 800.0), None);
        two_zones(&mut app);
        // A press on the ground (here inside a zone) places the tower under the cursor: it follows
        // no zone (the panel's dropdown sets one), is placed once and becomes the target.
        let at = app.world.zones.zones[0].pos;
        app.tool = crate::ui::ToolKind::PheroTower;
        click_world(&mut app, at);
        assert_eq!(app.tool, crate::ui::ToolKind::Inspect);
        let id = app.selected_tool_id().expect("the tower is selected");
        assert!(app.tool_panel_open);
        let Some(Tool::Phero(tower)) = app.world.tools.get(id) else {
            panic!("pheromone tower expected");
        };
        assert_eq!(tower.base.attached_zone, None);
        assert!(tower.base.pos.distance(at) < 0.5);
        // `ColorKiller.Place` does not select it: the first tower stays the target.
        app.tool = crate::ui::ToolKind::Killer;
        click_world(&mut app, Vec2::new(-300.0, 200.0));
        assert_eq!(app.world.tools.len(), 2);
        assert_eq!(app.selected_tool_id(), Some(id));
    }

    #[test]
    fn the_ghost_follows_the_cursor_while_a_tower_is_being_placed() {
        use crate::render::GhostKind;
        let mut app = App::new(synthetic(80, 0), Vec2::new(1280.0, 800.0), None);
        app.tool = crate::ui::ToolKind::Tower;
        let at = Vec2::new(120.0, -40.0);
        app.handle_input(&FrameInput {
            cursor: app.camera.world_to_screen(at),
            dt: 0.016,
            ..FrameInput::default()
        });
        let view = app.tool_view();
        let (kind, ghost) = view.ghost.expect("ghost while placing");
        assert_eq!(kind, GhostKind::Rad);
        assert!(ghost.distance(at) < 0.5);
        // The pellet placer and the inspect tool have none.
        app.tool = crate::ui::ToolKind::Pellet;
        assert_eq!(app.tool_view().ghost, None);
        app.tool = crate::ui::ToolKind::Inspect;
        assert_eq!(app.tool_view().ghost, None);
    }

    #[test]
    fn escape_closes_a_tower_panel_then_deselects_but_a_colour_killer_deselects_at_once() {
        let mut app = App::new(synthetic(81, 0), Vec2::new(1280.0, 800.0), None);
        let tower = app.world.place_rad_tower_at(Vec2::new(10.0, 10.0), None);
        app.select_placed_tool(tower);
        assert!(app.tool_panel_open && app.selected_tool.is_some());
        // `UIPanel.Escape` closes the panel and keeps the tower selected ...
        app.handle_input(&escape());
        assert!(!app.tool_panel_open);
        assert!(app.selected_tool.is_some());
        assert!(!app.panels.is_open(crate::ui::PanelKind::Escape));
        // ... the next one pops the target (`bibiteSelectedAction`).
        app.handle_input(&escape());
        assert!(app.selected_tool.is_none());
        assert!(!app.panels.is_open(crate::ui::PanelKind::Escape));
        // `ColorKillerPanel.Escape` deselects the killer itself.
        let killer = app.world.place_color_killer_at(Vec2::new(-90.0, 5.0), None);
        click_world(&mut app, Vec2::new(-90.0, 5.0));
        assert!(
            app.selected_tool.is_some(),
            "a click selects the colour killer"
        );
        assert!(app.tool_panel_open);
        app.handle_input(&escape());
        assert!(app.selected_tool.is_none() && !app.tool_panel_open);
        assert!(
            app.world.tools.get(killer).is_some(),
            "Escape does not remove it"
        );
    }

    #[test]
    fn a_click_on_a_tower_selects_it_and_a_bibite_in_reach_takes_over() {
        let mut app = App::new(synthetic(82, 0), Vec2::new(1280.0, 800.0), None);
        let at = Vec2::new(300.0, 300.0);
        app.world.place_color_killer_at(at, None);
        // Just off the collider (radius 17.5) nothing is hit.
        click_world(&mut app, at + Vec2::new(25.0, 0.0));
        assert!(app.selected_tool.is_none());
        click_world(&mut app, at + Vec2::new(10.0, 0.0));
        assert!(app.selected_tool.is_some() && app.tool_panel_open);
        // Clicking it again does not reopen a panel that was closed.
        app.tool_panel_open = false;
        click_world(&mut app, at);
        assert!(!app.tool_panel_open, "SelectTarget(same) returns early");
        // With a living bibite within the selection radius of the click, `SelectClosestBibite
        // AroundCursor` replaces the tower as the target.
        let mut app = App::new(synthetic(83, 4), Vec2::new(1280.0, 800.0), None);
        let bibite = app.world.live_bibite_ids()[0];
        let near = app.world.bibites.get(bibite).unwrap().pos;
        app.camera.center = near;
        let tower_at = near + Vec2::new(12.0, 0.0);
        app.world.place_rad_tower_at(tower_at, None);
        click_world(&mut app, tower_at);
        assert_eq!(app.selected, Some(bibite));
        assert!(app.selected_tool.is_none() && !app.tool_panel_open);
    }

    #[test]
    fn a_held_press_drags_an_unselected_tower_but_never_the_selected_one() {
        let mut app = App::new(synthetic(84, 0), Vec2::new(1280.0, 800.0), None);
        let at = Vec2::new(200.0, 100.0);
        let to = Vec2::new(260.0, 150.0);
        let id = app.world.place_color_killer_at(at, None);
        let position = |app: &App| app.world.tools.get(id).unwrap().base().pos;
        // Held past the click time, the press drags it to the cursor (no rigid body: it jumps).
        let from_screen = app.camera.world_to_screen(at);
        app.handle_input(&press(from_screen, true, true, false, 0.016));
        app.handle_input(&press(
            app.camera.world_to_screen(to),
            false,
            true,
            false,
            0.3,
        ));
        assert!(position(&app).distance(to) < 0.5, "{:?}", position(&app));
        app.handle_input(&press(
            app.camera.world_to_screen(to),
            false,
            false,
            true,
            0.016,
        ));
        assert!(app.selected_tool.is_none(), "a drag is not a click");
        // A tower that follows a zone is put back on it every tick, so it cannot be moved.
        let (alpha, _) = two_zones(&mut app);
        assert!(app.world.set_tool_attached_zone(id, Some(alpha)));
        let home = position(&app);
        app.handle_input(&press(
            app.camera.world_to_screen(home),
            true,
            true,
            false,
            0.016,
        ));
        app.handle_input(&press(
            app.camera.world_to_screen(home + Vec2::new(50.0, 0.0)),
            false,
            true,
            false,
            0.3,
        ));
        app.handle_input(&press(Vec2::ZERO, false, false, true, 0.016));
        assert_eq!(position(&app), home);
        assert!(app.world.set_tool_attached_zone(id, None));
        // The selected one stays where it is.
        click_world(&mut app, home);
        assert!(app.selected_tool.is_some());
        let at_select = position(&app);
        app.handle_input(&press(
            app.camera.world_to_screen(at_select),
            true,
            true,
            false,
            0.016,
        ));
        app.handle_input(&press(
            app.camera.world_to_screen(at_select + Vec2::new(70.0, 0.0)),
            false,
            true,
            false,
            0.3,
        ));
        assert_eq!(position(&app), at_select);
    }

    #[test]
    fn the_right_button_deselects_a_tower_like_any_target() {
        let mut app = App::new(synthetic(85, 0), Vec2::new(1280.0, 800.0), None);
        let tower = app.world.place_phero_tower_at(Vec2::new(5.0, 5.0), None);
        app.select_placed_tool(tower);
        assert!(app.selected_tool.is_some());
        app.handle_input(&FrameInput {
            secondary_down: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected_tool.is_none() && !app.tool_panel_open);
    }

    #[test]
    fn the_panel_controls_edit_the_selected_tower_and_remove_destroys_it() {
        use bibites_core::tools::Tool;
        let mut app = App::new(synthetic(86, 0), Vec2::new(1280.0, 800.0), None);
        let (alpha, beta) = two_zones(&mut app);
        let id = app.world.place_rad_tower_at(Vec2::new(400.0, 400.0), None);
        app.select_placed_tool(id);
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        // "Attached to Zone" goes through None and every zone in order, and the tower snaps to the
        // zone it follows.
        let zones: Vec<_> = app
            .world
            .zones
            .zones
            .iter()
            .map(|zone| (zone.id, zone.pos))
            .collect();
        assert!(zones.len() >= 2, "the world has zones");
        assert!(
            zones
                .iter()
                .any(|(zone, _)| *zone == alpha || *zone == beta)
        );
        let state = |app: &App| *app.world.tools.get(id).unwrap();
        for (zone, pos) in &zones {
            app.apply_ui_actions(&[UiAction::ToolZone]);
            assert_eq!(state(&app).base().attached_zone, Some(*zone));
            assert_eq!(state(&app).base().pos, *pos);
        }
        app.apply_ui_actions(&[UiAction::ToolZone]);
        assert_eq!(state(&app).base().attached_zone, None);
        // The taper slider (linear 0..3): the middle of the track is 1.5; a drag past the end
        // stops at the maximum.
        app.tool_slide(crate::tower_panel::TAPER, (100.0, 400.0), 250.0 * scale);
        let Tool::Rad(tower) = state(&app) else {
            panic!("rad")
        };
        assert!((tower.taper - 1.5).abs() < 1e-3, "{}", tower.taper);
        app.tool_slide(crate::tower_panel::TAPER, (100.0, 400.0), 900.0 * scale);
        let Tool::Rad(tower) = state(&app) else {
            panic!("rad")
        };
        assert!((tower.taper - 3.0).abs() < 1e-3);
        // The radius slider is logarithmic between 10 and half the world.
        app.tool_slide(crate::tower_panel::RADIUS, (100.0, 400.0), 100.0 * scale);
        let Tool::Rad(tower) = state(&app) else {
            panic!("rad")
        };
        assert!((tower.radius - 10.0).abs() < 1e-2, "{}", tower.radius);
        // Remove destroys the tower, clears the target and closes the panel.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::RemoveTool)]);
        assert!(app.world.tools.is_empty());
        assert!(app.selected_tool.is_none() && !app.tool_panel_open);
    }

    #[test]
    fn the_colour_killer_panel_switches_and_the_delete_key() {
        use bibites_core::tools::Tool;
        let mut app = App::new(synthetic(87, 0), Vec2::new(1280.0, 800.0), None);
        let id = app.world.place_color_killer_at(Vec2::new(50.0, 50.0), None);
        click_world(&mut app, Vec2::new(50.0, 50.0));
        assert!(app.tool_panel_open);
        app.apply_ui_actions(&[
            UiAction::ToolSwitch(crate::tower_panel::BODY_OR_EYES),
            UiAction::ToolSwitch(crate::tower_panel::ANY_OR_MOST),
        ]);
        let Some(Tool::Killer(killer)) = app.world.tools.get(id) else {
            panic!("killer");
        };
        assert!(killer.body_or_eye && killer.any_or_most);
        // Delete destroys a selected colour killer (`TryKillTarget`), but not a tower.
        app.handle_input(&FrameInput {
            kill_target: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.world.tools.is_empty());
        let tower = app.world.place_phero_tower_at(Vec2::new(5.0, 5.0), None);
        app.select_placed_tool(tower);
        app.handle_input(&FrameInput {
            kill_target: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(app.world.tools.len(), 1, "Delete does nothing to a tower");
    }

    #[test]
    fn a_pheromone_slider_reaches_the_spot_at_once_even_while_paused() {
        use bibites_core::tools::Tool;
        let mut app = App::new(synthetic(88, 0), Vec2::new(1280.0, 800.0), None);
        app.paused = true;
        let id = app.world.place_phero_tower_at(Vec2::new(30.0, 30.0), None);
        app.select_placed_tool(id);
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        // The right end of the red slider is 100 u².
        app.tool_slide(crate::tower_panel::RED, (0.0, 200.0), 200.0 * scale);
        let Some(Tool::Phero(tower)) = app.world.tools.get(id) else {
            panic!("phero");
        };
        assert!((tower.red - 100.0).abs() < 0.1);
        let spot = tower.spot.expect("the tower's spot");
        let strength = app.world.pheromones.strength(spot).expect("live spot");
        assert!((strength[0] - 100.0).abs() < 0.1, "{strength:?}");
    }

    #[test]
    fn towers_the_ghost_and_a_selected_killers_radius_reach_the_scene() {
        use crate::render::Primitive;
        use bibites_core::tools::Tool;
        let mut app = App::new(synthetic(90, 0), Vec2::new(1280.0, 800.0), None);
        app.camera = Camera::new(Vec2::ZERO, 2.0, Vec2::new(1280.0, 800.0));
        app.rebuild_scene();
        let empty = app.scene.primitives.len();
        let rad = app.world.place_rad_tower_at(Vec2::new(20.0, 0.0), None);
        app.world.place_phero_tower_at(Vec2::new(-20.0, 0.0), None);
        let killer = app.world.place_color_killer_at(Vec2::new(0.0, 30.0), None);
        app.rebuild_scene();
        let placed = app.scene.primitives.len();
        assert!(placed >= empty + 5, "{empty} -> {placed}");
        // A radiation tower shows its area: the radio zone's program over a square of its diameter.
        let Some(Tool::Rad(tower)) = app.world.tools.get(rad) else {
            panic!("rad");
        };
        let radius = tower.radius;
        let area = |app: &App, at: Vec2| {
            app.scene.primitives.iter().any(|primitive| {
                matches!(primitive, Primitive::RadioField { center, radius: r }
                    if (*r - radius).abs() < 1e-3 && center.distance(at) < 0.5)
            })
        };
        assert!(area(&app, Vec2::new(20.0, 0.0)));
        // The placement ghost follows the cursor: more of the same shapes, at its position.
        app.tool = crate::ui::ToolKind::Tower;
        app.cursor = app.camera.world_to_screen(Vec2::new(60.0, -20.0));
        app.rebuild_scene();
        assert!(app.scene.primitives.len() > placed);
        assert!(area(&app, Vec2::new(60.0, -20.0)));
        app.tool = crate::ui::ToolKind::Inspect;
        // A selected colour killer also draws the area it scans, at 30 % opacity.
        let Some(Tool::Killer(scan)) = app.world.tools.get(killer) else {
            panic!("killer");
        };
        let scan_radius = scan.radius;
        let disc = |app: &App| {
            app.scene.primitives.iter().any(|primitive| {
                matches!(primitive, Primitive::Circle { radius: r, color, .. }
                    if (*r - scan_radius).abs() < 1e-3 && color[3] == 77)
            })
        };
        app.rebuild_scene();
        assert!(!disc(&app));
        app.select_placed_tool(killer);
        app.rebuild_scene();
        assert!(disc(&app));
    }

    #[test]
    fn a_tower_that_disappears_leaves_the_selection() {
        let mut app = App::new(synthetic(89, 0), Vec2::new(1280.0, 800.0), None);
        let id = app.world.place_rad_tower_at(Vec2::new(30.0, 30.0), None);
        app.select_placed_tool(id);
        assert!(app.world.remove_tool(id));
        app.advance(0.016);
        assert!(app.selected_tool.is_none() && !app.tool_panel_open);
    }

    fn undo_key(redo: bool) -> FrameInput {
        FrameInput {
            undo: !redo,
            redo,
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    #[test]
    fn ctrl_z_takes_back_settings_edits_and_ctrl_shift_z_redoes_them() {
        use crate::setting_ui_data::{SETTING_UI, SettingUiKind};
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(75, 4), Vec2::new(1280.0, 800.0), None);
        let toggle = SETTING_UI
            .iter()
            .position(|row| row.kind == SettingUiKind::Bool)
            .expect("a toggle row");
        let original = app.world.settings.clone();

        // With nothing open on the Escape stack Ctrl + Z does nothing (the game returns first).
        app.dyn_toggle(toggle);
        let edited = app.world.settings.clone();
        assert_ne!(edited, original);
        app.handle_input(&undo_key(false));
        assert_eq!(app.world.settings, edited, "the stack was empty");

        // With a panel open it steps back, and Ctrl + Shift + Z forward again.
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::WorldDynamics)]);
        app.handle_input(&undo_key(false));
        assert_eq!(app.world.settings, original);
        app.handle_input(&undo_key(false));
        assert_eq!(app.world.settings, original, "nothing older to take back");
        app.handle_input(&undo_key(true));
        assert_eq!(app.world.settings, edited);
        app.handle_input(&undo_key(true));
        assert_eq!(app.world.settings, edited, "nothing newer to redo");

        // A new edit drops what could have been redone.
        app.handle_input(&undo_key(false));
        app.dyn_toggle(toggle);
        app.dyn_toggle(toggle);
        assert_eq!(app.undo.undoable(), 2);
        app.handle_input(&undo_key(true));
        assert_eq!(app.undo.undoable(), 2, "the redo tail is gone");
    }

    #[test]
    fn a_slider_drag_is_one_undo_step_and_only_sixty_steps_are_kept() {
        use crate::setting_ui_data::{SETTING_UI, SettingUiKind};
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(76, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::WorldDynamics)]);
        let slider = (0..SETTING_UI.len())
            .find(|index| {
                SETTING_UI[*index].kind != SettingUiKind::Bool
                    && crate::dyn_panel::slide_value(
                        &mut app.world.settings.clone(),
                        *index,
                        (0.0, 100.0),
                        50.0,
                        app.tps_defaults,
                    )
            })
            .expect("a slider row");
        let original = app.world.settings.clone();
        // Dragging across the track (several frames) is one edit once the button is released.
        app.dyn_panel.drag = Some((slider as u16, 0, 100));
        for x in [10.0_f32, 30.0, 60.0, 90.0] {
            app.dyn_slide(slider, (0.0, 100.0), x * app.camera.screen.y / 1080.0);
        }
        app.dyn_panel.drag = None;
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(app.undo.undoable(), 1);
        app.handle_input(&undo_key(false));
        assert_eq!(app.world.settings, original, "the whole drag was undone");

        // Sixty edits fit, the oldest drops off after that.
        let toggle = SETTING_UI
            .iter()
            .position(|row| row.kind == SettingUiKind::Bool)
            .expect("a toggle row");
        for _ in 0..70 {
            app.dyn_toggle(toggle);
        }
        assert_eq!(app.undo.undoable(), 60);
    }

    #[test]
    fn zone_edits_join_the_ctrl_z_history_and_a_zone_slider_drag_is_one_step() {
        use crate::ui::{DropdownTarget as Target, PanelKind};
        use crate::zone_editor::{DISTRIBUTION, FERTILITY};
        let mut app = App::new(synthetic(131, 0), Vec2::new(1280.0, 800.0), None);
        two_zones(&mut app);
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::WorldDynamics)]);
        let alpha = zone_named(&app, "Alpha");
        let beta = zone_named(&app, "Beta");
        let original = app.zone_settings_list()[alpha].clone();
        let beta_original = app.zone_settings_list()[beta].clone();

        // A dropdown pick is one step: Ctrl + Z takes it back, Ctrl + Shift + Z does it again.
        app.click_rect = [100.0, 200.0, 180.0, 30.0];
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::Zone(
            alpha as u16,
            DISTRIBUTION,
        ))]);
        app.apply_ui_actions(&[UiAction::DropdownPick(3)]);
        let picked = app.zone_settings_list()[alpha].clone();
        assert_ne!(picked.distribution, original.distribution);
        assert_eq!(app.undo.undoable(), 1);
        app.handle_input(&undo_key(false));
        assert_eq!(app.zone_settings_list()[alpha], original, "taken back");
        assert_eq!(
            app.world.zones.zones[alpha].config.0.distribution,
            original.distribution
        );
        app.handle_input(&undo_key(true));
        assert_eq!(app.zone_settings_list()[alpha], picked, "and done again");

        // A drag of a slider across several frames is one step, whatever it passed through.
        app.dyn_panel.zone_drag = Some((alpha as u16, FERTILITY, 0, 100));
        for x in [10.0_f32, 40.0, 70.0, 95.0] {
            app.zone_slide(
                alpha,
                FERTILITY,
                (0.0, 100.0),
                x * app.camera.screen.y / 1080.0,
            );
        }
        app.dyn_panel.zone_drag = None;
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        let dragged = app.zone_settings_list()[alpha].clone();
        assert_ne!(dragged.fertility, picked.fertility);
        assert_eq!(app.undo.undoable(), 2, "the drag is one more step");
        app.handle_input(&undo_key(false));
        assert_eq!(
            app.zone_settings_list()[alpha],
            picked,
            "the whole drag went back"
        );
        app.handle_input(&undo_key(false));
        assert_eq!(app.zone_settings_list()[alpha], original);
        app.handle_input(&undo_key(true));
        app.handle_input(&undo_key(true));
        assert_eq!(app.zone_settings_list()[alpha], dragged);
        // The other zone was never touched.
        assert_eq!(app.zone_settings_list()[beta], beta_original);

        // Edits of the scenario settings and of a zone share the one history, in order.
        use crate::setting_ui_data::{SETTING_UI, SettingUiKind};
        let toggle = SETTING_UI
            .iter()
            .position(|row| row.kind == SettingUiKind::Bool)
            .expect("a toggle row");
        let settings = app.world.settings.clone();
        app.dyn_toggle(toggle);
        assert_ne!(app.world.settings, settings);
        app.handle_input(&undo_key(false));
        assert_eq!(app.world.settings, settings, "the toggle first");
        assert_eq!(
            app.zone_settings_list()[alpha],
            dragged,
            "the zone is as it was"
        );
        app.handle_input(&undo_key(false));
        assert_eq!(
            app.zone_settings_list()[alpha],
            picked,
            "then the zone's drag"
        );

        // Deleting a zone renumbers the others: the steps that edited zones are forgotten, the
        // others stay and still work.
        let settings_now = app.world.settings.clone();
        app.dyn_toggle(toggle);
        let steps = app.undo.undoable();
        app.delete_zone(beta);
        assert!(app.undo.undoable() < steps && app.undo.undoable() >= 1);
        app.handle_input(&undo_key(false));
        assert_eq!(app.world.settings, settings_now);
    }

    #[test]
    fn kill_and_apply_settings_commands_are_wired() {
        let dir = std::env::temp_dir().join(format!("rustibites-kill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(32, 8), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        assert!(app.select_first());
        let selected = app.selected.expect("selected");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::KillSelected)]);
        assert!(
            app.selected.is_some(),
            "KILL should keep the corpse selected"
        );
        assert!(app.last_action.contains("killed"));
        assert!(app.world.bibites.get(selected).is_some());

        app.scene_options.grid = false;
        app.show_hud = false;
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ResetSettings)]);
        assert!(
            !app.scene_options.grid,
            "the game's ShowGrid default is off"
        );
        assert!(app.show_hud);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ApplySettings)]);
        assert!(app.last_action.contains("applied"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_autosave_is_written_on_its_own_thread_and_reported_when_done() {
        let dir = std::env::temp_dir().join(format!("rustibites-autosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(34, 8), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.prefs.autosave = true;
        app.prefs.autosave_real_time = true;
        app.prefs.autosave_period_minutes = 1;
        app.prefs.autosave_max_files = 5;
        // Not yet due: nothing happens.
        app.update_autosave(30.0, 0.0);
        assert!(!app.autosave_in_progress());
        // Due: the write starts on another thread and the call returns.
        app.update_autosave(31.0, 0.0);
        assert!(
            app.autosave_in_progress() || app.last_action.starts_with("autosaved"),
            "{}",
            app.last_action
        );
        let started = std::time::Instant::now();
        while app.autosave_in_progress() && started.elapsed().as_secs() < 60 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.update_autosave(0.0, 0.0);
        }
        assert!(
            app.last_action.starts_with("autosaved"),
            "{}",
            app.last_action
        );
        let written: Vec<_> = std::fs::read_dir(app.storage.autosaves_dir())
            .expect("autosave folder")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(written.len(), 1, "{written:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn quick_save_and_load_round_trip_through_ui_commands() {
        let dir =
            std::env::temp_dir().join(format!("rustibites-quick-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(33, 8), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::QuickSave)]);
        let path = app.quick_save_path();
        assert!(
            path.is_file(),
            "quick save missing: {} ({})",
            path.display(),
            app.last_action
        );
        let tick = app.world_tick();
        // A tower selected before the load belongs to the old world: its panel closes.
        let tower = app.world.place_rad_tower_at(Vec2::new(10.0, 10.0), None);
        app.select_placed_tool(tower);
        assert!(app.tool_panel_open);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::QuickLoad)]);
        assert!(app.last_action.contains("loaded"), "{}", app.last_action);
        assert_eq!(app.world_tick(), tick, "quick load changed simulation time");
        assert!(app.selected_tool.is_none() && !app.tool_panel_open);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One frame of left-button input at `cursor`.
    fn press(cursor: Vec2, pressed: bool, down: bool, released: bool, dt: f32) -> FrameInput {
        FrameInput {
            cursor,
            primary_pressed: pressed,
            primary_down: down,
            primary_released: released,
            dt,
            ..FrameInput::default()
        }
    }

    #[test]
    fn a_quick_click_on_a_bibite_selects_it_and_a_long_press_never_pans() {
        let mut app = App::new(synthetic(36, 8), Vec2::new(1280.0, 800.0), None);
        let id = app.world.live_bibite_ids()[0];
        let at = app
            .camera
            .world_to_screen(app.world.bibites.get(id).unwrap().pos);
        app.handle_input(&press(at, true, true, false, 0.016));
        app.handle_input(&press(at, false, false, true, 0.016));
        assert_eq!(app.selected, Some(id), "a click on a bibite selects it");

        // Holding past the click time on empty ground and releasing elsewhere draws a rectangle
        // (pressing a body never starts one); the camera stays.
        let centre = app.camera.center;
        let clear = Vec2::new(2.0, 2.0);
        let away = clear + Vec2::new(60.0, 40.0);
        app.handle_input(&press(clear, true, true, false, 0.016));
        app.handle_input(&press(away, false, true, false, 0.3));
        assert!(app.drag.is_some_and(|drag| drag.is_rectangle()));
        app.handle_input(&press(away, false, false, true, 0.016));
        assert_eq!(app.camera.center, centre, "the left button does not pan");
        assert!(app.drag.is_none());

        // A long press that starts on a body is that body's own gesture: no rectangle.
        app.handle_input(&press(at, true, true, false, 0.016));
        app.handle_input(&press(
            at + Vec2::new(200.0, 120.0),
            false,
            true,
            false,
            0.3,
        ));
        assert!(app.drag.is_some_and(|drag| !drag.is_rectangle()));
        app.handle_input(&press(at, false, false, true, 0.016));
    }

    /// One frame with the digit key `digit` pressed.
    fn digit(key: u32) -> FrameInput {
        FrameInput {
            digit: Some(key),
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    fn escape() -> FrameInput {
        FrameInput {
            quit: true,
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    #[test]
    fn digits_toggle_the_inspector_panels_like_open_bibite_panel() {
        let mut app = App::new(synthetic(51, 8), Vec2::new(1280.0, 800.0), None);
        let speed = app.speed;
        // Without a target the digits do nothing (the game has no speed hotkeys).
        for key in 1..=8 {
            app.handle_input(&digit(key));
        }
        assert!(!app.inspector_visible());
        assert_eq!(app.speed, speed);
        assert!(app.select_first());
        // Selecting opens no panel: only the tab buttons show.
        assert!(!app.inspector_visible());
        app.handle_input(&digit(2));
        assert!(app.inspector_visible());
        assert_eq!(app.inspector_tab, InspectorTab::Genes);
        // Another digit switches the panel, the same one closes it.
        app.handle_input(&digit(3));
        assert_eq!(app.inspector_tab, InspectorTab::Biology);
        app.handle_input(&digit(4));
        assert_eq!(app.inspector_tab, InspectorTab::Brain);
        app.handle_input(&digit(4));
        assert!(!app.inspector_visible());
        // The tab buttons behave the same way.
        app.apply_ui_actions(&[UiAction::SetTab(InspectorTab::Stats)]);
        assert!(app.inspector_visible());
        app.apply_ui_actions(&[UiAction::SetTab(InspectorTab::Stats)]);
        assert!(!app.inspector_visible());
        // 5-8 never did anything for a bibite.
        app.handle_input(&digit(7));
        assert!(!app.inspector_visible());
    }

    #[test]
    fn escape_peels_off_the_panel_then_the_target_then_opens_the_pause_menu() {
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(52, 8), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(1));
        assert!(app.inspector_visible());
        // First Escape: the panel closes and `curMenu` resets; the bibite stays selected.
        app.handle_input(&escape());
        assert!(!app.inspector_visible() && !app.inspector_open);
        assert!(app.selected.is_some());
        assert!(!app.panels.is_open(PanelKind::Escape));
        // Second: the target is dropped.
        app.handle_input(&escape());
        assert!(app.selected.is_none() && app.selection.is_none());
        assert!(!app.panels.is_open(PanelKind::Escape));
        // Third: nothing left to pop, so the pause menu opens.
        app.handle_input(&escape());
        assert!(app.panels.is_open(PanelKind::Escape));
        assert!(app.paused);
    }

    #[test]
    fn the_chosen_panel_comes_back_with_the_next_selection() {
        let mut app = App::new(synthetic(53, 8), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(2));
        assert_eq!(app.inspector_tab, InspectorTab::Genes);
        // Deselecting (Home) hides the panel but does not forget it (`curMenu` persists).
        app.handle_input(&FrameInput {
            camera_home: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected.is_none() && !app.inspector_visible());
        assert!(app.inspector_open);
        assert!(app.select_first());
        assert!(app.inspector_visible());
        assert_eq!(app.inspector_tab, InspectorTab::Genes);
    }

    #[test]
    fn a_panels_close_button_does_not_deselect_the_target() {
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(54, 8), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(1));
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Information)]);
        assert!(app.panels.is_open(PanelKind::Information));
        app.apply_ui_actions(&[UiAction::ClosePanel]);
        assert!(!app.panels.is_open(PanelKind::Information));
        assert!(app.selected.is_some() && app.inspector_visible());
        // The escape stack still has the newest-first order: the panel opened last closes first.
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Information)]);
        app.handle_input(&escape());
        assert!(!app.panels.is_open(PanelKind::Information));
        assert!(app.inspector_visible());
    }

    #[test]
    fn a_selected_egg_hands_the_selection_to_its_newborn() {
        let mut app = App::new(synthetic(61, 6), Vec2::new(1280.0, 800.0), None);
        let parent = app.world.live_bibite_ids()[0];
        assert_eq!(app.world.lay_egg_once(parent), 1);
        let egg = app.world.eggs.ids().next().expect("an egg");
        app.select_egg(egg);
        {
            let record = app.world.eggs.get_mut(egg).expect("the egg");
            record.hatch.is_hatching = true;
            record.hatch.hatch_progress = record.hatch.hatch_time - 0.001;
            record.hatch.energy = record.hatch.total_required_energy * 2.0;
        }
        let before = app.world.live_bibite_ids().len();
        app.advance(0.25);
        assert!(app.world.eggs.get(egg).is_none(), "the egg hatched");
        assert_eq!(app.world.live_bibite_ids().len(), before + 1);
        assert!(app.selected_egg.is_none());
        let child = app.selected.expect("the newborn is selected");
        assert!(app.world.bibites.get(child).is_some());
        assert!(app.follow_selected, "the camera follows the newborn");
    }

    #[test]
    fn an_aborted_egg_just_ends_the_selection() {
        let mut app = App::new(synthetic(62, 6), Vec2::new(1280.0, 800.0), None);
        let parent = app.world.live_bibite_ids()[0];
        assert_eq!(app.world.lay_egg_once(parent), 1);
        let egg = app.world.eggs.ids().next().expect("an egg");
        app.select_egg(egg);
        assert!(app.world.abort_egg(egg));
        app.advance(0.25);
        assert!(app.selected_egg.is_none() && app.selected.is_none());
        assert!(
            app.selection.is_none(),
            "the egg's snapshot goes with the egg"
        );
        assert!(!app.inspector_visible());
    }

    /// A living bibite's egg of a starter world, selected. The starter world has no species yet, so one is recorded for
    /// its bibites (the egg inherits its parent's species).
    fn selected_egg_of(app: &mut App) -> bibites_core::ids::EggId {
        let ids = app.world.live_bibite_ids();
        let (genome, brain) = {
            let record = app.world.bibites.get(ids[0]).expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let species = app
            .world
            .species
            .add_template_species("Zzalpha zzone", "", &genome, &brain);
        for id in &ids {
            app.world.bibites.get_mut(*id).expect("record").meta.species = Some(species);
        }
        app.world.reconcile_species_counts();
        let parent = ids[0];
        assert_eq!(app.world.lay_egg_once(parent), 1);
        let egg = app.world.eggs.ids().next().expect("an egg");
        app.select_egg(egg);
        egg
    }

    #[test]
    fn an_egg_target_has_the_inspector_snapshot_of_its_own_genes_brain_and_stats() {
        let mut app = App::new(synthetic(65, 6), Vec2::new(1280.0, 800.0), None);
        let egg = selected_egg_of(&mut app);
        let panel = app.selection.as_ref().expect("a snapshot of the egg");
        assert_eq!(panel.target, SelectionTarget::Egg(egg));
        let record = app.world.eggs.get(egg).expect("the egg");
        // The Stats panel's egg section (`UpdateEggStats`).
        let view = app.world.egg_stats(egg).expect("the egg's stats");
        assert_eq!(panel.egg, Some(view.clone()));
        assert_eq!(view.generation, record.meta.generation);
        assert!((view.egg_area - 5.0 * record.size_gene * record.size_gene).abs() < 1e-6);
        assert_eq!(view.energy, record.hatch.energy);
        // The Genes and Brain panels read the egg's `BibiteGenes` and `NEATBrain`.
        assert_eq!(panel.genes, *record.genome.values());
        assert_eq!(panel.generation, record.meta.generation);
        assert_eq!(panel.brain_nodes as usize, record.brain.node_count());
        assert_eq!(panel.brain_synapses as usize, record.brain.synapse_count());
        assert_eq!(panel.species_id, record.meta.species.map(SpeciesId::value));
        assert!(panel.species_genes.is_some(), "its species' template");
        assert!(panel.gene_distance.is_some() && panel.brain_distance.is_some());
        // `CameraManager` follows any target; no panel is open until one is chosen.
        assert!(app.follow_selected);
        assert!(!app.inspector_visible());
        // The digits: 1 Stats, 2 Genes, 3 Brain (an egg has no Biology panel), 4 nothing.
        app.handle_input(&digit(2));
        assert_eq!(app.inspector_tab, InspectorTab::Genes);
        assert!(app.inspector_visible());
        app.handle_input(&digit(3));
        assert_eq!(app.inspector_tab, InspectorTab::Brain);
        app.handle_input(&digit(4));
        assert_eq!(app.inspector_tab, InspectorTab::Brain);
        // The Biology button cannot open it either.
        app.apply_ui_actions(&[UiAction::SetTab(InspectorTab::Biology)]);
        assert_eq!(app.inspector_tab, InspectorTab::Brain);
        // Escape peels the panel off first, then the egg.
        app.handle_input(&escape());
        assert!(!app.inspector_visible() && app.selected_egg.is_some());
        app.handle_input(&escape());
        assert!(app.selected_egg.is_none() && app.selection.is_none());
    }

    #[test]
    fn an_open_biology_panel_hands_over_to_the_stats_panel_when_an_egg_is_selected() {
        let mut app = App::new(synthetic(66, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(3));
        assert_eq!(app.inspector_tab, InspectorTab::Biology);
        // `BiologyPanel.FillPanel`: `OpenBibitePanel(StatsPanel)` for an egg target.
        selected_egg_of(&mut app);
        assert!(app.inspector_visible());
        assert_eq!(app.inspector_tab, InspectorTab::Stats);
        // The chosen panel (`curMenu`) is the Stats panel from now on, for bibites too.
        assert!(app.select_first());
        assert_eq!(app.inspector_tab, InspectorTab::Stats);
        // Genes or Brain stay open across the change.
        app.handle_input(&digit(2));
        selected_egg_of(&mut app);
        assert_eq!(app.inspector_tab, InspectorTab::Genes);
    }

    #[test]
    fn the_biology_panel_has_its_stomach_pop_up_and_a_flow_that_adds_up() {
        use crate::biology_view::{PICTURE, popup_shows};
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(3));
        assert_eq!(app.inspector_tab, InspectorTab::Biology);
        let panel = app.selection.clone().expect("the selection panel");
        let params = panel.internals.expect("a bibite has internals");
        // The pointer on the middle of the stomach opens the pop-up (`internsMap` 2); on the corner of the picture (outside the body) it does not.
        let shape = crate::internals::StomachShape::of(&params);
        let on_stomach = Vec2::new(PICTURE[0] + (shape.offset[0] + 0.5) * 5.0, PICTURE[1] + PICTURE[3] / 2.0);
        assert!(popup_shows(&app, on_stomach), "{shape:?}");
        assert!(!popup_shows(&app, Vec2::new(PICTURE[0] + 2.0, PICTURE[1] + 2.0)));
        assert!(!popup_shows(&app, Vec2::new(900.0, 900.0)));
        // Another tab shows nothing.
        app.handle_input(&digit(2));
        assert!(!popup_shows(&app, on_stomach));
        // The flow is `EnergyFlowRate`: what the stomach and the fat give, less what the body spends, through the metabolic efficiency.
        let id = app.selected.expect("selected");
        let record = app.world.bibites.get_mut(id).expect("the bibite");
        record.body.movement_cost = 2.5;
        record.body.metabolism = 0.4;
        record.fat.last_energy_gain_rate = 1.5;
        record.fat.last_energy_sustain_rate = 0.25;
        record.body.dead = false;
        let efficiency = record.body.params.metabolic_efficiency;
        let expected = record.stomach.energy_gain_rate() + 1.5
            - (0.25
                + record.growth.growth_cost_rate
                + record.body.healing_rate_energy_cost
                + 0.4
                + 2.5
                + record.derived.brain_upkeep
                + record.egg_layer.energy_consumption)
                / efficiency;
        assert!((record.energy_flow_rate() - expected).abs() < 1e-5);
        // A corpse only pays the fat upkeep and its metabolism.
        record.body.dead = true;
        assert!((record.energy_flow_rate() - (-(0.25 + 0.4) / efficiency)).abs() < 1e-5);
    }

    #[test]
    fn the_organ_pop_ups_read_the_bibite_the_way_the_panel_does() {
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(3));
        let id = app.selected.expect("selected");
        let info = app.selection.as_ref().and_then(|panel| panel.organs.clone()).expect("organ info");
        let record = app.world.bibites.get(id).expect("the bibite");
        // The body pop-up's masses are the parts of `body.mass`.
        let sum = info.base_mass + info.armor_mass + info.stomach_mass + info.eggs_mass + info.fat_mass;
        assert!((sum - info.mass).abs() < 1e-3 * info.mass.max(1.0), "{sum} vs {}", info.mass);
        assert!(info.total_energy > 0.0 && info.energy_density > 0.0);
        // The womb: `eggCapacity = floor(area / eggSize)`, and a bibite that is not mature shows no stored eggs yet.
        assert_eq!(info.egg_capacity, (info.womb_area / info.egg_size).floor());
        assert_eq!(info.mature, record.growth.maturity(&record.growth_genes) >= 1.0);
        // The mouth's answers need the brain's wish and a finished bite period.
        assert!(!info.can_swallow || info.swallow_output > 0.15);
        assert!(!info.can_attack || info.attack_output > 0.15);
    }

    #[test]
    fn the_flow_details_pop_up_lists_the_ten_rates_the_panel_feeds_its_sliders() {
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(3));
        let id = app.selected.expect("selected");
        {
            let record = app.world.bibites.get_mut(id).expect("the bibite");
            record.body.dead = false;
            record.body.movement_cost = 2.5;
            record.body.metabolism = 0.4;
            record.fat.last_energy_gain_rate = 1.5;
            record.fat.last_energy_sustain_rate = 0.25;
            record.pheromone.cost = 0.02;
        }
        let record = app.world.bibites.get(id).expect("the bibite");
        let info = crate::biology_popups::organ_info_of(record, &app.world.settings);
        let efficiency = record.body.params.metabolic_efficiency;
        // Movement, metabolism, pheromones, fat conversion and fat sustainment, as the panel writes them.
        assert!((info.energy_flows[1] + 2.5 / efficiency).abs() < 1e-5);
        assert!((info.energy_flows[3] + 0.4 / efficiency).abs() < 1e-5);
        assert!((info.energy_flows[4] + 0.02 / efficiency).abs() < 1e-5);
        assert!((info.energy_flows[8] - 1.5).abs() < 1e-5);
        assert!((info.energy_flows[9] + 0.25 / efficiency).abs() < 1e-5);
        // The Flow bar's total leaves the pheromone cost out (`EnergyFlowRate`), so the ten rates add up to it plus that cost.
        let total: f32 = info.energy_flows.iter().sum();
        let pheromones = info.energy_flows[4];
        assert!((total - pheromones - record.energy_flow_rate()).abs() < 1e-4, "{total} {pheromones} {}", record.energy_flow_rate());
        // A corpse shows only the metabolism and the fat upkeep.
        {
            let record = app.world.bibites.get_mut(id).expect("the bibite");
            record.body.dead = true;
        }
        let record = app.world.bibites.get(id).expect("the bibite");
        let corpse = crate::biology_popups::organ_info_of(record, &app.world.settings);
        for (index, rate) in corpse.energy_flows.iter().enumerate() {
            assert!(index == 3 || index == 9 || *rate == 0.0, "row {index}: {rate}");
        }
        // The pop-up shows with the pointer on the Flow row or on the pop-up, and nowhere else.
        let row = Vec2::new(212.0, 223.0);
        assert!(crate::biology_popups::flow_row_at(row, false).is_some());
        assert!(crate::biology_popups::flow_row_at(Vec2::new(500.0, 223.0), false).is_none());
        assert!(crate::biology_popups::flow_row_at(Vec2::new(212.0, 180.0), false).is_none());
        // The pop-up opens after 0.3 s on the row and closes 0.3 s after leaving.
        use crate::biology_popups::{BarHover, HoverKey};
        let mut hover = BarHover::default();
        hover.update(row, false, true, 0.2);
        assert!(hover.shown.is_none());
        hover.update(row, false, true, 0.2);
        assert_eq!(hover.shown.map(|s| s.0), Some(HoverKey::Flow));
        hover.update(Vec2::new(900.0, 900.0), false, true, 0.2);
        assert!(hover.shown.is_some());
        hover.update(Vec2::new(900.0, 900.0), false, true, 0.2);
        assert!(hover.shown.is_none());
        // The Sort toggle flips the setting.
        assert!(!app.sort_energy_details);
        app.apply_ui_actions(&[UiAction::ToggleEnergySort]);
        assert!(app.sort_energy_details);
    }

    #[test]
    fn a_body_that_bursts_into_meat_sprays_blood_five_times_and_it_is_gone_after_half_a_second() {
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        app.paused = false;
        let id = app.world.bibites.iter().next().map(|(id, _)| id).expect("a bibite");
        let (pos, count) = {
            let record = app.world.bibites.get_mut(id).expect("the bibite");
            record.body.dead = true;
            (record.pos, (record.body.geometry.base_body_area as i32) / 20)
        };
        assert!(app.world.kill_or_explode_bibite(id));
        // The log has the burst, with the place and the count `ExplodeToMeat` gives.
        let burst = app.world.bibites.blood_log().last().map(|(_, burst)| *burst).expect("a burst");
        assert_eq!(burst.pos, pos);
        assert_eq!(burst.count, count);
        // The first emission is at once; the next four follow every 0.05 s of game time.
        let now = app.world.simulated_time();
        crate::blood_particles::advance(&app.world);
        let wide = (Vec2::new(-1.0e6, -1.0e6), Vec2::new(1.0e6, 1.0e6));
        let first = crate::blood_particles::drawn(wide.0, wide.1).len();
        assert!(count == 0 || first > 0, "count {count}");
        app.world.set_simulated_time(now + 0.3);
        crate::blood_particles::advance(&app.world);
        let later = crate::blood_particles::drawn(wide.0, wide.1).len();
        assert!(count == 0 || later > 0);
        // Half a second after the last emission nothing is left.
        app.world.set_simulated_time(now + 0.9);
        crate::blood_particles::advance(&app.world);
        app.world.set_simulated_time(now + 1.0);
        crate::blood_particles::advance(&app.world);
        assert!(crate::blood_particles::drawn(wide.0, wide.1).is_empty());
    }

    #[test]
    fn blood_is_gone_when_every_frame_steps_more_than_a_second_of_game_time() {
        // A time speed of 100 steps 32 ticks a frame, over a second of game time at 30 ticks a second. A step over a second used to skip the
        // particles' ageing ("not played back"), so the blood of a burst stayed on the screen for ever at a high speed.
        let wide = (Vec2::new(-1.0e6, -1.0e6), Vec2::new(1.0e6, 1.0e6));
        for overlap in [false, true] {
            let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
            // A model machine whose ticks take a millisecond: with the helper on, the frames' ticks go to it.
            app.prefs.min_fps = 0;
            app.model_tick_secs = Some(0.001);
            app.overlap_ticks = overlap;
            app.set_target_speed(100.0);
            let id = app.world.bibites.iter().next().map(|(id, _)| id).expect("a bibite");
            let count = {
                let record = app.world.bibites.get_mut(id).expect("the bibite");
                record.body.dead = true;
                (record.body.geometry.base_body_area as i32) / 20
            };
            assert!(count > 0, "the burst has particles ({count})");
            assert!(app.world.kill_or_explode_bibite(id));
            let mut most = 0;
            let mut last_step = 0.0;
            for _ in 0..10 {
                let before = app.world.simulated_time();
                app.advance(0.25);
                app.rebuild_scene();
                most = most.max(crate::blood_particles::drawn(wide.0, wide.1).len());
                app.dispatch_overlapped_ticks();
                app.join_overlapped_ticks();
                last_step = app.world.simulated_time() - before;
            }
            assert!(last_step > 1.0, "a frame steps {last_step} s of game time");
            assert!(most > 0, "the burst showed (overlap {overlap})");
            assert!(
                crate::blood_particles::drawn(wide.0, wide.1).is_empty(),
                "the blood is gone (overlap {overlap})"
            );
        }
    }

    #[test]
    fn pheromone_particles_age_through_a_step_of_many_seconds() {
        use bibites_core::pheromone::PheroEmission;
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        app.paused = false;
        app.scene_options.pheromones = true;
        app.prefs.pheromone_opacity = 1.0;
        let at = app.camera.center;
        app.world.pheromones.emit(PheroEmission::bibite(at, Vec2::new(0.0, 1.0), [5.0, 0.0, 5.0]));
        for _ in 0..90 {
            app.advance(1.0 / 60.0);
        }
        let wide = (Vec2::new(-1.0e6, -1.0e6), Vec2::new(1.0e6, 1.0e6));
        assert!(!crate::phero_particles::drawn(wide.0, wide.1).is_empty());
        // A step of ten seconds of game time (a fast forward, a load): what is in the air lives a few seconds, so it is all past its life
        // (the step used to be skipped when it was over a second, and the particles stayed).
        let now = app.world.simulated_time();
        app.world.set_simulated_time(now + 10.0);
        crate::phero_particles::advance(&app.world, false, 1.0);
        assert!(crate::phero_particles::drawn(wide.0, wide.1).is_empty());
    }

    #[test]
    fn a_pheromone_spot_emits_particles_as_the_game_does_and_only_while_pheromones_show() {
        use bibites_core::pheromone::PheroEmission;
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        app.paused = false;
        app.scene_options.pheromones = true;
        app.prefs.pheromone_opacity = 1.0;
        let at = app.camera.center;
        // A spot with red and blue in it (the green is nothing, so no green particle may come of it).
        app.world.pheromones.emit(PheroEmission::bibite(at, Vec2::new(0.0, 1.0), [5.0, 0.0, 5.0]));
        for _ in 0..90 {
            app.advance(1.0 / 60.0);
        }
        let seen = crate::phero_particles::drawn(Vec2::new(-1000.0, -1000.0), Vec2::new(1000.0, 1000.0));
        assert!(!seen.is_empty(), "a spot with strength emits (time {}, spots {})", app.world.simulated_time(), app.world.pheromones.ids().count());
        // Every particle is inside the cone's reach of the spot: at most 3 s at the sideways part of 25 m/s, plus the base disc.
        assert!(seen.iter().all(|p| (p.pos - at).length() < 3.0 * 25.0 * 25.0_f32.to_radians().sin() + 1.5));
        // The scene holds them as diamonds.
        app.rebuild_scene();
        assert!(app.scene.primitives.iter().any(|p| matches!(p, Primitive::Diamond { .. })));
        // With the setting off nothing new comes.
        app.scene_options.pheromones = false;
        let before = crate::phero_particles::drawn(Vec2::new(-1000.0, -1000.0), Vec2::new(1000.0, 1000.0)).len();
        for _ in 0..600 {
            app.advance(1.0 / 60.0);
        }
        let after = crate::phero_particles::drawn(Vec2::new(-1000.0, -1000.0), Vec2::new(1000.0, 1000.0)).len();
        assert!(after <= before, "no emission while hidden ({before} -> {after})");
    }

    /// `PHERO_DUMP=<dir>`: writes a picture of the three pheromone colours as particles (for looking at).
    #[test]
    fn dump_the_pheromone_picture() {
        use bibites_core::pheromone::PheroEmission;
        let Ok(dir) = std::env::var("PHERO_DUMP") else {
            return;
        };
        let mut app = App::new(synthetic(66, 6), Vec2::new(900.0, 500.0), None);
        app.paused = false;
        app.scene_options.pheromones = true;
        app.prefs.pheromone_opacity = 1.0;
        app.camera.zoom = 8.0;
        let centre = app.camera.center;
        for (k, strength) in [[3.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 3.0], [3.0, 3.0, 3.0]].into_iter().enumerate() {
            let at = centre + Vec2::new(-30.0 + 20.0 * k as f32, 0.0);
            app.world.pheromones.emit(PheroEmission::bibite(at, Vec2::new(0.0, 1.0), strength));
        }
        for _ in 0..240 {
            app.advance(1.0 / 60.0);
        }
        app.rebuild_scene();
        let image = crate::render::render_scene(&app.scene, &app.camera, 900, 500, 1, &[]);
        let file = std::fs::File::create(format!("{dir}/phero.png")).unwrap();
        let mut encoder = png::Encoder::new(file, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&image.rgba).unwrap();
    }

    #[test]
    fn the_eggs_snapshot_follows_the_incubation() {
        let mut app = App::new(synthetic(67, 6), Vec2::new(1280.0, 800.0), None);
        let egg = selected_egg_of(&mut app);
        let before = app.selection.as_ref().and_then(|p| p.egg.clone());
        let before = before.expect("egg stats").hatch_progress;
        app.world
            .eggs
            .get_mut(egg)
            .expect("the egg")
            .hatch
            .hatch_progress = before + 1.5;
        app.refresh_overlay(0.0);
        let after = app.selection.as_ref().and_then(|p| p.egg.clone());
        assert!((after.expect("egg stats").hatch_progress - before - 1.5).abs() < 1e-5);
    }

    /// The used nodes of the inspector's diagram with their places.
    fn inspector_layout(app: &App) -> Vec<(usize, [f32; 2])> {
        let state = app.inspector_brain.as_ref().expect("a filled tab");
        state
            .diagram
            .used
            .iter()
            .map(|&node| (node, state.diagram.position(node).expect("a place")))
            .collect()
    }

    #[test]
    fn the_inspectors_brain_tab_is_laid_out_for_its_target_and_keeps_the_layout_with_it() {
        let mut app = App::new(synthetic(70, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        // Nothing is laid out before the tab shows.
        app.refresh_overlay(0.0);
        assert!(app.inspector_brain.is_none());
        app.handle_input(&digit(4));
        app.refresh_overlay(0.0);
        let target = app.selection.as_ref().expect("a snapshot").target;
        assert_eq!(app.inspector_brain.as_ref().map(|s| s.target), Some(target));
        let layout = inspector_layout(&app);
        assert!(layout.len() >= 4, "the starter brain's nodes: {layout:?}");
        // Every node is inside the holder (x from its left edge, y down from its top edge, negative).
        let geometry =
            crate::inspector_brain::Geometry::of(&app.inspector_brain.as_ref().unwrap().diagram);
        for (node, at) in &layout {
            assert!(
                at[0] >= -1.0 && at[0] <= geometry.width + 1.0,
                "node {node} x {at:?} in {geometry:?}"
            );
            assert!(
                at[1] <= 1.0 && at[1] >= -geometry.height - 1.0,
                "node {node} y {at:?} in {geometry:?}"
            );
        }
        // The counts of the info row: the nodes the diagram draws and every connection.
        let (nodes, synapses) = app.inspector_brain_counts().expect("counts");
        let brain = &app.world.bibites.get(app.selected.unwrap()).unwrap().brain;
        assert_eq!(nodes, layout.len());
        assert_eq!(synapses, brain.synapse_count());
        // A dragged neuron stays where it was put, also while the tab is closed and opened again.
        let (node, at) = layout[0];
        app.camera.screen = Vec2::new(1920.0, 1080.0);
        let holder = geometry.holder();
        let cursor = Vec2::new(holder[0] + at[0], holder[1] - at[1]);
        app.inspector_brain_press_node(node, cursor);
        app.inspector_brain_drag_to(cursor + Vec2::new(0.0, 12.0), false);
        app.inspector_brain_drag = None;
        let dragged = inspector_layout(&app);
        assert_ne!(dragged[0].1, at, "the neuron moved");
        app.handle_input(&digit(4));
        assert!(!app.inspector_visible());
        app.refresh_overlay(0.0);
        assert_eq!(
            inspector_layout(&app),
            dragged,
            "kept while the tab is closed"
        );
        app.handle_input(&digit(4));
        app.refresh_overlay(0.0);
        assert_eq!(inspector_layout(&app), dragged, "and when it opens again");
        // Another target has its own layout; no target, none.
        let other = app
            .world
            .live_bibite_ids()
            .into_iter()
            .find(|id| Some(*id) != app.selected)
            .expect("another bibite");
        app.select_bibite(other);
        app.refresh_overlay(0.0);
        assert_eq!(
            app.inspector_brain.as_ref().map(|s| s.target),
            Some(SelectionTarget::Bibite(other))
        );
        app.clear_target();
        app.refresh_overlay(0.0);
        assert!(app.inspector_brain.is_none());
    }

    #[test]
    fn sorting_lays_the_inspectors_diagram_out_again_by_the_button_and_by_the_keys() {
        let mut app = App::new(synthetic(71, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(4));
        app.refresh_overlay(0.0);
        let serial = |app: &App| app.inspector_brain.as_ref().expect("filled").serial();
        let mut seen = vec![serial(&app)];
        // The Sort button (`SortN()`).
        app.apply_ui_actions(&[UiAction::InspectorBrainSort]);
        seen.push(serial(&app));
        // F1 sorts once, F3 six times, F4 eight times (`BrainPanelCommon.UpdatePanel`).
        for keys in [
            [true, false, false],
            [false, true, false],
            [false, false, true],
        ] {
            app.handle_input(&FrameInput {
                brain_sort_keys: keys,
                dt: 0.016,
                ..FrameInput::default()
            });
            seen.push(serial(&app));
        }
        // Every sort made a new picture, and every used node still has its place.
        seen.dedup();
        assert_eq!(seen.len(), 5, "{seen:?}");
        assert!(inspector_layout(&app).len() >= 4);
        // The keys do nothing while the tab is closed.
        app.handle_input(&digit(4));
        let before = serial(&app);
        app.handle_input(&FrameInput {
            brain_sort_keys: [true, false, false],
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(serial(&app), before);
    }

    #[test]
    fn highlight_differences_marks_what_differs_from_the_species_template() {
        use crate::species_panel::brain::Mark;
        let mut app = App::new(synthetic(72, 6), Vec2::new(1280.0, 800.0), None);
        // The starter world has no species yet: record one from the first bibite's brain, then change the bibite's own.
        let ids = app.world.live_bibite_ids();
        let (genome, brain) = {
            let record = app.world.bibites.get(ids[0]).expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let species = app
            .world
            .species
            .add_template_species("Zzalpha zzone", "", &genome, &brain);
        for id in &ids {
            app.world.bibites.get_mut(*id).expect("record").meta.species = Some(species);
        }
        app.world.reconcile_species_counts();
        app.select_bibite(ids[0]);
        let synapse = {
            let brain = &mut app.world.bibites.get_mut(ids[0]).expect("record").brain;
            assert!(brain.synapse_count() > 0);
            let weight = brain.graph_view().synapses[0].weight;
            assert!(brain.set_synapse_weight(0, weight + 3.0));
            0
        };
        app.handle_input(&digit(4));
        app.refresh_overlay(0.0);
        // Off at first: no marks.
        let marks = |app: &App| app.inspector_brain.as_ref().expect("filled").marks.clone();
        assert!(!app.show_brain_differences);
        assert!(marks(&app).synapses.iter().all(|m| *m == Mark::None));
        // The toggle (shared with the species panel's) marks the changed weight.
        app.apply_ui_actions(&[UiAction::BrainToggleDifferences]);
        assert!(app.show_brain_differences);
        let shown = marks(&app);
        assert!(
            matches!(shown.synapses[synapse], Mark::Changed { .. }),
            "{:?}",
            shown.synapses[synapse]
        );
        assert!(
            shown
                .synapses
                .iter()
                .enumerate()
                .all(|(i, m)| i == synapse || *m == Mark::None)
        );
        // Off again: the marks go.
        app.apply_ui_actions(&[UiAction::BrainToggleDifferences]);
        assert!(marks(&app).synapses.iter().all(|m| *m == Mark::None));
    }

    #[test]
    fn an_eggs_brain_tab_is_laid_out_like_a_bibites() {
        let mut app = App::new(synthetic(73, 6), Vec2::new(1280.0, 800.0), None);
        let egg = selected_egg_of(&mut app);
        app.handle_input(&digit(3));
        assert_eq!(app.inspector_tab, InspectorTab::Brain);
        app.refresh_overlay(0.0);
        let state = app.inspector_brain.as_ref().expect("filled for the egg");
        assert_eq!(state.target, SelectionTarget::Egg(egg));
        let brain = &app.world.eggs.get(egg).expect("egg").brain;
        assert_eq!(state.diagram.graph.synapses.len(), brain.synapse_count());
        assert!(state.diagram.used.len() >= 4);
    }

    #[test]
    fn the_players_easter_egg_setting_reaches_the_world_and_its_children() {
        let is_easter_egg = |app: &App, id| {
            app.world.eggs.get(id).is_some_and(|egg| {
                egg.meta.is_rad || egg.meta.is_rain || egg.meta.is_source || egg.meta.is_killer
            })
        };
        let mut app = App::new(synthetic(74, 6), Vec2::new(1280.0, 800.0), None);
        let parent = app.world.live_bibite_ids()[0];
        // Off by default (`AllowEasterEggs`): no child is an easter egg.
        app.advance(0.05);
        assert!(!app.world.settings.allow_easter_eggs);
        for _ in 0..30 {
            app.world.lay_egg_once(parent);
        }
        let before: Vec<_> = app.world.eggs.ids().collect();
        assert!(before.len() >= 30);
        assert!(before.iter().all(|id| !is_easter_egg(&app, *id)));
        // Allowed with a period of 1 (`EasterEggsProbability`): the world gets both, and every mutated child is one.
        app.allow_easter_eggs = true;
        app.easter_eggs_period = 1;
        app.advance(0.05);
        assert!(app.world.settings.allow_easter_eggs && app.world.settings.easter_eggs_period == 1);
        for _ in 0..30 {
            app.world.lay_egg_once(parent);
        }
        let new: Vec<_> = app
            .world
            .eggs
            .ids()
            .filter(|id| !before.contains(id))
            .collect();
        assert!(new.len() >= 30, "{}", new.len());
        assert!(new.iter().all(|id| is_easter_egg(&app, *id)));
    }

    #[test]
    fn selecting_a_bibite_replaces_the_eggs_snapshot() {
        let mut app = App::new(synthetic(68, 6), Vec2::new(1280.0, 800.0), None);
        selected_egg_of(&mut app);
        assert!(app.selection.as_ref().is_some_and(|p| p.egg.is_some()));
        let id = app.world.live_bibite_ids()[0];
        app.select_bibite(id);
        let panel = app.selection.as_ref().expect("the bibite's snapshot");
        assert_eq!(panel.target, SelectionTarget::Bibite(id));
        assert!(panel.egg.is_none() && app.selected_egg.is_none());
    }

    #[test]
    fn the_right_button_deselects_the_target() {
        let mut app = App::new(synthetic(63, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        app.handle_input(&FrameInput {
            secondary_down: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected.is_none() && app.selection.is_none());
    }

    #[test]
    fn a_hotkey_pick_ends_with_its_death_but_a_click_pick_lingers_as_a_corpse() {
        let mut app = App::new(synthetic(64, 6), Vec2::new(1280.0, 800.0), None);
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        let id = app.selected.expect("R picked a bibite");
        assert!(app.world.kill_or_explode_bibite(id));
        app.advance(0.05);
        assert!(app.selected.is_none(), "ReselectOnCorpses ends the pick");

        // A click pick has no death listener: the corpse stays selected.
        let id = app.world.live_bibite_ids()[0];
        app.select_bibite(id);
        assert!(app.world.kill_or_explode_bibite(id));
        app.advance(0.05);
        assert_eq!(app.selected, Some(id));

        // With the setting off a hotkey pick lingers too.
        app.reselect_on_corpses = false;
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        let id = app.selected.expect("R picked a bibite");
        assert!(app.world.kill_or_explode_bibite(id));
        app.advance(0.05);
        assert_eq!(app.selected, Some(id));
    }

    #[test]
    fn ctrl_r_repeats_the_pick_after_the_reselection_delay_until_escape() {
        let mut app = App::new(synthetic(65, 8), Vec2::new(1280.0, 800.0), None);
        app.reselection_delay = 0.5;
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            ctrl_down: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        let first = app.selected.expect("picked");
        assert_eq!(app.reselect, Some(super::Reselect::Random));
        assert!(app.world.kill_or_explode_bibite(first));
        // The dead target stays while the delay runs.
        app.advance(0.25);
        assert_eq!(app.selected, Some(first));
        // Once it ran out another living bibite is picked and the repeat stays armed.
        app.advance(0.25);
        app.advance(0.25);
        let next = app.selected.expect("a new pick");
        assert_ne!(next, first);
        assert!(!app.world.bibites.get(next).expect("alive").body.dead);
        assert_eq!(app.reselect, Some(super::Reselect::Random));
        // Escape deselects and cancels the repeat.
        app.handle_input(&escape());
        assert!(app.selected.is_none() && app.reselect.is_none());
    }

    #[test]
    fn arrow_keys_only_move_the_camera_when_the_setting_allows() {
        let mut app = App::new(synthetic(66, 4), Vec2::new(1280.0, 800.0), None);
        let start = app.camera.center;
        let held = FrameInput {
            move_axis: Vec2::new(1.0, 0.0),
            dt: 0.1,
            ..FrameInput::default()
        };
        app.allow_arrow_keys = false;
        app.handle_input(&held);
        assert_eq!(app.camera.center, start);
        app.allow_arrow_keys = true;
        app.handle_input(&held);
        assert_ne!(app.camera.center, start);
    }

    #[test]
    fn toolbar_panels_open_on_hover_after_half_a_second_and_close_half_a_second_after_leaving() {
        use super::HoverPanel;
        let mut panel = HoverPanel::default();
        // Resting on the button: nothing before 0.5 s.
        for _ in 0..4 {
            panel.update(true, 0.12);
        }
        assert!(!panel.open);
        panel.update(true, 0.12);
        assert!(panel.open);
        // Leaving: it stays for 0.5 s ...
        for _ in 0..4 {
            panel.update(false, 0.12);
        }
        assert!(panel.open);
        // ... and coming back within that time keeps it.
        panel.update(true, 0.12);
        for _ in 0..10 {
            panel.update(true, 0.12);
        }
        assert!(panel.open);
        panel.update(false, 0.12);
        for _ in 0..5 {
            panel.update(false, 0.12);
        }
        assert!(!panel.open);
        // A brush past the button (shorter than the open delay) never opens it.
        panel.update(true, 0.2);
        panel.update(false, 0.12);
        for _ in 0..10 {
            panel.update(false, 0.12);
        }
        assert!(!panel.open);
        // Pressing the button shows the panel at once.
        panel.show();
        assert!(panel.open);
    }

    #[test]
    fn the_toolbar_follows_the_pointer_zones_and_a_press_opens_at_once() {
        let mut app = App::new(synthetic(55, 4), Vec2::new(1280.0, 800.0), None);
        let zones = [
            Some((Vec2::new(0.0, 700.0), Vec2::new(50.0, 100.0))),
            Some((Vec2::new(60.0, 750.0), Vec2::new(50.0, 50.0))),
            None,
        ];
        app.update_toolbar_hover(&zones, Vec2::new(10.0, 720.0), 0.3);
        assert!(!app.toolbar[0].open);
        app.update_toolbar_hover(&zones, Vec2::new(10.0, 720.0), 0.3);
        assert!(app.toolbar[0].open && !app.toolbar[1].open);
        // The pointer moves onto the second button and presses it: its panel shows at once.
        app.update_toolbar_hover(&zones, Vec2::new(70.0, 760.0), 0.01);
        app.apply_ui_actions(&[UiAction::ShowToolbar(1)]);
        assert!(app.toolbar[1].open);
        // It leaves everything: both close after the delay, independently.
        app.update_toolbar_hover(&zones, Vec2::new(500.0, 100.0), 0.3);
        assert!(app.toolbar[0].open && app.toolbar[1].open);
        app.update_toolbar_hover(&zones, Vec2::new(500.0, 100.0), 0.3);
        assert!(!app.toolbar[0].open && !app.toolbar[1].open);
    }

    #[test]
    fn a_held_bibite_follows_the_cursor_but_the_selected_one_does_not() {
        let mut app = App::new(synthetic(77, 8), Vec2::new(1280.0, 800.0), None);
        app.speed = 2.0;
        let id = app.world.live_bibite_ids()[0];
        let at = app
            .camera
            .world_to_screen(app.world.bibites.get(id).unwrap().pos);
        let away = at + Vec2::new(120.0, 0.0);
        // A quick press is a click, not a drag: the bibite is only selected.
        app.handle_input(&press(at, true, true, false, 0.016));
        assert_eq!(app.world.bibites.get(id).unwrap().vel, Vec2::ZERO);
        app.handle_input(&press(at, false, false, true, 0.016));
        assert_eq!(app.selected, Some(id));
        // The selected one never follows the cursor, however long it is held.
        app.handle_input(&press(at, true, true, false, 0.016));
        app.handle_input(&press(away, false, true, false, 0.3));
        assert_eq!(app.world.bibites.get(id).unwrap().vel, Vec2::ZERO);
        app.handle_input(&press(away, false, false, true, 0.016));

        // Another bibite, held past the click time, heads for the cursor at 10 x offset / timeScale.
        app.clear_target();
        let other = app.world.live_bibite_ids()[1];
        let pos = app.world.bibites.get(other).unwrap().pos;
        let start = app.camera.world_to_screen(pos);
        let target_screen = start + Vec2::new(0.0, -90.0);
        app.handle_input(&press(start, true, true, false, 0.016));
        app.handle_input(&press(target_screen, false, true, false, 0.3));
        let record = app.world.bibites.get(other).unwrap();
        let wanted = (app.camera.screen_to_world(target_screen) - record.pos) * (10.0 / 2.0);
        assert!(
            (record.vel - wanted).length() < 1e-3,
            "{:?} vs {wanted:?}",
            record.vel
        );
        app.handle_input(&press(target_screen, false, false, true, 0.016));
        assert!(app.selected.is_none(), "a held press never selects");

        // Paused, it jumps to the cursor instead.
        app.paused = true;
        let third = app.world.live_bibite_ids()[2];
        let start = app
            .camera
            .world_to_screen(app.world.bibites.get(third).unwrap().pos);
        let there = start + Vec2::new(60.0, 40.0);
        app.handle_input(&press(start, true, true, false, 0.016));
        app.handle_input(&press(there, false, true, false, 0.3));
        let moved = app.world.bibites.get(third).unwrap().pos;
        assert!((moved - app.camera.screen_to_world(there)).length() < 1e-3);
        app.handle_input(&press(there, false, false, true, 0.016));
    }

    #[test]
    fn a_pellet_is_dragged_at_once_and_nothing_is_dragged_in_a_challenge() {
        let mut app = App::new(synthetic(78, 4), Vec2::new(1280.0, 800.0), None);
        app.paused = true;
        let pos = Vec2::new(0.0, 0.0);
        let pellet = app
            .world
            .spawn_pellet(bibites_core::pellet::PelletSpawn::new(
                bibites_core::pellet::MaterialKind::Plant,
                20.0,
                pos,
            ))
            .expect("a pellet");
        let start = app.camera.world_to_screen(pos);
        let there = start + Vec2::new(50.0, 30.0);
        app.handle_input(&press(start, true, true, false, 0.016));
        // No click time to wait for: the first held frame already moves it.
        app.handle_input(&press(there, false, true, false, 0.016));
        let moved = app.world.pellets.pos(pellet).expect("pellet");
        assert!((moved - app.camera.screen_to_world(there)).length() < 1e-3);
        app.handle_input(&press(there, false, false, true, 0.016));

        // In a challenge the objects stay where they are.
        app.start_local_challenge();
        let before = app.world.pellets.pos(pellet).expect("pellet");
        let start = app.camera.world_to_screen(before);
        app.handle_input(&press(start, true, true, false, 0.016));
        app.handle_input(&press(
            start + Vec2::new(40.0, 0.0),
            false,
            true,
            false,
            0.3,
        ));
        assert_eq!(app.world.pellets.pos(pellet), Some(before));
    }

    #[test]
    fn the_wheel_zooms_at_once_about_the_screen_centre() {
        let mut app = App::new(synthetic(79, 4), Vec2::new(1280.0, 800.0), None);
        let centre = app.camera.center;
        let zoom = app.camera.zoom;
        app.handle_input(&FrameInput {
            wheel: 1.0,
            cursor: Vec2::new(100.0, 100.0),
            dt: 0.016,
            ..FrameInput::default()
        });
        // One notch in: the ortho size shrinks by a tenth, the camera does not move, and the
        // zoom is already there (no easing frames).
        assert_eq!(app.camera.center, centre);
        assert!(app.camera.zoom > zoom, "zoomed in");
        assert!(app.zoom_target.is_none());
        let after_one = app.camera.zoom;
        app.advance(0.016);
        assert_eq!(app.camera.zoom, after_one, "nothing eases afterwards");
    }

    #[test]
    fn a_drag_rectangle_picks_the_entities_inside_and_delete_destroys_them() {
        let mut app = App::new(synthetic(37, 8), Vec2::new(1280.0, 800.0), None);
        let whole = Vec2::new(1280.0, 800.0);
        app.handle_input(&press(Vec2::new(1.0, 1.0), true, true, false, 0.016));
        app.handle_input(&press(whole, false, true, false, 0.3));
        app.handle_input(&press(whole, false, false, true, 0.016));
        let picked = app.multi.as_ref().expect("a multi-selection").len();
        assert!(picked > 1, "picked {picked}");
        assert!(app.selected.is_none());
        let panel = app.multi.as_ref().unwrap();
        assert_eq!(panel.info.kinds[0].count, panel.bibites.len());

        app.handle_input(&FrameInput {
            kill_target: true,
            ..FrameInput::default()
        });
        assert!(app.multi.is_none(), "deleting closes the panel");
        assert!(
            app.world
                .bibites
                .iter()
                .any(|(_, record)| record.body.dying || record.body.dead)
        );
    }

    #[test]
    fn a_rectangle_with_a_target_selected_only_clears_the_target() {
        // `UserControl.SelectObjectsInRectangle` opens the panel and then deselects the target,
        // which closes the panel again.
        let mut app = App::new(synthetic(38, 8), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let whole = Vec2::new(1280.0, 800.0);
        app.handle_input(&press(Vec2::new(1.0, 1.0), true, true, false, 0.016));
        app.handle_input(&press(whole, false, true, false, 0.3));
        app.handle_input(&press(whole, false, false, true, 0.016));
        assert!(app.selected.is_none());
        assert!(app.multi.is_none());
    }

    #[test]
    fn clicking_empty_space_keeps_the_current_target() {
        let mut app = App::new(synthetic(39, 4), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let target = app.selected;
        // The screen point furthest from every bibite (so outside all selection radii).
        let radius_px = app.selection_radius * app.camera.screen.y / 2.0;
        let bibites: Vec<(Vec2, f32)> = app
            .world
            .bibites
            .iter()
            .map(|(_, record)| {
                (
                    app.camera.world_to_screen(record.pos),
                    record.body.geometry.effective_radius * app.camera.zoom,
                )
            })
            .collect();
        let clearance = |point: Vec2| {
            bibites
                .iter()
                .map(|(at, body)| point.distance(*at) - body)
                .fold(f32::MAX, f32::min)
        };
        let mut best = (Vec2::ZERO, f32::MIN);
        for x in 0..16 {
            for y in 0..10 {
                let point = Vec2::new(40.0 + x as f32 * 80.0, 40.0 + y as f32 * 80.0);
                if clearance(point) > best.1 {
                    best = (point, clearance(point));
                }
            }
        }
        assert!(best.1 > radius_px + 1.0, "no empty spot: {best:?}");
        app.handle_input(&press(best.0, true, true, false, 0.016));
        app.handle_input(&press(best.0, false, false, true, 0.016));
        assert_eq!(app.selected, target);
    }

    #[test]
    fn autosave_names_use_local_time_like_the_game() {
        use super::autosave_timestamp_at;
        // 2025-10-09T08:53:20Z.
        assert_eq!(autosave_timestamp_at(1_760_000_000, 0), "20251009085320");
        assert_eq!(autosave_timestamp_at(1_760_000_000, -300), "20251009035320");
        assert_eq!(autosave_timestamp_at(1_760_000_000, 330), "20251009142320");
        // 2025-10-09T02:00:00Z is still the previous evening five hours behind UTC.
        assert_eq!(autosave_timestamp_at(1_759_975_200, -300), "20251008210000");
    }

    #[test]
    fn bibite_and_egg_exports_go_to_the_bibites_folder_not_the_saves_folder() {
        let dir = std::env::temp_dir().join(format!("rustibites-exports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(35, 6), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        assert!(app.select_first());
        app.save_selected_entity();
        let exported: Vec<_> = std::fs::read_dir(app.storage.bibites_dir())
            .expect("Bibites folder")
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "bb8"))
            .collect();
        assert_eq!(exported.len(), 1, "{}", app.last_action);
        // (The `Autosaves` sub-folder is part of the layout; only files would be saves.)
        let in_saves = std::fs::read_dir(app.storage.saves_dir())
            .map(|read| {
                read.flatten()
                    .filter(|entry| entry.path().is_file())
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(
            in_saves, 0,
            "single-bibite saves must not land in Savefiles"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn spawn_bibite_command_adds_a_bibite() {
        let mut app = App::new(synthetic(34, 4), Vec2::new(1280.0, 800.0), None);
        let before = app.world.live_bibite_ids().len();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SpawnBibite)]);
        assert_eq!(
            app.world.live_bibite_ids().len(),
            before + 1,
            "{}",
            app.last_action
        );
    }

    #[test]
    fn kill_half_dialog_confirms_and_cancels() {
        let mut app = App::new(synthetic(41, 10), Vec2::new(1280.0, 800.0), None);
        let before = app.world.live_bibite_ids().len();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::RequestKillHalf)]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::KillHalf));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Cancel)]);
        assert!(app.confirm_dialog.is_none());
        assert_eq!(
            app.world.live_bibite_ids().len(),
            before,
            "cancel killed bibites"
        );

        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::RequestKillHalf)]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        assert!(app.last_action.contains("half"), "{}", app.last_action);
    }

    #[test]
    fn remove_half_of_the_pellets_asks_first_and_keeps_the_selection() {
        let mut app = App::new(synthetic(43, 10), Vec2::new(1280.0, 800.0), None);
        for index in 0..300 {
            let pos = Vec2::new(index as f32 * 0.4, 2.0);
            let _ = app
                .world
                .spawn_pellet(bibites_core::pellet::PelletSpawn::new(
                    bibites_core::pellet::MaterialKind::Plant,
                    4.0,
                    pos,
                ));
        }
        assert!(app.select_first());
        let before = app.world.pellets.ids().count();
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::RequestRemoveHalfPellets,
        )]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::RemoveHalfPellets));
        assert_eq!(app.world.pellets.ids().count(), before, "asked, not done");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Cancel)]);
        assert!(app.confirm_dialog.is_none());
        assert_eq!(app.world.pellets.ids().count(), before);

        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::RequestRemoveHalfPellets,
        )]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        let after = app.world.pellets.ids().count();
        assert!(after < before && after > before / 4, "{before} -> {after}");
        assert!(app.selected.is_some(), "removing pellets never deselects");
    }

    #[test]
    fn killing_half_keeps_the_selection_for_a_bibite_that_survives() {
        let mut app = App::new(synthetic(44, 12), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let picked = app.selected;
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::RequestKillHalf)]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        // A click-picked bibite stays selected whether it died (a corpse) or not.
        assert_eq!(app.selected, picked);
    }

    #[test]
    fn view_panel_switches_flip_their_own_setting_and_never_the_hud() {
        use crate::ui::OverlayKind;
        let mut app = App::new(synthetic(45, 4), Vec2::new(1280.0, 800.0), None);
        // The game's defaults: bars off, field of view on, targets off.
        assert!(!app.show_status_bars && app.scene_options.fov && !app.show_targets);
        let hud = app.show_hud;
        app.apply_ui_actions(&[
            UiAction::ToggleOverlay(OverlayKind::StatusBars),
            UiAction::ToggleOverlay(OverlayKind::FieldOfView),
            UiAction::ToggleOverlay(OverlayKind::Targets),
        ]);
        assert!(app.show_status_bars && !app.scene_options.fov && app.show_targets);
        assert_eq!(app.show_hud, hud);
    }

    #[test]
    fn the_gear_opens_the_pause_menu_and_pauses_until_it_closes() {
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(46, 4), Vec2::new(1280.0, 800.0), None);
        assert!(!app.paused);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenEscapeMenu)]);
        assert!(app.panels.is_open(PanelKind::Escape) && app.paused);
        // Pressing it again does not stack another menu.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenEscapeMenu)]);
        app.handle_input(&escape());
        assert!(!app.panels.is_open(PanelKind::Escape));
        app.advance(0.016);
        assert!(!app.paused, "the previous pause state comes back");
    }

    #[test]
    fn the_pause_menu_stays_open_under_the_panels_it_opens_and_comes_back_when_they_close() {
        use crate::ui::{ControlCommand, PanelKind};
        // `UserSettingsManager`, `SaveGamePanel`, `LoadGamePanel` and the Library open over `EscapePanel` and leave it active.
        for (command, panel) in [
            (ControlCommand::OpenSettings, PanelKind::Settings),
            (ControlCommand::OpenSavePanel, PanelKind::SaveGame),
            (ControlCommand::OpenLoadPanel, PanelKind::LoadGame),
            (ControlCommand::OpenLibrary, PanelKind::Library),
        ] {
            let dir = std::env::temp_dir().join(format!(
                "rustibites-pause-stack-{}-{panel:?}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let mut app = App::new(synthetic(53, 4), Vec2::new(1280.0, 800.0), None);
            app.storage = StoragePaths::new(dir.clone());
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
            assert_eq!(app.panels.top(), Some(PanelKind::Escape));
            app.apply_ui_actions(&[UiAction::Invoke(command)]);
            assert!(app.panels.is_open(PanelKind::Escape), "{panel:?}");
            assert_eq!(app.panels.top(), Some(panel));
            app.advance(0.05);
            assert!(app.paused, "{panel:?}");
            // Escape (`UINavigationManager`) closes the newest entry: the pause menu is the top one again, still pausing.
            app.handle_input(&escape());
            assert!(!app.panels.is_open(panel), "{panel:?}");
            assert_eq!(app.panels.top(), Some(PanelKind::Escape), "{panel:?}");
            app.advance(0.05);
            assert!(app.paused, "{panel:?}");
            // The next one closes the pause menu and the game runs as it did before.
            app.handle_input(&escape());
            assert!(!app.panels.is_open(PanelKind::Escape), "{panel:?}");
            app.advance(0.05);
            assert!(!app.paused, "{panel:?}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_save_closes_the_save_panel_and_shows_the_pause_menu_again() {
        use crate::ui::{ControlCommand, PanelKind};
        let dir =
            std::env::temp_dir().join(format!("rustibites-pause-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(54, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        // The panel opens with an empty name and a Save button that does nothing.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SaveAs)]);
        assert!(app.panels.is_open(PanelKind::SaveGame));
        app.save_panel.as_mut().expect("the panel").name = "from-the-pause-menu".to_owned();
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SaveAs)]);
        // `SaveGamePanel.Save`: `SaveWorld`, then `ClosePanel`.
        assert!(
            app.save_entries
                .iter()
                .any(|entry| entry.name.starts_with("from-the-pause-menu"))
        );
        assert!(!app.panels.is_open(PanelKind::SaveGame));
        assert_eq!(app.panels.top(), Some(PanelKind::Escape));
        // Opened again the panel starts empty; a row puts its name in the field, and saving over a file asks first
        // (`AskOverwritePreviousSave`): cancelling keeps the panel, OK saves and closes it.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        assert_eq!(app.save_panel.as_ref().expect("the panel").name, "");
        app.apply_ui_actions(&[UiAction::SavePanel(crate::save_panel::SaveAction::Row(0))]);
        assert_eq!(
            app.save_panel.as_ref().expect("the panel").name,
            "from-the-pause-menu"
        );
        app.apply_ui_actions(&[UiAction::SavePanel(crate::save_panel::SaveAction::Save)]);
        assert!(matches!(
            app.confirm_dialog,
            Some(ConfirmKind::OverwriteSave(_))
        ));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        assert!(app.panels.is_open(PanelKind::SaveGame));
        app.apply_ui_actions(&[UiAction::SavePanel(crate::save_panel::SaveAction::Save)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(!app.panels.is_open(PanelKind::SaveGame));
        assert_eq!(app.panels.top(), Some(PanelKind::Escape));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_load_from_the_pause_menu_ends_it_and_the_game_runs_unless_pause_after_load_is_on() {
        use crate::ui::{ControlCommand, PanelKind};
        let dir =
            std::env::temp_dir().join(format!("rustibites-pause-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(55, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.save_named("to-load");
        for pause_after_load in [false, true] {
            app.prefs.pause_after_load = pause_after_load;
            app.paused = false;
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenLoadPanel)]);
            assert!(
                app.panels.is_open(PanelKind::Escape) && app.panels.is_open(PanelKind::LoadGame)
            );
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::LoadSelectedSave)]);
            // The scene is built again: the Escape stack is empty, the menu's pause is gone with it (a fresh `TimeController`).
            assert!(app.panels.is_empty(), "{:?}", app.panels);
            app.advance(0.05);
            assert_eq!(
                app.paused, pause_after_load,
                "PauseAfterLoad {pause_after_load}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_right_hand_panels_replace_each_other() {
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(47, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Graphs)]);
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Information)]);
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::WorldDynamics)]);
        assert!(app.panels.is_open(PanelKind::WorldDynamics));
        assert!(!app.panels.is_open(PanelKind::Graphs));
        // The quick-statistics popup is not one of the three: it stays "open", but is inactive (not shown) while the settings panel is.
        assert!(app.panels.is_open(PanelKind::Information));
        assert!(app.quick_stats_hidden());
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Graphs)]);
        assert!(app.panels.is_open(PanelKind::Graphs));
        assert!(!app.panels.is_open(PanelKind::WorldDynamics));
        assert!(!app.quick_stats_hidden(), "closing the settings panel activates `QuickStatsHolder` again");
    }

    /// `DynamicSettingsPanel.onOpen` deactivates `Right Panels/QuickStatsHolder` and `onClose` activates it again: the Information panel lay over the
    /// settings panel (over the animator creator's OK button), and nothing of it may be drawn or answer a click while that panel is open.
    #[test]
    fn the_information_panel_is_not_drawn_nor_clicked_while_the_settings_panel_is_open() {
        use crate::ui::{ControlCommand, PanelKind, UiAssets, UiFrame};
        let mut app = App::new(synthetic(47, 4), Vec2::new(1920.0, 1080.0), None);
        let draw = |app: &App| {
            let mut frame = UiFrame::new(1.0);
            crate::hud::draw_extra_panels(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0, false);
            frame
        };
        let has = |frame: &UiFrame, wanted: UiAction| frame.actions.iter().any(|(_, action)| *action == wanted);
        let close = UiAction::Invoke(ControlCommand::InformationClose);
        // The panel is up and answers.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenInformation)]);
        let shown = draw(&app);
        assert!(has(&shown, close), "the Information panel is up");
        let body = shown
            .actions
            .iter()
            .filter(|(_, action)| *action == UiAction::Consume)
            .filter_map(|(id, _)| shown.hits.iter().find(|(hit, ..)| hit == id))
            .map(|(_, pos, size)| (*pos, *size))
            .max_by(|a, b| (a.1.x * a.1.y).total_cmp(&(b.1.x * b.1.y)))
            .expect("its body takes the clicks");
        // (A point just inside its top left corner: no row or button is there.)
        let inside = body.0 + Vec2::new(4.0, 4.0);
        assert_eq!(shown.action_at(inside), Some(UiAction::Consume));
        // The settings panel opens: the Information panel is neither drawn nor answers (a click inside its old rectangle falls through).
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::WorldDynamics)]);
        let hidden = draw(&app);
        assert!(!has(&hidden, close), "no close button");
        assert_eq!(hidden.action_at(inside), None, "no body to take the click");
        assert!(hidden.draws.len() < shown.draws.len(), "nothing of it is drawn");
        // It is only inactive: it comes back, as it was, with the settings panel closed.
        assert!(app.panels.is_open(PanelKind::Information));
        app.apply_ui_actions(&[UiAction::ClosePanel]);
        assert!(!app.panels.is_open(PanelKind::WorldDynamics));
        assert!(app.panels.is_open(PanelKind::Information), "the settings panel closed, not the Information panel");
        let back = draw(&app);
        assert!(has(&back, close));
        assert_eq!(back.action_at(inside), Some(UiAction::Consume));
    }

    /// The panels' frame as the window builds it (the panels over the world's HUD), without the captured runtime frame.
    fn ids_frame(app: &App) -> crate::ui::UiFrame {
        let mut frame = crate::ui::UiFrame::new(1.0);
        crate::hud::draw_extra_panels(&mut frame, app, &crate::ui::UiAssets::empty(), None, 1920.0, 1080.0, false);
        frame
    }

    /// What a press and a release on `at` do: the actions `UiFrame::interact` gives (the window's pointer routing applies exactly these).
    fn ids_click(frame: &crate::ui::UiFrame, at: Vec2) -> Vec<UiAction> {
        frame.interact(at, true, true, &mut crate::ui::UiInteraction::default())
    }

    /// The middle of the click area that carries `wanted` (the topmost one, when several panels have such an action: a close button).
    fn ids_centre(frame: &crate::ui::UiFrame, wanted: UiAction) -> Vec2 {
        let (id, _) = frame.actions.iter().rev().find(|(_, action)| *action == wanted).unwrap_or_else(|| panic!("{wanted:?} is not in the frame"));
        let (_, pos, size) = frame.hits.iter().rev().find(|(hit, ..)| hit == id).expect("its rectangle");
        *pos + *size * 0.5
    }

    /// `UiFrame::interact` answers a click with every action registered under the id of the area it hit, so panels that hang over each other must have ids
    /// of their own. The Save Bibite dialog and the Information panel were both on 8800..: a click on the dialog's panel closed the Information panel behind
    /// it, one on its Name field swapped that panel's count / energy section.
    #[test]
    fn a_click_on_the_save_dialog_fires_nothing_of_the_information_panel_under_it() {
        use crate::ui::ControlCommand;
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        assert!(app.select_first());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenInformation)]);
        app.information.update(&app.world, 0.0);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SaveSelected)]);
        assert!(app.save_dialog.is_some(), "the dialog is open over the panel");
        let frame = ids_frame(&app);
        assert!(frame.actions.iter().any(|(_, action)| *action == UiAction::Invoke(ControlCommand::InformationClose)), "the panel is still there under it");
        assert_eq!(frame.shared_ids(), Vec::<u32>::new(), "no two click areas share an id");
        // The dialog's panel is the smaller of the two areas that swallow a click at the middle of the screen (the blocker covers the screen).
        let middle = Vec2::new(960.0, 540.0);
        let panel = frame
            .hits
            .iter()
            .zip(&frame.actions)
            .filter(|((_, pos, size), (_, action))| {
                *action == UiAction::Consume && middle.x >= pos.x && middle.y >= pos.y && middle.x < pos.x + size.x && middle.y < pos.y + size.y
            })
            .map(|((_, pos, size), _)| (*pos, *size))
            .min_by(|a, b| (a.1.x * a.1.y).total_cmp(&(b.1.x * b.1.y)))
            .expect("the dialog's panel");
        assert!(panel.1.x < 600.0, "that is the panel, not the blocker: {panel:?}");
        assert_eq!(ids_click(&frame, panel.0 + Vec2::new(3.0, 3.0)), vec![UiAction::Consume], "its margin only swallows the click");
        let name = ids_centre(&frame, UiAction::SaveDialogFocus(0));
        assert_eq!(ids_click(&frame, name), vec![UiAction::SaveDialogFocus(0)], "the Name field only takes the focus");
        assert_eq!(ids_click(&frame, Vec2::new(5.0, 5.0)), vec![UiAction::Consume], "the blocker");
        // The buttons of the dialog do their own thing alone.
        let cancel = ids_centre(&frame, UiAction::Invoke(ControlCommand::SaveDialogCancel));
        assert_eq!(ids_click(&frame, cancel), vec![UiAction::Invoke(ControlCommand::SaveDialogCancel)]);
    }

    /// The Flow row's pop-up of the Biology tab hangs over the Information panel: a click on its Sort toggle toggled the sort and closed the panel (8801).
    #[test]
    fn a_click_on_the_flow_pop_ups_sort_toggle_fires_nothing_of_the_information_panel() {
        use crate::ui::ControlCommand;
        let mut app = App::new(synthetic(66, 6), Vec2::new(1920.0, 1080.0), None);
        assert!(app.select_first());
        app.handle_input(&digit(3));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenInformation)]);
        app.information.update(&app.world, 0.0);
        let mut frame = ids_frame(&app);
        crate::biology_popups::draw_flow_popup(&mut frame, &app, &crate::ui::UiAssets::empty(), None, 1920.0, 1080.0, 223.0);
        assert!(frame.actions.iter().any(|(_, action)| *action == UiAction::Invoke(ControlCommand::InformationClose)), "the panel is there");
        assert_eq!(frame.shared_ids(), Vec::<u32>::new(), "no two click areas share an id");
        let sort = ids_centre(&frame, UiAction::ToggleEnergySort);
        assert_eq!(ids_click(&frame, sort), vec![UiAction::ToggleEnergySort]);
    }

    /// The pause menu hangs over the Graphs page: its veil (8100) was the page's exit button too, so a click on the dimmed screen closed the menu.
    #[test]
    fn a_click_on_the_pause_menus_veil_fires_nothing_of_the_graphs_page_under_it() {
        use crate::ui::{ControlCommand, PanelKind};
        let mut app = App::new(synthetic(47, 4), Vec2::new(1920.0, 1080.0), None);
        app.apply_ui_actions(&[UiAction::OpenPanel(PanelKind::Graphs)]);
        // The page alone: its exit button closes it, and nothing else answers there.
        let page = ids_frame(&app);
        assert_eq!(page.shared_ids(), Vec::<u32>::new());
        let exit = ids_centre(&page, UiAction::ClosePanel);
        assert_eq!(ids_click(&page, exit), vec![UiAction::ClosePanel]);
        // The menu over it: its veil only swallows the click.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
        assert_eq!(app.panels.top(), Some(PanelKind::Escape));
        let both = ids_frame(&app);
        assert_eq!(both.shared_ids(), Vec::<u32>::new(), "no two click areas share an id");
        assert_eq!(ids_click(&both, Vec2::new(40.0, 540.0)), vec![UiAction::Consume], "the veil outside the menu's section");
        // (The menu's own close button still closes it.)
        assert_eq!(ids_click(&both, ids_centre(&both, UiAction::ClosePanel)), vec![UiAction::ClosePanel]);
    }

    /// The Information panel's tags list has a row for every tag the world has, scrolled in a window of seven: a row on `8830 + its index` was on the id of
    /// the list's scroll bar from the 31st tag on, so a click on that row also paged the list.
    #[test]
    fn the_information_panels_tag_rows_never_share_an_id_with_its_scroll_bar() {
        use crate::ui::ControlCommand;
        let mut app = App::new(synthetic(47, 35), Vec2::new(1920.0, 1080.0), None);
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        for (n, id) in ids.iter().enumerate() {
            app.world.bibites.get_mut(*id).expect("record").meta.species_tag = Some(format!("tag {n}"));
        }
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenInformation)]);
        app.information.update(&app.world, 0.0);
        let first = ids_frame(&app);
        let tags = app.information.snapshot().expect("snapshot").tags.len();
        assert!(tags >= 35, "{tags} tags");
        // The wheel over the list takes it to its end (it needs the list's rectangle of a drawn frame).
        let over_list = ids_centre(&first, UiAction::Invoke(ControlCommand::InformationSelectTag(0)));
        assert!(app.information.wheel(over_list, -10_000.0));
        let frame = ids_frame(&app);
        let late: Vec<(u32, UiAction)> = frame
            .actions
            .iter()
            .filter_map(|(_, action)| match action {
                UiAction::Invoke(ControlCommand::InformationSelectTag(n)) if *n >= 30 => Some((*n, *action)),
                _ => None,
            })
            .collect();
        assert!(late.len() >= 2, "rows past the 30th are on screen: {late:?}");
        assert_eq!(frame.shared_ids(), Vec::<u32>::new(), "no two click areas share an id");
        for (n, action) in late {
            assert_eq!(ids_click(&frame, ids_centre(&frame, action)), vec![action], "row {n} answers alone");
        }
        let handle = ids_centre(&frame, UiAction::InformationScroll(true));
        assert_eq!(ids_click(&frame, handle), vec![UiAction::InformationScroll(true)]);
    }

    /// `SpeciesPanel.OpenPanel` calls `GlobalLineageManager.UpdateEffectiveSpeciesSpan()`: the used span takes the species that are active when the
    /// Lineage panel opens (the ones that died out left the list at the species log without the span being recounted), and the births that follow are
    /// judged by it.
    #[test]
    fn opening_the_lineage_panel_gives_the_used_span_the_species_that_are_active_then() {
        let mut app = App::new(synthetic(63, 4), Vec2::new(1280.0, 800.0), None);
        let first = app.world.live_bibite_ids()[0];
        let (genome, brain) = {
            let record = app.world.bibites.get(first).expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let species: Vec<_> = (0..25)
            .map(|n| app.world.species.add_template_species(&format!("Zz{} zzspecies", "a".repeat(n + 1)), "", &genome, &brain))
            .collect();
        let (base, scaling) = (
            app.world.settings.independents.species_genetic_span,
            app.world.settings.independents.species_span_scaling_factor,
        );
        // 22 of the 25 species have a member; the log (five of them: the panel wants that many records) drops the three that have none.
        app.world.species.reconcile_counts_by_species(species.iter().take(22).map(|id| (*id, 1_u32)));
        for _ in 0..5 {
            app.world.species.log_stats_with(&[]);
        }
        assert_eq!(app.world.species.active_species().len(), 22);
        assert!((app.lineage_effective_span() - (base + 5.0 * scaling)).abs() < 1e-6, "the span still counts the 25 it was last computed with");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenLineage)]);
        assert!((app.lineage_effective_span() - (base + 2.0 * scaling)).abs() < 1e-6, "the panel's opening recounts the active species: 22");
    }

    #[test]
    fn the_stats_buttons_do_nothing_in_a_challenge() {
        let mut app = App::new(synthetic(48, 6), Vec2::new(1280.0, 800.0), None);
        app.start_local_challenge();
        assert!(app.select_first());
        let id = app.selected.expect("selected");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::KillSelected)]);
        assert!(!app.world.bibites.get(id).expect("bibite").body.dead);
        let eggs = app.world.eggs.ids().count();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::LayEggs)]);
        assert_eq!(app.world.eggs.ids().count(), eggs);
    }

    #[test]
    fn the_children_button_selects_a_child_and_does_nothing_without_one() {
        let mut app = App::new(synthetic(49, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let parent = app.selected.expect("parent");
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SelectRandomChild,
        )]);
        assert_eq!(app.selected, Some(parent), "no child yet");
        assert_eq!(app.world.lay_egg_once(parent), 1);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SelectRandomChild,
        )]);
        assert!(app.selected_egg.is_some() && app.selected.is_none());
    }

    fn ctrl(copy: bool, paste: bool) -> FrameInput {
        FrameInput {
            copy_tag: copy,
            paste_tag: paste,
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    #[test]
    fn ctrl_c_and_ctrl_v_copy_and_paste_the_species_tag() {
        let mut app = App::new(synthetic(70, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let id = app.selected.expect("selected");
        // Nothing to copy without a target; with one, its tag goes to the clipboard.
        app.world.set_bibite_species_tag(id, Some("abc"));
        app.handle_input(&ctrl(true, false));
        assert_eq!(app.clipboard_out.take().as_deref(), Some("abc"));
        // Paste tags the target; the text is cut at 100 characters.
        app.clipboard_in = Some("x".repeat(130));
        app.handle_input(&ctrl(false, true));
        let tag = app.world.bibites.get(id).unwrap().meta.species_tag.clone();
        assert_eq!(tag.as_deref(), Some("x".repeat(100).as_str()));
        // An empty clipboard clears the tag (`AddToTag` with an empty string).
        app.clipboard_in = Some(String::new());
        app.handle_input(&ctrl(false, true));
        assert_eq!(app.world.bibites.get(id).unwrap().meta.species_tag, None);
        // Without a target Ctrl + C copies nothing.
        app.clear_target();
        app.handle_input(&ctrl(true, false));
        assert!(app.clipboard_out.is_none());
    }

    #[test]
    fn ctrl_v_tags_the_whole_multi_selection() {
        let mut app = App::new(synthetic(71, 8), Vec2::new(1280.0, 800.0), None);
        let whole = Vec2::new(1280.0, 800.0);
        app.handle_input(&press(Vec2::new(1.0, 1.0), true, true, false, 0.016));
        app.handle_input(&press(whole, false, true, false, 0.3));
        app.handle_input(&press(whole, false, false, true, 0.016));
        let picked = app
            .multi
            .as_ref()
            .expect("a multi-selection")
            .bibites
            .clone();
        assert!(picked.len() > 1);
        app.clipboard_in = Some("herd".to_owned());
        app.handle_input(&ctrl(false, true));
        for id in picked {
            let record = app.world.bibites.get(id).unwrap();
            assert_eq!(record.meta.species_tag.as_deref(), Some("herd"));
        }
    }

    #[test]
    fn the_tag_field_takes_typing_confirms_with_enter_and_blocks_hotkeys() {
        let mut app = App::new(synthetic(72, 6), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let id = app.selected.expect("selected");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditSpeciesTag)]);
        assert_eq!(app.tag_edit.as_deref(), Some(""));
        // Typing; hotkeys (here R, which would pick another bibite) are blocked meanwhile.
        app.typed = "wolf".to_owned();
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(app.selected, Some(id));
        assert_eq!(app.tag_edit.as_deref(), Some("wolf"));
        app.handle_input(&FrameInput {
            backspace: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(app.tag_edit.as_deref(), Some("wol"));
        // Ctrl + V pastes into the field instead of tagging.
        app.clipboard_in = Some("f!".to_owned());
        app.handle_input(&ctrl(false, true));
        assert_eq!(app.tag_edit.as_deref(), Some("wolf!"));
        assert_eq!(app.world.bibites.get(id).unwrap().meta.species_tag, None);
        app.handle_input(&FrameInput {
            enter: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.tag_edit.is_none());
        assert_eq!(
            app.world
                .bibites
                .get(id)
                .unwrap()
                .meta
                .species_tag
                .as_deref(),
            Some("wolf!")
        );
        // Escape leaves the field without touching the tag, and does not deselect.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditSpeciesTag)]);
        app.typed = "zzz".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        app.handle_input(&escape());
        assert!(app.tag_edit.is_none() && app.selected.is_some());
        assert_eq!(
            app.world
                .bibites
                .get(id)
                .unwrap()
                .meta
                .species_tag
                .as_deref(),
            Some("wolf!")
        );
        // Typed text outside the field is dropped.
        app.typed = "ignored".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.typed.is_empty());
    }

    #[test]
    fn the_play_button_returns_to_normal_speed_and_unpauses() {
        let mut app = App::new(synthetic(73, 4), Vec2::new(1280.0, 800.0), None);
        app.speed = 7.5;
        app.paused = true;
        app.apply_ui_actions(&[UiAction::SetSpeedSlider(1.0)]);
        assert_eq!(app.speed, 1.0);
        assert!(!app.paused);
        // The pause button only toggles the pause and keeps the speed.
        app.speed = 3.0;
        app.apply_ui_actions(&[UiAction::TogglePause]);
        assert!(app.paused && app.speed == 3.0);
    }

    #[test]
    fn the_brain_help_button_asks_for_a_web_page_and_the_species_button_hits_the_data_gate() {
        let mut app = App::new(synthetic(80, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenBrainHelp)]);
        assert!(
            app.pending_url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://"))
        );
        // A fresh world has too few species-log records: the same gate popup as the lineage button.
        assert!(app.select_first());
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::OpenLineageOfSelected,
        )]);
        assert!(app.lineage_gate_popup);
        assert!(!app.panels.is_open(crate::ui::PanelKind::Lineage));
    }

    #[test]
    fn clicking_the_world_confirms_the_tag_field_for_the_old_target() {
        let mut app = App::new(synthetic(81, 8), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let first = app.selected.expect("selected");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditSpeciesTag)]);
        app.typed = "pack".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        // A click on another bibite first ends the edit (for the bibite being edited), then selects.
        let other = app
            .world
            .live_bibite_ids()
            .into_iter()
            .find(|id| *id != first)
            .expect("another bibite");
        let at = app
            .camera
            .world_to_screen(app.world.bibites.get(other).unwrap().pos);
        app.handle_input(&press(at, true, true, false, 0.016));
        app.handle_input(&press(at, false, false, true, 0.016));
        assert!(app.tag_edit.is_none());
        assert_eq!(
            app.world
                .bibites
                .get(first)
                .unwrap()
                .meta
                .species_tag
                .as_deref(),
            Some("pack")
        );
        assert_eq!(app.world.bibites.get(other).unwrap().meta.species_tag, None);
        assert_eq!(app.selected, Some(other));
    }

    fn parasite_template() -> Option<PathBuf> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata/real/templates/10_Parasite_0.6.bb8template");
        path.is_file().then_some(path)
    }

    /// A fresh storage folder for the tests that write files.
    fn temp_storage(name: &str) -> (PathBuf, StoragePaths) {
        let dir = std::env::temp_dir().join(format!("rustibites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let storage = StoragePaths::new(dir.clone());
        (dir, storage)
    }

    #[test]
    fn the_save_button_opens_a_dialog_named_after_the_species_that_pauses_the_game() {
        let mut app = App::new(synthetic(96, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        assert!(app.save_dialog.is_none(), "nothing is selected");
        assert!(app.select_first());
        let species = app
            .selected
            .and_then(|id| app.world.bibites.get(id))
            .and_then(|record| record.meta.species)
            .and_then(|species| app.world.species.get(species))
            .map(|record| record.name())
            .unwrap_or_else(|| crate::save_dialog::DEFAULT_NAME.to_owned());
        assert!(!app.paused);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        let dialog = app.save_dialog.as_ref().expect("dialog");
        assert_eq!(dialog.name, species);
        assert!(app.paused, "LockControls pauses the game");
        // Every control is blocked: the selection hotkeys do nothing, typing goes to the field.
        app.typed = "!".to_owned();
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(
            app.save_dialog
                .as_ref()
                .is_some_and(|d| d.name.ends_with('!'))
        );
        // Escape closes it and the game goes back to what it was.
        app.handle_input(&escape());
        assert!(app.save_dialog.is_none() && !app.paused);
    }

    #[test]
    fn saving_writes_the_named_file_with_its_description_and_the_choices_persist() {
        let (dir, storage) = temp_storage("savedlg");
        let mut app = App::new(synthetic(97, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = storage;
        app.user_settings_path = Some(dir.join("settings.json"));
        assert!(app.select_first());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        {
            let dialog = app.save_dialog.as_mut().expect("dialog");
            dialog.name = "Wolf".to_owned();
            dialog.description = "a hunter".to_owned();
        }
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SaveDialogConfirm,
        )]);
        // A plain bibite: `Bibites/Wolf.bb8` with the game version and the description.
        let plain = app.storage.bibites_dir().join("Wolf.bb8");
        let text = std::fs::read_to_string(&plain).expect("the .bb8");
        let json: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(json["desc"], "a hunter");
        assert_eq!(json["version"], crate::app::GAME_VERSION);
        assert!(!text.contains('\n'), "compact unless Format JSON is on");
        assert!(app.save_dialog.is_none() && !app.paused);

        // "Save as template" and "Format JSON": a `.bb8template` in the templates folder, indented;
        // both choices persist (in memory and in the user settings file).
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        app.apply_ui_actions(&[
            UiAction::SaveDialogToggle(0),
            UiAction::SaveDialogToggle(1),
            UiAction::SaveDialogFocus(0),
        ]);
        assert!(app.save_as_template && app.format_bibite_json);
        let settings = crate::user_settings::UserSettings::load(Some(dir.join("settings.json")));
        assert!(settings.flag("Save as template") && settings.flag("Format JSON"));
        {
            let dialog = app.save_dialog.as_mut().expect("dialog");
            dialog.name = "Wolf".to_owned();
            dialog.description = "a hunter".to_owned();
        }
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SaveDialogConfirm,
        )]);
        let template = app.storage.templates_dir().join("Wolf.bb8template");
        let text = std::fs::read_to_string(&template).expect("the template");
        assert!(text.contains('\n'), "indented with Format JSON");
        let loaded = bibites_io::load_template(&template).expect("a template the game can load");
        assert_eq!(loaded.genome.values().len(), bibites_core::genome::N_GENES);
        // The next dialog starts from the stored choices.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        let dialog = app.save_dialog.as_ref().expect("dialog");
        assert!(dialog.as_template && dialog.format_json);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SaveDialogCancel,
        )]);
        assert!(app.save_dialog.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_invalid_name_cannot_be_saved_and_an_existing_one_is_overwritten() {
        let (dir, storage) = temp_storage("savedlg-names");
        let mut app = App::new(synthetic(98, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = storage;
        assert!(app.select_first());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        app.save_dialog.as_mut().expect("dialog").name = "bad/name".to_owned();
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SaveDialogConfirm,
        )]);
        assert!(app.save_dialog.is_some(), "Save stays disabled");
        {
            let dialog = app.save_dialog.as_mut().expect("dialog");
            dialog.name = "Twice".to_owned();
            dialog.description = "first".to_owned();
        }
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SaveDialogConfirm,
        )]);
        // The same name again overwrites (the dialog only warns).
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveSelected)]);
        {
            let dialog = app.save_dialog.as_mut().expect("dialog");
            dialog.name = "Twice".to_owned();
            dialog.description = "second".to_owned();
            assert!(
                dialog.would_overwrite(&app.storage.bibites_dir(), &app.storage.templates_dir())
            );
        }
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::SaveDialogConfirm,
        )]);
        let text =
            std::fs::read_to_string(app.storage.bibites_dir().join("Twice.bb8")).expect("file");
        assert!(text.contains("second") && !text.contains("first"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_pellet_placer_panel_sets_the_games_settings_and_turns_selection_off() {
        use crate::placer_panel::{PELLET_COUNT, PELLET_SIZE};
        let mut app = App::new(synthetic(95, 4), Vec2::new(1280.0, 800.0), None);
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        assert!(app.select_first());
        // `PelletPlacer.OpenPanel`: the target is dropped; the placer is the open panel.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SelectTool(
            crate::ui::ToolKind::Pellet,
        ))]);
        assert!(app.selected.is_none());
        assert_eq!(app.tool, crate::ui::ToolKind::Pellet);
        // `AllowSelection` is off: the random selection hotkey does nothing.
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected.is_none());
        // "Pellet Number": 1..=10 in whole numbers along the track.
        let track = (100.0, 300.0);
        app.pellet_slide(PELLET_COUNT, track, 100.0 * scale);
        assert_eq!(app.tool_settings.pellet_count, 1);
        app.pellet_slide(PELLET_COUNT, track, 300.0 * scale);
        assert_eq!(app.tool_settings.pellet_count, 10);
        app.pellet_slide(PELLET_COUNT, track, 200.0 * scale);
        assert_eq!(app.tool_settings.pellet_count, 6);
        // "Pellet Size Factor": logarithmic from 0.1 to 1000; a quarter of the track is 1.
        app.pellet_slide(PELLET_SIZE, track, 100.0 * scale);
        assert!((app.tool_settings.pellet_energy - 0.1).abs() < 1e-4);
        app.pellet_slide(PELLET_SIZE, track, 150.0 * scale);
        assert!((app.tool_settings.pellet_energy - 1.0).abs() < 1e-3);
        app.pellet_slide(PELLET_SIZE, track, 300.0 * scale);
        assert!((app.tool_settings.pellet_energy - 1000.0).abs() < 0.1);
        // Escape and the close button both close the panel.
        app.handle_input(&escape());
        assert_eq!(app.tool, crate::ui::ToolKind::Inspect);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SelectTool(
            crate::ui::ToolKind::Pellet,
        ))]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ClosePellets)]);
        assert_eq!(app.tool, crate::ui::ToolKind::Inspect);
        // With the panel closed, selection works again.
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected.is_some());
    }

    #[test]
    fn load_bibite_pauses_for_the_selector_and_the_choice_starts_placement() {
        let Some(template) = parasite_template() else {
            eprintln!("skipping: the committed test templates are absent");
            return;
        };
        let mut app = App::new(synthetic(91, 4), Vec2::new(1280.0, 800.0), None);
        assert!(!app.paused);
        app.placer_options.tag = "leftover".to_owned();
        // `ButtonFromTemplatePressed`: the custom tag resets, the selector opens, the game pauses.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::LoadBibite)]);
        assert_eq!(
            app.template_pick,
            Some(crate::app::TemplateUse::PlaceBibite)
        );
        assert!(app.paused);
        assert!(app.panels.is_open(crate::ui::PanelKind::Library));
        assert!(app.placer_options.tag.is_empty());
        // Closing the selector without a choice gives the game back its previous state.
        app.cancel_template_pick();
        assert!(!app.paused && app.template_pick.is_none() && app.placer.is_none());
        // A game that was already paused stays paused.
        app.paused = true;
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::LoadBibite)]);
        app.cancel_template_pick();
        assert!(app.paused);
        app.paused = false;
        // Choosing a template ends the pause and enters placement mode (`PlaceBibitesFromTemplate`):
        // the target is dropped.
        assert!(app.select_first());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::LoadBibite)]);
        app.start_bibite_placer(&template);
        assert!(!app.paused && app.template_pick.is_none());
        assert!(app.placer.is_some());
        assert!(app.selected.is_none(), "the target is dropped");
    }

    #[test]
    fn a_press_places_a_bibite_with_the_panels_choices_and_nothing_is_selected() {
        use bibites_core::settings::{GrowthAtSpawn, RandomizeGenes, Tagging};
        let Some(template) = parasite_template() else {
            eprintln!("skipping: the committed test templates are absent");
            return;
        };
        let mut app = App::new(synthetic(92, 4), Vec2::new(1280.0, 800.0), None);
        app.start_bibite_placer(&template);
        assert!(app.placer.is_some());
        app.placer_options.growth_at_spawn = GrowthAtSpawn::Adult;
        app.placer_options.randomize_genes = RandomizeGenes::No;
        app.placer_options.tagging = Tagging::CustomTagging;
        app.placer_options.tag = "wolf".to_owned();
        let at = Vec2::new(300.0, -200.0);
        let before = app.world.bibites.len();
        click_world(&mut app, at);
        assert_eq!(app.world.bibites.len(), before + 1);
        let placed = app
            .world
            .bibites
            .iter()
            .map(|(_, record)| record)
            .find(|record| record.pos.distance(at) < 1.0)
            .expect("the placed bibite");
        assert_eq!(placed.meta.species_tag.as_deref(), Some("wolf"));
        // It goes on: every press places another one.
        click_world(&mut app, Vec2::new(-300.0, 100.0));
        assert_eq!(app.world.bibites.len(), before + 2);
        // Egg growth places an egg.
        app.placer_options.growth_at_spawn = GrowthAtSpawn::Egg;
        let eggs = app.world.eggs.len();
        click_world(&mut app, Vec2::new(100.0, 300.0));
        assert_eq!(app.world.eggs.len(), eggs + 1);
        // Nothing gets selected while placing, neither by a click on a bibite nor by a hotkey.
        let living = app.world.live_bibite_ids()[0];
        let pos = app.world.bibites.get(living).unwrap().pos;
        click_world(&mut app, pos);
        assert!(app.selected.is_none());
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected.is_none());
    }

    #[test]
    fn escape_ends_placement_and_selection_works_again() {
        let Some(template) = parasite_template() else {
            eprintln!("skipping: the committed test templates are absent");
            return;
        };
        let mut app = App::new(synthetic(93, 4), Vec2::new(1280.0, 800.0), None);
        app.start_bibite_placer(&template);
        app.handle_input(&escape());
        assert!(app.placer.is_none());
        assert!(!app.panels.is_open(crate::ui::PanelKind::Escape));
        let id = app.world.live_bibite_ids()[0];
        let pos = app.world.bibites.get(id).unwrap().pos;
        click_world(&mut app, pos);
        assert_eq!(app.selected, Some(id));
        // The close button does the same.
        app.start_bibite_placer(&template);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ClosePlacer)]);
        assert!(app.placer.is_none());
    }

    #[test]
    fn the_placers_dropdowns_cycle_and_its_custom_tag_can_be_typed() {
        use crate::bibite_editor::TAGGING;
        use bibites_core::settings::Tagging;
        let Some(template) = parasite_template() else {
            eprintln!("skipping: the committed test templates are absent");
            return;
        };
        let mut app = App::new(synthetic(94, 2), Vec2::new(1280.0, 800.0), None);
        app.start_bibite_placer(&template);
        app.placer_options.tagging = Tagging::NoTagging;
        // The tag field only exists with custom tagging.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditPlacerTag)]);
        assert!(app.tag_edit.is_none());
        for _ in 0..8 {
            if app.placer_options.tagging == Tagging::CustomTagging {
                break;
            }
            app.apply_ui_actions(&[UiAction::PlacerCycle(TAGGING)]);
        }
        assert_eq!(app.placer_options.tagging, Tagging::CustomTagging);
        // Typing: the tag field takes characters, Enter confirms it as the placer's tag.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditPlacerTag)]);
        assert!(app.tag_edit.is_some() && app.tag_edit_is_placer());
        app.typed = "pack".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_eq!(app.tag_edit.as_deref(), Some("pack"));
        app.handle_input(&FrameInput {
            enter: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.tag_edit.is_none());
        assert_eq!(app.placer_options.tag, "pack");
        assert!(
            app.selected.is_none(),
            "the species tag of nothing was touched"
        );
        // Escape while typing leaves the field and keeps the placer.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditPlacerTag)]);
        app.typed = "xx".to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
        app.handle_input(&escape());
        assert!(app.tag_edit.is_none() && app.placer.is_some());
        assert_eq!(app.placer_options.tag, "pack");
    }

    #[test]
    fn the_worlds_default_bibites_can_be_added_edited_and_deleted() {
        use bibites_core::settings::{GrowthAtSpawn, SpawnType, Tagging};
        let Some(template) = parasite_template() else {
            eprintln!("skipping: the committed test templates are absent");
            return;
        };
        let mut app = App::new(synthetic(82, 4), Vec2::new(1280.0, 800.0), None);
        let before = app.world.bibite_spawns.len();
        // The `+` opens the selector in pick mode (the window loop does the rest).
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::AddDefaultBibite,
        )]);
        assert_eq!(
            app.template_pick,
            Some(crate::app::TemplateUse::DefaultBibite)
        );
        assert!(app.panels.is_open(crate::ui::PanelKind::Library));

        // "Select Bibite" hands the file back: a row with the game's defaults, its template
        // known to the world, the spawner running, and the new item open.
        app.add_default_bibite(&template);
        assert_eq!(app.world.bibite_spawns.len(), before + 1);
        let row = app.world.bibite_spawns.last().unwrap().clone();
        assert_eq!(row.path, "10_Parasite_0.6.bb8template");
        assert_eq!((row.spawn_priority, row.minimum), (1.0, 10));
        assert!(
            app.world
                .templates
                .iter()
                .any(|template| template.name.contains("10_Parasite_0.6"))
        );
        assert!(app.world.fresh_scenario_spawner_started());
        let index = (before) as u16;
        assert_eq!(app.dyn_panel.open_items, vec![(2, index)]);
        assert_eq!(app.default_bibite_names().len(), before + 1);

        // The item's controls edit the live row.
        use crate::bibite_editor::{GROWTH, MINIMUM, SPAWN_TYPE, TAGGING};
        app.apply_ui_actions(&[
            UiAction::BibiteCycle(index, TAGGING),
            UiAction::BibiteCycle(index, GROWTH),
            UiAction::BibiteCycle(index, SPAWN_TYPE),
        ]);
        let row = &app.world.bibite_spawns[usize::from(index)];
        assert_eq!(row.tagging, Tagging::SpeciesTagging);
        assert_eq!(row.growth_at_spawn, GrowthAtSpawn::Elder);
        assert_eq!(row.spawn_type, SpawnType::OneTime);
        // Dragging the target-count slider to the left end gives zero; reset gives ten.
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        app.bibite_slide(usize::from(index), MINIMUM, (100.0, 400.0), 100.0 * scale);
        assert_eq!(app.world.bibite_spawns[usize::from(index)].minimum, 0);
        app.apply_ui_actions(&[UiAction::BibiteReset(index, MINIMUM)]);
        assert_eq!(app.world.bibite_spawns[usize::from(index)].minimum, 10);

        // Clicking the item selects it, and clicking the selected one again changes nothing (`BibiteSettingsHandle.OnPointerClick`: one item at a time).
        app.apply_ui_actions(&[UiAction::ZoneOpen(2, index)]);
        assert_eq!(app.dyn_panel.open_items, vec![(2, index)]);
        app.apply_ui_actions(&[UiAction::ZoneOpen(2, index)]);
        assert_eq!(app.dyn_panel.open_items, vec![(2, index)]);

        // Delete goes at once; "remove all" asks first.
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::DeleteDefaultBibite(index),
        )]);
        assert_eq!(app.world.bibite_spawns.len(), before);
        app.add_default_bibite(&template);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::RequestClearDefaultBibites,
        )]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::ClearDefaultBibites));
        assert!(!app.world.bibite_spawns.is_empty());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        assert!(app.world.bibite_spawns.is_empty());
    }

    #[test]
    fn tool_settings_drive_pellet_pile_and_killer_tower() {
        let mut app = App::new(synthetic(42, 6), Vec2::new(1280.0, 800.0), None);
        app.tool = crate::ui::ToolKind::Pellet;
        app.tool_settings.pellet_count = 4;
        app.tool_settings.pellet_energy = 10.0;
        let before = app.world.pellets.len();
        app.apply_tool(Vec2::new(20.0, 20.0));
        assert_eq!(app.world.pellets.len(), before + 4);

        app.tool = crate::ui::ToolKind::Killer;
        let tools_before = app.world.tools.len();
        app.apply_tool(Vec2::new(-20.0, 10.0));
        assert_eq!(app.world.tools.len(), tools_before + 1);
    }

    #[test]
    fn tag_and_egg_save_commands_work() {
        let dir = std::env::temp_dir().join(format!("rustibites-tag-egg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(43, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        assert!(app.select_first());
        app.tag_draft = "viewer-tag".to_owned();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SetTag)]);
        assert!(app.last_action.contains("viewer-tag"));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SaveEggs)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The index of the zone called `name`.
    fn zone_named(app: &App, name: &str) -> usize {
        app.world
            .zones
            .zones
            .iter()
            .position(|zone| zone.config.0.name == name)
            .expect("the zone")
    }

    #[test]
    fn a_world_dropdown_opens_its_list_and_a_pick_applies_the_option() {
        use crate::bibite_editor::{GROWTH, SPAWN_ZONE, TAGGING};
        use crate::ui::DropdownTarget as Target;
        use crate::zone_editor::{DISTRIBUTION, MATERIAL, MOVEMENT, TARGET};
        use bibites_core::settings::{GrowthAtSpawn, MovementType, SpawnDistribution, Tagging};
        let mut app = App::new(synthetic(110, 0), Vec2::new(1280.0, 800.0), None);
        two_zones(&mut app);
        let alpha = zone_named(&app, "Alpha") as u16;
        let beta = zone_named(&app, "Beta") as u16;
        let anchor = [100.0, 200.0, 180.0, 30.0];
        let labels = |app: &App| -> Vec<String> {
            app.dropdown_open
                .as_ref()
                .expect("a list is open")
                .choices
                .iter()
                .map(|choice| choice.label.clone())
                .collect()
        };
        let open = |app: &mut App, target: Target| {
            app.click_rect = anchor;
            app.apply_ui_actions(&[UiAction::DropdownOpen(target)]);
        };
        let pick = |app: &mut App, index: u16| {
            app.apply_ui_actions(&[UiAction::DropdownPick(index)]);
            assert!(app.dropdown_open.is_none(), "a pick closes the list");
        };

        // A zone item: the list hangs under the control, shows every option and a pick applies it.
        open(&mut app, Target::Zone(alpha, DISTRIBUTION));
        assert_eq!(labels(&app).len(), 6);
        assert_eq!(labels(&app)[0], "Flat Circle");
        assert_eq!(
            app.dropdown_open.as_ref().map(|open| open.anchor),
            Some(anchor)
        );
        pick(&mut app, 3);
        assert_eq!(
            app.world.zones.zones[usize::from(alpha)]
                .config
                .0
                .distribution,
            SpawnDistribution::Ring
        );
        // Movement, then the target: "None" and every zone, the zone's own included.
        open(&mut app, Target::Zone(alpha, MOVEMENT));
        assert_eq!(labels(&app), ["None", "Free", "Locked to Target"]);
        pick(&mut app, 2);
        open(&mut app, Target::Zone(alpha, TARGET));
        let names = labels(&app);
        assert_eq!(names[0], "None");
        assert_eq!(names.len(), app.world.zones.zones.len() + 1);
        assert!(
            names.contains(&"Alpha".to_owned()),
            "the zone's own is listed"
        );
        pick(&mut app, usize::from(beta) as u16 + 1);
        let config = &app.world.zones.zones[usize::from(alpha)].config.0;
        assert_eq!(config.movement, MovementType::Attached);
        assert_eq!(
            config.target,
            app.world.zones.zones[usize::from(beta)].config.0.zone_id
        );
        // The spawn material: None, Plant, Meat.
        open(&mut app, Target::Zone(alpha, MATERIAL));
        assert_eq!(labels(&app), ["None", "Plant", "Meat"]);
        pick(&mut app, 2);
        assert_eq!(
            app.world.zones.zones[usize::from(alpha)].config.0.material,
            Some(bibites_core::settings::MaterialKind::Meat)
        );

        // Clicking the dropdown of the open list closes it, and another control's click moves it.
        open(&mut app, Target::Zone(alpha, MATERIAL));
        open(&mut app, Target::Zone(alpha, MATERIAL));
        assert!(app.dropdown_open.is_none());
        open(&mut app, Target::Zone(alpha, MATERIAL));
        open(&mut app, Target::Zone(alpha, MOVEMENT));
        assert_eq!(labels(&app), ["None", "Free", "Locked to Target"]);
        app.apply_ui_actions(&[UiAction::DropdownClose]);
        assert!(app.dropdown_open.is_none());

        // A default bibite item: tagging and the spawn zone (All, then every zone).
        app.world
            .set_bibite_spawns(vec![bibites_core::settings::BibiteSpawn::game_defaults()])
            .expect("a default bibite");
        open(&mut app, Target::Bibite(0, TAGGING));
        assert_eq!(labels(&app), ["None", "Species", "Custom", "Random Tag"]);
        pick(&mut app, 2);
        assert_eq!(app.world.bibite_spawns[0].tagging, Tagging::CustomTagging);
        open(&mut app, Target::Bibite(0, SPAWN_ZONE));
        assert_eq!(labels(&app).len(), app.world.zones.zones.len() + 1);
        pick(&mut app, usize::from(beta) as u16 + 1);
        assert_eq!(
            app.world.bibite_spawns[0].spawn_zone,
            app.world.zones.zones[usize::from(beta)].config.0.zone_id
        );

        // The bibite placer's dropdowns.
        open(&mut app, Target::Placer(GROWTH));
        assert_eq!(labels(&app), ["Egg", "Baby", "Adult", "Elder"]);
        pick(&mut app, 3);
        assert_eq!(app.placer_options.growth_at_spawn, GrowthAtSpawn::Elder);

        // The pellet placer's material dropdown: the two physical materials.
        open(&mut app, Target::PelletMaterial);
        assert_eq!(labels(&app), ["Plant", "Meat"]);
        assert_eq!(
            app.dropdown_open.as_ref().map(|open| open.selected),
            Some(0)
        );
        pick(&mut app, 1);
        assert_eq!(
            app.tool_settings.material,
            bibites_core::pellet::MaterialKind::Meat
        );
    }

    #[test]
    fn a_towers_follow_zone_dropdown_lists_none_and_the_zones() {
        use crate::ui::DropdownTarget as Target;
        let mut app = App::new(synthetic(111, 0), Vec2::new(1280.0, 800.0), None);
        two_zones(&mut app);
        let beta = zone_named(&app, "Beta");
        // Without a selected tower there is nothing to list.
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::ToolZone)]);
        assert!(app.dropdown_open.is_none());
        let tower = app.world.place_rad_tower_at(Vec2::new(10.0, 10.0), None);
        app.select_placed_tool(tower);
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::ToolZone)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        assert_eq!(open.choices[0].label, "None");
        assert_eq!(open.choices.len(), app.world.zones.zones.len() + 1);
        assert_eq!(open.selected, 0, "the tower follows nothing yet");
        app.apply_ui_actions(&[UiAction::DropdownPick(beta as u16 + 1)]);
        let Some(bibites_core::tools::Tool::Rad(rad)) = app.world.tools.get(tower) else {
            panic!("a radiation tower");
        };
        assert_eq!(
            rad.base.attached_zone,
            Some(app.world.zones.zones[beta].id),
            "the tower now follows Beta"
        );
        // And "None" lets it go again.
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::ToolZone)]);
        assert_eq!(
            app.dropdown_open.as_ref().map(|open| open.selected),
            Some(beta + 1)
        );
        app.apply_ui_actions(&[UiAction::DropdownPick(0)]);
        let Some(bibites_core::tools::Tool::Rad(rad)) = app.world.tools.get(tower) else {
            panic!("a radiation tower");
        };
        assert_eq!(rad.base.attached_zone, None);
    }

    #[test]
    fn escape_closes_an_open_dropdown_list_first_and_the_list_blocks_the_hotkeys() {
        use crate::ui::DropdownTarget as Target;
        let mut app = App::new(synthetic(112, 2), Vec2::new(1280.0, 800.0), None);
        two_zones(&mut app);
        app.click_rect = [10.0, 10.0, 100.0, 30.0];
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::PelletMaterial)]);
        assert!(app.dropdown_open.is_some());
        // A hotkey does nothing while the list is up (`R` selects a random bibite).
        app.handle_input(&FrameInput {
            dt: 0.016,
            select_random_bibite: true,
            ..FrameInput::default()
        });
        assert!(app.selected.is_none(), "the list blocks the hotkeys");
        // Escape closes the list and nothing else: no pause menu.
        app.handle_input(&escape());
        assert!(app.dropdown_open.is_none());
        assert!(!app.panels.is_open(crate::ui::PanelKind::Escape));
        // With the list gone the hotkey works again.
        app.handle_input(&FrameInput {
            dt: 0.016,
            select_random_bibite: true,
            ..FrameInput::default()
        });
        assert!(app.selected.is_some());
    }

    #[test]
    fn the_graphs_data_dropdown_lists_the_seven_groups_and_a_pick_plots_the_group() {
        use crate::ui::DropdownTarget as Target;
        let mut app = App::new(synthetic(120, 4), Vec2::new(1920.0, 1080.0), None);
        app.click_rect = [316.0, 87.0, 397.0, 37.0];
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsData)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        // `GraphsPanel.Start`: one option per stream of `DataLogger.parallelDataStreams`, each with
        // its description as the item's tooltip; the list closes when the pointer leaves it.
        let titles: Vec<&str> = open.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Biomass Data",
                "Entities Count Data",
                "Age Data",
                "Brain Size Data",
                "Age of Death Data",
                "Egg Laying Data",
                "Birth/Death Data"
            ]
        );
        assert!(
            open.choices
                .iter()
                .all(|c| !c.tip.is_empty() && c.title == c.label)
        );
        assert!(open.leave_closes && open.checked.is_none());
        assert_eq!(open.selected, 0);
        assert_eq!(open.anchor, [316.0, 87.0, 397.0, 37.0]);
        // A pick plots the group and closes the list.
        app.apply_ui_actions(&[UiAction::DropdownPick(4)]);
        assert!(app.dropdown_open.is_none());
        assert_eq!(app.graphs_stream, 4);
        // The list marks the group that is plotted.
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsData)]);
        assert_eq!(
            app.dropdown_open.as_ref().map(|open| open.selected),
            Some(4)
        );
        // Clicking the dropdown again leaves a custom dropdown's list as it is.
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsData)]);
        assert!(app.dropdown_open.is_some());
        // Esc closes it (`ICancelHandler`).
        app.handle_input(&escape());
        assert!(app.dropdown_open.is_none());
    }

    /// Pushes `hours` points of the genes stream whose gene `gene` is `n` bibites in one bucket (a different `n` each hour).
    fn log_gene_hours(app: &mut App, hours: usize) {
        for hour in 1..=hours {
            let genes = vec![bibites_core::genome::Genome::defaults().genes; 5 * hour];
            app.world
                .data_logger
                .genes
                .push(bibites_core::genes_log::GenesBuckets::from_genes(&genes));
        }
    }

    #[test]
    fn the_gene_analysis_page_opens_from_its_button_and_remembers_its_gene_and_time() {
        let mut app = App::new(synthetic(120, 4), Vec2::new(1920.0, 1080.0), None);
        // The panel starts on the data graphs, the dropdown on the 17th gene (Diet), the slider on "Now".
        assert!(!app.graphs_genes);
        assert_eq!(app.graphs_gene, 16);
        assert_eq!(
            crate::gene_analysis::gene_info(app.graphs_gene).name,
            "Diet"
        );
        assert_eq!(app.graphs_gene_time_value(), -1);
        // `OpenGeneAnalysis` / `OpenHistoricalGraph`.
        app.apply_ui_actions(&[UiAction::GraphsPage(true)]);
        assert!(app.graphs_genes);
        app.apply_ui_actions(&[UiAction::GraphsPage(false)]);
        assert!(!app.graphs_genes);
        // The legend of the data graphs does not show on the genes page, wherever the pointer rests.
        app.panels.open(crate::ui::PanelKind::Graphs);
        let on_legend_button = Vec2::new(1860.0, 160.0);
        app.handle_input(&FrameInput {
            cursor: on_legend_button,
            ..FrameInput::default()
        });
        assert!(app.graphs_legend_open, "the data page's \"?\" button");
        app.apply_ui_actions(&[UiAction::GraphsPage(true)]);
        app.handle_input(&FrameInput {
            cursor: on_legend_button,
            ..FrameInput::default()
        });
        assert!(!app.graphs_legend_open);
    }

    #[test]
    fn the_gene_dropdown_lists_the_35_genes_with_their_help_and_a_pick_shows_the_gene() {
        use crate::ui::DropdownTarget as Target;
        let mut app = App::new(synthetic(120, 4), Vec2::new(1920.0, 1080.0), None);
        app.click_rect = [365.0, 85.0, 250.0, 35.0];
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsGene)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        // `GraphsPanel.Start`: `geneSettingsOrdered`, the genes in the enum's order, each with its help text as the tooltip.
        assert_eq!(open.choices.len(), 35);
        assert_eq!(open.choices[0].label, "Lay Time");
        assert_eq!(open.choices[16].label, "Diet");
        assert!(
            open.choices
                .iter()
                .all(|c| !c.tip.is_empty() && c.title == c.label)
        );
        assert_eq!(open.selected, 16);
        assert_eq!(open.style, crate::dropdown::GENES);
        assert!(
            open.leave_closes,
            "a RealDropdown closes when the pointer leaves"
        );
        assert_eq!(open.anchor, [365.0, 85.0, 250.0, 35.0]);
        // A pick shows that gene and closes the list; the list marks it afterwards.
        app.apply_ui_actions(&[UiAction::DropdownPick(3)]);
        assert!(app.dropdown_open.is_none());
        assert_eq!(app.graphs_gene, 3);
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsGene)]);
        assert_eq!(
            app.dropdown_open.as_ref().map(|open| open.selected),
            Some(3)
        );
    }

    #[test]
    fn the_gene_time_slider_runs_from_now_through_the_logged_hours_right_to_left() {
        let mut app = App::new(synthetic(120, 4), Vec2::new(1920.0, 1080.0), None);
        // No hour logged: the slider only has "Now".
        assert_eq!(app.graphs_gene_range(), (-1, -1));
        app.graphs_gene_set_time(5);
        assert_eq!(app.graphs_gene_time_value(), -1);
        // `loggedGenes = nData` when the panel opens: three hours.
        log_gene_hours(&mut app, 3);
        assert_eq!(app.graphs_gene_range(), (-1, 2));
        // A press on the track jumps to the cursor (the newest, "Now", is at the right end; 1 reference px = 1 px here).
        app.graphs_gene_press(false, (250.0, 1670.0), 1670.0);
        assert_eq!(app.graphs_gene_time_value(), -1);
        assert!(app.graphs_gene_drag.is_some());
        // Held and moved left it goes back in time: the oldest point (2) at the left end, the others at equal steps.
        app.graphs_gene_slide(250.0);
        assert_eq!(app.graphs_gene_time_value(), 2);
        app.graphs_gene_slide(250.0 + 1420.0 / 3.0);
        assert_eq!(app.graphs_gene_time_value(), 1);
        app.graphs_gene_slide(250.0 + 1420.0 * 2.0 / 3.0);
        assert_eq!(app.graphs_gene_time_value(), 0);
        // The middle of -1..2 is 0.5: `Mathf.Round` rounds halves to the even number.
        app.graphs_gene_slide(250.0 + 1420.0 * 0.5);
        assert_eq!(app.graphs_gene_time_value(), 0);
        // Past either end the value stays inside the range.
        app.graphs_gene_slide(-500.0);
        assert_eq!(app.graphs_gene_time_value(), 2);
        app.graphs_gene_slide(5000.0);
        assert_eq!(app.graphs_gene_time_value(), -1);
        // On the handle the grab offset keeps the value under a cursor that is not in the middle of it.
        app.graphs_gene_set_time(2);
        app.graphs_gene_drag = None;
        app.graphs_gene_press(true, (250.0, 1670.0), 258.0);
        assert_eq!(
            app.graphs_gene_time_value(),
            2,
            "8 px from the handle's middle is no move"
        );
        app.graphs_gene_slide(258.0 + 1420.0 * (2.0 / 3.0));
        assert_eq!(app.graphs_gene_time_value(), 0);
        // The value is kept when the range shrinks (a world with fewer hours).
        app.world.data_logger.genes = bibites_core::genes_log::GenesLog::new();
        assert_eq!(app.graphs_gene_time_value(), -1);
    }

    #[test]
    fn the_graphs_curves_dropdown_toggles_curves_keeps_its_list_open_and_remembers_them() {
        use crate::ui::DropdownTarget as Target;
        let mut app = App::new(synthetic(121, 4), Vec2::new(1920.0, 1080.0), None);
        app.click_rect = [1421.0, 87.0, 397.0, 37.0];
        assert_eq!(app.graphs_curves_label(), "All");
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsCurves)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        let labels: Vec<&str> = open.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Bibites", "Plants", "Meats", "Free"]);
        assert!(open.choices.iter().all(|c| !c.tip.is_empty()));
        assert_eq!(
            open.checked,
            Some(vec![true; 4]),
            "every curve shows to begin with"
        );
        // An item switches its curve; the list stays open, with the item the selected one.
        app.apply_ui_actions(&[UiAction::DropdownPick(1)]);
        let open = app.dropdown_open.as_ref().expect("the list stays open");
        assert_eq!(open.checked, Some(vec![true, false, true, true]));
        assert_eq!(open.tint, Some(1));
        assert_eq!(app.graphs_curves_label(), "3 Selected");
        assert_eq!(app.graphs_curve_marks(), [true, false, true, true]);
        // Down to none (`TogglesDropdown.UpdateHandles`), then one back.
        for index in [0, 2, 3] {
            app.apply_ui_actions(&[UiAction::DropdownPick(index)]);
        }
        assert_eq!(app.graphs_curves_label(), "None Selected");
        app.apply_ui_actions(&[UiAction::DropdownPick(1)]);
        assert_eq!(app.graphs_curves_label(), "1 Selected");
        assert!(app.dropdown_open.is_some());
        // The choice belongs to the group (its `DataStreamDescription.state`): another group
        // starts with all its curves, and coming back finds the first as it was left.
        app.apply_ui_actions(&[UiAction::DropdownClose]);
        app.set_graphs_stream(1);
        assert_eq!(app.graphs_curves_label(), "All");
        assert_eq!(app.graphs_curve_marks().len(), 3);
        app.set_graphs_stream(0);
        assert_eq!(app.graphs_curve_marks(), [false, true, false, false]);
        // An index out of range toggles nothing.
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsCurves)]);
        app.toggle_graphs_curve(40);
        assert_eq!(app.graphs_curve_marks(), [false, true, false, false]);
    }

    #[test]
    fn a_custom_dropdowns_list_closes_when_the_pointer_leaves_it_and_a_standard_one_does_not() {
        use crate::ui::DropdownTarget as Target;
        let mut app = App::new(synthetic(122, 4), Vec2::new(1920.0, 1080.0), None);
        let screen = Vec2::new(1920.0, 1080.0);
        app.click_rect = [316.0, 87.0, 397.0, 37.0];
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::GraphsData)]);
        // On the dropdown, then on its list, then (a list is 245 high) in the gap beside it.
        app.close_dropdown_on_leave(Vec2::new(400.0, 100.0), screen, 1.0);
        assert!(app.dropdown_open.is_some());
        app.close_dropdown_on_leave(Vec2::new(400.0, 300.0), screen, 1.0);
        assert!(app.dropdown_open.is_some());
        app.close_dropdown_on_leave(Vec2::new(900.0, 300.0), screen, 1.0);
        assert!(app.dropdown_open.is_none(), "the pointer left the list");
        // A `TMP_Dropdown` list has a blocker instead: the pointer can wander.
        app.click_rect = [100.0, 200.0, 148.0, 30.0];
        app.apply_ui_actions(&[UiAction::DropdownOpen(Target::PelletMaterial)]);
        app.close_dropdown_on_leave(Vec2::new(900.0, 900.0), screen, 1.0);
        assert!(app.dropdown_open.is_some());
    }

    #[test]
    fn a_graphs_scale_without_data_cannot_be_picked_and_a_group_falls_back_to_the_deepest_one() {
        let mut app = App::new(synthetic(123, 4), Vec2::new(1920.0, 1080.0), None);
        // Only the finest scale has data in a new world.
        assert!(app.graphs_depth_has_data(0));
        assert!(!app.graphs_depth_has_data(1));
        assert_eq!(app.deepest_graphs_scale(), 0);
        app.apply_ui_actions(&[UiAction::SetGraphsScale(3)]);
        assert_eq!(
            app.graphs_scale, 0,
            "a scale without data is not interactable"
        );
        // Ten logs (the feed ratio) give the second scale its first point.
        let points: Vec<Vec<f32>> = app
            .world
            .data_logger
            .groups
            .iter()
            .map(|group| vec![1.0; group.streams.len()])
            .collect();
        for _ in 0..10 {
            app.world.data_logger.log(&points);
        }
        assert!(app.graphs_depth_has_data(1) && !app.graphs_depth_has_data(2));
        app.apply_ui_actions(&[UiAction::SetGraphsScale(1)]);
        assert_eq!(app.graphs_scale, 1);
        app.apply_ui_actions(&[UiAction::SetGraphsScale(2)]);
        assert_eq!(app.graphs_scale, 1);
        // `SetDataStream`: a group the scale has no data for falls back to the deepest it has.
        app.graphs_scale = 4;
        app.set_graphs_stream(2);
        assert_eq!(app.graphs_scale, 1);
        // A group index out of range is the last group.
        app.set_graphs_stream(99);
        assert_eq!(app.graphs_stream, 6);
    }

    #[test]
    fn tool_panel_adjustments_and_dropdown_picks_apply() {
        let mut app = App::new(synthetic(51, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::PelletMaterial]);
        assert_eq!(
            app.tool_settings.material,
            bibites_core::pellet::MaterialKind::Meat
        );
        app.apply_ui_actions(&[UiAction::PelletMaterial]);
        assert_eq!(
            app.tool_settings.material,
            bibites_core::pellet::MaterialKind::Plant
        );

        app.scene_options.grid = false;
        app.dropdown = Some(DropdownState {
            options: vec!["Off".to_owned(), "On".to_owned()],
            rect: [10.0, 10.0, 100.0, 24.0],
            session: "main_menu_settings".to_owned(),
            path: "Show Grid".to_owned(),
            label: "Show Grid".to_owned(),
        });
        app.apply_ui_actions(&[UiAction::PickDropdownChoice(1)]);
        assert!(app.scene_options.grid, "grid dropdown choice did not apply");
        assert!(app.dropdown.is_none());

        app.apply_ui_actions(&[UiAction::ToggleAutosaves]);
        assert!(app.include_autosaves);
    }

    #[test]
    fn lineage_panel_refreshes_and_selects_nodes() {
        use bibites_core::species_log::SpeciesDataPoint;
        let mut app = App::new(synthetic(61, 6), Vec2::new(1280.0, 800.0), None);
        app.world.species.set_n_records(5);
        // No species has two points in its log yet: there is nothing to draw, which is told like the gate.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenLineage)]);
        assert!(!app.panels.is_open(crate::ui::PanelKind::Lineage));
        assert!(app.lineage_gate_popup);
        app.lineage_gate_popup = false;
        let ids: Vec<_> = app.world.species.records().iter().map(|r| r.id).collect();
        if ids.is_empty() {
            // Generated starter worlds register species lazily; the gate path is still covered.
            return;
        }
        // Species that have been around for a while make a tree.
        for id in &ids {
            let log = &mut app.world.species.get_mut(*id).expect("species").stats.log;
            for _ in 0..4 {
                log.push(SpeciesDataPoint::from_counts(1, 5.0));
            }
        }
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenLineage)]);
        assert!(app.panels.is_open(crate::ui::PanelKind::Lineage));
        assert!(app.lineage_view.layout.built);
        // A press on a node of the tree selects its species.
        app.apply_ui_actions(&[UiAction::SelectLineageNode(ids[0].value())]);
        assert_eq!(app.species_info, Some(ids[0]));
        assert!(app.selected_lineage().is_some());
    }

    #[test]
    fn scenario_editor_apply_zone_and_dirty_flow() {
        let mut app = App::new(synthetic(71, 6), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::OpenScenarioEditor,
        )]);
        assert!(app.editor_settings.is_some());
        assert!(app.panels.is_open(crate::ui::PanelKind::ScenarioEditor));
        // Non-structural edit applies live.
        assert!(app.editor_set(
            "biomassDensity",
            bibites_core::settings::SettingValue::Float(0.0123)
        ));
        assert!(app.editor_dirty);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditorApply)]);
        assert!((app.world.settings.independents.biomass_density - 0.0123).abs() < 1e-5);
        // Zone add/apply/remove with stale-id safety.
        let before = app.world.zones.zones.len();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditorAddZone)]);
        assert_eq!(app.world.zones.zones.len(), before + 1);
        let index = app.world.zones.zones.len() - 1;
        assert!(app.editor_apply_zone(index, 2.0, 3.0));
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::EditorRemoveZone,
        )]);
        assert_eq!(app.world.zones.zones.len(), before);
        // Cancel closes/clears the editor.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditorCancel)]);
        assert!(app.editor_settings.is_none());
    }

    #[test]
    fn egg_tag_field_sets_and_clears() {
        let mut app = App::new(synthetic(81, 4), Vec2::new(1280.0, 800.0), None);
        assert!(app.select_first());
        let parent = app.selected.expect("parent");
        app.world.force_lay_eggs(parent);
        if app.world.eggs.ids().next().is_none() {
            // Starter bibites usually have no completed womb egg yet; let the
            // simulation produce one (bounded, deterministic) before opting out.
            for _ in 0..1800 {
                app.world.tick();
                if app.world.eggs.ids().next().is_some() {
                    break;
                }
            }
        }
        let Some(egg_id) = app.world.eggs.ids().next() else {
            return;
        };
        app.select_egg(egg_id);
        assert_eq!(app.selected_egg, Some(egg_id));
        app.tag_draft = "egg-tag".to_owned();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::SetTag)]);
        assert_eq!(
            app.world
                .eggs
                .get(egg_id)
                .expect("egg")
                .meta
                .species_tag
                .as_deref(),
            Some("egg-tag")
        );
        assert_eq!(app.egg_tag_draft, "egg-tag");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ClearTag)]);
        assert!(
            app.world
                .eggs
                .get(egg_id)
                .expect("egg")
                .meta
                .species_tag
                .is_none()
        );
    }

    #[test]
    fn gene_editor_edits_and_randomizes() {
        let mut app = App::new(synthetic(91, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenGeneEditor)]);
        assert!(app.editor_genome.is_some());
        assert!(app.panels.is_open(crate::ui::PanelKind::GeneEditor));
        assert!(!app.gene_editor_rows().is_empty());
        app.apply_ui_actions(&[UiAction::SelectGene(3), UiAction::AdjustGene(2)]);
        let before = app.gene_editor_rows()[3].1;
        app.apply_ui_actions(&[UiAction::AdjustGene(-4)]);
        let after = app.gene_editor_rows()[3].1;
        assert!(after < before, "gene adjust did not change the value");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditorRandomize)]);
        assert_eq!(app.last_action, "genome randomized");
        app.apply_ui_actions(&[UiAction::ClosePanel]);
        assert!(!app.panels.is_open(crate::ui::PanelKind::GeneEditor));
    }

    #[test]
    fn brain_editor_adds_nodes_synapses_and_validates() {
        let mut app = App::new(synthetic(92, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenBrainEditor)]);
        let brain = app.editor_brain.clone().expect("brain");
        let nodes_before = brain.graph_view().nodes.len();
        let synapses_before = brain.graph_view().synapses.len();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditorAddNode)]);
        let brain = app.editor_brain.clone().expect("brain");
        assert_eq!(brain.graph_view().nodes.len(), nodes_before + 1);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::EditorAddSynapse,
        )]);
        let brain = app.editor_brain.clone().expect("brain");
        assert_eq!(brain.graph_view().synapses.len(), synapses_before + 1);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::EditorValidate)]);
        assert!(!app.editor_validation.is_empty());
    }

    #[test]
    fn settings_pages_map_values_and_persist_preferences() {
        let dir = std::env::temp_dir().join(format!("rustibites-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(93, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::SetSettingsPage(1)]);
        assert_eq!(app.settings_page, 1);
        assert!(app.set_pref_by_key(
            "settingspanel show grid",
            crate::ui::ControlValue::Flag(false)
        ));
        assert!(!app.scene_options.grid);
        assert!(app.set_pref_by_key(
            "settingspanel ui scale",
            crate::ui::ControlValue::Float(1.5)
        ));
        assert!((app.prefs.ui_scale - 1.5).abs() < 1e-4);
        assert!(app.set_pref_by_key(
            "settingspanel autosave period",
            crate::ui::ControlValue::Float(5.0)
        ));
        assert_eq!(app.prefs.autosave_period_minutes, 5);
        app.apply_user_preferences();
        assert!(app.storage.preferences_path().is_file());
        let loaded =
            bibites_io::storage::load_user_preferences(&app.storage).expect("prefs reload");
        assert!((loaded.ui_scale - 1.5).abs() < 1e-4);
        assert!(!loaded.show_grid);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ResetSettings)]);
        assert!((app.prefs.ui_scale - 1.0).abs() < 1e-4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn destructive_save_dialogs_confirm_and_cancel() {
        let dir = std::env::temp_dir().join(format!("rustibites-dialog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(94, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.save_named("dialog-save");
        app.refresh_save_entries();
        assert!(!app.save_entries.is_empty());
        // Overwrite prompt instead of silently replacing.
        app.save_named("dialog-save");
        assert!(matches!(
            app.confirm_dialog,
            Some(ConfirmKind::OverwriteSave(_))
        ));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Cancel)]);
        assert!(app.confirm_dialog.is_none());
        // Delete prompts; cancel keeps the file, confirm removes it.
        app.refresh_save_entries();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::DeleteSave)]);
        assert!(matches!(
            app.confirm_dialog,
            Some(ConfirmKind::DeleteSave(_))
        ));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Cancel)]);
        assert!(!app.save_entries.is_empty(), "cancel deleted the save");
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::DeleteSave)]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        assert!(app.last_action.contains("deleted"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_delete_save_warning_keeps_do_not_show_again_only_when_confirmed() {
        use crate::ui::ControlCommand;
        use crate::warnings::ActionWarning;
        let dir = std::env::temp_dir().join(format!("rustibites-warning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(97, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        let settings_path = dir.join("settings.json");
        app.user_settings_path = Some(settings_path.clone());
        let run = |app: &mut App, command: ControlCommand| {
            app.apply_ui_actions(&[UiAction::Invoke(command)]);
        };
        // Whatever the installed game remembers, the warning starts as shown.
        run(&mut app, ControlCommand::ResetWarnings);
        assert!(!app.warning_off(ActionWarning::DeleteSaveFile));
        for name in ["first", "second", "third"] {
            app.save_named(name);
        }
        app.refresh_save_entries();
        let saves = app.save_entries.len();
        assert!(saves >= 3, "{saves} saves");

        // The popup opens unticked; ticking and then cancelling remembers nothing.
        run(&mut app, ControlCommand::DeleteSave);
        assert!(matches!(
            app.confirm_dialog,
            Some(ConfirmKind::DeleteSave(_))
        ));
        assert!(!app.confirm_do_not_show);
        run(&mut app, ControlCommand::ToggleDoNotShowAgain);
        assert!(app.confirm_do_not_show);
        run(&mut app, ControlCommand::Cancel);
        assert!(app.confirm_dialog.is_none() && !app.confirm_do_not_show);
        assert!(!app.warning_off(ActionWarning::DeleteSaveFile));
        assert_eq!(app.save_entries.len(), saves, "cancel deleted a save");

        // A new popup starts unticked again; confirming with the box ticked deletes the save and
        // switches the warning off for good (the settings file keeps it).
        run(&mut app, ControlCommand::DeleteSave);
        assert!(!app.confirm_do_not_show);
        run(&mut app, ControlCommand::ToggleDoNotShowAgain);
        run(&mut app, ControlCommand::Confirm);
        assert!(app.confirm_dialog.is_none());
        assert_eq!(app.save_entries.len(), saves - 1);
        assert!(app.warning_off(ActionWarning::DeleteSaveFile));
        assert!(
            crate::user_settings::UserSettings::load(Some(settings_path.clone()))
                .warning_off(ActionWarning::DeleteSaveFile)
        );

        // The next delete goes at once, with no popup (`doNotShowAgain` invokes `afterProceed`).
        run(&mut app, ControlCommand::DeleteSave);
        assert!(app.confirm_dialog.is_none(), "{:?}", app.confirm_dialog);
        assert_eq!(app.save_entries.len(), saves - 2);

        // "Reset Warnings" shows it again.
        run(&mut app, ControlCommand::ResetWarnings);
        assert!(!app.warning_off(ActionWarning::DeleteSaveFile));
        run(&mut app, ControlCommand::DeleteSave);
        assert!(matches!(
            app.confirm_dialog,
            Some(ConfirmKind::DeleteSave(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_settings_file_the_warning_flag_lasts_for_the_run() {
        use crate::ui::ControlCommand;
        use crate::warnings::ActionWarning;
        let dir = std::env::temp_dir().join(format!("rustibites-warnmem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(98, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        assert!(app.user_settings_path.is_none());
        app.save_named("only");
        app.refresh_save_entries();
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::DeleteSave)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ToggleDoNotShowAgain)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(app.warning_off(ActionWarning::DeleteSaveFile));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ResetWarnings)]);
        assert!(!app.warning_off(ActionWarning::DeleteSaveFile));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn plain_choice_dialogs_have_no_do_not_show_again_box() {
        use crate::ui::ControlCommand;
        let mut app = App::new(synthetic(99, 4), Vec2::new(1280.0, 800.0), None);
        app.confirm_dialog = Some(ConfirmKind::KillHalf);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ToggleDoNotShowAgain)]);
        assert!(
            !app.confirm_do_not_show,
            "`DisplayChoiceDialog` has no checkbox"
        );
        assert!(ConfirmKind::KillHalf.warning().is_none());
        assert_eq!(
            ConfirmKind::DeleteSave(PathBuf::new()).warning(),
            Some(crate::warnings::ActionWarning::DeleteSaveFile)
        );
    }

    #[test]
    fn world_settings_controls_apply_live_and_gene_pages_scroll() {
        let mut app = App::new(synthetic(95, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_world_captured_setting(
            "world_settings",
            "Bibite Limit",
            crate::ui::ControlValue::Float(123.0),
        );
        assert_eq!(app.world.settings.independents.bibite_limit, 123);
        app.apply_world_captured_setting(
            "world_settings",
            "Global Biomass Density",
            crate::ui::ControlValue::Float(0.02),
        );
        assert!((app.world.settings.independents.biomass_density - 0.02).abs() < 1e-5);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenGeneEditor)]);
        app.apply_ui_actions(&[UiAction::ScrollGene(1)]);
        assert_eq!(app.editor_gene_page, 1);
        assert_eq!(app.editor_gene, 14);
        app.apply_ui_actions(&[UiAction::ScrollGene(-1)]);
        assert_eq!(app.editor_gene_page, 0);
    }

    #[test]
    fn secondary_panels_report_progress_and_egg_utilities() {
        let mut app = App::new(synthetic(96, 8), Vec2::new(1280.0, 800.0), None);
        let progress = app.achievement_progress();
        assert_eq!(progress.len(), 7);
        assert!(progress.iter().any(|(name, _, _)| name.contains("Bibite")));
        assert!(app.select_first());
        let parent = app.selected.expect("parent");
        app.world.force_lay_eggs(parent);
        if app.world.eggs.ids().next().is_some() {
            app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AbortEggs)]);
            assert!(app.world.eggs.ids().next().is_none());
            assert!(app.last_action.contains("aborted"));
        }
    }

    #[test]
    fn brain_anchors_and_template_save_round_trip() {
        let dir = std::env::temp_dir().join(format!("rustibites-anchor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(97, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenBrainEditor)]);
        app.apply_ui_actions(&[UiAction::SelectBrainNode(0)]);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::EditorAnchorNode,
        )]);
        assert_eq!(app.editor_anchors.len(), 1);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::EditorSaveTemplate,
        )]);
        let path = app
            .storage
            .templates_dir()
            .join("viewer-editor.bb8template");
        assert!(path.is_file(), "template not saved: {}", app.last_action);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::EditorClearAnchors,
        )]);
        assert!(app.editor_anchors.is_empty());
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::EditorLoadTemplate,
        )]);
        assert_eq!(app.editor_anchors.len(), 1, "anchors not restored");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn local_challenge_starts_updates_and_closes() {
        let mut app = App::new(synthetic(98, 6), Vec2::new(1280.0, 800.0), None);
        app.start_local_challenge();
        let session = app.challenge.as_ref().expect("challenge running");
        assert_eq!(session.state, bibites_core::world::ChallengeState::Running);
        app.update_challenge(1.0);
        let (_, stars, elapsed, ended) = app.challenge_status().expect("status");
        assert!(!ended);
        assert!(stars <= 3);
        assert!(elapsed >= 0.0);
        // Simulate an ended session to exercise the popup state.
        if let Some(session) = app.challenge.as_mut() {
            session.state = bibites_core::world::ChallengeState::Ended;
            session.outcome = bibites_core::world::ChallengeOutcome::Completed;
            session.stars = 2;
            session.score = 120.0;
        }
        app.update_challenge(1.0);
        assert!(app.challenge_end_shown || app.challenge.is_some());
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::ChallengeContinue,
        )]);
        assert!(app.challenge.is_none());
        assert!(!app.challenge_end_shown);
    }

    #[test]
    fn workshop_browser_lists_exports_and_deletes_local_files() {
        let dir = std::env::temp_dir().join(format!("rustibites-workshop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let workshop = dir.join("Workshop").join("pack");
        std::fs::create_dir_all(&workshop).expect("workshop dir");
        std::fs::write(workshop.join("readme.txt"), b"hello").expect("file");
        let mut app = App::new(synthetic(99, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenWorkshop)]);
        assert!(app.panels.is_open(crate::ui::PanelKind::Workshop));
        assert!(!app.workshop.is_empty(), "{}", app.last_action);
        app.apply_ui_actions(&[UiAction::SelectWorkshopRow(0)]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::WorkshopExport)]);
        assert!(app.last_action.contains("exported"));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::WorkshopDelete)]);
        assert!(app.last_action.contains("deleted"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn challenge_picker_lists_scenarios_and_champions() {
        let dir = std::env::temp_dir().join(format!("rustibites-chal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Scenarios")).expect("scenarios");
        std::fs::create_dir_all(dir.join("Bibites/Templates")).expect("templates");
        std::fs::write(dir.join("Scenarios/TestChallenge.zip"), b"zip").expect("scenario");
        std::fs::write(dir.join("Bibites/Templates/champion.bb8template"), b"{}")
            .expect("template");
        let mut app = App::new(synthetic(100, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::OpenChallengePicker,
        )]);
        assert!(!app.challenge_scenarios.is_empty(), "{}", app.last_action);
        assert!(!app.champion_templates.is_empty(), "{}", app.last_action);
        app.apply_ui_actions(&[
            UiAction::SelectChallengeRow(0),
            UiAction::SelectChampionRow(0),
        ]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::StartChallenge)]);
        let session = app.challenge.as_ref().expect("challenge started");
        assert!(session.champion_name.contains("champion"));
        assert!(session.champion_path.contains("champion.bb8template"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn embedded_challenge_params_spawn_champion_row() {
        let mut app = App::new(synthetic(101, 6), Vec2::new(1280.0, 800.0), None);
        let mut params = bibites_core::settings::ChallengeParameters::game_defaults();
        params.champion.path = "starter".to_owned();
        params.champion.is_external = false;
        app.challenge_params = Some(params);
        app.start_local_challenge();
        let session = app.challenge.as_ref().expect("challenge started");
        assert_eq!(
            session.parameters.champion.path, "starter",
            "embedded challenge parameters were not used"
        );
        let _ = std::fs::remove_dir_all("target/generated-newgame");
    }

    #[test]
    fn lineage_species_tabs_and_selection() {
        let mut app = App::new(synthetic(102, 6), Vec2::new(1280.0, 800.0), None);
        app.world.species.set_n_records(5);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenLineage)]);
        app.apply_ui_actions(&[UiAction::SetLineageTab(1)]);
        assert_eq!(app.lineage_tab, 1);
        app.apply_ui_actions(&[UiAction::SetLineageTab(0)]);
        assert_eq!(app.lineage_tab, 0);
        if !app.lineage_species.is_empty() {
            app.apply_ui_actions(&[UiAction::SelectSpecies(0)]);
            assert_eq!(app.species_selected, 0);
            assert!(app.selected_species_entry().is_some());
        }
    }

    #[test]
    fn hotkeys_rebind_persist_and_workshop_edit_status() {
        let dir = std::env::temp_dir().join(format!("rustibites-hotkey-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new(synthetic(103, 4), Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.apply_ui_actions(&[UiAction::RebindHotkey(0)]);
        assert_eq!(app.hotkey_listening, Some(0));
        app.assign_hotkey(0, "KeyQ");
        assert_eq!(
            app.prefs.hotkeys.get("pause").map(String::as_str),
            Some("KeyQ")
        );
        let loaded =
            bibites_io::storage::load_user_preferences(&app.storage).expect("prefs reload");
        assert_eq!(
            loaded.hotkeys.get("pause").map(String::as_str),
            Some("KeyQ")
        );
        app.apply_ui_actions(&[UiAction::ResetHotkeys]);
        assert_eq!(
            app.prefs.hotkeys.get("pause").map(String::as_str),
            Some("Space")
        );

        let workshop = dir.join("Workshop").join("old.txt");
        std::fs::create_dir_all(workshop.parent().expect("parent")).expect("dir");
        std::fs::write(&workshop, b"x").expect("file");
        app.refresh_workshop();
        assert!(!app.workshop.is_empty());
        app.workshop_name_draft = "new-name.txt".to_owned();
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::WorkshopRename)]);
        assert!(app.last_action.contains("renamed"), "{}", app.last_action);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::WorkshopOnline)]);
        assert!(app.last_action.contains("Steam/online"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn challenge_parameters_editor_adjusts_and_applies_to_start() {
        let mut app = App::new(synthetic(104, 6), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::OpenChallengePicker,
        )]);
        assert!(app.challenge_draft.is_some());
        app.apply_ui_actions(&[UiAction::AdjustChallengeParam(0, 1)]);
        app.apply_ui_actions(&[UiAction::AdjustChallengeParam(1, 1)]);
        app.start_local_challenge();
        let session = app.challenge.as_ref().expect("challenge started");
        assert_eq!(app.challenge_timer_index, 5);
        assert_eq!(app.challenge_star_indices[0], 3);
        assert_eq!(
            format!("{:?}", session.parameters.exit_condition),
            format!(
                "{:?}",
                app.challenge_draft.as_ref().expect("draft").exit_condition
            )
        );
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::ChallengeSaveParams,
        )]);
        assert!(
            app.last_action.contains("no source scenario") || app.last_action.contains("saved")
        );
    }

    #[test]
    fn native_utility_controls_have_observable_effects() {
        let mut app = App::new(synthetic(105, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::ScrollList)]);
        assert_eq!(app.list_scroll, 1);
        assert!(app.last_action.contains("scroll"), "{}", app.last_action);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenFolder)]);
        assert!(app.last_action.contains("opened"), "{}", app.last_action);
    }

    fn press_z() -> FrameInput {
        FrameInput {
            select_random_zone: true,
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    #[test]
    fn z_selects_a_zone_and_opens_its_item_on_the_zones_tab() {
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(131, 4), Vec2::new(1280.0, 800.0), None);
        assert!(
            !app.world.zones.zones.is_empty(),
            "the starter world has zones"
        );
        // The Zones list holds the explicit zones: one is added, and Z picks among all the zones.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        app.dyn_panel.open_items.clear();
        app.handle_input(&press_z());
        let index = app.selected_zone.expect("Z picked a zone");
        // `GUIManager.OnTargetChanged`: the Dynamic panel opens on the Zones tab, the item open (a zone a group generated has no item).
        assert!(app.panels.is_open(PanelKind::WorldDynamics));
        assert_eq!(app.dyn_panel.tab, 1);
        if index < app.world.zones.explicit_count() {
            assert_eq!(app.dyn_panel.open_items, vec![(0, index as u16)]);
        } else {
            assert!(app.dyn_panel.open_items.is_empty());
        }
        assert!(app.has_target() && !app.target_is_bibite_or_egg());
        // Escape peels the panel off first, then the target (`bibiteSelectedAction`).
        app.handle_input(&escape());
        assert!(!app.panels.is_open(PanelKind::WorldDynamics));
        assert_eq!(app.selected_zone, Some(index));
        app.handle_input(&escape());
        assert_eq!(app.selected_zone, None);
        assert!(
            !app.panels.is_open(PanelKind::Escape),
            "the pause menu stays shut"
        );
        // Another target replaces the zone: a bibite pick drops it.
        app.handle_input(&press_z());
        assert!(app.selected_zone.is_some());
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.selected_zone.is_none() && app.selected.is_some());
    }

    #[test]
    fn z_without_open_zone_editor_on_select_only_targets_the_zone() {
        use crate::ui::PanelKind;
        let mut app = App::new(synthetic(132, 4), Vec2::new(1280.0, 800.0), None);
        app.open_zone_editor_on_select = false;
        app.handle_input(&press_z());
        assert!(app.selected_zone.is_some());
        assert!(!app.panels.is_open(PanelKind::WorldDynamics));
        assert!(app.dyn_panel.open_items.is_empty());
        // The target still has its Escape entry.
        app.handle_input(&escape());
        assert!(app.selected_zone.is_none());
    }

    #[test]
    fn z_does_nothing_without_zones_or_while_a_placer_blocks_selection() {
        let mut app = App::new(synthetic(133, 4), Vec2::new(1280.0, 800.0), None);
        // A pellet placer turns `AllowSelection` off.
        app.tool = crate::ui::ToolKind::Pellet;
        app.handle_input(&press_z());
        assert!(app.selected_zone.is_none());
        app.tool = crate::ui::ToolKind::Inspect;
        while let Some(last) = app.world.zones.zones.len().checked_sub(1) {
            let id = app.world.zones.zone_id_at(last).expect("zone id");
            app.world.remove_zone(id).expect("removed");
        }
        app.handle_input(&press_z());
        assert!(
            app.selected_zone.is_none(),
            "RandomElement of nothing selects nothing"
        );
    }

    // ---- typed values (`SettingSlider.OpenEditPanel`) ------------------------------------------

    /// Types `text` into whatever field is focused, in one frame.
    fn type_into(app: &mut App, text: &str) {
        app.typed = text.to_owned();
        app.handle_input(&FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        });
    }

    fn press_key(app: &mut App, input: FrameInput) {
        app.handle_input(&FrameInput { dt: 0.016, ..input });
    }

    #[test]
    fn a_slider_rows_typed_value_applies_on_enter_and_a_linear_one_is_not_revertable() {
        use crate::dyn_panel::FieldRef;
        let mut app = App::new(synthetic(140, 4), Vec2::new(1280.0, 800.0), None);
        let (index, row) = crate::dyn_panel::ui_of("dragCoefficient").expect("a linear row");
        let field = FieldRef::Setting(index as u16);
        let undoable = app.undo.undoable();
        // The value text opens the field with the value times the factor, selected.
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        let edit = app.value_edit.as_ref().expect("the field opens");
        let now = crate::dyn_panel::read_value(&app.world.settings, row);
        assert_eq!(edit.text, crate::value_edit::dotnet_float_string(now));
        assert!(edit.selected());
        // The first valid key replaces the selection; a letter is not taken.
        type_into(&mut app, "x4");
        assert_eq!(app.value_edit.as_ref().map(|e| e.text.as_str()), Some("4"));
        type_into(&mut app, "0");
        // 40 is past the slider's 25, which a setting that `canGoOutOfBounds` keeps.
        press_key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert!(app.value_edit.is_none(), "Enter closes the field");
        assert_eq!(
            crate::dyn_panel::read_value(&app.world.settings, row),
            40.0,
            "{}",
            app.last_action
        );
        assert_eq!(
            app.undo.undoable(),
            undoable,
            "`SubmitEditValue` of a linear slider is not revertable"
        );
    }

    #[test]
    fn a_log_sliders_typed_value_clamps_first_and_is_on_the_history() {
        use crate::dyn_panel::FieldRef;
        let mut app = App::new(synthetic(141, 4), Vec2::new(1280.0, 800.0), None);
        let (index, row) = crate::dyn_panel::ui_of("minPelletSize").expect("a log row");
        let field = FieldRef::Setting(index as u16);
        let original = app.world.settings.clone();
        let undoable = app.undo.undoable();
        // Ctrl + Z needs a panel open (the game returns first with an empty Escape stack).
        app.apply_ui_actions(&[UiAction::OpenPanel(crate::ui::PanelKind::WorldDynamics)]);
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        // 50 is past the setting's 10 and it cannot go out of bounds.
        type_into(&mut app, "50");
        press_key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(
            crate::dyn_panel::read_value(&app.world.settings, row),
            10.0,
            "{}",
            app.last_action
        );
        assert_eq!(
            app.undo.undoable(),
            undoable + 1,
            "`LogSettingSlider.SubmitEditValue` is revertable"
        );
        press_key(&mut app, undo_key(false));
        assert_eq!(app.world.settings, original);
    }

    #[test]
    fn escape_and_a_press_elsewhere_close_the_field_without_applying_it() {
        use crate::dyn_panel::FieldRef;
        let mut app = App::new(synthetic(142, 4), Vec2::new(1280.0, 800.0), None);
        let (index, row) = crate::dyn_panel::ui_of("dragCoefficient").expect("a row");
        let field = FieldRef::Setting(index as u16);
        let original = crate::dyn_panel::read_value(&app.world.settings, row);
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        type_into(&mut app, "3");
        press_key(
            &mut app,
            FrameInput {
                quit: true,
                ..FrameInput::default()
            },
        );
        assert!(app.value_edit.is_none());
        assert_eq!(
            crate::dyn_panel::read_value(&app.world.settings, row),
            original
        );
        assert!(
            !app.panels.is_open(crate::ui::PanelKind::Escape),
            "the Escape closed the field and nothing else"
        );
        // A press on the field itself keeps it; on any other control it closes it.
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        app.apply_ui_actions(&[UiAction::ValueEditInside]);
        assert!(app.value_edit.is_some());
        app.apply_ui_actions(&[UiAction::DynTab(1)]);
        assert!(app.value_edit.is_none());
        assert_eq!(
            crate::dyn_panel::read_value(&app.world.settings, row),
            original
        );
        // A press on the world does too.
        app.apply_ui_actions(&[UiAction::DynEditValue(field)]);
        press_key(
            &mut app,
            FrameInput {
                primary_pressed: true,
                cursor: Vec2::new(300.0, 300.0),
                ..FrameInput::default()
            },
        );
        assert!(app.value_edit.is_none());
    }

    #[test]
    fn the_keyboard_is_blocked_while_a_value_is_typed() {
        use crate::dyn_panel::FieldRef;
        let mut app = App::new(synthetic(143, 4), Vec2::new(1280.0, 800.0), None);
        let (index, _) = crate::dyn_panel::ui_of("dragCoefficient").expect("a row");
        app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::Setting(index as u16))]);
        let paused = app.paused;
        // Space does not pause (`UserControl.SetKeyboardBlockFromSource("SliderEdit")`).
        press_key(
            &mut app,
            FrameInput {
                toggle_pause: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.paused, paused);
        // Backspace deletes the selection, the field stays open.
        press_key(
            &mut app,
            FrameInput {
                backspace: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.value_edit.as_ref().map(|e| e.text.as_str()), Some(""));
        // An empty text is a zero.
        press_key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert!(app.value_edit.is_none());
        let (_, row) = crate::dyn_panel::ui_of("dragCoefficient").expect("a row");
        assert_eq!(crate::dyn_panel::read_value(&app.world.settings, row), 0.0);
    }

    /// A tester: "I can't click on the numbers to type in how fast I want it to go". The time-speed slider's number is its edit button
    /// (`SettingSlider.OpenEditPanel`): a typed speed is kept past the slider's 100 (`canGoOutOfBounds`), any positive speed plays, and a zero pauses.
    #[test]
    fn a_speed_can_be_typed_into_the_time_slider() {
        use crate::dyn_panel::FieldRef;
        let mut app = App::new(synthetic(145, 4), Vec2::new(1280.0, 800.0), None);
        app.paused = true;
        app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::TimeSpeed)]);
        let edit = app.value_edit.as_ref().expect("the field opens");
        assert_eq!(edit.text, crate::value_edit::dotnet_float_string(app.speed));
        type_into(&mut app, "7.5");
        press_key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert!(app.value_edit.is_none(), "Enter closes the field");
        assert_eq!(app.speed, 7.5);
        assert!(!app.paused, "a positive target time scale forces play");
        // A zero is a pause (`engineTimeScale` 0); the target keeps its place on the slider.
        app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::TimeSpeed)]);
        type_into(&mut app, "0");
        press_key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert!(app.paused);
        // Escape or a press elsewhere leaves it as it was.
        let speed = app.speed;
        app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::TimeSpeed)]);
        type_into(&mut app, "3");
        app.apply_ui_actions(&[UiAction::TogglePause]);
        assert!(app.value_edit.is_none());
        assert_eq!(app.speed, speed);
    }

    /// A tester: pellet sizes over 50x came out as 50x. `pelletSize` goes up to 50 on its slider but is `canGoOutOfBounds` like the fertility and biomass density: the
    /// number typed into it is kept, and a load keeps the value a save holds (the position, the radius and the other sizes are the ones the game clamps).
    #[test]
    fn a_zones_pellet_size_typed_past_the_slider_is_kept_and_a_position_is_not() {
        use crate::dyn_panel::FieldRef;
        use crate::zone_editor::{PELLET_SIZE, POS_X};
        let mut app = App::new(synthetic(146, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let index = (app.world.zones.zones.len() - 1) as u16;
        for (field, typed) in [(PELLET_SIZE, "100"), (POS_X, "5")] {
            app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::Zone(index, field))]);
            type_into(&mut app, typed);
            press_key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        }
        let config = &app.world.zones.zones[usize::from(index)].config.0;
        assert_eq!(config.pellet_size, 100.0, "{}", app.last_action);
        // The position the game clamps to its range (and keeps the zone inside the area): a 5 is not kept.
        assert!(config.pos_x <= 1.0 && config.pos_x > 0.0, "{}", config.pos_x);
    }

    /// A tester: "the bibites vibrate when you lock onto one". `CameraManager.LateUpdate` puts the camera on the followed bibite AFTER the frame's physics,
    /// so the bibite is in the middle of every picture. The follow ran before the frame's ticks: on each frame with a tick the bibite was drawn one tick of
    /// movement off the centre (and exactly on it on the frames without one), a flicker at the display's rate.
    #[test]
    fn the_camera_locked_on_a_bibite_ends_every_frame_where_the_bibite_is_after_the_ticks() {
        let mut app = App::new(synthetic(147, 6), Vec2::new(1280.0, 800.0), None);
        let id = app.world.live_bibite_ids()[0];
        app.select_bibite(id);
        app.paused = false;
        app.speed = 1.0;
        // A bibite on the move: one tick of it is a step the picture can show.
        app.world.bibites.get_mut(id).expect("the bibite").vel = Vec2::new(30.0, 0.0);
        let fixed = app.world.settings.fixed_dt();
        let mut moved = 0;
        for _ in 0..12 {
            let before = app.world.bibites.get(id).expect("alive").pos;
            let ticks = app.sim_ticks;
            app.advance(fixed);
            assert_eq!(app.sim_ticks, ticks + 1, "a frame as long as a tick runs one tick");
            let after = app.world.bibites.get(id).expect("alive").pos;
            moved += usize::from(after != before);
            assert_eq!(app.camera.center, after, "the camera is where the bibite is after the tick, not where it was before it");
        }
        assert!(moved > 0, "the bibite moved, or the test would not see the lag");
        // Frames that run no tick (a display faster than the ticks) keep the bibite in the middle as well.
        for _ in 0..4 {
            app.advance(fixed / 4.0);
            let now = app.world.bibites.get(id).expect("alive").pos;
            assert_eq!(app.camera.center, now);
        }
        // And while paused: a bibite that is moved (held by the pointer, put somewhere by a tool) carries the camera.
        app.paused = true;
        app.world.bibites.get_mut(id).expect("alive").pos = Vec2::new(123.0, -45.0);
        app.advance(fixed);
        assert_eq!(app.camera.center, Vec2::new(123.0, -45.0));
    }

    #[test]
    fn a_zone_sliders_typed_value_goes_past_its_ends_and_a_material_one_too() {
        use crate::dyn_panel::FieldRef;
        use crate::zone_editor::{BIOMASS, MAT_ENERGY};
        let mut app = App::new(synthetic(144, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let index = (app.world.zones.zones.len() - 1) as u16;
        // The zone's Biomass slider ends at 100; the setting can go out of bounds.
        app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::Zone(index, BIOMASS))]);
        type_into(&mut app, "250");
        press_key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(
            app.world.zones.zones[usize::from(index)]
                .config
                .0
                .biomass_density,
            250.0,
            "{}",
            app.last_action
        );
        // The Energy tab's material sliders the same way (the plant's energy density).
        let before = app
            .world
            .settings
            .materials
            .get(bibites_core::settings::MaterialKind::Plant)
            .energy_density;
        app.apply_ui_actions(&[UiAction::DynEditValue(FieldRef::Material(0, MAT_ENERGY))]);
        let edit = app.value_edit.as_ref().expect("opens");
        assert_eq!(edit.text, crate::value_edit::dotnet_float_string(before));
        type_into(&mut app, "9999");
        press_key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(
            app.world
                .settings
                .materials
                .get(bibites_core::settings::MaterialKind::Plant)
                .energy_density,
            9999.0,
            "{}",
            app.last_action
        );
    }

    #[test]
    fn the_zone_controls_edit_the_live_zone() {
        use crate::zone_editor::{MATERIAL, MOVEMENT, RENEW, TARGET};
        use bibites_core::settings::{MaterialKind, MovementType};
        let mut app = App::new(synthetic(134, 4), Vec2::new(1280.0, 800.0), None);
        // Two zones to work with, the second one fresh.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let second = app.world.zones.zones.len() - 1;
        let index = second as u16;
        // A toggle flips the live zone's setting.
        assert!(!app.world.zones.zones[second].config.0.renew_biomass);
        app.apply_ui_actions(&[UiAction::ZoneToggle(index, RENEW)]);
        assert!(app.world.zones.zones[second].config.0.renew_biomass);
        // A dropdown goes to its next choice: Plant -> Meat -> None.
        assert_eq!(
            app.world.zones.zones[second].config.0.material,
            Some(MaterialKind::Plant)
        );
        app.apply_ui_actions(&[UiAction::ZoneCycle(index, MATERIAL)]);
        assert_eq!(
            app.world.zones.zones[second].config.0.material,
            Some(MaterialKind::Meat)
        );
        // Reset puts the game's default back.
        app.apply_ui_actions(&[UiAction::ZoneReset(index, MATERIAL)]);
        assert_eq!(
            app.world.zones.zones[second].config.0.material,
            Some(MaterialKind::Plant)
        );
        // Following another zone: movement "Locked to Target" and a target make it follow.
        app.apply_ui_actions(&[UiAction::ZoneCycle(index, MOVEMENT)]);
        app.apply_ui_actions(&[UiAction::ZoneCycle(index, MOVEMENT)]);
        assert_eq!(
            app.world.zones.zones[second].config.0.movement,
            MovementType::Attached
        );
        app.apply_ui_actions(&[UiAction::ZoneCycle(index, TARGET)]);
        assert!(app.world.zones.zones[second].config.0.target.is_some());
        assert!(
            app.world.zones.zones[second].target_index.is_some(),
            "the zone follows its target at once"
        );
        // Back to no movement drops the target.
        app.apply_ui_actions(&[UiAction::ZoneCycle(index, MOVEMENT)]);
        assert_eq!(app.world.zones.zones[second].config.0.target, None);
        assert_eq!(app.world.zones.zones[second].target_index, None);
    }

    #[test]
    fn dragging_a_zone_slider_moves_the_live_zone() {
        use crate::zone_editor::{FERTILITY, POS_X};
        let mut app = App::new(synthetic(135, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let index = app.world.zones.zones.len() - 1;
        // The slider track spans reference x 100..500 (the cursor is in logical pixels; the
        // 1280-wide window scales the 1920 reference by 2/3).
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        let at = |fraction: f32| (100.0 + 400.0 * fraction) * scale;
        let before = app.world.zones.zones[index].config.0.pos_x;
        app.zone_slide(index, POS_X, (100.0, 500.0), at(0.75));
        let moved = app.world.zones.zones[index].config.0.pos_x;
        assert!(moved > before, "{moved} should be right of {before}");
        // The runtime position follows the setting.
        let size = app.world.settings.independents.simulation_size;
        let zone = &app.world.zones.zones[index];
        assert!((zone.pos.x - moved * size).abs() < 1.0);
        // A log slider moves the other way round when dragged left.
        let rich = {
            app.zone_slide(index, FERTILITY, (100.0, 500.0), at(1.0));
            app.world.zones.zones[index].config.0.fertility
        };
        app.zone_slide(index, FERTILITY, (100.0, 500.0), at(0.0));
        assert!(app.world.zones.zones[index].config.0.fertility < rich);
    }

    #[test]
    fn zones_are_added_deleted_after_asking_and_cleared() {
        use crate::app::ConfirmKind;
        let mut app = App::new(synthetic(136, 4), Vec2::new(1280.0, 800.0), None);
        // (The starter world's zones belong to its group: the list holds the zones added here.)
        let total = app.world.zones.zones.len();
        let before = app.world.zones.explicit_count();
        // `ZonesEditorPanel.AddZone`: a Plant zone "Zone n" at once, its item open.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        assert_eq!(app.world.zones.zones.len(), total + 1);
        let added = before;
        assert_eq!(
            app.world.zones.zones[added].config.0.name,
            format!("Zone {before}")
        );
        assert!(app.dyn_panel.open_items.contains(&(0, added as u16)));
        // Another zone follows it and one bibite row spawns in it.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        // The delete button asks first (`ZoneSettingsHandle.DeleteZone`), then removes.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::DeleteZone(
            added as u16,
        ))]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::DeleteZone(added)));
        assert_eq!(
            app.world.zones.zones.len(),
            total + 2,
            "nothing is deleted yet"
        );
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Cancel)]);
        assert!(app.confirm_dialog.is_none());
        assert_eq!(app.world.zones.zones.len(), total + 2);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::DeleteZone(
            added as u16,
        ))]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        assert_eq!(app.world.zones.zones.len(), total + 1);
        // The open flag of the later zone moved up with it.
        assert!(app.dyn_panel.open_items.contains(&(0, added as u16)));
        assert!(!app.dyn_panel.open_items.contains(&(0, added as u16 + 1)));
        // "Remove all" asks too, then empties the list.
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::RequestClearZones,
        )]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::ClearZones));
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::Confirm)]);
        assert_eq!(app.world.zones.explicit_count(), 0);
        assert_eq!(app.world.zones.zones.len(), total, "the group's zones stay");
        assert!(app.dyn_panel.open_items.is_empty());
        // The button is disabled with nothing to remove.
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::RequestClearZones,
        )]);
        assert!(app.confirm_dialog.is_none());
    }

    #[test]
    fn deleting_the_selected_zone_ends_the_target_and_later_zones_keep_theirs() {
        let mut app = App::new(synthetic(137, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let last = app.world.zones.explicit_count() - 1;
        app.select_zone(last);
        assert_eq!(app.selected_zone, Some(last));
        // A zone before the selected one goes: the selection moves up with its zone.
        app.delete_zone(last - 1);
        assert_eq!(app.selected_zone, Some(last - 1));
        assert_eq!(
            app.world.zones.zones[last - 1].config.0.name,
            format!("Zone {}", last),
            "the selected zone is still the second added one"
        );
        // The selected zone itself goes: `TargetDied` clears the target.
        app.delete_zone(last - 1);
        assert_eq!(app.selected_zone, None);
        assert!(!app.has_target());
    }

    #[test]
    fn the_camera_follows_a_selected_zone_or_tower_like_any_target() {
        use crate::zone_editor::POS_X;
        let mut app = App::new(synthetic(140, 4), Vec2::new(1280.0, 800.0), None);
        // A zone away from the middle: Z-selecting it centres the camera on it.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let index = app.world.zones.explicit_count() - 1;
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        app.zone_slide(index, POS_X, (100.0, 500.0), (100.0 + 400.0 * 0.9) * scale);
        let at = app.world.zones.zones[index].pos;
        assert!(at.x.abs() > 100.0, "the zone is off centre: {at:?}");
        app.select_zone(index);
        app.advance(0.016);
        assert!(
            (app.camera.center - at).length() < 1e-2,
            "{:?} vs {at:?}",
            app.camera.center
        );
        // It keeps following the zone when the zone moves.
        app.zone_slide(index, POS_X, (100.0, 500.0), (100.0 + 400.0 * 0.2) * scale);
        let moved = app.world.zones.zones[index].pos;
        app.advance(0.016);
        assert!((app.camera.center - moved).length() < 1e-2);
        // Steering with the arrow keys stops the follow, as it does for a selected bibite.
        app.handle_input(&FrameInput {
            move_axis: Vec2::new(1.0, 0.0),
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(!app.follow_selected);
        // A selected tower is followed the same way.
        let id = app.world.place_rad_tower_at(Vec2::new(321.0, -123.0), None);
        app.select_placed_tool(id);
        app.advance(0.016);
        assert!((app.camera.center - Vec2::new(321.0, -123.0)).length() < 1e-2);
        assert!(
            app.selected_zone.is_none(),
            "the tower replaced the zone as the target"
        );
    }

    #[test]
    fn the_material_blocks_edit_the_live_material_table() {
        use crate::zone_editor::{MAT_ENERGY, MAT_MASS};
        use bibites_core::settings::{MaterialKind, MaterialSettings};
        let mut app = App::new(synthetic(141, 4), Vec2::new(1280.0, 800.0), None);
        let defaults = MaterialSettings::game_defaults();
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        let at = |fraction: f32| (100.0 + 400.0 * fraction) * scale;
        let plant = |app: &App| *app.world.settings.materials.get(MaterialKind::Plant);
        // A slider drag changes the plant's energy density in the running world.
        let before = plant(&app).energy_density;
        app.material_slide(0, MAT_ENERGY, (100.0, 500.0), at(0.95));
        let after = plant(&app).energy_density;
        assert!(after > before, "{after} should be above {before}");
        // The other blocks are separate: the meat is untouched.
        assert_eq!(
            app.world
                .settings
                .materials
                .get(MaterialKind::Meat)
                .energy_density,
            defaults.get(MaterialKind::Meat).energy_density
        );
        // The decay switch flips and the pointer puts the default back.
        let decay = plant(&app).decay;
        app.apply_ui_actions(&[UiAction::MaterialToggle(0)]);
        assert_eq!(plant(&app).decay, !decay);
        app.apply_ui_actions(&[UiAction::MaterialReset(0, MAT_ENERGY)]);
        assert_eq!(
            plant(&app).energy_density,
            defaults.get(MaterialKind::Plant).energy_density
        );
        // "Reset all" of a block restores every parameter of that material.
        app.material_slide(1, MAT_MASS, (100.0, 500.0), at(0.1));
        let meat = |app: &App| *app.world.settings.materials.get(MaterialKind::Meat);
        assert_ne!(meat(&app), *defaults.get(MaterialKind::Meat));
        app.apply_ui_actions(&[UiAction::MaterialResetAll(1)]);
        assert_eq!(meat(&app), *defaults.get(MaterialKind::Meat));
        // The edits are on the Ctrl + Z history (with a panel open on the Escape stack).
        app.panels.open(crate::ui::PanelKind::WorldDynamics);
        app.handle_input(&FrameInput {
            undo: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert_ne!(
            meat(&app),
            *defaults.get(MaterialKind::Meat),
            "undo took back the reset"
        );
    }

    #[test]
    fn the_decay_switch_of_the_materials_reaches_the_pellets_already_in_the_world() {
        use bibites_core::pellet::MaterialKind as PelletKind;
        let mut app = App::new(synthetic(142, 4), Vec2::new(1280.0, 800.0), None);
        let plant = app
            .world
            .spawn_pellet_with_amount(PelletKind::Plant, Vec2::new(40.0, 40.0), 20.0)
            .expect("a plant pellet");
        assert!(!app.world.pellets.is_decaying(plant), "plants do not decay by default");
        // The Energy tab's Decay switch of the Plant block (`SettingsUpdater.UpdateDecay`): the plants that are already there start to decay,
        // with a fresh timer and no rot; switched off again they leave the decay processor.
        app.apply_ui_actions(&[UiAction::MaterialToggle(0)]);
        assert!(app.world.pellets.is_decaying(plant), "the switch registered the plant that was already there");
        assert_eq!(app.world.pellets.rot_amount(plant), Some(0.0));
        app.apply_ui_actions(&[UiAction::MaterialToggle(0)]);
        assert!(!app.world.pellets.is_decaying(plant));
    }

    #[test]
    fn the_apps_simulation_switches_reach_whatever_world_it_holds_before_it_ticks() {
        use bibites_core::improved::{Feature, ImprovedFeatures};
        let mut app = App::new(synthetic(143, 4), Vec2::new(1280.0, 800.0), None);
        assert!(app.improved.is_faithful() && app.world.improved.is_faithful(), "nothing chosen: the Faithful simulation");
        let on = ImprovedFeatures::FAITHFUL.with(Feature::SwitchProbe, true);
        app.improved = on;
        assert!(app.world.improved.is_faithful(), "the world has them only once it ticks");
        app.tick_world();
        assert_eq!(app.world.improved, on);
        // A world that replaces it (a load, a restore, a new game) starts with every switch off, and gets them with its first tick.
        app.world = synthetic(144, 4).world;
        assert!(app.world.improved.is_faithful());
        app.tick_world();
        assert_eq!(app.world.improved, on);
        // Switched off again: off again.
        app.improved = ImprovedFeatures::FAITHFUL;
        app.tick_world();
        assert!(app.world.improved.is_faithful());
    }

    #[test]
    fn the_pointer_resting_on_a_pellet_or_a_control_feeds_the_tooltip_timer() {
        use crate::tooltip::{Target, Tip, TipRect, find};
        use bibites_core::pellet::{MaterialKind, PelletSpawn};
        let mut app = App::new(synthetic(150, 3), Vec2::new(1280.0, 800.0), None);
        // A pellet away from every body.
        let mut spot = app.camera.center + Vec2::new(30.0, 20.0);
        for _ in 0..50 {
            if crate::selection::entity_at(&app.world, spot).is_none() {
                break;
            }
            spot += Vec2::new(25.0, 0.0);
        }
        let id = app
            .world
            .spawn_pellet(PelletSpawn::new(MaterialKind::Plant, 40.0, spot))
            .expect("pellet");
        let screen = app.camera.world_to_screen(spot);
        let none = crate::ui::UiFrame::new(1.0);
        // The pointer has to rest for half a second (real time) with no UI under it.
        app.update_tooltip(&none, screen, false, 0.3);
        assert!(app.tooltip.shown().is_none());
        app.update_tooltip(&none, screen, false, 0.3);
        assert!(matches!(app.tooltip.shown(), Some((Target::Pellet(shown), _)) if shown == id));
        // UI under the pointer hides it; so does moving off the pellet.
        app.update_tooltip(&none, screen, true, 0.1);
        assert!(app.tooltip.shown().is_none());
        app.update_tooltip(&none, screen, false, 0.6);
        assert!(app.tooltip.shown().is_some());
        app.update_tooltip(&none, screen + Vec2::new(300.0, 0.0), false, 0.1);
        assert!(app.tooltip.shown().is_none());
        // Hidden pellets have no tooltip.
        app.scene_options.pellets = false;
        app.update_tooltip(&none, screen, false, 0.6);
        assert!(app.tooltip.shown().is_none());
        app.scene_options.pellets = true;
        // A control with a trigger wins over what is behind it.
        let pause = find("UICanvas/MainUI/Layer0/Controls/TopCenter/PauseButton").expect("tip");
        let mut over = crate::ui::UiFrame::new(1.0);
        over.tips.push(TipRect {
            pos: screen - Vec2::new(10.0, 10.0),
            size: Vec2::new(20.0, 20.0),
            tip: pause,
        });
        app.update_tooltip(&over, screen, true, 0.6);
        assert!(
            matches!(app.tooltip.shown(), Some((Target::Control(tip), _)) if tip.title == pause.title)
        );
        // A trigger whose `isUITooltip` is false never shows on a UI element.
        let hidden = Tip { ui: false, ..pause };
        let mut never = crate::ui::UiFrame::new(1.0);
        never.tips.push(TipRect {
            pos: screen - Vec2::new(10.0, 10.0),
            size: Vec2::new(20.0, 20.0),
            tip: hidden,
        });
        app.update_tooltip(&never, screen, true, 0.6);
        assert!(app.tooltip.shown().is_none());
        // A dialog's screen blocker hides the tooltips behind it.
        app.confirm_dialog = Some(ConfirmKind::KillHalf);
        app.update_tooltip(&over, screen, true, 0.6);
        assert!(app.tooltip.shown().is_none());
    }

    #[test]
    fn a_zones_name_is_a_text_field_that_renames_on_enter_and_closes_on_escape() {
        let mut app = App::new(synthetic(151, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let index = app.world.zones.zones.len() - 1;
        let name = |app: &App| app.world.zones.zones[index].config.0.name.clone();
        let original = name(&app);
        let key = |backspace: bool, enter: bool| FrameInput {
            dt: 0.016,
            backspace,
            enter,
            ..FrameInput::default()
        };
        // A click on the name starts the field with the current name.
        app.apply_ui_actions(&[UiAction::ZoneNameEdit(index as u16)]);
        assert_eq!(app.zone_name_edit, Some((index, original.clone())));
        // Typing goes to the field and to nothing else: hotkeys are blocked meanwhile.
        app.typed = "!r".to_owned();
        app.handle_input(&key(false, false));
        assert_eq!(
            app.zone_name_edit.as_ref().map(|(_, text)| text.as_str()),
            Some(format!("{original}!r").as_str())
        );
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(
            app.selected.is_none(),
            "R typed into the field, not a hotkey"
        );
        assert_eq!(name(&app), original, "nothing is renamed before Enter");
        app.handle_input(&key(true, false));
        // Enter submits.
        app.typed = "Pond".to_owned();
        app.handle_input(&key(false, false));
        app.handle_input(&key(false, true));
        assert!(app.zone_name_edit.is_none());
        assert_eq!(name(&app), format!("{original}!Pond"));
        // Escape closes the field without renaming; so does a click on the world.
        app.apply_ui_actions(&[UiAction::ZoneNameEdit(index as u16)]);
        app.typed = "x".to_owned();
        app.handle_input(&key(false, false));
        app.handle_input(&FrameInput {
            quit: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.zone_name_edit.is_none());
        assert_eq!(name(&app), format!("{original}!Pond"));
        assert!(
            !app.panels.is_open(crate::ui::PanelKind::Escape),
            "Escape only closed the field"
        );
        app.apply_ui_actions(&[UiAction::ZoneNameEdit(index as u16)]);
        app.handle_input(&FrameInput {
            primary_pressed: true,
            primary_down: true,
            cursor: Vec2::new(100.0, 400.0),
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(app.zone_name_edit.is_none());
        // Typing without a field does not touch the zone's name.
        app.typed = "zzz".to_owned();
        app.handle_input(&key(false, false));
        assert_eq!(name(&app), format!("{original}!Pond"));
        // Deleting the zone while its field is open drops the field.
        app.apply_ui_actions(&[UiAction::ZoneNameEdit(index as u16)]);
        app.delete_zone(index);
        assert!(app.zone_name_edit.is_none());
    }

    #[test]
    fn the_include_autosaves_checkbox_is_a_persisted_user_setting() {
        let dir = std::env::temp_dir().join(format!("rustibites-autosaves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.json");
        let mut app = App::new(synthetic(152, 2), Vec2::new(1280.0, 800.0), None);
        app.user_settings_path = Some(path.clone());
        app.include_autosaves = false;
        let stored = || crate::user_settings::UserSettings::load(Some(path.clone()));
        // The Load Game panel's checkbox (`LoadGamePanel`) writes the setting at once.
        app.apply_ui_actions(&[UiAction::ToggleAutosaves]);
        assert!(app.include_autosaves);
        assert!(stored().flag("Include autosaves"));
        app.apply_ui_actions(&[UiAction::ToggleAutosaves]);
        assert!(!app.include_autosaves);
        assert!(!stored().flag("Include autosaves"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn press_k() -> FrameInput {
        FrameInput {
            select_autopanner: true,
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    #[test]
    fn k_makes_the_autopanner_the_target_and_the_camera_follows_it_while_the_world_runs() {
        let mut app = App::new(synthetic(153, 4), Vec2::new(1280.0, 800.0), None);
        assert!(
            app.autopan.pos == Vec2::ZERO,
            "the object starts at the origin"
        );
        app.handle_input(&press_k());
        assert!(app.selected_autopan && app.has_target() && !app.target_is_bibite_or_egg());
        // It drifts at up to 5 u/s with the simulation and the camera follows.
        for _ in 0..30 {
            app.advance(0.2);
        }
        assert!(app.autopan.pos.length() > 1.0, "{:?}", app.autopan.pos);
        assert!(app.autopan.vel.length() <= crate::autopanner::MAX_SPEED);
        app.advance(0.001);
        assert!(
            (app.camera.center - app.autopan.pos).length() < 0.5,
            "{:?} vs {:?}",
            app.camera.center,
            app.autopan.pos
        );
        // Selecting it again does nothing; Escape deselects it.
        app.handle_input(&press_k());
        assert!(app.selected_autopan);
        app.handle_input(&escape());
        assert!(!app.selected_autopan && !app.has_target());
        // It only moves while the game runs.
        app.paused = true;
        let before = app.autopan.pos;
        app.advance(2.0);
        assert_eq!(app.autopan.pos, before);
        app.paused = false;
        // Another target replaces it; a placer turns the key off.
        app.handle_input(&press_k());
        app.handle_input(&FrameInput {
            select_random_bibite: true,
            dt: 0.016,
            ..FrameInput::default()
        });
        assert!(!app.selected_autopan && app.selected.is_some());
        app.clear_target();
        app.tool = crate::ui::ToolKind::Pellet;
        app.handle_input(&press_k());
        assert!(!app.selected_autopan);
    }

    #[test]
    fn the_autopanner_heads_for_random_places_inside_the_zones() {
        let mut app = App::new(synthetic(154, 2), Vec2::new(1280.0, 800.0), None);
        let size = app.world.settings.independents.simulation_size;
        let mut places = Vec::new();
        for _ in 0..40 {
            let place = app.random_zone_position();
            assert!(place.x.abs() <= size && place.y.abs() <= size, "{place:?}");
            places.push(place);
        }
        assert!(
            places.iter().any(|place| *place != places[0]),
            "the places differ from one pick to the next"
        );
        // Without zones the place is the origin.
        while let Some(last) = app.world.zones.zones.len().checked_sub(1) {
            let id = app.world.zones.zone_id_at(last).expect("zone id");
            app.world.remove_zone(id).expect("removed");
        }
        assert_eq!(app.random_zone_position(), Vec2::ZERO);
    }

    #[test]
    fn loading_another_world_forgets_the_zone_editor_state() {
        let mut app = App::new(synthetic(138, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        app.select_zone(0);
        assert!(app.selected_zone.is_some() && !app.dyn_panel.open_items.is_empty());
        app.forget_zone_editor();
        assert!(app.selected_zone.is_none() && app.dyn_panel.open_items.is_empty());
    }

    #[test]
    fn the_animators_list_creates_edits_and_deletes_settings_animators() {
        use crate::dyn_panel::FieldRef;
        use crate::ui::{ControlCommand, DropdownTarget};
        use crate::zone_editor::{self as ze, ANIM_BASE_FIRST, ANIM_REPEAT, ANIM_START};
        let mut app = App::new(synthetic(150, 4), Vec2::new(1280.0, 800.0), None);
        // `+` opens the creator; its preset dropdown decides what OK makes.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        assert!(app.animator.creator.open);
        assert!(app.animator_dropdown_pick(DropdownTarget::AnimatorPreset, 1));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)]);
        assert!(!app.animator.creator.open);
        assert_eq!(app.world.settings_changers.len(), 1);
        assert_eq!(app.world.settings_changers[0].name, "Cycle Animator");
        assert!(app.world.settings_changers[0].repeat);
        assert!(app.dyn_panel.open_items.contains(&(3, 0)));
        // The creator's own sliders: a moved start slider is kept, an untouched one means "now".
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        app.animator_set_value(ze::CREATOR_REF, crate::animators::START, 3600.0, true);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)]);
        assert_eq!(app.world.settings_changers[1].start, 3600.0);
        app.delete_animator(1);
        // A base: the dropdown's second choice (the first is its caption) adds the first free setting at the value it has now.
        let free = app.animator_available();
        assert!(free.len() > 30, "{}", free.len());
        assert!(app.animator_dropdown_pick(DropdownTarget::AnimatorBase(0), 1));
        let base = app.world.settings_changers[0].settings_bases[0].clone();
        assert_eq!(base.key, free[0].key);
        assert!(app.world.is_animated(&base.key));
        assert!(!app.animator_available().iter().any(|entry| entry.key == base.key), "no second animator for it");
        // An animated setting cannot be edited by hand.
        let index = crate::setting_ui_data::SETTING_UI
            .iter()
            .position(|row| row.field == base.key)
            .expect("a settings row");
        let row = &crate::setting_ui_data::SETTING_UI[index];
        let before = crate::dyn_panel::read_value(&app.world.settings, row);
        app.dyn_slide(index, (0.0, 100.0), 10.0);
        assert_eq!(crate::dyn_panel::read_value(&app.world.settings, row), before);
        // The card's start slider, repeat toggle, base value (typed) and name.
        app.animator_set_value(ze::animator_ref(0), ANIM_START, 7200.0, true);
        assert_eq!(app.world.settings_changers[0].start, 7200.0);
        app.apply_ui_actions(&[UiAction::ZoneToggle(ze::animator_ref(0), ANIM_REPEAT)]);
        assert!(!app.world.settings_changers[0].repeat);
        app.apply_typed_value(FieldRef::Zone(ze::animator_ref(0), ANIM_BASE_FIRST), 0.25);
        assert_eq!(app.world.settings_changers[0].settings_bases[0].base, 0.25);
        app.zone_name_edit = Some((usize::from(ze::animator_ref(0)), "Slow".to_owned()));
        let mut enter = FrameInput::default();
        enter.enter = true;
        app.edit_zone_name(&enter);
        assert_eq!(app.world.settings_changers[0].name, "Slow");
        // The cross of the base lets the setting go.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::RemoveAnimatorBase(0, 0))]);
        assert!(app.world.settings_changers[0].settings_bases.is_empty());
        assert!(!app.world.is_animated(&base.key));
        // Delete asks first; "remove all" asks too and does nothing for an empty list.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::DeleteAnimator(0))]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::DeleteAnimator(0)));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        assert_eq!(app.world.settings_changers.len(), 1);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::RequestClearAnimators)]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::ClearAnimators));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(app.world.settings_changers.is_empty());
        assert!(!app.dyn_panel.open_items.iter().any(|item| item.0 == 3));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::RequestClearAnimators)]);
        assert!(app.confirm_dialog.is_none());
    }

    #[test]
    fn the_move_arrows_reorder_the_cards_and_an_animator_card_opens_and_closes() {
        use crate::ui::ControlCommand;
        let mut app = App::new(synthetic(151, 4), Vec2::new(1280.0, 800.0), None);
        for _ in 0..3 {
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
            app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)]);
        }
        let order = |app: &App| crate::dyn_panel::card_order(&app.dyn_panel.order, 3, app.world.settings_changers.len());
        assert_eq!(order(&app), vec![0, 1, 2]);
        // The third card goes up one place; the first cannot go up; the data keeps its order.
        app.apply_ui_actions(&[UiAction::ListMove(3, 2, true)]);
        assert_eq!(order(&app), vec![0, 2, 1]);
        app.apply_ui_actions(&[UiAction::ListMove(3, 0, true)]);
        assert_eq!(order(&app), vec![0, 2, 1]);
        app.apply_ui_actions(&[UiAction::ListMove(3, 1, false)]);
        assert_eq!(order(&app), vec![0, 2, 1], "the last card cannot go down");
        // Deleting the card in the middle of the shown order keeps the others where they are.
        app.delete_animator(2);
        assert_eq!(order(&app), vec![0, 1]);
        // The bar closes an open card and opens it again.
        assert!(app.dyn_panel.open_items.contains(&(3, 0)));
        app.apply_ui_actions(&[UiAction::ZoneOpen(3, 0)]);
        assert!(!app.dyn_panel.open_items.contains(&(3, 0)));
        app.apply_ui_actions(&[UiAction::ZoneOpen(3, 0)]);
        assert!(app.dyn_panel.open_items.contains(&(3, 0)));
        // Zones and groups reorder the same way.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddZoneGroup)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddZoneGroup)]);
        app.apply_ui_actions(&[UiAction::ListMove(1, 1, true)]);
        assert_eq!(crate::dyn_panel::card_order(&app.dyn_panel.order, 1, 2), vec![1, 0]);
        app.delete_zone_group(1);
        assert_eq!(crate::dyn_panel::card_order(&app.dyn_panel.order, 1, 1), vec![0]);
    }

    #[test]
    fn the_animator_graph_picks_drags_adds_and_removes_points() {
        use crate::ui::ControlCommand;
        use crate::zone_editor as ze;
        let mut app = App::new(synthetic(151, 4), Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenAnimatorCreator)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SubmitAnimatorCreator)]);
        let scale = crate::ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        // The plot's rectangle in reference px, and the screen position of a value in it.
        let rect = [100.0, 100.0, 931.0, 420.0];
        let size = [rect[2] - rect[0], rect[3] - rect[1]];
        // (The graph's frame is kept while the curve is edited and made again when a point leaves it: ask for the current one each time.)
        let screen = |app: &mut App, value: [f32; 2]| -> Vec2 {
            let view = app.animator_graph_view(0).expect("a frame");
            let at = view.to_rect(value, size);
            Vec2::new((rect[0] + at[0]) * scale, (rect[3] - at[1]) * scale)
        };
        let second = app.world.settings_changers[0].points[1];
        // A press on the second point picks it and starts a drag; the drag moves it (time and value) and the curve follows.
        let at = screen(&mut app, [second.x, second.y]);
        app.animator_graph_press(0, rect, at, false);
        assert_eq!(app.animator.point, Some((0, 1)));
        assert!(app.animator.drag.is_some());
        let at = screen(&mut app, [20_000.0, 1.8]);
        app.animator_graph_drag(at);
        let moved = app.world.settings_changers[0].points[1];
        assert!((moved.x - 20_000.0).abs() < 200.0 && (moved.y - 1.8).abs() < 0.02, "{moved:?}");
        app.animator_graph_release();
        assert!(app.animator.drag.is_none());
        // The point panel's typed time moves the selected point; the easing slider and the interpolation dropdown change it.
        app.animator_set_value(ze::animator_ref(0), ze::ANIM_POINT_TIME, 18_000.0, false);
        assert!((app.world.settings_changers[0].points[1].x - 18_000.0).abs() < 1.0);
        assert!(app.animator_dropdown_pick(crate::ui::DropdownTarget::AnimatorInterpolation(0), 3));
        assert_eq!(
            app.world.settings_changers[0].points[1].interpolation,
            bibites_core::settings::InterpolationType::EaseInOut
        );
        app.animator_set_value(ze::animator_ref(0), ze::ANIM_POINT_EASE, 4.0, false);
        assert_eq!(app.world.settings_changers[0].points[1].ease_factor, 4.0);
        // Ctrl + a press on empty ground adds a point (and selects it); Ctrl + a press on a point removes it.
        let at = screen(&mut app, [8_000.0, 0.4]);
        app.animator_graph_press(0, rect, at, true);
        assert_eq!(app.world.settings_changers[0].points.len(), 3);
        assert_eq!(app.animator.point, Some((0, 1)));
        let added = app.world.settings_changers[0].points[1];
        let at = screen(&mut app, [added.x, added.y]);
        app.animator_graph_press(0, rect, at, true);
        assert_eq!(app.world.settings_changers[0].points.len(), 2);
        // The first point and the last two stay.
        let first = app.world.settings_changers[0].points[0];
        let at = screen(&mut app, [first.x, first.y]);
        app.animator_graph_press(0, rect, at, true);
        assert_eq!(app.world.settings_changers[0].points.len(), 2);
        // A press on empty ground deselects.
        let at = screen(&mut app, [60_000.0, 0.2]);
        app.animator_graph_press(0, rect, at, false);
        assert_eq!(app.animator.point, None);
    }

    #[test]
    fn the_zone_groups_list_adds_edits_rolls_and_deletes_groups() {
        use crate::ui::ControlCommand;
        use crate::zone_editor::{self as ze, GROUP_COUNT, GROUP_SCALE};
        let mut app = App::new(synthetic(139, 4), Vec2::new(1280.0, 800.0), None);
        let zones = app.world.zones.zones.len();
        let groups = app.world.zone_groups.len();
        // `+`: "Pockets<n>" with ten zones, its card open.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::AddZoneGroup)]);
        assert_eq!(app.world.zone_groups.len(), groups + 1);
        assert_eq!(app.world.zone_groups[groups].name, format!("Pockets{groups}"));
        assert_eq!(app.world.zones.zones.len(), zones + 10);
        assert!(app.dyn_panel.open_items.contains(&(1, groups as u16)));
        // The count slider's typed value regenerates the zones (a group is addressed by `group_ref`).
        let group = groups;
        app.apply_typed_value(crate::dyn_panel::FieldRef::Zone(ze::group_ref(group), GROUP_COUNT), 4.0);
        assert_eq!(app.world.zone_groups[group].zone_count, 4);
        assert_eq!(app.world.zones.zones.len(), zones + 4);
        // "Count scales with sim" keeps the zones' number and swaps the slider.
        app.apply_ui_actions(&[UiAction::ZoneToggle(ze::group_ref(group), GROUP_SCALE)]);
        assert!(app.world.zone_groups[group].scale_with_sim);
        assert_eq!(app.world.zones.zones.len(), zones + 4);
        // A template field reaches the group's zones.
        app.edit_zone(usize::from(ze::template_ref(group)), |zone, _| zone.fertility = 7.0);
        let first = app.world.zones.group_span(group as u32).0;
        assert_eq!(app.world.zones.zones[first].config.fertility, 7.0);
        // The name is typed through the same field as a zone's.
        app.apply_ui_actions(&[UiAction::ZoneNameEdit(ze::group_ref(group))]);
        assert_eq!(app.zone_name_edit.as_ref().map(|(_, text)| text.as_str()), Some(format!("Pockets{groups}").as_str()));
        app.zone_name_edit = Some((usize::from(ze::group_ref(group)), "Lakes".to_owned()));
        let mut enter = FrameInput::default();
        enter.enter = true;
        app.edit_zone_name(&enter);
        assert_eq!(app.world.zone_groups[group].name, "Lakes");
        assert_eq!(app.world.zones.zones[first].config.name, "Lakes_0");
        // The reroll button moves the zones; the bar closes the card; "remove all" does nothing.
        let before = app.world.zones.zones[first].config.pos_x;
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::RerollZoneGroup(group as u16))]);
        assert_ne!(app.world.zones.zones[first].config.pos_x, before);
        app.apply_ui_actions(&[UiAction::ZoneOpen(1, group as u16)]);
        assert!(!app.dyn_panel.open_items.contains(&(1, group as u16)));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ClearZoneGroups)]);
        assert_eq!(app.world.zone_groups.len(), groups + 1);
        // Delete: the group and its zones go at once.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::DeleteZoneGroup(group as u16))]);
        assert_eq!(app.world.zone_groups.len(), groups);
        assert_eq!(app.world.zones.zones.len(), zones);
    }
}

#[cfg(test)]
mod simulation_thread_tests {
    use super::*;
    use bibites_core::sim::state_fingerprint;

    fn synthetic(seed: u64, count: usize) -> LoadedWorld {
        LoadedWorld::load(None, seed, count, 1).expect("starter world")
    }
    // -----------------------------------------------------------------------
    // The simulation thread (`--sim-thread`)
    // -----------------------------------------------------------------------

    /// One 60 Hz frame.
    const FRAME: f32 = 1.0 / 60.0;

    /// Waits (twenty seconds at the most) until `done` holds.
    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let started = std::time::Instant::now();
        while !done() {
            assert!(
                started.elapsed().as_secs() < 20,
                "timed out waiting for {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// An app on a small generated world with a simulation thread that is ready, at ten times the speed so that the thread
    /// has ticks to show for a short wait.
    fn threaded_app(seed: u64) -> App {
        let mut app = App::new(synthetic(seed, 24), Vec2::new(1280.0, 800.0), None);
        app.prefs.min_fps = 0;
        app.enable_sim_thread();
        wait_until("the simulation thread to start", || {
            app.sim.as_mut().is_some_and(|sim| sim.is_ready())
        });
        app.set_target_speed(10.0);
        app
    }

    /// One frame of the window loop, without the UI: where the world is, the input, the simulation, the display list.
    fn step_frame(app: &mut App, input: &FrameInput) {
        app.frame_world(input);
        app.handle_input(input);
        app.settle_world();
        app.advance(FRAME);
        app.rebuild_scene();
    }

    /// Frames that only watch, up to the one that sends the world away.
    fn watch_until_away(app: &mut App) {
        watch_until_away_with(app, &FrameInput::default());
    }

    /// The same with `input` in every frame (the pointer resting somewhere, say).
    fn watch_until_away_with(app: &mut App, input: &FrameInput) {
        for frame in 1..=LIGHT_FRAMES_BEFORE_AWAY {
            assert!(!app.world_away(), "away after {} frames", frame - 1);
            step_frame(app, input);
        }
        assert!(
            app.world_away(),
            "still home after {LIGHT_FRAMES_BEFORE_AWAY} frames of watching"
        );
    }

    fn worker_ticks(app: &App) -> u64 {
        app.sim.as_ref().map_or(0, |sim| sim.ticks())
    }

    #[test]
    fn every_input_is_either_a_camera_input_or_needs_the_world() {
        // Adding a field to `FrameInput` stops this from compiling: decide in `App::light_input` whether the new input can be
        // handled while the world is on the simulation thread (the camera and the toggles of what is drawn can; whatever
        // reads or changes bibites, pellets, panels or the pause cannot), and put it in the right list below.
        let FrameInput {
            cursor: _,
            primary_down: _,
            primary_pressed: _,
            primary_released: _,
            wheel: _,
            toggle_pause: _,
            step: _,
            fit: _,
            fit_bibites: _,
            reset_view: _,
            digit: _,
            digit1_down: _,
            toggle_grid: _,
            toggle_zones: _,
            toggle_pellets: _,
            toggle_pheromones: _,
            toggle_hud: _,
            toggle_objects: _,
            quit: _,
            select_random_bibite: _,
            select_high_generation: _,
            select_oldest: _,
            select_random_egg: _,
            select_random_zone: _,
            select_autopanner: _,
            quick_save: _,
            brain_sort_keys: _,
            brain_hidden_down: _,
            right_ctrl_down: _,
            lay_egg: _,
            kill_target: _,
            camera_home: _,
            zoom_key: _,
            secondary_down: _,
            ctrl_down: _,
            shift_down: _,
            middle_down: _,
            middle_pressed: _,
            move_axis: _,
            speed_scale_step: _,
            undo: _,
            redo: _,
            copy_tag: _,
            paste_tag: _,
            backspace: _,
            enter: _,
            dt: _,
            key_left: _,
            key_right: _,
            key_home: _,
            key_end: _,
            key_delete: _,
            select_all_key: _,
        } = FrameInput::default();

        let idle = FrameInput::default();
        assert!(App::light_input(&idle));
        let camera = [
            FrameInput {
                cursor: Vec2::new(300.0, 200.0),
                ..idle
            },
            FrameInput { wheel: 1.0, ..idle },
            FrameInput {
                zoom_key: 0.25,
                ..idle
            },
            FrameInput {
                move_axis: Vec2::new(1.0, -1.0),
                ..idle
            },
            FrameInput {
                secondary_down: true,
                ctrl_down: true,
                ..idle
            },
            FrameInput {
                shift_down: true,
                ..idle
            },
            FrameInput {
                middle_down: true,
                middle_pressed: true,
                ..idle
            },
            FrameInput {
                speed_scale_step: 1,
                ..idle
            },
            FrameInput {
                right_ctrl_down: true,
                ..idle
            },
            // Held key 1 only steps the Bibite Editor's brain simulator (the editor is a panel: the world is home then).
            FrameInput {
                digit1_down: true,
                ..idle
            },
            FrameInput {
                toggle_grid: true,
                toggle_zones: true,
                ..idle
            },
            FrameInput {
                toggle_pellets: true,
                toggle_pheromones: true,
                toggle_hud: true,
                ..idle
            },
            // RUSTibites Improved: the Hide Objects hotkey only changes what is drawn (the simulation thread builds its scene with the option).
            FrameInput {
                toggle_objects: true,
                ..idle
            },
            FrameInput { dt: FRAME, ..idle },
        ];
        for input in &camera {
            assert!(App::light_input(input), "{input:?} is a camera input");
        }
        let needs_the_world = [
            FrameInput {
                primary_down: true,
                ..idle
            },
            FrameInput {
                primary_pressed: true,
                ..idle
            },
            FrameInput {
                primary_released: true,
                ..idle
            },
            FrameInput {
                toggle_pause: true,
                ..idle
            },
            FrameInput { step: true, ..idle },
            FrameInput { fit: true, ..idle },
            FrameInput {
                fit_bibites: true,
                ..idle
            },
            FrameInput {
                reset_view: true,
                ..idle
            },
            FrameInput {
                digit: Some(1),
                ..idle
            },
            FrameInput { quit: true, ..idle },
            FrameInput {
                select_random_bibite: true,
                ..idle
            },
            FrameInput {
                select_high_generation: true,
                ..idle
            },
            FrameInput {
                select_oldest: true,
                ..idle
            },
            FrameInput {
                select_random_egg: true,
                ..idle
            },
            FrameInput {
                select_random_zone: true,
                ..idle
            },
            FrameInput {
                select_autopanner: true,
                ..idle
            },
            FrameInput {
                quick_save: true,
                ..idle
            },
            FrameInput {
                brain_sort_keys: [true, false, false],
                ..idle
            },
            FrameInput {
                brain_sort_keys: [false, false, true],
                ..idle
            },
            FrameInput {
                brain_hidden_down: true,
                ..idle
            },
            FrameInput {
                lay_egg: true,
                ..idle
            },
            FrameInput {
                kill_target: true,
                ..idle
            },
            FrameInput {
                camera_home: true,
                ..idle
            },
            FrameInput { undo: true, ..idle },
            FrameInput { redo: true, ..idle },
            FrameInput {
                copy_tag: true,
                ..idle
            },
            FrameInput {
                paste_tag: true,
                ..idle
            },
            FrameInput {
                backspace: true,
                ..idle
            },
            FrameInput {
                enter: true,
                ..idle
            },
        ];
        for input in &needs_the_world {
            assert!(!App::light_input(input), "{input:?} needs the world");
        }
    }

    #[test]
    fn a_watching_app_hands_its_world_to_the_thread_after_a_few_frames() {
        let mut app = threaded_app(9_201);
        // The world ticks at a tenth of the speed in these frames, so that the numbers below are those of the hand-over.
        app.set_target_speed(MIN_SPEED);
        let fixed = app.world.settings.fixed_dt();
        let idle = FrameInput::default();
        for frame in 1..LIGHT_FRAMES_BEFORE_AWAY {
            step_frame(&mut app, &idle);
            assert!(!app.world_away(), "away after {frame} frames of watching");
        }
        let (pellets, bibites) = (app.world.pellets.len(), app.world.bibites.len());
        let (time, tick) = (app.world.simulated_time(), app.world.clock.tick());
        assert!(bibites > 0 && pellets > 0);
        app.frame_world(&idle);
        assert!(app.world_away());

        // What stands in for the world has its settings and none of its content.
        assert_eq!(app.world.bibites.len(), 0);
        assert_eq!(app.world.pellets.len(), 0);
        assert_eq!(app.world.settings.fixed_dt(), fixed);
        // Until the thread publishes, the window has the numbers of the world as it left, not the husk's.
        assert_eq!(app.frame_counts(), (pellets, bibites));
        assert_eq!(app.simulated_time(), time);
        assert_eq!(app.world_tick(), tick);
        let snapshot = app.snapshot.as_ref().expect("a first snapshot");
        assert!(std::ptr::eq(app.drawn_scene(), &snapshot.scene));
        assert!(!snapshot.scene.primitives.is_empty());

        // The display list of an app whose world is away is the thread's: the app builds nothing.
        let before = app.scene.primitives.len();
        app.rebuild_scene();
        assert_eq!(app.scene.primitives.len(), before);
    }

    #[test]
    fn the_window_takes_what_the_thread_publishes() {
        let mut app = threaded_app(9_202);
        watch_until_away(&mut app);
        let (time, tick, counted) = (app.simulated_time(), app.world_tick(), app.sim_ticks);
        let idle = FrameInput::default();
        // Frames go on, and the numbers of the world with them.
        wait_until("the thread's snapshots to show a later tick", || {
            step_frame(&mut app, &idle);
            app.world_tick() > tick + 5
        });
        assert!(app.simulated_time() > time);
        let snapshot = app.snapshot.as_ref().expect("a snapshot");
        assert!(snapshot.seq > 0, "the seeded snapshot has been replaced");
        assert!(std::ptr::eq(app.drawn_scene(), &snapshot.scene));
        assert_eq!(app.hud, snapshot.hud);
        assert_eq!(app.frame_counts(), (snapshot.pellets, snapshot.bibites));
        // The ticks the thread ran are counted, for the time panel and the autosave.
        assert!(app.sim_ticks > counted, "{} ticks counted", app.sim_ticks);
    }

    #[test]
    fn a_frame_that_needs_the_world_takes_it_back() {
        for case in [
            "click",
            "hotkey",
            "typed key",
            "pause",
            "panel",
            "selection",
            "toolbar panel",
            "lineage popup",
            "tool",
            "autosave due",
        ] {
            let mut app = threaded_app(9_203);
            let bibite = app
                .world
                .bibites
                .iter()
                .next()
                .map(|(id, _)| id)
                .expect("a bibite");
            let bibites = app.world.bibites.len();
            watch_until_away(&mut app);
            wait_until("the thread to tick", || worker_ticks(&app) >= 3);
            let mut input = FrameInput::default();
            match case {
                "click" => input.primary_pressed = true,
                "hotkey" => input.select_random_bibite = true,
                "typed key" => input.enter = true,
                "pause" => app.paused = true,
                "panel" => app.panels.open(crate::ui::PanelKind::Escape),
                "selection" => app.selected = Some(bibite),
                "toolbar panel" => app.toolbar[0].show(),
                "lineage popup" => app.lineage_gate_popup = true,
                "tool" => app.tool = crate::ui::ToolKind::Pellet,
                "autosave due" => app.world_wanted = true,
                other => unreachable!("{other}"),
            }
            app.frame_world(&input);
            assert!(!app.world_away(), "{case}: the world stayed away");
            assert_eq!(app.world.bibites.len(), bibites, "{case}");
            assert!(
                app.world.clock.tick() >= 3,
                "{case}: the world that came back is the one that ticked ({})",
                app.world.clock.tick()
            );
            assert!(app.snapshot.is_none() && !app.world_wanted, "{case}");
            assert!(std::ptr::eq(app.drawn_scene(), &app.scene), "{case}");
        }
    }

    #[test]
    fn something_that_opens_during_the_frame_takes_the_world_back_before_the_frame_is_simulated() {
        let mut app = threaded_app(9_204);
        watch_until_away(&mut app);
        // The hover panel of a toolbar button opens when the pointer has rested on it (the UI routing runs after
        // `frame_world`, before the simulation).
        app.toolbar[1].show();
        app.settle_world();
        assert!(!app.world_away());
        assert!(!app.world.bibites.is_empty());
        // Nothing to settle while the app only watches.
        app.toolbar[1] = HoverPanel::default();
        watch_until_away(&mut app);
        app.settle_world();
        assert!(app.world_away());
    }

    #[test]
    fn a_resize_takes_the_world_back_only_when_the_view_is_fitted_to_it() {
        let mut fitted = threaded_app(9_205);
        assert_eq!(fitted.view_mode, ViewMode::Bibites);
        watch_until_away(&mut fitted);
        fitted.set_viewport(900.0, 700.0);
        assert!(!fitted.world_away(), "fitting the view reads the bibites");
        assert_eq!(fitted.camera.screen, Vec2::new(900.0, 700.0));

        let mut manual = threaded_app(9_205);
        manual.view_mode = ViewMode::Manual;
        watch_until_away(&mut manual);
        manual.set_viewport(900.0, 700.0);
        assert!(
            manual.world_away(),
            "a framing of the player's own needs no bibites"
        );
        assert_eq!(manual.camera.screen, Vec2::new(900.0, 700.0));
    }

    #[test]
    fn a_taller_window_keeps_the_orthographic_size() {
        // `Camera.orthographicSize` is half the window's height in world units and the window's size does not change it: fullscreen (or a resize)
        // shows the same part of the world, bigger. The zoom is pixels per unit, so it follows the height.
        let mut app = threaded_app(9_207);
        app.set_viewport(1280.0, 720.0);
        app.set_ortho_size(30.0, 720.0);
        assert!((app.camera.zoom - 12.0).abs() < 1e-4);
        app.set_viewport(1920.0, 1080.0);
        assert_eq!(app.view_mode, ViewMode::Manual);
        assert!((app.camera.zoom - 18.0).abs() < 1e-4, "720 px for 60 units is 12, 1080 px is 18 ({})", app.camera.zoom);
        assert!((app.camera.screen.y / (2.0 * app.camera.zoom) - 30.0).abs() < 1e-3, "still 30 units to the edge");
        // Only the height counts: a wider window shows more to the sides.
        app.set_viewport(2400.0, 1080.0);
        assert!((app.camera.zoom - 18.0).abs() < 1e-4);
    }

    #[test]
    fn a_world_watched_on_the_thread_is_the_world_run_headless() {
        let mut app = threaded_app(9_206);
        let first = app.world.clock.tick();
        watch_until_away(&mut app);
        wait_until("forty ticks on the thread", || worker_ticks(&app) >= 40);
        app.advance(FRAME);
        app.ensure_world_home();
        assert!(!app.world_away());
        let ticks = app.sim_ticks;
        assert!(ticks >= 40);
        assert_eq!(
            app.world.clock.tick(),
            first + ticks,
            "every tick of the world, inline or on the thread, is counted once"
        );
        let mut headless = synthetic(9_206, 24).world;
        headless.run(ticks);
        assert_eq!(
            state_fingerprint(&app.world),
            state_fingerprint(&headless),
            "a world that went to the thread and back diverged from headless execution"
        );
    }

    #[test]
    fn the_autopanner_makes_up_for_the_ticks_the_world_ran_away() {
        let mut app = threaded_app(9_207);
        watch_until_away(&mut app);
        wait_until("forty ticks on the thread", || worker_ticks(&app) >= 40);
        app.ensure_world_home();
        let ticks = app.sim_ticks;

        // An app that made every tick itself.
        let mut plain = App::new(synthetic(9_207, 24), Vec2::new(1280.0, 800.0), None);
        let fixed = plain.world.settings.fixed_dt();
        for _ in 0..ticks {
            plain.world.tick();
            plain.step_autopan(fixed);
        }
        assert_ne!(
            plain.autopan,
            crate::autopanner::AutoPanner::new(crate::autopanner::MIN_WAIT),
            "the auto-panner moved"
        );
        assert_eq!(app.autopan, plain.autopan);
        assert_eq!(app.autopan_rng, plain.autopan_rng);
        assert_eq!(app.autopan_owed, 0);
    }

    #[test]
    fn the_camera_moves_the_same_whether_or_not_the_world_is_away() {
        let mut threaded = threaded_app(9_208);
        let mut plain = App::new(synthetic(9_208, 24), Vec2::new(1280.0, 800.0), None);
        plain.prefs.min_fps = 0;
        plain.set_target_speed(10.0);
        let start = plain.camera;
        for frame in 0..90 {
            let input = FrameInput {
                cursor: Vec2::new(400.0 + frame as f32, 300.0),
                wheel: if frame % 7 == 0 { 1.0 } else { 0.0 },
                zoom_key: if frame % 11 == 0 { -0.2 } else { 0.0 },
                move_axis: Vec2::new(((frame / 10) % 3) as f32 - 1.0, (frame % 2) as f32),
                secondary_down: frame % 13 < 4,
                dt: FRAME,
                ..FrameInput::default()
            };
            step_frame(&mut threaded, &input);
            step_frame(&mut plain, &input);
            assert_eq!(threaded.camera, plain.camera, "frame {frame}");
            assert_eq!(threaded.view_mode, plain.view_mode, "frame {frame}");
        }
        assert!(
            threaded.world_away(),
            "the frames were light, so the world stayed away"
        );
        assert_ne!(threaded.camera, start, "the camera was moved");
    }

    #[test]
    fn what_the_player_toggles_is_drawn_by_the_thread() {
        let mut app = threaded_app(9_209);
        watch_until_away(&mut app);
        let idle = FrameInput::default();
        wait_until("a snapshot of the thread", || {
            step_frame(&mut app, &idle);
            app.snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.seq > 0)
        });
        step_frame(
            &mut app,
            &FrameInput {
                toggle_pellets: true,
                ..idle
            },
        );
        assert!(app.world_away(), "a toggle of what is drawn needs no world");
        assert!(!app.scene_options.pellets);
        // The thread is asked to leave them out (what it draws is checked in `sim_thread`).
        let request = app
            .sim
            .as_ref()
            .and_then(|sim| sim.request())
            .expect("a request");
        assert!(!request.options.pellets);
        assert!(app.world_away());
    }

    #[test]
    fn the_hide_objects_hotkey_needs_no_world_and_the_thread_then_draws_none_of_them() {
        use crate::render::Primitive;
        let mut app = threaded_app(9_231);
        watch_until_away(&mut app);
        let idle = FrameInput::default();
        let objects_in_the_picture = |app: &App| {
            app.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .scene
                    .primitives
                    .iter()
                    .any(|primitive| matches!(primitive, Primitive::Capsule { .. } | Primitive::Sprite { .. } | Primitive::Dots { count: 1.., .. }))
            })
        };
        wait_until("a snapshot of the thread with the bibites in it", || {
            step_frame(&mut app, &idle);
            objects_in_the_picture(&app)
        });
        let seen = app.snapshot.as_ref().map_or(0, |snapshot| snapshot.seq);
        step_frame(&mut app, &FrameInput { toggle_objects: true, ..idle });
        assert!(app.world_away(), "the hotkey changes what is drawn and needs no world");
        assert!(!app.objects_shown());
        assert!(!app.sim.as_ref().and_then(|sim| sim.request()).expect("a request").options.objects, "the thread is asked to leave them out");
        wait_until("a snapshot drawn without the objects", || {
            step_frame(&mut app, &idle);
            app.snapshot.as_ref().is_some_and(|snapshot| snapshot.seq > seen) && !objects_in_the_picture(&app)
        });
        assert!(app.world_away(), "all of it with the world away");
        // And back.
        step_frame(&mut app, &FrameInput { toggle_objects: true, ..idle });
        wait_until("a snapshot with the objects again", || {
            step_frame(&mut app, &idle);
            objects_in_the_picture(&app)
        });
    }

    #[test]
    fn the_pointer_resting_on_a_pellet_shows_its_tooltip_while_the_world_is_away() {
        use crate::tooltip::Target;
        use bibites_core::pellet::{MaterialKind, PelletSpawn};
        let mut app = threaded_app(9_214);
        // A pellet away from every body.
        let mut spot = app.camera.center + Vec2::new(30.0, 20.0);
        for _ in 0..200 {
            if crate::selection::entity_at(&app.world, spot).is_none() {
                break;
            }
            spot += Vec2::new(25.0, 0.0);
        }
        let id = app
            .world
            .spawn_pellet(PelletSpawn::new(MaterialKind::Plant, 40.0, spot))
            .expect("pellet");
        let body = crate::tooltip::pellet_body(&app.world, id).expect("a box");
        let screen = app.camera.world_to_screen(spot);
        let resting = FrameInput {
            cursor: screen,
            dt: FRAME,
            ..FrameInput::default()
        };
        watch_until_away_with(&mut app, &resting);
        let none = crate::ui::UiFrame::new(1.0);
        // The pointer has to rest for half a second (real time) with no UI under it; the thread answers for the point.
        wait_until("the pellet's tooltip", || {
            step_frame(&mut app, &resting);
            app.update_tooltip(&none, screen, false, 0.1);
            matches!(app.tooltip.shown(), Some((Target::Pellet(shown), _)) if shown == id)
        });
        assert!(
            app.world_away(),
            "a resting pointer is no reason to bring the world back"
        );
        // The box says what the pellet's box says.
        let shown = app.pellet_tooltip_body(id).expect("the thread's text");
        assert_eq!(shown.header, body.header);
        assert_eq!(shown.lines.len(), body.lines.len());
        // Moving off the pellet hides it, as does UI under the pointer.
        app.update_tooltip(&none, screen + Vec2::new(300.0, 0.0), false, 0.1);
        assert!(app.tooltip.shown().is_none());
        step_frame(&mut app, &resting);
        app.update_tooltip(&none, screen, true, 0.1);
        assert!(app.tooltip.shown().is_none());
    }

    #[test]
    fn an_autosave_that_falls_due_while_the_world_is_away_brings_it_back_and_is_written() {
        let dir = std::env::temp_dir().join(format!(
            "rustibites-simthread-autosave-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = threaded_app(9_210);
        app.storage = StoragePaths::new(dir.clone());
        app.prefs.autosave = true;
        app.prefs.autosave_real_time = true;
        app.prefs.autosave_period_minutes = 1;
        app.prefs.autosave_max_files = 5;
        watch_until_away(&mut app);
        wait_until("a tick on the thread", || worker_ticks(&app) >= 1);
        // Due while the world is away: nothing is written yet, the next frame wants the world.
        app.update_autosave(61.0, 0.0);
        assert!(!app.autosave_in_progress());
        assert!(app.world_wanted);
        let idle = FrameInput::default();
        app.frame_world(&idle);
        assert!(!app.world_away() && !app.world_wanted);
        assert!(app.world.clock.tick() >= 1);
        // Home again: the same call writes it.
        app.update_autosave(0.0, 0.0);
        wait_until("the autosave to be written", || {
            app.update_autosave(0.0, 0.0);
            !app.autosave_in_progress()
        });
        assert!(
            app.last_action.starts_with("autosaved"),
            "{}",
            app.last_action
        );
        let written = std::fs::read_dir(app.storage.autosaves_dir())
            .expect("the autosave folder")
            .filter_map(Result::ok)
            .count();
        assert_eq!(written, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_frame_that_needs_the_world_starts_the_count_again() {
        let mut app = threaded_app(9_212);
        let idle = FrameInput::default();
        for _ in 1..LIGHT_FRAMES_BEFORE_AWAY {
            step_frame(&mut app, &idle);
        }
        // `F` in the middle of the watching.
        step_frame(&mut app, &FrameInput { fit: true, ..idle });
        for frame in 1..LIGHT_FRAMES_BEFORE_AWAY {
            step_frame(&mut app, &idle);
            assert!(!app.world_away(), "away after {frame} frames since the key");
        }
        step_frame(&mut app, &idle);
        assert!(app.world_away());
    }

    #[test]
    fn the_speed_the_app_wants_is_the_speed_the_thread_runs_at() {
        let mut app = threaded_app(9_213);
        watch_until_away(&mut app);
        let idle = FrameInput::default();
        let rate = |app: &mut App| {
            let before = worker_ticks(app);
            let started = std::time::Instant::now();
            while started.elapsed() < std::time::Duration::from_millis(300) {
                step_frame(app, &idle);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            (worker_ticks(app) - before) as f64 / started.elapsed().as_secs_f64()
        };
        let fast = rate(&mut app);
        // The minimum-FPS governor slows the engine down to a tenth: the thread follows within a frame.
        app.engine_fraction = 0.1;
        let slow = rate(&mut app);
        assert!(app.world_away());
        assert!(
            slow < fast / 3.0,
            "{slow} ticks a second after the governor slowed the engine, {fast} before"
        );
    }

    #[test]
    fn leaving_the_app_with_the_world_away_ends_the_thread() {
        let mut app = threaded_app(9_211);
        watch_until_away(&mut app);
        wait_until("a tick on the thread", || worker_ticks(&app) >= 1);
        // `Drop` joins the worker; a hang here is a worker that does not look at its stop flag.
        drop(app);
    }
}
