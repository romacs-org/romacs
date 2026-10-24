//! Position newtypes. `BytePos` and `CharPos` are 0-based offsets into text;
//! `LispPos` is the 1-based position Lisp sees (`point-min` = 1).

use std::ops::{Add, Sub};

macro_rules! offset {
    ($name:ident) => {
        /// A zero-based offset. Arithmetic panics on overflow or underflow in
        /// every build profile; subtracting positions requires left >= right.
        /// The value alone does not establish a text boundary or buffer bounds.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);

        impl Add<usize> for $name {
            type Output = Self;
            #[inline]
            fn add(self, n: usize) -> Self {
                Self(self.0.checked_add(n).expect("position overflow"))
            }
        }

        impl Sub<usize> for $name {
            type Output = Self;
            #[inline]
            fn sub(self, n: usize) -> Self {
                Self(self.0.checked_sub(n).expect("position underflow"))
            }
        }

        impl Sub for $name {
            type Output = usize;
            #[inline]
            fn sub(self, other: Self) -> usize {
                self.0.checked_sub(other.0).expect("position underflow")
            }
        }
    };
}

offset!(BytePos);
offset!(CharPos);

/// A 1-based Lisp position; never 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LispPos(usize);

impl LispPos {
    pub const MIN: Self = Self(1);

    #[inline]
    pub fn new(n: usize) -> Option<Self> {
        (n > 0).then_some(Self(n))
    }

    #[inline]
    pub fn get(self) -> usize {
        self.0
    }
}

/// Adds the Lisp position origin. Panics if the offset is `usize::MAX`.
impl From<CharPos> for LispPos {
    #[inline]
    fn from(pos: CharPos) -> Self {
        Self(pos.0.checked_add(1).expect("Lisp position overflow"))
    }
}

impl From<LispPos> for CharPos {
    #[inline]
    fn from(pos: LispPos) -> Self {
        Self(pos.0 - 1)
    }
}
