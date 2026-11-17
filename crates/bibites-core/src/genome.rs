//! Bibite genome: the 35 genes, gene-derived body formulas and mutation.
//!
//! This is a faithful port of `Here/SimulationScripts.BibiteScripts/BibiteGenes.cs`
//! and `Here/SettingScripts/BibiteEditorSettings.cs`; every constant carries a
//! source reference (see `docs/spec/01-brain-genetics.md` §7/§8).
//!
//! * [`crate::genome::Gene`] enumerates the 35 genes in the game's exact array order.
//! * [`crate::genome::Genome`] is a plain `[f32; 35]` plus the derived formulas body/world need.
//! * [`crate::genome::GenomeMeta`] holds the non-gene, serialized per-bibite genetic state
//!   (generation, inherited species, easter-egg flags, parents).
//! * [`crate::genome::Genome::child`] implements the gene mutation algorithm exactly (Poisson
//!   count, relative + absolute Gaussian blend, per-gene clamping) and is fully
//!   deterministic given an [`Rng`].
//!
//! Settings access goes through the [`crate::genome::GenomeSettings`] trait. The game's
//! `SimSettings` will implement it when `settings.rs` lands (fmt-dev); until
//! then [`crate::genome::DefaultGenomeSettings`] carries the captured game defaults so this
//! module compiles and is testable on its own.

use crate::ids::{BibiteId, SpeciesId};
use crate::math::clamp;
use crate::rng::Rng;

/// Number of genes in a bibite genome (`BibiteGenes.NGene`, `BibiteGenes.cs:117`).
pub const N_GENES: usize = 35;

/// In-game display name of every gene in enum order
/// (`BibiteEditorSettings.geneSettings[*].Name`).
pub const GENE_DISPLAY_NAMES: [&str; 35] = [
    "Lay Time",
    "Brood Time",
    "Hatch Time",
    "Size Ratio",
    "Metabolism Speed",
    "Red Color",
    "Green Color",
    "Blue Color",
    "Gene Mutation Variance",
    "Average Gene Mutations",
    "Brain Mutation Variance",
    "Average Brain Mutations",
    "View Angle",
    "View Radius",
    "Clock Period",
    "Pheromones Sensing Radius",
    "Diet",
    "Herd Separation Weight",
    "Herd Alignment Weight",
    "Herd Cohesion Weight",
    "Herd Velocity Weight",
    "Herd Separation Distance",
    "Growth Scale Factor",
    "Growth Maturity Factor",
    "Growth Maturity Exponent",
    "Eye Hue Offset",
    "Stomach WAG",
    "Egg Organ WAG",
    "Fat Organ WAG",
    "Armor WAG",
    "Throat WAG",
    "Jaw Muscles WAG",
    "Arm Muscles WAG",
    "Fat Storage Threshold",
    "Fat Storage Deadband",
];

/// A bibite gene, in the exact order of the game's `BibiteGenes.Genes` enum
/// (`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:16-53`).
///
/// The discriminant is the index into [`Genome::genes`].
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Gene {
    /// Seconds to produce an egg at full egg-organ activation.
    LayTime = 0,
    /// Brood time (s); with `HatchTime` sets the birth growth ratio.
    BroodTime = 1,
    /// Egg incubation time (s).
    HatchTime = 2,
    /// 1-D body size multiplier.
    SizeRatio = 3,
    /// Metabolism speed (everything scales with it).
    SpeedRatio = 4,
    /// Red skin pigment.
    ColorR = 5,
    /// Green skin pigment.
    ColorG = 6,
    /// Blue skin pigment.
    ColorB = 7,
    /// Standard deviation of gene mutation deltas (fraction of span).
    MutationAmountSigma = 8,
    /// Mean number of gene mutations per offspring.
    AverageMutationNumber = 9,
    /// Standard deviation of brain mutation deltas.
    BrainMutationSigma = 10,
    /// Mean number of brain mutations per offspring.
    BrainAverageMutation = 11,
    /// Field-of-view angle (degrees).
    ViewAngle = 12,
    /// Field-of-view radius (world units, body-size scaled at runtime).
    ViewRadius = 13,
    /// Internal clock toggling period (s).
    ClockSpeed = 14,
    /// Pheromone sensing radius (world units).
    PheroSense = 15,
    /// Diet: 0 = herbivore, 1 = carnivore.
    Diet = 16,
    /// Herd separation weight.
    HerdSeparationWeight = 17,
    /// Herd alignment weight.
    HerdAlignmentWeight = 18,
    /// Herd cohesion weight.
    HerdCohesionWeight = 19,
    /// Herd velocity weight.
    HerdVelocityWeight = 20,
    /// Herd separation distance (world units).
    HerdSeparationDistance = 21,
    /// Growth curve scale factor.
    GrowthScale = 22,
    /// Growth curve maturity factor.
    GrowthMaturityFactor = 23,
    /// Growth curve maturity exponent.
    GrowthMaturityExponent = 24,
    /// Eye hue offset.
    EyeOffset = 25,
    /// Stomach weighted apportionment gene.
    StomachWAG = 26,
    /// Egg organ (womb) weighted apportionment gene.
    WombWAG = 27,
    /// Fat organ weighted apportionment gene.
    FatWAG = 28,
    /// Armor weighted apportionment gene.
    ArmorWAG = 29,
    /// Throat weighted apportionment gene.
    ThroatWAG = 30,
    /// Jaw-muscles weighted apportionment gene.
    MouthMusclesWAG = 31,
    /// Arm-muscles weighted apportionment gene.
    MoveMusclesWAG = 32,
    /// Fat storage energy-ratio threshold.
    FatStorageThreshold = 33,
    /// Fat storage hysteresis deadband.
    FatStorageDeadband = 34,
}

/// All genes in array order, for table-driven code and tests.
pub const ALL_GENES: [Gene; N_GENES] = {
    use Gene::*;
    [
        LayTime,
        BroodTime,
        HatchTime,
        SizeRatio,
        SpeedRatio,
        ColorR,
        ColorG,
        ColorB,
        MutationAmountSigma,
        AverageMutationNumber,
        BrainMutationSigma,
        BrainAverageMutation,
        ViewAngle,
        ViewRadius,
        ClockSpeed,
        PheroSense,
        Diet,
        HerdSeparationWeight,
        HerdAlignmentWeight,
        HerdCohesionWeight,
        HerdVelocityWeight,
        HerdSeparationDistance,
        GrowthScale,
        GrowthMaturityFactor,
        GrowthMaturityExponent,
        EyeOffset,
        StomachWAG,
        WombWAG,
        FatWAG,
        ArmorWAG,
        ThroatWAG,
        MouthMusclesWAG,
        MoveMusclesWAG,
        FatStorageThreshold,
        FatStorageDeadband,
    ]
};

/// Static definition of one gene: editor range, default, display metadata.
///
/// Values are ported from `Here/SettingScripts/BibiteEditorSettings.cs`
/// (the `geneSettings` list) and `docs/spec/01-brain-genetics.md` §7.1.
#[derive(Clone, Copy, Debug)]
pub struct GeneDef {
    /// JSON / enum name (`"LayTime"`, …).
    pub name: &'static str,
    /// In-game display name (`"Lay Time"`, …).
    pub display_name: &'static str,
    /// Minimum allowed value (`Mathf.Clamp` lower bound).
    pub min: f32,
    /// Maximum allowed value (`Mathf.Clamp` upper bound).
    pub max: f32,
    /// Editor default (used when a file lacks the gene).
    pub default: f32,
    /// Display unit string (may be empty).
    pub units: &'static str,
    /// When true the mutation kernel is 100 % relative (`isPurelyRelative`).
    pub purely_relative: bool,
}

impl GeneDef {
    /// Mutation span used as `geneSpan` in the mutation kernel: `max - min`
    /// (`Here/SettingScripts/GeneSetting.cs:13`).
    #[inline]
    pub fn span(&self) -> f32 {
        self.max - self.min
    }
}

