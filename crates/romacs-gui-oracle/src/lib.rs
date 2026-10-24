//! The GUI acceptance oracle: assert a compositor capture against a declared
//! scene specification.
//!
//! Inputs are the captured pixels, the scene declaration, and the font file
//! the declaration names. Nothing a renderer reports about its own output is
//! read, so the renderer under test cannot grade itself (ruling D-F3).
//!
//! * [`spec`] the declaration;
//! * [`capture`] the pixels;
//! * [`glyphs`] FreeType templates, independent of every renderer under test;
//! * [`extract`] reads a capture back into cells: text, style, colours,
//!   offsets, underline;
//! * [`check`] spec mode: compares a read-back with the declaration;
//! * [`pair`] pair mode: compares the read-backs of two captures.

pub mod capture;
pub mod check;
pub mod extract;
pub mod glyphs;
pub mod pair;
pub mod spec;

pub use capture::Capture;
pub use check::{Failure, Kind, Report, check};
pub use spec::{Rgb, Spec};
