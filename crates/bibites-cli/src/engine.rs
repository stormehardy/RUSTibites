//! Engine wiring for the CLI.
//!
//! Everything simulation-related lives in `bibites-core`; file formats live in
//! `bibites-io`. This module is the only place that knows both crates and wires
//! them to the command-line arguments.

use std::time::Instant;

use bibites_core::body::{BibiteSpawn, SpawnVitals};
use bibites_core::brain::Brain;
use bibites_core::genome::{Genome, GenomeMeta};
use bibites_core::math::Vec2;
use bibites_core::settings::SimSettings;
use bibites_core::world::{ScenarioData, TemplateSource, World};

use crate::args::{BenchArgs, CalibrateArgs, RunArgs, StatsFormat};
use crate::report::RunReport;

/// Runs a world/scenario/genome and prints statistics.
pub fn run(args: &RunArgs) -> Result<(), String> {
    let input = &args.input;
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let settings_override = match &args.settings {
        Some(path) => Some(bibites_io::load_settings_file(path).map_err(|error| {
            format!(
                "could not load settings override {}: {error}",
                path.display()
            )
        })?),
        None => None,
    };

    let mut prepared = if extension == "zip" {
        prepare_zip(
            input,
            args.seed,
            settings_override.as_ref(),
            args.templates.as_deref(),
        )?
    } else if extension == "bb8" || extension == "bb8template" {
        prepare_genome(input, args.seed)?
    } else {
        return Err(format!(
            "`run` supports scenarios/saves (.zip) and genomes (.bb8/.bb8template); got {}",
            input.display()
        ));
    };

    // Propagates the thread count to the sensing pool and the brain-tick path.
    prepared.world.set_threads(args.threads);

    // Restore the checkpoint sidecar (if any) so resumed runs are bit-identical
    // to a single long process. Scenario zips have no sidecar and are untouched.
    if extension == "zip" && !args.no_state {
        if let Some(snapshot) = bibites_io::read_checkpoint_state_checked(input, args.seed)
            .map_err(|error| {
                format!(
                    "could not load checkpoint state for {}: {error}",
                    input.display()
                )
            })?
        {
            prepared
                .world
                .restore_checkpoint_state(&snapshot)
                .map_err(|error| format!("could not restore checkpoint state: {error}"))?;
        }
    }

    if !args.quiet {
        eprintln!(
            "prepared {} ({} bibites, {} zones, {} plants) at {:.0} TPS with {} thread(s)",
            prepared.label,
            prepared.spawned,
            prepared.world.zones.zones.len(),
            prepared.world.pellets.ids().count(),
            prepared.world.settings.sim_tps,
            prepared.world.threads()
        );
    }

    let start = Instant::now();
    if let Some(capture_dir) = &args.capture {
        crate::capture::write_world_capture(
            &mut prepared.world,
            capture_dir,
            &args.input,
            args.seed,
            args.ticks,
            &[0, args.ticks],
        )?;
    } else {
        prepared.world.run(args.ticks);
    }
    let wall_secs = start.elapsed().as_secs_f64();
    let state_fingerprint = bibites_core::sim::state_fingerprint(&prepared.world);
    let stats = prepared.world.sample_stats();

    let report = RunReport {
        label: prepared.label.clone(),
        ticks: args.ticks,
        bibites_start: prepared.spawned,
        bibites_end: stats.counts.bibites as usize,
        wall_secs,
        sim_secs: args.ticks as f64 * f64::from(prepared.world.settings.fixed_dt()),
        threads: prepared.world.threads(),
        peak_bytes: None,
        state_fingerprint: Some(state_fingerprint),
    };

    match args.stats {
        StatsFormat::Human => {
            println!("{}", report.summary());
            println!(
                "final: {} bibites, {} eggs, {} corpses, {} plant pellets, {} meat pellets, species={}",
                stats.counts.bibites,
                stats.counts.eggs,
                stats.counts.corpses,
                stats.pellets.plant_n,
                stats.pellets.meat_n,
                stats.species.n
            );
        }
        StatsFormat::Json => println!("{}", report.to_json()),
        StatsFormat::Csv => {
            println!("{}", RunReport::CSV_HEADER);
            println!("{}", report.to_csv());
        }
    }

    if let Some(save_path) = &args.save {
        let started = Instant::now();
        let contents = prepared.world.to_contents();
        let contents_time = started.elapsed();
        bibites_io::save_world_contents(save_path, &contents)
            .map_err(|error| format!("could not save world to {}: {error}", save_path.display()))?;
        let write_time = started.elapsed() - contents_time;
        if !args.no_state {
            let snapshot = prepared
                .world
                .capture_checkpoint_state()
                .map_err(|error| format!("could not capture checkpoint state: {error}"))?;
            bibites_io::write_checkpoint_state_with_seed(save_path, &snapshot, args.seed).map_err(
                |error| {
                    format!(
                        "could not write checkpoint state for {}: {error}",
                        save_path.display()
                    )
                },
            )?;
        }
        if !args.quiet {
            eprintln!(
                "saved world to {} (contents {:.0} ms, serialising and compressing {:.0} ms)",
                save_path.display(),
                contents_time.as_secs_f64() * 1000.0,
                write_time.as_secs_f64() * 1000.0
            );
        }
    }
    Ok(())
}

