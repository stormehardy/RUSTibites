//! 2D vector math and scalar helpers used across the simulation.
//!
//! Everything is `f32`, matching the original Unity game. Functions are small
//! and `#[inline]`; the hot simulation loop should not need any other maths
//! crate.
//!
//! Angle conventions follow the original game:
//! * `Vec2::from_angle(a)` produces `(cos a, sin a)`.
//! * Positive angles are counter-clockwise in a standard maths coordinate
//!   system. The game's rotation direction sign is handled by the caller.

use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Full turn in radians.
pub const TAU: f32 = core::f32::consts::TAU;
/// Half turn in radians.
pub const PI: f32 = core::f32::consts::PI;
/// Tolerance used by approximate comparisons.
pub const EPS: f32 = 1e-6;

/// A 2D vector of `f32` components.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f32,
    /// Vertical component.
    pub y: f32,
}

impl Vec2 {
    /// The zero vector.
    pub const ZERO: Self = Self::new(0.0, 0.0);
    /// The vector with both components equal to one.
    pub const ONE: Self = Self::new(1.0, 1.0);
    /// Unit vector along the positive x axis.
    pub const X: Self = Self::new(1.0, 0.0);
    /// Unit vector along the positive y axis.
    pub const Y: Self = Self::new(0.0, 1.0);

    /// Creates a vector from two components.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Creates a vector with both components set to `v`.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self::new(v, v)
    }

    /// Creates a unit vector pointing at `angle` radians.
    #[inline]
    pub fn from_angle(angle: f32) -> Self {
        Self::new(angle.cos(), angle.sin())
    }

    /// Returns the squared length of the vector.
    #[inline]
    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// Returns the length of the vector.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }

    /// Returns the squared distance to `other`.
    #[inline]
    pub fn distance_sq(self, other: Self) -> f32 {
        (self - other).length_sq()
    }

    /// Returns the distance to `other`.
    #[inline]
    pub fn distance(self, other: Self) -> f32 {
        self.distance_sq(other).sqrt()
    }

    /// Dot product.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    /// 2D cross product (z component of the 3D cross product).
    #[inline]
    pub fn cross(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }

    /// Rotates the vector by `angle` radians counter-clockwise.
    #[inline]
    pub fn rotate(self, angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self::new(self.x * cos - self.y * sin, self.x * sin + self.y * cos)
    }

    /// Returns the angle of the vector in radians, in `(-PI, PI]`.
    #[inline]
    pub fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }

    /// Returns the vector rotated 90 degrees counter-clockwise.
    #[inline]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Returns the normalized vector, or [`Vec2::ZERO`] if the length is zero.
    #[inline]
    pub fn normalized(self) -> Self {
        self.normalize_or(Self::ZERO)
    }

    /// Returns the normalized vector, or `fallback` if the length is zero.
    #[inline]
    pub fn normalize_or(self, fallback: Self) -> Self {
        let len_sq = self.length_sq();
        if len_sq > EPS * EPS {
            self / len_sq.sqrt()
        } else {
            fallback
        }
    }

    /// Returns a vector with the same direction and the given length.
    #[inline]
    pub fn with_length(self, len: f32) -> Self {
        self.normalized() * len
    }

    /// Limits the length of the vector to at most `max_len`.
    #[inline]
    pub fn clamp_length(self, max_len: f32) -> Self {
        let len_sq = self.length_sq();
        if len_sq > max_len * max_len && len_sq > 0.0 {
            self * (max_len / len_sq.sqrt())
        } else {
            self
        }
    }

    /// Linear interpolation between two vectors.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    /// Returns `true` if both components are finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Component-wise minimum.
    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self::new(self.x.min(other.x), self.y.min(other.y))
    }

    /// Component-wise maximum.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self::new(self.x.max(other.x), self.y.max(other.y))
    }

    /// Approximate equality with the given absolute tolerance.
    #[inline]
    pub fn approx_eq(self, other: Self, eps: f32) -> bool {
        (self.x - other.x).abs() <= eps && (self.y - other.y).abs() <= eps
    }
}

