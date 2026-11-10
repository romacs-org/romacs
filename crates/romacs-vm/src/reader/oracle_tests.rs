// Frozen GNU corpus assertions; expected values are recorded, not regenerated.
// Finite number-sequence calls use their literal indexes to avoid a subr.el
// bootstrap dependency. Killed-buffer diagnostics are checked by identity.
// Values and callback ordering come from the pinned GNU reader.
#[test]
fn reader_duplicate_property_keys_refuse_instead_of_losing_data() {
    let mut vm = Vm::new();
    for literal in [
        r#"#("a" 0 1 (face bold face italic))"#,
        r#"#("a" 0 1 (face bold help-echo "x" face italic))"#,
        r#"#("a" 0 1 (face bold face bold))"#,
        r#"#("a" 0 1 (nil 1 nil 2))"#,
    ] {
        let source = format!(
            "(condition-case e (progn (read {literal:?}) nil) (error (equal (cdr e) '(\"Reader cannot retain duplicate text property keys\"))))"
        );
        let result = vm.eval_source_raw(&source).unwrap();
        assert_eq!(
            result,
            Value::TRUE,
            "must reject rather than collapse the GNU plist: {literal}"
        );
    }
    let result = vm.eval_source_raw(
        r##"(equal (text-properties-at 0 (read "#(\"a\" 0 1 (face bold help-echo \"x\"))")) '(face bold help-echo "x"))"##
    ).unwrap();
    assert_eq!(result, Value::TRUE);
}
#[test]
fn reader_nested_callback_unread_is_immediate() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let actual = vm.eval_source_raw(r#"
      (let ((source "(foo bar)") (i 0) log)
        (list (read (lambda (&optional c)
          (if c
            (progn (push (list 'unread c) log) (setq i (1- i)))
            (let ((v (if (< i (length source))
                         (prog1 (aref source i) (setq i (1+ i))) nil)))
              (push (list 'get v) log) v))))
          (nreverse log)))"#).unwrap();
    assert_eq!(vm.display(actual).unwrap(), "((foo bar) ((get 40) (get 102) (get 111) (get 111) (get 32) (unread 32) (get 32) (get 98) (get 97) (get 114) (get 41) (unread 41) (get 41)))");
}
#[test]
fn reader_error_callback_collection_preserves_condition() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let value = vm.eval_source_raw(r#"(let ((s ". a") (i 0))
      (condition-case e
        (read (lambda (&optional c)
          (if c (progn (garbage-collect) (setq i (1- i)))
            (if (< i (length s)) (prog1 (aref s i) (setq i (1+ i))) nil))))
        (invalid-read-syntax (list e i))))"#).unwrap();
    assert_eq!(vm.display(value).unwrap(), "((invalid-read-syntax \".\") 1)");
}

#[test]
fn reader_error_closer_is_not_unread() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let value = vm.eval_source_raw(r#"(let ((s "(a . )") (i 0) log)
      (condition-case e
        (read (lambda (&optional c)
          (if c (progn (push c log) (setq i (1- i)))
            (if (< i (length s)) (prog1 (aref s i) (setq i (1+ i))) nil))))
        (invalid-read-syntax (list e i (nreverse log)))))"#).unwrap();
    assert_eq!(vm.display(value).unwrap(), "((invalid-read-syntax \")\") 6 (32 32))");
}

#[test]
fn reader_chartable_valid_sub() {
    let mut vm = Vm::new();
    let literal = "#^^[1 0 nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil]";
    let (value, end) = vm.read_source_raw(literal).unwrap().unwrap();
    assert_eq!(end, literal.len());
    assert!(matches!(vm.object(value).unwrap(), Object::SubCharTable(_)));
}

#[test]
fn reader_string_sequence() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (list (read-from-string "12 34") (read-from-string "12 34" 2) (read-from-string "xx αβ zz" 3 5))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok ((12 . 2) (34 . 5) (αβ . 5))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_sequence: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_sequence mismatch");
}
#[test]
fn reader_string_negative_bounds() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (list (read-from-string "xx foo yy" -6 -3) (read-from-string "42" 0 -1))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok ((foo . 6) (4 . 1))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_negative_bounds: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_negative_bounds mismatch");
}
#[test]
fn reader_string_source_properties() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((s (propertize "\"abc\"" 'face 'bold))) (text-properties-at 0 (read s)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok nil))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_source_properties: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_string_source_properties mismatch"
    );
}
#[test]
fn reader_string_eof() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read " ;comment")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error end-of-file nil "End of file during parsing"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_eof: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_eof mismatch");
}
#[test]
fn reader_string_incomplete_list() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read "(abc")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error end-of-file nil "End of file during parsing"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_incomplete_list: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_incomplete_list mismatch");
}
#[test]
fn reader_string_closer() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read ")")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-read-syntax (")") "Invalid read syntax: \")\""))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_closer: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_closer mismatch");
}
#[test]
fn reader_string_trailing() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read-from-string "foo)bar")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (foo . 3)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_trailing: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_trailing mismatch");
}
#[test]
fn reader_string_range() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read-from-string "foo" 4)) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error args-out-of-range ("foo" 4 nil) "Args out of range: \"foo\", 4, nil"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_range: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_range mismatch");
}
#[test]
fn reader_string_wrong_type() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read-from-string 3)) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error wrong-type-argument (stringp 3) "Wrong type argument: stringp, 3"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_string_wrong_type: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_string_wrong_type mismatch");
}
#[test]
fn reader_read_wrong_stream() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read 7)) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-function (7) "Invalid function: 7"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_read_wrong_stream: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_read_wrong_stream mismatch");
}
#[test]
fn reader_stream_buffer() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (with-temp-buffer (insert "foo(bar) 99") (goto-char 1) (list (read (current-buffer)) (point) (read (current-buffer)) (point) (read (current-buffer)) (point)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (foo 4 (bar) 9 99 12)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_buffer: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_buffer mismatch");
}
#[test]
fn reader_stream_buffer_error() {
    let mut vm = Vm::new();
    let value=vm.eval_source_raw(r#"(with-temp-buffer (insert "(foo") (goto-char 1) (let ((b (current-buffer))) (list (condition-case e (read b) (error (list (car e) (eq (car (cdr e)) b)))) (point))))"#).unwrap();
    assert_eq!(vm.display(value).unwrap(), "((end-of-file t) 5)");
}

#[test]
fn reader_stream_buffer_narrow() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (with-temp-buffer (insert "xxαβ yy") (narrow-to-region 3 5) (goto-char 3) (list (read (current-buffer)) (point) (point-min) (point-max)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (αβ 5 3 5)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_buffer_narrow: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_buffer_narrow mismatch");
}
#[test]
fn reader_stream_marker() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (with-temp-buffer (insert "foo(bar)") (goto-char 7) (let ((m (copy-marker 1))) (list (read m) (marker-position m) (point) (read m) (marker-position m) (point))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (foo 4 7 (bar) 9 7)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_marker: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_marker mismatch");
}
#[test]
fn reader_stream_marker_error() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (with-temp-buffer (insert "(foo") (let ((m (copy-marker 1))) (list (condition-case e (read m) (error (list e (error-message-string e)))) (marker-position m) (point))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (((end-of-file) "End of file during parsing") 5 5)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_marker_error: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_marker_error mismatch");
}
#[test]
fn reader_stream_standard_input_omitted() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (with-temp-buffer (insert "one two") (goto-char 1) (let ((standard-input (current-buffer))) (list (read) (point) (read nil) (point))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (one 4 two 8)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_standard_input_omitted: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_stream_standard_input_omitted mismatch"
    );
}
#[test]
fn reader_stream_standard_input_string() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((standard-input "alpha beta")) (list (read) (read nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (alpha alpha)))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_standard_input_string: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_stream_standard_input_string mismatch"
    );
}
#[test]
fn reader_stream_callback_symbol() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "foo(bar)") (i 0) log) (let ((fn (lambda (&optional c) (if c (progn (push (list 'unread c) log) (setq i (1- i)) nil) (let ((v (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil))) (push (list 'get v) log) v))))) (list (condition-case e (read fn) (error (list e (error-message-string e)))) i (nreverse log))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (foo 3 ((get 102) (get 111) (get 111) (get 40) (unread 40)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_symbol: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_callback_symbol mismatch");
}
#[test]
fn reader_stream_callback_number() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "12 34") (i 0) log) (let ((fn (lambda (&optional c) (if c (progn (push (list 'unread c) log) (setq i (1- i)) nil) (let ((v (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil))) (push (list 'get v) log) v))))) (list (condition-case e (read fn) (error (list e (error-message-string e)))) i (nreverse log))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (12 2 ((get 49) (get 50) (get 32) (unread 32)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_number: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_callback_number mismatch");
}
#[test]
fn reader_stream_callback_char() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "?a x") (i 0) log) (let ((fn (lambda (&optional c) (if c (progn (push (list 'unread c) log) (setq i (1- i)) nil) (let ((v (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil))) (push (list 'get v) log) v))))) (list (condition-case e (read fn) (error (list e (error-message-string e)))) i (nreverse log))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (97 2 ((get 63) (get 97) (get 32) (unread 32)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_char: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_callback_char mismatch");
}
#[test]
fn reader_stream_callback_eof() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "foo") (i 0) log) (let ((fn (lambda (&optional c) (if c (progn (push (list 'unread c) log) (setq i (1- i)) nil) (let ((v (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil))) (push (list 'get v) log) v))))) (list (condition-case e (read fn) (error (list e (error-message-string e)))) i (nreverse log))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (foo 3 ((get 102) (get 111) (get 111) (get nil)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_eof: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_callback_eof mismatch");
}
#[test]
fn reader_stream_callback_error() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "(foo") (i 0) log) (let ((fn (lambda (&optional c) (if c (progn (push (list 'unread c) log) (setq i (1- i)) nil) (let ((v (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil))) (push (list 'get v) log) v))))) (list (condition-case e (read fn) (error (list e (error-message-string e)))) i (nreverse log))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (((end-of-file) "End of file during parsing") 4 ((get 40) (get 102) (get 111) (get 111) (get nil) (get nil)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_error: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_callback_error mismatch");
}
#[test]
fn reader_stream_callback_noncharacter() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "") (i 0) log) (let ((fn (lambda (&optional c) (push c log) 'not-a-character))) (list (condition-case e (read fn) (error (list e (error-message-string e)))) i (nreverse log))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (((end-of-file) "End of file during parsing") 0 (nil))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_noncharacter: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_stream_callback_noncharacter mismatch"
    );
}
#[test]
fn reader_stream_callback_reentrant() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source "foo ") (i 0) log) (let ((fn (lambda (&optional c) (if c (setq i (1- i)) (progn (push (read "(nested)") log) (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil)))))) (list (read fn) i log)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (foo 3 ((nested) (nested) (nested) (nested)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_reentrant: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_stream_callback_reentrant mismatch"
    );
}
#[test]
fn reader_stream_callback_invalid_char() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read (lambda (&optional c) 4194304))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error end-of-file nil "End of file during parsing"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_invalid_char: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_stream_callback_invalid_char mismatch"
    );
}
#[test]
fn reader_bytes_unibyte_literal() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read (unibyte-string 34 128 195 169 255 34)))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (nil 4 4 (128 195 169 255))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_unibyte_literal: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_unibyte_literal mismatch");
}
#[test]
fn reader_bytes_raw_escaped() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "\"\\200\\377\""))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (nil 2 2 (128 255))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_raw_escaped: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_raw_escaped mismatch");
}
#[test]
fn reader_bytes_mixed() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "\"α\\200β\\377\""))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (t 4 8 (945 4194176 946 4194303))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_mixed: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_mixed mismatch");
}
#[test]
fn reader_bytes_extended() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "\"\\x110000 \\x3fff7f\""))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (t 3 10 (1114112 32 4194175))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_extended: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_extended mismatch");
}
#[test]
fn reader_bytes_ascii() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "\"abc\""))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (nil 3 3 (97 98 99))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_ascii: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_ascii mismatch");
}
#[test]
fn reader_bytes_multibyte_ascii() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read (string-to-multibyte "\"abc\"")))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil)))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (nil 3 3 (97 98 99))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_multibyte_ascii: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_multibyte_ascii mismatch");
}
#[test]
fn reader_bytes_unibyte_buffer() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 34 128 195 169 255 34)) (goto-char 1) (let ((v (read (current-buffer)))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (nil 4 4 (128 195 169 255))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_unibyte_buffer: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_unibyte_buffer mismatch");
}
#[test]
fn reader_bytes_raw_character() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read (unibyte-string 63 255 32))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok 255))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_raw_character: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_raw_character mismatch");
}
#[test]
fn reader_bytes_callback_raw() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((source (vector 34 945 4194176 34)) (i 0) log) (let ((v (read (lambda (&optional c) (if c (progn (push (list 'unread c) log) (setq i (1- i)) nil) (let ((v (if (< i (length source)) (prog1 (aref source i) (setq i (1+ i))) nil))) (push (list 'get v) log) v)))))) (list (multibyte-string-p v) (length v) (string-bytes v) (append v nil))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (t 2 4 (945 4194176))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bytes_callback_raw: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_bytes_callback_raw mismatch");
}
#[test]
fn reader_properties_empty() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abc\")"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) '(0 1 2))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok ("abc" nil (nil nil nil))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_empty: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_empty mismatch");
}
#[test]
fn reader_properties_simple() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abc\" 0 2 (face bold))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) '(0 1 2))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (#("abc" 0 2 (face bold)) nil (#1=(face bold) #1# nil))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_simple: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_simple mismatch");
}
#[test]
fn reader_properties_overlap() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abcd\" 0 4 (face bold x 1) 1 3 (face italic))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) '(0 1 2 3))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (#("abcd" 0 1 (face bold x 1) 1 3 (face italic) 3 4 (face bold x 1)) nil (#1=(face bold x 1) #2=(face italic) #2# #3=(face bold x 1)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_overlap: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_overlap mismatch");
}
#[test]
fn reader_properties_clear() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abcd\" 0 4 (face bold) 1 3 nil)"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) '(0 1 2 3))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (#("abcd" 0 1 (face bold) 3 4 (face bold)) nil (#1=(face bold) nil nil #2=(face bold)))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_clear: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_clear mismatch");
}
#[test]
fn reader_properties_unicode() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"αβγ\" 1 2 (help-echo \"yes\" rear-nonsticky t))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) '(0 1 2))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (#("\x03b1\x03b2\x03b3" 1 2 (help-echo "yes" rear-nonsticky t)) t (nil #1=(help-echo "yes" rear-nonsticky t) nil))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_unicode: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_unicode mismatch");
}
#[test]
fn reader_properties_value_shapes() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"ab\" 0 2 (x (a . b) y [1 2] z nil))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) '(0 1))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok (#("ab" 0 2 (x (a . b) y [1 2] z nil)) nil (#1=(x (a . b) y [1 2] z nil) #1#))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_value_shapes: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_properties_value_shapes mismatch"
    );
}
#[test]
fn reader_properties_not_string() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(12 0 1 (face bold))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) (number-sequence 0 (1- (length v))))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-read-syntax ("#") "Invalid read syntax: \"#\""))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_not_string: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_not_string mismatch");
}
#[test]
fn reader_properties_missing() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abc\" 0 1)"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) (number-sequence 0 (1- (length v))))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-read-syntax ("Invalid string property list") "Invalid read syntax: \"Invalid string property list\""))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_missing: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_missing mismatch");
}
#[test]
fn reader_properties_range() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abc\" 0 4 (face bold))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) (number-sequence 0 (1- (length v))))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error args-out-of-range (0 4) "Args out of range: 0, 4"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_range: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_range mismatch");
}
#[test]
fn reader_properties_odd_plist() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abc\" 0 1 (face))"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) (number-sequence 0 (1- (length v))))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error error (#1="Odd length text property list") #1#))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_odd_plist: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_odd_plist mismatch");
}
#[test]
fn reader_properties_negative() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (let ((v (read "#(\"abc\" -1 1 nil)"))) (list v (multibyte-string-p v) (mapcar (lambda (i) (text-properties-at i v)) (number-sequence 0 (1- (length v))))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error args-out-of-range (-1 1) "Args out of range: -1, 1"))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_properties_negative: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_properties_negative mismatch");
}
#[test]
fn reader_stream_callback_signal() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (read (lambda (&optional c) (signal 'error '("callback fault"))))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error error (#1="callback fault") #1#))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_stream_callback_signal: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(value, Value::TRUE, "reader_stream_callback_signal mismatch");
}
#[test]
fn reader_bindings_control_digits() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let source = r#####"(equal (condition-case e (list :ok (mapcar (lambda (i) (append (read (format "[?\\C-%c]" i)) nil)) '(48 49 50 51 52 53 54 55 56 57))) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:ok ((67108912) (67108913) (67108914) (67108915) (67108916) (67108917) (67108918) (67108919) (67108920) (67108921))))"#####;
    let value = vm
        .eval_source_raw(source)
        .unwrap_or_else(|e| panic!("reader_bindings_control_digits: {e}"));
    if value != Value::TRUE {
        let detail = vm
            .eval_source_raw(&source.replacen("(equal ", "(list ", 1))
            .unwrap();
        panic!("{}", vm.display(detail).unwrap());
    }
    assert_eq!(
        value,
        Value::TRUE,
        "reader_bindings_control_digits mismatch"
    );
}

