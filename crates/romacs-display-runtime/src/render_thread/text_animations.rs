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
//!
//! Where the renderer allows dynamic effects (a GPU), an animation is drawn
//! as a page scene (page_scenes.rs) over its blanked cells, at the display's
//! rate; elsewhere, and if the GPU refuses the scene, as its cells.
//! Under ROMACS_DIAGNOSTICS=1 each change of how an animation is drawn
//! (scene and where, cells, or out of view) is a line on stderr.
use romacs_display_protocol::{FrameGlyph, FrameGlyphBuffer};
use romacs_display_protocol::text_animation::{TextAnimation, paint};
use romacs_renderer_wgpu::{WgpuGlyphAtlas, WgpuRenderer};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::page_scenes::{self, PageScene};

/// Most motion one frame advances, however late it is.
const MAX_STEP: Duration = Duration::from_millis(80);

/// Frames a second of an animation drawn as a page scene.
const SCENE_FPS: u32 = 60;

struct Running {
    animation: TextAnimation,
    /// Seconds of motion so far.
    moved: f64,
    /// When the last moving frame was due; None after a stop.
    last: Option<Instant>,
    /// The cells for `moved`, computed once per frame reached.
    cells: Option<(f64, Vec<romacs_logo::Cell>)>,
    /// The scene drawing it on the GPU.
    scene: Option<PageScene>,
    /// How the last painted frame drew it.
    drawn: Option<Drawn>,
}

/// How a frame draws an animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drawn {
    /// As its scene's placement, at this frame rectangle.
    Scene(kurbo::Rect),
    /// As its cells.
    Cells,
    /// Not at all: its block is out of its window's view.
    Hidden,
}

impl Running {
    fn drawn_as_scene(&self) -> bool {
        self.scene.as_ref().is_some_and(|s| s.shown.is_some() && !s.failed)
    }

    fn fps(&self) -> u32 {
        if self.drawn_as_scene() { SCENE_FPS } else { self.animation.fps }
    }
}

#[derive(Default)]
pub(crate) struct TextAnimations {
    running: BTreeMap<u64, Running>,
    unfocused: bool,
    occluded: bool,
    /// Scenes of animations that ended, for the renderer to free.
    retired: Vec<PageScene>,
}

impl TextAnimations {
    /// The animations a new frame names, by key: these and no others.
    /// One that continues its motion keeps its motion time; one under the
    /// same key keeps its scene, which starts its motion over by itself.
    pub(crate) fn replace(&mut self, animations: Vec<(u64, TextAnimation)>) {
        let mut previous = std::mem::take(&mut self.running);
        for (id, animation) in animations {
            let (scene, drawn, before) = match previous.remove(&id) {
                Some(mut r) => (r.scene.take(), r.drawn, Some(r).filter(|r| r.animation.continues(&animation))),
                None => (None, None, None),
            };
            let moved = before.as_ref().map_or(0.0, |r| r.moved);
            let last = before.as_ref().and_then(|r| r.last).filter(|_| animation.moving);
            let cells = before.filter(|r| r.animation == animation).and_then(|r| r.cells);
            self.running.insert(id, Running { animation, moved, last, cells, scene, drawn });
        }
        self.retired.extend(previous.into_values().filter_map(|r| r.scene));
    }

    /// Scenes to free.
    pub(crate) fn take_retired(&mut self) -> Vec<PageScene> {
        std::mem::take(&mut self.retired)
    }

    /// Bring the scene of every animation to its motion time in FRAME, where
    /// GPU allows it; the others are drawn as cells.
    pub(crate) fn prepare_scenes(&mut self, frame: &FrameGlyphBuffer, gpu: bool, renderer: &mut WgpuRenderer) {
        for running in self.running.values_mut() {
            if !gpu {
                if let Some(scene) = running.scene.take() { self.retired.push(scene); }
                continue;
            }
            let scene = running.scene.get_or_insert_with(PageScene::new);
            if !scene.failed { scene.prepare(frame, &running.animation, running.moved, renderer); }
        }
    }

    /// Draw the scenes [`Self::prepare_scenes`] left to draw.
    pub(crate) fn draw_scenes(&mut self, renderer: &mut WgpuRenderer, atlas: &mut WgpuGlyphAtlas) {
        for scene in self.running.values_mut().filter_map(|r| r.scene.as_mut()) {
            if !scene.failed { scene.draw(renderer, atlas); }
        }
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
            .map(|r| r.last.map_or(now, |last| last + Duration::from_secs(1) / r.fps()))
            .min()
    }

    /// Advance every animation whose frame is due at NOW; whether one did,
    /// so the window must be redrawn.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let mut advanced = false;
        let halted = self.halted();
        for running in self.running.values_mut() {
            if !moves(&running.animation, halted) { continue; }
            let period = Duration::from_secs(1) / running.fps();
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

    /// Paint every animation over FRAME, the copy about to be drawn: a
    /// scene's surface over blank cells, or its cells. Its glyphs then
    /// differ from the producer's, so it no longer promises to paint as
    /// another frame of its layout identity does (a retained root picture
    /// would show a still animation).
    pub(crate) fn paint(&mut self, frame: &mut FrameGlyphBuffer) {
        if !self.running.is_empty() { frame.layout_identity = 0; }
        for running in self.running.values_mut() {
            let block = page_scenes::block(frame, &running.animation);
            let shown = running.scene.as_ref().filter(|_| running.drawn_as_scene())
                .zip(block)
                .and_then(|(scene, block)| scene.glyph(&block));
            running.report(match (&shown, block) {
                (Some(FrameGlyph::Surface { x, y, width, height, .. }), _) =>
                    Drawn::Scene(kurbo::Rect::new(f64::from(*x), f64::from(*y), f64::from(x + width), f64::from(y + height))),
                (_, Some(_)) => Drawn::Cells,
                (_, None) => Drawn::Hidden,
            });
            if let Some(glyph) = shown {
                let blank = vec![(' ', None); (running.animation.cols * running.animation.rows) as usize];
                paint(frame, &running.animation, &blank);
                frame.glyphs.push(glyph);
                continue;
            }
            let moved = running.moved;
            if running.cells.as_ref().is_none_or(|(at, _)| *at != moved) {
                running.cells = Some((moved, running.animation.cells(moved)));
            }
            if let Some((_, cells)) = &running.cells { paint(frame, &running.animation, cells); }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool { self.running.is_empty() }

    /// How the last painted frame drew animation ID.
    #[cfg(test)]
    pub(crate) fn drawn(&self, id: u64) -> Option<Drawn> { self.running.get(&id).and_then(|r| r.drawn) }
}

impl Running {
    /// Keep how this frame draws it; say so when that changed.
    fn report(&mut self, drawn: Drawn) {
        if self.drawn == Some(drawn) { return; }
        self.drawn = Some(drawn);
        if !diagnostics() { return; }
        let window = self.animation.window;
        match drawn {
            Drawn::Scene(r) => eprintln!("romacs-text-animation: window={window} drawn=scene x={} y={} width={} height={}",
                                         r.x0, r.y0, r.width(), r.height()),
            Drawn::Cells => eprintln!("romacs-text-animation: window={window} drawn=cells"),
            Drawn::Hidden => eprintln!("romacs-text-animation: window={window} drawn=hidden"),
        }
    }
}

/// Whether ROMACS_DIAGNOSTICS=1 asks for diagnostics on stderr.
fn diagnostics() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ROMACS_DIAGNOSTICS").is_some_and(|v| v == "1"))
}

impl super::state::RenderApp {
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
mod tests;
