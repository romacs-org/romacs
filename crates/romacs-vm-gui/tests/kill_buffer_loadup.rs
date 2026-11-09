//! kill-buffer over GNU's window.el, as Fkill_buffer: a shown buffer with
//! no previous buffers in its window gives way through
//! `replace-buffer-in-windows' (whose `switch-to-prev-buffer' asks for the
//! frame's `buffer-list'), and a sole *scratch*, its own `other-buffer',
//! is kept. Compared with pinned GNU -Q --batch on the same form.
use romacs_vm_gui::startup;
use romacs_vm::Vm;

const FORM: &str = r#"(let* ((a (get-buffer-create "kl-a")))
  (set-window-buffer (selected-window) a)
  (set-window-prev-buffers nil nil)
  (list (kill-buffer a) (buffer-live-p a) (buffer-name (window-buffer))
        (progn (dolist (b (buffer-list))
                 (unless (or (equal (buffer-name b) "*scratch*") (minibufferp b) (eq (aref (buffer-name b) 0) ?\s))
                   (kill-buffer b)))
               (set-buffer "*scratch*")
               (kill-buffer "*scratch*"))
        (buffer-name)))"#;

#[test]
fn kill_buffer_replaces_windows_and_keeps_a_sole_scratch_as_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {FORM}))")])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert_eq!(gnu, r#"(t nil "*scratch*" nil "*scratch*")"#);
    let mine = std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        let started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        // GNU's batch shows no startup screen (command-line-1 under
        // `noninteractive'); a shown one is recorded first in `buffer-list'.
        vm.eval_source("(setq inhibit-startup-screen t)").unwrap();
        startup::finish_loadup(&mut vm, &started).unwrap();
        vm.eval_source(&format!("(prin1-to-string {FORM})")).unwrap().display().unwrap()
    }).unwrap().join().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}
