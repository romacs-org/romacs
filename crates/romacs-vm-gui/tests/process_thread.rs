//! `process-thread' through the code that reads it, in `romacs-vm-gui -Q
//! --batch' against pinned GNU's `emacs -Q --batch': the process menu, with
//! real processes of each kind. Romacs is the threadless build, so the
//! menu's Thread column is `--' where pinned GNU's is `Main' (VM2-THREAD1).
use std::process::Command;

#[test]
fn list_processes_shows_a_live_process_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let form = r#"(let ((p (make-process :name "t7" :command '("sleep" "5") :noquery t))
                        (s (make-network-process :name "t5" :server t :host "127.0.0.1" :service t :noquery t)))
       (unwind-protect
           (progn
             (list-processes)
             (with-current-buffer "*Process List*"
               (prin1 (list (eq (process-thread p) (process-thread s)) (null (process-thread p))
                            (derived-mode-p 'process-menu-mode)
                            (mapcar (lambda (entry)
                                      (let ((row (cadr entry)))
                                        (list (aref row 0) (aref row 2) (length row)
                                              (if (eq (car entry) p) (aref row 6) (substring (aref row 6) 0 16)))))
                                    (sort (copy-sequence tabulated-list-entries)
                                          (lambda (a b) (string< (aref (cadr a) 0) (aref (cadr b) 0)))))
                            (> (buffer-size) 0)))))
         (delete-process p) (delete-process s)))"#;
    let dir = std::env::temp_dir().join(format!("romacs-process-thread-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |binary: &str| {
        let output = Command::new(binary).args(["-Q", "--batch", "--eval", form]).current_dir(&dir)
            .env("HOME", &dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
            .output().unwrap();
        assert!(output.status.success(), "{binary}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(run(env!("CARGO_BIN_EXE_romacs-vm-gui")), run(&gnu));
}
