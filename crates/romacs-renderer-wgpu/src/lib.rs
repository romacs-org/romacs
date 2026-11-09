//! WGPU renderer primitives shared by display backends.

// Renderer entry points and GPU-pipeline builders take many positional
// parameters (geometry, colors, atlas/pipeline handles); folding them into
// structs is a separate refactor, so this bulk category is allowed crate-wide.
#![allow(clippy::too_many_arguments)]

pub mod gpu_policy;
pub mod diagnostics;
pub(crate) mod external_buffer;
pub(crate) mod frame_post;
pub mod glyph_atlas;
#[cfg(feature = "video")]
mod gpu_frame_timing;
pub mod image_cache;
pub use romacs_imaging::{
    image_bands, image_probe, image_scale, image_sequence, svg, svg_animation, xbm, xpm,
};
pub(crate) mod media_budget;
pub mod model;
#[cfg(target_os = "linux")]
pub mod native_presentation;
pub mod renderer;
mod root_gpu_timing;
pub mod scene_graph;
pub mod shader_surface;
pub mod shader_surface_cache;
#[cfg(test)]
#[path = "tests/texture_discipline_test.rs"]
mod texture_discipline_test;
pub(crate) mod tooltip_layout;
pub use image_bands::{BandPlacement, DecodedBand, RasterBand, RowRange, TextureRows};
pub use svg::SvgResourceContext;
pub mod vertex;

#[cfg(feature = "video")]
pub(crate) mod video_cache;

#[cfg(all(feature = "webview", target_os = "linux"))]
pub(crate) mod webview_cache;

#[cfg(all(feature = "webview", target_os = "linux"))]
mod submission_retirement;

#[cfg(all(
    any(feature = "video-dmabuf", feature = "webview"),
    target_os = "linux"
))]
pub(crate) mod vulkan_dmabuf;

pub use gpu_policy::{choose_adapter, new_instance, request_renderer_device};
#[cfg(target_os = "linux")]
pub use external_buffer::DmaBufBuffer;
pub use external_buffer::{BufferFormat, ExternalBuffer, PlatformBuffer, SharedMemoryBuffer};
pub use image_cache::{FilledRows, ImageCache, ImageCacheEvent, ImageMetadata, ImageState};
pub use image_probe::{ImageProbeSource, admit, probe_image_layout};
pub use renderer::{
    BudgetExceeded, CompositionRing, FrameRowDamage, FullFrameTexture, GpuBudgetOwner, PaneBlit,
    PaneSource, RendererFrameEffects, SnapshotLease, SnapshotSize, UnpooledTexture, WgpuRenderer,
    texture_bytes,
};
pub use shader_surface::{
    ShaderValidationError, SurfaceContract, SurfaceUniformInit, compose_surface_wgsl,
    validate_surface_wgsl,
};
pub use tooltip_layout::TooltipLayout;
pub use vertex::{Uniforms};
#[cfg(feature = "video")]
pub use video_cache::{VideoRecoveryManifest};
#[cfg(feature = "video")]
pub(crate) use video_cache::{CachedVideo, VideoCache, VideoState};
#[cfg(all(feature = "webview", target_os = "linux"))]
pub use webview_cache::{CachedWebView, WgpuWebViewCache};
pub use glyph_atlas::{WgpuGlyphAtlas, allocator, pages, types};
pub use shader_surface_cache::ShaderSurfaceCache;

/// Whether the WGPU backend has a concrete rendering path for a graphical
/// face feature.
///
/// Keep this match exhaustive: extending the shared capability domain should
/// fail this crate's build until the renderer makes an explicit decision.
pub const fn supports_graphical_face_attribute(
    attribute: romacs_display_protocol::GraphicalFaceAttribute,
) -> bool {
    use romacs_display_protocol::{GraphicalFaceAttribute, UnderlineStyle};

    match attribute {
        GraphicalFaceAttribute::Foreground
        | GraphicalFaceAttribute::Background
        | GraphicalFaceAttribute::DistantForeground
        | GraphicalFaceAttribute::Stipple
        | GraphicalFaceAttribute::Underline(
            UnderlineStyle::None
            | UnderlineStyle::Line
            | UnderlineStyle::Double
            | UnderlineStyle::Wave
            | UnderlineStyle::Dotted
            | UnderlineStyle::Dashed,
        )
        | GraphicalFaceAttribute::Overline
        | GraphicalFaceAttribute::StrikeThrough
        | GraphicalFaceAttribute::Box
        | GraphicalFaceAttribute::InverseVideo
        | GraphicalFaceAttribute::Extend => true,
    }
}

/// Re-exported effect configuration module for renderer internals and callers.
pub mod effect_config {
    pub use romacs_display_protocol::effect_config::*;
}

