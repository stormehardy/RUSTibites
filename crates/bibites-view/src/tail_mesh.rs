//! Deterministic simulation of the game's procedural tail.
//!
//! The game has **no tail mesh**: `SimulationScripts/ProceduralSpriteController`
//! sits on the Body `SpriteRenderer` and drives `_MiddleSegmentAngle` /
//! `_TailAngle` uniforms, which bend the tail region of the body sprite
//! (`ProceduralBibiteMaterial`). This module reproduces the *trailing-spring
//! dynamics* of that controller exactly, so the renderer can deform the body
//! quad while keeping playback deterministic.
//!
//! Constants are taken verbatim from the decompiled controller:
//! `segmentDist = 4.5 * d1`, `tailDist = 7.5 * d1`,
//! `maxSegmentAngle = 7.5°`, `maxTailAngle = 25°`, and the per-frame
//! relaxation `angle = sign * max(0, min(|target|, max) * (1 - 0.5*dt) - dt)`.
//!
//! The controller integrates with `Time.deltaTime`; the viewer only calls this
//! from its fixed timestep, so the result is a pure function of the body
//! transform history.

#![allow(dead_code)] // renderer hook lands next; simulation + tests are used now.

use bibites_core::math::Vec2;

/// Tail segment distances in `d1` units (game constants).
pub const SEGMENT_DIST_PER_D1: f32 = 4.5;
/// Tail tip distance in `d1` units (game constants).
pub const TAIL_DIST_PER_D1: f32 = 7.5;
/// Maximum middle-segment angle in degrees.
pub const MAX_SEGMENT_ANGLE_DEG: f32 = 7.5;
/// Maximum tail angle in degrees.
pub const MAX_TAIL_ANGLE_DEG: f32 = 25.0;

/// Tail angles and trailing points for one bibite.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TailState {
    /// `_MiddleSegmentAngle` in radians (body→segment bend).
    pub middle_angle: f32,
    /// `_TailAngle` in radians (segment→tail bend).
    pub tail_angle: f32,
    /// World-space segment joint.
    pub segment_point: Vec2,
    /// World-space tail tip.
    pub tail_point: Vec2,
    /// `damagedFlash` (`_HurtAmount` of the skin shader): set by a hit, falls by 2 a second.
    pub hurt: f32,
    /// The health the last tick saw (`NaN` before the first), to tell what a hit took.
    pub last_health: f32,
}

impl TailState {
    /// Initialises the trailing chain straight behind the body (`-up`), which
    /// is what `ProceduralSpriteController.OnBecameVisible` does.
    #[must_use]
    pub fn new(body_position: Vec2, up: Vec2, d1: f32) -> Self {
        let up = normalized_or(up, Vec2::Y);
        let segment_point = body_position - up * (SEGMENT_DIST_PER_D1 * d1);
        let tail_point = segment_point - up * (TAIL_DIST_PER_D1 * d1);
        Self {
            middle_angle: 0.0,
            tail_angle: 0.0,
            segment_point,
            tail_point,
            hurt: 0.0,
            last_health: f32::NAN,
        }
    }

    /// Advances the trailing chain by one fixed timestep.
    ///
    /// `body_position`/`up` are the body's world transform; `up` is the body's
    /// forward axis (`transform.up` in Unity). `dt` is the fixed timestep.
    pub fn step(&mut self, body_position: Vec2, up: Vec2, d1: f32, dt: f32) {
        let up = normalized_or(up, Vec2::Y);
        let segment_dist = SEGMENT_DIST_PER_D1 * d1;
        let tail_dist = TAIL_DIST_PER_D1 * d1;
        let dt = dt.max(0.0);

        // --- middle segment: point follows the body with a clamped angle. ---
        let mut segment = self.segment_point - body_position;
        if length_sq(segment) <= f32::EPSILON {
            segment = -up * segment_dist;
        }
        segment = normalized_or(segment, -up) * segment_dist;
        let target = signed_angle_deg(-up, segment);
        let clamped = clamp_angle(target, MAX_SEGMENT_ANGLE_DEG, dt);
        segment = rotate(segment, (clamped - target).to_radians());
        self.segment_point = body_position + segment;
        self.middle_angle = clamped.to_radians();

        // --- tail: follows the segment with a larger clamped angle. ---
        let mut tail = self.tail_point - self.segment_point;
        if length_sq(tail) <= f32::EPSILON {
            tail = segment;
        }
        tail = normalized_or(tail, segment) * tail_dist;
        let target = signed_angle_deg(segment, tail);
        let clamped = clamp_angle(target, MAX_TAIL_ANGLE_DEG, dt);
        tail = rotate(tail, (clamped - target).to_radians());
        self.tail_point = self.segment_point + tail;
        self.tail_angle = clamped.to_radians();
    }

