//! World sensing: the world-derived brain inputs ([`crate::senses::SenseOutputs`]), field of
//! view (FOV) math and pheromone-sensing math.
//!
//! Spec: `docs/spec/03-world-ecology.md` §3 (FieldOfView), §4 (Pherosense),
//! Appendix B; interface contract: `docs/INTERFACES.md` §5/§6.
//!
//! # Split between world and body senses
//!
//! The brain has 33 inputs (`brain::N_INPUTS`). The **world** owns 21 of them
//! and fills them through [`crate::senses::SenseOutputs`] / the [`brain::WorldSenseSource`]
//! trait:
//!
//! | Inputs | Owner | Contents |
//! |---|---|---|
//! | 0–8 | body | energy, maturity, life ratio, fullness, speed, rotation speed, grabbing, attacked damage, eggs stored |
//! | 9–20 | **world** | bibite/plant/meat closeness, angle and count/4, closest-bibite RGB |
//! | 21–23 | body | clock `tic`, chronological time, time alive |
//! | 24–32 | **world** | pheromone `tanh(sum/5)` per channel, source angles, gradient headings |
//!
//! # Layering
//!
//! This module is deliberately **pure math + plain data**: no `BibiteStore` /
//! `World` dependency. The world tick queries the spatial hash (`QueryScratch`
//! cap = 128, deterministic order), looks up entity views, and feeds
//! [`crate::senses::FovSensor`] / [`crate::senses::PheroAccumulator`]; the result is written into
//! [`crate::senses::SenseOutputs`]. The body owns the three throttle counters (spec §3.7) and
//! decides when to call the world sensing phase; the world only recomputes when
//! told to.
//!
//! The game's 128-object physics-query cap is represented by the spatial-hash
//! [`crate::world::QueryScratch`]; the accumulators themselves do
//! not truncate.

use crate::brain::{self, Brain, BrainInputs};
use crate::math::{PI, Vec2};
use crate::pellet::NORMAL_AMOUNT_AT_SCALE1;
use crate::pheromone::SENSE_TANH_DIVISOR;

/// Number of world-derived brain inputs (inputs 9–20 and 24–32).
pub const WORLD_INPUT_COUNT: usize = 21;

/// FOV weight constant: `(1.05 − d/R)` (`FieldOfView.ComputeSenses`).
pub const FOV_WEIGHT_BASE: f32 = 1.05;

/// Corpse meat-weight divisor: `baseBodyArea / 20` (`FieldOfView.cs:452`).
pub const CORPSE_SIZE_DIVISOR: f32 = 20.0;

/// Bibite weight divisor: `maxHealth / 100` (`FieldOfView.cs:467`).
pub const BIBITE_SIZE_DIVISOR: f32 = 100.0;

/// Red-death pseudo-pheromone base strength (`Pherosense.cs:203`: `100/Δ`).
pub const RED_DEATH_PHERO_STRENGTH: f32 = 100.0;

// ---------------------------------------------------------------------------
// SenseOutputs: the 21 world-derived brain inputs
// ---------------------------------------------------------------------------

/// The 21 world-derived brain inputs (vision + pheromone), in
/// [`brain::Input`] index order for the `values` helpers.
///
/// All fields are raw values as written to the brain; the world computation
/// already applies the game's normalisations (counts divided by 4, angles
/// divided by 180°, pheromone `tanh(sum/5)`).
///
/// The three **herd** fields at the end are not brain inputs: they carry the
/// game's `FieldOfView` herd outputs (`hasHerd`, `herdSumDirection` in signed
/// degrees, `herdSeparationProjection`) so the body's herding steering organ can
/// consume them from the same per-bibite snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SenseOutputs {
    /// 0..1: weighted-average bibite closeness (`BibiteCloseness`).
    pub bibite_closeness: f32,
    /// Normalized angle to the weighted-average bibite (`BibiteAngle`).
    pub bibite_angle: f32,
    /// Seen bibite count divided by four (`NBibites`).
    pub n_bibites: f32,
    /// 0..1: weighted-average plant closeness (`PlantCloseness`).
    pub plant_closeness: f32,
    /// Normalized angle to the weighted-average plant (`PlantAngle`).
    pub plant_angle: f32,
    /// Seen plant count divided by four (`NPlants`).
    pub n_plants: f32,
    /// 0..1: weighted-average meat closeness (`MeatCloseness`).
    pub meat_closeness: f32,
    /// Normalized angle to the weighted-average meat (`MeatAngle`).
    pub meat_angle: f32,
    /// Seen meat (pellets + corpses) count divided by four (`NMeats`).
    pub n_meats: f32,
    /// Red gene of the closest seen bibite (`RedBibite`).
    pub red_bibite: f32,
    /// Green gene of the closest seen bibite (`GreenBibite`).
    pub green_bibite: f32,
    /// Blue gene of the closest seen bibite (`BlueBibite`).
    pub blue_bibite: f32,
    /// `tanh(pheroSum1 / 5)` (`PheroSense1`).
    pub phero_sense_1: f32,
    /// `tanh(pheroSum2 / 5)` (`PheroSense2`).
    pub phero_sense_2: f32,
    /// `tanh(pheroSum3 / 5)` (`PheroSense3`).
    pub phero_sense_3: f32,
    /// Normalized angle to the red pheromone source (`Phero1Angle`).
    pub phero_1_angle: f32,
    /// Normalized angle to the green pheromone source (`Phero2Angle`).
    pub phero_2_angle: f32,
    /// Normalized angle to the blue pheromone source (`Phero3Angle`).
    pub phero_3_angle: f32,
    /// Normalized red pheromone gradient heading (`Phero1Heading`).
    pub phero_1_heading: f32,
    /// Normalized green pheromone gradient heading (`Phero2Heading`).
    pub phero_2_heading: f32,
    /// Normalized blue pheromone gradient heading (`Phero3Heading`).
    pub phero_3_heading: f32,
    /// `FieldOfView.hasHerd` (body herding steering input, not a brain input).
    pub has_herd: bool,
    /// `FieldOfView.herdSumDirection`, signed degrees (body steering input).
    pub herd_sum_direction: f32,
    /// `FieldOfView.herdSeparationProjection` (body steering input).
    pub herd_separation_projection: f32,
}

impl SenseOutputs {
    /// All-zero senses (the game's state when a sense channel is not wired).
    #[inline]
    pub const fn zeroed() -> Self {
        Self {
            bibite_closeness: 0.0,
            bibite_angle: 0.0,
            n_bibites: 0.0,
            plant_closeness: 0.0,
            plant_angle: 0.0,
            n_plants: 0.0,
            meat_closeness: 0.0,
            meat_angle: 0.0,
            n_meats: 0.0,
            red_bibite: 0.0,
            green_bibite: 0.0,
            blue_bibite: 0.0,
            phero_sense_1: 0.0,
            phero_sense_2: 0.0,
            phero_sense_3: 0.0,
            phero_1_angle: 0.0,
            phero_2_angle: 0.0,
            phero_3_angle: 0.0,
            phero_1_heading: 0.0,
            phero_2_heading: 0.0,
            phero_3_heading: 0.0,
            has_herd: false,
            herd_sum_direction: 0.0,
            herd_separation_projection: 0.0,
        }
    }

    /// The 21 values in [`brain::Input`] index order (9–20 then 24–32).
    ///
    /// Used by parity/digest tooling and io; allocation-free.
    #[must_use]
    pub fn values(&self) -> [f32; WORLD_INPUT_COUNT] {
        [
            self.bibite_closeness,
            self.bibite_angle,
            self.n_bibites,
            self.plant_closeness,
            self.plant_angle,
            self.n_plants,
            self.meat_closeness,
            self.meat_angle,
            self.n_meats,
            self.red_bibite,
            self.green_bibite,
            self.blue_bibite,
            self.phero_sense_1,
            self.phero_sense_2,
            self.phero_sense_3,
            self.phero_1_angle,
            self.phero_2_angle,
            self.phero_3_angle,
            self.phero_1_heading,
            self.phero_2_heading,
            self.phero_3_heading,
        ]
    }