/// The 35 gene definitions, indexed by [`crate::genome::Gene`] discriminant.
///
/// Source: `Here/SettingScripts/BibiteEditorSettings.cs:11-572`; see
/// `docs/spec/01-brain-genetics.md` §7.1 for the rendered table.
pub const GENE_DEFS: [GeneDef; N_GENES] = [
    GeneDef {
        name: "LayTime",
        display_name: "Lay Time",
        min: 1.0,
        max: 51.0,
        default: 15.0,
        units: "s",
        purely_relative: false,
    },
    GeneDef {
        name: "BroodTime",
        display_name: "Brood Time",
        min: 1.0,
        max: 101.0,
        default: 20.0,
        units: "s",
        purely_relative: false,
    },
    GeneDef {
        name: "HatchTime",
        display_name: "Hatch Time",
        min: 1.0,
        max: 101.0,
        default: 10.0,
        units: "s",
        purely_relative: false,
    },
    GeneDef {
        name: "SizeRatio",
        display_name: "Size Ratio",
        min: 0.05,
        max: 2.55,
        default: 1.0,
        units: "",
        purely_relative: true,
    },
    GeneDef {
        name: "SpeedRatio",
        display_name: "Metabolism Speed",
        min: 1e-6,
        max: 2.5,
        default: 0.75,
        units: "SPD",
        purely_relative: true,
    },
    GeneDef {
        name: "ColorR",
        display_name: "Red Color",
        min: 0.0,
        max: 1.0,
        default: 0.1,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "ColorG",
        display_name: "Green Color",
        min: 0.0,
        max: 1.0,
        default: 0.1,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "ColorB",
        display_name: "Blue Color",
        min: 0.0,
        max: 1.0,
        default: 0.1,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "MutationAmountSigma",
        display_name: "Gene Mutation Variance",
        min: 0.0,
        max: 0.25,
        default: 0.05,
        units: "%",
        purely_relative: false,
    },
    GeneDef {
        name: "AverageMutationNumber",
        display_name: "Average Gene Mutations",
        min: 0.0,
        max: 30.0,
        default: 2.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "BrainMutationSigma",
        display_name: "Brain Mutation Variance",
        min: 0.0,
        max: 0.25,
        default: 0.1,
        units: "%",
        purely_relative: false,
    },
    GeneDef {
        name: "BrainAverageMutation",
        display_name: "Average Brain Mutations",
        min: 0.0,
        max: 10.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "ViewAngle",
        display_name: "View Angle",
        min: 0.0,
        max: 360.0,
        default: 75.0,
        units: "°",
        purely_relative: false,
    },
    GeneDef {
        name: "ViewRadius",
        display_name: "View Radius",
        min: 0.0,
        max: 200.0,
        default: 180.0,
        units: "u",
        purely_relative: false,
    },
    GeneDef {
        name: "ClockSpeed",
        display_name: "Clock Period",
        min: 0.1,
        max: 10.1,
        default: 1.0,
        units: "s",
        purely_relative: false,
    },
    GeneDef {
        name: "PheroSense",
        display_name: "Pheromones Sensing Radius",
        min: 0.0,
        max: 500.0,
        default: 150.0,
        units: "u",
        purely_relative: false,
    },
    GeneDef {
        name: "Diet",
        display_name: "Diet",
        min: 0.0,
        max: 1.0,
        default: 0.2,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "HerdSeparationWeight",
        display_name: "Herd Separation Weight",
        min: 0.0,
        max: 4.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "HerdAlignmentWeight",
        display_name: "Herd Alignment Weight",
        min: 0.0,
        max: 4.0,
        default: 1.5,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "HerdCohesionWeight",
        display_name: "Herd Cohesion Weight",
        min: 0.0,
        max: 4.0,
        default: 0.75,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "HerdVelocityWeight",
        display_name: "Herd Velocity Weight",
        min: 0.0,
        max: 1.0,
        default: 0.25,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "HerdSeparationDistance",
        display_name: "Herd Separation Distance",
        min: 0.0,
        max: 100.0,
        default: 35.0,
        units: "u",
        purely_relative: false,
    },
    GeneDef {
        name: "GrowthScale",
        display_name: "Growth Scale Factor",
        min: 0.0,
        max: 2.0,
        default: 0.15,
        units: "",
        purely_relative: true,
    },
    GeneDef {
        name: "GrowthMaturityFactor",
        display_name: "Growth Maturity Factor",
        min: 0.0,
        max: 10.0,
        default: 2.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "GrowthMaturityExponent",
        display_name: "Growth Maturity Exponent",
        min: 0.0,
        max: 10.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "EyeOffset",
        display_name: "Eye Hue Offset",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "StomachWAG",
        display_name: "Stomach WAG",
        min: -1.0,
        max: 9.0,
        default: 2.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "WombWAG",
        display_name: "Egg Organ WAG",
        min: -1.0,
        max: 9.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "FatWAG",
        display_name: "Fat Organ WAG",
        min: -1.0,
        max: 9.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "ArmorWAG",
        display_name: "Armor WAG",
        min: -1.0,
        max: 9.0,
        default: 0.15,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "ThroatWAG",
        display_name: "Throat WAG",
        min: -1.0,
        max: 9.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "MouthMusclesWAG",
        display_name: "Jaw Muscles WAG",
        min: -1.0,
        max: 9.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "MoveMusclesWAG",
        display_name: "Arm Muscles WAG",
        min: -1.0,
        max: 9.0,
        default: 1.0,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "FatStorageThreshold",
        display_name: "Fat Storage Threshold",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        units: "",
        purely_relative: false,
    },
    GeneDef {
        name: "FatStorageDeadband",
        display_name: "Fat Storage Deadband",
        min: 0.0,
        max: 0.5,
        default: 0.25,
        units: "",
        purely_relative: false,
    },
];

impl Gene {
    /// Returns this gene's index into [`Genome::genes`].
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Returns this gene's static definition.
    #[inline]
    pub fn def(self) -> &'static GeneDef {
        &GENE_DEFS[self.index()]
    }

    /// Mutation span (`max - min`), used as `geneSpan` by the mutation kernel.
    #[inline]
    pub fn span(self) -> f32 {
        self.def().span()
    }

    /// In-game display name (`"Lay Time"`).
    #[inline]
    pub fn display_name(self) -> &'static str {
        self.def().display_name
    }

    /// Display unit string (may be empty).
    #[inline]
    pub fn units(self) -> &'static str {
        self.def().units
    }

    /// Editor `(min, max)` range.
    #[inline]
    pub fn range(self) -> (f32, f32) {
        let def = self.def();
        (def.min, def.max)
    }

    /// Editor default value.
    #[inline]
    pub fn default_value(self) -> f32 {
        self.def().default
    }

    /// Maps a gene index (e.g. from a save file) to a [`crate::genome::Gene`], or `None`.
    #[inline]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_GENES.get(index).copied()
    }

    /// Maps the game's JSON/enum name (e.g. `"LayTime"`) to a [`crate::genome::Gene`].
    pub fn from_name(name: &str) -> Option<Self> {
        GENE_DEFS
            .iter()
            .position(|def| def.name == name)
            .and_then(Self::from_index)
    }
}

