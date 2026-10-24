//! GNU keymap.c:2092–2368: descriptions of character and symbolic events.
use super::*;
use crate::lstring::char_string;

const ALT: i64 = 1 << 22;
const SUPER: i64 = 1 << 23;
const HYPER: i64 = 1 << 24;
const SHIFT: i64 = 1 << 25;
const CTRL: i64 = 1 << 26;
const META: i64 = 1 << 27;

fn character_description(ch: i64) -> Vec<u8> {
    let mut c = ch & ((META << 1) - 1);
    let base = c & !(ALT | SUPER | HYPER | SHIFT | CTRL | META);
    if base > 0x3fffff { return format!("[{c}]").into_bytes(); }
    let tab_as_ci = base == 9 && c & META != 0;
    let mut out = Vec::new();
    for (bit, label) in [(ALT, "A-"), (CTRL, "C-"), (HYPER, "H-"),
                         (META, "M-"), (SHIFT, "S-"), (SUPER, "s-")] {
        if c & bit != 0 || (bit == CTRL && ((base < 32 && !matches!(base, 9 | 13 | 27)) || tab_as_ci)) {
            out.extend_from_slice(label.as_bytes());
            c &= !bit;
        }
    }
    match c {
        27 => out.extend_from_slice(b"ESC"),
        9 if tab_as_ci => out.push(b'i'),
        9 => out.extend_from_slice(b"TAB"),
        13 => out.extend_from_slice(b"RET"),
        0..=31 => out.push((c + if c > 0 && c <= 26 { 96 } else { 64 }) as u8),
        127 => out.extend_from_slice(b"DEL"),
        32 => out.extend_from_slice(b"SPC"),
        _ => {
            let mut bytes = [0; 5];
            let len = char_string(c as u32, &mut bytes);
            out.extend_from_slice(&bytes[..len]);
        },
    }
    out
}

