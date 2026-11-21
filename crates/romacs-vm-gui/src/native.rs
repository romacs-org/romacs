//! Owned native transport -> GNU key event boundary.
//!
//! A key reaches Lisp as GNU's X and pgtk frames deliver it (keyboard.c
//! `make_lispy_event'), but for the keys a terminal could only send as
//! another: Ctrl+m, Ctrl+i and Ctrl+[ are the symbol events `C-m', `C-i'
//! and `C-[', not RET, TAB and ESC (ledger/divergences/VM2-GUI-KEYS).
//! lisp/romacs-editing.el makes an unbound one the character a terminal
//! sent, through `function-key-map', as simple.el makes `return' RET.
use romacs_display_runtime::backend::wgpu::{
    ROMACS_ALT_MASK as ALT, ROMACS_CTRL_MASK as CTRL, ROMACS_HYPER_MASK as HYPER, ROMACS_META_MASK as META,
    ROMACS_SHIFT_MASK as SHIFT, ROMACS_SUPER_MASK as SUPER,
};
use romacs_display_runtime::{host_types::FrontendKey, thread_comm::InputEvent};
use romacs_vm::CommandEvent;

/// GNU's modifier bits on a character event (lisp.h CHAR_ALT ... CHAR_META).
const CHAR_SHIFT: i64 = 1 << 25;
const CHAR_CTRL: i64 = 1 << 26;
const CHAR_BITS: [(u32, i64); 4] = [(ALT, 1 << 22), (SUPER, 1 << 23), (HYPER, 1 << 24), (META, 1 << 27)];

/// The characters whose Ctrl chord is its own key, a symbol event, where a
/// terminal sent the control character TAB, RET or ESC.
const GUI_CONTROL_KEYS: [char; 3] = ['i', 'm', '['];

pub fn translate(event: InputEvent) -> Option<CommandEvent> {
    let (key, modifiers) = match event {
        InputEvent::Tracked { event, .. } => return translate(*event),
        InputEvent::Key { key, modifiers, pressed: true, .. } => (key, modifiers),
        _ => return None,
    };
    match key {
        FrontendKey::Character(c) => character(c, modifiers),
        FrontendKey::Keysym(k) => keysym(k, modifiers),
    }
}

/// A keysym: an ASCII or Unicode one is its character, the rest function
/// keys, as GNU's X and pgtk frames name them.
fn keysym(k: u32, modifiers: u32) -> Option<CommandEvent> {
    let name = match k {
        0..=0x7f | 0xa0..=0xff => return character(char::from_u32(k)?, modifiers),
        0x01000000..=0x0110ffff => return character(char::from_u32(k & 0xffffff)?, modifiers),
        // Shift+Tab is the function key `backtab', as ISO_Left_Tab is to
        // GNU; modifiers other than Shift stay on it.
        0xff09 | 0xfe20 if modifiers & SHIFT != 0 => return named(modifiers & !SHIFT, "backtab"),
        // Backspace is DEL, but with Ctrl `C-backspace', a word's delete.
        0xff08 if modifiers & CTRL == 0 => return Some(CommandEvent::Character(with_bits(127, modifiers))),
        0xff08 => "backspace",
        // Tab, Return and Escape are the function keys `tab', `return' and
        // `escape'; `function-key-map' makes an unbound one TAB, RET or ESC
        // (simple.el).
        0xff09 => "tab", 0xff0d => "return", 0xff1b => "escape",
        0xff50 => "home", 0xff51 => "left", 0xff52 => "up", 0xff53 => "right", 0xff54 => "down",
        0xff55 => "prior", 0xff56 => "next", 0xff57 => "end", 0xffff => "delete", 0xff63 => "insert",
        0xffbe..=0xffe0 => return named(modifiers, &format!("f{}", k - 0xffbe + 1)),
        _ => return None,
    };
    named(modifiers, name)
}

/// A character typed with MODIFIERS, as keyboard.c `make_lispy_event'
/// makes it: Shift is already in the character, and with Ctrl a letter's
/// case follows Shift whatever caps lock says.
fn character(c: char, modifiers: u32) -> Option<CommandEvent> {
    if modifiers & CTRL != 0 {
        let c = if modifiers & SHIFT != 0 { single(c.to_uppercase()).unwrap_or(c) } else { single(c.to_lowercase()).unwrap_or(c) };
        let base = c.to_ascii_lowercase();
        if GUI_CONTROL_KEYS.contains(&base) {
            let shift = if c.is_ascii_uppercase() { SHIFT } else { 0 };
            return named((modifiers & !SHIFT) | shift, base.encode_utf8(&mut [0; 4]));
        }
        return Some(CommandEvent::Character(with_bits(make_ctrl_char(i64::from(u32::from(c))), modifiers)));
    }
    Some(CommandEvent::Character(with_bits(i64::from(u32::from(c)), modifiers)))
}

fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// keyboard.c `make_ctrl_char': the control character Ctrl makes of C, or C
/// with the control bit where no ASCII code says it (C-SPC, C-?, C-%).
fn make_ctrl_char(c: i64) -> i64 {
    match c {
        0x40..=0x5f => (c & 0x1f) | if (0x41..=0x5a).contains(&c) { CHAR_SHIFT } else { 0 },
        0x61..=0x7a => c & 0x1f,
        0..=0x1f => c,
        _ => c | CHAR_CTRL,
    }
}

/// C with the modifier bits of MODIFIERS but Control and Shift, which the
/// character already says.
fn with_bits(c: i64, modifiers: u32) -> i64 {
    CHAR_BITS.iter().filter(|(mask, _)| modifiers & mask != 0).fold(c, |c, (_, bit)| c | bit)
}