/// Per-bibite genetic state that is *not* part of the 35-gene array but is
/// serialized by the game and needed by mutation/speciation.
///
/// Mirrors the non-array fields of `Here/SimulationScripts.BibiteScripts/
/// BibiteGenes.cs` (`generation`, `species`, `speciesTag`, `isMutant`,
/// `isRad/isRain/isKiller/isSource`, `deuxParent`, `parent1/parent2`).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct GenomeMeta {
    /// Generation number (`BibiteGenes.generation`).
    pub generation: u32,
    /// Species inherited at birth (`BibiteGenes.species`); reassigned by
    /// speciation.
    pub species: Option<SpeciesId>,
    /// Free-form lineage tag (`BibiteGenes.speciesTag`).
    pub species_tag: Option<String>,
    /// Persistent two-step speciation outlier flag (`BibiteGenes.isMutant`).
    pub is_mutant: bool,
    /// Easter-egg flag: radiation carrier.
    pub is_rad: bool,
    /// Easter-egg flag: rainbow body.
    pub is_rain: bool,
    /// Easter-egg flag: reaper (insta-kill).
    pub is_killer: bool,
    /// Easter-egg flag: pellet producer.
    pub is_source: bool,
    /// Dead code in 0.6.3.1 (`BibiteGenes.deuxParent`); round-trip only.
    pub deux_parent: bool,
    /// First parent id (serialized as `parent1`).
    pub parent1: Option<BibiteId>,
    /// Second parent id (serialized as `parent2`).
    pub parent2: Option<BibiteId>,
}

/// Aggregated extra mutation pressure from the environment (radiation dose,
/// scenario events). Mirrors `MutationIncreases`
/// (`Here/SimulationScripts.BibiteScripts/MutationIncreases.cs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MutationFactor {
    /// Added to the Poisson mean of gene *and* brain mutation counts.
    pub probability_increase: f32,
    /// Added to the mutation standard deviation of genes and brain.
    pub intensity_increase: f32,
}

/// Settings needed by genome mutation and gene-derived energy formulas.
///
/// This is the seam between `genome.rs` and fmt-dev's `SimSettings`
/// (`docs/INTERFACES.md` §3): once `settings.rs` lands,
/// `impl GenomeSettings for SimSettings` is added here and callers pass
/// `&SimSettings` exactly as the interface document specifies. Until then,
/// [`crate::genome::DefaultGenomeSettings`] provides the captured game defaults.
pub trait GenomeSettings {
    /// Scenario setting `backgroundMutationChance` (default 1.5).
    fn background_mutation_chance(&self) -> f32;
    /// Scenario setting `backgroundMutationVariance` (default 0.05).
    fn background_mutation_variance(&self) -> f32;
    /// Scenario setting `relativeMutationShare` (default 0.75).
    fn relative_mutation_share(&self) -> f32;
    /// Scenario setting `preventMutations` (default false).
    fn prevent_mutations(&self) -> bool;
    /// User setting `AllowEasterEggs` (default false; not part of SimSettings).
    fn allow_easter_eggs(&self) -> bool {
        false
    }
    /// User setting `EasterEggsProbability` period (default 500_000; frequency `1/x`).
    fn easter_eggs_period(&self) -> u32 {
        500_000
    }
    /// Scenario setting `energyUsageEfficiency` (default 0.8).
    fn energy_usage_efficiency(&self) -> f32;
    /// Scenario setting `baseBodyGrowthCost` (default 2.0 E/u²).
    fn base_body_growth_cost(&self) -> f32;
    /// Scenario setting `viewAngleAddedCost` (default 0.003 E/u²).
    fn view_angle_added_cost(&self) -> f32;
    /// Scenario setting `viewRadiusAddedCost` (default 0.002 E/u²).
    fn view_radius_added_cost(&self) -> f32;
    /// Armor material `energyDensity` (default 1.5 E/u²).
    fn armor_energy_density(&self) -> f32;
    /// Scenario setting `storableEnergyPerArea` (default 1.5 E/u²).
    fn storable_energy_per_area(&self) -> f32;
    /// Scenario setting `healthEnergyFactor` (default 0.5).
    fn health_energy_factor(&self) -> f32;
}

/// Captured game defaults for the settings [`crate::genome::GenomeSettings`] exposes.
///
/// Temporary shim so genome code compiles before fmt-dev lands
/// `settings.rs` (task-9). Do not grow this into a second settings model:
/// add fields to `settings.rs` and map them in its `GenomeSettings` impl.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DefaultGenomeSettings {
    /// `backgroundMutationChance`.
    pub background_mutation_chance: f32,
    /// `backgroundMutationVariance`.
    pub background_mutation_variance: f32,
    /// `relativeMutationShare`.
    pub relative_mutation_share: f32,
    /// `preventMutations`.
    pub prevent_mutations: bool,
    /// `energyUsageEfficiency`.
    pub energy_usage_efficiency: f32,
    /// `baseBodyGrowthCost`.
    pub base_body_growth_cost: f32,
    /// `viewAngleAddedCost`.
    pub view_angle_added_cost: f32,
    /// `viewRadiusAddedCost`.
    pub view_radius_added_cost: f32,
    /// Armor energy density.
    pub armor_energy_density: f32,
    /// `storableEnergyPerArea`.
    pub storable_energy_per_area: f32,
    /// `healthEnergyFactor`.
    pub health_energy_factor: f32,
}

impl DefaultGenomeSettings {
    /// The captured 0.6.3.1 game defaults (`ScenarioSettings` / materials).
    pub fn game_defaults() -> Self {
        Self {
            background_mutation_chance: 1.5,
            background_mutation_variance: 0.05,
            relative_mutation_share: 0.75,
            prevent_mutations: false,
            energy_usage_efficiency: 0.8,
            base_body_growth_cost: 2.0,
            view_angle_added_cost: 0.003,
            view_radius_added_cost: 0.002,
            armor_energy_density: 1.5,
            storable_energy_per_area: 1.5,
            health_energy_factor: 0.5,
        }
    }
}

impl GenomeSettings for DefaultGenomeSettings {
    fn background_mutation_chance(&self) -> f32 {
        self.background_mutation_chance
    }
    fn background_mutation_variance(&self) -> f32 {
        self.background_mutation_variance
    }
    fn relative_mutation_share(&self) -> f32 {
        self.relative_mutation_share
    }
    fn prevent_mutations(&self) -> bool {
        self.prevent_mutations
    }
    fn energy_usage_efficiency(&self) -> f32 {
        self.energy_usage_efficiency
    }
    fn base_body_growth_cost(&self) -> f32 {
        self.base_body_growth_cost
    }
    fn view_angle_added_cost(&self) -> f32 {
        self.view_angle_added_cost
    }
    fn view_radius_added_cost(&self) -> f32 {
        self.view_radius_added_cost
    }
    fn armor_energy_density(&self) -> f32 {
        self.armor_energy_density
    }
    fn storable_energy_per_area(&self) -> f32 {
        self.storable_energy_per_area
    }
    fn health_energy_factor(&self) -> f32 {
        self.health_energy_factor
    }
}

/// Production seam: the aggregate simulation settings implement the genome's
/// settings view (`docs/INTERFACES.md` §3/§4). Field paths are maintained by
/// fmt-dev's `settings.rs`.
impl GenomeSettings for crate::settings::SimSettings {
    fn background_mutation_chance(&self) -> f32 {
        self.scenario.background_mutation_chance
    }
    fn background_mutation_variance(&self) -> f32 {
        self.scenario.background_mutation_variance
    }
    fn relative_mutation_share(&self) -> f32 {
        self.scenario.relative_mutation_share
    }
    fn prevent_mutations(&self) -> bool {
        self.scenario.prevent_mutations
    }
    fn allow_easter_eggs(&self) -> bool {
        self.allow_easter_eggs
    }
    fn easter_eggs_period(&self) -> u32 {
        self.easter_eggs_period
    }
    fn energy_usage_efficiency(&self) -> f32 {
        self.scenario.energy_usage_efficiency
    }
    fn base_body_growth_cost(&self) -> f32 {
        self.scenario.base_body_growth_cost
    }
    fn view_angle_added_cost(&self) -> f32 {
        self.scenario.view_angle_added_cost
    }
    fn view_radius_added_cost(&self) -> f32 {
        self.scenario.view_radius_added_cost
    }
    fn armor_energy_density(&self) -> f32 {
        self.materials.armor.energy_density
    }
    fn storable_energy_per_area(&self) -> f32 {
        self.scenario.storable_energy_per_area
    }
    fn health_energy_factor(&self) -> f32 {
        self.scenario.health_energy_factor
    }
}

