use romacs_vm::Vm;

#[test]
fn character_space_and_tab_end_before_adjacent_forms() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("'(seq (? ?-) (? ?. digit))", "(seq (32 45) (32 46 digit))"),
        ("(list ? 9 ?\t9)", "(32 9 9 9)"),
        ("(list (car (read-from-string \"? x\")) (cdr (read-from-string \"? x\")))", "(32 2)"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}

// lread.c read_char_literal: a literal may be followed by any control
// character or one of "';()[]#?`,. (characters.el writes `(?ꪀ.  ?ꪯ)').
#[test]
fn character_literals_end_before_gnu_terminators() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(car (read-from-string \"(?\u{aa80}.  ?\u{aaaf})\"))", "(43648 . 43695)"),
        ("(car (read-from-string \"(?a?b)\"))", "(97 98)"),
        ("(car (read-from-string \"(?a#'b)\"))", "(97 (function b))"),
        ("(car (read-from-string (string ?\\( ?? ?a 12 ?\\))))", "(97)"),
        ("(condition-case e (read-from-string \"?ab\") (invalid-read-syntax 'refused))", "refused"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}