/// Runs the benchmark harness.
pub fn bench(args: &BenchArgs) -> Result<(), String> {
    let mut best: Option<RunReport> = None;
    for repetition in 0..args.repeat {
        let seed = 0x5255_5354_0000_0000 ^ u64::from(repetition);
        let prepared = match &args.scenario {
            Some(path) => prepare_zip(path, seed, None, None)?,
            None => prepare_synthetic(args.bibites, seed)?,
        };
        let mut world = prepared.world;
        // Propagates the thread count to the sensing pool and brain-tick path.
        world.set_threads(args.threads);
        // `RUSTIBITES_GATES` picks how the tick uses the pool: `scaled` (what a world starts with: it times its own ticks inline
        // and on the pool and keeps the faster, from a first guess of where the pool pays), `scaled:<percent>` (the sizes of that
        // guess times the percentage), `fixed` (the same sizes with no measuring: the pool from that size, the reference the
        // measuring is checked against), `pooled:<level>` (every world on the pool at one of the four levels of eagerness, no
        // measuring: what a level costs on this machine), `always` (every stage on the pool from one item, whatever the work: the
        // reference for what that costs a small world, = `pooled:3`), `eager` (also the resting-pellet layer from one pellet),
        // `nosleep` (`scaled` with resting pellets kept in the physics step: the reference the sleeping layer is checked against).
        if let Ok(mode) = std::env::var("RUSTIBITES_GATES") {
            match bibites_core::sim::ParallelGates::from_mode(&mode) {
                Some(gates) => world.set_parallel_gates(gates),
                None => eprintln!(
                    "warning: RUSTIBITES_GATES={mode} is not scaled, scaled:<percent>, fixed, pooled:<level>, always, eager or nosleep: ignored"
                ),
            }
        }
        world.reset_phase_timings();
        let start = Instant::now();
        world.run(args.ticks);
        let wall_secs = start.elapsed().as_secs_f64();
        let state_fingerprint = bibites_core::sim::state_fingerprint(&world);
        let stats = world.sample_stats();
        let report = RunReport {
            label: format!("{} [rep {}]", prepared.label, repetition + 1),
            ticks: args.ticks,
            bibites_start: prepared.spawned,
            bibites_end: stats.counts.bibites as usize,
            wall_secs,
            sim_secs: args.ticks as f64 * f64::from(world.settings.fixed_dt()),
            threads: world.threads(),
            peak_bytes: None,
            state_fingerprint: Some(state_fingerprint),
        };
        if args.json {
            println!("{}", report.to_json());
        } else {
            println!("{}", report.summary());
            print_phase_table(&world);
        }
        if best
            .as_ref()
            .is_none_or(|current| report.wall_secs < current.wall_secs)
        {
            best = Some(report);
        }
    }
    if !args.json {
        if let Some(best) = best {
            println!("best: {}", best.summary());
        }
    }
    Ok(())
}