/// `baseBodyArea = 80 * d2Size` (`BibiteGenes.cs:87,133`; `BibiteBody.cs:298`).
pub const AREA_PER_D2: f32 = 80.0;
/// `bodyLength = 12.5 * d1Size` (`BibiteGenes.cs:89`; `BibiteBody.cs:296`).
pub const LENGTH_PER_D1: f32 = 12.5;
/// Energy-per-area factor used by `LayBodyEnergy`/`BodyEnergyAtBirth`
/// (`BibiteGenes.cs:125-127`, the literal `100f`).
pub const ENERGY_PER_D2: f32 = 100.0;

/// `BibiteGenes.GenesToColor` (`BibiteGenes.cs:488-499`): the body colour the game's
/// materials, the ColorKiller and the species / lineage colours get from the three
/// pigment genes (`ColorR`, `ColorG`, `ColorB`).
///
/// It is NOT the raw genes: each channel is `clamp(1 + v - max(other two), 0, 1) - min(r, g, b)`.
/// The hue is the genes' hue and `max - min` is kept, but the brightest channel becomes
/// `1 - min` and the darkest `1 - max`, so equal pigments (a "basic" bibite is `0.1, 0.1, 0.1`)
/// give a light grey `0.9, 0.9, 0.9`. Vision (`FieldOfView`) still reads the raw genes.
#[must_use]
pub fn genes_to_color(red: f32, green: f32, blue: f32) -> [f32; 3] {
    let minimum = red.min(green).min(blue);
    let channel = |value: f32, other: f32, third: f32| {
        // `(g > b) ? g : b` and the clamp of the C#, in the same order of operations.
        let strongest = if other > third { other } else { third };
        (1.0 + value - strongest).clamp(0.0, 1.0) - minimum
    };
    [
        channel(red, green, blue),
        channel(green, red, blue),
        channel(blue, red, green),
    ]
}

/// A bibite genome: the 35 gene values.
///
/// The game stores the same data in `BibiteGenes.genes` (`float[35]`).
#[derive(Clone, PartialEq, Debug)]
pub struct Genome {
    /// Gene values indexed by [`Gene::index`].
    pub genes: [f32; N_GENES],
}

impl Genome {
    /// Builds a genome from raw values, clamping each gene to its editor range
    /// (the game's `CapGenes`, `BibiteGenes.cs:409-416`).
    pub fn from_values(values: [f32; N_GENES]) -> Self {
        let mut genome = Self { genes: values };
        genome.cap();
        genome
    }

    /// The game's editor defaults for all 35 genes
    /// (`BibiteEditorSettings.SettingOfGene(i).DefaultValue`).
    pub fn defaults() -> Self {
        let mut genes = [0.0_f32; N_GENES];
        for (slot, def) in genes.iter_mut().zip(GENE_DEFS.iter()) {
            *slot = def.default;
        }
        Self { genes }
    }

    /// The official "Basic bibite" starter genome, captured from
    /// `testdata/real/templates_current/Basic_bibite_0.6.3.1.bb8template`.
    ///
    /// It happens to equal [`Genome::defaults`] for every gene.
    pub fn starter() -> Self {
        Self::defaults()
    }

    /// Randomizes every gene in the editor range, mirroring the game's
    /// `BibiteGenes.RandomGenes` (`BibiteGenes.cs:427-435`):
    /// all genes are drawn uniformly in their editor range, then
    /// `BroodTime = max(BroodTime, HatchTime / 0.75)` and
    /// `SizeRatio = 1.5^Normal(0,1)`.
    ///
    /// The game uses `UnityEngine.Random`; this API takes an explicit seeded
    /// [`Rng`] so library/editor operations are reproducible.
    #[must_use]
    pub fn randomized(rng: &mut Rng) -> Self {
        let mut genome = Self::defaults();
        genome.randomize_in_place(rng);
        genome
    }

    /// In-place [`Genome::randomized`].
    pub fn randomize_in_place(&mut self, rng: &mut Rng) {
        for (index, def) in GENE_DEFS.iter().enumerate() {
            self.genes[index] = rng.f32_range(def.min, def.max);
        }
        self.genes[Gene::BroodTime.index()] = self
            .gene(Gene::BroodTime)
            .max(self.gene(Gene::HatchTime) / 0.75);
        self.genes[Gene::SizeRatio.index()] = 1.5_f32.powf(rng.normal());
    }

    /// Randomizes the three skin-colour genes in their editor ranges
    /// (`BibiteGenes.RandomColor`, `BibiteGenes.cs:427-432`).
    pub fn randomize_color(&mut self, rng: &mut Rng) {
        for gene in [Gene::ColorR, Gene::ColorG, Gene::ColorB] {
            let def = gene.def();
            self.genes[gene.index()] = rng.f32_range(def.min, def.max);
        }
    }

    /// Applies one selected-gene draw from the same kernel as
    /// [`Genome::child`] (`BibiteGenes.cs:388-407`) and returns the new value.
    ///
    /// This is the editor/library "mutate this gene" primitive: it advances the
    /// supplied RNG once and never mutates other genes or metadata. It honours
    /// [`GenomeSettings::prevent_mutations`] and uses the current
    /// `MutationAmountSigma` plus background mutation settings. The game's
    /// child mutation draws the gene index itself; this API exposes the
    /// selected-gene form for UI use.
    pub fn mutate_gene<S: GenomeSettings + ?Sized>(
        &mut self,
        gene: Gene,
        rng: &mut Rng,
        s: &S,
        factor: &MutationFactor,
    ) -> f32 {
        let index = gene.index();
        if s.prevent_mutations() {
            return self.genes[index];
        }
        let def = gene.def();
        let sigma = self.gene(Gene::MutationAmountSigma)
            + s.background_mutation_variance()
            + factor.intensity_increase;
        let delta = next_mutation_value(
            rng,
            self.genes[index],
            sigma,
            def.span(),
            def.purely_relative,
            s.relative_mutation_share(),
        );
        self.genes[index] = clamp(self.genes[index] + delta, def.min, def.max);
        mutation_counters::add_gene_mutation(1);
        self.genes[index]
    }

    /// Returns one gene's value.
    #[inline]
    pub fn gene(&self, gene: Gene) -> f32 {
        self.genes[gene.index()]
    }

    /// Sets one gene, clamping to its editor range (game `CapGenes`).
    #[inline]
    pub fn set_gene(&mut self, gene: Gene, value: f32) {
        let def = gene.def();
        self.genes[gene.index()] = clamp(value, def.min, def.max);
    }

    /// Clamps every gene to its editor range, exactly like `CapGenes`.
    pub fn cap(&mut self) {
        for (value, def) in self.genes.iter_mut().zip(GENE_DEFS.iter()) {
            *value = clamp(*value, def.min, def.max);
        }
    }

    /// Raw gene slice for io (`bibites-io` writes the 35 named values).
    #[inline]
    pub fn values(&self) -> &[f32; N_GENES] {
        &self.genes
    }

    /// `growthAtBirth = (HatchTime / BroodTime)²` (`BibiteGenes.cs:163-166`).
    #[inline]
    pub fn growth_at_birth(&self) -> f32 {
        let ratio = self.gene(Gene::HatchTime) / self.gene(Gene::BroodTime);
        ratio * ratio
    }

    /// `Σ max(0, WAG)` over genes 26..=32 (`BibiteGenes.cs:223-226`).
    pub fn total_organ_wag(&self) -> f32 {
        let mut total = 0.0;
        for gene in [
            Gene::StomachWAG,
            Gene::WombWAG,
            Gene::FatWAG,
            Gene::ArmorWAG,
            Gene::ThroatWAG,
            Gene::MouthMusclesWAG,
            Gene::MoveMusclesWAG,
        ] {
            total += self.gene(gene).max(0.0);
        }
        total
    }