#[test]
fn reader_control_digit_characters() {
    let mut vm = Vm::new();
    for digit in 0..10 {
        let source = format!("[?\\C-{}]", digit);
        let (value, _) = vm.read_source_raw(&source).unwrap().unwrap();
        let Object::Vector(items) = vm.object(value).unwrap() else {
            panic!("vector required")
        };
        assert_eq!(items[0].integer_value(), Some((1 << 26) + 48 + digit));
    }
}
#[test]
fn reader_detached_marker_refuses_safely() {
    let mut vm = Vm::new();
    let value = vm
        .eval_source_raw("(condition-case e (read (make-marker)) (error (error-message-string e)))")
        .unwrap();
    assert_eq!(
        vm.str_view(value).unwrap().as_str(),
        Some("Reader refuses a detached marker")
    );
}
#[test]
fn reader_interactive_stream_is_explicitly_unwired() {
    let mut vm = Vm::new();
    let value = vm
        .eval_source_raw("(condition-case e (read t) (error (error-message-string e)))")
        .unwrap();
    assert_eq!(
        vm.str_view(value).unwrap().as_str(),
        Some("Reader interactive stdin is not wired")
    );
}

#[test]
fn reader_chartable_prefix() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^x")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-read-syntax ("#^x") "Invalid read syntax: \"#^x\""))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_space() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^ []")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-read-syntax ("#^ ") "Invalid read syntax: \"#^ \""))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_short() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^[]")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error invalid-read-syntax ("Invalid size char-table") "Invalid read syntax: \"Invalid size char-table\""))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_subshort() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^^[]")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error error (#1="Invalid size of sub-char-table") #1#))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_depth() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^^[4 0]")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error error (#1="Invalid depth in sub-char-table") #1#))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_size() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^^[1 0]")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error error (#1="Invalid size in sub-char-table") #1#))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_minimum() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source_raw(r#####"(equal (condition-case e (list :ok (read "#^^[1 -1 nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil nil]")) (error (list :error (car e) (cdr e) (error-message-string e)))) '(:error error (#1="Invalid minimum character in sub-char-table") #1#))"#####).unwrap(),Value::TRUE);
}

