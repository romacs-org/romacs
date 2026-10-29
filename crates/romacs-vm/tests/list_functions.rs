//! GNU `(lambda ...)' list functions: `defalias' stores the list as given
//! (data.c Fdefalias -> Ffset) and funcall_general calls it through
//! funcall_lambda with dynamic binding (eval.c:3172). Expected answers are
//! pinned GNU 32.0.50's (`emacs -Q --batch`).
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap_or_else(|e| format!("VM-ERROR {e}")),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

#[test]
fn a_lambda_list_is_stored_and_called_with_dynamic_binding() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    for (source, expected) in [
        ("(defalias 'lf-a '(lambda (x) (interactive \"P\") (list x lf-dyn)))", "lf-a"),
        ("(defvar lf-dyn 1)", "lf-dyn"),
        ("(funcall 'lf-a 2)", "(2 1)"),
        ("(let ((lf-dyn 5)) (lf-a 3))", "(3 5)"),
        ("(eq (symbol-function 'lf-a) (symbol-function 'lf-a))", "t"),
        ("(car (symbol-function 'lf-a))", "lambda"),
        ("(apply '(lambda (&rest r) r) 1 '(2 3))", "(1 2 3)"),
        ("(mapcar '(lambda (x) (* x x)) '(1 2 3))", "(1 4 9)"),
        // The list body is dynamic code even inside lexical code.
        ("(funcall (let ((f '(lambda () lf-dyn))) f))", "1"),
        ("(condition-case e (funcall '(lambda)) (error e))", "(invalid-function (lambda))"),
        ("(condition-case e (funcall '(lambda . 5)) (error e))", "(invalid-function (lambda . 5))"),
    ] {
        assert_eq!(eval(&mut vm, source), expected, "{source}");
    }
}
