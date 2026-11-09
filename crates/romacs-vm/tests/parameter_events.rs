//! An event with parameters is read by its head (keyboard.c read_key_sequence,
//! EVENT_HEAD), though `lookup-key' given one as a key reads it as a list of
//! modifiers.  The expected answers are pinned GNU's.
use romacs_vm::Vm;

fn eval(source: &str) -> String {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn a_key_sequence_looks_a_parameter_event_up_by_its_head() {
    let setup = "(use-global-map (make-sparse-keymap)) (define-key (current-global-map) [my-ev] #'ignore)";
    assert_eq!(
        eval(&format!("(progn {setup} (setq unread-command-events (list (list 'my-ev 'token))) (read-key-sequence nil))")),
        "[(my-ev token)]");
    assert_eq!(
        eval(&format!("(progn {setup} (condition-case e (lookup-key (current-global-map) [(my-ev token)]) (error e)))")),
        "(error \"Two bases given in one event\")");
}
