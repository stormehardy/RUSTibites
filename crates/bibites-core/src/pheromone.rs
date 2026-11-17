//! Pheromone spots, emission and dissipation (spec
//! `docs/spec/03-world-ecology.md` §4–§5).
//!
//! The world contains three independent pheromone channels (red, green, blue)
//! as `PheromoneSpot` objects. A spot stores a heading and one strength per
//! channel; sensing sums `strength / distance` over spots in range (the
//! `tanh(sum/5)` saturation is applied by the sensing module), and towers create
//! non-dissipating spots.
//!
//! # Exact game behaviour reproduced here
//!
//! * Dissipation is **0.25 strength/s**, applied in **0.25 s quanta**
//!   (`PheromoneSpot.FixedUpdate`), not the 1 u²/s claimed by older docs.
//! * Strengths are *not* clamped: they can become negative, and negative
//!   channels are ignored by sensing. A dissipating spot is only destroyed at the
//!   start of a later update once all three strengths are `<= 0`.
//! * Non-dissipating spots (pheromone towers) are never auto-destroyed.
//! * The original has **no spot cap**; the store grows/reuses slots on demand.
//!
//! # Units
//!
//! Strength is in `u²` (the game's pheromone "amount"). Position in world units.

use crate::arena::Handle;
use crate::ids::{PheromoneId, ToolId};
use crate::math::Vec2;
use crate::world::BitSet;

/// Dissipation quantum in seconds (`PheromoneSpot.DissipationPeriod`).
pub const DISSIPATION_QUANTUM: f32 = 0.25;

/// Dissipation rate per second (`PheromoneSpot.RateOfDissipation`).
pub const DISSIPATION_RATE: f32 = 0.25;

/// Bibite model-emission period in seconds (`BibitePheromoneOrgan`: one new
/// spot per simulated second).
pub const EMISSION_PERIOD: f32 = 1.0;

/// Minimum `r + g + b` output sum below which no bibite spot is created
/// (`BibitePheromoneOrgan`: `sum > 0.05`).
pub const EMISSION_THRESHOLD: f32 = 0.05;

/// The summed, strength-scaled brain output that decides whether a bibite emits and what it costs
/// (`BibitePheromoneOrgan.cs:70-80`).
#[inline]
#[must_use]
pub fn bibite_emission_sum(outputs: [f32; 3], production_strength: f32) -> f32 {
    let scaled = [
        outputs[0] * production_strength,
        outputs[1] * production_strength,
        outputs[2] * production_strength,
    ];
    scaled[0] + scaled[1] + scaled[2]
}

/// Whether a bibite whose [`bibite_emission_sum`] is `outputs_sum` creates a spot (NaN is "no emission": the game's
/// `sum > 0.05` is false for it).
#[inline]
#[must_use]
pub fn bibite_emission_triggers(outputs_sum: f32) -> bool {
    !(outputs_sum.is_nan() || outputs_sum <= EMISSION_THRESHOLD)
}

/// Default `pheromoneProductionStrength` (u² at brain output activation 1).
pub const DEFAULT_PRODUCTION_STRENGTH: f32 = 10.0;

/// Default `pheromoneProductionCost` (energy per unit of summed strength).
pub const DEFAULT_PRODUCTION_COST: f32 = 0.01;

/// Divisor applied to the phero sum before `tanh` when writing brain inputs
/// (`NEATBrain.UpdateSenses`: `tanh(pheroSum / 5)`).
pub const SENSE_TANH_DIVISOR: f32 = 5.0;

/// Parameters for creating a pheromone spot.
#[derive(Clone, Copy, Debug)]
pub struct PheroEmission {
    /// World position of the spot.
    pub pos: Vec2,
    /// Unit heading of the pheromone flow (drives `Phero*Heading`).
    pub heading: Vec2,
    /// Per-channel strengths `[red, green, blue]` in `u²`.
    pub strength: [f32; 3],
    /// Whether the spot dissipates. Bibite emissions `true`, towers `false`.
    pub dissipate: bool,
    /// Owning tower, if any (tower spots are refreshed and never dissipate).
    pub owner_tool: Option<ToolId>,
}

