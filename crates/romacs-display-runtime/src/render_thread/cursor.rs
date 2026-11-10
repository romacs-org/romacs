//! Cursor animation and size transition state. Blinking is Lisp's:
//! GNU's blink-cursor-mode turns a window's cursor off in the frame.

use romacs_display_protocol::frame_glyphs::CursorStyle;
use romacs_display_protocol::types::{
    AnimatedCursor, CursorAnimStyle, DisplayFrameId, DisplayWindowId, ease_in_out_cubic,
    ease_linear, ease_out_cubic, ease_out_expo, ease_out_quad,
};
use romacs_display_protocol::VisualConfig;

/// Target position/style for cursor animation
#[derive(Debug, Clone)]
pub(super) struct CursorTarget {
    pub(super) window_id: i64,
    pub(super) x: f32,
    pub(super) y: f32,
    /// The width the cursor shows at (`CursorTarget::new`): a bar's
    /// thickness, else its cell's, so a change of kind is a change of width
    /// the size transition morphs.
    pub(super) width: f32,
    /// The width of the cell it stands on, the glide's measure of distance.
    pub(super) cell: f32,
    pub(super) height: f32,
    pub(super) style: CursorStyle,
    /// Which frame owns this cursor (0 = root frame, non-zero = child frame_id)
    pub(super) frame_id: u64,
}

impl CursorTarget {
    /// A cursor of STYLE on the cell at X, Y, CELL by HEIGHT.
    pub(super) fn new(window_id: i64, (x, y, cell, height): (f32, f32, f32, f32), style: CursorStyle, frame_id: u64) -> Self {
        let width = match style {
            CursorStyle::Bar(thickness) => thickness.min(cell),
            _ => cell,
        };
        Self { window_id, x, y, width, cell, height, style, frame_id }
    }
}

/// Per-corner spring state for the 4-corner cursor trail animation.
/// Each corner has its own position, velocity, and spring frequency.
#[derive(Debug, Clone, Copy)]
pub(super) struct CornerSpring {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) vx: f32,
    pub(super) vy: f32,
    pub(super) target_x: f32,
    pub(super) target_y: f32,
    pub(super) omega: f32,
}

/// A critically damped spring with angular frequency w has come 99% of
/// its way at time ARRIVAL / w: (1 + u) e^-u = 0.01 at u = 6.64.
const ARRIVAL: f32 = 6.64;

/// Copyable cursor settings, excluding live animation and target state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CursorConfigSnapshot {
    anim_enabled: bool,
    anim_speed: f32,
    anim_style: CursorAnimStyle,
    anim_duration: f32,
    short_duration: f32,
    trail_size: f32,
    distance_length_adjust: bool,
    size_transition_enabled: bool,
    size_transition_duration: f32,
}

/// Cursor animation and size transition state.
///
/// Extracted from RenderApp to group all cursor-related fields together.
pub(super) struct CursorState {
    // Animation (smooth motion)
    pub(super) anim_enabled: bool,
    pub(super) anim_speed: f32,
    pub(super) anim_style: CursorAnimStyle,
    pub(super) anim_duration: f32, // seconds, for non-Exponential styles
    /// Seconds a short move takes in the `neovide` style.
    pub(super) short_duration: f32,
    pub(super) target: Option<CursorTarget>,
    pub(super) current_x: f32,
    pub(super) current_y: f32,
    pub(super) current_w: f32,
    pub(super) current_h: f32,
    pub(super) animating: bool,
    pub(super) last_anim_time: romacs_display_protocol::frame_time::EventTime,
    // For easing/linear styles: capture start position when animation begins
    pub(super) start_x: f32,
    pub(super) start_y: f32,
    pub(super) start_w: f32,
    pub(super) start_h: f32,
    pub(super) anim_start_time: romacs_display_protocol::frame_time::EventTime,
    // 4-corner spring trail state (TL, TR, BR, BL)
    pub(super) corner_springs: [CornerSpring; 4],
    pub(super) trail_size: f32,
    distance_length_adjust: bool,

    // Size transition (independent of position animation)
    pub(super) size_transition_enabled: bool,
    pub(super) size_transition_duration: f32, // seconds
    pub(super) size_animating: bool,
    pub(super) size_start_w: f32,
    pub(super) size_start_h: f32,
    pub(super) size_target_w: f32,
    pub(super) size_target_h: f32,
    pub(super) size_anim_start: romacs_display_protocol::frame_time::EventTime,