    /// Builds senses from the 21 values in [`brain::Input`] index order.
    #[must_use]
    pub fn from_values(values: [f32; WORLD_INPUT_COUNT]) -> Self {
        Self {
            bibite_closeness: values[0],
            bibite_angle: values[1],
            n_bibites: values[2],
            plant_closeness: values[3],
            plant_angle: values[4],
            n_plants: values[5],
            meat_closeness: values[6],
            meat_angle: values[7],
            n_meats: values[8],
            red_bibite: values[9],
            green_bibite: values[10],
            blue_bibite: values[11],
            phero_sense_1: values[12],
            phero_sense_2: values[13],
            phero_sense_3: values[14],
            phero_1_angle: values[15],
            phero_2_angle: values[16],
            phero_3_angle: values[17],
            phero_1_heading: values[18],
            phero_2_heading: values[19],
            phero_3_heading: values[20],
            has_herd: false,
            herd_sum_direction: 0.0,
            herd_separation_projection: 0.0,
        }
    }

    /// Writes these senses into a [`BrainInputs`] vector (the 21 world inputs).
    #[inline]
    pub fn write_into(&self, inputs: &mut BrainInputs) {
        inputs.write_world_senses(self);
    }

    /// Writes an [`FovResult`] into the vision inputs (inputs 9–20).
    #[inline]
    pub fn apply_fov(&mut self, fov: &FovResult) {
        self.bibite_closeness = fov.bibite.closeness;
        self.bibite_angle = fov.bibite.angle;
        self.n_bibites = fov.bibite.count as f32 / 4.0;
        self.plant_closeness = fov.plant.closeness;
        self.plant_angle = fov.plant.angle;
        self.n_plants = fov.plant.count as f32 / 4.0;
        self.meat_closeness = fov.meat.closeness;
        self.meat_angle = fov.meat.angle;
        self.n_meats = fov.meat.count as f32 / 4.0;
        self.red_bibite = fov.target_color[0];
        self.green_bibite = fov.target_color[1];
        self.blue_bibite = fov.target_color[2];
        self.has_herd = fov.herd.has_herd;
        self.herd_sum_direction = fov.herd.sum_direction;
        self.herd_separation_projection = fov.herd.separation_projection;
    }

    /// Writes a [`PheroSenses`] into the pheromone inputs (inputs 24–32).
    #[inline]
    pub fn apply_phero(&mut self, phero: &PheroSenses) {
        self.phero_sense_1 = phero.sense[0];
        self.phero_sense_2 = phero.sense[1];
        self.phero_sense_3 = phero.sense[2];
        self.phero_1_angle = phero.angle[0];
        self.phero_2_angle = phero.angle[1];
        self.phero_3_angle = phero.angle[2];
        self.phero_1_heading = phero.heading[0];
        self.phero_2_heading = phero.heading[1];
        self.phero_3_heading = phero.heading[2];
    }
}

impl brain::WorldSenseSource for SenseOutputs {
    #[inline]
    fn bibite_closeness(&self) -> f32 {
        self.bibite_closeness
    }
    #[inline]
    fn bibite_angle(&self) -> f32 {
        self.bibite_angle
    }
    #[inline]
    fn n_bibites(&self) -> f32 {
        self.n_bibites
    }
    #[inline]
    fn plant_closeness(&self) -> f32 {
        self.plant_closeness
    }
    #[inline]
    fn plant_angle(&self) -> f32 {
        self.plant_angle
    }
    #[inline]
    fn n_plants(&self) -> f32 {
        self.n_plants
    }
    #[inline]
    fn meat_closeness(&self) -> f32 {
        self.meat_closeness
    }
    #[inline]
    fn meat_angle(&self) -> f32 {
        self.meat_angle
    }
    #[inline]
    fn n_meats(&self) -> f32 {
        self.n_meats
    }
    #[inline]
    fn red_bibite(&self) -> f32 {
        self.red_bibite
    }
    #[inline]
    fn green_bibite(&self) -> f32 {
        self.green_bibite
    }
    #[inline]
    fn blue_bibite(&self) -> f32 {
        self.blue_bibite
    }
    #[inline]
    fn phero_sense_1(&self) -> f32 {
        self.phero_sense_1
    }
    #[inline]
    fn phero_sense_2(&self) -> f32 {
        self.phero_sense_2
    }
    #[inline]
    fn phero_sense_3(&self) -> f32 {
        self.phero_sense_3
    }
    #[inline]
    fn phero_1_angle(&self) -> f32 {
        self.phero_1_angle
    }
    #[inline]
    fn phero_2_angle(&self) -> f32 {
        self.phero_2_angle
    }
    #[inline]
    fn phero_3_angle(&self) -> f32 {
        self.phero_3_angle
    }
    #[inline]
    fn phero_1_heading(&self) -> f32 {
        self.phero_1_heading
    }
    #[inline]
    fn phero_2_heading(&self) -> f32 {
        self.phero_2_heading
    }
    #[inline]
    fn phero_3_heading(&self) -> f32 {
        self.phero_3_heading
    }
}

// ---------------------------------------------------------------------------
// FOV: lazy enable mask, parameters and target visibility
// ---------------------------------------------------------------------------

/// The game's two FOV throttle phases for one bibite in a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FovCadence {
    /// `FindSeenEntities` is due this tick.
    pub lookup_due: bool,
    /// `ComputeSenses` is due this tick (reads the cached list).
    pub sense_due: bool,
}

impl FovCadence {
    /// Computes due flags from body throttle counters and periods.
    #[must_use]
    pub fn from_counters(
        brain_tick: bool,
        lookup_counter: u32,
        sense_counter: u32,
        lookup_period: u32,
        sense_period: u32,
    ) -> Self {
        Self {
            lookup_due: brain_tick && lookup_counter + 1 >= lookup_period.max(1),
            sense_due: brain_tick && sense_counter + 1 >= sense_period.max(1),
        }
    }
}

/// Which FOV categories the brain actually consumes (lazy sensing, spec §3.2).
///
/// The game derives this from which brain inputs have outgoing synapses
/// (`NEATBrain.StartUsedSystems`); [`FovMask::from_brain`] reproduces that rule
/// using the brain's own usage masks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FovMask {
    /// Compute the bibite category (also needed for herding).
    pub bibite: bool,
    /// Compute the plant category.
    pub plant: bool,
    /// Compute the meat category.
    pub meat: bool,
}

impl FovMask {
    /// Mask with every category enabled.
    pub const ALL: Self = Self {
        bibite: true,
        plant: true,
        meat: true,
    };
    /// Mask with no categories enabled (the game's `needToSee == false`).
    pub const NONE: Self = Self {
        bibite: false,
        plant: false,
        meat: false,
    };

    /// Derives the mask from brain wiring exactly like `StartUsedSystems`
    /// (bibits also need vision if the herding output is wired/biased).
    #[must_use]
    pub fn from_brain(brain: &Brain) -> Self {
        Self {
            bibite: brain.needs_bibite_vision(),
            plant: brain.needs_plant_vision(),
            meat: brain.needs_meat_vision(),
        }
    }

    /// `true` if any category will be computed.
    #[inline]
    pub fn any(self) -> bool {
        self.bibite || self.plant || self.meat
    }
}

/// FOV parameters derived from the bibite's genes and growth (spec §3.1/§3.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FovParams {
    /// `gene(ViewRadius) · d1_size`, in world units (`BibiteBody.Set2DSize`).
    pub view_radius: f32,
    /// `gene(ViewAngle)`, in degrees.
    pub view_angle: f32,
    /// `gene(HerdSeparationDistance) · d1_size`, in world units.
    pub herd_separation_distance: f32,
    /// Herd separation weight.
    pub separation_weight: f32,
    /// Herd cohesion weight.
    pub cohesion_weight: f32,
    /// Herd alignment weight.
    pub alignment_weight: f32,
}

