//! Zones: geometry, plant biomass production, pellet spawning, zone movement,
//! initial seeding and scenario zone-group expansion.
//!
//! Spec: `docs/spec/03-world-ecology.md` §1 (zone model), §1.4 (production loop),
//! §1.5 (spawn distributions), §1.6 (the 10 s `pelletBiomass` recount lag),
//! §1.7 (movement/bounds), §1.8 (zone lists/groups), §8.2 (initial seeding) and
//! Appendix A (Default.zip worked example).
//!
//! # TEMPORARY settings model (W2)
//!
//! `settings.rs` (`fmt-dev`) has not landed yet, so this module defines a
//! **temporary** [`crate::zone::ZoneConfig`] / [`crate::zone::ZoneGlobals`] mirroring the game's
//! `ZoneSettings` / world-scope settings, plus the [`crate::settings::SpawnDistribution`] and
//! [`crate::settings::MovementType`] enums. When `settings::SimSettings` lands, one mapping
//! function replaces [`crate::zone::ZoneGlobals::game_defaults`] + the `ZoneConfig`
//! construction path; nothing else must change. Field names intentionally match
//! the spec's snake_case JSON names so the mapping is mechanical.
//!
//! # Units / conventions
//!
//! * Distances in world units (`u`), energy in `E`, area in `u²`, time in
//!   simulated seconds. `dt` is always the fixed tick length.
//! * Zone positions are `pos = sim_size · (pos_x, pos_y)`, with `pos_x/pos_y`
//!   normalised to `[-1, 1]` and clamped by the zone size (`CapPosition`).
//! * Pellets spawned by a zone are owned by it (`PelletSpawn.owner_zone`) and
//!   tracked in `ZoneData::owned_pellets` for recycling/recounting.
//!
//! # Determinism
//!
//! Each zone owns a derived RNG stream
//! (`Rng::derive(world_seed, domain::FOOD, zone_id + 1)`), used for pellet size
//! and spawn position. Two stores built from the same seed and configs produce
//! identical spawn sequences.

use crate::arena::Handle;
use crate::ids::{PelletId, ZoneId};
use crate::math::{PI, Vec2, clamp};
use crate::pellet::{MaterialKind, PelletSpawn, PelletStore};
use crate::rng::{Rng, domain};
use crate::world::{SpatialHash, WorldBounds, random_inside_unit_circle};

// The distribution/movement enums are owned by the settings façade
// (`fmt-dev`, Phase B). Zone code consumes and re-exports them so callers get a
// single type across settings/zone/save.
pub use crate::settings::{MovementType, SpawnDistribution};

/// Seconds between `pelletBiomass` recounts (`Zone.UpdateBiomassCount`, spec
/// §1.6).
pub const RECOUNT_PERIOD: f32 = 10.0;

/// Spawn threshold factor: the production loop spawns while
/// `pellet_progress > 1.5 · avg_pellet_energy` (spec §1.4).
pub const SPAWN_THRESHOLD_FACTOR: f32 = 1.5;

/// Power-Gaussian base for the pellet size multiplier (spec §1.4).
pub const PELLET_SIZE_POWER_BASE: f32 = 1.5;

/// Lower clamp of the pellet size multiplier (spec §1.4).
pub const PELLET_SIZE_MIN_FACTOR: f32 = 0.1;

/// Upper clamp of the pellet size multiplier (spec §1.4).
pub const PELLET_SIZE_MAX_FACTOR: f32 = 5.0;

/// Safety cap on the number of pellets a single zone may seed in one
/// `initial_seeding` call. The game loops until the biomass target, which can
/// hang in the red-death edge case (spec Q6); RUSTibites caps it and reports
/// `capped = true` instead.
pub const MAX_SEEDING_ITERATIONS_PER_ZONE: u32 = 100_000;

/// Safety cap on production-loop iterations per zone per tick (defensive only;
/// the loop normally runs zero or one time).
pub const MAX_SPAWN_ITERATIONS_PER_TICK: u32 = 10_000;

/// Converts a settings-façade material to the pellet module's material kind.
#[inline]
pub fn material_from_settings(material: crate::settings::MaterialKind) -> MaterialKind {
    match material {
        crate::settings::MaterialKind::Plant => MaterialKind::Plant,
        crate::settings::MaterialKind::Meat => MaterialKind::Meat,
        crate::settings::MaterialKind::Fat => MaterialKind::Fat,
        crate::settings::MaterialKind::Armor => MaterialKind::Armor,
    }
}

/// Converts a pellet material kind to the settings-façade material.
#[inline]
pub fn material_to_settings(material: MaterialKind) -> crate::settings::MaterialKind {
    match material {
        MaterialKind::Plant => crate::settings::MaterialKind::Plant,
        MaterialKind::Meat => crate::settings::MaterialKind::Meat,
        MaterialKind::Fat => crate::settings::MaterialKind::Fat,
        MaterialKind::Armor => crate::settings::MaterialKind::Armor,
    }
}

/// The world-scope settings the zone system needs, as a thin wrapper around
/// [`crate::settings::SimSettings`] (single source of truth; no duplicated fields).
///
/// `Deref` exposes the full settings; the accessors below are the exact values
/// the zone formulas read (spec §1.1/§2.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneGlobals(pub crate::settings::SimSettings);

