//! Pure 2-D collision geometry and a deterministic sequential-impulse solver.
//!
//! This module replaces Unity's Box2D for the headless engine (see
//! `docs/ARCHITECTURE.md` §7). It is deliberately store-free: callers build a
//! slice of [`crate::physics::PhysicsBody`] values (B2 adapts `BibiteStore`/`PelletStore`/
//! `EggStore` into this representation each tick), call [`crate::physics::step`], and write the
//! poses/velocities back through the public fields.
//!
//! ## Captured configuration
//!
//! All constants come from the parity capture
//! `testdata/parity/prefab_dump.json` (game `0.6.3.1`, Unity `6000.0.44f1`)
//! and Lead's P1 task:
//! * `physics2d.properties`: gravity `(0,0)`, velocityIterations `5`,
//!   positionIterations `3`, `defaultContactOffset` `0.01`, `bounceThreshold`
//!   `1`, `maxLinearCorrection` `0.2`, `maxAngularCorrection` `8`,
//!   `maxTranslationSpeed` `100`, `maxRotationSpeed` `360`,
//!   `baumgarteScale` `0.2`.
//! * collider prefabs: bibite Body capsule `(5,9)` offset `(-1.5,0)`, Tail
//!   capsule `(5,7)` offset `(0,-4.7)` (the tail is also
//!   `BibiteBody.bodyCollider`, which the game's inflate code overwrites),
//!   Mouth box `(2.72,7)` offset `(2.38,0)` trigger; egg circle radius `5`
//!   offset `(0,-0.5)`; pellet circle radius `2.5`. All prefab units are
//!   scaled by the entity transform by the caller (body `d1Size`, pellet
//!   `sqrt(amount/20)`, egg `D1SizeAtBirth`).
//! * `physics_materials`: `corps` friction `0.15` / bounciness `0.1` (bibite
//!   capsules and egg), `BouncyBox` `0.4` / `0.4` (plant pellet),
//!   `Slippery` `0.02` / `0.0` (meat pellet). Friction combine `Mean`,
//!   bounce combine `Maximum`.
//! * layer matrix: layers 0..11 = `Default, TransparentFX, IgnoreRaycast,
//!   UIRenders, Water, UI, UIPhysics, SceneTriggers, bibite, pellets,
//!   pheromones, UIRenders2`; enabled pairs are exactly `(6,6)`, `(7,8)`,
//!   `(7,9)`, `(8,8)`, `(8,9)`, `(9,9)`.
//!
//! ## Intentional P1 limitations
//!
//! * The broadphase is a deterministic sweep-and-prune over collider AABBs
//!   (kept inside this module because world-dev's `SpatialHash` is keyed by
//!   arena handles; the P1 solver is handle-free so it can be tested in
//!   isolation). It becomes a uniform hash later if profiling needs it.
//! * Each primitive pair produces one contact point (enough for the required
//!   circle/capsule/box behaviour and tests; 2-point box manifolds are a later
//!   refinement).
//! * Sleeping, sub-stepping, composite colliders and arbitrary polygon shapes
//!   are not modelled (the captured configuration has none of the latter
//!   enabled).
//! * `capsule-box` uses a deterministic 32-step ternary search for the closest
//!   point on the capsule segment to the oriented box (convex 1-D problem,
//!   error < 1e-6); a closed form is possible but unnecessary here.
//!
//! All state is `f32`; the tick path reuses [`crate::physics::PhysicsScratch`] buffers and does
//! not allocate once warm.

use crate::math::{Vec2, clamp};
use crate::sim::{Stage, StageClock};
use rayon::prelude::*;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Prefab constants (captured; see module docs)
// ---------------------------------------------------------------------------

/// Captured Unity prefab values that B2 needs to build physics shapes.
pub mod prefab {
    use crate::math::Vec2;

    /// Bibite `Body` capsule size `(width, height)` in local units.
    pub const BODY_CAPSULE_SIZE: Vec2 = Vec2::new(5.0, 9.0);
    /// Bibite `Body` capsule offset in local units.
    pub const BODY_CAPSULE_OFFSET: Vec2 = Vec2::new(-1.5, 0.0);
    /// Bibite `Tail` capsule size (also `BibiteBody.bodyCollider`).
    pub const TAIL_CAPSULE_SIZE: Vec2 = Vec2::new(5.0, 7.0);
    /// Bibite `Tail` capsule offset.
    pub const TAIL_CAPSULE_OFFSET: Vec2 = Vec2::new(0.0, -4.7);
    /// `Body`/`Mouth` child local rotation (Unity quaternion z=0.707107 →
    /// 90°, from assets/game/prefabs.json; `Tail` has identity rotation).
    pub const PART_ROTATION: f32 = std::f32::consts::FRAC_PI_2;
    /// Mouth trigger box size for the herbivore base form.
    pub const MOUTH_BOX_SIZE: Vec2 = Vec2::new(2.72, 7.0);
    /// Mouth trigger box offset in the Mouth child's local frame.
    pub const MOUTH_BOX_OFFSET: Vec2 = Vec2::new(2.38, 0.0);

    /// Game mouth-box length for a diet (`BibiteMouth.cs:295-298`):
    /// `diet < 0.5 ? 7 - 8*diet : 3`, width 2.72.
    #[must_use]
    pub fn mouth_box_size(diet: f32) -> Vec2 {
        Vec2::new(2.72, if diet < 0.5 { 7.0 - 8.0 * diet } else { 3.0 })
    }

    /// Mouth child offset in the bibite body frame (90° child rotation).
    #[must_use]
    pub fn mouth_offset(d1: f32) -> Vec2 {
        MOUTH_BOX_OFFSET.rotate(PART_ROTATION) * d1
    }

    /// Tail capsule size including the fat inflate lerp
    /// (`BibiteBody.cs:733-734`: 5→8.5 wide, 7→10.5 tall).
    #[must_use]
    pub fn tail_capsule_size(inflate_factor: f32) -> Vec2 {
        let t = inflate_factor.clamp(0.0, 1.0);
        Vec2::new(5.0 + 3.5 * t, 7.0 + 3.5 * t)
    }

    /// Tail capsule offset including inflate (`-4.7 → -5.5`).
    #[must_use]
    pub fn tail_capsule_offset(inflate_factor: f32) -> Vec2 {
        Vec2::new(0.0, -4.7 - 0.8 * inflate_factor.clamp(0.0, 1.0))
    }

    /// `Body` capsule offset in the bibite body frame (90° child rotation).
    #[must_use]
    pub fn body_capsule_offset(d1: f32) -> Vec2 {
        BODY_CAPSULE_OFFSET.rotate(PART_ROTATION) * d1
    }
    /// Egg circle radius (before transform scale).
    pub const EGG_CIRCLE_RADIUS: f32 = 5.0;
    /// Egg circle offset.
    pub const EGG_CIRCLE_OFFSET: Vec2 = Vec2::new(0.0, -0.5);
    /// Pellet circle radius at transform scale 1 (amount `20 u²`).
    pub const PELLET_CIRCLE_RADIUS: f32 = 2.5;
    /// Bibite/egg physics material friction (`corps`).
    pub const CORPS_FRICTION: f32 = 0.15;
    /// Bibite/egg physics material bounciness (`corps`).
    pub const CORPS_RESTITUTION: f32 = 0.1;
    /// Plant pellet physics material friction (`BouncyBox`). The asset says 0.4; against the game a plant pellet hit by the side of a bibite is carried
    /// forward further than 0.4 gives (a pass-by at a gap of -0.01 .. 0.8: 7.4 u against 6.3, while a meat pellet, friction 0.02, matches within 3 %) and
    /// piles of plant pellets pushed by a bibite end 1 u behind the game's at 76 ticks. Fitted: 0.7 (glancing hits 6.6 -> 2.6 u of total error, jittered
    /// piles 16.7 -> 12.2). Why the asset's value is not what runs is not known.
    pub const PLANT_FRICTION: f32 = 0.7;
    /// Plant pellet physics material bounciness (`BouncyBox`).
    pub const PLANT_RESTITUTION: f32 = 0.4;
    /// Meat pellet physics material friction (`Slippery`).
    pub const MEAT_FRICTION: f32 = 0.02;
    /// Meat pellet physics material bounciness (`Slippery`).
    pub const MEAT_RESTITUTION: f32 = 0.0;
    /// Prefab linear damping for bibites, pellets and eggs.
    pub const LINEAR_DAMPING: f32 = 1.0;
    /// Prefab angular damping for bibites.
    pub const BIBITE_ANGULAR_DAMPING: f32 = 0.5;
    /// Prefab angular damping for pellets.
    pub const PELLET_ANGULAR_DAMPING: f32 = 2.0;
    /// Prefab angular damping for eggs.
    pub const EGG_ANGULAR_DAMPING: f32 = 1.0;
}

// ---------------------------------------------------------------------------
// Layers and collision filtering
// ---------------------------------------------------------------------------

/// Name of each captured layer, indexed by layer id (`0..=11`).
pub const LAYER_NAMES: [&str; 12] = [
    "Default",
    "TransparentFX",
    "IgnoreRaycast",
    "UIRenders",
    "Water",
    "UI",
    "UIPhysics",
    "SceneTriggers",
    "bibite",
    "pellets",
    "pheromones",
    "UIRenders2",
];
/// `UIPhysics` layer (6) — only collides with itself in the capture.
pub const LAYER_UI_PHYSICS: u8 = 6;
/// `SceneTriggers` layer (7).
pub const LAYER_SCENE_TRIGGERS: u8 = 7;
/// `bibite` layer (8).
pub const LAYER_BIBITE: u8 = 8;
/// `pellets` layer (9).
pub const LAYER_PELLETS: u8 = 9;
/// `pheromones` layer (10). Nothing collides with it physically; pheromone
/// spots are query-only.
pub const LAYER_PHEROMONES: u8 = 10;
/// Number of captured layers with names.
pub const LAYER_COUNT: usize = 12;

/// Enabled collision pairs from the captured matrix (symmetric).
const ENABLED_PAIRS: [(u8, u8); 6] = [(6, 6), (7, 8), (7, 9), (8, 8), (8, 9), (9, 9)];

/// Returns whether two layers collide, exactly per the captured matrix
/// (`testdata/parity/prefab_dump.json`, `ignore_layer_collision_matrix`).
pub fn collision_allowed(layer_a: u8, layer_b: u8) -> bool {
    let (low, high) = if layer_a <= layer_b {
        (layer_a, layer_b)
    } else {
        (layer_b, layer_a)
    };
    ENABLED_PAIRS.contains(&(low, high))
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Rule used to combine two colliders' friction or restitution values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CombineRule {
    /// Geometric mean (`sqrt(a * b)`) — Box2D's `b2MixFriction`, captured as
    /// Unity's `Mean`.
    #[default]
    Mean,
    /// Smaller of the two values.
    Min,
    /// Product of the two values.
    Multiply,
    /// Larger of the two values (Box2D's `b2MixRestitution`, captured as
    /// Unity's `Maximum`).
    Maximum,
}

impl CombineRule {
    /// Combines two material coefficients.
    pub fn combine(self, a: f32, b: f32) -> f32 {
        match self {
            Self::Mean => (a * b).max(0.0).sqrt(),
            Self::Min => a.min(b),
            Self::Multiply => a * b,
            Self::Maximum => a.max(b),
        }
    }
}

/// Captured `Physics2D`/prefab configuration for the solver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsConfig {
    /// Sequential velocity iterations (`physics2d.properties.velocityIterations`).
    pub velocity_iterations: u32,
    /// Position-correction iterations (`positionIterations`).
    pub position_iterations: u32,
    /// Contact skin/generation offset (`defaultContactOffset`).
    pub contact_offset: f32,
    /// How far apart two solid shapes may be and still hold a contact (the soft step's speculative contacts: the bodies may close the gap, no
    /// further). Measured against the original's collisions.
    pub speculative_distance: f32,
    /// Minimum approach speed for restitution (`bounceThreshold`).
    pub bounce_threshold: f32,
    /// Maximum linear speed in u/s (`maxTranslationSpeed`).
    pub max_translation_speed: f32,
    /// Maximum angular speed in degrees/s (`maxRotationSpeed`).
    pub max_rotation_speed: f32,
    /// Box2D linear slop (position-solver target overlap).
    ///
    /// ASSUMPTION(BOX2D) — `b2_linearSlop = 0.005`; Unity does not expose it.
    pub linear_slop: f32,
    /// Baumgarte position-correction factor (`baumgarteScale`).
    pub baumgarte: f32,
    /// Maximum positional correction per iteration (`maxLinearCorrection`).
    pub max_linear_correction: f32,
    /// Maximum angular correction per position iteration
    /// (`maxAngularCorrection`, degrees).
    pub max_angular_correction: f32,
    /// Contacts found before the shapes touch when the step's motion will bring them together (the game's contacts catch a body flying at another one;
    /// measured at ten steps per tick). At one step per tick the game finds a contact only once the shapes touch (within the contact offset), a tick later.
    pub catch_fast_approach: bool,
    /// Box2D 2.4's solver for the step (velocity iterations, positions integrated with the solved velocities, then Baumgarte position iterations): the game at one
    /// physics step per tick. Off with the soft step (the game slowed down to ten steps per tick).
    pub box2d_solver: bool,
    /// Substeps of one step (the game's solver is Box2D's soft step: measured against the original, a body's damping and motion are split in ten).
    pub substeps: u32,
    /// Physics steps of one tick (the game runs ten, `FixedUpdate` adding the muscles' force before each): the bodies that touch, or are about to,
    /// are stepped that many times with the contacts found again every time; with 1 a tick is one step of `substeps` substeps.
    pub fine_steps: u32,
    /// Contact softness (`b2WorldDef::contactHertz`, capped at a quarter of the substep rate), its damping ratio, and the fastest a contact pushes
    /// overlapping bodies apart (`contactPushMaxVelocity`). Measured against the original with bodies spawned deep inside each other (a 400-unit pellet
    /// 1.2 units into a bibite is out by the end of the first tick, 6 units of separation in ten steps): the cap of the hertz (75 at ten steps of 30 ticks
    /// per second), a damping ratio of 1 and no limit on the push; the defaults of Box2D (30 Hz, 10, 3) take twelve ticks.
    pub contact_hertz: f32,
    /// See above.
    pub contact_damping_ratio: f32,
    /// See above.
    pub contact_push_speed: f32,
    /// A pellet that gets held has no drag (`disableDragOnHeldPellet`): the grabs made inside the steps need it.
    pub held_drag_off: bool,
    /// Friction combination rule (captured `Mean`).
    pub friction_combine: CombineRule,
    /// Restitution combination rule (captured `Maximum`).
    pub restitution_combine: CombineRule,
}

/// Tests of the ten-step machinery (the game slowed to 0.1x) ask for it: the guard makes [`PhysicsConfig::default`] on this thread ten steps of ten substeps.
#[cfg(test)]
pub(crate) struct TenStepsGuard;

#[cfg(test)]
thread_local! {
    static TEN_STEPS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
impl Drop for TenStepsGuard {
    fn drop(&mut self) {
        TEN_STEPS.with(|flag| flag.set(false));
    }
}

/// See [`TenStepsGuard`].
#[cfg(test)]
pub(crate) fn ten_steps_for_this_test() -> TenStepsGuard {
    TEN_STEPS.with(|flag| flag.set(true));
    TenStepsGuard
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        // The game at 1x speed (and faster) runs ONE physics step of `1 / simTPS` per tick (measured against the original on a 1200-bibite save: a
        // body's displacement is `dt * v_next`). `RUSTIBITES_PHYS="<fine_steps>,<substeps>"` overrides it (10,10 is the game slowed to 0.1x: ten
        // FixedUpdates of a tenth of the time each).
        let (fine, subs) = std::env::var("RUSTIBITES_PHYS")
            .ok()
            .and_then(|v| {
                let mut it = v.split(',').filter_map(|x| x.trim().parse::<u32>().ok());
                Some((it.next()?, it.next()?))
            })
            .unwrap_or((1, 1));
        #[cfg(test)]
        let (fine, subs) = if TEN_STEPS.with(std::cell::Cell::get) { (DAMPING_SLICES as u32, DAMPING_SLICES as u32) } else { (fine, subs) };
        Self {
            velocity_iterations: 5,
            position_iterations: 3,
            contact_offset: 0.01,
            // (Box2D 2.4 finds a contact within the shapes' skin, 0.01; the soft step looks 0.05 ahead.)
            speculative_distance: match std::env::var("RUSTIBITES_SPEC").ok().and_then(|v| v.trim().parse::<f32>().ok()) { Some(value) => value, None => if fine == 1 && subs == 1 { 0.01 } else { 0.05 } },
            bounce_threshold: 1.0,
            max_translation_speed: 400.0,
            max_rotation_speed: 360.0,
            linear_slop: 0.005,
            baumgarte: 0.2,
            max_linear_correction: 0.2,
            max_angular_correction: 8.0,
            box2d_solver: match std::env::var("RUSTIBITES_BOX2D").ok().as_deref() { Some("1") => true, Some("0") => false, _ => fine == 1 && subs == 1 },
            catch_fast_approach: match std::env::var("RUSTIBITES_CATCH").ok().as_deref() { Some("1") => true, Some("0") => false, _ => fine > 1 },
            substeps: subs,
            fine_steps: fine,
            contact_hertz: 1000.0,
            contact_damping_ratio: 1.0,
            contact_push_speed: 1000.0,
            held_drag_off: false,
            friction_combine: CombineRule::Mean,
            restitution_combine: CombineRule::Maximum,
        }
    }
}

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

/// Axis-aligned world bounds rectangle (`[min, max]` inclusive).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldBounds {
    /// Minimum corner.
    pub min: Vec2,
    /// Maximum corner.
    pub max: Vec2,
}

impl WorldBounds {
    /// Builds bounds from a half extent centred on the origin.
    pub fn from_half_extent(half_extent: Vec2) -> Self {
        Self {
            min: -half_extent,
            max: half_extent,
        }
    }

    /// Builds bounds from two corners.
    pub fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// Returns whether a point is inside the bounds.
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Clamps a point into the bounds.
    pub fn clamp_point(&self, point: Vec2) -> Vec2 {
        Vec2::new(
            clamp(point.x, self.min.x, self.max.x),
            clamp(point.y, self.min.y, self.max.y),
        )
    }

    /// Returns whether an AABB overlaps the bounds.
    pub fn overlaps(&self, aabb: &Aabb) -> bool {
        aabb.max.x >= self.min.x
            && aabb.min.x <= self.max.x
            && aabb.max.y >= self.min.y
            && aabb.min.y <= self.max.y
    }

    /// Width and height of the rectangle.
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: Vec2,
    /// Maximum corner.
    pub max: Vec2,
}

impl Aabb {
    /// Bounding box around a point with a radius.
    pub fn around(point: Vec2, radius: f32) -> Self {
        let r = Vec2::splat(radius);
        Self {
            min: point - r,
            max: point + r,
        }
    }

    /// Merges two boxes.
    pub fn merge(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Returns whether two boxes overlap (touching counts).
    pub fn overlaps(&self, other: &Self) -> bool {
        self.max.x >= other.min.x
            && other.max.x >= self.min.x
            && self.max.y >= other.min.y
            && other.max.y >= self.min.y
    }

    /// Centre of the box.
    pub fn centre(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// Half extents of the box.
    pub fn half_extents(&self) -> Vec2 {
        (self.max - self.min) * 0.5
    }
}

// ---------------------------------------------------------------------------
// Shapes, poses and colliders
// ---------------------------------------------------------------------------

/// A 2-D pose: translation plus rotation in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    /// World position.
    pub pos: Vec2,
    /// World rotation in radians (positive = counter-clockwise).
    pub rot: f32,
}

impl Pose {
    /// Creates a pose.
    pub fn new(pos: Vec2, rot: f32) -> Self {
        Self { pos, rot }
    }

    /// Transforms a local point into world space.
    pub fn point(&self, local: Vec2) -> Vec2 {
        self.pos + local.rotate(self.rot)
    }

    /// Local +Y axis in world space.
    pub fn up(&self) -> Vec2 {
        Vec2::Y.rotate(self.rot)
    }

    /// Local +X axis in world space.
    pub fn right(&self) -> Vec2 {
        Vec2::X.rotate(self.rot)
    }
}

/// Collision shape in local units (callers pre-scale sizes by the transform
/// scale: body `d1Size`, pellet `sqrt(amount/20)`, egg `D1SizeAtBirth`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Circle (pellets, eggs).
    Circle {
        /// Radius.
        radius: f32,
    },
    /// Capsule (bibite body/tail), axis along local +Y.
    Capsule {
        /// Half length of the centre segment (excluding caps).
        half_height: f32,
        /// Cap radius.
        radius: f32,
    },
    /// Oriented box (mouth trigger; static zone/bounds boxes).
    Box {
        /// Half extents on local x/y.
        half_extents: Vec2,
    },
}

impl Shape {
    /// Builds a circle.
    pub fn circle(radius: f32) -> Self {
        Self::Circle { radius }
    }

    /// Builds a vertical capsule from a Unity `CapsuleCollider2D` size:
    /// `radius = size.x/2`, `half_height = max(size.y/2 - radius, 0)`.
    pub fn capsule_from_size(size: Vec2) -> Self {
        let radius = size.x * 0.5;
        Self::Capsule {
            half_height: (size.y * 0.5 - radius).max(0.0),
            radius,
        }
    }

    /// Builds an oriented box from a Unity `BoxCollider2D` size.
    pub fn box_from_size(size: Vec2) -> Self {
        Self::Box {
            half_extents: size * 0.5,
        }
    }

    /// Radius of a circle that encloses the shape.
    pub fn bounding_radius(&self) -> f32 {
        match *self {
            Self::Circle { radius } => radius,
            Self::Capsule {
                half_height,
                radius,
            } => half_height + radius,
            Self::Box { half_extents } => half_extents.length(),
        }
    }

    /// World-space AABB for a pose.
    pub fn aabb(&self, pose: Pose) -> Aabb {
        match *self {
            Self::Circle { radius } => Aabb::around(pose.pos, radius),
            Self::Capsule {
                half_height,
                radius,
            } => {
                let axis = pose.up() * half_height;
                let a = pose.pos + axis;
                let b = pose.pos - axis;
                let r = Vec2::splat(radius);
                Aabb {
                    min: a.min(b) - r,
                    max: a.max(b) + r,
                }
            }
            Self::Box { half_extents } => {
                let corners = [
                    pose.point(Vec2::new(-half_extents.x, -half_extents.y)),
                    pose.point(Vec2::new(half_extents.x, -half_extents.y)),
                    pose.point(Vec2::new(half_extents.x, half_extents.y)),
                    pose.point(Vec2::new(-half_extents.x, half_extents.y)),
                ];
                let mut min = corners[0];
                let mut max = corners[0];
                for corner in &corners[1..] {
                    min = min.min(*corner);
                    max = max.max(*corner);
                }
                Aabb { min, max }
            }
        }
    }
}

/// One collider attached to a body (local offset and rotation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collider {
    /// Local shape.
    pub shape: Shape,
    /// Offset in the body's local frame.
    pub offset: Vec2,
    /// Rotation relative to the body, radians.
    pub rotation: f32,
    /// True for triggers/sensors: overlaps are reported but never resolved.
    pub is_sensor: bool,
    /// Friction coefficient (captured physics material).
    pub friction: f32,
    /// Restitution/bounciness (captured physics material).
    pub restitution: f32,
}

impl Collider {
    /// Solid collider with the captured `corps` defaults.
    pub fn solid(shape: Shape) -> Self {
        Self {
            shape,
            offset: Vec2::ZERO,
            rotation: 0.0,
            is_sensor: false,
            friction: prefab::CORPS_FRICTION,
            restitution: prefab::CORPS_RESTITUTION,
        }
    }

    /// Sensor/trigger collider.
    pub fn sensor(shape: Shape) -> Self {
        Self {
            shape,
            offset: Vec2::ZERO,
            rotation: 0.0,
            is_sensor: true,
            friction: prefab::CORPS_FRICTION,
            restitution: prefab::CORPS_RESTITUTION,
        }
    }

    /// Sets the friction/restitution pair (builder style).
    pub fn with_material(mut self, friction: f32, restitution: f32) -> Self {
        self.friction = friction;
        self.restitution = restitution;
        self
    }

    /// Sets the local offset (builder style).
    pub fn with_offset(mut self, offset: Vec2) -> Self {
        self.offset = offset;
        self
    }

    /// Sets the local rotation (builder style).
    pub fn with_rotation(mut self, rotation: f32) -> Self {
        self.rotation = rotation;
        self
    }

    /// World pose of this collider for a body pose.
    #[inline]
    pub fn world_pose(&self, body_pose: Pose) -> Pose {
        Pose {
            pos: body_pose.point(self.offset),
            rot: body_pose.rot + self.rotation,
        }
    }

    /// World-space AABB.
    #[inline]
    pub fn aabb(&self, body_pose: Pose) -> Aabb {
        let tight = self.shape.aabb(self.world_pose(body_pose));
        Aabb { min: tight.min - Vec2::splat(AABB_MARGIN), max: tight.max + Vec2::splat(AABB_MARGIN) }
    }
}

// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

/// Maximum number of colliders on one physics body (a bibite needs 3: Body,
/// Tail, Mouth trigger; pellets/eggs need 1).
pub const MAX_COLLIDERS: usize = 3;

/// A rigid body in the pure solver. B2 fills this from the stores each tick
/// (or keeps a mirror array) and writes poses/velocities back afterwards.
///
/// The fields are laid out in the order they are declared and the body starts on a cache line: the pose, the velocities, the mass
/// and the damping of the first 60 bytes are what a bibite's body is given again every tick, the colliders behind them only when it
/// grows (see `BibiteStore::step_physics`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C, align(64))]
pub struct PhysicsBody {
    /// Caller-defined identity (e.g. packed entity kind + arena index).
    pub tag: u64,
    /// World position.
    pub pos: Vec2,
    /// World rotation, radians.
    pub rot: f32,
    /// Linear velocity (u/s).
    pub vel: Vec2,
    /// Angular velocity (rad/s).
    pub ang_vel: f32,
    /// Mass (g). Zero for static bodies.
    pub mass: f32,
    /// Rotational inertia (mass·u²).
    pub inertia: f32,
    /// `1/mass` (0 when static).
    pub inv_mass: f32,
    /// `1/inertia` (0 when static or infinite inertia).
    pub inv_inertia: f32,
    /// Linear damping (1/s, captured prefab per-entity value).
    pub linear_damping: f32,
    /// Angular damping (1/s).
    pub angular_damping: f32,
    /// Captured Unity layer id (see [`LAYER_NAMES`]).
    pub layer: u8,
    /// Whether the body is simulated at all (inactive bodies are skipped).
    pub active: bool,
    /// Whether the body is static (no integration, infinite mass).
    pub is_static: bool,
    /// The force and torque the body's own muscles push with this step (`Rigidbody2D.AddForce`, `AddTorque`): the step integrates them with
    /// the damping, a slice at a time.
    pub force: Vec2,
    /// See `force`.
    pub torque: f32,
    /// Tag of the body this one is held by (0: none). The game joins what a bibite holds with a `FixedJoint2D`, whose bodies do
    /// not collide: the solid contacts of the pair are skipped (the sensors still see each other).
    pub attached_to: u64,
    /// How many more bodies this one's mouth would grab this tick (0: it does not want to grab, or its ten links are taken).
    pub grab_slots: u8,
    /// How far the body's box in the broadphase reaches beyond its colliders (a mouth that wants to grab must meet what it moves into during the tick).
    pub box_margin: f32,
    /// Whether a mouth may grab this body (`GrabbableObject`: pellets and bibites).
    pub grabbable: bool,
    colliders: [Collider; MAX_COLLIDERS],
    collider_count: usize,
}

impl PhysicsBody {
    /// Creates a static body (infinite mass) with no colliders.
    pub fn new_static(tag: u64, layer: u8) -> Self {
        Self {
            tag,
            pos: Vec2::ZERO,
            rot: 0.0,
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            mass: 0.0,
            inertia: 0.0,
            inv_mass: 0.0,
            inv_inertia: 0.0,
            linear_damping: prefab::LINEAR_DAMPING,
            angular_damping: prefab::BIBITE_ANGULAR_DAMPING,
            layer,
            active: true,
            is_static: true,
            force: Vec2::ZERO,
            torque: 0.0,
            attached_to: 0,
            grab_slots: 0,
            box_margin: 0.0,
            grabbable: false,
            colliders: [Collider::solid(Shape::circle(0.0)); MAX_COLLIDERS],
            collider_count: 0,
        }
    }

    /// Creates a dynamic body. `mass`/`inertia` must be finite and positive;
    /// non-positive values produce zero inverse mass (Box2D's massless-body
    /// behaviour).
    pub fn new_dynamic(tag: u64, layer: u8, mass: f32, inertia: f32) -> Self {
        let mut body = Self::new_static(tag, layer);
        body.is_static = false;
        body.set_mass(mass, inertia);
        body
    }

    /// Updates mass/inertia and their inverses.
    pub fn set_mass(&mut self, mass: f32, inertia: f32) {
        self.mass = mass;
        self.inertia = inertia;
        self.inv_mass = if mass > 0.0 && mass.is_finite() {
            1.0 / mass
        } else {
            0.0
        };
        self.inv_inertia = if inertia > 0.0 && inertia.is_finite() {
            1.0 / inertia
        } else {
            0.0
        };
    }

    /// The body's box for the broadphase: the colliders' box and the margin the body asks for.
    pub fn swept_aabb(&self) -> Option<Aabb> {
        let aabb = self.aabb()?;
        if self.box_margin > 0.0 {
            let margin = Vec2::splat(self.box_margin);
            Some(Aabb { min: aabb.min - margin, max: aabb.max + margin })
        } else {
            Some(aabb)
        }
    }

    /// Pose of the body.
    #[inline]
    pub fn pose(&self) -> Pose {
        Pose::new(self.pos, self.rot)
    }

    /// Adds a collider if capacity remains.
    pub fn add_collider(&mut self, collider: Collider) -> bool {
        if self.collider_count >= MAX_COLLIDERS {
            return false;
        }
        self.colliders[self.collider_count] = collider;
        self.collider_count += 1;
        true
    }

    /// Builder-style collider insertion (ignores overflow).
    pub fn with_collider(mut self, collider: Collider) -> Self {
        let _ = self.add_collider(collider);
        self
    }

    /// Number of attached colliders.
    pub fn collider_count(&self) -> usize {
        self.collider_count
    }

    /// Attached colliders.
    #[inline]
    pub fn colliders(&self) -> &[Collider] {
        &self.colliders[..self.collider_count]
    }

    /// Mutable attached colliders.
    pub fn colliders_mut(&mut self) -> &mut [Collider] {
        &mut self.colliders[..self.collider_count]
    }

    /// World AABB of all colliders, or `None` for a collider-less body.
    ///
    /// Computes the body rotation's `sin/cos` once and rotates each collider's
    /// local half extents analytically (hot path in the broadphase).
    pub fn aabb(&self) -> Option<Aabb> {
        let colliders = self.colliders();
        if colliders.is_empty() {
            return None;
        }
        // A pellet or an egg: one circle on the body's origin. Its box does not depend on the rotation, so the `sin/cos` pair is not
        // worked out (it is most of what a box of this kind costs, and the resting-pellet layer and the step ask for the boxes of
        // tens of thousands of them). What the general path below gives for this body is `pos + (cos * 0 - sin * 0)`, which is
        // the position again (the zero it adds only changes the sign of a zero position, and the radius that is taken off or
        // added next, a positive one, leaves nothing of that), so the two boxes are the same bits. The conditions keep the cases
        // where they would not be (a position or rotation that is not a number, a radius that is not a positive number) on the
        // general path.
        if let [only] = colliders
            && let Shape::Circle { radius } = only.shape
            && only.offset.x == 0.0
            && only.offset.y == 0.0
            && radius > 0.0
            && radius.is_finite()
            && self.pos.x.is_finite()
            && self.pos.y.is_finite()
            && self.rot.is_finite()
        {
            let extent = Vec2::splat(radius);
            let min = (self.pos - extent) - Vec2::splat(AABB_MARGIN);
            let max = (self.pos + extent) + Vec2::splat(AABB_MARGIN);
            return Some(self.swept_by_speed(min, max));
        }
        self.aabb_general()
    }

    /// [`Self::aabb`] for any body, with no shortcut (the reference the shortcut is compared with).
    fn aabb_general(&self) -> Option<Aabb> {
        let colliders = self.colliders();
        if colliders.is_empty() {
            return None;
        }
        let (sin, cos) = self.rot.sin_cos();
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for collider in colliders {
            let (sin_c, cos_c) = if collider.rotation == 0.0 {
                (0.0, 1.0)
            } else {
                collider.rotation.sin_cos()
            };
            let total_cos = cos * cos_c - sin * sin_c;
            let total_sin = sin * cos_c + cos * sin_c;
            // (The collider's offset is in the body's frame: only the body's rotation turns it.)
            let offset = Vec2::new(
                self.pos.x + cos * collider.offset.x - sin * collider.offset.y,
                self.pos.y + sin * collider.offset.x + cos * collider.offset.y,
            );
            let (ex, ey) = match collider.shape {
                Shape::Circle { radius } => (radius, radius),
                Shape::Capsule {
                    half_height,
                    radius,
                } => (
                    radius + half_height * total_sin.abs(),
                    radius + half_height * total_cos.abs(),
                ),
                Shape::Box { half_extents } => (
                    half_extents.x * total_cos.abs() + half_extents.y * total_sin.abs(),
                    half_extents.x * total_sin.abs() + half_extents.y * total_cos.abs(),
                ),
            };
            let extent = Vec2::new(ex, ey);
            min = min.min(offset - extent);
            max = max.max(offset + extent);
        }
        Some(self.swept_by_speed(min - Vec2::splat(AABB_MARGIN), max + Vec2::splat(AABB_MARGIN)))
    }

    /// The box `min`..`max` (the colliders' box with its margin), grown to cover where the step takes a fast body.
    #[inline]
    fn swept_by_speed(&self, mut min: Vec2, mut max: Vec2) -> Aabb {
        // A fast body also covers where the step takes it: the contact it is about to make is found before the shapes touch.
        if self.vel.length_sq() > FAST_SPEED * FAST_SPEED {
            let sweep = self.vel * SWEEP_TIME;
            min = min.min(min + sweep);
            max = max.max(max + sweep);
        }
        Aabb { min, max }
    }
}

/// The speed above which a body's box is swept along its velocity, and the time it is swept for (a step's worth at the slowest tick rate).
const FAST_SPEED: f32 = 50.0;
const SWEEP_TIME: f32 = 0.07;

/// How far the boxes of the broadphase are grown: a contact within the speculative distance must be found before the shapes touch.
pub const AABB_MARGIN: f32 = 0.06;

// ---------------------------------------------------------------------------
// Manifolds and narrowphase
// ---------------------------------------------------------------------------

/// Maximum number of points stored in a [`Manifold`]. P1 always emits one.
pub const MAX_MANIFOLD_POINTS: usize = 2;

