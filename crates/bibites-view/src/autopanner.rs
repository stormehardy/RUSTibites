//! The auto-panner (`Utility.AutoPanner`, the scene's `Autopanner` object): a rigid body that
//! drifts from zone position to zone position, which the K key makes the camera's target
//! (`UserControl.Update`: `SelectTarget(autoPanner)`, and `CameraManager` follows any target).
//!
//! Scene values (level2): `Rigidbody2D` dynamic, mass 10, linear damping 0.05, gravity scale 1 (the
//! world has no gravity); `AutoPanner`: force 10, max speed 5, wait 30 to 90 s. Every physics step it
//! adds `force * direction * (maxSpeed - speed)` towards its destination, picks a new destination
//! (`ZoneManager.GetRandomPositionInZone`) when it is within 10 units of it, and the coroutine
//! picks one every 30 to 90 s of game time besides.

use bibites_core::math::Vec2;

/// `AutoPanner.force`.
pub const FORCE: f32 = 10.0;
/// `AutoPanner.maxSpeed` (the scene overrides the script's default of 1).
pub const MAX_SPEED: f32 = 5.0;
/// `Rigidbody2D.mass`.
pub const MASS: f32 = 10.0;
/// `Rigidbody2D.linearDamping`.
pub const DAMPING: f32 = 0.05;
/// `AutoPanner.minWait`.
pub const MIN_WAIT: f32 = 30.0;
/// `AutoPanner.maxWait`.
pub const MAX_WAIT: f32 = 90.0;
/// The distance at which the destination counts as reached.
pub const ARRIVED: f32 = 10.0;

/// The body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoPanner {
    /// Position (the object starts at the origin).
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Where it is heading (the origin until the first step picks one).
    pub dest: Vec2,
    /// Game seconds until the coroutine picks the next destination.
    pub wait: f32,
}

/// What a step needs from outside: random choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// A new destination (`NewDir`: it arrived, or the wait ended).
    pub destination: bool,
    /// A new waiting time (the wait ended).
    pub wait: bool,
}

impl AutoPanner {
    /// The panner at the origin, waiting `first_wait` seconds before its first timed change.
    #[must_use]
    pub fn new(first_wait: f32) -> Self {
        Self {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            dest: Vec2::ZERO,
            wait: first_wait,
        }
    }

    /// One physics step of `dt` game seconds (`FixedUpdate` and the `ChangeDirection` coroutine).
    /// The caller picks the new destination and waiting time the result asks for.
    #[must_use]
    pub fn step(&mut self, dt: f32) -> Needs {
        let to = self.dest - self.pos;
        // The direction is the one measured before the destination changes (`NewDir` follows it).
        let direction = to.normalize_or(Vec2::ZERO);
        let arrived = to.length() < ARRIVED;
        let push = direction * (FORCE * (MAX_SPEED - self.vel.length()));
        // Box2D: velocity += dt * force / mass, then damping, then position += dt * velocity.
        self.vel += push * (dt / MASS);
        self.vel *= 1.0 / (1.0 + DAMPING * dt);
        self.pos += self.vel * dt;
        self.wait -= dt;
        let timer = self.wait <= 0.0;
        Needs {
            destination: arrived || timer,
            wait: timer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_speeds_up_towards_its_destination_and_levels_off_below_the_max_speed() {
        let mut panner = AutoPanner::new(1.0e9);
        panner.dest = Vec2::new(10_000.0, 0.0);
        for _ in 0..(60 * 60) {
            let _ = panner.step(1.0 / 60.0);
        }
        // After a minute it moves at about 5 u/s, a little less because of the damping.
        let speed = panner.vel.length();
        assert!(speed > 4.0 && speed < 5.0, "{speed}");
        assert!(panner.vel.y.abs() < 1e-3, "{:?}", panner.vel);
        assert!(
            panner.pos.x > 150.0 && panner.pos.x < 300.0,
            "{:?}",
            panner.pos
        );
    }

    #[test]
    fn it_asks_for_a_new_destination_when_it_gets_there_and_when_the_wait_ends() {
        // At the origin with the origin as destination: the first step asks.
        let mut panner = AutoPanner::new(1.0e9);
        let needs = panner.step(0.02);
        assert_eq!(
            needs,
            Needs {
                destination: true,
                wait: false
            }
        );
        // Far from it and not waiting out: nothing to ask.
        panner.dest = Vec2::new(5000.0, 5000.0);
        assert_eq!(panner.step(0.02), Needs::default());
        // The timer: asks for both when it ends.
        let mut panner = AutoPanner::new(0.05);
        panner.dest = Vec2::new(5000.0, 5000.0);
        let mut asked = 0;
        for _ in 0..10 {
            let needs = panner.step(0.02);
            if needs.destination {
                assert!(needs.wait);
                asked += 1;
                panner.wait = 70.0;
            }
        }
        assert_eq!(asked, 1);
    }
}