impl PheroEmission {
    /// A dissipating bibite-emitted spot.
    #[inline]
    pub fn bibite(pos: Vec2, heading: Vec2, strength: [f32; 3]) -> Self {
        Self {
            pos,
            heading,
            strength,
            dissipate: true,
            owner_tool: None,
        }
    }

    /// A non-dissipating tower spot.
    #[inline]
    pub fn tower(pos: Vec2, heading: Vec2, strength: [f32; 3], tool: ToolId) -> Self {
        Self {
            pos,
            heading,
            strength,
            dissipate: false,
            owner_tool: Some(tool),
        }
    }
}

/// Result of [`PheromoneStore::emit_bibite`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BibiteEmission {
    /// The created spot, or `None` when the output sum was below
    /// [`EMISSION_THRESHOLD`].
    pub spot: Option<PheromoneId>,
    /// Energy cost of the emission (0 when no spot was created).
    pub cost: f32,
    /// Total emitted strength `r + g + b` before scaling.
    pub outputs_sum: f32,
}

/// Cheap read-only copy of a spot's state for sensing/tools/save.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PheroView {
    /// Typed id of this spot.
    pub id: PheromoneId,
    /// World position.
    pub pos: Vec2,
    /// Heading of the pheromone flow.
    pub heading: Vec2,
    /// Per-channel strengths `[red, green, blue]`.
    pub strength: [f32; 3],
    /// Whether the spot dissipates.
    pub dissipate: bool,
    /// Owning tower, if any.
    pub owner_tool: Option<ToolId>,
}

/// A flat SoA store of pheromone spots with generational ids, a free list and a
/// reusable removed-id scratch list.
pub struct PheromoneStore {
    pos: Vec<Vec2>,
    heading: Vec<Vec2>,
    strength_r: Vec<f32>,
    strength_g: Vec<f32>,
    strength_b: Vec<f32>,
    dissipate: BitSet,
    accum: Vec<f32>,
    owner: Vec<Option<ToolId>>,
    generations: Vec<u32>,
    alive: BitSet,
    dense: Vec<u32>,
    dense_pos: Vec<u32>,
    free: Vec<u32>,
    removed: Vec<PheromoneId>,
}

impl PheromoneStore {
    /// Creates an empty store.
    pub const fn new() -> Self {
        Self {
            pos: Vec::new(),
            heading: Vec::new(),
            strength_r: Vec::new(),
            strength_g: Vec::new(),
            strength_b: Vec::new(),
            dissipate: BitSet::new(),
            accum: Vec::new(),
            owner: Vec::new(),
            generations: Vec::new(),
            alive: BitSet::new(),
            dense: Vec::new(),
            dense_pos: Vec::new(),
            free: Vec::new(),
            removed: Vec::new(),
        }
    }