/// Hard cap on broadphase candidate collider pairs per step.
///
/// The uniform grid keeps the sparse case near-linear; this cap only protects
/// pathological inputs (e.g. thousands of fully coincident bodies) from
/// exhausting memory. Excess candidates are dropped deterministically after
/// entry-index sorting.
pub const MAX_BROADPHASE_PAIRS: usize = 2_000_000;

/// Maximum number of grid cells; cell size is doubled until the grid fits.
const MAX_GRID_CELLS: usize = 1 << 18;

/// One contact point of a manifold.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ManifoldPoint {
    /// World-space contact point.
    pub point: Vec2,
    /// Signed separation along the manifold normal. Negative means overlap.
    pub separation: f32,
}

/// Collision manifold. The normal points from body **A** towards body **B**
/// (push B along `+normal`, A along `-normal`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manifold {
    /// Unit normal from A to B.
    pub normal: Vec2,
    /// Contact points (one, or two where flat sides meet).
    pub points: [ManifoldPoint; MAX_MANIFOLD_POINTS],
    /// Number of valid points.
    pub count: u8,
}

impl Default for Manifold {
    fn default() -> Self {
        Self {
            normal: Vec2::Y,
            points: [ManifoldPoint::default(); MAX_MANIFOLD_POINTS],
            count: 0,
        }
    }
}

impl Manifold {
    fn single(normal: Vec2, point: Vec2, separation: f32) -> Self {
        let mut points = [ManifoldPoint::default(); MAX_MANIFOLD_POINTS];
        points[0] = ManifoldPoint { point, separation };
        Self {
            normal,
            points,
            count: 1,
        }
    }

    /// A manifold with two points (a flat face against a flat face: the two ends of the overlap).
    fn pair(normal: Vec2, first: ManifoldPoint, second: ManifoldPoint) -> Self {
        let mut points = [ManifoldPoint::default(); MAX_MANIFOLD_POINTS];
        points[0] = first;
        points[1] = second;
        Self {
            normal,
            points,
            count: 2,
        }
    }

    fn flipped(self) -> Self {
        let mut flipped = self;
        flipped.normal = -flipped.normal;
        flipped
    }
}

/// Closest point on segment `[a, b]` to point `p`.
fn closest_point_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq <= f32::EPSILON {
        return a;
    }
    let t = clamp((p - a).dot(ab) / len_sq, 0.0, 1.0);
    a + ab * t
}

/// Closest points between two segments.
fn closest_points_segments(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> (Vec2, Vec2) {
    // Projection candidates plus a deterministic fallback: cold-path
    // capsule-vs-capsule is dominated by endpoint projections in this game's
    // shapes (short, fat capsules); accuracy is validated by the analytic
    // tests. This is allocation- and loop-free.
    let mut best_a = a0;
    let mut best_b = closest_point_on_segment(a0, b0, b1);
    let mut best_d = best_a.distance_sq(best_b);
    for (pa, pb) in [
        (a1, closest_point_on_segment(a1, b0, b1)),
        (closest_point_on_segment(b0, a0, a1), b0),
        (closest_point_on_segment(b1, a0, a1), b1),
    ] {
        let d = pa.distance_sq(pb);
        if d < best_d {
            best_d = d;
            best_a = pa;
            best_b = pb;
        }
    }
    (best_a, best_b)
}

fn circle_circle(radius_a: f32, center_a: Vec2, radius_b: f32, center_b: Vec2) -> Option<Manifold> {
    let delta = center_b - center_a;
    let distance_sq = delta.length_sq();
    let radius_sum = radius_a + radius_b;
    if distance_sq > radius_sum * radius_sum {
        return None;
    }
    let distance = distance_sq.sqrt();
    let normal = if distance > f32::EPSILON {
        delta / distance
    } else {
        Vec2::Y
    };
    let point_a = center_a + normal * radius_a;
    let point_b = center_b - normal * radius_b;
    Some(Manifold::single(
        normal,
        (point_a + point_b) * 0.5,
        distance - radius_sum,
    ))
}

fn circle_capsule(
    circle_radius: f32,
    circle_center: Vec2,
    half_height: f32,
    radius: f32,
    capsule_pose: Pose,
) -> Option<Manifold> {
    let axis = capsule_pose.up() * half_height;
    let a = capsule_pose.pos + axis;
    let b = capsule_pose.pos - axis;
    let closest = closest_point_on_segment(circle_center, a, b);
    let delta = closest - circle_center;
    let distance_sq = delta.length_sq();
    let radius_sum = circle_radius + radius;
    if distance_sq > radius_sum * radius_sum {
        return None;
    }
    let distance = distance_sq.sqrt();
    let normal = if distance > f32::EPSILON {
        delta / distance
    } else {
        Vec2::Y
    };
    let point_circle = circle_center + normal * circle_radius;
    let point_capsule = closest - normal * radius;
    Some(Manifold::single(
        normal,
        (point_circle + point_capsule) * 0.5,
        distance - radius_sum,
    ))
}

fn capsule_capsule(
    half_a: f32,
    radius_a: f32,
    pose_a: Pose,
    half_b: f32,
    radius_b: f32,
    pose_b: Pose,
) -> Option<Manifold> {
    let axis_a = pose_a.up() * half_a;
    let axis_b = pose_b.up() * half_b;
    let (pa, pb) = closest_points_segments(
        pose_a.pos + axis_a,
        pose_a.pos - axis_a,
        pose_b.pos + axis_b,
        pose_b.pos - axis_b,
    );
    let delta = pb - pa;
    let distance_sq = delta.length_sq();
    let radius_sum = radius_a + radius_b;
    if distance_sq > radius_sum * radius_sum {
        return None;
    }
    // Capsules lying side by side touch along their flat sides: the two ends of the overlap are the manifold's two points (a polygon pair's
    // manifold), so that the contact pushes without turning them.
    let (up_a, up_b) = (pose_a.up(), pose_b.up());
    if half_a > 0.0 && half_b > 0.0 && up_a.cross(up_b).abs() < 0.12 {
        let b_top = pose_b.pos + axis_b;
        let b_bottom = pose_b.pos - axis_b;
        let (t_top, t_bottom) = ((b_top - pose_a.pos).dot(up_a), (b_bottom - pose_a.pos).dot(up_a));
        let low = (-half_a).max(t_top.min(t_bottom));
        let high = half_a.min(t_top.max(t_bottom));
        if high - low > 0.1 * half_a.min(half_b) {
            let mut normal = up_a.perp();
            if normal.dot(pose_b.pos - pose_a.pos) < 0.0 {
                normal = -normal;
            }
            let end = |t: f32| {
                let on_a = pose_a.pos + up_a * t;
                let on_b = closest_point_on_segment(on_a, b_bottom, b_top);
                let separation = (on_b - on_a).dot(normal) - radius_sum;
                ManifoldPoint {
                    point: ((on_a + normal * radius_a) + (on_b - normal * radius_b)) * 0.5,
                    separation,
                }
            };
            return Some(Manifold::pair(normal, end(low), end(high)));
        }
    }
    let distance = distance_sq.sqrt();
    let normal = if distance > f32::EPSILON {
        delta / distance
    } else {
        let fallback = pose_b.pos - pose_a.pos;
        if fallback.length_sq() > f32::EPSILON {
            fallback.normalized()
        } else {
            Vec2::Y
        }
    };
    let point_a = pa + normal * radius_a;
    let point_b = pb - normal * radius_b;
    Some(Manifold::single(
        normal,
        (point_a + point_b) * 0.5,
        distance - radius_sum,
    ))
}

/// Signed distance from a point (box-local) to the box boundary and an
/// outward unit normal (box-local).
fn box_signed_distance(local: Vec2, half_extents: Vec2) -> (f32, Vec2) {
    let d = Vec2::new(
        local.x.abs() - half_extents.x,
        local.y.abs() - half_extents.y,
    );
    if d.x > 0.0 || d.y > 0.0 {
        let positive = Vec2::new(d.x.max(0.0), d.y.max(0.0));
        let distance = positive.length();
        let normal = if distance > f32::EPSILON {
            Vec2::new(positive.x * local.x.signum(), positive.y * local.y.signum()) / distance
        } else {
            Vec2::new(local.x.signum(), 0.0)
        };
        (distance, normal)
    } else if half_extents.x - local.x.abs() < half_extents.y - local.y.abs() {
        (d.x.max(d.y), Vec2::new(local.x.signum(), 0.0))
    } else {
        (d.x.max(d.y), Vec2::new(0.0, local.y.signum()))
    }
}

fn circle_box(
    circle_radius: f32,
    circle_center: Vec2,
    half_extents: Vec2,
    box_pose: Pose,
) -> Option<Manifold> {
    let local = (circle_center - box_pose.pos).rotate(-box_pose.rot);
    let (distance, normal_local) = box_signed_distance(local, half_extents);
    let separation = distance - circle_radius;
    if separation > 0.0 {
        return None;
    }
    let normal_world = normal_local.rotate(box_pose.rot);
    let clamped = Vec2::new(
        clamp(local.x, -half_extents.x, half_extents.x),
        clamp(local.y, -half_extents.y, half_extents.y),
    );
    let box_point = box_pose.point(clamped);
    let circle_point = circle_center - normal_world * circle_radius;
    Some(Manifold::single(
        normal_world,
        (box_point + circle_point) * 0.5,
        separation,
    ))
}

fn segment_intersects_aabb(a: Vec2, b: Vec2, half_extents: Vec2) -> bool {
    let delta = b - a;
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    for (origin, direction, half) in [
        (a.x, delta.x, half_extents.x),
        (a.y, delta.y, half_extents.y),
    ] {
        if direction.abs() < f32::EPSILON {
            if origin < -half || origin > half {
                return false;
            }
            continue;
        }
        let mut near = (-half - origin) / direction;
        let mut far = (half - origin) / direction;
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// Closed-form closest pair between a segment and an axis-aligned box at the
/// origin (the capsule is transformed into box-local space first).
///
/// For disjoint features the minimum is at an endpoint projection or a
/// segment-to-edge projection (4 edges × 4 projections); when the segment
/// overlaps the box the pair is a face-normal contact at the segment midpoint.
/// This replaces the previous 32-step ternary search (a dense-cluster hotspot)
/// while keeping the analytic separations for exterior contacts.
fn closest_segment_aabb(a: Vec2, b: Vec2, half_extents: Vec2) -> (Vec2, Vec2, f32, bool) {
    if segment_intersects_aabb(a, b, half_extents) {
        let midpoint = (a + b) * 0.5;
        let (separation, normal) = box_signed_distance(midpoint, half_extents);
        return (midpoint, midpoint, separation, true).clone_with_normal(normal);
    }
    let mut best_a = a;
    let mut best_b = Vec2::new(
        clamp(a.x, -half_extents.x, half_extents.x),
        clamp(a.y, -half_extents.y, half_extents.y),
    );
    let mut best_d = best_a.distance_sq(best_b);
    let endpoint_b = Vec2::new(
        clamp(b.x, -half_extents.x, half_extents.x),
        clamp(b.y, -half_extents.y, half_extents.y),
    );
    let d = b.distance_sq(endpoint_b);
    if d < best_d {
        best_d = d;
        best_a = b;
        best_b = endpoint_b;
    }
    let corners = [
        Vec2::new(-half_extents.x, -half_extents.y),
        Vec2::new(half_extents.x, -half_extents.y),
        Vec2::new(half_extents.x, half_extents.y),
        Vec2::new(-half_extents.x, half_extents.y),
    ];
    for index in 0..4 {
        let (pa, pb) = closest_points_segments(a, b, corners[index], corners[(index + 1) % 4]);
        let distance = pa.distance_sq(pb);
        if distance < best_d {
            best_d = distance;
            best_a = pa;
            best_b = pb;
        }
    }
    (best_a, best_b, best_d, false)
}

/// Small helper so the inside-box branch can carry its local face normal out
/// of [`closest_segment_aabb`] without an extra tuple field.
trait CloneWithNormal {
    fn clone_with_normal(self, normal: Vec2) -> (Vec2, Vec2, f32, bool);
}

impl CloneWithNormal for (Vec2, Vec2, f32, bool) {
    fn clone_with_normal(self, _normal: Vec2) -> (Vec2, Vec2, f32, bool) {
        self
    }
}

fn capsule_box(
    half_height: f32,
    radius: f32,
    capsule_pose: Pose,
    half_extents: Vec2,
    box_pose: Pose,
) -> Option<Manifold> {
    let axis = capsule_pose.up() * half_height;
    let a_local = (capsule_pose.pos + axis - box_pose.pos).rotate(-box_pose.rot);
    let b_local = (capsule_pose.pos - axis - box_pose.pos).rotate(-box_pose.rot);
    let inside = segment_intersects_aabb(a_local, b_local, half_extents);
    let (segment_local, box_local, distance_sq, inside_segment) =
        closest_segment_aabb(a_local, b_local, half_extents);
    let distance = distance_sq.sqrt();
    let separation = distance - radius;
    if !inside_segment && separation > 1e-4 {
        return None;
    }
    let normal_local = if inside_segment || distance <= f32::EPSILON {
        // Deep overlap: use the nearest box face from the segment midpoint.
        let midpoint = (a_local + b_local) * 0.5;
        box_signed_distance(midpoint, half_extents).1
    } else {
        (segment_local - box_local) / distance
    };
    let normal_world = normal_local.rotate(box_pose.rot);
    let box_point = box_pose.point(box_local);
    let capsule_point = box_pose.point(segment_local) - normal_world * radius;
    let separation = if inside_segment {
        separation
    } else {
        separation.max(-radius)
    };
    let separation = if inside {
        separation.min(0.0)
    } else {
        separation
    };
    Some(Manifold::single(
        normal_world,
        (box_point + capsule_point) * 0.5,
        separation,
    ))
}

/// Narrowphase dispatch. The returned normal points from A to B.
///
/// Box-box pairs are not modelled (P1 does not need them); `None` is returned.
pub fn collide(a: &Shape, pose_a: Pose, b: &Shape, pose_b: Pose) -> Option<Manifold> {
    match (*a, *b) {
        (Shape::Circle { radius: ra }, Shape::Circle { radius: rb }) => {
            circle_circle(ra, pose_a.pos, rb, pose_b.pos)
        }
        (
            Shape::Circle { radius },
            Shape::Capsule {
                half_height,
                radius: capsule_radius,
            },
        ) => circle_capsule(radius, pose_a.pos, half_height, capsule_radius, pose_b),
        (
            Shape::Capsule {
                half_height,
                radius: capsule_radius,
            },
            Shape::Circle { radius },
        ) => circle_capsule(radius, pose_b.pos, half_height, capsule_radius, pose_a)
            .map(Manifold::flipped),
        (
            Shape::Capsule {
                half_height: ha,
                radius: ra,
            },
            Shape::Capsule {
                half_height: hb,
                radius: rb,
            },
        ) => capsule_capsule(ha, ra, pose_a, hb, rb, pose_b),
        (Shape::Circle { radius }, Shape::Box { half_extents }) => {
            circle_box(radius, pose_a.pos, half_extents, pose_b).map(Manifold::flipped)
        }
        (Shape::Box { half_extents }, Shape::Circle { radius }) => {
            circle_box(radius, pose_b.pos, half_extents, pose_a)
        }
        (
            Shape::Capsule {
                half_height,
                radius,
            },
            Shape::Box { half_extents },
        ) => capsule_box(half_height, radius, pose_a, half_extents, pose_b).map(Manifold::flipped),
        (
            Shape::Box { half_extents },
            Shape::Capsule {
                half_height,
                radius,
            },
        ) => capsule_box(half_height, radius, pose_b, half_extents, pose_a),
        (Shape::Box { .. }, Shape::Box { .. }) => None,
    }
}

/// Collision with a contact-skin radius added to every shape's radius
/// (`Physics2D.defaultContactOffset`). This lets the solver generate contacts
/// for small gaps, like Box2D's polygon skin.
pub fn collide_with_skin(
    a: &Shape,
    pose_a: Pose,
    b: &Shape,
    pose_b: Pose,
    skin: f32,
) -> Option<Manifold> {
    let grown_a = grow_shape(a, skin);
    let grown_b = grow_shape(b, skin);
    collide(&grown_a, pose_a, &grown_b, pose_b)
}

fn grow_shape(shape: &Shape, skin: f32) -> Shape {
    if skin <= 0.0 {
        return *shape;
    }
    match *shape {
        Shape::Circle { radius } => Shape::Circle {
            radius: radius + skin,
        },
        Shape::Capsule {
            half_height,
            radius,
        } => Shape::Capsule {
            half_height,
            radius: radius + skin,
        },
        Shape::Box { half_extents } => Shape::Box {
            half_extents: half_extents + Vec2::splat(skin),
        },
    }
}

// ---------------------------------------------------------------------------
// Contact / trigger events
// ---------------------------------------------------------------------------

/// A resolved contact between two bodies, exposed after [`crate::physics::step`] for body/world
/// logic (collision damage, bite knockback, logging).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactEvent {
    /// Index of body A in the slice passed to [`crate::physics::step`].
    pub body_a: usize,
    /// Index of body B.
    pub body_b: usize,
    /// Tag of body A.
    pub tag_a: u64,
    /// Tag of body B.
    pub tag_b: u64,
    /// Collider index on body A.
    pub collider_a: u8,
    /// Collider index on body B.
    pub collider_b: u8,
    /// Unit normal from A to B.
    pub normal: Vec2,
    /// World contact point.
    pub point: Vec2,
    /// Initial signed separation (negative = overlap).
    pub separation: f32,
    /// Relative velocity at the contact point **before** solving (this is what
    /// the game's `Collision2D.relativeVelocity` exposes to damage code).
    pub relative_velocity: Vec2,
    /// Accumulated normal impulse.
    pub normal_impulse: f32,
    /// Accumulated tangent (friction) impulse.
    pub tangent_impulse: f32,
    /// Friction coefficient used.
    pub friction: f32,
    /// Restitution coefficient used.
    pub restitution: f32,
}

/// A sensor/trigger overlap between two bodies, exposed after [`crate::physics::step`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TriggerEvent {
    /// Index of body A in the slice passed to [`crate::physics::step`].
    pub body_a: usize,
    /// Index of body B.
    pub body_b: usize,
    /// Tag of body A.
    pub tag_a: u64,
    /// Tag of body B.
    pub tag_b: u64,
    /// Collider A is the sensor/trigger.
    pub sensor_a: bool,
    /// Collider B is the sensor/trigger.
    pub sensor_b: bool,
    /// Unit normal from A to B.
    pub normal: Vec2,
    /// World contact point.
    pub point: Vec2,
    /// Signed separation (negative = overlap).
    pub separation: f32,
}

// ---------------------------------------------------------------------------
// Scratch buffers
// ---------------------------------------------------------------------------

/// Per-body broadphase entry (merged AABB over all colliders).
#[derive(Clone, Copy, Debug, PartialEq)]
struct BodyEntry {
    body: u32,
    tag: u64,
    layer: u8,
    aabb: Aabb,
}

/// Candidate body pair (colliders are tested in narrowphase). The bodies' tags travel with it: the pairs are put in order
/// by tag, and asking 100,000 scattered bodies for them one by one is a cache miss each.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BodyPair {
    body_a: u32,
    body_b: u32,
    tag_a: u64,
    tag_b: u64,
}

/// The boxes of the broadphase entries by field: the sweep looks at the four numbers of every candidate in a body's x range
/// (about 180 of them at 100,000 bibites) and almost none is a hit, so it reads them from here, eight at a time, and only a hit
/// looks at the entry.
#[derive(Clone, Debug, Default)]
struct SweepColumns {
    min_x: Vec<f32>,
    max_x: Vec<f32>,
    min_y: Vec<f32>,
    max_y: Vec<f32>,
}

impl SweepColumns {
    /// Copies the boxes of `entries` (on the pool when there are two stretches of `min_parallel` entries: a worker for each, at
    /// most `threads`).
    fn fill(&mut self, entries: &[BodyEntry], threads: usize, min_parallel: usize) {
        let count = entries.len();
        for column in [
            &mut self.min_x,
            &mut self.max_x,
            &mut self.min_y,
            &mut self.max_y,
        ] {
            column.clear();
            column.resize(count, 0.0);
        }
        let workers = crate::sim::ParallelGates::workers_for(count, min_parallel, threads);
        if workers > 1 {
            let chunk_len = crate::sim::ParallelGates::chunk_len(count, min_parallel, workers, 1);
            rayon::scope(|scope| {
                for ((((chunk, min_x), max_x), min_y), max_y) in entries
                    .chunks(chunk_len)
                    .zip(self.min_x.chunks_mut(chunk_len))
                    .zip(self.max_x.chunks_mut(chunk_len))
                    .zip(self.min_y.chunks_mut(chunk_len))
                    .zip(self.max_y.chunks_mut(chunk_len))
                {
                    scope.spawn(move |_| {
                        for (at, entry) in chunk.iter().enumerate() {
                            min_x[at] = entry.aabb.min.x;
                            max_x[at] = entry.aabb.max.x;
                            min_y[at] = entry.aabb.min.y;
                            max_y[at] = entry.aabb.max.y;
                        }
                    });
                }
            });
        } else {
            for (at, entry) in entries.iter().enumerate() {
                self.min_x[at] = entry.aabb.min.x;
                self.max_x[at] = entry.aabb.max.x;
                self.min_y[at] = entry.aabb.min.y;
                self.max_y[at] = entry.aabb.max.y;
            }
        }
    }
}

/// What the broadphase needs of one body, gathered while its box is computed (the body is in the cache then).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BodyBox {
    pub(crate) aabb: Aabb,
    tag: u64,
    layer: u8,
    /// Whether the body takes part at all (active, with a collider).
    pub(crate) valid: bool,
}

impl BodyBox {
    /// The box of a body that takes no part.
    pub(crate) const INVALID: Self = Self {
        aabb: Aabb {
            min: Vec2::splat(f32::INFINITY),
            max: Vec2::splat(f32::NEG_INFINITY),
        },
        tag: 0,
        layer: 0,
        valid: false,
    };

    /// The box of a body, as the step computes it.
    pub(crate) fn of(body: &PhysicsBody) -> Self {
        if !body.active || body.collider_count() == 0 {
            return Self::INVALID;
        }
        match body.swept_aabb() {
            Some(aabb) => Self {
                aabb,
                tag: body.tag,
                layer: body.layer,
                valid: true,
            },
            None => Self::INVALID,
        }
    }
}

/// Solver-side copy of a body's dynamic state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SolverBody {
    pub(crate) pos: Vec2,
    pub(crate) rot: f32,
    pub(crate) vel: Vec2,
    pub(crate) ang_vel: f32,
    inv_mass: f32,
    inv_inertia: f32,
    linear_damping: f32,
    angular_damping: f32,
    force: Vec2,
    torque: f32,
    /// The body's centre of mass in its own frame (`Rigidbody2D.centerOfMass`, which Unity computes from the colliders): the solver works on the
    /// world position of the centre of mass during the step, and hands the body's own position back when it is done.
    com: Vec2,
    /// Where the step found the body (the substeps measure the contacts' motion from it) and whether a contact holds it this step.
    pos0: Vec2,
    rot0: f32,
    /// The rotation the tick began with (the muscles' force is given along it and turns with the body from there).
    rot_tick: f32,
    touched: bool,
    is_static: bool,
}

/// The game's physics splits a step's force, damping and motion into this many slices: measured against the original (a body flying free with damping
/// `c` keeps `(1 + h c / N)^-N` of its speed per step, N = 10, a body pushed from rest travels only the mean of its slices' speeds in the first
/// step), where a single `1 / (1 + h c)` step would lose a few percent more speed per step.
const DAMPING_SLICES: i32 = 10;

/// A body's centre of mass in its own frame: the average of its solid colliders' centres weighted by their area, as Unity computes it for a
/// `Rigidbody2D` whose mass is set by hand (the bibite's body and tail capsules lie behind the pivot, so it turns about a point behind it).
#[inline]
fn centre_of_mass(body: &PhysicsBody) -> Vec2 {
    let mut weight = 0.0f32;
    let mut sum = Vec2::ZERO;
    for collider in body.colliders() {
        if collider.is_sensor && !COM_COUNTS_SENSORS {
            continue;
        }
        let area = match collider.shape {
            Shape::Circle { radius } => std::f32::consts::PI * radius * radius,
            Shape::Capsule { half_height, radius } => 4.0 * half_height * radius + std::f32::consts::PI * radius * radius,
            Shape::Box { half_extents } => 4.0 * half_extents.x * half_extents.y,
        };
        weight += area;
        sum += collider.offset * area;
    }
    if weight > 0.0 { sum * (1.0 / weight) } else { Vec2::ZERO }
}

/// Whether trigger colliders (the bibite's mouth) take part in the centre of mass.
const COM_COUNTS_SENSORS: bool = false;

/// What the slices' recurrence `v = (v + h a) q`, `x += h v` (`q = 1 / (1 + h c)`, `h = dt / N`) comes to over a step for a damping `c`, in closed form:
/// `v' = keep v0 + gain s1`, `x' = h (v0 s1 + gain far)` with `gain = h a`. The same for every axis of a body.
#[derive(Clone, Copy)]
struct SliceTerms {
    h: f64,
    keep: f64,
    s1: f64,
    far: f64,
}

#[inline]
fn slice_terms(damping: f32, dt: f32, slices: i32) -> SliceTerms {
    let n = f64::from(slices);
    let h = f64::from(dt) / n;
    if damping <= 0.0 {
        return SliceTerms { h, keep: 1.0, s1: n, far: n * (n + 1.0) / 2.0 };
    }
    let q = 1.0 / (1.0 + h * f64::from(damping));
    let qn = q.powi(slices);
    let s1 = q * (1.0 - qn) / (1.0 - q);
    SliceTerms { h, keep: qn, s1, far: q * (n - s1) / (1.0 - q) }
}

/// One axis through a step: the final velocity and the displacement (see `SliceTerms`).
#[inline]
fn sliced_axis(terms: SliceTerms, v0: f32, accel: f32) -> (f32, f32) {
    let (v0, gain) = (f64::from(v0), f64::from(accel) * terms.h);
    let v = terms.keep * v0 + gain * terms.s1;
    let x = terms.h * (v0 * terms.s1 + gain * terms.far);
    (v as f32, x as f32)
}

/// One scalar contact constraint (one per manifold point), as `b2ContactConstraint` of the soft-step solver.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ContactConstraint {
    body_a: u32,
    body_b: u32,
    normal: Vec2,
    tangent: Vec2,
    /// The contact point relative to each body's centre, fixed for the step (`anchorA`, `anchorB`).
    anchor_a: Vec2,
    anchor_b: Vec2,
    /// The separation the step began with, less what the anchors' offset adds to it (`adjustedSeparation`); the substeps add the bodies' motion.
    adjusted_separation: f32,
    normal_mass: f32,
    tangent_mass: f32,
    /// The approach speed along the normal the step began with (the restitution reads it).
    relative_velocity: f32,
    restitution: f32,
    normal_impulse: f32,
    tangent_impulse: f32,
    /// What the substeps' normal impulses add up to (the step's impulse), and the greatest of them.
    total_normal_impulse: f32,
    max_normal_impulse: f32,
    friction: f32,
    event: u32,
    /// Whether the shapes are within the speculative distance or the tick's motion brings them to touching (a contact that is not near cannot touch
    /// during the tick: the bodies it ties do not need the fine steps).
    near: bool,
    /// Identity of the contact across steps (its colliders' bodies' tags and collider indices, smaller tag first) and whether
    /// this step saw the pair the other way round (the tangent impulse then changes sign).
    key: ContactKey,
    flipped: bool,
    /// The two bodies are welded to each other (a holder and what it holds): the weld keeps them where they are, so the contact pushes nothing out (a rigid
    /// joint against an unlimited push-out threw the pair about).
    welded: bool,
}

/// What names a contact from one step to the next (`b2ContactEdge` + the manifold point's id): the two bodies' tags, smaller first,
/// and the colliders' indices in that order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct ContactKey {
    tag_a: u64,
    tag_b: u64,
    collider_a: u8,
    collider_b: u8,
    point: u8,
}

/// The impulses a contact ended a step with (`b2ManifoldPoint::normalImpulse` / `tangentImpulse`), kept for warm starting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CarriedImpulse {
    key: ContactKey,
    normal: f32,
    tangent: f32,
}

/// One sensor collider used by the supplemental sensor broadphase.
#[derive(Clone, Copy, Debug)]
struct SensorEntry {
    body: u32,
    aabb: Aabb,
}

/// A weld between two bodies of the slice given to [`step`] (the game's `FixedJoint2D`, which a mouth puts between itself and what it grabs): the held
/// body keeps the pose it had relative to the holder when the joint was made. The joint gives way when its reaction force exceeds `break_force`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeldDef {
    /// Index of the held body.
    pub held: u32,
    /// Index of the holder.
    pub holder: u32,
    /// The held body's origin in the holder's own frame at the moment of the grab.
    pub anchor: Vec2,
    /// The held body's rotation relative to the holder's at the moment of the grab.
    pub angle: f32,
    /// `breakForce` and `breakTorque` (not positive: the joint never breaks).
    pub break_force: f32,
}

/// A weld during the fine steps: its bodies by place in the island, and the impulses it has applied (they warm start the next step).
#[derive(Clone, Copy, Debug)]
struct WeldState {
    holder: u32,
    held: u32,
    anchor: Vec2,
    angle: f32,
    break_force: f32,
    linear: Vec2,
    angular: f32,
    broken: bool,
    /// The held body's index in the step's slice and tag (the carried impulses are found by tag).
    held_global: u32,
    tag: u64,
    /// The holder's tag (a body may be held by several holders: a weld is named by both).
    holder_tag: u64,
}

/// A mouth and a body it may grab, by index in the slice given to [`step`] (`sensor`: the index of the mouth's trigger collider). `welded`: the
/// mouth holds it already (only its overlap is followed, for the mouth's list).
#[derive(Clone, Copy, Debug)]
struct GrabPair {
    holder: u32,
    target: u32,
    sensor: u8,
    welded: bool,
}

/// A [`GrabPair`] inside an island (bodies by place); `inside`: the mouth's trigger overlapped the body at the start of the last step.
#[derive(Clone, Copy, Debug)]
struct LocalGrab {
    holder: u32,
    target: u32,
    sensor: u8,
    welded: bool,
    inside: bool,
}

/// A grab made inside the steps of a tick (the game's `TryGrabTarget` in the `FixedUpdate` after the trigger was entered): the bodies by index in
/// the slice given to [`step`], the relative pose frozen at that moment.
#[derive(Clone, Copy, Debug)]
pub struct NewWeld {
    /// Index of the holder.
    pub holder: u32,
    /// Index of the held body.
    pub held: u32,
    /// The held body's origin in the holder's frame.
    pub anchor: Vec2,
    /// The held body's rotation relative to the holder's.
    pub angle: f32,
}

/// Reusable scratch buffers for [`crate::physics::step`]. Reuse one instance exclusively for
/// one simulation; it owns the solver-body mirror and contact/trigger lists.
#[derive(Clone, Debug, Default)]
pub struct PhysicsScratch {
    entries: Vec<BodyEntry>,
    pairs: Vec<BodyPair>,
    /// Uniform-grid cell heads (`u32::MAX` = empty); rebuilt every step.
    cell_heads: Vec<u32>,
    /// Grid linked-list node `next` pointers.
    cell_next: Vec<u32>,
    /// Grid linked-list node entry indices (parallel to `cell_next`).
    cell_node_entry: Vec<u32>,
    /// Supplemental sensor broadphase scratch (only used when the solid dense
    /// cap actually dropped pairs).
    sensor_entries: Vec<SensorEntry>,
    sensor_candidates: Vec<u32>,
    sensor_pair_keys: Vec<(u32, u32)>,
    /// Set by the dense broadphase when the per-body pair cap dropped pairs.
    dense_cap_hit: bool,
    /// Sorted/deduped `(entry_a << 32 | entry_b)` candidate keys.
    pair_keys: Vec<u64>,
    /// Every body's box, tag and layer, by body index.
    body_boxes: Vec<BodyBox>,
    /// The grid broadphase's lists of bodies by chunk of bodies and stripe of the world, and its stripes (see [`grid_broadphase`]).
    grid_lists: Vec<Vec<u32>>,
    grid_stripes: Vec<GridStripe>,
    /// Never ask the grid for the candidate pairs: the sweep (and its x order) does it, whatever the size of the world.
    pub sweep_only: bool,
    collider_aabbs: Vec<Aabb>,
    collider_poses: Vec<Pose>,
    /// Which bodies a candidate pair names (only theirs need collider poses).
    pair_bodies: Vec<bool>,
    body_order: Vec<u32>,
    /// The boxes' minimum x by body index (the serial x order's sort keys).
    order_keys: Vec<f32>,
    /// `(minimum x in total order << 32) | body index` for the bodies that take part: the pooled x order sorts these.
    sort_keys: Vec<u64>,
    /// The boxes of `entries` by field, for the sweep.
    sweep_columns: SweepColumns,
    /// How many of the first entries of `body_boxes` the caller already computed for the next step's first bodies.
    known_boxes: usize,
    /// How many of the next step's first bodies the step does not write back (the caller reads them from `solver`).
    skip_write_back: usize,
    solver: Vec<SolverBody>,
    constraints: Vec<ContactConstraint>,
    /// The impulses of the last step's contacts, by contact (warm starting), and the buffer the next list is built in.
    carried: Vec<CarriedImpulse>,
    carried_next: Vec<CarriedImpulse>,
    contacts: Vec<ContactEvent>,
    triggers: Vec<TriggerEvent>,
    /// The welds of the next step (see [`WeldDef`]; the step takes them).
    welds_in: Vec<WeldDef>,
    /// The impulses the welds ended the last step with, by the held body's tag (warm starting).
    joint_carried: Vec<(u64, u64, Vec2, f32)>,
    /// The welds that gave way in the last step: the held body (by index) and its holder's tag (a body can be held by several holders).
    broken_welds: Vec<(u32, u64)>,
    /// The grabs the last step made inside its steps.
    new_welds: Vec<NewWeld>,
    /// Workers of the pool the stages may use (0 or 1: everything serial); the caller sets it before each step.
    pub threads: usize,
    /// Bodies from which the cheap per-body stages use the pool (a worker for every quarter of it; see `ParallelGates`).
    pub min_bodies: usize,
    /// Bodies in the broadphase from which its sweep uses the pool.
    pub min_broadphase: usize,
    /// Bodies from which the candidate pairs are found by the grid (on a pool of enough workers) instead of the sweep; 0: never (see
    /// [`grid_broadphase_from`]).
    pub min_grid: usize,
    /// Candidate pairs from which the narrowphase uses the pool.
    pub min_narrow_pairs: usize,
    /// Bodies and boxes (candidate pairs: twice as many) from which the sorts and the column copies use the pool.
    pub min_sort: usize,
    /// Bodies held by near contacts from which the islands of the fine steps use the pool (with sixteen islands or more).
    pub min_island_bodies: usize,
    /// Contact constraints from which the islands of the soft step use the pool (with sixteen islands or more).
    pub min_island_constraints: usize,
    /// Wall time of the step's stages (observational only; the world drains it into its phase timings).
    pub clock: StageClock,
}

