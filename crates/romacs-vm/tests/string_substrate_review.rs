use romacs_vm::Vm;
fn printed(src: &str) -> String { let mut vm=Vm::new(); vm.set_collection_floor(1); let v=vm.eval_source(&format!("(prin1-to-string (condition-case e {src} (error e)))")).unwrap(); serde_json::from_str(&v.display().unwrap()).unwrap() }
// Live GNU: (nil nil)
#[test] fn p1a_review_predicate() { assert_eq!(printed("(list (multibyte-string-p nil) (multibyte-string-p 1))"), "(nil nil)"); }
// Live GNU: [a a]
#[test] fn p1a_review_fill_vector() { assert_eq!(printed("(fillarray [0 0] 'a)"), "[a a]"); }
// Live GNU: (2 4 "\300\200")
#[test] fn p1a_review_parse_raw_leads() { assert_eq!(printed("(let ((s (string-as-multibyte (unibyte-string 192 128)))) (list (length s) (string-bytes s) (string-as-unibyte s)))"), "(2 4 \"\\300\\200\")"); }
// Live GNU: (error "Cannot convert character at index 0 to unibyte")
#[test] fn p1a_review_strict_to_unibyte() { assert_eq!(printed("(condition-case e (string-to-unibyte \"é\") (error e))"), "(error \"Cannot convert character at index 0 to unibyte\")"); }
// Live GNU: (t nil nil)
#[test] fn p1a_review_construct_kinds() { assert_eq!(printed("(list (multibyte-string-p (string 4194303)) (multibyte-string-p (string 97)) (multibyte-string-p (char-to-string 97)))"), "(t nil nil)"); }
// Live GNU: (args-out-of-range "ab" 5)
#[test] fn p1a_review_aset_index_first() { assert_eq!(printed("(condition-case e (aset (copy-sequence \"ab\") 5 4194304) (error e))"), "(args-out-of-range \"ab\" 5)"); }
// Live GNU: (args-out-of-range "a" 5)
#[test] fn p1a_review_aset_unibyte_index() { assert_eq!(printed("(condition-case e (aset (unibyte-string 97) 5 955) (error e))"), "(args-out-of-range \"a\" 5)"); }
// Live GNU: (t nil)
#[test] fn p1a_review_make_multibyte_ascii() { assert_eq!(printed("(let ((s (unibyte-string 97))) (list (eq s (string-make-multibyte s)) (multibyte-string-p (string-make-multibyte s))))"), "(t nil)"); }
// Live GNU: (t nil nil t): make- keeps the ASCII unibyte object, to- always widens.
#[test] fn p1a_review_make_and_to_multibyte_ascii_differ() { assert_eq!(printed("(let ((s (unibyte-string 97))) (list (eq s (string-make-multibyte s)) (multibyte-string-p (string-make-multibyte s)) (eq s (string-to-multibyte s)) (multibyte-string-p (string-to-multibyte s))))"), "(t nil nil t)"); }
// Live GNU: "\"\\377\""
#[test] fn p1a_review_format_raw() { assert_eq!(printed("(format \"%S\" (char-to-string 4194303))"), "\"\\\"\\\\377\\\"\""); }
// Live GNU: "\377"
#[test] fn p1a_review_substring_octets() { assert_eq!(printed("(substring (unibyte-string 97 255 98) 1 2)"), "\"\\377\""); }
// Live GNU: bold
#[test] fn p1a_review_existing_properties() { assert_eq!(printed("(let ((s (copy-sequence \"ab\"))) (put-text-property 0 1 'face 'bold s) (get-text-property 0 'face s))"), "bold"); }

// Live GNU: (wrong-type-argument bufferp nil) for a name no buffer has, raw
// bytes included (VM2-BUFNAME-RAW1); with such a buffer, its buffer.
#[test] fn p1a_review_display_buffer_name_raw() {
    assert_eq!(printed("(condition-case e (set-window-buffer nil (char-to-string 4194303)) (error e))"), "(wrong-type-argument bufferp nil)");
    assert_eq!(printed("(progn (get-buffer-create (char-to-string 4194303)) (set-window-buffer nil (char-to-string 4194303)) (buffer-name (window-buffer)))"), "\"\\377\"");
}
