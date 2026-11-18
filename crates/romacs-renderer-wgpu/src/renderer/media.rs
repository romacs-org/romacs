//! Media methods for WgpuRenderer.

use super::WgpuRenderer;
use romacs_display_protocol::{
    ImageAnimationPolicy, ImageColorContext, ImageFrameIndex, ImageId, ImageLoadToken,
    ImageMaskPolicy, ImageRealization, ImageRotation, ImageSequenceId, ImageSizeSpec,
};

impl WgpuRenderer {
    pub fn upload_semantic_image(
        &mut self,
        load: ImageLoadToken,
        decoded: &crate::image_cache::SemanticImageDecoded,
    ) -> crate::image_cache::ImageCacheEvent {
        self.caches
            .image
            .upload_semantic_image(&self.device, &self.queue, load, decoded)
    }

    /// Retire completed timestamp queries and reserve one bounded slot for a
    /// frame-content pass that can actually draw video.
    ///
    /// Root and child frames use different renderers, so this shared seam is
    /// the only place allowed to begin the opt-in GPU timing transaction.

    /// Resolve and submit one timed video-bearing frame-content pass.
    ///
    /// Keeping queue submission and presentation accounting together prevents
    /// a render path from reporting submitted frames while silently omitting
    /// the timestamp query that describes the same GPU work.

    /// Exact image texture and decoded animation-sequence bytes currently
    /// retained by the renderer.
    pub fn image_cache_usage(&self) -> romacs_display_protocol::ImageCacheUsage {
        self.caches.image.memory_usage()
    }

    /// Realize chrome using its owning window's scale, not the last drawn window.
    pub fn load_toolbar_icon(&mut self, key: &romacs_display_protocol::ToolBarIconKey) -> ImageId {
        match key.source() {
            romacs_display_protocol::ToolBarImageSource::File { path } => {
                self.caches.image.load_file(
                    path,
                    key.size_spec(),
                    ImageRotation::None,
                    key.colors(),
                    key.realization().device_scale(),
                )
            }
        }
    }

    /// Load image from data with pre-allocated ID (for threaded mode)
    #[allow(clippy::too_many_arguments)]
    pub fn load_image_data_with_id(
        &mut self,
        load: ImageLoadToken,
        data: romacs_display_protocol::image::EncodedBytes,
        size: ImageSizeSpec,
        rotation: ImageRotation,
        realization: ImageRealization,
        colors: ImageColorContext,
        mask: ImageMaskPolicy,
        animation: ImageAnimationPolicy,
        frame: ImageFrameIndex,
        sequence: ImageSequenceId,
        resources: crate::SvgResourceContext,
        identity: romacs_display_protocol::image_diagnostic::ImageLoadIdentity,
    ) {
        self.caches.image.load_data_with_id(
            load,
            data,
            size,
            rotation,
            realization,
            colors,
            mask,
            animation,
            frame,
            sequence,
            resources,
            identity,
        )
    }

    /// Load image from raw ARGB32 pixel data with pre-allocated ID (for threaded mode)
    pub fn load_image_argb32_with_id(
        &mut self,
        load: ImageLoadToken,
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) {
        self.caches
            .image
            .load_raw_argb32_with_id(load, data, width, height, stride)
    }

    /// Get image dimensions (works for pending and loaded images)
    pub fn get_image_size(&self, image: ImageId) -> Option<(u32, u32)> {
        self.caches
            .image
            .get_dimensions(image)
            .map(|extent| extent.dimensions())
    }

    /// Check if image is ready for rendering
    pub fn is_image_ready(&self, image: ImageId) -> bool {
        self.caches.image.is_ready(image)
    }

    pub fn has_pending_images(&self) -> bool {
        self.caches.image.has_pending()
    }

    /// Begin presentation-safe retirement of an image texture.
    pub fn retire_image(&mut self, image: ImageId) {
        self.caches.image.retire(image)
    }

    /// Replace the complete set of image identities retained by render
    /// presentations, releasing any retirement whose last reference vanished.
    pub fn synchronize_retained_images(
        &mut self,
        retained: romacs_display_protocol::RetainedImageSet,
    ) {
        self.caches.image.synchronize_retained_images(retained);
    }

