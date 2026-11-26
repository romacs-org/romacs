//! GNU src/keyboard.c's record of this command's keys (`this_command_keys`,
//! `this_single_command_key_start`, `raw_keybuf`): the input stream's
//! (`crate::command_loop::Streams`), which the key reader fills.
// The stream's readers cannot fail, but a leaf primitive's body returns `Out`.
#![expect(clippy::unnecessary_wraps, reason = "a leaf primitive's body returns Out; reading the input stream cannot fail")]
use super::keyboard::{CHAR_META, event_array};
use crate::Value;
use crate::prim::{Fault, Leaf};

/// This command's keys, `this_command_keys[0 .. this_command_key_count]`.
fn command_keys(cx: &Leaf<'_>) -> Vec<Value> {
    cx.command_keys().to_vec()
}

defsubr! { name = "this-command-keys", gnu = "keyboard.c:12244", effect = Reads,
fn this_command_keys(cx) {
    let keys = command_keys(cx);
    Ok(event_array(cx, &keys))
}}

defsubr! { name = "this-command-keys-vector", gnu = "keyboard.c:12290", effect = Reads,
fn this_command_keys_vector(cx) {
    let keys = command_keys(cx);
    Ok(cx.vector(&keys))
}}

defsubr! { name = "this-single-command-keys", gnu = "keyboard.c:12302", effect = Reads,
fn this_single_command_keys(cx) {
    let keys = command_keys(cx);
    let start = cx.single_command_key_start();
    Ok(cx.vector(&keys[start..]))
}}

defsubr! { name = "this-single-command-raw-keys", gnu = "keyboard.c:12316", effect = Reads,
fn this_single_command_raw_keys(cx) {
    let keys = cx.raw_command_keys().to_vec();
    Ok(cx.vector(&keys))
}}

defsubr! { name = "clear-this-command-keys", gnu = "keyboard.c:12329", effect = Global,
fn clear_this_command_keys(cx; optional keep_record: Value) {
    cx.store_command_keys(crate::command_loop::Store::TruncateKeys(0));
    if keep_record == Value::NIL {
        cx.clear_recent_keys();
    }
    Ok(Value::NIL)
}}

defsubr! { name = "set--this-command-keys", gnu = "keyboard.c:12257", effect = Global,
fn set_this_command_keys(cx, keys: Value) {
    // Internal use (simple.el execute-extended-command): KEYS's characters
    // become this command's keys, a leading \370 spelled M-x.
    let text = cx.string_view(keys).map_err(|_| Fault::Wrong("stringp", keys))?;
    let mut events: Vec<Value> = text.chars().enumerate().map(|(i, c)| {
        let c = c as i64;
        // CHAR_BYTE8_P: a raw byte is the byte itself.
        let c = if (0x3fff80..=0x3fffff).contains(&c) { c - 0x3fff00 } else { c };
        Value::integer(if i == 0 && c == 248 { 'x' as i64 | CHAR_META } else { c }).unwrap()
    }).collect();
    if events.is_empty() {
        // GNU reads the first character unconditionally: an empty string's
        // terminating NUL.
        events.push(Value::integer(0).unwrap());
    }
    cx.store_command_keys(crate::command_loop::Store::Keys(&events, 0));
    Ok(Value::NIL)
}}

// This command's keys made KEYS, a vector, as minibuf.c:1156 restores the
// caller's after a read; with START, the single command's keys start there,
// as command_loop_1 clears them for the next command.
defsubr! { name = "romacs--set-command-keys", gnu = "minibuf.c:1156", effect = Global,
fn set_command_keys(cx, keys: Value; optional start: Value) {
    let events = cx.vector_values(keys)?;
    let start = match start.integer_value() {
        Some(n) => usize::try_from(n).map_err(|_| Fault::Wrong("natnump", start))?,
        None => cx.single_command_key_start(),
    };
    cx.store_command_keys(crate::command_loop::Store::Keys(&events, start));
    Ok(Value::NIL)
}}
