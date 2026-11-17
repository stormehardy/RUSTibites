//! Simulation orchestration: the statistics collector, sampling schema and
//! deterministic digests. Owner: world-dev.
//!
//! [`crate::sim::StatsSample`] is the Rust-side tick record of `docs/PARITY.md` §6.1: the
//! field groups, names and meanings match the capture schema so the CLI capture
//! emitter (`bibites-cli/src/capture.rs`) can serialize a sample directly.
//!
//! # Cadence
//!
//! `SimStats::accumulate_tick` runs every simulation tick and only touches
//! preallocated counters (no allocation, no hashing order). [`crate::sim::StatsSample`] is
//! built on demand by `World::sample_stats()` (capture ticks / logging cadence),
//! which is where the small temporary vectors (species counts, tags) are
//! allocated.

use crate::brain;
use crate::genome::{ALL_GENES, N_GENES};
use crate::settings::SimSettings;
use crate::world::World;
use rayon::prelude::*;

/// One unit of work executed by a [`ThreadPool`] worker.
pub type PoolJob = Box<dyn FnOnce() + Send + 'static>;

/// The degree of parallelism of a world, on top of rayon's persistent worker pool.
///
/// The workers are not owned by the world: they are rayon's, started once per process and kept, so a parallel stage
/// costs a wake-up (0.02-0.03 ms on the reference machine) instead of the spawn of fresh threads (0.4-0.7 ms), and the
/// core stays free of `unsafe` code. `threads` only decides into how many chunks a stage is cut: every stage cuts its
/// data by index, never by which worker is free, and merges the chunks' results in chunk order, so serial and parallel
/// runs are bit-identical whatever the size of the pool. [`ThreadPool::execute`] returns only after every submitted
/// job has finished (making the subsequent main-thread write-back deterministic).
pub struct ThreadPool {
    threads: usize,
}

impl ThreadPool {
    /// A handle that cuts stages into `threads` chunks (at least one).
    #[must_use]
    pub fn new(threads: usize) -> Self {
        Self {
            threads: threads.max(1),
        }
    }

    /// Number of chunks a stage is cut into (the configured parallelism).
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Executes every job and waits for all of them to finish (inline when there is one worker or one job).
    pub fn execute(&self, jobs: Vec<PoolJob>) {
        if jobs.is_empty() {
            return;
        }
        if self.threads <= 1 || jobs.len() == 1 {
            for job in jobs {
                job();
            }
            return;
        }
        rayon::scope(|scope| {
            for job in jobs {
                scope.spawn(move |_| job());
            }
        });
    }

    /// Maps owned chunks on the worker pool and returns results **in chunk
    /// order** (each chunk index is preserved), so callers merge
    /// deterministically.
    ///
    /// Falls back to inline mapping when there is only one chunk or one worker.
    pub fn parallel_chunks<T, R, F>(&self, chunks: Vec<Vec<T>>, f: F) -> Vec<R>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(Vec<T>) -> R + Send + Sync + 'static,
    {
        if chunks.len() <= 1 || self.threads() <= 1 {
            return chunks.into_iter().map(f).collect();
        }
        // An indexed parallel iterator collects in order, whichever worker finished first.
        chunks.into_par_iter().map(f).collect()
    }

    /// [`parallel_chunks`](Self::parallel_chunks) with a shared context that is
    /// cloned per chunk (use an `Arc` for heavy state).
    pub fn parallel_chunks_with<T, C, R, F>(&self, chunks: Vec<Vec<T>>, context: C, f: F) -> Vec<R>
    where
        T: Send + 'static,
        C: Clone + Send + Sync + 'static,
        R: Send + 'static,
        F: Fn(C, Vec<T>) -> R + Send + Sync + 'static,
    {
        self.parallel_chunks(chunks, move |chunk| f(context.clone(), chunk))
    }

    /// Splits `items` into at most `threads()` owned chunks and maps them in
    /// parallel, returning results in item order (chunk order).
    pub fn parallel_map<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(Vec<T>) -> R + Send + Sync + 'static,
    {
        let chunks = split_owned(items, self.threads());
        self.parallel_chunks(chunks, f)
    }
}

/// How the tick uses the worker pool: a choice for the whole world, made by measuring, and sizes for the stages that lose on the pool
/// even in a world that is on it.
///
/// A stage on the pool pays for it whatever it holds: waking idle workers, handing the chunks over and joining them, and the cores
/// that did not write the data fetching it from the caches of the ones that did (some 10-25 us a stage, a few us more for every
/// worker it puts to work). The stages of a small world cost less than that, so a world the pool does not pay for ticks on the calling
/// thread with no handle to the pool at all, exactly as in a world of one thread: a stage that went to the pool in such a world would
/// lose, and so would the stages around it (the data it left in other cores' caches has to come back). A world it pays for runs the
/// whole tick on one of the pool's workers and every stage that can use the pool on it (measured 2026-11-15 on a 6-core machine: from
/// about 1,200 bibites the pool wins, by 10 % there and 2.7x at 10,000), except the stages the measurements found losing in worlds of
/// a few thousand bodies: the cheap per-body stages of the physics step (10-40 ns a body), the sort of the x order from scratch (the
/// serial repair of the last step's order is cheaper unless the order has to be sorted anyway) and the scans of the per-slot
/// summaries (a few ns a slot); those have a size of their own below which they run inline.
///
/// **Where the line between the two lies is not written down, it is measured** (`adaptive`, [`crate::pool_tuner::PoolTuner`]): it
/// depends on the machine (cores, how fast the OS wakes one, what else is running) as much as on the world, and a size that is right
/// for a 6-core desktop is wrong for a 2-core laptop and for a 32-core workstation (on a pool of two or four workers the cheap stages pay from
/// a few hundred bibites on, on twelve they do not pay below a few thousand). The world times its own ticks, inline and on the pool at four
/// levels of eagerness ([`ParallelGates::pooled_level`]: the sizes of the stages times 1, 0.3, 0.08, and every size at 1), keeps the fastest,
/// and looks at the others now and then. `pool_bibites` and `pool_bodies` are only its first guess (the
/// sizes from which the pool paid on the machine they were measured on) and, divided by four, the floor below which no pool pays and
/// nothing is timed. Nothing a stage computes depends on how it runs (the tests compare whole checkpoints byte for byte, serial against
/// the pool, with every size at 1, and in worlds that change from one way to the other tick by tick); only the time does.
///
/// [`ParallelGates::SCALED`] is what a world starts with; [`ParallelGates::FIXED`] has the same sizes and no measuring: the world is on
/// the pool when it is at least that big (for the tests and for comparing); [`ParallelGates::ALWAYS`] puts every stage of every world on
/// the pool from one item (the benchmarks' reference for what that costs: a world of a few hundred bibites ticks several times slower
/// than on one thread); [`ParallelGates::EAGER`] also puts the resting-pellet layer on from one pellet (for the tests).
/// `RUSTIBITES_GATES` picks one of them (see [`ParallelGates::from_mode`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParallelGates {
    /// Whether the world measures its own ticks to choose between inline and the pool ([`crate::pool_tuner::PoolTuner`]):
    /// `pool_bibites` and `pool_bodies` are then only its first guess. Off: the world is on the pool when it has that many bibites or
    /// bodies, and the sizes are a plain switch.
    pub adaptive: bool,
    /// Live bibites from which the pool is expected to pay (or [`ParallelGates::pool_bodies`] bodies): the first guess of a measuring
    /// world, and, divided by four, its floor; with `adaptive` off, the switch.
    pub pool_bibites: usize,
    /// Bodies (bibites, pellets and eggs) from which the pool is expected to pay (or [`ParallelGates::pool_bibites`] bibites).
    pub pool_bodies: usize,
    /// Record slots from which the body sweep (pass A, the brain steps and the part of pass B that touches nothing but the bibite,
    /// in one pass over the records: 0.3-0.45 us a bibite) runs on the pool, in a world that is on it.
    pub sweep: usize,
    /// Live bibites from which the physics bridge builds their bodies on the pool, keeping the bodies of the last step (0.1 us a
    /// bibite), and scans their records.
    pub records: usize,
    /// Bodies (bibites, pellets and eggs) from which the physics step's cheap per-body stages (mirror, poses, free motion,
    /// write-back: 10-40 ns a body), the pellets' bodies, the resting pellets' promotion and the bridge's work after the step use
    /// the pool.
    pub bodies: usize,
    /// Bodies in the broadphase from which its sweep (about 0.06 us a body) runs on the pool, cut by first entry.
    pub broadphase: usize,
    /// Candidate pairs from which the narrowphase (about 0.25 us a pair) runs on the pool.
    pub narrow_pairs: usize,
    /// Due bibites from which the stages that copy a sensing or brain result into the records (or its inputs out of them) run on
    /// the pool.
    pub due_brains: usize,
    /// Due brains from which a brain batch is split (0.3-1 us a brain).
    pub brains: usize,
    /// Due bibites from which sensing (about 1 us a bibite) runs on the pool.
    pub sensing: usize,
    /// Record slots from which the scans of the store's per-slot summaries (a few ns a slot) run on the pool.
    pub scans: usize,
    /// Bodies and boxes (and, twice as many, candidate pairs) from which the physics step's sorts and column copies run on the
    /// pool. The x order is sorted from scratch there instead of repaired from the last step's, which the pool only beats in a big
    /// world, or in one where the repair gives up (the number of bodies changes every step).
    pub sorts: usize,
    /// Bodies held by near contacts from which the physics step's islands of fine steps use the pool (with sixteen islands or
    /// more: the pool takes the islands eight to a chunk).
    pub island_bodies: usize,
    /// Contact constraints from which the physics step's islands of the soft step use the pool (with sixteen islands or more).
    pub island_constraints: usize,
    /// Pellets from which resting pellets skip the physics step (`usize::MAX`: never). Unlike the other gates this one is not
    /// about the pool: it works with one thread too.
    pub sleep_pellets: usize,
}

impl ParallelGates {
    /// Every parallel stage on from one item, and the resting-pellet layer from one pellet (for the tests that compare it with
    /// the serial one, and run the pooled paths in small worlds).
    pub const EAGER: Self = Self {
        adaptive: false,
        pool_bibites: 1,
        pool_bodies: 1,
        sweep: 1,
        records: 1,
        bodies: 1,
        broadphase: 1,
        narrow_pairs: 1,
        due_brains: 1,
        brains: 1,
        sensing: 1,
        scans: 1,
        sorts: 1,
        island_bodies: 1,
        island_constraints: 1,
        sleep_pellets: 1,
    };

    /// Every stage that can use the pool uses it from one item, whatever the work: the reference the benchmarks compare
    /// [`ParallelGates::SCALED`] with. The resting-pellet layer keeps the size from which it pays (it is not about the pool).
    pub const ALWAYS: Self = Self {
        sleep_pellets: Self::SCALED.sleep_pellets,
        ..Self::EAGER
    };

    /// What a world starts with: it measures its own ticks to choose between inline and the pool, starting from the world sizes
    /// from which the pool paid on the machine this was measured on (a 6-core desktop), and the sizes of the stages that lose on the
    /// pool in a world that is on it, from the timings of the synthetic worlds of 100 to 10,000 bibites (`docs/BENCHMARKS.md`,
    /// "Multithreading sized to the work").
    pub const SCALED: Self = Self {
        adaptive: true,
        pool_bibites: 1_200,
        pool_bodies: 3_000,
        sweep: 256,
        records: 256,
        bodies: 3_000,
        broadphase: 512,
        narrow_pairs: 64,
        due_brains: 1_500,
        brains: 128,
        sensing: 96,
        scans: 8_192,
        sorts: 4_096,
        island_bodies: 256,
        island_constraints: 512,
        sleep_pellets: 2_048,
    };

    /// The same sizes with no measuring: a world is on the pool when it has that many bibites or bodies (the way the tests and the
    /// comparisons with the measuring world have it: the choice does not depend on timings).
    pub const FIXED: Self = Self {
        adaptive: false,
        ..Self::SCALED
    };

    /// The gates a benchmark's mode name stands for (any case): `scaled`, `scaled:<percent>` (the sizes of `scaled` times that
    /// percentage, to find where a stage starts to pay), `fixed` (`scaled` without the measuring), `pooled:<level>` (every world on the
    /// pool at that level of eagerness, 0 to 3, without measuring: `pooled:3` is `always`), `always`, `eager`, or `nosleep`
    /// (`scaled` with the resting-pellet layer off, the reference the layer is checked against). `None` for any other name.
    #[must_use]
    pub fn from_mode(mode: &str) -> Option<Self> {
        let is = |name: &str| mode.eq_ignore_ascii_case(name);
        if is("scaled") {
            Some(Self::SCALED)
        } else if is("fixed") {
            Some(Self::FIXED)
        } else if is("always") {
            Some(Self::ALWAYS)
        } else if is("eager") {
            Some(Self::EAGER)
        } else if is("nosleep") {
            Some(Self::SCALED.without_sleep())
        } else if let Some(level) = mode
            .to_ascii_lowercase()
            .strip_prefix("pooled:")
            .and_then(|level| level.parse::<u8>().ok())
            .filter(|level| usize::from(*level) < crate::pool_tuner::POOLED_LEVELS)
        {
            // Every world on the pool at one level of eagerness, no measuring: what a level costs on a machine, one by one.
            Some(Self {
                pool_bibites: 1,
                pool_bodies: 1,
                ..Self::FIXED.pooled_level(level)
            })
        } else {
            let (name, percent) = mode.split_once(':')?;
            if !name.eq_ignore_ascii_case("scaled") {
                return None;
            }
            percent
                .parse::<usize>()
                .ok()
                .filter(|percent| *percent > 0)
                .map(|percent| Self::SCALED.scaled_by(percent))
        }
    }