impl PhysicsScratch {
    /// Creates empty scratch buffers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Workers the per-body stages put to work for `bodies` bodies (1 = inline): none below `min_bodies` of them, then a worker for
    /// every quarter of that, at most `threads`. Every stage cuts by index and merges in order, so the results do not depend on it.
    fn effective_threads(&self, bodies: usize) -> usize {
        crate::sim::ParallelGates::workers_for(bodies, self.min_bodies, self.threads)
    }

    /// Creates scratch buffers with preallocated capacity.
    pub fn with_capacity(bodies: usize, colliders: usize, pairs: usize) -> Self {
        Self {
            entries: Vec::with_capacity(colliders),
            pairs: Vec::with_capacity(pairs),
            cell_heads: Vec::new(),
            cell_next: Vec::new(),
            cell_node_entry: Vec::new(),
            sensor_entries: Vec::new(),
            sensor_candidates: Vec::new(),
            sensor_pair_keys: Vec::new(),
            dense_cap_hit: false,
            pair_keys: Vec::new(),
            body_boxes: Vec::new(),
            grid_lists: Vec::new(),
            grid_stripes: Vec::new(),
            sweep_only: false,
            collider_aabbs: Vec::new(),
            collider_poses: Vec::new(),
            pair_bodies: Vec::new(),
            body_order: Vec::new(),
            order_keys: Vec::new(),
            sort_keys: Vec::new(),
            sweep_columns: SweepColumns::default(),
            known_boxes: 0,
            skip_write_back: 0,
            solver: Vec::with_capacity(bodies),
            constraints: Vec::with_capacity(pairs),
            carried: Vec::new(),
            carried_next: Vec::new(),
            contacts: Vec::with_capacity(pairs),
            triggers: Vec::with_capacity(pairs),
            threads: 0,
            min_bodies: 0,
            min_broadphase: 0,
            min_grid: 0,
            min_narrow_pairs: 0,
            min_sort: 0,
            min_island_bodies: 0,
            min_island_constraints: 0,
            clock: StageClock::default(),
            welds_in: Vec::new(),
            joint_carried: Vec::new(),
            broken_welds: Vec::new(),
            new_welds: Vec::new(),
        }
    }

    /// The vector the caller fills with the boxes of the next step's first bodies (see [`PhysicsScratch::boxes_ready`]).
    pub(crate) fn boxes_for_next_step(&mut self) -> &mut Vec<BodyBox> {
        &mut self.body_boxes
    }

    /// Says that the first `count` boxes of the vector from [`PhysicsScratch::boxes_for_next_step`] are those of the next
    /// step's first `count` bodies, made by [`BodyBox::of`] from the bodies as they are when the step starts: the step
    /// computes the others only. It is used by one step, which resets it.
    pub(crate) fn boxes_ready(&mut self, count: usize) {
        self.known_boxes = count;
    }

    /// Says that the next step need not write back the final state of its first `count` bodies into the bodies: the caller reads
    /// it from [`PhysicsScratch::solved`] (the copies of the bibites' poses into bodies that are rebuilt the next tick were
    /// 22 MB of writes at 100,000 bibites, read again at once). It is used by one step, which resets it.
    pub(crate) fn skip_write_back_of(&mut self, count: usize) {
        self.skip_write_back = count;
    }

    /// The solver's copy of every body of the last step, by body index: where the step ended, for the bodies it did not
    /// write back.
    pub(crate) fn solved(&self) -> &[SolverBody] {
        &self.solver
    }

    /// The boxes announced by [`PhysicsScratch::boxes_ready`].
    pub(crate) fn ready_boxes(&self) -> &[BodyBox] {
        &self.body_boxes[..self.known_boxes.min(self.body_boxes.len())]
    }

    /// Contact events produced by the last [`crate::physics::step`].
    pub fn contacts(&self) -> &[ContactEvent] {
        &self.contacts
    }

    /// Trigger events produced by the last [`crate::physics::step`].
    pub fn triggers(&self) -> &[TriggerEvent] {
        &self.triggers
    }

    /// Asks the next step to hold `weld` (a body held by another, see [`WeldDef`]).
    pub fn add_weld(&mut self, weld: WeldDef) {
        self.welds_in.push(weld);
    }

    /// The welds that gave way in the last step: the held body (by index in the step's slice) and the tag of the holder whose weld it was (a body can be
    /// held by several holders, and only the one whose weld gave way lets go).
    pub fn broken_welds(&self) -> &[(u32, u64)] {
        &self.broken_welds
    }

    /// The grabs the last step made inside its steps (see [`NewWeld`]): the caller records them as held.
    pub fn new_welds(&self) -> &[NewWeld] {
        &self.new_welds
    }

    /// Number of broadphase candidate collider pairs in the last [`crate::physics::step`].
    pub fn candidate_pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// Forgets the impulses the contacts carry from step to step (warm starting): what a loaded save or a restored checkpoint starts without, as the
    /// game's own loading does.
    pub fn forget_contact_impulses(&mut self) {
        self.carried.clear();
        self.carried_next.clear();
    }

    /// Clears all per-tick output while retaining capacity.
    pub fn clear(&mut self) {
        self.dense_cap_hit = false;
        self.sensor_entries.clear();
        self.sensor_candidates.clear();
        self.sensor_pair_keys.clear();
        self.entries.clear();
        self.pairs.clear();
        self.pair_keys.clear();
        self.solver.clear();
        self.constraints.clear();
        self.contacts.clear();
        self.triggers.clear();
    }
}

// ---------------------------------------------------------------------------
// Step
// ---------------------------------------------------------------------------

/// Returns mutable references to two distinct slice elements.
fn two_mut<T>(slice: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    if i < j {
        let (left, right) = slice.split_at_mut(j);
        (&mut left[i], &mut right[0])
    } else {
        let (left, right) = slice.split_at_mut(i);
        (&mut right[0], &mut left[j])
    }
}

/// Moves a body no contact holds through the step in closed form: its muscles' force and torque, the damping and the motion, the speed limits.
/// Every body is its own: the pool takes them in stretches.
#[inline]
fn integrate_free(body: &mut SolverBody, config: &PhysicsConfig, dt: f32, max_angular: f32) {
    if body.is_static || body.inv_mass == 0.0 {
        return;
    }
    let (vx, vy, dx, dy, w, dr);
    if body.force != Vec2::ZERO && (body.ang_vel != 0.0 || body.torque != 0.0) {
        // The body turns while its muscles push: the force follows the body's heading slice by slice (the game applies it before each of its ten
        // physics steps, along the transform's `up` of that moment), so the closed form does not hold.
        let n = config.substeps.max(1) as i32;
        let h = dt / n as f32;
        let (keep_linear, keep_angular) = (1.0 / (1.0 + h * body.linear_damping), 1.0 / (1.0 + h * body.angular_damping));
        let (gain_torque, gain_force) = (h * body.torque * body.inv_inertia, h * body.inv_mass);
        let (mut v, mut omega) = (body.vel, body.ang_vel);
        let (mut moved, mut turned) = (Vec2::ZERO, 0.0f32);
        for _ in 0..n {
            let push = turned_by(body.force, turned);
            v = (v + push * gain_force) * keep_linear;
            omega = (omega + gain_torque) * keep_angular;
            moved += v * h;
            turned += omega * h;
        }
        (vx, vy, dx, dy, w, dr) = (v.x, v.y, moved.x, moved.y, omega, turned);
    } else {
        let accel = body.force * body.inv_mass;
        let linear = slice_terms(body.linear_damping, dt, config.substeps.max(1) as i32);
        let (a, b) = sliced_axis(linear, body.vel.x, accel.x);
        let (c, d) = sliced_axis(linear, body.vel.y, accel.y);
        let (e, f) = sliced_axis(slice_terms(body.angular_damping, dt, config.substeps.max(1) as i32), body.ang_vel, body.torque * body.inv_inertia);
        (vx, dx, vy, dy, w, dr) = (a, b, c, d, e, f);
    }
    body.vel = Vec2::new(vx, vy).clamp_length(config.max_translation_speed);
    body.ang_vel = clamp(w, -max_angular, max_angular);
    let mut delta = Vec2::new(dx, dy);
    let max_delta = config.max_translation_speed * dt;
    let delta_sq = delta.length_sq();
    if delta_sq > max_delta * max_delta && delta_sq > 0.0 {
        delta *= max_delta / delta_sq.sqrt();
    }
    body.pos += delta;
    let max_angular_delta = max_angular * dt;
    body.rot += clamp(dr, -max_angular_delta, max_angular_delta);
}

/// `v` turned by `angle` radians: a polynomial for the small angles of a few slices (a body hardly turns in a tenth of a step), the exact rotation otherwise.
#[inline]
fn turned_by(v: Vec2, angle: f32) -> Vec2 {
    if angle.abs() < 0.3 {
        let a2 = angle * angle;
        let cos = 1.0 - a2 * 0.5 * (1.0 - a2 * (1.0 / 12.0));
        let sin = angle * (1.0 - a2 * (1.0 / 6.0) * (1.0 - a2 * 0.05));
        Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
    } else {
        v.rotate(angle)
    }
}

/// The sort key of the pooled x order: the box's start in the order of [`f32::total_cmp`] (negative numbers have all their
/// bits flipped, the others only the sign bit), over the body index. Sorted as integers, the keys come out in the order of
/// "start by `total_cmp`, equal starts by body index" - equal keys are equal bits, and `total_cmp` tells bits apart.
#[inline]
fn total_order_key(start: f32, body_index: u32) -> u64 {
    let bits = start.to_bits();
    let ordered = if bits >> 31 == 1 {
        !bits
    } else {
        bits | 0x8000_0000
    };
    (u64::from(ordered) << 32) | u64::from(body_index)
}

/// Abort threshold for the sweep fast path: above this many candidates the
/// population is dense/clustered and the grid fallback takes over.
const SWEEP_ABORT_PAIRS: usize = 50_000;

/// Dense-only deterministic pair cap per body. Synthetic 100x100 2000-body
/// clusters produce ~145k candidate pairs; without a cap the narrowphase and
/// solver spend most of the frame on an unrealistically dense pile. The cap is
/// applied only when the grid path is used with more than 512 bodies, so normal
/// and spread populations are bit-identical to the uncapped result. This is a
/// documented performance approximation for the dense benchmark profile.
const DENSE_PAIR_CAP_PER_BODY: usize = 32;
const DENSE_PAIR_CAP_MIN_ENTRIES: usize = 512;

/// Max candidate bodies supplemented per sensor collider in a dense world.
const SENSOR_CANDIDATE_CAP: usize = 0;

/// The x order and the sweep: the candidate pairs of the step, found the way the game's physics world is checked since the first version
/// (the bodies sorted by where their boxes start, each body paired with the ones whose boxes start before its own ends, and a dense
/// world handed to the capped grid). The stages are timed into `lap`.
fn sweep_broadphase(scratch: &mut PhysicsScratch, pool: usize, min_sort: usize, lap: &mut Instant) {
    let sort_workers = |items: usize, from: usize| crate::sim::ParallelGates::workers_for(items, from, pool);
    let boxes = &scratch.body_boxes;
    // The x order: the bodies sorted by where their boxes start, bodies whose boxes start at exactly the same x by body index.
    // That is a total order, so the result is the same whichever way it is reached - sorted from scratch (on the pool or
    // not) or carried from the last tick and repaired - and nothing depends on the history of the order.
    scratch.entries.clear();
    let order_workers = sort_workers(boxes.len(), min_sort);
    if order_workers > 1 {
        // On the pool the order is sorted from scratch as plain integers: the box's start in total order over the body index
        // (equal starts then go by body index, as the order wants), so a comparison looks nothing up. The carried order (the
        // serial repair below) is not kept; with the resting-pellet layer on, the number of bodies changes every tick and
        // the repair gave up and sorted from scratch anyway.
        let min_len = crate::sim::ParallelGates::chunk_len(boxes.len(), min_sort, order_workers, 4);
        let mut keys = std::mem::take(&mut scratch.sort_keys);
        keys.clear();
        keys.par_extend(
            boxes
                .par_iter()
                .enumerate()
                .with_min_len(min_len)
                .filter_map(|(index, entry)| {
                    entry
                        .valid
                        .then(|| total_order_key(entry.aabb.min.x, index as u32))
                }),
        );
        if sort_workers(keys.len(), min_sort) > 1 {
            keys.par_sort_unstable();
        } else {
            keys.sort_unstable();
        }
        // The entries come from the boxes (one record per body: its tag and layer are in it), in the order of the keys.
        scratch
            .entries
            .par_extend(keys.par_iter().with_min_len(min_len).map(|key| {
                let index = (*key & 0xFFFF_FFFF) as usize;
                let entry = &boxes[index];
                BodyEntry {
                    body: index as u32,
                    tag: entry.tag,
                    layer: entry.layer,
                    aabb: entry.aabb,
                }
            }));
        scratch.sort_keys = keys;
        scratch.body_order.clear();
    } else {
        scratch.order_keys.clear();
        scratch
            .order_keys
            .extend(boxes.iter().map(|entry| entry.aabb.min.x));
        let keys = &scratch.order_keys;
        let by_x = |a: &u32, b: &u32| {
            keys[*a as usize]
                .total_cmp(&keys[*b as usize])
                .then(a.cmp(b))
        };
        let order = &mut scratch.body_order;
        let mut sort_from_scratch = order.len() != boxes.len();
        if !sort_from_scratch {
            // Insertion sort: positions move a little per tick, so this is ~O(n). If the carried order turns out to be far
            // from sorted (the bodies behind the indices changed) it gives up and sorts from scratch instead of going
            // quadratic.
            let mut budget = order.len().saturating_mul(8).saturating_add(1024);
            'repair: for i in 1..order.len() {
                let mut j = i;
                while j > 0 {
                    if by_x(&order[j - 1], &order[j]) != std::cmp::Ordering::Greater {
                        break;
                    }
                    if budget == 0 {
                        sort_from_scratch = true;
                        break 'repair;
                    }
                    budget -= 1;
                    order.swap(j - 1, j);
                    j -= 1;
                }
            }
        }
        if sort_from_scratch {
            order.clear();
            order.extend((0..boxes.len() as u32).filter(|index| boxes[*index as usize].valid));
            order.sort_unstable_by(by_x);
        }
        for index in order.iter() {
            let body_index = *index as usize;
            let entry = &boxes[body_index];
            if !entry.valid {
                continue;
            }
            scratch.entries.push(BodyEntry {
                body: body_index as u32,
                tag: entry.tag,
                layer: entry.layer,
                aabb: entry.aabb,
            });
        }
    }
    scratch.clock.lap(Stage::PhysOrder, lap);
    build_broadphase(scratch);
    scratch.clock.lap(Stage::PhysBroadphase, lap);
}

/// The grid broadphase and the sweep give the same candidate pairs; this says from how many bodies the step asks the grid first.
/// `RUSTIBITES_BROADPHASE=sweep` never uses it, `=grid` uses it from [`GRID_MIN_BODIES`] bodies, and anything else (or nothing) from the
/// scratch's `min_grid` ([`crate::sim::ParallelGates::grid_bodies`]: about where it pays), and only for a tick on a pool of
/// [`GRID_MIN_THREADS`] workers or more: the pairs do not depend on it, only the time.
fn grid_broadphase_from(min_grid: usize, threads: usize) -> usize {
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    match *MODE.get_or_init(|| match std::env::var("RUSTIBITES_BROADPHASE").ok().as_deref() {
        Some("sweep") => 1,
        Some("grid") => 2,
        _ => 0,
    }) {
        1 => usize::MAX,
        2 => GRID_MIN_BODIES,
        // (A serial tick carries its x order from one step to the next, and has no pool to spread the grid over; a `min_grid` of 0 is a
        // scratch the caller did not set.)
        _ if min_grid == 0 || threads < GRID_MIN_THREADS => usize::MAX,
        _ => min_grid.max(GRID_MIN_BODIES),
    }
}

/// The fewest bodies the grid is tried for.
const GRID_MIN_BODIES: usize = 8;
/// In the default mode the grid is only for a tick on a pool of at least this many workers (below it the stripes have too few to run on).
const GRID_MIN_THREADS: usize = 4;
/// A cell is this many times the mean size of a box.
const GRID_CELL_FACTOR: f32 = 3.0;
/// A box that spans more cells than this is not put in the grid: it is looked at against every body instead.
const GRID_LARGE_SPAN: i64 = 36;
/// The grid gives way to the sweep when the large boxes times the bodies (the tests they cost) are more than this.
const GRID_LARGE_BUDGET: usize = 1_000_000;

/// One body's place in a cell of the grid: the cell, the body and a copy of its box, so that the tests of what shares a cell read
/// neighbours in memory instead of asking the boxes of scattered bodies.
#[derive(Clone, Copy, Debug, Default)]
struct GridMember {
    cell_x: i32,
    cell_y: i32,
    body: u32,
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

/// The cell of the coordinate `value` on a grid whose first cell starts at `origin` (it never decreases as `value` grows).
#[inline]
fn grid_cell(value: f32, origin: f32, inverse: f32) -> i32 {
    ((value - origin) * inverse).floor() as i32
}

/// The bucket of a cell in a table of `mask + 1` buckets (a different cell may share it: the members carry their cell).
#[inline]
fn grid_bucket(cell_x: i32, cell_y: i32, mask: u32) -> usize {
    let hash = (cell_x as u32).wrapping_mul(0x9E37_79B1) ^ (cell_y as u32).wrapping_mul(0x85EB_CA77);
    ((hash ^ (hash >> 15)) & mask) as usize
}

/// One stripe of the grid broadphase's world: the places of the bodies in its cells by bucket, where each bucket starts, and the pairs it found.
#[derive(Clone, Debug, Default)]
struct GridStripe {
    members: Vec<GridMember>,
    buckets: Vec<u32>,
    pairs: Vec<BodyPair>,
}

/// What one stretch of boxes says to the grid broadphase: how many take part, where they lie, how big they are, and whether every one is a
/// finite box.
#[derive(Clone, Copy)]
struct GridLook {
    valid: usize,
    low: Vec2,
    high: Vec2,
    extent_sum: f64,
    area_sum: f64,
    finite: bool,
}

/// The candidate pairs of a big, spread-out world by a hashed uniform grid, with no x order: **the pairs the sweep finds, the first of
/// a pair being the body whose box starts first in the sweep's order** (the start in the order of [`f32::total_cmp`], then the body
/// index), so that once the step puts them in order by tag they are the same list. Returns whether it did: `false` leaves the
/// pairs alone and the sweep does the work. It declines a world with fewer than `from` bodies, a box that is not a finite box, a
/// world dense enough for the sweep to hand it to the capped grid (or one with more candidates than that), and one with too many large
/// boxes.
///
/// Every box is put in each cell it overlaps (a box that spans more than [`GRID_LARGE_SPAN`] cells is kept aside and looked at against
/// every body); a pair of overlapping boxes is reported once, in the cell that holds the corner of the boxes' common part, which both
/// boxes are in. The world is cut into stripes of columns of cells, and every phase runs on the pool: the look at the boxes, the
/// sorting of the bodies into the stripes they reach, and then each stripe on its own, with a small hashed grid (a counting sort of its
/// members by bucket) that stays in a core's cache. A pair is found by the stripe that holds its corner cell and by no other, so no
/// stripe waits for another, and nothing depends on how many there are or in what order they finished.
fn grid_broadphase(scratch: &mut PhysicsScratch, from: usize) -> bool {
    let count = scratch.body_boxes.len();
    if count < from {
        return false;
    }
    let workers = crate::sim::ParallelGates::workers_for(count, scratch.min_broadphase, scratch.threads);
    let chunk_len = if workers > 1 {
        crate::sim::ParallelGates::chunk_len(count, scratch.min_broadphase, workers, 2)
    } else {
        count.max(1)
    };
    let boxes = &scratch.body_boxes;
    // 1. One look at every box: they must be finite boxes, and the world must be one the sweep would not call dense (by a margin that
    // makes the order the areas are added in, which differs here, irrelevant).
    let look = |chunk: &[BodyBox]| {
        let mut seen = GridLook {
            valid: 0,
            low: Vec2::splat(f32::INFINITY),
            high: Vec2::splat(f32::NEG_INFINITY),
            extent_sum: 0.0,
            area_sum: 0.0,
            finite: true,
        };
        for entry in chunk {
            if !entry.valid {
                continue;
            }
            let aabb = entry.aabb;
            if !(aabb.min.x.is_finite() && aabb.min.y.is_finite() && aabb.max.x.is_finite() && aabb.max.y.is_finite())
                || aabb.max.x < aabb.min.x
                || aabb.max.y < aabb.min.y
            {
                seen.finite = false;
                return seen;
            }
            seen.valid += 1;
            seen.low = seen.low.min(aabb.min);
            seen.high = seen.high.max(aabb.max);
            let extent = aabb.max - aabb.min;
            seen.extent_sum += f64::from(extent.x.max(extent.y));
            seen.area_sum += f64::from(extent.x) * f64::from(extent.y);
        }
        seen
    };
    let looks: Vec<GridLook> = if workers > 1 {
        boxes.par_chunks(chunk_len).map(look).collect()
    } else {
        vec![look(boxes)]
    };
    let mut valid = 0usize;
    let mut low = Vec2::splat(f32::INFINITY);
    let mut high = Vec2::splat(f32::NEG_INFINITY);
    let (mut extent_sum, mut area_sum) = (0.0f64, 0.0f64);
    for seen in &looks {
        if !seen.finite {
            return false;
        }
        valid += seen.valid;
        low = low.min(seen.low);
        high = high.max(seen.high);
        extent_sum += seen.extent_sum;
        area_sum += seen.area_sum;
    }
    if valid < 2 || !extent_sum.is_finite() || !area_sum.is_finite() {
        return false;
    }
    let union = high - low;
    let union_area = f64::from(union.x) * f64::from(union.y);
    if !union_area.is_finite() || (union_area > 0.0 && area_sum * 2.0 > union_area * 8.0) {
        return false;
    }
    let cell = ((extent_sum / valid as f64) as f32 * GRID_CELL_FACTOR).max(2.0);
    if !cell.is_finite() {
        return false;
    }
    let inverse = 1.0 / cell;
    let cells_of = |aabb: &Aabb| {
        (
            grid_cell(aabb.min.x, low.x, inverse),
            grid_cell(aabb.max.x, low.x, inverse),
            grid_cell(aabb.min.y, low.y, inverse),
            grid_cell(aabb.max.y, low.y, inverse),
        )
    };
    let spans_of = |(x0, x1, y0, y1): (i32, i32, i32, i32)| (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1);
    // 2. The stripes: columns of cells, as many as will keep the workers busy (a few each), and the lists of the bodies each reaches, by
    // chunk of bodies (a chunk fills its own lists), with the bodies too big for the grid kept aside.
    let columns = i64::from(grid_cell(high.x, low.x, inverse)) + 1;
    let wanted = if workers > 1 { (workers * 4) as i64 } else { 1 };
    let stripe_columns = ((columns + wanted - 1) / wanted).max(1);
    let stripes = ((columns + stripe_columns - 1) / stripe_columns).max(1) as usize;
    let chunks = count.div_ceil(chunk_len);
    let lists = &mut scratch.grid_lists;
    lists.truncate(chunks * stripes);
    lists.resize_with(chunks * stripes, Vec::new);
    let classify = |chunk: &[BodyBox], lists: &mut [Vec<u32>], base: usize| {
        let mut large: Vec<u32> = Vec::new();
        for list in lists.iter_mut() {
            list.clear();
        }
        for (offset, entry) in chunk.iter().enumerate() {
            if !entry.valid {
                continue;
            }
            let cells = cells_of(&entry.aabb);
            let body = (base + offset) as u32;
            if spans_of(cells) > GRID_LARGE_SPAN {
                large.push(body);
                continue;
            }
            let (x0, x1, ..) = cells;
            for stripe in (i64::from(x0) / stripe_columns)..=(i64::from(x1) / stripe_columns) {
                lists[stripe as usize].push(body);
            }
        }
        large
    };
    let larges: Vec<Vec<u32>> = if workers > 1 {
        boxes
            .par_chunks(chunk_len)
            .zip(lists.par_chunks_mut(stripes))
            .enumerate()
            .map(|(index, (chunk, lists))| classify(chunk, lists, index * chunk_len))
            .collect()
    } else {
        boxes
            .chunks(chunk_len)
            .zip(lists.chunks_mut(stripes))
            .enumerate()
            .map(|(index, (chunk, lists))| classify(chunk, lists, index * chunk_len))
            .collect()
    };
    let large: Vec<u32> = larges.into_iter().flatten().collect();
    if large.len().saturating_mul(valid) > GRID_LARGE_BUDGET {
        return false;
    }
    // A pair of overlapping boxes: first the one the sweep's order puts first, and only if their tags differ and their layers collide.
    let push = |pairs: &mut Vec<BodyPair>, a: u32, b: u32| {
        let key = |body: u32| total_order_key(boxes[body as usize].aabb.min.x, body);
        let (first, second) = if key(a) < key(b) { (a, b) } else { (b, a) };
        let (first_entry, second_entry) = (&boxes[first as usize], &boxes[second as usize]);
        if first_entry.tag == second_entry.tag || !collision_allowed(first_entry.layer, second_entry.layer) {
            return;
        }
        pairs.push(BodyPair {
            body_a: first,
            body_b: second,
            tag_a: first_entry.tag,
            tag_b: second_entry.tag,
        });
    };
    // 3. Each stripe on its own: its members by bucket (count, then place from the back so that each bucket's start is left in
    // `buckets`), and the pairs of what shares a cell.
    let grid = &mut scratch.grid_stripes;
    grid.truncate(stripes);
    grid.resize_with(stripes, GridStripe::default);
    let lists: &[Vec<u32>] = &scratch.grid_lists;
    let run_stripe = |index: usize, stripe: &mut GridStripe| {
        let first_column = index as i64 * stripe_columns;
        let last_column = first_column + stripe_columns - 1;
        let in_stripe = |body: u32| {
            let (x0, x1, y0, y1) = cells_of(&boxes[body as usize].aabb);
            (
                (i64::from(x0).max(first_column)) as i32,
                (i64::from(x1).min(last_column)) as i32,
                y0,
                y1,
            )
        };
        let mut places = 0usize;
        for chunk in 0..chunks {
            for &body in &lists[chunk * stripes + index] {
                places += spans_of(in_stripe(body)) as usize;
            }
        }
        let bucket_count = (places.max(8) * 2).next_power_of_two();
        let mask = (bucket_count - 1) as u32;
        stripe.pairs.clear();
        stripe.buckets.clear();
        stripe.buckets.resize(bucket_count + 1, 0);
        for chunk in 0..chunks {
            for &body in &lists[chunk * stripes + index] {
                let (x0, x1, y0, y1) = in_stripe(body);
                for cell_y in y0..=y1 {
                    for cell_x in x0..=x1 {
                        stripe.buckets[grid_bucket(cell_x, cell_y, mask)] += 1;
                    }
                }
            }
        }
        for at in 1..bucket_count {
            stripe.buckets[at] += stripe.buckets[at - 1];
        }
        stripe.buckets[bucket_count] = places as u32;
        stripe.members.clear();
        stripe.members.resize(places, GridMember::default());
        for chunk in 0..chunks {
            for &body in &lists[chunk * stripes + index] {
                let aabb = boxes[body as usize].aabb;
                let (x0, x1, y0, y1) = in_stripe(body);
                for cell_y in y0..=y1 {
                    for cell_x in x0..=x1 {
                        let bucket = grid_bucket(cell_x, cell_y, mask);
                        stripe.buckets[bucket] -= 1;
                        stripe.members[stripe.buckets[bucket] as usize] = GridMember {
                            cell_x,
                            cell_y,
                            body,
                            min_x: aabb.min.x,
                            min_y: aabb.min.y,
                            max_x: aabb.max.x,
                            max_y: aabb.max.y,
                        };
                    }
                }
            }
        }
        let GridStripe { members, buckets, pairs } = stripe;
        for bucket in 0..bucket_count {
            let (start, end) = (buckets[bucket] as usize, buckets[bucket + 1] as usize);
            for first in start..end {
                let a = &members[first];
                for b in &members[first + 1..end] {
                    if a.cell_x != b.cell_x
                        || a.cell_y != b.cell_y
                        || !(b.max_x >= a.min_x && a.max_x >= b.min_x && b.max_y >= a.min_y && a.max_y >= b.min_y)
                    {
                        continue;
                    }
                    // Found in the cell that holds the corner of the common part of the two boxes, and nowhere else.
                    if grid_cell(a.min_x.max(b.min_x), low.x, inverse) != a.cell_x
                        || grid_cell(a.min_y.max(b.min_y), low.y, inverse) != a.cell_y
                    {
                        continue;
                    }
                    push(pairs, a.body, b.body);
                }
            }
        }
    };
    if workers > 1 {
        grid.par_iter_mut().enumerate().for_each(|(index, stripe)| run_stripe(index, stripe));
    } else {
        for (index, stripe) in grid.iter_mut().enumerate() {
            run_stripe(index, stripe);
        }
    }
    let mut found: Vec<BodyPair> = Vec::with_capacity(grid.iter().map(|stripe| stripe.pairs.len()).sum());
    for stripe in grid.iter() {
        found.extend_from_slice(&stripe.pairs);
    }
    // 4. The large boxes against every body (a pair of two of them from the one that comes first in `large`).
    for &big in &large {
        let big_box = boxes[big as usize].aabb;
        for (other, entry) in boxes.iter().enumerate() {
            if !entry.valid || other == big as usize {
                continue;
            }
            let aabb = entry.aabb;
            if !(aabb.max.x >= big_box.min.x
                && big_box.max.x >= aabb.min.x
                && aabb.max.y >= big_box.min.y
                && big_box.max.y >= aabb.min.y)
            {
                continue;
            }
            if (other as u32) < big && large.binary_search(&(other as u32)).is_ok() {
                continue;
            }
            push(&mut found, big, other as u32);
        }
    }
    // More candidates than the sweep keeps is a dense world: the sweep (and then the capped grid) answers for it.
    if found.len() > SWEEP_ABORT_PAIRS {
        return false;
    }
    scratch.pairs.clear();
    scratch.pairs = found;
    scratch.pair_keys.clear();
    scratch.body_order.clear();
    scratch.dense_cap_hit = false;
    true
}

/// Builds the broadphase candidate list.
///
/// Deterministic sweep-and-prune is fastest for spread populations; if it finds
/// more than [`SWEEP_ABORT_PAIRS`] candidates (dense/clustered case) it aborts
/// and the uniform-grid path finishes with a hard cap. Both emit deterministic
/// pair order once the caller sorts by body tags.
fn build_broadphase(scratch: &mut PhysicsScratch) {
    scratch.pairs.clear();
    scratch.pair_keys.clear();
    if scratch.entries.is_empty() {
        return;
    }
    // Dense populations exceed the sweep abort threshold almost immediately;
    // estimate AABB coverage first and go straight to the grid so the sweep
    // does not waste time collecting candidates it will discard.
    if scratch.entries.len() > 256 {
        let mut bounds_min = Vec2::splat(f32::INFINITY);
        let mut bounds_max = Vec2::splat(f32::NEG_INFINITY);
        let mut area_sum = 0.0f64;
        for entry in &scratch.entries {
            bounds_min = bounds_min.min(entry.aabb.min);
            bounds_max = bounds_max.max(entry.aabb.max);
            let extent = (entry.aabb.max - entry.aabb.min).max(Vec2::ZERO);
            area_sum += f64::from(extent.x) * f64::from(extent.y);
        }
        let union = (bounds_max - bounds_min).max(Vec2::ZERO);
        let union_area = f64::from(union.x) * f64::from(union.y);
        if union_area > 0.0 && area_sum > union_area * 8.0 {
            build_grid_broadphase(scratch);
            return;
        }
    }
    // The sweep is cut by first entry: each entry's candidates are found independently of the others', and the cuts'
    // pairs are joined in order, which is the order one pass over all the entries gives. One cut is the serial pass.
    let threads = crate::sim::ParallelGates::workers_for(
        scratch.entries.len(),
        scratch.min_broadphase,
        scratch.threads,
    );
    scratch
        .sweep_columns
        .fill(&scratch.entries, scratch.threads, scratch.min_sort);
    let entries = &scratch.entries;
    let columns = &scratch.sweep_columns;
    let cut_len = if threads > 1 {
        crate::sim::ParallelGates::chunk_len(entries.len(), scratch.min_broadphase, threads, 4)
    } else {
        entries.len()
    };
    let starts: Vec<usize> = (0..entries.len()).step_by(cut_len).collect();
    let cuts: Vec<(Vec<BodyPair>, bool)> = if threads > 1 {
        starts
            .par_iter()
            .map(|&start| {
                sweep_cut(
                    entries,
                    columns,
                    start,
                    (start + cut_len).min(entries.len()),
                )
            })
            .collect()
    } else {
        starts
            .iter()
            .map(|&start| {
                sweep_cut(
                    entries,
                    columns,
                    start,
                    (start + cut_len).min(entries.len()),
                )
            })
            .collect()
    };
    // The serial pass gave up at the first candidate past the limit: that is more candidates than the limit in all.
    let total: usize = cuts.iter().map(|(pairs, _)| pairs.len()).sum();
    if total > SWEEP_ABORT_PAIRS || cuts.iter().any(|(_, overflow)| *overflow) {
        scratch.pairs.clear();
        build_grid_broadphase(scratch);
        return;
    }
    scratch.pairs.reserve(total);
    for (pairs, _) in cuts {
        scratch.pairs.extend(pairs);
    }
}

/// The sweep's candidate pairs for the first entries `start..end`, in order, and whether there were more than
/// [`SWEEP_ABORT_PAIRS`] of them (it stops there).
///
/// For an entry the candidates are the entries after it whose box starts at or before where its box ends (the x order makes
/// them a run, found by a binary search; an entry whose box has a NaN in it overlaps nothing, wherever the search ends), and a
/// candidate is a hit when the boxes overlap in y and the other's box ends at or after where this one starts. The other
/// conditions (not the same body or tag, layers that collide) are asked of the hits only: they are all pure, so the pairs are
/// the same as when every candidate is asked everything, and in the same order.
fn sweep_cut(
    entries: &[BodyEntry],
    columns: &SweepColumns,
    start: usize,
    end: usize,
) -> (Vec<BodyPair>, bool) {
    let mut pairs = Vec::new();
    for i in start..end {
        let first = &entries[i];
        let (first_min_x, first_max_x) = (first.aabb.min.x, first.aabb.max.x);
        let (first_min_y, first_max_y) = (first.aabb.min.y, first.aabb.max.y);
        let low = i + 1;
        let high = low
            + columns.min_x[low..]
                .partition_point(|candidate_start| *candidate_start <= first_max_x);
        let mut at = low;
        while at + 8 <= high {
            let mut hits = overlap_mask8(columns, at, first_min_x, first_max_y, first_min_y);
            while hits != 0 {
                let lane = hits.trailing_zeros() as usize;
                hits &= hits - 1;
                if push_pair(&mut pairs, first, &entries[at + lane]) {
                    return (pairs, true);
                }
            }
            at += 8;
        }
        while at < high {
            if columns.max_x[at] >= first_min_x
                && columns.min_y[at] <= first_max_y
                && columns.max_y[at] >= first_min_y
                && push_pair(&mut pairs, first, &entries[at])
            {
                return (pairs, true);
            }
            at += 1;
        }
    }
    (pairs, false)
}

/// Bit `lane` of the result is set when the box of entry `at + lane` is hit by the query box whose left edge is `min_x` and whose
/// y range is `min_y..=max_y`: it ends at or after `min_x` and its y range meets the query's (a comparison with a NaN is false).
/// Eight entries at once; with the `prefetch` feature (on by default) as two SSE2 vectors of four, which does not depend on what
/// the compiler makes of a loop of eight comparisons (it vectorized that one or not, depending on code far away: 1.7 or 2.5 ms
/// for the broadphase of the same stage in two builds), and as that loop without it.
#[inline(always)]
fn overlap_mask8(columns: &SweepColumns, at: usize, min_x: f32, max_y: f32, min_y: f32) -> u32 {
    let max_x_lanes = <&[f32; 8]>::try_from(&columns.max_x[at..at + 8]).expect("eight entries");
    let min_y_lanes = <&[f32; 8]>::try_from(&columns.min_y[at..at + 8]).expect("eight entries");
    let max_y_lanes = <&[f32; 8]>::try_from(&columns.max_y[at..at + 8]).expect("eight entries");
    #[cfg(feature = "prefetch")]
    {
        bibites_prefetch::overlap_mask8(max_x_lanes, min_y_lanes, max_y_lanes, min_x, max_y, min_y)
    }
    #[cfg(not(feature = "prefetch"))]
    {
        let mut hits = 0u32;
        for lane in 0..8 {
            let hit = (max_x_lanes[lane] >= min_x)
                & (min_y_lanes[lane] <= max_y)
                & (max_y_lanes[lane] >= min_y);
            hits |= u32::from(hit) << lane;
        }
        hits
    }
}

/// Adds the pair of a hit of the sweep if the two bodies are different, their tags too, and their layers collide. Returns
/// whether the list is past [`SWEEP_ABORT_PAIRS`] (the pair is then not added).
#[inline]
fn push_pair(pairs: &mut Vec<BodyPair>, first: &BodyEntry, second: &BodyEntry) -> bool {
    if first.body == second.body
        || first.tag == second.tag
        || !collision_allowed(first.layer, second.layer)
    {
        return false;
    }
    if pairs.len() >= SWEEP_ABORT_PAIRS {
        return true;
    }
    pairs.push(BodyPair {
        body_a: first.body,
        body_b: second.body,
        tag_a: first.tag,
        tag_b: second.tag,
    });
    false
}

/// Builds the broadphase candidate list with a deterministic uniform grid.
///
/// Every collider AABB is inserted into the grid cells it overlaps; a pair is
/// emitted once when the two collider AABBs share a cell and overlap. Candidate
/// keys are sorted/deduped before conversion so a pair spanning several shared
/// cells is reported once. Output is capped at [`MAX_BROADPHASE_PAIRS`].
fn build_grid_broadphase(scratch: &mut PhysicsScratch) {
    scratch.pairs.clear();
    scratch.pair_keys.clear();
    if scratch.entries.is_empty() {
        return;
    }
    let mut bounds_min = Vec2::splat(f32::INFINITY);
    let mut bounds_max = Vec2::splat(f32::NEG_INFINITY);
    let mut max_extent = 0.0f32;
    for entry in &scratch.entries {
        bounds_min = bounds_min.min(entry.aabb.min);
        bounds_max = bounds_max.max(entry.aabb.max);
        let extent = (entry.aabb.max - entry.aabb.min).max(Vec2::ZERO);
        max_extent = max_extent.max(extent.x.max(extent.y));
    }
    if !bounds_min.is_finite()
        || !bounds_max.is_finite()
        || !max_extent.is_finite()
        || max_extent <= 0.0
    {
        return;
    }
    // Cell size starts at the largest collider extent, then grows until the
    // cell count fits the cap (keeps grids for huge worlds bounded).
    let mut cell = (max_extent * 0.5).max(4.0);
    let mut nx;
    let mut ny;
    loop {
        nx = (((bounds_max.x - bounds_min.x) / cell).ceil() as usize).max(1);
        ny = (((bounds_max.y - bounds_min.y) / cell).ceil() as usize).max(1);
        if nx.saturating_mul(ny) <= MAX_GRID_CELLS || !cell.is_finite() {
            break;
        }
        cell *= 2.0;
    }
    let cell_count = nx.saturating_mul(ny).min(MAX_GRID_CELLS);
    nx = nx.min(cell_count.max(1));
    ny = ny.min(cell_count.max(1));
    let inv_cell = 1.0 / cell;
    scratch.cell_heads.clear();
    scratch.cell_heads.resize(nx * ny, u32::MAX);
    scratch.cell_next.clear();
    scratch.cell_node_entry.clear();
    let cell_range = |aabb: Aabb| -> (usize, usize, usize, usize) {
        let cx0 = (((aabb.min.x - bounds_min.x) * inv_cell).floor() as isize)
            .clamp(0, nx as isize - 1) as usize;
        let cy0 = (((aabb.min.y - bounds_min.y) * inv_cell).floor() as isize)
            .clamp(0, ny as isize - 1) as usize;
        let cx1 = (((aabb.max.x - bounds_min.x) * inv_cell).floor() as isize)
            .clamp(0, nx as isize - 1) as usize;
        let cy1 = (((aabb.max.y - bounds_min.y) * inv_cell).floor() as isize)
            .clamp(0, ny as isize - 1) as usize;
        (cx0, cy0, cx1, cy1)
    };
    // Insert each entry into every cell its AABB overlaps.
    for (index, entry) in scratch.entries.iter().enumerate() {
        let (cx0, cy0, cx1, cy1) = cell_range(entry.aabb);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let cell_slot = cy * nx + cx;
                let node = scratch.cell_next.len() as u32;
                scratch.cell_node_entry.push(index as u32);
                scratch.cell_next.push(scratch.cell_heads[cell_slot]);
                scratch.cell_heads[cell_slot] = node;
            }
        }
    }
    // Collect unique candidate keys.
    'outer: for (index, entry) in scratch.entries.iter().enumerate() {
        let (cx0, cy0, cx1, cy1) = cell_range(entry.aabb);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let mut node = scratch.cell_heads[cy * nx + cx];
                while node != u32::MAX {
                    let other_index = scratch.cell_node_entry[node as usize] as usize;
                    if other_index > index {
                        let candidate = &scratch.entries[other_index];
                        if !entry.aabb.overlaps(&candidate.aabb)
                            || candidate.body == entry.body
                            || candidate.tag == entry.tag
                            || !collision_allowed(entry.layer, candidate.layer)
                        {
                            node = scratch.cell_next[node as usize];
                            continue;
                        }
                        if scratch.pair_keys.len() >= MAX_BROADPHASE_PAIRS {
                            break 'outer;
                        }
                        scratch
                            .pair_keys
                            .push(((index as u64) << 32) | other_index as u64);
                    }
                    node = scratch.cell_next[node as usize];
                }
            }
        }
    }
    scratch.pair_keys.sort_unstable();
    scratch.pair_keys.dedup();
    let cap = if scratch.entries.len() > DENSE_PAIR_CAP_MIN_ENTRIES {
        DENSE_PAIR_CAP_PER_BODY
    } else {
        usize::MAX
    };
    let max_body = scratch
        .entries
        .iter()
        .map(|entry| entry.body as usize)
        .max()
        .unwrap_or(0);
    let mut per_body = vec![0usize; max_body + 1];
    for key in &scratch.pair_keys {
        let a = (key >> 32) as usize;
        let b = (*key & 0xFFFF_FFFF) as usize;
        let first = scratch.entries[a];
        let second = scratch.entries[b];
        let body_a = first.body as usize;
        let body_b = second.body as usize;
        if per_body[body_a] >= cap || per_body[body_b] >= cap {
            // Solid cap for pathological dense worlds; the separate sensor
            // broadphase below re-adds any missing mouth/trigger pairs.
            scratch.dense_cap_hit = true;
            continue;
        }
        per_body[body_a] += 1;
        per_body[body_b] += 1;
        scratch.pairs.push(BodyPair {
            body_a: first.body,
            body_b: second.body,
            tag_a: first.tag,
            tag_b: second.tag,
        });
    }
}

