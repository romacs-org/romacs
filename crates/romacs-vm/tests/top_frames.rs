//! Top-level frames against pinned GNU: making, selecting, cycling,
//! hiding and deleting them (top_frames.el), evaluated form by form in one
//! session over a host that shows more than one top-level frame, and
//! compared, as printed, with pinned GNU X11's answers (top_frames.gnu,
//! child_frames_oracle.py --probe).
use romacs_vm::{HostEvent, HostFrame, HostInput, Vm};
#[path = "support/gui_probe.rs"]
mod gui_probe;

/// A host whose display shows any number of top-level frames, and which
/// is never asked for input.
struct Display;

impl HostInput for Display {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> { Err("no input".into()) }
    fn top_level_frames(&mut self) -> bool { true }
}

fn forms(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(';'))
}

fn session(host: bool) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_display_host(Some(Box::new(gui_probe::Pgtk)));
    assert!(vm.open_display());
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    if host { vm.set_host_input(Some(Box::new(Display))); }
    vm
}

#[test]
fn top_level_frames_match_pinned_gnu() {
    let probe = include_str!("top_frames.el");
    let expected: Vec<&str> = forms(include_str!("top_frames.gnu")).collect();
    let mut vm = session(true);
    let actual: Vec<String> = forms(probe).map(|form| {
        if form.starts_with("(setq ") {
            vm.eval_source(form).map(|_| "-".to_owned()).unwrap_or_else(|e| format!("setup failed: {e}"))
        } else {
            let source = format!("(prin1-to-string (condition-case e {form} (error (list 'signal e))))");
            vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
                printed[1..printed.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
            }).unwrap_or_else(|e| format!("eval failed: {e}"))
        }
    }).collect();
    assert_eq!(actual.len(), expected.len(), "top_frames.gnu is stale: rerun child_frames_oracle.py --probe");
    let failures: Vec<String> = forms(probe).zip(&expected).zip(&actual).filter(|((_, gnu), romacs)| gnu != romacs)
        .map(|((form, gnu), romacs)| format!("{form}\n  GNU    {gnu}\n  Romacs {romacs}")).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A host showing one top-level frame keeps it the only one: `make-frame'
/// without a `parent-frame' is refused, as before a second window exists.
#[test]
fn a_host_with_one_window_refuses_a_second_top_level_frame() {
    let mut vm = session(false);
    let made = vm.eval_source("(condition-case e (romacs--create-frame '((name . \"two\"))) (error (car (cdr e))))").unwrap();
    assert_eq!(made.display().unwrap(), "\"This display shows one top-level frame; a new frame needs a `parent-frame' parameter\"");
    assert_eq!(vm.eval_source("(length (frame-list))").unwrap().display().unwrap(), "1");
}

/// Each top-level frame lays its windows out in its own size: making and
/// sizing a second frame leaves the first frame's windows where they were.
#[test]
fn a_second_frame_leaves_the_first_frames_windows_alone() {
    let mut vm = session(true);
    let edges = "(mapcar (lambda (w) (list (window-pixel-left w) (window-pixel-top w) (window-pixel-width w) (window-pixel-height w))) (window-list F t))";
    vm.eval_source("(setq F (selected-frame))").unwrap();
    let before = vm.eval_source(edges).unwrap().display().unwrap();
    vm.eval_source("(setq G (romacs--create-frame '((width . 30) (height . 8))))").unwrap();
    assert_eq!(vm.eval_source(edges).unwrap().display().unwrap(), before);
    assert_eq!(vm.eval_source("(list (frame-text-cols G) (frame-text-lines G) (frame-text-cols F) (frame-text-lines F))").unwrap().display().unwrap(),
               "(30 8 78 30)");
    vm.eval_source("(delete-frame G)").unwrap();
    assert_eq!(vm.eval_source(edges).unwrap().display().unwrap(), before);
}

