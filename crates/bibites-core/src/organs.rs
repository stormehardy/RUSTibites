//! Pure organ state machines and formulas: stomach, fat, armor, egg laying,
//! egg hatching, internal clock, pheromone emission, mouth and propulsion.
//!
//! Like [`crate::body`] this module has **no store/genome/brain/settings
//! dependency**; callers pass primitive parameter structs. All formulas follow
//! `docs/spec/02-body-organs-physics.md` and cite the decompiled source in the
//! doc comments. `f32` only, allocation-free, deterministic (no randomness).
//!
//! B2 will hold one of each state struct per bibite and call the `update`
//! functions in the game's organ order (`armor, stomach, eggLayer, fat, move,
//! mouth, pheromone`; spec 02 §2.3).

use crate::math::{EPS, PI, Vec2, clamp, clamp01, inv_lerp, remap, sign, tanh};

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

/// Number of physical stomach materials: Plant and Meat
/// (`MatterMaterialManager.PhysicalMaterials`, 2 entries).
pub const STOMACH_SLOTS: usize = 2;
/// Death region for `Want2Eat`/`Want2Attack` outputs (`BibiteMouth.cs:121-125`).
pub const MOUTH_DEAD_ZONE: f32 = 0.15;
/// `EggProduction` dead zone (`BibiteEggLayingOrgan.cs:125`).
pub const EGG_PRODUCTION_THRESHOLD: f32 = 0.15;
/// `Want2Lay` threshold to lay completed eggs (`BibiteEggLayingOrgan.cs:58`).
pub const LAYING_THRESHOLD: f32 = 0.25;
/// Egg reabsorption efficiency (`BibiteEggLayingOrgan.cs:60`).
pub const EGG_REABSORPTION_EFFICIENCY: f32 = 0.5;
/// Clock output above which `chronoTime` resets (`InternalClock.cs:76-83`).
pub const CLOCK_RESET_THRESHOLD: f32 = 0.75;
/// Ageing event period in seconds (`InternalClock.cs:84-89`).
pub const AGE_EVENT_PERIOD: f32 = 120.0;
/// Seconds in one day (achievement boundary, `InternalClock.cs:91-95`).
pub const DAY_SECONDS: f32 = 86_400.0;
/// Pheromone emission period in seconds (`BibitePheromoneOrgan.cs:68-71`).
pub const PHEROMONE_EMIT_PERIOD: f32 = 1.0;
/// Minimum summed pheromone output to spawn a spot (`BibitePheromoneOrgan.cs:75`).
pub const PHEROMONE_EMIT_THRESHOLD: f32 = 0.05;
/// Armor shell thickness factor (`BibiteArmor.cs:60`).
pub const ARMOR_THICKNESS_FACTOR: f32 = 5.0;

// ---------------------------------------------------------------------------
// Material parameters
// ---------------------------------------------------------------------------

/// Runtime material constants needed by organ math. The values mirror
/// `MaterialProperties::game_defaults()` in `docs/INTERFACES.md` §7 (captured
/// from a default `0.6.3.1` save); `settings.rs`/`pellet.rs` will own the
/// authoritative copies in B2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialParams {
    /// `Cohesiveness` (bite resistance, N/u).
    pub cohesiveness: f32,
    /// `Reactivity` (digestion speed, u/s).
    pub reactivity: f32,
    /// Energy per u² (E/u²).
    pub energy_density: f32,
    /// Mass per u² (g/u²).
    pub mass_density: f32,
    /// Conversion efficiency at zero affinity.
    pub min_efficiency: f32,
    /// Conversion efficiency at full affinity.
    pub max_efficiency: f32,
}

impl MaterialParams {
    /// Plant defaults (`settings.bb8settings.materials.PlantSettings`).
    pub const PLANT: Self = Self {
        cohesiveness: 10.0,
        reactivity: 0.03,
        energy_density: 5.0,
        mass_density: 0.015,
        min_efficiency: -0.25,
        max_efficiency: 0.5,
    };
    /// Meat defaults.
    pub const MEAT: Self = Self {
        cohesiveness: 35.0,
        reactivity: 0.075,
        energy_density: 10.0,
        mass_density: 0.025,
        min_efficiency: 0.15,
        max_efficiency: 0.85,
    };
    /// Fat defaults.
    pub const FAT: Self = Self {
        cohesiveness: 0.25,
        reactivity: 1.0,
        energy_density: 30.0,
        mass_density: 0.03,
        min_efficiency: 0.5,
        max_efficiency: 0.85,
    };
    /// Armor defaults.
    pub const ARMOR: Self = Self {
        cohesiveness: 500.0,
        reactivity: 1.0,
        energy_density: 1.5,
        mass_density: 0.05,
        min_efficiency: 1.0,
        max_efficiency: 1.0,
    };

    /// `minEff + clamp01(affinity) * (maxEff - minEff)` (`MatterMaterial.cs:41-44`).
    pub fn conversion_efficiency(&self, affinity: f32) -> f32 {
        self.min_efficiency + clamp01(affinity) * (self.max_efficiency - self.min_efficiency)
    }

    /// `amount * energyDensity`.
    pub fn energy_of_amount(&self, amount: f32) -> f32 {
        amount * self.energy_density
    }

    /// `energy / energyDensity`.
    pub fn amount_of_energy(&self, energy: f32) -> f32 {
        energy / self.energy_density
    }

    /// `sqrt(amount / PI)` (`MatterMaterial.cs:56-59`).
    pub fn radius_of_amount(&self, amount: f32) -> f32 {
        (amount / PI).sqrt()
    }
}

/// The two materials that can be physically stored in a stomach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StomachMaterial {
    /// Plant pellets.
    Plant,
    /// Meat (pellets or bitten flesh).
    Meat,
}

impl StomachMaterial {
    /// Material constants for this stomach material.
    pub fn params(self) -> MaterialParams {
        match self {
            Self::Plant => MaterialParams::PLANT,
            Self::Meat => MaterialParams::MEAT,
        }
    }
}