    /// Organ share of the internal area: `max(0, WAG) / max(totalWAG, 0.0001)`
    /// (`BibiteGenes.cs:218-221`; organ growth in `BibiteSpatialOrgan.cs:25,35`).
    ///
    /// Returns `0.0` for non-WAG genes.
    pub fn organ_area_portion(&self, organ: Gene) -> f32 {
        let wag = self.gene(organ).max(0.0);
        wag / self.total_organ_wag().max(0.0001)
    }

    /// `wombAreaPortion` convenience (`BibiteGenes.cs:183-186`).
    #[inline]
    pub fn womb_area_portion(&self) -> f32 {
        self.organ_area_portion(Gene::WombWAG)
    }

    /// `growthAtMature = max(1, growthAtBirth / max(wombAreaPortion, 0.001))`
    /// (`BibiteGenes.cs:168-171`).
    pub fn growth_at_mature(&self) -> f32 {
        let growth = self.growth_at_birth() / self.womb_area_portion().max(0.001);
        growth.max(1.0)
    }

    /// `D2SizeAtBirth = SizeRatio² * growthAtBirth` (`BibiteGenes.cs:131`).
    #[inline]
    pub fn d2_size_at_birth(&self) -> f32 {
        let size = self.gene(Gene::SizeRatio);
        size * size * self.growth_at_birth()
    }

    /// `D1SizeAtBirth = SizeRatio * sqrt(growthAtBirth)` (`BibiteGenes.cs:129,143`).
    #[inline]
    pub fn d1_size_at_birth(&self) -> f32 {
        self.gene(Gene::SizeRatio) * self.growth_at_birth().sqrt()
    }

    /// `areaAtBirth = 80 * D2SizeAtBirth` (`BibiteGenes.cs:133`).
    #[inline]
    pub fn area_at_birth(&self) -> f32 {
        AREA_PER_D2 * self.d2_size_at_birth()
    }

    /// `AreaAtMature = 80 * growthAtMature * SizeRatio²` (`BibiteGenes.cs:173-176`).
    #[inline]
    pub fn area_at_mature(&self) -> f32 {
        let size = self.gene(Gene::SizeRatio);
        AREA_PER_D2 * self.growth_at_mature() * size * size
    }

    /// `LengthAtMature = 12.5 * sqrt(growthAtMature) * SizeRatio`
    /// (`BibiteGenes.cs:178-181`).
    #[inline]
    pub fn length_at_mature(&self) -> f32 {
        LENGTH_PER_D1 * self.growth_at_mature().sqrt() * self.gene(Gene::SizeRatio)
    }

    /// `metabolicRate = SpeedRatio` (`BibiteGenes.cs:135`).
    #[inline]
    pub fn metabolic_rate(&self) -> f32 {
        self.gene(Gene::SpeedRatio)
    }

    /// `MetabolicEfficiency = energyUsageEfficiency ^ SpeedRatio`
    /// (`BibiteGenes.cs:137`).
    #[inline]
    pub fn metabolic_efficiency<S: GenomeSettings + ?Sized>(&self, s: &S) -> f32 {
        s.energy_usage_efficiency().powf(self.metabolic_rate())
    }

    /// `BodyEnergyDensity` (`BibiteGenes.cs:121`).
    pub fn body_energy_density<S: GenomeSettings + ?Sized>(&self, s: &S) -> f32 {
        s.base_body_growth_cost()
            + self.gene(Gene::ViewAngle) * s.view_angle_added_cost()
            + self.gene(Gene::ViewRadius) * s.view_radius_added_cost()
            + self.organ_area_portion(Gene::ArmorWAG) * s.armor_energy_density()
    }

    /// `SumBodyAndHealthEnergyRatio = BodyEnergyDensity * (1 + healthEnergyFactor)`
    /// (`BibiteGenes.cs:119`).
    #[inline]
    pub fn sum_body_and_health_energy_ratio<S: GenomeSettings + ?Sized>(&self, s: &S) -> f32 {
        self.body_energy_density(s) * (1.0 + s.health_energy_factor())
    }

    /// `HealthEnergyRatio = BodyEnergyDensity * healthEnergyFactor`
    /// (`BibiteGenes.cs:123`).
    #[inline]
    pub fn health_energy_ratio<S: GenomeSettings + ?Sized>(&self, s: &S) -> f32 {
        self.body_energy_density(s) * s.health_energy_factor()
    }

    /// `BodyEnergyAtBirth = 100 * D2SizeAtBirth * SumBodyAndHealthEnergyRatio`
    /// (`BibiteGenes.cs:127`).
    #[inline]
    pub fn body_energy_at_birth<S: GenomeSettings + ?Sized>(&self, s: &S) -> f32 {
        ENERGY_PER_D2 * self.d2_size_at_birth() * self.sum_body_and_health_energy_ratio(s)
    }

    /// `LayBodyEnergy = 100 * D2SizeAtBirth * (storableEnergyPerArea + Sum…)`
    /// (`BibiteGenes.cs:125`).
    #[inline]
    pub fn lay_body_energy<S: GenomeSettings + ?Sized>(&self, s: &S) -> f32 {
        ENERGY_PER_D2
            * self.d2_size_at_birth()
            * (s.storable_energy_per_area() + self.sum_body_and_health_energy_ratio(s))
    }

    /// Creates the child genome and its meta, applying the game's gene
    /// mutation algorithm (`BibiteGenes.cs:287-407`) and inheritance rules.
    ///
    /// * generation advances by one;
    /// * the four easter-egg flags are inherited with 10 % probability each
    ///   (`BibiteGenes.cs:299-314`), and a rare roll may set one
    ///   (`BibiteGenes.cs:370-387`, off unless
    ///   [`GenomeSettings::allow_easter_eggs`]);
    /// * `species`, `species_tag`, `is_mutant` and `deux_parent` are inherited;
    /// * parent ids are *not* set here — the caller sets them on the child meta;
    /// * mutation count is `Poisson(genes[9] + backgroundMutationChance +
    ///   factor.probability_increase)` (`BibiteGenes.cs:388`); each mutation
    ///   picks a uniform gene index, draws a delta with
    ///   `next_mutation_value` and clamps to the gene range
    ///   (`BibiteGenes.cs:395-407`).
    ///
    /// Deterministic for a given `rng` state.
    pub fn child<S: GenomeSettings + ?Sized>(
        &self,
        meta: &GenomeMeta,
        rng: &mut Rng,
        s: &S,
        factor: &MutationFactor,
    ) -> (Genome, GenomeMeta) {
        let mut child = self.clone();
        let mut child_meta = meta.clone();

        child_meta.generation = meta.generation.saturating_add(1);
        // Easter-egg flag inheritance: each parent flag propagates with p=0.1.
        // Short-circuit `&&` matches the game's RNG consumption (only draws when
        // the parent has the flag: BibiteGenes.cs:299-314).
        child_meta.is_rad = meta.is_rad && rng.f32() < 0.1;
        child_meta.is_rain = meta.is_rain && rng.f32() < 0.1;
        child_meta.is_killer = meta.is_killer && rng.f32() < 0.1;
        child_meta.is_source = meta.is_source && rng.f32() < 0.1;
        // Children do not inherit parent ids; the caller sets them at lay/hatch.
        child_meta.parent1 = None;
        child_meta.parent2 = None;

        if s.prevent_mutations() {
            return (child, child_meta);
        }

        // Rare easter-egg roll (game: Mutate(), BibiteGenes.cs:370-387).
        if s.allow_easter_eggs() {
            let period = s.easter_eggs_period().max(1) as f32;
            if rng.f32() < 1.0 / period {
                match rng.usize_below(4) {
                    0 => child_meta.is_rad = true,
                    1 => child_meta.is_rain = true,
                    2 => child_meta.is_source = true,
                    _ => child_meta.is_killer = true,
                }
            }
        }

        // `count` is drawn once; sigma is re-read every mutation because the
        // mutation-sigma gene itself may mutate first (BibiteGenes.cs:388-407).
        let lambda = child.gene(Gene::AverageMutationNumber)
            + s.background_mutation_chance()
            + factor.probability_increase;
        let count = poisson(rng, f64::from(lambda.max(0.0)));
        child.mutate_times(count as usize, rng, s, factor);

        (child, child_meta)
    }