impl FovParams {
    /// Game gene defaults (`BibiteEditorSettings`): radius 180, angle 75°,
    /// herd distance 35, separation 1, cohesion 0.75, alignment 1.5.
    pub const GAME_DEFAULTS: Self = Self {
        view_radius: 180.0,
        view_angle: 75.0,
        herd_separation_distance: 35.0,
        separation_weight: 1.0,
        cohesion_weight: 0.75,
        alignment_weight: 1.5,
    };

    /// The game's default parameters (adult size 1).
    #[inline]
    pub const fn game_defaults() -> Self {
        Self::GAME_DEFAULTS
    }

    /// `view_angle / 2` in degrees.
    #[inline]
    pub fn half_angle_deg(&self) -> f32 {
        self.view_angle / 2.0
    }

    /// `true` if the FOV is not degenerate (`gene(ViewAngle)·gene(ViewRadius) > 0`).
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.view_radius > 0.0 && self.view_angle > 0.0
    }
}

/// Result of the game's `TargetIsInView` test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InView {
    /// Effective distance used by the weight formula
    /// (`max(centre_distance − object_radius, 0.001)`, or the cone-edge
    /// intersection distance).
    pub distance: f32,
    /// Angle to the target in degrees (signed: positive = CCW = the bibite's
    /// left when facing `up`). Clamped to `±view_angle/2` on the cone edge.
    pub angle_deg: f32,
    /// Distance-scaled direction vector `up.rotated(angle) · distance` (the
    /// game's `dir2Target`).
    pub direction: Vec2,
}

/// Unity's `Vector2.SignedAngle(from, to)` in degrees: `atan2(cross, dot)`.
#[inline]
pub fn signed_angle_deg(from: Vec2, to: Vec2) -> f32 {
    from.cross(to).atan2(from.dot(to)).to_degrees()
}

/// Unity's `Vector2.Angle(a, b)` in degrees; returns 0 when either vector is
/// (numerically) zero, matching Unity's `kEpsilonNormalSqrt` behaviour.
#[inline]
pub fn angle_deg(a: Vec2, b: Vec2) -> f32 {
    let denominator = (a.length_sq() * b.length_sq()).sqrt();
    if denominator < 1e-15 {
        return 0.0;
    }
    (a.dot(b) / denominator)
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// `FieldOfView.TargetIsInView`: exact cone/circle test including the
/// cone-edge intersection branch for objects whose centre is outside the cone
/// but whose radius crosses it (spec §3.4).
///
/// Returns `None` when the target is not visible. `object_radius` is the
/// target's `sideRadius` (bibites) or analytic pellet radius.
#[must_use]
pub fn target_in_view(
    subject_pos: Vec2,
    subject_up: Vec2,
    params: &FovParams,
    target_pos: Vec2,
    object_radius: f32,
) -> Option<InView> {
    let dir = target_pos - subject_pos;
    let distance = dir.length();
    if distance - object_radius > params.view_radius {
        return None;
    }
    let half_angle = params.half_angle_deg();
    let dot = subject_up.dot(dir);
    let cross = subject_up.cross(dir);
    // Cheap cone classification before the transcendental: most candidates are
    // outside the view cone, and rejecting them with cross/dot comparisons
    // avoids an atan2 + to_degrees per pellet (the dominant sensing cost).
    let inside = if half_angle >= 90.0 {
        // `FieldOfView.cs:563`: `|angle| < viewAngle / 2` for any width. The
        // half-plane shortcut is only exact below 90 degrees.
        signed_angle_deg(subject_up, dir).abs() < half_angle
    } else {
        let tan_half = (half_angle * PI / 180.0).tan();
        dot > 0.0 && cross.abs() <= dot * tan_half
    };
    if inside {
        let angle = signed_angle_deg(subject_up, dir);
        let effective = (distance - object_radius).max(0.001);
        return Some(InView {
            distance: effective,
            angle_deg: angle,
            direction: dir.normalize_or(Vec2::X) * effective,
        });
    }

    // Outside the cone: a target circle can still cross the cone edge. Cheap
    // conservative reject using `distance·sin(|angle|−half) ≈ cross·cos − dot·sin`
    // before paying for the exact atan2/edge solve.
    let half_rad = half_angle * PI / 180.0;
    let sin_offset = (cross.abs() * half_rad.cos() - dot * half_rad.sin()) / distance;
    if sin_offset > object_radius / distance {
        return None;
    }
    let angle = signed_angle_deg(subject_up, dir);

    // Outside the cone: solve the intersection of the view-cone boundary with
    // the target circle (game's line/circle maths).
    let k1 = 2.0 * distance * ((angle.abs() - half_angle) * PI / 180.0).cos();
    let k2 = distance * distance - object_radius * object_radius;
    let discriminant = k1 * k1 - 4.0 * k2;
    if discriminant <= 0.0 {
        return None;
    }
    let effective = (k1 / 2.0 - discriminant.sqrt() / 2.0).max(0.001);
    if effective > params.view_radius {
        return None;
    }
    let clamped_angle = angle.signum() * half_angle;
    Some(InView {
        distance: effective,
        angle_deg: clamped_angle,
        direction: subject_up.rotate(clamped_angle * PI / 180.0) * effective,
    })
}

/// One target's FOV weight:
/// `(1.05 − d/R)² · size_term · (1.05 − |angle|/(A/2))`
/// (spec §3.5; `FieldOfView.ComputeSenses`).
#[inline]
pub fn fov_weight(distance: f32, angle_deg: f32, params: &FovParams, size_term: f32) -> f32 {
    let distance_term = FOV_WEIGHT_BASE - distance / params.view_radius;
    let angle_term = FOV_WEIGHT_BASE - angle_deg.abs() / params.half_angle_deg();
    distance_term * distance_term * size_term * angle_term
}

/// Plant/meat pellet size term (`MatterPellet.sizeFactor = amount/20`).
#[inline]
pub fn pellet_size_term(amount: f32) -> f32 {
    amount.max(0.0) / NORMAL_AMOUNT_AT_SCALE1
}

/// Bibite size term (`maxHealth / 100`).
#[inline]
pub fn bibite_size_term(max_health: f32) -> f32 {
    max_health / BIBITE_SIZE_DIVISOR
}

/// Corpse meat-channel size term (`baseBodyArea/20 · (1 − healthRatio)`).
#[inline]
pub fn corpse_size_term(base_body_area: f32, health_ratio: f32) -> f32 {
    base_body_area / CORPSE_SIZE_DIVISOR * (1.0 - health_ratio)
}

/// Aggregated FOV result for one category.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FovChannel {
    /// `1 − min(|weighted mean dir| / view_radius, 1)`; 0 when nothing seen.
    pub closeness: f32,
    /// `2 · SignedAngle(up, weighted mean dir) / view_angle`.
    pub angle: f32,
    /// Number of objects accepted into this channel.
    pub count: u32,
    /// Largest raw weight in this channel (`maxPlantWeight` etc.).
    pub max_weight: f32,
}

/// Herd metrics computed while iterating visible bibites (spec §3.6).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HerdState {
    /// Whether at least one bibite was seen (`hasHerd`).
    pub has_herd: bool,
    /// Sum of distance-scaled directions to seen bibites.
    pub center_of_herd: Vec2,
    /// Sum of `−dir·(1 − d/herd_distance)` for near neighbours.
    pub separation: Vec2,
    /// Sum of seen bibites' up vectors.
    pub heading: Vec2,
    /// `SignedAngle(up, combined herd vector)`, degrees.
    pub sum_direction: f32,
    /// `cos(Angle(up, separation))` (1 when separation is zero, matching
    /// Unity's zero-vector angle quirk).
    pub separation_projection: f32,
}

