use romacs_vm::Vm;

#[test]
fn plist_member_gnu_contract() {
    for (source, expected) in [
        ("(plist-member '(a 1 b nil) 'b)", "(b nil)"),
        ("(let ((p (list 'a 1 'b 2))) (eq (plist-member p 'a) p))", "t"),
        ("(list (plist-member '(a) 'a) (plist-member '(a) 'z) (plist-member nil 'a))", "((a) nil nil)"),
        ("(condition-case e (plist-member '(a . 1) 'z) (error e))", "(wrong-type-argument plistp (a . 1))"),
        ("(plist-member '(a . 1) 'a)", "(a . 1)"),
        ("(plist-member '(\"a\" 1 \"b\" 2) (copy-sequence \"b\") 'equal)", "(\"b\" 2)"),
        ("(let ((p (list 'a 1 'b 2)) calls) (list (plist-member p 'b (lambda (x y) (setq calls (cons (list x y) calls)) (garbage-collect) (eq x y))) (nreverse calls)))", "((b 2) ((a b) (b b)))"),
        ("(let ((p (list 'a 1 'b 2))) (plist-member p 'c (lambda (x y) (if (eq x 'a) (setcar (cdr (cdr p)) 'c)) (garbage-collect) (eq x y))))", "(c 2)"),
        ("(catch 'done (plist-member '(a 1) 'x (lambda (x y) (throw 'done 7))))", "7"),
        ("(let ((p (list 'a 1))) (setcdr (cdr p) p) (eq (plist-member p 'a) p))", "t"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let expression = format!("(let ((actual {source})) (if (equal actual '{expected}) t (list 'got actual)))");
        assert_eq!(vm.eval_source(&expression).unwrap().display().unwrap(), "t", "{source}");
    }
}
