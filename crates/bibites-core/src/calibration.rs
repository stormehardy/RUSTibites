//! A calibration of the machine: how many workers a tick should use, from what size of world the pool pays, and at which level.
//!
//! The simulation never computes anything differently for it (serial and pool ticks are bit-identical, whatever the number of workers, the level
//! or the mix: the tests compare whole checkpoints), so a calibration is only about *time*. [`crate::pool_tuner::PoolTuner`] already finds the
//! fastest way for the world it is ticking, but it starts from a first guess that was measured on one machine and has to look at the other
//! ways while the player waits. A calibration looks at them once, on purpose, on synthetic worlds of every size, and hands the tuner a first
//! guess that is right for this machine ([`crate::sim::MachineProfile`]):
//!
//! 1. **Workers.** A world of a few thousand bibites ticks on the pool with 2, 3, 4, ... up to every thread the machine has, at each of the
//!    four levels of eagerness; the most workers that are within 12 % of the fastest are the ones a tick uses (a tick that is memory-bound
//!    stops gaining at some number of workers, so the times of many are alike and the noise of the measurement (about a tenth) decides which is the fastest; a machine
//!    with efficiency cores or hyper-threads can lose by using them all, and then the times are clearly worse).
//! 2. **The landscape.** With those workers, worlds of 150 to 10,000 bibites tick inline and at each level, alternating round after round
//!    (the cost of a synthetic world drifts, so no way is measured all at once) and each way keeps its best block. The pool pays from the
//!    size where its best level takes under 92 % of the inline time for good; the level that was the best at the big sizes is the one a
//!    world starts at.
//!
//! The result is plain numbers ([`Calibration`]), kept as a text file ([`Calibration::to_text`]), and a [`crate::sim::MachineProfile`] for the
//! process. A world that is bigger or smaller than the synthetic ones, or a machine that is busy, is still the tuner's business: it measures the
//! world's own ticks all the time and leaves the profile's guess behind as soon as the guess is wrong.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::body::{BibiteSpawn, SpawnVitals};
use crate::brain::Brain;
use crate::genome::{Genome, GenomeMeta};
use crate::math::Vec2;
use crate::pool_tuner::{ARMS, Arm, Choice, POOLED_LEVELS};
use crate::settings::SimSettings;
use crate::sim::{MachineProfile, ParallelGates};
use crate::world::{ScenarioData, World};

/// The pool has to take less than this share of the inline time to count as paying (the tuner's own margin is 90 %: a first guess a little
/// early is cheaper than one that is late).
const PAYS_BELOW: f64 = 0.92;
/// Workers within this share above the fastest count as equally fast (the most of them are used). The tick is memory-bound from a few workers on, so the times
/// of 3 and of 12 workers are alike and differ by the noise of the measurement (about a tenth); a tie band narrower than that picks by luck.
const WORKERS_TIE: f64 = 1.12;
/// Bodies for every bibite in the tuner's prior (3,000 bodies over 1,200 bibites on the reference machine).
const BODIES_PER_BIBITE: f64 = 2.5;
/// The smallest size the pool is guessed to pay from (the tuner's floor is a quarter of it).
const SMALLEST_CROSSOVER: usize = 240;
/// When the pool did not pay at any size that was measured, the guess is this many times the biggest size.
const NEVER_FACTOR: usize = 2;
/// Ticks of a block that are not measured (cold caches, sleeping workers).
const WARM_TICKS: u32 = 3;
/// A calibration whose blocks of the same way differ (the median over the best) by more than this share (the machine was busy) is marked noisy.
const NOISY_ABOVE: f64 = 0.25;
/// What the text of a stored calibration starts with.
const MAGIC: &str = "rustibites-calibration 1";

/// What a calibration measures, and how hard.
#[derive(Clone, Debug)]
pub struct Config {
    /// The threads the machine has (workers are tried from 2 up to this).
    pub threads: usize,
    /// Bibites of the world the workers are compared on.
    pub workers_bibites: usize,
    /// The sizes of the landscape, in bibites, smallest first.
    pub sizes: Vec<usize>,
    /// Times every way is measured at every size (and every number of workers): each way keeps its best block.
    pub rounds: u32,
    /// How long a block of ticks should take, in nanoseconds.
    pub block_nanos: f64,
    /// Ticks a new world runs inline before it is measured.
    pub settle_ticks: u32,
}

impl Config {
    /// The calibration the Settings page runs: about half a minute on a desktop.
    #[must_use]
    pub fn full(threads: usize) -> Self {
        let mut sizes = vec![150, 300, 600, 1_000, 1_600, 2_500, 4_000, 6_500];
        if threads >= 8 {
            sizes.push(10_000);
        }
        if threads <= 4 {
            sizes.retain(|size| *size <= 4_000);
        }
        Self {
            threads,
            workers_bibites: (threads * 500).clamp(3_000, 8_000),
            sizes,
            rounds: 5,
            block_nanos: 24_000_000.0,
            settle_ticks: 250,
        }
    }

