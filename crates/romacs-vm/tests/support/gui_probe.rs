//! A window-system session with GNU's Lisp on a 1216x876 frame of 10x25
//! cells, and the runner for a probe of forms pinned GNU X11 answered on
//! the same frame (child_frames_oracle.py --probe).
use romacs_vm::Vm;
use std::path::Path;
#[path = "gnu_ert_vm.rs"]
mod gnu_ert_vm;

/// A window system that answers nothing about fonts.
pub struct Pgtk;
impl romacs_vm::DisplayHost for Pgtk {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
}

/// A window-system frame of 1216x876 pixels and 10x25 cells, with GNU's Lisp.
pub fn gnu_session() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    vm.set_frame_geometry(1216, 876, 25, 10).unwrap();
    vm.set_display_host(Some(Box::new(Pgtk)));
    assert!(vm.open_display());
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    vm
}

fn forms(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(';'))
}

pub fn printed(vm: &mut Vm, form: &str) -> String {
    let source = format!("(prin1-to-string (condition-case e {form} (error (list 'signal e))))");
    // `display' prints the printed string as Lisp reads it.
    vm.eval_source(&source).and_then(|v| v.display())
        .map(|printed| printed[1..printed.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\"))
        .unwrap_or_else(|e| format!("eval failed: {e}"))
}

/// PROBE's forms through section SECTION (a `;; NAME' line),
/// evaluated in one session with a `probe-resize' form setting the host
/// geometry, each section's AFTER run after each of its forms; the
/// mismatches with pinned GNU's ANSWERS in that section.
pub fn probe(probe: &str, answers: &str, section: &str, after: impl Fn(&mut Vm, &str)) -> Vec<String> {
    assert_eq!(forms(probe).count(), forms(answers).count(), "the .gnu answers are stale: rerun child_frames_oracle.py --probe");
    let mut gnu = forms(answers);
    let mut vm = gnu_session();
    let (mut current, mut failures) = ("", vec![]);
    for line in probe.lines().filter(|line| !line.is_empty()) {
        if let Some(name) = line.strip_prefix(";; ").filter(|name| !name.contains(' ')) {
            if current == section { break; }
            current = name;
            continue;
        }
        if line.starts_with(';') { continue; }
        let actual = match line.strip_prefix("(probe-resize ").and_then(|rest| rest.strip_suffix(')')) {
            Some(size) => {
                let [w, h] = [0, 1].map(|i| size.split(' ').nth(i).unwrap().parse::<u32>().unwrap());
                vm.frame_snapshot(w, h).unwrap();
                printed(&mut vm, "(list (frame-native-width) (frame-native-height))")
            }
            None => printed(&mut vm, line),
        };
        let expected = gnu.next().unwrap();
        if current == section {
            if actual != expected { failures.push(format!("{line}\n  GNU    {expected}\n  Romacs {actual}")); }
            after(&mut vm, line);
        }
    }
    assert_eq!(current, section, "no section {section}");
    failures
}

