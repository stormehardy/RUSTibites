//! A cache-line prefetch hint and an eight-lane box test.
//!
//! The passes of the simulation over its records are limited by the cache lines they pull from memory, and the access
//! pattern of one record (a few lines here, a run of ten there) is not one the hardware prefetchers follow. A pass that knows
//! which record it takes next can say so with [`read_l2`], and the lines arrive while it works on the one before.
//!
//! This is the only place of the simulation's crates that needs `unsafe` (the standard library's own safe prefetch,
//! `core::hint::prefetch_read`, is not stable yet): `_mm_prefetch` is a hint to the cache hierarchy. It never faults, on
//! valid or invalid addresses, and changes no value the program can observe, so the function is safe to call with any pointer.

/// Hints that the cache line holding `ptr` will be read soon, and asks for it in the L2 cache and up (not in the L1, which a
/// prefetch of a few kilobytes ahead would flush). Does nothing on targets other than x86-64. The pointer is not dereferenced
/// and need not be valid.
#[inline(always)]
pub fn read_l2(ptr: *const u8) {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: a prefetch is a hint: it has no architectural effect, never faults (whatever the address) and SSE is part
        // of the x86-64 baseline.
        unsafe {
            core::arch::x86_64::_mm_prefetch::<{ core::arch::x86_64::_MM_HINT_T1 }>(
                ptr.cast::<i8>(),
            );
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = ptr;
    }
}

/// Tests eight candidate boxes against one query box in one go: bit `lane` of the result is set when
/// `max_x[lane] >= query_min_x && min_y[lane] <= query_max_y && max_y[lane] >= query_min_y` (a comparison with a NaN is false,
/// as in the scalar code this replaces). The broadphase's sweep asks it for every run of eight entries after a box; with SSE2
/// (part of the x86-64 baseline) it is two vectors of four, whatever the compiler makes of a loop of eight comparisons, which
/// it vectorizes or not depending on code far away (a 0.8 ms swing of the broadphase between builds of the same stage).
#[inline(always)]
pub fn overlap_mask8(
    max_x: &[f32; 8],
    min_y: &[f32; 8],
    max_y: &[f32; 8],
    query_min_x: f32,
    query_max_y: f32,
    query_min_y: f32,
) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::{
            _mm_and_ps, _mm_cmpge_ps, _mm_cmple_ps, _mm_loadu_ps, _mm_movemask_ps, _mm_set1_ps,
        };
        // SAFETY: SSE2 is part of the x86-64 baseline; every load reads four floats (16 bytes, no alignment needed) from the
        // eight-float arrays the references point to.
        unsafe {
            let (x, hi_y, lo_y) = (
                _mm_set1_ps(query_min_x),
                _mm_set1_ps(query_max_y),
                _mm_set1_ps(query_min_y),
            );
            let half = |at: usize| {
                let right = _mm_cmpge_ps(_mm_loadu_ps(max_x.as_ptr().add(at)), x);
                let below = _mm_cmple_ps(_mm_loadu_ps(min_y.as_ptr().add(at)), hi_y);
                let above = _mm_cmpge_ps(_mm_loadu_ps(max_y.as_ptr().add(at)), lo_y);
                _mm_movemask_ps(_mm_and_ps(_mm_and_ps(right, below), above)) as u32
            };
            half(0) | (half(4) << 4)
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut hits = 0u32;
        for lane in 0..8 {
            let hit = (max_x[lane] >= query_min_x)
                & (min_y[lane] <= query_max_y)
                & (max_y[lane] >= query_min_y);
            hits |= u32::from(hit) << lane;
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::overlap_mask8;

    fn scalar(
        max_x: &[f32; 8],
        min_y: &[f32; 8],
        max_y: &[f32; 8],
        query_min_x: f32,
        query_max_y: f32,
        query_min_y: f32,
    ) -> u32 {
        let mut hits = 0u32;
        for lane in 0..8 {
            let hit = (max_x[lane] >= query_min_x)
                & (min_y[lane] <= query_max_y)
                & (max_y[lane] >= query_min_y);
            hits |= u32::from(hit) << lane;
        }
        hits
    }

    #[test]
    fn the_mask_is_the_scalar_one_for_every_kind_of_number() {
        // A small deterministic generator, and values that make ties, NaNs, infinities and signed zeros likely.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let special = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::MAX,
            f32::MIN_POSITIVE,
        ];
        let value = |next: &mut dyn FnMut() -> u64| -> f32 {
            let bits = next();
            if bits % 5 == 0 {
                special[(bits >> 8) as usize % special.len()]
            } else {
                ((bits >> 16) % 9) as f32 - 4.0
            }
        };
        for _ in 0..20_000 {
            let mut lanes = [[0.0f32; 8]; 3];
            for lane in &mut lanes {
                for entry in lane.iter_mut() {
                    *entry = value(&mut next);
                }
            }
            let query = [value(&mut next), value(&mut next), value(&mut next)];
            assert_eq!(
                overlap_mask8(
                    &lanes[0], &lanes[1], &lanes[2], query[0], query[1], query[2]
                ),
                scalar(
                    &lanes[0], &lanes[1], &lanes[2], query[0], query[1], query[2]
                ),
                "{lanes:?} {query:?}"
            );
        }
    }
}