#[test]
fn reader_chartable_uniform() {
    reader_table_fixture(include_str!("fixtures/uniform.el"));
}

#[test]
fn reader_chartable_subdivision() {
    reader_table_fixture(include_str!("fixtures/subdivision.el"));
}

#[test]
fn reader_chartable_case() {
    reader_table_fixture(include_str!("fixtures/case.el"));
}

#[test]
fn reader_chartable_syntax() {
    reader_table_fixture(include_str!("fixtures/syntax.el"));
}

fn reader_table_fixture(literal: &str) {
    let mut vm = Vm::new();
    let literal = literal.trim();
    let input = vm.make_lisp_string(literal.as_bytes().to_vec(), false);
    let slot = vm.slot("reader-fixture");
    vm.slots[slot].value = input;
    let value=vm.eval_source_raw("(let ((print-circle t) (print-escape-nonascii t) (print-escape-multibyte t)) (prin1-to-string (read reader-fixture)))").unwrap();
    let actual = vm.str_view(value).unwrap().bytes();
    assert!(
        normalize_reader_labels(literal.as_bytes()) == normalize_reader_labels(actual),
        "table graph roundtrip differs"
    );
}

// Label numbers are printer traversal choices; normalize labels, not graph edges.
fn normalize_reader_labels(bytes: &[u8]) -> Vec<u8> {
    let mut labels = BTreeMap::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start && end < bytes.len() && matches!(bytes[end], b'=' | b'#') {
                let next = labels.len() + 1;
                let number = *labels.entry(bytes[start..end].to_vec()).or_insert(next);
                out.push(b'#');
                out.extend(number.to_string().as_bytes());
                out.push(bytes[end]);
                i = end + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}
#[test]
fn reader_numbered_aliases_keep_identity() {
    let mut vm = Vm::new();
    assert_eq!(
        vm.eval_source_raw(r#"(let ((v (read "(#1=(a) #1#)"))) (eq (car v) (car (cdr v))))"#)
            .unwrap(),
        Value::TRUE
    );
}
#[test]
fn reader_callback_partials_and_source_survive_compaction() {
    let mut vm = Vm::new();
    let function=vm.eval_source_raw(r#"(let ((s "(\"first\" [\"second\"] (\"third\"))") (i 0)) (lambda (&optional c) (if c (setq i (1- i)) (if (< i (length s)) (prog1 (aref s i) (setq i (1+ i))) nil))))"#).unwrap();
    let root = vm.external_roots.len();
    vm.external_roots.push(function);
    let shown = crate::threaded::toplevel(&mut vm, root, |mut m, gc, root| {
        let mut reader = Reader {
            m: &mut m,
            gc: Some(gc),
            input: Input::Function { root },
            at: 0,
            lookahead: None,
            error: None,
            eof: false,
            labels: BTreeMap::new(),
            signal: None,
            collect_subforms: true,
        };
        let value = reader.form(0).and_then(|value| reader.finish().map(|()| value));
        m.leaf(value, |vm, value| value.and_then(|root| vm.display(vm.external_roots[root])))
    });
    assert_eq!(shown, Ok(r#"("first" ["second"] ("third"))"#.to_owned()));
}
