//! RUSTibites Improved: the hydrothermal vent tower.
//!
//! A plant pellet spawner that the player places in the world. Every shot it throws one plant pellet out of its mouth in a RANDOM direction at the set velocity; the
//! pellet's size (its energy), the shoot rate, a cap on the energy of the pellets it has alive and whether the oldest of them are recycled at the cap are set in its
//! panel. Each shot varies by up to a quarter in speed and in size (the pellet placer's variation), so the pellets do not all stop on one ring.
//!
//! The game's drag stops a pellet within a few tens of units (and nothing moves faster than 400 u/s), so a vent has a **range**: how far a pellet slides in open ground.
//! The vent lowers the drag of ITS pellet while it flies, to `speed / range` a second (the distance an exponentially slowing body covers is its speed over its drag),
//! and gives it the game's drag back when it has slowed to [`VENT_GLIDE_END_SPEED`]; every other pellet, and the vent's own once it has stopped, keeps the game's drag.
//! A range shorter than the game's own reach changes nothing.
//!
//! The cap is a zone's (`Zone.RecyclePellets`, "Biomass" and "Renew Biomass" of a zone): the vent counts the energy of the pellets it shot that are still in the
//! world. Over the cap with Recycle ON it takes the energy of its OLDEST pellets until it is back at the cap (they shrink, and vanish when nothing is left) and goes on
//! shooting; with Recycle OFF it stops shooting at the cap and starts again when bibites have eaten enough.
//!
//! The game has no such tower, so it is never written into a save (the game-format save writer leaves it out; the viewer keeps these towers in a file beside the
//! save), and it acts only while [`crate::improved::Feature::HydrothermalVent`] is on. The pellets it shot ARE ordinary pellets and stay in a save, but a saved world
//! has no memory of which vent shot them: after a load the vent counts and recycles only what it shoots from then on. With none in a world nothing of this runs.

use crate::math::Vec2;
use crate::tools::TowerBase;

/// The slowest launch speed (u/s).
pub const VENT_VELOCITY_MIN: f32 = 10.0;
/// The fastest launch speed (u/s): the world's top speed for anything that moves (`PhysicsConfig::max_translation_speed`), so a faster shot would only be cut back.
pub const VENT_VELOCITY_MAX: f32 = 400.0;
/// The shortest range (u): about what the game's own drag gives a pellet of the default size (a shorter one changes nothing).
pub const VENT_RANGE_MIN: f32 = 10.0;
/// The longest range (u).
pub const VENT_RANGE_MAX: f32 = 3000.0;
/// The range of a new vent (u).
pub const VENT_RANGE_DEFAULT: f32 = 200.0;
/// A flying pellet that has slowed below this (u/s) has the game's drag again.
pub const VENT_GLIDE_END_SPEED: f32 = 10.0;
/// The lowest factor on the game's drag (a very long range at a low speed).
pub const VENT_DRAG_FACTOR_MIN: f32 = 0.000_5;
/// The launch speed of a new vent (u/s).
pub const VENT_VELOCITY_DEFAULT: f32 = 250.0;
/// The slowest shoot rate (pellets per second).
pub const VENT_RATE_MIN: f32 = 0.1;
/// The fastest shoot rate (pellets per second).
pub const VENT_RATE_MAX: f32 = 50.0;
/// The shoot rate of a new vent (pellets per second).
pub const VENT_RATE_DEFAULT: f32 = 2.0;
/// The smallest pellet (E): the game's `pelletEnergy` range.
pub const VENT_PELLET_MIN: f32 = 10.0;
/// The biggest pellet (E): the game's `pelletEnergy` range.
pub const VENT_PELLET_MAX: f32 = 5000.0;
/// The pellet size of a vent when the world's default pellet size cannot be read (E): the game's default.
pub const VENT_PELLET_DEFAULT: f32 = 200.0;
/// The smallest cap (E of pellets alive).
pub const VENT_CAP_MIN: f32 = 100.0;
/// The biggest cap (E of pellets alive).
pub const VENT_CAP_MAX: f32 = 1_000_000.0;
/// The cap of a new vent (E): a hundred pellets of the default size.
pub const VENT_CAP_DEFAULT: f32 = 20_000.0;
/// How far from the vent's centre a pellet comes out (u).
pub const VENT_MOUTH: f32 = 15.0;
/// The most pellets one tick may shoot (a vent at the top rate with a long tick does not flood the world).
pub const VENT_MAX_SHOTS_PER_TICK: u32 = 64;
/// How much a shot varies in speed and in size (a fraction either way).
pub const VENT_VARIATION: f32 = 0.25;

