//! Frame ingestion and cursor target extraction.

use super::RenderApp;
use super::frame_windows::{
    ActivePresentationTransition, GuiFrameRenderState,
};
use crate::render_thread::cursor::{CursorConfigSnapshot, CursorTarget};

struct CursorSyncOutcome {
    target: CursorTarget,
}

impl RenderApp {
    pub(super) fn ingest_paint(&mut self, scene: crate::thread_comm::PaintScene) {
        let crate::thread_comm::PaintScene { root: mut frame, children, text_animations } = scene;
        if self.frame_windows.primary_window().and_then(|window| window.lifecycle.native())
            .is_some_and(|native| !native.composites_alpha())
        {
            frame.background_alpha = 1.0;
        }
        let current = self.frame_windows.primary_window().and_then(|window| {
            window.render.compositor.current_frame.as_ref()
        });
        if crate::paint_host::validate_frame(&frame).is_err()
            || crate::paint_host::validate_children(&frame, &children).is_err()
            || current.is_some_and(|current| {
                current.presentation_id.get() >= frame.presentation_id.get()
            })
        {
            self.comms.send_input(crate::thread_comm::InputEvent::PresentationDiscarded {
                presentation: frame.presentation_id.get(),
                emacs_frame_id: frame.frame_placement.frame().get(),
            });
            return;
        }
        // Seam P already owns its font projection. No matrix materialization,
        // Lisp, or speculative row-reuse metadata is needed for this full paint.
        super::frame_stats::note_scene_commit(
            romacs_display_protocol::frame_time::observe_platform_now(),
        );
        // The frame names the animations drawn over it: the last frame's
        // are gone with it.
        if let Some(window) = self.frame_windows.primary_window_mut() {
            window.render.compositor.text_animations.replace(text_animations);
        }
        self.ingest_primary_paint(
            frame,
            Default::default(),
            Default::default(),
            Default::default(),
        );
        // The scene's children replace the last scene's: one gone is removed.
        let shown: Vec<u64> = children.iter().map(|c| c.frame_placement.frame().get()).collect();
        for gone in std::mem::take(&mut self.scene_children) {
            if !shown.contains(&gone) {
                self.remove_child_frame(gone);
            }
        }
        for child in children {
            // A child removed by an earlier scene is marked hidden; this
            // scene shows it again.
            self.frame_windows.show_child_frame_in_top_level_windows(child.frame_placement.frame().get());
            self.update_primary_child(child);
        }
        self.scene_children = shown;
        self.scene = super::state::SceneDraw::Arrived;
    }

    fn ingest_primary_paint(
        &mut self,
        frame: romacs_display_protocol::frame_glyphs::FrameGlyphBuffer,
        row_damage: romacs_renderer_wgpu::FrameRowDamage,
        scroll_anchors: crate::render_thread::frame_compositor::ScrollAnchorsByWindow,
        reflow_imprints: crate::render_thread::frame_compositor::ReflowImprintsByWindow,
    ) {
        let frame_id = frame.frame_placement.frame();
        let cursor_config = self.cursor_defaults.config_snapshot();
        if let Some(primary_frame) = self
            .frame_windows
            .primary_window_mut()
            .map(|ws| &mut ws.render)
        {
            let presentation = Self::ingest_top_level_render_frame(
                primary_frame,
                frame,
                row_damage,
                cursor_config,
                scroll_anchors,
                reflow_imprints,
            );
            if let Some(transition) = presentation {
                self.comms.send_input(
                    crate::thread_comm::InputEvent::PresentationActivated {
                        presentation: transition.activated.get(),
                        emacs_frame_id: frame_id.get(),
                    },
                );
                if let Some(presentation) = transition.replaced.and_then(|replaced| {
                    primary_frame.route_presentation_retirement(replaced.get())
                }) {
                    self.comms.send_input(
                        crate::thread_comm::InputEvent::PresentationRetired {
                            presentation,
                        },
                    );
                }
            }
        } else {
            self.comms.send_input(
                crate::thread_comm::InputEvent::PresentationDiscarded {
                    presentation: frame.presentation_id.get(),
                    emacs_frame_id: frame_id.get(),
                },
            );
        }
    }

    /// Install or update FRAME, a child of the primary frame, in the
    /// primary window: its open, resize and drift animations, and the
    /// presentation events its activation or discard owes the producer.
    fn update_primary_child(&mut self, frame: romacs_display_protocol::frame_glyphs::FrameGlyphBuffer) {
        let frame_id = frame.frame_placement.frame();
        if let Some(ws) = self.frame_windows.primary_window_mut() {
            let old_presentation = ws
                .render
                .compositor
                .child_frames
                .frames
                .get(&frame_id.get())
                .map(|entry| entry.frame.presentation_id);
            let new_presentation = frame.presentation_id;
            if ws.render.update_child_frame(frame) {
                if let Some(transition) = ActivePresentationTransition::between(
                    old_presentation,
                    new_presentation,
                ) {
                    self.comms.send_input(
                        crate::thread_comm::InputEvent::PresentationActivated {
                            presentation: transition.activated.get(),
                            emacs_frame_id: frame_id.get(),
                        },
                    );
                    if let Some(presentation) =
                        transition.replaced.and_then(|replaced| {
                            ws.render.route_presentation_retirement(replaced.get())
                        })
                    {
                        self.comms.send_input(
                            crate::thread_comm::InputEvent::PresentationRetired {
                                presentation,
                            },
                        );
                    }
                }
            } else if old_presentation != Some(new_presentation) {
                self.comms.send_input(
                    crate::thread_comm::InputEvent::PresentationDiscarded {
                        presentation: new_presentation.get(),
                        emacs_frame_id: frame_id.get(),
                    },
                );
            }
        } else {
            self.comms.send_input(
                crate::thread_comm::InputEvent::PresentationDiscarded {
                    presentation: frame.presentation_id.get(),
                    emacs_frame_id: frame_id.get(),
                },
            );
        }
    }

