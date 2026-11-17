//! Deterministic pseudo-random number generation for the simulation.
//!
//! RUSTibites does **not** use a global RNG. Every random decision is drawn
//! from an explicitly seeded [`Rng`] stream so that a run is fully reproducible
//! from `(seed, scenario, tick count)` — including across platforms and across
//! single-threaded/parallel execution.
//!
//! # Design
//!
//! * [`Rng`] is `xoshiro256**`, seeded through SplitMix64. The algorithm is
//!   fixed here, so the same seed always yields the same sequence.
//! * Streams are derived with [`Rng::derive`] / [`Rng::fork`], which mix a
//!   domain tag and (optionally) an entity id into the seed. Deriving a
//!   per-entity stream makes parallel execution order-independent: each entity
//!   consumes its own stream in its own logical order.
//! * The simulation must never use `std::time`, thread-local randomness, or
//!   OS randomness. Only these streams.

/// SplitMix64 step, used for seeding.
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Mixes two `u64` values into a well-distributed seed.
#[inline]
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.rotate_left(32);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    x
}

/// Serializable position of an [`Rng`] stream (task-75 checkpointing).
///
/// `seed` is the original stream seed (used by [`Rng::fork`]/[`Rng::derive`]);
/// `words` is the raw xoshiro256** state. Both are required: `fork` is
/// position-independent from the seed, so restoring only the seed would replay
/// the stream from its beginning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RngState {
    /// Seed the stream was created from.
    pub seed: u64,
    /// Raw xoshiro256** state words (must not be all zero).
    pub words: [u64; 4],
}

impl RngState {
    /// Builds a state value from a seed and raw xoshiro words.
    #[inline]
    pub const fn new(seed: u64, words: [u64; 4]) -> Self {
        Self { seed, words }
    }

    /// xoshiro256** must not run from the all-zero state.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.words != [0; 4]
    }
}

/// A seeded, deterministic pseudo-random number generator (`xoshiro256**`).
#[derive(Clone, Debug)]
pub struct Rng {
    seed: u64,
    state: [u64; 4],
}

impl Rng {
    /// Creates a generator from a 64-bit seed.
    pub fn from_seed(seed: u64) -> Self {
        let mut sm = seed;
        let mut state = [0u64; 4];
        for word in &mut state {
            *word = splitmix64(&mut sm);
        }
        // xoshiro must not start in the all-zero state.
        if state == [0; 4] {
            state[0] = 0x9E37_79B9_7F4A_7C15;
        }
        Self { seed, state }
    }

    /// Creates an independent stream derived from `seed`, a domain tag, and an
    /// entity id.
    ///
    /// The result depends only on the arguments, never on how many values have
    /// already been drawn, so it is safe to use in parallel code.
    pub fn derive(seed: u64, domain: u64, entity: u64) -> Self {
        let mixed = mix_seed(seed ^ domain.wrapping_mul(0x9E37_79B9_7F4A_7C15), entity);
        Self::from_seed(mixed)
    }

    /// Returns a new, independent stream forked from this generator's seed.
    ///
    /// This is position-independent: the same parent seed and `stream` tag
    /// always produce the same child stream.
    pub fn fork(&self, stream: u64) -> Self {
        Self::from_seed(mix_seed(self.seed, stream))
    }

    /// Returns the seed this generator was created from.
    #[inline]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Captures the exact stream position (for checkpointing).
    #[inline]
    pub fn state(&self) -> RngState {
        RngState {
            seed: self.seed,
            words: self.state,
        }
    }

    /// Recreates a generator at a captured [`RngState`] position.
    ///
    /// Returns `None` for an invalid all-zero xoshiro state.
    pub fn from_state(state: RngState) -> Option<Self> {
        if !state.is_valid() {
            return None;
        }
        Some(Self {
            seed: state.seed,
            state: state.words,
        })
    }

    /// Restores this generator to a captured [`RngState`] position.
    ///
    /// Returns `false` (leaving the generator untouched) for an invalid
    /// all-zero state.
    pub fn set_state(&mut self, state: RngState) -> bool {
        if !state.is_valid() {
            return false;
        }
        self.seed = state.seed;
        self.state = state.words;
        true
    }