    /// Creates an empty store with room for `capacity` spots.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut store = Self::new();
        store.reserve(capacity);
        store
    }

    /// Reserves capacity for at least `additional` more spots.
    pub fn reserve(&mut self, additional: usize) {
        self.pos.reserve(additional);
        self.heading.reserve(additional);
        self.strength_r.reserve(additional);
        self.strength_g.reserve(additional);
        self.strength_b.reserve(additional);
        self.accum.reserve(additional);
        self.owner.reserve(additional);
        self.generations.reserve(additional);
        self.dense.reserve(additional);
        self.dense_pos.reserve(additional);
        self.free.reserve(additional);
    }

    /// Number of live spots.
    #[inline]
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// `true` if the store holds no spots.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Number of allocated slots (live + vacant).
    #[inline]
    pub fn slot_capacity(&self) -> usize {
        self.generations.len()
    }

    /// Returns `true` if `id` refers to a live spot.
    #[inline]
    pub fn contains(&self, id: PheromoneId) -> bool {
        self.slot_of(id).is_some()
    }

    /// Iterates live spot ids in dense order.
    pub fn ids(&self) -> impl Iterator<Item = PheromoneId> + '_ {
        self.dense
            .iter()
            .map(move |&slot| self.id_at_slot(slot as usize))
    }

    /// Cheap copy of a spot's state.
    #[inline]
    pub fn get(&self, id: PheromoneId) -> Option<PheroView> {
        let slot = self.slot_of(id)?;
        Some(PheroView {
            id,
            pos: self.pos[slot],
            heading: self.heading[slot],
            strength: [
                self.strength_r[slot],
                self.strength_g[slot],
                self.strength_b[slot],
            ],
            dissipate: self.dissipate.get(slot),
            owner_tool: self.owner[slot],
        })
    }

    /// World position of a spot.
    #[inline]
    pub fn pos(&self, id: PheromoneId) -> Option<Vec2> {
        self.slot_of(id).map(|slot| self.pos[slot])
    }

    /// Heading of a spot.
    #[inline]
    pub fn heading(&self, id: PheromoneId) -> Option<Vec2> {
        self.slot_of(id).map(|slot| self.heading[slot])
    }

    /// Per-channel strengths of a spot.
    #[inline]
    pub fn strength(&self, id: PheromoneId) -> Option<[f32; 3]> {
        self.slot_of(id).map(|slot| {
            [
                self.strength_r[slot],
                self.strength_g[slot],
                self.strength_b[slot],
            ]
        })
    }

    /// Whether a spot dissipates.
    #[inline]
    pub fn dissipates(&self, id: PheromoneId) -> Option<bool> {
        self.slot_of(id).map(|slot| self.dissipate.get(slot))
    }

    /// Owning tower of a spot.
    #[inline]
    pub fn owner_tool(&self, id: PheromoneId) -> Option<Option<ToolId>> {
        self.slot_of(id).map(|slot| self.owner[slot])
    }

    /// Creates a pheromone spot (`PheromoneSpot` creation path).
    ///
    /// Returns `None` when the position or a strength is not finite.
    pub fn emit(&mut self, spec: PheroEmission) -> Option<PheromoneId> {
        if !spec.pos.is_finite()
            || !spec.heading.is_finite()
            || spec.strength.iter().any(|s| !s.is_finite())
        {
            return None;
        }

        let slot = if let Some(slot) = self.free.pop() {
            let slot = slot as usize;
            self.generations[slot] = self.generations[slot].wrapping_add(1);
            slot
        } else {
            let slot = self.pos.len();
            self.push_slot();
            slot
        };

        self.pos[slot] = spec.pos;
        self.heading[slot] = spec.heading;
        self.strength_r[slot] = spec.strength[0];
        self.strength_g[slot] = spec.strength[1];
        self.strength_b[slot] = spec.strength[2];
        self.dissipate.set(slot, spec.dissipate);
        self.accum[slot] = 0.0;
        self.owner[slot] = spec.owner_tool;
        self.alive.set(slot, true);

        self.dense_pos[slot] = self.dense.len() as u32;
        self.dense.push(slot as u32);

        Some(self.id_at_slot(slot))
    }

    /// Bibite pheromone-organ emission (`BibitePheromoneOrgan.UpdateOrgan`).
    ///
    /// Scales each brain output by `production_strength`, creates a dissipating
    /// spot when the summed output exceeds [`EMISSION_THRESHOLD`], and returns
    /// the energy cost `sum · production_cost` (zero when nothing was emitted).
    /// The caller is responsible for subtracting `cost` from the bibite's
    /// energy.
    pub fn emit_bibite(
        &mut self,
        pos: Vec2,
        heading: Vec2,
        outputs: [f32; 3],
        production_strength: f32,
        production_cost: f32,
    ) -> BibiteEmission {
        // `BibitePheromoneOrgan.cs:70-80`: each channel is scaled by the
        // production strength first; the 0.05 threshold and the energy cost
        // both use the *scaled* sum.
        let scaled = [
            outputs[0] * production_strength,
            outputs[1] * production_strength,
            outputs[2] * production_strength,
        ];
        let outputs_sum = bibite_emission_sum(outputs, production_strength);
        if !bibite_emission_triggers(outputs_sum) {
            return BibiteEmission {
                spot: None,
                cost: 0.0,
                outputs_sum,
            };
        }
        let spot = self.emit(PheroEmission::bibite(pos, heading, scaled));
        BibiteEmission {
            spot,
            cost: if spot.is_some() {
                outputs_sum * production_cost
            } else {
                0.0
            },
            outputs_sum,
        }
    }

    /// Refreshes a tower spot's strengths and heading in place.
    ///
    /// Returns `false` if the spot is not live.
    pub fn set_tower_state(&mut self, id: PheromoneId, heading: Vec2, strength: [f32; 3]) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.heading[slot] = heading;
                self.strength_r[slot] = strength[0];
                self.strength_g[slot] = strength[1];
                self.strength_b[slot] = strength[2];
                true
            }
            None => false,
        }
    }

    /// Immediately removes a spot.
    pub fn remove(&mut self, id: PheromoneId) -> bool {
        match self.slot_of(id) {
            Some(slot) => {
                self.destroy_slot(slot);
                true
            }
            None => false,
        }
    }

    /// Advances dissipation by `dt` seconds (`PheromoneSpot.FixedUpdate`).
    ///
    /// Spots are processed in dense order. A dissipating spot whose three
    /// strengths are all `<= 0` at the start of the update is destroyed; after
    /// that, each 0.25 s of accumulated time subtracts `0.25 · quanta` from every
    /// channel that is currently positive (unclamped, so strengths may go
    /// negative). Non-dissipating tower spots skip the subtraction entirely and
    /// therefore keep their strengths forever.
    ///
    /// Removed ids are queued for [`Self::drain_removed`] so the spatial hash can
    /// be updated in the same tick.
    pub fn dissipate(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.removed.clear();
        for i in 0..self.dense.len() {
            let slot = self.dense[i] as usize;
            let dissipate = self.dissipate.get(slot);
            if dissipate
                && self.strength_r[slot] <= 0.0
                && self.strength_g[slot] <= 0.0
                && self.strength_b[slot] <= 0.0
            {
                self.removed.push(self.id_at_slot(slot));
                continue;
            }
            self.accum[slot] += dt;
            if self.accum[slot] < DISSIPATION_QUANTUM {
                continue;
            }
            // quanta = floor(accum / quantum) · quantum, exactly like the game.
            let quanta = (self.accum[slot] / DISSIPATION_QUANTUM).floor() * DISSIPATION_QUANTUM;
            self.accum[slot] -= quanta;
            if dissipate {
                let decay = quanta * DISSIPATION_RATE;
                if self.strength_r[slot] > 0.0 {
                    self.strength_r[slot] -= decay;
                }
                if self.strength_g[slot] > 0.0 {
                    self.strength_g[slot] -= decay;
                }
                if self.strength_b[slot] > 0.0 {
                    self.strength_b[slot] -= decay;
                }
            }
        }

        // Apply removals after the scan (the game destroys at the start of the
        // next FixedUpdate; queuing here keeps the dense list stable while
        // iterating and lets the world update the spatial hash immediately).
        for i in 0..self.removed.len() {
            let id = self.removed[i];
            if let Some(slot) = self.slot_of(id) {
                self.destroy_slot(slot);
            }
        }
    }

    /// Calls `f` for every id removed since the last [`Self::dissipate`] call,
    /// then clears the queue.
    pub fn drain_removed(&mut self, mut f: impl FnMut(PheromoneId)) {
        for i in 0..self.removed.len() {
            f(self.removed[i]);
        }
        self.removed.clear();
    }

    /// Number of ids currently queued from the last dissipation update.
    #[inline]
    pub fn removed_count(&self) -> usize {
        self.removed.len()
    }

    /// Returns the decay accumulator (seconds toward the next quantum).
    #[inline]
    pub fn decay_accumulator(&self, id: PheromoneId) -> Option<f32> {
        self.slot_of(id).map(|slot| self.accum[slot])
    }

    #[inline]
    fn id_at_slot(&self, slot: usize) -> PheromoneId {
        PheromoneId::from_handle(Handle::new(slot as u32, self.generations[slot]))
    }

    #[inline]
    fn slot_of(&self, id: PheromoneId) -> Option<usize> {
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

    fn destroy_slot(&mut self, slot: usize) {
        if !self.alive.get(slot) {
            return;
        }
        let position = self.dense_pos[slot] as usize;
        self.dense.swap_remove(position);
        if let Some(&moved) = self.dense.get(position) {
            self.dense_pos[moved as usize] = position as u32;
        }
        self.dense_pos[slot] = u32::MAX;
        self.alive.set(slot, false);
        self.dissipate.set(slot, false);
        self.owner[slot] = None;
        self.strength_r[slot] = 0.0;
        self.strength_g[slot] = 0.0;
        self.strength_b[slot] = 0.0;
        self.accum[slot] = 0.0;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        self.free.push(slot as u32);
    }

    fn push_slot(&mut self) {
        let slot = self.pos.len();
        self.pos.push(Vec2::ZERO);
        self.heading.push(Vec2::X);
        self.strength_r.push(0.0);
        self.strength_g.push(0.0);
        self.strength_b.push(0.0);
        self.accum.push(0.0);
        self.owner.push(None);
        self.generations.push(0);
        self.dense_pos.push(u32::MAX);
        self.dissipate.ensure(slot + 1);
        self.alive.ensure(slot + 1);
    }
}

