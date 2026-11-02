//! Grouped GPU resources owned by `WgpuRenderer`: render pipelines, the
//! stencil clip targets, texture/media caches, and per-frame vertex arenas.

use super::super::image_cache::ImageCache;
use super::super::shader_surface_cache::ShaderSurfaceCache;
#[cfg(feature = "video")]
use super::super::video_cache::VideoCache;
#[cfg(all(feature = "webview", target_os = "linux"))]
use super::super::webview_cache::WgpuWebViewCache;
use super::deferred_resource::DeferredResource;
use super::dynamic_buffer::FrameVertexArena;
use super::full_frame_texture::FullFrameTexture;
use super::gpu_budget::UnpooledTexture;
use super::snapshot_pool::SnapshotSize;
use crate::vertex::{CoverageGlyphVertex, GlyphVertex, RectVertex, RoundedRectVertex};

/// All render pipelines. The `stencil_*` variants are identical to their base
/// counterparts except for stencil state; they draw only where the stencil
/// buffer was written (child frame rounded-corner clipping).
pub(crate) struct Pipelines {
    pub(crate) rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) background_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) transition_background_add: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) transition_composition_marked: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) transition_uncovered_background: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) background_rounded_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) transparent_glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) rounded_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) rounded_fill: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) corner_mask: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) grayscale_glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) subpixel_glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) image: DeferredResource<wgpu::RenderPipeline>,
    pub(super) surface_copy: DeferredResource<wgpu::RenderPipeline>,
    pub(super) native_copy: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) composition: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) crossfade_add: DeferredResource<wgpu::RenderPipeline>,
    pub(super) picture_interpolate: DeferredResource<wgpu::RenderPipeline>,
    #[cfg(feature = "video")]
    pub(crate) bi_planar_video: DeferredResource<wgpu::RenderPipeline>,
    #[cfg(feature = "video")]
    pub(crate) bi_planar_video_copy: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) opaque_image: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_background_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_background_rounded_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_transparent_glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_rounded_rect: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_grayscale_glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_subpixel_glyph: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_image: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_composition: DeferredResource<wgpu::RenderPipeline>,
    #[cfg(feature = "video")]
    pub(crate) stencil_bi_planar_video: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_opaque_image: DeferredResource<wgpu::RenderPipeline>,
    pub(crate) stencil_write: DeferredResource<wgpu::RenderPipeline>,
}

/// The format of the stencil clip target.
///
/// A stencil texel is one byte, not the four a colour attachment costs, so a
/// full-frame target is about 3.7 MB at 2560x1440 rather than 15. Small beside
/// a glyph atlas, and still real: a ceiling is only worth consulting if it can
/// say what is under it, and an allocation nobody counts is headroom the pool
/// believes it can hand out twice.
pub(crate) const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

/// The size a stencil clip target is actually allocated at for a window of
/// `width` by `height` device pixels.
///
/// wgpu rejects a zero extent, so a degenerate window still gets one texel.
/// Named separately from the constructor only so a test with no GPU can ask
/// what the clamp does; the constructor is its sole production caller, which
/// keeps this one description of the size rather than a second one.
pub(crate) fn stencil_clip_size(width: u32, height: u32) -> SnapshotSize {
    SnapshotSize::new(width.max(1), height.max(1))
        .expect("a dimension clamped to at least one is non-zero")
}

/// Stencil texture/view used to clip child frames to rounded corners.
/// Recreated on resize.
pub(crate) struct StencilTargets {
    /// A [`FullFrameTexture`] rather than a raw handle: this is the one
    /// renderer-owned allocation that tracks the window size, so it has to
    /// carry its own census category and cost. The arithmetic that charges it
    /// used to live in a separate function beside the descriptor, which is two
    /// descriptions of one texture and therefore two things that can drift.
    pub(crate) texture: FullFrameTexture,
}

