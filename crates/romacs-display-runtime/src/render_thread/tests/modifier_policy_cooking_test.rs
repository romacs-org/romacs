//! The render-thread half of issue #442: the compiled NS modifier policy
//! cooking raw modifier facts into the transport bits Emacs receives.
//!
//! The arithmetic being pinned is GNU's `EV_MODIFIERS2`
//! (`src/nsterm.m:397-425`): the default policy must keep the pre-policy
//! hardcode's answers (Option -> meta, Command -> super), and a policy push
//! from Lisp must change what Emacs sees on the very next key event.

use super::RenderApp;
use super::tests::make_test_app;
use romacs_display_protocol::{
    EmacsModifierKey, ModifierEventKind, ModifierPolicy, ModifierSideState, RawModifiers,
    TransportModifierBit,
};
use winit::keyboard::ModifiersState;

/// Simulate the aggregate answer `ModifiersChanged` would install.
fn set_aggregate(app: &mut RenderApp, shift: bool, ctrl: bool, alt: bool, meta: bool) {
    let mut state = ModifiersState::empty();
    if shift {
        state |= ModifiersState::SHIFT;
    }
    if ctrl {
        state |= ModifiersState::CONTROL;
    }
    if alt {
        state |= ModifiersState::ALT;
    }
    if meta {
        state |= ModifiersState::META;
    }
    app.modifier_state = state;
}

/// The default policy must reproduce the pre-policy hardcode:
/// `ModifiersChanged` mapped alt(Option) -> META and meta(Command) -> SUPER
/// (`window_events.rs` before this change).
#[test]
fn default_policy_keeps_the_pre_policy_hardcode() {
    let mut app = make_test_app();
    assert_eq!(app.modifier_policy, ModifierPolicy::gnu_ns_default());

    // Option, Command and Control all down; sides unobserved -> GNU's
    // left rule.
    set_aggregate(&mut app, false, true, true, true);
    let ordinary = app.cook_modifiers(ModifierEventKind::Ordinary);
    assert!(ordinary.get(TransportModifierBit::Meta));
    assert!(ordinary.get(TransportModifierBit::Super));
    assert!(ordinary.get(TransportModifierBit::Ctrl));
    assert!(!ordinary.get(TransportModifierBit::Alt));
}

/// GNU's arithmetic, restated through the raw facts the transport types:
/// the union rules for every ModifierSideState on the command family.
#[test]
fn side_states_cook_like_ev_modifiers_helper() {
    let policy = ModifierPolicy::gnu_ns_default()
        .with_command(EmacsModifierKey::Meta)
        .with_right_command(EmacsModifierKey::Hyper);
    let cook = |side: Option<ModifierSideState>| -> u32 {
        policy
            .cook(
                RawModifiers {
                    shift: false,
                    ctrl: None,
                    command: side,
                    option: None,
                },
                ModifierEventKind::Ordinary,
            )
            .bits()
    };
    let meta = TransportModifierBit::Meta.mask();
    let hyper = TransportModifierBit::Hyper.mask();
    assert_eq!(cook(None), 0, "family up");
    assert_eq!(
        cook(Some(ModifierSideState::Unknown)),
        meta,
        "unidentified -> left"
    );
    assert_eq!(cook(Some(ModifierSideState::LeftOnly)), meta);
    assert_eq!(cook(Some(ModifierSideState::RightOnly)), hyper);
    assert_eq!(
        cook(Some(ModifierSideState::Both)),
        meta | hyper,
        "nsterm.m:384-391 parses right then left"
    );
}
