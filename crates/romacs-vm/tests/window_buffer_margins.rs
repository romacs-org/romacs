//! `set-window-buffer' gives the window its buffer's margin widths unless
//! KEEP-MARGINS (window.c:4397), as keymap-popup's buffers rely on.
use romacs_vm::Vm;

const EXPRESSIONS: &[&str] = &[
    r#"(let ((b (get-buffer-create "margins")) (w (selected-window)))
         (with-current-buffer b (set (make-local-variable 'left-margin-width) 1)
                                (set (make-local-variable 'right-margin-width) 2))
         (set-window-buffer w b)
         (let ((taken (window-margins w)))
           (set-window-margins w 3 nil)
           (set-window-buffer w b t)
           (let ((kept (window-margins w)))
             (set-window-buffer w (get-buffer-create "plain"))
             (list taken kept (window-margins w)))))"#,
];

#[test]
fn window_buffer_margins_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    for expression in EXPRESSIONS {
        let source = format!("(prin1-to-string {expression})");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display())
            .map(|printed| serde_json::from_str::<String>(&printed).unwrap_or(printed));
        assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{expression}");
    }
}
