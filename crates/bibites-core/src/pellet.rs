//! Plant/meat pellets, matter, decay and materials (spec
//! `docs/spec/03-world-ecology.md` §2).
//!
//! This module is the authoritative model of the game's "matter" concept:
//!
//! * [`crate::pellet::MaterialProperties`] mirrors the game's `MatterMaterial` asset data
//!   (cohesiveness, reactivity, energy/mass density, conversion efficiencies,
//!   decay parameters). Defaults are the captured 0.6.3.x values from a fresh
//!   scenario (`Scenarios/RadTower.zip` → `settings.bb8settings/materials`).
//! * [`crate::pellet::Matter`] mirrors the game's `Matter` class: an amount in `u²` with a
//!   maximum, from which energy, mass and radius are derived.
//! * [`crate::pellet::PelletStore`] is a flat, generational-id structure-of-arrays store for
//!   every world pellet. It owns the pellet formulas, the decay processor,
//!   merging, ripping/splitting, meat-pile conversion and the plant/meat
//!   counters.
//!
//! # Amount rule (important)
//!
//! Pellet amount is private and every mutation goes through a choke point
//! ([`crate::pellet::PelletStore::change_amount`] is the contract one used by body feeding
//! code; [`crate::pellet::PelletStore::set_amount`], [`crate::pellet::PelletStore::remove_amount`],
//! [`crate::pellet::PelletStore::remove_energy`], [`crate::pellet::PelletStore::try_merge`] and
//! [`crate::pellet::PelletStore::rip_chunk`] are the finer-grained operations). This keeps the
//! plant/meat counters, decay registration and spatial-hash radius updates
//! consistent by construction.
//!
//! # Fidelity notes
//!
//! * Amounts and energy use `f32` like the game; counter biomass sums use `f64`
//!   like the game's `double` counters.
//! * Unity's deferred `Object.Destroy` is replaced by immediate removal
//!   (documented divergence in `docs/ARCHITECTURE.md`).
//! * The game's prefab collider radius is `2.5 · sqrt(amount/20)` while the
//!   sensing radius is `sqrt(amount/π)`. Both are exposed
//!   ([`crate::pellet::PelletStore::get`] returns the analytic radius,
//!   [`crate::pellet::PelletStore::collider_radius`] the prefab one); the custom solver uses
//!   the analytic radius (spec Q8).

use crate::arena::Handle;
use crate::ids::{PelletId, ZoneId};
use crate::math::{PI, Vec2, clamp01};
use crate::rng::Rng;
use crate::world::{BitSet, random_inside_unit_circle};

/// Amount in `u²` that corresponds to a render scale of 1
/// (`MatterMaterial.NormalAmountAtScale1`).
pub const NORMAL_AMOUNT_AT_SCALE1: f32 = 20.0;

/// Physical prefab collider radius at scale 1
/// (`MatterMaterial.NormalRadiusAtScale1`).
pub const NORMAL_RADIUS_AT_SCALE1: f32 = 2.5;

/// The `decayAmount/amount` ratio above which the decay processor chops rotten
/// matter off a pellet (`MatterDecayOfArrays.Process`).
pub const DECAY_REMOVE_THRESHOLD: f32 = 0.05;

/// The epsilon the game adds to an amount when clamping a decay removal
/// (`toRemove = min(decayAmount, amount + 1e-6)`), guaranteeing full removal
/// instead of leaving a numerical sliver.
pub const DECAY_REMOVE_EPSILON: f32 = 1e-6;

/// Default `minPelletSize` setting (spec §2.6).
pub const DEFAULT_MIN_PELLET_SIZE: f32 = 0.1;

/// Multiplier applied to the corpse half-length when spreading a meat pile
/// (`SpawnMeatPile`: `radius *= 0.1`).
pub const MEAT_PILE_RADIUS_FACTOR: f32 = 0.1;

/// The game's material identities. Only [`MaterialKind::Plant`] and
/// [`MaterialKind::Meat`] can exist as world pellets; fat and armour exist for
/// organs (spec §2.2/§2.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    /// Plant matter: the default zone food (`EnergyDensity = 5 E/u²`, no decay).
    Plant,
    /// Meat matter: corpses and meat piles (`EnergyDensity = 10 E/u²`, rots).
    Meat,
    /// Fat stored in a bibite's fat organ (not spawnable as a pellet).
    Fat,
    /// Armour matter (not spawnable as a pellet).
    Armor,
}

impl MaterialKind {
    /// Parses a game material name (`Plant`, `Meat`, `Fat`, `Armor`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Plant" => Some(Self::Plant),
            "Meat" => Some(Self::Meat),
            "Fat" => Some(Self::Fat),
            "Armor" => Some(Self::Armor),
            _ => None,
        }
    }

    /// Returns `true` for materials that can be spawned as world pellets
    /// (`Plant` and `Meat`); the others are organ-internal.
    #[inline]
    pub const fn is_world_pellet(self) -> bool {
        matches!(self, Self::Plant | Self::Meat)
    }
}

/// Editable properties of a matter material (mirrors `MatterMaterial`).
///
/// Defaults are the captured 0.6.3.x values from a fresh scenario. Units:
/// `cohesiveness` N/u, `reactivity` u/s, `energy_density` E/u²,
/// `mass_density` g/u², `fresh_time` s, `decay_rate` u²/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialProperties {
    /// How hard the material holds together (biting/tearing strength).
    pub cohesiveness: f32,
    /// Base reactivity (`u/s`); digestion speed.
    pub reactivity: f32,
    /// Energy per square unit.
    pub energy_density: f32,
    /// Mass per square unit.
    pub mass_density: f32,
    /// Conversion efficiency at diet affinity 0.
    pub min_efficiency: f32,
    /// Conversion efficiency at diet affinity 1.
    pub max_efficiency: f32,
    /// Whether pellets of this material rot.
    pub decay: bool,
    /// Seconds a pellet stays fresh before rot accumulates.
    pub fresh_time: f32,
    /// Rot accumulation rate once a pellet is not fresh.
    pub decay_rate: f32,
}

impl MaterialProperties {
    /// Plant matter defaults (captured): no decay.
    pub const PLANT: Self = Self {
        cohesiveness: 10.0,
        reactivity: 0.03,
        energy_density: 5.0,
        mass_density: 0.015,
        min_efficiency: -0.25,
        max_efficiency: 0.5,
        decay: false,
        fresh_time: 0.0,
        decay_rate: 0.0,
    };

    /// Meat matter defaults (captured): fresh 2000 s, then 0.02 u²/s rot.
    pub const MEAT: Self = Self {
        cohesiveness: 35.0,
        reactivity: 0.075,
        energy_density: 10.0,
        mass_density: 0.025,
        min_efficiency: 0.15,
        max_efficiency: 0.85,
        decay: true,
        fresh_time: 2000.0,
        decay_rate: 0.02,
    };

    /// Fat matter defaults (captured): organ-internal.
    pub const FAT: Self = Self {
        cohesiveness: 0.25,
        reactivity: 1.0,
        energy_density: 30.0,
        mass_density: 0.03,
        min_efficiency: 0.5,
        max_efficiency: 0.85,
        decay: false,
        fresh_time: 0.0,
        decay_rate: 0.0,
    };

    /// Armour matter defaults (captured): organ-internal.
    pub const ARMOR: Self = Self {
        cohesiveness: 500.0,
        reactivity: 1.0,
        energy_density: 1.5,
        mass_density: 0.05,
        min_efficiency: 1.0,
        max_efficiency: 1.0,
        decay: false,
        fresh_time: 0.0,
        decay_rate: 0.0,
    };

    /// Conversion efficiency at a given diet affinity:
    /// `min + clamp01(affinity) · (max − min)`.
    #[inline]
    pub fn conversion_efficiency(&self, affinity: f32) -> f32 {
        self.min_efficiency + clamp01(affinity) * (self.max_efficiency - self.min_efficiency)
    }

    /// Energy contained in `amount` square units.
    #[inline]
    pub fn energy_of_amount(&self, amount: f32) -> f32 {
        amount * self.energy_density
    }

    /// Amount of matter needed to hold `energy`.
    #[inline]
    pub fn amount_of_energy(&self, energy: f32) -> f32 {
        energy / self.energy_density
    }

    /// Radius of a circular blob of `amount` square units.
    #[inline]
    pub fn radius_of_amount(&self, amount: f32) -> f32 {
        (amount.max(0.0) / PI).sqrt()
    }

    /// Area of a circle with `radius`.
    #[inline]
    pub fn amount_of_radius(&self, radius: f32) -> f32 {
        PI * radius * radius
    }

    /// Energy of a pellet at render scale 1 (`NormalEnergyAtScale1`).
    #[inline]
    pub fn normal_energy_at_scale1(&self) -> f32 {
        self.energy_density * NORMAL_AMOUNT_AT_SCALE1
    }
}

/// The four world materials with their properties (mirrors
/// `MatterMaterialManager.MaterialSettings` for Plant/Meat/Fat/Armor).
///
/// Settings code may mutate this table at runtime; after changing an energy
/// density call [`PelletStore::recompute_counts`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialTable {
    /// Plant matter properties.
    pub plant: MaterialProperties,
    /// Meat matter properties.
    pub meat: MaterialProperties,
    /// Fat matter properties (organ-internal).
    pub fat: MaterialProperties,
    /// Armour matter properties (organ-internal).
    pub armor: MaterialProperties,
}

impl MaterialTable {
    /// Returns the properties of `kind`.
    #[inline]
    pub fn get(&self, kind: MaterialKind) -> &MaterialProperties {
        match kind {
            MaterialKind::Plant => &self.plant,
            MaterialKind::Meat => &self.meat,
            MaterialKind::Fat => &self.fat,
            MaterialKind::Armor => &self.armor,
        }
    }

    /// Returns a mutable reference to the properties of `kind`.
    #[inline]
    pub fn get_mut(&mut self, kind: MaterialKind) -> &mut MaterialProperties {
        match kind {
            MaterialKind::Plant => &mut self.plant,
            MaterialKind::Meat => &mut self.meat,
            MaterialKind::Fat => &mut self.fat,
            MaterialKind::Armor => &mut self.armor,
        }
    }
}

impl Default for MaterialTable {
    fn default() -> Self {
        Self {
            plant: MaterialProperties::PLANT,
            meat: MaterialProperties::MEAT,
            fat: MaterialProperties::FAT,
            armor: MaterialProperties::ARMOR,
        }
    }
}

/// One lump of matter: amount in `u²`, a cap (infinite for world pellets) and
/// the material properties (mirrors `Matter`; spec §2.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matter {
    /// Current amount in `u²`.
    pub amount: f32,
    /// Maximum amount in `u²`; `f32::INFINITY` for pellets.
    pub max_amount: f32,
    /// Material properties of this matter.
    pub material: MaterialProperties,
}

impl Matter {
    /// Creates matter with an infinite cap.
    #[inline]
    pub fn new(material: MaterialProperties, amount: f32) -> Self {
        Self {
            amount,
            max_amount: f32::INFINITY,
            material,
        }
    }

    /// Creates matter with an explicit maximum (organs use finite caps).
    #[inline]
    pub fn with_max(material: MaterialProperties, amount: f32, max_amount: f32) -> Self {
        Self {
            amount,
            max_amount,
            material,
        }
    }

    /// `amount / max_amount`, clamped at 0 (`Matter.fullness`).
    #[inline]
    pub fn fullness(&self) -> f32 {
        (self.amount / self.max_amount).max(0.0)
    }

