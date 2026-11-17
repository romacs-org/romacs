//! A buffer's locals through a world image: a world captured with buffers
//! that hold an automatic local, an explicit one, a permanent one and a
//! localized built-in Boolean and integer restores each with its value,
//! and the restored buffers keep, set, bind and kill them as before.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp_with;

const MADE: &str = "(progn
  (defvar-local bw-auto 'default)
  (defvar bw-explicit 'default)
  (defvar bw-perm 'default)
  (put 'bw-perm 'permanent-local t)
  (with-current-buffer (get-buffer-create \"bw-a\")
    (setq bw-auto 'auto-a)
    (set (make-local-variable 'bw-explicit) 'explicit-a)
    (set (make-local-variable 'bw-perm) 'perm-a)
    (set (make-local-variable 'inhibit-modification-hooks) t)
    (set (make-local-variable 'undo-limit) 1234))
  (with-current-buffer (get-buffer-create \"bw-b\")
    (setq bw-auto 'auto-b))
  t)";

const READ: &str = "(let ((a (get-buffer \"bw-a\")) (b (get-buffer \"bw-b\")))
  (list (buffer-local-value 'bw-auto a) (buffer-local-value 'bw-auto b) bw-auto
        (buffer-local-value 'bw-explicit a) bw-explicit
        (with-current-buffer a (list bw-perm inhibit-modification-hooks undo-limit
                                     (local-variable-p 'bw-explicit) (local-variable-p 'bw-perm)))
        inhibit-modification-hooks
        (with-current-buffer a (let ((bw-auto 'let-a)) (list bw-auto (buffer-local-value 'bw-auto b))))
        (with-current-buffer a (setq bw-explicit 'set-a) (kill-all-local-variables)
                             (list bw-auto bw-explicit bw-perm (local-variable-p 'bw-perm)))
        (buffer-local-value 'bw-auto b)))";

#[test]
fn buffer_locals_survive_a_world_image() {
    let printed = std::thread::Builder::new().stack_size(256 << 20).spawn(|| {
        let mut vm = gnu_lisp_with(MADE);
        vm.eval_source(READ).and_then(|v| v.display()).unwrap()
    }).unwrap().join().unwrap();
    assert_eq!(printed, "(auto-a auto-b default explicit-a default (perm-a t 1234 t t) nil (let-a auto-b) (default default perm-a t) auto-b)");
}
