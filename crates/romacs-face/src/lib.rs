//! Romacs faces and themes as immutable data.
//!
//! A face is a plain attribute record; a theme is an immutable value.
//!
//! * `defface` condition specs are collapsed once, at load, for the one
//!   render target (graphical, truecolour, pgtk); only the background mode
//!   remains a live choice ([`Collapsed`]).
//! * A [`Theme`] resolves `:inherit` once, topologically, and refuses to
//!   exist if the graph has a cycle. There is no recursive merge anywhere a
//!   cyclic input could reach: Lisp structure is converted to an acyclic,
//!   depth/size-bounded [`BoundedDatum`] at every data boundary.
//! * Every theme, remapping and palette carries a [`Generation`] minted at
//!   construction. Changing anything yields a new value with a new
//!   generation, so caches keyed by generation ([`FaceCache`]) cannot serve
//!   stale data and need no manually bumped counter.
//! * Switching themes swaps one value for another ([`ActiveTheme`]).
//!
//! This crate depends on no evaluator and no layout engine. It holds no
//! Lisp handles: the moving collector has nothing here to trace.
//! GNU-DIVERGENCES.md lists every deliberate behavioural difference.
//!
//! The evaluator keeps faces in GNU's lface vectors and realises them through
//! this crate's merge on demand ([`LiveView`]). [`Theme`] and spec collapse
//! are the reference model the parity tests hold against pinned GNU.

pub mod attr;
pub mod cache;
pub mod datum;
pub mod palette;
pub mod remap;
pub mod registry;
pub mod spec;
pub mod theme;

pub use attr::{Attribute, Attrs, AttrsBuilder, Height, Slot, Value};
pub use cache::FaceCache;
pub use datum::{BoundedDatum, Datum, DatumError, LispGraph, NodeView};
pub use palette::{Palette, PaletteError, PaletteValue};
pub use remap::{FaceRef, FaceSource, LiveView, Remap, View};
pub use spec::{Background, Collapsed, FaceDef, FaceError};
pub use theme::{
    ActiveTheme, Base, Deffaces, Generation, Inherit, Theme, ThemeSpec, UserOverrides,
};
