//! Game version strings and ordering.
//!
//! The original game's `Utility.Version` (`Here/Utility/Version.cs`) has five
//! components: `v0.v1.v2.v3` plus an *alpha* number. A string like `0.6.3a3`
//! parses as `(0, 6, 3, 0, alpha=3)`, while `0.6.3.1` parses as
//! `(0, 6, 3, 1, alpha=0)`. A release (`alpha == 0`) sorts **above** the same
//! numeric version with an alpha suffix, and a larger alpha sorts above a
//! smaller one.
//!
//! The version written to newly exported files is [`PRESENT`] (`"0.6.3.1"`, the
//! installed game, confirmed by the parity bridge). See
//! `docs/spec/04-formats-settings.md` section 8.1.

use core::cmp::Ordering;
use core::fmt;

use bibites_core::{Error, Result};

/// The version string written by this crate's writers (`Application.version` of
/// the installed game; spec 04 section 0.1).
pub const PRESENT: &str = "0.6.3.1";

/// A parsed game version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Version {
    /// First component.
    pub v0: u32,
    /// Second component.
    pub v1: u32,
    /// Third component.
    pub v2: u32,
    /// Fourth component (release builds only, e.g. `0.6.3.1`).
    pub v3: u32,
    /// Alpha number, 0 for releases (`0.6.3a3` has `alpha = 3`).
    pub alpha: u32,
}

impl Version {
    /// Creates a version from its components.
    #[must_use]
    pub const fn new(v0: u32, v1: u32, v2: u32, v3: u32, alpha: u32) -> Self {
        Self {
            v0,
            v1,
            v2,
            v3,
            alpha,
        }
    }

    /// Parses a version string with the game's exact grammar
    /// (`^[0-9]+(\.[0-9]+)(\.[0-9]+)?(\.[0-9]+)?([aA][0-9]+)?$`).
    ///
    /// Returns a [`Error::Unsupported`] error when the string is not a version.
    pub fn parse(text: &str) -> Result<Self> {
        let mut v = Self::default();
        let bytes = text.as_bytes();
        let mut index = 0usize;
        let mut component = 0usize;
        let mut last_was_dot = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte.is_ascii_digit() {
                let start = index;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                let value: u32 = text[start..index].parse().map_err(|_| {
                    Error::unsupported(format!("version component overflows: {text}"))
                })?;
                if component >= 4 {
                    return Err(Error::unsupported(format!(
                        "too many version components: {text}"
                    )));
                }
                match component {
                    0 => v.v0 = value,
                    1 => v.v1 = value,
                    2 => v.v2 = value,
                    _ => v.v3 = value,
                }
                component += 1;
                last_was_dot = false;
            } else if byte == b'.' {
                // A dot must follow a number and there must be a next character.
                if last_was_dot || index + 1 >= bytes.len() || !(1..4).contains(&component) {
                    return Err(Error::unsupported(format!(
                        "invalid version string: {text}"
                    )));
                }
                index += 1;
                last_was_dot = true;
            } else if byte == b'a' || byte == b'A' {
                if component < 2 || last_was_dot {
                    return Err(Error::unsupported(format!(
                        "invalid version string: {text}"
                    )));
                }
                index += 1;
                let start = index;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                if start == index {
                    return Err(Error::unsupported(format!(
                        "alpha version without a number: {text}"
                    )));
                }
                v.alpha = text[start..index].parse().map_err(|_| {
                    Error::unsupported(format!("alpha component overflows: {text}"))
                })?;
                if index != bytes.len() {
                    return Err(Error::unsupported(format!(
                        "trailing characters in version: {text}"
                    )));
                }
                return Ok(v);
            } else {
                return Err(Error::unsupported(format!(
                    "invalid version string: {text}"
                )));
            }
        }
        if component < 2 || last_was_dot {
            return Err(Error::unsupported(format!(
                "invalid version string: {text}"
            )));
        }
        Ok(v)
    }

    /// Compares two versions exactly like the game (`Version.operator<`).
    fn cmp_game(self, other: Self) -> Ordering {
        let numeric = self
            .v0
            .cmp(&other.v0)
            .then(self.v1.cmp(&other.v1))
            .then(self.v2.cmp(&other.v2))
            .then(self.v3.cmp(&other.v3));
        if numeric != Ordering::Equal {
            return numeric;
        }
        // Alpha 0 (a release) sorts above any alpha > 0; larger alphas sort
        // above smaller ones.
        match (self.alpha, other.alpha) {
            (0, 0) => Ordering::Equal,
            (0, _) => Ordering::Greater,
            (_, 0) => Ordering::Less,
            (a, b) => a.cmp(&b),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.v0, self.v1, self.v2)?;
        if self.v3 != 0 {
            write!(f, ".{}", self.v3)?;
        }
        if self.alpha != 0 {
            write!(f, "a{}", self.alpha)?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.cmp_game(*other)
    }
}