    // The frame's focus as its cursor shows it (`romacs-cursor-look'): 1
    // focused, 0 not, eased between over the size transition's time.
    pub(super) light: f32,
    light_from: f32,
    focused: bool,
    focus_since: romacs_display_protocol::frame_time::EventTime,
}

impl CursorState {
    /// Cursor state for a window that has just been created.
    ///
    /// There is no `Default`: every timing anchor here needs a real moment,
    /// and the four `Instant::now()` calls this replaces could disagree with
    /// each other by however long construction took.
    pub(super) fn new(at: romacs_display_protocol::frame_time::EventTime) -> Self {
        let visual = VisualConfig::default();
        Self {
            anim_enabled: visual.cursor_motion.enabled,
            anim_speed: visual.cursor_motion.speed,
            anim_style: visual.cursor_motion.style,
            anim_duration: visual.cursor_motion.duration.as_secs_f32(),
            short_duration: visual.cursor_motion.short_duration.as_secs_f32(),
            target: None,
            current_x: 0.0,
            current_y: 0.0,
            current_w: 0.0,
            current_h: 0.0,
            animating: false,
            last_anim_time: at,
            start_x: 0.0,
            start_y: 0.0,
            start_w: 0.0,
            start_h: 0.0,
            anim_start_time: at,
            corner_springs: [CornerSpring {
                x: 0.0,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                target_x: 0.0,
                target_y: 0.0,
                omega: 26.7,
            }; 4],
            trail_size: visual.cursor_motion.trail_size,
            distance_length_adjust: visual.cursor_motion.distance_length_adjust,
            size_transition_enabled: look_morphs(&visual.cursor_look),
            size_transition_duration: visual.cursor_look.morph.as_secs_f32(),
            size_animating: false,
            size_start_w: 0.0,
            size_start_h: 0.0,
            size_target_w: 0.0,
            size_target_h: 0.0,
            size_anim_start: at,
            light: 1.0,
            light_from: 1.0,
            focused: true,
            focus_since: at,
        }
    }
}

/// Whether LOOK morphs the cursor between kinds and sizes: any look but
/// `plain' (GNU's, which changes at once), in its `morph' time.
fn look_morphs(look: &romacs_display_protocol::CursorLookConfig) -> bool {
    look.style != romacs_display_protocol::CursorLookStyle::Plain && !look.morph.is_zero()
}

impl CursorState {
    pub(super) fn apply_visual_config(&mut self, config: &VisualConfig) {
        self.anim_enabled = config.cursor_motion.enabled;
        self.anim_speed = config.cursor_motion.speed;
        self.anim_style = config.cursor_motion.style;
        self.anim_duration = config.cursor_motion.duration.as_secs_f32();
        self.short_duration = config.cursor_motion.short_duration.as_secs_f32();
        self.trail_size = config.cursor_motion.trail_size;
        self.distance_length_adjust = config.cursor_motion.distance_length_adjust;
        self.size_transition_enabled = look_morphs(&config.cursor_look);
        self.size_transition_duration = config.cursor_look.morph.as_secs_f32();
        self.settle_disabled();
    }

    /// Bring a motion that was just turned off to where it was going: a
    /// glide stopped midway would otherwise strand the cursor there.
    fn settle_disabled(&mut self) {
        if !self.anim_enabled && self.animating {
            match self.target.clone() {
                Some(target) => self.snap(&target),
                None => self.animating = false,
            }
        }
        if !self.size_transition_enabled && self.size_animating {
            self.current_w = self.size_target_w;
            self.current_h = self.size_target_h;
            self.size_animating = false;
        }
    }

    pub(super) fn config_snapshot(&self) -> CursorConfigSnapshot {
        CursorConfigSnapshot {
            anim_enabled: self.anim_enabled,
            anim_speed: self.anim_speed,
            anim_style: self.anim_style,
            anim_duration: self.anim_duration,
            short_duration: self.short_duration,
            trail_size: self.trail_size,
            distance_length_adjust: self.distance_length_adjust,
            size_transition_enabled: self.size_transition_enabled,
            size_transition_duration: self.size_transition_duration,
        }
    }