impl StencilTargets {
    /// A clip target covering `width` by `height` device pixels.
    ///
    /// The renderer reaches this only through
    /// `WgpuRenderer::install_stencil_targets`, which reports the GPU budget
    /// in the same statement that installs the result — so there is one place
    /// a stencil target starts being used and it is the place that pays for it.
    ///
    /// Clamps rather than refusing a zero dimension: `resize` already returns
    /// early on one, and the clamp is what lets [`Self::placeholder`] exist at
    /// all.
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = stencil_clip_size(width, height);
        Self {
            texture: FullFrameTexture::allocate(
                device,
                UnpooledTexture::StencilClip,
                "Stencil Texture",
                size,
                STENCIL_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
        }
    }

    /// A one-texel target for the moment before the renderer exists.
    ///
    /// The renderer is built by a struct literal, and a literal cannot call
    /// the method that reports the budget. Starting from a target that costs a
    /// byte, then installing the real one, keeps `install_stencil_targets` the
    /// single allocation site rather than the second of two.
    pub(crate) fn placeholder(device: &wgpu::Device) -> Self {
        Self::new(device, 1, 1)
    }
}

/// Texture/media caches.
pub(crate) struct RenderCaches {
    pub(crate) image: ImageCache,
    #[cfg(feature = "video")]
    pub(crate) video: VideoCache,
    #[cfg(all(feature = "webview", target_os = "linux"))]
    pub(crate) webview: WgpuWebViewCache,
    pub(crate) surface: ShaderSurfaceCache,
}

/// Per-frame reusable vertex upload arenas.
///
/// Inside an open frame submission (`frame_submission`) the arenas reset once,
/// when the frame opens, and every pass bump-allocates after the previous one:
/// the frame's uploads all land before its single submission, so a mid-frame
/// reset would overwrite vertices an earlier pass still reads. Outside a frame
/// each pass submits before returning and the queue is in order, so a pass
/// may reset them at its start.
pub(crate) struct VertexArenas {
    pub(crate) glyph: FrameVertexArena<GlyphVertex>,
    pub(crate) coverage: FrameVertexArena<CoverageGlyphVertex>,
    /// Textured quads drawn through the image pipelines (inline/floating
    /// images, videos, WebViews, blits, transition quads).
    pub(crate) image: FrameVertexArena<GlyphVertex>,
    /// Solid-color quads (backgrounds, cursors, decorations, effects).
    pub(crate) rect: FrameVertexArena<RectVertex>,
    /// SDF rounded-rect quads (box fills/borders, scroll thumbs, masks).
    pub(crate) rounded: FrameVertexArena<RoundedRectVertex>,
    /// `buffers_created` total at the previous per-frame snapshot.
    created_snapshot: u64,
}

impl VertexArenas {
    pub(crate) fn new() -> Self {
        Self {
            glyph: FrameVertexArena::new("Glyph Vertex Arena"),
            coverage: FrameVertexArena::new("Coverage Glyph Vertex Arena"),
            image: FrameVertexArena::new("Image Vertex Arena"),
            rect: FrameVertexArena::new("Rect Vertex Arena"),
            rounded: FrameVertexArena::new("Rounded Rect Vertex Arena"),
            created_snapshot: 0,
        }
    }

    pub(crate) fn begin_frame(&mut self) {
        self.glyph.begin_frame();
        self.coverage.begin_frame();
        self.image.begin_frame();
        self.rect.begin_frame();
        self.rounded.begin_frame();
    }

    fn buffers_created_total(&self) -> u64 {
        self.glyph.buffers_created()
            + self.coverage.buffers_created()
            + self.image.buffers_created()
            + self.rect.buffers_created()
            + self.rounded.buffers_created()
    }

    /// GPU buffer allocations since the previous snapshot (i.e. this frame).
    /// Zero in steady state once the arenas reach their high-water capacity.
    pub(crate) fn buffers_created_since_snapshot(&mut self) -> u64 {
        let total = self.buffers_created_total();
        let delta = total - self.created_snapshot;
        self.created_snapshot = total;
        delta
    }
}