/// `ThresholdRemap(value, threshold, 1)` from `Here/ScriptHelpers/MathfE.cs:14-17`:
/// sign-preserving remap of `|value|` from `[threshold, 1]` to `[0, 1]`.
pub fn threshold_remap(value: f32, threshold: f32) -> f32 {
    if threshold >= 1.0 {
        return 0.0;
    }
    sign(value) * remap(value.abs(), threshold, 1.0, 0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Stomach
// ---------------------------------------------------------------------------

/// One material slot of the stomach (`StomachContent.cs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StomachContent {
    /// Material stored in this slot.
    pub material: StomachMaterial,
    /// Amount in u².
    pub amount: f32,
    /// Mean chunk size in u² (`averageChunkAmount`).
    pub average_chunk_amount: f32,
    /// Cached conversion efficiency for the current affinity.
    pub efficiency: f32,
    /// Last digestion rate (u²/s), UI/diagnostic.
    pub digestion_rate: f32,
    /// Conversion efficiency used by the last digestion tick.
    pub last_conversion_efficiency: f32,
}

impl StomachContent {
    /// Creates an empty Plant slot.
    pub fn new() -> Self {
        Self {
            material: StomachMaterial::Plant,
            amount: 0.0,
            average_chunk_amount: 0.0,
            efficiency: 0.0,
            digestion_rate: 0.0,
            last_conversion_efficiency: 0.0,
        }
    }

    /// `approximateN = max(1, amount / averageChunkAmount)` (1 when the mean
    /// chunk is not positive) (`StomachContent.cs:23-33`).
    pub fn approximate_n(&self) -> f32 {
        if self.average_chunk_amount > 0.0 {
            (self.amount / self.average_chunk_amount).max(1.0)
        } else {
            1.0
        }
    }

    /// `digestionRate * energyDensity * lastConversionEfficiency`
    /// (`StomachContent.cs:35`).
    pub fn energy_gain_rate(&self) -> f32 {
        self.digestion_rate
            * self.material.params().energy_density
            * self.last_conversion_efficiency
    }
}

impl Default for StomachContent {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of one stomach tick (`BibiteStomach.UpdateOrgan`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DigestionTick {
    /// Energy gained this tick (already efficiency-adjusted).
    pub energy_gain: f32,
    /// Total amount after digestion.
    pub total_content: f32,
    /// Digestion output (acid level).
    pub acid_level: f32,
    /// `acidLevel / fullness` (0 when empty).
    pub acid_ratio: f32,
    /// Combined digestion speed multiplier.
    pub effectiveness: f32,
    /// Extra speed from acid above fullness.
    pub potential_bonus: f32,
    /// Efficiency loss from acid above fullness.
    pub efficiency_malus: f32,
    /// Number of content slots fully digested and removed this tick.
    pub finished_slots: u8,
}

/// Stomach state (`BibiteStomach`). Capacity is the organ area (1 u² = 1
/// amount unit); at most two slots (Plant, Meat).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stomach {
    contents: [StomachContent; STOMACH_SLOTS],
    n_content: usize,
    /// Organ area = capacity (u²).
    pub area: f32,
    /// `Diet` gene, `0` herbivore .. `1` carnivore.
    pub diet: f32,
    /// `plantAffinityPowerFactor`.
    pub plant_affinity_power: f32,
    /// `meatAffinityPowerFactor`.
    pub meat_affinity_power: f32,
    /// `stomachContentContribution` (mass contribution, default 1.0).
    pub stomach_content_contribution: f32,
    /// Total content after the last tick (`totalContent`).
    pub total_content: f32,
    /// Last `Digestion` output (`acidLevel`).
    pub acid_level: f32,
    /// Last acid/fullness ratio.
    pub acid_ratio: f32,
    /// Last digestion effectiveness.
    pub digestion_effectiveness: f32,
    /// Last potential bonus.
    pub digestion_potential_bonus: f32,
    /// Last efficiency malus.
    pub efficiency_malus: f32,
}

impl Stomach {
    /// Creates an empty stomach for an organ area and genome/settings values.
    pub fn new(
        area: f32,
        diet: f32,
        plant_affinity_power: f32,
        meat_affinity_power: f32,
        stomach_content_contribution: f32,
    ) -> Self {
        Self {
            contents: [StomachContent::new(); STOMACH_SLOTS],
            n_content: 0,
            area,
            diet,
            plant_affinity_power,
            meat_affinity_power,
            stomach_content_contribution,
            total_content: 0.0,
            acid_level: 0.0,
            acid_ratio: 0.0,
            digestion_effectiveness: 0.0,
            digestion_potential_bonus: 0.0,
            efficiency_malus: 0.0,
        }
    }

    /// Number of occupied slots.
    pub fn n_content(&self) -> usize {
        self.n_content
    }

    /// Slot iterator over occupied contents.
    pub fn contents(&self) -> impl Iterator<Item = &StomachContent> {
        self.contents[..self.n_content].iter()
    }

    /// Slot accessor.
    pub fn content(&self, index: usize) -> Option<&StomachContent> {
        self.contents.get(index)
    }

    /// Sets the organ area (capacity).
    pub fn set_area(&mut self, area: f32) {
        self.area = area;
    }

    /// Restores stomach slots in the saved order, carrying the mean chunk size
    /// that `add_matter` cannot infer. Existing per-material conversion caches
    /// from a prior load are preserved when the material matches.
    pub fn restore_contents(&mut self, slots: &[(StomachMaterial, f32, f32)]) {
        let old = self.contents;
        self.contents = [StomachContent::new(); STOMACH_SLOTS];
        self.n_content = 0;
        for (material, amount, average_chunk_amount) in slots {
            if self.n_content >= STOMACH_SLOTS {
                break;
            }
            let mut content = old
                .iter()
                .find(|content| content.material == *material)
                .copied()
                .unwrap_or_else(StomachContent::new);
            content.material = *material;
            content.amount = *amount;
            content.average_chunk_amount = *average_chunk_amount;
            self.contents[self.n_content] = content;
            self.n_content += 1;
        }
        self.total_content = self.contents[..self.n_content]
            .iter()
            .map(|content| content.amount)
            .sum();
    }

    /// Replaces contents with exact runtime slots (checkpoint restore).
    pub fn set_contents(&mut self, contents: &[StomachContent]) {
        self.contents = [StomachContent::new(); STOMACH_SLOTS];
        self.n_content = contents.len().min(STOMACH_SLOTS);
        self.contents[..self.n_content].copy_from_slice(&contents[..self.n_content]);
        self.total_content = self.contents[..self.n_content]
            .iter()
            .map(|content| content.amount)
            .sum();
    }

    /// `fullness = area > 0 ? totalAmount/area : 0` (`BibiteStomach.cs:79-89`).
    pub fn fullness(&self) -> f32 {
        if self.area > 0.0 {
            self.total_amount() / self.area
        } else {
            0.0
        }
    }

    /// `area - totalAmount`, floored at 0 (`BibiteStomach.cs:108`).
    pub fn available_space(&self) -> f32 {
        (self.area - self.total_amount()).max(0.0)
    }

    /// True when no material slot is occupied (`BibiteStomach.cs:91`).
    pub fn is_empty(&self) -> bool {
        self.n_content == 0
    }

    /// Sum of all content amounts.
    pub fn total_amount(&self) -> f32 {
        self.contents[..self.n_content]
            .iter()
            .map(|c| c.amount)
            .sum()
    }

    /// `mass = Σ amount·material.MassDensity·stomachContentContribution`
    /// (`BibiteStomach.cs:64-75`).
    pub fn mass(&self) -> f32 {
        self.contents[..self.n_content]
            .iter()
            .map(|c| c.amount * c.material.params().mass_density)
            .sum::<f32>()
            * self.stomach_content_contribution
    }

    /// Total energy gain rate of all contents (E/s).
    pub fn energy_gain_rate(&self) -> f32 {
        self.contents[..self.n_content]
            .iter()
            .map(|c| c.energy_gain_rate())
            .sum()
    }

    /// `Diet` affinity for a material (`BibiteStomach.cs:323-334`).
    pub fn affinity(&self, material: StomachMaterial) -> f32 {
        match material {
            StomachMaterial::Plant => (1.0 - self.diet).powf(self.plant_affinity_power),
            StomachMaterial::Meat => self.diet.powf(self.meat_affinity_power),
        }
    }

    /// Finds the slot holding `material`, searching from the end like the game.
    fn slot_of(&self, material: StomachMaterial) -> Option<usize> {
        self.contents[..self.n_content]
            .iter()
            .rposition(|c| c.material == material)
    }

    /// Recomputes cached conversion efficiencies (called when material
    /// settings or the diet change).
    pub fn refresh_efficiencies(&mut self, plant: &MaterialParams, meat: &MaterialParams) {
        // Compute the affinities before the mutable iteration so `*self` is not
        // borrowed inside the loop.
        let plant_affinity = self.affinity(StomachMaterial::Plant);
        let meat_affinity = self.affinity(StomachMaterial::Meat);
        for content in self.contents[..self.n_content].iter_mut() {
            let (params, affinity) = match content.material {
                StomachMaterial::Plant => (plant, plant_affinity),
                StomachMaterial::Meat => (meat, meat_affinity),
            };
            content.efficiency = params.conversion_efficiency(affinity);
        }
    }

    /// `AddMatter` (`BibiteStomach.cs:257-292`): clamps to available space,
    /// creates a slot or merges into the existing material's chunk distribution.
    /// Returns the amount actually added.
    pub fn add_matter(
        &mut self,
        material: StomachMaterial,
        amount: f32,
        plant: &MaterialParams,
        meat: &MaterialParams,
    ) -> f32 {
        if amount.abs() <= EPS {
            return 0.0;
        }
        let amount = amount.min(self.available_space());
        let params = match material {
            StomachMaterial::Plant => plant,
            StomachMaterial::Meat => meat,
        };
        let affinity = self.affinity(material);
        match self.slot_of(material) {
            Some(slot) => {
                let content = &mut self.contents[slot];
                let old_amount = content.amount;
                let old_average = content.average_chunk_amount;
                if old_amount > 0.0 {
                    let ratio_in = amount / old_amount;
                    let ratio_out = old_amount / amount;
                    content.average_chunk_amount =
                        (old_average / (1.0 + ratio_in) + amount / (1.0 + ratio_out)).max(0.0);
                }
                content.amount = old_amount + amount;
                amount
            }
            None => {
                if self.n_content >= STOMACH_SLOTS {
                    return 0.0;
                }
                let content = &mut self.contents[self.n_content];
                content.material = material;
                content.amount = amount;
                content.average_chunk_amount = amount.max(0.0);
                content.efficiency = params.conversion_efficiency(affinity);
                content.digestion_rate = 0.0;
                content.last_conversion_efficiency = 0.0;
                self.n_content += 1;
                amount
            }
        }
    }

    /// `RemoveMatter` (`BibiteStomach.cs:294-321`).
    ///
    /// Mirrors the decompiled behaviour (including its quirk that the
    /// whole-slot removal branch is unreachable because the amount is clamped
    /// first): the returned amount is what was removed and the slot stays with
    /// the reduced amount. B2 decides whether to compact empty slots.
    pub fn remove_matter(&mut self, material: StomachMaterial, amount: f32) -> f32 {
        let Some(slot) = self.slot_of(material) else {
            return 0.0;
        };
        let content = &mut self.contents[slot];
        let removed = amount.min(content.amount);
        content.amount -= removed;
        removed
    }

    /// `UpdateOrgan` (`BibiteStomach.cs:162-219`).
    ///
    /// `digestion_output` is the brain's `Digestion` output (acid level);
    /// `metabolic_rate` is the `SpeedRatio` gene. Returns the energy gained
    /// this tick; fully digested slots are compacted and counted.
    pub fn update(
        &mut self,
        dt: f32,
        digestion_output: f32,
        metabolic_rate: f32,
        plant: &MaterialParams,
        meat: &MaterialParams,
    ) -> DigestionTick {
        if self.is_empty() {
            return DigestionTick::default();
        }
        let total = self.total_amount();
        self.total_content = total;
        self.acid_level = digestion_output;
        let fullness = if self.area > 0.0 {
            total / self.area
        } else {
            0.0
        };
        self.acid_ratio = if fullness > 0.0 {
            self.acid_level / fullness
        } else {
            0.0
        };
        let excess = self.acid_level - fullness;
        self.digestion_potential_bonus = if self.acid_level < fullness {
            0.0
        } else {
            excess
        };
        self.efficiency_malus = if self.acid_level < fullness {
            0.0
        } else {
            excess / 2.0
        };
        self.digestion_effectiveness =
            self.acid_ratio.min(1.0) * metabolic_rate * (1.0 + self.digestion_potential_bonus);

        let mut energy_gain = 0.0;
        let mut finished = false;
        let efficiency_malus = self.efficiency_malus;
        let effectiveness = self.digestion_effectiveness;
        let occupied = self.n_content;
        for slot in self.contents[..occupied].iter_mut() {
            let mut content = *slot;
            let params = match content.material {
                StomachMaterial::Plant => plant,
                StomachMaterial::Meat => meat,
            };
            let efficiency = content.efficiency;
            let conversion_efficiency = if efficiency > 0.0 {
                efficiency * (1.0 - efficiency_malus)
            } else {
                efficiency - efficiency_malus
            };
            let chunk_radius = (content.average_chunk_amount / PI).sqrt();
            let depth = (params.reactivity * effectiveness * dt).min(chunk_radius);
            let segment_area = depth * (2.0 * chunk_radius - depth) * PI;
            let mut dissolved = content.approximate_n() * segment_area;
            if depth >= chunk_radius {
                dissolved = content.amount;
                finished = true;
                content.amount = -0.000_1;
                content.average_chunk_amount = 0.0;
            } else {
                content.average_chunk_amount =
                    (content.average_chunk_amount - segment_area).max(0.0);
                content.amount -= dissolved;
            }
            content.digestion_rate = if dt > 0.0 { dissolved / dt } else { 0.0 };
            content.last_conversion_efficiency = conversion_efficiency;
            energy_gain += params.energy_of_amount(dissolved) * conversion_efficiency;
            *slot = content;
        }

        let mut finished_slots = 0u8;
        if finished {
            let mut index = 0;
            while index < self.n_content {
                if self.contents[index].amount > 0.0 {
                    index += 1;
                } else {
                    self.contents[index] = self.contents[self.n_content - 1];
                    self.n_content -= 1;
                    finished_slots += 1;
                }
            }
        }

        DigestionTick {
            energy_gain,
            total_content: total,
            acid_level: self.acid_level,
            acid_ratio: self.acid_ratio,
            effectiveness: self.digestion_effectiveness,
            potential_bonus: self.digestion_potential_bonus,
            efficiency_malus: self.efficiency_malus,
            finished_slots,
        }
    }
}

impl Default for Stomach {
    fn default() -> Self {
        Self::new(0.0, 0.0, 1.0, 1.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Fat organ
// ---------------------------------------------------------------------------

/// Result of one fat-organ tick (`BibiteFatOrgan.UpdateOrgan`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FatTick {
    /// Change of stored fat in u² (positive = storing, negative = burning).
    pub amount_delta: f32,
    /// Energy change: negative when storing (cost), positive when burning.
    pub energy_gain: f32,
    /// Energy spent sustaining the reserves this tick (`rate * dt`).
    pub sustain_cost: f32,
    /// New inflation factor.
    pub inflate_factor: f32,
    /// Whether the inflation factor changed enough to update the body collider
    /// (`|Δ| >= 0.025`, `BibiteFatOrgan.cs:118-126`).
    pub inflate_changed: bool,
}

/// Fat storage state (`BibiteFatOrgan`): converts active energy to/from a
/// reserve with threshold+deadband hysteresis and inflates the body when
/// overflow exceeds the base capacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FatOrgan {
    /// Stored fat amount (u²).
    pub amount: f32,
    /// Maximum stored fat (inflated organ area).
    pub max_amount: f32,
    /// Current organ area (base capacity capped at 2× by inflation).
    pub area: f32,
    /// Base capacity = body area × FatWAG portion.
    pub base_capacity: f32,
    /// `baseBodyArea * 0.35`.
    pub max_inflate_amount: f32,
    /// Energy-ratio neutral point (gene).
    pub threshold: f32,
    /// Deadband half-width (gene).
    pub deadband: f32,
    /// Current inflation factor.
    pub inflate_factor: f32,
    /// Last applied inflation factor.
    pub last_inflate_factor: f32,
    /// Last activation in `-1..1`.
    pub last_activation: f32,
    /// Last amount delta (u²/tick).
    pub last_amount_delta: f32,
    /// Last amount rate (u²/s).
    pub last_amount_rate: f32,
    /// Last conversion affinity.
    pub last_affinity: f32,
    /// Last energy gain (signed).
    pub last_energy_gain: f32,
    /// Last energy gain rate (E/s).
    pub last_energy_gain_rate: f32,
    /// Last fat sustain rate (E/s).
    pub last_energy_sustain_rate: f32,
}

impl FatOrgan {
    /// Creates the organ for a FatWAG portion relative to `base_body_area`
    /// (`BibiteFatOrgan.OnGrowth`, `BibiteFatOrgan.cs:82-88`).
    pub fn new(fat_area_portion: f32, base_body_area: f32, threshold: f32, deadband: f32) -> Self {
        let mut organ = Self {
            amount: 0.0,
            max_amount: 0.0,
            area: 0.0,
            base_capacity: 0.0,
            max_inflate_amount: 0.0,
            threshold,
            deadband,
            inflate_factor: 0.0,
            last_inflate_factor: 0.0,
            last_activation: 0.0,
            last_amount_delta: 0.0,
            last_amount_rate: 0.0,
            last_affinity: 1.0,
            last_energy_gain: 0.0,
            last_energy_gain_rate: 0.0,
            last_energy_sustain_rate: 0.0,
        };
        organ.on_growth(fat_area_portion, base_body_area);
        organ
    }

    /// Recomputes capacity/area after a growth event
    /// (`BibiteFatOrgan.OnGrowth`).
    pub fn on_growth(&mut self, fat_area_portion: f32, base_body_area: f32) {
        self.base_capacity = base_body_area * fat_area_portion;
        self.max_inflate_amount = base_body_area * crate::body::FAT_INFLATE_FRACTION;
        self.area = if fat_area_portion > 0.0 {
            self.base_capacity
                * (1.0 + crate::body::FAT_INFLATE_FRACTION / fat_area_portion).min(2.0)
        } else {
            0.0
        };
        // `BibiteFatOrgan.OnGrowth` (cs:82-88) only updates the capacity: the
        // stored amount may overflow it (that is what inflates the tail) and
        // the inflate state is left to `CheckInflateFactor`.
        self.max_amount = self.area;
    }

    /// `mass = amount * fat material massDensity`.
    pub fn mass(&self, fat: &MaterialParams) -> f32 {
        self.amount * fat.mass_density
    }

    /// `fullness = amount / maxAmount`.
    pub fn fullness(&self) -> f32 {
        if self.max_amount > 0.0 {
            self.amount / self.max_amount
        } else {
            0.0
        }
    }

    /// Fat above the base capacity (`overflowAmount`).
    pub fn overflow_amount(&self) -> f32 {
        (self.amount - self.base_capacity).max(0.0)
    }

    /// `CheckInflateFactor(check: false)` for the fat the body holds now (what a loaded bibite's inflation is).
    pub fn refresh_inflate(&mut self) {
        self.check_inflate(false);
    }

    /// Recomputes the body inflation; `check` applies the 0.025 deadband
    /// (`BibiteFatOrgan.CheckInflateFactor`, `BibiteFatOrgan.cs:118-126`).
    fn check_inflate(&mut self, check: bool) -> bool {
        let candidate = if self.max_inflate_amount > 0.0 {
            clamp01(inv_lerp(
                0.0,
                self.max_inflate_amount,
                self.overflow_amount(),
            ))
        } else {
            0.0
        };
        self.inflate_factor = candidate;
        if !check || (candidate - self.last_inflate_factor).abs() >= 0.025 {
            self.last_inflate_factor = candidate;
            true
        } else {
            false
        }
    }

    /// `UpdateOrgan` (`BibiteFatOrgan.cs:90-116`).
    ///
    /// `energy_ratio` is `Energy.Amount / Energy.MaxAmount`; `d2_size` is the
    /// body size state (the game uses `body.d2Size`); `fat_sustain` is the
    /// global sustain cost setting. Returns signed energy gain and the sustain
    /// cost for the caller to apply through the body energy accounting.
    pub fn update(
        &mut self,
        dt: f32,
        energy_ratio: f32,
        d2_size: f32,
        metabolic_rate: f32,
        fat_sustain: f32,
        fat: &MaterialParams,
    ) -> FatTick {
        let inactive = (energy_ratio - self.threshold).abs() < self.deadband
            || (energy_ratio > self.threshold && self.amount >= self.max_amount)
            || (energy_ratio < self.threshold && self.amount <= 0.0);
        if inactive {
            self.last_activation = 0.0;
            self.last_amount_delta = 0.0;
            self.last_amount_rate = 0.0;
            self.last_energy_gain = 0.0;
            self.last_energy_gain_rate = 0.0;
            self.last_affinity = 1.0;
            self.last_energy_sustain_rate = self.amount * fat.energy_density * fat_sustain;
            return FatTick {
                amount_delta: 0.0,
                energy_gain: 0.0,
                sustain_cost: self.last_energy_sustain_rate * dt,
                inflate_factor: self.inflate_factor,
                inflate_changed: false,
            };
        }

        let activation = if energy_ratio > self.threshold {
            inv_lerp(
                (self.threshold + self.deadband).min(0.99),
                1.0,
                energy_ratio,
            )
        } else {
            -inv_lerp(self.threshold - self.deadband, 0.0, energy_ratio)
        };
        let activation = if activation > 0.0 {
            activation * (1.0 - self.fullness().powi(10))
        } else {
            activation
        };
        self.last_activation = activation;

        let mut delta = activation * d2_size * metabolic_rate * (fat.reactivity * dt).powi(2) * PI;
        delta = if delta > 0.0 {
            // `MatterReserves.available` is `max(MaxAmount - Amount, 0)`.
            delta.min((self.max_amount - self.amount).max(0.0))
        } else {
            delta.max(-self.amount)
        };
        self.last_amount_delta = delta;
        self.last_amount_rate = if dt > 0.0 { delta / dt } else { 0.0 };
        self.last_affinity = 1.0 - activation.abs().sqrt();

        let affinity = self.last_affinity;
        let efficiency = fat.conversion_efficiency(affinity);
        let energy_gain = if delta > 0.0 {
            -delta * fat.energy_density / efficiency
        } else {
            -delta * fat.energy_density * efficiency
        };
        self.amount += delta;
        self.last_energy_gain = energy_gain;
        self.last_energy_gain_rate = if dt > 0.0 { energy_gain / dt } else { 0.0 };
        let inflate_changed = self.check_inflate(true);
        self.last_energy_sustain_rate = self.amount * fat.energy_density * fat_sustain;
        FatTick {
            amount_delta: delta,
            energy_gain,
            sustain_cost: self.last_energy_sustain_rate * dt,
            inflate_factor: self.inflate_factor,
            inflate_changed,
        }
    }
}

// ---------------------------------------------------------------------------
// Armor organ
// ---------------------------------------------------------------------------

/// Armor shell state (`BibiteArmor`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ArmorOrgan {
    /// Body-area portion in `0..=1`.
    pub portion: f32,
    /// Shell thickness (u).
    pub thickness: f32,
    /// Shell strength before the body's ×2.
    pub strength: f32,
}

impl ArmorOrgan {
    /// Creates the armor organ and derives thickness/strength.
    pub fn new(portion: f32, d1_size: f32, inflate_factor: f32, cohesiveness: f32) -> Self {
        let mut organ = Self {
            portion,
            thickness: 0.0,
            strength: 0.0,
        };
        organ.update(portion, d1_size, inflate_factor, cohesiveness);
        organ
    }

    /// Recomputes thickness/strength (game `OnGrowth`/`UpdateThickness`).
    pub fn update(&mut self, portion: f32, d1_size: f32, inflate_factor: f32, cohesiveness: f32) {
        self.portion = portion;
        self.thickness = armor_thickness(portion, d1_size, inflate_factor);
        self.strength = self.thickness * cohesiveness;
    }

    /// `mass = area * armor massDensity` (the caller passes the organ area).
    pub fn mass(&self, area: f32, armor: &MaterialParams) -> f32 {
        area * armor.mass_density
    }

    /// `BibiteBody.armorStrength = 2 * armor.armorStrength` (`BibiteBody.cs:246`).
    pub fn effective_strength(&self) -> f32 {
        2.0 * self.strength
    }
}

/// `armorThickness = 5*d1Size*(1 - sqrt(1 - portion/(1+inflateFactor)))`
/// (`BibiteArmor.cs:60`).
pub fn armor_thickness(portion: f32, d1_size: f32, inflate_factor: f32) -> f32 {
    let ratio = portion / (1.0 + inflate_factor);
    ARMOR_THICKNESS_FACTOR * d1_size * (1.0 - (1.0 - ratio).sqrt())
}

// ---------------------------------------------------------------------------
// Egg laying organ (womb)
// ---------------------------------------------------------------------------

/// Result of one egg-layer tick (`BibiteEggLayingOrgan.UpdateOrgan`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EggLayerTick {
    /// Energy spent producing eggs this tick (0 when reabsorbing).
    pub energy_cost: f32,
    /// Energy gained from reabsorbed eggs this tick.
    pub energy_gain: f32,
    /// The organ calls `UseEnergy` this tick (production is on and `num >= 0`), whatever the cost.
    pub spends: bool,
    /// Completed eggs the caller may lay this tick (0 when limited/mature-locked).
    pub eggs_to_lay: u32,
    /// Egg progress rate (eggs/s).
    pub progress_rate: f32,
    /// Energy consumption rate (E/s).
    pub energy_consumption: f32,
}

/// Womb state (`BibiteEggLayingOrgan`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EggLayer {
    /// Maturity latch set by the growth organ (`isMature`).
    pub is_mature: bool,
    /// Egg progress; integers are completed eggs.
    pub progress: f32,
    /// Egg capacity = `floor(area / eggSize)`.
    pub capacity: u32,
    /// `areaAtBirth` of the child (u²).
    pub egg_size: f32,
    /// Energy invested per egg (E).
    pub egg_energy: f32,
    /// `LayTime` gene (seconds).
    pub laying_period: f32,
    /// Womb organ area (u²).
    pub area: f32,
    /// Last progress rate (eggs/s).
    pub progress_rate: f32,
    /// Last energy consumption rate (E/s).
    pub energy_consumption: f32,
    /// Total eggs laid.
    pub n_laid: u32,
}