    /// Process pending decoded images (call each frame before rendering)
    pub fn process_pending_images(&mut self) -> Vec<crate::ImageCacheEvent> {
        self.caches.image.process_pending(&self.device, &self.queue)
    }

    /// Load video from file path (async - returns immediately)
    /// Returns video ID, frames decode in background

    /// Load video from file path with a pre-allocated ID.

    /// Load video from URI with a pre-allocated ID.

    /// Open one stable editor video session from a fully typed request.

    /// Apply one typed playback transition to an existing video session.

    /// Close one stable editor video session.

    /// Snapshot the native video pipeline without exposing renderer handles.

    /// Acknowledge a new native-video diagnostics measurement epoch.

    /// Begin evidence collection for one swapchain-bound render transaction.

    /// Discard pending submission evidence when the swapchain render did not finish.

    /// Record surface handoff after queue.present returns for these video draws.

    /// Get video dimensions

    /// Get video state

    /// Play video

    /// Pause video

    /// Stop video

    /// Set video loop count (-1 for infinite)

    /// Free a video from cache




    /// Get cached video for rendering

    // =========== Shader surfaces (docs/display-engine/SHADER_SURFACES.md) ===========

    /// Create a shader surface from user WGSL. The composite bind group uses
    /// the image cache's layout/sampler so the inline-media phase can draw it
    /// with the shared image pipeline. Texture is allocated at physical
    /// resolution (logical size x current scale factor).
    #[allow(clippy::too_many_arguments)]
    pub fn create_shader_surface(
        &mut self,
        id: u32,
        language: crate::shader_surface::SurfaceShaderLanguage,
        user_source: &str,
        uniforms: &[crate::shader_surface::SurfaceUniformInit],
        width: u32,
        height: u32,
        animate: bool,
        fps: Option<u32>,
        channel0: Option<crate::shader_surface::SurfaceChannelSource>,
        recreatable: bool,
    ) -> Result<(), String> {
        let layout = self.caches.image.bind_group_layout();
        let sampler = self.caches.image.sampler();
        let (width_px, height_px) = self.caches.surface.create_shader(
            &self.device,
            layout,
            sampler,
            self.surface_format,
            id,
            language,
            user_source,
            uniforms,
            width,
            height,
            self.scale_factor,
            animate,
            fps,
            channel0,
        )?;
        self.register_surface_bytes(id, width_px, height_px, recreatable);
        Ok(())
    }

    /// Register a created surface's physical bytes and run the eviction
    /// driver: over budget, free least-recently-used *recreatable* surfaces
    /// (declarative specs re-resolve on the next redisplay walk). One victim
    /// per iteration so the shrinking shortfall requeries the candidate
    /// prefix; never the surface just created.
    pub(super) fn register_surface_bytes(
        &mut self,
        id: u32,
        width_px: u32,
        height_px: u32,
        recreatable: bool,
    ) {
        use crate::media_budget::MediaType;
        let bytes = width_px as usize * height_px as usize * 4;
        self.media_budget.register(MediaType::Surface, id, bytes);
        self.surface_recreatable.insert(id, recreatable);
        while self.media_budget.is_over_budget() {
            let victim = self
                .media_budget
                .get_eviction_candidates(0)
                .into_iter()
                .find(|(kind, victim_id)| {
                    *kind == MediaType::Surface
                        && *victim_id != id
                        && self.surface_recreatable.get(victim_id) == Some(&true)
                });
            let Some((_, victim_id)) = victim else {
                tracing::debug!(
                    "media budget over limit ({} / {} bytes) with no \
                     recreatable shader surface to evict",
                    self.media_budget.current_usage(),
                    self.media_budget.max_limit()
                );
                break;
            };
            self.caches.surface.free(victim_id);
            self.media_budget.unregister(MediaType::Surface, victim_id);
            self.surface_recreatable.remove(&victim_id);
            tracing::info!("evicting shader surface {victim_id} (over media budget)");
        }
    }

    /// Set the unified media budget limit (bytes).
    pub fn set_media_budget_limit(&mut self, max_bytes: usize) {
        self.media_budget = crate::media_budget::MediaBudget::with_max_bytes(max_bytes);
    }

