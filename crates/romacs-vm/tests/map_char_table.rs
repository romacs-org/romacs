use romacs_vm::Vm;

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source =
        format!("(let ((actual {source})) (if (equal actual '{expected}) t (list 'got actual)))");
    let value = vm.eval_source(&source).unwrap();
    assert_eq!(value.display().unwrap(), "t", "{source}");
}

#[test]
fn map_char_table_callable_and_empty_boundaries() {
    for (source, expected) in [
        (
            "(list (fboundp 'map-char-table) (subrp (symbol-function 'map-char-table)) (functionp 'map-char-table) (funcall 'map-char-table 'missing (make-char-table nil)))",
            "(t t t nil)",
        ),
        (
            "(condition-case e (map-char-table 'missing 17) (error e))",
            "(wrong-type-argument char-table-p 17)",
        ),
        (
            "(let ((tab (make-char-table nil))) (aset tab 1 t) (condition-case e (map-char-table 'missing tab) (error e)))",
            "(void-function missing)",
        ),
        (
            "(let ((tab (make-char-table nil)) (calls nil)) (aset tab 65 t) (list (map-char-table (lambda (k v) (setq calls (cons (cons k v) calls)) 'ignored) tab) calls))",
            "(nil ((65 . t)))",
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn map_char_table_ranges_defaults_and_parents() {
    for (source, expected) in [
        (
            "(let ((tab (make-char-table nil)) out) (aset tab 65 (list 1)) (aset tab 66 (list 1)) (map-char-table (lambda (k v) (setq out (cons (list (if (consp k) (cons (car k) (cdr k)) k) v) out))) tab) (nreverse out))",
            "((65 (1)) (66 (1)))",
        ),
        (
            "(let ((tab (make-char-table nil)) (value (list 1)) out) (aset tab 65 value) (aset tab 66 value) (map-char-table (lambda (k v) (setq out (cons (list (if (consp k) (cons (car k) (cdr k)) k) v) out))) tab) (nreverse out))",
            "(((65 . 66) (1)))",
        ),
        (
            "(let ((tab (make-char-table nil 'd)) out) (map-char-table (lambda (k v) (setq out (cons (cons (cons (car k) (cdr k)) v) out))) tab) out)",
            "(((0 . 4194303) . d))",
        ),
        (
            "(let ((tab (make-char-table nil 'd)) (parent (make-char-table nil 'p)) out) (set-char-table-parent tab parent) (aset tab 65 'a) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out))) tab) (nreverse out))",
            "(((0 . 64) . d) (65 . a) ((66 . 4194303) . d))",
        ),
        (
            "(let ((tab (make-char-table nil)) (parent (make-char-table nil 'p)) out) (set-char-table-range tab nil 'd) (set-char-table-parent tab parent) (aset tab 65 'a) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out))) tab) (nreverse out))",
            "(((0 . -1) . p) ((0 . 64) . d) (65 . a) ((66 . 4194303) . d))",
        ),
        (
            "(let ((tab (make-char-table nil)) out) (aset tab 4194303 'last) (map-char-table (lambda (k v) (setq out (cons (cons k v) out))) tab) out)",
            "((4194303 . last))",
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn map_char_table_shared_range_and_callback_state() {
    for (source, expected) in [
        (
            "(let ((tab (make-char-table nil)) out) (set-char-table-range tab '(1 . 2) 'a) (set-char-table-range tab '(5 . 6) 'b) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) tab) (nreverse out))",
            "(((7 . 4194303) . a) ((7 . 4194303) . b))",
        ),
        (
            "(let ((tab (make-char-table nil)) out) (set-char-table-range tab '(1 . 2) 'a) (set-char-table-range tab '(5 . 4194303) 'b) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (setcar k 'bad) (setcdr k 'bad) (garbage-collect)) tab) (nreverse out))",
            "(((bad . bad) . a) ((bad . bad) . b))",
        ),
        (
            "(let ((tab (make-char-table nil)) out) (set-char-table-range tab '(1 . 2) 'a) (set-char-table-range tab '(5 . 6) 'b) (map-keymap (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) (list 'keymap tab)) (nreverse out))",
            "(((1 . 2) . a) ((5 . 6) . b))",
        ),
        (
            "(let ((tab (make-char-table nil)) out) (aset tab 1 'a) (aset tab 2 'b) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (if (eq v 'a) (aset tab 2 'changed)) (garbage-collect)) tab) (list (nreverse out) (aref tab 2)))",
            "(((1 . a) (2 . b)) changed)",
        ),
        (
            "(let ((tab (make-char-table nil)) out) (aset tab 65537 'a) (aset tab 65539 'b) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (if (eq v 'a) (set-char-table-range tab '(65536 . 131071) 'replacement)) (garbage-collect)) tab) (list (nreverse out) (aref tab 65539)))",
            "(((65537 . a) (65539 . b)) replacement)",
        ),
        (
            "(let ((tab (make-char-table nil)) key) (set-char-table-range tab '(1 . 2) t) (list (catch 'done (map-char-table (lambda (k v) (setq key k) (setcar k 'kept) (garbage-collect) (throw 'done 7)) tab)) key (map-char-table (lambda (k v) nil) (make-char-table nil))))",
            "(7 (kept . 2) nil)",
        ),
        (
            "(let ((tab (make-char-table nil))) (aset tab 65 t) (list (condition-case e (map-char-table (lambda (k v) (error \"stop\")) tab) (error e)) (map-char-table (lambda (k v) nil) tab)))",
            "((error \"stop\") nil)",
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn unicode_property_cells_are_expanded_and_decoded() {
    for (body, expected) in [
        ("(set-char-table-range tab '(0 . 127) (string 2 1 130 2 129)) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out)) (garbage-collect)) tab) (nreverse out)", "(((0 . 1) . Lu) (2 . Ll))"),
        ("(set-char-table-range tab '(0 . 127) (string 1 0 1 2)) (map-keymap (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) (list 'keymap tab)) (nreverse out)", "((0 . 1) (1 . 2))"),
        ("(set-char-table-range tab '(128 . 255) (string 1 0 1 2)) (let ((child (make-char-table nil))) (set-char-table-range child '(0 . 127) 'own) (set-char-table-parent child tab) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out)) (garbage-collect)) child)) (nreverse out)", "(((0 . 127) . own) (128 . Lu) (129 . Ll))"),
        ("(set-char-table-range tab '(0 . 127) (string 1 0 1 2)) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) tab) (nreverse out)", "((0 . Lu) (1 . Ll))"),
        (
            "(aset tab 65 1) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) tab) out",
            "((65 . Lu))",
        ),
        (
            "(set-char-table-range tab '(128 . 255) (string 1 0 1 2)) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) tab) (nreverse out)",
            "((128 . Lu) (129 . Ll))",
        ),
        (
            "(set-char-table-range tab '(128 . 255) (string 2 1 130 2 129)) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out)) (garbage-collect)) tab) (nreverse out)",
            "(((128 . 129) . Lu) (130 . Ll))",
        ),
        (
            "(aset tab 65 1) (aset tab 66 2) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (set-char-table-extra-slot tab 1 nil) (garbage-collect)) tab) (nreverse out)",
            "((65 . Lu) (66 . Ll))",
        ),
        (
            "(aset tab 65 1) (aset tab 66 2) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (set-char-table-extra-slot tab 4 [nil new-a new-b]) (garbage-collect)) tab) (nreverse out)",
            "((65 . Lu) (66 . new-b))",
        ),
        (
            "(aset tab 65 1) (aset tab 66 2) (let ((child (make-char-table nil))) (set-char-table-parent child tab) (map-char-table (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) child)) (nreverse out)",
            "((65 . Lu) (66 . Ll))",
        ),
        (
            "(set-char-table-range tab t 1) (let ((child (make-char-table nil))) (set-char-table-parent child tab) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out))) child)) (nreverse out)",
            "(((0 . 4194303) . 1))",
        ),
        (
            "(set-char-table-range tab '(128 . 255) (string 2 0 130)) (map-char-table (lambda (k v) (setq out (cons (cons (if (consp k) (cons (car k) (cdr k)) k) v) out)) (garbage-collect)) tab) (nreverse out)",
            "(((128 . 129)))",
        ),
        (
            "(set-char-table-range tab '(128 . 255) (string 1 0 1 2)) (map-keymap (lambda (k v) (setq out (cons (cons k v) out)) (garbage-collect)) (list 'keymap tab)) (nreverse out)",
            "((128 . 1) (129 . 2))",
        ),
    ] {
        let source = format!(
            "(progn (put 'char-code-property-table 'char-table-extra-slots 5) (let ((tab (make-char-table 'char-code-property-table)) out) (set-char-table-extra-slot tab 1 0) (set-char-table-extra-slot tab 4 [nil Lu Ll]) {body}))"
        );
        check(&source, expected);
    }
}