    /// `BibiteGenes.OneMutation` `count` times (the game's `Mutate(n)`): a uniform gene moves by a drawn delta and is clamped to its range;
    /// the sigma is read again for each (the sigma gene itself may mutate first).
    pub fn mutate_times<S: GenomeSettings + ?Sized>(&mut self, count: usize, rng: &mut Rng, s: &S, factor: &MutationFactor) {
        for _ in 0..count {
            mutation_counters::add_gene_mutation(1);
            let index = rng.usize_below(N_GENES);
            let gene = ALL_GENES[index];
            let def = gene.def();
            let sigma = self.gene(Gene::MutationAmountSigma) + s.background_mutation_variance() + factor.intensity_increase;
            let delta = next_mutation_value(rng, self.genes[index], sigma, def.span(), def.purely_relative, s.relative_mutation_share());
            self.genes[index] = clamp(self.genes[index] + delta, def.min, def.max);
        }
    }
}

/// One draw from a Poisson distribution, matching the game's
/// `RandomExtension.NextPoisson` (`Here/ScriptHelpers/RandomExtension.cs:48-61`):
/// `L = exp(-λ); p = 1; k = 0; do { k++; p *= U } while (p > L); return k-1`.
///
/// The game consumes one `System.Random.NextDouble()` per iteration. This port
/// uses [`Rng::f64`] so the *distribution* matches while the sequence differs
/// (statistical parity; the game's RNG is unseeded).
pub(crate) fn poisson(rng: &mut Rng, lambda: f64) -> u32 {
    let limit = (-lambda).exp();
    let mut product = 1.0_f64;
    let mut k = 0u32;
    loop {
        k += 1;
        product *= rng.f64();
        if product <= limit {
            return k - 1;
        }
    }
}

/// Relative (multiplicative, log-normal) mutation component:
/// `value * ((1 + σ)^N - 1)` with `N ~ N(0, 1)`
/// (`Here/ScriptHelpers/RandomExtension.cs:41-46`).
pub(crate) fn next_relative_gaussian_mutation(
    rng: &mut Rng,
    value: f32,
    mutation_sigma: f32,
) -> f32 {
    if mutation_sigma <= -1.0 {
        // Domain guard: the game would produce NaN/∞ via Mathf.Pow for σ <= -1;
        // negative sigmas cannot arise from genes but radiation factors could.
        return 0.0;
    }
    let gaussian = rng.normal();
    let factor = (1.0 + mutation_sigma).powf(gaussian);
    value * (factor - 1.0)
}