/// The symbol event NAME with MODIFIERS, prefixed in GNU's order.
fn named(modifiers: u32, name: &str) -> Option<CommandEvent> {
    let prefix: String = [(ALT, "A-"), (CTRL, "C-"), (HYPER, "H-"), (META, "M-"), (SHIFT, "S-"), (SUPER, "s-")]
        .into_iter().filter(|(mask, _)| modifiers & mask != 0).map(|(_, label)| label).collect();
    Some(CommandEvent::Named(prefix + name))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: FrontendKey, modifiers: u32, pressed: bool) -> InputEvent {
        InputEvent::Key { key, modifiers, pressed, emacs_frame_id: 0 }
    }
    fn chr(c: char, modifiers: u32) -> Option<CommandEvent> { translate(key(FrontendKey::Character(c), modifiers, true)) }
    fn sym(k: u32, modifiers: u32) -> Option<CommandEvent> { translate(key(FrontendKey::Keysym(k), modifiers, true)) }
    fn named(name: &str) -> Option<CommandEvent> { Some(CommandEvent::Named(name.into())) }
    fn code(c: i64) -> Option<CommandEvent> { Some(CommandEvent::Character(c)) }

    #[test]
    fn control_backspace_keeps_its_word_delete_identity() {
        assert_eq!(sym(0xff08, CTRL), named("C-backspace"));
        assert_eq!(sym(0xff08, META), code((1 << 27) | 127));
    }

    #[test]
    fn native_unicode_modifiers_named_keys_and_releases() {
        assert_eq!(chr('Δ', 0), code('Δ' as i64));
        assert_eq!(sym(0x010003bb, 0), code('λ' as i64));
        assert_eq!(sym(0x65, CTRL), code(5));
        assert_eq!(chr('x', META), code((1 << 27) | 120));
        assert_eq!(sym(0xff51, CTRL), named("C-left"));
        assert_eq!(sym(0xff08, 0), code(127));
        assert_eq!(sym(0xff0d, 0), named("return"));
        assert_eq!(sym(0xff0d, CTRL), named("C-return"));
        assert_eq!(chr('j', CTRL), code(10));
        assert_eq!(sym(0xffe1, 0), None);
        assert_eq!(translate(key(FrontendKey::Character('x'), 0, false)), None);
    }

    /// Ctrl+m, Ctrl+i and Ctrl+[ are their own keys, not RET, TAB and ESC,
    /// with every other modifier in GNU's order; Shift shows as S- only on
    /// the letters, as make_ctrl_char gives it.
    #[test]
    fn control_m_i_and_bracket_are_their_own_keys() {
        assert_eq!(chr('m', CTRL), named("C-m"));
        assert_eq!(chr('i', CTRL), named("C-i"));
        assert_eq!(chr('[', CTRL), named("C-["));
        assert_eq!(sym(0x6d, CTRL), named("C-m"));
        assert_eq!(chr('M', CTRL | SHIFT), named("C-S-m"));
        assert_eq!(chr('m', CTRL | META), named("C-M-m"));
        assert_eq!(chr('I', CTRL | META | SHIFT), named("C-M-S-i"));
        assert_eq!(chr('m', CTRL | SUPER | ALT | HYPER), named("A-C-H-s-m"));
        assert_eq!(chr('[', CTRL | META), named("C-M-["));
        // Caps lock: an upper-case letter without Shift is the letter.
        assert_eq!(chr('M', CTRL), named("C-m"));
        // The other control characters stay as GNU's GUI makes them.
        assert_eq!(chr('h', CTRL), code(8));
        assert_eq!(chr('g', CTRL), code(7));
        assert_eq!(chr(']', CTRL), code(29));
        assert_eq!(chr('@', CTRL), code(0));
        // Return, Tab and Escape are the function keys.
        assert_eq!(sym(0xff0d, 0), named("return"));
        assert_eq!(sym(0xff1b, 0), named("escape"));
    }

    /// The Tab key is the function key `tab', with its modifiers; Shift+Tab
    /// is `backtab'.
    #[test]
    fn tab_is_the_tab_function_key() {
        assert_eq!(sym(0xff09, 0), named("tab"));
        assert_eq!(sym(0xff09, CTRL), named("C-tab"));
        assert_eq!(sym(0xff09, META), named("M-tab"));
        assert_eq!(sym(0xff09, SHIFT), named("backtab"));
        assert_eq!(sym(0xfe20, SHIFT | CTRL), named("C-backtab"));
    }

    /// Ctrl+Space and Ctrl+? are C-SPC and C-?, as GNU's GUI frames make
    /// them (make_ctrl_char), not the NUL and DEL a terminal sends; Shift
    /// stays only on a letter's control character.
    #[test]
    fn control_space_and_question_mark_keep_their_keys() {
        assert_eq!(chr(' ', CTRL), code((1 << 26) | 32));
        assert_eq!(chr('?', CTRL | SHIFT), code((1 << 26) | 63));
        assert_eq!(chr(' ', CTRL | META), code((1 << 27) | (1 << 26) | 32));
        assert_eq!(chr('%', CTRL | SHIFT), code((1 << 26) | 37));
        assert_eq!(chr('_', CTRL | SHIFT), code(31));
        assert_eq!(chr('A', CTRL | SHIFT), code((1 << 25) | 1));
        assert_eq!(chr('λ', CTRL), code((1 << 26) | 'λ' as i64));
    }
}