#[cfg(test)]
#[path = "resources/tests/resources_test.rs"]
mod tests;

#[cfg(test)]
mod deferred_tests {
    use super::super::WgpuRenderer;
    #[test]
    fn unused_pipelines_wait_until_first_use_and_all_compile_when_requested() {
        let renderer = WgpuRenderer::new(None, 80, 40).expect("hardware/offscreen renderer");
        let pipelines = [
            ("rect", &renderer.pipelines.rect),
            ("background_rect", &renderer.pipelines.background_rect),
            (
                "transition_background_add",
                &renderer.pipelines.transition_background_add,
            ),
            (
                "transition_composition_marked",
                &renderer.pipelines.transition_composition_marked,
            ),
            (
                "transition_uncovered_background",
                &renderer.pipelines.transition_uncovered_background,
            ),
            (
                "background_rounded_rect",
                &renderer.pipelines.background_rounded_rect,
            ),
            ("transparent_glyph", &renderer.pipelines.transparent_glyph),
            ("rounded_rect", &renderer.pipelines.rounded_rect),
            ("rounded_fill", &renderer.pipelines.rounded_fill),
            ("corner_mask", &renderer.pipelines.corner_mask),
            ("glyph", &renderer.pipelines.glyph),
            ("grayscale_glyph", &renderer.pipelines.grayscale_glyph),
            ("subpixel_glyph", &renderer.pipelines.subpixel_glyph),
            ("image", &renderer.pipelines.image),
            ("surface_copy", &renderer.pipelines.surface_copy),
            ("native_copy", &renderer.pipelines.native_copy),
            ("composition", &renderer.pipelines.composition),
            ("crossfade_add", &renderer.pipelines.crossfade_add),
            (
                "picture_interpolate",
                &renderer.pipelines.picture_interpolate,
            ),
            #[cfg(feature = "video")]
            ("bi_planar_video", &renderer.pipelines.bi_planar_video),
            #[cfg(feature = "video")]
            (
                "bi_planar_video_copy",
                &renderer.pipelines.bi_planar_video_copy,
            ),
            ("opaque_image", &renderer.pipelines.opaque_image),
            (
                "stencil_background_rect",
                &renderer.pipelines.stencil_background_rect,
            ),
            (
                "stencil_background_rounded_rect",
                &renderer.pipelines.stencil_background_rounded_rect,
            ),
            (
                "stencil_transparent_glyph",
                &renderer.pipelines.stencil_transparent_glyph,
            ),
            ("stencil_rect", &renderer.pipelines.stencil_rect),
            (
                "stencil_rounded_rect",
                &renderer.pipelines.stencil_rounded_rect,
            ),
            (
                "stencil_grayscale_glyph",
                &renderer.pipelines.stencil_grayscale_glyph,
            ),
            (
                "stencil_subpixel_glyph",
                &renderer.pipelines.stencil_subpixel_glyph,
            ),
            ("stencil_image", &renderer.pipelines.stencil_image),
            ("stencil_composition", &renderer.pipelines.stencil_composition),
            #[cfg(feature = "video")]
            (
                "stencil_bi_planar_video",
                &renderer.pipelines.stencil_bi_planar_video,
            ),
            (
                "stencil_opaque_image",
                &renderer.pipelines.stencil_opaque_image,
            ),
            ("stencil_write", &renderer.pipelines.stencil_write),
        ];
        for (name, pipeline) in &pipelines {
            assert!(!pipeline.is_ready(), "{name} compiled before first use");
        }
        let errors = renderer
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        for (name, pipeline) in pipelines {
            let _ = std::ops::Deref::deref(pipeline);
            assert!(pipeline.is_ready(), "{name} did not finish compilation");
        }
        assert!(pollster::block_on(errors.pop()).is_none());
    }
}
