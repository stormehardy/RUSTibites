//! Scenario world tools: radiation towers, pheromone towers, colour killers and
//! the Red Death bloom.
//!
//! Spec: `docs/spec/03-world-ecology.md` §6 (tools), §7 (shade/Red Death),
//! Appendix B; interface contract: `docs/INTERFACES.md` §6/§7.
//!
//! # Layering
//!
//! Tools are **pure data + math** here: they never reach into `BibiteStore`.
//! The world collects candidate targets from the spatial hash (deterministic
//! order) and calls the `collect_*` methods, which write results into reusable
//! scratch buffers inside [`crate::tools::ToolStore`]; the world then applies those results
//! through the body/genome hooks once B2 lands (mutation dose, `kill`, pellet
//! amount removal). Pheromone towers are fully wired because they only need the
//! [`PheromoneStore`](crate::pheromone::PheromoneStore).
//!
//! # Fidelity notes (documented ambiguities)
//!
//! * **ColorKiller polarity:** the compiled game code kills when the hue
//!   *distance is **>= tolerance*** (`if (!(num5 < tolerance)) { kill }`,
//!   `ColorKiller.cs:166`), i.e. it culls colours **outside** a tolerance band —
//!   a "keep this colour lineage" tool. **Capture-confirmed** by parity task-18
//!   (5/5 cases, all four `bodyOrEye`x`anyOrMost` combinations; see
//!   `docs/PARITY.md` Appendix A and
//!   `testdata/parity/experiments/color_killer_polarity`). The supplemental
//!   guide's opposite claim is wrong. Both helpers remain for clarity:
//!   [`crate::tools::ColorKiller::wants_kill`] (compiled/captured) and
//!   [`crate::tools::ColorKiller::hue_matches_alternative`] (the disproven reading, kept
//!   documented).
//! * The compiled code has a stale-`NonAlloc`-buffer bug (loop bounded by
//!   `targetsInView[i] != null` over a 128 array that is never cleared). This
//!   implementation instead iterates exactly the 128 deterministic candidates
//!   provided by the caller (fixed bug; documented divergence).
//! * The compiled code's first-scan period is `period` seconds after placement
//!   (progress starts at 0 and is persisted); reproduced.
//! * RadTower/RedDeath need per-bibite state; the world/body applies their
//!   outputs in W4 once `BibiteStore` (B2) lands.

use crate::arena::Handle;
use crate::ids::{BibiteId, PheromoneId, ToolId, ZoneId};
use crate::math::{Vec2, clamp};
use crate::pheromone::PheromoneStore;
use crate::rng::Rng;
use crate::zone::ZoneStore;

/// RadTower dose cadence (`RadioArea.period`, spec §6.2).
pub const RAD_PERIOD: f32 = 0.5;

/// Red Death damage cadence (`RedDeathBloomManager.period`, spec §7.2).
pub const RED_DEATH_PERIOD: f32 = 0.5;

/// Red Death base damage default (`redDeathBaseDamages`, 0.5).
pub const RED_DEATH_BASE_DAMAGE: f32 = 0.5;

/// Red Death damage velocity default (`redDeathDamageVelocity`, 0.15).
pub const RED_DEATH_DAMAGE_VELOCITY: f32 = 0.15;

/// Red Death pheromone emission density (`pheromonesDensityPerSecond`, visual).
pub const RED_DEATH_PHERO_DENSITY: f32 = 0.0001;

/// RadTower minimum radius (`RadTower.radius.minValue`).
pub const RAD_MIN_RADIUS: f32 = 10.0;

/// RadTower fallback default radius (`RadTower.radius.DefaultValue = min(250, max/2)`).
pub const RAD_DEFAULT_RADIUS: f32 = 250.0;

/// Maximum number of candidates a ColorKiller scans per cycle (game's 128-slot
/// `OverlapCircleNonAlloc` cap).
pub const COLOR_KILLER_MAX_TARGETS: usize = 128;

/// Common state of every placeable tower (`EffectTower`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TowerBase {
    /// Serialized tower id (`EffectTower.ID`, stored in `scene.bb8scene`).
    pub save_id: u64,
    /// World position.
    pub pos: Vec2,
    /// Zone this tower follows, if any (`attachedZone`).
    pub attached_zone: Option<ZoneId>,
    /// Loaded from a save: `EffectTower.LoadState` leaves the prefab's colour (the radiation and pheromone towers' bases are 0.7725 opaque),
    /// `Place()` sets it to 1 (the viewer draws with it; nothing in the simulation reads it).
    pub loaded: bool,
}

impl TowerBase {
    /// Creates a tower base at `pos`.
    #[inline]
    pub fn new(save_id: u64, pos: Vec2) -> Self {
        Self {
            save_id,
            pos,
            attached_zone: None,
            loaded: false,
        }
    }

    /// Snaps the tower to its attached zone's centre (game
    /// `EffectTower.FixedUpdate` / `ColorKiller.FixedUpdate`).
    pub fn snap_to_zone(&mut self, zones: &ZoneStore) {
        if let Some(zone) = self.attached_zone.and_then(|id| zones.get(id)) {
            self.pos = zone.pos;
        }
    }
}

/// Radiation tower (`RadTower` + `RadioArea`, spec §6.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadTower {
    /// Common tower state.
    pub base: TowerBase,
    /// Mutation-count dose rate (`count`, default 0.001 N/s).
    pub count_rate: f32,
    /// Mutation-intensity dose rate (`intensity`, default 0.001 %/s).
    pub intensity_rate: f32,
    /// Effect radius in world units.
    pub radius: f32,
    /// Taper factor (0 = flat field, 1 = linear falloff, ...).
    pub taper: f32,
    /// Accumulator toward the 0.5 s dose tick.
    pub progress: f32,
}

