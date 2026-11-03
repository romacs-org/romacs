//! The terminal encoder, moved from the host's key router into the VM
//! (`romacs--term-encode-event'), sends the bytes the router sent: every
//! host key, as the host translates it into the Lisp event a command sees,
//! encodes as the router's encoder (frozen in `frozen/router_encode.rs')
//! encoded it, in both cursor modes.
#[path = "../src/native.rs"] mod native;
#[path = "frozen/router_encode.rs"] mod terminal_input;
use romacs_display_runtime::{backend::wgpu::*, host_types::FrontendKey, thread_comm::InputEvent};
use romacs_vm::{CommandEvent, Vm};

fn key(key: FrontendKey, modifiers: u32) -> InputEvent {
    InputEvent::Key { key, modifiers, pressed: true, emacs_frame_id: 0 }
}

/// The host keys checked: characters (ASCII, non-ASCII, the control
/// characters), the editing keys, arrows, navigation and function keys,
/// each under the modifier combinations a user can type.
fn keys() -> Vec<InputEvent> {
    let mut keys = Vec::new();
    let characters = ['a', 'z', 'A', 'Z', '0', ' ', '@', '?', '[', '\\', ']', '^', '_', '`', '~', '%', '/', 'λ', 'é', '中'];
    let modifiers = [0, ROMACS_CTRL_MASK, ROMACS_META_MASK, ROMACS_CTRL_MASK | ROMACS_META_MASK,
                     ROMACS_SHIFT_MASK, ROMACS_CTRL_MASK | ROMACS_SHIFT_MASK];
    for &c in &characters {
        for &m in &modifiers {
            keys.push(key(FrontendKey::Character(c), m));
        }
    }
    let keysyms = [0xff08, 0xff09, 0xff0d, 0xff1b, 0xff50, 0xff51, 0xff52, 0xff53, 0xff54, 0xff55, 0xff56, 0xff57,
                   0xff63, 0xffff, 0xffbe, 0xffbf, 0xffc0, 0xffc1, 0xffc2, 0xffc3, 0xffc4, 0xffc5, 0xffc6, 0xffc7, 0xffc8, 0xffc9];
    let modifiers = [0, ROMACS_SHIFT_MASK, ROMACS_CTRL_MASK, ROMACS_META_MASK, ROMACS_CTRL_MASK | ROMACS_SHIFT_MASK,
                     ROMACS_CTRL_MASK | ROMACS_META_MASK | ROMACS_SHIFT_MASK];
    for &k in &keysyms {
        for &m in &modifiers {
            keys.push(key(FrontendKey::Keysym(k), m));
        }
    }
    keys
}

/// The Lisp reader's form of EVENT, a command event.
fn lisp(event: &CommandEvent) -> String {
    match event {
        CommandEvent::Character(code) => code.to_string(),
        CommandEvent::Named(name) => format!("'{name}"),
    }
}

#[test]
fn every_host_key_encodes_as_the_router_encoded_it() {
    let mut vm = Vm::new();
    let mut failures = Vec::new();
    let mut checked = 0;
    for event in keys() {
        let Some(command) = native::translate(event.clone()) else { continue };
        for application_cursor in [false, true] {
            let old = terminal_input::encode(&event, application_cursor);
            let form = format!("(condition-case nil (append (romacs--term-encode-event {} {}) nil) (error 'error))",
                               lisp(&command), if application_cursor { "t" } else { "nil" });
            let new = vm.eval_source(&form).and_then(|v| v.display()).unwrap();
            let expected = match &old {
                Ok(bytes) if bytes.is_empty() => "nil".to_owned(),
                Ok(bytes) => format!("({})", bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")),
                Err(_) => "error".to_owned(),
            };
            checked += 1;
            if new != expected {
                failures.push(format!("{event:?} as {} (application cursor {application_cursor}): router {expected}, VM {new}", lisp(&command)));
            }
        }
    }
    assert!(checked > 200, "only {checked} keys checked");
    assert!(failures.is_empty(), "{} of {checked} differ:\n{}", failures.len(), failures.join("\n"));
}
