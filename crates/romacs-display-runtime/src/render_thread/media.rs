use super::RenderApp;
#[cfg(feature = "video")]
use super::frame_sched::{NativeWindowId, PacingAction};
#[cfg(feature = "webview")]
use crate::thread_comm::InputEvent;

#[cfg(feature = "video")]
fn video_service_request(
    now: std::time::Instant,
    presentations: impl IntoIterator<
        Item = (romacs_display_protocol::types::VideoId, std::time::Duration),
    >,
) -> romacs_video::VideoServiceRequest {
    let mut request = romacs_video::VideoServiceRequest::new(now);
    for (id, lead) in presentations {
        request.set_presentation_target(id, now.checked_add(lead).unwrap_or(now));
    }
    request
}

#[cfg(all(feature = "webview", target_os = "linux"))]
use romacs_renderer_wgpu::WgpuRenderer;

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

    #[cfg(feature = "webview")]
    pub(super) fn pump_glib(&mut self) {
        let Some(system) = self.webview_system.as_mut() else {
            return;
        };
        system.service();
        for event in system.drain_events() {
            self.comms.send_input(InputEvent::WebView(event));
        }
    }

    #[cfg(not(feature = "webview"))]
    pub(super) fn pump_glib(&mut self) {}

    /// Process webkit frames and import to wgpu textures
    #[cfg(all(feature = "webview", target_os = "linux"))]
    pub(super) fn process_webkit_frames(&mut self) {
        use romacs_renderer_wgpu::DmaBufBuffer;
        use std::os::fd::OwnedFd;

        let (Some(renderer), Some(system)) = (self.renderer.as_mut(), self.webview_system.as_mut())
        else {
            return;
        };
        let view_ids = system.view_ids();

        let try_upload_dmabuf = |renderer: &mut WgpuRenderer,
                                 view_id: romacs_webview::WebViewId,
                                 dmabuf: romacs_webview::DmaBufFrame|
         -> bool {
            // Never turn producer synchronization into a render-thread stall.
            // A future Vulkan semaphore-import path can consume the fence on
            // the GPU; until then an unready experimental DMA-BUF frame is
            // skipped and WPE remains free to produce its replacement.
            match dmabuf.wait_until_ready(std::time::Duration::ZERO) {
                Ok(romacs_webview::DmaBufReadiness::Ready) => {}
                Ok(romacs_webview::DmaBufReadiness::TimedOut) => {
                    tracing::warn!(
                        "WPE rendering fence is not ready for webview {}; skipping frame",
                        view_id.get()
                    );
                    return false;
                }
                Err(error) => {
                    tracing::warn!(
                        "WPE rendering fence failed for webview {}: {}; skipping frame",
                        view_id.get(),
                        error
                    );
                    return false;
                }
            }
            let width = dmabuf.width();
            let height = dmabuf.height();
            let fourcc = dmabuf.fourcc();
            let modifier = dmabuf.modifier();
            let planes = dmabuf.planes();
            let num_planes = planes.len().min(4) as u32;
            let mut fds: [Option<OwnedFd>; 4] = [None, None, None, None];
            let mut strides = [0u32; 4];
            let mut offsets = [0u32; 4];

            for (index, plane) in planes.iter().take(4).enumerate() {
                strides[index] = plane.stride();
                offsets[index] = plane.offset();
                let Ok(file) = plane.file().try_clone() else {
                    tracing::warn!(
                        "failed to duplicate DMA-BUF plane for webview {}; skipping frame",
                        view_id.get()
                    );
                    return false;
                };
                fds[index] = Some(file.into());
            }

            let buffer = DmaBufBuffer::new(
                fds, strides, offsets, num_planes, width, height, fourcc, modifier,
            );

            // Ownership of the complete frame crosses into the renderer. Its
            // opaque WPE lease cannot be released until the exact GPU copy
            // submission retires on the renderer's retirement worker.
            match renderer.update_webview_dmabuf(view_id, buffer, dmabuf) {
                Ok(()) => true,
                Err(frame) => {
                    // Return the still-owned native frame, not a future repaint
                    // request: static pages must recover even if WPE is idle.
                    frame.request_pixel_fallback();
                    false
                }
            }
        };

        for view_id in view_ids {
            let Some(frame) = system.take_frame(view_id) else {
                continue;
            };
            match frame {
                romacs_webview::WebViewFrame::DmaBuf(frame) => {
                    if try_upload_dmabuf(renderer, view_id, frame) {
                        tracing::debug!("imported DMA-BUF for webview {}", view_id.get());
                    }
                }
                romacs_webview::WebViewFrame::Pixels(frame) => {
                    if renderer.update_webview_pixels(
                        view_id,
                        frame.width(),
                        frame.height(),
                        frame.pixels(),
                    ) {
                        tracing::debug!("uploaded pixels for webview {}", view_id.get());
                    }
                }
            }
        }
    }

    #[cfg(not(all(feature = "webview", target_os = "linux")))]
    pub(super) fn process_webkit_frames(&mut self) {}

    /// Service native decoder events once. A due frame is mapped through the
    /// presentation-owned video index to exactly the native windows whose
    /// accepted root/child snapshots reference it. The scheduler consumes
    /// those one-shot targets; the decoder's next PTS becomes a coordinator-
    /// owned service wake rather than parallel lifecycle state.
    #[cfg(feature = "video")]
    pub(super) fn process_video_frames(
        &mut self,
        now: romacs_display_protocol::frame_time::EventTime,
    ) {
        tracing::trace!("process_video_frames called");
        let presentations = self
            .frame_windows
            .windows
            .iter()
            .filter_map(|(key, window)| {
                let native_id = match key {
                    super::frame_windows::FrameKey::Pending => 0,
                    super::frame_windows::FrameKey::Adopted(id) => *id,
                };
                self.frame_coordinator
                    .is_eligible(super::frame_sched::NativeWindowId(native_id))
                    .then(|| {
                        let interval = std::time::Duration::from_secs_f64(
                            1.0 / f64::from(Self::window_max_rate(window).get()),
                        );
                        (window, interval)
                    })
            })
            .flat_map(|(window, interval)| {
                window
                    .render
                    .presented_video_ids()
                    .map(move |id| (id, interval))
            });
        // `romacs-video` keeps its own `Instant` clock domain; bridge explicitly
        // rather than leaking EventTime into that crate's API.
        let request = video_service_request(now.into_instant(), presentations);
        let Some(renderer) = self.renderer.as_mut() else {
            self.frame_coordinator
                .reconcile_video_service_deadline(None);
            return;
        };
        let service = renderer.process_pending_videos(request).clone();
        self.frame_coordinator.reconcile_video_service_deadline(
            service
                .next_deadline
                .map(romacs_display_protocol::frame_time::EventTime::from_observed_instant),
        );
        for ready in service.ready_frames {
            for (key, window_state) in &self.frame_windows.windows {
                let native_id = match key {
                    super::frame_windows::FrameKey::Pending => 0,
                    super::frame_windows::FrameKey::Adopted(id) => *id,
                };
                let native_window = NativeWindowId(native_id);
                if !self.frame_coordinator.is_eligible(native_window) {
                    continue;
                }
                if window_state.render.presents_video(ready.id) {
                    let action = self
                        .frame_coordinator
                        .submit_ready_video_frame(native_window, now);
                    if action == PacingAction::RequestRedraw {
                        window_state.request_redraw();
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "video"))]
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

    /// Check if any WebKit view needs redraw
    #[cfg(feature = "webview")]
    pub(super) fn has_webkit_needing_redraw(&self) -> bool {
        self.webview_system
            .as_ref()
            .is_some_and(romacs_webview::WebViewSystem::has_pending_frame)
    }

    #[cfg(not(feature = "webview"))]
    pub(super) fn has_webkit_needing_redraw(&self) -> bool {
        false
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
        if let romacs_display_protocol::ImageStateEvent::Evicted(image) = event {
            self.comms.lost_images.lock().unwrap_or_else(|e| e.into_inner()).evicted.push(image.get());
        }
        self.comms
            .send_input(crate::thread_comm::InputEvent::ImageStateChanged { event });
    }

    /// Publish the complete set of image identities which can still be drawn.
    /// Accepted presentations, deferred child presentations, and renderer-owned
    /// toolbar chrome all participate in the same lifetime fence.
    pub(super) fn synchronize_image_residency(&mut self) {
        let mut retained = self.frame_windows.retained_images();
        for pending in self.pending_child_frames.values() {
            retained.extend(pending.state.referenced_images().iter());
        }
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

#[cfg(test)]
#[path = "tests/media_test.rs"]
mod followup_tests;