    pub(super) fn apply_config(&mut self, config: CursorConfigSnapshot) {
        self.anim_enabled = config.anim_enabled;
        self.anim_speed = config.anim_speed;
        self.anim_style = config.anim_style;
        self.anim_duration = config.anim_duration;
        self.short_duration = config.short_duration;
        self.trail_size = config.trail_size;
        self.distance_length_adjust = config.distance_length_adjust;
        self.size_transition_enabled = config.size_transition_enabled;
        self.size_transition_duration = config.size_transition_duration;
        self.settle_disabled();
    }

    pub(super) fn set_target(
        &mut self,
        new_target: CursorTarget,
        at: romacs_display_protocol::frame_time::EventTime,
    ) -> (bool, bool) {
        let had_target = self.target.is_some();
        let same_window = self.target.as_ref().is_some_and(|old| {
            old.window_id == new_target.window_id && old.frame_id == new_target.frame_id
        });
        let target_moved = self.target.as_ref().is_none_or(|old| {
            (old.x - new_target.x).abs() > 0.5
                || (old.y - new_target.y).abs() > 0.5
                || (old.width - new_target.width).abs() > 0.5
                || (old.height - new_target.height).abs() > 0.5
        });
        // One owner for each change of shape.  A cursor that moves (or is
        // already gliding) glides, the glide carrying its change of kind or
        // size with it; one that changes shape where it stands morphs, the
        // size transition alone easing its width and height.  Both writing
        // them would pull the width between the two.
        let stays = self.target.as_ref().is_some_and(|old| {
            same_window && (old.x - new_target.x).abs() <= 0.5 && (old.y - new_target.y).abs() <= 0.5
        });
        let glides = had_target && self.anim_enabled && target_moved && (!stays || self.animating);
        let morphs = self.size_transition_enabled
            && had_target
            && !glides
            && ((new_target.width - self.size_target_w).abs() > 2.0
                || (new_target.height - self.size_target_h).abs() > 2.0);

        if glides {
            if self.size_animating {
                // A morph cut short by a move: the glide starts from the
                // shape shown, not from where the morph was going.
                let (x, y, w, h) = (self.current_x, self.current_y, self.current_w, self.current_h);
                for (spring, (cx, cy)) in self.corner_springs.iter_mut().zip([(x, y), (x + w, y), (x + w, y + h), (x, y + h)]) {
                    (spring.x, spring.y, spring.vx, spring.vy) = (cx, cy, 0.0, 0.0);
                }
            }
            self.animating = true;
            self.last_anim_time = at;
            self.start_x = self.current_x;
            self.start_y = self.current_y;
            self.start_w = self.current_w;
            self.start_h = self.current_h;
            self.anim_start_time = at;

            if matches!(
                self.anim_style,
                CursorAnimStyle::Neovide | CursorAnimStyle::CriticallyDampedSpring
            ) {
                self.aim_corners(&new_target, same_window);
            }
            self.size_animating = false;
        } else if !had_target || target_moved || !self.anim_enabled {
            // The same target again leaves a glide in flight alone.
            if morphs {
                self.size_animating = true;
                self.size_start_w = self.current_w;
                self.size_start_h = self.current_h;
                self.size_anim_start = at;
            } else if !self.size_animating {
                self.current_w = new_target.width;
                self.current_h = new_target.height;
            }
            self.current_x = new_target.x;
            self.current_y = new_target.y;
            self.animating = false;
            let corners = Self::target_corners(&new_target);
            for (spring, (x, y)) in self.corner_springs.iter_mut().zip(corners) {
                spring.x = x;
                spring.y = y;
                spring.vx = 0.0;
                spring.vy = 0.0;
                spring.target_x = x;
                spring.target_y = y;
            }
        }
        self.size_target_w = new_target.width;
        self.size_target_h = new_target.height;

        self.target = Some(new_target);
        (had_target, target_moved)
    }

    pub(super) fn animated_cursor(&self) -> Option<AnimatedCursor> {
        let target = self.target.as_ref()?;
        // A morph shows through the animated rect even with the glide off.
        if !self.anim_enabled && !self.size_animating {
            return None;
        }
        let corners = if matches!(
            self.anim_style,
            CursorAnimStyle::CriticallyDampedSpring | CursorAnimStyle::Neovide
        ) && self.animating
        {
            Some([
                (self.corner_springs[0].x, self.corner_springs[0].y),
                (self.corner_springs[1].x, self.corner_springs[1].y),
                (self.corner_springs[2].x, self.corner_springs[2].y),
                (self.corner_springs[3].x, self.corner_springs[3].y),
            ])
        } else {
            None
        };
        Some(AnimatedCursor {
            window_id: DisplayWindowId::new(target.window_id),
            x: self.current_x,
            y: self.current_y,
            width: self.current_w,
            height: self.current_h,
            corners,
            frame_id: DisplayFrameId::new(target.frame_id),
        })
    }