/// The game's `NextMutationValue` (`Here/ScriptHelpers/RandomExtension.cs:33-39`):
/// blends a relative (multiplicative) delta and an absolute (`N(0, σ) * span`)
/// delta by `relativeMutationShare` (100 % relative for `purely_relative` genes).
pub(crate) fn next_mutation_value(
    rng: &mut Rng,
    gene_value: f32,
    mutation_sigma: f32,
    gene_span: f32,
    purely_relative: bool,
    relative_share: f32,
) -> f32 {
    let share = if purely_relative { 1.0 } else { relative_share };
    let relative = next_relative_gaussian_mutation(rng, gene_value, mutation_sigma * share);
    let absolute = rng.normal_with(0.0, mutation_sigma * (1.0 - share)) * gene_span;
    relative + absolute
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_within_ranges(genome: &Genome) -> bool {
        GENE_DEFS.iter().enumerate().all(|(i, def)| {
            let v = genome.genes[i];
            v >= def.min && v <= def.max && v.is_finite()
        })
    }

    #[test]
    fn gene_defs_match_enum_order_and_defaults() {
        for (i, gene) in ALL_GENES.iter().enumerate() {
            assert_eq!(gene.index(), i);
            assert_eq!(Gene::from_index(i), Some(*gene));
            assert_eq!(Gene::from_name(gene.def().name), Some(*gene));
            assert_eq!(Gene::from_name("NotAGene"), None);
            let def = gene.def();
            assert!(def.min <= def.default, "{}", def.name);
            assert!(def.default <= def.max, "{}", def.name);
        }
        // Spot checks against the editor settings table (spec §7.1).
        assert_eq!(Gene::LayTime.def().max, 51.0);
        assert_eq!(Gene::SpeedRatio.def().min, 1e-6);
        assert_eq!(Gene::ClockSpeed.def().min, 0.1);
        assert_eq!(Gene::StomachWAG.def().min, -1.0);
        assert_eq!(Gene::FatStorageDeadband.def().max, 0.5);
        assert!(Gene::SizeRatio.def().purely_relative);
        assert!(Gene::SpeedRatio.def().purely_relative);
        assert!(Gene::GrowthScale.def().purely_relative);
        assert!(!Gene::LayTime.def().purely_relative);
    }

    #[test]
    fn genes_to_color_is_the_games_body_colour() {
        let near = |actual: [f32; 3], expected: [f32; 3]| {
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < 1e-6)
        };
        // The basic bibite (0.1, 0.1, 0.1) is a LIGHT grey, not the dark raw pigment.
        assert!(near(genes_to_color(0.1, 0.1, 0.1), [0.9, 0.9, 0.9]));
        // Black pigment is white skin and white pigment is black skin.
        assert!(near(genes_to_color(0.0, 0.0, 0.0), [1.0, 1.0, 1.0]));
        assert!(near(genes_to_color(1.0, 1.0, 1.0), [0.0, 0.0, 0.0]));
        // Pure channels stay pure.
        assert!(near(genes_to_color(1.0, 0.0, 0.0), [1.0, 0.0, 0.0]));
        assert!(near(genes_to_color(0.0, 0.0, 1.0), [0.0, 0.0, 1.0]));
        // A pale red: brightest channel `1 - min`, darkest `1 - max`.
        assert!(near(genes_to_color(0.5, 0.2, 0.2), [0.8, 0.5, 0.5]));
        // The raw genes of the starter genome are the dark pigment the skin is NOT.
        let starter = Genome::starter();
        let skin = genes_to_color(
            starter.gene(Gene::ColorR),
            starter.gene(Gene::ColorG),
            starter.gene(Gene::ColorB),
        );
        assert!(skin[0] > 0.8 && skin[1] > 0.8 && skin[2] > 0.8, "{skin:?}");
    }

    #[test]
    fn genes_to_color_keeps_the_hue_and_the_spread_of_the_genes() {
        // `max - min` and the hue are those of the genes; only the level moves.
        let hue = |[r, g, b]: [f32; 3]| {
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max - min;
            let h = if (max - r).abs() < 1e-6 {
                ((g - b) / delta).rem_euclid(6.0)
            } else if (max - g).abs() < 1e-6 {
                (b - r) / delta + 2.0
            } else {
                (r - g) / delta + 4.0
            };
            h / 6.0
        };
        let mut checked = 0;
        for r in 0..=10 {
            for g in 0..=10 {
                for b in 0..=10 {
                    let raw = [r as f32 / 10.0, g as f32 / 10.0, b as f32 / 10.0];
                    let spread = raw[0].max(raw[1]).max(raw[2]) - raw[0].min(raw[1]).min(raw[2]);
                    if spread < 0.05 {
                        continue;
                    }
                    let skin = genes_to_color(raw[0], raw[1], raw[2]);
                    let skin_spread =
                        skin[0].max(skin[1]).max(skin[2]) - skin[0].min(skin[1]).min(skin[2]);
                    assert!((spread - skin_spread).abs() < 1e-5, "{raw:?} -> {skin:?}");
                    let (a, b) = (hue(raw), hue(skin));
                    let distance = (a - b).abs().min(1.0 - (a - b).abs());
                    assert!(distance < 1e-4, "{raw:?} -> {skin:?}: hue {a} vs {b}");
                    assert!(skin.iter().all(|c| (-1e-6..=1.0 + 1e-6).contains(c)));
                    checked += 1;
                }
            }
        }
        assert!(checked > 900, "{checked}");
    }

    #[test]
    fn starter_is_the_captured_basic_bibite_genome() {
        // Values from testdata/real/templates_current/Basic_bibite_0.6.3.1.bb8template.
        let g = Genome::starter();
        let expected = [
            (Gene::LayTime, 15.0),
            (Gene::BroodTime, 20.0),
            (Gene::HatchTime, 10.0),
            (Gene::SizeRatio, 1.0),
            (Gene::SpeedRatio, 0.75),
            (Gene::ViewAngle, 75.0),
            (Gene::ViewRadius, 180.0),
            (Gene::PheroSense, 150.0),
            (Gene::Diet, 0.2),
            (Gene::GrowthScale, 0.15),
            (Gene::StomachWAG, 2.0),
            (Gene::WombWAG, 1.0),
            (Gene::ArmorWAG, 0.15),
            (Gene::FatStorageThreshold, 0.5),
            (Gene::FatStorageDeadband, 0.25),
        ];
        for (gene, value) in expected {
            assert!((g.gene(gene) - value).abs() < 1e-6, "{gene:?}");
        }
        assert_eq!(g, Genome::defaults());
    }

    #[test]
    fn set_gene_clamps_to_editor_range() {
        let mut g = Genome::defaults();
        g.set_gene(Gene::LayTime, 1000.0);
        assert_eq!(g.gene(Gene::LayTime), 51.0);
        g.set_gene(Gene::LayTime, -1000.0);
        assert_eq!(g.gene(Gene::LayTime), 1.0);
        g.set_gene(Gene::StomachWAG, -5.0);
        assert_eq!(g.gene(Gene::StomachWAG), -1.0);
        g.set_gene(Gene::PheroSense, f32::NAN);
        // `f32::clamp` propagates NaN; the game's Mathf.Clamp also returns NaN.
        assert!(g.gene(Gene::PheroSense).is_nan());
    }

    #[test]
    fn derived_formulas_match_starter_numbers() {
        let g = Genome::starter();
        let s = DefaultGenomeSettings::game_defaults();
        // totalWAG = 2 + 1 + 1 + 0.15 + 1 + 1 + 1 = 7.15
        assert!((g.total_organ_wag() - 7.15).abs() < 1e-5);
        let womb = g.womb_area_portion();
        assert!((womb - 1.0 / 7.15).abs() < 1e-5);
        let growth_birth = 0.25; // (10/20)^2
        assert!((g.growth_at_birth() - growth_birth).abs() < 1e-6);
        let growth_mature = (growth_birth / womb).max(1.0);
        assert!((g.growth_at_mature() - growth_mature).abs() < 1e-5);
        assert!((g.d2_size_at_birth() - 0.25).abs() < 1e-6);
        assert!((g.d1_size_at_birth() - 0.5).abs() < 1e-6);
        assert!((g.area_at_birth() - 20.0).abs() < 1e-5);
        let expected_area_mature = 80.0 * growth_mature;
        assert!((g.area_at_mature() - expected_area_mature).abs() < 1e-4);
        assert!((g.length_at_mature() - 12.5 * growth_mature.sqrt()).abs() < 1e-4);
        // Body energy density = 2 + 75*0.003 + 180*0.002 + (0.15/7.15)*1.5
        let density = 2.0 + 75.0 * 0.003 + 180.0 * 0.002 + (0.15 / 7.15) * 1.5;
        assert!((g.body_energy_density(&s) - density).abs() < 1e-4);
        assert!((g.health_energy_ratio(&s) - density * 0.5).abs() < 1e-4);
        assert!((g.metabolic_efficiency(&s) - 0.8_f32.powf(0.75)).abs() < 1e-6);
        assert!((g.metabolic_rate() - 0.75).abs() < 1e-6);
        let sum = density * 1.5;
        assert!((g.body_energy_at_birth(&s) - 100.0 * 0.25 * sum).abs() < 1e-3);
        assert!((g.lay_body_energy(&s) - 100.0 * 0.25 * (1.5 + sum)).abs() < 1e-3);
    }

    #[test]
    fn negative_wags_contribute_zero() {
        let mut g = Genome::defaults();
        for gene in [
            Gene::StomachWAG,
            Gene::WombWAG,
            Gene::FatWAG,
            Gene::ArmorWAG,
            Gene::ThroatWAG,
            Gene::MouthMusclesWAG,
            Gene::MoveMusclesWAG,
        ] {
            g.set_gene(gene, -1.0);
        }
        assert_eq!(g.total_organ_wag(), 0.0);
        // Division is guarded by max(total, 0.0001).
        assert_eq!(g.organ_area_portion(Gene::StomachWAG), 0.0);
        assert!(g.growth_at_mature().is_finite());
        assert!(g.growth_at_mature() >= 1.0);
    }

    #[test]
    fn poisson_mean_matches_lambda() {
        let mut rng = Rng::from_seed(0xBEEF);
        let n = 20_000;
        let lambda = 2.0_f64;
        let mut sum = 0u64;
        for _ in 0..n {
            sum += u64::from(poisson(&mut rng, lambda));
        }
        let mean = sum as f64 / n as f64;
        assert!((mean - lambda).abs() < 0.1, "mean {mean}");
        // Lambda 0 must always draw zero events.
        for _ in 0..100 {
            assert_eq!(poisson(&mut rng, 0.0), 0);
        }
    }

    #[test]
    fn mutation_kernel_relative_and_absolute() {
        let mut rng = Rng::from_seed(7);
        // 100 % relative: a zero gene can never leave zero.
        for _ in 0..100 {
            assert_eq!(
                next_mutation_value(&mut rng, 0.0, 0.1, 50.0, true, 0.75),
                0.0
            );
        }
        // sigma 0 => no change in either component.
        for _ in 0..100 {
            assert_eq!(
                next_mutation_value(&mut rng, 3.0, 0.0, 50.0, false, 0.75),
                0.0
            );
        }
        // Absolute-only share: delta has mean ~0 and variance ~sigma^2*span^2.
        let mut rng = Rng::from_seed(11);
        let mut sum = 0.0f64;
        let mut sum_sq = 0.0f64;
        let n = 10_000;
        for _ in 0..n {
            let d = next_mutation_value(&mut rng, 5.0, 0.1, 1.0, false, 0.0) as f64;
            sum += d;
            sum_sq += d * d;
        }
        let mean = sum / n as f64;
        let var = sum_sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!((var - 0.01).abs() < 0.002, "var {var}");
    }

    #[test]
    fn child_is_deterministic_and_in_range() {
        let meta = GenomeMeta {
            generation: 7,
            is_rad: true,
            species_tag: Some("Test".into()),
            ..Default::default()
        };
        let s = DefaultGenomeSettings::game_defaults();
        let factor = MutationFactor {
            probability_increase: 5.0,
            intensity_increase: 0.2,
        };
        let a = Genome::starter().child(&meta, &mut Rng::from_seed(42), &s, &factor);
        let b = Genome::starter().child(&meta, &mut Rng::from_seed(42), &s, &factor);
        assert_eq!(a, b);
        assert_eq!(a.1.generation, 8);
        assert_eq!(a.1.species_tag.as_deref(), Some("Test"));
        assert!(all_within_ranges(&a.0));
        assert_eq!(a.1.is_mutant, meta.is_mutant);
        assert_eq!(a.1.species, meta.species);
        assert!(a.1.parent1.is_none() && a.1.parent2.is_none());

        let c = Genome::starter().child(&meta, &mut Rng::from_seed(43), &s, &factor);
        assert_ne!(
            a.0, c.0,
            "different seeds should produce different children"
        );
    }

    #[test]
    fn child_respects_prevent_mutations() {
        let meta = GenomeMeta::default();
        let mut s = DefaultGenomeSettings::game_defaults();
        s.prevent_mutations = true;
        let (child, child_meta) = Genome::starter().child(
            &meta,
            &mut Rng::from_seed(1),
            &s,
            &MutationFactor::default(),
        );
        assert_eq!(child, Genome::starter());
        assert_eq!(child_meta.generation, 1);
    }

    #[test]
    fn easter_eggs_are_born_only_when_the_player_allows_them() {
        // `BibiteGenes.Mutate`: `AllowEasterEggs && Random.value < 1 / EasterEggsProbability`, then one of the four flags.
        let parent = Genome::starter();
        let meta = GenomeMeta::default();
        let flagged =
            |meta: &GenomeMeta| meta.is_rad || meta.is_rain || meta.is_source || meta.is_killer;
        let mut settings = crate::settings::SimSettings::game_defaults();
        let mut rng = Rng::from_seed(11);
        let none = (0..200)
            .filter(|_| {
                let (_, child) =
                    parent.child(&meta, &mut rng, &settings, &MutationFactor::default());
                flagged(&child)
            })
            .count();
        assert_eq!(none, 0, "off by default: no easter eggs");
        // A period of 1 makes every mutated child one; each of the four kinds turns up.
        settings.allow_easter_eggs = true;
        settings.easter_eggs_period = 1;
        let mut kinds = [0_u32; 4];
        for _ in 0..400 {
            let (_, child) = parent.child(&meta, &mut rng, &settings, &MutationFactor::default());
            assert!(flagged(&child));
            kinds[0] += u32::from(child.is_rad);
            kinds[1] += u32::from(child.is_rain);
            kinds[2] += u32::from(child.is_source);
            kinds[3] += u32::from(child.is_killer);
        }
        assert!(kinds.iter().all(|count| *count > 50), "{kinds:?}");
        // A long period makes them rare again.
        settings.easter_eggs_period = 1_000_000;
        let rare = (0..400)
            .filter(|_| {
                let (_, child) =
                    parent.child(&meta, &mut rng, &settings, &MutationFactor::default());
                flagged(&child)
            })
            .count();
        assert!(rare <= 1, "{rare}");
    }

    #[test]
    fn easter_egg_flags_inherit_at_ten_percent() {
        let meta = GenomeMeta {
            is_rad: true,
            ..Default::default()
        };
        let s = DefaultGenomeSettings::game_defaults();
        let factor = MutationFactor::default();
        let trials = 10_000;
        let mut inherited = 0;
        let mut rng = Rng::from_seed(0xABCD);
        for _ in 0..trials {
            let (_, child) = Genome::starter().child(&meta, &mut rng, &s, &factor);
            if child.is_rad {
                inherited += 1;
            }
            assert!(!child.is_rain && !child.is_killer && !child.is_source);
        }
        let ratio = f64::from(inherited) / f64::from(trials);
        assert!((ratio - 0.1).abs() < 0.02, "ratio {ratio}");
    }

    #[test]
    fn child_mutation_count_is_usually_nonzero() {
        let meta = GenomeMeta::default();
        let s = DefaultGenomeSettings::game_defaults();
        // lambda = genes[9] (2.0) + background (1.5) = 3.5
        let factor = MutationFactor::default();
        let mut rng = Rng::from_seed(0x1234);
        let mut changed = 0u64;
        let trials = 2_000;
        for _ in 0..trials {
            let (child, _) = Genome::starter().child(&meta, &mut rng, &s, &factor);
            if child != Genome::starter() {
                changed += 1;
            }
        }
        // P(no mutation) = exp(-3.5) ≈ 0.030 => almost all children change.
        let changed_ratio = changed as f64 / f64::from(trials);
        assert!(changed_ratio > 0.95, "ratio {changed_ratio}");
    }
}

