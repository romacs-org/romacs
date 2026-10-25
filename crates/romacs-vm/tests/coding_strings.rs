use romacs_vm::Vm;

const SETUP: &str = r#"
(define-coding-system-internal 'coding-string-utf8 85 'utf-8 nil t nil nil nil nil 0 nil nil nil nil)
(define-coding-system-internal 'coding-string-raw 82 'raw-text nil t nil nil nil nil 0 t nil nil)
(define-coding-system-internal 'coding-string-multi 77 'raw-text nil t nil nil nil nil 0 nil nil 'unix)
(define-coding-system-internal 'coding-string-bom 66 'utf-8 nil nil nil nil nil nil 0 nil nil 'unix t)
(define-coding-system-alias 'coding-string-alias 'coding-string-utf8)
"#;

fn expressions() -> Vec<String> {
    let mut cases = Vec::new();
    for operation in ["encode-coding-string", "decode-coding-string"] {
        for coding in ["nil", "'no-conversion", "'coding-string-utf8", "'coding-string-utf8-unix", "'coding-string-utf8-dos", "'coding-string-utf8-mac", "'coding-string-raw", "'coding-string-multi", "'coding-string-alias", "'coding-string-bom"] {
            for source in [r#"(unibyte-string 97 0 98)"#, r#"(string-as-multibyte "ascii")"#, r#""λé\0x""#, r#"(unibyte-string 195 169 255)"#, r#"(unibyte-string 97 13 10 98 10)"#, r#"(unibyte-string 97 13 10 98 13)"#, r#""λ\r\n""#, r#"(unibyte-string 239 187 191 97)"#] {
                cases.push(format!("(let* ((source {source}) (result ({operation} source {coding}))) (list (multibyte-string-p result) (vconcat result) last-coding-system-used (eq result source)))"));
            }
        }
        for source in [r#"(unibyte-string 97)"#, r#"(string-as-multibyte "ascii")"#, r#""é""#, r#"(unibyte-string 255)"#] {
            for coding in ["nil", "'no-conversion", "'coding-string-utf8"] {
                cases.push(format!("(let* ((source {source}) (result ({operation} source {coding} t))) (list (multibyte-string-p result) (vconcat result) last-coding-system-used (eq result source)))"));
            }
        }
    }
    for operation in ["encode-coding-string", "decode-coding-string"] {
        for source in ["(string 55296)", "(string 1114112)", "(string 2097152)", "(string 4194175)", "(string 4194281)", "(unibyte-string 237 160 128)", "(unibyte-string 244 144 128 128)", "(unibyte-string 248 136 128 128 128)"] {
            cases.push(format!("(let ((result ({operation} {source} 'coding-string-utf8))) (list (multibyte-string-p result) (vconcat result)))"));
        }
    }
    for source in ["(string 13 233)", "(string 13 233 65)", "(string 13 233 10)", "(string 13 233 13)", "(unibyte-string 13 195 169)", "(unibyte-string 13 195)", "(unibyte-string 13 244)", "(unibyte-string 13 244 128)", "(unibyte-string 13 195 65)", "(unibyte-string 13 255 65)", "(unibyte-string 13 13)", "(unibyte-string 13 13 65)", "(unibyte-string 13 10)", "(unibyte-string 13)", "(unibyte-string 13 195 169 13 10)", "(unibyte-string 13 195 13 10)", "(string 13 4194243 233)", "(string 13 4194243 4194303)", "(string 13 4194243 4194175 65 66)"] {
        for coding in ["'coding-string-utf8-dos", "'coding-string-utf8", "'coding-string-utf8-unix"] {
            cases.push(format!("(let ((result (decode-coding-string {source} {coding}))) (list (vconcat result) last-coding-system-used))"));
        }
    }
    for source in ["(unibyte-string 192 160)", "(unibyte-string 193 191)", "(unibyte-string 192 128)", "(unibyte-string 193 128)", "(unibyte-string 192 160 193 191)", "(unibyte-string 192 10)", "(unibyte-string 192)", "(unibyte-string 193)", "(unibyte-string 192 192 160)", "(unibyte-string 192 160 13 10)", "(unibyte-string 237 160 128)", "(unibyte-string 248 136 128 128 128)"] {
        for coding in ["'coding-string-utf8-unix", "'coding-string-utf8-dos", "'coding-string-raw"] {
            cases.push(format!("(let ((result (encode-coding-string {source} {coding}))) (list (vconcat result) last-coding-system-used))"));
        }
    }
    for bytes in ["248 143 191 189 191", "248 143 191 190 128", "248 143 191 191 191"] {
        for (prefix, suffix) in [("", ""), ("13 ", ""), ("13 ", " 13 10"), ("", " 13 10")] {
            for coding in ["'coding-string-utf8-unix", "'coding-string-utf8-dos", "'coding-string-utf8-mac", "'coding-string-utf8"] {
                cases.push(format!("(let ((result (decode-coding-string (unibyte-string {prefix}{bytes}{suffix}) {coding}))) (list (vconcat result) (multibyte-string-p result) last-coding-system-used))"));
            }
        }
    }
    cases.extend([
        r#"(let ((inhibit-eol-conversion t)) (list (vconcat (decode-coding-string "λ\r\n" 'coding-string-utf8)) last-coding-system-used))"#,
        r#"(let ((source (propertize "a" 'p t))) (list (get-text-property 0 'p (encode-coding-string source nil)) (get-text-property 0 'p (decode-coding-string source 'no-conversion))))"#,
        r#"(list (encode-coding-string 1 'wrong) (decode-coding-string 1 nil))"#,
        r#"(encode-coding-string "a" 'coding-string-missing)"#,
        r#"(encode-coding-string "a" 15)"#,
        r#"(decode-coding-string "a" 'coding-string-utf8 nil 'wrong)"#,
        r#"(let* ((source "é") (result (decode-coding-string source nil t t))) (list (eq result source) (multibyte-string-p result) (vconcat result) last-coding-system-used))"#,
    ].into_iter().map(str::to_owned));
    cases
}

#[test]
fn nil_coding_and_utf8_roundtrip_are_real_conversions() {
    let mut vm = Vm::new();
    vm.eval_source(SETUP).unwrap();
    let source = r#"(let* ((s "λé\0x") (encoded (encode-coding-string s 'coding-string-utf8)) (decoded (decode-coding-string encoded 'coding-string-utf8))) (list (multibyte-string-p encoded) (vconcat encoded) (equal decoded s) last-coding-system-used))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(nil [206 187 195 169 0 120] t coding-string-utf8)");
    let source = r#"(let ((s (propertize "ascii" 'p t))) (list (eq s (encode-coding-string s nil t)) (eq s (decode-coding-string s nil t)) (get-text-property 0 'p (decode-coding-string s nil)) last-coding-system-used))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t t no-conversion)");
}

#[test]
fn string_coding_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in expressions() {
        let source = format!("(condition-case e {expression} (error (list 'signal (car e))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(progn {SETUP} (prin1 {source}))")]).output().unwrap();
        assert!(output.status.success(), "native coding probe failed: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.eval_source(SETUP).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{expression}\nGNU {expected}\nVM {actual:?}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn lazy_coding_definitions_keep_arguments_and_bindings_across_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = r#"(progn
      (setq coding-observed nil inhibit-redisplay nil)
      (defun coding-observe () (setq coding-observed inhibit-redisplay))
      (put 'coding-string-lazy 'coding-system-define-form
        '(progn (garbage-collect) (coding-observe)
          (define-coding-system-internal 'coding-string-lazy 85 'utf-8 nil t nil nil nil nil 0 nil nil 'unix nil)))
      (fset 'check-coding-system (lambda (&rest args) (error "mutable cell must not run")))
      (list (vconcat (encode-coding-string (concat "λ" "é") 'coding-string-lazy))
            coding-observed inhibit-redisplay
            (get 'coding-string-lazy 'coding-system-define-form)))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "([206 187 195 169] t nil nil)");
    let source = r#"(let ((last-coding-system-used 'keep) (inhibit-redisplay nil))
      (put 'coding-string-bad 'coding-system-define-form '(progn (garbage-collect) (error "failed definition")))
      (list (condition-case e (encode-coding-string "λ" 'coding-string-bad) (error (car e)))
            inhibit-redisplay last-coding-system-used))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(coding-system-error nil keep)");
}

#[test]
fn utf8_dos_lookahead_and_unibyte_internal_escapes_follow_gnu() {
    let mut vm = Vm::new();
    vm.eval_source(SETUP).unwrap();
    for (source, expected) in [
        ("(decode-coding-string (string 13 233) 'coding-string-utf8-dos)", "[13]"),
        ("(decode-coding-string (unibyte-string 13 195 169) 'coding-string-utf8-dos)", "[13 233]"),
        ("(decode-coding-string (unibyte-string 13 244) 'coding-string-utf8-dos)", "[13]"),
        ("(decode-coding-string (unibyte-string 13 244 128) 'coding-string-utf8-dos)", "[13 4194176]"),
        ("(encode-coding-string (unibyte-string 192 160) 'coding-string-utf8-unix)", "[160]"),
        ("(encode-coding-string (unibyte-string 193 191) 'coding-string-utf8-unix)", "[255]"),
    ] {
        let expression = format!("(vconcat {source})");
        assert_eq!(vm.eval_source(&expression).unwrap().display().unwrap(), expected, "{expression}");
    }
}

#[test]
fn external_utf8_accepts_five_byte_forms_of_byte8_characters() {
    let mut vm = Vm::new();
    vm.eval_source(SETUP).unwrap();
    for (bytes, expected) in [
        ("248 143 191 189 191", "[4194175]"),
        ("248 143 191 190 128", "[4194176]"),
        ("248 143 191 191 191", "[4194303]"),
    ] {
        let source = format!("(vconcat (decode-coding-string (unibyte-string {bytes}) 'coding-string-utf8-unix))");
        assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), expected, "{source}");
    }
}
