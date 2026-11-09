//! Media surfaces presented by the compositor: video identities visible in the
//! current scene.
//!
//! These methods own `FrameCompositor::visible_videos`, which is why they live
//! inside this module rather than in `frame_windows`.

#[cfg(feature = "video")]
use std::collections::HashSet;

use crate::render_thread::frame_windows::GuiFrameRenderState;
#[cfg(feature = "video")]
use romacs_display_protocol::frame_glyphs::{FrameGlyph, FrameGlyphBuffer};
#[cfg(feature = "video")]
use romacs_display_protocol::types::VideoId;

impl GuiFrameRenderState {
    #[cfg(feature = "video")]
    pub(in crate::render_thread) fn refresh_visible_videos(&mut self) {
        fn collect(frame: &FrameGlyphBuffer, output: &mut HashSet<VideoId>) {
            output.extend(frame.glyphs.iter().filter_map(|glyph| match glyph {
                FrameGlyph::Video { video_id, .. } => Some(*video_id),
                _ => None,
            }));
        }

        let mut visible = HashSet::new();
        if let Some(frame) = &self.compositor.current_frame {
            collect(frame, &mut visible);
        }
        for entry in self.compositor.child_frames.frames.values() {
            collect(&entry.frame, &mut visible);
        }
        self.compositor.visible_videos = visible;
    }

    #[cfg(feature = "video")]
    pub(in crate::render_thread) fn presents_video(&self, id: VideoId) -> bool {
        self.compositor.visible_videos.contains(&id)
    }

    #[cfg(feature = "video")]
    pub(in crate::render_thread) fn presented_video_ids(
        &self,
    ) -> impl Iterator<Item = VideoId> + '_ {
        self.compositor.visible_videos.iter().copied()
    }
}
