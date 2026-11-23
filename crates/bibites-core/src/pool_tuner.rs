//! How a tick runs (inline, or on the pool at one of a few levels of eagerness), decided by measuring it.
//!
//! The pool pays when a tick has enough work to share and costs when it has not, and where that line lies depends on the machine (how
//! many cores, how fast the OS hands a job to an idle one, what else is running) as much as on the world: 1,200 bibites is about the
//! line on a 6-core desktop with twelve threads, and the line is far lower on a 2-core laptop and far higher on a 32-core workstation. How
//! eagerly the stages of a tick that is on the pool should use it (from how many items each one is worth a hand-off) depends on the machine
//! in the same way. A size written into the code is right on one machine at best. [`PoolTuner`] therefore does not guess: it times the
//! world's own ticks in every way it can run them and keeps the fastest.
//!
//! The way a tick runs never changes what it computes (serial and pool ticks are bit-identical, whatever the mix), so the choice is only
//! about time, and a wrong choice costs a little time until the next measurement and nothing else.
//!
//! The ways ([`Arm`]), from the least to the most parallel: **inline** (on the calling thread, no handle to the pool: every stage runs
//! inline), and **on the pool at level 0 to 3** (the whole tick on one of the pool's workers; the stages that use the pool are the ones
//! that have the work for it, and a level is a multiple of the sizes that say how much work that is: level 0 the sizes measured on the
//! reference machine, each next level smaller, the last one every stage on the pool from one item).
//!
//! How it measures, in short:
//! - **A first guess** ([`Prior`]: the size from which the pool paid on the reference machine) decides which way to start. **A floor**
//!   (a quarter of it) is a size below which no machine's pool pays enough to matter (a tick of fifty bibites takes less time than waking the
//!   workers for it): such a world runs inline, is not timed and is never probed.
//! - **Steady running**: the *incumbent* way runs, and its mean tick time is kept (a moving average of the last few dozen ticks,
//!   the first ticks after a switch left out: they pay for cold caches and sleeping workers). A tick that takes eight times the mean is a
//!   hiccup (the OS, a pause), not a measurement: it is left out of the average, out of a block's mean, and it takes two of them in a
//!   probe to end it. Whatever the first block of ticks holds, one tick cannot decide the way a world runs.
//! - **Probes**: now and then another way runs for a block of ticks (12 to 100, about 200 ms of ticks at most) and its mean is compared
//!   with the incumbent's. A way that is clearly worse is dropped after a few ticks (two ticks ten times worse are enough). Every way has its own wait
//!   before it is looked at again, which doubles each time it loses (300 ticks, 600, up to 8,192): a settled world spends about 1 % of
//!   its ticks probing at most, and ways close to the incumbent in cost cost nothing to look at. The nearest ways are looked at first, and a switch is
//!   followed at once by a look at the next way in the same direction, so a world climbs the levels quickly when each is better than the last.
//!   (The cost along the ways is not always a slope: on a small pool the first level can lose to inline while the last one wins, so every way is
//!   looked at, not only the neighbours.)
//! - **A win is seen twice**: timings are noisy in one direction (another program takes a core, the OS pauses a thread, and a block of ticks
//!   is slower for it, never faster), so a way that wins a probe is not believed yet: the incumbent is measured again from scratch, the way
//!   is probed again, and only a second win in a row changes the way the world runs. A world that sits on a near tie does not wander.
//! - **The line is not symmetrical**: the pool takes over from inline only when it takes less than 90 % of the inline time (it also takes
//!   cores from everything else on the machine, the renderer included), inline takes back when it is within 5 % of the pool's, and between
//!   levels of the pool a more eager one has to be 4 % better while a less eager one only has to be no worse (so a near tie goes to the
//!   level that asks less of the machine, and no chain of switches can lead round to where it began).
//! - **The world changes**: when its size has moved by 41 % since the measurements, they are taken again (the incumbent keeps running
//!   until a probe says otherwise).
//!
//! The tuner is a plain state machine without a clock: [`PoolTuner::choose`] says how the next tick runs, the caller times it and tells
//! [`PoolTuner::record`]. The tests drive it with simulated machines.

use std::cmp::Reverse;

/// Levels of the pool a tick can run at (see the module's documentation).
pub const POOLED_LEVELS: usize = 4;
/// Ways a tick can run: inline, and every level of the pool.
pub const ARMS: usize = 1 + POOLED_LEVELS;

/// How a tick runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arm {
    /// On the calling thread, with no handle to the pool: every stage inline (the serial tick).
    Inline,
    /// On one of the pool's workers, every stage that has the work for it on the pool. The level (0 to [`POOLED_LEVELS`] - 1) says how eagerly: 0 the sizes
    /// that say how much work a stage needs are those measured on the reference machine, each next level has smaller ones.
    Pooled(u8),
}

impl Arm {
    /// Every way, from the least to the most parallel.
    pub const ALL: [Self; ARMS] = [
        Self::Inline,
        Self::Pooled(0),
        Self::Pooled(1),
        Self::Pooled(2),
        Self::Pooled(3),
    ];

    /// The position of this way in [`Arm::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Inline => 0,
            Self::Pooled(level) => 1 + level as usize,
        }
    }

    /// The way at a position of [`Arm::ALL`] (the last one for a position past the end).
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        if index == 0 {
            Self::Inline
        } else if index >= ARMS {
            Self::Pooled(POOLED_LEVELS as u8 - 1)
        } else {
            Self::Pooled((index - 1) as u8)
        }
    }

    /// Whether the tick runs on the pool.
    #[must_use]
    pub const fn is_pooled(self) -> bool {
        matches!(self, Self::Pooled(_))
    }

    /// The level of the pool (0 for inline, which has none).
    #[must_use]
    pub const fn level(self) -> u8 {
        match self {
            Self::Inline => 0,
            Self::Pooled(level) => level,
        }
    }

    /// A few words for the logs.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Inline => "inline".to_owned(),
            Self::Pooled(level) => format!("on the pool (level {level})"),
        }
    }
}

/// How the next tick runs, and whether to time it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The way the tick runs.
    pub arm: Arm,
    /// Whether the caller times the tick and passes it to [`PoolTuner::record`] (a world too small for the pool is not timed).
    pub timed: bool,
}

/// The size of a world as the tuner sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldSize {
    /// Live bibites.
    pub bibites: usize,
    /// Bodies of the physics step (bibites, pellets and eggs).
    pub bodies: usize,
}

impl WorldSize {
    /// One number for the size: the tick's cost follows the bibites, and the bodies a fourth as hard.
    fn metric(self) -> usize {
        self.bibites.max(self.bodies / 4)
    }
}

