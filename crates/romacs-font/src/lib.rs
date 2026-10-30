//! Evaluator-free font resolution.
//!
//! A display that never links an evaluator must still turn a face — family,
//! weight, slant, width, size — and the characters it draws into the exact
//! fonts the renderer replays. This crate does that from plain data:
//!
//! - [`FontRequest`] is a face's font attributes, already realized;
//! - [`FontsetPolicy`] is a fontset as Rust data (sorted character ranges to
//!   ordered font specs, plus a fallback list), the shape `set-fontset-font`
//!   edits;
//! - [`FontResolver`] answers both through the platform catalog and publishes
//!   [`romacs_display_protocol::font::ResolvedFont`] records and per-character
//!   [`romacs_display_protocol::font::ResolvedCharGlyph`] bindings.
//!
//! No Lisp value crosses this boundary, and none can: the crate does not
//! depend on any evaluator. Layout and rendering both depend on it, which is
//! why the renderer's font helpers live here too.

pub mod cluster;
pub mod entity;
pub mod font_match;
#[cfg(target_os = "linux")]
mod fontconfig_catalog;
mod fontset;
mod frame;
pub mod instance;
pub mod probe;
mod resolver;
mod style;
pub mod subpixel;
pub mod system;

pub use fontset::{FontsetEntry, FontsetFontSpec, FontsetMode, FontsetPolicy, FontsetTarget};
pub use frame::{PublishedFace, publish_face_text};
pub use resolver::{CharResolution, CharResolutionSource, FontResolveError, FontResolver};
pub use style::{FontRequest, StyleScore, style_score};

/// Make a font file visible to this process's platform catalog, the way an
/// application font directory would. Returns whether the catalog accepted it.
///
/// Registration is additive. Resolvers created afterwards observe it; an
/// existing resolver keeps the answers it already cached.
pub fn register_font_file(path: &std::path::Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        fontconfig_catalog::register_font_file(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}
