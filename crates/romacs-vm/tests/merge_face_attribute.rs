//! xfaces.c Fmerge_face_attribute and merge_face_heights, as faces.el's
//! `face-attribute' uses them to follow `:inherit' (a face inheriting
//! `fixed-pitch' takes its family): against pinned GNU.
use romacs_vm::Vm;

const EXPRESSIONS: &[&str] = &[
    r#"(list (merge-face-attribute :family 'unspecified "Monospace") (merge-face-attribute :family "A" "B") (merge-face-attribute :weight :ignore-defface 'bold))"#,
    "(list (merge-face-attribute :height 1.5 100) (merge-face-attribute :height 1.5 2.0) (merge-face-attribute :height 1.5 'unspecified) (merge-face-attribute :height 120 1.5) (merge-face-attribute :height 'unspecified 1.5))",
    "(list (merge-face-attribute :height (lambda (h) (* 2 h)) 50) (functionp (merge-face-attribute :height (lambda (h) (* 1.5 h)) 50)) (merge-face-attribute :height (lambda (h) (* 1.5 h)) 2.0))",
    "(list (merge-face-attribute :foreground 1.5 100) (merge-face-attribute :height \"x\" 100))",
    "(condition-case e (merge-face-attribute :height 1.5) (error (car e)))",
];

#[test]
fn merge_face_attribute_matches_pinned_gnu() {
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