impl RadTower {
    /// Creates a rad tower with the game defaults at `pos` for a simulation of
    /// half-size `sim_size` (radius `min(250, sim_size/4)`, clamped to
    /// `[10, sim_size/2]`).
    #[must_use]
    pub fn new(save_id: u64, pos: Vec2, sim_size: f32) -> Self {
        let max_radius = sim_size / 2.0;
        let radius = RAD_DEFAULT_RADIUS.min(max_radius / 2.0).max(RAD_MIN_RADIUS);
        Self {
            base: TowerBase::new(save_id, pos),
            count_rate: 0.001,
            intensity_rate: 0.001,
            radius,
            taper: 1.0,
            progress: 0.0,
        }
    }

    /// Clamps `radius` to the game's slider range `[10, sim_size/2]`.
    pub fn clamp_radius(&mut self, sim_size: f32) {
        self.radius = clamp(
            self.radius,
            RAD_MIN_RADIUS,
            (sim_size / 2.0).max(RAD_MIN_RADIUS),
        );
    }

    /// Taper factor `f = 1` if `taper ≈ 0`, else `1 − (d/radius)^(1/taper)`.
    #[must_use]
    pub fn taper_factor(&self, distance: f32) -> f32 {
        if self.radius <= 0.0 {
            return 0.0;
        }
        let ratio = distance / self.radius;
        if ratio > 1.0 {
            return 0.0;
        }
        if self.taper.abs() <= 1e-6 {
            1.0
        } else {
            1.0 - ratio.powf(1.0 / self.taper)
        }
    }

    /// Advances the 0.5 s dose timer; returns `true` when a dose is due.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.progress += dt;
        if self.progress < RAD_PERIOD {
            return false;
        }
        self.progress -= RAD_PERIOD;
        true
    }
}

/// One bibite's accumulated rad dose for this tick (`MutationIncreases` keyed
/// under `"Radioactivity"`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadDose {
    /// Target bibite.
    pub id: BibiteId,
    /// Added mutation-count term.
    pub count: f32,
    /// Added mutation-intensity (σ) term.
    pub intensity: f32,
}

/// A bibite position sample fed to the rad/colour tool scans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BibiteTarget {
    /// Bibite id.
    pub id: BibiteId,
    /// Position (the game's bibite root transform).
    pub pos: Vec2,
}

/// A bibite colour sample fed to a ColorKiller cycle.
///
/// `color` is the body colour from the genes (`GenesToColor`), `alpha` its alpha
/// (rainbow/`isRain` bibites are translucent and partly immune), `eye_offset`
/// the `EyeOffset` gene for eye mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorCandidate {
    /// Bibite id.
    pub id: BibiteId,
    /// Position.
    pub pos: Vec2,
    /// Body RGB in `[0, 1]` (`GetBodyColor`).
    pub color: [f32; 3],
    /// Body colour alpha (`GetBodyColor().a`; 0 for rainbow bibites).
    pub alpha: f32,
    /// `EyeOffset` gene (used when `body_or_eye` is true).
    pub eye_offset: f32,
}

/// Pheromone tower (`PheroTower`, spec §6.3): a persistent, non-dissipating
/// spot with fixed per-channel strengths and a heading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PheroTower {
    /// Common tower state.
    pub base: TowerBase,
    /// Red channel strength (`r`, 0–100 u²).
    pub red: f32,
    /// Green channel strength (`g`, 0–100 u²).
    pub green: f32,
    /// Blue channel strength (`b`, 0–100 u²).
    pub blue: f32,
    /// Heading in radians (`dir`; direction = `Vec2::X.rotated(heading)`).
    pub heading: f32,
    /// The persistent spot owned by this tower (runtime only).
    pub spot: Option<PheromoneId>,
}

impl PheroTower {
    /// Creates a phero tower at `pos` with zero strengths (game defaults).
    #[must_use]
    pub fn new(save_id: u64, pos: Vec2) -> Self {
        Self {
            base: TowerBase::new(save_id, pos),
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            heading: 0.0,
            spot: None,
        }
    }

    /// Emitted heading vector (`Vector2.right.Rotate(heading)`).
    #[inline]
    pub fn heading_vector(&self) -> Vec2 {
        Vec2::X.rotate(self.heading)
    }

    /// Spot strengths `[r, g, b]`.
    #[inline]
    pub fn strength(&self) -> [f32; 3] {
        [self.red, self.green, self.blue]
    }
}

/// ColorKiller (`ColorKiller`, spec §6.4): periodically selects at most one
/// bibite by hue and kills it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorKiller {
    /// Common tower state (ColorKiller also supports following a zone).
    pub base: TowerBase,
    /// Target hue `[0, 1]`.
    pub hue: f32,
    /// Hue-distance tolerance.
    pub tolerance: f32,
    /// `false` = body colour, `true` = eye colour (hue shifted by `EyeOffset`).
    pub body_or_eye: bool,
    /// `false` = kill the first candidate; `true` = kill the largest hue
    /// distance candidate.
    pub any_or_most: bool,
    /// Overlap radius.
    pub radius: f32,
    /// Seconds between kill cycles.
    pub period: f32,
    /// Accumulator toward the next cycle.
    pub progress: f32,
    /// Number of kills performed (runtime stat).
    pub killed: u32,
}

impl ColorKiller {
    /// Creates a killer with the values serialized in the game's `ColorKiller` prefab
    /// (`sharedassets2.assets`): hue 0, tolerance 0.1, body colour, any candidate, radius 400,
    /// period 100 s (read from the prefab's raw bytes with `tools/unity/`).
    #[must_use]
    pub fn new(save_id: u64, pos: Vec2) -> Self {
        Self {
            base: TowerBase::new(save_id, pos),
            hue: 0.0,
            tolerance: 0.1,
            body_or_eye: false,
            any_or_most: false,
            radius: 400.0,
            period: 100.0,
            progress: 0.0,
            killed: 0,
        }
    }

