//! A bold row beside regular ones measures and lays out as in pinned GNU
//! X11 (bold_metrics.el, its answers bold_metrics.gnu from
//! crates/romacs-vm/tests/child_frames_oracle.py): Iosevka's Bold has its
//! Regular's metrics, so a row's height is the frame's line and a
//! column's width the frame's character width whatever the weight, in
//! `line-pixel-height', `window-text-pixel-size', `buffer-text-pixel-size'
//! and `fit-window-to-buffer' alike, on the GUI's display with its layout.
use romacs_vm_gui::{fonts, measure, startup};
use romacs_vm::Vm;

fn forms(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(';'))
}

#[test]
fn bold_rows_measure_and_lay_out_as_gnu() {
    let expected: Vec<String> = forms(include_str!("bold_metrics.gnu")).map(String::from).collect();
    // The fonts pinned GNU saw: the gate's fontconfig, where the
    // environment names it (romacs-env.sh), as the oracle and popup_live use.
    if let Some(config) = std::env::var_os("ROMACS_GATE_FONTCONFIG") {
        // SAFETY: before any thread of this test process reads the environment.
        unsafe { std::env::set_var("FONTCONFIG_FILE", config) };
    }
    let actual = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout2::LayoutEngine::new().unwrap()));
        vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
        // The probe's font, and the frame's cell from it, as the GUI sets it
        // once startup Lisp chose the default face.
        vm.eval_source("(set-face-attribute 'default nil :family \"Iosevka\" :height 140)").unwrap();
        let (line, column) = romacs_layout2::frame_metrics(&vm.display_default_face().unwrap()).unwrap();
        vm.set_frame_geometry(1280, 800, line, column).unwrap();
        // The probe finds the tree's keymap-popup beside it, as in GNU.
        vm.eval_source(&format!("(setenv \"PROBE\" {:?})", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/bold_metrics.el"))).unwrap();
        let mut answers = vec![];
        for form in forms(include_str!("bold_metrics.el")) {
            // The GUI lays the frame out before each question, as its
            // redisplay does (GNU's `redisplay' in the probe).
            let snapshot = vm.frame_snapshot(1280, 800).unwrap();
            let (_, ends) = engine.borrow_mut().layout_frame(&snapshot, 1, None).unwrap();
            vm.display_apply_layout(&ends).unwrap();
            let answer = if form.starts_with("(setq ") {
                vm.eval_source(form).map(|_| "-".to_string())
            } else {
                vm.eval_source(form).and_then(|v| v.display())
            };
            answers.push(answer.unwrap_or_else(|e| format!("VM-ERROR {e}")));
        }
        answers
    }).unwrap().join().unwrap();
    assert_eq!(actual.len(), expected.len(), "bold_metrics.gnu is stale: rerun child_frames_oracle.py");
    for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
        println!("{i}: {a} | {e}");
    }
    assert_eq!(actual, expected);
}