/// Calibrates the machine (see `bibites_core::calibration`): how many workers a tick should use, from what size of world the pool of workers pays, and at which
/// level; prints the numbers it found and, with `--save` or `--out`, keeps them.
pub fn calibrate(args: &CalibrateArgs) -> Result<(), String> {
    use bibites_core::calibration;
    let threads = if args.threads == 0 { calibration::available_threads() } else { args.threads };
    let config = if args.quick { calibration::Config::quick(threads) } else { calibration::Config::full(threads) };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    println!("calibrating this machine for {threads} threads ({} test) ...", if args.quick { "quick" } else { "full" });
    let result = calibration::run(&config, &cancel, &mut |progress| {
        eprintln!("[{}/{}] {}", progress.done + 1, progress.total, progress.what);
    })
    .map_err(|stop| format!("the calibration did not finish: {stop:?}"))?;
    println!("took {:.1} s; noise between blocks of the same way: {:.0} %{}", result.seconds, result.noise * 100.0, if result.noisy() { " (the machine was busy: run it again)" } else { "" });
    if !result.worker_rows.is_empty() {
        println!("\nworkers ({} bibites; the fastest level at each number of workers), ms per tick:", config.workers_bibites);
        for row in &result.worker_rows {
            println!("  {:>3} workers: {:>8.3}  (level {})", row.workers, row.ns / 1e6, row.level);
        }
    }
    if !result.rows.is_empty() {
        println!("\nsizes with {} workers, ms per tick:", result.workers);
        println!("  {:>8} {:>8} {:>9} {:>9} {:>9} {:>9} {:>9}   best way", "bibites", "bodies", "inline", "level 0", "level 1", "level 2", "level 3");
        for row in &result.rows {
            let (level, best) = row.best_pooled();
            let way = if best < row.ns[0] { format!("level {level}, {:.2}x", row.ns[0] / best) } else { "inline".to_owned() };
            println!(
                "  {:>8} {:>8} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3}   {way}",
                row.bibites, row.bodies, row.ns[0] / 1e6, row.ns[1] / 1e6, row.ns[2] / 1e6, row.ns[3] / 1e6, row.ns[4] / 1e6
            );
        }
    }
    println!(
        "\nresult: {} workers; the pool pays from about {} bibites; a world that big starts at level {}; up to {:.2}x faster than one thread at the biggest size measured",
        result.workers, result.pool_bibites, result.start_level, result.speedup
    );
    let text = result.to_text();
    let mut targets: Vec<std::path::PathBuf> = args.out.iter().cloned().collect();
    if args.save {
        targets.push(bibites_io::storage::StoragePaths::default_local().root.join("calibration.txt"));
    }
    for path in targets {
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, &text).map_err(|error| format!("could not write {}: {error}", path.display()))?;
        println!("saved: {}", path.display());
    }
    Ok(())
}

/// A world prepared for a run/benchmark.
struct PreparedWorld {
    world: World,
    spawned: usize,
    label: String,
}

/// Builds a world from a scenario or save zip.
fn prepare_zip(
    path: &std::path::Path,
    seed: u64,
    settings_override: Option<&SimSettings>,
    templates_dir: Option<&std::path::Path>,
) -> Result<PreparedWorld, String> {
    match prepare_scenario(path, seed, settings_override, templates_dir) {
        Ok(prepared) => Ok(prepared),
        Err(scenario_error) => {
            prepare_save(path, seed, settings_override, templates_dir).map_err(|save_error| {
                format!("not a scenario ({scenario_error}) and not a save ({save_error})")
            })
        }
    }
}

/// Builds a world from a save zip.
fn prepare_save(
    path: &std::path::Path,
    seed: u64,
    settings_override: Option<&SimSettings>,
    templates_dir: Option<&std::path::Path>,
) -> Result<PreparedWorld, String> {
    let mut contents = bibites_io::load_world_contents(path, settings_override)
        .map_err(|error| error.to_string())?;
    contents.seed = seed;
    let label = format!("save {} ({})", contents.scene.name, contents.scene.version);
    let spawned = contents.bibites.len();
    let mut world = World::from_contents(contents)
        .map_err(|error| format!("could not load world contents: {error}"))?;

    // A resumed save can still have an active fresh-scenario spawner (mid-spawn
    // checkpoint). The spawner resolves rows by template name, so load the same
    // external templates the scenario path uses before running.
    if let Some(dir) = templates_dir
        .map(std::path::Path::to_path_buf)
        .or_else(default_templates_dir)
    {
        let templates = load_template_dir(&dir)?;
        if !templates.is_empty() {
            eprintln!(
                "loaded {} template(s) from {}",
                templates.len(),
                dir.display()
            );
        }
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
        // `SaveSystem.LoadWorld` -> `ResumeSpawners`: a loaded world runs its default-bibite spawner (unless a checkpoint brought the spawner's own state).
        if !world.fresh_scenario_spawner_started() {
            let _ = world.resume_spawner_after_load();
        }
    }

    Ok(PreparedWorld {
        world,
        spawned,
        label,
    })
}