/// A hydrothermal vent tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HydrothermalVent {
    /// Common tower state (the position, the serialized id, the zone it follows).
    pub base: TowerBase,
    /// Launch speed in u/s ([`VENT_VELOCITY_MIN`] to [`VENT_VELOCITY_MAX`]).
    pub velocity: f32,
    /// How far a pellet slides in open ground, in u ([`VENT_RANGE_MIN`] to [`VENT_RANGE_MAX`]).
    pub range: f32,
    /// Energy of a pellet in E ([`VENT_PELLET_MIN`] to [`VENT_PELLET_MAX`]).
    pub pellet_energy: f32,
    /// Pellets per second ([`VENT_RATE_MIN`] to [`VENT_RATE_MAX`]).
    pub rate: f32,
    /// Over the cap: take the energy of the oldest pellets and go on shooting (`true`), or stop shooting until some is eaten (`false`).
    pub recycle: bool,
    /// The most energy of its pellets the vent has alive (E, [`VENT_CAP_MIN`] to [`VENT_CAP_MAX`]).
    pub cap: f32,
    /// Shots owed: grows by `rate` a second, one pellet is shot for every whole one.
    pub progress: f32,
    /// Pellets shot so far (a runtime count).
    pub shot: u32,
    /// Its pellets alive as of the last tick (a runtime figure for the panel).
    pub alive: u32,
    /// Their energy as of the last tick (a runtime figure for the panel and what the cap is measured against).
    pub biomass: f32,
}

/// One shot: where the pellet comes out relative to the vent, how fast it goes and how much energy it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    /// Offset from the vent's centre: [`VENT_MOUTH`] in the shot's direction.
    pub offset: Vec2,
    /// Launch velocity in u/s.
    pub velocity: Vec2,
    /// Pellet energy in E.
    pub energy: f32,
}

impl HydrothermalVent {
    /// A vent at `pos` with the default velocity, rate and cap and a pellet of `pellet_energy` (the world's default pellet size, clamped), recycling off.
    #[must_use]
    pub fn new(save_id: u64, pos: Vec2, pellet_energy: f32) -> Self {
        let mut vent = Self {
            base: TowerBase::new(save_id, pos),
            velocity: VENT_VELOCITY_DEFAULT,
            range: VENT_RANGE_DEFAULT,
            pellet_energy: VENT_PELLET_DEFAULT,
            rate: VENT_RATE_DEFAULT,
            recycle: false,
            cap: VENT_CAP_DEFAULT,
            progress: 0.0,
            shot: 0,
            alive: 0,
            biomass: 0.0,
        };
        vent.set_pellet_energy(pellet_energy);
        vent
    }

    /// Sets the launch speed (kept inside its range; a value that is not a number is ignored).
    pub fn set_velocity(&mut self, velocity: f32) {
        if velocity.is_finite() {
            self.velocity = velocity.clamp(VENT_VELOCITY_MIN, VENT_VELOCITY_MAX);
        }
    }

    /// Sets the range (kept inside its own range; a value that is not a number is ignored).
    pub fn set_range(&mut self, range: f32) {
        if range.is_finite() {
            self.range = range.clamp(VENT_RANGE_MIN, VENT_RANGE_MAX);
        }
    }

