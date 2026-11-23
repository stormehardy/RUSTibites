//! RUSTibites Improved: the force tower.
//!
//! A tower the player places in the world that PULLS the things near it towards itself, or PUSHES them away from itself (a negative strength). It acts on the bibites
//! (the dead ones too: a corpse is a body), the pellets and the eggs inside its **range**, each of them with its own switch in the tower's panel. Like gravity it is an
//! acceleration, not a force: a heavy body and a light one gain the same speed from it, so the tower does not favour the small. Every tick the tower adds
//! `strength * shape(distance) * dt` to the velocity of each body in range, in the direction of the tower (or away from it); the world's own drag and collisions do the
//! rest, so a body settles where the pull and the drag balance (its speed there is about `strength / drag`) and a pile of pellets pulled in jam against each other.
//!
//! The **falloff** is how the strength changes with the distance: [`ForceFalloff::Constant`] is the same all the way to the edge of the range, [`ForceFalloff::Fading`]
//! (the default) is full at the tower and fades in a straight line to nothing at the edge, so there is no step in the pull where the range ends. Inside [`FORCE_CORE`]
//! units of the tower nothing acts (the direction to a point is not defined there), so a body that has arrived is left to rest.
//!
//! The game has no such tower, so it is never written into a save (the game-format save writer leaves it out; the viewer keeps these towers in a file beside the
//! save), and it acts only while [`crate::improved::Feature::ForceTower`] is on. It draws nothing from the world's random numbers; with none in a world nothing of this
//! runs.

use crate::math::Vec2;
use crate::tools::TowerBase;

/// The shortest range (u).
pub const FORCE_RANGE_MIN: f32 = 10.0;
/// The longest range (u).
pub const FORCE_RANGE_MAX: f32 = 5000.0;
/// The range of a new tower (u).
pub const FORCE_RANGE_DEFAULT: f32 = 300.0;
/// The strongest push (the most negative strength, u/s per second).
pub const FORCE_STRENGTH_MIN: f32 = -500.0;
/// The strongest pull (u/s per second).
pub const FORCE_STRENGTH_MAX: f32 = 500.0;
/// The strength of a new tower (u/s per second; positive pulls).
pub const FORCE_STRENGTH_DEFAULT: f32 = 100.0;
/// Inside this distance of the tower (u) nothing acts: there is no direction to the centre of a body that is on it.
pub const FORCE_CORE: f32 = 2.0;

/// How the strength of a force tower changes with the distance from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForceFalloff {
    /// The same strength at every distance inside the range.
    Constant,
    /// Full strength at the tower, fading in a straight line to nothing at the edge of the range.
    #[default]
    Fading,
}

impl ForceFalloff {
    /// The word a sidecar file and a command line use.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Fading => "fading",
        }
    }

    /// The falloff a word names (any case); `None` for anything else.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let key = key.trim();
        [Self::Constant, Self::Fading].into_iter().find(|falloff| falloff.key().eq_ignore_ascii_case(key))
    }
}

/// A force tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForceTower {
    /// Common tower state (the position, the serialized id, the zone it follows).
    pub base: TowerBase,
    /// How far from the tower it acts, in u ([`FORCE_RANGE_MIN`] to [`FORCE_RANGE_MAX`]).
    pub range: f32,
    /// The acceleration it gives, in u/s per second, towards the tower when positive and away from it when negative ([`FORCE_STRENGTH_MIN`] to [`FORCE_STRENGTH_MAX`]).
    pub strength: f32,
    /// How the strength changes with the distance.
    pub falloff: ForceFalloff,
    /// Whether it acts on bibites (the dead ones too).
    pub bibites: bool,
    /// Whether it acts on pellets (plant and meat; the ones a bibite holds in its mouth are left alone).
    pub pellets: bool,
    /// Whether it acts on eggs.
    pub eggs: bool,
    /// How many bodies it moved in the last tick (a runtime figure for the panel).
    pub affected: u32,
}

