//! A generational arena with stable handles and dense iteration.
//!
//! Simulation entities (bibites, eggs, pellets, pheromone spots, ...) need:
//!
//! * **stable ids** that survive removals and save/load conversion,
//! * **O(1) insert / remove / lookup**,
//! * **cache-friendly dense iteration** in the hot loop,
//! * **stale-handle detection** so a removed entity cannot be accidentally
//!   addressed after its slot is reused.
//!
//! [`Arena`] provides all four. Handles are `(slot, generation)` pairs; each
//! time a slot is reused its generation increments, invalidating old handles.
//!
//! # Example
//!
//! ```
//! use bibites_core::Arena;
//!
//! let mut arena = Arena::new();
//! let a = arena.insert("alpha");
//! let b = arena.insert("beta");
//! assert_eq!(arena.get(a), Some(&"alpha"));
//! assert_eq!(arena.remove(a), Some("alpha"));
//! assert_eq!(arena.get(a), None);
//! assert_eq!(arena.len(), 1);
//! assert_eq!(arena.iter().count(), 1);
//! let _ = b;
//! ```

use core::fmt;
use core::ops::{Index, IndexMut};

/// A stable handle into an [`Arena`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle {
    slot: u32,
    generation: u32,
}

impl Handle {
    /// Creates a handle from its raw parts. Prefer the handles returned by
    /// [`Arena`].
    #[inline]
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }

    /// Returns the slot index.
    #[inline]
    pub const fn slot(self) -> u32 {
        self.slot
    }

    /// Returns the generation counter.
    #[inline]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.slot, self.generation)
    }
}

/// One slot of an [`Arena`].
struct Slot<T> {
    generation: u32,
    /// `None` when the slot is vacant.
    value: Option<T>,
}

/// A generational arena with dense iteration.
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    /// Slot indices of live entries, packed for cache-friendly iteration.
    dense: Vec<u32>,
    /// Position of each slot inside [`Arena::dense`].
    dense_pos: Vec<u32>,
    /// Slots available for reuse.
    free: Vec<u32>,
    /// Number of live entries.
    len: usize,
    /// Highest number of live entries ever observed.
    peak: usize,
}

impl<T> Arena<T> {
    /// Creates an empty arena.
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            dense: Vec::new(),
            dense_pos: Vec::new(),
            free: Vec::new(),
            len: 0,
            peak: 0,
        }
    }

    /// Creates an empty arena with room for `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            dense: Vec::with_capacity(capacity),
            dense_pos: Vec::with_capacity(capacity),
            free: Vec::new(),
            len: 0,
            peak: 0,
        }
    }

    /// Inserts a value and returns its stable handle.
    pub fn insert(&mut self, value: T) -> Handle {
        let (slot_index, generation) = if let Some(slot) = self.free.pop() {
            let slot = slot as usize;
            // Reuse an existing slot, bumping its generation.
            let generation = self.slots[slot].generation.wrapping_add(1);
            self.slots[slot] = Slot {
                generation,
                value: Some(value),
            };
            (slot as u32, generation)
        } else {
            let slot = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                value: Some(value),
            });
            self.dense_pos.push(u32::MAX);
            (slot, 0)
        };

        let position = self.dense.len() as u32;
        self.dense.push(slot_index);
        self.dense_pos[slot_index as usize] = position;
        self.len += 1;
        self.peak = self.peak.max(self.len);

        Handle::new(slot_index, generation)
    }

    /// Removes the value behind `handle`, returning it if the handle is live.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        if !self.is_live(handle) {
            return None;
        }
        let slot_index = handle.slot as usize;
        let value = self.slots[slot_index].value.take();
        self.slots[slot_index].generation = self.slots[slot_index].generation.wrapping_add(1);
        self.free.push(handle.slot);

        // Swap-remove from the dense list.
        let position = self.dense_pos[slot_index] as usize;
        self.dense.swap_remove(position);
        if let Some(&moved_slot) = self.dense.get(position) {
            self.dense_pos[moved_slot as usize] = position as u32;
        }
        self.dense_pos[slot_index] = u32::MAX;
        self.len -= 1;

        value
    }

    /// Returns `true` if `handle` points at a live value.
    #[inline]
    pub fn is_live(&self, handle: Handle) -> bool {
        match self.slots.get(handle.slot as usize) {
            Some(slot) => slot.generation == handle.generation && slot.value.is_some(),
            None => false,
        }
    }

    /// Returns a reference to the value behind `handle`, if live.
    #[inline]
    pub fn get(&self, handle: Handle) -> Option<&T> {
        let slot = self.slots.get(handle.slot as usize)?;
        if slot.generation == handle.generation {
            slot.value.as_ref()
        } else {
            None
        }
    }

    /// Returns a mutable reference to the value behind `handle`, if live.
    #[inline]
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.slot as usize)?;
        if slot.generation == handle.generation {
            slot.value.as_mut()
        } else {
            None
        }
    }

    /// Returns the handle that is currently stored in slot `slot_index`, if
    /// that slot is live. Useful for rebuilding handle lists after
    /// deserialization.
    #[inline]
    pub fn handle_at(&self, slot_index: u32) -> Option<Handle> {
        let slot = self.slots.get(slot_index as usize)?;
        if slot.value.is_some() {
            Some(Handle::new(slot_index, slot.generation))
        } else {
            None
        }
    }

    /// Number of live entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the arena holds no live entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Highest number of live entries ever observed (useful for capacity
    /// planning after heavy churn).
    #[inline]
    pub fn peak_len(&self) -> usize {
        self.peak
    }

    /// Number of allocated slots (live + vacant).
    #[inline]
    pub fn slot_capacity(&self) -> usize {
        self.slots.len()
    }

    /// Iterates over `(handle, &value)` pairs in dense order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> + '_ {
        self.dense.iter().map(|&slot_index| {
            let slot = &self.slots[slot_index as usize];
            (
                Handle::new(slot_index, slot.generation),
                slot.value
                    .as_ref()
                    .expect("dense slot must be occupied by invariant"),
            )
        })
    }

    /// Calls `f` with `(handle, &mut value)` for every live entry in dense
    /// order.
    ///
    /// This deliberately takes a callback instead of returning an iterator:
    /// safe Rust cannot hand out `&mut T` references from an iterator over
    /// indices without `unsafe`, and the hot loop only ever needs a visit.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(Handle, &mut T)) {
        for &slot_index in &self.dense {
            let slot = &mut self.slots[slot_index as usize];
            let generation = slot.generation;
            let value = slot
                .value
                .as_mut()
                .expect("dense slot must be occupied by invariant");
            f(Handle::new(slot_index, generation), value);
        }
    }

    /// Calls `f` with `&mut value` for every live entry in dense order.
    pub fn for_each_value_mut(&mut self, mut f: impl FnMut(&mut T)) {
        for &slot_index in &self.dense {
            let value = self.slots[slot_index as usize]
                .value
                .as_mut()
                .expect("dense slot must be occupied by invariant");
            f(value);
        }
    }

    /// Iterates over values in dense order.
    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        self.dense.iter().map(|&slot_index| {
            self.slots[slot_index as usize]
                .value
                .as_ref()
                .expect("dense slot must be occupied by invariant")
        })
    }

    /// Iterates over the handles of all live entries in dense order.
    pub fn handles(&self) -> impl Iterator<Item = Handle> + '_ {
        self.dense.iter().map(|&slot_index| {
            let slot = &self.slots[slot_index as usize];
            Handle::new(slot_index, slot.generation)
        })
    }

    /// Clears all entries while keeping allocated capacity.
    pub fn clear(&mut self) {
        for (slot_index, slot) in self.slots.iter_mut().enumerate() {
            if slot.value.take().is_some() {
                slot.generation = slot.generation.wrapping_add(1);
                self.free.push(slot_index as u32);
            }
        }
        self.dense.clear();
        self.dense_pos.fill(u32::MAX);
        self.len = 0;
    }

    /// Reserves capacity for at least `additional` more entries.
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
        self.dense.reserve(additional);
        self.dense_pos.reserve(additional);
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Index<Handle> for Arena<T> {
    type Output = T;

    #[inline]
    fn index(&self, handle: Handle) -> &T {
        self.get(handle).expect("invalid or stale arena handle")
    }
}

