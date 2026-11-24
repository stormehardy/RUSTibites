//! Resting pellets skip the physics step.
//!
//! A pellet at rest that touches nothing is a fixed point of the physics step: its body is rebuilt from the store every tick,
//! the step integrates a zero velocity, finds no pair for it and writes back the pose it started with. For tens of thousands of
//! pellets that is most of the tick (the body build, the solver mirror, the integration, the boxes, the x order, the sweep, the
//! collider poses, the write-back and the store write, all for pellets that did not change).
//!
//! The bridge keeps such pellets out of the step. A pellet *falls asleep* at the end of a step in which its pose did not change
//! (bit for bit) and it is not held; it is then registered in a grid of boxes, and from the next tick the bridge no longer builds
//! its body. It *wakes* when anyone writes to it through the store (the store marks the slot), or when the store is replaced.
//! (A resting body stays at rest whatever its mass, drag or material are: the step has no force that acts on a body at rest.)
//!
//! Nothing a sleeper does is lost by leaving it out of a step. Its own step is the identity; the only way it affects anything is
//! through a pair (a contact, or a mouth's sensor overlapping it), and a pair needs the boxes of two bodies to overlap. So before
//! the step the bridge asks the grid which sleepers overlap the box of any body that is in the step (a bibite, an egg, an awake
//! pellet) and *promotes* them: it builds their bodies from the store, exactly as it would have every tick, and puts them in the
//! step in the place the dense order gives them. Sleepers never overlap each other (a pellet only falls asleep where no sleeper's
//! box overlaps its own), so one round of promotion finds every pair there is. The step then sees the same bodies in the same
//! order, apart from the sleepers that nobody touches, which would not have taken part in anything.
//!
//! **Relaxed mode** (`ParallelGates::settle_rotation`, on by default since 2026-11-23, the owner's decision): two pellets that touch
//! each other settle into a state where the position solver's last correction (below the rounding of a position of a thousand
//! units) cannot move either of them, but still turns each by a few 1e-10 radians a tick (the rounding of the contact's lever
//! arm). Their position, velocity and angular velocity are the same every tick and only the rotation drifts, which nothing in the
//! step reads (a pellet is a circle, the step only looks at differences of rotation within a step), so a drifting pellet is not
//! kept awake: a rotation that moved by less than [`REST_ROTATION_NOISE`] in a step counts as not moved. Such pellets overlap, so in
//! this mode a sleeper may overlap up to [`MAX_OVERLAPS`] other sleepers (each of them with room for one more), and promotion is
//! transitive: the sleepers that overlap a promoted sleeper are promoted with it (the contacts of a pile hold each other), so a
//! touched pile of resting pellets is in the step whole, exactly as it would be if nothing slept. What differs from the exact layer
//! is the stored rotation of the pellets that rest (it stops drifting; the largest drift ever seen is 1e-9 radians a tick) and
//! nothing else.

use crate::pellet::PelletStore;
use crate::physics::Aabb;

/// How many `f32` bits identify what the grid was laid out for (the world size).
pub(crate) const STAMP_LEN: usize = 1;

/// Largest side of the grid in cells.
const MAX_GRID_SIDE: f32 = 512.0;
/// Smallest cell side in world units (a pellet is a few units wide, a bibite a few tens).
const MIN_CELL: f32 = 64.0;
/// A pellet whose box covers more cells than this stays awake: the grid is for small things.
const MAX_SLEEPER_CELLS: i32 = 16;
/// A query box is widened by this much, so that rounding can never hide an overlap the broadphase would find.
const QUERY_MARGIN: f32 = 1e-3;
/// Relaxed mode: a pellet whose rotation moved by less than this many radians in a step (and whose position, velocity and angular
/// velocity did not change at all) did not move. The drift of a pair at rest is a few 1e-10 radians a tick.
pub(crate) const REST_ROTATION_NOISE: f32 = 1e-6;
/// Relaxed mode: how many other sleepers a sleeper's box may overlap, and how many a sleeper may be overlapped by (a pile of pellets
/// that all touch each other stays awake: a pile wakes as one, and one that holds hundreds would be in every step it was touched in).
const MAX_OVERLAPS: usize = 3;

