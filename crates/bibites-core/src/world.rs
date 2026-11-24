//! World-level infrastructure: bounds, uniform-grid spatial hash, shared bitset,
//! the world container and its tick skeleton.
//!
//! The full sensing/tools/orchestration pipeline lands in W3
//! (`docs/INTERFACES.md` §6.1); this module currently provides:
//!
//! * [`crate::world::WorldBounds`] — simulation half-size and the shade/wrap/cull radii from
//!   spec §0.2/§7.1.
//! * [`crate::world::SpatialHash`] — a uniform grid over the simulation area with
//!   **extensible class masks** (bibite / pellet / pheromone spot / egg),
//!   incremental `insert` / `remove` / `update` / `retain`, and deterministic
//!   capped queries into a reusable [`crate::world::QueryScratch`].
//! * [`crate::world::ClassMask`] — a bitmask selecting which entity classes a query returns.
//! * `BitSet` — the shared slot bitset used by the pellet and pheromone
//!   stores.
//! * [`crate::world::World`] — the ecology skeleton: bounds + zones + pellets + pheromones +
//!   hash + clock, with a partial `tick()` running the phase-1/4/7/8 systems
//!   (clock, zones/spawning, pheromone dissipation, pellet decay). W3 extends it
//!   to the full spec §9.2 pipeline and the INTERFACES fields.
//! * [`crate::world::random_inside_unit_circle`] — the deterministic equivalent of Unity's
//!   `Random.insideUnitCircle` (uniform in the unit disc).
//!
//! # Determinism & allocation rules
//!
//! * Query results are sorted by `(class, slot, generation)` before being
//!   returned, so results do **not** depend on insertion/removal history and
//!   float summation over them is order-stable.
//! * Queries never allocate once the [`crate::world::QueryScratch`] has been warmed up (the
//!   scratch is reused and sorts in place). Results are capped at the scratch
//!   capacity (the game's `Physics2D` query limit is **128**); when a query is
//!   truncated, scanning stops and `truncated = true`.
//! * Entities outside the grid bounds are clamped into border cells; exact
//!   distance/overlap tests still discard them, so a query never returns a
//!   false positive because of clamping.
//!
//! # Cell size
//!
//! The grid covers a configured world rectangle (`min..max`). Cell size should
//! be on the order of the typical query radius. A huge grid is mostly empty
//! `Vec`s, so prefer a cell size that keeps `nx · ny` modest; [`crate::world::World::new_ecology`]
//! derives `max(32, sim_size/128)` from the simulation size.

use crate::arena::Handle;
use crate::body::BibiteStore;
use crate::body::SlotSummary;
use crate::body::{
    BibiteEvent, BibiteSave, BibiteScratch, BibiteSpawn, BibiteTickCtx, DeathCause, EggSave,
    EggStore, FovSeenCache, SpawnVitals, SteeringInputs, ZoneCircle,
};
use crate::brain::Brain;
use crate::clock::FixedClock;
use crate::error::{Error, Result};
use crate::genome::{Gene, Genome, GenomeMeta, MutationFactor};
use crate::ids::{BibiteId, EggId, PelletId, ToolId, ZoneId};
use crate::math::{TAU, Vec2};
use crate::pellet::{
    DecayThrottle, MaterialKind, MaterialProperties, MaterialTable, PelletSpawn, PelletStore,
};
use crate::pheromone::{PheroEmission, PheromoneStore};
use crate::pool_tuner::{Arm, Choice, PoolSwitch, PoolTuner, PoolTunerStats, WorldSize};
use crate::rng::{Rng, domain};
use crate::senses::{self, FovCadence, FovMask, FovParams, SenseOutputs};
use crate::settings::RandomizeGenes;
use crate::settings::{
    self, BibiteSpawn as SpawnRow, ChallengeParameters, GrowthAtSpawn, InterpolationType,
    SettingKind, SettingValue, SettingsChanger, SimMetric, SimSettings, SimulationMetric,
    SplinePoint, Tagging,
};
use crate::sim::{
    FeedingInterval, Phase, PhaseTimings, PoolJob, SimSnapshot, SimStats, StatsSample, ThreadPool,
};
use crate::species::{LineageRecordIo, SPECIES_LOG_PERIOD_SECONDS, SpeciesRegistry};
use crate::tools::{
    BibiteTarget, ColorCandidate, ColorKiller, PheroTower, RadDose, RadTower, RedDeathBloom,
    ToolStore,
};
use crate::zone::{SeedingReport, ZoneGlobals, ZoneStore, ZoneUpdateReport};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

/// Query result cap used by the game's `Physics2D.*NonAlloc` calls.
pub const DEFAULT_QUERY_CAP: usize = 128;

/// Entity classes tracked by the spatial hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpatialClass {
    /// Bibite bodies (colliders live on body parts in the game).
    Bibite,
    /// Plant/meat pellets.
    Pellet,
    /// Pheromone spots.
    Pheromone,
    /// Eggs.
    Egg,
}

impl SpatialClass {
    /// Number of classes (size of the per-class slot maps).
    pub const COUNT: usize = 4;

    /// Canonical serialization/array order for the per-class slot maps.
    /// Capture and restore must both use this order (task-126 audit).
    pub(crate) const ORDER: [SpatialClass; Self::COUNT] =
        [Self::Bibite, Self::Pellet, Self::Pheromone, Self::Egg];

    /// Dense index of this class, used for the per-class slot maps.
    #[inline]
    pub const fn index(self) -> usize {
        match self {
            Self::Bibite => 0,
            Self::Pellet => 1,
            Self::Pheromone => 2,
            Self::Egg => 3,
        }
    }

    /// Single-class mask for this class.
    #[inline]
    pub const fn mask(self) -> ClassMask {
        ClassMask(1 << self.index())
    }
}

/// A bitmask of [`SpatialClass`]es for spatial queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassMask(u8);

impl ClassMask {
    /// Empty mask (matches nothing).
    pub const NONE: Self = Self(0);
    /// Matches bibite bodies only.
    pub const BIBITE: Self = Self(1);
    /// Matches pellets only.
    pub const PELLET: Self = Self(1 << 1);
    /// Matches pheromone spots only.
    pub const PHEROMONE: Self = Self(1 << 2);
    /// Matches eggs only.
    pub const EGG: Self = Self(1 << 3);
    /// Matches every class.
    pub const ALL: Self = Self(0b1111);

    /// Creates a mask from a class.
    #[inline]
    pub const fn from_class(class: SpatialClass) -> Self {
        class.mask()
    }

    /// Union of two masks.
    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns `true` if `class` is selected by this mask.
    #[inline]
    pub const fn contains(self, class: SpatialClass) -> bool {
        self.0 & class.mask().0 != 0
    }

    /// Returns `true` if the mask selects nothing.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Raw bits (for save/debug output).
    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl core::ops::BitOr for ClassMask {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for ClassMask {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// One query result entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryHit {
    /// Entity handle (typed by the owning store).
    pub handle: Handle,
    /// Class of the entity.
    pub class: SpatialClass,
    /// Precomputed sort key `(class, slot)`; slots are unique per class among
    /// live entries, so this is equivalent to `(class, slot, generation)` for a
    /// single query while letting the hot sort compare `u64`s.
    key: u64,
}

impl QueryHit {
    #[inline]
    fn new(handle: Handle, class: SpatialClass) -> Self {
        Self {
            handle,
            class,
            key: ((class.index() as u64) << 32) | u64::from(handle.slot()),
        }
    }

    /// Sort key used by [`QueryScratch::finish`].
    #[inline]
    fn sort_key(&self) -> u64 {
        self.key
    }
}

/// One spatial-hash entry.
#[derive(Clone, Copy, Debug)]
struct Entry {
    handle: Handle,
    class: SpatialClass,
    pos: Vec2,
    radius: f32,
    cell: u32,
    cell_slot: u32,
}

/// Reusable query output buffer (cap defaults to [`DEFAULT_QUERY_CAP`]).
///
/// The world owns one or more of these and passes them to query methods; the
/// buffer capacity **is** the result cap. Sorting happens in place, so queries
/// do not allocate once the buffer has reached its cap.
pub struct QueryScratch {
    hits: Vec<QueryHit>,
    truncated: bool,
}

impl QueryScratch {
    /// Creates a scratch buffer with the game's default cap (128).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_QUERY_CAP)
    }

    /// Creates a scratch buffer with an explicit cap.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            hits: Vec::with_capacity(cap),
            truncated: false,
        }
    }

    /// The result cap (buffer capacity).
    #[inline]
    pub fn cap(&self) -> usize {
        self.hits.capacity()
    }

    /// Number of hits from the last query.
    #[inline]
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// `true` if the last query returned no hits.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// `true` if the last query hit the cap and stopped scanning early.
    #[inline]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Hits from the last query, in deterministic sorted order.
    #[inline]
    pub fn as_slice(&self) -> &[QueryHit] {
        &self.hits
    }

    /// Clears the buffer (capacity is retained).
    #[inline]
    pub fn clear(&mut self) {
        self.hits.clear();
        self.truncated = false;
    }

    #[inline]
    fn begin(&mut self) {
        self.hits.clear();
        self.truncated = false;
    }

    #[inline]
    fn push(&mut self, hit: QueryHit) -> bool {
        if self.hits.len() >= self.hits.capacity() {
            self.truncated = true;
            return false;
        }
        self.hits.push(hit);
        true
    }

    #[inline]
    fn finish(&mut self) -> QueryResult<'_> {
        self.hits.sort_unstable_by_key(QueryHit::sort_key);
        QueryResult {
            hits: &self.hits,
            truncated: self.truncated,
        }
    }

    /// Finishes without sorting (per-bibite FOV accumulation only): the hash's
    /// iteration order is deterministic for a given world history and each
    /// bibite's senses are computed independently, so serial and parallel runs
    /// still agree. Avoids the per-query 128-hit sort that dominated sensing.
    #[inline]
    fn finish_unsorted(&mut self) -> QueryResult<'_> {
        QueryResult {
            hits: &self.hits,
            truncated: self.truncated,
        }
    }
}

impl Default for QueryScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// A query result borrowing the scratch buffer it was written into.
#[derive(Debug)]
pub struct QueryResult<'a> {
    /// Sorted hits.
    pub hits: &'a [QueryHit],
    /// Whether the query hit the scratch cap and stopped early.
    pub truncated: bool,
}

impl QueryResult<'_> {
    /// Number of hits.
    #[inline]
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// `true` if there were no hits.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }
}

/// Radius of a pheromone spot's trigger collider: 0.001 for the spots bibites emit, 0.1 for a tower's (read from the two `PheromoneSpot` prefabs).
/// `Pherosense` finds spots with `Physics2D.OverlapCircleNonAlloc`, so a spot whose collider touches the sense circle counts although its
/// centre lies outside it; the strength it adds still comes from the centre.
fn pheromone_spot_radius(tower: bool) -> f32 {
    if tower { 0.1 } else { 0.001 }
}

/// A uniform-grid spatial hash over a rectangular world region.
///
/// Entities are inserted per class with a bounding radius; queries use the
/// largest tracked radius to expand the candidate cell range so that large
/// entities whose centres lie outside the query shape are still considered and
/// then exact-tested.
#[derive(Clone)]
pub struct SpatialHash {
    min: Vec2,
    inv_cell: f32,
    cell_size: f32,
    nx: u32,
    ny: u32,
    cells: Vec<Vec<u32>>,
    entries: Vec<Entry>,
    free_entries: Vec<u32>,
    /// Per-class maps from handle slot to entry index (`u32::MAX` = absent).
    slot_maps: [Vec<u32>; SpatialClass::COUNT],
    len: usize,
    max_radius: f32,
    /// Reusable scratch for [`SpatialHash::retain`] (no allocation after warm-up).
    retain_scratch: Vec<u32>,
}

impl SpatialHash {
    /// Creates a grid covering `min..max` with the given cell size.
    ///
    /// # Panics
    ///
    /// Panics if `cell_size` is not finite/positive, or if either axis of
    /// `min..max` is not positive/finite.
    pub fn new(min: Vec2, max: Vec2, cell_size: f32) -> Self {
        assert!(
            cell_size.is_finite() && cell_size > 0.0,
            "spatial hash cell size must be positive and finite"
        );
        assert!(
            min.is_finite() && max.is_finite() && max.x > min.x && max.y > min.y,
            "spatial hash bounds must be finite and strictly ordered"
        );
        let nx = (((max.x - min.x) / cell_size).ceil() as u32).max(1);
        let ny = (((max.y - min.y) / cell_size).ceil() as u32).max(1);
        let cell_count = nx as usize * ny as usize;
        Self {
            min,
            inv_cell: 1.0 / cell_size,
            cell_size,
            nx,
            ny,
            cells: vec![Vec::new(); cell_count],
            entries: Vec::new(),
            free_entries: Vec::new(),
            slot_maps: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            len: 0,
            max_radius: 0.0,
            retain_scratch: Vec::new(),
        }
    }

    /// Creates a grid and reserves `entries` slots.
    pub fn with_capacity(entries: usize, min: Vec2, max: Vec2, cell_size: f32) -> Self {
        let mut hash = Self::new(min, max, cell_size);
        hash.entries.reserve(entries);
        hash.free_entries.reserve(entries);
        hash
    }

    /// The cell size in world units.
    #[inline]
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Grid dimensions `(nx, ny)`.
    #[inline]
    pub fn grid_dimensions(&self) -> (u32, u32) {
        (self.nx, self.ny)
    }

    /// The grid's lower-left corner.
    #[inline]
    pub fn min(&self) -> Vec2 {
        self.min
    }

    /// Number of tracked entities.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` if no entities are tracked.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Largest bounding radius currently tracked (never shrinks until
    /// [`SpatialHash::clear`]; conservative expansion for queries).
    #[inline]
    pub fn max_entry_radius(&self) -> f32 {
        self.max_radius
    }

    /// Inserts an entity. Returns `false` if the handle is already present in
    /// that class, or the position/radius is not finite/negative.
    pub fn insert(&mut self, handle: Handle, class: SpatialClass, pos: Vec2, radius: f32) -> bool {
        if !pos.is_finite() || !radius.is_finite() || radius < 0.0 {
            return false;
        }
        if self.contains(handle, class) {
            return false;
        }
        let cell = self.cell_of(pos);
        let entry_index = if let Some(index) = self.free_entries.pop() {
            index
        } else {
            self.entries.push(Entry {
                handle,
                class,
                pos,
                radius,
                cell,
                cell_slot: 0,
            });
            (self.entries.len() - 1) as u32
        };
        let cell_slot = self.cells[cell as usize].len() as u32;
        self.cells[cell as usize].push(entry_index);
        {
            let entry = &mut self.entries[entry_index as usize];
            entry.handle = handle;
            entry.class = class;
            entry.pos = pos;
            entry.radius = radius;
            entry.cell = cell;
            entry.cell_slot = cell_slot;
        }

        let map = &mut self.slot_maps[class.index()];
        let slot = handle.slot() as usize;
        if map.len() <= slot {
            map.resize(slot + 1, u32::MAX);
        }
        map[slot] = entry_index;

        self.len += 1;
        self.max_radius = self.max_radius.max(radius);
        true
    }

    /// Removes an entity. Returns `false` if it is not present.
    pub fn remove(&mut self, handle: Handle, class: SpatialClass) -> bool {
        let Some(entry_index) = self.entry_index(handle, class) else {
            return false;
        };
        self.remove_entry(entry_index);
        true
    }

    /// Moves an entity to a new position/radius, inserting it if absent.
    ///
    /// Returns `false` only when the position/radius is invalid.
    pub fn update(&mut self, handle: Handle, class: SpatialClass, pos: Vec2, radius: f32) -> bool {
        if !pos.is_finite() || !radius.is_finite() || radius < 0.0 {
            return false;
        }
        let Some(entry_index) = self.entry_index(handle, class) else {
            return self.insert(handle, class, pos, radius);
        };
        self.max_radius = self.max_radius.max(radius);
        let new_cell = self.cell_of(pos);
        let (old_cell, old_slot) = {
            let entry = &mut self.entries[entry_index as usize];
            entry.pos = pos;
            entry.radius = radius;
            if entry.cell == new_cell {
                return true;
            }
            (entry.cell, entry.cell_slot)
        };

        // Detach from the old cell.
        let last = self.cells[old_cell as usize].len() - 1;
        self.cells[old_cell as usize].swap_remove(old_slot as usize);
        if (old_slot as usize) < last {
            let moved_entry = self.cells[old_cell as usize][old_slot as usize];
            self.entries[moved_entry as usize].cell_slot = old_slot;
        }

        // Attach to the new cell.
        let new_slot = self.cells[new_cell as usize].len() as u32;
        self.cells[new_cell as usize].push(entry_index);
        let entry = &mut self.entries[entry_index as usize];
        entry.cell = new_cell;
        entry.cell_slot = new_slot;
        true
    }

    /// Removes all entries while retaining allocated capacity.
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
        }
        self.entries.clear();
        self.free_entries.clear();
        for map in &mut self.slot_maps {
            map.clear();
        }
        self.len = 0;
        self.max_radius = 0.0;
    }

    /// Moves this hash out (for the parallel sensing snapshot) and leaves an
    /// empty hash with the same grid layout in place.
    pub(crate) fn take_same_layout(&mut self) -> Self {
        let min = self.min;
        let cell_size = self.cell_size;
        let max = Vec2::new(
            min.x + self.nx as f32 * cell_size,
            min.y + self.ny as f32 * cell_size,
        );
        core::mem::replace(self, Self::new(min, max, cell_size))
    }

    /// The handle of the entity of `class` tracked under the handle slot `slot`, if any.
    pub(crate) fn handle_at_slot(&self, class: SpatialClass, slot: usize) -> Option<Handle> {
        let index = *self.slot_maps[class.index()].get(slot)?;
        (index != u32::MAX).then(|| self.entries[index as usize].handle)
    }

    /// Whether `other` is the very same structure: entries, the order inside every cell, the free lists.
    #[cfg(test)]
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.min.x.to_bits() == other.min.x.to_bits()
            && self.min.y.to_bits() == other.min.y.to_bits()
            && self.nx == other.nx
            && self.ny == other.ny
            && self.cells == other.cells
            && self.free_entries == other.free_entries
            && self.slot_maps == other.slot_maps
            && self.len == other.len
            && self.max_radius.to_bits() == other.max_radius.to_bits()
            && self.entries.len() == other.entries.len()
            && self.entries.iter().zip(&other.entries).all(|(a, b)| {
                a.handle == b.handle
                    && a.class == b.class
                    && a.pos.x.to_bits() == b.pos.x.to_bits()
                    && a.pos.y.to_bits() == b.pos.y.to_bits()
                    && a.radius.to_bits() == b.radius.to_bits()
                    && a.cell == b.cell
                    && a.cell_slot == b.cell_slot
            })
    }

    /// Returns `true` if `handle` is tracked in `class`.
    pub fn contains(&self, handle: Handle, class: SpatialClass) -> bool {
        self.entry_index(handle, class).is_some()
    }

    /// Position of a tracked entity.
    pub fn pos_of(&self, handle: Handle, class: SpatialClass) -> Option<Vec2> {
        self.entry_index(handle, class)
            .map(|index| self.entries[index as usize].pos)
    }

    /// Bounding radius of a tracked entity.
    pub fn radius_of(&self, handle: Handle, class: SpatialClass) -> Option<f32> {
        self.entry_index(handle, class)
            .map(|index| self.entries[index as usize].radius)
    }

    /// Removes every entry of `class` for which `keep(handle)` returns `false`.
    ///
    /// Used to reconcile the hash after a store removes entities outside the
    /// hash's knowledge (pellet decay, eaten/swallowed pellets, dissipated
    /// pheromone spots). Allocation-free after the internal scratch has grown.
    pub fn retain(&mut self, class: SpatialClass, mut keep: impl FnMut(Handle) -> bool) {
        self.retain_scratch.clear();
        {
            let map = &self.slot_maps[class.index()];
            for &index in map {
                if index == u32::MAX {
                    continue;
                }
                let handle = self.entries[index as usize].handle;
                if !keep(handle) {
                    self.retain_scratch.push(index);
                }
            }
        }
        for i in 0..self.retain_scratch.len() {
            let index = self.retain_scratch[i];
            let entry = self.entries[index as usize];
            let slot = entry.handle.slot() as usize;
            // The entry may have been removed indirectly by an earlier removal
            // in this pass (slot map swap) — only remove if still live.
            if self.slot_maps[entry.class.index()].get(slot).copied() == Some(index) {
                self.remove_entry(index);
            }
        }
    }

    /// Circle query: returns entities whose bounding circle overlaps the query
    /// circle (`distance <= query_radius + entry_radius`).
    ///
    /// A non-positive radius still returns entities containing the centre point.
    pub fn query_circle<'a>(
        &self,
        center: Vec2,
        radius: f32,
        mask: ClassMask,
        scratch: &'a mut QueryScratch,
    ) -> QueryResult<'a> {
        scratch.begin();
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return scratch.finish();
        }
        let expand = radius + self.max_radius;
        let (min, max) = (
            Vec2::new(center.x - expand, center.y - expand),
            Vec2::new(center.x + expand, center.y + expand),
        );
        self.for_cells_in_aabb(min, max, &mut |entry_index| {
            let entry = &self.entries[entry_index as usize];
            if !mask.contains(entry.class) {
                return true;
            }
            let reach = radius + entry.radius;
            if entry.pos.distance_sq(center) <= reach * reach
                && !scratch.push(QueryHit::new(entry.handle, entry.class))
            {
                return false;
            }
            true
        });
        scratch.finish()
    }

    /// Circle query without the deterministic slot sort.
    ///
    /// Used by the per-bibite FOV accumulation, where each query's hits are
    /// summed independently: the hash iteration order is deterministic for a
    /// given world history and does not depend on the number of worker threads,
    /// so serial/parallel results stay identical while skipping the per-query
    /// sort (the dominant sensing cost). All other systems use the sorted
    /// [`SpatialHash::query_circle`].
    pub fn query_circle_unsorted<'a>(
        &self,
        center: Vec2,
        radius: f32,
        mask: ClassMask,
        scratch: &'a mut QueryScratch,
    ) -> QueryResult<'a> {
        scratch.begin();
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return scratch.finish_unsorted();
        }
        let expand = radius + self.max_radius;
        let (min, max) = (
            Vec2::new(center.x - expand, center.y - expand),
            Vec2::new(center.x + expand, center.y + expand),
        );
        self.for_cells_in_aabb(min, max, &mut |entry_index| {
            let entry = &self.entries[entry_index as usize];
            if !mask.contains(entry.class) {
                return true;
            }
            let reach = radius + entry.radius;
            if entry.pos.distance_sq(center) <= reach * reach
                && !scratch.push(QueryHit::new(entry.handle, entry.class))
            {
                return false;
            }
            true
        });
        scratch.finish_unsorted()
    }

    /// AABB query: returns entities whose bounding box overlaps the query box.
    pub fn query_aabb<'a>(
        &self,
        min: Vec2,
        max: Vec2,
        mask: ClassMask,
        scratch: &'a mut QueryScratch,
    ) -> QueryResult<'a> {
        scratch.begin();
        if !min.is_finite() || !max.is_finite() || max.x < min.x || max.y < min.y {
            return scratch.finish();
        }
        let expand = self.max_radius;
        let cell_min = Vec2::new(min.x - expand, min.y - expand);
        let cell_max = Vec2::new(max.x + expand, max.y + expand);
        self.for_cells_in_aabb(cell_min, cell_max, &mut |entry_index| {
            let entry = &self.entries[entry_index as usize];
            if !mask.contains(entry.class) {
                return true;
            }
            if entry.pos.x + entry.radius >= min.x
                && entry.pos.x - entry.radius <= max.x
                && entry.pos.y + entry.radius >= min.y
                && entry.pos.y - entry.radius <= max.y
                && !scratch.push(QueryHit::new(entry.handle, entry.class))
            {
                return false;
            }
            true
        });
        scratch.finish()
    }

    /// Rotated-box query (the game's FOV cone is a rotated `OverlapBox`).
    ///
    /// `half_extents` are the box half sizes; `rotation` is in radians (CCW).
    /// Returns entities whose bounding circle overlaps the rotated box.
    pub fn query_rotated_box<'a>(
        &self,
        center: Vec2,
        half_extents: Vec2,
        rotation: f32,
        mask: ClassMask,
        scratch: &'a mut QueryScratch,
    ) -> QueryResult<'a> {
        scratch.begin();
        if !center.is_finite()
            || !half_extents.is_finite()
            || half_extents.x < 0.0
            || half_extents.y < 0.0
            || !rotation.is_finite()
        {
            return scratch.finish();
        }
        let (sin, cos) = rotation.sin_cos();
        // World-space AABB of the rotated box, expanded by the largest radius.
        let extent = Vec2::new(
            half_extents.x * cos.abs() + half_extents.y * sin.abs(),
            half_extents.x * sin.abs() + half_extents.y * cos.abs(),
        );
        let expand = Vec2::new(extent.x + self.max_radius, extent.y + self.max_radius);
        let cell_min = Vec2::new(center.x - expand.x, center.y - expand.y);
        let cell_max = Vec2::new(center.x + expand.x, center.y + expand.y);
        self.for_cells_in_aabb(cell_min, cell_max, &mut |entry_index| {
            let entry = &self.entries[entry_index as usize];
            if !mask.contains(entry.class) {
                return true;
            }
            let local = entry.pos - center;
            let local_x = local.x * cos + local.y * sin;
            let local_y = -local.x * sin + local.y * cos;
            if local_x.abs() <= half_extents.x + entry.radius
                && local_y.abs() <= half_extents.y + entry.radius
                && !scratch.push(QueryHit::new(entry.handle, entry.class))
            {
                return false;
            }
            true
        });
        scratch.finish()
    }

    /// Iterates the cells covering `min..max` in deterministic (row-major) order
    /// and calls `visit(entry_index)` for each entry. `visit` returns `false` to
    /// stop iteration (used for query caps).
    fn for_cells_in_aabb(&self, min: Vec2, max: Vec2, visit: &mut impl FnMut(u32) -> bool) {
        let (x0, y0) = self.cell_coords(min);
        let (x1, y1) = self.cell_coords(max);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let cell = (y * self.nx + x) as usize;
                for &entry_index in &self.cells[cell] {
                    if !visit(entry_index) {
                        return;
                    }
                }
            }
        }
    }

    #[inline]
    fn cell_coords(&self, pos: Vec2) -> (u32, u32) {
        let x = ((pos.x - self.min.x) * self.inv_cell).floor();
        let y = ((pos.y - self.min.y) * self.inv_cell).floor();
        let x = x.clamp(0.0, (self.nx - 1) as f32) as u32;
        let y = y.clamp(0.0, (self.ny - 1) as f32) as u32;
        (x, y)
    }

    #[inline]
    fn cell_of(&self, pos: Vec2) -> u32 {
        let (x, y) = self.cell_coords(pos);
        y * self.nx + x
    }

    #[inline]
    fn entry_index(&self, handle: Handle, class: SpatialClass) -> Option<u32> {
        let map = &self.slot_maps[class.index()];
        let slot = handle.slot() as usize;
        let index = *map.get(slot)?;
        if index == u32::MAX {
            return None;
        }
        let entry = &self.entries[index as usize];
        if entry.handle == handle {
            Some(index)
        } else {
            None
        }
    }

    fn remove_entry(&mut self, entry_index: u32) {
        let entry = self.entries[entry_index as usize];
        // Detach from its cell.
        let cell = entry.cell as usize;
        let cell_slot = entry.cell_slot as usize;
        let last = self.cells[cell].len() - 1;
        self.cells[cell].swap_remove(cell_slot);
        if cell_slot < last {
            let moved = self.cells[cell][cell_slot];
            self.entries[moved as usize].cell_slot = cell_slot as u32;
        }
        // Clear the class slot map.
        let map = &mut self.slot_maps[entry.class.index()];
        let slot = entry.handle.slot() as usize;
        if slot < map.len() {
            map[slot] = u32::MAX;
        }
        self.free_entries.push(entry_index);
        self.len -= 1;
        // max_radius intentionally stays conservative until clear().
    }
}

/// Shade starts at `1.5 · simulation_size`
/// (`BackgroundManager.shadeStart`; spec §0.2).
pub const SHADE_START_FACTOR: f32 = 1.5;

/// Constant shade fade band width (`BackgroundManager.shadeFadeLength`).
pub const SHADE_FADE_LENGTH: f32 = 1000.0;

/// Pellets with `|pos| > 1.5·S + 500` are culled when shade is enabled
/// (`ZoneManager.RefreshPelletBiomassCounter`; spec §2.7).
pub const PELLET_CULL_MARGIN: f32 = 500.0;

/// Default material decay period in ticks (`ScenarioIndependentSettings.decayTPS`
/// default 2; spec §2.4). TEMP until `SimSettings` lands.
pub const DEFAULT_DECAY_TPS: u32 = 2;

/// Margin added around the shade radius when sizing the spatial hash so
/// out-of-bounds entities still land in a valid cell.
const HASH_EXTENT_MARGIN: f32 = 512.0;

/// Simulation-area bounds and world-edge constants (spec §0.2/§7.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBounds {
    sim_size: f32,
}

impl WorldBounds {
    /// Creates bounds from the simulation half-size (`SimulationSize`).
    ///
    /// # Panics
    ///
    /// Panics if `sim_size` is not finite/positive.
    pub fn new(sim_size: f32) -> Self {
        assert!(
            sim_size.is_finite() && sim_size > 0.0,
            "simulation size must be positive and finite"
        );
        Self { sim_size }
    }

    /// The simulation half-size `S` (world spans `[-S, +S]²`).
    #[inline]
    pub fn sim_size(&self) -> f32 {
        self.sim_size
    }

    /// Lower-left corner of the simulation area.
    #[inline]
    pub fn min(&self) -> Vec2 {
        Vec2::splat(-self.sim_size)
    }

    /// Upper-right corner of the simulation area.
    #[inline]
    pub fn max(&self) -> Vec2 {
        Vec2::splat(self.sim_size)
    }

    /// Half extents `(S, S)`.
    #[inline]
    pub fn half_extents(&self) -> Vec2 {
        Vec2::splat(self.sim_size)
    }

    /// `true` if `pos` is inside the lit simulation square (`|x|,|y| <= S`).
    #[inline]
    pub fn contains(&self, pos: Vec2) -> bool {
        pos.x.abs() <= self.sim_size && pos.y.abs() <= self.sim_size
    }

    /// Clamps a position into the simulation square.
    #[inline]
    pub fn clamp_position(&self, pos: Vec2) -> Vec2 {
        Vec2::new(
            pos.x.clamp(-self.sim_size, self.sim_size),
            pos.y.clamp(-self.sim_size, self.sim_size),
        )
    }

    /// Shade start radius `1.5·S`.
    #[inline]
    pub fn shade_start(&self) -> f32 {
        SHADE_START_FACTOR * self.sim_size
    }

    /// Shade end radius `1.5·S + 1000`.
    #[inline]
    pub fn shade_end(&self) -> f32 {
        self.shade_start() + SHADE_FADE_LENGTH
    }

    /// Pellet cull radius `1.5·S + 500`.
    #[inline]
    pub fn pellet_cull_radius(&self) -> f32 {
        self.shade_start() + PELLET_CULL_MARGIN
    }

    /// `true` when a pellet at `pos` would be culled by the shade census.
    #[inline]
    pub fn pellet_out_of_bounds(&self, pos: Vec2) -> bool {
        pos.length() > self.pellet_cull_radius()
    }

    /// World-wrap teleport (`BibitePropulsion`: leaves through the shade and
    /// re-enters 180° away when `|pos| − body_length >= 1.5·S + 1000`).
    ///
    /// Returns the new position when a wrap is due.
    pub fn wrap_position(&self, pos: Vec2, body_length: f32) -> Option<Vec2> {
        let limit = self.shade_end();
        if pos.length() - body_length >= limit {
            let target = limit - 0.95 * body_length;
            Some(-pos.normalize_or(Vec2::X) * target)
        } else {
            None
        }
    }

    /// Maximum absolute centre coordinate for a circular zone of `radius`
    /// (`Zone.Move`: `maxD = S − radius`).
    #[inline]
    pub fn zone_centre_limit(&self, radius: f32) -> f32 {
        self.sim_size - radius
    }

    /// Maximum centre `(x, y)` for a rectangular zone with the given half
    /// extents (`Zone.Move`: `S − rect.width`, `S − rect.height`).
    #[inline]
    pub fn zone_rect_centre_limit(&self, half_extents: Vec2) -> Vec2 {
        Vec2::new(
            self.sim_size - half_extents.x,
            self.sim_size - half_extents.y,
        )
    }
}

/// Default spatial-hash cell size for a simulation of half-size `sim_size`
/// (`max(32, sim_size/128)` keeps the grid modest for large worlds).
#[inline]
pub fn default_cell_size(sim_size: f32) -> f32 {
    (sim_size / 32.0).max(32.0)
}

/// Core-neutral bibite template used by the world spawner.
///
/// `bibites-io` decodes `templates/*.bb8` into its own `LoadedGenome`, then
/// converts it into this struct (genome + meta + brain + display name) so the
/// core never parses files. See `World::set_templates` / `World::step_spawner`.
#[derive(Clone, Debug)]
pub struct TemplateSource {
    /// Template display/file name (matched against spawn-row `path`).
    pub name: String,
    /// Genome.
    pub genome: Genome,
    /// Genome metadata (generation/species/tags/egg flags).
    pub meta: GenomeMeta,
    /// Brain.
    pub brain: Brain,
    /// Brain-editor node anchors keyed by node innovation (`nodeAnchors`).
    /// Layout-only state; io supplies it from `TemplateMeta::node_anchors`.
    pub node_anchors: std::collections::BTreeMap<i64, (f32, f32)>,
    /// `BibiteTemplate.speciesName`: a template that names a species joins it (`FindSpeciesFromTemplate`) instead of being placed by genetic distance.
    pub species_name: Option<String>,
}

impl TemplateSource {
    /// The species name of this template (`BibiteTemplate.speciesName`): the name its file carries, else its own name in species format (`ToSpeciesFormat`: "Basic bibite" for
    /// `Basic bibite.bb8template`). Every template has one: a bibite placed from it joins the species of that name, and the first one founds it.
    #[must_use]
    pub fn game_species_name(&self) -> String {
        match self.species_name.as_deref().filter(|name| !name.is_empty()) {
            Some(name) => name.to_owned(),
            None => crate::species::to_species_format(
                self.name.rsplit(['/', '\\']).next().unwrap_or(&self.name).trim_end_matches(".bb8template").trim_end_matches(".bb8"),
            ),
        }
    }

    /// Creates a template source.
    #[must_use]
    pub fn new(name: impl Into<String>, genome: Genome, meta: GenomeMeta, brain: Brain) -> Self {
        Self {
            name: name.into(),
            genome,
            meta,
            brain,
            node_anchors: std::collections::BTreeMap::new(),
            species_name: None,
        }
    }

    /// The species the template names.
    #[must_use]
    pub fn with_species_name(mut self, name: Option<String>) -> Self {
        self.species_name = name.filter(|name| !name.is_empty());
        self
    }

    /// Replaces the brain-editor node anchors (builder style).
    #[must_use]
    pub fn with_node_anchors(
        mut self,
        anchors: std::collections::BTreeMap<i64, (f32, f32)>,
    ) -> Self {
        self.node_anchors = anchors;
        self
    }

    /// Saved editor position for a node innovation, if any.
    #[must_use]
    pub fn node_anchor(&self, innovation: i64) -> Option<(f32, f32)> {
        self.node_anchors.get(&innovation).copied()
    }

    /// Creates the official starter bibite template (Basic bibite genome +
    /// brain, empty meta). This is the core-side equivalent of the game's
    /// `BibiteTemplate` created for a new library/editor entry.
    #[must_use]
    pub fn starter(name: impl Into<String>) -> Self {
        Self::new(
            name,
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        )
    }

    /// [`TemplateSource::starter`] with an explicit brain period (usually
    /// `2 / sim_tps`); useful for editor previews.
    #[must_use]
    pub fn starter_with_period(name: impl Into<String>, period: f32) -> Self {
        Self::new(
            name,
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter_with_period(period),
        )
    }

    /// Deep copy of this template (genome, metadata and brain); editing the
    /// copy does not affect the source. Equivalent to `Clone`, exposed by name
    /// for viewer/library call sites.
    #[must_use]
    pub fn duplicate(&self) -> Self {
        self.clone()
    }
}

/// Round-trip state of the save's `scene.bb8scene` section that is owned by the
/// world (metadata + simulated time; tools/entities live in their stores).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldScene {
    /// Scenario/game display name.
    pub name: String,
    /// Scenario version string.
    pub version: String,
    /// Simulated time in seconds (`simulatedTime`).
    pub simulated_time: f64,
    /// Free-form scenario tags.
    pub tags: Vec<String>,
    /// Bibite spawn rows persisted in the settings file.
    pub bibite_spawns: Vec<SpawnRow>,
}

/// Serialized pellet state for [`WorldContents`] (`pellets.bb8scene`).
///
/// Mirrors the game's save/load quirks: only position + amount + material +
/// owner bucket + decay state survive; velocity/rotation are not restored (the
/// pellet respawns at rest with identity rotation), so they are not modelled
/// here. `owner_zone` is the zone **list index** (`None` = free pellets); io
/// resolves the bucket name to this index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PelletRecord {
    /// Material.
    pub material: MaterialKind,
    /// Amount in `u²`.
    pub amount: f32,
    /// Position.
    pub pos: Vec2,
    /// Owning zone list index (`None` = free pellet).
    pub owner_zone: Option<u32>,
    /// Held by a mouth at save time.
    pub held: bool,
    /// Fresh time remaining (`matterDecay.timeAlive` → `freshTime − timeAlive`);
    /// `None` for non-decaying pellets.
    pub fresh_time_remaining: Option<f32>,
    /// Accumulated rot (`matterDecay.rotAmount`).
    pub rot_amount: f32,
}

/// Serialized pheromone spot for [`WorldContents`] (`pheromones.bb8scene`).
///
/// `heading` is always lost by the game serializer (§5.5); the record keeps it
/// so io can reproduce the on-disk shape (or the zero it loads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PheromoneRecord {
    /// Position.
    pub pos: Vec2,
    /// Heading (`Vector2.zero` after a real-game load).
    pub heading: Vec2,
    /// Per-channel strengths `[r, g, b]` (may be negative in real saves).
    pub strength: [f32; 3],
    /// Dissipation flag (not serialized; prefab default true for bibite spots).
    pub dissipate: bool,
    /// Owning tool, if any (runtime only).
    pub owner_tool: Option<ToolId>,
}

/// Every section `bibites-io` serializes for a world save, in core plain types
/// (no serde). This is the I4 boundary: io maps `SaveBundle` JSON ↔ this struct,
/// and the CLI calls [`World::from_contents`] / [`World::to_contents`].
#[derive(Clone)]
pub struct WorldContents {
    /// Effective simulation settings.
    pub settings: SimSettings,
    /// RNG seed for the reconstructed world (not serialized by the game; the CLI
    /// supplies it, defaulting to the capture seed).
    pub seed: u64,
    /// Scene metadata (name/version/simulatedTime/tags/spawn rows).
    pub scene: WorldScene,
    /// Explicit zones in list order (`ScenarioSettings.zones`).
    pub zones: Vec<settings::ZoneSettings>,
    /// Zone groups, each with the zones it generated (`ZoneGroupSettings.zones`: a group saved without them gets them generated on
    /// load). The zone ids (the targets, the pellets' owners, the spawn rows' zones) number the explicit zones first and then the
    /// groups' zones in turn (`ScenarioSettings.UpdateAllZonesList`): see [`WorldContents::all_zones`].
    pub zone_groups: Vec<settings::ZoneGroupSettings>,
    /// `settingsChangers` animators (from the settings file).
    pub settings_changers: Vec<SettingsChanger>,
    /// Radiation towers (`radTowers`).
    pub rad_towers: Vec<RadTower>,
    /// Pheromone towers (`pheroTowers`; runtime spot ids are not serialized).
    pub phero_towers: Vec<PheroTower>,
    /// Colour killers (`colorSelectors`).
    pub color_killers: Vec<ColorKiller>,
    /// Pellets (`pellets.bb8scene`).
    pub pellets: Vec<PelletRecord>,
    /// Pheromone spots (`pheromones.bb8scene`).
    pub pheromones: Vec<PheromoneRecord>,
    /// Bibites (`bibites/*.bb8`), in save order.
    pub bibites: Vec<BibiteSave>,
    /// Eggs (`eggs/*.bb8`), in save order.
    pub eggs: Vec<EggSave>,
    /// `[parent1, parent2]` of each bibite of `bibites` as positions in that list, empty when the links are not known
    /// (`SaveSystem.LoadWorld`: `parent1ID` is looked up among the bibites, and `parent2ID` only when the first was found).
    pub bibite_parents: Vec<[Option<u32>; 2]>,
    /// The same for the eggs of `eggs` (the parents of an egg are bibites).
    pub egg_parents: Vec<[Option<u32>; 2]>,
    /// Species/lineage registry (`speciesData.json`), when present.
    pub species: Option<LineageRecordIo>,
    /// Opaque `data.bin` history blob (M1 pass-through).
    pub data_bin: Vec<u8>,
    /// `img.png` of a save: the 400 x 400 screenshot of the whole world that `SceneScreenShotHandler.SaveScreenshotToZip` writes into
    /// every save. The core has no picture of the world; the window sets it. `None` keeps the one the loaded save had (a save made
    /// without a window).
    pub img_png: Option<Vec<u8>>,
}

impl WorldContents {
    /// Every zone in id order: the explicit zones, then each group's zones in turn (`ScenarioSettings.UpdateAllZonesList`). The ids a save
    /// refers to zones by (targets, the pellets' owners, the towers' attached zone, the spawn rows' zone) number this list.
    #[must_use]
    pub fn all_zones(&self) -> Vec<settings::ZoneSettings> {
        let mut all = self.zones.clone();
        for group in &self.zone_groups {
            all.extend(group.zones.iter().cloned());
        }
        all
    }
}

/// Plain-data scenario content handed from `bibites-io` to [`World::new`].
///
/// This is the canonical boundary between the file layer and the simulation: it
/// contains **only core types** (no serde) and enough data to build every store
/// without body/io templates except bibite templates (spawn rows reference
/// template files; loading those templates and spawning is done by the caller
/// after construction, see [`World::new`]).
///
/// Settings-derived scenario payload for the viewer's scenario editor / save
/// path. Mirrors the mutable fields an editor can change; io combines it with a
/// loaded `ScenarioSettingsFile` to preserve unknown keys, groups and challenge
/// data.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldScenarioSettings {
    /// Effective simulation settings (scenario + independent + materials).
    pub settings: SimSettings,
    /// Zone configs in runtime order (file zone ids preserved).
    pub zones: Vec<settings::ZoneSettings>,
    /// Spawn rows in list order (runtime counters included).
    pub spawn_rows: Vec<SpawnRow>,
    /// Settings animators / changers.
    pub settings_changers: Vec<SettingsChanger>,
}

/// Serialization mapping (`scene.bb8scene` + `zoneData.bb8zoneData`):
/// `name`/`version` from the scene header, `zones`/`zone_groups` from the zone
/// lists, `bibite_spawns` from `bibiteSpawnList`, `initial_pellets` from the
/// initial pellet list, `rad_towers`/`phero_towers`/`color_killers` from the
/// `tools` arrays, `tags` from the scene tag list.
#[derive(Clone, Debug, Default)]
pub struct ScenarioData {
    /// Scenario display name (scene header; may be empty).
    pub name: String,
    /// Scenario version string as stored in the scene (may be empty).
    pub version: String,
    /// Explicit zones, in file order.
    pub zones: Vec<settings::ZoneSettings>,
    /// Zone groups; generated zones are appended after the explicit ones.
    pub zone_groups: Vec<settings::ZoneGroupSettings>,
    /// Bibite spawn rows (template path + counts/priority/tagging).
    pub bibite_spawns: Vec<SpawnRow>,
    /// Initial pellets created at construction (already in world coordinates).
    pub initial_pellets: Vec<PelletSpawn>,
    /// Radiation towers.
    pub rad_towers: Vec<RadTower>,
    /// Pheromone towers.
    pub phero_towers: Vec<PheroTower>,
    /// Colour killers.
    pub color_killers: Vec<ColorKiller>,
    /// Settings animators (`settingsChangers`, spec 04 §7.1).
    pub settings_changers: Vec<SettingsChanger>,
    /// Scenario tags (free-form strings).
    pub tags: Vec<String>,
}

impl ScenarioData {
    /// An empty scenario that produces just the default settings' world (no
    /// zones, pellets or tools).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Scenario with the game-default zone group (10 relative-radius zones).
    #[must_use]
    pub fn with_default_zone_group() -> Self {
        Self {
            zone_groups: vec![settings::ZoneGroupSettings::game_defaults()],
            ..Self::default()
        }
    }

    /// Validates every piece of scenario data before it reaches the stores.
    ///
    /// Checks all zone/group/spawn numeric fields, pellet amounts/materials/
    /// positions, and tool parameters. `World::new` calls this first.
    pub fn validate(&self) -> Result<()> {
        for zone in &self.zones {
            zone.validate()?;
        }
        for group in &self.zone_groups {
            group.validate()?;
        }
        for changer in &self.settings_changers {
            changer.validate()?;
        }
        for spawn in &self.bibite_spawns {
            spawn.validate()?;
        }
        for pellet in &self.initial_pellets {
            if !pellet.material.is_world_pellet() {
                return Err(Error::invalid_data(format!(
                    "initial pellet has non-world material {:?}",
                    pellet.material
                )));
            }
            if !pellet.amount.is_finite() || pellet.amount <= 0.0 {
                return Err(Error::invalid_data(format!(
                    "initial pellet has invalid amount {}",
                    pellet.amount
                )));
            }
            if !pellet.pos.x.is_finite() || !pellet.pos.y.is_finite() {
                return Err(Error::invalid_data("initial pellet position is not finite"));
            }
        }
        for tower in &self.rad_towers {
            if !tower.base.pos.x.is_finite() || !tower.base.pos.y.is_finite() {
                return Err(Error::invalid_data("rad tower position is not finite"));
            }
            if !tower.radius.is_finite() || tower.radius < 0.0 {
                return Err(Error::invalid_data("rad tower radius is invalid"));
            }
            if !tower.count_rate.is_finite()
                || !tower.intensity_rate.is_finite()
                || tower.count_rate < 0.0
                || tower.intensity_rate < 0.0
            {
                return Err(Error::invalid_data("rad tower dose rates are invalid"));
            }
            if !tower.taper.is_finite() || tower.taper < 0.0 {
                return Err(Error::invalid_data("rad tower taper is invalid"));
            }
        }
        for tower in &self.phero_towers {
            if !tower.base.pos.x.is_finite() || !tower.base.pos.y.is_finite() {
                return Err(Error::invalid_data("phero tower position is not finite"));
            }
            for strength in [tower.red, tower.green, tower.blue] {
                if !strength.is_finite() || strength < 0.0 {
                    return Err(Error::invalid_data(
                        "phero tower strength is not a finite non-negative number",
                    ));
                }
            }
            if !tower.heading.is_finite() {
                return Err(Error::invalid_data("phero tower heading is not finite"));
            }
        }
        for killer in &self.color_killers {
            if !killer.base.pos.x.is_finite() || !killer.base.pos.y.is_finite() {
                return Err(Error::invalid_data("colour killer position is not finite"));
            }
            if !killer.hue.is_finite() || !(0.0..=1.0).contains(&killer.hue) {
                return Err(Error::invalid_data("colour killer hue is outside 0..=1"));
            }
            if !killer.tolerance.is_finite() || !(0.0..=1.0).contains(&killer.tolerance) {
                return Err(Error::invalid_data(
                    "colour killer tolerance is outside 0..=1",
                ));
            }
            if !killer.radius.is_finite() || killer.radius < 0.0 {
                return Err(Error::invalid_data("colour killer radius is invalid"));
            }
            if !killer.period.is_finite() || killer.period <= 0.0 {
                return Err(Error::invalid_data("colour killer period is invalid"));
            }
        }
        Ok(())
    }
}

/// Converts a settings-façade material into the runtime pellet material.
fn material_properties_from_settings(material: &settings::MaterialSetting) -> MaterialProperties {
    MaterialProperties {
        cohesiveness: material.cohesiveness,
        reactivity: material.reactivity,
        energy_density: material.energy_density,
        mass_density: material.mass_density,
        min_efficiency: material.min_efficiency,
        max_efficiency: material.max_efficiency,
        decay: material.decay,
        fresh_time: material.fresh_time,
        decay_rate: material.decay_rate,
    }
}

/// Converts settings material kinds into runtime pellet material kinds.
fn material_kind_from_settings(kind: settings::MaterialKind) -> crate::pellet::MaterialKind {
    crate::zone::material_from_settings(kind)
}

/// The tag a template spawn gets (`BibiteGenes.StartGenesFromTemplate`, cs:272-278): the species name, `Bibite` and a random number below
/// `int.MaxValue` (`Random.Range(0, int.MaxValue)`), the custom text, or none.
fn template_tag(tagging: settings::Tagging, custom: &str, species_name: Option<String>, rng: &mut Rng) -> Option<String> {
    match tagging {
        settings::Tagging::NoTagging => None,
        settings::Tagging::SpeciesTagging => species_name,
        settings::Tagging::RandomTagging => Some(format!("Bibite{}", rng.next_u64() % i32::MAX as u64)),
        settings::Tagging::CustomTagging => Some(custom.to_owned()),
    }
}

/// Bounding radius registered for eggs in the spatial hash (eggs are small; the
/// exact collider lives in the physics bridge, the hash only serves queries).
const EGG_HASH_RADIUS: f32 = 0.5;

#[inline]
fn id_key(id: BibiteId) -> (u32, u32) {
    (id.handle().slot(), id.handle().generation())
}

#[inline]
fn pellet_key(id: PelletId) -> (u32, u32) {
    (id.handle().slot(), id.handle().generation())
}

/// Sorted per-bibite Red Death accumulator (`TryAdd` + reset-on-entry).
fn acc_entry(vec: &mut Vec<(BibiteId, f32)>, id: BibiteId, init: f32) -> &mut f32 {
    match vec.binary_search_by_key(&id_key(id), |(other, _)| id_key(*other)) {
        Ok(index) => &mut vec[index].1,
        Err(index) => {
            vec.insert(index, (id, init));
            &mut vec[index].1
        }
    }
}

fn remove_acc(vec: &mut Vec<(BibiteId, f32)>, id: BibiteId) {
    if let Ok(index) = vec.binary_search_by_key(&id_key(id), |(other, _)| id_key(*other)) {
        vec.remove(index);
    }
}

fn pellet_acc_entry(vec: &mut Vec<(PelletId, f32)>, id: PelletId, init: f32) -> &mut f32 {
    match vec.binary_search_by_key(&pellet_key(id), |(other, _)| pellet_key(*other)) {
        Ok(index) => &mut vec[index].1,
        Err(index) => {
            vec.insert(index, (id, init));
            &mut vec[index].1
        }
    }
}

fn remove_pellet_acc(vec: &mut Vec<(PelletId, f32)>, id: PelletId) {
    if let Ok(index) = vec.binary_search_by_key(&pellet_key(id), |(other, _)| pellet_key(*other)) {
        vec.remove(index);
    }
}

fn acc_entry_f32(vec: &mut Vec<(BibiteId, f32)>, id: BibiteId) -> &mut f32 {
    acc_entry(vec, id, 0.0)
}

/// Converts a runtime [`ZoneId`] to the save-file zone id shape
/// (`generation 0`, slot = zone list index) for [`WorldContents`] tools.
fn raw_zone_id(zones: &ZoneStore, runtime: Option<ZoneId>) -> Option<ZoneId> {
    runtime
        .and_then(|zone| zones.index_of(zone))
        .and_then(|index| u32::try_from(index).ok())
        .map(|index| ZoneId::from_handle(Handle::new(index, 0)))
}

/// Remaps save-shape (`generation 0`) attached-zone ids in `tools` to runtime
/// zone ids. Real runtime ids (generation != 0) are left untouched.
fn remap_tool_attachments(tools: &mut ToolStore, zones: &ZoneStore) {
    for index in 0..tools.len() {
        let Some(tool_id) = ToolStore::tool_id_at(index) else {
            continue;
        };
        let Some(tool) = tools.get_mut(tool_id) else {
            continue;
        };
        let base = tool.base_mut();
        if let Some(zone) = base.attached_zone {
            if zone.handle().generation() == 0 {
                base.attached_zone = zones.zones.get(zone.handle().slot() as usize).map(|z| z.id);
            }
        }
    }
}

/// Parses a `Zone(<id>).<field>` animator key.
fn parse_zone_setting_key(key: &str) -> Option<(u32, &str)> {
    let rest = key.strip_prefix("Zone(")?;
    let (id, field) = rest.split_once(").")?;
    id.parse::<u32>().ok().map(|id| (id, field))
}

/// Applies one animated zone setting (C# field names) to the runtime config.
fn apply_zone_setting(config: &mut crate::zone::ZoneConfig, field: &str, value: f32) -> bool {
    match field {
        "fertility" => config.fertility = value,
        "biomassDensity" => config.biomass_density = value,
        "pelletSize" => config.pellet_size = value,
        "speed" => config.speed = value,
        "posX" => config.pos_x = value,
        "posY" => config.pos_y = value,
        "radiusRelative" => config.radius_relative = value,
        "radiusAbsolute" => config.radius_absolute = value,
        "widthRelative" => config.width_relative = value,
        "widthAbsolute" => config.width_absolute = value,
        "heightRelative" => config.height_relative = value,
        "heightAbsolute" => config.height_absolute = value,
        "insideRadius" => config.inside_radius = value,
        _ => return false,
    }
    true
}

/// `SettingsChanger` evaluation (spec 04 §7.1): start gate, non-repeat end,
/// then the spline factor. Returns `None` while inactive/ended.
fn animator_factor(
    changer: &SettingsChanger,
    time: f32,
    started: &mut bool,
    ended: &mut bool,
) -> Option<f32> {
    let t = time - changer.start;
    if !*started {
        if t < 0.0 {
            return None;
        }
        *started = true;
    } else if !changer.repeat {
        let last = changer.points.last()?.x;
        if t > last {
            *ended = true;
            return None;
        }
    }
    if t < 0.0 {
        *started = false;
        return None;
    }
    let factor = spline_factor(&changer.points, t, changer.repeat);
    if factor.is_nan() { None } else { Some(factor) }
}

/// `SplineHelper.FindValueAtX` (what the animator editor's graph draws): the curve's value at `x`; before the first point it is that point's
/// value, after the last one the last point's value (a repeating curve starts over).
#[must_use]
pub fn find_value_at_x(points: &[SplinePoint], x: f32, repeat: bool) -> f32 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return 1.0;
    };
    if x < first.x {
        return first.y;
    }
    let mut x = x;
    if x >= last.x {
        if !repeat || last.x <= first.x {
            return last.y;
        }
        x = (x - first.x) % (last.x - first.x) + first.x;
    }
    let mut index = 0usize;
    while index + 1 < points.len() - 1 && x > points[index + 1].x {
        index += 1;
    }
    let span = points[index + 1].x - points[index].x;
    if span <= 0.0 {
        return points[index].y;
    }
    interpolate_segment(&points[index], &points[index + 1], (x - points[index].x) / span)
}

/// `SettingsChanger.FactorAtTime` + `SplineHelper.Interpolate` y component.
fn spline_factor(points: &[SplinePoint], time: f32, repeat: bool) -> f32 {
    if points.is_empty() {
        return 1.0;
    }
    if points.len() < 2 {
        return points[0].y;
    }
    let first = points[0].x;
    let last = points[points.len() - 1].x;
    let mut t = time;
    if repeat && t > last {
        let span = last - first;
        t = if span > 0.0 {
            first + (t - first) % span
        } else {
            first
        };
    }
    let mut index = 0usize;
    while index + 1 < points.len() && t >= points[index + 1].x {
        index += 1;
    }
    if index + 1 >= points.len() {
        // The game would index out of range exactly at/after the last point;
        // clamp to the last segment instead (documented divergence).
        index = points.len() - 2;
    }
    let span = points[index + 1].x - points[index].x;
    if span <= 0.0 {
        return points[index].y;
    }
    let frac = (t - points[index].x) / span;
    interpolate_segment(&points[index], &points[index + 1], frac)
}

fn segment_curve(p1: &SplinePoint, p2: &SplinePoint) -> InterpolationType {
    let flag = !settings::approximately(p1.ease_factor, 1.0)
        && matches!(
            p1.interpolation,
            InterpolationType::EaseOut | InterpolationType::EaseInOut
        );
    let flag2 = !settings::approximately(p2.ease_factor, 1.0)
        && matches!(
            p2.interpolation,
            InterpolationType::EaseIn | InterpolationType::EaseInOut
        );
    match (flag, flag2) {
        (true, true) => InterpolationType::EaseInOut,
        (true, false) => InterpolationType::EaseIn,
        (false, true) => InterpolationType::EaseOut,
        (false, false) => InterpolationType::Linear,
    }
}

/// `SplineHelper.Interpolate(SplinePoint, SplinePoint, t)` y component.
fn interpolate_segment(p1: &SplinePoint, p2: &SplinePoint, t: f32) -> f32 {
    let curve = segment_curve(p1, p2);
    let eased = if curve == InterpolationType::EaseInOut
        && !settings::approximately(t, 0.0)
        && !settings::approximately(t, 1.0)
    {
        let num2 =
            (p1.ease_factor - 1.0).abs() * p2.ease_factor * (1.0 - t).powf(p2.ease_factor - 1.0);
        let num3 = (p2.ease_factor - 1.0).abs() * p1.ease_factor * t.powf(p1.ease_factor - 1.0);
        let denominator = num2 + num3;
        if denominator.abs() <= 1e-12 {
            t
        } else {
            t.powf(p1.ease_factor) * num2 / denominator
                + (1.0 - (1.0 - t).powf(p2.ease_factor)) * num3 / denominator
        }
    } else {
        match curve {
            InterpolationType::Linear => t,
            InterpolationType::EaseIn => t.powf(p1.ease_factor),
            InterpolationType::EaseOut => 1.0 - (1.0 - t).powf(p2.ease_factor),
            _ => t,
        }
    };
    p1.y * (1.0 - eased) + p2.y * eased
}

/// One per-bibite sensing result: id, optional senses/steering result and an
/// optional refreshed FOV seen-cache from a lookup tick.
pub(crate) type SensingOutcome = (
    BibiteId,
    Option<(SenseOutputs, Option<SteeringInputs>)>,
    Option<FovSeenCache>,
);

/// True when the body tick will perform a brain update that consumes world
/// senses for this bibite (sense refresh, FOV lookup, or pheromone sensing due).
#[inline]
#[cfg(test)]
fn bibite_senses_due(record: &crate::body::BibiteRecord, settings: &SimSettings) -> bool {
    if record.body.dead || !record.is_ready {
        return false;
    }
    senses_due_from(
        &record.throttle,
        record.brain.needs_pheromone_sensing(),
        settings,
    )
}

/// The cadence part of [`bibite_senses_due`] for a bibite that is alive and ready: its throttle counters and whether its brain
/// asks for pheromone sensing.
fn senses_due_from(
    throttle: &crate::body::SenseThrottle,
    needs_pheromones: bool,
    settings: &SimSettings,
) -> bool {
    let brain_tick = throttle.brain_counter + 1 >= settings.brain_tps.max(1);
    let cadence = FovCadence::from_counters(
        brain_tick,
        throttle.lookup_counter,
        throttle.sense_counter,
        settings.vision_lookup_updates,
        settings.vision_sense_updates,
    );
    // The pheromone sense is computed in every tick of a bibite that uses it (the game's `PherosenseAround` runs at the end of every fixed update); the vision only on brain ticks.
    (brain_tick && (cadence.lookup_due || cadence.sense_due)) || needs_pheromones
}

/// Immutable snapshot of the stores the sensing pass reads.
///
/// The stores are moved out of [`crate::world::World`] for the duration of [`World::step_sensing`]
/// so the parallel sensing jobs can own an `Arc` snapshot without any `unsafe`
/// or lifetime tricks, then moved back and the buffered results applied in dense
/// slot order on the main thread. This also guarantees no store mutation can
/// race with the workers by construction.
pub(crate) struct SensingData {
    pub(crate) bibites: BibiteStore,
    pub(crate) pellets: PelletStore,
    pub(crate) pheromones: PheromoneStore,
    pub(crate) hash: SpatialHash,
    pub(crate) settings: SimSettings,
    pub(crate) red_death: RedDeathBloom,
}

/// One compute_senses result: optional senses/steering and optional cache refresh.
pub(crate) type SensingResult = Option<(
    Option<(SenseOutputs, Option<SteeringInputs>)>,
    Option<FovSeenCache>,
)>;

/// Computes one bibite's world senses (serial and parallel paths use exactly
/// this function, so both produce bit-identical results).
pub(crate) fn compute_senses(
    data: &SensingData,
    id: BibiteId,
    scratch: &mut QueryScratch,
) -> SensingResult {
    let bibites = &data.bibites;
    let record = bibites.get(id)?;
    if record.body.dead || !record.is_ready {
        return None;
    }
    let brain_tps = data.settings.brain_tps.max(1);
    let throttle = record.throttle;
    let brain_tick = throttle.brain_counter + 1 >= brain_tps;
    let cadence = FovCadence::from_counters(
        brain_tick,
        throttle.lookup_counter,
        throttle.sense_counter,
        data.settings.vision_lookup_updates,
        data.settings.vision_sense_updates,
    );
    let phero_needed = record.brain.needs_pheromone_sensing();
    if !cadence.lookup_due && !cadence.sense_due && !phero_needed {
        return None;
    }

    // Lookup tick: refresh the cached seen-entity list (game `FindSeenEntities`).
    // Sense ticks read this cache; a freshly loaded save has an empty cache.
    let cache_update = if cadence.lookup_due {
        Some(fov_lookup_cache(data, record, scratch))
    } else {
        None
    };

    let mut result = None;
    if cadence.sense_due || phero_needed {
        let mut senses = bibites.senses(id).copied().unwrap_or_default();
        let mut steering = None;
        if cadence.sense_due {
            let cache = cache_update.as_ref().unwrap_or(&record.fov_seen);
            let (fov_senses, fov_steering) = fov_senses_from_cache(data, record, cache);
            senses = fov_senses;
            steering = fov_steering;
        }
        if phero_needed {
            let up = crate::math::unity::transform_up(record.rot);
            let phero_radius = record.genome.gene(Gene::PheroSense).max(0.0);
            if data.pheromones.is_empty() && !data.red_death.enabled {
                delayed_phero(&mut senses, &senses::PheroSenses::default(), brain_tick);
            } else {
                let mut accumulator = senses::PheroAccumulator::new(record.pos);
                if !data.pheromones.is_empty() {
                    let spot_hits = data.hash.query_circle_unsorted(
                        record.pos,
                        phero_radius,
                        ClassMask::PHEROMONE,
                        scratch,
                    );
                    for hit in spot_hits.hits {
                        let spot_id = crate::ids::PheromoneId::from_handle(hit.handle);
                        let Some(view) = data.pheromones.get(spot_id) else {
                            continue;
                        };
                        accumulator.add_spot(view.pos, view.heading, view.strength);
                    }
                }
                if data.red_death.enabled {
                    accumulator.add_red_death(data.red_death.safe_radius, phero_radius);
                }
                let phero = accumulator.finish(up);
                delayed_phero(&mut senses, &phero, brain_tick);
            }
        }
        result = Some((senses, steering));
    }
    Some((result, cache_update))
}

/// The game's order for the pheromone sense: a brain tick reads the values the fixed update before wrote (`Pherosense.pheroSum1..` and the angles are set at the end of every
/// fixed update, from the pose and the pheromones of that moment), and this tick's own values wait for the next one. A bibite that has not sensed yet (just loaded) reads zeros,
/// as a freshly loaded game does.
fn delayed_phero(senses: &mut SenseOutputs, phero: &senses::PheroSenses, brain_tick: bool) {
    if brain_tick {
        senses.apply_pending_phero();
    }
    senses.set_pending_phero(phero);
}

/// `FieldOfView.FindSeenEntities`: overlap-query candidates only (the FOV cone
/// filter and per-object weights are applied later by `ComputeSenses`).
fn fov_lookup_cache(
    data: &SensingData,
    record: &crate::body::BibiteRecord,
    scratch: &mut QueryScratch,
) -> FovSeenCache {
    let params = FovParams {
        view_radius: record.genome.gene(Gene::ViewRadius) * record.derived.d1_size,
        view_angle: record.genome.gene(Gene::ViewAngle),
        herd_separation_distance: record.genome.gene(Gene::HerdSeparationDistance)
            * record.derived.d1_size,
        separation_weight: record.genome.gene(Gene::HerdSeparationWeight),
        cohesion_weight: record.genome.gene(Gene::HerdCohesionWeight),
        alignment_weight: record.genome.gene(Gene::HerdAlignmentWeight),
    };
    let mask = FovMask::from_brain(&record.brain);
    let mut cache = FovSeenCache::default();
    if !params.is_enabled() || !mask.any() {
        return cache;
    }
    let want_pellets = (mask.plant || mask.meat) && !data.pellets.is_empty();
    let want_bibites = mask.bibite && data.bibites.len() > 1;
    if !want_pellets && !want_bibites {
        return cache;
    }
    let mut class_mask = ClassMask::NONE;
    if want_pellets {
        class_mask = class_mask.union(ClassMask::PELLET);
    }
    if want_bibites {
        class_mask = class_mask.union(ClassMask::BIBITE);
    }
    // `FieldOfView.cs:233-251`: one overlap query fills a single 128-slot
    // buffer shared by pellets and bibites. Views narrower than 225 degrees
    // use an oriented box ahead of the bibite; wider views use a circle of
    // `R + min(0.25R, 25)`.
    let radius = params.view_radius;
    let margin = (0.25 * radius).min(25.0);
    let half = params.view_angle / 2.0 * (std::f32::consts::PI / 180.0);
    let hits = if half < std::f32::consts::PI * 5.0 / 8.0 {
        let behind = (radius * half.cos()).min(0.0);
        let width = 2.0
            * radius
            * if params.view_angle > 180.0 {
                1.0
            } else {
                half.sin()
            };
        let size = Vec2::new(width + 2.0 * margin, radius - behind + 2.0 * margin);
        let up = crate::math::unity::transform_up(record.rot);
        let center = record.pos + up * ((radius + behind) / 2.0);
        data.hash
            .query_rotated_box(center, size * 0.5, record.rot, class_mask, scratch)
    } else {
        data.hash
            .query_circle_unsorted(record.pos, radius + margin, class_mask, scratch)
    };
    for hit in hits.hits {
        match hit.class {
            SpatialClass::Pellet => cache.pellets.push(PelletId::from_handle(hit.handle)),
            SpatialClass::Bibite => {
                let other = BibiteId::from_handle(hit.handle);
                if other != record.id {
                    cache.bibites.push(other);
                }
            }
            _ => {}
        }
    }
    cache
}

/// `FieldOfView.ComputeSenses` from the cached lookup list; live stores are
/// resolved here (game object refs), so positions/radii/health are current.
fn fov_senses_from_cache(
    data: &SensingData,
    record: &crate::body::BibiteRecord,
    cache: &FovSeenCache,
) -> (SenseOutputs, Option<SteeringInputs>) {
    let mut senses = data.bibites.senses(record.id).copied().unwrap_or_default();
    let up = crate::math::unity::transform_up(record.rot);
    let mut steering = None;
    let params = FovParams {
        view_radius: record.genome.gene(Gene::ViewRadius) * record.derived.d1_size,
        view_angle: record.genome.gene(Gene::ViewAngle),
        herd_separation_distance: record.genome.gene(Gene::HerdSeparationDistance)
            * record.derived.d1_size,
        separation_weight: record.genome.gene(Gene::HerdSeparationWeight),
        cohesion_weight: record.genome.gene(Gene::HerdCohesionWeight),
        alignment_weight: record.genome.gene(Gene::HerdAlignmentWeight),
    };
    let mask = FovMask::from_brain(&record.brain);
    if params.is_enabled() && mask.any() {
        let mut sensor = senses::FovSensor::new(record.pos, up, params, mask);
        if mask.plant || mask.meat {
            for pellet in &cache.pellets {
                let Some(view) = data.pellets.get(*pellet) else {
                    continue;
                };
                // `FieldOfView.PelletIsHeld` compares the pellet with `held[i] = mouth.links[i].attachedRigidbody.transform`: a joint's `attachedRigidbody` is the body the
                // joint is ON (the holder), not the one it is connected to, so no pellet ever matches and a held pellet stays in the holder's view (measured 2026-11-18: a
                // shrek holding a plant pellet in front of its mouth reads PlantCloseness 1.0 in the game and 0.0 with the pellet left out).
                if view.amount < 0.01 {
                    continue;
                }
                match view.material {
                    MaterialKind::Plant if mask.plant => {
                        sensor.add_plant(view.pos, view.radius, view.amount);
                    }
                    MaterialKind::Meat if mask.meat => {
                        sensor.add_meat(view.pos, view.radius, view.amount);
                    }
                    _ => {}
                }
            }
        }
        if mask.bibite {
            for other in &cache.bibites {
                if *other == record.id {
                    continue;
                }
                let Some(view) = data.bibites.view(*other) else {
                    continue;
                };
                if view.dead {
                    sensor.add_corpse(
                        view.pos,
                        view.side_radius(),
                        view.base_body_area,
                        view.health_ratio,
                    );
                }
                sensor.add_bibite(
                    view.pos,
                    view.side_radius(),
                    view.up(),
                    view.max_health,
                    view.color,
                    view.is_ready,
                );
            }
        }
        let fov = sensor.finish();
        senses.apply_fov(&fov);
        steering = Some(SteeringInputs::from_fov(&fov.herd));
    }
    (senses, steering)
}

impl World {
    /// What the field of view of bibite `id` sees now, object by object with its weight: the lists the game's `FieldOfView` keeps for
    /// `FieldOfViewDisplay`'s labels ("Show Targets"). The candidates are the bibite's cached lookup and the test and weight functions are the ones
    /// `fov_senses_from_cache` runs on them (an empty pellet, and a bibite that is the subject itself, are left out the same way; a held pellet stays in);
    /// nothing of the simulation changes. Empty while the bibite is dead or its brain uses no field of view input.
    #[must_use]
    pub fn fov_targets(&self, id: BibiteId) -> Option<senses::FovTargets> {
        use senses::SeenTarget;
        let record = self.bibites.get(id)?;
        let mut targets = senses::FovTargets::default();
        let params = FovParams {
            view_radius: record.genome.gene(Gene::ViewRadius) * record.derived.d1_size,
            view_angle: record.genome.gene(Gene::ViewAngle),
            herd_separation_distance: record.genome.gene(Gene::HerdSeparationDistance)
                * record.derived.d1_size,
            separation_weight: record.genome.gene(Gene::HerdSeparationWeight),
            cohesion_weight: record.genome.gene(Gene::HerdCohesionWeight),
            alignment_weight: record.genome.gene(Gene::HerdAlignmentWeight),
        };
        let mask = FovMask::from_brain(&record.brain);
        if record.body.dead || !params.is_enabled() || !mask.any() {
            return Some(targets);
        }
        let up = crate::math::unity::transform_up(record.rot);
        let weight_of = |pos: Vec2, radius: f32, what: senses::WeightOf| -> Option<f32> {
            let view = senses::target_in_view(record.pos, up, &params, pos, radius)?;
            Some(senses::fov_weight_of(
                view.distance,
                view.angle_deg,
                &params,
                what,
            ))
        };
        if mask.plant || mask.meat {
            for pellet in &record.fov_seen.pellets {
                let Some(view) = self.pellets.get(*pellet) else {
                    continue;
                };
                if view.amount < 0.01 {
                    continue;
                }
                let list = match view.material {
                    MaterialKind::Plant if mask.plant => &mut targets.plants,
                    MaterialKind::Meat if mask.meat => &mut targets.meat,
                    _ => continue,
                };
                let size_term = senses::pellet_size_term(view.amount);
                if let Some(weight) = weight_of(view.pos, view.radius, senses::WeightOf::Pellet(size_term)) {
                    list.push(SeenTarget {
                        key: crate::body::pellet_tag(*pellet),
                        pos: view.pos,
                        scale: size_term.sqrt(),
                        weight,
                    });
                }
            }
        }
        if mask.bibite {
            for other in &record.fov_seen.bibites {
                if *other == id {
                    continue;
                }
                let Some(view) = self.bibites.view(*other) else {
                    continue;
                };
                let key = crate::body::bibite_tag(*other);
                if view.dead {
                    let corpse = senses::WeightOf::Corpse {
                        base_body_area: view.base_body_area,
                        health_ratio: view.health_ratio,
                    };
                    if let Some(weight) = weight_of(view.pos, view.side_radius(), corpse) {
                        targets.meat.push(SeenTarget {
                            key,
                            pos: view.pos,
                            scale: view.d1_size,
                            weight,
                        });
                    }
                }
                let size = senses::WeightOf::Bibite {
                    max_health: view.max_health,
                };
                if let Some(weight) = weight_of(view.pos, view.side_radius(), size) {
                    targets.bibites.push(SeenTarget {
                        key,
                        pos: view.pos,
                        scale: view.d1_size,
                        weight,
                    });
                }
            }
        }
        Some(targets)
    }
}

/// Checkpoint extension tag: zone runtime state (task-103).
pub const CHECKPOINT_ZONE_TAG: u16 = 0x0103;
/// Checkpoint extension tag: bibite-store arena/runtime bytes.
pub const CHECKPOINT_BIBITE_TAG: u16 = 0x0101;
/// Checkpoint extension tag: egg-store arena/runtime bytes.
pub const CHECKPOINT_EGG_TAG: u16 = 0x0102;
/// Checkpoint extension tag: species-registry runtime state.
pub const CHECKPOINT_SPECIES_TAG: u16 = 0x0104;
/// Checkpoint extension tag: `SimStats` counters.
pub const CHECKPOINT_STATS_TAG: u16 = 0x0105;
/// Checkpoint extension tag: spawner coroutine rows/accumulator.
pub const CHECKPOINT_SPAWNER_TAG: u16 = 0x0106;
/// Checkpoint extension tag: recorded `state_fingerprint` guard.
pub const CHECKPOINT_FINGERPRINT_TAG: u16 = 0x0107;
/// Checkpoint extension tag: pellet-store arena/runtime bytes.
pub const CHECKPOINT_PELLET_TAG: u16 = 0x0108;
/// Checkpoint extension tag: pheromone-store arena/runtime bytes.
pub const CHECKPOINT_PHEROMONE_TAG: u16 = 0x0109;
/// Checkpoint extension tag: tool runtime bytes.
pub const CHECKPOINT_TOOLS_TAG: u16 = 0x010A;
/// Checkpoint extension tag: exact spatial-hash layout.
pub const CHECKPOINT_HASH_TAG: u16 = 0x010B;
/// Checkpoint extension tag: the zones' logs (the species Position tab's history).
pub const CHECKPOINT_ZONE_LOG_TAG: u16 = 0x010D;

fn cp_push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn cp_push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn cp_push_f32(out: &mut Vec<u8>, value: f32) {
    cp_push_u32(out, value.to_bits());
}
fn cp_push_vec2(out: &mut Vec<u8>, value: Vec2) {
    cp_push_f32(out, value.x);
    cp_push_f32(out, value.y);
}
fn cp_push_zone(out: &mut Vec<u8>, zone: Option<ZoneId>) {
    match zone {
        Some(zone) => {
            out.push(1);
            cp_push_u32(out, zone.handle().slot());
            cp_push_u32(out, zone.handle().generation());
        }
        None => out.push(0),
    }
}

fn capture_tools_checkpoint(tools: &crate::tools::ToolStore) -> Vec<u8> {
    use crate::tools::Tool;
    let mut out = Vec::new();
    cp_push_u32(&mut out, tools.len() as u32);
    for tool in tools.tools() {
        match tool {
            Tool::Rad(tower) => {
                out.push(0);
                cp_push_u64(&mut out, tower.base.save_id);
                cp_push_vec2(&mut out, tower.base.pos);
                cp_push_zone(&mut out, tower.base.attached_zone);
                cp_push_f32(&mut out, tower.progress);
            }
            Tool::Phero(tower) => {
                out.push(1);
                cp_push_u64(&mut out, tower.base.save_id);
                cp_push_vec2(&mut out, tower.base.pos);
                cp_push_zone(&mut out, tower.base.attached_zone);
                cp_push_f32(&mut out, tower.heading);
                cp_push_f32(&mut out, tower.red);
                cp_push_f32(&mut out, tower.green);
                cp_push_f32(&mut out, tower.blue);
                match tower.spot {
                    Some(spot) => {
                        out.push(1);
                        cp_push_u32(&mut out, spot.handle().slot());
                        cp_push_u32(&mut out, spot.handle().generation());
                    }
                    None => out.push(0),
                }
            }
            Tool::Killer(killer) => {
                out.push(2);
                cp_push_u64(&mut out, killer.base.save_id);
                cp_push_vec2(&mut out, killer.base.pos);
                cp_push_zone(&mut out, killer.base.attached_zone);
                cp_push_f32(&mut out, killer.progress);
                cp_push_u32(&mut out, killer.killed);
            }
        }
    }
    out
}

fn restore_tools_checkpoint(
    tools: &mut crate::tools::ToolStore,
    bytes: &[u8],
) -> crate::error::Result<()> {
    use crate::tools::Tool;
    let mut reader = CheckpointReader::new(bytes);
    let count = reader.u32()? as usize;
    if count != tools.len() {
        return Err(crate::error::Error::invalid_data(format!(
            "checkpoint has {count} tools, world has {}",
            tools.len()
        )));
    }
    for _ in 0..count {
        let tag = reader.u8()?;
        let save_id = reader.u64()?;
        let tool = tools
            .tools_mut()
            .iter_mut()
            .find(|tool| tool.save_id() == save_id)
            .ok_or_else(|| {
                crate::error::Error::invalid_data(format!(
                    "checkpoint tool {save_id} not present in world"
                ))
            })?;
        match (tag, tool) {
            (0, Tool::Rad(tower)) => {
                tower.base.pos = reader.vec2()?;
                tower.base.attached_zone = reader.zone()?;
                tower.progress = reader.f32()?;
            }
            (1, Tool::Phero(tower)) => {
                tower.base.pos = reader.vec2()?;
                tower.base.attached_zone = reader.zone()?;
                tower.heading = reader.f32()?;
                tower.red = reader.f32()?;
                tower.green = reader.f32()?;
                tower.blue = reader.f32()?;
                tower.spot = if reader.u8()? == 1 {
                    Some(crate::ids::PheromoneId::from_handle(
                        crate::arena::Handle::new(reader.u32()?, reader.u32()?),
                    ))
                } else {
                    None
                };
            }
            (2, Tool::Killer(killer)) => {
                killer.base.pos = reader.vec2()?;
                killer.base.attached_zone = reader.zone()?;
                killer.progress = reader.f32()?;
                killer.killed = reader.u32()?;
            }
            _ => {
                return Err(crate::error::Error::invalid_data(
                    "checkpoint tool variant/id mismatch",
                ));
            }
        }
    }
    Ok(())
}

/// Conservative encoded-payload estimate used to reject 100k-scale worlds
/// before allocating their arena checkpoints (~4 KiB/bibite measured at 5k/10k).
fn checkpoint_payload_estimate(bibites: usize, pellets: usize) -> usize {
    bibites
        .saturating_mul(4096)
        .saturating_add(pellets.saturating_mul(64))
        .saturating_add(1 << 20)
}

/// Bounds-checked little-endian reader for checkpoint extension payloads.
struct CheckpointReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> CheckpointReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, len: usize) -> crate::error::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| crate::error::Error::invalid_data("checkpoint length overflow"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| crate::error::Error::invalid_data("truncated checkpoint section"))?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> crate::error::Result<u8> {
        Ok(self.take(1)?[0])
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

    fn zone(&mut self) -> crate::error::Result<Option<ZoneId>> {
        if self.u8()? == 1 {
            Ok(Some(ZoneId::from_handle(crate::arena::Handle::new(
                self.u32()?,
                self.u32()?,
            ))))
        } else {
            Ok(None)
        }
    }
}

/// Cumulative runtime-only conservation terms for pellet matter.
///
/// These are never serialized and never influence the simulation; they are the
/// Rust side of `docs/design/FEEDING_METRICS.md` §3. Energy terms are in E,
/// matter terms in u². Body-owned corpse/rip/attack meat creation is folded
/// into `produced_meat_energy` by the body/egg/physics phase hooks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConservationTotals {
    /// Energy created by initial zone seeding (plant).
    pub seeded_plant_energy: f64,
    /// Energy created by initial zone seeding (meat).
    pub seeded_meat_energy: f64,
    /// Energy added by zone production (plant).
    pub produced_plant_energy: f64,
    /// Energy added by zone production (meat).
    pub produced_meat_energy: f64,
    /// Energy removed by over-cap recycling (plant).
    pub recycled_plant_energy: f64,
    /// Energy removed by over-cap recycling (meat).
    pub recycled_meat_energy: f64,
    /// Energy removed by pellet decay (plant).
    pub decayed_plant_energy: f64,
    /// Energy removed by pellet decay (meat).
    pub decayed_meat_energy: f64,
    /// Energy removed by out-of-bounds culling (plant).
    pub culled_plant_energy: f64,
    /// Energy removed by out-of-bounds culling (meat).
    pub culled_meat_energy: f64,
    /// Energy removed by explicit `World::despawn_pellet` (plant).
    pub removed_plant_energy: f64,
    /// Energy removed by explicit `World::despawn_pellet` (meat).
    pub removed_meat_energy: f64,
    /// Plant matter consumed by bibites (body `FeedingCounters` deltas).
    pub consumed_plant_u2: f64,
    /// Meat matter consumed by bibites.
    pub consumed_meat_u2: f64,
    /// Plant consumed energy (`consumed_plant_u2 × plant density`).
    pub consumed_plant_energy: f64,
    /// Meat consumed energy.
    pub consumed_meat_energy: f64,
}

impl ConservationTotals {
    /// Plant energy residual over an interval:
    /// `Δstock − (Δseeded + Δproduced − Δconsumed − Δdecayed − Δculled −
    /// Δremoved − Δrecycled)`. `created_meat` is not part of the plant identity.
    #[must_use]
    pub fn plant_energy_residual(&self, start: &Self, stock_start: f64, stock_end: f64) -> f64 {
        let produced = (self.seeded_plant_energy - start.seeded_plant_energy)
            + (self.produced_plant_energy - start.produced_plant_energy);
        let removed = (self.consumed_plant_energy - start.consumed_plant_energy)
            + (self.decayed_plant_energy - start.decayed_plant_energy)
            + (self.culled_plant_energy - start.culled_plant_energy)
            + (self.removed_plant_energy - start.removed_plant_energy)
            + (self.recycled_plant_energy - start.recycled_plant_energy);
        (stock_end - stock_start) - (produced - removed)
    }

    /// Meat energy residual (diagnostic; corpse/meat creation is not captured).
    #[must_use]
    pub fn meat_energy_residual(&self, start: &Self, stock_start: f64, stock_end: f64) -> f64 {
        let produced = (self.seeded_meat_energy - start.seeded_meat_energy)
            + (self.produced_meat_energy - start.produced_meat_energy);
        let removed = (self.consumed_meat_energy - start.consumed_meat_energy)
            + (self.decayed_meat_energy - start.decayed_meat_energy)
            + (self.culled_meat_energy - start.culled_meat_energy)
            + (self.removed_meat_energy - start.removed_meat_energy)
            + (self.recycled_meat_energy - start.recycled_meat_energy);
        (stock_end - stock_start) - (produced - removed)
    }
}

/// Snapshot of conservation counters + live pellet stock energy at a sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConservationSnapshot {
    /// Cumulative counters at the sample.
    pub totals: ConservationTotals,
    /// Live plant pellet energy (`PelletCounts::plant_biomass`).
    pub stock_plant_energy: f64,
    /// Live meat pellet energy.
    pub stock_meat_energy: f64,
    /// `created_meat` is not tracked yet (corpse path is body-owned).
    pub terms_missing: bool,
}

/// Result of a template placement (cursor spawn).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateSpawn {
    /// Template placed as a live bibite at the requested growth.
    Bibite(BibiteId),
    /// Template placed as an incubating egg (game `GrowthAtSpawn::Egg`).
    Egg(EggId),
}

fn normalize_tag(tag: Option<&str>) -> Option<String> {
    tag.map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
}

fn summary_entry<'a>(summaries: &'a mut Vec<TagSummary>, tag: &str) -> &'a mut TagSummary {
    if let Some(index) = summaries.iter().position(|summary| summary.tag == tag) {
        return &mut summaries[index];
    }
    summaries.push(TagSummary {
        tag: tag.to_owned(),
        ..TagSummary::default()
    });
    summaries.last_mut().expect("summary just pushed")
}

/// Aggregated live-bibite/egg group for one species tag (`TagsManager`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagSummary {
    /// Tag string.
    pub tag: String,
    /// Live bibites carrying the tag.
    pub bibites: u32,
    /// Incubating eggs carrying the tag.
    pub eggs: u32,
    /// Sum of body+stored energy and egg energy (game `BibiteTag.energy`).
    pub energy: f32,
}

/// In-world challenge session state (`ChallengeManager`/`ChallengeTrackerPanel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChallengeState {
    /// Still running; checks happen once per simulated second.
    Running,
    /// Exit condition met (or explicit end).
    Ended,
}

/// Outcome of an ended challenge (`ChallengeEndPopup`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChallengeOutcome {
    /// Still running.
    Running,
    /// Ended with at least one star.
    Completed,
    /// Ended with zero stars.
    Failed,
}

/// High-score record for a challenge (`ChallengeScoreInfo`); wall-clock
/// timestamp is intentionally represented as simulated completion time.
#[derive(Clone, Debug, PartialEq)]
pub struct ChallengeScoreInfo {
    /// Challenge name/key.
    pub challenge_name: String,
    /// Stars attained (0..=3).
    pub star_attained: u8,
    /// Final scoring-metric value.
    pub score: f32,
    /// Simulated time at completion (s).
    pub elapsed_s: f64,
    /// Champion template display name.
    pub champion_name: String,
    /// Champion template path.
    pub champion_path: String,
}

impl ChallengeScoreInfo {
    /// `ChallengesProgress.NameToKey`: the key under which a challenge's best
    /// result is stored (spaces removed, lower case).
    #[must_use]
    pub fn name_to_key(name: &str) -> String {
        name.replace(' ', "").to_lowercase()
    }

    /// `ChallengesProgress.TrySetHighScoreOfChallenge` decision (cs:54-89): a
    /// result replaces the stored best only if it is not NaN, does not have
    /// fewer stars than the stored best, and (when a stored score exists) is
    /// strictly better according to `highIsBetter`. `other` is the stored best,
    /// `None` when the challenge was never completed.
    #[must_use]
    pub fn beats(&self, other: Option<&Self>, high_is_better: bool) -> bool {
        if self.score.is_nan() {
            return false;
        }
        let Some(other) = other else {
            return true;
        };
        if other.star_attained > self.star_attained {
            return false;
        }
        if other.score.is_nan() {
            return true;
        }
        if high_is_better {
            self.score > other.score
        } else {
            self.score < other.score
        }
    }
}

/// Running challenge session: parameters, timer, score, stars, outcome and
/// champion identity for the viewer's tracker/end popup.
#[derive(Clone, Debug, PartialEq)]
pub struct ChallengeSession {
    /// Challenge definition.
    pub parameters: ChallengeParameters,
    /// Display name (`challengeName`).
    pub challenge_name: String,
    /// Description text.
    pub description: String,
    /// Star description strings shown by the tracker.
    pub star_descriptions: [String; 3],
    /// Champion template display name.
    pub champion_name: String,
    /// Champion template path.
    pub champion_path: String,
    /// Simulated time when the challenge started.
    pub started_time: f64,
    /// Elapsed simulated seconds since start.
    pub elapsed_s: f64,
    /// Current score (`scoringMetric.Evaluate`).
    pub score: f32,
    /// Current stars (0..=3).
    pub stars: u8,
    /// Running/ended state.
    pub state: ChallengeState,
    /// Final outcome once ended.
    pub outcome: ChallengeOutcome,
    /// Game checks once per simulated second.
    check_accumulator: f32,
}

impl ChallengeSession {
    /// Creates a running session starting at `started_time` (usually
    /// `world.simulated_time` when the scenario starts).
    #[must_use]
    pub fn new(
        parameters: ChallengeParameters,
        challenge_name: impl Into<String>,
        description: impl Into<String>,
        champion_name: impl Into<String>,
        champion_path: impl Into<String>,
        started_time: f64,
    ) -> Self {
        Self {
            parameters,
            challenge_name: challenge_name.into(),
            description: description.into(),
            star_descriptions: [String::new(), String::new(), String::new()],
            champion_name: champion_name.into(),
            champion_path: champion_path.into(),
            started_time,
            elapsed_s: 0.0,
            score: 0.0,
            stars: 0,
            state: ChallengeState::Running,
            outcome: ChallengeOutcome::Running,
            check_accumulator: 0.0,
        }
    }

    /// Evaluates score/stars/exit immediately.
    pub fn update(&mut self, world: &World) {
        if self.state == ChallengeState::Ended {
            return;
        }
        self.elapsed_s = (world.simulated_time - self.started_time).max(0.0);
        self.score = world.challenge_metric(&self.parameters.scoring_metric);
        self.stars = world.challenge_stars(&self.parameters);
        let exit = self
            .parameters
            .exit_condition
            .evaluate(&|metric, arg| match metric {
                // The game starts a challenge in a fresh world, so its `Time`
                // metric counts from the challenge start.
                SimMetric::Time => (world.simulated_time - self.started_time).max(0.0) as f32,
                _ => world.metric_value(metric, arg),
            });
        if exit {
            self.state = ChallengeState::Ended;
            self.outcome = if self.stars > 0 {
                ChallengeOutcome::Completed
            } else {
                ChallengeOutcome::Failed
            };
        }
    }

    /// Advances the once-per-second tracker check. Returns true on the tick the
    /// challenge transitions to ended.
    pub fn tick(&mut self, world: &World, dt: f32) -> bool {
        if self.state == ChallengeState::Ended || dt <= 0.0 {
            return false;
        }
        self.check_accumulator += dt;
        let mut ended_now = false;
        while self.check_accumulator >= 1.0 {
            self.check_accumulator -= 1.0;
            self.update(world);
            if self.state == ChallengeState::Ended {
                ended_now = true;
                break;
            }
        }
        ended_now
    }

    /// Builds the final high-score record.
    #[must_use]
    pub fn score_info(&self) -> ChallengeScoreInfo {
        ChallengeScoreInfo {
            challenge_name: self.challenge_name.clone(),
            star_attained: self.stars,
            score: self.score,
            elapsed_s: self.elapsed_s,
            champion_name: self.champion_name.clone(),
            champion_path: self.champion_path.clone(),
        }
    }
}

/// Hatches kept for [`World::take_hatched_children`] when nothing takes them.
const HATCH_LOG_CAP: usize = 1024;

/// The canonical world container.
///
/// Holds every store the simulation phases touch; [`World::tick`] is being
/// extended phase-by-phase (W4 adds the body/physics phases).
pub struct World {
    /// Effective simulation settings (authoritative at runtime).
    pub settings: SimSettings,
    /// Simulation bounds and shade constants.
    pub bounds: WorldBounds,
    /// Zones (ordered; save/processing order).
    pub zones: ZoneStore,
    /// Bibites (body/organ/brain state; populated after construction by loaders
    /// or the spawner).
    pub bibites: BibiteStore,
    /// Eggs.
    pub eggs: EggStore,
    /// Pellets.
    pub pellets: PelletStore,
    /// Pheromone spots.
    pub pheromones: PheromoneStore,
    /// Spatial hash (bibites/pellets/pheromone spots/eggs).
    pub hash: SpatialHash,
    /// Scenario world tools (towers, colour killers) and the Red Death bloom.
    pub tools: ToolStore,
    /// Species registry (owned by the world; populated as bibites spawn).
    pub species: SpeciesRegistry,
    /// Scenario name as stored in the scene (save round-trip metadata).
    pub scenario_name: String,
    /// Scenario version string (save round-trip metadata).
    pub scenario_version: String,
    /// Scenario tags (save round-trip metadata).
    pub tags: Vec<String>,
    /// Bibite spawn rows (template paths + counts; the world W4 spawner consumes
    /// them, io reads them back for save round-trip).
    pub bibite_spawns: Vec<SpawnRow>,
    /// Zone group definitions the world was built from (scenario round-trip).
    pub zone_groups: Vec<settings::ZoneGroupSettings>,
    /// Settings animators (`settingsChangers`; drive phase 2 of the tick).
    pub settings_changers: Vec<SettingsChanger>,
    /// Decoded templates available to the spawner (`TemplateSource` contract).
    pub templates: Vec<TemplateSource>,
    /// Spawner cadence accumulator (1 s scaled coroutine in the game).
    pub spawner_accumulator: f32,
    /// Per-row `spawnProgress` for the fresh-scenario spawner coroutine.
    spawn_progress: Vec<f32>,
    /// Species founded by each fresh-scenario spawn row (descendant counts).
    spawn_row_species: Vec<Option<crate::ids::SpeciesId>>,
    /// RNG seed the world was constructed with (io supplies it on load).
    pub seed: u64,
    /// Opaque `data.bin` history blob (M1 pass-through; core never parses it).
    pub data_bin: Vec<u8>,
    /// Fixed-tick clock.
    pub clock: FixedClock,
    /// Simulated time in seconds (save `simulatedTime`; advanced by the clock).
    pub simulated_time: f64,
    /// World-level RNG stream (derived from the run seed).
    pub rng: Rng,
    /// Material decay period in ticks (`decayTPS`).
    pub decay_tps: u32,
    /// Statistics collector (parity §6.1 schema).
    pub(crate) stats: SimStats,
    /// Multi-resolution biomass/count history for the Graphs panel
    /// (`DataLogger`). Not part of checkpoints or the state fingerprint.
    pub data_logger: crate::datalog::DataLogger,
    /// Runtime-only cumulative pellet conservation counters.
    accounting: ConservationTotals,
    /// Previous per-bibite feeding counters (slot → (generation, counters)).
    feeding_prev: Vec<Option<(u32, crate::body::FeedingCounters)>>,
    /// Feeding-counter deltas since the last stats sample.
    pub(crate) feeding_interval: FeedingInterval,
    /// Per-tick phase wall-time breakdown (observational only).
    phase_timings: PhaseTimings,
    /// Requested sensing worker count (1 = serial).
    threads: usize,
    /// Persistent sensing pool (`None` while serial, and during the ticks of a world too small for the pool: with no handle to it
    /// no stage can hand work over, so every stage runs inline).
    sensing_pool: Option<ThreadPool>,
    /// Whether the pool may be used: [`World::tick`] sets it from the way the tick runs (inline or on the pool, see `pool_tuner`);
    /// true until the first tick, for the stages the tests call by themselves.
    pool_wanted: bool,
    /// Sizes from which each stage of the tick runs on the worker pool (the gates the world was given: the tuner's levels of the pool are made from them).
    parallel_gates: crate::sim::ParallelGates,
    /// The sizes the tick that runs (or ran last) goes by: those of `parallel_gates` or of the level of the pool the tick runs at.
    active_gates: crate::sim::ParallelGates,
    /// Chooses, by timing the world's own ticks, whether a tick runs inline or on the pool ([`World::tick`]); idle in a world of one
    /// thread and with gates that are not `adaptive`.
    pool_tuner: PoolTuner,
    /// Reused per-worker query scratch for parallel sensing.
    sensing_worker_scratch: Vec<QueryScratch>,
    /// Reused per-chunk result buffers.
    sensing_chunks: Vec<Vec<SensingOutcome>>,
    /// Reused list of bibites whose senses are due this tick.
    sensing_due_ids: Vec<BibiteId>,
    /// Reused list of the bibites the store removed since the last hash sync.
    bibite_removed_scratch: Vec<BibiteId>,
    /// `true` once any live/spawned bibite has `meta.is_source`; lets the
    /// pellet-producer phase skip its full population scan in source-free
    /// worlds (the synthetic benchmarks).
    has_sources: bool,
    /// Persistent result channel for parallel sensing jobs.
    sensing_result_tx: Sender<(usize, QueryScratch, Vec<SensingOutcome>)>,
    sensing_result_rx: Receiver<(usize, QueryScratch, Vec<SensingOutcome>)>,
    decay: DecayThrottle,
    // ---- W4 per-tick scratch (allocation-free after warm-up) ----
    bibite_scratch: BibiteScratch,
    events: Vec<BibiteEvent>,
    zone_circles: Vec<ZoneCircle>,
    senses_scratch: Vec<SensingOutcome>,
    query_scratch: QueryScratch,
    hatched: Vec<EggId>,
    /// Eggs that hatched, with the bibite each became (`EggHatching.newBornBody`), until a viewer
    /// takes them; bounded so unwatched runs do not grow it.
    hatched_children: Vec<(EggId, BibiteId)>,
    /// Natural births of a bibite whose brain has 100 hidden nodes or more since a viewer took the count (`ACH_BIBITE_EINSTEIN`).
    einstein_births: u32,
    aborted: Vec<(EggId, f32)>,
    egg_positions: Vec<(EggId, Vec2, f32)>,
    rad_targets: Vec<BibiteTarget>,
    color_candidates: Vec<ColorCandidate>,
    red_bibite_acc: Vec<(BibiteId, f32)>,
    red_pellet_acc: Vec<(PelletId, f32)>,
    producer_progress: Vec<(BibiteId, f32)>,
    ids_scratch: Vec<BibiteId>,
    pellet_ids_scratch: Vec<PelletId>,
    /// The pellet slots one hash sync visits (reused).
    pellet_dirty_scratch: Vec<u32>,
    animator_progress: f32,
    animator_started: Vec<bool>,
    animator_ended: Vec<bool>,
}

impl World {
    /// Creates an empty ecology world with game-default settings, then replaces
    /// the settings-derived state.
    ///
    /// Prefer [`World::new`] for scenario/load construction; this low-level
    /// constructor stays for tests and for callers that build stores manually.
    pub fn new_ecology(bounds: WorldBounds, globals: ZoneGlobals, seed: u64) -> Self {
        let extent = bounds.shade_end() + HASH_EXTENT_MARGIN;
        let hash = SpatialHash::new(
            Vec2::splat(-extent),
            Vec2::splat(extent),
            default_cell_size(bounds.sim_size()),
        );
        let (sensing_result_tx, sensing_result_rx) = mpsc::channel();
        let mut world = Self {
            settings: SimSettings::game_defaults(),
            bounds,
            zones: ZoneStore::new(bounds, globals, seed),
            bibites: BibiteStore::new(),
            eggs: EggStore::new(),
            pellets: PelletStore::new(),
            pheromones: PheromoneStore::new(),
            hash,
            tools: ToolStore::new(bounds.shade_end()),
            species: SpeciesRegistry::new(),
            scenario_name: String::new(),
            scenario_version: String::new(),
            tags: Vec::new(),
            bibite_spawns: Vec::new(),
            zone_groups: Vec::new(),
            settings_changers: Vec::new(),
            templates: Vec::new(),
            spawner_accumulator: 0.0,
            spawn_progress: Vec::new(),
            spawn_row_species: Vec::new(),
            seed,
            data_bin: Vec::new(),
            clock: FixedClock::default_rate(),
            simulated_time: 0.0,
            rng: Rng::derive(seed, domain::WORLD, 0),
            decay_tps: DEFAULT_DECAY_TPS,
            stats: SimStats::new(),
            data_logger: crate::datalog::DataLogger::new(),
            accounting: ConservationTotals::default(),
            feeding_prev: Vec::new(),
            feeding_interval: FeedingInterval::default(),
            phase_timings: PhaseTimings::new(),
            threads: 1,
            sensing_pool: None,
            pool_wanted: true,
            parallel_gates: crate::sim::ParallelGates::calibrated(),
            active_gates: crate::sim::ParallelGates::calibrated(),
            pool_tuner: PoolTuner::new(crate::sim::ParallelGates::calibrated().pool_prior()),
            sensing_worker_scratch: Vec::new(),
            sensing_chunks: Vec::new(),
            sensing_due_ids: Vec::new(),
            bibite_removed_scratch: Vec::new(),
            has_sources: false,
            sensing_result_tx,
            sensing_result_rx,
            decay: DecayThrottle::new(DEFAULT_DECAY_TPS),
            bibite_scratch: BibiteScratch::new(),
            events: Vec::new(),
            zone_circles: Vec::new(),
            senses_scratch: Vec::new(),
            query_scratch: QueryScratch::new(),
            hatched: Vec::new(),
            hatched_children: Vec::new(),
            einstein_births: 0,
            aborted: Vec::new(),
            egg_positions: Vec::new(),
            rad_targets: Vec::new(),
            color_candidates: Vec::new(),
            red_bibite_acc: Vec::new(),
            red_pellet_acc: Vec::new(),
            producer_progress: Vec::new(),
            ids_scratch: Vec::new(),
            pellet_ids_scratch: Vec::new(),
            pellet_dirty_scratch: Vec::new(),
            animator_progress: 0.0,
            animator_started: Vec::new(),
            animator_ended: Vec::new(),
        };
        // Worlds start serial; `World::set_threads` propagates the current
        // setting to the body store's parallel-brain path.
        world.bibites.set_brain_threads(1);
        world
    }

    /// Canonical constructor for a new or loaded world.
    ///
    /// Builds every store from `settings` + `scenario`:
    ///
    /// 1. validates settings and scenario data (invalid data → [`Error`]),
    /// 2. derives `WorldBounds` from `independents.simulationSize` and the zone
    ///    globals from the settings (biomass density, fertility, pellet energy,
    ///    initial seeding, global zone speed, pellet-energy floor),
    /// 3. applies the settings material table to the pellet store,
    /// 4. builds the zone list (explicit zones first, then generated zone
    ///    groups, deterministic RNG from `seed`),
    /// 5. creates tools (rad/phero/killers; phero spots are emitted by the
    ///    first [`World::step_tools`] call),
    /// 6. spawns the scenario's initial pellets (hash-registered),
    /// 7. records scenario metadata (`name`/`version`/`tags`/`bibite_spawns`).
    ///
    /// # Load sequence for `bibites-io`
    ///
    /// Bibite and species content is **not** part of this call (it needs
    /// templates/genome decoding from io). After construction the loader must,
    /// in this order:
    /// 1. `world.bibites.set_settings(settings.clone())` is already done;
    /// 2. spawn saved bibites through the B2 save path (`BibiteStore::spawn_loaded`,
    ///    one `body::BibiteSave` per saved bibite) and eggs through
    ///    `EggStore::spawn_loaded`, then insert their hash entries via
    ///    `World::sync_bibite_hash` / egg equivalents;
    /// 3. restore the species registry through its io constructors
    ///    (`SpeciesRegistry` load accessors) and per-bibite `SpeciesId` links;
    /// 4. for a *new* scenario (not a save), call [`World::initial_seeding`]
    ///    explicitly when `settings.scenario.initial_seeding > 0` (the game seeds
    ///    a fresh world but never re-seeds on load).
    ///
    /// # Errors
    ///
    /// Returns [`Error::invalid_data`] when the settings or any scenario row is
    /// invalid (non-finite numbers, out-of-range zone/killer parameters, invalid
    /// pellet material/amount).
    pub fn new(settings: SimSettings, scenario: ScenarioData, seed: u64) -> Result<Self> {
        settings.validate()?;
        scenario.validate()?;
        let sim_size = settings.independents.simulation_size;
        if !sim_size.is_finite() || sim_size <= 0.0 {
            return Err(Error::invalid_data(
                "simulationSize is not a positive number",
            ));
        }
        let bounds = WorldBounds::new(sim_size);
        let globals = ZoneGlobals::from_settings(&settings);
        let mut world = Self::new_ecology(bounds, globals.clone(), seed);

        // Settings-derived state.
        world.settings = settings.clone();
        world.bibites = BibiteStore::with_settings(settings.clone());
        world.clock = FixedClock::new(settings.sim_tps);
        world.decay_tps = settings.decay_tps;
        world.decay = DecayThrottle::new(settings.decay_tps);
        // Red Death (`RedDeathBloomManager` subscribes to the loaded settings and starts as they say): a world made from a save or a scenario with the bloom
        // switched on has it from its first tick. Only an edit of the settings panel (`apply_settings`) set it before, so a loaded bloom hurt nothing, was
        // not smelled and drew nothing until some setting was touched.
        world.tools.red_death.base_damage = settings.scenario.red_death_base_damages;
        world.tools.red_death.damage_velocity = settings.scenario.red_death_damage_velocity;
        world.tools.red_death.set_enabled(settings.scenario.enable_red_death);
        world.tools.red_death.set_fill(settings.scenario.red_death_fill);

        // Material table from settings (spec §7: settings overrides defaults).
        let mut materials = MaterialTable::default();
        for kind in settings::MaterialKind::ALL {
            let runtime_kind = material_kind_from_settings(kind);
            *materials.get_mut(runtime_kind) =
                material_properties_from_settings(settings.materials.get(kind));
        }
        *world.pellets.materials_mut() = materials;
        world.pellets.recompute_counts();

        // Zones: explicit first, then generated groups.
        let explicit: Vec<crate::zone::ZoneConfig> = scenario
            .zones
            .iter()
            .map(crate::zone::ZoneConfig::from_settings)
            .collect();
        let groups: Vec<crate::zone::ZoneGroupConfig> = scenario
            .zone_groups
            .iter()
            .map(crate::zone::ZoneGroupConfig::from_settings)
            .collect();
        world.zones = ZoneStore::from_scenario(explicit, &groups, bounds, globals, seed);

        // Tools.
        for mut tower in scenario.rad_towers {
            tower.base.loaded = true;
            world.tools.add_rad_tower(tower);
        }
        for mut tower in scenario.phero_towers {
            tower.base.loaded = true;
            world.tools.add_phero_tower(tower);
        }
        for mut killer in scenario.color_killers {
            killer.base.loaded = true;
            world.tools.add_color_killer(killer);
        }

        // Initial pellets (material amount/pos already validated).
        for spawn in scenario.initial_pellets {
            world
                .spawn_pellet(spawn)
                .ok_or_else(|| Error::invalid_data("failed to spawn an initial pellet"))?;
        }
        world.pellets.recompute_counts();

        // Scenario round-trip metadata.
        world.scenario_name = scenario.name;
        world.scenario_version = scenario.version;
        world.tags = scenario.tags;
        world.bibite_spawns = scenario.bibite_spawns;
        // The zones a group generated live in the zone store (each tagged with its group); the group keeps no copy of them.
        world.zone_groups = scenario.zone_groups;
        for group in &mut world.zone_groups {
            group.zones.clear();
        }
        world.settings_changers = scenario.settings_changers;
        world
            .animator_started
            .resize(world.settings_changers.len(), false);
        world
            .animator_ended
            .resize(world.settings_changers.len(), false);
        Ok(world)
    }

    /// Creates a world around an already-built zone store (legacy load helper).
    ///
    /// Prefer [`World::new`]; this helper keeps game-default settings around the
    /// supplied zone store.
    pub fn from_zone_store(bounds: WorldBounds, zones: ZoneStore, seed: u64) -> Self {
        assert_eq!(
            bounds.sim_size(),
            zones.bounds.sim_size(),
            "zone store bounds must match world bounds"
        );
        let globals = zones.globals.clone();
        let mut world = Self::new_ecology(bounds, globals, seed);
        world.zones = zones;
        world
    }

    /// World-scope zone globals (TEMP model; settings land in W3).
    #[inline]
    pub fn globals(&self) -> &ZoneGlobals {
        &self.zones.globals
    }

    // -----------------------------------------------------------------------
    // Save/load accessor surface (io I4; no serde in core)
    // -----------------------------------------------------------------------

    /// Reads the world-owned scene metadata for `scene.bb8scene` (save path).
    #[must_use]
    pub fn scene(&self) -> WorldScene {
        WorldScene {
            name: self.scenario_name.clone(),
            version: self.scenario_version.clone(),
            simulated_time: self.simulated_time,
            tags: self.tags.clone(),
            bibite_spawns: self.bibite_spawns.clone(),
        }
    }

    /// Restores world-owned scene metadata (load path); does not touch entities.
    pub fn restore_scene(&mut self, scene: WorldScene) {
        self.scenario_name = scene.name;
        self.scenario_version = scene.version;
        self.simulated_time = scene.simulated_time;
        self.tags = scene.tags;
        self.bibite_spawns = scene.bibite_spawns;
    }

    /// Simulated time in seconds (`simulatedTime` round-trip).
    #[inline]
    pub fn simulated_time(&self) -> f64 {
        self.simulated_time
    }

    /// Sets simulated time (load path).
    #[inline]
    pub fn set_simulated_time(&mut self, seconds: f64) {
        self.simulated_time = seconds;
    }

    /// Replaces the whole zone store (load path; keeps the world's settings).
    pub fn set_zones(&mut self, zones: ZoneStore) {
        assert_eq!(
            zones.bounds.sim_size(),
            self.bounds.sim_size(),
            "zone store bounds must match world bounds"
        );
        self.zones = zones;
    }

    /// Replaces the whole tool store (load path; phero spots are re-created by
    /// the next [`Self::step_tools`] call).
    pub fn set_tools(&mut self, tools: ToolStore) {
        self.tools = tools;
        self.sync_pheromone_hash();
    }

    /// Species registry accessor (io read path).
    #[inline]
    pub fn species_registry(&self) -> &SpeciesRegistry {
        &self.species
    }

    /// Replaces the species registry (load path).
    #[inline]
    pub fn set_species_registry(&mut self, registry: SpeciesRegistry) {
        self.species = registry;
    }

    /// Load path for one saved egg: spawns it and registers its hash entry.
    pub fn spawn_loaded_egg(&mut self, save: crate::body::EggSave) -> EggId {
        let id = self.eggs.spawn_loaded(save);
        self.register_egg(id);
        id
    }

    /// Registers an egg in the spatial hash from its current store record.
    pub fn register_egg(&mut self, id: EggId) -> bool {
        let Some(egg) = self.eggs.get(id) else {
            return false;
        };
        self.hash
            .insert(id.handle(), SpatialClass::Egg, egg.pos, EGG_HASH_RADIUS);
        true
    }

    /// Registers a pellet in the spatial hash from its current store record.
    pub fn register_pellet(&mut self, id: PelletId) -> bool {
        let Some(view) = self.pellets.get(id) else {
            return false;
        };
        self.hash
            .insert(id.handle(), SpatialClass::Pellet, view.pos, view.radius);
        true
    }

    /// Registers a pheromone spot in the spatial hash from its current record.
    pub fn register_pheromone(&mut self, id: crate::ids::PheromoneId) -> bool {
        let Some(view) = self.pheromones.get(id) else {
            return false;
        };
        let radius = pheromone_spot_radius(self.pheromones.owner_tool(id).flatten().is_some());
        self.hash
            .insert(id.handle(), SpatialClass::Pheromone, view.pos, radius);
        true
    }

    /// Rebuilds every spatial-hash class from the stores (after a bulk load).
    pub fn rebuild_hashes(&mut self) {
        self.sync_bibite_hash();
        self.sync_pellet_hash_full();
        self.sync_egg_hash();
        self.sync_pheromone_hash();
    }

    /// Reads every serialized world section into plain core types (io I4 save
    /// path). No serde is involved; `bibites-io` maps these to the file JSON.
    #[must_use]
    pub fn to_contents(&self) -> WorldContents {
        // `ScenarioSettings.zones` (the explicit zones) and each group's own: a group keeps the zones it generated.
        let zones: Vec<settings::ZoneSettings> = self
            .zones
            .zones
            .iter()
            .filter(|zone| zone.group.is_none())
            .map(|zone| zone.config.0.clone())
            .collect();
        let zone_groups: Vec<settings::ZoneGroupSettings> = self
            .zone_groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let mut group = group.clone();
                group.zones = self
                    .zones
                    .zones
                    .iter()
                    .filter(|zone| zone.group == Some(index as u32))
                    .map(|zone| zone.config.0.clone())
                    .collect();
                group
            })
            .collect();
        let mut rad_towers = Vec::new();
        let mut phero_towers = Vec::new();
        let mut color_killers = Vec::new();
        for tool in self.tools.tools() {
            match tool {
                crate::tools::Tool::Rad(tower) => {
                    let mut tower = *tower;
                    tower.base.attached_zone = raw_zone_id(&self.zones, tower.base.attached_zone);
                    tower.base.loaded = false; // a view state, not part of the save
                    rad_towers.push(tower);
                }
                crate::tools::Tool::Phero(tower) => {
                    let mut tower = *tower;
                    tower.spot = None; // runtime spot ids are not serialized
                    tower.base.attached_zone = raw_zone_id(&self.zones, tower.base.attached_zone);
                    tower.base.loaded = false;
                    phero_towers.push(tower);
                }
                crate::tools::Tool::Killer(killer) => {
                    let mut killer = *killer;
                    killer.base.attached_zone = raw_zone_id(&self.zones, killer.base.attached_zone);
                    killer.base.loaded = false;
                    color_killers.push(killer);
                }
            }
        }
        let mut pellets = Vec::new();
        for id in self.pellets.ids() {
            let Some(view) = self.pellets.get(id) else {
                continue;
            };
            let owner_zone = view
                .zone
                .and_then(|zone| self.zones.index_of(zone))
                .map(|index| index as u32);
            let fresh_time_remaining = if self.pellets.is_decaying(id) {
                self.pellets.fresh_time_remaining(id)
            } else {
                None
            };
            pellets.push(PelletRecord {
                material: view.material,
                amount: view.amount,
                pos: view.pos,
                owner_zone,
                held: view.held,
                fresh_time_remaining,
                rot_amount: self.pellets.rot_amount(id).unwrap_or(0.0),
            });
        }
        let mut pheromones = Vec::new();
        for id in self.pheromones.ids() {
            if let Some(view) = self.pheromones.get(id) {
                pheromones.push(PheromoneRecord {
                    pos: view.pos,
                    heading: view.heading,
                    strength: view.strength,
                    dissipate: view.dissipate,
                    owner_tool: view.owner_tool,
                });
            }
        }
        let (bibite_ids, bibites): (Vec<BibiteId>, Vec<BibiteSave>) = self
            .bibites
            .ids()
            .filter_map(|id| self.bibites.to_save(id).map(|save| (id, save)))
            .unzip();
        let (egg_ids, eggs): (Vec<EggId>, Vec<EggSave>) = self
            .eggs
            .ids()
            .filter_map(|id| self.eggs.to_save(id).map(|save| (id, save)))
            .unzip();
        // The parent links as positions in the saved list (a parent that is gone leaves no link).
        let slot_of: std::collections::HashMap<BibiteId, u32> = bibite_ids
            .iter()
            .enumerate()
            .map(|(slot, id)| (*id, slot as u32))
            .collect();
        let links = |meta: &crate::genome::GenomeMeta| -> [Option<u32>; 2] {
            let first = meta.parent1.and_then(|id| slot_of.get(&id).copied());
            let second = first.and_then(|_| meta.parent2.and_then(|id| slot_of.get(&id).copied()));
            [first, second]
        };
        let bibite_parents: Vec<[Option<u32>; 2]> = bibite_ids
            .iter()
            .map(|id| {
                self.bibites
                    .get(*id)
                    .map_or([None, None], |record| links(&record.meta))
            })
            .collect();
        let egg_parents: Vec<[Option<u32>; 2]> = egg_ids
            .iter()
            .map(|id| {
                self.eggs
                    .get(*id)
                    .map_or([None, None], |record| links(&record.meta))
            })
            .collect();
        WorldContents {
            settings: self.settings.clone(),
            seed: self.seed,
            scene: self.scene(),
            zones,
            zone_groups,
            settings_changers: self.settings_changers.clone(),
            rad_towers,
            phero_towers,
            color_killers,
            pellets,
            pheromones,
            bibites,
            eggs,
            bibite_parents,
            egg_parents,
            species: Some(self.species.to_records()),
            data_bin: self.data_bin_for_save(),
            img_png: None,
        }
    }

    /// Builds a complete world from [`WorldContents`] (io I4 load path).
    ///
    /// Call order: validate every section → [`World::new`]-equivalent stores
    /// (zones/tools/settings, **no seeding**) → scene metadata → pellets (spawn +
    /// decay state) → pheromone spots → bibites → eggs → species registry →
    /// zone recount → spatial-hash rebuild.
    ///
    /// # Errors
    ///
    /// Returns [`Error::invalid_data`] for invalid settings/zones/spawn rows,
    /// pellets with non-world materials/bad amounts/out-of-range owner index,
    /// non-finite pheromone data, or body-save errors.
    pub fn from_contents(contents: WorldContents) -> Result<Self> {
        let WorldContents {
            settings,
            seed,
            scene,
            zones,
            zone_groups,
            settings_changers,
            rad_towers,
            phero_towers,
            color_killers,
            pellets,
            pheromones,
            bibites,
            eggs,
            bibite_parents,
            egg_parents,
            species,
            data_bin,
            img_png: _,
        } = contents;
        settings.validate()?;
        for zone in &zones {
            zone.validate()?;
        }
        for group in &zone_groups {
            group.validate()?;
        }
        for changer in &settings_changers {
            changer.validate()?;
        }
        // A group saved without its zones has them generated, after the others: a pellet can belong to those as well.
        let zone_total = zones.len()
            + zone_groups
                .iter()
                .map(|group| {
                    if group.zones.is_empty() {
                        group.count(settings.independents.simulation_size) as usize
                    } else {
                        group.zones.len()
                    }
                })
                .sum::<usize>();
        for pellet in &pellets {
            if !pellet.material.is_world_pellet() {
                return Err(Error::invalid_data("saved pellet has non-world material"));
            }
            if !pellet.amount.is_finite() || pellet.amount <= 0.0 {
                return Err(Error::invalid_data("saved pellet has invalid amount"));
            }
            if !pellet.pos.x.is_finite() || !pellet.pos.y.is_finite() {
                return Err(Error::invalid_data("saved pellet position is not finite"));
            }
            if pellet
                .owner_zone
                .is_some_and(|index| index as usize >= zone_total)
            {
                return Err(Error::invalid_data(
                    "saved pellet owner zone index is out of range",
                ));
            }
        }
        for spot in &pheromones {
            if !spot.pos.x.is_finite() || !spot.pos.y.is_finite() {
                return Err(Error::invalid_data(
                    "saved pheromone position is not finite",
                ));
            }
            if spot.strength.iter().any(|value| !value.is_finite()) {
                return Err(Error::invalid_data(
                    "saved pheromone strength is not finite",
                ));
            }
        }
        let scenario = ScenarioData {
            name: scene.name.clone(),
            version: scene.version.clone(),
            zones,
            zone_groups,
            bibite_spawns: scene.bibite_spawns.clone(),
            initial_pellets: Vec::new(),
            rad_towers,
            phero_towers,
            color_killers,
            settings_changers,
            tags: scene.tags.clone(),
        };
        let mut world = Self::new(settings, scenario, seed)?;
        {
            let (tools, zones) = (&mut world.tools, &world.zones);
            remap_tool_attachments(tools, zones);
        }
        world.restore_scene(scene);
        world.data_bin = data_bin;
        for pellet in pellets {
            let owner_zone = pellet
                .owner_zone
                .map(|index| world.zones.zones[index as usize].id);
            let spawn = PelletSpawn {
                material: pellet.material,
                amount: pellet.amount,
                pos: pellet.pos,
                velocity: Vec2::ZERO,
                rotation: 0.0,
                angular_velocity: 0.0,
                owner_zone,
                held: pellet.held,
            };
            let id = world
                .spawn_pellet(spawn)
                .ok_or_else(|| Error::invalid_data("saved pellet could not be spawned"))?;
            if let Some(fresh) = pellet.fresh_time_remaining {
                world.pellets.set_decay_state(id, fresh, pellet.rot_amount);
            }
        }
        world.zones.recount_all(&world.pellets);
        for spot in pheromones {
            let emission = PheroEmission {
                pos: spot.pos,
                heading: spot.heading,
                strength: spot.strength,
                dissipate: spot.dissipate,
                owner_tool: spot.owner_tool,
            };
            if let Some(id) = world.pheromones.emit(emission) {
                world.register_pheromone(id);
            }
        }
        let mut bibite_ids: Vec<BibiteId> = Vec::with_capacity(bibites.len());
        for mut save in bibites {
            // `BibiteGenes.LoadState` ends with `CapGenes()`.
            save.genome.cap();
            bibite_ids.push(world.spawn_loaded_bibite(save)?);
        }
        let mut egg_ids: Vec<EggId> = Vec::with_capacity(eggs.len());
        for mut save in eggs {
            save.genome.cap();
            egg_ids.push(world.spawn_loaded_egg(save));
        }
        // The parent links: positions in the saved list become the ids the bibites were given.
        let resolve = |slots: &[Option<u32>; 2]| -> (Option<BibiteId>, Option<BibiteId>) {
            let first = slots[0].and_then(|slot| bibite_ids.get(slot as usize).copied());
            let second = first
                .and_then(|_| slots[1].and_then(|slot| bibite_ids.get(slot as usize).copied()));
            (first, second)
        };
        for (id, slots) in bibite_ids.iter().zip(&bibite_parents) {
            let (first, second) = resolve(slots);
            if let Some(record) = world.bibites.get_mut(*id) {
                record.meta.parent1 = first;
                record.meta.parent2 = second;
            }
        }
        for (id, slots) in egg_ids.iter().zip(&egg_parents) {
            let (first, second) = resolve(slots);
            if let Some(record) = world.eggs.get_mut(*id) {
                record.meta.parent1 = first;
                record.meta.parent2 = second;
            }
        }
        if let Some(record) = species {
            world.species = SpeciesRegistry::from_records(record, SPECIES_LOG_PERIOD_SECONDS)?;
            world.load_species_logs();
            // `EggHatching.ResumeHatch` -> `CheckSpeciesOfEgg`: a loaded egg takes the first active species within the span (else the nearest).
            for id in &egg_ids {
                let Some(egg) = world.eggs.get(*id) else {
                    continue;
                };
                let (genome, brain) = (egg.genome.clone(), egg.brain.clone());
                if let Some(species) = world.species.check_species_of_egg(&genome, &brain, &world.settings)
                    && let Some(egg) = world.eggs.get_mut(*id)
                {
                    egg.meta.species = Some(species);
                }
            }
        }
        world.load_zone_logs();
        world.load_logger_blocks();
        // A loaded save starts with live pellet stock; seed the conservation
        // ledger with it so the identity `initial + produced = current + ...`
        // holds for Ring/save-loaded worlds (fresh scenarios get this from
        // `seed_scenario_world`).
        let counts = world.pellets.counts();
        world.accounting.seeded_plant_energy += counts.plant_biomass;
        world.accounting.seeded_meat_energy += counts.meat_biomass;
        world.rebuild_hashes();
        Ok(world)
    }

    // -----------------------------------------------------------------------
    // Bibite spawner / templates (spec §8.3)
    // -----------------------------------------------------------------------

    /// Registers decoded templates for the spawner (io path; `TemplateSource`
    /// is the core-neutral contract).
    pub fn set_templates(&mut self, templates: Vec<TemplateSource>) {
        self.templates = templates;
    }

    fn template_for_path(&self, path: &str) -> Option<&TemplateSource> {
        self.templates
            .iter()
            .find(|template| Self::template_name_matches(path, &template.name))
    }

    /// Whether a template named `name` (a file name, with or without its extension) is what a spawn row with `path` resolves to: the first template for which
    /// this holds is the row's. The row's path is a bundled file name or an absolute path of the game's own machine, so it may carry either separator; an empty
    /// path names nothing. A caller that loads templates from a folder can leave out the ones that no row's path matches: they can never be found.
    #[must_use]
    pub fn template_name_matches(path: &str, name: &str) -> bool {
        if path.is_empty() {
            return false;
        }
        let stem = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .trim_end_matches(".bb8template")
            .trim_end_matches(".bb8");
        name == path
            || name == stem
            || name
                .trim_end_matches(".bb8template")
                .trim_end_matches(".bb8")
                == stem
    }

    /// Spawns one bibite from a template at `pos` (virgin-spawn path, spec
    /// §8.3): applies growth-at-spawn, tagging and initial orientation.
    ///
    /// # Errors
    ///
    /// Returns an error if the body store rejects the spawn state.
    pub fn spawn_bibite_from_template(
        &mut self,
        template: &TemplateSource,
        growth_at_spawn: GrowthAtSpawn,
        tagging: Tagging,
        tag: &str,
        pos: Vec2,
        randomize: RandomizeGenes,
    ) -> Result<BibiteId> {
        // The tag is set below, once the species is known (`StartGenesFromTemplate`: species name / random / custom / none).
        let mut meta = template.meta.clone();
        meta.species_tag = None;
        // `WorldObjectsSpawner.SpawnBibiteFromTemplate` (cs:148-176): AllGenes
        // randomises every gene (keeping the template generation),
        // NormalMutations runs one `Mutate()` pass, Color re-rolls the colour.
        // The brain is copied unmutated.
        let mut genome = template.genome.clone();
        // `BibiteGenes.StartGenesFromTemplate`: `CapGenes()` (a template may carry genes past their editor range: an average brain mutation of 16 is 10).
        genome.cap();
        match randomize {
            RandomizeGenes::No => {}
            RandomizeGenes::AllGenes => genome.randomize_in_place(&mut self.rng),
            RandomizeGenes::Color => genome.randomize_color(&mut self.rng),
            RandomizeGenes::NormalMutations => {
                let (mutated, _) = genome.child(
                    &meta,
                    &mut self.rng,
                    &self.settings,
                    &MutationFactor::default(),
                );
                genome = mutated;
            }
        }
        // `WorldObjectsSpawner`: `CopyBrain(template.nodes, template.synapses, mutate: false)`: a template's saved node values are not kept.
        let mut template_brain = template.brain.clone();
        template_brain.reset_runtime_state(self.settings.brain_period());
        let mut spawn = BibiteSpawn::new(genome, meta, template_brain);
        spawn.vitals = SpawnVitals::from_growth_at_spawn(growth_at_spawn);
        spawn.pos = pos;
        spawn.rot = self.rng.f32() * TAU;
        // The size a growth mode asks for comes from the TEMPLATE's genes (cs:182-188), however the spawned genes were randomised.
        let id = self.bibites.spawn_with_growth_basis(spawn, &template.genome)?;
        if let Some(record) = self.bibites.get_mut(id) {
            let genome = record.genome.clone();
            let brain = record.brain.clone();
            let mut meta = record.meta.clone();
            // `BibiteGenes.StartGenesFromTemplate`: `FindSpeciesFromTemplate` - the species the template names (every template names one), founded by the first bibite.
            let species = self.species.find_species_from_template(&template.game_species_name(), "", &genome, &brain, false);
            meta.species = Some(species);
            // The same call sets the tag (`speciesTag = tagStyle switch ...`). The game's AllGenes branch of a live spawn never calls it (cs:163-167: it only copies
            // the generation), so there the original leaves the bibite untagged; the player asked for Random Tag with Random (All) Genes to tag, so this port tags it.
            let species_name = self.species.get(species).map(|found| found.name());
            meta.species_tag = template_tag(tagging, tag, species_name, &mut self.rng);
            record.meta = meta;
            let parent = record.meta.species.unwrap_or(species);
            self.species
                .on_birth(species, parent, self.bibite_spawns.first().map_or(0, |_| 0));
        }
        self.randomize_sense_phase(id);
        self.register_bibite(id);
        self.stats.births_since_sample += 1;
        self.stats.births_total += 1;
        Ok(id)
    }

    /// Spawns a template at a cursor position, with an explicit rotation.
    ///
    /// `angle_radians = None` uses a random orientation like the game. With
    /// [`GrowthAtSpawn::Egg`] this creates an incubating egg using
    /// `LayBodyEnergy` as the initial energy; other modes create a live bibite.
    /// The egg takes the ordinary child mutation of the game's egg spawn (`Mutate((int)genes[9])`); for the other randomise-gene modes use
    /// [`Self::spawn_template_at_randomized`].
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid saved body size or egg spawn.
    pub fn spawn_template_at(
        &mut self,
        template: &TemplateSource,
        pos: Vec2,
        angle_radians: Option<f32>,
        growth_at_spawn: GrowthAtSpawn,
        tagging: Tagging,
        tag: &str,
    ) -> Result<TemplateSpawn> {
        self.spawn_template_at_randomized(template, pos, angle_radians, growth_at_spawn, tagging, tag, RandomizeGenes::NormalMutations)
    }

    /// [`Self::spawn_template_at`] with the spawn row's `randomizeGenes` choice for an EGG (`WorldObjectsSpawner.SpawnEggFromTemplate`,
    /// cs:207-219): AllGenes replaces every gene (`RandomGenes()`), Color re-rolls the colour after the usual `Mutate((int)genes[9])`, and
    /// the brain is mutated whatever the choice. The live (non-egg) modes ignore `randomize`: [`Self::spawn_bibite_from_template`] spawns those.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid saved body size or egg spawn.
    #[allow(clippy::too_many_arguments)] // the game's `SpawnBibiteFromTemplate` takes the same seven
    pub fn spawn_template_at_randomized(
        &mut self,
        template: &TemplateSource,
        pos: Vec2,
        angle_radians: Option<f32>,
        growth_at_spawn: GrowthAtSpawn,
        tagging: Tagging,
        tag: &str,
        randomize: RandomizeGenes,
    ) -> Result<TemplateSpawn> {
        if growth_at_spawn != GrowthAtSpawn::Egg {
            let mut meta = template.meta.clone();
            // The tag is set below, with the species (`StartGenesFromTemplate`).
            meta.species_tag = None;
            let mut capped = template.genome.clone();
            capped.cap();
            let mut template_brain = template.brain.clone();
            template_brain.reset_runtime_state(self.settings.brain_period());
            let mut spawn = BibiteSpawn::new(capped, meta, template_brain);
            spawn.vitals = SpawnVitals::from_growth_at_spawn(growth_at_spawn);
            spawn.pos = pos;
            spawn.rot = angle_radians.unwrap_or_else(|| self.rng.f32() * TAU);
            let id = self.bibites.spawn_with_growth_basis(spawn, &template.genome)?;
            if let Some(record) = self.bibites.get_mut(id) {
                let genome = record.genome.clone();
                let brain = record.brain.clone();
                let mut meta = record.meta.clone();
                // `StartGenesFromTemplate`: the species the template names (the placer's bibites join it, the first founds it, named for the template).
                let species = self.species.find_species_from_template(&template.game_species_name(), "", &genome, &brain, false);
                meta.species = Some(species);
                let species_name = self.species.get(species).map(|found| found.name());
                meta.species_tag = template_tag(tagging, tag, species_name, &mut self.rng);
                record.meta = meta;
                let parent = record.meta.species.unwrap_or(species);
                self.species
                    .on_birth(species, parent, record.meta.generation);
            }
            self.register_bibite(id);
            self.stats.births_since_sample += 1;
            self.stats.births_total += 1;
            return Ok(TemplateSpawn::Bibite(id));
        }
        let mut meta = template.meta.clone();
        // The tag is set below, with the species (`StartGenesFromTemplate` comes first in `SpawnEggFromTemplate`, so even an AllGenes egg is tagged).
        meta.species_tag = None;
        // `SpawnEggFromTemplate`: `Mutate((int)genes[9])` (a fixed count, the average mutation number) and `CopyBrain(..., mutate: true)`;
        // AllGenes calls `RandomGenes()` instead of the mutation, Color re-rolls the colour after it.
        let mut genome = template.genome.clone();
        genome.cap();
        let mut brain = template.brain.clone();
        if randomize == RandomizeGenes::AllGenes {
            genome.randomize_in_place(&mut self.rng);
        } else if !self.settings.scenario.prevent_mutations {
            let count = genome.gene(crate::genome::Gene::AverageMutationNumber) as usize;
            genome.mutate_times(count, &mut self.rng, &self.settings, &MutationFactor::default());
        }
        if randomize == RandomizeGenes::Color {
            genome.randomize_color(&mut self.rng);
        }
        if !self.settings.scenario.prevent_mutations {
            brain.mutate(&mut self.rng, &self.settings, &genome, &MutationFactor::default());
        }
        brain.reset_runtime_state(self.settings.brain_period());
        let upkeep_counts = brain.upkeep();
        let upkeep = upkeep_counts.synapses as f32 * self.settings.scenario.synapse_upkeep_cost
            + upkeep_counts.hidden as f32 * self.settings.scenario.neuron_upkeep_cost;
        let required = crate::organs::egg_required_energy(
            genome.body_energy_at_birth(&self.settings),
            upkeep,
            genome.gene(crate::genome::Gene::SizeRatio),
        );
        let energy = genome.lay_body_energy(&self.settings);
        let hatch_time = genome.gene(crate::genome::Gene::HatchTime);
        let size_ratio = genome.gene(crate::genome::Gene::SizeRatio);
        let _ = size_ratio;
        // The egg's genes come from `StartGenesFromTemplate` too: it is of the template's species.
        let species = self.species.find_species_from_template(&template.game_species_name(), "", &genome, &brain, false);
        meta.species = Some(species);
        let species_name = self.species.get(species).map(|found| found.name());
        meta.species_tag = template_tag(tagging, tag, species_name, &mut self.rng);
        let id = self.eggs.spawn(crate::body::EggSpawn {
            pos,
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome,
            meta,
            brain,
            energy,
            hatch_time,
            total_required_energy: required,
            birth_orientation: -Vec2::Y,
        });
        self.register_egg(id);
        Ok(TemplateSpawn::Egg(id))
    }

    /// Live bibite ids in dense order (UI selection).
    #[must_use]
    pub fn live_bibite_ids(&self) -> Vec<BibiteId> {
        self.bibites.live_ids().collect()
    }

    /// Cycles to the next live bibite for follow/selection.
    #[must_use]
    pub fn next_live_bibite(&self, current: Option<BibiteId>) -> Option<BibiteId> {
        self.bibites.next_live_bibite(current)
    }

    /// Cycles to the previous live bibite.
    #[must_use]
    pub fn previous_live_bibite(&self, current: Option<BibiteId>) -> Option<BibiteId> {
        self.bibites.previous_live_bibite(current)
    }

    /// Oldest live bibite.
    #[must_use]
    pub fn oldest_live_bibite(&self) -> Option<BibiteId> {
        self.bibites.oldest_live_bibite()
    }

    /// Highest-generation live bibite.
    #[must_use]
    pub fn highest_generation_live_bibite(&self) -> Option<BibiteId> {
        self.bibites.highest_generation_live_bibite()
    }

    /// Nearest live bibite to a world position.
    #[must_use]
    pub fn nearest_live_bibite(&self, position: Vec2) -> Option<BibiteId> {
        self.bibites.nearest_live_bibite(position)
    }

    /// Revives a dead corpse (`BibiteBody.Revive`); restores max health/energy
    /// immediately (the game animates the same restoration over ~0.1 s steps).
    pub fn revive_bibite(&mut self, id: BibiteId) -> bool {
        let Some(record) = self.bibites.get_mut(id) else {
            return false;
        };
        if !record.revive() {
            return false;
        }
        self.register_bibite(id)
    }

    /// Lays every completed egg in a bibite's womb now (`LayEgg` UI action).
    /// Newly spawned eggs are registered in the spatial hash; returns the count.
    pub fn force_lay_eggs(&mut self, id: BibiteId) -> u32 {
        let before: Vec<EggId> = self.eggs.ids().collect();
        let laid = {
            let mut ctx = BibiteTickCtx {
                pellets: &mut self.pellets,
                pheromones: &mut self.pheromones,
                eggs: &mut self.eggs,
                zones: &self.zone_circles,
                settings: &self.settings,
                rng: &mut self.rng,
                scratch: &mut self.bibite_scratch,
                events: &mut self.events,
            };
            self.bibites.force_lay_eggs(id, &mut ctx)
        };
        let new_eggs: Vec<EggId> = self
            .eggs
            .ids()
            .filter(|egg| !before.contains(egg))
            .collect();
        for egg in new_eggs {
            self.register_egg(egg);
        }
        laid
    }

    /// Lays one egg for `id` ignoring egg progress (the LAY button / Insert key).
    pub fn lay_egg_once(&mut self, id: BibiteId) -> u32 {
        let before: Vec<EggId> = self.eggs.ids().collect();
        let laid = {
            let mut ctx = BibiteTickCtx {
                pellets: &mut self.pellets,
                pheromones: &mut self.pheromones,
                eggs: &mut self.eggs,
                zones: &self.zone_circles,
                settings: &self.settings,
                rng: &mut self.rng,
                scratch: &mut self.bibite_scratch,
                events: &mut self.events,
            };
            self.bibites.lay_single_egg(id, &mut ctx)
        };
        let new_eggs: Vec<EggId> = self
            .eggs
            .ids()
            .filter(|egg| !before.contains(egg))
            .collect();
        for egg in new_eggs {
            self.register_egg(egg);
        }
        laid
    }

    /// Sets or clears a live bibite's species tag (`TagsManager` edit).
    /// Empty/whitespace tags clear the field.
    pub fn set_bibite_species_tag(&mut self, id: BibiteId, tag: Option<&str>) -> bool {
        let Some(record) = self.bibites.get_mut(id) else {
            return false;
        };
        record.meta.species_tag = normalize_tag(tag);
        true
    }

    /// Sets or clears an incubating egg's species tag.
    pub fn set_egg_species_tag(&mut self, id: EggId, tag: Option<&str>) -> bool {
        let Some(record) = self.eggs.get_mut(id) else {
            return false;
        };
        record.meta.species_tag = normalize_tag(tag);
        true
    }

    /// Live bibites carrying `tag` (simple scan; no separate index yet).
    #[must_use]
    pub fn bibites_with_tag(&self, tag: &str) -> Vec<BibiteId> {
        self.bibites
            .iter()
            .filter(|(_, record)| {
                !record.body.dead && record.meta.species_tag.as_deref() == Some(tag)
            })
            .map(|(id, _)| id)
            .collect()
    }

    /// Incubating eggs carrying `tag`.
    #[must_use]
    pub fn eggs_with_tag(&self, tag: &str) -> Vec<EggId> {
        self.eggs
            .ids()
            .filter(|id| {
                self.eggs
                    .get(*id)
                    .is_some_and(|egg| egg.meta.species_tag.as_deref() == Some(tag))
            })
            .collect()
    }

    /// Sorted distinct tags across live bibites and incubating eggs.
    #[must_use]
    pub fn species_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = Vec::new();
        for (_, record) in self.bibites.iter() {
            if !record.body.dead {
                if let Some(tag) = &record.meta.species_tag {
                    tags.push(tag.clone());
                }
            }
        }
        for id in self.eggs.ids() {
            if let Some(tag) = self
                .eggs
                .get(id)
                .and_then(|egg| egg.meta.species_tag.as_ref())
            {
                tags.push(tag.clone());
            }
        }
        tags.sort_unstable();
        tags.dedup();
        tags
    }

    /// Tag groups with counts and summed energy (`TagsManager` tooltip values).
    #[must_use]
    pub fn tag_summaries(&self) -> Vec<TagSummary> {
        let mut summaries: Vec<TagSummary> = Vec::new();
        let fat_density = self.settings.materials.fat.energy_density;
        for (_, record) in self.bibites.iter() {
            if record.body.dead {
                continue;
            }
            let Some(tag) = &record.meta.species_tag else {
                continue;
            };
            let entry = summary_entry(&mut summaries, tag);
            entry.bibites += 1;
            let extra = record.fat.amount * fat_density
                + record.egg_layer.progress * record.egg_layer.egg_energy;
            entry.energy += record.body.total_energy(extra);
        }
        for id in self.eggs.ids() {
            let Some(egg) = self.eggs.get(id) else {
                continue;
            };
            let Some(tag) = &egg.meta.species_tag else {
                continue;
            };
            let entry = summary_entry(&mut summaries, tag);
            if !egg.hatch.aborting {
                entry.eggs += 1;
                entry.energy += egg.hatch.energy;
            }
        }
        summaries.sort_unstable_by(|a, b| a.tag.cmp(&b.tag));
        summaries
    }

    /// Summary of one tag (champion tracking helper), or `None` when no live
    /// bibite/egg carries it.
    #[must_use]
    pub fn tag_summary(&self, tag: &str) -> Option<TagSummary> {
        self.tag_summaries()
            .into_iter()
            .find(|summary| summary.tag == tag)
    }

    /// One-call save snapshot for the Stats SAVE button. File writing is
    /// intentionally left to `bibites-io`/fmt-dev; this returns the exact
    /// serialized bibite (genome, meta, brain, stomach, body, position).
    #[must_use]
    pub fn bibite_save(&self, id: BibiteId) -> Option<BibiteSave> {
        self.bibites.to_save(id)
    }

    /// Egg equivalent of [`Self::bibite_save`].
    #[must_use]
    pub fn egg_save(&self, id: EggId) -> Option<EggSave> {
        self.eggs.to_save(id)
    }

    /// Template-source view of a live bibite for the Stats EDIT button.
    #[must_use]
    pub fn bibite_template_source(&self, id: BibiteId) -> Option<TemplateSource> {
        let record = self.bibites.get(id)?;
        let name = record
            .meta
            .species_tag
            .clone()
            .filter(|tag| !tag.is_empty())
            .unwrap_or_else(|| format!("Bibite {}", id.handle().slot()));
        Some(TemplateSource::new(
            name,
            record.genome.clone(),
            record.meta.clone(),
            record.brain.clone(),
        ))
    }

    /// Template-source view of an incubating egg for the Stats EDIT button.
    #[must_use]
    pub fn egg_template_source(&self, id: EggId) -> Option<TemplateSource> {
        let egg = self.eggs.get(id)?;
        let name = egg
            .meta
            .species_tag
            .clone()
            .filter(|tag| !tag.is_empty())
            .unwrap_or_else(|| format!("Egg {}", id.handle().slot()));
        Some(TemplateSource::new(
            name,
            egg.genome.clone(),
            egg.meta.clone(),
            egg.brain.clone(),
        ))
    }

    /// Stats-panel snapshot for a live bibite.
    #[must_use]
    pub fn bibite_stats(&self, id: BibiteId) -> Option<crate::body::BibiteStatsView> {
        self.bibites
            .get(id)
            .map(crate::body::BibiteRecord::stats_view)
    }

    /// Stats-panel snapshot for an incubating egg.
    #[must_use]
    pub fn egg_stats(&self, id: EggId) -> Option<crate::body::EggStatsView> {
        self.eggs.get(id).map(|egg| egg.stats_view(&self.settings))
    }

    /// Game `ZoneManager.GetRandomPositionInZone` / `row.zone.GetSpawnLocation`:
    /// picks a zone and samples its **shape-aware** spawn distribution
    /// (`ZoneData::spawn_location`, spec §1.5) instead of a generic disc around
    /// the zone centre. The generic disc was wrong for rectangular/ring zones
    /// (Ring's food strip is a 97,500×2,500 rect whose shape radius is ~48,750),
    /// placing fresh-scenario bibites far outside the pellet field.
    fn spawn_position_for_row(&mut self, spawn_zone: Option<u32>) -> Vec2 {
        if self.zones.zones.is_empty() {
            return Vec2::ZERO;
        }
        let index = spawn_zone
            .and_then(|file_id| {
                self.zones
                    .zones
                    .iter()
                    .position(|zone| zone.config.runtime_zone_id() == file_id)
            })
            .unwrap_or_else(|| (self.rng.next_u64() as usize) % self.zones.zones.len());
        self.zones.zones[index].spawn_location()
    }

    /// Spawns the spawn table's `minimum` population once at world start.
    ///
    /// Returns the number spawned. Rows without a matching template are skipped
    /// (io is expected to call [`Self::set_templates`] first).
    pub fn spawn_initial_population(&mut self) -> usize {
        let mut spawned = 0;
        for row_index in 0..self.bibite_spawns.len() {
            let row = self.bibite_spawns[row_index].clone();
            let already = row.total_spawned.max(0) as usize;
            let target = row.minimum.max(0) as usize;
            if row.path.is_empty() || already >= target {
                continue;
            }
            let Some(template) = self.template_for_path(&row.path).cloned() else {
                continue;
            };
            for _ in already..target {
                let pos = self.spawn_position_for_row(row.spawn_zone);
                if self
                    .spawn_bibite_from_template(
                        &template,
                        row.growth_at_spawn,
                        row.tagging,
                        &row.tag,
                        pos,
                        row.randomize_genes,
                    )
                    .is_ok()
                {
                    spawned += 1;
                    self.bibite_spawns[row_index].total_spawned += 1;
                }
            }
        }
        spawned
    }

    /// Starts the game's fresh-scenario spawner coroutine.
    ///
    /// The game's `BibiteSpawner.SpawnEntity(1f)` waits one frame and then runs
    /// its first iteration immediately (`BibiteSpawner.cs:124-185`), so a fresh
    /// scenario has 1 bibite at t=0 and gains one per simulated second
    /// (`virginSpawnRate` default 1 Hz) until the spawn row's minimum is met.
    /// Loaded saves do **not** call this (the game's save path only resumes zone
    /// spawners), which preserves the existing save/parity behaviour.
    ///
    /// Returns the number of bibites spawned by the immediate iteration.
    pub fn start_fresh_scenario_spawner(&mut self) -> usize {
        if self.bibite_spawns.is_empty() {
            return 0;
        }
        self.spawn_progress = vec![0.0; self.bibite_spawns.len()];
        self.spawn_row_species = vec![None; self.bibite_spawns.len()];
        self.spawner_accumulator = 0.0;
        self.spawner_iteration(1.0)
    }

    /// `true` once [`World::start_fresh_scenario_spawner`] has run.
    #[inline]
    pub fn fresh_scenario_spawner_started(&self) -> bool {
        !self.spawn_progress.is_empty()
    }

    /// One iteration of `BibiteSpawner.SpawnEntity` (`delay` is the 1 s wait /
    /// the initial immediate iteration's 1 s step).
    fn spawner_iteration(&mut self, delay: f32) -> usize {
        if self.bibite_spawns.is_empty()
            || self.spawn_progress.len() != self.bibite_spawns.len()
            || self.spawn_row_species.len() != self.bibite_spawns.len()
        {
            return 0;
        }
        let rate = self.settings.independents.virgin_spawn_rate.max(0.0);
        let share_total: f32 = self
            .bibite_spawns
            .iter()
            .filter(|row| row.spawn_type == settings::SpawnType::Continuous)
            .map(|row| row.spawn_priority.max(0.0))
            .sum();
        let limit = if self.settings.independents.limit_bibite_birth {
            self.settings.independents.bibite_limit.max(0) as usize
        } else {
            usize::MAX
        };
        let mut n_to_limit = limit.saturating_sub(self.bibites.len() + self.eggs.len());
        if n_to_limit == 0 {
            return 0;
        }

        // `descendantCount` per row: the row's species subtree count (live
        // bibites+eggs), or `totalSpawned` for one-time rows.
        let mut descendants = Vec::with_capacity(self.bibite_spawns.len());
        let mut any_missing = false;
        for index in 0..self.bibite_spawns.len() {
            let row = &self.bibite_spawns[index];
            let current = if row.spawn_type == settings::SpawnType::OneTime {
                row.total_spawned.max(0) as usize
            } else {
                self.spawn_row_species[index]
                    .map_or(0, |species| self.species_subtree_count(species))
            };
            descendants.push(current);
            let minimum = row.minimum.max(0) as usize;
            if !row.path.is_empty() && current < minimum {
                any_missing = true;
            }
        }
        if !any_missing {
            return 0;
        }

        let mut spawned_total = 0;
        // Clone the rows so the loop can mutate `spawn_progress` /
        // `bibite_spawns[index]` while iterating.
        let rows = self.bibite_spawns.clone();
        for (index, row) in rows.iter().enumerate() {
            let row = row.clone();
            if row.path.is_empty() || n_to_limit == 0 {
                continue;
            }
            let one_time = row.spawn_type == settings::SpawnType::OneTime;
            let minimum = row.minimum.max(0) as usize;
            let current = descendants[index];
            if current >= minimum {
                continue;
            }
            let missing = minimum - current;
            if one_time {
                self.spawn_progress[index] = (missing as f32 + 0.1) * row.spawn_priority;
            } else if share_total > 0.0 {
                self.spawn_progress[index] +=
                    rate * (row.spawn_priority.max(0.0) / share_total) * delay;
            }
            let Some(template) = self.template_for_path(&row.path).cloned() else {
                continue;
            };
            let cap = missing.min(n_to_limit);
            let mut row_spawned = 0;
            while self.spawn_progress[index] >= 1.0 && row_spawned < cap {
                self.spawn_progress[index] -= 1.0;
                let pos = self.spawn_position_for_row(row.spawn_zone);
                match self.spawn_bibite_from_template(
                    &template,
                    row.growth_at_spawn,
                    row.tagging,
                    &row.tag,
                    pos,
                    row.randomize_genes,
                ) {
                    Ok(id) => {
                        row_spawned += 1;
                        n_to_limit -= 1;
                        self.bibite_spawns[index].total_spawned += 1;
                        if self.spawn_row_species[index].is_none() {
                            self.spawn_row_species[index] =
                                self.bibites.get(id).and_then(|record| record.meta.species);
                        }
                    }
                    Err(_) => break,
                }
            }
            spawned_total += row_spawned;
        }
        spawned_total
    }

    /// Live members (bibites+eggs) in a species subtree, matching
    /// `Species.GetDescendantCount` (`Species.cs:81-88`).
    fn species_subtree_count(&self, root: crate::ids::SpeciesId) -> usize {
        let mut total = 0;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(record) = self.species.get(id) else {
                continue;
            };
            total += record.stats.count as usize;
            stack.extend(record.children.iter().copied());
        }
        total
    }

    /// Continuous spawner tick (1 s scaled cadence in the game): runs the
    /// fresh-scenario spawner coroutine if it was started.
    pub fn step_spawner(&mut self, dt: f32) -> usize {
        if self.spawn_progress.is_empty() {
            return 0;
        }
        self.spawner_accumulator += dt;
        let mut spawned = 0;
        // At normal fixed dt this runs at most once; loop only for large dt.
        while self.spawner_accumulator >= 1.0 {
            self.spawner_accumulator -= 1.0;
            spawned += self.spawner_iteration(1.0);
        }
        spawned
    }

    /// Initial plant seeding for every zone (spec §8.2).
    pub fn initial_seeding(&mut self) -> SeedingReport {
        self.zones
            .initial_seeding(&mut self.pellets, &mut self.hash, None)
    }

    /// Initial seeding with an optional red-death safe radius (spec §1.4/§7.2).
    pub fn initial_seeding_with_red_death(&mut self, safe_radius: Option<f32>) -> SeedingReport {
        let report = self
            .zones
            .initial_seeding(&mut self.pellets, &mut self.hash, safe_radius);
        self.accounting.seeded_plant_energy += report.seeded_plant_energy;
        self.accounting.seeded_meat_energy += report.seeded_meat_energy;
        report
    }

    /// **Canonical tick** (`docs/spec/03-world-ecology.md` §9.2).
    ///
    /// Order: clock → tools positioning/phero towers → zones → world sensing
    /// (FOV on the look-up/sense throttles, pheromone sensing every tick) →
    /// body tick → egg incubation/hatch → pheromone dissipation → pellet decay →
    /// rad towers → Red Death → colour killers → pellet producers → physics →
    /// events/species/stats → spatial-hash reconciliation.
    ///
    /// Deliberately deferred (documented in the W4 report): settings animators
    /// (phase 2) and the game-native metric engine; everything else in §9.2 is
    /// executed in this order, deterministic whatever the thread count.
    ///
    /// With more than one thread a tick runs one of two ways, and the world **measures which is faster** on the machine it is on
    /// ([`crate::pool_tuner::PoolTuner`]: it times its own ticks both ways, keeps the faster and looks at the other now and then; a
    /// size written into the code is right on one machine at best).
    ///
    /// *On the pool*: the whole tick runs on one of the pool's workers, and the stages of it that use the pool (some twenty a tick)
    /// are then calls from a worker, which pushes their chunks to its own queue and runs the ones nobody takes, and not hand-offs
    /// from the calling thread (which blocks until a worker has run the stage and is woken again, some 20 us each). The calling
    /// thread waits once a tick. *Inline*: the tick runs on the calling thread with no handle to the pool at all, so every stage runs
    /// inline (handing the tick to a worker would cost a third of a small one, and a stage that went to the pool from the calling
    /// thread would cost more than it saves). A world too small for any pool to pay is not timed at all. Gates that are not
    /// `adaptive` ([`crate::sim::ParallelGates::FIXED`], `ALWAYS`, `EAGER`) choose by the size of the world alone
    /// ([`crate::sim::ParallelGates::pool_has_work_for`]). Which way a tick runs never changes what it computes.
    pub fn tick(&mut self) {
        let choice = self.choose_tick_arm();
        self.tick_in(choice);
    }

    /// How the next tick runs: inline in a world of one thread, by the world's size with gates that are not `adaptive`, and as the
    /// tuner says otherwise.
    fn choose_tick_arm(&mut self) -> Choice {
        let bibites = self.bibites.len();
        let bodies = bibites + self.pellets.len() + self.eggs.len();
        if self.threads <= 1 {
            Choice {
                arm: Arm::Inline,
                timed: false,
            }
        } else if self.parallel_gates.adaptive {
            self.pool_tuner.choose(WorldSize { bibites, bodies })
        } else {
            let pooled = self.parallel_gates.pool_has_work_for(bibites, bodies);
            Choice {
                arm: if pooled { Arm::Pooled(0) } else { Arm::Inline },
                timed: false,
            }
        }
    }

    /// One tick, run the way `choice` says (the tuner's `choice`, or whatever way a test wants: a world may change from one way to
    /// the other tick by tick, and the results do not change). The tick is timed for the tuner when `choice.timed`.
    pub(crate) fn tick_in(&mut self, choice: Choice) {
        self.pool_wanted = choice.arm.is_pooled() && self.threads > 1;
        if let Arm::Pooled(level) = choice.arm {
            // The level of the pool says how eagerly the stages use it (what a measuring world has the sizes of the stages for; the others keep theirs).
            let gates = if self.parallel_gates.adaptive {
                self.parallel_gates.pooled_level(level)
            } else {
                self.parallel_gates
            };
            if gates != self.active_gates {
                self.active_gates = gates;
                self.bibites.set_parallel_gates(gates);
            }
        }
        let started = choice.timed.then(Instant::now);
        if self.pool_wanted {
            self.ensure_sensing_pool();
            rayon::scope(|_| self.tick_stages());
        } else {
            self.sensing_pool = None;
            self.tick_stages();
        }
        if let Some(started) = started {
            let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            self.pool_tuner.record(nanos);
        }
    }

    /// The stages of [`World::tick`], on whichever thread runs them.
    fn tick_stages(&mut self) {
        // The tick's length is the settings' rate: a tool that wrote `settings.sim_tps` itself (instead of [`World::set_sim_tps`] or
        // [`World::apply_settings`]) is followed here, so the clock and the bodies (which read the rate from the settings) never disagree.
        if self.settings.sim_tps > 0 && self.clock.tps() != self.settings.sim_tps {
            self.clock.set_tps(self.settings.sim_tps);
        }
        let dt = self.clock.dt();
        let start = Instant::now();
        self.clock.step();
        self.simulated_time += f64::from(dt);
        self.mark_phase(Phase::Clock, start);
        // Phase 2: settings animators (0.1 s manager cadence).
        let start = Instant::now();
        self.step_settings_animators(dt);
        self.mark_phase(Phase::Animators, start);
        let start = Instant::now();
        self.step_tools(dt); // phase 3
        self.mark_phase(Phase::Tools, start);
        let start = Instant::now();
        self.step_zones(dt); // phase 4
        self.refresh_zone_circles();
        self.mark_phase(Phase::Zones, start);
        let start = Instant::now();
        self.step_sensing(); // phase 5a (world senses before the body tick)
        self.mark_phase(Phase::Sensing, start);
        let start = Instant::now();
        self.step_body(dt); // phase 5b
        self.mark_phase(Phase::Body, start);
        let start = Instant::now();
        self.step_eggs(dt); // phase 6
        self.mark_phase(Phase::Eggs, start);
        let start = Instant::now();
        self.step_pheromones(dt); // phase 7
        self.mark_phase(Phase::Pheromones, start);
        let start = Instant::now();
        self.step_decay(dt); // phase 8
        self.mark_phase(Phase::Decay, start);
        let start = Instant::now();
        self.step_rad_zones(); // phase 9 (isRad bibites' RadioZone triggers)
        self.step_rad_towers(dt); // phase 9
        self.mark_phase(Phase::Rad, start);
        let start = Instant::now();
        self.step_red_death_phase(dt); // phase 10
        self.mark_phase(Phase::RedDeath, start);
        let start = Instant::now();
        self.step_color_killers(dt); // phase 11
        self.mark_phase(Phase::ColourKillers, start);
        let start = Instant::now();
        self.step_pellet_producers(dt); // phase 12
        self.mark_phase(Phase::Producers, start);
        let start = Instant::now();
        self.step_death_resolution(); // world-inflicted kills become corpses/events
        self.mark_phase(Phase::Deaths, start);
        let start = Instant::now();
        self.step_physics(); // phase 13 (+ collision callbacks in the bridge)
        self.mark_phase(Phase::Physics, start);
        let start = Instant::now();
        self.step_events_and_stats(dt); // phase 14/15
        let _ = self.step_spawner(dt); // game spawner coroutine (1 s scaled)
        self.step_data_logger(dt);
        self.mark_phase(Phase::Events, start);
        let start = Instant::now();
        self.sync_all_hashes();
        self.mark_phase(Phase::HashSync, start);
        self.phase_timings.tick_complete();
    }

    /// `DataLogger.FixedUpdate`: once per simulated minute, logs every stream
    /// (`s.Log()`).
    fn step_data_logger(&mut self, dt: f32) {
        if !self.data_logger.advance(dt) {
            return;
        }
        let points = self.data_points();
        // `GetBirthDeathCount(peek: false)` starts the next minute's count afresh.
        self.data_logger.hatches_since_log = 0;
        self.data_logger.deaths_since_log = 0;
        self.data_logger.log(&points);
        // `nMinutes++`: a full hour logs the distribution of the genes.
        if self.data_logger.count_minute() {
            let buckets = self.current_genes_buckets();
            self.data_logger.genes.push(buckets);
        }
    }

    /// One gene of [`Self::current_genes_buckets`] (what the Gene Analysis page shows, and asks for again each frame while it is
    /// open).
    #[must_use]
    pub fn current_gene_point(&self, gene: usize) -> crate::genes_log::GenesBucketPoint {
        let mut values = Vec::with_capacity(self.bibites.len() + self.eggs.len());
        for (_, record) in self.bibites.iter() {
            if !record.body.dead {
                values.push(record.genome.genes[gene]);
            }
        }
        for id in self.eggs.ids() {
            if let Some(egg) = self.eggs.get(id) {
                values.push(egg.genome.genes[gene]);
            }
        }
        crate::genes_log::GenesBucketPoint::from_values(&values, gene)
    }

    /// `BibiteTracker.GetCurrentGenesBuckets`: the distribution of every gene over the living bibites and the eggs (what the Gene
    /// Analysis page shows as "Now", and what the genes stream logs once an hour).
    #[must_use]
    pub fn current_genes_buckets(&self) -> crate::genes_log::GenesBuckets {
        let mut genes = Vec::with_capacity(self.bibites.len() + self.eggs.len());
        for (_, record) in self.bibites.iter() {
            if !record.body.dead {
                genes.push(record.genome.genes);
            }
        }
        for id in self.eggs.ids() {
            if let Some(egg) = self.eggs.get(id) {
                genes.push(egg.genome.genes);
            }
        }
        crate::genes_log::GenesBuckets::from_genes(&genes)
    }

    /// `BibiteTracker.Get*Data(peek: true)`: the current point of every stream, in the order of
    /// [`crate::datalog::DataLogger::groups`]. The Graphs panel plots it as the newest value
    /// (`IDataPointStream.PeekCurrentDataPoint`).
    #[must_use]
    pub fn data_points(&self) -> Vec<Vec<f32>> {
        let fat_density = self.settings.materials.fat.energy_density;
        let mut bibite_biomass = 0.0_f32;
        let mut ages: Vec<f32> = Vec::new();
        let mut eggs_laid: Vec<u32> = Vec::new();
        let (mut node_min, mut node_max, mut node_sum) = (u32::MAX, 0_u32, 0_u64);
        let (mut syn_min, mut syn_max, mut syn_sum) = (u32::MAX, 0_u32, 0_u64);
        let mut note_brain = |nodes: u32, synapses: u32| {
            node_min = node_min.min(nodes);
            node_max = node_max.max(nodes);
            node_sum += u64::from(nodes);
            syn_min = syn_min.min(synapses);
            syn_max = syn_max.max(synapses);
            syn_sum += u64::from(synapses);
        };
        let mut n_live = 0_usize;
        for (_, record) in self.bibites.iter() {
            // `BibiteTracker.bibites` only holds living bibites (`BibiteDeath`
            // removes them), so corpses are excluded from every stream.
            if !record.body.dead {
                n_live += 1;
                let extra = record.egg_layer.progress * record.egg_layer.egg_energy
                    + record.fat.amount * fat_density;
                bibite_biomass += record.body.total_energy(extra);
                // `BibiteTracker.GetAgeData`: ages (hours) of tracked bibites.
                ages.push(record.clock.time_alive / 3600.0);
                eggs_laid.push(record.egg_layer.n_laid);
                note_brain(
                    record.brain.used_input_mask().count_ones()
                        + record.brain.used_output_mask().count_ones()
                        + record.brain.hidden_count() as u32,
                    record.brain.synapse_count() as u32,
                );
            }
        }
        let mut egg_energy = 0.0_f32;
        for id in self.eggs.ids() {
            if let Some(egg) = self.eggs.get(id) {
                egg_energy += egg.hatch.energy;
                note_brain(
                    egg.brain.used_input_mask().count_ones()
                        + egg.brain.used_output_mask().count_ones()
                        + egg.brain.hidden_count() as u32,
                    egg.brain.synapse_count() as u32,
                );
            }
        }
        let pellets = self.pellets.counts();
        let n_entities = n_live + self.eggs.len();
        ages.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        eggs_laid.sort_unstable();
        let n = ages.len();
        let age_point = if n < 1 {
            vec![0.0; 4]
        } else {
            vec![ages[n / 4], ages[n / 2], ages[3 * n / 4], ages[n - 1]]
        };
        let n_eggs_laid = eggs_laid.len();
        let eggs_point = if n_eggs_laid < 1 {
            vec![0.0; 6]
        } else {
            let sum: u32 = eggs_laid.iter().sum();
            vec![
                eggs_laid[n_eggs_laid / 2] as f32,
                eggs_laid[7 * n_eggs_laid / 10] as f32,
                eggs_laid[8 * n_eggs_laid / 10] as f32,
                eggs_laid[9 * n_eggs_laid / 10] as f32,
                eggs_laid[n_eggs_laid - 1] as f32,
                sum as f32 / n_eggs_laid as f32,
            ]
        };
        let brain_point = if n_entities < 1 || node_min == u32::MAX {
            vec![0.0; 6]
        } else {
            vec![
                node_min as f32,
                node_sum as f32 / n_entities as f32,
                node_max as f32,
                syn_min as f32,
                syn_sum as f32 / n_entities as f32,
                syn_max as f32,
            ]
        };
        // `BibiteTracker.nBirth` counts hatches only (`StartBody`), `nDeath` every
        // tracked death.
        let births = self.data_logger.hatches_since_log;
        let deaths = self.data_logger.deaths_since_log;
        vec![
            vec![
                bibite_biomass + egg_energy,
                pellets.plant_biomass as f32,
                pellets.meat_biomass as f32,
                self.zones.free_biomass(),
            ],
            vec![
                n_entities as f32,
                pellets.plant_count as f32,
                pellets.meat_count as f32,
            ],
            age_point,
            brain_point,
            self.data_logger.death_age_point(),
            eggs_point,
            vec![births as f32, deaths as f32],
        ]
    }

    /// `BibiteBody.StartBody` / `StartBodyAtGrowthAndNormalize` (cs:436-438,
    /// 454-456): a newborn's brain, vision-sensing and vision-lookup counters
    /// start at `Random.Range(0, period)` so births are not phase-locked.
    fn randomize_sense_phase(&mut self, id: BibiteId) {
        let brain = self.settings.brain_tps.max(1) as usize;
        let lookup = self.settings.vision_lookup_updates.max(1) as usize;
        let sense = self.settings.vision_sense_updates.max(1) as usize;
        let brain_counter = self.rng.usize_below(brain) as u32;
        let sense_counter = self.rng.usize_below(sense) as u32;
        let lookup_counter = self.rng.usize_below(lookup) as u32;
        if let Some(record) = self.bibites.get_mut(id) {
            record.throttle.brain_counter = brain_counter;
            record.throttle.sense_counter = sense_counter;
            record.throttle.lookup_counter = lookup_counter;
        }
    }

    /// Records a phase duration in nanoseconds (the body and physics phases also hand over their stages' times).
    #[inline]
    fn mark_phase(&mut self, phase: Phase, start: Instant) {
        self.phase_timings
            .add(phase, start.elapsed().as_nanos() as u64);
        if matches!(phase, Phase::Body | Phase::Physics) {
            self.bibite_scratch.drain_clocks(&mut self.phase_timings);
        }
    }

    /// Per-phase wall-time breakdown accumulated since construction/last reset.
    #[inline]
    pub fn phase_timings(&self) -> &PhaseTimings {
        &self.phase_timings
    }

    /// Resets the phase timing accumulators (bench harnesses).
    pub fn reset_phase_timings(&mut self) {
        self.phase_timings.reset();
    }

    /// What the resting-pellet layer of the physics bridge did (all zero while the world has too few pellets for it).
    #[must_use]
    pub fn pellet_sleep_stats(&self) -> crate::pellet_sleep::SleepStats {
        self.bibite_scratch.pellet_sleep_stats()
    }

    /// How the bibites' pass-B turns were shared out between the worker pool and the calling thread on the ticks that ran in
    /// parallel (all zero for a serial world); observational, for the bench tables and the tests.
    #[must_use]
    pub fn pass_b_lanes(&self) -> crate::body::PassBLanes {
        self.bibite_scratch.pass_b_lanes()
    }

    /// Sets the number of worker threads of the tick.
    ///
    /// `0` means "automatic" (`std::thread::available_parallelism`), `1` forces
    /// the serial path (what a new world has), `n >= 2` enables the persistent
    /// worker pool, created on the first tick. Every stage that can use the pool
    /// then uses as many workers as it has chunks of work for ([`crate::sim::ParallelGates::SCALED`], the
    /// gates a world starts with; [`World::set_parallel_gates`] changes them): a world of a few hundred
    /// bibites ticks on the calling thread, a bigger one puts more of the pool to work, the biggest all of it. Serial
    /// and parallel runs are bit-identical; only which thread computes each item changes.
    pub fn set_threads(&mut self, threads: usize) {
        let requested = if threads == 0 {
            // Automatic: the workers the machine's calibration found the best ([`crate::calibration`]), or every thread the machine has.
            crate::sim::machine_profile()
                .map(|profile| profile.workers)
                .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get))
        } else {
            threads
        }
        .max(1);
        if requested == self.threads {
            return;
        }
        self.threads = requested;
        // The pool is built on the first tick that can use it (`ensure_sensing_pool`), and what was timed was for another pool.
        self.sensing_pool = None;
        self.pool_tuner.reset();
        // Keep the body store's parallel brain path in sync with --threads:
        // 1 forces serial, n uses the body-owned threshold/gating. Physics
        // parallel stages use the same configured worker count.
        self.bibites.set_brain_threads(requested);
        self.sensing_worker_scratch
            .resize_with(requested, QueryScratch::new);
        self.sensing_chunks.resize_with(requested, Vec::new);
    }

    /// How much work each stage of the tick needs per chunk to use the worker pool (see [`crate::sim::ParallelGates`]).
    #[must_use]
    pub fn parallel_gates(&self) -> crate::sim::ParallelGates {
        self.parallel_gates
    }

    /// Sets how much work each stage needs per chunk to run on the worker pool (the results never depend on it; the tests
    /// lower it to run every parallel path on a small world).
    pub fn set_parallel_gates(&mut self, gates: crate::sim::ParallelGates) {
        self.parallel_gates = gates;
        self.active_gates = gates;
        self.bibites.set_parallel_gates(gates);
        self.pool_tuner = PoolTuner::new(gates.pool_prior());
    }

    /// Takes the calibration of the machine ([`crate::sim::machine_profile`]) now, or the reference machine's numbers when there is none (the
    /// player switched it off): its workers (a world that was set to a number of threads by hand keeps that: pass `automatic` false) and
    /// its first guess of where the pool pays, which the world's pool tuner then starts from. The world keeps ticking the same: nothing a
    /// tick computes depends on any of it.
    pub fn apply_machine_profile(&mut self, automatic: bool) {
        if automatic {
            self.set_threads(0);
        }
        // (`calibrated` is the profile's gates, the reference machine's when there is no profile, or the ones `RUSTIBITES_GATES` names.) A
        // world whose gates were set by hand (a benchmark, a test: not measuring) keeps them.
        let gates = crate::sim::ParallelGates::calibrated();
        if self.parallel_gates.adaptive && self.parallel_gates != gates {
            self.set_parallel_gates(gates);
        }
    }

    /// What the world's pool tuner has done so far: ticks each way, probes, switches and the mean tick time of each way (all zero for
    /// a world that does not measure: one thread, or gates that are not `adaptive`).
    #[must_use]
    pub fn pool_tuner_stats(&self) -> PoolTunerStats {
        self.pool_tuner.stats()
    }

    /// The way the world's ticks run between probes of the others: inline, or on the pool at a level (inline for a world that does not measure).
    #[must_use]
    pub fn pool_arm(&self) -> Arm {
        self.pool_tuner.incumbent()
    }

    /// The changes of that way since the last call (the last 32 at most), for the logs.
    pub fn take_pool_switches(&mut self) -> Vec<PoolSwitch> {
        self.pool_tuner.take_switches()
    }

    /// Creates the persistent pool on first use by a gated parallel phase.
    fn ensure_sensing_pool(&mut self) {
        if self.threads > 1 && self.pool_wanted && self.sensing_pool.is_none() {
            self.sensing_pool = Some(ThreadPool::new(self.threads));
        }
    }

    /// Workers the stages of the tick that runs may use: the pool's, or 1 when the world has no pool (one thread, or too small
    /// for it).
    fn pool_threads(&self) -> usize {
        self.sensing_pool.as_ref().map_or(1, ThreadPool::threads)
    }

    /// Number of sensing worker threads currently configured (1 = serial).
    #[inline]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Serial/parallel execution handle for body/physics work.
    ///
    /// Borrows the persistent world pool (`Serial` when `threads == 1`, and during the ticks of a world too small for the pool). The
    /// returned handle only accepts **owned** chunk payloads, so workers cannot
    /// alias mutable world state; callers pass immutable snapshots inside the
    /// payload/context and merge results in chunk order.
    #[inline]
    pub fn executor(&self) -> crate::sim::ParallelExecutor<'_> {
        crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref())
    }

    /// Advances `ticks` ticks.
    pub fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.tick();
        }
    }

    /// Global statistics collector (latest sample + running counters).
    #[inline]
    pub fn stats(&self) -> &SimStats {
        &self.stats
    }

    /// Collects a statistics sample (parity `ticks.jsonl` record groups).
    ///
    /// Safe to call after any tick; the sample also resets the since-sample
    /// lifecycle/behaviour accumulators.
    /// Snapshots per-bibite body `FeedingCounters` and turns them into the
    /// interval deltas used by §6.1 feeding rates + conservation accounting.
    fn update_feeding_interval(&mut self) {
        let mut interval = FeedingInterval::default();
        let mut updates: Vec<(usize, u32, crate::body::FeedingCounters)> = Vec::new();
        for (id, record) in self.bibites.iter() {
            let slot = id.handle().slot() as usize;
            let current_generation = id.handle().generation();
            let current = record.feeding;
            let previous = self
                .feeding_prev
                .get(slot)
                .copied()
                .flatten()
                .filter(|(generation, _)| *generation == current_generation)
                .map(|(_, counters)| counters);
            let old = previous.unwrap_or_default();
            let plant_bites = current
                .plant_bite_events
                .saturating_sub(old.plant_bite_events);
            let meat_bites = current
                .meat_bite_events
                .saturating_sub(old.meat_bite_events);
            let bibite_bites = current
                .bibite_bite_events
                .saturating_sub(old.bibite_bite_events);
            interval.plant_bite_events += plant_bites;
            interval.meat_bite_events += meat_bites;
            interval.bibite_bite_events += bibite_bites;
            interval.bibite_damage_dealt +=
                (current.bibite_damage_dealt - old.bibite_damage_dealt).max(0.0);
            interval.plant_consumed_u2 +=
                (current.plant_consumed_u2 - old.plant_consumed_u2).max(0.0);
            interval.meat_consumed_u2 += (current.meat_consumed_u2 - old.meat_consumed_u2).max(0.0);
            interval.stomach_energy_gain_e +=
                (current.stomach_energy_gain_e - old.stomach_energy_gain_e).max(0.0);
            interval.energy_gained_e += (current.energy_gained_e - old.energy_gained_e).max(0.0);
            interval.energy_spent_e += (current.energy_spent_e - old.energy_spent_e).max(0.0);
            interval.live_seconds += (current.live_seconds - old.live_seconds).max(0.0);
            updates.push((slot, current_generation, current));
        }
        let needed = updates
            .iter()
            .map(|(slot, _, _)| slot + 1)
            .max()
            .unwrap_or(0);
        if self.feeding_prev.len() < needed {
            self.feeding_prev.resize(needed, None);
        }
        for (slot, generation, counters) in updates {
            self.feeding_prev[slot] = Some((generation, counters));
        }
        self.accounting.consumed_plant_u2 += interval.plant_consumed_u2;
        self.accounting.consumed_meat_u2 += interval.meat_consumed_u2;
        self.accounting.consumed_plant_energy +=
            interval.plant_consumed_u2 * f64::from(self.settings.materials.plant.energy_density);
        self.accounting.consumed_meat_energy +=
            interval.meat_consumed_u2 * f64::from(self.settings.materials.meat.energy_density);
        self.feeding_interval = interval;
    }

    /// Body-owned corpse/rip/meat creation is not routed through the zone
    /// ledger; account the positive meat-stock delta created during a body
    /// phase so `produced_meat_energy` covers corpse/rip/attack overflow.
    fn account_body_meat_creation(&mut self, before_energy: f64) {
        let after = self.pellets.counts().meat_biomass;
        let created = (after - before_energy).max(0.0);
        self.accounting.produced_meat_energy += created;
    }

    /// Cumulative runtime-only pellet conservation counters.
    #[inline]
    pub fn conservation_totals(&self) -> ConservationTotals {
        self.accounting
    }

    /// Feeding-counter deltas since the previous stats sample.
    #[inline]
    pub fn feeding_interval(&self) -> FeedingInterval {
        self.feeding_interval
    }

    /// Snapshots conservation counters + live pellet stock for identity checks.
    pub fn conservation_snapshot(&mut self) -> ConservationSnapshot {
        self.update_feeding_interval();
        let counts = self.pellets.counts();
        ConservationSnapshot {
            totals: self.accounting,
            stock_plant_energy: counts.plant_biomass,
            stock_meat_energy: counts.meat_biomass,
            terms_missing: false,
        }
    }

    /// Forgets the contact impulses the physics carries between ticks (warm starting). A restored checkpoint and a loaded save start without
    /// them, so a run that is to match its own restore calls this where the checkpoint is taken.
    pub fn forget_contact_impulses(&mut self) {
        self.bibite_scratch.forget_contact_impulses();
    }

    /// Captures the deterministic checkpoint state for a bit-identical resume.
    ///
    /// Base fields (clock/time/RNG/throttles/red death) follow
    /// [`SimSnapshot`]; entity-arena and world runtime state travel in
    /// extension sections (see `docs/design/CHECKPOINT_STATE.md`).
    pub fn capture_checkpoint_state(
        &self,
    ) -> std::result::Result<SimSnapshot, crate::sim::SnapshotError> {
        // Preflight for 100k-scale worlds: the data would exceed the encoded
        // snapshot payload budget, so fail cleanly before allocating the arena.
        let estimated = checkpoint_payload_estimate(self.bibites.len(), self.pellets.len());
        if estimated > crate::sim::payload_limit() {
            return Err(crate::sim::SnapshotError::LimitExceeded(
                "checkpoint payload (world too large; >100k unsupported)",
            ));
        }
        let mut snapshot = SimSnapshot::new(
            self.clock.tick(),
            self.clock.tps(),
            self.simulated_time,
            self.rng.state(),
        );
        snapshot.zone_rngs = self
            .zones
            .zones
            .iter()
            .enumerate()
            .map(|(index, zone)| (index as u32, zone.capture_runtime().rng))
            .collect();
        snapshot.spawner_accumulator = self.spawner_accumulator;
        snapshot.decay_period = self.decay.period();
        snapshot.decay_progress = self.decay.progress();
        snapshot.animator_progress = self.animator_progress;
        snapshot.animator_started.clone_from(&self.animator_started);
        snapshot.animator_ended.clone_from(&self.animator_ended);
        snapshot.red_death_progress = self.tools.red_death.progress;
        snapshot.red_death_safe_radius = self.tools.red_death.safe_radius;
        let species = self.species.capture_checkpoint();
        snapshot.species_name_rng = Some(species.name_rng);
        snapshot.push_section(CHECKPOINT_SPECIES_TAG, species.encode())?;
        snapshot.push_section(
            CHECKPOINT_BIBITE_TAG,
            self.bibites.capture_checkpoint().encode(),
        )?;
        snapshot.push_section(CHECKPOINT_EGG_TAG, self.eggs.capture_checkpoint().encode())?;
        snapshot.push_section(CHECKPOINT_ZONE_TAG, self.zones.encode_runtime())?;
        snapshot.push_section(CHECKPOINT_ZONE_LOG_TAG, self.zones.encode_logs())?;
        snapshot.push_section(CHECKPOINT_PELLET_TAG, self.pellets.capture_checkpoint())?;
        snapshot.push_section(
            CHECKPOINT_PHEROMONE_TAG,
            self.pheromones.capture_checkpoint(),
        )?;
        snapshot.push_section(CHECKPOINT_TOOLS_TAG, capture_tools_checkpoint(&self.tools))?;
        snapshot.push_section(CHECKPOINT_HASH_TAG, self.hash.capture_checkpoint_bytes())?;
        snapshot.push_section(CHECKPOINT_BRAIN_TAG, capture_brain_checkpoint(self))?;
        snapshot.push_section(CHECKPOINT_STATS_TAG, self.stats.encode_checkpoint())?;
        let started = !self.spawn_progress.is_empty();
        let mut spawner = Vec::with_capacity(5 + self.bibite_spawns.len() * 9);
        spawner.extend_from_slice(&(self.bibite_spawns.len() as u32).to_le_bytes());
        spawner.push(u8::from(started));
        for row in 0..self.bibite_spawns.len() {
            let progress = if started {
                self.spawn_progress.get(row).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            spawner.extend_from_slice(&progress.to_bits().to_le_bytes());
            match if started {
                self.spawn_row_species.get(row).copied().flatten()
            } else {
                None
            } {
                Some(species) => {
                    spawner.push(1);
                    spawner.extend_from_slice(&species.value().to_le_bytes());
                }
                None => spawner.push(0),
            }
        }
        snapshot.push_section(CHECKPOINT_SPAWNER_TAG, spawner)?;
        snapshot.push_section(
            CHECKPOINT_FINGERPRINT_TAG,
            crate::sim::state_fingerprint(self).to_le_bytes().to_vec(),
        )?;
        Ok(snapshot)
    }

    /// Restores a checkpoint snapshot produced by
    /// [`World::capture_checkpoint_state`] into a world loaded from the matching
    /// game save, then verifies the recorded state fingerprint.
    pub fn restore_checkpoint_state(&mut self, snapshot: &SimSnapshot) -> crate::error::Result<()> {
        if snapshot.sim_tps != self.clock.tps() {
            return Err(crate::error::Error::invalid_data(format!(
                "checkpoint sim_tps {} does not match world {}",
                snapshot.sim_tps,
                self.clock.tps()
            )));
        }
        let clock = snapshot.restore_clock().map_err(|error| {
            crate::error::Error::invalid_data(format!("invalid checkpoint clock: {error:?}"))
        })?;
        self.clock = clock;
        self.simulated_time = snapshot.simulated_time;
        self.data_logger = crate::datalog::DataLogger::new();
        self.bibite_scratch.reset_collision_tracking();
        if !self.rng.set_state(snapshot.rng) {
            return Err(crate::error::Error::invalid_data(
                "invalid checkpoint world RNG state",
            ));
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_BIBITE_TAG) {
            let bytes = bytes.as_slice();
            let arena = crate::body::BibiteCheckpointArena::decode(bytes)?;
            self.bibites.restore_checkpoint(arena)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_EGG_TAG) {
            let bytes = bytes.as_slice();
            let arena = crate::body::EggCheckpointArena::decode(bytes)?;
            self.eggs.restore_checkpoint(arena)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_BRAIN_TAG) {
            let bytes = bytes.as_slice();
            restore_brain_checkpoint(self, bytes)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_SPECIES_TAG) {
            let bytes = bytes.as_slice();
            let species = crate::species::SpeciesCheckpoint::decode(bytes)?;
            self.species.restore_checkpoint(species)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_ZONE_TAG) {
            let bytes = bytes.as_slice();
            let zones = crate::zone::ZoneStore::decode_runtime(bytes)?;
            self.zones.restore_runtime(&zones)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_ZONE_LOG_TAG) {
            self.zones.restore_logs(bytes.as_slice())?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_PELLET_TAG) {
            let bytes = bytes.as_slice();
            self.pellets.restore_checkpoint(bytes)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_PHEROMONE_TAG) {
            let bytes = bytes.as_slice();
            self.pheromones.restore_checkpoint(bytes)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_TOOLS_TAG) {
            let bytes = bytes.as_slice();
            restore_tools_checkpoint(&mut self.tools, bytes)?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_STATS_TAG) {
            let bytes = bytes.as_slice();
            self.stats = crate::sim::SimStats::decode_checkpoint(bytes).map_err(|error| {
                crate::error::Error::invalid_data(format!("invalid stats section: {error:?}"))
            })?;
        }
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_SPAWNER_TAG) {
            let bytes = bytes.as_slice();
            let mut reader = CheckpointReader::new(bytes);
            let rows = reader.u32()? as usize;
            let started = reader.u8()? == 1;

            if rows != self.bibite_spawns.len() {
                return Err(crate::error::Error::invalid_data(format!(
                    "checkpoint spawner has {rows} rows, world has {}",
                    self.bibite_spawns.len()
                )));
            }
            if started {
                self.spawn_progress.resize(rows, 0.0);
                self.spawn_row_species.resize(rows, None);
                for row in 0..rows {
                    self.spawn_progress[row] = reader.f32()?;
                    self.spawn_row_species[row] = if reader.u8()? == 1 {
                        Some(crate::ids::SpeciesId::new(reader.u32()?))
                    } else {
                        None
                    };
                }
            } else {
                self.spawn_progress.clear();
                self.spawn_row_species.clear();
                for _ in 0..rows {
                    let _ = reader.f32()?;
                    if reader.u8()? == 1 {
                        let _ = reader.u32()?;
                    }
                }
            }
        }
        self.spawner_accumulator = snapshot.spawner_accumulator;
        if snapshot.decay_period != self.decay.period() {
            return Err(crate::error::Error::invalid_data(
                "checkpoint decay period does not match world",
            ));
        }
        self.decay.set_progress(snapshot.decay_progress);
        self.animator_progress = snapshot.animator_progress;
        if snapshot.animator_started.len() == self.settings_changers.len() {
            self.animator_started.clone_from(&snapshot.animator_started);
            self.animator_ended.clone_from(&snapshot.animator_ended);
        }
        self.tools.red_death.progress = snapshot.red_death_progress;
        self.tools.red_death.safe_radius = snapshot.red_death_safe_radius;
        self.refresh_zone_circles();
        if let Some(bytes) = snapshot.extension_payload(CHECKPOINT_HASH_TAG) {
            let bytes = bytes.as_slice();
            self.hash.restore_checkpoint_bytes(bytes)?;
        } else {
            self.rebuild_hashes();
        }
        let recorded = snapshot
            .extension(CHECKPOINT_FINGERPRINT_TAG)
            .ok_or_else(|| {
                crate::error::Error::invalid_data(
                    "checkpoint is missing the world fingerprint section",
                )
            })?;
        if recorded.len() != 8 {
            return Err(crate::error::Error::invalid_data(
                "invalid checkpoint fingerprint section length",
            ));
        }
        let mut raw = [0u8; 8];
        raw.copy_from_slice(recorded);
        let expected = u64::from_le_bytes(raw);
        let actual = crate::sim::state_fingerprint(self);
        if actual != expected {
            return Err(crate::error::Error::invalid_data(format!(
                "checkpoint fingerprint mismatch: expected {expected:#x}, got {actual:#x}"
            )));
        }
        Ok(())
    }

    /// Collects a statistics sample (parity `ticks.jsonl` record groups) and
    /// updates the interval feeding/conservation counters first.
    pub fn sample_stats(&mut self) -> StatsSample {
        self.update_feeding_interval();
        let tick = self.clock.tick();
        let t = self.simulated_time as f32;
        let sample = crate::sim::collect_stats(self, tick, t);
        self.stats.record(sample.clone());
        sample
    }

    /// Adds a settings animator (settings-changer editor action). Returns its
    /// index in [`Self::settings_changers`]; the animator starts on its own
    /// `start` time and runs from the next animator phase.
    pub fn add_settings_changer(&mut self, changer: SettingsChanger) -> Result<usize> {
        changer.validate()?;
        self.settings_changers.push(changer);
        self.animator_started.push(false);
        self.animator_ended.push(false);
        Ok(self.settings_changers.len() - 1)
    }

    /// Removes a settings animator by index (UI delete action).
    pub fn remove_settings_changer(&mut self, index: usize) -> Option<SettingsChanger> {
        if index >= self.settings_changers.len() {
            return None;
        }
        if index < self.animator_started.len() {
            self.animator_started.remove(index);
        }
        if index < self.animator_ended.len() {
            self.animator_ended.remove(index);
        }
        Some(self.settings_changers.remove(index))
    }

    /// Whether a settings animator already animates the setting with this key (`ScenarioSettings.animatedSettings`): such a setting cannot be
    /// animated by a second animator, and while it is animated it cannot be edited by hand.
    #[must_use]
    pub fn is_animated(&self, key: &str) -> bool {
        self.settings_changers
            .iter()
            .any(|changer| changer.settings_bases.iter().any(|base| base.key == key))
    }

    /// Edits animator `index` (the animator editor's handle): the changed animator must still be valid (at least one point, the points in
    /// non-decreasing order of time - two may share a time, which is a jump - and a start that is not negative), otherwise nothing changes.
    pub fn edit_settings_changer(
        &mut self,
        index: usize,
        edit: impl FnOnce(&mut SettingsChanger),
    ) -> Result<()> {
        let Some(current) = self.settings_changers.get(index) else {
            return Err(Error::invalid_data("unknown settings animator"));
        };
        let mut changer = current.clone();
        edit(&mut changer);
        changer.validate()?;
        self.settings_changers[index] = changer;
        Ok(())
    }

    /// `SettingsChanger.AddBase`: animator `index` takes over the setting `key`, which it then multiplies by the curve from the value it has now
    /// (`value`). Refused (`false`) when another animator or this one already has it.
    pub fn add_changer_base(&mut self, index: usize, key: &str, value: f32) -> bool {
        if index >= self.settings_changers.len() || self.is_animated(key) {
            return false;
        }
        self.settings_changers[index]
            .settings_bases
            .push(settings::SettingBase {
                key: key.to_owned(),
                base: value,
            });
        true
    }

    /// Removes every settings animator (UI clear action).
    pub fn clear_settings_changers(&mut self) {
        self.settings_changers.clear();
        self.animator_started.clear();
        self.animator_ended.clear();
    }

    /// Phase 2: applies the settings animators every 0.1 s of simulated time.
    ///
    /// Mirrors `SettingsChangerManager.FixedUpdate` (spec 04 §7.1): accumulate
    /// fixed time, evaluate every 0.1 s, drop animators that set `ended`.
    pub fn step_settings_animators(&mut self, dt: f32) {
        if self.settings_changers.is_empty() {
            return;
        }
        self.animator_progress += dt;
        let mut iterations = 0u32;
        while self.animator_progress >= 0.1 && iterations < 64 {
            self.animator_progress -= 0.1;
            iterations += 1;
            self.apply_settings_animators();
        }
    }

    fn apply_settings_animators(&mut self) {
        for index in 0..self.settings_changers.len() {
            if self.animator_ended.get(index).copied().unwrap_or(false) {
                continue;
            }
            let changer = core::mem::take(&mut self.settings_changers[index]);
            let mut started = self.animator_started.get(index).copied().unwrap_or(false);
            let mut ended = false;
            let factor = animator_factor(
                &changer,
                self.simulated_time as f32,
                &mut started,
                &mut ended,
            );
            self.animator_started
                .resize(self.settings_changers.len(), false);
            if let Some(factor) = factor {
                for base in &changer.settings_bases {
                    self.apply_animated_setting(&base.key, base.base * factor);
                }
            }
            self.animator_started[index] = started;
            self.animator_ended[index] = ended;
            self.settings_changers[index] = changer;
        }
        // `SettingsChangerManager.FixedUpdate` (cs:55-75): finished non-repeating
        // animators are cleared and removed from the scenario.
        let finished: Vec<usize> = (0..self.settings_changers.len())
            .filter(|index| self.animator_ended.get(*index).copied().unwrap_or(false))
            .collect();
        for index in finished.into_iter().rev() {
            self.remove_settings_changer(index);
        }
    }

    /// Applies one animated setting: `Zone(<id>).<field>`, then a scenario
    /// setting key, then an independent setting key (spec 04 §7.1 key order).
    fn apply_animated_setting(&mut self, key: &str, value: f32) {
        if !value.is_finite() {
            return;
        }
        if let Some((zone_id, field)) = parse_zone_setting_key(key) {
            let bounds = &self.bounds;
            let globals = self.zones.globals.clone();
            if let Some(zone) = self
                .zones
                .zones
                .iter_mut()
                .find(|zone| zone.config.runtime_zone_id() == zone_id)
            {
                if apply_zone_setting(&mut zone.config, field, value) {
                    zone.refresh(bounds, &globals);
                    if field == "posX" || field == "posY" {
                        // `Zone.UpdatePosition` (Zone.cs:548-569), including
                        // its quirk of clamping the *fraction* against a
                        // world-unit limit.
                        let sim_size = bounds.sim_size();
                        let (limit_x, limit_y) = zone.centre_bounds(sim_size);
                        let clamp = |v: f32, limit: f32| v.abs().min(limit) * v.signum();
                        zone.pos = Vec2::new(
                            sim_size * clamp(zone.config.pos_x, limit_x),
                            sim_size * clamp(zone.config.pos_y, limit_y),
                        );
                    }
                }
            }
            self.refresh_zone_circles();
            return;
        }
        let int_value = value as i32;
        if let Some(meta) = settings::ScenarioSettings::SETTINGS
            .iter()
            .find(|meta| meta.key == key)
        {
            if meta.animatable {
                let setting = match meta.kind {
                    SettingKind::Int => SettingValue::Int(int_value),
                    _ => SettingValue::Float(value),
                };
                self.settings.scenario.set_raw(key, setting);
                self.propagate_animated_settings();
            }
            return;
        }
        if let Some(meta) = settings::IndependentSettings::SETTINGS
            .iter()
            .find(|meta| meta.key == key)
        {
            if meta.animatable {
                let setting = match meta.kind {
                    SettingKind::Int => SettingValue::Int(int_value),
                    _ => SettingValue::Float(value),
                };
                self.settings.independents.set_raw(key, setting);
                self.propagate_animated_settings();
            }
        }
    }

    /// `SettingsChanger` writes through `SetFromFloat`, which fires the
    /// setting's subscribers (`Zone.UpdateBiomass/Fertility/AvgPelletEnergy/
    /// UpdateSpeed`, bibite organ parameters, Red Death). Mirror that by
    /// refreshing the derived runtime state after an animated write; unlike
    /// [`World::apply_settings`] this keeps materials and the decay throttle.
    fn propagate_animated_settings(&mut self) {
        self.settings.sync_rates_from_independents();
        let globals = ZoneGlobals::from_settings(&self.settings);
        let bounds = self.bounds;
        self.zones.globals = globals.clone();
        for zone in &mut self.zones.zones {
            zone.refresh(&bounds, &globals);
        }
        self.bibites.set_settings(self.settings.clone());
        self.tools.red_death.base_damage = self.settings.scenario.red_death_base_damages;
        self.tools.red_death.damage_velocity = self.settings.scenario.red_death_damage_velocity;
        self.tools
            .red_death
            .set_enabled(self.settings.scenario.enable_red_death);
        self.tools
            .red_death
            .set_fill(self.settings.scenario.red_death_fill);
    }

    /// Rebuilds the [`ZoneCircle`] list consumed by the body tick.
    fn refresh_zone_circles(&mut self) {
        self.zone_circles.clear();
        for zone in &self.zones.zones {
            self.zone_circles.push(ZoneCircle {
                id: zone.id,
                pos: zone.pos,
                // `Zone.radius` is set for circular zones only (`Zone.UpdateSize`): a rectangle's is 0 for the void avoidance, which treats it as its centre.
                radius: if zone.is_rect { 0.0 } else { zone.radius },
                is_rect: zone.is_rect,
                half_extents: zone.half_extents,
            });
        }
    }

    /// Phase 5a: fills each live bibite's `SenseOutputs` from the spatial hash.
    ///
    /// Serial and parallel paths call exactly `compute_senses` once per due
    /// bibite; the parallel path only splits the dense-slot id list into chunks,
    /// computes into per-chunk buffers with a per-job `QueryScratch`, then applies
    /// the buffers in dense slot order. The simulation state is identical by
    /// construction (same per-bibite function, same write order).
    pub fn step_sensing(&mut self) {
        self.senses_scratch.clear();
        // Prefilter before moving any store out of the world: most ticks have no
        // bibite due (sense cadence is 1/10 ticks), and moving the spatial hash
        // out/in allocates its cell grid, which would otherwise dominate those
        // ticks. The due list is reused between ticks.
        let mut ids = core::mem::take(&mut self.sensing_due_ids);
        ids.clear();
        if self.threads > 1 {
            self.ensure_sensing_pool();
        }
        self.bibites.refresh_summaries();
        {
            let settings = &self.settings;
            self.bibites.filter_map_dense_summaries_into(
                self.executor(),
                &mut ids,
                |id, summary, records, slot| {
                    if summary.bits & SlotSummary::SENSIBLE == 0 {
                        return None;
                    }
                    let record = records[slot].as_ref()?;
                    senses_due_from(
                        &record.throttle,
                        summary.bits & SlotSummary::PHERO != 0,
                        settings,
                    )
                    .then_some(id)
                },
            );
        }
        #[cfg(test)]
        assert_eq!(
            ids,
            self.bibites
                .iter()
                .filter(|(_, record)| bibite_senses_due(record, &self.settings))
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            "the parallel prefilter for due senses differs from the plain one"
        );
        if ids.is_empty() {
            self.sensing_due_ids = ids;
            return;
        }
        // The vision lookups ask the spatial hash where the bibites are *now* (`Physics2D.Overlap*` reads the colliders' current places): a
        // bibite's entry moves with it. (Without this, an entry stayed where its bibite was spawned or loaded and the others stopped seeing it.)
        self.refresh_bibite_hash();
        let data = SensingData {
            bibites: core::mem::take(&mut self.bibites),
            pellets: core::mem::take(&mut self.pellets),
            pheromones: core::mem::take(&mut self.pheromones),
            hash: self.hash.take_same_layout(),
            // Moved (not cloned): the sensing snapshot owns the settings for the
            // duration of the phase, avoiding a per-tick deep clone.
            settings: core::mem::take(&mut self.settings),
            red_death: self.tools.red_death,
        };
        let mut scratch = core::mem::take(&mut self.query_scratch);
        // The prefilter above tells us how many bibites are due: from `ParallelGates::sensing` of them (in a world that is on the
        // pool at all) a worker for every quarter of that, at most the pool. Below it the serial path (bit-identical to --threads 1) runs.
        let sensing_workers = crate::sim::ParallelGates::workers_for(
            ids.len(),
            self.active_gates.sensing,
            self.pool_threads(),
        );
        // (More than one worker means the pool is there: `pool_threads` is 1 without it.)
        let use_parallel = sensing_workers > 1 && self.sensing_pool.is_some();
        let data = if use_parallel {
            let data_arc = Arc::new(data);
            let ids_arc = Arc::new(ids);
            let len = ids_arc.len();
            let chunk_count = sensing_workers;
            let result_tx = self.sensing_result_tx.clone();
            let mut jobs: Vec<PoolJob> = Vec::with_capacity(chunk_count);
            // The jobs' result buffers are kept from tick to tick (empty, with their capacity): a fresh buffer every tick was
            // 24 MB of new pages at 100,000 bibites.
            let mut chunk_results = core::mem::take(&mut self.sensing_chunks);
            chunk_results.truncate(chunk_count);
            chunk_results.resize_with(chunk_count, Vec::new);
            for (chunk, buffer) in chunk_results.iter_mut().enumerate() {
                let start = chunk * len / chunk_count;
                let end = (chunk + 1) * len / chunk_count;
                let chunk_data = Arc::clone(&data_arc);
                let chunk_ids = Arc::clone(&ids_arc);
                let tx = result_tx.clone();
                // Reuse a preallocated scratch buffer (no per-tick allocation).
                let scratch = self.sensing_worker_scratch.pop().unwrap_or_default();
                let mut results = core::mem::take(buffer);
                results.clear();
                results.reserve(end - start);
                jobs.push(Box::new(move || {
                    let mut scratch = scratch;
                    for index in start..end {
                        let id = chunk_ids[index];
                        if let Some((senses, steering)) =
                            compute_senses(&chunk_data, id, &mut scratch)
                        {
                            results.push((id, senses, steering));
                        }
                    }
                    let _ = tx.send((chunk, scratch, results));
                }));
            }
            if let Some(pool) = self.sensing_pool.as_ref() {
                pool.execute(jobs);
            }
            for _ in 0..chunk_count {
                match self.sensing_result_rx.recv() {
                    Ok((chunk, scratch, results)) => {
                        self.sensing_worker_scratch.push(scratch);
                        if let Some(slot) = chunk_results.get_mut(chunk) {
                            *slot = results;
                        }
                    }
                    Err(_) => break,
                }
            }
            // The results stay in the jobs' buffers: they are applied from there.
            self.sensing_chunks = chunk_results;
            ids = match Arc::try_unwrap(ids_arc) {
                Ok(ids) => ids,
                Err(_) => panic!("sensing id list still shared after worker join"),
            };
            match Arc::try_unwrap(data_arc) {
                Ok(data) => data,
                Err(_) => panic!("sensing snapshot still shared after worker join"),
            }
        } else {
            for &id in &ids {
                if let Some((senses, cache)) = compute_senses(&data, id, &mut scratch) {
                    self.senses_scratch.push((id, senses, cache));
                }
            }
            data
        };
        self.bibites = data.bibites;
        self.pellets = data.pellets;
        self.pheromones = data.pheromones;
        self.hash = data.hash;
        self.settings = data.settings;
        self.query_scratch = scratch;
        self.sensing_due_ids = ids;
        let executor = crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref());
        if use_parallel {
            let mut chunks = core::mem::take(&mut self.sensing_chunks);
            self.bibites.apply_senses_chunks(&mut chunks, executor);
            self.sensing_chunks = chunks;
        } else {
            let results = core::mem::take(&mut self.senses_scratch);
            self.bibites.apply_senses(&results, executor);
            self.senses_scratch = results;
        }
    }

    /// Phase 5b: the body tick (`base metabolism → brain → organs → propulsion →
    /// UseEnergy`, body spec §2.3/§2.6).
    pub fn step_body(&mut self, dt: f32) {
        // Every stage of the body tick runs on the pool when the world is big enough for it and the stage has the work (see
        // `ParallelGates`; with no pool handle, or one thread, every stage is inline, and the brain batches are serial too).
        let parallel = self.threads > 1;
        if parallel {
            self.ensure_sensing_pool();
        }
        self.bibites.set_brain_threads(self.pool_threads());
        let executor = if parallel {
            crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref())
        } else {
            crate::sim::ParallelExecutor::Serial
        };
        let meat_before = self.pellets.counts().meat_biomass;
        let mut ctx = BibiteTickCtx {
            pellets: &mut self.pellets,
            pheromones: &mut self.pheromones,
            eggs: &mut self.eggs,
            zones: &self.zone_circles,
            settings: &self.settings,
            rng: &mut self.rng,
            scratch: &mut self.bibite_scratch,
            events: &mut self.events,
        };
        self.bibites.tick_with_executor(&mut ctx, executor);
        self.account_body_meat_creation(meat_before);
        let _ = dt;
    }

    /// The eggs that hatched since the last call, each with the bibite it became
    /// (`EggHatching.newBornBody`); the selection follows a hatching egg to its newborn.
    pub fn take_hatched_children(&mut self) -> Vec<(EggId, BibiteId)> {
        std::mem::take(&mut self.hatched_children)
    }

    /// How many bibites were born with a brain of 100 hidden nodes or more since the last call (`BibiteBody.Born`: `naturalBirth`
    /// and `Nodes.Length - NInputs - NOutputs >= 100` is the `ACH_BIBITE_EINSTEIN` trigger). Not part of the simulation's state.
    pub fn take_einstein_births(&mut self) -> u32 {
        std::mem::take(&mut self.einstein_births)
    }

    /// Phase 6: egg incubation, hatching (spawn child) and aborts (meat pile).
    pub fn step_eggs(&mut self, dt: f32) {
        self.egg_positions.clear();
        for id in self.eggs.ids() {
            if let Some(egg) = self.eggs.get(id) {
                self.egg_positions
                    .push((id, egg.pos, egg.size_gene.max(0.0)));
            }
        }
        self.hatched.clear();
        {
            let settings = &self.settings;
            self.eggs.tick(dt, settings, &mut self.hatched);
        }
        for index in 0..self.hatched.len() {
            let id = self.hatched[index];
            let Some(mut payload) = self.eggs.hatch_payload(id) else {
                self.eggs.remove(id);
                continue;
            };
            // `newBornBrain.CopyBrain(eggBrain.Nodes, eggBrain.Synapses, mutate: false)`: the newborn's nodes start fresh.
            payload.brain.reset_runtime_state(self.settings.brain_period());
            self.eggs.remove(id);
            self.hash.remove(id.handle(), SpatialClass::Egg);
            let mut meta = payload.meta.clone();
            let parent = meta.species;
            // `GlobalLineageManager.CheckNewSpecies` (cs:235-246): a new species
            // is founded from the *parent* body (the child itself when the
            // parent is gone) and the parent moves into it too.
            let parent_id = meta.parent1;
            let species_before = self.species.records().len();
            let species = {
                let founder = parent_id
                    .and_then(|pid| self.bibites.get(pid))
                    .map(|record| (&record.genome, &record.brain));
                self.species.assign_with_founder(
                    &payload.genome,
                    &payload.brain,
                    &mut meta,
                    founder,
                    &self.settings,
                )
            };
            if self.species.records().len() > species_before
                && let Some(pid) = parent_id
                && let Some(parent_record) = self.bibites.get_mut(pid)
            {
                // `bibiteBody.gene.species = species3` always; the member list
                // and the mutant flag only change for living bodies.
                parent_record.meta.species = Some(species);
                if !parent_record.body.dead {
                    parent_record.meta.is_mutant = false;
                }
            }
            self.species
                .on_birth(species, parent.unwrap_or(species), meta.generation);
            // `EggHatching.Hatch` (cs:185-194): the newborn's `up` is the egg's
            // `birthOrientation` (= -parent.up); with no orientation it gets a
            // random 1..359 degree rotation.
            let birth_dir = payload.birth_orientation;
            let rot = if birth_dir.length() > 0.01 {
                (-birth_dir.x).atan2(birth_dir.y)
            } else {
                (1 + self.rng.usize_below(359)) as f32 * std::f32::consts::PI / 180.0
            };
            let einstein = payload.brain.hidden_count() >= 100;
            let spawn = BibiteSpawn {
                pos: payload.pos,
                rot,
                vel: Vec2::ZERO,
                ang_vel: 0.0,
                genome: payload.genome,
                meta,
                brain: payload.brain,
                vitals: SpawnVitals::FromEgg,
                energy: Some(payload.energy),
                health: None,
            };
            if let Ok(child) = self.bibites.spawn(spawn) {
                if einstein {
                    self.einstein_births = self.einstein_births.saturating_add(1);
                }
                if self.hatched_children.len() >= HATCH_LOG_CAP {
                    self.hatched_children.remove(0);
                }
                self.hatched_children.push((id, child));
                self.randomize_sense_phase(child);
                self.data_logger.hatches_since_log += 1;
                self.register_bibite(child);
                self.stats.births_since_sample += 1;
                self.stats.births_total += 1;
            }
        }
        let meat_before = self.pellets.counts().meat_biomass;
        self.aborted.clear();
        self.aborted.extend(self.eggs.drain_aborted());
        for index in 0..self.aborted.len() {
            let (id, energy) = self.aborted[index];
            let (pos, size) = self
                .egg_positions
                .iter()
                .find(|(egg_id, _, _)| *egg_id == id)
                .map_or((Vec2::ZERO, 0.0), |(_, pos, size)| (*pos, *size));
            // `EggHatching.Abort` (cs:217): the pile spreads over `scale.x / 2`.
            let _ = self.pellets.spawn_meat_pile_with_radius(
                pos,
                energy,
                size / 2.0,
                &mut self.rng,
                self.settings.scenario.min_pellet_size,
            );
        }
        self.account_body_meat_creation(meat_before);
    }

    /// Phase 13: body physics bridge (also routes collision damage and rebuilds
    /// mouth target lists).
    pub fn step_physics(&mut self) {
        let parallel = self.threads > 1;
        if parallel {
            self.ensure_sensing_pool();
        }
        let executor = if parallel {
            crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref())
        } else {
            crate::sim::ParallelExecutor::Serial
        };
        let meat_before = self.pellets.counts().meat_biomass;
        self.bibites.step_physics_with_executor(
            &mut self.pellets,
            &mut self.eggs,
            &self.settings,
            &mut self.rng,
            &mut self.bibite_scratch,
            &mut self.events,
            executor,
        );
        self.account_body_meat_creation(meat_before);
    }

    /// Resolves world-inflicted kills (`kill` queues death intent) into corpses
    /// and `Death` events in the same tick.
    pub fn step_death_resolution(&mut self) {
        self.bibites.resolve_pending_deaths(
            &mut self.pellets,
            &self.settings,
            &mut self.rng,
            &mut self.events,
        );
    }

    /// `RadioZone`: every live `isRad` bibite carries a 250-unit trigger; any other
    /// bibite whose body overlaps it gets the permanent `RadZone` mutation factor.
    pub fn step_rad_zones(&mut self) {
        // Which bibites carry a zone is found by the pool (a scan over every record, almost always finding none).
        if self.threads > 1 {
            self.ensure_sensing_pool();
        }
        let mut rad: Vec<BibiteId> = Vec::new();
        self.bibites.refresh_summaries();
        // The store counts them as it refreshes: with none, there is nothing to scan for (almost always).
        if self.bibites.rad_bibites() > 0 {
            self.bibites.filter_map_dense_summaries_into(
                self.executor(),
                &mut rad,
                |id, summary, _, _| (summary.bits & SlotSummary::RAD != 0).then_some(id),
            );
        }
        #[cfg(test)]
        assert_eq!(
            rad,
            self.bibites
                .iter()
                .filter(|(_, record)| record.meta.is_rad && !record.body.dead)
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            "the parallel scan for rad bibites differs from the plain one"
        );
        let mut touched: Vec<BibiteId> = Vec::new();
        for id in rad {
            let Some(record) = self.bibites.get(id) else {
                continue;
            };
            let hits = self.hash.query_circle(
                record.pos,
                crate::body::RAD_ZONE_RADIUS,
                ClassMask::BIBITE,
                &mut self.query_scratch,
            );
            for hit in hits.hits {
                let other = BibiteId::from_handle(hit.handle);
                if other != id {
                    touched.push(other);
                }
            }
        }
        for id in touched {
            self.bibites.mark_rad_zone(id);
        }
    }

    /// Phase 9: radiation towers dose every bibite inside their radius.
    pub fn step_rad_towers(&mut self, dt: f32) {
        if !self
            .tools
            .tools()
            .iter()
            .any(|tool| matches!(tool, crate::tools::Tool::Rad(_)))
        {
            return;
        }
        self.rad_targets.clear();
        let mut max_radius = 0.0f32;
        for tool in self.tools.tools() {
            if let crate::tools::Tool::Rad(tower) = tool {
                max_radius = max_radius.max(tower.radius);
            }
        }
        if max_radius <= 0.0 {
            return;
        }
        for tower in self.tools.tools() {
            let crate::tools::Tool::Rad(tower) = tower else {
                continue;
            };
            let hits = self.hash.query_circle(
                tower.base.pos,
                tower.radius,
                ClassMask::BIBITE,
                &mut self.query_scratch,
            );
            for hit in hits.hits {
                let id = BibiteId::from_handle(hit.handle);
                if let Some(view) = self.bibites.view(id) {
                    if !view.dead {
                        self.rad_targets.push(BibiteTarget { id, pos: view.pos });
                    }
                }
            }
        }
        self.rad_targets
            .sort_unstable_by_key(|target| id_key(target.id));
        self.rad_targets.dedup_by_key(|target| id_key(target.id));
        let doses = self
            .tools
            .collect_rad_doses(dt, &self.zones, self.rad_targets.iter().copied());
        for dose in doses {
            self.bibites
                .add_mutation_increase(dose.id, dose.count, dose.intensity);
        }
    }

    /// Phase 10: Red Death bloom damage (escalating DoT outside `safeRadius`).
    pub fn step_red_death_phase(&mut self, dt: f32) {
        if !self.tools.step_red_death(dt) {
            return;
        }
        let bloom = self.tools.red_death;
        let safe_radius = bloom.safe_radius;
        // Bibites.
        self.ids_scratch.clear();
        self.ids_scratch.extend(self.bibites.ids());
        for index in 0..self.ids_scratch.len() {
            let id = self.ids_scratch[index];
            let pos = match self.bibites.get(id) {
                Some(record) => record.pos,
                None => continue,
            };
            if pos.length() < safe_radius {
                remove_acc(&mut self.red_bibite_acc, id);
                continue;
            }
            let accumulator = acc_entry(&mut self.red_bibite_acc, id, bloom.base_damage);
            let damage = bloom.damage_tick(accumulator);
            // Corpses stay in the bloom's damage list too: `Hurting` on a dead
            // body below zero health bursts it into meat
            // (`RedDeathBloomManager.cs:237-252`, `BibiteBody.cs:762-777`).
            let report = match self.bibites.get_mut(id) {
                Some(record) => record.body.hurt(damage),
                None => continue,
            };
            if report.exploded {
                self.bibites
                    .explode_corpse(id, &mut self.pellets, &mut self.rng, &self.settings);
                remove_acc(&mut self.red_bibite_acc, id);
            } else if report.died {
                self.bibites.kill(id, DeathCause::Hazard);
            }
        }
        // Pellets.
        self.pellet_ids_scratch.clear();
        self.pellet_ids_scratch.extend(self.pellets.ids());
        for index in 0..self.pellet_ids_scratch.len() {
            let id = self.pellet_ids_scratch[index];
            let Some(view) = self.pellets.get(id) else {
                continue;
            };
            if view.held {
                continue;
            }
            if view.pos.length() < safe_radius {
                remove_pellet_acc(&mut self.red_pellet_acc, id);
                continue;
            }
            let accumulator = pellet_acc_entry(&mut self.red_pellet_acc, id, bloom.base_damage);
            let damage = bloom.damage_tick(accumulator);
            self.pellets.change_amount(id, -damage);
        }
    }

    /// Phase 11: colour killers (`kill iff hue-distance >= tolerance`, captured
    /// semantics; see `tools.rs`).
    pub fn step_color_killers(&mut self, dt: f32) {
        if !self
            .tools
            .tools()
            .iter()
            .any(|tool| matches!(tool, crate::tools::Tool::Killer(_)))
        {
            return;
        }
        self.color_candidates.clear();
        for killer in self.tools.tools() {
            let crate::tools::Tool::Killer(killer) = killer else {
                continue;
            };
            let hits = self.hash.query_circle(
                killer.base.pos,
                // Colliders reach beyond their bibite's centre: look a body's length further, the touch test below decides.
                killer.radius + 40.0,
                ClassMask::BIBITE,
                &mut self.query_scratch,
            );
            for hit in hits.hits {
                let id = BibiteId::from_handle(hit.handle);
                let Some(view) = self.bibites.view(id) else {
                    continue;
                };
                if view.dead {
                    continue;
                }
                // `OverlapCircleNonAlloc` finds colliders, not centres: the circle must touch the bibite's own colliders.
                if !self.bibites.get(id).is_some_and(|record| record.touches_circle(killer.base.pos, killer.radius)) {
                    continue;
                }
                // `component.GetBodyColor()` is `GenesToColor(genes[5..=7])`, not the raw pigment
                // genes `view.color` holds for the vision cones (`isRain` gives `Color.clear`).
                self.color_candidates.push(ColorCandidate {
                    id,
                    pos: view.pos,
                    color: crate::genome::genes_to_color(
                        view.color[0],
                        view.color[1],
                        view.color[2],
                    ),
                    alpha: if view.is_rain { 0.0 } else { 1.0 },
                    eye_offset: view.eye_offset,
                });
            }
        }
        self.color_candidates
            .sort_unstable_by_key(|candidate| id_key(candidate.id));
        self.color_candidates
            .dedup_by_key(|candidate| id_key(candidate.id));
        let kills = self.tools.collect_color_kills(
            dt,
            &self.zones,
            self.color_candidates.iter().copied(),
            &mut self.rng,
        );
        for &id in kills {
            self.bibites.kill(id, DeathCause::Hazard);
        }
    }

    /// Phase 12: `isSource` bibites emit free plant pellets.
    ///
    /// `period = 0.5 / metabolicRate`, pellet energy = `0.1 · baseArea ·
    /// Plant.energyDensity`, emitted at `pos − up·8·d1Size` (`PelletProducer.cs`).
    pub fn step_pellet_producers(&mut self, dt: f32) {
        if !self.has_sources {
            return;
        }
        // Finding the sources is a look at every record (a cache miss on each, in a world of thousands), so the pool takes it
        // when the world is on it; the progress and the spawns that follow keep the dense order of the plain loop.
        let plant_density = self.settings.materials.plant.energy_density;
        let mut sources: Vec<(BibiteId, f32, f32, Vec2)> = Vec::new();
        self.bibites
            .filter_map_dense_into(self.executor(), &mut sources, |id, record| {
                if !record.meta.is_source || record.body.dead {
                    return None;
                }
                let period = 0.5 / record.genome.metabolic_rate().max(f32::EPSILON);
                let pellet_size = record.body.geometry.base_body_area * 0.1;
                let energy = pellet_size * plant_density;
                let pos = record.pos
                    - crate::math::unity::transform_up(record.rot)
                        * (8.0 * record.derived.d1_size);
                Some((id, period, energy, pos))
            });
        let any = !sources.is_empty();
        for (id, period, energy, pos) in sources {
            let progress = acc_entry_f32(&mut self.producer_progress, id);
            *progress += dt;
            // `PelletProducer.FixedUpdate` (cs:25-31) spawns at most one
            // pellet per fixed update.
            if *progress >= period {
                *progress -= period;
                let _ = self
                    .pellets
                    .spawn_with_energy(MaterialKind::Plant, energy, pos);
            }
        }
        if any {
            self.producer_progress
                .retain(|(id, _)| self.bibites.view(*id).is_some());
        }
    }

    /// Phase 14/15: drains body events into species/lifecycle stats.
    pub fn step_events_and_stats(&mut self, _dt: f32) {
        for index in 0..self.events.len() {
            let event = self.events[index];
            match event {
                BibiteEvent::Death { bibite, .. } => {
                    if let Some(record) = self.bibites.get(bibite) {
                        if let Some(species) = record.meta.species {
                            self.species.on_death(species);
                        }
                        // `BibiteTracker.BibiteDeath`: the age it died at (hours).
                        self.data_logger
                            .record_death_age(record.clock.time_alive / 3600.0);
                    }
                    self.stats.deaths_since_sample += 1;
                    self.stats.deaths_total += 1;
                    self.data_logger.deaths_since_log += 1;
                }
                BibiteEvent::EggLaid { egg, .. } => {
                    // Egg hash registration happens in the next hash sync.
                    let _ = egg;
                }
                BibiteEvent::Bitten { .. } | BibiteEvent::Swallowed { .. } => {}
            }
        }
        self.events.clear();
        // Deaths whose records were already removed (swallowed/unborn) cannot be
        // looked up by the event loop; reconcile per-species counts from the
        // authoritative live stream at the tick boundary.
        self.reconcile_species_counts();
        // `DataLogger` -> `GlobalLineageManager.LogSpeciesInfo` every 5 minutes: every species logs the
        // energy and the energy weighted position of its members.
        if self.species.advance_clock(_dt) {
            // `SpeciesPrunePointLimit` is read when the cleanup runs.
            self.species
                .set_prune_point_limit(self.settings.independents.species_prune_point_limit);
            let samples = self.species_samples();
            self.species.log_stats_with(&samples);
            // `ZoneManager.LogAllZonesData` follows `LogSpeciesInfo` in the same `DataLogger` tick.
            self.zones.log_all();
            self.apply_species_merges();
        }
    }

    /// Counts every species' members again from the live bibites and eggs.
    /// `Species.count = bibites.Count + eggs.Count` (Species.cs:81): a bibite leaves its species when it dies
    /// (`BibiteBody.Die`), and an egg is a member from `StartHatch` until it hatches or aborts. The game's count is
    /// always right; here the counts are kept at the tick boundary, so a world that was just loaded (or a panel that
    /// opens while the world is paused) asks for them first.
    pub fn reconcile_species_counts(&mut self) {
        if self.threads > 1 {
            self.ensure_sensing_pool();
        }
        let mut live: Vec<crate::ids::SpeciesId> = Vec::new();
        self.bibites.refresh_summaries();
        self.bibites.filter_map_dense_summaries_into(
            self.executor(),
            &mut live,
            |_, summary, _, _| {
                (summary.bits & SlotSummary::HAS_SPECIES != 0)
                    .then(|| crate::ids::SpeciesId::new(summary.species))
            },
        );
        #[cfg(test)]
        assert_eq!(
            live,
            self.bibites
                .iter()
                .filter(|(_, record)| !record.body.dead)
                .filter_map(|(_, record)| record.meta.species)
                .collect::<Vec<_>>(),
            "the parallel species recount differs from the plain one"
        );
        for id in self.eggs.ids() {
            if let Some(egg) = self.eggs.get(id)
                && !egg.hatch.aborting
                && let Some(species) = egg.meta.species
            {
                live.push(species);
            }
        }
        // Count the members of each species first: the registry finds a species by a linear search through every species ever
        // recorded, which is too much to ask of every bibite in every tick once there are hundreds of species.
        #[cfg(test)]
        let reference = {
            let mut reference = self.species.clone();
            reference.reconcile_counts(live.iter().copied());
            reference
        };
        let mut by_id: Vec<u32> = Vec::new();
        let mut wild: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        let limit = live.len().saturating_mul(4).saturating_add(4096);
        for species in &live {
            let at = species.value() as usize;
            if at < limit {
                if at >= by_id.len() {
                    by_id.resize(at + 1, 0);
                }
                by_id[at] += 1;
            } else {
                *wild.entry(species.value()).or_default() += 1;
            }
        }
        let counts = by_id
            .iter()
            .enumerate()
            .filter(|(_, members)| **members > 0)
            .map(|(at, members)| (crate::ids::SpeciesId::new(at as u32), *members))
            .chain(
                wild.into_iter()
                    .map(|(id, members)| (crate::ids::SpeciesId::new(id), members)),
            );
        self.species.reconcile_counts_by_species(counts);
        #[cfg(test)]
        assert_eq!(
            self.species
                .records()
                .iter()
                .map(|record| record.stats.count)
                .collect::<Vec<_>>(),
            reference
                .records()
                .iter()
                .map(|record| record.stats.count)
                .collect::<Vec<_>>(),
            "the recount by species differs from the recount by member"
        );
    }

    /// `new SpeciesDataPoint(species)` of every species that has members, as they are now (what the lineage mesh's
    /// "now" rungs are made of).
    #[must_use]
    pub fn species_points_now(
        &self,
    ) -> Vec<(crate::ids::SpeciesId, crate::species_log::SpeciesDataPoint)> {
        self.species_samples_of(None)
    }

    /// `new SpeciesDataPoint(species)` of every species that has members: the bibites' `totalEnergy` and the
    /// eggs' energy, and where they are.
    fn species_samples(
        &self,
    ) -> Vec<(crate::ids::SpeciesId, crate::species_log::SpeciesDataPoint)> {
        self.species_samples_of(None)
    }

    /// `new SpeciesDataPoint(species)`: the point of species `id` as it is now (the Position tab's "Now"); a
    /// species without members gives the empty point.
    #[must_use]
    pub fn species_point_now(
        &self,
        id: crate::ids::SpeciesId,
    ) -> crate::species_log::SpeciesDataPoint {
        self.species_samples_of(Some(id))
            .into_iter()
            .next()
            .map(|(_, point)| point)
            .unwrap_or_default()
    }

    /// The samples of every species, or of the one `only` names.
    fn species_samples_of(
        &self,
        only: Option<crate::ids::SpeciesId>,
    ) -> Vec<(crate::ids::SpeciesId, crate::species_log::SpeciesDataPoint)> {
        use crate::species_log::{SpeciesDataPoint, SpeciesMember};
        type Members = (Vec<SpeciesMember>, Vec<SpeciesMember>);
        let fat_density = self.settings.materials.fat.energy_density;
        let mut members: std::collections::BTreeMap<crate::ids::SpeciesId, Members> =
            std::collections::BTreeMap::new();
        for (_, record) in self.bibites.iter() {
            if record.body.dead {
                continue;
            }
            let Some(species) = record.meta.species else {
                continue;
            };
            if only.is_some_and(|only| only != species) {
                continue;
            }
            let extra = record.fat.amount * fat_density
                + record.egg_layer.progress * record.egg_layer.egg_energy;
            members.entry(species).or_default().0.push(SpeciesMember {
                energy: record.body.total_energy(extra),
                pos: [record.pos.x, record.pos.y],
            });
        }
        for id in self.eggs.ids() {
            let Some(egg) = self.eggs.get(id) else {
                continue;
            };
            if egg.hatch.aborting {
                continue;
            }
            let Some(species) = egg.meta.species else {
                continue;
            };
            if only.is_some_and(|only| only != species) {
                continue;
            }
            members.entry(species).or_default().1.push(SpeciesMember {
                energy: egg.hatch.energy,
                pos: [egg.pos.x, egg.pos.y],
            });
        }
        members
            .into_iter()
            .map(|(id, (bibites, eggs))| (id, SpeciesDataPoint::from_members(&bibites, &eggs)))
            .collect()
    }

    /// `PruneSpeciesFromTree` hands the bibites and the eggs of a pruned species to the one it was merged into.
    fn apply_species_merges(&mut self) {
        for (removed, survivor) in self.species.take_merges() {
            let bibites: Vec<_> = self.bibites.ids().collect();
            for id in bibites {
                if let Some(record) = self.bibites.get_mut(id)
                    && record.meta.species == Some(removed)
                {
                    record.meta.species = Some(survivor);
                }
            }
            let eggs: Vec<_> = self.eggs.ids().collect();
            for id in eggs {
                if let Some(egg) = self.eggs.get_mut(id)
                    && egg.meta.species == Some(removed)
                {
                    egg.meta.species = Some(survivor);
                }
            }
        }
    }

    /// `SpeciesInfoPanel.ActuallyPruneSpecies`: `PruneSpeciesFromTree` on a species, whose members go to the
    /// species it was merged into. Returns that species (the species itself when it has too many points to
    /// be pruned).
    pub fn prune_species(&mut self, id: crate::ids::SpeciesId) -> Option<crate::ids::SpeciesId> {
        let survivor = self.species.prune_from_tree(id, None);
        self.apply_species_merges();
        survivor
    }

    /// `GlobalLineageManager.LoadStateBin`: the species logs of the loaded `data.bin`, for the species the
    /// save lists (the block exists from game version 0.6.0a15 on; an older file keeps empty logs).
    fn load_species_logs(&mut self) {
        if !crate::data_bin::has_species_block(&self.scenario_version) {
            return;
        }
        let Some(blocks) = crate::data_bin::split_blocks(&self.data_bin) else {
            return;
        };
        let Some(block) = blocks.get(crate::data_bin::SPECIES_BLOCK) else {
            return;
        };
        let size = self.bounds.sim_size();
        let mut loaded = Vec::new();
        for (id, payload) in crate::data_bin::species_entries(block) {
            let Ok(id) = u32::try_from(id) else {
                continue;
            };
            let species = crate::ids::SpeciesId::new(id);
            if self.species.get(species).is_none() {
                continue;
            }
            if let Ok(log) = crate::species_log::SpeciesLog::load_state_bin(payload, size, false) {
                loaded.push((species, log));
            }
        }
        for (species, log) in loaded {
            self.species.set_log(species, log);
        }
        self.species.raise_max_energy_from_history();
    }

    /// `DataLogger.LoadStateBin` and `BibiteTracker.LoadStateBin`: the Graphs history of the loaded `data.bin` (from game version
    /// 0.6.0a9 on; an older file keeps an empty history), with the births and deaths the tracker had counted since the last point.
    fn load_logger_blocks(&mut self) {
        if !crate::data_bin::has_logger_history(&self.scenario_version) {
            return;
        }
        let Some(blocks) = crate::data_bin::split_blocks(&self.data_bin) else {
            return;
        };
        let blocks: Vec<Vec<u8>> = blocks.into_iter().map(<[u8]>::to_vec).collect();
        if let Some(block) = blocks.get(crate::data_bin::LOGGER_BLOCK) {
            self.data_logger.load_state_bin(block);
        }
        if let Some(block) = blocks.get(crate::data_bin::TRACKER_BLOCK)
            && let Some((births, deaths)) = self.data_logger.load_tracker_bin(block)
        {
            self.data_logger.hatches_since_log = u32::try_from(births).unwrap_or(0);
            self.data_logger.deaths_since_log = u32::try_from(deaths).unwrap_or(0);
        }
    }

    /// `ZoneManager.LoadStateBin`: the zone logs of the loaded `data.bin`, matched with the zones by their
    /// ids (the block exists from game version 0.6.0a10 on; an older file keeps empty logs).
    fn load_zone_logs(&mut self) {
        if !crate::data_bin::has_zone_block(&self.scenario_version) {
            return;
        }
        let Some(blocks) = crate::data_bin::split_blocks(&self.data_bin) else {
            return;
        };
        let Some(block) = blocks.get(crate::data_bin::ZONE_BLOCK) else {
            return;
        };
        let mut loaded = Vec::new();
        for (id, payload) in crate::data_bin::zone_entries(block) {
            let Ok(id) = u32::try_from(id) else {
                continue;
            };
            let Some(index) = self
                .zones
                .zones
                .iter()
                .position(|zone| zone.config.runtime_zone_id() == id)
            else {
                continue;
            };
            if let Ok(log) = crate::zone_log::ZoneLog::load_state_bin(payload) {
                loaded.push((index, log));
            }
        }
        for (index, log) in loaded {
            self.zones.zones[index].log = log;
        }
    }

    /// The species block of the `data.bin` a save carries, written afresh from the logs.
    fn species_block_for_save(&self, loaded: Option<&[&[u8]]>) -> Vec<u8> {
        let size = self.bounds.sim_size();
        // A log that has not changed since it was loaded is written back as the game wrote it (saving
        // rounds the positions, so a rewrite of an untouched log would drift by a unit).
        let originals: std::collections::HashMap<i64, &[u8]> = loaded
            .and_then(|blocks| blocks.get(crate::data_bin::SPECIES_BLOCK).copied())
            .map(|block| {
                crate::data_bin::species_entries(block)
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        let entries: Vec<(i64, Vec<u8>)> = self
            .species
            .records()
            .iter()
            .map(|record| {
                let id = i64::from(record.id.value());
                let untouched = originals.get(&id).filter(|bytes| {
                    crate::species_log::SpeciesLog::load_state_bin(bytes, size, false)
                        .is_ok_and(|loaded| loaded == record.stats.log)
                });
                let payload = untouched.map_or_else(
                    || record.stats.log.save_state_bin(size),
                    |bytes| bytes.to_vec(),
                );
                (id, payload)
            })
            .collect();
        crate::data_bin::species_block(&entries)
    }

    /// The zone block of the `data.bin` a save carries: every zone's log in zone order (the game writes the
    /// bytes it loaded back for a log that has not changed).
    fn zone_block_for_save(&self, loaded: Option<&[&[u8]]>) -> Vec<u8> {
        let originals: std::collections::HashMap<i32, &[u8]> = loaded
            .and_then(|blocks| blocks.get(crate::data_bin::ZONE_BLOCK).copied())
            .map(|block| crate::data_bin::zone_entries(block).into_iter().collect())
            .unwrap_or_default();
        let entries: Vec<(i32, Vec<u8>)> = self
            .zones
            .zones
            .iter()
            .map(|zone| {
                let id = zone.config.runtime_zone_id() as i32;
                let untouched = originals.get(&id).filter(|bytes| {
                    crate::zone_log::ZoneLog::load_state_bin(bytes)
                        .is_ok_and(|loaded| loaded == zone.log)
                });
                let payload =
                    untouched.map_or_else(|| zone.log.save_state_bin(), |bytes| bytes.to_vec());
                (id, payload)
            })
            .collect();
        crate::data_bin::zone_block(&entries)
    }

    /// The `data.bin` a save carries: the loaded one with its species and zone blocks written afresh from
    /// the logs, or (a world that was not loaded from a game save) a blank history around those blocks once a
    /// species or a zone has been logged.
    fn data_bin_for_save(&self) -> Vec<u8> {
        let loaded = crate::data_bin::split_blocks(&self.data_bin);
        let species = self.species_block_for_save(loaded.as_deref());
        let zones = self.zone_block_for_save(loaded.as_deref());
        // The Graphs history and the tracker, written afresh from the logger.
        let mut logger = self.data_logger.clone();
        let births = i32::try_from(logger.hatches_since_log).unwrap_or(i32::MAX);
        let deaths = i32::try_from(logger.deaths_since_log).unwrap_or(i32::MAX);
        let logger_block = logger.save_state_bin();
        let tracker_block = logger.save_tracker_bin(births, deaths);
        if !self.data_bin.is_empty() {
            let mut data = self.data_bin.clone();
            for (index, block) in [
                (crate::data_bin::LOGGER_BLOCK, &logger_block),
                (crate::data_bin::TRACKER_BLOCK, &tracker_block),
            ] {
                if let Some(replaced) = crate::data_bin::replace_block(&data, index, block) {
                    data = replaced;
                }
            }
            if let Some(replaced) =
                crate::data_bin::replace_block(&data, crate::data_bin::SPECIES_BLOCK, &species)
            {
                data = replaced;
            }
            if let Some(replaced) =
                crate::data_bin::replace_block(&data, crate::data_bin::ZONE_BLOCK, &zones)
            {
                data = replaced;
            }
            return data;
        }
        let logged = self.data_logger.has_history()
            || self
                .species
                .records()
                .iter()
                .any(|record| record.stats.log.total_index_of_apparition() >= 0)
            || self
                .zones
                .zones
                .iter()
                .any(|zone| zone.log.total_index_of_apparition() >= 0);
        if logged {
            crate::data_bin::assemble(&logger_block, &tracker_block, &species, &zones)
        } else {
            Vec::new()
        }
    }

    /// Reconciles every spatial-hash class with its store.
    pub fn sync_all_hashes(&mut self) {
        self.sync_bibite_hash();
        self.sync_pellet_hash();
        self.sync_egg_hash();
        self.sync_pheromone_hash();
    }

    /// Removes spatial-hash pellet entries whose pellets are no longer live and
    /// updates moved pellet poses/radii.
    ///
    /// Only the pellets the store marked since the last sync (spawned, moved, resized or gone) are visited, in the order
    /// the full pass would have reached them; the full pass does nothing to the others. In tests every sync is checked
    /// against the full pass on a copy of the hash (the whole structure, down to the order inside every cell).
    pub fn sync_pellet_hash(&mut self) {
        let mut dirty = std::mem::take(&mut self.pellet_dirty_scratch);
        self.pellets.take_hash_dirty(&mut dirty);
        #[cfg(test)]
        let expected = (self.pellets.len() <= 20_000).then(|| {
            let mut hash = self.hash.clone();
            full_pellet_hash_sync(&mut hash, &self.pellets);
            hash
        });
        // What `retain` removes: the entries of pellets that are gone, in slot order.
        dirty.sort_unstable();
        for &slot in &dirty {
            if let Some(handle) = self
                .hash
                .handle_at_slot(SpatialClass::Pellet, slot as usize)
                && self.pellets.get(PelletId::from_handle(handle)).is_none()
            {
                self.hash.remove(handle, SpatialClass::Pellet);
            }
        }
        // What the full pass does to the others: the live ones, in dense order.
        dirty.retain(|&slot| self.pellets.dense_position(slot as usize).is_some());
        dirty.sort_unstable_by_key(|&slot| self.pellets.dense_position(slot as usize));
        for &slot in &dirty {
            if let Some((id, pos, radius)) = self.pellets.hash_entry(slot as usize) {
                self.hash
                    .update(id.handle(), SpatialClass::Pellet, pos, radius);
            }
        }
        dirty.clear();
        self.pellet_dirty_scratch = dirty;
        #[cfg(test)]
        if let Some(expected) = expected {
            assert!(
                self.hash.same_as(&expected),
                "the incremental pellet hash sync differs from the full pass"
            );
        }
    }

    /// [`Self::sync_pellet_hash`] over every pellet (after the hash was rebuilt or replaced).
    pub fn sync_pellet_hash_full(&mut self) {
        full_pellet_hash_sync(&mut self.hash, &self.pellets);
        self.pellets.clear_hash_dirty();
    }

    /// Removes spatial-hash egg entries whose eggs are gone and updates poses.
    pub fn sync_egg_hash(&mut self) {
        let eggs = &self.eggs;
        self.hash.retain(SpatialClass::Egg, |handle| {
            eggs.get(EggId::from_handle(handle)).is_some()
        });
        for id in eggs.ids() {
            if let Some(egg) = eggs.get(id) {
                self.hash
                    .update(id.handle(), SpatialClass::Egg, egg.pos, EGG_HASH_RADIUS);
            }
        }
    }

    /// Explicit census: shade-culls out-of-bounds pellets and recounts zone
    /// biomass (called on save/sim-size change and by `World::census_and_cull`).
    pub fn census_and_cull(&mut self) -> usize {
        let mut culled = 0;
        self.pellet_ids_scratch.clear();
        self.pellet_ids_scratch.extend(self.pellets.ids());
        for index in 0..self.pellet_ids_scratch.len() {
            let id = self.pellet_ids_scratch[index];
            let Some(view) = self.pellets.get(id) else {
                continue;
            };
            if view.held {
                continue;
            }
            if self.bounds.pellet_out_of_bounds(view.pos) {
                let energy = f64::from(self.pellets.energy(id).unwrap_or(0.0));
                match self.pellets.material(id) {
                    Some(crate::pellet::MaterialKind::Meat) => {
                        self.accounting.culled_meat_energy += energy;
                    }
                    _ => self.accounting.culled_plant_energy += energy,
                }
                self.pellets.remove(id);
                culled += 1;
            }
        }
        let executor = crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref());
        self.zones
            .recount_all_with(&self.pellets, executor, self.active_gates.scans);
        self.sync_pellet_hash();
        culled
    }

    /// Phase 4: zone movement + production/recycling/recount (spec §1.4–§1.7).
    pub fn step_zones(&mut self, dt: f32) -> ZoneUpdateReport {
        self.zones.update_movement(dt);
        let safe_radius = self
            .tools
            .red_death
            .enabled
            .then_some(self.tools.red_death.safe_radius);
        let executor = crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref());
        let report = self.zones.update_phase_with(
            dt,
            &mut self.pellets,
            &mut self.hash,
            safe_radius,
            executor,
            self.active_gates.scans,
        );
        self.accounting.produced_plant_energy += report.produced_plant_energy;
        self.accounting.produced_meat_energy += report.produced_meat_energy;
        self.accounting.recycled_plant_energy += report.recycled_plant_energy;
        self.accounting.recycled_meat_energy += report.recycled_meat_energy;
        report
    }

    /// Phase 3 + pheromone-tower part of phase 7: snaps attached towers to their
    /// zones and keeps each pheromone tower's persistent spot alive at the
    /// tower position (spec §6.3/§9.2).
    ///
    /// Rad dosing (phase 9), Red Death damage (phase 10) and colour killers
    /// (phase 11) need the body store; their data collection lives in
    /// [`Self::collect_rad_doses`] / [`Self::collect_color_kills`] and is wired
    /// into the tick in W4 once B2 lands.
    pub fn step_tools(&mut self, _dt: f32) {
        self.tools.snap_attached(&self.zones);
        let update = self
            .tools
            .step_phero_towers(&self.zones, &mut self.pheromones);
        // Remove old entries first: the store may reuse the same slot for the
        // replacement spot, and the hash keys entries by (slot, generation).
        for &spot in update.removed {
            self.hash.remove(spot.handle(), SpatialClass::Pheromone);
        }
        for &(spot, pos) in update.added {
            self.hash.insert(
                spot.handle(),
                SpatialClass::Pheromone,
                pos,
                pheromone_spot_radius(true),
            );
        }
    }

    /// Spawns one user-placed pellet from an explicit amount in u²
    /// (`SpawnPelletOfMatter(amount=...)` path). Energy is derived from the
    /// material table and booked as produced.
    pub fn spawn_pellet_with_amount(
        &mut self,
        material: MaterialKind,
        pos: Vec2,
        amount: f32,
    ) -> Option<PelletId> {
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        let id = self.spawn_pellet(PelletSpawn::new(material, amount, pos))?;
        let energy = self.pellets.energy(id).unwrap_or(0.0) as f64;
        match material {
            MaterialKind::Meat => self.accounting.produced_meat_energy += energy,
            _ => self.accounting.produced_plant_energy += energy,
        }
        Some(id)
    }

    /// Spawns one user-placed pellet (`PelletPlacer` single click). Routes the
    /// energy through the world's production ledger and registers the hash.
    pub fn spawn_pellet_with_energy(
        &mut self,
        material: MaterialKind,
        pos: Vec2,
        energy: f32,
    ) -> Option<PelletId> {
        let id = self.pellets.spawn_with_energy(material, energy, pos)?;
        self.register_pellet(id);
        let actual = self.pellets.energy(id).unwrap_or(0.0) as f64;
        match material {
            MaterialKind::Meat => self.accounting.produced_meat_energy += actual,
            _ => self.accounting.produced_plant_energy += actual,
        }
        Some(id)
    }

    /// User-placed pellet pile (`PelletPlacer` multi-pellet mode): `count`
    /// pellets with `energy ±25%` scattered uniformly inside `pile_radius`.
    ///
    /// `count` is bounded to 1..=1000 to keep a viewer action from allocating an
    /// unbounded pile; the game UI exposes 1..=10.
    pub fn spawn_pellet_pile(
        &mut self,
        material: MaterialKind,
        pos: Vec2,
        count: usize,
        energy_per_pellet: f32,
        pile_radius: f32,
    ) -> Vec<PelletId> {
        if !energy_per_pellet.is_finite()
            || energy_per_pellet <= 0.0
            || !pile_radius.is_finite()
            || pile_radius < 0.0
        {
            return Vec::new();
        }
        let count = count.min(1000);
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let offset = random_inside_unit_circle(&mut self.rng) * pile_radius;
            let factor = self.rng.f32_range(0.75, 1.25);
            if let Some(id) =
                self.spawn_pellet_with_energy(material, pos + offset, energy_per_pellet * factor)
            {
                ids.push(id);
            }
        }
        ids
    }

    /// Pile radius of the pellet placer (`PelletPlacer.PlacementEvent`):
    /// `sqrt(2 * count * amountOfEnergy(E) / pi)` with
    /// `E = scenario.pelletEnergy * size_factor`.
    #[must_use]
    pub fn pellet_placer_pile_radius(
        &self,
        material: MaterialKind,
        count: usize,
        size_factor: f32,
    ) -> f32 {
        let energy = self.settings.scenario.pellet_energy * size_factor;
        let amount = self
            .pellets
            .materials()
            .get(material)
            .amount_of_energy(energy);
        (2.0 * count as f32 * amount / std::f32::consts::PI)
            .max(0.0)
            .sqrt()
    }

    /// `PelletPlacer.PlacementEvent` (cs:131-150): one pellet lands exactly on
    /// the click; several are scattered inside the pile radius. Each pellet's
    /// energy is `pelletEnergy * size_factor * U(0.75, 1.25)`.
    pub fn place_pellets_like_game(
        &mut self,
        material: MaterialKind,
        pos: Vec2,
        count: usize,
        size_factor: f32,
    ) -> Vec<PelletId> {
        let energy = self.settings.scenario.pellet_energy * size_factor;
        if !energy.is_finite() || energy <= 0.0 {
            return Vec::new();
        }
        let count = count.clamp(1, 10);
        let pile = self.pellet_placer_pile_radius(material, count, size_factor);
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let at = if count <= 1 {
                pos
            } else {
                pos + random_inside_unit_circle(&mut self.rng) * pile
            };
            let jitter = self.rng.f32_range(0.75, 1.25);
            if let Some(id) = self.spawn_pellet_with_energy(material, at, energy * jitter) {
                ids.push(id);
            }
        }
        ids
    }

    /// Places a rad tower at `pos` (viewer tool action), optionally attached to
    /// a zone so it follows that zone. Returns the new tool's id.
    pub fn place_rad_tower_at(&mut self, pos: Vec2, attached_zone: Option<ZoneId>) -> ToolId {
        let save_id = self.tools.next_save_id();
        let mut tower = RadTower::new(save_id, pos, self.bounds.sim_size());
        tower.base.attached_zone = attached_zone;
        let id = self.tools.add_rad_tower(tower);
        self.tools.snap_attached(&self.zones);
        id
    }

    /// Places a pheromone tower at `pos` (viewer tool action); creates its
    /// persistent pheromone spot immediately.
    pub fn place_phero_tower_at(&mut self, pos: Vec2, attached_zone: Option<ZoneId>) -> ToolId {
        let save_id = self.tools.next_save_id();
        let mut tower = PheroTower::new(save_id, pos);
        tower.base.attached_zone = attached_zone;
        let id = self.tools.add_phero_tower(tower);
        self.step_tools(0.0);
        id
    }

    /// Places a colour killer at `pos` (viewer tool action).
    pub fn place_color_killer_at(&mut self, pos: Vec2, attached_zone: Option<ZoneId>) -> ToolId {
        let save_id = self.tools.next_save_id();
        let mut killer = ColorKiller::new(save_id, pos);
        killer.base.attached_zone = attached_zone;
        let id = self.tools.add_color_killer(killer);
        self.tools.snap_attached(&self.zones);
        id
    }

    /// Typed rad-tower property setter (`RadTowerPanel`: count, intensity,
    /// radius, taper). Radius is clamped to the game's `[10, simSize/2]`.
    pub fn set_rad_tower_params(
        &mut self,
        id: ToolId,
        count_rate: f32,
        intensity_rate: f32,
        radius: f32,
        taper: f32,
    ) -> bool {
        let sim_size = self.bounds.sim_size();
        let Some(crate::tools::Tool::Rad(tower)) = self.tools.get_mut(id) else {
            return false;
        };
        if !count_rate.is_finite() || !intensity_rate.is_finite() || !taper.is_finite() {
            return false;
        }
        tower.count_rate = count_rate.max(0.0);
        tower.intensity_rate = intensity_rate.max(0.0);
        tower.radius = radius;
        tower.taper = taper;
        tower.clamp_radius(sim_size);
        true
    }

    /// Typed pheromone-tower property setter (`PheroTowerPanel`: RGB strengths
    /// + heading in radians).
    pub fn set_phero_tower_params(
        &mut self,
        id: ToolId,
        strengths: [f32; 3],
        heading: f32,
    ) -> bool {
        let Some(crate::tools::Tool::Phero(tower)) = self.tools.get_mut(id) else {
            return false;
        };
        if strengths.iter().any(|value| !value.is_finite()) || !heading.is_finite() {
            return false;
        }
        tower.red = strengths[0].max(0.0);
        tower.green = strengths[1].max(0.0);
        tower.blue = strengths[2].max(0.0);
        tower.heading = heading;
        true
    }

    /// Typed colour-killer property setter (`ColorKillerPanel`: hue, tolerance,
    /// body/eye mode, any/most selection, radius, period).
    // One argument per ColorKillerPanel control; a parameter struct would only
    // mirror the UI form one-for-one, so keep the narrow allow.
    #[allow(clippy::too_many_arguments)]
    pub fn set_color_killer_params(
        &mut self,
        id: ToolId,
        hue: f32,
        tolerance: f32,
        body_or_eye: bool,
        any_or_most: bool,
        radius: f32,
        period: f32,
    ) -> bool {
        let Some(crate::tools::Tool::Killer(killer)) = self.tools.get_mut(id) else {
            return false;
        };
        if !hue.is_finite()
            || !tolerance.is_finite()
            || !radius.is_finite()
            || !period.is_finite()
            || radius < 0.0
            || period <= 0.0
        {
            return false;
        }
        killer.hue = hue.rem_euclid(1.0);
        killer.tolerance = tolerance.max(0.0);
        killer.body_or_eye = body_or_eye;
        killer.any_or_most = any_or_most;
        killer.radius = radius;
        killer.period = period;
        true
    }

    /// Deletes a placed tool (viewer destroy action), removing a pheromone
    /// tower's spot/hash entry as well. Later `ToolId`s shift down by one.
    pub fn remove_tool(&mut self, id: ToolId) -> bool {
        if let Some(crate::tools::Tool::Phero(tower)) = self.tools.get(id) {
            if let Some(spot) = tower.spot {
                self.hash.remove(spot.handle(), SpatialClass::Pheromone);
                self.pheromones.remove(spot);
            }
        }
        self.tools.remove_tool(id).is_some()
    }

    /// Moves a placed tool (`DraggableObject.OnMouseDrag` on a tower or colour killer, which have
    /// no rigid body: the transform jumps to the cursor). A tool that follows a zone cannot be
    /// moved, since `FixedUpdate` puts it back on the zone every tick. A pheromone tower's spot
    /// follows at once. Returns whether the tool moved.
    pub fn set_tool_position(&mut self, id: ToolId, pos: Vec2) -> bool {
        if !pos.x.is_finite() || !pos.y.is_finite() {
            return false;
        }
        let Some(tool) = self.tools.get_mut(id) else {
            return false;
        };
        let base = tool.base_mut();
        if base.attached_zone.is_some() {
            return false;
        }
        base.pos = pos;
        if matches!(self.tools.get(id), Some(crate::tools::Tool::Phero(_))) {
            self.step_tools(0.0);
        }
        true
    }

    /// Sets (or clears) a tower's attached zone and snaps it immediately.
    pub fn set_tool_attached_zone(&mut self, id: ToolId, zone: Option<ZoneId>) -> bool {
        let Some(tool) = self.tools.get_mut(id) else {
            return false;
        };
        tool.base_mut().attached_zone = zone;
        self.tools.snap_attached(&self.zones);
        // A pheromone tower's persistent spot follows its position; recreate it
        // immediately so the viewer sees the move before the next tick.
        if matches!(self.tools.get(id), Some(crate::tools::Tool::Phero(_))) {
            self.step_tools(0.0);
        }
        true
    }

    /// Adds a live zone (`ZonesEditorPanel.AddZone`): validates settings and adds a new zone with a fresh per-zone RNG stream; it goes after
    /// the other explicit zones and in front of the groups' zones (`ScenarioSettings.UpdateAllZonesList`). The new zone owns no pellets yet;
    /// target/attached references use scenario file ids as usual.
    pub fn add_zone(&mut self, settings: settings::ZoneSettings) -> Result<ZoneId> {
        settings.validate()?;
        let config = crate::zone::ZoneConfig::from_settings(&settings);
        let at = self.zones.explicit_count();
        if at == self.zones.zones.len() {
            // Nothing comes after it: the zones keep their ids.
            let id = self.zones.add_zone(config);
            self.refresh_zone_circles();
            return Ok(id);
        }
        self.splice_zones(at..at, vec![config], None);
        self.zones
            .zone_id_at(at)
            .ok_or_else(|| Error::invalid_data("zone not added"))
    }

    /// Replaces the zones at `range` by `configs` (which belong to group `group`, `None`: explicit) and brings everything that refers to
    /// zones along: the pellets the zones that stay own, the towers attached to them, the spawn rows limited to one and the targets of the
    /// groups' templates. The removed zones' pellets are destroyed. Returns how many pellets went and where each old zone is now.
    fn splice_zones(
        &mut self,
        range: core::ops::Range<usize>,
        configs: Vec<crate::zone::ZoneConfig>,
        group: Option<u32>,
    ) -> (usize, Vec<Option<usize>>) {
        let old_ids: Vec<ZoneId> = (0..self.zones.zones.len())
            .filter_map(|index| self.zones.zone_id_at(index))
            .collect();
        let old_files: Vec<u32> = self
            .zones
            .zones
            .iter()
            .map(|zone| zone.config.runtime_zone_id())
            .collect();
        let (removed, mapping) = self.zones.splice_zones(range, configs, group);
        let mut destroyed = 0usize;
        for zone in removed {
            for pellet in zone.owned_pellets {
                if self.pellets.contains(pellet) {
                    self.despawn_pellet(pellet);
                    destroyed += 1;
                }
            }
        }
        // The zones that stayed have new handles: their pellets own them again.
        for new in mapping.iter().flatten().copied() {
            let Some(id) = self.zones.zone_id_at(new) else {
                continue;
            };
            for pellet in self.zones.zones[new].owned_pellets.clone() {
                self.pellets.set_owner_zone(pellet, Some(id));
            }
        }
        for tool in self.tools.tools_mut() {
            let base = tool.base_mut();
            base.attached_zone = base.attached_zone.and_then(|old| {
                let old_index = old_ids.iter().position(|candidate| *candidate == old)?;
                let new_index = mapping[old_index]?;
                self.zones.zone_id_at(new_index)
            });
        }
        let renumber = |file: u32| -> Option<u32> {
            let old_index = old_files.iter().position(|candidate| *candidate == file)?;
            mapping[old_index].map(|index| index as u32)
        };
        for row in &mut self.bibite_spawns {
            row.spawn_zone = row.spawn_zone.and_then(renumber);
        }
        for group in &mut self.zone_groups {
            group.template.target = group.template.target.and_then(renumber);
        }
        self.refresh_zone_circles();
        (destroyed, mapping)
    }

    /// `ZoneGroupSettings.GenerateZones`: the group's zones go and `count` new ones, copies of its template named `<group>_<i>`, appear
    /// at random places. Returns where each zone of the old list is now (the removed ones: `None`).
    pub fn generate_zone_group(&mut self, group: usize) -> Result<Vec<Option<usize>>> {
        let Some(settings) = self.zone_groups.get(group) else {
            return Err(Error::invalid_data("unknown zone group"));
        };
        let sim_size = self.bounds.sim_size();
        let count = settings.count(sim_size) as usize;
        let template = settings.template.clone();
        let name = settings.name.clone();
        let mut configs = Vec::with_capacity(count);
        for index in 0..count {
            let mut config = crate::zone::ZoneConfig::from_settings(&template);
            config.name = format!("{name}_{index}");
            config.randomize_position(&mut self.rng, sim_size);
            configs.push(config);
        }
        let (start, len) = self.zones.group_span(group as u32);
        let (_, mapping) = self.splice_zones(start..start + len, configs, Some(group as u32));
        Ok(mapping)
    }

    /// `ZonesEditorPanel.AddZoneGroup`: a new group (its zones generated at once) joins the others. Returns its index.
    pub fn add_zone_group(&mut self, mut group: settings::ZoneGroupSettings) -> Result<usize> {
        group.validate()?;
        group.zones.clear();
        // (The handle that shows the group makes its two ways of counting agree at once.)
        group.align_count_types(self.bounds.sim_size());
        self.zone_groups.push(group);
        let index = self.zone_groups.len() - 1;
        self.generate_zone_group(index)?;
        Ok(index)
    }

    /// `ZonesEditorPanel.GroupDeleted`: the group and its zones go (the zones' pellets with them). Returns where each zone of the old list
    /// is now.
    pub fn remove_zone_group(&mut self, group: usize) -> Result<Vec<Option<usize>>> {
        if group >= self.zone_groups.len() {
            return Err(Error::invalid_data("unknown zone group"));
        }
        let (start, len) = self.zones.group_span(group as u32);
        let (_, mapping) = self.splice_zones(start..start + len, Vec::new(), None);
        self.zone_groups.remove(group);
        for zone in &mut self.zones.zones {
            if let Some(owner) = zone.group
                && owner as usize > group
            {
                zone.group = Some(owner - 1);
            }
        }
        Ok(mapping)
    }

    /// Edits group `group`'s settings the way the game's handles react to them: a new name renames its zones
    /// (`ZoneGroupSettings.RenameZones`), the two ways of counting stay in step (`AlignCountTypes`) and the zones are generated again when
    /// their number is not the count any more (`ZoneGroupHandle.OnCountChange`), and a changed template goes to every zone of the group
    /// (`ApplyChangesToAllZones`). Returns where each zone of the old list is now when the zones were generated again.
    pub fn edit_zone_group(
        &mut self,
        group: usize,
        edit: impl FnOnce(&mut settings::ZoneGroupSettings),
    ) -> Result<Option<Vec<Option<usize>>>> {
        let Some(before) = self.zone_groups.get(group) else {
            return Err(Error::invalid_data("unknown zone group"));
        };
        let sim_size = self.bounds.sim_size();
        let mut after = before.clone();
        edit(&mut after);
        after.zones.clear();
        after.validate()?;
        after.align_count_types(sim_size);
        let renamed = after.name != before.name;
        let template_changed = after.template != before.template;
        self.zone_groups[group] = after;
        let (start, len) = self.zones.group_span(group as u32);
        let wanted = self.zone_groups[group].count(sim_size) as usize;
        if len != wanted {
            return self.generate_zone_group(group).map(Some);
        }
        if renamed {
            let name = self.zone_groups[group].name.clone();
            for (offset, zone) in self.zones.zones[start..start + len].iter_mut().enumerate() {
                zone.config.name = format!("{name}_{offset}");
            }
        }
        if template_changed {
            self.apply_zone_group_template(group)?;
        }
        Ok(None)
    }

    /// `ZoneGroupSettings.ApplyChangesToAllZones`: every zone of the group takes the template's settings (not its own name or place).
    pub fn apply_zone_group_template(&mut self, group: usize) -> Result<()> {
        let Some(settings) = self.zone_groups.get(group) else {
            return Err(Error::invalid_data("unknown zone group"));
        };
        let template = settings.template.clone();
        let (start, len) = self.zones.group_span(group as u32);
        for index in start..start + len {
            let Some(id) = self.zones.zone_id_at(index) else {
                continue;
            };
            let mut zone = self.zones.zones[index].config.0.clone();
            template.copy_template_into(&mut zone);
            self.apply_zone_settings(id, zone)?;
        }
        Ok(())
    }

    /// Removes a live zone (scenario editor). Its owned pellets are despawned
    /// (booked as removed energy), attached towers are detached or remapped to
    /// the renumbered zone, and spawner rows targeting its file id fall back to
    /// `None` (all zones). All previously returned `ZoneId`s become stale; the
    /// result is the number of destroyed pellets.
    pub fn remove_zone(&mut self, id: ZoneId) -> Result<usize> {
        let Some(index) = self.zones.index_of(id) else {
            return Err(Error::invalid_data("unknown zone id"));
        };
        // (The towers attached to it are detached, the spawn rows limited to it go back to "all zones", the others keep their zone.)
        let (destroyed, _) = self.splice_zones(index..index + 1, Vec::new(), None);
        Ok(destroyed)
    }

    /// Live zone editor apply (Settings/zone panel): replaces an existing
    /// zone's configuration in place, refreshes its derived area/biomass and
    /// snaps attached towers. Adding/removing zones is still deferred (per-zone
    /// RNG streams, owned pellets, tower/spawn-row remapping).
    pub fn apply_zone_settings(
        &mut self,
        id: ZoneId,
        settings: settings::ZoneSettings,
    ) -> Result<()> {
        settings.validate()?;
        let Some(index) = self.zones.zones.iter().position(|zone| zone.id == id) else {
            return Err(Error::invalid_data("unknown zone id"));
        };
        let mut config = crate::zone::ZoneConfig::from_settings(&settings);
        let bounds = self.bounds;
        let globals = self.zones.globals.clone();
        let sim_size = bounds.sim_size();
        config.cap_position(sim_size);
        let old = &self.zones.zones[index].config;
        let moved = old.pos_x != config.pos_x || old.pos_y != config.pos_y;
        self.zones.zones[index].config = config;
        self.zones.zones[index].refresh(&bounds, &globals);
        // A new movement type or target makes the zone follow (or stop following) another one.
        self.zones.resolve_all_targets();
        // New posX / posY put the zone there at once (`Zone.UpdatePosition`) with its followers.
        if moved {
            let config = &self.zones.zones[index].config;
            let pos = Vec2::new(sim_size * config.pos_x, sim_size * config.pos_y);
            self.zones.teleport(index, pos);
        }
        self.refresh_zone_circles();
        self.tools.snap_attached(&self.zones);
        Ok(())
    }

    /// Viewer kill action: marks a live bibite for death (death/meat resolution
    /// happens on the next body tick). Returns `false` for missing/dead/corpse
    /// targets; exploding an already-dead corpse is not yet a core API.
    pub fn kill_bibite(&mut self, id: BibiteId) -> bool {
        let Some(record) = self.bibites.get(id) else {
            return false;
        };
        if record.body.dead || record.body.dying {
            return false;
        }
        self.bibites.kill(id, DeathCause::Hazard);
        true
    }

    /// `UserActions.KillHalfBibites` (cs:29-49): every bibite and every egg
    /// independently has a 50 % chance of dying / being aborted. Returns
    /// `(bibites_killed, eggs_aborted)`.
    pub fn kill_random_half(&mut self) -> (usize, usize) {
        let bibites: Vec<BibiteId> = self.bibites.ids().collect();
        let mut killed = 0;
        for id in bibites {
            if self.rng.f32() < 0.5 && self.kill_bibite(id) {
                killed += 1;
            }
        }
        let eggs: Vec<EggId> = self.eggs.ids().collect();
        let mut aborted = 0;
        for id in eggs {
            if self.rng.f32() < 0.5 && self.abort_egg(id) {
                aborted += 1;
            }
        }
        (killed, aborted)
    }

    /// `UserActions.RemoveHalfPellets` (cs:52-62): every pellet is removed when `Random.value >=
    /// 0.5`, then `ZoneManager.RefreshPelletBiomassCounter` recounts the zones. Returns how many
    /// pellets went.
    pub fn remove_random_half_pellets(&mut self) -> usize {
        let ids: Vec<PelletId> = self.pellets.ids().collect();
        let mut removed = 0;
        for id in ids {
            if self.rng.f32() >= 0.5 {
                self.despawn_pellet(id);
                removed += 1;
            }
        }
        self.census_and_cull();
        removed
    }

    /// `BibiteStatsPanel.KillCurrentBibite` (cs:237-258): a living bibite is
    /// killed, an already dead one is exploded into meat. Returns `false` when
    /// the id is unknown or the bibite is already dying and not yet a corpse.
    pub fn kill_or_explode_bibite(&mut self, id: BibiteId) -> bool {
        let Some(record) = self.bibites.get(id) else {
            return false;
        };
        if !record.body.dead {
            return self.kill_bibite(id);
        }
        self.bibites
            .explode_corpse(id, &mut self.pellets, &mut self.rng, &self.settings)
    }

    /// Viewer egg abort action (`EggHatching.Abort`): sets the hatching abort
    /// flag; the next egg phase removes it and spawns the meat pile. An egg that is
    /// incubating stops first: the egg phase only collects idle eggs flagged
    /// `aborting`, so a hatching one would otherwise carry on and hatch.
    pub fn abort_egg(&mut self, id: EggId) -> bool {
        let Some(egg) = self.eggs.get_mut(id) else {
            return false;
        };
        if egg.hatch.aborting {
            return false;
        }
        egg.hatch.aborting = true;
        egg.hatch.is_hatching = false;
        true
    }

    /// Spawns a bibite from a loaded template path at `pos` (viewer spawn
    /// action). Uses the same species/tag assignment as the scenario spawner.
    pub fn spawn_bibite_by_template_path(
        &mut self,
        path: &str,
        growth: GrowthAtSpawn,
        tagging: Tagging,
        tag: &str,
        pos: Vec2,
    ) -> Result<BibiteId> {
        let template = self
            .template_for_path(path)
            .cloned()
            .ok_or_else(|| Error::invalid_data(format!("unknown template '{path}'")))?;
        self.spawn_bibite_from_template(&template, growth, tagging, tag, pos, RandomizeGenes::No)
    }

    /// Replaces the spawner rows (spawner editor action), validating each row. A row that was there keeps its progress and its species (the game's
    /// `SettingsAdded` / `SettingsRemoved` touch only the row that came or went); a new one starts as `BibiteSpawnInfo`'s constructor does: its species is the one
    /// its template names (founded when there is none yet), and a one-time row starts with its minimum already due.
    pub fn set_bibite_spawns(&mut self, rows: Vec<SpawnRow>) -> Result<()> {
        for row in &rows {
            row.validate()?;
        }
        let old_rows = std::mem::take(&mut self.bibite_spawns);
        let mut old_progress = std::mem::take(&mut self.spawn_progress);
        let mut old_species = std::mem::take(&mut self.spawn_row_species);
        old_progress.resize(old_rows.len(), 0.0);
        old_species.resize(old_rows.len(), None);
        let mut taken = vec![false; old_rows.len()];
        let mut progress = Vec::with_capacity(rows.len());
        let mut species = Vec::with_capacity(rows.len());
        for row in &rows {
            let same = old_rows.iter().enumerate().position(|(index, old)| !taken[index] && old == row);
            match same {
                Some(index) => {
                    taken[index] = true;
                    progress.push(old_progress[index]);
                    species.push(old_species[index]);
                }
                None => {
                    progress.push(if row.spawn_type == settings::SpawnType::OneTime { row.minimum as f32 + 0.1 } else { 0.0 });
                    species.push(self.look_up_row_species(row));
                }
            }
        }
        self.bibite_spawns = rows;
        self.spawn_progress = progress;
        self.spawn_row_species = species;
        self.spawner_accumulator = 0.0;
        Ok(())
    }

    /// Appends one spawner row (spawner editor action).
    pub fn add_bibite_spawn(&mut self, row: SpawnRow) -> Result<()> {
        let mut rows = self.bibite_spawns.clone();
        rows.push(row);
        self.set_bibite_spawns(rows)
    }

    /// `BibiteSpawnInfo(settings)`: `FindSpeciesFromTemplate(template, onlyLookUp: true)`, the species the row's template names (founded when no species of that
    /// name is recorded). `None` for a row without a template the world knows.
    fn look_up_row_species(&mut self, row: &SpawnRow) -> Option<crate::ids::SpeciesId> {
        let template = self.template_for_path(&row.path).cloned()?;
        Some(self.species.find_species_from_template(&template.game_species_name(), "", &template.genome, &template.brain, true))
    }

    /// `SaveSystem.LoadWorld` -> `SimulationManager.ResumeSpawners` -> `BibiteSpawner.StartSpawner`: a loaded world runs its spawner too (a save whose scenario has a default
    /// bibite below its minimum spawns more of it). The rows' templates must be set first ([`World::set_templates`]). Returns the number spawned by the first iteration,
    /// which the coroutine runs at once (`yield return null`).
    pub fn resume_spawner_after_load(&mut self) -> usize {
        if self.bibite_spawns.is_empty() {
            return 0;
        }
        let rows = self.bibite_spawns.clone();
        self.spawn_progress = rows.iter().map(|row| if row.spawn_type == settings::SpawnType::OneTime { row.minimum as f32 + 0.1 } else { 0.0 }).collect();
        self.spawn_row_species = rows.iter().map(|row| self.look_up_row_species(row)).collect();
        self.spawner_accumulator = 0.0;
        self.spawner_iteration(1.0)
    }

    /// Runtime settings apply (global/scenario editor). A structural change that
    /// would invalidate the spatial hash (the simulation size) is rejected;
    /// material/zone-global/decay/spawn-limit fields are rebuilt in-place.
    /// Viewers should recreate the world for a rejected setting.
    ///
    /// The ticks per second take effect from the next tick on, as in the game, where
    /// `TimeController.UpdateFixedDeltaTime` and `NEATBrain.UpdateSimTPS` subscribe to `simTPS`: the clock's tick gets the new length
    /// ([`FixedClock::set_tps`]; the time made so far is kept) and the bibites, whose bodies, brains and physics read the rate from the
    /// settings every tick, follow with it ([`World::set_sim_tps`]).
    pub fn apply_settings(&mut self, mut settings: SimSettings) -> Result<()> {
        settings.sync_rates_from_independents();
        settings.validate()?;
        if settings.independents.simulation_size != self.settings.independents.simulation_size {
            return Err(Error::invalid_data(
                "simulationSize change requires a new world (bounds/hash are prebuilt)",
            ));
        }
        let bounds = self.bounds;
        let globals = ZoneGlobals::from_settings(&settings);
        let mut materials = MaterialTable::default();
        for kind in settings::MaterialKind::ALL {
            let runtime_kind = material_kind_from_settings(kind);
            let material = settings.materials.get(kind);
            *materials.get_mut(runtime_kind) = material_properties_from_settings(material);
        }
        self.zones.globals = globals.clone();
        for zone in &mut self.zones.zones {
            zone.refresh(&bounds, &globals);
        }
        // The pellets that exist follow the decay edits (`SettingsUpdater`: a switched Decay registers them all, a new freshness time moves their timers).
        self.pellets.replace_materials(materials);
        self.pellets.recompute_counts();
        // The tick's length (`sync_rates_from_independents` made the rate positive).
        self.clock.set_tps(settings.sim_tps);
        self.decay_tps = settings.decay_tps.max(1);
        self.decay = DecayThrottle::new(self.decay_tps);
        // `GlobalLineageManager.Awake` subscribes `UpdateEffectiveSpeciesSpan` to both span settings (`SpeciesGeneticSpan.OnChange`,
        // `SpeciesSpanScalingFactor.OnChange`): an edit of either recounts the active species, like a species' creation and the Lineage
        // panel's opening do (the count is otherwise held, see `SpeciesRegistry::refresh_span`).
        let span_edited = settings.independents.species_genetic_span
            != self.settings.independents.species_genetic_span
            || settings.independents.species_span_scaling_factor
                != self.settings.independents.species_span_scaling_factor;
        self.bibites.set_settings(settings.clone());
        self.settings = settings;
        if span_edited {
            self.species.refresh_span();
        }
        // Red Death is separate runtime bloom state driven by scenario flags.
        self.tools.red_death.base_damage = self.settings.scenario.red_death_base_damages;
        self.tools.red_death.damage_velocity = self.settings.scenario.red_death_damage_velocity;
        self.tools
            .red_death
            .set_enabled(self.settings.scenario.enable_red_death);
        self.tools
            .red_death
            .set_fill(self.settings.scenario.red_death_fill);
        Ok(())
    }

    /// Changes the simulation's ticks per second while it runs (the in-game settings panel's "Ticks Per Second", `simTPS`). The next tick is
    /// `1 / tps` seconds long; everything per tick (the bodies' energy, ageing and physics, the brains' period `brain_tps / sim_tps`, eggs,
    /// pheromones, decay, the spawner) is made from that length, and the simulated time goes on from where it was. Any rate is taken as the
    /// game takes it (its setter does not clamp to the slider's 5 to 100), but never less than 1.
    pub fn set_sim_tps(&mut self, tps: u32) -> Result<()> {
        let mut settings = self.settings.clone();
        settings.independents.sim_tps = i32::try_from(tps.max(1)).unwrap_or(i32::MAX);
        self.apply_settings(settings)
    }

    /// Evaluates one `SimulationMetric` against this world (challenge score /
    /// conditions): species count/biomass, tag count/biomass (live bibites +
    /// incubating eggs), material count/biomass, time and constant.
    #[must_use]
    pub fn metric_value(&self, metric: SimMetric, argument: &str) -> f32 {
        match metric {
            SimMetric::SpeciesCount | SimMetric::SpeciesBiomass => {
                let record = self
                    .species
                    .records()
                    .iter()
                    .find(|record| record.specific_name == argument);
                record.map_or(0.0, |record| {
                    if metric == SimMetric::SpeciesCount {
                        record.stats.count as f32
                    } else {
                        record.stats.energy
                    }
                })
            }
            SimMetric::TagCount | SimMetric::TagBiomass => {
                let summary = self
                    .tag_summaries()
                    .into_iter()
                    .find(|summary| summary.tag == argument);
                summary.map_or(0.0, |summary| {
                    if metric == SimMetric::TagCount {
                        (summary.bibites + summary.eggs) as f32
                    } else {
                        summary.energy
                    }
                })
            }
            SimMetric::MaterialCount | SimMetric::MaterialBiomass => {
                let counts = self.pellets.counts();
                let (count, energy) =
                    if MaterialKind::from_name(argument) == Some(MaterialKind::Meat) {
                        (counts.meat_count, counts.meat_biomass)
                    } else if MaterialKind::from_name(argument) == Some(MaterialKind::Plant) {
                        (counts.plant_count, counts.plant_biomass)
                    } else {
                        (0, 0.0)
                    };
                if metric == SimMetric::MaterialCount {
                    count as f32
                } else {
                    energy as f32
                }
            }
            SimMetric::Time => self.simulated_time as f32,
            SimMetric::Constant => argument.parse().unwrap_or(0.0),
        }
    }

    /// Evaluates a `SimulationMetric` struct (metric + argument).
    #[must_use]
    pub fn challenge_metric(&self, metric: &SimulationMetric) -> f32 {
        self.metric_value(metric.metric, &metric.argument)
    }

    /// Stars attained for challenge parameters (3/2/1/0), evaluated live.
    #[must_use]
    pub fn challenge_stars(&self, parameters: &ChallengeParameters) -> u8 {
        let evaluate = |group: &crate::settings::ConditionGroup| {
            group.evaluate(&|metric, arg| self.metric_value(metric, arg))
        };
        if evaluate(&parameters.three_star_condition) {
            3
        } else if evaluate(&parameters.two_star_condition) {
            2
        } else if evaluate(&parameters.one_star_condition) {
            1
        } else {
            0
        }
    }

    /// Creates a fresh scenario world from settings + scenario data + decoded
    /// templates: seeds initial zone pellets and starts the virgin spawner.
    /// This is the core half of the game's "New Game" action; io supplies the
    /// scenario bundle and `load_scenario_world_contents(path, Some(settings))`
    /// for dropdown overrides.
    pub fn new_fresh_scenario(
        settings: SimSettings,
        scenario: ScenarioData,
        seed: u64,
        templates: Vec<TemplateSource>,
    ) -> Result<Self> {
        let mut world = Self::new(settings, scenario, seed)?;
        world.set_templates(templates);
        let _ = world.seed_scenario_world();
        let _ = world.start_fresh_scenario_spawner();
        Ok(world)
    }

    /// Snapshot of scenario-editable settings/zones/spawns/changers for io or
    /// the viewer's scenario editor. Persist by copying these into a parsed
    /// `bibites_io::ScenarioSettingsFile` (which preserves unknown keys and
    /// groups) and calling `bibites_io::storage::update_scenario_settings`.
    #[must_use]
    pub fn scenario_settings_snapshot(&self) -> WorldScenarioSettings {
        WorldScenarioSettings {
            settings: self.settings.clone(),
            zones: self
                .zones
                .zones
                .iter()
                .map(|zone| zone.config.0.clone())
                .collect(),
            spawn_rows: self.bibite_spawns.clone(),
            settings_changers: self.settings_changers.clone(),
        }
    }

    /// Applies one scenario-independent setting by its game JSON key.
    pub fn set_independent_setting(&mut self, key: &str, value: SettingValue) -> Result<()> {
        let mut settings = self.settings.clone();
        if settings.independents.set_raw(key, value).is_none() {
            return Err(Error::invalid_data(format!(
                "unknown independent setting '{key}'"
            )));
        }
        self.apply_settings(settings)
    }

    /// Applies one scenario setting by its game JSON key.
    pub fn set_scenario_setting(&mut self, key: &str, value: SettingValue) -> Result<()> {
        let mut settings = self.settings.clone();
        if settings.scenario.set_raw(key, value).is_none() {
            return Err(Error::invalid_data(format!(
                "unknown scenario setting '{key}'"
            )));
        }
        self.apply_settings(settings)
    }

    /// Adds a radiation tower and returns its id.
    pub fn add_rad_tower(&mut self, tower: RadTower) -> ToolId {
        self.tools.add_rad_tower(tower)
    }

    /// Adds a pheromone tower and returns its id.
    pub fn add_phero_tower(&mut self, tower: PheroTower) -> ToolId {
        self.tools.add_phero_tower(tower)
    }

    /// Adds a colour killer and returns its id.
    pub fn add_color_killer(&mut self, killer: ColorKiller) -> ToolId {
        self.tools.add_color_killer(killer)
    }

    /// The Red Death bloom state.
    #[inline]
    pub fn red_death(&self) -> &RedDeathBloom {
        &self.tools.red_death
    }

    /// Mutable Red Death bloom state (settings changes / event control).
    #[inline]
    pub fn red_death_mut(&mut self) -> &mut RedDeathBloom {
        &mut self.tools.red_death
    }

    /// Phase 9 data collection: rad doses for the supplied bibite positions.
    /// The caller applies them through the genome mutation-factor hook.
    pub fn collect_rad_doses(
        &mut self,
        dt: f32,
        targets: impl IntoIterator<Item = BibiteTarget>,
    ) -> &[RadDose] {
        self.tools.collect_rad_doses(dt, &self.zones, targets)
    }

    /// Phase 11 data collection: one kill per due colour killer. The caller
    /// performs the kills through the body store.
    pub fn collect_color_kills(
        &mut self,
        dt: f32,
        candidates: impl IntoIterator<Item = ColorCandidate>,
    ) -> &[BibiteId] {
        let rng = &mut self.rng;
        let zones = &self.zones;
        self.tools.collect_color_kills(dt, zones, candidates, rng)
    }

    /// Phase 10 timer: `true` when a Red Death damage tick is due.
    pub fn step_red_death_timer(&mut self, dt: f32) -> bool {
        self.tools.step_red_death(dt)
    }

    /// Seeds a fresh scenario world when `scenario.initialSeeding > 0`, honouring
    /// the Red Death safe radius when the event is enabled.
    ///
    /// Returns `None` when seeding is disabled or there are no zones. Call this
    /// explicitly for a **new** scenario; a loaded save must not re-seed.
    pub fn seed_scenario_world(&mut self) -> Option<SeedingReport> {
        if self.settings.scenario.initial_seeding <= 0.0 || self.zones.zones.is_empty() {
            return None;
        }
        let safe_radius = self
            .tools
            .red_death
            .enabled
            .then_some(self.tools.red_death.safe_radius);
        Some(self.initial_seeding_with_red_death(safe_radius))
    }

    /// Registers a bibite in the spatial hash from its current body view.
    ///
    /// Returns `false` if the id is not live. Loaders call this after
    /// `BibiteStore::spawn_loaded`; the spawner calls it after spawning.
    pub fn register_bibite(&mut self, id: BibiteId) -> bool {
        let Some(view) = self.bibites.view(id) else {
            return false;
        };
        self.hash.insert(
            id.handle(),
            SpatialClass::Bibite,
            view.pos,
            view.side_radius(),
        );
        if self.bibites.get(id).is_some_and(|r| r.meta.is_source) {
            self.has_sources = true;
        }
        true
    }

    /// Body-save load path: spawns a bibite from its serialized state and
    /// registers it in the spatial hash.
    ///
    /// This is the call `bibites-io` must use per saved bibite after
    /// [`World::new`] (spec §8 load sequence).
    pub fn spawn_loaded_bibite(&mut self, save: crate::body::BibiteSave) -> Result<BibiteId> {
        let id = self.bibites.spawn_loaded(save)?;
        self.register_bibite(id);
        Ok(id)
    }

    /// Moves every bibite's spatial-hash entry to where the bibite is now (its pose changes every physics step).
    pub fn refresh_bibite_hash(&mut self) {
        let hash = &mut self.hash;
        for (id, _) in self.bibites.iter() {
            if let Some(view) = self.bibites.view(id) {
                hash.update(id.handle(), SpatialClass::Bibite, view.pos, view.side_radius());
            }
        }
    }

    /// Removes spatial-hash bibite entries whose bibites are no longer live.
    ///
    /// The store logs the bibites it removes, and a bibite's entry can only be stale because the bibite was removed, so the
    /// entries to drop are found from the log (in the order of their handle slots, the order a pass over every entry reaches
    /// them in) instead of asking every entry whether its record is still there, which costs a cache miss on each 2 KB
    /// record. When the log is incomplete (a restored checkpoint) the pass over every entry runs. In tests every sync is
    /// checked against that pass on a copy of the hash (the whole structure, down to the order inside every cell).
    pub fn sync_bibite_hash(&mut self) {
        let mut removed = std::mem::take(&mut self.bibite_removed_scratch);
        let unknown = self.bibites.drain_removed(&mut removed);
        #[cfg(test)]
        let expected = (self.hash.len() <= 30_000).then(|| {
            let mut hash = self.hash.clone();
            let bibites = &self.bibites;
            hash.retain(SpatialClass::Bibite, |handle| {
                bibites.view(BibiteId::from_handle(handle)).is_some()
            });
            hash
        });
        if unknown {
            let bibites = &self.bibites;
            self.hash.retain(SpatialClass::Bibite, |handle| {
                bibites.is_live(BibiteId::from_handle(handle))
            });
        } else {
            removed.sort_unstable_by_key(|id| id.handle().slot());
            removed.dedup_by_key(|id| id.handle().slot());
            for id in &removed {
                if let Some(handle) = self
                    .hash
                    .handle_at_slot(SpatialClass::Bibite, id.handle().slot() as usize)
                    && !self.bibites.is_live(BibiteId::from_handle(handle))
                {
                    self.hash.remove(handle, SpatialClass::Bibite);
                }
            }
        }
        removed.clear();
        self.bibite_removed_scratch = removed;
        #[cfg(test)]
        if let Some(expected) = expected {
            assert!(
                self.hash.same_as(&expected),
                "the bibite hash sync from the removal log differs from the pass over every entry"
            );
        }
    }

    /// Phase 8: pellet decay, throttled by `decayTPS` (spec §2.4).
    ///
    /// `dt` is the fixed tick length; the decay processor receives
    /// `dt · decay_tps`. Returns the number of pellets destroyed.
    pub fn step_decay(&mut self, dt: f32) -> crate::pellet::DecayReport {
        if !self.decay.tick() {
            return crate::pellet::DecayReport::default();
        }
        let period = self.decay.period() as f32;
        let executor = crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref());
        let report = self.pellets.process_decay_with(
            dt * period,
            executor,
            self.active_gates.scans,
        );
        self.accounting.decayed_plant_energy +=
            report.plant_amount * f64::from(self.settings.materials.plant.energy_density);
        self.accounting.decayed_meat_energy +=
            report.meat_amount * f64::from(self.settings.materials.meat.energy_density);
        self.sync_pellet_hash();
        report
    }

    /// Phase 7: pheromone dissipation (spec §5.3) and hash reconciliation.
    pub fn step_pheromones(&mut self, dt: f32) {
        self.pheromones.dissipate(dt);
        let hash = &mut self.hash;
        self.pheromones.drain_removed(|id| {
            hash.remove(id.handle(), SpatialClass::Pheromone);
        });
    }

    /// Removes spatial-hash pheromone entries whose spots are no longer live and
    /// registers spots emitted by bodies/tools since the last reconciliation.
    pub fn sync_pheromone_hash(&mut self) {
        let pheromones = &self.pheromones;
        self.hash.retain(SpatialClass::Pheromone, |handle| {
            pheromones.contains(crate::ids::PheromoneId::from_handle(handle))
        });
        // Body emission writes to the PheromoneStore only; the world owns the
        // hash, so newly emitted spots are inserted here (spots never move; tower
        // replacement is handled incrementally by `step_tools`).
        for id in pheromones.ids() {
            if self.hash.contains(id.handle(), SpatialClass::Pheromone) {
                continue;
            }
            if let Some(view) = pheromones.get(id) {
                let radius = pheromone_spot_radius(pheromones.owner_tool(id).flatten().is_some());
                self.hash
                    .insert(id.handle(), SpatialClass::Pheromone, view.pos, radius);
            }
        }
    }

    /// Spawns a pellet through the store and registers it in the hash.
    ///
    /// Does **not** adjust a zone's lagging `pellet_biomass` (matching the
    /// game's load path; the next recount reconciles it), but it does append to
    /// the owning zone's pellet list so recycling can find it.
    pub fn spawn_pellet(&mut self, spec: PelletSpawn) -> Option<PelletId> {
        let owner = spec.owner_zone;
        let id = self.pellets.try_spawn(spec)?;
        if let (Some(pos), Some(radius)) = (self.pellets.pos(id), self.pellets.radius(id)) {
            self.hash
                .insert(id.handle(), SpatialClass::Pellet, pos, radius);
        }
        if let Some(zone) = owner.and_then(|zone| self.zones.get_mut(zone)) {
            zone.owned_pellets.push(id);
        }
        Some(id)
    }

    /// Removes a pellet from the store, hash and owning zone's list.
    pub fn despawn_pellet(&mut self, id: PelletId) {
        let owner = self.pellets.owner_zone(id).flatten();
        let energy = f64::from(self.pellets.energy(id).unwrap_or(0.0));
        match self.pellets.material(id) {
            Some(crate::pellet::MaterialKind::Meat) => {
                self.accounting.removed_meat_energy += energy;
            }
            _ => self.accounting.removed_plant_energy += energy,
        }
        self.hash.remove(id.handle(), SpatialClass::Pellet);
        self.pellets.remove(id);
        if let Some(zone) = owner {
            self.zones.forget_pellet(zone, id);
        }
    }

    /// Changes a pellet's amount (the body-feeding choke point) and keeps the
    /// hash radius in sync, removing the entry when the pellet is consumed.
    pub fn change_pellet_amount(&mut self, id: PelletId, delta: f32) -> f32 {
        let changed = self.pellets.change_amount(id, delta);
        if self.pellets.contains(id) {
            if let (Some(pos), Some(radius)) = (self.pellets.pos(id), self.pellets.radius(id)) {
                self.hash
                    .update(id.handle(), SpatialClass::Pellet, pos, radius);
            }
        } else {
            self.hash.remove(id.handle(), SpatialClass::Pellet);
        }
        changed
    }

    /// Movement of a pellet (physics); keeps the hash position in sync.
    pub fn move_pellet(&mut self, id: PelletId, pos: Vec2) -> bool {
        if !self.pellets.set_pos(id, pos) {
            return false;
        }
        if let Some(radius) = self.pellets.radius(id) {
            self.hash
                .update(id.handle(), SpatialClass::Pellet, pos, radius);
        }
        true
    }

    /// First zone containing `pos` (void avoidance / effect lookup hook).
    #[inline]
    pub fn zone_at(&self, pos: Vec2) -> Option<ZoneId> {
        self.zones.zone_at(pos)
    }

    /// Sum of zone free biomass (`ZoneManager.freeBiomass`).
    #[inline]
    pub fn free_biomass(&self) -> f32 {
        self.zones.free_biomass()
    }

    /// Sum of zone max biomass.
    #[inline]
    pub fn total_max_biomass(&self) -> f32 {
        self.zones.total_max_biomass()
    }
}

/// Deterministic `Random.insideUnitCircle`: a point uniformly distributed in
/// the unit disc (radius `sqrt(u)`, uniform angle).
#[inline]
pub fn random_inside_unit_circle(rng: &mut crate::rng::Rng) -> Vec2 {
    let radius = rng.f32().sqrt();
    let angle = rng.f32() * TAU;
    Vec2::from_angle(angle) * radius
}

/// The full pass of the pellet hash sync: drops the entries of pellets that are gone and updates every live one in
/// dense order.
fn full_pellet_hash_sync(hash: &mut SpatialHash, pellets: &PelletStore) {
    hash.retain(SpatialClass::Pellet, |handle| {
        pellets.get(PelletId::from_handle(handle)).is_some()
    });
    for id in pellets.ids() {
        if let Some(view) = pellets.get(id) {
            hash.update(id.handle(), SpatialClass::Pellet, view.pos, view.radius);
        }
    }
}

/// A growable bitset addressed by slot index, shared by the entity stores.
///
/// Not part of the public API (`pub(crate)`); stores use it for their `alive`,
/// `held` and `decaying` flags.
#[derive(Clone, Debug, Default)]
pub(crate) struct BitSet {
    words: Vec<u64>,
    /// How many bits are set (kept by [`Self::set`] and [`Self::clear`]).
    ones: usize,
}

impl BitSet {
    /// Creates an empty bitset.
    pub(crate) const fn new() -> Self {
        Self {
            words: Vec::new(),
            ones: 0,
        }
    }

    /// How many bits are set.
    #[inline]
    pub(crate) const fn ones(&self) -> usize {
        self.ones
    }

    /// Ensures at least `bits` bits exist (extra bits are zeroed on growth).
    pub(crate) fn ensure(&mut self, bits: usize) {
        let words = bits.div_ceil(64);
        if self.words.len() < words {
            self.words.resize(words, 0);
        }
    }

    /// Returns the bit at `bit` (out-of-range reads are `false`).
    #[inline]
    pub(crate) fn get(&self, bit: usize) -> bool {
        let word = bit / 64;
        match self.words.get(word) {
            Some(value) => (value >> (bit % 64)) & 1 != 0,
            None => false,
        }
    }

    /// Sets or clears the bit at `bit`, growing the bitset when needed.
    #[inline]
    pub(crate) fn set(&mut self, bit: usize, value: bool) {
        self.ensure(bit + 1);
        let word = &mut self.words[bit / 64];
        let mask = 1u64 << (bit % 64);
        let had = *word & mask != 0;
        if value {
            *word |= mask;
        } else {
            *word &= !mask;
        }
        self.ones = self.ones + usize::from(value) - usize::from(had);
    }

    /// Clears all bits while retaining capacity.
    ///
    /// Used by the store reset paths once world orchestration lands; exercised
    /// by the unit tests meanwhile.
    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.words.fill(0);
        self.ones = 0;
    }
}

// ---------------------------------------------------------------------------
// Checkpoint P2: exact spatial-hash layout (task-103)
// ---------------------------------------------------------------------------

impl SpatialHash {
    /// Captures the exact grid/entry layout so query iteration order survives a
    /// resume (float summation order is behaviour).
    fn capture_checkpoint_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.nx.to_le_bytes());
        out.extend_from_slice(&self.ny.to_le_bytes());
        out.extend_from_slice(&self.min.x.to_bits().to_le_bytes());
        out.extend_from_slice(&self.min.y.to_bits().to_le_bytes());
        out.extend_from_slice(&self.cell_size.to_bits().to_le_bytes());
        out.extend_from_slice(&self.inv_cell.to_bits().to_le_bytes());
        out.extend_from_slice(&self.max_radius.to_bits().to_le_bytes());
        out.extend_from_slice(&(self.len as u32).to_le_bytes());
        out.extend_from_slice(&(self.cells.len() as u32).to_le_bytes());
        for cell in &self.cells {
            out.extend_from_slice(&(cell.len() as u32).to_le_bytes());
            for entry in cell {
                out.extend_from_slice(&entry.to_le_bytes());
            }
        }
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            out.extend_from_slice(&entry.handle.slot().to_le_bytes());
            out.extend_from_slice(&entry.handle.generation().to_le_bytes());
            out.push(match entry.class {
                SpatialClass::Bibite => 0,
                SpatialClass::Pellet => 1,
                SpatialClass::Egg => 2,
                SpatialClass::Pheromone => 3,
            });
            out.extend_from_slice(&entry.pos.x.to_bits().to_le_bytes());
            out.extend_from_slice(&entry.pos.y.to_bits().to_le_bytes());
            out.extend_from_slice(&entry.radius.to_bits().to_le_bytes());
            out.extend_from_slice(&entry.cell.to_le_bytes());
            out.extend_from_slice(&entry.cell_slot.to_le_bytes());
        }
        out.extend_from_slice(&(self.free_entries.len() as u32).to_le_bytes());
        for entry in &self.free_entries {
            out.extend_from_slice(&entry.to_le_bytes());
        }
        for class in SpatialClass::ORDER {
            let map = &self.slot_maps[class.index()];
            out.extend_from_slice(&(map.len() as u32).to_le_bytes());
            for entry in map {
                out.extend_from_slice(&entry.to_le_bytes());
            }
        }
        out
    }

    fn restore_checkpoint_bytes(&mut self, bytes: &[u8]) -> crate::error::Result<()> {
        let mut reader = CheckpointReader::new(bytes);
        let nx = reader.u32()?;
        let ny = reader.u32()?;
        let min = reader.vec2()?;
        let cell_size = reader.f32()?;
        let inv_cell = reader.f32()?;
        if nx != self.nx
            || ny != self.ny
            || min.x.to_bits() != self.min.x.to_bits()
            || min.y.to_bits() != self.min.y.to_bits()
            || cell_size.to_bits() != self.cell_size.to_bits()
            || inv_cell.to_bits() != self.inv_cell.to_bits()
        {
            return Err(crate::error::Error::invalid_data(
                "checkpoint spatial-hash geometry does not match world",
            ));
        }
        self.max_radius = reader.f32()?;
        self.len = reader.u32()? as usize;
        let cell_count = reader.u32()? as usize;
        if cell_count != self.cells.len() {
            return Err(crate::error::Error::invalid_data(
                "checkpoint spatial-hash cell count mismatch",
            ));
        }
        self.cells = Vec::with_capacity(cell_count);
        for _ in 0..cell_count {
            let n = reader.u32()? as usize;
            let mut cell = Vec::with_capacity(n);
            for _ in 0..n {
                cell.push(reader.u32()?);
            }
            self.cells.push(cell);
        }
        let entry_count = reader.u32()? as usize;
        self.entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let slot = reader.u32()?;
            let generation = reader.u32()?;
            let class = match reader.u8()? {
                0 => SpatialClass::Bibite,
                1 => SpatialClass::Pellet,
                2 => SpatialClass::Egg,
                3 => SpatialClass::Pheromone,
                _ => {
                    return Err(crate::error::Error::invalid_data(
                        "invalid spatial-hash class tag",
                    ));
                }
            };
            self.entries.push(Entry {
                handle: Handle::new(slot, generation),
                class,
                pos: reader.vec2()?,
                radius: reader.f32()?,
                cell: reader.u32()?,
                cell_slot: reader.u32()?,
            });
        }
        let free_count = reader.u32()? as usize;
        self.free_entries = Vec::with_capacity(free_count);
        for _ in 0..free_count {
            self.free_entries.push(reader.u32()?);
        }
        for class in SpatialClass::ORDER {
            let count = reader.u32()? as usize;
            let map = &mut self.slot_maps[class as usize];
            map.clear();
            map.reserve(count);
            for _ in 0..count {
                map.push(reader.u32()?);
            }
        }
        Ok(())
    }
}

/// Checkpoint extension tag: per-bibite brain runtime (task-103 P2).
pub const CHECKPOINT_BRAIN_TAG: u16 = 0x010C;

fn capture_brain_checkpoint(world: &World) -> Vec<u8> {
    let mut out = Vec::new();
    let mut entries: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for (id, record) in world.bibites.iter() {
        entries.push((
            id.handle().slot(),
            id.handle().generation(),
            record.brain.capture_checkpoint().encode(),
        ));
    }
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (slot, generation, bytes) in entries {
        out.extend_from_slice(&slot.to_le_bytes());
        out.extend_from_slice(&generation.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    out
}

fn restore_brain_checkpoint(world: &mut World, bytes: &[u8]) -> crate::error::Result<()> {
    let mut reader = CheckpointReader::new(bytes);
    let count = reader.u32()? as usize;
    for _ in 0..count {
        let slot = reader.u32()?;
        let generation = reader.u32()?;
        let len = reader.u32()? as usize;
        let bytes = reader.take(len)?;
        let id = BibiteId::from_handle(Handle::new(slot, generation));
        let record = world.bibites.get_mut(id).ok_or_else(|| {
            crate::error::Error::invalid_data(format!(
                "brain checkpoint references missing bibite slot {slot} gen {generation}"
            ))
        })?;
        let checkpoint = crate::brain::BrainCheckpoint::decode(bytes)?;
        record.brain.restore_checkpoint(checkpoint)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::EggSpawn;
    use crate::rng::Rng;

    const CLASSES: [SpatialClass; 4] = [
        SpatialClass::Bibite,
        SpatialClass::Pellet,
        SpatialClass::Pheromone,
        SpatialClass::Egg,
    ];

    fn handle(slot: u32) -> Handle {
        Handle::new(slot, 0)
    }

    fn test_hash() -> SpatialHash {
        SpatialHash::new(Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0), 10.0)
    }

    #[test]
    fn insert_contains_remove_update() {
        let mut hash = test_hash();
        let h = handle(0);
        assert!(hash.insert(h, SpatialClass::Pellet, Vec2::new(1.0, 2.0), 3.0));
        assert_eq!(hash.len(), 1);
        assert!(hash.contains(h, SpatialClass::Pellet));
        assert!(!hash.contains(h, SpatialClass::Bibite));
        assert_eq!(
            hash.pos_of(h, SpatialClass::Pellet),
            Some(Vec2::new(1.0, 2.0))
        );
        assert_eq!(hash.radius_of(h, SpatialClass::Pellet), Some(3.0));
        assert!(!hash.insert(h, SpatialClass::Pellet, Vec2::ZERO, 0.0));

        assert!(hash.update(h, SpatialClass::Pellet, Vec2::new(50.0, 50.0), 1.0));
        assert_eq!(
            hash.pos_of(h, SpatialClass::Pellet),
            Some(Vec2::new(50.0, 50.0))
        );
        assert!(hash.remove(h, SpatialClass::Pellet));
        assert!(!hash.remove(h, SpatialClass::Pellet));
        assert!(hash.is_empty());
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let mut hash = test_hash();
        let h = handle(0);
        assert!(!hash.insert(h, SpatialClass::Pellet, Vec2::new(f32::NAN, 0.0), 1.0));
        assert!(!hash.insert(h, SpatialClass::Pellet, Vec2::ZERO, -1.0));
        assert!(!hash.update(h, SpatialClass::Pellet, Vec2::ZERO, f32::NAN));
        assert!(!hash.contains(h, SpatialClass::Pellet));
    }

    #[test]
    fn same_slot_different_classes_are_distinct() {
        let mut hash = test_hash();
        let slot = handle(5);
        assert!(hash.insert(slot, SpatialClass::Bibite, Vec2::ZERO, 1.0));
        assert!(hash.insert(slot, SpatialClass::Pellet, Vec2::new(5.0, 5.0), 1.0));
        assert_eq!(hash.len(), 2);
        let mut scratch = QueryScratch::new();
        let result = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::ALL, &mut scratch);
        assert_eq!(result.len(), 2);
        let classes: Vec<_> = result.hits.iter().map(|hit| hit.class).collect();
        assert_eq!(classes, vec![SpatialClass::Bibite, SpatialClass::Pellet]);
        assert!(hash.remove(slot, SpatialClass::Bibite));
        assert!(hash.contains(slot, SpatialClass::Pellet));
    }

    #[test]
    fn class_masks_filter_results() {
        let mut hash = test_hash();
        hash.insert(handle(0), SpatialClass::Bibite, Vec2::ZERO, 1.0);
        hash.insert(handle(1), SpatialClass::Pellet, Vec2::new(1.0, 0.0), 1.0);
        hash.insert(handle(2), SpatialClass::Pheromone, Vec2::new(2.0, 0.0), 1.0);
        hash.insert(handle(3), SpatialClass::Egg, Vec2::new(3.0, 0.0), 1.0);
        let mut scratch = QueryScratch::new();
        let mask = ClassMask::BIBITE | ClassMask::PELLET;
        let result = hash.query_circle(Vec2::ZERO, 50.0, mask, &mut scratch);
        assert_eq!(result.len(), 2);
        assert!(
            result
                .hits
                .iter()
                .all(|hit| hit.class == SpatialClass::Bibite || hit.class == SpatialClass::Pellet)
        );
        assert_eq!(ClassMask::ALL.bits(), 0b1111);
        assert!(!ClassMask::NONE.contains(SpatialClass::Egg));
        assert!(ClassMask::from_class(SpatialClass::Egg).contains(SpatialClass::Egg));
        assert!(ClassMask::NONE.is_empty());
    }

    fn brute_force_circle(
        entries: &[(Handle, SpatialClass, Vec2, f32)],
        center: Vec2,
        radius: f32,
        mask: ClassMask,
    ) -> Vec<(SpatialClass, Handle)> {
        let mut hits: Vec<_> = entries
            .iter()
            .filter(|(_, class, pos, entry_radius)| {
                mask.contains(*class)
                    && pos.distance_sq(center)
                        <= (radius + *entry_radius) * (radius + *entry_radius)
            })
            .map(|(handle, class, _, _)| (*class, *handle))
            .collect();
        hits.sort_unstable_by_key(|(class, handle)| {
            (class.index(), handle.slot(), handle.generation())
        });
        hits
    }

    fn brute_force_aabb(
        entries: &[(Handle, SpatialClass, Vec2, f32)],
        min: Vec2,
        max: Vec2,
        mask: ClassMask,
    ) -> Vec<(SpatialClass, Handle)> {
        let mut hits: Vec<_> = entries
            .iter()
            .filter(|(_, class, pos, entry_radius)| {
                mask.contains(*class)
                    && pos.x + entry_radius >= min.x
                    && pos.x - entry_radius <= max.x
                    && pos.y + entry_radius >= min.y
                    && pos.y - entry_radius <= max.y
            })
            .map(|(handle, class, _, _)| (*class, *handle))
            .collect();
        hits.sort_unstable_by_key(|(class, handle)| {
            (class.index(), handle.slot(), handle.generation())
        });
        hits
    }

    fn brute_force_rotated_box(
        entries: &[(Handle, SpatialClass, Vec2, f32)],
        center: Vec2,
        half: Vec2,
        rotation: f32,
        mask: ClassMask,
    ) -> Vec<(SpatialClass, Handle)> {
        let (sin, cos) = rotation.sin_cos();
        let mut hits: Vec<_> = entries
            .iter()
            .filter(|(_, class, pos, entry_radius)| {
                if !mask.contains(*class) {
                    return false;
                }
                let local = *pos - center;
                let local_x = local.x * cos + local.y * sin;
                let local_y = -local.x * sin + local.y * cos;
                local_x.abs() <= half.x + entry_radius && local_y.abs() <= half.y + entry_radius
            })
            .map(|(handle, class, _, _)| (*class, *handle))
            .collect();
        hits.sort_unstable_by_key(|(class, handle)| {
            (class.index(), handle.slot(), handle.generation())
        });
        hits
    }

    #[test]
    fn queries_match_brute_force() {
        let mut hash = test_hash();
        let mut rng = Rng::from_seed(0xBEEF);
        let mut entries: Vec<(Handle, SpatialClass, Vec2, f32)> = Vec::new();
        for i in 0..300u32 {
            let class = CLASSES[(i as usize) % CLASSES.len()];
            let pos = Vec2::new(rng.f32_range(-100.0, 100.0), rng.f32_range(-100.0, 100.0));
            let radius = rng.f32_range(0.0, 8.0);
            let handle = handle(i);
            assert!(hash.insert(handle, class, pos, radius));
            entries.push((handle, class, pos, radius));
        }
        let mut scratch = QueryScratch::new();
        for _ in 0..200 {
            let center = Vec2::new(rng.f32_range(-100.0, 100.0), rng.f32_range(-100.0, 100.0));
            let radius = rng.f32_range(0.0, 30.0);
            let mask = if rng.f32() < 0.5 {
                ClassMask::ALL
            } else {
                ClassMask::BIBITE | ClassMask::PHEROMONE
            };
            let result = hash.query_circle(center, radius, mask, &mut scratch);
            let got: Vec<_> = result
                .hits
                .iter()
                .map(|hit| (hit.class, hit.handle))
                .collect();
            let expected = brute_force_circle(&entries, center, radius, mask);
            assert_eq!(got, expected);

            let min = center - Vec2::splat(10.0);
            let max = center + Vec2::splat(10.0);
            let result = hash.query_aabb(min, max, mask, &mut scratch);
            let got: Vec<_> = result
                .hits
                .iter()
                .map(|hit| (hit.class, hit.handle))
                .collect();
            let expected = brute_force_aabb(&entries, min, max, mask);
            assert_eq!(got, expected);

            let rotation = rng.f32_range(-3.0, 3.0);
            let half = Vec2::new(rng.f32_range(0.0, 25.0), rng.f32_range(0.0, 25.0));
            let result = hash.query_rotated_box(center, half, rotation, mask, &mut scratch);
            let got: Vec<_> = result
                .hits
                .iter()
                .map(|hit| (hit.class, hit.handle))
                .collect();
            let expected = brute_force_rotated_box(&entries, center, half, rotation, mask);
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn large_entities_outside_the_query_box_are_found() {
        let mut hash = test_hash();
        hash.insert(handle(0), SpatialClass::Pellet, Vec2::new(20.0, 0.0), 30.0);
        let mut scratch = QueryScratch::new();
        let result = hash.query_circle(Vec2::ZERO, 5.0, ClassMask::PELLET, &mut scratch);
        assert_eq!(result.len(), 1);
        let result = hash.query_aabb(
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, 1.0),
            ClassMask::PELLET,
            &mut scratch,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn query_results_are_sorted_and_history_independent() {
        let mut a = test_hash();
        let mut b = test_hash();
        let mut rng = Rng::from_seed(123);
        let mut entries = Vec::new();
        for i in 0..200u32 {
            let class = CLASSES[(i as usize) % CLASSES.len()];
            let pos = Vec2::new(rng.f32_range(-100.0, 100.0), rng.f32_range(-100.0, 100.0));
            let radius = rng.f32_range(0.0, 5.0);
            entries.push((handle(i), class, pos, radius));
        }
        for (handle, class, pos, radius) in &entries {
            a.insert(*handle, *class, *pos, *radius);
        }
        for (handle, class, pos, radius) in entries.iter().rev() {
            b.insert(*handle, *class, *pos, *radius);
        }
        b.remove(handle(10), SpatialClass::Pheromone);
        b.insert(
            handle(10),
            SpatialClass::Pheromone,
            entries[10].2,
            entries[10].3,
        );
        let mut scratch_a = QueryScratch::new();
        let mut scratch_b = QueryScratch::new();
        let result_a = a.query_circle(Vec2::ZERO, 100.0, ClassMask::ALL, &mut scratch_a);
        let got_a: Vec<_> = result_a.hits.to_vec();
        let result_b = b.query_circle(Vec2::ZERO, 100.0, ClassMask::ALL, &mut scratch_b);
        let got_b: Vec<_> = result_b.hits.to_vec();
        assert_eq!(got_a, got_b);
        let mut sorted = got_a.clone();
        sorted.sort_unstable_by_key(QueryHit::sort_key);
        assert_eq!(got_a, sorted);
    }

    #[test]
    fn cap_truncates_deterministically() {
        let mut hash = test_hash();
        for i in 0..200u32 {
            hash.insert(
                handle(i),
                SpatialClass::Pellet,
                Vec2::new((i % 10) as f32, (i / 10) as f32),
                0.1,
            );
        }
        let mut scratch = QueryScratch::with_capacity(DEFAULT_QUERY_CAP);
        let first = {
            let result = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::PELLET, &mut scratch);
            assert_eq!(result.len(), DEFAULT_QUERY_CAP);
            assert!(result.truncated);
            result.hits.to_vec()
        };
        let second = {
            let result = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::PELLET, &mut scratch);
            result.hits.to_vec()
        };
        assert_eq!(first, second);
        let mut big = QueryScratch::with_capacity(512);
        let result = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::PELLET, &mut big);
        assert_eq!(result.len(), 200);
        assert!(!result.truncated);
        let mut zero = QueryScratch::with_capacity(0);
        let result = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::PELLET, &mut zero);
        assert_eq!(result.len(), 0);
        assert!(result.truncated);
    }

    #[test]
    fn query_path_reuses_scratch_capacity() {
        let mut hash = test_hash();
        let mut rng = Rng::from_seed(99);
        for i in 0..500u32 {
            hash.insert(
                handle(i),
                CLASSES[(i as usize) % CLASSES.len()],
                Vec2::new(rng.f32_range(-100.0, 100.0), rng.f32_range(-100.0, 100.0)),
                rng.f32_range(0.0, 4.0),
            );
        }
        let mut scratch = QueryScratch::new();
        let _ = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::ALL, &mut scratch);
        let capacity = scratch.cap();
        let pointer = scratch.as_slice().as_ptr();
        for _ in 0..1000 {
            let _ = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::ALL, &mut scratch);
            let _ = hash.query_aabb(
                Vec2::new(-50.0, -50.0),
                Vec2::new(50.0, 50.0),
                ClassMask::ALL,
                &mut scratch,
            );
            let _ = hash.query_rotated_box(
                Vec2::ZERO,
                Vec2::new(20.0, 10.0),
                0.3,
                ClassMask::ALL,
                &mut scratch,
            );
        }
        assert_eq!(scratch.cap(), capacity);
        assert_eq!(scratch.as_slice().as_ptr(), pointer);
    }

    #[test]
    fn update_moves_between_cells() {
        let mut hash = test_hash();
        let h = handle(0);
        hash.insert(h, SpatialClass::Egg, Vec2::new(-90.0, -90.0), 1.0);
        let mut scratch = QueryScratch::new();
        assert_eq!(
            hash.query_circle(Vec2::new(-90.0, -90.0), 1.0, ClassMask::EGG, &mut scratch)
                .len(),
            1
        );
        hash.update(h, SpatialClass::Egg, Vec2::new(90.0, 90.0), 1.0);
        assert_eq!(
            hash.query_circle(Vec2::new(-90.0, -90.0), 1.0, ClassMask::EGG, &mut scratch)
                .len(),
            0
        );
        assert_eq!(
            hash.query_circle(Vec2::new(90.0, 90.0), 1.0, ClassMask::EGG, &mut scratch)
                .len(),
            1
        );
    }

    #[test]
    fn clear_retains_capacity_and_empties() {
        let mut hash = test_hash();
        let capacity_before = hash.cells.len();
        for i in 0..100u32 {
            hash.insert(handle(i), SpatialClass::Pellet, Vec2::ZERO, 1.0);
        }
        hash.clear();
        assert!(hash.is_empty());
        assert_eq!(hash.max_entry_radius(), 0.0);
        assert_eq!(hash.cells.len(), capacity_before);
        assert!(!hash.contains(handle(0), SpatialClass::Pellet));
    }

    #[test]
    fn out_of_bounds_entities_are_clamped_but_exact() {
        let mut hash = test_hash();
        hash.insert(handle(0), SpatialClass::Pellet, Vec2::new(500.0, 0.0), 1.0);
        let mut scratch = QueryScratch::new();
        assert_eq!(
            hash.query_circle(Vec2::new(500.0, 0.0), 2.0, ClassMask::PELLET, &mut scratch)
                .len(),
            1
        );
        assert_eq!(
            hash.query_circle(Vec2::ZERO, 2.0, ClassMask::PELLET, &mut scratch)
                .len(),
            0
        );
    }

    #[test]
    fn bitset_basics() {
        let mut bits = BitSet::new();
        assert!(!bits.get(0));
        bits.set(0, true);
        bits.set(65, true);
        assert!(bits.get(0));
        assert!(!bits.get(64));
        assert!(bits.get(65));
        bits.set(0, false);
        assert!(!bits.get(0));
        bits.clear();
        assert!(!bits.get(65));
        bits.ensure(200);
        assert!(!bits.get(199));
    }

    #[test]
    fn random_inside_unit_circle_is_in_disc() {
        let mut rng = Rng::from_seed(5);
        for _ in 0..10_000 {
            let p = random_inside_unit_circle(&mut rng);
            assert!(p.length_sq() <= 1.0 + 1e-5);
        }
    }

    #[test]
    fn world_bounds_shade_and_wrap_constants() {
        let bounds = WorldBounds::new(2000.0);
        assert_eq!(bounds.sim_size(), 2000.0);
        assert_eq!(bounds.shade_start(), 3000.0);
        assert_eq!(bounds.shade_end(), 4000.0);
        assert_eq!(bounds.pellet_cull_radius(), 3500.0);
        assert!(bounds.contains(Vec2::new(2000.0, -2000.0)));
        assert!(!bounds.contains(Vec2::new(2000.1, 0.0)));
        assert_eq!(
            bounds.clamp_position(Vec2::new(5000.0, -5000.0)),
            Vec2::new(2000.0, -2000.0)
        );
        assert!(!bounds.pellet_out_of_bounds(Vec2::new(3000.0, 0.0)));
        assert!(bounds.pellet_out_of_bounds(Vec2::new(3501.0, 0.0)));

        // World wrap triggers at |pos| - bodyLength >= 1.5S + 1000.
        let body_length = 10.0;
        assert!(
            bounds
                .wrap_position(Vec2::new(0.0, 4000.0), body_length)
                .is_none()
        );
        let wrapped = bounds
            .wrap_position(Vec2::new(0.0, 4010.0), body_length)
            .expect("wrap");
        assert!(wrapped.y < 0.0);
        assert!((wrapped.length() - (4000.0 - 0.95 * body_length)).abs() < 1e-3);
        assert_eq!(bounds.zone_centre_limit(500.0), 1500.0);
        assert_eq!(
            bounds.zone_rect_centre_limit(Vec2::new(100.0, 250.0)),
            Vec2::new(1900.0, 1750.0)
        );
    }

    #[test]
    fn hash_retain_drops_missing_entries() {
        let mut hash = test_hash();
        for slot in 0..10u32 {
            hash.insert(
                handle(slot),
                SpatialClass::Pellet,
                Vec2::splat(slot as f32),
                1.0,
            );
        }
        assert_eq!(hash.len(), 10);
        hash.retain(SpatialClass::Pellet, |h| matches!(h.slot(), 0 | 2 | 4 | 6));
        assert_eq!(hash.len(), 4);
        let mut scratch = QueryScratch::new();
        let result = hash.query_circle(Vec2::ZERO, 100.0, ClassMask::PELLET, &mut scratch);
        let slots: Vec<u32> = result.hits.iter().map(|hit| hit.handle.slot()).collect();
        assert_eq!(slots, vec![0, 2, 4, 6]);
        // Retaining a different class does not touch pellets.
        hash.retain(SpatialClass::Pheromone, |_| false);
        assert_eq!(hash.len(), 4);
    }

    fn zone_with(
        base: crate::zone::ZoneConfig,
        apply: impl FnOnce(&mut settings::ZoneSettings),
    ) -> crate::zone::ZoneConfig {
        let mut zone = base;
        apply(&mut zone.0);
        zone
    }

    fn globals_with(base: ZoneGlobals, apply: impl FnOnce(&mut SimSettings)) -> ZoneGlobals {
        let mut globals = base;
        apply(&mut globals.0);
        globals
    }

    fn default_zip_world() -> World {
        use crate::zone::{SpawnDistribution, ZoneConfig, ZoneStore};
        let bounds = WorldBounds::new(2000.0);
        let globals = globals_with(ZoneGlobals::game_defaults(), |settings| {
            settings.independents.biomass_density = 0.01;
            settings.independents.pellet_growth = 1e-5;
        });
        let config = zone_with(ZoneConfig::default_zone(), |zone| {
            zone.name = "World".to_owned();
            zone.radius_relative = 1.0;
            zone.fertility = 0.7827918;
            zone.biomass_density = 1.0;
            zone.pellet_size = 1.0;
            zone.distribution = SpawnDistribution::CentricGradual;
        });
        let zones = ZoneStore::from_configs(vec![config], bounds, globals, 7);
        World::from_zone_store(bounds, zones, 7)
    }

    #[test]
    fn pellet_placer_matches_game_pile_rules() {
        let mut world = default_zip_world();
        let origin = Vec2::new(100.0, 100.0);
        // A single pellet lands exactly on the click.
        let one = world.place_pellets_like_game(MaterialKind::Plant, origin, 1, 1.0);
        assert_eq!(one.len(), 1);
        assert_eq!(world.pellets.get(one[0]).unwrap().pos, origin);
        // A pile stays inside its radius and has the requested size.
        let radius = world.pellet_placer_pile_radius(MaterialKind::Plant, 5, 10.0);
        assert!(radius > 0.0);
        let pile = world.place_pellets_like_game(MaterialKind::Plant, origin, 5, 10.0);
        assert_eq!(pile.len(), 5);
        for id in pile {
            let view = world.pellets.get(id).unwrap();
            assert!(view.pos.distance(origin) <= radius + 1e-3);
        }
    }

    #[test]
    fn data_logger_records_one_point_per_simulated_minute() {
        let mut world = default_zip_world();
        let _ = world.initial_seeding();
        let ticks = (world.settings.sim_tps * 65) as usize;
        for _ in 0..ticks {
            world.tick();
        }
        use crate::datalog::group;
        let count = &world.data_logger.groups[group::COUNT].history;
        assert_eq!(count.n_data_at_scale(0), 1, "one point after 65 s");
        let plants = count.get(0, 0)[1];
        assert!(plants > 300.0, "plants logged: {plants}");
        let biomass = &world.data_logger.groups[group::BIOMASS].history;
        assert!(biomass.get(0, 0)[3].is_finite());
        for group in &world.data_logger.groups {
            assert_eq!(group.history.n_data_at_scale(0), 1, "{}", group.title);
        }
    }

    #[test]
    fn zones_log_with_the_species_every_five_simulated_minutes() {
        let mut world = default_zip_world();
        let _ = world.initial_seeding();
        assert_eq!(
            world.zones.zones[0].log.total_index_of_apparition(),
            -1,
            "nothing is logged yet"
        );
        let ticks = (world.settings.sim_tps * 305) as usize;
        for _ in 0..ticks {
            world.tick();
        }
        let zone = &world.zones.zones[0];
        assert_eq!(
            zone.log.total_index_of_apparition(),
            0,
            "one point after five minutes"
        );
        let point = zone.log.at(0);
        assert!(point.present());
        assert_eq!((point.pos_x, point.pos_y), (0, 0));
        assert_eq!(point.radius, 32767, "a zone as big as the world");
        assert!((point.fertility - zone.total_growth).abs() < 1e-3);
        assert!(point.biomass > 0.0, "the zone owns the pellets it seeded");
    }

    #[test]
    fn the_zone_logs_travel_through_data_bin_and_checkpoints() {
        let mut world = default_zip_world();
        let _ = world.initial_seeding();
        for step in 0..30 {
            world.zones.zones[0].config.pos_x = 0.01 * step as f32;
            world.zones.log_all();
        }
        let log = world.zones.zones[0].log.clone();
        // 12 points at full resolution and 9 compressed in twos.
        assert_eq!(log.total_index_of_apparition(), 20);
        // A world that was not loaded from a save writes a blank history around the zone block.
        let data = world.data_bin_for_save();
        assert!(!data.is_empty());
        let blocks = crate::data_bin::split_blocks(&data).expect("a data.bin");
        let entries = crate::data_bin::zone_entries(blocks[crate::data_bin::ZONE_BLOCK]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, 0, "the zone's id");
        // A world loaded from it has the same history.
        let mut loaded = default_zip_world();
        loaded.data_bin = data.clone();
        loaded.scenario_version = "0.6.3a3".to_owned();
        loaded.load_zone_logs();
        for index in 0..=20 {
            assert_eq!(
                loaded.zones.zones[0].log.at(index),
                log.at(index),
                "point {index}"
            );
        }
        // A log that has not changed is written back as it was loaded; a log that has is not.
        assert_eq!(loaded.data_bin_for_save(), data);
        let before = loaded.zones.zones[0].log.at(0);
        loaded.zones.log_all();
        assert_ne!(loaded.data_bin_for_save(), data);
        assert_eq!(
            loaded.zones.zones[0].log.at(1),
            before,
            "the old newest point is index 1"
        );
        // A save from before the zone block was added is not read.
        let mut old = default_zip_world();
        old.data_bin = data;
        old.scenario_version = "0.6.0a9".to_owned();
        old.load_zone_logs();
        assert_eq!(old.zones.zones[0].log.total_index_of_apparition(), -1);
        // A checkpoint carries the logs too.
        let bytes = world.zones.encode_logs();
        let mut restored = default_zip_world();
        restored.zones.restore_logs(&bytes).expect("restores");
        assert_eq!(restored.zones.encode_logs(), bytes);
        for index in 0..=20 {
            assert_eq!(restored.zones.zones[0].log.at(index), log.at(index));
        }
        // The same logs do not fit a world with other zones.
        let mut two = default_zip_world();
        let extra = two.zones.zones[0].config.clone();
        two.zones.add_zone(extra);
        assert!(two.zones.restore_logs(&bytes).is_err());
    }

    #[test]
    fn the_genes_are_logged_when_an_hour_is_full() {
        use crate::datalog::group;
        let mut world = default_zip_world();
        let _ = world.initial_seeding();
        // The 59th minute of the hour has been logged; the next one fills it.
        world.data_logger.n_minutes = 59;
        assert_eq!(world.data_logger.genes.n_data(), 0);
        let ticks = (world.settings.sim_tps * 65) as usize;
        for _ in 0..ticks {
            world.tick();
        }
        assert_eq!(
            world.data_logger.genes.n_data(),
            1,
            "one point for the hour"
        );
        assert_eq!(
            world.data_logger.save_state_bin()[4..8],
            0_i32.to_le_bytes(),
            "the minutes start over"
        );
        // Every gene's histogram counts each living bibite and egg once: the same population as the count stream logged that minute.
        let logged = world.data_logger.groups[group::COUNT].history.get(0, 0)[0];
        let point = world.data_logger.genes.get(0).expect("the newest point");
        for (gene, histogram) in point.genes.iter().enumerate() {
            let sum: u32 = histogram.data.iter().map(|n| u32::from(*n)).sum();
            assert_eq!(sum as f32, logged, "gene {gene}");
        }
        // The page's "Now" is the same distribution of the population as it is.
        let now = world.current_genes_buckets();
        let now_sum: u32 = now[0].data.iter().map(|n| u32::from(*n)).sum();
        assert_eq!(
            now_sum as usize,
            world
                .bibites
                .iter()
                .filter(|(_, record)| !record.body.dead)
                .count()
                + world.eggs.len()
        );
    }

    #[test]
    fn the_graphs_history_travels_through_data_bin() {
        use crate::datalog::group;
        let mut world = default_zip_world();
        let _ = world.initial_seeding();
        // A world that has not logged anything writes no history.
        assert!(!world.data_logger.has_history());
        // Two simulated minutes log two points of every stream.
        let ticks = (world.settings.sim_tps * 125) as usize;
        for _ in 0..ticks {
            world.tick();
        }
        assert_eq!(
            world.data_logger.groups[group::COUNT]
                .history
                .n_data_at_scale(0),
            2
        );
        // The births and deaths since the last point wait in the tracker.
        world.data_logger.hatches_since_log = 4;
        world.data_logger.deaths_since_log = 3;
        let data = world.data_bin_for_save();
        let blocks = crate::data_bin::split_blocks(&data).expect("a data.bin");
        assert_eq!(blocks.len(), 4);
        assert_ne!(
            blocks[crate::data_bin::LOGGER_BLOCK].len(),
            188,
            "more than a blank game's block"
        );

        // A world loaded from it has the same history, the same pending counts and writes the same bytes again.
        let mut loaded = default_zip_world();
        loaded.data_bin = data.clone();
        loaded.scenario_version = "0.6.3a3".to_owned();
        loaded.load_logger_blocks();
        for (index, group) in world.data_logger.groups.iter().enumerate() {
            assert_eq!(
                loaded.data_logger.groups[index].history.all_points(),
                group.history.all_points(),
                "{}",
                group.title
            );
        }
        // The progress to the next point and the minutes are the first 8 bytes of the logger block.
        assert_eq!(
            loaded.data_logger.save_state_bin()[..8],
            world.data_logger.save_state_bin()[..8]
        );
        assert_eq!(loaded.data_logger.hatches_since_log, 4);
        assert_eq!(
            loaded.data_logger.deaths_since_log, 3,
            "3 deaths since the last point"
        );
        assert_eq!(loaded.data_points()[group::BIRTH_DEATH], [4.0, 3.0]);
        assert_eq!(
            crate::data_bin::split_blocks(&loaded.data_bin_for_save()).expect("a data.bin")[..2],
            blocks[..2],
            "the history is written back as it was loaded"
        );
        // A save from before the format was changed is not read: the history stays empty.
        let mut old = default_zip_world();
        old.data_bin = data;
        old.scenario_version = "0.6.0a8".to_owned();
        old.load_logger_blocks();
        assert!(!old.data_logger.has_history());
    }

    #[test]
    fn the_point_of_one_species_is_its_sample_and_a_species_without_members_has_none() {
        use crate::brain::Brain;
        use crate::genome::Genome;
        use crate::species_log::SpeciesDataPoint;
        let mut world = default_zip_world();
        let first = spawn_test_bibite(&mut world, Vec2::new(100.0, 50.0));
        let second = spawn_test_bibite(&mut world, Vec2::new(-40.0, 10.0));
        let other = spawn_test_bibite(&mut world, Vec2::new(10.0, 10.0));
        let ours = world.species.add_template_species(
            "Zzpoint zzspecies",
            "",
            &Genome::starter(),
            &Brain::starter(),
        );
        let theirs = world.species.add_template_species(
            "Zzother zzspecies",
            "",
            &Genome::starter(),
            &Brain::starter(),
        );
        for (bibite, species) in [(first, ours), (second, ours), (other, theirs)] {
            world
                .bibites
                .get_mut(bibite)
                .expect("the bibite")
                .meta
                .species = Some(species);
        }
        let samples = world.species_samples();
        assert_eq!(samples.len(), 2);
        for (id, sample) in &samples {
            assert_eq!(world.species_point_now(*id), *sample);
        }
        // Two bibites, their mean position and a spread around it.
        let point = world.species_point_now(ours);
        assert_eq!(point.count, 2);
        assert_eq!(world.species_point_now(theirs).count, 1);
        assert!(point.avg_pos[0] > -40.0 && point.avg_pos[0] < 100.0);
        assert!(point.pos_std_dev[0] > 0.0);
        // A species without members has no point.
        assert_eq!(
            world.species_point_now(crate::ids::SpeciesId::new(99_999)),
            SpeciesDataPoint::default()
        );
    }

    #[test]
    fn the_current_data_points_follow_the_groups_and_a_death_feeds_the_age_of_death_stream() {
        use crate::datalog::group;
        let mut world = default_zip_world();
        let _ = world.initial_seeding();
        let points = world.data_points();
        assert_eq!(points.len(), world.data_logger.groups.len());
        for (point, stream) in points.iter().zip(&world.data_logger.groups) {
            assert_eq!(point.len(), stream.streams.len(), "{}", stream.title);
        }
        // Peeking logs nothing and changes nothing.
        assert_eq!(world.data_points(), points);
        assert!(
            world
                .data_logger
                .groups
                .iter()
                .all(|stream| stream.history.n_data_at_scale(0) == 0)
        );
        assert_eq!(points[group::DEATH_AGE], vec![0.0; 4], "nobody died yet");

        // A bibite that lived two hours and dies feeds the stream with its age (`BibiteDeath`).
        let bibite = spawn_test_bibite(&mut world, Vec2::new(0.0, 10.0));
        world
            .bibites
            .get_mut(bibite)
            .expect("the bibite")
            .clock
            .time_alive = 7200.0;
        assert!(world.kill_bibite(bibite));
        world.run(5);
        let dead = world.data_points()[group::DEATH_AGE].clone();
        assert!(
            dead.iter().all(|age| (age - 2.0).abs() < 0.01),
            "the only death was at 2 hours: {dead:?}"
        );
        // The births and deaths since the last log are the current value of the last group.
        let birth_death = &world.data_points()[group::BIRTH_DEATH];
        assert!(birth_death[1] >= 1.0, "{birth_death:?}");
    }

    #[test]
    fn world_default_scenario_seeds_and_ticks() {
        let mut world = default_zip_world();
        let report = world.initial_seeding();
        assert!(!report.capped);
        let seeded = world.pellets.counts().plant_count;
        assert!(seeded > 380 && seeded < 520, "seeded {seeded}");
        assert_eq!(world.pellets.len(), seeded as usize);
        assert_eq!(world.hash.len(), seeded as usize);
        assert_eq!(world.zones.zones[0].owned_pellets.len(), seeded as usize);

        // Every seeded pellet is owned by the zone and findable in the hash.
        let zone_id = world.zones.zones[0].id;
        let mut scratch = QueryScratch::with_capacity(4096);
        let result = world
            .hash
            .query_circle(Vec2::ZERO, 2100.0, ClassMask::PELLET, &mut scratch);
        assert_eq!(result.len(), seeded as usize);
        assert!(!result.truncated);
        for id in world.pellets.ids() {
            assert_eq!(world.pellets.owner_zone(id), Some(Some(zone_id)));
        }

        let before = world.pellets.counts().plant_count;
        for _ in 0..200 {
            world.tick();
        }
        assert!(world.pellets.counts().plant_count > before);
        assert_eq!(world.pellets.len(), world.hash.len());
        assert_eq!(
            world.pellets.len(),
            world.zones.zones[0].owned_pellets.len()
        );
        // Under 10 s of simulated time: no recount yet (the lag is intentional).
        assert!(world.zones.zones[0].time_since_recount < 10.0);
        assert!(world.zones.zones[0].pellet_biomass > 0.0);
        assert!(world.free_biomass() < world.total_max_biomass());
        assert_eq!(world.zone_at(Vec2::ZERO), Some(world.zones.zones[0].id));
    }

    #[test]
    fn world_spawn_change_despawn_keeps_hash_in_sync() {
        use crate::pellet::MaterialKind;
        let mut world =
            World::new_ecology(WorldBounds::new(100.0), ZoneGlobals::game_defaults(), 1);
        let id = world
            .spawn_pellet(PelletSpawn::new(
                MaterialKind::Plant,
                40.0,
                Vec2::new(5.0, 5.0),
            ))
            .expect("spawn");
        assert!(world.hash.contains(id.handle(), SpatialClass::Pellet));
        assert_eq!(world.hash.len(), 1);

        let before_radius = world.pellets.radius(id).unwrap();
        world.change_pellet_amount(id, -20.0);
        let after_radius = world.pellets.radius(id).unwrap();
        assert!(after_radius < before_radius);
        assert_eq!(
            world.hash.radius_of(id.handle(), SpatialClass::Pellet),
            Some(after_radius)
        );

        world.move_pellet(id, Vec2::new(50.0, 0.0));
        assert_eq!(
            world.hash.pos_of(id.handle(), SpatialClass::Pellet),
            Some(Vec2::new(50.0, 0.0))
        );

        world.despawn_pellet(id);
        assert!(!world.pellets.contains(id));
        assert!(!world.hash.contains(id.handle(), SpatialClass::Pellet));
        assert!(world.hash.is_empty());
    }

    #[test]
    fn world_spawn_pellet_at_zone_owner_is_recyclable() {
        use crate::pellet::MaterialKind;
        use crate::zone::{SpawnDistribution, ZoneConfig, ZoneStore};
        let bounds = WorldBounds::new(100.0);
        let globals = globals_with(ZoneGlobals::game_defaults(), |settings| {
            settings.independents.biomass_density = 0.1;
        });
        let config = zone_with(ZoneConfig::default_zone(), |zone| {
            zone.radius_is_relative = false;
            zone.radius_absolute = 10.0;
            zone.distribution = SpawnDistribution::Flat;
        });
        let zones = ZoneStore::from_configs(vec![config], bounds, globals, 1);
        let mut world = World::from_zone_store(bounds, zones, 1);
        let zone_id = world.zones.zones[0].id;
        let id = world
            .spawn_pellet(PelletSpawn {
                material: MaterialKind::Plant,
                amount: 6.0,
                pos: Vec2::ZERO,
                velocity: Vec2::ZERO,
                rotation: 0.0,
                angular_velocity: 0.0,
                owner_zone: Some(zone_id),
                held: false,
            })
            .expect("spawn");
        assert_eq!(world.zones.zones[0].owned_pellets, vec![id]);
        world.despawn_pellet(id);
        assert!(world.zones.zones[0].owned_pellets.is_empty());
    }

    #[test]
    fn world_phero_tower_syncs_spot_and_hash() {
        use crate::math::PI;
        use crate::tools::{PheroTower, Tool};
        let mut world =
            World::new_ecology(WorldBounds::new(2000.0), ZoneGlobals::game_defaults(), 1);
        let mut tower = PheroTower::new(1, Vec2::new(10.0, 10.0));
        tower.red = 3.0;
        tower.heading = PI / 2.0;
        world.add_phero_tower(tower);
        world.step_tools(0.0);
        assert_eq!(world.pheromones.len(), 1);
        assert_eq!(world.hash.len(), 1);
        let spot = world.pheromones.ids().next().expect("spot");
        assert!(!world.pheromones.dissipates(spot).unwrap());
        assert!(world.hash.contains(spot.handle(), SpatialClass::Pheromone));

        // Non-dissipating tower spot survives many dissipation ticks.
        for _ in 0..100 {
            world.step_pheromones(1.0);
        }
        assert!(world.pheromones.contains(spot));

        // Moving the tower recreates the spot at the new position and updates
        // the spatial hash.
        let tool_id = crate::tools::ToolStore::tool_id_at(0).expect("tool id");
        let Tool::Phero(tower) = world.tools.get_mut(tool_id).unwrap() else {
            panic!("not a phero tower");
        };
        tower.base.pos = Vec2::new(100.0, 0.0);
        world.step_tools(0.0);
        assert_eq!(world.pheromones.len(), 1);
        let moved = world.pheromones.ids().next().unwrap();
        assert_ne!(moved, spot);
        assert!(!world.pheromones.contains(spot));
        assert_eq!(world.pheromones.pos(moved), Some(Vec2::new(100.0, 0.0)));
        assert_eq!(world.hash.len(), 1);
        assert!(world.hash.contains(moved.handle(), SpatialClass::Pheromone));
        assert!(!world.hash.contains(spot.handle(), SpatialClass::Pheromone));
    }

    /// `Pherosense` finds spots with `Physics2D.OverlapCircleNonAlloc`: a spot whose trigger collider (radius 0.001 for the spots bibites emit, 0.1
    /// for a tower's) touches the sense circle is sensed although its centre lies outside it.
    #[test]
    fn a_spots_collider_reaches_into_the_sense_circle() {
        use crate::tools::PheroTower;
        let mut world =
            World::new_ecology(WorldBounds::new(2000.0), ZoneGlobals::game_defaults(), 1);
        // A tower's spot at 100.05 from the sensing bibite, a bibite-emitted one at 100.0005 on the other side.
        let mut tower = PheroTower::new(1, Vec2::new(100.05, 0.0));
        tower.red = 3.0;
        world.add_phero_tower(tower);
        world.step_tools(0.0);
        let emitted = world
            .pheromones
            .emit(PheroEmission {
                pos: Vec2::new(-100.0005, 0.0),
                heading: Vec2::Y,
                strength: [1.0, 0.0, 0.0],
                dissipate: true,
                owner_tool: None,
            })
            .expect("spot");
        world.register_pheromone(emitted);
        let mut scratch = QueryScratch::default();
        let mut hits_at = |radius: f32| -> usize {
            world
                .hash
                .query_circle_unsorted(Vec2::ZERO, radius, ClassMask::PHEROMONE, &mut scratch)
                .hits
                .len()
        };
        assert_eq!(hits_at(99.9), 0, "both centres and colliders are outside");
        // The tower's collider (0.1) reaches 100.05 - 0.1 = 99.95; the emitted one (0.001) reaches 99.9995.
        assert_eq!(hits_at(99.96), 1);
        assert_eq!(hits_at(99.9996), 2);
        assert_eq!(hits_at(100.1), 2);
    }

    #[test]
    fn world_rad_dose_collection_uses_tower_cadence() {
        use crate::ids::BibiteId;
        use crate::tools::{BibiteTarget, RadTower};
        let mut world =
            World::new_ecology(WorldBounds::new(2000.0), ZoneGlobals::game_defaults(), 1);
        let mut tower = RadTower::new(1, Vec2::ZERO, 2000.0);
        tower.radius = 100.0;
        tower.count_rate = 0.01;
        world.add_rad_tower(tower);
        let target = BibiteTarget {
            id: BibiteId::from_handle(Handle::new(3, 0)),
            pos: Vec2::new(10.0, 0.0),
        };
        // 0.25 s -> not due.
        assert!(world.collect_rad_doses(0.25, [target]).is_empty());
        // Next 0.25 s -> one dose with the taper factor at d=10/100 -> 0.9.
        let doses = world.collect_rad_doses(0.25, [target]);
        assert_eq!(doses.len(), 1);
        assert!((doses[0].count - 0.01 * 0.5 * 0.9).abs() < 1e-6);
    }

    #[test]
    fn world_red_death_defaults_to_disabled() {
        let mut world =
            World::new_ecology(WorldBounds::new(2000.0), ZoneGlobals::game_defaults(), 1);
        assert!(!world.red_death().enabled);
        assert_eq!(world.red_death().safe_radius, f32::INFINITY);
        assert!(!world.step_red_death_timer(100.0));
        world.red_death_mut().set_enabled(true);
        world.red_death_mut().set_fill(0.75);
        // shade_end = 1.5*2000+1000 = 4000; safe radius = 4000*sqrt(0.25)=2000.
        assert!((world.red_death().safe_radius - 2000.0).abs() < 1e-3);
        assert!(world.step_red_death_timer(0.5));
    }

    /// A world made from settings that have the Red Death bloom on starts with it on (`RedDeathBloomManager` reads the loaded settings): a saved world with
    /// Red Death enabled hurt nothing, was not smelled and drew nothing until some setting was edited.
    #[test]
    fn a_world_made_from_settings_with_red_death_on_has_the_bloom() {
        let mut settings = default_settings();
        settings.scenario.enable_red_death = true;
        settings.scenario.red_death_fill = 0.75;
        settings.scenario.red_death_base_damages = 2.0;
        let world = World::new(settings, ScenarioData::default(), 1).expect("world");
        let bloom = world.red_death();
        assert!(bloom.enabled);
        assert!((bloom.fill - 0.75).abs() < 1e-6 && (bloom.base_damage - 2.0).abs() < 1e-6);
        assert!(bloom.safe_radius.is_finite() && (bloom.safe_radius - bloom.shade_end * 0.5).abs() < 1e-3);
        // Switched off in the settings, it stays off.
        let off = World::new(default_settings(), ScenarioData::default(), 1).expect("world");
        assert!(!off.red_death().enabled && off.red_death().safe_radius == f32::INFINITY);
    }

    // ----- World::new / ScenarioData -----

    fn default_settings() -> SimSettings {
        SimSettings::game_defaults()
    }

    #[test]
    fn new_world_from_defaults_is_empty_and_settings_driven() {
        use crate::body::BibiteStore;
        let settings = default_settings();
        let world = World::new(settings.clone(), ScenarioData::default(), 42).expect("world");
        assert_eq!(world.settings, settings);
        assert_eq!(world.clock.tps(), settings.sim_tps);
        assert_eq!(world.clock.dt(), settings.fixed_dt());
        assert_eq!(world.decay_tps, settings.decay_tps);
        assert_eq!(
            world.bounds.sim_size(),
            settings.independents.simulation_size
        );
        assert!(world.zones.zones.is_empty());
        assert!(world.pellets.is_empty());
        assert!(world.pheromones.is_empty());
        assert_eq!(world.bibites.len(), 0);
        assert_eq!(world.eggs.len(), 0);
        assert_eq!(world.species.species_count(), 0);
        assert_eq!(world.tools.len(), 0);
        assert_eq!(world.bibites.settings(), &settings);
        assert_eq!(BibiteStore::new().len(), 0);
        // Scenario metadata defaults are empty.
        assert!(world.scenario_name.is_empty());
        assert!(world.scenario_version.is_empty());
        assert!(world.tags.is_empty());
        assert!(world.bibite_spawns.is_empty());
    }

    #[test]
    fn new_world_applies_material_settings() {
        let mut settings = default_settings();
        settings.materials.meat.energy_density = 12.5;
        let world = World::new(settings, ScenarioData::default(), 1).expect("world");
        let meat = world
            .pellets
            .materials()
            .get(crate::pellet::MaterialKind::Meat);
        assert_eq!(meat.energy_density, 12.5);
        assert!(meat.decay);
        assert_eq!(meat.fresh_time, 2000.0);
        assert_eq!(world.settings.materials.meat.energy_density, 12.5);
    }

    // ----- Live edits of a material's decay settings (`SettingsUpdater`) -----

    /// Settings with plant decay `on`, quick and strong enough to show in a few seconds of game time: fresh for 1 s, then 5 u2/s.
    fn plant_decay_settings(on: bool) -> SimSettings {
        let mut settings = default_settings();
        settings.materials.plant.decay = on;
        settings.materials.plant.fresh_time = 1.0;
        settings.materials.plant.decay_rate = 5.0;
        settings
    }

    /// Ticks the world for `seconds` of game time.
    fn tick_for(world: &mut World, seconds: f32) {
        let ticks = (seconds / world.clock.dt()).ceil() as usize;
        for _ in 0..ticks {
            world.tick();
        }
    }

    #[test]
    fn plants_rot_in_a_world_that_starts_with_plant_decay_on() {
        let mut world = World::new(plant_decay_settings(true), ScenarioData::default(), 1).expect("world");
        let id = world
            .spawn_pellet_with_amount(crate::pellet::MaterialKind::Plant, Vec2::new(10.0, 10.0), 20.0)
            .expect("pellet");
        assert!(world.pellets.is_decaying(id));
        tick_for(&mut world, 30.0);
        assert!(!world.pellets.contains(id), "20 u2 at 5 u2/s after 1 s are gone in 30 s");
        assert!(world.accounting.decayed_plant_energy > 0.0);
    }

    #[test]
    fn switching_plant_decay_on_while_the_world_runs_makes_the_plants_already_there_rot() {
        // `SettingsUpdater.UpdateDecay`: the toggle registers EVERY pellet of the material with the decay processor, fresh timer from the
        // material (not only the ones spawned afterwards).
        let mut world = World::new(plant_decay_settings(false), ScenarioData::default(), 1).expect("world");
        let id = world
            .spawn_pellet_with_amount(crate::pellet::MaterialKind::Plant, Vec2::new(10.0, 10.0), 20.0)
            .expect("pellet");
        assert!(!world.pellets.is_decaying(id));
        tick_for(&mut world, 30.0);
        assert!(world.pellets.contains(id), "no decay yet: the plant is still there");
        world.apply_settings(plant_decay_settings(true)).expect("settings");
        assert!(world.pellets.is_decaying(id), "the toggle registered the existing plant");
        assert!((world.pellets.fresh_time_remaining(id).expect("pellet") - 1.0).abs() < 1e-4);
        assert_eq!(world.pellets.rot_amount(id), Some(0.0));
        tick_for(&mut world, 30.0);
        assert!(!world.pellets.contains(id), "the plant that was there before the toggle rotted away");
    }

    #[test]
    fn switching_plant_decay_off_stops_the_rot_of_the_plants_already_there() {
        let mut world = World::new(plant_decay_settings(true), ScenarioData::default(), 1).expect("world");
        let id = world
            .spawn_pellet_with_amount(crate::pellet::MaterialKind::Plant, Vec2::new(10.0, 10.0), 20.0)
            .expect("pellet");
        tick_for(&mut world, 2.0);
        world.apply_settings(plant_decay_settings(false)).expect("settings");
        assert!(!world.pellets.is_decaying(id));
        let amount = world.pellets.amount(id).expect("pellet");
        tick_for(&mut world, 30.0);
        assert_eq!(world.pellets.amount(id), Some(amount), "a plant that does not decay keeps its amount");
        // Switching it on again starts the fresh timer over (`TryAddUnique`).
        world.apply_settings(plant_decay_settings(true)).expect("settings");
        assert!((world.pellets.fresh_time_remaining(id).expect("pellet") - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_new_freshness_time_moves_the_fresh_timers_of_the_decaying_plants_by_the_difference() {
        let mut settings = plant_decay_settings(true);
        settings.materials.plant.fresh_time = 1000.0;
        let mut world = World::new(settings.clone(), ScenarioData::default(), 1).expect("world");
        let id = world
            .spawn_pellet_with_amount(crate::pellet::MaterialKind::Plant, Vec2::new(10.0, 10.0), 20.0)
            .expect("pellet");
        let meat = world
            .spawn_pellet_with_amount(crate::pellet::MaterialKind::Meat, Vec2::new(-10.0, 10.0), 20.0)
            .expect("pellet");
        let meat_fresh = world.pellets.fresh_time_remaining(meat).expect("pellet");
        // `UpdateFreshValueDifference` (new - old) then `UpdateDecayParameters`: every decaying pellet of the material gets the difference.
        settings.materials.plant.fresh_time = 400.0;
        world.apply_settings(settings.clone()).expect("settings");
        assert!((world.pellets.fresh_time_remaining(id).expect("pellet") - 400.0).abs() < 1e-3);
        assert_eq!(world.pellets.fresh_time_remaining(meat), Some(meat_fresh), "the other material is not touched");
        // The decay rate's own handler runs `UpdateDecayParameters` with the LAST difference again (the game's quirk: `freshValueDifference`
        // is a field that stays): -600 once more.
        settings.materials.plant.decay_rate = 6.0;
        world.apply_settings(settings).expect("settings");
        assert!((world.pellets.fresh_time_remaining(id).expect("pellet") - (-200.0)).abs() < 1e-3);
    }

    #[test]
    fn an_edit_that_leaves_the_decay_settings_alone_does_not_touch_the_pellets() {
        let mut world = World::new(plant_decay_settings(true), ScenarioData::default(), 1).expect("world");
        let id = world
            .spawn_pellet_with_amount(crate::pellet::MaterialKind::Plant, Vec2::new(10.0, 10.0), 20.0)
            .expect("pellet");
        tick_for(&mut world, 2.0);
        let (fresh, rot) = (world.pellets.fresh_time_remaining(id), world.pellets.rot_amount(id));
        let mut settings = plant_decay_settings(true);
        settings.materials.plant.energy_density = 6.0;
        world.apply_settings(settings).expect("settings");
        assert_eq!((world.pellets.fresh_time_remaining(id), world.pellets.rot_amount(id)), (fresh, rot));
        assert!(world.pellets.is_decaying(id));
    }

    #[test]
    fn new_world_builds_zones_pellets_and_tools() {
        use crate::pellet::MaterialKind;
        use crate::settings::{ZoneGroupSettings, ZoneSettings};
        use crate::tools::{ColorKiller, PheroTower, RadTower};

        let mut zone = ZoneSettings::game_defaults();
        zone.name = "explicit".to_owned();
        zone.zone_id = Some(1);
        zone.pos_x = 0.25;
        zone.pos_y = -0.25;
        let scenario = ScenarioData {
            name: "scenario".to_owned(),
            version: "1.2.3".to_owned(),
            zones: vec![zone],
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            bibite_spawns: vec![crate::settings::BibiteSpawn::game_defaults()],
            initial_pellets: vec![PelletSpawn::new(
                MaterialKind::Plant,
                20.0,
                Vec2::new(100.0, 0.0),
            )],
            rad_towers: vec![RadTower::new(1, Vec2::ZERO, 2000.0)],
            phero_towers: vec![PheroTower::new(2, Vec2::new(10.0, 0.0))],
            color_killers: vec![ColorKiller::new(3, Vec2::new(-10.0, 0.0))],
            settings_changers: Vec::new(),
            tags: vec!["tag".to_owned()],
        };
        let world = World::new(default_settings(), scenario, 7).expect("world");
        // One explicit zone + the default group's generated zones.
        assert_eq!(world.zones.zones.len(), 1 + 10);
        assert_eq!(world.zones.zones[0].config.name, "explicit");
        assert_eq!(world.zones.zones[1].config.name, "_0");
        assert_eq!(world.pellets.len(), 1);
        assert_eq!(world.hash.len(), 1);
        assert_eq!(world.tools.len(), 3);
        assert_eq!(world.scenario_name, "scenario");
        assert_eq!(world.scenario_version, "1.2.3");
        assert_eq!(world.tags, vec!["tag".to_owned()]);
        assert_eq!(world.bibite_spawns.len(), 1);
    }

    #[test]
    fn new_world_seed_is_deterministic() {
        use crate::settings::ZoneGroupSettings;
        let scenario = || ScenarioData {
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            ..ScenarioData::default()
        };
        let a = World::new(default_settings(), scenario(), 99).expect("a");
        let b = World::new(default_settings(), scenario(), 99).expect("b");
        let c = World::new(default_settings(), scenario(), 100).expect("c");
        assert_eq!(a.zones.zones.len(), b.zones.zones.len());
        for (za, zb) in a.zones.zones.iter().zip(&b.zones.zones) {
            assert_eq!(za.pos, zb.pos);
            assert_eq!(za.config.name, zb.config.name);
            assert_eq!(za.id, zb.id);
        }
        assert!(
            a.zones
                .zones
                .iter()
                .zip(&c.zones.zones)
                .any(|(za, zc)| za.pos != zc.pos)
        );
    }

    #[test]
    fn new_world_seed_scenario_seeds_pellets_when_configured() {
        use crate::settings::{ZoneGroupSettings, ZoneSettings};
        let mut zone = ZoneSettings::game_defaults();
        zone.name = "seed".to_owned();
        zone.fertility = 1.0;
        let scenario = ScenarioData {
            zones: vec![zone],
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            ..ScenarioData::default()
        };
        let mut world = World::new(default_settings(), scenario, 3).expect("world");
        let before = world.pellets.len();
        let report = world.seed_scenario_world().expect("seeded");
        assert_eq!(report.spawned as usize, world.pellets.len() - before);
        assert!(report.spawned > 0);
        // Second call refuses: already-seeded world is the caller's decision, but
        // the settings gate is still open, so it seeds again (documented).
        assert!(world.seed_scenario_world().is_some());
    }

    #[test]
    fn seed_scenario_world_respects_disabled_initial_seeding() {
        let mut settings = default_settings();
        settings.scenario.initial_seeding = 0.0;
        let mut world =
            World::new(settings, ScenarioData::with_default_zone_group(), 3).expect("world");
        assert!(world.seed_scenario_world().is_none());
        assert!(world.pellets.is_empty());
    }

    #[test]
    fn new_world_rejects_invalid_settings() {
        let mut settings = default_settings();
        settings.sim_tps = 0;
        assert!(matches!(
            World::new(settings, ScenarioData::default(), 1),
            Err(crate::error::Error::InvalidData(_))
        ));
    }

    #[test]
    fn new_world_rejects_invalid_scenario_rows() {
        use crate::settings::BibiteSpawn;
        let mut scenario = ScenarioData::default();
        scenario.initial_pellets.push(PelletSpawn::new(
            crate::pellet::MaterialKind::Plant,
            0.0,
            Vec2::ZERO,
        ));
        assert!(matches!(
            World::new(default_settings(), scenario, 1),
            Err(Error::InvalidData(_))
        ));

        let mut scenario = ScenarioData {
            bibite_spawns: vec![BibiteSpawn::game_defaults()],
            ..ScenarioData::default()
        };
        scenario.bibite_spawns[0].minimum = -1;
        assert!(matches!(
            World::new(default_settings(), scenario, 1),
            Err(Error::InvalidData(_))
        ));

        let mut scenario = ScenarioData::default();
        let mut killer = crate::tools::ColorKiller::new(1, Vec2::ZERO);
        killer.period = 0.0;
        scenario.color_killers.push(killer);
        assert!(matches!(
            World::new(default_settings(), scenario, 1),
            Err(Error::InvalidData(_))
        ));
    }

    // ----- W4a: full tick pipeline / stats / determinism -----

    fn spawn_test_bibite(world: &mut World, pos: Vec2) -> BibiteId {
        use crate::body::{BibiteSpawn, SpawnVitals};
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = pos;
        let id = world.bibites.spawn(spawn).expect("spawn bibite");
        world.register_bibite(id);
        id
    }

    fn scenario_with_zone_group() -> ScenarioData {
        use crate::settings::ZoneGroupSettings;
        ScenarioData {
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            ..ScenarioData::default()
        }
    }

    #[test]
    fn settings_animator_applies_scenario_and_zone_settings() {
        use crate::settings::{SettingBase, SettingsChanger, SplinePoint, ZoneSettings};
        let mut zone = ZoneSettings::game_defaults();
        zone.name = "animated".to_owned();
        zone.pos_x = 0.1;
        zone.fertility = 1.0;
        let changer = SettingsChanger {
            name: "anim".to_owned(),
            settings_bases: vec![
                SettingBase {
                    key: "pelletEnergy".to_owned(),
                    base: 200.0,
                },
                SettingBase {
                    key: "Zone(0).fertility".to_owned(),
                    base: 1.0,
                },
            ],
            points: vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(10.0, 3.0)],
            repeat: false,
            start: 0.0,
        };
        let scenario = ScenarioData {
            zones: vec![zone],
            settings_changers: vec![changer],
            ..ScenarioData::default()
        };
        let mut world = World::new(default_settings(), scenario, 41).expect("world");
        assert_eq!(world.settings.scenario.pellet_energy, 200.0);
        world.run(30); // 1.0 s -> factor 1.2
        assert!((world.settings.scenario.pellet_energy - 240.0).abs() < 1e-2);
        assert!((world.zones.zones[0].config.fertility - 1.2).abs() < 1e-4);
        // Run past the spline's end: the last written factor is near 3.0 (the
        // manager may stop one 0.1 s step before/after the exact end).
        world.run(300);
        assert!(world.settings.scenario.pellet_energy > 590.0);
        assert!(world.settings.scenario.pellet_energy <= 600.0);
        assert!(world.zones.zones[0].config.fertility > 2.95);
    }

    #[test]
    fn settings_animator_start_delay_and_repeat_wrap() {
        use crate::settings::{SettingBase, SettingsChanger, SplinePoint};
        let changer = SettingsChanger {
            name: "delayed".to_owned(),
            settings_bases: vec![SettingBase {
                key: "pelletEnergy".to_owned(),
                base: 100.0,
            }],
            points: vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(1.0, 2.0)],
            repeat: true,
            start: 1.0,
        };
        let mut world = World::new(
            default_settings(),
            ScenarioData {
                settings_changers: vec![changer],
                ..ScenarioData::default()
            },
            43,
        )
        .expect("world");
        // Before the start gate no writes.
        world.run(15); // 0.5 s
        assert_eq!(world.settings.scenario.pellet_energy, 200.0);
        world.run(30); // 1.5 s -> time-start = 0.5 -> factor 1.5
        assert!((world.settings.scenario.pellet_energy - 150.0).abs() < 1e-2);
        // Repeat: 2.0 s later wraps (period 1 s) back to factor 1.0 at the
        // wrapped x = 0.
        world.run(60); // total 3.5 s -> time-start = 2.5 -> wrap 0.5 -> 1.5
        assert!((world.settings.scenario.pellet_energy - 150.0).abs() < 1.0);
    }

    /// A negative `start` is a phase shift, not an error (`SettingsChanger.Update` only holds the animator back while `time - start` is below zero): the
    /// published scenario "Seasonal Desert v3" starts its winter animators at -43,200 and its world refused to build (the tester's Start returned to the menu).
    #[test]
    fn a_negative_animator_start_has_the_curve_already_under_way() {
        use crate::settings::{SettingBase, SettingsChanger, SplinePoint};
        let changer = SettingsChanger {
            name: "shifted".to_owned(),
            settings_bases: vec![SettingBase {
                key: "pelletEnergy".to_owned(),
                base: 100.0,
            }],
            points: vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(2.0, 3.0)],
            repeat: false,
            start: -1.0,
        };
        let mut world = World::new(
            default_settings(),
            ScenarioData {
                settings_changers: vec![changer],
                ..ScenarioData::default()
            },
            44,
        )
        .expect("a negative start is a valid animator");
        world.run(15); // 0.5 s: the curve is 1.5 s along -> factor 2.5
        let energy = world.settings.scenario.pellet_energy;
        assert!((230.0..=255.0).contains(&energy), "{energy}");
    }

    #[test]
    fn full_tick_empty_world_is_stable_and_samples_schema() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        world.run(60);
        assert_eq!(world.clock.tick(), 60);
        let sample = world.sample_stats();
        assert_eq!(sample.tick, 60);
        assert!((sample.t - 2.0).abs() < 1e-5);
        assert_eq!(sample.counts.bibites, 0);
        assert_eq!(sample.counts.eggs, 0);
        assert_eq!(sample.counts.corpses, 0);
        assert_eq!(sample.pellets.plant_n, 0);
        assert_eq!(sample.genes.mean.len(), crate::genome::N_GENES);
        assert_eq!(sample.behavior.outputs.len(), crate::brain::N_OUTPUTS);
        assert_eq!(sample.species.n, 0);
        assert!(sample.metrics.is_empty());
        // Stats reset the since-sample lifecycle counters.
        let next = world.sample_stats();
        assert_eq!(next.lifecycle.births, 0);
        assert_eq!(next.lifecycle.deaths, 0);
    }

    #[test]
    fn full_tick_deterministic_repeat_run() {
        let run = |seed: u64| {
            let mut world =
                World::new(default_settings(), scenario_with_zone_group(), seed).expect("world");
            world.seed_scenario_world();
            spawn_test_bibite(&mut world, Vec2::new(100.0, 100.0));
            spawn_test_bibite(&mut world, Vec2::new(-100.0, 50.0));
            world.run(300);
            let sample = world.sample_stats();
            (
                crate::sim::sample_fingerprint(&sample),
                crate::sim::state_fingerprint(&world),
            )
        };
        let a = run(1234);
        let b = run(1234);
        assert_eq!(a, b, "same settings/scenario/seed must be bit-identical");
        let c = run(99);
        assert_ne!(
            a.1, c.1,
            "a different seed should produce a different world trajectory"
        );
    }

    /// A bibite's spatial-hash entry follows the bibite (the others find it where it is, not where it was spawned or loaded).
    #[test]
    fn the_bibites_hash_entries_follow_the_bibites() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 7).expect("world");
        world.seed_scenario_world();
        let ids = [
            spawn_test_bibite(&mut world, Vec2::new(100.0, 100.0)),
            spawn_test_bibite(&mut world, Vec2::new(-100.0, 50.0)),
        ];
        let start: Vec<Vec2> = ids.iter().map(|id| world.bibites.get(*id).expect("bibite").pos).collect();
        world.run(400);
        let mut moved = 0.0_f32;
        for (id, from) in ids.iter().zip(&start) {
            let Some(record) = world.bibites.get(*id) else {
                continue;
            };
            moved = moved.max((record.pos - *from).length());
            let entry = world.hash.pos_of(id.handle(), SpatialClass::Bibite).expect("an entry");
            // (The entries are refreshed when the senses are due, a few ticks apart.)
            assert!(
                (entry - record.pos).length() < 60.0,
                "entry {entry:?} but the bibite is at {:?}",
                record.pos
            );
        }
        assert!(moved > 200.0, "the bibites moved ({moved})");
    }

    #[test]
    fn full_tick_multi_system_smoke() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 7).expect("world");
        world.seed_scenario_world();
        assert!(
            !world.pellets.is_empty(),
            "initial seeding should spawn plants"
        );
        let a = spawn_test_bibite(&mut world, Vec2::new(50.0, 50.0));
        let b = spawn_test_bibite(&mut world, Vec2::new(80.0, 50.0));
        let start_energy: f32 = [a, b]
            .iter()
            .filter_map(|id| world.bibites.get(*id).map(|r| r.body.energy.amount))
            .sum();
        world.run(600);
        let sample = world.sample_stats();
        assert!(sample.counts.bibites <= 2);
        assert!(sample.pellets.plant_n > 0, "zones keep producing plants");
        assert!(sample.vitals.energy_mean.is_finite());
        assert!(sample.genes.mean.iter().all(|value| value.is_finite()));
        assert!(
            sample
                .behavior
                .outputs
                .iter()
                .all(|value| value.is_finite())
        );
        assert!(sample.pellets.plant_biomass.is_finite());
        assert!(sample.vitals.energy_mean >= 0.0 && start_energy >= 0.0);
        // Every live entity has a spatial-hash entry after reconciliation.
        for id in world.bibites.ids() {
            assert!(world.hash.contains(id.handle(), SpatialClass::Bibite));
        }
        for id in world.pellets.ids() {
            assert!(world.hash.contains(id.handle(), SpatialClass::Pellet));
        }
        // No NaN in any live bibite's core state.
        for (_, record) in world.bibites.iter() {
            assert!(record.pos.is_finite());
            assert!(record.vel.is_finite());
            assert!(record.body.energy.amount.is_finite());
            assert!(record.body.health.amount.is_finite());
        }
    }

    #[test]
    fn sensing_prefilter_includes_bibites_whose_brain_senses_pheromones() {
        use crate::body::{BibiteSpawn, SpawnVitals};
        use crate::brain::{Brain, FIRST_OUTPUT, Input, Output, SynapseRecord};
        use crate::genome::{Genome, GenomeMeta};
        let mut world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        let starter = Brain::starter();
        let nodes = starter.to_node_records();
        let mut synapses = starter.to_synapse_records();
        synapses.push(SynapseRecord {
            innovation: 10,
            node_in: Input::PheroSense1.index() as u32,
            node_out: (FIRST_OUTPUT + Output::Accelerate.index()) as u32,
            weight: 1.0,
            enabled: true,
        });
        let sniffer = Brain::from_template_records(&nodes, &synapses, 0.5).expect("a valid brain");
        assert!(sniffer.needs_pheromone_sensing());
        for (index, brain) in [starter, sniffer.clone(), sniffer].into_iter().enumerate() {
            let mut spawn = BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), brain);
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(10.0 * index as f32, 10.0);
            let id = world.bibites.spawn(spawn).expect("spawn bibite");
            world.register_bibite(id);
        }
        // `step_sensing` compares its prefilter with the plain scan on every tick of a test build; on the brain ticks where
        // neither the vision lookup nor the sense is due, only the pheromone bit puts a bibite in the list.
        world.run(60);
    }

    #[test]
    fn a_rad_bibite_marks_the_bibites_inside_its_zone() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        let carrier = spawn_test_bibite(&mut world, Vec2::new(10.0, 10.0));
        let near = spawn_test_bibite(&mut world, Vec2::new(60.0, 10.0));
        let far = spawn_test_bibite(&mut world, Vec2::new(700.0, 700.0));
        world.bibites.get_mut(carrier).expect("carrier").meta.is_rad = true;
        world.run(3);
        assert!(world.bibites.get(near).expect("near").rad_zone);
        assert!(!world.bibites.get(far).expect("far").rad_zone);
        assert!(!world.bibites.get(carrier).expect("carrier").rad_zone);
    }

    #[test]
    fn full_tick_egg_hatches_into_bibite() {
        use crate::body::EggSpawn;
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world = World::new(default_settings(), ScenarioData::default(), 11).expect("world");
        let genome = Genome::starter();
        let brain = Brain::starter();
        let egg = world.eggs.spawn(EggSpawn {
            pos: Vec2::new(25.0, 25.0),
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome,
            meta: GenomeMeta::default(),
            brain,
            energy: 50.0,
            hatch_time: 0.1,
            total_required_energy: 1.0,
            birth_orientation: -Vec2::Y,
        });
        world.sync_egg_hash();
        assert!(world.hash.contains(egg.handle(), SpatialClass::Egg));
        world.run(10);
        assert_eq!(world.eggs.len(), 0);
        assert_eq!(world.bibites.len(), 1);
        assert!(!world.hash.contains(egg.handle(), SpatialClass::Egg));
        let child = world.bibites.ids().next().expect("child");
        assert!(world.hash.contains(child.handle(), SpatialClass::Bibite));
    }

    #[test]
    fn census_and_cull_removes_out_of_bounds_pellets() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 3).expect("world");
        let pellet = world.pellets.spawn(crate::pellet::PelletSpawn::new(
            crate::pellet::MaterialKind::Plant,
            10.0,
            Vec2::new(9000.0, 0.0),
        ));
        world.sync_pellet_hash();
        assert!(world.hash.contains(pellet.handle(), SpatialClass::Pellet));
        let culled = world.census_and_cull();
        assert_eq!(culled, 1);
        assert!(world.conservation_totals().culled_plant_energy > 0.0);
        assert!(!world.pellets.contains(pellet));
        assert!(!world.hash.contains(pellet.handle(), SpatialClass::Pellet));
    }

    #[test]
    fn full_tick_rad_tower_mutates_bibite() {
        use crate::tools::RadTower;
        let mut world = World::new(default_settings(), ScenarioData::default(), 13).expect("world");
        let id = spawn_test_bibite(&mut world, Vec2::ZERO);
        let mut tower = RadTower::new(1, Vec2::ZERO, 2000.0);
        tower.radius = 100.0;
        tower.count_rate = 1.0;
        tower.intensity_rate = 1.0;
        world.add_rad_tower(tower);
        world.run(40);
        let record = world.bibites.get(id).expect("bibite still alive");
        assert!(record.mutation_factor.probability_increase > 0.0);
        assert!(record.mutation_factor.intensity_increase > 0.0);
    }

    #[test]
    fn full_tick_color_killer_kills_matching_victim() {
        use crate::tools::ColorKiller;
        let mut world = World::new(default_settings(), ScenarioData::default(), 17).expect("world");
        let mut killer = ColorKiller::new(1, Vec2::ZERO);
        killer.hue = 0.0; // keep red band; kills non-red
        killer.tolerance = 0.1;
        killer.radius = 2000.0;
        killer.period = 0.1;
        world.add_color_killer(killer);
        let red = spawn_test_bibite(&mut world, Vec2::new(0.0, 10.0));
        {
            // Make it red so the killer spares it.
            let record = world.bibites.get_mut(red).unwrap();
            record.genome.set_gene(Gene::ColorR, 1.0);
            record.genome.set_gene(Gene::ColorG, 0.0);
            record.genome.set_gene(Gene::ColorB, 0.0);
            record.color = [1.0, 0.0, 0.0];
        }
        let blue = spawn_test_bibite(&mut world, Vec2::new(0.0, 20.0));
        {
            let record = world.bibites.get_mut(blue).unwrap();
            record.genome.set_gene(Gene::ColorR, 0.0);
            record.genome.set_gene(Gene::ColorG, 0.0);
            record.genome.set_gene(Gene::ColorB, 1.0);
            record.color = [0.0, 0.0, 1.0];
        }
        world.run(30);
        assert!(
            world
                .bibites
                .get(red)
                .is_some_and(|record| !record.body.dead),
            "red bibite should be inside the tolerance band"
        );
        assert!(
            world
                .bibites
                .get(blue)
                .is_none_or(|record| record.body.dead),
            "blue bibite should have been killed"
        );
        assert!(world.stats.deaths_total >= 1);
    }

    #[test]
    fn color_killer_candidates_carry_the_games_body_colour_not_the_raw_pigment_genes() {
        use crate::tools::ColorKiller;
        let mut world = World::new(default_settings(), ScenarioData::default(), 23).expect("world");
        let mut killer = ColorKiller::new(1, Vec2::ZERO);
        killer.radius = 2000.0;
        killer.period = 1000.0; // not due: only the colours handed to the scan matter here
        world.add_color_killer(killer);
        let basic = spawn_test_bibite(&mut world, Vec2::new(0.0, 10.0));
        {
            // The basic bibite: pigment genes 0.1 / 0.1 / 0.1.
            let record = world.bibites.get_mut(basic).unwrap();
            for gene in [Gene::ColorR, Gene::ColorG, Gene::ColorB] {
                record.genome.set_gene(gene, 0.1);
            }
            record.color = [0.1, 0.1, 0.1];
        }
        let pale_red = spawn_test_bibite(&mut world, Vec2::new(0.0, 20.0));
        {
            let record = world.bibites.get_mut(pale_red).unwrap();
            record.genome.set_gene(Gene::ColorR, 0.5);
            record.genome.set_gene(Gene::ColorG, 0.2);
            record.genome.set_gene(Gene::ColorB, 0.2);
            record.color = [0.5, 0.2, 0.2];
        }
        world.run(2);
        let colour_of = |id: BibiteId| {
            world
                .color_candidates
                .iter()
                .find(|candidate| candidate.id == id)
                .map(|candidate| candidate.color)
                .expect("the killer scanned the bibite")
        };
        let near = |actual: [f32; 3], expected: [f32; 3]| {
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < 1e-5)
        };
        let basic_colour = colour_of(basic);
        assert!(near(basic_colour, [0.9, 0.9, 0.9]), "{basic_colour:?}");
        let pale_colour = colour_of(pale_red);
        assert!(near(pale_colour, [0.8, 0.5, 0.5]), "{pale_colour:?}");
        // The vision cones keep reading the raw pigment genes.
        assert_eq!(world.bibites.view(basic).unwrap().color, [0.1, 0.1, 0.1]);
    }

    #[test]
    fn full_tick_red_death_damages_pellets_outside_safe_radius() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 19).expect("world");
        let pellet = world.pellets.spawn(crate::pellet::PelletSpawn::new(
            crate::pellet::MaterialKind::Plant,
            100.0,
            Vec2::new(5000.0, 0.0),
        ));
        world.sync_pellet_hash();
        world.red_death_mut().set_enabled(true);
        world.red_death_mut().set_fill(0.75); // safe radius 2000
        let before = world.pellets.get(pellet).map_or(0.0, |view| view.amount);
        world.run(20);
        let after = world.pellets.get(pellet).map_or(0.0, |view| view.amount);
        assert!(after < before, "bloom should remove pellet amount");
    }

    #[test]
    fn checkpoint_round_trip_preserves_populated_world() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::settings::{BibiteSpawn as SettingsSpawn, SpawnType};

        let mut source =
            World::new(default_settings(), scenario_with_zone_group(), 74).expect("world");
        source.seed_scenario_world();
        source.set_templates(vec![TemplateSource::new(
            "starter",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        )]);
        let row = SettingsSpawn {
            path: "starter".to_owned(),
            minimum: 4,
            spawn_type: SpawnType::Continuous,
            ..SettingsSpawn::game_defaults()
        };
        source.bibite_spawns = vec![row];
        source.start_fresh_scenario_spawner();
        source.run(120);
        source.forget_contact_impulses();
        let snapshot = source.capture_checkpoint_state().expect("capture");
        let bytes = snapshot.encode().expect("encode");
        let decoded = crate::sim::SimSnapshot::decode(&bytes).expect("decode");
        let mut restored =
            World::new(default_settings(), scenario_with_zone_group(), 74).expect("world");
        restored.seed_scenario_world();
        restored.bibite_spawns.clone_from(&source.bibite_spawns);
        restored
            .restore_checkpoint_state(&decoded)
            .expect("restore");
        assert_eq!(restored.bibites.len(), source.bibites.len());
        assert_eq!(restored.eggs.len(), source.eggs.len());
        assert_eq!(
            crate::sim::state_fingerprint(&restored),
            crate::sim::state_fingerprint(&source)
        );
        // The next ticks must stay bit-identical (RNG/arena/timers/caches).
        source.run(10);
        restored.run(10);
        assert_eq!(
            crate::sim::state_fingerprint(&restored),
            crate::sim::state_fingerprint(&source)
        );
        assert_eq!(
            restored.bibites.capture_checkpoint(),
            source.bibites.capture_checkpoint()
        );
        // (A restored world starts with every pellet awake: the pellets of the source that rest have stopped turning by rounding noise, the restored
        // ones turn by it until they rest too, so their rotations are put back to zero.)
        for world in [&mut source, &mut restored] {
            let ids: Vec<PelletId> = world.pellets.ids().collect();
            for id in ids {
                world.pellets.set_rotation(id, 0.0);
            }
        }
        assert_eq!(
            restored.pellets.capture_checkpoint(),
            source.pellets.capture_checkpoint()
        );
        assert_eq!(restored.rng.state(), source.rng.state());
    }

    #[test]
    fn checkpoint_chunked_resume_matches_single_run() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::ids::SpeciesId;
        use crate::settings::{BibiteSpawn as SettingsSpawn, SpawnType};

        // Spawner-focused world: no zones/pellets/collisions, so a split during
        // virgin spawning exercises spawner active-state restore, not physics.
        fn fresh(seed: u64) -> World {
            let mut world =
                World::new(default_settings(), ScenarioData::default(), seed).expect("world");
            world.set_templates(vec![TemplateSource::new(
                "starter",
                Genome::starter(),
                GenomeMeta {
                    species: Some(SpeciesId::new(1)),
                    ..GenomeMeta::default()
                },
                Brain::starter(),
            )]);
            world.bibite_spawns = vec![SettingsSpawn {
                path: "starter".to_owned(),
                minimum: 2,
                spawn_type: SpawnType::Continuous,
                ..SettingsSpawn::game_defaults()
            }];
            world.start_fresh_scenario_spawner();
            world
        }

        for seed in [91u64, 92] {
            let mut single = fresh(seed);
            single.run(30);
            let expected = crate::sim::state_fingerprint(&single);
            for chunks in [2usize] {
                let mut chunked = fresh(seed);
                let ticks_per_chunk = 30 / chunks;
                for _ in 0..chunks {
                    chunked.run(ticks_per_chunk as u64);
                    let snapshot = chunked.capture_checkpoint_state().expect("capture");
                    let bytes = snapshot.encode().expect("encode");
                    let decoded = crate::sim::SimSnapshot::decode(&bytes).expect("decode");
                    let mut next = fresh(seed);
                    next.restore_checkpoint_state(&decoded).expect("restore");
                    assert!(
                        next.fresh_scenario_spawner_started(),
                        "restore must re-activate the fresh spawner"
                    );
                    chunked = next;
                }
                assert_eq!(
                    crate::sim::state_fingerprint(&chunked),
                    expected,
                    "seed {seed}, {chunks} chunks diverged"
                );
            }
        }
    }

    #[test]
    #[ignore = "long Default-like chunked resume (10k ticks)"]
    fn checkpoint_long_chunked_resume_matches_single_run() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::settings::{BibiteSpawn as SettingsSpawn, SpawnType};

        fn fresh(seed: u64) -> World {
            let mut world =
                World::new(default_settings(), scenario_with_zone_group(), seed).expect("world");
            world.seed_scenario_world();
            world.set_templates(vec![TemplateSource::new(
                "starter",
                Genome::starter(),
                GenomeMeta::default(),
                Brain::starter(),
            )]);
            world.bibite_spawns = vec![SettingsSpawn {
                path: "starter".to_owned(),
                minimum: 10,
                spawn_type: SpawnType::Continuous,
                ..SettingsSpawn::game_defaults()
            }];
            world.start_fresh_scenario_spawner();
            world
        }

        let mut single = fresh(123);
        single.run(10_000);
        let expected = crate::sim::state_fingerprint(&single);
        let mut chunked = fresh(123);
        for _ in 0..5 {
            chunked.run(2000);
            let snapshot = chunked.capture_checkpoint_state().expect("capture");
            let bytes = snapshot.encode().expect("encode");
            let decoded = crate::sim::SimSnapshot::decode(&bytes).expect("decode");
            let mut next = fresh(123);
            next.restore_checkpoint_state(&decoded).expect("restore");
            chunked = next;
        }
        assert_eq!(
            crate::sim::state_fingerprint(&chunked),
            expected,
            "long chunked resume must be bit-identical"
        );
    }

    #[test]
    #[ignore = "very long chunked resume (100k ticks); run manually"]
    fn checkpoint_100k_chunked_resume_matches_single_run() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::settings::{BibiteSpawn as SettingsSpawn, SpawnType};

        fn fresh(seed: u64) -> World {
            let mut world =
                World::new(default_settings(), scenario_with_zone_group(), seed).expect("world");
            world.seed_scenario_world();
            world.set_templates(vec![TemplateSource::new(
                "starter",
                Genome::starter(),
                GenomeMeta::default(),
                Brain::starter(),
            )]);
            world.bibite_spawns = vec![SettingsSpawn {
                path: "starter".to_owned(),
                minimum: 10,
                spawn_type: SpawnType::Continuous,
                ..SettingsSpawn::game_defaults()
            }];
            world.start_fresh_scenario_spawner();
            world
        }

        let mut single = fresh(123);
        single.run(100_000);
        let expected = crate::sim::state_fingerprint(&single);
        let mut chunked = fresh(123);
        for _ in 0..50 {
            chunked.run(2000);
            let snapshot = chunked.capture_checkpoint_state().expect("capture");
            let bytes = snapshot.encode().expect("encode");
            let decoded = crate::sim::SimSnapshot::decode(&bytes).expect("decode");
            let mut next = fresh(123);
            next.restore_checkpoint_state(&decoded).expect("restore");
            chunked = next;
        }
        assert_eq!(
            crate::sim::state_fingerprint(&chunked),
            expected,
            "100k chunked resume must be bit-identical"
        );
    }

    #[test]
    fn ui_spawn_pellet_pile_registers_hash_and_ledger() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 17).expect("world");
        let before = world.pellets.counts();
        let ids =
            world.spawn_pellet_pile(MaterialKind::Plant, Vec2::new(10.0, 20.0), 5, 100.0, 50.0);
        assert_eq!(ids.len(), 5);
        let after = world.pellets.counts();
        assert_eq!(after.plant_count, before.plant_count + 5);
        for id in &ids {
            assert!(world.hash.contains(id.handle(), SpatialClass::Pellet));
        }
        assert!(world.conservation_totals().produced_plant_energy > 0.0);
    }

    #[test]
    fn ui_place_and_remove_tools() {
        use crate::tools::Tool;
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 18).expect("world");
        let zone = world.zones.zones[0].id;
        let rad = world.place_rad_tower_at(Vec2::new(50.0, 50.0), Some(zone));
        assert_eq!(world.tools.len(), 1);
        match world.tools.get(rad) {
            Some(Tool::Rad(tower)) => {
                assert_eq!(tower.base.attached_zone, Some(zone));
                assert_eq!(tower.base.pos, world.zones.zones[0].pos);
            }
            _ => panic!("rad tower missing"),
        }
        let phero = world.place_phero_tower_at(Vec2::new(10.0, 10.0), None);
        let spot = match world.tools.get(phero) {
            Some(Tool::Phero(tower)) => tower.spot,
            _ => panic!("phero tower missing"),
        };
        assert!(spot.is_some());
        assert!(
            world
                .hash
                .contains(spot.unwrap().handle(), SpatialClass::Pheromone)
        );
        assert!(world.remove_tool(phero));
        assert!(!world.pheromones.contains(spot.unwrap()));
        assert_eq!(world.tools.len(), 1);
        let _ = world.place_color_killer_at(Vec2::ZERO, Some(zone));
        assert_eq!(world.tools.len(), 2);
    }

    #[test]
    fn ui_kill_and_abort_actions() {
        use crate::genome::{Genome, GenomeMeta};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 19).expect("world");
        let id = spawn_test_bibite(&mut world, Vec2::ZERO);
        assert!(world.kill_bibite(id));
        assert!(world.bibites.get(id).expect("record").body.dying);
        assert!(!world.kill_bibite(id));
        let egg = world.eggs.spawn(EggSpawn {
            pos: Vec2::new(25.0, 25.0),
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain: Brain::starter(),
            energy: 50.0,
            hatch_time: 100.0,
            total_required_energy: 1.0,
            birth_orientation: -Vec2::Y,
        });
        assert!(world.abort_egg(egg));
        assert!(world.eggs.get(egg).expect("egg").hatch.aborting);
        assert!(!world.abort_egg(egg));
    }

    #[test]
    fn removing_half_of_the_pellets_removes_about_half_and_recounts() {
        use crate::pellet::{MaterialKind, PelletSpawn};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 22).expect("world");
        let before = world.pellets.ids().count();
        for index in 0..400 {
            let pos = Vec2::new(index as f32 * 0.5, 3.0);
            let _ = world.spawn_pellet(PelletSpawn::new(MaterialKind::Plant, 5.0, pos));
        }
        let total = world.pellets.ids().count();
        assert!(total >= before + 300, "pellets spawned: {total}");
        let removed = world.remove_random_half_pellets();
        let left = world.pellets.ids().count();
        assert_eq!(total - removed, left);
        let share = removed as f32 / total as f32;
        assert!((0.35..0.65).contains(&share), "removed {share}");
    }

    #[test]
    fn aborting_a_hatching_egg_removes_it_on_the_next_tick() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 21).expect("world");
        let parent = spawn_test_bibite(&mut world, Vec2::ZERO);
        assert_eq!(world.lay_egg_once(parent), 1);
        let egg = world.eggs.ids().next().expect("a laid egg");
        assert!(
            world.eggs.get(egg).expect("egg").hatch.is_hatching,
            "laid eggs incubate"
        );
        assert!(world.abort_egg(egg));
        world.tick();
        assert!(world.eggs.get(egg).is_none(), "the aborted egg is gone");
        assert!(world.take_hatched_children().is_empty());
    }

    /// `BibiteBody.Born`: a bibite born with a brain of 100 hidden nodes or more is counted (`ACH_BIBITE_EINSTEIN`); an ordinary
    /// newborn is not.
    #[test]
    fn a_newborn_with_a_hundred_hidden_nodes_is_counted_once() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 22).expect("world");
        let parent = spawn_test_bibite(&mut world, Vec2::ZERO);
        let hatch = |world: &mut World, brain: Option<crate::brain::Brain>| -> bool {
            assert_eq!(world.lay_egg_once(parent), 1);
            let egg = world.eggs.ids().last().expect("a laid egg");
            if let Some(brain) = brain {
                world.eggs.get_mut(egg).expect("egg").brain = brain;
            }
            for _ in 0..30_000 {
                world.tick();
                if world.eggs.get(egg).is_none() {
                    return !world.take_hatched_children().is_empty();
                }
            }
            false
        };
        assert!(hatch(&mut world, None), "an egg hatches");
        assert_eq!(world.take_einstein_births(), 0, "an ordinary brain is not counted");
        let mut big = crate::brain::Brain::starter();
        while big.hidden_count() < 100 {
            big.add_hidden_node(crate::brain::NodeType::Linear, None)
                .expect("a hidden node");
        }
        assert!(hatch(&mut world, Some(big)), "the second egg hatches");
        assert_eq!(world.take_einstein_births(), 1);
        assert_eq!(world.take_einstein_births(), 0, "the count is taken");
    }

    #[test]
    fn ui_apply_settings_safe_fields_and_reject_structural() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 20).expect("world");
        let mut settings = world.settings.clone();
        assert!(
            settings
                .independents
                .set_raw("biomassDensity", SettingValue::Float(0.02))
                .is_some()
        );
        world.apply_settings(settings).expect("safe settings");
        assert!(world.zones.globals.global_biomass_density() > 0.015);
        let mut red = world.settings.clone();
        assert!(
            red.scenario
                .set_raw("enableRedDeath", SettingValue::Bool(true))
                .is_some()
        );
        world.apply_settings(red).expect("red death apply");
        assert!(world.red_death().enabled);
        let mut bad = world.settings.clone();
        assert!(
            bad.independents
                .set_raw("SimulationSize", SettingValue::Float(3000.0))
                .is_some()
        );
        assert!(world.apply_settings(bad).is_err());
        // The ticks per second change while the world runs (the game's `simTPS` subscribers): the clock follows.
        let mut tps = world.settings.clone();
        assert!(
            tps.independents
                .set_raw("simTPS", SettingValue::Int(60))
                .is_some()
        );
        world.apply_settings(tps).expect("simTPS apply");
        assert_eq!((world.clock.tps(), world.settings.sim_tps), (60, 60));
        assert_eq!(world.clock.dt(), world.settings.fixed_dt());
    }

    /// `GlobalLineageManager.Awake` subscribes `UpdateEffectiveSpeciesSpan` to the `OnChange` of `speciesGeneticSpan` and of
    /// `speciesSpanScalingFactor`: an edit of either recounts the active species, so the species that died out since the span was last read (the
    /// species log drops them without recounting) stop being counted. An edit of any other setting does not.
    #[test]
    fn editing_a_span_setting_recounts_the_active_species() {
        let stale_world = || {
            let mut world = World::new(default_settings(), ScenarioData::default(), 21).expect("world");
            let (genome, brain) = (Genome::starter(), Brain::starter());
            let species: Vec<_> = (0..25)
                .map(|n| world.species.add_template_species(&format!("Zz{} zzspecies", "a".repeat(n + 1)), "", &genome, &brain))
                .collect();
            // 22 of the 25 have a member; the log (five of them) drops the three that have none, and does not recount.
            world.species.reconcile_counts_by_species(species.iter().take(22).map(|id| (*id, 1_u32)));
            for _ in 0..5 {
                world.species.log_stats_with(&[]);
            }
            assert_eq!(world.species.active_species_count(), 22);
            world
        };
        let span = |world: &World| world.species.current_used_span(&world.settings);
        let expected = |world: &World, species: f32| {
            world.settings.independents.species_genetic_span + species * world.settings.independents.species_span_scaling_factor
        };
        let mut world = stale_world();
        assert!((span(&world) - expected(&world, 5.0)).abs() < 1e-6, "still counting the 25 it was last computed with");
        let mut other = world.settings.clone();
        assert!(other.independents.set_raw("biomassDensity", SettingValue::Float(0.02)).is_some());
        world.apply_settings(other).expect("another setting");
        assert!((span(&world) - expected(&world, 5.0)).abs() < 1e-6, "an edit of another setting does not recount");
        for (key, value) in [("speciesGeneticSpan", 2.0_f32), ("speciesSpanScalingFactor", 0.1)] {
            let mut world = stale_world();
            let mut edited = world.settings.clone();
            assert!(edited.independents.set_raw(key, SettingValue::Float(value)).is_some(), "{key}");
            world.apply_settings(edited).expect("the span setting applies");
            assert!((span(&world) - expected(&world, 2.0)).abs() < 1e-6, "{key}: an edit recounts the 22 active species");
        }
        // The same value written again is no edit: the count it holds stays.
        let mut world = stale_world();
        let same = world.settings.clone();
        world.apply_settings(same).expect("the same settings");
        assert!((span(&world) - expected(&world, 5.0)).abs() < 1e-6, "no setting changed: no recount");
    }

    #[test]
    fn the_ticks_per_second_change_while_the_world_runs() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 42).expect("world");
        let id = spawn_test_bibite(&mut world, Vec2::new(10.0, 10.0));
        for _ in 0..30 {
            world.tick();
        }
        let (tick, time) = (world.clock.tick(), world.simulated_time());
        let age = world.bibites.get(id).expect("record").clock.time_alive;
        assert_eq!(world.clock.tps(), 30);
        // 60 ticks a second from here: the in-game panel's "Ticks Per Second", edited in the middle of a simulation.
        world.set_sim_tps(60).expect("rate change");
        assert_eq!((world.clock.tps(), world.settings.sim_tps, world.settings.independents.sim_tps), (60, 60, 60));
        assert_eq!(world.clock.dt(), 1.0 / 60.0);
        assert_eq!(world.settings.fixed_dt(), 1.0 / 60.0);
        // The change itself moves nothing: the tick count and the time are where they were.
        assert_eq!((world.clock.tick(), world.simulated_time()), (tick, time));
        for _ in 0..60 {
            world.tick();
        }
        // 60 ticks of 1/60 s are one more second, for the world's time, the clock's and the bibite's age alike.
        assert_eq!(world.clock.tick(), tick + 60);
        assert!((world.simulated_time() - (time + 1.0)).abs() < 1e-4, "{} after {time}", world.simulated_time());
        assert!((world.clock.time_secs() - world.simulated_time()).abs() < 1e-4);
        let aged = world.bibites.get(id).expect("record").clock.time_alive - age;
        assert!((aged - 1.0).abs() < 1e-3, "the bibite aged {aged} s in a simulated second");
        // And back to 15: 15 ticks make the next second.
        world.set_sim_tps(15).expect("rate change back");
        let time = world.simulated_time();
        for _ in 0..15 {
            world.tick();
        }
        assert!((world.simulated_time() - (time + 1.0)).abs() < 1e-4);
        assert_eq!(world.clock.tps(), 15);
    }

    #[test]
    fn a_tool_that_writes_the_rate_itself_is_followed_at_the_next_tick() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 43).expect("world");
        world.settings.sim_tps = 45;
        assert_eq!(world.clock.tps(), 30, "nothing has ticked yet");
        world.tick();
        assert_eq!(world.clock.tps(), 45);
        assert!((world.simulated_time() - 1.0 / 45.0).abs() < 1e-6, "{}", world.simulated_time());
    }

    #[test]
    fn the_other_performance_factors_change_while_the_world_runs_too() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 44).expect("world");
        let id = spawn_test_bibite(&mut world, Vec2::new(-20.0, 5.0));
        for _ in 0..10 {
            world.tick();
        }
        let mut settings = world.settings.clone();
        for (key, value) in [("simTPS", 40), ("brainTPS", 5), ("decayTPS", 3), ("visionLookupUpdateFactor", 4), ("visionSenseUpdateFactor", 3)] {
            assert!(settings.independents.set_raw(key, SettingValue::Int(value)).is_some(), "{key}");
        }
        world.apply_settings(settings).expect("the factors apply");
        assert_eq!(
            (world.clock.tps(), world.settings.brain_tps, world.decay_tps, world.settings.vision_lookup_updates, world.settings.vision_sense_updates),
            (40, 5, 3, 4, 3)
        );
        // The brains' period is `brain_tps / sim_tps` seconds: 5 ticks of 1/40 s.
        assert_eq!(world.settings.brain_period(), 5.0 / 40.0);
        for _ in 0..80 {
            world.tick();
        }
        assert!(world.bibites.get(id).is_some(), "the bibite goes on living");
        assert_eq!(world.clock.tick(), 90);
    }

    #[test]
    fn any_rate_is_taken_but_never_less_than_one() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 45).expect("world");
        // The game's setter does not clamp to the slider's 5 to 100.
        world.set_sim_tps(240).expect("240");
        assert_eq!((world.clock.tps(), world.settings.sim_tps), (240, 240));
        world.tick();
        // Zero would make an infinite tick: it is taken as 1.
        world.set_sim_tps(0).expect("0");
        assert_eq!((world.clock.tps(), world.settings.sim_tps), (1, 1));
        world.tick();
        assert!((world.clock.dt() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ui_spawner_row_actions_validate_and_append() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 21).expect("world");
        let before = world.bibite_spawns.len();
        let row = crate::settings::BibiteSpawn::game_defaults();
        world.add_bibite_spawn(row).expect("add row");
        assert_eq!(world.bibite_spawns.len(), before + 1);
        world.set_bibite_spawns(Vec::new()).expect("clear rows");
        assert!(world.bibite_spawns.is_empty());
    }

    #[test]
    fn ui_tool_attach_and_settings_changer_actions() {
        use crate::settings::{SettingBase, SettingsChanger, SplinePoint};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 22).expect("world");
        let zone = world.zones.zones[1].id;
        let id = world.place_rad_tower_at(Vec2::new(5.0, 5.0), None);
        assert!(world.set_tool_attached_zone(id, Some(zone)));
        assert_eq!(
            world.tools.get(id).expect("tool").base().attached_zone,
            Some(zone)
        );
        assert!(world.set_tool_attached_zone(id, None));
        let mut changer = SettingsChanger::game_defaults();
        changer.name = "ui".to_owned();
        changer.points = vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(10.0, 2.0)];
        changer.settings_bases = vec![SettingBase {
            key: "biomassDensity".to_owned(),
            base: 0.0075,
        }];
        let index = world.add_settings_changer(changer).expect("add changer");
        assert_eq!(world.settings_changers.len(), 1);
        assert!(world.remove_settings_changer(index).is_some());
        assert!(world.settings_changers.is_empty());
        assert!(
            world
                .add_settings_changer(SettingsChanger::game_defaults())
                .is_err()
        );
    }

    #[test]
    fn viewer_placement_flow_integration() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 23).expect("world");
        world.set_templates(vec![TemplateSource::new(
            "starter",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        )]);
        let zone = world.zones.zones[0].id;
        let before = world.pellets.counts();
        let pile =
            world.spawn_pellet_pile(MaterialKind::Plant, Vec2::new(100.0, 50.0), 6, 120.0, 40.0);
        assert_eq!(pile.len(), 6);
        assert_eq!(world.pellets.counts().plant_count, before.plant_count + 6);
        assert!(world.conservation_totals().produced_plant_energy >= 6.0 * 90.0);
        let rad = world.place_rad_tower_at(Vec2::new(20.0, 20.0), Some(zone));
        let phero = world.place_phero_tower_at(Vec2::new(30.0, 30.0), Some(zone));
        let killer = world.place_color_killer_at(Vec2::new(40.0, 40.0), Some(zone));
        assert_eq!(world.tools.len(), 3);
        assert!(world.set_tool_attached_zone(rad, Some(zone)));
        assert!(world.set_tool_attached_zone(killer, None));
        assert!(world.remove_tool(phero));
        assert_eq!(world.tools.len(), 2);
        let bibite = world
            .spawn_bibite_by_template_path(
                "starter",
                GrowthAtSpawn::Adult,
                Tagging::NoTagging,
                "",
                Vec2::new(10.0, 20.0),
            )
            .expect("spawn template");
        assert!(world.bibites.get(bibite).is_some());
        assert!(world.kill_bibite(bibite));
        let mut zone_settings = world.zones.zones[0].config.0.clone();
        zone_settings.fertility = 2.0;
        world
            .apply_zone_settings(zone, zone_settings)
            .expect("apply zone");
        assert!((world.zones.zones[0].config.0.fertility - 2.0).abs() < 1e-6);
        let mut settings = world.settings.clone();
        settings
            .independents
            .set_raw("biomassDensity", SettingValue::Float(0.02))
            .expect("setting key");
        world.apply_settings(settings).expect("apply settings");
    }

    #[test]
    fn a_template_is_found_from_an_absolute_path_with_either_separator() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 25).expect("world");
        world.set_templates(vec![TemplateSource::new(
            "Accept.bb8template",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        )]);
        // A champion row of the game keeps the absolute path of the player's own machine (`BibiteSettings.fullPath`).
        for path in [
            "Accept.bb8template",
            "Accept",
            "C:\\Users\\me\\AppData\\LocalLow\\The Bibites\\The Bibites\\Bibites\\Templates\\Accept.bb8template",
            "/home/me/Bibites/Templates/Accept.bb8template",
        ] {
            assert!(
                world
                    .spawn_bibite_by_template_path(
                        path,
                        GrowthAtSpawn::Adult,
                        Tagging::NoTagging,
                        "",
                        Vec2::ZERO,
                    )
                    .is_ok(),
                "{path}"
            );
        }
        assert!(
            world
                .spawn_bibite_by_template_path(
                    "C:\\somewhere\\Other.bb8template",
                    GrowthAtSpawn::Adult,
                    Tagging::NoTagging,
                    "",
                    Vec2::ZERO,
                )
                .is_err()
        );
    }

    #[test]
    fn ui_typed_tool_property_setters_apply_and_reject() {
        use crate::tools::Tool;
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 24).expect("world");
        let rad = world.place_rad_tower_at(Vec2::ZERO, None);
        let sim_size = world.bounds.sim_size();
        assert!(world.set_rad_tower_params(rad, 0.01, 0.02, 1.0e9, 2.0));
        match world.tools.get(rad) {
            Some(Tool::Rad(tower)) => {
                assert!((tower.radius - sim_size / 2.0).abs() < 1e-3);
                assert_eq!(tower.taper, 2.0);
            }
            _ => panic!("rad missing"),
        }
        assert!(!world.set_rad_tower_params(rad, f32::NAN, 0.0, 100.0, 1.0));
        let phero = world.place_phero_tower_at(Vec2::ZERO, None);
        assert!(world.set_phero_tower_params(phero, [1.0, 2.0, 3.0], 0.5));
        match world.tools.get(phero) {
            Some(Tool::Phero(tower)) => {
                assert_eq!(tower.strength(), [1.0, 2.0, 3.0]);
                assert_eq!(tower.heading, 0.5);
            }
            _ => panic!("phero missing"),
        }
        assert!(!world.set_phero_tower_params(phero, [f32::NAN, 0.0, 0.0], 0.0));
        let killer = world.place_color_killer_at(Vec2::ZERO, None);
        assert!(world.set_color_killer_params(killer, 1.25, 0.1, true, true, 500.0, 10.0));
        match world.tools.get(killer) {
            Some(Tool::Killer(k)) => {
                assert!((k.hue - 0.25).abs() < 1e-6);
                assert_eq!(k.radius, 500.0);
                assert_eq!(k.period, 10.0);
            }
            _ => panic!("killer missing"),
        }
        assert!(!world.set_color_killer_params(killer, 0.0, 0.0, false, false, -1.0, 10.0));
        assert!(!world.set_color_killer_params(killer, 0.0, 0.0, false, false, 100.0, 0.0));
        let before = world.pellets.counts();
        let pellet = world
            .spawn_pellet_with_amount(MaterialKind::Plant, Vec2::new(1.0, 2.0), 40.5)
            .expect("amount spawn");
        assert_eq!(world.pellets.counts().plant_count, before.plant_count + 1);
        assert!(world.hash.contains(pellet.handle(), SpatialClass::Pellet));
        assert!(
            world
                .spawn_pellet_with_amount(MaterialKind::Plant, Vec2::ZERO, 0.0)
                .is_none()
        );
    }

    #[test]
    fn a_tool_moves_unless_it_follows_a_zone() {
        use crate::tools::Tool;
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 26).expect("world");
        let phero = world.place_phero_tower_at(Vec2::ZERO, None);
        assert!(world.set_tool_position(phero, Vec2::new(30.0, 40.0)));
        let Some(Tool::Phero(tower)) = world.tools.get(phero) else {
            panic!("phero tower missing");
        };
        assert_eq!(tower.base.pos, Vec2::new(30.0, 40.0));
        // The persistent spot moved with the tower.
        let spot = tower.spot.expect("spot");
        assert_eq!(world.pheromones.pos(spot), Some(Vec2::new(30.0, 40.0)));
        assert!(!world.set_tool_position(phero, Vec2::new(f32::NAN, 0.0)));
        // A tool that follows a zone is put back on it every tick, so it cannot be moved.
        let zone = world.zones.zones[0].id;
        let rad = world.place_rad_tower_at(Vec2::new(5.0, 5.0), Some(zone));
        let at = world.tools.get(rad).map(|tool| tool.base().pos);
        assert!(!world.set_tool_position(rad, Vec2::new(500.0, 500.0)));
        assert_eq!(world.tools.get(rad).map(|tool| tool.base().pos), at);
        assert!(world.set_tool_attached_zone(rad, None));
        assert!(world.set_tool_position(rad, Vec2::new(500.0, 500.0)));
    }

    #[test]
    fn ui_zone_add_remove_and_persistence() {
        use crate::tools::Tool;
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 25).expect("world");
        let before = world.zones.zones.len();
        let mut settings = world.zones.zones[0].config.0.clone();
        settings.zone_id = Some(999);
        settings.pos_x = 0.2;
        settings.pos_y = -0.2;
        settings.name = "live".to_owned();
        let id = world.add_zone(settings.clone()).expect("add zone");
        assert_eq!(world.zones.zones.len(), before + 1);
        // An explicit zone goes in front of the zones of the group.
        assert_eq!(world.zones.index_of(id), Some(0));
        assert_eq!(world.zone_at(world.zones.zones[0].pos), Some(id));
        let mut spawn = PelletSpawn::new(MaterialKind::Plant, 30.0, world.zones.zones[0].pos);
        spawn.owner_zone = Some(id);
        let pellet = world.spawn_pellet(spawn).expect("pellet");
        let rad = world.place_rad_tower_at(Vec2::ZERO, Some(id));
        let destroyed = world.remove_zone(id).expect("remove zone");
        assert_eq!(destroyed, 1);
        assert!(!world.pellets.contains(pellet));
        assert_eq!(world.zones.zones.len(), before);
        match world.tools.get(rad) {
            Some(Tool::Rad(tower)) => assert_eq!(tower.base.attached_zone, None),
            _ => panic!("rad missing"),
        }
        let contents = world.to_contents();
        let loaded = World::from_contents(contents).expect("persist");
        assert_eq!(loaded.zones.zones.len(), before);
        assert_eq!(
            loaded.settings.independents.biomass_density,
            world.settings.independents.biomass_density
        );
    }

    #[test]
    fn removing_a_zone_keeps_spawner_rows_on_their_zone() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 27).expect("world");
        let base = world.zones.zones.len() as u32;
        let template = world.zones.zones[0].config.0.clone();
        // The explicit zones are numbered before the zones of the group: they are zones 0, 1 and 2.
        for _ in 0..3 {
            world.add_zone(template.clone()).expect("add zone");
        }
        let mut rows = vec![SpawnRow::game_defaults(); 3];
        rows[0].spawn_zone = Some(0); // the zone that goes
        rows[1].spawn_zone = Some(2); // a later zone: its id drops by one
        rows[2].spawn_zone = None; // all zones
        world.set_bibite_spawns(rows).expect("rows");
        let first = world.zones.zone_id_at(0).expect("zone 0");
        world.remove_zone(first).expect("remove zone");
        let zones: Vec<Option<u32>> = world.bibite_spawns.iter().map(|r| r.spawn_zone).collect();
        assert_eq!(zones, vec![None, Some(1), None]);
        // The row still names the zone it named: the last one.
        assert_eq!(world.zones.zones.len() as u32, base + 2);
    }

    #[test]
    fn editing_a_zone_to_follow_another_one_takes_effect() {
        use crate::settings::MovementType;
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 28).expect("world");
        assert!(world.zones.zones.len() >= 2);
        let id = world.zones.zones[1].id;
        let mut settings = world.zones.zones[1].config.0.clone();
        settings.movement = MovementType::Attached;
        settings.target = Some(0);
        world.apply_zone_settings(id, settings).expect("apply");
        assert_eq!(world.zones.zones[1].target_index, Some(0));
        // No movement again: the zone stops following.
        let mut settings = world.zones.zones[1].config.0.clone();
        settings.movement = MovementType::None;
        settings.target = None;
        world.apply_zone_settings(id, settings).expect("apply");
        assert_eq!(world.zones.zones[1].target_index, None);
    }

    #[test]
    fn editing_a_zone_position_moves_it_and_the_zones_attached_to_it() {
        use crate::settings::MovementType;
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 29).expect("world");
        assert!(world.zones.zones.len() >= 3);
        let size = world.settings.independents.simulation_size;
        // Zone 2 follows zone 1, which stays where it is unless moved by hand.
        let id1 = world.zones.zones[1].id;
        let mut leader = world.zones.zones[1].config.0.clone();
        leader.movement = MovementType::None;
        world.apply_zone_settings(id1, leader).expect("leader");
        let id2 = world.zones.zones[2].id;
        let mut follower = world.zones.zones[2].config.0.clone();
        follower.movement = MovementType::Attached;
        follower.target = Some(1);
        world.apply_zone_settings(id2, follower).expect("attach");
        let before1 = world.zones.zones[1].pos;
        let before2 = world.zones.zones[2].pos;
        // The leader's position settings change: it is there at once.
        let mut leader = world.zones.zones[1].config.0.clone();
        leader.pos_x = 0.1;
        leader.pos_y = -0.1;
        world.apply_zone_settings(id1, leader).expect("move");
        let after1 = world.zones.zones[1].pos;
        let (x, y) = (
            world.zones.zones[1].config.pos_x,
            world.zones.zones[1].config.pos_y,
        );
        assert!((after1.x - x * size).abs() < 1e-3 && (after1.y - y * size).abs() < 1e-3);
        assert!((x - 0.1).abs() < 1e-6 && (y + 0.1).abs() < 1e-6, "{x} {y}");
        // The follower travelled the same way and its position setting says so.
        let after2 = world.zones.zones[2].pos;
        assert!(((after2 - before2) - (after1 - before1)).length() < 1e-2);
        assert!((world.zones.zones[2].config.pos_x - after2.x / size).abs() < 1e-5);
        // The edit sticks while the world runs (the tick writes the position back to the setting).
        world.tick();
        assert!((world.zones.zones[1].config.pos_x - 0.1).abs() < 1e-5);
        assert!((world.zones.zones[1].pos.x - 0.1 * size).abs() < 1e-2);
    }

    #[test]
    fn ui_scenario_settings_snapshot_reflects_edits() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 26).expect("world");
        world
            .set_independent_setting("biomassDensity", SettingValue::Float(0.02))
            .expect("density");
        let mut zone = world.zones.zones[0].config.0.clone();
        zone.fertility = 3.0;
        world
            .apply_zone_settings(world.zones.zones[0].id, zone)
            .expect("zone");
        let snap = world.scenario_settings_snapshot();
        assert_eq!(snap.settings.independents.biomass_density, 0.02);
        assert_eq!(snap.zones.len(), world.zones.zones.len());
        assert_eq!(snap.zones[0].fertility, 3.0);
        assert_eq!(snap.spawn_rows.len(), world.bibite_spawns.len());
        assert_eq!(snap.settings_changers.len(), world.settings_changers.len());
    }

    #[test]
    fn new_fresh_scenario_seeds_and_starts_spawner() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let template = TemplateSource::new(
            "starter",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        );
        let mut scenario = scenario_with_zone_group();
        let mut row = crate::settings::BibiteSpawn::game_defaults();
        row.path = "starter".to_owned();
        row.minimum = 10;
        scenario.bibite_spawns = vec![row];
        let world = World::new_fresh_scenario(default_settings(), scenario, 27, vec![template])
            .expect("fresh scenario");
        assert_eq!(world.templates.len(), 1);
        assert!(world.fresh_scenario_spawner_started());
        assert!(!world.bibites.is_empty());
        assert!(!world.zones.zones.is_empty());
    }

    #[test]
    fn template_node_anchors_round_trip_through_template_source() {
        use std::collections::BTreeMap;
        let mut anchors = BTreeMap::new();
        anchors.insert(42_i64, (12.5, -3.0));
        let template = TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        )
        .with_node_anchors(anchors);
        assert_eq!(template.node_anchor(42), Some((12.5, -3.0)));
        assert_eq!(template.node_anchor(7), None);
    }

    #[test]
    fn challenge_session_scores_stars_and_ends() {
        use crate::settings::{
            Comparator, ConditionGroup, SimCondition, SimMetric, SimulationMetric,
        };
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 28).expect("world");
        let mut params = ChallengeParameters::game_defaults();
        params.exit_condition = ConditionGroup::single(SimCondition::new(
            SimMetric::Time,
            "",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "1",
        ));
        params.scoring_metric = SimulationMetric::new(SimMetric::TagCount, "Champion");
        params.one_star_condition = ConditionGroup::single(SimCondition::new(
            SimMetric::TagCount,
            "Champion",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "1",
        ));
        params.two_star_condition = ConditionGroup::single(SimCondition::new(
            SimMetric::TagCount,
            "Champion",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "2",
        ));
        params.three_star_condition = ConditionGroup::single(SimCondition::new(
            SimMetric::TagCount,
            "Champion",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "3",
        ));
        let id = spawn_test_bibite(&mut world, Vec2::ZERO);
        world.bibites.get_mut(id).expect("record").meta.species_tag = Some("Champion".to_owned());
        world.simulated_time = 1.0;
        assert_eq!(world.challenge_metric(&params.scoring_metric), 1.0);
        assert_eq!(world.challenge_stars(&params), 1);
        let mut session = ChallengeSession::new(params, "test", "desc", "champ", "/tmp/champ", 0.0);
        assert!(session.tick(&world, 1.0));
        assert_eq!(session.state, ChallengeState::Ended);
        assert_eq!(session.outcome, ChallengeOutcome::Completed);
        assert_eq!(session.stars, 1);
        assert_eq!(session.score, 1.0);
        let info = session.score_info();
        assert!(info.beats(None, true));
        let low = ChallengeScoreInfo {
            score: 0.0,
            ..info.clone()
        };
        assert!(info.beats(Some(&low), true));
        assert!(!low.beats(Some(&info), true));
    }

    /// `ChallengesProgress.TrySetHighScoreOfChallenge`: stars take precedence
    /// over score, NaN is rejected, ties do not replace, `highIsBetter` flips
    /// the comparison, and the key ignores spaces and case.
    #[test]
    fn high_score_rule_matches_challenges_progress() {
        let record = |stars: u8, score: f32| ChallengeScoreInfo {
            challenge_name: "Big Lake".to_owned(),
            star_attained: stars,
            score,
            elapsed_s: 0.0,
            champion_name: String::new(),
            champion_path: String::new(),
        };
        let stored = record(2, 100.0);
        // Fewer stars never replace, even with a much better score.
        assert!(!record(1, 1_000.0).beats(Some(&stored), true));
        // Same stars: strictly better score replaces, a tie does not.
        assert!(record(2, 101.0).beats(Some(&stored), true));
        assert!(!record(2, 100.0).beats(Some(&stored), true));
        // More stars still need a strictly better score (stars only block fewer stars).
        assert!(!record(3, 1.0).beats(Some(&stored), true));
        assert!(record(3, 200.0).beats(Some(&stored), true));
        // Lower is better flips only the score comparison.
        assert!(record(2, 99.0).beats(Some(&stored), false));
        assert!(!record(2, 101.0).beats(Some(&stored), false));
        // NaN is rejected; a stored NaN (never scored) is replaced.
        assert!(!record(3, f32::NAN).beats(None, true));
        assert!(record(0, 5.0).beats(Some(&record(0, f32::NAN)), true));
        assert_eq!(ChallengeScoreInfo::name_to_key("Big Lake"), "biglake");
    }

    #[test]
    fn challenge_metric_material_and_time() {
        use crate::settings::{SimMetric, SimulationMetric};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 29).expect("world");
        world.simulated_time = 12.5;
        assert_eq!(world.metric_value(SimMetric::Time, ""), 12.5);
        let before = world.metric_value(SimMetric::MaterialCount, "Plant");
        let _ = world.spawn_pellet_with_energy(MaterialKind::Plant, Vec2::ZERO, 100.0);
        assert_eq!(
            world.metric_value(SimMetric::MaterialCount, "Plant"),
            before + 1.0
        );
        assert_eq!(
            world.metric_value(SimMetric::MaterialBiomass, "Plant"),
            100.0
        );
        let metric = SimulationMetric::new(SimMetric::Constant, "7.5");
        assert_eq!(world.challenge_metric(&metric), 7.5);
    }

    #[test]
    fn challenge_champion_row_spawns_from_loaded_template() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut scenario = scenario_with_zone_group();
        scenario.bibite_spawns.clear();
        let mut world = World::new(default_settings(), scenario, 30).expect("world");
        world.set_templates(vec![TemplateSource::new(
            "champion_template",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        )]);
        let mut champion = crate::settings::BibiteSpawn::champion_defaults();
        champion.path = "champion_template".to_owned();
        champion.minimum = 1;
        world.add_bibite_spawn(champion).expect("champion row");
        let _ = world.seed_scenario_world();
        world.start_fresh_scenario_spawner();
        assert!(!world.bibites.is_empty());
        assert!(
            world
                .bibites
                .iter()
                .any(|(_, record)| record.meta.species_tag.as_deref() == Some("Champion"))
        );
    }

    #[test]
    fn challenge_star_threshold_editor_helpers() {
        let mut params = ChallengeParameters::game_defaults();
        assert_eq!(
            params.star_thresholds(),
            [Some(50.0), Some(100.0), Some(200.0)]
        );
        params
            .set_star_thresholds([10.0, 20.0, 30.0])
            .expect("set thresholds");
        assert_eq!(
            params.star_thresholds(),
            [Some(10.0), Some(20.0), Some(30.0)]
        );
        params.validate().expect("valid");
        let mut composite = params.clone();
        composite.one_star_condition =
            crate::settings::ConditionGroup::new(crate::settings::ConditionGroupLogic::And);
        assert!(composite.star_thresholds()[0].is_none());
        assert!(composite.set_star_thresholds([1.0, 2.0, 3.0]).is_err());
    }

    #[test]
    fn spatial_hash_checkpoint_preserves_class_slot_maps() {
        let mut hash = SpatialHash::new(Vec2::ZERO, Vec2::splat(2000.0), 100.0);
        let bibite = crate::arena::Handle::new(6, 1);
        let pellet = crate::arena::Handle::new(4, 9);
        let egg = crate::arena::Handle::new(3, 7);
        let phero = crate::arena::Handle::new(5, 2);
        hash.insert(bibite, SpatialClass::Bibite, Vec2::new(5.0, 5.0), 2.0);
        hash.insert(pellet, SpatialClass::Pellet, Vec2::new(7.0, 7.0), 1.5);
        hash.insert(egg, SpatialClass::Egg, Vec2::new(10.0, 10.0), 1.0);
        hash.insert(phero, SpatialClass::Pheromone, Vec2::new(20.0, 20.0), 0.0);
        let bytes = hash.capture_checkpoint_bytes();
        let mut restored = SpatialHash::new(Vec2::ZERO, Vec2::splat(2000.0), 100.0);
        restored
            .restore_checkpoint_bytes(&bytes)
            .expect("restore hash");
        assert_eq!(restored.capture_checkpoint_bytes(), bytes);
        assert_eq!(restored.slot_maps[SpatialClass::Bibite as usize].len(), 7);
        assert_eq!(restored.slot_maps[SpatialClass::Pellet as usize].len(), 5);
        assert_eq!(
            restored.slot_maps[SpatialClass::Pheromone as usize].len(),
            6
        );
        assert_eq!(restored.slot_maps[SpatialClass::Egg as usize].len(), 4);
    }

    #[test]
    fn fov_sense_before_lookup_uses_empty_cache() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::pellet::MaterialKind;

        let mut world = World::new(default_settings(), ScenarioData::default(), 63).expect("world");
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::ZERO;
        spawn.rot = 0.0;
        let id = world.bibites.spawn(spawn).expect("bibite");
        world.register_bibite(id);
        world
            .pellets
            .spawn_with_energy(MaterialKind::Plant, 200.0, Vec2::Y * 6.0)
            .expect("pellet");
        world.sync_pellet_hash();
        // Sense phase due (5th brain update), lookup phase not due (1/10):
        // the game's first senses come from the empty loaded cache.
        {
            let record = world.bibites.get_mut(id).expect("record");
            record.throttle.brain_counter = world.settings.brain_tps - 1;
            record.throttle.lookup_counter = 0;
            record.throttle.sense_counter = world.settings.vision_sense_updates - 1;
        }
        world.step_sensing();
        let senses = world.bibites.senses(id).copied().unwrap_or_default();
        assert_eq!(senses.plant_closeness, 0.0);
        assert!(world.bibites.get(id).unwrap().fov_seen.pellets.is_empty());
        // Now make both due: lookup fills the cache, then senses use it.
        {
            let record = world.bibites.get_mut(id).expect("record");
            record.throttle.lookup_counter = world.settings.vision_lookup_updates - 1;
            record.throttle.sense_counter = world.settings.vision_sense_updates - 1;
        }
        world.step_sensing();
        let record = world.bibites.get(id).expect("record");
        assert!(!record.fov_seen.pellets.is_empty());
        assert!(record.senses.plant_closeness > 0.0);
    }

    #[test]
    fn fov_targets_are_what_the_sensor_sees_with_the_weights_it_computes() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::pellet::{MaterialKind, NORMAL_AMOUNT_AT_SCALE1};
        use crate::senses::TargetKind;

        let mut world = World::new(default_settings(), ScenarioData::default(), 63).expect("world");
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::ZERO;
        spawn.rot = 0.0;
        let id = world.bibites.spawn(spawn).expect("bibite");
        world.register_bibite(id);
        // Before any lookup there is nothing to list (the cache is empty, as it is for the sensor).
        assert_eq!(
            world.fov_targets(id).expect("targets"),
            crate::senses::FovTargets::default()
        );
        // A plant straight ahead and one straight behind the bibite (outside its cone).
        let ahead = world
            .pellets
            .spawn_with_energy(MaterialKind::Plant, 200.0, Vec2::Y * 6.0)
            .expect("pellet");
        let behind = world
            .pellets
            .spawn_with_energy(MaterialKind::Plant, 200.0, Vec2::Y * -6.0)
            .expect("pellet");
        world.sync_pellet_hash();
        {
            let record = world.bibites.get_mut(id).expect("record");
            record.throttle.brain_counter = world.settings.brain_tps - 1;
            record.throttle.lookup_counter = world.settings.vision_lookup_updates - 1;
            record.throttle.sense_counter = world.settings.vision_sense_updates - 1;
        }
        world.step_sensing();
        let record = world.bibites.get(id).expect("record");
        let view = world.pellets.get(ahead).expect("pellet");
        let view_radius = record.genome.gene(Gene::ViewRadius) * record.derived.d1_size;
        // The weight of `FieldOfView.ComputeSenses`: (1.05 - d / R)^2 * size term * (1.05 - |angle| / (A / 2)), here at angle 0.
        let distance = 6.0 - view.radius;
        let size_term = view.amount / NORMAL_AMOUNT_AT_SCALE1;
        let expected = (1.05 - distance / view_radius).powi(2) * size_term * 1.05;
        let targets = world.fov_targets(id).expect("targets");
        let (plants, sum, max) = targets.channel(TargetKind::Plant);
        assert_eq!(plants.len(), 1, "only the plant ahead is in the cone");
        assert_ne!(plants[0].key, crate::body::pellet_tag(behind));
        assert_eq!(plants[0].key, crate::body::pellet_tag(ahead));
        assert!(
            (plants[0].weight - expected).abs() <= 1e-4 * expected.abs().max(1.0),
            "{} vs {expected}",
            plants[0].weight
        );
        assert_eq!((sum, max), (plants[0].weight, plants[0].weight));
        assert!((plants[0].scale - size_term.sqrt()).abs() < 1e-6);
        assert_eq!(plants[0].pos, view.pos);
        // The sensor agrees on the geometry: one plant in view, so its closeness is 1 - distance / R.
        assert!((record.senses.plant_closeness - (1.0 - distance / view_radius)).abs() < 1e-4);
        // A bibite that is not there has no targets.
        let gone = BibiteId::from_handle(crate::arena::Handle::new(999, 1));
        assert!(world.fov_targets(gone).is_none());
    }

    #[test]
    fn checkpoint_100k_scale_is_rejected_by_preflight() {
        // ~425 MB estimated for 100k, beyond the 256 MiB payload cap; capture
        // must return LimitExceeded instead of panicking.
        let estimate = checkpoint_payload_estimate(100_000, 10_000);
        assert!(estimate > crate::sim::MAX_PAYLOAD_BYTES);
        let ok = checkpoint_payload_estimate(10_000, 2_000);
        assert!(ok < crate::sim::MAX_PAYLOAD_BYTES);
    }

    #[test]
    #[ignore = "large checkpoint scale (5k/10k entities)"]
    fn checkpoint_large_world_5k_10k_roundtrip() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::settings::{BibiteSpawn as SettingsSpawn, SpawnType};

        fn large(count: usize) -> World {
            let mut settings = default_settings();
            settings.independents.simulation_size = 2000.0 * (count as f32 / 2000.0).sqrt();
            let mut world = World::new(settings, scenario_with_zone_group(), 123).expect("world");
            world.seed_scenario_world();
            world.set_templates(vec![TemplateSource::new(
                "starter",
                Genome::starter(),
                GenomeMeta::default(),
                Brain::starter(),
            )]);
            world.bibite_spawns = vec![SettingsSpawn {
                path: "starter".to_owned(),
                minimum: count as i32,
                spawn_type: SpawnType::OneTime,
                ..SettingsSpawn::game_defaults()
            }];
            let spawned = world.spawn_initial_population();
            assert_eq!(spawned, count);
            world
        }

        for count in [5000usize, 10000] {
            let world = large(count);
            let started = std::time::Instant::now();
            let snapshot = world
                .capture_checkpoint_state()
                .expect("capture large checkpoint");
            let captured = started.elapsed();
            let encode_started = std::time::Instant::now();
            let bytes = snapshot.encode().expect("encode large snapshot");
            let encoded = encode_started.elapsed();
            let decode_started = std::time::Instant::now();
            let decoded = crate::sim::SimSnapshot::decode(&bytes).expect("decode large snapshot");
            let decoded_time = decode_started.elapsed();
            let mut restored = large(count);
            let restored_started = std::time::Instant::now();
            restored
                .restore_checkpoint_state(&decoded)
                .expect("restore large checkpoint");
            let restored_time = restored_started.elapsed();
            assert_eq!(
                crate::sim::state_fingerprint(&restored),
                crate::sim::state_fingerprint(&world)
            );
            eprintln!(
                "large checkpoint {count}: capture={captured:?} encode={encoded:?} decode={decoded_time:?} restore={restored_time:?} bytes={} extensions={} chunks={}",
                bytes.len(),
                snapshot.extensions.len(),
                decoded.extensions.len()
            );
        }
    }

    #[test]
    fn checkpoint_round_trip_preserves_empty_world_control_state() {
        let mut source =
            World::new(default_settings(), scenario_with_zone_group(), 73).expect("world");
        source.seed_scenario_world();
        source.run(10);
        let snapshot = source.capture_checkpoint_state().expect("capture");
        let bytes = snapshot.encode().expect("encode");
        let decoded = crate::sim::SimSnapshot::decode(&bytes).expect("decode");
        let mut restored =
            World::new(default_settings(), scenario_with_zone_group(), 73).expect("world");
        restored.seed_scenario_world();
        restored
            .restore_checkpoint_state(&decoded)
            .expect("restore");
        assert_eq!(restored.clock.tick(), source.clock.tick());
        assert_eq!(restored.simulated_time, source.simulated_time);
        assert_eq!(restored.rng.state(), source.rng.state());
        assert_eq!(
            crate::sim::state_fingerprint(&restored),
            crate::sim::state_fingerprint(&source)
        );
        assert_eq!(
            restored.zones.encode_runtime(),
            source.zones.encode_runtime()
        );
    }

    #[test]
    fn conservation_identity_holds_without_feeding() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 71).expect("world");
        let seeded = world.seed_scenario_world().expect("seeded");
        assert!(seeded.seeded_plant_energy > 0.0);
        let start = world.conservation_snapshot();
        world.run(300);
        let end = world.conservation_snapshot();
        let residual = end.totals.plant_energy_residual(
            &start.totals,
            start.stock_plant_energy,
            end.stock_plant_energy,
        );
        let scale = start.stock_plant_energy.max(1.0);
        assert!(
            residual.abs() <= scale * 1e-9,
            "plant conservation residual {residual} (stock {scale})"
        );
        assert!(end.totals.produced_plant_energy >= start.totals.produced_plant_energy);
    }

    #[test]
    fn conservation_identity_includes_eaten_matter() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::pellet::MaterialKind;

        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 72).expect("world");
        world.seed_scenario_world();
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::ZERO;
        spawn.rot = 0.0;
        let id = world.bibites.spawn(spawn).expect("bibite");
        world.register_bibite(id);
        let pellet = world
            .pellets
            .spawn_with_energy(MaterialKind::Plant, 200.0, Vec2::Y * 6.0)
            .expect("pellet");
        world.sync_pellet_hash();
        let start = world.conservation_snapshot();
        world.run(300);
        let end = world.conservation_snapshot();
        assert!(
            end.totals.consumed_plant_u2 > start.totals.consumed_plant_u2,
            "expected plant intake to be accounted"
        );
        let residual = end.totals.plant_energy_residual(
            &start.totals,
            start.stock_plant_energy,
            end.stock_plant_energy,
        );
        let scale = start.stock_plant_energy.max(1.0);
        assert!(
            residual.abs() <= scale * 1e-9,
            "plant conservation residual with feeding {residual} (stock {scale}, pellet {pellet:?})"
        );
    }

    /// A tester's report: "no bibites spawned from this spawner". A default bibite added to a world that is already running (the in-game Default Bibites panel
    /// adds the row with `set_bibite_spawns`), or one a save carries, spawns its lineage up to the row's minimum (`BibiteSpawner.SpawnEntity`).
    #[test]
    fn a_spawner_row_added_to_a_running_world_spawns() {
        use crate::settings::{BibiteSpawn, SpawnType, ZoneGroupSettings};
        let template = |name: &str| {
            TemplateSource::new(
                name,
                crate::genome::Genome::starter(),
                crate::genome::GenomeMeta::default(),
                crate::brain::Brain::starter(),
            )
        };
        let scenario = ScenarioData { zone_groups: vec![ZoneGroupSettings::game_defaults()], ..ScenarioData::default() };
        let mut world = World::new(default_settings(), scenario, 41).expect("world");
        world.set_templates(vec![template("Basic bibite.bb8template")]);
        assert!(world.bibites.is_empty());
        let row = BibiteSpawn {
            path: "Basic bibite.bb8template".to_owned(),
            minimum: 5,
            spawn_type: SpawnType::Continuous,
            ..BibiteSpawn::game_defaults()
        };
        world.set_bibite_spawns(vec![row]).expect("row");
        world.run(30 * 12);
        assert_eq!(world.bibites.len(), 5, "the spawner brings the lineage to the row's minimum");
    }

    /// The game resumes the spawner after a load (`SaveSystem` -> `SimulationManager.ResumeSpawners` -> `BibiteSpawner.StartSpawner`): a save whose scenario has a
    /// default bibite below its minimum spawns more.
    #[test]
    fn a_loaded_world_resumes_its_spawner() {
        use crate::settings::{BibiteSpawn, SpawnType, ZoneGroupSettings};
        let row = BibiteSpawn {
            path: "starter".to_owned(),
            minimum: 4,
            spawn_type: SpawnType::Continuous,
            ..BibiteSpawn::game_defaults()
        };
        let scenario = ScenarioData { zone_groups: vec![ZoneGroupSettings::game_defaults()], bibite_spawns: vec![row], ..ScenarioData::default() };
        let template = TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        );
        let mut world = World::new(default_settings(), scenario, 43).expect("world");
        world.set_templates(vec![template]);
        assert!(!world.fresh_scenario_spawner_started());
        world.resume_spawner_after_load();
        assert!(world.fresh_scenario_spawner_started());
        world.run(30 * 10);
        assert_eq!(world.bibites.len(), 4, "a loaded world's spawner brings the lineage to the minimum");
    }

    #[test]
    fn fresh_scenario_spawner_matches_game_cadence() {
        use crate::settings::{BibiteSpawn, SpawnType, ZoneGroupSettings};
        let row = BibiteSpawn {
            path: "starter".to_owned(),
            minimum: 10,
            spawn_type: SpawnType::Continuous,
            ..BibiteSpawn::game_defaults()
        };
        let scenario = ScenarioData {
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            bibite_spawns: vec![row],
            ..ScenarioData::default()
        };
        let template = TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        );
        let mut world = World::new(default_settings(), scenario, 39).expect("world");
        world.set_templates(vec![template]);
        assert!(!world.fresh_scenario_spawner_started());
        // Game coroutine's first iteration is immediate (`yield return null`):
        // a fresh scenario has 1 bibite at t=0.
        assert_eq!(world.start_fresh_scenario_spawner(), 1);
        assert_eq!(world.bibites.len(), 1);
        assert!(world.fresh_scenario_spawner_started());

        // One bibite per simulated second at virginSpawnRate=1 (30 TPS default).
        world.run(29);
        assert_eq!(world.bibites.len(), 1);
        world.run(1);
        assert_eq!(world.bibites.len(), 2, "first 1 s interval");
        world.run(30 * 7);
        assert_eq!(world.bibites.len(), 9, "8 s after start");
        world.run(30);
        assert_eq!(world.bibites.len(), 10, "minimum reached at 9 s");
        world.run(30 * 5);
        assert_eq!(world.bibites.len(), 10, "spawner pauses at the minimum");
        assert_eq!(world.bibite_spawns[0].total_spawned, 10);
    }

    #[test]
    fn scenario_spawn_positions_are_inside_zone_shapes() {
        use crate::settings::{BibiteSpawn, SpawnDistribution};

        // Ring-like long thin Rect: the old generic-disc sampler used the shape
        // radius (48,750 for Ring) and spawned thousands of units outside it.
        let mut zone = crate::settings::ZoneSettings::game_defaults();
        zone.name = "strip".to_owned();
        zone.zone_id = Some(0);
        zone.material = Some(crate::settings::MaterialKind::Plant);
        zone.distribution = SpawnDistribution::Rect;
        zone.pos_x = 0.0;
        zone.pos_y = 0.0;
        zone.radius_is_relative = true;
        zone.width_relative = 1.9;
        zone.height_relative = 0.05;
        let row = BibiteSpawn {
            path: "starter".to_owned(),
            minimum: 10,
            ..BibiteSpawn::game_defaults()
        };
        let scenario = ScenarioData {
            zones: vec![zone],
            bibite_spawns: vec![row],
            ..ScenarioData::default()
        };
        let template = TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        );

        let mut world = World::new(default_settings(), scenario.clone(), 45).expect("world");
        world.seed_scenario_world();
        world.set_templates(vec![template.clone()]);
        world.start_fresh_scenario_spawner();
        while world.bibites.len() < 10 {
            world.step_spawner(1.0);
        }
        assert_eq!(world.bibites.len(), 10);
        for (_, record) in world.bibites.iter() {
            assert!(
                world.zones.zones[0].contains(record.pos),
                "spawn outside zone: {:?}",
                record.pos
            );
        }

        // The legacy immediate path must use the same sampler.
        let mut world = World::new(default_settings(), scenario, 46).expect("world");
        world.set_templates(vec![template]);
        assert_eq!(world.spawn_initial_population(), 10);
        for (_, record) in world.bibites.iter() {
            assert!(
                world.zones.zones[0].contains(record.pos),
                "initial spawn outside zone: {:?}",
                record.pos
            );
        }
    }

    #[test]
    fn runtime_spawned_starter_eats_pellet_at_mouth() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::pellet::MaterialKind;

        // Deterministic feeding-path regression: place the pellet directly at
        // the mouth instead of relying on a starter brain to forage (Ring
        // starters are known-poor foragers; see docs/PARITY_ROUND2.md §8/§9).
        let mut world = World::new(default_settings(), ScenarioData::default(), 48).expect("world");
        let mut spawn =
            BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::ZERO;
        spawn.rot = 0.0;
        let id = world.bibites.spawn(spawn).expect("bibite");
        world.register_bibite(id);
        let up = Vec2::Y;
        let pellet = world
            .pellets
            .spawn_with_energy(MaterialKind::Plant, 200.0, up * 6.0)
            .expect("pellet");
        world.sync_pellet_hash();
        let before = world.pellets.get(pellet).expect("pellet").amount;
        let mut max_stomach = 0.0f32;
        for _ in 0..300 {
            world.tick();
            if let Some(record) = world.bibites.get(id) {
                max_stomach =
                    max_stomach.max(record.stomach.contents().map(|c| c.amount).sum::<f32>());
            }
        }
        let after = world.pellets.get(pellet).map_or(0.0, |p| p.amount);
        assert!(
            after < before - 1.0 || max_stomach > 0.0,
            "runtime-spawned starter did not feed: amount {before}->{after}, stomach {max_stomach}"
        );
    }

    #[test]
    fn spawn_row_zone_is_honoured() {
        use crate::settings::{BibiteSpawn, SpawnDistribution};

        let mut zones = Vec::new();
        for x in [-0.5f32, 0.5] {
            let mut zone = crate::settings::ZoneSettings::game_defaults();
            zone.name = format!("zone{x}");
            zone.material = Some(crate::settings::MaterialKind::Plant);
            zone.distribution = SpawnDistribution::Rect;
            zone.pos_x = x;
            zone.pos_y = 0.0;
            zone.radius_is_relative = true;
            zone.width_relative = 0.1;
            zone.height_relative = 0.1;
            zones.push(zone);
        }
        let row = BibiteSpawn {
            path: "starter".to_owned(),
            minimum: 6,
            spawn_zone: Some(1),
            ..BibiteSpawn::game_defaults()
        };
        let scenario = ScenarioData {
            zones,
            bibite_spawns: vec![row],
            ..ScenarioData::default()
        };
        let mut world = World::new(default_settings(), scenario, 49).expect("world");
        world.set_templates(vec![TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        )]);
        world.start_fresh_scenario_spawner();
        while world.bibites.len() < 6 {
            world.step_spawner(1.0);
        }
        for (_, record) in world.bibites.iter() {
            assert!(
                world.zones.zones[1].contains(record.pos),
                "spawn_zone=1 but spawn landed outside target zone: {:?}",
                record.pos
            );
        }
    }

    #[test]
    fn fresh_scenario_spawner_is_deterministic_and_off_for_loaded_worlds() {
        use crate::settings::{BibiteSpawn, ZoneGroupSettings};
        let row = BibiteSpawn {
            path: "starter".to_owned(),
            minimum: 5,
            ..BibiteSpawn::game_defaults()
        };
        let scenario = ScenarioData {
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            bibite_spawns: vec![row],
            ..ScenarioData::default()
        };
        let template = TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        );
        let build = || {
            let mut world = World::new(default_settings(), scenario.clone(), 41).expect("world");
            world.set_templates(vec![template.clone()]);
            world.start_fresh_scenario_spawner();
            world.run(30 * 4);
            world
        };
        let a = build();
        let b = build();
        let positions_a: Vec<Vec2> = a.bibites.iter().map(|(_, record)| record.pos).collect();
        let positions_b: Vec<Vec2> = b.bibites.iter().map(|(_, record)| record.pos).collect();
        assert_eq!(positions_a, positions_b);
        assert_eq!(a.bibites.len(), b.bibites.len());

        // A freshly constructed world (the save-load path) has no fresh spawner.
        let loaded = World::new(default_settings(), ScenarioData::default(), 42).expect("world");
        assert!(!loaded.fresh_scenario_spawner_started());
    }

    #[test]
    fn spawner_initial_population_is_deterministic_and_limited() {
        use crate::settings::{BibiteSpawn, SpawnType, ZoneGroupSettings};
        let row = BibiteSpawn {
            path: "starter".to_owned(),
            minimum: 3,
            spawn_type: SpawnType::Continuous,
            ..BibiteSpawn::game_defaults()
        };
        let scenario = ScenarioData {
            zone_groups: vec![ZoneGroupSettings::game_defaults()],
            bibite_spawns: vec![row],
            ..ScenarioData::default()
        };
        let template = TemplateSource::new(
            "starter",
            crate::genome::Genome::starter(),
            crate::genome::GenomeMeta::default(),
            crate::brain::Brain::starter(),
        );
        let mut a = World::new(default_settings(), scenario.clone(), 31).expect("world a");
        a.set_templates(vec![template.clone()]);
        assert_eq!(a.spawn_initial_population(), 3);
        assert_eq!(a.bibites.len(), 3);
        assert_eq!(a.bibite_spawns[0].total_spawned, 3);
        // The table is satisfied; the continuous spawner tops nothing up.
        assert_eq!(a.step_spawner(5.0), 0);
        assert_eq!(a.spawn_initial_population(), 0);

        let mut b = World::new(default_settings(), scenario, 31).expect("world b");
        b.set_templates(vec![template]);
        b.spawn_initial_population();
        let positions_a: Vec<Vec2> = a.bibites.iter().map(|(_, record)| record.pos).collect();
        let positions_b: Vec<Vec2> = b.bibites.iter().map(|(_, record)| record.pos).collect();
        assert_eq!(positions_a, positions_b);
    }

    #[test]
    fn scene_round_trip_time_and_loaded_egg() {
        use crate::body::EggSave;
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world = World::new(default_settings(), ScenarioData::default(), 21).expect("world");
        world.run(30);
        let scene = world.scene();
        assert!((scene.simulated_time - 1.0).abs() < 1e-6);

        let mut loaded =
            World::new(default_settings(), ScenarioData::default(), 22).expect("world");
        loaded.restore_scene(scene.clone());
        assert_eq!(loaded.scene(), scene);
        loaded.set_simulated_time(123.5);
        let sample = loaded.sample_stats();
        assert!((sample.t - 123.5).abs() < 1e-3);

        let egg = loaded.spawn_loaded_egg(EggSave {
            pos: Vec2::new(5.0, 5.0),
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain: Brain::starter(),
            energy: 10.0,
            hatch_progress: 0.0,
            hatch_time: 100.0,
            total_required_energy: 5.0,
            is_hatching: true,
            aborting: false,
            birth_orientation: -Vec2::Y,
            size_gene: 1.0,
        });
        assert!(loaded.eggs.get(egg).is_some());
        assert!(loaded.hash.contains(egg.handle(), SpatialClass::Egg));
        loaded.rebuild_hashes();
        assert!(loaded.hash.contains(egg.handle(), SpatialClass::Egg));
    }

    // ----- W4c: WorldContents round-trip -----

    #[test]
    fn contents_round_trip_empty_world() {
        let world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        let contents = world.to_contents();
        assert_eq!(contents.seed, 5);
        assert_eq!(contents.scene, world.scene());
        let loaded = World::from_contents(contents).expect("load");
        assert_eq!(loaded.to_contents().zones, world.to_contents().zones);
        assert_eq!(loaded.settings, world.settings);
        assert_eq!(loaded.scene(), world.scene());
        assert_eq!(loaded.bibites.len(), 0);
        assert_eq!(loaded.eggs.len(), 0);
        assert_eq!(loaded.pellets.len(), 0);
        assert_eq!(loaded.tools.len(), 0);
        assert_eq!(loaded.species.species_count(), 0);
        assert_eq!(loaded.seed, 5);
    }

    #[test]
    fn loaded_save_seeds_conservation_ledger_with_initial_stock() {
        let mut source =
            World::new(default_settings(), scenario_with_zone_group(), 37).expect("world");
        source.seed_scenario_world();
        source.run(10);
        let stock = source.pellets.counts().plant_biomass;
        let loaded = World::from_contents(source.to_contents()).expect("load");
        assert!(loaded.conservation_totals().seeded_plant_energy >= stock);
    }

    #[test]
    fn parent_links_follow_the_saved_positions_through_the_contents() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};

        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 43).expect("world");
        world.seed_scenario_world();
        let first = spawn_test_bibite(&mut world, Vec2::new(10.0, 20.0));
        let second = spawn_test_bibite(&mut world, Vec2::new(-10.0, 20.0));
        let third = spawn_test_bibite(&mut world, Vec2::new(0.0, -20.0));
        let orphan = spawn_test_bibite(&mut world, Vec2::new(0.0, 30.0));
        let gone = spawn_test_bibite(&mut world, Vec2::new(0.0, 40.0));
        let lost = spawn_test_bibite(&mut world, Vec2::new(0.0, 50.0));
        world.bibites.get_mut(second).expect("second").meta.parent1 = Some(first);
        {
            let meta = &mut world.bibites.get_mut(third).expect("third").meta;
            meta.parent1 = Some(first);
            meta.parent2 = Some(second);
        }
        // The second parent counts only with a first one (`SaveSystem`).
        world.bibites.get_mut(orphan).expect("orphan").meta.parent2 = Some(first);
        // A parent that is gone leaves no link, and then the second one does not count either.
        {
            let meta = &mut world.bibites.get_mut(lost).expect("lost").meta;
            meta.parent1 = Some(gone);
            meta.parent2 = Some(first);
        }
        assert!(world.bibites.remove(gone).is_some());
        world.eggs.spawn(crate::body::EggSpawn {
            pos: Vec2::new(-10.0, 5.0),
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta {
                parent1: Some(third),
                ..GenomeMeta::default()
            },
            brain: Brain::starter(),
            energy: 20.0,
            hatch_time: 100.0,
            total_required_energy: 5.0,
            birth_orientation: -Vec2::Y,
        });
        let contents = world.to_contents();
        assert_eq!(contents.bibites.len(), 5);
        assert_eq!(
            contents.bibite_parents,
            vec![
                [None, None],
                [Some(0), None],
                [Some(0), Some(1)],
                [None, None],
                [None, None]
            ]
        );
        assert_eq!(contents.egg_parents, vec![[Some(2), None]]);

        let loaded = World::from_contents(contents).expect("load");
        let ids: Vec<BibiteId> = loaded.bibites.ids().collect();
        let meta = |index: usize| loaded.bibites.get(ids[index]).expect("record").meta.clone();
        assert_eq!(meta(0).parent1, None);
        assert_eq!(meta(1).parent1, Some(ids[0]));
        assert_eq!(meta(1).parent2, None);
        assert_eq!(meta(2).parent1, Some(ids[0]));
        assert_eq!(meta(2).parent2, Some(ids[1]));
        assert_eq!((meta(3).parent1, meta(3).parent2), (None, None));
        assert_eq!((meta(4).parent1, meta(4).parent2), (None, None));
        let egg = loaded.eggs.ids().next().expect("egg");
        assert_eq!(
            loaded.eggs.get(egg).expect("egg").meta.parent1,
            Some(ids[2])
        );
    }

    #[test]
    fn contents_round_trip_populated_world() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        use crate::pheromone::PheroEmission;
        use crate::tools::{ColorKiller, PheroTower, RadTower};

        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 41).expect("world");
        world.seed_scenario_world();
        let _bibite = spawn_test_bibite(&mut world, Vec2::new(10.0, 20.0));
        world.eggs.spawn(crate::body::EggSpawn {
            pos: Vec2::new(-10.0, 5.0),
            vel: Vec2::ZERO,
            rot: 0.0,
            ang_vel: 0.0,
            genome: Genome::starter(),
            meta: GenomeMeta::default(),
            brain: Brain::starter(),
            energy: 20.0,
            hatch_time: 100.0,
            total_required_energy: 5.0,
            birth_orientation: -Vec2::Y,
        });
        world.add_rad_tower(RadTower::new(1, Vec2::ZERO, 2000.0));
        world.add_phero_tower(PheroTower::new(2, Vec2::new(5.0, 0.0)));
        world.add_color_killer(ColorKiller::new(3, Vec2::new(-5.0, 0.0)));
        // Attach the rad tower to zone 0 (runtime id) for attachment remapping.
        let zone0 = world.zones.zones[0].id;
        if let Some(tool) = world
            .tools
            .get_mut(crate::tools::ToolStore::tool_id_at(0).unwrap())
        {
            tool.base_mut().attached_zone = Some(zone0);
        }
        if let Some(spot) = world.pheromones.emit(PheroEmission {
            pos: Vec2::new(1.0, 2.0),
            heading: Vec2::Y,
            strength: [3.0, 2.0, 1.0],
            dissipate: true,
            owner_tool: None,
        }) {
            world.register_pheromone(spot);
        }
        world.run(5);
        let contents = world.to_contents();
        assert_eq!(contents.bibites.len(), 1);
        assert_eq!(contents.eggs.len(), 1);
        assert!(contents.pellets.len() > 10);
        assert!(!contents.pheromones.is_empty());
        assert_eq!(
            (
                contents.rad_towers.len(),
                contents.phero_towers.len(),
                contents.color_killers.len()
            ),
            (1, 1, 1)
        );

        let loaded = World::from_contents(contents.clone()).expect("load");
        // Save-shape (generation-0) attached zone is remapped to the runtime id.
        assert_eq!(
            loaded
                .tools
                .get(crate::tools::ToolStore::tool_id_at(0).unwrap())
                .map(|tool| tool.base().attached_zone),
            Some(Some(loaded.zones.zones[0].id))
        );
        let again = loaded.to_contents();
        assert_eq!(again.settings, contents.settings);
        assert_eq!(again.scene, contents.scene);
        assert_eq!(again.zones, contents.zones);
        assert_eq!(again.zone_groups, contents.zone_groups);
        assert_eq!(again.settings_changers, contents.settings_changers);
        assert_eq!(again.rad_towers, contents.rad_towers);
        assert_eq!(again.phero_towers, contents.phero_towers);
        assert_eq!(again.color_killers, contents.color_killers);
        assert_eq!(again.pellets, contents.pellets);
        assert_eq!(again.pheromones, contents.pheromones);
        assert_eq!(again.eggs.len(), contents.eggs.len());
        assert_eq!(again.bibites.len(), contents.bibites.len());
        assert_eq!(again.seed, contents.seed);
        assert_eq!(again.data_bin, contents.data_bin);
        assert_eq!(again.species, contents.species);
        // Loaded entity gets a valid hash entry and the same body-save data.
        let loaded_bibite = loaded.bibites.ids().next().expect("bibite");
        assert!(
            loaded
                .hash
                .contains(loaded_bibite.handle(), SpatialClass::Bibite)
        );
        assert_eq!(
            loaded.bibites.to_save(loaded_bibite).map(|save| save.pos),
            Some(contents.bibites[0].pos)
        );
    }

    #[test]
    fn contents_load_then_run_is_deterministic() {
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 43).expect("world");
        world.seed_scenario_world();
        spawn_test_bibite(&mut world, Vec2::new(30.0, 30.0));
        spawn_test_bibite(&mut world, Vec2::new(-30.0, 10.0));
        world.run(20);
        let contents = world.to_contents();
        let mut a = World::from_contents(contents.clone()).expect("a");
        let mut b = World::from_contents(contents).expect("b");
        a.run(100);
        b.run(100);
        let sample_a = a.sample_stats();
        let sample_b = b.sample_stats();
        assert_eq!(
            crate::sim::sample_fingerprint(&sample_a),
            crate::sim::sample_fingerprint(&sample_b)
        );
        assert_eq!(
            crate::sim::state_fingerprint(&a),
            crate::sim::state_fingerprint(&b)
        );
        for id in a.bibites.ids() {
            assert!(a.hash.contains(id.handle(), SpatialClass::Bibite));
        }
        for id in a.pellets.ids() {
            assert!(a.hash.contains(id.handle(), SpatialClass::Pellet));
        }
    }

    #[test]
    fn contents_rejects_invalid_pellet() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 7).expect("world");
        world
            .pellets
            .spawn(PelletSpawn::new(MaterialKind::Plant, 10.0, Vec2::ZERO));
        let mut contents = world.to_contents();
        contents.pellets[0].amount = 0.0;
        assert!(matches!(
            World::from_contents(contents),
            Err(Error::InvalidData(_))
        ));
    }

    // ----- PERF-31: phase audit harness (manual, release) -----

    fn audit_world(count: usize) -> World {
        audit_world_with(count, default_settings())
    }

    fn audit_world_with(count: usize, settings: SimSettings) -> World {
        use crate::body::SpawnVitals;
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world =
            World::new(settings.clone(), scenario_with_zone_group(), 1234).expect("world");
        world.seed_scenario_world();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let min = world.bounds.min();
        let max = world.bounds.max();
        let margin = (max - min).length() * 0.05;
        let low = min + Vec2::splat(margin);
        let high = max - Vec2::splat(margin);
        for index in 0..count {
            // Match the CLI synthetic spawner: deterministic spread positions.
            let mut rng = world
                .rng
                .fork(0xB1B1_7E50 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let pos = Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y));
            let rot = rng.f32_range(0.0, std::f32::consts::TAU);
            let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = pos;
            spawn.rot = rot;
            if let Ok(id) = world.bibites.spawn(spawn) {
                // Worst-case sensing cadence: next body tick is a brain tick and
                // a sense refresh is due.
                if let Some(record) = world.bibites.get_mut(id) {
                    record.throttle.brain_counter = settings.brain_tps.saturating_sub(1);
                    record.throttle.lookup_counter = settings.vision_lookup_updates;
                    record.throttle.sense_counter = settings.vision_sense_updates;
                }
                world.register_bibite(id);
            }
        }
        world
    }

    fn print_phase_table(label: &str, world: &World, wall: std::time::Duration) {
        let timings = world.phase_timings();
        let ticks = timings.ticks.max(1);
        println!(
            "{label}: ticks={} wall={:.3} ms/tick={:.3}",
            timings.ticks,
            wall.as_secs_f64() * 1000.0,
            wall.as_secs_f64() * 1000.0 / ticks as f64
        );
        let mut rows: Vec<(crate::sim::Phase, f64, f64)> = crate::sim::Phase::ALL
            .iter()
            .map(|&phase| {
                (
                    phase,
                    timings.nanos(phase) as f64 / ticks as f64 / 1_000_000.0,
                    timings.share(phase) * 100.0,
                )
            })
            .collect();
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (phase, ms, pct) in rows {
            if ms > 0.000_1 {
                println!("  {:<16} {:>9.3} ms/tick  {:>5.1}%", phase.name(), ms, pct);
            }
        }
    }

    // ----- PERF-35: serial/parallel sensing equivalence -----

    #[test]
    fn parallel_sensing_is_bit_identical_to_serial() {
        let build = || audit_world(500);
        let mut serial = build();
        serial.set_threads(1);
        let mut parallel4 = build();
        parallel4.set_threads(4);
        let mut parallel8 = build();
        parallel8.set_threads(8);
        assert_eq!(serial.threads(), 1);
        assert_eq!(parallel4.threads(), 4);
        assert_eq!(parallel8.threads(), 8);

        serial.run(100);
        parallel4.run(100);
        parallel8.run(100);

        let serial_state = crate::sim::state_fingerprint(&serial);
        assert_eq!(
            serial_state,
            crate::sim::state_fingerprint(&parallel4),
            "4-thread sensing must match serial state"
        );
        assert_eq!(
            serial_state,
            crate::sim::state_fingerprint(&parallel8),
            "8-thread sensing must match serial state"
        );
        let serial_sample = serial.sample_stats();
        let parallel_sample = parallel4.sample_stats();
        assert_eq!(
            crate::sim::sample_fingerprint(&serial_sample),
            crate::sim::sample_fingerprint(&parallel_sample)
        );
        for (a, b) in serial.bibites.iter().zip(parallel4.bibites.iter()) {
            assert_eq!(a.0, b.0);
            assert_eq!(a.1.senses, b.1.senses);
        }
    }

    /// The first byte at which two encoded checkpoints differ (for a readable failure).
    fn first_difference(a: &[u8], b: &[u8]) -> String {
        match a.iter().zip(b.iter()).position(|(x, y)| x != y) {
            Some(at) => format!("first difference at byte {at} of {} / {}", a.len(), b.len()),
            None => format!(
                "one is a prefix of the other ({} / {} bytes)",
                a.len(),
                b.len()
            ),
        }
    }

    /// A world of `count` adults packed into a square `side` units wide, so that bodies overlap, bite and collide (the
    /// spread `audit_world` has almost no contacts).
    fn packed_world(count: usize, side: f32) -> World {
        packed_world_with(count, side, default_settings())
    }

    /// [`packed_world`] with other settings.
    fn packed_world_with(count: usize, side: f32, settings: SimSettings) -> World {
        packed_world_of(count, side, settings, |_| crate::body::SpawnVitals::Adult)
    }

    /// [`packed_world`] with other settings and `vitals(index)` for the spawn modes (babies grow, which changes what a biter
    /// sees of them from one moment of the tick to the next).
    fn packed_world_of(
        count: usize,
        side: f32,
        settings: SimSettings,
        vitals: impl Fn(usize) -> crate::body::SpawnVitals,
    ) -> World {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world = World::new(settings, scenario_with_zone_group(), 4321).expect("world");
        world.seed_scenario_world();
        for index in 0..count {
            let mut rng = world
                .rng
                .fork(0xB1B1_7E50 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut spawn =
                BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
            spawn.vitals = vitals(index);
            spawn.pos = Vec2::new(
                rng.f32_range(-side / 2.0, side / 2.0),
                rng.f32_range(-side / 2.0, side / 2.0),
            );
            spawn.rot = rng.f32_range(0.0, std::f32::consts::TAU);
            if let Ok(id) = world.bibites.spawn(spawn) {
                world.register_bibite(id);
            }
        }
        world
    }

    /// A bibite out of energy that is set to produce eggs, with a Want2Heal output that is a number (0, 0.5) and one that is NaN: in
    /// every case it starves to death and lays nothing. The healing runs last in a turn and adds `hp` after its own spend, and a NaN
    /// output makes `hp` the whole missing health (`min` of a NaN is the other number, in the game's `Mathf.Min` too): the lethal spends
    /// of the turn were undone, and the bibite lived on, and laid eggs, on health alone. That is how the glitch save of 2026-11-13
    /// grew 257,000 bibites that no ecology could feed (a death in a `UseEnergy` stands, as in the game's `Hurting` -> `Die()`).
    #[test]
    fn a_bibite_out_of_energy_dies_whatever_its_want2heal_output_is() {
        use crate::body::SpawnVitals;
        use crate::brain::{ALL_OUTPUTS, Brain, N_OUTPUTS, Output};
        use crate::genome::{Genome, GenomeMeta};
        let mut settings = default_settings();
        // The brains never step, so the outputs set by hand stay as they are.
        settings.brain_tps = 1_000_000;
        settings.independents.limit_bibite_birth = false;
        let mut world = World::new(settings, ScenarioData::default(), 5).expect("world");
        let want = [0.0, 0.5, f32::NAN];
        let mut ids = Vec::new();
        for index in 0..want.len() {
            let mut spawn =
                BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(index as f32 * 3000.0, 0.0);
            let id = world.bibites.spawn(spawn).expect("spawn");
            world.register_bibite(id);
            ids.push(id);
        }
        for (id, heal) in ids.iter().zip(want) {
            let record = world.bibites.get_mut(*id).expect("record");
            let mut out = [0.0f32; N_OUTPUTS];
            out[Output::Want2Lay.index()] = 1.0;
            out[Output::EggProduction.index()] = 1.0;
            out[Output::Want2Heal.index()] = heal;
            for output in ALL_OUTPUTS {
                record.brain.set_output_value(output, out[output.index()]);
            }
            record.body.energy.amount = 0.0;
            record.egg_layer.is_mature = true;
        }
        world.run(900);
        for (id, heal) in ids.iter().zip(want) {
            let record = world.bibites.get(*id);
            assert!(
                record.is_none_or(|record| record.body.dead),
                "Want2Heal {heal}: the bibite without energy is alive after 30 s"
            );
            assert_eq!(
                record.map_or(0, |record| record.egg_layer.n_laid),
                0,
                "Want2Heal {heal}: a bibite without energy laid eggs"
            );
        }
        assert_eq!(world.eggs.len(), 0);
    }

    /// `count` upright adults on one vertical line (`spacing` apart): every body's box starts at exactly the same x, so the
    /// broadphase's x order is all ties and its result must not depend on how the sort or the sweep is split.
    fn lined_world(count: usize, spacing: f32) -> World {
        use crate::body::SpawnVitals;
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 777).expect("world");
        world.seed_scenario_world();
        for index in 0..count {
            let mut spawn =
                BibiteSpawn::new(Genome::starter(), GenomeMeta::default(), Brain::starter());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(0.0, index as f32 * spacing);
            spawn.rot = 0.0;
            if let Ok(id) = world.bibites.spawn(spawn) {
                world.register_bibite(id);
            }
        }
        world
    }

    /// Runs the worlds `rounds` times `ticks` ticks and compares their whole checkpoints (every body, pellet, egg and
    /// RNG state, as bytes) with the first one's after each round.
    fn assert_same_checkpoints(worlds: Vec<(String, World)>, rounds: usize, ticks: u64) {
        let _ = run_same_checkpoints(worlds, rounds, ticks, |_, _| {});
    }

    /// [`assert_same_checkpoints`] with `prepare(world, round)` called on every world before each round; hands the worlds
    /// back for a look at their counters.
    fn run_same_checkpoints(
        mut worlds: Vec<(String, World)>,
        rounds: usize,
        ticks: u64,
        prepare: impl Fn(&mut World, usize),
    ) -> Vec<(String, World)> {
        for round in 1..=rounds {
            for (_, world) in &mut worlds {
                prepare(world, round);
                world.run(ticks);
            }
            let encode = |world: &World| {
                world
                    .capture_checkpoint_state()
                    .expect("checkpoint")
                    .encode()
                    .expect("encoded checkpoint")
            };
            let expected = encode(&worlds[0].1);
            for (name, world) in &worlds[1..] {
                let actual = encode(world);
                assert!(
                    actual == expected,
                    "{name} differs from {} after {} ticks: {}",
                    worlds[0].0,
                    round as u64 * ticks,
                    first_difference(&expected, &actual)
                );
            }
        }
        worlds
    }

    /// Every stage that can run on the worker pool (body pass A, the physics bridge and stages, the brain batches,
    /// sensing) with its gate lowered to 1, against the serial tick: the whole checkpoint, byte for byte, as the run goes
    /// on.
    #[test]
    fn every_parallel_stage_is_bit_identical_to_serial() {
        let build = |threads: usize| {
            let mut world = audit_world(700);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            world
        };
        assert_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("3 threads".to_owned(), build(3)),
                ("8 threads".to_owned(), build(8)),
            ],
            5,
            12,
        );
    }

    /// The pellet decay, the producers' look at every record and the zones' recounts on the pool (their gates lowered to 1), against
    /// the serial tick, with twenty source bibites, plenty of meat that rots at once and a decay pass in every tick (so the chops, the
    /// removals and the pellets that take the freed slots follow each other all the time): the whole checkpoint, byte for byte.
    #[test]
    fn decay_producers_and_recounts_on_the_pool_give_the_serial_worlds_results() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let build = |threads: usize| {
            let mut world = audit_world(200);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            for index in 0..20 {
                let meta = GenomeMeta {
                    is_source: true,
                    ..GenomeMeta::default()
                };
                let mut spawn = BibiteSpawn::new(Genome::starter(), meta, Brain::starter());
                spawn.vitals = crate::body::SpawnVitals::Adult;
                spawn.pos = Vec2::new(index as f32 * 40.0 - 400.0, 25.0);
                let id = world.bibites.spawn(spawn).expect("source bibite");
                world.register_bibite(id);
            }
            for index in 0..1_500usize {
                let pos = Vec2::new(
                    ((index * 37) % 900) as f32 - 450.0,
                    ((index * 91) % 700) as f32 - 350.0,
                );
                let amount = 3.0 + ((index * 13) % 40) as f32;
                let _ = world.spawn_pellet_with_amount(MaterialKind::Meat, pos, amount);
            }
            world.decay = DecayThrottle::new(1);
            world.pellets.materials_mut().meat.decay_rate = 30.0;
            world
                .pellets
                .update_decay_parameters(MaterialKind::Meat, -5_000.0);
            world
        };
        let worlds = run_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("3 threads".to_owned(), build(3)),
                ("8 threads".to_owned(), build(8)),
            ],
            4,
            15,
            |_, _| {},
        );
        for (name, world) in &worlds {
            assert!(world.has_sources, "{name}");
            assert!(
                world.accounting.decayed_meat_energy > 0.0,
                "{name}: nothing decayed, the test does not look at the decay"
            );
        }
    }

    /// The grid broadphase (from a handful of bodies, the way the pool's gates lowered to 1 ask for it, on a pool of four workers or more) against the sweep
    /// (a grid gate no world reaches): the whole checkpoint, byte for byte, in a spread world and a packed one.
    #[test]
    fn the_grid_broadphase_gives_the_sweeps_worlds() {
        let build = |packed: bool, threads: usize, sweep_only: bool| {
            let mut world = if packed { packed_world(400, 300.0) } else { audit_world(700) };
            let mut gates = crate::sim::ParallelGates::EAGER;
            if sweep_only {
                gates.grid_bodies = usize::MAX;
            }
            world.set_parallel_gates(gates);
            world.set_threads(threads);
            world
        };
        for packed in [false, true] {
            assert_same_checkpoints(
                vec![
                    ("sweep".to_owned(), build(packed, 1, true)),
                    ("sweep, 8 threads".to_owned(), build(packed, 8, true)),
                    ("grid, serial".to_owned(), build(packed, 1, false)),
                    ("grid, 4 threads".to_owned(), build(packed, 4, false)),
                    ("grid, 8 threads".to_owned(), build(packed, 8, false)),
                ],
                4,
                10,
            );
        }
    }

    /// A world of sixty bibites has no stage that pays for the pool: with the sizes a world starts with (and no measuring) it ticks on
    /// the calling thread, every stage inline (the body sweep never runs on the pool), and a world of sixteen hundred bibites puts the
    /// sweep (from 1,500 records) and the rest of its stages that have the work on the pool. A world that measures its own ticks (what a
    /// new world does), whichever way it ends up running them, and the gates that put every stage on the pool from one item, give the
    /// serial worlds' checkpoints byte for byte.
    #[test]
    fn the_gates_a_world_starts_with_size_the_pool_to_the_work_with_the_serial_worlds_results() {
        let build = |bibites: usize, threads: usize, gates: Option<crate::sim::ParallelGates>| {
            let mut world = audit_world(bibites);
            assert_eq!(
                world.parallel_gates(),
                crate::sim::ParallelGates::SCALED,
                "the gates a new world has"
            );
            assert!(world.parallel_gates().adaptive, "a new world measures");
            if let Some(gates) = gates {
                world.set_parallel_gates(gates);
            }
            world.set_threads(threads);
            world
        };
        let fixed = Some(crate::sim::ParallelGates::FIXED);
        let worlds = run_same_checkpoints(
            vec![
                ("serial".to_owned(), build(60, 1, None)),
                ("8 threads, fixed sizes".to_owned(), build(60, 8, fixed)),
                ("8 threads, measuring".to_owned(), build(60, 8, None)),
                (
                    "8 threads, every stage on the pool".to_owned(),
                    build(60, 8, Some(crate::sim::ParallelGates::ALWAYS)),
                ),
            ],
            4,
            10,
            |_, _| {},
        );
        assert_eq!(
            worlds[0].1.pass_b_lanes(),
            crate::body::PassBLanes::default(),
            "serial"
        );
        assert_eq!(
            worlds[1].1.pass_b_lanes(),
            crate::body::PassBLanes::default(),
            "a world of 60 bibites has no stage with the work for the pool"
        );
        assert_eq!(
            worlds[1].1.pool_tuner_stats(),
            PoolTunerStats::default(),
            "fixed sizes do not measure"
        );
        assert!(
            worlds[3].1.pass_b_lanes().ticks > 0,
            "the gates that put every stage on the pool did not put the sweep of a world of 60 there: {:?}",
            worlds[3].1.pass_b_lanes()
        );
        let worlds = run_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1_600, 1, None)),
                ("8 threads, fixed sizes".to_owned(), build(1_600, 8, fixed)),
                ("8 threads, measuring".to_owned(), build(1_600, 8, None)),
            ],
            3,
            8,
            |_, _| {},
        );
        assert!(
            worlds[1].1.pass_b_lanes().ticks > 0,
            "no tick of a world of 1,600 bibites ran its sweep on the pool: {:?}",
            worlds[1].1.pass_b_lanes()
        );
        // The world that measures starts the way its size says (the pool, from the sizes of the first guess), so the same holds for it.
        assert!(
            worlds[2].1.pass_b_lanes().ticks > 0,
            "the measuring world of 1,600 bibites never ran a tick on the pool: {:?}",
            worlds[2].1.pass_b_lanes()
        );
    }

    /// The way a tick runs never changes what it computes, and a world may change from one way to the other tick by tick (what the
    /// pool tuner does when it probes): ticks that run inline (no handle to the pool: every stage inline) and ticks that run on the
    /// pool with every stage on it from one item, in runs of one to seven ticks of each, give the serial world's checkpoint byte for
    /// byte, in a spread of bibites and in a dense pile (contacts, welds and the solver), as the run goes on.
    #[test]
    fn a_world_switching_between_inline_and_pooled_ticks_has_the_serial_worlds_results() {
        let arms: Vec<Arm> = {
            let mut state = 0x9E37_79B9_u32;
            let mut arm = Arm::Pooled(0);
            let mut arms = Vec::new();
            while arms.len() < 70 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                for _ in 0..(1 + (state >> 28) % 7) {
                    arms.push(arm);
                }
                arm = if arm.is_pooled() {
                    Arm::Inline
                } else {
                    Arm::Pooled(0)
                };
            }
            arms
        };
        assert!(arms.contains(&Arm::Inline) && arms.contains(&Arm::Pooled(0)));
        let encode = |world: &World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        type Make = fn() -> World;
        let makes: [(&str, Make); 2] = [
            ("spread of bibites", || audit_world(300)),
            ("dense pile", || packed_world(300, 70.0)),
        ];
        for (label, make) in makes {
            let mut serial = make();
            serial.set_threads(1);
            let mut mixed = make();
            mixed.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            mixed.set_threads(8);
            let mut ticks = 0;
            let mut pooled_ticks = 0;
            for round in arms.chunks(10) {
                for &arm in round {
                    serial.tick();
                    mixed.tick_in(Choice { arm, timed: false });
                    ticks += 1;
                    match arm {
                        Arm::Inline => assert!(
                            mixed.sensing_pool.is_none(),
                            "{label}: an inline tick has a handle to the pool"
                        ),
                        Arm::Pooled(_) => {
                            pooled_ticks += 1;
                            assert!(
                                mixed.sensing_pool.is_some(),
                                "{label}: a tick on the pool has no pool"
                            );
                        }
                    }
                }
                let (expected, actual) = (encode(&serial), encode(&mixed));
                assert!(
                    actual == expected,
                    "{label}: the world that changes ways differs from serial after {ticks} ticks: {}",
                    first_difference(&expected, &actual)
                );
            }
            assert!(
                mixed.pass_b_lanes().ticks > 0,
                "{label}: no tick ran its sweep on the pool"
            );
            assert!(pooled_ticks > 10 && pooled_ticks < ticks - 10);
        }
    }

    /// Every level of the pool, and inline, in runs of one to seven ticks each (the levels are different stage sizes: in a world of 300
    /// bibites level 0 has only a few stages on the pool and the last level has them all), give the serial world's checkpoint byte for byte,
    /// with the sizes a world starts with.
    #[test]
    fn a_world_that_changes_level_of_the_pool_tick_by_tick_has_the_serial_worlds_results() {
        let arms: Vec<Arm> = {
            let mut state = 0x1234_5678_u32;
            let mut arms = Vec::new();
            while arms.len() < 80 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let arm = Arm::from_index((state >> 24) as usize % crate::pool_tuner::ARMS);
                for _ in 0..(1 + (state >> 8) % 4) {
                    arms.push(arm);
                }
            }
            arms
        };
        for arm in Arm::ALL {
            assert!(arms.contains(&arm), "{arm:?} is not in the sequence");
        }
        let encode = |world: &World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        type Make = fn() -> World;
        let makes: [(&str, Make); 2] = [
            ("spread of bibites", || audit_world(300)),
            ("dense pile", || packed_world(300, 70.0)),
        ];
        for (label, make) in makes {
            let mut serial = make();
            serial.set_threads(1);
            // The sizes a world starts with (measuring: the levels are made from them), the arms forced by hand.
            let mut mixed = make();
            assert!(mixed.parallel_gates().adaptive);
            mixed.set_threads(8);
            let mut ticks = 0;
            for round in arms.chunks(10) {
                for &arm in round {
                    serial.tick();
                    mixed.tick_in(Choice { arm, timed: false });
                    ticks += 1;
                    if let Arm::Pooled(level) = arm {
                        assert_eq!(
                            mixed.active_gates,
                            mixed.parallel_gates().pooled_level(level),
                            "{label}: the tick at level {level} ran with other sizes"
                        );
                        assert!(
                            mixed.sensing_pool.is_some(),
                            "{label}: no pool on level {level}"
                        );
                    } else {
                        assert!(
                            mixed.sensing_pool.is_none(),
                            "{label}: an inline tick has the pool"
                        );
                    }
                }
                let (expected, actual) = (encode(&serial), encode(&mixed));
                assert!(
                    actual == expected,
                    "{label}: the world that changes levels differs from serial after {ticks} ticks: {}",
                    first_difference(&expected, &actual)
                );
            }
            assert!(
                mixed.pass_b_lanes().ticks > 0,
                "{label}: no tick ran its sweep on the pool"
            );
        }
    }

    /// A world too small for any pool to matter (under a quarter of the sizes of the first guess) ticks inline, is not timed and is never
    /// probed: nothing is spent on looking at the pool.
    #[test]
    fn a_world_too_small_for_any_pool_ticks_inline_untimed_and_is_never_probed() {
        let mut world = audit_world(20);
        world.set_threads(8);
        assert!(world.parallel_gates().adaptive);
        let prior = world.parallel_gates().pool_prior();
        let bodies = world.bibites.len() + world.pellets.len() + world.eggs.len();
        assert!(
            world.bibites.len() < prior.bibites / 4 && bodies < prior.bodies / 4,
            "the world is not below the floor: {} bibites, {bodies} bodies",
            world.bibites.len()
        );
        world.run(80);
        let stats = world.pool_tuner_stats();
        assert_eq!(stats.inline_ticks, 80, "{stats:?}");
        assert_eq!(stats.pooled_ticks, 0, "{stats:?}");
        assert_eq!(stats.probes, 0, "{stats:?}");
        assert!(stats.inline_ns().is_none() && stats.pooled_ns().is_none());
        assert!(world.sensing_pool.is_none());
        assert_eq!(world.pass_b_lanes(), crate::body::PassBLanes::default());
    }

    /// A world above the floor times its own ticks: it starts the way its size says, looks at the other way for a block of ticks, has
    /// both means afterwards, and the results are the serial world's whichever way it runs. Changing the threads or the gates makes it
    /// start measuring again, and a world of one thread, or with fixed sizes, measures nothing.
    #[test]
    fn a_measuring_world_times_its_ticks_tries_the_other_way_and_keeps_the_results() {
        // Sizes a tenth of the reference ones: the first guess says the pool for this world and the floor is far below it.
        let gates = crate::sim::ParallelGates::SCALED.scaled_by(10);
        assert!(gates.adaptive);
        let build = |threads: usize| {
            let mut world = audit_world(300);
            world.set_parallel_gates(gates);
            world.set_threads(threads);
            world
        };
        let mut worlds = run_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("8 threads, measuring".to_owned(), build(8)),
            ],
            5,
            25,
            |_, _| {},
        );
        let stats = worlds[1].1.pool_tuner_stats();
        assert_eq!(stats.inline_ticks + stats.pooled_ticks, 125, "{stats:?}");
        assert!(
            stats.pooled_ticks > 0 && stats.inline_ticks > 0,
            "{stats:?}"
        );
        assert!(stats.probes >= 1, "{stats:?}");
        assert!(
            stats.inline_ns().is_some() && stats.pooled_ns().is_some(),
            "both ways were not measured: {stats:?}"
        );
        assert_eq!(
            worlds[0].1.pool_tuner_stats(),
            PoolTunerStats::default(),
            "a world of one thread measures nothing"
        );
        worlds[1].1.set_threads(4);
        assert_eq!(worlds[1].1.pool_tuner_stats(), PoolTunerStats::default());
        worlds[1].1.run(20);
        assert!(worlds[1].1.pool_tuner_stats().pooled_ticks > 0);
        worlds[1]
            .1
            .set_parallel_gates(crate::sim::ParallelGates::FIXED.scaled_by(10));
        worlds[1].1.run(20);
        assert_eq!(
            worlds[1].1.pool_tuner_stats(),
            PoolTunerStats::default(),
            "fixed sizes do not measure"
        );
        assert_eq!(worlds[1].1.pool_arm(), Arm::Inline);
        assert!(worlds[1].1.take_pool_switches().is_empty());
    }

    /// The stages on the pool at their smallest and emptiest: no bibite at all (only pellets), one, two, three, and a world whose
    /// bibites all die partway (the pool goes on with nothing but pellets and corpses): every checkpoint is the serial one, and
    /// from two bibites on (a chunk each) the fused body sweep runs on the pool.
    #[test]
    fn a_world_with_no_bibite_or_one_ticks_on_the_pool_as_it_does_serially() {
        for count in [0usize, 1, 2, 3] {
            let build = |threads: usize| {
                let mut world = audit_world(count);
                world.set_parallel_gates(crate::sim::ParallelGates::ALWAYS);
                world.set_threads(threads);
                world
            };
            let worlds = run_same_checkpoints(
                vec![
                    ("serial".to_owned(), build(1)),
                    ("3 threads".to_owned(), build(3)),
                    ("8 threads".to_owned(), build(8)),
                ],
                3,
                15,
                |_, _| {},
            );
            if count >= 2 {
                for (name, world) in &worlds[1..] {
                    assert!(
                        world.pass_b_lanes().ticks > 0,
                        "{count} bibites, {name}: no tick ran on the pool"
                    );
                }
            }
        }
        // Everyone dies partway.
        let build = |threads: usize| {
            let mut world = audit_world(24);
            world.set_parallel_gates(crate::sim::ParallelGates::ALWAYS);
            world.set_threads(threads);
            world.run(5);
            let ids: Vec<BibiteId> = world.bibites.ids().collect();
            for id in ids {
                assert!(world.kill_bibite(id));
            }
            world
        };
        assert_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("4 threads".to_owned(), build(4)),
            ],
            3,
            10,
        );
    }

    /// The same with a dense pile, where the narrowphase finds many contacts and the solver, the contact bookkeeping
    /// and the collision damage all work.
    #[test]
    fn parallel_stages_are_bit_identical_to_serial_in_a_dense_pile() {
        let build = |threads: usize| {
            let mut world = packed_world(400, 90.0);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            world
        };
        assert_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("4 threads".to_owned(), build(4)),
                ("8 threads".to_owned(), build(8)),
            ],
            4,
            10,
        );
    }

    /// Bodies whose boxes start at exactly the same x (a line of upright bibites) have an x order that is all ties: the
    /// sort and the sweep are cut differently in parallel, yet the contacts, the order of the solver and everything
    /// after it must come out the same.
    #[test]
    fn parallel_stages_are_bit_identical_to_serial_with_tied_positions() {
        let build = |threads: usize| {
            let mut world = lined_world(250, 1.0);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            world
        };
        assert_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("4 threads".to_owned(), build(4)),
                ("8 threads".to_owned(), build(8)),
            ],
            4,
            10,
        );
    }

    /// A world with a thousand pellets or more (the zone's biomass density raised), for the pellet stages.
    fn pelleted_world(bibites: usize) -> World {
        let mut settings = default_settings();
        settings.independents.biomass_density *= 60.0;
        audit_world_with(bibites, settings)
    }

    /// Thousands of pellets next to the bibites: the pellet bodies, the broadphase's x order and sweep, the poses and
    /// the write-back all see far more bodies than there are bibites.
    #[test]
    fn parallel_stages_are_bit_identical_to_serial_with_thousands_of_pellets() {
        assert!(
            pelleted_world(10).pellets.len() >= 1_000,
            "{} pellets",
            pelleted_world(10).pellets.len()
        );
        let build = |threads: usize| {
            let mut world = pelleted_world(150);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            world
        };
        assert_same_checkpoints(
            vec![
                ("serial".to_owned(), build(1)),
                ("3 threads".to_owned(), build(3)),
                ("8 threads".to_owned(), build(8)),
            ],
            4,
            10,
        );
    }

    /// Sets the brain outputs (and a little state: a loaded stomach, a full womb, a pheromone clock about to strike) of every
    /// bibite to one of nine hand-made profiles, shifted by `shift` so that each bibite goes through all of them over the
    /// rounds: vomiting, laying, emitting pheromone, biting, grabbing, letting go, moving and growing, or random outputs.
    fn force_pass_b_profiles(world: &mut World, shift: usize) {
        use crate::brain::{ALL_OUTPUTS, N_OUTPUTS, Output};
        use crate::organs::{PHEROMONE_EMIT_PERIOD, StomachContent, StomachMaterial};
        let ids: Vec<BibiteId> = world.bibites.ids().collect();
        for (index, id) in ids.into_iter().enumerate() {
            let Some(record) = world.bibites.get_mut(id) else {
                continue;
            };
            let mut out = [0.0f32; N_OUTPUTS];
            match (index + shift) % 9 {
                0 => {}
                1 => {
                    out[Output::Want2Eat.index()] = -0.6;
                    record.stomach.set_contents(&[StomachContent {
                        material: StomachMaterial::Plant,
                        amount: 4.0,
                        average_chunk_amount: 1.0,
                        ..StomachContent::new()
                    }]);
                    record.mouth.bite_progress = record.mouth_derived.bite_period * 2.0;
                }
                2 => {
                    out[Output::Want2Lay.index()] = 1.0;
                    out[Output::EggProduction.index()] = 1.0;
                    record.egg_layer.is_mature = true;
                    record.egg_layer.capacity = record.egg_layer.capacity.max(2);
                    record.egg_layer.progress = record.egg_layer.progress.max(1.0);
                }
                3 => {
                    out[Output::PhereOut1.index()] = 1.0;
                    out[Output::PhereOut2.index()] = 0.5;
                    out[Output::PhereOut3.index()] = 0.5;
                    record.pheromone.progress = PHEROMONE_EMIT_PERIOD - 0.01;
                }
                4 => {
                    out[Output::Want2Attack.index()] = 1.0;
                    out[Output::Want2Eat.index()] = 1.0;
                }
                5 => {
                    out[Output::Grab.index()] = 1.0;
                    out[Output::Want2Eat.index()] = 0.3;
                }
                6 => {
                    out[Output::Grab.index()] = -0.9;
                    out[Output::Want2Eat.index()] = 0.2;
                }
                7 => {
                    out[Output::Accelerate.index()] = 1.0;
                    out[Output::Rotate.index()] = 0.5;
                    out[Output::Digestion.index()] = 1.0;
                    out[Output::Want2Grow.index()] = 1.0;
                    out[Output::Want2Heal.index()] = 1.0;
                }
                _ => {
                    for (salt, value) in out.iter_mut().enumerate() {
                        let hash = (index as u64 + 1)
                            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                            .wrapping_add(
                                (salt as u64 + shift as u64 * 31) * 0xBF58_476D_1CE4_E5B9,
                            );
                        *value = ((hash >> 40) % 2001) as f32 / 1000.0 - 1.0;
                    }
                }
            }
            for output in ALL_OUTPUTS {
                record.brain.set_output_value(output, out[output.index()]);
            }
        }
    }

    /// A dense pile whose bibites go through every kind of pass-B turn (see [`force_pass_b_profiles`]): the turns that run on
    /// the pool and the turns that run in dense order, each kind of the latter, and the targets that come before and after
    /// a biter in dense order, must leave the very same world as the serial tick.
    #[test]
    fn parallel_stages_are_bit_identical_to_serial_with_every_kind_of_pass_b_turn() {
        let build = |threads: usize| {
            let mut settings = default_settings();
            // The brains never step, so the outputs set by hand stay as they are.
            settings.brain_tps = 1_000_000;
            settings.independents.limit_bibite_birth = false;
            // One bibite in three is a baby: it grows, so its size (what a biter reads) changes within the tick.
            let mut world = packed_world_of(260, 350.0, settings, |index| {
                if index % 3 == 1 {
                    crate::body::SpawnVitals::Baby
                } else {
                    crate::body::SpawnVitals::Adult
                }
            });
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            let ids: Vec<BibiteId> = world.bibites.ids().collect();
            for (index, id) in ids.into_iter().enumerate() {
                if index % 29 == 5 {
                    assert!(world.kill_bibite(id));
                }
            }
            world
        };
        let mut worlds = vec![
            ("serial".to_owned(), build(1)),
            ("4 threads".to_owned(), build(4)),
            ("8 threads".to_owned(), build(8)),
        ];
        let encode = |world: &World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        for round in 1..=9usize {
            for (_, world) in &mut worlds {
                force_pass_b_profiles(world, round);
                world.run(6);
            }
            let expected = encode(&worlds[0].1);
            for (name, world) in &worlds[1..] {
                let actual = encode(world);
                assert!(
                    actual == expected,
                    "{name} differs from serial after round {round}: {}",
                    first_difference(&expected, &actual)
                );
            }
            // What the dense-order turns saw of their targets, entry by entry (not only what came of it).
            let seen = std::mem::take(&mut worlds[0].1.bibite_scratch.snapshot_log);
            assert!(!seen.is_empty() || round == 1);
            for (name, world) in &mut worlds[1..] {
                let theirs = std::mem::take(&mut world.bibite_scratch.snapshot_log);
                assert!(
                    theirs == seen,
                    "{name}: a target looked different to a biter than in the serial tick (round {round}, {} entries against {}, first difference at entry {:?})",
                    theirs.len(),
                    seen.len(),
                    theirs.iter().zip(&seen).position(|(a, b)| a != b)
                );
            }
        }
        assert_eq!(
            worlds[0].1.pass_b_lanes(),
            crate::body::PassBLanes::default()
        );
        for (name, world) in &worlds[1..] {
            let lanes = world.pass_b_lanes();
            assert!(
                lanes.ticks > 0
                    && lanes.light > 0
                    && lanes.mouth > 0
                    && lanes.held > 0
                    && lanes.vomit > 0
                    && lanes.eggs > 0
                    && lanes.emit > 0
                    && lanes.dead > 0,
                "{name}: some kind of turn never happened: {lanes:?}"
            );
            // The heavy turns were cut in parts: the sweep took heads, and the pool took the last part of some of them.
            assert!(
                lanes.head_in_sweep > 0 && lanes.post_on_pool > 0,
                "{name}: no heavy turn was cut in parts: {lanes:?}"
            );
            assert!(!world.eggs.is_empty(), "{name}: no egg was ever laid");
        }
    }

    /// Bibites whose health is not a number starve at the end of their turn: on the pool or in dense order, the deaths must be
    /// queued in dense order after pass A's (with corpses off each death spawns a meat pile from the world's random stream, so
    /// the order shows).
    #[test]
    fn starvation_deaths_from_every_lane_come_out_in_dense_order() {
        let build = |threads: usize| {
            let mut settings = default_settings();
            settings.scenario.corpses_enabled = false;
            let mut world = packed_world_with(240, 300.0, settings);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            // Let the bodies touch first, so that some turns have mouth targets and the rest do not.
            world.run(3);
            let ids: Vec<BibiteId> = world.bibites.ids().collect();
            for (index, id) in ids.into_iter().enumerate() {
                if index % 4 == 1 {
                    world
                        .bibites
                        .get_mut(id)
                        .expect("record")
                        .body
                        .health
                        .amount = f32::NAN;
                }
            }
            world
        };
        let mut worlds = vec![
            ("serial".to_owned(), build(1)),
            ("4 threads".to_owned(), build(4)),
            ("8 threads".to_owned(), build(8)),
        ];
        let alive_before = worlds[0].1.bibites.len();
        for (_, world) in &mut worlds {
            world.run(1);
        }
        let encode = |world: &World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        let expected = encode(&worlds[0].1);
        assert!(
            worlds[0].1.bibites.len() + 40 < alive_before,
            "{} of {alive_before} left: the starving bibites did not die",
            worlds[0].1.bibites.len()
        );
        for (name, world) in &worlds[1..] {
            let actual = encode(world);
            assert!(
                actual == expected,
                "{name} differs from serial: {}",
                first_difference(&expected, &actual)
            );
            let lanes = world.pass_b_lanes();
            assert!(lanes.mouth > 0 && lanes.light > 0, "{name}: {lanes:?}");
        }
    }

    /// The starvation deaths pass A finds (a bibite that is out of energy and whose health runs out with it) are queued in dense
    /// order, whichever stretch of records they are found in. With corpses off each death spawns a meat pile from the world's
    /// random stream, so the order shows; the dense order is made different from the slot order first (removing a bibite puts
    /// the last one in its place).
    #[test]
    fn starvation_deaths_found_by_pass_a_come_out_in_dense_order() {
        let build = |threads: usize| {
            let mut settings = default_settings();
            settings.scenario.corpses_enabled = false;
            let mut world = packed_world_with(240, 300.0, settings);
            world.set_parallel_gates(crate::sim::ParallelGates::EAGER);
            world.set_threads(threads);
            world.run(3);
            let ids: Vec<BibiteId> = world.bibites.ids().collect();
            for id in ids.iter().take(12) {
                world.bibites.remove(*id);
            }
            let ids: Vec<BibiteId> = world.bibites.ids().collect();
            for (index, id) in ids.into_iter().enumerate() {
                if index % 4 == 1 {
                    let record = world.bibites.get_mut(id).expect("record");
                    record.body.energy.amount = 0.0;
                    record.body.health.amount = 1e-6;
                }
            }
            world
        };
        let mut worlds = vec![
            ("serial".to_owned(), build(1)),
            ("4 threads".to_owned(), build(4)),
            ("8 threads".to_owned(), build(8)),
        ];
        let alive_before = worlds[0].1.bibites.len();
        for (_, world) in &mut worlds {
            world.run(1);
        }
        let encode = |world: &World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        let expected = encode(&worlds[0].1);
        assert!(
            worlds[0].1.bibites.len() + 40 < alive_before,
            "{} of {alive_before} left: the starving bibites did not die",
            worlds[0].1.bibites.len()
        );
        for (name, world) in &worlds[1..] {
            let actual = encode(world);
            assert!(
                actual == expected,
                "{name} differs from serial: {}",
                first_difference(&expected, &actual)
            );
        }
    }

    /// Pellets that rest are left out of the physics step; the world must come out exactly as when the step sees every pellet
    /// every tick. Bibites wander through the pellets, eat them, and sleepers wake and fall asleep again.
    #[test]
    fn resting_pellets_leave_the_world_as_it_would_be_without_the_sleeping_layer() {
        let build = |threads: usize, gates: crate::sim::ParallelGates| {
            // Several hundred pellets, spread out enough for most of them to be left alone.
            let mut settings = default_settings();
            settings.independents.biomass_density *= 10.0;
            let mut world = audit_world_with(150, settings);
            world.set_parallel_gates(gates);
            world.set_threads(threads);
            world
        };
        // (The exact layer: the relaxed one is checked by `relaxed_resting_pellets_change_nothing_but_the_rotation_of_the_pellets`.)
        let eager = crate::sim::ParallelGates {
            settle_rotation: false,
            ..crate::sim::ParallelGates::EAGER
        };
        let worlds = run_same_checkpoints(
            vec![
                ("no sleeping".to_owned(), build(1, eager.without_sleep())),
                ("sleeping".to_owned(), build(1, eager)),
                ("sleeping, 8 threads".to_owned(), build(8, eager)),
            ],
            8,
            12,
            |world, round| {
                // Writes from outside to pellets that mostly rest: a kick, a resize, a nudge, a spin, a removal.
                if round == 3 || round == 6 {
                    let ids: Vec<PelletId> = world.pellets.ids().collect();
                    for (index, id) in ids.into_iter().enumerate() {
                        match index % 40 {
                            0 => {
                                world.pellets.set_velocity(id, Vec2::new(40.0, -25.0));
                            }
                            7 => {
                                world.pellets.add_amount(id, 5.0);
                            }
                            14 => {
                                if let Some(pos) = world.pellets.pos(id) {
                                    world.pellets.set_pos(id, pos + Vec2::new(3.0, 1.0));
                                }
                            }
                            21 => {
                                world.pellets.set_angular_velocity(id, 2.5);
                            }
                            28 => world.despawn_pellet(id),
                            _ => {}
                        }
                    }
                }
            },
        );
        assert_eq!(
            worlds[0].1.pellet_sleep_stats(),
            crate::pellet_sleep::SleepStats::default()
        );
        for (name, world) in &worlds[1..] {
            let stats = world.pellet_sleep_stats();
            assert!(
                stats.asleep > 300 && stats.slept > 300 && stats.promoted > 0 && stats.woken > 0,
                "{name}: the layer hardly did anything: {stats:?}"
            );
        }
    }

    /// The same with every kind of pass-B turn (vomited pellets that fly and settle, grabbed ones that follow a mouth,
    /// eaten and bitten ones, meat from the dead) in a pile of bibites among thousands of pellets.
    #[test]
    fn resting_pellets_leave_a_busy_world_as_it_would_be_without_the_sleeping_layer() {
        let build = |threads: usize, gates: crate::sim::ParallelGates| {
            let mut settings = default_settings();
            settings.brain_tps = 1_000_000;
            settings.independents.limit_bibite_birth = false;
            settings.independents.biomass_density *= 20.0;
            let mut world = packed_world_of(260, 350.0, settings, |index| {
                if index % 3 == 1 {
                    crate::body::SpawnVitals::Baby
                } else {
                    crate::body::SpawnVitals::Adult
                }
            });
            world.set_parallel_gates(gates);
            world.set_threads(threads);
            let ids: Vec<BibiteId> = world.bibites.ids().collect();
            for (index, id) in ids.into_iter().enumerate() {
                if index % 29 == 5 {
                    assert!(world.kill_bibite(id));
                }
            }
            world
        };
        let eager = crate::sim::ParallelGates {
            settle_rotation: false,
            ..crate::sim::ParallelGates::EAGER
        };
        let worlds = run_same_checkpoints(
            vec![
                ("no sleeping".to_owned(), build(1, eager.without_sleep())),
                ("sleeping".to_owned(), build(1, eager)),
                ("sleeping, 4 threads".to_owned(), build(4, eager)),
            ],
            9,
            6,
            force_pass_b_profiles,
        );
        for (name, world) in &worlds[1..] {
            let stats = world.pellet_sleep_stats();
            assert!(
                stats.slept > 100 && stats.promoted > 0 && stats.woken > 0,
                "{name}: the layer hardly did anything: {stats:?}"
            );
        }
    }

    /// The relaxed layer (resting pellets whose rotation only drifts by rounding noise rest, and resting pellets that touch rest together, all of them
    /// in the step whole when one of them is touched) changes nothing but the stored rotation of those pellets: with every pellet's rotation put back
    /// to zero after each round, the worlds are the same byte for byte as the world that lets no pellet sleep. Twins are put on top of every fifth
    /// pellet so that there are touching ones that settle into the noise, and bibites wander through them.
    #[test]
    fn relaxed_resting_pellets_change_nothing_but_the_rotation_of_the_pellets() {
        let build = |threads: usize, gates: crate::sim::ParallelGates| {
            let mut settings = default_settings();
            settings.independents.biomass_density *= 10.0;
            let mut world = audit_world_with(150, settings);
            world.set_parallel_gates(gates);
            world.set_threads(threads);
            world
        };
        let eager = crate::sim::ParallelGates::EAGER;
        let relaxed = crate::sim::ParallelGates {
            settle_rotation: true,
            ..eager
        };
        assert_eq!(relaxed, eager, "the gates of the pool all have the relaxed layer");
        let mut worlds = vec![
            ("no sleeping".to_owned(), build(1, eager.without_sleep())),
            ("relaxed".to_owned(), build(1, relaxed)),
            ("relaxed, 8 threads".to_owned(), build(8, relaxed)),
        ];
        let encode = |world: &World| {
            world
                .capture_checkpoint_state()
                .expect("checkpoint")
                .encode()
                .expect("encoded checkpoint")
        };
        for round in 1..=12 {
            for (_, world) in &mut worlds {
                let ids: Vec<PelletId> = world.pellets.ids().collect();
                if round == 1 {
                    for (index, id) in ids.iter().enumerate() {
                        if index % 5 == 0
                            && let Some(pos) = world.pellets.pos(*id)
                        {
                            world.pellets.spawn(crate::pellet::PelletSpawn::new(
                                crate::pellet::MaterialKind::Plant,
                                20.0,
                                pos + Vec2::new(3.0, 0.5),
                            ));
                        }
                    }
                }
                if round == 6 {
                    // Writes from outside to pellets that mostly rest: a kick, a resize, a nudge, a removal.
                    for (index, id) in ids.into_iter().enumerate() {
                        match index % 40 {
                            0 => {
                                world.pellets.set_velocity(id, Vec2::new(40.0, -25.0));
                            }
                            7 => {
                                world.pellets.add_amount(id, 5.0);
                            }
                            14 => {
                                if let Some(pos) = world.pellets.pos(id) {
                                    world.pellets.set_pos(id, pos + Vec2::new(3.0, 1.0));
                                }
                            }
                            28 => world.despawn_pellet(id),
                            _ => {}
                        }
                    }
                }
                world.run(12);
                // (Nothing reads a pellet's rotation: every one goes back to zero, here and in the reference.)
                let ids: Vec<PelletId> = world.pellets.ids().collect();
                for id in ids {
                    world.pellets.set_rotation(id, 0.0);
                }
            }
            let expected = encode(&worlds[0].1);
            for (name, world) in &worlds[1..] {
                let actual = encode(world);
                assert!(
                    actual == expected,
                    "{name} differs from {} after round {round}: {}",
                    worlds[0].0,
                    first_difference(&expected, &actual)
                );
            }
        }
        for (name, world) in &worlds[1..] {
            let stats = world.pellet_sleep_stats();
            assert!(
                stats.slept > 200 && stats.promoted > 0 && stats.woken > 0 && stats.overlapped > 5,
                "{name}: the layer hardly did anything: {stats:?}"
            );
        }
    }

    /// A world of a hundred pellets or so and a few bibites rests its pellets with the default gates (the layer starts at 64 pellets; it was 2,048 until
    /// 2026-11-23, and the scenarios with a few hundred to two thousand pellets spent most of a tick on pellets that did not move) and comes out as it
    /// would without the layer, but for the rotation of the pellets (the default layer is the relaxed one: they are put back to zero every round).
    #[test]
    fn a_small_pellet_world_rests_its_pellets_by_default() {
        let build = |gates: crate::sim::ParallelGates| {
            let mut settings = default_settings();
            settings.independents.biomass_density *= 4.0;
            let mut world = audit_world_with(24, settings);
            world.set_parallel_gates(gates);
            world.set_threads(1);
            world
        };
        let fixed = crate::sim::ParallelGates::FIXED;
        assert_eq!(fixed.sleep_pellets, 64, "the layer's default start");
        assert!(fixed.settle_rotation, "the default layer is the relaxed one");
        let mut worlds = vec![
            ("no sleeping".to_owned(), build(fixed.without_sleep())),
            ("default gates".to_owned(), build(fixed)),
        ];
        for round in 1..=6 {
            for (_, world) in &mut worlds {
                world.run(15);
                let ids: Vec<PelletId> = world.pellets.ids().collect();
                for id in ids {
                    world.pellets.set_rotation(id, 0.0);
                }
            }
            let encode = |world: &World| {
                world
                    .capture_checkpoint_state()
                    .expect("checkpoint")
                    .encode()
                    .expect("encoded checkpoint")
            };
            let expected = encode(&worlds[0].1);
            let actual = encode(&worlds[1].1);
            assert!(
                actual == expected,
                "default gates differ from no sleeping after round {round}: {}",
                first_difference(&expected, &actual)
            );
        }
        let pellets = worlds[1].1.pellets.len();
        assert!(pellets >= 64, "{pellets} pellets: the world is too small to start the layer");
        assert_eq!(
            worlds[0].1.pellet_sleep_stats(),
            crate::pellet_sleep::SleepStats::default()
        );
        let stats = worlds[1].1.pellet_sleep_stats();
        assert!(
            stats.asleep > 20 && stats.slept > 20,
            "the layer hardly did anything: {stats:?}"
        );
    }

    #[test]
    fn a_spawn_rows_path_matches_the_templates_it_can_resolve() {
        for (path, name, matches) in [
            ("Basic bibite.bb8template", "Basic bibite.bb8template", true),
            ("Basic bibite", "Basic bibite.bb8template", true),
            ("C:\\Users\\x\\Templates\\Basic bibite.bb8template", "Basic bibite.bb8template", true),
            ("/home/x/Basic bibite.bb8", "Basic bibite.bb8template", true),
            ("Basic bibite.bb8template", "Basic bibite 2.bb8template", false),
            ("Basic", "Basic bibite.bb8template", false),
            ("", "Basic bibite.bb8template", false),
            ("", "", false),
        ] {
            assert_eq!(World::template_name_matches(path, name), matches, "{path:?} / {name:?}");
        }
        // The world's lookup is the same rule, first match in list order.
        let mut world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        let template = |name: &str| {
            use crate::brain::Brain;
            use crate::genome::{Genome, GenomeMeta};
            TemplateSource::new(name, Genome::starter(), GenomeMeta::default(), Brain::starter())
        };
        world.set_templates(vec![template("A.bb8template"), template("B.bb8template"), template("B.bb8")]);
        assert_eq!(world.template_for_path("B").map(|found| found.name.as_str()), Some("B.bb8template"));
        assert_eq!(world.template_for_path("C:/x/B.bb8").map(|found| found.name.as_str()), Some("B.bb8template"));
        assert!(world.template_for_path("").is_none() && world.template_for_path("Z").is_none());
    }

    #[test]
    fn set_threads_auto_and_serial_paths() {
        let mut world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        assert_eq!(world.threads(), 1);
        world.set_threads(1);
        assert_eq!(world.threads(), 1);
        world.set_threads(0);
        assert!(world.threads() >= 1);
        world.set_threads(4);
        assert_eq!(world.threads(), 4);
        world.set_threads(1);
        assert_eq!(world.threads(), 1);
        world.tick();
    }

    #[test]
    fn generation_stats_track_live_metas_and_document_the_population_policy() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};

        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 61).expect("world");
        world.seed_scenario_world();
        let mut ids = Vec::new();
        for (index, generation) in [5u32, 10].into_iter().enumerate() {
            let meta = GenomeMeta {
                generation,
                ..GenomeMeta::default()
            };
            let mut spawn = BibiteSpawn::new(Genome::starter(), meta, Brain::starter());
            spawn.vitals = SpawnVitals::Adult;
            spawn.pos = Vec2::new(index as f32 * 10.0, 0.0);
            let id = world.bibites.spawn(spawn).expect("bibite");
            world.register_bibite(id);
            ids.push(id);
        }
        let sample = world.sample_stats();
        assert_eq!(sample.generation.gen_max, 10);
        assert!((sample.generation.gen_mean - 7.5).abs() < 1e-6);

        // Loaded/save metas are preserved by spawn_loaded_bibite.
        let saved = world.bibites.to_save(ids[1]).expect("save record");
        let mut loaded =
            World::new(default_settings(), ScenarioData::default(), 62).expect("loaded world");
        loaded.spawn_loaded_bibite(saved).expect("load bibite");
        let loaded_sample = loaded.sample_stats();
        assert_eq!(loaded_sample.generation.gen_max, 10);
        assert!((loaded_sample.generation.gen_mean - 10.0).abs() < 1e-6);

        // Policy: generation stats aggregate **living** bibites (eggs/corpses
        // excluded), matching the game's living-population tracker. Removing the
        // max-generation bibite therefore lowers the max; it is not a reset.
        world
            .bibites
            .remove(ids[1])
            .expect("remove high-gen bibite");
        world.sync_bibite_hash();
        let sample = world.sample_stats();
        assert_eq!(sample.generation.gen_max, 5);
        assert!((sample.generation.gen_mean - 5.0).abs() < 1e-6);
    }

    #[test]
    fn stats_match_game_definitions() {
        use crate::body::SpawnVitals;
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};

        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 51).expect("world");
        world.seed_scenario_world();
        let genome = Genome::starter();
        let brain = Brain::starter();
        let mut spawn = BibiteSpawn::new(genome, GenomeMeta::default(), brain);
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::new(10.0, 10.0);
        let id = world.bibites.spawn(spawn).expect("bibite");
        world.register_bibite(id);
        let settings = world.settings.clone();
        let expected = {
            let record = world.bibites.get_mut(id).expect("record");
            record.clock.time_alive = 7200.0; // 2 hours
            record.vel = Vec2::new(2.0, 0.0);
            record.propulsion.total_travel = 3600.0; // 0.5 u/s lifetime
            record.mouth.bibites_bitten = 36;
            record.mouth.total_damage_dealt = 72.0;
            record.egg_layer.n_laid = 18;
            record.fat.amount = 2.0;
            record.egg_layer.progress = 0.5;
            let extra = record.egg_layer.progress * record.egg_layer.egg_energy
                + record.fat.amount * settings.materials.fat.energy_density;
            let total_energy = record.body.total_energy(extra);
            let outputs: Vec<f32> = crate::brain::ALL_OUTPUTS
                .iter()
                .map(|output| record.brain.output_value(*output))
                .collect();
            (total_energy, outputs)
        };
        let active_nodes = {
            let record = world.bibites.get(id).expect("record");
            record.brain.used_input_mask().count_ones()
                + record.brain.used_output_mask().count_ones()
                + record.brain.hidden_count() as u32
        };
        let sample = world.sample_stats();
        assert!((sample.vitals.age_mean - 2.0).abs() < 1e-5, "age is hours");
        assert!((sample.vitals.age_max - 2.0).abs() < 1e-5);
        assert_eq!(sample.brain.nodes_max, active_nodes);
        assert!((sample.pellets.bibite_biomass - expected.0).abs() < 1e-2);
        assert!((sample.pellets.free_biomass - world.zones.free_biomass()).abs() < 1e-3);
        assert!((sample.behavior.speed_mean - 2.0).abs() < 1e-5);
        assert!((sample.behavior.speed_max - 2.0).abs() < 1e-5);
        assert!((sample.behavior.moving_frac - 1.0).abs() < 1e-5);
        assert!((sample.behavior.travel_rate_mean - 0.5).abs() < 1e-4);
        assert!((sample.behavior.bite_rate_mean - 36.0 / 7200.0).abs() < 1e-6);
        assert!((sample.behavior.damage_dealt_rate_mean - 72.0 / 7200.0).abs() < 1e-6);
        assert!((sample.behavior.egg_rate_mean - 18.0 / 7200.0).abs() < 1e-6);
        for (index, expected_output) in expected.1.iter().enumerate() {
            assert!(
                (sample.behavior.outputs[index] - expected_output).abs() < 1e-6,
                "output {index}"
            );
        }
        // Speed <= 1 counts as not moving.
        if let Some(record) = world.bibites.get_mut(id) {
            record.vel = Vec2::new(0.5, 0.0);
        }
        assert_eq!(world.sample_stats().behavior.moving_frac, 0.0);
    }

    #[test]
    #[ignore = "manual performance audit (run with --release)"]
    fn perf_sensing_tick_profile() {
        for threads in [1usize, 8] {
            let mut world = audit_world(2000);
            world.set_threads(threads);
            let mut due_ticks = 0u64;
            let mut due_total = 0u64;
            let mut sensing_sum = 0u64;
            let mut sensing_due_sum = 0u64;
            let mut zero_ticks = 0u64;
            for _ in 0..100 {
                let mut due = 0u64;
                for id in world.bibites.ids() {
                    if let Some(record) = world.bibites.get(id) {
                        if bibite_senses_due(record, &world.settings) {
                            due += 1;
                        }
                    }
                }
                world.reset_phase_timings();
                world.tick();
                let nanos = world.phase_timings().nanos(crate::sim::Phase::Sensing);
                sensing_sum += nanos;
                due_total += due;
                if due > 0 {
                    due_ticks += 1;
                    sensing_due_sum += nanos;
                } else {
                    zero_ticks += 1;
                }
            }
            let ticks = 100.0;
            println!(
                "threads={threads}: due_ticks={due_ticks} zero_ticks={zero_ticks} mean_due={:.1} avg_sensing={:.3} ms due_tick_sensing={:.3} ms per_due={:.3} us pop={} pellets={}",
                due_total as f64 / ticks,
                sensing_sum as f64 / ticks / 1e6,
                sensing_due_sum as f64 / due_ticks.max(1) as f64 / 1e6,
                sensing_due_sum as f64 / due_total.max(1) as f64 / 1e3,
                world.bibites.len(),
                world.pellets.len()
            );
            // Per-bibite FOV microbenchmark on a due bibite (query + accumulate).
            if let Some(due_id) = world.bibites.ids().find(|id| {
                world
                    .bibites
                    .get(*id)
                    .is_some_and(|r| bibite_senses_due(r, &world.settings))
            }) {
                let record = world.bibites.get(due_id).expect("record").clone();
                let params = FovParams {
                    view_radius: record.genome.gene(crate::genome::Gene::ViewRadius)
                        * record.derived.d1_size,
                    view_angle: record.genome.gene(crate::genome::Gene::ViewAngle),
                    herd_separation_distance: 0.0,
                    separation_weight: 0.0,
                    cohesion_weight: 0.0,
                    alignment_weight: 0.0,
                };
                let mask = FovMask::from_brain(&record.brain);
                let up = Vec2::Y.rotate(record.rot);
                let mut scratch = QueryScratch::new();
                let rounds = 1000;
                let start = std::time::Instant::now();
                for _ in 0..rounds {
                    let mut sensor = senses::FovSensor::new(record.pos, up, params, mask);
                    let hits = world.hash.query_circle_unsorted(
                        record.pos,
                        params.view_radius,
                        ClassMask::PELLET,
                        &mut scratch,
                    );
                    for hit in hits.hits {
                        let pellet_id = PelletId::from_handle(hit.handle);
                        if let Some(view) = world.pellets.get(pellet_id) {
                            if !view.held && view.amount >= 0.01 {
                                match view.material {
                                    MaterialKind::Plant if mask.plant => {
                                        sensor.add_plant(view.pos, view.radius, view.amount);
                                    }
                                    MaterialKind::Meat if mask.meat => {
                                        sensor.add_meat(view.pos, view.radius, view.amount);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    std::hint::black_box(sensor.finish());
                }
                println!(
                    "threads={threads}: due-bibite FOV {:.3} us",
                    start.elapsed().as_secs_f64() * 1e6 / f64::from(rounds)
                );
            }
            let start = std::time::Instant::now();
            for _ in 0..100 {
                let hash = world.hash.take_same_layout();
                world.hash = hash;
            }
            println!(
                "threads={threads}: hash take/restore {:.3} ms",
                start.elapsed().as_secs_f64() * 1000.0 / 100.0
            );
            let start = std::time::Instant::now();
            for _ in 0..100 {
                let mut due = 0u32;
                for id in world.bibites.ids() {
                    if let Some(record) = world.bibites.get(id) {
                        if bibite_senses_due(record, &world.settings) {
                            due += 1;
                        }
                    }
                }
                std::hint::black_box(due);
            }
            println!(
                "threads={threads}: prefilter {:.3} ms/tick",
                start.elapsed().as_secs_f64() * 1000.0 / 100.0
            );
            let mut phero_needs = 0u32;
            let mut sense_due = 0u32;
            for id in world.bibites.ids() {
                if let Some(record) = world.bibites.get(id) {
                    let brain_tick =
                        record.throttle.brain_counter + 1 >= world.settings.brain_tps.max(1);
                    if brain_tick && record.brain.needs_pheromone_sensing() {
                        phero_needs += 1;
                    }
                    if brain_tick
                        && record.throttle.sense_counter + 1
                            >= world.settings.vision_sense_updates.max(1)
                    {
                        sense_due += 1;
                    }
                }
            }
            println!(
                "threads={threads}: sense_due={sense_due} phero_needs={phero_needs} spots={}",
                world.pheromones.len()
            );
            let mut scratch = QueryScratch::new();
            let start = std::time::Instant::now();
            let mut hits_sum = 0usize;
            for id in world.bibites.ids() {
                if let Some(record) = world.bibites.get(id) {
                    let hits = world.hash.query_circle_unsorted(
                        record.pos,
                        180.0,
                        ClassMask::PELLET,
                        &mut scratch,
                    );
                    hits_sum += hits.hits.len();
                }
            }
            println!(
                "threads={threads}: query-only {:.3} ms / {} bibites, avg_hits={:.1}",
                start.elapsed().as_secs_f64() * 1000.0,
                world.bibites.len(),
                hits_sum as f64 / world.bibites.len().max(1) as f64
            );
        }
    }

    #[test]
    #[ignore = "manual performance audit (run with --release)"]
    fn perf_sensing_breakdown() {
        let force_due = |world: &mut World| {
            let brain_tps = world.settings.brain_tps;
            let sense_updates = world.settings.vision_sense_updates;
            let mut ids = Vec::new();
            ids.extend(world.bibites.ids());
            for id in ids {
                if let Some(record) = world.bibites.get_mut(id) {
                    record.throttle.brain_counter = brain_tps.saturating_sub(1);
                    record.throttle.sense_counter = sense_updates;
                }
            }
        };
        let mut world = audit_world(2000);
        let mut mask_plant = 0;
        let mut mask_meat = 0;
        let mut mask_bibite = 0;
        let mut phero = 0;
        for (_, record) in world.bibites.iter() {
            let mask = FovMask::from_brain(&record.brain);
            mask_plant += u32::from(mask.plant);
            mask_meat += u32::from(mask.meat);
            mask_bibite += u32::from(mask.bibite);
            phero += u32::from(record.brain.needs_pheromone_sensing());
        }
        println!(
            "N=2000 masks: plant={mask_plant} meat={mask_meat} bibite={mask_bibite} phero={phero} spots={}",
            world.pheromones.len()
        );
        for threads in [1usize, 8] {
            world.set_threads(threads);
            force_due(&mut world);
            world.reset_phase_timings();
            let start = std::time::Instant::now();
            world.step_sensing();
            println!(
                "N=2000 threads={threads} forced-due sensing: {:.3} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        // Herding off (bibite category likely only queried for the herd).
        world.set_threads(1);
        let mut ids = Vec::new();
        ids.extend(world.bibites.ids());
        for id in ids {
            if let Some(record) = world.bibites.get_mut(id) {
                record.brain.set_herding_enabled(false);
            }
        }
        force_due(&mut world);
        let start = std::time::Instant::now();
        world.step_sensing();
        println!(
            "N=2000 herding-off forced-due sensing: {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        // No pheromone spots.
        world.pheromones = PheromoneStore::default();
        force_due(&mut world);
        let start = std::time::Instant::now();
        world.step_sensing();
        println!(
            "N=2000 herding-off/no-spots sensing: {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        // Query-only vs full accumulate cost (one serial pass, all due).
        world.set_threads(1);
        let mut scratch = QueryScratch::new();
        let start = std::time::Instant::now();
        for (_, record) in world.bibites.iter() {
            let hits = world
                .hash
                .query_circle(record.pos, 180.0, ClassMask::PELLET, &mut scratch);
            std::hint::black_box(hits.len());
        }
        println!(
            "N=2000 plant query-only serial pass: {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        // Sort cost: sort 2000 buffers of 128 synthetic hits.
        let start = std::time::Instant::now();
        let mut sort_scratch = QueryScratch::with_capacity(128);
        for round in 0..2000u32 {
            sort_scratch.begin();
            for index in 0..128u32 {
                let _ = sort_scratch.push(QueryHit::new(
                    Handle::new(index ^ round, 0),
                    SpatialClass::Pellet,
                ));
            }
            std::hint::black_box(sort_scratch.finish().len());
        }
        println!(
            "N=2000 sort-128-hit buffers: {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        // Pool activation overhead: 8 no-op jobs per activation.
        world.set_threads(8);
        if let Some(pool) = world.sensing_pool.as_ref() {
            let start = std::time::Instant::now();
            for _ in 0..1000 {
                let mut jobs: Vec<PoolJob> = Vec::with_capacity(8);
                for _ in 0..8 {
                    jobs.push(Box::new(|| {}));
                }
                pool.execute(jobs);
            }
            println!(
                "pool 8 no-op jobs: {:.1} us/activation",
                start.elapsed().as_secs_f64() * 1e6 / 1000.0
            );
        }
    }

    #[test]
    fn pellet_producers_skip_source_free_worlds_but_run_for_sources() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};

        let mut world =
            World::new(default_settings(), scenario_with_zone_group(), 77).expect("world");
        world.seed_scenario_world();
        assert!(!world.has_sources);
        let before = world.pellets.len();
        world.step_pellet_producers(1.0);
        assert_eq!(world.pellets.len(), before, "source-free world must skip");

        let meta = GenomeMeta {
            is_source: true,
            ..GenomeMeta::default()
        };
        let mut spawn = BibiteSpawn::new(Genome::starter(), meta, Brain::starter());
        spawn.vitals = SpawnVitals::Adult;
        spawn.pos = Vec2::ZERO;
        let id = world.bibites.spawn(spawn).expect("source bibite");
        world.register_bibite(id);
        assert!(world.has_sources);
        let before = world.pellets.len();
        world.step_pellet_producers(1.0);
        assert!(
            world.pellets.len() > before,
            "source bibite must emit plant pellets"
        );
    }

    #[test]
    #[ignore = "manual performance audit (run with --release)"]
    fn perf_scale_sizes() {
        use crate::body::BibiteRecord;
        use crate::brain::Brain;
        use core::mem::size_of;
        println!(
            "sizes: BibiteRecord={} Brain={} SenseOutputs={} SteeringInputs={} Entry={} QueryHit={}",
            size_of::<BibiteRecord>(),
            size_of::<Brain>(),
            size_of::<SenseOutputs>(),
            size_of::<SteeringInputs>(),
            size_of::<Entry>(),
            size_of::<QueryHit>(),
        );
        println!(
            "sizes: BibiteEvent={} PelletRecord={} EggRecord={} StatsSample={}",
            size_of::<BibiteEvent>(),
            size_of::<crate::pellet::PelletView>(),
            size_of::<crate::body::EggRecord>(),
            size_of::<StatsSample>(),
        );
        let brain = Brain::starter();
        println!(
            "starter brain: nodes={} syns={}",
            brain.node_count(),
            brain.synapse_count()
        );
    }

    #[test]
    #[ignore = "manual performance audit (run with --release)"]
    fn perf_phase_audit() {
        for (count, ticks) in [(500usize, 50u64), (2000, 100)] {
            let mut world = audit_world(count);
            world.reset_phase_timings();
            let start = std::time::Instant::now();
            world.run(ticks);
            print_phase_table(&format!("N={count}"), &world, start.elapsed());
        }
        {
            let mut world = audit_world(2000);
            world.set_threads(8);
            world.reset_phase_timings();
            let start = std::time::Instant::now();
            world.run(100);
            print_phase_table("N=2000 threads=8", &world, start.elapsed());
        }
        // Non-physics-only audit for the 5000-bibite case (physics is being fixed
        // separately by body-dev; a full tick is too slow to measure here).
        let mut world = audit_world(5000);
        let dt = world.clock.dt();
        let start = std::time::Instant::now();
        world.step_sensing();
        let sensing = start.elapsed();
        let start = std::time::Instant::now();
        world.step_events_and_stats(dt);
        let events = start.elapsed();
        let start = std::time::Instant::now();
        world.sync_all_hashes();
        let hash = start.elapsed();
        println!(
            "N=5000 non-physics: sensing={:.3} ms, events-stats={:.3} ms, hash-sync={:.3} ms",
            sensing.as_secs_f64() * 1000.0,
            events.as_secs_f64() * 1000.0,
            hash.as_secs_f64() * 1000.0
        );
        println!(
            "N=5000 per-bibite: sensing={:.3} us, events-stats={:.3} us, hash-sync={:.3} us",
            sensing.as_secs_f64() * 1e6 / 5000.0,
            events.as_secs_f64() * 1e6 / 5000.0,
            hash.as_secs_f64() * 1e6 / 5000.0
        );
        // Second sensing pass (sense counters now reset) measures the cheap path.
        let start = std::time::Instant::now();
        world.step_sensing();
        println!(
            "N=5000 second sensing (no sense due): {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    #[test]
    fn ui_template_spawn_stats_and_selection_helpers() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world = World::new(default_settings(), ScenarioData::default(), 7).expect("world");
        let template = TemplateSource::new(
            "ui-template",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        );
        let first = world
            .spawn_template_at(
                &template,
                Vec2::new(11.0, -7.0),
                Some(0.5),
                GrowthAtSpawn::Adult,
                Tagging::NoTagging,
                "",
            )
            .expect("spawn adult");
        let first_id = match first {
            TemplateSpawn::Bibite(id) => id,
            TemplateSpawn::Egg(_) => panic!("adult template spawned an egg"),
        };
        let record = world.bibites.get(first_id).expect("record");
        assert!((record.pos - Vec2::new(11.0, -7.0)).length() < 1e-4);
        assert!((record.rot - 0.5).abs() < 1e-4);
        let stats = record.stats_view();
        assert_eq!(stats.generation, 0);
        assert!(stats.body_length > 0.0 && stats.mass > 0.0);
        assert!(!stats.dead);
        let second = world
            .spawn_template_at(
                &template,
                Vec2::new(-20.0, 3.0),
                Some(0.0),
                GrowthAtSpawn::Baby,
                Tagging::CustomTagging,
                "ui-tag",
            )
            .expect("spawn baby");
        let second_id = match second {
            TemplateSpawn::Bibite(id) => id,
            TemplateSpawn::Egg(_) => panic!("baby template spawned an egg"),
        };
        assert_eq!(
            world
                .bibites
                .get(second_id)
                .unwrap()
                .meta
                .species_tag
                .as_deref(),
            Some("ui-tag")
        );
        // Kill the first and verify cycle/selection helpers skip it.
        world.bibites.get_mut(first_id).unwrap().body.dead = true;
        assert_eq!(world.live_bibite_ids(), vec![second_id]);
        assert_eq!(world.next_live_bibite(None), Some(second_id));
        assert_eq!(world.next_live_bibite(Some(second_id)), Some(second_id));
        assert_eq!(world.previous_live_bibite(Some(second_id)), Some(second_id));
        assert_eq!(world.oldest_live_bibite(), Some(second_id));
        assert_eq!(world.highest_generation_live_bibite(), Some(second_id));
        assert_eq!(
            world.nearest_live_bibite(Vec2::new(-20.0, 3.0)),
            Some(second_id)
        );
        // Egg mode: template placed as an incubating egg with LayBodyEnergy.
        let egg = world
            .spawn_template_at(
                &template,
                Vec2::new(5.0, 5.0),
                None,
                GrowthAtSpawn::Egg,
                Tagging::NoTagging,
                "",
            )
            .expect("spawn egg");
        let egg_id = match egg {
            TemplateSpawn::Egg(id) => id,
            TemplateSpawn::Bibite(_) => panic!("egg growth spawned a live bibite"),
        };
        let egg_stats = world
            .eggs
            .get(egg_id)
            .expect("egg")
            .stats_view(&world.settings);
        assert!(egg_stats.energy > 0.0 && egg_stats.egg_mass > 0.0);
        assert!(egg_stats.egg_area > 0.0 && egg_stats.hatch_time > 0.0);
        // The status bars of a selected egg: spent share of the required energy over what is stored.
        assert!(egg_stats.total_required_energy > 0.0);
        assert!(egg_stats.total_required_energy <= 2.0 * egg_stats.energy);
        assert_eq!(egg_stats.free_energy, egg_stats.energy);
    }
    /// "Mutate Genes: All Genes ... spawning wayyy too big of bibites": the size a growth mode asks for is read from the TEMPLATE's genes
    /// (`SpawnBibiteFromTemplate`: `StartBodyAtGrowthAndNormalize(GrowthAtMature(template.genes))`), however every gene was randomised. The spawned
    /// genome's own `GrowthAtMature` is not the game's answer: a random womb share makes it reach the hundreds.
    #[test]
    fn an_all_genes_spawn_takes_its_size_from_the_templates_genes() {
        use crate::brain::Brain;
        let mut world = World::new(default_settings(), ScenarioData::default(), 5).expect("world");
        let template = TemplateSource::new("all-genes", Genome::starter(), GenomeMeta::default(), Brain::starter());
        let wanted_adult = template.genome.growth_at_mature();
        let wanted_baby = template.genome.growth_at_birth();
        let mut own_growth_differed = false;
        for _ in 0..40 {
            for (mode, wanted) in [(GrowthAtSpawn::Adult, wanted_adult), (GrowthAtSpawn::Baby, wanted_baby), (GrowthAtSpawn::Elder, 2.0 * wanted_adult)] {
                let id = world
                    .spawn_bibite_from_template(&template, mode, Tagging::NoTagging, "", Vec2::ZERO, RandomizeGenes::AllGenes)
                    .expect("spawn");
                let record = world.bibites.get(id).expect("record");
                assert!((record.growth.growth - wanted).abs() < 1e-3 * wanted.max(1.0), "{mode:?}: growth {} for the template's {wanted}", record.growth.growth);
                own_growth_differed |= (record.genome.growth_at_mature() - wanted_adult).abs() > 1.0;
            }
        }
        // The test would not notice the old behaviour if no random genome had a growth of its own to go wrong with.
        assert!(own_growth_differed, "forty random genomes all had the template's growth at mature");
    }

    /// `BibiteGenes.StartGenesFromTemplate` tags what it spawns (`speciesTag = tagStyle switch ...`): the species name, `Bibite` and a random number,
    /// the custom text, or no tag. Random and Species tagging did nothing in a live spawn or an egg (only None and Custom were handled). The original's
    /// live All Genes spawn never calls it (it only copies the generation) and stays untagged; this port tags that one too, on the player's request.
    #[test]
    fn a_template_spawn_is_tagged_the_way_start_genes_from_template_tags_it() {
        use crate::brain::Brain;
        let mut world = World::new(default_settings(), ScenarioData::default(), 9).expect("world");
        let template = TemplateSource::new("tagged template", Genome::starter(), GenomeMeta::default(), Brain::starter());
        let live_tag = |world: &mut World, tagging: Tagging, custom: &str, randomize: RandomizeGenes| {
            let id = world
                .spawn_bibite_from_template(&template, GrowthAtSpawn::Adult, tagging, custom, Vec2::ZERO, randomize)
                .expect("spawn");
            let record = world.bibites.get(id).expect("record");
            (record.meta.species_tag.clone(), record.meta.species)
        };
        let egg_tag = |world: &mut World, tagging: Tagging, custom: &str, randomize: RandomizeGenes| {
            let id = match world
                .spawn_template_at_randomized(&template, Vec2::ZERO, None, GrowthAtSpawn::Egg, tagging, custom, randomize)
                .expect("spawn egg")
            {
                TemplateSpawn::Egg(id) => id,
                TemplateSpawn::Bibite(_) => panic!("an egg was asked for"),
            };
            let egg = world.eggs.get(id).expect("egg");
            (egg.meta.species_tag.clone(), egg.meta.species)
        };
        // None and Custom.
        assert_eq!(live_tag(&mut world, Tagging::NoTagging, "", RandomizeGenes::No).0, None);
        assert_eq!(live_tag(&mut world, Tagging::CustomTagging, "mine", RandomizeGenes::No).0.as_deref(), Some("mine"));
        assert_eq!(egg_tag(&mut world, Tagging::CustomTagging, "mine", RandomizeGenes::No).0.as_deref(), Some("mine"));
        // Random: `Bibite` and a number, a different one each time.
        let mut seen = Vec::new();
        for _ in 0..4 {
            for tag in [
                live_tag(&mut world, Tagging::RandomTagging, "", RandomizeGenes::No).0,
                egg_tag(&mut world, Tagging::RandomTagging, "", RandomizeGenes::NormalMutations).0,
            ] {
                let tag = tag.expect("a random tag");
                let number = tag.strip_prefix("Bibite").expect("starts with Bibite");
                assert!(!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()), "{tag}");
                assert!(number.parse::<u64>().expect("a number") < i32::MAX as u64, "{tag}");
                seen.push(tag);
            }
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 8, "every random tag is its own: {seen:?}");
        // Species: the name of the species the spawn joined.
        let (tag, species) = live_tag(&mut world, Tagging::SpeciesTagging, "", RandomizeGenes::No);
        let name = world.species.get(species.expect("a species")).expect("record").name();
        assert_eq!(tag.as_deref(), Some(name.as_str()));
        let (tag, species) = egg_tag(&mut world, Tagging::SpeciesTagging, "", RandomizeGenes::No);
        let name = world.species.get(species.expect("a species")).expect("record").name();
        assert_eq!(tag.as_deref(), Some(name.as_str()));
        // All Genes plus a tag style tags too (the original leaves a live All Genes spawn untagged; the player wants it tagged), live and egg alike.
        let all_genes_tag = live_tag(&mut world, Tagging::RandomTagging, "", RandomizeGenes::AllGenes).0.expect("a random tag on an All Genes spawn");
        assert!(all_genes_tag.starts_with("Bibite"), "{all_genes_tag}");
        assert_eq!(live_tag(&mut world, Tagging::CustomTagging, "mine", RandomizeGenes::AllGenes).0.as_deref(), Some("mine"));
        assert_eq!(live_tag(&mut world, Tagging::NoTagging, "", RandomizeGenes::AllGenes).0, None);
        assert!(egg_tag(&mut world, Tagging::RandomTagging, "", RandomizeGenes::AllGenes).0.is_some());
    }

    /// A placed egg takes the spawn row's "Mutate Genes" choice (`SpawnEggFromTemplate`): AllGenes replaces every gene, the others mutate as an egg always does.
    #[test]
    fn an_all_genes_egg_has_every_gene_redrawn() {
        use crate::brain::Brain;
        let mut world = World::new(default_settings(), ScenarioData::default(), 13).expect("world");
        let template = TemplateSource::new("egg template", Genome::starter(), GenomeMeta::default(), Brain::starter());
        let id = match world
            .spawn_template_at_randomized(&template, Vec2::ZERO, None, GrowthAtSpawn::Egg, Tagging::NoTagging, "", RandomizeGenes::AllGenes)
            .expect("egg")
        {
            TemplateSpawn::Egg(id) => id,
            TemplateSpawn::Bibite(_) => panic!("an egg was asked for"),
        };
        let genome = &world.eggs.get(id).expect("egg").genome;
        let moved = crate::genome::ALL_GENES.iter().filter(|gene| (genome.gene(**gene) - template.genome.gene(**gene)).abs() > 1e-4).count();
        assert!(moved >= 25, "{moved} of {} genes differ from the template's", crate::genome::N_GENES);
    }

    #[test]
    fn inspector_actions_tags_force_lay_revive_and_save_views() {
        use crate::brain::Brain;
        use crate::genome::{Genome, GenomeMeta};
        let mut world = World::new(default_settings(), ScenarioData::default(), 21).expect("world");
        let template = TemplateSource::new(
            "inspector",
            Genome::starter(),
            GenomeMeta::default(),
            Brain::starter(),
        );
        let id = match world
            .spawn_template_at(
                &template,
                Vec2::new(3.0, 4.0),
                Some(0.0),
                GrowthAtSpawn::Adult,
                Tagging::NoTagging,
                "",
            )
            .expect("spawn")
        {
            TemplateSpawn::Bibite(id) => id,
            TemplateSpawn::Egg(_) => panic!("adult expected"),
        };
        // Stats/EDIT/SAVE views.
        let stats = world.bibite_stats(id).expect("stats");
        assert_eq!(stats.generation, 0);
        assert!(stats.body_length > 0.0 && stats.mass > 0.0);
        let save = world.bibite_save(id).expect("save snapshot");
        assert_eq!(save.meta.generation, 0);
        assert!(!world.bibite_template_source(id).unwrap().name.is_empty());
        // Tag set/clear + manager summaries.
        assert!(world.set_bibite_species_tag(id, Some(" ui-tag ")));
        assert_eq!(
            world.bibite_stats(id).unwrap().species_tag.as_deref(),
            Some("ui-tag")
        );
        assert_eq!(world.bibites_with_tag("ui-tag"), vec![id]);
        assert_eq!(world.species_tags(), vec!["ui-tag".to_owned()]);
        let summary = world.tag_summaries().pop().expect("tag summary");
        assert_eq!((summary.bibites, summary.eggs), (1, 0));
        assert!(summary.energy > 0.0);
        assert!(world.set_bibite_species_tag(id, None));
        assert!(world.species_tags().is_empty());
        // Forced lay: one completed egg in the womb.
        {
            let record = world.bibites.get_mut(id).expect("record");
            record.egg_layer.is_mature = true;
            record.egg_layer.progress = 1.0;
        }
        assert_eq!(world.force_lay_eggs(id), 1);
        let egg_id = world.eggs.ids().next().expect("laid egg");
        let egg_stats = world.egg_stats(egg_id).expect("egg stats");
        assert!(egg_stats.energy > 0.0 && egg_stats.egg_area > 0.0);
        let egg_save = world.egg_save(egg_id).expect("egg save");
        assert_eq!(egg_save.meta.parent1, Some(id));
        assert!(!world.egg_template_source(egg_id).unwrap().name.is_empty());
        // Tag manager includes eggs.
        assert!(world.set_egg_species_tag(egg_id, Some("ui-tag")));
        let summary = world.tag_summaries().pop().expect("tag summary");
        assert_eq!((summary.bibites, summary.eggs), (0, 1));
        assert!(summary.energy > 0.0);
        assert_eq!(world.eggs_with_tag("ui-tag"), vec![egg_id]);
        // Revive: corpse restored to full health/energy and re-registered.
        {
            let record = world.bibites.get_mut(id).expect("record");
            record.body.dead = true;
            record.is_corpse = true;
            record.body.dying = true;
            record.body.health.amount = 0.0;
            record.body.energy.amount = 0.0;
            record.body.corpse_decay = 0.5;
        }
        assert!(world.revive_bibite(id));
        let record = world.bibites.get(id).expect("revived");
        assert!(!record.body.dead && !record.is_corpse && !record.body.dying);
        // The first step of `Revival` ran at once: health follows the decay it started from, the decay has begun to fall.
        assert!((record.body.health.amount - record.body.health.max_amount * (1.0 - 0.5 / 1.1)).abs() < 1e-3);
        assert_eq!(record.body.energy.amount, record.body.energy.max_amount);
        assert!((record.body.corpse_decay - (0.5 - 1.0 / 150.0)).abs() < 1e-6);
        assert!(
            !world.revive_bibite(id),
            "revive is idempotent for live bibites"
        );
    }

    // ----- zone groups: membership, generation, editing, saving -----

    fn group_world(groups: usize, explicit: usize) -> World {
        use crate::settings::{ZoneGroupSettings, ZoneSettings};
        let mut scenario = ScenarioData::default();
        for index in 0..explicit {
            let mut zone = ZoneSettings::game_defaults();
            zone.name = format!("explicit {index}");
            scenario.zones.push(zone);
        }
        for index in 0..groups {
            let mut group = ZoneGroupSettings::game_defaults();
            group.name = format!("Pockets{index}");
            group.zone_count = 3 + index as i32;
            scenario.zone_groups.push(group);
        }
        World::new(default_settings(), scenario, 5).expect("world")
    }

    #[test]
    fn zones_of_a_group_follow_the_explicit_ones_and_know_their_group() {
        let world = group_world(2, 2);
        let groups: Vec<Option<u32>> = world.zones.zones.iter().map(|zone| zone.group).collect();
        assert_eq!(
            groups,
            vec![None, None, Some(0), Some(0), Some(0), Some(1), Some(1), Some(1), Some(1)]
        );
        assert_eq!(world.zones.explicit_count(), 2);
        assert_eq!(world.zones.group_span(0), (2, 3));
        assert_eq!(world.zones.group_span(1), (5, 4));
        // The ids are the list positions and the names `<group>_<i>`.
        for (index, zone) in world.zones.zones.iter().enumerate() {
            assert_eq!(zone.config.runtime_zone_id() as usize, index);
        }
        assert_eq!(world.zones.zones[3].config.name, "Pockets0_1");
        assert_eq!(world.zones.zones[8].config.name, "Pockets1_3");
        // The groups keep no copy of their zones.
        assert!(world.zone_groups.iter().all(|group| group.zones.is_empty()));
    }

    #[test]
    fn a_new_explicit_zone_goes_in_front_of_the_groups_zones_and_everything_follows() {
        use crate::pellet::{MaterialKind, PelletSpawn};
        use crate::settings::ZoneSettings;
        let mut world = group_world(1, 1);
        // A pellet of the last group zone and a tower attached to it, a spawn row limited to it.
        let last = world.zones.zones.len() - 1;
        let owner = world.zones.zones[last].id;
        let pellet = world
            .spawn_pellet(PelletSpawn {
                material: MaterialKind::Plant,
                amount: 6.0,
                pos: Vec2::ZERO,
                velocity: Vec2::ZERO,
                rotation: 0.0,
                angular_velocity: 0.0,
                owner_zone: Some(owner),
                held: false,
            })
            .expect("spawn");
        let mut tower = crate::tools::RadTower::new(1, Vec2::ZERO, 2000.0);
        tower.base.attached_zone = Some(owner);
        let tool = world.tools.add_rad_tower(tower);
        let mut row = crate::settings::BibiteSpawn::game_defaults();
        row.spawn_zone = Some(last as u32);
        world.bibite_spawns.push(row);
        // The template of the group follows its zone 1 as a target.
        world.zone_groups[0].template.target = Some(1);

        let mut zone = ZoneSettings::game_defaults();
        zone.name = "new".to_owned();
        let id = world.add_zone(zone).expect("added");
        assert_eq!(world.zones.index_of(id), Some(1), "after the explicit zone, before the group's");
        assert_eq!(world.zones.zones[1].config.name, "new");
        assert_eq!(world.zones.zones[1].group, None);
        assert_eq!(world.zones.explicit_count(), 2);
        let moved = world.zones.zones.len() - 1;
        assert_eq!(moved, last + 1);
        assert_eq!(world.zones.zones[moved].group, Some(0));
        // The pellet still belongs to the same zone (now one place later), the tower is attached to it, the row and the target follow.
        assert_eq!(world.pellets.owner_zone(pellet), Some(Some(world.zones.zones[moved].id)));
        assert_eq!(world.zones.zones[moved].owned_pellets, vec![pellet]);
        assert_eq!(
            world.tools.get(tool).expect("tool").base().attached_zone,
            Some(world.zones.zones[moved].id)
        );
        assert_eq!(world.bibite_spawns[0].spawn_zone, Some(moved as u32));
        assert_eq!(world.zone_groups[0].template.target, Some(2));
    }

    #[test]
    fn adding_and_removing_a_group_generates_and_destroys_its_zones() {
        use crate::settings::ZoneGroupSettings;
        let mut world = group_world(1, 1);
        let before = world.zones.zones.len();
        let mut group = ZoneGroupSettings::game_defaults();
        group.name = "Pockets1".to_owned();
        group.zone_count = 4;
        let index = world.add_zone_group(group).expect("group");
        assert_eq!(index, 1);
        assert_eq!(world.zones.zones.len(), before + 4);
        assert_eq!(world.zones.group_span(1), (before, 4));
        assert_eq!(world.zones.zones[before + 2].config.name, "Pockets1_2");
        // Removing the first group takes its zones and moves the second one up.
        let mapping = world.remove_zone_group(0).expect("removed");
        assert_eq!(world.zone_groups.len(), 1);
        assert_eq!(world.zone_groups[0].name, "Pockets1");
        assert_eq!(world.zones.zones.len(), 1 + 4);
        assert!(world.zones.zones[1..].iter().all(|zone| zone.group == Some(0)));
        assert_eq!(mapping[0], Some(0));
        assert_eq!(mapping[1], None);
        assert_eq!(mapping[before], Some(1));
        assert!(world.remove_zone_group(5).is_err());
    }

    #[test]
    fn editing_a_group_renames_regenerates_and_applies_its_template() {
        let mut world = group_world(1, 0);
        let sim = world.bounds.sim_size();
        // A new name renames the zones; they stay where they are.
        let places: Vec<(f32, f32)> = world
            .zones
            .zones
            .iter()
            .map(|zone| (zone.config.pos_x, zone.config.pos_y))
            .collect();
        let changed = world
            .edit_zone_group(0, |group| group.name = "Lakes".to_owned())
            .expect("edit");
        assert!(changed.is_none(), "no zone was generated again");
        assert_eq!(world.zones.zones[1].config.name, "Lakes_1");
        let after: Vec<(f32, f32)> = world
            .zones
            .zones
            .iter()
            .map(|zone| (zone.config.pos_x, zone.config.pos_y))
            .collect();
        assert_eq!(places, after);
        // A template change reaches every zone but not their names or places.
        world
            .edit_zone_group(0, |group| {
                group.template.fertility = 7.0;
                group.template.radius_relative = 0.1;
            })
            .expect("edit");
        for (zone, place) in world.zones.zones.iter().zip(&places) {
            assert_eq!(zone.config.fertility, 7.0);
            assert_eq!(zone.config.radius_relative, 0.1);
            assert_eq!((zone.config.pos_x, zone.config.pos_y), *place);
        }
        // A new count generates the zones again; the count follows the density and the other way round.
        let mapping = world
            .edit_zone_group(0, |group| group.zone_count = 5)
            .expect("edit")
            .expect("generated again");
        assert_eq!(world.zones.zones.len(), 5);
        assert!(mapping.iter().all(Option::is_none));
        let density = world.zone_groups[0].zone_density;
        assert!((density - 5.0 / (4.0 * sim * sim / 1_000_000.0)).abs() < 1e-3);
        world
            .edit_zone_group(0, |group| {
                group.scale_with_sim = true;
                group.zone_density = 0.5;
            })
            .expect("edit");
        let wanted = world.zone_groups[0].count(sim) as usize;
        assert_eq!(world.zones.zones.len(), wanted);
        assert_eq!(world.zone_groups[0].zone_count as usize, wanted);
        // Rolling the group again keeps the number and moves the zones.
        let first = (world.zones.zones[0].config.pos_x, world.zones.zones[0].config.pos_y);
        world.generate_zone_group(0).expect("rerolled");
        assert_eq!(world.zones.zones.len(), wanted);
        assert_ne!((world.zones.zones[0].config.pos_x, world.zones.zones[0].config.pos_y), first);
    }

    #[test]
    fn removing_a_zone_keeps_the_owners_of_the_pellets_of_the_others() {
        use crate::pellet::{MaterialKind, PelletSpawn};
        let mut world = group_world(1, 2);
        let keeper = world.zones.zones[3].id;
        let pellet = world
            .spawn_pellet(PelletSpawn {
                material: MaterialKind::Plant,
                amount: 6.0,
                pos: Vec2::ZERO,
                velocity: Vec2::ZERO,
                rotation: 0.0,
                angular_velocity: 0.0,
                owner_zone: Some(keeper),
                held: false,
            })
            .expect("spawn");
        let doomed = world.zones.zones[0].id;
        world.remove_zone(doomed).expect("removed");
        let now = world.zones.zones[2].id;
        assert_eq!(world.pellets.owner_zone(pellet), Some(Some(now)));
        // So the save still knows whose it is: the position of the zone in the list.
        let contents = world.to_contents();
        assert_eq!(contents.pellets.iter().filter(|record| record.owner_zone == Some(2)).count(), 1);
    }

    #[test]
    fn groups_and_their_zones_survive_a_save() {
        let mut world = group_world(2, 1);
        world
            .edit_zone_group(1, |group| group.template.fertility = 4.0)
            .expect("edit");
        let contents = world.to_contents();
        assert_eq!(contents.zones.len(), 1);
        assert_eq!(contents.zone_groups.len(), 2);
        assert_eq!(contents.zone_groups[0].zones.len(), 3);
        assert_eq!(contents.zone_groups[1].zones.len(), 4);
        assert_eq!(contents.all_zones().len(), 8);
        let loaded = World::from_contents(contents.clone()).expect("loaded");
        assert_eq!(loaded.zones.zones.len(), 8);
        assert_eq!(loaded.zone_groups.len(), 2);
        assert_eq!(loaded.zone_groups[1].template.fertility, 4.0);
        for (a, b) in world.zones.zones.iter().zip(&loaded.zones.zones) {
            assert_eq!(a.group, b.group);
            assert_eq!(a.config.name, b.config.name);
            assert_eq!((a.config.pos_x, a.config.pos_y), (b.config.pos_x, b.config.pos_y));
        }
        // A group saved without its zones has them generated on load.
        let mut bare = contents;
        for group in &mut bare.zone_groups {
            group.zones.clear();
        }
        let generated = World::from_contents(bare).expect("loaded");
        assert_eq!(generated.zones.zones.len(), 8);
    }

    // ----- settings animators: editing -----

    fn linear_changer() -> SettingsChanger {
        use crate::settings::SplinePoint;
        SettingsChanger {
            name: "Linear Animator".to_owned(),
            settings_bases: Vec::new(),
            points: vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(36_000.0, 1.5)],
            repeat: false,
            start: 0.0,
        }
    }

    #[test]
    fn the_graph_reads_the_curve_flat_outside_it_and_starting_over_when_it_repeats() {
        let mut changer = linear_changer();
        assert_eq!(changer.value_at(-5.0), 1.0);
        assert!((changer.value_at(18_000.0) - 1.25).abs() < 1e-5);
        assert_eq!(changer.value_at(36_000.0), 1.5);
        assert_eq!(changer.value_at(1.0e6), 1.5);
        changer.repeat = true;
        // After the last point the curve starts over (`FindValueAtX`).
        assert!((changer.value_at(36_000.0 + 18_000.0) - 1.25).abs() < 1e-4);
    }

    #[test]
    fn an_animator_is_edited_only_into_a_valid_one_and_a_setting_has_one_animator() {
        use crate::settings::SplinePoint;
        let mut world = World::new(default_settings(), ScenarioData::default(), 3).expect("world");
        let first = world.add_settings_changer(linear_changer()).expect("added");
        let second = world.add_settings_changer(linear_changer()).expect("added");
        assert!(!world.is_animated("pelletEnergy"));
        assert!(world.add_changer_base(first, "pelletEnergy", 200.0));
        assert!(world.is_animated("pelletEnergy"));
        // `RequestAnimateSetting`: the second animator cannot take a setting the first one animates.
        assert!(!world.add_changer_base(second, "pelletEnergy", 200.0));
        assert!(world.add_changer_base(second, "globalZoneSpeed", 1.0));
        // A valid edit goes through, an invalid one (a point before its predecessor) leaves the animator as it was.
        world
            .edit_settings_changer(first, |changer| {
                changer.name = "Slow".to_owned();
                changer.points.push(SplinePoint::new(72_000.0, 0.5));
            })
            .expect("valid");
        assert_eq!(world.settings_changers[first].name, "Slow");
        assert_eq!(world.settings_changers[first].points.len(), 3);
        assert!(world
            .edit_settings_changer(first, |changer| changer.points[2].x = 1_000.0)
            .is_err());
        assert_eq!(world.settings_changers[first].points[2].x, 72_000.0);
        assert!(world.edit_settings_changer(9, |_| {}).is_err());
    }
}

#[cfg(test)]
mod spline_jump_tests {
    use super::*;

    fn jump_points() -> Vec<SplinePoint> {
        // Held at 1 until time 100, then a jump to 3 at the same time, held at 3 to 200.
        vec![
            SplinePoint::new(0.0, 1.0),
            SplinePoint::new(100.0, 1.0),
            SplinePoint::new(100.0, 3.0),
            SplinePoint::new(200.0, 3.0),
        ]
    }

    #[test]
    fn a_curve_with_two_points_at_one_time_jumps_there_as_the_game_does() {
        let points = jump_points();
        // `SettingsChanger.FactorAtTime` takes the first segment whose end is after the time: at the shared time the curve is already
        // at the later point's value, and the zero-width segment between them is never evaluated.
        for (time, expected) in [(0.0, 1.0), (50.0, 1.0), (99.99, 1.0), (100.0, 3.0), (150.0, 3.0), (199.0, 3.0)] {
            let value = spline_factor(&points, time, false);
            assert!((value - expected).abs() < 1e-3, "at {time}: {value}");
            assert!(value.is_finite());
        }
        // The editor's curve (`FindValueAtX`) agrees away from the shared time and shows the first point's value on it.
        assert!((find_value_at_x(&points, 50.0, false) - 1.0).abs() < 1e-3);
        assert!((find_value_at_x(&points, 150.0, false) - 3.0).abs() < 1e-3);
        assert!(find_value_at_x(&points, 100.0, false).is_finite());
    }

    #[test]
    fn an_animator_with_a_jump_is_valid_and_repeats() {
        let mut changer = settings::SettingsChanger::game_defaults();
        changer.name = "Steps".to_owned();
        changer.points = jump_points();
        changer.validate().expect("equal times are allowed");
        // Repeating: the second lap jumps again.
        let lap = |time: f32| spline_factor(&changer.points, time, true);
        assert!((lap(250.0) - 1.0).abs() < 1e-3 || (lap(250.0) - 3.0).abs() < 1e-3);
        assert!((lap(299.0) - 1.0).abs() < 1e-3);
        assert!((lap(301.0) - 3.0).abs() < 1e-3);
        // A point before its predecessor is still refused.
        changer.points.push(SplinePoint::new(50.0, 2.0));
        assert!(changer.validate().is_err());
    }

    /// The game computes the pheromone sense at the END of every fixed update and the next brain tick reads it (`Pherosense.PherosenseAround` / `NEATBrain.UpdateSenses`): a brain
    /// tick sees the values of the tick before, and a tick that is not a brain tick only stores its own.
    #[test]
    fn the_pheromone_sense_is_read_one_tick_late() {
        let sense = |a: f32, b: f32, c: f32| senses::PheroSenses {
            sum: [0.0; 3],
            sense: [a, a, a],
            angle: [b, b, b],
            heading: [c, c, c],
        };
        let mut s = SenseOutputs::zeroed();
        // tick 1 is not a brain tick: the inputs stay as they were, the values wait
        delayed_phero(&mut s, &sense(0.1, 0.2, 0.3), false);
        assert_eq!((s.phero_sense_1, s.phero_1_angle, s.phero_1_heading), (0.0, 0.0, 0.0));
        assert_eq!(s.pending_phero, [0.1, 0.1, 0.1, 0.2, 0.2, 0.2, 0.3, 0.3, 0.3]);
        // tick 2 is a brain tick: it reads tick 1's values, and its own wait
        delayed_phero(&mut s, &sense(0.4, 0.5, 0.6), true);
        assert_eq!((s.phero_sense_2, s.phero_2_angle, s.phero_2_heading), (0.1, 0.2, 0.3));
        assert_eq!(s.pending_phero[0], 0.4);
        // a bibite that has not sensed before (just loaded) reads zeros on its first brain tick, as a freshly loaded game does
        let mut fresh = SenseOutputs::zeroed();
        delayed_phero(&mut fresh, &sense(0.7, 0.8, 0.9), true);
        assert_eq!((fresh.phero_sense_3, fresh.phero_3_angle, fresh.phero_3_heading), (0.0, 0.0, 0.0));
    }
}
