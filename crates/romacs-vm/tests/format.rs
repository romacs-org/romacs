use romacs_vm::Vm;

#[test]
fn mixed_raw_octets_and_unicode_follow_d002_without_silent_decoding() {
    // Pinned GNU: "λ\200\303\251\377", multibyte: a unibyte argument's
    // bytes above 127 become raw-byte characters (VM2-STR2), never UTF-8.
    let mut vm = Vm::new();
    let value = vm.eval_source(r#"(prin1-to-string (list (format "λ%s" (unibyte-string 128 195 169 255)) (multibyte-string-p (format "λ%s" (unibyte-string 128 195 169 255)))))"#).unwrap();
    let printed: String = serde_json::from_str(&value.display().unwrap()).unwrap();
    assert_eq!(printed, r#"("λ\200\303\251\377" t)"#);
    let value = vm
        .eval_source(r#"(format "λ%s" (string-as-multibyte (unibyte-string 195 169)))"#)
        .unwrap();
    assert_eq!(value.display().unwrap(), r#""λé""#);
}

#[test]
fn percent_s_accepts_both_unibyte_constructors() {
    let mut vm = Vm::new();
    for source in [
        r#"(format "%s" (unibyte-string 104 105))"#,
        r#"(format "%s" (string-to-unibyte "hi"))"#,
    ] {
        let result = vm.eval_source(source);
        assert!(result.is_ok(), "{source}: {result:?}");
        assert_eq!(result.unwrap().display().unwrap(), r#""hi""#);
    }
}

#[test]
fn direct_unibyte_format_preserves_octets_not_utf8() {
    let mut vm = Vm::new();
    for source in [
        r#"(format "%s" (unibyte-string 128 195 169 255))"#,
        r#"(format (unibyte-string 37 115) (unibyte-string 128 195 169 255))"#,
        r#"(format "%s" (unibyte-string 195 169))"#,
    ] {
        let value = vm.eval_source(source).unwrap();
        let expected: &[u8] = if source.contains("128") {
            &[128, 195, 169, 255]
        } else {
            &[195, 169]
        };
        assert_eq!(value.bytes().unwrap(), expected, "{source}");
    }
}

#[test]
fn quoted_and_nested_unibyte_format_uses_gnu_octal_escapes() {
    let mut vm = Vm::new();
    for (source, expected) in [
        (r#"(format "%S" (unibyte-string 104 105))"#, "\"hi\""),
        (r#"(format "%s" (list (unibyte-string 104 105)))"#, "(hi)"),
        (
            r#"(format "%S" (unibyte-string 128 195 169 255))"#,
            "\"\\200\\303\\251\\377\"",
        ),
        (
            r#"(format "%s" (list (unibyte-string 128 195 169 255)))"#,
            "(\\200\\303\\251\\377)",
        ),
        (
            r#"(format "%S" (list (unibyte-string 104 105)))"#,
            "(\"hi\")",
        ),
    ] {
        let value = vm.eval_source(source).unwrap();
        let printed = value.display().unwrap();
        let text: String = serde_json::from_str(&printed).unwrap();
        assert_eq!(text, expected, "{source}");
    }
}

/// styled_format prints a %S argument with prin1 and a non-string %s one
/// with princ (editfns.c:3693), so the printer and its variables decide:
/// a newline, TAB or other control character prints raw unless
/// `print-escape-newlines', and `print-length' holds. The expected list
/// is what pinned GNU's `equal' accepted for the same forms.
#[test]
fn percent_s_prints_as_prin1_and_princ_do() {
    let mut vm = Vm::new();
    let value = vm.eval_source(r#"(equal (list (format "%S" "a\nb")
             (format "%S" "t\te\eu\177")
             (format "%S" "é\"\\")
             (format "%S" '("a\nb" sym 1.5 [v "w\n"]))
             (format "%s" '("a\nb" "c"))
             (let ((print-escape-newlines t)) (format "%S" "a\nb\fc"))
             (let ((print-length 2)) (format "%S" '(1 2 3 4)))
             (format "%-8S|" "a")
             (format "%S" (unibyte-string 97 200))
             (let ((print-gensym t)) (format "%s %S" (make-symbol "g") (make-symbol "g"))))
        '("\"a\nb\"" "\"t\te\eu\177\"" "\"é\\\"\\\\\"" "(\"a\nb\" sym 1.5 [v \"w\n\"])" "(a\nb c)"
          "\"a\\nb\\fc\"" "(1 2 ...)" "\"a\"     |" "\"a\\310\"" "g #:g"))"#).unwrap();
    assert_eq!(value.display().unwrap(), "t");
}

/// print.c prints windows, frames, the terminal, overlays, markers and
/// buffers as `#<...>' forms, princ giving a buffer its bare name; format,
/// prin1-to-string and message print each as pinned GNU does. A frame's
/// pointer is its own, so only its presence is compared.
#[test]
fn display_objects_print_as_gnu_prints_them() {
    let source = r#"(let* ((b (get-buffer-create "pk"))
       (o (with-current-buffer b (insert "hello") (make-overlay 1 3)))
       (gone (with-current-buffer b (let ((o (make-overlay 1 2))) (delete-overlay o) o)))
       (killed (let ((k (get-buffer-create "pk-killed"))) (kill-buffer k) k)))
  (mapcar (lambda (x) (list (format "%s|%S" x x) (prin1-to-string x) (prin1-to-string x t) (message "%s" x)))
          (list (selected-frame) (selected-window) (minibuffer-window) (frame-terminal)
                (current-window-configuration) b killed (with-current-buffer b (copy-marker 2))
                (make-marker) o gone (list (selected-window) o))))"#;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pointers = |text: &str| {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find(" 0x") {
            out.push_str(&rest[..at + 3]);
            rest = rest[at + 3..].trim_start_matches(|c: char| c.is_ascii_hexdigit());
        }
        out + rest
    };
    let expected = pointers(&String::from_utf8(output.stdout).unwrap());
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let printed = vm.eval_source(&format!("(prin1-to-string {source})")).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap();
    assert!(actual.contains("#<frame F1 0x") && !actual.contains("#<frame F1 0x>"), "{actual}");
    assert_eq!(pointers(&actual), expected);
}