impl core::ops::Deref for ZoneGlobals {
    type Target = crate::settings::SimSettings;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl core::ops::DerefMut for ZoneGlobals {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl ZoneGlobals {
    /// Captured game defaults for a fresh scenario (spec §1.1/§2.2).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self(crate::settings::SimSettings::game_defaults())
    }

    /// Wraps the settings façade's effective values.
    #[must_use]
    pub fn from_settings(settings: &crate::settings::SimSettings) -> Self {
        Self(settings.clone())
    }

    /// Global biomass density in `E/u²` (`biomassDensity`).
    #[inline]
    pub fn global_biomass_density(&self) -> f32 {
        self.0.independents.biomass_density
    }

    /// Global fertility in `E/u²/s` (`pelletGrowth`).
    #[inline]
    pub fn global_pellet_growth(&self) -> f32 {
        self.0.independents.pellet_growth
    }

    /// Default pellet energy (`pelletEnergy`).
    #[inline]
    pub fn pellet_energy(&self) -> f32 {
        self.0.scenario.pellet_energy
    }

    /// Pellet-energy floor (`pelletEnergy.minValue`, load-bearing).
    #[inline]
    pub fn pellet_energy_min(&self) -> f32 {
        self.0.pellet_energy_min
    }

    /// Fraction of max biomass seeded at world start (`initialSeeding`).
    #[inline]
    pub fn initial_seeding(&self) -> f32 {
        self.0.scenario.initial_seeding
    }

    /// Global zone speed multiplier (`globalZoneSpeed`).
    #[inline]
    pub fn global_zone_speed(&self) -> f32 {
        self.0.scenario.global_zone_speed
    }
}

/// **TEMPORARY (W2)** per-zone definition mirroring the game's `ZoneSettings`
/// (spec §1.1). Field names match the save JSON in snake_case.
///
/// The game keeps both relative and absolute size fields in sync; this struct
/// does the same and [`crate::settings::ZoneSettings::radius_is_relative`] selects which one is
/// authoritative, exactly like `ZoneSettings.sizeScalesWithSim`.
/// Per-zone definition: a thin wrapper around [`crate::settings::ZoneSettings`] so the
/// serialized settings type is the single source of truth, plus the geometry
/// helpers the zone runtime needs (spec §1.1).
///
/// `Deref` exposes the `ZoneSettings` fields; `zone_id` is the serialized id when
/// present and the runtime (list-order) id otherwise.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneConfig(pub crate::settings::ZoneSettings);

impl core::ops::Deref for ZoneConfig {
    type Target = crate::settings::ZoneSettings;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl core::ops::DerefMut for ZoneConfig {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl ZoneConfig {
    /// Runtime zone id (serialized id when present, otherwise the list index).
    #[inline]
    pub fn runtime_zone_id(&self) -> u32 {
        self.0.zone_id.unwrap_or(0)
    }

    /// Sets the runtime zone id (also used for list-order renumbering).
    #[inline]
    pub fn set_runtime_zone_id(&mut self, id: u32) {
        self.0.zone_id = Some(id);
    }
    /// `ZoneSettings.DefaultZone()`: Plant, radius relative 1.0, fertility 1
    /// (`Here/SettingScripts/ZoneSettings.cs:561-568`).
    pub fn default_zone() -> Self {
        let mut settings = crate::settings::ZoneSettings::game_defaults();
        settings.material = Some(crate::settings::MaterialKind::Plant);
        settings.radius_relative = 1.0;
        settings.fertility = 1.0;
        Self(settings)
    }

    /// `ZoneSettings.DefaultTemplate()`: radius relative 0.15, fertility 10,
    /// biomass density 10, `posX/posY/name = null`, `zoneID = -1`
    /// (`Here/SettingScripts/ZoneSettings.cs:545-559`). The null name/position
    /// cannot be represented here; use [`Self::default`] and override.
    pub fn default_template() -> Self {
        let mut settings = crate::settings::ZoneSettings::game_defaults();
        settings.material = Some(crate::settings::MaterialKind::Plant);
        settings.distribution = SpawnDistribution::CentricGradual;
        settings.fertility = 10.0;
        settings.biomass_density = 10.0;
        settings.radius_relative = 0.15;
        Self(settings)
    }

    /// Wraps a settings-façade [`crate::settings::ZoneSettings`] (identity now;
    /// kept for existing call sites).
    #[must_use]
    pub fn from_settings(settings: &crate::settings::ZoneSettings) -> Self {
        Self(settings.clone())
    }

    /// `true` for `Ring`/`FlatRing` (`ZoneSettings.isRing`).
    #[inline]
    pub fn is_ring(&self) -> bool {
        matches!(
            self.distribution,
            SpawnDistribution::Ring | SpawnDistribution::FlatRing
        )
    }

    /// `true` for `Rect` (`ZoneSettings.isRect`).
    #[inline]
    pub fn is_rect(&self) -> bool {
        self.distribution == SpawnDistribution::Rect
    }

    /// `true` for non-ring, non-rect zones (`ZoneSettings.isCircle`).
    #[inline]
    pub fn is_circle(&self) -> bool {
        !self.is_ring() && !self.is_rect()
    }

    /// Relative radius (fraction of `sim_size`): `radiusRelative`, or
    /// `radiusAbsolute / sim_size` when absolute sizing is used.
    #[inline]
    pub fn relative_radius(&self, sim_size: f32) -> f32 {
        if self.radius_is_relative {
            self.radius_relative
        } else {
            self.radius_absolute / sim_size
        }
    }

    /// Absolute radius in world units: `radiusRelative · sim_size` or
    /// `radiusAbsolute`.
    #[inline]
    pub fn absolute_radius(&self, sim_size: f32) -> f32 {
        if self.radius_is_relative {
            self.radius_relative * sim_size
        } else {
            self.radius_absolute
        }
    }

    /// Relative width: `widthRelative`, or `widthAbsolute / sim_size`.
    #[inline]
    pub fn relative_width(&self, sim_size: f32) -> f32 {
        if self.radius_is_relative {
            self.width_relative
        } else {
            self.width_absolute / sim_size
        }
    }

    /// Absolute width in world units.
    #[inline]
    pub fn absolute_width(&self, sim_size: f32) -> f32 {
        if self.radius_is_relative {
            self.width_relative * sim_size
        } else {
            self.width_absolute
        }
    }

    /// Relative height: `heightRelative`, or `heightAbsolute / sim_size`.
    #[inline]
    pub fn relative_height(&self, sim_size: f32) -> f32 {
        if self.radius_is_relative {
            self.height_relative
        } else {
            self.height_absolute / sim_size
        }
    }

    /// Absolute height in world units.
    #[inline]
    pub fn absolute_height(&self, sim_size: f32) -> f32 {
        if self.radius_is_relative {
            self.height_relative * sim_size
        } else {
            self.height_absolute
        }
    }

    /// Zone area in `u²` (`ZoneSettings.area`):
    /// `πr²` for discs, `πr²(1 − inside²)` for rings, `width·height` for rects.
    pub fn area(&self, sim_size: f32) -> f32 {
        let radius = self.absolute_radius(sim_size);
        match self.distribution {
            SpawnDistribution::Flat
            | SpawnDistribution::CentricGradual
            | SpawnDistribution::ExteriorGradual => PI * radius * radius,
            SpawnDistribution::Ring | SpawnDistribution::FlatRing => {
                PI * radius * radius * (1.0 - self.inside_radius * self.inside_radius)
            }
            SpawnDistribution::Rect => {
                self.absolute_width(sim_size) * self.absolute_height(sim_size)
            }
        }
    }

    /// Derived zone radius in world units (`radius`): for rect zones the game
    /// uses `max(width, height)/2` (`AlignSize`), otherwise the absolute radius.
    pub fn shape_radius(&self, sim_size: f32) -> f32 {
        if self.is_rect() {
            self.absolute_width(sim_size)
                .max(self.absolute_height(sim_size))
                / 2.0
        } else {
            self.absolute_radius(sim_size)
        }
    }

    /// Max biomass in `E` (`ZoneSettings.maxBiomass`):
    /// `biomassDensity · area · globalBiomassDensity`; 0 when material is None.
    pub fn max_biomass(&self, globals: &ZoneGlobals, sim_size: f32) -> f32 {
        match self.material {
            None => 0.0,
            Some(_) => {
                self.biomass_density * self.area(sim_size) * globals.global_biomass_density()
            }
        }
    }

    /// Production in `E/s` (`ZoneSettings.totalGrowth`):
    /// `fertility · area · globalPelletGrowth`; 0 when material is None.
    pub fn total_growth(&self, globals: &ZoneGlobals, sim_size: f32) -> f32 {
        match self.material {
            None => 0.0,
            Some(_) => self.fertility * self.area(sim_size) * globals.global_pellet_growth(),
        }
    }

    /// Average spawned pellet energy in `E` (`Zone.UpdateAvgPelletEnergy`):
    /// `pelletEnergy · pelletSize`.
    #[inline]
    pub fn avg_pellet_energy(&self, globals: &ZoneGlobals) -> f32 {
        globals.pellet_energy() * self.pellet_size
    }

    /// UI/CLI pellet estimate (`ZoneSettings.estimatedPellets`):
    /// `ceil(maxBiomass / (pelletSize · pelletEnergy))`.
    pub fn estimated_pellets(&self, globals: &ZoneGlobals, sim_size: f32) -> u32 {
        if self.material.is_none() || globals.pellet_energy() <= 0.0 {
            return 0;
        }
        let max = self.max_biomass(globals, sim_size);
        let per = (self.pellet_size * globals.pellet_energy()).max(1e-6);
        (max / per).ceil() as u32
    }

    /// Clamps `pos_x/pos_y` into the legal range (`ZoneSettings.CapPosition`).
    pub fn cap_position(&mut self, sim_size: f32) {
        let (cap_x, cap_y) = if self.is_rect() {
            (
                clamp(1.0 - self.relative_width(sim_size) / 2.0, 0.0, 1.0),
                clamp(1.0 - self.relative_height(sim_size) / 2.0, 0.0, 1.0),
            )
        } else {
            let cap = clamp(1.0 - self.relative_radius(sim_size), 0.0, 1.0);
            (cap, cap)
        };
        self.pos_x = clamp(self.pos_x, -cap_x, cap_x);
        self.pos_y = clamp(self.pos_y, -cap_y, cap_y);
    }

    /// Sets a normalised position, clamped (`UpdatePosition` + `CapPosition`).
    pub fn set_position(&mut self, pos_x: f32, pos_y: f32, sim_size: f32) {
        self.pos_x = pos_x;
        self.pos_y = pos_y;
        self.cap_position(sim_size);
    }

    /// `ZoneSettings.SetRandomPositionInRange`: uniform within the legal
    /// position range.
    pub fn randomize_position(&mut self, rng: &mut Rng, sim_size: f32) {
        let (cap_x, cap_y) = if self.is_rect() {
            (
                clamp(1.0 - self.relative_width(sim_size) / 2.0, 0.0, 1.0),
                clamp(1.0 - self.relative_height(sim_size) / 2.0, 0.0, 1.0),
            )
        } else {
            let cap = clamp(1.0 - self.relative_radius(sim_size), 0.0, 1.0);
            (cap, cap)
        };
        self.pos_x = rng.f32_range(-cap_x, cap_x);
        self.pos_y = rng.f32_range(-cap_y, cap_y);
    }
}

/// Scenario zone-group definition: thin wrapper around
/// [`crate::settings::ZoneGroupSettings`] (spec §1.8; single source of truth).
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneGroupConfig(pub crate::settings::ZoneGroupSettings);

impl core::ops::Deref for ZoneGroupConfig {
    type Target = crate::settings::ZoneGroupSettings;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl core::ops::DerefMut for ZoneGroupConfig {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Default for ZoneGroupConfig {
    fn default() -> Self {
        let mut group = crate::settings::ZoneGroupSettings::game_defaults();
        group.name = "Group 1".to_owned();
        group.template.radius_relative = 0.05;
        Self(group)
    }
}

impl ZoneGroupConfig {
    /// Number of generated zones (`ZoneGroupSettings.count`).
    pub fn count(&self, sim_size: f32) -> u32 {
        self.0.count(sim_size)
    }

    /// Wraps a settings-façade [`crate::settings::ZoneGroupSettings`].
    #[must_use]
    pub fn from_settings(settings: &crate::settings::ZoneGroupSettings) -> Self {
        Self(settings.clone())
    }
}

/// Live zone state: configuration + cached derived values + runtime production.
#[derive(Debug)]
pub struct ZoneData {
    /// Zone definition.
    pub config: ZoneConfig,
    /// Stable runtime handle (`slot = index in ZoneStore.zones`, `generation =
    /// store generation`).
    pub id: ZoneId,
    /// World-space centre (`sim_size · (pos_x, pos_y)`).
    pub pos: Vec2,
    /// Current movement velocity in `u/s`.
    pub vel: Vec2,
    /// World-space centre delta from the last movement phase (used by
    /// `Attached` followers).
    pub last_delta: Vec2,
    /// Absolute shape radius (`radius` in the game): disc radius or rect
    /// `max(w,h)/2`.
    pub radius: f32,
    /// Area in `u²`.
    pub area: f32,
    /// Ring inner radius in world units (0 for non-rings).
    pub inside_radius_abs: f32,
    /// Rect half extents (zero for non-rects).
    pub half_extents: Vec2,
    /// `maxBiomass` in `E` (cached from config + globals).
    pub max_biomass: f32,
    /// `totalGrowth` in `E/s` (cached).
    pub total_growth: f32,
    /// `avgPelletEnergy` in `E` (cached).
    pub avg_pellet_energy: f32,
    /// `true` for `Rect` zones.
    pub is_rect: bool,
    /// `true` for `Ring`/`FlatRing` zones.
    pub is_ring: bool,
    /// Accumulated production waiting to become a pellet (`pelletProgress`).
    pub pellet_progress: f32,
    /// Cached zone-owned pellet biomass in `E` (`pelletBiomass`), lagging by up
    /// to [`RECOUNT_PERIOD`] seconds (spec §1.6).
    pub pellet_biomass: f32,
    /// Pellets owned by this zone, roughly creation order (recycle order).
    pub owned_pellets: Vec<PelletId>,
    /// Seconds since the last biomass recount.
    pub time_since_recount: f32,
    /// Resolved target zone index for `Attached` movement.
    pub target_index: Option<usize>,
    /// Per-zone deterministic RNG stream for pellet size/position.
    rng: Rng,
    /// `zoneData`: where the zone was and how fertile, every few simulated minutes
    /// ([`crate::zone_log::ZoneLog`]); the species Position tab shows it.
    pub log: crate::zone_log::ZoneLog,
    /// The zone group that generated this zone (`ZoneGroupSettings.zones`: an index into the world's groups), `None` for an explicit zone
    /// (`ScenarioSettings.zones`). The store keeps the explicit zones first and then each group's zones one after the other
    /// (`ScenarioSettings.UpdateAllZonesList`), so a zone's list position is its zone id.
    pub group: Option<u32>,
}

impl ZoneData {
    /// Builds a zone from a config; assigns the runtime handle for `index` and
    /// derives the per-zone RNG from `world_seed` + `zone_id`.
    pub fn new(
        config: ZoneConfig,
        index: usize,
        generation: u32,
        bounds: &WorldBounds,
        globals: &ZoneGlobals,
        world_seed: u64,
    ) -> Self {
        let sim_size = bounds.sim_size();
        let mut rng = Rng::derive(
            world_seed,
            domain::FOOD,
            u64::from(config.runtime_zone_id()) + 1,
        );
        let mut config = config;
        config.cap_position(sim_size);
        let pos = Vec2::new(sim_size * config.pos_x, sim_size * config.pos_y);
        let speed = Self::movement_speed(&config, globals, sim_size);
        // The game initialises zone velocity with a random direction
        // (`Zone.InitializeZone: vel = Random.insideUnitCircle * speed`).
        let vel = random_inside_unit_circle(&mut rng) * speed;
        let mut zone = Self {
            config,
            id: ZoneId::from_handle(Handle::new(index as u32, generation)),
            pos,
            vel,
            last_delta: Vec2::ZERO,
            radius: 0.0,
            area: 0.0,
            inside_radius_abs: 0.0,
            half_extents: Vec2::ZERO,
            max_biomass: 0.0,
            total_growth: 0.0,
            avg_pellet_energy: 0.0,
            is_rect: false,
            is_ring: false,
            pellet_progress: 0.0,
            pellet_biomass: 0.0,
            owned_pellets: Vec::new(),
            time_since_recount: 0.0,
            target_index: None,
            rng,
            log: crate::zone_log::ZoneLog::new(),
            group: None,
        };
        zone.refresh(bounds, globals);
        zone
    }

    /// `new ZoneDataPoint(zone)`: the zone as the log records it now.
    #[must_use]
    pub fn log_point(&self, sim_size: f32) -> crate::zone_log::ZoneDataPoint {
        let radius = if self.config.is_rect() {
            (self.config.relative_width(sim_size) / 2.0)
                .max(self.config.relative_height(sim_size) / 2.0)
        } else {
            self.config.relative_radius(sim_size)
        };
        crate::zone_log::ZoneDataPoint::new(
            [self.config.pos_x, self.config.pos_y],
            radius,
            self.total_growth,
            self.pellet_biomass,
        )
    }

    /// Recomputes cached derived geometry/production values after a config
    /// change.
    pub fn refresh(&mut self, bounds: &WorldBounds, globals: &ZoneGlobals) {
        let sim_size = bounds.sim_size();
        self.is_rect = self.config.is_rect();
        self.is_ring = self.config.is_ring();
        self.radius = self.config.shape_radius(sim_size);
        self.area = self.config.area(sim_size);
        self.inside_radius_abs = if self.config.is_ring() {
            self.config.inside_radius * self.radius
        } else {
            0.0
        };
        self.half_extents = if self.is_rect {
            Vec2::new(
                self.config.absolute_width(sim_size) / 2.0,
                self.config.absolute_height(sim_size) / 2.0,
            )
        } else {
            Vec2::ZERO
        };
        self.max_biomass = self.config.max_biomass(globals, sim_size);
        self.total_growth = self.config.total_growth(globals, sim_size);
        self.avg_pellet_energy = self.config.avg_pellet_energy(globals);
    }

    /// Zone movement speed in `u/s` (`Zone.UpdateSpeed`):
    /// `simSize · globalZoneSpeed · zone.speed / 3600`.
    pub fn movement_speed(config: &ZoneConfig, globals: &ZoneGlobals, sim_size: f32) -> f32 {
        sim_size * globals.global_zone_speed() * config.speed / 3600.0
    }

    /// `ZoneSettings.estimatedPellets` for this zone.
    #[inline]
    pub fn estimated_pellets(&self, globals: &ZoneGlobals, sim_size: f32) -> u32 {
        self.config.estimated_pellets(globals, sim_size)
    }

    /// Free biomass `max_biomass − pellet_biomass` (may be negative).
    #[inline]
    pub fn free_biomass(&self) -> f32 {
        self.max_biomass - self.pellet_biomass
    }

    /// `true` if `pos` lies inside the zone shape (rect/ring/disc).
    pub fn contains(&self, pos: Vec2) -> bool {
        let delta = pos - self.pos;
        if self.is_rect {
            delta.x.abs() <= self.half_extents.x && delta.y.abs() <= self.half_extents.y
        } else {
            let dist = delta.length();
            if self.is_ring {
                dist >= self.inside_radius_abs && dist <= self.radius
            } else {
                dist <= self.radius
            }
        }
    }

    /// Legal centre bounds `(max_x, max_y)` from `sim_size` (`Zone.UpdatePosition`):
    /// rects use `simSize − halfExtent`, discs use `simSize − radius` on both axes.
    pub fn centre_bounds(&self, sim_size: f32) -> (f32, f32) {
        if self.is_rect {
            (
                sim_size - self.half_extents.x,
                sim_size - self.half_extents.y,
            )
        } else {
            let max_d = sim_size - self.radius;
            (max_d, max_d)
        }
    }

    /// Samples a spawn offset relative to the zone centre for this zone's
    /// distribution (spec §1.5, `Zone.GetSpawnLocation`).
    pub fn spawn_offset(&mut self) -> Vec2 {
        match self.config.distribution {
            SpawnDistribution::Ring => self.ring_point(self.radius, self.config.inside_radius, 3.0),
            SpawnDistribution::FlatRing => {
                self.ring_point(self.radius, self.config.inside_radius, 1.0)
            }
            SpawnDistribution::Rect => Vec2::new(
                self.rng.f32_range(-1.0, 1.0) * self.half_extents.x,
                self.rng.f32_range(-1.0, 1.0) * self.half_extents.y,
            ),
            SpawnDistribution::Flat => self.warped_disc_point(1.0) * self.radius,
            SpawnDistribution::CentricGradual => self.warped_disc_point(0.5) * self.radius,
            SpawnDistribution::ExteriorGradual => self.warped_disc_point(2.0) * self.radius,
        }
    }

    /// Full world-space spawn position (`pos + offset`).
    pub fn spawn_location(&mut self) -> Vec2 {
        self.pos + self.spawn_offset()
    }

    /// `Zone.InsideUnitCircleWarped`: `u.normalized · (1 − (1 − |u|)^p)` where
    /// `u` is uniform in the unit disc.
    fn warped_disc_point(&mut self, power: f32) -> Vec2 {
        let u = random_inside_unit_circle(&mut self.rng);
        let magnitude = u.length();
        let dir = u.normalize_or(Vec2::X);
        dir * (1.0 - (1.0 - magnitude).powf(power))
    }

    /// `RandomExtension.RandomPointInRing(outsideRadius, insideRadiusRelative,
    /// warp)` (spec §1.5).
    fn ring_point(&mut self, outside_radius: f32, inside_relative: f32, warp: f32) -> Vec2 {
        let dir = random_unit_direction(&mut self.rng);
        let inner_sq = inside_relative * inside_relative;
        let t = ((self.rng.f32() * (1.0 - inner_sq) + inner_sq).sqrt() - inside_relative)
            / (1.0 - inside_relative);
        let radius = (1.0 + t.powf(1.0 / warp) - (1.0 - t).powf(1.0 / warp)) / 2.0
            * (1.0 - inside_relative)
            + inside_relative;
        dir * (outside_radius * radius)
    }

    /// `Zone.Move`: applies the zone's movement mode for one tick and reflects
    /// the centre at the simulation bounds. `Attached` followers use their
    /// target's `last_delta` (see [`ZoneStore::update_movement`]).
    pub fn move_zone(&mut self, dt: f32, bounds: &WorldBounds, globals: &ZoneGlobals) {
        let sim_size = bounds.sim_size();
        let speed = Self::movement_speed(&self.config, globals, sim_size);
        let old_pos = self.pos;
        match self.config.movement {
            MovementType::Free => {
                self.vel += random_inside_unit_circle(&mut self.rng) * dt * speed / 10.0;
                if self.vel.length() > speed {
                    self.vel = self.vel.normalized() * speed;
                }
                self.pos += self.vel * dt;
            }
            MovementType::None | MovementType::Repulsed | MovementType::Orbit => {
                self.vel = Vec2::ZERO;
            }
            MovementType::Attached => {
                // The game implements Attached by transform parenting; the
                // behavioural equivalent is "follow the target's per-tick
                // translation". Repulsed/Orbit are legacy no-ops (spec §1.7).
                let delta = self.last_delta;
                self.pos += delta;
                self.vel = if dt > 0.0 { delta / dt } else { Vec2::ZERO };
            }
        }

        // Bounds reflection (`Zone.Move`): compare against centre bounds and
        // flip the velocity component on contact.
        let (max_x, max_y) = self.centre_bounds(sim_size);
        if self.pos.x.abs() > max_x {
            self.pos.x = self.pos.x.signum() * max_x;
            self.vel.x *= -1.0;
        }
        if self.pos.y.abs() > max_y {
            self.pos.y = self.pos.y.signum() * max_y;
            self.vel.y *= -1.0;
        }
        // Keep the normalised config position in sync (`Move` writes posX/posY).
        self.config.pos_x = self.pos.x / sim_size;
        self.config.pos_y = self.pos.y / sim_size;
        self.last_delta = self.pos - old_pos;
    }

    /// Recomputes `pellet_biomass` from the live owned pellets and prunes stale
    /// ids (`Zone.RecountBiomass`).
    pub fn recount(&mut self, pellets: &PelletStore) {
        self.owned_pellets.retain(|id| pellets.contains(*id));
        self.pellet_biomass = self
            .owned_pellets
            .iter()
            .map(|id| pellets.energy(*id).unwrap_or(0.0))
            .sum();
        self.time_since_recount = 0.0;
    }

    /// Drops a pellet id from the owned list (used when a pellet is removed).
    pub fn forget_pellet(&mut self, id: PelletId) {
        if let Some(index) = self.owned_pellets.iter().position(|owned| *owned == id) {
            self.owned_pellets.remove(index);
        }
    }
}

/// Returns a uniformly random unit vector (equivalent to Unity's
/// `Random.insideUnitCircle.normalized`; uses two uniforms).
pub fn random_unit_direction(rng: &mut Rng) -> Vec2 {
    let angle = rng.f32() * crate::math::TAU;
    Vec2::from_angle(angle)
}

/// One zone's production/recycling result for a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ZoneUpdateReport {
    /// Pellets spawned this tick (all zones).
    pub spawned: u32,
    /// Pellets fully destroyed by over-cap recycling.
    pub recycled: u32,
    /// Zones whose production loop hit the defensive iteration cap.
    pub capped: bool,
    /// Energy added by zone pellet production this tick (plant).
    pub produced_plant_energy: f64,
    /// Energy added by zone pellet production this tick (meat).
    pub produced_meat_energy: f64,
    /// Energy removed by over-cap recycling this tick (plant).
    pub recycled_plant_energy: f64,
    /// Energy removed by over-cap recycling this tick (meat).
    pub recycled_meat_energy: f64,
}

/// Result of an initial seeding pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeedingReport {
    /// Pellets spawned by seeding.
    pub spawned: u32,
    /// `true` if at least one zone hit [`MAX_SEEDING_ITERATIONS_PER_ZONE`]
    /// (would have hung in the game's red-death edge case).
    pub capped: bool,
    /// Energy seeded into plant pellets.
    pub seeded_plant_energy: f64,
    /// Energy seeded into meat pellets.
    pub seeded_meat_energy: f64,
}

/// Owns all zones in save/processing order (`ZoneManager.zones`).
///
/// Order is semantic: it is the save order, the per-tick processing order and
/// the zone-id assignment order (`UpdateAllZonesList` renumbers ids 0..n).
#[derive(Debug)]
pub struct ZoneStore {
    /// Zones, in scenario/processing order.
    pub zones: Vec<ZoneData>,
    /// World bounds (simulation size, shade constants).
    pub bounds: WorldBounds,
    /// TEMP world-scope settings (see [`crate::zone::ZoneGlobals`]).
    pub globals: ZoneGlobals,
    /// Seed for per-zone and group RNG streams.
    world_seed: u64,
    /// Generation used in `ZoneId` handles; bumped on rebuild/removal so stale
    /// handles are rejected.
    generation: u32,
}

impl ZoneStore {
    /// Creates an empty zone store.
    pub fn new(bounds: WorldBounds, globals: ZoneGlobals, world_seed: u64) -> Self {
        Self {
            zones: Vec::new(),
            bounds,
            globals,
            world_seed,
            generation: 0,
        }
    }

    /// Creates a store from explicit zone configs, assigning zone ids in list
    /// order (spec §1.8).
    pub fn from_configs(
        configs: Vec<ZoneConfig>,
        bounds: WorldBounds,
        globals: ZoneGlobals,
        world_seed: u64,
    ) -> Self {
        let mut store = Self::new(bounds, globals, world_seed);
        store.rebuild(configs);
        store
    }

    /// Creates a store from explicit zones + zone groups (spec §1.8): explicit zones first, then each group's zones. A group that comes with
    /// its zones (a saved world: `ZoneGroupSettings.zones`) keeps them; one that does not has them generated from its template
    /// (`ZoneGroupSettings.GenerateZones`: `count` zones named `<group>_<i>` at random places).
    pub fn from_scenario(
        explicit: Vec<ZoneConfig>,
        groups: &[ZoneGroupConfig],
        bounds: WorldBounds,
        globals: ZoneGlobals,
        world_seed: u64,
    ) -> Self {
        let mut configs = explicit;
        let mut owners: Vec<Option<u32>> = vec![None; configs.len()];
        for (group_index, group) in groups.iter().enumerate() {
            if !group.zones.is_empty() {
                for zone in &group.zones {
                    configs.push(ZoneConfig(zone.clone()));
                    owners.push(Some(group_index as u32));
                }
                continue;
            }
            let mut rng = Rng::derive(world_seed, domain::WORLD, group_index as u64 + 1);
            let count = group.count(bounds.sim_size());
            for i in 0..count {
                let mut config = ZoneConfig(group.template.clone());
                config.name = format!("{}_{}", group.name, i);
                config.set_runtime_zone_id(0); // renumbered below
                config.randomize_position(&mut rng, bounds.sim_size());
                configs.push(config);
                owners.push(Some(group_index as u32));
            }
        }
        let mut store = Self::from_configs(configs, bounds, globals, world_seed);
        for (zone, owner) in store.zones.iter_mut().zip(owners) {
            zone.group = owner;
        }
        store
    }

    /// How many zones are explicit (`ScenarioSettings.zones`): they come first.
    #[must_use]
    pub fn explicit_count(&self) -> usize {
        self.zones.iter().take_while(|zone| zone.group.is_none()).count()
    }

    /// Where the zones of group `group` start, and how many there are (`ZoneGroupSettings.zones`; none: where they would start).
    #[must_use]
    pub fn group_span(&self, group: u32) -> (usize, usize) {
        let start = self
            .zones
            .iter()
            .position(|zone| zone.group.is_some_and(|owner| owner >= group))
            .unwrap_or(self.zones.len());
        let len = self.zones[start..]
            .iter()
            .take_while(|zone| zone.group == Some(group))
            .count();
        (start, len)
    }

    /// Replaces the zones at `range` by `configs`, which become zones of group `group` (`None`: explicit ones), and renumbers the list as
    /// `ScenarioSettings.UpdateAllZonesList` does: every zone's id is its position, a follower keeps following the same zone (and loses
    /// its target when that zone is one of the removed). Bumps the handle generation. Returns the removed zones and, for each zone of the
    /// old list, its place in the new one (`None` when removed).
    pub fn splice_zones(
        &mut self,
        range: core::ops::Range<usize>,
        configs: Vec<ZoneConfig>,
        group: Option<u32>,
    ) -> (Vec<ZoneData>, Vec<Option<usize>>) {
        let old_ids: Vec<u32> = self
            .zones
            .iter()
            .map(|zone| zone.config.runtime_zone_id())
            .collect();
        let old_len = self.zones.len();
        let added = configs.len();
        let (start, end) = (range.start.min(old_len), range.end.min(old_len).max(range.start.min(old_len)));
        let moved = |old: usize| -> Option<usize> {
            if old < start {
                Some(old)
            } else if old < end {
                None
            } else {
                Some(old - (end - start) + added)
            }
        };
        let mapping: Vec<Option<usize>> = (0..old_len).map(moved).collect();
        self.generation = self.generation.wrapping_add(1);
        let sim_size = self.bounds.sim_size();
        let fresh: Vec<ZoneData> = configs
            .into_iter()
            .enumerate()
            .map(|(offset, mut config)| {
                config.cap_position(sim_size);
                let mut zone = ZoneData::new(
                    config,
                    start + offset,
                    self.generation,
                    &self.bounds,
                    &self.globals,
                    self.world_seed,
                );
                zone.group = group;
                zone
            })
            .collect();
        let removed: Vec<ZoneData> = self.zones.splice(start..end, fresh).collect();
        for (index, zone) in self.zones.iter_mut().enumerate() {
            zone.config.set_runtime_zone_id(index as u32);
            zone.id = ZoneId::from_handle(Handle::new(index as u32, self.generation));
        }
        // A follower's target is a zone, not a number: it follows the renumbering (a zone that is new has the target its template gave it,
        // which already is a position of the new list).
        for new in mapping.iter().flatten().copied() {
            let target = self.zones[new].config.target;
            self.zones[new].config.target = target.and_then(|target| {
                old_ids
                    .iter()
                    .position(|id| *id == target)
                    .and_then(|position| mapping[position])
                    .map(|position| position as u32)
            });
        }
        self.resolve_all_targets();
        (removed, mapping)
    }

    /// Rebuilds the zone list from configs. Bumps the handle generation, so all
    /// previously returned [`ZoneId`]s become stale.
    pub fn rebuild(&mut self, configs: Vec<ZoneConfig>) {
        self.generation = self.generation.wrapping_add(1);
        self.zones.clear();
        self.zones.reserve(configs.len());
        for (index, mut config) in configs.into_iter().enumerate() {
            config.set_runtime_zone_id(index as u32);
            config.cap_position(self.bounds.sim_size());
            let mut zone = ZoneData::new(
                config,
                index,
                self.generation,
                &self.bounds,
                &self.globals,
                self.world_seed,
            );
            zone.target_index = self.resolve_target(&zone.config, index);
            self.zones.push(zone);
        }
        // Second pass: targets may point forward in the list.
        for index in 0..self.zones.len() {
            let target = self.resolve_target(&self.zones[index].config, index);
            self.zones[index].target_index = target;
        }
    }

    fn resolve_target(&self, config: &ZoneConfig, own_index: usize) -> Option<usize> {
        if config.movement != MovementType::Attached {
            return None;
        }
        let target_id = config.target?;
        let found = self
            .zones
            .iter()
            .position(|zone| zone.config.runtime_zone_id() == target_id)?;
        if found == own_index {
            None
        } else {
            Some(found)
        }
    }

    /// Number of zones.
    #[inline]
    pub fn len(&self) -> usize {
        self.zones.len()
    }

    /// `ZoneManager.LogAllZonesData`: every zone logs a point.
    pub fn log_all(&mut self) {
        let sim_size = self.bounds.sim_size();
        for zone in &mut self.zones {
            let point = zone.log_point(sim_size);
            zone.log.push(point);
        }
    }

    /// `true` if there are no zones.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Zones in processing order.
    #[inline]
    pub fn zones(&self) -> &[ZoneData] {
        &self.zones
    }

    /// Mutable zones in processing order.
    #[inline]
    pub fn zones_mut(&mut self) -> &mut [ZoneData] {
        &mut self.zones
    }

    /// Runtime handle for the zone at `index` (`slot = index, generation`).
    #[inline]
    pub fn zone_id_at(&self, index: usize) -> Option<ZoneId> {
        self.zones
            .get(index)
            .map(|_| ZoneId::from_handle(Handle::new(index as u32, self.generation)))
    }

    /// Index of a live `ZoneId`.
    #[inline]
    pub fn index_of(&self, id: ZoneId) -> Option<usize> {
        let handle = id.handle();
        let index = handle.slot() as usize;
        if handle.generation() == self.generation && index < self.zones.len() {
            Some(index)
        } else {
            None
        }
    }

    /// Zone by runtime handle.
    #[inline]
    pub fn get(&self, id: ZoneId) -> Option<&ZoneData> {
        self.index_of(id).map(|index| &self.zones[index])
    }

    /// Mutable zone by runtime handle.
    #[inline]
    pub fn get_mut(&mut self, id: ZoneId) -> Option<&mut ZoneData> {
        self.index_of(id).map(|index| &mut self.zones[index])
    }

    /// Appends a zone (zone id = current length) and returns its handle.
    pub fn add_zone(&mut self, mut config: ZoneConfig) -> ZoneId {
        let index = self.zones.len();
        config.set_runtime_zone_id(index as u32);
        config.cap_position(self.bounds.sim_size());
        let mut zone = ZoneData::new(
            config,
            index,
            self.generation,
            &self.bounds,
            &self.globals,
            self.world_seed,
        );
        zone.target_index = self.resolve_target(&zone.config, index);
        self.zones.push(zone);
        ZoneId::from_handle(Handle::new(index as u32, self.generation))
    }

    /// Removes a zone and renumbers the remaining zones (mirrors
    /// `ZoneRemoved` + `UpdateAllZonesList`). Bumps the handle generation. A zone's `target` is a
    /// reference to another zone object in the game, so the followers keep following the same
    /// zone through the renumbering and lose their target when it is the removed one.
    pub fn remove_zone(&mut self, id: ZoneId) -> Option<ZoneData> {
        let index = self.index_of(id)?;
        let (mut removed, _) = self.splice_zones(index..index + 1, Vec::new(), None);
        removed.pop()
    }

    /// Puts zone `index` at `pos` (`Zone.UpdatePosition`, run when its posX / posY settings
    /// change) and carries along every zone attached to it, directly or through other followers,
    /// as the game's transform parenting does. The followers' own settings follow their new
    /// places; the zone's own settings are the ones that asked for the move and stay as given.
    pub fn teleport(&mut self, index: usize, pos: Vec2) {
        let sim_size = self.bounds.sim_size();
        let Some(zone) = self.zones.get(index) else {
            return;
        };
        let delta = pos - zone.pos;
        let mut visited = vec![false; self.zones.len()];
        let mut pending = vec![index];
        while let Some(at) = pending.pop() {
            if std::mem::replace(&mut visited[at], true) {
                continue;
            }
            let zone = &mut self.zones[at];
            zone.pos += delta;
            if at != index {
                zone.config.pos_x = zone.pos.x / sim_size;
                zone.config.pos_y = zone.pos.y / sim_size;
            }
            pending.extend(
                self.zones
                    .iter()
                    .enumerate()
                    .filter(|(other, follower)| {
                        !visited[*other] && follower.target_index == Some(at)
                    })
                    .map(|(other, _)| other),
            );
        }
    }

    /// Re-resolves every follower's target after zone settings (movement, target, ids) changed
    /// (`Zone.UpdateTarget` re-parents the zone when its target setting changes).
    pub fn resolve_all_targets(&mut self) {
        for i in 0..self.zones.len() {
            let target = self.resolve_target(&self.zones[i].config, i);
            self.zones[i].target_index = target;
        }
    }

    /// First zone (in list order) whose shape contains `pos`. Used by
    /// `zone_of` hooks (void avoidance / environmental effects) `[I]`:
    /// overlapping zones resolve to the earliest in processing order.
    pub fn zone_at(&self, pos: Vec2) -> Option<ZoneId> {
        self.zones
            .iter()
            .position(|zone| zone.contains(pos))
            .and_then(|index| self.zone_id_at(index))
    }

    /// Runs the movement phase for all zones (`Zone.FixedUpdate`→`Move`).
    ///
    /// Done in list order; `Attached` zones follow their target's movement this
    /// tick when the target appears earlier in the list, otherwise they use the
    /// target's previous-tick delta. (The game parents transforms, which makes
    /// the effective order Unity-hierarchy dependent; this is the documented
    /// deterministic equivalent, spec §1.7 `[I]`.)
    pub fn update_movement(&mut self, dt: f32) {
        for index in 0..self.zones.len() {
            let target_delta = self.zones[index]
                .target_index
                .map(|target| self.zones[target].last_delta)
                .unwrap_or(Vec2::ZERO);
            self.zones[index].last_delta = target_delta;
            self.zones[index].move_zone(dt, &self.bounds, &self.globals);
        }
    }

    /// Runs the per-tick zone production/recycling/recount phase (spec §1.4,
    /// §1.6) and syncs the spatial hash for spawned/recycled pellets.
    ///
    /// `red_death_safe_radius`: when red death is active, `Zone.SpawnAPellet`
    /// refuses to create pellets outside the safe radius (it still returns the
    /// would-be energy, which the game subtracts from the progress; the pellet
    /// is simply not created). Pass `None` when red death is disabled.
    pub fn update_phase(
        &mut self,
        dt: f32,
        pellets: &mut PelletStore,
        hash: &mut SpatialHash,
        red_death_safe_radius: Option<f32>,
    ) -> ZoneUpdateReport {
        let mut report = ZoneUpdateReport::default();
        for index in 0..self.zones.len() {
            let spawn = self.zones[index].update_production(
                dt,
                &self.globals,
                &self.bounds,
                pellets,
                hash,
                red_death_safe_radius,
            );
            report.spawned += spawn.spawned;
            report.recycled += spawn.recycled;
            report.capped |= spawn.capped;
            report.produced_plant_energy += spawn.produced_plant_energy;
            report.produced_meat_energy += spawn.produced_meat_energy;
            report.recycled_plant_energy += spawn.recycled_plant_energy;
            report.recycled_meat_energy += spawn.recycled_meat_energy;
            // Recount when due (Zone.UpdateBiomassCount coroutine).
            let zone = &mut self.zones[index];
            zone.time_since_recount += dt;
            if zone.time_since_recount >= RECOUNT_PERIOD {
                zone.recount(pellets);
            }
        }
        report
    }

    /// Initial plant seeding for every zone (spec §8.2):
    /// `while pellet_biomass < initialSeeding · maxBiomass: SpawnAPellet()`.
    pub fn initial_seeding(
        &mut self,
        pellets: &mut PelletStore,
        hash: &mut SpatialHash,
        red_death_safe_radius: Option<f32>,
    ) -> SeedingReport {
        let mut report = SeedingReport::default();
        for index in 0..self.zones.len() {
            let target = self.globals.initial_seeding() * self.zones[index].max_biomass;
            let mut iterations: u32 = 0;
            while self.zones[index].pellet_biomass < target {
                if iterations >= MAX_SEEDING_ITERATIONS_PER_ZONE {
                    report.capped = true;
                    break;
                }
                iterations += 1;
                let outcome = self.zones[index].spawn_ap_pellet(
                    &self.globals,
                    &self.bounds,
                    pellets,
                    hash,
                    red_death_safe_radius,
                );
                if !outcome.created {
                    // No progress is possible (e.g. every spawn point is in the
                    // red death); the game would loop forever here (spec Q6).
                    report.capped = true;
                    break;
                }
                report.spawned += 1;
                if self.zones[index].config.material == Some(crate::settings::MaterialKind::Meat) {
                    report.seeded_meat_energy += f64::from(outcome.energy);
                } else {
                    report.seeded_plant_energy += f64::from(outcome.energy);
                }
            }
        }
        report
    }

    /// Recounts every zone's biomass from the pellet store (load/teleport path).
    pub fn recount_all(&mut self, pellets: &PelletStore) {
        for zone in &mut self.zones {
            zone.recount(pellets);
        }
    }

    /// Sum of `maxBiomass` over all zones.
    pub fn total_max_biomass(&self) -> f32 {
        self.zones.iter().map(|zone| zone.max_biomass).sum()
    }

    /// Sum of zone free biomass (`ZoneManager.freeBiomass`).
    pub fn free_biomass(&self) -> f32 {
        self.zones.iter().map(ZoneData::free_biomass).sum()
    }

    /// Sum of zone production in `E/s` (`ScenarioSettings.TotalGrowth`).
    pub fn total_growth(&self) -> f32 {
        self.zones.iter().map(|zone| zone.total_growth).sum()
    }

    /// Drops a removed pellet from a zone's owned list (O(n) lookup).
    pub fn forget_pellet(&mut self, zone: ZoneId, pellet: PelletId) {
        if let Some(zone) = self.get_mut(zone) {
            zone.forget_pellet(pellet);
        }
    }
}

/// Per-zone production result.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SpawnOutcome {
    spawned: u32,
    recycled: u32,
    capped: bool,
    produced_plant_energy: f64,
    produced_meat_energy: f64,
    recycled_plant_energy: f64,
    recycled_meat_energy: f64,
}

/// One pellet spawn attempt result.
#[derive(Clone, Copy, Debug)]
struct PelletSpawnOutcome {
    energy: f32,
    created: bool,
}

impl ZoneData {
    /// One zone's `Zone.FixedUpdate` production/recycle logic plus the recount
    /// accumulator (spec §1.4/§1.6).
    fn update_production(
        &mut self,
        dt: f32,
        globals: &ZoneGlobals,
        bounds: &WorldBounds,
        pellets: &mut PelletStore,
        hash: &mut SpatialHash,
        red_death_safe_radius: Option<f32>,
    ) -> SpawnOutcome {
        let mut outcome = SpawnOutcome::default();

        if self.pellet_biomass > self.max_biomass {
            let overage = self.pellet_biomass - self.max_biomass;
            let (recycled, remaining, recycled_plant, recycled_meat) =
                self.recycle_pellets(overage, pellets, hash);
            outcome.recycled = recycled;
            outcome.recycled_plant_energy = recycled_plant;
            outcome.recycled_meat_energy = recycled_meat;
            self.pellet_biomass -= overage - remaining;
        } else if (self.config.renew_biomass || self.pellet_biomass < self.max_biomass)
            && self.config.material.is_some()
        {
            self.pellet_progress += self.total_growth * dt;
            if !self.config.renew_biomass {
                self.pellet_progress = self.pellet_progress.min(self.free_biomass().max(0.0));
            }
            let threshold = SPAWN_THRESHOLD_FACTOR * self.avg_pellet_energy;
            let mut iterations = 0;
            while self.pellet_progress > threshold {
                if iterations >= MAX_SPAWN_ITERATIONS_PER_TICK {
                    outcome.capped = true;
                    break;
                }
                iterations += 1;
                let spawned =
                    self.spawn_ap_pellet(globals, bounds, pellets, hash, red_death_safe_radius);
                self.pellet_progress -= spawned.energy;
                if !spawned.created {
                    // Red-death refusal: energy is consumed (matching the game)
                    // but no pellet exists; progress keeps decreasing so the loop
                    // terminates as long as energy > 0.
                    if spawned.energy <= 0.0 {
                        break;
                    }
                } else {
                    outcome.spawned += 1;
                    if self.config.material == Some(crate::settings::MaterialKind::Meat) {
                        outcome.produced_meat_energy += f64::from(spawned.energy);
                    } else {
                        outcome.produced_plant_energy += f64::from(spawned.energy);
                    }
                }
            }
        }
        outcome
    }