    /// Advances the cycle timer; returns `true` when a scan is due.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.progress += dt;
        if self.progress < self.period {
            return false;
        }
        self.progress -= self.period;
        true
    }

    /// Compiled game hue-distance score (`num5` in `ColorKiller.cs`):
    /// circular hue distance, optionally hue-shifted for eye mode and weakened
    /// by saturation/value for body mode.
    #[must_use]
    pub fn hue_distance_score(&self, candidate: &ColorCandidate) -> f32 {
        let (hue, saturation, value) = rgb_to_hsv(candidate.color);
        let shifted = if self.body_or_eye {
            wrap_unit(hue - candidate.eye_offset)
        } else {
            hue
        };
        let distance = (shifted - self.hue).abs();
        let mut score = distance.min((distance - 1.0).abs());
        if !self.body_or_eye {
            score = 1.0 - (1.0 - score) * saturation * value;
        }
        score
    }

    /// **Compiled polarity:** the game kills when the score is
    /// `>= tolerance` (spec §6.4; `if (!(num5 < tolerance)) { kill }`).
    #[inline]
    pub fn wants_kill(&self, score: f32) -> bool {
        score >= self.tolerance
    }

    /// **Documented alternative interpretation:** kill when the hue matches
    /// within tolerance (`score < tolerance`). NOT what the compiled branch
    /// does; kept for parity-capture validation and user-facing docs.
    #[inline]
    pub fn hue_matches_alternative(&self, score: f32) -> bool {
        score < self.tolerance
    }
}

/// Red Death bloom state (`RedDeathBloomManager`, spec §7.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RedDeathBloom {
    /// Whether the event is enabled (`enableRedDeath`).
    pub enabled: bool,
    /// Bloom area fraction `[0, 1]` (`redDeathFill`).
    pub fill: f32,
    /// Damage accumulator seed on entry (`redDeathBaseDamages`, 0.5).
    pub base_damage: f32,
    /// Damage-velocity accumulator growth (`redDeathDamageVelocity`, 0.15).
    pub damage_velocity: f32,
    /// Shade end radius `1.5·S + 1000` (world coordinate of the bloom).
    pub shade_end: f32,
    /// Damage cadence accumulator.
    pub progress: f32,
    /// Current safe radius (`shadeEnd · sqrt(1 − fill)`); `+∞` when disabled.
    pub safe_radius: f32,
}

impl RedDeathBloom {
    /// Creates an (initially disabled) bloom for a world whose shade ends at
    /// `shade_end`.
    #[must_use]
    pub fn new(shade_end: f32) -> Self {
        let mut bloom = Self {
            enabled: false,
            fill: 0.0,
            base_damage: RED_DEATH_BASE_DAMAGE,
            damage_velocity: RED_DEATH_DAMAGE_VELOCITY,
            shade_end,
            progress: 0.0,
            safe_radius: f32::INFINITY,
        };
        bloom.recompute_radius();
        bloom
    }

    /// Enables/disables the event; disabling sets the safe radius to `+∞`.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.recompute_radius();
    }

    /// Sets the bloom fill fraction (clamped to `[0, 1]`) and recomputes the
    /// safe radius.
    pub fn set_fill(&mut self, fill: f32) {
        self.fill = clamp(fill, 0.0, 1.0);
        self.recompute_radius();
    }

    /// `safeRadius = shadeEnd · sqrt(1 − fill)`:
    /// from `safeRadius = shadeEnd·(1 − (1 − sqrt(1 − fill)))`
    /// (`RedDeathBloomManager.UpdateRadius`; spec §7.2).
    pub fn recompute_radius(&mut self) {
        self.safe_radius = if self.enabled {
            self.shade_end * (1.0 - self.fill).max(0.0).sqrt()
        } else {
            f32::INFINITY
        };
    }

    /// `true` when `pos` is inside the bloom (`|pos| >= safeRadius`).
    #[inline]
    pub fn in_bloom(&self, pos: Vec2) -> bool {
        self.enabled && pos.length() >= self.safe_radius
    }

    /// Advances the 0.5 s damage timer; returns `true` when damage is due.
    pub fn tick(&mut self, dt: f32) -> bool {
        if !self.enabled {
            return false;
        }
        self.progress += dt;
        if self.progress < RED_DEATH_PERIOD {
            return false;
        }
        self.progress -= RED_DEATH_PERIOD;
        true
    }

    /// Seed accumulator on entry (`OnBibiteEnter`/`OnPelletEnter`).
    #[inline]
    pub fn entry_accumulator(&self) -> f32 {
        self.base_damage
    }

    /// One damage tick: `acc += velocity·0.5; damage = acc·0.5`.
    #[inline]
    pub fn damage_tick(&self, acc: &mut f32) -> f32 {
        *acc += self.damage_velocity * RED_DEATH_PERIOD;
        *acc * RED_DEATH_PERIOD
    }
}

/// One scenario tool (flat table with stable `ToolId` indices).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    /// Radiation tower.
    Rad(RadTower),
    /// Pheromone tower.
    Phero(PheroTower),
    /// Colour killer.
    Killer(ColorKiller),
}

impl Tool {
    /// Serialized tower id.
    #[inline]
    pub fn save_id(&self) -> u64 {
        match self {
            Self::Rad(tower) => tower.base.save_id,
            Self::Phero(tower) => tower.base.save_id,
            Self::Killer(tower) => tower.base.save_id,
        }
    }

    /// Mutable common tower state.
    #[inline]
    pub fn base_mut(&mut self) -> &mut TowerBase {
        match self {
            Self::Rad(tower) => &mut tower.base,
            Self::Phero(tower) => &mut tower.base,
            Self::Killer(tower) => &mut tower.base,
        }
    }

    /// Shared common tower state.
    #[inline]
    pub fn base(&self) -> &TowerBase {
        match self {
            Self::Rad(tower) => &tower.base,
            Self::Phero(tower) => &tower.base,
            Self::Killer(tower) => &tower.base,
        }
    }
}