/// Supplemental sensor-only pass used only when the dense solid cap dropped
/// pairs. Queries a small body grid per sensor collider and emits trigger
/// overlaps for sensor pairs not already present in the solid pair list. It
/// never adds candidate pairs to the main broadphase, so cluster pair counts and
/// throughput stay at the frozen level; realistic densities never hit the cap
/// and never run this pass.
fn append_sensor_triggers(
    bodies: &[PhysicsBody],
    scratch: &mut PhysicsScratch,
    config: &PhysicsConfig,
) {
    use std::collections::HashSet;

    if SENSOR_CANDIDATE_CAP == 0 {
        return;
    }
    scratch.sensor_entries.clear();
    for (body_index, body) in bodies.iter().enumerate() {
        for (collider_index, collider) in body.colliders().iter().enumerate() {
            if !collider.is_sensor {
                continue;
            }
            let slot = body_index * MAX_COLLIDERS + collider_index;
            scratch.sensor_entries.push(SensorEntry {
                body: body_index as u32,
                aabb: scratch.collider_aabbs[slot],
            });
        }
    }
    if scratch.sensor_entries.is_empty() || scratch.entries.is_empty() {
        return;
    }
    let mut bounds_min = Vec2::splat(f32::INFINITY);
    let mut bounds_max = Vec2::splat(f32::NEG_INFINITY);
    let mut max_extent: f32 = 4.0;
    for entry in &scratch.entries {
        bounds_min = bounds_min.min(entry.aabb.min);
        bounds_max = bounds_max.max(entry.aabb.max);
        max_extent = max_extent.max(
            (entry.aabb.max - entry.aabb.min)
                .x
                .max((entry.aabb.max - entry.aabb.min).y),
        );
    }
    let mut cell = (max_extent * 0.5).max(4.0);
    let mut nx;
    let mut ny;
    loop {
        nx = (((bounds_max.x - bounds_min.x) / cell).ceil() as usize).max(1);
        ny = (((bounds_max.y - bounds_min.y) / cell).ceil() as usize).max(1);
        if nx.saturating_mul(ny) <= MAX_GRID_CELLS || !cell.is_finite() {
            break;
        }
        cell *= 2.0;
    }
    let cell_count = nx.saturating_mul(ny).min(MAX_GRID_CELLS);
    nx = nx.min(cell_count.max(1));
    ny = ny.min(cell_count.max(1));
    let inv_cell = 1.0 / cell;
    scratch.cell_heads.clear();
    scratch.cell_heads.resize(nx * ny, u32::MAX);
    scratch.cell_next.clear();
    scratch.cell_node_entry.clear();
    let cell_range = |aabb: Aabb| -> (usize, usize, usize, usize) {
        let cx0 = (((aabb.min.x - bounds_min.x) * inv_cell).floor() as isize)
            .clamp(0, nx as isize - 1) as usize;
        let cy0 = (((aabb.min.y - bounds_min.y) * inv_cell).floor() as isize)
            .clamp(0, ny as isize - 1) as usize;
        let cx1 = (((aabb.max.x - bounds_min.x) * inv_cell).floor() as isize)
            .clamp(0, nx as isize - 1) as usize;
        let cy1 = (((aabb.max.y - bounds_min.y) * inv_cell).floor() as isize)
            .clamp(0, ny as isize - 1) as usize;
        (cx0, cy0, cx1, cy1)
    };
    for (index, entry) in scratch.entries.iter().enumerate() {
        let (cx0, cy0, cx1, cy1) = cell_range(entry.aabb);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let cell_slot = cy * nx + cx;
                let node = scratch.cell_next.len() as u32;
                scratch.cell_node_entry.push(index as u32);
                scratch.cell_next.push(scratch.cell_heads[cell_slot]);
                scratch.cell_heads[cell_slot] = node;
            }
        }
    }
    scratch.sensor_pair_keys.clear();
    for sensor in &scratch.sensor_entries {
        let (cx0, cy0, cx1, cy1) = cell_range(sensor.aabb);
        scratch.sensor_candidates.clear();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let mut node = scratch.cell_heads[cy * nx + cx];
                while node != u32::MAX {
                    let entry = &scratch.entries[scratch.cell_node_entry[node as usize] as usize];
                    if entry.body != sensor.body
                        && entry.tag != bodies[sensor.body as usize].tag
                        && collision_allowed(entry.layer, bodies[sensor.body as usize].layer)
                        && entry.aabb.overlaps(&sensor.aabb)
                    {
                        scratch.sensor_candidates.push(entry.body);
                    }
                    node = scratch.cell_next[node as usize];
                }
            }
        }
        scratch.sensor_candidates.sort_unstable();
        scratch.sensor_candidates.dedup();
        for other in scratch.sensor_candidates.iter().take(SENSOR_CANDIDATE_CAP) {
            let a = sensor.body.min(*other);
            let b = sensor.body.max(*other);
            scratch.sensor_pair_keys.push((a, b));
        }
    }
    scratch.sensor_pair_keys.sort_unstable();
    scratch.sensor_pair_keys.dedup();
    let existing: HashSet<(u32, u32)> = scratch
        .pairs
        .iter()
        .map(|pair| (pair.body_a.min(pair.body_b), pair.body_a.max(pair.body_b)))
        .collect();
    for key in &scratch.sensor_pair_keys {
        if existing.contains(key) {
            continue;
        }
        let (a, b) = *key;
        let body_a = &bodies[a as usize];
        let body_b = &bodies[b as usize];
        if !collision_allowed(body_a.layer, body_b.layer) || body_a.tag == body_b.tag {
            continue;
        }
        let slot_a = a as usize * MAX_COLLIDERS;
        let slot_b = b as usize * MAX_COLLIDERS;
        for (collider_a_index, collider_a) in body_a.colliders().iter().enumerate() {
            let world_a = scratch.collider_poses[slot_a + collider_a_index];
            let aabb_a = scratch.collider_aabbs[slot_a + collider_a_index];
            for (collider_b_index, collider_b) in body_b.colliders().iter().enumerate() {
                if !(collider_a.is_sensor || collider_b.is_sensor) {
                    continue;
                }
                if !aabb_a.overlaps(&scratch.collider_aabbs[slot_b + collider_b_index]) {
                    continue;
                }
                let world_b = scratch.collider_poses[slot_b + collider_b_index];
                let Some(manifold) = collide_with_skin(
                    &collider_a.shape,
                    world_a,
                    &collider_b.shape,
                    world_b,
                    config.contact_offset,
                ) else {
                    continue;
                };
                let point = manifold.points[0];
                scratch.triggers.push(TriggerEvent {
                    body_a: a as usize,
                    body_b: b as usize,
                    tag_a: body_a.tag,
                    tag_b: body_b.tag,
                    sensor_a: collider_a.is_sensor,
                    sensor_b: collider_b.is_sensor,
                    normal: manifold.normal,
                    point: point.point,
                    separation: point.separation,
                });
            }
        }
    }
}

/// Result of one narrowphase chunk. `constraint.event` indexes the chunk's
/// own contacts; the caller rebases it when merging chunks in order.
#[derive(Default)]
struct NarrowOutput {
    contacts: Vec<ContactEvent>,
    constraints: Vec<ContactConstraint>,
    triggers: Vec<TriggerEvent>,
}

fn narrow_phase(
    pairs: &[BodyPair],
    bodies: &[PhysicsBody],
    solver: &[SolverBody],
    collider_aabbs: &[Aabb],
    collider_poses: &[Pose],
    config: &PhysicsConfig,
    dt: f32,
) -> NarrowOutput {
    let mut output = NarrowOutput {
        contacts: Vec::with_capacity(pairs.len()),
        constraints: Vec::with_capacity(pairs.len()),
        triggers: Vec::with_capacity(pairs.len() / 2),
    };
    narrow_phase_into(pairs, bodies, solver, collider_aabbs, collider_poses, config, dt, false, &mut output);
    output
}

/// [`narrow_phase`] into buffers the caller keeps (they are cleared first).
fn narrow_phase_into(
    pairs: &[BodyPair],
    bodies: &[PhysicsBody],
    solver: &[SolverBody],
    collider_aabbs: &[Aabb],
    collider_poses: &[Pose],
    config: &PhysicsConfig,
    dt: f32,
    skip_sensors: bool,
    output: &mut NarrowOutput,
) {
    output.contacts.clear();
    output.constraints.clear();
    output.triggers.clear();
    let contacts = &mut output.contacts;
    let constraints = &mut output.constraints;
    let triggers = &mut output.triggers;
    for pair in pairs {
        let body_a = &bodies[pair.body_a as usize];
        let body_b = &bodies[pair.body_b as usize];
        if !collision_allowed(body_a.layer, body_b.layer) || body_a.tag == body_b.tag {
            continue;
        }
        let attached = (body_a.attached_to != 0 && body_a.attached_to == body_b.tag)
            || (body_b.attached_to != 0 && body_b.attached_to == body_a.tag);
        let slot_a = pair.body_a as usize * MAX_COLLIDERS;
        let slot_b = pair.body_b as usize * MAX_COLLIDERS;
        // Two bodies that cover more than the speculative distance in a step find each other from further apart: a contact that the step's motion
        // will bring to touching (the game's contacts catch a body flying at another one, which a fixed margin would let through).
        let speed = (solver[pair.body_b as usize].vel - solver[pair.body_a as usize].vel).length();
        let margin = if config.catch_fast_approach { (speed * dt).clamp(config.speculative_distance, 12.0) } else { config.speculative_distance };
        let fast = margin > config.speculative_distance;
        let (event_start, constraint_start) = (contacts.len(), constraints.len());
        for (collider_a_index, collider_a) in body_a.colliders().iter().enumerate() {
            let aabb_a = collider_aabbs[slot_a + collider_a_index];
            let world_a = collider_poses[slot_a + collider_a_index];
            for (collider_b_index, collider_b) in body_b.colliders().iter().enumerate() {
                if skip_sensors && (collider_a.is_sensor || collider_b.is_sensor) {
                    continue;
                }
                if !aabb_a.overlaps(&collider_aabbs[slot_b + collider_b_index])
                    && !(fast && Aabb { min: aabb_a.min - Vec2::splat(margin), max: aabb_a.max + Vec2::splat(margin) }.overlaps(&collider_aabbs[slot_b + collider_b_index]))
                {
                    continue;
                }
                let world_b = collider_poses[slot_b + collider_b_index];
                // Two circles (pellets, eggs) make a contact only when they overlap: the game's pellets 0.0005 apart do not push each other, 0.0005 into
                // each other they do (measured with piles of pellets spaced 4.998 ... 5.002 apart, radius 2.5); the other shapes keep the margin.
                let both_circles = !(collider_a.is_sensor || collider_b.is_sensor) && matches!((collider_a.shape, collider_b.shape), (Shape::Circle { .. }, Shape::Circle { .. }));
                let margin = if both_circles { 0.0 } else { margin };
                let speculative = if both_circles { 0.0 } else { config.speculative_distance };
                let Some(manifold) = collide_with_skin(
                    &collider_a.shape,
                    world_a,
                    &collider_b.shape,
                    world_b,
                    if collider_a.is_sensor || collider_b.is_sensor { config.contact_offset } else { 0.5 * margin },
                ) else {
                    continue;
                };
                let point = manifold.points[0];
                if collider_a.is_sensor || collider_b.is_sensor {
                    triggers.push(TriggerEvent {
                        body_a: pair.body_a as usize,
                        body_b: pair.body_b as usize,
                        tag_a: body_a.tag,
                        tag_b: body_b.tag,
                        sensor_a: collider_a.is_sensor,
                        sensor_b: collider_b.is_sensor,
                        normal: manifold.normal,
                        point: point.point,
                        separation: point.separation,
                    });
                    continue;
                }
                if attached {
                    continue;
                }
                let mut first_event = u32::MAX;
                for point_index in 0..manifold.count as usize {
                let point = manifold.points[point_index];
                let solver_a = &solver[pair.body_a as usize];
                let solver_b = &solver[pair.body_b as usize];
                let ra = point.point - solver_a.pos;
                let rb = point.point - solver_b.pos;
                let va = solver_a.vel + ra.perp() * solver_a.ang_vel;
                let vb = solver_b.vel + rb.perp() * solver_b.ang_vel;
                let relative_velocity = vb - va;
                let normal_speed = relative_velocity.dot(manifold.normal);
                let friction = config
                    .friction_combine
                    .combine(collider_a.friction, collider_b.friction);
                let restitution = config
                    .restitution_combine
                    .combine(collider_a.restitution, collider_b.restitution);
                let tangent = manifold.normal.perp();
                let rn_a = ra.cross(manifold.normal);
                let rn_b = rb.cross(manifold.normal);
                let k_normal = solver_a.inv_mass
                    + solver_b.inv_mass
                    + solver_a.inv_inertia * rn_a * rn_a
                    + solver_b.inv_inertia * rn_b * rn_b;
                let rt_a = ra.cross(tangent);
                let rt_b = rb.cross(tangent);
                let k_tangent = solver_a.inv_mass
                    + solver_b.inv_mass
                    + solver_a.inv_inertia * rt_a * rt_a
                    + solver_b.inv_inertia * rt_b * rt_b;
                // The shapes were grown by the contact offset to find the contact: the real separation is that of the shapes themselves, and
                // a contact within the speculative distance may still be open.
                let separation = point.separation + margin;
                // The contact is reported (the game's `OnCollisionEnter2D`) when the shapes are within the speculative distance or the step's motion
                // brings them to touching.
                let near = separation <= speculative || (config.catch_fast_approach && separation <= -normal_speed * dt);
                let reported = point_index == 0 && near;
                let event_index = if point_index > 0 { first_event } else if reported { contacts.len() as u32 } else { u32::MAX };
                first_event = event_index;
                if reported {
                contacts.push(ContactEvent {
                    body_a: pair.body_a as usize,
                    body_b: pair.body_b as usize,
                    tag_a: body_a.tag,
                    tag_b: body_b.tag,
                    collider_a: collider_a_index as u8,
                    collider_b: collider_b_index as u8,
                    normal: manifold.normal,
                    point: point.point,
                    separation: point.separation,
                    relative_velocity,
                    friction,
                    restitution,
                    ..ContactEvent::default()
                });
                }
                constraints.push(ContactConstraint {
                    body_a: pair.body_a,
                    body_b: pair.body_b,
                    normal: manifold.normal,
                    tangent,
                    anchor_a: ra,
                    anchor_b: rb,
                    adjusted_separation: separation - (rb - ra).dot(manifold.normal),
                    normal_mass: if k_normal > 0.0 { 1.0 / k_normal } else { 0.0 },
                    tangent_mass: if k_tangent > 0.0 {
                        1.0 / k_tangent
                    } else {
                        0.0
                    },
                    relative_velocity: normal_speed,
                    restitution,
                    normal_impulse: 0.0,
                    tangent_impulse: 0.0,
                    total_normal_impulse: 0.0,
                    max_normal_impulse: 0.0,
                    friction,
                    event: event_index,
                    near,
                    key: if body_a.tag <= body_b.tag {
                        ContactKey { tag_a: body_a.tag, tag_b: body_b.tag, collider_a: collider_a_index as u8, collider_b: collider_b_index as u8, point: point_index as u8 }
                    } else {
                        ContactKey { tag_a: body_b.tag, tag_b: body_a.tag, collider_a: collider_b_index as u8, collider_b: collider_a_index as u8, point: point_index as u8 }
                    },
                    flipped: body_a.tag > body_b.tag,
                    welded: false,
                });
                }
            }
        }
        // Of the parts of two bodies that a fast approach will bring together, the first to touch is the collision (the others touch only if the
        // bodies pass through each other): one report, as the game's one callback for the contact that stops them.
        if fast && contacts.len() > event_start + 1 {
            let nearest = contacts[event_start..].iter().map(|event| event.separation).fold(f32::INFINITY, f32::min);
            // (Bodies already overlapping have no first contact to pick: every part that touches is a collision.)
            if nearest + margin <= 0.0 {
                continue;
            }
            let mut kept: Vec<ContactEvent> = Vec::new();
            for constraint in &mut constraints[constraint_start..] {
                if constraint.event == u32::MAX {
                    continue;
                }
                let event = contacts[constraint.event as usize];
                if event.separation <= nearest + config.speculative_distance {
                    constraint.event = (event_start + kept.len()) as u32;
                    kept.push(event);
                } else {
                    constraint.event = u32::MAX;
                }
            }
            contacts.truncate(event_start);
            contacts.extend(kept);
        }
    }
}