    fn ingest_top_level_render_frame(
        render: &mut GuiFrameRenderState,
        frame: romacs_display_protocol::frame_glyphs::FrameGlyphBuffer,
        row_damage: romacs_renderer_wgpu::FrameRowDamage,
        cursor_config: CursorConfigSnapshot,
        scroll_anchors: crate::render_thread::frame_compositor::ScrollAnchorsByWindow,
        reflow_imprints: crate::render_thread::frame_compositor::ReflowImprintsByWindow,
    ) -> Option<ActivePresentationTransition> {
        use romacs_display_protocol::frame_chrome::FrameChromeKind;
        if frame.frame_chrome.band(FrameChromeKind::MenuBar).is_none() {
            render.chrome.interaction.clear_menu_bar();
        }
        if frame.frame_chrome.band(FrameChromeKind::ToolBar).is_none() {
            render.chrome.interaction.clear_toolbar();
        }
        if frame
            .frame_chrome
            .band(FrameChromeKind::CompactBar)
            .is_none()
        {
            render.chrome.interaction.clear_compact_bar();
        }
        if frame.frame_chrome.band(FrameChromeKind::TabBar).is_none() {
            render.chrome.interaction.clear_tab_bar();
        }
        let presentation = render.set_current_frame(
            Some(frame),
            Some(row_damage),
            scroll_anchors,
            reflow_imprints,
        );
        Self::sync_render_cursor(render, cursor_config);
        render.sync_visual_cursors_from_current_frame(
            |cursor| cursor.apply_config(cursor_config),
            romacs_display_protocol::frame_time::observe_platform_now(),
        );
        render.mark_dirty();
        presentation
    }

    fn sync_render_cursor(
        render: &mut GuiFrameRenderState,
        cursor_config: CursorConfigSnapshot,
    ) -> Option<CursorSyncOutcome> {
        let mut active_cursor = render.compositor.current_frame.as_ref().and_then(|frame| {
            crate::render_thread::frame_windows::GuiFrameWindowManager::cursor_target_for_frame(
                render.emacs_frame_id,
                frame,
            )
        });

        if active_cursor.is_none() {
            for entry in render.compositor.child_frames.frames.values() {
                if let Some(cursor) = entry.frame.active_cursor() {
                    // Resolve through the shared cursor_draw_rect (where the
                    // static cursor draws), not the grid-approximate cursor
                    // geometry; see cursor_target_for_frame.
                    let (x, y, width, height) = entry.frame.cursor_draw_rect(
                        cursor.slot_id,
                        cursor.style,
                        cursor.ascent,
                        (cursor.x, cursor.y, cursor.width, cursor.height),
                    );
                    active_cursor = Some(CursorTarget::new(
                        cursor.window_id.get(),
                        (x, y, width, height),
                        cursor.style,
                        entry.frame_id,
                    ));
                    break;
                }
            }
        }

        render.cursor.apply_config(cursor_config);
        if let Some(new_target) = active_cursor {
            let (_, target_moved) = render.cursor.set_target(
                new_target.clone(),
                romacs_display_protocol::frame_time::observe_platform_now(),
            );
            if target_moved {
                render.mark_dirty();
            }
            Some(CursorSyncOutcome { target: new_target })
        } else if Self::cursor_blinked_off(render) {
            // The cursor's paint is off; its target stays the IME's anchor.
            None
        } else {
            render.cursor.clear_target();
            render.clear_ime_preedit();
            None
        }
    }

    /// RENDER's frame has its active cursor blinked off, not gone: IME
    /// state stands (`FrameGlyphBuffer::cursor_blinked_off`).
    pub(super) fn cursor_blinked_off(render: &GuiFrameRenderState) -> bool {
        render.compositor.current_frame.as_ref().is_some_and(|frame| frame.cursor_blinked_off)
    }

    /// Advance child-frame animation and resync the primary frame's cursor
    /// and IME anchor, once per loop wake. Frames arrive through
    /// [`Self::ingest_paint`].
    pub(super) fn refresh_frame_state(&mut self) {
        self.frame_windows.tick_top_level_child_frames();
        let cursor_config = self.cursor_defaults.config_snapshot();
        if let Some(primary_frame) = self
            .frame_windows
            .primary_window_mut()
            .map(|ws| &mut ws.render)
        {
            let cursor_sync = Self::sync_render_cursor(primary_frame, cursor_config);
            primary_frame.sync_visual_cursors_from_current_frame(
                |cursor| cursor.apply_config(cursor_config),
                romacs_display_protocol::frame_time::observe_platform_now(),
            );
            let blinked_off = Self::cursor_blinked_off(primary_frame);
            if let Some(cursor_sync) = cursor_sync {
                self.update_ime_cursor_area_if_needed(&cursor_sync.target);
            } else if !blinked_off {
                if let Some(window_state) = self.frame_windows.primary_window_mut() {
                    window_state.reset_ime_cursor_area()
                };
                if let Some(ws) = self.frame_windows.primary_window_mut() {
                    ws.render.clear_ime_preedit()
                };
            }
        }
    }
}