    /// `Zone.SpawnAPellet`: picks a location, rolls the pellet energy and creates
    /// the pellet (unless it would be outside the red-death safe radius).
    fn spawn_ap_pellet(
        &mut self,
        globals: &ZoneGlobals,
        bounds: &WorldBounds,
        pellets: &mut PelletStore,
        hash: &mut SpatialHash,
        red_death_safe_radius: Option<f32>,
    ) -> PelletSpawnOutcome {
        let _ = bounds;
        let material = match self.config.material {
            Some(material) => material_from_settings(material),
            None => {
                return PelletSpawnOutcome {
                    energy: 0.0,
                    created: false,
                };
            }
        };
        let pos = self.spawn_location();
        let energy = self.roll_pellet_energy(globals);
        let outside_red_death =
            red_death_safe_radius.is_some_and(|safe_radius| pos.length() > safe_radius);
        if outside_red_death {
            return PelletSpawnOutcome {
                energy,
                created: false,
            };
        }
        let energy_density = pellets.materials().get(material).energy_density;
        let amount = energy / energy_density;
        let spawn = PelletSpawn {
            material,
            amount,
            pos,
            velocity: Vec2::ZERO,
            rotation: 0.0,
            angular_velocity: 0.0,
            owner_zone: Some(self.id),
            held: false,
        };
        match pellets.try_spawn(spawn) {
            Some(pellet) => {
                self.owned_pellets.push(pellet);
                self.pellet_biomass += energy;
                if let Some(radius) = pellets.radius(pellet) {
                    hash.insert(
                        pellet.handle(),
                        crate::world::SpatialClass::Pellet,
                        pos,
                        radius,
                    );
                }
                PelletSpawnOutcome {
                    energy,
                    created: true,
                }
            }
            None => PelletSpawnOutcome {
                energy: 0.0,
                created: false,
            },
        }
    }