/// FNV-1a hash over a sequence of u64 values (debug trace only).
fn trace_hash(values: impl IntoIterator<Item = u64>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for value in values {
        hash ^= value;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn trace_hash_f32(hash: u64, value: f32) -> u64 {
    let mut hash = hash ^ u64::from(value.to_bits());
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash
}

fn physics_trace_enabled() -> bool {
    std::env::var_os("RUSTIBITES_PHYS_TRACE").is_some()
}

fn trace_scratch(label: &str, scratch: &PhysicsScratch) {
    let pair_hash = trace_hash(
        scratch
            .pairs
            .iter()
            .flat_map(|pair| [u64::from(pair.body_a), u64::from(pair.body_b)]),
    );
    let contact_hash = trace_hash(scratch.contacts.iter().flat_map(|contact| {
        [
            contact.body_a as u64,
            contact.body_b as u64,
            contact.tag_a,
            contact.tag_b,
        ]
    }));
    let trigger_hash = trace_hash(scratch.triggers.iter().flat_map(|trigger| {
        [
            trigger.body_a as u64,
            trigger.body_b as u64,
            trigger.tag_a,
            trigger.tag_b,
        ]
    }));
    let mut constraint_hash = 0xcbf2_9ce4_8422_2325_u64;
    for constraint in &scratch.constraints {
        constraint_hash = trace_hash_f32(constraint_hash, constraint.normal_impulse);
        constraint_hash = trace_hash_f32(constraint_hash, constraint.tangent_impulse);
    }
    let mut solver_hash = 0xcbf2_9ce4_8422_2325_u64;
    for body in &scratch.solver {
        solver_hash = trace_hash_f32(solver_hash, body.pos.x);
        solver_hash = trace_hash_f32(solver_hash, body.pos.y);
        solver_hash = trace_hash_f32(solver_hash, body.vel.x);
        solver_hash = trace_hash_f32(solver_hash, body.vel.y);
        solver_hash = trace_hash_f32(solver_hash, body.ang_vel);
    }
    eprintln!(
        "PHYS_TRACE {label} pairs={} pair_hash={pair_hash:016x} contacts={} contact_hash={contact_hash:016x} constraints={} constraint_hash={constraint_hash:016x} triggers={} trigger_hash={trigger_hash:016x} solver_hash={solver_hash:016x}",
        scratch.pairs.len(),
        scratch.contacts.len(),
        scratch.constraints.len(),
        scratch.triggers.len()
    );
}

/// Runs one fixed physics step over `bodies` with timestep `dt`, leaving
/// contacts/triggers in `scratch`.
///
/// Bodies are integrated semi-implicitly (gravity `0`), contacts are resolved
/// with `velocity_iterations` sequential impulses plus `position_iterations`
/// position corrections. Sensor colliders only produce [`TriggerEvent`]s.
pub fn step(
    bodies: &mut [PhysicsBody],
    config: &PhysicsConfig,
    dt: f32,
    scratch: &mut PhysicsScratch,
) {
    scratch.clear();
    // The boxes the caller announced are for this step only, and so is what it does not want written back.
    let welds = std::mem::take(&mut scratch.welds_in);
    scratch.broken_welds.clear();
    scratch.new_welds.clear();
    let known = std::mem::take(&mut scratch.known_boxes);
    let skip_write_back = std::mem::take(&mut scratch.skip_write_back);
    if dt <= 0.0 || bodies.is_empty() {
        return;
    }
    let mut lap = Instant::now();

    // 1. Copy dynamic state into the solver mirror and integrate each body's velocity as it goes in (parallel over
    // contiguous body ranges; every body is its own and there are no reductions, so the result is bit-identical).
    let threads = scratch.effective_threads(bodies.len());
    let pool = scratch.threads;
    let min_sort = scratch.min_sort.max(1);
    let grid_from = if scratch.sweep_only {
        usize::MAX
    } else {
        grid_broadphase_from(scratch.min_grid, scratch.threads)
    };
    let sort_workers =
        |items: usize, from: usize| crate::sim::ParallelGates::workers_for(items, from, pool);
    let max_angular = config.max_rotation_speed.to_radians();
    let mirror = |body: &PhysicsBody, solved: &mut SolverBody| {
        let dynamic = body.active && !body.is_static;
        let com = if dynamic { centre_of_mass(body) } else { Vec2::ZERO };
        *solved = SolverBody {
            pos: if com == Vec2::ZERO { body.pos } else { body.pos + com.rotate(body.rot) },
            rot: body.rot,
            vel: body.vel,
            ang_vel: body.ang_vel,
            inv_mass: if dynamic { body.inv_mass } else { 0.0 },
            inv_inertia: if dynamic { body.inv_inertia } else { 0.0 },
            linear_damping: body.linear_damping,
            angular_damping: body.angular_damping,
            force: if dynamic { body.force } else { Vec2::ZERO },
            torque: if dynamic { body.torque } else { 0.0 },
            com,
            pos0: if com == Vec2::ZERO { body.pos } else { body.pos + com.rotate(body.rot) },
            rot0: body.rot,
            rot_tick: body.rot,
            touched: false,
            is_static: !dynamic,
        };
    };
    scratch.solver.clear();
    scratch.solver.resize(bodies.len(), SolverBody::default());
    if threads > 1 {
        let chunk_len = bodies.len().div_ceil(threads).max(1);
        rayon::scope(|scope| {
            for (body_chunk, solver_chunk) in bodies
                .chunks(chunk_len)
                .zip(scratch.solver.chunks_mut(chunk_len))
            {
                scope.spawn(move |_| {
                    for (body, solved) in body_chunk.iter().zip(solver_chunk.iter_mut()) {
                        mirror(body, solved);
                    }
                });
            }
        });
    } else {
        for (body, solved) in bodies.iter().zip(scratch.solver.iter_mut()) {
            mirror(body, solved);
        }
    }

    scratch.clock.lap(Stage::PhysMirror, &mut lap);
    // 2. The velocities were integrated with the copy (see above).
    scratch.clock.lap(Stage::PhysIntegrate, &mut lap);

    // 3. Broadphase: one box per body, gathered with the body's tag and layer while the body is in the cache. The caller may
    // have done the first ones already (the bibites, where it built the bodies).
    let known = known.min(bodies.len()).min(scratch.body_boxes.len());
    #[cfg(test)]
    for (index, body) in bodies[..known].iter().enumerate() {
        let (fresh, given) = (BodyBox::of(body), scratch.body_boxes[index]);
        let same_box = |a: &Aabb, b: &Aabb| {
            [a.min.x, a.min.y, a.max.x, a.max.y]
                .iter()
                .zip([b.min.x, b.min.y, b.max.x, b.max.y])
                .all(|(a, b)| a.to_bits() == b.to_bits())
        };
        assert!(
            fresh.valid == given.valid
                && fresh.tag == given.tag
                && fresh.layer == given.layer
                && same_box(&fresh.aabb, &given.aabb),
            "body {index}: the box the caller announced is not the body's box"
        );
    }
    if known > 0 {
        scratch.body_boxes.truncate(known);
        let rest = &bodies[known..];
        let workers = sort_workers(rest.len(), min_sort);
        if workers > 1 {
            let min_len = crate::sim::ParallelGates::chunk_len(rest.len(), min_sort, workers, 4);
            scratch
                .body_boxes
                .par_extend(rest.par_iter().with_min_len(min_len).map(BodyBox::of));
        } else {
            scratch.body_boxes.extend(rest.iter().map(BodyBox::of));
        }
    } else if threads > 1 {
        let min_len = bodies.len().div_ceil(threads).max(1);
        bodies
            .par_iter()
            .with_min_len(min_len)
            .map(BodyBox::of)
            .collect_into_vec(&mut scratch.body_boxes);
    } else {
        scratch.body_boxes.clear();
        scratch.body_boxes.extend(bodies.iter().map(BodyBox::of));
    }
    scratch.clock.lap(Stage::PhysAabb, &mut lap);
    // The grid finds the pairs of a big, spread-out world without the x order; anything else (and any world it will not answer for) takes the sweep.
    if grid_broadphase(scratch, grid_from) {
        scratch.clock.lap(Stage::PhysOrder, &mut lap);
        scratch.clock.lap(Stage::PhysBroadphase, &mut lap);
    } else {
        sweep_broadphase(scratch, pool, min_sort, &mut lap);
    }
    let trace = physics_trace_enabled();
    if trace {
        trace_scratch("broadphase", scratch);
    }
    // Deterministic pair order independent of insertion history (the tags travel in the pair: no lookup per comparison).
    if sort_workers(scratch.pairs.len(), min_sort.saturating_mul(2)) > 1 {
        scratch
            .pairs
            .par_sort_unstable_by_key(|pair| (pair.tag_a, pair.tag_b, pair.body_a, pair.body_b));
    } else {
        scratch
            .pairs
            .sort_unstable_by_key(|pair| (pair.tag_a, pair.tag_b, pair.body_a, pair.body_b));
    }

    scratch.clock.lap(Stage::PhysPairSort, &mut lap);
    // Cache each collider's world pose/AABB once; the pair loop otherwise recomputes them per candidate pair (a dense-cluster
    // hotspot). Only the bodies a candidate pair names are looked at later (a seventh of them at 100,000 bibites), so only
    // theirs are computed, and the arrays keep whatever they held for the rest (nothing reads it).
    let collider_slots = bodies.len() * MAX_COLLIDERS;
    if scratch.collider_aabbs.len() < collider_slots {
        scratch
            .collider_aabbs
            .resize(collider_slots, Aabb::default());
    }
    if scratch.collider_poses.len() < collider_slots {
        scratch
            .collider_poses
            .resize(collider_slots, Pose::new(Vec2::ZERO, 0.0));
    }
    // The (dead) supplemental sensor pass reads the boxes of every sensor collider.
    let everything = scratch.dense_cap_hit && SENSOR_CANDIDATE_CAP != 0;
    scratch.pair_bodies.clear();
    scratch.pair_bodies.resize(bodies.len(), everything);
    for pair in &scratch.pairs {
        scratch.pair_bodies[pair.body_a as usize] = true;
        scratch.pair_bodies[pair.body_b as usize] = true;
    }
    let named = &scratch.pair_bodies;
    if threads > 1 {
        let chunk_len = bodies.len().div_ceil(threads).max(1);
        rayon::scope(|scope| {
            for (((body_chunk, named_chunk), pose_chunk), aabb_chunk) in bodies
                .chunks(chunk_len)
                .zip(named.chunks(chunk_len))
                .zip(scratch.collider_poses[..collider_slots].chunks_mut(chunk_len * MAX_COLLIDERS))
                .zip(scratch.collider_aabbs[..collider_slots].chunks_mut(chunk_len * MAX_COLLIDERS))
            {
                scope.spawn(move |_| {
                    for (offset, body) in body_chunk.iter().enumerate() {
                        if !named_chunk[offset] {
                            continue;
                        }
                        let pose = body.pose();
                        for (collider_index, collider) in body.colliders().iter().enumerate() {
                            let slot = offset * MAX_COLLIDERS + collider_index;
                            pose_chunk[slot] = collider.world_pose(pose);
                            aabb_chunk[slot] = collider.aabb(pose);
                        }
                    }
                });
            }
        });
    } else {
        for (body_index, body) in bodies.iter().enumerate() {
            if !named[body_index] {
                continue;
            }
            let pose = body.pose();
            for (collider_index, collider) in body.colliders().iter().enumerate() {
                let slot = body_index * MAX_COLLIDERS + collider_index;
                scratch.collider_poses[slot] = collider.world_pose(pose);
                scratch.collider_aabbs[slot] = collider.aabb(pose);
            }
        }
    }

    scratch.clock.lap(Stage::PhysPoses, &mut lap);
    // 4. Narrowphase: contacts and trigger overlaps. Dense contact piles run
    // independent pair chunks in parallel; chunks are merged back in pair order
    // so contacts/constraints/triggers are bit-identical to the serial loop.
    let narrow = {
        let pairs = &scratch.pairs;
        let solver = &scratch.solver;
        let collider_aabbs = &scratch.collider_aabbs;
        let collider_poses = &scratch.collider_poses;
        let chunk_count = crate::sim::ParallelGates::workers_for(pairs.len(), scratch.min_narrow_pairs, scratch.threads);
        if chunk_count > 1 {
            let bodies_shared: &[PhysicsBody] = bodies;
            let chunk_size = pairs.len().div_ceil(chunk_count);
            // An indexed parallel iterator collects in pair order, whichever worker finished first.
            let chunks: Vec<NarrowOutput> = pairs
                .par_chunks(chunk_size)
                .map(|chunk| {
                    narrow_phase(
                        chunk,
                        bodies_shared,
                        solver,
                        collider_aabbs,
                        collider_poses,
                        config,
                        dt,
                    )
                })
                .collect();
            let mut merged = NarrowOutput {
                contacts: Vec::with_capacity(chunks.iter().map(|c| c.contacts.len()).sum()),
                constraints: Vec::with_capacity(chunks.iter().map(|c| c.constraints.len()).sum()),
                triggers: Vec::with_capacity(chunks.iter().map(|c| c.triggers.len()).sum()),
            };
            for chunk in chunks {
                let base = merged.contacts.len() as u32;
                merged.contacts.extend(chunk.contacts);
                merged
                    .constraints
                    .extend(chunk.constraints.into_iter().map(|mut c| {
                        if c.event != u32::MAX {
                            c.event += base;
                        }
                        c
                    }));
                merged.triggers.extend(chunk.triggers);
            }
            merged
        } else {
            narrow_phase(
                pairs,
                bodies,
                solver,
                collider_aabbs,
                collider_poses,
                config,
                dt,
            )
        }
    };
    scratch.contacts.clear();
    scratch.contacts.extend(narrow.contacts);
    scratch.constraints.clear();
    scratch.constraints.extend(narrow.constraints);
    scratch.triggers.clear();
    scratch.triggers.extend(narrow.triggers);
    if scratch.dense_cap_hit {
        append_sensor_triggers(bodies, scratch, config);
    }

    if trace {
        trace_scratch("narrowphase", scratch);
    }
    scratch.clock.lap(Stage::PhysNarrow, &mut lap);
    // The mouths that want to grab, and what their triggers may meet during the ten steps: those pairs are stepped (the game's mouth sees a trigger enter at
    // some physics step and grabs in the next `FixedUpdate`). The mouths that hold something are followed too (what leaves the trigger is let go).
    // With one step per tick the plain solver below knows no joints: a tick with a weld, or a mouth that wants to grab, goes through the island solver (one step).
    let island_mode = config.fine_steps > 1 || !welds.is_empty() || bodies.iter().any(|body| body.grab_slots > 0);
    let mut grabs: Vec<GrabPair> = Vec::new();
    if island_mode {
        let sensor_of = |body: &PhysicsBody| body.colliders().iter().position(|collider| collider.is_sensor);
        let mut welded: Vec<(u32, u32)> = welds.iter().map(|weld| (weld.holder, weld.held)).collect();
        welded.sort_unstable();
        for weld in &welds {
            if let Some(sensor) = sensor_of(&bodies[weld.holder as usize]) {
                grabs.push(GrabPair { holder: weld.holder, target: weld.held, sensor: sensor as u8, welded: true });
            }
        }
        for pair in &scratch.pairs {
            for (holder, target) in [(pair.body_a, pair.body_b), (pair.body_b, pair.body_a)] {
                let (holder_body, target_body) = (&bodies[holder as usize], &bodies[target as usize]);
                if holder_body.grab_slots == 0 || !target_body.grabbable || holder_body.tag == target_body.tag || !collision_allowed(holder_body.layer, target_body.layer) {
                    continue;
                }
                if welded.binary_search(&(holder, target)).is_ok() {
                    continue;
                }
                let Some(sensor) = sensor_of(holder_body) else {
                    continue;
                };
                let speed = (scratch.solver[target as usize].vel - scratch.solver[holder as usize].vel).length();
                let margin = (speed * dt).min(12.0) + config.contact_offset + config.speculative_distance;
                let mouth = scratch.collider_aabbs[holder as usize * MAX_COLLIDERS + sensor];
                let reach = Aabb { min: mouth.min - Vec2::splat(margin), max: mouth.max + Vec2::splat(margin) };
                if (0..target_body.colliders().len()).any(|collider| reach.overlaps(&scratch.collider_aabbs[target as usize * MAX_COLLIDERS + collider])) {
                    grabs.push(GrabPair { holder, target, sensor: sensor as u8, welded: false });
                }
            }
        }
    }
    // 5. The bodies no contact holds move in closed form (the ten slices of damping and motion of a free body, see `sliced_damping`); the ones a contact
    // holds go through the soft-step substeps below.
    for constraint in &scratch.constraints {
        if constraint.normal_mass == 0.0 || (island_mode && !constraint.near) {
            continue;
        }
        scratch.solver[constraint.body_a as usize].touched = true;
        scratch.solver[constraint.body_b as usize].touched = true;
    }
    if island_mode {
        for weld in &welds {
            scratch.solver[weld.held as usize].touched = true;
            scratch.solver[weld.holder as usize].touched = true;
        }
        for grab in &grabs {
            scratch.solver[grab.holder as usize].touched = true;
            scratch.solver[grab.target as usize].touched = true;
        }
    }
    let free_motion = |body: &mut SolverBody| {
        if body.touched {
            return;
        }
        integrate_free(body, config, dt, max_angular);
    };
    if trace {
        for body in scratch.solver.iter_mut() {
            free_motion(body);
        }
    } else if threads > 1 {
        let chunk_len = scratch.solver.len().div_ceil(threads).max(1);
        scratch.solver.par_chunks_mut(chunk_len).for_each(|chunk| {
            for body in chunk {
                free_motion(body);
            }
        });
    } else {
        for body in scratch.solver.iter_mut() {
            free_motion(body);
        }
    }
    if !island_mode {
        if !scratch.constraints.is_empty() {
            solve_soft_step(scratch, config, dt, max_angular, if trace { 1 } else { threads });
        }
    } else if !scratch.constraints.is_empty() || !welds.is_empty() || !grabs.is_empty() {
        fine_steps(bodies, scratch, config, dt, max_angular, if trace { 1 } else { threads }, &welds, &grabs);
    }
    // What the contacts ended with is what the next step warm starts from (a contact that is gone loses its impulses).
    if island_mode {
        // (The fine steps carry their own.)
        if scratch.constraints.is_empty() && welds.is_empty() && grabs.is_empty() {
            scratch.carried.clear();
        }
    } else {
    scratch.carried_next.clear();
    for constraint in &scratch.constraints {
        if constraint.normal_mass == 0.0 {
            continue;
        }
        scratch.carried_next.push(CarriedImpulse {
            key: constraint.key,
            normal: constraint.normal_impulse,
            tangent: if constraint.flipped { -constraint.tangent_impulse } else { constraint.tangent_impulse },
        });
    }
    scratch.carried_next.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    scratch.carried_next.dedup_by(|a, b| a.key == b.key);
    std::mem::swap(&mut scratch.carried, &mut scratch.carried_next);
    }
    if trace {
        trace_scratch("position-solve", scratch);
    }
    scratch.clock.lap(Stage::PhysSolve, &mut lap);
    // The bodies are back at their own positions (the solver moved their centres of mass).
    let to_own_position = |body: &mut SolverBody| {
        if body.com != Vec2::ZERO {
            body.pos -= body.com.rotate(body.rot);
        }
    };
    if threads > 1 && !trace {
        let chunk_len = scratch.solver.len().div_ceil(threads).max(1);
        scratch.solver.par_chunks_mut(chunk_len).for_each(|chunk| {
            for body in chunk {
                to_own_position(body);
            }
        });
    } else {
        for body in scratch.solver.iter_mut() {
            to_own_position(body);
        }
    }
    // 8. Publish impulse stats and write the solver state back.
    for constraint in &scratch.constraints {
        if let Some(event) = scratch.contacts.get_mut(constraint.event as usize) {
            event.normal_impulse += constraint.total_normal_impulse;
            event.tangent_impulse += constraint.tangent_impulse;
        }
    }
    let skip = skip_write_back.min(bodies.len());
    let (written, solved) = (&mut bodies[skip..], &scratch.solver[skip..]);
    if threads > 1 {
        let chunk_len = written.len().div_ceil(threads).max(1);
        rayon::scope(|scope| {
            for (body_chunk, solved_chunk) in
                written.chunks_mut(chunk_len).zip(solved.chunks(chunk_len))
            {
                scope.spawn(move |_| {
                    for (body, solved) in body_chunk.iter_mut().zip(solved_chunk.iter()) {
                        if !body.active || body.is_static {
                            continue;
                        }
                        body.pos = solved.pos;
                        body.rot = solved.rot;
                        body.vel = solved.vel;
                        body.ang_vel = solved.ang_vel;
                    }
                });
            }
        });
    } else {
        for (body, solved) in written.iter_mut().zip(solved.iter()) {
            if !body.active || body.is_static {
                continue;
            }
            body.pos = solved.pos;
            body.rot = solved.rot;
            body.vel = solved.vel;
            body.ang_vel = solved.ang_vel;
        }
    }
    scratch.clock.lap(Stage::PhysWriteBack, &mut lap);
}

/// Islands in a chunk of the pool's work on the islands of a step (an island is a few bodies: a chunk of fewer costs more to hand over
/// than to solve). With fewer than twice this many islands the step solves them on the calling thread.
const ISLANDS_PER_CHUNK: usize = 8;

/// The bodies the tick's near contacts hold, stepped `config.fine_steps` times (the game's ten physics steps per tick) with the contacts found again at
/// every step. Bodies that touch, directly or through others, form islands that do not influence each other; each island is stepped by itself (on the
/// pool when there are many), its colliders' poses, the contacts of its candidate pairs and the soft step of one substep made again at every step, its
/// muscles' force turning with the body from step to step. The impulses of the steps are added to the contact events of the tick, and the last step's
/// impulses are what the next tick warm starts from.
fn fine_steps(bodies: &[PhysicsBody], scratch: &mut PhysicsScratch, config: &PhysicsConfig, dt: f32, max_angular: f32, threads: usize, welds: &[WeldDef], grabs: &[GrabPair]) {
    let min_island_bodies = scratch.min_island_bodies.max(1);
    // One step per tick (the game at 1x) with the Box2D 2.4 solver: the same islands, one step, the rigid solver and rigid joints.
    let one_step = config.fine_steps <= 1;
    let steps = if one_step { 1 } else { config.fine_steps.max(2) };
    let h = dt / steps as f32;
    let inv_h = 1.0 / h;
    let rigid_solver = config.box2d_solver && one_step;
    let contact_hertz = if rigid_solver { 0.0 } else { config.contact_hertz.min(0.25 * inv_h) };
    let params = IslandParams {
        substeps: 1,
        h,
        inv_h,
        soft: make_soft(contact_hertz, config.contact_damping_ratio, h),
        soft_static: make_soft(2.0 * contact_hertz, config.contact_damping_ratio, h),
        push_speed: config.contact_push_speed,
        max_translation_speed: config.max_translation_speed,
        max_angular,
        bounce_threshold: config.bounce_threshold,
        joint_soft: if rigid_solver { make_soft(0.0, JOINT_DAMPING_RATIO, h) } else { make_soft(JOINT_HERTZ, JOINT_DAMPING_RATIO, h) },
        rigid: rigid_solver.then_some(RigidSolver {
            velocity_iterations: config.velocity_iterations.max(1),
            position_iterations: config.position_iterations,
            baumgarte: config.baumgarte,
            max_linear_correction: config.max_linear_correction,
            linear_slop: config.linear_slop,
        }),
    };
    let global = &mut scratch.solver;
    let constraints = &scratch.constraints;
    let holds = |constraint: &ContactConstraint| constraint.normal_mass != 0.0 && constraint.near;
    // The bodies the near contacts hold, once each, and their places.
    let mut place = vec![u32::MAX; global.len()];
    let mut held: Vec<u32> = Vec::new();
    for constraint in constraints.iter().filter(|constraint| holds(constraint)) {
        for body in [constraint.body_a, constraint.body_b] {
            if place[body as usize] == u32::MAX {
                place[body as usize] = held.len() as u32;
                held.push(body);
            }
        }
    }
    for weld in welds {
        for body in [weld.holder, weld.held] {
            if place[body as usize] == u32::MAX {
                place[body as usize] = held.len() as u32;
                held.push(body);
            }
        }
    }
    for grab in grabs {
        for body in [grab.holder, grab.target] {
            if place[body as usize] == u32::MAX {
                place[body as usize] = held.len() as u32;
                held.push(body);
            }
        }
    }
    if held.is_empty() {
        scratch.carried.clear();
        return;
    }
    let mut parent: Vec<u32> = (0..held.len() as u32).collect();
    fn find(parent: &mut [u32], mut node: u32) -> u32 {
        while parent[node as usize] != node {
            let up = parent[parent[node as usize] as usize];
            parent[node as usize] = up;
            node = up;
        }
        node
    }
    for constraint in constraints.iter().filter(|constraint| holds(constraint)) {
        let a = find(&mut parent, place[constraint.body_a as usize]);
        let b = find(&mut parent, place[constraint.body_b as usize]);
        if a != b {
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    for weld in welds {
        let a = find(&mut parent, place[weld.holder as usize]);
        let b = find(&mut parent, place[weld.held as usize]);
        if a != b {
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    for grab in grabs {
        let a = find(&mut parent, place[grab.holder as usize]);
        let b = find(&mut parent, place[grab.target as usize]);
        if a != b {
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    let mut island_of_root = vec![u32::MAX; held.len()];
    let mut island_of_place = vec![0u32; held.len()];
    let mut island_bodies: Vec<u32> = Vec::new();
    for p in 0..held.len() {
        let root = find(&mut parent, p as u32) as usize;
        if island_of_root[root] == u32::MAX {
            island_of_root[root] = island_bodies.len() as u32;
            island_bodies.push(0);
        }
        island_of_place[p] = island_of_root[root];
        island_bodies[island_of_root[root] as usize] += 1;
    }
    let island_count = island_bodies.len();
    let mut body_start = vec![0u32; island_count + 1];
    for i in 0..island_count {
        body_start[i + 1] = body_start[i] + island_bodies[i];
    }
    let mut new_place = vec![0u32; held.len()];
    let mut local: Vec<SolverBody> = vec![SolverBody::default(); held.len()];
    let mut local_bodies: Vec<PhysicsBody> = Vec::with_capacity(held.len());
    let mut next_body = body_start.clone();
    let mut order: Vec<u32> = vec![0; held.len()];
    for p in 0..held.len() {
        let island = island_of_place[p] as usize;
        new_place[p] = next_body[island];
        order[next_body[island] as usize] = p as u32;
        local[next_body[island] as usize] = global[held[p] as usize];
        next_body[island] += 1;
    }
    let mut global_of_local: Vec<u32> = Vec::with_capacity(held.len());
    for &p in &order {
        local_bodies.push(bodies[held[p as usize] as usize]);
        global_of_local.push(held[p as usize]);
    }
    // The candidate pairs of each island, with the bodies' places inside it (the constraints of a pair follow each other).
    let mut island_pairs: Vec<Vec<BodyPair>> = vec![Vec::new(); island_count];
    for constraint in constraints.iter().filter(|constraint| holds(constraint)) {
        let pa = place[constraint.body_a as usize] as usize;
        let pb = place[constraint.body_b as usize] as usize;
        let island = island_of_place[pa] as usize;
        let base = body_start[island];
        let (la, lb) = (new_place[pa] - base, new_place[pb] - base);
        let pairs = &mut island_pairs[island];
        if pairs.last().is_some_and(|last| last.body_a == la && last.body_b == lb) {
            continue;
        }
        pairs.push(BodyPair {
            body_a: la,
            body_b: lb,
            tag_a: local_bodies[new_place[pa] as usize].tag,
            tag_b: local_bodies[new_place[pb] as usize].tag,
        });
    }
    // The welds: their pair collides too (the game's joints are made with `enableCollision`), and what they carried from the last step.
    let mut island_welds: Vec<Vec<WeldState>> = vec![Vec::new(); island_count];
    for weld in welds {
        let (pa, pb) = (place[weld.holder as usize] as usize, place[weld.held as usize] as usize);
        let island = island_of_place[pa] as usize;
        let base = body_start[island];
        let (la, lb) = (new_place[pa] - base, new_place[pb] - base);
        let pairs = &mut island_pairs[island];
        if !pairs.iter().any(|pair| (pair.body_a == la && pair.body_b == lb) || (pair.body_a == lb && pair.body_b == la)) {
            pairs.push(BodyPair { body_a: la, body_b: lb, tag_a: local_bodies[new_place[pa] as usize].tag, tag_b: local_bodies[new_place[pb] as usize].tag });
        }
        let tag = local_bodies[new_place[pb] as usize].tag;
        let holder_tag = local_bodies[new_place[pa] as usize].tag;
        let (mut linear, mut angular) = (Vec2::ZERO, 0.0);
        if let Ok(at) = scratch.joint_carried.binary_search_by_key(&(tag, holder_tag), |entry| (entry.0, entry.1)) {
            (linear, angular) = (scratch.joint_carried[at].2, scratch.joint_carried[at].3);
        }
        island_welds[island].push(WeldState {
            holder: la,
            held: lb,
            anchor: weld.anchor,
            angle: weld.angle,
            break_force: weld.break_force,
            linear,
            angular,
            broken: false,
            held_global: weld.held,
            tag,
            holder_tag,
        });
    }
    // The grabs: a pair that may be grabbed collides too, and its mouth's trigger is tested at every step.
    let mut island_grabs: Vec<Vec<LocalGrab>> = vec![Vec::new(); island_count];
    for grab in grabs {
        let (pa, pb) = (place[grab.holder as usize] as usize, place[grab.target as usize] as usize);
        let island = island_of_place[pa] as usize;
        let base = body_start[island];
        let (la, lb) = (new_place[pa] - base, new_place[pb] - base);
        let pairs = &mut island_pairs[island];
        if !pairs.iter().any(|pair| (pair.body_a == la && pair.body_b == lb) || (pair.body_a == lb && pair.body_b == la)) {
            pairs.push(BodyPair { body_a: la, body_b: lb, tag_a: local_bodies[new_place[pa] as usize].tag, tag_b: local_bodies[new_place[pb] as usize].tag });
        }
        island_grabs[island].push(LocalGrab { holder: la, target: lb, sensor: grab.sensor, welded: grab.welded, inside: false });
    }
    let carried_global = std::mem::take(&mut scratch.carried);
    let mut outputs: Vec<IslandOutput> = Vec::new();
    {
        let mut pieces: Vec<(&mut [SolverBody], &[PhysicsBody], &[BodyPair], &[u32], &mut Vec<WeldState>, &mut Vec<LocalGrab>)> = Vec::with_capacity(island_count);
        let (mut solver_rest, mut bodies_rest, mut index_rest) = (&mut local[..], &local_bodies[..], &global_of_local[..]);
        for (island, (island_weld_list, island_grab_list)) in island_welds.iter_mut().zip(island_grabs.iter_mut()).enumerate() {
            let count = island_bodies[island] as usize;
            let (solver_part, rest_s) = solver_rest.split_at_mut(count);
            let (bodies_part, rest_b) = bodies_rest.split_at(count);
            let (index_part, rest_i) = index_rest.split_at(count);
            solver_rest = rest_s;
            bodies_rest = rest_b;
            index_rest = rest_i;
            pieces.push((solver_part, bodies_part, &island_pairs[island][..], index_part, island_weld_list, island_grab_list));
        }
        let carried = &carried_global[..];
        if threads > 1 && island_count >= 2 * ISLANDS_PER_CHUNK && held.len() >= min_island_bodies {
            use rayon::prelude::*;
            outputs = pieces
                .into_par_iter()
                .with_min_len(ISLANDS_PER_CHUNK)
                .map_init(IslandScratch::default, |work, (solver, island_bodies, pairs, indices, island_weld_list, island_grab_list)| {
                    run_island(work, solver, island_bodies, pairs, carried, indices, island_weld_list, island_grab_list, config, &params, steps)
                })
                .collect();
        } else {
            let mut work = IslandScratch::default();
            for (solver, island_bodies, pairs, indices, island_weld_list, island_grab_list) in pieces {
                outputs.push(run_island(&mut work, solver, island_bodies, pairs, carried, indices, island_weld_list, island_grab_list, config, &params, steps));
            }
        }
    }
    for p in 0..held.len() {
        global[held[p] as usize] = local[new_place[p] as usize];
    }
    // The grabs made inside the steps; the mouths' trigger lists are what the last step saw (a trigger the slice-0 pass found and the mouth had left, or one it
    // entered since, is corrected).
    let mut finals: Vec<(u32, u32, bool)> = Vec::new();
    for output in &outputs {
        scratch.new_welds.extend_from_slice(&output.new_welds);
        finals.extend_from_slice(&output.overlaps);
    }
    if !finals.is_empty() {
        finals.sort_unstable_by_key(|entry| (entry.0, entry.1));
        let known = |holder: usize, target: usize| finals.binary_search_by_key(&(holder as u32, target as u32), |entry| (entry.0, entry.1)).is_ok();
        for trigger in scratch.triggers.iter_mut() {
            if trigger.sensor_a && known(trigger.body_a, trigger.body_b) {
                trigger.sensor_a = false;
            }
            if trigger.sensor_b && known(trigger.body_b, trigger.body_a) {
                trigger.sensor_b = false;
            }
        }
        scratch.triggers.retain(|trigger| trigger.sensor_a || trigger.sensor_b);
        for &(holder, target, inside) in &finals {
            if inside {
                scratch.triggers.push(TriggerEvent {
                    body_a: holder as usize,
                    body_b: target as usize,
                    tag_a: bodies[holder as usize].tag,
                    tag_b: bodies[target as usize].tag,
                    sensor_a: true,
                    sensor_b: false,
                    ..TriggerEvent::default()
                });
            }
        }
    }
    // What the last step's contacts carried is what the next tick warm starts from.
    let mut carried_next = carried_global;
    carried_next.clear();
    for output in &outputs {
        carried_next.extend_from_slice(&output.carried);
    }
    carried_next.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    scratch.carried = carried_next;
    // What the welds carried is what the next tick warm starts them from; the ones that gave way are reported.
    scratch.joint_carried.clear();
    for list in &island_welds {
        for weld in list {
            if weld.broken {
                scratch.broken_welds.push((weld.held_global, weld.holder_tag));
            } else {
                scratch.joint_carried.push((weld.tag, weld.holder_tag, weld.linear, weld.angular));
            }
        }
    }
    scratch.joint_carried.sort_unstable_by_key(|entry| (entry.0, entry.1));
    // The contacts of the tick are those the steps found (a contact the tick's motion was thought to bring about, and did not, is none).
    scratch.contacts.clear();
    for output in &outputs {
        scratch.contacts.extend_from_slice(&output.events);
    }
}

/// The buffers one worker reuses for the islands it steps.
#[derive(Default)]
struct IslandScratch {
    poses: Vec<Pose>,
    aabbs: Vec<Aabb>,
    narrow: NarrowOutput,
    carried: Vec<CarriedImpulse>,
}

/// What an island's fine steps leave behind: the last step's impulses by contact and its contact events.
struct IslandOutput {
    carried: Vec<CarriedImpulse>,
    /// The island's contact events: each contact as it was when the step that first reported it began (its relative velocity is what the collision
    /// damage reads), with the impulses of all the steps added up. Bodies by index in the slice given to `step`.
    events: Vec<ContactEvent>,
    /// The grabs made in the steps.
    new_welds: Vec<NewWeld>,
    /// For each grab pair (holder, target by index in the slice given to `step`): whether the mouth's trigger overlapped the body at the start of the last step.
    overlaps: Vec<(u32, u32, bool)>,
}

/// Whether collider `sensor` of body `holder` overlaps any collider of body `target` (indices inside the island; `poses` and `aabbs` by collider slot).
fn sensor_overlaps(bodies: &[PhysicsBody], aabbs: &[Aabb], poses: &[Pose], holder: usize, target: usize, sensor: usize, skin: f32) -> bool {
    let mouth = &bodies[holder].colliders()[sensor];
    let (mouth_box, mouth_pose) = (aabbs[holder * MAX_COLLIDERS + sensor], poses[holder * MAX_COLLIDERS + sensor]);
    bodies[target].colliders().iter().enumerate().any(|(collider_index, collider)| {
        mouth_box.overlaps(&aabbs[target * MAX_COLLIDERS + collider_index])
            && collide_with_skin(&mouth.shape, mouth_pose, &collider.shape, poses[target * MAX_COLLIDERS + collider_index], skin).is_some()
    })
}

/// A body's origin (the solver keeps the centre of mass).
fn own_position(body: &SolverBody) -> Vec2 {
    if body.com == Vec2::ZERO { body.pos } else { body.pos - body.com.rotate(body.rot) }
}

/// One island's fine steps (see `fine_steps`): `solver` and `bodies` are the island's bodies, `pairs` its candidate pairs by their places in it.
fn run_island(
    work: &mut IslandScratch,
    solver: &mut [SolverBody],
    bodies: &[PhysicsBody],
    pairs: &[BodyPair],
    carried_global: &[CarriedImpulse],
    global_index: &[u32],
    welds: &mut Vec<WeldState>,
    grabs: &mut [LocalGrab],
    config: &PhysicsConfig,
    params: &IslandParams,
    steps: u32,
) -> IslandOutput {
    let slots = bodies.len() * MAX_COLLIDERS;
    let IslandScratch { poses, aabbs, narrow, carried } = work;
    poses.clear();
    poses.resize(slots, Pose::new(Vec2::ZERO, 0.0));
    aabbs.clear();
    aabbs.resize(slots, Aabb::default());
    carried.clear();
    let mut seen: Vec<(ContactEventKey, u32, ContactEvent)> = Vec::new();
    let mut new_welds: Vec<NewWeld> = Vec::new();
    let mut slots_left: Vec<u8> = if grabs.is_empty() { Vec::new() } else { bodies.iter().map(|body| body.grab_slots).collect() };
    for step in 0..steps {
        for (index, (body, source)) in solver.iter_mut().zip(bodies).enumerate() {
            body.pos0 = body.pos;
            body.rot0 = body.rot;
            let own = if body.com == Vec2::ZERO { body.pos } else { body.pos - body.com.rotate(body.rot) };
            let pose = Pose::new(own, body.rot);
            for (collider_index, collider) in source.colliders().iter().enumerate() {
                poses[index * MAX_COLLIDERS + collider_index] = collider.world_pose(pose);
                aabbs[index * MAX_COLLIDERS + collider_index] = collider.aabb(pose);
            }
        }
        if !grabs.is_empty() {
            // With ONE step per tick the game welds at the start of the step in which the trigger overlap shows (measured: a bibite driving at pellets of 60 /
            // 400 units from 18 u welds them at the pose of the start of that step, and they move with it in that same step; welding one step later left
            // the pellet where it was for 3-6 ticks and 0.9-3 u closer to the bibite): the overlap is read from the poses of this step's start.
            if steps == 1 {
                for grab in grabs.iter_mut() {
                    grab.inside = sensor_overlaps(bodies, aabbs, poses, grab.holder as usize, grab.target as usize, grab.sensor as usize, config.contact_offset);
                }
            }
            // The mouths grab what their triggers met at an earlier step (the game's `UpdateOrgan` runs before each physics step and the trigger events of
            // the step before it have arrived): the nearest first, while the mouth has links left; the pose at this moment is the one the joint freezes.
            if step > 0 || steps == 1 {
                let mut ready: Vec<(u32, f32, u64, usize)> = Vec::new();
                for (at, grab) in grabs.iter().enumerate() {
                    if grab.welded || !grab.inside || slots_left[grab.holder as usize] == 0 {
                        continue;
                    }
                    let mouth = poses[grab.holder as usize * MAX_COLLIDERS + grab.sensor as usize].pos;
                    ready.push((grab.holder, (own_position(&solver[grab.target as usize]) - mouth).length_sq(), bodies[grab.target as usize].tag, at));
                }
                ready.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
                for (holder, _, _, at) in ready {
                    if slots_left[holder as usize] == 0 {
                        continue;
                    }
                    slots_left[holder as usize] -= 1;
                    let target = grabs[at].target;
                    grabs[at].welded = true;
                    let (holder_body, target_body) = (&solver[holder as usize], &solver[target as usize]);
                    let anchor = (own_position(target_body) - own_position(holder_body)).rotate(-holder_body.rot);
                    let angle = target_body.rot - holder_body.rot;
                    welds.push(WeldState {
                        holder,
                        held: target,
                        anchor,
                        angle,
                        break_force: 0.0,
                        linear: Vec2::ZERO,
                        angular: 0.0,
                        broken: false,
                        held_global: global_index[target as usize],
                        tag: bodies[target as usize].tag,
                        holder_tag: bodies[holder as usize].tag,
                    });
                    new_welds.push(NewWeld { holder: global_index[holder as usize], held: global_index[target as usize], anchor, angle });
                    if config.held_drag_off && bodies[target as usize].layer == LAYER_PELLETS {
                        solver[target as usize].linear_damping = 0.0;
                    }
                }
            }
            for grab in grabs.iter_mut() {
                grab.inside = sensor_overlaps(bodies, aabbs, poses, grab.holder as usize, grab.target as usize, grab.sensor as usize, config.contact_offset);
            }
        }
        // A weld whose two anchors are far apart is not a weld of this pair (a holder that was wrapped across the world, or a body the weld was never made
        // with): the rigid solver takes the whole error out in one step, pulling the pair together across the distance, and what it leaves in the velocities
        // threw bodies out of the world (the glitch save of 2026-11-13 had welds with errors of 4,000 and 21,000 units). Such a weld is let go of, as a weld
        // that gave way is (the holder lets go). It cannot be a real one: the position solver takes the error out at every step, and a held body is where it
        // was a tick ago.
        for weld in welds.iter_mut() {
            if !weld.broken {
                let (ra, rb) = weld_arms(solver, weld);
                let (holder, held) = (&solver[weld.holder as usize], &solver[weld.held as usize]);
                let error = (held.pos + rb) - (holder.pos + ra);
                if !(error.length() <= WELD_MAX_ERROR) {
                    weld.broken = true;
                }
            }
        }
        narrow_phase_into(pairs, bodies, solver, aabbs, poses, config, params.h, true, narrow);
        let NarrowOutput { contacts: events, constraints, .. } = narrow;
        if !welds.is_empty() {
            // Two bodies are tied to each other when a weld joins them (the holder and what it holds: `collideConnected` is off for the game's joints) or when one holder
            // holds both (what a holder holds overlaps by construction and moves as one with it): none of their contacts pushes. The test is for each pair of bodies, not
            // for a body's first holder: a bibite that holds something and is itself held (a bibite holding a bibite that holds a pellet), or a body in two mouths, belongs
            // to more than one such set, and the contacts between its welded neighbours were left in the velocity solve; their carried impulses wound up by tens a tick
            // (819 to 1,108 in five ticks on the island of the "deadly pile" save of 2026-11-22) and the group threw what it touched.
            let tied = |a: u32, b: u32| {
                welds.iter().any(|weld| (weld.holder == a && weld.held == b) || (weld.holder == b && weld.held == a))
                    || welds.iter().any(|first| first.held == a && welds.iter().any(|second| second.held == b && second.holder == first.holder))
            };
            for constraint in constraints.iter_mut() {
                constraint.welded = tied(constraint.body_a, constraint.body_b);
            }
        }
        solve_island(solver, constraints, if step == 0 { carried_global } else { carried }, &mut welds[..], params);
        for weld in welds.iter_mut() {
            if !weld.broken && weld.break_force > 0.0 && (weld.linear.length() * params.inv_h > weld.break_force || weld.angular.abs() * params.inv_h > weld.break_force) {
                weld.broken = true;
            }
        }
        for constraint in constraints.iter() {
            if let Some(event) = events.get_mut(constraint.event as usize) {
                event.normal_impulse += constraint.total_normal_impulse;
                event.tangent_impulse += constraint.tangent_impulse;
            }
        }
        for event in events.iter() {
            let (key, _) = ContactEventKey::of(event);
            seen.push((key, step, *event));
        }
        carried.clear();
        for constraint in constraints.iter() {
            if constraint.normal_mass == 0.0 {
                continue;
            }
            carried.push(CarriedImpulse {
                key: constraint.key,
                normal: constraint.normal_impulse,
                tangent: if constraint.flipped { -constraint.tangent_impulse } else { constraint.tangent_impulse },
            });
        }
        carried.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        carried.dedup_by(|a, b| a.key == b.key);
    }
    // One event per contact: as the first step to report it saw it, the impulses of the steps added up (a contact keeps the orientation of its first report).
    seen.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut events: Vec<ContactEvent> = Vec::with_capacity(seen.len().min(16));
    let mut last_key: Option<ContactEventKey> = None;
    for (key, _, event) in seen {
        if last_key == Some(key) {
            let first = events.last_mut().expect("a first report");
            let same_way = first.tag_a == event.tag_a;
            first.normal_impulse += event.normal_impulse;
            first.tangent_impulse += if same_way { event.tangent_impulse } else { -event.tangent_impulse };
        } else {
            let mut event = event;
            event.body_a = global_index[event.body_a] as usize;
            event.body_b = global_index[event.body_b] as usize;
            events.push(event);
            last_key = Some(key);
        }
    }
    let overlaps = grabs.iter().map(|grab| (global_index[grab.holder as usize], global_index[grab.target as usize], grab.inside)).collect();
    IslandOutput { carried: carried.clone(), events, new_welds, overlaps }
}

/// What names a contact event: the two bodies' tags, smaller first, and the colliders' indices in that order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ContactEventKey {
    tag_a: u64,
    tag_b: u64,
    collider_a: u8,
    collider_b: u8,
}

impl ContactEventKey {
    /// The key of an event and whether it names the pair the other way round.
    fn of(event: &ContactEvent) -> (Self, bool) {
        if event.tag_a <= event.tag_b {
            (Self { tag_a: event.tag_a, tag_b: event.tag_b, collider_a: event.collider_a, collider_b: event.collider_b }, false)
        } else {
            (Self { tag_a: event.tag_b, tag_b: event.tag_a, collider_a: event.collider_b, collider_b: event.collider_a }, true)
        }
    }
}

/// Box2D's soft constraint coefficients (`b2MakeSoft`).
#[derive(Clone, Copy)]
struct Soft {
    bias_rate: f32,
    mass_scale: f32,
    impulse_scale: f32,
}

fn make_soft(hertz: f32, damping_ratio: f32, h: f32) -> Soft {
    if hertz == 0.0 {
        return Soft { bias_rate: 0.0, mass_scale: 1.0, impulse_scale: 0.0 };
    }
    let omega = 2.0 * std::f32::consts::PI * hertz;
    let a1 = 2.0 * damping_ratio + h * omega;
    let a2 = h * omega * a1;
    let a3 = 1.0 / (1.0 + a2);
    Soft { bias_rate: omega / a1, mass_scale: a2 * a3, impulse_scale: a3 }
}

/// One solve of every contact (`b2SolveContact`): the soft push-out and the speculative allowance when `use_bias`, the plain velocity
/// constraint (the relax pass) when not.
fn solve_contacts(
    solver: &mut [SolverBody],
    constraints: &mut [ContactConstraint],
    soft: Soft,
    soft_static: Soft,
    inv_h: f32,
    push_speed: f32,
    use_bias: bool,
) {
    for constraint in constraints.iter_mut() {
        if constraint.normal_mass == 0.0 {
            continue;
        }
        let (body_a, body_b) = two_mut(solver, constraint.body_a as usize, constraint.body_b as usize);
        let (ra, rb) = (constraint.anchor_a, constraint.anchor_b);
        // Where the bodies are now, from the motion since the step began.
        let delta_a = body_a.pos - body_a.pos0;
        let delta_b = body_b.pos - body_b.pos0;
        let pr_a = ra.rotate(body_a.rot - body_a.rot0);
        let pr_b = rb.rotate(body_b.rot - body_b.rot0);
        let d = (delta_b - delta_a) + (pr_b - pr_a);
        let s = d.dot(constraint.normal) + constraint.adjusted_separation;
        let (bias, mass_scale, impulse_scale) = if s > 0.0 {
            // Speculative: the bodies may close the gap, no further.
            (s * inv_h, 1.0, 0.0)
        } else if use_bias && !constraint.welded {
            let params = if body_a.inv_mass == 0.0 || body_b.inv_mass == 0.0 { soft_static } else { soft };
            ((params.bias_rate * s).max(-push_speed), params.mass_scale, params.impulse_scale)
        } else {
            (0.0, 1.0, 0.0)
        };
        let va = body_a.vel + ra.perp() * body_a.ang_vel;
        let vb = body_b.vel + rb.perp() * body_b.ang_vel;
        let normal_speed = (vb - va).dot(constraint.normal);
        let impulse = -constraint.normal_mass * mass_scale * (normal_speed + bias) - impulse_scale * constraint.normal_impulse;
        let old = constraint.normal_impulse;
        constraint.normal_impulse = (old + impulse).max(0.0);
        let applied = constraint.normal_impulse - old;
        constraint.max_normal_impulse = constraint.max_normal_impulse.max(applied);
        constraint.total_normal_impulse += constraint.normal_impulse;
        apply_pair_impulse(body_a, body_b, ra, rb, constraint.normal, applied);
        // Friction.
        let va = body_a.vel + ra.perp() * body_a.ang_vel;
        let vb = body_b.vel + rb.perp() * body_b.ang_vel;
        let tangent_speed = (vb - va).dot(constraint.tangent);
        let max_friction = constraint.friction * constraint.normal_impulse;
        let old_tangent = constraint.tangent_impulse;
        constraint.tangent_impulse = clamp(
            old_tangent - constraint.tangent_mass * tangent_speed,
            -max_friction,
            max_friction,
        );
        let applied_tangent = constraint.tangent_impulse - old_tangent;
        apply_pair_impulse(body_a, body_b, ra, rb, constraint.tangent, applied_tangent);
    }
}

/// The soft step (`b2SolverStage`s) for the bodies the contacts hold. The contacts tie the bodies into islands (bodies that touch, directly or through
/// others); the islands do not influence each other, so each is solved by itself (on the pool when there are many) with the same result as solving them
/// all in one list: see `solve_island`.
fn solve_soft_step(scratch: &mut PhysicsScratch, config: &PhysicsConfig, dt: f32, max_angular: f32, threads: usize) {
    let min_island_constraints = scratch.min_island_constraints.max(1);
    let substeps = config.substeps.max(1);
    let h = dt / substeps as f32;
    let inv_h = 1.0 / h;
    let contact_hertz = if config.box2d_solver { 0.0 } else { config.contact_hertz.min(0.25 * inv_h) };
    let params = IslandParams {
        substeps,
        h,
        inv_h,
        soft: make_soft(contact_hertz, config.contact_damping_ratio, h),
        soft_static: make_soft(2.0 * contact_hertz, config.contact_damping_ratio, h),
        push_speed: config.contact_push_speed,
        max_translation_speed: config.max_translation_speed,
        max_angular,
        bounce_threshold: config.bounce_threshold,
        joint_soft: make_soft(2.0 * contact_hertz, JOINT_DAMPING_RATIO, h),
        rigid: config.box2d_solver.then_some(RigidSolver {
            velocity_iterations: config.velocity_iterations.max(1),
            position_iterations: config.position_iterations,
            baumgarte: config.baumgarte,
            max_linear_correction: config.max_linear_correction,
            linear_slop: config.linear_slop,
        }),
    };
    let global = &mut scratch.solver;
    let constraints = &mut scratch.constraints;
    // The bodies the contacts hold, once each, and their places.
    let mut place = vec![u32::MAX; global.len()];
    let mut held: Vec<u32> = Vec::with_capacity(constraints.len() * 2);
    for constraint in constraints.iter() {
        if constraint.normal_mass == 0.0 {
            continue;
        }
        for body in [constraint.body_a, constraint.body_b] {
            if place[body as usize] == u32::MAX {
                place[body as usize] = held.len() as u32;
                held.push(body);
            }
        }
    }
    // Islands: union-find over the held bodies.
    let mut parent: Vec<u32> = (0..held.len() as u32).collect();
    fn find(parent: &mut [u32], mut node: u32) -> u32 {
        while parent[node as usize] != node {
            let up = parent[parent[node as usize] as usize];
            parent[node as usize] = up;
            node = up;
        }
        node
    }
    for constraint in constraints.iter() {
        if constraint.normal_mass == 0.0 {
            continue;
        }
        let a = find(&mut parent, place[constraint.body_a as usize]);
        let b = find(&mut parent, place[constraint.body_b as usize]);
        if a != b {
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    // Island numbers in order of first appearance among the held bodies; bodies and constraints sorted by island (the order inside an island is kept).
    let mut island_of_root = vec![u32::MAX; held.len()];
    let mut island_of_place = vec![0u32; held.len()];
    let mut island_bodies: Vec<u32> = Vec::new();
    for p in 0..held.len() {
        let root = find(&mut parent, p as u32) as usize;
        if island_of_root[root] == u32::MAX {
            island_of_root[root] = island_bodies.len() as u32;
            island_bodies.push(0);
        }
        let island = island_of_root[root];
        island_of_place[p] = island;
        island_bodies[island as usize] += 1;
    }
    let island_count = island_bodies.len();
    let mut body_start = vec![0u32; island_count + 1];
    for i in 0..island_count {
        body_start[i + 1] = body_start[i] + island_bodies[i];
    }
    let mut island_constraints = vec![0u32; island_count];
    for constraint in constraints.iter() {
        if constraint.normal_mass == 0.0 {
            continue;
        }
        island_constraints[island_of_place[place[constraint.body_a as usize] as usize] as usize] += 1;
    }
    let mut constraint_start = vec![0u32; island_count + 1];
    for i in 0..island_count {
        constraint_start[i + 1] = constraint_start[i] + island_constraints[i];
    }
    let mut new_place = vec![0u32; held.len()];
    let mut local: Vec<SolverBody> = vec![SolverBody::default(); held.len()];
    let mut next_body = body_start.clone();
    for p in 0..held.len() {
        let island = island_of_place[p] as usize;
        new_place[p] = next_body[island];
        local[next_body[island] as usize] = global[held[p] as usize];
        next_body[island] += 1;
    }
    let constraint_total = constraint_start[island_count] as usize;
    let mut sorted: Vec<ContactConstraint> = vec![ContactConstraint::default(); constraint_total];
    let mut origin: Vec<u32> = vec![0; constraint_total];
    let mut next_constraint = constraint_start.clone();
    for (index, constraint) in constraints.iter().enumerate() {
        if constraint.normal_mass == 0.0 {
            continue;
        }
        let p = place[constraint.body_a as usize] as usize;
        let island = island_of_place[p] as usize;
        let at = next_constraint[island] as usize;
        next_constraint[island] += 1;
        let base = body_start[island];
        let mut copy = *constraint;
        copy.body_a = new_place[p] - base;
        copy.body_b = new_place[place[constraint.body_b as usize] as usize] - base;
        sorted[at] = copy;
        origin[at] = index as u32;
    }
    {
        let carried = &scratch.carried[..];
        let mut pieces: Vec<(&mut [SolverBody], &mut [ContactConstraint])> = Vec::with_capacity(island_count);
        let (mut bodies_rest, mut cons_rest) = (&mut local[..], &mut sorted[..]);
        for island in 0..island_count {
            let (bodies, rest_b) = bodies_rest.split_at_mut(island_bodies[island] as usize);
            let (cons, rest_c) = cons_rest.split_at_mut(island_constraints[island] as usize);
            bodies_rest = rest_b;
            cons_rest = rest_c;
            pieces.push((bodies, cons));
        }
        if threads > 1 && constraint_total >= min_island_constraints && island_count >= 2 * ISLANDS_PER_CHUNK {
            use rayon::prelude::*;
            pieces.into_par_iter().with_min_len(ISLANDS_PER_CHUNK).for_each(|(bodies, cons)| solve_island(bodies, cons, carried, &mut [], &params));
        } else {
            for (bodies, cons) in pieces {
                solve_island(bodies, cons, carried, &mut [], &params);
            }
        }
    }
    for p in 0..held.len() {
        global[held[p] as usize] = local[new_place[p] as usize];
    }
    for (at, solved) in sorted.iter().enumerate() {
        let target = &mut constraints[origin[at] as usize];
        let (body_a, body_b) = (target.body_a, target.body_b);
        *target = *solved;
        target.body_a = body_a;
        target.body_b = body_b;
    }
}

/// What an island's substeps need.
#[derive(Clone, Copy)]
struct IslandParams {
    substeps: u32,
    h: f32,
    inv_h: f32,
    soft: Soft,
    soft_static: Soft,
    push_speed: f32,
    max_translation_speed: f32,
    max_angular: f32,
    bounce_threshold: f32,
    joint_soft: Soft,
    /// The velocity-iteration + Baumgarte position solver of Box2D 2.4 (the game at one physics step per tick) instead of the soft step: no push-out velocity, the
    /// positions integrated with the solved velocities, then the overlap removed by position iterations.
    rigid: Option<RigidSolver>,
}

/// See [`IslandParams::rigid`].
#[derive(Clone, Copy)]
struct RigidSolver {
    velocity_iterations: u32,
    position_iterations: u32,
    baumgarte: f32,
    max_linear_correction: f32,
    linear_slop: f32,
}

/// The soft step of one island: the warm start from the impulses the same contacts ended the last step with; then for each substep the velocities
/// are integrated (force, damping, speed limits), the carried impulses applied, the contacts solved with their soft push-out, the positions integrated, and the
/// contacts relaxed; after the substeps the restitution is applied.
fn solve_island(solver: &mut [SolverBody], constraints: &mut [ContactConstraint], carried: &[CarriedImpulse], welds: &mut [WeldState], params: &IslandParams) {
    let IslandParams { substeps, h, inv_h, soft, soft_static, push_speed, max_translation_speed, max_angular, bounce_threshold, joint_soft, rigid } = *params;
    if !carried.is_empty() {
        for constraint in constraints.iter_mut() {
            if let Ok(at) = carried.binary_search_by(|entry| entry.key.cmp(&constraint.key)) {
                let entry = carried[at];
                constraint.normal_impulse = entry.normal;
                constraint.tangent_impulse = if constraint.flipped { -entry.tangent } else { entry.tangent };
            }
        }
    }
    if let Some(rigid) = rigid {
        // The bodies one holder holds (and the holder) are tied to each other by welds, and none of their contacts pushes (`constraint.welded`: the soft step leaves out their
        // push-out bias and the position solve below skips them). A weld anchors what is held at the holder's origin, so the pair overlaps by construction, and the weld is
        // rigid: their contacts have nothing to push apart. Left in the velocity solve they fought the weld, and the impulses they carried from tick to tick wound up by a
        // hundred or more a tick: a holder that turned (its spin on the clamp) in a pile of bodies threw the light ones near it at the speed limit, and what they hit died (the
        // glitch save of 2026-11-20; a reload cleared it, as the carried impulses are not saved). They are out of the solve altogether, with the impulses they carried. Which
        // pairs are tied is decided where the contacts are made (a weld joins them, or one holder holds both): the first version looked only at a body's first holder, and a
        // bibite that held a bibite while another held it, or a body in two mouths, kept its contacts (the glitch save of 2026-11-22).
        for constraint in constraints.iter_mut().filter(|constraint| constraint.welded) {
            constraint.normal_mass = 0.0;
            constraint.normal_impulse = 0.0;
            constraint.tangent_impulse = 0.0;
        }
        // Box2D 2.4's step: integrate the velocities, warm start, the velocity iterations, integrate the positions with the solved velocities, the position iterations.
        for body in solver.iter_mut() {
            if body.is_static || body.inv_mass == 0.0 {
                continue;
            }
            body.vel = (body.vel + turned_by(body.force, body.rot - body.rot_tick) * (h * body.inv_mass)) * (1.0 / (1.0 + h * body.linear_damping));
            body.ang_vel = (body.ang_vel + body.torque * (h * body.inv_inertia)) * (1.0 / (1.0 + h * body.angular_damping));
            body.vel = body.vel.clamp_length(max_translation_speed);
            body.ang_vel = clamp(body.ang_vel, -max_angular, max_angular);
        }
        for weld in welds.iter().filter(|weld| !weld.broken) {
            warm_start_weld(solver, weld);
        }
        for constraint in constraints.iter() {
            let (body_a, body_b) = two_mut(solver, constraint.body_a as usize, constraint.body_b as usize);
            apply_pair_impulse(body_a, body_b, constraint.anchor_a, constraint.anchor_b, constraint.normal, constraint.normal_impulse);
            apply_pair_impulse(body_a, body_b, constraint.anchor_a, constraint.anchor_b, constraint.tangent, constraint.tangent_impulse);
        }
        // `b2Island::Solve`: every velocity iteration solves the joints and then the contacts. The welds used to be solved once, before the iterations (the soft step's order),
        // and a group of welds that is not a tree (a body held by two mouths ties two holders together, a bibite holds a bibite that holds a pellet, ...) does not come out of
        // one pass with its bodies moving together: the pass left the welded bodies' velocities up to hundreds of units a second apart at the end of every tick (the "deadly
        // pile" save of 2026-11-22: a pile of bibites and pellets that threw what touched it, quiet again after a reload).
        for _ in 0..rigid.velocity_iterations {
            for weld in welds.iter_mut().filter(|weld| !weld.broken) {
                solve_weld(solver, weld, joint_soft, true);
            }
            solve_contacts(solver, constraints, soft, soft_static, inv_h, push_speed, false);
        }
        // (The iterations added the accumulated impulse each time: the step's impulse is the accumulated one.)
        for constraint in constraints.iter_mut() {
            constraint.total_normal_impulse = constraint.normal_impulse;
        }
        apply_restitution(solver, constraints, bounce_threshold);
        for body in solver.iter_mut() {
            if body.is_static || body.inv_mass == 0.0 {
                continue;
            }
            body.vel = body.vel.clamp_length(max_translation_speed);
            body.ang_vel = clamp(body.ang_vel, -max_angular, max_angular);
            body.pos += body.vel * h;
            body.rot += body.ang_vel * h;
        }
        for weld in welds.iter_mut().filter(|weld| !weld.broken) {
            solve_weld(solver, weld, joint_soft, false);
        }
        if !welds.is_empty() {
            // The limits hold for what the relax pass left too: it works on velocities after they were clamped, and a body that a weld pulls at a long lever must still not leave
            // the step faster than the physics lets anything move.
            for body in solver.iter_mut() {
                if body.is_static || body.inv_mass == 0.0 {
                    continue;
                }
                body.vel = body.vel.clamp_length(max_translation_speed);
                body.ang_vel = clamp(body.ang_vel, -max_angular, max_angular);
            }
        }
        for _ in 0..rigid.position_iterations {
            // (`b2WeldJoint::SolvePositionConstraints`, rigid: the whole error is taken out.)
            for weld in welds.iter().filter(|weld| !weld.broken) {
                solve_weld_position(solver, weld);
            }
            for constraint in constraints.iter() {
                if constraint.normal_mass == 0.0 || constraint.welded {
                    continue;
                }
                let (body_a, body_b) = two_mut(solver, constraint.body_a as usize, constraint.body_b as usize);
                let (ra, rb) = (constraint.anchor_a, constraint.anchor_b);
                let delta_a = body_a.pos - body_a.pos0;
                let delta_b = body_b.pos - body_b.pos0;
                let pr_a = ra.rotate(body_a.rot - body_a.rot0);
                let pr_b = rb.rotate(body_b.rot - body_b.rot0);
                let s = ((delta_b - delta_a) + (pr_b - pr_a)).dot(constraint.normal) + constraint.adjusted_separation;
                let c = (rigid.baumgarte * (s + rigid.linear_slop)).clamp(-rigid.max_linear_correction, 0.0);
                if c == 0.0 {
                    continue;
                }
                let (rn_a, rn_b) = (pr_a.cross(constraint.normal), pr_b.cross(constraint.normal));
                let k = body_a.inv_mass + body_b.inv_mass + body_a.inv_inertia * rn_a * rn_a + body_b.inv_inertia * rn_b * rn_b;
                if k <= 0.0 {
                    continue;
                }
                let impulse = -c / k;
                body_a.pos -= constraint.normal * (impulse * body_a.inv_mass);
                body_a.rot -= body_a.inv_inertia * rn_a * impulse;
                body_b.pos += constraint.normal * (impulse * body_b.inv_mass);
                body_b.rot += body_b.inv_inertia * rn_b * impulse;
            }
        }
    } else {
    for _ in 0..substeps {
        for body in solver.iter_mut() {
            if body.is_static || body.inv_mass == 0.0 {
                continue;
            }
            body.vel = (body.vel + turned_by(body.force, body.rot - body.rot_tick) * (h * body.inv_mass)) * (1.0 / (1.0 + h * body.linear_damping));
            body.ang_vel = (body.ang_vel + body.torque * (h * body.inv_inertia)) * (1.0 / (1.0 + h * body.angular_damping));
            body.vel = body.vel.clamp_length(max_translation_speed);
            body.ang_vel = clamp(body.ang_vel, -max_angular, max_angular);
        }
        // Warm start.
        for weld in welds.iter().filter(|weld| !weld.broken) {
            warm_start_weld(solver, weld);
        }
        for constraint in constraints.iter() {
            let (body_a, body_b) = two_mut(solver, constraint.body_a as usize, constraint.body_b as usize);
            apply_pair_impulse(body_a, body_b, constraint.anchor_a, constraint.anchor_b, constraint.normal, constraint.normal_impulse);
            apply_pair_impulse(body_a, body_b, constraint.anchor_a, constraint.anchor_b, constraint.tangent, constraint.tangent_impulse);
        }
        for weld in welds.iter_mut().filter(|weld| !weld.broken) {
            solve_weld(solver, weld, joint_soft, true);
        }
        solve_contacts(solver, constraints, soft, soft_static, inv_h, push_speed, true);
        for body in solver.iter_mut() {
            if body.is_static || body.inv_mass == 0.0 {
                continue;
            }
            body.pos += body.vel * h;
            body.rot += body.ang_vel * h;
        }
        for weld in welds.iter_mut().filter(|weld| !weld.broken) {
            solve_weld(solver, weld, joint_soft, false);
        }
        solve_contacts(solver, constraints, soft, soft_static, inv_h, push_speed, false);
        // The limits hold for what the solver left.
        for body in solver.iter_mut() {
            if body.is_static || body.inv_mass == 0.0 {
                continue;
            }
            body.vel = body.vel.clamp_length(max_translation_speed);
            body.ang_vel = clamp(body.ang_vel, -max_angular, max_angular);
        }
    }
    }
    if rigid.is_none() {
        apply_restitution(solver, constraints, bounce_threshold);
    }
}

/// `b2ApplyRestitution`: the bounce of the contacts that closed fast enough (after the soft step; in the 2.4 solver before the positions are integrated).
fn apply_restitution(solver: &mut [SolverBody], constraints: &mut [ContactConstraint], bounce_threshold: f32) {
    for constraint in constraints.iter_mut() {
        if constraint.restitution == 0.0 || constraint.relative_velocity > -bounce_threshold || constraint.max_normal_impulse == 0.0 {
            continue;
        }
        let (body_a, body_b) = two_mut(solver, constraint.body_a as usize, constraint.body_b as usize);
        let (ra, rb) = (constraint.anchor_a, constraint.anchor_b);
        let va = body_a.vel + ra.perp() * body_a.ang_vel;
        let vb = body_b.vel + rb.perp() * body_b.ang_vel;
        let normal_speed = (vb - va).dot(constraint.normal);
        let impulse = -constraint.normal_mass * (normal_speed + constraint.restitution * constraint.relative_velocity);
        let old = constraint.normal_impulse;
        constraint.normal_impulse = (old + impulse).max(0.0);
        let applied = constraint.normal_impulse - old;
        constraint.max_normal_impulse = constraint.max_normal_impulse.max(applied);
        constraint.total_normal_impulse += applied;
        apply_pair_impulse(body_a, body_b, ra, rb, constraint.normal, applied);
    }
}

/// `b2_defaultJointDampingRatio`.
/// A weld's softness is Box2D's own for joints (`jointHertz` 60, `jointDampingRatio` 2), not the contacts' stiffness: at 1500 Hz a light body welded to a heavy one at a
/// long lever (a bibite holding a 1900-unit pellet) oscillated with growing amplitude.
const JOINT_HERTZ: f32 = 60.0;
const JOINT_DAMPING_RATIO: f32 = 2.0;

/// The distance (units) between a weld's two anchors above which the weld is let go of instead of solved (see `run_island`).
const WELD_MAX_ERROR: f32 = 200.0;

/// The arms of a weld from each body's centre of mass to the anchor, in the world.
///
/// The anchor is the HOLDER's origin, as in the game (`FixedJoint2D` added to the holder: its anchor is the holder's own origin, and the connected anchor is set to the same
/// place in the held body's frame). With the anchor at the held body's origin (the first version) a light holder welded to a heavy body had the distance between them for a
/// lever: a bibite holding a giant pellet (a plant of 400,000 u^2 is 360 units across its radius) was tied to the giant's centre by a rod of 360 units, so a sliver of angle
/// error was tens of units of position error at the anchor, and the rigid solver answered with tens of units of position correction and hundreds of rad/s of spin (the
/// maze save of 2026-11-15: a bibite that grabbed a meat pellet pressed against the giant it already held left at 434 u/s, spinning at 414 rad/s).
///
/// `weld.anchor` and `weld.angle` stay what they were (the held body's origin in the holder's frame, and the relative rotation, at the moment of the grab): the holder's origin
/// in the held body's frame is `(-anchor).rotate(-angle)`.
#[inline]
fn weld_arms(solver: &[SolverBody], weld: &WeldState) -> (Vec2, Vec2) {
    let (holder, held) = (&solver[weld.holder as usize], &solver[weld.held as usize]);
    let holder_origin_in_held = (-weld.anchor).rotate(-weld.angle);
    ((-holder.com).rotate(holder.rot), (holder_origin_in_held - held.com).rotate(held.rot))
}

/// Applies the impulses a weld carried (`b2WeldJoint` warm start).
fn warm_start_weld(solver: &mut [SolverBody], weld: &WeldState) {
    let (ra, rb) = weld_arms(solver, weld);
    let (a, b) = two_mut(solver, weld.holder as usize, weld.held as usize);
    a.vel -= weld.linear * a.inv_mass;
    a.ang_vel -= a.inv_inertia * (ra.cross(weld.linear) + weld.angular);
    b.vel += weld.linear * b.inv_mass;
    b.ang_vel += b.inv_inertia * (rb.cross(weld.linear) + weld.angular);
}

/// One solve of a weld (`b2SolveWeldJoint` with its soft constraint): the angle first, then the two anchors' coincidence. Without `use_bias` it is the plain
/// velocity constraint (the relax pass).
fn solve_weld(solver: &mut [SolverBody], weld: &mut WeldState, soft: Soft, use_bias: bool) {
    let (ra, rb) = weld_arms(solver, weld);
    let (a, b) = two_mut(solver, weld.holder as usize, weld.held as usize);
    let (bias_rate, mass_scale, impulse_scale) = if use_bias { (soft.bias_rate, soft.mass_scale, soft.impulse_scale) } else { (0.0, 1.0, 0.0) };
    // The angle and the two anchors' coincidence are one constraint of three rows, solved together (`b2WeldJoint`'s 3 x 3 block): a light body held at a long lever by a
    // heavy one couples the rows strongly, and solving them one after the other did not converge (the light body rang, and with a stiff joint it blew up).
    let (ma, mb, ia, ib) = (a.inv_mass, b.inv_mass, a.inv_inertia, b.inv_inertia);
    let k11 = ma + mb + ra.y * ra.y * ia + rb.y * rb.y * ib;
    let k12 = -ra.y * ra.x * ia - rb.y * rb.x * ib;
    let k13 = -ra.y * ia - rb.y * ib;
    let k22 = ma + mb + ra.x * ra.x * ia + rb.x * rb.x * ib;
    let k23 = ra.x * ia + rb.x * ib;
    let k33 = ia + ib;
    let c = (b.pos + rb) - (a.pos + ra);
    let c_angle = (b.rot - a.rot) - weld.angle;
    let cdot = (b.vel + rb.perp() * b.ang_vel) - (a.vel + ra.perp() * a.ang_vel);
    let cdot_angle = b.ang_vel - a.ang_vel;
    let rhs = [cdot.x + bias_rate * c.x, cdot.y + bias_rate * c.y, cdot_angle + bias_rate * c_angle];
    // x = K^-1 rhs by cofactors.
    let c11 = k22 * k33 - k23 * k23;
    let c12 = k13 * k23 - k12 * k33;
    let c13 = k12 * k23 - k13 * k22;
    let det = k11 * c11 + k12 * c12 + k13 * c13;
    if det.abs() < 1e-24 {
        return;
    }
    let c22 = k11 * k33 - k13 * k13;
    let c23 = k12 * k13 - k11 * k23;
    let c33 = k11 * k22 - k12 * k12;
    let inv = 1.0 / det;
    let x = [
        (c11 * rhs[0] + c12 * rhs[1] + c13 * rhs[2]) * inv,
        (c12 * rhs[0] + c22 * rhs[1] + c23 * rhs[2]) * inv,
        (c13 * rhs[0] + c23 * rhs[1] + c33 * rhs[2]) * inv,
    ];
    let impulse = Vec2::new(-mass_scale * x[0], -mass_scale * x[1]) - weld.linear * impulse_scale;
    let impulse_angle = -mass_scale * x[2] - impulse_scale * weld.angular;
    weld.linear += impulse;
    weld.angular += impulse_angle;
    a.vel -= impulse * ma;
    a.ang_vel -= ia * (ra.cross(impulse) + impulse_angle);
    b.vel += impulse * mb;
    b.ang_vel += ib * (rb.cross(impulse) + impulse_angle);
}

/// The position half of a rigid weld (`b2WeldJoint::SolvePositionConstraints`): the anchors' and the angle's error is removed with the 3 x 3 block of the velocity
/// solve, moving the bodies' positions and rotations.
fn solve_weld_position(solver: &mut [SolverBody], weld: &WeldState) {
    let (ra, rb) = weld_arms(solver, weld);
    let (a, b) = two_mut(solver, weld.holder as usize, weld.held as usize);
    let (ma, mb, ia, ib) = (a.inv_mass, b.inv_mass, a.inv_inertia, b.inv_inertia);
    let k11 = ma + mb + ra.y * ra.y * ia + rb.y * rb.y * ib;
    let k12 = -ra.y * ra.x * ia - rb.y * rb.x * ib;
    let k13 = -ra.y * ia - rb.y * ib;
    let k22 = ma + mb + ra.x * ra.x * ia + rb.x * rb.x * ib;
    let k23 = ra.x * ia + rb.x * ib;
    let k33 = ia + ib;
    let c = (b.pos + rb) - (a.pos + ra);
    let c_angle = (b.rot - a.rot) - weld.angle;
    let c11 = k22 * k33 - k23 * k23;
    let c12 = k13 * k23 - k12 * k33;
    let c13 = k12 * k23 - k13 * k22;
    let det = k11 * c11 + k12 * c12 + k13 * c13;
    if det.abs() < 1e-24 {
        return;
    }
    let c22 = k11 * k33 - k13 * k13;
    let c23 = k12 * k13 - k11 * k23;
    let c33 = k11 * k22 - k12 * k12;
    let inv = 1.0 / det;
    let x = [
        (c11 * c.x + c12 * c.y + c13 * c_angle) * inv,
        (c12 * c.x + c22 * c.y + c23 * c_angle) * inv,
        (c13 * c.x + c23 * c.y + c33 * c_angle) * inv,
    ];
    let impulse = Vec2::new(-x[0], -x[1]);
    let impulse_angle = -x[2];
    a.pos -= impulse * ma;
    a.rot -= ia * (ra.cross(impulse) + impulse_angle);
    b.pos += impulse * mb;
    b.rot += ib * (rb.cross(impulse) + impulse_angle);
}

/// Applies an impulse to a body pair at contact offsets.
fn apply_pair_impulse(
    body_a: &mut SolverBody,
    body_b: &mut SolverBody,
    ra: Vec2,
    rb: Vec2,
    direction: Vec2,
    impulse: f32,
) {
    if impulse == 0.0 {
        return;
    }
    body_a.vel -= direction * (impulse * body_a.inv_mass);
    body_a.ang_vel -= body_a.inv_inertia * ra.cross(direction) * impulse;
    body_b.vel += direction * (impulse * body_b.inv_mass);
    body_b.ang_vel += body_b.inv_inertia * rb.cross(direction) * impulse;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod contact_solver_tests {
    use super::*;

    #[test]
    fn position_solver_rotates_off_centre_contacts() {
        // Dynamic 10x10 box at the origin, static capsule overlapping its
        // top-right corner. Velocities are zero, so only the position solver
        // can separate them; Box2D does so with translation *and* rotation.
        let mut dynamic = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.0, 16.666_668);
        dynamic.pos = Vec2::ZERO;
        let _ = dynamic.add_collider(Collider::solid(Shape::box_from_size(Vec2::new(10.0, 10.0))));
        let mut fixed = PhysicsBody::new_static(2, LAYER_BIBITE);
        fixed.pos = Vec2::new(6.5, 4.0);
        let _ = fixed.add_collider(Collider::solid(Shape::capsule_from_size(Vec2::new(
            5.0, 9.0,
        ))));
        let mut bodies = vec![dynamic, fixed];
        let mut scratch = PhysicsScratch::new();
        let config = PhysicsConfig::default();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert!(
            bodies[0].rot.abs() > 1e-6,
            "off-centre position correction did not rotate the body: rot={}",
            bodies[0].rot
        );
        assert!(bodies[0].pos.length() > 0.0);
    }
}

#[cfg(test)]
mod contact_sensitivity_probe {
    use super::*;

    fn head_on(config: &PhysicsConfig) -> (f32, f32) {
        // Two equal capsules meeting head-on; report final gap and relative
        // speed after one step.
        let mut a = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.0, 2.0);
        a.pos = Vec2::new(-1.8, 0.0);
        a.vel = Vec2::new(6.0, 0.0);
        let _ = a.add_collider(Collider::solid(Shape::capsule_from_size(Vec2::new(
            4.0, 8.0,
        ))));
        let mut b = PhysicsBody::new_dynamic(2, LAYER_BIBITE, 1.0, 2.0);
        b.pos = Vec2::new(1.8, 0.0);
        b.vel = Vec2::new(-6.0, 0.0);
        let _ = b.add_collider(Collider::solid(Shape::capsule_from_size(Vec2::new(
            4.0, 8.0,
        ))));
        let mut bodies = vec![a, b];
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, config, 1.0 / 30.0, &mut scratch);
        let gap = (bodies[1].pos.x - bodies[0].pos.x).abs();
        let rel = (bodies[0].vel.x - bodies[1].vel.x).abs();
        (gap, rel)
    }

    fn resting(config: &PhysicsConfig) -> (f32, f32) {
        // Static floor + dynamic circle dropped from small height; report
        // steady-state penetration and vertical drift after 60 steps.
        let mut floor = PhysicsBody::new_static(1, LAYER_BIBITE);
        floor.add_collider(Collider::solid(Shape::box_from_size(Vec2::new(
            100.0, 10.0,
        ))));
        let mut ball = PhysicsBody::new_dynamic(2, LAYER_BIBITE, 1.0, 1.0);
        ball.pos = Vec2::new(0.0, 8.0);
        let _ = ball.add_collider(Collider::solid(Shape::circle(4.0)));
        let mut bodies = vec![ball, floor];
        let mut scratch = PhysicsScratch::new();
        let mut min_gap = f32::INFINITY;
        for _ in 0..60 {
            step(&mut bodies, config, 1.0 / 30.0, &mut scratch);
            min_gap = min_gap.min(bodies[0].pos.y - 4.0 - 5.0);
        }
        let drift = bodies[0].pos.y - 9.0;
        (min_gap, drift)
    }

    #[test]
    #[ignore = "diagnostic: prints contact solver parameter sensitivity"]
    fn contact_solver_sensitivity_probe() {
        let base = PhysicsConfig::default();
        eprintln!(
            "base: head_on gap/rel={:?} resting pen/drift={:?}",
            head_on(&base),
            resting(&base)
        );
        for iterations in [1u32, 5, 10] {
            let mut config = base;
            config.velocity_iterations = iterations;
            eprintln!(
                "velocity_iterations={iterations}: head_on={:?} resting={:?}",
                head_on(&config),
                resting(&config)
            );
        }
        for iterations in [0u32, 1, 3, 8] {
            let mut config = base;
            config.position_iterations = iterations;
            eprintln!(
                "position_iterations={iterations}: head_on={:?} resting={:?}",
                head_on(&config),
                resting(&config)
            );
        }
        for baumgarte in [0.05f32, 0.2, 0.4] {
            let mut config = base;
            config.baumgarte = baumgarte;
            eprintln!(
                "baumgarte={baumgarte}: head_on={:?} resting={:?}",
                head_on(&config),
                resting(&config)
            );
        }
        for skin in [0.0f32, 0.01, 0.05] {
            let mut config = base;
            config.contact_offset = skin;
            eprintln!(
                "contact_offset={skin}: head_on={:?} resting={:?}",
                head_on(&config),
                resting(&config)
            );
        }
        for angular in [0.0f32, 8.0] {
            let mut config = base;
            config.max_angular_correction = angular;
            eprintln!(
                "max_angular_correction={angular}: head_on={:?} resting={:?}",
                head_on(&config),
                resting(&config)
            );
        }
    }
}

#[cfg(test)]
mod broadphase_tests {
    use super::*;

    /// The sweep as it was: every candidate in the x range asked everything.
    fn plain_sweep(entries: &[BodyEntry], start: usize, end: usize) -> (Vec<BodyPair>, bool) {
        let mut pairs = Vec::new();
        for i in start..end {
            let first = entries[i];
            for second in &entries[i + 1..] {
                if second.aabb.min.x > first.aabb.max.x {
                    break;
                }
                if !first.aabb.overlaps(&second.aabb)
                    || first.body == second.body
                    || first.tag == second.tag
                    || !collision_allowed(first.layer, second.layer)
                {
                    continue;
                }
                if pairs.len() >= SWEEP_ABORT_PAIRS {
                    return (pairs, true);
                }
                pairs.push(BodyPair {
                    body_a: first.body,
                    body_b: second.body,
                    tag_a: first.tag,
                    tag_b: second.tag,
                });
            }
        }
        (pairs, false)
    }

    /// Entries in the x order, with boxes of all sizes, equal starts, boxes that start where another ends, infinite and NaN
    /// numbers, repeated tags, layers that do and do not collide.
    fn random_entries(count: usize, seed: u64, wild: bool) -> Vec<BodyEntry> {
        let mut state = seed;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        let mut entries: Vec<BodyEntry> = (0..count)
            .map(|index| {
                let x = (next() % 400) as f32 * 0.5;
                let y = (next() % 400) as f32 * 0.5;
                let (w, h) = ((next() % 24) as f32 * 0.5, (next() % 24) as f32 * 0.5);
                let mut aabb = Aabb {
                    min: Vec2::new(x, y),
                    max: Vec2::new(x + w, y + h),
                };
                if wild {
                    match next() % 40 {
                        0 => aabb.min.x = f32::NAN,
                        1 => aabb.max.y = f32::NAN,
                        2 => aabb.max.x = f32::INFINITY,
                        3 => aabb.min.x = f32::NEG_INFINITY,
                        4 => aabb.min.y = f32::NAN,
                        5 => aabb.min.x = -f32::NAN,
                        _ => {}
                    }
                }
                BodyEntry {
                    body: index as u32 / 2 * 2 + (next() % 2),
                    tag: u64::from(next() % (count as u32 / 2 + 1)),
                    layer: [6u8, 7, 8, 8, 9, 9, 10][(next() % 7) as usize],
                    aabb,
                }
            })
            .collect();
        entries.sort_by(|a, b| {
            a.aabb
                .min
                .x
                .total_cmp(&b.aabb.min.x)
                .then(a.body.cmp(&b.body))
        });
        entries
    }

    /// The sweep over columns finds the pairs the plain sweep finds, in the same order, for every range of entries it may be
    /// cut into, with boxes of every kind.
    #[test]
    fn the_sweep_over_columns_finds_the_pairs_of_the_plain_sweep() {
        for (seed, count, wild) in [
            (1u64, 40usize, false),
            (2, 300, false),
            (3, 1500, false),
            (4, 40, true),
            (5, 300, true),
            (6, 1500, true),
            (7, 7, true),
            (8, 0, false),
        ] {
            let entries = random_entries(count, seed, wild);
            let mut columns = SweepColumns::default();
            columns.fill(&entries, 1, 1);
            for (start, end) in [
                (0, count),
                (0, count / 3),
                (count / 3, count / 2),
                (count / 2, count),
            ] {
                assert_eq!(
                    sweep_cut(&entries, &columns, start, end),
                    plain_sweep(&entries, start, end),
                    "seed {seed}, {count} entries, wild {wild}, range {start}..{end}"
                );
            }
        }
    }

    /// The columns filled by the pool are the ones filled in a row, from whatever size the pool is asked to take them from (4,096
    /// entries with the gates a world starts with, one with the eager ones).
    #[test]
    fn the_columns_filled_on_the_pool_are_the_columns_filled_in_a_row() {
        for (count, seed, min_parallel) in [
            (9000usize, 11u64, crate::sim::ParallelGates::SCALED.sorts),
            (9000, 11, 1),
            (50, 12, 1),
            (3, 13, 1),
            (0, 14, 1),
        ] {
            let entries = random_entries(count, seed, true);
            let (mut serial, mut pooled) = (SweepColumns::default(), SweepColumns::default());
            serial.fill(&entries, 1, min_parallel);
            pooled.fill(&entries, 4, min_parallel);
            for (a, b) in [
                (&serial.min_x, &pooled.min_x),
                (&serial.max_x, &pooled.max_x),
                (&serial.min_y, &pooled.min_y),
                (&serial.max_y, &pooled.max_y),
            ] {
                assert_eq!(a.len(), b.len(), "{count} entries from {min_parallel}");
                assert!(
                    a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{count} entries from {min_parallel}"
                );
            }
        }
    }

    fn circle_body(tag: u64, pos: Vec2, layer: u8) -> PhysicsBody {
        let mut body = PhysicsBody::new_dynamic(tag, layer, 1.0, 1.0);
        body.pos = pos;
        let _ = body.add_collider(Collider::solid(Shape::circle(4.0)));
        body
    }

    #[test]
    fn spread_population_pair_count_is_far_below_n_squared() {
        let n = 5000usize;
        let side = (n as f32).sqrt().ceil() as usize;
        let mut bodies = Vec::with_capacity(n);
        for index in 0..n {
            let x = (index % side) as f32 * 24.0;
            let y = (index / side) as f32 * 24.0;
            bodies.push(circle_body(index as u64, Vec2::new(x, y), LAYER_BIBITE));
        }
        let mut scratch = PhysicsScratch::with_capacity(n, n, 1024);
        step(
            &mut bodies,
            &PhysicsConfig::default(),
            1.0 / 30.0,
            &mut scratch,
        );
        let pairs = scratch.candidate_pair_count();
        let all_pairs = n * (n - 1) / 2;
        assert!(
            pairs < 100_000,
            "grid broadphase generated {pairs} pairs for {n} spread bodies"
        );
        assert!(pairs * 100 < all_pairs);
    }

    #[test]
    fn clustered_candidates_are_capped_deterministically() {
        let n = 2000usize;
        let mut scratch = PhysicsScratch::default();
        let aabb = Aabb::around(Vec2::ZERO, 4.0);
        for index in 0..n {
            scratch.entries.push(BodyEntry {
                body: index as u32,
                tag: index as u64,
                layer: LAYER_BIBITE,
                aabb,
            });
        }
        build_broadphase(&mut scratch);
        let pairs = scratch.pairs.len();
        assert!(pairs > 0);
        assert!(pairs <= MAX_BROADPHASE_PAIRS);
        // Same input -> same candidate list (deterministic truncation).
        let mut second = PhysicsScratch::default();
        second.entries.extend_from_slice(&scratch.entries);
        build_broadphase(&mut second);
        assert_eq!(second.pairs, scratch.pairs);
    }

    #[test]
    fn layer_disallowed_pairs_are_pruned_before_final_pairs() {
        let mut scratch = PhysicsScratch::default();
        let aabb = Aabb::around(Vec2::ZERO, 4.0);
        for index in 0..10u32 {
            scratch.entries.push(BodyEntry {
                body: index,
                tag: u64::from(index),
                layer: 0,
                aabb,
            });
        }
        build_broadphase(&mut scratch);
        assert!(scratch.pairs.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::PI;
    use crate::rng::Rng;

    const TOL: f32 = 1e-4;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= TOL * expected.abs().max(1.0),
            "expected {expected}, got {actual}"
        );
    }

    fn circle_body(tag: u64, pos: Vec2, radius: f32, mass: f32, layer: u8) -> PhysicsBody {
        let mut body = PhysicsBody::new_dynamic(tag, layer, mass, mass * 0.5 * radius * radius);
        body.pos = pos;
        let _ = body.add_collider(Collider::solid(Shape::circle(radius)));
        body
    }

    /// The grid broadphase finds the pairs of the sweep, bit for bit once both are in order by tag: the same pairs, the first of each the body whose
    /// box starts first in the sweep's order. Worlds of every size and spread, boxes on whole numbers (so that they touch exactly and start at the
    /// same x), boxes with no size, big boxes, repeated tags, layers that do not collide, bodies that take no part; serial and on the pool.
    #[test]
    fn the_grid_broadphase_finds_the_pairs_of_the_sweep() {
        let mut rng = Rng::from_seed(2026);
        let layers = [LAYER_BIBITE, LAYER_PELLETS, LAYER_SCENE_TRIGGERS, 0u8];
        let key = |pair: &BodyPair| (pair.tag_a, pair.tag_b, pair.body_a, pair.body_b);
        let (mut used, mut with_pairs, mut declined) = (0, 0, 0);
        for case in 0..400usize {
            let n = [12, 60, 300, 1_500, 4_000][case % 5] + (case / 5) % 7;
            let world = [200.0f32, 1_000.0, 6_000.0, 40_000.0][(case / 5) % 4];
            let whole_numbers = case % 3 == 0;
            let mut boxes = Vec::with_capacity(n);
            for index in 0..n {
                let coordinate = |rng: &mut Rng| {
                    let value = rng.f32_range(-world, world);
                    if whole_numbers { value.round() } else { value }
                };
                let (x, y) = (coordinate(&mut rng), coordinate(&mut rng));
                let half = match rng.f32() {
                    r if r < 0.01 => rng.f32_range(80.0, 1_500.0),
                    r if r < 0.05 => 0.0,
                    r if r < 0.3 => rng.f32_range(8.0, 40.0),
                    _ => rng.f32_range(1.0, 8.0),
                };
                let half = if whole_numbers { half.round() } else { half };
                let tag = if rng.f32() < 0.05 { (index / 2) as u64 } else { index as u64 + 1 };
                boxes.push(BodyBox {
                    aabb: Aabb { min: Vec2::new(x - half, y - half), max: Vec2::new(x + half, y + half) },
                    tag,
                    layer: layers[(rng.f32() * 4.0) as usize % 4],
                    valid: rng.f32() > 0.05,
                });
            }
            for threads in [0usize, 4] {
                let mut sweep = PhysicsScratch::default();
                sweep.threads = threads;
                sweep.min_broadphase = 1;
                sweep.min_sort = 1;
                sweep.body_boxes = boxes.clone();
                let mut lap = Instant::now();
                sweep_broadphase(&mut sweep, threads, 1, &mut lap);
                let mut grid = PhysicsScratch::default();
                grid.threads = threads;
                grid.min_broadphase = 1;
                grid.body_boxes = boxes.clone();
                if !grid_broadphase(&mut grid, 1) {
                    declined += 1;
                    continue;
                }
                used += 1;
                with_pairs += usize::from(!sweep.pairs.is_empty());
                sweep.pairs.sort_unstable_by_key(key);
                grid.pairs.sort_unstable_by_key(key);
                assert_eq!(grid.pairs, sweep.pairs, "case {case}: {n} bodies in a world of {world}, {threads} threads");
                assert!(!grid.dense_cap_hit);
            }
        }
        assert!(used > 200 && with_pairs > 150, "the grid took {used} worlds (declined {declined}), {with_pairs} of them with pairs");
    }

    /// A box that is not a finite box, or a world the sweep would hand to the capped grid, is not the grid broadphase's to answer: it declines and
    /// leaves the pairs as they were.
    #[test]
    fn the_grid_broadphase_declines_what_it_cannot_answer_for() {
        let finite = |x: f32| BodyBox {
            aabb: Aabb { min: Vec2::new(x, 0.0), max: Vec2::new(x + 3.0, 3.0) },
            tag: x as u64 + 1,
            layer: LAYER_BIBITE,
            valid: true,
        };
        let spread: Vec<BodyBox> = (0..40).map(|i| finite(i as f32 * 20.0)).collect();
        let mut scratch = PhysicsScratch::default();
        scratch.body_boxes = spread.clone();
        assert!(grid_broadphase(&mut scratch, 8), "a plain spread world is the grid's");
        for broken in [f32::NAN, f32::INFINITY] {
            let mut boxes = spread.clone();
            boxes[7].aabb.max.x = broken;
            let mut scratch = PhysicsScratch::default();
            scratch.body_boxes = boxes;
            assert!(!grid_broadphase(&mut scratch, 8), "a box with {broken} in it");
        }
        // Everything on top of everything: dense.
        let pile: Vec<BodyBox> = (0..300)
            .map(|i| BodyBox { aabb: Aabb { min: Vec2::ZERO, max: Vec2::splat(4.0) }, tag: i + 1, layer: LAYER_BIBITE, valid: true })
            .collect();
        let mut scratch = PhysicsScratch::default();
        scratch.body_boxes = pile;
        assert!(!grid_broadphase(&mut scratch, 8), "a pile of bodies");
        // Too few bodies for the size asked.
        let mut scratch = PhysicsScratch::default();
        scratch.body_boxes = spread;
        assert!(!grid_broadphase(&mut scratch, 100));
    }

    /// The box of a body that is one circle on its origin (a pellet, an egg) is worked out without the rotation: it must be the box the
    /// general path gives, bit for bit, whatever the position (signed zeros and huge values), the rotation, the radius and the speed
    /// are, and the bodies the shortcut does not take (a position that is not a number, an offset collider, two colliders) must come out
    /// the same too.
    #[test]
    fn the_box_of_a_circle_on_the_origin_is_the_general_box() {
        let mut rng = Rng::from_seed(77);
        let specials = [0.0, -0.0, 1e-30, -1e-30, 1e7, -1e7, 0.06, -0.06];
        let mut shortcut_cases = 0;
        for case in 0..40_000 {
            let pick = |rng: &mut Rng| {
                if rng.f32() < 0.15 {
                    specials[(rng.f32() * specials.len() as f32) as usize % specials.len()]
                } else {
                    rng.f32_range(-3000.0, 3000.0)
                }
            };
            let pos = Vec2::new(pick(&mut rng), pick(&mut rng));
            let radius = if rng.f32() < 0.05 {
                [0.0, f32::INFINITY, f32::NAN, -1.0][case % 4]
            } else {
                rng.f32_range(0.001, 60.0)
            };
            let mut body = PhysicsBody::new_dynamic(case as u64 + 1, LAYER_PELLETS, 1.0, 1.0);
            body.pos = pos;
            body.rot = if rng.f32() < 0.1 {
                [0.0, -0.0, 1e6, 1e-9, f32::NAN, f32::INFINITY][case % 6]
            } else {
                rng.f32_range(-60.0, 60.0)
            };
            body.vel = if rng.f32() < 0.3 {
                Vec2::new(rng.f32_range(-200.0, 200.0), rng.f32_range(-200.0, 200.0))
            } else {
                Vec2::new(rng.f32_range(-5.0, 5.0), rng.f32_range(-5.0, 5.0))
            };
            if case % 10 == 0 {
                body.pos = Vec2::new(f32::NAN, pos.y);
            }
            let collider = if case % 17 == 0 {
                Collider::solid(Shape::circle(radius)).with_offset(Vec2::new(1.5, -0.5))
            } else {
                Collider::solid(Shape::circle(radius))
            };
            let _ = body.add_collider(collider);
            if case % 23 == 0 {
                let _ = body.add_collider(Collider::solid(Shape::circle(2.0)));
            }
            let (fast, general) = (body.aabb(), body.aabb_general());
            let bits = |boxed: Option<Aabb>| {
                boxed.map(|b| [b.min.x.to_bits(), b.min.y.to_bits(), b.max.x.to_bits(), b.max.y.to_bits()])
            };
            assert_eq!(bits(fast), bits(general), "case {case}: {body:?}");
            if body.collider_count() == 1 && radius > 0.0 && radius.is_finite() && body.pos.x.is_finite() && body.rot.is_finite() && case % 17 != 0 {
                shortcut_cases += 1;
            }
        }
        assert!(shortcut_cases > 20_000, "only {shortcut_cases} cases took the shortcut");
    }

    #[test]
    fn layer_matrix_matches_capture() {
        assert!(collision_allowed(LAYER_UI_PHYSICS, LAYER_UI_PHYSICS));
        assert!(collision_allowed(LAYER_SCENE_TRIGGERS, LAYER_BIBITE));
        assert!(collision_allowed(LAYER_BIBITE, LAYER_SCENE_TRIGGERS));
        assert!(collision_allowed(LAYER_BIBITE, LAYER_BIBITE));
        assert!(collision_allowed(LAYER_BIBITE, LAYER_PELLETS));
        assert!(collision_allowed(LAYER_PELLETS, LAYER_PELLETS));
        assert!(!collision_allowed(LAYER_PHEROMONES, LAYER_BIBITE));
        assert!(!collision_allowed(LAYER_PHEROMONES, LAYER_PHEROMONES));
        assert!(!collision_allowed(
            LAYER_SCENE_TRIGGERS,
            LAYER_SCENE_TRIGGERS
        ));
        assert!(!collision_allowed(0, LAYER_BIBITE));
        assert_eq!(LAYER_NAMES[LAYER_BIBITE as usize], "bibite");
        assert_eq!(LAYER_NAMES.len(), LAYER_COUNT);
    }

    #[test]
    fn combine_rules() {
        close(CombineRule::Mean.combine(0.4, 0.1), 0.2);
        close(CombineRule::Min.combine(0.4, 0.1), 0.1);
        close(CombineRule::Multiply.combine(0.4, 0.1), 0.04);
        close(CombineRule::Maximum.combine(0.4, 0.1), 0.4);
    }

    #[test]
    fn circle_circle_analytic() {
        let a = Shape::circle(2.0);
        let b = Shape::circle(2.0);
        let m = collide(
            &a,
            Pose::new(Vec2::ZERO, 0.0),
            &b,
            Pose::new(Vec2::new(3.0, 0.0), 0.0),
        )
        .unwrap();
        assert_eq!(m.count, 1);
        assert!(m.normal.approx_eq(Vec2::X, TOL));
        close(m.points[0].separation, -1.0);
        close(m.points[0].point.x, 1.5);
        // Exactly touching.
        let m = collide(
            &a,
            Pose::new(Vec2::ZERO, 0.0),
            &b,
            Pose::new(Vec2::new(4.0, 0.0), 0.0),
        )
        .unwrap();
        close(m.points[0].separation, 0.0);
        // Separated.
        assert!(
            collide(
                &a,
                Pose::new(Vec2::ZERO, 0.0),
                &b,
                Pose::new(Vec2::new(5.0, 0.0), 0.0),
            )
            .is_none()
        );
    }

    #[test]
    fn circle_capsule_analytic() {
        let circle = Shape::circle(1.0);
        let capsule = Shape::capsule_from_size(Vec2::new(2.0, 4.0)); // r=1, hl=1
        let m = collide(
            &circle,
            Pose::new(Vec2::ZERO, 0.0),
            &capsule,
            Pose::new(Vec2::new(0.0, 3.0), 0.0),
        )
        .unwrap();
        assert!(m.normal.approx_eq(Vec2::Y, TOL));
        close(m.points[0].separation, 0.0);
        // Reversed order flips the normal.
        let flipped = collide(
            &capsule,
            Pose::new(Vec2::new(0.0, 3.0), 0.0),
            &circle,
            Pose::new(Vec2::ZERO, 0.0),
        )
        .unwrap();
        assert!(flipped.normal.approx_eq(-Vec2::Y, TOL));
    }

    #[test]
    fn capsule_capsule_crossing() {
        let a = Shape::capsule_from_size(Vec2::new(1.0, 5.0)); // r .5, hl 2
        let b = Shape::capsule_from_size(Vec2::new(1.0, 5.0));
        // A horizontal (rotation pi/2), B vertical offset so segments are 0.5 apart.
        let m = collide(
            &a,
            Pose::new(Vec2::ZERO, PI / 2.0),
            &b,
            Pose::new(Vec2::new(1.5, 2.5), 0.0),
        )
        .unwrap();
        close(m.points[0].separation, -0.5);
        assert!(m.normal.y > 0.9);
    }

    #[test]
    fn circle_box_analytic() {
        let circle = Shape::circle(0.5);
        let box_shape = Shape::box_from_size(Vec2::new(2.0, 2.0)); // half (1,1)
        // Overlap 0.1: circle centre 1.4, box face 1.0.
        let m = collide(
            &circle,
            Pose::new(Vec2::new(1.4, 0.0), 0.0),
            &box_shape,
            Pose::new(Vec2::ZERO, 0.0),
        )
        .unwrap();
        assert!(m.normal.approx_eq(-Vec2::X, TOL)); // A=circle -> B=box
        close(m.points[0].separation, -0.1);
        // Reversed order: box -> circle normal +X.
        let flipped = collide(
            &box_shape,
            Pose::new(Vec2::ZERO, 0.0),
            &circle,
            Pose::new(Vec2::new(1.4, 0.0), 0.0),
        )
        .unwrap();
        assert!(flipped.normal.approx_eq(Vec2::X, TOL));
        // Circle centre inside the box: 0.1 to the +x face plus the radius.
        let inside = collide(
            &box_shape,
            Pose::new(Vec2::ZERO, 0.0),
            &circle,
            Pose::new(Vec2::new(0.9, 0.0), 0.0),
        )
        .unwrap();
        assert!(inside.normal.approx_eq(Vec2::X, TOL));
        close(inside.points[0].separation, -0.6);
        // Rotated box: closest point transforms with the pose.
        let rotated = collide(
            &circle,
            Pose::new(Vec2::new(0.0, 1.4), 0.0),
            &box_shape,
            Pose::new(Vec2::ZERO, PI / 2.0),
        )
        .unwrap();
        assert!(rotated.normal.approx_eq(-Vec2::Y, TOL));
        close(rotated.points[0].separation, -0.1);
    }

    #[test]
    fn capsule_box_analytic() {
        let capsule = Shape::capsule_from_size(Vec2::new(2.0, 4.0)); // r1, hl1
        let box_shape = Shape::box_from_size(Vec2::new(4.0, 4.0)); // half (2,2)
        // Capsule pose y = 4 puts the segment end at y = 3, i.e. the cap
        // surface exactly on the box top edge y = 2 (centre-line distance = r).
        let m = collide(
            &capsule,
            Pose::new(Vec2::new(0.0, 4.0), 0.0),
            &box_shape,
            Pose::new(Vec2::ZERO, 0.0),
        )
        .unwrap();
        assert!(m.normal.approx_eq(-Vec2::Y, TOL));
        close(m.points[0].separation, 0.0);
        // Overlap by 0.5.
        let m = collide(
            &capsule,
            Pose::new(Vec2::new(0.0, 3.5), 0.0),
            &box_shape,
            Pose::new(Vec2::ZERO, 0.0),
        )
        .unwrap();
        close(m.points[0].separation, -0.5);
        // Separated.
        assert!(
            collide(
                &capsule,
                Pose::new(Vec2::new(0.0, 5.0), 0.0),
                &box_shape,
                Pose::new(Vec2::ZERO, 0.0),
            )
            .is_none()
        );
    }

    #[test]
    fn contact_skin_generates_gap_contacts() {
        let a = Shape::circle(1.0);
        let b = Shape::circle(1.0);
        let pa = Pose::new(Vec2::ZERO, 0.0);
        let pb = Pose::new(Vec2::new(2.005, 0.0), 0.0);
        assert!(collide(&a, pa, &b, pb).is_none());
        let m = collide_with_skin(&a, pa, &b, pb, 0.01).unwrap();
        close(m.points[0].separation, 2.005 - 2.02);
    }

    #[test]
    fn bounds_helpers() {
        let bounds = WorldBounds::from_half_extent(Vec2::splat(10.0));
        assert!(bounds.contains(Vec2::new(10.0, -10.0)));
        assert!(!bounds.contains(Vec2::new(11.0, 0.0)));
        assert_eq!(
            bounds.clamp_point(Vec2::new(11.0, 5.0)),
            Vec2::new(10.0, 5.0)
        );
        assert!(bounds.overlaps(&Aabb::around(Vec2::new(9.0, 0.0), 2.0)));
        assert!(!bounds.overlaps(&Aabb::around(Vec2::new(13.0, 0.0), 1.0)));
        assert_eq!(bounds.size(), Vec2::splat(20.0));
    }

    #[test]
    fn capsules_side_by_side_make_two_contact_points() {
        let a = Shape::capsule_from_size(Vec2::new(5.0, 9.0));
        let m = collide(&a, Pose::new(Vec2::ZERO, 0.0), &a, Pose::new(Vec2::new(4.5, 0.3), 0.0)).expect("overlap");
        assert_eq!(m.count, 2, "{m:?}");
        // Both ends of the shared side are on the same side of the middle, symmetrically.
        assert!(m.points[0].point.y != m.points[1].point.y);
        assert!((m.normal.x - 1.0).abs() < 1e-3, "{m:?}");
    }

    #[test]
    fn damping_and_speed_clamps() {
        let _ten = ten_steps_for_this_test();
        let config = PhysicsConfig::default();
        let mut bodies = [circle_body(1, Vec2::ZERO, 1.0, 1.0, LAYER_BIBITE)];
        bodies[0].vel = Vec2::new(10.0, 0.0);
        bodies[0].linear_damping = 1.0;
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 0.1, &mut scratch);
        // The game's damping over one step: ten slices of `1 / (1 + h c / 10)`.
        close(bodies[0].vel.x, 10.0 * (1.0_f32 + 0.1 * 1.0 / 10.0).powi(-10));
        // Clamp to max translation speed.
        bodies[0].vel = Vec2::new(900.0, 0.0);
        bodies[0].linear_damping = 0.0;
        step(&mut bodies, &config, 0.1, &mut scratch);
        close(bodies[0].vel.x, 400.0);
        // Angular clamp: 360 deg/s = 2*pi rad/s.
        bodies[0].ang_vel = 100.0;
        step(&mut bodies, &config, 0.1, &mut scratch);
        close(bodies[0].ang_vel, core::f32::consts::TAU);
    }

    #[test]
    fn a_body_turns_about_its_centre_of_mass() {
        // A circle whose centre lies 2 units behind the pivot: the centre of mass is there, so a spinning body's centre stays put and the pivot swings round it.
        let config = PhysicsConfig::default();
        let mut body = circle_body(1, Vec2::ZERO, 1.0, 1.0, LAYER_BIBITE);
        body.colliders_mut()[0].offset = Vec2::new(0.0, -2.0);
        body.ang_vel = 2.0;
        body.linear_damping = 0.0;
        body.angular_damping = 0.0;
        let mut bodies = [body];
        let mut scratch = PhysicsScratch::new();
        for _ in 0..30 {
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        }
        let centre = bodies[0].pos + Vec2::new(0.0, -2.0).rotate(bodies[0].rot);
        assert!(centre.distance(Vec2::new(0.0, -2.0)) < 0.02, "the centre of mass moved to {centre:?}");
        assert!((bodies[0].rot - 2.0).abs() < 1e-3);
        assert!(bodies[0].pos.length() > 1.0, "the pivot should swing round the centre of mass");
        // A body that is pushed, not spun, moves like one without an offset.
        let mut pushed = circle_body(2, Vec2::ZERO, 1.0, 1.0, LAYER_BIBITE);
        pushed.colliders_mut()[0].offset = Vec2::new(0.0, -2.0);
        pushed.vel = Vec2::new(3.0, 0.0);
        pushed.linear_damping = 0.0;
        let mut bodies = [pushed];
        step(&mut bodies, &config, 0.1, &mut scratch);
        close(bodies[0].pos.x, 0.3);
        close(bodies[0].pos.y, 0.0);
    }

    #[test]
    fn a_push_follows_the_heading_of_a_turning_body() {
        let _ten = ten_steps_for_this_test();
        // The game adds the force before each of its ten physics steps, along the heading of that moment: a body that turns while it is pushed forward
        // drifts to the side (measured against the original: vx -0.0009 after the first tick of a push of 2.1 u/s with a turn of 0.08 degrees).
        let config = PhysicsConfig::default();
        let mut body = circle_body(1, Vec2::ZERO, 1.0, 1.0, LAYER_BIBITE);
        body.force = Vec2::new(0.0, 60.0);
        body.torque = 5.0;
        body.linear_damping = 0.5;
        body.angular_damping = 0.5;
        let (dt, h) = (1.0 / 30.0, 1.0 / 300.0);
        let (mut v, mut w, mut theta) = (Vec2::ZERO, 0.0f32, 0.0f32);
        for _ in 0..10 {
            let push = body.force.rotate(theta);
            v = (v + push * (h * body.inv_mass)) * (1.0 / (1.0 + h * 0.5));
            w = (w + body.torque * h * body.inv_inertia) * (1.0 / (1.0 + h * 0.5));
            theta += w * h;
        }
        let mut bodies = [body];
        step(&mut bodies, &config, dt, &mut PhysicsScratch::new());
        assert!(bodies[0].vel.x < 0.0);
        assert!((bodies[0].vel.x - v.x).abs() < 1e-4 * v.y.abs().max(1.0) && (bodies[0].vel.y - v.y).abs() < 1e-3, "{:?} vs {v:?}", bodies[0].vel);
        assert!((bodies[0].rot - theta).abs() < 1e-5);
    }

    #[test]
    fn overlapping_bodies_separate() {
        let config = PhysicsConfig::default();
        let mut bodies = [
            circle_body(1, Vec2::ZERO, 1.0, 1.0, LAYER_BIBITE),
            circle_body(2, Vec2::new(1.0, 0.0), 1.0, 1.0, LAYER_BIBITE),
        ];
        let start = bodies[0].pos.distance(bodies[1].pos);
        let mut scratch = PhysicsScratch::new();
        for _ in 0..10 {
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        }
        let end = bodies[0].pos.distance(bodies[1].pos);
        assert!(end > start, "bodies did not separate: {start} -> {end}");
        assert!((end - 2.0).abs() < 0.15, "separation not resolved: {end}");
        assert!(
            bodies
                .iter()
                .all(|b| b.pos.is_finite() && b.vel.is_finite())
        );
    }

    fn static_wall(tag: u64) -> PhysicsBody {
        let mut wall = PhysicsBody::new_static(tag, LAYER_BIBITE);
        wall.pos = Vec2::new(2.0, 0.0);
        let _ = wall.add_collider(Collider::solid(Shape::box_from_size(Vec2::new(1.0, 10.0))));
        wall
    }

    #[test]
    fn restitution_bounces_above_threshold_only() {
        let config = PhysicsConfig::default();
        let mut scratch = PhysicsScratch::new();
        let mut bodies = [
            circle_body(1, Vec2::new(1.0, 0.0), 0.5, 1.0, LAYER_BIBITE),
            static_wall(2),
        ];
        bodies[0].vel = Vec2::new(10.0, 0.0);
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert!(
            bodies[0].vel.x < 0.0,
            "expected bounce, got {:?}",
            bodies[0].vel
        );
        assert!(!scratch.contacts().is_empty());
        // Below the 1 u/s bounce threshold, restitution is suppressed.
        let mut bodies = [
            circle_body(1, Vec2::new(1.0, 0.0), 0.5, 1.0, LAYER_BIBITE),
            static_wall(2),
        ];
        bodies[0].vel = Vec2::new(0.5, 0.0);
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert!(bodies[0].vel.x <= 0.0);
    }

    #[test]
    fn sensor_reports_trigger_without_impulse() {
        let config = PhysicsConfig::default();
        let mut bibite = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.0, 1.0);
        let _ = bibite.add_collider(Collider::sensor(Shape::box_from_size(Vec2::new(2.0, 2.0))));
        let mut pellet = circle_body(2, Vec2::new(0.5, 0.0), 0.5, 1.0, LAYER_PELLETS);
        pellet.vel = Vec2::ZERO;
        let bibite_before = bibite.pos;
        let pellet_before = pellet.pos;
        let mut bodies = [bibite, pellet];
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert_eq!(scratch.triggers().len(), 1);
        assert!(scratch.contacts().is_empty());
        assert_eq!(bodies[1].vel, Vec2::ZERO);
        assert_eq!(bodies[1].pos, pellet_before);
        assert_eq!(bodies[0].pos, bibite_before);
    }

    #[test]
    fn sensor_sensor_pair_reports_both_owners() {
        let config = PhysicsConfig::default();
        let mut first = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.0, 1.0);
        let _ = first.add_collider(Collider::sensor(Shape::circle(1.0)));
        let mut second = PhysicsBody::new_dynamic(2, LAYER_BIBITE, 1.0, 1.0);
        let _ = second.add_collider(Collider::sensor(Shape::circle(1.0)));
        second.pos = Vec2::new(0.5, 0.0);
        let mut bodies = [first, second];
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        let trigger = scratch.triggers().first().expect("sensor overlap");
        assert!(trigger.sensor_a && trigger.sensor_b);
        assert!(scratch.contacts().is_empty());
    }

    #[test]
    fn layer_filtering_blocks_pheromones() {
        let config = PhysicsConfig::default();
        let mut bodies = [
            circle_body(1, Vec2::ZERO, 1.0, 1.0, LAYER_BIBITE),
            circle_body(2, Vec2::new(0.5, 0.0), 1.0, 1.0, LAYER_PHEROMONES),
        ];
        bodies[0].linear_damping = 0.0;
        bodies[1].linear_damping = 0.0;
        bodies[0].vel = Vec2::new(1.0, 0.0);
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert!(scratch.contacts().is_empty());
        assert!(scratch.triggers().is_empty());
        close(bodies[0].vel.x, 1.0);
    }

    #[test]
    fn static_bodies_never_move() {
        let config = PhysicsConfig::default();
        let mut bodies = [
            circle_body(1, Vec2::new(1.0, 0.0), 1.0, 1.0, LAYER_BIBITE),
            static_wall(2),
        ];
        let wall_pos = bodies[1].pos;
        let wall_rot = bodies[1].rot;
        bodies[0].vel = Vec2::new(10.0, 0.0);
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert_eq!(bodies[1].pos, wall_pos);
        assert_eq!(bodies[1].rot, wall_rot);
        assert_eq!(bodies[1].vel, Vec2::ZERO);
    }

    #[test]
    fn deterministic_repeated_runs() {
        let config = PhysicsConfig::default();
        let build = || {
            let mut bodies = [
                circle_body(3, Vec2::ZERO, 1.0, 1.5, LAYER_BIBITE),
                circle_body(1, Vec2::new(1.2, 0.4), 0.8, 2.0, LAYER_BIBITE),
                circle_body(2, Vec2::new(-0.7, 1.1), 1.1, 0.7, LAYER_PELLETS),
            ];
            bodies[0].vel = Vec2::new(2.0, 0.0);
            bodies[1].vel = Vec2::new(-1.0, 0.5);
            bodies
        };
        let mut scratch = PhysicsScratch::new();
        let mut first = build();
        let mut second = build();
        for _ in 0..25 {
            step(&mut first, &config, 1.0 / 30.0, &mut scratch);
        }
        for _ in 0..25 {
            step(&mut second, &config, 1.0 / 30.0, &mut scratch);
        }
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.vel, b.vel);
            assert_eq!(a.rot, b.rot);
            assert_eq!(a.ang_vel, b.ang_vel);
        }
    }

    #[test]
    fn broadphase_contacts_match_brute_force() {
        let config = PhysicsConfig::default();
        let mut bodies = Vec::new();
        let mut rng = Rng::from_seed(7);
        for i in 0..12u64 {
            let radius = 0.5 + rng.f32();
            let pos = Vec2::new(rng.f32_range(-4.0, 4.0), rng.f32_range(-4.0, 4.0));
            let layer = if i % 2 == 0 {
                LAYER_BIBITE
            } else {
                LAYER_PELLETS
            };
            bodies.push(circle_body(i + 1, pos, radius, 1.0, layer));
        }
        // Brute-force expected pairs from the pre-step poses.
        let mut expected = Vec::new();
        for i in 0..bodies.len() {
            for j in (i + 1)..bodies.len() {
                if bodies[i].tag == bodies[j].tag
                    || !collision_allowed(bodies[i].layer, bodies[j].layer)
                {
                    continue;
                }
                let (Shape::Circle { radius: ra }, Shape::Circle { radius: rb }) = (
                    &bodies[i].colliders()[0].shape,
                    &bodies[j].colliders()[0].shape,
                ) else {
                    unreachable!()
                };
                // Contact skin (0.01 each) means contacts appear for gaps <= 0.02.
                if bodies[i].pos.distance(bodies[j].pos) - ra - rb <= 0.020_1 {
                    expected.push((
                        bodies[i].tag.min(bodies[j].tag),
                        bodies[i].tag.max(bodies[j].tag),
                    ));
                }
            }
        }
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        let mut actual: Vec<(u64, u64)> = Vec::new();
        actual.extend(
            scratch
                .contacts()
                .iter()
                .map(|c| (c.tag_a.min(c.tag_b), c.tag_a.max(c.tag_b))),
        );
        actual.extend(
            scratch
                .triggers()
                .iter()
                .map(|c| (c.tag_a.min(c.tag_b), c.tag_a.max(c.tag_b))),
        );
        actual.sort_unstable();
        actual.dedup();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(actual, expected);
    }

    #[test]
    fn fuzz_never_produces_nan_or_explodes() {
        let config = PhysicsConfig::default();
        let mut rng = Rng::from_seed(0xC0FFEE);
        let mut bodies = Vec::with_capacity(24);
        for i in 0..24u64 {
            let shape = match i % 3 {
                0 => Shape::circle(0.2 + rng.f32() * 1.5),
                1 => Shape::capsule_from_size(Vec2::new(
                    0.5 + rng.f32() * 2.0,
                    1.0 + rng.f32() * 4.0,
                )),
                _ => Shape::box_from_size(Vec2::new(0.5 + rng.f32() * 2.0, 0.5 + rng.f32() * 2.0)),
            };
            let layer = [LAYER_BIBITE, LAYER_PELLETS, LAYER_SCENE_TRIGGERS][(i % 3) as usize];
            let mass = 0.1 + rng.f32() * 5.0;
            let mut body = PhysicsBody::new_dynamic(i + 1, layer, mass, mass * (0.2 + rng.f32()));
            body.pos = Vec2::new(rng.f32_range(-5.0, 5.0), rng.f32_range(-5.0, 5.0));
            body.rot = rng.f32_range(-PI, PI);
            body.vel = Vec2::new(rng.f32_range(-3.0, 3.0), rng.f32_range(-3.0, 3.0));
            body.ang_vel = rng.f32_range(-2.0, 2.0);
            let mut collider = if i % 5 == 0 {
                Collider::sensor(shape)
            } else {
                Collider::solid(shape)
            };
            collider.friction = rng.f32_range(0.0, 1.0);
            collider.restitution = rng.f32_range(0.0, 1.0);
            let _ = body.add_collider(collider);
            bodies.push(body);
        }
        let mut scratch = PhysicsScratch::new();
        for _ in 0..300 {
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
            for body in &bodies {
                assert!(body.pos.is_finite(), "non-finite position");
                assert!(body.rot.is_finite(), "non-finite rotation");
                assert!(body.vel.is_finite(), "non-finite velocity");
                assert!(body.ang_vel.is_finite(), "non-finite angular velocity");
                assert!(body.vel.length() <= config.max_translation_speed + 1e-3);
                assert!(body.ang_vel.abs() <= config.max_rotation_speed.to_radians() + 1e-3);
            }
            for contact in scratch.contacts() {
                assert!(contact.normal.is_finite());
                assert!(contact.point.is_finite());
                assert!(contact.normal_impulse.is_finite());
                assert!(contact.normal_impulse >= 0.0);
            }
        }
    }

    #[test]
    fn stacked_overlaps_converge_without_exploding() {
        let config = PhysicsConfig::default();
        let mut bodies = [
            circle_body(1, Vec2::new(0.0, 0.0), 1.0, 1.0, LAYER_BIBITE),
            circle_body(2, Vec2::new(0.0, 1.2), 1.0, 1.0, LAYER_BIBITE),
            circle_body(3, Vec2::new(0.0, 2.4), 1.0, 1.0, LAYER_BIBITE),
            circle_body(4, Vec2::new(0.0, 3.6), 1.0, 1.0, LAYER_BIBITE),
        ];
        let mut scratch = PhysicsScratch::new();
        for _ in 0..30 {
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
            for body in &bodies {
                assert!(body.pos.is_finite() && body.vel.is_finite());
            }
        }
        for pair in bodies.windows(2) {
            let gap = pair[0].pos.distance(pair[1].pos);
            assert!(gap >= 2.0 - 0.05, "stack did not separate: gap {gap}");
        }
    }

    #[test]
    fn head_on_collision_does_not_add_kinetic_energy() {
        let config = PhysicsConfig::default();
        let mut bodies = [
            circle_body(1, Vec2::new(-0.6, 0.0), 0.5, 1.0, LAYER_BIBITE),
            circle_body(2, Vec2::new(0.6, 0.0), 0.5, 1.0, LAYER_BIBITE),
        ];
        bodies[0].linear_damping = 0.0;
        bodies[1].linear_damping = 0.0;
        bodies[0].vel = Vec2::new(5.0, 0.0);
        bodies[1].vel = Vec2::new(-5.0, 0.0);
        let kinetic = |b: &[PhysicsBody]| {
            b.iter()
                .map(|body| 0.5 * body.mass * body.vel.length_sq())
                .sum::<f32>()
        };
        let before = kinetic(&bodies);
        let mut scratch = PhysicsScratch::new();
        let mut collided = false;
        for _ in 0..10 {
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
            if !scratch.contacts().is_empty() {
                collided = true;
            }
            let after = kinetic(&bodies);
            assert!(
                after <= before * 1.05 + 1e-3,
                "solver injected energy: {before} -> {after}"
            );
        }
        assert!(collided, "the head-on bodies never produced a contact");
    }

    #[test]
    fn world_bounds_from_capture_scale() {
        let bounds = WorldBounds::from_half_extent(Vec2::splat(2000.0));
        assert!(bounds.contains(Vec2::ZERO));
        assert!(!bounds.contains(Vec2::new(2000.1, 0.0)));
        assert_eq!(bounds.size(), Vec2::splat(4000.0));
    }

    /// Circles on a lattice, so that whole columns of boxes start at exactly the same x.
    fn lattice(columns: usize, rows: usize) -> Vec<PhysicsBody> {
        let mut bodies = Vec::new();
        for column in 0..columns {
            for row in 0..rows {
                let mut body =
                    PhysicsBody::new_dynamic(bodies.len() as u64 + 1, LAYER_PELLETS, 1.0, 1.0);
                body.pos = Vec2::new(column as f32 * 10.0, row as f32 * 10.0);
                let _ = body.add_collider(Collider::solid(Shape::circle(2.0)));
                bodies.push(body);
            }
        }
        bodies
    }

    /// The bodies of `scratch.entries`, in order.
    fn x_order_of(scratch: &PhysicsScratch) -> Vec<u32> {
        scratch.entries.iter().map(|entry| entry.body).collect()
    }

    /// The x order is a total order (equal box starts by body index): sorted from scratch serially or on the pool, or carried
    /// from the previous step and repaired (or, far from sorted, sorted from scratch after all), it is always the same.
    #[test]
    fn x_order_is_the_same_however_it_is_reached() {
        let config = PhysicsConfig::default();
        let dt = 1.0 / 30.0;
        let bodies = lattice(80, 60);
        assert!(bodies.len() >= crate::sim::ParallelGates::SCALED.sorts);
        let sorted_by_hand = {
            let mut order: Vec<u32> = (0..bodies.len() as u32).collect();
            order.sort_by(|a, b| {
                let (a, b) = (&bodies[*a as usize], &bodies[*b as usize]);
                a.pos.x.total_cmp(&b.pos.x).then(a.tag.cmp(&b.tag))
            });
            order
        };
        // From scratch, serial.
        let mut serial = PhysicsScratch::new();
        serial.sweep_only = true;
        step(&mut bodies.clone(), &config, dt, &mut serial);
        assert_eq!(x_order_of(&serial), sorted_by_hand);
        // From scratch, on the pool.
        let mut pooled = PhysicsScratch::new();
        pooled.sweep_only = true;
        pooled.threads = 4;
        pooled.min_bodies = 1;
        step(&mut bodies.clone(), &config, dt, &mut pooled);
        assert_eq!(x_order_of(&pooled), sorted_by_hand);
        // Carried: a step with a few bodies moved, then the real positions again (insertion repair).
        let mut carried = PhysicsScratch::new();
        carried.sweep_only = true;
        let mut moved = bodies.clone();
        for index in (0..moved.len()).step_by(97) {
            moved[index].pos.x += 23.0;
        }
        step(&mut moved, &config, dt, &mut carried);
        step(&mut bodies.clone(), &config, dt, &mut carried);
        assert_eq!(x_order_of(&carried), sorted_by_hand);
        // Carried but far from sorted (every body somewhere else): the repair gives up and sorts from scratch.
        let mut scrambled = PhysicsScratch::new();
        scrambled.sweep_only = true;
        let mut shuffled = bodies.clone();
        for index in 0..shuffled.len() {
            shuffled[index].pos.x = ((index * 7919) % shuffled.len()) as f32;
        }
        step(&mut shuffled, &config, dt, &mut scrambled);
        step(&mut bodies.clone(), &config, dt, &mut scrambled);
        assert_eq!(x_order_of(&scrambled), sorted_by_hand);
    }

    /// A bibite holding a maze pellet (a plant of 423,000 u^2 has a collider of radius 364, and in the maze save, with plants at a mass density of 1e35, a mass past the largest
    /// f32): the weld between them is 364 units long, and the holder, which weighs a few units, is the only body of the pair that can move. The weld's anchor used to be the
    /// held body's origin, 364 units from the holder, and the 3 x 3 block that the weld solves (`b2WeldJoint`'s) then has terms of 7,000 and a determinant of 0.02: solved
    /// by cofactors in f32 it left the holder turning at 34 rad/s after the solve that was to stop it, the step turned it 0.3 rad (a hundred units at the anchor), and the
    /// relax solve that follows answered with a velocity of 2,500 u/s and a spin of 570 rad/s (the maze save of 2026-11-15: a bibite that grabbed a meat pellet pressed against
    /// the giant it already held left at 434 u/s, spinning at 414 rad/s, and what it hit died). The game's joint is anchored at the holder's origin: nothing is levered.
    #[test]
    fn a_light_body_welded_to_a_giant_one_far_off_is_not_thrown_by_its_own_turning() {
        let config = PhysicsConfig::default();
        let dt = 1.0 / 20.0;
        let radius = 363.6;
        // (The holder's rotation is an unwrapped angle of the long life of the bibite: -155 rad, as in the save.)
        let rot0 = -155.09f32;
        for (turn, spin_in, speed_in, thrust, torque, label) in [
            (0.09f32, -1.27f32, Vec2::new(9.4, 2.8), Vec2::new(18.0, 25.0), 15.0f32, "turning and pushing"),
            (-0.09, 1.9, Vec2::new(-6.0, 4.0), Vec2::new(-12.0, 30.0), -9.0, "turning the other way and pushing"),
            (0.0, 0.7, Vec2::new(0.0, 12.0), Vec2::new(0.0, 40.0), 0.0, "straight and pushing at the giant"),
            (0.0, -2.5, Vec2::new(25.0, 0.0), Vec2::new(25.0, 0.0), 12.0, "spinning and pushing along its rim"),
        ] {
            let mut holder = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.7, 17.3);
            holder.pos = Vec2::ZERO;
            let _ = holder.add_collider(Collider::solid(Shape::circle(4.0)));
            // (The maze save's plants have a mass density of 1e35: a pellet of 423,000 u^2 is past the largest f32, an infinite mass with no inverse.)
            let mut giant = PhysicsBody::new_dynamic(2, LAYER_PELLETS, f32::INFINITY, f32::INFINITY);
            giant.pos = Vec2::new(78.5, 358.2);
            let _ = giant.add_collider(Collider::solid(Shape::circle(radius)));
            // The weld froze the pose they had: the giant's origin in the holder's frame, and the relative rotation.
            let weld = WeldDef { held: 1, holder: 0, anchor: giant.pos.rotate(-rot0), angle: -rot0, break_force: 0.0 };
            holder.rot = rot0 + turn;
            holder.vel = speed_in;
            holder.ang_vel = spin_in;
            holder.force = thrust;
            holder.torque = torque;
            let mut bodies = vec![holder, giant];
            let mut scratch = PhysicsScratch::new();
            let (mut fastest, mut spin, mut drift) = (0.0f32, 0.0f32, 0.0f32);
            for _ in 0..40 {
                scratch.add_weld(weld);
                step(&mut bodies, &config, dt, &mut scratch);
                fastest = fastest.max(bodies[0].vel.length());
                spin = spin.max(bodies[0].ang_vel.abs());
                drift = drift.max(bodies[0].pos.length());
            }
            assert!(fastest < 30.0, "{label}: the holder left at {fastest} u/s");
            assert!(spin < 3.0, "{label}: the holder spun at {spin} rad/s");
            assert!(drift < 8.0, "{label}: the holder was moved {drift} units");
            assert!(bodies[1].vel.length() < 0.01, "{label}: the giant moved");
        }
    }

    /// A pellet in two mouths is held by two welds. Only the weld whose two ends are far apart gives way (a holder wrapped across the world), and the step names
    /// the holder it was: the other holder keeps its hold (the first holder of the held body used to be the one that let go).
    #[test]
    fn the_holder_of_the_weld_that_gave_way_is_named_when_a_body_has_two() {
        let config = PhysicsConfig::default();
        let dt = 1.0 / 20.0;
        let mut near = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.7, 17.3);
        near.pos = Vec2::ZERO;
        let _ = near.add_collider(Collider::solid(Shape::circle(4.0)));
        let mut far = PhysicsBody::new_dynamic(2, LAYER_BIBITE, 1.7, 17.3);
        far.pos = Vec2::new(1000.0, 0.0);
        let _ = far.add_collider(Collider::solid(Shape::circle(4.0)));
        let mut pellet = PhysicsBody::new_dynamic(3, LAYER_PELLETS, 5.0, 5.0);
        pellet.pos = Vec2::new(12.0, 0.0);
        let _ = pellet.add_collider(Collider::solid(Shape::circle(3.0)));
        // Both welds were made with the pellet 12 units from the holder; the second holder is 1000 units from where the pellet is.
        let weld = |holder: u32| WeldDef { held: 2, holder, anchor: Vec2::new(12.0, 0.0), angle: 0.0, break_force: 0.0 };
        for (order, label) in [([0u32, 1u32], "the holder that stays first"), ([1, 0], "the holder that lets go first")] {
            let mut bodies = vec![near, far, pellet];
            let mut scratch = PhysicsScratch::new();
            for holder in order {
                scratch.add_weld(weld(holder));
            }
            step(&mut bodies, &config, dt, &mut scratch);
            assert_eq!(scratch.broken_welds(), &[(2, 2)], "{label}: the pellet's weld to the holder with tag 2 gave way, and only that one");
        }
    }

    /// A bibite that holds two others (the glitch save of 2026-11-20: a pile of bibites and pellets that threw everything that touched it, and was gone after a reload).
    /// A weld anchors what is held at the holder's origin, so the holder and the bodies it holds overlap by construction, and their contacts have nothing to push apart: the
    /// weld is rigid. Those contacts were still solved as velocity constraints and carried their impulses from tick to tick; a holder that turned (its spin sits on the
    /// clamp) wound them up by 100 to 250 a tick, past a thousand in ten ticks, and the light bodies near them were thrown at the speed limit. The contacts inside a weld
    /// group do not push (`constraint.welded`: not by the soft bias, not by the position solve, and now not by the velocity solve either).
    #[test]
    fn the_contacts_inside_a_weld_group_carry_no_impulse_while_the_holder_turns() {
        let config = PhysicsConfig::default();
        let dt = 1.0 / 30.0;
        // (anchor of the first held body, anchor of the second, holder's spin to begin with, its torque, a light pellet that touches the group)
        for (label, first_at, second_at, spin, torque, pellet) in [
            ("overlapping, turning", Vec2::new(4.0, 0.0), Vec2::new(-3.0, 3.0), 0.0f32, 40.0f32, false),
            ("overlapping, spin on the clamp", Vec2::new(4.0, 0.0), Vec2::new(-3.0, 3.0), 6.28, 40.0, false),
            ("held a little further off", Vec2::new(8.0, 0.0), Vec2::new(-6.0, 6.0), 6.28, 40.0, false),
            ("on the holder's origin", Vec2::new(0.5, 0.0), Vec2::new(-0.5, 0.5), 6.28, 40.0, false),
            ("with a light pellet touching", Vec2::new(4.0, 0.0), Vec2::new(-3.0, 3.0), 6.28, 40.0, true),
        ] {
            let mut holder = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 0.675, 4.1);
            let _ = holder.add_collider(Collider::solid(Shape::circle(3.5)));
            let mut first = PhysicsBody::new_dynamic(2, LAYER_BIBITE, 2.0, 9.0);
            first.pos = first_at;
            let _ = first.add_collider(Collider::solid(Shape::circle(3.0)));
            let mut second = PhysicsBody::new_dynamic(3, LAYER_BIBITE, 1.85, 8.0);
            second.pos = second_at;
            let _ = second.add_collider(Collider::solid(Shape::circle(3.0)));
            // Both welds froze the pose they were made in: the held body's origin in the holder's frame, and the relative rotation.
            let welds = [
                WeldDef { held: 1, holder: 0, anchor: first_at, angle: 0.0, break_force: 0.0 },
                WeldDef { held: 2, holder: 0, anchor: second_at, angle: 0.0, break_force: 0.0 },
            ];
            // The holder turns as hard as its muscles let it, and pushes.
            holder.torque = torque;
            holder.ang_vel = spin;
            holder.force = Vec2::new(30.0, 10.0);
            let mut bodies = vec![holder, first, second];
            if pellet {
                let mut light = PhysicsBody::new_dynamic(4, LAYER_PELLETS, 0.0168, 0.0003);
                light.pos = Vec2::new(first_at.x + 3.2, first_at.y + 1.0);
                let _ = light.add_collider(Collider::solid(Shape::circle(0.6)));
                bodies.push(light);
            }
            let mut scratch = PhysicsScratch::new();
            let (mut fastest, mut inside, mut outside) = (0.0f32, 0.0f32, 0.0f32);
            for _ in 0..60 {
                for weld in welds {
                    scratch.add_weld(weld);
                }
                step(&mut bodies, &config, dt, &mut scratch);
                for body in &bodies[..3] {
                    fastest = fastest.max(body.vel.length());
                }
                for contact in scratch.contacts() {
                    if contact.tag_a < 4 && contact.tag_b < 4 {
                        inside = inside.max(contact.normal_impulse.abs());
                    } else {
                        outside = outside.max(contact.normal_impulse.abs());
                    }
                }
            }
            assert_eq!(inside, 0.0, "{label}: a contact inside the weld group carried an impulse");
            assert!(fastest < 30.0, "{label}: the group left at {fastest} u/s");
            if pellet {
                assert!(outside > 0.0, "{label}: the light pellet that touches the group is not pushed by it any more");
            }
        }
    }

    /// What a held body does to a body outside its group is unchanged: a pellet held by a holder still pushes a free body that it overlaps.
    #[test]
    fn a_held_body_still_pushes_a_body_that_is_not_in_its_weld_group() {
        let config = PhysicsConfig::default();
        let dt = 1.0 / 30.0;
        let mut holder = PhysicsBody::new_dynamic(1, LAYER_BIBITE, 1.7, 17.3);
        let _ = holder.add_collider(Collider::solid(Shape::circle(4.0)));
        let mut held = PhysicsBody::new_dynamic(2, LAYER_PELLETS, 3.0, 3.0);
        held.pos = Vec2::new(12.0, 0.0);
        let _ = held.add_collider(Collider::solid(Shape::circle(3.0)));
        let mut free = PhysicsBody::new_dynamic(3, LAYER_BIBITE, 1.0, 4.0);
        free.pos = Vec2::new(16.5, 0.0);
        let _ = free.add_collider(Collider::solid(Shape::circle(3.0)));
        let weld = WeldDef { held: 1, holder: 0, anchor: Vec2::new(12.0, 0.0), angle: 0.0, break_force: 0.0 };
        let mut bodies = vec![holder, held, free];
        let mut scratch = PhysicsScratch::new();
        scratch.add_weld(weld);
        for _ in 0..5 {
            scratch.add_weld(weld);
            step(&mut bodies, &config, dt, &mut scratch);
        }
        // (A resting overlap is taken out by the position solve, not by a velocity.)
        assert!(bodies[2].pos.x > 16.5 + 0.1, "the free body that overlaps the held pellet was not pushed away: {:?}", bodies[2].pos);
    }

    /// A loop of welds (the "deadly pile" save of 2026-11-22: two clumps of pellets that threw and killed what touched them, and were quiet again after a reload). The island is
    /// the one the solver trace showed at tick 49,967 of a replay of that save: a heavy bibite held a light bibite, a small pellet and a heavy pellet; the light bibite held a
    /// second light bibite; and that one held a pellet and the same heavy pellet again (a body can be held by two mouths, as in the game): six rigid welds between six bodies, with
    /// masses from 11 to 0.05 and inertias from 500 to 0.02, each body pushed by its muscles and slowed by its own drag. Box2D 2.4 solves the joints in every one of its velocity
    /// iterations, which makes a group like that move as one; a single pass over them left the welded bodies' velocities 70-160 u/s apart at the end of every tick (the first pass
    /// threw them to 110-185 u/s and moved them four units, the relax pass put most of the velocities back and the position solve the bodies), and what the muscles added each
    /// tick stayed: within twenty ticks bibites were thrown at the speed limit and what they hit died.
    #[test]
    fn a_loop_of_welds_moves_as_one_body() {
        let config = PhysicsConfig::default();
        let dt = 1.0 / 30.0;
        // (mass, inertia, linear damping, position, rotation, velocity, spin, the muscles' force, their torque), heavy holder first. Linear damping is the drag coefficient
        // (5) times the size over the mass; angular damping is half the coefficient.
        let specs: [(f32, f32, f32, Vec2, f32, Vec2, f32, Vec2, f32); 6] = [
            (11.11, 500.0, 10.7, Vec2::new(-83.49, 248.03), -459.67, Vec2::new(-22.4, 9.5), -1.30, Vec2::new(339.48, 220.97), -101.26),
            (2.02, 22.2, 28.8, Vec2::new(-68.44, 247.33), -63.07, Vec2::new(45.4, 27.2), 0.08, Vec2::new(10.66, 44.60), -11.01),
            (2.31, 26.3, 25.7, Vec2::new(-60.73, 251.18), 19.40, Vec2::new(33.9, 25.9), 2.78, Vec2::new(-83.18, 134.33), -80.37),
            (0.256, 0.685, 91.0, Vec2::new(-61.11, 259.16), 4.51, Vec2::new(51.0, 34.7), -0.75, Vec2::ZERO, 0.0),
            (2.72, 77.0, 28.0, Vec2::new(-68.45, 262.50), 13.24, Vec2::new(7.3, -1.4), 2.78, Vec2::ZERO, 0.0),
            (0.0546, 0.0186, 90.0, Vec2::new(-73.32, 250.13), -0.10, Vec2::new(-42.7, -39.2), -3.80, Vec2::ZERO, 0.0),
        ];
        // (holder, held): 0 holds 5, 1 and 4; 1 holds 2; 2 holds 3 and 4 again.
        let pairs: [(u32, u32); 6] = [(0, 5), (0, 1), (0, 4), (1, 2), (2, 3), (2, 4)];
        let mut bodies: Vec<PhysicsBody> = Vec::new();
        for (index, &(mass, inertia, damping, pos, rot, vel, spin, force, torque)) in specs.iter().enumerate() {
            let layer = if index < 3 { LAYER_BIBITE } else { LAYER_PELLETS };
            let mut body = PhysicsBody::new_dynamic(index as u64 + 1, layer, mass, inertia);
            body.pos = pos;
            body.rot = rot;
            body.vel = vel;
            body.ang_vel = spin;
            body.linear_damping = damping;
            body.angular_damping = 2.5;
            // (The bodies lie as close together as the real ones did: the neighbours touch.)
            let radius = [8.0, 5.0, 5.0, 3.5, 8.0, 0.6][index];
            let _ = body.add_collider(Collider::solid(Shape::circle(radius)));
            body.force = force;
            body.torque = torque;
            bodies.push(body);
        }
        // Each weld froze the held body's pose in the holder's frame when the grab was made.
        let welds: Vec<WeldDef> = pairs
            .iter()
            .map(|&(holder, held)| {
                let (a, b) = (&bodies[holder as usize], &bodies[held as usize]);
                WeldDef { held, holder, anchor: (b.pos - a.pos).rotate(-a.rot), angle: b.rot - a.rot, break_force: 0.0 }
            })
            .collect();
        // Two bodies are tied when a weld joins them or one holder holds both (the tags are the places plus one).
        let tag = |index: u32| u64::from(index) + 1;
        let tied = |a: u64, b: u64| {
            pairs.iter().any(|&(holder, held)| (tag(holder) == a && tag(held) == b) || (tag(holder) == b && tag(held) == a))
                || pairs.iter().any(|&(first, held)| tag(held) == a && pairs.iter().any(|&(second, other)| tag(other) == b && second == first))
        };
        let mut scratch = PhysicsScratch::new();
        let (mut residual, mut fastest, mut inside, mut touching) = (0.0f32, 0.0f32, 0.0f32, 0usize);
        for tick in 0..40 {
            for weld in &welds {
                scratch.add_weld(*weld);
            }
            step(&mut bodies, &config, dt, &mut scratch);
            for contact in scratch.contacts() {
                if tied(contact.tag_a, contact.tag_b) {
                    touching += 1;
                    inside = inside.max(contact.normal_impulse.abs());
                }
            }
            // The group settles in a few ticks; after that every tick must end with the welded bodies moving together.
            if tick < 10 {
                continue;
            }
            for body in &bodies {
                fastest = fastest.max(body.vel.length());
            }
            for &(holder, held) in &pairs {
                // What a weld keeps at zero: the velocity of the holder's origin as a point of the held body against the holder's own, and the difference of the spins.
                let (a, b) = (&bodies[holder as usize], &bodies[held as usize]);
                residual = residual.max((b.vel + (a.pos - b.pos).perp() * b.ang_vel - a.vel).length() + (b.ang_vel - a.ang_vel).abs() * 10.0);
            }
        }
        assert!(touching > 0, "the bodies of the group do not touch: the test checks nothing");
        assert_eq!(inside, 0.0, "a contact between bodies that a weld ties carried an impulse");
        assert!(residual < 30.0, "welded bodies ended a tick {residual} (u/s, plus ten times rad/s) apart");
        assert!(fastest < 100.0, "the weld group reached {fastest} u/s");
    }

    #[test]
    fn empty_step_is_noop() {
        let config = PhysicsConfig::default();
        let mut bodies: [PhysicsBody; 0] = [];
        let mut scratch = PhysicsScratch::new();
        step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
        assert!(scratch.contacts().is_empty());
        assert!(scratch.triggers().is_empty());
        step(&mut bodies, &config, 0.0, &mut scratch);
    }
}