/// What the sleeping layer did, for the bench table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SleepStats {
    /// Ticks the layer was in use.
    pub ticks: u64,
    /// Pellets asleep now.
    pub asleep: usize,
    /// Pellets awake now.
    pub awake: usize,
    /// Sleepers put into a step because a body's box overlapped theirs (sum over ticks).
    pub promoted: u64,
    /// Pellets that fell asleep (sum over ticks).
    pub slept: u64,
    /// Sleepers that woke up (sum over ticks).
    pub woken: u64,
    /// Pellets that fell asleep with their box over the box of another sleeper (relaxed mode; sum over ticks).
    pub overlapped: u64,
}

#[derive(Clone, Copy, Debug)]
struct Sleeper {
    aabb: Aabb,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// The resting-pellet state of one world (see the module docs).
#[derive(Clone, Debug, Default)]
pub(crate) struct PelletSleep {
    enabled: bool,
    epoch: u64,
    stamp: [u32; STAMP_LEN],
    origin: f32,
    inv_cell: f32,
    side: i32,
    cells: Vec<Vec<u32>>,
    /// One bit per cell, set while the cell holds a sleeper. A query asks it first: nearly every cell is empty, and the
    /// vector of an empty cell is a cache miss that the bit (32 KB for the biggest grid) spares.
    occupied: Vec<u64>,
    /// By pellet slot.
    sleepers: Vec<Option<Sleeper>>,
    /// Relaxed mode (see the module docs): rotation noise does not keep a pellet awake, and sleepers may overlap a few others.
    relaxed: bool,
    /// By pellet slot: how many other sleepers the sleeper's box overlaps (relaxed mode; 0 for an awake pellet).
    degree: Vec<u8>,
    sleeper_count: usize,
    /// Slots of the live pellets that are not asleep (each once; checked against the store at the start of a tick).
    awake: Vec<u32>,
    awake_listed: Vec<bool>,
    dirty: Vec<u32>,
    promoted_mark: Vec<bool>,
    promoted: Vec<u32>,
    stats: SleepStats,
}

fn overlaps(a: &Aabb, b: &Aabb) -> bool {
    a.overlaps(b)
}

impl PelletSleep {
    /// Whether the layer is in use (the world has enough pellets).
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Chooses the mode (see the module docs) for the ticks that follow; call it before [`Self::begin_tick`]. A change forgets what
    /// the layer knows (the sleepers of one mode are not the sleepers of the other).
    pub(crate) fn set_relaxed(&mut self, relaxed: bool) {
        if self.relaxed != relaxed {
            self.disable();
            self.relaxed = relaxed;
        }
    }

    /// Forgets everything; the next [`Self::begin_tick`] starts afresh with every pellet awake.
    pub(crate) fn disable(&mut self) {
        if self.enabled {
            *self = Self {
                stats: self.stats,
                ..Self::default()
            };
        }
    }

    /// What the layer did so far.
    pub(crate) fn stats(&self) -> SleepStats {
        SleepStats {
            asleep: self.sleeper_count,
            awake: self.awake.len(),
            ..self.stats
        }
    }

    fn ensure_slot(&mut self, slot: usize) {
        if self.sleepers.len() <= slot {
            self.sleepers.resize(slot + 1, None);
            self.degree.resize(slot + 1, 0);
            self.awake_listed.resize(slot + 1, false);
            self.promoted_mark.resize(slot + 1, false);
        }
    }

