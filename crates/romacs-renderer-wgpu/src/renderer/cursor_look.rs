//! The cursor's look (`romacs-cursor-look-*`): how much of a cursor shows
//! and the light around it, from the look, whether it is its frame's active
//! cursor, and the frame's focus light.  Pure: the cursor emitter draws what
//! this decides, so every path that draws a cursor shows the same one.

use romacs_display_protocol::{CursorLookConfig, CursorLookStyle, CursorStyle};

/// One cursor's light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CursorLight {
    /// The cursor's opacity, 1 for a lit cursor, the ghost's for an
    /// unselected or unfocused one.
    pub(super) body: f32,
    /// The glow's opacity around a bar or hbar; 0 draws none.
    pub(super) glow: f32,
    /// A filled box drawn as its outline: an unfocused frame's cursor, as
    /// GNU draws it hollow.
    pub(super) hollow_box: bool,
}

/// The light of a cursor of STYLE under LOOK: ACTIVE when it is its
/// frame's selected window's cursor, FOCUS the frame's focus light (1
/// focused, 0 not, between while it fades).
pub(super) fn cursor_light(look: &CursorLookConfig, style: CursorStyle, active: bool, focus: f32) -> CursorLight {
    if look.style == CursorLookStyle::Plain {
        return CursorLight { body: 1.0, glow: 0.0, hollow_box: false };
    }
    let ghost = look.ghost_opacity.clamp(0.0, 1.0);
    let lit = if active { focus.clamp(0.0, 1.0) } else { 0.0 };
    let line = matches!(style, CursorStyle::Bar(_) | CursorStyle::Hbar(_));
    CursorLight {
        body: ghost + (1.0 - ghost) * lit,
        glow: if line { look.glow_opacity.clamp(0.0, 1.0) * lit } else { 0.0 },
        hollow_box: matches!(style, CursorStyle::FilledBox) && lit < 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_draws_every_cursor_whole_and_unlit() {
        let look = CursorLookConfig { style: CursorLookStyle::Plain, ..Default::default() };
        for (style, active, focus) in [(CursorStyle::Bar(2.0), true, 0.0), (CursorStyle::FilledBox, false, 1.0)] {
            assert_eq!(cursor_light(&look, style, active, focus), CursorLight { body: 1.0, glow: 0.0, hollow_box: false });
        }
    }

    #[test]
    fn soft_lights_the_focused_cursor_and_ghosts_the_rest() {
        let look = CursorLookConfig::default();
        let lit = cursor_light(&look, CursorStyle::Bar(2.0), true, 1.0);
        assert_eq!((lit.body, lit.glow, lit.hollow_box), (1.0, look.glow_opacity, false));
        let unselected = cursor_light(&look, CursorStyle::Bar(1.0), false, 1.0);
        assert_eq!((unselected.body, unselected.glow), (look.ghost_opacity, 0.0));
        let unfocused = cursor_light(&look, CursorStyle::FilledBox, true, 0.0);
        assert_eq!((unfocused.body, unfocused.hollow_box), (look.ghost_opacity, true));
        let fading = cursor_light(&look, CursorStyle::Bar(2.0), true, 0.5);
        assert!(fading.body > look.ghost_opacity && fading.body < 1.0 && fading.glow > 0.0 && fading.glow < look.glow_opacity);
        assert_eq!(cursor_light(&look, CursorStyle::FilledBox, true, 1.0).glow, 0.0, "a block is not lit");
    }
}