impl EggLayer {
    /// Creates the womb for an organ area and genome values.
    pub fn new(
        area: f32,
        egg_size: f32,
        egg_energy: f32,
        laying_period: f32,
        is_mature: bool,
    ) -> Self {
        let mut layer = Self {
            is_mature,
            progress: 0.0,
            capacity: 0,
            egg_size,
            egg_energy,
            laying_period,
            area,
            progress_rate: 0.0,
            energy_consumption: 0.0,
            n_laid: 0,
        };
        layer.set_area(area);
        layer
    }

    /// Recomputes the capacity from the organ area (`OnGrowth`).
    pub fn set_area(&mut self, area: f32) {
        self.area = area;
        self.capacity = if self.egg_size > 0.0 {
            (area / self.egg_size).floor().max(0.0) as u32
        } else {
            0
        };
    }

    /// Number of completed eggs (`floor(progress)`).
    pub fn n_eggs(&self) -> u32 {
        self.progress.floor().max(0.0) as u32
    }

    /// Progress of the egg currently being built (`currentEggProgress`).
    pub fn current_egg_progress(&self) -> f32 {
        if self.n_eggs() != self.capacity || self.capacity == 0 {
            self.progress % 1.0
        } else {
            1.0
        }
    }

    /// `UpdateOrgan` (`BibiteEggLayingOrgan.cs:119-157`).
    ///
    /// `egg_production` is the brain output, `want_to_lay` the second output,
    /// `birth_limit_reached` the global population limit check. Production and
    /// energy accounting are applied; laying itself is reported in
    /// [`EggLayerTick::eggs_to_lay`] and must be confirmed by
    /// [`EggLayer::on_eggs_laid`] after the caller successfully spawns them.
    pub fn update(
        &mut self,
        dt: f32,
        egg_production: f32,
        metabolic_rate: f32,
        want_to_lay: f32,
        birth_limit_reached: bool,
    ) -> EggLayerTick {
        if !self.is_mature {
            return EggLayerTick::default();
        }
        if egg_production.abs() < EGG_PRODUCTION_THRESHOLD {
            self.energy_consumption = 0.0;
            self.progress_rate = 0.0;
            return EggLayerTick::default();
        }
        let remap_value = threshold_remap(egg_production, EGG_PRODUCTION_THRESHOLD);
        let mut delta = metabolic_rate * remap_value * dt / self.laying_period;
        delta = if delta >= 0.0 {
            delta.min(self.capacity as f32 - self.progress)
        } else {
            delta.max(-self.progress)
        };
        let delta_was_spend = delta >= 0.0;
        let (energy_cost, energy_gain) = if delta >= 0.0 {
            (delta * self.egg_energy, 0.0)
        } else {
            (
                0.0,
                (delta * self.egg_energy * EGG_REABSORPTION_EFFICIENCY).abs(),
            )
        };
        self.progress += delta;
        self.progress_rate = if dt > 0.0 { delta / dt } else { 0.0 };
        self.energy_consumption = self.progress_rate * self.egg_energy;

        let eggs_to_lay =
            if self.progress >= 1.0 && want_to_lay >= LAYING_THRESHOLD && !birth_limit_reached {
                self.n_eggs()
            } else {
                0
            };
        EggLayerTick {
            energy_cost,
            energy_gain,
            spends: delta_was_spend,
            eggs_to_lay,
            progress_rate: self.progress_rate,
            energy_consumption: self.energy_consumption,
        }
    }

    /// Applies the progress bookkeeping after the caller successfully laid
    /// `count` eggs (`BibiteEggLayingOrgan.cs:151-156`).
    pub fn on_eggs_laid(&mut self, count: u32) {
        self.progress -= count as f32;
        self.n_laid += count;
    }
}

/// `eggEnergy = LayBodyEnergy + brainUpkeepCost * SizeRatio`
/// (`BibiteEggLayingOrgan.cs:107`).
pub fn egg_energy_investment(lay_body_energy: f32, brain_upkeep: f32, size_ratio: f32) -> f32 {
    lay_body_energy + brain_upkeep * size_ratio
}

// ---------------------------------------------------------------------------
// Egg hatching
// ---------------------------------------------------------------------------

/// Result of one egg tick (`EggHatching.FixedUpdate`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum EggTick {
    /// Not incubating.
    #[default]
    Idle,
    /// Incubating; carries the hatch ratio.
    Progress {
        /// `min(hatchProgress / hatchTime, 1)`.
        ratio: f32,
    },
    /// Hatched this tick.
    Hatched,
    /// Aborted this tick (ran out of energy budget).
    Aborted,
}

/// Egg incubation state (`EggHatching`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Egg {
    /// Seconds incubated.
    pub hatch_progress: f32,
    /// Gene `HatchTime` seconds.
    pub hatch_time: f32,
    /// Energy stored in the egg (E).
    pub energy: f32,
    /// Energy required to complete the build (E).
    pub total_required_energy: f32,
    /// Currently incubating.
    pub is_hatching: bool,
    /// Aborting flag (set by [`Egg::start_hatch`] or [`Egg::tick`]).
    pub aborting: bool,
}