    /// Drain the caches' accounting events into the budget. Runs once per
    /// frame from `process_shader_surfaces`.
    fn reconcile_media_budget(&mut self) {
        for event in self.caches.image.drain_accounting() {
            self.media_budget.apply(event);
        }
    }

    /// Update one named uniform on a shader surface.
    pub fn set_surface_uniform(&mut self, id: u32, name: &str, value: [f32; 4]) {
        self.caches.surface.set_uniform(id, name, value);
    }

    /// Resize shader surface `id` to a logical size, keeping its compiled
    /// pipeline. Whether it had one to resize.
    pub fn resize_shader_surface(&mut self, id: u32, width: u32, height: u32) -> bool {
        let layout = self.caches.image.bind_group_layout();
        let sampler = self.caches.image.sampler();
        let resized = self.caches.surface.resize(
            &self.device,
            layout,
            sampler,
            self.surface_format,
            id,
            width,
            height,
        );
        if let Some((width_px, height_px)) = resized {
            let recreatable = self.surface_recreatable.get(&id).copied().unwrap_or(false);
            self.media_budget
                .unregister(crate::media_budget::MediaType::Surface, id);
            self.register_surface_bytes(id, width_px, height_px, recreatable);
        }
        resized.is_some()
    }

    /// Whether surface `id` exists (it may have been freed by an eviction
    /// or lost with the device).
    #[must_use]
    pub fn has_surface(&self, id: u32) -> bool {
        self.caches.surface.get(id).is_some()
    }

    /// Whether any shader surface exists, so a pointer path can skip searching
    /// a frame's glyphs for one when the answer could only be "none".
    #[must_use]
    pub fn has_shader_surfaces(&self) -> bool {
        !self.caches.surface.is_empty()
    }

    /// Route the pointer's hover position over a surface glyph into its
    /// `iMouse.xy`. `u`/`v` are that position normalized inside the composited
    /// quad (top-left origin); click state (`zw`) is untouched.
    ///
    /// Which surface is under the pointer, and where inside it, is resolved by
    /// the compositor from the projection that placed the panes — the renderer
    /// receives the answer rather than recomputing it from a pointer position
    /// whose coordinate space it cannot check.
    pub fn surface_mouse_hover(&mut self, id: u32, u: f32, v: f32) {
        self.caches.surface.set_mouse_uv(id, u, v);
    }

    /// Route a button press over a surface glyph into its `iMouse.zw`
    /// (Shadertoy click state). `u`/`v` are the press position normalized
    /// inside the composited quad (top-left origin), mapped like hover's
    /// `surface_mouse_hover`; zw stay positive until `surface_mouse_release`.
    pub fn surface_mouse_press(&mut self, id: u32, u: f32, v: f32) {
        self.caches.surface.set_mouse_press_uv(id, u, v);
    }

    /// End the click on whichever surface is pressed (negates its
    /// `iMouse.zw`). Safe to call on every button release.
    pub fn surface_mouse_release(&mut self) {
        self.caches.surface.set_mouse_release();
    }

    /// The surfaces composited since the last call, or `None` when no
    /// glyph composite ran (a retained frame), so the caller can tell "the
    /// frame lacks it" from "the frame was not recomposited".
    pub fn take_drawn_surfaces(&mut self) -> Option<std::collections::HashSet<u32>> {
        self.caches.surface.take_drawn()
    }

    /// A retained build starts: collect the surfaces its glyph passes
    /// reach, for [`Self::take_referenced_surfaces`].
    pub fn collect_referenced_surfaces(&mut self) {
        self.caches.surface.collect_referenced();
    }

    /// The surfaces whose glyphs a composite reached since
    /// [`Self::collect_referenced_surfaces`], drawn or not (a superset of
    /// the drawn ones): what a retained raster depends on, including models
    /// not yet drawable. Ends the collection.
    pub fn take_referenced_surfaces(&mut self) -> std::collections::HashSet<u32> {
        self.caches.surface.take_referenced()
    }