    pub(super) fn target_cloned(&self) -> Option<CursorTarget> {
        self.target.clone()
    }

    pub(super) fn clear_target(&mut self) {
        self.target = None;
        self.animating = false;
        self.size_animating = false;
    }

    pub(super) fn is_animating(&self) -> bool {
        self.animating || self.size_animating || self.light_fading()
    }

    /// Whether the focus light is between lit and unlit.
    pub(super) fn light_fading(&self) -> bool {
        self.light != self.light_target()
    }

    fn light_target(&self) -> f32 {
        if self.focused { 1.0 } else { 0.0 }
    }

    /// The frame gained or lost focus AT: its cursor's light fades from
    /// where it is toward lit or unlit.
    pub(super) fn set_focused(&mut self, focused: bool, at: romacs_display_protocol::frame_time::EventTime) {
        if focused != self.focused {
            self.light_from = self.light;
            self.focused = focused;
            self.focus_since = at;
        }
    }

    /// The window this cursor is drawn in has, or has not, the focus as it
    /// appears: lit or unlit at once.  A window made without the focus
    /// never hears that it lost it.
    pub(super) fn seed_focus(&mut self, focused: bool) {
        self.focused = focused;
        self.light = self.light_target();
        self.light_from = self.light;
    }

    /// Advance the focus light to AT; whether it changed (needs redraw).
    pub(super) fn tick_light(&mut self, at: romacs_display_protocol::frame_time::EventTime) -> bool {
        let target = self.light_target();
        let next = if !self.size_transition_enabled || self.size_transition_duration <= 0.0 {
            target
        } else {
            let t = (at.saturating_since(self.focus_since).as_secs_f32() / self.size_transition_duration).min(1.0);
            self.light_from + (target - self.light_from) * t
        };
        let changed = next != self.light;
        self.light = next;
        changed
    }