impl Default for PheromoneStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Checkpoint P2: pheromone store arena/runtime bytes (task-103)
// ---------------------------------------------------------------------------

impl PheromoneStore {
    /// Captures the full arena/runtime state as deterministic little-endian bytes.
    #[must_use]
    pub fn capture_checkpoint(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, self.pos.len() as u32);
        for slot in 0..self.pos.len() {
            let alive = self.alive.get(slot);
            out.push(u8::from(alive));
            // Preserve dead-slot generations for the id allocator.
            push_u32(&mut out, self.generations[slot]);
            if !alive {
                continue;
            }
            push_vec2(&mut out, self.pos[slot]);
            push_vec2(&mut out, self.heading[slot]);
            push_f32(&mut out, self.strength_r[slot]);
            push_f32(&mut out, self.strength_g[slot]);
            push_f32(&mut out, self.strength_b[slot]);
            out.push(u8::from(self.dissipate.get(slot)));
            push_f32(&mut out, self.accum[slot]);
            match self.owner[slot] {
                Some(tool) => {
                    out.push(1);
                    push_u32(&mut out, tool.handle().slot());
                    push_u32(&mut out, tool.handle().generation());
                }
                None => out.push(0),
            }
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

    /// Restores state produced by [`PheromoneStore::capture_checkpoint`].
    pub fn restore_checkpoint(&mut self, bytes: &[u8]) -> crate::error::Result<()> {
        let mut reader = PheroCheckpointReader { bytes, pos: 0 };
        let capacity = reader.u32()? as usize;
        self.pos = vec![Vec2::ZERO; capacity];
        self.heading = vec![Vec2::X; capacity];
        self.strength_r = vec![0.0; capacity];
        self.strength_g = vec![0.0; capacity];
        self.strength_b = vec![0.0; capacity];
        self.dissipate = BitSet::new();
        self.accum = vec![0.0; capacity];
        self.owner = vec![None; capacity];
        self.generations = vec![0; capacity];
        self.alive = BitSet::new();
        for slot in 0..capacity {
            let alive = reader.bool()?;
            self.alive.set(slot, alive);
            self.generations[slot] = reader.u32()?;
            if !alive {
                continue;
            }
            self.pos[slot] = reader.vec2()?;
            self.heading[slot] = reader.vec2()?;
            self.strength_r[slot] = reader.f32()?;
            self.strength_g[slot] = reader.f32()?;
            self.strength_b[slot] = reader.f32()?;
            self.dissipate.set(slot, reader.bool()?);
            self.accum[slot] = reader.f32()?;
            self.owner[slot] = if reader.u8()? == 1 {
                Some(ToolId::from_handle(Handle::new(
                    reader.u32()?,
                    reader.u32()?,
                )))
            } else {
                None
            };
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
                "trailing bytes in pheromone checkpoint",
            ));
        }
        if dense_pos.len() != capacity
            || dense.iter().any(|slot| *slot as usize >= capacity)
            || free.iter().any(|slot| *slot as usize >= capacity)
            || dense.len() + free.len() != capacity
        {
            return Err(crate::error::Error::invalid_data(
                "invalid pheromone checkpoint arena layout",
            ));
        }
        for (position, slot) in dense.iter().enumerate() {
            if dense_pos.get(*slot as usize).copied() != Some(position as u32) {
                return Err(crate::error::Error::invalid_data(
                    "inconsistent pheromone checkpoint dense positions",
                ));
            }
        }
        for slot in &free {
            if dense_pos.get(*slot as usize).copied() != Some(u32::MAX) {
                return Err(crate::error::Error::invalid_data(
                    "free pheromone slot has a dense position",
                ));
            }
        }
        self.free = free;
        self.dense = dense;
        self.dense_pos = dense_pos;
        self.removed.clear();
        Ok(())
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

struct PheroCheckpointReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl PheroCheckpointReader<'_> {
    fn take(&mut self, len: usize) -> crate::error::Result<&[u8]> {
        let end = self.pos.checked_add(len).ok_or_else(|| {
            crate::error::Error::invalid_data("pheromone checkpoint length overflow")
        })?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| crate::error::Error::invalid_data("truncated pheromone checkpoint"))?;
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
                "invalid bool in pheromone checkpoint",
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