    /// Returns the next 64 random bits.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.state[1] << 17;

        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);

        result
    }

    /// Returns the next 32 random bits.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Returns a uniform `f32` in `[0, 1)`.
    #[inline]
    pub fn f32(&mut self) -> f32 {
        // Use the top 24 bits: exactly representable in f32.
        ((self.next_u64() >> 40) as f32) * (1.0 / 16_777_216.0)
    }

    /// Returns a uniform `f32` in `[min, max)`.
    #[inline]
    pub fn f32_range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.f32()
    }

    /// Returns a uniform `f64` in `[0, 1)`.
    #[inline]
    pub fn f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Returns `true` with the given probability.
    #[inline]
    pub fn bool(&mut self, probability: f32) -> bool {
        self.f32() < probability
    }

    /// Returns a uniform integer in `[0, n)`. Returns `0` when `n == 0`.
    #[inline]
    pub fn usize_below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % (n as u64)) as usize
    }

    /// Returns a uniform integer in `[min, max)`.
    #[inline]
    pub fn i32_range(&mut self, min: i32, max: i32) -> i32 {
        debug_assert!(max > min, "empty integer range");
        min + (self.next_u64() % ((max - min) as u64)) as i32
    }

    /// Returns a sample from a standard normal distribution (mean 0, sd 1).
    ///
    /// Uses the Marsaglia polar method; no cached spare value, so it consumes
    /// exactly two uniforms per rejection until a sample is accepted.
    pub fn normal(&mut self) -> f32 {
        loop {
            let u = 2.0 * self.f32() - 1.0;
            let v = 2.0 * self.f32() - 1.0;
            let s = u * u + v * v;
            if s > 0.0 && s < 1.0 {
                let factor = (-2.0 * s.ln() / s).sqrt();
                return u * factor;
            }
        }
    }

    /// Returns a sample from a normal distribution with the given mean and
    /// standard deviation.
    #[inline]
    pub fn normal_with(&mut self, mean: f32, std_dev: f32) -> f32 {
        mean + std_dev * self.normal()
    }

    /// Shuffles a slice in place with Fisher-Yates.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.usize_below(i + 1);
            slice.swap(i, j);
        }
    }

    /// Returns a random element of `slice`, or `None` if it is empty.
    pub fn choose<'a, T>(&mut self, slice: &'a [T]) -> Option<&'a T> {
        if slice.is_empty() {
            return None;
        }
        slice.get(self.usize_below(slice.len()))
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::from_seed(0)
    }
}

/// Domain-separation tags for derived streams.
///
/// Using named constants keeps streams stable if code is reordered, and stops
/// unrelated subsystems from accidentally sharing a stream.
pub mod domain {
    /// Mutation of genomes.
    pub const MUTATION: u64 = 0x4D55_5441_5445_0001;
    /// Initial population / spawning.
    pub const SPAWN: u64 = 0x5350_4157_4E00_0002;
    /// Plant and food spawn decisions.
    pub const FOOD: u64 = 0x464F_4F44_0000_0003;
    /// Pheromone emission.
    pub const PHEROMONE: u64 = 0x5048_4552_4F00_0004;
    /// Physics tie-breaking and jitter.
    pub const PHYSICS: u64 = 0x5048_5953_4943_0005;
    /// Speciation and lineage decisions.
    pub const SPECIES: u64 = 0x5350_4543_4945_0006;
    /// Miscellaneous world events.
    pub const WORLD: u64 = 0x574F_524C_4400_0007;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::from_seed(12345);
        let mut b = Rng::from_seed(12345);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn floats_are_in_range() {
        let mut rng = Rng::from_seed(7);
        for _ in 0..10_000 {
            let f = rng.f32();
            assert!((0.0..1.0).contains(&f), "out of range: {f}");
            let g = rng.f32_range(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&g), "out of range: {g}");
        }
    }

    #[test]
    fn derived_streams_are_position_independent() {
        let mut parent = Rng::from_seed(99);
        let child_a = parent.fork(5);
        // Advance the parent; forking again must give the same child.
        let _ = parent.next_u64();
        let child_b = parent.fork(5);
        let mut a = child_a;
        let mut b = child_b;
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn normal_is_reasonable() {
        let mut rng = Rng::from_seed(3);
        let n = 20_000;
        let mut sum = 0.0f64;
        let mut sum_sq = 0.0f64;
        for _ in 0..n {
            let x = rng.normal() as f64;
            sum += x;
            sum_sq += x * x;
        }
        let mean = sum / n as f64;
        let var = sum_sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((var - 1.0).abs() < 0.1, "variance {var}");
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut rng = Rng::from_seed(42);
        let mut v: Vec<u32> = (0..64).collect();
        rng.shuffle(&mut v);
        v.sort_unstable();
        assert_eq!(v, (0..64).collect::<Vec<_>>());
    }
}