/// Complete FOV result for one bibite.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FovResult {
    /// Bibite channel.
    pub bibite: FovChannel,
    /// Plant-pellet channel.
    pub plant: FovChannel,
    /// Meat channel (meat pellets + corpses).
    pub meat: FovChannel,
    /// `[r, g, b]` genes of the closest seen bibite (`targetR/G/B`).
    pub target_color: [f32; 3],
    /// Herd metrics.
    pub herd: HerdState,
}

/// What kind of object a field-of-view target is (`TargetType` of the game's `WeightDisplay`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// A bibite (alive, or dead: a corpse is seen as a bibite and as meat).
    Bibite,
    /// A plant pellet.
    Plant,
    /// A meat pellet or a corpse.
    Meat,
}

/// One object a bibite's field of view sees (`GameObjectWeight`): where it is, how big its transform is and its weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeenTarget {
    /// The object's identity: a corpse is one object in the bibite channel and in the meat channel, with the same key.
    pub key: u64,
    /// World position of the object.
    pub pos: Vec2,
    /// `Transform.localScale.x` of the object: a bibite's `d1Size`, a pellet's `sqrt(amount / 20)`.
    pub scale: f32,
    /// The object's FOV weight (`fov_weight`).
    pub weight: f32,
}

/// The objects a bibite's field of view sees, with their weights, per channel: the lists `FieldOfView` keeps (`bibiteWeights`, `plantWeights`,
/// `meatWeights`, their sums and maxima) that `FieldOfViewDisplay` labels when "Show Targets" is on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FovTargets {
    /// The bibite channel.
    pub bibites: Vec<SeenTarget>,
    /// The plant channel.
    pub plants: Vec<SeenTarget>,
    /// The meat channel (meat pellets and corpses).
    pub meat: Vec<SeenTarget>,
}

impl FovTargets {
    /// The targets of a channel, with the sum and the largest of their weights (`bibiteWeightSum`, `maxBibiteWeight`, ...).
    #[must_use]
    pub fn channel(&self, kind: TargetKind) -> (&[SeenTarget], f32, f32) {
        let list = match kind {
            TargetKind::Bibite => &self.bibites,
            TargetKind::Plant => &self.plants,
            TargetKind::Meat => &self.meat,
        };
        let sum = list.iter().map(|target| target.weight).sum();
        let max = list.iter().fold(0.0_f32, |max, target| max.max(target.weight));
        (list, sum, max)
    }
}

/// Accumulating state for one bibite's FOV computation.
///
/// The caller feeds visible objects in the spatial hash's deterministic order
/// (sorted by `(class, slot, generation)`); the accumulators reproduce the
/// game's float summation order for that deterministic order.
#[derive(Debug)]
pub struct FovSensor {
    subject_pos: Vec2,
    subject_up: Vec2,
    params: FovParams,
    mask: FovMask,
    bibite_weight_sum: f32,
    bibite_dir_sum: Vec2,
    bibite: FovChannel,
    plant_weight_sum: f32,
    plant_dir_sum: Vec2,
    plant: FovChannel,
    meat_weight_sum: f32,
    meat_dir_sum: Vec2,
    meat: FovChannel,
    target_color: [f32; 3],
    closest_distance: f32,
    herd_heading: Vec2,
    herd_center: Vec2,
    herd_separation: Vec2,
}

impl FovSensor {
    /// Starts a FOV computation for a bibite at `subject_pos` facing
    /// `subject_up` (`Vec2::Y` rotated by the body rotation).
    #[must_use]
    pub fn new(subject_pos: Vec2, subject_up: Vec2, params: FovParams, mask: FovMask) -> Self {
        Self {
            subject_pos,
            subject_up,
            params,
            mask,
            bibite_weight_sum: 0.0,
            bibite_dir_sum: Vec2::ZERO,
            bibite: FovChannel::default(),
            plant_weight_sum: 0.0,
            plant_dir_sum: Vec2::ZERO,
            plant: FovChannel::default(),
            meat_weight_sum: 0.0,
            meat_dir_sum: Vec2::ZERO,
            meat: FovChannel::default(),
            target_color: [0.0; 3],
            closest_distance: f32::INFINITY,
            herd_heading: Vec2::ZERO,
            herd_center: Vec2::ZERO,
            herd_separation: Vec2::ZERO,
        }
    }

    /// The FOV mask this sensor was created with.
    #[inline]
    pub fn mask(&self) -> FovMask {
        self.mask
    }

    /// Feeds a plant pellet (spec §3.5 plant channel).
    ///
    /// `amount` is the pellet's matter amount (`sizeFactor = amount/20`), and
    /// `pellet_radius` is the analytic radius used for the visibility test.
    pub fn add_plant(&mut self, pos: Vec2, pellet_radius: f32, amount: f32) {
        if !self.mask.plant {
            return;
        }
        let Some(view) = target_in_view(
            self.subject_pos,
            self.subject_up,
            &self.params,
            pos,
            pellet_radius,
        ) else {
            return;
        };
        let weight = fov_weight(
            view.distance,
            view.angle_deg,
            &self.params,
            pellet_size_term(amount),
        );
        self.plant.count += 1;
        self.plant.max_weight = self.plant.max_weight.max(weight);
        self.plant_dir_sum += view.direction * weight;
        self.plant_weight_sum += weight;
    }

    /// Feeds a meat pellet (spec §3.5 meat channel).
    pub fn add_meat(&mut self, pos: Vec2, pellet_radius: f32, amount: f32) {
        if !self.mask.meat {
            return;
        }
        let Some(view) = target_in_view(
            self.subject_pos,
            self.subject_up,
            &self.params,
            pos,
            pellet_radius,
        ) else {
            return;
        };
        let weight = fov_weight(
            view.distance,
            view.angle_deg,
            &self.params,
            pellet_size_term(amount),
        );
        self.meat.count += 1;
        self.meat.max_weight = self.meat.max_weight.max(weight);
        self.meat_dir_sum += view.direction * weight;
        self.meat_weight_sum += weight;
    }

    /// Feeds a corpse into the meat channel
    /// (`baseBodyArea/20 · (1 − healthRatio)` size term).
    pub fn add_corpse(
        &mut self,
        pos: Vec2,
        side_radius: f32,
        base_body_area: f32,
        health_ratio: f32,
    ) {
        if !self.mask.meat {
            return;
        }
        let Some(view) = target_in_view(
            self.subject_pos,
            self.subject_up,
            &self.params,
            pos,
            side_radius,
        ) else {
            return;
        };
        let weight = fov_weight(
            view.distance,
            view.angle_deg,
            &self.params,
            corpse_size_term(base_body_area, health_ratio),
        );
        self.meat.count += 1;
        self.meat.max_weight = self.meat.max_weight.max(weight);
        self.meat_dir_sum += view.direction * weight;
        self.meat_weight_sum += weight;
    }

    /// Feeds a bibite (alive or dead) into the bibite channel and herd
    /// accumulators (spec §3.5/§3.6). Dead bibites are also fed to
    /// [`Self::add_corpse`] by the caller, exactly like the game's dual loop.
    ///
    /// `up` is the target bibite's forward vector; `color` its `[r,g,b]` genes.
    #[allow(clippy::too_many_arguments)]
    pub fn add_bibite(
        &mut self,
        pos: Vec2,
        side_radius: f32,
        up: Vec2,
        max_health: f32,
        color: [f32; 3],
        is_ready: bool,
    ) {
        if !self.mask.bibite {
            return;
        }
        let Some(view) = target_in_view(
            self.subject_pos,
            self.subject_up,
            &self.params,
            pos,
            side_radius,
        ) else {
            return;
        };
        let weight = fov_weight(
            view.distance,
            view.angle_deg,
            &self.params,
            bibite_size_term(max_health),
        );
        self.bibite.count += 1;
        self.bibite.max_weight = self.bibite.max_weight.max(weight);
        self.bibite_dir_sum += view.direction * weight;
        self.bibite_weight_sum += weight;

        // Herd accumulators (also for dead bibites, like the game).
        self.herd_heading += up;
        self.herd_center += view.direction;
        if view.distance < self.params.herd_separation_distance {
            self.herd_separation += -view.direction.normalize_or(Vec2::X)
                * (1.0 - view.distance / self.params.herd_separation_distance);
        }
        // `FieldOfView.cs:479-488`: `dist2Bibite` is only ever initialised to
        // +inf and never assigned, so `!(dist2Target > dist2Bibite)` is always
        // true: the colour is that of the *last* seen bibite whose gene is
        // ready (not the closest one).
        self.closest_distance = view.distance;
        if is_ready {
            self.target_color = color;
        }
    }