    /// The factor on the game's drag for a pellet shot at `speed` whose own linear drag is `base_drag` (per second): the one that makes it slide about `range`
    /// units. 1 (the game's drag) when the range is no longer than the game's drag gives anyway, or when there is no drag or no speed.
    #[must_use]
    pub fn glide_factor(&self, speed: f32, base_drag: f32) -> f32 {
        if !(speed.is_finite() && speed > 0.0 && base_drag.is_finite() && base_drag > 0.0) {
            return 1.0;
        }
        ((speed / self.range) / base_drag).clamp(VENT_DRAG_FACTOR_MIN, 1.0)
    }

    /// Sets the pellet size (kept inside its range; a value that is not a number is ignored).
    pub fn set_pellet_energy(&mut self, energy: f32) {
        if energy.is_finite() {
            self.pellet_energy = energy.clamp(VENT_PELLET_MIN, VENT_PELLET_MAX);
        }
    }

    /// Sets the shoot rate (kept inside its range; a value that is not a number is ignored).
    pub fn set_rate(&mut self, rate: f32) {
        if rate.is_finite() {
            self.rate = rate.clamp(VENT_RATE_MIN, VENT_RATE_MAX);
        }
    }

    /// Sets the cap (kept inside its range; a value that is not a number is ignored).
    pub fn set_cap(&mut self, cap: f32) {
        if cap.is_finite() {
            self.cap = cap.clamp(VENT_CAP_MIN, VENT_CAP_MAX);
        }
    }

    /// Whether the vent may shoot with `biomass` of its pellets alive: always with Recycle on (the oldest are taken back), only below the cap with it off.
    #[must_use]
    pub fn may_shoot(&self, biomass: f32) -> bool {
        self.recycle || biomass < self.cap
    }

    /// Advances the shoot timer by `dt` seconds and returns how many pellets are due (at most [`VENT_MAX_SHOTS_PER_TICK`]). While `blocked` (at the cap, no
    /// recycling) nothing is due and at most one shot is kept waiting, so the vent does not start with a burst when bibites free some room.
    pub fn due_shots(&mut self, dt: f32, blocked: bool) -> u32 {
        if dt > 0.0 && dt.is_finite() {
            self.progress += dt * self.rate;
        }
        if blocked {
            self.progress = self.progress.min(1.0);
            return 0;
        }
        let due = (self.progress.floor().max(0.0) as u32).min(VENT_MAX_SHOTS_PER_TICK);
        self.progress = (self.progress - due as f32).clamp(0.0, 1.0 + VENT_MAX_SHOTS_PER_TICK as f32);
        due
    }