    /// `true` if the amount is approximately zero (`Matter.isEmpty`).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.amount.abs() <= 1e-5
    }

    /// `true` if the matter is full (`Matter.isFull`).
    #[inline]
    pub fn is_full(&self) -> bool {
        self.available().abs() <= 1e-5
    }

    /// Amount above the cap (`Matter.overflow`).
    #[inline]
    pub fn overflow(&self) -> f32 {
        (self.amount - self.max_amount).max(0.0)
    }

    /// Room left under the cap (`Matter.available`).
    #[inline]
    pub fn available(&self) -> f32 {
        (self.max_amount - self.amount).max(0.0)
    }

    /// Radius `sqrt(amount/π)` (`Matter.radius`).
    #[inline]
    pub fn radius(&self) -> f32 {
        (self.amount.max(0.0) / PI).sqrt()
    }

    /// Cohesiveness of the material (`Matter.cohesiveness`).
    #[inline]
    pub fn cohesiveness(&self) -> f32 {
        self.material.cohesiveness
    }

    /// Mass `|amount · mass_density|` (`Matter.mass`).
    #[inline]
    pub fn mass(&self) -> f32 {
        (self.amount * self.material.mass_density).abs()
    }

    /// Energy `amount · energy_density`.
    #[inline]
    pub fn energy(&self) -> f32 {
        self.amount * self.material.energy_density
    }

    /// Replaces the amount with the equivalent of `energy`.
    #[inline]
    pub fn set_energy(&mut self, energy: f32) {
        self.amount = energy / self.material.energy_density;
    }

    /// Amount of matter if converted to another material (`ConvertToMaterial`).
    #[inline]
    pub fn convert_to_material(
        &self,
        to: &MaterialProperties,
        with_losses: bool,
        affinity_from: f32,
        affinity_to: f32,
    ) -> f32 {
        let efficiency = if with_losses {
            self.material.conversion_efficiency(affinity_from)
                * to.conversion_efficiency(affinity_to)
        } else {
            1.0
        };
        self.energy() / to.energy_density * efficiency
    }

    /// Transfers `amount` units into (positive) or out of (negative) this
    /// matter, returning the signed energy exchange
    /// (`Matter.ConvertToFromEnergy`).
    #[inline]
    pub fn convert_to_from_energy(&mut self, amount: f32, affinity: f32) -> f32 {
        let clamped = if amount > 0.0 {
            amount.min(self.max_amount - self.amount)
        } else {
            amount.max(-self.amount)
        };
        self.amount += clamped;
        let efficiency = self.material.conversion_efficiency(affinity);
        if clamped > 0.0 {
            -clamped * self.material.energy_density / efficiency
        } else {
            -clamped * self.material.energy_density * efficiency
        }
    }
}

/// Parameters for spawning a new pellet.
#[derive(Clone, Copy, Debug)]
pub struct PelletSpawn {
    /// Material of the new pellet (must be [`MaterialKind::is_world_pellet`]).
    pub material: MaterialKind,
    /// Amount in `u²` (must be finite and `> 0`).
    pub amount: f32,
    /// World position.
    pub pos: Vec2,
    /// Initial linear velocity (usually zero).
    pub velocity: Vec2,
    /// Initial rotation in radians.
    pub rotation: f32,
    /// Initial angular velocity in radians/s.
    pub angular_velocity: f32,
    /// Owning zone; `None` means "free pellets".
    pub owner_zone: Option<ZoneId>,
    /// Whether the pellet is held by a mouth at spawn time.
    pub held: bool,
}

impl PelletSpawn {
    /// Creates a spawn request at rest (`velocity = 0`, `rotation = 0`).
    #[inline]
    pub fn new(material: MaterialKind, amount: f32, pos: Vec2) -> Self {
        Self {
            material,
            amount,
            pos,
            velocity: Vec2::ZERO,
            rotation: 0.0,
            angular_velocity: 0.0,
            owner_zone: None,
            held: false,
        }
    }
}

/// Cheap read-only copy of the pellet fields the body/world need
/// (`docs/INTERFACES.md` §6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PelletView {
    /// Typed id of this pellet.
    pub id: PelletId,
    /// Amount in `u²`.
    pub amount: f32,
    /// Material kind.
    pub material: MaterialKind,
    /// World position.
    pub pos: Vec2,
    /// Linear velocity.
    pub velocity: Vec2,
    /// Analytic radius `sqrt(amount/π)`.
    pub radius: f32,
    /// Whether a mouth holds this pellet.
    pub held: bool,
    /// Owning zone (`None` = free pellet).
    pub zone: Option<ZoneId>,
}

/// Global plant/meat counters (mirrors `PlantCounter` / `MeatCounter`; the
/// interface contract calls this `PelletCounts`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PelletCounts {
    /// Number of live plant pellets.
    pub plant_count: u32,
    /// Number of live meat pellets.
    pub meat_count: u32,
    /// Total energy of live plant pellets.
    pub plant_biomass: f64,
    /// Total energy of live meat pellets.
    pub meat_biomass: f64,
}

impl PelletCounts {
    /// Total pellet count (plants + meat).
    #[inline]
    pub fn total_count(&self) -> u32 {
        self.plant_count + self.meat_count
    }

    /// Total biomass of live pellets.
    #[inline]
    pub fn total_biomass(&self) -> f64 {
        self.plant_biomass + self.meat_biomass
    }
}

/// A pellet's physics pose (copy of the SoA transform fields).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PelletPose {
    /// World position.
    pub pos: Vec2,
    /// Linear velocity in `u/s`.
    pub velocity: Vec2,
    /// Rotation in radians.
    pub rotation: f32,
    /// Angular velocity in radians/s.
    pub angular_velocity: f32,
}

/// Mutable pose view used by the physics solver for one pellet.
#[derive(Debug)]
pub struct PoseMut<'a> {
    /// World position.
    pub pos: &'a mut Vec2,
    /// Linear velocity.
    pub velocity: &'a mut Vec2,
    /// Rotation in radians.
    pub rotation: &'a mut f32,
    /// Angular velocity in radians/s.
    pub angular_velocity: &'a mut f32,
}

/// Ticks between decay updates (the game's `decayTPS`; default 2).
///
/// The decay processor runs once every `period` ticks and must be advanced with
/// `dt_decay = fixed_dt · period`, matching `MatterDecayProcessor.FixedUpdate`.
#[derive(Clone, Copy, Debug)]
pub struct DecayThrottle {
    period: u32,
    progress: u32,
}

impl DecayThrottle {
    /// Creates a throttle with the given period in ticks (`decayTPS`).
    ///
    /// # Panics
    ///
    /// Panics if `period == 0`.
    pub fn new(period: u32) -> Self {
        assert!(period > 0, "decay period must be positive");
        Self {
            period,
            progress: 0,
        }
    }

    /// Advances one tick and returns `true` when the decay processor is due.
    #[inline]
    pub fn tick(&mut self) -> bool {
        self.progress += 1;
        if self.progress >= self.period {
            self.progress = 0;
            true
        } else {
            false
        }
    }

    /// The configured period in ticks.
    #[inline]
    pub fn period(&self) -> u32 {
        self.period
    }

    /// The current progress toward the next update.
    #[inline]
    pub fn progress(&self) -> u32 {
        self.progress
    }

    /// Restores progress within the period (checkpoint resume).
    #[inline]
    pub fn set_progress(&mut self, progress: u32) {
        self.progress = progress.min(self.period.saturating_sub(1));
    }

    /// Resets the progress counter (after load / settings changes).
    #[inline]
    pub fn reset(&mut self) {
        self.progress = 0;
    }
}

/// Result of [`crate::pellet::PelletStore::rip_chunk`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RipChunkResult {
    /// The newly created pellet carrying the ripped chunk.
    pub new_pellet: PelletId,
    /// Amount removed from the source pellet (`chunk_amount` after clamping).
    pub removed_amount: f32,
}

/// A flat SoA store of world pellets with generational ids.
///
/// Arrays are indexed by *slot*; ids are `(slot, generation)` handles. Slot
/// reuse invalidates old handles. Live pellets are iterated in dense
/// (insertion-like) order via [`PelletStore::ids`], which is deterministic for a
/// given mutation history.
pub struct PelletStore {
    amount: Vec<f32>,
    material: Vec<MaterialKind>,
    pos: Vec<Vec2>,
    velocity: Vec<Vec2>,
    rotation: Vec<f32>,
    angular_velocity: Vec<f32>,
    held: BitSet,
    owner: Vec<Option<ZoneId>>,
    decaying: BitSet,
    fresh_remaining: Vec<f32>,
    decay_amount: Vec<f32>,
    /// RUSTibites Improved: the factor on the pellet's linear drag, 1 for every pellet but a hydrothermal vent's while it flies (`vent.rs`).
    drag_factor: Vec<f32>,
    generations: Vec<u32>,
    alive: BitSet,
    dense: Vec<u32>,
    dense_pos: Vec<u32>,
    free: Vec<u32>,
    materials: MaterialTable,
    counts: PelletCounts,
    pending_removals: Vec<(u32, f32)>,
    /// `SettingsUpdater.freshValueDifference`: the last change of a Plant or Meat freshness time (new - old). The game keeps it in a field
    /// and applies it again whenever a decay RATE is edited (see [`Self::replace_materials`]). Run-time only: it is not in a save.
    fresh_value_difference: f32,
    /// Slots whose position, radius or liveness may differ from what the world's spatial hash holds (every write to
    /// those goes through this file, and marks the slot); the hash sync drains it, so it visits the pellets that
    /// changed instead of all of them.
    hash_dirty: Vec<u32>,
    hash_dirty_bits: BitSet,
    /// Slots whose physics body may differ from the one built from them last time (anything written to a pellet except the
    /// physics step's own write-back); drained by the physics bridge, which lets resting pellets skip the step.
    phys_dirty: Vec<u32>,
    phys_dirty_bits: BitSet,
    /// Bumped when the whole store is replaced (a checkpoint restore): everything derived from the slots is stale.
    epoch: u64,
}

/// Materials removed by one decay tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DecayReport {
    /// Pellets fully destroyed by this decay pass.
    pub removed_pellets: u32,
    /// Plant matter removed (u²), including partial chops.
    pub plant_amount: f64,
    /// Meat matter removed (u²), including partial chops.
    pub meat_amount: f64,
}

impl PelletStore {
    /// Creates an empty store with the captured default material table.
    pub const fn new() -> Self {
        Self {
            amount: Vec::new(),
            material: Vec::new(),
            pos: Vec::new(),
            velocity: Vec::new(),
            rotation: Vec::new(),
            angular_velocity: Vec::new(),
            held: BitSet::new(),
            owner: Vec::new(),
            decaying: BitSet::new(),
            fresh_remaining: Vec::new(),
            decay_amount: Vec::new(),
            drag_factor: Vec::new(),
            generations: Vec::new(),
            alive: BitSet::new(),
            dense: Vec::new(),
            dense_pos: Vec::new(),
            free: Vec::new(),
            materials: MaterialTable {
                plant: MaterialProperties::PLANT,
                meat: MaterialProperties::MEAT,
                fat: MaterialProperties::FAT,
                armor: MaterialProperties::ARMOR,
            },
            counts: PelletCounts {
                plant_count: 0,
                meat_count: 0,
                plant_biomass: 0.0,
                meat_biomass: 0.0,
            },
            pending_removals: Vec::new(),
            fresh_value_difference: 0.0,
            hash_dirty: Vec::new(),
            hash_dirty_bits: BitSet::new(),
            phys_dirty: Vec::new(),
            phys_dirty_bits: BitSet::new(),
            epoch: 0,
        }
    }

