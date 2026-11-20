//! A primitive Romacs adds (`gnu = "romacs:..."') answers `documentation'
//! with the comment above its defsubr! (build.rs collects it); a GNU
//! primitive still answers from DOC (romacs#340).
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    let printed = vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| panic!("{source}: {e}"));
    serde_json::from_str(&printed).unwrap_or(printed)
}

/// FUNCTION's raw documentation, as simple.el's `function-documentation'
/// gives a primitive's: through `internal-subr-documentation'.
fn documentation(vm: &mut Vm, function: &str) -> String {
    eval(vm, &format!("(progn (defalias 'function-documentation #'internal-subr-documentation) (documentation '{function} t))"))
}

#[test]
fn a_romacs_primitive_is_documented_by_the_comment_above_its_defsubr() {
    let mut vm = Vm::new();
    // The comment's lines, each less its `// ', joined by newlines.
    assert_eq!(documentation(&mut vm, "romacs--pointer"),
        "Where the pointer is over the host frame, in pixels: (X . Y), as GNU's\n\
         `mouse_position_hook' finds it; nil with no pointer over the frame, or no display.");
    for (function, start) in [
        ("romacs-syntax-summaries-stats", "Counters of the current buffer's syntax summaries"),
        ("romacs-json-read", "Parse the JSON text SOURCE"),
        ("freeze", "Freeze OBJECT"),
        // Two adjacent primitives each keep their own block.
        ("romacs--native-window", "The host frame's native window"),
        ("romacs-startup-progress", "What startup is doing"),
    ] {
        let doc = documentation(&mut vm, function);
        assert!(doc.starts_with(start), "{function}: {doc}");
    }
    // The text is a string from the table, not a position into DOC.
    assert_eq!(eval(&mut vm, "(stringp (internal-subr-documentation (symbol-function 'romacs-json-read)))"), "t");
}

#[test]
fn a_gnu_primitive_still_answers_from_doc() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(numberp (internal-subr-documentation (symbol-function 'car)))"), "t");
    assert!(documentation(&mut vm, "car").starts_with("Return the car of LIST."));
}
