//! The window title, as xdisp.c gui_consider_frame_title sets it at each
//! redisplay: `frame-title-format' in the selected window's buffer, sent when
//! it changes.  A format whose dependencies are known is expanded only when
//! `Vm::frame_title_key' moves; any other at each redisplay, as GNU's.
use romacs_vm::Vm;

/// Forms with no string, so the source cache compiles each once.
const FORM: &str = "(string-to-multibyte (substring-no-properties (format-mode-line frame-title-format)))";
const STATIC: &str = "(romacs--title-format-static-p frame-title-format)";

#[derive(Default)]
pub struct Title {
    /// The format last read, and whether its dependencies are known.
    format: Option<(u64, bool)>,
    key: Option<u64>,
    text: Option<String>,
}

/// At a redisplay, painted or not.  A server without a window keeps the
/// title for its next.
pub fn consider(vm: &mut Vm, sender: &romacs_display_runtime::paint_host::PaintSender, host: &std::cell::RefCell<crate::prompt::Shared>) {
    let Ok((format, key)) = vm.frame_title_key() else { return };
    let known = host.borrow().title.format.filter(|(seen, _)| *seen == format).map(|(_, known)| known);
    let known = known.unwrap_or_else(|| {
        let known = vm.eval_source(STATIC).is_ok_and(|value| value != romacs_vm::Fixed::NIL);
        host.borrow_mut().title.format = Some((format, known));
        known
    });
    if known && host.borrow().title.key == Some(key) { return; }
    host.borrow_mut().title.key = Some(key);
    let Some(title) = title(vm) else { return };
    if host.borrow().title.text.as_ref() == Some(&title) { return; }
    host.borrow_mut().title.text = Some(title.clone());
    let shown = host.borrow().window_shown();
    if let Some(server) = host.borrow_mut().server.as_mut() { server.title = title.clone(); }
    if crate::diagnostics() { eprintln!("{}", serde_json::json!({"window_title": title})); }
    if shown { let _ = sender.set_title(title); }
}

/// The title's text.  One that does not read back as text (raw bytes)
/// leaves the window's as it is.
fn title(vm: &mut Vm) -> Option<String> {
    serde_json::from_str(&vm.eval_source(FORM).ok()?.display().ok()?).ok()
}