impl Egg {
    /// Creates an egg state.
    pub fn new(hatch_time: f32, total_required_energy: f32, energy: f32) -> Self {
        Self {
            hatch_progress: 0.0,
            hatch_time,
            energy,
            total_required_energy,
            is_hatching: false,
            aborting: false,
        }
    }

    /// `StartHatch` (`EggHatching.cs:108-134`): aborts when `hatchTime <= 0`
    /// or the egg has less than half the required energy. Returns `true` when
    /// incubation started.
    pub fn start_hatch(&mut self) -> bool {
        if self.hatch_time <= 0.0 || self.total_required_energy > 2.0 * self.energy {
            self.aborting = true;
            self.is_hatching = false;
            return false;
        }
        self.hatch_progress = 0.0;
        self.is_hatching = true;
        true
    }

    /// Resumes incubation after a load (`ResumeHatch`).
    pub fn resume_hatch(&mut self) {
        self.is_hatching = true;
    }

    /// `hatchProgress / hatchTime`, clamped to 1 (`HatchRatio`).
    pub fn hatch_ratio(&self) -> f32 {
        if self.hatch_time > 0.0 {
            (self.hatch_progress / self.hatch_time).min(1.0)
        } else {
            1.0
        }
    }

    /// `energy - totalRequiredEnergy * hatchProgress / hatchTime`.
    pub fn free_energy(&self) -> f32 {
        if self.hatch_time > 0.0 {
            self.energy - self.total_required_energy * self.hatch_progress / self.hatch_time
        } else {
            self.energy
        }
    }

    /// One incubation tick (`EggHatching.FixedUpdate`, `EggHatching.cs:159-173`).
    pub fn tick(&mut self, dt: f32) -> EggTick {
        if !self.is_hatching {
            return EggTick::Idle;
        }
        self.hatch_progress += dt;
        if self.hatch_ratio() * self.total_required_energy > self.energy {
            self.aborting = true;
            self.is_hatching = false;
            return EggTick::Aborted;
        }
        if self.hatch_progress >= self.hatch_time {
            self.is_hatching = false;
            return EggTick::Hatched;
        }
        EggTick::Progress {
            ratio: self.hatch_ratio(),
        }
    }
}

/// `totalRequiredEnergy = BodyEnergyAtBirth + brainUpkeepCost * SizeRatio`
/// (`EggHatching.cs:93`).
pub fn egg_required_energy(body_energy_at_birth: f32, brain_upkeep: f32, size_ratio: f32) -> f32 {
    body_energy_at_birth + brain_upkeep * size_ratio
}

// ---------------------------------------------------------------------------
// Internal clock
// ---------------------------------------------------------------------------

/// Result of a clock tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClockTick {
    /// A 120 s ageing event fired (`OnAging`).
    pub aged: bool,
    /// The bibite reached one day of life (achievement trigger).
    pub first_day: bool,
}

/// Internal clock (`InternalClock`): `Tic` square wave, `Minute` chronometer,
/// `TimeAlive`, and the 120 s ageing event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InternalClock {
    /// Square-wave state, 0 or 1 (`tic`).
    pub tic: u8,
    /// Progress toward the next tic toggle.
    pub tic_progress: f32,
    /// Seconds alive (`timeAlive`).
    pub time_alive: f32,
    /// Seconds since the last clock reset (`chronoTime`).
    pub chrono_time: f32,
    /// `ClockSpeed` gene (seconds per tic period).
    pub period: f32,
    /// Counting is enabled (game waits for the brain to be ready).
    pub counting: bool,
    /// Accumulator for the 120 s ageing event.
    pub age_progress: f32,
    /// One-day achievement already fired.
    pub day_triggered: bool,
}

impl InternalClock {
    /// Creates a started clock for `period` seconds.
    pub fn new(period: f32) -> Self {
        Self {
            tic: 0,
            tic_progress: 0.0,
            time_alive: 0.0,
            chrono_time: 0.0,
            period,
            counting: true,
            age_progress: 0.0,
            day_triggered: false,
        }
    }

    /// `StopClock`.
    pub fn stop(&mut self) {
        self.counting = false;
    }

    /// `ResumeClock`.
    pub fn resume(&mut self) {
        self.counting = true;
    }

    /// One fixed tick (`InternalClock.FixedUpdate`, `InternalClock.cs:65-97`).
    pub fn tick(&mut self, dt: f32, reset_output: f32) -> ClockTick {
        let mut tick = ClockTick::default();
        if !self.counting || dt <= 0.0 {
            return tick;
        }
        self.tic_progress += dt;
        if self.tic_progress >= self.period {
            self.tic_progress -= self.period;
            self.tic = 1 - self.tic;
        }
        if reset_output > CLOCK_RESET_THRESHOLD {
            self.chrono_time = 0.0;
        } else {
            self.chrono_time += dt;
        }
        self.age_progress += dt;
        if self.age_progress > AGE_EVENT_PERIOD {
            self.age_progress = 0.0;
            tick.aged = true;
        }
        self.time_alive += dt;
        if !self.day_triggered && self.time_alive >= DAY_SECONDS {
            self.day_triggered = true;
            tick.first_day = true;
        }
        tick
    }
}

// ---------------------------------------------------------------------------
// Pheromone organ
// ---------------------------------------------------------------------------

/// A pheromone spot request emitted by the organ.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PheromoneEmission {
    /// Red channel strength.
    pub r: f32,
    /// Green channel strength.
    pub g: f32,
    /// Blue channel strength.
    pub b: f32,
    /// Energy cost of this emission (E).
    pub energy_cost: f32,
}

/// Pheromone emission state (`BibitePheromoneOrgan`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PheromoneOrgan {
    /// Seconds accumulated towards the 1 s emission.
    pub progress: f32,
    /// `pheromoneCost`: the energy of the last emission (it keeps its value until the next one; the Biology panel's Flow details show it as a rate).
    pub cost: f32,
}

impl PheromoneOrgan {
    /// Creates a fresh emitter.
    pub fn new() -> Self {
        Self::default()
    }