    /// Brings the layer up to date with the store at the start of a physics step: forgets all it knew when the store was
    /// replaced or a physics input outside the store changed, otherwise wakes the pellets that were written to; leaves
    /// [`Self::awake_slots`] the live pellets that are not asleep.
    pub(crate) fn begin_tick(
        &mut self,
        pellets: &mut PelletStore,
        stamp: [u32; STAMP_LEN],
        half_extent: f32,
    ) {
        self.stats.ticks += 1;
        let stale = !self.enabled || self.epoch != pellets.epoch() || self.stamp != stamp;
        if stale {
            let (stats, relaxed) = (self.stats, self.relaxed);
            *self = Self::default();
            self.stats = stats;
            self.relaxed = relaxed;
            self.enabled = true;
            self.epoch = pellets.epoch();
            self.stamp = stamp;
            let extent = (half_extent * 2.0 + 512.0).max(1.0);
            let cell = MIN_CELL.max(extent / MAX_GRID_SIDE);
            self.side = (extent / cell).ceil() as i32 + 1;
            self.origin = -(half_extent + 256.0);
            self.inv_cell = 1.0 / cell;
            self.cells = vec![Vec::new(); (self.side as usize) * (self.side as usize)];
            self.occupied = vec![0; self.cells.len().div_ceil(64)];
            // Everything the store was told about so far is covered by starting from every live pellet.
            let mut discard = std::mem::take(&mut self.dirty);
            pellets.take_phys_dirty(&mut discard);
            self.dirty = discard;
            if let Some(&last) = pellets.dense_slots().iter().max() {
                self.ensure_slot(last as usize);
            }
            for &slot in pellets.dense_slots() {
                self.awake.push(slot);
                self.awake_listed[slot as usize] = true;
            }
        } else {
            let mut dirty = std::mem::take(&mut self.dirty);
            pellets.take_phys_dirty(&mut dirty);
            for &slot in &dirty {
                let slot = slot as usize;
                self.ensure_slot(slot);
                if self.sleepers[slot].is_some() {
                    self.remove_sleeper(slot);
                    self.stats.woken += 1;
                }
                if pellets.live_id(slot).is_some() && !self.awake_listed[slot] {
                    self.awake.push(slot as u32);
                    self.awake_listed[slot] = true;
                }
            }
            self.dirty = dirty;
        }
        // Drop the slots that died or fell asleep.
        let mut kept = 0;
        for index in 0..self.awake.len() {
            let slot = self.awake[index] as usize;
            if pellets.live_id(slot).is_some() && self.sleepers[slot].is_none() {
                self.awake[kept] = slot as u32;
                kept += 1;
            } else {
                self.awake_listed[slot] = false;
            }
        }
        self.awake.truncate(kept);
    }

    /// The live pellets that are not asleep (valid after [`Self::begin_tick`]).
    pub(crate) fn awake_slots(&self) -> &[u32] {
        &self.awake
    }

    fn is_occupied(&self, cell: usize) -> bool {
        (self.occupied[cell >> 6] >> (cell & 63)) & 1 == 1
    }

    fn set_occupied(&mut self, cell: usize, occupied: bool) {
        if occupied {
            self.occupied[cell >> 6] |= 1 << (cell & 63);
        } else {
            self.occupied[cell >> 6] &= !(1 << (cell & 63));
        }
    }

    fn cell_range(&self, aabb: &Aabb) -> (i32, i32, i32, i32) {
        let axis = |value: f32| {
            (((value - self.origin) * self.inv_cell).floor() as i32).clamp(0, self.side - 1)
        };
        (
            axis(aabb.min.x),
            axis(aabb.min.y),
            axis(aabb.max.x),
            axis(aabb.max.y),
        )
    }