/// Result of the pheromone-tower update: spots the world must insert into /
/// remove from its spatial hash.
#[derive(Debug)]
pub struct PheroTowerUpdate<'a> {
    /// Newly created spots `(id, pos)` that must be hash-inserted.
    pub added: &'a [(PheromoneId, Vec2)],
    /// Spots removed because the tower moved (replaced by a new one).
    pub removed: &'a [PheromoneId],
}

/// All scenario tools plus the Red Death state.
///
/// M1 tools are created/loaded at world init and (optionally) removed by editor
/// actions; `ToolId` is the index into [`ToolStore::tools`] with generation 0
/// (no runtime arena reuse in M1 - document if removal support lands).
pub struct ToolStore {
    tools: Vec<Tool>,
    /// Red Death event state (also a "tool" but global instead of placed).
    pub red_death: RedDeathBloom,
    rad_scratch: Vec<RadDose>,
    rad_due: Vec<usize>,
    kill_scratch: Vec<BibiteId>,
    kill_due: Vec<usize>,
    kill_best: Vec<Option<BibiteId>>,
    kill_best_score: Vec<f32>,
    kill_done: Vec<bool>,
    kill_scanned: Vec<u16>,
    kill_hit: Vec<bool>,
    phero_added: Vec<(PheromoneId, Vec2)>,
    phero_removed: Vec<PheromoneId>,
}

impl ToolStore {
    /// Creates an empty tool store with a disabled Red Death bloom.
    #[must_use]
    pub fn new(shade_end: f32) -> Self {
        Self {
            tools: Vec::new(),
            red_death: RedDeathBloom::new(shade_end),
            rad_scratch: Vec::new(),
            rad_due: Vec::new(),
            kill_scratch: Vec::new(),
            kill_due: Vec::new(),
            kill_best: Vec::new(),
            kill_best_score: Vec::new(),
            kill_done: Vec::new(),
            kill_scanned: Vec::new(),
            kill_hit: Vec::new(),
            phero_added: Vec::new(),
            phero_removed: Vec::new(),
        }
    }

    /// Number of placed tools.
    #[inline]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// `true` if no tools are placed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The placed tools.
    #[inline]
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    /// Mutable placed tools (load-path attachment remapping; order is stable).
    #[inline]
    pub fn tools_mut(&mut self) -> &mut [Tool] {
        &mut self.tools
    }

    /// `ToolId` of the tool at `index` (generation 0).
    #[inline]
    pub fn tool_id_at(index: usize) -> Option<ToolId> {
        if index < u32::MAX as usize {
            Some(ToolId::from_handle(Handle::new(index as u32, 0)))
        } else {
            None
        }
    }

    /// Appends a tool and returns its id.
    pub fn push(&mut self, tool: Tool) -> ToolId {
        let index = self.tools.len();
        self.tools.push(tool);
        Self::tool_id_at(index).expect("tool table full")
    }

    /// Tool by id.
    #[inline]
    pub fn get(&self, id: ToolId) -> Option<&Tool> {
        self.tools.get(id.handle().slot() as usize)
    }

    /// Mutable tool by id.
    #[inline]
    pub fn get_mut(&mut self, id: ToolId) -> Option<&mut Tool> {
        self.tools.get_mut(id.handle().slot() as usize)
    }

    /// Next unused serialized tool id (`max(save_id) + 1`, or 1 when empty).
    #[must_use]
    pub fn next_save_id(&self) -> u64 {
        self.tools
            .iter()
            .map(|tool| tool.save_id())
            .max()
            .map_or(1, |id| id.saturating_add(1))
    }

    /// Removes a tool by index id (viewer delete action). Later `ToolId`s shift
    /// down by one; serialized identity remains the stable `save_id`.
    pub fn remove_tool(&mut self, id: ToolId) -> Option<Tool> {
        let index = id.handle().slot() as usize;
        if index < self.tools.len() {
            Some(self.tools.remove(index))
        } else {
            None
        }
    }

    /// Adds a rad tower and returns its id.
    pub fn add_rad_tower(&mut self, tower: RadTower) -> ToolId {
        self.push(Tool::Rad(tower))
    }

    /// Adds a phero tower and returns its id.
    pub fn add_phero_tower(&mut self, tower: PheroTower) -> ToolId {
        self.push(Tool::Phero(tower))
    }

    /// Adds a colour killer and returns its id.
    pub fn add_color_killer(&mut self, killer: ColorKiller) -> ToolId {
        self.push(Tool::Killer(killer))
    }

    /// Phase 3: snaps every attached tower to its zone (game
    /// `EffectTower.FixedUpdate` / `ColorKiller.FixedUpdate`).
    pub fn snap_attached(&mut self, zones: &ZoneStore) {
        for tool in &mut self.tools {
            tool.base_mut().snap_to_zone(zones);
        }
    }

