//! A Lisp input event as the bytes a terminal sends for it: xterm's
//! encoding, as Alacritty's own input sends them (the encoder the GUI host
//! used before keys reached the command loop, moved here so Lisp commands
//! send keys: `romacs--term-encode-event', `romacs--term-send-event').

/// GNU's modifier bits on a character event (lisp.h CHAR_ALT ... CHAR_META).
const ALT: i64 = 0x0400000;
const SUPER: i64 = 0x0800000;
const HYPER: i64 = 0x1000000;
const SHIFT: i64 = 0x2000000;
const CONTROL: i64 = 0x4000000;
const META: i64 = 0x8000000;

/// What an event names: a character, or a function key by its symbol name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base<'a> {
    Char(char),
    Key(&'a str),
}

/// An event's base and its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyEvent<'a> {
    pub base: Base<'a>,
    pub shift: bool,
    pub control: bool,
    pub meta: bool,
    /// Super, hyper or alt: no terminal encoding.
    pub other: bool,
}

/// A character event: an integer, its modifier bits above the character.
pub(crate) fn character_event(code: i64) -> Option<KeyEvent<'static>> {
    let base = char::from_u32((code & 0x3fffff) as u32)?;
    Some(KeyEvent {
        base: Base::Char(base),
        shift: code & SHIFT != 0,
        control: code & CONTROL != 0,
        meta: code & META != 0,
        other: code & (ALT | SUPER | HYPER) != 0,
    })
}

/// A function-key event: a symbol NAME, its modifiers as GNU prefixes them
/// (`C-M-up', `S-f5').
pub(crate) fn symbol_event(name: &str) -> KeyEvent<'_> {
    let mut event = KeyEvent { base: Base::Key(name), shift: false, control: false, meta: false, other: false };
    let mut rest = name;
    while rest.len() > 2 && rest.as_bytes()[1] == b'-' {
        match rest.as_bytes()[0] {
            b'C' => event.control = true,
            b'M' => event.meta = true,
            b'S' => event.shift = true,
            b's' | b'H' | b'A' => event.other = true,
            _ => break,
        }
        rest = &rest[2..];
    }
    event.base = Base::Key(rest);
    event
}

/// The bytes EVENT sends; nothing for a key the terminal has no bytes for
/// (a lone modifier, mouse movement). APPLICATION_CURSOR is DECCKM.
pub(crate) fn encode(event: KeyEvent<'_>, application_cursor: bool) -> Result<Vec<u8>, String> {
    if event.other {
        return Err("Terminal encoding for Super/Hyper/Alt is not implemented".into());
    }
    let character = match event.base {
        Base::Char(c) => Some(c),
        Base::Key(name) => match name {
            "return" => Some('\r'),
            "tab" => Some('\t'),
            "backspace" => Some('\u{7f}'),
            "escape" => Some('\x1b'),
            "backtab" => return Ok(b"\x1b[Z".to_vec()),
            _ => None,
        },
    };
    if let Some(mut c) = character {
        if c == '\t' && event.shift {
            return Ok(b"\x1b[Z".to_vec());
        }
        if event.control {
            c = match c as u32 {
                0..=31 | 127 => c,
                32 | 64 => '\0',
                63 => '\u{7f}',
                65..=95 | 97..=122 => char::from_u32((c as u32) & 31).unwrap(),
                _ => return Err("Unsupported terminal control character".into()),
            };
        }
        let mut bytes = Vec::with_capacity(5);
        if event.meta {
            bytes.push(27);
        }
        let mut utf8 = [0; 4];
        bytes.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
        return Ok(bytes);
    }
    let Base::Key(name) = event.base else { unreachable!("a character is encoded above") };
    let modifier = 1 + u8::from(event.shift) + 2 * u8::from(event.meta) + 4 * u8::from(event.control);
    let final_byte = match name {
        "left" => Some('D'), "up" => Some('A'), "right" => Some('C'), "down" => Some('B'),
        "home" => Some('H'), "end" => Some('F'),
        "f1" => Some('P'), "f2" => Some('Q'), "f3" => Some('R'), "f4" => Some('S'),
        _ => None,
    };
    if let Some(final_byte) = final_byte {
        let function = matches!(name, "f1" | "f2" | "f3" | "f4");
        let sequence = if modifier != 1 {
            format!("\x1b[1;{modifier}{final_byte}")
        } else if application_cursor || function {
            format!("\x1bO{final_byte}")
        } else {
            format!("\x1b[{final_byte}")
        };
        return Ok(sequence.into_bytes());
    }
    let number = match name {
        "insert" => 2, "delete" | "deletechar" => 3, "prior" => 5, "next" => 6,
        "f5" => 15, "f6" => 17, "f7" => 18, "f8" => 19,
        "f9" => 20, "f10" => 21, "f11" => 23, "f12" => 24,
        _ => return Err(format!("Unsupported terminal key {name}")),
    };
    Ok(if modifier == 1 { format!("\x1b[{number}~") } else { format!("\x1b[{number};{modifier}~") }.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(code: i64, app: bool) -> Vec<u8> {
        encode(character_event(code).unwrap(), app).unwrap()
    }
    fn key(name: &str, app: bool) -> Vec<u8> {
        encode(symbol_event(name), app).unwrap()
    }

    #[test]
    fn characters_encode_as_the_host_encoder_did() {
        assert_eq!(chars('a' as i64, false), b"a");
        assert_eq!(chars('λ' as i64, false), "λ".as_bytes());
        assert_eq!(chars(1, false), b"\x01");
        assert_eq!(chars(META | 'x' as i64, false), b"\x1bx");
        assert_eq!(chars(CONTROL | META | 'a' as i64, false), b"\x1b\x01");
        assert!(encode(character_event(CONTROL | '%' as i64).unwrap(), false).is_err());
        assert!(encode(character_event(SUPER | 'a' as i64).unwrap(), false).is_err());
    }

    #[test]
    fn function_keys_encode_as_the_host_encoder_did() {
        assert_eq!(key("up", false), b"\x1b[A");
        assert_eq!(key("up", true), b"\x1bOA");
        assert_eq!(key("C-up", true), b"\x1b[1;5A");
        assert_eq!(key("C-M-S-left", false), b"\x1b[1;8D");
        assert_eq!(key("f1", false), b"\x1bOP");
        assert_eq!(key("f5", false), b"\x1b[15~");
        assert_eq!(key("S-f12", false), b"\x1b[24;2~");
        assert_eq!(key("prior", false), b"\x1b[5~");
        assert_eq!(key("backtab", false), b"\x1b[Z");
        assert_eq!(key("return", false), b"\r");
        assert_eq!(key("M-backspace", false), b"\x1b\x7f");
    }
}
