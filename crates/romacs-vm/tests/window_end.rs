//! window-end (window.c:1782) in batch: GNU's noninteractive session
//! answers from the end the last redisplay recorded, with or without
//! UPDATE, after decode_live_window's checks; compared against pinned GNU.
//! romacs-vm-gui's measure.rs tests UPDATE in a window a host lays out.
use romacs_vm::Vm;

const EXPRESSIONS: &[&str] = &[
    "(list (window-end) (window-end nil t) (window-end (selected-window) t))",
    r#"(progn (set-window-buffer nil (get-buffer-create "end-abc")) (with-current-buffer "end-abc" (insert "abc\ndef")) (list (window-end) (window-end nil t)))"#,
    "(window-end (minibuffer-window) t)",
    "(condition-case e (window-end 'x t) (error e))",
    "(condition-case e (window-end 1) (error e))",
];

#[test]
fn batch_window_end_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mut failures = Vec::new();
    for expression in EXPRESSIONS {
        let source = format!("(prin1-to-string {expression})");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