/// Mutation instrumentation for the evolution benchmark (task-48).
///
/// Always compiled (a few relaxed atomics on the birth path, not the per-tick
/// hot path) so `cargo run --release --example evolution_bench` gets the same
/// counts as the test harness.
pub mod mutation_counters {
    use std::sync::atomic::{AtomicU64, Ordering};

    static GENE_MUTATIONS: AtomicU64 = AtomicU64::new(0);

    /// Adds `count` gene mutation kernels.
    pub fn add_gene_mutation(count: u64) {
        GENE_MUTATIONS.fetch_add(count, Ordering::Relaxed);
    }

    /// Gene mutations applied since [`reset`].
    pub fn gene_mutations() -> u64 {
        GENE_MUTATIONS.load(Ordering::Relaxed)
    }

    /// Resets the instrumentation counter.
    pub fn reset() {
        GENE_MUTATIONS.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod construction_api_tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn randomize_is_seeded_and_respects_ranges() {
        let mut a = Rng::from_seed(7);
        let mut b = Rng::from_seed(7);
        let first = Genome::randomized(&mut a);
        let second = Genome::randomized(&mut b);
        assert_eq!(first, second);
        for (index, def) in GENE_DEFS.iter().enumerate() {
            if index == Gene::BroodTime.index() {
                continue;
            }
            let value = first.genes[index];
            assert!(
                value >= def.min - 1e-5 && value <= def.max + 1e-5,
                "{} out of range: {value}",
                def.name
            );
        }
        // RandomGenes forces BroodTime to at least HatchTime / 0.75, which can
        // exceed its editor maximum; this is the game's exact post-processing.
        assert!(first.gene(Gene::BroodTime) >= first.gene(Gene::HatchTime) / 0.75);
        assert!(first.gene(Gene::SizeRatio) > 0.0);
    }

    #[test]
    fn random_color_only_touches_color_genes() {
        let mut genome = Genome::starter();
        let before = *genome.values();
        let mut rng = Rng::from_seed(11);
        genome.randomize_color(&mut rng);
        for (index, def) in GENE_DEFS.iter().enumerate() {
            if matches!(def.name, "ColorR" | "ColorG" | "ColorB") {
                assert_ne!(genome.genes[index], before[index]);
            } else {
                assert_eq!(genome.genes[index], before[index]);
            }
        }
    }

    #[test]
    fn gene_editor_metadata_is_available() {
        assert_eq!(Gene::LayTime.display_name(), "Lay Time");
        assert_eq!(Gene::LayTime.units(), "s");
        assert_eq!(Gene::LayTime.range(), (1.0, 51.0));
        assert_eq!(Gene::LayTime.default_value(), 15.0);
        assert_eq!(Gene::LayTime.span(), 50.0);
        assert_eq!(Gene::from_name("LayTime"), Some(Gene::LayTime));
    }

    #[test]
    fn selected_gene_mutation_accepts_sim_settings() {
        let mut genome = Genome::starter();
        let settings = crate::settings::SimSettings::game_defaults();
        let mut rng = Rng::from_seed(23);
        let value = genome.mutate_gene(
            Gene::SizeRatio,
            &mut rng,
            &settings,
            &MutationFactor::default(),
        );
        assert_eq!(genome.gene(Gene::SizeRatio), value);
    }

    #[test]
    fn selected_gene_mutation_changes_only_that_gene() {
        let mut genome = Genome::starter();
        let before = *genome.values();
        let settings = DefaultGenomeSettings::game_defaults();
        let mut rng = Rng::from_seed(19);
        let new_value = genome.mutate_gene(
            Gene::LayTime,
            &mut rng,
            &settings,
            &MutationFactor::default(),
        );
        assert_eq!(genome.genes[Gene::LayTime.index()], new_value);
        for (index, def) in GENE_DEFS.iter().enumerate() {
            if index != Gene::LayTime.index() {
                assert_eq!(genome.genes[index], before[index], "{} changed", def.name);
            }
        }
        let def = Gene::LayTime.def();
        assert!(new_value >= def.min && new_value <= def.max);
    }
}