    /// A short calibration (about a fifth of the time, rougher): for the tests and `--quick`.
    #[must_use]
    pub fn quick(threads: usize) -> Self {
        Self {
            sizes: vec![200, 500, 1_000, 2_000, 4_000],
            rounds: 3,
            block_nanos: 10_000_000.0,
            settle_ticks: 80,
            ..Self::full(threads)
        }
    }

    /// A calibration of two tiny worlds, only for the tests of the machinery.
    #[must_use]
    pub fn tiny(threads: usize) -> Self {
        Self { threads, workers_bibites: 80, sizes: vec![60, 120], rounds: 1, block_nanos: 1_000_000.0, settle_ticks: 4 }
    }
}

/// How far a calibration is.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// Steps done.
    pub done: usize,
    /// Steps there are.
    pub total: usize,
    /// What the step that runs now does.
    pub what: String,
}

impl Progress {
    /// The share done, 0 to 1.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.total == 0 { 0.0 } else { (self.done as f32 / self.total as f32).clamp(0.0, 1.0) }
    }
}

/// Why a calibration ended without a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The caller asked it to stop.
    Cancelled,
    /// The machine could not build the worlds it measures.
    NoWorld,
}

/// The time of a tick at one size of world, in each way (best block, nanoseconds), by [`Arm::index`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeRow {
    /// Bibites of the world.
    pub bibites: usize,
    /// Bodies of the world (bibites, pellets and eggs) when it was measured.
    pub bodies: usize,
    /// A tick's time in each way.
    pub ns: [f64; ARMS],
}

impl SizeRow {
    /// The level of the pool that was the fastest, and its time.
    #[must_use]
    pub fn best_pooled(&self) -> (u8, f64) {
        let mut best = (0_u8, self.ns[1]);
        for level in 1..POOLED_LEVELS {
            if self.ns[1 + level] < best.1 {
                best = (level as u8, self.ns[1 + level]);
            }
        }
        best
    }

    /// The pool's best time over the inline one (under 1: the pool is faster).
    #[must_use]
    pub fn ratio(&self) -> f64 {
        self.best_pooled().1 / self.ns[0]
    }

    /// How many times faster the best way is than inline (1 when inline is the best).
    #[must_use]
    pub fn speedup(&self) -> f64 {
        (self.ns[0] / self.best_pooled().1).max(1.0)
    }
}

/// The time of a tick on the pool with a number of workers (the best level, nanoseconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkerRow {
    /// Workers of the pool.
    pub workers: usize,
    /// The level that was the fastest with them.
    pub level: u8,
    /// A tick's time at that level.
    pub ns: f64,
}

/// What a calibration found.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    /// Threads the machine had when it was measured (a calibration of another machine is not used).
    pub threads: usize,
    /// Workers a tick uses.
    pub workers: usize,
    /// Live bibites from which the pool pays (the first guess of a world).
    pub pool_bibites: usize,
    /// The level of the pool a world starts at.
    pub start_level: u8,
    /// How many times faster the fastest way was than inline at the biggest size.
    pub speedup: f64,
    /// How much the blocks of the same way differed (0.04: 4 %): a high number means the machine was busy.
    pub noise: f64,
    /// Seconds the calibration took.
    pub seconds: f64,
    /// When it was made (seconds since the Unix epoch).
    pub when: u64,
    /// The time at every number of workers that was tried.
    pub worker_rows: Vec<WorkerRow>,
    /// The time at every size, in every way.
    pub rows: Vec<SizeRow>,
}

impl Calibration {
    /// The profile the process uses.
    #[must_use]
    pub fn profile(&self) -> MachineProfile {
        MachineProfile {
            workers: self.workers.max(1),
            pool_bibites: self.pool_bibites.max(1),
            pool_bodies: ((self.pool_bibites as f64 * BODIES_PER_BIBITE).round() as usize).max(1),
            start_level: self.start_level,
        }
    }

    /// Whether the machine was too busy for the numbers to be trusted.
    #[must_use]
    pub fn noisy(&self) -> bool {
        self.noise > NOISY_ABOVE
    }

    /// Whether this was made on a machine with as many threads as this one (else it says nothing about it).
    #[must_use]
    pub fn is_for_this_machine(&self) -> bool {
        self.threads == available_threads()
    }