/// The first guess: the sizes from which the pool paid on the machine the sizes were measured on. It decides which way a world starts and
/// where the floor is; the measurements decide the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prior {
    /// Bibites from which the pool is expected to pay.
    pub bibites: usize,
    /// Bodies from which the pool is expected to pay.
    pub bodies: usize,
    /// The level of the pool ([`Arm::Pooled`]) a world big enough for the pool starts at (0 on the reference machine; a calibration
    /// of the machine says which level was the fastest on it, which saves the world the climb).
    pub level: u8,
}

/// What the tuner did so far (for the logs and the benchmarks).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PoolTunerStats {
    /// Ticks that ran inline.
    pub inline_ticks: u64,
    /// Ticks that ran on the pool (at any level).
    pub pooled_ticks: u64,
    /// Ticks that ran on the pool, by level.
    pub level_ticks: [u64; POOLED_LEVELS],
    /// Probes of another way started.
    pub probes: u32,
    /// Probes dropped early because the other way was clearly worse.
    pub aborted_probes: u32,
    /// Times the incumbent way changed.
    pub switches: u32,
    /// The latest mean tick time of each way in nanoseconds, by [`Arm::index`] (`None`: never measured).
    pub arm_ns: [Option<f64>; ARMS],
}

impl PoolTunerStats {
    /// The latest mean tick time inline, in nanoseconds.
    #[must_use]
    pub fn inline_ns(&self) -> Option<f64> {
        self.arm_ns[0]
    }

    /// The lowest of the latest mean tick times on the pool (any level), in nanoseconds.
    #[must_use]
    pub fn pooled_ns(&self) -> Option<f64> {
        self.arm_ns[1..].iter().flatten().copied().reduce(f64::min)
    }
}

/// A change of the incumbent way, for the logs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoolSwitch {
    /// The way the world ran until now.
    pub from: Arm,
    /// The way the world runs from now on.
    pub to: Arm,
    /// The tick (counted by the tuner) at which it changed.
    pub tick: u64,
    /// The size metric of the world then (bibites, or a fourth of the bodies if more).
    pub size: usize,
    /// The mean tick time of the way it left and of the way it took, in nanoseconds, that decided it.
    pub from_ns: f64,
    /// See [`PoolSwitch::from_ns`].
    pub to_ns: f64,
}

/// Ticks after a switch of way that are not measured (cold caches, sleeping workers).
const WARM_UP_TICKS: u32 = 3;
/// Fewest ticks of a block (a steady mean, a probe).
const MIN_BLOCK: u32 = 12;
/// Most ticks of a block (a world whose ticks take a fraction of a millisecond is measured over a few dozen milliseconds, over the hiccups of the OS).
const MAX_BLOCK: u32 = 100;
/// The time a block of ticks should take at most, in nanoseconds (a block of slow ticks is short).
const BLOCK_NANOS: f64 = 200_000_000.0;
/// Ticks a way waits before it is looked at again after it lost, at first.
const MIN_PERIOD: u32 = 300;
/// Most ticks a way waits.
const MAX_PERIOD: u32 = 8_192;
/// Ticks of the incumbent before the way next to a new incumbent, in the direction it moved, is looked at.
const MOMENTUM_WAIT: u32 = 24;
/// Ticks of the incumbent that each step of distance beyond the first adds to the wait before a way is looked at, when the measuring starts.
const FAR_WAIT: u32 = 40;
/// Measured ticks of a probe after which a way that is twice as bad as the incumbent is dropped.
const SOFT_ABORT_TICKS: u32 = 4;
/// A probe is dropped at once by a second tick (even a warm-up one) this many times the incumbent's mean (one tick that bad may be a hiccup).
const HARD_ABORT_FACTOR: f64 = 10.0;
/// A probe whose mean is this many times the incumbent's is dropped after [`SOFT_ABORT_TICKS`] (a way that is half as slow again as the
/// incumbent is not worth the rest of its block: that is most of the cost of looking at ways that lose).
const SOFT_ABORT_FACTOR: f64 = 1.5;
/// A tick this many times the mean is a hiccup, not a measurement.
const SPIKE_DROP_FACTOR: f64 = 8.0;
/// The pool takes over from inline when its mean is under this share of the inline mean.
const TO_POOLED: f64 = 0.90;
/// A more eager level of the pool takes over when its mean is under this share of the incumbent's.
const TO_MORE_EAGER: f64 = 0.96;
/// Inline takes back when its mean is under this share of the pool's mean (so, within 5 % above it).
const TO_INLINE: f64 = 1.05;
/// A less eager level of the pool takes over when its mean is under this share of the incumbent's (it only has to be no worse: a near tie goes to the
/// level that asks less of the machine; a margin above 1 would let a world slide down the levels one step at a time, each within the margin, and climb
/// back in one jump, round and round).
const TO_LESS_EAGER: f64 = 1.0;
/// The size has changed enough for the measurements to be taken again when it is this many times (or one over it) what it was.
const SIZE_STEP: f64 = 1.41;
/// The floor is the prior over this: a world that small is never timed (a tick of it takes a fraction of a millisecond, which no machine needs
/// to make faster, and the pool pays at no machine's size).
const FLOOR_DIVISOR: usize = 4;
/// Weight of a new tick in the moving average of the incumbent's mean.
const EWMA_WEIGHT: f64 = 0.05;
/// One tick counts for at most this many times the mean (a heavy tick is real, up to a point: the cadence of the bibites' brains and senses).
const SPIKE_FACTOR: f64 = 4.0;
/// Ticks of the incumbent between a win and the second look at the way that won.
const CONFIRM_WAIT: u32 = 0;
/// Switches kept for [`PoolTuner::take_switches`].
const SWITCH_LOG: usize = 32;

/// The unfinished probe of another way.
#[derive(Clone, Copy, Debug, Default)]
struct Probe {
    /// The way ([`Arm::index`]).
    arm: usize,
    /// Ticks measured (warm-up ticks left out).
    count: u32,
    /// Their times, summed, in nanoseconds.
    sum: f64,
    /// Ticks so bad (see [`HARD_ABORT_FACTOR`]) that they were left out; a second one ends the probe.
    spikes: u32,
}