    /// Rolls a spawned pellet energy:
    /// `max(avgPelletEnergy, pelletEnergy.min) · clamp(1.5^N(0,1), 0.1, 5)`.
    pub fn roll_pellet_energy(&mut self, globals: &ZoneGlobals) -> f32 {
        let base = self.avg_pellet_energy.max(globals.pellet_energy_min());
        let factor = clamp(
            PELLET_SIZE_POWER_BASE.powf(self.rng.normal()),
            PELLET_SIZE_MIN_FACTOR,
            PELLET_SIZE_MAX_FACTOR,
        );
        base * factor
    }

    /// Recycles pellets from the front of the owned list until `overage` energy
    /// has been removed (`Zone.RecyclePellets`). Returns
    /// `(fully_recycled_pellets, leftover_overage)`.
    fn recycle_pellets(
        &mut self,
        overage: f32,
        pellets: &mut PelletStore,
        hash: &mut SpatialHash,
    ) -> (u32, f32, f64, f64) {
        let mut remaining = overage.max(0.0);
        let mut recycled = 0;
        let mut recycled_plant = 0.0f64;
        let mut recycled_meat = 0.0f64;
        let mut index = 0;
        while remaining > 0.0 && index < self.owned_pellets.len() {
            let pellet = self.owned_pellets[index];
            if !pellets.contains(pellet) {
                self.owned_pellets.remove(index);
                continue;
            }
            let energy = pellets.energy(pellet).unwrap_or(0.0);
            let material = pellets.material(pellet);
            // `RecyclePellets`: RemoveEnergy(min(amount, energy + 1)).
            let removed = pellets.remove_energy(pellet, remaining.min(energy + 1.0));
            match material {
                Some(crate::pellet::MaterialKind::Meat) => {
                    recycled_meat += f64::from(removed);
                }
                _ => recycled_plant += f64::from(removed),
            }
            remaining -= removed;
            if pellets.contains(pellet) {
                // Partially eaten pellet: its radius shrank, refresh the hash.
                if let (Some(pos), Some(radius)) = (pellets.pos(pellet), pellets.radius(pellet)) {
                    hash.update(
                        pellet.handle(),
                        crate::world::SpatialClass::Pellet,
                        pos,
                        radius,
                    );
                }
                index += 1;
            } else {
                hash.remove(pellet.handle(), crate::world::SpatialClass::Pellet);
                self.owned_pellets.remove(index);
                recycled += 1;
            }
        }
        (recycled, remaining, recycled_plant, recycled_meat)
    }
}

// ---------------------------------------------------------------------------
// Checkpoint P2: zone runtime state (task-103)
// ---------------------------------------------------------------------------

/// Runtime-only zone state needed for a bit-identical checkpoint resume.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneRuntimeCheckpoint {
    /// Per-zone RNG position.
    pub rng: crate::rng::RngState,
    /// World-space centre.
    pub pos: Vec2,
    /// Movement velocity.
    pub vel: Vec2,
    /// Last movement delta (Attached followers).
    pub last_delta: Vec2,
    /// Absolute shape radius.
    pub radius: f32,
    /// Area.
    pub area: f32,
    /// Ring inner radius.
    pub inside_radius_abs: f32,
    /// Rect half extents.
    pub half_extents: Vec2,
    /// Cached max biomass.
    pub max_biomass: f32,
    /// Cached total growth.
    pub total_growth: f32,
    /// Cached average pellet energy.
    pub avg_pellet_energy: f32,
    /// Rect flag.
    pub is_rect: bool,
    /// Ring flag.
    pub is_ring: bool,
    /// Pending production.
    pub pellet_progress: f32,
    /// Lagging zone pellet biomass.
    pub pellet_biomass: f32,
    /// Owned pellet ids in recycle order.
    pub owned_pellets: Vec<PelletId>,
    /// Time since last recount.
    pub time_since_recount: f32,
    /// Resolved Attached target index.
    pub target_index: Option<usize>,
}

impl ZoneData {
    /// Captures this zone's runtime checkpoint state.
    #[must_use]
    pub fn capture_runtime(&self) -> ZoneRuntimeCheckpoint {
        ZoneRuntimeCheckpoint {
            rng: self.rng.state(),
            pos: self.pos,
            vel: self.vel,
            last_delta: self.last_delta,
            radius: self.radius,
            area: self.area,
            inside_radius_abs: self.inside_radius_abs,
            half_extents: self.half_extents,
            max_biomass: self.max_biomass,
            total_growth: self.total_growth,
            avg_pellet_energy: self.avg_pellet_energy,
            is_rect: self.is_rect,
            is_ring: self.is_ring,
            pellet_progress: self.pellet_progress,
            pellet_biomass: self.pellet_biomass,
            owned_pellets: self.owned_pellets.clone(),
            time_since_recount: self.time_since_recount,
            target_index: self.target_index,
        }
    }