    /// Finishes the FOV computation (`FieldOfView.ComputeSenses` tail).
    #[must_use]
    pub fn finish(mut self) -> FovResult {
        self.bibite = self.finish_channel(self.bibite_weight_sum, self.bibite_dir_sum, self.bibite);
        self.plant = self.finish_channel(self.plant_weight_sum, self.plant_dir_sum, self.plant);
        self.meat = self.finish_channel(self.meat_weight_sum, self.meat_dir_sum, self.meat);

        let herd = if self.bibite.count > 0 {
            let combined = self.herd_separation.normalized() * self.params.separation_weight
                + self.herd_center.normalized() * self.params.cohesion_weight
                + self.herd_heading * self.params.alignment_weight;
            HerdState {
                has_herd: true,
                center_of_herd: self.herd_center,
                separation: self.herd_separation,
                heading: self.herd_heading,
                sum_direction: signed_angle_deg(self.subject_up, combined),
                separation_projection: if self.herd_separation.length_sq() <= 1e-12 {
                    1.0
                } else {
                    self.subject_up
                        .dot(self.herd_separation.normalized())
                        .clamp(-1.0, 1.0)
                },
            }
        } else {
            HerdState::default()
        };

        FovResult {
            bibite: self.bibite,
            plant: self.plant,
            meat: self.meat,
            target_color: self.target_color,
            herd,
        }
    }

    fn finish_channel(
        &self,
        weight_sum: f32,
        dir_sum: Vec2,
        mut channel: FovChannel,
    ) -> FovChannel {
        if weight_sum > 0.0 {
            let mean = dir_sum / weight_sum;
            channel.closeness = 1.0 - (mean.length() / self.params.view_radius).min(1.0);
            channel.angle = 2.0 * signed_angle_deg(self.subject_up, mean) / self.params.view_angle;
        } else {
            channel.closeness = 0.0;
            channel.angle = 0.0;
        }
        channel
    }
}

// ---------------------------------------------------------------------------
// Pheromone sensing
// ---------------------------------------------------------------------------

/// Aggregated pheromone sensing result for one bibite (spec §4.2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PheroSenses {
    /// Raw per-channel sums `Σ strength/distance` (before `tanh`).
    pub sum: [f32; 3],
    /// `tanh(sum / 5)` per channel (the brain inputs).
    pub sense: [f32; 3],
    /// `SignedAngle(up, Σ dir·(strength/d)) / 180` per channel.
    pub angle: [f32; 3],
    /// `SignedAngle(up, Σ spot.heading·(strength/d)) / 180` per channel.
    pub heading: [f32; 3],
}

/// Accumulates pheromone spots for one bibite (`Pherosense.PherosenseAround`).
///
/// The caller queries spots with the bibite's pheromone-sense radius
/// (`gene.PheroSense`, **not** scaled by size) and feeds them here; the red-death
/// pseudo-channel is added via [`Self::add_red_death`] when red death is on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PheroAccumulator {
    subject_pos: Vec2,
    sums: [f32; 3],
    dir_sums: [Vec2; 3],
    heading_sums: [Vec2; 3],
}

impl PheroAccumulator {
    /// Starts accumulation for a bibite at `subject_pos`.
    #[must_use]
    pub fn new(subject_pos: Vec2) -> Self {
        Self {
            subject_pos,
            sums: [0.0; 3],
            dir_sums: [Vec2::ZERO; 3],
            heading_sums: [Vec2::ZERO; 3],
        }
    }

    /// Feeds one spot (`strength` in `u²`, `heading` the spot's unit heading).
    /// Only positive channel strengths contribute; negative channels are ignored
    /// by sensing (spec §4.2/§5.3).
    pub fn add_spot(&mut self, pos: Vec2, heading: Vec2, strength: [f32; 3]) {
        let delta = pos - self.subject_pos;
        let distance = delta.length();
        if distance <= 0.0 || !distance.is_finite() {
            return;
        }
        for (channel, &value) in strength.iter().enumerate() {
            if value > 0.0 {
                let scaled = value / distance;
                self.sums[channel] += scaled;
                self.dir_sums[channel] += delta * scaled;
                self.heading_sums[channel] += heading * scaled;
            }
        }
    }

    /// Adds the red-death pseudo-pheromone when enabled (`Pherosense.cs:195-208`):
    /// with `Δ = safe_radius − |pos|` and `k = 100 / max(1, Δ)`, only when
    /// `phero_radius > Δ`.
    pub fn add_red_death(&mut self, safe_radius: f32, phero_radius: f32) {
        if !safe_radius.is_finite() {
            return;
        }
        let distance_to_edge = safe_radius - self.subject_pos.length();
        if phero_radius > distance_to_edge {
            let k = RED_DEATH_PHERO_STRENGTH / distance_to_edge.max(1.0);
            self.sums[0] += k;
            self.dir_sums[0] += self.subject_pos.normalize_or(Vec2::X) * (distance_to_edge * k);
            self.heading_sums[0] += -self.subject_pos.normalize_or(Vec2::X) * k;
        }
    }

    /// Finishes accumulation (`Pherosense.cs:209-214`): applies `tanh(sum/5)` and
    /// normalises angles by 180°.
    #[must_use]
    pub fn finish(self, subject_up: Vec2) -> PheroSenses {
        let mut result = PheroSenses::default();
        for channel in 0..3 {
            result.sum[channel] = self.sums[channel];
            result.sense[channel] = (self.sums[channel] / SENSE_TANH_DIVISOR).tanh();
            result.angle[channel] = signed_angle_deg(subject_up, self.dir_sums[channel]) / 180.0;
            result.heading[channel] =
                signed_angle_deg(subject_up, self.heading_sums[channel]) / 180.0;
        }
        result
    }
}

/// The pheromone sensing radius for a gene value (spec §4.1): the raw
/// `gene(PheroSense)` value, **not** scaled by body size.
#[inline]
pub fn phero_sense_radius(gene_value: f32) -> f32 {
    gene_value.max(0.0)
}

/// Writes `tanh(sum/5)` directly (used by tests/parity digests).
#[inline]
pub fn phero_sense_value(sum: f32) -> f32 {
    (sum / SENSE_TANH_DIVISOR).tanh()
}

// ---------------------------------------------------------------------------
// World-facing entry points (feed these from spatial-hash queries)
// ---------------------------------------------------------------------------

/// A pellet as seen by [`compute_fov`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FovPellet {
    /// Position.
    pub pos: Vec2,
    /// Analytic radius (`sqrt(amount/π)`).
    pub radius: f32,
    /// Matter amount in `u²` (size factor `amount/20`).
    pub amount: f32,
}

/// A bibite as seen by [`compute_fov`] (dead bibites also feed the meat channel
/// exactly like the game's corpse loop).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FovBibite {
    /// Position.
    pub pos: Vec2,
    /// Body side radius (`4.5 · d1Size`).
    pub side_radius: f32,
    /// Forward vector (`Vec2::Y` rotated by the body rotation).
    pub up: Vec2,
    /// Maximum health (`maxHealth/100` weight term).
    pub max_health: f32,
    /// Body area (`baseBodyArea/20 · (1−healthRatio)` corpse term).
    pub base_body_area: f32,
    /// Health ratio (corpse decay term).
    pub health_ratio: f32,
    /// Whether this is a corpse.
    pub dead: bool,
    /// `[r, g, b]` genes for the closest-bibite colour readout.
    pub color: [f32; 3],
    /// Whether the gene data is ready (`IsReady`).
    pub is_ready: bool,
}