    /// These gates with every size of the pool multiplied by `percent` / 100 (none below 1; the resting-pellet layer is not about
    /// the pool and stays).
    #[must_use]
    pub fn scaled_by(self, percent: usize) -> Self {
        let scale = |size: usize| (size.saturating_mul(percent) / 100).max(1);
        Self {
            adaptive: self.adaptive,
            pool_bibites: scale(self.pool_bibites),
            pool_bodies: scale(self.pool_bodies),
            sweep: scale(self.sweep),
            records: scale(self.records),
            bodies: scale(self.bodies),
            broadphase: scale(self.broadphase),
            narrow_pairs: scale(self.narrow_pairs),
            due_brains: scale(self.due_brains),
            brains: scale(self.brains),
            sensing: scale(self.sensing),
            scans: scale(self.scans),
            sorts: scale(self.sorts),
            island_bodies: scale(self.island_bodies),
            island_constraints: scale(self.island_constraints),
            sleep_pellets: self.sleep_pellets,
        }
    }

    /// These gates with resting pellets switched off: the step sees every pellet, every tick (the reference the tests compare
    /// the sleeping layer with).
    #[must_use]
    pub const fn without_sleep(mut self) -> Self {
        self.sleep_pellets = usize::MAX;
        self
    }

    /// The workers a stage of `items` items puts to work, of a pool of `pool`, when it pays for the pool from `from` items: none
    /// (1: the stage runs inline on the calling thread) below `from`, or in a world with one thread; from `from` a worker for every
    /// quarter of `from` items (four at `from`, so every chunk holds at least a quarter of the size at which the pool pays), but
    /// two at least and the pool at most. A `from` of 0 counts as 1.
    #[must_use]
    pub fn workers_for(items: usize, from: usize, pool: usize) -> usize {
        let from = from.max(1);
        if pool < 2 || items < from {
            1
        } else {
            (items.saturating_mul(4) / from).clamp(2, pool)
        }
    }

    /// The length of the chunks a stage of `items` items that pays for the pool from `from` puts `workers` to work on:
    /// `per_worker` chunks for each worker (a few more than one balance the load), none shorter than a quarter of `from` (and
    /// never 0).
    #[must_use]
    pub fn chunk_len(items: usize, from: usize, workers: usize, per_worker: usize) -> usize {
        items
            .div_ceil(workers.saturating_mul(per_worker).max(1))
            .max(from.div_ceil(4))
            .max(1)
    }
}

impl ParallelGates {
    /// The gates `RUSTIBITES_GATES` names (see [`ParallelGates::from_mode`]; read once per process), [`ParallelGates::SCALED`] when it
    /// is not set or names nothing. The one switch for the CLI, the viewer and the worlds of the tests.
    #[must_use]
    pub fn from_environment() -> Self {
        static GATES: std::sync::OnceLock<ParallelGates> = std::sync::OnceLock::new();
        *GATES.get_or_init(|| {
            std::env::var("RUSTIBITES_GATES")
                .ok()
                .and_then(|mode| Self::from_mode(&mode))
                .unwrap_or(Self::SCALED)
        })
    }

    /// Whether a world with `bibites` bibites and `bodies` bodies is on the pool by its size alone: the whole tick then runs on one
    /// of the pool's workers (from there a stage's hand-off to the other workers costs about half of what it costs from a thread
    /// outside the pool). Below it the tick runs on the calling thread with no handle to the pool at all: every stage is inline. This
    /// is the whole choice of a world with `adaptive` off, and the first guess of one that measures.
    #[must_use]
    pub fn pool_has_work_for(&self, bibites: usize, bodies: usize) -> bool {
        bibites >= self.pool_bibites.max(1) || bodies >= self.pool_bodies.max(1)
    }

    /// The gates of one level of the pool, for a world that measures ([`crate::pool_tuner::PoolTuner`]): level 0 is these gates (the sizes measured on the
    /// reference machine), each next level has smaller sizes (30 %, 8 %), so more of the stages of a tick on the pool are worth a hand-off, and the last one has
    /// every size at 1 (every stage on the pool from one item, as [`ParallelGates::ALWAYS`]). How eagerly the stages should use the pool depends on the machine
    /// (what a hand-off costs there, how many workers it wakes) as much as the size from which the pool pays at all does, so the world measures the levels too.
    #[must_use]
    pub fn pooled_level(&self, level: u8) -> Self {
        match level {
            0 => *self,
            1 => self.scaled_by(30),
            2 => self.scaled_by(8),
            _ => Self {
                adaptive: self.adaptive,
                pool_bibites: self.pool_bibites,
                pool_bodies: self.pool_bodies,
                sleep_pellets: self.sleep_pellets,
                ..Self::EAGER
            },
        }
    }

    /// The first guess of a world that measures ([`crate::pool_tuner::PoolTuner`]): the sizes from which the pool is expected to pay.
    #[must_use]
    pub fn pool_prior(&self) -> crate::pool_tuner::Prior {
        crate::pool_tuner::Prior {
            bibites: self.pool_bibites.max(1),
            bodies: self.pool_bodies.max(1),
        }
    }
}

impl Default for ParallelGates {
    /// [`ParallelGates::SCALED`]; `RUSTIBITES_GATES` picks other gates (see [`ParallelGates::from_environment`]).
    fn default() -> Self {
        Self::from_environment()
    }
}

/// Splits an owned vector into at most `count` contiguous chunks (chunk order
/// preserved; empty input yields no chunks).
fn split_owned<T>(items: Vec<T>, count: usize) -> Vec<Vec<T>> {
    let len = items.len();
    if len == 0 {
        return Vec::new();
    }
    let count = count.clamp(1, len);
    let base = len / count;
    let extra = len % count;
    let mut iter = items.into_iter();
    (0..count)
        .map(|index| {
            let take = base + usize::from(index < extra);
            iter.by_ref().take(take).collect()
        })
        .collect()
}

/// Serial/parallel execution handle for a [`World`].
///
/// Obtained from [`World::executor`]. In serial worlds every method runs inline
/// (no threads, bit-identical order); in parallel worlds the work is mapped onto
/// the world-owned persistent [`ThreadPool`] with results returned in chunk
/// order. This is the API body/physics code should use so that no stage spawns
/// threads of its own.
#[derive(Clone, Copy)]
pub enum ParallelExecutor<'a> {
    /// One thread: run everything inline.
    Serial,
    /// Persistent worker pool (borrowed from the [`World`]).
    Pool(&'a ThreadPool),
}

impl<'a> ParallelExecutor<'a> {
    /// Builds a handle from an optional pool (`None` = serial).
    #[must_use]
    pub fn new(pool: Option<&'a ThreadPool>) -> Self {
        match pool {
            Some(pool) => Self::Pool(pool),
            None => Self::Serial,
        }
    }

    /// Number of workers (1 in serial mode).
    #[must_use]
    pub fn threads(&self) -> usize {
        match self {
            Self::Serial => 1,
            Self::Pool(pool) => pool.threads(),
        }
    }

    /// `true` when work is actually dispatched to the pool.
    #[must_use]
    pub fn is_parallel(&self) -> bool {
        matches!(self, Self::Pool(_))
    }

    /// Number of chunks to split `len` items into (1 in serial mode).
    #[must_use]
    pub fn chunk_count(&self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        match self {
            Self::Serial => 1,
            Self::Pool(pool) => pool.threads().clamp(1, len),
        }
    }

    /// Maps owned chunks with deterministic chunk-order results.
    ///
    /// # Panics
    ///
    /// Panics if a pool chunk job is lost (worker panic).
    pub fn parallel_chunks<T, R, F>(&self, chunks: Vec<Vec<T>>, f: F) -> Vec<R>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(Vec<T>) -> R + Send + Sync + 'static,
    {
        match self {
            Self::Serial => chunks.into_iter().map(f).collect(),
            Self::Pool(pool) => pool.parallel_chunks(chunks, f),
        }
    }

    /// [`parallel_chunks`](Self::parallel_chunks) with a per-chunk cloned
    /// context.
    pub fn parallel_chunks_with<T, C, R, F>(&self, chunks: Vec<Vec<T>>, context: C, f: F) -> Vec<R>
    where
        T: Send + 'static,
        C: Clone + Send + Sync + 'static,
        R: Send + 'static,
        F: Fn(C, Vec<T>) -> R + Send + Sync + 'static,
    {
        match self {
            Self::Serial => chunks
                .into_iter()
                .map(|chunk| f(context.clone(), chunk))
                .collect(),
            Self::Pool(pool) => pool.parallel_chunks_with(chunks, context, f),
        }
    }

    /// Maps `items` in parallel, returning results in item order.
    pub fn parallel_map<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send + 'static,
        R: Send + 'static,
        F: Fn(Vec<T>) -> R + Send + Sync + 'static,
    {
        let count = self.chunk_count(items.len());
        let chunks = split_owned(items, count);
        match self {
            Self::Serial => chunks.into_iter().map(f).collect(),
            Self::Pool(pool) => pool.parallel_chunks(chunks, f),
        }
    }
}

impl<'a> ParallelExecutor<'a> {
    /// Runs `f(chunk_index, &mut chunk)` over disjoint contiguous chunks of a
    /// borrowed mutable slice.
    ///
    /// Needed by the body/physics bridge, whose records cannot be moved into
    /// `'static` pool jobs. In serial worlds the chunks run inline in order; in
    /// parallel worlds they run on rayon's persistent workers (which borrow the
    /// slice safely), and results are returned in chunk order (deterministic merge).
    pub fn parallel_borrow_chunks<T, F>(&self, items: &mut [T], f: F)
    where
        T: Send,
        F: Fn(usize, &mut [T]) + Sync,
    {
        let threads = self.chunk_count(items.len());
        if threads <= 1 {
            if !items.is_empty() {
                f(0, items);
            }
            return;
        }
        let chunk_len = items.len().div_ceil(threads);
        items
            .par_chunks_mut(chunk_len)
            .enumerate()
            .for_each(|(index, chunk)| f(index, chunk));
    }

    /// `parallel_borrow_chunks` with a per-chunk result, returned in chunk order.
    pub fn parallel_borrow_chunks_map<T, R, F>(&self, items: &mut [T], f: F) -> Vec<R>
    where
        T: Send,
        R: Send,
        F: Fn(usize, &mut [T]) -> R + Sync,
    {
        let threads = self.chunk_count(items.len());
        if threads <= 1 {
            if items.is_empty() {
                return Vec::new();
            }
            return vec![f(0, items)];
        }
        let chunk_len = items.len().div_ceil(threads);
        items
            .par_chunks_mut(chunk_len)
            .enumerate()
            .map(|(index, chunk)| f(index, chunk))
            .collect()
    }
}

/// One measured phase of `World::tick`.
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Fixed clock + simulated time.
    Clock = 0,
    /// Settings animators (0.1 s manager).
    Animators,
    /// Tool positioning + pheromone-tower spots.
    Tools,
    /// Zone movement/production/recycling.
    Zones,
    /// World sensing (FOV + pheromone) and `set_senses`.
    Sensing,
    /// Body tick (metabolism/brain/organs).
    Body,
    /// Egg incubation/hatch/abort.
    Eggs,
    /// Pheromone dissipation.
    Pheromones,
    /// Pellet decay.
    Decay,
    /// Rad towers.
    Rad,
    /// Red Death.
    RedDeath,
    /// Colour killers.
    ColourKillers,
    /// Pellet producers.
    Producers,
    /// World-inflicted death resolution.
    Deaths,
    /// Physics bridge (`BibiteStore::step_physics`).
    Physics,
    /// Body events + species/lifecycle/stats accumulation.
    Events,
    /// Spatial-hash reconciliation.
    HashSync,
}

/// Number of measured phases.
pub const PHASE_COUNT: usize = 17;

impl Phase {
    /// All phases in tick order.
    pub const ALL: [Phase; PHASE_COUNT] = [
        Phase::Clock,
        Phase::Animators,
        Phase::Tools,
        Phase::Zones,
        Phase::Sensing,
        Phase::Body,
        Phase::Eggs,
        Phase::Pheromones,
        Phase::Decay,
        Phase::Rad,
        Phase::RedDeath,
        Phase::ColourKillers,
        Phase::Producers,
        Phase::Deaths,
        Phase::Physics,
        Phase::Events,
        Phase::HashSync,
    ];

    /// Stable display name (CLI bench breakdown).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Phase::Clock => "clock",
            Phase::Animators => "animators",
            Phase::Tools => "tools",
            Phase::Zones => "zones",
            Phase::Sensing => "sensing",
            Phase::Body => "body",
            Phase::Eggs => "eggs",
            Phase::Pheromones => "pheromones",
            Phase::Decay => "decay",
            Phase::Rad => "rad",
            Phase::RedDeath => "red-death",
            Phase::ColourKillers => "colour-killers",
            Phase::Producers => "producers",
            Phase::Deaths => "deaths",
            Phase::Physics => "physics",
            Phase::Events => "events-stats",
            Phase::HashSync => "hash-sync",
        }
    }
}