    /// Phase 3/7: ensures every pheromone tower owns a non-dissipating spot at
    /// its current position, recreating it when an attached zone moved the
    /// tower.
    ///
    /// Returns the spots to register/remove in the world's spatial hash.
    pub fn step_phero_towers<'a>(
        &'a mut self,
        zones: &ZoneStore,
        pheromones: &mut PheromoneStore,
    ) -> PheroTowerUpdate<'a> {
        self.phero_added.clear();
        self.phero_removed.clear();
        for index in 0..self.tools.len() {
            let Tool::Phero(tower) = &mut self.tools[index] else {
                continue;
            };
            tower.base.snap_to_zone(zones);
            let existing = tower.spot.filter(|spot| pheromones.contains(*spot));
            if let Some(existing) = existing {
                let needs_move = pheromones
                    .pos(existing)
                    .is_some_and(|pos| pos.distance_sq(tower.base.pos) > 1e-6);
                if !needs_move {
                    // `PheroTower.UpdatePheroSpot`: the spot follows every edit of the strengths
                    // and the heading (the tower panel's sliders).
                    pheromones.set_tower_state(existing, tower.heading_vector(), tower.strength());
                    continue;
                }
                pheromones.remove(existing);
                self.phero_removed.push(existing);
            }
            // Load path: adopt a loaded non-dissipating spot already at the
            // tower position (real 0.6.3.1 saves do not serialize tower spots,
            // so this is normally a no-op; robust for future formats).
            if tower.spot.is_none() {
                let adopted = pheromones.ids().find(|id| {
                    pheromones.get(*id).is_some_and(|view| {
                        !view.dissipate && view.pos.distance_sq(tower.base.pos) <= 1e-4
                    })
                });
                if let Some(spot) = adopted {
                    tower.spot = Some(spot);
                    continue;
                }
            }
            let spec = crate::pheromone::PheroEmission::tower(
                tower.base.pos,
                tower.heading_vector(),
                tower.strength(),
                ToolId::from_handle(Handle::new(index as u32, 0)),
            );
            if let Some(spot) = pheromones.emit(spec) {
                tower.spot = Some(spot);
                self.phero_added.push((spot, tower.base.pos));
            }
        }
        PheroTowerUpdate {
            added: &self.phero_added,
            removed: &self.phero_removed,
        }
    }

    /// Phase 9: collects rad doses for the bibites supplied by the world.
    ///
    /// Caller responsibilities: query the world for bibites within each tower's
    /// radius (hash circle query with `Bibite` class), pass them as `targets`,
    /// then apply every [`RadDose`] via the genome mutation-factor hook.
    pub fn collect_rad_doses(
        &mut self,
        dt: f32,
        zones: &ZoneStore,
        targets: impl IntoIterator<Item = BibiteTarget>,
    ) -> &[RadDose] {
        self.rad_scratch.clear();
        self.rad_due.clear();
        for index in 0..self.tools.len() {
            let Tool::Rad(tower) = &mut self.tools[index] else {
                continue;
            };
            tower.base.snap_to_zone(zones);
            if tower.tick(dt) {
                self.rad_due.push(index);
            }
        }
        for target in targets {
            for &index in &self.rad_due {
                let Tool::Rad(tower) = &self.tools[index] else {
                    continue;
                };
                let distance = target.pos.distance(tower.base.pos);
                if distance > tower.radius {
                    continue;
                }
                let factor = tower.taper_factor(distance);
                self.rad_scratch.push(RadDose {
                    id: target.id,
                    count: tower.count_rate * RAD_PERIOD * factor,
                    intensity: tower.intensity_rate * RAD_PERIOD * factor,
                });
            }
        }
        &self.rad_scratch
    }

    /// Phase 11: runs due colour killers over the supplied candidates and
    /// returns the bibites to kill (at most one per killer).
    ///
    /// Candidates are scanned in the order supplied (the world passes the
    /// spatial hash's deterministic order); each killer scans at most
    /// [`COLOR_KILLER_MAX_TARGETS`] candidates, matching the game's 128-slot
    /// `NonAlloc` cap. The game's stale-buffer bug is fixed by construction.
    pub fn collect_color_kills(
        &mut self,
        dt: f32,
        zones: &ZoneStore,
        candidates: impl IntoIterator<Item = ColorCandidate>,
        rng: &mut Rng,
    ) -> &[BibiteId] {
        self.kill_scratch.clear();
        self.kill_due.clear();
        for index in 0..self.tools.len() {
            let Tool::Killer(killer) = &mut self.tools[index] else {
                continue;
            };
            killer.base.snap_to_zone(zones);
            if killer.tick(dt) {
                self.kill_due.push(index);
            }
        }
        let due = self.kill_due.len();
        self.kill_best.clear();
        self.kill_best.resize(due, None);
        self.kill_best_score.clear();
        self.kill_best_score.resize(due, 0.0);
        self.kill_done.clear();
        self.kill_done.resize(due, false);
        self.kill_scanned.clear();
        self.kill_scanned.resize(due, 0);
        self.kill_hit.clear();
        self.kill_hit.resize(due, false);

        for candidate in candidates {
            for slot in 0..due {
                if self.kill_done[slot]
                    || self.kill_scanned[slot] as usize >= COLOR_KILLER_MAX_TARGETS
                {
                    continue;
                }
                let index = self.kill_due[slot];
                let Tool::Killer(killer) = &self.tools[index] else {
                    continue;
                };
                if candidate.pos.distance(killer.base.pos) > killer.radius {
                    continue;
                }
                self.kill_scanned[slot] += 1;
                // Rainbow/translucent bibites are partly immune
                // (`Random.value > bodyColor.a`).
                if rng.f32() > candidate.alpha {
                    continue;
                }
                let score = killer.hue_distance_score(&candidate);
                if !killer.wants_kill(score) {
                    continue;
                }
                if !killer.any_or_most {
                    self.kill_scratch.push(candidate.id);
                    self.kill_done[slot] = true;
                    self.kill_hit[slot] = true;
                } else if score >= self.kill_best_score[slot] {
                    self.kill_best_score[slot] = score;
                    self.kill_best[slot] = Some(candidate.id);
                    self.kill_hit[slot] = true;
                }
            }
        }
        for slot in 0..due {
            if !self.kill_hit[slot] {
                continue;
            }
            if let Some(id) = self.kill_best[slot] {
                self.kill_scratch.push(id);
            }
        }
        // Update per-killer kill counters after the immutable scans.
        for slot in 0..due {
            if self.kill_hit[slot] {
                let index = self.kill_due[slot];
                if let Tool::Killer(killer) = &mut self.tools[index] {
                    killer.killed += 1;
                }
            }
        }
        &self.kill_scratch
    }

    /// Phase 10: advances the Red Death timer; returns `true` when one damage
    /// tick is due. The world then applies [`RedDeathBloom::damage_tick`] to
    /// every bibite/pellet with `|pos| >= safe_radius`, seeded with
    /// [`RedDeathBloom::entry_accumulator`] on first entry.
    pub fn step_red_death(&mut self, dt: f32) -> bool {
        self.red_death.tick(dt)
    }
}