    /// One tick (`BibitePheromoneOrgan.UpdateOrgan`, `BibitePheromoneOrgan.cs:63-83`).
    ///
    /// `outputs` are `PhereOut1..3`, `prevented` the per-channel global
    /// disable switches, `strength` the production strength setting and
    /// `cost_per_strength` the energy cost setting. Returns an emission when
    /// the sum crosses the threshold.
    pub fn tick(
        &mut self,
        dt: f32,
        outputs: [f32; 3],
        prevented: [bool; 3],
        strength: f32,
        cost_per_strength: f32,
    ) -> Option<PheromoneEmission> {
        self.progress += dt;
        if self.progress < PHEROMONE_EMIT_PERIOD {
            return None;
        }
        self.progress -= PHEROMONE_EMIT_PERIOD;
        let channel = |index: usize| {
            if prevented[index] {
                0.0
            } else {
                outputs[index] * strength
            }
        };
        let (r, g, b) = (channel(0), channel(1), channel(2));
        let total = r + g + b;
        if total > PHEROMONE_EMIT_THRESHOLD {
            Some(PheromoneEmission {
                r,
                g,
                b,
                energy_cost: total * cost_per_strength,
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Mouth: derived constants, bite geometry, grab/throw/vomit formulas
// ---------------------------------------------------------------------------

/// Settings used to derive mouth/jaw constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouthSettings {
    /// `bibiteMassDensity` (g/u²).
    pub bibite_mass_density: f32,
    /// `bitingPressure` (N/u).
    pub biting_pressure: f32,
    /// `jawMusclesSizingPower`.
    pub jaw_muscles_sizing_power: f32,
    /// `bitePeriodFactor`.
    pub bite_period_factor: f32,
}

impl Default for MouthSettings {
    fn default() -> Self {
        Self {
            bibite_mass_density: 0.025,
            biting_pressure: 80.0,
            jaw_muscles_sizing_power: 2.0,
            bite_period_factor: 1.0,
        }
    }
}

/// Derived mouth/jaw constants (`BibiteMouth.UpdateMouthSizeParameters`,
/// `BibiteMouth.cs:315-332`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MouthDerived {
    /// `jawArea = baseBodyArea * jawPortion`.
    pub jaw_area: f32,
    /// `jawMass = jawArea * bibiteMassDensity / 2`.
    pub jaw_mass: f32,
    /// `throatWidth = sqrt(throatArea)`.
    pub throat_width: f32,
    /// `jawStrength = bitingPressure * (jawArea*SpeedRatio)^(power/2)`.
    pub jaw_strength: f32,
    /// `bitePeriod = sqrt(throatWidth*jawMass/jawStrength)*70*factor`.
    pub bite_period: f32,
}

/// `jawArea = baseBodyArea * jawAreaPortion` (`BibiteMouth.cs:317`).
pub fn jaw_area(base_body_area: f32, jaw_area_portion: f32) -> f32 {
    base_body_area * jaw_area_portion
}

/// `jawMass = jawArea * bibiteMassDensity / 2` (`BibiteMouth.cs:318`).
pub fn jaw_mass(jaw_area: f32, bibite_mass_density: f32) -> f32 {
    jaw_area * bibite_mass_density / 2.0
}

/// `throatWidth = sqrt(area)` (`BibiteMouth.cs:319`).
pub fn throat_width(throat_area: f32) -> f32 {
    throat_area.sqrt()
}

/// `jawStrength = bitingPressure * (jawArea*metabolicRate)^(power/2)`
/// (`BibiteMouth.cs:331`).
pub fn jaw_strength(
    biting_pressure: f32,
    jaw_area: f32,
    metabolic_rate: f32,
    jaw_muscles_sizing_power: f32,
) -> f32 {
    biting_pressure * (jaw_area * metabolic_rate).powf(jaw_muscles_sizing_power / 2.0)
}

/// `bitePeriod = sqrt(throatWidth*jawMass/jawStrength) * 70 * factor`
/// (`BibiteMouth.cs:324-327`); `1e6` when the jaw area is zero.
pub fn bite_period(
    throat_width: f32,
    jaw_mass: f32,
    jaw_strength: f32,
    bite_period_factor: f32,
    jaw_area: f32,
) -> f32 {
    if jaw_area > 0.0 && jaw_strength > 0.0 {
        (throat_width * jaw_mass / jaw_strength).sqrt() * 70.0 * bite_period_factor
    } else {
        1_000_000.0
    }
}

/// Derives all mouth constants from the organ areas and genome/settings values.
pub fn mouth_derived(
    base_body_area: f32,
    jaw_area_portion: f32,
    throat_area: f32,
    metabolic_rate: f32,
    settings: &MouthSettings,
) -> MouthDerived {
    let jaw = jaw_area(base_body_area, jaw_area_portion);
    let mass = jaw_mass(jaw, settings.bibite_mass_density);
    let width = throat_width(throat_area);
    let strength = jaw_strength(
        settings.biting_pressure,
        jaw,
        metabolic_rate,
        settings.jaw_muscles_sizing_power,
    );
    MouthDerived {
        jaw_area: jaw,
        jaw_mass: mass,
        throat_width: width,
        jaw_strength: strength,
        bite_period: bite_period(width, mass, strength, settings.bite_period_factor, jaw),
    }
}

/// `swallowWidth = Want2Eat > 0.15 ? Want2Eat * throatWidth : 0`
/// (`BibiteMouth.cs:195-205`).
pub fn swallow_width(want_to_eat: f32, throat_width: f32) -> f32 {
    if want_to_eat > MOUTH_DEAD_ZONE {
        want_to_eat * throat_width
    } else {
        0.0
    }
}

/// `biteWidth = Want2Attack > 0.15 ? Want2Attack * throatWidth : 0`
/// (`BibiteMouth.cs:223-233`).
pub fn bite_width(want_to_attack: f32, throat_width: f32) -> f32 {
    if want_to_attack > MOUTH_DEAD_ZONE {
        want_to_attack * throat_width
    } else {
        0.0
    }
}

/// `biteForce = Want2Attack > 0.15 ? jawStrength * Want2Attack : 0`
/// (`BibiteMouth.cs:211-221`).
pub fn bite_force(want_to_attack: f32, jaw_strength: f32) -> f32 {
    if want_to_attack > MOUTH_DEAD_ZONE {
        jaw_strength * want_to_attack
    } else {
        0.0
    }
}

/// `swallowAmount = PI * (swallowWidth/2)²` (`BibiteMouth.cs:207`).
pub fn swallow_amount(width: f32) -> f32 {
    PI * (width / 2.0).powi(2)
}

/// `maxAmountNoBiteNeeded = PI * (swallowWidth/4)²` (`BibiteMouth.cs:209`).
pub fn max_amount_no_bite_needed(width: f32) -> f32 {
    PI * (width / 4.0).powi(2)
}

/// `ableToSwallow = Want2Eat > 0.15 && biteProgress >= bitePeriod`
/// (`BibiteMouth.cs:171-181`).
pub fn able_to_swallow(want_to_eat: f32, bite_progress: f32, bite_period: f32) -> bool {
    want_to_eat > MOUTH_DEAD_ZONE && bite_progress >= bite_period
}

/// `readyToAttack = Want2Attack > 0.15 && biteProgress >= bitePeriod`
/// (`BibiteMouth.cs:183-193`).
pub fn ready_to_attack(want_to_attack: f32, bite_progress: f32, bite_period: f32) -> bool {
    want_to_attack > MOUTH_DEAD_ZONE && bite_progress >= bite_period
}

/// Mouth runtime state (`BibiteMouth` fields; no Unity objects).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouthState {
    /// Seconds until the next bite (`biteProgress`).
    pub bite_progress: f32,
    /// Attacked in the previous tick (`attackedLastFrame`).
    pub attacked_last_frame: bool,
    /// Attacked this tick (`attackedThisFrame`).
    pub attacked_this_frame: bool,
    /// Number of first bites (`bibitesBitten`).
    pub bibites_bitten: u32,
    /// Total damage dealt (`totalDamageDealt`).
    pub total_damage_dealt: f32,
    /// Murders (`totalMurders`).
    pub total_murders: u32,
    /// Murders' area (`murderedArea`).
    pub murdered_area: f32,
    /// Current `Want2Attack` (latched by `UpdateOrgan`).
    pub desire_attack: f32,
    /// Current `Want2Eat`.
    pub desire_eat: f32,
    /// Current `Grab`.
    pub desire_grab: f32,
    /// Cached `mouthOpening` (swallow width).
    pub mouth_opening: f32,
    /// Cached `biteOpening` (bite width).
    pub bite_opening: f32,
    /// Cached bite force.
    pub bite_strength: f32,
    /// Cached `canSwallow`.
    pub can_swallow: bool,
    /// Objects inside the mouth trigger.
    pub n_in_mouth: usize,
    /// Objects held by grab joints.
    pub n_held: usize,
}

impl MouthState {
    /// Creates an idle mouth.
    pub fn new() -> Self {
        Self {
            bite_progress: 0.0,
            attacked_last_frame: false,
            attacked_this_frame: false,
            bibites_bitten: 0,
            total_damage_dealt: 0.0,
            total_murders: 0,
            murdered_area: 0.0,
            desire_attack: 0.0,
            desire_eat: 0.0,
            desire_grab: 0.0,
            mouth_opening: 0.0,
            bite_opening: 0.0,
            bite_strength: 0.0,
            can_swallow: false,
            n_in_mouth: 0,
            n_held: 0,
        }
    }

    /// Start-of-tick bookkeeping (`BibiteMouth.cs:342-347`): rolls the attack
    /// flags and recharges the bite period.
    pub fn begin_tick(&mut self, dt: f32, bite_period: f32) {
        self.attacked_last_frame = self.attacked_this_frame;
        self.attacked_this_frame = false;
        if self.bite_progress < bite_period {
            self.bite_progress += dt;
        }
    }

    /// Latches the per-tick brain outputs and derived values
    /// (`BibiteMouth.cs:408-414`).
    pub fn latch_outputs(&mut self, attack: f32, eat: f32, grab: f32, derived: &MouthDerived) {
        self.desire_attack = attack;
        self.desire_eat = eat;
        self.desire_grab = grab;
        self.can_swallow = able_to_swallow(eat, self.bite_progress, derived.bite_period);
        self.mouth_opening = swallow_width(eat, derived.throat_width);
        self.bite_opening = bite_width(attack, derived.throat_width);
        self.bite_strength = bite_force(attack, derived.jaw_strength);
    }

    /// `biteProgress >= bitePeriod`.
    pub fn bite_ready(&self, bite_period: f32) -> bool {
        self.bite_progress >= bite_period
    }

    /// Consumes one bite period (`biteProgress -= bitePeriod`).
    pub fn consume_bite_period(&mut self, bite_period: f32) {
        self.bite_progress -= bite_period;
    }

    /// Records a first bite (`bibitesBitten++`, `attackedThisFrame = true`).
    pub fn record_first_bite(&mut self) {
        self.bibites_bitten += 1;
        self.attacked_this_frame = true;
    }

    /// Records a murder and returns whether the reaper easter egg unlocks
    /// (`BibiteMouth.cs:730-738`).
    pub fn record_murder(&mut self, victim_area: f32, own_area: f32) -> bool {
        self.total_murders += 1;
        self.murdered_area += victim_area;
        self.total_murders >= 50 && self.murdered_area >= 50.0 * own_area
    }
}

impl Default for MouthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Half-angle of the first circular segment (`BibiteMouth.cs:667`).
pub fn bite_segment_angle(k: f32, mouth_opening: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    ((k.min(1.0) * mouth_opening) / (2.0 * radius))
        .min(1.0)
        .asin()
}

/// Newton solution for the second bite segment angle for `k > 1`
/// (`BibiteMouth.cs:669-683`).
///
/// Uses the exact seed and exactly two iterations, not a convergence loop.
pub fn newton_bite_angle(k: f32) -> f32 {
    let seed = 0.49 * ((k - 1.0).ln() - 0.3125);
    let exponent = (seed / (16f32.powf(seed) - 1.0)).exp();
    let mut theta = (PI / 2.0) * (1.0 - 1.0 / k.powf(exponent));
    for _ in 0..2 {
        let sin = theta.sin();
        let cos = theta.cos();
        theta -= (theta - k * sin * cos) / (1.0 - theta * cos / sin + k * sin * sin);
    }
    theta
}

/// The chunk-area computation of a bite: `A1` plus, when `K_B > 1`, `A2`
/// (`BibiteMouth.cs:666-683`).
///
/// * `mouth_opening` is `h_m` (scaled to the target's internals for bibites),
/// * `force` is `F` (after the first-bite armor subtraction for bibites),
/// * `cohesiveness` is the bitten material's σ_y,
/// * `radius` is the target's radius (`internalsRadius` for bibites).
pub fn bite_chunk_area(input: &BiteInput) -> f32 {
    if input.mouth_opening <= 0.0 || input.radius <= 0.0 || input.cohesiveness <= 0.0 {
        return 0.0;
    }
    let k = input.force / (input.mouth_opening * input.cohesiveness);
    let theta = bite_segment_angle(k, input.mouth_opening, input.radius);
    let mut area = input.radius * input.radius * (theta - theta.sin() * theta.cos());
    if k > 1.0 {
        let theta_b = newton_bite_angle(k);
        let sin = theta_b.sin();
        let radius_b = input.mouth_opening / (2.0 * sin);
        area += radius_b * radius_b * (theta_b - sin * theta_b.cos());
    }
    area
}

/// Input of [`bite_chunk_area`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BiteInput {
    /// `h_m`: maximum mouth opening scaled to the target.
    pub mouth_opening: f32,
    /// `F`: bite force (armor already subtracted for a bibite first bite).
    pub force: f32,
    /// σ_y of the bitten material.
    pub cohesiveness: f32,
    /// Target radius.
    pub radius: f32,
}

/// Builds the bite input for a pellet (`BibiteMouth.cs:628-633,684-709`).
pub fn pellet_bite_input(
    mouth_opening: f32,
    bite_strength: f32,
    cohesiveness: f32,
    pellet_radius: f32,
) -> BiteInput {
    BiteInput {
        mouth_opening,
        force: bite_strength,
        cohesiveness,
        radius: pellet_radius,
    }
}

/// Builds the bite input for a bibite target (`BibiteMouth.cs:640-653`).
///
/// On a first bite (`!ongoing`) the target's doubled armor strength is
/// subtracted from the force (floored at zero, [`crate::body::reduced_bite_force`]).
pub fn bibite_bite_input(
    bite_width: f32,
    bite_strength: f32,
    armor_strength: f32,
    internals_radius: f32,
    effective_radius: f32,
    ongoing: bool,
) -> BiteInput {
    let force = if ongoing {
        bite_strength
    } else {
        crate::body::reduced_bite_force(bite_strength, armor_strength)
    };
    let mouth_opening = if effective_radius > 0.0 {
        bite_width * internals_radius / effective_radius
    } else {
        0.0
    };
    BiteInput {
        mouth_opening,
        force,
        cohesiveness: MaterialParams::MEAT.cohesiveness,
        radius: internals_radius,
    }
}

/// Result of applying the damage rules of a bite.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BiteDamage {
    /// Damage to apply to the victim (already scaled for ongoing bites).
    pub damage: f32,
    /// True when the target's armor stopped all damage.
    pub armor_blocked: bool,
}

/// Input of [`resolve_bite_damage`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BiteDamageInput {
    /// Original `biteStrength` (before armor subtraction).
    pub bite_strength: f32,
    /// Target's doubled armor strength (`BibiteBody.armorStrength`).
    pub armor_strength: f32,
    /// Victim max health.
    pub target_max_health: f32,
    /// Victim real body area (inflated).
    pub target_real_body_area: f32,
    /// Chunk area from [`bite_chunk_area`].
    pub chunk_area: f32,
    /// True for a continuing attack.
    pub ongoing: bool,
    /// Victim's bite period.
    pub bite_period: f32,
    /// Simulation delta time.
    pub dt: f32,
    /// `bitingDamageFactor` setting.
    pub damage_factor: f32,
}

/// Damage rules of `MaybeAttack` (`BibiteMouth.cs:710-728`).
///
/// * damage `= factor * maxHealth * chunk / realArea`,
/// * ongoing attacks scale by `dt / (5 * bitePeriod)`,
/// * armor blocks completely when `biteStrength <= armorStrength`.
pub fn resolve_bite_damage(input: &BiteDamageInput) -> BiteDamage {
    if input.target_real_body_area <= 0.0 {
        return BiteDamage::default();
    }
    let mut damage = input.damage_factor * input.target_max_health * input.chunk_area
        / input.target_real_body_area;
    if crate::body::armor_blocks(input.bite_strength, input.armor_strength) {
        return BiteDamage {
            damage: 0.0,
            armor_blocked: true,
        };
    }
    if input.ongoing {
        damage *= crate::body::ongoing_bite_damage_scale(input.dt, input.bite_period);
    }
    BiteDamage {
        damage,
        armor_blocked: false,
    }
}

/// Result of evaluating a whole-pellet swallow (`BibiteMouth.TrySwallowTarget`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PelletSwallow {
    /// Pellet radius does not fit in the mouth opening (`r > mouthOpening/2`).
    TooBig,
    /// Not enough stomach space.
    NoSpace,
    /// Pellet is medium-sized and the bite period is not ready.
    NeedsBite,
    /// The pellet can be swallowed this tick.
    Swallow,
}

/// Whole-pellet swallow rules (`BibiteMouth.cs:538-562`).
pub fn try_swallow_pellet(
    pellet_radius: f32,
    pellet_amount: f32,
    available_space: f32,
    mouth_opening: f32,
    bite_progress: f32,
    bite_period: f32,
) -> PelletSwallow {
    if pellet_radius > mouth_opening / 2.0 {
        return PelletSwallow::TooBig;
    }
    if pellet_amount > available_space {
        return PelletSwallow::NoSpace;
    }
    if pellet_radius > mouth_opening / 4.0 && bite_progress < bite_period {
        return PelletSwallow::NeedsBite;
    }
    PelletSwallow::Swallow
}

/// Result of evaluating a whole-bibite swallow (`BibiteMouth.TrySwallowTarget`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BibiteSwallow {
    /// Prey length does not fit the mouth opening.
    TooBig,
    /// Prey meat + prey stomach content exceeds the biter's stomach space.
    NoSpace,
    /// Prey is medium-sized and the bite period is not ready.
    NeedsBite,
    /// The prey can be swallowed and destroyed this tick.
    Swallow,
}

/// Whole-bibite swallow rules (`BibiteMouth.cs:563-594`).
pub fn try_swallow_bibite(
    prey_length: f32,
    prey_meat_amount: f32,
    prey_stomach_amount: f32,
    available_space: f32,
    mouth_opening: f32,
    bite_progress: f32,
    bite_period: f32,
) -> BibiteSwallow {
    if prey_length > mouth_opening {
        return BibiteSwallow::TooBig;
    }
    if prey_meat_amount + prey_stomach_amount > available_space {
        return BibiteSwallow::NoSpace;
    }
    if prey_length > mouth_opening / 2.0 && bite_progress < bite_period {
        return BibiteSwallow::NeedsBite;
    }
    BibiteSwallow::Swallow
}

/// Action chosen for one object inside the mouth (`BibiteMouth.cs:442-456`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetAction {
    /// Try to swallow the target.
    Swallow,
    /// Bite/attack the target.
    Attack,
}