impl core::str::FromStr for Version {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// The installed game's version (`0.6.3.1`; spec 04 section 0.1).
#[must_use]
pub fn present() -> Version {
    Version::new(0, 6, 3, 1, 0)
}

/// What the game makes of a save written by `from` (`VersionTracker.CanUpdateFromVersion` / `ChangesSinceVersion` over its list of save file changes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveCompat {
    /// Written by this structure of files: loads without a word.
    Current,
    /// Structure changes since: the game warns ("we tried updating it, but you might still run into issues").
    Updated,
    /// Older than 0.4: the game refuses ("incompatible with the present version").
    Incompatible,
}

/// The game's verdict on a save of version `from`. Its four-number versions are alphas (`new Version(0, 6, 0, 15)` is `0.6a15`); the newest change in
/// its list is `0.6a15`, and `0.4` and `0.3` cannot be updated from.
#[must_use]
pub fn save_compat(from: Version) -> SaveCompat {
    if from < Version::new(0, 4, 0, 0, 0) {
        SaveCompat::Incompatible
    } else if Version::new(0, 6, 0, 0, 15) > from {
        SaveCompat::Updated
    } else {
        SaveCompat::Current
    }
}

/// Returns true when `from` predates `threshold` and therefore needs the
/// migration guarded by that threshold.
#[must_use]
pub fn before(from: Version, threshold: Version) -> bool {
    from < threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_saves_are_updated_or_refused_as_the_games_version_tracker_does() {
        let compat = |text: &str| save_compat(Version::parse(text).unwrap());
        assert_eq!(compat("0.6.3.1"), SaveCompat::Current);
        assert_eq!(compat("0.6a15"), SaveCompat::Current);
        assert_eq!(compat("0.6a14"), SaveCompat::Updated);
        assert_eq!(compat("0.5a1"), SaveCompat::Updated);
        assert_eq!(compat("0.3.9"), SaveCompat::Incompatible);
    }

    #[test]
    fn parses_release_and_alpha_versions() {
        assert_eq!(
            Version::parse("0.6.3.1").unwrap(),
            Version::new(0, 6, 3, 1, 0)
        );
        assert_eq!(Version::parse("0.6").unwrap(), Version::new(0, 6, 0, 0, 0));
        assert_eq!(
            Version::parse("0.6.3").unwrap(),
            Version::new(0, 6, 3, 0, 0)
        );
        assert_eq!(
            Version::parse("0.6.3a3").unwrap(),
            Version::new(0, 6, 3, 0, 3)
        );
        assert_eq!(
            Version::parse("0.6a15").unwrap(),
            Version::new(0, 6, 0, 0, 15)
        );
        assert_eq!(Version::parse("0.5").unwrap(), Version::new(0, 5, 0, 0, 0));
    }

    #[test]
    fn rejects_invalid_versions() {
        for input in [
            "",
            "0",
            "a1",
            "0.6.a3",
            "0.6.3a",
            "0.6.3x",
            "0.6.3.1.2",
            "1.2.3 4",
        ] {
            assert!(
                Version::parse(input).is_err(),
                "{input:?} should be rejected"
            );
        }
    }

    #[test]
    fn display_round_trips() {
        for text in ["0.6.3.1", "0.6.3a3", "0.6.0a15", "0.5.0.4"] {
            let version = Version::parse(text).unwrap();
            assert_eq!(version.to_string(), text);
        }
        // Zero alpha and zero v3 are omitted.
        assert_eq!(Version::parse("0.6.0.0").unwrap().to_string(), "0.6.0");
    }

    #[test]
    fn ordering_matches_the_game() {
        let release = Version::parse("0.6.3.1").unwrap();
        let alpha = Version::parse("0.6.3a3").unwrap();
        let older = Version::parse("0.6.2a1").unwrap();
        assert!(alpha < release);
        assert!(older < alpha);
        assert!(Version::parse("0.6").unwrap() > Version::parse("0.6a15").unwrap());
        assert_eq!(
            Version::parse("0.6.3.1").unwrap().cmp_game(present()),
            Ordering::Equal
        );
        assert!(before(older, present()));
        assert!(!before(present(), present()));
    }
}
