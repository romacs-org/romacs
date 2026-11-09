//! UI overlay, animation, and effect render commands.

use super::RenderApp;
use crate::thread_comm::ConfigCommand;

impl RenderApp {
    pub(super) fn handle_config(&mut self, cmd: ConfigCommand) {
        match cmd {
            ConfigCommand::SetVisualConfig(config) => {
                self.requested_visual_config = config;
                self.apply_requested_visual_config();
                self.frame_windows.mark_top_level_dirty();
            }
        }
    }
}