#[cfg(test)]
mod trigger_completeness_tests {
    use super::*;

    fn lcg(state: &mut u64) -> f32 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((*state >> 33) as u32) as f32 / (u32::MAX >> 1) as f32
    }

    fn build_world(bibites: usize, pellets: usize, spread: f32) -> Vec<PhysicsBody> {
        let mut bodies = Vec::with_capacity(bibites + pellets);
        let mut state = 0x5EED_u64;
        for index in 0..bibites {
            let mut body = PhysicsBody::new_dynamic(index as u64, 8, 1.0, 2.0);
            body.pos = Vec2::new(lcg(&mut state) - 0.5, lcg(&mut state) - 0.5) * spread;
            body.add_collider(Collider::solid(Shape::circle(5.0)));
            body.add_collider(Collider::sensor(Shape::box_from_size(Vec2::new(7.0, 2.72))));
            bodies.push(body);
        }
        for index in 0..pellets {
            let mut body = PhysicsBody::new_static((bibites + index) as u64, 9);
            body.pos = Vec2::new(lcg(&mut state) - 0.5, lcg(&mut state) - 0.5) * spread;
            body.add_collider(Collider::solid(Shape::circle(2.5)));
            bodies.push(body);
        }
        bodies
    }

    fn sensor_overlaps(a: &PhysicsBody, b: &PhysicsBody, config: &PhysicsConfig) -> usize {
        let mut count = 0;
        for collider_a in a.colliders() {
            let pose_a = collider_a.world_pose(a.pose());
            for collider_b in b.colliders() {
                if !(collider_a.is_sensor || collider_b.is_sensor) {
                    continue;
                }
                let pose_b = collider_b.world_pose(b.pose());
                if collider_a
                    .aabb(a.pose())
                    .overlaps(&collider_b.aabb(b.pose()))
                    && collide_with_skin(
                        &collider_a.shape,
                        pose_a,
                        &collider_b.shape,
                        pose_b,
                        config.contact_offset,
                    )
                    .is_some()
                {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn trigger_pairs_cover_every_sensor_overlap_at_ecology_density() {
        // 36k Default is tens of bibites plus ~650 pellets over a 2000-unit
        // world; include scale-benchmark densities too, but not the pathological
        // all-overlapping cluster (which intentionally exercises the dense cap).
        for (bibites, pellets, spread) in [
            (20usize, 650usize, 2000.0f32),
            (50, 650, 2000.0),
            (500, 2000, 4000.0),
            (1500, 4000, 2000.0),
        ] {
            let mut bodies = build_world(bibites, pellets, spread);
            let before = bodies.clone();
            let config = PhysicsConfig::default();
            let mut scratch = PhysicsScratch::with_capacity(bodies.len(), bodies.len() * 4, 20_000);
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
            let mut expected = 0usize;
            for i in 0..before.len() {
                for j in (i + 1)..before.len() {
                    if before[i].tag != before[j].tag
                        && collision_allowed(before[i].layer, before[j].layer)
                    {
                        expected += sensor_overlaps(&before[i], &before[j], &config);
                    }
                }
            }
            assert_eq!(
                scratch.triggers().len(),
                expected,
                "missed sensor overlap at bibites={bibites} pellets={pellets}"
            );
        }
    }

    /// What a step leaves, as bits: the bodies, the contacts and the triggers.
    fn outcome(bodies: &[PhysicsBody], scratch: &PhysicsScratch) -> String {
        let mut text = String::new();
        for body in bodies {
            text.push_str(&format!(
                "{:x} {:x} {:x} {:x} {:x} {:x}\n",
                body.pos.x.to_bits(),
                body.pos.y.to_bits(),
                body.rot.to_bits(),
                body.vel.x.to_bits(),
                body.vel.y.to_bits(),
                body.ang_vel.to_bits()
            ));
        }
        text.push_str(&format!(
            "{:?}\n{:?}\n",
            scratch.contacts(),
            scratch.triggers()
        ));
        text
    }

    /// Every stage the pool takes (mirror, boxes, x order and entries, sweep, poses, narrowphase cut into chunks of one pair
    /// each) gives the step the serial one gives, bit for bit, over several steps of a crowded world.
    #[test]
    fn a_pooled_step_equals_the_serial_step_whatever_the_chunks() {
        for (bibites, pellets, spread) in [(300usize, 400usize, 500.0f32), (1200, 3000, 1500.0)] {
            let config = PhysicsConfig::default();
            let mut serial_bodies = build_world(bibites, pellets, spread);
            // Let the dynamic bodies move: give them velocities (the pellets stay put).
            let mut state = 0xC0FFEE_u64;
            for body in serial_bodies.iter_mut().take(bibites) {
                body.vel = Vec2::new(lcg(&mut state) - 0.5, lcg(&mut state) - 0.5) * 40.0;
                body.ang_vel = lcg(&mut state) - 0.5;
            }
            let mut pooled_bodies = serial_bodies.clone();
            let mut serial = PhysicsScratch::new();
            let mut pooled = PhysicsScratch::new();
            pooled.threads = 4;
            pooled.min_bodies = 1;
            pooled.min_broadphase = 1;
            pooled.min_narrow_pairs = 1;
            let mut contacts = 0;
            for round in 0..6 {
                step(&mut serial_bodies, &config, 1.0 / 30.0, &mut serial);
                step(&mut pooled_bodies, &config, 1.0 / 30.0, &mut pooled);
                contacts += serial.contacts().len();
                assert!(
                    outcome(&serial_bodies, &serial) == outcome(&pooled_bodies, &pooled),
                    "round {round} of {bibites} bibites: the pooled step differs from the serial one"
                );
            }
            assert!(contacts > 0, "the world never produced a contact");
        }
    }

    #[test]
    fn dense_cluster_pair_count_is_bounded() {
        for (bibites, pellets, bound) in [(2000usize, 4000usize, 40_000usize), (4000, 8000, 60_000)]
        {
            let mut bodies = build_world(bibites, pellets, 800.0);
            let config = PhysicsConfig::default();
            let mut scratch = PhysicsScratch::with_capacity(bodies.len(), bodies.len() * 4, 20_000);
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
            assert!(
                scratch.candidate_pair_count() <= bound,
                "dense cluster {bibites}: {} candidate pairs > {bound}",
                scratch.candidate_pair_count()
            );
        }
    }

    #[test]
    #[ignore = "extreme dense cluster intentionally exercises the per-body pair cap"]
    fn dense_cluster_sensor_pairs_are_prioritized() {
        for (bibites, pellets) in [(2000usize, 4000usize), (4000, 8000)] {
            let mut bodies = build_world(bibites, pellets, 800.0);
            let config = PhysicsConfig::default();
            let mut scratch = PhysicsScratch::with_capacity(bodies.len(), bodies.len() * 4, 20_000);
            let start = std::time::Instant::now();
            step(&mut bodies, &config, 1.0 / 30.0, &mut scratch);
            eprintln!(
                "dense cluster {bibites}: elapsed_ms={:.2} candidates={} triggers={}",
                start.elapsed().as_secs_f64() * 1000.0,
                scratch.candidate_pair_count(),
                scratch.triggers().len()
            );
            assert!(!scratch.triggers().is_empty());
        }
    }
}
