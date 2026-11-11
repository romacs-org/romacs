//! Shared protocol types between layout, renderer, and runtime crates.

// The wide scene-/glyph-builder and FFI-index constructors in this crate
// (frame_glyphs, scroll_animation, transition_policy) mirror fixed wire/data
// layouts; folding their parameters into structs is a separate refactor, so
// `too_many_arguments` is allowed crate-wide rather than at each of the ~15 sites.
#![allow(clippy::too_many_arguments)]

pub mod call_count;
pub mod animated_visual;
pub(crate) mod child_frame_animation;
pub mod cursor;
pub(crate) mod effect_command;
pub mod effect_config;
pub mod face;
pub mod font;
pub mod frame_chrome;
pub mod frame_glyphs;
pub mod frame_time;
pub mod geometry;
pub mod glyph_matrix;
pub mod gradient;
pub mod image;
pub mod image_diagnostic;
pub mod input_progress;
pub mod interaction_projection;
pub mod menu;
pub mod model;
pub mod modifier_policy;
pub mod motion_spec;
pub mod present_mapping;
pub mod present_trace;
pub mod presentation_origin;
pub(crate) mod presented_frame;
pub mod presented_pointer;
pub mod scene;
pub mod scroll_animation;
pub mod scroll_coverage;
pub mod scroll_input;
pub(crate) mod snapshot_text;
pub mod terminal_color;
pub mod text_animation;
pub(crate) mod toolbar_icon;
pub mod transition_policy;
pub(crate) mod tty_palette;
pub mod types;
pub(crate) mod ui_types;
pub(crate) mod visual_config;
pub(crate) mod window_animation;
pub mod window_chrome;
pub mod x11_colors;
pub(crate) mod xterm_palette;
pub(crate) mod xwidget_extent;
pub use glyph_matrix::*;

pub use effect_command::*;
pub use effect_config::*;
pub use face::*;
pub use frame_chrome::*;
pub use frame_glyphs::*;
pub use geometry::*;
pub use gradient::*;
pub use image::*;
pub use interaction_projection::*;
pub use modifier_policy::*;
pub use present_mapping::*;
pub use window_chrome::*;

pub use child_frame_animation::*;
pub use presented_frame::*;
pub use presented_pointer::*;
pub use scene::*;
pub use scroll_animation::*;
pub use scroll_input::*;
pub use terminal_color::TerminalColor;
pub use toolbar_icon::*;
pub use transition_policy::*;
pub use tty_palette::TtyPalette;
pub use types::*;
pub use ui_types::*;
pub use visual_config::*;
pub use x11_colors::{x11_color_lookup, x11_color_spec_16bit, x11_color_value};
pub use xterm_palette::xterm_256_rgb;
pub use xwidget_extent::*;

#[cfg(test)]
mod tests;