    fn spot(store: &mut PheromoneStore, strength: [f32; 3]) -> PheromoneId {
        store
            .emit(PheroEmission::bibite(Vec2::ZERO, Vec2::Y, strength))
            .expect("emit")
    }

    #[test]
    fn emit_stores_fields() {
        let mut store = PheromoneStore::new();
        let id = store
            .emit(PheroEmission::bibite(
                Vec2::new(3.0, 4.0),
                Vec2::Y,
                [1.0, 2.0, 3.0],
            ))
            .unwrap();
        let view = store.get(id).unwrap();
        assert_eq!(view.pos, Vec2::new(3.0, 4.0));
        assert_eq!(view.heading, Vec2::Y);
        assert_eq!(view.strength, [1.0, 2.0, 3.0]);
        assert!(view.dissipate);
        assert_eq!(view.owner_tool, None);
        assert_eq!(store.len(), 1);
        assert!(store.contains(id));
    }

    #[test]
    fn invalid_emit_is_rejected() {
        let mut store = PheromoneStore::new();
        assert!(
            store
                .emit(PheroEmission::bibite(
                    Vec2::new(f32::NAN, 0.0),
                    Vec2::Y,
                    [1.0, 0.0, 0.0]
                ))
                .is_none()
        );
        assert!(
            store
                .emit(PheroEmission::bibite(
                    Vec2::ZERO,
                    Vec2::Y,
                    [1.0, f32::INFINITY, 0.0]
                ))
                .is_none()
        );
    }