/// Builds a world from a scenario zip and seeds/spawns its starting population.
fn prepare_scenario(
    path: &std::path::Path,
    seed: u64,
    settings_override: Option<&SimSettings>,
    templates_dir: Option<&std::path::Path>,
) -> Result<PreparedWorld, String> {
    let bundle = bibites_io::load_scenario_bundle(path).map_err(|error| error.to_string())?;
    let label = format!("scenario {}", bundle.info.name);
    let mut settings = bundle.settings;
    if let Some(over) = settings_override {
        settings.independents = over.independents.clone();
        settings.sync_rates_from_independents();
    }
    let mut templates = bundle.templates;
    let external_templates_dir = templates_dir
        .map(std::path::Path::to_path_buf)
        .or_else(default_templates_dir);
    if let Some(dir) = &external_templates_dir {
        let external = load_template_dir(dir)?;
        if !external.is_empty() {
            eprintln!(
                "loaded {} template(s) from {}",
                external.len(),
                dir.display()
            );
        }
        templates.extend(external);
    }
    let mut world = World::new(settings, bundle.scenario, seed)
        .map_err(|error| format!("could not build world: {error}"))?;
    let _ = world.seed_scenario_world();
    // The game's fresh-scenario spawner: one bibite immediately, then
    // `virginSpawnRate` (default 1/s) until each spawn row's minimum is met.
    // (The old up-front mass spawn is replaced; save loading is unaffected.)
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
    Ok(PreparedWorld {
        world,
        spawned,
        label,
    })
}

/// The installed game's user template folder, when present. Used only when
/// `--templates` is not supplied, so scenario runs resolve the same templates
/// the game would use.
fn default_templates_dir() -> Option<std::path::PathBuf> {
    let user_profile = std::env::var_os("USERPROFILE")?;
    let dir = std::path::PathBuf::from(user_profile)
        .join("AppData")
        .join("LocalLow")
        .join("The Bibites")
        .join("The Bibites")
        .join("Bibites")
        .join("Templates");
    dir.is_dir().then_some(dir)
}

/// Loads every `.bb8template` in a directory (sorted by file name) so scenario
/// spawn rows can resolve templates the scenario does not bundle.
fn load_template_dir(
    dir: &std::path::Path,
) -> Result<Vec<(String, bibites_io::LoadedGenome)>, String> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
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
            // User template folders can contain edited/broken files that the
            // game itself repairs or ignores; one bad file must not block a run.
            Err(error) => eprintln!(
                "warning: skipping unreadable template {}: {error}",
                path.display()
            ),
        }
    }
    Ok(templates)
}

/// Builds a synthetic benchmark world: default ecology + N starter bibites.
///
/// The world area scales with `sqrt(N)` so every population is benchmarked at
/// the same density as the 2,000-bibite reference world (4,000 × 4,000 units);
/// zone count scales with area, keeping pellet/zone density constant.
fn prepare_synthetic(count: usize, seed: u64) -> Result<PreparedWorld, String> {
    let mut settings = SimSettings::game_defaults();
    let scale = (count as f32 / 2_000.0).sqrt();
    settings.independents.simulation_size = (2_000.0 * scale).min(20_000.0);
    let scenario = ScenarioData::with_default_zone_group();
    let mut world = World::new(settings, scenario, seed)
        .map_err(|error| format!("could not build world: {error}"))?;
    let _ = world.seed_scenario_world();
    let spawned = spawn_synthetic_bibites(&mut world, count)?;
    Ok(PreparedWorld {
        world,
        spawned,
        label: format!("synthetic {count} bibites"),
    })
}

/// Builds a world containing a single bibite from a genome file.
fn prepare_genome(path: &std::path::Path, seed: u64) -> Result<PreparedWorld, String> {
    let loaded = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8template"))
    {
        bibites_io::load_template(path)
    } else {
        bibites_io::load_genome(path)
    }
    .map_err(|error| error.to_string())?;
    let label = format!("genome {}", path.display());
    let settings = SimSettings::game_defaults();
    let scenario = ScenarioData::with_default_zone_group();
    let mut world = World::new(settings, scenario, seed)
        .map_err(|error| format!("could not build world: {error}"))?;
    let _ = world.seed_scenario_world();
    spawn_one(
        &mut world,
        &loaded.genome,
        &loaded.meta,
        &loaded.brain,
        SpawnVitals::Adult,
        0,
        true,
    )?;
    Ok(PreparedWorld {
        world,
        spawned: 1,
        label,
    })
}

/// Spawns `count` starter adult bibites at deterministic random positions.
fn spawn_synthetic_bibites(world: &mut World, count: usize) -> Result<usize, String> {
    for index in 0..count {
        spawn_one(
            world,
            &Genome::starter(),
            &GenomeMeta::default(),
            &Brain::starter(),
            SpawnVitals::Adult,
            index as u64,
            false,
        )?;
    }
    Ok(count)
}