impl ForceTower {
    /// A tower at `pos` with the default range and strength (a pull that fades with the distance), acting on bibites, pellets and eggs.
    #[must_use]
    pub fn new(save_id: u64, pos: Vec2) -> Self {
        Self {
            base: TowerBase::new(save_id, pos),
            range: FORCE_RANGE_DEFAULT,
            strength: FORCE_STRENGTH_DEFAULT,
            falloff: ForceFalloff::Fading,
            bibites: true,
            pellets: true,
            eggs: true,
            affected: 0,
        }
    }

    /// Sets the range (kept inside its own range; a value that is not a number is ignored).
    pub fn set_range(&mut self, range: f32) {
        if range.is_finite() {
            self.range = range.clamp(FORCE_RANGE_MIN, FORCE_RANGE_MAX);
        }
    }

    /// Sets the strength (kept inside its own range; a value that is not a number is ignored).
    pub fn set_strength(&mut self, strength: f32) {
        if strength.is_finite() {
            self.strength = strength.clamp(FORCE_STRENGTH_MIN, FORCE_STRENGTH_MAX);
        }
    }

    /// The acceleration (u/s per second) a body `distance` units from the tower gets, towards the tower when positive: 0 outside the range and inside
    /// [`FORCE_CORE`], the strength times the falloff's shape between.
    #[must_use]
    pub fn acceleration_at(&self, distance: f32) -> f32 {
        if !distance.is_finite() || distance >= self.range || distance < FORCE_CORE {
            return 0.0;
        }
        let shape = match self.falloff {
            ForceFalloff::Constant => 1.0,
            ForceFalloff::Fading => 1.0 - distance / self.range,
        };
        self.strength * shape
    }

    /// The change of velocity a body at `at` gets from `dt` seconds of the tower: along the line to the tower (away from it for a push), nothing out of range.
    #[must_use]
    pub fn kick(&self, at: Vec2, dt: f32) -> Vec2 {
        let to_tower = self.base.pos - at;
        let distance = to_tower.length();
        if !(dt > 0.0 && dt.is_finite() && distance.is_finite()) {
            return Vec2::ZERO;
        }
        let acceleration = self.acceleration_at(distance);
        if acceleration == 0.0 {
            return Vec2::ZERO;
        }
        to_tower * (acceleration * dt / distance)
    }