impl<T> IndexMut<Handle> for Arena<T> {
    #[inline]
    fn index_mut(&mut self, handle: Handle) -> &mut T {
        self.get_mut(handle).expect("invalid or stale arena handle")
    }
}

impl<T: fmt::Debug> fmt::Debug for Arena<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Arena")
            .field("len", &self.len)
            .field("slots", &self.slots.len())
            .field("free", &self.free.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_remove() {
        let mut arena = Arena::new();
        let a = arena.insert(10);
        let b = arena.insert(20);
        assert_eq!(arena.len(), 2);
        assert_eq!(arena.get(a), Some(&10));
        assert_eq!(arena.get(b), Some(&20));
        assert_eq!(arena.remove(a), Some(10));
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.len(), 1);
        assert_eq!(arena.iter().count(), 1);
    }

    #[test]
    fn removed_handles_are_stale_after_reuse() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena.remove(a);
        let b = arena.insert(2);
        assert_eq!(a.slot(), b.slot());
        assert_ne!(a.generation(), b.generation());
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get(b), Some(&2));
    }

    #[test]
    fn dense_iteration_matches_len() {
        let mut arena = Arena::new();
        let handles: Vec<_> = (0..100).map(|i| arena.insert(i)).collect();
        for handle in handles.iter().step_by(2) {
            arena.remove(*handle);
        }
        assert_eq!(arena.len(), 50);
        assert_eq!(arena.iter().count(), 50);
        assert_eq!(arena.values().count(), 50);
        assert_eq!(arena.handles().count(), 50);
    }

    #[test]
    fn for_each_mut_visits_every_live_value() {
        let mut arena = Arena::new();
        for i in 0..8 {
            arena.insert(i);
        }
        arena.for_each_value_mut(|value| *value *= 2);
        let mut values: Vec<i32> = arena.values().copied().collect();
        values.sort_unstable();
        assert_eq!(values, vec![0, 2, 4, 6, 8, 10, 12, 14]);

        let mut handles_seen = Vec::new();
        arena.for_each_mut(|handle, _| handles_seen.push(handle));
        assert_eq!(handles_seen.len(), 8);
    }

    #[test]
    fn clear_works() {
        let mut arena = Arena::new();
        for i in 0..10 {
            arena.insert(i);
        }
        arena.clear();
        assert!(arena.is_empty());
        assert_eq!(arena.iter().count(), 0);
        let h = arena.insert(42);
        assert_eq!(arena[h], 42);
    }

    #[test]
    fn handle_at_returns_live_or_none() {
        let mut arena = Arena::new();
        let a = arena.insert(7);
        let b = arena.insert(8);
        assert_eq!(arena.handle_at(a.slot()), Some(a));
        assert_eq!(arena.handle_at(b.slot()), Some(b));
        arena.remove(a);
        assert_eq!(arena.handle_at(a.slot()), None);
    }
}