/// Spawns one bibite at a deterministic random position inside the world bounds.
///
/// `spawn_index` gives each spawn its own RNG stream (`Rng::fork` is
/// position-independent, so re-forking the same tag would put every bibite at
/// the same position — see `docs/PERF_DIAGNOSIS.md` F1). `zone_spawn` selects
/// the game's shape-aware zone spawn location (for explicit template/genome
/// runs) instead of a uniform point in the world (synthetic benchmarks).
fn spawn_one(
    world: &mut World,
    genome: &Genome,
    meta: &GenomeMeta,
    brain: &Brain,
    vitals: SpawnVitals,
    spawn_index: u64,
    zone_spawn: bool,
) -> Result<(), String> {
    let mut rng = world
        .rng
        .fork(0xB1B1_7E50 ^ spawn_index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let pos = if zone_spawn && !world.zones.zones.is_empty() {
        let index = (rng.next_u64() as usize) % world.zones.zones.len();
        world.zones.zones[index].spawn_location()
    } else {
        let min = world.bounds.min();
        let max = world.bounds.max();
        let margin = (max - min).length() * 0.05;
        let low = min + Vec2::splat(margin);
        let high = max - Vec2::splat(margin);
        Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y))
    };
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

/// Inspects a file without simulating.
pub fn info(path: &std::path::Path) -> Result<(), String> {
    let kind = file_kind(path);
    match kind {
        FileKind::Genome => {
            let summary = bibites_io::inspect(path).map_err(|error| error.to_string())?;
            println!("file:       {}", path.display());
            println!("shape:      {}", shape_label(summary.shape));
            println!("version:    {}", summary.version);
            println!(
                "genes:      {} (expected {})",
                summary.gene_count,
                bibites_core::genome::N_GENES
            );
            println!(
                "nodes:      {} (inputs {}, outputs {}, hidden {})",
                summary.node_count, summary.input_count, summary.output_count, summary.hidden_count
            );
            println!("synapses:   {}", summary.synapse_count);
            if let Some(tag) = &summary.species_tag {
                println!("species:    {tag}");
            }
            println!("generation: {}", summary.generation);
            Ok(())
        }
        FileKind::Settings => {
            let settings =
                bibites_io::load_settings_file(path).map_err(|error| error.to_string())?;
            print_settings_summary(path, &settings);
            Ok(())
        }
        FileKind::Bundle => info_bundle(path),
        FileKind::Unknown => Err(format!(
            "unsupported file type for `info`: {} (supported: .bb8, .bb8template, \
             .bb8settings, .zip)",
            path.display()
        )),
    }
}

fn print_settings_summary(path: &std::path::Path, settings: &SimSettings) {
    println!("file:       {}", path.display());
    println!("kind:       settings (.bb8settings)");
    println!("sim tps:    {}", settings.sim_tps);
    println!("brain tps:  {}", settings.brain_tps);
    println!("decay tps:  {}", settings.decay_tps);
    println!("sim size:   {}", settings.independents.simulation_size);
    println!(
        "scenario:   initialSeeding={} pelletEnergy={}",
        settings.scenario.initial_seeding, settings.scenario.pellet_energy
    );
}

fn info_bundle(path: &std::path::Path) -> Result<(), String> {
    match bibites_io::load_scenario_bundle(path) {
        Ok(bundle) => {
            println!("file:       {}", path.display());
            println!("kind:       scenario (.zip)");
            println!("name:       {}", bundle.info.name);
            println!("version:    {}", bundle.info.version_text);
            println!(
                "settings:   simTPS={} brainTPS={} decayTPS={} simSize={}",
                bundle.settings.sim_tps,
                bundle.settings.brain_tps,
                bundle.settings.decay_tps,
                bundle.settings.independents.simulation_size
            );
            println!(
                "world:      {} zones, {} zone groups, {} spawn rows",
                bundle.scenario.zones.len(),
                bundle.scenario.zone_groups.len(),
                bundle.scenario.bibite_spawns.len()
            );
            println!(
                "tools:      {} rad towers, {} phero towers, {} colour killers",
                bundle.scenario.rad_towers.len(),
                bundle.scenario.phero_towers.len(),
                bundle.scenario.color_killers.len()
            );
            println!("templates:  {} bundled", bundle.templates.len());
            Ok(())
        }
        Err(scenario_error) => {
            let save = bibites_io::load_save_bundle(path).map_err(|save_error| {
                format!("not a scenario ({scenario_error}) and not a save ({save_error})")
            })?;
            println!("file:       {}", path.display());
            println!("kind:       save (.zip)");
            println!("version:    {}", save.version);
            println!(
                "settings:   simTPS={} brainTPS={} decayTPS={} simSize={}",
                save.settings.sim_tps,
                save.settings.brain_tps,
                save.settings.decay_tps,
                save.settings.independents.simulation_size
            );
            println!(
                "entities:   {} bibites, {} eggs",
                save.bibites.len(),
                save.eggs.len()
            );
            println!(
                "sections:   pellets={} pheromones={} species={} data.bin={} templates={}",
                save.pellets.is_some(),
                save.pheromones.is_some(),
                save.species.is_some() || save.species_raw.is_some(),
                save.data_bin.as_ref().map_or(0, Vec::len),
                save.templates.len()
            );
            Ok(())
        }
    }
}