    /// Applies runtime checkpoint state (definitions must already be loaded).
    pub fn restore_runtime(
        &mut self,
        checkpoint: &ZoneRuntimeCheckpoint,
    ) -> crate::error::Result<()> {
        if !self.rng.set_state(checkpoint.rng) {
            return Err(crate::error::Error::invalid_data(
                "invalid zone checkpoint RNG state",
            ));
        }
        self.pos = checkpoint.pos;
        self.vel = checkpoint.vel;
        self.last_delta = checkpoint.last_delta;
        self.radius = checkpoint.radius;
        self.area = checkpoint.area;
        self.inside_radius_abs = checkpoint.inside_radius_abs;
        self.half_extents = checkpoint.half_extents;
        self.max_biomass = checkpoint.max_biomass;
        self.total_growth = checkpoint.total_growth;
        self.avg_pellet_energy = checkpoint.avg_pellet_energy;
        self.is_rect = checkpoint.is_rect;
        self.is_ring = checkpoint.is_ring;
        self.pellet_progress = checkpoint.pellet_progress;
        self.pellet_biomass = checkpoint.pellet_biomass;
        self.owned_pellets.clone_from(&checkpoint.owned_pellets);
        self.time_since_recount = checkpoint.time_since_recount;
        self.target_index = checkpoint.target_index;
        Ok(())
    }
}

impl ZoneStore {
    /// Captures every zone's runtime checkpoint state in zone order.
    #[must_use]
    pub fn capture_runtime(&self) -> Vec<ZoneRuntimeCheckpoint> {
        self.zones.iter().map(ZoneData::capture_runtime).collect()
    }

