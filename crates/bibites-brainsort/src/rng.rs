//! The one deliberate difference from the original: a small deterministic generator that
//! replaces `UnityEngine.Random` inside [`Layout::sort`](crate::Layout::sort).

use crate::vec2::Vec2;

/// Upper bound on rejection-sampling rounds in [`Rng::inside_unit_circle`]. Each round accepts
/// with probability pi/4, so exhausting this bound has probability below `1e-40`; it only
/// exists so that the function can never loop forever.
const MAX_CIRCLE_ATTEMPTS: usize = 64;

/// Deterministic pseudo-random generator (xorshift64*, seeded through SplitMix64).
///
/// It stands in for the two `UnityEngine.Random` entry points that `BrainSorter.Sort` uses:
/// [`Rng::range`] for `Random.Range(float, float)` and [`Rng::inside_unit_circle`] for
/// `Random.insideUnitCircle`. The same seed always yields the same layout. The *sequence* of
/// numbers is of course not Unity's, but the *order and number of draws* made by the sorter is
/// exactly the C# one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Rng::new(0)
    }
}

impl Rng {
    /// Creates a generator from any seed (including 0). The seed is scrambled with SplitMix64
    /// so that small consecutive seeds give unrelated streams.
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // xorshift needs a non-zero state.
        Rng(if z == 0 { 0x9E37_79B9_7F4A_7C15 } else { z })
    }

    /// The next 64 random bits (xorshift64*).
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform value in `[0, 1)` with 24 bits of resolution.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / 16_777_216.0)
    }

    /// Replacement for `Random.Range(minInclusive, maxInclusive)` (float overload): a uniform
    /// value in `[min, max)`. (Unity's version is inclusive of `max`; the difference is a single
    /// representable value and has no effect on the layout.)
    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }

    /// Replacement for `Random.insideUnitCircle`: a point inside the unit circle, by rejection
    /// sampling from the enclosing square.
    pub fn inside_unit_circle(&mut self) -> Vec2 {
        for _ in 0..MAX_CIRCLE_ATTEMPTS {
            let v = Vec2::new(self.range(-1.0, 1.0), self.range(-1.0, 1.0));
            if v.sqr_magnitude() <= 1.0 {
                return v;
            }
        }
        Vec2::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_different_seed_different_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let mut c = Rng::new(43);
        let sa: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let sb: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let sc: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(sa, sb);
        assert_ne!(sa, sc);
    }

    #[test]
    fn seed_zero_is_usable() {
        let mut r = Rng::new(0);
        let first = r.next_u64();
        let second = r.next_u64();
        assert_ne!(first, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn range_stays_in_bounds_and_covers_both_signs() {
        let mut r = Rng::new(7);
        let mut seen_neg = false;
        let mut seen_pos = false;
        for _ in 0..2000 {
            let v = r.range(-1.0, 1.0);
            assert!((-1.0..1.0).contains(&v), "{v}");
            seen_neg |= v < 0.0;
            seen_pos |= v > 0.0;
        }
        assert!(seen_neg && seen_pos);
        let v = r.range(3.0, 3.5);
        assert!((3.0..3.5).contains(&v));
    }

    #[test]
    fn inside_unit_circle_is_inside_the_circle() {
        let mut r = Rng::new(99);
        let mut max_len = 0.0f32;
        for _ in 0..2000 {
            let v = r.inside_unit_circle();
            assert!(v.sqr_magnitude() <= 1.0, "{v:?}");
            max_len = max_len.max(v.magnitude());
        }
        // 2000 uniform samples reach well past radius 0.9.
        assert!(max_len > 0.9, "{max_len}");
    }
}
