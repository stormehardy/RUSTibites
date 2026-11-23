//! Pure body math for a bibite: geometry, mass/inertia, energy/health accounting,
//! growth, ageing, damage and death bookkeeping.
//!
//! This module deliberately has **no dependency on the genome, brain, stores or
//! the settings façade**. Callers pass primitive parameter structs
//! ([`crate::body::BodySettings`], [`crate::body::BodyParams`], [`crate::body::GrowthGenes`]). B2 wires these to
//! `genome.rs`/`settings.rs`; the formulas here are exactly the ones in
//! `docs/spec/02-body-organs-physics.md` and are unit-tested against its
//! worked starter-bibite numbers.
//!
//! Unity reference anchors (from `Here/`):
//! * `BibiteBody.cs` — the hub this mirrors.
//! * `BibiteGenes.cs` — gene-derived density/area formulas.
//! * `BibiteGrowth.cs` — growth ODE, maturity and growth energy cost.
//!
//! All values are `f32`; no allocation, no randomness, no I/O.

use crate::arena::Handle;
use crate::brain::{Brain, BrainInputs, BrainOutputs, InternalSenseSource, N_OUTPUTS, Output};
use crate::genome::{Gene, Genome, GenomeMeta, MutationFactor};
pub use crate::ids::EggId;
use crate::ids::{BibiteId, PelletId, ZoneId};
use crate::math::{EPS, PI, Vec2, clamp, clamp01, inv_lerp};
use crate::organs::{
    self, ArmorOrgan, BibiteSwallow, BiteDamageInput, Egg, EggLayer, EggTick, FatOrgan,
    InternalClock, LAYING_THRESHOLD, MOUTH_DEAD_ZONE, MaterialParams, MouthDerived, MouthSettings,
    MouthState, MovementInput, PHEROMONE_EMIT_PERIOD, PelletSwallow, PheromoneOrgan,
    PropulsionDerived, PropulsionSettings, PropulsionState, STOMACH_SLOTS, Stomach, StomachContent,
    StomachMaterial, bibite_bite_input, bite_chunk_area, bite_knockback_impulse,
    blend_turn_towards, compute_movement, egg_energy_investment, egg_required_energy,
    mouth_derived, movement_penalty, out_of_bound_dist_sq, pellet_bite_input, propulsion_derived,
    resolve_bite_damage, shade_avoidance_steering, try_swallow_bibite,
    try_swallow_pellet, void_avoidance_weight, world_wrap_position,
};
use crate::pellet::{MaterialKind, PelletPose, PelletStore};
use crate::pellet_sleep::{PelletSleep, STAMP_LEN};
use crate::pheromone::{PheromoneStore, bibite_emission_sum, bibite_emission_triggers};
use crate::physics as phys;
use rayon::prelude::*;

/// Physics ticks a mouth target survives without a fresh sensor overlap
/// (matches Unity's persistent OnTriggerEnter/Exit list across one missed
/// narrowphase tick).
const MOUTH_CONTACT_GRACE_DEFAULT: u8 = 0;

/// EXPERIMENT: `RUSTIBITES_GRACE=<ticks>` overrides the mouth contact grace.
fn mouth_contact_grace_ticks() -> u8 {
    static GRACE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *GRACE.get_or_init(|| {
        std::env::var("RUSTIBITES_GRACE")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(MOUTH_CONTACT_GRACE_DEFAULT)
    })
}
use crate::senses::SenseOutputs;
use crate::settings::{GrowthAtSpawn, SimSettings};

// ---------------------------------------------------------------------------
// Structural constants (spec 02 §1.3, §1.7, §3)
// ---------------------------------------------------------------------------

/// `baseBodyArea = 80 * d2Size` (`BibiteGenes.cs:87,133`; `BibiteBody.cs:298`).
pub const AREA_PER_D2: f32 = 80.0;
/// `bodyLength = 12.5 * d1Size` (`BibiteGenes.cs:89`; `BibiteBody.cs:296`).
pub const LENGTH_PER_D1: f32 = 12.5;
/// `NormalBodyPointAtSize1` — declared in `BibiteBody.cs:69` but unused in the game.
pub const BODY_POINT_AT_SIZE_1: f32 = 100.0;
/// Fat inflation fraction at maximum overflow: `maxInflateAmount = area * 0.35`
/// (`BibiteFatOrgan.cs:52,85`).
pub const FAT_INFLATE_FRACTION: f32 = 0.35;
/// Capsule collider offset (local units) at `inflateFactor = 0` (`BibiteBody.cs:733`).
pub const CAPSULE_OFFSET_NOT_INFLATED: f32 = -4.7;
/// Capsule collider offset (local units) at `inflateFactor = 1` (`BibiteBody.cs:733`).
pub const CAPSULE_OFFSET_FULLY_INFLATED: f32 = -5.5;
/// Capsule collider width (local units) at `inflateFactor = 0` (`BibiteBody.cs:734`).
pub const CAPSULE_WIDTH_NOT_INFLATED: f32 = 5.0;
/// Capsule collider width (local units) at `inflateFactor = 1` (`BibiteBody.cs:734`).
pub const CAPSULE_WIDTH_FULLY_INFLATED: f32 = 8.5;
/// Capsule collider height (local units) at `inflateFactor = 0` (`BibiteBody.cs:734`).
pub const CAPSULE_HEIGHT_NOT_INFLATED: f32 = 7.0;
/// Capsule collider height (local units) at `inflateFactor = 1` (`BibiteBody.cs:734`).
pub const CAPSULE_HEIGHT_FULLY_INFLATED: f32 = 10.5;

/// Multiplicative decay base of `attackedDmg` per second (`BibiteBody.cs:624`).
pub const ATTACKED_DMG_DECAY_BASE: f32 = 0.707_16;
/// Linear decay of `attackedDmg` per second (`BibiteBody.cs:624`).
pub const ATTACKED_DMG_DECAY_LINEAR: f32 = 0.003;
/// Ongoing bites apply `dt / (5 * bitePeriod)` of the base damage
/// (`BibiteMouth.cs:720-723`).
pub const ONGOING_BITE_PERIODS: f32 = 5.0;

// ---------------------------------------------------------------------------
// Tunable settings subset needed by body math (spec 02 §12.1)
// ---------------------------------------------------------------------------

/// Tunable settings used by the pure body math. Defaults are the game's
/// scenario defaults. B2 replaces this with the fields from `settings.rs`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodySettings {
    /// `healthPerArea` — max HP per u² of base body area. Default `1.25`.
    pub health_per_area: f32,
    /// `storableEnergyPerArea` — active energy per u². Default `1.5`.
    pub storable_energy_per_area: f32,
    /// `bibiteMassDensity` — grams per u². Default `0.025`.
    pub bibite_mass_density: f32,
    /// `baseBodyGrowthCost` — energy per u² of body built. Default `2.0`.
    pub base_body_growth_cost: f32,
    /// `viewAngleAddedCost` — extra body energy density per degree of FOV. Default `0.003`.
    pub view_angle_added_cost: f32,
    /// `viewRadiusAddedCost` — extra body energy density per unit view radius. Default `0.002`.
    pub view_radius_added_cost: f32,
    /// `MatterMaterialManager.Armor.EnergyDensity`, used by `BodyEnergyDensity`.
    /// Default `1.5` E/u².
    pub armor_energy_density: f32,
    /// `healthEnergyFactor` — blood energy density relative to body density. Default `0.5`.
    pub health_energy_factor: f32,
    /// `baseMetabolismCost` — E/(u²·s). Default `0.001`.
    pub base_metabolism_cost: f32,
    /// `energyUsageEfficiency` — base of `efficiency^SpeedRatio`. Default `0.8`.
    pub energy_usage_efficiency: f32,
    /// `healRate` — fraction of max HP per second at size 1. Default `0.001`.
    pub heal_rate: f32,
    /// `healPowerFactor` — size exponent of healing. Default `1.0`.
    pub heal_power_factor: f32,
    /// `healEfficiency` — energy-to-health multiplier divisor. Default `1.0`.
    pub heal_efficiency: f32,
    /// `ageingThreshold` — hours until ageing starts at SpeedRatio 1. Default `1.0`.
    pub base_aging_threshold: f32,
    /// `ageStrengthPenalties` — strength loss per ageing factor unit. Default `0.25`.
    pub age_strength_penalties: f32,
    /// `ageMetabolismPenalties` — metabolism increase per ageing factor unit. Default `0.5`.
    pub age_metabolism_penalties: f32,
    /// `corpseDecayFactor` — metabolism multiplier of corpse decay. Default `1.0`.
    pub corpse_decay_factor: f32,
    /// `dragCoefficient` — drag constant. Default `5.0`.
    pub drag_coefficient: f32,
    /// `collisionDamageConstant` — damage per u/s above the threshold. Default `0.1`.
    pub collision_damage_constant: f32,
    /// `collisionDamageThreshold` — minimum collision speed to hurt. Default `50.0`.
    pub collision_damage_threshold: f32,
    /// `bitingDamageFactor` — damage multiplier of a bite. Default `5.0`.
    pub biting_damage_factor: f32,
    /// Energy-material energy density (E per energy unit).
    ///
    /// PARITY-CAPTURED(BQ-1): `MatterMaterialManager.Energy.EnergyDensity = 1.0`
    /// (`NormalEnergyAtScale1` = 20 with `NormalAmountAtScale1` = 20).
    pub energy_material_density: f32,
    /// Health-material energy density (E per HP).
    ///
    /// PARITY-CAPTURED(BQ-1): `MatterMaterialManager.Health.EnergyDensity = 0.25`
    /// (`NormalEnergyAtScale1` = 5 with `NormalAmountAtScale1` = 20). Used for
    /// starvation damage (`UseEnergy` deficit -> HP, 4 HP per energy unit) and
    /// for the corpse energy offset/decay.
    pub health_material_density: f32,
}

impl Default for BodySettings {
    fn default() -> Self {
        Self {
            health_per_area: 1.25,
            storable_energy_per_area: 1.5,
            bibite_mass_density: 0.025,
            base_body_growth_cost: 2.0,
            view_angle_added_cost: 0.003,
            view_radius_added_cost: 0.002,
            armor_energy_density: 1.5,
            health_energy_factor: 0.5,
            base_metabolism_cost: 0.001,
            energy_usage_efficiency: 0.8,
            heal_rate: 0.001,
            heal_power_factor: 1.0,
            heal_efficiency: 1.0,
            base_aging_threshold: 1.0,
            age_strength_penalties: 0.25,
            age_metabolism_penalties: 0.5,
            corpse_decay_factor: 1.0,
            drag_coefficient: 5.0,
            collision_damage_constant: 0.1,
            collision_damage_threshold: 50.0,
            biting_damage_factor: 5.0,
            energy_material_density: 1.0,
            health_material_density: 0.25,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-bibite gene-derived parameters
// ---------------------------------------------------------------------------

/// Per-bibite gene-derived constants used by body math.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyParams {
    /// `SpeedRatio` gene (`metabolicRate` in the C#).
    pub metabolic_rate: f32,
    /// `energyUsageEfficiency ^ SpeedRatio` (`BibiteGenes.cs:137`).
    pub metabolic_efficiency: f32,
    /// `gene.BodyEnergyDensity` (E/u²) (`BibiteGenes.cs:121`).
    pub base_energy_density: f32,
    /// `BodyEnergyDensity * healthEnergyFactor` (E per HP) (`BibiteGenes.cs:123`).
    pub health_energy_ratio: f32,
    /// `BodyEnergyDensity * (1 + healthEnergyFactor)` (E/u²) (`BibiteGenes.cs:119`).
    pub sum_body_health_energy_ratio: f32,
    /// Health-material energy density used to convert energy debt to HP
    /// (0.25 E/HP captured from the game; 1 energy unit of debt = 4 HP).
    pub health_material_density: f32,
}

impl BodyParams {
    /// Builds the parameters from gene-derived density and `SpeedRatio`.
    ///
    /// `base_energy_density` is [`body_energy_density`]'s result (the genome
    /// module computes it in B2).
    pub fn new(metabolic_rate: f32, base_energy_density: f32, settings: &BodySettings) -> Self {
        let health_energy_ratio = base_energy_density * settings.health_energy_factor;
        Self {
            metabolic_rate,
            metabolic_efficiency: settings.energy_usage_efficiency.powf(metabolic_rate),
            base_energy_density,
            health_energy_ratio,
            sum_body_health_energy_ratio: base_energy_density
                * (1.0 + settings.health_energy_factor),
            health_material_density: settings.health_material_density,
        }
    }

    /// Convenience constructor from the genes that feed [`body_energy_density`].
    pub fn from_genes(
        metabolic_rate: f32,
        view_angle: f32,
        view_radius: f32,
        armor_area_portion: f32,
        settings: &BodySettings,
    ) -> Self {
        Self::new(
            metabolic_rate,
            body_energy_density(view_angle, view_radius, armor_area_portion, settings),
            settings,
        )
    }
}

/// `BodyEnergyDensity = baseBodyGrowthCost + ViewAngle*cost + ViewRadius*cost
/// + armorPortion*Armor.EnergyDensity` (`BibiteGenes.cs:121`).
pub fn body_energy_density(
    view_angle: f32,
    view_radius: f32,
    armor_area_portion: f32,
    settings: &BodySettings,
) -> f32 {
    settings.base_body_growth_cost
        + view_angle * settings.view_angle_added_cost
        + view_radius * settings.view_radius_added_cost
        + armor_area_portion * settings.armor_energy_density
}

// ---------------------------------------------------------------------------
// Matter pools (Energy / Health)
// ---------------------------------------------------------------------------

/// A game `Matter` pool: an `amount` of a material with a `max_amount` and an
/// energy density (E per amount unit). `Matter.cs:8-57`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MatterPool {
    /// Current amount (E for energy, HP for health).
    pub amount: f32,
    /// Maximum amount; not clamped when changed (matches the game's
    /// `Energy.MaxAmount = ...` assignment in `Set2DSize`).
    pub max_amount: f32,
    /// Energy per amount unit (1.0 for energy; `health_material_density` for HP).
    pub density: f32,
}

impl MatterPool {
    /// Creates a pool.
    pub fn new(amount: f32, max_amount: f32, density: f32) -> Self {
        Self {
            amount,
            max_amount,
            density,
        }
    }

    /// `max(amount / max_amount, 0)`; `0` when the maximum is not positive
    /// (`Matter.cs:16`).
    pub fn fullness(&self) -> f32 {
        if self.max_amount > 0.0 {
            (self.amount / self.max_amount).max(0.0)
        } else {
            0.0
        }
    }

    /// `max(amount - max_amount, 0)` (`Matter.cs:22`).
    pub fn overflow(&self) -> f32 {
        (self.amount - self.max_amount).max(0.0)
    }

    /// `max(max_amount - amount, 0)` (`Matter.cs:24`).
    pub fn available(&self) -> f32 {
        (self.max_amount - self.amount).max(0.0)
    }

    /// `Mathf.Approximately(amount, 0)` equivalent (`Matter.cs:18`).
    pub fn is_empty(&self) -> bool {
        self.amount.abs() <= EPS
    }

    /// `Mathf.Approximately(available, 0)` equivalent (`Matter.cs:20`).
    pub fn is_full(&self) -> bool {
        self.available() <= EPS
    }

    /// `sqrt(amount / PI)` — the circular radius of the amount (`Matter.cs:26`).
    pub fn radius(&self) -> f32 {
        (self.amount / PI).sqrt()
    }

    /// `amount * density`.
    pub fn energy(&self) -> f32 {
        self.amount * self.density
    }

    /// Converts an amount of this matter to energy (`MatterMaterial.EnergyOfAmount`).
    pub fn energy_of_amount(&self, amount: f32) -> f32 {
        amount * self.density
    }

    /// Converts energy to an amount of this matter (`MatterMaterial.AmountOfEnergy`).
    pub fn amount_of_energy(&self, energy: f32) -> f32 {
        energy / self.density
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Local-space capsule collider shape (`CapsuleCollider2D`; size and offset are
/// multiplied by `d1Size` by the transform scale). `BibiteBody.cs:728-737`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CapsuleGeometry {
    /// Local width × height.
    pub size: Vec2,
    /// Local offset from the body origin.
    pub offset: Vec2,
}

impl CapsuleGeometry {
    /// Capsule shape at a given inflation factor (`BibiteBody.UpdateInflateFactor`).
    pub fn at_inflate(inflate_factor: f32) -> Self {
        let t = clamp01(inflate_factor);
        Self {
            size: Vec2::new(
                crate::math::lerp(CAPSULE_WIDTH_NOT_INFLATED, CAPSULE_WIDTH_FULLY_INFLATED, t),
                crate::math::lerp(
                    CAPSULE_HEIGHT_NOT_INFLATED,
                    CAPSULE_HEIGHT_FULLY_INFLATED,
                    t,
                ),
            ),
            offset: Vec2::new(
                0.0,
                crate::math::lerp(
                    CAPSULE_OFFSET_NOT_INFLATED,
                    CAPSULE_OFFSET_FULLY_INFLATED,
                    t,
                ),
            ),
        }
    }

    /// World-space shape after applying the body's uniform scale (`d1Size`).
    pub fn scaled(&self, d1_size: f32) -> Self {
        Self {
            size: self.size * d1_size,
            offset: self.offset * d1_size,
        }
    }
}

/// Derived body geometry. Recomputing this is the Rust equivalent of
/// `BibiteBody.Set2DSize`/`UpdateInflateFactor`/`UpdateMetabolism` geometry
/// side effects (`BibiteBody.cs:699-748`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyGeometry {
    /// `d2Size` state variable (`growth * SizeRatio²`).
    pub d2_size: f32,
    /// `sqrt(d2Size)` (`BibiteBody.cs:705`).
    pub d1_size: f32,
    /// `80 * d2Size` (`BibiteBody.cs:298`).
    pub base_body_area: f32,
    /// `12.5 * d1Size` (`BibiteBody.cs:296`).
    pub body_length: f32,
    /// Fat inflation factor, `0..=1` (`BibiteFatOrgan.cs:118-126`).
    pub inflate_factor: f32,
    /// `base_body_area * (1 + inflate_factor)` (`BibiteBody.cs:300`).
    pub real_body_area: f32,
    /// `sqrt(real_body_area / PI)` (`BibiteBody.cs:250`).
    pub effective_radius: f32,
    /// Armor shell thickness supplied by the armor organ; reduces the bite
    /// radius (`BibiteBody.cs:252`, `BibiteArmor.cs:58-62`).
    pub armor_thickness: f32,
    /// `effective_radius - armor_thickness` (`BibiteBody.cs:252`).
    pub internals_radius: f32,
    /// `bibiteMassDensity * base_body_area` (`BibiteBody.cs:724`).
    pub base_mass: f32,
    /// `healthPerArea * base_body_area` (`BibiteBody.cs:719`).
    pub max_health: f32,
    /// `base_body_area * storableEnergyPerArea` (`BibiteBody.cs:720`).
    pub max_energy: f32,
}

impl BodyGeometry {
    /// Builds geometry for a size and inflation state with no armor.
    pub fn from_d2(d2_size: f32, inflate_factor: f32, settings: &BodySettings) -> Self {
        let mut geometry = Self {
            d2_size,
            inflate_factor,
            ..Self::default()
        };
        geometry.recompute(settings);
        geometry
    }

    /// Recomputes all derived quantities from `d2_size`, `inflate_factor` and
    /// `armor_thickness`.
    ///
    /// Mirrors `Set2DSize` (area/length/mass/caps) plus `realBodyArea` and the
    /// two radii. Deliberately does **not** clamp `d2_size`: a negative value
    /// produces the same NaN the game would.
    pub fn recompute(&mut self, settings: &BodySettings) {
        self.d1_size = self.d2_size.sqrt();
        self.base_body_area = AREA_PER_D2 * self.d2_size;
        self.body_length = LENGTH_PER_D1 * self.d1_size;
        self.real_body_area = self.base_body_area * (1.0 + self.inflate_factor);
        self.effective_radius = (self.real_body_area / PI).sqrt();
        self.internals_radius = self.effective_radius - self.armor_thickness;
        self.base_mass = settings.bibite_mass_density * self.base_body_area;
        self.max_health = settings.health_per_area * self.base_body_area;
        self.max_energy = self.base_body_area * settings.storable_energy_per_area;
    }

    /// Sets the size and recomputes; returns the delta of `base_body_area`
    /// (`num2` in `Set2DSize`, `BibiteBody.cs:703-706`).
    pub fn set_d2(&mut self, new_d2_size: f32, settings: &BodySettings) -> f32 {
        let old_area = self.base_body_area;
        self.d2_size = new_d2_size;
        self.recompute(settings);
        self.base_body_area - old_area
    }

    /// Sets the fat inflation factor and recomputes area/radii.
    pub fn set_inflate(&mut self, inflate_factor: f32, settings: &BodySettings) {
        self.inflate_factor = inflate_factor;
        self.recompute(settings);
    }

    /// Sets the armor thickness (owned by the armor organ) and recomputes the
    /// internal radius.
    pub fn set_armor_thickness(&mut self, armor_thickness: f32) {
        self.armor_thickness = armor_thickness;
        self.internals_radius = self.effective_radius - armor_thickness;
    }

    /// Capsule collider shape at the current inflation factor.
    pub fn capsule(&self) -> CapsuleGeometry {
        CapsuleGeometry::at_inflate(self.inflate_factor)
    }

    /// Uniform sprite/transform scale (`Vector3.one * d1Size`,
    /// `BibiteBody.cs:709`).
    pub fn sprite_scale(&self) -> f32 {
        self.d1_size
    }

    /// `energyDensity = totalEnergy / realBodyArea` (`BibiteBody.cs:302`).
    pub fn energy_density(&self, total_energy: f32) -> f32 {
        total_energy / self.real_body_area
    }
}

// ---------------------------------------------------------------------------
// Aggregate body dynamics
// ---------------------------------------------------------------------------

/// Mass contributions of the mass-bearing organs (`BibiteBody.mass`,
/// `BibiteBody.cs:230`). The mouth and propulsion organs contribute zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OrganMasses {
    /// Stomach content mass (content mass × `stomachContentContribution`).
    pub stomach: f32,
    /// Stored-egg mass (`eggMassDensity * eggSize * eggProgress`).
    pub egg_layer: f32,
    /// Fat reserve mass.
    pub fat: f32,
    /// Armor shell mass.
    pub armor: f32,
}

impl OrganMasses {
    /// Sum of all organ masses.
    pub fn total(&self) -> f32 {
        self.stomach + self.egg_layer + self.fat + self.armor
    }
}

/// Rigid-body properties derived from the body (`BibiteBody.cs:636-637,741-742`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RigidBodyProperties {
    /// `rb2d.mass` (g).
    pub mass: f32,
    /// `rb2d.inertia = mass / 2 * realBodyArea / PI` (mass·u²).
    pub inertia: f32,
    /// `rb2d.linearDamping = dragCoefficient * d1Size / mass`.
    pub linear_damping: f32,
    /// `rb2d.angularDamping`, equal to the linear damping.
    pub angular_damping: f32,
}

/// Result of a generic energy spend.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnergySpend {
    /// Energy actually removed from the active pool (after efficiency).
    pub energy: f32,
    /// Health damage caused by an energy deficit (0 when the pool covered it).
    pub health_damage: f32,
    /// True when the spend killed the bibite.
    pub died: bool,
    /// True when the health debt was applied while already dead (corpse burst).
    pub exploded: bool,
}

/// Result of a damage application.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HurtReport {
    /// Damage actually applied (capped at `health + 0.01`).
    pub dealt: f32,
    /// True when the hit took health below zero for a living bibite.
    pub died: bool,
    /// True when the hit took a corpse below zero (corpse bursts into meat).
    pub exploded: bool,
}

/// Result of a bite (`BibiteBody.Attack`, `BibiteBody.cs:750-760`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AttackReport {
    /// Damage actually applied.
    pub dealt: f32,
    /// True when the victim died from this bite.
    pub died: bool,
    /// True when a corpse burst into meat from this bite.
    pub exploded: bool,
    /// Energy value of the flesh/blood removed (`dealt * HealthEnergyRatio`).
    pub energy_value: f32,
}

/// Result of a healing tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HealReport {
    /// Hit points gained (negative if `want_to_heal` is negative).
    pub hp: f32,
    /// Energy cost of the healing (before metabolic efficiency).
    pub energy_cost: f32,
    /// Spend outcome (includes efficiency and starvation damage).
    pub spend: EnergySpend,
}

/// Result of a size change (`Set2DSize`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeChange {
    /// Delta of `base_body_area` (the C# `num2`).
    pub delta_area: f32,
    /// Energy required to build that area (`delta_area * BodyEnergyDensity *
    /// (1 + healthEnergyFactor)`).
    pub energy_cost: f32,
}

/// Result of applying birth size (`StartBody`, `BibiteBody.cs:413-441`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BirthReport {
    /// Geometry/energy change of the initial size.
    pub size_change: SizeChange,
    /// Spend of the initial body build (no efficiency applied).
    pub build_spend: EnergySpend,
    /// Spend that removes the energy overflow above the new maximum.
    pub overflow_spend: EnergySpend,
}

/// Result of growing one size step (`BibiteGrowth.SetMaturity`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowthApplication {
    /// Geometry/energy change of the size update.
    pub size_change: SizeChange,
    /// Spend of the growth cost (metabolic efficiency applied).
    pub spend: EnergySpend,
    /// `growthCost / growthUpdatePeriod` (E/s, used by `EnergyFlowRate`).
    pub growth_cost_rate: f32,
}

/// Signal convention used for the collision normal speed.
///
/// ASSUMPTION(BQ-4): Unity does not document whether
/// `dot(contact.normal, collision.relativeVelocity)` is positive for a
/// head-on approach. The game uses it raw (`BibitePart.cs:57`); parity capture
/// will tell us whether [`AsIs`](CollisionSign::AsIs) or
/// [`Flipped`](CollisionSign::Flipped) is correct. [`Absolute`](CollisionSign::Absolute)
/// is a safe fallback if the sign proves irrelevant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollisionSign {
    /// Use the raw dot product (mirrors the decompiled source).
    #[default]
    AsIs,
    /// Use the negated dot product.
    Flipped,
    /// Use the absolute value.
    Absolute,
}

/// Outcome of entering the corpse state (`BibiteBody.Die`, `BibiteBody.cs:788-833`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeathReport {
    /// The bibite became a decaying corpse.
    pub became_corpse: bool,
    /// The body was swallowed: destroy it and do not spawn meat.
    pub swallowed_and_destroyed: bool,
    /// Meat energy to spawn immediately (no corpses setting), or `None`.
    pub meat_energy: Option<f32>,
    /// True when the bibite was not born yet, so no corpse/meat is produced.
    pub unborn_no_meat: bool,
}

/// One corpse tick (`BibiteBody.CorpseUpdate`, `BibiteBody.cs:641-648`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CorpseTick {
    /// Health damage from the corpse's own metabolism.
    pub health_damage: f32,
    /// Visual decay factor in `0..=1`.
    pub corpse_decay: f32,
    /// True when the corpse fell below zero health and bursts into meat.
    pub exploded: bool,
}

// ---------------------------------------------------------------------------
// Body state
// ---------------------------------------------------------------------------

/// Pure body state: geometry + energy/health pools + damage/lifecycle
/// bookkeeping. This is the Rust analogue of the `BibiteBody` fields that are
/// not organs or the brain.
///
/// The fields are laid out in the order they are declared, the ones the turn of a bibite writes first, then those it only reads
/// (see [`BibiteRecord`]).
#[derive(Clone, Debug, PartialEq)]
#[repr(C)]
pub struct BodyState {
    /// Runtime-only cumulative energy removed via [`BodyState::use_energy`]
    /// (E; never serialized / fingerprinted).
    pub energy_spent_total: f64,
    /// Runtime-only cumulative energy accepted via [`BodyState::gain_energy`]
    /// (E; never serialized / fingerprinted).
    pub energy_gained_total: f64,
    /// Active energy pool (E).
    pub energy: MatterPool,
    /// Health pool (HP).
    pub health: MatterPool,
    /// Pain accumulator (`attackedDmg`, `BibiteBody.cs:622-625`).
    pub attacked_dmg: f32,
    /// Total bite damage suffered (`totalDamageSuffered`).
    pub total_damage_suffered: f32,
    /// Last healing tick's energy rate (E/s), used by `EnergyFlowRate`.
    pub healing_rate_energy_cost: f32,
    /// Last movement cost (E/s, `BibitePropulsion.movementCost`), used by `EnergyFlowRate`. Not saved.
    pub movement_cost: f32,
    /// Base metabolism (E/s), including the ageing penalty
    /// (`BibiteBody.cs:745-748`).
    pub metabolism: f32,
    /// Number of *distinct* first bites taken (`timesAttacked`).
    pub times_attacked: u32,
    /// Energy subtracted from a corpse's meat value
    /// (`corpseEnergyOffset`, `BibiteBody.cs:828`).
    pub corpse_energy_offset: f32,
    /// Visual corpse decay in `0..=1`.
    pub corpse_decay: f32,
    /// Set when death starts (`dying`).
    pub dying: bool,
    /// Corpse flag (`dead`).
    pub dead: bool,
    /// Runtime-only (never saved): how many `Hurting` calls of this tick left the health below zero of a body that was not dead yet.
    /// The game's `Die()` runs on the first and, with corpses off, bursts the body into its meat pile on the spot; the body is only
    /// destroyed at the end of the frame, so every `UseEnergy` / `Attack` after the first one finds a dead body below zero health and
    /// bursts it again: one more meat pile (and one more stomach spill) per call. A starving body gets four (metabolism, fat,
    /// muscles, healing). Read by [`BibiteStore::die_now`], reset by [`BodyState::die`].
    pub death_hurts: u8,
    /// Gene-derived parameters.
    pub params: BodyParams,
    /// Derived geometry (size, area, radii, caps).
    pub geometry: BodyGeometry,
}

impl BodyState {
    /// Creates an empty body (size 0) with pools at 0 for the given params.
    pub fn new(params: BodyParams, settings: &BodySettings) -> Self {
        Self {
            geometry: BodyGeometry::default(),
            params,
            energy: MatterPool::new(0.0, 0.0, settings.energy_material_density),
            health: MatterPool::new(0.0, 0.0, settings.health_material_density),
            metabolism: 0.0,
            healing_rate_energy_cost: 0.0,
            movement_cost: 0.0,
            attacked_dmg: 0.0,
            times_attacked: 0,
            total_damage_suffered: 0.0,
            dying: false,
            dead: false,
            death_hurts: 0,
            corpse_energy_offset: 0.0,
            corpse_decay: 0.0,
            energy_gained_total: 0.0,
            energy_spent_total: 0.0,
        }
    }

    /// Builds a body at a given size and full health/energy (load/spawn path
    /// minus the birth energy spend; see [`BodyState::birth`]).
    pub fn with_size(
        params: BodyParams,
        d2_size: f32,
        health: f32,
        energy: f32,
        settings: &BodySettings,
    ) -> Self {
        let mut body = Self::new(params, settings);
        body.geometry = BodyGeometry::from_d2(d2_size, 0.0, settings);
        body.health = MatterPool::new(
            health,
            body.geometry.max_health,
            settings.health_material_density,
        );
        body.energy = MatterPool::new(
            energy,
            body.geometry.max_energy,
            settings.energy_material_density,
        );
        body
    }

    /// `body.mass` = base mass + mass-bearing organs (`BibiteBody.cs:230`).
    pub fn mass(&self, organs: OrganMasses) -> f32 {
        self.geometry.base_mass + organs.total()
    }

    /// Rigid-body properties for the current geometry and organ masses
    /// (`BibiteBody.cs:636-637,741-742`).
    pub fn rigid_body(&self, organs: OrganMasses, settings: &BodySettings) -> RigidBodyProperties {
        let mass = self.mass(organs);
        let inertia = if mass != 0.0 {
            mass / 2.0 * self.geometry.real_body_area / PI
        } else {
            0.0
        };
        let damping = if mass > 0.0 {
            settings.drag_coefficient * self.geometry.d1_size / mass
        } else {
            0.0
        };
        RigidBodyProperties {
            mass,
            inertia,
            linear_damping: damping,
            angular_damping: damping,
        }
    }

    /// `Energy.Amount / Energy.MaxAmount`.
    pub fn energy_ratio(&self) -> f32 {
        self.energy.fullness()
    }

    /// `Health.Amount / Health.MaxAmount`.
    pub fn health_ratio(&self) -> f32 {
        if self.health.max_amount > 0.0 {
            self.health.amount / self.health.max_amount
        } else {
            0.0
        }
    }

    /// `totalEnergy` (`BibiteBody.cs:256-266`). `extra_stored_energy` is the
    /// stored-egg energy plus fat-reserve energy, which live in organs.
    pub fn total_energy(&self, extra_stored_energy: f32) -> f32 {
        let corpse_offset = if self.dead {
            self.corpse_energy_offset
        } else {
            0.0
        };
        self.params.sum_body_health_energy_ratio * self.geometry.base_body_area
            + self.energy.amount
            + extra_stored_energy
            - corpse_offset
    }

    /// Energy content of a full health bar (`Health.Material.EnergyOfAmount(max)`).
    pub fn health_energy_of_max(&self) -> f32 {
        self.health.max_amount * self.params.health_material_density
    }

    /// Recomputes [`BodyState::metabolism`] from the area and the current age.
    /// This is the Rust equivalent of `BibiteBody.UpdateMetabolism`
    /// (`BibiteBody.cs:745-748`), which the game calls on `Set2DSize` and every
    /// 120 s of ageing.
    pub fn update_metabolism(&mut self, age_hours: f32, settings: &BodySettings) {
        let factor = ageing_factor(
            age_hours,
            settings.base_aging_threshold,
            self.params.metabolic_rate,
        );
        self.metabolism = settings.base_metabolism_cost
            * self.geometry.base_body_area
            * (1.0 + factor * settings.age_metabolism_penalties);
    }

    /// `(1/d2Size)^healPowerFactor * healRate * SpeedRatio` — fraction of max
    /// HP per second (`BibiteBody.cs:244`).
    pub fn heal_max_rate(&self, settings: &BodySettings) -> f32 {
        if self.geometry.d2_size > 0.0 {
            (1.0 / self.geometry.d2_size).powf(settings.heal_power_factor)
                * settings.heal_rate
                * self.params.metabolic_rate
        } else {
            0.0
        }
    }

    /// `UseEnergy` (`BibiteBody.cs:658-669`).
    ///
    /// An energy deficit is converted to health damage through the health
    /// material density and can kill the bibite (or burst a corpse).
    pub fn use_energy(&mut self, qty_e: f32, use_efficiency: bool) -> EnergySpend {
        let mut qty = if use_efficiency {
            qty_e / self.params.metabolic_efficiency
        } else {
            qty_e
        };
        if self.energy.amount > qty {
            self.energy.amount -= qty;
            self.energy_spent_total += f64::from(qty);
            return EnergySpend {
                energy: qty,
                ..EnergySpend::default()
            };
        }
        qty -= self.energy.amount;
        let spent = self.energy.amount;
        self.energy.amount = 0.0;
        self.energy_spent_total += f64::from(spent);
        let damage = qty / self.params.health_material_density;
        let hurt = self.hurt(damage);
        EnergySpend {
            energy: spent,
            health_damage: hurt.dealt,
            died: hurt.died,
            exploded: hurt.exploded,
        }
    }

    /// `GainEnergy` (`BibiteBody.cs:687-697`). Negative `qty_e` drains energy
    /// (used by fat storage and shrinking growth).
    pub fn gain_energy(&mut self, qty_e: f32) -> f32 {
        // A gain that is NaN is no gain: the comparison below is false for it, which fills the energy bar (the game's `GainEnergy` does the
        // same: a NaN digestion or fat output would give a bibite a full bar every tick).
        if qty_e.is_nan() {
            return 0.0;
        }
        let available = self.energy.available();
        let accepted = if available > qty_e {
            self.energy.amount += qty_e;
            qty_e
        } else {
            self.energy.amount = self.energy.max_amount;
            available
        };
        self.energy_gained_total += f64::from(accepted.max(0.0));
        accepted
    }

    /// Applies a size change (`Set2DSize`, `BibiteBody.cs:699-726`).
    ///
    /// Mirrors the game exactly:
    /// * no health delta when `initial_set`,
    /// * `health += healthPerArea * delta_area` otherwise (can go negative),
    /// * max health/energy recomputed afterwards (no clamping of amounts),
    /// * metabolism recomputed at `age_hours`.
    pub fn set_2d_size(
        &mut self,
        new_d2_size: f32,
        initial_set: bool,
        age_hours: f32,
        settings: &BodySettings,
    ) -> SizeChange {
        let delta_area = self.geometry.set_d2(new_d2_size, settings);
        if !initial_set {
            self.health.amount += settings.health_per_area * delta_area;
        }
        self.health.max_amount = self.geometry.max_health;
        self.energy.max_amount = self.geometry.max_energy;
        self.update_metabolism(age_hours, settings);
        SizeChange {
            delta_area,
            energy_cost: delta_area
                * self.params.base_energy_density
                * (1.0 + settings.health_energy_factor),
        }
    }

    /// Sets the fat inflation factor (collider/sprite/armor geometry side).
    pub fn set_inflate_factor(&mut self, inflate_factor: f32, settings: &BodySettings) {
        self.geometry.set_inflate(inflate_factor, settings);
    }

    /// Sets the armor thickness supplied by the armor organ.
    pub fn set_armor_thickness(&mut self, armor_thickness: f32) {
        self.geometry.set_armor_thickness(armor_thickness);
    }

    /// Birth path of `StartBody` (`BibiteBody.cs:413-441`):
    /// initial size, full health, build the body from energy without
    /// efficiency, then remove energy overflow above the new maximum.
    pub fn birth(&mut self, new_d2_size: f32, settings: &BodySettings) -> BirthReport {
        let size_change = self.set_2d_size(new_d2_size, true, 0.0, settings);
        self.health.amount = self.health.max_amount;
        let build_spend = self.use_energy(size_change.energy_cost, false);
        let overflow = self.energy.overflow();
        let overflow_spend = if overflow > 0.0 {
            self.use_energy(overflow, false)
        } else {
            EnergySpend::default()
        };
        BirthReport {
            size_change,
            build_spend,
            overflow_spend,
        }
    }

    /// `Heal` (`BibiteBody.cs:671-685`). Note the game applies
    /// `healEfficiency` here and `MetabolicEfficiency` again inside
    /// `UseEnergy` (double penalty) — reproduce both.
    pub fn heal(&mut self, want_to_heal: f32, dt: f32, settings: &BodySettings) -> HealReport {
        if self.health.amount >= self.health.max_amount {
            self.healing_rate_energy_cost = 0.0;
            return HealReport::default();
        }
        let hp = (self.health.max_amount * self.heal_max_rate(settings) * want_to_heal * dt)
            .min(self.health.max_amount - self.health.amount);
        let cost = hp * self.params.health_energy_ratio / settings.heal_efficiency;
        self.healing_rate_energy_cost = if dt > 0.0 { cost / dt } else { 0.0 };
        let spend = self.use_energy(cost, true);
        self.health.amount += hp;
        HealReport {
            hp,
            energy_cost: cost,
            spend,
        }
    }

    /// `Hurting` (`BibiteBody.cs:762-779`). Does not perform the death
    /// transition itself; the caller inspects the report and calls
    /// [`BodyState::die`] / handles the corpse burst.
    pub fn hurt(&mut self, dmg: f32) -> HurtReport {
        let dealt = dmg.min(self.health.amount + 0.01);
        self.health.amount -= dealt;
        if self.health.amount < 0.0 {
            if self.dead {
                HurtReport {
                    dealt,
                    died: false,
                    exploded: true,
                }
            } else {
                self.death_hurts = self.death_hurts.saturating_add(1);
                HurtReport {
                    dealt,
                    died: true,
                    exploded: false,
                }
            }
        } else {
            HurtReport {
                dealt,
                died: false,
                exploded: false,
            }
        }
    }

    /// `Attack` (`BibiteBody.cs:750-760`).
    pub fn attack(&mut self, dmg: f32, drain: bool) -> AttackReport {
        let hurt = self.hurt(dmg);
        self.attacked_dmg += hurt.dealt;
        self.total_damage_suffered += hurt.dealt;
        if !drain {
            self.times_attacked += 1;
        }
        AttackReport {
            dealt: hurt.dealt,
            died: hurt.died,
            exploded: hurt.exploded,
            energy_value: hurt.dealt * self.params.health_energy_ratio,
        }
    }

    /// Starts death (`BibiteBody.Die`, `BibiteBody.cs:788-833`).
    ///
    /// `extra_stored_energy` is stored-egg energy + fat energy (organs). With
    /// corpses disabled, returns the meat-pile energy to spawn. With corpses
    /// enabled and `swallowed` true, the body is destroyed without meat.
    pub fn die(
        &mut self,
        corpses_enabled: bool,
        born: bool,
        swallowed: bool,
        extra_stored_energy: f32,
    ) -> DeathReport {
        self.dying = true;
        if self.dead {
            return DeathReport::default();
        }
        self.dead = true;
        self.death_hurts = 0;
        if !corpses_enabled {
            return DeathReport {
                meat_energy: Some(self.total_energy(extra_stored_energy)),
                ..DeathReport::default()
            };
        }
        if !born {
            return DeathReport {
                unborn_no_meat: true,
                ..DeathReport::default()
            };
        }
        if swallowed {
            return DeathReport {
                swallowed_and_destroyed: true,
                ..DeathReport::default()
            };
        }
        self.corpse_energy_offset = self.health_energy_of_max() - self.energy.amount;
        self.energy.amount = 0.0;
        self.health.amount = self.health.max_amount;
        DeathReport {
            became_corpse: true,
            ..DeathReport::default()
        }
    }

    /// One corpse tick (`CorpseUpdate`, `BibiteBody.cs:641-648`): the corpse
    /// burns `metabolism * corpseDecayFactor` from an empty energy pool, so it
    /// damages health. `damagedFlash` is visual-only and omitted.
    pub fn corpse_tick(&mut self, dt: f32, settings: &BodySettings) -> CorpseTick {
        self.dying = false;
        let spend = self.use_energy(self.metabolism * settings.corpse_decay_factor * dt, true);
        let ratio = self.health_ratio();
        // Mathf.InverseLerp clamps to 0..1.
        self.corpse_decay = clamp01(inv_lerp(1.0, 0.5, ratio));
        CorpseTick {
            health_damage: spend.health_damage,
            corpse_decay: self.corpse_decay,
            exploded: spend.died || spend.exploded || self.health.amount < 0.0,
        }
    }

    /// Energy of the meat pile spawned when this corpse bursts (dead body, so
    /// `corpse_energy_offset` is subtracted).
    pub fn corpse_meat_energy(&self, extra_stored_energy: f32) -> f32 {
        self.total_energy(extra_stored_energy)
    }

    /// Applies one [`GrowthStep`] to the body (`SetMaturity`,
    /// `BibiteGrowth.cs:124-144`): resize then spend the growth energy.
    ///
    /// **Ordering (game-faithful):** the game first fires `onGrowth` (organs
    /// recompute their `area` from the *old* `baseBodyArea`) and only then calls
    /// `Set2DSize`. B2 must therefore refresh organ areas before calling this
    /// method; B1 exposes the pre-change area in [`GrowthStep::organ_base_body_area`].
    pub fn apply_growth_step(
        &mut self,
        step: &GrowthStep,
        age_hours: f32,
        settings: &BodySettings,
    ) -> GrowthApplication {
        let size_change = self.set_2d_size(step.new_d2_size, false, age_hours, settings);
        let spend = self.use_energy(size_change.energy_cost, true);
        GrowthApplication {
            size_change,
            spend,
            growth_cost_rate: if step.period > 0.0 {
                size_change.energy_cost / step.period
            } else {
                0.0
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Free formula helpers: ageing, metabolism, healing, damage
// ---------------------------------------------------------------------------

/// `age = timeAlive / 3600` hours (`BibiteBody.cs:304`).
pub fn age_hours(time_alive_seconds: f32) -> f32 {
    time_alive_seconds / 3600.0
}

/// `ageingFactor = max((age - agingThreshold) / agingThreshold, 0)` with
/// `agingThreshold = baseAgingThreshold / SpeedRatio` (`BibiteBody.cs:306-308`).
pub fn ageing_factor(age_hours: f32, base_aging_threshold: f32, metabolic_rate: f32) -> f32 {
    if metabolic_rate > 0.0 {
        let threshold = base_aging_threshold / metabolic_rate;
        ((age_hours - threshold) / threshold).max(0.0)
    } else {
        0.0
    }
}

/// `(1 - ageStrengthPenalties)^ageingFactor` (`BibiteBody.cs:310`).
pub fn age_strength_penalty_factor(ageing_factor: f32, settings: &BodySettings) -> f32 {
    (1.0 - settings.age_strength_penalties).powf(ageing_factor)
}

/// `ageingFactor * ageMetabolismPenalties` (`BibiteBody.cs:312`).
pub fn age_metabolism_penalty_factor(ageing_factor: f32, settings: &BodySettings) -> f32 {
    ageing_factor * settings.age_metabolism_penalties
}

/// `attackedDmg = max(0, attackedDmg * 0.70716^dt - 0.003 * dt)`
/// (`BibiteBody.cs:622-625`).
pub fn attacked_dmg_decay(attacked_dmg: f32, dt: f32) -> f32 {
    (attacked_dmg * ATTACKED_DMG_DECAY_BASE.powf(dt) - ATTACKED_DMG_DECAY_LINEAR * dt).max(0.0)
}

/// Brain input `AttackedDamage = min(1, 2 * attackedDmg / health)`
/// (`NEATBrain.cs:583-584`).
pub fn attacked_dmg_brain_input(attacked_dmg: f32, health: f32) -> f32 {
    if health > 0.0 {
        (2.0 * attacked_dmg / health).min(1.0)
    } else {
        0.0
    }
}

/// Collision damage (`BibitePart.cs:47-66`).
///
/// ```text
/// impulseSpeed = 2 * otherMass * vn / (otherMass + selfMass)
/// damage       = max(0, impulseSpeed - threshold) * constant
/// ```
///
/// `vn` is `dot(contact.normal, other.relativeVelocity)`; see
/// [`CollisionSign`] for the (unverified) sign convention.
pub fn collision_damage(
    normal_speed: f32,
    self_mass: f32,
    settings: &BodySettings,
    sign: CollisionSign,
) -> f32 {
    let vn = match sign {
        CollisionSign::AsIs => normal_speed,
        CollisionSign::Flipped => -normal_speed,
        CollisionSign::Absolute => normal_speed.abs(),
    };
    // `BibitePart.cs:55-57`: `other.otherRigidbody` is the *receiving*
    // body's own rigidbody (Unity 2D `Collision2D`), so the mass term is
    // `2m / (m + m)` and the metric reduces to the normal speed.
    let total_mass = self_mass + self_mass;
    if total_mass == 0.0 {
        return 0.0;
    }
    let impulse_speed = 2.0 * self_mass * vn / total_mass;
    if impulse_speed < settings.collision_damage_threshold {
        0.0
    } else {
        (impulse_speed - settings.collision_damage_threshold) * settings.collision_damage_constant
    }
}

/// Bite damage to a bibite (`BibiteMouth.cs:715`):
/// `bitingDamageFactor * target.maxHealth * chunkArea / target.realBodyArea`.
pub fn bite_damage(
    target_max_health: f32,
    chunk_area: f32,
    target_real_body_area: f32,
    settings: &BodySettings,
) -> f32 {
    if target_real_body_area <= 0.0 {
        0.0
    } else {
        settings.biting_damage_factor * target_max_health * chunk_area / target_real_body_area
    }
}

/// Scale applied to ongoing-bite damage: `dt / (5 * bitePeriod)`
/// (`BibiteMouth.cs:720-723`).
pub fn ongoing_bite_damage_scale(dt: f32, bite_period: f32) -> f32 {
    if bite_period > 0.0 {
        dt / (ONGOING_BITE_PERIODS * bite_period)
    } else {
        0.0
    }
}

/// Armor blocks damage when `biteStrength <= armorStrength`
/// (`BibiteMouth.cs:716-719`). `armor_strength` must be the **doubled** value
/// exposed by `BibiteBody.armorStrength` (`BibiteBody.cs:246`).
pub fn armor_blocks(bite_strength: f32, armor_strength: f32) -> bool {
    bite_strength <= armor_strength
}

/// Bite force after the armor shell absorbs part of it on a first bite:
/// `max(biteStrength - armorStrength, 0)` (`BibiteMouth.cs:647-649`).
pub fn reduced_bite_force(bite_strength: f32, armor_strength: f32) -> f32 {
    (bite_strength - armor_strength).max(0.0)
}

// ---------------------------------------------------------------------------
// Growth ODE and maturity (spec 02 §3.4)
// ---------------------------------------------------------------------------

/// `growthAtBirth = (HatchTime / BroodTime)²` (`BibiteGenes.cs:163-166`).
pub fn growth_at_birth(hatch_time: f32, brood_time: f32) -> f32 {
    if brood_time > 0.0 {
        (hatch_time / brood_time).powi(2)
    } else {
        0.0
    }
}

/// `growthAtMature = max(1, growthAtBirth / max(wombPortion, 0.001))`
/// (`BibiteGenes.cs:168-171`).
pub fn growth_at_mature(growth_at_birth: f32, womb_area_portion: f32) -> f32 {
    (growth_at_birth / womb_area_portion.max(0.001)).max(1.0)
}

/// `AreaAtMature = 80 * growthAtMature * SizeRatio²` (`BibiteGenes.cs:173-176`).
pub fn area_at_mature(growth_at_mature: f32, size_ratio: f32) -> f32 {
    AREA_PER_D2 * growth_at_mature * size_ratio * size_ratio
}

/// `LengthAtMature = 12.5 * sqrt(growthAtMature) * SizeRatio`
/// (`BibiteGenes.cs:178-181`).
pub fn length_at_mature(growth_at_mature: f32, size_ratio: f32) -> f32 {
    LENGTH_PER_D1 * growth_at_mature.sqrt() * size_ratio
}

/// Gene-derived growth parameters (the subset of [`crate::genome`] the growth
/// ODE needs). Primitive struct so B1 does not depend on `genome.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowthGenes {
    /// `gene.GrowthAtBigEnoughEggOrgan` = [`growth_at_mature`] value.
    pub growth_at_mature: f32,
    /// `SizeRatio` gene.
    pub size_ratio: f32,
    /// `GrowthScale` gene.
    pub growth_scale: f32,
    /// `GrowthMaturityFactor` gene.
    pub growth_maturity_factor: f32,
    /// `GrowthMaturityExponent` gene.
    pub growth_maturity_exponent: f32,
    /// `SpeedRatio` gene (= `metabolicRate`).
    pub metabolic_rate: f32,
    /// `HerdVelocityWeight` gene: the movement step of every bibite reads it every tick.
    pub herd_velocity_weight: f32,
}

/// `rate(maturity) = SpeedRatio / (1 + GrowthMaturityFactor * maturity^Exp)`
/// (`BibiteGrowth.cs:114-117`).
pub fn growth_rate(maturity: f32, genes: &GrowthGenes) -> f32 {
    // `Mathf.Pow` is a float (the double power rounded); the rest of the expression is one double expression rounded once on return (the game's arithmetic, see `math::sigmoid`).
    let power = f64::from(crate::math::pow(maturity, genes.growth_maturity_exponent));
    (f64::from(genes.metabolic_rate) / (1.0 + f64::from(genes.growth_maturity_factor) * power)) as f32
}

/// One growth-step decision (`BibiteGrowth.FixedUpdate`, `BibiteGrowth.cs:91-107`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowthStep {
    /// New `growth` value after the step.
    pub new_growth: f32,
    /// New `d2Size = new_growth * SizeRatio²`.
    pub new_d2_size: f32,
    /// `baseBodyArea` **before** the step; organs that receive `onGrowth`
    /// before `Set2DSize` recompute their area from this value.
    pub organ_base_body_area: f32,
    /// Whether the transform scale should be updated
    /// (`(new-applied)/applied > 0.005`).
    pub update_scale: bool,
    /// Growth period used for this step (seconds).
    pub period: f32,
    /// The bibite crossed the maturity threshold in this step.
    pub became_mature: bool,
    /// The bibite dropped below the maturity threshold in this step.
    pub became_immature: bool,
}

/// Growth/maturity state machine (`BibiteGrowth`).
#[derive(Clone, Debug, PartialEq)]
pub struct GrowthState {
    /// Current growth state variable (`growth`), serialized by the game.
    pub growth: f32,
    /// Last growth value applied to the geometry (`appliedGrowth`).
    pub applied_growth: f32,
    /// Accumulated time towards the next growth step (`progress`).
    pub progress: f32,
    /// Current adaptive period (`growthUpdatePeriod`, starts at 0.2 s).
    pub update_period: f32,
    /// Last growth energy rate (E/s), used by `EnergyFlowRate`.
    pub growth_cost_rate: f32,
    /// Whether the bibite has reached maturity.
    pub mature: bool,
    /// Whether growth processing is enabled (`growing`).
    pub growing: bool,
}

impl GrowthState {
    /// Creates a growth state at a given growth value (`ResumeGrowth`,
    /// `BibiteGrowth.cs:74-89`).
    pub fn new(growth: f32, genes: &GrowthGenes) -> Self {
        Self {
            growth,
            applied_growth: growth,
            progress: 0.0,
            update_period: 0.2,
            growth_cost_rate: 0.0,
            mature: growth >= genes.growth_at_mature,
            growing: true,
        }
    }

    /// `maturity = growth / growthAtMature` (`BibiteGrowth.cs:52`).
    pub fn maturity(&self, genes: &GrowthGenes) -> f32 {
        if genes.growth_at_mature > 0.0 {
            self.growth / genes.growth_at_mature
        } else {
            0.0
        }
    }

    /// Advances the growth timer by `dt`; returns a step when one is due.
    ///
    /// At most one step per call (the game has no catch-up loop). Applies the
    /// growth/maturity state updates immediately; the caller then refreshes
    /// organ areas (game-faithful pre-size order) and calls
    /// [`BodyState::apply_growth_step`].
    pub fn tick(&mut self, dt: f32, want_to_grow: f32, genes: &GrowthGenes) -> Option<GrowthStep> {
        if !self.growing || dt <= 0.0 {
            return None;
        }
        self.progress += dt;
        let period = self.update_period.max(dt);
        if self.progress < period {
            return None;
        }
        self.update_period = period;
        self.progress -= period;

        let maturity = self.maturity(genes);
        let rate = growth_rate(maturity, genes);
        // `brain.Output(Want2Grow) * num * growthUpdatePeriod * growthScale / 100f`: one double expression, rounded once when stored.
        let delta = (f64::from(want_to_grow) * f64::from(rate) * f64::from(period) * f64::from(genes.growth_scale) / 100.0) as f32;
        let new_growth = self.growth + delta;
        let organ_base_body_area = AREA_PER_D2 * self.growth * genes.size_ratio * genes.size_ratio;
        let was_mature = self.mature;

        self.growth = new_growth;
        self.mature = new_growth >= genes.growth_at_mature;
        self.update_period = if rate > 0.0 {
            0.2 / rate
        } else {
            self.update_period
        };

        // `(growth - appliedGrowth) / appliedGrowth > 0.005f`: the quotient stays a double in the comparison.
        let update_scale = if self.applied_growth > 0.0 {
            (f64::from(new_growth) - f64::from(self.applied_growth)) / f64::from(self.applied_growth) > f64::from(0.005_f32)
        } else {
            true
        };
        if update_scale {
            self.applied_growth = new_growth;
        }

        Some(GrowthStep {
            new_growth,
            // `body.Set2DSize(growth * sizeRatio * sizeRatio, ..)`: the argument is one double expression rounded once.
            new_d2_size: (f64::from(new_growth) * f64::from(genes.size_ratio) * f64::from(genes.size_ratio)) as f32,
            organ_base_body_area,
            update_scale,
            period,
            became_mature: !was_mature && self.mature,
            became_immature: was_mature && !self.mature,
        })
    }

    /// Records the growth cost rate computed by
    /// [`BodyState::apply_growth_step`].
    pub fn record_cost_rate(&mut self, growth_cost_rate: f32) {
        self.growth_cost_rate = growth_cost_rate;
    }

    /// Stops growth (`StopGrowth`).
    pub fn stop(&mut self) {
        self.growing = false;
        self.growth_cost_rate = 0.0;
    }
}

// ---------------------------------------------------------------------------
// B2a: BibiteStore / BibiteView / spawn and save accessors, EggStore
// ---------------------------------------------------------------------------

/// Physics tag kind for a bibite.
pub const TAG_KIND_BIBITE: u8 = 1;
/// Physics tag kind for an egg.
pub const TAG_KIND_EGG: u8 = 2;
/// Bibite main-body capsule collider index.
pub const COLLIDER_BODY: u8 = 0;
/// Bibite tail capsule collider index.
pub const COLLIDER_TAIL: u8 = 1;
/// Bibite mouth trigger collider index.
pub const COLLIDER_MOUTH: u8 = 2;

/// Packs an entity kind and arena handle into a physics tag.
pub fn encode_tag(kind: u8, handle: Handle) -> u64 {
    (u64::from(kind) << 56) | (u64::from(handle.generation()) << 32) | u64::from(handle.slot())
}

/// Returns the kind byte of a physics tag.
pub fn tag_kind(tag: u64) -> u8 {
    (tag >> 56) as u8
}

/// Returns the arena handle encoded in a physics tag.
pub fn decode_tag(tag: u64) -> Handle {
    Handle::new(
        (tag & 0xFFFF_FFFF) as u32,
        ((tag >> 32) & 0x00FF_FFFF) as u32,
    )
}

/// Physics tag of a bibite.
pub fn bibite_tag(id: BibiteId) -> u64 {
    encode_tag(TAG_KIND_BIBITE, id.handle())
}

/// Physics tag of an egg.
pub fn egg_tag(id: EggId) -> u64 {
    encode_tag(TAG_KIND_EGG, id.handle())
}

impl BodySettings {
    /// Builds the pure-math body settings from the aggregated simulation
    /// settings (`docs/INTERFACES.md` §3) using fmt-dev's field mapping.
    pub fn from_sim_settings(s: &SimSettings) -> Self {
        let sc = &s.scenario;
        Self {
            health_per_area: sc.health_per_area,
            storable_energy_per_area: sc.storable_energy_per_area,
            bibite_mass_density: sc.bibite_mass_density,
            base_body_growth_cost: sc.base_body_growth_cost,
            view_angle_added_cost: sc.view_angle_added_cost,
            view_radius_added_cost: sc.view_radius_added_cost,
            armor_energy_density: s.materials.armor.energy_density,
            health_energy_factor: sc.health_energy_factor,
            base_metabolism_cost: sc.base_metabolism_cost,
            energy_usage_efficiency: sc.energy_usage_efficiency,
            heal_rate: sc.heal_rate,
            heal_power_factor: sc.heal_power_factor,
            heal_efficiency: sc.heal_efficiency,
            base_aging_threshold: sc.ageing_threshold,
            age_strength_penalties: sc.age_strength_penalties,
            age_metabolism_penalties: sc.age_metabolism_penalties,
            corpse_decay_factor: sc.corpse_decay_factor,
            drag_coefficient: sc.drag_coefficient,
            collision_damage_constant: sc.collision_damage_constant,
            collision_damage_threshold: sc.collision_damage_threshold,
            biting_damage_factor: sc.biting_damage_factor,
            energy_material_density: 1.0,
            health_material_density: 0.25,
        }
    }
}

/// Converts a settings-side material into the organ-math form.
pub fn organ_material(m: &crate::settings::MaterialSetting) -> MaterialParams {
    MaterialParams {
        cohesiveness: m.cohesiveness,
        reactivity: m.reactivity,
        energy_density: m.energy_density,
        mass_density: m.mass_density,
        min_efficiency: m.min_efficiency,
        max_efficiency: m.max_efficiency,
    }
}

fn mouth_settings(s: &SimSettings) -> MouthSettings {
    MouthSettings {
        bibite_mass_density: s.scenario.bibite_mass_density,
        biting_pressure: s.scenario.biting_pressure,
        jaw_muscles_sizing_power: s.scenario.jaw_muscles_sizing_power,
        bite_period_factor: s.scenario.bite_period_factor,
    }
}

fn propulsion_settings(s: &SimSettings) -> PropulsionSettings {
    PropulsionSettings {
        arm_muscle_pressure: s.scenario.arm_muscle_pressure,
        forward_force_size_power: s.scenario.forward_force_size_power,
        turn_force_size_power: s.scenario.turn_force_size_power,
        backward_fraction: s.scenario.backward_fraction,
        move_muscles_cost: s.scenario.move_muscles_cost,
    }
}

fn growth_genes_of(genome: &Genome) -> GrowthGenes {
    GrowthGenes {
        growth_at_mature: genome.growth_at_mature(),
        size_ratio: genome.gene(Gene::SizeRatio),
        growth_scale: genome.gene(Gene::GrowthScale),
        growth_maturity_factor: genome.gene(Gene::GrowthMaturityFactor),
        growth_maturity_exponent: genome.gene(Gene::GrowthMaturityExponent),
        metabolic_rate: genome.metabolic_rate(),
        herd_velocity_weight: genome.gene(Gene::HerdVelocityWeight),
    }
}

fn brain_upkeep_per_second(brain: &Brain, s: &SimSettings) -> f32 {
    brain.upkeep().energy_per_second(
        s.scenario.synapse_upkeep_cost,
        s.scenario.neuron_upkeep_cost,
    )
}

/// Vitals mode of a [`BibiteSpawn`] (game `GrowthAtSpawn`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpawnVitals {
    /// Hatch from an egg: birth size, egg energy (the `Egg` mode resolves to an
    /// egg in the spawner, and the hatched child then uses this mode).
    #[default]
    FromEgg,
    /// Spawn just after hatching (`Baby`: growth at birth).
    Baby,
    /// Spawn as an adult (`Adult`: growth at mature).
    Adult,
    /// Spawn at twice the mature growth (`Elder`).
    Elder,
}

impl SpawnVitals {
    /// Maps a scenario `GrowthAtSpawn` mode.
    #[must_use]
    pub fn from_growth_at_spawn(mode: GrowthAtSpawn) -> Self {
        match mode {
            GrowthAtSpawn::Egg => Self::FromEgg,
            GrowthAtSpawn::Baby => Self::Baby,
            GrowthAtSpawn::Adult => Self::Adult,
            GrowthAtSpawn::Elder => Self::Elder,
        }
    }

    /// Target growth value for this mode (`None` for egg birth size).
    #[must_use]
    pub fn target_growth(self, genome: &Genome) -> Option<f32> {
        match self {
            Self::FromEgg => None,
            Self::Baby => Some(genome.growth_at_birth()),
            Self::Adult => Some(genome.growth_at_mature()),
            Self::Elder => Some(2.0 * genome.growth_at_mature()),
        }
    }
}

/// Request to spawn a bibite into the store.
#[derive(Clone, Debug)]
pub struct BibiteSpawn {
    /// World position.
    pub pos: Vec2,
    /// World rotation (radians).
    pub rot: f32,
    /// Initial linear velocity.
    pub vel: Vec2,
    /// Initial angular velocity (rad/s).
    pub ang_vel: f32,
    /// Genome.
    pub genome: Genome,
    /// Genome metadata.
    pub meta: GenomeMeta,
    /// Brain.
    pub brain: Brain,
    /// Vitals mode.
    pub vitals: SpawnVitals,
    /// Energy override (`None`: egg energy for `FromEgg`, full energy otherwise).
    pub energy: Option<f32>,
    /// Health override (`None`: full health).
    pub health: Option<f32>,
}

impl BibiteSpawn {
    /// Creates a resting spawn with default vitals (adult, full energy).
    pub fn new(genome: Genome, meta: GenomeMeta, brain: Brain) -> Self {
        Self {
            pos: Vec2::ZERO,
            rot: 0.0,
            vel: Vec2::ZERO,
            ang_vel: 0.0,
            genome,
            meta,
            brain,
            vitals: SpawnVitals::Adult,
            energy: None,
            health: None,
        }
    }
}

/// Why a bibite died (world-visible; B2b consumes this for meat/removal).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeathCause {
    /// Energy debt.
    #[default]
    Starvation,
    /// Killed by a bite.
    Bitten,
    /// Swallowed whole.
    Swallowed,
    /// Collision damage.
    Collision,
    /// World hazard (red death, tools).
    Hazard,
    /// Removed by a load/tool path.
    Removed,
}

/// Per-bibite brain sense-throttle counters (to be advanced by the B2b tick).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SenseThrottle {
    /// Ticks since the last brain update.
    pub brain_counter: u32,
    /// Brain updates since the last field-of-view entity lookup.
    pub lookup_counter: u32,
    /// Brain updates since the last sense refresh.
    pub sense_counter: u32,
}

/// Body-owned steering inputs written by the world's FOV/zone phase.
///
/// The body never computes FOV itself (`docs/INTERFACES.md` §5). The world
/// derives these from `senses::FovResult.herd` plus its zone pass and calls
/// `BibiteStore::set_steering` before [`BibiteStore::tick`]. Fields not yet
/// produced by the world (`herd_velocity_match`, `nearest_separation_distance`,
/// `void_avoidance`) are reserved and currently unused by the motion organ.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SteeringInputs {
    /// `FieldOfView.hasHerd`.
    pub has_herd: bool,
    /// `FieldOfView.herdSumDirection`, signed degrees.
    pub herd_sum_direction_degrees: f32,
    /// `FieldOfView.herdSeparationProjection`.
    pub herd_separation_projection: f32,
    /// Reserved: herd velocity-matching vector (world FOV alignment term).
    pub herd_velocity_match: Vec2,
    /// Reserved: nearest herd-neighbour distance.
    pub nearest_separation_distance: f32,
    /// Reserved: optional void-avoidance direction supplied by the world.
    pub void_avoidance: Vec2,
}

impl SteeringInputs {
    /// Builds the input from world FOV herd metrics.
    #[must_use]
    pub fn from_fov(herd: &crate::senses::HerdState) -> Self {
        Self {
            has_herd: herd.has_herd,
            herd_sum_direction_degrees: herd.sum_direction,
            herd_separation_projection: herd.separation_projection,
            ..Self::default()
        }
    }
}

/// Derived per-bibite cache (recomputed on spawn/load/growth).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BibiteDerived {
    /// `body.mass` including mass-bearing organs (g).
    pub mass: f32,
    /// `rb2d.inertia` (mass·u²).
    pub inertia: f32,
    /// `rb2d.linearDamping`.
    pub linear_damping: f32,
    /// `rb2d.angularDamping`.
    pub angular_damping: f32,
    /// 1-D size.
    pub d1_size: f32,
    /// 2-D size.
    pub d2_size: f32,
    /// `80 * d2Size`.
    pub base_body_area: f32,
    /// `12.5 * d1Size`.
    pub body_length: f32,
    /// Brain upkeep rate (E/s).
    pub brain_upkeep: f32,
}

/// UI-facing snapshot of the fields shown by the game's Bibite Stats panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BibiteStatsView {
    /// `BibiteGenes.generation`.
    pub generation: u32,
    /// `BibiteBody.bodyLength`.
    pub body_length: f32,
    /// `BibiteBody.realBodyArea`.
    pub real_body_area: f32,
    /// Current rigid-body mass including organs.
    pub mass: f32,
    /// `BibitePropulsion.SignedVelocity`.
    pub speed: f32,
    /// `InternalClock.timeAlive` (seconds).
    pub time_alive: f32,
    /// `BibitePropulsion.totalTravel`.
    pub travelled: f32,
    /// `BibiteEggLayingOrgan.nEggsLaid`.
    pub eggs_laid: u32,
    /// Live children in the egg layer's child list.
    pub alive_children: u32,
    /// `BibiteMouth.bibitesBitten` (first bites).
    pub bibites_bitten: u32,
    /// `BibiteMouth.totalDamageDealt`.
    pub damage_dealt: f32,
    /// `BibiteBody.timesAttacked`.
    pub times_attacked: u32,
    /// `BibiteBody.totalDamageSuffered`.
    pub damage_received: f32,
    /// Species tag (`BibiteGenes.speciesTag`).
    pub species_tag: Option<String>,
    /// Species id (`BibiteGenes.species`).
    pub species: Option<crate::ids::SpeciesId>,
    /// First parent id.
    pub parent1: Option<BibiteId>,
    /// Second parent id.
    pub parent2: Option<BibiteId>,
    /// Easter-egg radiation flag.
    pub is_rad: bool,
    /// Easter-egg rainbow flag.
    pub is_rain: bool,
    /// Easter-egg pellet-source flag.
    pub is_source: bool,
    /// Easter-egg reaper flag.
    pub is_killer: bool,
    /// `BibiteBody.dead`.
    pub dead: bool,
    /// Corpse record retained for revive/meat.
    pub is_corpse: bool,
}

/// UI-facing snapshot of an incubating egg's Stats panel fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EggStatsView {
    /// `BibiteGenes.generation`.
    pub generation: u32,
    /// Display egg area (`5·sizeGene²`).
    pub egg_area: f32,
    /// Egg mass (`areaAtBirth · bibiteMassDensity`).
    pub egg_mass: f32,
    /// Stored egg energy (`EggHatching.energy`).
    pub energy: f32,
    /// Incubation progress (seconds).
    pub hatch_progress: f32,
    /// Remaining free energy (`freeEnergy`).
    pub free_energy: f32,
    /// Required incubation time (seconds).
    pub hatch_time: f32,
    /// Energy the newborn's body and brain need (`totalRequiredEnergy`); the egg's health bar is the share of it
    /// already spent (`HatchRatio() * totalRequiredEnergy`).
    pub total_required_energy: f32,
}

/// Cheap read-only copy of the fields sensing/tools need
/// (`docs/INTERFACES.md` §5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BibiteView {
    /// Typed id.
    pub id: BibiteId,
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Rotation (radians).
    pub rot: f32,
    /// Angular velocity (rad/s).
    pub ang_vel: f32,
    /// 1-D size.
    pub d1_size: f32,
    /// 2-D size.
    pub d2_size: f32,
    /// Base body area.
    pub base_body_area: f32,
    /// Maximum health.
    pub max_health: f32,
    /// Health / max health.
    pub health_ratio: f32,
    /// Dead flag.
    pub dead: bool,
    /// Corpse flag.
    pub is_corpse: bool,
    /// Genome/brain ready.
    pub is_ready: bool,
    /// Body colour genes `(R, G, B)`.
    pub color: [f32; 3],
    /// Eye hue offset gene.
    pub eye_offset: f32,
    /// Easter-egg rainbow rendering flag (`GenomeMeta.is_rain`).
    pub is_rain: bool,
}

impl BibiteView {
    /// Forward unit vector (`transform.up`, see [`crate::math::unity::transform_up`]).
    #[must_use]
    pub fn up(&self) -> Vec2 {
        crate::math::unity::transform_up(self.rot)
    }

    /// Alias of [`BibiteView::up`] for sensing code that calls it "forward".
    #[must_use]
    pub fn forward(&self) -> Vec2 {
        self.up()
    }

    /// FOV bounding radius of the body (`BibiteBody.sideRadius = 4.5 * d1Size`).
    #[must_use]
    pub fn side_radius(&self) -> f32 {
        4.5 * self.d1_size
    }
}

/// Serialized stomach content.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StomachContentSave {
    /// Material: `0` = Plant, `1` = Meat.
    pub material: u8,
    /// Amount (u²).
    pub amount: f32,
    /// Average chunk amount (u²).
    pub average_chunk_amount: f32,
}

/// Serialized body/organ/clock state (spec 02 §9). io fills this from JSON and
/// converts back through [`BibiteStore::spawn_loaded`].
#[derive(Clone, Debug, PartialEq)]
pub struct BibiteSave {
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Rotation in game transform degrees (Unity `transform.rotation.z`,
    /// serialized as degrees). [`BibiteStore::spawn_loaded`] converts to the
    /// record's radians; [`BibiteStore::to_save`] converts back.
    pub rot: f32,
    /// Angular velocity.
    pub ang_vel: f32,
    /// Genome (`genes` object).
    pub genome: Genome,
    /// Genome metadata (`genes` fields).
    pub meta: GenomeMeta,
    /// Brain (`brain` object).
    pub brain: Brain,
    /// Active energy (E).
    pub energy: f32,
    /// Health (HP).
    pub health: f32,
    /// `d2Size`.
    pub d2_size: f32,
    /// `dying`.
    pub dying: bool,
    /// `dead`.
    pub dead: bool,
    /// `attackedDmg`.
    pub attacked_dmg: f32,
    /// `timesAttacked`.
    pub times_attacked: u32,
    /// `totalDamageSuffered`.
    pub total_damage_suffered: f32,
    /// `brainTicksCount`.
    pub brain_ticks: f32,
    /// `visionLookupCount`.
    pub vision_lookup: f32,
    /// `visionSensingCount`.
    pub vision_sensing: f32,
    /// `corpseEnergyOffset`.
    pub corpse_energy_offset: f32,
    /// Growth (`growth` value; derived from `d2Size / SizeRatio²` on load).
    pub growth: f32,
    /// Stomach slots (`stomach.content`, at most 2 entries).
    pub stomach: [StomachContentSave; STOMACH_SLOTS],
    /// Occupied stomach slots.
    pub n_stomach: u8,
    /// Mouth `biteProgress`.
    pub bite_progress: f32,
    /// Mouth `attackedLastFrame`.
    pub attacked_last_frame: bool,
    /// Mouth `bibitesBitten`.
    pub bibites_bitten: u32,
    /// Mouth `totalDamageDealt`.
    pub total_damage_dealt: f32,
    /// Mouth `totalMurders`.
    pub total_murders: u32,
    /// Mouth `murderedArea`.
    pub murdered_area: f32,
    /// Egg-layer `eggProgress`.
    pub egg_progress: f32,
    /// Egg-layer `nEggsLaid`.
    pub n_eggs_laid: u32,
    /// Egg-layer `children` (id list).
    pub children: Vec<BibiteId>,
    /// Fat `fatReservesAmount` (u²).
    pub fat_amount: f32,
    /// Pheromone organ `progress`.
    pub phero_progress: f32,
    /// Clock `tic`.
    pub tic: u8,
    /// Clock `ticProgress`.
    pub tic_progress: f32,
    /// Clock `timeAlive`.
    pub time_alive: f32,
    /// Clock `chronoTime`.
    pub chrono_time: f32,
    /// Propulsion `totalTravel`.
    pub total_travel: f32,
}

/// Cached FOV seen-entity ids from the last `FindSeenEntities` pass.
///
/// The game refreshes this list only on the vision-lookup cadence and then runs
/// `ComputeSenses` from it on the faster sense cadence; after a save load the
/// list starts empty. Positions/radii are resolved live by the world when
/// senses are computed, so only ids are stored here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FovSeenCache {
    /// Seen plant/meat pellets in deterministic query order.
    pub pellets: Vec<PelletId>,
    /// Seen bibites in deterministic query order.
    pub bibites: Vec<BibiteId>,
}

/// One live bibite record. Field groups mirror the game components; the B2b
/// tick operates on these fields.
/// Runtime-only feeding/energy counters (never serialized into saves or
/// included in state fingerprints). Units: `u2` = pellet/stomach matter,
/// `e` = energy, counts are cumulative event counts. See
/// `docs/design/FEEDING_METRICS.md` §2.1/§2.2/§2.3.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct FeedingCounters {
    // ---- Counters that only a bibite with something in its mouth touches come first, so that the 48 bytes the turn of every
    // bibite writes every tick are the last of the struct, next to the organs that change every tick (see `BibiteRecord`).
    /// Ticks with at least one object inside the mouth trigger.
    pub mouth_contact_ticks: u64,
    /// Ticks with at least one plant pellet inside the mouth trigger.
    pub plant_contact_ticks: u64,
    /// Transitions from no mouth target to at least one.
    pub mouth_contact_enter_events: u64,
    /// Successful plant-pellet chunk bites / whole-swallow-neutral bites.
    pub plant_bite_events: u64,
    /// Successful meat-pellet chunk bites.
    pub meat_bite_events: u64,
    /// First bites on another bibite (game `bibitesBitten`).
    pub bibite_bite_events: u64,
    /// Whole plant pellets swallowed.
    pub plant_swallow_events: u64,
    /// Whole meat pellets swallowed.
    pub meat_swallow_events: u64,
    /// Whole bibites swallowed.
    pub bibite_swallow_events: u64,
    /// Plant matter transferred to the stomach.
    pub plant_consumed_u2: f64,
    /// Meat matter transferred to the stomach.
    pub meat_consumed_u2: f64,
    /// Meat+stomach matter gained by swallowing whole bibites.
    pub bibite_consumed_u2: f64,
    /// Plant matter ripped into a new ground pellet.
    pub plant_ripped_u2: f64,
    /// Meat matter ripped into a new ground pellet.
    pub meat_ripped_u2: f64,
    /// Sum of bite damage dealt to bibites.
    pub bibite_damage_dealt: f64,
    /// Bibites killed by bites.
    pub bibite_kills: u64,
    // ---- Written by the turn of every bibite, every tick or whenever it digests.
    /// Latched previous-tick contact state (diagnostics only).
    pub contact_latched: bool,
    /// Energy accepted through `BodyState::gain_energy`.
    pub energy_gained_e: f64,
    /// Matter dissolved by digestion.
    pub stomach_digested_u2: f64,
    /// Energy gained from digestion (after conversion efficiency).
    pub stomach_energy_gain_e: f64,
    /// Energy removed through `BodyState::use_energy`.
    pub energy_spent_e: f64,
    /// Living bibite-time.
    pub live_seconds: f64,
}

/// `RadioZone.effect.probabilityIncrease`.
pub const RAD_ZONE_PROBABILITY: f32 = 10.0;
/// `RadioZone.effect.intensityIncrease`.
pub const RAD_ZONE_INTENSITY: f32 = 0.35;
/// `RadioZone` prefab trigger radius: `CircleCollider2D.radius` 0.5 x transform
/// scale 500 (`sharedassets2.assets`, GameObject "RadioZone").
pub const RAD_ZONE_RADIUS: f32 = 250.0;

/// What the scans over the records ask of a slot, kept in a dense array of the store so that a scan reads eight bytes a slot
/// instead of two or three lines of a record (see [`BibiteStore::refresh_summaries`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotSummary {
    /// The species of a bibite that is not dead (meaningful with [`SlotSummary::HAS_SPECIES`]).
    pub(crate) species: u32,
    pub(crate) bits: u8,
}

impl SlotSummary {
    /// The slot holds a record whose body is not dead.
    pub(crate) const LIVE: u8 = 1;
    /// Live and `meta.is_rad`.
    pub(crate) const RAD: u8 = 2;
    /// Live and `is_ready`: the first test of the sensing prefilter.
    pub(crate) const SENSIBLE: u8 = 4;
    /// The record's brain asks for pheromone sensing (`Brain::needs_pheromone_sensing`).
    pub(crate) const PHERO: u8 = 8;
    /// Live and the bibite has a species.
    pub(crate) const HAS_SPECIES: u8 = 16;

    /// The summary of a record.
    pub(crate) fn of(record: &BibiteRecord) -> Self {
        let mut bits = 0;
        let mut species = 0;
        if !record.body.dead {
            bits |= Self::LIVE;
            if record.meta.is_rad {
                bits |= Self::RAD;
            }
            if record.is_ready {
                bits |= Self::SENSIBLE;
            }
            if let Some(id) = record.meta.species {
                bits |= Self::HAS_SPECIES;
                species = id.value();
            }
        }
        if record.brain.needs_pheromone_sensing() {
            bits |= Self::PHERO;
        }
        Self { species, bits }
    }
}

/// The marker that makes `Option<BibiteRecord>` keep "this slot is empty" in a hot line of the record. A single-variant enum
/// with a `u64` representation has every other value of its eight bytes as a niche, more than any `Vec` capacity has, so rustc
/// stores `None` here: without it the niche was the capacity of the last `Vec` field, on the coldest line of the record, and
/// every sweep and scan that asked whether a slot is empty pulled that line through memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum SlotMarker {
    /// The only value: the record is a live record.
    Occupied = 0,
}

/// One live bibite record. Field groups mirror the game components.
///
/// The fields are laid out in the order they are declared (`repr(C)`, aligned to a cache line so that a record's lines are its
/// own), hot ones first, because the passes over the records
/// (the organs' turns, the physics bridge's build and sweep, the scans) are limited by the cache lines they pull through the
/// cores, not by arithmetic, and the compiler's own order scattered the fields of one pass over twenty lines of a 33-line
/// record. Lines 0-1 are what the physics bridge reads and writes for every bibite every tick (the sweep after the step of a
/// bibite with no contacts touches nothing else), lines 2-3 what the build of the physics body also reads, then come the
/// organs and the body, the lists of contacts and targets (touched when there are any), and last what nothing touches per
/// tick. `print_the_layout_of_a_bibite_record` prints the map.
#[derive(Clone, Debug)]
#[repr(C, align(64))]
pub struct BibiteRecord {
    // ---- Line 0: the pose and the rigid body the physics body is made of.
    /// Derived mass/inertia/drag/size/brain-upkeep cache.
    pub derived: BibiteDerived,
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Rotation (radians).
    pub rot: f32,
    /// Angular velocity (rad/s).
    pub ang_vel: f32,
    /// The epoch of the body tick whose turn left `derived` (mass, inertia, drag, sizes) as the physics body and the sweep after
    /// the step want it; `0`, or an older epoch, when anything may have touched the record since (see
    /// [`BibiteRecord::refresh_physics_cache`]).
    pub(crate) phys_stamp: u32,
    // ---- Line 1: the forces, the id, the last step's contact counters and the counts of the lists below.
    /// Pending force accumulated by the B2b movement organ.
    pub pending_force: Vec2,
    /// Makes `None` of an `Option<BibiteRecord>` live here (see [`SlotMarker`]).
    pub(crate) slot_marker: SlotMarker,
    /// Pending torque accumulated by the B2b movement organ.
    pub pending_torque: f32,
    /// Typed id.
    pub id: BibiteId,
    /// Solid contacts touching this bibite in the last physics step (debug trace).
    pub last_contact_count: u16,
    /// Solid contacts with another bibite in the last physics step.
    pub last_contact_bibite: u16,
    /// Solid contacts with a pellet in the last physics step.
    pub last_contact_pellet: u16,
    /// Solid contacts with an egg in the last physics step.
    pub last_contact_egg: u16,
    /// Solid contacts on Body/Tail/Mouth colliders this step.
    pub last_contact_collider: [u16; 3],
    /// New solid contacts (Unity OnCollisionEnter2D analogue) this step.
    pub last_contact_enter_count: u16,
    /// Number of valid `prev_contact_tags`.
    pub n_prev_contact_tags: u8,
    /// Number of valid mouth target tags.
    pub n_mouth_targets: u8,
    /// Number of held tags.
    pub n_held_targets: u8,
    /// Ready flag.
    pub is_ready: bool,
    /// Born flag.
    pub born: bool,
    /// Corpse flag.
    pub is_corpse: bool,
    /// Touched an `isRad` bibite's `RadioZone` (`AddMutationFactor(.., "RadZone")`,
    /// keyed so it applies once). Not part of checkpoints: re-detected on contact.
    pub rad_zone: bool,
    /// Death cause once [`BibiteStore::kill`] was called (`None` while alive).
    pub death_cause: Option<DeathCause>,
    /// Sense throttle counters (the sensing prefilter reads them with the flags above).
    pub throttle: SenseThrottle,
    /// Which set of colliders `colliders` holds: a new number from [`next_collider_generation`] each time they are made again. A
    /// physics body that has this number has these colliders (see `step_physics_inner`).
    pub(crate) colliders_gen: u32,
    /// Per-target missed-physics-tick ages for mouth contact grace retention.
    pub mouth_target_ages: [u8; 10],
    // ---- Lines 2-3: what the physics body is built from besides the pose.
    /// Cached local collider shapes (scaled by the current `d1Size`), refreshed
    /// when the body size changes; hot path for the physics bridge mirror.
    pub colliders: [phys::Collider; 3],
    /// Eye hue offset. Cold; it fills the four bytes up to the line boundary where the run of the organs' turn starts.
    pub eye_offset: f32,
    // ---- The organs' turn: everything that the turn of a bibite touches every tick lies in one run of 640 bytes, from the
    // fourth to the last of `feeding`'s lines (line 6) to line 15, ten lines in all, instead of the twenty-two lines the organs
    // were spread over. What the turn writes every tick comes first (the last 48 bytes of `feeding`, the clock, the propulsion,
    // the pheromone emitter, the growth, the womb, the fat, and the start of the body), so that it dirties five lines, not ten.
    /// Runtime-only feeding/energy counters (the first 128 bytes are counters that only a bibite with something in its mouth
    /// touches; they come before the run).
    pub feeding: FeedingCounters,
    /// Clock.
    pub clock: InternalClock,
    /// Propulsion kinematics.
    pub propulsion: PropulsionState,
    /// Pheromone emitter.
    pub pheromone: PheromoneOrgan,
    /// Growth/maturity state.
    pub growth: GrowthState,
    /// Womb.
    pub egg_layer: EggLayer,
    /// Fat organ.
    pub fat: FatOrgan,
    /// Body state.
    pub body: BodyState,
    /// Stomach.
    pub stomach: Stomach,
    /// Mouth state.
    pub mouth: MouthState,
    /// Propulsion-derived cache.
    pub propulsion_derived: PropulsionDerived,
    /// Mouth-derived cache.
    pub mouth_derived: MouthDerived,
    /// Growth gene cache.
    pub growth_genes: GrowthGenes,
    /// Steering inputs written by the world FOV/zone phase.
    pub steering: SteeringInputs,
    // ---- Touched on the ticks a bibite senses or thinks, or by an event.
    /// World senses (written by the world sensing phase in B2b).
    pub senses: SenseOutputs,
    /// Armor organ.
    pub armor: ArmorOrgan,
    // ---- The lists of contacts and targets: touched by the bibites that have any.
    /// Other-body tags active in the previous step (enter-event detection).
    pub prev_contact_tags: [u64; 8],
    /// Tags currently inside the mouth trigger (updated by the physics bridge).
    pub mouth_targets: [u64; 10],
    /// Tags currently held by the mouth.
    pub held_targets: [u64; 10],
    /// Sum of |normal impulse| of the contacts counted in `last_contact_count` (debug trace; zero when none was counted).
    pub last_contact_impulse: f32,
    /// Sum of |normal impulse| of new solid contacts this step (zero when none was counted).
    pub last_contact_enter_impulse: f32,
    /// Body colour genes.
    pub color: [f32; 3],
    /// Accumulated environmental mutation pressure.
    pub mutation_factor: MutationFactor,
    /// Brain.
    pub brain: Brain,
    // ---- What nothing reads every tick.
    /// Genome.
    pub genome: Genome,
    /// Metadata.
    pub meta: GenomeMeta,
    /// Cached FOV seen ids (vision-lookup cadence; empty after load).
    pub fov_seen: FovSeenCache,
    /// Egg-layer children ids (serialized by the game world save).
    pub children: Vec<BibiteId>,
}

impl BibiteRecord {
    /// Mutation pressure including the permanent `RadZone` factor
    /// (`RadioZone.effect`: +10 count, +0.35 intensity) once touched.
    #[must_use]
    pub fn effective_mutation_factor(&self) -> MutationFactor {
        let mut factor = self.mutation_factor;
        if self.rad_zone {
            factor.probability_increase += RAD_ZONE_PROBABILITY;
            factor.intensity_increase += RAD_ZONE_INTENSITY;
        }
        factor
    }

    /// Recomputes every derived cache from the body/organs and settings.
    pub fn recompute_derived(&mut self, settings: &SimSettings) {
        let bs = BodySettings::from_sim_settings(settings);
        let genome = self.genome.clone();
        let area = self.body.geometry.base_body_area;
        let metabolic_rate = genome.metabolic_rate();
        self.growth_genes = growth_genes_of(&genome);
        self.stomach
            .set_area(area * genome.organ_area_portion(Gene::StomachWAG));
        self.fat
            .on_growth(genome.organ_area_portion(Gene::FatWAG), area);
        self.armor.update(
            genome.organ_area_portion(Gene::ArmorWAG),
            self.body.geometry.d1_size,
            self.body.geometry.inflate_factor,
            settings.materials.armor.cohesiveness,
        );
        self.body.set_armor_thickness(self.armor.thickness);
        self.egg_layer
            .set_area(area * genome.organ_area_portion(Gene::WombWAG));
        self.mouth_derived = mouth_derived(
            area,
            genome.organ_area_portion(Gene::MouthMusclesWAG),
            area * genome.organ_area_portion(Gene::ThroatWAG),
            metabolic_rate,
            &mouth_settings(settings),
        );
        self.propulsion_derived = propulsion_derived(
            area * genome.organ_area_portion(Gene::MoveMusclesWAG),
            metabolic_rate,
            &propulsion_settings(settings),
        );
        self.derived.brain_upkeep = brain_upkeep_per_second(&self.brain, settings);
        self.egg_layer.egg_energy = egg_energy_investment(
            genome.lay_body_energy(settings),
            self.derived.brain_upkeep,
            genome.gene(Gene::SizeRatio),
        );
        self.color = [
            genome.gene(Gene::ColorR),
            genome.gene(Gene::ColorG),
            genome.gene(Gene::ColorB),
        ];
        self.eye_offset = genome.gene(Gene::EyeOffset);
        let organ_masses = OrganMasses {
            stomach: self.stomach.mass(),
            egg_layer: settings.scenario.bibite_mass_density
                * self.egg_layer.egg_size
                * self.egg_layer.progress,
            fat: self.fat.mass(&organ_material(&settings.materials.fat)),
            armor: self.armor.mass(
                area * self.armor.portion,
                &organ_material(&settings.materials.armor),
            ),
        };
        let rb = self.body.rigid_body(organ_masses, &bs);
        self.derived.mass = rb.mass;
        self.derived.inertia = rb.inertia;
        self.derived.linear_damping = rb.linear_damping;
        self.derived.angular_damping = rb.angular_damping;
        self.derived.d1_size = self.body.geometry.d1_size;
        self.derived.d2_size = self.body.geometry.d2_size;
        self.derived.base_body_area = area;
        self.derived.body_length = self.body.geometry.body_length;
        self.refresh_colliders();
    }

    /// Refreshes the derived rigid-body cache from the current geometry and
    /// organ masses. The game overwrites `rb2d.mass/inertia/damping` every
    /// FixedUpdate; keeping this in sync is required for collision damage,
    /// bite knockback, stats and sensing after growth/fat/stomach changes.
    pub fn update_derived_rigid_body(&mut self, settings: &SimSettings) {
        let bs = BodySettings::from_sim_settings(settings);
        let area = self.body.geometry.base_body_area;
        let organ_masses = OrganMasses {
            stomach: self.stomach.mass(),
            egg_layer: settings.scenario.bibite_mass_density
                * self.egg_layer.egg_size
                * self.egg_layer.progress,
            fat: self.fat.mass(&organ_material(&settings.materials.fat)),
            armor: self.armor.mass(
                area * self.armor.portion,
                &organ_material(&settings.materials.armor),
            ),
        };
        let rb = self.body.rigid_body(organ_masses, &bs);
        self.derived.mass = rb.mass;
        self.derived.inertia = rb.inertia;
        self.derived.linear_damping = rb.linear_damping;
        self.derived.angular_damping = rb.angular_damping;
        self.derived.d1_size = self.body.geometry.d1_size;
        self.derived.d2_size = self.body.geometry.d2_size;
        self.derived.base_body_area = area;
        self.derived.body_length = self.body.geometry.body_length;
    }

    /// Cheap geometry-derived cache sync used after growth; the full
    /// mass/inertia/damping cache is refreshed once per tick in the physics
    /// bridge (where the rigid body is already computed), avoiding a duplicate
    /// per-record rigid-body build in the body phase.
    fn sync_derived_geometry(&mut self) {
        self.derived.d1_size = self.body.geometry.d1_size;
        self.derived.d2_size = self.body.geometry.d2_size;
        self.derived.base_body_area = self.body.geometry.base_body_area;
        self.derived.body_length = self.body.geometry.body_length;
    }

    /// The rigid body of the record from its organs and its body, the way the physics body is made of it.
    fn rigid_body_now(
        &self,
        settings: &SimSettings,
        bs: &BodySettings,
        fat: &MaterialParams,
        armor: &MaterialParams,
    ) -> RigidBodyProperties {
        let organ_masses = OrganMasses {
            stomach: self.stomach.mass(),
            egg_layer: settings.scenario.bibite_mass_density
                * self.egg_layer.egg_size
                * self.egg_layer.progress,
            fat: self.fat.mass(fat),
            armor: self.armor.mass(
                self.body.geometry.base_body_area * self.armor.portion,
                armor,
            ),
        };
        self.body.rigid_body(organ_masses, bs)
    }

    /// Leaves in `derived` what the physics body will be built from and what the sweep after the step would write there (the
    /// rigid body, as the body gets it, and the copies of the geometry), and stamps the record with the epoch. It runs at the
    /// end of the bibite's turn, with the organs in the cache, so that the build does not have to pull nine lines of every
    /// record through memory again to do the same arithmetic.
    ///
    /// The stamp is only believed while nothing could have changed the record since: every mutable access that is not a turn
    /// ([`BibiteStore::get_mut`], [`BibiteStore::for_each_mut`], a change of settings) clears it, and the next epoch needs a
    /// new turn. A record without a current stamp is handled the old way.
    fn refresh_physics_cache(&mut self, env: &OrganEnv<'_>) {
        let rb = self.rigid_body_now(env.settings, env.bs, env.fat_params, env.armor_params);
        self.derived.mass = rb.mass.max(0.000_1);
        self.derived.inertia = rb.inertia.max(0.000_1);
        self.derived.linear_damping = rb.linear_damping;
        self.derived.angular_damping = rb.angular_damping;
        self.sync_derived_geometry();
        self.phys_stamp = env.epoch;
    }

    /// Revives a dead corpse (`BibiteBody.Revive`), restoring max health/energy
    /// and clearing corpse/decay state. Returns `false` for a live bibite.
    pub fn revive(&mut self) -> bool {
        if !self.body.dead && !self.is_corpse {
            return false;
        }
        self.body.dead = false;
        self.body.dying = false;
        self.is_corpse = false;
        self.death_cause = None;
        self.body.corpse_energy_offset = 0.0;
        // `Revival` (a coroutine every 0.1 s): while `corpseDecay > 0` health is `maxHealth * (1 - decay / 1.1)`, energy is full and the decay falls by 1/150;
        // `revival_step` does it (the first step now, the others from the tick).
        self.revival_step();
        self.n_mouth_targets = 0;
        self.mouth.n_in_mouth = 0;
        self.n_held_targets = 0;
        true
    }

    /// One iteration of `BibiteBody.Revival`: a live bibite whose `corpseDecay` is still above 0 is being revived.
    pub fn revival_step(&mut self) {
        let decay = self.body.corpse_decay;
        if decay > 0.0 {
            self.body.health.amount = self.body.health.max_amount * (1.0 - decay / 1.1);
            self.body.energy.amount = self.body.energy.max_amount;
            self.body.corpse_decay = (decay - 1.0 / 150.0).max(0.0);
        } else {
            self.body.health.amount = self.body.health.max_amount;
            self.body.energy.amount = self.body.energy.max_amount;
        }
    }

    /// `Physics2D.OverlapCircleNonAlloc`: whether a circle touches any collider of the bibite (body, tail, mouth box, ...), triggers included.
    #[must_use]
    pub fn touches_circle(&self, center: Vec2, radius: f32) -> bool {
        let pose = phys::Pose::new(self.pos, self.rot);
        let circle = phys::Shape::circle(radius.max(1e-4));
        let at = phys::Pose::new(center, 0.0);
        self.colliders().iter().any(|collider| phys::collide(&circle, at, &collider.shape, collider.world_pose(pose)).is_some())
    }

    /// Read-only snapshot of the game's Bibite Stats panel fields.
    #[must_use]
    pub fn stats_view(&self) -> BibiteStatsView {
        BibiteStatsView {
            generation: self.meta.generation,
            body_length: self.body.geometry.body_length,
            real_body_area: self.body.geometry.real_body_area,
            mass: self.derived.mass,
            speed: self.propulsion.signed_velocity,
            time_alive: self.clock.time_alive,
            travelled: self.propulsion.total_travel,
            eggs_laid: self.egg_layer.n_laid,
            alive_children: self.children.len() as u32,
            bibites_bitten: self.mouth.bibites_bitten,
            damage_dealt: self.mouth.total_damage_dealt,
            times_attacked: self.body.times_attacked,
            damage_received: self.body.total_damage_suffered,
            species_tag: self.meta.species_tag.clone(),
            species: self.meta.species,
            parent1: self.meta.parent1,
            parent2: self.meta.parent2,
            is_rad: self.meta.is_rad,
            is_rain: self.meta.is_rain,
            is_source: self.meta.is_source,
            is_killer: self.meta.is_killer,
            dead: self.body.dead,
            is_corpse: self.is_corpse,
        }
    }

    /// Current rigid-body mass from live organ masses, computed on demand for
    /// the rare bite-knockback path (not cached every tick).
    fn current_mass(&self, settings: &SimSettings) -> f32 {
        let bs = BodySettings::from_sim_settings(settings);
        let organ_masses = OrganMasses {
            stomach: self.stomach.mass(),
            egg_layer: settings.scenario.bibite_mass_density
                * self.egg_layer.egg_size
                * self.egg_layer.progress,
            fat: self.fat.mass(&organ_material(&settings.materials.fat)),
            armor: self.armor.mass(
                self.body.geometry.base_body_area * self.armor.portion,
                &organ_material(&settings.materials.armor),
            ),
        };
        self.body.rigid_body(organ_masses, &bs).mass
    }

    /// Collider shape descriptors for the physics hookup (B2b): main-body
    /// capsule, tail capsule (the game's `BibiteBody.bodyCollider`, which the
    /// inflate code overwrites) and the mouth trigger box. Sizes/offsets are
    /// already scaled by `d1Size` and cached until the size changes.
    #[must_use]
    pub fn colliders(&self) -> [phys::Collider; 3] {
        self.colliders
    }

    /// Recomputes the cached collider shapes from the current `d1Size`.
    pub fn refresh_colliders(&mut self) {
        let d1 = self.body.geometry.d1_size;
        self.colliders = [
            phys::Collider::solid(phys::Shape::capsule_from_size(
                phys::prefab::BODY_CAPSULE_SIZE * d1,
            ))
            .with_offset(phys::prefab::body_capsule_offset(d1))
            .with_rotation(phys::prefab::PART_ROTATION)
            .with_material(
                phys::prefab::CORPS_FRICTION,
                phys::prefab::CORPS_RESTITUTION,
            ),
            phys::Collider::solid(phys::Shape::capsule_from_size(
                phys::prefab::tail_capsule_size(self.body.geometry.inflate_factor) * d1,
            ))
            .with_offset(phys::prefab::tail_capsule_offset(self.body.geometry.inflate_factor) * d1)
            .with_material(
                phys::prefab::CORPS_FRICTION,
                phys::prefab::CORPS_RESTITUTION,
            ),
            phys::Collider::sensor(phys::Shape::box_from_size(
                phys::prefab::mouth_box_size(self.genome.gene(Gene::Diet)) * d1,
            ))
            .with_offset(phys::prefab::mouth_offset(d1))
            .with_rotation(phys::prefab::PART_ROTATION),
        ];
        self.colliders_gen = next_collider_generation();
    }
}

impl BibiteRecord {
    /// `BibiteBody.EnergyFlowRate` (E/s): what the Biology panel's Flow bar shows. A living bibite gains what the stomach and the fat give and pays
    /// the fat's upkeep, growth, healing, metabolism, movement, the brain and the egg being made (all through the metabolic efficiency); a corpse only pays the fat
    /// upkeep and its metabolism.
    #[must_use]
    pub fn energy_flow_rate(&self) -> f32 {
        let efficiency = self.body.params.metabolic_efficiency;
        if self.body.dead {
            return -(self.fat.last_energy_sustain_rate + self.body.metabolism) / efficiency;
        }
        self.stomach.energy_gain_rate() + self.fat.last_energy_gain_rate
            - (self.fat.last_energy_sustain_rate
                + self.growth.growth_cost_rate
                + self.body.healing_rate_energy_cost
                + self.body.metabolism
                + self.body.movement_cost
                + self.derived.brain_upkeep
                + self.egg_layer.energy_consumption)
                / efficiency
    }
}

impl InternalSenseSource for BibiteRecord {
    fn energy_ratio(&self) -> f32 {
        self.body.energy_ratio()
    }
    fn maturity(&self) -> f32 {
        // Game brain input `Maturity` is the **raw** growth value
        // (`NEATBrain.cs:577` `Nodes[1].Value = growth.growth`), not the
        // growth/growthAtMature ratio used for display.
        self.growth.growth
    }
    fn life_ratio(&self) -> f32 {
        self.body.health_ratio()
    }
    fn fullness(&self) -> f32 {
        self.stomach.fullness()
    }
    fn speed(&self) -> f32 {
        self.propulsion.normalized_velocity
    }
    fn rotation_speed(&self) -> f32 {
        self.propulsion.angular_velocity
    }
    fn is_grabbing(&self) -> f32 {
        // `NEATBrain.cs:582`: `Nodes[6].Value = body.mouth.nHeld`.
        f32::from(self.n_held_targets)
    }
    fn attacked_damage(&self) -> f32 {
        attacked_dmg_brain_input(self.body.attacked_dmg, self.body.health.amount)
    }
    fn egg_stored(&self) -> f32 {
        self.egg_layer.n_eggs() as f32
    }
    fn tic(&self) -> f32 {
        f32::from(self.clock.tic)
    }
    fn minute(&self) -> f32 {
        self.clock.chrono_time / 60.0
    }
    fn time_alive(&self) -> f32 {
        self.clock.time_alive / 60.0
    }
}

/// The bibite store: a generational slot vector with a dense iteration list.
///
/// Storage is one [`BibiteRecord`] per slot (record-per-slot rather than fully
/// split SoA): the fields of a record are contiguous for the B2b tick, and the
/// store still provides dense, deterministic iteration and `BibiteId`
/// generation semantics identical to [`crate::arena::Arena`].
/// Slot-aligned persistent+runtime snapshot of a [`BibiteStore`] for
/// checkpoint/restore (world checkpoint extension 0x0101/0x0102).
#[derive(Clone, Debug, PartialEq)]
pub struct BibiteCheckpointArena {
    /// Slot → generation.
    pub generations: Vec<u32>,
    /// Free-list order.
    pub free: Vec<u32>,
    /// Dense slot order.
    pub dense: Vec<u32>,
    /// Slot-aligned records (`None` = free slot).
    pub records: Vec<Option<BibiteCheckpointRecord>>,
}

/// One slot's persistent save + runtime-only body state.
#[derive(Clone, Debug, PartialEq)]
pub struct BibiteCheckpointRecord {
    /// Persistent body/genome/brain state.
    pub save: BibiteSave,
    /// Raw internal rotation in radians (save stores lossy degrees).
    pub rot_radians: f32,
    /// Sense throttle counters.
    pub throttle: SenseThrottle,
    /// World senses.
    pub senses: SenseOutputs,
    /// Steering inputs.
    pub steering: SteeringInputs,
    /// Cached FOV seen ids.
    pub fov_seen: FovSeenCache,
    /// Pending force for the next physics step.
    pub pending_force: Vec2,
    /// Pending torque for the next physics step.
    pub pending_torque: f32,
    /// Mouth target tags.
    pub mouth_targets: [u64; 10],
    /// Number of mouth targets.
    pub n_mouth_targets: u8,
    /// Per-target missed-physics-tick ages for mouth contact grace retention.
    pub mouth_target_ages: [u8; 10],
    /// Held target tags.
    pub held_targets: [u64; 10],
    /// Number of held targets.
    pub n_held_targets: u8,
    /// Pending death cause.
    pub death_cause: Option<DeathCause>,
    /// Corpse flag.
    pub is_corpse: bool,
    /// Born flag.
    pub born: bool,
    /// Ready flag.
    pub is_ready: bool,
    /// Accumulated mutation pressure.
    pub mutation_factor: MutationFactor,
    /// Brain node innovation high-water mark.
    pub brain_node_max_innovation: i64,
    /// Brain connection innovation high-water mark.
    pub brain_connection_max_innovation: i64,
    /// Live body geometry (inflate/armor/max pools), which a save/load
    /// recomputation is not guaranteed to reproduce bit-identically.
    pub body_geometry: BodyGeometry,
    /// Live body parameters (metabolic efficiency etc).
    pub body_params: BodyParams,
    /// Live base metabolism (E/s).
    pub body_metabolism: f32,
    /// Live derived mass/inertia/drag/size cache.
    pub derived: BibiteDerived,
    /// Live cached collider shapes (physics mirror input).
    pub colliders: [phys::Collider; 3],
    /// Live propulsion-derived movement constants.
    pub propulsion_derived: organs::PropulsionDerived,
    /// Live mouth-derived jaw/cooldown constants.
    pub mouth_derived: organs::MouthDerived,
    /// Live propulsion kinematics state.
    pub propulsion: organs::PropulsionState,
    /// Live growth timer/maturity state.
    pub growth: GrowthState,
    /// Live clock accumulators.
    pub clock: organs::InternalClock,
    /// Live fat organ caches.
    pub fat: organs::FatOrgan,
    /// Live armor organ thickness/strength.
    pub armor: organs::ArmorOrgan,
    /// Live womb organ caches.
    pub egg_layer: organs::EggLayer,
    /// Live stomach area and last-tick caches (contents come from `save`).
    /// Live stomach organ area/capacity.
    pub stomach_area: f32,
    /// Live stomach total content cache.
    pub stomach_total_content: f32,
    /// Live stomach acid level cache.
    pub stomach_acid_level: f32,
    /// Live stomach acid/fullness ratio cache.
    pub stomach_acid_ratio: f32,
    /// Live stomach digestion effectiveness cache.
    pub stomach_digestion_effectiveness: f32,
    /// Live stomach digestion potential bonus cache.
    pub stomach_digestion_potential_bonus: f32,
    /// Live stomach efficiency malus cache.
    pub stomach_efficiency_malus: f32,
    /// Exact runtime stomach slots (amounts, mean chunk, digestion caches).
    pub stomach_contents: Vec<StomachContent>,
    /// Live pheromone accumulator.
    pub pheromone_progress: f32,
    /// Live mouth state (desire/opening/counters).
    pub mouth: organs::MouthState,
    /// Full live body state (pools/healing/corpse/energy-flow caches).
    pub body: BodyState,
    /// Live colour genes.
    pub color: [f32; 3],
    /// Live eye hue offset.
    pub eye_offset: f32,
    /// Runtime feeding/energy diagnostic counters.
    pub feeding: FeedingCounters,
}

/// Slot-aligned snapshot of an [`EggStore`].
#[derive(Clone, Debug, PartialEq)]
pub struct EggCheckpointArena {
    /// Slot → generation.
    pub generations: Vec<u32>,
    /// Free-list order.
    pub free: Vec<u32>,
    /// Dense slot order.
    pub dense: Vec<u32>,
    /// Slot-aligned records (`None` = free slot).
    pub records: Vec<Option<EggCheckpointRecord>>,
}

/// One egg slot's persistent save + runtime-only incubation state.
#[derive(Clone, Debug, PartialEq)]
pub struct EggCheckpointRecord {
    /// Persistent egg state.
    pub save: EggSave,
    /// Raw internal rotation in radians (save stores lossy degrees).
    pub rot_radians: f32,
    /// Incubation active flag.
    pub is_hatching: bool,
    /// Aborting flag.
    pub aborting: bool,
    /// Birth orientation.
    pub birth_orientation: Vec2,
    /// Child size gene (physics scale).
    pub size_gene: f32,
}

/// What `BibiteBody.ExplodeToMeat` asks the particle master for (`BloodSplatterAtPosition`): a burst of blood over a disc of `radius` at `pos`, `count`
/// particles five times, 0.05 s apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloodBurst {
    /// Where the body was.
    pub pos: Vec2,
    /// `bodyLength / 2`.
    pub radius: f32,
    /// `(int)baseBodyArea / 20`.
    pub count: i32,
}

/// The most recent bursts are kept for the viewer (it only draws them); older ones are dropped.
const BLOOD_LOG_CAP: usize = 256;

static BLOOD_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The bibite store: a generational slot vector with a dense iteration list.
#[derive(Clone, Debug)]
pub struct BibiteStore {
    /// The blood bursts of the bodies that exploded, each with a number that only grows (across stores too), for the viewer's particles.
    blood: Vec<(u64, BloodBurst)>,
    records: Vec<Option<BibiteRecord>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    dense: Vec<u32>,
    settings: SimSettings,
    collision_sign: CollisionSign,
    pending_kills: Vec<BibiteId>,
    brain_threads: usize,
    /// Sizes from which each stage runs on the worker pool.
    gates: crate::sim::ParallelGates,
    /// Bibites removed since the world's spatial hash last drained the log (it drops their entries at the end of the tick).
    removed: Vec<BibiteId>,
    /// Something the log does not cover happened (a checkpoint was restored, or the log overflowed): the hash has to look at
    /// every entry.
    removed_unknown: bool,
    /// A death was queued ([`BibiteStore::kill`]), or may have been (a restored checkpoint), since the pending kills were last
    /// looked for: without it the scan over every record finds nothing and is skipped.
    kills_queued: bool,
    /// The number of the body tick under way or last run (it starts at 1): a record's `phys_stamp` is current if it equals it.
    epoch: u32,
    /// The slots of the bibites that had mouth targets when the sweep after the last physics step ended, in slot order, if
    /// `mouth_candidates_valid`: the next body tick looks at these records instead of at every record. Only that sweep gives a
    /// bibite mouth targets (and a spawn or a restore with some invalidates the list), so no other can have any.
    mouth_candidates: Vec<u32>,
    mouth_candidates_valid: bool,
    /// The summary of every slot (see [`SlotSummary`]); right after [`BibiteStore::refresh_summaries`].
    summaries: Vec<SlotSummary>,
    /// The slots whose record may have changed since the last refresh (every `get_mut` adds its slot), and which are in the list.
    summary_touched: Vec<u32>,
    summary_listed: Vec<bool>,
    /// Everything may have changed (a checkpoint was restored, or `for_each_mut` ran): the next refresh looks at every slot.
    summaries_all: bool,
    /// How many summaries carry [`SlotSummary::RAD`], right after [`BibiteStore::refresh_summaries`].
    rad_live: usize,
    /// The position of every slot in `dense` (`u32::MAX` for a slot with no live record): `remove` finds the slot to move in one
    /// step, and the rank of every slot (the order the serial passes visit the bibites in) is a copy of this.
    dense_pos: Vec<u32>,
}

/// Removals the log keeps before the hash is told to look at every entry instead.
const REMOVED_LOG_LIMIT: usize = 65_536;

/// The last generation number given to a set of cached colliders (see [`BibiteRecord::colliders_gen`]).
static COLLIDER_GENERATION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// A generation number nobody has had before (and never 0): equal numbers on a record and on its physics body mean that the body has
/// the record's colliders. Only equality is ever asked of it, so which number a record gets cannot change a result.
fn next_collider_generation() -> u32 {
    loop {
        let generation = COLLIDER_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if generation != 0 {
            return generation;
        }
    }
}

/// The lines of a record that a bibite's turn reads: the pose and the forces (lines 0 and 1), the organs' run, and the first
/// lines of the brain (the headers a read of its output values goes through).
#[cfg(feature = "prefetch")]
const RUN_FIRST_LINE: usize = (std::mem::offset_of!(BibiteRecord, feeding) + 128) / 64;
#[cfg(feature = "prefetch")]
const RUN_LAST_LINE: usize =
    (std::mem::offset_of!(BibiteRecord, steering) + std::mem::size_of::<SteeringInputs>() - 1) / 64;
#[cfg(feature = "prefetch")]
const BRAIN_FIRST_LINE: usize = std::mem::offset_of!(BibiteRecord, brain) / 64;

/// Asks the caches for the lines of the record at `record` that the turn of its bibite touches. A hint: it changes nothing a
/// program can observe (with the `prefetch` feature off it does nothing at all).
#[inline(always)]
fn prefetch_turn(record: *const u8) {
    #[cfg(feature = "prefetch")]
    {
        let lines = [0, 1, BRAIN_FIRST_LINE, BRAIN_FIRST_LINE + 1]
            .into_iter()
            .chain(RUN_FIRST_LINE..=RUN_LAST_LINE);
        for line in lines {
            bibites_prefetch::read_l2(record.wrapping_add(line * 64));
        }
    }
    #[cfg(not(feature = "prefetch"))]
    let _ = record;
}

impl BibiteStore {
    /// Creates an empty store with vanilla settings.
    pub fn new() -> Self {
        Self::with_settings(SimSettings::game_defaults())
    }

    /// Creates an empty store with explicit settings (used for spawn vitals).
    pub fn with_settings(settings: SimSettings) -> Self {
        Self {
            records: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            dense: Vec::new(),
            settings,
            collision_sign: CollisionSign::AsIs,
            pending_kills: Vec::new(),
            brain_threads: 0,
            gates: crate::sim::ParallelGates::default(),
            removed: Vec::new(),
            removed_unknown: false,
            blood: Vec::new(),
            kills_queued: false,
            epoch: 1,
            mouth_candidates: Vec::new(),
            mouth_candidates_valid: false,
            summaries: Vec::new(),
            summary_touched: Vec::new(),
            summary_listed: Vec::new(),
            summaries_all: false,
            rad_live: 0,
            dense_pos: Vec::new(),
        }
    }

    /// Replaces the settings snapshot (e.g. scenario load).
    pub fn set_settings(&mut self, settings: SimSettings) {
        let old_metabolism_cost =
            BodySettings::from_sim_settings(&self.settings).base_metabolism_cost;
        self.settings = settings;
        // `MetabolicEfficiency` is re-read on `energyUsageEfficiency` changes
        // (`BibiteBody.cs:384`) and `HealthEnergyRatio` /
        // `SumBodyAndHealthEnergyRatio` are live properties of the genome
        // (`BibiteGenes.cs:119-123`), so existing bibites follow settings
        // edits. `baseEnergyDensity` stays cached, as in the game.
        let bs = BodySettings::from_sim_settings(&self.settings);
        // `MetabolismCost` changes re-run `UpdateMetabolism` (`BibiteBody.cs:511`).
        let metabolism_changed = bs.base_metabolism_cost != old_metabolism_cost;
        for record in self.records.iter_mut().flatten() {
            record.phys_stamp = 0;
            if metabolism_changed {
                record
                    .body
                    .update_metabolism(age_hours(record.clock.time_alive), &bs);
            }
            let density = record.genome.body_energy_density(&self.settings);
            let params = &mut record.body.params;
            params.metabolic_efficiency = bs.energy_usage_efficiency.powf(params.metabolic_rate);
            params.health_energy_ratio = density * bs.health_energy_factor;
            params.sum_body_health_energy_ratio = density * (1.0 + bs.health_energy_factor);
            params.health_material_density = bs.health_material_density;
        }
    }

    /// Settings snapshot.
    pub fn settings(&self) -> &SimSettings {
        &self.settings
    }

    /// Collision-normal sign policy for B2b collision damage (BQ-4).
    pub fn set_collision_sign(&mut self, sign: CollisionSign) {
        self.collision_sign = sign;
    }

    /// Worker threads for the parallel brain pass (`0` = auto-detect).
    ///
    /// Results are bit-identical for any thread count; the world should set
    /// this from its `--threads` option (1 forces the serial path).
    pub fn set_brain_threads(&mut self, threads: usize) {
        self.brain_threads = threads;
    }

    /// Worker-thread setting for the parallel brain pass (`0` = auto).
    pub fn brain_threads(&self) -> usize {
        self.brain_threads
    }

    /// Sets the sizes from which each stage runs on the worker pool.
    pub fn set_parallel_gates(&mut self, gates: crate::sim::ParallelGates) {
        self.gates = gates;
    }

    /// Number of live bibites.
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Number of slots ever allocated (diagnostics).
    pub fn slot_capacity(&self) -> usize {
        self.records.len()
    }

    fn id_at_slot(&self, slot: usize) -> BibiteId {
        BibiteId::from_handle(Handle::new(slot as u32, self.generations[slot]))
    }

    fn slot_of(&self, id: BibiteId) -> Option<usize> {
        let handle = id.handle();
        let slot = handle.slot() as usize;
        if slot < self.records.len()
            && self.generations[slot] == handle.generation()
            && self.records[slot].is_some()
        {
            Some(slot)
        } else {
            None
        }
    }

    /// Where the live bibite `id` is in the dense order (its physics body's place in a step), `None` for an id that is not a live
    /// bibite: a body that is gone and a newcomer in its slot are two bodies, and a tag from the past must not find the newcomer.
    fn dense_position_of(&self, id: BibiteId) -> Option<usize> {
        let slot = self.slot_of(id)?;
        self.dense_pos
            .get(slot)
            .copied()
            .filter(|at| *at != u32::MAX)
            .map(|at| at as usize)
    }

    /// Record access.
    pub fn get(&self, id: BibiteId) -> Option<&BibiteRecord> {
        self.records[self.slot_of(id)?].as_ref()
    }

    /// Mutable record access.
    pub fn get_mut(&mut self, id: BibiteId) -> Option<&mut BibiteRecord> {
        let slot = self.slot_of(id)?;
        self.touch_summary(slot);
        let record = self.records[slot].as_mut()?;
        // Whatever the caller does, the physics cache is not known to hold afterwards (see `refresh_physics_cache`).
        record.phys_stamp = 0;
        Some(record)
    }

    /// Dense iteration over live ids.
    pub fn ids(&self) -> impl Iterator<Item = BibiteId> + '_ {
        self.dense
            .iter()
            .map(move |slot| self.id_at_slot(*slot as usize))
    }

    /// `BibiteEggLayingOrgan.LayEgg` called directly (`GUIManager.LayEggIfBibite`,
    /// the LAY button / Insert key): one egg is created for a living, ready
    /// bibite without consuming egg progress or energy; only `nEggsLaid` grows.
    pub fn lay_single_egg(&mut self, id: BibiteId, ctx: &mut BibiteTickCtx<'_>) -> u32 {
        let Some(record) = self.get_mut(id) else {
            return 0;
        };
        if !record.born || !record.is_ready || record.body.dead {
            return 0;
        }
        let laid = lay_eggs(record, ctx, 1);
        record.egg_layer.n_laid += laid;
        laid
    }

    /// Forces every completed egg in a bibite's womb to be laid now.
    /// Returns the number laid.
    pub fn force_lay_eggs(&mut self, id: BibiteId, ctx: &mut BibiteTickCtx<'_>) -> u32 {
        let Some(record) = self.get_mut(id) else {
            return 0;
        };
        if !record.born || !record.is_ready || record.body.dead {
            return 0;
        }
        let count = record.egg_layer.n_eggs();
        if count == 0 {
            return 0;
        }
        let laid = lay_eggs(record, ctx, count);
        if laid > 0 {
            record.egg_layer.on_eggs_laid(laid);
        }
        laid
    }

    /// Live (non-corpse) bibite ids in dense order. UI selection helpers.
    pub fn live_ids(&self) -> impl Iterator<Item = BibiteId> + '_ {
        self.iter()
            .filter(|(_, record)| !record.body.dead)
            .map(|(id, _)| id)
    }

    /// Number of live bibites.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live_ids().count()
    }

    /// Next live bibite after `current`, wrapping in dense order.
    #[must_use]
    pub fn next_live_bibite(&self, current: Option<BibiteId>) -> Option<BibiteId> {
        let live: Vec<BibiteId> = self.live_ids().collect();
        if live.is_empty() {
            return None;
        }
        let next = current
            .and_then(|id| live.iter().position(|candidate| *candidate == id))
            .map_or(0, |index| (index + 1) % live.len());
        Some(live[next])
    }

    /// Previous live bibite before `current`, wrapping in dense order.
    #[must_use]
    pub fn previous_live_bibite(&self, current: Option<BibiteId>) -> Option<BibiteId> {
        let live: Vec<BibiteId> = self.live_ids().collect();
        if live.is_empty() {
            return None;
        }
        let previous = current
            .and_then(|id| live.iter().position(|candidate| *candidate == id))
            .map_or(0, |index| (index + live.len() - 1) % live.len());
        Some(live[previous])
    }

    /// Oldest live bibite (`SelectOldestBibite`).
    #[must_use]
    pub fn oldest_live_bibite(&self) -> Option<BibiteId> {
        self.iter()
            .filter(|(_, record)| !record.body.dead)
            .max_by(|(_, a), (_, b)| a.clock.time_alive.total_cmp(&b.clock.time_alive))
            .map(|(id, _)| id)
    }

    /// Live bibite with the highest generation (`SelectHighGenBibite`).
    #[must_use]
    pub fn highest_generation_live_bibite(&self) -> Option<BibiteId> {
        self.iter()
            .filter(|(_, record)| !record.body.dead)
            .max_by_key(|(_, record)| record.meta.generation)
            .map(|(id, _)| id)
    }

    /// Nearest live bibite to a world position (`SelectClosestEntity` subset).
    #[must_use]
    pub fn nearest_live_bibite(&self, position: Vec2) -> Option<BibiteId> {
        self.iter()
            .filter(|(_, record)| !record.body.dead)
            .min_by(|(_, a), (_, b)| {
                a.pos
                    .distance_sq(position)
                    .total_cmp(&b.pos.distance_sq(position))
            })
            .map(|(id, _)| id)
    }

    /// Dense iteration over `(id, record)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (BibiteId, &BibiteRecord)> + '_ {
        self.dense.iter().map(move |slot| {
            let slot = *slot as usize;
            (self.id_at_slot(slot), self.records[slot].as_ref().unwrap())
        })
    }

    /// Dense mutable iteration (record-level; no cross-record access).
    pub fn for_each_mut(&mut self, mut f: impl FnMut(BibiteId, &mut BibiteRecord)) {
        self.summaries_all = true;
        for index in 0..self.dense.len() {
            let slot = self.dense[index] as usize;
            let id = self.id_at_slot(slot);
            if let Some(record) = self.records[slot].as_mut() {
                record.phys_stamp = 0;
                f(id, record);
            }
        }
    }

    /// Cheap view for sensing/tools.
    pub fn view(&self, id: BibiteId) -> Option<BibiteView> {
        self.get(id).map(|r| BibiteView {
            id: r.id,
            pos: r.pos,
            vel: r.vel,
            rot: r.rot,
            ang_vel: r.ang_vel,
            d1_size: r.derived.d1_size,
            d2_size: r.derived.d2_size,
            base_body_area: r.derived.base_body_area,
            max_health: r.body.geometry.max_health,
            health_ratio: r.body.health_ratio(),
            dead: r.body.dead,
            is_corpse: r.is_corpse,
            is_ready: r.is_ready,
            color: r.color,
            eye_offset: r.eye_offset,
            is_rain: r.meta.is_rain,
        })
    }

    /// Genome of a bibite.
    pub fn genome(&self, id: BibiteId) -> Option<&Genome> {
        self.get(id).map(|r| &r.genome)
    }

    /// Brain of a bibite.
    pub fn brain(&self, id: BibiteId) -> Option<&Brain> {
        self.get(id).map(|r| &r.brain)
    }

    /// Last world senses written for a bibite.
    pub fn senses(&self, id: BibiteId) -> Option<&SenseOutputs> {
        self.get(id).map(|r| &r.senses)
    }

    /// Writes the world senses (called by the world sensing phase before B2b's
    /// tick).
    pub fn set_senses(&mut self, id: BibiteId, senses: SenseOutputs) {
        if let Some(record) = self.get_mut(id) {
            record.senses = senses;
        }
    }

    /// Writes the results of the sensing phase into the records (`results` are in dense order). Every result is for a record of
    /// its own, so with enough of them the pool takes the records by stretches of slots; the records end up as when the results
    /// are written one by one ([`BibiteStore::set_senses`], [`BibiteStore::set_steering`], and the refreshed FOV cache).
    pub(crate) fn apply_senses(
        &mut self,
        results: &[crate::world::SensingOutcome],
        executor: crate::sim::ParallelExecutor<'_>,
    ) {
        let workers = crate::sim::ParallelGates::workers_for(
            results.len(),
            self.gates.due_brains,
            executor.threads(),
        );
        if workers <= 1 {
            for (id, senses, cache) in results {
                if let Some((senses, steering)) = senses {
                    self.set_senses(*id, *senses);
                    if let Some(steering) = steering {
                        self.set_steering(*id, *steering);
                    }
                }
                if let Some(cache) = cache
                    && let Some(record) = self.get_mut(*id)
                {
                    record.fov_seen.clone_from(cache);
                }
            }
            return;
        }
        let chunk_len = self.records.len().div_ceil(workers * 4).max(1);
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); self.records.len().div_ceil(chunk_len)];
        for (index, (id, _, _)) in results.iter().enumerate() {
            buckets[id.handle().slot() as usize / chunk_len].push(index as u32);
        }
        self.records
            .par_chunks_mut(chunk_len)
            .zip(buckets.par_iter())
            .enumerate()
            .for_each(|(chunk_index, (chunk, bucket))| {
                for &index in bucket {
                    let (id, senses, cache) = &results[index as usize];
                    let offset = id.handle().slot() as usize - chunk_index * chunk_len;
                    let Some(record) = chunk[offset].as_mut() else {
                        continue;
                    };
                    if record.id != *id {
                        continue;
                    }
                    record.phys_stamp = 0;
                    if let Some((senses, steering)) = senses {
                        record.senses = *senses;
                        if let Some(steering) = steering {
                            record.steering = *steering;
                        }
                    }
                    if let Some(cache) = cache {
                        record.fov_seen.clone_from(cache);
                    }
                }
            });
    }

    /// [`BibiteStore::apply_senses`] for the results as the sensing jobs left them, one buffer each: the pool cuts every buffer into
    /// the records' stretches in parallel, takes the records by stretches and applies what is for them, and empties the buffers
    /// (the drop of the results' caches is the pool's too). No bibite has more than one result, so the order in which they are
    /// applied cannot show.
    pub(crate) fn apply_senses_chunks(
        &mut self,
        chunks: &mut [Vec<crate::world::SensingOutcome>],
        executor: crate::sim::ParallelExecutor<'_>,
    ) {
        let total: usize = chunks.iter().map(Vec::len).sum();
        let workers =
            crate::sim::ParallelGates::workers_for(total, self.gates.due_brains, executor.threads());
        if workers <= 1 {
            for chunk in chunks.iter_mut() {
                self.apply_senses(chunk, executor);
                chunk.clear();
            }
            return;
        }
        let chunk_len = self.records.len().div_ceil(workers * 4).max(1);
        let stretches = self.records.len().div_ceil(chunk_len);
        // For every buffer, the indices of its results by the stretch of the record they are for.
        let buckets: Vec<Vec<Vec<u32>>> = chunks
            .par_iter()
            .map(|results| {
                let mut by_stretch: Vec<Vec<u32>> = vec![Vec::new(); stretches];
                for (index, (id, _, _)) in results.iter().enumerate() {
                    by_stretch[id.handle().slot() as usize / chunk_len].push(index as u32);
                }
                by_stretch
            })
            .collect();
        let chunks_ref: &[Vec<crate::world::SensingOutcome>] = chunks;
        self.records
            .par_chunks_mut(chunk_len)
            .enumerate()
            .for_each(|(stretch, records)| {
                for (job, results) in chunks_ref.iter().enumerate() {
                    for &index in &buckets[job][stretch] {
                        let (id, senses, cache) = &results[index as usize];
                        let offset = id.handle().slot() as usize - stretch * chunk_len;
                        let Some(record) = records[offset].as_mut() else {
                            continue;
                        };
                        if record.id != *id {
                            continue;
                        }
                        record.phys_stamp = 0;
                        if let Some((senses, steering)) = senses {
                            record.senses = *senses;
                            if let Some(steering) = steering {
                                record.steering = *steering;
                            }
                        }
                        if let Some(cache) = cache {
                            record.fov_seen.clone_from(cache);
                        }
                    }
                }
            });
        chunks.par_iter_mut().for_each(Vec::clear);
    }

    /// Sense throttle counters.
    pub fn sense_throttle(&self, id: BibiteId) -> Option<SenseThrottle> {
        self.get(id).map(|r| r.throttle)
    }

    /// Mutable sense throttle counters.
    pub fn sense_throttle_mut(&mut self, id: BibiteId) -> Option<&mut SenseThrottle> {
        self.get_mut(id).map(|r| &mut r.throttle)
    }

    /// Writes the steering inputs for a bibite (world FOV/zone phase).
    pub fn set_steering(&mut self, id: BibiteId, steering: SteeringInputs) {
        if let Some(record) = self.get_mut(id) {
            record.steering = steering;
        }
    }

    /// Steering inputs of a bibite.
    pub fn steering(&self, id: BibiteId) -> Option<SteeringInputs> {
        self.get(id).map(|record| record.steering)
    }

    /// Adds a mutation count/sigma increase (convenience wrapper around
    /// [`BibiteStore::add_mutation_factor`] for rad towers/red death; `count`
    /// maps to `probability_increase`, `sigma` to `intensity_increase`).
    pub fn add_mutation_increase(&mut self, id: BibiteId, count: f32, sigma: f32) {
        self.add_mutation_factor(
            id,
            MutationFactor {
                probability_increase: count,
                intensity_increase: sigma,
            },
        );
    }

    /// Marks a bibite as touched by a `RadioZone` (idempotent, like the game's
    /// keyed `mutationFactors["RadZone"]`).
    pub fn mark_rad_zone(&mut self, id: BibiteId) {
        if let Some(record) = self.get_mut(id) {
            record.rad_zone = true;
        }
    }

    /// Adds environmental mutation pressure to a bibite.
    pub fn add_mutation_factor(&mut self, id: BibiteId, factor: MutationFactor) {
        if let Some(record) = self.get_mut(id) {
            record.mutation_factor.probability_increase += factor.probability_increase;
            record.mutation_factor.intensity_increase += factor.intensity_increase;
        }
    }

    fn insert_record(&mut self, mut record: BibiteRecord) -> BibiteId {
        let slot = match self.free.pop() {
            Some(slot) => {
                let slot = slot as usize;
                self.generations[slot] = self.generations[slot].wrapping_add(1);
                slot
            }
            None => {
                let slot = self.records.len();
                self.records.push(None);
                self.generations.push(0);
                slot
            }
        };
        self.touch_summary(slot);
        record.id = self.id_at_slot(slot);
        if record.n_mouth_targets > 0 {
            self.mouth_candidates_valid = false;
        }
        self.records[slot] = Some(record);
        self.dense.push(slot as u32);
        if self.dense_pos.len() <= slot {
            self.dense_pos.resize(slot + 1, u32::MAX);
        }
        self.dense_pos[slot] = (self.dense.len() - 1) as u32;
        self.id_at_slot(slot)
    }

    /// Notes the blood of a body that bursts into meat (`ParticlesMaster.BloodSplatterAtPosition`).
    fn log_blood(&mut self, burst: BloodBurst) {
        if self.blood.len() >= BLOOD_LOG_CAP {
            self.blood.remove(0);
        }
        let number = BLOOD_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.blood.push((number, burst));
    }

    /// The bursts of blood noted so far (oldest first) with their numbers.
    #[must_use]
    pub fn blood_log(&self) -> &[(u64, BloodBurst)] {
        &self.blood
    }

    /// Removes a bibite from the store and returns its record.
    pub fn remove(&mut self, id: BibiteId) -> Option<BibiteRecord> {
        let slot = self.slot_of(id)?;
        self.touch_summary(slot);
        let record = self.records[slot].take();
        let index = self.dense_pos.get(slot).copied().unwrap_or(u32::MAX) as usize;
        if self.dense.get(index) == Some(&(slot as u32)) {
            self.dense.swap_remove(index);
            if let Some(&moved) = self.dense.get(index) {
                self.dense_pos[moved as usize] = index as u32;
            }
        }
        if let Some(position) = self.dense_pos.get_mut(slot) {
            *position = u32::MAX;
        }
        self.free.push(slot as u32);
        if self.removed.len() < REMOVED_LOG_LIMIT {
            self.removed.push(id);
        } else {
            self.removed_unknown = true;
        }
        record
    }

    /// Moves the ids removed since the last call into `out` (replacing its contents) and says whether the log is incomplete
    /// (then every spatial-hash entry has to be looked at, not only these).
    pub fn drain_removed(&mut self, out: &mut Vec<BibiteId>) -> bool {
        out.clear();
        out.append(&mut self.removed);
        std::mem::take(&mut self.removed_unknown)
    }

    /// Whether `id` names a live record. This is the question only: [`BibiteStore::get`] hands out the record.
    #[must_use]
    pub fn is_live(&self, id: BibiteId) -> bool {
        self.slot_of(id).is_some()
    }

    /// Appends to `out`, in dense order, what `f` makes of each live record (`None` skips it).
    ///
    /// With a pool and enough bibites the dense list is cut into stretches that the workers read, and the stretches' answers are
    /// joined in order, so the result is the one a plain loop gives. `f` only reads: this is for the scans that look at every
    /// record and keep a few, each of which costs a cache miss on a 2 KB record that the pool takes in parallel.
    pub fn filter_map_dense_into<R, F>(
        &self,
        executor: crate::sim::ParallelExecutor<'_>,
        out: &mut Vec<R>,
        f: F,
    ) where
        R: Send,
        F: Fn(BibiteId, &BibiteRecord) -> Option<R> + Sync,
    {
        let workers = crate::sim::ParallelGates::workers_for(
            self.dense.len(),
            self.gates.records,
            executor.threads(),
        );
        if workers <= 1 {
            out.extend(self.iter().filter_map(|(id, record)| f(id, record)));
            return;
        }
        let records = &self.records;
        let generations = &self.generations;
        let min_len = crate::sim::ParallelGates::chunk_len(
            self.dense.len(),
            self.gates.records,
            workers,
            4,
        );
        out.par_extend(
            self.dense
                .par_iter()
                .with_min_len(min_len)
                .filter_map(|slot| {
                    let slot = *slot as usize;
                    let record = records[slot].as_ref()?;
                    f(
                        BibiteId::from_handle(Handle::new(slot as u32, generations[slot])),
                        record,
                    )
                }),
        );
    }

    /// Copies the dense position of every slot into `rank` (`u32::MAX` for the slots with no live record).
    fn copy_dense_ranks(&self, rank: &mut Vec<u32>) {
        #[cfg(test)]
        {
            let mut fresh = vec![u32::MAX; self.records.len()];
            for (index, slot) in self.dense.iter().enumerate() {
                fresh[*slot as usize] = index as u32;
            }
            let mut mirrored = self.dense_pos.clone();
            mirrored.resize(self.records.len(), u32::MAX);
            assert_eq!(
                mirrored, fresh,
                "dense_pos is not the position of the slots in dense"
            );
        }
        rank.clear();
        rank.extend_from_slice(&self.dense_pos);
        rank.resize(self.records.len(), u32::MAX);
    }

    /// Notes that the record in `slot` may be changed by whoever holds it (see [`BibiteStore::refresh_summaries`]).
    fn touch_summary(&mut self, slot: usize) {
        if self.summaries_all {
            return;
        }
        if slot >= self.summary_listed.len() {
            // A slot the mirrors have not met yet (the store grew by one).
            self.summary_listed.resize(slot + 1, false);
            self.summaries.resize(slot + 1, SlotSummary::default());
        }
        if !self.summary_listed[slot] {
            self.summary_listed[slot] = true;
            self.summary_touched.push(slot as u32);
        }
    }

    /// Brings the summaries of the slots up to date: the ones `get_mut` handed out, that were inserted or removed, or all of
    /// them after a restore or a `for_each_mut`. A scan over the summaries ([`BibiteStore::filter_map_dense_summaries_into`])
    /// is right if it follows this. (The fields summarized - dead, ready, rad, species, the brain's pheromone sensing - are
    /// changed through `get_mut` or by building or removing a record, and by nothing else: the sweeps do not touch them.)
    pub(crate) fn refresh_summaries(&mut self) {
        let count = self.records.len();
        self.summaries.resize(count, SlotSummary::default());
        self.summary_listed.resize(count, false);
        if self.summaries_all {
            let mut rad = 0;
            for slot in 0..count {
                let summary = self.records[slot]
                    .as_ref()
                    .map_or_else(SlotSummary::default, SlotSummary::of);
                rad += usize::from(summary.bits & SlotSummary::RAD != 0);
                self.summaries[slot] = summary;
            }
            self.rad_live = rad;
            self.summary_listed.fill(false);
            self.summary_touched.clear();
            self.summaries_all = false;
        } else {
            let touched = std::mem::take(&mut self.summary_touched);
            for &slot in &touched {
                let slot = slot as usize;
                self.summary_listed[slot] = false;
                let summary = self.records[slot]
                    .as_ref()
                    .map_or_else(SlotSummary::default, SlotSummary::of);
                self.rad_live -= usize::from(self.summaries[slot].bits & SlotSummary::RAD != 0);
                self.rad_live += usize::from(summary.bits & SlotSummary::RAD != 0);
                self.summaries[slot] = summary;
            }
            self.summary_touched = touched;
            self.summary_touched.clear();
        }
        #[cfg(test)]
        self.assert_summaries_current();
    }

    /// Test oracle: every summary equals the one made from its record now. A write to a summarized field that does not go
    /// through `get_mut`, `insert_record`, `remove` or a restore makes this fail in whichever test ticks the world.
    #[cfg(test)]
    pub(crate) fn assert_summaries_current(&self) {
        assert_eq!(self.summaries.len(), self.records.len());
        assert_eq!(
            self.rad_live,
            self.summaries
                .iter()
                .filter(|summary| summary.bits & SlotSummary::RAD != 0)
                .count(),
            "the count of rad bibites is not the summaries'"
        );
        for (slot, record) in self.records.iter().enumerate() {
            let fresh = record
                .as_ref()
                .map_or_else(SlotSummary::default, SlotSummary::of);
            assert_eq!(
                self.summaries[slot], fresh,
                "the summary of slot {slot} is stale"
            );
        }
    }

    /// How many live bibites carry the rad flag (needs [`BibiteStore::refresh_summaries`] first).
    pub(crate) fn rad_bibites(&self) -> usize {
        debug_assert!(
            self.summary_touched.is_empty() && !self.summaries_all,
            "the count is read before the summaries were refreshed"
        );
        self.rad_live
    }

    /// Like [`BibiteStore::filter_map_dense_into`] for scans that ask the summary of a slot and only some of them the record:
    /// `f` gets the id, the summary, the slots and the slot number, and takes `slots[slot]` only when it must. Needs
    /// [`BibiteStore::refresh_summaries`] first.
    pub(crate) fn filter_map_dense_summaries_into<R, F>(
        &self,
        executor: crate::sim::ParallelExecutor<'_>,
        out: &mut Vec<R>,
        f: F,
    ) where
        R: Send,
        F: Fn(BibiteId, SlotSummary, &[Option<BibiteRecord>], usize) -> Option<R> + Sync,
    {
        debug_assert!(
            self.summary_touched.is_empty()
                && !self.summaries_all
                && self.summaries.len() == self.records.len(),
            "the summaries are read before they were refreshed"
        );
        let pool = executor.threads();
        let records = &self.records[..];
        let summaries = &self.summaries;
        let generations = &self.generations;
        let ask = |slot: u32| {
            let slot = slot as usize;
            f(
                BibiteId::from_handle(Handle::new(slot as u32, generations[slot])),
                summaries[slot],
                records,
                slot,
            )
        };
        let workers =
            crate::sim::ParallelGates::workers_for(self.dense.len(), self.gates.scans, pool);
        if workers <= 1 {
            out.extend(self.dense.iter().filter_map(|slot| ask(*slot)));
            return;
        }
        let min_len = crate::sim::ParallelGates::chunk_len(
            self.dense.len(),
            self.gates.scans,
            workers,
            4,
        );
        out.par_extend(
            self.dense
                .par_iter()
                .with_min_len(min_len)
                .filter_map(|slot| ask(*slot)),
        );
    }

    /// The bibites that hold something, in dense order: the physics step pins what they hold to their mouths.
    ///
    /// Telling takes one line of every record, a cache miss each: 17 ms for 100,000 bibites on one thread, where the pool takes
    /// them in parallel.
    pub(crate) fn holders(&self, executor: crate::sim::ParallelExecutor<'_>) -> Vec<BibiteId> {
        let mut holders = Vec::new();
        self.filter_map_dense_into(executor, &mut holders, |id, record| {
            (record.n_held_targets > 0).then_some(id)
        });
        holders
    }

    /// Spawns a bibite, computing vitals/geometry from its genes.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-positive/non-finite spawn size.
    pub fn spawn(&mut self, spawn: BibiteSpawn) -> crate::error::Result<BibiteId> {
        let record = Self::build_record(spawn, &self.settings, None)?;
        Ok(self.insert_record(record))
    }

    /// [`Self::spawn`] for a template spawn: the size a growth mode asks for is read from `basis`, the TEMPLATE's genes, not from the genes the
    /// bibite is spawned with (`WorldObjectsSpawner.SpawnBibiteFromTemplate`: `StartBodyAtGrowthAndNormalize(GrowthAtMature(template.genes))`).
    /// It matters when the genes were randomised: a random womb share makes the spawned genome's own `GrowthAtMature` huge.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-positive/non-finite spawn size.
    pub fn spawn_with_growth_basis(&mut self, spawn: BibiteSpawn, basis: &Genome) -> crate::error::Result<BibiteId> {
        let record = Self::build_record(spawn, &self.settings, Some(basis))?;
        Ok(self.insert_record(record))
    }

    /// Spawns a bibite from serialized body/organ state.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid saved size.
    pub fn spawn_loaded(&mut self, save: BibiteSave) -> crate::error::Result<BibiteId> {
        let settings = self.settings.clone();
        let spawn = BibiteSpawn {
            pos: save.pos,
            rot: save.rot.to_radians(),
            vel: save.vel,
            ang_vel: save.ang_vel,
            genome: save.genome.clone(),
            meta: save.meta.clone(),
            // A save the port wrote while children kept their parents' running node values can hold a stale value in a node nothing feeds (a constant pheromone or herding
            // output with the synapse disabled): put it back to what the game would hold there (`Brain::heal_unfed_nodes`; a game save is not touched).
            brain: {
                let mut brain = save.brain.clone();
                brain.heal_unfed_nodes();
                brain
            },
            vitals: SpawnVitals::Baby,
            energy: Some(save.energy.max(0.0)),
            health: Some(save.health.max(0.0)),
        };
        let id = self.spawn(spawn)?;
        let bs = BodySettings::from_sim_settings(&settings);
        if let Some(record) = self.get_mut(id) {
            record.body.set_2d_size(save.d2_size, true, 0.0, &bs);
            // Organs are derived from the saved body size, not the birth size
            // used by build_record; refresh their areas/caps before restoring
            // saved amounts (fat/womb/armor/mouth/propulsion).
            record.refresh_organs(&settings);
            record.refresh_colliders();
            record.update_derived_rigid_body(&settings);
            record.body.energy.amount = save.energy;
            record.body.health.amount = save.health;
            record.body.dying = save.dying;
            record.body.dead = save.dead;
            record.body.attacked_dmg = save.attacked_dmg;
            record.body.times_attacked = save.times_attacked;
            record.body.total_damage_suffered = save.total_damage_suffered;
            record.body.corpse_energy_offset = save.corpse_energy_offset;
            record.throttle = SenseThrottle {
                brain_counter: save.brain_ticks as u32,
                lookup_counter: save.vision_lookup as u32,
                sense_counter: save.vision_sensing as u32,
            };
            record.growth.growth = save.growth;
            record.growth.applied_growth = save.growth;
            record.growth.mature = save.growth >= record.growth_genes.growth_at_mature;
            record.egg_layer.is_mature = record.growth.mature;
            record.mouth.bite_progress = save.bite_progress;
            record.mouth.attacked_last_frame = save.attacked_last_frame;
            record.mouth.bibites_bitten = save.bibites_bitten;
            record.mouth.total_damage_dealt = save.total_damage_dealt;
            record.mouth.total_murders = save.total_murders;
            record.mouth.murdered_area = save.murdered_area;
            record.egg_layer.progress = save.egg_progress;
            record.egg_layer.n_laid = save.n_eggs_laid;
            record.children = save.children.clone();
            // `fat.fatReserves.Amount = state["fatReservesAmount"]` is a plain field: a save may hold more than the organ's capacity.
            record.fat.amount = save.fat_amount.max(0.0);
            // The fat the save holds inflates the body from the start (`CheckInflateFactor` at load): the sprite, the collider and the armour follow.
            record.fat.refresh_inflate();
            if record.fat.inflate_factor > 0.0 {
                let body_settings = BodySettings::from_sim_settings(&settings);
                record.body.set_inflate_factor(record.fat.inflate_factor, &body_settings);
                record.refresh_organs(&settings);
                record.refresh_colliders();
            }
            record.pheromone.progress = save.phero_progress;
            record.clock.tic = save.tic;
            record.clock.tic_progress = save.tic_progress;
            record.clock.time_alive = save.time_alive;
            record.clock.chrono_time = save.chrono_time;
            record.propulsion.total_travel = save.total_travel;
            record.is_corpse = save.dead;
            let mut stomach = record.stomach;
            let plant = organ_material(&settings.materials.plant);
            let meat = organ_material(&settings.materials.meat);
            let saved_slots = &save.stomach[..save.n_stomach as usize];
            for content in saved_slots {
                let material = if content.material == 0 {
                    StomachMaterial::Plant
                } else {
                    StomachMaterial::Meat
                };
                stomach.add_matter(material, content.amount, &plant, &meat);
            }
            // `add_matter` only carries amounts and groups by material; restore
            // the saved slot order and the mean chunk size verbatim.
            let slots: Vec<(StomachMaterial, f32, f32)> = saved_slots
                .iter()
                .map(|content| {
                    let material = if content.material == 0 {
                        StomachMaterial::Plant
                    } else {
                        StomachMaterial::Meat
                    };
                    (material, content.amount, content.average_chunk_amount)
                })
                .collect();
            stomach.restore_contents(&slots);
            record.stomach = stomach;
            record.recompute_derived(&settings);
        }
        Ok(id)
    }

    /// Kills a bibite with the default [`DeathCause::Hazard`] (tools that do not
    /// care about the cause); see [`BibiteStore::kill`] for an explicit cause.
    pub fn kill_default(&mut self, id: BibiteId) {
        self.kill(id, DeathCause::Hazard);
    }

    /// Queues a death (world tools, color killers, red death).
    ///
    /// Sets the intent fields only: [`BibiteStore::resolve_pending_deaths`]
    /// performs the actual corpse/meat/event transition. Calling it before
    /// [`BibiteStore::tick`] resolves at the start of that tick; calling
    /// `resolve_pending_deaths` directly resolves on the same tick.
    pub fn kill(&mut self, id: BibiteId, cause: DeathCause) {
        if let Some(record) = self.get_mut(id) {
            if record.death_cause.is_none() {
                record.body.dying = true;
                record.death_cause = Some(cause);
                record.n_mouth_targets = 0;
                record.mouth.n_in_mouth = 0;
                self.kills_queued = true;
            }
        }
    }

    /// Resolves every queued [`BibiteStore::kill`] through the shared death
    /// path (corpse creation, meat pellet, `DeathEvent`).
    pub fn resolve_pending_deaths(
        &mut self,
        pellets: &mut PelletStore,
        settings: &SimSettings,
        rng: &mut crate::rng::Rng,
        events: &mut Vec<BibiteEvent>,
    ) {
        // Nothing was queued since the last look, so the scan over every record would find nothing (a death is only queued by
        // `kill`, which raises the flag; a restored checkpoint raises it too).
        if !self.kills_queued {
            #[cfg(test)]
            assert!(
                self.dense.iter().all(|slot| self.records[*slot as usize]
                    .as_ref()
                    .is_none_or(|record| record.death_cause.is_none() || record.body.dead)),
                "a death was queued without raising the flag"
            );
            return;
        }
        self.kills_queued = false;
        let mut pending = std::mem::take(&mut self.pending_kills);
        pending.clear();
        for index in 0..self.dense.len() {
            let slot = self.dense[index] as usize;
            if let Some(record) = self.records[slot].as_ref() {
                if record.death_cause.is_some() && !record.body.dead {
                    let id =
                        BibiteId::from_handle(Handle::new(slot as u32, self.generations[slot]));
                    pending.push(id);
                }
            }
        }
        for id in &pending {
            let cause = self
                .get(*id)
                .and_then(|record| record.death_cause)
                .unwrap_or(DeathCause::Hazard);
            self.die_now(*id, cause, false, pellets, rng, settings, events);
        }
        self.pending_kills = pending;
    }

    /// Reads serialized body/organ/clock state for io.
    pub fn to_save(&self, id: BibiteId) -> Option<BibiteSave> {
        let r = self.get(id)?;
        let mut stomach = [StomachContentSave::default(); STOMACH_SLOTS];
        for (index, content) in r.stomach.contents().enumerate() {
            stomach[index] = StomachContentSave {
                material: match content.material {
                    StomachMaterial::Plant => 0,
                    StomachMaterial::Meat => 1,
                },
                amount: content.amount,
                average_chunk_amount: content.average_chunk_amount,
            };
        }
        Some(BibiteSave {
            pos: r.pos,
            vel: r.vel,
            rot: r.rot.to_degrees(),
            ang_vel: r.ang_vel,
            genome: r.genome.clone(),
            meta: r.meta.clone(),
            brain: r.brain.clone(),
            energy: r.body.energy.amount,
            health: r.body.health.amount,
            d2_size: r.body.geometry.d2_size,
            dying: r.body.dying,
            dead: r.body.dead,
            attacked_dmg: r.body.attacked_dmg,
            times_attacked: r.body.times_attacked,
            total_damage_suffered: r.body.total_damage_suffered,
            brain_ticks: r.throttle.brain_counter as f32,
            vision_lookup: r.throttle.lookup_counter as f32,
            vision_sensing: r.throttle.sense_counter as f32,
            corpse_energy_offset: r.body.corpse_energy_offset,
            growth: r.growth.growth,
            stomach,
            n_stomach: r.stomach.n_content() as u8,
            bite_progress: r.mouth.bite_progress,
            attacked_last_frame: r.mouth.attacked_last_frame,
            bibites_bitten: r.mouth.bibites_bitten,
            total_damage_dealt: r.mouth.total_damage_dealt,
            total_murders: r.mouth.total_murders,
            murdered_area: r.mouth.murdered_area,
            egg_progress: r.egg_layer.progress,
            n_eggs_laid: r.egg_layer.n_laid,
            children: r.children.clone(),
            fat_amount: r.fat.amount,
            phero_progress: r.pheromone.progress,
            tic: r.clock.tic,
            tic_progress: r.clock.tic_progress,
            time_alive: r.clock.time_alive,
            chrono_time: r.clock.chrono_time,
            total_travel: r.propulsion.total_travel,
        })
    }

    fn build_record(
        spawn: BibiteSpawn,
        settings: &SimSettings,
        growth_basis: Option<&Genome>,
    ) -> crate::error::Result<BibiteRecord> {
        let bs = BodySettings::from_sim_settings(settings);
        let genome = spawn.genome;
        let metabolic_rate = genome.metabolic_rate();
        let size_ratio = genome.gene(Gene::SizeRatio);
        let params = BodyParams::new(metabolic_rate, genome.body_energy_density(settings), &bs);
        let growth_genes = growth_genes_of(&genome);
        let d2_birth = genome.d2_size_at_birth();
        let d2 = match spawn.vitals.target_growth(growth_basis.unwrap_or(&genome)) {
            Some(growth) => growth * size_ratio * size_ratio,
            None => {
                // `StartBody`: a newborn with less energy than `BodyEnergyAtBirth` is only as big as that share (`newSize = Energy / bodyEnergyAtBirth`).
                let energy = spawn.energy.unwrap_or_else(|| genome.lay_body_energy(settings));
                let needed = genome.body_energy_at_birth(settings);
                if energy < needed && needed > 0.0 { energy / needed } else { d2_birth }
            }
        };
        if !(d2.is_finite() && d2 > 0.0) {
            return Err(crate::error::Error::invalid_data(
                "bibite spawn size is not positive/finite",
            ));
        }
        let mut body = BodyState::new(params, &bs);
        let growth = if matches!(spawn.vitals, SpawnVitals::FromEgg) {
            body.energy.amount = spawn
                .energy
                .unwrap_or_else(|| genome.lay_body_energy(settings));
            body.birth(d2, &bs);
            genome.growth_at_birth()
        } else {
            body.set_2d_size(d2, true, 0.0, &bs);
            body.health.amount = spawn.health.unwrap_or(body.geometry.max_health);
            body.energy.amount = spawn.energy.unwrap_or(body.geometry.max_energy);
            d2 / (size_ratio * size_ratio)
        };
        body.update_metabolism(0.0, &bs);
        let brain_upkeep = brain_upkeep_per_second(&spawn.brain, settings);
        let egg_energy =
            egg_energy_investment(genome.lay_body_energy(settings), brain_upkeep, size_ratio);
        let area = body.geometry.base_body_area;
        let mature = growth >= growth_genes.growth_at_mature;
        let armor_params = organ_material(&settings.materials.armor);
        let armor = ArmorOrgan::new(
            genome.organ_area_portion(Gene::ArmorWAG),
            body.geometry.d1_size,
            0.0,
            armor_params.cohesiveness,
        );
        body.set_armor_thickness(armor.thickness);
        let mut record = BibiteRecord {
            id: BibiteId::from_handle(Handle::new(0, 0)),
            pos: spawn.pos,
            vel: spawn.vel,
            rot: spawn.rot,
            ang_vel: spawn.ang_vel,
            meta: spawn.meta,
            brain: spawn.brain,
            body,
            growth: GrowthState::new(growth, &growth_genes),
            growth_genes,
            stomach: Stomach::new(
                area * genome.organ_area_portion(Gene::StomachWAG),
                genome.gene(Gene::Diet),
                settings.scenario.plant_affinity_power_factor,
                settings.scenario.meat_affinity_power_factor,
                settings.scenario.stomach_content_contribution,
            ),
            fat: FatOrgan::new(
                genome.organ_area_portion(Gene::FatWAG),
                area,
                genome.gene(Gene::FatStorageThreshold),
                genome.gene(Gene::FatStorageDeadband),
            ),
            armor,
            egg_layer: EggLayer::new(
                area * genome.organ_area_portion(Gene::WombWAG),
                genome.area_at_birth(),
                egg_energy,
                genome.gene(Gene::LayTime),
                mature,
            ),
            mouth: MouthState::new(),
            feeding: FeedingCounters::default(),
            clock: InternalClock::new(genome.gene(Gene::ClockSpeed)),
            pheromone: PheromoneOrgan::new(),
            propulsion: PropulsionState::new(spawn.pos),
            mouth_derived: MouthDerived::default(),
            propulsion_derived: PropulsionDerived::default(),
            derived: BibiteDerived::default(),
            colliders: [
                phys::Collider::solid(phys::Shape::circle(0.0)),
                phys::Collider::solid(phys::Shape::circle(0.0)),
                phys::Collider::solid(phys::Shape::circle(0.0)),
            ],
            colliders_gen: next_collider_generation(),
            senses: SenseOutputs::zeroed(),
            throttle: SenseThrottle::default(),
            steering: SteeringInputs::default(),
            fov_seen: FovSeenCache::default(),
            last_contact_count: 0,
            last_contact_impulse: 0.0,
            last_contact_bibite: 0,
            last_contact_pellet: 0,
            last_contact_egg: 0,
            last_contact_collider: [0; 3],
            last_contact_enter_count: 0,
            last_contact_enter_impulse: 0.0,
            prev_contact_tags: [0; 8],
            n_prev_contact_tags: 0,
            mutation_factor: MutationFactor::default(),
            rad_zone: false,
            color: [0.0; 3],
            eye_offset: 0.0,
            is_ready: true,
            born: true,
            is_corpse: false,
            death_cause: None,
            children: Vec::new(),
            pending_force: Vec2::ZERO,
            pending_torque: 0.0,
            mouth_targets: [0; 10],
            n_mouth_targets: 0,
            mouth_target_ages: [0; 10],
            held_targets: [0; 10],
            n_held_targets: 0,
            phys_stamp: 0,
            slot_marker: SlotMarker::Occupied,
            genome,
        };
        record.recompute_derived(settings);
        Ok(record)
    }
}

impl BibiteStore {
    /// Captures the slot layout, persistent saves and runtime-only body state
    /// for checkpointing (world snapshot extension 0x0101/0x0102).
    #[must_use]
    pub fn capture_checkpoint(&self) -> BibiteCheckpointArena {
        let mut records = Vec::with_capacity(self.records.len());
        for slot in 0..self.records.len() {
            records.push(self.records[slot].as_ref().map(|record| {
                let save = self
                    .to_save(record.id)
                    .expect("live record has a save representation");
                BibiteCheckpointRecord {
                    save,
                    rot_radians: record.rot,
                    throttle: record.throttle,
                    senses: record.senses,
                    steering: record.steering,
                    fov_seen: record.fov_seen.clone(),
                    pending_force: record.pending_force,
                    pending_torque: record.pending_torque,
                    mouth_targets: record.mouth_targets,
                    n_mouth_targets: record.n_mouth_targets,
                    mouth_target_ages: record.mouth_target_ages,
                    held_targets: record.held_targets,
                    n_held_targets: record.n_held_targets,
                    death_cause: record.death_cause,
                    is_corpse: record.is_corpse,
                    born: record.born,
                    is_ready: record.is_ready,
                    mutation_factor: record.mutation_factor,
                    brain_node_max_innovation: record.brain.node_max_innovation(),
                    brain_connection_max_innovation: record.brain.connection_max_innovation(),
                    body_geometry: record.body.geometry,
                    body_params: record.body.params,
                    body_metabolism: record.body.metabolism,
                    derived: record.derived,
                    colliders: record.colliders(),
                    propulsion_derived: record.propulsion_derived,
                    mouth_derived: record.mouth_derived,
                    propulsion: record.propulsion,
                    growth: record.growth.clone(),
                    clock: record.clock,
                    fat: record.fat,
                    armor: record.armor,
                    egg_layer: record.egg_layer,
                    stomach_area: record.stomach.area,
                    stomach_total_content: record.stomach.total_content,
                    stomach_acid_level: record.stomach.acid_level,
                    stomach_acid_ratio: record.stomach.acid_ratio,
                    stomach_digestion_effectiveness: record.stomach.digestion_effectiveness,
                    stomach_digestion_potential_bonus: record.stomach.digestion_potential_bonus,
                    stomach_efficiency_malus: record.stomach.efficiency_malus,
                    stomach_contents: record.stomach.contents().copied().collect(),
                    pheromone_progress: record.pheromone.progress,
                    mouth: record.mouth,
                    body: record.body.clone(),
                    color: record.color,
                    eye_offset: record.eye_offset,
                    feeding: record.feeding,
                }
            }));
        }
        BibiteCheckpointArena {
            generations: self.generations.clone(),
            free: self.free.clone(),
            dense: self.dense.clone(),
            records,
        }
    }

    /// Replaces this store with a captured checkpoint. Records are rebuilt from
    /// their persistent saves, then runtime-only fields are applied; derived
    /// caches are recomputed by the normal save-load path. Invalid layouts are
    /// rejected without modifying the store.
    ///
    /// # Errors
    ///
    /// Returns an error for mismatched arena lengths, out-of-range dense/free
    /// slots, or a save that cannot be rebuilt.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: BibiteCheckpointArena,
    ) -> crate::error::Result<()> {
        self.mouth_candidates_valid = false;
        validate_checkpoint_layout(
            &checkpoint.records,
            &checkpoint.generations,
            &checkpoint.dense,
            &checkpoint.free,
        )?;
        let mut rebuilt: Vec<Option<BibiteRecord>> = vec![None; checkpoint.records.len()];
        for (slot, entry) in checkpoint.records.iter().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            let mut temp = BibiteStore::with_settings(self.settings.clone());
            let temp_id = temp.spawn_loaded(entry.save.clone())?;
            let Some(mut record) = temp.remove(temp_id) else {
                return Err(crate::error::Error::invalid_data(
                    "checkpoint record vanished while rebuilding",
                ));
            };
            record.id =
                BibiteId::from_handle(Handle::new(slot as u32, checkpoint.generations[slot]));
            apply_bibite_checkpoint(&mut record, entry);
            rebuilt[slot] = Some(record);
        }
        self.records = rebuilt;
        self.summaries = vec![SlotSummary::default(); self.records.len()];
        self.summary_listed = vec![false; self.records.len()];
        self.summary_touched.clear();
        self.summaries_all = true;
        self.generations = checkpoint.generations;
        self.free = checkpoint.free;
        self.dense = checkpoint.dense;
        self.dense_pos = vec![u32::MAX; self.records.len()];
        for (index, slot) in self.dense.iter().enumerate() {
            self.dense_pos[*slot as usize] = index as u32;
        }
        self.pending_kills.clear();
        self.removed.clear();
        self.removed_unknown = true;
        self.kills_queued = true;
        Ok(())
    }
}

fn apply_bibite_checkpoint(record: &mut BibiteRecord, checkpoint: &BibiteCheckpointRecord) {
    record.throttle = checkpoint.throttle;
    record.senses = checkpoint.senses;
    record.steering = checkpoint.steering;
    record.fov_seen = checkpoint.fov_seen.clone();
    record.pending_force = checkpoint.pending_force;
    record.pending_torque = checkpoint.pending_torque;
    record.mouth_targets = checkpoint.mouth_targets;
    record.n_mouth_targets = checkpoint.n_mouth_targets;
    record.mouth_target_ages = checkpoint.mouth_target_ages;
    record.held_targets = checkpoint.held_targets;
    record.n_held_targets = checkpoint.n_held_targets;
    record.death_cause = checkpoint.death_cause;
    record.is_corpse = checkpoint.is_corpse;
    record.born = checkpoint.born;
    record.is_ready = checkpoint.is_ready;
    record.mutation_factor = checkpoint.mutation_factor;
    record.brain.set_innovations(
        checkpoint.brain_node_max_innovation,
        checkpoint.brain_connection_max_innovation,
    );
    record.body.geometry = checkpoint.body_geometry;
    record.body.params = checkpoint.body_params;
    record.body.metabolism = checkpoint.body_metabolism;
    record.derived = checkpoint.derived;
    record.colliders = checkpoint.colliders;
    record.colliders_gen = next_collider_generation();
    record.propulsion_derived = checkpoint.propulsion_derived;
    record.mouth_derived = checkpoint.mouth_derived;
    record.propulsion = checkpoint.propulsion;
    record.growth = checkpoint.growth.clone();
    record.clock = checkpoint.clock;
    record.fat = checkpoint.fat;
    record.armor = checkpoint.armor;
    record.egg_layer = checkpoint.egg_layer;
    record.stomach.area = checkpoint.stomach_area;
    record.stomach.acid_level = checkpoint.stomach_acid_level;
    record.stomach.acid_ratio = checkpoint.stomach_acid_ratio;
    record.stomach.digestion_effectiveness = checkpoint.stomach_digestion_effectiveness;
    record.stomach.digestion_potential_bonus = checkpoint.stomach_digestion_potential_bonus;
    record.stomach.efficiency_malus = checkpoint.stomach_efficiency_malus;
    record.stomach.set_contents(&checkpoint.stomach_contents);
    // `set_contents` recomputes total_content from the slot amounts; restore the
    // saved live cache afterwards so the checkpoint remains bit-exact.
    record.stomach.total_content = checkpoint.stomach_total_content;
    record.pheromone.progress = checkpoint.pheromone_progress;
    record.mouth = checkpoint.mouth;
    record.rot = checkpoint.rot_radians;
    record.body = checkpoint.body.clone();
    record.color = checkpoint.color;
    record.eye_offset = checkpoint.eye_offset;
    record.feeding = checkpoint.feeding;
}

fn validate_checkpoint_layout(
    records: &[Option<impl Sized>],
    generations: &[u32],
    dense: &[u32],
    free: &[u32],
) -> crate::error::Result<()> {
    let slots = records.len();
    if generations.len() != slots {
        return Err(crate::error::Error::invalid_data(
            "checkpoint generations length does not match records",
        ));
    }
    let mut seen = vec![false; slots];
    for slot in dense {
        let index = *slot as usize;
        if index >= slots || seen[index] {
            return Err(crate::error::Error::invalid_data(
                "checkpoint dense order has an invalid slot",
            ));
        }
        seen[index] = true;
    }
    for slot in free {
        if *slot as usize >= slots {
            return Err(crate::error::Error::invalid_data(
                "checkpoint free list has an invalid slot",
            ));
        }
    }
    Ok(())
}

impl Default for BibiteStore {
    fn default() -> Self {
        Self::new()
    }
}

/// An egg (`EggHatching`) plus the child's genome/brain/meta.
#[derive(Clone, Debug)]
pub struct EggRecord {
    /// Typed id.
    pub id: EggId,
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Rotation (radians).
    pub rot: f32,
    /// Angular velocity (rad/s).
    pub ang_vel: f32,
    /// Child genome.
    pub genome: Genome,
    /// Child metadata (parents live here).
    pub meta: GenomeMeta,
    /// Child brain.
    pub brain: Brain,
    /// Incubation state (energy/hatch time/required energy/progress).
    pub hatch: Egg,
    /// Birth orientation (`−parent.up` for laid eggs).
    pub birth_orientation: Vec2,
    /// Child 1-D size gene at birth (physics scale).
    pub size_gene: f32,
}

impl EggRecord {
    /// Read-only snapshot of the game's egg Stats panel fields.
    #[must_use]
    pub fn stats_view(&self, settings: &crate::settings::SimSettings) -> EggStatsView {
        EggStatsView {
            generation: self.meta.generation,
            egg_area: 5.0 * self.size_gene * self.size_gene,
            egg_mass: self.genome.area_at_birth() * settings.scenario.bibite_mass_density,
            energy: self.hatch.energy,
            hatch_progress: self.hatch.hatch_progress,
            free_energy: self.hatch.free_energy(),
            hatch_time: self.hatch.hatch_time,
            total_required_energy: self.hatch.total_required_energy,
        }
    }
}

/// Request to spawn an egg.
#[derive(Clone)]
pub struct EggSpawn {
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Rotation.
    pub rot: f32,
    /// Angular velocity.
    pub ang_vel: f32,
    /// Child genome.
    pub genome: Genome,
    /// Child metadata.
    pub meta: GenomeMeta,
    /// Child brain.
    pub brain: Brain,
    /// Stored energy (E).
    pub energy: f32,
    /// Hatch time (gene).
    pub hatch_time: f32,
    /// Total required energy (body-at-birth + brain upkeep × SizeRatio).
    pub total_required_energy: f32,
    /// Birth orientation.
    pub birth_orientation: Vec2,
}

/// Everything the world needs to spawn the child when an egg hatches.
#[derive(Clone)]
pub struct HatchPayload {
    /// Child genome.
    pub genome: Genome,
    /// Child metadata (parents/generation).
    pub meta: GenomeMeta,
    /// Child brain.
    pub brain: Brain,
    /// Egg energy available to the newborn.
    pub energy: f32,
    /// Egg position.
    pub pos: Vec2,
    /// Egg rotation.
    pub rot: f32,
    /// Birth orientation for the child (egg `birthOrientation`).
    pub birth_orientation: Vec2,
    /// Child 1-D size gene (physics scale).
    pub size_gene: f32,
}

/// Serialized egg state (spec 02 §9.3).
#[derive(Clone, Debug, PartialEq)]
pub struct EggSave {
    /// Position.
    pub pos: Vec2,
    /// Velocity.
    pub vel: Vec2,
    /// Rotation in game transform degrees (see [`BibiteSave::rot`]).
    pub rot: f32,
    /// Angular velocity.
    pub ang_vel: f32,
    /// Genome.
    pub genome: Genome,
    /// Metadata.
    pub meta: GenomeMeta,
    /// Brain.
    pub brain: Brain,
    /// Energy.
    pub energy: f32,
    /// Incubation progress.
    pub hatch_progress: f32,
    /// Hatch time.
    pub hatch_time: f32,
    /// Total required energy.
    pub total_required_energy: f32,
    /// Hatching flag.
    pub is_hatching: bool,
    /// Aborting flag.
    pub aborting: bool,
    /// Birth orientation.
    pub birth_orientation: Vec2,
    /// Child size gene.
    pub size_gene: f32,
}

/// Egg store (generational slots, dense iteration, reused scratch for ticking so
/// the hot path does not allocate).
#[derive(Clone, Debug, Default)]
pub struct EggStore {
    records: Vec<Option<EggRecord>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    dense: Vec<u32>,
    aborted: Vec<(EggId, f32)>,
    tick_ids: Vec<EggId>,
}

impl EggStore {
    /// Captures the egg slot layout, persistent saves and runtime incubation
    /// state for checkpointing.
    #[must_use]
    pub fn capture_checkpoint(&self) -> EggCheckpointArena {
        let mut records = Vec::with_capacity(self.records.len());
        for slot in 0..self.records.len() {
            records.push(self.records[slot].as_ref().map(|record| {
                EggCheckpointRecord {
                    rot_radians: record.rot,
                    save: self
                        .to_save(record.id)
                        .expect("live egg has a save representation"),
                    is_hatching: record.hatch.is_hatching,
                    aborting: record.hatch.aborting,
                    birth_orientation: record.birth_orientation,
                    size_gene: record.size_gene,
                }
            }));
        }
        EggCheckpointArena {
            generations: self.generations.clone(),
            free: self.free.clone(),
            dense: self.dense.clone(),
            records,
        }
    }

    /// Replaces this store with a captured egg checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error for mismatched arena lengths or invalid dense/free slots.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: EggCheckpointArena,
    ) -> crate::error::Result<()> {
        validate_checkpoint_layout(
            &checkpoint.records,
            &checkpoint.generations,
            &checkpoint.dense,
            &checkpoint.free,
        )?;
        let mut rebuilt: Vec<Option<EggRecord>> = vec![None; checkpoint.records.len()];
        for (slot, entry) in checkpoint.records.iter().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            let mut temp = EggStore::new();
            let temp_id = temp.spawn_loaded(entry.save.clone());
            let Some(mut record) = temp.remove(temp_id) else {
                return Err(crate::error::Error::invalid_data(
                    "checkpoint egg vanished while rebuilding",
                ));
            };
            record.id = EggId::from_handle(Handle::new(slot as u32, checkpoint.generations[slot]));
            record.rot = entry.rot_radians;
            record.hatch.is_hatching = entry.is_hatching;
            record.hatch.aborting = entry.aborting;
            record.birth_orientation = entry.birth_orientation;
            record.size_gene = entry.size_gene;
            rebuilt[slot] = Some(record);
        }
        self.records = rebuilt;
        self.generations = checkpoint.generations;
        self.free = checkpoint.free;
        self.dense = checkpoint.dense;
        self.aborted.clear();
        self.tick_ids.clear();
        Ok(())
    }
}

impl EggStore {
    /// Creates an empty egg store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of eggs.
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    fn id_at_slot(&self, slot: usize) -> EggId {
        EggId::from_handle(Handle::new(slot as u32, self.generations[slot]))
    }

    fn slot_of(&self, id: EggId) -> Option<usize> {
        let handle = id.handle();
        let slot = handle.slot() as usize;
        if slot < self.records.len()
            && self.generations[slot] == handle.generation()
            && self.records[slot].is_some()
        {
            Some(slot)
        } else {
            None
        }
    }

    /// Live egg ids in dense order.
    pub fn ids(&self) -> impl Iterator<Item = EggId> + '_ {
        self.dense
            .iter()
            .map(move |slot| self.id_at_slot(*slot as usize))
    }

    /// Egg access.
    pub fn get(&self, id: EggId) -> Option<&EggRecord> {
        self.records[self.slot_of(id)?].as_ref()
    }

    /// Mutable egg access.
    pub fn get_mut(&mut self, id: EggId) -> Option<&mut EggRecord> {
        let slot = self.slot_of(id)?;
        self.records[slot].as_mut()
    }

    /// Spawns an egg (starts incubation).
    pub fn spawn(&mut self, spec: EggSpawn) -> EggId {
        let slot = match self.free.pop() {
            Some(slot) => {
                let slot = slot as usize;
                self.generations[slot] = self.generations[slot].wrapping_add(1);
                slot
            }
            None => {
                let slot = self.records.len();
                self.records.push(None);
                self.generations.push(0);
                slot
            }
        };
        let id = self.id_at_slot(slot);
        let size_gene = spec.genome.d1_size_at_birth();
        let mut hatch = Egg::new(spec.hatch_time, spec.total_required_energy, spec.energy);
        let _ = hatch.start_hatch();
        self.records[slot] = Some(EggRecord {
            id,
            pos: spec.pos,
            vel: spec.vel,
            rot: spec.rot,
            ang_vel: spec.ang_vel,
            genome: spec.genome,
            meta: spec.meta,
            brain: spec.brain,
            hatch,
            birth_orientation: spec.birth_orientation,
            size_gene,
        });
        self.dense.push(slot as u32);
        id
    }

    /// Removes and returns an egg.
    pub fn remove(&mut self, id: EggId) -> Option<EggRecord> {
        let slot = self.slot_of(id)?;
        let record = self.records[slot].take();
        if let Some(index) = self.dense.iter().position(|s| *s as usize == slot) {
            self.dense.swap_remove(index);
        }
        self.free.push(slot as u32);
        record
    }

    /// Advances incubation; hatched ids are pushed to `out` in dense order.
    /// Eggs that abort are removed and their remaining energy is queued for
    /// [`EggStore::drain_aborted`] (the world spawns the meat pile).
    pub fn tick(&mut self, dt: f32, settings: &SimSettings, out: &mut Vec<EggId>) {
        let _ = settings;
        self.tick_ids.clear();
        for index in 0..self.dense.len() {
            let slot = self.dense[index] as usize;
            self.tick_ids.push(EggId::from_handle(Handle::new(
                slot as u32,
                self.generations[slot],
            )));
        }
        let mut aborted = Vec::new();
        for index in 0..self.tick_ids.len() {
            let id = self.tick_ids[index];
            let Some(egg) = self.get_mut(id) else {
                continue;
            };
            if !egg.hatch.is_hatching {
                if egg.hatch.aborting {
                    aborted.push(id);
                }
                continue;
            }
            match egg.hatch.tick(dt) {
                EggTick::Hatched => out.push(id),
                EggTick::Aborted => aborted.push(id),
                _ => {}
            }
        }
        for id in aborted {
            if let Some(record) = self.remove(id) {
                self.aborted.push((id, record.hatch.energy));
            }
        }
    }

    /// Drains energies of eggs that aborted.
    pub fn drain_aborted(&mut self) -> impl Iterator<Item = (EggId, f32)> + '_ {
        self.aborted.drain(..)
    }

    /// Payload for spawning the child of a hatched egg (call after `tick`
    /// reported the id, then `remove` it).
    pub fn hatch_payload(&self, id: EggId) -> Option<HatchPayload> {
        let egg = self.get(id)?;
        Some(HatchPayload {
            genome: egg.genome.clone(),
            meta: egg.meta.clone(),
            brain: egg.brain.clone(),
            energy: egg.hatch.energy,
            pos: egg.pos,
            rot: egg.rot,
            birth_orientation: egg.birth_orientation,
            size_gene: egg.size_gene,
        })
    }

    /// Stored energy of an egg (meat value when it aborts).
    pub fn stored_energy(&self, id: EggId) -> Option<f32> {
        self.get(id).map(|egg| egg.hatch.energy)
    }

    /// Serialized egg state.
    pub fn to_save(&self, id: EggId) -> Option<EggSave> {
        let egg = self.get(id)?;
        Some(EggSave {
            pos: egg.pos,
            vel: egg.vel,
            rot: egg.rot.to_degrees(),
            ang_vel: egg.ang_vel,
            genome: egg.genome.clone(),
            meta: egg.meta.clone(),
            brain: egg.brain.clone(),
            energy: egg.hatch.energy,
            hatch_progress: egg.hatch.hatch_progress,
            hatch_time: egg.hatch.hatch_time,
            total_required_energy: egg.hatch.total_required_energy,
            is_hatching: egg.hatch.is_hatching,
            aborting: egg.hatch.aborting,
            birth_orientation: egg.birth_orientation,
            size_gene: egg.size_gene,
        })
    }

    /// Loads an egg from serialized state.
    pub fn spawn_loaded(&mut self, save: EggSave) -> EggId {
        let id = self.spawn(EggSpawn {
            pos: save.pos,
            vel: save.vel,
            rot: save.rot.to_radians(),
            ang_vel: save.ang_vel,
            genome: save.genome,
            meta: save.meta,
            brain: save.brain,
            energy: save.energy,
            hatch_time: save.hatch_time,
            total_required_energy: save.total_required_energy,
            birth_orientation: save.birth_orientation,
        });
        if let Some(egg) = self.get_mut(id) {
            egg.hatch.hatch_progress = save.hatch_progress;
            egg.hatch.is_hatching = save.is_hatching;
            egg.hatch.aborting = save.aborting;
            egg.size_gene = save.size_gene;
        }
        id
    }
}

// ---------------------------------------------------------------------------
// B2b: per-bibite tick, brain/senses wiring, feeding, death/corpse
// ---------------------------------------------------------------------------

/// Physics tag kind for a pellet.
pub const TAG_KIND_PELLET: u8 = 3;

/// Physics tag of a pellet.
pub fn pellet_tag(id: PelletId) -> u64 {
    encode_tag(TAG_KIND_PELLET, id.handle())
}

/// Decodes a bibite id from a tag.
pub fn bibite_id_from_tag(tag: u64) -> Option<BibiteId> {
    (tag_kind(tag) == TAG_KIND_BIBITE).then(|| BibiteId::from_handle(decode_tag(tag)))
}

/// Decodes a pellet id from a tag.
pub fn pellet_id_from_tag(tag: u64) -> Option<PelletId> {
    (tag_kind(tag) == TAG_KIND_PELLET).then(|| PelletId::from_handle(decode_tag(tag)))
}

/// Decodes an egg id from a tag.
pub fn egg_id_from_tag(tag: u64) -> Option<EggId> {
    (tag_kind(tag) == TAG_KIND_EGG).then(|| EggId::from_handle(decode_tag(tag)))
}

/// Minimal zone geometry for void avoidance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneCircle {
    /// Zone id.
    pub id: ZoneId,
    /// Centre.
    pub pos: Vec2,
    /// Radius.
    pub radius: f32,
    /// Rectangular zone.
    pub is_rect: bool,
    /// Rectangle half extents.
    pub half_extents: Vec2,
}

/// World-visible body events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BibiteEvent {
    /// A bite landed.
    Bitten {
        /// Biter.
        by: BibiteId,
        /// Victim.
        target: BibiteId,
        /// Damage applied.
        damage: f32,
    },
    /// A pellet was swallowed whole.
    Swallowed {
        /// Eater.
        by: BibiteId,
        /// Pellet.
        pellet: PelletId,
    },
    /// An egg was laid.
    EggLaid {
        /// Parent.
        by: BibiteId,
        /// Egg.
        egg: EggId,
    },
    /// A bibite died.
    Death {
        /// Victim.
        bibite: BibiteId,
        /// Cause.
        cause: DeathCause,
    },
}

/// What the body tick needs from the world (frozen `docs/INTERFACES.md` §5).
pub struct BibiteTickCtx<'a> {
    /// Pellets (feeding).
    pub pellets: &'a mut PelletStore,
    /// Pheromone spots (emission).
    pub pheromones: &'a mut PheromoneStore,
    /// Eggs (laying/hatching).
    pub eggs: &'a mut EggStore,
    /// Zone geometry (void avoidance; reserved until steering lands).
    pub zones: &'a [ZoneCircle],
    /// Live settings.
    pub settings: &'a SimSettings,
    /// Deterministic RNG.
    pub rng: &'a mut crate::rng::Rng,
    /// Reused scratch.
    pub scratch: &'a mut BibiteScratch,
    /// Events drained by the world.
    pub events: &'a mut Vec<BibiteEvent>,
}

#[derive(Clone, Copy, Debug)]
struct PendingBite {
    by: BibiteId,
    target: BibiteId,
    damage: f32,
    drain: bool,
    knockback: f32,
    direction: Vec2,
}

/// A holder let go of bibites with a strong negative grab (`ReleaseAndMaybeThrowAllHeldObjects`): they are thrown along its up vector, it recoils.
#[derive(Clone, Debug)]
struct PendingThrow {
    holder: BibiteId,
    prey: Vec<BibiteId>,
    /// The held pellets: they share the force with the bibites (the game sums the masses of everything held).
    pellets: Vec<PelletId>,
    up: Vec2,
    /// `n_held` of the holder (pellets included): the share each object gets.
    n_held: f32,
    throwing_force_factor: f32,
    jaw_strength: f32,
    desire: f32,
    jaw_mass: f32,
}

#[derive(Clone, Copy, Debug)]
struct PendingDeath {
    id: BibiteId,
    cause: DeathCause,
    swallowed: bool,
}

#[derive(Clone, Copy, Debug)]
struct PendingSwallow {
    predator: BibiteId,
    prey: BibiteId,
}

impl PendingSwallow {
    fn prey_tag(&self) -> u64 {
        bibite_tag(self.prey)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TargetSnapshot {
    tag: u64,
    is_pellet: bool,
    is_egg: bool,
    pos: Vec2,
    radius: f32,
    amount: f32,
    material: u8,
    max_health: f32,
    real_body_area: f32,
    internals_radius: f32,
    effective_radius: f32,
    armor_strength: f32,
    body_length: f32,
    meat_amount: f32,
    stomach_amount: f32,
}

/// Reusable scratch buffers for the bibite phase.
#[derive(Clone, Debug, Default)]
pub struct BibiteScratch {
    /// Wall time of the body and bridge stages (observational only; the world drains it into its phase timings).
    pub clock: crate::sim::StageClock,
    ids: Vec<BibiteId>,
    bites: Vec<PendingBite>,
    throws: Vec<PendingThrow>,
    deaths: Vec<PendingDeath>,
    swallows: Vec<PendingSwallow>,
    /// The relative pose of each held body at the grab, by the held body's tag and the holder's (a pellet may be held by several bibites at once): `(held tag, holder tag, the held body's origin in the holder's frame, the relative rotation)`.
    weld_frames: Vec<(u64, u64, Vec2, f32)>,
    /// The grabs let go of this tick, `(holder tag, held tag)`: the game destroys a `FixedJoint2D` at the end of the frame, so the joint is still there for the physics
    /// steps of the tick in which the mouth lets go (the recoil of a throw is added to a holder that is still welded to what it held).
    pending_releases: Vec<(u64, u64)>,
    /// Collider pairs in contact on the previous physics step; damage only
    /// fires for pairs that are new (`OnCollisionEnter2D`, `BibitePart.cs:47`).
    prev_collisions: CollisionSet,
    /// Set after a checkpoint restore: the next physics step only records the
    /// pairs already touching (they are not new `OnCollisionEnter2D` events)
    /// and must not deal impact damage. A fresh world starts with `false`
    /// because its first contacts really are new.
    suppress_collision_damage: bool,
    /// Bibite count at the start of the current tick (egg-limit check).
    population: usize,
    cur_collisions: CollisionSet,
    physics_bodies: Vec<phys::PhysicsBody>,
    /// The first `kept_bibite_bodies` of `physics_bodies` are the bibites' bodies of the last parallel step, in dense order (the
    /// pellets' and eggs' bodies behind them are made again every step); `bibite_body_gens[i]` is the generation of the colliders
    /// body `i` was given.
    kept_bibite_bodies: usize,
    bibite_body_gens: Vec<u32>,
    physics: phys::PhysicsScratch,
    bibite_ids: Vec<BibiteId>,
    pellet_ids: Vec<PelletId>,
    egg_ids: Vec<EggId>,
    due_ids: Vec<BibiteId>,
    due_slots: Vec<u32>,
    due_inputs: Vec<BrainInputs>,
    due_outputs: Vec<BrainOutputs>,
    /// Dense rank of every record slot (`u32::MAX` for a slot without a live bibite): the order of the serial pass B.
    /// Built by the parallel pass A, for the split pass B.
    rank: Vec<u32>,
    /// Slots of the records that have mouth targets (the parallel pass A lists them for the split pass B).
    mouth_lane: Vec<u32>,
    /// The split pass B is in its dense-order stretch: a bibite looking at a target that comes after it in dense order reads
    /// the target's `pre_snapshots` entry (its state before pass B), as the serial order would.
    split: bool,
    /// `(tag, snapshot)` of the bibite targets that a mouth-bearing bibite looks at before their own turn, sorted by tag.
    pre_snapshots: Vec<(u64, Option<TargetSnapshot>)>,
    /// Per record slot: the sweep takes a snapshot of the bibite here before its pass-B turn (it is the bibite target of
    /// a bibite that comes before it in dense order).
    snapshot_slots: Vec<bool>,
    /// Per record slot: the bibite is a mouth target of a bibite that comes after it in dense order, which reads its state as it
    /// stands after its own turn: that turn must be whole by the time the other's comes, so the rest of it is not left for the pool
    /// (see [`BibiteStore::pass_b_dense`]).
    viewed_slots: Vec<bool>,
    /// How the pass-B turns were shared out between the pool and the calling thread.
    lanes: PassBLanes,
    /// The pellets at rest that the physics bridge keeps out of the step.
    sleep: PelletSleep,
    /// The step's contacts and triggers cut into rows per bibite, for the sweep after the step.
    bridge: BridgeRows,
    /// Tests: `(viewer tag, target tag, snapshot)` of every mouth target a dense-order turn looked at, in order; a parallel
    /// tick must show exactly what the serial tick shows.
    #[cfg(test)]
    pub(crate) snapshot_log: Vec<(u64, u64, String)>,
}

/// How the bibites' pass-B turns were shared between the worker pool and the calling thread over a run (observational).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassBLanes {
    /// Ticks on which the split pass B ran.
    pub ticks: u64,
    /// Turns that ran on the pool: bibites that touch nothing but themselves.
    pub light: u64,
    /// Turns that ran in dense order on the calling thread (a bibite with several reasons is counted once here).
    pub heavy: u64,
    /// Heavy turns of dead bodies (corpse decay, bursting, eggs laid at death).
    pub dead: u64,
    /// Heavy turns of bibites with something in the mouth.
    pub mouth: u64,
    /// Heavy turns of bibites holding a pellet.
    pub held: u64,
    /// Heavy turns of bibites that could vomit.
    pub vomit: u64,
    /// Heavy turns of bibites laying eggs.
    pub eggs: u64,
    /// Heavy turns of bibites emitting pheromone.
    pub emit: u64,
    /// Heavy turns whose head (the stomach, the womb, the fat, the movement) the sweep took on the pool.
    pub head_in_sweep: u64,
    /// Heavy turns whose last part (growth, healing, the clock) the pool took after the dense-order stretch.
    pub post_on_pool: u64,
}

/// What one stretch of records hands back from the pool in the body sweep.
#[derive(Default)]
struct PoolPart {
    /// Slots whose turn needs the world's stores, with what the sweep already did of it (a mask of [`turn`] bits).
    heavy: Vec<(u32, u8)>,
    /// `(dense rank, death)` of bibites whose health went non-finite in their pass-B turn on the pool.
    deaths: Vec<(u32, PendingDeath)>,
    /// `(dense rank, death)` of the bibites pass A found starved.
    starved: Vec<(u32, PendingDeath)>,
    /// `(tag, snapshot)` of the bibites that a mouth-bearing bibite before them in dense order looks at, taken before their turn.
    snapshots: Vec<(u64, Option<TargetSnapshot>)>,
    lanes: PassBLanes,
}

/// Records a pool worker takes through pass A, the brains and pass B together before it moves on to the next block, so that
/// the lines of a record that the three have in common are fetched from memory once and are still in the core's own cache for
/// the next (64 records with their hot lines are well under the 512 KB of a core's L2). Measured at 20,000 bibites, any block
/// from 16 to 1,024 records runs the same: the gain over three passes is in the memory trips and the three fork-joins saved.
const SWEEP_BLOCK: usize = 64;

/// How many stretches of records each worker's share of a sweep over the records is cut into: the stretches are taken by whichever
/// worker is free, so the more of them there are, the less the slowest one holds the others up at the end of the sweep.
const SWEEP_CHUNKS_PER_THREAD: usize = 16;

impl PassBLanes {
    fn add_reasons(&mut self, reasons: u8) {
        self.heavy += 1;
        self.mouth += u64::from(reasons & heavy::MOUTH != 0);
        self.held += u64::from(reasons & heavy::HELD != 0);
        self.vomit += u64::from(reasons & heavy::VOMIT != 0);
        self.eggs += u64::from(reasons & heavy::EGGS != 0);
        self.emit += u64::from(reasons & heavy::EMIT != 0);
    }

    fn merge(&mut self, other: &PassBLanes) {
        self.light += other.light;
        self.heavy += other.heavy;
        self.dead += other.dead;
        self.mouth += other.mouth;
        self.held += other.held;
        self.vomit += other.vomit;
        self.eggs += other.eggs;
        self.emit += other.emit;
        self.head_in_sweep += other.head_in_sweep;
        self.post_on_pool += other.post_on_pool;
    }
}

impl BibiteScratch {
    /// See [`phys::PhysicsScratch::forget_contact_impulses`].
    pub fn forget_contact_impulses(&mut self) {
        self.physics.forget_contact_impulses();
    }

    /// New scratch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the stage times of the body tick and of the physics step into `timings`.
    pub fn drain_clocks(&mut self, timings: &mut crate::sim::PhaseTimings) {
        self.clock.drain_into(timings);
        self.physics.clock.drain_into(timings);
    }

    /// Forgets which collider pairs were touching (call after restoring a
    /// checkpoint or loading a scene into a running world).
    pub fn reset_collision_tracking(&mut self) {
        self.prev_collisions.clear();
        self.cur_collisions.clear();
        self.suppress_collision_damage = true;
    }

    /// The pass-B lane counters since the world was built.
    #[must_use]
    pub fn pass_b_lanes(&self) -> PassBLanes {
        self.lanes
    }

    /// What the resting-pellet layer of the physics bridge did.
    #[must_use]
    pub fn pellet_sleep_stats(&self) -> crate::pellet_sleep::SleepStats {
        self.sleep.stats()
    }

    /// Dense rank of the record `id` names (`u32::MAX` when it has no live record).
    fn rank_of(&self, id: BibiteId) -> u32 {
        self.rank
            .get(id.handle().slot() as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    /// Clears per-tick state.
    pub fn clear(&mut self) {
        self.ids.clear();
        self.bites.clear();
        self.throws.clear();
        self.pending_releases.clear();
        self.deaths.clear();
        self.swallows.clear();
        self.due_ids.clear();
        self.due_slots.clear();
        self.due_inputs.clear();
        self.due_outputs.clear();
    }

    /// Clears only the physics-bridge scratch lists.
    pub fn clear_physics(&mut self) {
        self.physics_bodies.clear();
        self.kept_bibite_bodies = 0;
        self.bibite_body_gens.clear();
        self.bibite_ids.clear();
        self.pellet_ids.clear();
        self.egg_ids.clear();
    }
}

impl BibiteRecord {
    /// Recomputes the derived organ constants for the current geometry.
    pub fn refresh_organs(&mut self, settings: &SimSettings) {
        let area = self.body.geometry.base_body_area;
        let genome = self.genome.clone();
        let rate = genome.metabolic_rate();
        self.stomach
            .set_area(area * genome.organ_area_portion(Gene::StomachWAG));
        self.fat
            .on_growth(genome.organ_area_portion(Gene::FatWAG), area);
        self.armor.update(
            genome.organ_area_portion(Gene::ArmorWAG),
            self.body.geometry.d1_size,
            self.body.geometry.inflate_factor,
            settings.materials.armor.cohesiveness,
        );
        self.body.set_armor_thickness(self.armor.thickness);
        self.egg_layer
            .set_area(area * genome.organ_area_portion(Gene::WombWAG));
        self.mouth_derived = mouth_derived(
            area,
            genome.organ_area_portion(Gene::MouthMusclesWAG),
            area * genome.organ_area_portion(Gene::ThroatWAG),
            rate,
            &mouth_settings(settings),
        );
        self.propulsion_derived = propulsion_derived(
            area * genome.organ_area_portion(Gene::MoveMusclesWAG),
            rate,
            &propulsion_settings(settings),
        );
        self.derived.brain_upkeep = brain_upkeep_per_second(&self.brain, settings);
        self.egg_layer.egg_energy = egg_energy_investment(
            genome.lay_body_energy(settings),
            self.derived.brain_upkeep,
            genome.gene(Gene::SizeRatio),
        );
    }
}

/// What a bibite showing its mouth targets sees of another bibite `r`, whose tag is `tag`.
fn bibite_snapshot(tag: u64, r: &BibiteRecord) -> TargetSnapshot {
    TargetSnapshot {
        tag,
        pos: r.pos,
        radius: r.derived.d1_size * 4.5,
        max_health: r.body.geometry.max_health,
        real_body_area: r.body.geometry.real_body_area,
        internals_radius: r.body.geometry.internals_radius,
        effective_radius: r.body.geometry.effective_radius,
        armor_strength: r.armor.effective_strength(),
        body_length: r.body.geometry.body_length,
        meat_amount: r.body.corpse_meat_energy(
            r.egg_layer.progress * r.egg_layer.egg_energy + r.fat.amount * 30.0,
        ) / 10.0,
        stomach_amount: r.stomach.total_amount(),
        ..TargetSnapshot::default()
    }
}

impl BibiteStore {
    fn snapshot_target(&self, tag: u64, pellets: &PelletStore) -> Option<TargetSnapshot> {
        if let Some(id) = bibite_id_from_tag(tag) {
            let r = self.get(id)?;
            return Some(bibite_snapshot(tag, r));
        }
        if let Some(id) = pellet_id_from_tag(tag) {
            let view = pellets.get(id)?;
            return Some(TargetSnapshot {
                tag,
                is_pellet: true,
                pos: view.pos,
                radius: view.radius,
                amount: view.amount,
                material: match view.material {
                    MaterialKind::Plant => 0,
                    MaterialKind::Meat => 1,
                    _ => 0,
                },
                ..TargetSnapshot::default()
            });
        }
        egg_id_from_tag(tag).map(|_| TargetSnapshot {
            tag,
            is_egg: true,
            ..TargetSnapshot::default()
        })
    }

    /// Per-bibite phase-5 tick (body spec §2.3/§2.6).
    ///
    /// Two passes so the brain cadence can run as one batch (optionally on
    /// worker threads): pass A does energy/metabolism and assembles
    /// `BrainInputs` for due records; the batch steps every due brain; pass B
    /// runs organs/movement/mouth/growth/death in dense order, reading live
    /// brain node values. Serial and parallel batches are bit-identical.
    pub fn tick(&mut self, ctx: &mut BibiteTickCtx<'_>) {
        self.tick_inner(ctx, None);
    }

    /// Per-bibite phase-5 tick with a world-owned executor.
    ///
    /// Pass A (metabolism/cadence) runs over disjoint record chunks when the
    /// executor is parallel; due-brain collection is sorted by slot afterwards
    /// and deaths are merged in dense order, so the result is bit-identical to
    /// [`BibiteStore::tick`]. Pass B remains serial because organ/mouth effects
    /// touch shared pellet/egg stores and event order.
    pub fn tick_with_executor(
        &mut self,
        ctx: &mut BibiteTickCtx<'_>,
        executor: crate::sim::ParallelExecutor<'_>,
    ) {
        self.tick_inner(ctx, Some(executor));
    }

    fn tick_inner(
        &mut self,
        ctx: &mut BibiteTickCtx<'_>,
        executor: Option<crate::sim::ParallelExecutor<'_>>,
    ) {
        ctx.scratch.population = self.len();
        self.epoch = self.epoch.wrapping_add(1).max(1);
        let mut lap = std::time::Instant::now();
        self.resolve_pending_deaths(ctx.pellets, ctx.settings, ctx.rng, ctx.events);
        ctx.scratch
            .clock
            .lap(crate::sim::Stage::BodyDeaths, &mut lap);
        ctx.scratch.clear();
        let bs = BodySettings::from_sim_settings(ctx.settings);
        let dt = 1.0 / ctx.settings.sim_tps.max(1) as f32;
        let plant_material = organ_material(&ctx.settings.materials.plant);
        let meat_material = organ_material(&ctx.settings.materials.meat);
        let fat_material = organ_material(&ctx.settings.materials.fat);
        let armor_material = organ_material(&ctx.settings.materials.armor);
        let env = OrganEnv {
            settings: ctx.settings,
            zones: ctx.zones,
            bs: &bs,
            dt,
            plant: &plant_material,
            meat: &meat_material,
            fat_params: &fat_material,
            armor_params: &armor_material,
            epoch: self.epoch,
        };
        // A world on the pool takes pass A, the brains and pass B through the records in one sweep (see `fused_body_tick`); the
        // others run the three passes one after the other, in dense order.
        let parallel = executor.is_some_and(|executor| {
            executor.is_parallel()
                && crate::sim::ParallelGates::workers_for(
                    self.records.len(),
                    self.gates.sweep,
                    executor.threads(),
                ) > 1
        });
        if parallel {
            let executor = executor.expect("parallel executor");
            self.fused_body_tick(ctx, &env, executor, &mut lap);
        } else {
            // The serial passes walk the ids of the start of the tick (the sweep takes slots, and ranks for the order).
            ctx.scratch.ids.extend(self.ids());
            let id_count = ctx.scratch.ids.len();
            for index in 0..id_count {
                let id = ctx.scratch.ids[index];
                self.tick_brain_pre(id, ctx);
            }
            ctx.scratch
                .clock
                .lap(crate::sim::Stage::BodyPassA, &mut lap);
            self.assemble_due_brain_inputs(ctx, executor);
            ctx.scratch
                .clock
                .lap(crate::sim::Stage::BodyAssemble, &mut lap);
            self.step_due_brains(ctx);
            ctx.scratch
                .clock
                .lap(crate::sim::Stage::BodyBrains, &mut lap);
            for index in 0..id_count {
                let id = ctx.scratch.ids[index];
                self.tick_brain_post(id, ctx, &env);
            }
        }
        ctx.scratch
            .clock
            .lap(crate::sim::Stage::BodyPassB, &mut lap);
        let bite_count = ctx.scratch.bites.len();
        for index in 0..bite_count {
            let bite = ctx.scratch.bites[index];
            self.resolve_bite(bite, ctx.pellets, ctx.rng, ctx.settings, ctx.events);
        }
        let throws = std::mem::take(&mut ctx.scratch.throws);
        for throw in &throws {
            self.resolve_throw(throw, ctx.pellets, ctx.settings);
        }
        ctx.scratch.throws = throws;
        let swallow_count = ctx.scratch.swallows.len();
        for index in 0..swallow_count {
            let swallow = ctx.scratch.swallows[index];
            self.resolve_swallow(swallow, ctx);
        }
        let death_count = ctx.scratch.deaths.len();
        for index in 0..death_count {
            let death = ctx.scratch.deaths[index];
            self.resolve_death(death, ctx);
        }
        ctx.scratch
            .clock
            .lap(crate::sim::Stage::BodyTails, &mut lap);
    }

    /// The body tick of a world on the pool, with the same result as pass A, the brains and pass B one after the other in
    /// dense order.
    ///
    /// Pass A, the brain steps and the part of pass B that touches nothing but the bibite itself each read and write one
    /// record (see [`OrganShared`] and [`pass_b_heavy_reasons`]), so a worker takes its stretch of records through all three
    /// block by block: the lines of a record that the three have in common are pulled in from memory once and are in the core's
    /// own cache for the others. What crosses records is kept as it was:
    /// - a bibite's turn changes its own record and, only when it has something in its mouth or held, lays an egg, vomits or
    ///   emits pheromone, the world's stores; it reads another bibite only through its mouth targets. The turns that reach no
    ///   store run in the sweep, in any order; the others run afterwards in dense order ([`Self::pass_b_dense`]), exactly as
    ///   they would have among the rest;
    /// - what a dense-order turn reads of the others must be what the serial order shows it: a target before it in dense order
    ///   has had its turn in both orders (live read); a target after it has not had it yet in the serial order but may have had
    ///   it in the sweep, so its state after pass A and its brain step and before its turn is taken in the sweep, by the worker
    ///   that has the target's record (a bibite is only a mouth target of another when they touch, so there are few), and
    ///   kept in `pre_snapshots`; pellets and eggs are only changed by dense-order turns (live read);
    /// - the starvation deaths pass A finds are queued in dense order, and then those of pass B (in the sweep or not), in dense
    ///   order.
    fn fused_body_tick(
        &mut self,
        ctx: &mut BibiteTickCtx<'_>,
        env: &OrganEnv<'_>,
        executor: crate::sim::ParallelExecutor<'_>,
        lap: &mut std::time::Instant,
    ) {
        // Dense rank of every slot: the order the serial passes visit the bibites in (a copy: the dense-order turns may remove
        // records, and the ranks are those of the start of the tick).
        self.copy_dense_ranks(&mut ctx.scratch.rank);
        let workers = crate::sim::ParallelGates::workers_for(
            self.records.len(),
            self.gates.sweep,
            executor.threads(),
        );
        let chunk_len = crate::sim::ParallelGates::chunk_len(
            self.records.len(),
            self.gates.sweep,
            workers,
            SWEEP_CHUNKS_PER_THREAD,
        );
        // The bibites with something in their mouth, in slot order (almost none have), and the bibite targets among theirs
        // that come after them in dense order: those are the ones whose state before their own turn is wanted.
        let mut mouth_lane = std::mem::take(&mut ctx.scratch.mouth_lane);
        mouth_lane.clear();
        if self.mouth_candidates_valid {
            // The sweep after the last step listed the bibites with mouth targets (and nothing gives a bibite any since): the
            // list is for this tick only, and each record is looked at again, as the scan would have.
            for &slot in &self.mouth_candidates {
                if self
                    .records
                    .get(slot as usize)
                    .and_then(|entry| entry.as_ref())
                    .is_some_and(|record| record.n_mouth_targets > 0)
                {
                    mouth_lane.push(slot);
                }
            }
            #[cfg(test)]
            {
                let scanned: Vec<u32> = self
                    .records
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.as_ref().is_some_and(|r| r.n_mouth_targets > 0))
                    .map(|(slot, _)| slot as u32)
                    .collect();
                assert_eq!(
                    mouth_lane, scanned,
                    "the list of bibites with mouth targets differs from a scan over every record"
                );
            }
        } else {
            let lanes: Vec<Vec<u32>> = self
                .records
                .par_chunks(chunk_len)
                .enumerate()
                .map(|(index, chunk)| {
                    let base = index * chunk_len;
                    chunk
                        .iter()
                        .enumerate()
                        .filter_map(|(offset, entry)| {
                            entry
                                .as_ref()
                                .filter(|record| record.n_mouth_targets > 0)
                                .map(|_| (base + offset) as u32)
                        })
                        .collect()
                })
                .collect();
            for lane in lanes {
                mouth_lane.extend(lane);
            }
        }
        self.mouth_candidates_valid = false;
        let mut wanted: Vec<u64> = Vec::new();
        let mut snapshot_slots = std::mem::take(&mut ctx.scratch.snapshot_slots);
        snapshot_slots.clear();
        snapshot_slots.resize(self.records.len(), false);
        let mut viewed_slots = std::mem::take(&mut ctx.scratch.viewed_slots);
        viewed_slots.clear();
        viewed_slots.resize(self.records.len(), false);
        for &slot in &mouth_lane {
            let Some(record) = self
                .records
                .get(slot as usize)
                .and_then(|entry| entry.as_ref())
            else {
                continue;
            };
            let own_rank = ctx.scratch.rank_of(record.id);
            for &tag in &record.mouth_targets[..record.n_mouth_targets as usize] {
                let Some(target) = bibite_id_from_tag(tag) else {
                    continue;
                };
                if ctx.scratch.rank_of(target) <= own_rank {
                    // A target before the viewer in dense order is read as it stands after its own turn (a target after it is read
                    // from the snapshot the sweep took before that turn), so that turn must be whole when the viewer's comes.
                    if let Some(flag) = viewed_slots.get_mut(target.handle().slot() as usize) {
                        *flag = true;
                    }
                }
                if ctx.scratch.rank_of(target) > own_rank {
                    wanted.push(tag);
                    if let Some(flag) = snapshot_slots.get_mut(target.handle().slot() as usize) {
                        *flag = true;
                    }
                }
            }
        }
        wanted.sort_unstable();
        wanted.dedup();

        // The sweep.
        let settings = ctx.settings;
        let period = settings.brain_tps as f32 / settings.sim_tps.max(1) as f32;
        let rank: &[u32] = &ctx.scratch.rank;
        let wanted_tags: &[u64] = &wanted;
        let wanted_slots: &[bool] = &snapshot_slots;
        let split_turns = split_turns_enabled();
        let parts: Vec<PoolPart> = self
            .records
            .par_chunks_mut(chunk_len)
            .map(|chunk| {
                let mut part = PoolPart::default();
                let mut due: Vec<usize> = Vec::with_capacity(SWEEP_BLOCK);
                for block in chunk.chunks_mut(SWEEP_BLOCK) {
                    // Pass A: metabolism and upkeep, the cadences, who is due for a brain step.
                    due.clear();
                    for (offset, entry) in block.iter_mut().enumerate() {
                        let Some(record) = entry.as_mut() else {
                            continue;
                        };
                        let (is_due, death) = Self::tick_brain_pre_record(record, settings);
                        if is_due {
                            due.push(offset);
                        }
                        if let Some(death) = death {
                            part.starved
                                .push((rank[record.id.handle().slot() as usize], death));
                        }
                    }
                    // The brains that are due.
                    for &offset in &due {
                        if let Some(record) = block[offset].as_mut() {
                            let inputs = Self::assemble_record_inputs(record);
                            let _ = record.brain.step(&inputs, period);
                        }
                    }
                    // Pass B for the turns that touch nothing but their own record.
                    for entry in block.iter_mut() {
                        let Some(record) = entry.as_mut() else {
                            continue;
                        };
                        let slot = record.id.handle().slot();
                        if wanted_slots.get(slot as usize).copied().unwrap_or(false) {
                            let tag = bibite_tag(record.id);
                            if wanted_tags.binary_search(&tag).is_ok() {
                                part.snapshots
                                    .push((tag, Some(bibite_snapshot(tag, record))));
                            }
                        }
                        if record.body.dead {
                            part.heavy.push((slot, 0));
                            part.lanes.dead += 1;
                            part.lanes.heavy += 1;
                            continue;
                        }
                        if !record.born || !record.is_ready {
                            continue;
                        }
                        let outputs = record.brain.output_values_copy();
                        let reasons = pass_b_heavy_reasons(record, &outputs, env.settings, env.dt);
                        if reasons != 0 {
                            let mut done = 0;
                            // The head of the turn (the counters, the stomach, the womb, the fat, the movement, the mouth's latching of
                            // the outputs) touches nothing but this record when no egg is to be laid, so it is taken here whatever else
                            // the turn will do. (A revival step comes before it in the dense order: those turns stay whole.)
                            if split_turns
                                && reasons & heavy::EGGS == 0
                                && !(record.body.corpse_decay > 0.0)
                            {
                                pass_b_head(record, &mut LocalOnly, env, &outputs);
                                part.lanes.head_in_sweep += 1;
                                done = turn::HEAD_DONE;
                                if reasons & heavy::EMIT != 0 {
                                    done |= turn::EMITS;
                                }
                            }
                            part.heavy.push((slot, done));
                            part.lanes.add_reasons(reasons);
                            continue;
                        }
                        part.lanes.light += 1;
                        if pass_b_live(record, &mut LocalOnly, env, &outputs, &[]) {
                            part.deaths.push((
                                rank[slot as usize],
                                PendingDeath {
                                    id: record.id,
                                    cause: DeathCause::Starvation,
                                    swallowed: false,
                                },
                            ));
                        }
                    }
                }
                part
            })
            .collect();
        let mut heavy_slots: Vec<(u32, u8)> = Vec::new();
        let mut pool_deaths: Vec<(u32, PendingDeath)> = Vec::new();
        let mut starved: Vec<(u32, PendingDeath)> = Vec::new();
        let mut snapshots: Vec<(u64, Option<TargetSnapshot>)> = Vec::new();
        for part in parts {
            heavy_slots.extend(part.heavy);
            pool_deaths.extend(part.deaths);
            starved.extend(part.starved);
            snapshots.extend(part.snapshots);
            ctx.scratch.lanes.merge(&part.lanes);
        }
        ctx.scratch.lanes.ticks += 1;
        // Pass A's starvation deaths, in dense order.
        starved.sort_unstable_by_key(|(rank, _)| *rank);
        ctx.scratch
            .deaths
            .extend(starved.into_iter().map(|(_, death)| death));
        // A first look at every bibite target that comes after its viewer: the sweep took the state of the ones that exist, and
        // the others (gone, or a slot another bibite has) read as nothing.
        snapshots.sort_unstable_by_key(|(tag, _)| *tag);
        let mut pre = std::mem::take(&mut ctx.scratch.pre_snapshots);
        pre.clear();
        for &tag in &wanted {
            match snapshots.binary_search_by_key(&tag, |(entry, _)| *entry) {
                Ok(index) => pre.push(snapshots[index]),
                Err(_) => pre.push((tag, None)),
            }
        }
        ctx.scratch.pre_snapshots = pre;
        ctx.scratch.mouth_lane = mouth_lane;
        ctx.scratch.snapshot_slots = snapshot_slots;
        ctx.scratch.viewed_slots = viewed_slots;
        ctx.scratch.clock.lap(crate::sim::Stage::BodySweep, lap);
        self.pass_b_dense(ctx, env, executor, heavy_slots, pool_deaths);
    }

    /// The pass-B turns that need the world's stores, in dense order (see [`Self::fused_body_tick`]), and the starvation
    /// deaths of the pass-B turns that ran in the sweep merged with theirs in dense order.
    fn pass_b_dense(
        &mut self,
        ctx: &mut BibiteTickCtx<'_>,
        env: &OrganEnv<'_>,
        executor: crate::sim::ParallelExecutor<'_>,
        mut heavy_slots: Vec<(u32, u8)>,
        mut pool_deaths: Vec<(u32, PendingDeath)>,
    ) {
        heavy_slots.sort_unstable_by_key(|(slot, _)| ctx.scratch.rank[*slot as usize]);
        let first_death = ctx.scratch.deaths.len();
        // The turns whose head the sweep took have only the mouth left that reaches the world's stores. With enough of them the pool
        // takes the rest of those turns too, after this stretch (the growth, the healing, the clock, the propulsion bookkeeping: one
        // record each), except for the bibites a later turn of this stretch may look at (they are whole when it comes) and the ones
        // that have a pheromone puff to make.
        let split_turns = heavy_slots
            .iter()
            .filter(|(_, done)| done & turn::HEAD_DONE != 0)
            .count();
        let leave_to_pool = executor.is_parallel() && split_turns >= self.gates.brains.max(1);
        let mut deferred: Vec<u32> = Vec::new();
        ctx.scratch.split = true;
        for position in 0..heavy_slots.len() {
            // The records of the turns a few places ahead are asked for now: these are the bibites with something in their
            // mouths, a few in a hundred, and their records are in memory (the sweep went through all the others since).
            for ahead in [position + 3, position + 6] {
                if let Some(entry) = heavy_slots
                    .get(ahead)
                    .and_then(|(slot, _)| self.records.get(*slot as usize))
                {
                    prefetch_turn((entry as *const Option<BibiteRecord>).cast::<u8>());
                }
            }
            let (slot, done) = heavy_slots[position];
            let Some(id) = self
                .records
                .get(slot as usize)
                .and_then(|entry| entry.as_ref())
                .map(|record| record.id)
            else {
                continue;
            };
            if done & turn::HEAD_DONE != 0 {
                let leave = leave_to_pool
                    && done & turn::EMITS == 0
                    && !ctx
                        .scratch
                        .viewed_slots
                        .get(slot as usize)
                        .copied()
                        .unwrap_or(true);
                if self.tick_brain_post_split(id, ctx, env, leave) {
                    deferred.push(slot);
                }
            } else {
                self.tick_brain_post(id, ctx, env);
            }
        }
        ctx.scratch.split = false;
        if !deferred.is_empty() {
            // The rest of the left turns, on the pool: a worker takes a stretch of the slots and does the turns of the left ones in it.
            deferred.sort_unstable();
            let workers = crate::sim::ParallelGates::workers_for(
                deferred.len(),
                self.gates.brains.max(1),
                executor.threads(),
            );
            let chunk_len = self.records.len().div_ceil(workers.max(1) * 4).max(1);
            let rank: &[u32] = &ctx.scratch.rank;
            let slots: &[u32] = &deferred;
            let parts: Vec<Vec<(u32, PendingDeath)>> = self
                .records
                .par_chunks_mut(chunk_len)
                .enumerate()
                .map(|(index, chunk)| {
                    let base = index * chunk_len;
                    let first = slots.partition_point(|slot| (*slot as usize) < base);
                    let end = slots.partition_point(|slot| (*slot as usize) < base + chunk.len());
                    let mut deaths = Vec::new();
                    for &slot in &slots[first..end] {
                        let Some(record) = chunk[slot as usize - base].as_mut() else {
                            continue;
                        };
                        let outputs = record.brain.output_values_copy();
                        let up = crate::math::unity::transform_up(record.rot);
                        if pass_b_post(record, &mut LocalOnly, env, &outputs, up) {
                            deaths.push((
                                rank[slot as usize],
                                PendingDeath {
                                    id: record.id,
                                    cause: DeathCause::Starvation,
                                    swallowed: false,
                                },
                            ));
                        }
                    }
                    deaths
                })
                .collect();
            for part in parts {
                pool_deaths.extend(part);
            }
            ctx.scratch.lanes.post_on_pool += deferred.len() as u64;
        }
        if !pool_deaths.is_empty() {
            let dense_deaths: Vec<PendingDeath> = ctx.scratch.deaths.drain(first_death..).collect();
            pool_deaths.extend(
                dense_deaths
                    .into_iter()
                    .map(|death| (ctx.scratch.rank_of(death.id), death)),
            );
            pool_deaths.sort_unstable_by_key(|(rank, _)| *rank);
            ctx.scratch
                .deaths
                .extend(pool_deaths.into_iter().map(|(_, death)| death));
        }
    }

    /// Pass A for one record. Returns `(due_for_brain, starvation_death)`.
    fn tick_brain_pre_record(
        record: &mut BibiteRecord,
        settings: &SimSettings,
    ) -> (bool, Option<PendingDeath>) {
        let dt = 1.0 / settings.sim_tps.max(1) as f32;
        if record.body.dead || !record.born || !record.is_ready {
            return (false, None);
        }
        // `UseEnergy((metabolism + brain.brainUpkeepCost) * Time.fixedDeltaTime)`: the argument is one double expression rounded once.
        let upkeep = ((f64::from(record.body.metabolism) + f64::from(record.derived.brain_upkeep)) * f64::from(dt)) as f32;
        // A new frame of the game: the `Hurting` calls of the last one are over (see `BodyState::death_hurts`).
        record.body.death_hurts = 0;
        let spend = record.body.use_energy(upkeep, true);
        let mut death = None;
        if spend.died {
            death = Some(PendingDeath {
                id: record.id,
                cause: DeathCause::Starvation,
                swallowed: false,
            });
        }
        record.throttle.brain_counter += 1;
        let mut due = false;
        // The game SUBTRACTS the period (`brainTicksCount -= brainTickFactor`, `visionLookupCount -= visionLookupPeriod`, `visionSensingCount -= visionSensingPeriod`), so a counter
        // that is above its period (the setting was lowered while the world ran) stays above it; resetting to 0 gave the same schedule only while the counters were below it.
        let brain_period = settings.brain_tps.max(1);
        if record.throttle.brain_counter >= brain_period {
            record.throttle.brain_counter -= brain_period;
            record.throttle.lookup_counter += 1;
            let lookup_period = settings.vision_lookup_updates.max(1);
            if record.throttle.lookup_counter >= lookup_period {
                record.throttle.lookup_counter -= lookup_period;
            }
            record.throttle.sense_counter += 1;
            let sense_period = settings.vision_sense_updates.max(1);
            if record.throttle.sense_counter >= sense_period {
                record.throttle.sense_counter -= sense_period;
            }
            due = true;
        }
        (due, death)
    }

    /// Pass A: base metabolism/upkeep spend, sense-throttle cadence and due
    /// collection. No organ/movement/energy-side cross-record effects.
    #[inline]
    fn tick_brain_pre(&mut self, id: BibiteId, ctx: &mut BibiteTickCtx<'_>) {
        let Some(record) = self.get_mut(id) else {
            return;
        };
        let (due, death) = Self::tick_brain_pre_record(record, ctx.settings);
        if due {
            ctx.scratch.due_ids.push(id);
        }
        if let Some(death) = death {
            ctx.scratch.deaths.push(death);
        }
    }

    /// Builds `BrainInputs` for every due record in slot order, rebuilding any
    /// stale execution plan on the owning thread.
    fn assemble_due_brain_inputs(
        &mut self,
        ctx: &mut BibiteTickCtx<'_>,
        executor: Option<crate::sim::ParallelExecutor<'_>>,
    ) {
        let mut ids = std::mem::take(&mut ctx.scratch.due_ids);
        ids.sort_unstable_by_key(|id| id.handle().slot());
        ctx.scratch.due_slots.clear();
        ctx.scratch.due_inputs.clear();
        let workers = crate::sim::ParallelGates::workers_for(
            ids.len(),
            self.gates.due_brains,
            executor.map_or(1, |executor| executor.threads()),
        );
        if workers > 1 {
            // Each due record only reads and rebuilds itself: cut the slots into stretches, one per worker, and join the
            // stretches' inputs in slot order (the order the serial loop has).
            let chunk_len = self.records.len().div_ceil(workers).max(1);
            let due: &[BibiteId] = &ids;
            let parts: Vec<(Vec<u32>, Vec<BrainInputs>)> = self
                .records
                .par_chunks_mut(chunk_len)
                .enumerate()
                .map(|(index, chunk)| {
                    let start = index * chunk_len;
                    let end = start + chunk.len();
                    let first = due.partition_point(|id| (id.handle().slot() as usize) < start);
                    let last = due.partition_point(|id| (id.handle().slot() as usize) < end);
                    let mut slots = Vec::with_capacity(last - first);
                    let mut inputs = Vec::with_capacity(last - first);
                    for id in &due[first..last] {
                        let slot = id.handle().slot() as usize;
                        let Some(record) = chunk[slot - start].as_mut() else {
                            continue;
                        };
                        slots.push(slot as u32);
                        inputs.push(Self::assemble_record_inputs(record));
                    }
                    (slots, inputs)
                })
                .collect();
            for (slots, inputs) in parts {
                ctx.scratch.due_slots.extend(slots);
                ctx.scratch.due_inputs.extend(inputs);
            }
        } else {
            for id in &ids {
                let slot = id.handle().slot() as usize;
                let Some(record) = self.records.get_mut(slot).and_then(|entry| entry.as_mut())
                else {
                    continue;
                };
                ctx.scratch.due_slots.push(slot as u32);
                ctx.scratch
                    .due_inputs
                    .push(Self::assemble_record_inputs(record));
            }
        }
        ctx.scratch.due_ids = ids;
    }

    /// The brain inputs of one due record (its brain's plan is rebuilt first when it changed).
    fn assemble_record_inputs(record: &mut BibiteRecord) -> BrainInputs {
        if record.brain.plan_dirty() {
            record.brain.rebuild();
        }
        let mut inputs = BrainInputs::new();
        inputs.write_internal(record);
        record.senses.write_into(&mut inputs);
        inputs
    }

    /// Steps all due brains with one `Brain::step_batch` call (parallel above
    /// the gate). Results are independent of chunking/thread count.
    fn step_due_brains(&mut self, ctx: &mut BibiteTickCtx<'_>) {
        let due_count = ctx.scratch.due_slots.len();
        if due_count == 0 {
            return;
        }
        let period = ctx.settings.brain_tps as f32 / ctx.settings.sim_tps.max(1) as f32;
        ctx.scratch.due_outputs.clear();
        ctx.scratch
            .due_outputs
            .resize(due_count, BrainOutputs::default());
        let configured = self.brain_threads;
        let threads = if configured == 0 {
            std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1)
        } else {
            configured
        };
        let chunk_count =
            crate::sim::ParallelGates::workers_for(due_count, self.gates.brains, threads);
        if chunk_count > 1 && self.records.len() > 1 {
            let chunk_len = self.records.len().div_ceil(chunk_count).max(1);
            let inputs = std::mem::take(&mut ctx.scratch.due_inputs);
            let mut outputs = std::mem::take(&mut ctx.scratch.due_outputs);
            let slots = std::mem::take(&mut ctx.scratch.due_slots);
            let records = &mut self.records;
            rayon::scope(|scope| {
                let mut in_rest: &[BrainInputs] = &inputs;
                let mut out_rest: &mut [BrainOutputs] = &mut outputs;
                let mut slot_cursor = 0usize;
                let mut slot_start = 0usize;
                for chunk in records.chunks_mut(chunk_len) {
                    let slot_end = slot_start + chunk.len();
                    let d0 = slots.partition_point(|slot| (*slot as usize) < slot_start);
                    let d1 = slots.partition_point(|slot| (*slot as usize) < slot_end);
                    let take = d1 - d0;
                    if take > 0 {
                        let (inputs_part, in_next) = in_rest.split_at(take);
                        in_rest = in_next;
                        let (outputs_part, out_next) = out_rest.split_at_mut(take);
                        out_rest = out_next;
                        let slots_part = &slots[slot_cursor..slot_cursor + take];
                        slot_cursor += take;
                        let chunk_base = slot_start;
                        scope.spawn(move |_| {
                            let mut brains: Vec<&mut Brain> = Vec::with_capacity(inputs_part.len());
                            for (offset, entry) in chunk.iter_mut().enumerate() {
                                let slot = (chunk_base + offset) as u32;
                                if slots_part.binary_search(&slot).is_ok() {
                                    if let Some(record) = entry.as_mut() {
                                        brains.push(&mut record.brain);
                                    }
                                }
                            }
                            Brain::step_batch(&mut brains, inputs_part, period, outputs_part);
                        });
                    }
                    slot_start = slot_end;
                }
            });
            ctx.scratch.due_inputs = inputs;
            ctx.scratch.due_outputs = outputs;
            ctx.scratch.due_slots = slots;
        } else {
            let BibiteScratch {
                due_slots,
                due_inputs,
                due_outputs,
                ..
            } = &mut ctx.scratch;
            let mut brains: Vec<&mut Brain> = Vec::with_capacity(due_count);
            for (slot, entry) in self.records.iter_mut().enumerate() {
                if due_slots.binary_search(&(slot as u32)).is_ok() {
                    if let Some(record) = entry.as_mut() {
                        brains.push(&mut record.brain);
                    }
                }
            }
            Brain::step_batch(&mut brains, due_inputs, period, due_outputs);
        }
    }

    /// Pass B: corpse decay/removal, organ updates, clock, NaN check and
    /// propulsion kinematics, in dense iteration order.
    fn tick_brain_post(&mut self, id: BibiteId, ctx: &mut BibiteTickCtx<'_>, env: &OrganEnv<'_>) {
        let (bs, dt) = (env.bs, env.dt);
        {
            let Some(record) = self.get_mut(id) else {
                return;
            };
            if record.body.dead {
                // `BibiteEggLayingOrgan.OnDeathStatusChange` (cs:215-231):
                // completed eggs are laid when the bibite dies, subject to
                // the same population limit.
                // `dying` is true only on the first tick after death (the corpse tick
                // below clears it), which is the single `OnDeathStatusChange` moment.
                if record.body.dying
                    && record.born
                    && record.egg_layer.n_eggs() > 0
                    && !birth_limit_reached(ctx)
                {
                    let count = record.egg_layer.n_eggs();
                    let laid = lay_eggs(record, ctx, count);
                    if laid > 0 {
                        record.egg_layer.on_eggs_laid(laid);
                    }
                }
                let tick = record.body.corpse_tick(dt, bs);
                if tick.exploded {
                    let extra = record.egg_layer.progress * record.egg_layer.egg_energy
                        + record.fat.amount * ctx.settings.materials.fat.energy_density;
                    let energy = record.body.corpse_meat_energy(extra);
                    let pos = record.pos;
                    let radius = record.body.geometry.body_length / 2.0;
                    let min_size = ctx.settings.scenario.min_pellet_size;
                    let _ = ctx
                        .pellets
                        .spawn_meat_pile_with_radius(pos, energy, radius, ctx.rng, min_size);
                    spill_stomach(record, ctx.pellets, ctx.settings);
                    let count = (record.body.geometry.base_body_area as i32) / 20;
                    self.log_blood(BloodBurst { pos, radius, count });
                    self.remove(id);
                }
                return;
            }
            if !record.born || !record.is_ready {
                return;
            }
            // `Revival`'s 0.1 s steps (a live bibite with a corpse decay left is being revived).
            if record.body.corpse_decay > 0.0 {
                let now = record.clock.time_alive;
                if ((now - dt) * 10.0).floor() != (now * 10.0).floor() {
                    record.revival_step();
                }
            }
        }
        let Some((snapshots, count)) = self.mouth_snapshots(id, ctx) else {
            return;
        };
        let Some(record) = self.get_mut(id) else {
            return;
        };
        let outputs = record.brain.output_values_copy();
        let non_finite = pass_b_live(
            record,
            &mut CtxShared(&mut *ctx),
            env,
            &outputs,
            &snapshots[..count],
        );
        if non_finite {
            ctx.scratch.deaths.push(PendingDeath {
                id,
                cause: DeathCause::Starvation,
                swallowed: false,
            });
        }
    }

    /// What the bibite `id` sees of each of its mouth targets at its turn, nearest first, and how many there are.
    fn mouth_snapshots(
        &mut self,
        id: BibiteId,
        ctx: &mut BibiteTickCtx<'_>,
    ) -> Option<([TargetSnapshot; 10], usize)> {
        let (targets, count) = match self.get(id) {
            Some(record) => (record.mouth_targets, record.n_mouth_targets as usize),
            None => return None,
        };
        let mut snapshots = [TargetSnapshot::default(); 10];
        for index in 0..count {
            snapshots[index] = self
                .target_snapshot(id, targets[index], ctx)
                .unwrap_or_default();
            #[cfg(test)]
            ctx.scratch.snapshot_log.push((
                bibite_tag(id),
                targets[index],
                format!("{:?}", snapshots[index]),
            ));
        }
        // `BibiteMouth.cs:359-394`: `objectsInMouth` is sorted by distance
        // from the mouth transform (the bibite origin) every update, so the
        // nearest target is bitten/swallowed first. Stable sort keeps
        // trigger order as the tie-break.
        let origin = self.get(id)?.pos;
        snapshots[..count].sort_by(|a, b| a.pos.distance(origin).total_cmp(&b.pos.distance(origin)));
        Some((snapshots, count))
    }

    /// The dense-order part of a heavy turn whose head the sweep took ([`turn::HEAD_DONE`]): the mouth's work with the world's stores,
    /// and then the rest of the turn, unless `leave` says the pool takes that after the dense-order stretch ([`Self::pass_b_dense`]).
    /// Returns whether it was left.
    fn tick_brain_post_split(
        &mut self,
        id: BibiteId,
        ctx: &mut BibiteTickCtx<'_>,
        env: &OrganEnv<'_>,
        leave: bool,
    ) -> bool {
        let Some((snapshots, count)) = self.mouth_snapshots(id, ctx) else {
            return false;
        };
        let Some(record) = self.get_mut(id) else {
            return false;
        };
        let outputs = record.brain.output_values_copy();
        organs_hooks(record, &mut CtxShared(&mut *ctx), &outputs, &snapshots[..count]);
        if leave {
            return true;
        }
        let up = crate::math::unity::transform_up(record.rot);
        if pass_b_post(record, &mut CtxShared(&mut *ctx), env, &outputs, up) {
            ctx.scratch.deaths.push(PendingDeath {
                id,
                cause: DeathCause::Starvation,
                swallowed: false,
            });
        }
        false
    }

    /// The snapshot of a mouth target as the serial pass B sees it when `viewer` takes its turn. In the split pass B the
    /// bibites that touch nothing but themselves have already had their turn, so a bibite target that comes after the viewer in
    /// dense order is read from `pre_snapshots` (taken before any turn); everything else is read live: the targets before the
    /// viewer have had their turn in both orders, the pellets and eggs are only changed by dense-order turns.
    fn target_snapshot(
        &self,
        viewer: BibiteId,
        tag: u64,
        ctx: &BibiteTickCtx<'_>,
    ) -> Option<TargetSnapshot> {
        if ctx.scratch.split {
            if let Some(target) = bibite_id_from_tag(tag) {
                if ctx.scratch.rank_of(target) > ctx.scratch.rank_of(viewer) {
                    return match ctx
                        .scratch
                        .pre_snapshots
                        .binary_search_by_key(&tag, |(entry, _)| *entry)
                    {
                        Ok(index) => ctx.scratch.pre_snapshots[index].1,
                        Err(_) => {
                            lane_violation("a target nobody took a first snapshot of");
                            self.snapshot_target(tag, ctx.pellets)
                        }
                    };
                }
            }
        }
        self.snapshot_target(tag, ctx.pellets)
    }

    /// `BibiteBody.ExplodeToMeat` (cs:847-863): bursts a body into a meat
    /// pile, spills its stomach and destroys it. Returns `false` if the id is
    /// unknown.
    pub fn explode_corpse(
        &mut self,
        id: BibiteId,
        pellets: &mut PelletStore,
        rng: &mut crate::rng::Rng,
        settings: &SimSettings,
    ) -> bool {
        let Some(target) = self.get(id) else {
            return false;
        };
        let extra = target.egg_layer.progress * target.egg_layer.egg_energy
            + target.fat.amount * settings.materials.fat.energy_density;
        let energy = target.body.corpse_meat_energy(extra);
        let radius = target.body.geometry.body_length / 2.0;
        let _ = pellets.spawn_meat_pile_with_radius(
            target.pos,
            energy,
            radius,
            rng,
            settings.scenario.min_pellet_size,
        );
        spill_stomach(target, pellets, settings);
        let burst = BloodBurst {
            pos: target.pos,
            radius: target.body.geometry.body_length / 2.0,
            count: (target.body.geometry.base_body_area as i32) / 20,
        };
        self.log_blood(burst);
        self.remove(id);
        true
    }

    /// `BibiteMouth.ReleaseAndMaybeThrowAllHeldObjects` (cs:777-810), the throwing half: the force comes from the mass of everything held
    /// (`links.Sum(mass)`), every held body gets `up * force / nHeld` as an impulse (its velocity changes by impulse / mass, on top of the
    /// velocity it has), and the holder is pushed back by the whole force when the grab is below -0.25.
    fn resolve_throw(&mut self, throw: &PendingThrow, pellets: &mut PelletStore, settings: &SimSettings) {
        let masses: Vec<(BibiteId, f32)> = throw
            .prey
            .iter()
            .filter_map(|id| self.get(*id).map(|record| (*id, record.current_mass(settings).max(0.001))))
            .collect();
        let pellet_masses: Vec<(PelletId, f32, PelletPose)> = throw
            .pellets
            .iter()
            .filter_map(|id| {
                let view = pellets.get(*id)?;
                let density = pellets.materials().get(view.material).mass_density;
                Some((*id, (view.amount * density).max(0.000_1), pellets.pose(*id)?))
            })
            .collect();
        let total: f32 = masses.iter().map(|(_, mass)| *mass).sum::<f32>() + pellet_masses.iter().map(|(_, mass, _)| *mass).sum::<f32>();
        if total <= 0.0 {
            return;
        }
        let force = crate::organs::throw_force_magnitude(throw.throwing_force_factor, throw.jaw_strength, throw.desire, total, throw.jaw_mass);
        // The game's arithmetic overflows for a mass that big (a held wall of the maze save weighs infinity, plant density 1e35: infinity over infinity is NaN, and a few
        // thousand times the jaw's strength of mass makes the product infinite, `BibiteMouth.cs:792`): a force that is not a number moves nothing here (what Unity's
        // `AddForce` does with one was not found out). Added to the velocities it made them NaN, and a bibite with a NaN pose dies as a hazard.
        if !force.is_finite() {
            return;
        }
        for (id, mass) in masses {
            if let Some(record) = self.get_mut(id) {
                record.vel += throw.up * (force / throw.n_held / mass);
            }
        }
        for (id, mass, pose) in pellet_masses {
            let _ = pellets.set_pose(id, PelletPose { velocity: pose.velocity + throw.up * (force / throw.n_held / mass), ..pose });
        }
        if throw.desire < -0.25 {
            if let Some(holder) = self.get_mut(throw.holder) {
                let mass = holder.current_mass(settings).max(0.001);
                holder.vel -= throw.up * (force / mass);
            }
        }
    }

    fn resolve_bite(
        &mut self,
        bite: PendingBite,
        pellets: &mut PelletStore,
        rng: &mut crate::rng::Rng,
        settings: &SimSettings,
        events: &mut Vec<BibiteEvent>,
    ) {
        // `BibiteMouth.cs:725-728`: a reaper (`instaKill`, set by the killer
        // flag / `BecomeReaper`) deals `health + 0.01` per bite.
        let insta_kill = self.get(bite.by).is_some_and(|by| by.meta.is_killer);
        let (report, target_pos, victim_dying, victim_area) = match self.get_mut(bite.target) {
            Some(target) => {
                let damage = if insta_kill {
                    target.body.health.amount + 0.01
                } else {
                    bite.damage
                };
                let report = target.body.attack(damage, bite.drain);
                let pos = target.pos;
                let mass = target.current_mass(settings).max(0.001);
                target.vel += bite.direction * (bite.knockback / mass);
                // `otherBody.dying` (`BibiteMouth.cs:730`): set when death starts and
                // cleared again by the first corpse tick, so biting a corpse does
                // not count.
                let dying = report.died || target.body.dying;
                (report, pos, dying, target.body.geometry.base_body_area)
            }
            None => return,
        };
        if report.died {
            if let Some(by) = self.get_mut(bite.by) {
                by.feeding.bibite_kills += 1;
            }
            self.kill(bite.target, DeathCause::Bitten);
        }
        // `BibiteBody.Hurting` (cs:767-776): a hit that takes a corpse below
        // zero health bursts it into meat immediately.
        if report.exploded {
            self.explode_corpse(bite.target, pellets, rng, settings);
        }
        // `BibiteMouth.cs:730-738`: every bite that leaves the target dying
        // (including repeat bites of an already dead body) counts as a murder;
        // 50 murders worth >= 50x the biter's own base area make a reaper.
        if victim_dying {
            if let Some(by) = self.get_mut(bite.by) {
                let own_area = by.body.geometry.base_body_area;
                if by.mouth.record_murder(victim_area, own_area) {
                    by.meta.is_killer = true;
                }
            }
        }
        events.push(BibiteEvent::Bitten {
            by: bite.by,
            target: bite.target,
            damage: report.dealt,
        });
        if let Some(by) = self.get_mut(bite.by) {
            let meat_amount = report.energy_value / settings.materials.meat.energy_density;
            if meat_amount > 0.0 {
                let plant = organ_material(&settings.materials.plant);
                let meat = organ_material(&settings.materials.meat);
                let added =
                    by.stomach
                        .add_matter(StomachMaterial::Meat, meat_amount, &plant, &meat);
                if added < meat_amount - settings.scenario.min_pellet_size {
                    let pos = (by.pos + target_pos) * 0.5;
                    let _ = pellets.spawn_with_energy(
                        MaterialKind::Meat,
                        report.energy_value - added * settings.materials.meat.energy_density,
                        pos,
                    );
                }
            }
        }
    }

    fn resolve_swallow(&mut self, swallow: PendingSwallow, ctx: &mut BibiteTickCtx<'_>) {
        let settings = ctx.settings;
        let plant = organ_material(&settings.materials.plant);
        let meat = organ_material(&settings.materials.meat);
        let Some(prey) = self.get(swallow.prey) else {
            return;
        };
        let meat_amount = prey.body.corpse_meat_energy(
            prey.egg_layer.progress * prey.egg_layer.egg_energy
                + prey.fat.amount * settings.materials.fat.energy_density,
        ) / settings.materials.meat.energy_density;
        let prey_length = prey.body.geometry.body_length;
        let mut prey_contents: [Option<(StomachMaterial, f32)>; STOMACH_SLOTS] =
            [None; STOMACH_SLOTS];
        for (content_count, content) in prey.stomach.contents().enumerate() {
            prey_contents[content_count] = Some((content.material, content.amount));
        }
        if prey_length <= 0.0 {
            return;
        }
        if self.get(swallow.prey).is_none() {
            return;
        }
        self.die_now(
            swallow.prey,
            DeathCause::Swallowed,
            true,
            ctx.pellets,
            ctx.rng,
            settings,
            ctx.events,
        );
        if let Some(predator) = self.get_mut(swallow.predator) {
            // `BibiteMouth.cs:583-590`: the prey's stomach contents are added
            // first, then the body meat (slot order matters for digestion).
            for content in prey_contents.iter().flatten() {
                predator
                    .stomach
                    .add_matter(content.0, content.1, &plant, &meat);
            }
            if meat_amount > 0.0 {
                predator
                    .stomach
                    .add_matter(StomachMaterial::Meat, meat_amount, &plant, &meat);
            }
            predator.release_tag(swallow.prey_tag(), ctx);
        }
    }

    fn resolve_death(&mut self, death: PendingDeath, ctx: &mut BibiteTickCtx<'_>) {
        let settings = ctx.settings;
        let (report, pos, radius, bursts) = {
            let Some(record) = self.get_mut(death.id) else {
                return;
            };
            if record.body.dead && !death.swallowed {
                return;
            }
            let extra = record.egg_layer.progress * record.egg_layer.egg_energy
                + record.fat.amount * settings.materials.fat.energy_density;
            // With corpses off the game bursts the body on `Die()` and once more for every later `Hurting` of the same frame.
            let bursts = record.body.death_hurts.max(1);
            let had_numbers = body_has_numbers(&record.body);
            let report = record.body.die(
                settings.scenario.corpses_enabled,
                record.born,
                death.swallowed,
                extra,
            );
            let report = report_for_numbers(report, had_numbers);
            record.let_go_on_death(ctx.pellets);
            (report, record.pos, record.body.geometry.body_length / 2.0, bursts)
        };
        if let Some(energy) = report.meat_energy {
            for _ in 0..bursts {
                let _ = ctx.pellets.spawn_meat_pile_with_radius(
                    pos,
                    energy,
                    radius,
                    ctx.rng,
                    settings.scenario.min_pellet_size,
                );
                if !death.swallowed
                    && let Some(record) = self.get(death.id)
                {
                    spill_stomach(record, ctx.pellets, settings);
                }
            }
        }
        ctx.events.push(BibiteEvent::Death {
            bibite: death.id,
            cause: death.cause,
        });
        if report.swallowed_and_destroyed || report.unborn_no_meat || report.meat_energy.is_some() {
            self.remove(death.id);
        } else if let Some(record) = self.get_mut(death.id) {
            record.is_corpse = true;
            record.n_mouth_targets = 0;
            record.mouth.n_in_mouth = 0;
        }
    }
}

/// A body whose size, health bar or energy is not a number (a negative growth with a fractional exponent is a NaN size; the game kills a bibite whose
/// health is NaN, `BibiteBody.FixedUpdate`) leaves nothing to put in the world when it dies: no corpse (its mass would be the smallest a body can have, NaN
/// being the other number of a `max`, and a pellet of NaN energy is NaN for good), and no meat pile. It is destroyed.
fn body_has_numbers(body: &BodyState) -> bool {
    body.geometry.d2_size.is_finite()
        && body.energy.amount.is_finite()
        && body.health.max_amount.is_finite()
}

/// The report of a death of a body that [`body_has_numbers`] (asked before `die`, which empties the energy) says has none: destroyed.
fn report_for_numbers(report: DeathReport, had_numbers: bool) -> DeathReport {
    if had_numbers {
        report
    } else {
        DeathReport {
            swallowed_and_destroyed: true,
            ..DeathReport::default()
        }
    }
}

/// What an organs tick reads and never changes: the same for every bibite of a tick, and `Sync`, so the pool's workers share
/// one.
#[derive(Clone, Copy)]
struct OrganEnv<'a> {
    settings: &'a SimSettings,
    zones: &'a [ZoneCircle],
    bs: &'a BodySettings,
    dt: f32,
    plant: &'a MaterialParams,
    meat: &'a MaterialParams,
    fat_params: &'a MaterialParams,
    armor_params: &'a MaterialParams,
    /// The epoch of the body tick (see [`BibiteRecord::refresh_physics_cache`]).
    epoch: u32,
}

/// `BibiteMouth.cs:348`: a `Want2Eat` below this with a loaded jaw vomits.
const VOMIT_WANT_EAT: f32 = -0.15;

/// The few things an organs tick does outside the bibite's own record: looking at the pellets, counting the eggs, laying them,
/// vomiting, letting go of held pellets, the mouth's targets and emitting pheromone. A bibite whose turn cannot reach any of
/// them this tick (see [`pass_b_heavy_reasons`]) runs on a worker thread with [`LocalOnly`]; every other turn runs in dense
/// order with [`CtxShared`], the world's stores. The organs' own code is the same for both.
trait OrganShared {
    /// Whether any of the bibite's mouth targets is a plant pellet.
    fn plant_in_mouth(&self, record: &BibiteRecord) -> bool;
    /// `BibiteEggLayingOrgan.cs:148-150`.
    fn birth_limit_reached(&self) -> bool;
    /// Lays `count` eggs; returns how many were laid.
    fn lay_eggs(&mut self, record: &mut BibiteRecord, count: u32) -> u32;
    /// `BibiteMouth.Vomit`.
    fn vomit(&mut self, record: &mut BibiteRecord, want_eat: f32);
    /// Lets go of every held pellet.
    fn release_grab(&mut self, record: &mut BibiteRecord);
    /// Bites, swallows and grabs what is in the mouth.
    fn tick_mouth(&mut self, record: &mut BibiteRecord, targets: &[TargetSnapshot]);
    /// Emits the bibite's pheromone spot; returns the energy it cost.
    fn emit_pheromone(
        &mut self,
        pos: Vec2,
        heading: Vec2,
        out: [f32; 3],
        settings: &SimSettings,
    ) -> f32;
}

/// The world's stores: the turns that run in dense order.
struct CtxShared<'c, 'a>(&'c mut BibiteTickCtx<'a>);

impl OrganShared for CtxShared<'_, '_> {
    fn plant_in_mouth(&self, record: &BibiteRecord) -> bool {
        for tag in &record.mouth_targets[..record.n_mouth_targets as usize] {
            let Some(pellet_id) = pellet_id_from_tag(*tag) else {
                continue;
            };
            if let Some(view) = self.0.pellets.get(pellet_id) {
                if matches!(view.material, MaterialKind::Plant) {
                    return true;
                }
            }
        }
        false
    }

    fn birth_limit_reached(&self) -> bool {
        birth_limit_reached(self.0)
    }

    fn lay_eggs(&mut self, record: &mut BibiteRecord, count: u32) -> u32 {
        lay_eggs(record, self.0, count)
    }

    fn vomit(&mut self, record: &mut BibiteRecord, want_eat: f32) {
        vomit(record, self.0, want_eat);
    }

    fn release_grab(&mut self, record: &mut BibiteRecord) {
        release_grab(record, self.0);
    }

    fn tick_mouth(&mut self, record: &mut BibiteRecord, targets: &[TargetSnapshot]) {
        tick_mouth(record, self.0, targets);
    }

    fn emit_pheromone(
        &mut self,
        pos: Vec2,
        heading: Vec2,
        out: [f32; 3],
        settings: &SimSettings,
    ) -> f32 {
        self.0
            .pheromones
            .emit_bibite(
                pos,
                heading,
                out,
                settings.scenario.pheromone_production_strength,
                settings.scenario.pheromone_production_cost,
            )
            .cost
    }
}

/// No stores at all: the turns that run on the pool. Every hook is what the organs' code does for a bibite with nothing in the
/// mouth, nothing held, an empty stomach when it would vomit, no egg to lay and no pheromone to emit this tick (the case
/// [`pass_b_heavy_reasons`] picks out), so each does nothing; a hook reached with work to do means that test and the organs
/// disagree, which stops the tests.
struct LocalOnly;

/// A pool turn reached a thing only a dense-order turn may do (see [`LocalOnly`]). Tests stop here; a normal build carries on
/// without the effect.
#[cold]
fn lane_violation(what: &str) {
    if cfg!(test) {
        panic!("a bibite on the worker pool reached {what}");
    }
}

impl OrganShared for LocalOnly {
    fn plant_in_mouth(&self, _record: &BibiteRecord) -> bool {
        lane_violation("the pellets in its mouth");
        false
    }

    fn birth_limit_reached(&self) -> bool {
        // The limit only decides whether the eggs the womb reports are laid; the womb's own progress is the same either way,
        // and a turn on the pool has no egg to report (the heavy test probes the womb with the limit off).
        false
    }

    fn lay_eggs(&mut self, _record: &mut BibiteRecord, _count: u32) -> u32 {
        lane_violation("an egg to lay");
        0
    }

    fn vomit(&mut self, record: &mut BibiteRecord, _want_eat: f32) {
        if !record.stomach.is_empty() {
            lane_violation("a stomach to vomit");
        }
    }

    fn release_grab(&mut self, record: &mut BibiteRecord) {
        if record.n_held_targets > 0 {
            lane_violation("a held pellet");
        }
        record.n_held_targets = 0;
        record.mouth.n_held = 0;
    }

    fn tick_mouth(&mut self, _record: &mut BibiteRecord, targets: &[TargetSnapshot]) {
        if !targets.is_empty() {
            lane_violation("a mouth target");
        }
    }

    fn emit_pheromone(
        &mut self,
        _pos: Vec2,
        _heading: Vec2,
        out: [f32; 3],
        settings: &SimSettings,
    ) -> f32 {
        let sum = bibite_emission_sum(out, settings.scenario.pheromone_production_strength);
        if bibite_emission_triggers(sum) {
            lane_violation("a pheromone to emit");
        }
        0.0
    }
}

/// What the sweep did of a heavy turn before it handed it to the dense-order stretch (the bits of the flags that go with its slot).
mod turn {
    /// The head of the turn ([`super::pass_b_head`]) was taken in the sweep: the dense-order stretch does the rest.
    pub const HEAD_DONE: u8 = 1;
    /// A pheromone puff is due this tick: the rest of the turn reaches the pheromone store, so it stays in dense order to the end.
    pub const EMITS: u8 = 2;
}

/// Whether a bibite's heavy turn is cut in parts ([`pass_b_head`], [`organs_hooks`], [`pass_b_post`]) so that the pool takes the parts
/// that touch nothing but the record. `RUSTIBITES_NOSPLIT` (any value) puts the whole turn in dense order, as it was (the results do not
/// depend on it; it is there to time the two against each other and for the tests).
fn split_turns_enabled() -> bool {
    static SPLIT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SPLIT.get_or_init(|| std::env::var_os("RUSTIBITES_NOSPLIT").is_none())
}

/// Bits of [`pass_b_heavy_reasons`].
mod heavy {
    /// Something is in the mouth.
    pub const MOUTH: u8 = 1;
    /// A pellet is held.
    pub const HELD: u8 = 2;
    /// The jaw may vomit a loaded stomach.
    pub const VOMIT: u8 = 4;
    /// The womb reports eggs to lay.
    pub const EGGS: u8 = 8;
    /// A pheromone spot is to be emitted.
    pub const EMIT: u8 = 16;
}

/// The brain's three pheromone outputs after the scenario's "prevent production" switches.
fn pheromone_channels(settings: &SimSettings, outputs: &[f32; N_OUTPUTS]) -> [f32; 3] {
    [
        if settings.scenario.prevent_red_phero_production {
            0.0
        } else {
            outputs[Output::PhereOut1.index()]
        },
        if settings.scenario.prevent_green_phero_production {
            0.0
        } else {
            outputs[Output::PhereOut2.index()]
        },
        if settings.scenario.prevent_blue_phero_production {
            0.0
        } else {
            outputs[Output::PhereOut3.index()]
        },
    ]
}

/// Why the pass-B turn of a live, born and ready bibite with these brain outputs may need the world's stores (a mask of
/// [`heavy`] bits; zero = the turn touches nothing but the bibite's own record and can run on a worker thread):
/// - mouth targets: it reads the pellets and the other bibites, and bites, swallows or grabs;
/// - held pellets: it may let go of them;
/// - vomiting: a loaded stomach spawns pellets (an empty one has nothing to spit out, and digestion only ever empties it);
/// - eggs: the womb, probed on a copy with the birth limit off (the limit only suppresses laying), reports eggs to lay;
/// - pheromone: this tick's emission is above the threshold.
///
/// Everything here is decided from the state at the start of the turn by the same expressions the organs use.
fn pass_b_heavy_reasons(
    record: &BibiteRecord,
    outputs: &[f32; N_OUTPUTS],
    settings: &SimSettings,
    dt: f32,
) -> u8 {
    let mut reasons = 0;
    if record.n_mouth_targets > 0 {
        reasons |= heavy::MOUTH;
    }
    if record.n_held_targets > 0 {
        reasons |= heavy::HELD;
    }
    if outputs[Output::Want2Eat.index()] < VOMIT_WANT_EAT && !record.stomach.is_empty() {
        reasons |= heavy::VOMIT;
    }
    let want_to_lay = outputs[Output::Want2Lay.index()];
    if want_to_lay >= LAYING_THRESHOLD {
        let mut womb = record.egg_layer;
        let rate = record.genome.metabolic_rate();
        let probe = womb.update(
            dt,
            outputs[Output::EggProduction.index()],
            rate,
            want_to_lay,
            false,
        );
        if probe.eggs_to_lay > 0 {
            reasons |= heavy::EGGS;
        }
    }
    if record.pheromone.progress + dt >= PHEROMONE_EMIT_PERIOD {
        let out = pheromone_channels(settings, outputs);
        let sum = bibite_emission_sum(out, settings.scenario.pheromone_production_strength);
        if bibite_emission_triggers(sum) {
            reasons |= heavy::EMIT;
        }
    }
    reasons
}

/// A live, born and ready bibite's pass-B turn after its mouth targets were looked at: the attacked-damage decay, the organs,
/// the clock, and the propulsion bookkeeping. Returns whether its health went non-finite (a starvation death for the caller
/// to queue in dense order).
fn pass_b_live<S: OrganShared>(
    record: &mut BibiteRecord,
    shared: &mut S,
    env: &OrganEnv<'_>,
    outputs: &[f32; N_OUTPUTS],
    snapshots: &[TargetSnapshot],
) -> bool {
    let up = pass_b_head(record, shared, env, outputs);
    organs_hooks(record, shared, outputs, snapshots);
    pass_b_post(record, shared, env, outputs, up)
}

/// The first part of [`pass_b_live`]: the attacked-damage decay and [`organs_head`]. Returns the forward vector.
fn pass_b_head<S: OrganShared>(
    record: &mut BibiteRecord,
    shared: &mut S,
    env: &OrganEnv<'_>,
    outputs: &[f32; N_OUTPUTS],
) -> Vec2 {
    // `BibiteBody.cs:622-625`: after the brain step and before the
    // organs, `attackedDmg` decays every fixed update.
    if record.body.attacked_dmg > 0.0 {
        record.body.attacked_dmg = attacked_dmg_decay(record.body.attacked_dmg, env.dt);
    }
    organs_head(record, shared, env, outputs)
}

/// The last part of [`pass_b_live`], after the mouth: [`organs_post`], the clock and the propulsion bookkeeping. Returns whether the
/// health went non-finite (a starvation death for the caller to queue in dense order). `up` is what [`pass_b_head`] returned (the
/// rotation does not change in a turn: the forward vector is not worked out a second time).
fn pass_b_post<S: OrganShared>(
    record: &mut BibiteRecord,
    shared: &mut S,
    env: &OrganEnv<'_>,
    outputs: &[f32; N_OUTPUTS],
    up: Vec2,
) -> bool {
    let (bs, dt) = (env.bs, env.dt);
    organs_post(record, shared, env, outputs, up);
    let clock_tick = record
        .clock
        .tick(dt, record.brain.output_value(Output::ClkReset));
    if clock_tick.aged {
        // `clock.OnAging` -> `UpdateMetabolism` every 120 s alive
        // (`BibiteBody.cs:510`, `InternalClock.cs:86-89`).
        record
            .body
            .update_metabolism(age_hours(record.clock.time_alive), bs);
    }
    // A spend this turn that took the health below zero (the egg layer's, the fat's, the muscles', the healing's) is the game's
    // `UseEnergy` -> `Hurting` -> `Die()` on the spot; the results of those spends are not looked at one by one, so the turn ends
    // with the check: without it a bibite whose fat refills the energy before the next metabolism spend lives on at negative health.
    // The death stands whatever the rest of the turn did to the health (`death_hurts` counts the lethal spends): the healing runs
    // last and adds `hp` after its own spend, and a Want2Heal output that is NaN makes `hp` the whole missing health (`min` of a NaN
    // is the other number, in the game's `Mathf.Min` as here), which brought a bibite without energy back from every lethal spend:
    // it lived on, and laid eggs, on health alone (the glitch save of 2026-11-13: 257,000 of them).
    let non_finite = !record.body.health.amount.is_finite()
        || record.body.health.amount < 0.0
        || record.body.death_hurts > 0;
    let last_pos = record.propulsion.last_pos;
    record.propulsion.total_travel += record.pos.distance(last_pos);
    record.propulsion.last_pos = record.pos;
    record.propulsion.angular_velocity = record.ang_vel.to_degrees() / 360.0;
    // `SignedVelocity = Vector2.Dot(linearVelocity, transform.up)`: the sum in double, rounded once.
    record.propulsion.signed_velocity = crate::math::unity::dot(record.vel, up);
    record.propulsion.normalized_velocity = if record.body.geometry.body_length > 0.0 {
        record.propulsion.signed_velocity / record.body.geometry.body_length
    } else {
        0.0
    };
    record.refresh_physics_cache(env);
    non_finite
}

/// The first part of a bibite's organs turn: the feeding counters, the stomach, the womb, the fat, the movement and the mouth's latching of
/// the brain's outputs. Returns the forward vector. It reads and writes the bibite's own record and nothing else when no egg is to be
/// laid (see [`pass_b_heavy_reasons`]), so a bibite whose turn has to reach the world's stores later (the mouth's targets, a held pellet,
/// a loaded stomach to vomit, a pheromone to emit) can still take this part in the worker pool's sweep ([`pass_b_head`]).
fn organs_head<S: OrganShared>(
    record: &mut BibiteRecord,
    shared: &mut S,
    env: &OrganEnv<'_>,
    outputs: &[f32; N_OUTPUTS],
) -> Vec2 {
    let OrganEnv {
        settings,
        zones,
        bs,
        dt,
        plant,
        meat,
        fat_params,
        ..
    } = *env;
    record.feeding.live_seconds += f64::from(dt);
    let had_contact = record.n_mouth_targets > 0;
    if had_contact {
        record.feeding.mouth_contact_ticks += 1;
        if !record.feeding.contact_latched {
            record.feeding.mouth_contact_enter_events += 1;
        }
    }
    record.feeding.contact_latched = had_contact;
    // The genes the turn reads every tick come from `growth_genes` (on a line the turn reads anyway), not from the genome's lines.
    #[cfg(test)]
    assert!(
        record.growth_genes.metabolic_rate.to_bits() == record.genome.metabolic_rate().to_bits()
            && record.growth_genes.herd_velocity_weight.to_bits()
                == record.genome.gene(Gene::HerdVelocityWeight).to_bits(),
        "the genes cached for the turn are not the genome's"
    );
    let rate = record.growth_genes.metabolic_rate;
    // Stomach.
    let stomach_before = record.stomach.total_amount();
    let digest = record
        .stomach
        .update(dt, outputs[Output::Digestion.index()], rate, plant, meat);
    record.feeding.stomach_digested_u2 +=
        f64::from((stomach_before - record.stomach.total_amount()).max(0.0));
    record.feeding.stomach_energy_gain_e += f64::from(digest.energy_gain);
    record.body.gain_energy(digest.energy_gain);
    // Egg layer.
    let egg_tick = record.egg_layer.update(
        dt,
        outputs[Output::EggProduction.index()],
        rate,
        outputs[Output::Want2Lay.index()],
        shared.birth_limit_reached(),
    );
    if egg_tick.spends {
        // `BibiteEggLayingOrgan.UpdateOrgan`: `num >= 0` spends, even when the egg tray is full and `num` is 0.
        record.body.use_energy(egg_tick.energy_cost, true);
    } else if egg_tick.energy_gain > 0.0 {
        record.body.gain_energy(egg_tick.energy_gain);
    }
    if egg_tick.eggs_to_lay > 0 {
        let laid = shared.lay_eggs(record, egg_tick.eggs_to_lay);
        if laid > 0 {
            record.egg_layer.on_eggs_laid(laid);
        }
    }
    // Fat.
    let fat_tick = record.fat.update(
        dt,
        record.body.energy_ratio(),
        record.body.geometry.d2_size,
        rate,
        settings.scenario.fat_sustain,
        fat_params,
    );
    record.body.gain_energy(fat_tick.energy_gain);
    record.body.use_energy(fat_tick.sustain_cost, true);
    if fat_tick.inflate_changed {
        record.body.set_inflate_factor(fat_tick.inflate_factor, bs);
        record.refresh_organs(settings);
        record.refresh_colliders();
    }
    // Movement.
    let ageing = ageing_factor(
        age_hours(record.clock.time_alive),
        settings.scenario.ageing_threshold,
        rate,
    );
    let strength_penalty = age_strength_penalty_factor(ageing, bs);
    let health_fullness = record.body.health.fullness();
    let movement = MovementInput {
        accelerate: outputs[Output::Accelerate.index()],
        rotate: outputs[Output::Rotate.index()],
        herding: outputs[Output::Herding.index()],
        has_herd: record.steering.has_herd && !settings.scenario.disable_herding,
        herd_sum_direction_degrees: record.steering.herd_sum_direction_degrees,
        herd_separation_projection: record.steering.herd_separation_projection,
        herd_velocity_weight: record.growth_genes.herd_velocity_weight,
        health_fullness,
        age_strength_penalty: strength_penalty,
    };
    let up = crate::math::unity::transform_up(record.rot);
    let command = compute_movement(
        &record.propulsion_derived,
        &movement,
        up,
        settings.scenario.backward_fraction,
    );
    let penalty = movement_penalty(health_fullness, strength_penalty);
    let command = apply_environment_steering(record, settings, zones, command, up, penalty);
    record.pending_force += command.force;
    record.pending_torque += command.torque;
    record.body.movement_cost = command.movement_cost;
    record.body.use_energy(command.movement_cost * dt, true);
    // Mouth.
    record
        .mouth
        .begin_tick(dt, record.mouth_derived.bite_period);
    record.mouth.latch_outputs(
        outputs[Output::Want2Attack.index()],
        outputs[Output::Want2Eat.index()],
        outputs[Output::Grab.index()],
        &record.mouth_derived,
    );
    up
}

/// The middle of a bibite's organs turn, the part that reaches the world's stores when the mouth has anything to do: the plant pellets
/// in the mouth are counted, and the mouth vomits, lets go, bites, swallows and grabs.
fn organs_hooks<S: OrganShared>(
    record: &mut BibiteRecord,
    shared: &mut S,
    outputs: &[f32; N_OUTPUTS],
    targets: &[TargetSnapshot],
) {
    // (This was the top of the turn: what it counts is read by nothing in the head, so a turn that took its head in the sweep counts
    // here, with the pellets as the turns before it in dense order left them, which is what the serial order shows it.)
    if record.n_mouth_targets > 0 && shared.plant_in_mouth(record) {
        record.feeding.plant_contact_ticks += 1;
    }
    // `BibiteMouth.cs:348-352`: a strong negative Want2Eat with a loaded jaw
    // vomits and skips the rest of the mouth update for this tick.
    let want_eat = outputs[Output::Want2Eat.index()];
    if want_eat < VOMIT_WANT_EAT && record.mouth.bite_progress >= record.mouth_derived.bite_period {
        shared.vomit(record, want_eat);
        record.mouth.bite_progress -= record.mouth_derived.bite_period;
    } else {
        if record.mouth.desire_grab <= -MOUTH_DEAD_ZONE {
            shared.release_grab(record);
        }
        shared.tick_mouth(record, targets);
    }
}

/// The last part of a bibite's organs turn: the pheromone puff, growth and healing, and the totals. `up` is the forward vector
/// [`organs_head`] returned. With no puff to emit (see [`pass_b_heavy_reasons`]) it reads and writes the bibite's own record and nothing
/// else, and it needs no more of the world than the turn's own results so far, so a bibite that nobody looks at later this tick can take
/// it in the worker pool after the mouth was dealt with ([`pass_b_post`]).
fn organs_post<S: OrganShared>(
    record: &mut BibiteRecord,
    shared: &mut S,
    env: &OrganEnv<'_>,
    outputs: &[f32; N_OUTPUTS],
    up: Vec2,
) {
    let OrganEnv {
        settings, bs, dt, ..
    } = *env;
    // Pheromone.
    record.pheromone.progress += dt;
    if record.pheromone.progress >= PHEROMONE_EMIT_PERIOD {
        record.pheromone.progress -= PHEROMONE_EMIT_PERIOD;
        let out = pheromone_channels(settings, outputs);
        let cost = shared.emit_pheromone(record.pos, up, out, settings);
        if cost > 0.0 {
            record.pheromone.cost = cost;
            // `BibitePheromoneOrgan.UpdateOrgan`: only a puff that is made costs energy.
            record.body.use_energy(cost, true);
        }
    }
    // Growth (record ref stored separately because it is a third-party type).
    let mut growth = record.growth.clone();
    let genes = record.growth_genes;
    if let Some(step) = growth.tick(dt, outputs[Output::Want2Grow.index()], &genes) {
        record.growth = growth;
        record.refresh_organs(settings);
        let application =
            record
                .body
                .apply_growth_step(&step, age_hours(record.clock.time_alive), bs);
        record.growth.record_cost_rate(application.growth_cost_rate);
        record.sync_derived_geometry();
        record.refresh_colliders();
        if step.became_mature {
            record.egg_layer.is_mature = true;
        }
    } else {
        record.growth = growth;
    }
    // Healing.
    record.body.heal(outputs[Output::Want2Heal.index()], dt, bs);
    // BodyState energy totals are runtime-only monotonic counters.
    record.feeding.energy_gained_e = record.body.energy_gained_total;
    record.feeding.energy_spent_e = record.body.energy_spent_total;
}

/// Applies shade/void steering to the movement command after the herding
/// blend (`BibitePropulsion.cs:290-306`). `turn` is recovered from the command
/// torque, blended toward the avoidance direction, then re-clamped and the
/// energy cost recomputed so force/torque/cost stay consistent.
fn apply_environment_steering(
    record: &mut BibiteRecord,
    settings: &SimSettings,
    zones: &[ZoneCircle],
    mut command: organs::MovementCommand,
    up: Vec2,
    penalty: f32,
) -> organs::MovementCommand {
    let no_void = !settings.scenario.void_avoidance || zones.is_empty();
    if !settings.scenario.shade_avoidance && no_void {
        return command;
    }
    let derived = record.propulsion_derived;
    if derived.muscle_turn_strength <= f32::EPSILON {
        return command;
    }
    // `command.torque` is `clamp(turn) * penalty * strength`; the game blends
    // the avoidance terms into the raw turn and applies the penalty once at
    // the end (`BibitePropulsion.cs:308-309`), so undo the penalty here.
    let mut turn = if penalty > f32::EPSILON {
        command.torque / derived.muscle_turn_strength / penalty
    } else {
        0.0
    };
    let sim_size = settings.independents.simulation_size;
    if settings.scenario.void_avoidance && !zones.is_empty() {
        // `BibitePropulsion.cs:272-292`: the zone whose edge is nearest (every zone is a circle of its `radius`, a rectangle's being half of its longer side),
        // and the bibite steers for ITS centre when it is outside that one. The scan keeps a vector `val4` to the chosen zone's edge: a zone replaces the
        // chosen one when `distance - radius` (negative inside it) is less than the length of that vector (infinite at first), so the last zone the bibite is
        // inside wins over the ones before it.
        let mut chosen: Option<&ZoneCircle> = None;
        let mut reach = f32::INFINITY;
        for zone in zones {
            let to_centre = zone.pos - record.pos;
            let distance = to_centre.length();
            if distance - zone.radius < reach {
                // `val5 - radius * val5.normalized`: along the line to the centre, `|distance - radius|` long (nothing when the bibite is at the centre of a zero-radius zone).
                reach = if distance > 0.0 { (distance - zone.radius).abs() } else { 0.0 };
                chosen = Some(zone);
            }
        }
        if let Some(zone) = chosen {
            let to_centre = zone.pos - record.pos;
            let distance = to_centre.length();
            if distance > zone.radius {
                let weight = void_avoidance_weight(distance - zone.radius, settings.scenario.void_avoidance_distance);
                // `Mathf.Tanh(Vector2.SignedAngle(up, val4))`: the angle in degrees.
                let angle = up.cross(to_centre).atan2(up.dot(to_centre)).to_degrees();
                turn = blend_turn_towards(turn, angle, weight);
            }
        }
    }
    // Void avoidance runs before shade avoidance in the game (cs:272-307).
    if settings.scenario.shade_avoidance && settings.scenario.shade_outside_of_bounds {
        let out2 = out_of_bound_dist_sq(sim_size);
        let to_origin = -record.pos;
        let angle = up.cross(to_origin).atan2(up.dot(to_origin)).to_degrees();
        turn = shade_avoidance_steering(turn, record.pos.length_sq(), out2, angle);
    }
    turn = clamp(turn, -1.0, 1.0) * penalty;
    let accelerate = if derived.muscle_move_strength > f32::EPSILON {
        command.force.dot(up) / derived.muscle_move_strength
    } else {
        0.0
    };
    command.torque = turn * derived.muscle_turn_strength;
    command.movement_cost =
        derived.base_acceleration_cost * accelerate.abs() + derived.base_turn_cost * turn.abs();
    command
}

/// `BibiteEggLayingOrgan.cs:148-150`: laying is blocked when the shared
/// bibite/egg holder's child count reaches `bibiteLimit` (only with
/// `limitBibiteBirth`).
fn birth_limit_reached(ctx: &BibiteTickCtx<'_>) -> bool {
    let independents = &ctx.settings.independents;
    independents.limit_bibite_birth
        && (ctx.scratch.population + ctx.eggs.len()) as f32 >= independents.bibite_limit as f32
}

fn lay_eggs(record: &mut BibiteRecord, ctx: &mut BibiteTickCtx<'_>, count: u32) -> u32 {
    let settings = ctx.settings;
    let mut laid = 0;
    for _ in 0..count {
        // Seed the child meta from the parent so generation, species,
        // lineage tag and easter-egg/mutant flags are inherited; only the
        // parent ids are set here (`Genome::child` increments generation and
        // clears parent ids itself).
        let mut parent_meta = record.meta.clone();
        parent_meta.parent1 = Some(record.id);
        parent_meta.parent2 = None;
        let (genome, mut meta) = record.genome.child(
            &parent_meta,
            ctx.rng,
            settings,
            &record.effective_mutation_factor(),
        );
        meta.parent1 = Some(record.id);
        meta.parent2 = None;
        let mut brain = record.brain.clone();
        brain.mutate(
            ctx.rng,
            settings,
            &genome,
            &record.effective_mutation_factor(),
        );
        // `eggBrain.CopyBrain(brain.Nodes, brain.Synapses, mutate: true)`: the egg's brain is built from the parent's structure, its nodes start fresh.
        brain.reset_runtime_state(settings.brain_period());
        let upkeep = brain_upkeep_per_second(&brain, settings);
        let required = egg_required_energy(
            genome.body_energy_at_birth(settings),
            upkeep,
            genome.gene(Gene::SizeRatio),
        );
        let hatch_time = genome.gene(Gene::HatchTime);
        let up = crate::math::unity::transform_up(record.rot);
        let pos = record.pos - up * (8.0 * record.body.geometry.d1_size);
        let egg = ctx.eggs.spawn(EggSpawn {
            pos,
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome,
            meta,
            brain,
            energy: record.egg_layer.egg_energy,
            hatch_time,
            total_required_energy: required,
            birth_orientation: -up,
        });
        ctx.events.push(BibiteEvent::EggLaid { by: record.id, egg });
        laid += 1;
    }
    laid
}

fn tick_mouth(record: &mut BibiteRecord, ctx: &mut BibiteTickCtx<'_>, targets: &[TargetSnapshot]) {
    let settings = ctx.settings;
    // `OnTriggerExit2D` -> `ReleaseTargetIfHeld`: what left the mouth trigger is let go. A target the last step did not see is out.
    let mut held_at = 0;
    while held_at < record.n_held_targets as usize {
        let tag = record.held_targets[held_at];
        let inside = (0..record.n_mouth_targets as usize).any(|at| record.mouth_targets[at] == tag && record.mouth_target_ages[at] == 0);
        if inside {
            held_at += 1;
        } else {
            record.release_tag(tag, ctx);
        }
    }
    let bite_period = record.mouth_derived.bite_period;
    for target in targets {
        if target.is_egg {
            continue;
        }
        if target.is_pellet {
            tick_mouth_pellet(record, ctx, *target, bite_period);
            continue;
        }
        let Some(target_id) = bibite_id_from_tag(target.tag) else {
            continue;
        };
        if target_id == record.id {
            continue;
        }
        if record.mouth.desire_grab >= MOUTH_DEAD_ZONE
            && !record.is_held(target.tag)
            && record.n_held_targets < record.held_targets.len() as u8
        {
            record.add_held(target.tag);
        }
        let grabbed = record.is_held(target.tag);
        let can_swallow = record.mouth.can_swallow
            && record.mouth.desire_eat
                > if grabbed {
                    record.mouth.desire_grab
                } else {
                    MOUTH_DEAD_ZONE
                };
        if can_swallow {
            match try_swallow_bibite(
                target.body_length,
                target.meat_amount,
                target.stomach_amount,
                record.stomach.available_space(),
                record.mouth.mouth_opening,
                record.mouth.bite_progress,
                bite_period,
            ) {
                BibiteSwallow::Swallow => {
                    if target.body_length > record.mouth.mouth_opening / 2.0 {
                        if record.mouth.bite_progress < bite_period {
                            continue;
                        }
                        record.mouth.consume_bite_period(bite_period);
                    }
                    record.feeding.bibite_swallow_events += 1;
                    record.feeding.bibite_consumed_u2 +=
                        f64::from(target.meat_amount.max(0.0) + target.stomach_amount.max(0.0));
                    ctx.scratch.swallows.push(PendingSwallow {
                        predator: record.id,
                        prey: target_id,
                    });
                    continue;
                }
                BibiteSwallow::NeedsBite | BibiteSwallow::NoSpace | BibiteSwallow::TooBig => {}
            }
        }
        // `BibiteMouth.cs:618`: `MaybeAttack` returns once a bibite was hit
        // this tick, so only the first (nearest) target is ever attacked.
        if record.mouth.attacked_this_frame || record.mouth.desire_attack <= MOUTH_DEAD_ZONE {
            continue;
        }
        let ongoing = record.mouth.attacked_last_frame;
        if !ongoing && !record.mouth.bite_ready(bite_period) {
            continue;
        }
        if !ongoing {
            record.mouth.consume_bite_period(bite_period);
            record.mouth.record_first_bite();
            record.feeding.bibite_bite_events += 1;
        }
        // `BibiteMouth.cs:714`: mark the attack before the armor/damage checks
        // so the next tick's `attackedLastFrame` enables the sustained-damage
        // path while the target remains in the mouth.
        record.mouth.attacked_this_frame = true;
        let input = bibite_bite_input(
            record.mouth.bite_opening,
            record.mouth.bite_strength,
            target.armor_strength,
            target.internals_radius,
            target.effective_radius,
            ongoing,
        );
        let chunk = bite_chunk_area(&input);
        let damage = resolve_bite_damage(&BiteDamageInput {
            bite_strength: record.mouth.bite_strength,
            armor_strength: target.armor_strength,
            target_max_health: target.max_health,
            target_real_body_area: target.real_body_area,
            chunk_area: chunk,
            ongoing,
            bite_period,
            dt: 1.0 / settings.sim_tps.max(1) as f32,
            damage_factor: settings.scenario.biting_damage_factor,
        });
        if damage.armor_blocked {
            continue;
        }
        record.mouth.total_damage_dealt += damage.damage;
        record.feeding.bibite_damage_dealt += f64::from(damage.damage);
        ctx.scratch.bites.push(PendingBite {
            by: record.id,
            target: target_id,
            damage: damage.damage,
            drain: ongoing,
            // Only the initial bite imparts the throw impulse; sustained
            // (ongoing) attack ticks apply damage without re-knocking the
            // victim (the game lets the first impulse decay under drag).
            knockback: if ongoing {
                0.0
            } else {
                bite_knockback_impulse(
                    settings.scenario.throwing_force_factor,
                    settings.scenario.biting_throw_force_factor,
                    record.mouth.bite_strength,
                )
            },
            direction: (target.pos - record.pos).normalized(),
        });
    }
}

fn tick_mouth_pellet(
    record: &mut BibiteRecord,
    ctx: &mut BibiteTickCtx<'_>,
    target: TargetSnapshot,
    bite_period: f32,
) {
    let settings = ctx.settings;
    let Some(pellet_id) = pellet_id_from_tag(target.tag) else {
        return;
    };
    let plant = organ_material(&settings.materials.plant);
    let meat = organ_material(&settings.materials.meat);
    let material = if target.material == 0 {
        StomachMaterial::Plant
    } else {
        StomachMaterial::Meat
    };
    // `TryGrabTarget`: `if (nHeld < 10)` comes before `targetToGrab.TryGrab(this)`: a mouth that holds ten flags nothing as held (the pellet would be exempt from Red Death
    // and from sleeping, and lose its drag, with no holder ever to let it go).
    if record.mouth.desire_grab >= MOUTH_DEAD_ZONE
        && !record.is_held(target.tag)
        && (record.n_held_targets as usize) < record.held_targets.len()
        && ctx.pellets.set_held(pellet_id, true)
    {
        record.add_held(target.tag);
    }
    let grabbed = record.is_held(target.tag);
    let can_swallow = record.mouth.can_swallow
        && record.mouth.desire_eat
            > if grabbed {
                record.mouth.desire_grab
            } else {
                MOUTH_DEAD_ZONE
            };
    if can_swallow {
        match try_swallow_pellet(
            target.radius,
            target.amount,
            record.stomach.available_space(),
            record.mouth.mouth_opening,
            record.mouth.bite_progress,
            bite_period,
        ) {
            PelletSwallow::Swallow => {
                if target.radius > record.mouth.mouth_opening / 4.0 {
                    if record.mouth.bite_progress < bite_period {
                        return;
                    }
                    record.mouth.consume_bite_period(bite_period);
                }
                ctx.pellets.remove(pellet_id);
                record
                    .stomach
                    .add_matter(material, target.amount, &plant, &meat);
                match material {
                    StomachMaterial::Plant => {
                        record.feeding.plant_swallow_events += 1;
                        record.feeding.plant_consumed_u2 += f64::from(target.amount);
                    }
                    StomachMaterial::Meat => {
                        record.feeding.meat_swallow_events += 1;
                        record.feeding.meat_consumed_u2 += f64::from(target.amount);
                    }
                }
                ctx.events.push(BibiteEvent::Swallowed {
                    by: record.id,
                    pellet: pellet_id,
                });
                record.release_tag(target.tag, ctx);
                return;
            }
            PelletSwallow::NeedsBite | PelletSwallow::NoSpace | PelletSwallow::TooBig => {}
        }
    }
    if record.mouth.desire_attack <= MOUTH_DEAD_ZONE {
        return;
    }
    let ongoing = record.mouth.attacked_last_frame;
    if !ongoing && !record.mouth.bite_ready(bite_period) {
        return;
    }
    if !ongoing {
        record.mouth.consume_bite_period(bite_period);
    }
    let cohesiveness = if target.material == 0 {
        plant.cohesiveness
    } else {
        meat.cohesiveness
    };
    let input = pellet_bite_input(
        record.mouth.mouth_opening,
        record.mouth.bite_strength,
        cohesiveness,
        target.radius,
    );
    let chunk = bite_chunk_area(&input);
    if record.mouth.desire_attack > record.mouth.desire_eat {
        let normal = (record.pos - target.pos).normalized();
        if let Some(ripped) = ctx.pellets.rip_chunk(
            pellet_id,
            chunk,
            Some(normal),
            settings.scenario.min_pellet_size,
            ctx.rng,
        ) {
            match material {
                StomachMaterial::Plant => {
                    record.feeding.plant_bite_events += 1;
                    record.feeding.plant_ripped_u2 += f64::from(ripped.removed_amount);
                }
                StomachMaterial::Meat => {
                    record.feeding.meat_bite_events += 1;
                    record.feeding.meat_ripped_u2 += f64::from(ripped.removed_amount);
                }
            }
            return;
        }
    }
    let space = record.stomach.available_space();
    let amount = chunk.min(space).min(target.amount);
    if amount <= 0.0 {
        return;
    }
    let removed = if target.amount - amount < settings.scenario.min_pellet_size {
        ctx.pellets.remove(pellet_id);
        target.amount
    } else {
        -(ctx.pellets.change_amount(pellet_id, -amount))
    };
    if removed > 0.0 {
        record.stomach.add_matter(material, removed, &plant, &meat);
        match material {
            StomachMaterial::Plant => {
                record.feeding.plant_bite_events += 1;
                record.feeding.plant_consumed_u2 += f64::from(removed);
            }
            StomachMaterial::Meat => {
                record.feeding.meat_bite_events += 1;
                record.feeding.meat_consumed_u2 += f64::from(removed);
            }
        }
    }
}

impl BibiteRecord {
    /// `BibiteMouth.OnDeathStatusChange(alive: false)` (cs:510-519): a bibite that dies lets go of everything it holds, without a
    /// throw, and its mouth is off. A corpse that kept its holds stayed welded to what it held (a pellet flagged as held for good,
    /// or a live bibite dragged along by a body that cannot move on its own), and the weld of a dead holder was one of the
    /// joints that blew up in the glitch save of 2026-11-13.
    fn let_go_on_death(&mut self, pellets: &mut PelletStore) {
        for index in 0..self.n_held_targets as usize {
            if let Some(id) = pellet_id_from_tag(self.held_targets[index]) {
                pellets.set_held(id, false);
            }
            self.held_targets[index] = 0;
        }
        self.n_held_targets = 0;
        self.mouth.n_held = 0;
        self.n_mouth_targets = 0;
        self.mouth.n_in_mouth = 0;
    }

    fn add_held(&mut self, tag: u64) {
        if (self.n_held_targets as usize) < self.held_targets.len() {
            let index = self.n_held_targets as usize;
            self.held_targets[index] = tag;
            self.n_held_targets += 1;
            self.mouth.n_held = self.n_held_targets as usize;
        }
    }

    pub(crate) fn is_held(&self, tag: u64) -> bool {
        self.held_targets[..self.n_held_targets as usize].contains(&tag)
    }

    fn release_tag(&mut self, tag: u64, ctx: &mut BibiteTickCtx<'_>) {
        let count = self.n_held_targets as usize;
        if let Some(index) = self.held_targets[..count].iter().position(|t| *t == tag) {
            ctx.scratch.pending_releases.push((bibite_tag(self.id), tag));
            self.held_targets[index] = self.held_targets[count - 1];
            self.n_held_targets -= 1;
            self.mouth.n_held = self.n_held_targets as usize;
            if let Some(id) = pellet_id_from_tag(tag) {
                ctx.pellets.set_held(id, false);
            }
        }
    }
}

/// `BibiteStomach.OnBodyDestroyed` (cs:227-255): when a body is destroyed
/// by explosion every stomach slot above `minPelletSize` is released as a
/// pellet at the body position.
fn spill_stomach(record: &BibiteRecord, pellets: &mut PelletStore, settings: &SimSettings) {
    let min_size = settings.scenario.min_pellet_size;
    for content in record.stomach.contents() {
        if content.amount <= min_size {
            continue;
        }
        let (kind, density) = match content.material {
            StomachMaterial::Plant => {
                (MaterialKind::Plant, settings.materials.plant.energy_density)
            }
            StomachMaterial::Meat => (MaterialKind::Meat, settings.materials.meat.energy_density),
        };
        let _ = pellets.spawn_with_energy(kind, content.amount * density, record.pos);
    }
}

/// `BibiteMouth.Vomit` (cs:864-901): removes `|eat/0.95|` of every stomach
/// slot above `minPelletSize`, spawns pellets at the mouth and throws them
/// along the bibite's up axis with an impulse split by pellet mass.
fn vomit(record: &mut BibiteRecord, ctx: &mut BibiteTickCtx<'_>, want_eat: f32) {
    if record.stomach.is_empty() {
        return;
    }
    let settings = ctx.settings;
    let min_size = settings.scenario.min_pellet_size;
    let fraction = crate::organs::vomit_fraction(want_eat);
    let slots: Vec<(StomachMaterial, f32)> = record
        .stomach
        .contents()
        .map(|content| (content.material, content.amount))
        .collect();
    let mut spawned: Vec<PelletId> = Vec::new();
    for (material, amount) in slots {
        if amount <= min_size {
            continue;
        }
        let removed = record.stomach.remove_matter(material, amount * fraction);
        let (kind, density) = match material {
            StomachMaterial::Plant => {
                (MaterialKind::Plant, settings.materials.plant.energy_density)
            }
            StomachMaterial::Meat => (MaterialKind::Meat, settings.materials.meat.energy_density),
        };
        if let Some(id) = ctx
            .pellets
            .spawn_with_energy(kind, removed * density, record.pos)
        {
            spawned.push(id);
        }
    }
    let total_mass: f32 = spawned.iter().filter_map(|id| ctx.pellets.mass(*id)).sum();
    if total_mass <= 0.0 {
        return;
    }
    let magnitude = crate::organs::vomit_throw_magnitude(
        settings.scenario.throwing_force_factor,
        record.mouth_derived.jaw_strength,
        fraction,
        total_mass,
        record.mouth_derived.jaw_mass,
    );
    // Impulse `up * magnitude * m / total` on a pellet of mass `m` is a
    // velocity change of `up * magnitude / total` for every pellet.
    let velocity = crate::math::unity::transform_up(record.rot) * (magnitude / total_mass);
    for id in spawned {
        let _ = ctx.pellets.set_velocity(id, velocity);
    }
}

fn release_grab(record: &mut BibiteRecord, ctx: &mut BibiteTickCtx<'_>) {
    let count = record.n_held_targets as usize;
    if count == 0 {
        return;
    }
    // `desireToGrab > -0.25` lets go without a throw.
    if record.mouth.desire_grab <= -0.25 {
        ctx.scratch.throws.push(PendingThrow {
            holder: record.id,
            prey: (0..count).filter_map(|index| bibite_id_from_tag(record.held_targets[index])).collect(),
            pellets: (0..count).filter_map(|index| pellet_id_from_tag(record.held_targets[index])).collect(),
            up: crate::math::unity::transform_up(record.rot),
            n_held: count as f32,
            throwing_force_factor: ctx.settings.scenario.throwing_force_factor,
            jaw_strength: record.mouth_derived.jaw_strength,
            desire: record.mouth.desire_grab,
            jaw_mass: record.mouth_derived.jaw_mass,
        });
    }
    for index in 0..count {
        ctx.scratch.pending_releases.push((bibite_tag(record.id), record.held_targets[index]));
        if let Some(id) = pellet_id_from_tag(record.held_targets[index]) {
            ctx.pellets.set_held(id, false);
        }
    }
    record.n_held_targets = 0;
    record.mouth.n_held = 0;
}

/// The physics step's events cut into one row of items per bibite body: what the sweep after the step reads.
///
/// An item is `event number << 1 | side` (side 0: the bibite is body A of the event, side 1: body B). The items of a row are in
/// the order of the events, the order the events were walked in one by one, so a record sees its events as it always did. A row
/// is a linked list: `head[i]` is the first item of bibite body `i` (or [`NO_ITEM`]), `next[item]` the one after it. (Counting
/// the events into rows needed a prefix sum over every body, and almost no body has an event.)
#[derive(Clone, Debug, Default)]
struct BridgeRows {
    contact_head: Vec<u32>,
    contact_next: Vec<u32>,
    /// The same for the triggers whose sensor is on the bibite's body.
    trigger_head: Vec<u32>,
    trigger_next: Vec<u32>,
    /// Dense index of every record slot (`u32::MAX`: no live bibite), for the sweep over slot ranges.
    rank: Vec<u32>,
}

/// The collider pairs that touched in a step: only asked about (never walked), and asked about for every contact of every step.
type CollisionSet =
    std::collections::HashSet<(u64, u64, u8, u8), std::hash::BuildHasherDefault<FastHasher>>;

/// A fast hasher for the keys of [`CollisionSet`]: the standard one costs more than the rest of the loop it is used in. The
/// sets are only asked whether they hold a key, so the hash does not decide anything but speed.
#[derive(Clone, Copy, Default)]
struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl std::hash::Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.add(u64::from(*byte));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }
}

/// The end of a row of [`BridgeRows`].
const NO_ITEM: u32 = u32::MAX;

impl BridgeRows {
    /// Cuts the events into rows; `bibite_count` bodies (the first of the step) have one.
    fn cut(
        &mut self,
        bibite_count: usize,
        contacts: &[phys::ContactEvent],
        triggers: &[phys::TriggerEvent],
    ) {
        cut_rows(
            bibite_count,
            contacts,
            |contact| [Some(contact.body_a), Some(contact.body_b)],
            &mut self.contact_head,
            &mut self.contact_next,
        );
        cut_rows(
            bibite_count,
            triggers,
            |trigger| {
                [
                    trigger.sensor_a.then_some(trigger.body_a),
                    trigger.sensor_b.then_some(trigger.body_b),
                ]
            },
            &mut self.trigger_head,
            &mut self.trigger_next,
        );
    }
}

/// Links the events that name each of the first `count` bodies into rows: the events are walked backwards and each item is put
/// at the head of its row, so a row reads in the order of the events (and, within an event, side 0 before side 1).
fn cut_rows<E>(
    count: usize,
    events: &[E],
    sides: impl Fn(&E) -> [Option<usize>; 2],
    head: &mut Vec<u32>,
    next: &mut Vec<u32>,
) {
    head.clear();
    head.resize(count, NO_ITEM);
    next.clear();
    next.resize(events.len() * 2, NO_ITEM);
    for (number, event) in events.iter().enumerate().rev() {
        let bodies = sides(event);
        for side in (0..2usize).rev() {
            if let Some(body) = bodies[side]
                && body < count
            {
                let item = ((number as u32) << 1) | side as u32;
                next[item as usize] = head[body];
                head[body] = item;
            }
        }
    }
}

/// What the sweep after the physics step shares between the records (read only).
struct AfterStep<'a> {
    bodies: &'a [phys::PhysicsBody],
    /// Where the step left every body (the bibites' bodies were not written back).
    solver: &'a [phys::SolverBody],
    contacts: &'a [phys::ContactEvent],
    triggers: &'a [phys::TriggerEvent],
    rows: &'a BridgeRows,
    /// Bodies `0..bibite_count` are bibites, `bibite_count..pellet_end` pellets, the rest eggs.
    bibite_count: usize,
    pellet_end: usize,
    settings: &'a SimSettings,
    /// The epoch of the body tick: a record stamped with it has its rigid body and sizes in `derived` already.
    epoch: u32,
}

/// Everything the physics step leaves to do to the record of bibite body `index`, in the order the serial bridge did it: the
/// contact counters of the last step and the tags it touched (for the enter events of the next one), the pose and the rigid-body
/// cache from the body, and the mouth's target list from the triggers. Reads and writes this record only.
///
/// Returns whether the step left the bibite with a pose that is not a number (see [`write_back_bibite_pose`]).
fn after_step(record: &mut BibiteRecord, index: usize, after: &AfterStep<'_>) -> bool {
    // The counters and sums of the last step are set together (a contact sets the count and the rest in the same turn), so with
    // none counted they are all zero already.
    if record.last_contact_count != 0 {
        record.last_contact_count = 0;
        record.last_contact_impulse = 0.0;
        record.last_contact_bibite = 0;
        record.last_contact_pellet = 0;
        record.last_contact_egg = 0;
        record.last_contact_collider = [0; 3];
        record.last_contact_enter_count = 0;
        record.last_contact_enter_impulse = 0.0;
    }
    let mut current_tags = [0u64; 8];
    let mut current_count = 0usize;
    let rows = after.rows;
    let mut item = rows.contact_head[index];
    let touched = item != NO_ITEM;
    while item != NO_ITEM {
        let current = item;
        item = rows.contact_next[current as usize];
        let item = current;
        let contact = &after.contacts[(item >> 1) as usize];
        let (other_index, collider_index) = if item & 1 == 0 {
            (contact.body_b, contact.collider_a as usize)
        } else {
            (contact.body_a, contact.collider_b as usize)
        };
        let other_tag = after
            .bodies
            .get(other_index)
            .map(|body| body.tag)
            .unwrap_or(u64::MAX);
        record.last_contact_count = record.last_contact_count.saturating_add(1);
        record.last_contact_impulse += contact.normal_impulse.abs();
        if collider_index < record.last_contact_collider.len() {
            record.last_contact_collider[collider_index] =
                record.last_contact_collider[collider_index].saturating_add(1);
        }
        if other_index < after.bibite_count {
            record.last_contact_bibite = record.last_contact_bibite.saturating_add(1);
        } else if other_index < after.pellet_end {
            record.last_contact_pellet = record.last_contact_pellet.saturating_add(1);
        } else {
            record.last_contact_egg = record.last_contact_egg.saturating_add(1);
        }
        if !record.prev_contact_tags[..record.n_prev_contact_tags as usize].contains(&other_tag) {
            record.last_contact_enter_count = record.last_contact_enter_count.saturating_add(1);
            record.last_contact_enter_impulse += contact.normal_impulse.abs();
        }
        if current_count < current_tags.len() && !current_tags[..current_count].contains(&other_tag)
        {
            current_tags[current_count] = other_tag;
            current_count += 1;
        }
    }
    // A bibite with no contact now and none before has nothing to forget: the list is zeroes already.
    if touched || record.n_prev_contact_tags != 0 {
        record.prev_contact_tags = [0; 8];
        record.prev_contact_tags[..current_count].copy_from_slice(&current_tags[..current_count]);
        record.n_prev_contact_tags = current_count as u8;
    }
    // Write the pose back (a bibite's body is always active and dynamic, so the solver has its final state).
    let solved = &after.solver[index];
    let lost = write_back_bibite_pose(
        record,
        BridgePose {
            pos: solved.pos,
            rot: solved.rot,
            vel: solved.vel,
            ang_vel: solved.ang_vel,
        },
        &after.bodies[index],
        after.settings,
        record.phys_stamp == after.epoch,
    );
    // Mouth trigger lists from sensor overlaps. A trigger event is emitted once per collider pair, but a sensor-sensor pair
    // (mouth vs mouth) belongs to both bodies: each sensor adds the other body as a target.
    let mut item = rows.trigger_head[index];
    let triggered = item != NO_ITEM;
    while item != NO_ITEM {
        let current = item;
        item = rows.trigger_next[current as usize];
        let item = current;
        let trigger = &after.triggers[(item >> 1) as usize];
        record.push_mouth_target(if item & 1 == 0 {
            trigger.tag_b
        } else {
            trigger.tag_a
        });
    }
    // With no target before and no trigger now there is nothing to prune (`mouth.n_in_mouth` is zero with the count).
    if triggered || record.n_mouth_targets != 0 {
        record.prune_mouth_targets();
    }
    lost
}

/// The serial loops that [`after_step`] and its sweep replaced, as they were, on a copy of the records (tests compare the two).
#[cfg(test)]
fn after_step_reference(
    records: &mut [Option<BibiteRecord>],
    ids: &[BibiteId],
    bodies: &[phys::PhysicsBody],
    contacts: &[phys::ContactEvent],
    triggers: &[phys::TriggerEvent],
    pellet_end: usize,
    settings: &SimSettings,
) {
    let slot = |id: &BibiteId| id.handle().slot() as usize;
    for id in ids {
        if let Some(record) = records[slot(id)].as_mut() {
            record.last_contact_count = 0;
            record.last_contact_impulse = 0.0;
            record.last_contact_bibite = 0;
            record.last_contact_pellet = 0;
            record.last_contact_egg = 0;
            record.last_contact_collider = [0; 3];
            record.last_contact_enter_count = 0;
            record.last_contact_enter_impulse = 0.0;
        }
    }
    let mut current_tags = vec![[0u64; 8]; ids.len()];
    let mut current_counts = vec![0u8; ids.len()];
    for contact in contacts {
        for (self_index, other_index) in [
            (contact.body_a, contact.body_b),
            (contact.body_b, contact.body_a),
        ] {
            if self_index >= ids.len() {
                continue;
            }
            let other_tag = bodies
                .get(other_index)
                .map(|body| body.tag)
                .unwrap_or(u64::MAX);
            let Some(record) = records[slot(&ids[self_index])].as_mut() else {
                continue;
            };
            record.last_contact_count = record.last_contact_count.saturating_add(1);
            record.last_contact_impulse += contact.normal_impulse.abs();
            let collider_index = if self_index == contact.body_a {
                contact.collider_a as usize
            } else {
                contact.collider_b as usize
            };
            if collider_index < record.last_contact_collider.len() {
                record.last_contact_collider[collider_index] =
                    record.last_contact_collider[collider_index].saturating_add(1);
            }
            if other_index < ids.len() {
                record.last_contact_bibite = record.last_contact_bibite.saturating_add(1);
            } else if other_index < pellet_end {
                record.last_contact_pellet = record.last_contact_pellet.saturating_add(1);
            } else {
                record.last_contact_egg = record.last_contact_egg.saturating_add(1);
            }
            if !record.prev_contact_tags[..record.n_prev_contact_tags as usize].contains(&other_tag)
            {
                record.last_contact_enter_count = record.last_contact_enter_count.saturating_add(1);
                record.last_contact_enter_impulse += contact.normal_impulse.abs();
            }
            let count = current_counts[self_index] as usize;
            if count < current_tags[self_index].len()
                && !current_tags[self_index][..count].contains(&other_tag)
            {
                current_tags[self_index][count] = other_tag;
                current_counts[self_index] += 1;
            }
        }
    }
    for (self_index, id) in ids.iter().enumerate() {
        if let Some(record) = records[slot(id)].as_mut() {
            let count = current_counts[self_index] as usize;
            record.prev_contact_tags = [0; 8];
            record.prev_contact_tags[..count].copy_from_slice(&current_tags[self_index][..count]);
            record.n_prev_contact_tags = count as u8;
        }
    }
    for (index, id) in ids.iter().enumerate() {
        if let Some(record) = records[slot(id)].as_mut() {
            let body = &bodies[index];
            let pose = BridgePose {
                pos: body.pos,
                rot: body.rot,
                vel: body.vel,
                ang_vel: body.ang_vel,
            };
            let _ = write_back_bibite_pose(record, pose, body, settings, false);
        }
    }
    for trigger in triggers {
        let mut add_target = |bibite_index: usize, other_tag: u64| {
            if bibite_index >= ids.len() {
                return;
            }
            if let Some(record) = records[slot(&ids[bibite_index])].as_mut() {
                record.push_mouth_target(other_tag);
            }
        };
        if trigger.sensor_a {
            add_target(trigger.body_a, trigger.tag_b);
        }
        if trigger.sensor_b {
            add_target(trigger.body_b, trigger.tag_a);
        }
    }
    for id in ids {
        if let Some(record) = records[slot(id)].as_mut() {
            record.prune_mouth_targets();
        }
    }
}

/// Every field the sweep after the step writes, as bits (tests compare records with it).
#[cfg(test)]
fn bridge_trace(record: &BibiteRecord) -> Vec<u64> {
    let f = |value: f32| u64::from(value.to_bits());
    let mut trace = vec![
        f(record.pos.x),
        f(record.pos.y),
        f(record.rot),
        f(record.vel.x),
        f(record.vel.y),
        f(record.ang_vel),
        f(record.pending_force.x),
        f(record.pending_force.y),
        f(record.pending_torque),
        f(record.derived.mass),
        f(record.derived.inertia),
        f(record.derived.linear_damping),
        f(record.derived.angular_damping),
        f(record.derived.d1_size),
        f(record.derived.d2_size),
        f(record.derived.base_body_area),
        f(record.derived.body_length),
        u64::from(record.last_contact_count),
        f(record.last_contact_impulse),
        u64::from(record.last_contact_bibite),
        u64::from(record.last_contact_pellet),
        u64::from(record.last_contact_egg),
        u64::from(record.last_contact_collider[0]),
        u64::from(record.last_contact_collider[1]),
        u64::from(record.last_contact_collider[2]),
        u64::from(record.last_contact_enter_count),
        f(record.last_contact_enter_impulse),
        u64::from(record.n_prev_contact_tags),
        u64::from(record.n_mouth_targets),
        record.mouth.n_in_mouth as u64,
    ];
    trace.extend(record.prev_contact_tags);
    trace.extend(record.mouth_targets);
    trace.extend(record.mouth_target_ages.iter().map(|age| u64::from(*age)));
    trace
}

/// Where the step left a bibite.
#[derive(Clone, Copy)]
struct BridgePose {
    pos: Vec2,
    rot: f32,
    vel: Vec2,
    ang_vel: f32,
}

/// Writes the pose of the step back into the record. `cache_valid`: the turn left the rigid body and the geometry copies in
/// `derived` as the body has them (see [`BibiteRecord::refresh_physics_cache`]), so they are not written again.
///
/// Returns `true` when the pose the step left is not a number (a position, a rotation or a speed that is NaN or infinite): it is not
/// written (the bibite keeps its last pose and stands still) and the caller kills the bibite. A body with a NaN pose stays NaN for
/// good (every number made from it is), touches nothing and is touched by nothing, so it can neither eat nor be eaten nor be hurt,
/// and the eggs it lays are born at NaN: in the glitch save of 2026-11-13 such bibites had made 257,000 more.
fn write_back_bibite_pose(
    record: &mut BibiteRecord,
    pose: BridgePose,
    body: &phys::PhysicsBody,
    settings: &SimSettings,
    cache_valid: bool,
) -> bool {
    // One test for the six numbers: a NaN or an infinity in any of them makes the sum one (and a sum of finite poses overflows only
    // at 1e38 units, a pose that is lost as well).
    let lost = !(pose.pos.x + pose.pos.y + pose.rot + pose.vel.x + pose.vel.y + pose.ang_vel).is_finite();
    if lost {
        record.vel = Vec2::ZERO;
        record.ang_vel = 0.0;
    } else {
        record.pos = pose.pos;
        record.rot = pose.rot;
        record.vel = pose.vel;
        record.ang_vel = pose.ang_vel;
    }
    record.pending_force = Vec2::ZERO;
    record.pending_torque = 0.0;
    if !cache_valid {
        // The bridge already built this rigid body; publish the current cache here
        // instead of rebuilding it at the end of every body tick.
        record.derived.mass = body.mass;
        record.derived.inertia = body.inertia;
        record.derived.linear_damping = body.linear_damping;
        record.derived.angular_damping = body.angular_damping;
        record.sync_derived_geometry();
    }
    record.age_mouth_targets();
    // `BibitePropulsion.cs:298-306`: the wrap is the `else` of the shade
    // avoidance branch (which always wins beyond the wrap radius) and only
    // moves `rb.position`, keeping velocity.
    if settings.scenario.world_wrapping
        && settings.scenario.shade_outside_of_bounds
        && !settings.scenario.shade_avoidance
    {
        let sim_size = settings.independents.simulation_size;
        if let Some(wrapped) =
            world_wrap_position(record.pos, record.body.geometry.body_length, sim_size)
        {
            record.pos = wrapped;
        }
    }
    lost
}

/// The place of the pellet `id` in `ids`, the pellets of the step in dense order (every one of them live): the answer of a search through
/// the list from the front, found by halving it, since a holder asks for each pellet it holds and a world with thousands of grabbers and
/// tens of thousands of awake pellets made those searches the most expensive thing the bridge did. A stale id (its slot holds a newer
/// pellet) is not in the list, as before.
fn pellet_offset(pellets: &PelletStore, ids: &[PelletId], id: PelletId) -> Option<usize> {
    let found = pellets
        .dense_position(id.handle().slot() as usize)
        .and_then(|dense| {
            let at = ids.partition_point(|other| {
                pellets
                    .dense_position(other.handle().slot() as usize)
                    .is_some_and(|place| place < dense)
            });
            (ids.get(at) == Some(&id)).then_some(at)
        });
    #[cfg(test)]
    assert_eq!(
        found,
        ids.iter().position(|other| *other == id),
        "the pellets of the step are not in dense order"
    );
    found
}

/// The physics body of one pellet (`None` when the id is not in the store).
fn build_pellet_physics_body(
    pellets: &PelletStore,
    id: PelletId,
    settings: &SimSettings,
) -> Option<phys::PhysicsBody> {
    let view = pellets.get(id)?;
    let scale = (view.amount / 20.0).max(0.0).sqrt();
    let radius = phys::prefab::PELLET_CIRCLE_RADIUS * scale;
    let density = pellets.materials().get(view.material).mass_density;
    let mass = (view.amount * density).max(0.000_1);
    let mut body = phys::PhysicsBody::new_dynamic(
        pellet_tag(id),
        phys::LAYER_PELLETS,
        mass,
        mass * 0.5 * radius * radius,
    );
    let pose = pellets.pose(id).unwrap_or_default();
    body.pos = pose.pos;
    body.rot = pose.rotation;
    body.vel = pose.velocity;
    body.ang_vel = pose.angular_velocity;
    // RUSTibites Improved: a hydrothermal vent's pellet in flight has a lower drag (the factor is 1 for every other pellet, so this changes nothing for them).
    body.linear_damping = settings.scenario.drag_coefficient * scale / mass * pellets.drag_factor(id).unwrap_or(1.0);
    if settings.scenario.disable_drag_on_held_pellet && pellets.is_held(id) {
        body.linear_damping = 0.0;
    }
    body.angular_damping = settings.scenario.drag_coefficient / 2.0;
    let (friction, restitution) = match view.material {
        MaterialKind::Plant => (
            phys::prefab::PLANT_FRICTION,
            phys::prefab::PLANT_RESTITUTION,
        ),
        MaterialKind::Meat => (phys::prefab::MEAT_FRICTION, phys::prefab::MEAT_RESTITUTION),
        _ => (
            phys::prefab::CORPS_FRICTION,
            phys::prefab::CORPS_RESTITUTION,
        ),
    };
    // `MatterPellet.ToggleCollision`: without pellet collision the collider is a trigger and the rotation is frozen.
    let collider = if settings.scenario.pellet_collision {
        phys::Collider::solid(phys::Shape::circle(radius)).with_material(friction, restitution)
    } else {
        body.ang_vel = 0.0;
        phys::Collider::sensor(phys::Shape::circle(radius))
    };
    let _ = body.add_collider(collider);
    body.grabbable = true;
    Some(body)
}

/// The physics body of an egg (`EggHatching.UpdateDrag`, cs:154-157).
fn build_egg_physics_body(egg: &EggRecord, id: EggId, settings: &SimSettings) -> phys::PhysicsBody {
    let scale = egg.size_gene.max(0.0);
    let radius = phys::prefab::EGG_CIRCLE_RADIUS * scale;
    let mass = (egg.genome.area_at_birth() * settings.scenario.bibite_mass_density).max(0.000_1);
    let mut body = phys::PhysicsBody::new_dynamic(
        egg_tag(id),
        phys::LAYER_BIBITE,
        mass,
        mass * 0.5 * radius * radius,
    );
    body.pos = egg.pos;
    body.rot = egg.rot;
    body.vel = egg.vel;
    body.ang_vel = egg.ang_vel;
    body.linear_damping = settings.scenario.drag_coefficient * scale / mass;
    body.angular_damping = 1.0;
    let _ = body.add_collider(
        phys::Collider::solid(phys::Shape::circle(radius))
            .with_offset(phys::prefab::EGG_CIRCLE_OFFSET * scale)
            .with_material(
                phys::prefab::CORPS_FRICTION,
                phys::prefab::CORPS_RESTITUTION,
            ),
    );
    body
}

/// What the resting-pellet grid is laid out for.
fn pellet_sleep_stamp(settings: &SimSettings) -> [u32; STAMP_LEN] {
    [settings.independents.simulation_size.to_bits()]
}

/// The box the promotion pass asks the resting pellets about, for body `index` of the step: the one the build made when there
/// is one, else the body's own (`None`: it takes no part, so it overlaps no sleeper).
fn promotion_box(
    known: &[phys::BodyBox],
    index: usize,
    body: &phys::PhysicsBody,
) -> Option<phys::Aabb> {
    match known.get(index) {
        Some(boxed) => boxed.valid.then_some(boxed.aabb),
        None => {
            if body.active {
                body.swept_aabb()
            } else {
                None
            }
        }
    }
}

/// The rigid body a bibite's physics body is made of: from the cache its turn left in `derived` when that is current, else from its
/// organs.
fn bibite_rigid_body(
    record: &BibiteRecord,
    settings: &SimSettings,
    bs: &BodySettings,
    epoch: u32,
) -> RigidBodyProperties {
    if record.phys_stamp == epoch {
        // The turn left the rigid body in `derived`, clamped the way the body wants it.
        #[cfg(test)]
        {
            let fresh = record.rigid_body_now(
                settings,
                bs,
                &organ_material(&settings.materials.fat),
                &organ_material(&settings.materials.armor),
            );
            assert_eq!(
                [
                    record.derived.mass.to_bits(),
                    record.derived.inertia.to_bits(),
                    record.derived.linear_damping.to_bits(),
                    record.derived.angular_damping.to_bits(),
                ],
                [
                    fresh.mass.max(0.000_1).to_bits(),
                    fresh.inertia.max(0.000_1).to_bits(),
                    fresh.linear_damping.to_bits(),
                    fresh.angular_damping.to_bits(),
                ],
                "a stamped physics cache is not the record's rigid body"
            );
        }
        RigidBodyProperties {
            mass: record.derived.mass,
            inertia: record.derived.inertia,
            linear_damping: record.derived.linear_damping,
            angular_damping: record.derived.angular_damping,
        }
    } else {
        record.rigid_body_now(
            settings,
            bs,
            &organ_material(&settings.materials.fat),
            &organ_material(&settings.materials.armor),
        )
    }
}

/// The physics body of a bibite, made from its record.
#[inline]
fn build_bibite_physics_body(
    record: &BibiteRecord,
    settings: &SimSettings,
    bs: &BodySettings,
    dt: f32,
    epoch: u32,
) -> phys::PhysicsBody {
    let rb = bibite_rigid_body(record, settings, bs, epoch);
    let mass = rb.mass.max(0.000_1);
    let inertia = rb.inertia.max(0.000_1);
    let mut body =
        phys::PhysicsBody::new_dynamic(bibite_tag(record.id), phys::LAYER_BIBITE, mass, inertia);
    body.pos = record.pos;
    body.rot = record.rot;
    body.vel = record.vel;
    body.ang_vel = record.ang_vel;
    body.force = record.pending_force;
    body.torque = record.pending_torque;
    body.linear_damping = rb.linear_damping;
    body.angular_damping = rb.angular_damping;
    for collider in record.colliders() {
        let _ = body.add_collider(collider);
    }
    body.grabbable = true;
    set_grab_reach(&mut body, record, dt);
    body
}

/// What a mouth that wants to grab needs of the broadphase: its box reaches as far as it (and what it meets) can move in a tick.
fn set_grab_reach(body: &mut phys::PhysicsBody, record: &BibiteRecord, dt: f32) {
    body.grab_slots = grab_slots_of(record);
    body.box_margin = if body.grab_slots > 0 { (record.vel.length() + GRAB_REACH_SPEED) * dt } else { 0.0 };
}

/// The speed (u/s) of what a grabbing mouth may meet, on top of its own, for the box it asks the broadphase for.
const GRAB_REACH_SPEED: f32 = 30.0;

/// How many more bodies the bibite's mouth takes this tick: `desireToGrab >= 0.15` and links left (`links` holds ten).
fn grab_slots_of(record: &BibiteRecord) -> u8 {
    if record.mouth.desire_grab >= MOUTH_DEAD_ZONE && !record.is_corpse {
        (record.held_targets.len() as u8).saturating_sub(record.n_held_targets)
    } else {
        0
    }
}

/// Brings the physics body `body` of a bibite up to date: the body [`build_bibite_physics_body`] makes, but a body that still belongs to
/// the bibite (its tag) and has the record's colliders (`colliders_gen`) is updated in place; only the pose, the velocities, the mass
/// and the damping are given again, and the colliders (108 of its 192 bytes) are not touched until the bibite's size changes.
fn update_bibite_physics_body(
    body: &mut phys::PhysicsBody,
    colliders_gen: &mut u32,
    record: &BibiteRecord,
    settings: &SimSettings,
    bs: &BodySettings,
    dt: f32,
    epoch: u32,
) {
    body.attached_to = 0;
    if body.tag != bibite_tag(record.id) || *colliders_gen != record.colliders_gen {
        *body = build_bibite_physics_body(record, settings, bs, dt, epoch);
        *colliders_gen = record.colliders_gen;
        return;
    }
    let rb = bibite_rigid_body(record, settings, bs, epoch);
    let mass = rb.mass.max(0.000_1);
    let inertia = rb.inertia.max(0.000_1);
    body.set_mass(mass, inertia);
    body.pos = record.pos;
    body.rot = record.rot;
    body.vel = record.vel;
    body.ang_vel = record.ang_vel;
    body.force = record.pending_force;
    body.torque = record.pending_torque;
    body.linear_damping = rb.linear_damping;
    body.angular_damping = rb.angular_damping;
    set_grab_reach(body, record, dt);
}

// ---------------------------------------------------------------------------
// B2b physics bridge
// ---------------------------------------------------------------------------

impl BibiteStore {
    /// Builds the physics mirrors, steps the solver and writes poses back.
    ///
    /// Call order per tick: `world.set_senses(...)` → `store.tick(ctx)` →
    /// `store.step_physics(...)`. The bridge applies pending movement forces as
    /// velocity deltas before the step, routes solid contacts to two-sided
    /// collision damage (BQ-4: `dot(normal, relativeVelocity)` is approach
    /// positive, so the closing speed is `-(normal · (v_b − v_a))`), and
    /// rebuilds each bibite's mouth target list from the sensor triggers.
    ///
    /// Held pellets are pinned to the holder's mouth while the grab lasts (the
    /// body-side impulse constraint approximation documented in B2b).
    pub fn step_physics(
        &mut self,
        pellets: &mut PelletStore,
        eggs: &mut EggStore,
        settings: &SimSettings,
        rng: &mut crate::rng::Rng,
        scratch: &mut BibiteScratch,
        events: &mut Vec<BibiteEvent>,
    ) {
        self.step_physics_inner(pellets, eggs, settings, rng, scratch, events, None);
    }

    /// Bridge step with a world-owned executor (physics parallel stages).
    #[allow(clippy::too_many_arguments)]
    pub fn step_physics_with_executor(
        &mut self,
        pellets: &mut PelletStore,
        eggs: &mut EggStore,
        settings: &SimSettings,
        rng: &mut crate::rng::Rng,
        scratch: &mut BibiteScratch,
        events: &mut Vec<BibiteEvent>,
        executor: crate::sim::ParallelExecutor<'_>,
    ) {
        self.step_physics_inner(
            pellets,
            eggs,
            settings,
            rng,
            scratch,
            events,
            Some(executor),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn step_physics_inner(
        &mut self,
        pellets: &mut PelletStore,
        eggs: &mut EggStore,
        settings: &SimSettings,
        rng: &mut crate::rng::Rng,
        scratch: &mut BibiteScratch,
        events: &mut Vec<BibiteEvent>,
        executor: Option<crate::sim::ParallelExecutor<'_>>,
    ) {
        let mut lap = std::time::Instant::now();
        let bs = BodySettings::from_sim_settings(settings);
        let dt = 1.0 / settings.sim_tps.max(1) as f32;
        let epoch = self.epoch;
        let config = phys::PhysicsConfig { held_drag_off: settings.scenario.disable_drag_on_held_pellet, ..phys::PhysicsConfig::default() };
        let mut bodies = std::mem::take(&mut scratch.physics_bodies);
        let mut body_gens = std::mem::take(&mut scratch.bibite_body_gens);
        let threads = executor.map_or(1, |executor| executor.threads());
        let record_workers =
            crate::sim::ParallelGates::workers_for(self.dense.len(), self.gates.records, threads);
        let parallel = record_workers > 1;
        // On the pool the bibites' bodies of the last step are kept (the pellets' and eggs' bodies behind them go): each is
        // brought up to date below, in place when it is still the same bibite's with the same colliders. Anywhere else the
        // vector starts again, and a step that returns early leaves nothing to keep.
        let kept = if parallel {
            scratch.kept_bibite_bodies.min(bodies.len())
        } else {
            0
        };
        scratch.kept_bibite_bodies = 0;
        bodies.truncate(kept);
        if !parallel {
            body_gens.clear();
        }
        bodies.reserve(self.len() + pellets.len() + eggs.len());
        let mut bibite_ids = std::mem::take(&mut scratch.bibite_ids);
        bibite_ids.clear();
        // The bodies are built straight into place in dense order (each one reads only its own record), so the pool's
        // workers fill disjoint stretches of the vector and nothing has to be sorted or merged afterwards.
        // The bibites' boxes are made here, where the body is in the cache: the pass below that asks the resting pellets who
        // is overlapped, and the step, reuse them instead of computing every box again.
        let boxes = scratch.physics.boxes_for_next_step();
        boxes.clear();
        // The bibites that hold something, as dense positions, found by the build, which looks at every record anyway.
        let held_dense: Option<Vec<u32>>;
        if parallel {
            let records = &self.records;
            let generations = &self.generations;
            let count = self.dense.len();
            let min_len = crate::sim::ParallelGates::chunk_len(
                count,
                self.gates.records,
                record_workers,
                1,
            );
            // The kept bodies are the first ones (those of the same dense positions as last time); a body that is not the
            // bibite's now is made again. New positions get a body no bibite's tag or generation matches.
            bodies.truncate(count);
            bodies.resize(count, phys::PhysicsBody::new_static(0, phys::LAYER_BIBITE));
            body_gens.truncate(count);
            body_gens.resize(count, 0);
            boxes.clear();
            boxes.resize(count, phys::BodyBox::INVALID);
            let held_parts: Vec<Vec<u32>> = bodies
                .par_chunks_mut(min_len)
                .zip(body_gens.par_chunks_mut(min_len))
                .zip(boxes.par_chunks_mut(min_len))
                .zip(self.dense.par_chunks(min_len))
                .enumerate()
                .map(|(chunk, (((bodies, gens), boxes), slots))| {
                    let mut held = Vec::new();
                    for (offset, (((body, generation), boxed), slot)) in bodies
                        .iter_mut()
                        .zip(gens)
                        .zip(boxes)
                        .zip(slots)
                        .enumerate()
                    {
                        let record = records[*slot as usize]
                            .as_ref()
                            .expect("every dense slot holds a record");
                        update_bibite_physics_body(
                            body, generation, record, settings, &bs, dt, epoch,
                        );
                        *boxed = phys::BodyBox::of(body);
                        if record.n_held_targets > 0 {
                            held.push((chunk * min_len + offset) as u32);
                        }
                    }
                    held
                })
                .collect();
            held_dense = Some(held_parts.concat());
            #[cfg(test)]
            for (index, slot) in self.dense.iter().enumerate() {
                let record = self.records[*slot as usize]
                    .as_ref()
                    .expect("every dense slot holds a record");
                let fresh = build_bibite_physics_body(record, settings, &bs, dt, epoch);
                assert_eq!(
                    format!("{:?}", bodies[index]),
                    format!("{fresh:?}"),
                    "the physics body kept for dense position {index} is not the one a fresh build makes"
                );
            }
            // The ids come from the generations, which are packed: the records themselves are not touched again.
            bibite_ids.par_extend(self.dense.par_iter().with_min_len(min_len).map(|slot| {
                BibiteId::from_handle(Handle::new(*slot, generations[*slot as usize]))
            }));
        } else {
            boxes.clear();
            // The same list as on the pool, made in the loop that reads every record anyway: a scan for the holders afterwards
            // is another pass over all the records.
            let mut held = Vec::new();
            for id in self.ids() {
                let Some(record) = self.get(id) else {
                    continue;
                };
                let body = build_bibite_physics_body(record, settings, &bs, dt, epoch);
                boxes.push(phys::BodyBox::of(&body));
                if record.n_held_targets > 0 {
                    held.push(bodies.len() as u32);
                }
                bodies.push(body);
                bibite_ids.push(id);
            }
            held_dense = Some(held);
        }
        let bibite_count = bodies.len();
        scratch.physics.boxes_ready(bibite_count);
        scratch.physics.skip_write_back_of(bibite_count);
        scratch.clock.lap(crate::sim::Stage::PhysBuild, &mut lap);
        // Pellets. With enough of them, the ones at rest are left out of the step (see `pellet_sleep`): only the awake ones are
        // built here, and the sleepers that a body of the step overlaps join it below.
        let mut pellet_ids = std::mem::take(&mut scratch.pellet_ids);
        pellet_ids.clear();
        let mut sleep = std::mem::take(&mut scratch.sleep);
        let sleep_gate = self.gates.sleep_pellets;
        // The layer costs a grid query for every body of the step and saves the physics of the pellets at rest, a few hundred
        // nanoseconds each: it pays when the pellets are a good part of the bibites (a quarter to start, an eighth to go on) and
        // loses when they are a few in a hundred.
        let bibites = self.dense.len();
        let sleep_wanted = sleep_gate != usize::MAX
            && if sleep.enabled() {
                pellets.len().saturating_mul(2) >= sleep_gate
                    && pellets.len().saturating_mul(8) >= bibites
            } else {
                pellets.len() >= sleep_gate && pellets.len().saturating_mul(4) >= bibites
            };
        if !sleep_wanted {
            sleep.disable();
        }
        let pellet_workers = crate::sim::ParallelGates::workers_for(
            pellets.len(),
            self.gates.bodies,
            executor.map_or(1, |executor| executor.threads()),
        );
        if sleep_wanted {
            sleep.begin_tick(
                pellets,
                pellet_sleep_stamp(settings),
                // Pellets are culled beyond `1.5 S + 500` (`S` is the half-size of the world).
                settings.independents.simulation_size * crate::world::SHADE_START_FACTOR
                    + crate::world::PELLET_CULL_MARGIN,
            );
            let mut awake: Vec<(u32, u32)> = sleep
                .awake_slots()
                .iter()
                .filter_map(|&slot| {
                    pellets
                        .dense_position(slot as usize)
                        .map(|dense| (dense, slot))
                })
                .collect();
            awake.sort_unstable();
            // The bodies of the awake pellets: each reads only its own pellet, so with enough of them the pool fills them in place.
            let awake_workers = crate::sim::ParallelGates::workers_for(
                awake.len(),
                self.gates.bodies,
                executor.map_or(1, |executor| executor.threads()),
            );
            if awake_workers > 1 {
                pellet_ids.extend(
                    awake
                        .iter()
                        .filter_map(|&(_, slot)| pellets.live_id(slot as usize)),
                );
                let pellets_ref: &PelletStore = pellets;
                let min_len = crate::sim::ParallelGates::chunk_len(
                    pellet_ids.len(),
                    self.gates.bodies,
                    awake_workers,
                    1,
                );
                bodies.par_extend(pellet_ids.par_iter().with_min_len(min_len).map(|id| {
                    build_pellet_physics_body(pellets_ref, *id, settings)
                        .expect("`live_id` names live pellets only")
                }));
            } else {
                for (_, slot) in awake {
                    let Some(id) = pellets.live_id(slot as usize) else {
                        continue;
                    };
                    let Some(body) = build_pellet_physics_body(pellets, id, settings) else {
                        continue;
                    };
                    bodies.push(body);
                    pellet_ids.push(id);
                }
            }
        } else if pellet_workers > 1 {
            // Each body reads only its own pellet: filled in place in id order by the pool.
            let pellets_ref: &PelletStore = pellets;
            pellet_ids.extend(pellets_ref.ids());
            let min_len = crate::sim::ParallelGates::chunk_len(
                pellet_ids.len(),
                self.gates.bodies,
                pellet_workers,
                1,
            );
            bodies.par_extend(pellet_ids.par_iter().with_min_len(min_len).map(|id| {
                build_pellet_physics_body(pellets_ref, *id, settings)
                    .expect("`ids` lists live pellets only")
            }));
        } else {
            for id in pellets.ids() {
                let Some(body) = build_pellet_physics_body(pellets, id, settings) else {
                    continue;
                };
                bodies.push(body);
                pellet_ids.push(id);
            }
        }
        let pellet_start = bibite_count;
        scratch
            .clock
            .lap(crate::sim::Stage::PhysBuildPellets, &mut lap);
        // Eggs.
        let mut egg_ids = std::mem::take(&mut scratch.egg_ids);
        egg_ids.clear();
        for id in eggs.ids() {
            let Some(egg) = eggs.get(id) else {
                continue;
            };
            bodies.push(build_egg_physics_body(egg, id, settings));
            egg_ids.push(id);
        }
        // What a holder holds is welded to it (the game's `FixedJoint2D`: the held body keeps the pose it had when it was grabbed, the joint gives
        // way at `breakForce`). The pairs collide, as the game's joints are made with `enableCollision`.
        let holders_now: Vec<BibiteId> = match &held_dense {
            Some(held) => held
                .iter()
                .map(|index| bibite_ids[*index as usize])
                .collect(),
            None => self.holders(executor.unwrap_or(crate::sim::ParallelExecutor::Serial)),
        };
        #[cfg(test)]
        assert_eq!(
            holders_now,
            self.holders(crate::sim::ParallelExecutor::Serial),
            "the holders the build listed are not the ones a scan finds"
        );
        if sleep_wanted {
            // The sleepers that a body of the step overlaps join it, in the place the dense order gives them (a pair needs
            // two overlapping boxes, and a sleeper nobody overlaps would not have taken part in anything).
            let promotion_workers = crate::sim::ParallelGates::workers_for(
                bodies.len(),
                self.gates.bodies,
                executor.map_or(1, |executor| executor.threads()),
            );
            if promotion_workers > 1 {
                // The boxes cost a sin/cos pair for the body and one for each collider (13 ms for 100,000 bibites in a
                // row), and asking the grid reads it only: the pool takes the bodies in stretches, and the sleepers are
                // marked afterwards in the order of the stretches, which is the order the plain loop marks them in.
                let stretch = crate::sim::ParallelGates::chunk_len(
                    bodies.len(),
                    self.gates.bodies,
                    promotion_workers,
                    4,
                );
                let grid: &crate::pellet_sleep::PelletSleep = &sleep;
                let known = scratch.physics.ready_boxes();
                let answers: Vec<Vec<u32>> = bodies
                    .par_chunks(stretch)
                    .enumerate()
                    .map(|(chunk_index, chunk)| {
                        let mut found = Vec::new();
                        for (offset, body) in chunk.iter().enumerate() {
                            if let Some(aabb) =
                                promotion_box(known, chunk_index * stretch + offset, body)
                            {
                                grid.overlapping(&aabb, &mut found);
                            }
                        }
                        found
                    })
                    .collect();
                for answer in &answers {
                    sleep.promote_slots(answer);
                }
            } else {
                let known = scratch.physics.ready_boxes();
                for (index, body) in bodies.iter().enumerate() {
                    if let Some(aabb) = promotion_box(known, index, body) {
                        sleep.promote_overlapping(&aabb);
                    }
                }
            }
            // A pellet let go of this tick keeps its joint for the tick's steps: it takes part in them whether or not it was resting.
            for &(_, tag) in &scratch.pending_releases {
                if let Some(pellet_id) = pellet_id_from_tag(tag) {
                    let slot = pellet_id.handle().slot();
                    if sleep.is_asleep(slot as usize) {
                        sleep.promote_slots(&[slot]);
                    }
                }
            }
            if !sleep.promoted().is_empty() {
                // The promoted sleepers take their places among the awake pellets' bodies, in dense order. The awake bodies are
                // in that order already and only a few sleepers are promoted (a hundred in fifty thousand pellets), so the few are
                // sorted and merged in, and every awake body is moved once: (sorting all of them, each 224 bytes with its key,
                // and moving them in and out of the sort's buffer cost as much as building the bodies of the awake pellets.)
                let mut promoted: Vec<(u32, PelletId, phys::PhysicsBody)> =
                    Vec::with_capacity(sleep.promoted().len());
                for &slot in sleep.promoted() {
                    let Some(id) = pellets.live_id(slot as usize) else {
                        continue;
                    };
                    let Some(body) = build_pellet_physics_body(pellets, id, settings) else {
                        continue;
                    };
                    let dense = pellets.dense_position(slot as usize).unwrap_or(u32::MAX);
                    promoted.push((dense, id, body));
                }
                promoted.sort_unstable_by_key(|entry| entry.0);
                let mut awake_bodies = bodies.split_off(pellet_start);
                let eggs_tail = awake_bodies.split_off(pellet_ids.len());
                let awake_ids = std::mem::take(&mut pellet_ids);
                pellet_ids.reserve(awake_ids.len() + promoted.len());
                let mut promoted = promoted.into_iter().peekable();
                for (id, body) in awake_ids.into_iter().zip(awake_bodies) {
                    let dense = pellets
                        .dense_position(id.handle().slot() as usize)
                        .unwrap_or(u32::MAX);
                    while promoted.peek().is_some_and(|entry| entry.0 < dense) {
                        if let Some((_, promoted_id, promoted_body)) = promoted.next() {
                            pellet_ids.push(promoted_id);
                            bodies.push(promoted_body);
                        }
                    }
                    pellet_ids.push(id);
                    bodies.push(body);
                }
                for (_, promoted_id, promoted_body) in promoted {
                    pellet_ids.push(promoted_id);
                    bodies.push(promoted_body);
                }
                bodies.extend(eggs_tail);
                #[cfg(test)]
                assert!(
                    pellet_ids.windows(2).all(|pair| {
                        let place = |id: &PelletId| pellets.dense_position(id.handle().slot() as usize);
                        place(&pair[0]) < place(&pair[1])
                    }),
                    "the pellets of the step are not in dense order after the promoted sleepers joined them"
                );
            }
            sleep.clear_promoted();
        }
        let mut weld_info: Vec<(u32, BibiteId, u64)> = Vec::new();
        let mut frames_now: Vec<(u64, u64, Vec2, f32)> = Vec::new();
        for id in holders_now {
            let Some(record) = self.get(id) else {
                continue;
            };
            let holder_at = self.dense_pos.get(id.handle().slot() as usize).copied().unwrap_or(u32::MAX) as usize;
            if holder_at >= bodies.len() {
                continue;
            }
            let holder_tag = bibite_tag(id);
            // The joint's `breakForce` (and `breakTorque`) is what `BibiteMouth.FixedUpdate` sets on every link each tick while the grab output is up:
            // `jawStrength * max(diet, 0.1) * desireToGrab / nHeld` (`grab_break_force`). A herbivore's grip is a tenth of a carnivore's: a shrek (diet 0) holding a plant
            // pellet in front of its mouth lets go of it within ten ticks of the first acceleration in the game (measured 2026-11-18 on 49 identical shrek, each with one
            // 70-unit pellet in its mouth: the hold is gone by tick 8 and is not made again), which a weld that never gave way did not reproduce: 63 % of the shrek of
            // `shrekvssabertoothtest1` held something at every moment here against 5 % in the game.
            // (The game only writes the new value while the grab output is at least 0.15 and leaves the last one in place below that: no value below the 0.15 one is ever set.)
            let break_force = crate::organs::grab_break_force(record.mouth_derived.jaw_strength, record.genome.gene(Gene::Diet), record.mouth.desire_grab.max(0.15), record.n_held_targets as usize);
            for held_index in 0..record.n_held_targets as usize {
                let tag = record.held_targets[held_index];
                let held_at = if let Some(prey) = bibite_id_from_tag(tag) {
                    // A prey that is gone is gone: its slot may already hold a newcomer (a hatchling takes the slot of what was just eaten, in the same tick), and a
                    // weld to the newcomer joined the holder to a bibite from anywhere in the world, with the frame the prey had: the rigid weld pulled the pair
                    // together across thousands of units in one step, and what the solver did with that threw bodies out of the world (the glitch save of 2026-11-13).
                    match self.dense_position_of(prey) {
                        Some(at) => at,
                        None => continue,
                    }
                } else if let Some(pellet_id) = pellet_id_from_tag(tag) {
                    match pellet_offset(pellets, &pellet_ids, pellet_id) {
                        Some(offset) => pellet_start + offset,
                        None => continue,
                    }
                } else {
                    continue;
                };
                if held_at >= bodies.len() || held_at == holder_at {
                    continue;
                }
                let known = scratch.weld_frames.binary_search_by_key(&(tag, holder_tag), |frame| (frame.0, frame.1)).ok();
                let (anchor, angle) = match known {
                    Some(at) => (scratch.weld_frames[at].2, scratch.weld_frames[at].3),
                    None => {
                        let (holder, held) = (&bodies[holder_at], &bodies[held_at]);
                        ((held.pos - holder.pos).rotate(-holder.rot), held.rot - holder.rot)
                    }
                };
                frames_now.push((tag, holder_tag, anchor, angle));
                scratch.physics.add_weld(phys::WeldDef { held: held_at as u32, holder: holder_at as u32, anchor, angle, break_force });
                weld_info.push((held_at as u32, id, tag));
            }
        }
        // The joints of the grabs let go of this tick are still there (the game destroys them at the end of the frame): welded for this tick's steps, forgotten after.
        for index in 0..scratch.pending_releases.len() {
            let (holder_tag, tag) = scratch.pending_releases[index];
            if frames_now.iter().any(|frame| frame.0 == tag && frame.1 == holder_tag) {
                continue;
            }
            let Some(holder_id) = bibite_id_from_tag(holder_tag) else {
                continue;
            };
            let Some(holder_at) = self.dense_position_of(holder_id) else {
                continue;
            };
            let held_at = if let Some(prey) = bibite_id_from_tag(tag) {
                match self.dense_position_of(prey) {
                    Some(at) => at,
                    None => continue,
                }
            } else if let Some(pellet_id) = pellet_id_from_tag(tag) {
                match pellet_offset(pellets, &pellet_ids, pellet_id) {
                    Some(offset) => pellet_start + offset,
                    None => continue,
                }
            } else {
                continue;
            };
            if holder_at >= bodies.len() || held_at >= bodies.len() || held_at == holder_at {
                continue;
            }
            let Some(at) = scratch.weld_frames.binary_search_by_key(&(tag, holder_tag), |frame| (frame.0, frame.1)).ok() else {
                continue;
            };
            let (anchor, angle) = (scratch.weld_frames[at].2, scratch.weld_frames[at].3);
            scratch.physics.add_weld(phys::WeldDef { held: held_at as u32, holder: holder_at as u32, anchor, angle, break_force: 0.0 });
        }
        scratch.pending_releases.clear();
        frames_now.sort_unstable_by_key(|frame| (frame.0, frame.1));
        scratch.weld_frames = frames_now;
        scratch
            .clock
            .lap(crate::sim::Stage::PhysBuildEggs, &mut lap);
        scratch.physics.threads = executor.map_or(1, |executor| executor.threads());
        scratch.physics.min_bodies = self.gates.bodies;
        scratch.physics.min_broadphase = self.gates.broadphase;
        scratch.physics.min_narrow_pairs = self.gates.narrow_pairs;
        scratch.physics.min_sort = self.gates.sorts;
        scratch.physics.min_island_bodies = self.gates.island_bodies;
        scratch.physics.min_island_constraints = self.gates.island_constraints;
        scratch.physics.min_grid = self.gates.grid_bodies;
        phys::step(&mut bodies, &config, dt, &mut scratch.physics);
        // The joints that gave way (`OnLinkBroke`): the holder lets go.
        for &(held_at, holder_tag) in scratch.physics.broken_welds() {
            // (A body can be held by several holders: the weld that gave way is the one of the holder the step names, not the first of the held body's.)
            let Some(&(_, holder_id, tag)) = weld_info.iter().find(|info| info.0 == held_at && bibite_tag(info.1) == holder_tag) else {
                continue;
            };
            if let Some(record) = self.get_mut(holder_id) {
                let count = record.n_held_targets as usize;
                if let Some(at) = record.held_targets[..count].iter().position(|t| *t == tag) {
                    record.held_targets[at] = record.held_targets[count - 1];
                    record.n_held_targets -= 1;
                    record.mouth.n_held = record.n_held_targets as usize;
                }
            }
            if let Some(pellet_id) = pellet_id_from_tag(tag) {
                pellets.set_held(pellet_id, false);
            }
        }
        // The grabs the steps made (the mouth saw the trigger enter at some step and grabbed in the next): they are held from now on.
        let made: Vec<phys::NewWeld> = scratch.physics.new_welds().to_vec();
        for weld in made {
            let holder_at = weld.holder as usize;
            if holder_at >= bibite_count || weld.held as usize >= bodies.len() {
                continue;
            }
            let (holder_tag, tag) = (bodies[holder_at].tag, bodies[weld.held as usize].tag);
            let Some(record) = self.get_mut(bibite_ids[holder_at]) else {
                continue;
            };
            if record.is_held(tag) || record.n_held_targets as usize >= record.held_targets.len() {
                continue;
            }
            if let Some(pellet_id) = pellet_id_from_tag(tag) {
                pellets.set_held(pellet_id, true);
            }
            record.add_held(tag);
            scratch.weld_frames.retain(|frame| !(frame.0 == tag && frame.1 == holder_tag));
            let at = scratch.weld_frames.partition_point(|frame| (frame.0, frame.1) < (tag, holder_tag));
            scratch.weld_frames.insert(at, (tag, holder_tag, weld.anchor, weld.angle));
        }
        // The step timed its own stages; the bridge's second half starts here.
        lap = std::time::Instant::now();
        let pellet_end = bibite_count + pellet_ids.len();
        #[cfg(test)]
        let reference = (self.dense.len() <= 300).then(|| {
            let mut records = self.records.clone();
            // The reference loops read the bibites' final poses from the bodies, where the step did not put them.
            let mut written_back = bodies.clone();
            for (body, solved) in written_back
                .iter_mut()
                .zip(scratch.physics.solved())
                .take(bibite_count)
            {
                body.pos = solved.pos;
                body.rot = solved.rot;
                body.vel = solved.vel;
                body.ang_vel = solved.ang_vel;
            }
            after_step_reference(
                &mut records,
                &bibite_ids,
                &written_back,
                scratch.physics.contacts(),
                scratch.physics.triggers(),
                pellet_end,
                settings,
            );
            records
        });
        // What the step leaves to do to a bibite's record (the contact bookkeeping, the pose, the mouth's trigger list) reads that
        // record, its body and its own events, and writes nothing else: with enough bibites the pool does it, one stretch of
        // records each, after the events were cut into one row of items per bibite (in the order of the events).
        scratch.bridge.cut(
            bibite_count,
            scratch.physics.contacts(),
            scratch.physics.triggers(),
        );
        let mut holders: Vec<u32> = Vec::new();
        let mut lost_poses: Vec<u32> = Vec::new();
        let bridge_workers =
            crate::sim::ParallelGates::workers_for(self.records.len(), self.gates.bodies, threads);
        if parallel && bridge_workers > 1 {
            self.copy_dense_ranks(&mut scratch.bridge.rank);
            let after = AfterStep {
                bodies: &bodies,
                solver: scratch.physics.solved(),
                contacts: scratch.physics.contacts(),
                triggers: scratch.physics.triggers(),
                rows: &scratch.bridge,
                bibite_count,
                pellet_end,
                settings,
                epoch,
            };
            let rank: &[u32] = &scratch.bridge.rank;
            let chunk_len = crate::sim::ParallelGates::chunk_len(
                self.records.len(),
                self.gates.bodies,
                bridge_workers,
                SWEEP_CHUNKS_PER_THREAD,
            );
            let parts: Vec<(Vec<u32>, Vec<u32>, Vec<u32>)> = self
                .records
                .par_chunks_mut(chunk_len)
                .map(|chunk| {
                    let mut held = Vec::new();
                    let mut mouthed = Vec::new();
                    let mut lost = Vec::new();
                    for entry in chunk.iter_mut() {
                        let Some(record) = entry.as_mut() else {
                            continue;
                        };
                        let slot = record.id.handle().slot();
                        let index = rank[slot as usize];
                        if index == u32::MAX {
                            continue;
                        }
                        if after_step(record, index as usize, &after) {
                            lost.push(index);
                        }
                        if record.n_held_targets > 0 {
                            held.push(index);
                        }
                        if record.n_mouth_targets > 0 {
                            mouthed.push(slot);
                        }
                    }
                    (held, mouthed, lost)
                })
                .collect();
            self.mouth_candidates.clear();
            for (held, mouthed, lost) in parts {
                holders.extend(held);
                self.mouth_candidates.extend(mouthed);
                lost_poses.extend(lost);
            }
            self.mouth_candidates_valid = true;
            // The stretches are slot ranges: dense order is restored here.
            holders.sort_unstable();
            lost_poses.sort_unstable();
        } else {
            let after = AfterStep {
                bodies: &bodies,
                solver: scratch.physics.solved(),
                contacts: scratch.physics.contacts(),
                triggers: scratch.physics.triggers(),
                rows: &scratch.bridge,
                bibite_count,
                pellet_end,
                settings,
                epoch,
            };
            self.mouth_candidates_valid = false;
            for (index, id) in bibite_ids.iter().enumerate() {
                if let Some(record) = self.get_mut(*id) {
                    if after_step(record, index, &after) {
                        lost_poses.push(index as u32);
                    }
                    if record.n_held_targets > 0 {
                        holders.push(index as u32);
                    }
                }
            }
        }
        // The bibites the step left with a pose that is not a number die here, in dense order (a death in the step like a collision's).
        for index in lost_poses {
            scratch.deaths.push(PendingDeath {
                id: bibite_ids[index as usize],
                cause: DeathCause::Hazard,
                swallowed: false,
            });
        }
        #[cfg(test)]
        if let Some(reference) = reference {
            for (slot, (got, want)) in self.records.iter().zip(&reference).enumerate() {
                assert_eq!(
                    got.as_ref().map(bridge_trace),
                    want.as_ref().map(bridge_trace),
                    "slot {slot}: the sweep after the step differs from the serial loops it replaced"
                );
            }
        }
        // Pellets. A pellet whose pose the step left as it was, and that nobody holds, rests from now on.
        for (index, id) in pellet_ids.iter().enumerate() {
            let body = &bodies[pellet_start + index];
            let mut pose = PelletPose {
                pos: body.pos,
                velocity: body.vel,
                rotation: body.rot,
                angular_velocity: body.ang_vel,
            };
            if !(pose.pos.is_finite()
                && pose.velocity.is_finite()
                && pose.rotation.is_finite()
                && pose.angular_velocity.is_finite())
            {
                // What the step left of this pellet is not a number: it stays where it was, at rest (a NaN pellet stays NaN for good).
                let before = pellets.pose(*id).unwrap_or_default();
                pose = PelletPose {
                    pos: before.pos,
                    velocity: Vec2::ZERO,
                    rotation: before.rotation,
                    angular_velocity: 0.0,
                };
            }
            if sleep_wanted {
                let unchanged = !pellets.is_held(*id)
                    && pellets.pose(*id).is_some_and(|before| {
                        before.pos.x.to_bits() == pose.pos.x.to_bits()
                            && before.pos.y.to_bits() == pose.pos.y.to_bits()
                            && before.velocity.x.to_bits() == pose.velocity.x.to_bits()
                            && before.velocity.y.to_bits() == pose.velocity.y.to_bits()
                            && before.rotation.to_bits() == pose.rotation.to_bits()
                            && before.angular_velocity.to_bits() == pose.angular_velocity.to_bits()
                    });
                let _ = pellets.set_pose_from_physics(*id, pose);
                sleep.settle(id.handle().slot() as usize, unchanged, body.aabb());
            } else {
                let _ = pellets.set_pose_from_physics(*id, pose);
            }
        }
        // Eggs.
        let egg_start = pellet_start + pellet_ids.len();
        for (index, id) in egg_ids.iter().enumerate() {
            let body = &bodies[egg_start + index];
            if let Some(egg) = eggs.get_mut(*id) {
                if body.pos.is_finite()
                    && body.vel.is_finite()
                    && body.rot.is_finite()
                    && body.ang_vel.is_finite()
                {
                    egg.pos = body.pos;
                    egg.rot = body.rot;
                    egg.vel = body.vel;
                    egg.ang_vel = body.ang_vel;
                } else {
                    // Not a number: the egg stays where it was, at rest.
                    egg.vel = Vec2::ZERO;
                    egg.ang_vel = 0.0;
                }
            }
        }
        // Solid contacts → two-sided collision damage.
        // `OnCollisionEnter2D` fires once when a collider pair starts
        // touching and the game reads `GetContact(0)` only, so damage is
        // applied for the first event of each pair that was not in contact on
        // the previous step.
        scratch.cur_collisions.clear();
        for contact in scratch.physics.contacts() {
            let key = if contact.tag_a <= contact.tag_b {
                (
                    contact.tag_a,
                    contact.tag_b,
                    contact.collider_a,
                    contact.collider_b,
                )
            } else {
                (
                    contact.tag_b,
                    contact.tag_a,
                    contact.collider_b,
                    contact.collider_a,
                )
            };
            if !scratch.cur_collisions.insert(key)
                || scratch.suppress_collision_damage
                || scratch.prev_collisions.contains(&key)
            {
                continue;
            }
            // `MatterPellet.OnCollisionEnter2D` with `pelletMerge`: of two pellets of one material that touch, the one that is not smaller takes the other's
            // amount and momentum (when both are as big the first callback wins: here the lower tag).
            if settings.scenario.pellet_merge
                && let (Some(a), Some(b)) = (pellet_id_from_tag(contact.tag_a), pellet_id_from_tag(contact.tag_b))
            {
                let (first, second) = if contact.tag_a <= contact.tag_b { (a, b) } else { (b, a) };
                if pellets.try_merge(first, second).is_none() {
                    let _ = pellets.try_merge(second, first);
                }
                continue;
            }
            let closing = -(contact.normal.dot(contact.relative_velocity));
            for victim_tag in [contact.tag_a, contact.tag_b] {
                let Some(victim_id) = bibite_id_from_tag(victim_tag) else {
                    continue;
                };
                let Some(record) = self.get_mut(victim_id) else {
                    continue;
                };
                let self_mass = record.derived.mass.max(0.000_1);
                let damage = collision_damage(closing, self_mass, &bs, CollisionSign::AsIs);
                // `Hurting` calls `Die` at once, so a second hit in the same step meets the corpse (full health), not the dying body.
                if damage > 0.0 && record.body.hurt(damage).died {
                    self.die_now(victim_id, DeathCause::Collision, false, pellets, rng, settings, events);
                }
            }
        }
        std::mem::swap(&mut scratch.prev_collisions, &mut scratch.cur_collisions);
        scratch.suppress_collision_damage = false;
        let deaths = std::mem::take(&mut scratch.deaths);
        for death in &deaths {
            self.die_now(death.id, death.cause, false, pellets, rng, settings, events);
        }
        scratch.deaths = deaths;
        scratch.bibite_ids = bibite_ids;
        scratch.pellet_ids = pellet_ids;
        scratch.egg_ids = egg_ids;
        scratch.sleep = sleep;
        scratch.physics_bodies = bodies;
        scratch.bibite_body_gens = body_gens;
        scratch.kept_bibite_bodies = if parallel { bibite_count } else { 0 };
        scratch.clock.lap(crate::sim::Stage::PhysBridge, &mut lap);
    }

    #[allow(clippy::too_many_arguments)] // body-store method mirrors the shared death path
    fn die_now(
        &mut self,
        id: BibiteId,
        cause: DeathCause,
        swallowed: bool,
        pellets: &mut PelletStore,
        rng: &mut crate::rng::Rng,
        settings: &SimSettings,
        events: &mut Vec<BibiteEvent>,
    ) {
        let (report, pos, radius, bursts) = {
            let Some(record) = self.get_mut(id) else {
                return;
            };
            if record.body.dead {
                return;
            }
            let extra = record.egg_layer.progress * record.egg_layer.egg_energy
                + record.fat.amount * settings.materials.fat.energy_density;
            // With corpses off the game bursts the body on `Die()` and once more for every later `Hurting` of the same frame.
            let bursts = record.body.death_hurts.max(1);
            let had_numbers = body_has_numbers(&record.body);
            let report = record.body.die(
                settings.scenario.corpses_enabled,
                record.born,
                swallowed,
                extra,
            );
            let report = report_for_numbers(report, had_numbers);
            record.let_go_on_death(pellets);
            (report, record.pos, record.body.geometry.body_length / 2.0, bursts)
        };
        if let Some(energy) = report.meat_energy {
            for _ in 0..bursts {
                let _ = pellets.spawn_meat_pile_with_radius(
                    pos,
                    energy,
                    radius,
                    rng,
                    settings.scenario.min_pellet_size,
                );
                if !swallowed && let Some(record) = self.get(id) {
                    spill_stomach(record, pellets, settings);
                }
            }
        }
        events.push(BibiteEvent::Death { bibite: id, cause });
        if report.swallowed_and_destroyed || report.unborn_no_meat || report.meat_energy.is_some() {
            self.remove(id);
        } else if let Some(record) = self.get_mut(id) {
            record.is_corpse = true;
            record.death_cause = None;
        }
    }
}

impl BibiteRecord {
    /// Ages retained mouth targets by one physics tick. Seen targets reset to
    /// age 0 in [`BibiteRecord::push_mouth_target`]; unseen targets older than
    /// [`MOUTH_CONTACT_GRACE_TICKS`] are pruned after the trigger pass.
    fn age_mouth_targets(&mut self) {
        for index in 0..self.n_mouth_targets as usize {
            self.mouth_target_ages[index] = self.mouth_target_ages[index].saturating_add(1);
        }
    }

    /// Drops mouth targets that missed more than
    /// [`MOUTH_CONTACT_GRACE_TICKS`] physics ticks and compacts the array.
    fn prune_mouth_targets(&mut self) {
        self.prune_mouth_targets_within(mouth_contact_grace_ticks());
    }

    /// [`Self::prune_mouth_targets`] with the grace given.
    fn prune_mouth_targets_within(&mut self, grace: u8) {
        let count = self.n_mouth_targets as usize;
        let mut write = 0;
        for read in 0..count {
            if self.mouth_target_ages[read] <= grace {
                self.mouth_targets[write] = self.mouth_targets[read];
                self.mouth_target_ages[write] = self.mouth_target_ages[read];
                write += 1;
            }
        }
        for index in write..count {
            self.mouth_targets[index] = 0;
            self.mouth_target_ages[index] = 0;
        }
        self.n_mouth_targets = write as u8;
        self.mouth.n_in_mouth = self.n_mouth_targets as usize;
    }

    fn push_mouth_target(&mut self, tag: u64) {
        let count = self.n_mouth_targets as usize;
        if count >= self.mouth_targets.len() {
            return;
        }
        if let Some(index) = self.mouth_targets[..count]
            .iter()
            .position(|target| *target == tag)
        {
            self.mouth_target_ages[index] = 0;
        } else {
            self.mouth_targets[count] = tag;
            self.mouth_target_ages[count] = 0;
            self.n_mouth_targets += 1;
            self.mouth.n_in_mouth = self.n_mouth_targets as usize;
        }
    }
}

#[cfg(test)]
fn mouth_tip(record: &BibiteRecord) -> Vec2 {
    let d1 = record.body.geometry.d1_size;
    let offset = phys::prefab::mouth_offset(d1);
    record.pos + offset.rotate(record.rot)
}

#[cfg(test)]
mod record_layout_tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// Prints where each field of a bibite record lies and how big it is, by cache line (run with `--ignored --nocapture`).
    /// The passes over the records are limited by the memory they touch, so this is the map for moving cold parts out of line
    /// and for keeping the fields one pass reads on the same lines.
    #[test]
    #[ignore = "prints the layout of a record"]
    fn print_the_layout_of_a_bibite_record() {
        fn size_of_field<T>(_: fn(&BibiteRecord) -> &T) -> usize {
            size_of::<T>()
        }
        macro_rules! field {
            ($name:ident) => {
                (
                    stringify!($name),
                    offset_of!(BibiteRecord, $name),
                    size_of_field(|record: &BibiteRecord| &record.$name),
                )
            };
        }
        let mut fields = vec![
            field!(id),
            field!(pos),
            field!(vel),
            field!(rot),
            field!(ang_vel),
            field!(genome),
            field!(meta),
            field!(brain),
            field!(body),
            field!(growth),
            field!(growth_genes),
            field!(stomach),
            field!(fat),
            field!(armor),
            field!(egg_layer),
            field!(mouth),
            field!(feeding),
            field!(clock),
            field!(pheromone),
            field!(propulsion),
            field!(mouth_derived),
            field!(propulsion_derived),
            field!(derived),
            field!(colliders),
            field!(senses),
            field!(throttle),
            field!(steering),
            field!(fov_seen),
            field!(last_contact_count),
            field!(last_contact_impulse),
            field!(last_contact_bibite),
            field!(last_contact_pellet),
            field!(last_contact_egg),
            field!(last_contact_collider),
            field!(last_contact_enter_count),
            field!(last_contact_enter_impulse),
            field!(prev_contact_tags),
            field!(n_prev_contact_tags),
            field!(mutation_factor),
            field!(rad_zone),
            field!(color),
            field!(eye_offset),
            field!(is_ready),
            field!(born),
            field!(is_corpse),
            field!(death_cause),
            field!(children),
            field!(pending_force),
            field!(pending_torque),
            field!(mouth_targets),
            field!(n_mouth_targets),
            field!(mouth_target_ages),
            field!(held_targets),
            field!(n_held_targets),
        ];
        fields.sort_by_key(|field| field.1);
        println!(
            "BibiteRecord: {} bytes, {} cache lines",
            size_of::<BibiteRecord>(),
            size_of::<BibiteRecord>().div_ceil(64)
        );
        for (name, offset, size) in fields {
            println!(
                "  line {:>2}  offset {:>4}  size {:>4}  {}",
                offset / 64,
                offset,
                size,
                name
            );
        }
    }
}

#[cfg(test)]
mod b2b_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::rng::Rng;
    use crate::settings::SimSettings;

    fn fixture() -> (BibiteStore, SimSettings, Genome, Brain) {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        (
            BibiteStore::with_settings(settings.clone()),
            settings,
            genome,
            brain,
        )
    }

    fn spawn_adult(store: &mut BibiteStore, genome: &Genome, brain: &Brain) -> BibiteId {
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.vitals = SpawnVitals::Adult;
        store.spawn(spawn).unwrap()
    }

    struct Services {
        pellets: PelletStore,
        pheromones: PheromoneStore,
        eggs: EggStore,
        zones: [ZoneCircle; 0],
        rng: Rng,
        scratch: BibiteScratch,
        events: Vec<BibiteEvent>,
    }

    impl Services {
        fn new() -> Self {
            Self {
                pellets: PelletStore::default(),
                pheromones: PheromoneStore::default(),
                eggs: EggStore::new(),
                zones: [],
                rng: Rng::from_seed(42),
                scratch: BibiteScratch::new(),
                events: Vec::new(),
            }
        }

        fn ctx<'a>(&'a mut self, settings: &'a SimSettings) -> BibiteTickCtx<'a> {
            BibiteTickCtx {
                pellets: &mut self.pellets,
                pheromones: &mut self.pheromones,
                eggs: &mut self.eggs,
                zones: &self.zones,
                settings,
                rng: &mut self.rng,
                scratch: &mut self.scratch,
                events: &mut self.events,
            }
        }
    }

    #[test]
    fn tick_spends_energy_and_advances_clock() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let before = store.get(id).unwrap().body.energy.amount;
        let mut services = Services::new();
        {
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
        }
        let record = store.get(id).unwrap();
        assert!(record.body.energy.amount < before);
        assert!(record.clock.time_alive > 0.0);
        assert_eq!(record.throttle.brain_counter, 1);
        assert!(record.pos.is_finite());
    }

    #[test]
    fn brain_cadence_and_senses_are_wired() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        store.set_senses(id, SenseOutputs::from_values([0.5; 21]));
        let mut services = Services::new();
        for _ in 0..2 {
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
        }
        assert_eq!(store.sense_throttle(id).unwrap().brain_counter, 0);
        assert_eq!(store.senses(id).unwrap().values(), [0.5; 21]);
        let record = store.get(id).unwrap();
        let mut inputs = BrainInputs::new();
        inputs.write_internal(record);
        assert!(inputs.get(crate::brain::Input::EnergyRatio) <= 1.0);
    }

    #[test]
    fn swallow_pellet_moves_matter_to_stomach() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let mut services = Services::new();
        let pellet = services.pellets.spawn(crate::pellet::PelletSpawn::new(
            MaterialKind::Plant,
            5.0,
            Vec2::new(1.0, 0.0),
        ));
        let record = store.get_mut(id).unwrap();
        record.mouth.can_swallow = true;
        record.mouth.desire_eat = 1.0;
        record.mouth.desire_attack = 0.0;
        record.mouth.desire_grab = 0.0;
        record.mouth.mouth_opening = 10.0;
        record.mouth.bite_progress = record.mouth_derived.bite_period;
        let bite_period = record.mouth_derived.bite_period;
        let snapshot = TargetSnapshot {
            tag: pellet_tag(pellet),
            is_pellet: true,
            radius: 1.0,
            amount: 5.0,
            material: 0,
            ..TargetSnapshot::default()
        };
        {
            let mut ctx = services.ctx(&settings);
            tick_mouth_pellet(record, &mut ctx, snapshot, bite_period);
        }
        assert!(services.pellets.get(pellet).is_none());
        assert!((record.stomach.total_amount() - 5.0).abs() < 1e-4);
        assert_eq!(record.feeding.plant_swallow_events, 1);
        assert_eq!(record.feeding.plant_bite_events, 0);
        assert!((record.feeding.plant_consumed_u2 - 5.0).abs() < 1e-6);
        assert!(record.feeding.live_seconds >= 0.0);
    }

    #[test]
    fn populated_body_arena_codec_round_trip() {
        use crate::settings::ZoneGroupSettings;
        use crate::world::{ScenarioData, World};
        let settings = SimSettings::game_defaults();
        let scenario = ScenarioData {
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            ..ScenarioData::default()
        };
        let mut world = World::new(settings, scenario, 74).expect("world");
        world.seed_scenario_world();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut rng = Rng::from_seed(99);
        for _ in 0..4 {
            let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(rng.f32_range(-20.0, 20.0), rng.f32_range(-20.0, 20.0));
            world.bibites.spawn(spawn).expect("spawn");
        }
        world.run(120);
        let arena = world.bibites.capture_checkpoint();
        let bytes = arena.encode();
        let decoded = BibiteCheckpointArena::decode(&bytes).expect("decode populated body arena");
        assert_eq!(decoded, arena);
    }

    #[test]
    #[ignore = "heavy 1k/2k checkpoint regression; run manually"]
    fn large_store_checkpoint_restore_step_bit_equal() {
        for n in [1000usize, 2000] {
            let settings = SimSettings::game_defaults();
            let genome = Genome::starter();
            let brain = Brain::starter();
            let mut rng = Rng::from_seed(0x5EED_0000 + n as u64);
            let mut store = BibiteStore::with_settings(settings.clone());
            for _ in 0..n {
                let mut spawn =
                    BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
                spawn.vitals = SpawnVitals::Adult;
                spawn.pos = Vec2::new(rng.f32_range(-900.0, 900.0), rng.f32_range(-900.0, 900.0));
                spawn.rot = rng.f32_range(0.0, std::f32::consts::TAU);
                let id = store.spawn(spawn).unwrap();
                store.get_mut(id).unwrap().stomach.restore_contents(&[(
                    StomachMaterial::Plant,
                    2.0,
                    0.5 + rng.f32_range(0.0, 0.5),
                )]);
            }
            let checkpoint = store.capture_checkpoint();
            let mut restored = BibiteStore::with_settings(settings.clone());
            restored
                .restore_checkpoint(BibiteCheckpointArena::decode(&checkpoint.encode()).unwrap())
                .unwrap();
            assert_eq!(
                restored.capture_checkpoint(),
                checkpoint,
                "restore at n={n}"
            );
            let mut services_source = Services::new();
            let mut events_source = Vec::new();
            store.step_physics(
                &mut services_source.pellets,
                &mut services_source.eggs,
                &settings,
                &mut services_source.rng,
                &mut services_source.scratch,
                &mut events_source,
            );
            let mut services_restored = Services::new();
            let mut events_restored = Vec::new();
            restored.step_physics(
                &mut services_restored.pellets,
                &mut services_restored.eggs,
                &settings,
                &mut services_restored.rng,
                &mut services_restored.scratch,
                &mut events_restored,
            );
            assert_eq!(
                restored.capture_checkpoint(),
                store.capture_checkpoint(),
                "post-restore step at n={n}"
            );
        }
    }

    #[test]
    fn stomach_average_chunk_amount_round_trips() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        store.get_mut(id).unwrap().stomach.restore_contents(&[
            (StomachMaterial::Meat, 1.0, 0.5),
            (StomachMaterial::Plant, 2.0, 0.75),
        ]);
        store.get_mut(id).unwrap().stomach.total_content = 12.119_801;
        let checkpoint = store.capture_checkpoint();
        let mut restored = BibiteStore::with_settings(settings);
        restored
            .restore_checkpoint(BibiteCheckpointArena::decode(&checkpoint.encode()).unwrap())
            .unwrap();
        let stomach = &restored.get(id).unwrap().stomach;
        assert_eq!(stomach.total_content, 12.119_801);
        let slots: Vec<_> = stomach
            .contents()
            .map(|content| {
                (
                    content.material,
                    content.amount,
                    content.average_chunk_amount,
                )
            })
            .collect();
        assert_eq!(
            slots,
            vec![
                (StomachMaterial::Meat, 1.0, 0.5),
                (StomachMaterial::Plant, 2.0, 0.75),
            ]
        );
    }

    #[test]
    fn checkpoint_restore_then_physics_step_matches_source() {
        let (mut store, settings, genome, brain) = fixture();
        let id_a = spawn_adult(&mut store, &genome, &brain);
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::new(5.0, 2.0);
        let id_b = store.spawn(spawn).unwrap();
        // Populate live derived state, pending movement force and organ caches.
        let mut services = Services::new();
        for _ in 0..40 {
            {
                let record = store.get_mut(id_a).unwrap();
                record.pending_force = Vec2::new(1.25, -0.75);
                record.pending_torque = 0.125;
            }
            let mut events = Vec::new();
            store.step_physics(
                &mut services.pellets,
                &mut services.eggs,
                &settings,
                &mut services.rng,
                &mut services.scratch,
                &mut events,
            );
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
        }
        // A radian value that is not exactly representable through the game's
        // degree-serialized `BibiteSave.rot`; the checkpoint must keep raw bits.
        store.get_mut(id_a).unwrap().rot = f32::from_bits(0x3ff6_b7dd);
        let checkpoint = store.capture_checkpoint();
        let decoded = BibiteCheckpointArena::decode(&checkpoint.encode()).expect("decode");
        let mut restored = BibiteStore::with_settings(settings.clone());
        restored.restore_checkpoint(decoded).expect("restore");
        assert_eq!(restored.capture_checkpoint(), checkpoint);
        assert_eq!(
            restored.get(id_a).unwrap().rot.to_bits(),
            store.get(id_a).unwrap().rot.to_bits(),
            "raw radians not restored bit-exactly"
        );

        let mut services_source = Services::new();
        let mut events_source = Vec::new();
        store.step_physics(
            &mut services_source.pellets,
            &mut services_source.eggs,
            &settings,
            &mut services_source.rng,
            &mut services_source.scratch,
            &mut events_source,
        );
        let mut services_restored = Services::new();
        let mut events_restored = Vec::new();
        restored.step_physics(
            &mut services_restored.pellets,
            &mut services_restored.eggs,
            &settings,
            &mut services_restored.rng,
            &mut services_restored.scratch,
            &mut events_restored,
        );
        let source_after = store.capture_checkpoint();
        let restored_after = restored.capture_checkpoint();
        assert_eq!(
            restored_after, source_after,
            "restored body store diverged after one physics step"
        );
        store.step_physics(
            &mut services_source.pellets,
            &mut services_source.eggs,
            &settings,
            &mut services_source.rng,
            &mut services_source.scratch,
            &mut events_source,
        );
        restored.step_physics(
            &mut services_restored.pellets,
            &mut services_restored.eggs,
            &settings,
            &mut services_restored.rng,
            &mut services_restored.scratch,
            &mut events_restored,
        );
        assert_eq!(
            restored.capture_checkpoint(),
            store.capture_checkpoint(),
            "restored body store diverged after two physics steps"
        );
        let _ = (id_a, id_b);
    }

    #[test]
    fn mouth_contact_grace_survives_one_missed_tick_then_drops() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let d1 = store.get(id).unwrap().derived.d1_size;
        let mouth = crate::physics::prefab::mouth_offset(d1);
        let mut services = Services::new();
        let pellet = services.pellets.spawn(crate::pellet::PelletSpawn::new(
            MaterialKind::Plant,
            20.0,
            Vec2::new(mouth.x, mouth.y),
        ));
        let mut events = Vec::new();
        let mut step_at = |store: &mut BibiteStore, services: &mut Services, pos: Vec2| {
            {
                let record = store.get_mut(id).unwrap();
                let tip = record.pos + (pos - mouth).rotate(record.rot);
                let _ = services.pellets.set_pose(
                    pellet,
                    crate::pellet::PelletPose {
                        pos: tip,
                        velocity: Vec2::ZERO,
                        rotation: 0.0,
                        angular_velocity: 0.0,
                    },
                );
            }
            store.step_physics(
                &mut services.pellets,
                &mut services.eggs,
                &settings,
                &mut services.rng,
                &mut services.scratch,
                &mut events,
            );
        };
        // Fresh overlap: target present at age 0.
        let record = store.get(id).unwrap();
        let tip = record.pos + mouth.rotate(record.rot);
        step_at(&mut store, &mut services, tip);
        assert_eq!(store.get(id).unwrap().n_mouth_targets, 1);
        // The game at 1x runs one physics step per tick and Unity's exit event comes with the step that ends the overlap: no grace by default
        // (`RUSTIBITES_GRACE=<ticks>` keeps a missed target that long).
        step_at(&mut store, &mut services, tip + Vec2::new(1000.0, 0.0));
        let record = store.get(id).unwrap();
        let grace = u32::from(mouth_contact_grace_ticks());
        assert_eq!(u32::from(record.n_mouth_targets), u32::from(grace > 0));
        // The pruning with a grace keeps a target for that many missed ticks and then drops it.
        let mut record = store.get(id).unwrap().clone();
        record.mouth_targets[0] = 7;
        record.n_mouth_targets = 1;
        for miss in 1..=8u8 {
            record.mouth_target_ages[0] = miss;
            record.n_mouth_targets = 1;
            record.prune_mouth_targets_within(8);
            assert_eq!(record.n_mouth_targets, 1, "missed tick {miss}");
        }
        record.mouth_target_ages[0] = 9;
        record.n_mouth_targets = 1;
        record.prune_mouth_targets_within(8);
        assert_eq!(record.n_mouth_targets, 0);
        assert!(record.mouth_targets.iter().all(|tag| *tag == 0));
    }

    #[test]
    fn mouth_contact_persists_while_pellet_overlaps() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let d1 = store.get(id).unwrap().derived.d1_size;
        let mouth = crate::physics::prefab::mouth_offset(d1);
        let mut services = Services::new();
        let pellet = services.pellets.spawn(crate::pellet::PelletSpawn::new(
            MaterialKind::Plant,
            20.0,
            Vec2::new(mouth.x, mouth.y),
        ));
        let mut events = Vec::new();
        for tick in 0..20 {
            {
                let record = store.get_mut(id).unwrap();
                let tip = record.pos + mouth.rotate(record.rot);
                let _ = services.pellets.set_pose(
                    pellet,
                    crate::pellet::PelletPose {
                        pos: tip,
                        velocity: Vec2::ZERO,
                        rotation: 0.0,
                        angular_velocity: 0.0,
                    },
                );
            }
            store.step_physics(
                &mut services.pellets,
                &mut services.eggs,
                &settings,
                &mut services.rng,
                &mut services.scratch,
                &mut events,
            );
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
            let record = store.get(id).unwrap();
            assert_eq!(
                record.n_mouth_targets, 1,
                "target dropped at tick {tick} despite overlap"
            );
            assert!(record.feeding.mouth_contact_ticks > 0);
        }
        assert_eq!(store.get(id).unwrap().feeding.mouth_contact_enter_events, 1);
    }

    #[test]
    fn checkpoint_round_trip_preserves_body_arena_and_runtime() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        {
            let record = store.get_mut(id).unwrap();
            record.throttle.brain_counter = 7;
            record.senses.bibite_closeness = 0.5;
            record.senses.has_herd = true;
            record.senses.herd_sum_direction = -22.28796;
            record.senses.herd_separation_projection = -0.99347574;
            record.steering.has_herd = true;
            record.fov_seen = FovSeenCache {
                pellets: vec![PelletId::from_handle(Handle::new(3, 2))],
                bibites: vec![BibiteId::from_handle(Handle::new(7, 1))],
            };
            record.pending_force = Vec2::new(1.5, -2.0);
            record.pending_torque = 0.25;
            record.mouth_targets[0] = 0x1234;
            record.n_mouth_targets = 1;
            record.held_targets[0] = 0x5678;
            record.n_held_targets = 1;
            record.death_cause = Some(DeathCause::Bitten);
            record.is_ready = false;
            record.born = true;
            record.mutation_factor.probability_increase = 0.125;
            record.brain.set_innovations(77, 66);
        }
        let checkpoint = store.capture_checkpoint();
        let mut restored = BibiteStore::with_settings(settings);
        restored
            .restore_checkpoint(checkpoint.clone())
            .expect("restore");
        assert_eq!(restored.len(), store.len());
        assert_eq!(restored.capture_checkpoint(), checkpoint);
        let record = restored.get(restored.ids().next().unwrap()).unwrap();
        assert_eq!(record.throttle.brain_counter, 7);
        assert_eq!(record.senses.bibite_closeness, 0.5);
        assert!(record.senses.has_herd);
        assert_eq!(record.senses.herd_sum_direction, -22.28796);
        assert_eq!(record.senses.herd_separation_projection, -0.99347574);
        assert!(record.steering.has_herd);
        assert_eq!(
            record.fov_seen.pellets,
            vec![PelletId::from_handle(Handle::new(3, 2))]
        );
        assert_eq!(
            record.fov_seen.bibites,
            vec![BibiteId::from_handle(Handle::new(7, 1))]
        );
        assert_eq!(record.pending_force, Vec2::new(1.5, -2.0));
        assert_eq!(record.pending_torque, 0.25);
        assert_eq!(record.mouth_targets[0], 0x1234);
        assert_eq!(record.n_mouth_targets, 1);
        assert_eq!(record.held_targets[0], 0x5678);
        assert_eq!(record.n_held_targets, 1);
        assert_eq!(record.death_cause, Some(DeathCause::Bitten));
        assert!(!record.is_ready);
        assert_eq!(record.brain.node_max_innovation(), 77);
        assert_eq!(record.brain.connection_max_innovation(), 66);
    }

    #[test]
    fn checkpoint_round_trip_preserves_egg_arena_and_runtime() {
        let (store, settings, genome, brain) = fixture();
        let _ = store;
        let mut eggs = EggStore::new();
        let mut spec = EggSpawn {
            pos: Vec2::new(2.0, -3.0),
            vel: Vec2::new(0.25, 0.5),
            rot: 0.75,
            ang_vel: 0.125,
            genome,
            meta: GenomeMeta::default(),
            brain,
            energy: 40.0,
            hatch_time: 10.0,
            total_required_energy: 5.0,
            birth_orientation: -Vec2::Y,
        };
        spec.energy = 40.0;
        let id = eggs.spawn(spec);
        {
            let egg = eggs.get_mut(id).unwrap();
            egg.hatch.is_hatching = false;
            egg.hatch.aborting = true;
            egg.brain.set_herding_enabled(false);
            egg.birth_orientation = Vec2::new(0.5, 0.5);
        }
        let _ = settings;
        eggs.get_mut(id).unwrap().rot = f32::from_bits(0x3ff6_b7dd);
        let checkpoint = eggs.capture_checkpoint();
        let mut restored = EggStore::new();
        restored
            .restore_checkpoint(checkpoint.clone())
            .expect("restore");
        assert_eq!(restored.len(), eggs.len());
        assert_eq!(restored.capture_checkpoint(), checkpoint);
        assert_eq!(
            restored.get(id).unwrap().rot.to_bits(),
            eggs.get(id).unwrap().rot.to_bits(),
            "egg raw radians not restored bit-exactly"
        );
        let egg = restored.get(restored.ids().next().unwrap()).unwrap();
        assert!(!egg.hatch.is_hatching);
        assert!(egg.hatch.aborting);
        assert_eq!(egg.birth_orientation, Vec2::new(0.5, 0.5));
        assert!(!egg.brain.herding_enabled(), "egg brain herding flag lost");
    }

    #[test]
    fn feeding_counters_zero_without_contact() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let mut services = Services::new();
        for _ in 0..30 {
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
        }
        let record = store.get(id).unwrap();
        assert_eq!(record.feeding.plant_bite_events, 0);
        assert_eq!(record.feeding.meat_bite_events, 0);
        assert_eq!(record.feeding.bibite_bite_events, 0);
        assert_eq!(record.feeding.plant_consumed_u2, 0.0);
        assert_eq!(record.feeding.meat_consumed_u2, 0.0);
        assert!(record.feeding.live_seconds > 0.0);
    }

    #[test]
    fn feeding_counters_track_digestion_and_energy() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let plant = organ_material(&settings.materials.plant);
        let meat = organ_material(&settings.materials.meat);
        {
            let record = store.get_mut(id).unwrap();
            record.body.energy.amount = 0.0;
            record
                .stomach
                .add_matter(StomachMaterial::Plant, 20.0, &plant, &meat);
        }
        let mut services = Services::new();
        for _ in 0..120 {
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
        }
        let record = store.get(id).unwrap();
        assert!(record.feeding.stomach_digested_u2 > 0.0);
        assert!(record.feeding.stomach_energy_gain_e > 0.0);
        assert!(record.feeding.live_seconds > 0.0);
        assert!(record.feeding.energy_gained_e > 0.0);
        assert!(record.feeding.energy_spent_e > 0.0);
    }

    #[test]
    fn death_without_corpses_spawns_meat_and_event() {
        let (mut store, mut settings, genome, brain) = fixture();
        settings.scenario.corpses_enabled = false;
        store.set_settings(settings.clone());
        let id = spawn_adult(&mut store, &genome, &brain);
        let mut services = Services::new();
        {
            let mut ctx = services.ctx(&settings);
            store.resolve_death(
                PendingDeath {
                    id,
                    cause: DeathCause::Starvation,
                    swallowed: false,
                },
                &mut ctx,
            );
        }
        assert_eq!(services.pellets.len(), 1);
        assert_eq!(store.len(), 0);
        assert!(
            services
                .events
                .iter()
                .any(|event| matches!(event, BibiteEvent::Death { .. }))
        );
    }

    /// With corpses off the game's `Die()` bursts the body on the spot, and the body lives on until the end of the frame, so every later
    /// `UseEnergy` of that frame (fat, muscles, healing) meets a dead body below zero health and bursts it again: a starving bibite
    /// leaves four meat piles of the same energy (measured against the game, 2026-11-11). A death that was not a hurt leaves one.
    #[test]
    fn a_starving_body_without_corpses_bursts_once_per_energy_spend_of_the_frame() {
        let (mut store, mut settings, genome, brain) = fixture();
        settings.scenario.corpses_enabled = false;
        store.set_settings(settings.clone());
        let id = spawn_adult(&mut store, &genome, &brain);
        {
            let record = store.get_mut(id).unwrap();
            record.body.energy.amount = 0.0;
            record.body.health.amount = 0.001;
        }
        let mut services = Services::new();
        for _ in 0..200 {
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
            if store.len() == 0 {
                break;
            }
        }
        assert_eq!(store.len(), 0, "the bibite starved");
        let amounts: Vec<f32> = services
            .pellets
            .ids()
            .filter_map(|pellet| services.pellets.get(pellet).map(|view| view.amount))
            .collect();
        // Metabolism (which kills), fat, muscles and healing always spend; an adult that makes eggs spends a fifth time.
        assert!((4..=5).contains(&amounts.len()), "each energy spend of the frame bursts the dead body again: {amounts:?}");
        assert!(amounts.iter().all(|amount| (*amount - amounts[0]).abs() < 1e-3), "all four carry the body's energy: {amounts:?}");
    }

    /// `GUIManager.LayEggIfBibite` -> `LayEgg()`: one egg regardless of egg
    /// progress, progress untouched, `nEggsLaid` incremented.
    #[test]
    fn lay_single_egg_ignores_progress_and_counts_the_egg() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let progress_before = store.get(id).unwrap().egg_layer.progress;
        let laid_before = store.get(id).unwrap().egg_layer.n_laid;
        let mut services = Services::new();
        let laid = {
            let mut ctx = services.ctx(&settings);
            store.lay_single_egg(id, &mut ctx)
        };
        assert_eq!(laid, 1);
        assert_eq!(services.eggs.ids().count(), 1);
        let record = store.get(id).unwrap();
        assert_eq!(record.egg_layer.progress, progress_before);
        assert_eq!(record.egg_layer.n_laid, laid_before + 1);
    }

    #[test]
    fn egg_laying_sets_parents_and_generation() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let parent_generation = 5;
        {
            let record = store.get_mut(id).unwrap();
            record.meta.generation = parent_generation;
            record.meta.species = Some(crate::ids::SpeciesId::new(42));
            record.meta.species_tag = Some("lineage".to_string());
            record.meta.is_mutant = true;
        }
        let record = store.get_mut(id).unwrap();
        let mut services = Services::new();
        let laid = {
            let mut ctx = services.ctx(&settings);
            lay_eggs(record, &mut ctx, 1)
        };
        assert_eq!(laid, 1);
        let egg_id = services.eggs.ids().next().unwrap();
        let egg = services.eggs.get(egg_id).unwrap();
        assert_eq!(egg.meta.parent1, Some(id));
        assert_eq!(egg.meta.parent2, None);
        assert_eq!(egg.meta.generation, parent_generation + 1);
        assert_eq!(egg.meta.species, Some(crate::ids::SpeciesId::new(42)));
        assert_eq!(egg.meta.species_tag.as_deref(), Some("lineage"));
        assert!(egg.meta.is_mutant);
        assert!(egg.hatch.energy > 0.0);
        // Hatch time is the child's (mutated) gene, not the parent's.
        assert_eq!(egg.hatch.hatch_time, egg.genome.gene(Gene::HatchTime));
        let mut hatched = Vec::new();
        services.eggs.tick(1000.0, &settings, &mut hatched);
        assert_eq!(hatched, vec![egg_id]);
    }

    /// The egg's brain is the parent's structure with fresh nodes (`CopyBrain` + `FinalizeEditing`), not a copy of the parent's running node values.
    #[test]
    fn an_egg_gets_a_brain_with_fresh_nodes() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let record = store.get_mut(id).unwrap();
        record.brain.set_output_value(Output::Accelerate, 1.0);
        record.brain.set_output_value(Output::PhereOut1, 7.0);
        let mut services = Services::new();
        let laid = {
            let mut ctx = services.ctx(&settings);
            lay_eggs(record, &mut ctx, 1)
        };
        assert_eq!(laid, 1);
        let egg_id = services.eggs.ids().next().unwrap();
        let egg = services.eggs.get(egg_id).unwrap();
        // (the egg's brain may have mutated: each node holds what a fresh node with ITS bias holds)
        let accelerate = egg.brain.output_value(Output::Accelerate);
        assert!((accelerate - egg.brain.output_bias(Output::Accelerate).tanh()).abs() < 1e-5, "the parent's 1.0 is not inherited: {accelerate}");
        assert_eq!(egg.brain.output_value(Output::PhereOut1), egg.brain.output_bias(Output::PhereOut1).max(0.0));
    }

    #[test]
    fn brain_maturity_input_is_raw_growth() {
        let (mut store, _settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain);
        let record = store.get_mut(id).unwrap();
        record.growth.growth = 2.5;
        record.growth_genes.growth_at_mature = 1.5;
        assert_eq!(InternalSenseSource::maturity(record), 2.5);
    }

    #[test]
    fn organs_read_live_brain_output_values_on_first_tick() {
        let (mut store, settings, genome, mut brain) = fixture();
        brain.set_output_value(Output::Accelerate, 1.0);
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.vitals = SpawnVitals::Adult;
        let id = store.spawn(spawn).unwrap();
        let mut services = Services::new();
        {
            let mut ctx = services.ctx(&settings);
            store.tick(&mut ctx);
        }
        let pending = store.get(id).unwrap().pending_force;
        assert!(
            pending.length() > 0.0,
            "first tick did not read the brain's live Accelerate value"
        );
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(3);
        let mut scratch = BibiteScratch::new();
        let mut events = Vec::new();
        store.step_physics(
            &mut pellets,
            &mut eggs,
            &settings,
            &mut rng,
            &mut scratch,
            &mut events,
        );
        assert!(store.get(id).unwrap().vel.length() > 0.0);
    }

    #[test]
    fn loaded_bibite_uses_saved_brain_node_values_on_first_tick() {
        let (mut store, settings, genome, mut brain) = fixture();
        brain.set_output_value(Output::Accelerate, 1.0);
        let mut spawn = BibiteSpawn::new(genome, GenomeMeta::default(), brain);
        spawn.vitals = SpawnVitals::Adult;
        let id = store.spawn(spawn).unwrap();
        let save = store.to_save(id).unwrap();
        let mut loaded = BibiteStore::with_settings(settings.clone());
        let loaded_id = loaded.spawn_loaded(save).unwrap();
        let mut services = Services::new();
        {
            let mut ctx = services.ctx(&settings);
            loaded.tick(&mut ctx);
        }
        assert!(
            loaded.get(loaded_id).unwrap().pending_force.length() > 0.0,
            "loaded bibite ignored saved brain node values on its first tick"
        );
    }
}

#[cfg(test)]
mod b2b2_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::rng::Rng;
    use crate::settings::SimSettings;

    fn fixture() -> (BibiteStore, SimSettings, Genome, Brain) {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        (
            BibiteStore::with_settings(settings.clone()),
            settings,
            genome,
            brain,
        )
    }

    fn spawn_adult(store: &mut BibiteStore, genome: &Genome, brain: &Brain, pos: Vec2) -> BibiteId {
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = pos;
        store.spawn(spawn).unwrap()
    }

    struct Services {
        pellets: PelletStore,
        pheromones: PheromoneStore,
        eggs: EggStore,
        zones: Vec<ZoneCircle>,
        rng: Rng,
        scratch: BibiteScratch,
        events: Vec<BibiteEvent>,
    }

    impl Services {
        fn new() -> Self {
            Self {
                pellets: PelletStore::default(),
                pheromones: PheromoneStore::default(),
                eggs: EggStore::new(),
                zones: Vec::new(),
                rng: Rng::from_seed(7),
                scratch: BibiteScratch::new(),
                events: Vec::new(),
            }
        }

        fn ctx<'a>(&'a mut self, settings: &'a SimSettings) -> BibiteTickCtx<'a> {
            BibiteTickCtx {
                pellets: &mut self.pellets,
                pheromones: &mut self.pheromones,
                eggs: &mut self.eggs,
                zones: &self.zones,
                settings,
                rng: &mut self.rng,
                scratch: &mut self.scratch,
                events: &mut self.events,
            }
        }
    }

    #[test]
    fn steering_inputs_round_trip() {
        let herd = crate::senses::HerdState {
            has_herd: true,
            sum_direction: -32.0,
            separation_projection: 0.5,
            ..crate::senses::HerdState::default()
        };
        let steering = SteeringInputs::from_fov(&herd);
        assert!(steering.has_herd);
        assert!((steering.herd_sum_direction_degrees + 32.0).abs() < 1e-6);
        let (mut store, _settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        store.set_steering(id, steering);
        assert_eq!(store.steering(id), Some(steering));
    }

    #[test]
    fn void_avoidance_blends_turn_back() {
        let (mut store, mut settings, genome, brain) = fixture();
        settings.scenario.void_avoidance = true;
        settings.scenario.shade_avoidance = false;
        let id = spawn_adult(&mut store, &genome, &brain, Vec2::new(-40.0, 0.0));
        let mut services = Services::new();
        services.zones.push(ZoneCircle {
            id: ZoneId::from_handle(Handle::new(0, 0)),
            pos: Vec2::ZERO,
            radius: 10.0,
            is_rect: false,
            half_extents: Vec2::ZERO,
        });
        let record = store.get_mut(id).unwrap();
        let command = organs::MovementCommand {
            force: Vec2::Y,
            torque: 0.0,
            movement_cost: 0.0,
        };
        let ctx = services.ctx(&settings);
        let steered =
            apply_environment_steering(record, ctx.settings, ctx.zones, command, Vec2::Y, 1.0);
        assert!(steered.torque < 0.0);
    }

    /// `BibitePropulsion` steers for the centre of the ONE zone whose edge is nearest, and only when the bibite is outside that one: a bibite inside a zone is
    /// not pulled towards another zone it is outside of (the old code took the zone it was farthest outside of).
    #[test]
    fn void_avoidance_follows_the_nearest_zone_only() {
        let (store, mut settings, genome, brain) = fixture();
        settings.scenario.void_avoidance = true;
        settings.scenario.shade_avoidance = false;
        let zone = |index: u32, x: f32, radius: f32| ZoneCircle {
            id: ZoneId::from_handle(Handle::new(index, 0)),
            pos: Vec2::new(x, 0.0),
            radius,
            is_rect: false,
            half_extents: Vec2::ZERO,
        };
        let command = organs::MovementCommand { force: Vec2::Y, torque: 0.0, movement_cost: 0.0 };
        let steer = |position: Vec2, zones: &[ZoneCircle]| {
            let mut store = store.clone();
            let mut settings = settings.clone();
            settings.scenario.void_avoidance = true;
            let id = spawn_adult(&mut store, &genome, &brain, position);
            let record = store.get_mut(id).unwrap();
            apply_environment_steering(record, &settings, zones, command, Vec2::Y, 1.0).torque
        };
        // Inside the first zone (centre 0, radius 100) and far outside the second (centre 5000): no steering.
        assert_eq!(steer(Vec2::new(50.0, 0.0), &[zone(0, 0.0, 100.0), zone(1, 5000.0, 100.0)]), 0.0);
        // The same bibite with the zones the other way round: the last zone it is inside wins, still no steering.
        assert_eq!(steer(Vec2::new(50.0, 0.0), &[zone(1, 5000.0, 100.0), zone(0, 0.0, 100.0)]), 0.0);
        // Outside both: it heads for the nearer one (to its left, the bibite faces +y: a left turn is positive torque... the zone at -x is on its left).
        let near_left = steer(Vec2::new(-300.0, 0.0), &[zone(0, -500.0, 100.0), zone(1, 5000.0, 100.0)]);
        let near_right = steer(Vec2::new(300.0, 0.0), &[zone(0, 500.0, 100.0), zone(1, -5000.0, 100.0)]);
        assert!(near_left > 0.0 && near_right < 0.0, "{near_left} {near_right}");
        // The pull grows with the distance outside it up to `voidAvoidanceDistance` (100): 100 outside is the whole of it.
        let weak = steer(Vec2::new(-650.0, 0.0), &[zone(0, -500.0, 100.0)]);
        let full = steer(Vec2::new(-800.0, 0.0), &[zone(0, -500.0, 100.0)]);
        assert!(weak.abs() <= full.abs());
    }

    #[test]
    fn whole_bibite_swallow_kills_prey_and_transfers_matter() {
        let (mut store, settings, genome, brain) = fixture();
        let predator = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let prey = spawn_adult(&mut store, &genome, &brain, Vec2::new(1.0, 0.0));
        let mut services = Services::new();
        let mut ctx = services.ctx(&settings);
        store.resolve_swallow(PendingSwallow { predator, prey }, &mut ctx);
        assert!(store.get(prey).is_none());
        assert!(store.get(predator).unwrap().stomach.total_amount() > 0.0);
        assert!(services.events.iter().any(|event| matches!(
            event,
            BibiteEvent::Death { bibite, cause }
                if *bibite == prey && *cause == DeathCause::Swallowed
        )));
    }

    #[test]
    fn bibite_grab_adds_held_target() {
        let (mut store, settings, genome, brain) = fixture();
        let predator = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let prey = spawn_adult(&mut store, &genome, &brain, Vec2::new(1.0, 0.0));
        let mut services = Services::new();
        let snapshot = store
            .snapshot_target(bibite_tag(prey), &services.pellets)
            .unwrap();
        let record = store.get_mut(predator).unwrap();
        record.mouth.desire_grab = 1.0;
        record.mouth.desire_eat = 0.0;
        record.mouth.desire_attack = 0.0;
        record.mouth.can_swallow = false;
        let mut ctx = services.ctx(&settings);
        tick_mouth(record, &mut ctx, &[snapshot]);
        assert_eq!(record.n_held_targets, 1);
    }

    #[test]
    fn what_leaves_the_mouth_is_let_go_and_what_stays_is_kept() {
        // `OnTriggerExit2D` -> `ReleaseTargetIfHeld`.
        let (mut store, settings, genome, brain) = fixture();
        let predator = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let kept = spawn_adult(&mut store, &genome, &brain, Vec2::new(1.0, 0.0));
        let gone = spawn_adult(&mut store, &genome, &brain, Vec2::new(2.0, 0.0));
        let mut services = Services::new();
        let record = store.get_mut(predator).unwrap();
        record.held_targets[0] = bibite_tag(kept);
        record.held_targets[1] = bibite_tag(gone);
        record.n_held_targets = 2;
        record.mouth_targets[0] = bibite_tag(kept);
        record.mouth_target_ages[0] = 0;
        record.n_mouth_targets = 1;
        record.mouth.desire_grab = 0.5;
        let mut ctx = services.ctx(&settings);
        tick_mouth(record, &mut ctx, &[]);
        assert_eq!(record.n_held_targets, 1);
        assert_eq!(record.held_targets[0], bibite_tag(kept));
    }

    #[test]
    fn a_holder_can_hold_ten() {
        // `links = new FixedJoint2D[10]`.
        let (mut store, _settings, genome, brain) = fixture();
        let holder = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let record = store.get_mut(holder).unwrap();
        for tag in 1..=12u64 {
            record.add_held(tag);
        }
        assert_eq!(record.n_held_targets, 10);
        assert!(record.is_held(10));
        assert!(!record.is_held(11));
    }

    #[test]
    fn a_throw_shares_its_force_between_everything_held() {
        // `ReleaseAndMaybeThrowAllHeldObjects`: the force follows the summed mass of what is held, each held body gets an impulse of
        // force / nHeld on top of its velocity, and the holder recoils below -0.25.
        let (mut store, settings, genome, brain) = fixture();
        let holder = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let prey = spawn_adult(&mut store, &genome, &brain, Vec2::new(0.0, 8.0));
        let mut pellets = PelletStore::default();
        let pellet = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::new(0.0, 6.0)));
        let before = pellets.pose(pellet).unwrap();
        store.get_mut(prey).unwrap().vel = Vec2::new(1.0, 0.0);
        let prey_mass = store.get(prey).unwrap().current_mass(&settings).max(0.001);
        let pellet_mass = (30.0 * pellets.materials().get(MaterialKind::Plant).mass_density).max(0.000_1);
        let (factor, jaw_strength, jaw_mass) = (settings.scenario.throwing_force_factor, 50.0_f32, 2.0_f32);
        let throw = PendingThrow {
            holder,
            prey: vec![prey],
            pellets: vec![pellet],
            up: Vec2::Y,
            n_held: 2.0,
            throwing_force_factor: factor,
            jaw_strength,
            desire: -1.0,
            jaw_mass,
        };
        let holder_mass = store.get(holder).unwrap().current_mass(&settings).max(0.001);
        store.resolve_throw(&throw, &mut pellets, &settings);
        let total = prey_mass + pellet_mass;
        let force = factor * jaw_strength * total / (jaw_mass + total);
        let after = pellets.pose(pellet).unwrap();
        assert!((after.velocity.y - before.velocity.y - force / 2.0 / pellet_mass).abs() < 1e-3 * (1.0 + after.velocity.y.abs()));
        let thrown = store.get(prey).unwrap();
        assert!((thrown.vel.y - force / 2.0 / prey_mass).abs() < 1e-3 * (1.0 + thrown.vel.y.abs()));
        assert_eq!(thrown.vel.x, 1.0, "the velocity it had is kept");
        assert!((store.get(holder).unwrap().vel.y + force / holder_mass).abs() < 1e-3 * (1.0 + (force / holder_mass).abs()));
    }

    #[test]
    fn a_throw_of_a_mass_that_overflows_the_force_moves_nothing() {
        // A held wall of the maze save weighs infinity (plant density 1e35 over 423,000 u^2): the game's `Mtotal / (jawMass + Mtotal)` is NaN (`BibiteMouth.cs:792`),
        // and a NaN added to the velocities killed the holder (a bibite with a NaN pose dies as a hazard). A force that is not a number moves nothing.
        let (mut store, settings, genome, brain) = fixture();
        let holder = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let mut pellets = PelletStore::default();
        pellets.materials_mut().get_mut(MaterialKind::Plant).mass_density = 1.0e35;
        let wall = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 423_065.0, Vec2::new(0.0, 6.0)));
        let before = pellets.pose(wall).unwrap();
        store.get_mut(holder).unwrap().vel = Vec2::new(2.0, 1.0);
        let throw = PendingThrow {
            holder,
            prey: Vec::new(),
            pellets: vec![wall],
            up: Vec2::Y,
            n_held: 1.0,
            throwing_force_factor: settings.scenario.throwing_force_factor,
            jaw_strength: 2000.0,
            desire: -1.0,
            jaw_mass: 2.0,
        };
        store.resolve_throw(&throw, &mut pellets, &settings);
        assert_eq!(store.get(holder).unwrap().vel, Vec2::new(2.0, 1.0), "the holder is not pushed by a force that is not a number");
        assert_eq!(pellets.pose(wall).unwrap().velocity, before.velocity, "and the wall keeps its velocity");
        // An ordinary throw of the same holder still works.
        let mut ordinary = PelletStore::default();
        let pellet = ordinary.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::new(0.0, 6.0)));
        let throw = PendingThrow { pellets: vec![pellet], ..throw };
        store.resolve_throw(&throw, &mut ordinary, &settings);
        assert!(ordinary.pose(pellet).unwrap().velocity.y > 0.0, "an ordinary pellet is thrown");
    }

    #[test]
    fn a_mouth_that_holds_ten_flags_no_pellet_as_held() {
        // `TryGrabTarget`: `if (nHeld < 10)` comes before `targetToGrab.TryGrab(this)`.
        let (mut store, settings, genome, brain) = fixture();
        let holder = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        let mut services = Services::new();
        let first = services.pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 5.0, Vec2::new(1.0, 0.0)));
        let second = services.pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 5.0, Vec2::new(1.5, 0.0)));
        let targets = [
            store.snapshot_target(pellet_tag(first), &services.pellets).unwrap(),
            store.snapshot_target(pellet_tag(second), &services.pellets).unwrap(),
        ];
        let record = store.get_mut(holder).unwrap();
        record.mouth.desire_grab = 1.0;
        record.mouth.desire_eat = 0.0;
        record.mouth.desire_attack = 0.0;
        record.mouth.can_swallow = false;
        // Nine are held already, and in the mouth (what is held and not in the mouth is let go first): the first pellet is the tenth, the second finds the mouth full.
        for (at, tag) in (1..=9u64).enumerate() {
            record.add_held(tag);
            record.mouth_targets[at] = tag;
            record.mouth_target_ages[at] = 0;
        }
        record.n_mouth_targets = 9;
        {
            let mut ctx = services.ctx(&settings);
            tick_mouth(record, &mut ctx, &targets);
        }
        assert_eq!(record.n_held_targets, 10);
        assert!(record.is_held(pellet_tag(first)) && !record.is_held(pellet_tag(second)));
        assert!(services.pellets.is_held(first), "the pellet the mouth holds is flagged");
        assert!(!services.pellets.is_held(second), "a pellet no mouth holds is not flagged as held");
    }

    #[test]
    fn death_resolver_turns_kill_into_corpse_and_event() {
        let (mut store, settings, genome, brain) = fixture();
        let id = spawn_adult(&mut store, &genome, &brain, Vec2::ZERO);
        store.kill(id, DeathCause::Hazard);
        let mut services = Services::new();
        store.resolve_pending_deaths(
            &mut services.pellets,
            &settings,
            &mut services.rng,
            &mut services.events,
        );
        let record = store.get(id).unwrap();
        assert!(record.is_corpse);
        assert_eq!(record.death_cause, None);
        assert!(
            services
                .events
                .iter()
                .any(|event| matches!(event, BibiteEvent::Death { .. }))
        );
    }
}

#[cfg(test)]
mod b2b_physics_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::rng::Rng;
    use crate::settings::SimSettings;

    fn spawn_pair() -> (BibiteStore, SimSettings, BibiteId, BibiteId) {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::new(-1.0, 0.0);
        let first = store.spawn(spawn).unwrap();
        let mut spawn = BibiteSpawn::new(genome, GenomeMeta::default(), brain);
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::new(1.0, 0.0);
        let second = store.spawn(spawn).unwrap();
        (store, settings, first, second)
    }

    fn services() -> (PelletStore, EggStore, Rng, BibiteScratch) {
        (
            PelletStore::default(),
            EggStore::new(),
            Rng::from_seed(9),
            BibiteScratch::new(),
        )
    }

    #[test]
    fn overlapping_bibites_take_two_sided_collision_damage() {
        let (mut store, settings, first, second) = spawn_pair();
        let max_health = store.get(first).unwrap().body.health.amount;
        for (id, vel) in [
            (first, Vec2::new(200.0, 0.0)),
            (second, Vec2::new(-200.0, 0.0)),
        ] {
            if let Some(record) = store.get_mut(id) {
                record.vel = vel;
                record.body.health.amount = max_health;
            }
        }
        let (mut pellets, mut eggs, mut rng, mut scratch) = services();
        let mut events = Vec::new();
        for _ in 0..20 {
            store.step_physics(
                &mut pellets,
                &mut eggs,
                &settings,
                &mut rng,
                &mut scratch,
                &mut events,
            );
        }
        let a = store.get(first).unwrap();
        let b = store.get(second).unwrap();
        assert!(a.body.health.amount < max_health, "first took no damage");
        assert!(b.body.health.amount < max_health, "second took no damage");
        assert!(a.pos.is_finite() && b.pos.is_finite());
    }

    #[test]
    fn touching_pellets_merge_only_with_the_pellet_merge_setting() {
        let run = |merge: bool| {
            let mut settings = SimSettings::game_defaults();
            settings.scenario.pellet_merge = merge;
            let mut store = BibiteStore::with_settings(settings.clone());
            let (mut pellets, mut eggs, mut rng, mut scratch) = services();
            let big = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 20.0, Vec2::new(0.0, 0.0)));
            let small = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 5.0, Vec2::new(1.0, 0.0)));
            let mut events = Vec::new();
            for _ in 0..3 {
                store.step_physics(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events);
            }
            (pellets.get(big).map(|view| view.amount), pellets.get(small).is_some())
        };
        let (kept, small_alive) = run(true);
        assert!(!small_alive, "the smaller pellet was absorbed");
        assert!((kept.expect("the big one stays") - 25.0).abs() < 1e-3);
        let (kept, small_alive) = run(false);
        assert!(small_alive && (kept.expect("the big one stays") - 20.0).abs() < 1e-3);
    }

    #[test]
    fn a_pellets_drag_factor_scales_its_linear_damping_and_nothing_else() {
        let settings = SimSettings::game_defaults();
        let (mut pellets, _, _, _) = services();
        let id = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 20.0, Vec2::ZERO));
        let plain = build_pellet_physics_body(&pellets, id, &settings).expect("a body");
        assert!(plain.linear_damping > 0.0);
        assert!(pellets.set_drag_factor(id, 0.1));
        let gliding = build_pellet_physics_body(&pellets, id, &settings).expect("a body");
        assert!((gliding.linear_damping - 0.1 * plain.linear_damping).abs() < 1e-6 * plain.linear_damping.max(1.0), "{} vs {}", gliding.linear_damping, plain.linear_damping);
        assert_eq!((gliding.angular_damping, gliding.colliders()[0].is_sensor), (plain.angular_damping, plain.colliders()[0].is_sensor));
    }

    #[test]
    fn pellets_without_collision_have_a_trigger_collider_and_no_spin() {
        let make = |collision: bool| {
            let mut settings = SimSettings::game_defaults();
            settings.scenario.pellet_collision = collision;
            let (mut pellets, _, _, _) = services();
            let id = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 20.0, Vec2::ZERO));
            build_pellet_physics_body(&pellets, id, &settings).expect("a body")
        };
        assert!(!make(true).colliders()[0].is_sensor);
        let body = make(false);
        assert!(body.colliders()[0].is_sensor && body.ang_vel == 0.0);
    }

    #[test]
    fn mouth_sensor_populates_target_list() {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut spawn = BibiteSpawn::new(genome, GenomeMeta::default(), brain);
        spawn.vitals = SpawnVitals::Adult;
        let id = store.spawn(spawn).unwrap();
        let d1 = store.get(id).unwrap().derived.d1_size;
        let mouth = crate::physics::prefab::mouth_offset(d1);
        let (mut pellets, mut eggs, mut rng, mut scratch) = services();
        let pellet = pellets.spawn(crate::pellet::PelletSpawn::new(
            MaterialKind::Plant,
            20.0,
            Vec2::new(mouth.x, mouth.y),
        ));
        let mut events = Vec::new();
        store.step_physics(
            &mut pellets,
            &mut eggs,
            &settings,
            &mut rng,
            &mut scratch,
            &mut events,
        );
        let record = store.get(id).unwrap();
        assert_eq!(record.n_mouth_targets, 1);
        assert_eq!(record.mouth_targets[0], pellet_tag(pellet));
    }

    #[test]
    fn physics_step_is_deterministic_for_same_state() {
        let settings = SimSettings::game_defaults();
        let build = || {
            let genome = Genome::starter();
            let brain = Brain::starter();
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut spawn = BibiteSpawn::new(genome, GenomeMeta::default(), brain);
            spawn.vitals = SpawnVitals::Adult;
            let a = store.spawn(spawn.clone()).unwrap();
            let mut second = spawn;
            second.pos = Vec2::new(0.5, 0.25);
            let b = store.spawn(second).unwrap();
            (store, a, b)
        };
        let (mut first, a1, b1) = build();
        let (mut second, a2, b2) = build();
        if let Some(record) = first.get_mut(a1) {
            record.vel = Vec2::new(10.0, 3.0);
        }
        if let Some(record) = second.get_mut(a2) {
            record.vel = Vec2::new(10.0, 3.0);
        }
        let (mut p1, mut e1, mut r1, mut s1) = services();
        let (mut p2, mut e2, mut r2, mut s2) = services();
        let mut events = Vec::new();
        first.step_physics(&mut p1, &mut e1, &settings, &mut r1, &mut s1, &mut events);
        second.step_physics(&mut p2, &mut e2, &settings, &mut r2, &mut s2, &mut events);
        assert_eq!(first.get(a1).unwrap().pos, second.get(a2).unwrap().pos);
        assert_eq!(first.get(b1).unwrap().pos, second.get(b2).unwrap().pos);
    }
}

#[cfg(test)]
mod holder_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::pellet::{MaterialKind, PelletSpawn};
    use crate::rng::Rng;
    use crate::settings::SimSettings;
    use crate::sim::{ParallelExecutor, ParallelGates, ThreadPool};

    fn spawn_adult_at(store: &mut BibiteStore, pos: Vec2) -> BibiteId {
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = pos;
        store.spawn(spawn).unwrap()
    }

    /// `count` bibites on a grid, every seventh removed and a few spawned after, so that the dense list is not in slot order;
    /// every fifth of the ones left holds one or two things.
    fn store_with_holders(count: usize) -> BibiteStore {
        let mut store = BibiteStore::with_settings(SimSettings::game_defaults());
        let mut ids = Vec::new();
        for index in 0..count {
            let pos = Vec2::new((index % 20) as f32 * 25.0, (index / 20) as f32 * 25.0);
            ids.push(spawn_adult_at(&mut store, pos));
        }
        let mut kept = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            if index % 7 == 3 {
                store.remove(*id);
            } else {
                kept.push(*id);
            }
        }
        for index in 0..count / 10 {
            kept.push(spawn_adult_at(
                &mut store,
                Vec2::new(index as f32 * 25.0, -60.0),
            ));
        }
        for (index, id) in kept.iter().enumerate() {
            if index % 5 == 0 {
                let record = store.get_mut(*id).unwrap();
                record.held_targets[0] =
                    pellet_tag(PelletId::from_handle(Handle::new(index as u32, 1)));
                record.n_held_targets = 1 + (index % 2) as u8;
            }
        }
        store
    }

    fn physics_steps(
        store: &mut BibiteStore,
        pellets: &mut PelletStore,
        eggs: &mut EggStore,
        rng: &mut Rng,
        scratch: &mut BibiteScratch,
        settings: &SimSettings,
        ticks: usize,
    ) {
        let mut events = Vec::new();
        for _ in 0..ticks {
            store.step_physics_with_executor(
                pellets,
                eggs,
                settings,
                rng,
                scratch,
                &mut events,
                ParallelExecutor::Serial,
            );
        }
    }

    /// Every size of the store's gates reaches the physics step (its per-body stages, narrowphase, sorts and islands are cut by
    /// what the world says), whichever gates it has: the scaled ones a world starts with, the always-on ones, and the eager ones.
    #[test]
    fn the_gates_of_the_store_reach_the_physics_step() {
        for (name, gates) in [
            ("scaled", ParallelGates::SCALED),
            ("always", ParallelGates::ALWAYS),
            ("eager", ParallelGates::EAGER),
        ] {
            let settings = SimSettings::game_defaults();
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(7);
            let mut scratch = BibiteScratch::new();
            store.set_parallel_gates(gates);
            spawn_adult_at(&mut store, Vec2::ZERO);
            spawn_adult_at(&mut store, Vec2::new(0.0, 7.0));
            let pool = ThreadPool::new(3);
            let mut events = Vec::new();
            store.step_physics_with_executor(
                &mut pellets,
                &mut eggs,
                &settings,
                &mut rng,
                &mut scratch,
                &mut events,
                ParallelExecutor::new(Some(&pool)),
            );
            let physics = &scratch.physics;
            assert_eq!(physics.threads, 3, "{name}");
            assert_eq!(
                [
                    physics.min_bodies,
                    physics.min_broadphase,
                    physics.min_narrow_pairs,
                    physics.min_sort,
                    physics.min_island_bodies,
                    physics.min_island_constraints,
                ],
                [
                    gates.bodies,
                    gates.broadphase,
                    gates.narrow_pairs,
                    gates.sorts,
                    gates.island_bodies,
                    gates.island_constraints,
                ],
                "{name}: the gates the physics step was given"
            );
        }
    }

    /// A holder holds a prey that is then gone, and a newcomer far away takes the prey's slot (a hatchling takes the slot of what was just eaten, in the
    /// same tick): the hold is a tag from the past, and the weld must not join the holder to the newcomer. It did, with the frame the prey had, and the rigid
    /// weld pulled the newcomer across the world in one step (welds with errors of 4,000 and 21,000 units in the glitch save of 2026-11-13); what the solver
    /// then did to the pair threw bodies out of the world until their positions were NaN.
    #[test]
    fn a_hold_on_a_bibite_that_is_gone_does_not_weld_the_newcomer_in_its_slot() {
        for threads in [1usize, 4] {
            // (The world wraps what goes beyond `1.5 S + 1000` to the other side: not what this test is about.)
            let mut settings = SimSettings::game_defaults();
            settings.scenario.shade_outside_of_bounds = false;
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(5);
            let mut scratch = BibiteScratch::new();
            store.set_parallel_gates(ParallelGates::EAGER);
            let pool = (threads > 1).then(|| ThreadPool::new(threads));
            let holder = spawn_adult_at(&mut store, Vec2::ZERO);
            let prey = spawn_adult_at(&mut store, Vec2::new(0.0, 7.0));
            {
                let record = store.get_mut(holder).unwrap();
                record.held_targets[0] = bibite_tag(prey);
                record.n_held_targets = 1;
                record.mouth.n_held = 1;
            }
            let mut events = Vec::new();
            let mut step = |store: &mut BibiteStore, ticks: usize| {
                for _ in 0..ticks {
                    store.step_physics_with_executor(
                        &mut pellets,
                        &mut eggs,
                        &settings,
                        &mut rng,
                        &mut scratch,
                        &mut events,
                        ParallelExecutor::new(pool.as_ref()),
                    );
                }
            };
            step(&mut store, 3);
            assert!(store.get(holder).unwrap().is_held(bibite_tag(prey)));
            store.remove(prey);
            let newcomer = spawn_adult_at(&mut store, Vec2::new(5000.0, -3000.0));
            assert_eq!(newcomer.handle().slot(), prey.handle().slot(), "the newcomer takes the slot of the prey");
            assert_ne!(newcomer, prey, "and is another bibite");
            let before = store.get(newcomer).unwrap().pos;
            step(&mut store, 6);
            let moved = (store.get(newcomer).unwrap().pos - before).length();
            assert!(moved < 50.0, "{threads} threads: the newcomer was pulled {moved} units by a hold on the bibite that was in its slot");
            assert!(
                (store.get(holder).unwrap().pos.length()) < 50.0,
                "{threads} threads: the holder was pulled to {:?}",
                store.get(holder).unwrap().pos
            );
        }
    }

    /// `BibiteMouth.OnDeathStatusChange(false)`: a bibite that dies lets go of everything it holds (a corpse that kept its holds stayed welded to them).
    #[test]
    fn a_bibite_that_dies_lets_go_of_what_it_holds() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut rng = Rng::from_seed(5);
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let prey = spawn_adult_at(&mut store, Vec2::new(0.0, 9.0));
        let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::new(4.0, 9.0)));
        {
            let record = store.get_mut(holder).unwrap();
            record.held_targets[0] = bibite_tag(prey);
            record.held_targets[1] = pellet_tag(pellet);
            record.n_held_targets = 2;
            record.mouth.n_held = 2;
        }
        pellets.set_held(pellet, true);
        store.kill(holder, DeathCause::Hazard);
        let mut events = Vec::new();
        store.resolve_pending_deaths(&mut pellets, &settings, &mut rng, &mut events);
        let record = store.get(holder).expect("a corpse stays");
        assert!(record.body.dead && record.is_corpse);
        assert_eq!(record.n_held_targets, 0, "the corpse holds nothing");
        assert_eq!(record.mouth.n_held, 0);
        assert!(!pellets.is_held(pellet), "the pellet is let go of");
        assert!(store.get(prey).is_some(), "and the prey is not touched");
    }

    /// A bibite that holds something and is eaten, or bursts into meat (the record is removed): its pellets are let go of too (a pellet flagged as held
    /// by a bibite that is gone is held for ever: no drag, never asleep).
    #[test]
    fn a_bibite_that_is_swallowed_lets_go_of_its_pellets() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut rng = Rng::from_seed(5);
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::new(4.0, 9.0)));
        {
            let record = store.get_mut(holder).unwrap();
            record.held_targets[0] = pellet_tag(pellet);
            record.n_held_targets = 1;
            record.mouth.n_held = 1;
        }
        pellets.set_held(pellet, true);
        let mut events = Vec::new();
        store.die_now(holder, DeathCause::Swallowed, true, &mut pellets, &mut rng, &settings, &mut events);
        assert!(store.get(holder).is_none(), "the swallowed bibite is destroyed");
        assert!(!pellets.is_held(pellet), "its pellet is let go of");
    }

    /// A bibite whose size is not a number (a negative growth with a fractional exponent makes one) leaves no corpse and no meat when it dies: a corpse of NaN
    /// mass would weigh the least a body can (the `max` of a physics body's mass ignores NaN), and a pile of NaN energy is NaN for good.
    #[test]
    fn a_bibite_with_a_nan_size_is_destroyed_when_it_dies() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut rng = Rng::from_seed(5);
        let broken = spawn_adult_at(&mut store, Vec2::ZERO);
        let healthy = spawn_adult_at(&mut store, Vec2::new(300.0, 0.0));
        store.get_mut(broken).unwrap().body.geometry.d2_size = f32::NAN;
        store.kill(broken, DeathCause::Starvation);
        store.kill(healthy, DeathCause::Starvation);
        let mut events = Vec::new();
        store.resolve_pending_deaths(&mut pellets, &settings, &mut rng, &mut events);
        assert!(store.get(broken).is_none(), "the bibite of NaN size is destroyed");
        assert!(store.get(healthy).is_some_and(|record| record.is_corpse), "a bibite with numbers leaves a corpse");
        assert_eq!(pellets.len(), 0, "and no pile of meat is made of NaN");
        assert!(events.iter().any(|event| matches!(event, BibiteEvent::Death { bibite, .. } if *bibite == broken)));
    }

    /// A weld whose two anchors are thousands of units apart is let go of, not solved: the rigid weld would take the whole error out in one step.
    #[test]
    fn a_weld_between_bodies_far_apart_is_let_go_of_and_pulls_nobody() {
        let mut settings = SimSettings::game_defaults();
        settings.scenario.shade_outside_of_bounds = false;
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(5);
        let mut scratch = BibiteScratch::new();
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        // A pellet held by the holder, in the frame the holder had it in (7 units ahead), and now thousands of units away (a wrapped holder, a stale frame).
        let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::new(0.0, 7.0)));
        {
            let record = store.get_mut(holder).unwrap();
            record.held_targets[0] = pellet_tag(pellet);
            record.n_held_targets = 1;
            record.mouth.n_held = 1;
        }
        pellets.set_held(pellet, true);
        physics_steps(&mut store, &mut pellets, &mut eggs, &mut rng, &mut scratch, &settings, 3);
        assert!(store.get(holder).unwrap().is_held(pellet_tag(pellet)), "the pair is joined");
        // The holder is moved (the weld keeps the frame): 6,000 units away from the pellet.
        store.get_mut(holder).unwrap().pos = Vec2::new(6000.0, 0.0);
        physics_steps(&mut store, &mut pellets, &mut eggs, &mut rng, &mut scratch, &settings, 2);
        let pellet_at = pellets.pos(pellet).unwrap();
        assert!(
            (pellet_at - Vec2::new(0.0, 7.0)).length() < 100.0,
            "the pellet was pulled to {pellet_at:?}"
        );
        assert!(
            store.get(holder).unwrap().pos.x > 5900.0,
            "the holder was pulled to {:?}",
            store.get(holder).unwrap().pos
        );
        assert!(!store.get(holder).unwrap().is_held(pellet_tag(pellet)), "the weld that cannot be one is let go of");
        assert!(!pellets.is_held(pellet));
    }

    /// A body the step leaves with a pose that is not a number is not written back and its bibite dies: a NaN position stays NaN for good (every number
    /// made from it is), nothing touches the body, the bibite can neither eat nor be eaten, and its eggs and children are born at NaN.
    #[test]
    fn a_bibite_the_step_leaves_with_a_nan_pose_dies_and_keeps_its_last_position() {
        for threads in [1usize, 4] {
            let mut settings = SimSettings::game_defaults();
            settings.scenario.shade_outside_of_bounds = false;
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(5);
            let mut scratch = BibiteScratch::new();
            store.set_parallel_gates(ParallelGates::EAGER);
            let pool = (threads > 1).then(|| ThreadPool::new(threads));
            let bad = spawn_adult_at(&mut store, Vec2::new(100.0, 50.0));
            let fine = spawn_adult_at(&mut store, Vec2::new(-900.0, 50.0));
            store.get_mut(bad).unwrap().vel = Vec2::new(f32::NAN, f32::NAN);
            let mut events = Vec::new();
            for _ in 0..3 {
                store.step_physics_with_executor(
                    &mut pellets,
                    &mut eggs,
                    &settings,
                    &mut rng,
                    &mut scratch,
                    &mut events,
                    ParallelExecutor::new(pool.as_ref()),
                );
            }
            match store.get(bad) {
                Some(record) => {
                    assert!(record.body.dead, "{threads} threads: the bibite with a NaN pose is alive");
                    assert!(record.pos.is_finite() && record.vel.is_finite() && record.rot.is_finite(), "{threads} threads: its pose is {:?} {:?} {}", record.pos, record.vel, record.rot);
                    assert_eq!(record.pos, Vec2::new(100.0, 50.0), "it keeps the last position it had");
                }
                None => {}
            }
            let fine = store.get(fine).unwrap();
            assert!(!fine.body.dead && fine.pos.is_finite(), "{threads} threads: the others are untouched");
            assert!(events.iter().any(|event| matches!(event, BibiteEvent::Death { bibite, .. } if *bibite == bad)), "{threads} threads: a death event was reported");
        }
    }

    /// The same for a pellet and an egg: what the step leaves of them when it is not a number is not written back.
    #[test]
    fn a_pellet_and_an_egg_the_step_leaves_with_nan_stay_where_they_were() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(5);
        let mut scratch = BibiteScratch::new();
        let _bibite = spawn_adult_at(&mut store, Vec2::new(500.0, 0.0));
        let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::new(-300.0, 20.0)));
        let _ = pellets.set_velocity(pellet, Vec2::new(f32::NAN, 1.0));
        physics_steps(&mut store, &mut pellets, &mut eggs, &mut rng, &mut scratch, &settings, 3);
        let at = pellets.pos(pellet).unwrap();
        assert!(at.is_finite(), "the pellet is at {at:?}");
    }

    #[test]
    fn the_holders_found_with_the_pool_are_the_ones_a_plain_scan_finds() {
        for count in [0_usize, 1, 5, 64, 300] {
            let mut store = store_with_holders(count);
            let plain: Vec<BibiteId> = store
                .iter()
                .filter(|(_, record)| record.n_held_targets > 0)
                .map(|(id, _)| id)
                .collect();
            if count >= 20 {
                assert!(
                    !plain.is_empty(),
                    "{count} bibites: someone holds something"
                );
                assert!(
                    plain.len() < store.len(),
                    "{count} bibites: not everyone does"
                );
            }
            for gates in [ParallelGates::default(), ParallelGates::EAGER] {
                store.set_parallel_gates(gates);
                for threads in [1_usize, 3, 8] {
                    let pool = (threads > 1).then(|| ThreadPool::new(threads));
                    let found = store.holders(ParallelExecutor::new(pool.as_ref()));
                    assert_eq!(found, plain, "{count} bibites, {threads} threads");
                }
            }
        }
    }

    /// What the holders hold is pinned to their mouths before the step. Here every second bibite holds a pellet that is
    /// stored on the mouth of the next one: pinned, it touches nobody; left there, the neighbour's mouth finds it. A pool that
    /// missed a holder would leave the pellet in place and change what the neighbour's mouth sees.
    #[test]
    fn held_pellets_follow_their_holders_whatever_finds_the_holders() {
        let run = |threads: usize| {
            let settings = SimSettings::game_defaults();
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(9);
            let mut scratch = BibiteScratch::new();
            let mut ids = Vec::new();
            for index in 0..30 {
                let pos = Vec2::new((index % 6) as f32 * 40.0, (index / 6) as f32 * 40.0);
                ids.push(spawn_adult_at(&mut store, pos));
            }
            let mut held = Vec::new();
            for index in (0..ids.len() - 1).step_by(2) {
                let neighbour_mouth = mouth_tip(store.get(ids[index + 1]).unwrap());
                let pellet =
                    pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, neighbour_mouth));
                let record = store.get_mut(ids[index]).unwrap();
                record.held_targets[0] = pellet_tag(pellet);
                record.n_held_targets = 1;
                held.push(pellet);
            }
            store.set_parallel_gates(ParallelGates::EAGER);
            let pool = (threads > 1).then(|| ThreadPool::new(threads));
            let mut events = Vec::new();
            for _ in 0..3 {
                store.step_physics_with_executor(
                    &mut pellets,
                    &mut eggs,
                    &settings,
                    &mut rng,
                    &mut scratch,
                    &mut events,
                    ParallelExecutor::new(pool.as_ref()),
                );
            }
            let bibites: Vec<_> = ids
                .iter()
                .map(|id| {
                    let record = store.get(*id).unwrap();
                    (
                        record.pos,
                        record.vel,
                        record.rot,
                        record.n_mouth_targets,
                        record.mouth_targets,
                    )
                })
                .collect();
            let pellets_at: Vec<_> = held.iter().map(|id| pellets.pos(*id)).collect();
            (bibites, pellets_at)
        };
        let serial = run(1);
        assert_eq!(run(3), serial);
        assert_eq!(run(8), serial);
    }

    /// What a holder holds is joined to it (the game's `FixedJoint2D`): the pair is pushed by the holder's muscles but weighs both, so a holder
    /// with a heavy pellet picks up less speed from the same push.
    #[test]
    fn a_holder_with_a_heavy_pellet_speeds_up_less() {
        let speed_after = |holding: bool| {
            let settings = SimSettings::game_defaults();
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(9);
            let mut scratch = BibiteScratch::new();
            let holder = spawn_adult_at(&mut store, Vec2::ZERO);
            if holding {
                let mouth = mouth_tip(store.get(holder).unwrap());
                let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 400.0, mouth));
                let record = store.get_mut(holder).unwrap();
                record.held_targets[0] = pellet_tag(pellet);
                record.n_held_targets = 1;
            }
            let mut events = Vec::new();
            store.get_mut(holder).unwrap().pending_force = Vec2::new(0.0, 80.0);
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
            store.get(holder).unwrap().vel.length()
        };
        let (free, loaded) = (speed_after(false), speed_after(true));
        assert!(free > 0.0 && loaded < 0.8 * free, "free {free}, with the pellet {loaded}");
    }

    /// What is held keeps the pose it had when it was grabbed (the game's `FixedJoint2D` joins the two as they are), however the holder moves and turns,
    /// and the pair reaches the same speed as the holder pushing the pellet in front of it (the terminal speeds measured against the original).
    #[test]
    fn a_held_pellet_keeps_its_place_relative_to_a_moving_holder() {
        let _ten = phys::ten_steps_for_this_test();
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(9);
        let mut scratch = BibiteScratch::new();
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 60.0, Vec2::new(0.0, 6.0)));
        {
            let record = store.get_mut(holder).unwrap();
            record.held_targets[0] = pellet_tag(pellet);
            record.n_held_targets = 1;
            record.mouth.n_held = 1;
            record.mouth.desire_grab = 1.0;
        }
        pellets.set_held(pellet, true);
        let mut events = Vec::new();
        for _ in 0..40 {
            let record = store.get_mut(holder).unwrap();
            record.pending_force = Vec2::Y.rotate(record.rot) * 60.0;
            record.pending_torque = 400.0;
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
        }
        let record = store.get(holder).unwrap();
        assert!(record.pos.length() > 3.0 && record.rot.abs() > 0.2, "the holder moved to {:?}, turned {}", record.pos, record.rot);
        let offset = (pellets.pos(pellet).unwrap() - record.pos).rotate(-record.rot);
        assert!((offset - Vec2::new(0.0, 6.0)).length() < 0.1, "the pellet is at {offset:?} in the holder's frame");
        assert_eq!(record.n_held_targets, 1, "the joint held");
    }

    /// A bibite grabbed while its body overlaps the holder's mouth is held without the pair shaking: the weld keeps the pose it was grabbed in, and the pair's own
    /// contact (the joint collides, as the game's does) must not push against it.
    #[test]
    fn a_prey_grabbed_in_the_mouth_is_held_without_jolting() {
        let _ten = phys::ten_steps_for_this_test();
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(5);
        let mut scratch = BibiteScratch::new();
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let prey = spawn_adult_at(&mut store, Vec2::new(0.0, 6.5));
        let mut events = Vec::new();
        let mut last = Vec2::ZERO;
        let mut worst = 0.0f32;
        for tick in 0..90 {
            let record = store.get_mut(holder).unwrap();
            record.mouth.desire_grab = 1.0;
            let record = store.get_mut(prey).unwrap();
            record.pending_force = Vec2::Y.rotate(record.rot) * 30.0;
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
            let record = store.get(holder).unwrap();
            if tick >= 10 {
                worst = worst.max((record.vel - last).length());
            }
            last = record.vel;
        }
        assert!(store.get(holder).unwrap().is_held(bibite_tag(prey)), "the prey was not held");
        assert!(worst < 3.0, "the holder's velocity jumped by {worst} in one tick");
    }

    /// At ONE physics step per tick (the game at 1x) a bibite that grabs a pellet carries it: the weld forms at the start of the step in which the mouth's trigger
    /// meets the pellet and the pellet moves with the holder from that same step. The plain one-step solver knew no joints, so the pellet stayed where it was for
    /// several ticks and the holder drove through it. (Measured against the game: a bibite accelerating at 0.6 towards a 60-unit pellet 18 u ahead holds it 10.81 u
    /// ahead of its centre, the port 10.86; here the pellet starts 12 u ahead because the thrust of the test is small.)
    #[test]
    fn at_one_step_per_tick_a_grabbed_pellet_travels_with_its_holder() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(5);
        let mut scratch = BibiteScratch::new();
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let pellet = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 60.0, Vec2::new(0.0, 12.0)));
        let mut events = Vec::new();
        let mut held_at = None;
        let mut offsets: Vec<f32> = Vec::new();
        for tick in 0..60 {
            let record = store.get_mut(holder).unwrap();
            record.mouth.desire_grab = 1.0;
            record.pending_force = Vec2::Y.rotate(record.rot) * 40.0;
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
            if held_at.is_none() && store.get(holder).unwrap().is_held(pellet_tag(pellet)) {
                held_at = Some(tick);
            }
            if held_at.is_some_and(|at| tick > at + 1) {
                offsets.push(pellets.pos(pellet).unwrap().y - store.get(holder).unwrap().pos.y);
            }
        }
        assert!(held_at.is_some(), "the pellet was not held");
        assert!(offsets.len() > 10);
        let (lowest, highest) = offsets.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &x| (low.min(x), high.max(x)));
        assert!(highest - lowest < 0.5, "the held pellet slid in the mouth: its offset went from {lowest} to {highest}");
        assert!(lowest > 4.0, "the pellet is inside the holder: offset {lowest}");
    }

    /// A bibite holding a pellet thousands of times its own mass, a long way off (the Archipelago save's holders): the weld is a three-row constraint solved as one
    /// block, so the light body neither rings nor builds up speed under its own thrust and turning.
    #[test]
    fn a_light_holder_of_a_huge_pellet_stays_calm() {
        let _ten = phys::ten_steps_for_this_test();
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(5);
        let mut scratch = BibiteScratch::new();
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let pellet = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 1900.0, Vec2::new(0.0, 24.0)));
        let mut events = Vec::new();
        let mut fastest = 0.0f32;
        for tick in 0..90 {
            let record = store.get_mut(holder).unwrap();
            record.mouth.desire_grab = 1.0;
            record.pending_force = Vec2::Y.rotate(record.rot) * 25.0;
            record.pending_torque = 15.0;
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
            if tick >= 15 {
                fastest = fastest.max(store.get(holder).unwrap().vel.length());
            }
        }
        assert!(store.get(holder).unwrap().is_held(pellet_tag(pellet)), "the pellet was not held");
        assert!(fastest < 12.0, "the holder reached {fastest} u/s");
    }

    /// The maze save of 2026-11-15: the plants weigh 1e35 a square unit, so a pellet of 423,000 u^2 (a collider of radius 364) is past the largest f32, a mass with no inverse,
    /// and a wall. Bibites that touch one grab it (a grab has no limit of mass) and are welded to it, 364 units from the weld's anchor when that was the held body's origin;
    /// the meat that a bibite that starved there left lies pressed against the rim, and the next bibite that grabbed it as well was thrown at 434 u/s, spinning at 414 rad/s,
    /// and hurt what it hit. A holder of a wall that grabs a meat pellet next to it, under its own thrust and turning, stays where it is. (The whole flow: the density, the grab of
    /// both, the weld. A holder alone against one wall was calm with the old anchor too: what threw the bibite of the maze save needed the weld's block to be badly conditioned,
    /// which `a_light_body_welded_to_a_giant_one_far_off_is_not_thrown_by_its_own_turning` in `physics.rs` has, and fails without the fix.)
    #[test]
    fn a_holder_of_a_wall_that_grabs_the_meat_pressed_against_it_is_not_thrown() {
        let settings = SimSettings::game_defaults();
        let radius = 2.5 * (423_065.0f32 / 20.0).sqrt();
        for (variant, meat_at) in [("meat ahead of the mouth", Vec2::new(-1.0, 4.4)), ("meat beside the mouth", Vec2::new(-3.5, 3.9)), ("meat under the holder", Vec2::new(-0.5, 2.2))] {
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            pellets.materials_mut().plant.mass_density = 1.0e35;
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(11);
            let mut scratch = BibiteScratch::new();
            let holder = spawn_adult_at(&mut store, Vec2::ZERO);
            let wall = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 423_065.0, Vec2::new(0.0, radius + 3.1)));
            let meat = pellets.spawn(PelletSpawn::new(MaterialKind::Meat, 4.74, meat_at));
            let mut events = Vec::new();
            let (mut fastest, mut spin) = (0.0f32, 0.0f32);
            let mut held_both_at = None;
            for tick in 0..150 {
                let record = store.get_mut(holder).unwrap();
                record.mouth.desire_grab = 1.0;
                record.pending_force = Vec2::Y.rotate(record.rot) * 30.0;
                record.pending_torque = if tick % 40 < 20 { 6.0 } else { -6.0 };
                store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
                let record = store.get(holder).unwrap();
                fastest = fastest.max(record.vel.length());
                spin = spin.max(record.ang_vel.abs());
                if held_both_at.is_none() && record.is_held(pellet_tag(wall)) && record.is_held(pellet_tag(meat)) {
                    held_both_at = Some(tick);
                }
                assert!(pellets.get(meat).is_none_or(|view| view.velocity.length() < 60.0), "{variant}: the meat left at {} u/s on tick {tick}", pellets.get(meat).unwrap().velocity.length());
            }
            assert!(held_both_at.is_some(), "{variant}: the holder never held both the wall and the meat (holds {} things)", store.get(holder).unwrap().n_held_targets);
            assert!(fastest < 40.0, "{variant}: the holder reached {fastest} u/s");
            assert!(spin < 4.0, "{variant}: the holder spun at {spin} rad/s");
        }
    }

    /// Several bibites hold one big pellet (as in the Archipelago save): each has its own joint, with its own frozen pose and its own carried impulses, so none of
    /// them is thrown about by another's.
    #[test]
    fn two_holders_of_one_pellet_each_keep_their_own_joint() {
        let _ten = phys::ten_steps_for_this_test();
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(5);
        let mut scratch = BibiteScratch::new();
        let left = spawn_adult_at(&mut store, Vec2::new(-10.0, 0.0));
        let right = spawn_adult_at(&mut store, Vec2::new(10.0, 0.0));
        let pellet = pellets.spawn(crate::pellet::PelletSpawn::new(MaterialKind::Plant, 1900.0, Vec2::new(0.0, 26.0)));
        let mut events = Vec::new();
        let mut last = [Vec2::ZERO; 2];
        let mut worst = 0.0f32;
        for tick in 0..90 {
            for (k, id) in [left, right].iter().enumerate() {
                let record = store.get_mut(*id).unwrap();
                record.mouth.desire_grab = 0.5;
                record.pending_force = Vec2::Y.rotate(record.rot) * 20.0;
                record.pending_torque = if k == 0 { 8.0 } else { -8.0 };
            }
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
            for (k, id) in [left, right].iter().enumerate() {
                let record = store.get(*id).unwrap();
                if tick >= 20 {
                    worst = worst.max((record.vel - last[k]).length());
                }
                last[k] = record.vel;
            }
        }
        let holding = [left, right].iter().filter(|id| store.get(**id).unwrap().is_held(pellet_tag(pellet))).count();
        assert!(holding >= 1, "nobody held the pellet");
        assert!(worst < 4.0, "a holder's velocity jumped by {worst} in one tick");
    }

    /// A mouth that wants to grab, driving into a pellet: the physics steps see the trigger enter in the middle of a tick and the grab is made in the next
    /// step (the game's `FixedUpdate` after the `OnTriggerEnter2D`), not at the start of the next tick; the pellet is held and rides along.
    #[test]
    fn a_mouth_grabs_what_it_drives_into_in_the_tick_it_meets_it() {
        let _ten = phys::ten_steps_for_this_test();
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut pellets = PelletStore::default();
        let mut eggs = EggStore::new();
        let mut rng = Rng::from_seed(9);
        let mut scratch = BibiteScratch::new();
        let holder = spawn_adult_at(&mut store, Vec2::ZERO);
        let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 60.0, Vec2::new(0.0, 14.0)));
        let mut events = Vec::new();
        let mut grabbed_at = None;
        for tick in 0..60 {
            let record = store.get_mut(holder).unwrap();
            record.mouth.desire_grab = 1.0;
            record.pending_force = Vec2::Y.rotate(record.rot) * 60.0;
            store.step_physics_with_executor(&mut pellets, &mut eggs, &settings, &mut rng, &mut scratch, &mut events, ParallelExecutor::Serial);
            if grabbed_at.is_none() && store.get(holder).unwrap().is_held(pellet_tag(pellet)) {
                grabbed_at = Some(tick);
                assert!(pellets.is_held(pellet), "the pellet is marked held");
            }
        }
        let record = store.get(holder).unwrap();
        assert!(grabbed_at.is_some(), "the pellet was never grabbed (holder at {:?}, pellet at {:?})", record.pos, pellets.pos(pellet));
        assert!(record.is_held(pellet_tag(pellet)), "still held at the end");
        let offset = (pellets.pos(pellet).unwrap() - record.pos).rotate(-record.rot);
        assert!(offset.length() < 20.0 && pellets.pos(pellet).unwrap().y > 14.0, "the pellet rides with the holder: {offset:?}");
    }

    /// A holder holding a pellet or a bibite stays where it is (it was pushed backwards by the held body's contact).
    #[test]
    fn a_holder_is_not_pushed_away_by_what_it_holds() {
        let _ten = phys::ten_steps_for_this_test();
        let settings = SimSettings::game_defaults();
        for scenario in ["pellet held", "bibite held", "on a pellet", "nothing"] {
            let mut store = BibiteStore::with_settings(settings.clone());
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(9);
            let mut scratch = BibiteScratch::new();
            let holder = spawn_adult_at(&mut store, Vec2::ZERO);
            match scenario {
                "pellet held" => {
                    let mouth = mouth_tip(store.get(holder).unwrap());
                    let pellet = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, mouth));
                    let record = store.get_mut(holder).unwrap();
                    record.held_targets[0] = pellet_tag(pellet);
                    record.n_held_targets = 1;
                }
                "bibite held" => {
                    let prey = spawn_adult_at(&mut store, Vec2::new(0.0, 10.6));
                    let record = store.get_mut(holder).unwrap();
                    record.held_targets[0] = bibite_tag(prey);
                    record.n_held_targets = 1;
                    record.mouth.n_held = 1;
                    record.mouth.desire_grab = 1.0;
                }
                "on a pellet" => {
                    pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 30.0, Vec2::ZERO));
                }
                _ => {}
            }
            let mut events = Vec::new();
            let mut line = String::new();
            for step in 0..40 {
                store.step_physics_with_executor(
                    &mut pellets,
                    &mut eggs,
                    &settings,
                    &mut rng,
                    &mut scratch,
                    &mut events,
                    ParallelExecutor::Serial,
                );
                if step % 8 == 0 {
                    let record = store.get(holder).unwrap();
                    line += &format!(" [{step}: pos {:.2},{:.2} vel {:.2},{:.2}]", record.pos.x, record.pos.y, record.vel.x, record.vel.y);
                }
            }
            println!("PROBE {scenario}:{line}");
            if scenario.contains("held") {
                // The game joins held things with a joint (the pair still collides): the holder is not pushed away by what it holds.
                let record = store.get(holder).unwrap();
                assert!(record.pos.length() < 0.5, "{scenario}: the holder drifted to {:?}", record.pos);
            }
        }
    }
}

#[cfg(test)]
mod perf_probe_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::rng::Rng;
    use crate::settings::SimSettings;
    use std::time::Instant;

    fn fixture(count: usize, half_extent: f32) -> (BibiteStore, SimSettings) {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut store = BibiteStore::with_settings(settings.clone());
        let mut rng = Rng::from_seed(0x5255_5354);
        for _ in 0..count {
            let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(
                rng.f32_range(-half_extent, half_extent),
                rng.f32_range(-half_extent, half_extent),
            );
            spawn.rot = rng.f32_range(0.0, std::f32::consts::TAU);
            store.spawn(spawn).unwrap();
        }
        (store, settings)
    }

    #[test]
    #[ignore = "manual performance probe"]
    fn perf_probe_clustered_and_spread() {
        for (label, count, half) in [
            ("spread2000", 2000usize, 1900.0f32),
            ("spread5000", 5000, 1900.0),
            ("cluster2000", 2000, 50.0),
            ("cluster4000", 4000, 50.0),
        ] {
            let (mut store, settings) = fixture(count, half);
            let mut pellets = PelletStore::default();
            let mut eggs = EggStore::new();
            let mut rng = Rng::from_seed(7);
            let mut scratch = BibiteScratch::new();
            let mut events = Vec::new();
            let start = Instant::now();
            store.step_physics(
                &mut pellets,
                &mut eggs,
                &settings,
                &mut rng,
                &mut scratch,
                &mut events,
            );
            let elapsed = start.elapsed();
            eprintln!(
                "PERF {label} bibites={} elapsed={:.3}ms pairs={} contacts={} triggers={}",
                store.len(),
                elapsed.as_secs_f64() * 1e3,
                scratch.physics.candidate_pair_count(),
                scratch.physics.contacts().len(),
                scratch.physics.triggers().len(),
            );
        }
    }
}

#[cfg(test)]
mod rot_units_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::settings::SimSettings;

    #[test]
    fn save_rotation_is_game_degrees_and_record_is_radians() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings);
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.rot = 0.0;
        let id = store.spawn(spawn).unwrap();
        let mut save = store.to_save(id).unwrap();
        save.rot = 90.0;
        save.pos = Vec2::new(5.0, 6.0);
        save.vel = Vec2::ZERO;
        save.ang_vel = 0.0;
        let loaded_id = store.spawn_loaded(save).unwrap();
        let record = store.get(loaded_id).unwrap();
        assert!((record.rot - core::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let round = store.to_save(loaded_id).unwrap();
        assert!((round.rot - 90.0).abs() < 1e-3);
    }
}

#[cfg(test)]
mod world_perf_probe_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::settings::SimSettings;
    use crate::world::{ScenarioData, World};

    #[test]
    #[ignore = "manual performance probe"]
    fn perf_probe_world_phase_table() {
        let n = std::env::var("PROBE_N")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2000usize);
        let settings = SimSettings::game_defaults();
        let scenario = ScenarioData::with_default_zone_group();
        let mut world = World::new(settings, scenario, 0x5255_5354).unwrap();
        let _ = world.seed_scenario_world();
        let genome = Genome::starter();
        let brain = Brain::starter();
        for index in 0..n {
            let mut rng = world
                .rng
                .fork(0xB1B1_7E50 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let min = world.bounds.min();
            let max = world.bounds.max();
            let margin = (max - min).length() * 0.05;
            let low = min + Vec2::splat(margin);
            let high = max - Vec2::splat(margin);
            let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
            spawn.pos = Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y));
            spawn.rot = rng.f32_range(0.0, std::f32::consts::TAU);
            spawn.vitals = SpawnVitals::Adult;
            let id = world.bibites.spawn(spawn).unwrap();
            world.register_bibite(id);
        }
        if let Ok(value) = std::env::var("BRAIN_THREADS") {
            if let Ok(threads) = value.parse() {
                world.bibites.set_brain_threads(threads);
            }
        }
        world.tick();
        world.tick();
        world.reset_phase_timings();
        let measured_ticks = 4u64;
        for _ in 0..measured_ticks {
            world.tick();
        }
        let names = [
            "clock",
            "animators",
            "tools",
            "zones",
            "sensing",
            "body",
            "eggs",
            "phero",
            "decay",
            "rad",
            "red",
            "killers",
            "producers",
            "deaths",
            "physics",
            "events",
            "spawner",
            "hashes",
        ];
        let nanos = world.phase_timings().nanos;
        let mut total = 0u64;
        let divisor = measured_ticks as f64;
        for (index, value) in nanos.iter().enumerate() {
            total += value;
            eprintln!(
                "PHASE n={n} {}: {:.3}ms",
                names.get(index).copied().unwrap_or("?"),
                *value as f64 / divisor / 1e6
            );
        }
        eprintln!("PHASE n={n} TOTAL: {:.3}ms", total as f64 / divisor / 1e6);
    }
}

#[cfg(test)]
mod parallel_brain_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::rng::Rng;
    use crate::settings::SimSettings;

    fn run(threads: usize) -> Vec<u64> {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut store = BibiteStore::with_settings(settings.clone());
        store.set_brain_threads(threads);
        let mut rng = Rng::from_seed(0x5255_5354);
        for _ in 0..5000 {
            let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(rng.f32_range(-900.0, 900.0), rng.f32_range(-900.0, 900.0));
            spawn.rot = rng.f32_range(0.0, std::f32::consts::TAU);
            store.spawn(spawn).unwrap();
        }
        let mut pellets = PelletStore::default();
        let mut pheromones = PheromoneStore::default();
        let mut eggs = EggStore::new();
        let zones: [ZoneCircle; 0] = [];
        let mut rng = Rng::from_seed(11);
        let mut scratch = BibiteScratch::new();
        let mut events = Vec::new();
        for _ in 0..2 {
            let mut ctx = BibiteTickCtx {
                pellets: &mut pellets,
                pheromones: &mut pheromones,
                eggs: &mut eggs,
                zones: &zones,
                settings: &settings,
                rng: &mut rng,
                scratch: &mut scratch,
                events: &mut events,
            };
            store.tick(&mut ctx);
        }
        let outputs = [
            Output::Accelerate,
            Output::Rotate,
            Output::Herding,
            Output::Digestion,
            Output::EggProduction,
            Output::Want2Attack,
            Output::Want2Eat,
            Output::Want2Grow,
            Output::Want2Heal,
            Output::Want2Lay,
            Output::Grab,
            Output::PhereOut1,
            Output::PhereOut2,
            Output::PhereOut3,
            Output::ClkReset,
        ];
        let mut state = Vec::new();
        for id in store.ids() {
            let record = store.get(id).unwrap();
            state.push(record.pos.x.to_bits() as u64);
            state.push(record.pos.y.to_bits() as u64);
            state.push(record.vel.x.to_bits() as u64);
            state.push(record.vel.y.to_bits() as u64);
            state.push(record.rot.to_bits() as u64);
            state.push(record.ang_vel.to_bits() as u64);
            state.push(record.body.energy.amount.to_bits() as u64);
            state.push(record.body.health.amount.to_bits() as u64);
            state.push(record.clock.time_alive.to_bits() as u64);
            state.push(record.stomach.total_amount().to_bits() as u64);
            for output in outputs {
                state.push(record.brain.output_value(output).to_bits() as u64);
            }
        }
        state
    }

    #[test]
    fn parallel_and_serial_brain_batches_are_bit_identical() {
        assert_eq!(run(1), run(8));
    }
}

#[cfg(test)]
mod b2a_tests {
    use super::*;
    use crate::brain::Brain;
    use crate::genome::Genome;
    use crate::organs::egg_required_energy;
    use crate::rng::Rng;
    use crate::settings::SimSettings;

    const TOL: f32 = 1e-4;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= TOL * expected.abs().max(1.0),
            "expected {expected}, got {actual}"
        );
    }

    fn starter_store() -> (BibiteStore, SimSettings, Genome, Brain) {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let brain = Brain::starter();
        (
            BibiteStore::with_settings(settings.clone()),
            settings,
            genome,
            brain,
        )
    }

    fn spawn_with(
        store: &mut BibiteStore,
        genome: &Genome,
        brain: &Brain,
        vitals: SpawnVitals,
        energy: Option<f32>,
    ) -> BibiteId {
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.vitals = vitals;
        spawn.energy = energy;
        store.spawn(spawn).expect("spawn")
    }

    #[test]
    fn spawn_vitals_match_starter_spec_numbers() {
        let (mut store, settings, genome, brain) = starter_store();
        let size_ratio = genome.gene(Gene::SizeRatio);
        let g_birth = genome.growth_at_birth();
        let g_mature = genome.growth_at_mature();
        // Adult.
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let record = store.get(id).unwrap();
        let d2 = g_mature * size_ratio * size_ratio;
        close(record.body.geometry.d2_size, d2);
        close(record.body.geometry.base_body_area, 80.0 * d2);
        close(record.body.geometry.max_health, 1.25 * 80.0 * d2);
        close(record.body.health.amount, record.body.geometry.max_health);
        close(record.body.energy.amount, record.body.geometry.max_energy);
        assert!(record.growth.mature);
        // Baby.
        let id = store
            .spawn(BibiteSpawn {
                vitals: SpawnVitals::Baby,
                ..BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone())
            })
            .unwrap();
        let record = store.get(id).unwrap();
        let d2 = g_birth * size_ratio * size_ratio;
        close(record.body.geometry.d2_size, d2);
        close(record.body.geometry.base_body_area, 80.0 * d2);
        assert!(!record.growth.mature);
        // Elder.
        let id = store
            .spawn(BibiteSpawn {
                vitals: SpawnVitals::Elder,
                ..BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone())
            })
            .unwrap();
        close(
            store.get(id).unwrap().body.geometry.d2_size,
            2.0 * g_mature * size_ratio * size_ratio,
        );
        // Egg hatch: energy starts at the egg investment and the birth build
        // clamps the remainder to the newborn's max energy.
        let lay_energy = genome.lay_body_energy(&settings);
        let id = spawn_with(
            &mut store,
            &genome,
            &brain,
            SpawnVitals::FromEgg,
            Some(lay_energy),
        );
        let record = store.get(id).unwrap();
        close(record.body.geometry.d2_size, genome.d2_size_at_birth());
        close(record.body.health.amount, record.body.geometry.max_health);
        close(record.body.energy.amount, record.body.geometry.max_energy);
        close(record.growth.growth, g_birth);
    }

    #[test]
    fn spawn_size_scales_with_size_ratio() {
        let settings = SimSettings::game_defaults();
        let mut store = BibiteStore::with_settings(settings.clone());
        let brain = Brain::starter();
        let starter = Genome::starter();
        let mut big_values = *starter.values();
        big_values[Gene::SizeRatio.index()] = 2.0;
        let big = Genome::from_values(big_values);
        let mut small_values = *starter.values();
        small_values[Gene::SizeRatio.index()] = 0.5;
        let small = Genome::from_values(small_values);
        let adult = spawn_with(&mut store, &starter, &brain, SpawnVitals::Adult, None);
        let big_id = spawn_with(&mut store, &big, &brain, SpawnVitals::Adult, None);
        let small_id = spawn_with(&mut store, &small, &brain, SpawnVitals::Adult, None);
        let base = store.get(adult).unwrap().body.geometry;
        let big_geom = store.get(big_id).unwrap().body.geometry;
        let small_geom = store.get(small_id).unwrap().body.geometry;
        close(big_geom.d2_size, base.d2_size * 4.0);
        close(big_geom.base_body_area, base.base_body_area * 4.0);
        close(small_geom.d2_size, base.d2_size * 0.25);
        close(small_geom.base_body_area, base.base_body_area * 0.25);
    }

    #[test]
    fn growth_refreshes_derived_rigid_body_cache() {
        let (mut store, settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Baby, None);
        let bs = BodySettings::from_sim_settings(&settings);
        let initial_d2 = store.get(id).unwrap().body.geometry.d2_size;
        for _ in 0..10_000 {
            let record = store.get_mut(id).unwrap();
            record.body.energy.amount = record.body.geometry.max_energy;
            let genes = record.growth_genes;
            let mut growth = record.growth.clone();
            if let Some(step) = growth.tick(0.1, 1.0, &genes) {
                record.growth = growth;
                record.refresh_organs(&settings);
                let _ = record.body.apply_growth_step(&step, 0.0, &bs);
                record.refresh_colliders();
                record.update_derived_rigid_body(&settings);
                if record.body.geometry.d2_size > initial_d2 * 1.01 {
                    break;
                }
            } else {
                record.growth = growth;
            }
        }
        let record = store.get(id).unwrap();
        assert!(record.body.geometry.d2_size > initial_d2 * 1.01);
        close(record.derived.d1_size, record.body.geometry.d1_size);
        close(record.derived.d2_size, record.body.geometry.d2_size);
        close(record.derived.body_length, record.body.geometry.body_length);
        let organ_masses = OrganMasses {
            stomach: record.stomach.mass(),
            egg_layer: settings.scenario.bibite_mass_density
                * record.egg_layer.egg_size
                * record.egg_layer.progress,
            fat: record.fat.mass(&organ_material(&settings.materials.fat)),
            armor: record.armor.mass(
                record.body.geometry.base_body_area * record.armor.portion,
                &organ_material(&settings.materials.armor),
            ),
        };
        let rb = record.body.rigid_body(organ_masses, &bs);
        close(record.derived.mass, rb.mass);
        close(record.derived.inertia, rb.inertia);
    }

    #[test]
    fn fat_inflate_resizes_tail_collider() {
        let (mut store, settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let bs = BodySettings::from_sim_settings(&settings);
        let record = store.get_mut(id).unwrap();
        let d1 = record.body.geometry.d1_size;
        let narrow = record.colliders()[1];
        record.body.set_inflate_factor(1.0, &bs);
        record.refresh_colliders();
        let wide = record.colliders()[1];
        let phys::Shape::Capsule {
            radius,
            half_height,
        } = wide.shape
        else {
            panic!("tail is not a capsule");
        };
        close(radius, 4.25 * d1);
        close(half_height, 1.0 * d1);
        close(wide.offset.y, -5.5 * d1);
        let phys::Shape::Capsule {
            radius: old_radius, ..
        } = narrow.shape
        else {
            panic!("old tail is not a capsule");
        };
        assert!(radius > old_radius);
    }

    #[test]
    fn derived_cache_matches_formulas() {
        let (mut store, settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let record = store.get(id).unwrap();
        let derived = record.derived;
        let geometry = record.body.geometry;
        close(derived.d1_size, geometry.d1_size);
        close(derived.d2_size, geometry.d2_size);
        close(derived.base_body_area, geometry.base_body_area);
        close(derived.body_length, 12.5 * geometry.d1_size);
        let base_mass = settings.scenario.bibite_mass_density * geometry.base_body_area;
        let armor_mass =
            geometry.base_body_area * record.armor.portion * settings.materials.armor.mass_density;
        close(derived.mass, base_mass + armor_mass);
        close(
            derived.inertia,
            derived.mass / 2.0 * geometry.real_body_area / PI,
        );
        close(
            derived.linear_damping,
            settings.scenario.drag_coefficient * geometry.d1_size / derived.mass,
        );
        close(derived.angular_damping, derived.linear_damping);
        let upkeep = brain.upkeep();
        close(
            derived.brain_upkeep,
            upkeep.synapses as f32 * settings.scenario.synapse_upkeep_cost
                + upkeep.hidden as f32 * settings.scenario.neuron_upkeep_cost,
        );
        assert!(record.is_ready && record.born);
    }

    #[test]
    fn ids_are_generational_and_dense_iteration_is_stable() {
        let (mut store, _settings, genome, brain) = starter_store();
        let first = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let second = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        assert_eq!(store.len(), 2);
        assert_eq!(store.ids().count(), 2);
        assert_eq!(store.iter().count(), 2);
        let first_slot = first.handle().slot();
        let _removed = store.remove(first).expect("remove");
        assert_eq!(store.len(), 1);
        assert!(store.get(first).is_none());
        let third = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        assert_eq!(third.handle().slot(), first_slot);
        assert_ne!(third.handle().generation(), first.handle().generation());
        let ids: Vec<BibiteId> = store.ids().collect();
        assert!(ids.contains(&second));
        assert!(ids.contains(&third));
        assert_eq!(store.slot_capacity(), 2);
    }

    /// The game SUBTRACTS the period from its counters (`brainTicksCount -= brainTickFactor`, `visionLookupCount -= visionLookupPeriod`, `visionSensingCount -= visionSensingPeriod`),
    /// so a counter above its period (the setting was lowered while the world ran) stays above it; measured against the original game (`sense_vs_game`, 2026-11-19: counters 2 and 4 with
    /// both periods at 1 stayed 2 and 4 for ever, and the port's reset to 0 gave 0).
    #[test]
    fn cadence_counters_subtract_the_period() {
        let (mut store, mut settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        settings.brain_tps = 2;
        settings.vision_lookup_updates = 1;
        settings.vision_sense_updates = 1;
        {
            let throttle = store.sense_throttle_mut(id).expect("throttle");
            throttle.brain_counter = 1;
            throttle.lookup_counter = 2;
            throttle.sense_counter = 4;
        }
        let record = store.get_mut(id).expect("record");
        record.born = true;
        record.is_ready = true;
        let (due, _) = BibiteStore::tick_brain_pre_record(record, &settings);
        assert!(due, "the brain counter reached its period");
        let throttle = store.sense_throttle(id).expect("throttle");
        assert_eq!((throttle.brain_counter, throttle.lookup_counter, throttle.sense_counter), (0, 2, 4));
        // below its period a counter does what it always did
        settings.vision_lookup_updates = 10;
        settings.vision_sense_updates = 5;
        {
            let throttle = store.sense_throttle_mut(id).expect("throttle");
            throttle.brain_counter = 1;
            throttle.lookup_counter = 9;
            throttle.sense_counter = 4;
        }
        let record = store.get_mut(id).expect("record");
        let (due, _) = BibiteStore::tick_brain_pre_record(record, &settings);
        assert!(due);
        let throttle = store.sense_throttle(id).expect("throttle");
        assert_eq!((throttle.brain_counter, throttle.lookup_counter, throttle.sense_counter), (0, 0, 0));
    }

    #[test]
    fn view_senses_throttle_and_mutation_hooks() {
        let (mut store, _settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let view = store.view(id).unwrap();
        assert_eq!(view.id, id);
        assert_eq!(view.color[0], genome.gene(Gene::ColorR));
        assert!((view.up().length() - 1.0).abs() < 1e-5);
        let senses = SenseOutputs::from_values([0.25; 21]);
        store.set_senses(id, senses);
        assert_eq!(store.senses(id), Some(&senses));
        if let Some(throttle) = store.sense_throttle_mut(id) {
            throttle.brain_counter = 7;
            throttle.lookup_counter = 3;
            throttle.sense_counter = 2;
        }
        assert_eq!(store.sense_throttle(id).unwrap().brain_counter, 7);
        store.add_mutation_factor(
            id,
            MutationFactor {
                probability_increase: 2.0,
                intensity_increase: 0.5,
            },
        );
        let record = store.get(id).unwrap();
        close(record.mutation_factor.probability_increase, 2.0);
        close(record.mutation_factor.intensity_increase, 0.5);
    }

    #[test]
    fn body_part_transform_matches_game_world_aabb_at_22_degrees() {
        let d1 = 1.147_573_5_f32;
        let collider = phys::Collider::solid(phys::Shape::capsule_from_size(
            phys::prefab::BODY_CAPSULE_SIZE * d1,
        ))
        .with_offset(phys::prefab::body_capsule_offset(d1))
        .with_rotation(phys::prefab::PART_ROTATION);
        let pose = phys::Pose::new(Vec2::ZERO, 22.223_844_f32.to_radians());
        let aabb = collider.aabb(pose);
        let size = aabb.max - aabb.min;
        // Game runtime census (physics2.jsonl) for 465701042:
        // Body world bounds 10.003 x 7.4859 at body rot 22.22 deg.
        assert!((size.x - 10.003).abs() < 0.15, "body AABB width {}", size.x);
        assert!((size.y - 7.486).abs() < 0.15, "body AABB height {}", size.y);
    }

    #[test]
    fn collider_descriptors_match_captured_prefab() {
        let (mut store, _settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let record = store.get(id).unwrap();
        let d1 = record.body.geometry.d1_size;
        let colliders = record.colliders();
        let phys::Shape::Capsule {
            half_height,
            radius,
        } = colliders[0].shape
        else {
            panic!("body collider is not a capsule");
        };
        close(radius, 2.5 * d1);
        close(half_height, 2.0 * d1);
        close(colliders[0].offset.x, 0.0);
        close(colliders[0].offset.y, -1.5 * d1);
        close(colliders[0].rotation, std::f32::consts::FRAC_PI_2);
        let phys::Shape::Capsule { radius, .. } = colliders[1].shape else {
            panic!("tail collider is not a capsule");
        };
        close(radius, 2.5 * d1);
        close(colliders[1].offset.y, -4.7 * d1);
        close(colliders[1].rotation, 0.0);
        assert!(colliders[2].is_sensor);
        close(colliders[2].offset.x, 0.0);
        close(colliders[2].offset.y, 2.38 * d1);
        close(colliders[2].rotation, std::f32::consts::FRAC_PI_2);
        let phys::Shape::Box { half_extents } = colliders[2].shape else {
            panic!("mouth collider is not a box");
        };
        close(half_extents.x, 1.36 * d1);
        let diet = record.genome.gene(crate::genome::Gene::Diet);
        close(
            half_extents.y,
            0.5 * phys::prefab::mouth_box_size(diet).y * d1,
        );
        assert_eq!(colliders[0].friction, phys::prefab::CORPS_FRICTION);
    }

    #[test]
    fn kill_queues_and_resolver_creates_corpse() {
        let (mut store, settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        store.kill(id, DeathCause::Hazard);
        let record = store.get(id).unwrap();
        assert!(record.body.dying);
        assert!(!record.body.dead);
        assert_eq!(record.death_cause, Some(DeathCause::Hazard));
        let mut pellets = PelletStore::default();
        let mut rng = Rng::from_seed(1);
        let mut events = Vec::new();
        store.resolve_pending_deaths(&mut pellets, &settings, &mut rng, &mut events);
        let record = store.get(id).unwrap();
        assert!(record.is_corpse);
        assert!(record.body.dead);
        assert_eq!(record.death_cause, None);
        assert!(events
            .iter()
            .any(|event| matches!(event, BibiteEvent::Death { bibite, cause } if *bibite == id && *cause == DeathCause::Hazard)));
    }

    #[test]
    fn summaries_follow_every_change_of_a_record() {
        let (mut store, settings, genome, brain) = starter_store();
        let a = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let b = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let c = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let (slot_a, slot_b, slot_c) = (
            store.slot_of(a).unwrap(),
            store.slot_of(b).unwrap(),
            store.slot_of(c).unwrap(),
        );
        let alive = SlotSummary::LIVE | SlotSummary::SENSIBLE;
        store.refresh_summaries();
        for slot in [slot_a, slot_b, slot_c] {
            assert_eq!(store.summaries[slot].bits & alive, alive);
            assert_eq!(store.summaries[slot].bits & SlotSummary::RAD, 0);
            assert_eq!(store.summaries[slot].bits & SlotSummary::HAS_SPECIES, 0);
        }

        // The species and the rad flag, changed through `get_mut`.
        {
            let record = store.get_mut(b).unwrap();
            record.meta.species = Some(crate::ids::SpeciesId::new(7));
            record.meta.is_rad = true;
        }
        store.refresh_summaries();
        assert_eq!(store.summaries[slot_b].species, 7);
        assert_ne!(store.summaries[slot_b].bits & SlotSummary::HAS_SPECIES, 0);
        assert_ne!(store.summaries[slot_b].bits & SlotSummary::RAD, 0);
        assert_eq!(store.summaries[slot_a].bits & SlotSummary::RAD, 0);

        // A death resolved by the shared path (a corpse, or a meat pile and an empty slot).
        store.kill(a, DeathCause::Hazard);
        let mut pellets = PelletStore::default();
        let mut rng = Rng::from_seed(1);
        let mut events = Vec::new();
        store.resolve_pending_deaths(&mut pellets, &settings, &mut rng, &mut events);
        store.refresh_summaries();
        assert_eq!(store.summaries[slot_a].bits & SlotSummary::LIVE, 0);
        assert_eq!(store.summaries[slot_a].bits & SlotSummary::SENSIBLE, 0);
        assert_eq!(store.summaries[slot_b].bits & alive, alive);

        // A slot freed and taken again by a newer bibite.
        store.remove(c).unwrap();
        store.refresh_summaries();
        assert_eq!(store.summaries[slot_c], SlotSummary::default());
        let d = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let slot_d = store.slot_of(d).unwrap();
        store.refresh_summaries();
        assert_eq!(store.summaries[slot_d].bits & alive, alive);

        // A brain that asks for pheromone sensing sets its bit; the starter brain does not.
        {
            use crate::brain::{FIRST_OUTPUT, Input, SynapseRecord};
            let nodes = brain.to_node_records();
            let mut synapses = brain.to_synapse_records();
            synapses.push(SynapseRecord {
                innovation: 10,
                node_in: Input::PheroSense1.index() as u32,
                node_out: (FIRST_OUTPUT + Output::Accelerate.index()) as u32,
                weight: 1.0,
                enabled: true,
            });
            let phero_brain =
                Brain::from_template_records(&nodes, &synapses, 0.5).expect("a valid brain");
            assert!(phero_brain.needs_pheromone_sensing());
            let sniffer = spawn_with(&mut store, &genome, &phero_brain, SpawnVitals::Adult, None);
            let slot_sniffer = store.slot_of(sniffer).unwrap();
            store.refresh_summaries();
            assert_ne!(store.summaries[slot_sniffer].bits & SlotSummary::PHERO, 0);
            assert_eq!(store.summaries[slot_b].bits & SlotSummary::PHERO, 0);
        }

        // `for_each_mut` hands out every record at once: the next refresh looks at all of them.
        store.for_each_mut(|_, record| record.meta.is_rad = true);
        store.refresh_summaries();
        for id in store.ids().collect::<Vec<_>>() {
            let slot = store.slot_of(id).unwrap();
            let rad = store.summaries[slot].bits & SlotSummary::RAD != 0;
            assert_eq!(rad, !store.get(id).unwrap().body.dead);
        }

        // A restored checkpoint rebuilds every record: the summaries are made again from the new ones.
        let checkpoint = store.capture_checkpoint();
        store.restore_checkpoint(checkpoint).unwrap();
        store.refresh_summaries();
        store.assert_summaries_current();
        assert_eq!(store.summaries[slot_b].species, 7);
        assert_ne!(store.summaries[slot_b].bits & SlotSummary::RAD, 0);

        // Many births at once grow the mirrors with the store.
        for _ in 0..40 {
            spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        }
        store.refresh_summaries();
        store.assert_summaries_current();
    }

    #[test]
    fn growth_at_spawn_mapping() {
        assert_eq!(
            SpawnVitals::from_growth_at_spawn(GrowthAtSpawn::Egg),
            SpawnVitals::FromEgg
        );
        assert_eq!(
            SpawnVitals::from_growth_at_spawn(GrowthAtSpawn::Baby),
            SpawnVitals::Baby
        );
        assert_eq!(
            SpawnVitals::from_growth_at_spawn(GrowthAtSpawn::Adult),
            SpawnVitals::Adult
        );
        assert_eq!(
            SpawnVitals::from_growth_at_spawn(GrowthAtSpawn::Elder),
            SpawnVitals::Elder
        );
    }

    fn spawn_egg(store: &mut EggStore, settings: &SimSettings, energy_scale: f32) -> EggId {
        let genome = Genome::starter();
        let brain = Brain::starter();
        let size_ratio = genome.gene(Gene::SizeRatio);
        let upkeep = brain_upkeep_per_second(&brain, settings);
        let total_required =
            egg_required_energy(genome.body_energy_at_birth(settings), upkeep, size_ratio);
        let hatch_time = genome.gene(Gene::HatchTime);
        store.spawn(EggSpawn {
            pos: Vec2::new(10.0, 20.0),
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome,
            meta: GenomeMeta::default(),
            brain,
            energy: total_required * energy_scale,
            hatch_time,
            total_required_energy: total_required,
            birth_orientation: -Vec2::Y,
        })
    }

    #[test]
    fn egg_store_hatches_after_hatch_time_in_dense_order() {
        let settings = SimSettings::game_defaults();
        let mut eggs = EggStore::new();
        let first = spawn_egg(&mut eggs, &settings, 4.0);
        let second = spawn_egg(&mut eggs, &settings, 4.0);
        assert_eq!(eggs.len(), 2);
        let mut out = Vec::new();
        eggs.tick(1.0, &settings, &mut out);
        assert!(out.is_empty());
        eggs.tick(100.0, &settings, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], first);
        assert_eq!(out[1], second);
        assert!(eggs.remove(first).is_some());
        assert_eq!(eggs.len(), 1);
    }

    #[test]
    fn egg_hatches_exactly_at_hatch_time_not_before() {
        let settings = SimSettings::game_defaults();
        let genome = Genome::starter();
        let hatch_time = genome.gene(Gene::HatchTime);
        let mut eggs = EggStore::new();
        let brain = Brain::starter();
        let total_required = egg_required_energy(
            genome.body_energy_at_birth(&settings),
            brain_upkeep_per_second(&brain, &settings),
            genome.gene(Gene::SizeRatio),
        );
        let id = eggs.spawn(EggSpawn {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome,
            meta: GenomeMeta::default(),
            brain,
            energy: total_required,
            hatch_time,
            total_required_energy: total_required,
            birth_orientation: -Vec2::Y,
        });
        let mut out = Vec::new();
        eggs.tick(hatch_time - 0.001, &settings, &mut out);
        assert!(out.is_empty());
        assert!(eggs.get(id).is_some());
        eggs.tick(0.002, &settings, &mut out);
        assert_eq!(out, vec![id]);
    }

    #[test]
    fn egg_with_insufficient_energy_aborts_and_queues_energy() {
        let settings = SimSettings::game_defaults();
        let mut eggs = EggStore::new();
        let id = spawn_egg(&mut eggs, &settings, 0.25);
        let mut out = Vec::new();
        eggs.tick(1.0, &settings, &mut out);
        assert!(out.is_empty());
        assert!(eggs.get(id).is_none());
        let aborted: Vec<(EggId, f32)> = eggs.drain_aborted().collect();
        assert_eq!(aborted.len(), 1);
        assert!(aborted[0].1 > 0.0);
    }

    #[test]
    fn egg_save_round_trip() {
        let settings = SimSettings::game_defaults();
        let mut eggs = EggStore::new();
        let id = spawn_egg(&mut eggs, &settings, 4.0);
        if let Some(egg) = eggs.get_mut(id) {
            egg.hatch.hatch_progress = 3.5;
        }
        let save = eggs.to_save(id).unwrap();
        let mut loaded = EggStore::new();
        let loaded_id = loaded.spawn_loaded(save);
        let record = loaded.get(loaded_id).unwrap();
        close(record.hatch.energy, eggs.get(id).unwrap().hatch.energy);
        close(record.hatch.hatch_progress, 3.5);
        close(
            record.hatch.hatch_time,
            eggs.get(id).unwrap().hatch.hatch_time,
        );
        close(record.size_gene, eggs.get(id).unwrap().size_gene);
        assert_eq!(
            record.genome.gene(Gene::SizeRatio),
            eggs.get(id).unwrap().genome.gene(Gene::SizeRatio)
        );
    }

    #[test]
    fn body_save_round_trip_preserves_serialized_fields() {
        let (mut store, settings, genome, brain) = starter_store();
        let id = spawn_with(&mut store, &genome, &brain, SpawnVitals::Adult, None);
        let plant = organ_material(&settings.materials.plant);
        let meat = organ_material(&settings.materials.meat);
        if let Some(record) = store.get_mut(id) {
            record.body.energy.amount = 42.0;
            record.body.health.amount = 77.0;
            record.mouth.bite_progress = 0.625;
            record.mouth.bibites_bitten = 4;
            record.egg_layer.progress = 1.5;
            record.egg_layer.n_laid = 3;
            record.fat.amount = 2.0;
            record.clock.tic = 1;
            record.clock.tic_progress = 0.25;
            record.clock.time_alive = 123.0;
            record.clock.chrono_time = 45.0;
            record.growth.growth = 1.2;
            record
                .stomach
                .add_matter(StomachMaterial::Plant, 5.0, &plant, &meat);
        }
        let save = store.to_save(id).unwrap();
        let mut loaded = BibiteStore::with_settings(settings);
        let loaded_id = loaded.spawn_loaded(save).unwrap();
        let a = store.get(id).unwrap();
        let b = loaded.get(loaded_id).unwrap();
        close(b.body.energy.amount, a.body.energy.amount);
        close(b.body.health.amount, a.body.health.amount);
        close(b.body.geometry.d2_size, a.body.geometry.d2_size);
        close(b.mouth.bite_progress, a.mouth.bite_progress);
        assert_eq!(b.mouth.bibites_bitten, 4);
        close(b.egg_layer.progress, 1.5);
        assert_eq!(b.egg_layer.n_laid, 3);
        close(b.fat.amount, 2.0);
        assert_eq!(b.clock.tic, 1);
        close(b.clock.time_alive, 123.0);
        close(b.stomach.total_amount(), 5.0);
        close(b.growth.growth, 1.2);
        assert_eq!(
            b.genome.gene(Gene::SizeRatio),
            a.genome.gene(Gene::SizeRatio)
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-4;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= TOL * expected.abs().max(1.0),
            "expected {expected}, got {actual}"
        );
    }

    /// Starter bibite parameters from `docs/spec/02` §3.6.
    fn starter_params() -> (BodyParams, BodySettings) {
        let settings = BodySettings::default();
        let params = BodyParams::from_genes(1.0, 180.0, 75.0, 0.066_844_9 / 13.101_6, &settings);
        (params, settings)
    }

    #[test]
    fn starter_body_energy_density_and_ratios() {
        let (params, settings) = starter_params();
        assert_close(params.base_energy_density, 2.697_65);
        assert_close(params.health_energy_ratio, 1.348_83);
        assert_close(params.sum_body_health_energy_ratio, 4.046_48);
        assert_close(params.metabolic_efficiency, 0.8);
        assert_eq!(
            params.health_material_density,
            settings.health_material_density
        );
    }

    #[test]
    fn starter_birth_and_mature_geometry() {
        let (params, settings) = starter_params();
        // growthAtBirth = (10/20)^2 = 0.25, womb portion = 2.5/13.1016.
        let g_birth = growth_at_birth(10.0, 20.0);
        assert_close(g_birth, 0.25);
        let womb_portion = 2.5 / 13.101_6;
        let g_mature = growth_at_mature(g_birth, womb_portion);
        assert_close(g_mature, 1.310_16);

        let birth = BodyGeometry::from_d2(0.25, 0.0, &settings);
        assert_close(birth.base_body_area, 20.0);
        assert_close(birth.body_length, 6.25);
        assert_close(birth.max_health, 25.0);
        assert_close(birth.max_energy, 30.0);
        assert_close(birth.base_mass, 0.5);

        let mature = BodyGeometry::from_d2(g_mature, 0.0, &settings);
        assert_close(mature.base_body_area, 104.812_8);
        assert_close(mature.body_length, 14.307_6);
        assert_close(mature.max_health, 131.016);
        assert_close(mature.max_energy, 157.219);
        assert_close(mature.base_mass, 2.620_32);
        // Inertia and damping at maturity with no organ mass.
        let body = BodyState::with_size(
            params,
            g_mature,
            mature.max_health,
            mature.max_energy,
            &settings,
        );
        let rb = body.rigid_body(OrganMasses::default(), &settings);
        assert_close(rb.mass, 2.620_32);
        assert_close(rb.inertia, 43.710_8);
        assert_close(rb.linear_damping, 2.184_13);
        assert_eq!(rb.angular_damping, rb.linear_damping);
    }

    #[test]
    fn starter_birth_costs_and_mature_growth_cost() {
        let (params, settings) = starter_params();
        // Newborn has 138.66 E from the egg; build cost = 20 u² * 2.69765 * 1.5.
        let mut body = BodyState::new(params, &settings);
        body.energy.amount = 138.662;
        let report = body.birth(0.25, &settings);
        assert_close(report.size_change.energy_cost, 80.929_6);
        // Cost paid without efficiency, then overflow above 30 E refunded.
        assert_close(body.energy.amount, 30.0);
        assert_close(body.health.amount, 25.0);
        assert_eq!(report.build_spend.health_damage, 0.0);

        // Grow from birth to maturity: cost = 84.8128 u² * 2.69765 * 1.5.
        let mut body = BodyState::with_size(params, 0.25, 25.0, 30.0, &settings);
        let change = body.set_2d_size(1.310_16, false, 0.0, &settings);
        assert_close(change.delta_area, 84.812_8);
        assert_close(change.energy_cost, 343.203_8);
    }

    #[test]
    fn capsule_geometry_matches_game() {
        let no_fat = CapsuleGeometry::at_inflate(0.0);
        assert_eq!(no_fat.size, Vec2::new(5.0, 7.0));
        assert_eq!(no_fat.offset, Vec2::new(0.0, -4.7));
        let full_fat = CapsuleGeometry::at_inflate(1.0);
        assert_eq!(full_fat.size, Vec2::new(8.5, 10.5));
        assert_eq!(full_fat.offset, Vec2::new(0.0, -5.5));
        let scaled = no_fat.scaled(2.0);
        assert_eq!(scaled.size, Vec2::new(10.0, 14.0));
        assert_eq!(scaled.offset, Vec2::new(0.0, -9.4));
    }

    #[test]
    fn inflate_changes_radii_and_area() {
        let settings = BodySettings::default();
        let mut geometry = BodyGeometry::from_d2(1.0, 0.0, &settings);
        let base_area = geometry.base_body_area;
        let base_radius = geometry.effective_radius;
        geometry.set_inflate(1.0, &settings);
        assert_close(geometry.real_body_area, base_area * 2.0);
        assert_close(
            geometry.effective_radius,
            base_radius * core::f32::consts::SQRT_2,
        );
    }

    #[test]
    fn use_energy_with_and_without_efficiency() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.0, 100.0, 50.0, &settings);
        let spend = body.use_energy(8.0, true);
        assert_close(spend.energy, 10.0); // 8 / 0.8
        assert_close(body.energy.amount, 40.0);
        let spend = body.use_energy(8.0, false);
        assert_close(spend.energy, 8.0);
        assert_close(body.energy.amount, 32.0);
        assert_eq!(spend.health_damage, 0.0);
    }

    #[test]
    fn starvation_pays_with_health_then_kills() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.0, 100.0, 1.0, &settings);
        let spend = body.use_energy(2.0, false); // 1 E left, then 1 E debt
        assert_close(spend.energy, 1.0);
        assert_close(spend.health_damage, 4.0); // Health density = 0.25 E/HP
        assert_close(body.health.amount, 96.0);
        assert!(!spend.died);

        let spend = body.use_energy(1000.0, false);
        assert!(spend.died);
        assert!(body.health.amount < 0.0);
    }

    #[test]
    fn gain_energy_semantics() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.0, 100.0, 100.0, &settings);
        body.energy.max_amount = 120.0;
        assert_close(body.gain_energy(30.0), 20.0); // only 20 available
        assert_close(body.energy.amount, 120.0);
        // Negative gain drains without touching the max.
        assert_close(body.gain_energy(-5.0), -5.0);
        assert_close(body.energy.amount, 115.0);
        // A gain that is NaN is no gain (it used to fill the bar: `available > NaN` is false).
        assert_eq!(body.gain_energy(f32::NAN), 0.0);
        assert_close(body.energy.amount, 115.0);
    }

    #[test]
    fn heal_costs_health_energy_ratio_with_double_penalty() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.310_16, 50.0, 100.0, &settings);
        // maxHP * healMaxRate * SpeedRatio = 131.016 * 0.0007633 * 1 = 0.1 HP/s.
        let report = body.heal(1.0, 10.0, &settings);
        assert_close(report.hp, 1.0);
        assert_close(report.energy_cost, 1.348_83);
        // Cost divided by MetabolicEfficiency 0.8.
        assert_close(report.spend.energy, 1.348_83 / 0.8);
        assert_close(body.health.amount, 51.0);
    }

    #[test]
    fn hurt_caps_damage_and_reports_death() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.0, 0.005, 10.0, &settings);
        let report = body.hurt(10.0);
        assert_close(report.dealt, 0.015); // health + 0.01
        assert!(report.died);
        assert!(!report.exploded);

        let mut corpse = BodyState::with_size(params, 1.0, 0.005, 0.0, &settings);
        corpse.dead = true;
        let report = corpse.hurt(1.0);
        assert!(report.exploded);
    }

    #[test]
    fn bite_damage_formula() {
        let settings = BodySettings::default();
        let damage = bite_damage(131.016, 3.0, 104.812_8, &settings);
        assert_close(damage, 18.75);
        assert_close(ongoing_bite_damage_scale(1.0 / 30.0, 1.5), 1.0 / 30.0 / 7.5);
    }

    #[test]
    fn armor_rules() {
        assert!(armor_blocks(100.0, 100.0));
        assert!(!armor_blocks(100.01, 100.0));
        assert_close(reduced_bite_force(100.0, 14.62), 85.38);
        assert_eq!(reduced_bite_force(10.0, 14.62), 0.0);
    }

    #[test]
    fn bite_damage_formula_invariants() {
        let base = BiteDamageInput {
            bite_strength: 100.0,
            armor_strength: 0.0,
            target_max_health: 100.0,
            target_real_body_area: 100.0,
            chunk_area: 10.0,
            ongoing: false,
            bite_period: 1.0,
            dt: 1.0 / 30.0,
            damage_factor: 0.1,
        };
        let hit = resolve_bite_damage(&base);
        assert!(!hit.armor_blocked);
        assert!((hit.damage - 1.0).abs() < 1e-5);
        let blocked = resolve_bite_damage(&BiteDamageInput {
            armor_strength: 100.0,
            ..base
        });
        assert!(blocked.armor_blocked);
        assert_eq!(blocked.damage, 0.0);
        let bigger_chunk = resolve_bite_damage(&BiteDamageInput {
            chunk_area: 20.0,
            ..base
        });
        assert!((bigger_chunk.damage - hit.damage * 2.0).abs() < 1e-5);
        let ongoing = resolve_bite_damage(&BiteDamageInput {
            ongoing: true,
            ..base
        });
        assert!(ongoing.damage > 0.0 && ongoing.damage < hit.damage);
    }

    #[test]
    fn collision_damage_formula_and_signs() {
        let settings = BodySettings::default();
        // The metric is the normal speed regardless of masses (receiver mass).
        let dmg = collision_damage(60.0, 1.0, &settings, CollisionSign::AsIs);
        assert_close(dmg, (60.0 - 50.0) * 0.1);
        assert_eq!(
            collision_damage(-60.0, 1.0, &settings, CollisionSign::AsIs),
            0.0
        );
        assert_close(
            collision_damage(-60.0, 1.0, &settings, CollisionSign::Absolute),
            1.0,
        );
        assert_close(
            collision_damage(-60.0, 1.0, &settings, CollisionSign::Flipped),
            1.0,
        );
    }

    #[test]
    fn attacked_damage_decays_and_feeds_brain_input() {
        let settings = BodySettings::default();
        let dt = 1.0 / 30.0;
        let decayed = attacked_dmg_decay(10.0, dt);
        assert!(decayed < 10.0 && decayed > 9.8);
        assert_eq!(attacked_dmg_decay(0.0, dt), 0.0);
        let settings2 = settings;
        assert_close(age_metabolism_penalty_factor(1.0, &settings2), 0.5);
        assert_close(age_strength_penalty_factor(1.0, &settings2), 0.75);
        assert_close(ageing_factor(2.0, settings.base_aging_threshold, 1.0), 1.0);
        assert_close(attacked_dmg_brain_input(3.0, 10.0), 0.6);
        assert_eq!(attacked_dmg_brain_input(3.0, 0.0), 0.0);
    }

    #[test]
    fn metabolism_and_healing_rates() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.310_16, 131.016, 157.219, &settings);
        body.update_metabolism(0.0, &settings);
        assert_close(body.metabolism, 0.104_812_8);
        // Ageing threshold = 1 / SpeedRatio = 1 h; at 2 h, factor = 1.
        body.update_metabolism(2.0, &settings);
        assert_close(body.metabolism, 0.104_812_8 * 1.5);
        assert_close(body.heal_max_rate(&settings), 0.001 / 1.310_16);
    }

    #[test]
    fn death_paths() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.0, 125.0, 40.0, &settings);
        // No corpses: total meat energy = sum ratio*area + energy.
        let extra = 7.0;
        let report = body.die(false, true, false, extra);
        assert_close(
            report.meat_energy.unwrap(),
            params.sum_body_health_energy_ratio * 80.0 + 40.0 + extra,
        );
        assert!(body.dead);

        let mut body = BodyState::with_size(params, 1.0, 125.0, 40.0, &settings);
        let report = body.die(true, true, true, extra);
        assert!(report.swallowed_and_destroyed);
        assert!(body.dead);
        assert_eq!(body.energy.amount, 40.0); // swallowed path does not zero energy

        let mut body = BodyState::with_size(params, 1.0, 125.0, 40.0, &settings);
        let report = body.die(true, true, false, extra);
        assert!(report.became_corpse);
        assert_eq!(body.energy.amount, 0.0);
        assert_close(body.health.amount, 100.0);
        // maxHP * 0.25 E/HP - 40 E = -15 (the corpse keeps 15 E of value).
        assert_close(body.corpse_energy_offset, 25.0 - 40.0);
    }

    #[test]
    fn corpse_decays_to_explosion() {
        let (params, settings) = starter_params();
        let mut body = BodyState::with_size(params, 1.0, 125.0, 40.0, &settings);
        body.die(true, true, false, 0.0);
        // Metabolism at d2 = 1 is 0.08 E/s; corpse burns it as health.
        body.update_metabolism(0.0, &settings);
        let tick = body.corpse_tick(1.0, &settings);
        assert!(!tick.exploded);
        // metabolism/0.8 of energy debt, /0.25 E per HP.
        assert_close(tick.health_damage, body.metabolism / 0.8 / 0.25);
        assert!(body.health.amount < 125.0);
        assert!(tick.corpse_decay >= 0.0 && tick.corpse_decay <= 1.0);
    }

    #[test]
    fn growth_state_reaches_maturity_and_costs_energy() {
        let (params, settings) = starter_params();
        let genes = GrowthGenes {
            growth_at_mature: 1.310_16,
            size_ratio: 1.0,
            growth_scale: 0.35,
            growth_maturity_factor: 2.0,
            growth_maturity_exponent: 1.0,
            metabolic_rate: 1.0,
            herd_velocity_weight: 0.0,
        };
        let mut growth = GrowthState::new(0.25, &genes);
        assert!(!growth.mature);
        let mut body = BodyState::with_size(params, 0.25, 25.0, 30.0, &settings);
        let mut steps = 0u32;
        for _ in 0..200_000 {
            let Some(step) = growth.tick(1.0 / 30.0, 1.0, &genes) else {
                continue;
            };
            steps += 1;
            let application = body.apply_growth_step(&step, 0.0, &settings);
            growth.record_cost_rate(application.growth_cost_rate);
            assert!(step.new_d2_size >= step.organ_base_body_area / AREA_PER_D2);
            if growth.mature {
                break;
            }
        }
        assert!(steps > 0);
        assert!(growth.mature, "growth did not reach maturity");
        assert!(growth.growth >= 1.310_16);
        assert!(body.geometry.base_body_area > 104.0);
        assert!(growth.growth_cost_rate >= 0.0);
    }

    #[test]
    fn growth_at_birth_and_mature_formulas() {
        assert_close(growth_at_birth(10.0, 20.0), 0.25);
        assert_close(growth_at_mature(0.25, 0.190_82), 1.310_16);
        assert_close(area_at_mature(1.310_16, 1.0), 104.812_8);
        assert_close(length_at_mature(1.310_16, 1.0), 14.307_6);
        assert_close(age_hours(3600.0), 1.0);
    }

    #[test]
    fn matter_pool_clamps() {
        let mut pool = MatterPool::new(150.0, 100.0, 1.0);
        assert_close(pool.overflow(), 50.0);
        assert_eq!(pool.available(), 0.0);
        assert!(pool.is_full());
        pool.amount = 50.0;
        assert_close(pool.fullness(), 0.5);
        assert_close(pool.available(), 50.0);
        assert!(!pool.is_empty());
        pool.amount = 0.0;
        assert!(pool.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Checkpoint byte codec (world snapshot extensions 0x0101 / 0x0102)
// ---------------------------------------------------------------------------

mod checkpoint_codec {
    use super::*;
    use crate::brain::{Brain, NodeRecord, NodeType, SynapseRecord};

    const MAX_RECORDS: u32 = 1_000_000;
    const MAX_STRING: u32 = 1 << 20;
    const MAX_NODES: u32 = 1 << 20;
    const MAX_SYNAPSES: u32 = 1 << 22;

    fn bad(message: &str) -> crate::error::Error {
        crate::error::Error::invalid_data(format!("checkpoint: {message}"))
    }

    struct CkptW {
        out: Vec<u8>,
    }

    impl CkptW {
        fn new() -> Self {
            Self { out: Vec::new() }
        }
        fn u8(&mut self, value: u8) {
            self.out.push(value);
        }
        fn u32(&mut self, value: u32) {
            self.out.extend_from_slice(&value.to_le_bytes());
        }
        fn i64(&mut self, value: i64) {
            self.out.extend_from_slice(&value.to_le_bytes());
        }
        fn u64(&mut self, value: u64) {
            self.out.extend_from_slice(&value.to_le_bytes());
        }
        fn f32(&mut self, value: f32) {
            self.u32(value.to_bits());
        }
        fn f64(&mut self, value: f64) {
            self.i64(value.to_bits() as i64);
        }
        fn bool(&mut self, value: bool) {
            self.u8(u8::from(value));
        }
        fn len(&mut self, len: usize) {
            self.u32(len as u32);
        }
        fn string(&mut self, value: &str) {
            self.len(value.len());
            self.out.extend_from_slice(value.as_bytes());
        }
        fn opt_string(&mut self, value: Option<&str>) {
            match value {
                Some(value) => {
                    self.u8(1);
                    self.string(value);
                }
                None => self.u8(0),
            }
        }
        fn vec2(&mut self, value: Vec2) {
            self.f32(value.x);
            self.f32(value.y);
        }
        fn id(&mut self, value: BibiteId) {
            let handle = value.handle();
            self.u32(handle.slot());
            self.u32(handle.generation());
        }
        fn opt_id(&mut self, value: Option<BibiteId>) {
            match value {
                Some(value) => {
                    self.u8(1);
                    self.id(value);
                }
                None => self.u8(0),
            }
        }
    }

    struct CkptR<'a> {
        data: &'a [u8],
        at: usize,
    }

    impl<'a> CkptR<'a> {
        fn new(data: &'a [u8]) -> Self {
            Self { data, at: 0 }
        }
        fn take(&mut self, len: usize) -> crate::error::Result<&'a [u8]> {
            let end = self.at.checked_add(len).ok_or_else(|| bad("truncated"))?;
            if end > self.data.len() {
                return Err(bad("truncated"));
            }
            let slice = &self.data[self.at..end];
            self.at = end;
            Ok(slice)
        }
        fn u8(&mut self) -> crate::error::Result<u8> {
            Ok(self.take(1)?[0])
        }
        fn u32(&mut self) -> crate::error::Result<u32> {
            let b = self.take(4)?;
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        fn i64(&mut self) -> crate::error::Result<i64> {
            let b = self.take(8)?;
            let mut v = [0u8; 8];
            v.copy_from_slice(b);
            Ok(i64::from_le_bytes(v))
        }
        fn u64(&mut self) -> crate::error::Result<u64> {
            let b = self.take(8)?;
            let mut v = [0u8; 8];
            v.copy_from_slice(b);
            Ok(u64::from_le_bytes(v))
        }
        fn f32(&mut self) -> crate::error::Result<f32> {
            Ok(f32::from_bits(self.u32()?))
        }
        fn f64(&mut self) -> crate::error::Result<f64> {
            Ok(f64::from_bits(self.i64()? as u64))
        }
        fn bool(&mut self) -> crate::error::Result<bool> {
            match self.u8()? {
                0 => Ok(false),
                1 => Ok(true),
                value => Err(bad(&format!("invalid bool {value} at {}", self.at))),
            }
        }
        fn len(&mut self, max: u32) -> crate::error::Result<usize> {
            let len = self.u32()?;
            if len > max {
                return Err(bad("length limit exceeded"));
            }
            Ok(len as usize)
        }
        fn string(&mut self) -> crate::error::Result<String> {
            let len = self.len(MAX_STRING)?;
            let bytes = self.take(len)?;
            String::from_utf8(bytes.to_vec()).map_err(|_| bad("invalid UTF-8"))
        }
        fn opt_string(&mut self) -> crate::error::Result<Option<String>> {
            match self.u8()? {
                0 => Ok(None),
                1 => Ok(Some(self.string()?)),
                _ => Err(bad("invalid option tag")),
            }
        }
        fn vec2(&mut self) -> crate::error::Result<Vec2> {
            Ok(Vec2::new(self.f32()?, self.f32()?))
        }
        fn id(&mut self) -> crate::error::Result<BibiteId> {
            Ok(BibiteId::from_handle(Handle::new(self.u32()?, self.u32()?)))
        }
        fn opt_id(&mut self) -> crate::error::Result<Option<BibiteId>> {
            match self.u8()? {
                0 => Ok(None),
                1 => Ok(Some(self.id()?)),
                _ => Err(bad("invalid option tag")),
            }
        }
        fn finish(&self) -> crate::error::Result<()> {
            if self.at != self.data.len() {
                return Err(bad("trailing bytes"));
            }
            Ok(())
        }
    }

    fn write_meta(w: &mut CkptW, meta: &GenomeMeta) {
        w.u32(meta.generation);
        match meta.species {
            Some(species) => {
                w.u8(1);
                w.u32(species.value());
            }
            None => w.u8(0),
        }
        w.opt_string(meta.species_tag.as_deref());
        w.bool(meta.is_mutant);
        w.bool(meta.is_rad);
        w.bool(meta.is_rain);
        w.bool(meta.is_killer);
        w.bool(meta.is_source);
        w.bool(meta.deux_parent);
        w.opt_id(meta.parent1);
        w.opt_id(meta.parent2);
    }

    fn read_meta(r: &mut CkptR<'_>) -> crate::error::Result<GenomeMeta> {
        let generation = r.u32()?;
        let species = match r.u8()? {
            0 => None,
            1 => Some(crate::ids::SpeciesId::new(r.u32()?)),
            _ => return Err(bad("invalid species tag")),
        };
        Ok(GenomeMeta {
            generation,
            species,
            species_tag: r.opt_string()?,
            is_mutant: r.bool()?,
            is_rad: r.bool()?,
            is_rain: r.bool()?,
            is_killer: r.bool()?,
            is_source: r.bool()?,
            deux_parent: r.bool()?,
            parent1: r.opt_id()?,
            parent2: r.opt_id()?,
        })
    }

    fn write_brain(w: &mut CkptW, brain: &Brain) {
        let nodes = brain.to_node_records();
        w.len(nodes.len());
        for node in &nodes {
            w.u8(node.node_type as u8);
            w.u32(node.index);
            w.i64(node.innovation);
            w.opt_string(node.desc.as_deref());
            w.f32(node.base_activation);
            w.f32(node.value);
            w.f32(node.last_input);
            w.f32(node.last_output);
        }
        let synapses = brain.to_synapse_records();
        w.len(synapses.len());
        for synapse in &synapses {
            w.i64(synapse.innovation);
            w.u32(synapse.node_in);
            w.u32(synapse.node_out);
            w.f32(synapse.weight);
            w.bool(synapse.enabled);
        }
        w.i64(brain.node_max_innovation());
        w.i64(brain.connection_max_innovation());
        w.bool(brain.herding_enabled());
    }

    fn read_brain(r: &mut CkptR<'_>) -> crate::error::Result<Brain> {
        let node_count = r.len(MAX_NODES)?;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let node_type = NodeType::from_u8(r.u8()?).ok_or_else(|| bad("invalid node type"))?;
            nodes.push(NodeRecord {
                node_type,
                index: r.u32()?,
                innovation: r.i64()?,
                desc: r.opt_string()?,
                base_activation: r.f32()?,
                value: r.f32()?,
                last_input: r.f32()?,
                last_output: r.f32()?,
            });
        }
        let synapse_count = r.len(MAX_SYNAPSES)?;
        let mut synapses = Vec::with_capacity(synapse_count);
        for _ in 0..synapse_count {
            synapses.push(SynapseRecord {
                innovation: r.i64()?,
                node_in: r.u32()?,
                node_out: r.u32()?,
                weight: r.f32()?,
                enabled: r.bool()?,
            });
        }
        let node_max = r.i64()?;
        let connection_max = r.i64()?;
        let herding_enabled = r.bool()?;
        let mut brain = Brain::from_records(&nodes, &synapses)
            .map_err(|error| bad(&format!("invalid brain: {error}")))?;
        brain.set_innovations(node_max, connection_max);
        brain.set_herding_enabled(herding_enabled);
        Ok(brain)
    }

    fn write_bibite_save(w: &mut CkptW, save: &BibiteSave) {
        w.vec2(save.pos);
        w.vec2(save.vel);
        w.f32(save.rot);
        w.f32(save.ang_vel);
        for gene in save.genome.genes {
            w.f32(gene);
        }
        write_meta(w, &save.meta);
        write_brain(w, &save.brain);
        for value in [
            save.energy,
            save.health,
            save.d2_size,
            save.attacked_dmg,
            save.total_damage_suffered,
            save.brain_ticks,
            save.vision_lookup,
            save.vision_sensing,
            save.corpse_energy_offset,
            save.growth,
            save.bite_progress,
            save.total_damage_dealt,
            save.murdered_area,
            save.egg_progress,
            save.fat_amount,
            save.phero_progress,
            save.tic_progress,
            save.time_alive,
            save.chrono_time,
            save.total_travel,
        ] {
            w.f32(value);
        }
        w.bool(save.dying);
        w.bool(save.dead);
        w.u32(save.times_attacked);
        for content in &save.stomach {
            w.u8(content.material);
            w.f32(content.amount);
            w.f32(content.average_chunk_amount);
        }
        w.u8(save.n_stomach);
        w.bool(save.attacked_last_frame);
        w.u32(save.bibites_bitten);
        w.u32(save.total_murders);
        w.u32(save.n_eggs_laid);
        w.len(save.children.len());
        for child in &save.children {
            w.id(*child);
        }
        w.u8(save.tic);
    }

    fn read_bibite_save(r: &mut CkptR<'_>) -> crate::error::Result<BibiteSave> {
        let pos = r.vec2()?;
        let vel = r.vec2()?;
        let rot = r.f32()?;
        let ang_vel = r.f32()?;
        let mut genes = [0.0f32; crate::genome::N_GENES];
        for gene in &mut genes {
            *gene = r.f32()?;
        }
        let genome = crate::genome::Genome::from_values(genes);
        let meta = read_meta(r)?;
        let brain = read_brain(r)?;
        let mut floats = [0.0f32; 20];
        for value in &mut floats {
            *value = r.f32()?;
        }
        let dying = r.bool()?;
        let dead = r.bool()?;
        let times_attacked = r.u32()?;
        let mut stomach = [StomachContentSave {
            material: 0,
            amount: 0.0,
            average_chunk_amount: 0.0,
        }; STOMACH_SLOTS];
        for content in &mut stomach {
            content.material = r.u8()?;
            content.amount = r.f32()?;
            content.average_chunk_amount = r.f32()?;
        }
        let n_stomach = r.u8()?;
        if n_stomach as usize > STOMACH_SLOTS {
            return Err(bad("too many stomach slots"));
        }
        let attacked_last_frame = r.bool()?;
        let bibites_bitten = r.u32()?;
        let total_murders = r.u32()?;
        let n_eggs_laid = r.u32()?;
        let child_count = r.len(MAX_RECORDS)?;
        let mut children = Vec::with_capacity(child_count);
        for _ in 0..child_count {
            children.push(r.id()?);
        }
        let tic = r.u8()?;
        Ok(BibiteSave {
            pos,
            vel,
            rot,
            ang_vel,
            genome,
            meta,
            brain,
            energy: floats[0],
            health: floats[1],
            d2_size: floats[2],
            attacked_dmg: floats[3],
            total_damage_suffered: floats[4],
            brain_ticks: floats[5],
            vision_lookup: floats[6],
            vision_sensing: floats[7],
            corpse_energy_offset: floats[8],
            growth: floats[9],
            bite_progress: floats[10],
            total_damage_dealt: floats[11],
            murdered_area: floats[12],
            egg_progress: floats[13],
            fat_amount: floats[14],
            phero_progress: floats[15],
            tic_progress: floats[16],
            time_alive: floats[17],
            chrono_time: floats[18],
            total_travel: floats[19],
            dying,
            dead,
            times_attacked,
            stomach,
            n_stomach,
            attacked_last_frame,
            bibites_bitten,
            total_murders,
            n_eggs_laid,
            children,
            tic,
        })
    }

    fn write_geometry(w: &mut CkptW, geometry: &BodyGeometry) {
        for value in [
            geometry.d2_size,
            geometry.d1_size,
            geometry.base_body_area,
            geometry.body_length,
            geometry.inflate_factor,
            geometry.real_body_area,
            geometry.effective_radius,
            geometry.armor_thickness,
            geometry.internals_radius,
            geometry.base_mass,
            geometry.max_health,
            geometry.max_energy,
        ] {
            w.f32(value);
        }
    }

    fn read_geometry(r: &mut CkptR<'_>) -> crate::error::Result<BodyGeometry> {
        Ok(BodyGeometry {
            d2_size: r.f32()?,
            d1_size: r.f32()?,
            base_body_area: r.f32()?,
            body_length: r.f32()?,
            inflate_factor: r.f32()?,
            real_body_area: r.f32()?,
            effective_radius: r.f32()?,
            armor_thickness: r.f32()?,
            internals_radius: r.f32()?,
            base_mass: r.f32()?,
            max_health: r.f32()?,
            max_energy: r.f32()?,
        })
    }

    fn write_params(w: &mut CkptW, params: &BodyParams) {
        for value in [
            params.metabolic_rate,
            params.metabolic_efficiency,
            params.base_energy_density,
            params.health_energy_ratio,
            params.sum_body_health_energy_ratio,
            params.health_material_density,
        ] {
            w.f32(value);
        }
    }

    fn read_params(r: &mut CkptR<'_>) -> crate::error::Result<BodyParams> {
        Ok(BodyParams {
            metabolic_rate: r.f32()?,
            metabolic_efficiency: r.f32()?,
            base_energy_density: r.f32()?,
            health_energy_ratio: r.f32()?,
            sum_body_health_energy_ratio: r.f32()?,
            health_material_density: r.f32()?,
        })
    }

    fn write_derived(w: &mut CkptW, derived: &BibiteDerived) {
        for value in [
            derived.mass,
            derived.inertia,
            derived.linear_damping,
            derived.angular_damping,
            derived.d1_size,
            derived.d2_size,
            derived.base_body_area,
            derived.body_length,
            derived.brain_upkeep,
        ] {
            w.f32(value);
        }
    }

    fn read_derived(r: &mut CkptR<'_>) -> crate::error::Result<BibiteDerived> {
        Ok(BibiteDerived {
            mass: r.f32()?,
            inertia: r.f32()?,
            linear_damping: r.f32()?,
            angular_damping: r.f32()?,
            d1_size: r.f32()?,
            d2_size: r.f32()?,
            base_body_area: r.f32()?,
            body_length: r.f32()?,
            brain_upkeep: r.f32()?,
        })
    }

    fn write_collider(w: &mut CkptW, collider: &crate::physics::Collider) {
        match collider.shape {
            crate::physics::Shape::Circle { radius } => {
                w.u8(0);
                w.f32(radius);
            }
            crate::physics::Shape::Capsule {
                half_height,
                radius,
            } => {
                w.u8(1);
                w.f32(half_height);
                w.f32(radius);
            }
            crate::physics::Shape::Box { half_extents } => {
                w.u8(2);
                w.vec2(half_extents);
            }
        }
        w.vec2(collider.offset);
        w.f32(collider.rotation);
        w.bool(collider.is_sensor);
        w.f32(collider.friction);
        w.f32(collider.restitution);
    }

    fn read_collider(r: &mut CkptR<'_>) -> crate::error::Result<crate::physics::Collider> {
        let shape = match r.u8()? {
            0 => crate::physics::Shape::Circle { radius: r.f32()? },
            1 => crate::physics::Shape::Capsule {
                half_height: r.f32()?,
                radius: r.f32()?,
            },
            2 => crate::physics::Shape::Box {
                half_extents: r.vec2()?,
            },
            _ => return Err(bad("invalid collider shape")),
        };
        Ok(crate::physics::Collider {
            shape,
            offset: r.vec2()?,
            rotation: r.f32()?,
            is_sensor: r.bool()?,
            friction: r.f32()?,
            restitution: r.f32()?,
        })
    }

    fn write_growth(w: &mut CkptW, growth: &GrowthState) {
        for value in [
            growth.growth,
            growth.applied_growth,
            growth.progress,
            growth.update_period,
            growth.growth_cost_rate,
        ] {
            w.f32(value);
        }
        w.bool(growth.mature);
        w.bool(growth.growing);
    }

    fn read_growth(r: &mut CkptR<'_>) -> crate::error::Result<GrowthState> {
        Ok(GrowthState {
            growth: r.f32()?,
            applied_growth: r.f32()?,
            progress: r.f32()?,
            update_period: r.f32()?,
            growth_cost_rate: r.f32()?,
            mature: r.bool()?,
            growing: r.bool()?,
        })
    }

    fn write_clock(w: &mut CkptW, clock: &organs::InternalClock) {
        w.u8(clock.tic);
        for value in [
            clock.tic_progress,
            clock.time_alive,
            clock.chrono_time,
            clock.period,
            clock.age_progress,
        ] {
            w.f32(value);
        }
        w.bool(clock.counting);
        w.bool(clock.day_triggered);
    }

    fn read_clock(r: &mut CkptR<'_>) -> crate::error::Result<organs::InternalClock> {
        Ok(organs::InternalClock {
            tic: r.u8()?,
            tic_progress: r.f32()?,
            time_alive: r.f32()?,
            chrono_time: r.f32()?,
            period: r.f32()?,
            age_progress: r.f32()?,
            counting: r.bool()?,
            day_triggered: r.bool()?,
        })
    }

    fn write_fat(w: &mut CkptW, fat: &organs::FatOrgan) {
        for value in [
            fat.amount,
            fat.max_amount,
            fat.area,
            fat.base_capacity,
            fat.max_inflate_amount,
            fat.threshold,
            fat.deadband,
            fat.inflate_factor,
            fat.last_inflate_factor,
            fat.last_activation,
            fat.last_amount_delta,
            fat.last_amount_rate,
            fat.last_affinity,
            fat.last_energy_gain,
            fat.last_energy_gain_rate,
            fat.last_energy_sustain_rate,
        ] {
            w.f32(value);
        }
    }

    fn read_fat(r: &mut CkptR<'_>) -> crate::error::Result<organs::FatOrgan> {
        Ok(organs::FatOrgan {
            amount: r.f32()?,
            max_amount: r.f32()?,
            area: r.f32()?,
            base_capacity: r.f32()?,
            max_inflate_amount: r.f32()?,
            threshold: r.f32()?,
            deadband: r.f32()?,
            inflate_factor: r.f32()?,
            last_inflate_factor: r.f32()?,
            last_activation: r.f32()?,
            last_amount_delta: r.f32()?,
            last_amount_rate: r.f32()?,
            last_affinity: r.f32()?,
            last_energy_gain: r.f32()?,
            last_energy_gain_rate: r.f32()?,
            last_energy_sustain_rate: r.f32()?,
        })
    }

    fn write_armor(w: &mut CkptW, armor: &organs::ArmorOrgan) {
        w.f32(armor.portion);
        w.f32(armor.thickness);
        w.f32(armor.strength);
    }

    fn read_armor(r: &mut CkptR<'_>) -> crate::error::Result<organs::ArmorOrgan> {
        Ok(organs::ArmorOrgan {
            portion: r.f32()?,
            thickness: r.f32()?,
            strength: r.f32()?,
        })
    }

    fn write_egg_layer(w: &mut CkptW, layer: &organs::EggLayer) {
        w.bool(layer.is_mature);
        w.u32(layer.capacity);
        w.u32(layer.n_laid);
        for value in [
            layer.progress,
            layer.egg_size,
            layer.egg_energy,
            layer.laying_period,
            layer.area,
            layer.progress_rate,
            layer.energy_consumption,
        ] {
            w.f32(value);
        }
    }

    fn read_egg_layer(r: &mut CkptR<'_>) -> crate::error::Result<organs::EggLayer> {
        Ok(organs::EggLayer {
            is_mature: r.bool()?,
            capacity: r.u32()?,
            n_laid: r.u32()?,
            progress: r.f32()?,
            egg_size: r.f32()?,
            egg_energy: r.f32()?,
            laying_period: r.f32()?,
            area: r.f32()?,
            progress_rate: r.f32()?,
            energy_consumption: r.f32()?,
        })
    }

    fn write_stomach_scalars(w: &mut CkptW, record: &BibiteCheckpointRecord) {
        w.len(record.stomach_contents.len());
        for content in &record.stomach_contents {
            w.u8(match content.material {
                StomachMaterial::Plant => 0,
                StomachMaterial::Meat => 1,
            });
            w.f32(content.amount);
            w.f32(content.average_chunk_amount);
            w.f32(content.efficiency);
            w.f32(content.digestion_rate);
            w.f32(content.last_conversion_efficiency);
        }
        for value in [
            record.stomach_area,
            record.stomach_total_content,
            record.stomach_acid_level,
            record.stomach_acid_ratio,
            record.stomach_digestion_effectiveness,
            record.stomach_digestion_potential_bonus,
            record.stomach_efficiency_malus,
        ] {
            w.f32(value);
        }
    }

    fn write_mouth(w: &mut CkptW, mouth: &organs::MouthState) {
        w.f32(mouth.bite_progress);
        w.bool(mouth.attacked_last_frame);
        w.bool(mouth.attacked_this_frame);
        w.u32(mouth.bibites_bitten);
        w.f32(mouth.total_damage_dealt);
        w.u32(mouth.total_murders);
        for value in [
            mouth.murdered_area,
            mouth.desire_attack,
            mouth.desire_eat,
            mouth.desire_grab,
            mouth.mouth_opening,
            mouth.bite_opening,
            mouth.bite_strength,
        ] {
            w.f32(value);
        }
        w.bool(mouth.can_swallow);
        w.len(mouth.n_in_mouth);
        w.len(mouth.n_held);
    }

    fn read_mouth(r: &mut CkptR<'_>) -> crate::error::Result<organs::MouthState> {
        Ok(organs::MouthState {
            bite_progress: r.f32()?,
            attacked_last_frame: r.bool()?,
            attacked_this_frame: r.bool()?,
            bibites_bitten: r.u32()?,
            total_damage_dealt: r.f32()?,
            total_murders: r.u32()?,
            murdered_area: r.f32()?,
            desire_attack: r.f32()?,
            desire_eat: r.f32()?,
            desire_grab: r.f32()?,
            mouth_opening: r.f32()?,
            bite_opening: r.f32()?,
            bite_strength: r.f32()?,
            can_swallow: r.bool()?,
            n_in_mouth: r.len(MAX_RECORDS)?,
            n_held: r.len(MAX_RECORDS)?,
        })
    }

    fn write_pool(w: &mut CkptW, pool: &MatterPool) {
        w.f32(pool.amount);
        w.f32(pool.max_amount);
        w.f32(pool.density);
    }

    fn read_pool(r: &mut CkptR<'_>) -> crate::error::Result<MatterPool> {
        Ok(MatterPool {
            amount: r.f32()?,
            max_amount: r.f32()?,
            density: r.f32()?,
        })
    }

    fn write_body(w: &mut CkptW, body: &BodyState) {
        write_geometry(w, &body.geometry);
        write_params(w, &body.params);
        write_pool(w, &body.energy);
        write_pool(w, &body.health);
        w.f32(body.metabolism);
        w.f32(body.healing_rate_energy_cost);
        w.f32(body.movement_cost);
        w.f32(body.attacked_dmg);
        w.u32(body.times_attacked);
        w.f32(body.total_damage_suffered);
        w.bool(body.dying);
        w.bool(body.dead);
        w.f32(body.corpse_energy_offset);
        w.f32(body.corpse_decay);
        w.f64(body.energy_gained_total);
        w.f64(body.energy_spent_total);
    }

    fn read_body(r: &mut CkptR<'_>) -> crate::error::Result<BodyState> {
        Ok(BodyState {
            geometry: read_geometry(r)?,
            params: read_params(r)?,
            energy: read_pool(r)?,
            health: read_pool(r)?,
            metabolism: r.f32()?,
            healing_rate_energy_cost: r.f32()?,
            movement_cost: r.f32()?,
            attacked_dmg: r.f32()?,
            times_attacked: r.u32()?,
            total_damage_suffered: r.f32()?,
            dying: r.bool()?,
            dead: r.bool()?,
            death_hurts: 0,
            corpse_energy_offset: r.f32()?,
            corpse_decay: r.f32()?,
            energy_gained_total: r.f64()?,
            energy_spent_total: r.f64()?,
        })
    }

    fn write_feeding(w: &mut CkptW, feeding: &FeedingCounters) {
        for value in [
            feeding.plant_bite_events,
            feeding.meat_bite_events,
            feeding.bibite_bite_events,
            feeding.plant_swallow_events,
            feeding.meat_swallow_events,
            feeding.bibite_swallow_events,
        ] {
            w.u64(value);
        }
        for value in [
            feeding.plant_consumed_u2,
            feeding.meat_consumed_u2,
            feeding.bibite_consumed_u2,
            feeding.plant_ripped_u2,
            feeding.meat_ripped_u2,
            feeding.bibite_damage_dealt,
            feeding.stomach_digested_u2,
            feeding.stomach_energy_gain_e,
            feeding.energy_gained_e,
            feeding.energy_spent_e,
            feeding.live_seconds,
        ] {
            w.f64(value);
        }
        for value in [
            feeding.bibite_kills,
            feeding.mouth_contact_ticks,
            feeding.plant_contact_ticks,
            feeding.mouth_contact_enter_events,
        ] {
            w.u64(value);
        }
        w.bool(feeding.contact_latched);
    }

    fn read_feeding(r: &mut CkptR<'_>) -> crate::error::Result<FeedingCounters> {
        let mut u64s = [0u64; 6];
        for value in &mut u64s {
            *value = r.u64()?;
        }
        let mut f64s = [0.0f64; 11];
        for value in &mut f64s {
            *value = r.f64()?;
        }
        let mut extra = [0u64; 4];
        for value in &mut extra {
            *value = r.u64()?;
        }
        Ok(FeedingCounters {
            plant_bite_events: u64s[0],
            meat_bite_events: u64s[1],
            bibite_bite_events: u64s[2],
            plant_swallow_events: u64s[3],
            meat_swallow_events: u64s[4],
            bibite_swallow_events: u64s[5],
            plant_consumed_u2: f64s[0],
            meat_consumed_u2: f64s[1],
            bibite_consumed_u2: f64s[2],
            plant_ripped_u2: f64s[3],
            meat_ripped_u2: f64s[4],
            bibite_damage_dealt: f64s[5],
            stomach_digested_u2: f64s[6],
            stomach_energy_gain_e: f64s[7],
            energy_gained_e: f64s[8],
            energy_spent_e: f64s[9],
            live_seconds: f64s[10],
            bibite_kills: extra[0],
            mouth_contact_ticks: extra[1],
            plant_contact_ticks: extra[2],
            mouth_contact_enter_events: extra[3],
            contact_latched: r.bool()?,
        })
    }

    fn write_bibite_runtime(w: &mut CkptW, record: &BibiteCheckpointRecord) {
        write_bibite_save(w, &record.save);
        w.f32(record.rot_radians);
        w.u32(record.throttle.brain_counter);
        w.u32(record.throttle.lookup_counter);
        w.u32(record.throttle.sense_counter);
        for value in record.senses.values() {
            w.f32(value);
        }
        w.bool(record.senses.has_herd);
        w.f32(record.senses.herd_sum_direction);
        w.f32(record.senses.herd_separation_projection);
        w.bool(record.steering.has_herd);
        w.f32(record.steering.herd_sum_direction_degrees);
        w.f32(record.steering.herd_separation_projection);
        w.vec2(record.steering.herd_velocity_match);
        w.f32(record.steering.nearest_separation_distance);
        w.vec2(record.steering.void_avoidance);
        w.len(record.fov_seen.pellets.len());
        for id in &record.fov_seen.pellets {
            let handle = id.handle();
            w.u32(handle.slot());
            w.u32(handle.generation());
        }
        w.len(record.fov_seen.bibites.len());
        for id in &record.fov_seen.bibites {
            let handle = id.handle();
            w.u32(handle.slot());
            w.u32(handle.generation());
        }
        w.vec2(record.pending_force);
        w.f32(record.pending_torque);
        for target in record.mouth_targets {
            w.u32((target >> 32) as u32);
            w.u32(target as u32);
        }
        w.u8(record.n_mouth_targets);
        for age in record.mouth_target_ages {
            w.u8(age);
        }
        for target in record.held_targets {
            w.u32((target >> 32) as u32);
            w.u32(target as u32);
        }
        w.u8(record.n_held_targets);
        match record.death_cause {
            Some(cause) => {
                w.u8(1);
                w.u8(death_cause_code(cause));
            }
            None => w.u8(0),
        }
        w.bool(record.is_corpse);
        w.bool(record.born);
        w.bool(record.is_ready);
        w.f32(record.mutation_factor.probability_increase);
        w.f32(record.mutation_factor.intensity_increase);
        w.i64(record.brain_node_max_innovation);
        w.i64(record.brain_connection_max_innovation);
        write_geometry(w, &record.body_geometry);
        write_params(w, &record.body_params);
        w.f32(record.body_metabolism);
        write_derived(w, &record.derived);
        for collider in &record.colliders {
            write_collider(w, collider);
        }
        for value in [
            record.propulsion_derived.muscle_move_strength,
            record.propulsion_derived.muscle_turn_strength,
            record.propulsion_derived.base_acceleration_cost,
            record.propulsion_derived.base_turn_cost,
        ] {
            w.f32(value);
        }
        for value in [
            record.mouth_derived.jaw_area,
            record.mouth_derived.jaw_mass,
            record.mouth_derived.throat_width,
            record.mouth_derived.jaw_strength,
            record.mouth_derived.bite_period,
        ] {
            w.f32(value);
        }
        w.vec2(record.propulsion.last_pos);
        for value in [
            record.propulsion.total_travel,
            record.propulsion.signed_velocity,
            record.propulsion.normalized_velocity,
            record.propulsion.angular_velocity,
            record.propulsion.angle_degrees,
        ] {
            w.f32(value);
        }
        write_growth(w, &record.growth);
        write_clock(w, &record.clock);
        write_fat(w, &record.fat);
        write_armor(w, &record.armor);
        write_egg_layer(w, &record.egg_layer);
        write_stomach_scalars(w, record);
        w.f32(record.pheromone_progress);
        write_mouth(w, &record.mouth);
        write_body(w, &record.body);
        for value in record.color {
            w.f32(value);
        }
        w.f32(record.eye_offset);
        write_feeding(w, &record.feeding);
    }

    fn read_bibite_runtime(r: &mut CkptR<'_>) -> crate::error::Result<BibiteCheckpointRecord> {
        let save = read_bibite_save(r)?;
        let rot_radians = r.f32()?;
        let throttle = SenseThrottle {
            brain_counter: r.u32()?,
            lookup_counter: r.u32()?,
            sense_counter: r.u32()?,
        };
        let mut senses = [0.0f32; crate::senses::WORLD_INPUT_COUNT];
        for value in &mut senses {
            *value = r.f32()?;
        }
        let sense_has_herd = r.bool()?;
        let sense_herd_sum_direction = r.f32()?;
        let sense_herd_separation_projection = r.f32()?;
        let steering = SteeringInputs {
            has_herd: r.bool()?,
            herd_sum_direction_degrees: r.f32()?,
            herd_separation_projection: r.f32()?,
            herd_velocity_match: r.vec2()?,
            nearest_separation_distance: r.f32()?,
            void_avoidance: r.vec2()?,
        };
        let fov_seen = {
            let pellet_count = r.len(MAX_RECORDS)?;
            let mut pellets = Vec::with_capacity(pellet_count);
            for _ in 0..pellet_count {
                pellets.push(PelletId::from_handle(Handle::new(r.u32()?, r.u32()?)));
            }
            let bibite_count = r.len(MAX_RECORDS)?;
            let mut bibites = Vec::with_capacity(bibite_count);
            for _ in 0..bibite_count {
                bibites.push(BibiteId::from_handle(Handle::new(r.u32()?, r.u32()?)));
            }
            FovSeenCache { pellets, bibites }
        };
        let pending_force = r.vec2()?;
        let pending_torque = r.f32()?;
        let mut mouth_targets = [0u64; 10];
        for target in &mut mouth_targets {
            let hi = r.u32()? as u64;
            let lo = r.u32()? as u64;
            *target = (hi << 32) | lo;
        }
        let n_mouth_targets = r.u8()?;
        if n_mouth_targets as usize > mouth_targets.len() {
            return Err(bad("too many mouth targets"));
        }
        let mut mouth_target_ages = [0u8; 10];
        for age in &mut mouth_target_ages {
            *age = r.u8()?;
        }
        let mut held_targets = [0u64; 10];
        for target in &mut held_targets {
            let hi = r.u32()? as u64;
            let lo = r.u32()? as u64;
            *target = (hi << 32) | lo;
        }
        let n_held_targets = r.u8()?;
        if n_held_targets as usize > held_targets.len() {
            return Err(bad("too many held targets"));
        }
        let death_cause = match r.u8()? {
            0 => None,
            1 => Some(death_cause_from_code(r.u8()?)?),
            _ => return Err(bad("invalid death-cause tag")),
        };
        let is_corpse = r.bool()?;
        let born = r.bool()?;
        let is_ready = r.bool()?;
        let mutation_factor = MutationFactor {
            probability_increase: r.f32()?,
            intensity_increase: r.f32()?,
        };
        let brain_node_max_innovation = r.i64()?;
        let brain_connection_max_innovation = r.i64()?;
        let body_geometry = read_geometry(r)?;
        let body_params = read_params(r)?;
        let body_metabolism = r.f32()?;
        let derived = read_derived(r)?;
        let colliders = [read_collider(r)?, read_collider(r)?, read_collider(r)?];
        let propulsion_derived = organs::PropulsionDerived {
            muscle_move_strength: r.f32()?,
            muscle_turn_strength: r.f32()?,
            base_acceleration_cost: r.f32()?,
            base_turn_cost: r.f32()?,
        };
        let mouth_derived = organs::MouthDerived {
            jaw_area: r.f32()?,
            jaw_mass: r.f32()?,
            throat_width: r.f32()?,
            jaw_strength: r.f32()?,
            bite_period: r.f32()?,
        };
        let propulsion = organs::PropulsionState {
            last_pos: r.vec2()?,
            total_travel: r.f32()?,
            signed_velocity: r.f32()?,
            normalized_velocity: r.f32()?,
            angular_velocity: r.f32()?,
            angle_degrees: r.f32()?,
        };
        let growth = read_growth(r)?;
        let clock = read_clock(r)?;
        let fat = read_fat(r)?;
        let armor = read_armor(r)?;
        let egg_layer = read_egg_layer(r)?;
        let stomach_contents = {
            let count = r.len(STOMACH_SLOTS as u32)?;
            let mut contents = Vec::with_capacity(count);
            for _ in 0..count {
                let material = match r.u8()? {
                    0 => StomachMaterial::Plant,
                    1 => StomachMaterial::Meat,
                    _ => return Err(bad("invalid stomach material")),
                };
                contents.push(StomachContent {
                    material,
                    amount: r.f32()?,
                    average_chunk_amount: r.f32()?,
                    efficiency: r.f32()?,
                    digestion_rate: r.f32()?,
                    last_conversion_efficiency: r.f32()?,
                });
            }
            contents
        };
        let stomach_area = r.f32()?;
        let stomach_total_content = r.f32()?;
        let stomach_acid_level = r.f32()?;
        let stomach_acid_ratio = r.f32()?;
        let stomach_digestion_effectiveness = r.f32()?;
        let stomach_digestion_potential_bonus = r.f32()?;
        let stomach_efficiency_malus = r.f32()?;
        let pheromone_progress = r.f32()?;
        let mouth = read_mouth(r)?;
        Ok(BibiteCheckpointRecord {
            save,
            rot_radians,
            throttle,
            senses: {
                let mut senses = SenseOutputs::from_values(senses);
                senses.has_herd = sense_has_herd;
                senses.herd_sum_direction = sense_herd_sum_direction;
                senses.herd_separation_projection = sense_herd_separation_projection;
                senses
            },
            steering,
            fov_seen,
            pending_force,
            pending_torque,
            mouth_targets,
            n_mouth_targets,
            mouth_target_ages,
            held_targets,
            n_held_targets,
            death_cause,
            is_corpse,
            born,
            is_ready,
            mutation_factor,
            brain_node_max_innovation,
            brain_connection_max_innovation,
            body_geometry,
            body_params,
            body_metabolism,
            derived,
            colliders,
            propulsion_derived,
            mouth_derived,
            propulsion,
            growth,
            clock,
            fat,
            armor,
            egg_layer,
            stomach_area,
            stomach_total_content,
            stomach_acid_level,
            stomach_acid_ratio,
            stomach_digestion_effectiveness,
            stomach_digestion_potential_bonus,
            stomach_efficiency_malus,
            stomach_contents,
            pheromone_progress,
            mouth,
            body: read_body(r)?,
            color: [r.f32()?, r.f32()?, r.f32()?],
            eye_offset: r.f32()?,
            feeding: read_feeding(r)?,
        })
    }

    fn death_cause_code(cause: DeathCause) -> u8 {
        match cause {
            DeathCause::Starvation => 0,
            DeathCause::Bitten => 1,
            DeathCause::Swallowed => 2,
            DeathCause::Collision => 3,
            DeathCause::Hazard => 4,
            DeathCause::Removed => 5,
        }
    }

    fn death_cause_from_code(code: u8) -> crate::error::Result<DeathCause> {
        Ok(match code {
            0 => DeathCause::Starvation,
            1 => DeathCause::Bitten,
            2 => DeathCause::Swallowed,
            3 => DeathCause::Collision,
            4 => DeathCause::Hazard,
            5 => DeathCause::Removed,
            _ => return Err(bad("invalid death cause")),
        })
    }

    pub(super) fn encode_bibite_arena(arena: &BibiteCheckpointArena) -> Vec<u8> {
        let mut w = CkptW::new();
        w.len(arena.generations.len());
        for value in &arena.generations {
            w.u32(*value);
        }
        w.len(arena.free.len());
        for value in &arena.free {
            w.u32(*value);
        }
        w.len(arena.dense.len());
        for value in &arena.dense {
            w.u32(*value);
        }
        w.len(arena.records.len());
        for record in &arena.records {
            match record {
                Some(record) => {
                    w.u8(1);
                    write_bibite_runtime(&mut w, record);
                }
                None => w.u8(0),
            }
        }
        w.out
    }

    pub(super) fn decode_bibite_arena(bytes: &[u8]) -> crate::error::Result<BibiteCheckpointArena> {
        let mut r = CkptR::new(bytes);
        let generation_count = r.len(MAX_RECORDS)?;
        let mut generations = Vec::with_capacity(generation_count);
        for _ in 0..generation_count {
            generations.push(r.u32()?);
        }
        let free_count = r.len(MAX_RECORDS)?;
        let mut free = Vec::with_capacity(free_count);
        for _ in 0..free_count {
            free.push(r.u32()?);
        }
        let dense_count = r.len(MAX_RECORDS)?;
        let mut dense = Vec::with_capacity(dense_count);
        for _ in 0..dense_count {
            dense.push(r.u32()?);
        }
        let record_count = r.len(MAX_RECORDS)?;
        let record_count = record_count.max(generations.len());
        let mut records = Vec::with_capacity(record_count);
        for _ in 0..record_count {
            match r.u8()? {
                0 => records.push(None),
                1 => records.push(Some(read_bibite_runtime(&mut r)?)),
                value => return Err(bad(&format!("invalid record tag {value} at {}", r.at))),
            }
        }
        r.finish()?;
        Ok(BibiteCheckpointArena {
            generations,
            free,
            dense,
            records,
        })
    }

    fn write_egg_save(w: &mut CkptW, save: &EggSave) {
        w.vec2(save.pos);
        w.vec2(save.vel);
        w.f32(save.rot);
        w.f32(save.ang_vel);
        for gene in save.genome.genes {
            w.f32(gene);
        }
        write_meta(w, &save.meta);
        write_brain(w, &save.brain);
        for value in [
            save.energy,
            save.hatch_progress,
            save.hatch_time,
            save.total_required_energy,
            save.size_gene,
        ] {
            w.f32(value);
        }
        w.bool(save.is_hatching);
        w.bool(save.aborting);
        w.vec2(save.birth_orientation);
    }

    fn read_egg_save(r: &mut CkptR<'_>) -> crate::error::Result<EggSave> {
        let pos = r.vec2()?;
        let vel = r.vec2()?;
        let rot = r.f32()?;
        let ang_vel = r.f32()?;
        let mut genes = [0.0f32; crate::genome::N_GENES];
        for gene in &mut genes {
            *gene = r.f32()?;
        }
        let genome = crate::genome::Genome::from_values(genes);
        Ok(EggSave {
            pos,
            vel,
            rot,
            ang_vel,
            genome,
            meta: read_meta(r)?,
            brain: read_brain(r)?,
            energy: r.f32()?,
            hatch_progress: r.f32()?,
            hatch_time: r.f32()?,
            total_required_energy: r.f32()?,
            size_gene: r.f32()?,
            is_hatching: r.bool()?,
            aborting: r.bool()?,
            birth_orientation: r.vec2()?,
        })
    }

    pub(super) fn encode_egg_arena(arena: &EggCheckpointArena) -> Vec<u8> {
        let mut w = CkptW::new();
        w.len(arena.generations.len());
        for value in &arena.generations {
            w.u32(*value);
        }
        w.len(arena.free.len());
        for value in &arena.free {
            w.u32(*value);
        }
        w.len(arena.dense.len());
        for value in &arena.dense {
            w.u32(*value);
        }
        w.len(arena.records.len());
        for record in &arena.records {
            match record {
                Some(record) => {
                    w.u8(1);
                    write_egg_save(&mut w, &record.save);
                    w.f32(record.rot_radians);
                    w.bool(record.is_hatching);
                    w.bool(record.aborting);
                    w.vec2(record.birth_orientation);
                    w.f32(record.size_gene);
                }
                None => w.u8(0),
            }
        }
        w.out
    }

    pub(super) fn decode_egg_arena(bytes: &[u8]) -> crate::error::Result<EggCheckpointArena> {
        let mut r = CkptR::new(bytes);
        let generation_count = r.len(MAX_RECORDS)?;
        let mut generations = Vec::with_capacity(generation_count);
        for _ in 0..generation_count {
            generations.push(r.u32()?);
        }
        let free_count = r.len(MAX_RECORDS)?;
        let mut free = Vec::with_capacity(free_count);
        for _ in 0..free_count {
            free.push(r.u32()?);
        }
        let dense_count = r.len(MAX_RECORDS)?;
        let mut dense = Vec::with_capacity(dense_count);
        for _ in 0..dense_count {
            dense.push(r.u32()?);
        }
        let record_count = r.len(MAX_RECORDS)?;
        let record_count = record_count.max(generations.len());
        let mut records = Vec::with_capacity(record_count);
        for _ in 0..record_count {
            match r.u8()? {
                0 => records.push(None),
                1 => records.push(Some(EggCheckpointRecord {
                    save: read_egg_save(&mut r)?,
                    rot_radians: r.f32()?,
                    is_hatching: r.bool()?,
                    aborting: r.bool()?,
                    birth_orientation: r.vec2()?,
                    size_gene: r.f32()?,
                })),
                value => return Err(bad(&format!("invalid egg record tag {value} at {}", r.at))),
            }
        }
        r.finish()?;
        Ok(EggCheckpointArena {
            generations,
            free,
            dense,
            records,
        })
    }
}

impl BibiteCheckpointArena {
    /// Encodes this checkpoint arena as deterministic little-endian bytes
    /// (length-prefixed, f32 bit patterns preserved exactly).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        checkpoint_codec::encode_bibite_arena(self)
    }

    /// Decodes bytes produced by [`BibiteCheckpointArena::encode`]. Fails closed
    /// on truncation, trailing bytes, unknown tags or oversized fields.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error for malformed/truncated input.
    pub fn decode(bytes: &[u8]) -> crate::error::Result<Self> {
        checkpoint_codec::decode_bibite_arena(bytes)
    }
}

impl EggCheckpointArena {
    /// Encodes this egg checkpoint arena as deterministic little-endian bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        checkpoint_codec::encode_egg_arena(self)
    }

    /// Decodes bytes produced by [`EggCheckpointArena::encode`]. Fails closed on
    /// malformed input.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error for malformed/truncated input.
    pub fn decode(bytes: &[u8]) -> crate::error::Result<Self> {
        checkpoint_codec::decode_egg_arena(bytes)
    }
}