/// Complete FOV entry point: computes one bibite's vision from deterministic
/// iterators of visible entities.
///
/// The world should feed the entities from `SpatialHash` circle/box queries
/// (class masks `Pellet` and `Bibite`, `QueryScratch` cap 128, sorted by
/// `(class, slot, generation)`), which gives a deterministic float summation
/// order. Dead bibites are fed to both the bibite channel and the meat
/// (corpse) channel, matching the game's dual loop.
///
/// The caller checks the lazy [`FovMask`] (and the `ViewAngle·ViewRadius > 0`
/// gene gate) and only calls this when the body's sense throttle says a sense
/// refresh is due.
pub fn compute_fov(
    subject_pos: Vec2,
    subject_up: Vec2,
    params: FovParams,
    mask: FovMask,
    plant_pellets: impl IntoIterator<Item = FovPellet>,
    meat_pellets: impl IntoIterator<Item = FovPellet>,
    bibites: impl IntoIterator<Item = FovBibite>,
) -> FovResult {
    let mut sensor = FovSensor::new(subject_pos, subject_up, params, mask);
    for pellet in plant_pellets {
        sensor.add_plant(pellet.pos, pellet.radius, pellet.amount);
    }
    for pellet in meat_pellets {
        sensor.add_meat(pellet.pos, pellet.radius, pellet.amount);
    }
    for bibite in bibites {
        if bibite.dead {
            sensor.add_corpse(
                bibite.pos,
                bibite.side_radius,
                bibite.base_body_area,
                bibite.health_ratio,
            );
        }
        sensor.add_bibite(
            bibite.pos,
            bibite.side_radius,
            bibite.up,
            bibite.max_health,
            bibite.color,
            bibite.is_ready,
        );
    }
    sensor.finish()
}

/// A pheromone spot as seen by [`compute_phero`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PheroSpot {
    /// Spot position.
    pub pos: Vec2,
    /// Spot heading (`PheromoneSpot.heading`).
    pub heading: Vec2,
    /// Per-channel strengths `[r, g, b]` (`u²`).
    pub strength: [f32; 3],
}