    /// The numbers as text (one `key=value` per line, the rows as lists), for the file that keeps them.
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut text = format!("{MAGIC}\nthreads={}\nworkers={}\npool_bibites={}\nstart_level={}\nspeedup={:.3}\nnoise={:.3}\nseconds={:.1}\nwhen={}\n",
            self.threads, self.workers, self.pool_bibites, self.start_level, self.speedup, self.noise, self.seconds, self.when);
        for row in &self.worker_rows {
            let _ = writeln!(text, "workers_row={},{},{:.0}", row.workers, row.level, row.ns);
        }
        for row in &self.rows {
            let _ = write!(text, "row={},{}", row.bibites, row.bodies);
            for ns in row.ns {
                let _ = write!(text, ",{ns:.0}");
            }
            text.push('\n');
        }
        text
    }

    /// Reads what [`Calibration::to_text`] wrote (a line it does not know is skipped; a missing number is an error).
    ///
    /// # Errors
    /// The text is not a calibration, or a number of it is missing or does not read.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(MAGIC) {
            return Err("not a calibration of this program".to_owned());
        }
        let mut found = std::collections::BTreeMap::new();
        let mut worker_rows = Vec::new();
        let mut rows = Vec::new();
        for line in lines {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "workers_row" => {
                    let parts: Vec<&str> = value.split(',').collect();
                    let [workers, level, ns] = parts[..] else {
                        return Err(format!("a workers row has {} numbers, not 3", parts.len()));
                    };
                    worker_rows.push(WorkerRow {
                        workers: workers.parse().map_err(|_| "a workers row".to_owned())?,
                        level: level.parse().map_err(|_| "a workers row".to_owned())?,
                        ns: ns.parse().map_err(|_| "a workers row".to_owned())?,
                    });
                }
                "row" => {
                    let numbers: Vec<&str> = value.split(',').collect();
                    if numbers.len() != 2 + ARMS {
                        return Err(format!("a size row has {} numbers, not {}", numbers.len(), 2 + ARMS));
                    }
                    let mut ns = [0.0; ARMS];
                    for (slot, number) in ns.iter_mut().zip(&numbers[2..]) {
                        *slot = number.parse().map_err(|_| "a size row".to_owned())?;
                    }
                    rows.push(SizeRow {
                        bibites: numbers[0].parse().map_err(|_| "a size row".to_owned())?,
                        bodies: numbers[1].parse().map_err(|_| "a size row".to_owned())?,
                        ns,
                    });
                }
                other => {
                    found.insert(other.to_owned(), value.to_owned());
                }
            }
        }
        fn number<T: std::str::FromStr>(found: &std::collections::BTreeMap<String, String>, key: &str) -> Result<T, String> {
            found.get(key).ok_or_else(|| format!("{key} is missing"))?.parse().map_err(|_| format!("{key} does not read"))
        }
        Ok(Self {
            threads: number(&found, "threads")?,
            workers: number(&found, "workers")?,
            pool_bibites: number(&found, "pool_bibites")?,
            start_level: number::<u8>(&found, "start_level")?.min(POOLED_LEVELS as u8 - 1),
            speedup: number(&found, "speedup")?,
            noise: number(&found, "noise")?,
            seconds: number(&found, "seconds")?,
            when: number(&found, "when")?,
            worker_rows,
            rows,
        })
    }
}

/// The threads this machine has.
#[must_use]
pub fn available_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// The numbers of workers worth trying on a machine of `threads` threads: all of them up to 4, then 6, 8, 12, 16, 24, 32 and the machine's own number.
#[must_use]
pub fn worker_candidates(threads: usize) -> Vec<usize> {
    let mut counts: Vec<usize> = (2..=threads.min(4)).collect();
    counts.extend([6, 8, 12, 16, 24, 32].into_iter().filter(|count| *count <= threads));
    if threads >= 2 {
        counts.push(threads);
    }
    counts.sort_unstable();
    counts.dedup();
    counts
}

/// The number of workers a tick uses: the most of them that are within 12 % of the fastest (`rows` empty: one).
#[must_use]
pub fn pick_workers(rows: &[WorkerRow]) -> usize {
    let Some(best) = rows.iter().map(|row| row.ns).reduce(f64::min) else {
        return 1;
    };
    rows.iter().filter(|row| row.ns <= best * WORKERS_TIE).map(|row| row.workers).max().unwrap_or(1)
}

/// The size from which the pool pays for good: between the last size where it did not and the first of those after it where it did, found by
/// interpolating the ratio of the two times over the logarithm of the size. `None`: it did not pay at the biggest size. A pool that paid at
/// the smallest size pays from there.
#[must_use]
pub fn crossover(rows: &[SizeRow]) -> Option<usize> {
    let first = (0..rows.len()).find(|&start| rows[start..].iter().all(|row| row.ratio() < PAYS_BELOW))?;
    if first == 0 {
        return Some(rows[0].bibites);
    }
    let (before, after) = (&rows[first - 1], &rows[first]);
    let (r0, r1) = (before.ratio(), after.ratio());
    let t = if r0 > r1 { ((r0 - PAYS_BELOW) / (r0 - r1)).clamp(0.0, 1.0) } else { 1.0 };
    let (s0, s1) = (before.bibites as f64, after.bibites as f64);
    Some((s0 * (s1 / s0).powf(t)).round() as usize)
}