    /// Restores per-zone runtime state; zone count must match.
    pub fn restore_runtime(
        &mut self,
        checkpoints: &[ZoneRuntimeCheckpoint],
    ) -> crate::error::Result<()> {
        if checkpoints.len() != self.zones.len() {
            return Err(crate::error::Error::invalid_data(format!(
                "zone checkpoint has {} zones, world has {}",
                checkpoints.len(),
                self.zones.len()
            )));
        }
        for (zone, checkpoint) in self.zones.iter_mut().zip(checkpoints) {
            zone.restore_runtime(checkpoint)?;
        }
        Ok(())
    }

    /// Encodes runtime state (little-endian, length-prefixed).
    #[must_use]
    pub fn encode_runtime(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_u32(&mut out, self.zones.len() as u32);
        for checkpoint in self.capture_runtime() {
            put_u64(&mut out, checkpoint.rng.seed);
            for word in checkpoint.rng.words {
                put_u64(&mut out, word);
            }
            put_vec2(&mut out, checkpoint.pos);
            put_vec2(&mut out, checkpoint.vel);
            put_vec2(&mut out, checkpoint.last_delta);
            put_f32(&mut out, checkpoint.radius);
            put_f32(&mut out, checkpoint.area);
            put_f32(&mut out, checkpoint.inside_radius_abs);
            put_vec2(&mut out, checkpoint.half_extents);
            put_f32(&mut out, checkpoint.max_biomass);
            put_f32(&mut out, checkpoint.total_growth);
            put_f32(&mut out, checkpoint.avg_pellet_energy);
            out.push(u8::from(checkpoint.is_rect));
            out.push(u8::from(checkpoint.is_ring));
            put_f32(&mut out, checkpoint.pellet_progress);
            put_f32(&mut out, checkpoint.pellet_biomass);
            put_u32(&mut out, checkpoint.owned_pellets.len() as u32);
            for pellet in checkpoint.owned_pellets {
                put_u32(&mut out, pellet.handle().slot());
                put_u32(&mut out, pellet.handle().generation());
            }
            put_f32(&mut out, checkpoint.time_since_recount);
            match checkpoint.target_index {
                Some(index) => {
                    out.push(1);
                    put_u32(&mut out, index as u32);
                }
                None => out.push(0),
            }
        }
        out
    }

    /// The zones' logs for a checkpoint: the count, then for each zone in order the length and the bytes of
    /// [`crate::zone_log::ZoneLog::save_state_bin`] (the points are stored as they are, so nothing is lost).
    #[must_use]
    pub fn encode_logs(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_u32(&mut out, self.zones.len() as u32);
        for zone in &self.zones {
            let bytes = zone.log.save_state_bin();
            put_u32(&mut out, bytes.len() as u32);
            out.extend_from_slice(&bytes);
        }
        out
    }

    /// Restores what [`ZoneStore::encode_logs`] wrote; the zone count must match.
    pub fn restore_logs(&mut self, bytes: &[u8]) -> crate::error::Result<()> {
        let mut reader = ZoneSliceReader { bytes, pos: 0 };
        let count = reader.u32()? as usize;
        if count != self.zones.len() {
            return Err(crate::error::Error::invalid_data(format!(
                "zone log checkpoint has {count} zones, world has {}",
                self.zones.len()
            )));
        }
        let mut logs = Vec::with_capacity(count);
        for _ in 0..count {
            let length = reader.u32()? as usize;
            logs.push(crate::zone_log::ZoneLog::load_state_bin(
                reader.take(length)?,
            )?);
        }
        for (zone, log) in self.zones.iter_mut().zip(logs) {
            zone.log = log;
        }
        Ok(())
    }