    /// Angle used by the body-sprite warp: first row below the body centre.
    #[must_use]
    pub fn warp_angles(&self) -> (f32, f32) {
        (self.middle_angle, self.tail_angle)
    }
}

/// Game's per-frame clamping/relaxation (`ProceduralSpriteController.Update`).
fn clamp_angle(target_deg: f32, max_deg: f32, dt: f32) -> f32 {
    let magnitude = target_deg.abs().min(max_deg);
    let relaxed = (magnitude * (1.0 - 0.5 * dt) - dt).max(0.0);
    target_deg.signum() * relaxed
}

/// Unity `Vector2.SignedAngle(from, to)` in degrees, `(-180, 180]`.
fn signed_angle_deg(from: Vec2, to: Vec2) -> f32 {
    if length_sq(from) <= f32::EPSILON || length_sq(to) <= f32::EPSILON {
        return 0.0;
    }
    let cross = from.x * to.y - from.y * to.x;
    let dot = from.dot(to);
    cross.atan2(dot).to_degrees()
}

fn length_sq(vector: Vec2) -> f32 {
    vector.dot(vector)
}

fn rotate(vector: Vec2, radians: f32) -> Vec2 {
    let (sin, cos) = radians.sin_cos();
    Vec2::new(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
    )
}

fn normalized_or(vector: Vec2, fallback: Vec2) -> Vec2 {
    let length = vector.length();
    if length > 1e-6 && length.is_finite() {
        vector / length
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D1: f32 = 1.0;
    const DT: f32 = 1.0 / 30.0;

    #[test]
    fn initialises_straight_behind_body() {
        let state = TailState::new(Vec2::ZERO, Vec2::Y, D1);
        assert_eq!(state.middle_angle, 0.0);
        assert_eq!(state.tail_angle, 0.0);
        assert!((state.segment_point - Vec2::new(0.0, -4.5)).length() < 1e-5);
        assert!((state.tail_point - Vec2::new(0.0, -12.0)).length() < 1e-5);
    }

    #[test]
    fn segment_angle_is_clamped() {
        let mut state = TailState::new(Vec2::ZERO, Vec2::Y, D1);
        // Move the body sharply sideways so the desired bend exceeds 7.5°.
        state.step(Vec2::new(10.0, 0.0), Vec2::Y, D1, DT);
        assert!(
            state.middle_angle.to_degrees().abs() <= MAX_SEGMENT_ANGLE_DEG + 1e-3,
            "middle angle {} exceeds clamp",
            state.middle_angle.to_degrees()
        );
        // The tail follows the segment and respects its own (larger) clamp.
        state.step(Vec2::new(20.0, 0.0), Vec2::Y, D1, DT);
        assert!(
            state.tail_angle.to_degrees().abs() <= MAX_TAIL_ANGLE_DEG + 1e-3,
            "tail angle {} exceeds clamp",
            state.tail_angle.to_degrees()
        );
    }

    #[test]
    fn relaxed_to_zero_when_aligned() {
        let mut state = TailState::new(Vec2::ZERO, Vec2::Y, D1);
        state.step(Vec2::new(5.0, 0.0), Vec2::Y, D1, DT);
        let bent = state.middle_angle.abs();
        // Hold the chain straight behind the body: the spring relaxes to 0.
        for _ in 0..600 {
            state.step(Vec2::ZERO, Vec2::Y, D1, DT);
        }
        assert!(bent > 0.0);
        assert!(
            state.middle_angle.abs() < 1e-3,
            "middle angle did not relax: {}",
            state.middle_angle
        );
        assert!(state.tail_angle.abs() < 1e-3);
    }

    #[test]
    fn tuning_matches_game_constants() {
        assert!((SEGMENT_DIST_PER_D1 - 4.5).abs() < f32::EPSILON);
        assert!((TAIL_DIST_PER_D1 - 7.5).abs() < f32::EPSILON);
        assert!((MAX_SEGMENT_ANGLE_DEG - 7.5).abs() < f32::EPSILON);
        assert!((MAX_TAIL_ANGLE_DEG - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn signed_angle_wraps() {
        assert!((signed_angle_deg(Vec2::Y, Vec2::Y) - 0.0).abs() < 1e-4);
        assert!((signed_angle_deg(Vec2::Y, Vec2::X) + 90.0).abs() < 1e-4);
        assert!((signed_angle_deg(Vec2::Y, -Vec2::X) - 90.0).abs() < 1e-4);
    }
}