/// Chooses, by timing the ticks of a world, how each one runs (see the module's documentation).
#[derive(Clone, Debug)]
pub struct PoolTuner {
    prior: Prior,
    /// The way ([`Arm::index`]) the world runs between probes.
    incumbent: usize,
    /// The way of the tick that ran last (or runs now), and how many ticks in a row have run that way, that one included.
    current: usize,
    in_arm: u32,
    /// The incumbent's mean tick time in nanoseconds: that of its first block (without the hiccups) once that has been measured, then a moving average.
    steady: Option<f64>,
    /// The ticks of the first block so far, and how many.
    first: [f64; MIN_BLOCK as usize],
    first_count: u32,
    /// Whether the last tick of the incumbent was a hiccup (a second one in a row is taken as real).
    last_spike: bool,
    /// For every way: measured ticks of the incumbent to run before it is looked at, and how long it waits after its next loss.
    wait: [u32; ARMS],
    backoff: [u32; ARMS],
    /// The way that won its last probe and has to win another one (see the module's documentation).
    tentative: Option<usize>,
    probe: Option<Probe>,
    /// The size metric the measurements are for (0: none yet).
    measured_size: usize,
    ticks: u64,
    stats: PoolTunerStats,
    switches: Vec<PoolSwitch>,
}

impl PoolTuner {
    /// A tuner that starts from `prior`.
    #[must_use]
    pub fn new(prior: Prior) -> Self {
        Self {
            prior,
            incumbent: 0,
            current: 0,
            in_arm: 0,
            steady: None,
            first: [0.0; MIN_BLOCK as usize],
            first_count: 0,
            last_spike: false,
            wait: [0; ARMS],
            backoff: [MIN_PERIOD; ARMS],
            tentative: None,
            probe: None,
            measured_size: 0,
            ticks: 0,
            stats: PoolTunerStats::default(),
            switches: Vec::new(),
        }
    }

    /// Forgets everything measured (the pool changed size, the machine changed under the world).
    pub fn reset(&mut self) {
        *self = Self::new(self.prior);
    }

    /// What the tuner did so far (the incumbent's mean tick time is the live one).
    #[must_use]
    pub fn stats(&self) -> PoolTunerStats {
        let mut stats = self.stats;
        if let Some(mean) = self.steady {
            stats.arm_ns[self.incumbent] = Some(mean);
        }
        stats
    }

    /// The way the world runs between probes.
    #[must_use]
    pub fn incumbent(&self) -> Arm {
        Arm::from_index(self.incumbent)
    }

    /// The changes of the incumbent way since the last call (at most the last 32).
    pub fn take_switches(&mut self) -> Vec<PoolSwitch> {
        std::mem::take(&mut self.switches)
    }

    /// The way the world is expected to run from its size alone: the prior's level of the pool (0 unless a calibration said otherwise) from the
    /// prior's size.
    fn prior_arm(&self, size: WorldSize) -> usize {
        if size.bibites >= self.prior.bibites || size.bodies >= self.prior.bodies {
            1 + usize::from(self.prior.level).min(POOLED_LEVELS - 1)
        } else {
            0
        }
    }

    /// Whether the world is too small for any pool to pay.
    fn below_floor(&self, size: WorldSize) -> bool {
        size.bibites < self.prior.bibites / FLOOR_DIVISOR
            && size.bodies < self.prior.bodies / FLOOR_DIVISOR
    }

    /// Starts measuring for a world of this size (the incumbent stays). The ways next to the incumbent are looked at as soon as it is measured,
    /// the farther ones a little later each (every look is a block of ticks).
    fn begin_measuring(&mut self, metric: usize) {
        self.measured_size = metric;
        self.steady = None;
        self.first_count = 0;
        self.last_spike = false;
        self.tentative = None;
        for arm in 0..ARMS {
            self.wait[arm] = FAR_WAIT * arm.abs_diff(self.incumbent).saturating_sub(1) as u32;
        }
        self.backoff = [MIN_PERIOD; ARMS];
    }

    /// Whether the world has changed size enough for the measurements to be taken again.
    fn size_changed(&self, metric: usize) -> bool {
        let (now, then) = (metric as f64, self.measured_size as f64);
        now > then * SIZE_STEP || now * SIZE_STEP < then
    }

    /// Ticks of a probe block, from the incumbent's mean tick time.
    fn probe_block(&self) -> u32 {
        match self.steady {
            Some(mean) if mean > 0.0 => ((BLOCK_NANOS / mean) as u32).clamp(MIN_BLOCK, MAX_BLOCK),
            _ => MAX_BLOCK,
        }
    }

    /// The way to look at next, if one is due: the one that won its last look (to see it win twice), else the nearest to the incumbent, the more
    /// parallel of two at the same distance.
    fn due_arm(&self) -> Option<usize> {
        if let Some(arm) = self.tentative
            && self.wait[arm] == 0
        {
            return Some(arm);
        }
        (0..ARMS)
            .filter(|&arm| arm != self.incumbent && self.wait[arm] == 0)
            .min_by_key(|&arm| (arm.abs_diff(self.incumbent), Reverse(arm)))
    }

    /// How the next tick runs. Call [`PoolTuner::record`] with its time afterwards when [`Choice::timed`].
    pub fn choose(&mut self, size: WorldSize) -> Choice {
        if self.below_floor(size) {
            // No pool pays this small: inline, not timed, and what was measured is for a world that is gone.
            if self.measured_size != 0 {
                let (prior, stats, switches, ticks) = (
                    self.prior,
                    self.stats,
                    std::mem::take(&mut self.switches),
                    self.ticks,
                );
                *self = Self::new(prior);
                self.stats = stats;
                self.switches = switches;
                self.ticks = ticks;
            }
            self.enter(0);
            return Choice {
                arm: Arm::Inline,
                timed: false,
            };
        }
        let metric = size.metric();
        if self.measured_size == 0 {
            self.incumbent = self.prior_arm(size);
            self.begin_measuring(metric);
        } else if self.probe.is_none() && self.size_changed(metric) {
            self.begin_measuring(metric);
        }
        let arm = if let Some(probe) = self.probe {
            probe.arm
        } else if self.steady.is_some()
            && let Some(due) = self.due_arm()
        {
            self.probe = Some(Probe {
                arm: due,
                ..Probe::default()
            });
            self.stats.probes += 1;
            due
        } else {
            self.incumbent
        };
        self.enter(arm);
        Choice {
            arm: Arm::from_index(arm),
            timed: true,
        }
    }

    /// Notes that the next tick runs in the way at `arm`.
    fn enter(&mut self, arm: usize) {
        if arm == self.current {
            self.in_arm = self.in_arm.saturating_add(1);
        } else {
            self.current = arm;
            self.in_arm = 1;
        }
        self.ticks += 1;
        if arm == 0 {
            self.stats.inline_ticks += 1;
        } else {
            self.stats.pooled_ticks += 1;
            self.stats.level_ticks[arm - 1] += 1;
        }
    }

