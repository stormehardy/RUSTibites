//! Fixed-timestep simulation clock.
//!
//! The original game advances its simulation in fixed physics steps of
//! `1 / simTPS` seconds (scenario-independent default 30 TPS, though a
//! scenario's settings may override it — e.g. 15 or 40). Wall-clock time,
//! frame rate and rendering speed must never influence simulation results, so
//! the headless engine is driven purely by [`FixedClock::step`].

/// Default simulation ticks per second, matching
/// `ScenarioIndependentSettings.simTPS.DefaultValue = 30`.
pub const DEFAULT_SIM_TPS: u32 = 30;

/// Default brain ticks: the brain updates every 2 simulation ticks
/// (`ScenarioIndependentSettings.brainTPS.DefaultValue = 2`), i.e. 15 Hz at
/// 30 TPS.
pub const DEFAULT_BRAIN_TICKS: u32 = 2;

/// The period passed to stateful brain neurons, in seconds.
///
/// With the defaults this is `2 / 30 = 1/15 s`. Note the original's formula is
/// `brainPeriod = brainTPS / simTPS` (a brain update happens every `brainTPS`
/// simulation ticks).
#[inline]
pub fn brain_period_secs(sim_tps: u32, brain_ticks: u32) -> f32 {
    brain_ticks as f32 / sim_tps as f32
}

/// Tracks tick count and simulation time for a fixed-timestep loop.
///
/// The rate may change while the simulation runs ([`FixedClock::set_tps`], the game's `TimeController.UpdateFixedDeltaTime`): the ticks
/// already made keep the length they had, so [`FixedClock::time_secs`] stays continuous.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock {
    tps: u32,
    dt: f32,
    tick: u64,
    /// The simulation time at tick `base_tick`: what the ticks made before the last rate change add up to (0 until there is one).
    base_time: f64,
    base_tick: u64,
}

impl FixedClock {
    /// Creates a clock running at `tps` ticks per second.
    ///
    /// # Panics
    ///
    /// Panics if `tps == 0`.
    pub fn new(tps: u32) -> Self {
        assert!(tps > 0, "simulation TPS must be positive");
        Self {
            tps,
            dt: 1.0 / tps as f32,
            tick: 0,
            base_time: 0.0,
            base_tick: 0,
        }
    }

    /// Changes the rate to `tps` ticks per second from the next tick on. The tick count goes on, the time made so far is kept, and the
    /// next ticks are `1 / tps` seconds long. Does nothing when the rate is the one it already is.
    ///
    /// # Panics
    ///
    /// Panics if `tps == 0`.
    pub fn set_tps(&mut self, tps: u32) {
        assert!(tps > 0, "simulation TPS must be positive");
        if tps == self.tps {
            return;
        }
        self.base_time = self.time_secs();
        self.base_tick = self.tick;
        self.tps = tps;
        self.dt = 1.0 / tps as f32;
    }

    /// Creates a clock at the game's default rate (30 TPS).
    pub fn default_rate() -> Self {
        Self::new(DEFAULT_SIM_TPS)
    }

    /// Ticks per second.
    #[inline]
    pub fn tps(&self) -> u32 {
        self.tps
    }

    /// Length of one tick in seconds.
    #[inline]
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Number of completed ticks.
    #[inline]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Simulation time in seconds: the time made up to the last rate change plus the ticks since, each `dt` long.
    #[inline]
    pub fn time_secs(&self) -> f64 {
        self.base_time + (self.tick - self.base_tick) as f64 * self.dt as f64
    }

    /// Simulation time in seconds as `f32` (may lose precision after long
    /// runs; prefer [`FixedClock::time_secs`] for reporting).
    #[inline]
    pub fn time_secs_f32(&self) -> f32 {
        self.time_secs() as f32
    }

    /// Advances by one tick.
    #[inline]
    pub fn step(&mut self) {
        self.tick += 1;
    }

    /// Advances by `ticks` ticks.
    #[inline]
    pub fn advance(&mut self, ticks: u64) {
        self.tick += ticks;
    }

    /// Resets the tick counter (and the time) to zero; the rate stays.
    #[inline]
    pub fn reset(&mut self) {
        self.tick = 0;
        self.base_tick = 0;
        self.base_time = 0.0;
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::default_rate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rate_is_30_tps() {
        let clock = FixedClock::default();
        assert_eq!(clock.tps(), 30);
        assert!((clock.dt() - 1.0 / 30.0).abs() < 1e-9);
        assert!((brain_period_secs(30, 2) - 1.0 / 15.0).abs() < 1e-9);
    }

    #[test]
    fn stepping_accumulates_time() {
        let mut clock = FixedClock::new(30);
        clock.advance(30);
        assert_eq!(clock.tick(), 30);
        assert!((clock.time_secs() - 1.0).abs() < 1e-6);
        clock.step();
        assert_eq!(clock.tick(), 31);
    }

    #[test]
    #[should_panic]
    fn zero_tps_panics() {
        let _ = FixedClock::new(0);
    }

    #[test]
    #[should_panic]
    fn changing_to_zero_tps_panics() {
        FixedClock::new(30).set_tps(0);
    }

    #[test]
    fn a_rate_change_keeps_the_ticks_and_the_time_made_so_far() {
        let mut clock = FixedClock::new(30);
        clock.advance(30);
        let made = clock.time_secs();
        assert!((made - 1.0).abs() < 1e-6, "{made}");
        // 60 ticks a second from here: the tick count goes on, the second made at 30 is still a second (exactly what it was), and the next
        // ticks are 1/60 s long.
        clock.set_tps(60);
        assert_eq!((clock.tps(), clock.tick()), (60, 30));
        assert_eq!(clock.dt(), 1.0 / 60.0);
        assert_eq!(clock.time_secs(), made);
        clock.advance(60);
        assert_eq!(clock.tick(), 90);
        assert!((clock.time_secs() - 2.0).abs() < 1e-6, "{}", clock.time_secs());
        // And back: 15 ticks a second, 15 ticks make another second.
        clock.set_tps(15);
        clock.advance(15);
        assert!((clock.time_secs() - 3.0).abs() < 1e-6, "{}", clock.time_secs());
        assert!((f64::from(clock.time_secs_f32()) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn the_same_rate_changes_nothing_and_reset_starts_over() {
        let mut clock = FixedClock::new(30);
        clock.advance(7);
        let before = clock.time_secs();
        clock.set_tps(30);
        assert_eq!((clock.tick(), clock.time_secs()), (7, before));
        clock.set_tps(40);
        clock.advance(4);
        clock.reset();
        assert_eq!((clock.tick(), clock.time_secs(), clock.tps()), (0, 0.0, 40));
        clock.step();
        assert!((clock.time_secs() - 1.0 / 40.0).abs() < 1e-9);
    }
}
