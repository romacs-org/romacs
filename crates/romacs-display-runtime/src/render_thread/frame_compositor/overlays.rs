//! Transient overlays owned by the compositor: visual bell,
//! typing-speed and idle-dim state. Native tooltips live in `tooltips`.
//!
//! Paths here are absolute on purpose, matching the sibling submodules.

use romacs_display_protocol::frame_time::EventTime;

use crate::render_thread::frame_windows::GuiFrameRenderState;
use romacs_display_protocol::effect_config::IdleDimConfig;

impl GuiFrameRenderState {
    pub(in crate::render_thread) fn record_typing_keypress(&mut self, now: EventTime) {
        self.overlays.typing_speed.key_press_times.push(now);
        self.compositor.dirty = true;
    }

    pub(in crate::render_thread) fn record_idle_activity(&mut self, now: EventTime) {
        self.overlays.idle_dim.last_activity_time = now;
        self.compositor.dirty = true;
    }

    pub(in crate::render_thread) fn tick_idle_dim(
        &mut self,
        config: &IdleDimConfig,
        now: EventTime,
    ) -> bool {
        let idle_time = now.saturating_since(self.overlays.idle_dim.last_activity_time);
        let target_alpha = if idle_time >= config.delay {
            config.opacity
        } else {
            0.0
        };
        let diff = target_alpha - self.overlays.idle_dim.current_alpha;
        if diff.abs() > 0.001 {
            let fade_speed = if config.fade_duration.as_secs_f32() > 0.0 {
                1.0 / config.fade_duration.as_secs_f32() * 0.016
            } else {
                1.0
            };
            if diff > 0.0 {
                self.overlays.idle_dim.current_alpha = (self.overlays.idle_dim.current_alpha
                    + fade_speed * config.opacity)
                    .min(target_alpha);
            } else {
                self.overlays.idle_dim.current_alpha =
                    (self.overlays.idle_dim.current_alpha - fade_speed * config.opacity).max(0.0);
            }
            self.overlays.idle_dim.active = true;
            self.compositor.dirty = true;
            true
        } else if self.overlays.idle_dim.current_alpha > 0.001 {
            self.overlays.idle_dim.active = true;
            false
        } else {
            self.overlays.idle_dim.active = false;
            false
        }
    }

    pub(in crate::render_thread) fn clear_idle_dim(&mut self) {
        self.overlays.idle_dim.active = false;
        self.overlays.idle_dim.current_alpha = 0.0;
    }
}