/// The level of the pool a world that is big enough starts at: the one that was the fastest at the most of the sizes from the crossover on
/// (the bigger size decides a tie).
#[must_use]
pub fn start_level(rows: &[SizeRow], crossover: Option<usize>) -> u8 {
    let from = crossover.unwrap_or(usize::MAX);
    let mut votes = [0_u32; POOLED_LEVELS];
    let mut last = 0_u8;
    for row in rows.iter().filter(|row| row.bibites >= from) {
        let (level, _) = row.best_pooled();
        votes[usize::from(level)] += 1;
        last = level;
    }
    let most = votes.iter().copied().max().unwrap_or(0);
    if most == 0 {
        return 0;
    }
    if votes[usize::from(last)] == most { last } else { votes.iter().position(|count| *count == most).map_or(0, |level| level as u8) }
}

/// How much the blocks of one way differed: the median over the ways of (the median block over the fastest) minus one. (The slowest block would count the
/// drift of a world that is still settling, and one hiccup; a busy machine slows most blocks.)
fn spread_of(blocks: &[Vec<f64>]) -> f64 {
    let mut spreads: Vec<f64> = blocks
        .iter()
        .filter(|way| way.len() >= 2)
        .map(|way| {
            let mut sorted = way.clone();
            sorted.sort_by(f64::total_cmp);
            sorted[sorted.len() / 2] / sorted[0] - 1.0
        })
        .collect();
    if spreads.is_empty() {
        return 0.0;
    }
    spreads.sort_by(f64::total_cmp);
    spreads[spreads.len() / 2]
}

/// A synthetic world of `count` starter bibites at the density of the 2,000-bibite reference world (the benchmarks' world).
///
/// # Errors
/// The world could not be built or a bibite could not be spawned.
pub fn synthetic_world(count: usize, seed: u64) -> Result<World, String> {
    let scale = (count as f32 / 2_000.0).sqrt();
    synthetic_world_in(count, seed, (2_000.0 * scale).clamp(500.0, 20_000.0))
}

/// A synthetic world of `count` starter bibites in a world of `simulation_size` (its half width): a small size packs them (the window's graphics test
/// draws a crowd).
///
/// # Errors
/// The world could not be built or a bibite could not be spawned.
pub fn synthetic_world_in(count: usize, seed: u64, simulation_size: f32) -> Result<World, String> {
    let mut settings = SimSettings::game_defaults();
    settings.independents.simulation_size = simulation_size;
    let scenario = ScenarioData::with_default_zone_group();
    let mut world = World::new(settings, scenario, seed).map_err(|error| format!("could not build world: {error}"))?;
    let _ = world.seed_scenario_world();
    let (genome, meta, brain) = (Genome::starter(), GenomeMeta::default(), Brain::starter());
    for index in 0..count {
        let mut rng = world.rng.fork(0xB1B1_7E50 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (min, max) = (world.bounds.min(), world.bounds.max());
        let margin = (max - min).length() * 0.05;
        let (low, high) = (min + Vec2::splat(margin), max - Vec2::splat(margin));
        let mut fresh = brain.clone();
        fresh.reset_runtime_state(world.settings.brain_period());
        let spawn = BibiteSpawn {
            pos: Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y)),
            rot: rng.f32_range(0.0, std::f32::consts::TAU),
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            genome: genome.clone(),
            meta: meta.clone(),
            brain: fresh,
            vitals: SpawnVitals::Adult,
            energy: None,
            health: None,
        };
        let id = world.bibites.spawn(spawn).map_err(|error| format!("could not spawn bibite: {error}"))?;
        world.register_bibite(id);
    }
    Ok(world)
}

/// The mean time of a tick in nanoseconds over a block of `ticks` run the way `arm` says (after a few that are not counted).
fn time_block(world: &mut World, arm: Arm, ticks: u32) -> f64 {
    let choice = Choice { arm, timed: false };
    for _ in 0..WARM_TICKS {
        world.tick_in(choice);
    }
    let started = Instant::now();
    for _ in 0..ticks {
        world.tick_in(choice);
    }
    started.elapsed().as_nanos() as f64 / f64::from(ticks)
}

/// Ticks of a block of `block_nanos`, from what one tick costs (4 to 400).
fn block_ticks(block_nanos: f64, tick_nanos: f64) -> u32 {
    if tick_nanos <= 0.0 { 400 } else { ((block_nanos / tick_nanos) as u32).clamp(4, 400) }
}

/// The progress of a calibration, and whether to go on.
struct Run<'a> {
    cancel: &'a AtomicBool,
    report: &'a mut dyn FnMut(&Progress),
    done: usize,
    total: usize,
}

impl Run<'_> {
    fn step(&mut self, what: String) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        (self.report)(&Progress { done: self.done, total: self.total, what });
        self.done += 1;
        Ok(())
    }

    fn check(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) { Err(Stop::Cancelled) } else { Ok(()) }
    }
}