/// Swallow-vs-attack priority for one mouth object (`BibiteMouth.cs:442-456`).
pub fn target_action(
    can_swallow: bool,
    want_to_eat: f32,
    grabbed: bool,
    grab_output: f32,
) -> TargetAction {
    let threshold = if grabbed {
        grab_output
    } else {
        MOUTH_DEAD_ZONE
    };
    if can_swallow && want_to_eat > threshold {
        TargetAction::Swallow
    } else {
        TargetAction::Attack
    }
}

/// Grab joint break force/torque (`BibiteMouth.cs:430-436`).
///
/// The `max(diet, 0.1)` factor is the herbivore grip penalty.
pub fn grab_break_force(jaw_strength: f32, diet: f32, desire_to_grab: f32, n_held: usize) -> f32 {
    if n_held == 0 {
        0.0
    } else {
        jaw_strength * diet.max(0.1) * desire_to_grab / n_held as f32
    }
}

/// `throwForce = throwingFactor * jawStrength * |desire| * Mtotal/(jawMass+Mtotal)`
/// (`BibiteMouth.cs:791-792`). The caller divides by `n_held` per object.
pub fn throw_force_magnitude(
    throwing_force_factor: f32,
    jaw_strength: f32,
    desire_to_grab: f32,
    total_held_mass: f32,
    jaw_mass: f32,
) -> f32 {
    throwing_force_factor * jaw_strength * desire_to_grab.abs() * total_held_mass
        / (jaw_mass + total_held_mass)
}

/// Bite knockback impulse (`BibiteMouth.cs:770-772`).
pub fn bite_knockback_impulse(
    throwing_force_factor: f32,
    biting_throw_force_factor: f32,
    bite_strength: f32,
) -> f32 {
    throwing_force_factor * biting_throw_force_factor * bite_strength
}

/// Fraction of stomach content vomited (`BibiteMouth.cs:879`):
/// `|Want2Eat| / 0.95`.
pub fn vomit_fraction(want_to_eat: f32) -> f32 {
    (want_to_eat / 0.95).abs()
}

/// Vomit throw magnitude (`BibiteMouth.cs:896`).
pub fn vomit_throw_magnitude(
    throwing_force_factor: f32,
    jaw_strength: f32,
    fraction: f32,
    total_mass: f32,
    jaw_mass: f32,
) -> f32 {
    throwing_force_factor * jaw_strength * fraction * total_mass / (jaw_mass + total_mass)
}

// ---------------------------------------------------------------------------
// Propulsion
// ---------------------------------------------------------------------------

/// Tunable settings used by propulsion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropulsionSettings {
    /// `armMusclePressure` (N/u).
    pub arm_muscle_pressure: f32,
    /// `forwardForceSizePower`.
    pub forward_force_size_power: f32,
    /// `turnForceSizePower`.
    pub turn_force_size_power: f32,
    /// `backwardFraction`.
    pub backward_fraction: f32,
    /// `moveMusclesCost` (E/(u²·s)).
    pub move_muscles_cost: f32,
}

impl Default for PropulsionSettings {
    fn default() -> Self {
        Self {
            arm_muscle_pressure: 50.0,
            forward_force_size_power: 1.5,
            turn_force_size_power: 3.0,
            backward_fraction: 0.5,
            move_muscles_cost: 0.02,
        }
    }
}

/// Derived propulsion constants (`BibitePropulsion.UpdateMuscleStrength` /
/// `UpdateMuscleCosts`, `BibitePropulsion.cs:178-189`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropulsionDerived {
    /// `muscleMoveStrength = pressure * (area*SpeedRatio)^(forwardPower/2)`.
    pub muscle_move_strength: f32,
    /// `muscleTurnStrength = pressure/4 * (area*SpeedRatio)^(turnPower/2)`.
    pub muscle_turn_strength: f32,
    /// `baseAccelerationCost = moveMusclesCost * area`.
    pub base_acceleration_cost: f32,
    /// `baseTurnCost = baseAccelerationCost / 2`.
    pub base_turn_cost: f32,
}

/// Derives propulsion constants from the movable-muscle organ area.
pub fn propulsion_derived(
    move_area: f32,
    metabolic_rate: f32,
    settings: &PropulsionSettings,
) -> PropulsionDerived {
    let power_base = move_area * metabolic_rate;
    PropulsionDerived {
        muscle_move_strength: settings.arm_muscle_pressure
            * power_base.powf(settings.forward_force_size_power / 2.0),
        muscle_turn_strength: settings.arm_muscle_pressure / 4.0
            * power_base.powf(settings.turn_force_size_power / 2.0),
        base_acceleration_cost: settings.move_muscles_cost * move_area,
        base_turn_cost: settings.move_muscles_cost * move_area / 2.0,
    }
}

/// Per-tick kinematics written back into the brain senses
/// (`BibitePropulsion.UpdateOrgan`, `BibitePropulsion.cs:248-262`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropulsionState {
    /// Position at the previous tick (`lastPos`).
    pub last_pos: Vec2,
    /// Total path length travelled (`totalTravel`).
    pub total_travel: f32,
    /// Forward speed along `up` (`SignedVelocity`).
    pub signed_velocity: f32,
    /// Signed velocity divided by body length (`NormalizedVelocity`).
    pub normalized_velocity: f32,
    /// Angular velocity in turns/second (`AngularVelocity`).
    pub angular_velocity: f32,
    /// Velocity angle in degrees matching `BibitePropulsion.Angle`.
    pub angle_degrees: f32,
}

impl PropulsionState {
    /// Creates a state at `position`.
    pub fn new(position: Vec2) -> Self {
        Self {
            last_pos: position,
            ..Self::default()
        }
    }

    /// Updates travel/velocity senses for this tick.
    pub fn update_kinematics(
        &mut self,
        position: Vec2,
        velocity: Vec2,
        angular_velocity_degrees_per_second: f32,
        up: Vec2,
        body_length: f32,
    ) {
        self.total_travel += position.distance(self.last_pos);
        self.last_pos = position;
        self.angular_velocity = angular_velocity_degrees_per_second / 360.0;
        self.signed_velocity = velocity.dot(up);
        self.normalized_velocity = if body_length > 0.0 {
            self.signed_velocity / body_length
        } else {
            0.0
        };
        let mut angle = velocity.angle().to_degrees();
        if angle < 0.0 {
            angle += 360.0;
        }
        self.angle_degrees = angle;
    }
}

/// Brain-driven movement request plus body/gene context.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementInput {
    /// `Accelerate` output.
    pub accelerate: f32,
    /// `Rotate` output.
    pub rotate: f32,
    /// `Herding` output.
    pub herding: f32,
    /// Whether the field of view has herd neighbours.
    pub has_herd: bool,
    /// Signed direction to the herd, degrees (`herdSumDirection`).
    pub herd_sum_direction_degrees: f32,
    /// Herd separation projection (`herdSeparationProjection`).
    pub herd_separation_projection: f32,
    /// `HerdVelocityWeight` gene.
    pub herd_velocity_weight: f32,
    /// `Health.Amount / Health.MaxAmount`.
    pub health_fullness: f32,
    /// `ageStrengthPenaltyFactor`.
    pub age_strength_penalty: f32,
}

/// Force/torque/cost command produced by [`compute_movement`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementCommand {
    /// Force to add (ForceMode2D.Force), N.
    pub force: Vec2,
    /// Torque to add (ForceMode2D.Force).
    pub torque: f32,
    /// Energy rate (E/s) to spend for this movement.
    pub movement_cost: f32,
}

/// `movementPenalty = sqrt(healthFullness) * ageStrengthPenaltyFactor`
/// (`BibitePropulsion.cs:265`).
pub fn movement_penalty(health_fullness: f32, age_strength_penalty: f32) -> f32 {
    health_fullness.max(0.0).sqrt() * age_strength_penalty
}

/// Herding turn blend (`BibitePropulsion.cs:269`):
/// `turn*(1-|H|) + tanh(herdSumDirection/90) * H`.
pub fn herding_steering(turn: f32, herding: f32, herd_sum_direction_degrees: f32) -> f32 {
    turn * (1.0 - herding.abs()) + tanh(herd_sum_direction_degrees / 90.0) * herding
}

/// Blends `turn` toward a signed target angle with a `weight` in `0..=1`
/// (`tanh` of the angle in degrees, as the game does for void/shade avoidance).
pub fn blend_turn_towards(turn: f32, signed_angle_degrees: f32, weight: f32) -> f32 {
    let weight = clamp01(weight);
    turn * (1.0 - weight) + tanh(signed_angle_degrees) * weight
}

/// Computes the movement command (`BibitePropulsion.UpdateOrgan`).
///
/// Order is exact: herding blend, clamp to `[-1,1] * penalty`, backward
/// fraction, then force/torque/cost.
pub fn compute_movement(
    derived: &PropulsionDerived,
    input: &MovementInput,
    up: Vec2,
    backward_fraction: f32,
) -> MovementCommand {
    let penalty = movement_penalty(input.health_fullness, input.age_strength_penalty);
    let mut accelerate = input.accelerate;
    let mut turn = input.rotate;
    if input.has_herd {
        turn = herding_steering(turn, input.herding, input.herd_sum_direction_degrees);
        accelerate += input.herd_velocity_weight * input.herd_separation_projection * input.herding;
    }
    accelerate = clamp(accelerate, -1.0, 1.0) * penalty;
    turn = clamp(turn, -1.0, 1.0) * penalty;
    if accelerate < 0.0 {
        accelerate *= backward_fraction;
    }
    MovementCommand {
        force: up * (accelerate * derived.muscle_move_strength),
        torque: turn * derived.muscle_turn_strength,
        movement_cost: derived.base_acceleration_cost * accelerate.abs()
            + derived.base_turn_cost * turn.abs(),
    }
}

/// `OutOfBoundDist2 = simSize² * 2.25` (`BibitePropulsion.cs:78-80`).
pub fn out_of_bound_dist_sq(sim_size: f32) -> f32 {
    sim_size * sim_size * 2.25
}

/// Shade-avoidance steering blend (`BibitePropulsion.cs:298-302`): returns the
/// new turn after blending toward `-pos` with weight
/// `clamp01(InverseLerp(out2, out2*1.5, distanceSq))`.
///
/// `angle_to_origin_degrees` is `SignedAngle(up, -position)` in degrees.
pub fn shade_avoidance_steering(
    turn: f32,
    distance_sq: f32,
    out_bound_dist_sq: f32,
    angle_to_origin_degrees: f32,
) -> f32 {
    let weight = clamp01(inv_lerp(
        out_bound_dist_sq,
        out_bound_dist_sq * 1.5,
        distance_sq,
    ));
    if weight <= 0.0 {
        turn
    } else {
        blend_turn_towards(turn, angle_to_origin_degrees, weight)
    }
}

/// Void-avoidance blend weight (`BibitePropulsion.cs:288-291`): the fraction
/// of the turn toward the zone that should be applied, given the distance
/// outside the zone and the `voidAvoidanceDistance` setting.
pub fn void_avoidance_weight(distance_outside: f32, void_avoidance_distance: f32) -> f32 {
    if void_avoidance_distance > 0.0 {
        (distance_outside / void_avoidance_distance).min(1.0)
    } else {
        1.0
    }
}

/// World-wrap threshold: `simSize*1.5 + 1000` (`BibitePropulsion.cs:303-305`).
pub fn world_wrap_threshold(sim_size: f32) -> f32 {
    sim_size * 1.5 + 1000.0
}