    /// Whether the tower acts on anything at all (a strength of 0, or every switch off, leaves the world as it is).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.strength != 0.0 && (self.bibites || self.pellets || self.eggs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tower() -> ForceTower {
        ForceTower::new(7, Vec2::new(100.0, -50.0))
    }

    #[test]
    fn a_new_tower_pulls_with_a_fading_force_on_bibites_pellets_and_eggs() {
        let tower = tower();
        assert_eq!((tower.range, tower.strength), (FORCE_RANGE_DEFAULT, FORCE_STRENGTH_DEFAULT));
        assert!(tower.strength > 0.0, "a pull");
        assert_eq!(tower.falloff, ForceFalloff::Fading);
        assert!(tower.bibites && tower.pellets && tower.eggs && tower.is_active());
        assert_eq!((tower.base.save_id, tower.base.pos, tower.affected), (7, Vec2::new(100.0, -50.0), 0));
    }

    #[test]
    fn the_setters_keep_every_value_inside_its_range_and_ignore_what_is_not_a_number() {
        let mut tower = tower();
        tower.set_range(1.0e9);
        assert_eq!(tower.range, FORCE_RANGE_MAX);
        tower.set_range(-3.0);
        assert_eq!(tower.range, FORCE_RANGE_MIN);
        tower.set_range(f32::NAN);
        assert_eq!(tower.range, FORCE_RANGE_MIN, "not a number: unchanged");
        tower.set_strength(1.0e9);
        assert_eq!(tower.strength, FORCE_STRENGTH_MAX);
        tower.set_strength(-1.0e9);
        assert_eq!(tower.strength, FORCE_STRENGTH_MIN, "a push can be as strong as a pull");
        tower.set_strength(f32::INFINITY);
        assert_eq!(tower.strength, FORCE_STRENGTH_MIN);
        tower.set_strength(0.0);
        assert!(!tower.is_active(), "a strength of 0 does nothing");
        tower.set_strength(10.0);
        (tower.bibites, tower.pellets, tower.eggs) = (false, false, false);
        assert!(!tower.is_active(), "with every switch off it does nothing");
    }

    #[test]
    fn a_constant_force_is_the_same_all_the_way_to_the_edge_and_a_fading_one_goes_down_to_nothing_there() {
        let mut tower = tower();
        tower.set_range(400.0);
        tower.set_strength(100.0);
        tower.falloff = ForceFalloff::Constant;
        for distance in [FORCE_CORE, 10.0, 200.0, 399.0] {
            assert_eq!(tower.acceleration_at(distance), 100.0, "{distance}");
        }
        tower.falloff = ForceFalloff::Fading;
        assert!((tower.acceleration_at(FORCE_CORE) - 100.0 * (1.0 - FORCE_CORE / 400.0)).abs() < 1e-4);
        assert!((tower.acceleration_at(200.0) - 50.0).abs() < 1e-4, "half way, half the strength");
        assert!((tower.acceleration_at(399.0) - 0.25).abs() < 1e-3, "almost nothing at the edge");
        // Outside the range and on the tower itself: nothing, whatever the falloff; and nothing for a distance that is not a number.
        for falloff in [ForceFalloff::Constant, ForceFalloff::Fading] {
            tower.falloff = falloff;
            for distance in [400.0, 401.0, 1.0e6, 0.0, FORCE_CORE - 0.01, f32::NAN, f32::INFINITY] {
                assert_eq!(tower.acceleration_at(distance), 0.0, "{falloff:?} at {distance}");
            }
        }
    }

    #[test]
    fn a_pull_points_at_the_tower_a_push_away_from_it_and_the_size_is_the_accelerations_times_dt() {
        let mut tower = tower();
        tower.set_strength(60.0);
        tower.falloff = ForceFalloff::Constant;
        let dt = 1.0 / 30.0;
        // East of the tower: pulled west.
        let kick = tower.kick(Vec2::new(200.0, -50.0), dt);
        assert!((kick.x + 60.0 * dt).abs() < 1e-5 && kick.y.abs() < 1e-6, "{kick:?}");
        // North-east: along the diagonal, towards the tower.
        let kick = tower.kick(Vec2::new(160.0, 10.0), dt);
        assert!(kick.x < 0.0 && kick.y < 0.0 && (kick.x - kick.y).abs() < 1e-5 && (kick.length() - 60.0 * dt).abs() < 1e-5, "{kick:?}");
        // A push: the same size, the other way.
        tower.set_strength(-60.0);
        let push = tower.kick(Vec2::new(200.0, -50.0), dt);
        assert!((push.x - 60.0 * dt).abs() < 1e-5 && push.y.abs() < 1e-6, "{push:?}");
        // Out of range, on the tower, and a tick that is not a tick: nothing.
        assert_eq!(tower.kick(Vec2::new(1000.0, -50.0), dt), Vec2::ZERO);
        assert_eq!(tower.kick(tower.base.pos, dt), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(200.0, -50.0), 0.0), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(200.0, -50.0), f32::NAN), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(f32::NAN, 0.0), dt), Vec2::ZERO);
    }

    #[test]
    fn the_falloff_words_round_trip() {
        for falloff in [ForceFalloff::Constant, ForceFalloff::Fading] {
            assert_eq!(ForceFalloff::from_key(falloff.key()), Some(falloff));
            assert_eq!(ForceFalloff::from_key(&format!("  {} ", falloff.key().to_uppercase())), Some(falloff));
        }
        assert_eq!(ForceFalloff::from_key("linear"), None);
    }
}
