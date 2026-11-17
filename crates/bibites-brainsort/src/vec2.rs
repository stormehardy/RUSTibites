//! A tiny `UnityEngine.Vector2` stand-in with exactly the operators the sorter uses.

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::math;

/// `UnityEngine.Vector2`: two `f32`s with Unity's operator semantics.
///
/// Note that, as in Unity, `Vec2 * Vec2` and `Vec2 / Vec2` are **component-wise** (the sorter
/// relies on this in `Sort` and in the final rescale of `OptimizeHiddenPlacement`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    /// `Vector2.x`.
    pub x: f32,
    /// `Vector2.y`.
    pub y: f32,
}

impl Vec2 {
    /// `Vector2.zero`.
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    /// `Vector2.one`.
    pub const ONE: Vec2 = Vec2 { x: 1.0, y: 1.0 };
    /// `Vector2.kEpsilon`, the magnitude threshold below which `normalized` gives zero.
    pub const K_EPSILON: f32 = 1e-5;

    /// `new Vector2(x, y)`.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }

    /// `Vector2.magnitude`: `(float)Math.Sqrt(x * x + y * y)`.
    #[inline]
    pub fn magnitude(self) -> f32 {
        math::sqrt(self.x * self.x + self.y * self.y)
    }

    /// `Vector2.sqrMagnitude`: `x * x + y * y`.
    #[inline]
    pub fn sqr_magnitude(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// `Vector2.normalized`: the vector divided by its magnitude, or the zero vector when the
    /// magnitude is not greater than [`Vec2::K_EPSILON`] (Unity's `Normalize`).
    #[inline]
    pub fn normalized(self) -> Vec2 {
        let magnitude = self.magnitude();
        if magnitude > Self::K_EPSILON {
            self / magnitude
        } else {
            Vec2::ZERO
        }
    }

    /// `Vector2.Dot(lhs, rhs)`: `lhs.x * rhs.x + lhs.y * rhs.y`.
    #[inline]
    pub fn dot(lhs: Vec2, rhs: Vec2) -> f32 {
        lhs.x * rhs.x + lhs.y * rhs.y
    }

    /// `VectorExtend.Rotate(this Vector2 v, float delta)`: rotation by `delta` radians,
    /// counter-clockwise, evaluated in the same operation order as the C#.
    #[inline]
    pub fn rotate(self, delta: f32) -> Vec2 {
        Vec2::new(
            self.x * math::cos(delta) - self.y * math::sin(delta),
            self.x * math::sin(delta) + self.y * math::cos(delta),
        )
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(0.0 - self.x, 0.0 - self.y)
    }
}

/// Component-wise product (`Vector2 * Vector2`).
impl Mul for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x * rhs.x, self.y * rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, rhs: f32) -> Vec2 {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

impl Mul<Vec2> for f32 {
    type Output = Vec2;
    #[inline]
    fn mul(self, rhs: Vec2) -> Vec2 {
        Vec2::new(rhs.x * self, rhs.y * self)
    }
}

/// Component-wise quotient (`Vector2 / Vector2`).
impl Div for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x / rhs.x, self.y / rhs.y)
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, rhs: f32) -> Vec2 {
        Vec2::new(self.x / rhs, self.y / rhs)
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Vec2) {
        *self = *self + rhs;
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Vec2) {
        *self = *self - rhs;
    }
}

impl MulAssign<f32> for Vec2 {
    #[inline]
    fn mul_assign(&mut self, rhs: f32) {
        *self = *self * rhs;
    }
}

impl DivAssign for Vec2 {
    #[inline]
    fn div_assign(&mut self, rhs: Vec2) {
        *self = *self / rhs;
    }
}

impl DivAssign<f32> for Vec2 {
    #[inline]
    fn div_assign(&mut self, rhs: f32) {
        *self = *self / rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_and_basic_operators() {
        assert_eq!(Vec2::ZERO, Vec2::new(0.0, 0.0));
        assert_eq!(Vec2::ONE, Vec2::new(1.0, 1.0));
        let a = Vec2::new(1.0, 2.0);
        let b = Vec2::new(3.0, -4.0);
        assert_eq!(a + b, Vec2::new(4.0, -2.0));
        assert_eq!(a - b, Vec2::new(-2.0, 6.0));
        assert_eq!(-a, Vec2::new(-1.0, -2.0));
        assert_eq!(Vec2::dot(a, b), -5.0);
    }

    #[test]
    fn vector_vector_product_and_quotient_are_component_wise() {
        let a = Vec2::new(2.0, 3.0);
        let b = Vec2::new(4.0, 5.0);
        assert_eq!(a * b, Vec2::new(8.0, 15.0));
        assert_eq!(b / a, Vec2::new(2.0, 5.0 / 3.0));
        assert_eq!(2.0 * a, a * 2.0);
        let mut c = a;
        c /= b;
        assert_eq!(c, Vec2::new(0.5, 0.6));
    }

    #[test]
    fn magnitude_and_normalized() {
        let v = Vec2::new(3.0, 4.0);
        assert_eq!(v.magnitude(), 5.0);
        assert_eq!(v.sqr_magnitude(), 25.0);
        assert_eq!(v.normalized(), Vec2::new(0.6, 0.8));
    }

    #[test]
    fn normalized_of_tiny_or_zero_vector_is_zero() {
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
        // magnitude 5e-6 <= kEpsilon: Unity returns zero rather than a unit vector.
        assert_eq!(Vec2::new(5e-6, 0.0).normalized(), Vec2::ZERO);
        // NaN magnitude is not > kEpsilon either.
        assert_eq!(Vec2::new(f32::NAN, 1.0).normalized(), Vec2::ZERO);
        // Just above the threshold it is still normalised.
        assert_eq!(Vec2::new(2e-5, 0.0).normalized(), Vec2::new(1.0, 0.0));
    }

    #[test]
    fn rotate_quarter_turn_swaps_axes_like_the_csharp() {
        let r = Vec2::new(1.0, 0.0).rotate(std::f32::consts::PI / 2.0);
        assert!(r.x.abs() < 1e-6, "{r:?}");
        assert_eq!(r.y, 1.0);
        let r = Vec2::new(0.0, 1.0).rotate(std::f32::consts::PI / 2.0);
        assert_eq!(r.x, -1.0);
        assert!(r.y.abs() < 1e-6, "{r:?}");
    }
}