/// World-wrap reposition when the bibite has passed through the shade
/// (`BibitePropulsion.cs:303-306`).
///
/// Returns the new position when the wrap condition holds, else `None`.
pub fn world_wrap_position(position: Vec2, body_length: f32, sim_size: f32) -> Option<Vec2> {
    let magnitude = position.length();
    let threshold = world_wrap_threshold(sim_size);
    if magnitude <= 0.0 || magnitude - body_length < threshold {
        return None;
    }
    Some(position.normalized() * (body_length * 0.95 - threshold))
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

    fn close_to(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected} ± {tolerance}, got {actual}"
        );
    }

    #[test]
    fn material_efficiency_and_energy_conversions() {
        let plant = MaterialParams::PLANT;
        assert_close(plant.conversion_efficiency(1.0), 0.5);
        assert_close(plant.conversion_efficiency(0.0), -0.25);
        assert_close(plant.conversion_efficiency(0.5), 0.125);
        assert_close(MaterialParams::MEAT.conversion_efficiency(0.0), 0.15);
        assert_close(MaterialParams::MEAT.conversion_efficiency(1.0), 0.85);
        assert_close(plant.energy_of_amount(3.0), 15.0);
        assert_close(plant.amount_of_energy(15.0), 3.0);
        assert_close(plant.radius_of_amount(PI), 1.0);
    }

    #[test]
    fn threshold_remap_matches_game() {
        assert_close(threshold_remap(1.0, 0.15), 1.0);
        assert_close(threshold_remap(0.15, 0.15), 0.0);
        assert_close(threshold_remap(0.575, 0.15), 0.5);
        assert_close(threshold_remap(-1.0, 0.15), -1.0);
    }

    #[test]
    fn stomach_affinity_and_efficiency() {
        let mut stomach = Stomach::new(100.0, 0.3, 1.0, 1.0, 1.0);
        assert_close(stomach.affinity(StomachMaterial::Plant), 0.7);
        assert_close(stomach.affinity(StomachMaterial::Meat), 0.3);
        stomach.add_matter(
            StomachMaterial::Plant,
            10.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        assert_close(stomach.content(0).unwrap().efficiency, 0.275);
        stomach.refresh_efficiencies(&MaterialParams::PLANT, &MaterialParams::MEAT);
        assert_close(stomach.content(0).unwrap().efficiency, 0.275);
    }

    #[test]
    fn stomach_add_merge_and_capacity() {
        let mut stomach = Stomach::new(20.0, 0.5, 1.0, 1.0, 1.0);
        let added = stomach.add_matter(
            StomachMaterial::Plant,
            10.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        assert_close(added, 10.0);
        let added = stomach.add_matter(
            StomachMaterial::Plant,
            15.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        assert_close(added, 10.0); // clamped to 10 u² of remaining space
        assert_close(stomach.total_amount(), 20.0);
        assert_close(stomach.fullness(), 1.0);
        assert_eq!(stomach.available_space(), 0.0);
        assert_eq!(stomach.n_content(), 1);
        // averageChunkAmount merge formula: oldAvg 10, amount 10.
        assert_close(stomach.content(0).unwrap().average_chunk_amount, 10.0);
        // A second material uses the second slot.
        let mut stomach = Stomach::new(20.0, 0.5, 1.0, 1.0, 1.0);
        stomach.add_matter(
            StomachMaterial::Plant,
            5.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        stomach.add_matter(
            StomachMaterial::Meat,
            5.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        assert_eq!(stomach.n_content(), 2);
        assert_close(stomach.mass(), 5.0 * 0.015 + 5.0 * 0.025);
        assert_close(stomach.remove_matter(StomachMaterial::Plant, 2.0), 2.0);
        assert_close(stomach.total_amount(), 8.0);
    }

    #[test]
    fn stomach_digestion_matches_hand_calculation() {
        let mut stomach = Stomach::new(100.0, 0.3, 1.0, 1.0, 1.0);
        stomach.add_matter(
            StomachMaterial::Plant,
            10.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        // One tick: fullness 0.1, acid 0.5 => ratio 5 => effectiveness 1.4.
        let tick = stomach.update(
            1.0 / 30.0,
            0.5,
            1.0,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        assert_close(tick.acid_ratio, 5.0);
        assert_close(tick.potential_bonus, 0.4);
        assert_close(tick.efficiency_malus, 0.2);
        assert_close(tick.effectiveness, 1.4);
        // r=sqrt(10/PI), d=0.03*1.4/30=0.0014, segment area d*(2r-d)*PI.
        let radius = (10.0f32 / PI).sqrt();
        let depth = 0.03 * 1.4 / 30.0;
        let segment = depth * (2.0 * radius - depth) * PI;
        let efficiency = 0.275 * (1.0 - 0.2);
        assert_close(tick.energy_gain, segment * 5.0 * efficiency);
        assert_close(stomach.content(0).unwrap().amount, 10.0 - segment);
        assert_close(stomach.content(0).unwrap().digestion_rate, segment * 30.0);
        assert_close(
            stomach.content(0).unwrap().last_conversion_efficiency,
            efficiency,
        );
    }

    #[test]
    fn stomach_full_dissolution_compacts_slot() {
        let mut stomach = Stomach::new(100.0, 0.0, 1.0, 1.0, 1.0);
        stomach.add_matter(
            StomachMaterial::Meat,
            0.001,
            &MaterialParams::PLANT,
            &MaterialParams::MEAT,
        );
        let tick = stomach.update(1.0, 1.0, 1.0, &MaterialParams::PLANT, &MaterialParams::MEAT);
        assert_eq!(tick.finished_slots, 1);
        assert_eq!(stomach.n_content(), 0);
        assert!(tick.energy_gain.is_finite());
    }

    #[test]
    fn fat_deadband_and_storage_sign() {
        let mut fat = FatOrgan::new(0.2, 100.0, 0.5, 0.25);
        // Deadband: energy ratio 0.5 +- 0.25 => no storage.
        let tick = fat.update(1.0 / 30.0, 0.5, 1.0, 1.0, 0.0, &MaterialParams::FAT);
        assert_eq!(tick.amount_delta, 0.0);
        assert_eq!(tick.energy_gain, 0.0);
        // Ratios outside the deadband (0.25 ..= 0.75).
        let mut above = FatOrgan::new(0.2, 100.0, 0.5, 0.25);
        let tick = above.update(1.0 / 30.0, 0.8, 1.0, 1.0, 0.0, &MaterialParams::FAT);
        assert!(tick.amount_delta > 0.0);
        assert!(tick.energy_gain < 0.0); // storing costs energy
        let mut below = FatOrgan::new(0.2, 100.0, 0.5, 0.25);
        below.amount = 5.0;
        let tick = below.update(1.0 / 30.0, 0.2, 1.0, 1.0, 0.0, &MaterialParams::FAT);
        assert!(tick.amount_delta < 0.0);
        assert!(tick.energy_gain > 0.0); // burning yields energy
    }

    #[test]
    fn fat_inflation_and_capacity() {
        let fat = FatOrgan::new(0.2, 100.0, 0.5, 0.25);
        // baseCapacity 20, area = 20 * min(2, 1+0.35/0.2=2.75) = 40.
        assert_close(fat.base_capacity, 20.0);
        assert_close(fat.area, 40.0);
        assert_close(fat.max_amount, 40.0);
        assert_close(fat.max_inflate_amount, 35.0);
        // The inactive branch returns before recomputing inflation.
        let mut fat = FatOrgan::new(0.2, 100.0, 0.5, 0.25);
        fat.amount = 20.0 + 17.5;
        let tick = fat.update(1.0 / 30.0, 0.5, 1.0, 1.0, 0.0, &MaterialParams::FAT);
        assert_eq!(tick.inflate_factor, 0.0);
        // FatWAG portion 1.0: base capacity 100, area 135, max inflate 35.
        let mut full = FatOrgan::new(1.0, 100.0, 0.5, 0.25);
        full.amount = 134.999;
        let _ = full.update(1.0, 0.9, 1.0, 1.0, 0.0, &MaterialParams::FAT);
        assert_close(full.inflate_factor, 1.0);
        assert!(full.mass(&MaterialParams::FAT) > 0.0);
    }

    #[test]
    fn armor_thickness_numeric() {
        let d1 = 1.310_16f32.sqrt();
        let portion = 0.066_844_9 / 13.101_6;
        let thickness = armor_thickness(portion, d1, 0.0);
        assert_close(thickness, 0.014_622);
        assert_close(thickness * 500.0, 7.309_3);
        let armor = ArmorOrgan::new(portion, d1, 0.0, 500.0);
        assert_close(armor.effective_strength(), 14.618_5);
        assert_close(
            armor.mass(104.812_8 * portion, &MaterialParams::ARMOR),
            104.812_8 * portion * 0.05,
        );
    }

    #[test]
    fn egg_layer_production_and_laying() {
        // Womb area 20, eggSize 20 => capacity 1, LayTime 15 s.
        let mut layer = EggLayer::new(20.0, 20.0, 138.66, 15.0, false);
        assert_eq!(layer.capacity, 1);
        // Immature: no progress.
        let tick = layer.update(1.0, 1.0, 1.0, 1.0, false);
        assert_eq!(tick, EggLayerTick::default());
        layer.is_mature = true;
        // Below the 0.15 dead zone: no progress.
        let tick = layer.update(1.0, 0.1, 1.0, 1.0, false);
        assert_eq!(tick.progress_rate, 0.0);
        // Full production: 1 egg per 15 s.
        let tick = layer.update(1.0, 1.0, 1.0, 1.0, false);
        assert_close(tick.progress_rate, 1.0 / 15.0);
        assert_close(tick.energy_cost, 138.66 / 15.0);
        // Fast-forward to a complete egg.
        for _ in 0..20 {
            layer.update(1.0, 1.0, 1.0, 1.0, false);
        }
        let tick = layer.update(1.0, 1.0, 1.0, 1.0, false);
        assert_eq!(tick.eggs_to_lay, 1);
        layer.on_eggs_laid(tick.eggs_to_lay);
        assert_eq!(layer.n_laid, 1);
        assert_eq!(layer.n_eggs(), 0);
        // Birth limit blocks laying but progress accumulates.
        for _ in 0..15 {
            layer.update(1.0, 1.0, 1.0, 1.0, true);
        }
        assert_eq!(layer.n_eggs(), 1);
    }

    #[test]
    fn egg_layer_reabsorbs_at_half_efficiency() {
        let mut layer = EggLayer::new(20.0, 20.0, 100.0, 15.0, true);
        layer.progress = 1.0;
        let tick = layer.update(15.0, -1.0, 1.0, 0.0, false);
        assert!(tick.energy_gain > 0.0);
        assert_eq!(tick.energy_cost, 0.0);
        assert!(layer.progress <= 0.0);
    }

    #[test]
    fn egg_incubation_paths() {
        let mut egg = Egg::new(10.0, 101.16, 138.66);
        assert!(egg.start_hatch());
        assert_eq!(egg.tick(1.0), EggTick::Progress { ratio: 0.1 });
        match egg.tick(9.0) {
            EggTick::Hatched => {}
            other => panic!("expected hatch, got {other:?}"),
        }
        // Not enough energy to start (< half required).
        let mut poor = Egg::new(10.0, 100.0, 40.0);
        assert!(!poor.start_hatch());
        assert!(poor.aborting);
        // Starts but aborts mid-way when the budget runs out.
        let mut mid = Egg::new(10.0, 100.0, 60.0);
        assert!(mid.start_hatch());
        let result = mid.tick(7.0);
        assert_eq!(result, EggTick::Aborted);
        // Zero hatch time aborts immediately.
        let mut zero = Egg::new(0.0, 10.0, 100.0);
        assert!(!zero.start_hatch());
        assert_close(egg_energy_investment(138.66, 0.000_75, 1.0), 138.66);
        assert_close(egg_required_energy(101.16, 0.000_75, 1.0), 101.16);
    }

    #[test]
    fn clock_ticks_and_age_events() {
        let mut clock = InternalClock::new(1.0);
        let tick = clock.tick(0.5, 0.0);
        assert!(!tick.aged);
        assert_eq!(clock.tic, 0);
        let tick = clock.tick(0.5, 0.0);
        assert_eq!(clock.tic, 1);
        assert!(!tick.aged);
        let mut aged = false;
        for _ in 0..120 {
            if clock.tick(1.0, 0.0).aged {
                aged = true;
            }
        }
        assert!(aged);
        assert!(clock.time_alive > 0.0);
        // Clock reset zeroes the chronometer.
        clock.tick(1.0, 0.9);
        assert_eq!(clock.chrono_time, 0.0);
        // Stop/resume.
        clock.stop();
        let before = clock.time_alive;
        clock.tick(1.0, 0.0);
        assert_eq!(clock.time_alive, before);
        clock.resume();
        clock.tick(1.0, 0.0);
        assert!(clock.time_alive > before);
    }

    #[test]
    fn clock_day_achievement_fires_once() {
        let mut clock = InternalClock::new(1.0);
        clock.time_alive = DAY_SECONDS - 0.5;
        let tick = clock.tick(1.0, 0.0);
        assert!(tick.first_day);
        let tick = clock.tick(1.0, 0.0);
        assert!(!tick.first_day);
    }

    #[test]
    fn pheromone_emission_cadence_and_limits() {
        let mut organ = PheromoneOrgan::new();
        assert!(
            organ
                .tick(0.5, [1.0, 0.0, 0.0], [false; 3], 10.0, 0.01)
                .is_none()
        );
        let emission = organ
            .tick(0.5, [1.0, 0.5, 0.0], [false; 3], 10.0, 0.01)
            .unwrap();
        assert_close(emission.r, 10.0);
        assert_close(emission.g, 5.0);
        assert_close(emission.energy_cost, 15.0 * 0.01);
        // Below threshold: no spot.
        let mut organ = PheromoneOrgan::new();
        assert!(
            organ
                .tick(1.0, [0.001, 0.0, 0.0], [false; 3], 10.0, 0.01)
                .is_none()
        );
        // Prevented channel contributes zero.
        let mut organ = PheromoneOrgan::new();
        let emission = organ
            .tick(1.0, [1.0, 1.0, 0.0], [true, false, false], 10.0, 0.01)
            .unwrap();
        assert_eq!(emission.r, 0.0);
        assert_close(emission.g, 10.0);
    }

    #[test]
    fn mouth_constants_starter_bibite() {
        let derived = mouth_derived(
            104.812_8,
            1.069_518_8 / 13.101_6,
            104.812_8 * 1.310_160_4 / 13.101_6,
            1.0,
            &MouthSettings::default(),
        );
        assert_close(derived.jaw_area, 8.556_15);
        assert_close(derived.jaw_mass, 0.106_952);
        assert_close(derived.throat_width, 3.237_48);
        assert_close(derived.jaw_strength, 684.492);
        assert_close(derived.bite_period, 1.574_388);
        // Cached output helpers.
        assert_close(swallow_width(1.0, derived.throat_width), 3.237_48);
        assert_eq!(swallow_width(0.1, derived.throat_width), 0.0);
        assert_close(bite_force(0.5, derived.jaw_strength), 342.246);
        assert_eq!(bite_force(0.1, derived.jaw_strength), 0.0);
        assert_close(swallow_amount(3.237_48), PI * (3.237_48f32 / 2.0).powi(2));
        assert_close(
            max_amount_no_bite_needed(3.237_48),
            PI * (3.237_48f32 / 4.0).powi(2),
        );
    }

    #[test]
    fn bite_chunk_geometry_phase_one_and_two() {
        // K = 0.5: no second segment.
        let input = BiteInput {
            mouth_opening: 2.0,
            force: 10.0,
            cohesiveness: 10.0,
            radius: 1.0,
        };
        let area = bite_chunk_area(&input);
        assert_close(area, 0.090_586_1);
        // Starter plant pellet: h_m 3.2375, F 684.5, sigma 10, r = sqrt(20/PI).
        let input = pellet_bite_input(
            3.237_48,
            684.492,
            MaterialParams::PLANT.cohesiveness,
            (20.0f32 / PI).sqrt(),
        );
        let area = bite_chunk_area(&input);
        close_to(area, 5.062_25, 5e-3);
        // Same bite on meat (sigma 35): smaller chunk.
        let input = pellet_bite_input(
            3.237_48,
            684.492,
            MaterialParams::MEAT.cohesiveness,
            (20.0f32 / PI).sqrt(),
        );
        close_to(bite_chunk_area(&input), 4.392_31, 5e-3);
        // Degenerate inputs are zero, not NaN.
        assert_eq!(bite_chunk_area(&BiteInput::default()), 0.0);
    }

    #[test]
    fn newton_bite_angle_is_finite_and_near_seed() {
        for k in [1.0001, 1.5, 2.0, 6.0, 21.146, 100.0] {
            let theta = newton_bite_angle(k);
            assert!(theta.is_finite(), "k={k} gave {theta}");
        }
    }

    #[test]
    fn bite_damage_rules() {
        let input = BiteDamageInput {
            bite_strength: 684.492,
            armor_strength: 14.62,
            target_max_health: 131.016,
            target_real_body_area: 104.812_8,
            chunk_area: 3.0,
            ongoing: false,
            bite_period: 1.5,
            dt: 1.0 / 30.0,
            damage_factor: 5.0,
        };
        let damage = resolve_bite_damage(&input);
        assert!(!damage.armor_blocked);
        assert_close(damage.damage, 5.0 * 131.016 * 3.0 / 104.812_8);
        let blocked = resolve_bite_damage(&BiteDamageInput {
            bite_strength: 10.0,
            armor_strength: 14.62,
            ..input
        });
        assert!(blocked.armor_blocked);
        assert_eq!(blocked.damage, 0.0);
        let ongoing = resolve_bite_damage(&BiteDamageInput {
            ongoing: true,
            ..input
        });
        assert_close(
            ongoing.damage,
            5.0 * 131.016 * 3.0 / 104.812_8 * (1.0 / 30.0) / (5.0 * 1.5),
        );
    }

    #[test]
    fn swallow_rules() {
        // Fits without a bite: r <= width/4.
        assert_eq!(
            try_swallow_pellet(0.5, 5.0, 10.0, 3.2, 0.0, 1.5),
            PelletSwallow::Swallow
        );
        // Medium pellet: needs a full bite period.
        assert_eq!(
            try_swallow_pellet(1.0, 5.0, 10.0, 3.2, 0.0, 1.5),
            PelletSwallow::NeedsBite
        );
        assert_eq!(
            try_swallow_pellet(1.0, 5.0, 10.0, 3.2, 1.5, 1.5),
            PelletSwallow::Swallow
        );
        assert_eq!(
            try_swallow_pellet(2.0, 5.0, 10.0, 3.2, 0.0, 1.5),
            PelletSwallow::TooBig
        );
        assert_eq!(
            try_swallow_pellet(0.5, 50.0, 10.0, 3.2, 0.0, 1.5),
            PelletSwallow::NoSpace
        );
        // Bibite swallow: bodyLength <= mouthOpening.
        assert_eq!(
            try_swallow_bibite(3.0, 20.0, 5.0, 40.0, 3.2, 1.5, 1.5),
            BibiteSwallow::Swallow
        );
        assert_eq!(
            try_swallow_bibite(4.0, 20.0, 5.0, 40.0, 3.2, 0.0, 1.5),
            BibiteSwallow::TooBig
        );
        assert_eq!(
            try_swallow_bibite(2.0, 20.0, 5.0, 10.0, 3.2, 0.0, 1.5),
            BibiteSwallow::NoSpace
        );
        // Priority: swallow only when can_swallow and Want2Eat beats the
        // (grab-aware) threshold.
        assert_eq!(target_action(true, 0.5, false, 0.0), TargetAction::Swallow);
        assert_eq!(target_action(true, 0.1, false, 0.0), TargetAction::Attack);
        assert_eq!(target_action(false, 1.0, false, 0.0), TargetAction::Attack);
        assert_eq!(target_action(true, 0.6, true, 0.8), TargetAction::Attack);
    }

    #[test]
    fn mouth_state_latches_and_timers() {
        let derived = mouth_derived(100.0, 0.1, 10.0, 1.0, &MouthSettings::default());
        let mut mouth = MouthState::new();
        mouth.begin_tick(0.1, derived.bite_period);
        mouth.latch_outputs(1.0, 1.0, 1.0, &derived);
        assert!(mouth.bite_strength > 0.0);
        assert!(mouth.mouth_opening > 0.0);
        // Force the bite clock to completion.
        for _ in 0..1000 {
            mouth.begin_tick(0.1, derived.bite_period);
        }
        assert!(mouth.bite_ready(derived.bite_period));
        mouth.consume_bite_period(derived.bite_period);
        assert!(!mouth.bite_ready(derived.bite_period));
        mouth.record_first_bite();
        assert_eq!(mouth.bibites_bitten, 1);
        assert!(mouth.attacked_this_frame);
        assert!(!mouth.record_murder(10.0, 100.0));
        mouth.total_murders = 50;
        assert!(mouth.record_murder(5000.0, 100.0));
    }

    #[test]
    fn grab_throw_and_vomit_formulas() {
        assert_close(grab_break_force(600.0, 0.3, 1.0, 2), 600.0 * 0.3 / 2.0);
        assert_close(grab_break_force(600.0, 0.0, 1.0, 1), 600.0 * 0.1);
        assert_eq!(grab_break_force(600.0, 1.0, 1.0, 0), 0.0);
        assert_close(
            throw_force_magnitude(0.25, 600.0, -1.0, 2.0, 0.1),
            0.25 * 600.0 * 1.0 * 2.0 / 2.1,
        );
        assert_close(
            bite_knockback_impulse(0.25, 0.25, 600.0),
            0.25 * 0.25 * 600.0,
        );
        assert_close(vomit_fraction(-0.95), 1.0);
        assert_close(
            vomit_throw_magnitude(0.25, 600.0, 1.0, 2.0, 0.1),
            0.25 * 600.0 * 1.0 * 2.0 / 2.1,
        );
    }

    #[test]
    fn propulsion_constants_starter_bibite() {
        let derived = propulsion_derived(
            104.812_8 * 0.655_080_2 / 13.101_6,
            1.0,
            &PropulsionSettings::default(),
        );
        assert_close(derived.muscle_move_strength, 173.184);
        assert_close(derived.muscle_turn_strength, 149.964);
        assert_close(derived.base_acceleration_cost, 0.104_813);
        assert_close(derived.base_turn_cost, 0.052_406);
    }

    #[test]
    fn movement_command_formula() {
        let derived = PropulsionDerived {
            muscle_move_strength: 100.0,
            muscle_turn_strength: 50.0,
            base_acceleration_cost: 0.1,
            base_turn_cost: 0.05,
        };
        let input = MovementInput {
            accelerate: 2.0, // clamped to 1
            rotate: 0.5,
            health_fullness: 1.0,
            age_strength_penalty: 1.0,
            ..MovementInput::default()
        };
        let command = compute_movement(&derived, &input, Vec2::Y, 0.5);
        assert_close(command.force.y, 100.0);
        assert_close(command.torque, 25.0);
        assert_close(command.movement_cost, 0.1 + 0.05 * 0.5);
        // Backward is halved and health/age penalty applies.
        let input = MovementInput {
            accelerate: -1.0,
            health_fullness: 0.25,
            age_strength_penalty: 0.5,
            ..MovementInput::default()
        };
        let command = compute_movement(&derived, &input, Vec2::Y, 0.5);
        assert_close(command.force.y, -100.0 * 0.5 * 0.5 * 0.5);
        assert_close(command.movement_cost, 0.1 * 0.25 * 0.5);
    }

    #[test]
    fn herding_and_avoidance_helpers() {
        assert_close(herding_steering(1.0, 0.0, 90.0), 1.0);
        assert_close(herding_steering(0.0, 1.0, 90.0), tanh(1.0));
        assert_close(herding_steering(1.0, 1.0, 0.0), 0.0);
        assert_close(blend_turn_towards(1.0, 0.0, 0.5), 0.5);
        assert_close(blend_turn_towards(1.0, 0.0, 0.0), 1.0);
        assert_close(out_of_bound_dist_sq(2000.0), 2000.0 * 2000.0 * 2.25);
        assert_close(void_avoidance_weight(50.0, 100.0), 0.5);
        assert_eq!(shade_avoidance_steering(1.0, 0.0, 100.0, -90.0), 1.0);
        // Fully outside: blend entirely toward tanh(angle to origin).
        assert_close(shade_avoidance_steering(1.0, 225.0, 100.0, -90.0), -1.0);
    }

    #[test]
    fn world_wrap_and_kinematics() {
        assert_eq!(world_wrap_position(Vec2::new(10.0, 0.0), 5.0, 2000.0), None);
        let wrapped = world_wrap_position(Vec2::new(-5000.0, 0.0), 10.0, 2000.0).unwrap();
        assert_close(wrapped.x, 4000.0 - 9.5);
        assert_close(wrapped.y, 0.0);
        let mut state = PropulsionState::new(Vec2::ZERO);
        state.update_kinematics(
            Vec2::new(3.0, 4.0),
            Vec2::new(0.0, 2.0),
            180.0,
            Vec2::Y,
            4.0,
        );
        assert_close(state.total_travel, 5.0);
        assert_close(state.signed_velocity, 2.0);
        assert_close(state.normalized_velocity, 0.5);
        assert_close(state.angular_velocity, 0.5);
        assert_close(state.angle_degrees, 90.0);
        // -y velocity maps to 270 degrees like Unity's Angle + sign flip.
        state.update_kinematics(Vec2::ZERO, Vec2::new(0.0, -2.0), 0.0, Vec2::X, 1.0);
        assert_close(state.angle_degrees, 270.0);
        assert_close(state.signed_velocity, 0.0);
        assert!(movement_penalty(4.0, 1.0) >= 2.0);
    }

    #[test]
    fn material_params_are_distinct_and_finite() {
        for params in [
            MaterialParams::PLANT,
            MaterialParams::MEAT,
            MaterialParams::FAT,
            MaterialParams::ARMOR,
        ] {
            assert!(params.cohesiveness > 0.0);
            assert!(params.reactivity > 0.0);
            assert!(params.energy_density > 0.0);
            assert!(params.mass_density > 0.0);
            assert!(params.min_efficiency.is_finite());
            assert!(params.max_efficiency.is_finite());
            assert_eq!(clamp01(0.5), 0.5);
        }
    }
}
