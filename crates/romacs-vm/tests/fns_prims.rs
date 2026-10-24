use romacs_vm::Vm;

fn shown(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(v) => v.display().unwrap(),
        Err(e) => format!("ERROR: {e}"),
    }
}

#[test]
fn inventory() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for name in ["elt", "copy-alist", "rassoc", "string-distance", "mapconcat", "sort", "compare-strings", "string-version-lessp", "string-collate-lessp", "string-collate-equalp", "value<", "nthcdr", "aref", "string-search", "safe-length", "length<", "length=", "length>", "regexp-quote", "eql", "funcall", "mapcar"] {
        println!("{name}: {}", shown(&mut vm, &format!("(fboundp '{name})")));
    }
    println!("funcall: {}", shown(&mut vm, "(funcall #'car '(7))"));
    println!("mapcar: {}", shown(&mut vm, "(mapcar #'car '((7) (8)))"));
}

#[test]
fn elt_contract() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(elt '(a b c) 1)", "b"),
        ("(elt '(a b c) -1)", "a"),
        ("(elt nil 9)", "nil"),
        ("(elt '(a) 2305843009213693951)", "nil"),
        ("(elt [a b] 1)", "b"),
        ("(elt \"aλ雪😀z\" 2)", "38634"),
        ("(elt (unibyte-string 255 128 97) 1)", "128"),
        ("(elt (string-to-multibyte (unibyte-string 255 128 97)) 1)", "4194176"),
        ("(let ((ct (make-char-table 'test 'default))) (set-char-table-range ct 955 'lambda) (elt ct 955))", "lambda"),
        ("(condition-case e (elt \"a\" 1) (error e))", "(args-out-of-range \"a\" 1)"),
        ("(condition-case e (elt [] 0) (error e))", "(args-out-of-range [] 0)"),
        ("(condition-case e (elt \"\" -1) (error e))", "(args-out-of-range \"\" -1)"),
        ("(condition-case e (elt [a] 2305843009213693951) (error e))", "(args-out-of-range [a] 2305843009213693951)"),
        ("(condition-case e (elt [a] 'bad) (error e))", "(wrong-type-argument fixnump bad)"),
        ("(condition-case e (elt 'bad 'bad-index) (error e))", "(wrong-type-argument sequencep bad)"),
        ("(condition-case e (elt '(a . b) 1) (error e))", "(wrong-type-argument listp b)"),
        ("(condition-case e (elt (make-char-table 'test) -1) (error (list (car e) (car (cdr e)))))", "(wrong-type-argument characterp)"),
        ("(elt '(a) (car (read-from-string \"#x2000000000000000\")))", "nil"),
        ("(elt '(a) (car (read-from-string \"#x-2000000000000001\")))", "a"),
        ("(let ((xs (list 'a 'b 'c))) (setcdr (cdr (cdr xs)) xs) (elt xs (car (read-from-string \"#x2000000000000000\"))))", "c"),
        ("(condition-case e (elt [a] (car (read-from-string \"#x2000000000000000\"))) (error (list (car e) (car (cdr e)))))", "(wrong-type-argument fixnump)"),
        // GNU 32.0.50 (45b0f769): syntax tables are character-table sequences.
        ("(condition-case e (elt (standard-syntax-table) 97) (error (list (car e) (car (cdr e)))))", "(2)"),
        ("(funcall #'elt (standard-syntax-table) 97)", "(2)"),
        ("(funcall #'elt [a b] 1)", "b"),
        ("(let ((xs (list 'a 'b))) (setcdr (cdr xs) xs) (elt xs 2305843009213693951))", "b"),
    ] {
        assert_eq!(shown(&mut vm, source), expected, "elt contract: {source}");
    }
}

#[test]
fn copy_alist_contract() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(copy-alist '((a . 1) (b . 2)))", "((a . 1) (b . 2))"),
        ("(copy-alist nil)", "nil"),
        ("(let* ((entry (cons (list 'key) (list 'value))) (atom (vector 1)) (xs (list entry atom)) (ys (copy-alist xs))) (list (eq xs ys) (eq entry (car ys)) (eq (car entry) (car (car ys))) (eq (cdr entry) (cdr (car ys))) (eq atom (car (cdr ys)))))", "(nil nil t t t)"),
        ("(condition-case e (copy-alist '(a . b)) (error e))", "(wrong-type-argument listp b)"),
        ("(condition-case e (copy-alist [a]) (error e))", "(wrong-type-argument listp [a])"),
        ("(let ((xs (list 'a 'b))) (setcdr (cdr xs) xs) (condition-case e (copy-alist xs) (circular-list (list (car e) (eq (car (cdr e)) (cdr xs))))))", "(circular-list t)"),
        ("(funcall #'copy-alist '((a . 2)))", "((a . 2))"),
    ] { assert_eq!(shown(&mut vm, source), expected, "copy-alist contract: {source}"); }
}

#[test]
fn rassoc_contract() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(rassoc '(1 2) '((a 1 2) (b 3)))", "(a 1 2)"),
        ("(rassoc nil '(nil a (a) (b . 2)))", "(a)"),
        ("(rassoc 2 '((a . 1) (b . 2)))", "(b . 2)"),
        ("(let* ((entry (cons 'key (list 1 2))) (xs (list entry))) (eq entry (rassoc (list 1 2) xs)))", "t"),
        ("(condition-case e (rassoc 1 '((a . 2) . bad)) (error e))", "(wrong-type-argument listp ((a . 2) . bad))"),
        ("(rassoc 1 '((a . 1) . bad))", "(a . 1)"),
        ("(let ((xs (list (cons 'a 1)))) (setcdr xs xs) (condition-case e (rassoc 2 xs) (circular-list (eq (car (cdr e)) xs))))", "t"),
        ("(funcall #'rassoc 2 '((a . 2)))", "(a . 2)"),
    ] { assert_eq!(shown(&mut vm, source), expected, "rassoc contract: {source}"); }
}

#[test]
fn string_distance_contract() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(string-distance \"kitten\" \"sitting\")", "3"),
        ("(string-distance \"λ雪\" \"λ😀\")", "1"),
        ("(string-distance \"\" \"λ雪\")", "2"),
        ("(string-distance \"\" \"λ雪\" t)", "5"),
        ("(string-distance (unibyte-string 255) (string-to-multibyte (unibyte-string 255)))", "1"),
        ("(string-distance (unibyte-string 255) (unibyte-string 128))", "1"),
        ("(condition-case e (string-distance 'wrong 'also-wrong) (error e))", "(wrong-type-argument stringp wrong)"),
        ("(condition-case e (string-distance \"ok\" 'wrong) (error e))", "(wrong-type-argument stringp wrong)"),
        ("(funcall #'string-distance \"ABC\" \"abc\")", "3"),
    ] { assert_eq!(shown(&mut vm, source), expected, "string-distance contract: {source}"); }
}
