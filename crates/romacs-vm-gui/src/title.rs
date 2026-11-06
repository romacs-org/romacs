//! The window title, as xdisp.c gui_consider_frame_title sets it after a
//! redisplay: `frame-title-format' in the selected window's buffer, worked
//! out again only when `Vm::frame_title_key' moves.
use romacs_vm::Vm;

/// A form with no string, so the source cache compiles it once.
const FORM: &str = "(string-to-multibyte (substring-no-properties (format-mode-line frame-title-format)))";

/// After a paint.  A server without a window keeps the title for its next.
pub fn consider(vm: &mut Vm, sender: &romacs_display_runtime::paint_host::PaintSender, host: &std::cell::RefCell<crate::prompt::Shared>) {
    let Ok(key) = vm.frame_title_key() else { return };
    if host.borrow().title_key == Some(key) { return; }
    host.borrow_mut().title_key = Some(key);
    let Some(title) = title(vm) else { return };
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
