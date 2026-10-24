//! Exact printer witnesses recorded from GNU 45b0f769. No class-only pins.
use romacs_vm::Vm;
use sha2::{Digest, Sha256};

fn cases() -> Vec<(&'static str, String)> {
    let fresh = format!("#^[nil nil test nil {}]", vec!["nil"; 64].join(" "));
    let initialized = format!("#^[fallback nil test init {}]", vec!["init"; 64].join(" "));
    let extras = format!("#^[init nil test init {} init changed]", vec!["init"; 64].join(" "));
    let cycle = format!("#1=#^[nil nil test nil {} #1#]", vec!["nil"; 64].join(" "));
    vec![
        ("(prin1-to-string (obarray-make 0))", "#<obarray n=0>".into()),
        ("(prin1-to-string (obarray-make 1) t)", "#<obarray n=0>".into()),
        ("(prin1-to-string (propertize (unibyte-string 255 97) 'p t))", "#(\"\\377a\" 0 2 (p t))".into()),
        ("(let ((o (obarray-make 4))) (intern \"a\" o) (intern \"b\" o) (unintern \"a\" o) (prin1-to-string o))", "#<obarray n=1>".into()),
        ("(let ((print-circle t) (o (obarray-make 1))) (prin1-to-string (list o o)))", "(#<obarray n=0> #<obarray n=0>)".into()),
        ("(prin1-to-string (propertize \"abc\" 'face 'bold 'help-echo \"help\"))", "#(\"abc\" 0 3 (face bold help-echo \"help\"))".into()),
        ("(let ((s (copy-sequence \"αβc\"))) (put-text-property 1 2 'face 'bold s) (prin1-to-string s))", "#(\"αβc\" 1 2 (face bold))".into()),
        ("(let ((s (copy-sequence \"αβc\"))) (put-text-property 1 2 'face 'bold s) (prin1-to-string s t))", "αβc".into()),
        ("(let ((s (copy-sequence \"abcd\"))) (put-text-property 0 2 'a 1 s) (put-text-property 1 3 'b nil s) (prin1-to-string s))", "#(\"abcd\" 0 1 (a 1) 1 2 (b nil a 1) 2 3 (b nil))".into()),
        ("(let ((s (copy-sequence \"abc\")) (print-circle t)) (put-text-property 0 1 'self s s) (prin1-to-string s))", "#1=#(\"abc\" 0 1 (self #1#))".into()),
        ("(let* ((s (copy-sequence \"λx\")) (v (vector s)) (print-circle t)) (put-text-property 1 2 'back v s) (prin1-to-string v))", "#1=[#(\"λx\" 1 2 (back #1#))]".into()),
        ("(prin1-to-string (condition-case e (list 'ok (reverse (make-char-table 'test))) (error (list 'sig e))))", format!("(sig (wrong-type-argument sequencep {fresh}))")),
        ("(prin1-to-string (condition-case e (list 'ok (vconcat (make-char-table 'test))) (error (list 'sig e))))", format!("(sig (wrong-type-argument sequencep {fresh}))")),
        ("(let ((print-length 1)) (prin1-to-string (propertize \"abc\" 'a 1 'b 2)))", "#(\"abc\" 0 3 (a ...))".into()),
        ("(let ((print-level 1)) (prin1-to-string (propertize \"abc\" 'a 1 'b 2)))", "#(\"abc\" 0 3 ...)".into()),
        ("(let ((print-level 2)) (prin1-to-string (propertize \"abc\" 'a 1 'b 2)))", "#(\"abc\" 0 3 (a 1 b 2))".into()),
        ("(prin1-to-string (make-char-table 'test))", fresh.clone()),
        ("(let ((print-level 0)) (prin1-to-string (make-char-table 'test)))", fresh),
        ("(let ((ct (make-char-table 'test 'init))) (set-char-table-range ct nil 'fallback) (prin1-to-string ct))", initialized),
        ("(progn (put 'test 'char-table-extra-slots 2) (let ((ct (make-char-table 'test 'init))) (set-char-table-extra-slot ct 1 'changed) (prin1-to-string ct)))", extras),
        ("(let ((print-length 0)) (prin1-to-string (make-char-table 'test)))", "#^[...]".into()),
        ("(let ((print-length 4)) (prin1-to-string (make-char-table 'test)))", "#^[nil nil test nil ...]".into()),
        ("(progn (put 'test 'char-table-extra-slots 1) (let ((ct (make-char-table 'test)) (print-circle t)) (set-char-table-extra-slot ct 0 ct) (prin1-to-string ct)))", cycle),
    ]
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

#[test]
fn exact_printer_restoration() {
    for (source, expected) in cases() {
        let mut vm = Vm::new();
        let assertion = format!("(equal {source} {})", quote(&expected));
        assert_eq!(vm.eval_source(&assertion).map(|v| v.display().unwrap()), Ok("t".into()), "{source}: expected {expected}");
    }
}

#[test]
fn pinned_live_gnu_printer_restoration() {
    let Ok(binary) = std::env::var("ROMACS_GNU_ORACLE") else { return; };
    if !std::path::Path::new(&binary).exists() { return; }
    let bytes = std::fs::read(&binary).unwrap();
    let hash: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8", "live test requires the pinned GNU binary");
    for (source, expected) in cases() {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "{source}");
        eprintln!("GNU exact: {source}");
    }
}

#[test]
fn property_cycles_without_circle_are_bounded_in_child() {
    if std::env::var_os("ROMACS_PROPERTY_CYCLE_CHILD").is_some() {
        let mut vm = Vm::new();
        let result = vm.eval_source("(let ((s (copy-sequence \"abc\"))) (put-text-property 0 1 'self s s) (prin1-to-string s))");
        assert!(result.is_err(), "cycle must signal rather than overflow the native stack");
        assert_eq!(vm.eval_source("(prin1-to-string 'ok)").unwrap().display().unwrap(), "\"ok\"");
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "property_cycles_without_circle_are_bounded_in_child", "--nocapture"])
        .env("ROMACS_PROPERTY_CYCLE_CHILD", "1").status().unwrap();
    assert!(status.success(), "property cycle child: {status}");
}