    /// Creates an empty store with room for `capacity` pellets.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut store = Self::new();
        store.reserve(capacity);
        store
    }

    /// Reserves capacity for at least `additional` more pellets.
    pub fn reserve(&mut self, additional: usize) {
        self.amount.reserve(additional);
        self.material.reserve(additional);
        self.pos.reserve(additional);
        self.velocity.reserve(additional);
        self.rotation.reserve(additional);
        self.angular_velocity.reserve(additional);
        self.owner.reserve(additional);
        self.fresh_remaining.reserve(additional);
        self.decay_amount.reserve(additional);
        self.drag_factor.reserve(additional);
        self.generations.reserve(additional);
        self.dense.reserve(additional);
        self.dense_pos.reserve(additional);
        self.free.reserve(additional);
    }

    /// The store's material table.
    #[inline]
    pub fn materials(&self) -> &MaterialTable {
        &self.materials
    }

    /// Mutable access to the material table.
    ///
    /// After changing an `energy_density`, call [`Self::recompute_counts`].
    #[inline]
    pub fn materials_mut(&mut self) -> &mut MaterialTable {
        &mut self.materials
    }

    /// Number of live pellets.
    #[inline]
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// `true` if the store holds no pellets.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// The plant/meat counters (`docs/INTERFACES.md` §6: `counts()`).
    #[inline]
    pub fn counts(&self) -> PelletCounts {
        self.counts
    }

    /// Recomputes the counters from scratch (after load or material changes).
    pub fn recompute_counts(&mut self) {
        let mut counts = PelletCounts::default();
        for &slot in &self.dense {
            let slot = slot as usize;
            if !self.alive.get(slot) {
                continue;
            }
            let energy = self.energy_at(slot) as f64;
            match self.material[slot] {
                MaterialKind::Plant => {
                    counts.plant_count += 1;
                    counts.plant_biomass += energy;
                }
                MaterialKind::Meat => {
                    counts.meat_count += 1;
                    counts.meat_biomass += energy;
                }
                MaterialKind::Fat | MaterialKind::Armor => {}
            }
        }
        self.counts = counts;
    }

    /// Number of allocated slots (live + vacant).
    #[inline]
    pub fn slot_capacity(&self) -> usize {
        self.generations.len()
    }

    /// Returns `true` if `id` refers to a live pellet.
    #[inline]
    pub fn contains(&self, id: PelletId) -> bool {
        self.slot_of(id).is_some()
    }

    /// Iterates over all live pellet ids in dense order
    /// (`docs/INTERFACES.md` §6: `ids()`).
    pub fn ids(&self) -> impl Iterator<Item = PelletId> + '_ {
        self.dense
            .iter()
            .map(move |&slot| self.id_at_slot(slot as usize))
    }

    /// Cheap copy of the pellet fields body/world need
    /// (`docs/INTERFACES.md` §6: `get(id)`).
    #[inline]
    pub fn get(&self, id: PelletId) -> Option<PelletView> {
        let slot = self.slot_of(id)?;
        Some(PelletView {
            id,
            amount: self.amount[slot],
            material: self.material[slot],
            pos: self.pos[slot],
            velocity: self.velocity[slot],
            radius: (self.amount[slot].max(0.0) / PI).sqrt(),
            held: self.held.get(slot),
            zone: self.owner[slot],
        })
    }

    /// Pellet amount in `u²`.
    #[inline]
    pub fn amount(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.amount[slot])
    }

    /// Energy of a pellet (`amount · energy_density`).
    #[inline]
    pub fn energy(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.energy_at(slot))
    }

    /// Material of a pellet.
    #[inline]
    pub fn material(&self, id: PelletId) -> Option<MaterialKind> {
        self.slot_of(id).map(|slot| self.material[slot])
    }

    /// Position of a pellet.
    #[inline]
    pub fn pos(&self, id: PelletId) -> Option<Vec2> {
        self.slot_of(id).map(|slot| self.pos[slot])
    }

    /// Linear velocity of a pellet.
    #[inline]
    pub fn velocity(&self, id: PelletId) -> Option<Vec2> {
        self.slot_of(id).map(|slot| self.velocity[slot])
    }

    /// Rotation of a pellet in radians.
    #[inline]
    pub fn rotation(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.rotation[slot])
    }

    /// Angular velocity of a pellet in radians/s.
    #[inline]
    pub fn angular_velocity(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.angular_velocity[slot])
    }

    /// Returns a copy of a pellet's pose.
    #[inline]
    pub fn pose(&self, id: PelletId) -> Option<PelletPose> {
        let slot = self.slot_of(id)?;
        Some(PelletPose {
            pos: self.pos[slot],
            velocity: self.velocity[slot],
            rotation: self.rotation[slot],
            angular_velocity: self.angular_velocity[slot],
        })
    }

    /// Overwrites a pellet's pose. Returns `false` if the pellet is not live.
    #[inline]
    pub fn set_pose(&mut self, id: PelletId, pose: PelletPose) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                if self.pos[slot].x.to_bits() != pose.pos.x.to_bits()
                    || self.pos[slot].y.to_bits() != pose.pos.y.to_bits()
                {
                    self.mark_hash_dirty(slot);
                }
                self.mark_phys_dirty(slot);
                self.pos[slot] = pose.pos;
                self.velocity[slot] = pose.velocity;
                self.rotation[slot] = pose.rotation;
                self.angular_velocity[slot] = pose.angular_velocity;
                true
            }
            None => false,
        }
    }

    /// Analytic sensing radius `sqrt(amount/π)` (`Matter.radius`).
    #[inline]
    pub fn radius(&self, id: PelletId) -> Option<f32> {
        self.amount(id).map(|amount| (amount.max(0.0) / PI).sqrt())
    }

    /// Prefab collider radius `2.5 · sqrt(amount/20)` (see module docs).
    #[inline]
    pub fn collider_radius(&self, id: PelletId) -> Option<f32> {
        self.amount(id).map(|amount| {
            NORMAL_RADIUS_AT_SCALE1 * (amount.max(0.0) / NORMAL_AMOUNT_AT_SCALE1).sqrt()
        })
    }

    /// Render scale factor `amount/20` (`MatterPellet.sizeFactor`).
    #[inline]
    pub fn size_factor(&self, id: PelletId) -> Option<f32> {
        self.amount(id)
            .map(|amount| amount.max(0.0) / NORMAL_AMOUNT_AT_SCALE1)
    }

    /// Render scale `sqrt(sizeFactor)` (`MatterPellet.radiusFactor`).
    #[inline]
    pub fn scale(&self, id: PelletId) -> Option<f32> {
        self.size_factor(id).map(f32::sqrt)
    }

    /// Mass in `g` (`|amount · mass_density|`).
    #[inline]
    pub fn mass(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.mass_at(slot))
    }

    /// Linear drag `drag · scale / mass`, or `0` for a held pellet when
    /// `disable_drag_on_held` is enabled (`MatterPellet.UpdateDrag`).
    pub fn linear_drag(
        &self,
        id: PelletId,
        drag_coefficient: f32,
        disable_drag_on_held: bool,
    ) -> Option<f32> {
        let slot = self.slot_of(id)?;
        if disable_drag_on_held && self.held.get(slot) {
            return Some(0.0);
        }
        let mass = self.mass_at(slot);
        if mass <= 0.0 {
            return Some(0.0);
        }
        let scale = (self.amount[slot].max(0.0) / NORMAL_AMOUNT_AT_SCALE1).sqrt();
        Some(drag_coefficient * scale / mass)
    }

    /// Angular damping `drag / 2` (set once at creation by the game;
    /// `MatterPellet.Awake`).
    #[inline]
    pub fn angular_damping(drag_coefficient: f32) -> f32 {
        drag_coefficient / 2.0
    }

    /// Owning zone, or `None` for free pellets.
    #[inline]
    pub fn owner_zone(&self, id: PelletId) -> Option<Option<ZoneId>> {
        self.slot_of(id).map(|slot| self.owner[slot])
    }

    /// Assigns the owning zone of a live pellet.
    #[inline]
    pub fn set_owner_zone(&mut self, id: PelletId, owner: Option<ZoneId>) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.owner[slot] = owner;
                true
            }
            None => false,
        }
    }

    /// RUSTibites Improved: the factor on a live pellet's linear drag (1: the game's drag).
    #[inline]
    pub fn drag_factor(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.drag_factor[slot])
    }

    /// RUSTibites Improved: sets the factor on a live pellet's linear drag (a value that is not a positive number leaves it).
    #[inline]
    pub fn set_drag_factor(&mut self, id: PelletId, factor: f32) -> bool {
        match self.slot_of(id) {
            Some(slot) if factor.is_finite() && factor > 0.0 => {
                self.drag_factor[slot] = factor;
                true
            }
            _ => false,
        }
    }

    /// Whether a pellet is held by a mouth.
    #[inline]
    pub fn is_held(&self, id: PelletId) -> bool {
        self.slot_of(id).is_some_and(|slot| self.held.get(slot))
    }

    /// Marks a pellet as held / released.
    #[inline]
    pub fn set_held(&mut self, id: PelletId, held: bool) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.held.set(slot, held);
                self.mark_phys_dirty(slot);
                true
            }
            None => false,
        }
    }

    /// Whether this pellet's material currently has decay enabled.
    #[inline]
    pub fn is_decaying(&self, id: PelletId) -> bool {
        self.slot_of(id).is_some_and(|slot| self.decaying.get(slot))
    }

    /// Fresh time remaining before rot starts, in seconds.
    #[inline]
    pub fn fresh_time_remaining(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.fresh_remaining[slot])
    }

    /// Accumulated rot in `u²`.
    #[inline]
    pub fn rot_amount(&self, id: PelletId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.decay_amount[slot])
    }

    /// Restores serialized decay state (`matterDecay {timeAlive, rotAmount}`):
    /// `fresh_time_remaining = material.freshTime − timeAlive`.
    ///
    /// The game saves decay only for decaying materials; callers may pass the
    /// material's `fresh_time` for unknown/legacy saves.
    pub fn set_decay_state(
        &mut self,
        id: PelletId,
        fresh_time_remaining: f32,
        rot_amount: f32,
    ) -> bool {
        let Some(slot) = self.slot_of(id) else {
            return false;
        };
        self.fresh_remaining[slot] = fresh_time_remaining.max(0.0);
        self.decay_amount[slot] = rot_amount.max(0.0);
        true
    }

    /// Spawns a new pellet, panicking on invalid input.
    ///
    /// # Panics
    ///
    /// Panics if the material is not world-spawnable, or the amount/position is
    /// invalid. Load paths that must tolerate bad data should use
    /// [`PelletStore::try_spawn`].
    pub fn spawn(&mut self, spec: PelletSpawn) -> PelletId {
        self.try_spawn(spec).expect(
            "invalid pellet spawn: non-world material, non-positive amount or non-finite position",
        )
    }

    /// Spawns a new pellet, returning `None` on invalid input.
    pub fn try_spawn(&mut self, spec: PelletSpawn) -> Option<PelletId> {
        if !spec.material.is_world_pellet()
            || !spec.amount.is_finite()
            || spec.amount <= 0.0
            || !spec.pos.is_finite()
        {
            return None;
        }

        let slot = if let Some(slot) = self.free.pop() {
            let slot = slot as usize;
            self.generations[slot] = self.generations[slot].wrapping_add(1);
            slot
        } else {
            let slot = self.amount.len();
            self.push_slot();
            slot
        };

        self.amount[slot] = spec.amount;
        self.material[slot] = spec.material;
        self.pos[slot] = spec.pos;
        self.velocity[slot] = spec.velocity;
        self.rotation[slot] = spec.rotation;
        self.angular_velocity[slot] = spec.angular_velocity;
        self.owner[slot] = spec.owner_zone;
        self.decay_amount[slot] = 0.0;
        self.drag_factor[slot] = 1.0;
        let props = *self.materials.get(spec.material);
        self.fresh_remaining[slot] = props.fresh_time;
        self.decaying.set(slot, props.decay);
        self.held.set(slot, spec.held);
        self.alive.set(slot, true);
        self.mark_dirty(slot);

        self.dense_pos[slot] = self.dense.len() as u32;
        self.dense.push(slot as u32);

        self.counts_add(spec.material, self.energy_at(slot) as f64);

        Some(self.id_at_slot(slot))
    }

    /// Spawns a pellet from an energy value (`SpawnPelletOfMatter(energy=…)`).
    ///
    /// Returns `None` if the derived amount is not positive/finite.
    pub fn spawn_with_energy(
        &mut self,
        material: MaterialKind,
        energy: f32,
        pos: Vec2,
    ) -> Option<PelletId> {
        if !energy.is_finite() || energy <= 0.0 || !material.is_world_pellet() {
            return None;
        }
        let amount = self.materials.get(material).amount_of_energy(energy);
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        self.try_spawn(PelletSpawn::new(material, amount, pos))
    }

    /// Immediately removes a pellet (`docs/INTERFACES.md` §6: `remove(id)`).
    pub fn remove(&mut self, id: PelletId) {
        if let Some(slot) = self.slot_of(id) {
            self.destroy_slot(slot);
        }
    }

    /// Immediately removes a pellet and returns `true` if it was alive.
    pub fn despawn(&mut self, id: PelletId) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.destroy_slot(slot);
                true
            }
            None => false,
        }
    }

    /// **The single amount change choke point used by body feeding code**
    /// (`docs/INTERFACES.md` §6): adds (`delta > 0`) or removes (`delta < 0`)
    /// matter and returns the signed amount actually changed.
    ///
    /// Removing at least the remaining amount destroys the pellet and returns a
    /// negative value whose magnitude is the remaining amount (mirrors
    /// `MatterPellet.RemoveAmount`). Counters are updated exactly.
    pub fn change_amount(&mut self, id: PelletId, delta: f32) -> f32 {
        if !delta.is_finite() || delta == 0.0 {
            return 0.0;
        }
        if delta > 0.0 {
            let current = match self.amount(id) {
                Some(current) => current,
                None => return 0.0,
            };
            self.set_amount(id, current + delta);
            delta
        } else {
            -self.remove_amount(id, -delta)
        }
    }

    /// Sets a pellet's amount, adjusting the counters by the energy difference.
    ///
    /// Does **not** destroy a pellet that reaches zero (the game's amount setter
    /// behaves the same); use [`Self::change_amount`] / [`Self::remove_amount`]
    /// for the destruction-aware path.
    pub fn set_amount(&mut self, id: PelletId, amount: f32) -> f32 {
        let slot = match self.slot_of(id) {
            Some(slot) => slot,
            None => return 0.0,
        };
        if !amount.is_finite() || amount < 0.0 {
            return self.amount[slot];
        }
        let density = self.materials.get(self.material[slot]).energy_density;
        let delta = amount - self.amount[slot];
        self.amount[slot] = amount;
        self.mark_dirty(slot);
        self.counts_add_energy(self.material[slot], (delta * density) as f64);
        amount
    }

    /// Adds `delta` to a pellet's amount, returning the amount actually added.
    pub fn add_amount(&mut self, id: PelletId, delta: f32) -> f32 {
        if !delta.is_finite() || delta <= 0.0 {
            return 0.0;
        }
        let current = match self.amount(id) {
            Some(current) => current,
            None => return 0.0,
        };
        self.set_amount(id, current + delta);
        delta
    }

    /// Removes up to `max_amount` from a pellet, destroying it when the whole
    /// remaining amount is removed (`MatterPellet.RemoveAmount`).
    ///
    /// Returns the amount actually removed.
    pub fn remove_amount(&mut self, id: PelletId, max_amount: f32) -> f32 {
        let slot = match self.slot_of(id) {
            Some(slot) => slot,
            None => return 0.0,
        };
        if !max_amount.is_finite() || max_amount <= 0.0 {
            return 0.0;
        }
        let current = self.amount[slot];
        if current <= max_amount {
            self.set_amount_by_slot(slot, 0.0);
            self.destroy_slot(slot);
            current
        } else {
            self.set_amount_by_slot(slot, current - max_amount);
            max_amount
        }
    }

    /// Removes up to `energy` worth of matter, returning the energy actually
    /// removed (`MatterPellet.RemoveEnergy`).
    pub fn remove_energy(&mut self, id: PelletId, energy: f32) -> f32 {
        let slot = match self.slot_of(id) {
            Some(slot) => slot,
            None => return 0.0,
        };
        if !energy.is_finite() || energy <= 0.0 {
            return 0.0;
        }
        let density = self.materials.get(self.material[slot]).energy_density;
        let removed = self.remove_amount(id, energy / density);
        removed * density
    }

    /// The game's pellet merge rule (`MatterPellet.OnCollisionEnter2D`): if both
    /// pellets share a material and `absorbed.amount <= keeper.amount`, the
    /// keeper absorbs the other, taking its amount and momentum-weighted
    /// velocity.
    ///
    /// Returns the surviving pellet id (`keeper`) on success.
    pub fn try_merge(&mut self, keeper: PelletId, absorbed: PelletId) -> Option<PelletId> {
        if keeper == absorbed {
            return None;
        }
        let keeper_slot = self.slot_of(keeper)?;
        let absorbed_slot = self.slot_of(absorbed)?;
        if self.material[keeper_slot] != self.material[absorbed_slot] {
            return None;
        }
        let keeper_amount = self.amount[keeper_slot];
        let absorbed_amount = self.amount[absorbed_slot];
        if absorbed_amount > keeper_amount {
            return None;
        }

        let density = self.materials.get(self.material[keeper_slot]).mass_density;
        let keeper_mass = keeper_amount * density;
        let absorbed_mass = absorbed_amount * density;
        let total_mass = keeper_mass + absorbed_mass;
        if total_mass > 0.0 {
            let velocity = (self.velocity[keeper_slot] * keeper_mass
                + self.velocity[absorbed_slot] * absorbed_mass)
                / total_mass;
            self.velocity[keeper_slot] = velocity;
            self.mark_phys_dirty(keeper_slot);
        }
        self.add_amount(keeper, absorbed_amount);
        self.destroy_slot(absorbed_slot);
        Some(keeper)
    }

    /// Splits a chunk off a pellet (`MatterPellet.RipChunkOff`).
    ///
    /// `chunk_amount` is clamped to the pellet's amount. Fails (`None`) when
    /// either the chunk or the remaining pellet would be smaller than
    /// `min_pellet_size`.
    ///
    /// `normal`, when provided, orients the chunk (the game uses the bite
    /// normal); otherwise a random point in the unit disc is used, consuming
    /// `rng`. The new pellet keeps the source's owning zone and starts at rest.
    pub fn rip_chunk(
        &mut self,
        id: PelletId,
        mut chunk_amount: f32,
        normal: Option<Vec2>,
        min_pellet_size: f32,
        rng: &mut Rng,
    ) -> Option<RipChunkResult> {
        let slot = self.slot_of(id)?;
        if !chunk_amount.is_finite() || chunk_amount <= 0.0 {
            return None;
        }
        chunk_amount = chunk_amount.min(self.amount[slot]);
        if chunk_amount < min_pellet_size || self.amount[slot] - chunk_amount < min_pellet_size {
            return None;
        }

        let material = self.material[slot];
        let owner = self.owner[slot];

        // The game removes the chunk first, then uses the remaining radius.
        self.remove_amount(id, chunk_amount);
        let remaining_slot = self.slot_of(id)?;
        let remaining_radius = (self.amount[remaining_slot].max(0.0) / PI).sqrt();

        // num = sqrt(chunk)/π (note: NOT sqrt(chunk/π) in the source).
        let num = chunk_amount.sqrt() / PI;
        let direction = match normal {
            Some(normal) => normal,
            None => random_inside_unit_circle(rng),
        } * (num + remaining_radius);

        let chunk_shift = direction * (chunk_amount / (chunk_amount + self.amount[remaining_slot]));
        let new_source_pos = self.pos[remaining_slot] - chunk_shift;
        let chunk_pos = new_source_pos + direction;

        // The game offsets the source body and spawns the chunk at source+dir.
        self.pos[remaining_slot] = new_source_pos;
        self.mark_dirty(remaining_slot);
        let new_pellet = self.try_spawn(PelletSpawn {
            material,
            amount: chunk_amount,
            pos: chunk_pos,
            velocity: Vec2::ZERO,
            rotation: 0.0,
            angular_velocity: 0.0,
            owner_zone: owner,
            held: false,
        })?;
        Some(RipChunkResult {
            new_pellet,
            removed_amount: chunk_amount,
        })
    }

    /// The interface-contract meat pile spawner
    /// (`docs/INTERFACES.md` §6): spawns one meat pellet of `energy` at `pos`
    /// with the default minimum-size gate and no positional jitter.
    ///
    /// For corpse conversion use [`Self::spawn_meat_pile_with_radius`], which
    /// reproduces the game's `radius *= 0.1` jitter from the corpse half-length.
    pub fn spawn_meat_pile(&mut self, pos: Vec2, energy: f32, rng: &mut Rng) -> Option<PelletId> {
        self.spawn_meat_pile_with_radius(pos, energy, 0.0, rng, DEFAULT_MIN_PELLET_SIZE)
    }

    /// Creates a meat pile from corpse/pellet energy (`SpawnMeatPile`).
    ///
    /// The game always spawns a single meat pellet carrying all the energy when
    /// `total_energy >= Meat.EnergyDensity · min_pellet_size`; otherwise no
    /// pellet is created and the energy is lost. The spawn position is jittered
    /// by `±0.1·radius` on each axis (`radius` is the corpse half-length).
    pub fn spawn_meat_pile_with_radius(
        &mut self,
        pos: Vec2,
        total_energy: f32,
        radius: f32,
        rng: &mut Rng,
        min_pellet_size: f32,
    ) -> Option<PelletId> {
        if !total_energy.is_finite()
            || total_energy < self.materials.meat.energy_density * min_pellet_size
        {
            return None;
        }
        let jitter = radius * MEAT_PILE_RADIUS_FACTOR;
        let spawn_pos = pos
            + Vec2::new(
                rng.f32_range(-jitter, jitter),
                rng.f32_range(-jitter, jitter),
            );
        self.spawn_with_energy(MaterialKind::Meat, total_energy, spawn_pos)
    }

    /// Registers a pellet with the decay processor, resetting its fresh timer
    /// (`MatterDecayProcessor.TryAddUnique`).
    pub fn register_decay(&mut self, id: PelletId) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                let props = *self.materials.get(self.material[slot]);
                self.fresh_remaining[slot] = props.fresh_time;
                self.decay_amount[slot] = 0.0;
                self.decaying.set(slot, true);
                true
            }
            None => false,
        }
    }

    /// Removes a pellet from the decay processor, discarding its rot state.
    pub fn unregister_decay(&mut self, id: PelletId) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.decaying.set(slot, false);
                self.fresh_remaining[slot] = 0.0;
                self.decay_amount[slot] = 0.0;
                true
            }
            None => false,
        }
    }

    /// Applies a `freshTime` settings delta to every decaying pellet of a
    /// material (`MatterDecayProcessor.UpdateDecayParameters`). The decay rate is
    /// read live from the material table.
    pub fn update_decay_parameters(&mut self, material: MaterialKind, fresh_time_delta: f32) {
        for i in 0..self.dense.len() {
            let slot = self.dense[i] as usize;
            if self.material[slot] == material && self.decaying.get(slot) {
                self.fresh_remaining[slot] += fresh_time_delta;
            }
        }
    }

    /// `SettingsUpdater.UpdateDecay`: the Decay switch of `material` was flipped while the world runs. EVERY pellet of the material goes
    /// into the decay processor (`TryAddUnique`: the ones that were not in it yet, with a fresh timer from the material's freshness time and no
    /// rot) or out of it (`TryRemove`), not only the pellets spawned afterwards.
    pub fn set_material_decay(&mut self, material: MaterialKind, on: bool) {
        for i in 0..self.dense.len() {
            let slot = self.dense[i] as usize;
            if self.material[slot] != material {
                continue;
            }
            if on {
                if !self.decaying.get(slot) {
                    self.fresh_remaining[slot] = self.materials.get(material).fresh_time;
                    self.decay_amount[slot] = 0.0;
                    self.decaying.set(slot, true);
                }
            } else if self.decaying.get(slot) {
                self.decaying.set(slot, false);
                self.fresh_remaining[slot] = 0.0;
                self.decay_amount[slot] = 0.0;
            }
        }
    }

    /// A live edit of the materials (the settings panel, the scenario editor, undo): the table becomes `table`, and what `SettingsUpdater`
    /// does for Plant and Meat is done to the pellets that exist, in the order the game's settings fire their events (the Decay switch, the
    /// freshness time, the decay rate), each against the table as the game has it at that moment:
    ///
    /// * Decay switched: [`Self::set_material_decay`];
    /// * freshness time changed: its difference (new - old) is kept and added to the fresh timer of every decaying pellet of the material
    ///   (`UpdateFreshValueDifference`, `UpdateDecayParameters`);
    /// * decay rate changed: `UpdateDecayParameters` runs with the LAST kept difference again (the game's field `freshValueDifference` stays;
    ///   a rate edited after a freshness edit moves the timers once more). The rate itself is read live from the table.
    ///
    /// Fat and Armor have no such handlers in the game. After a change of an `energy_density` call [`Self::recompute_counts`].
    pub fn replace_materials(&mut self, table: MaterialTable) {
        for kind in [MaterialKind::Plant, MaterialKind::Meat] {
            let new = *table.get(kind);
            let old = *self.materials.get(kind);
            if new.decay != old.decay {
                self.materials.get_mut(kind).decay = new.decay;
                self.set_material_decay(kind, new.decay);
            }
            if new.fresh_time != old.fresh_time {
                self.materials.get_mut(kind).fresh_time = new.fresh_time;
                self.fresh_value_difference = new.fresh_time - old.fresh_time;
                self.update_decay_parameters(kind, self.fresh_value_difference);
            }
            if new.decay_rate != old.decay_rate {
                self.materials.get_mut(kind).decay_rate = new.decay_rate;
                self.update_decay_parameters(kind, self.fresh_value_difference);
            }
        }
        self.materials = table;
    }

    /// Runs one decay update over every registered pellet and returns the
    /// removed pellets/matter for conservation accounting.
    ///
    /// `dt_decay` must be `fixed_dt · decayTPS`; the caller advances a
    /// [`DecayThrottle`] to decide when to call this.
    ///
    /// Algorithm (`MatterDecayOfArrays.Process`):
    /// * `fresh -= dt`;
    /// * once not fresh, `rot += dt · decayRate`;
    /// * if `rot / amount > 5 %`, the accumulated rot is removed in one chop
    ///   (and `rot` resets to 0);
    /// * removal uses `min(rot, amount + 1e-6)`, so the pellet is destroyed when
    ///   rot reaches the remaining amount.
    pub fn process_decay(&mut self, dt_decay: f32) -> DecayReport {
        self.process_decay_with(dt_decay, crate::sim::ParallelExecutor::Serial, usize::MAX)
    }

    /// [`Self::process_decay`] with the scan over the pellets on the worker pool when the store has at least `from` slots (a world
    /// on the pool: `from` is [`crate::sim::ParallelGates::scans`]). Every pellet's decay reads and writes its own slot only, so a worker
    /// takes a stretch of the slot columns; the chops and the removals that follow stay in dense order on one thread, because the
    /// order the pellets are removed in decides which slots the next spawns get, and so every id of the world after it.
    pub fn process_decay_with(
        &mut self,
        dt_decay: f32,
        executor: crate::sim::ParallelExecutor<'_>,
        from: usize,
    ) -> DecayReport {
        if !dt_decay.is_finite() || dt_decay <= 0.0 {
            return DecayReport::default();
        }
        self.pending_removals.clear();
        let workers = crate::sim::ParallelGates::workers_for(
            self.generations.len(),
            from,
            executor.threads(),
        );
        if workers > 1 {
            self.scan_decay_pooled(dt_decay, workers, from);
        } else {
            for i in 0..self.dense.len() {
                let slot = self.dense[i] as usize;
                if !self.decaying.get(slot) {
                    continue;
                }
                self.fresh_remaining[slot] -= dt_decay;
                if self.fresh_remaining[slot] <= 0.0 {
                    let rate = self.materials.get(self.material[slot]).decay_rate;
                    self.decay_amount[slot] += dt_decay * rate;
                }
                let amount = self.amount[slot];
                let rot = self.decay_amount[slot];
                let remove = if amount > 0.0 {
                    rot / amount > DECAY_REMOVE_THRESHOLD
                } else {
                    true
                };
                if remove {
                    let to_remove = rot.min(amount + DECAY_REMOVE_EPSILON);
                    self.decay_amount[slot] = 0.0;
                    self.pending_removals.push((slot as u32, to_remove));
                }
            }
        }

        let mut report = DecayReport::default();
        for i in 0..self.pending_removals.len() {
            let (slot, to_remove) = self.pending_removals[i];
            let slot = slot as usize;
            if self.alive.get(slot) {
                let was_alive = self.amount[slot] > 0.0;
                let removed = to_remove.min(self.amount[slot].max(0.0));
                match self.material[slot] {
                    MaterialKind::Plant => report.plant_amount += f64::from(removed),
                    MaterialKind::Meat => report.meat_amount += f64::from(removed),
                    MaterialKind::Fat | MaterialKind::Armor => {}
                }
                self.remove_amount_by_slot(slot, to_remove);
                if was_alive && !self.alive.get(slot) {
                    report.removed_pellets += 1;
                }
            }
        }
        report
    }

    /// The first half of [`Self::process_decay_with`] on the pool: the freshness, the rot and the chop test of every decaying
    /// pellet, a stretch of the slot columns to a worker. Fills `pending_removals` with the chops in dense order, the order the
    /// serial scan finds them in.
    fn scan_decay_pooled(&mut self, dt_decay: f32, workers: usize, from: usize) {
        use rayon::prelude::*;
        let slots = self.generations.len();
        let chunk = crate::sim::ParallelGates::chunk_len(slots, from, workers, 4);
        let decaying = &self.decaying;
        let alive = &self.alive;
        let materials = &self.materials;
        let parts: Vec<Vec<(u32, f32)>> = self
            .fresh_remaining
            .par_chunks_mut(chunk)
            .zip(self.decay_amount.par_chunks_mut(chunk))
            .zip(self.amount.par_chunks(chunk))
            .zip(self.material.par_chunks(chunk))
            .enumerate()
            .map(|(index, (((fresh, rotten), amounts), kinds))| {
                let base = index * chunk;
                let mut removals: Vec<(u32, f32)> = Vec::new();
                for offset in 0..fresh.len() {
                    let slot = base + offset;
                    if !decaying.get(slot) || !alive.get(slot) {
                        continue;
                    }
                    fresh[offset] -= dt_decay;
                    if fresh[offset] <= 0.0 {
                        let rate = materials.get(kinds[offset]).decay_rate;
                        rotten[offset] += dt_decay * rate;
                    }
                    let amount = amounts[offset];
                    let rot = rotten[offset];
                    let remove = if amount > 0.0 {
                        rot / amount > DECAY_REMOVE_THRESHOLD
                    } else {
                        true
                    };
                    if remove {
                        let to_remove = rot.min(amount + DECAY_REMOVE_EPSILON);
                        rotten[offset] = 0.0;
                        removals.push((slot as u32, to_remove));
                    }
                }
                removals
            })
            .collect();
        for part in parts {
            self.pending_removals.extend(part);
        }
        // The serial scan walks the dense list: the chops go in that order.
        let dense_pos = &self.dense_pos;
        self.pending_removals
            .sort_unstable_by_key(|(slot, _)| dense_pos[*slot as usize]);
    }

    /// Converts a live pellet's state to a [`crate::pellet::Matter`] value.
    #[inline]
    pub fn matter(&self, id: PelletId) -> Option<Matter> {
        let slot = self.slot_of(id)?;
        Some(Matter::new(
            *self.materials.get(self.material[slot]),
            self.amount[slot],
        ))
    }

    /// Sets the position of a live pellet (public physics helper).
    #[inline]
    pub fn set_pos(&mut self, id: PelletId, pos: Vec2) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.pos[slot] = pos;
                self.mark_dirty(slot);
                true
            }
            None => false,
        }
    }

    /// Sets the linear velocity of a live pellet.
    #[inline]
    pub fn set_velocity(&mut self, id: PelletId, velocity: Vec2) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.velocity[slot] = velocity;
                self.mark_phys_dirty(slot);
                true
            }
            None => false,
        }
    }

    /// Adds to a pellet's linear velocity.
    #[inline]
    pub fn add_velocity(&mut self, id: PelletId, delta: Vec2) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.velocity[slot] += delta;
                self.mark_phys_dirty(slot);
                true
            }
            None => false,
        }
    }

    /// Sets the rotation of a live pellet in radians.
    #[inline]
    pub fn set_rotation(&mut self, id: PelletId, rotation: f32) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.rotation[slot] = rotation;
                self.mark_phys_dirty(slot);
                true
            }
            None => false,
        }
    }

    /// Sets the angular velocity of a live pellet in radians/s.
    #[inline]
    pub fn set_angular_velocity(&mut self, id: PelletId, angular_velocity: f32) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.angular_velocity[slot] = angular_velocity;
                self.mark_phys_dirty(slot);
                true
            }
            None => false,
        }
    }

    /// Writes the pose the physics step computed for a pellet. Like [`Self::set_pose`], except that it does not tell the
    /// physics bridge the pellet changed (the bridge made the change itself).
    #[inline]
    pub(crate) fn set_pose_from_physics(&mut self, id: PelletId, pose: PelletPose) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                if self.pos[slot].x.to_bits() != pose.pos.x.to_bits()
                    || self.pos[slot].y.to_bits() != pose.pos.y.to_bits()
                {
                    self.mark_hash_dirty(slot);
                }
                self.pos[slot] = pose.pos;
                self.velocity[slot] = pose.velocity;
                self.rotation[slot] = pose.rotation;
                self.angular_velocity[slot] = pose.angular_velocity;
                true
            }
            None => false,
        }
    }

    /// Mutable pose view of a live pellet, for the physics solver.
    ///
    /// `physics.rs` (body-dev) consumes this together with
    /// [`Self::for_each_pose_mut`]; until that module lands it is exercised by
    /// the unit tests, hence the `dead_code` allowance.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn pose_mut(&mut self, id: PelletId) -> Option<PoseMut<'_>> {
        let slot = self.slot_of(id)?;
        self.mark_dirty(slot);
        Some(PoseMut {
            pos: &mut self.pos[slot],
            velocity: &mut self.velocity[slot],
            rotation: &mut self.rotation[slot],
            angular_velocity: &mut self.angular_velocity[slot],
        })
    }

    /// Calls `f` with a mutable pose view for every live pellet, in dense order.
    ///
    /// Physics-only helper (`pub(crate)`): amount/material/owner mutations must
    /// still go through the public choke points.
    #[allow(dead_code)]
    pub(crate) fn for_each_pose_mut(&mut self, mut f: impl FnMut(PelletId, PoseMut<'_>)) {
        for i in 0..self.dense.len() {
            let slot = self.dense[i] as usize;
            let id = self.id_at_slot(slot);
            self.mark_dirty(slot);
            let pose = PoseMut {
                pos: &mut self.pos[slot],
                velocity: &mut self.velocity[slot],
                rotation: &mut self.rotation[slot],
                angular_velocity: &mut self.angular_velocity[slot],
            };
            f(id, pose);
        }
    }

    #[inline]
    fn id_at_slot(&self, slot: usize) -> PelletId {
        PelletId::from_handle(Handle::new(slot as u32, self.generations[slot]))
    }

    #[inline]
    fn slot_of(&self, id: PelletId) -> Option<usize> {
        let handle = id.handle();
        let slot = handle.slot() as usize;
        if slot < self.generations.len()
            && self.alive.get(slot)
            && self.generations[slot] == handle.generation()
        {
            Some(slot)
        } else {
            None
        }
    }

    #[inline]
    fn energy_at(&self, slot: usize) -> f32 {
        self.amount[slot] * self.materials.get(self.material[slot]).energy_density
    }

    #[inline]
    fn mass_at(&self, slot: usize) -> f32 {
        (self.amount[slot] * self.materials.get(self.material[slot]).mass_density).abs()
    }

    #[inline]
    fn counts_add(&mut self, material: MaterialKind, energy_delta: f64) {
        match material {
            MaterialKind::Plant => {
                self.counts.plant_count = self.counts.plant_count.wrapping_add(1);
                self.counts.plant_biomass += energy_delta;
            }
            MaterialKind::Meat => {
                self.counts.meat_count = self.counts.meat_count.wrapping_add(1);
                self.counts.meat_biomass += energy_delta;
            }
            MaterialKind::Fat | MaterialKind::Armor => {}
        }
    }

    #[inline]
    fn counts_add_energy(&mut self, material: MaterialKind, energy_delta: f64) {
        match material {
            MaterialKind::Plant => self.counts.plant_biomass += energy_delta,
            MaterialKind::Meat => self.counts.meat_biomass += energy_delta,
            MaterialKind::Fat | MaterialKind::Armor => {}
        }
    }

    #[inline]
    fn set_amount_by_slot(&mut self, slot: usize, amount: f32) {
        let density = self.materials.get(self.material[slot]).energy_density;
        let delta = amount - self.amount[slot];
        self.amount[slot] = amount;
        self.mark_dirty(slot);
        self.counts_add_energy(self.material[slot], (delta * density) as f64);
    }

    fn remove_amount_by_slot(&mut self, slot: usize, max_amount: f32) {
        if !max_amount.is_finite() || max_amount <= 0.0 {
            return;
        }
        let current = self.amount[slot];
        if current <= max_amount {
            self.set_amount_by_slot(slot, 0.0);
            self.destroy_slot(slot);
        } else {
            self.set_amount_by_slot(slot, current - max_amount);
        }
    }

    fn destroy_slot(&mut self, slot: usize) {
        if !self.alive.get(slot) {
            return;
        }
        self.mark_dirty(slot);
        // Counters: count-- and biomass -= current energy (0 when the
        // RemoveAmount path already zeroed the amount), mirroring
        // PlantCounter/MeatCounter.OnDestroy.
        let energy = self.energy_at(slot) as f64;
        match self.material[slot] {
            MaterialKind::Plant => {
                self.counts.plant_count = self.counts.plant_count.saturating_sub(1);
                self.counts.plant_biomass -= energy;
            }
            MaterialKind::Meat => {
                self.counts.meat_count = self.counts.meat_count.saturating_sub(1);
                self.counts.meat_biomass -= energy;
            }
            MaterialKind::Fat | MaterialKind::Armor => {}
        }

        let position = self.dense_pos[slot] as usize;
        self.dense.swap_remove(position);
        if let Some(&moved) = self.dense.get(position) {
            self.dense_pos[moved as usize] = position as u32;
        }
        self.dense_pos[slot] = u32::MAX;

        self.alive.set(slot, false);
        self.held.set(slot, false);
        self.decaying.set(slot, false);
        self.amount[slot] = 0.0;
        self.owner[slot] = None;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        self.free.push(slot as u32);
    }

    fn push_slot(&mut self) {
        let slot = self.amount.len();
        self.amount.push(0.0);
        self.material.push(MaterialKind::Plant);
        self.pos.push(Vec2::ZERO);
        self.velocity.push(Vec2::ZERO);
        self.rotation.push(0.0);
        self.angular_velocity.push(0.0);
        self.owner.push(None);
        self.fresh_remaining.push(0.0);
        self.decay_amount.push(0.0);
        self.drag_factor.push(1.0);
        self.generations.push(0);
        self.dense_pos.push(u32::MAX);
        self.held.ensure(slot + 1);
        self.decaying.ensure(slot + 1);
        self.alive.ensure(slot + 1);
        self.hash_dirty_bits.ensure(slot + 1);
        self.phys_dirty_bits.ensure(slot + 1);
    }

    /// Notes that the pellet in `slot` may no longer match what was derived from it for the physics step.
    #[inline]
    fn mark_phys_dirty(&mut self, slot: usize) {
        if !self.phys_dirty_bits.get(slot) {
            self.phys_dirty_bits.set(slot, true);
            self.phys_dirty.push(slot as u32);
        }
    }

    /// Notes a change that matters to both the spatial hash and the physics step.
    #[inline]
    fn mark_dirty(&mut self, slot: usize) {
        self.mark_hash_dirty(slot);
        self.mark_phys_dirty(slot);
    }

    /// Moves the slots written since the last call (by anyone but the physics step's own write-back) into `out`, replacing
    /// its contents.
    pub(crate) fn take_phys_dirty(&mut self, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.phys_dirty);
        for &slot in &self.phys_dirty {
            self.phys_dirty_bits.set(slot as usize, false);
        }
        self.phys_dirty.clear();
    }

    /// Changes when the whole store is replaced (a checkpoint restore).
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The slots of the live pellets in dense order.
    pub(crate) fn dense_slots(&self) -> &[u32] {
        &self.dense
    }

    /// The id of the live pellet in `slot`.
    pub(crate) fn live_id(&self, slot: usize) -> Option<PelletId> {
        self.alive.get(slot).then(|| self.id_at_slot(slot))
    }

    /// Notes that the pellet in `slot` may no longer match its spatial-hash entry (position, radius, or being alive).
    #[inline]
    fn mark_hash_dirty(&mut self, slot: usize) {
        if !self.hash_dirty_bits.get(slot) {
            self.hash_dirty_bits.set(slot, true);
            self.hash_dirty.push(slot as u32);
        }
    }

    /// Moves the slots that may differ from the spatial hash since the last call into `out` (replacing its contents) and
    /// forgets them: the world's hash sync visits these instead of every pellet.
    pub(crate) fn take_hash_dirty(&mut self, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.hash_dirty);
        for &slot in &self.hash_dirty {
            self.hash_dirty_bits.set(slot as usize, false);
        }
        self.hash_dirty.clear();
    }

    /// Forgets which slots differ from the spatial hash (the hash was just restored or rebuilt from the whole store).
    pub(crate) fn clear_hash_dirty(&mut self) {
        for &slot in &self.hash_dirty {
            self.hash_dirty_bits.set(slot as usize, false);
        }
        self.hash_dirty.clear();
    }

    /// What the spatial hash holds for the live pellet in `slot`: its id, position, and bounding radius; with its dense index.
    pub(crate) fn hash_entry(&self, slot: usize) -> Option<(PelletId, Vec2, f32)> {
        if !self.alive.get(slot) {
            return None;
        }
        Some((
            self.id_at_slot(slot),
            self.pos[slot],
            (self.amount[slot].max(0.0) / PI).sqrt(),
        ))
    }

    /// Position of the pellet in `slot` among the live ones (the order of [`Self::ids`]); `None` if it is not live.
    pub(crate) fn dense_position(&self, slot: usize) -> Option<u32> {
        self.alive
            .get(slot)
            .then(|| self.dense_pos.get(slot).copied())
            .flatten()
    }
}