impl Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Mul<Vec2> for f32 {
    type Output = Vec2;
    #[inline]
    fn mul(self, rhs: Vec2) -> Vec2 {
        rhs * self
    }
}

impl MulAssign<f32> for Vec2 {
    #[inline]
    fn mul_assign(&mut self, rhs: f32) {
        self.x *= rhs;
        self.y *= rhs;
    }
}

impl Div<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f32) -> Self {
        Self::new(self.x / rhs, self.y / rhs)
    }
}

impl DivAssign<f32> for Vec2 {
    #[inline]
    fn div_assign(&mut self, rhs: f32) {
        self.x /= rhs;
        self.y /= rhs;
    }
}

impl Neg for Vec2 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl From<[f32; 2]> for Vec2 {
    #[inline]
    fn from(v: [f32; 2]) -> Self {
        Self::new(v[0], v[1])
    }
}

impl From<Vec2> for [f32; 2] {
    #[inline]
    fn from(v: Vec2) -> Self {
        [v.x, v.y]
    }
}

// ---------------------------------------------------------------------------
// Scalar helpers
// ---------------------------------------------------------------------------

/// Clamps `x` into `[min, max]`.
#[inline]
pub fn clamp(x: f32, min: f32, max: f32) -> f32 {
    if x < min {
        min
    } else if x > max {
        max
    } else {
        x
    }
}

/// Clamps `x` into `[0, 1]`.
#[inline]
pub fn clamp01(x: f32) -> f32 {
    clamp(x, 0.0, 1.0)
}

/// Linear interpolation.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Inverse linear interpolation: where `v` sits between `a` and `b`.
#[inline]
pub fn inv_lerp(a: f32, b: f32, v: f32) -> f32 {
    if (b - a).abs() <= EPS {
        0.0
    } else {
        (v - a) / (b - a)
    }
}

/// Remaps `v` from one range to another.
#[inline]
pub fn remap(v: f32, in_min: f32, in_max: f32, out_min: f32, out_max: f32) -> f32 {
    lerp(out_min, out_max, inv_lerp(in_min, in_max, v))
}

/// Smooth Hermite interpolation between two edges.
#[inline]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp01(inv_lerp(edge0, edge1, x));
    t * t * (3.0 - 2.0 * t)
}

/// Returns `-1.0`, `0.0` or `1.0` depending on the sign of `x`.
#[inline]
pub fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// Unity `Mathf.Exp(float)`: `(float)Math.Exp((double)x)`.
///
/// Mono's `Mathf` wrappers evaluate in `double` and cast the result back to
/// `f32`; matching that (instead of `f32::exp`) is parity-critical.
#[inline]
pub fn exp(x: f32) -> f32 {
    f64::from(x).exp() as f32
}

/// Unity `Mathf.Sin(float)`: `(float)Math.Sin((double)x)`.
#[inline]
pub fn sin(x: f32) -> f32 {
    f64::from(x).sin() as f32
}

/// Unity `Mathf.Pow(float, float)`: `(float)Math.Pow((double)f, (double)p)`.
#[inline]
pub fn pow(base: f32, exponent: f32) -> f32 {
    (base as f64).powf(exponent as f64) as f32
}

/// Logistic sigmoid, exactly as `NEATBrain.ExecuteNeuron`
/// (`:625`): `1f / (1f + Mathf.Pow((float)Math.E, 0f - input))`.
///
/// `Mathf.Pow` is evaluated in `double` and cast back to `f32`; the result is
/// overflow-safe because the `double` power saturates before the cast (the
/// reciprocal then rounds to 0/1).
#[inline]
pub fn sigmoid(x: f32) -> f32 {
    // `(float)Math.E` in the game: the f32 constant 2.7182817.
    let base = std::f32::consts::E;
    1.0 / (1.0 + pow(base, -x))
}

