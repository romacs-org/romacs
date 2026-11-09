//! Asset and embedded-content render commands.

use super::RenderApp;
use crate::thread_comm::AssetCommand;
use romacs_display_protocol::ImageId;

fn clear_image_terminals(shared: &super::SharedImageRenderState, image: ImageId) {
    shared.clear_image_terminals(image);
}

#[cfg(test)]
/// The failure GNU's size limit answers a load with, if it answers one.
///
/// GNU checks `max-image-size` inside each loader, against the header it has
/// just read, and gives up before creating any image storage
/// (`check_image_size`, `src/image.c:1811`). Both load paths here read the same
/// header first, so this is that moment: a source over the bound is answered
/// with a failed decode — GNU's own diagnostic — and the renderer is never
/// asked for a pixel.
///
/// The verdict is returned as a plain cache event so it travels the path a
/// decoder failure already travels ([`RenderApp::handle_image_event`]) and
/// carries no separate meaning of its own.
fn oversized_load_event(
    load: romacs_display_protocol::ImageLoadToken,
    source: romacs_renderer_wgpu::ImageProbeSource<'_>,
    limit: romacs_display_protocol::ImageSizeLimit,
) -> Option<romacs_renderer_wgpu::ImageCacheEvent> {
    let Err(oversized) = romacs_renderer_wgpu::admit(source, limit) else {
        return None;
    };
    tracing::debug!(
        image = %load.image(),
        extent = ?oversized.extent().dimensions(),
        maximum = ?limit.maximum().dimensions(),
        "refusing an image larger than max-image-size"
    );
    Some(romacs_renderer_wgpu::ImageCacheEvent::Failed {
        load,
        error: romacs_display_protocol::image_diagnostic::ImageDiagnostic::InvalidSize,
    })
}

impl RenderApp {
    pub(super) fn handle_asset(&mut self, cmd: AssetCommand) {
        match cmd {
            AssetCommand::ImageLoadDecoded { load, decoded } => {
                clear_image_terminals(&self.image_metadata, load.image());
                if let Some(renderer) = self.renderer.as_mut() {
                    let event = renderer.upload_semantic_image(load, &decoded);
                    // Publish residency accounting before Ready wakes the evaluator.
                    self.publish_image_cache_usage();
                    self.handle_image_event(event);
                } else {
                    self.publish_image_cache_usage();
                }
            }
            AssetCommand::ImageRetire { image } => {
                clear_image_terminals(&self.image_metadata, image);
                tracing::debug!("Retiring image {} after its last presentation", image);
                if let Some(ref mut renderer) = self.renderer {
                    renderer.retire_image(image);
                }
                self.publish_image_cache_usage();
            }
            AssetCommand::DebugSimulateDeviceLoss => {
                tracing::warn!(
                    "simulating wgpu device loss (debug): rebuilding GPU state on the next pass"
                );
                self.device_lost.mark_lost_now();
                // Guarantee another event-loop pass observes the latch even
                // on an otherwise idle session: dirty content plus explicit
                // redraw requests keep the loop awake until recovery runs.
                self.frame_windows.mark_top_level_dirty();
                self.frame_windows
                    .for_each_top_level_window(|window_state| window_state.request_redraw());
            }
        }
    }
}

#[cfg(test)]
#[path = "asset_commands/tests/image_terminal_test.rs"]
mod image_terminal_tests;

#[cfg(test)]
#[path = "asset_commands/tests/image_size_limit_test.rs"]
mod image_size_limit_tests;