/// Validates a file and its round-trip.
pub fn check(path: &std::path::Path) -> Result<(), String> {
    match file_kind(path) {
        FileKind::Genome => {
            let loaded = if is_template(path) {
                bibites_io::load_template(path).map_err(|error| error.to_string())?
            } else {
                bibites_io::load_genome(path).map_err(|error| error.to_string())?
            };
            let written = loaded.to_json().map_err(|error| error.to_string())?;
            let reparsed = bibites_io::from_json(&written).map_err(|error| error.to_string())?;

            if loaded.genome != reparsed.genome {
                return Err(format!("round-trip mismatch in genes: {}", path.display()));
            }
            // The I1 writer intentionally serializes genome + brain *structure*
            // only (templates carry no runtime state, and living .bb8 files are
            // re-emitted as genome-only). Compare the serialized structure, not
            // the transient Value/LastInput/LastOutput state.
            if !brains_structurally_equal(&loaded.brain, &reparsed.brain) {
                return Err(format!(
                    "round-trip mismatch in brain topology/parameters: {}",
                    path.display()
                ));
            }
            println!(
                "OK: {} ({} -> {}) genes={} nodes={} synapses={} [structural round-trip]",
                path.display(),
                loaded.version,
                bibites_io::present_version(),
                loaded.genome.genes.len(),
                loaded.brain.node_count(),
                loaded.brain.synapse_count()
            );
            Ok(())
        }
        FileKind::Settings => {
            let settings =
                bibites_io::load_settings_file(path).map_err(|error| error.to_string())?;
            let serialized = bibites_io::serialize_save_settings(&settings)
                .map_err(|error| error.to_string())?;
            bibites_io::parse_settings_json(&serialized).map_err(|error| error.to_string())?;
            println!(
                "OK: {} settings round-trip (simTPS={} brainTPS={} decayTPS={})",
                path.display(),
                settings.sim_tps,
                settings.brain_tps,
                settings.decay_tps
            );
            Ok(())
        }
        FileKind::Bundle => {
            info_bundle(path)?;
            println!("OK: bundle parsed");
            Ok(())
        }
        FileKind::Unknown => Err(format!(
            "unsupported file type for `check`: {} (supported: .bb8, .bb8template, \
             .bb8settings, .zip)",
            path.display()
        )),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Genome,
    Settings,
    Bundle,
    Unknown,
}

fn file_kind(path: &std::path::Path) -> FileKind {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("bb8") | Some("bb8template") => FileKind::Genome,
        Some("bb8settings") => FileKind::Settings,
        Some("zip") => FileKind::Bundle,
        _ => FileKind::Unknown,
    }
}

fn is_template(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8template"))
}

fn shape_label(shape: bibites_io::SourceShape) -> &'static str {
    use bibites_io::SourceShape;
    match shape {
        SourceShape::Template => "template (.bb8template)",
        SourceShape::GenomeOnlyBb8 => "genome (.bb8)",
        SourceShape::LivingBb8 => "living bibite (.bb8)",
        SourceShape::EggBb8 => "egg (.bb8)",
    }
}

/// Compares the parts of a brain that the I1 serializer round-trips: node
/// topology/types/innovations/base activations/descriptions and every synapse
/// record (weight, enabled, ids). Runtime state is intentionally excluded.
fn brains_structurally_equal(
    left: &bibites_core::brain::Brain,
    right: &bibites_core::brain::Brain,
) -> bool {
    if left.node_count() != right.node_count() || left.synapse_count() != right.synapse_count() {
        return false;
    }
    let left_nodes = left.to_node_records();
    let right_nodes = right.to_node_records();
    if left_nodes.len() != right_nodes.len() {
        return false;
    }
    for (a, b) in left_nodes.iter().zip(right_nodes.iter()) {
        if a.node_type != b.node_type
            || a.index != b.index
            || a.innovation != b.innovation
            || a.base_activation.to_bits() != b.base_activation.to_bits()
        {
            return false;
        }
        // `desc` is deliberately not compared: the game derives canonical names
        // for base/hidden nodes during load normalisation (e.g. "Hidden0"), so
        // None and the derived name are equivalent. Custom descriptions are
        // preserved by the writer and covered by the io unit tests.
    }
    left.to_synapse_records() == right.to_synapse_records()
}

