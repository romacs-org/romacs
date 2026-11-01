//! The text animations a window draws over its evaluator frame
//! (romacs_display_protocol::text_animation): their clock, their motion
//! time and the cells of the frame they reached.  Nothing here runs Lisp
//! or waits for the evaluator; a tick only asks for a redraw, and every
//! draw paints the cells over its copy of the current frame.
//!
//! An animation moves only while Lisp says it may (`moving`) and the
//! window has focus and is not occluded.  Motion stops at once, and starts
//! again with a still first frame: a pause, a hidden workspace or a lost
//! focus costs nothing and is never caught up.
use romacs_display_protocol::FrameGlyphBuffer;
use romacs_display_protocol::text_animation::{paint, TextAnimation};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Most motion one frame advances, however late it is.
const MAX_STEP: Duration = Duration::from_millis(80);

struct Running {
    animation: TextAnimation,
    /// Seconds of motion so far.
    moved: f64,
    /// When the last moving frame was due; None after a stop.
    last: Option<Instant>,
    /// The cells for `moved`, computed once per frame reached.
    cells: Option<(f64, Vec<u8>)>,
}

#[derive(Default)]
pub(crate) struct TextAnimations {
    running: BTreeMap<u64, Running>,
    unfocused: bool,
    occluded: bool,
}

impl TextAnimations {
    /// Install, change (None: remove) animation ID.  A change that
    /// continues its motion keeps the motion time.
    pub(crate) fn set(&mut self, id: u64, animation: Option<TextAnimation>) {
        let Some(animation) = animation else { self.running.remove(&id); return };
        let previous = self.running.remove(&id).filter(|r| r.animation.continues(&animation));
        let moved = previous.as_ref().map_or(0.0, |r| r.moved);
        let last = previous.and_then(|r| r.last).filter(|_| animation.moving);
        self.running.insert(id, Running { animation, moved, last, cells: None });
    }

    pub(crate) fn focus(&mut self, focused: bool) { self.halt_if(!focused, |s| &mut s.unfocused); }

    pub(crate) fn occlude(&mut self, occluded: bool) { self.halt_if(occluded, |s| &mut s.occluded); }

    fn halt_if(&mut self, halted: bool, flag: impl Fn(&mut Self) -> &mut bool) {
        *flag(self) = halted;
        if halted { for running in self.running.values_mut() { running.last = None; } }
    }

    fn halted(&self) -> bool { self.unfocused || self.occluded }

    /// When the next moving frame is due, if any moves.
    pub(crate) fn deadline(&self, now: Instant) -> Option<Instant> {
        let halted = self.halted();
        self.running.values().filter(|r| moves(&r.animation, halted))
            .map(|r| r.last.map_or(now, |last| last + Duration::from_secs(1) / r.animation.fps))
            .min()
    }

    /// Advance every animation whose frame is due at NOW; whether one did,
    /// so the window must be redrawn.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let mut advanced = false;
        let halted = self.halted();
        for running in self.running.values_mut() {
            if !moves(&running.animation, halted) { continue; }
            let period = Duration::from_secs(1) / running.animation.fps;
            match running.last {
                Some(last) if now < last + period => {}
                Some(last) => {
                    running.moved += now.saturating_duration_since(last).min(MAX_STEP).as_secs_f64();
                    running.last = Some(now);
                    advanced = true;
                }
                // The first frame after a stop is the one it stopped at.
                None => { running.last = Some(now); advanced = true; }
            }
        }
        advanced
    }

    /// Paint every animation over FRAME, the copy about to be drawn.
    pub(crate) fn paint(&mut self, frame: &mut FrameGlyphBuffer) {
        for running in self.running.values_mut() {
            let moved = running.moved;
            if running.cells.as_ref().is_none_or(|(at, _)| *at != moved) {
                running.cells = Some((moved, running.animation.cells(moved)));
            }
            if let Some((_, cells)) = &running.cells { paint(frame, &running.animation, cells); }
        }
    }

    pub(crate) fn is_empty(&self) -> bool { self.running.is_empty() }
}

impl super::state::RenderApp {
    /// `RenderCommand::TextAnimation`: on the primary window, drawn from
    /// its next frame on.
    pub(super) fn set_text_animation(&mut self, id: u64, animation: Option<TextAnimation>) {
        if let Some(window) = self.frame_windows.primary_window_mut() {
            window.render.compositor.text_animations.set(id, animation);
            redraw(window);
        }
    }

    /// Advance the primary window's due animations and redraw it if one
    /// moved; when the next frame is due.
    pub(super) fn service_text_animations(&mut self, now: Instant) -> Option<Instant> {
        let window = self.frame_windows.primary_window_mut()?;
        if window.render.compositor.text_animations.tick(now) { redraw(window); }
        window.render.compositor.text_animations.deadline(now)
    }
}

/// Redraw WINDOW whole: its scene changed with no new frame.
fn redraw(window: &mut super::frame_windows::GuiFrameWindowState) {
    let compositor = &mut window.render.compositor;
    compositor.current_scene_generation = super::frame_state::next_scene_generation();
    compositor.current_row_damage = None;
    window.render.mark_dirty();
    window.request_redraw();
}

fn moves(animation: &TextAnimation, halted: bool) -> bool {
    animation.moving && !halted && animation.fps > 0
}

#[cfg(test)]
#[path = "text_animations/tests/text_animations_test.rs"]
mod tests;