/// A timed stage inside the body and physics phases (observational only, like the phases).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Body: `resolve_pending_deaths`.
    BodyDeaths = 0,
    /// Body: pass A (energy, cadence, due list, starvation).
    BodyPassA,
    /// Body: the due brains' inputs.
    BodyAssemble,
    /// Body: the brain batch.
    BodyBrains,
    /// Body, on the worker pool: pass A, the brain steps and the part of pass B that touches nothing but the bibite itself,
    /// record block by record block (the serial path times these as pass A, assemble, brains and part of pass B).
    BodySweep,
    /// Body: pass B (organs, mouth, movement) in dense order (with the parallel part: the bibites that touch shared state).
    BodyPassB,
    /// Body: bites, swallows and deaths resolved after pass B.
    BodyTails,
    /// Physics bridge: one physics body per bibite.
    PhysBuild,
    /// Physics bridge: one physics body per pellet.
    PhysBuildPellets,
    /// Physics bridge: one physics body per egg, and the held pellets following their holders' mouths.
    PhysBuildEggs,
    /// Physics step: the solver mirror copy.
    PhysMirror,
    /// Physics step: velocity integration.
    PhysIntegrate,
    /// Physics step: one merged AABB per body.
    PhysAabb,
    /// Physics step: the x order and the broadphase entries.
    PhysOrder,
    /// Physics step: the broadphase's candidate pairs.
    PhysBroadphase,
    /// Physics step: the candidate pairs' sort.
    PhysPairSort,
    /// Physics step: the colliders' poses and AABBs.
    PhysPoses,
    /// Physics step: the narrowphase.
    PhysNarrow,
    /// Physics step: the velocity and position solvers.
    PhysSolve,
    /// Physics step: publishing impulses and writing the bodies back.
    PhysWriteBack,
    /// Physics bridge: contact bookkeeping, mouth triggers, collision damage and the pass after the step.
    PhysBridge,
}

/// Number of timed stages.
pub const STAGE_COUNT: usize = 21;

impl Stage {
    /// All stages in tick order.
    pub const ALL: [Stage; STAGE_COUNT] = [
        Stage::BodyDeaths,
        Stage::BodyPassA,
        Stage::BodyAssemble,
        Stage::BodyBrains,
        Stage::BodySweep,
        Stage::BodyPassB,
        Stage::BodyTails,
        Stage::PhysBuild,
        Stage::PhysBuildPellets,
        Stage::PhysBuildEggs,
        Stage::PhysMirror,
        Stage::PhysIntegrate,
        Stage::PhysAabb,
        Stage::PhysOrder,
        Stage::PhysBroadphase,
        Stage::PhysPairSort,
        Stage::PhysPoses,
        Stage::PhysNarrow,
        Stage::PhysSolve,
        Stage::PhysWriteBack,
        Stage::PhysBridge,
    ];

    /// The phase this stage belongs to.
    #[must_use]
    pub fn phase(self) -> Phase {
        match self {
            Stage::BodyDeaths
            | Stage::BodyPassA
            | Stage::BodyAssemble
            | Stage::BodyBrains
            | Stage::BodySweep
            | Stage::BodyPassB
            | Stage::BodyTails => Phase::Body,
            _ => Phase::Physics,
        }
    }

    /// Short name for tables.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Stage::BodyDeaths => "deaths",
            Stage::BodyPassA => "pass-a",
            Stage::BodyAssemble => "assemble",
            Stage::BodyBrains => "brains",
            Stage::BodySweep => "sweep",
            Stage::BodyPassB => "pass-b",
            Stage::BodyTails => "tails",
            Stage::PhysBuild => "build-bibites",
            Stage::PhysBuildPellets => "build-pellets",
            Stage::PhysBuildEggs => "build-eggs+held",
            Stage::PhysMirror => "mirror",
            Stage::PhysIntegrate => "integrate-vel",
            Stage::PhysAabb => "aabb",
            Stage::PhysOrder => "x-order",
            Stage::PhysBroadphase => "broadphase",
            Stage::PhysPairSort => "pair-sort",
            Stage::PhysPoses => "poses",
            Stage::PhysNarrow => "narrowphase",
            Stage::PhysSolve => "solve",
            Stage::PhysWriteBack => "write-back",
            Stage::PhysBridge => "bridge-after",
        }
    }
}

/// Nanoseconds accumulated per [`Stage`] by the code that runs it; the world moves them into its [`PhaseTimings`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageClock {
    /// Nanoseconds per stage (index = stage as usize).
    pub nanos: [u64; STAGE_COUNT],
}

impl StageClock {
    /// Adds the time since `since` to `stage` and restarts the lap.
    #[inline]
    pub fn lap(&mut self, stage: Stage, since: &mut std::time::Instant) {
        let now = std::time::Instant::now();
        let slot = &mut self.nanos[stage as usize];
        *slot = slot.saturating_add(now.duration_since(*since).as_nanos() as u64);
        *since = now;
    }

    /// Moves the accumulated times into `timings` and zeroes them.
    pub fn drain_into(&mut self, timings: &mut PhaseTimings) {
        for (total, taken) in timings.stage_nanos.iter_mut().zip(self.nanos.iter_mut()) {
            *total = total.saturating_add(*taken);
            *taken = 0;
        }
    }
}

/// Accumulated per-phase wall time (nanoseconds): small, `Copy`, no allocation.
///
/// Always-on measurement (17 `Instant` pairs per tick, sub-microsecond) so the
/// CLI bench breakdown is always available. Timings are observational only and
/// never affect simulation state or determinism.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhaseTimings {
    /// Nanoseconds accumulated per [`Phase`] (index = phase as usize).
    pub nanos: [u64; PHASE_COUNT],
    /// Number of ticks measured.
    pub ticks: u64,
    /// Nanoseconds accumulated per [`Stage`] inside the body and physics phases (index = stage as usize).
    pub stage_nanos: [u64; STAGE_COUNT],
}

impl PhaseTimings {
    /// Creates zeroed timings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a phase duration.
    #[inline]
    pub fn add(&mut self, phase: Phase, nanos: u64) {
        self.nanos[phase as usize] = self.nanos[phase as usize].saturating_add(nanos);
    }

    /// Marks one completed tick.
    #[inline]
    pub fn tick_complete(&mut self) {
        self.ticks += 1;
    }

    /// Nanoseconds accumulated for `phase`.
    #[must_use]
    pub fn nanos(&self, phase: Phase) -> u64 {
        self.nanos[phase as usize]
    }

    /// Total accumulated nanoseconds across all phases.
    #[must_use]
    pub fn total_nanos(&self) -> u64 {
        self.nanos.iter().sum()
    }

    /// Mean nanoseconds per tick for `phase` (0 before the first tick).
    #[must_use]
    pub fn mean_nanos(&self, phase: Phase) -> f64 {
        if self.ticks == 0 {
            0.0
        } else {
            self.nanos(phase) as f64 / self.ticks as f64
        }
    }

    /// Mean nanoseconds per tick for `stage` (0 before the first tick).
    #[must_use]
    pub fn mean_stage_nanos(&self, stage: Stage) -> f64 {
        if self.ticks == 0 {
            0.0
        } else {
            self.stage_nanos[stage as usize] as f64 / self.ticks as f64
        }
    }

    /// Share of total measured time for `phase` in `[0, 1]`.
    #[must_use]
    pub fn share(&self, phase: Phase) -> f64 {
        let total = self.total_nanos();
        if total == 0 {
            0.0
        } else {
            self.nanos(phase) as f64 / total as f64
        }
    }

    /// Resets every accumulator.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Population counts (`counts` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CountStats {
    /// Live bibites (including corpses still in the body store).
    pub bibites: u32,
    /// Eggs waiting to hatch.
    pub eggs: u32,
    /// Dead bibites (`is_corpse`).
    pub corpses: u32,
}

/// Pellet counters and biomass (`pellets` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PelletStats {
    /// Live plant pellets.
    pub plant_n: u32,
    /// Total plant pellet energy.
    pub plant_biomass: f32,
    /// Live meat pellets.
    pub meat_n: u32,
    /// Total meat pellet energy.
    pub meat_biomass: f32,
    /// Pellet energy not owned by a zone.
    pub free_biomass: f32,
    /// Total energy held in living bibites.
    pub bibite_biomass: f32,
}

/// Living-population vitals (`vitals` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VitalStats {
    /// Mean active energy (E).
    pub energy_mean: f32,
    /// Mean health (HP).
    pub health_mean: f32,
    /// Mean age in seconds.
    pub age_mean: f32,
    /// Maximum age in seconds.
    pub age_max: f32,
}

/// Generation statistics (`generation` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GenerationStats {
    /// Maximum generation.
    pub gen_max: u32,
    /// Mean generation.
    pub gen_mean: f32,
}

/// Active species counts (`species` group).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeciesStats {
    /// Number of distinct active species.
    pub n: u32,
    /// `(species id, bibite count)` sorted by species id.
    pub counts: Vec<(u32, u32)>,
}

/// Brain size statistics (`brain` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrainStats {
    /// Mean node count.
    pub nodes_mean: f32,
    /// Maximum node count.
    pub nodes_max: u32,
    /// Mean synapse count.
    pub syn_mean: f32,
    /// Maximum synapse count.
    pub syn_max: u32,
}

/// Per-gene population statistics (`genes` group), in game gene-enum order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneStats {
    /// Mean of each gene.
    pub mean: [f32; N_GENES],
    /// Population standard deviation of each gene.
    pub std: [f32; N_GENES],
}

impl Default for GeneStats {
    fn default() -> Self {
        Self {
            mean: [0.0; N_GENES],
            std: [0.0; N_GENES],
        }
    }
}

/// Birth/death counts since the previous sample (`lifecycle` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LifecycleStats {
    /// Births (hatched/spawned bibites) since the previous sample.
    pub births: u64,
    /// Deaths since the previous sample.
    pub deaths: u64,
}

/// Behaviour statistics (`behavior` group).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BehaviorStats {
    /// Mean forward speed (u/s).
    pub speed_mean: f32,
    /// Maximum forward speed (u/s).
    pub speed_max: f32,
    /// Fraction of samples with non-zero speed.
    pub moving_frac: f32,
    /// Mean distance travelled per second.
    pub travel_rate_mean: f32,
    /// Legacy bite rate (`bibitesBitten / age`; not a feeding metric).
    pub bite_rate_mean: f32,
    /// Damage dealt per second.
    pub damage_dealt_rate_mean: f32,
    /// Egg-laying events per second.
    pub egg_rate_mean: f32,
    /// Mean activation of the 15 brain outputs.
    pub outputs: [f32; brain::N_OUTPUTS],
    /// Plant-pellet bite events per live-bibite second.
    pub plant_bites_per_s: f64,
    /// Meat-pellet bite events per live-bibite second.
    pub meat_bites_per_s: f64,
    /// First bites on other bibites per live-bibite second.
    pub bibite_bites_per_s: f64,
    /// Bite damage dealt to bibites per live-bibite second.
    pub bibite_damage_per_s: f64,
    /// Plant matter consumed (u²) per live-bibite second.
    pub plant_consumed_u2_per_s: f64,
    /// Meat matter consumed (u²) per live-bibite second.
    pub meat_consumed_u2_per_s: f64,
    /// Plant energy consumed (E) per live-bibite second.
    pub plant_consumed_e_per_s: f64,
    /// Meat energy consumed (E) per live-bibite second.
    pub meat_consumed_e_per_s: f64,
    /// Stomach energy gain (E) per live-bibite second.
    pub stomach_energy_gain_per_s: f64,
}

impl Default for BehaviorStats {
    fn default() -> Self {
        Self {
            speed_mean: 0.0,
            speed_max: 0.0,
            moving_frac: 0.0,
            travel_rate_mean: 0.0,
            bite_rate_mean: 0.0,
            damage_dealt_rate_mean: 0.0,
            egg_rate_mean: 0.0,
            outputs: [0.0; brain::N_OUTPUTS],
            plant_bites_per_s: 0.0,
            meat_bites_per_s: 0.0,
            bibite_bites_per_s: 0.0,
            bibite_damage_per_s: 0.0,
            plant_consumed_u2_per_s: 0.0,
            meat_consumed_u2_per_s: 0.0,
            plant_consumed_e_per_s: 0.0,
            meat_consumed_e_per_s: 0.0,
            stomach_energy_gain_per_s: 0.0,
        }
    }
}

/// Interval deltas of the body `FeedingCounters`, used for derived rates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeedingInterval {
    /// Plant matter transferred to stomachs this interval (u²).
    pub plant_consumed_u2: f64,
    /// Meat matter transferred to stomachs this interval (u²).
    pub meat_consumed_u2: f64,
    /// Plant bite events this interval.
    pub plant_bite_events: u64,
    /// Meat bite events this interval.
    pub meat_bite_events: u64,
    /// Bibite bite events this interval.
    pub bibite_bite_events: u64,
    /// Bibite damage dealt this interval.
    pub bibite_damage_dealt: f64,
    /// Stomach energy gain this interval (E).
    pub stomach_energy_gain_e: f64,
    /// Energy gained this interval (E).
    pub energy_gained_e: f64,
    /// Energy spent this interval (E).
    pub energy_spent_e: f64,
    /// Live bibite-time this interval (s).
    pub live_seconds: f64,
}

impl BehaviorStats {
    fn fill_feeding_rates(&mut self, interval: &FeedingInterval) {
        let seconds = interval.live_seconds.max(1e-9);
        self.plant_bites_per_s = interval.plant_bite_events as f64 / seconds;
        self.meat_bites_per_s = interval.meat_bite_events as f64 / seconds;
        self.bibite_bites_per_s = interval.bibite_bite_events as f64 / seconds;
        self.bibite_damage_per_s = interval.bibite_damage_dealt / seconds;
        self.plant_consumed_u2_per_s = interval.plant_consumed_u2 / seconds;
        self.meat_consumed_u2_per_s = interval.meat_consumed_u2 / seconds;
        self.stomach_energy_gain_per_s = interval.stomach_energy_gain_e / seconds;
    }
}