    #[test]
    fn dissipation_is_quantized_at_quarter_second() {
        let mut store = PheromoneStore::new();
        let id = spot(&mut store, [10.0, 0.0, 0.0]);

        // 0.1 + 0.1 + 0.1 = 0.3 s -> one 0.25 s quantum decays 0.0625.
        store.dissipate(0.1);
        assert_eq!(store.strength(id).unwrap(), [10.0, 0.0, 0.0]);
        store.dissipate(0.1);
        assert_eq!(store.strength(id).unwrap(), [10.0, 0.0, 0.0]);
        store.dissipate(0.1);
        assert!(store.strength(id).unwrap()[0] < 10.0);
        assert!((store.strength(id).unwrap()[0] - 9.9375).abs() < 1e-5);
        // Accumulator keeps the leftover 0.05 s.
        assert!((store.decay_accumulator(id).unwrap() - 0.05).abs() < 1e-5);

        // Channels with non-positive strength are untouched.
        assert_eq!(store.strength(id).unwrap()[1], 0.0);
        assert_eq!(store.strength(id).unwrap()[2], 0.0);
    }

    #[test]
    fn negative_strengths_are_allowed_and_removed_next_update() {
        let mut store = PheromoneStore::new();
        let id = spot(&mut store, [1.0, 0.0, 0.0]);
        store.dissipate(10.0); // one update: 10 s * 0.25 = 2.5 decay -> -1.5
        assert!((store.strength(id).unwrap()[0] + 1.5).abs() < 1e-4);
        assert!(store.contains(id));
        // Next update sees all strengths <= 0 and destroys it.
        store.dissipate(0.001);
        assert!(!store.contains(id));
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn decaying_spot_survives_until_zero_then_dies() {
        let mut store = PheromoneStore::new();
        let id = spot(&mut store, [10.0, 0.0, 0.0]);
        // 40 s of decay: 10 / 0.25 = 40 s.
        for _ in 0..160 {
            store.dissipate(0.25);
        }
        assert!(store.contains(id));
        assert!((store.strength(id).unwrap()[0]).abs() < 1e-4);
        store.dissipate(0.25);
        assert!(!store.contains(id));
        let mut seen = Vec::new();
        store.drain_removed(|removed| seen.push(removed));
        assert_eq!(seen, vec![id]);
        assert_eq!(store.removed_count(), 0);
    }

    #[test]
    fn tower_spots_never_dissipate_or_die() {
        let mut store = PheromoneStore::new();
        let tool = ToolId::from_handle(crate::arena::Handle::new(7, 0));
        let id = store
            .emit(PheroEmission::tower(
                Vec2::ZERO,
                Vec2::X,
                [5.0, 0.0, 0.0],
                tool,
            ))
            .unwrap();
        assert!(!store.dissipates(id).unwrap());
        assert_eq!(store.owner_tool(id), Some(Some(tool)));
        for _ in 0..1000 {
            store.dissipate(0.25);
        }
        assert!(store.contains(id));
        assert_eq!(store.strength(id).unwrap(), [5.0, 0.0, 0.0]);

        // A zero-strength tower spot also persists.
        let empty = store
            .emit(PheroEmission::tower(Vec2::ZERO, Vec2::X, [0.0; 3], tool))
            .unwrap();
        for _ in 0..10 {
            store.dissipate(0.25);
        }
        assert!(store.contains(empty));
        // Refresh in place (tower settings change).
        assert!(store.set_tower_state(empty, Vec2::Y, [1.0, 2.0, 3.0]));
        assert_eq!(store.strength(empty).unwrap(), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn bibite_emission_threshold_scaling_and_cost() {
        let mut store = PheromoneStore::new();
        let out = store.emit_bibite(
            Vec2::ZERO,
            Vec2::Y,
            [0.0, 0.0, 0.0],
            DEFAULT_PRODUCTION_STRENGTH,
            DEFAULT_PRODUCTION_COST,
        );
        assert_eq!(out.spot, None);
        assert_eq!(out.cost, 0.0);

        // Raw sum 0.004 -> scaled 0.04 (strength 10) -> below the 0.05 threshold.
        let out = store.emit_bibite(
            Vec2::ZERO,
            Vec2::Y,
            [0.004, 0.0, 0.0],
            DEFAULT_PRODUCTION_STRENGTH,
            DEFAULT_PRODUCTION_COST,
        );
        assert_eq!(out.spot, None);

        // Raw sum 0.06 -> scaled sum 0.6 emitted; cost = scaled sum * 0.01
        // (`BibitePheromoneOrgan.cs:78`).
        let out = store.emit_bibite(
            Vec2::ZERO,
            Vec2::Y,
            [0.03, 0.02, 0.01],
            DEFAULT_PRODUCTION_STRENGTH,
            DEFAULT_PRODUCTION_COST,
        );
        let id = out.spot.expect("emitted");
        assert!((out.cost - 0.006).abs() < 1e-7, "{}", out.cost);
        let strength = store.strength(id).unwrap();
        for (actual, expected) in strength.iter().zip([0.3, 0.2, 0.1]) {
            assert!((actual - expected).abs() < 1e-6, "{strength:?}");
        }
        assert!(store.dissipates(id).unwrap());
    }

    #[test]
    fn pooling_reuses_slots_and_invalidates_old_ids() {
        let mut store = PheromoneStore::new();
        let first = spot(&mut store, [1.0, 0.0, 0.0]);
        assert!(store.remove(first));
        assert_eq!(store.len(), 0);
        let second = spot(&mut store, [2.0, 0.0, 0.0]);
        assert_eq!(first.handle().slot(), second.handle().slot());
        assert_ne!(first.handle().generation(), second.handle().generation());
        assert!(!store.contains(first));
        assert!(store.contains(second));
        assert_eq!(store.slot_capacity(), 1);
    }

    #[test]
    fn id_iteration_is_deterministic() {
        let mut a = PheromoneStore::new();
        let mut b = PheromoneStore::new();
        let insert = |store: &mut PheromoneStore| -> Vec<PheromoneId> {
            (0..10)
                .map(|i| {
                    store
                        .emit(PheroEmission::bibite(
                            Vec2::new(i as f32, 0.0),
                            Vec2::Y,
                            [1.0, 0.0, 0.0],
                        ))
                        .unwrap()
                })
                .collect()
        };
        let ids_a = insert(&mut a);
        let ids_b = insert(&mut b);
        for i in [3usize, 7] {
            a.remove(ids_a[i]);
            b.remove(ids_b[i]);
        }
        let order_a: Vec<PheromoneId> = a.ids().collect();
        let order_b: Vec<PheromoneId> = b.ids().collect();
        assert_eq!(order_a, order_b);
        let mut slots: Vec<u32> = order_a.iter().map(|id| id.handle().slot()).collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 1, 2, 4, 5, 6, 8, 9]);
    }

    #[test]
    fn dissipate_removal_queue_can_be_drained() {
        let mut store = PheromoneStore::new();
        let id = spot(&mut store, [0.1, 0.0, 0.0]);
        store.dissipate(1.0); // 0.1 - 0.25 = -0.15
        store.dissipate(1.0); // all <= 0 -> queued + removed
        let mut seen = Vec::new();
        store.drain_removed(|removed| seen.push(removed));
        assert_eq!(seen, vec![id]);
        // Draining twice doesn't re-report.
        let mut seen_again = Vec::new();
        store.drain_removed(|removed| seen_again.push(removed));
        assert!(seen_again.is_empty());
    }

    #[test]
    fn non_finite_dt_is_ignored() {
        let mut store = PheromoneStore::new();
        let id = spot(&mut store, [1.0, 0.0, 0.0]);
        store.dissipate(f32::NAN);
        store.dissipate(-1.0);
        assert_eq!(store.strength(id).unwrap(), [1.0, 0.0, 0.0]);
    }
}