/// Complete pheromone-sensing entry point for one bibite.
///
/// The caller queries spots within `phero_radius` (`gene.PheroSense`, not
/// scaled by size — spec §4.1) and passes `red_death` as
/// `Some((safe_radius, phero_radius))` when the event is enabled.
pub fn compute_phero(
    subject_pos: Vec2,
    subject_up: Vec2,
    phero_radius: f32,
    spots: impl IntoIterator<Item = PheroSpot>,
    red_death: Option<(f32, f32)>,
) -> PheroSenses {
    let mut accumulator = PheroAccumulator::new(subject_pos);
    let radius_sq = phero_radius.max(0.0) * phero_radius.max(0.0);
    for spot in spots {
        // Defensive re-filter: the caller's spatial query already uses this
        // radius, but the sense range is part of the contract.
        if spot.pos.distance_sq(subject_pos) <= radius_sq {
            accumulator.add_spot(spot.pos, spot.heading, spot.strength);
        }
    }
    if let Some((safe_radius, radius)) = red_death {
        accumulator.add_red_death(safe_radius, radius);
    }
    accumulator.finish(subject_up)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::{ALL_INPUTS, Input};

    fn almost(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4
    }

    fn almost_vec(a: Vec2, b: Vec2) -> bool {
        almost(a.x, b.x) && almost(a.y, b.y)
    }

    fn default_params() -> FovParams {
        FovParams::game_defaults()
    }

    // ----- SenseOutputs -----

    #[test]
    fn default_is_all_zero() {
        let senses = SenseOutputs::default();
        assert_eq!(senses, SenseOutputs::zeroed());
        assert!(senses.values().iter().all(|value| *value == 0.0));
    }

    #[test]
    fn trait_getters_round_trip() {
        let senses = SenseOutputs::from_values([
            0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7,
            1.8, 1.9, 2.0, 2.1,
        ]);
        assert_eq!(senses.values()[0], 0.1);
        assert_eq!(senses.values()[20], 2.1);
        use brain::WorldSenseSource as _;
        assert_eq!(senses.bibite_closeness(), 0.1);
        assert_eq!(senses.n_bibites(), 0.3);
        assert_eq!(senses.blue_bibite(), 1.2);
        assert_eq!(senses.phero_sense_1(), 1.3);
        assert_eq!(senses.phero_3_heading(), 2.1);
    }

    #[test]
    fn write_into_fills_only_world_inputs() {
        let senses = SenseOutputs::from_values([
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
            17.0, 18.0, 19.0, 20.0, 21.0,
        ]);
        let mut inputs = BrainInputs::default();
        senses.write_into(&mut inputs);

        let mut world_inputs = Vec::new();
        for input in ALL_INPUTS {
            let index = input as usize;
            let is_world = (9..=20).contains(&index) || (24..=32).contains(&index);
            if is_world {
                world_inputs.push(input);
                assert_ne!(inputs.get(input), 0.0, "{input:?} should be written");
            } else {
                assert_eq!(inputs.get(input), 0.0, "{input:?} is body-owned");
            }
        }
        assert_eq!(world_inputs.len(), WORLD_INPUT_COUNT);
        assert_eq!(inputs.get(Input::BibiteCloseness), 1.0);
        assert_eq!(inputs.get(Input::BlueBibite), 12.0);
        assert_eq!(inputs.get(Input::PheroSense1), 13.0);
        assert_eq!(inputs.get(Input::Phero3Heading), 21.0);
    }

    // ----- angle helpers -----

    #[test]
    fn signed_angle_matches_unity_convention() {
        // Unity: SignedAngle(up, right) == -90, (up, left) == +90.
        assert!(almost(signed_angle_deg(Vec2::Y, Vec2::X), -90.0));
        assert!(almost(signed_angle_deg(Vec2::Y, -Vec2::X), 90.0));
        assert!(almost(signed_angle_deg(Vec2::X, Vec2::Y), 90.0));
        // Zero vectors: atan2(0, 0) == 0.
        assert_eq!(signed_angle_deg(Vec2::Y, Vec2::ZERO), 0.0);
        // Unity's Angle() returns 0 when a vector is zero.
        assert_eq!(angle_deg(Vec2::Y, Vec2::ZERO), 0.0);
        assert!(almost(angle_deg(Vec2::X, Vec2::Y), 90.0));
    }

    // ----- TargetIsInView -----

    #[test]
    fn target_on_axis_is_visible() {
        let params = default_params();
        let view = target_in_view(Vec2::ZERO, Vec2::Y, &params, Vec2::new(0.0, 10.0), 2.0)
            .expect("visible");
        assert!(almost(view.distance, 8.0)); // 10 - radius 2
        assert_eq!(view.angle_deg, 0.0);
        assert!(almost_vec(view.direction, Vec2::new(0.0, 8.0)));
    }

    #[test]
    fn target_outside_radius_is_invisible() {
        let params = default_params();
        assert!(
            target_in_view(
                Vec2::ZERO,
                Vec2::Y,
                &params,
                Vec2::new(0.0, 180.0 + 1.0 + 2.0),
                2.0
            )
            .is_none()
        );
        // But surface exactly at the radius is visible.
        assert!(
            target_in_view(
                Vec2::ZERO,
                Vec2::Y,
                &params,
                Vec2::new(0.0, 180.0 + 2.0),
                2.0
            )
            .is_some()
        );
    }

    #[test]
    fn target_off_axis_inside_cone_uses_true_angle() {
        let params = default_params(); // half angle 37.5°
        let pos = Vec2::new(10.0, 10.0); // 45° -> outside the cone
        assert!(target_in_view(Vec2::ZERO, Vec2::Y, &params, pos, 0.0).is_none());
        // 30° off axis is inside; positive angle is CCW = to the left, so a
        // target on the right has a negative angle.
        let distance = 50.0;
        let angle = 30.0f32.to_radians();
        let pos = Vec2::new(distance * angle.sin(), distance * angle.cos());
        let view = target_in_view(Vec2::ZERO, Vec2::Y, &params, pos, 0.0).expect("visible");
        assert!(almost(view.angle_deg, -30.0));
        assert!(almost(view.distance, distance));
    }

    #[test]
    fn cone_edge_intersection_makes_big_targets_visible() {
        // Half angle 45° (view angle 90). Target centre exactly on the cone edge
        // at distance sqrt(2)*10: its radius crosses into the cone.
        let params = FovParams {
            view_angle: 90.0,
            view_radius: 100.0,
            ..FovParams::game_defaults()
        };
        let target = Vec2::new(10.0, 10.0); // angle -45° (right side)
        let view =
            target_in_view(Vec2::ZERO, Vec2::Y, &params, target, 2.0).expect("edge target visible");
        assert!(almost(view.angle_deg, -45.0)); // clamped to the cone edge
        // k1 = 2*d*cos(0) = 2*14.142; k2 = d² - r² = 200 - 4 = 196;
        // disc = 800 - 784 = 16; dist = 14.142 - 2 = 12.142.
        assert!(almost(view.distance, 2.0f32.sqrt() * 10.0 - 2.0));
    }

    #[test]
    fn target_behind_is_invisible() {
        let params = default_params();
        assert!(target_in_view(Vec2::ZERO, Vec2::Y, &params, Vec2::new(0.0, -10.0), 0.0).is_none());
    }

    #[test]
    fn target_check_uses_surface_distance() {
        let params = default_params();
        // Centre outside the radius but surface inside.
        let view = target_in_view(Vec2::ZERO, Vec2::Y, &params, Vec2::new(0.0, 181.0), 2.0)
            .expect("visible");
        assert!(almost(view.distance, 179.0));
    }

    // ----- weights / accumulators -----

    #[test]
    fn weight_formula_hand_computed() {
        let params = default_params();
        // On axis, distance 0, size term 1: (1.05)^2 * 1 * 1.05.
        let weight = fov_weight(0.0, 0.0, &params, 1.0);
        assert!(almost(weight, 1.05 * 1.05 * 1.05));
        // Distance 90 (half radius), angle 0, size 2:
        // (1.05 - 0.5)^2 * 2 * 1.05.
        let weight = fov_weight(90.0, 0.0, &params, 2.0);
        assert!(almost(weight, 0.55 * 0.55 * 2.0 * 1.05));
        // On the cone edge, angle 37.5 -> angle term is 1.05 - 1 = 0.05.
        let weight = fov_weight(0.0, 37.5, &params, 1.0);
        assert!(almost(weight, 1.05 * 1.05 * 0.05));
    }

    #[test]
    fn single_plant_ahead_closeness_and_angle() {
        let params = default_params();
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, FovMask::ALL);
        sensor.add_plant(Vec2::new(0.0, 10.0), 0.0, 20.0); // size factor 1
        let result = sensor.finish();
        assert_eq!(result.plant.count, 1);
        // mean dir = up*10 -> closeness = 1 - 10/180; angle 0.
        assert!(almost(result.plant.closeness, 1.0 - 10.0 / 180.0));
        assert_eq!(result.plant.angle, 0.0);
        assert!(almost(
            result.plant.max_weight,
            fov_weight(10.0, 0.0, &params, 1.0)
        ));
        assert_eq!(result.meat.count, 0);
    }

    #[test]
    fn two_symmetric_plants_cancel_angle() {
        let params = default_params();
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, FovMask::ALL);
        sensor.add_plant(Vec2::new(-5.0, 10.0), 0.0, 20.0);
        sensor.add_plant(Vec2::new(5.0, 10.0), 0.0, 20.0);
        let result = sensor.finish();
        assert_eq!(result.plant.count, 2);
        assert!(almost(result.plant.angle, 0.0));
        // mean dir = up*10-ish (y weighted), closeness > 0.
        assert!(result.plant.closeness > 0.8);
    }

    #[test]
    fn masks_disable_categories() {
        let params = default_params();
        let mask = FovMask {
            bibite: false,
            plant: true,
            meat: false,
        };
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, mask);
        sensor.add_plant(Vec2::new(0.0, 10.0), 0.0, 20.0);
        sensor.add_meat(Vec2::new(0.0, 10.0), 0.0, 20.0);
        sensor.add_bibite(
            Vec2::new(0.0, 10.0),
            1.0,
            Vec2::Y,
            100.0,
            [1.0, 0.0, 0.0],
            true,
        );
        let result = sensor.finish();
        assert_eq!(result.plant.count, 1);
        assert_eq!(result.meat.count, 0);
        assert_eq!(result.bibite.count, 0);
        assert_eq!(result.target_color, [0.0; 3]);
        assert!(!result.herd.has_herd);
    }

    #[test]
    fn closest_bibite_color_ties_favour_later_and_ready() {
        let params = default_params();
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, FovMask::ALL);
        sensor.add_bibite(
            Vec2::new(0.0, 20.0),
            1.0,
            Vec2::Y,
            100.0,
            [1.0, 0.0, 0.0],
            true,
        );
        // Same distance: later wins (<=), and its colour is used.
        sensor.add_bibite(
            Vec2::new(0.0, 20.0),
            1.0,
            Vec2::Y,
            100.0,
            [0.0, 1.0, 0.0],
            true,
        );
        // Closer but not ready: colour must stay from the previous ready one.
        sensor.add_bibite(
            Vec2::new(0.0, 10.0),
            1.0,
            Vec2::Y,
            100.0,
            [0.0, 0.0, 1.0],
            false,
        );
        let result = sensor.finish();
        assert_eq!(result.target_color, [0.0, 1.0, 0.0]);
        assert_eq!(result.bibite.count, 3);
    }

    #[test]
    fn herd_metrics_are_accumulated() {
        let params = default_params();
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, FovMask::ALL);
        // One bibite to the left, one to the right, both facing up.
        sensor.add_bibite(Vec2::new(-5.0, 10.0), 0.0, Vec2::Y, 100.0, [0.0; 3], true);
        sensor.add_bibite(Vec2::new(5.0, 10.0), 0.0, Vec2::Y, 100.0, [0.0; 3], true);
        let result = sensor.finish();
        assert!(result.herd.has_herd);
        assert!(almost_vec(result.herd.center_of_herd, Vec2::new(0.0, 20.0)));
        // Both are within the 35 u separation distance -> non-zero separation.
        assert!(result.herd.separation.length() > 0.0);
        assert!(almost(result.herd.heading.y, 2.0));
        assert!(!result.herd.sum_direction.is_nan());
        assert!((-1.0..=1.0).contains(&result.herd.separation_projection));
    }

    #[test]
    fn zero_separation_projection_is_one_like_unity() {
        let params = default_params();
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, FovMask::ALL);
        // Far bibite: outside herd separation distance -> zero separation vector.
        sensor.add_bibite(Vec2::new(0.0, 100.0), 1.0, Vec2::Y, 100.0, [0.0; 3], true);
        let result = sensor.finish();
        assert_eq!(result.herd.separation, Vec2::ZERO);
        // Unity's Angle(up, zero) == 0 -> cos(0) == 1.
        assert!(almost(result.herd.separation_projection, 1.0));
        assert!(almost(result.herd.sum_direction, 0.0));
    }

    #[test]
    fn apply_fov_writes_vision_inputs_only() {
        let params = default_params();
        let mut sensor = FovSensor::new(Vec2::ZERO, Vec2::Y, params, FovMask::ALL);
        sensor.add_plant(Vec2::new(0.0, 10.0), 0.0, 20.0);
        sensor.add_corpse(Vec2::new(0.0, 20.0), 1.0, 80.0, 0.5);
        let fov = sensor.finish();
        let mut outputs = SenseOutputs::zeroed();
        outputs.apply_fov(&fov);
        assert!(outputs.plant_closeness > 0.0);
        assert!(almost(outputs.n_plants, 0.25));
        assert!(almost(outputs.n_meats, 0.25)); // one corpse
        assert_eq!(outputs.phero_sense_1, 0.0);
    }

    // ----- pheromones -----

    #[test]
    fn phero_sum_is_inverse_distance() {
        let mut phero = PheroAccumulator::new(Vec2::ZERO);
        phero.add_spot(Vec2::new(0.0, 10.0), Vec2::Y, [10.0, 0.0, 0.0]);
        let result = phero.finish(Vec2::Y);
        assert!(almost(result.sum[0], 1.0)); // 10 / 10
        assert!(almost(result.sense[0], (0.2f32).tanh()));
        assert!(almost(result.angle[0], 0.0));
        assert_eq!(result.sum[1], 0.0);
        assert_eq!(result.sum[2], 0.0);
    }

    #[test]
    fn phero_angles_and_headings_normalise_by_180() {
        let mut phero = PheroAccumulator::new(Vec2::ZERO);
        // Spot to the right (+X) with heading +X.
        phero.add_spot(Vec2::new(20.0, 0.0), Vec2::X, [5.0, 5.0, 0.0]);
        let result = phero.finish(Vec2::Y);
        // SignedAngle(up, +X) = -90 -> -0.5.
        assert!(almost(result.angle[0], -0.5));
        assert!(almost(result.angle[1], -0.5));
        assert!(almost(result.heading[0], -0.5));
        assert!(almost(result.heading[1], -0.5));
    }

    #[test]
    fn phero_negative_and_zero_strengths_are_ignored() {
        let mut phero = PheroAccumulator::new(Vec2::ZERO);
        phero.add_spot(Vec2::new(0.0, 10.0), Vec2::Y, [10.0, -5.0, 0.0]);
        let result = phero.finish(Vec2::Y);
        assert!(almost(result.sum[0], 1.0));
        assert_eq!(result.sum[1], 0.0);
        assert_eq!(result.sum[2], 0.0);
    }

    #[test]
    fn phero_skips_zero_distance_spot() {
        let mut phero = PheroAccumulator::new(Vec2::ZERO);
        phero.add_spot(Vec2::ZERO, Vec2::Y, [10.0, 10.0, 10.0]);
        let result = phero.finish(Vec2::Y);
        assert_eq!(result.sum, [0.0; 3]);
    }

    #[test]
    fn phero_red_death_pseudo_channel() {
        // Bibite at (50, 0), safe radius 100 -> Δ = 50, k = 100/max(1,50) = 2.
        let mut phero = PheroAccumulator::new(Vec2::new(50.0, 0.0));
        phero.add_red_death(100.0, 150.0); // phero radius 150 > Δ 50
        let result = phero.finish(Vec2::Y);
        assert!(almost(result.sum[0], 2.0));
        // dir = +X * (Δ*k = 100) -> SignedAngle(up, +X)/180 = -0.5.
        assert!(almost(result.angle[0], -0.5));
        // heading = -X * 2 -> SignedAngle(up, -X)/180 = +0.5.
        assert!(almost(result.heading[0], 0.5));
        assert_eq!(result.sum[1], 0.0);
        assert_eq!(result.sum[2], 0.0);
    }

    #[test]
    fn phero_red_death_not_added_when_out_of_smell_range() {
        let mut phero = PheroAccumulator::new(Vec2::new(50.0, 0.0));
        phero.add_red_death(100.0, 10.0); // 10 < Δ 50 -> no term
        assert_eq!(phero.finish(Vec2::Y).sum, [0.0; 3]);
        // Also no term when red death is disabled (safe radius +inf).
        let mut phero = PheroAccumulator::new(Vec2::ZERO);
        phero.add_red_death(f32::INFINITY, 150.0);
        assert_eq!(phero.finish(Vec2::Y).sum, [0.0; 3]);
    }

    #[test]
    fn apply_phero_writes_phero_inputs_only() {
        let mut phero = PheroAccumulator::new(Vec2::ZERO);
        phero.add_spot(Vec2::new(0.0, 10.0), Vec2::Y, [10.0, 0.0, 0.0]);
        let result = phero.finish(Vec2::Y);
        let mut outputs = SenseOutputs::zeroed();
        outputs.apply_phero(&result);
        assert!(almost(outputs.phero_sense_1, (0.2f32).tanh()));
        assert_eq!(outputs.phero_1_angle, 0.0);
        assert_eq!(outputs.plant_closeness, 0.0);
    }

    #[test]
    fn phero_sense_value_helpers() {
        assert!(almost(phero_sense_value(5.0), (1.0f32).tanh()));
        assert_eq!(phero_sense_radius(150.0), 150.0);
        assert_eq!(phero_sense_radius(-3.0), 0.0);
    }

    #[test]
    fn fov_mask_from_brain_uses_brain_usage_masks() {
        // The starter brain has a few synapses; the mask must match the brain's
        // own category helpers exactly.
        let brain = Brain::starter();
        let mask = FovMask::from_brain(&brain);
        assert_eq!(mask.bibite, brain.needs_bibite_vision());
        assert_eq!(mask.plant, brain.needs_plant_vision());
        assert_eq!(mask.meat, brain.needs_meat_vision());
        assert_eq!(mask.any(), mask.bibite || mask.plant || mask.meat);
    }

    #[test]
    fn compute_fov_entry_point() {
        let result = compute_fov(
            Vec2::ZERO,
            Vec2::Y,
            FovParams::game_defaults(),
            FovMask::ALL,
            [FovPellet {
                pos: Vec2::new(0.0, 10.0),
                radius: 0.0,
                amount: 20.0,
            }],
            [],
            [FovBibite {
                pos: Vec2::new(5.0, 10.0),
                side_radius: 0.0,
                up: Vec2::Y,
                max_health: 100.0,
                base_body_area: 80.0,
                health_ratio: 1.0,
                dead: false,
                color: [1.0, 0.0, 0.0],
                is_ready: true,
            }],
        );
        assert_eq!(result.plant.count, 1);
        assert_eq!(result.bibite.count, 1);
        assert_eq!(result.meat.count, 0);
        assert_eq!(result.target_color, [1.0, 0.0, 0.0]);
        assert!(result.herd.has_herd);
    }

    #[test]
    fn compute_fov_dead_bibite_feeds_meat_channel_too() {
        let result = compute_fov(
            Vec2::ZERO,
            Vec2::Y,
            FovParams::game_defaults(),
            FovMask::ALL,
            [],
            [],
            [FovBibite {
                pos: Vec2::new(0.0, 10.0),
                side_radius: 0.0,
                up: Vec2::Y,
                max_health: 100.0,
                base_body_area: 80.0,
                health_ratio: 0.5,
                dead: true,
                color: [0.0; 3],
                is_ready: true,
            }],
        );
        assert_eq!(result.bibite.count, 1, "dead bodies still count as bibites");
        assert_eq!(result.meat.count, 1, "and as corpses in the meat channel");
    }

    #[test]
    fn compute_phero_entry_point() {
        let result = compute_phero(
            Vec2::ZERO,
            Vec2::Y,
            150.0,
            [PheroSpot {
                pos: Vec2::new(0.0, 10.0),
                heading: Vec2::Y,
                strength: [10.0, 0.0, 0.0],
            }],
            None,
        );
        assert!(almost(result.sum[0], 1.0));
        assert!(almost(result.sense[0], (0.2f32).tanh()));
    }

    #[test]
    fn compute_phero_entry_point_with_red_death() {
        let result = compute_phero(
            Vec2::new(50.0, 0.0),
            Vec2::Y,
            150.0,
            [],
            Some((100.0, 150.0)),
        );
        assert!(almost(result.sum[0], 2.0));
        assert!(almost(result.angle[0], -0.5));
        assert!(almost(result.heading[0], 0.5));
    }
}