    /// Point each corner's spring at NEW's matching corner, with its own
    /// timing, so the cursor keeps the velocity it has.
    ///
    /// In the `neovide` style a short move, within two cells and one row
    /// of the same window, carries the cursor whole in `short_duration`:
    /// typing and line motion stay quiet. Anything farther, and any move
    /// to another window, stretches the cursor along its travel: the
    /// corners leading it take `1 - trail_size` of the time the trailing
    /// ones take, and with `distance_length_adjust` that time grows with
    /// the jump's length in cells, to 1.6 times `duration`. Each time is
    /// when a corner has come 99% of its way. The
    /// `critically-damped-spring` style always stretches, in `duration`.
    fn aim_corners(&mut self, new: &CursorTarget, same_window: bool) {
        let neovide = self.anim_style == CursorAnimStyle::Neovide;
        let corners = Self::target_corners(new);
        let dx = (new.x + new.width / 2.0) - (self.current_x + self.current_w / 2.0);
        let dy = (new.y + new.height / 2.0) - (self.current_y + self.current_h / 2.0);
        let cells = (dx / new.cell.max(1.0), dy / new.height.max(1.0));
        let short = neovide && same_window && cells.0.abs() <= 2.0 + f32::EPSILON
            && cells.1.abs() <= 1.0 + f32::EPSILON;
        let durations = if short {
            [self.short_duration; 4]
        } else {
            let reach = if neovide && self.distance_length_adjust {
                (1.0 + 0.5 * (cells.0.hypot(cells.1) / 4.0).log10().max(0.0)).min(1.6)
            } else {
                1.0
            };
            let trailing = self.anim_duration * reach;
            let length = dx.hypot(dy);
            let (dir_x, dir_y) = if length > 0.001 { (dx / length, dy / length) } else { (1.0, 0.0) };
            // How far each corner points along the travel, from the
            // trailing (0) to the leading (1): corners the travel treats
            // alike move alike, so a straight move does not shear.
            let along = [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(x, y)| x * dir_x + y * dir_y);
            let (low, high) = along.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &a| {
                (lo.min(a), hi.max(a))
            });
            let trail = self.trail_size.clamp(0.0, 1.0);
            along.map(|a| {
                let lead = if high - low > 1e-3 { (a - low) / (high - low) } else { 0.0 };
                trailing * (1.0 - trail * lead)
            })
        };
        for ((spring, corner), duration) in self.corner_springs.iter_mut().zip(corners).zip(durations) {
            spring.target_x = corner.0;
            spring.target_y = corner.1;
            spring.omega = if duration > 0.0 { ARRIVAL / duration } else { f32::INFINITY };
        }
    }

    /// Compute the 4 target corners for a cursor based on its style.
    /// Returns [TL, TR, BR, BL] as (x, y) tuples.
    pub(super) fn target_corners(target: &CursorTarget) -> [(f32, f32); 4] {
        match target.style {
            CursorStyle::FilledBox => {
                // Filled box: full rectangle
                let x0 = target.x;
                let y0 = target.y;
                let x1 = target.x + target.width;
                let y1 = target.y + target.height;
                [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            }
            CursorStyle::Bar(bar_w) => {
                // Bar: thin vertical line
                let x0 = target.x;
                let y0 = target.y;
                let x1 = target.x + bar_w;
                let y1 = target.y + target.height;
                [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            }
            CursorStyle::Hbar(hbar_h) => {
                // Underline: thin horizontal line at bottom
                let x0 = target.x;
                let y0 = target.y + target.height - hbar_h;
                let x1 = target.x + target.width;
                let y1 = target.y + target.height;
                [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            }
            CursorStyle::Hollow => {
                // Hollow: full rectangle (border only drawn elsewhere)
                let x0 = target.x;
                let y0 = target.y;
                let x1 = target.x + target.width;
                let y1 = target.y + target.height;
                [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            }
        }
    }

    /// Tick cursor animation, returns true if position changed (needs redraw)
    pub(super) fn tick_animation(
        &mut self,
        at: romacs_display_protocol::frame_time::EventTime,
    ) -> bool {
        if !self.anim_enabled || !self.animating {
            return false;
        }

        // The anchor belongs to the integrator, so it advances on every tick
        // this gate admits -- including one that finds no target. Leaving it
        // behind there would make the next tick with a target measure `dt`
        // across both intervals and take a doubled, visible step.
        let dt = at.saturating_since(self.last_anim_time).as_secs_f32();
        self.last_anim_time = at;

        let target = match self.target.as_ref() {
            Some(t) => t.clone(),
            None => return false,
        };

        match self.anim_style {
            CursorAnimStyle::Exponential => {
                let factor = 1.0 - (-self.anim_speed * dt).exp();
                let dx = target.x - self.current_x;
                let dy = target.y - self.current_y;
                let dw = target.width - self.current_w;
                let dh = target.height - self.current_h;
                self.current_x += dx * factor;
                self.current_y += dy * factor;
                self.current_w += dw * factor;
                self.current_h += dh * factor;
                if dx.abs() < 0.5 && dy.abs() < 0.5 && dw.abs() < 0.5 && dh.abs() < 0.5 {
                    self.snap(&target);
                }
            }
            CursorAnimStyle::Neovide | CursorAnimStyle::CriticallyDampedSpring => {
                let mut all_settled = true;
                for i in 0..4 {
                    let spring = &mut self.corner_springs[i];
                    let omega = spring.omega;
                    if !omega.is_finite() {
                        // A corner given no time arrives at once.
                        (spring.x, spring.y, spring.vx, spring.vy) =
                            (spring.target_x, spring.target_y, 0.0, 0.0);
                        continue;
                    }
                    let exp_term = (-omega * dt).exp();

                    let x0 = spring.x - spring.target_x;
                    let vx0 = spring.vx;
                    let new_x = (x0 + (vx0 + omega * x0) * dt) * exp_term;
                    spring.vx = ((vx0 + omega * x0) * exp_term)
                        - omega * (x0 + (vx0 + omega * x0) * dt) * exp_term;
                    spring.x = spring.target_x + new_x;

                    let y0 = spring.y - spring.target_y;
                    let vy0 = spring.vy;
                    let new_y = (y0 + (vy0 + omega * y0) * dt) * exp_term;
                    spring.vy = ((vy0 + omega * y0) * exp_term)
                        - omega * (y0 + (vy0 + omega * y0) * dt) * exp_term;
                    spring.y = spring.target_y + new_y;

                    let dist =
                        (spring.x - spring.target_x).abs() + (spring.y - spring.target_y).abs();
                    let vel = spring.vx.abs() + spring.vy.abs();
                    // At rest once it is within half a pixel and would
                    // move less than that in a 60 Hz frame: a spring's tail
                    // creeps on long after it looks arrived.
                    if dist > 0.5 || vel > 30.0 {
                        all_settled = false;
                    }
                }

                let min_x = self
                    .corner_springs
                    .iter()
                    .map(|s| s.x)
                    .fold(f32::INFINITY, f32::min);
                let min_y = self
                    .corner_springs
                    .iter()
                    .map(|s| s.y)
                    .fold(f32::INFINITY, f32::min);
                let max_x = self
                    .corner_springs
                    .iter()
                    .map(|s| s.x)
                    .fold(f32::NEG_INFINITY, f32::max);
                let max_y = self
                    .corner_springs
                    .iter()
                    .map(|s| s.y)
                    .fold(f32::NEG_INFINITY, f32::max);
                self.current_x = min_x;
                self.current_y = min_y;
                self.current_w = max_x - min_x;
                self.current_h = max_y - min_y;

                if all_settled {
                    let target_corners = Self::target_corners(&target);
                    for (spring, corner) in
                        self.corner_springs.iter_mut().zip(target_corners.iter())
                    {
                        spring.x = corner.0;
                        spring.y = corner.1;
                    }
                    // Velocities are cleared by `snap`, which every style's
                    // completion path shares.
                    self.snap(&target);
                }
            }
            style => {
                let elapsed = at.saturating_since(self.anim_start_time).as_secs_f32();
                // A duration that is not positive means "no animation": finish
                // at once. Dividing by it would leave completion to float edge
                // cases -- 0.0/0.0 is NaN and only snaps because `f32::min`
                // discards NaN, and a negative duration would run backwards
                // and never reach 1.0 at all.
                let raw_t = if self.anim_duration > 0.0 {
                    (elapsed / self.anim_duration).min(1.0)
                } else {
                    1.0
                };
                let t = match style {
                    CursorAnimStyle::EaseOutQuad => ease_out_quad(raw_t),
                    CursorAnimStyle::EaseOutCubic => ease_out_cubic(raw_t),
                    CursorAnimStyle::EaseOutExpo => ease_out_expo(raw_t),
                    CursorAnimStyle::EaseInOutCubic => ease_in_out_cubic(raw_t),
                    CursorAnimStyle::Linear => ease_linear(raw_t),
                    _ => raw_t,
                };
                self.current_x = self.start_x + (target.x - self.start_x) * t;
                self.current_y = self.start_y + (target.y - self.start_y) * t;
                self.current_w = self.start_w + (target.width - self.start_w) * t;
                self.current_h = self.start_h + (target.height - self.start_h) * t;
                if raw_t >= 1.0 {
                    self.snap(&target);
                }
            }
        }

        true
    }

    /// Snap cursor to target and stop animating.
    ///
    /// This is the single exit from cursor motion -- every style's completion
    /// path ends here -- so it also brings the per-corner springs to rest.
    /// A spring left with velocity by one style's completion check would carry
    /// that velocity into the next animation.
    pub(super) fn snap(&mut self, target: &CursorTarget) {
        self.current_x = target.x;
        self.current_y = target.y;
        self.current_w = target.width;
        self.current_h = target.height;
        for (spring, (x, y)) in self
            .corner_springs
            .iter_mut()
            .zip(Self::target_corners(target))
        {
            spring.x = x;
            spring.y = y;
            spring.vx = 0.0;
            spring.vy = 0.0;
        }
        self.animating = false;
    }

    /// Tick cursor size transition, returns true if size changed (needs redraw).
    pub(super) fn tick_size_animation(
        &mut self,
        at: romacs_display_protocol::frame_time::EventTime,
    ) -> bool {
        if !self.size_transition_enabled || !self.size_animating {
            return false;
        }
        let elapsed = at.saturating_since(self.size_anim_start).as_secs_f32();
        let raw_t = (elapsed / self.size_transition_duration).min(1.0);
        let t = raw_t * (2.0 - raw_t); // ease-out-quad
        self.current_w = self.size_start_w + (self.size_target_w - self.size_start_w) * t;
        self.current_h = self.size_start_h + (self.size_target_h - self.size_start_h) * t;
        if raw_t >= 1.0 {
            self.current_w = self.size_target_w;
            self.current_h = self.size_target_h;
            self.size_animating = false;
        }
        true
    }
}

#[cfg(test)]
mod tests;
