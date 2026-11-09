//! `deactivate-mark' is automatically buffer-local (keyboard.c:14043), so a
//! change in another buffer leaves this buffer's region alone. The expected
//! answers are pinned GNU's.
use romacs_vm::Vm;

fn eval(source: &str) -> String {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn setting_it_makes_it_local() {
    assert_eq!(eval("(progn (setq deactivate-mark nil) (list (local-variable-p 'deactivate-mark) deactivate-mark))"), "(t nil)");
    assert_eq!(eval("(local-variable-if-set-p 'deactivate-mark)"), "t");
}

#[test]
fn a_change_marks_only_its_own_buffer() {
    assert_eq!(
        eval("(progn (setq deactivate-mark nil)
                (list (with-temp-buffer (insert \"x\") (list deactivate-mark (local-variable-p 'deactivate-mark)))
                      deactivate-mark (default-value 'deactivate-mark)))"),
        "((t t) nil nil)");
    assert_eq!(
        eval("(let ((other (get-buffer-create \"o\")))
                (with-current-buffer other (insert \"y\"))
                (list deactivate-mark (default-value 'deactivate-mark) (buffer-local-value 'deactivate-mark other)))"),
        "(nil nil t)");
}

#[test]
fn a_let_keeps_the_region_of_its_buffer() {
    assert_eq!(
        eval("(progn (setq deactivate-mark nil)
                (with-temp-buffer
                  (list (let (deactivate-mark) (insert \"z\") deactivate-mark)
                        deactivate-mark (default-value 'deactivate-mark))))"),
        "(t nil nil)");
}

#[test]
fn a_new_buffer_reads_the_default() {
    assert_eq!(
        eval("(progn (set-default 'deactivate-mark 7)
                (with-temp-buffer (list deactivate-mark (local-variable-p 'deactivate-mark))))"),
        "(7 nil)");
}

#[test]
fn a_change_with_hooks_inhibited_leaves_it() {
    assert_eq!(
        eval("(with-temp-buffer (let ((inhibit-modification-hooks t)) (insert \"z\")) deactivate-mark)"),
        "nil");
}

/// An inherited t is not this buffer's: a change still makes its own binding.
#[test]
fn an_inherited_t_still_makes_a_local() {
    assert_eq!(
        eval("(progn (set-default 'deactivate-mark t)
                (with-temp-buffer
                  (insert \"x\")
                  (set-default 'deactivate-mark nil)
                  (list deactivate-mark (local-variable-p 'deactivate-mark))))"),
        "(t t)");
}

/// A write of t over t still tells a watcher.
#[test]
fn a_watcher_sees_every_change() {
    assert_eq!(
        eval("(let ((n 0))
                (add-variable-watcher 'deactivate-mark (lambda (&rest _) (setq n (1+ n))))
                (with-temp-buffer (insert \"x\") (insert \"y\"))
                n)"),
        "2");
}