    /// The time in nanoseconds of the tick that [`PoolTuner::choose`] last chose (only when it said to time it).
    pub fn record(&mut self, nanos: u64) {
        let nanos = nanos as f64;
        if self.current != self.incumbent {
            self.record_probe_tick(nanos);
            return;
        }
        if self.in_arm <= WARM_UP_TICKS {
            return;
        }
        match self.steady {
            Some(mean) => {
                if nanos > SPIKE_DROP_FACTOR * mean && !self.last_spike {
                    // A hiccup: left out (a second one in a row is taken as real, below).
                    self.last_spike = true;
                } else {
                    self.last_spike = false;
                    let capped = nanos.min(SPIKE_FACTOR * mean);
                    self.steady = Some(mean + EWMA_WEIGHT * (capped - mean));
                }
            }
            None => {
                self.first[self.first_count as usize] = nanos;
                self.first_count += 1;
                if self.first_count >= MIN_BLOCK {
                    self.steady = Some(robust_mean(&self.first));
                }
            }
        }
        for wait in &mut self.wait {
            *wait = wait.saturating_sub(1);
        }
    }

    /// A tick of the probe that runs.
    fn record_probe_tick(&mut self, nanos: f64) {
        let (Some(steady), Some(probe)) = (self.steady, self.probe.as_mut()) else {
            return;
        };
        // A tick that bad, warm-up or not, is left out; one may be a hiccup, a second one proves the way hopeless.
        if nanos > HARD_ABORT_FACTOR * steady {
            probe.spikes += 1;
            if probe.spikes >= 2 {
                self.finish_probe(nanos, Abort::Hard);
            }
            return;
        }
        if self.in_arm <= WARM_UP_TICKS {
            return;
        }
        probe.count += 1;
        probe.sum += nanos.min(SPIKE_FACTOR * steady);
        let mean = probe.sum / f64::from(probe.count);
        if probe.count >= SOFT_ABORT_TICKS && mean > SOFT_ABORT_FACTOR * steady {
            self.finish_probe(mean, Abort::Soft);
        } else if probe.count >= self.probe_block() {
            self.finish_probe(mean, Abort::No);
        }
    }

    /// Ends the probe with its mean tick time, and decides.
    fn finish_probe(&mut self, challenger_mean: f64, abort: Abort) {
        let Some(probe) = self.probe.take() else {
            return;
        };
        let (challenger, incumbent) = (probe.arm, self.incumbent);
        let incumbent_mean = self.steady.unwrap_or(challenger_mean);
        let margin = if challenger > incumbent {
            if incumbent == 0 {
                TO_POOLED
            } else {
                TO_MORE_EAGER
            }
        } else if challenger == 0 {
            TO_INLINE
        } else {
            TO_LESS_EAGER
        };
        let wins = challenger_mean < incumbent_mean * margin;
        self.stats.arm_ns[challenger] = Some(challenger_mean);
        self.stats.arm_ns[incumbent] = Some(incumbent_mean);
        if abort != Abort::No {
            self.stats.aborted_probes += 1;
        }
        if wins && self.tentative != Some(challenger) {
            // The first win is not believed: the incumbent is measured again from scratch (an independent look at it), and the way that won is
            // probed again as soon as that is done.
            self.tentative = Some(challenger);
            self.steady = None;
            self.first_count = 0;
            self.last_spike = false;
            self.wait[challenger] = CONFIRM_WAIT;
        } else if wins {
            self.tentative = None;
            self.incumbent = challenger;
            self.steady = Some(challenger_mean);
            self.last_spike = false;
            self.stats.switches += 1;
            if self.switches.len() >= SWITCH_LOG {
                self.switches.remove(0);
            }
            self.switches.push(PoolSwitch {
                from: Arm::from_index(incumbent),
                to: Arm::from_index(challenger),
                tick: self.ticks,
                size: self.measured_size,
                from_ns: incumbent_mean,
                to_ns: challenger_mean,
            });
            // The way it left is looked at again at the normal pace, to check the switch; the next one in the direction it moved, soon.
            self.backoff[incumbent] = MIN_PERIOD;
            self.wait[incumbent] = MIN_PERIOD;
            self.backoff[challenger] = MIN_PERIOD;
            let onward = if challenger > incumbent {
                challenger + 1
            } else {
                challenger.wrapping_sub(1)
            };
            if onward < ARMS {
                self.wait[onward] = self.wait[onward].min(MOMENTUM_WAIT);
            }
        } else {
            if self.tentative == Some(challenger) {
                self.tentative = None;
            }
            // Every loss doubles the wait of the way (more after an abort), so a world where nothing beats the incumbent stops looking.
            let factor = match abort {
                Abort::Hard => 4,
                Abort::Soft => 3,
                Abort::No => 2,
            };
            self.backoff[challenger] =
                (self.backoff[challenger].saturating_mul(factor)).clamp(MIN_PERIOD, MAX_PERIOD);
            self.wait[challenger] = self.backoff[challenger];
        }
    }
}

/// The mean of a block of ticks without its hiccups (the ticks more than [`SPIKE_DROP_FACTOR`] times the block's median).
fn robust_mean(block: &[f64]) -> f64 {
    let mut sorted = [0.0; MIN_BLOCK as usize];
    let sorted = &mut sorted[..block.len()];
    sorted.copy_from_slice(block);
    sorted.sort_by(f64::total_cmp);
    let limit = SPIKE_DROP_FACTOR * sorted[sorted.len() / 2];
    let (sum, count) = block
        .iter()
        .filter(|&&tick| tick <= limit)
        .fold((0.0, 0.0), |(sum, count), &tick| (sum + tick, count + 1.0));
    sum / count
}