    /// Decodes state produced by [`ZoneStore::encode_runtime`].
    pub fn decode_runtime(bytes: &[u8]) -> crate::error::Result<Vec<ZoneRuntimeCheckpoint>> {
        let mut reader = ZoneSliceReader { bytes, pos: 0 };
        let count = reader.u32()? as usize;
        let mut checkpoints = Vec::with_capacity(count);
        for _ in 0..count {
            let seed = reader.u64()?;
            let words = [reader.u64()?, reader.u64()?, reader.u64()?, reader.u64()?];
            let rng = crate::rng::RngState::new(seed, words);
            let pos = reader.vec2()?;
            let vel = reader.vec2()?;
            let last_delta = reader.vec2()?;
            let radius = reader.f32()?;
            let area = reader.f32()?;
            let inside_radius_abs = reader.f32()?;
            let half_extents = reader.vec2()?;
            let max_biomass = reader.f32()?;
            let total_growth = reader.f32()?;
            let avg_pellet_energy = reader.f32()?;
            let is_rect = reader.bool()?;
            let is_ring = reader.bool()?;
            let pellet_progress = reader.f32()?;
            let pellet_biomass = reader.f32()?;
            let n_owned = reader.u32()? as usize;
            let mut owned_pellets = Vec::with_capacity(n_owned);
            for _ in 0..n_owned {
                let slot = reader.u32()?;
                let generation = reader.u32()?;
                owned_pellets.push(PelletId::from_handle(Handle::new(slot, generation)));
            }
            let time_since_recount = reader.f32()?;
            let target_index = if reader.u8()? == 1 {
                Some(reader.u32()? as usize)
            } else {
                None
            };
            checkpoints.push(ZoneRuntimeCheckpoint {
                rng,
                pos,
                vel,
                last_delta,
                radius,
                area,
                inside_radius_abs,
                half_extents,
                max_biomass,
                total_growth,
                avg_pellet_energy,
                is_rect,
                is_ring,
                pellet_progress,
                pellet_biomass,
                owned_pellets,
                time_since_recount,
                target_index,
            });
        }
        Ok(checkpoints)
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_f32(out: &mut Vec<u8>, value: f32) {
    put_u32(out, value.to_bits());
}
fn put_vec2(out: &mut Vec<u8>, value: Vec2) {
    put_f32(out, value.x);
    put_f32(out, value.y);
}

struct ZoneSliceReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl ZoneSliceReader<'_> {
    fn take(&mut self, len: usize) -> crate::error::Result<&[u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| crate::error::Error::invalid_data("zone checkpoint length overflow"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| crate::error::Error::invalid_data("truncated zone checkpoint"))?;
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
                "invalid bool in zone checkpoint",
            )),
        }
    }
    fn u32(&mut self) -> crate::error::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn u64(&mut self) -> crate::error::Result<u64> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
    fn f32(&mut self) -> crate::error::Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn vec2(&mut self) -> crate::error::Result<Vec2> {
        Ok(Vec2::new(self.f32()?, self.f32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::PI;
    use crate::world::WorldBounds;

    fn bounds(sim_size: f32) -> WorldBounds {
        WorldBounds::new(sim_size)
    }

    fn default_globals() -> ZoneGlobals {
        ZoneGlobals::game_defaults()
    }

    fn plant_zone(radius_relative: f32) -> ZoneConfig {
        zone_with(ZoneConfig::default_zone(), |zone| {
            zone.radius_relative = radius_relative;
        })
    }

    fn zone_with(
        mut zone: ZoneConfig,
        apply: impl FnOnce(&mut crate::settings::ZoneSettings),
    ) -> ZoneConfig {
        apply(&mut zone.0);
        zone
    }

    fn globals_with(
        mut globals: ZoneGlobals,
        apply: impl FnOnce(&mut crate::settings::SimSettings),
    ) -> ZoneGlobals {
        apply(&mut globals.0);
        globals
    }

    fn almost(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3_f32.max(b.abs() * 1e-5)
    }

    fn store_with(zone: ZoneConfig) -> (ZoneStore, PelletStore) {
        let store = ZoneStore::from_configs(vec![zone], bounds(2000.0), default_globals(), 42);
        (store, PelletStore::new())
    }

    fn hash_for(store: &ZoneStore) -> SpatialHash {
        let extent = store.bounds.shade_end() + 512.0;
        SpatialHash::new(
            Vec2::splat(-extent),
            Vec2::splat(extent),
            (store.bounds.sim_size() / 128.0).max(32.0),
        )
    }

    #[test]
    fn default_zip_appendix_a_numbers() {
        // Appendix A: one Plant zone radiusRelative 1.0, fertility 0.7827918,
        // biomassDensity 1, pelletSize 1; globals from Default.zip recommended.
        let globals = globals_with(ZoneGlobals::game_defaults(), |settings| {
            settings.independents.biomass_density = 0.01; // recommended.biomassDensity
            settings.independents.pellet_growth = 1e-5; // recommended.pelletGrowth
        });
        let sim_size = 2000.0;
        let config = zone_with(ZoneConfig::default_zone(), |zone| {
            zone.name = "World".to_owned();
            zone.radius_relative = 1.0;
            zone.fertility = 0.7827918;
            zone.biomass_density = 1.0;
            zone.pellet_size = 1.0;
            zone.distribution = SpawnDistribution::CentricGradual;
        });
        let area = PI * 2000.0 * 2000.0;
        assert!(almost(config.area(sim_size), area));
        assert!(almost(config.max_biomass(&globals, sim_size), area * 0.01));
        assert!(almost(
            config.total_growth(&globals, sim_size),
            0.7827918 * area * 1e-5
        ));
        assert!(almost(config.avg_pellet_energy(&globals), 200.0));
        assert_eq!(config.estimated_pellets(&globals, sim_size), 629);

        let mut store = ZoneStore::from_configs(vec![config], bounds(sim_size), globals.clone(), 7);
        let mut pellets = PelletStore::new();
        let mut hash = hash_for(&store);
        let report = store.initial_seeding(&mut pellets, &mut hash, None);
        assert!(!report.capped);
        let target = globals.initial_seeding() * store.zones[0].max_biomass;
        assert!(store.zones[0].pellet_biomass >= target);
        assert!(store.zones[0].pellet_biomass < target + 5.0 * 200.0 + 1.0);
        assert_eq!(
            report.spawned as usize,
            pellets.counts().plant_count as usize
        );
        assert!(report.seeded_plant_energy > 0.0);
        assert!(almost(
            report.seeded_plant_energy as f32,
            store.zones[0].pellet_biomass
        ));
        // ~434-471 pellets at ~200 E nominal (mean multiplier 1.086).
        assert!(
            report.spawned > 380 && report.spawned < 520,
            "seeded {} pellets",
            report.spawned
        );
    }

    #[test]
    fn spawn_threshold_is_one_and_a_half_average() {
        let mut zone = plant_zone(1.0);
        zone.fertility = 1.0;
        zone.biomass_density = 1.0;
        let globals = ZoneGlobals::game_defaults();
        let mut store = ZoneStore::from_configs(vec![zone], bounds(2000.0), globals.clone(), 3);
        let mut pellets = PelletStore::new();
        let mut hash = hash_for(&store);
        let zone = &mut store.zones[0];
        assert!(almost(zone.total_growth, PI * 4_000_000.0 * 1e-5));
        let dt = 1.0 / 30.0;
        let threshold = SPAWN_THRESHOLD_FACTOR * zone.avg_pellet_energy;
        let per_tick = zone.total_growth * dt;
        let mut ticks = 0u32;
        let mut spawned = 0u32;
        let mut produced_energy = 0.0f64;
        while spawned == 0 {
            ticks += 1;
            assert!(ticks < 10_000, "no pellet spawned");
            let outcome =
                zone.update_production(dt, &globals, &store.bounds, &mut pellets, &mut hash, None);
            spawned += outcome.spawned;
            produced_energy += outcome.produced_plant_energy;
        }
        assert_eq!(spawned, 1);
        assert!(produced_energy > 0.0);
        // The spawn happens exactly on the tick where accumulated production
        // first exceeds 1.5 × avgPelletEnergy.
        assert!(ticks as f32 * per_tick > threshold);
        assert!((ticks - 1) as f32 * per_tick <= threshold);

        let energy = pellets
            .ids()
            .map(|id| pellets.energy(id).unwrap())
            .sum::<f32>();
        assert!(energy >= 0.1 * zone.avg_pellet_energy.max(globals.pellet_energy_min()));
        assert!(
            energy
                <= PELLET_SIZE_MAX_FACTOR * zone.avg_pellet_energy.max(globals.pellet_energy_min())
        );
        assert_eq!(pellets.counts().plant_count, 1);
        assert_eq!(zone.owned_pellets.len(), 1);
    }

    #[test]
    fn pellet_energy_multiplier_is_clamped_lognormal() {
        let mut store = store_with(plant_zone(0.25)).0;
        let zone = &mut store.zones[0];
        let globals = default_globals();
        let mut sum = 0.0;
        let n = 20_000;
        for _ in 0..n {
            let energy = zone.roll_pellet_energy(&globals);
            assert!(energy >= 0.1 * 200.0 - 1e-4);
            assert!(energy <= 5.0 * 200.0 + 1e-4);
            sum += energy;
        }
        let mean = sum / n as f32;
        // Mean of clamp(1.5^N(0,1), 0.1, 5) ~ 1.086 -> ~217 E.
        assert!(mean > 205.0 && mean < 230.0, "mean {mean}");
    }

    #[test]
    fn all_distributions_spawn_inside_their_regions() {
        let sim_size = 2000.0;
        let globals = default_globals();
        for distribution in SpawnDistribution::ALL {
            let config = zone_with(ZoneConfig::default_zone(), |zone| {
                zone.distribution = distribution;
                zone.radius_relative = 0.25;
                zone.inside_radius = 0.5;
                zone.width_relative = 0.5;
                zone.height_relative = 0.25;
            });
            let mut store =
                ZoneStore::from_configs(vec![config], bounds(sim_size), globals.clone(), 11);
            let zone = &mut store.zones[0];
            for _ in 0..2_000 {
                let pos = zone.spawn_location();
                let delta = pos - zone.pos;
                match distribution {
                    SpawnDistribution::Flat
                    | SpawnDistribution::CentricGradual
                    | SpawnDistribution::ExteriorGradual => {
                        assert!(
                            delta.length() <= zone.radius + 1e-3,
                            "{distribution:?} radius {}",
                            delta.length()
                        );
                    }
                    SpawnDistribution::Ring | SpawnDistribution::FlatRing => {
                        assert!(
                            delta.length() >= zone.inside_radius_abs - 1e-3,
                            "{distribution:?} below inside radius"
                        );
                        assert!(
                            delta.length() <= zone.radius + 1e-3,
                            "{distribution:?} above outer radius"
                        );
                    }
                    SpawnDistribution::Rect => {
                        assert!(delta.x.abs() <= zone.half_extents.x + 1e-3);
                        assert!(delta.y.abs() <= zone.half_extents.y + 1e-3);
                    }
                }
            }
        }
    }

    #[test]
    fn distribution_radial_ordering() {
        let sim_size = 2000.0;
        let globals = default_globals();
        let mean_radius = |distribution: SpawnDistribution| -> f32 {
            let config = zone_with(ZoneConfig::default_zone(), |zone| {
                zone.distribution = distribution;
                zone.radius_relative = 0.25;
                zone.inside_radius = 0.5;
            });
            let mut store =
                ZoneStore::from_configs(vec![config], bounds(sim_size), globals.clone(), 5);
            let zone = &mut store.zones[0];
            let mut sum = 0.0;
            for _ in 0..10_000 {
                sum += (zone.spawn_location() - zone.pos).length();
            }
            sum / 10_000.0
        };
        let centric = mean_radius(SpawnDistribution::CentricGradual);
        let flat = mean_radius(SpawnDistribution::Flat);
        let exterior = mean_radius(SpawnDistribution::ExteriorGradual);
        let radius = 500.0;
        assert!(centric < flat, "centric {centric} flat {flat}");
        assert!(flat < exterior, "flat {flat} exterior {exterior}");
        assert!(centric < radius * 0.8);
        assert!(exterior > radius * 0.8);
    }

    #[test]
    fn over_cap_recycle_removes_from_front_and_accounts() {
        // Small zone with a known cap: area = π·10², density 1, global 0.1.
        let globals = globals_with(default_globals(), |settings| {
            settings.independents.biomass_density = 0.1;
        });
        let config = zone_with(ZoneConfig::default_zone(), |zone| {
            zone.radius_relative = 0.0;
            zone.radius_absolute = 10.0;
            zone.radius_is_relative = false;
        });
        let mut store = ZoneStore::from_configs(vec![config], bounds(100.0), globals.clone(), 9);
        let mut hash = hash_for(&store);
        let zone = &mut store.zones[0];
        assert!(almost(zone.max_biomass, PI * 100.0 * 0.1));
        let cap = zone.max_biomass;
        let mut pellets = PelletStore::new();
        // Spawn 5 pellets of 30 E each (6 u² plant): 150 E > cap 31.4 E.
        for _ in 0..5 {
            let pos = zone.pos + Vec2::new(1.0, 0.0);
            let id = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 6.0, pos));
            let radius = pellets.radius(id).unwrap();
            hash.insert(id.handle(), crate::world::SpatialClass::Pellet, pos, radius);
            zone.owned_pellets.push(id);
            zone.pellet_biomass += 30.0;
        }
        let first = zone.owned_pellets[0];
        let report =
            zone.update_production(0.0, &globals, &store.bounds, &mut pellets, &mut hash, None);
        assert!(report.recycled >= 1);
        assert!(
            !pellets.contains(first),
            "first-created pellet recycled first"
        );
        assert!(zone.pellet_biomass <= cap + 1e-2);
        assert!(zone.pellet_biomass >= cap - 1e-2);
        assert_eq!(
            pellets.counts().plant_count as usize,
            zone.owned_pellets.len()
        );
        assert_eq!(hash.len(), pellets.len());
    }

    #[test]
    fn pellet_biomass_lags_until_ten_second_recount() {
        let mut store = store_with(plant_zone(0.25)).0;
        let mut pellets = PelletStore::new();
        let mut hash = hash_for(&store);
        let zone = &mut store.zones[0];
        let id = pellets.spawn(PelletSpawn::new(MaterialKind::Plant, 20.0, zone.pos));
        zone.owned_pellets.push(id);
        zone.pellet_biomass = 100.0;
        pellets.remove(id); // eaten externally; zone counter is stale (the game lags)
        assert!(zone.pellet_biomass == 100.0);

        // Advance < 10 s: no recount yet.
        for _ in 0..29 {
            store.update_phase(0.25, &mut pellets, &mut hash, None);
        }
        assert!(store.zones[0].pellet_biomass == 100.0);
        // Cross the 10 s boundary.
        for _ in 0..11 {
            store.update_phase(0.25, &mut pellets, &mut hash, None);
        }
        assert!(store.zones[0].pellet_biomass.abs() < 1e-3);
    }

    #[test]
    fn free_zone_movement_stays_inside_and_reflects() {
        let mut config = plant_zone(0.05); // radius 100 at S=2000
        config.movement = MovementType::Free;
        // speed = 2000·1·1800/3600 = 1000 u/s; bounds centre = 1900.
        config.speed = 1800.0;
        let mut store = ZoneStore::from_configs(vec![config], bounds(2000.0), default_globals(), 1);
        let zone = &mut store.zones[0];
        zone.pos = Vec2::new(1890.0, 0.0);
        zone.vel = Vec2::new(50.0, 0.0);
        store.update_movement(1.0);
        let zone = &store.zones[0];
        assert!(zone.pos.x <= 1900.0 + 1e-3);
        assert!(zone.vel.x < 0.0, "velocity reflected: {}", zone.vel.x);
        assert!(almost(zone.config.pos_x, zone.pos.x / 2000.0));
    }

    #[test]
    fn attached_zone_follows_target_delta() {
        let mut target = plant_zone(0.1);
        target.name = "target".to_owned();
        target.movement = MovementType::Free;
        target.speed = 1800.0;
        let mut follower = plant_zone(0.1);
        follower.name = "follower".to_owned();
        follower.movement = MovementType::Attached;
        follower.target = Some(0); // target zone id after renumbering
        let mut store =
            ZoneStore::from_configs(vec![target, follower], bounds(2000.0), default_globals(), 2);
        store.zones[0].pos = Vec2::new(100.0, 100.0);
        store.zones[0].vel = Vec2::new(20.0, 0.0);
        let before = store.zones[1].pos;
        store.update_movement(0.5);
        assert!(store.zones[1].pos.x > before.x);
    }

    #[test]
    fn zone_groups_expand_deterministically() {
        let bounds = bounds(2000.0);
        let globals = default_globals();
        let mut group = ZoneGroupConfig::default();
        group.0.name = "Group 1".to_owned();
        group.0.zone_count = 3;
        group.0.scale_with_sim = false;
        group.0.template = zone_with(ZoneConfig::default_template(), |zone| {
            zone.radius_relative = 0.05;
        })
        .0;
        let a = ZoneStore::from_scenario(Vec::new(), &[group.clone()], bounds, globals.clone(), 21);
        let b = ZoneStore::from_scenario(Vec::new(), &[group.clone()], bounds, globals.clone(), 21);
        assert_eq!(a.len(), 3);
        assert_eq!(a.len(), b.len());
        for i in 0..3 {
            assert_eq!(a.zones[i].config.name, format!("Group 1_{i}"));
            assert_eq!(a.zones[i].config.zone_id, Some(i as u32));
            assert_eq!(a.zones[i].pos, b.zones[i].pos);
            let cap = 1.0 - a.zones[i].config.relative_radius(2000.0);
            assert!(a.zones[i].config.pos_x.abs() <= cap + 1e-6);
            assert!(a.zones[i].config.pos_y.abs() <= cap + 1e-6);
        }

        // scaleWithSim count = ceil(density · 4S²/1e6).
        let mut scaling = ZoneGroupConfig::default();
        scaling.0.scale_with_sim = true;
        scaling.0.zone_density = 0.66;
        scaling.0.zone_count = 1;
        scaling.0.template = group.0.template.clone();
        let count = scaling.count(2000.0);
        assert_eq!(
            count,
            ((0.66f32 * 16_000_000.0 / 1_000_000.0).ceil()) as u32
        );
        let store = ZoneStore::from_scenario(Vec::new(), &[scaling], bounds, globals.clone(), 21);
        assert_eq!(store.len(), count as usize);
    }

    #[test]
    fn per_zone_rng_is_deterministic_across_stores() {
        let config = plant_zone(0.25);
        let build = |seed: u64| {
            let mut store = ZoneStore::from_configs(
                vec![config.clone()],
                bounds(2000.0),
                default_globals(),
                seed,
            );
            let mut pellets = PelletStore::new();
            let mut hash = SpatialHash::new(Vec2::splat(-4000.0), Vec2::splat(4000.0), 32.0);
            store.initial_seeding(&mut pellets, &mut hash, None);
            let mut energies: Vec<f32> = pellets
                .ids()
                .map(|id| pellets.energy(id).unwrap())
                .collect();
            energies.sort_by(|a, b| a.partial_cmp(b).unwrap());
            energies
        };
        let a = build(123);
        let b = build(123);
        let c = build(124);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn zone_at_respects_shape_and_order() {
        let mut circle = plant_zone(0.1);
        circle.name = "circle".to_owned();
        circle.distribution = SpawnDistribution::Flat;
        circle.pos_x = 0.0;
        circle.pos_y = 0.0;
        let mut rect = plant_zone(0.1);
        rect.name = "rect".to_owned();
        rect.distribution = SpawnDistribution::Rect;
        rect.width_relative = 0.2;
        rect.height_relative = 0.2;
        rect.pos_x = 0.1; // centre (200, 0); x∈[0,400], y∈[-200,200]
        rect.pos_y = 0.0;
        let store =
            ZoneStore::from_configs(vec![circle, rect], bounds(2000.0), default_globals(), 1);
        assert_eq!(store.zone_at(Vec2::ZERO), Some(store.zones[0].id));
        // Inside the rect only (x=300 is outside the 200-radius circle).
        assert_eq!(
            store.zone_at(Vec2::new(300.0, 0.0)),
            Some(store.zones[1].id)
        );
        assert_eq!(store.zone_at(Vec2::new(1800.0, 1800.0)), None);
        // Overlap resolves to the first zone in list order.
        assert_eq!(
            store.zone_at(Vec2::new(100.0, 0.0)),
            Some(store.zones[0].id)
        );
    }

    #[test]
    fn ring_zone_contains_only_the_annulus() {
        let mut config = plant_zone(0.1); // radius 200
        config.distribution = SpawnDistribution::Ring;
        config.inside_radius = 0.5;
        let store = ZoneStore::from_configs(vec![config], bounds(2000.0), default_globals(), 1);
        let zone = &store.zones[0];
        assert!(zone.contains(Vec2::new(150.0, 0.0)));
        assert!(!zone.contains(Vec2::new(10.0, 0.0)));
        assert!(!zone.contains(Vec2::new(260.0, 0.0)));
    }

    #[test]
    fn zone_positions_and_ids_are_stable_and_capped() {
        let mut config = plant_zone(0.25);
        config.pos_x = 3.0; // out of range -> clamped to 1 - 0.25 = 0.75
        let store = ZoneStore::from_configs(
            vec![config, plant_zone(0.25)],
            bounds(2000.0),
            default_globals(),
            1,
        );
        assert_eq!(store.len(), 2);
        assert!(almost(store.zones[0].config.pos_x, 0.75));
        assert_eq!(store.zones[0].config.zone_id, Some(0));
        assert_eq!(store.zones[1].config.zone_id, Some(1));
        assert_eq!(store.zone_id_at(1), Some(store.zones[1].id));
        assert_eq!(
            store.get(store.zones[1].id).map(|z| z.config.zone_id),
            Some(Some(1))
        );
        assert!(store.get(ZoneId::from_handle(Handle::new(9, 0))).is_none());
    }

    #[test]
    fn removal_renumbers_and_invalidates_handles() {
        let mut store = ZoneStore::from_configs(
            vec![plant_zone(0.1), plant_zone(0.1), plant_zone(0.1)],
            bounds(2000.0),
            default_globals(),
            1,
        );
        let old = store.zones[1].id;
        let removed = store.remove_zone(old).expect("removed");
        assert_eq!(removed.config.zone_id, Some(1));
        assert_eq!(store.len(), 2);
        assert!(store.get(old).is_none());
        assert_eq!(store.zones[0].config.zone_id, Some(0));
        assert_eq!(store.zones[1].config.zone_id, Some(1));
    }

    #[test]
    fn removal_keeps_followers_on_the_same_zone() {
        let mut first = plant_zone(0.1);
        let second = plant_zone(0.1);
        let mut third = plant_zone(0.1);
        first.movement = MovementType::Attached;
        first.target = Some(2); // follows the third zone
        third.movement = MovementType::Attached;
        third.target = Some(1); // follows the second zone
        let mut store = ZoneStore::from_configs(
            vec![first, second, third],
            bounds(2000.0),
            default_globals(),
            1,
        );
        assert_eq!(store.zones[0].target_index, Some(2));
        assert_eq!(store.zones[2].target_index, Some(1));
        // The first zone goes: the others move up one place and keep following their zones.
        let id = store.zones[0].id;
        store.remove_zone(id).expect("removed");
        assert_eq!(store.zones[1].config.target, Some(0));
        assert_eq!(store.zones[1].target_index, Some(0));
        // The followed zone goes: the follower loses its target.
        let id = store.zones[0].id;
        store.remove_zone(id).expect("removed");
        assert_eq!(store.zones[0].config.target, None);
        assert_eq!(store.zones[0].target_index, None);
    }

    #[test]
    fn dist_and_movement_names_round_trip() {
        for distribution in SpawnDistribution::ALL {
            assert_eq!(
                SpawnDistribution::from_name(distribution.name()),
                Some(distribution)
            );
        }
        for movement in MovementType::ALL {
            assert_eq!(MovementType::from_name(movement.name()), Some(movement));
        }
        assert_eq!(SpawnDistribution::from_name("bogus"), None);
        assert_eq!(MovementType::from_name("bogus"), None);
    }

    #[test]
    fn zone_without_material_produces_nothing() {
        let config = zone_with(ZoneConfig::default_zone(), |zone| {
            zone.material = None;
        });
        let store = ZoneStore::from_configs(vec![config], bounds(2000.0), default_globals(), 1);
        assert_eq!(store.zones[0].max_biomass, 0.0);
        assert_eq!(store.zones[0].total_growth, 0.0);
        assert_eq!(
            store.zones[0].estimated_pellets(&default_globals(), 2000.0),
            0
        );
    }

    #[test]
    fn settings_mapping_matches_game_defaults() {
        use crate::settings::{MaterialKind as SettingsMaterial, SimSettings};
        let settings = SimSettings::game_defaults();
        let globals = ZoneGlobals::from_settings(&settings);
        assert_eq!(globals, ZoneGlobals::game_defaults());
        assert_eq!(settings.pellet_energy_min, 10.0);

        let zone_settings = crate::settings::ZoneSettings {
            name: "mapped".to_owned(),
            zone_id: Some(3),
            material: Some(SettingsMaterial::Meat),
            distribution: SpawnDistribution::Ring,
            fertility: 2.5,
            biomass_density: 3.5,
            pellet_size: 1.25,
            renew_biomass: true,
            movement: MovementType::Free,
            target: Some(7),
            speed: 2.0,
            pos_x: 0.25,
            pos_y: -0.25,
            radius_is_relative: false,
            radius_relative: 0.3,
            radius_absolute: 750.0,
            inside_radius: 0.6,
            ..crate::settings::ZoneSettings::game_defaults()
        };
        let config = ZoneConfig::from_settings(&zone_settings);
        assert_eq!(config.name, "mapped");
        assert_eq!(config.zone_id, Some(3));
        assert_eq!(config.material, Some(crate::settings::MaterialKind::Meat));
        assert_eq!(config.distribution, SpawnDistribution::Ring);
        assert_eq!(config.fertility, 2.5);
        assert_eq!(config.biomass_density, 3.5);
        assert_eq!(config.pellet_size, 1.25);
        assert!(config.renew_biomass);
        assert_eq!(config.movement, MovementType::Free);
        assert_eq!(config.target, Some(7));
        assert_eq!(config.speed, 2.0);
        assert!(!config.radius_is_relative);
        assert_eq!(config.radius_absolute, 750.0);
        assert_eq!(config.inside_radius, 0.6);
        // Round-trip material mapping.
        assert_eq!(
            material_from_settings(material_to_settings(MaterialKind::Plant)),
            MaterialKind::Plant
        );

        let group_settings = crate::settings::ZoneGroupSettings {
            name: "g".to_owned(),
            scale_with_sim: true,
            zone_count: 5,
            zone_density: 0.5,
            template: zone_settings,
            zones: Vec::new(),
        };
        let group = ZoneGroupConfig::from_settings(&group_settings);
        assert_eq!(group.name, "g");
        assert!(group.scale_with_sim);
        assert_eq!(group.zone_density, 0.5);
        assert_eq!(group.count(2000.0), group_settings.count(2000.0));
        assert_eq!(group.template.name, "mapped");
    }
}