/// Measures every way `arms` lists on `world`, `rounds` times round, each way starting a round after the one that started the round before (so that
/// none of them always meets the drift of the world first); returns the best block of each way and every block, by position in `arms`.
fn measure_ways(world: &mut World, arms: &[Arm], config: &Config, run: &Run<'_>) -> Result<(Vec<f64>, Vec<Vec<f64>>), Stop> {
    let inline = time_block(world, Arm::Inline, 6);
    let ticks = block_ticks(config.block_nanos, inline);
    let mut blocks: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    for round in 0..config.rounds.max(1) {
        for offset in 0..arms.len() {
            run.check()?;
            let way = (offset + round as usize) % arms.len();
            blocks[way].push(time_block(world, arms[way], ticks));
        }
    }
    let best = blocks.iter().map(|way| way.iter().copied().fold(f64::MAX, f64::min)).collect();
    Ok((best, blocks))
}

/// Calibrates the machine. `report` is told how far it is; `cancel` stops it at the next block (`Err(Stop::Cancelled)`).
///
/// # Errors
/// [`Stop::Cancelled`] when `cancel` was set, [`Stop::NoWorld`] when a synthetic world could not be built.
pub fn run(config: &Config, cancel: &AtomicBool, report: &mut dyn FnMut(&Progress)) -> Result<Calibration, Stop> {
    let started = Instant::now();
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let threads = config.threads.max(1);
    let candidates = worker_candidates(threads);
    let measures_workers = candidates.len() > 1;
    let mut steps = Run { cancel, report, done: 0, total: usize::from(measures_workers) + config.sizes.len() + 1 };
    let mut all_blocks: Vec<Vec<f64>> = Vec::new();
    let mut worker_rows = Vec::new();
    let mut rows = Vec::new();

    if threads < 2 {
        // One thread: nothing to share; every world ticks inline.
        steps.step("This machine has one thread: nothing to calibrate".to_owned())?;
        return Ok(Calibration {
            threads,
            workers: 1,
            pool_bibites: config.sizes.iter().copied().max().unwrap_or(1_000) * NEVER_FACTOR,
            start_level: 0,
            speedup: 1.0,
            noise: 0.0,
            seconds: started.elapsed().as_secs_f64(),
            when,
            worker_rows,
            rows,
        });
    }

    // 1. Workers: the pool with 2, 3, 4, ... workers at the four levels, in a world of a few thousand bibites. Two things bias this if done the easy way. A
    // synthetic world's ticks get dearer as it runs for the first thousand ticks or so, so counts measured one after the other favour the first: every
    // (workers, level) pair is therefore measured in every round, in turn. And a world that is told another number of threads drops or grows the per-worker
    // buffers of its stages, so the first ticks after a change are slower for a pool of more workers: every number of workers therefore has a world of its own
    // (the same world, made the same way), which keeps its threads, its buffers and its cache lines from block to block.
    let mut workers = threads;
    if measures_workers {
        steps.step(format!("Workers: {} bibites on 2 to {threads} workers", config.workers_bibites))?;
        let mut worlds: Vec<World> = Vec::with_capacity(candidates.len());
        for &count in &candidates {
            steps.check()?;
            let mut world = synthetic_world(config.workers_bibites, 0xCA11_B0A7).map_err(|_| Stop::NoWorld)?;
            world.set_parallel_gates(ParallelGates::SCALED);
            world.set_threads(count);
            world.run(u64::from(config.settle_ticks));
            worlds.push(world);
        }
        let pairs: Vec<(usize, u8)> = (0..candidates.len()).flat_map(|world| (0..POOLED_LEVELS as u8).map(move |level| (world, level))).collect();
        let inline = time_block(&mut worlds[0], Arm::Inline, 6);
        let ticks = block_ticks(config.block_nanos, inline);
        let mut blocks: Vec<Vec<f64>> = vec![Vec::new(); pairs.len()];
        for round in 0..config.rounds.max(1) {
            for offset in 0..pairs.len() {
                steps.check()?;
                // (Each round starts at another pair, so that none always meets the drift first.)
                let at = (offset + round as usize * 7) % pairs.len();
                let (world, level) = pairs[at];
                blocks[at].push(time_block(&mut worlds[world], Arm::Pooled(level), ticks));
            }
        }
        for (index, &count) in candidates.iter().enumerate() {
            let best = pairs
                .iter()
                .enumerate()
                .filter(|(_, (world, _))| *world == index)
                .flat_map(|(at, (_, level))| blocks[at].iter().map(move |ns| (*level, *ns)))
                .fold((0_u8, f64::MAX), |best, (level, ns)| if ns < best.1 { (level, ns) } else { best });
            worker_rows.push(WorkerRow { workers: count, level: best.0, ns: best.1 });
        }
        all_blocks.extend(blocks);
        workers = pick_workers(&worker_rows);
    }

    // 2. The landscape, with those workers: inline and the four levels at every size.
    let ways = Arm::ALL;
    for &bibites in &config.sizes {
        steps.step(format!("Sizes: {bibites} bibites, inline and on {workers} workers"))?;
        let mut world = synthetic_world(bibites, 0xCA11_B0A8 ^ bibites as u64).map_err(|_| Stop::NoWorld)?;
        world.set_parallel_gates(ParallelGates::SCALED);
        world.set_threads(threads);
        world.run(u64::from(config.settle_ticks));
        world.set_threads(workers);
        let bodies = world.bibites.len() + world.pellets.len() + world.eggs.len();
        let (best, blocks) = measure_ways(&mut world, &ways, config, &steps)?;
        let mut ns = [0.0; ARMS];
        ns.copy_from_slice(&best);
        rows.push(SizeRow { bibites, bodies, ns });
        all_blocks.extend(blocks);
    }

    steps.step("Working out where the pool pays".to_owned())?;
    let crossover = crossover(&rows);
    let level = start_level(&rows, crossover);
    let biggest = rows.last().copied();
    Ok(Calibration {
        threads,
        workers,
        pool_bibites: crossover.map_or_else(|| rows.last().map_or(1_000, |row| row.bibites) * NEVER_FACTOR, |size| size.max(SMALLEST_CROSSOVER)),
        start_level: level,
        speedup: biggest.map_or(1.0, |row| row.speedup()),
        noise: spread_of(&all_blocks),
        seconds: started.elapsed().as_secs_f64(),
        when,
        worker_rows,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(bibites: usize, inline: f64, levels: [f64; POOLED_LEVELS]) -> SizeRow {
        let mut ns = [inline; ARMS];
        ns[1..].copy_from_slice(&levels);
        SizeRow { bibites, bodies: bibites * 3, ns }
    }

    /// A machine where the pool loses below ~1,000 bibites and wins above (level 1 the best).
    fn desktop() -> Vec<SizeRow> {
        vec![
            row(150, 100.0, [300.0, 280.0, 260.0, 400.0]),
            row(300, 200.0, [350.0, 330.0, 300.0, 450.0]),
            row(600, 400.0, [520.0, 480.0, 450.0, 600.0]),
            row(1_000, 700.0, [700.0, 640.0, 650.0, 800.0]),
            row(1_600, 1_100.0, [800.0, 700.0, 720.0, 900.0]),
            row(2_500, 1_800.0, [1_000.0, 850.0, 900.0, 1_100.0]),
        ]
    }

    #[test]
    fn the_pool_pays_from_the_size_where_its_best_level_takes_under_92_percent_for_good() {
        let rows = desktop();
        // 1,000 bibites: 640 / 700 = 0.914 (pays); 600: 450 / 400 = 1.125 (does not): between them, towards the 1,000 end.
        let size = crossover(&rows).expect("the pool pays at the big sizes");
        assert!((600..=1_000).contains(&size), "{size}");
        assert!(size > 900, "the ratio at 1,000 is right at the line, so the crossing is near it: {size}");
        // A pool that pays at every size pays from the smallest; one that never pays has no crossover.
        let all: Vec<SizeRow> = rows.iter().map(|row| SizeRow { ns: [100.0, 50.0, 60.0, 70.0, 80.0], ..*row }).collect();
        assert_eq!(crossover(&all), Some(150));
        let never: Vec<SizeRow> = rows.iter().map(|row| SizeRow { ns: [100.0, 150.0, 160.0, 170.0, 180.0], ..*row }).collect();
        assert_eq!(crossover(&never), None);
        assert_eq!(crossover(&[]), None);
    }

    #[test]
    fn a_size_where_the_pool_wins_after_losing_does_not_move_the_crossover_down() {
        // A fluke at 300 (the pool wins once), then losing again at 600: the crossover is where it wins for good.
        let mut rows = desktop();
        rows[1] = row(300, 200.0, [100.0, 100.0, 100.0, 100.0]);
        let size = crossover(&rows).unwrap();
        assert!(size > 600, "{size}");
    }

    #[test]
    fn the_start_level_is_the_one_that_was_the_fastest_from_the_crossover_on() {
        let rows = desktop();
        let size = crossover(&rows);
        assert_eq!(start_level(&rows, size), 1, "level 1 wins at 1,000, 1,600 and 2,500");
        // Two votes for level 2 and one for level 0: level 2. A tie goes to the biggest size's.
        let mixed = vec![
            row(1_000, 700.0, [500.0, 600.0, 400.0, 800.0]),
            row(2_000, 1_400.0, [800.0, 700.0, 600.0, 900.0]),
            row(4_000, 2_800.0, [1_000.0, 1_500.0, 1_400.0, 1_500.0]),
        ];
        assert_eq!(start_level(&mixed, Some(1_000)), 2);
        let tie = vec![row(1_000, 700.0, [500.0, 600.0, 400.0, 800.0]), row(2_000, 1_400.0, [800.0, 700.0, 900.0, 900.0])];
        assert_eq!(start_level(&tie, Some(1_000)), 1, "one vote each: the bigger size's");
        assert_eq!(start_level(&rows, None), 0, "no crossover, no level");
    }

    #[test]
    fn the_workers_are_the_most_that_are_within_twelve_percent_of_the_fastest() {
        let at = |workers, ns| WorkerRow { workers, level: 1, ns };
        // Flat from 4 on (bandwidth-bound): every one of 4, 6, 8, 12 is within 12 %: use 12.
        let flat = [at(2, 1_500.0), at(3, 1_200.0), at(4, 1_005.0), at(6, 1_000.0), at(8, 1_001.0), at(12, 1_010.0)];
        assert_eq!(pick_workers(&flat), 12);
        // The measured shapes of a 6-core machine, four runs (2, 6 or 12 workers the fastest, by noise): 12 every time.
        let first = [at(2, 2_108.0), at(3, 1_987.0), at(4, 2_001.0), at(6, 1_931.0), at(8, 1_968.0), at(12, 1_886.0)];
        let second = [at(2, 1_897.0), at(3, 2_144.0), at(4, 2_181.0), at(6, 2_061.0), at(8, 2_073.0), at(12, 2_063.0)];
        let third = [at(2, 4_285.0), at(3, 3_983.0), at(4, 3_910.0), at(6, 3_925.0), at(8, 3_855.0), at(12, 3_457.0)];
        let fourth = [at(2, 4_995.0), at(3, 4_777.0), at(4, 4_595.0), at(6, 4_151.0), at(8, 4_448.0), at(12, 4_552.0)];
        assert_eq!([&first, &second, &third, &fourth].map(|run| pick_workers(run)), [12, 12, 12, 12]);
        // The last ones lose (hyper-threads, efficiency cores) by more than the noise: the best count that is not slower.
        let loses = [at(2, 1_500.0), at(4, 1_000.0), at(6, 1_200.0), at(8, 1_300.0), at(12, 1_400.0)];
        assert_eq!(pick_workers(&loses), 4);
        assert_eq!(pick_workers(&[]), 1);
    }

    #[test]
    fn the_numbers_of_workers_tried_are_every_one_up_to_four_then_the_round_ones_and_the_machines_own() {
        assert_eq!(worker_candidates(1), Vec::<usize>::new());
        assert_eq!(worker_candidates(2), [2]);
        assert_eq!(worker_candidates(4), [2, 3, 4]);
        assert_eq!(worker_candidates(6), [2, 3, 4, 6]);
        assert_eq!(worker_candidates(12), [2, 3, 4, 6, 8, 12]);
        assert_eq!(worker_candidates(20), [2, 3, 4, 6, 8, 12, 16, 20]);
        assert_eq!(worker_candidates(64), [2, 3, 4, 6, 8, 12, 16, 24, 32, 64]);
    }

    #[test]
    fn a_calibration_reads_back_as_it_was_written() {
        let rows = desktop();
        let size = crossover(&rows).unwrap();
        let calibration = Calibration {
            threads: 12,
            workers: 8,
            pool_bibites: size,
            start_level: start_level(&rows, Some(size)),
            speedup: 2.123,
            noise: 0.043,
            seconds: 31.4,
            when: 1_790_000_000,
            worker_rows: vec![WorkerRow { workers: 4, level: 1, ns: 1_000.0 }, WorkerRow { workers: 8, level: 2, ns: 900.0 }],
            rows,
        };
        let back = Calibration::from_text(&calibration.to_text()).expect("it reads");
        assert_eq!((back.threads, back.workers, back.pool_bibites, back.start_level, back.when), (12, 8, size, 1, 1_790_000_000));
        assert!((back.speedup - 2.123).abs() < 1e-3 && (back.noise - 0.043).abs() < 1e-3 && (back.seconds - 31.4).abs() < 0.05);
        assert_eq!(back.worker_rows, calibration.worker_rows);
        assert_eq!(back.rows.len(), calibration.rows.len());
        assert_eq!(back.rows[3].ns, calibration.rows[3].ns);
        // What it hands the process: the pool's guess, bodies in the tuner's ratio.
        let profile = back.profile();
        assert_eq!((profile.workers, profile.pool_bibites, profile.start_level), (8, size, 1));
        assert_eq!(profile.pool_bodies, (size as f64 * 2.5).round() as usize);
    }

    #[test]
    fn a_text_that_is_not_a_calibration_is_an_error_not_a_guess() {
        assert!(Calibration::from_text("").is_err());
        assert!(Calibration::from_text("hello\nthreads=4\n").is_err());
        assert!(Calibration::from_text(&format!("{MAGIC}\nthreads=4\n")).is_err(), "the numbers are missing");
        let good = Calibration { threads: 4, workers: 4, pool_bibites: 900, start_level: 0, speedup: 1.0, noise: 0.0, seconds: 1.0, when: 1, worker_rows: vec![], rows: vec![] };
        let text = good.to_text().replace("workers=4", "workers=lots");
        assert!(Calibration::from_text(&text).is_err());
        // A line this version does not know is skipped (a newer file read by an older program).
        let extra = format!("{}future=1\n", good.to_text());
        assert!(Calibration::from_text(&extra).is_ok());
    }

    #[test]
    fn a_real_tiny_calibration_runs_to_the_end_and_reports_every_step() {
        let cancel = AtomicBool::new(false);
        let mut reports = Vec::new();
        let result = run(&Config::tiny(3), &cancel, &mut |progress| reports.push(progress.clone())).expect("it finishes");
        assert_eq!(result.threads, 3);
        assert!((2..=3).contains(&result.workers), "one of the numbers of workers that were tried: {}", result.workers);
        assert_eq!(result.rows.iter().map(|row| row.bibites).collect::<Vec<_>>(), [60, 120]);
        assert!(result.rows.iter().all(|row| row.ns.iter().all(|ns| *ns > 0.0 && ns.is_finite())), "{:?}", result.rows);
        assert_eq!(result.worker_rows.iter().map(|row| row.workers).collect::<Vec<_>>(), [2, 3]);
        assert!(result.pool_bibites >= SMALLEST_CROSSOVER && result.start_level < POOLED_LEVELS as u8);
        // A step before every part: the workers, the two sizes, the working out.
        assert_eq!(reports.len(), 4, "{reports:?}");
        assert_eq!(reports[0].done, 0);
        assert_eq!(reports.last().map(|progress| (progress.done, progress.total)), Some((3, 4)));
        assert!(reports.windows(2).all(|pair| pair[0].done < pair[1].done));
    }

    #[test]
    fn a_calibration_that_is_cancelled_gives_nothing() {
        let cancel = AtomicBool::new(true);
        assert_eq!(run(&Config::tiny(2), &cancel, &mut |_| {}), Err(Stop::Cancelled));
    }

    #[test]
    fn a_machine_of_one_thread_has_nothing_to_calibrate_and_never_uses_the_pool() {
        let cancel = AtomicBool::new(false);
        let result = run(&Config::tiny(1), &cancel, &mut |_| {}).expect("it finishes at once");
        assert_eq!((result.workers, result.start_level), (1, 0));
        assert!(result.rows.is_empty());
        assert!(result.pool_bibites >= 240, "a guess far above the sizes: {}", result.pool_bibites);
    }

    #[test]
    fn the_synthetic_world_is_the_benchmarks_with_the_bibites_it_was_asked_for() {
        let world = synthetic_world(50, 7).expect("a world");
        assert_eq!(world.bibites.len(), 50);
        // The same seed gives the same world.
        let again = synthetic_world(50, 7).expect("a world");
        assert_eq!(crate::sim::state_fingerprint(&world), crate::sim::state_fingerprint(&again));
    }

    #[test]
    fn the_ways_a_calibration_times_do_not_change_what_a_world_computes() {
        // The calibration ticks one world in every way in turn; the world ends where a plain inline run ends.
        let mut inline = synthetic_world(80, 3).expect("a world");
        inline.set_parallel_gates(ParallelGates::SCALED);
        inline.set_threads(1);
        inline.run(30);
        let mut mixed = synthetic_world(80, 3).expect("a world");
        mixed.set_parallel_gates(ParallelGates::SCALED);
        mixed.set_threads(3);
        for tick in 0..30 {
            mixed.tick_in(Choice { arm: Arm::from_index(tick % ARMS), timed: false });
        }
        assert_eq!(crate::sim::state_fingerprint(&inline), crate::sim::state_fingerprint(&mixed));
    }

    #[test]
    fn a_world_takes_the_workers_and_the_guess_of_a_profile_it_is_told_about() {
        // (A profile for the process is set in `tests/machine_profile.rs`, a test of its own: the other tests make worlds.) Here the gates a profile gives.
        let profile = MachineProfile { workers: 6, pool_bibites: 700, pool_bodies: 1_750, start_level: 2 };
        let gates = ParallelGates::SCALED.with_profile(profile);
        assert_eq!((gates.pool_bibites, gates.pool_bodies, gates.start_level), (700, 1_750, 2));
        assert!(gates.adaptive, "the world still measures");
        assert_eq!(gates.sweep, ParallelGates::SCALED.sweep, "the stages' sizes are the levels'");
        let prior = gates.pool_prior();
        assert_eq!((prior.bibites, prior.bodies, prior.level), (700, 1_750, 2));
    }
}
