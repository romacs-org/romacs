//! `format' carries text properties as GNU's styled_format does
//! (editfns.c:4315-4410): the format string's, moved to where its characters
//! land, and each `%s' argument's, extended over its padded field.
use romacs_vm::Vm;

const EXPRESSIONS: &[&str] = &[
    r#"(format "%s" (propertize "ab" 'face 'bold))"#,
    r#"(format "x%sy" (propertize "ab" 'face 'bold))"#,
    r#"(format "Studied today: %s attempts" (propertize "42" 'face 'success))"#,
    r#"(format "%-5s|" (propertize "ab" 'face 'bold))"#,
    r#"(format "%5s|" (propertize "ab" 'face 'bold))"#,
    r#"(format "%5s|" (concat (propertize "a" 'face 'bold) "b"))"#,
    r#"(format "%.1s|" (propertize "ab" 'face 'bold))"#,
    r#"(format "%s %s" (propertize "α" 'face 'bold) (propertize "βγ" 'help-echo "h"))"#,
    r#"(format "%S" (propertize "ab" 'face 'bold))"#,
    r#"(format "%d %s" 7 (propertize "ab" 'face 'bold 'mouse-face 'highlight))"#,
    r#"(format (propertize "%s!" 'face 'italic) "ab")"#,
    r#"(format (concat "<" (propertize "%s" 'face 'italic) ">") "abc")"#,
    r#"(format (concat (propertize "a%%b" 'face 'italic) "%s") "c")"#,
    r#"(format (propertize "%s-%s" 'face 'italic) (propertize "x" 'face 'bold) "y")"#,
    r#"(format "%2$s %1$s" (propertize "one" 'face 'bold) "two")"#,
    r#"(format-message "`%s' %s" (propertize "q" 'face 'bold) "z")"#,
    r#"(format "%s" (propertize "" 'face 'bold))"#,
    // A propertied format string: the padded field takes the argument's
    // properties on both sides (styled_format's fmt_props).
    r#"(format (propertize "%5s|" 'face 'italic) (propertize "ab" 'face 'bold))"#,
    r#"(format (propertize "%-5s|" 'face 'italic) (propertize "ab" 'face 'bold))"#,
    r#"(format (propertize "<%.1s>" 'face 'italic) (propertize "ab" 'face 'bold))"#,
    r#"(format (propertize "<%4.1s>" 'face 'italic) (propertize "ab" 'face 'bold))"#,
    r#"(format (propertize "%5S|" 'face 'italic) (propertize "ab" 'face 'bold))"#,
    r#"(format (concat "x" (propertize "%6s" 'face 'italic)) (concat (propertize "a" 'face 'bold) "b"))"#,
];

#[test]
fn format_properties_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
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
