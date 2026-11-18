use super::RenderApp;


/// Publish one renderer image event, and say whether the evaluator needs to
/// hear about it.
///
/// A band is published to the shared state and goes no further: nothing the
/// evaluator holds — layout, readiness, cache size — changed when a decode
/// advanced, so waking it per band would buy a redisplay per band and nothing
/// else. The terminal events are the ones it reconciles against. What a band
/// does owe the screen is a *repaint* — the rows it carried are in the texture
/// now, and the frame already on screen was drawn without them — which is the
/// render thread's own business and does not go through the evaluator either.
fn publish_image_cache_event(
    shared: &super::SharedImageRenderState,
    event: romacs_renderer_wgpu::ImageCacheEvent,
) -> Option<crate::thread_comm::ImageStateEvent> {
    let (event, terminal) = match event {
        romacs_renderer_wgpu::ImageCacheEvent::Band { load, rows } => {
            shared.publish_band(load, rows);
            return None;
        }
        romacs_renderer_wgpu::ImageCacheEvent::Ready { load, metadata } => {
            let metadata = crate::host_types::ResolvedImageMetadata {
                layout: metadata.layout,
                reported: metadata.reported,
                background: metadata.background,
                background_transparent: metadata.background_transparent,
                mask: metadata.mask,
                embedded: metadata.embedded,
            };
            (
                crate::thread_comm::ImageStateEvent::DecodeCompleted(load),
                Some(super::ImageDecodeTerminal::Ready(metadata)),
            )
        }
        romacs_renderer_wgpu::ImageCacheEvent::Failed { load, error } => (
            crate::thread_comm::ImageStateEvent::DecodeCompleted(load),
            Some(super::ImageDecodeTerminal::Failed(error)),
        ),
        romacs_renderer_wgpu::ImageCacheEvent::Evicted { image } => {
            (crate::thread_comm::ImageStateEvent::Evicted(image), None)
        }
    };

    if let Some(terminal) = terminal {
        let crate::thread_comm::ImageStateEvent::DecodeCompleted(load) = event else {
            unreachable!("only decode completion publishes terminal metadata")
        };
        shared.publish_terminal(load, terminal);
    } else {
        shared.clear_image_terminals(event.image());
    }
    Some(event)
}

impl RenderApp {
    pub(super) fn publish_image_cache_usage(&self) {
        let usage = self
            .renderer
            .as_ref()
            .map(romacs_renderer_wgpu::WgpuRenderer::image_cache_usage)
            .unwrap_or_default();
        self.image_metadata.publish_cache_usage(usage);
    }

    /// Service native decoder events once. A due frame is mapped through the
    /// presentation-owned video index to exactly the native windows whose
    /// accepted root/child snapshots reference it. The scheduler consumes
    /// those one-shot targets; the decoder's next PTS becomes a coordinator-
    /// owned service wake rather than parallel lifecycle state.

    pub(super) fn process_video_frames(
        &mut self,
        _now: romacs_display_protocol::frame_time::EventTime,
    ) {
    }

    /// Render pending shader-surface passes (call each frame before the main
    /// pass samples the surface textures).
    pub(super) fn process_shader_surfaces(&mut self) {
        if let Some(ref mut renderer) = self.renderer {
            renderer.process_shader_surfaces();
        }
    }

    /// Check if any animated shader surface was composited recently (needs
    /// continuous rendering while visible).
    pub(super) fn has_active_shader_surfaces(&self) -> bool {
        self.renderer
            .as_ref()
            .is_some_and(|r| r.has_active_shader_surfaces())
            // A dirty model redraws once (no continuous cadence).
            || self.models.has_dirty()
    }

    /// The cadence the shader-surface demand should run at (max of active
    /// `:fps` caps, else the display rate); see
    /// `WgpuRenderer::shader_surface_demand_rate`.
    pub(super) fn shader_surface_demand_rate(&self, display_rate: u32) -> u32 {
        self.renderer
            .as_ref()
            .map_or(display_rate, |r| r.shader_surface_demand_rate(display_rate))
    }

    /// Process pending image uploads (decode → GPU texture)
    pub(super) fn process_pending_images(&mut self) {
        let events = self
            .renderer
            .as_mut()
            .map(romacs_renderer_wgpu::WgpuRenderer::process_pending_images)
            .unwrap_or_default();
        // Publish residency before wakeups: an evaluator reacting to the
        // completion event must already observe the corresponding exact cache
        // size, never the previous presentation's snapshot.
        self.publish_image_cache_usage();
        for event in events {
            self.handle_image_event(event);
        }
    }

    pub(super) fn handle_image_event(&mut self, event: romacs_renderer_wgpu::ImageCacheEvent) {
        if matches!(event, romacs_renderer_wgpu::ImageCacheEvent::Band { .. }) {
            // The band's rows are in the image's texture now and nothing else
            // about the frame moved: step 1 already placed the glyph and the
            // quad, so what the screen is owed is a repaint of the frame it is
            // showing, not a redisplay. Marking the content dirty asks for
            // exactly that, and `handle_about_to_wait` drains these events
            // before it reconciles frame demands, so the repaint is requested in
            // the same pass the band is drained in. Without it a band is a
            // texture write nobody draws: the loop idles between decodes, and
            // the pixels would wait for the next unrelated frame.
            self.frame_windows.mark_top_level_dirty();
        }
        let Some(event) = publish_image_cache_event(&self.image_metadata, event) else {
            return;
        };
        if self
            .toolbar
            .textures()
            .values()
            .any(|image| *image == event.image())
        {
            self.frame_windows.mark_top_level_dirty();
        }
        match event {
            crate::thread_comm::ImageStateEvent::Evicted(image) => self.comms.tell(crate::thread_comm::Fact::ImageEvicted(image.get())),
            event => self.comms.send_input(crate::thread_comm::InputEvent::ImageStateChanged { event }),
        }
    }

    /// Publish the complete set of image identities which can still be drawn.
    /// Accepted presentations and renderer-owned toolbar chrome participate in
    /// the same lifetime fence.
    pub(super) fn synchronize_image_residency(&mut self) {
        let mut retained = self.frame_windows.retained_images();
        retained.extend(self.toolbar.textures().values().copied());
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.synchronize_retained_images(retained);
        }
        self.publish_image_cache_usage();
    }

    pub(super) fn has_pending_images(&self) -> bool {
        self.renderer
            .as_ref()
            .is_some_and(|renderer| renderer.has_pending_images())
    }
}

#[cfg(test)]
#[path = "media/tests/image_cache_event_test.rs"]
mod image_cache_event_tests;