    /// The shot made of three random numbers in `[0, 1)`: the direction (a full circle), the speed and the size (each `velocity` / `pellet_energy` varied by
    /// [`VENT_VARIATION`] either way).
    #[must_use]
    pub fn shot(&self, direction: f32, speed: f32, size: f32) -> Shot {
        let vary = |unit: f32| 1.0 - VENT_VARIATION + 2.0 * VENT_VARIATION * unit.clamp(0.0, 1.0);
        let dir = Vec2::X.rotate(std::f32::consts::TAU * direction.clamp(0.0, 1.0));
        Shot {
            offset: dir * VENT_MOUTH,
            velocity: dir * (self.velocity * vary(speed)),
            energy: (self.pellet_energy * vary(size)).max(1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vent() -> HydrothermalVent {
        HydrothermalVent::new(7, Vec2::new(3.0, -4.0), 200.0)
    }

    #[test]
    fn a_new_vent_has_the_defaults_and_takes_the_worlds_pellet_size_inside_the_games_range() {
        let vent = vent();
        assert_eq!((vent.velocity, vent.rate, vent.cap, vent.pellet_energy, vent.recycle), (VENT_VELOCITY_DEFAULT, VENT_RATE_DEFAULT, VENT_CAP_DEFAULT, 200.0, false));
        assert_eq!(vent.range, VENT_RANGE_DEFAULT);
        assert_eq!((vent.progress, vent.shot, vent.alive, vent.biomass), (0.0, 0, 0, 0.0));
        assert_eq!(vent.base.pos, Vec2::new(3.0, -4.0));
        assert_eq!(HydrothermalVent::new(1, Vec2::ZERO, 1.0).pellet_energy, VENT_PELLET_MIN);
        assert_eq!(HydrothermalVent::new(1, Vec2::ZERO, 1.0e9).pellet_energy, VENT_PELLET_MAX);
        assert_eq!(HydrothermalVent::new(1, Vec2::ZERO, f32::NAN).pellet_energy, VENT_PELLET_DEFAULT, "a size that is not a number leaves the default");
    }

    #[test]
    fn the_setters_keep_every_value_inside_its_range_and_ignore_what_is_not_a_number() {
        let mut vent = vent();
        vent.set_velocity(1.0e9);
        vent.set_rate(0.0);
        vent.set_cap(-5.0);
        vent.set_pellet_energy(0.0);
        vent.set_range(1.0e9);
        assert_eq!((vent.velocity, vent.rate, vent.cap, vent.pellet_energy, vent.range), (VENT_VELOCITY_MAX, VENT_RATE_MIN, VENT_CAP_MIN, VENT_PELLET_MIN, VENT_RANGE_MAX));
        vent.set_range(0.0);
        assert_eq!(vent.range, VENT_RANGE_MIN);
        vent.set_range(f32::NAN);
        assert_eq!(vent.range, VENT_RANGE_MIN);
        vent.set_velocity(f32::NAN);
        vent.set_rate(f32::INFINITY);
        vent.set_cap(f32::NAN);
        vent.set_pellet_energy(f32::NAN);
        assert_eq!((vent.velocity, vent.rate, vent.cap, vent.pellet_energy), (VENT_VELOCITY_MAX, VENT_RATE_MIN, VENT_CAP_MIN, VENT_PELLET_MIN));
    }

    #[test]
    fn the_timer_shoots_one_pellet_for_every_whole_shot_owed_and_keeps_the_rest() {
        let mut vent = vent();
        vent.set_rate(2.0);
        assert_eq!(vent.due_shots(0.1, false), 0, "0.2 of a shot");
        assert_eq!(vent.due_shots(0.1, false), 0, "0.4");
        assert_eq!(vent.due_shots(0.1, false), 0, "0.6");
        assert_eq!(vent.due_shots(0.1, false), 0, "0.8");
        assert_eq!(vent.due_shots(0.1, false), 1, "one, the rest is kept");
        assert!(vent.progress.abs() < 1e-4, "{}", vent.progress);
        vent.set_rate(10.0);
        assert_eq!(vent.due_shots(0.35, false), 3);
        assert!((vent.progress - 0.5).abs() < 1e-4, "{}", vent.progress);
        // A tick that is not a time, or a negative one, owes nothing.
        let owed = vent.progress;
        assert_eq!(vent.due_shots(-1.0, false) + vent.due_shots(f32::NAN, false), 0);
        assert_eq!(vent.progress, owed);
    }

    #[test]
    fn a_long_tick_at_the_top_rate_shoots_at_most_the_limit_and_does_not_bank_the_rest() {
        let mut vent = vent();
        vent.set_rate(VENT_RATE_MAX);
        assert_eq!(vent.due_shots(60.0, false), VENT_MAX_SHOTS_PER_TICK);
        assert!(vent.progress <= 1.0 + VENT_MAX_SHOTS_PER_TICK as f32);
    }

    #[test]
    fn at_the_cap_without_recycling_nothing_is_due_and_at_most_one_shot_waits() {
        let mut vent = vent();
        vent.set_rate(10.0);
        assert!(!vent.may_shoot(vent.cap) && vent.may_shoot(vent.cap - 1.0));
        for _ in 0..50 {
            assert_eq!(vent.due_shots(1.0, true), 0);
        }
        assert!(vent.progress <= 1.0, "{}", vent.progress);
        assert_eq!(vent.due_shots(0.0, false), 1, "the one that waited, no burst");
        vent.recycle = true;
        assert!(vent.may_shoot(vent.cap * 10.0), "recycling shoots at any biomass");
    }

    #[test]
    fn the_glide_factor_gives_the_pellet_the_drag_that_slides_it_the_range_and_never_more_than_the_games() {
        let mut vent = vent();
        vent.set_range(200.0);
        // A pellet shot at 250 u/s whose game's drag is 12 a second: it needs 250 / 200 = 1.25 a second, a tenth of that (the exact figure).
        let factor = vent.glide_factor(250.0, 12.0);
        assert!((factor - 1.25 / 12.0).abs() < 1e-6, "{factor}");
        // The distance an exponentially slowing body covers is its speed over its drag: 250 / (12 * factor) is the range.
        assert!((250.0 / (12.0 * factor) - 200.0).abs() < 1e-3);
        // A shorter range than the game's own, no drag, no speed, numbers that are not numbers: the game's drag (1).
        vent.set_range(VENT_RANGE_MIN);
        assert_eq!(vent.glide_factor(250.0, 12.0), 1.0, "the game's own reach is 250 / 12 = 21 u, more than 10");
        vent.set_range(500.0);
        for (speed, drag) in [(250.0, 0.0), (0.0, 12.0), (-5.0, 12.0), (f32::NAN, 12.0), (250.0, f32::NAN), (250.0, -1.0)] {
            assert_eq!(vent.glide_factor(speed, drag), 1.0, "{speed} {drag}");
        }
        // A long range at a low speed does not go below the lowest factor.
        vent.set_range(VENT_RANGE_MAX);
        assert_eq!(vent.glide_factor(VENT_VELOCITY_MIN, 12.0), (VENT_VELOCITY_MIN / VENT_RANGE_MAX / 12.0).max(VENT_DRAG_FACTOR_MIN));
        assert!(vent.glide_factor(VENT_VELOCITY_MIN, 12.0) >= VENT_DRAG_FACTOR_MIN);
    }

    #[test]
    fn a_shot_goes_out_of_the_mouth_in_its_direction_at_the_set_speed_and_size_varied_by_a_quarter() {
        let mut vent = vent();
        vent.set_velocity(100.0);
        vent.set_pellet_energy(400.0);
        // The middle of the variation is the set value; the direction is a full circle.
        let middle = vent.shot(0.25, 0.5, 0.5);
        assert!((middle.velocity.length() - 100.0).abs() < 1e-3 && (middle.energy - 400.0).abs() < 1e-3);
        assert!(middle.offset.x.abs() < 1e-3 && (middle.offset.y - VENT_MOUTH).abs() < 1e-3, "a quarter turn goes up: {:?}", middle.offset);
        assert!(middle.velocity.x.abs() < 1e-3 && middle.velocity.y > 0.0);
        let (slow, small) = (vent.shot(0.0, 0.0, 0.0), vent.shot(0.0, 0.0, 0.0));
        assert!((slow.velocity.length() - 75.0).abs() < 1e-3 && (small.energy - 300.0).abs() < 1e-3);
        let (fast, big) = (vent.shot(0.5, 1.0, 1.0), vent.shot(0.5, 1.0, 1.0));
        assert!((fast.velocity.length() - 125.0).abs() < 1e-3 && (big.energy - 500.0).abs() < 1e-3);
        assert!(fast.velocity.x < 0.0, "half a turn goes left");
        // Every direction comes out of the same mouth.
        for step in 0..16 {
            let shot = vent.shot(step as f32 / 16.0, 0.3, 0.7);
            assert!((shot.offset.length() - VENT_MOUTH).abs() < 1e-3);
            assert!(shot.velocity.normalized().dot(shot.offset / VENT_MOUTH) > 0.999, "the pellet leaves along its offset");
        }
    }
}
