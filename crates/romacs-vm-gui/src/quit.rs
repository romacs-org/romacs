//! The render thread's half of GNU's out-of-band `C-g' (romacs_vm's
//! quit.rs): keyboard.c never queues the quit character typed while Lisp
//! runs, it sets `quit-flag' at once (kbd_buffer_store_buffered_event,
//! handle_interrupt).
use romacs_display_runtime::backend::wgpu::ROMACS_CTRL_MASK;
use romacs_display_runtime::host_types::FrontendKey;
use romacs_display_runtime::paint_host::PRIMARY_FRAME;
use romacs_display_runtime::thread_comm::{InputEvent, InputInterrupt};
use romacs_vm::{CommandEvent, QUIT_CHAR, QuitSignal};
use std::sync::Arc;

/// Takes the quit character out of band while the evaluator is busy.
pub struct QuitHook(pub Arc<QuitSignal>);

impl InputInterrupt for QuitHook {
    fn interrupt(&self, event: &InputEvent) -> bool {
        is_quit_char(event) && self.0.interrupt()
    }
    fn busy_for(&self) -> Option<std::time::Duration> {
        self.0.busy_for()
    }
}

/// Whether EVENT is the quit character as typed: its key with its control
/// folded in (make_ctrl_char) and no other modifier, before
/// `keyboard-translate-table' or any key map, as GNU compares it.
pub fn is_quit_char(event: &InputEvent) -> bool {
    matches!(event, InputEvent::Key { pressed: true, .. })
        && crate::native::translate(event.clone()) == Some(CommandEvent::Character(QUIT_CHAR))
}

/// The quit character as a key event: what read_char returns for a quit
/// that is pending when it reads.
pub fn quit_key() -> InputEvent {
    InputEvent::Key { key: FrontendKey::Character('g'), modifiers: ROMACS_CTRL_MASK, pressed: true, emacs_frame_id: PRIMARY_FRAME }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_display_runtime::backend::wgpu::{ROMACS_META_MASK, ROMACS_SHIFT_MASK};

    fn key(c: char, modifiers: u32) -> InputEvent {
        InputEvent::Key { key: FrontendKey::Character(c), modifiers, pressed: true, emacs_frame_id: PRIMARY_FRAME }
    }

    #[test]
    fn only_a_plain_control_g_is_the_quit_character() {
        assert!(is_quit_char(&quit_key()));
        assert!(!is_quit_char(&key('g', 0)));
        assert!(!is_quit_char(&key('g', ROMACS_CTRL_MASK | ROMACS_META_MASK)), "C-M-g is a key");
        assert!(!is_quit_char(&key('G', ROMACS_CTRL_MASK | ROMACS_SHIFT_MASK)), "C-S-g is a key");
        let released = InputEvent::Key { key: FrontendKey::Character('g'), modifiers: ROMACS_CTRL_MASK, pressed: false, emacs_frame_id: PRIMARY_FRAME };
        assert!(!is_quit_char(&released));
    }

    #[test]
    fn the_hook_takes_it_only_from_a_busy_evaluator() {
        let signal = Arc::new(QuitSignal::new());
        let hook = QuitHook(signal.clone());
        signal.set_busy(false);
        assert!(!hook.interrupt(&quit_key()), "waiting: queued as a key");
        signal.set_busy(true);
        assert!(!hook.interrupt(&key('x', 0)));
        assert!(hook.interrupt(&quit_key()));
        assert_eq!(signal.set_busy(false), 1);
    }
}