impl Default for PelletStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the analytic matter radius `sqrt(amount/π)`, clamped to 0.
#[inline]
pub fn matter_radius(amount: f32) -> f32 {
    (amount.max(0.0) / PI).sqrt()
}

/// Returns `true` when the game's `Mathf.Approximately(amount, 0)` would treat
/// the amount as empty (used by pellet eligibility filters).
#[inline]
pub fn is_negligible_amount(amount: f32) -> bool {
    amount.abs() <= 1e-5
}

/// Clamps a matter amount to a non-negative finite value (loader utility).
#[inline]
pub fn sanitize_amount(amount: f32) -> f32 {
    if amount.is_finite() {
        amount.max(0.0)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Checkpoint P2: pellet store arena/runtime bytes (task-103)
// ---------------------------------------------------------------------------

impl PelletStore {
    /// Captures the full arena/runtime state (slot/generation/free/dense layout
    /// plus per-pellet runtime fields) as deterministic little-endian bytes.
    #[must_use]
    pub fn capture_checkpoint(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, self.amount.len() as u32);
        for slot in 0..self.amount.len() {
            let alive = self.alive.get(slot);
            out.push(u8::from(alive));
            // Dead-slot generations are allocator state: the next spawn reuses
            // the slot and must produce the same PelletId generation.
            push_u32(&mut out, self.generations[slot]);
            if !alive {
                continue;
            }
            push_f32(&mut out, self.amount[slot]);
            out.push(material_code(self.material[slot]));
            push_vec2(&mut out, self.pos[slot]);
            push_vec2(&mut out, self.velocity[slot]);
            push_f32(&mut out, self.rotation[slot]);
            push_f32(&mut out, self.angular_velocity[slot]);
            out.push(u8::from(self.held.get(slot)));
            match self.owner[slot] {
                Some(zone) => {
                    out.push(1);
                    push_u32(&mut out, zone.handle().slot());
                    push_u32(&mut out, zone.handle().generation());
                }
                None => out.push(0),
            }
            out.push(u8::from(self.decaying.get(slot)));
            push_f32(&mut out, self.fresh_remaining[slot]);
            push_f32(&mut out, self.decay_amount[slot]);
            push_f32(&mut out, self.drag_factor[slot]);
        }
        push_u32(&mut out, self.free.len() as u32);
        for slot in &self.free {
            push_u32(&mut out, *slot);
        }
        push_u32(&mut out, self.dense.len() as u32);
        for slot in &self.dense {
            push_u32(&mut out, *slot);
        }
        push_u32(&mut out, self.dense_pos.len() as u32);
        for position in &self.dense_pos {
            push_u32(&mut out, *position);
        }
        out
    }

    /// Restores state produced by [`PelletStore::capture_checkpoint`].
    pub fn restore_checkpoint(&mut self, bytes: &[u8]) -> crate::error::Result<()> {
        let mut reader = PelletCheckpointReader { bytes, pos: 0 };
        let capacity = reader.u32()? as usize;
        // The world restores its hash from the same checkpoint (or rebuilds it from the whole store): nothing is pending.
        self.hash_dirty.clear();
        self.hash_dirty_bits = BitSet::new();
        self.phys_dirty.clear();
        self.phys_dirty_bits = BitSet::new();
        self.epoch += 1;
        self.amount = vec![0.0; capacity];
        self.material = vec![MaterialKind::Plant; capacity];
        self.pos = vec![Vec2::ZERO; capacity];
        self.velocity = vec![Vec2::ZERO; capacity];
        self.rotation = vec![0.0; capacity];
        self.angular_velocity = vec![0.0; capacity];
        self.held = BitSet::new();
        self.owner = vec![None; capacity];
        self.decaying = BitSet::new();
        self.fresh_remaining = vec![0.0; capacity];
        self.decay_amount = vec![0.0; capacity];
        self.drag_factor = vec![1.0; capacity];
        self.generations = vec![0; capacity];
        self.alive = BitSet::new();
        for slot in 0..capacity {
            let alive = reader.bool()?;
            self.alive.set(slot, alive);
            self.generations[slot] = reader.u32()?;
            if !alive {
                continue;
            }
            self.amount[slot] = reader.f32()?;
            self.material[slot] = material_from_code(reader.u8()?)?;
            self.pos[slot] = reader.vec2()?;
            self.velocity[slot] = reader.vec2()?;
            self.rotation[slot] = reader.f32()?;
            self.angular_velocity[slot] = reader.f32()?;
            self.held.set(slot, reader.bool()?);
            self.owner[slot] = if reader.u8()? == 1 {
                Some(ZoneId::from_handle(Handle::new(
                    reader.u32()?,
                    reader.u32()?,
                )))
            } else {
                None
            };
            self.decaying.set(slot, reader.bool()?);
            self.fresh_remaining[slot] = reader.f32()?;
            self.decay_amount[slot] = reader.f32()?;
            self.drag_factor[slot] = reader.f32()?;
        }
        let free_count = reader.u32()? as usize;
        let mut free = Vec::with_capacity(free_count);
        for _ in 0..free_count {
            free.push(reader.u32()?);
        }
        let dense_count = reader.u32()? as usize;
        let mut dense = Vec::with_capacity(dense_count);
        for _ in 0..dense_count {
            dense.push(reader.u32()?);
        }
        let dense_pos_count = reader.u32()? as usize;
        let mut dense_pos = Vec::with_capacity(dense_pos_count);
        for _ in 0..dense_pos_count {
            dense_pos.push(reader.u32()?);
        }
        if !reader.finished() {
            return Err(crate::error::Error::invalid_data(
                "trailing bytes in pellet checkpoint",
            ));
        }
        if dense_pos.len() != capacity
            || dense.iter().any(|slot| *slot as usize >= capacity)
            || free.iter().any(|slot| *slot as usize >= capacity)
            || dense.len() + free.len() != capacity
        {
            return Err(crate::error::Error::invalid_data(format!(
                "invalid pellet checkpoint arena layout: capacity={capacity} dense={} free={} dense_pos={} bad_dense={} bad_free={}",
                dense.len(),
                free.len(),
                dense_pos.len(),
                dense
                    .iter()
                    .filter(|slot| **slot as usize >= capacity)
                    .count(),
                free.iter()
                    .filter(|slot| **slot as usize >= capacity)
                    .count(),
            )));
        }
        for (position, slot) in dense.iter().enumerate() {
            if dense_pos.get(*slot as usize).copied() != Some(position as u32) {
                return Err(crate::error::Error::invalid_data(
                    "inconsistent pellet checkpoint dense positions",
                ));
            }
        }
        for slot in &free {
            if dense_pos.get(*slot as usize).copied() != Some(u32::MAX) {
                return Err(crate::error::Error::invalid_data(
                    "free pellet slot has a dense position",
                ));
            }
        }
        self.free = free;
        self.dense = dense;
        self.dense_pos = dense_pos;
        self.pending_removals.clear();
        self.counts = PelletCounts::default();
        for slot in 0..capacity {
            if !self.alive.get(slot) {
                continue;
            }
            let energy = self.energy_at(slot) as f64;
            match self.material[slot] {
                MaterialKind::Plant => {
                    self.counts.plant_count += 1;
                    self.counts.plant_biomass += energy;
                }
                MaterialKind::Meat => {
                    self.counts.meat_count += 1;
                    self.counts.meat_biomass += energy;
                }
                MaterialKind::Fat | MaterialKind::Armor => {}
            }
        }
        Ok(())
    }
}

fn material_code(material: MaterialKind) -> u8 {
    match material {
        MaterialKind::Plant => 0,
        MaterialKind::Meat => 1,
        MaterialKind::Fat => 2,
        MaterialKind::Armor => 3,
    }
}

fn material_from_code(code: u8) -> crate::error::Result<MaterialKind> {
    match code {
        0 => Ok(MaterialKind::Plant),
        1 => Ok(MaterialKind::Meat),
        2 => Ok(MaterialKind::Fat),
        3 => Ok(MaterialKind::Armor),
        _ => Err(crate::error::Error::invalid_data(
            "invalid material code in pellet checkpoint",
        )),
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_f32(out: &mut Vec<u8>, value: f32) {
    push_u32(out, value.to_bits());
}
fn push_vec2(out: &mut Vec<u8>, value: Vec2) {
    push_f32(out, value.x);
    push_f32(out, value.y);
}

struct PelletCheckpointReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl PelletCheckpointReader<'_> {
    fn take(&mut self, len: usize) -> crate::error::Result<&[u8]> {
        let end = self.pos.checked_add(len).ok_or_else(|| {
            crate::error::Error::invalid_data("pellet checkpoint length overflow")
        })?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| crate::error::Error::invalid_data("truncated pellet checkpoint"))?;
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> crate::error::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> crate::error::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(crate::error::Error::invalid_data(
                "invalid bool in pellet checkpoint",
            )),
        }
    }
    fn u32(&mut self) -> crate::error::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn f32(&mut self) -> crate::error::Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn vec2(&mut self) -> crate::error::Result<Vec2> {
        Ok(Vec2::new(self.f32()?, self.f32()?))
    }
    fn finished(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn almost(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4
    }

    fn almost_vec(a: Vec2, b: Vec2) -> bool {
        almost(a.x, b.x) && almost(a.y, b.y)
    }

    fn almost64(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-6
    }

    fn pellet(store: &mut PelletStore, amount: f32, x: f32, y: f32) -> PelletId {
        store.spawn(PelletSpawn::new(
            MaterialKind::Plant,
            amount,
            Vec2::new(x, y),
        ))
    }

    #[test]
    fn checkpoint_preserves_dead_slot_generations() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 100.0, 1.0, 2.0);
        store.remove(id);
        let bytes = store.capture_checkpoint();
        let mut restored = PelletStore::new();
        restored.restore_checkpoint(&bytes).expect("restore");
        assert_eq!(restored.capture_checkpoint(), bytes);
        // The next allocation reuses the dead slot; its generation must match.
        let a = pellet(&mut store, 50.0, 3.0, 4.0);
        let b = pellet(&mut restored, 50.0, 3.0, 4.0);
        assert_eq!(a.handle().slot(), b.handle().slot());
        assert_eq!(a.handle().generation(), b.handle().generation());
    }

    #[test]
    fn material_defaults_match_captured_table() {
        let plant = MaterialProperties::PLANT;
        assert_eq!(plant.cohesiveness, 10.0);
        assert!(almost(plant.reactivity, 0.03));
        assert_eq!(plant.energy_density, 5.0);
        assert!(almost(plant.mass_density, 0.015));
        assert!(almost(plant.min_efficiency, -0.25));
        assert_eq!(plant.max_efficiency, 0.5);
        assert!(!plant.decay);

        let meat = MaterialProperties::MEAT;
        assert_eq!(meat.cohesiveness, 35.0);
        assert!(almost(meat.reactivity, 0.075));
        assert_eq!(meat.energy_density, 10.0);
        assert!(almost(meat.mass_density, 0.025));
        assert!(almost(meat.min_efficiency, 0.15));
        assert!(almost(meat.max_efficiency, 0.85));
        assert!(meat.decay);
        assert_eq!(meat.fresh_time, 2000.0);
        assert!(almost(meat.decay_rate, 0.02));

        assert_eq!(MaterialProperties::FAT.energy_density, 30.0);
        assert_eq!(MaterialProperties::ARMOR.cohesiveness, 500.0);
        assert!(MaterialKind::Plant.is_world_pellet());
        assert!(MaterialKind::Meat.is_world_pellet());
        assert!(!MaterialKind::Fat.is_world_pellet());
        assert!(!MaterialKind::Armor.is_world_pellet());
    }

    #[test]
    fn matter_formulas() {
        let matter = Matter::new(MaterialProperties::PLANT, 40.0);
        assert!(almost(matter.radius(), (40.0f32 / PI).sqrt()));
        assert!(almost(matter.energy(), 200.0));
        assert!(almost(matter.mass(), 0.6));
        assert!(almost(matter.cohesiveness(), 10.0));
        // Infinite-cap matter has infinite `available`, so it is never "full"
        // (Mathf.Approximately(inf, 0) is false in the game too).
        assert!(!matter.is_full());
        assert!(almost(matter.overflow(), 0.0));
        assert_eq!(matter.available(), f32::INFINITY);

        let mut finite = Matter::with_max(MaterialProperties::MEAT, 5.0, 20.0);
        assert!(almost(finite.fullness(), 0.25));
        assert!(almost(finite.available(), 15.0));
        finite.amount = 25.0;
        assert!(almost(finite.overflow(), 5.0));

        let mut m = Matter::new(MaterialProperties::PLANT, 10.0);
        m.set_energy(50.0);
        assert!(almost(m.amount, 10.0));
        assert!(almost(m.energy(), 50.0));
    }

    #[test]
    fn conversion_helpers() {
        let plant = MaterialProperties::PLANT;
        assert!(almost(plant.conversion_efficiency(0.0), -0.25));
        assert!(almost(plant.conversion_efficiency(1.0), 0.5));
        assert!(almost(plant.conversion_efficiency(0.5), 0.125));
        assert!(almost(plant.conversion_efficiency(-2.0), -0.25));
        assert!(almost(plant.conversion_efficiency(2.0), 0.5));
        assert!(almost(plant.energy_of_amount(40.0), 200.0));
        assert!(almost(plant.amount_of_energy(200.0), 40.0));
        assert!(almost(plant.radius_of_amount(40.0), (40.0f32 / PI).sqrt()));
        assert!(almost(plant.amount_of_radius(3.0), PI * 9.0));
        assert!(almost(plant.normal_energy_at_scale1(), 100.0));

        let mut matter = Matter::new(MaterialProperties::PLANT, 10.0);
        // Add 10 u² of plant at affinity 1 (efficiency 0.5): 50 E stored but
        // 50/0.5 = 100 E of energy must be paid.
        let cost = matter.convert_to_from_energy(10.0, 1.0);
        assert!(almost(matter.amount, 20.0));
        assert!(almost(cost, -100.0));
        // Remove 20 u² (100 E) at efficiency 0.5 -> 50 E gained.
        let gain = matter.convert_to_from_energy(-20.0, 1.0);
        assert!(almost(matter.amount, 0.0));
        assert!(almost(gain, 50.0));
        let mut capped = Matter::with_max(MaterialProperties::PLANT, 5.0, 10.0);
        capped.convert_to_from_energy(100.0, 0.0);
        assert!(almost(capped.amount, 10.0));
    }

    #[test]
    fn spawn_sets_defaults_and_counts() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 40.0, 1.0, 2.0);
        assert_eq!(store.len(), 1);
        assert!(store.contains(id));
        let view = store.get(id).unwrap();
        assert!(almost(view.amount, 40.0));
        assert_eq!(view.material, MaterialKind::Plant);
        assert!(almost(view.radius, (40.0f32 / PI).sqrt()));
        assert!(almost(store.energy(id).unwrap(), 200.0));
        assert!(almost(store.size_factor(id).unwrap(), 2.0));
        assert!(almost(store.scale(id).unwrap(), 2.0f32.sqrt()));
        assert!(almost(store.mass(id).unwrap(), 0.6));
        assert!(almost_vec(store.pos(id).unwrap(), Vec2::new(1.0, 2.0)));
        let counts = store.counts();
        assert_eq!(counts.plant_count, 1);
        assert_eq!(counts.meat_count, 0);
        assert!(almost(counts.plant_biomass as f32, 200.0));
    }

    #[test]
    fn spawn_rejects_invalid_inputs() {
        let mut store = PelletStore::new();
        assert!(
            store
                .try_spawn(PelletSpawn::new(MaterialKind::Fat, 1.0, Vec2::ZERO))
                .is_none()
        );
        assert!(
            store
                .try_spawn(PelletSpawn::new(MaterialKind::Plant, 0.0, Vec2::ZERO))
                .is_none()
        );
        assert!(
            store
                .try_spawn(PelletSpawn::new(MaterialKind::Plant, f32::NAN, Vec2::ZERO))
                .is_none()
        );
        assert!(
            store
                .try_spawn(PelletSpawn::new(
                    MaterialKind::Plant,
                    1.0,
                    Vec2::new(f32::NAN, 0.0)
                ))
                .is_none()
        );
        assert!(
            store
                .spawn_with_energy(MaterialKind::Meat, -1.0, Vec2::ZERO)
                .is_none()
        );
    }

    #[test]
    fn spawn_with_energy_converts() {
        let mut store = PelletStore::new();
        let id = store
            .spawn_with_energy(MaterialKind::Meat, 200.0, Vec2::ZERO)
            .unwrap();
        assert!(almost(store.amount(id).unwrap(), 20.0));
        assert!(almost(store.energy(id).unwrap(), 200.0));
        let counts = store.counts();
        assert_eq!(counts.meat_count, 1);
        assert!(almost(counts.meat_biomass as f32, 200.0));
    }

    #[test]
    fn change_amount_is_the_choke_point() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 40.0, 0.0, 0.0);
        assert!(almost(store.change_amount(id, 10.0), 10.0));
        assert!(almost(store.amount(id).unwrap(), 50.0));
        assert!(almost(store.change_amount(id, -20.0), -20.0));
        assert!(almost(store.amount(id).unwrap(), 30.0));
        assert!(almost(store.counts().plant_biomass as f32, 150.0));
        // Removing more than the remaining amount destroys it and reports the
        // actual (negative) amount removed.
        assert!(almost(store.change_amount(id, -1000.0), -30.0));
        assert!(!store.contains(id));
        assert_eq!(store.counts().plant_count, 0);
        assert!(almost(store.counts().plant_biomass as f32, 0.0));
        // No-op changes.
        assert_eq!(store.change_amount(id, 5.0), 0.0);
        assert_eq!(store.change_amount(id, f32::NAN), 0.0);
    }

    #[test]
    fn remove_amount_and_remove_energy() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 40.0, 0.0, 0.0);
        assert!(almost(store.remove_amount(id, 10.0), 10.0));
        assert!(almost(store.amount(id).unwrap(), 30.0));
        assert!(almost(store.counts().plant_biomass as f32, 150.0));

        let id2 = store
            .spawn_with_energy(MaterialKind::Plant, 200.0, Vec2::ZERO)
            .unwrap();
        assert!(almost(store.remove_energy(id2, 50.0), 50.0));
        assert!(almost(store.amount(id2).unwrap(), 30.0));
        assert!(almost(store.energy(id2).unwrap(), 150.0));
    }

    #[test]
    fn amount_setters_cannot_corrupt_store() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 20.0, 0.0, 0.0);
        store.set_amount(id, 40.0);
        assert!(almost(store.amount(id).unwrap(), 40.0));
        assert!(almost(store.counts().plant_biomass as f32, 200.0));
        store.add_amount(id, 10.0);
        assert!(almost(store.amount(id).unwrap(), 50.0));
        assert!(almost(store.counts().plant_biomass as f32, 250.0));
        store.set_amount(id, -5.0);
        assert!(almost(store.amount(id).unwrap(), 50.0));
        store.set_owner_zone(id, None);
        assert_eq!(store.owner_zone(id), Some(None));
    }

    #[test]
    fn handles_become_stale_and_slots_are_reused() {
        let mut store = PelletStore::new();
        let a = pellet(&mut store, 10.0, 0.0, 0.0);
        store.remove(a);
        let b = pellet(&mut store, 20.0, 0.0, 0.0);
        assert_eq!(a.handle().slot(), b.handle().slot());
        assert_ne!(a.handle().generation(), b.handle().generation());
        assert!(!store.contains(a));
        assert!(store.contains(b));
        assert!(store.amount(a).is_none());
        assert_eq!(store.slot_capacity(), 1);
    }

    #[test]
    fn dense_iteration_is_deterministic() {
        let mut store = PelletStore::new();
        let ids: Vec<_> = (0..10)
            .map(|i| pellet(&mut store, 20.0, i as f32, 0.0))
            .collect();
        store.remove(ids[3]);
        store.remove(ids[7]);
        assert_eq!(store.ids().count(), 8);
        let seen: Vec<u32> = store.ids().map(|id| id.handle().slot()).collect();

        let mut store2 = PelletStore::new();
        let ids2: Vec<_> = (0..10)
            .map(|i| pellet(&mut store2, 20.0, i as f32, 0.0))
            .collect();
        store2.remove(ids2[3]);
        store2.remove(ids2[7]);
        let seen2: Vec<u32> = store2.ids().map(|id| id.handle().slot()).collect();
        assert_eq!(seen, seen2);
    }

    #[test]
    fn physics_pose_helpers() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 20.0, 0.0, 0.0);
        {
            let pose = store.pose_mut(id).unwrap();
            pose.pos.y += 1.0;
            pose.velocity.x = 2.0;
        }
        store.for_each_pose_mut(|_, pose| {
            *pose.rotation += 0.5;
            *pose.angular_velocity = 1.0;
        });
        assert!(almost(store.pos(id).unwrap().y, 1.0));
        assert!(almost(store.velocity(id).unwrap().x, 2.0));
        assert!(almost(store.rotation(id).unwrap(), 0.5));
        assert!(almost(store.angular_velocity(id).unwrap(), 1.0));
        assert!(store.set_pose(
            id,
            PelletPose {
                pos: Vec2::new(3.0, 4.0),
                velocity: Vec2::ZERO,
                rotation: 0.0,
                angular_velocity: 0.0,
            }
        ));
        assert!(almost(store.pos(id).unwrap().x, 3.0));
    }

    #[test]
    fn drag_and_damping_formulas() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 40.0, 0.0, 0.0);
        let expected = 5.0 * 2.0f32.sqrt() / 0.6;
        assert!(almost(store.linear_drag(id, 5.0, false).unwrap(), expected));
        store.set_held(id, true);
        assert_eq!(store.linear_drag(id, 5.0, true).unwrap(), 0.0);
        assert!(almost(store.linear_drag(id, 5.0, false).unwrap(), expected));
        assert!(almost(PelletStore::angular_damping(5.0), 2.5));
    }

    #[test]
    fn the_drag_factor_is_one_for_a_new_pellet_can_be_set_and_survives_a_checkpoint_and_a_reused_slot() {
        let mut store = PelletStore::new();
        let a = pellet(&mut store, 40.0, 0.0, 0.0);
        let b = pellet(&mut store, 20.0, 1.0, 0.0);
        assert_eq!((store.drag_factor(a), store.drag_factor(b)), (Some(1.0), Some(1.0)));
        assert!(store.set_drag_factor(a, 0.25));
        assert!(!store.set_drag_factor(a, 0.0) && !store.set_drag_factor(a, f32::NAN) && !store.set_drag_factor(a, -1.0), "not a positive number: left as it was");
        assert_eq!((store.drag_factor(a), store.drag_factor(b)), (Some(0.25), Some(1.0)));
        let mut restored = PelletStore::new();
        restored.restore_checkpoint(&store.capture_checkpoint()).expect("restore");
        assert_eq!((restored.drag_factor(a), restored.drag_factor(b)), (Some(0.25), Some(1.0)));
        // A pellet that goes and a new one in its slot: the newcomer has the game's drag.
        store.remove(a);
        let c = pellet(&mut store, 10.0, 2.0, 0.0);
        assert_eq!(store.drag_factor(c), Some(1.0));
        assert_eq!(store.drag_factor(a), None, "the old id is gone");
    }

    #[test]
    fn merge_conserves_energy_and_momentum() {
        let mut store = PelletStore::new();
        let a = pellet(&mut store, 40.0, 0.0, 0.0);
        let b = pellet(&mut store, 20.0, 1.0, 0.0);
        store.set_velocity(a, Vec2::new(1.0, 0.0));
        store.set_velocity(b, Vec2::new(-1.0, 0.0));
        let biomass_before = store.counts().plant_biomass;

        assert_eq!(store.try_merge(a, b), Some(a));
        assert!(!store.contains(b));
        assert!(almost(store.amount(a).unwrap(), 60.0));
        assert!(almost(store.velocity(a).unwrap().x, 1.0 / 3.0));
        assert!(almost64(store.counts().plant_biomass, biomass_before));
        assert_eq!(store.counts().plant_count, 1);
    }

    #[test]
    fn merge_rejects_wrong_direction_or_material() {
        let mut store = PelletStore::new();
        let a = pellet(&mut store, 10.0, 0.0, 0.0);
        let b = pellet(&mut store, 20.0, 0.0, 0.0);
        assert!(store.try_merge(a, b).is_none());
        let meat = store.spawn(PelletSpawn::new(MaterialKind::Meat, 5.0, Vec2::ZERO));
        let plant = pellet(&mut store, 50.0, 0.0, 0.0);
        assert!(store.try_merge(plant, meat).is_none());
        assert!(store.try_merge(plant, plant).is_none());
    }

    #[test]
    fn rip_chunk_splits_amounts_and_places_chunk() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 40.0, 0.0, 0.0);
        let mut rng = Rng::from_seed(7);
        let result = store
            .rip_chunk(id, 10.0, Some(Vec2::new(1.0, 0.0)), 0.1, &mut rng)
            .expect("rip");
        assert!(almost(result.removed_amount, 10.0));
        assert!(almost(store.amount(id).unwrap(), 30.0));
        assert!(almost(store.amount(result.new_pellet).unwrap(), 10.0));
        let src = store.pos(id).unwrap();
        let chunk = store.pos(result.new_pellet).unwrap();
        assert!(chunk.x > src.x);
        assert!(almost(src.y, chunk.y));
        let counts = store.counts();
        assert_eq!(counts.plant_count, 2);
        assert!(almost(counts.plant_biomass as f32, 200.0));
    }

    #[test]
    fn rip_chunk_rejects_small_pieces() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 1.0, 0.0, 0.0);
        let mut rng = Rng::from_seed(7);
        // Chunk below minPelletSize.
        assert!(store.rip_chunk(id, 0.05, None, 0.1, &mut rng).is_none());
        // Remaining pellet would be below minPelletSize.
        assert!(store.rip_chunk(id, 0.95, None, 0.1, &mut rng).is_none());
        // Non-positive chunk.
        assert!(store.rip_chunk(id, 0.0, None, 0.1, &mut rng).is_none());
        // Equal halves are allowed.
        let result = store
            .rip_chunk(id, 0.5, None, 0.1, &mut rng)
            .expect("equal halves are valid");
        assert!(almost(result.removed_amount, 0.5));
        assert!(almost(store.amount(id).unwrap(), 0.5));
    }

    #[test]
    fn meat_pile_contract_and_radius_variant() {
        let mut store = PelletStore::new();
        let mut rng = Rng::from_seed(11);
        let id = store
            .spawn_meat_pile(Vec2::new(10.0, 10.0), 300.0, &mut rng)
            .expect("meat pile");
        assert!(almost(store.energy(id).unwrap(), 300.0));
        assert!(almost(store.amount(id).unwrap(), 30.0));
        assert!(almost_vec(store.pos(id).unwrap(), Vec2::new(10.0, 10.0)));
        assert!(store.spawn_meat_pile(Vec2::ZERO, 0.5, &mut rng).is_none());

        let mut rng = Rng::from_seed(11);
        let id = store
            .spawn_meat_pile_with_radius(Vec2::new(10.0, 10.0), 300.0, 8.0, &mut rng, 0.1)
            .expect("meat pile with radius");
        let pos = store.pos(id).unwrap();
        assert!(pos.x >= 10.0 - 0.8 - 1e-4 && pos.x <= 10.0 + 0.8 + 1e-4);
        assert!(pos.y >= 10.0 - 0.8 - 1e-4 && pos.y <= 10.0 + 0.8 + 1e-4);
    }

    #[test]
    fn decay_is_fresh_then_rots_and_chops() {
        let mut store = PelletStore::new();
        let id = store.spawn(PelletSpawn::new(MaterialKind::Meat, 1000.0, Vec2::ZERO));
        assert!(store.is_decaying(id));

        // First 1000 s of the 2000 s fresh window: no rot.
        store.process_decay(1000.0);
        assert!(almost(store.fresh_time_remaining(id).unwrap(), 1000.0));
        assert!(almost(store.rot_amount(id).unwrap(), 0.0));

        // Crossing the freshness boundary on a 1000 s step accrues the whole
        // step's rot: 1000 s * 0.02 u²/s = 20 u² (2 % of amount, below the 5 %
        // chop gate).
        store.process_decay(1000.0);
        assert!(almost(store.fresh_time_remaining(id).unwrap(), 0.0));
        assert!(almost(store.rot_amount(id).unwrap(), 20.0));
        assert!(almost(store.amount(id).unwrap(), 1000.0));

        // +32 u² rot on the next 1600 s step -> 52 u² > 5 % of 1000 -> chopped.
        store.process_decay(1600.0);
        assert!(almost(store.rot_amount(id).unwrap(), 0.0));
        assert!(almost(store.amount(id).unwrap(), 948.0));
        assert!(store.contains(id));
        assert!(almost(store.counts().meat_biomass as f32, 9480.0));

        // Keep decaying until the pellet is destroyed.
        for _ in 0..1000 {
            store.process_decay(1000.0);
            if !store.contains(id) {
                break;
            }
        }
        assert!(!store.contains(id));
        assert_eq!(store.counts().meat_count, 0);
        assert!(almost(store.counts().meat_biomass as f32, 0.0));
    }

    #[test]
    fn decay_below_threshold_accumulates_without_chopping() {
        let mut store = PelletStore::new();
        let id = store.spawn(PelletSpawn::new(MaterialKind::Meat, 1000.0, Vec2::ZERO));
        // Consume the fresh window: rot becomes 2000 * 0.02 = 40 u² (4 %).
        store.process_decay(2000.0);
        assert!(almost(store.rot_amount(id).unwrap(), 40.0));
        assert!(almost(store.amount(id).unwrap(), 1000.0));
        store.process_decay(100.0);
        assert!(almost(store.rot_amount(id).unwrap(), 42.0));
        // +12 u² -> 54 u² > 50 u² (5 %) -> chop 54 u².
        store.process_decay(600.0);
        assert!(almost(store.rot_amount(id).unwrap(), 0.0));
        assert!(almost(store.amount(id).unwrap(), 946.0));
    }

    /// A store of `count` pellets of both materials with some taken out again: holes among the slots, and a dense order that is not
    /// the slot order, the way a world that has run for a while looks.
    fn churned_store(count: usize, seed: u64) -> PelletStore {
        let mut rng = Rng::from_seed(seed);
        let mut store = PelletStore::new();
        let mut ids = Vec::new();
        for index in 0..count {
            let material = if index % 3 == 0 {
                MaterialKind::Plant
            } else {
                MaterialKind::Meat
            };
            let amount = rng.f32_range(2.0, 400.0);
            let id = store.spawn(PelletSpawn::new(
                material,
                amount,
                Vec2::new(index as f32, 0.0),
            ));
            if material == MaterialKind::Plant && index % 2 == 0 {
                store.register_decay(id);
            }
            ids.push(id);
        }
        for (index, id) in ids.iter().enumerate() {
            if index % 7 == 3 {
                store.remove(*id);
            }
        }
        store
    }

    /// The decay pass on the worker pool is the serial one: the whole store byte for byte after every pass, with new pellets taking
    /// the slots the chops freed in between (so the order the pellets were removed in shows in their ids).
    #[test]
    fn decay_on_the_pool_is_the_serial_decay() {
        let pool = crate::sim::ThreadPool::new(4);
        let executor = crate::sim::ParallelExecutor::new(Some(&pool));
        let mut serial = churned_store(6_000, 11);
        let mut pooled = churned_store(6_000, 11);
        for store in [&mut serial, &mut pooled] {
            store.materials_mut().plant.decay_rate = 0.01;
        }
        assert_eq!(serial.capture_checkpoint(), pooled.capture_checkpoint());
        let mut destroyed = 0;
        for round in 0..40 {
            let dt = 150.0 + 5.0 * round as f32;
            let a = serial.process_decay(dt);
            let b = pooled.process_decay_with(dt, executor, 1);
            assert_eq!(a, b, "round {round}: the report");
            destroyed += a.removed_pellets;
            for extra in 0..6 {
                for store in [&mut serial, &mut pooled] {
                    store.spawn(PelletSpawn::new(
                        MaterialKind::Meat,
                        9.0 + extra as f32,
                        Vec2::new(round as f32, extra as f32),
                    ));
                }
            }
            assert_eq!(
                serial.capture_checkpoint(),
                pooled.capture_checkpoint(),
                "round {round}: the store"
            );
            assert!(serial.ids().eq(pooled.ids()), "round {round}: the dense order");
        }
        assert!(destroyed > 100, "only {destroyed} pellets were destroyed");
    }

    #[test]
    fn plants_do_not_decay_but_can_be_registered() {
        let mut store = PelletStore::new();
        let id = pellet(&mut store, 20.0, 0.0, 0.0);
        assert!(!store.is_decaying(id));
        store.process_decay(10_000.0);
        assert!(almost(store.amount(id).unwrap(), 20.0));

        assert!(store.register_decay(id));
        assert!(store.is_decaying(id));
        assert!(almost(store.fresh_time_remaining(id).unwrap(), 0.0));
        store.process_decay(1000.0);
        assert!(almost(store.amount(id).unwrap(), 20.0));

        store.materials_mut().plant.decay_rate = 0.02;
        store.process_decay(1000.0); // 20 u² rot >= amount -> destroyed
        assert!(!store.contains(id));
        assert!(!store.unregister_decay(id));
    }

    #[test]
    fn decay_throttle_matches_game_period() {
        let mut throttle = DecayThrottle::new(2);
        assert_eq!(throttle.period(), 2);
        assert!(!throttle.tick());
        assert!(throttle.tick());
        assert!(!throttle.tick());
        assert!(throttle.tick());
        assert_eq!(throttle.progress(), 0);
        throttle.reset();
        assert_eq!(throttle.progress(), 0);
    }

    #[test]
    fn decay_parameters_update() {
        let mut store = PelletStore::new();
        let id = store.spawn(PelletSpawn::new(MaterialKind::Meat, 20.0, Vec2::ZERO));
        store.update_decay_parameters(MaterialKind::Meat, 500.0);
        assert!(almost(store.fresh_time_remaining(id).unwrap(), 2500.0));
        store.update_decay_parameters(MaterialKind::Plant, 500.0);
        assert!(almost(store.fresh_time_remaining(id).unwrap(), 2500.0));
    }

    #[test]
    fn recompute_counts_after_material_change() {
        let mut store = PelletStore::new();
        pellet(&mut store, 20.0, 0.0, 0.0);
        store.spawn(PelletSpawn::new(MaterialKind::Meat, 10.0, Vec2::ZERO));
        store.materials_mut().plant.energy_density = 10.0;
        store.recompute_counts();
        let counts = store.counts();
        assert_eq!(counts.plant_count, 1);
        assert_eq!(counts.meat_count, 1);
        assert!(almost(counts.plant_biomass as f32, 200.0));
        assert!(almost(counts.meat_biomass as f32, 100.0));
        assert_eq!(counts.total_count(), 2);
        assert!(almost(counts.total_biomass() as f32, 300.0));
    }

    #[test]
    fn helpers_are_stable() {
        assert!(almost(matter_radius(40.0), (40.0f32 / PI).sqrt()));
        assert_eq!(matter_radius(-5.0), 0.0);
        assert!(is_negligible_amount(0.0));
        assert!(!is_negligible_amount(1.0));
        assert_eq!(sanitize_amount(f32::NAN), 0.0);
        assert_eq!(sanitize_amount(-2.0), 0.0);
    }
}