/// Body statistics (`body` group).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyStats {
    /// Mean 1-D size.
    pub size_mean: f32,
    /// Mean base body area.
    pub area_mean: f32,
    /// Mean body mass.
    pub mass_mean: f32,
    /// Mean maximum energy.
    pub max_energy_mean: f32,
    /// Mean maximum health.
    pub max_health_mean: f32,
    /// Mean base metabolism (E/s).
    pub metabolism_mean: f32,
}

/// Tag statistics (`tags` group).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagStats {
    /// Tag string.
    pub tag: String,
    /// Number of bibites carrying the tag.
    pub n: u32,
    /// Total energy of the tagged bibites.
    pub biomass: f32,
}

/// One statistics sample (the parity tick record's aggregate groups).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatsSample {
    /// Absolute simulation tick.
    pub tick: u64,
    /// Simulated time in seconds.
    pub t: f32,
    /// Population counts.
    pub counts: CountStats,
    /// Pellet counters.
    pub pellets: PelletStats,
    /// Vitals.
    pub vitals: VitalStats,
    /// Generations.
    pub generation: GenerationStats,
    /// Active species.
    pub species: SpeciesStats,
    /// Brain sizes.
    pub brain: BrainStats,
    /// Gene mean/std.
    pub genes: GeneStats,
    /// Births/deaths since the previous sample.
    pub lifecycle: LifecycleStats,
    /// Behaviour.
    pub behavior: BehaviorStats,
    /// Body stats.
    pub body: BodyStats,
    /// Tags.
    pub tags: Vec<TagStats>,
    /// Runtime-only pellet conservation counters + live stock at this sample.
    pub conservation: crate::world::ConservationSnapshot,
    /// Game-native metric values (empty until the metric engine lands).
    pub metrics: Vec<(String, f32)>,
}

impl StatsSample {
    /// The 35 gene means in game gene order.
    #[must_use]
    pub fn gene_means(&self) -> &[f32; N_GENES] {
        &self.genes.mean
    }

    /// The 35 gene standard deviations in game gene order.
    #[must_use]
    pub fn gene_stds(&self) -> &[f32; N_GENES] {
        &self.genes.std
    }
}

/// Simulation-wide statistics state (lifecycle counters + the latest sample).
#[derive(Clone, Debug, Default)]
pub struct SimStats {
    /// Number of samples taken so far.
    pub samples: u64,
    /// Births since the last sample.
    pub births_since_sample: u64,
    /// Deaths since the last sample.
    pub deaths_since_sample: u64,
    /// Cumulative births.
    pub births_total: u64,
    /// Cumulative deaths.
    pub deaths_total: u64,
    /// Latest sample (empty until the first `World::sample_stats`).
    pub last: StatsSample,
}

impl SimStats {
    /// Creates an empty stats collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The most recent sample.
    #[inline]
    pub fn last(&self) -> &StatsSample {
        &self.last
    }

    /// Encodes the cumulative/since-sample counters for a checkpoint (the last
    /// full sample is recomputed on the next `sample_stats`, so it is omitted).
    #[must_use]
    pub fn encode_checkpoint(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(40);
        for value in [
            self.samples,
            self.births_since_sample,
            self.deaths_since_sample,
            self.births_total,
            self.deaths_total,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Restores counters produced by [`SimStats::encode_checkpoint`].
    pub fn decode_checkpoint(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if bytes.len() != 40 {
            return Err(if bytes.len() < 40 {
                SnapshotError::Truncated
            } else {
                SnapshotError::TrailingData
            });
        }
        let mut values = [0u64; 5];
        for (index, value) in values.iter_mut().enumerate() {
            let start = index * 8;
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes[start..start + 8]);
            *value = u64::from_le_bytes(raw);
        }
        Ok(Self {
            samples: values[0],
            births_since_sample: values[1],
            deaths_since_sample: values[2],
            births_total: values[3],
            deaths_total: values[4],
            last: StatsSample::default(),
        })
    }
}

/// Collects a [`crate::sim::StatsSample`] from the world's stores.
///
/// This is the single implementation used by `World::sample_stats`; it reads
/// living bibites in dense store order (deterministic) and does not mutate any
/// simulation state.
pub(crate) fn collect_stats(world: &World, tick: u64, t: f32) -> StatsSample {
    let mut sample = StatsSample {
        tick,
        t,
        ..StatsSample::default()
    };
    let settings: &SimSettings = &world.settings;
    let mut n_live = 0u32;
    let mut energy_sum = 0.0f64;
    let mut health_sum = 0.0f64;
    let mut age_sum = 0.0f64;
    let mut age_max = 0.0f32;
    let mut gen_sum = 0.0f64;
    let mut gen_max = 0u32;
    let mut nodes_sum = 0.0f64;
    let mut nodes_max = 0u32;
    let mut syn_sum = 0.0f64;
    let mut syn_max = 0u32;
    let mut gene_sum = [0.0f64; N_GENES];
    let mut gene_sq_sum = [0.0f64; N_GENES];
    let mut size_sum = 0.0f64;
    let mut area_sum = 0.0f64;
    let mut mass_sum = 0.0f64;
    let mut max_energy_sum = 0.0f64;
    let mut max_health_sum = 0.0f64;
    let mut metabolism_sum = 0.0f64;
    let mut bibite_biomass = 0.0f64;
    let mut speed_sum = 0.0f64;
    let mut speed_max = 0.0f32;
    let mut moving = 0u32;
    let mut travel_rate_sum = 0.0f64;
    let mut bite_rate_sum = 0.0f64;
    let mut damage_rate_sum = 0.0f64;
    let mut egg_rate_sum = 0.0f64;
    let mut output_sums = [0.0f64; brain::N_OUTPUTS];
    let mut species_counts: Vec<(u32, u32)> = Vec::new();
    let mut tag_stats: Vec<TagStats> = Vec::new();

    for (_, record) in world.bibites.iter() {
        if record.body.dead {
            sample.counts.corpses += 1;
            continue;
        }
        n_live += 1;
        energy_sum += f64::from(record.body.energy.amount);
        health_sum += f64::from(record.body.health.amount);
        // Game age is in HOURS (`clock.timeAlive / 3600`).
        let age_hours = record.clock.time_alive / 3600.0;
        age_sum += f64::from(age_hours);
        age_max = age_max.max(age_hours);
        gen_sum += f64::from(record.meta.generation);
        gen_max = gen_max.max(record.meta.generation);
        // Game `nActiveNodes = nActiveInputs + nActiveOutputs + nHidden`
        // (inputs/outputs = nodes with at least one outgoing/incoming synapse,
        // regardless of enabled).
        let nodes = record.brain.used_input_mask().count_ones()
            + record.brain.used_output_mask().count_ones()
            + record.brain.hidden_count() as u32;
        let synapses = record.brain.synapse_count() as u32;
        nodes_sum += f64::from(nodes);
        nodes_max = nodes_max.max(nodes);
        syn_sum += f64::from(synapses);
        syn_max = syn_max.max(synapses);
        // Game `totalEnergy` (body+health+eggs+fat, minus corpse offset).
        let extra_stored_energy = record.egg_layer.progress * record.egg_layer.egg_energy
            + record.fat.amount * settings.materials.fat.energy_density;
        bibite_biomass += f64::from(record.body.total_energy(extra_stored_energy));
        // Behaviour definitions from the game tracker.
        let speed = record.vel.length();
        speed_sum += f64::from(speed);
        speed_max = speed_max.max(speed);
        if speed > 1.0 {
            moving += 1;
        }
        // Lifetime rates: counter / max(1, age_seconds) (age_hours * 3600).
        let age_seconds = record.clock.time_alive.max(0.0);
        let denominator = f64::from(age_seconds.max(1.0));
        travel_rate_sum += f64::from(record.propulsion.total_travel) / denominator;
        bite_rate_sum += f64::from(record.mouth.bibites_bitten) / denominator;
        damage_rate_sum += f64::from(record.mouth.total_damage_dealt) / denominator;
        egg_rate_sum += f64::from(record.egg_layer.n_laid) / denominator;
        for output in brain::ALL_OUTPUTS {
            output_sums[output.index()] += f64::from(record.brain.output_value(output));
        }
        for (index, gene) in ALL_GENES.iter().enumerate() {
            let value = record.genome.gene(*gene);
            gene_sum[index] += f64::from(value);
            gene_sq_sum[index] += f64::from(value) * f64::from(value);
        }
        size_sum += f64::from(record.derived.d1_size);
        area_sum += f64::from(record.body.geometry.base_body_area);
        mass_sum += f64::from(record.derived.mass);
        max_energy_sum += f64::from(record.body.geometry.max_energy);
        max_health_sum += f64::from(record.body.geometry.max_health);
        metabolism_sum += f64::from(record.body.metabolism);
        let species_id = record.meta.species.map_or(0, |id| id.value());
        if let Some(entry) = species_counts.iter_mut().find(|(id, _)| *id == species_id) {
            entry.1 += 1;
        } else {
            species_counts.push((species_id, 1));
        }
        if let Some(tag) = &record.meta.species_tag {
            if let Some(entry) = tag_stats.iter_mut().find(|entry| &entry.tag == tag) {
                entry.n += 1;
                entry.biomass += record.body.energy.amount;
            } else {
                tag_stats.push(TagStats {
                    tag: tag.clone(),
                    n: 1,
                    biomass: record.body.energy.amount,
                });
            }
        }
    }

    sample.counts.bibites = n_live;
    sample.counts.eggs = world.eggs.len() as u32;

    let counts = world.pellets.counts();
    sample.pellets.plant_n = counts.plant_count;
    sample.pellets.plant_biomass = counts.plant_biomass as f32;
    sample.pellets.meat_n = counts.meat_count;
    sample.pellets.meat_biomass = counts.meat_biomass as f32;
    // Game `ZoneManager.freeBiomass = Σ_zone (maxBiomass − pelletBiomass)`.
    sample.pellets.free_biomass = world.zones.free_biomass();
    sample.pellets.bibite_biomass = bibite_biomass as f32;

    if n_live > 0 {
        let n = f64::from(n_live);
        sample.vitals.energy_mean = (energy_sum / n) as f32;
        sample.vitals.health_mean = (health_sum / n) as f32;
        sample.vitals.age_mean = (age_sum / n) as f32;
        sample.vitals.age_max = age_max;
        sample.generation.gen_max = gen_max;
        sample.generation.gen_mean = (gen_sum / n) as f32;
        sample.brain.nodes_mean = (nodes_sum / n) as f32;
        sample.brain.nodes_max = nodes_max;
        sample.brain.syn_mean = (syn_sum / n) as f32;
        sample.brain.syn_max = syn_max;
        sample.body.size_mean = (size_sum / n) as f32;
        sample.body.area_mean = (area_sum / n) as f32;
        sample.body.mass_mean = (mass_sum / n) as f32;
        sample.body.max_energy_mean = (max_energy_sum / n) as f32;
        sample.body.max_health_mean = (max_health_sum / n) as f32;
        sample.body.metabolism_mean = (metabolism_sum / n) as f32;
        let mut outputs = [0.0f32; brain::N_OUTPUTS];
        for (index, value) in outputs.iter_mut().enumerate() {
            *value = (output_sums[index] / n) as f32;
        }
        sample.behavior = BehaviorStats {
            speed_mean: (speed_sum / n) as f32,
            speed_max,
            moving_frac: (moving as f64 / n) as f32,
            travel_rate_mean: (travel_rate_sum / n) as f32,
            bite_rate_mean: (bite_rate_sum / n) as f32,
            damage_dealt_rate_mean: (damage_rate_sum / n) as f32,
            egg_rate_mean: (egg_rate_sum / n) as f32,
            outputs,
            ..BehaviorStats::default()
        };
        sample.behavior.fill_feeding_rates(&world.feeding_interval);
        let seconds = world.feeding_interval.live_seconds.max(1e-9);
        sample.behavior.plant_consumed_e_per_s = world.feeding_interval.plant_consumed_u2
            * f64::from(settings.materials.plant.energy_density)
            / seconds;
        sample.behavior.meat_consumed_e_per_s = world.feeding_interval.meat_consumed_u2
            * f64::from(settings.materials.meat.energy_density)
            / seconds;
        for index in 0..N_GENES {
            let mean = gene_sum[index] / n;
            let variance = (gene_sq_sum[index] / n) - mean * mean;
            sample.genes.mean[index] = mean as f32;
            sample.genes.std[index] = variance.max(0.0).sqrt() as f32;
        }
    }

    species_counts.sort_unstable_by_key(|(id, _)| *id);
    sample.species.n = species_counts.len() as u32;
    sample.species.counts = species_counts;

    tag_stats.sort_unstable_by(|a, b| a.tag.cmp(&b.tag));
    sample.tags = tag_stats;

    sample.lifecycle.births = world.stats.births_since_sample;
    sample.lifecycle.deaths = world.stats.deaths_since_sample;
    let counts = world.pellets.counts();
    sample.conservation = crate::world::ConservationSnapshot {
        totals: world.conservation_totals(),
        stock_plant_energy: counts.plant_biomass,
        stock_meat_energy: counts.meat_biomass,
        terms_missing: true,
    };
    sample
}

impl SimStats {
    /// Resets the since-sample counters after a sample was taken.
    pub fn reset_since_sample(&mut self) {
        self.births_since_sample = 0;
        self.deaths_since_sample = 0;
    }

