//! The GUI host's terminal encoder, as it was before keys reached the
//! command loop: kept unchanged as the reference the VM encoder
//! (`romacs--term-encode-event') is checked against. Not built into the GUI.
use romacs_display_runtime::thread_comm::InputEvent;

pub fn encode(event: &InputEvent, application_cursor: bool) -> Result<Vec<u8>, String> {
    use romacs_display_runtime::backend::wgpu::*;
    use romacs_display_runtime::host_types::FrontendKey;
    let (key, modifiers) = match event {
        InputEvent::Tracked { event, .. } | InputEvent::Observed { event, .. } => return encode(event, application_cursor),
        InputEvent::Key { key, modifiers, pressed: true, .. } => (key, *modifiers),
        _ => return Ok(Vec::new()),
    };
    let alt = modifiers & (ROMACS_ALT_MASK | ROMACS_META_MASK) != 0;
    let control = modifiers & ROMACS_CTRL_MASK != 0;
    let shift = modifiers & ROMACS_SHIFT_MASK != 0;
    if modifiers & (ROMACS_SUPER_MASK | ROMACS_HYPER_MASK) != 0 {
        return Err("Terminal encoding for Super/Hyper is not implemented".into());
    }
    let mut character = match key {
        FrontendKey::Character(c) => Some(*c),
        FrontendKey::Keysym(k) => match *k {
            0..=0xff => char::from_u32(*k),
            0x01000000..=0x0110ffff => char::from_u32(k & 0xffffff),
            0xff08 => Some('\u{7f}'), 0xff09 => Some('\t'), 0xff0d => Some('\r'), 0xff1b => Some('\x1b'),
            _ => None,
        },
    };
    if let Some(mut c) = character.take() {
        if c == '\n' { c = '\r'; }
        if c == '\t' && shift { return Ok(b"\x1b[Z".to_vec()); }
        if control {
            c = match c as u32 {
                0..=31 | 127 => c,
                32 | 64 => '\0', 63 => '\u{7f}',
                65..=95 | 97..=122 => char::from_u32((c as u32) & 31).unwrap(),
                _ => return Err("Unsupported terminal control character".into()),
            };
        }
        let mut bytes = Vec::with_capacity(5);
        if alt { bytes.push(27); }
        let mut utf8 = [0; 4];
        bytes.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
        return Ok(bytes);
    }
    let FrontendKey::Keysym(key) = key else { return Ok(Vec::new()); };
    if (0xffe1..=0xffee).contains(key) { return Ok(Vec::new()); }
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let final_byte = match *key {
        0xff51 => Some('D'), 0xff52 => Some('A'), 0xff53 => Some('C'), 0xff54 => Some('B'),
        0xff50 => Some('H'), 0xff57 => Some('F'), 0xffbe..=0xffc1 => char::from_u32('P' as u32 + key - 0xffbe),
        _ => None,
    };
    if let Some(final_byte) = final_byte {
        let sequence = if modifier != 1 { format!("\x1b[1;{modifier}{final_byte}") }
            else if application_cursor || (0xffbe..=0xffc1).contains(key) { format!("\x1bO{final_byte}") }
            else { format!("\x1b[{final_byte}") };
        return Ok(sequence.into_bytes());
    }
    let number = match *key {
        0xff63 => 2, 0xffff => 3, 0xff55 => 5, 0xff56 => 6,
        0xffc2 => 15, 0xffc3 => 17, 0xffc4 => 18, 0xffc5 => 19,
        0xffc6 => 20, 0xffc7 => 21, 0xffc8 => 23, 0xffc9 => 24,
        _ => return Err(format!("Unsupported terminal keysym {key:#x}")),
    };
    Ok(if modifier == 1 { format!("\x1b[{number}~") } else { format!("\x1b[{number};{modifier}~") }.into_bytes())
}
