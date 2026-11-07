//! Complete user-facing visual configuration snapshot.
//!
//! Shader effects, cursor behavior, and window transitions share one control
//! plane even though the renderer stores and executes them in different
//! subsystems.  Keeping that distinction behind `VisualConfig` gives Elisp a
//! single named, typed, atomic interface without flattening the runtime model.

use crate::child_frame_animation::{
    ChildFrameAnimation, ChildFrameAnimationsConfig, default_child_frame_close,
    default_child_frame_movement, default_child_frame_open, default_child_frame_resize,
    default_child_frame_selection,
};
use crate::window_animation::{
    WindowAnimation, WindowAnimationsConfig, default_window_close, default_window_movement,
    default_window_open, default_window_resize,
};
use crate::{
    CursorAnimStyle, EffectsConfig, TransitionAxisPreference, TransitionDirection,
    TransitionEasing, TransitionEffect,
};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorMotionConfig {
    pub enabled: bool,
    pub speed: f32,
    pub style: CursorAnimStyle,
    pub duration: Duration,
    /// How long a short move takes in the `neovide` style: within two
    /// cells and one row of the same window.
    pub short_duration: Duration,
    pub trail_size: f32,
    pub distance_length_adjust: bool,
}

impl Default for CursorMotionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
            style: CursorAnimStyle::Neovide,
            duration: Duration::from_millis(100),
            short_duration: Duration::from_millis(40),
            trail_size: 0.7,
            distance_length_adjust: true,
        }
    }
}

/// How the cursor is drawn (`CursorLookStyle`), with its light: a glow
/// GLOW_REACH pixels around the selected cursor at GLOW_OPACITY, an
/// unselected or unfocused cursor at GHOST_OPACITY, and MORPH, how long a
/// change of kind (a bar becoming a block) and a focus change take.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorLookConfig {
    pub style: crate::CursorLookStyle,
    pub glow_reach: f32,
    pub glow_opacity: f32,
    pub ghost_opacity: f32,
    pub morph: Duration,
}

impl Default for CursorLookConfig {
    fn default() -> Self {
        Self {
            style: crate::CursorLookStyle::Soft,
            glow_reach: 6.0,
            glow_opacity: 0.35,
            ghost_opacity: 0.35,
            morph: Duration::from_millis(120),
        }
    }
}

/// Motion as a whole: REDUCED turns every animation off, the cursor's,
/// transitions', windows' and layers', for the user or the desktop that
/// asks for less motion.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MotionConfig {
    pub reduced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BufferTransitionConfig {
    pub enabled: bool,
    pub duration: Duration,
    pub effect: TransitionEffect,
    pub easing: TransitionEasing,
    /// Preferred orientation for effects that support either axis.
    pub axis: TransitionAxisPreference,
    /// Semantic movement direction for effects that move content. Forward
    /// moves old content left/up and introduces new content from the
    /// right/bottom.
    pub direction: TransitionDirection,
}