    /// Free a shader surface's GPU objects.
    pub fn free_surface(&mut self, id: u32) {
        self.caches.surface.free(id);
        self.media_budget
            .unregister(crate::media_budget::MediaType::Surface, id);
        self.surface_recreatable.remove(&id);
    }

    /// Render pending shader-surface passes (call each frame before the main
    /// pass samples the surface textures). Image/video channels resolve here,
    /// where every cache is visible; missing/not-ready sources fall back to
    /// transparent black inside the surface cache.
    pub fn process_shader_surfaces(&mut self) {
        use crate::shader_surface::SurfaceChannelSource;
        let sample = self.frame_sample;
        self.reconcile_media_budget();
        let sources = self.caches.surface.external_channel_sources();
        let mut external = std::collections::HashMap::new();
        for source in sources {
            let view = match source {
                SurfaceChannelSource::Surface(_) => None,
                SurfaceChannelSource::Image(id) => {
                    self.caches.image.get(id).map(|cached| cached.view.clone())
                }
                SurfaceChannelSource::Video(_) => None,
            };
            if let Some(view) = view {
                external.insert(source, view);
            }
        }
        self.caches
            .surface
            .render_pending(&self.device, &self.queue, &external, sample);
    }

    /// Whether any animated shader surface was composited recently — drives
    /// `DemandReason::ShaderSurface` pacing. A frame post shader is always
    /// animated while installed.
    pub fn has_active_shader_surfaces(&self) -> bool {
        self.caches.surface.has_active_surfaces() || self.frame_post.is_some()
    }

    /// The cadence the `DemandReason::ShaderSurface` demand should run at,
    /// given the window's display refresh as the ceiling. The frame post
    /// shader re-shades the whole composited frame (cursor blink, transitions)
    /// so it forces the full rate; otherwise the rate is the max of the active
    /// surfaces' `:fps` caps (uncapped active surfaces also force full rate).
    /// Result is clamped to `[1, display_rate]`.
    pub fn shader_surface_demand_rate(&self, display_rate: u32) -> u32 {
        let ceiling = display_rate.max(1);
        if self.frame_post.is_some() {
            return ceiling;
        }
        match self.caches.surface.active_animation_max_fps() {
            Some(cap) => cap.clamp(1, ceiling),
            None => ceiling,
        }
    }

    // =========== Frame post shader (docs/display-engine/SHADER_SURFACES.md) ===========

    /// Install (or replace) the full-frame post shader from an
    /// already-composed WGSL module (the host validates + composes on the
    /// Lisp thread, uniform accessors included; `uniforms` carries the
    /// name -> slot table and initial values in slot order).
    pub fn set_frame_post(
        &mut self,
        language: crate::shader_surface::SurfaceShaderLanguage,
        composed_source: &str,
        uniforms: &[crate::shader_surface::SurfaceUniformInit],
    ) -> Result<(), String> {
        let post = crate::frame_post::FramePost::new(
            &self.device,
            self.surface_format,
            language,
            composed_source,
            uniforms,
        )?;
        self.frame_post = Some(post);
        tracing::info!("frame post shader installed");
        Ok(())
    }

    /// Whether a frame post shader is installed.
    pub fn has_frame_post(&self) -> bool {
        self.frame_post.is_some()
    }

    /// Run the post pass: shade `src_view` (the rendered frame) into
    /// `dst_view`. `mouse` is the pointer in logical px (y-down, window
    /// coords); converted here to the physical y-up contract.
    pub fn frame_post_to_view(
        &mut self,
        src_view: &wgpu::TextureView,
        dst_view: &wgpu::TextureView,
        width_px: u32,
        height_px: u32,
        mouse: (f32, f32),
    ) {
        let scale = self.scale_factor;
        let mouse_px = (
            (mouse.0 * scale).clamp(0.0, width_px as f32),
            (height_px as f32 - mouse.1 * scale).clamp(0.0, height_px as f32),
        );
        let sample = self.frame_sample;
        if let Some(post) = self.frame_post.as_mut() {
            let commands = post.run(
                &self.device,
                &self.queue,
                src_view,
                dst_view,
                width_px,
                height_px,
                scale,
                mouse_px,
                sample,
            );
            self.submit_commands([commands]);
        }
    }
}
