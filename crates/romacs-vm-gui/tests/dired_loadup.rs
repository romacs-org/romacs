//! Dired in a `--loadup` session is GNU's dired.el: `m` marks a file and
//! `C` copies it through `dired-mode-map`, and the copy's line joins the
//! read-only listing. Compared with pinned GNU -Q --batch on the same form.
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/startup.rs"] mod startup;
#[path = "../src/command.rs"] mod command;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/native.rs"] mod native;
#[path = "../src/echo.rs"] mod echo;
use romacs_vm::Vm;

#[allow(dead_code)]
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

/// The bench's 3.2/3.3 steps, with dired.el loaded first as eshell, vc or
/// an init loads it: in a directory's Dired buffer, `m` on
/// README.md, then `C` to README.copy.md (the prompt answered by
/// `read-file-name`), as commands the buffer's keys reach. GNU's map
/// suppresses self-insertion, so a letter it binds (`y`) runs its command.
const FORM: &str = r#"(let* ((dir (file-name-as-directory (make-temp-file "dired-copy" t)))
       (source (expand-file-name "README.md" dir))
       (target (expand-file-name "README.copy.md" dir))
       (reader (symbol-function 'read-file-name)))
  (unwind-protect
      (progn
        (require 'dired)
        (with-temp-file source (insert "tally\n"))
        (with-current-buffer (dired-noselect dir)
          (dired-goto-file source)
          (call-interactively (key-binding "m"))
          (let ((marked (dired-get-marked-files t)))
            (fset 'read-file-name (lambda (&rest _) target))
            (call-interactively (key-binding "C"))
            (list (key-binding "m") (key-binding "C") (key-binding "y")
                  (substring-no-properties (substitute-command-keys "\\[revert-buffer]"))
                  marked
                  (with-temp-buffer (insert-file-contents target) (buffer-string))
                  (and (dired-goto-file target) t)
                  buffer-read-only))))
    (fset 'read-file-name reader)
    (delete-directory dir t)))"#;

#[test]
fn dired_marks_and_copies_through_gnu_dired_mode_map() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {FORM}))")])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert_eq!(gnu, r#"(dired-mark dired-do-copy dired-show-file-type "g" ("README.md") "tally
" t t)"#);
    let mine = std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        let started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        vm.eval_source("(setq inhibit-startup-screen t)").unwrap();
        startup::finish_loadup(&mut vm, &started).unwrap();
        vm.eval_source(&format!("(prin1-to-string {FORM})")).unwrap().display().unwrap()
    }).unwrap().join().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}