/// Prints the per-phase simulation timing breakdown collected by `World`.
fn print_phase_table(world: &World) {
    let timings = world.phase_timings();
    println!("{:<18} {:>10} {:>8}", "phase", "ms/tick", "share");
    for phase in bibites_core::sim::Phase::ALL {
        println!(
            "{:<18} {:>10.3} {:>7.1}%",
            phase.name(),
            timings.mean_nanos(phase) / 1e6,
            timings.share(phase) * 100.0
        );
        // The stages inside the phase, indented, with their share of the phase.
        let phase_nanos = timings.nanos(phase).max(1) as f64;
        for stage in bibites_core::sim::Stage::ALL
            .into_iter()
            .filter(|stage| stage.phase() == phase)
        {
            println!(
                "  {:<16} {:>10.3} {:>7.1}%  of {}",
                stage.name(),
                timings.mean_stage_nanos(stage) / 1e6,
                timings.stage_nanos[stage as usize] as f64 / phase_nanos * 100.0,
                phase.name()
            );
        }
    }
    println!(
        "at the end: {} bibites, {} pellets, {} eggs",
        world.bibites.len(),
        world.pellets.len(),
        world.eggs.len()
    );
    let sleep = world.pellet_sleep_stats();
    if sleep.ticks > 0 {
        println!(
            "resting pellets: {} asleep, {} awake; per tick {:.1} promoted into the step, {:.1} fell asleep, {:.1} woke",
            sleep.asleep,
            sleep.awake,
            sleep.promoted as f64 / sleep.ticks as f64,
            sleep.slept as f64 / sleep.ticks as f64,
            sleep.woken as f64 / sleep.ticks as f64
        );
    }
    // Which of the bibites' pass-B turns ran on the worker pool (parallel ticks only).
    let lanes = world.pass_b_lanes();
    let turns = (lanes.light + lanes.heavy).max(1) as f64;
    if lanes.ticks > 0 {
        println!(
            "pass-b turns: {:.1}% on the pool; in dense order {:.1}% (mouth {:.1}%, held {:.1}%, vomit {:.1}%, eggs {:.1}%, pheromone {:.1}%, dead {:.1}%)",
            lanes.light as f64 / turns * 100.0,
            lanes.heavy as f64 / turns * 100.0,
            lanes.mouth as f64 / turns * 100.0,
            lanes.held as f64 / turns * 100.0,
            lanes.vomit as f64 / turns * 100.0,
            lanes.eggs as f64 / turns * 100.0,
            lanes.emit as f64 / turns * 100.0,
            lanes.dead as f64 / turns * 100.0
        );
        // The dense-order turns that were cut in parts: what of them the pool took.
        let heavy = lanes.heavy.max(1) as f64;
        println!(
            "dense-order turns cut in parts: head taken in the sweep {:.1}%, last part on the pool {:.1}% of the dense-order turns",
            lanes.head_in_sweep as f64 / heavy * 100.0,
            lanes.post_on_pool as f64 / heavy * 100.0
        );
    }
    // How a world that measures its own ticks ran them (inline, on the pool at each level), what the probes found, and the way it ended on.
    let tuner = world.pool_tuner_stats();
    if tuner.inline_ticks + tuner.pooled_ticks > 0 {
        let mean = |nanos: Option<f64>| {
            nanos.map_or_else(|| "-".to_owned(), |nanos| format!("{:.3}", nanos / 1e6))
        };
        println!(
            "pool tuner: {} ticks inline, {} on the pool (by level {:?}); {} probes ({} dropped early), {} switches, ended {}; latest mean tick in ms: inline {}, pool level 0 {}, 1 {}, 2 {}, 3 {}",
            tuner.inline_ticks,
            tuner.pooled_ticks,
            tuner.level_ticks,
            tuner.probes,
            tuner.aborted_probes,
            tuner.switches,
            world.pool_arm().label(),
            mean(tuner.arm_ns[0]),
            mean(tuner.arm_ns[1]),
            mean(tuner.arm_ns[2]),
            mean(tuner.arm_ns[3]),
            mean(tuner.arm_ns[4])
        );
    }
}

