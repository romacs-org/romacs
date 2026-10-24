#[path = "../src/terminal_input.rs"]
mod terminal_input;

use romacs_display_runtime::backend::wgpu::{ROMACS_CTRL_MASK, ROMACS_META_MASK};
use romacs_display_runtime::host_types::FrontendKey;
use romacs_display_runtime::thread_comm::InputEvent;
use terminal_input::{Mode, Target, route};

#[test]
fn application_key_bytes_preserve_unicode_controls_and_cursor_modes() {
    assert_eq!(terminal_input::encode(&key('b', ROMACS_CTRL_MASK, true), false).unwrap(), vec![2], "TERMINAL_ENCODER_CTRL_BYTES");
    assert_eq!(terminal_input::encode(&key('λ', 0, true), false).unwrap(), "λ".as_bytes());
    let up = InputEvent::Key { key: FrontendKey::Keysym(0xff52), modifiers: 0, pressed: true, emacs_frame_id: 1 };
    assert_eq!(terminal_input::encode(&up, false).unwrap(), b"\x1b[A");
    assert_eq!(terminal_input::encode(&up, true).unwrap(), b"\x1bOA");
}

fn key(character: char, modifiers: u32, pressed: bool) -> InputEvent {
    InputEvent::Key {
        key: FrontendKey::Character(character),
        modifiers,
        pressed,
        emacs_frame_id: 1,
    }
}

#[test]
fn terminal_preserves_m_x_and_c_g_as_editor_commands() {
    for event in [key('x', ROMACS_META_MASK, true), key('g', ROMACS_CTRL_MASK, true), key('\u{7}', 0, true)] {
        assert_eq!(route(&event, Mode::Terminal, false), Target::Editor, "TERMINAL_EDITOR_M_X_C_G");
    }
}

#[test]
fn c_c_is_an_editor_prefix_not_an_immediate_terminal_interrupt() {
    assert_eq!(route(&key('c', ROMACS_CTRL_MASK, true), Mode::Terminal, false), Target::Editor, "TERMINAL_C_C_PREFIX");
    for suffix in ['k', 'c', 'g'] {
        assert_eq!(route(&key(suffix, ROMACS_CTRL_MASK, true), Mode::Terminal, true), Target::Editor, "TERMINAL_PREFIX_SUFFIX_STAYS_IN_EDITOR");
    }
    assert_eq!(route(&key('z', 0, true), Mode::Terminal, true), Target::Editor, "UNKNOWN_PREFIX_CANNOT_LEAK_BYTES");
}

#[test]
fn c_x_and_its_entire_prefix_sequence_remain_editor_owned() {
    for event in [key('x', ROMACS_CTRL_MASK, true), key('\u{18}', 0, true),
        InputEvent::Key { key: FrontendKey::Keysym(0x78), modifiers: ROMACS_CTRL_MASK, pressed: true, emacs_frame_id: 1 },
        InputEvent::Key { key: FrontendKey::Keysym(24), modifiers: 0, pressed: true, emacs_frame_id: 1 }] {
        assert_eq!(route(&event, Mode::Terminal, false), Target::Editor, "TERMINAL_C_X_MUST_STAY_IN_EDITOR");
    }
    for event in [key('o', 0, true), key('b', 0, true), key('f', ROMACS_CTRL_MASK, true), key('z', 0, true)] {
        assert_eq!(route(&event, Mode::Terminal, true), Target::Editor, "EVERY_C_X_SUFFIX_STAYS_WITH_EDITOR_DISPATCH");
    }
    assert_eq!(route(&key('x', ROMACS_CTRL_MASK, false), Mode::Terminal, false), Target::Ignore);
}

#[test]
fn emacs_mode_navigates_without_terminal_writes() {
    for event in [key('b', ROMACS_CTRL_MASK, true), key('p', ROMACS_CTRL_MASK, true), key('x', ROMACS_CTRL_MASK, true), key('s', ROMACS_CTRL_MASK, true), key('λ', 0, true)] {
        assert_eq!(route(&event, Mode::Emacs, false), Target::Editor, "EMACS_INPUT_IS_EDITOR_NAVIGATION");
    }
}

#[test]
fn terminal_mode_keeps_non_reserved_application_input() {
    for event in [key('b', ROMACS_CTRL_MASK, true), key('s', ROMACS_CTRL_MASK, true), key('q', ROMACS_CTRL_MASK, true), key('λ', 0, true)] {
        assert_eq!(route(&event, Mode::Terminal, false), Target::Terminal);
    }
    assert_eq!(route(&key('x', 0, false), Mode::Terminal, false), Target::Terminal, "TERMINAL_KEY_RELEASE_REMAINS_AVAILABLE_TO_ENCODER");
}

#[test]
fn reserved_releases_and_non_key_events_do_not_reach_child() {
    assert_eq!(route(&key('x', ROMACS_META_MASK, false), Mode::Terminal, false), Target::Ignore);
    assert_eq!(route(&key('g', ROMACS_CTRL_MASK, false), Mode::Terminal, false), Target::Ignore);
    assert_eq!(route(&InputEvent::WindowClose { emacs_frame_id: 1 }, Mode::Terminal, false), Target::Ignore);
}
