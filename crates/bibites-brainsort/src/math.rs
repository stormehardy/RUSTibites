//! Bit-faithful mirrors of the `UnityEngine.Mathf` / `System.Single` helpers that the C#
//! sorter calls. Unity's float helpers are *not* the same as Rust's `f32` methods when NaN or
//! signed zero is involved (`Mathf.Sign(NaN)` is `-1`, `Mathf.Min(NaN, x)` is `x`, ...), and
//! several of them round-trip through `double`, so each one is spelled out here once.

use std::cmp::Ordering;

/// `Mathf.Sign(f)`: `f >= 0f ? 1f : -1f` (so `Sign(NaN) == -1` and `Sign(-0f) == 1`).
#[inline]
pub(crate) fn sign(f: f32) -> f32 {
    if f >= 0.0 { 1.0 } else { -1.0 }
}

/// `Mathf.Min(a, b)`: `a < b ? a : b` (returns `b` whenever either side is NaN).
#[inline]
pub(crate) fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `Mathf.Max(a, b)`: `a > b ? a : b` (returns `b` whenever either side is NaN).
#[inline]
pub(crate) fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `Mathf.Abs(f)` (`Math.Abs(float)`).
#[inline]
pub(crate) fn abs(f: f32) -> f32 {
    f.abs()
}

/// `Mathf.Sqrt(f)`: `(float)Math.Sqrt((double)f)`.
#[inline]
pub(crate) fn sqrt(f: f32) -> f32 {
    f64::from(f).sqrt() as f32
}

/// `Mathf.Exp(f)`: `(float)Math.Exp((double)f)`.
#[inline]
pub(crate) fn exp(f: f32) -> f32 {
    f64::from(f).exp() as f32
}

/// `Mathf.Pow(f, p)`: `(float)Math.Pow((double)f, (double)p)`.
#[inline]
pub(crate) fn pow(f: f32, p: f32) -> f32 {
    f64::from(f).powf(f64::from(p)) as f32
}

/// `Mathf.Cos(f)`: `(float)Math.Cos((double)f)`.
#[inline]
pub(crate) fn cos(f: f32) -> f32 {
    f64::from(f).cos() as f32
}

/// `Mathf.Sin(f)`: `(float)Math.Sin((double)f)`.
#[inline]
pub(crate) fn sin(f: f32) -> f32 {
    f64::from(f).sin() as f32
}

/// `float.CompareTo(float)` as used by `Comparer<float>.Default` (and therefore by LINQ's
/// `OrderBy`/`OrderByDescending` on float keys): NaN sorts before every other value and equals
/// NaN, `-0f` equals `+0f`. This is a valid total preorder, so it is safe for `sort_by`.
#[inline]
pub(crate) fn compare_f32(a: f32, b: f32) -> Ordering {
    if a < b {
        Ordering::Less
    } else if a > b {
        Ordering::Greater
    } else if a == b {
        Ordering::Equal
    } else if a.is_nan() {
        if b.is_nan() {
            Ordering::Equal
        } else {
            Ordering::Less
        }
    } else {
        Ordering::Greater
    }
}

/// `List<T>.Remove(item)` for a list of plain values: removes the *first* element equal to
/// `item` (reference identity in the C#, arena-index equality here) and reports whether one
/// was removed. Order of the remaining elements is preserved.
#[inline]
pub(crate) fn remove_first<T: PartialEq>(list: &mut Vec<T>, item: &T) -> bool {
    match list.iter().position(|x| x == item) {
        Some(pos) => {
            list.remove(pos);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_matches_unity() {
        assert_eq!(sign(2.0), 1.0);
        assert_eq!(sign(0.0), 1.0);
        assert_eq!(sign(-0.0), 1.0);
        assert_eq!(sign(-3.0), -1.0);
        // `NaN >= 0f` is false, so Unity answers -1 (Rust's signum would answer NaN).
        assert_eq!(sign(f32::NAN), -1.0);
    }

    #[test]
    fn min_max_follow_unity_ternaries() {
        assert_eq!(min(1.0, 2.0), 1.0);
        assert_eq!(max(1.0, 2.0), 2.0);
        // Either NaN makes the comparison false, so the *second* argument comes back.
        assert_eq!(min(1.0, f32::NAN).to_bits(), f32::NAN.to_bits());
        assert_eq!(min(f32::NAN, 1.0), 1.0);
        assert_eq!(max(f32::NAN, 1.0), 1.0);
        assert!(max(1.0, f32::NAN).is_nan());
    }

    #[test]
    fn compare_is_float_compare_to() {
        assert_eq!(compare_f32(1.0, 2.0), Ordering::Less);
        assert_eq!(compare_f32(2.0, 1.0), Ordering::Greater);
        assert_eq!(compare_f32(0.0, -0.0), Ordering::Equal);
        assert_eq!(compare_f32(f32::NAN, f32::NAN), Ordering::Equal);
        assert_eq!(compare_f32(f32::NAN, f32::NEG_INFINITY), Ordering::Less);
        assert_eq!(compare_f32(f32::NEG_INFINITY, f32::NAN), Ordering::Greater);
    }

    #[test]
    fn remove_first_only_takes_one_occurrence() {
        let mut v = vec![3, 1, 3, 2];
        assert!(remove_first(&mut v, &3));
        assert_eq!(v, vec![1, 3, 2]);
        assert!(!remove_first(&mut v, &9));
        assert_eq!(v, vec![1, 3, 2]);
    }

    #[test]
    fn pow_goes_through_double() {
        // 0.6f is 0.60000002384..., not 0.6: the double round-trip is observable.
        let direct = f64::from(0.6f32).powf(0.5) as f32;
        assert_eq!(pow(0.6, 0.5), direct);
    }
}