impl Default for BufferTransitionConfig {
    fn default() -> Self {
        Self {
            // Off by default: switching buffers repaints instantly, matching
            // stock Emacs, the same choice ScrollTransitionConfig makes below.
            // Opt in at runtime with
            //   (setq romacs-buffer-transition-enabled t)
            // or via the `romacs-effects' profile. The effect/duration/easing
            // below are the values used once it is enabled.
            enabled: false,
            duration: Duration::from_millis(200),
            effect: TransitionEffect::Slide,
            easing: TransitionEasing::EaseOutQuad,
            axis: TransitionAxisPreference::Auto,
            direction: TransitionDirection::Forward,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScrollTransitionConfig {
    pub enabled: bool,
    pub duration: Duration,
    pub effect: TransitionEffect,
    pub easing: TransitionEasing,
}

impl Default for ScrollTransitionConfig {
    fn default() -> Self {
        Self {
            // Off by default: C-v/M-v (and other scrolls) update instantly,
            // matching stock Emacs. Opt in at runtime with
            //   (setq romacs-scroll-transition-enabled t)
            // or via the `romacs-effects' profile. The effect/duration/easing
            // below are the values used once it is enabled.
            enabled: false,
            duration: Duration::from_millis(150),
            effect: TransitionEffect::Slide,
            easing: TransitionEasing::EaseOutQuad,
        }
    }
}

/// Desired visual configuration owned by the evaluator and published as one
/// snapshot to the render thread.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VisualConfig {
    /// The large shader-effect catalog remains a focused renderer type.  Serde
    /// flattening exposes it beside the behavioral configs in the registry.
    #[serde(flatten)]
    pub effects: EffectsConfig,
    pub cursor_motion: CursorMotionConfig,
    #[serde(default)]
    pub cursor_look: CursorLookConfig,
    #[serde(default)]
    pub motion: MotionConfig,
    pub buffer_transition: BufferTransitionConfig,
    pub scroll_transition: ScrollTransitionConfig,
    /// Global controls over every window-animation slot below.
    #[serde(default)]
    pub window_animations: WindowAnimationsConfig,
    /// A window appearing: `split-window` and anything else that adds one.
    #[serde(default = "default_window_open")]
    pub window_open: WindowAnimation,
    /// A window going away: `delete-window`, `delete-other-windows`.
    #[serde(default = "default_window_close")]
    pub window_close: WindowAnimation,
    /// A surviving window whose *size* changes. Drives geometry.
    #[serde(default = "default_window_resize")]
    pub window_resize: WindowAnimation,
    /// A surviving window that only moves. Drives geometry.
    #[serde(default = "default_window_movement")]
    pub window_movement: WindowAnimation,
    /// Global controls over every child-frame-animation slot below.
    #[serde(default)]
    pub child_frame_animations: ChildFrameAnimationsConfig,
    /// A child frame appearing: posframe and Corfu popups fading in.
    #[serde(default = "default_child_frame_open")]
    pub child_frame_open: ChildFrameAnimation,
    /// A child frame going away: `delete-frame` on a child frame.
    #[serde(default = "default_child_frame_close")]
    pub child_frame_close: ChildFrameAnimation,
    /// A child frame whose anchor moved. Drives placement only.
    #[serde(default = "default_child_frame_movement")]
    pub child_frame_movement: ChildFrameAnimation,
    /// A child frame whose size changes: its rect tweens, content placed
    /// as it settles.
    #[serde(default = "default_child_frame_resize")]
    pub child_frame_resize: ChildFrameAnimation,
    /// A layer's selection highlight moving between rows.
    #[serde(default = "default_child_frame_selection")]
    pub child_frame_selection: ChildFrameAnimation,
}

/// Written out rather than derived, because the four window-animation slots do
/// not share a default: opening and closing are easings, resizing and moving
/// are springs, and closing ships disabled. A `#[derive(Default)]` would give
/// all four the same value and silently ignore the `#[serde(default = ...)]`
/// attributes above, which only apply to deserialization.
impl Default for VisualConfig {
    fn default() -> Self {
        Self {
            effects: EffectsConfig::default(),
            cursor_motion: CursorMotionConfig::default(),
            cursor_look: CursorLookConfig::default(),
            motion: MotionConfig::default(),
            buffer_transition: BufferTransitionConfig::default(),
            scroll_transition: ScrollTransitionConfig::default(),
            window_animations: WindowAnimationsConfig::default(),
            window_open: default_window_open(),
            window_close: default_window_close(),
            window_resize: default_window_resize(),
            window_movement: default_window_movement(),
            child_frame_animations: ChildFrameAnimationsConfig::default(),
            child_frame_open: default_child_frame_open(),
            child_frame_close: default_child_frame_close(),
            child_frame_movement: default_child_frame_movement(),
            child_frame_resize: default_child_frame_resize(),
            child_frame_selection: default_child_frame_selection(),
        }
    }
}

// The behavioural slots publish their schema too. Hand-written rather than
// macro-declared because their defaults differ per instance, but a widget needs
// the same description of them either way.
crate::effect_schema!(CursorMotionConfig {
    enabled: bool,
    speed: f32,
    style: CursorAnimStyle,
    duration: Duration,
    short_duration: Duration,
    trail_size: f32,
    distance_length_adjust: bool,
});
crate::effect_schema!(CursorLookConfig {
    style: crate::CursorLookStyle,
    glow_reach: f32,
    glow_opacity: f32,
    ghost_opacity: f32,
    morph: Duration,
});
crate::effect_schema!(MotionConfig { reduced: bool });
crate::effect_schema!(BufferTransitionConfig {
    enabled: bool,
    duration: Duration,
    effect: TransitionEffect,
    easing: TransitionEasing,
    axis: TransitionAxisPreference,
    direction: TransitionDirection,
});
crate::effect_schema!(ScrollTransitionConfig {
    enabled: bool,
    duration: Duration,
    effect: TransitionEffect,
    easing: TransitionEasing,
});
