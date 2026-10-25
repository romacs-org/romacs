use romacs_vm::Vm;

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let result = vm
        .eval_source(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"));
    assert_eq!(result.display().unwrap(), expected, "{source}");
}

#[test]
fn callable_and_default_sort() {
    for (source, expected) in [
        (
            r#"(list (fboundp 'value<) (subrp (symbol-function 'value<)) (functionp 'value<) (funcall 'value< 1 2))"#,
            r#"(t t t t)"#,
        ),
        (r#"(sort (list "b" "a"))"#, r#"("a" "b")"#),
        (r#"(sort [3 1 2])"#, r#"[1 2 3]"#),
        (r#"(sort (list '(1 b) '(1 a) nil))"#, r#"(nil (1 a) (1 b))"#),
        (
            r#"(condition-case e (sort (list "b" 'x)) (error e))"#,
            r#"(type-mismatch x "b")"#,
        ),
        (
            r#"(condition-case e (funcall 'value< 1) (error (list (car e) (eq (car (cdr e)) (symbol-function 'value<)) (car (cdr (cdr e))))))"#,
            r#"(wrong-number-of-arguments t 1)"#,
        ),
        (
            r#"(condition-case e (funcall 'value< 1 2 3) (error (list (car e) (eq (car (cdr e)) (symbol-function 'value<)) (car (cdr (cdr e))))))"#,
            r#"(wrong-number-of-arguments t 3)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn numeric_order_is_exact_and_nan_is_unordered() {
    for (source, expected) in [
        (
            r#"(list (value< -2 -1) (value< 2 1) (value< 1 1) (value< -0.0 0.0) (value< 1 1.5) (value< -1.5 -1))"#,
            r#"(t nil nil nil t t)"#,
        ),
        (
            r#"(list (value< 9007199254740992.0 9007199254740993) (value< 9007199254740993 9007199254740992.0) (value< 2305843009213693951 2305843009213693952.0))"#,
            r#"(t nil t)"#,
        ),
        (
            r#"(let* ((a (+ 2305843009213693951 1)) (b (+ a 1))) (list (value< a b) (value< (- b) (- a)) (value< a 1.0e+INF) (value< -1.0e+INF (- b))))"#,
            r#"(t t t t)"#,
        ),
        (
            r#"(let ((a (+ 2305843009213693951 1))) (list (value< 0.0e+NaN 1) (value< 1 0.0e+NaN) (value< 0.0e+NaN a) (value< a 0.0e+NaN)))"#,
            r#"(nil nil nil nil)"#,
        ),
        (
            r#"(condition-case e (value< (+ 2305843009213693951 1) "a") (error e))"#,
            r#"(type-mismatch "a" 2305843009213693952)"#,
        ),
        (
            r#"(condition-case e (value< (+ 2305843009213693951 1) [1]) (error e))"#,
            r#"(type-mismatch 2305843009213693952 [1])"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn strings_and_symbols_compare_character_codes() {
    for (source, expected) in [
        (
            r#"(list (value< "a" "aa") (value< "å" "α") (value< 'a 'b) (value< nil t) (value< (make-symbol "same") (make-symbol "same")))"#,
            r#"(t t t t nil)"#,
        ),
        (
            r#"(list (value< (unibyte-string 255) (string #x3fffff)) (value< (string #x3fff80) "å") (value< (unibyte-string 128) "å") (value< "å" (string #x3fff80)))"#,
            r#"(t nil t t)"#,
        ),
        (
            r#"(condition-case e (value< nil "") (error e))"#,
            r#"(type-mismatch nil "")"#,
        ),
        (
            r#"(let* ((table (vector 0)) (name (unibyte-string 195 169)) (a (intern name table)) (b (intern "é" table))) (list (equal name (symbol-name a)) (value< a b) (value< b a)))"#,
            r#"(t t nil)"#,
        ),
        (
            r#"(let* ((name (unibyte-string 195 169)) (s (intern name (vector 0)))) (aset name 0 255) (list (eq name (symbol-name s)) (value< 'z s) (value< s 'z)))"#,
            r#"(t t nil)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn list_order_reports_current_tails_and_preserves_cycle_rules() {
    for (source, expected) in [
        (
            r#"(list (value< nil '(1)) (value< '(1) nil) (value< '(1 "a") '(1 "b")) (value< '(1) '(1 2)) (value< '(1 3) '(1 2)))"#,
            r#"(t nil t t nil)"#,
        ),
        (
            r#"(condition-case e (value< '(1 . a) '(1 2)) (error e))"#,
            r#"(type-mismatch a (2))"#,
        ),
        (
            r#"(condition-case e (value< '(1 2 . 3) '(1 2)) (error e))"#,
            r#"(type-mismatch 3 nil)"#,
        ),
        (
            r#"(let ((a (list 1))) (setcdr a a) (value< a (list 1 1 1)))"#,
            r#"nil"#,
        ),
        (
            r#"(let ((b (list 1))) (setcdr b b) (condition-case e (value< (list 1 1 1) b) (error (list (car e) (eq (car (cdr e)) b)))))"#,
            r#"(circular-list t)"#,
        ),
        (
            r#"(let ((tail (list 1))) (setcdr tail tail) (condition-case e (value< (cons 1 tail) (cons 1 tail)) (error (list (car e) (eq (car (cdr e)) tail)))))"#,
            r#"(circular-list t)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn vectors_records_and_bool_vectors_are_lexicographic() {
    for (source, expected) in [
        (
            r#"(list (value< [1 a] [1 b]) (value< [1] [1 2]) (value< [1 2] [1]) (value< (record 'a 9) (record 'b 0)) (value< (record 'a 1) (record 'a 1 2)))"#,
            r#"(t t nil t t)"#,
        ),
        (
            r#"(let ((a (make-bool-vector 9 nil)) (b (make-bool-vector 9 nil))) (aset a 7 t) (aset b 0 t) (list (value< a b) (value< b a) (value< (make-bool-vector 8 nil) (make-bool-vector 9 nil))))"#,
            r#"(t nil t)"#,
        ),
        (
            r#"(let ((a (record 'a))) (condition-case e (value< a [a]) (error (list (car e) (eq (car (cdr e)) a) (equal (car (cdr (cdr e))) [a])))))"#,
            r#"(type-mismatch t t)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn recursion_limit_and_identity_match_gnu() {
    for (source, expected) in [
        (
            r#"(let ((a 1) (b 2) (i 0)) (while (< i 200) (setq a (list a) b (list b) i (1+ i))) (value< a b))"#,
            r#"t"#,
        ),
        (
            r#"(let ((a 1) (b 2) (i 0)) (while (< i 201) (setq a (list a) b (list b) i (1+ i))) (condition-case e (value< a b) (error e)))"#,
            r#"(error "Maximum depth exceeded in comparison")"#,
        ),
        (
            r#"(let ((a 1) (b 2) (i 0)) (while (< i 200) (setq a (vector a) b (vector b) i (1+ i))) (value< a b))"#,
            r#"t"#,
        ),
        (
            r#"(let ((a 1) (b 2) (i 0)) (while (< i 201) (setq a (vector a) b (vector b) i (1+ i))) (condition-case e (value< a b) (error e)))"#,
            r#"(error "Maximum depth exceeded in comparison")"#,
        ),
        (
            r#"(let ((a 1) (i 0)) (while (< i 300) (setq a (vector a) i (1+ i))) (value< a a))"#,
            r#"nil"#,
        ),
        (
            r#"(let ((a 1) (b 1) (i 0)) (while (< i 201) (setq a (vector a) b (vector b) i (1+ i))) (condition-case e (value< a b) (error e)))"#,
            r#"(error "Maximum depth exceeded in comparison")"#,
        ),
        (
            r#"(let ((a 1) (b 2) (i 0)) (while (< i 2000) (setq a (vector a) b (vector b) i (1+ i))) (list (condition-case e (value< a b) (error e)) (value< 1 2)))"#,
            r#"((error "Maximum depth exceeded in comparison") t)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn mutable_objects_use_live_state_and_distinct_basic_types() {
    for (source, expected) in [
        (
            r#"(list (value< (make-hash-table) (make-hash-table)) (value< (make-char-table 'x) (standard-syntax-table)) (value< (lambda (x) x) (lambda (y) y)) (value< (symbol-function 'car) (symbol-function 'cdr)))"#,
            r#"(nil nil nil nil)"#,
        ),
        (
            r#"(let ((f (symbol-function 'car)) (g (lambda (x) x))) (condition-case e (value< f g) (error (list (car e) (eq (car (cdr e)) f) (eq (car (cdr (cdr e))) g)))))"#,
            r#"(type-mismatch t t)"#,
        ),
        (
            r#"(let ((a (get-buffer-create "value-order-a")) (b (get-buffer-create "value-order-b"))) (kill-buffer a) (list (value< a b) (value< b a)))"#,
            r#"(t nil)"#,
        ),
        (
            r#"(let ((a (make-marker)) (b (make-marker))) (set-marker b 1 (get-buffer-create "value-order-live")) (list (value< a b) (value< b a) (value< (make-marker) (make-marker))))"#,
            r#"(t nil nil)"#,
        ),
        (
            r#"(let ((a (make-marker)) (b (make-marker))) (set-marker a 1 (get-buffer-create "value-order-z")) (set-marker b 1 (get-buffer-create "value-order-a")) (list (value< b a) (value< a b)))"#,
            r#"(t nil)"#,
        ),
        (
            r#"(let ((a (make-marker)) (b (make-marker)) (z (get-buffer-create "value-order-z")) (y (get-buffer-create "value-order-a"))) (set-buffer y) (insert "abcd") (set-marker a 1 z) (set-marker b 4 y) (list (value< a b) (value< b a)))"#,
            r#"(nil t)"#,
        ),
    ] {
        check(source, expected);
    }
}