fn events(cx: &Leaf<'_>, sequence: Value) -> std::result::Result<Vec<Value>, Fault> {
    if sequence == Value::NIL { return Ok(Vec::new()); }
    if let Ok(view) = cx.string_view(sequence) {
        return Ok(view.chars().map(|c| {
            let c = i64::from(c);
            Value::integer(if !view.is_multibyte() && c & 128 != 0 { c ^ (128 | META) } else { c }).unwrap()
        }).collect());
    }
    if let Ok(values) = cx.vector_values(sequence) { return Ok(values); }
    if cx.pair(sequence).is_some() {
        let mut tail = sequence;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        while let Some((head, rest)) = cx.pair(tail) {
            if !seen.insert(tail.0) { return Err(Fault::Circular(tail)); }
            out.push(head);
            tail = rest;
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
        return Ok(out);
    }
    Err(Fault::Wrong("sequencep", sequence))
}

fn describe(cx: &Leaf<'_>, mut key: Value, no_angles: bool) -> std::result::Result<Vec<u8>, Fault> {
    if let Some((head, tail)) = cx.pair(key) {
        if head.integer_value().is_some() && tail.integer_value().is_some() {
            let mut out = describe(cx, head, no_angles)?;
            out.extend_from_slice(b"..");
            out.extend(describe(cx, tail, no_angles)?);
            return Ok(out);
        }
        // Lucid-style event lists contain modifier symbols and a final event.
        let modifier = |v| cx.symbol_name(v).ok().and_then(|s| match s.as_str() {
            "alt" => Some(ALT), "super" => Some(SUPER), "hyper" => Some(HYPER),
            "shift" => Some(SHIFT), "control" => Some(CTRL), "meta" => Some(META), _ => None,
        });
        if modifier(head).is_some() {
            let items = events(cx, key)?;
            let mut bits = 0;
            let mut base = None;
            for item in items {
                if let Some(bit) = modifier(item) { bits |= bit; } else { base = Some(item); }
            }
            if let Some(base) = base {
                let character = base.integer_value().or_else(|| {
                    let name = cx.symbol_name(base).ok()?;
                    let mut chars = name.chars();
                    let c = chars.next()?;
                    chars.next().is_none().then_some(i64::from(c as u32))
                });
                if let Some(mut c) = character {
                    if bits & SHIFT != 0 && (97..=122).contains(&c) { c -= 32; bits &= !SHIFT; }
                    if bits & CTRL != 0 && (64..=95).contains(&c) {
                        if (65..=90).contains(&c) { bits |= SHIFT; }
                        c &= 31; bits &= !CTRL;
                    } else if bits & CTRL != 0 && ((97..=122).contains(&c) || c < 32) {
                        c &= 31; bits &= !CTRL;
                    }
                    return Ok(character_description(c | bits));
                }
                let mut name = String::new();
                for (bit, label) in [(ALT,"A-"),(CTRL,"C-"),(HYPER,"H-"),(META,"M-"),(SHIFT,"S-"),(SUPER,"s-")] {
                    if bits & bit != 0 { name.push_str(label); }
                }
                name.push_str(&cx.symbol_name(base)?);
                return Ok(symbol_description(&name, no_angles));
            }
        }
        key = head;
    }
    if let Some(c) = key.integer_value() { return Ok(character_description(c)); }
    if let Ok(name) = cx.symbol_name(key) { return Ok(symbol_description(&name, no_angles)); }
    if let Ok(view) = cx.string_view(key) { return Ok(view.bytes); }
    Err(Fault::Error("KEY must be an integer, cons, symbol, or string".into()))
}

fn symbol_description(name: &str, no_angles: bool) -> Vec<u8> {
    if no_angles { return name.as_bytes().to_vec(); }
    let bytes = name.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() && bytes[i + 1] == b'-' && b"CMSsHA".contains(&bytes[i]) { i += 2; }
    let mut out = bytes[..i].to_vec();
    out.push(b'<'); out.extend_from_slice(&bytes[i..]); out.push(b'>'); out
}

defsubr! { name = "single-key-description", gnu = "keymap.c:2307", effect = Pure,
fn single_key_description(cx, key: Value; optional no_angles: Value) {
    let bytes = describe(cx, key, no_angles != Value::NIL)?;
    Ok(cx.make_string(bytes, true))
}}

defsubr! { name = "key-description", gnu = "keymap.c:2092", effect = Reads,
fn key_description(cx, keys: Value; optional prefix: Value) {
    let meta_symbol = cx.intern("meta-prefix-char");
    let meta_prefix = cx.variable_value(meta_symbol)?.unwrap_or(Value::integer(27).unwrap());
    let mut all = events(cx, prefix)?;
    all.extend(events(cx, keys)?);
    let mut parts = Vec::new();
    let mut add_meta = false;
    for mut key in all {
        if add_meta {
            if key == meta_prefix || key.integer_value().is_none_or(|c| c & META != 0) {
                parts.push(describe(cx, meta_prefix, false)?);
                if key == meta_prefix { continue; }
            } else { key = Value::integer(key.integer_value().unwrap() | META).unwrap(); }
            add_meta = false;
        } else if key == meta_prefix { add_meta = true; continue; }
        parts.push(describe(cx, key, false)?);
    }
    if add_meta { parts.push(describe(cx, meta_prefix, false)?); }
    let bytes = parts.join(&b' ');
    let multibyte = !bytes.is_empty();
    Ok(cx.make_string(bytes, multibyte))
}}

#[cfg(test)]
mod tests {
    use crate::{Value, Vm};
    #[test]
    fn key_description_gnu_oracle() {
        let cases = [
            "(key-description nil)", "(key-description [24 108])",
            "(key-description [97] [24])", "(key-description [97] [27])",
            "(key-description [27 27 97])", "(key-description [27 f1])",
            "(key-description [27 134217825])", "(key-description [27])",
            "(key-description [0 9 13 27 32 127 233])",
            "(key-description (unibyte-string 225))",
            "(key-description '(24 108))", "(single-key-description 'C-M-f1)",
            "(single-key-description 'C-M-f1 t)", "(single-key-description '(mouse-1 (1 2)))",
            "(single-key-description '(1 . 26))", "(single-key-description '(control meta a))",
            "(single-key-description '(shift 97))", "(single-key-description 134217737)",
            "(single-key-description 264241249)", "(single-key-description nil)",
            "(let ((meta-prefix-char 24)) (key-description [24 97]))",
            "(let ((meta-prefix-char nil)) (key-description [27 97]))",
            "(single-key-description '(meta shift control a))",
            "(single-key-description '(control 63))",
            "(condition-case e (key-description 42) (error e))",
            "(condition-case e (key-description '(1 . 2)) (error e))",
            "(condition-case e (single-key-description []) (error e))",
        ];
        let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("source romacs-keymap-env.sh");
        let mut vm = Vm::new();
        vm.eval_source("(defvar meta-prefix-char 27)").unwrap();
        for form in cases {
            let output = std::process::Command::new(&oracle).args(["--batch", "-Q", "--eval", &format!("(prin1 {form})")]).output().unwrap();
            assert!(output.status.success(), "{form}: {:?}", output);
            let expected = String::from_utf8(output.stdout).unwrap();
            let check = format!("(equal {form} '{expected})");
            assert_eq!(vm.eval_source_raw(&check).unwrap(), Value::TRUE, "{form}: {expected}");
        }
    }
}
