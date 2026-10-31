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