/// Emits Rust brain evaluator outputs for the independent Python checker.
pub fn braincheck(fixture: &std::path::Path, out: Option<&std::path::Path>) -> Result<(), String> {
    crate::braincheck::run(fixture, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save+sidecar resume must produce exactly the same world as one long run.
    #[test]
    fn checkpoint_sidecar_resume_is_bit_identical() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let default = root.join("testdata/real/Default.zip");
        if !default.exists() {
            eprintln!("skipping: testdata/real/Default.zip is absent");
            return;
        }
        let seed = 12_345u64;
        let single = {
            let mut prepared = prepare_zip(&default, seed, None, None).expect("default scenario");
            prepared.world.set_threads(1);
            prepared.world.run(400);
            bibites_core::sim::state_fingerprint(&prepared.world)
        };

        let split = {
            let mut first = prepare_zip(&default, seed, None, None).expect("default scenario");
            first.world.set_threads(1);
            // Split mid-virgin-spawn (Default starts 1 and reaches 10 around
            // tick 270); this also exercises spawner template resolution after
            // the save reload.
            first.world.run(200);
            let fp_first = bibites_core::sim::state_fingerprint(&first.world);

            let save = root.join("target/checkpoint_cli_test.zip");
            let contents = first.world.to_contents();
            bibites_io::save_world_contents(&save, &contents).expect("save world");
            let snapshot = first
                .world
                .capture_checkpoint_state()
                .expect("capture checkpoint");
            bibites_io::write_checkpoint_state_with_seed(&save, &snapshot, seed)
                .expect("write sidecar");

            let mut resumed = prepare_save(&save, seed, None, None).expect("reload save");
            resumed.world.set_threads(1);
            let snapshot = bibites_io::read_checkpoint_state_checked(&save, seed)
                .expect("read sidecar")
                .expect("sidecar present");
            resumed
                .world
                .restore_checkpoint_state(&snapshot)
                .expect("restore checkpoint");
            let fp_restored = bibites_core::sim::state_fingerprint(&resumed.world);
            eprintln!("fp first after 200 = {fp_first}, restored = {fp_restored}");
            resumed.world.run(200);
            bibites_core::sim::state_fingerprint(&resumed.world)
        };

        let _ = std::fs::remove_file(root.join("target/checkpoint_cli_test.zip"));
        let _ = std::fs::remove_file(bibites_io::checkpoint_path(
            &root.join("target/checkpoint_cli_test.zip"),
        ));
        assert_eq!(split, single, "sidecar resume diverged from a single run");
    }

    #[test]
    fn template_spawn_lands_inside_a_zone() {
        let mut world = World::new(
            SimSettings::game_defaults(),
            ScenarioData::with_default_zone_group(),
            7,
        )
        .expect("default world");
        let _ = world.seed_scenario_world();
        assert!(
            !world.zones.zones.is_empty(),
            "default zone group should seed zones"
        );
        spawn_one(
            &mut world,
            &Genome::starter(),
            &GenomeMeta::default(),
            &Brain::starter(),
            SpawnVitals::Adult,
            0,
            true,
        )
        .expect("template spawn");
        let pos = world
            .bibites
            .ids()
            .next()
            .and_then(|id| world.bibites.view(id).map(|view| view.pos))
            .expect("spawned bibite");
        assert!(
            world.zones.zones.iter().any(|zone| zone.contains(pos)),
            "zone-aware spawn at {pos:?} is outside every zone"
        );
    }

    #[test]
    fn synthetic_spawns_use_distinct_positions() {
        let mut world = World::new(
            SimSettings::game_defaults(),
            ScenarioData::with_default_zone_group(),
            7,
        )
        .expect("default world");
        let count = spawn_synthetic_bibites(&mut world, 64).expect("spawn");
        assert_eq!(count, 64);
        assert_eq!(world.bibites.len(), 64);

        let positions: Vec<Vec2> = world
            .bibites
            .ids()
            .filter_map(|id| world.bibites.view(id).map(|view| view.pos))
            .collect();
        assert_eq!(positions.len(), 64);

        // Regression for the coincident-spawn bug (docs/PERF_DIAGNOSIS.md F1).
        assert!(
            positions.windows(2).any(|pair| pair[0] != pair[1]),
            "all spawn positions are identical"
        );
        let mut min_distance = f32::INFINITY;
        for (index, left) in positions.iter().enumerate() {
            for right in &positions[index + 1..] {
                min_distance = min_distance.min(left.distance(*right));
            }
        }
        assert!(
            min_distance > 1.0,
            "spawn positions are clustered (min distance {min_distance})"
        );
    }
}