/// How a probe ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Abort {
    /// It ran its whole block.
    No,
    /// Its mean was half as much again as the incumbent's after a few ticks.
    Soft,
    /// Two ticks of it were ten times the incumbent's mean.
    Hard,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference machine's first guess.
    const PRIOR: Prior = Prior {
        bibites: 1_200,
        bodies: 3_000,
        level: 0,
    };

    /// A machine for the tests: what a tick costs in each way, in nanoseconds, as a fixed part and a part for every bibite of the world
    /// ([`Arm::index`] order: inline, then the levels of the pool).
    #[derive(Clone, Copy)]
    struct Machine {
        costs: [(f64, f64); ARMS],
    }

    impl Machine {
        fn cost(&self, arm: Arm, bibites: usize) -> f64 {
            let (fixed, per_bibite) = self.costs[arm.index()];
            fixed + per_bibite * bibites as f64
        }

        /// The cheapest way for a world of this size, and its cost.
        fn best(&self, bibites: usize) -> (Arm, f64) {
            Arm::ALL
                .into_iter()
                .map(|arm| (arm, self.cost(arm, bibites)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .expect("ways")
        }
    }

    /// The reference machine (a 6-core desktop with twelve threads): 0.98 us a bibite inline, 0.6 ms of fixed cost and 0.45 us a bibite on the
    /// pool at level 0 (about 1,200 bibites to break even), and ever more eager levels that cost more to start and a little less per bibite.
    const DESKTOP: Machine = Machine {
        costs: [
            (0.0, 980.0),
            (600_000.0, 450.0),
            (700_000.0, 440.0),
            (850_000.0, 420.0),
            (1_100_000.0, 400.0),
        ],
    };

    /// A small laptop with a pool of two: the tick on a worker without its stages costs more than it saves (level 0 never beats inline),
    /// and the more eager the stages the better (the last level beats inline from a few hundred bibites).
    const LAPTOP: Machine = Machine {
        costs: [
            (0.0, 1_400.0),
            (400_000.0, 1_500.0),
            (300_000.0, 1_250.0),
            (200_000.0, 1_000.0),
            (150_000.0, 800.0),
        ],
    };

    /// A big workstation: a cheap wake and a pool that takes the work apart ten ways; the eager levels only pay in a huge world.
    const WORKSTATION: Machine = Machine {
        costs: [
            (0.0, 800.0),
            (150_000.0, 100.0),
            (250_000.0, 95.0),
            (400_000.0, 90.0),
            (700_000.0, 85.0),
        ],
    };

    /// A machine whose pool never pays (the cores are taken by something else).
    const BUSY: Machine = Machine {
        costs: [
            (0.0, 900.0),
            (3_000_000.0, 1_100.0),
            (3_200_000.0, 1_050.0),
            (3_500_000.0, 1_000.0),
            (4_000_000.0, 950.0),
        ],
    };

    /// A small deterministic noise source.
    struct Noise(u64);

    impl Noise {
        fn unit(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as f64) / f64::from(1u32 << 31)
        }
    }

    /// What a run of the tuner on a machine cost, and what the best way would have.
    struct Run {
        total: f64,
        best: f64,
        fixed: [f64; ARMS],
        best_fixed: f64,
        worst_fixed: f64,
        tuner: PoolTuner,
        timed_ticks: u64,
    }

    /// Runs the tuner for `ticks` ticks on `machine`, for a world of `size_at(tick)` bibites, with `noise` (a share) on every tick and a
    /// spike of twenty times now and then when `spikes`.
    fn drive(
        prior: Prior,
        machine: Machine,
        ticks: u64,
        size_at: impl Fn(u64) -> usize,
        noise: f64,
        spikes: bool,
    ) -> Run {
        let mut tuner = PoolTuner::new(prior);
        let mut rng = Noise(0x5EED);
        let (mut total, mut best) = (0.0, 0.0);
        let mut fixed = [0.0; ARMS];
        let mut timed_ticks = 0;
        for tick in 0..ticks {
            let bibites = size_at(tick);
            let choice = tuner.choose(WorldSize {
                bibites,
                bodies: bibites,
            });
            let jitter = 1.0 + noise * (rng.unit() * 2.0 - 1.0);
            let spike = if spikes && tick % 97 == 13 { 20.0 } else { 1.0 };
            let cost = machine.cost(choice.arm, bibites) * jitter * spike;
            if choice.timed {
                tuner.record(cost as u64);
                timed_ticks += 1;
            }
            total += cost;
            for arm in Arm::ALL {
                fixed[arm.index()] += machine.cost(arm, bibites) * jitter * spike;
            }
            best += machine.best(bibites).1 * jitter * spike;
        }
        Run {
            total,
            best,
            fixed,
            best_fixed: fixed.into_iter().fold(f64::INFINITY, f64::min),
            worst_fixed: fixed.into_iter().fold(0.0, f64::max),
            tuner,
            timed_ticks,
        }
    }

    fn size(bibites: usize) -> WorldSize {
        WorldSize {
            bibites,
            bodies: bibites,
        }
    }

    #[test]
    fn a_world_big_enough_for_the_pool_starts_at_the_level_a_calibration_found_and_a_small_one_inline() {
        // The prior's level is where a world that has reached the prior's size starts; below the size it starts inline whatever the level.
        for level in 0..POOLED_LEVELS as u8 {
            let mut tuner = PoolTuner::new(Prior { level, ..PRIOR });
            assert_eq!(tuner.choose(size(5_000)).arm, Arm::Pooled(level), "level {level}");
            assert_eq!(tuner.incumbent(), Arm::Pooled(level));
            let mut small = PoolTuner::new(Prior { level, ..PRIOR });
            assert_eq!(small.choose(size(500)).arm, Arm::Inline, "level {level}: below the prior's size");
        }
        // A level past the last one is the last one (a stored number from a build with more levels).
        let mut tuner = PoolTuner::new(Prior { level: 9, ..PRIOR });
        assert_eq!(tuner.choose(size(5_000)).arm, Arm::Pooled(POOLED_LEVELS as u8 - 1));
        // And it is only a start: a machine where another level is better moves there on its own.
        let mut tuner = PoolTuner::new(Prior { level: 3, ..PRIOR });
        let mut level_one = DESKTOP;
        level_one.costs[2] = (level_one.costs[2].0 * 0.5, level_one.costs[2].1 * 0.5);
        for _ in 0..20_000 {
            let choice = tuner.choose(size(5_000));
            tuner.record(level_one.cost(choice.arm, 5_000) as u64);
        }
        assert_eq!(tuner.incumbent(), Arm::Pooled(1), "{:?}", tuner.stats());
    }

    #[test]
    fn the_arms_are_numbered_from_inline_to_the_most_eager_level() {
        for (index, arm) in Arm::ALL.into_iter().enumerate() {
            assert_eq!(arm.index(), index);
            assert_eq!(Arm::from_index(index), arm);
        }
        assert!(Arm::Inline < Arm::Pooled(0) && Arm::Pooled(0) < Arm::Pooled(3));
        assert_eq!(
            Arm::from_index(ARMS + 7),
            Arm::Pooled(POOLED_LEVELS as u8 - 1)
        );
        assert!(!Arm::Inline.is_pooled() && Arm::Pooled(2).is_pooled());
        assert_eq!((Arm::Inline.level(), Arm::Pooled(2).level()), (0, 2));
        assert_eq!(Arm::Inline.label(), "inline");
        assert_eq!(Arm::Pooled(1).label(), "on the pool (level 1)");
    }

    #[test]
    fn a_world_below_the_floor_runs_inline_untimed_and_is_never_probed() {
        let run = drive(PRIOR, DESKTOP, 5_000, |_| 250, 0.05, false);
        let stats = run.tuner.stats();
        assert_eq!(run.timed_ticks, 0);
        assert_eq!(stats.pooled_ticks, 0);
        assert_eq!(stats.probes, 0);
        assert_eq!(stats.inline_ticks, 5_000);
        // Costs exactly what the inline tick costs: no exploration at all.
        assert!((run.total - run.best_fixed).abs() < 1e-6 * run.best_fixed);
    }

    #[test]
    fn every_machine_ends_on_a_way_close_to_the_best_for_the_size() {
        for (name, machine) in [
            ("desktop", DESKTOP),
            ("laptop", LAPTOP),
            ("workstation", WORKSTATION),
            ("busy", BUSY),
        ] {
            for bibites in [300, 600, 1_000, 1_500, 3_000, 10_000, 50_000] {
                let run = drive(PRIOR, machine, 8_000, |_| bibites, 0.03, false);
                let (best, best_cost) = machine.best(bibites);
                let settled = run.tuner.incumbent();
                let settled_cost = machine.cost(settled, bibites);
                // A way within a few per cent of the best is as good (the margins keep a world where it is on a near tie: the pool has to be 10 %
                // better than inline to take the world from it, so inline may be 11 % over the best way).
                assert!(
                    settled_cost <= best_cost * 1.12,
                    "{name}, {bibites} bibites: settled on {settled:?} ({:.0} us a tick) but {best:?} costs {:.0} us ({:?} ns means, {} probes, {} switches)",
                    settled_cost / 1_000.0,
                    best_cost / 1_000.0,
                    run.tuner.stats().arm_ns,
                    run.tuner.stats().probes,
                    run.tuner.stats().switches
                );
            }
        }
    }

    #[test]
    fn the_tuner_is_never_far_from_the_better_way_on_any_machine() {
        // Every machine of the table, every size: the exploration costs a few per cent of a run of a few thousand ticks, and less as it goes on.
        let mut worst: f64 = 1.0;
        for (name, machine) in [
            ("desktop", DESKTOP),
            ("laptop", LAPTOP),
            ("workstation", WORKSTATION),
            ("busy", BUSY),
        ] {
            for bibites in [300, 600, 1_000, 1_500, 3_000, 10_000, 50_000] {
                let run = drive(PRIOR, machine, 8_000, |_| bibites, 0.03, true);
                let regret = run.total / run.best_fixed;
                worst = worst.max(regret);
                // The pool has to be 10 % better than inline to take a world from it (the cores are worth something): inline may be 11 % over the best way.
                assert!(
                    regret < 1.12,
                    "{name}, {bibites} bibites: {:.1} % over the best fixed way (settled on {:?}, {} probes, {} switches)",
                    (regret - 1.0) * 100.0,
                    run.tuner.incumbent(),
                    run.tuner.stats().probes,
                    run.tuner.stats().switches
                );
                // And what looking costs, the wrong way at the start included (a world that starts on inline takes a few hundred ticks to look at
                // every way and see the best one win twice): not much over what the way it settled on costs from the start.
                let settled = run.fixed[run.tuner.incumbent().index()];
                assert!(
                    run.total < settled * 1.10,
                    "{name}, {bibites} bibites: {:.1} % over the way it settled on ({:?}, {} probes, {} switches)",
                    (run.total / settled - 1.0) * 100.0,
                    run.tuner.incumbent(),
                    run.tuner.stats().probes,
                    run.tuner.stats().switches
                );
                // Never worse than the worst fixed way either, which is what a wrong fixed choice costs.
                assert!(run.total <= run.worst_fixed * 1.01);
            }
        }
        assert!(worst < 1.12, "{worst}");
    }

    #[test]
    fn a_pool_that_loses_at_level_zero_but_wins_at_the_last_level_is_found() {
        // The laptop's level 0 is worse than inline at every size (the tick on a worker, no stage to gain from it), the last level wins from
        // a few hundred bibites: the neighbours of inline say no, and the tuner has to look past them.
        for bibites in [500, 1_000, 3_000, 20_000] {
            let run = drive(PRIOR, LAPTOP, 6_000, |_| bibites, 0.02, false);
            let (best, best_cost) = LAPTOP.best(bibites);
            assert!(best.is_pooled(), "{bibites}: {best:?}");
            let settled = run.tuner.incumbent();
            assert!(
                LAPTOP.cost(settled, bibites) <= best_cost * 1.07,
                "{bibites} bibites: settled on {settled:?}, best {best:?}"
            );
            assert!(settled.is_pooled(), "{bibites} bibites: {settled:?}");
        }
    }

    #[test]
    fn the_levels_are_climbed_one_after_the_other_when_each_is_better_than_the_last() {
        // A machine on which every level beats the one before by 9 % or more in a world of 100,000 bibites.
        const CLIMB: Machine = Machine {
            costs: [
                (0.0, 800.0),
                (150_000.0, 100.0),
                (200_000.0, 90.0),
                (250_000.0, 80.0),
                (300_000.0, 70.0),
            ],
        };
        let mut run = drive(PRIOR, CLIMB, 3_000, |_| 100_000, 0.01, false);
        assert_eq!(run.tuner.incumbent(), Arm::Pooled(3));
        let switches = run.tuner.take_switches();
        assert!(
            (3..=4).contains(&switches.len()),
            "{} switches: {switches:?}",
            switches.len()
        );
        // Each switch goes one level further, and the climb is quick (the next level is looked at at once after a win).
        for pair in switches.windows(2) {
            assert!(pair[1].tick - pair[0].tick < 600, "{pair:?}");
        }
    }

    #[test]
    fn the_exploration_gets_cheaper_the_longer_a_world_runs() {
        // A world where the pool is hopeless at every level (the worst case for the cost of looking): at least 3 ms a tick more on the pool.
        let short = drive(PRIOR, BUSY, 1_000, |_| 2_000, 0.02, false);
        let long = drive(PRIOR, BUSY, 40_000, |_| 2_000, 0.02, false);
        let longer = drive(PRIOR, BUSY, 160_000, |_| 2_000, 0.02, false);
        let overhead = |run: &Run| run.total / run.best_fixed - 1.0;
        assert!(overhead(&long) < 0.015, "{}", overhead(&long));
        // What is left is the soft aborts of three ways that are three times worse, once every 8,192 ticks (the longest wait): under 1 %.
        assert!(overhead(&longer) < 0.011, "{}", overhead(&longer));
        assert!(overhead(&longer) < overhead(&long) && overhead(&long) < overhead(&short) + 1e-9);
        // The probes thin out: a few dozen per way in 40,000 ticks.
        assert!(
            long.tuner.stats().probes < 60,
            "{}",
            long.tuner.stats().probes
        );
    }

    #[test]
    fn one_good_block_does_not_change_the_way_a_world_runs_but_two_do() {
        // At 600 bibites inline is the better way on the reference machine, and level 0 of the pool costs half as much again. A lucky first
        // look at level 0 (half the inline cost for the first 130 ticks, as if the machine had been busy while the incumbent was measured) must
        // not move the world: the second look says what it costs.
        let mut tuner = PoolTuner::new(PRIOR);
        for tick in 0..3_000 {
            let choice = tuner.choose(size(600));
            let cost = if choice.arm == Arm::Pooled(0) && tick < 130 {
                300_000.0
            } else {
                DESKTOP.cost(choice.arm, 600)
            };
            tuner.record(cost as u64);
        }
        assert_eq!(tuner.incumbent(), Arm::Inline);
        assert_eq!(tuner.stats().switches, 0, "{:?}", tuner.stats());
        // The same luck twice (the pool really costs half as much): the world moves, once, and only after the second look.
        let mut tuner = PoolTuner::new(PRIOR);
        let mut first_switch_tick = None;
        for tick in 0..3_000 {
            let choice = tuner.choose(size(600));
            let cost = if choice.arm == Arm::Pooled(0) {
                300_000.0
            } else {
                DESKTOP.cost(choice.arm, 600)
            };
            tuner.record(cost as u64);
            if first_switch_tick.is_none() && tuner.stats().switches > 0 {
                first_switch_tick = Some(tick);
            }
        }
        assert!(tuner.incumbent().is_pooled());
        // Two blocks of the way (a hundred ticks each) and the incumbent measured again between them: the switch cannot come before the second ends.
        assert!(
            first_switch_tick.expect("switched") > 200,
            "{first_switch_tick:?}"
        );
    }

    #[test]
    fn a_near_tie_under_bursts_of_load_does_not_make_a_world_wander() {
        // The last level is 4 % better than inline at 1,000 bibites (not worth leaving inline for), and every now and then something else on
        // the machine slows the ticks by half for a few dozen of them (one-sided noise that is as long as a block): the world must not
        // switch back and forth, and must not pay more than a few per cent for looking.
        let tie = Machine {
            costs: [
                (0.0, 1_000.0),
                (0.0, 1_250.0),
                (0.0, 1_250.0),
                (0.0, 1_250.0),
                (0.0, 960.0),
            ],
        };
        let mut tuner = PoolTuner::new(PRIOR);
        let mut rng = Noise(0xB0B);
        let (mut total, mut inline_total, mut burst_left, mut burst_factor) =
            (0.0, 0.0, 0_u32, 1.0);
        for _ in 0..30_000 {
            if burst_left == 0 && rng.unit() < 1.0 / 150.0 {
                burst_left = 15 + (rng.unit() * 60.0) as u32;
                burst_factor = 1.5 + rng.unit();
            }
            let disturbance = if burst_left > 0 {
                burst_left -= 1;
                burst_factor
            } else {
                1.0
            };
            let choice = tuner.choose(size(1_000));
            let cost =
                tie.cost(choice.arm, 1_000) * (1.0 + 0.03 * (rng.unit() * 2.0 - 1.0)) * disturbance;
            tuner.record(cost as u64);
            total += cost;
            inline_total += tie.cost(Arm::Inline, 1_000) * disturbance;
        }
        let stats = tuner.stats();
        // A few moves between ways that cost the same within 4 % are harmless; a world that wandered all the time would show here.
        assert!(
            stats.switches <= 10,
            "{} switches: {stats:?}",
            stats.switches
        );
        // What staying inline all the time would have cost (the near tie is not worth leaving it for): looking at the others costs a few per cent.
        assert!(total / inline_total < 1.04, "{}", total / inline_total);
    }

    #[test]
    fn a_hiccup_in_the_first_block_does_not_send_a_world_to_the_wrong_way() {
        // Inline is the better way on the reference machine at 600 bibites. A tick twenty times the others in the first block of ticks (and one in
        // every probe) must not make the pool look good: the world stays inline from the start to the end.
        let mut tuner = PoolTuner::new(PRIOR);
        let mut rng = Noise(0xFACE);
        let (mut pooled_ticks_after_start, mut switches_seen) = (0, 0);
        for tick in 0..4_000 {
            let choice = tuner.choose(size(600));
            let spike = if tick % 13 == 7 { 20.0 } else { 1.0 };
            let jitter = 1.0 + 0.02 * (rng.unit() * 2.0 - 1.0);
            tuner.record((DESKTOP.cost(choice.arm, 600) * jitter * spike) as u64);
            if tick >= 1_000 && choice.arm.is_pooled() {
                pooled_ticks_after_start += 1;
            }
            switches_seen = tuner.stats().switches;
        }
        assert_eq!(tuner.incumbent(), Arm::Inline);
        assert_eq!(switches_seen, 0, "{:?}", tuner.stats());
        // After the first thousand ticks only the thin probes run on the pool.
        assert!(pooled_ticks_after_start < 400, "{pooled_ticks_after_start}");
    }

    #[test]
    fn the_mean_of_a_block_leaves_out_its_hiccups_and_keeps_its_heavy_ticks() {
        // Twelve ticks of 100 with one of 2,000 (a hiccup): 100. A cadence of heavy ticks (every third one five times the rest) stays.
        let mut block = [100.0; 12];
        block[7] = 2_000.0;
        assert!((robust_mean(&block) - 100.0).abs() < 1e-9);
        let cadence: [f64; 12] =
            std::array::from_fn(|index| if index % 3 == 0 { 500.0 } else { 100.0 });
        assert!(
            (robust_mean(&cadence) - 233.333_333).abs() < 1e-3,
            "{}",
            robust_mean(&cadence)
        );
        assert!((robust_mean(&[7.0; 12]) - 7.0).abs() < 1e-12);
    }

    #[test]
    fn a_near_tie_does_not_flap_and_stays_on_the_less_parallel_way() {
        // The pool is 3 % better: not worth the cores (it has to be 10 % better), so inline keeps the world.
        let tie = Machine {
            costs: [
                (0.0, 1_000.0),
                (0.0, 970.0),
                (0.0, 1_300.0),
                (0.0, 1_300.0),
                (0.0, 1_300.0),
            ],
        };
        let run = drive(PRIOR, tie, 20_000, |_| 2_000, 0.03, false);
        assert_eq!(run.tuner.incumbent(), Arm::Inline);
        assert!(
            run.tuner.stats().switches <= 1,
            "{}",
            run.tuner.stats().switches
        );
        // The pool is 15 % better: it takes the world, once.
        let better = Machine {
            costs: [
                (0.0, 1_000.0),
                (0.0, 850.0),
                (0.0, 1_300.0),
                (0.0, 1_300.0),
                (0.0, 1_300.0),
            ],
        };
        let run = drive(PRIOR, better, 20_000, |_| 2_000, 0.03, false);
        assert_eq!(run.tuner.incumbent(), Arm::Pooled(0));
        assert!(
            run.tuner.stats().switches <= 2,
            "{}",
            run.tuner.stats().switches
        );
        // Two levels within 2 % of each other: the less eager one keeps the world (a more eager one has to be 4 % better).
        let close = Machine {
            costs: [
                (0.0, 2_000.0),
                (0.0, 1_000.0),
                (0.0, 980.0),
                (0.0, 1_500.0),
                (0.0, 1_500.0),
            ],
        };
        let run = drive(PRIOR, close, 20_000, |_| 2_000, 0.03, false);
        assert_eq!(run.tuner.incumbent(), Arm::Pooled(0));
        assert!(
            run.tuner.stats().switches <= 2,
            "{}",
            run.tuner.stats().switches
        );
    }

    #[test]
    fn noise_and_spikes_do_not_decide_it() {
        // A 30 % better pool under 10 % noise and a twentyfold spike every 97 ticks still wins, and does not flap.
        let machine = Machine {
            costs: [
                (0.0, 1_000.0),
                (0.0, 700.0),
                (0.0, 1_100.0),
                (0.0, 1_100.0),
                (0.0, 1_100.0),
            ],
        };
        let run = drive(PRIOR, machine, 30_000, |_| 3_000, 0.10, true);
        assert_eq!(run.tuner.incumbent(), Arm::Pooled(0));
        assert!(
            run.tuner.stats().switches <= 3,
            "{}",
            run.tuner.stats().switches
        );
        assert!(
            run.total / run.best_fixed < 1.04,
            "{}",
            run.total / run.best_fixed
        );
    }

    #[test]
    fn a_world_that_grows_moves_to_the_pool_where_it_starts_to_pay() {
        // 300 bibites growing to 12,000 over 30,000 ticks on the reference machine (it pays from about 1,200): the pool takes over
        // within a few thousand bibites of that and the run costs less than 5 % more than the best way at every size.
        let ticks = 30_000;
        let size_at = |tick: u64| 300 + (tick as usize * 12_000 / ticks as usize);
        let run = drive(PRIOR, DESKTOP, ticks, size_at, 0.03, false);
        assert!(run.total / run.best < 1.05, "{}", run.total / run.best);
        assert!(run.tuner.incumbent().is_pooled());
        let switches = run.tuner.stats().switches;
        assert!((1..=6).contains(&switches), "{switches}");
        // And it shrinks back: 12,000 bibites down to 300.
        let size_at = |tick: u64| 12_000 - (tick as usize * 11_700 / ticks as usize);
        let run = drive(PRIOR, DESKTOP, ticks, size_at, 0.03, false);
        assert!(run.total / run.best < 1.05, "{}", run.total / run.best);
        assert_eq!(run.tuner.incumbent(), Arm::Inline);
    }

    #[test]
    fn the_first_guess_only_decides_where_it_starts() {
        // A machine on which the pool pays from 150 bibites (a workstation with a quick wake) and the prior says 1,200: the probe finds out.
        let quick = Machine {
            costs: [
                (0.0, 1_000.0),
                (20_000.0, 150.0),
                (40_000.0, 150.0),
                (80_000.0, 150.0),
                (160_000.0, 150.0),
            ],
        };
        let run = drive(PRIOR, quick, 3_000, |_| 500, 0.02, false);
        assert_eq!(run.tuner.incumbent(), Arm::Pooled(0));
        // And a machine on which it never pays from 5,000 (a laptop whose cores are busy): the probe finds out too.
        let slow = Machine {
            costs: [
                (0.0, 1_000.0),
                (8_000_000.0, 800.0),
                (8_000_000.0, 800.0),
                (8_000_000.0, 800.0),
                (8_000_000.0, 800.0),
            ],
        };
        let run = drive(PRIOR, slow, 3_000, |_| 5_000, 0.02, false);
        assert_eq!(run.tuner.incumbent(), Arm::Inline);
    }

    #[test]
    fn a_switch_is_logged_once_with_the_times_that_decided_it() {
        let mut run = drive(PRIOR, DESKTOP, 2_000, |_| 20_000, 0.0, false);
        // The prior says pool for 20,000 bibites: it may climb a level or two (the eager levels are a little cheaper there), and every
        // switch is logged with the means of the two ways.
        for switch in run.tuner.take_switches() {
            assert!(switch.to_ns < switch.from_ns, "{switch:?}");
            assert_ne!(switch.from, switch.to);
        }
        assert!(run.tuner.stats().inline_ns().unwrap() > run.tuner.stats().pooled_ns().unwrap());
        // A machine that contradicts the prior (400 bibites, a quick pool): the first switch is from inline to the pool, with the means that decided it.
        let quick = Machine {
            costs: [
                (0.0, 1_000.0),
                (20_000.0, 150.0),
                (60_000.0, 160.0),
                (90_000.0, 170.0),
                (120_000.0, 180.0),
            ],
        };
        let mut run = drive(PRIOR, quick, 3_000, |_| 400, 0.0, false);
        let switches = run.tuner.take_switches();
        assert_eq!(switches.len(), 1, "{switches:?}");
        assert_eq!(switches[0].from, Arm::Inline);
        assert_eq!(switches[0].to, Arm::Pooled(0));
        assert!(switches[0].to_ns < 0.9 * switches[0].from_ns);
        assert!(run.tuner.take_switches().is_empty(), "drained");
    }

    #[test]
    fn reset_forgets_the_measurements_but_not_the_prior() {
        let mut run = drive(PRIOR, DESKTOP, 2_000, |_| 20_000, 0.0, false);
        assert!(run.tuner.stats().probes > 0);
        run.tuner.reset();
        assert_eq!(run.tuner.stats(), PoolTunerStats::default());
        let choice = run.tuner.choose(size(20_000));
        assert_eq!(
            choice,
            Choice {
                arm: Arm::Pooled(0),
                timed: true
            }
        );
    }
}
