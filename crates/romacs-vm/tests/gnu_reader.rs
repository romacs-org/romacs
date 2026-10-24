use romacs_vm::Vm;

#[test]
fn gnu_modifier_escapes() {
    let mut vm = Vm::new();
    for (source, expected) in [
        (r"?\C-x", 24),
        (r"?\M-x", 134217848),
        (r"?\^X", 24),
        (r"?\s-x", 8388728),
        (r"?\H-x", 16777336),
        (r"?\S-x", 33554552),
        (r"?\A-x", 4194424),
        (r"?\C-1", 67108913),
        (r"?\C-\C-a", 67108865),
        (r"?\M-\C-x", 134217752),
        (r"?\C-?", 127),
    ] {
        let result = vm.eval_source(source).and_then(|v| v.as_integer());
        assert_eq!(result, Ok(expected), "GNU escape {source}");
    }
}

#[test]
fn gnu_string_modifiers() {
    let mut vm = Vm::new();
    for (source, expected) in [
        (r#"(aref "\C-a" 0)"#, 1),
        (r#"(aref "\C- " 0)"#, 0),
        (r#"(aref "\S-a" 0)"#, 65),
        (r#"(aref "\M-x" 0)"#, 248),
        (r#"(aref "\s-x" 1)"#, 45),
    ] {
        let result = vm.eval_source(source).and_then(|v| v.as_integer());
        assert_eq!(result, Ok(expected), "GNU string {source}");
    }
}

#[test]
fn meta_string_keeps_raw_octets() {
    let mut vm = Vm::new();
    let result = vm.eval_source(r#"(multibyte-string-p "\M-x")"#);
    assert!(
        matches!(&result, Ok(v) if *v == romacs_vm::Fixed::NIL),
        "Meta strings must retain GNU byte representation: {result:?}"
    );
}

#[test]
fn user_macros_cannot_compile_internal_annotations() {
    let mut vm = Vm::new();
    vm.eval_source("(defmacro when (&rest args) (error \"user macro executed\"))")
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vm.eval_source("'(insert-for-yank)").map(|_| ())
    }));
    assert!(result.is_ok(), "interning GNU input must not panic");
    assert!(
        result.unwrap().is_ok(),
        "internal compilation must not run user macros"
    );
}

#[test]
fn declarations_in_gnu_function_and_macro_bodies() {
    let mut vm = Vm::new();
    let result = vm
        .eval_source(
            "(defun s106-f (x) \"doc\" (declare (pure t) (side-effect-free t)) (+ x 1)) (s106-f 4)",
        )
        .and_then(|v| v.as_integer());
    assert_eq!(result, Ok(5), "function declaration is not a function call");
    let result = vm
        .eval_source("(defmacro s106-m (x) (declare (indent 1) (debug t)) (list '1+ x)) (s106-m 4)")
        .and_then(|v| v.as_integer());
    assert_eq!(result, Ok(5), "macro declaration is not a function call");
}

#[test]
fn gnu_lambda_macro_does_not_reexpand_function_operand() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .args(["--source"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"(defmacro lambda (&rest cdr) (list 'function (cons 'lambda cdr))) (funcall (lambda (x) (+ x 1)) 4)").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "GNU lambda macro must not overflow: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "5");
}

#[test]
fn gnu_backquote_evaluation() {
    let mut vm = Vm::new();
    let result = vm
        .eval_source("`(a ,(+ 1 1) ,@(list 3 4) . z)")
        .and_then(|v| v.display());
    assert_eq!(
        result,
        Ok("(a 2 3 4 . z)".into()),
        "backquote evaluates comma and splice"
    );
}
