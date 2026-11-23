//! RUSTibites Improved: the whirlpool tower.
//!
//! A tower the player places in the world that turns the things around it in a circle: it works like a [stream](crate::stream), but the direction of the push is not one
//! line, it is the tangent of the circle round the tower, **clockwise** or **counterclockwise** (as the world is seen on the screen, with y up). It acts on the bibites
//! (the dead ones too: a corpse is a body), the pellets and the eggs inside its **range**, each kind with its own switch in the tower's panel. Like the force tower (and the
//! stream) it is an acceleration, not a force: a heavy body and a light one gain the same speed from it. Every tick the tower adds `speed * shape(distance) * dt` to the
//! velocity of each body in range, along the circle through the body, and the world's own drag and collisions do the rest, so a body settles at a speed of about `speed / drag`
//! round the tower (a pellet, whose drag is high, goes round slowly; a bibite, whose drag is low, goes faster).
//!
//! A push along the circle alone would fling a fast body off it (a body that moves has momentum and goes straight on): a bibite given a few seconds of it would spiral out of
//! the whirlpool. So the tower also **holds** the bodies on their circles: it adds the inward acceleration a body needs to turn with the speed it has along the circle (`v_t² / r`, the
//! centripetal acceleration, weighted by the same falloff as the push and never more than takes the whole of that speed in one tick). A body that goes round stays at about its distance;
//! one that swims outwards on its own is not stopped and leaves when it gets out of range.
//!
//! The **falloff** is the force tower's: [`crate::force_tower::ForceFalloff::Constant`] is the same all the way to the edge of the range, the default
//! [`crate::force_tower::ForceFalloff::Fading`] is full at the tower and fades in a straight line to nothing at the edge. Inside [`WHIRL_CORE`] units of the tower nothing
//! acts (a point on the tower is on no circle).
//!
//! The game has no such tower, so it is never written into a save (the game-format save writer leaves it out; the viewer keeps these towers in a file beside the save), and it
//! acts only while [`crate::improved::Feature::Whirlpool`] is on. It draws nothing from the world's random numbers; with none in a world nothing of this runs.

use crate::force_tower::ForceFalloff;
use crate::math::Vec2;
use crate::tools::TowerBase;

/// The shortest range (u).
pub const WHIRL_RANGE_MIN: f32 = 10.0;
/// The longest range (u).
pub const WHIRL_RANGE_MAX: f32 = 5000.0;
/// The range of a new tower (u).
pub const WHIRL_RANGE_DEFAULT: f32 = 300.0;
/// The slowest whirl: none at all (u/s per second).
pub const WHIRL_SPEED_MIN: f32 = 0.0;
/// The fastest whirl (u/s per second).
pub const WHIRL_SPEED_MAX: f32 = 500.0;
/// The speed of a new tower (u/s per second).
pub const WHIRL_SPEED_DEFAULT: f32 = 100.0;
/// Inside this distance of the tower (u) nothing acts: a body on the tower is on no circle round it.
pub const WHIRL_CORE: f32 = 2.0;

/// Which way a whirlpool turns, as the world is seen on the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spin {
    /// The way the hands of a clock go.
    Clockwise,
    /// The other way.
    #[default]
    Counterclockwise,
}

impl Spin {
    /// The word a sidecar file and a command line use.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Clockwise => "clockwise",
            Self::Counterclockwise => "counterclockwise",
        }
    }

    /// The spin a word names (any case; `cw` and `ccw` too); `None` for anything else.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let key = key.trim();
        if key.eq_ignore_ascii_case("cw") {
            return Some(Self::Clockwise);
        }
        if key.eq_ignore_ascii_case("ccw") {
            return Some(Self::Counterclockwise);
        }
        [Self::Clockwise, Self::Counterclockwise].into_iter().find(|spin| spin.key().eq_ignore_ascii_case(key))
    }

    /// The other way.
    #[must_use]
    pub const fn reversed(self) -> Self {
        match self {
            Self::Clockwise => Self::Counterclockwise,
            Self::Counterclockwise => Self::Clockwise,
        }
    }

    /// `1` for counterclockwise, `-1` for clockwise: the turn of the unit tangent from the unit vector away from the tower.
    #[must_use]
    pub const fn sign(self) -> f32 {
        match self {
            Self::Clockwise => -1.0,
            Self::Counterclockwise => 1.0,
        }
    }
}