    /// Marks for promotion the sleepers whose boxes could be paired by the broadphase with `aabb` (the box of a body that is
    /// in the step).
    pub(crate) fn promote_overlapping(&mut self, aabb: &Aabb) {
        if self.sleeper_count == 0 || !aabb.min.is_finite() || !aabb.max.is_finite() {
            return;
        }
        let query = Aabb {
            min: aabb.min - crate::math::Vec2::splat(QUERY_MARGIN),
            max: aabb.max + crate::math::Vec2::splat(QUERY_MARGIN),
        };
        let (x0, y0, x1, y1) = self.cell_range(&query);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let cell = (cy * self.side + cx) as usize;
                if !self.is_occupied(cell) {
                    continue;
                }
                for index in 0..self.cells[cell].len() {
                    let slot = self.cells[cell][index] as usize;
                    if self.promoted_mark[slot] {
                        continue;
                    }
                    let Some(sleeper) = &self.sleepers[slot] else {
                        continue;
                    };
                    if overlaps(&sleeper.aabb, &query) {
                        self.promoted_mark[slot] = true;
                        self.promoted.push(slot as u32);
                    }
                }
            }
        }
    }

    /// Appends to `found`, in the order [`Self::promote_overlapping`] meets them, the slots of the sleepers whose boxes could be
    /// paired with `aabb`. Reads the grid only (the marks are not looked at): a sleeper over several cells, or found for
    /// several boxes, comes more than once, and [`Self::promote_slots`] drops the repeats. This is the part of the promotion
    /// that many bodies can do at once.
    pub(crate) fn overlapping(&self, aabb: &Aabb, found: &mut Vec<u32>) {
        if self.sleeper_count == 0 || !aabb.min.is_finite() || !aabb.max.is_finite() {
            return;
        }
        let query = Aabb {
            min: aabb.min - crate::math::Vec2::splat(QUERY_MARGIN),
            max: aabb.max + crate::math::Vec2::splat(QUERY_MARGIN),
        };
        let (x0, y0, x1, y1) = self.cell_range(&query);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let cell = (cy * self.side + cx) as usize;
                if !self.is_occupied(cell) {
                    continue;
                }
                for &slot in &self.cells[cell] {
                    if let Some(sleeper) = &self.sleepers[slot as usize]
                        && overlaps(&sleeper.aabb, &query)
                    {
                        found.push(slot);
                    }
                }
            }
        }
    }

    /// Marks for promotion the sleepers in `slots`, in order, the ones already marked skipped: what calling
    /// [`Self::promote_overlapping`] for the boxes that [`Self::overlapping`] answered for, one after the other, does.
    pub(crate) fn promote_slots(&mut self, slots: &[u32]) {
        for &slot in slots {
            if !self.promoted_mark[slot as usize] {
                self.promoted_mark[slot as usize] = true;
                self.promoted.push(slot);
            }
        }
    }

    /// Whether the pellet in `slot` rests (and is not in the step unless a box promotes it).
    pub(crate) fn is_asleep(&self, slot: usize) -> bool {
        self.sleepers.get(slot).is_some_and(|sleeper| sleeper.is_some())
    }

    /// The sleepers marked since the last [`Self::clear_promoted`].
    pub(crate) fn promoted(&self) -> &[u32] {
        &self.promoted
    }

    /// Forgets the marks (after the promoted sleepers were put into the step).
    pub(crate) fn clear_promoted(&mut self) {
        self.stats.promoted += self.promoted.len() as u64;
        for &slot in &self.promoted {
            self.promoted_mark[slot as usize] = false;
        }
        self.promoted.clear();
    }

    /// Decides at the end of a step, for a pellet that was in it, whether it rests: `unchanged` says its pose did not change
    /// (bit for bit) and it is not held; `aabb` is its box.
    pub(crate) fn settle(&mut self, slot: usize, unchanged: bool, aabb: Option<Aabb>) {
        self.ensure_slot(slot);
        if self.sleepers[slot].is_some() {
            // A sleeper that was promoted into the step.
            if !unchanged {
                self.remove_sleeper(slot);
                self.stats.woken += 1;
                if !self.awake_listed[slot] {
                    self.awake.push(slot as u32);
                    self.awake_listed[slot] = true;
                }
            }
            return;
        }
        if !unchanged {
            return;
        }
        let Some(aabb) = aabb else {
            return;
        };
        if !aabb.min.is_finite() || !aabb.max.is_finite() {
            return;
        }
        let (x0, y0, x1, y1) = self.cell_range(&aabb);
        if (x1 - x0 + 1) * (y1 - y0 + 1) > MAX_SLEEPER_CELLS {
            return;
        }
        // Sleepers never overlap each other; in relaxed mode a few may, and each of them has room for one more.
        let mut overlapped = [0u32; MAX_OVERLAPS];
        let mut overlapped_count = 0;
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let cell = (cy * self.side + cx) as usize;
                for &other in &self.cells[cell] {
                    if let Some(sleeper) = &self.sleepers[other as usize]
                        && overlaps(&sleeper.aabb, &aabb)
                    {
                        if !self.relaxed {
                            return;
                        }
                        // (A sleeper over several cells is met in each of them.)
                        if overlapped[..overlapped_count].contains(&other) {
                            continue;
                        }
                        if overlapped_count == MAX_OVERLAPS
                            || self.degree[other as usize] as usize >= MAX_OVERLAPS
                        {
                            return;
                        }
                        overlapped[overlapped_count] = other;
                        overlapped_count += 1;
                    }
                }
            }
        }
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let cell = (cy * self.side + cx) as usize;
                self.cells[cell].push(slot as u32);
                self.set_occupied(cell, true);
            }
        }
        self.sleepers[slot] = Some(Sleeper {
            aabb,
            x0,
            y0,
            x1,
            y1,
        });
        self.degree[slot] = overlapped_count as u8;
        for &other in &overlapped[..overlapped_count] {
            self.degree[other as usize] += 1;
        }
        if overlapped_count > 0 {
            self.stats.overlapped += 1;
        }
        self.sleeper_count += 1;
        self.stats.slept += 1;
    }

    /// Relaxed mode: promotes, with every sleeper that is marked, the sleepers whose boxes overlap its box, and so on (a pile of
    /// resting pellets that touch each other is in the step whole when one of them is). Call it after the marks of the bodies of the
    /// step and before the promoted sleepers are built. Does nothing in the exact mode, where sleepers do not overlap.
    pub(crate) fn close_promoted(&mut self) {
        if !self.relaxed {
            return;
        }
        let mut index = 0;
        while index < self.promoted.len() {
            let slot = self.promoted[index] as usize;
            index += 1;
            if let Some(aabb) = self.sleepers[slot].map(|sleeper| sleeper.aabb) {
                self.promote_overlapping(&aabb);
            }
        }
    }

    fn remove_sleeper(&mut self, slot: usize) {
        let Some(sleeper) = self.sleepers[slot].take() else {
            return;
        };
        if self.degree[slot] > 0 {
            // The sleepers that overlapped this one have one overlap fewer (each counted once, whatever number of cells they share).
            let mut seen = [0u32; MAX_OVERLAPS];
            let mut seen_count = 0;
            for cy in sleeper.y0..=sleeper.y1 {
                for cx in sleeper.x0..=sleeper.x1 {
                    let index = (cy * self.side + cx) as usize;
                    for &other in &self.cells[index] {
                        if other as usize != slot
                            && seen_count < MAX_OVERLAPS
                            && !seen[..seen_count].contains(&other)
                            && let Some(neighbour) = &self.sleepers[other as usize]
                            && overlaps(&neighbour.aabb, &sleeper.aabb)
                        {
                            seen[seen_count] = other;
                            seen_count += 1;
                            self.degree[other as usize] = self.degree[other as usize].saturating_sub(1);
                        }
                    }
                }
            }
            self.degree[slot] = 0;
        }
        for cy in sleeper.y0..=sleeper.y1 {
            for cx in sleeper.x0..=sleeper.x1 {
                let index = (cy * self.side + cx) as usize;
                let cell = &mut self.cells[index];
                if let Some(position) = cell.iter().position(|entry| *entry == slot as u32) {
                    cell.swap_remove(position);
                }
                let empty = cell.is_empty();
                self.set_occupied(index, !empty);
            }
        }
        self.sleeper_count -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;
    use crate::pellet::{MaterialKind, PelletSpawn};

    fn store_with(count: usize) -> PelletStore {
        let mut store = PelletStore::new();
        for index in 0..count {
            store.spawn(PelletSpawn::new(
                MaterialKind::Plant,
                20.0,
                Vec2::new(index as f32 * 100.0, 0.0),
            ));
        }
        store
    }

    fn boxed(x: f32, y: f32, half: f32) -> Aabb {
        Aabb {
            min: Vec2::new(x - half, y - half),
            max: Vec2::new(x + half, y + half),
        }
    }

    #[test]
    fn a_pellet_that_did_not_move_sleeps_and_one_that_did_stays_awake() {
        let mut store = store_with(3);
        let mut sleep = PelletSleep::default();
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!(sleep.awake_slots().len(), 3);
        sleep.settle(0, true, Some(boxed(0.0, 0.0, 2.5)));
        sleep.settle(1, false, Some(boxed(100.0, 0.0, 2.5)));
        sleep.settle(2, true, None);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!(sleep.awake_slots(), &[1, 2]);
        assert_eq!(sleep.stats().asleep, 1);
    }

    #[test]
    fn sleepers_never_overlap_each_other() {
        let mut store = store_with(2);
        let mut sleep = PelletSleep::default();
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        sleep.settle(0, true, Some(boxed(0.0, 0.0, 2.5)));
        // Overlaps the first sleeper's box (touching counts): stays awake.
        sleep.settle(1, true, Some(boxed(4.0, 0.0, 2.5)));
        assert_eq!(sleep.stats().asleep, 1);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!(sleep.awake_slots(), &[1]);
    }

    #[test]
    fn a_query_promotes_exactly_the_sleepers_it_overlaps_and_a_write_wakes_one() {
        let mut store = store_with(3);
        let mut sleep = PelletSleep::default();
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        for slot in 0..3 {
            sleep.settle(slot, true, Some(boxed(slot as f32 * 100.0, 0.0, 2.5)));
        }
        assert_eq!(sleep.stats().asleep, 3);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert!(sleep.awake_slots().is_empty());
        // A box over the middle one only (cells are 64 wide: the neighbours share no cell with it).
        sleep.promote_overlapping(&boxed(100.0, 0.0, 6.0));
        assert_eq!(sleep.promoted(), &[1]);
        sleep.clear_promoted();
        assert!(sleep.promoted().is_empty());
        // Touching the edge of a box counts, a hair away does not.
        sleep.promote_overlapping(&boxed(0.0, 7.0, 4.5));
        assert_eq!(sleep.promoted(), &[0]);
        sleep.clear_promoted();
        sleep.promote_overlapping(&boxed(0.0, 20.0, 4.5));
        assert!(sleep.promoted().is_empty());
        // A write through the store wakes its sleeper.
        let id = store.live_id(2).expect("live");
        store.set_velocity(id, Vec2::new(1.0, 0.0));
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!(sleep.awake_slots(), &[2]);
        assert_eq!(sleep.stats().asleep, 2);
    }

    /// The query as it was before the occupancy bits: every cell in range is looked into.
    fn plain_query(sleep: &mut PelletSleep, aabb: &Aabb) {
        if sleep.sleeper_count == 0 || !aabb.min.is_finite() || !aabb.max.is_finite() {
            return;
        }
        let query = Aabb {
            min: aabb.min - Vec2::splat(QUERY_MARGIN),
            max: aabb.max + Vec2::splat(QUERY_MARGIN),
        };
        let (x0, y0, x1, y1) = sleep.cell_range(&query);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let cell = (cy * sleep.side + cx) as usize;
                for index in 0..sleep.cells[cell].len() {
                    let slot = sleep.cells[cell][index] as usize;
                    if sleep.promoted_mark[slot] {
                        continue;
                    }
                    let Some(sleeper) = &sleep.sleepers[slot] else {
                        continue;
                    };
                    if overlaps(&sleeper.aabb, &query) {
                        sleep.promoted_mark[slot] = true;
                        sleep.promoted.push(slot as u32);
                    }
                }
            }
        }
    }

    /// Whether the bit of every cell says what the cell's vector does.
    fn occupancy_is_consistent(sleep: &PelletSleep) -> bool {
        (0..sleep.cells.len()).all(|cell| sleep.is_occupied(cell) == !sleep.cells[cell].is_empty())
    }

    #[test]
    fn the_occupancy_bits_change_nothing_a_query_finds() {
        use crate::rng::Rng;
        let mut store = store_with(400);
        let mut sleep = PelletSleep::default();
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        let mut rng = Rng::from_seed(5);
        for slot in 0..400 {
            let (x, y) = (rng.f32_range(-900.0, 900.0), rng.f32_range(-900.0, 900.0));
            sleep.settle(slot, true, Some(boxed(x, y, 2.5)));
        }
        assert!(sleep.stats().asleep > 300, "most of them sleep");
        assert!(occupancy_is_consistent(&sleep));
        // Waking every third one takes them out of their cells again.
        for slot in (0..400).step_by(3) {
            if let Some(id) = store.live_id(slot) {
                store.set_velocity(id, Vec2::new(1.0, 0.0));
            }
        }
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert!(occupancy_is_consistent(&sleep));
        let mut found = 0;
        for query in 0..2_000 {
            let half = [3.0, 6.0, 30.0, 80.0, 200.0][query % 5];
            let aabb = boxed(
                rng.f32_range(-1000.0, 1000.0),
                rng.f32_range(-1000.0, 1000.0),
                half,
            );
            let mut expected = sleep.clone();
            plain_query(&mut expected, &aabb);
            let mut got = sleep.clone();
            got.promote_overlapping(&aabb);
            assert_eq!(got.promoted(), expected.promoted(), "query {query}");
            found += got.promoted().len();
        }
        assert!(found > 100, "the queries found sleepers ({found})");
    }

    /// The boxes of many bodies asked about in stretches (each stretch's answers read the grid only) and marked afterwards in
    /// the order of the stretches give the sleepers in the order the plain loop, one box after the other, marks them in.
    #[test]
    fn stretches_of_boxes_asked_apart_mark_the_sleepers_in_the_order_of_the_plain_loop() {
        use crate::rng::Rng;
        let mut store = store_with(400);
        let mut sleep = PelletSleep::default();
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        let mut rng = Rng::from_seed(11);
        for slot in 0..400 {
            let (x, y) = (rng.f32_range(-900.0, 900.0), rng.f32_range(-900.0, 900.0));
            sleep.settle(slot, true, Some(boxed(x, y, 2.5)));
        }
        let boxes: Vec<Aabb> = (0..600)
            .map(|index| {
                let half = [3.0, 12.0, 40.0][index % 3];
                boxed(
                    rng.f32_range(-1000.0, 1000.0),
                    rng.f32_range(-1000.0, 1000.0),
                    half,
                )
            })
            .collect();
        let mut plain = sleep.clone();
        for aabb in &boxes {
            plain_query(&mut plain, aabb);
        }
        assert!(
            plain.promoted().len() > 50,
            "{} promoted",
            plain.promoted().len()
        );
        for stretch in [1_usize, 7, 64, 600] {
            let mut pooled = sleep.clone();
            let answers: Vec<Vec<u32>> = boxes
                .chunks(stretch)
                .map(|chunk| {
                    let mut found = Vec::new();
                    for aabb in chunk {
                        sleep.overlapping(aabb, &mut found);
                    }
                    found
                })
                .collect();
            for answer in &answers {
                pooled.promote_slots(answer);
            }
            assert_eq!(
                pooled.promoted(),
                plain.promoted(),
                "stretches of {stretch}"
            );
        }
    }

    #[test]
    fn relaxed_sleepers_may_overlap_a_few_others_and_are_promoted_together() {
        let mut store = store_with(5);
        let mut sleep = PelletSleep::default();
        sleep.set_relaxed(true);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        // A chain: 0 overlaps 1 and 1 overlaps 2 (boxes 5 wide, 4 apart); 3 is far away.
        sleep.settle(0, true, Some(boxed(0.0, 0.0, 2.5)));
        sleep.settle(1, true, Some(boxed(4.0, 0.0, 2.5)));
        sleep.settle(2, true, Some(boxed(8.0, 0.0, 2.5)));
        sleep.settle(3, true, Some(boxed(300.0, 0.0, 2.5)));
        let stats = sleep.stats();
        assert_eq!((stats.asleep, stats.overlapped), (4, 2));
        // A box over the first only: the chain comes with it, the far one does not.
        sleep.promote_overlapping(&boxed(-3.0, 0.0, 1.0));
        assert_eq!(sleep.promoted(), &[0]);
        sleep.close_promoted();
        let mut promoted = sleep.promoted().to_vec();
        promoted.sort_unstable();
        assert_eq!(promoted, vec![0, 1, 2]);
        sleep.clear_promoted();
        // The exact layer lets no sleeper overlap another and closes nothing.
        let mut exact = PelletSleep::default();
        exact.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        exact.settle(0, true, Some(boxed(0.0, 0.0, 2.5)));
        exact.settle(1, true, Some(boxed(4.0, 0.0, 2.5)));
        assert_eq!(exact.stats().asleep, 1);
        exact.promote_overlapping(&boxed(-3.0, 0.0, 1.0));
        exact.close_promoted();
        assert_eq!(exact.promoted(), &[0]);
    }

    #[test]
    fn relaxed_mode_keeps_a_crowd_awake_and_counts_overlaps_back_when_a_sleeper_wakes() {
        let mut store = store_with(6);
        let mut sleep = PelletSleep::default();
        sleep.set_relaxed(true);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        // Five boxes that all overlap each other: the fifth would overlap four sleepers.
        for slot in 0..5 {
            sleep.settle(slot, true, Some(boxed(slot as f32 * 0.5, 0.0, 2.5)));
        }
        assert_eq!(sleep.stats().asleep, 4, "four overlap each other, the fifth stays awake");
        // Waking the first one takes it off the others' counts: the fifth can rest now.
        let id = store.live_id(0).expect("live");
        store.set_velocity(id, Vec2::new(1.0, 0.0));
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!(sleep.stats().asleep, 3);
        sleep.settle(4, true, Some(boxed(2.0, 0.0, 2.5)));
        assert_eq!(sleep.stats().asleep, 4);
        // A change of mode forgets the sleepers (the exact layer would not hold overlapping ones).
        sleep.set_relaxed(false);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!(sleep.stats().asleep, 0);
    }

    #[test]
    fn a_relaxed_sleeper_over_two_cells_is_counted_once() {
        let mut store = store_with(2);
        let mut sleep = PelletSleep::default();
        sleep.set_relaxed(true);
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        // (The grid's cells are 64 wide from -1256: there is a cell edge at x = 24.) Both boxes straddle it, so each sleeper is in two cells.
        sleep.settle(0, true, Some(boxed(24.0, 0.0, 2.5)));
        sleep.settle(1, true, Some(boxed(26.0, 0.0, 2.5)));
        let (first, second) = (sleep.sleepers[0].expect("asleep"), sleep.sleepers[1].expect("asleep"));
        assert_ne!(first.x0, first.x1, "the box covers two cells");
        assert_ne!(second.x0, second.x1, "the box covers two cells");
        assert_eq!(sleep.stats().asleep, 2);
        assert_eq!((sleep.degree[0], sleep.degree[1]), (1, 1), "each overlaps the other once");
        // Waking one takes one overlap off the other, not two.
        let id = store.live_id(0).expect("live");
        store.set_velocity(id, Vec2::new(1.0, 0.0));
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        assert_eq!((sleep.degree[0], sleep.degree[1]), (0, 0));
    }

    #[test]
    fn a_replaced_store_or_a_resized_world_starts_everything_awake() {
        let mut store = store_with(2);
        let mut sleep = PelletSleep::default();
        sleep.begin_tick(&mut store, [1000.0f32.to_bits()], 1000.0);
        sleep.settle(0, true, Some(boxed(0.0, 0.0, 2.5)));
        sleep.settle(1, true, Some(boxed(100.0, 0.0, 2.5)));
        sleep.begin_tick(&mut store, [2000.0f32.to_bits()], 2000.0);
        assert_eq!(sleep.awake_slots().len(), 2);
        assert_eq!(sleep.stats().asleep, 0);
    }
}