/// Converts RGB `[0, 1]` to HSV `(h, s, v)` like Unity's `Color.RGBToHSV`.
#[must_use]
pub fn rgb_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let value = max;
    let saturation = if max <= 0.0 { 0.0 } else { delta / max };
    let hue = if delta <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    (hue, saturation, value)
}

/// Wraps a hue into `[0, 1)`.
#[inline]
pub fn wrap_unit(value: f32) -> f32 {
    value - value.floor()
}

/// HSV-to-RGB for `s = v = 1` (the ColorKiller crystal/laser colour, and eye
/// colour derivation), returning `[r, g, b]`.
#[must_use]
pub fn hue_to_rgb(hue: f32) -> [f32; 3] {
    let h = wrap_unit(hue) * 6.0;
    let sector = h.floor();
    let f = h - sector;
    let q = 1.0 - f;
    let t = f;
    match sector as i32 % 6 {
        0 => [1.0, t, 0.0],
        1 => [q, 1.0, 0.0],
        2 => [0.0, 1.0, t],
        3 => [0.0, q, 1.0],
        4 => [t, 0.0, 1.0],
        _ => [1.0, 0.0, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BibiteId;
    use crate::math::PI;
    use crate::world::WorldBounds;
    use crate::zone::{ZoneConfig, ZoneStore};

    fn almost(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4
    }

    fn empty_zones() -> ZoneStore {
        ZoneStore::new(
            WorldBounds::new(2000.0),
            crate::zone::ZoneGlobals::game_defaults(),
            1,
        )
    }

    #[test]
    fn rad_taper_formula() {
        let mut tower = RadTower::new(1, Vec2::ZERO, 2000.0);
        tower.radius = 100.0;
        tower.taper = 1.0;
        assert!(almost(tower.taper_factor(0.0), 1.0));
        assert!(almost(tower.taper_factor(50.0), 0.5));
        assert!(almost(tower.taper_factor(100.0), 0.0));
        tower.taper = 0.0;
        assert!(almost(tower.taper_factor(90.0), 1.0));
        tower.taper = 2.0;
        assert!(almost(tower.taper_factor(50.0), 1.0 - 0.5f32.sqrt()));
        assert_eq!(tower.taper_factor(101.0), 0.0);
    }

    #[test]
    fn rad_cadence_is_half_second_and_single_subtract() {
        let mut tower = RadTower::new(1, Vec2::ZERO, 2000.0);
        assert!(!tower.tick(0.25));
        assert!(!tower.tick(0.24));
        assert!(tower.tick(0.01));
        // A huge dt only fires once and keeps the remainder.
        let mut tower = RadTower::new(1, Vec2::ZERO, 2000.0);
        assert!(tower.tick(1.2));
        assert!(almost(tower.progress, 0.7));
    }

    #[test]
    fn rad_default_radius_clamps_to_sim_size() {
        let mut tower = RadTower::new(1, Vec2::ZERO, 200.0);
        // max radius 100 -> default min(250, 50) = 50.
        assert!(almost(tower.radius, 50.0));
        tower.radius = 1.0;
        tower.clamp_radius(200.0);
        assert!(almost(tower.radius, RAD_MIN_RADIUS));
        tower.radius = 1000.0;
        tower.clamp_radius(200.0);
        assert!(almost(tower.radius, 100.0));
    }

    #[test]
    fn rad_collects_doses_with_taper() {
        let zones = empty_zones();
        let mut store = ToolStore::new(4000.0);
        let mut tower = RadTower::new(7, Vec2::ZERO, 2000.0);
        tower.radius = 100.0;
        tower.count_rate = 0.01;
        tower.intensity_rate = 0.02;
        store.add_rad_tower(tower);
        let b1 = BibiteId::from_handle(Handle::new(1, 0));
        let b2 = BibiteId::from_handle(Handle::new(2, 0));
        let targets = [
            BibiteTarget {
                id: b1,
                pos: Vec2::ZERO,
            },
            BibiteTarget {
                id: b2,
                pos: Vec2::new(50.0, 0.0),
            },
        ];
        // 0.25 s -> not due.
        assert!(store.collect_rad_doses(0.25, &zones, targets).is_empty());
        // Another 0.25 -> due: two doses.
        let doses = store.collect_rad_doses(0.25, &zones, targets);
        assert_eq!(doses.len(), 2);
        assert_eq!(doses[0].id, b1);
        assert!(almost(doses[0].count, 0.01 * 0.5 * 1.0));
        assert!(almost(doses[0].intensity, 0.02 * 0.5 * 1.0));
        assert!(almost(doses[1].count, 0.01 * 0.5 * 0.5));
    }

    #[test]
    fn hsv_helpers_round_trip_primaries() {
        let (h, s, v) = rgb_to_hsv([1.0, 0.0, 0.0]);
        assert!(almost(h, 0.0));
        assert!(almost(s, 1.0));
        assert!(almost(v, 1.0));
        let (h, _, _) = rgb_to_hsv([0.0, 1.0, 0.0]);
        assert!(almost(h, 1.0 / 3.0));
        let (h, _, _) = rgb_to_hsv([0.0, 0.0, 1.0]);
        assert!(almost(h, 2.0 / 3.0));
        assert_eq!(hue_to_rgb(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(hue_to_rgb(1.0 / 3.0), [0.0, 1.0, 0.0]);
        assert_eq!(hue_to_rgb(2.0 / 3.0), [0.0, 0.0, 1.0]);
    }

    fn killer_red_target_blue() -> (ColorKiller, ColorCandidate) {
        let mut killer = ColorKiller::new(1, Vec2::ZERO);
        killer.hue = 0.0; // red target
        killer.tolerance = 0.1;
        killer.radius = 100.0;
        killer.period = 1.0;
        let blue = ColorCandidate {
            id: BibiteId::from_handle(Handle::new(1, 0)),
            pos: Vec2::new(10.0, 0.0),
            color: [0.0, 0.0, 1.0],
            alpha: 1.0,
            eye_offset: 0.0,
        };
        (killer, blue)
    }

    #[test]
    fn color_killer_compiled_polarity_kills_non_matching_colors() {
        let (killer, blue) = killer_red_target_blue();
        let score = killer.hue_distance_score(&blue);
        // Blue hue 0.667 vs target 0: circular distance 0.333.
        assert!(almost(score, 1.0 / 3.0));
        assert!(killer.wants_kill(score), "compiled polarity kills it");
        assert!(!killer.hue_matches_alternative(score));

        // A pure red bibite is inside the tolerance band -> spared.
        let red = ColorCandidate {
            color: [1.0, 0.0, 0.0],
            ..blue
        };
        let score = killer.hue_distance_score(&red);
        assert!(almost(score, 0.0));
        assert!(!killer.wants_kill(score));
        assert!(killer.hue_matches_alternative(score));
    }

    #[test]
    fn color_killer_eye_mode_shifts_hue() {
        let mut killer = ColorKiller::new(1, Vec2::ZERO);
        killer.hue = 0.0;
        killer.tolerance = 0.1;
        killer.body_or_eye = true;
        // Body red, eye offset 0.5 -> eye hue 0.5 (cyan), distance 0.5 -> kill.
        let candidate = ColorCandidate {
            id: BibiteId::from_handle(Handle::new(1, 0)),
            pos: Vec2::ZERO,
            color: [1.0, 0.0, 0.0],
            alpha: 1.0,
            eye_offset: 0.5,
        };
        assert!(almost(killer.hue_distance_score(&candidate), 0.5));
        assert!(killer.wants_kill(0.5));
        // Body mode ignores the eye offset.
        killer.body_or_eye = false;
        assert!(almost(killer.hue_distance_score(&candidate), 0.0));
    }

    #[test]
    fn color_killer_any_selects_first_and_most_selects_max_distance() {
        let zones = empty_zones();
        let blue = ColorCandidate {
            id: BibiteId::from_handle(Handle::new(1, 0)),
            pos: Vec2::ZERO,
            color: [0.0, 0.0, 1.0],
            alpha: 1.0,
            eye_offset: 0.0,
        };
        let green = ColorCandidate {
            id: BibiteId::from_handle(Handle::new(2, 0)),
            color: [0.0, 1.0, 0.0],
            ..blue
        };
        let mut rng = Rng::from_seed(1);

        // "Any" mode kills the first matching candidate.
        let mut store = ToolStore::new(4000.0);
        let mut any = ColorKiller::new(1, Vec2::ZERO);
        any.period = 1.0;
        any.radius = 100.0;
        any.tolerance = 0.1;
        store.add_color_killer(any);
        let kills = store.collect_color_kills(1.0, &zones, [blue, green], &mut rng);
        assert_eq!(kills, &[blue.id]);

        // "Most" mode picks the largest hue distance; ties favour the later
        // candidate (`>=`), so green wins the blue/green tie.
        let mut store = ToolStore::new(4000.0);
        let mut most = ColorKiller::new(2, Vec2::ZERO);
        most.period = 1.0;
        most.radius = 100.0;
        most.tolerance = 0.1;
        most.any_or_most = true;
        store.add_color_killer(most);
        let kills = store.collect_color_kills(1.0, &zones, [blue, green], &mut rng);
        assert_eq!(kills, &[green.id]);
    }

    #[test]
    fn color_killer_skips_translucent_bibites() {
        let zones = empty_zones();
        let mut store = ToolStore::new(4000.0);
        let mut killer = ColorKiller::new(1, Vec2::ZERO);
        killer.period = 1.0;
        killer.radius = 100.0;
        killer.tolerance = 0.1;
        store.add_color_killer(killer);
        let rainbow = ColorCandidate {
            id: BibiteId::from_handle(Handle::new(1, 0)),
            pos: Vec2::ZERO,
            color: [0.0, 0.0, 1.0],
            alpha: 0.0, // Color.clear for isRain
            eye_offset: 0.0,
        };
        let mut rng = Rng::from_seed(1);
        let kills = store.collect_color_kills(1.0, &zones, [rainbow], &mut rng);
        assert!(kills.is_empty());
    }

    #[test]
    fn color_killer_caps_candidates_at_128() {
        let zones = empty_zones();
        let mut store = ToolStore::new(4000.0);
        let mut killer = ColorKiller::new(1, Vec2::ZERO);
        killer.period = 1.0;
        killer.radius = 1e9;
        killer.tolerance = 0.01;
        killer.any_or_most = true;
        store.add_color_killer(killer);
        let candidates: Vec<ColorCandidate> = (0..200u32)
            .map(|i| ColorCandidate {
                id: BibiteId::from_handle(Handle::new(i, 0)),
                pos: Vec2::ZERO,
                color: [0.0, 0.0, 1.0],
                alpha: 1.0,
                eye_offset: 0.0,
            })
            .collect();
        let mut rng = Rng::from_seed(1);
        let kills = store.collect_color_kills(1.0, &zones, candidates, &mut rng);
        assert_eq!(kills.len(), 1);
        // Ties pick the last *scanned* candidate, i.e. index 127 (cap).
        assert_eq!(kills[0].handle().slot(), 127);
    }

    #[test]
    fn phero_tower_heading_vector_and_tower_spot() {
        let zones = empty_zones();
        let mut store = ToolStore::new(4000.0);
        let mut pheromones = PheromoneStore::new();
        let mut tower = PheroTower::new(5, Vec2::new(10.0, 0.0));
        tower.red = 4.0;
        tower.heading = PI / 2.0; // right rotated +90 -> up
        assert!(almost(tower.heading_vector().x, 0.0));
        assert!(almost(tower.heading_vector().y, 1.0));
        let _id = store.add_phero_tower(tower);
        let update = store.step_phero_towers(&zones, &mut pheromones);
        assert_eq!(update.added.len(), 1);
        assert!(update.removed.is_empty());
        let (spot, pos) = update.added[0];
        assert_eq!(pos, Vec2::new(10.0, 0.0));
        let view = pheromones.get(spot).unwrap();
        assert!(!view.dissipate);
        assert_eq!(view.strength, [4.0, 0.0, 0.0]);
        assert!(view.owner_tool.is_some());
        // Stable across ticks when the tower does not move.
        let update = store.step_phero_towers(&zones, &mut pheromones);
        assert!(update.added.is_empty());
        assert!(update.removed.is_empty());
    }

    /// `PheroTower.UpdatePheroSpot`: an edit of the strengths or the heading (the tower panel's
    /// sliders) reaches the existing spot without recreating it.
    #[test]
    fn phero_tower_spot_follows_edits_of_strengths_and_heading() {
        let zones = empty_zones();
        let mut store = ToolStore::new(4000.0);
        let mut pheromones = PheromoneStore::new();
        let id = store.add_phero_tower(PheroTower::new(5, Vec2::new(10.0, 0.0)));
        let update = store.step_phero_towers(&zones, &mut pheromones);
        let (spot, _) = update.added[0];
        assert_eq!(pheromones.get(spot).unwrap().strength, [0.0, 0.0, 0.0]);
        let Some(Tool::Phero(tower)) = store.get_mut(id) else {
            panic!("phero tower missing");
        };
        tower.red = 1.0;
        tower.green = 2.0;
        tower.blue = 3.0;
        tower.heading = PI / 2.0;
        let update = store.step_phero_towers(&zones, &mut pheromones);
        assert!(update.added.is_empty() && update.removed.is_empty());
        let view = pheromones.get(spot).unwrap();
        assert_eq!(view.strength, [1.0, 2.0, 3.0]);
        assert!(almost(view.heading.x, 0.0) && almost(view.heading.y, 1.0));
    }

    /// The values serialized in the game's `ColorKiller` prefab.
    #[test]
    fn a_new_color_killer_has_the_prefab_values() {
        let killer = ColorKiller::new(1, Vec2::ZERO);
        assert_eq!(killer.hue, 0.0);
        assert_eq!(killer.tolerance, 0.1);
        assert!(!killer.body_or_eye && !killer.any_or_most);
        assert_eq!(killer.radius, 400.0);
        assert_eq!(killer.period, 100.0);
        assert_eq!(killer.progress, 0.0);
    }

    #[test]
    fn red_death_safe_radius_formula() {
        let mut bloom = RedDeathBloom::new(4000.0);
        assert!(!bloom.enabled);
        assert_eq!(bloom.safe_radius, f32::INFINITY);
        bloom.set_enabled(true);
        assert_eq!(bloom.safe_radius, 4000.0);
        bloom.set_fill(0.75);
        assert!(almost(bloom.safe_radius, 4000.0 * 0.5));
        bloom.set_fill(1.0);
        assert!(almost(bloom.safe_radius, 0.0));
        bloom.set_fill(0.0);
        assert!(almost(bloom.safe_radius, 4000.0));
        // in_bloom uses the shade-space radial distance.
        bloom.set_fill(0.75); // safe radius 2000
        assert!(!bloom.in_bloom(Vec2::new(1999.0, 0.0)));
        assert!(bloom.in_bloom(Vec2::new(2000.0, 0.0)));
        bloom.set_enabled(false);
        assert!(!bloom.in_bloom(Vec2::new(5000.0, 0.0)));
    }

    #[test]
    fn red_death_damage_cadence_and_formula() {
        let mut bloom = RedDeathBloom::new(4000.0);
        bloom.set_enabled(true);
        let mut acc = bloom.entry_accumulator();
        assert!(almost(acc, 0.5));
        // 0.25 s -> no tick.
        assert!(!bloom.tick(0.25));
        assert!(bloom.tick(0.25));
        // acc = 0.5 + 0.15*0.5 = 0.575; damage = 0.2875.
        let damage = bloom.damage_tick(&mut acc);
        assert!(almost(damage, 0.2875));
        assert!(almost(acc, 0.575));
        assert!(bloom.tick(0.5));
        let damage = bloom.damage_tick(&mut acc);
        // acc = 0.575 + 0.075 = 0.65; damage = 0.325.
        assert!(almost(damage, 0.325));
        assert!(almost(acc, 0.65));
    }

    #[test]
    fn red_death_disabled_never_ticks() {
        let mut bloom = RedDeathBloom::new(4000.0);
        assert!(!bloom.tick(100.0));
        assert_eq!(bloom.progress, 0.0);
    }

    #[test]
    fn tower_snap_follows_attached_zone() {
        let mut zone_config = ZoneConfig::default_zone();
        zone_config.pos_x = 0.5;
        let zones = ZoneStore::from_configs(
            vec![zone_config],
            WorldBounds::new(2000.0),
            crate::zone::ZoneGlobals::game_defaults(),
            1,
        );
        let zone_id = zones.zones[0].id;
        let mut store = ToolStore::new(4000.0);
        let mut tower = RadTower::new(1, Vec2::ZERO, 2000.0);
        tower.base.attached_zone = Some(zone_id);
        store.add_rad_tower(tower);
        store.snap_attached(&zones);
        match store.get(ToolStore::tool_id_at(0).unwrap()).unwrap() {
            Tool::Rad(tower) => assert_eq!(tower.base.pos, zones.zones[0].pos),
            _ => panic!("wrong tool type"),
        }
    }
}