/// A whirlpool tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WhirlpoolTower {
    /// Common tower state (the position, the serialized id, the zone it follows).
    pub base: TowerBase,
    /// How far from the tower it acts, in u ([`WHIRL_RANGE_MIN`] to [`WHIRL_RANGE_MAX`]).
    pub range: f32,
    /// The acceleration it gives along the circle, in u/s per second ([`WHIRL_SPEED_MIN`] to [`WHIRL_SPEED_MAX`]).
    pub speed: f32,
    /// Which way it turns.
    pub spin: Spin,
    /// How the speed changes with the distance.
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

impl WhirlpoolTower {
    /// A tower at `pos` with the default range and speed (counterclockwise, fading with the distance), acting on bibites, pellets and eggs.
    #[must_use]
    pub fn new(save_id: u64, pos: Vec2) -> Self {
        Self {
            base: TowerBase::new(save_id, pos),
            range: WHIRL_RANGE_DEFAULT,
            speed: WHIRL_SPEED_DEFAULT,
            spin: Spin::Counterclockwise,
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
            self.range = range.clamp(WHIRL_RANGE_MIN, WHIRL_RANGE_MAX);
        }
    }

    /// Sets the speed (kept inside its own range; a value that is not a number is ignored).
    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() {
            self.speed = speed.clamp(WHIRL_SPEED_MIN, WHIRL_SPEED_MAX);
        }
    }

    /// The acceleration (u/s per second) a body `distance` units from the tower gets along its circle: 0 outside the range and inside [`WHIRL_CORE`], the speed times the
    /// falloff's shape between.
    #[must_use]
    pub fn acceleration_at(&self, distance: f32) -> f32 {
        if !distance.is_finite() || distance >= self.range || distance < WHIRL_CORE {
            return 0.0;
        }
        let shape = match self.falloff {
            ForceFalloff::Constant => 1.0,
            ForceFalloff::Fading => 1.0 - distance / self.range,
        };
        self.speed * shape
    }

    /// The change of velocity a body at `at` moving at `velocity` gets from `dt` seconds of the tower: a push along the tangent of the circle round the tower through the
    /// body, the way the tower turns, and the inward acceleration that holds it on that circle (see the module's documentation); nothing out of range.
    #[must_use]
    pub fn kick(&self, at: Vec2, velocity: Vec2, dt: f32) -> Vec2 {
        let away = at - self.base.pos;
        let distance = away.length();
        if !(dt > 0.0 && dt.is_finite() && distance.is_finite()) {
            return Vec2::ZERO;
        }
        let acceleration = self.acceleration_at(distance);
        if acceleration == 0.0 {
            return Vec2::ZERO;
        }
        // `perp` turns a vector a quarter turn counterclockwise: for a body east of the tower that is north, and that is how a counterclockwise whirl moves it.
        let outward = away * (1.0 / distance);
        let tangent = outward.perp() * self.spin.sign();
        let push = tangent * (acceleration * dt);
        // The hold: the centripetal acceleration of the speed the body has along its circle, in the same proportion as the push (the falloff's shape), and not more than turns
        // that whole speed in one tick.
        let along = velocity.dot(tangent);
        let shape = acceleration / self.speed;
        let hold = if along.is_finite() { (along * along / distance * dt * shape).min(along.abs()) } else { 0.0 };
        push - outward * hold
    }

    /// Whether the tower acts on anything at all (a speed of 0, or every switch off, leaves the world as it is).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.speed > 0.0 && (self.bibites || self.pellets || self.eggs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tower() -> WhirlpoolTower {
        WhirlpoolTower::new(9, Vec2::new(100.0, -50.0))
    }

    #[test]
    fn a_new_tower_turns_counterclockwise_with_a_fading_push_on_bibites_pellets_and_eggs() {
        let tower = tower();
        assert_eq!((tower.range, tower.speed), (WHIRL_RANGE_DEFAULT, WHIRL_SPEED_DEFAULT));
        assert_eq!((tower.spin, tower.falloff), (Spin::Counterclockwise, ForceFalloff::Fading));
        assert!(tower.bibites && tower.pellets && tower.eggs && tower.is_active());
        assert_eq!((tower.base.save_id, tower.base.pos, tower.affected), (9, Vec2::new(100.0, -50.0), 0));
    }

    #[test]
    fn the_setters_keep_every_value_inside_its_range_and_ignore_what_is_not_a_number() {
        let mut tower = tower();
        tower.set_range(1.0e9);
        assert_eq!(tower.range, WHIRL_RANGE_MAX);
        tower.set_range(-3.0);
        assert_eq!(tower.range, WHIRL_RANGE_MIN);
        tower.set_range(f32::NAN);
        assert_eq!(tower.range, WHIRL_RANGE_MIN, "not a number: unchanged");
        tower.set_speed(1.0e9);
        assert_eq!(tower.speed, WHIRL_SPEED_MAX);
        tower.set_speed(-5.0);
        assert_eq!(tower.speed, WHIRL_SPEED_MIN, "the way it turns is the spin's, not the sign of the speed");
        assert!(!tower.is_active(), "a speed of 0 does nothing");
        tower.set_speed(f32::INFINITY);
        assert_eq!(tower.speed, WHIRL_SPEED_MIN);
        tower.set_speed(10.0);
        (tower.bibites, tower.pellets, tower.eggs) = (false, false, false);
        assert!(!tower.is_active(), "with every switch off it does nothing");
    }

    #[test]
    fn a_counterclockwise_whirl_pushes_a_body_east_of_the_tower_north_and_a_clockwise_one_south() {
        let mut tower = tower();
        tower.set_speed(60.0);
        tower.falloff = ForceFalloff::Constant;
        let dt = 1.0 / 30.0;
        let east = Vec2::new(200.0, -50.0);
        let kick = tower.kick(east, Vec2::ZERO, dt);
        assert!(kick.x.abs() < 1e-5 && (kick.y - 60.0 * dt).abs() < 1e-5, "{kick:?}");
        tower.spin = Spin::Clockwise;
        let kick = tower.kick(east, Vec2::ZERO, dt);
        assert!(kick.x.abs() < 1e-5 && (kick.y + 60.0 * dt).abs() < 1e-5, "{kick:?}");
        // The four sides of a clockwise whirl: east goes south, south goes west, west goes north, north goes east.
        for (offset, expected) in [((50.0, 0.0), (0.0, -1.0)), ((0.0, -50.0), (-1.0, 0.0)), ((-50.0, 0.0), (0.0, 1.0)), ((0.0, 50.0), (1.0, 0.0))] {
            let kick = tower.kick(tower.base.pos + Vec2::new(offset.0, offset.1), Vec2::ZERO, dt);
            let (x, y) = (kick.x / (60.0 * dt), kick.y / (60.0 * dt));
            assert!((x - expected.0).abs() < 1e-4 && (y - expected.1).abs() < 1e-4, "{offset:?}: {kick:?}");
        }
        // Along a diagonal the push is still along the circle (at right angles to the line to the tower) and the same size.
        let at = tower.base.pos + Vec2::new(30.0, 40.0);
        let kick = tower.kick(at, Vec2::ZERO, dt);
        assert!(kick.dot(at - tower.base.pos).abs() < 1e-4, "{kick:?} is not along the circle");
        assert!((kick.length() - 60.0 * dt).abs() < 1e-5);
    }

    #[test]
    fn a_body_that_is_going_round_is_held_on_its_circle_with_the_centripetal_acceleration_of_its_speed() {
        let mut tower = tower();
        tower.set_speed(60.0);
        tower.falloff = ForceFalloff::Constant;
        let dt = 1.0 / 30.0;
        // 100 u east of the tower, going north (the way a counterclockwise whirl goes) at 60 u/s: v^2 / r = 36 u/s^2 towards the tower, on top of the push of 60 north.
        let at = tower.base.pos + Vec2::new(100.0, 0.0);
        let kick = tower.kick(at, Vec2::new(0.0, 60.0), dt);
        assert!((kick.x + 36.0 * dt).abs() < 1e-4 && (kick.y - 60.0 * dt).abs() < 1e-5, "{kick:?}");
        // Going the other way round (south) it is held on its circle just the same.
        let kick = tower.kick(at, Vec2::new(0.0, -60.0), dt);
        assert!((kick.x + 36.0 * dt).abs() < 1e-4, "{kick:?}");
        // Going straight outwards or inwards (no speed along the circle): no hold.
        for velocity in [Vec2::new(80.0, 0.0), Vec2::new(-80.0, 0.0), Vec2::ZERO] {
            assert!(tower.kick(at, velocity, dt).x.abs() < 1e-6, "{velocity:?}");
        }
        // A fading whirl holds in the same proportion as it pushes: at half way, half of both.
        tower.set_range(400.0);
        tower.falloff = ForceFalloff::Fading;
        let half_way = tower.base.pos + Vec2::new(200.0, 0.0);
        let kick = tower.kick(half_way, Vec2::new(0.0, 60.0), dt);
        assert!((kick.y - 30.0 * dt).abs() < 1e-4 && (kick.x + 0.5 * (3600.0 / 200.0) * dt).abs() < 1e-4, "{kick:?}");
        // Close to the tower a fast body is not turned by more than the whole of its speed in one tick.
        tower.set_range(400.0);
        tower.falloff = ForceFalloff::Constant;
        let close = tower.base.pos + Vec2::new(WHIRL_CORE, 0.0);
        let fast = Vec2::new(0.0, 400.0);
        let kick = tower.kick(close, fast, dt);
        assert!(kick.x.is_finite() && kick.x <= 0.0 && kick.x.abs() <= 400.0 + 1e-3, "{kick:?}");
        // A velocity that is not a number does no hold.
        let kick = tower.kick(at, Vec2::new(f32::NAN, 0.0), dt);
        assert!(kick.is_finite());
    }

    #[test]
    fn the_falloff_and_the_edges_are_the_force_towers() {
        let mut tower = tower();
        tower.set_range(400.0);
        tower.set_speed(100.0);
        tower.falloff = ForceFalloff::Constant;
        for distance in [WHIRL_CORE, 10.0, 200.0, 399.0] {
            assert_eq!(tower.acceleration_at(distance), 100.0, "{distance}");
        }
        tower.falloff = ForceFalloff::Fading;
        assert!((tower.acceleration_at(200.0) - 50.0).abs() < 1e-4, "half way, half the speed");
        for falloff in [ForceFalloff::Constant, ForceFalloff::Fading] {
            tower.falloff = falloff;
            for distance in [400.0, 401.0, 1.0e6, 0.0, WHIRL_CORE - 0.01, f32::NAN, f32::INFINITY] {
                assert_eq!(tower.acceleration_at(distance), 0.0, "{falloff:?} at {distance}");
            }
        }
        // On the tower, out of range, and a tick that is not a tick: nothing.
        let dt = 1.0 / 30.0;
        let moving = Vec2::new(0.0, 50.0);
        assert_eq!(tower.kick(tower.base.pos, moving, dt), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(5000.0, -50.0), moving, dt), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(200.0, -50.0), moving, 0.0), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(200.0, -50.0), moving, f32::NAN), Vec2::ZERO);
        assert_eq!(tower.kick(Vec2::new(f32::NAN, 0.0), moving, dt), Vec2::ZERO);
    }

    #[test]
    fn the_spin_words_round_trip_and_the_short_ones_work() {
        for spin in [Spin::Clockwise, Spin::Counterclockwise] {
            assert_eq!(Spin::from_key(spin.key()), Some(spin));
            assert_eq!(Spin::from_key(&format!("  {} ", spin.key().to_uppercase())), Some(spin));
            assert_eq!(spin.reversed().reversed(), spin);
            assert_ne!(spin.reversed(), spin);
            assert_eq!(spin.sign(), -spin.reversed().sign());
        }
        assert_eq!((Spin::from_key("cw"), Spin::from_key("CCW")), (Some(Spin::Clockwise), Some(Spin::Counterclockwise)));
        assert_eq!(Spin::from_key("left"), None);
        assert_eq!(Spin::default(), Spin::Counterclockwise);
    }
}