    /// Records a sample: stores it, bumps the sample counter and resets the
    /// since-sample accumulators.
    pub fn record(&mut self, sample: StatsSample) {
        self.last = sample;
        self.samples += 1;
        self.reset_since_sample();
    }
}

/// Deterministic fingerprint of a statistics sample (for repeat-run tests and
/// the CLI). FNV-1a over the numeric fields; not a cryptographic digest.
#[must_use]
pub fn sample_fingerprint(sample: &StatsSample) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |value: u64| {
        hash ^= value;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(sample.tick);
    mix(u64::from(sample.t.to_bits()));
    mix(u64::from(sample.counts.bibites));
    mix(u64::from(sample.counts.eggs));
    mix(u64::from(sample.counts.corpses));
    mix(u64::from(sample.pellets.plant_n));
    mix(u64::from(sample.pellets.meat_n));
    mix(u64::from(sample.pellets.plant_biomass.to_bits()));
    mix(u64::from(sample.pellets.meat_biomass.to_bits()));
    mix(u64::from(sample.vitals.energy_mean.to_bits()));
    mix(u64::from(sample.vitals.health_mean.to_bits()));
    mix(u64::from(sample.vitals.age_mean.to_bits()));
    mix(u64::from(sample.generation.gen_mean.to_bits()));
    mix(sample.lifecycle.births);
    mix(sample.lifecycle.deaths);
    for gene in sample.genes.mean {
        mix(u64::from(gene.to_bits()));
    }
    for gene in sample.genes.std {
        mix(u64::from(gene.to_bits()));
    }
    for output in sample.behavior.outputs {
        mix(u64::from(output.to_bits()));
    }
    for (id, count) in &sample.species.counts {
        mix(u64::from(*id));
        mix(u64::from(*count));
    }
    hash
}

/// Position-velocity fingerprint of every live entity (deterministic repeat-run
/// evidence for the world state, independent of stats rounding).
#[must_use]
pub fn state_fingerprint(world: &World) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |value: u64| {
        hash ^= value;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for (id, record) in world.bibites.iter() {
        mix(u64::from(id.handle().slot()));
        mix(u64::from(id.handle().generation()));
        mix(u64::from(record.pos.x.to_bits()));
        mix(u64::from(record.pos.y.to_bits()));
        mix(u64::from(record.vel.x.to_bits()));
        mix(u64::from(record.vel.y.to_bits()));
        mix(u64::from(record.body.energy.amount.to_bits()));
        mix(u64::from(record.body.health.amount.to_bits()));
    }
    for id in world.pellets.ids() {
        let Some(view) = world.pellets.get(id) else {
            continue;
        };
        mix(u64::from(id.handle().slot()));
        mix(u64::from(id.handle().generation()));
        mix(u64::from(view.pos.x.to_bits()));
        mix(u64::from(view.pos.y.to_bits()));
        mix(u64::from(view.amount.to_bits()));
    }
    for id in world.eggs.ids() {
        mix(u64::from(id.handle().slot()));
        mix(u64::from(id.handle().generation()));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn thread_pool_executes_every_job_and_is_reusable() {
        let pool = ThreadPool::new(4);
        assert_eq!(pool.threads(), 4);
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..200 {
            let counter = Arc::clone(&counter);
            pool.execute(vec![Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            })]);
        }
        // Batch of many jobs across four workers.
        let mut jobs: Vec<PoolJob> = Vec::new();
        for _ in 0..100 {
            let counter = Arc::clone(&counter);
            jobs.push(Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }));
        }
        pool.execute(jobs);
        assert_eq!(counter.load(Ordering::Relaxed), 300);
    }

    #[test]
    fn parallel_executor_order_and_reduce() {
        let pool = ThreadPool::new(4);
        let parallel = ParallelExecutor::new(Some(&pool));
        let serial = ParallelExecutor::new(None);
        assert!(parallel.is_parallel());
        assert_eq!(parallel.threads(), 4);
        assert_eq!(serial.threads(), 1);
        assert_eq!(serial.chunk_count(10_000), 1);
        assert_eq!(parallel.chunk_count(10_000), 4);

        let items: Vec<u64> = (0..10_000).collect();
        let expected: u64 = items.iter().sum();
        let sums = parallel.parallel_map(items, |chunk| chunk.into_iter().sum::<u64>());
        assert_eq!(sums.iter().sum::<u64>(), expected);
        assert_eq!(sums.len(), 4, "one result per chunk, in chunk order");

        // Out-of-order completion must still merge in chunk order.
        let chunks = vec![vec![3u32, 1], vec![2], vec![5, 4, 0], vec![9, 8]];
        let mapped = parallel.parallel_chunks(chunks.clone(), |chunk| {
            let mut sorted = chunk;
            sorted.reverse();
            sorted
        });
        assert_eq!(mapped, vec![vec![1, 3], vec![2], vec![0, 4, 5], vec![8, 9]]);
        let serial_mapped = serial.parallel_chunks(chunks, |mut chunk| {
            chunk.reverse();
            chunk
        });
        assert_eq!(mapped, serial_mapped);

        // Context variant is deterministic too (10k values, 4 chunks).
        let context = Arc::new(7u64);
        let sums = parallel.parallel_chunks_with(
            (0..4).map(|_| vec![1u64; 2500]).collect(),
            Arc::clone(&context),
            |factor, chunk| factor.saturating_mul(chunk.len() as u64),
        );
        assert_eq!(sums, vec![17_500; 4]);
    }

    #[test]
    #[ignore = "manual performance audit (run with --release)"]
    fn perf_parallel_executor_overhead() {
        let pool = ThreadPool::new(8);
        let executor = ParallelExecutor::new(Some(&pool));
        let chunks: Vec<Vec<u64>> = (0..8).map(|_| vec![0u64; 1]).collect();
        let rounds = 1000;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            let out =
                executor.parallel_chunks(chunks.clone(), |chunk| chunk.into_iter().sum::<u64>());
            std::hint::black_box(out);
        }
        println!(
            "pool 8x1-item chunks: {:.1} us/call",
            start.elapsed().as_secs_f64() * 1e6 / f64::from(rounds)
        );
        let serial = ParallelExecutor::new(None);
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            let out =
                serial.parallel_chunks(chunks.clone(), |chunk| chunk.into_iter().sum::<u64>());
            std::hint::black_box(out);
        }
        println!(
            "serial 8x1-item chunks: {:.1} us/call",
            start.elapsed().as_secs_f64() * 1e6 / f64::from(rounds)
        );
    }

    /// The sizes of the pool-backed stages, by name.
    fn sizes_of(gates: ParallelGates) -> [(&'static str, usize); 14] {
        [
            ("pool_bibites", gates.pool_bibites),
            ("pool_bodies", gates.pool_bodies),
            ("sweep", gates.sweep),
            ("records", gates.records),
            ("bodies", gates.bodies),
            ("broadphase", gates.broadphase),
            ("narrow_pairs", gates.narrow_pairs),
            ("due_brains", gates.due_brains),
            ("brains", gates.brains),
            ("sensing", gates.sensing),
            ("scans", gates.scans),
            ("sorts", gates.sorts),
            ("island_bodies", gates.island_bodies),
            ("island_constraints", gates.island_constraints),
        ]
    }

    #[test]
    fn a_world_starts_with_gates_that_size_the_pool_to_the_work() {
        // (`RUSTIBITES_GATES` is not set in the tests' environment: it would change the default.)
        let scaled = ParallelGates::default();
        assert_eq!(scaled, ParallelGates::SCALED);
        // Every size asks for real work: a stage of a few dozen items costs less than waking the pool (the sensing and narrowphase
        // sizes are in due bibites and pairs, which come in dozens, with chunks of a quarter of that).
        for (name, size) in sizes_of(scaled) {
            assert!(
                size >= 64,
                "the {name} size of {size} puts a stage that is all hand-off on the pool"
            );
        }
        // A world on the pool has the records and the bodies for the stages that are not about a few of them.
        assert!(scaled.pool_bibites >= scaled.sweep.max(scaled.records).max(scaled.broadphase));
        assert!(scaled.pool_bodies >= scaled.broadphase);
        // What is cheap per item asks for more items than what is dear: the cheap per-body stages (10-40 ns), the scans of the
        // summaries (a few ns).
        assert!(scaled.bodies > scaled.sweep && scaled.scans > scaled.bodies);
        // The x order is only sorted from scratch on the pool in a world big enough for the per-body stages.
        assert!(scaled.sorts >= scaled.bodies);
        // A world measures its own ticks to choose between inline and the pool; the fixed gates have the same sizes and do not.
        assert!(scaled.adaptive);
        assert!(!ParallelGates::FIXED.adaptive);
        assert_eq!(
            ParallelGates::FIXED,
            ParallelGates {
                adaptive: false,
                ..scaled
            }
        );
        // The gates that put every stage on the pool from one item: every size at 1, and nothing to measure.
        for gates in [ParallelGates::ALWAYS, ParallelGates::EAGER] {
            assert!(!gates.adaptive);
            for (name, size) in sizes_of(gates) {
                assert_eq!(size, 1, "the {name} size keeps a small stage off the pool");
            }
        }
        // The resting-pellet layer is not about the pool: it keeps the size from which it pays, except in the tests.
        assert_eq!(ParallelGates::ALWAYS.sleep_pellets, scaled.sleep_pellets);
        assert_eq!(ParallelGates::EAGER.sleep_pellets, 1);
    }

    #[test]
    fn a_stage_runs_inline_below_its_size_and_gets_a_worker_for_every_quarter_of_it_above() {
        use ParallelGates as G;
        // Below the size, or in a world with no pool: inline (one worker).
        assert_eq!(G::workers_for(0, 1_500, 12), 1);
        assert_eq!(G::workers_for(1_499, 1_500, 12), 1);
        assert_eq!(G::workers_for(1_000_000, 1_500, 1), 1);
        assert_eq!(G::workers_for(1_000_000, 1_500, 0), 1);
        // From the size: four workers, then one more for every quarter of the size, up to the pool.
        assert_eq!(G::workers_for(1_500, 1_500, 12), 4);
        assert_eq!(G::workers_for(1_875, 1_500, 12), 5);
        assert_eq!(G::workers_for(2_250, 1_500, 12), 6);
        assert_eq!(G::workers_for(4_500, 1_500, 12), 12);
        assert_eq!(G::workers_for(1_000_000, 1_500, 12), 12);
        assert_eq!(G::workers_for(1_500, 1_500, 3), 3);
        assert_eq!(G::workers_for(1_500, 1_500, 2), 2);
        // A size of 0 counts as 1; from a size of 1 any stage with an item is on the pool (two workers at least).
        assert_eq!(G::workers_for(1, 0, 12), 4);
        assert_eq!(G::workers_for(1, 1, 12), 4);
        assert_eq!(G::workers_for(5, 1, 12), 12);
        assert_eq!(G::workers_for(1, 1, 2), 2);
        assert_eq!(G::workers_for(0, 1, 12), 1);
        // The chunks are never shorter than a quarter of the size, and never empty.
        assert_eq!(G::chunk_len(1_500, 1_500, 4, 4), 375);
        assert_eq!(G::chunk_len(100_000, 1_500, 12, 4), 2_084);
        assert_eq!(G::chunk_len(100_000, 1_500, 12, 1), 8_334);
        assert_eq!(G::chunk_len(0, 0, 0, 0), 1);
        for from in [1usize, 7, 256, 1_500] {
            for items in [
                0usize, 1, 63, 64, 255, 256, 1_000, 1_499, 1_500, 4_097, 100_000,
            ] {
                for pool in [1usize, 2, 12] {
                    let workers = G::workers_for(items, from, pool);
                    assert!((1..=pool.max(1)).contains(&workers));
                    if workers > 1 {
                        assert!(items >= from, "{items} items on the pool from {from}");
                    }
                    for per_worker in [1usize, 4] {
                        let len = G::chunk_len(items, from, workers, per_worker);
                        assert!(
                            len >= from.div_ceil(4).max(1),
                            "{items} items, size {from}: chunks of {len}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_whole_tick_moves_onto_the_pool_only_when_a_stage_has_work_for_it() {
        let scaled = ParallelGates::SCALED;
        assert!(!scaled.pool_has_work_for(0, 0));
        assert!(!scaled.pool_has_work_for(50, 100));
        assert!(!scaled.pool_has_work_for(scaled.pool_bibites - 1, scaled.pool_bodies - 1));
        assert!(scaled.pool_has_work_for(scaled.pool_bibites, 0));
        assert!(scaled.pool_has_work_for(0, scaled.pool_bodies));
        // With every size at 1 a world with one body is on the pool.
        assert!(ParallelGates::EAGER.pool_has_work_for(1, 0));
        assert!(ParallelGates::ALWAYS.pool_has_work_for(0, 1));
        assert!(!ParallelGates::EAGER.pool_has_work_for(0, 0));
    }

    #[test]
    fn each_level_of_the_pool_has_smaller_sizes_than_the_last_and_the_last_has_every_size_at_one() {
        let gates = ParallelGates::SCALED;
        assert_eq!(gates.pooled_level(0), gates);
        let sizes = |gates: ParallelGates| {
            [
                gates.sweep,
                gates.records,
                gates.bodies,
                gates.broadphase,
                gates.narrow_pairs,
                gates.due_brains,
                gates.brains,
                gates.sensing,
                gates.scans,
                gates.sorts,
                gates.island_bodies,
                gates.island_constraints,
            ]
        };
        for level in 0..crate::pool_tuner::POOLED_LEVELS as u8 - 1 {
            let (more, less) = (
                sizes(gates.pooled_level(level + 1)),
                sizes(gates.pooled_level(level)),
            );
            for (index, (more, less)) in more.into_iter().zip(less).enumerate() {
                assert!(
                    more <= less,
                    "level {} size {index}: {more} > {less}",
                    level + 1
                );
            }
            assert!(
                sizes(gates.pooled_level(level + 1)) != sizes(gates.pooled_level(level)),
                "level {level}"
            );
        }
        let last = gates.pooled_level(crate::pool_tuner::POOLED_LEVELS as u8 - 1);
        assert!(sizes(last).into_iter().all(|size| size == 1), "{last:?}");
        // What the levels are not about: the measuring, the prior and the resting-pellet layer.
        for level in 0..crate::pool_tuner::POOLED_LEVELS as u8 {
            let at = gates.pooled_level(level);
            assert_eq!(
                (at.adaptive, at.sleep_pellets),
                (gates.adaptive, gates.sleep_pellets)
            );
        }
        assert_eq!(last.pool_bibites, gates.pool_bibites);
        // Out of range counts as the last level.
        assert_eq!(gates.pooled_level(200), last);
    }

    #[test]
    fn a_level_of_the_pool_can_be_asked_for_by_name_for_measuring_it_alone() {
        for level in 0..crate::pool_tuner::POOLED_LEVELS as u8 {
            let gates = ParallelGates::from_mode(&format!("pooled:{level}")).expect("a level");
            assert!(!gates.adaptive, "level {level} measures");
            // Every world is on the pool, whatever its size, with the stage sizes of the level.
            assert!(gates.pool_has_work_for(1, 1));
            assert_eq!(gates.sweep, ParallelGates::FIXED.pooled_level(level).sweep);
        }
        assert_eq!(
            ParallelGates::from_mode("POOLED:3")
                .expect("any case")
                .sweep,
            ParallelGates::ALWAYS.sweep
        );
        assert!(ParallelGates::from_mode("pooled:4").is_none());
        assert!(ParallelGates::from_mode("pooled:x").is_none());
        assert!(ParallelGates::from_mode("pooled").is_none());
    }

    #[test]
    fn the_benchmark_switch_names_its_gates() {
        assert_eq!(
            ParallelGates::from_mode("scaled"),
            Some(ParallelGates::SCALED)
        );
        assert_eq!(
            ParallelGates::from_mode("Fixed"),
            Some(ParallelGates::FIXED)
        );
        assert_eq!(
            ParallelGates::from_mode("Always"),
            Some(ParallelGates::ALWAYS)
        );
        assert_eq!(
            ParallelGates::from_mode("EAGER"),
            Some(ParallelGates::EAGER)
        );
        assert_eq!(
            ParallelGates::from_mode("nosleep"),
            Some(ParallelGates::SCALED.without_sleep())
        );
        // `scaled:<percent>` scales the sizes, never to 0, and leaves the resting-pellet layer alone.
        assert_eq!(
            ParallelGates::from_mode("scaled:100"),
            Some(ParallelGates::SCALED)
        );
        let half = ParallelGates::from_mode("Scaled:50").expect("a percentage");
        assert_eq!(half.pool_bibites, ParallelGates::SCALED.pool_bibites / 2);
        assert_eq!(half.bodies, ParallelGates::SCALED.bodies / 2);
        assert_eq!(half.sleep_pellets, ParallelGates::SCALED.sleep_pellets);
        let tiny = ParallelGates::from_mode("scaled:1").expect("a percentage");
        for (name, size) in sizes_of(tiny) {
            assert!(size >= 1, "{name}");
        }
        let triple = ParallelGates::from_mode("scaled:300").expect("a percentage");
        assert_eq!(triple.pool_bibites, ParallelGates::SCALED.pool_bibites * 3);
        assert_eq!(ParallelGates::from_mode("scaled:0"), None);
        assert_eq!(ParallelGates::from_mode("scaled:"), None);
        assert_eq!(ParallelGates::from_mode("scaled:fast"), None);
        assert_eq!(ParallelGates::from_mode("always:50"), None);
        assert_eq!(ParallelGates::from_mode(""), None);
        assert_eq!(ParallelGates::from_mode("scaled "), None);
        assert_eq!(ParallelGates::from_mode("tuned"), None);
        assert_eq!(ParallelGates::from_mode("serial"), None);
    }

    #[test]
    fn phase_timings_helpers() {
        let mut timings = PhaseTimings::new();
        assert_eq!(timings.ticks, 0);
        assert_eq!(timings.mean_nanos(Phase::Sensing), 0.0);
        timings.add(Phase::Sensing, 2_000_000);
        timings.add(Phase::Physics, 8_000_000);
        timings.tick_complete();
        assert_eq!(timings.nanos(Phase::Sensing), 2_000_000);
        assert!((timings.mean_nanos(Phase::Sensing) - 2_000_000.0).abs() < 1e-6);
        assert!((timings.share(Phase::Physics) - 0.8).abs() < 1e-9);
        assert_eq!(timings.total_nanos(), 10_000_000);
        assert_eq!(Phase::ALL.len(), PHASE_COUNT);
        timings.reset();
        assert_eq!(timings.ticks, 0);
    }
}

// ---------------------------------------------------------------------------
// Checkpoint snapshot (task-75 P1): dependency-free LE encoding of the
// simulation-control state that the game save does not carry.
// ---------------------------------------------------------------------------

use crate::clock::FixedClock;
use crate::rng::{Rng, RngState};

/// Magic bytes at the start of a [`SimSnapshot`] encoding.
pub const SIM_SNAPSHOT_MAGIC: [u8; 8] = *b"RBSNAP01";
/// Snapshot format version.
pub const SIM_SNAPSHOT_VERSION: u16 = 1;

const SNAPSHOT_HEADER_LEN: usize = 8 + 2 + 2 + 4;
const SNAPSHOT_CHECKSUM_LEN: usize = 8;
const MAX_ZONE_STREAMS: usize = 100_000;
const MAX_BOOLS: usize = 1_000_000;
const MAX_EXTENSIONS: usize = 1_024;
const MAX_EXTENSION_BYTES: usize = 64 * 1024 * 1024;
/// Maximum encoded snapshot payload (all sections + base control state).
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
/// Upper bound accepted by [`payload_limit`] (streamed v2 transport).
pub const STREAM_MAX_PAYLOAD_BYTES: usize = 1024 * 1024 * 1024;

/// Effective contiguous-payload limit for v1 [`SimSnapshot::encode`]/[`SimSnapshot::decode`].
///
/// Defaults to [`MAX_PAYLOAD_BYTES`] (v1 unchanged). Streaming transports may
/// raise it up to [`STREAM_MAX_PAYLOAD_BYTES`] with
/// `RUSTIBITES_SNAPSHOT_MAX_BYTES`; per-section 64 MiB chunking is unaffected.
#[must_use]
pub fn payload_limit() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("RUSTIBITES_SNAPSHOT_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .map(|value| value.clamp(MAX_PAYLOAD_BYTES, STREAM_MAX_PAYLOAD_BYTES))
            .unwrap_or(MAX_PAYLOAD_BYTES)
    })
}
/// Chunk payload size for section splitting (large worlds).
const EXTENSION_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// Chunk section tag family (`base | parent_tag & 0xFF`).
const EXTENSION_CHUNK_TAG_BASE: u16 = 0x0200;

/// Errors from [`SimSnapshot::encode`] / [`SimSnapshot::decode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Input ended before all declared fields were read.
    Truncated,
    /// Magic bytes do not match [`SIM_SNAPSHOT_MAGIC`].
    BadMagic,
    /// Unsupported format version.
    UnsupportedVersion(u16),
    /// Unsupported flag bits (none are defined in v1).
    UnsupportedFlags(u16),
    /// Trailing bytes after the declared payload.
    TrailingData,
    /// Checksum mismatch (corrupt/truncated payload).
    ChecksumMismatch,
    /// A length/count is beyond the supported limit.
    LimitExceeded(&'static str),
    /// An encoded boolean byte was not 0 or 1.
    InvalidBool,
    /// An RNG stream state is invalid (all zero).
    InvalidRngState,
    /// `sim_tps` must be positive.
    InvalidSimTps,
    /// Streamed sections are malformed/inconsistent.
    InvalidData(&'static str),
}

impl core::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => write!(f, "snapshot is truncated"),
            Self::BadMagic => write!(f, "snapshot magic mismatch"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported snapshot version {version}")
            }
            Self::UnsupportedFlags(flags) => write!(f, "unsupported snapshot flags {flags}"),
            Self::TrailingData => write!(f, "snapshot has trailing data"),
            Self::ChecksumMismatch => write!(f, "snapshot checksum mismatch"),
            Self::LimitExceeded(what) => write!(f, "snapshot {what} exceeds the supported limit"),
            Self::InvalidBool => write!(f, "snapshot has an invalid boolean byte"),
            Self::InvalidRngState => write!(f, "snapshot has an invalid all-zero RNG state"),
            Self::InvalidSimTps => write!(f, "snapshot sim_tps must be positive"),
            Self::InvalidData(what) => write!(f, "snapshot {what}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Opaque forward-compatible snapshot section for phase-2 state (entity
/// runtime, arenas, species counters) owned by a later implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionSection {
    /// Section tag (caller-defined; 0 is reserved).
    pub tag: u16,
    /// Section payload (raw bytes; the core does not interpret it).
    pub data: Vec<u8>,
}

/// One streamed snapshot section: tag `0` is the control preamble, all other
/// tags are extension sections in their original order.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotSection<'a> {
    /// Section tag (`0` = control preamble).
    pub tag: u16,
    /// Section bytes (control bytes are owned; extension bytes borrow the snapshot).
    pub data: std::borrow::Cow<'a, [u8]>,
}

/// Simulation-control state for a bit-identical checkpoint resume (task-75).
///
/// This is **not** the game save: it carries the RNG stream positions, clock and
/// cadence/timer state that the game format drops. Entity runtime state and
/// arena layout are phase-2 `extensions` so the schema can grow without changing
/// this version. The encoding is explicit little-endian, dependency-free, and
/// checksummed; floats are stored as raw bit patterns (NaN/-0.0 preserved).
#[derive(Clone, Debug)]
pub struct SimSnapshot {
    /// Absolute simulation tick (must match `simulated_time` via `sim_tps`).
    pub tick: u64,
    /// Ticks per simulated second (`FixedClock` rate).
    pub sim_tps: u32,
    /// Simulated time in seconds.
    pub simulated_time: f64,
    /// `World.rng` position.
    pub rng: RngState,
    /// Per-zone RNG positions, in ascending zone-index order.
    pub zone_rngs: Vec<(u32, RngState)>,
    /// `SpeciesRegistry.name_rng` position, when the registry exposes one.
    pub species_name_rng: Option<RngState>,
    /// Spawner coroutine accumulator (`World.spawner_accumulator`).
    pub spawner_accumulator: f32,
    /// Pellet-decay throttle period (`World.decay`), in ticks.
    pub decay_period: u32,
    /// Pellet-decay throttle progress, in ticks.
    pub decay_progress: u32,
    /// Settings-animator 0.1 s accumulator (`World.animator_progress`).
    pub animator_progress: f32,
    /// Per-animator "started" flags (parallel to `settings_changers`).
    pub animator_started: Vec<bool>,
    /// Per-animator "ended" flags (parallel to `settings_changers`).
    pub animator_ended: Vec<bool>,
    /// Red Death damage-cadence accumulator.
    pub red_death_progress: f32,
    /// Red Death safe radius (`f32::INFINITY` when disabled is allowed; bits kept).
    pub red_death_safe_radius: f32,
    /// Phase-2 sections (entity runtime, arenas, species counters, ...).
    pub extensions: Vec<ExtensionSection>,
}

impl Default for SimSnapshot {
    fn default() -> Self {
        let rng = Rng::from_seed(0).state();
        Self {
            tick: 0,
            sim_tps: 30,
            simulated_time: 0.0,
            rng,
            zone_rngs: Vec::new(),
            species_name_rng: None,
            spawner_accumulator: 0.0,
            decay_period: 2,
            decay_progress: 0,
            animator_progress: 0.0,
            animator_started: Vec::new(),
            animator_ended: Vec::new(),
            red_death_progress: 0.0,
            red_death_safe_radius: f32::INFINITY,
            extensions: Vec::new(),
        }
    }
}

impl SimSnapshot {
    /// Creates a snapshot with the mandatory simulation-control fields.
    pub fn new(tick: u64, sim_tps: u32, simulated_time: f64, rng: RngState) -> Self {
        Self {
            tick,
            sim_tps,
            simulated_time,
            rng,
            ..Self::default()
        }
    }

    /// Appends a forward-compatible extension section.
    pub fn push_extension(&mut self, tag: u16, data: Vec<u8>) -> Result<(), SnapshotError> {
        if self.extensions.len() >= MAX_EXTENSIONS {
            return Err(SnapshotError::LimitExceeded("extension count"));
        }
        if data.len() > MAX_EXTENSION_BYTES {
            return Err(SnapshotError::LimitExceeded("extension bytes"));
        }
        if tag == 0 {
            return Err(SnapshotError::LimitExceeded("extension tag 0 is reserved"));
        }
        self.extensions.push(ExtensionSection { tag, data });
        Ok(())
    }

    /// Appends a section, splitting payloads above `MAX_EXTENSION_BYTES` into
    /// multiple tag-chunk sections (large 5k/10k/100k entity arenas). A direct
    /// section is used when it fits, so small-world snapshots are unchanged.
    pub fn push_section(&mut self, tag: u16, data: Vec<u8>) -> Result<(), SnapshotError> {
        if data.len() <= MAX_EXTENSION_BYTES {
            return self.push_extension(tag, data);
        }
        let chunks = data.len().div_ceil(EXTENSION_CHUNK_BYTES);
        if chunks > MAX_EXTENSIONS {
            return Err(SnapshotError::LimitExceeded("extension chunk count"));
        }
        for (index, chunk) in data.chunks(EXTENSION_CHUNK_BYTES).enumerate() {
            let mut section = Vec::with_capacity(10 + chunk.len());
            section.extend_from_slice(&tag.to_le_bytes());
            section.extend_from_slice(&(index as u32).to_le_bytes());
            section.extend_from_slice(&(chunks as u32).to_le_bytes());
            section.extend_from_slice(chunk);
            self.push_extension(EXTENSION_CHUNK_TAG_BASE | (tag & 0x00FF), section)?;
        }
        Ok(())
    }

    /// Finds the first extension with `tag`.
    pub fn extension(&self, tag: u16) -> Option<&[u8]> {
        self.extensions
            .iter()
            .find(|section| section.tag == tag)
            .map(|section| section.data.as_slice())
    }

    /// Reassembles a direct section or its chunk sections.
    pub fn extension_payload(&self, tag: u16) -> Option<Vec<u8>> {
        if let Some(data) = self.extension(tag) {
            return Some(data.to_vec());
        }
        let chunk_tag = EXTENSION_CHUNK_TAG_BASE | (tag & 0x00FF);
        let mut chunks: Vec<(u32, u32, &[u8])> = Vec::new();
        for section in &self.extensions {
            if section.tag != chunk_tag || section.data.len() < 10 {
                continue;
            }
            let parent = u16::from_le_bytes([section.data[0], section.data[1]]);
            if parent != tag {
                continue;
            }
            let index = u32::from_le_bytes(section.data[2..6].try_into().ok()?);
            let total = u32::from_le_bytes(section.data[6..10].try_into().ok()?);
            chunks.push((index, total, &section.data[10..]));
        }
        if chunks.is_empty() {
            return None;
        }
        let total = chunks[0].1;
        if total as usize != chunks.len() {
            return None;
        }
        chunks.sort_unstable_by_key(|(index, _, _)| *index);
        let mut out = Vec::new();
        for (expected, (index, _, bytes)) in chunks.iter().enumerate() {
            if *index != expected as u32 {
                return None;
            }
            out.extend_from_slice(bytes);
        }
        Some(out)
    }

    /// Rebuilds a [`FixedClock`] at the snapshot tick/rate.
    pub fn restore_clock(&self) -> Result<FixedClock, SnapshotError> {
        if self.sim_tps == 0 {
            return Err(SnapshotError::InvalidSimTps);
        }
        let mut clock = FixedClock::new(self.sim_tps);
        clock.advance(self.tick);
        Ok(clock)
    }

    /// Encodes the snapshot to bytes (little-endian, checksummed).
    ///
    /// Deterministic: the same field values and extension order always produce
    /// the same bytes.
    /// Validates and encodes the control preamble (all scalar control state plus
    /// the declared extension count). Used by [`Self::encode`] and by streaming
    /// writers via [`Self::encode_sections`].
    fn encode_control(&self) -> Result<Vec<u8>, SnapshotError> {
        if self.sim_tps == 0 {
            return Err(SnapshotError::InvalidSimTps);
        }
        if !self.rng.is_valid() {
            return Err(SnapshotError::InvalidRngState);
        }
        if self.zone_rngs.len() > MAX_ZONE_STREAMS {
            return Err(SnapshotError::LimitExceeded("zone stream count"));
        }
        if let Some(state) = self.species_name_rng
            && !state.is_valid()
        {
            return Err(SnapshotError::InvalidRngState);
        }
        for (_, state) in &self.zone_rngs {
            if !state.is_valid() {
                return Err(SnapshotError::InvalidRngState);
            }
        }
        if self.animator_started.len() > MAX_BOOLS || self.animator_ended.len() > MAX_BOOLS {
            return Err(SnapshotError::LimitExceeded("animator flag count"));
        }
        if self.extensions.len() > MAX_EXTENSIONS {
            return Err(SnapshotError::LimitExceeded("extension count"));
        }
        for section in &self.extensions {
            if section.data.len() > MAX_EXTENSION_BYTES {
                return Err(SnapshotError::LimitExceeded("extension bytes"));
            }
        }
        let mut payload = Writer::default();
        payload.u64(self.tick);
        payload.u32(self.sim_tps);
        payload.u64(self.simulated_time.to_bits());
        payload.rng_state(self.rng);
        payload.u32(self.zone_rngs.len() as u32);
        for (index, state) in &self.zone_rngs {
            payload.u32(*index);
            payload.rng_state(*state);
        }
        match self.species_name_rng {
            Some(state) => {
                payload.u8(1);
                payload.rng_state(state);
            }
            None => payload.u8(0),
        }
        payload.u32(self.spawner_accumulator.to_bits());
        payload.u32(self.decay_period);
        payload.u32(self.decay_progress);
        payload.u32(self.animator_progress.to_bits());
        payload.bools(&self.animator_started);
        payload.bools(&self.animator_ended);
        payload.u32(self.red_death_progress.to_bits());
        payload.u32(self.red_death_safe_radius.to_bits());
        payload.u32(self.extensions.len() as u32);
        Ok(payload.bytes)
    }

    /// Streams the snapshot as control preamble + extension sections without
    /// first building one contiguous payload. Concatenating `tag`/`len`/`data`
    /// in iteration order reproduces [`Self::encode`]'s payload exactly.
    ///
    /// Errors are returned before iteration starts (the validation is the same
    /// as [`Self::encode`]).
    pub fn encode_sections(
        &self,
    ) -> Result<impl Iterator<Item = SnapshotSection<'_>>, SnapshotError> {
        let control = self.encode_control()?;
        let total = self
            .extensions
            .iter()
            .fold(control.len(), |sum, section| sum + 6 + section.data.len());
        if total > payload_limit() {
            return Err(SnapshotError::LimitExceeded("payload bytes"));
        }
        let control_section = SnapshotSection {
            tag: 0,
            data: std::borrow::Cow::Owned(control),
        };
        let extensions = self.extensions.iter().map(|section| SnapshotSection {
            tag: section.tag,
            data: std::borrow::Cow::Borrowed(section.data.as_slice()),
        });
        Ok(std::iter::once(control_section).chain(extensions))
    }

    /// Encodes the snapshot into the v1 contiguous byte format.
    pub fn encode(&self) -> Result<Vec<u8>, SnapshotError> {
        if self.sim_tps == 0 {
            return Err(SnapshotError::InvalidSimTps);
        }
        if !self.rng.is_valid() {
            return Err(SnapshotError::InvalidRngState);
        }
        if self.zone_rngs.len() > MAX_ZONE_STREAMS {
            return Err(SnapshotError::LimitExceeded("zone stream count"));
        }
        if let Some(state) = self.species_name_rng {
            if !state.is_valid() {
                return Err(SnapshotError::InvalidRngState);
            }
        }
        for (_, state) in &self.zone_rngs {
            if !state.is_valid() {
                return Err(SnapshotError::InvalidRngState);
            }
        }
        if self.animator_started.len() > MAX_BOOLS || self.animator_ended.len() > MAX_BOOLS {
            return Err(SnapshotError::LimitExceeded("animator flag count"));
        }
        if self.extensions.len() > MAX_EXTENSIONS {
            return Err(SnapshotError::LimitExceeded("extension count"));
        }
        for section in &self.extensions {
            if section.data.len() > MAX_EXTENSION_BYTES {
                return Err(SnapshotError::LimitExceeded("extension bytes"));
            }
        }

        let mut payload = Writer::default();
        payload.u64(self.tick);
        payload.u32(self.sim_tps);
        payload.u64(self.simulated_time.to_bits());
        payload.rng_state(self.rng);
        payload.u32(self.zone_rngs.len() as u32);
        for (index, state) in &self.zone_rngs {
            payload.u32(*index);
            payload.rng_state(*state);
        }
        match self.species_name_rng {
            Some(state) => {
                payload.u8(1);
                payload.rng_state(state);
            }
            None => payload.u8(0),
        }
        payload.u32(self.spawner_accumulator.to_bits());
        payload.u32(self.decay_period);
        payload.u32(self.decay_progress);
        payload.u32(self.animator_progress.to_bits());
        payload.bools(&self.animator_started);
        payload.bools(&self.animator_ended);
        payload.u32(self.red_death_progress.to_bits());
        payload.u32(self.red_death_safe_radius.to_bits());
        payload.u32(self.extensions.len() as u32);
        for section in &self.extensions {
            payload.u16(section.tag);
            payload.u32(section.data.len() as u32);
            payload.bytes(&section.data);
        }
        if payload.bytes.len() > payload_limit() {
            return Err(SnapshotError::LimitExceeded("payload bytes"));
        }

        let checksum = fnv1a64(&payload.bytes);
        let mut out = Vec::with_capacity(SNAPSHOT_HEADER_LEN + payload.bytes.len() + 8);
        out.extend_from_slice(&SIM_SNAPSHOT_MAGIC);
        out.extend_from_slice(&SIM_SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(payload.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload.bytes);
        out.extend_from_slice(&checksum.to_le_bytes());
        Ok(out)
    }

    /// Decodes bytes produced by [`SimSnapshot::encode`].
    ///
    /// Fails closed on magic/version/flags/checksum/length problems.
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if bytes.len() < SNAPSHOT_HEADER_LEN + SNAPSHOT_CHECKSUM_LEN {
            return Err(SnapshotError::Truncated);
        }
        if bytes[..8] != SIM_SNAPSHOT_MAGIC {
            return Err(SnapshotError::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != SIM_SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion(version));
        }
        let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
        if flags != 0 {
            return Err(SnapshotError::UnsupportedFlags(flags));
        }
        let payload_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        if payload_len > payload_limit() {
            return Err(SnapshotError::LimitExceeded("payload bytes"));
        }
        let expected = SNAPSHOT_HEADER_LEN + payload_len + SNAPSHOT_CHECKSUM_LEN;
        if bytes.len() < expected {
            return Err(SnapshotError::Truncated);
        }
        if bytes.len() > expected {
            return Err(SnapshotError::TrailingData);
        }
        let payload = &bytes[SNAPSHOT_HEADER_LEN..SNAPSHOT_HEADER_LEN + payload_len];
        let stored_checksum = u64::from_le_bytes(
            bytes[SNAPSHOT_HEADER_LEN + payload_len..SNAPSHOT_HEADER_LEN + payload_len + 8]
                .try_into()
                .map_err(|_| SnapshotError::Truncated)?,
        );
        if stored_checksum != fnv1a64(payload) {
            return Err(SnapshotError::ChecksumMismatch);
        }

        let mut reader = Reader::new(payload);
        let tick = reader.u64()?;
        let sim_tps = reader.u32()?;
        if sim_tps == 0 {
            return Err(SnapshotError::InvalidSimTps);
        }
        let simulated_time = f64::from_bits(reader.u64()?);
        let rng = reader.rng_state()?;
        let zone_count = reader.u32()? as usize;
        if zone_count > MAX_ZONE_STREAMS {
            return Err(SnapshotError::LimitExceeded("zone stream count"));
        }
        let mut zone_rngs = Vec::with_capacity(zone_count);
        for _ in 0..zone_count {
            let index = reader.u32()?;
            zone_rngs.push((index, reader.rng_state()?));
        }
        let species_name_rng = match reader.u8()? {
            0 => None,
            1 => Some(reader.rng_state()?),
            _ => return Err(SnapshotError::InvalidBool),
        };
        let spawner_accumulator = f32::from_bits(reader.u32()?);
        let decay_period = reader.u32()?;
        let decay_progress = reader.u32()?;
        let animator_progress = f32::from_bits(reader.u32()?);
        let animator_started = reader.bools()?;
        let animator_ended = reader.bools()?;
        let red_death_progress = f32::from_bits(reader.u32()?);
        let red_death_safe_radius = f32::from_bits(reader.u32()?);
        let extension_count = reader.u32()? as usize;
        if extension_count > MAX_EXTENSIONS {
            return Err(SnapshotError::LimitExceeded("extension count"));
        }
        let mut extensions = Vec::with_capacity(extension_count);
        for _ in 0..extension_count {
            let tag = reader.u16()?;
            let len = reader.u32()? as usize;
            if len > MAX_EXTENSION_BYTES {
                return Err(SnapshotError::LimitExceeded("extension bytes"));
            }
            extensions.push(ExtensionSection {
                tag,
                data: reader.take(len)?.to_vec(),
            });
        }
        reader.finish()?;
        Ok(Self {
            tick,
            sim_tps,
            simulated_time,
            rng,
            zone_rngs,
            species_name_rng,
            spawner_accumulator,
            decay_period,
            decay_progress,
            animator_progress,
            animator_started,
            animator_ended,
            red_death_progress,
            red_death_safe_radius,
            extensions,
        })
    }
}

impl SimSnapshot {
    /// Decodes sections produced by [`Self::encode_sections`] (or an equivalent
    /// streamed transport). The first section must be tag `0`; extension
    /// sections are consumed in order and must match the declared count.
    pub fn decode_sections<'a, I>(sections: I) -> Result<Self, SnapshotError>
    where
        I: IntoIterator<Item = SnapshotSection<'a>>,
    {
        let mut sections = sections.into_iter();
        let control = sections.next().ok_or(SnapshotError::Truncated)?;
        if control.tag != 0 {
            return Err(SnapshotError::InvalidData(
                "first snapshot section must be control",
            ));
        }
        let mut reader = Reader::new(&control.data);
        let tick = reader.u64()?;
        let sim_tps = reader.u32()?;
        if sim_tps == 0 {
            return Err(SnapshotError::InvalidSimTps);
        }
        let simulated_time = f64::from_bits(reader.u64()?);
        let rng = reader.rng_state()?;
        let zone_count = reader.u32()? as usize;
        if zone_count > MAX_ZONE_STREAMS {
            return Err(SnapshotError::LimitExceeded("zone stream count"));
        }
        let mut zone_rngs = Vec::with_capacity(zone_count);
        for _ in 0..zone_count {
            let index = reader.u32()?;
            zone_rngs.push((index, reader.rng_state()?));
        }
        let species_name_rng = match reader.u8()? {
            0 => None,
            1 => Some(reader.rng_state()?),
            _ => return Err(SnapshotError::InvalidBool),
        };
        let spawner_accumulator = f32::from_bits(reader.u32()?);
        let decay_period = reader.u32()?;
        let decay_progress = reader.u32()?;
        let animator_progress = f32::from_bits(reader.u32()?);
        let animator_started = reader.bools()?;
        let animator_ended = reader.bools()?;
        let red_death_progress = f32::from_bits(reader.u32()?);
        let red_death_safe_radius = f32::from_bits(reader.u32()?);
        let extension_count = reader.u32()? as usize;
        if extension_count > MAX_EXTENSIONS {
            return Err(SnapshotError::LimitExceeded("extension count"));
        }
        reader.finish()?;
        let mut extensions = Vec::with_capacity(extension_count);
        let mut total = control.data.len();
        for section in sections {
            if section.tag == 0 {
                return Err(SnapshotError::InvalidData("duplicate control section"));
            }
            if section.data.len() > MAX_EXTENSION_BYTES {
                return Err(SnapshotError::LimitExceeded("extension bytes"));
            }
            total = total.saturating_add(6 + section.data.len());
            if total > payload_limit() {
                return Err(SnapshotError::LimitExceeded("payload bytes"));
            }
            extensions.push(ExtensionSection {
                tag: section.tag,
                data: section.data.into_owned(),
            });
        }
        if extensions.len() != extension_count {
            return Err(SnapshotError::InvalidData("extension count mismatch"));
        }
        Ok(Self {
            tick,
            sim_tps,
            simulated_time,
            rng,
            zone_rngs,
            species_name_rng,
            spawner_accumulator,
            decay_period,
            decay_progress,
            animator_progress,
            animator_started,
            animator_ended,
            red_death_progress,
            red_death_safe_radius,
            extensions,
        })
    }
}

impl PartialEq for SimSnapshot {
    /// Bitwise float comparison (NaN and -0.0 are part of checkpoint identity).
    fn eq(&self, other: &Self) -> bool {
        self.tick == other.tick
            && self.sim_tps == other.sim_tps
            && self.simulated_time.to_bits() == other.simulated_time.to_bits()
            && self.rng == other.rng
            && self.zone_rngs == other.zone_rngs
            && self.species_name_rng == other.species_name_rng
            && self.spawner_accumulator.to_bits() == other.spawner_accumulator.to_bits()
            && self.decay_period == other.decay_period
            && self.decay_progress == other.decay_progress
            && self.animator_progress.to_bits() == other.animator_progress.to_bits()
            && self.animator_started == other.animator_started
            && self.animator_ended == other.animator_ended
            && self.red_death_progress.to_bits() == other.red_death_progress.to_bits()
            && self.red_death_safe_radius.to_bits() == other.red_death_safe_radius.to_bits()
            && self.extensions == other.extensions
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
    fn rng_state(&mut self, state: RngState) {
        self.u64(state.seed);
        for word in state.words {
            self.u64(word);
        }
    }
    fn bools(&mut self, values: &[bool]) {
        self.u32(values.len() as u32);
        for &value in values {
            self.u8(u8::from(value));
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self
            .position
            .checked_add(len)
            .ok_or(SnapshotError::Truncated)?;
        if end > self.data.len() {
            return Err(SnapshotError::Truncated);
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, SnapshotError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn u32(&mut self) -> Result<u32, SnapshotError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn u64(&mut self) -> Result<u64, SnapshotError> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }
    fn rng_state(&mut self) -> Result<RngState, SnapshotError> {
        let seed = self.u64()?;
        let mut words = [0u64; 4];
        for word in &mut words {
            *word = self.u64()?;
        }
        let state = RngState::new(seed, words);
        if !state.is_valid() {
            return Err(SnapshotError::InvalidRngState);
        }
        Ok(state)
    }
    fn bools(&mut self) -> Result<Vec<bool>, SnapshotError> {
        let count = self.u32()? as usize;
        if count > MAX_BOOLS {
            return Err(SnapshotError::LimitExceeded("animator flag count"));
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(match self.u8()? {
                0 => false,
                1 => true,
                _ => return Err(SnapshotError::InvalidBool),
            });
        }
        Ok(values)
    }
    fn finish(&self) -> Result<(), SnapshotError> {
        if self.position == self.data.len() {
            Ok(())
        } else {
            Err(SnapshotError::TrailingData)
        }
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn streamed_sections_match_v1_payload_and_roundtrip() {
        let snapshot = sample_snapshot();
        let bytes = snapshot.encode().expect("encode");
        let payload = &bytes[SNAPSHOT_HEADER_LEN..bytes.len() - SNAPSHOT_CHECKSUM_LEN];
        let sections: Vec<_> = snapshot.encode_sections().expect("sections").collect();
        let mut joined = Vec::new();
        for (index, section) in sections.iter().enumerate() {
            if index > 0 {
                joined.extend_from_slice(&section.tag.to_le_bytes());
                joined.extend_from_slice(&(section.data.len() as u32).to_le_bytes());
            }
            joined.extend_from_slice(&section.data);
        }
        assert_eq!(joined, payload);
        let decoded = SimSnapshot::decode_sections(sections).expect("decode sections");
        assert_eq!(decoded, snapshot);
    }

    fn sample_snapshot() -> SimSnapshot {
        let mut rng = Rng::from_seed(0x5EED);
        for _ in 0..37 {
            let _ = rng.next_u64();
        }
        let mut snapshot = SimSnapshot::new(1_234, 30, 41.133_333, rng.state());
        snapshot.zone_rngs = vec![
            (0, Rng::derive(0x5EED, 1, 0).state()),
            (1, Rng::derive(0x5EED, 1, 1).state()),
        ];
        snapshot.species_name_rng = Some(Rng::from_seed(9).state());
        snapshot.spawner_accumulator = 0.875;
        snapshot.decay_period = 2;
        snapshot.decay_progress = 1;
        snapshot.animator_progress = 0.05;
        snapshot.animator_started = vec![true, false, true];
        snapshot.animator_ended = vec![false, false, true];
        snapshot.red_death_progress = 0.25;
        snapshot.red_death_safe_radius = f32::INFINITY;
        snapshot.push_extension(7, vec![1, 2, 3, 4]).unwrap();
        snapshot.push_extension(9, b"phase-2".to_vec()).unwrap();
        snapshot
    }

    #[test]
    fn rng_state_continues_the_exact_stream() {
        let mut original = Rng::from_seed(0x1234_5678);
        for _ in 0..100 {
            let _ = original.next_u64();
        }
        let state = original.state();
        let mut restored = Rng::from_state(state).expect("valid state");
        assert_eq!(restored.seed(), original.seed());
        for _ in 0..100 {
            assert_eq!(restored.next_u64(), original.next_u64());
        }
        // set_state works in place and keeps the seed used by fork/derive.
        let mut in_place = Rng::from_seed(1);
        assert!(in_place.set_state(state));
        assert_eq!(in_place.seed(), state.seed);
        let mut fork_a = in_place.fork(77);
        let mut fork_b = Rng::from_state(state).expect("valid").fork(77);
        for _ in 0..8 {
            assert_eq!(fork_a.next_u64(), fork_b.next_u64());
        }
    }

    #[test]
    fn rng_state_rejects_all_zero() {
        let zero = RngState::new(1, [0; 4]);
        assert!(!zero.is_valid());
        assert!(Rng::from_state(zero).is_none());
        let mut rng = Rng::from_seed(3);
        let before = rng.state();
        assert!(!rng.set_state(zero));
        assert_eq!(rng.state(), before);
    }

    #[test]
    fn snapshot_roundtrip_is_byte_deterministic() {
        let snapshot = sample_snapshot();
        let first = snapshot.encode().expect("encode");
        let second = snapshot.encode().expect("encode twice");
        assert_eq!(first, second, "encoding must be deterministic");
        let decoded = SimSnapshot::decode(&first).expect("decode");
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.encode().expect("re-encode"), first);
        assert_eq!(decoded.extension(7), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(decoded.extension(999), None);
    }

    #[test]
    fn snapshot_large_sections_are_chunked_and_reassemble() {
        let mut snapshot = sample_snapshot();
        let large: Vec<u8> = (0..70 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        snapshot
            .push_section(0x0101, large.clone())
            .expect("chunked push");
        assert!(snapshot.extension(0x0101).is_none(), "must be chunked");
        assert!(snapshot.extensions.len() > 1);
        let decoded = SimSnapshot::decode(&snapshot.encode().expect("encode")).expect("decode");
        let restored = decoded
            .extension_payload(0x0101)
            .expect("reassembled section");
        assert_eq!(restored, large);
        assert_eq!(decoded.extension_payload(0x0999), None);
    }

    #[test]
    fn snapshot_restores_clock_at_exact_tick() {
        let snapshot = sample_snapshot();
        let clock = snapshot.restore_clock().expect("clock");
        assert_eq!(clock.tps(), snapshot.sim_tps);
        assert_eq!(clock.tick(), snapshot.tick);
        assert!((clock.dt() - 1.0 / snapshot.sim_tps as f32).abs() < f32::EPSILON);
    }

    #[test]
    fn snapshot_preserves_float_bits() {
        let snapshot = SimSnapshot {
            spawner_accumulator: -0.0,
            animator_progress: f32::NAN,
            red_death_progress: f32::from_bits(0x7FA0_0001), // NaN payload
            simulated_time: f64::from_bits(0xFFF0_0000_0000_0001), // negative NaN
            ..SimSnapshot::default()
        };
        let decoded = SimSnapshot::decode(&snapshot.encode().expect("encode")).expect("decode");
        assert_eq!(decoded.spawner_accumulator.to_bits(), (-0.0f32).to_bits());
        assert!(decoded.animator_progress.is_nan());
        assert_eq!(decoded.animator_progress.to_bits(), f32::NAN.to_bits());
        assert_eq!(decoded.red_death_progress.to_bits(), 0x7FA0_0001);
        assert_eq!(decoded.simulated_time.to_bits(), 0xFFF0_0000_0000_0001);
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn snapshot_decodes_reject_corruption_and_truncation() {
        let bytes = sample_snapshot().encode().expect("encode");
        // Every strict prefix must fail cleanly (no panic).
        for cut in 0..bytes.len() {
            assert!(SimSnapshot::decode(&bytes[..cut]).is_err(), "cut {cut}");
        }
        let mut bad = bytes.clone();
        bad[0] ^= 0xFF;
        assert_eq!(SimSnapshot::decode(&bad), Err(SnapshotError::BadMagic));
        let mut bad = bytes.clone();
        bad[8..10].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            SimSnapshot::decode(&bad),
            Err(SnapshotError::UnsupportedVersion(99))
        );
        let mut bad = bytes.clone();
        bad[10..12].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            SimSnapshot::decode(&bad),
            Err(SnapshotError::UnsupportedFlags(1))
        );
        let mut bad = bytes.clone();
        let len = bad.len();
        bad[len - 1] ^= 0x01;
        assert_eq!(
            SimSnapshot::decode(&bad),
            Err(SnapshotError::ChecksumMismatch)
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            SimSnapshot::decode(&trailing),
            Err(SnapshotError::TrailingData)
        );
        // Payload length tag corruption: declare more than available.
        let mut bad = bytes.clone();
        let declared = u32::from_le_bytes([bad[12], bad[13], bad[14], bad[15]]);
        bad[12..16].copy_from_slice(&(declared + 1).to_le_bytes());
        assert_eq!(SimSnapshot::decode(&bad), Err(SnapshotError::Truncated));
    }

    #[test]
    fn snapshot_encoding_rejects_limits_and_invalid_rng() {
        let mut snapshot = SimSnapshot::default();
        assert_eq!(
            snapshot.push_extension(0, Vec::new()),
            Err(SnapshotError::LimitExceeded("extension tag 0 is reserved"))
        );
        let snapshot = SimSnapshot {
            sim_tps: 0,
            ..SimSnapshot::default()
        };
        assert_eq!(snapshot.encode(), Err(SnapshotError::InvalidSimTps));
        let snapshot = SimSnapshot {
            rng: RngState::new(0, [0; 4]),
            ..SimSnapshot::default()
        };
        assert_eq!(snapshot.encode(), Err(SnapshotError::InvalidRngState));
        let snapshot = SimSnapshot {
            zone_rngs: vec![(0, RngState::new(0, [0; 4]))],
            ..SimSnapshot::default()
        };
        assert_eq!(snapshot.encode(), Err(SnapshotError::InvalidRngState));
    }

    #[test]
    fn snapshot_default_roundtrip() {
        let snapshot = SimSnapshot::default();
        let decoded = SimSnapshot::decode(&snapshot.encode().expect("encode")).expect("decode");
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.zone_rngs.len(), 0);
        assert!(decoded.species_name_rng.is_none());
    }
}
