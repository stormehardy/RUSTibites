//! Typed entity identifiers.
//!
//! Raw [`Handle`]s are untyped; wrapping them in newtypes prevents mixing up
//! bibite, egg, pellet and pheromone handles at compile time. A few identifiers
//! are plain integers because they are stable, persistent indices rather than
//! arena slots (species and lineages).

use crate::arena::Handle;
use core::fmt;

macro_rules! handle_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Handle);

        impl $name {
            /// Wraps an arena handle as this identifier type.
            #[inline]
            pub const fn from_handle(handle: Handle) -> Self {
                Self(handle)
            }

            /// Returns the underlying arena handle.
            #[inline]
            pub const fn handle(self) -> Handle {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

handle_id! {
    /// Identifier of a live bibite.
    BibiteId
}
handle_id! {
    /// Identifier of an egg waiting to hatch.
    EggId
}
handle_id! {
    /// Identifier of a matter pellet (plant or meat).
    PelletId
}
handle_id! {
    /// Identifier of a pheromone spot.
    PheromoneId
}
handle_id! {
    /// Identifier of a zone.
    ZoneId
}
handle_id! {
    /// Identifier of a scenario world tool (tower, colour killer, ...).
    ToolId
}

macro_rules! raw_id {
    ($(#[$meta:meta])* $name:ident, $ty:ty) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(pub $ty);

        impl $name {
            /// Creates an identifier from its raw integer value.
            #[inline]
            pub const fn new(value: $ty) -> Self {
                Self(value)
            }

            /// Returns the raw integer value.
            #[inline]
            pub const fn value(self) -> $ty {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

raw_id! {
    /// Identifier of an evolving species.
    SpeciesId,
    u32
}
raw_id! {
    /// Identifier of a lineage (a bibite's ancestral line).
    LineageId,
    u32
}
raw_id! {
    /// Identifier of a matter material (energy, health, plant, meat, ...).
    MaterialId,
    u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Handle;

    #[test]
    fn handle_ids_preserve_handle() {
        let handle = Handle::new(3, 7);
        let id = BibiteId::from_handle(handle);
        assert_eq!(id.handle(), handle);
        assert_eq!(format!("{id:?}"), "BibiteId(3v7)");
    }

    #[test]
    fn raw_ids_expose_value() {
        assert_eq!(SpeciesId::new(42).value(), 42);
        assert_eq!(MaterialId::new(2).0, 2);
    }
}
