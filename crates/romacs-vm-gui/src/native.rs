//! Owned native transport -> GNU key event boundary.
use romacs_display_runtime::{host_types::FrontendKey, thread_comm::InputEvent};
use romacs_vm::CommandEvent;

pub fn translate(event: InputEvent) -> Option<CommandEvent> {
    use romacs_display_runtime::backend::wgpu::*;
    let (key, modifiers) = match event {
        InputEvent::Tracked { event, .. } | InputEvent::Observed { event, .. } => return translate(*event),
        InputEvent::Key { key, modifiers, pressed: true, .. } => (key, modifiers),
        _ => return None,
    };
    // Shift+Tab is the function key `backtab', as GNU's X and pgtk frames
    // deliver ISO_Left_Tab; modifiers other than Shift stay on it.
    if let FrontendKey::Keysym(0xff09 | 0xfe20) = key
        && modifiers & ROMACS_SHIFT_MASK != 0
    {
        let mut prefix = String::new();
        for (mask, label) in [(ROMACS_ALT_MASK, "A-"), (ROMACS_CTRL_MASK, "C-"), (ROMACS_HYPER_MASK, "H-"), (ROMACS_META_MASK, "M-"), (ROMACS_SUPER_MASK, "s-")] {
            if modifiers & mask != 0 { prefix.push_str(label); }
        }
        return Some(CommandEvent::Named(prefix + "backtab"));
    }
    let mut code = match key {
        FrontendKey::Character(c) => Some(c as i64),
        FrontendKey::Keysym(k) => match k {
            0xff08 if modifiers & ROMACS_CTRL_MASK != 0 => None,
            // Return is the function key `return', as GNU's X and pgtk
            // frames deliver it; `local-function-key-map' makes an unbound
            // one RET (simple.el).
            0xff08 => Some(127), 0xff09 => Some(9), 0xff1b => Some(27),
            0..=0x7f | 0xa0..=0xff => Some(k as i64),
            0x01000000..=0x0110ffff => char::from_u32(k & 0xffffff).map(|c| c as i64),
            _ => None,
        },
    };
    if let Some(ref mut c) = code {
        if modifiers & ROMACS_CTRL_MASK != 0 {
            *c = match *c { 0..=31 | 127 => *c, 32 => 0, 64..=95 | 97..=122 => *c & 31, 63 => 127, _ => *c | (1 << 26) };
        }
        for (mask, bit) in [(ROMACS_META_MASK,27),(ROMACS_SUPER_MASK,23),(ROMACS_ALT_MASK,22),(ROMACS_HYPER_MASK,24)] {
            if modifiers & mask != 0 { *c |= 1 << bit; }
        }
        // Shift is already reflected in native text. Preserve it for control chords.
        if modifiers & ROMACS_SHIFT_MASK != 0 && modifiers & ROMACS_CTRL_MASK != 0 { *c |= 1 << 25; }
        return Some(CommandEvent::Character(*c));
    }
    let FrontendKey::Keysym(k) = key else { return None; };
    let name = match k {
        0xff08 => "backspace".into(), 0xff0d => "return".into(),
        0xff50 => "home".into(), 0xff51 => "left".into(), 0xff52 => "up".into(),
        0xff53 => "right".into(), 0xff54 => "down".into(), 0xff55 => "prior".into(),
        0xff56 => "next".into(), 0xff57 => "end".into(), 0xffff => "delete".into(),
        0xff63 => "insert".into(), 0xffbe..=0xffe0 => format!("f{}", k - 0xffbe + 1),
        _ => return None,
    };
    let mut prefix=String::new();
    for (mask,label) in [(ROMACS_ALT_MASK,"A-"),(ROMACS_CTRL_MASK,"C-"),(ROMACS_HYPER_MASK,"H-"),(ROMACS_META_MASK,"M-"),(ROMACS_SHIFT_MASK,"S-"),(ROMACS_SUPER_MASK,"s-")] {
        if modifiers & mask != 0 { prefix.push_str(label); }
    }
    Some(CommandEvent::Named(prefix + &name))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: FrontendKey, modifiers: u32, pressed: bool) -> InputEvent {
        InputEvent::Key { key, modifiers, pressed, emacs_frame_id: 0 }
    }
    #[test]
    fn control_backspace_keeps_its_word_delete_identity() {
        use romacs_display_runtime::backend::wgpu::*;
        assert_eq!(translate(key(FrontendKey::Keysym(0xff08), ROMACS_CTRL_MASK, true)), Some(CommandEvent::Named("C-backspace".into())));
        assert_eq!(translate(key(FrontendKey::Keysym(0xff08), ROMACS_META_MASK, true)), Some(CommandEvent::Character((1 << 27) | 127)));
    }

    #[test]
    fn native_unicode_modifiers_named_keys_and_releases() {
        use romacs_display_runtime::backend::wgpu::*;
        assert_eq!(translate(key(FrontendKey::Character('Δ'), 0, true)), Some(CommandEvent::Character('Δ' as i64)));
        assert_eq!(translate(key(FrontendKey::Keysym(0x010003bb), 0, true)), Some(CommandEvent::Character('λ' as i64)));
        assert_eq!(translate(key(FrontendKey::Keysym(0x65), ROMACS_CTRL_MASK, true)), Some(CommandEvent::Character(5)));
        assert_eq!(translate(key(FrontendKey::Character('x'), ROMACS_META_MASK, true)), Some(CommandEvent::Character((1 << 27) | 120)));
        assert_eq!(translate(key(FrontendKey::Keysym(0xff51), ROMACS_CTRL_MASK, true)), Some(CommandEvent::Named("C-left".into())));
        assert_eq!(translate(key(FrontendKey::Keysym(0xff08), 0, true)), Some(CommandEvent::Character(127)));
        assert_eq!(translate(key(FrontendKey::Keysym(0xff0d), 0, true)), Some(CommandEvent::Named("return".into())));
        assert_eq!(translate(key(FrontendKey::Keysym(0xff0d), ROMACS_CTRL_MASK, true)), Some(CommandEvent::Named("C-return".into())));
        assert_eq!(translate(key(FrontendKey::Character('j'), ROMACS_CTRL_MASK, true)), Some(CommandEvent::Character(10)));
        assert_eq!(translate(key(FrontendKey::Keysym(0xffe1), 0, true)), None);
        assert_eq!(translate(key(FrontendKey::Character('x'), 0, false)), None);
    }
}