/// Hyperbolic tangent: `(f32)Math.Tanh((double)x)`.
///
/// The game does **not** use an `f32` tanh: `tanh(0.4535929f32)` is
/// `0x3ed985aa` via the double path but `0x3ed985a9` via `tanhf` (1 ULP).
#[inline]
pub fn tanh(x: f32) -> f32 {
    f64::from(x).tanh() as f32
}

/// Gaussian-shaped bump, exactly as the game:
/// `1f / (1f + Mathf.Pow(input, 2f))` (double power, cast to `f32`).
#[inline]
pub fn gaussian(x: f32) -> f32 {
    1.0 / (1.0 + pow(x, 2.0))
}

/// Rectified linear unit.
#[inline]
pub fn relu(x: f32) -> f32 {
    if x > 0.0 { x } else { 0.0 }
}

/// Wraps an angle into `[-PI, PI)`.
#[inline]
pub fn wrap_angle(angle: f32) -> f32 {
    let mut a = angle;
    if a >= PI || a < -PI {
        a = (a + PI).rem_euclid(TAU) - PI;
    }
    a
}

/// Shortest signed angular difference `to - from`, in `[-PI, PI)`.
#[inline]
pub fn angle_delta(from: f32, to: f32) -> f32 {
    wrap_angle(to - from)
}

/// Approximate equality for scalars.
#[inline]
pub fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_arithmetic() {
        let a = Vec2::new(1.0, 2.0);
        let b = Vec2::new(3.0, -1.0);
        assert_eq!(a + b, Vec2::new(4.0, 1.0));
        assert_eq!(a - b, Vec2::new(-2.0, 3.0));
        assert_eq!(a * 2.0, Vec2::new(2.0, 4.0));
        assert_eq!(a.dot(b), 1.0);
        assert_eq!(a.cross(b), -7.0);
    }

    #[test]
    fn rotation_and_angles() {
        let v = Vec2::X.rotate(PI / 2.0);
        assert!(v.approx_eq(Vec2::Y, 1e-5));
        assert!((Vec2::Y.angle() - PI / 2.0).abs() < 1e-5);
        let a = Vec2::from_angle(0.7);
        assert!((a.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn normalization_handles_zero() {
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
        assert_eq!(Vec2::ZERO.normalize_or(Vec2::X), Vec2::X);
    }

    #[test]
    fn scalar_helpers() {
        assert_eq!(clamp(5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp01(-0.5), 0.0);
        assert_eq!(lerp(0.0, 10.0, 0.25), 2.5);
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!((sigmoid(100.0) - 1.0).abs() < 1e-6);
        assert!(sigmoid(-100.0) >= 0.0 && sigmoid(-100.0) < 1e-6);
        assert!((gaussian(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn angle_helpers() {
        // Three half-turns wraps to -PI under the [-PI, PI) convention.
        assert!((wrap_angle(3.0 * PI).abs() - PI).abs() < 1e-4);
        assert!((angle_delta(0.2, -0.2) + 0.4).abs() < 1e-6);
    }

    #[test]
    fn unity_math_matches_game_float_rounding() {
        // Unity/Mono evaluates Mathf.Pow / Mathf.Exp / Mathf.Sin and Math.Tanh
        // in double and casts back to f32. These bit patterns pin that (the
        // f32-libm variants are 1 ULP off, e.g. tanh -> 0x3ed985a9).
        assert_eq!(tanh(0.453_592_9).to_bits(), 0x3ed9_85aa);
        assert_eq!(sigmoid(-2.066_895_5).to_bits(), 0x3de6_1b0e);
        assert_eq!(sigmoid(0.0).to_bits(), 0x3f00_0000);
        assert_eq!(sin(1.2).to_bits(), 0x3f6e_9a1d);
        assert_eq!(exp(-0.5).to_bits(), 0x3f1b_4598);
        assert_eq!(gaussian(1.5).to_bits(), 0x3e9d_89d9);
        // Double power saturates before the f32 cast, so no overflow/NaN.
        assert_eq!(sigmoid(-1000.0), 0.0);
        assert_eq!(sigmoid(1000.0), 1.0);
    }
}
