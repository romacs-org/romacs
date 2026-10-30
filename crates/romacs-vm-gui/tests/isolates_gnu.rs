//! Isolates on the full GNU Lisp chain: a batch session boots from source
//! and spawns workers that boot from source too (`:allow-source-boot t`),
//! so a worker can run `make-process` and the project commands. Covers
//! what the bare-worker tests (romacs-vm `tests/isolates.rs`) cannot: child
//! reporting with pidfds and retirement, the kill reaching a reported
//! child's group, and the project consumer's results being those of the
//! synchronous definitions.
use std::path::Path;
use std::process::Command;

fn batch(script: &str, cwd: &Path) -> (bool, String, String) {
    let scratch = std::env::temp_dir().join(format!("romacs-isolates-gnu-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["--batch", "-Q", "--eval", script])
        .current_dir(cwd)
        .env("HOME", &scratch)
        .env("XDG_CACHE_HOME", scratch.join("cache"))
        .env("XDG_CONFIG_HOME", scratch.join("config"))
        .env("ROMACS_WORLD", "off")
        .output()
        .expect("batch session");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

const PREAMBLE: &str = "(setq isolate-boot 'source isolate-pool-size 1)";

fn process_exists(pid: i64) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

#[test]
fn a_killed_workers_reported_child_is_reached_and_a_reaped_one_is_retired() {
    let script = format!("(progn {PREAMBLE}
      (let* ((iso (isolate-create :allow-source-boot t))
             (id (romacs--isolate-id iso))
             (child (isolate-await (isolate-spawn '(lambda () (process-id (make-process :name \"sleeper\" :command '(\"sleep\" \"30\") :connection-type 'pipe))) :isolate iso)))
             (reaped (isolate-await (isolate-spawn '(lambda () (let ((p (make-process :name \"quick\" :command '(\"true\") :connection-type 'pipe))) (while (process-live-p p) (accept-process-output p 0.1)) (process-id p))) :isolate iso))))
        (romacs--isolate-pump 0.3)
        (prin1 (list 'worker (romacs--isolate-pid id) 'child child 'reported (romacs--isolate-children id) 'reaped reaped
                     'outcomes (isolate-kill iso) 'live (isolate-list)))))");
    let (ok, out, err) = batch(&script, Path::new("/"));
    assert!(ok, "batch failed: {err}");
    let words: Vec<&str> = out.trim().trim_matches(|c| c == '(' || c == ')').split_whitespace().collect();
    let after = |key: &str| words.iter().position(|w| *w == key).map(|i| words[i + 1].trim_matches(|c| c == '(' || c == ')'));
    let worker: i64 = after("worker").unwrap().parse().unwrap();
    let child: i64 = after("child").unwrap().parse().unwrap();
    assert_eq!(after("reported").unwrap(), child.to_string(), "{out}");
    assert!(out.contains(&format!("outcomes (({child} . ok))")), "{out}");
    assert!(out.ends_with("live nil)\n") || out.trim().ends_with("live nil)"), "{out}");
    for _ in 0..100 {
        if !process_exists(worker) && !process_exists(child) { return; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("worker {worker} or orphan {child} survived: {out}");
}

#[test]
fn the_project_commands_give_the_synchronous_results() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    // The repository itself is the project: a git tree of several thousand files.
    let script = format!("(progn {PREAMBLE} (require 'project) (load {module:?})
      (let* ((pr (project-current nil {dir:?}))
             (sync-files (funcall (eval romacs-project--files-job t) pr (list (project-root pr))))
             (iso-files (thaw (isolate-await (romacs-project--spawn pr romacs-project--files-job pr (list (project-root pr))))))
             (sync-matches (funcall (eval romacs-project--matches-job t) \"defcustom project-\" pr))
             (iso-matches (thaw (isolate-await (romacs-project--spawn pr romacs-project--matches-job \"defcustom project-\" pr))))
             (summary (lambda (m) (list (xref-location-group (xref-item-location m)) (xref-location-line (xref-item-location m)) (substring-no-properties (xref-item-summary m))))))
        (prin1 (list 'files (length sync-files) (equal sync-files iso-files)
                     'matches (length sync-matches) (equal (mapcar summary sync-matches) (mapcar summary iso-matches))
                     'boot (isolate-await (isolate-spawn '(lambda () isolate-boot) :allow-source-boot t))))
        (isolate-kill-all)))", dir = root.join("gnu-lisp").display().to_string() + "/", module = root.join("rlisp/romacs-project.el").display().to_string());
    let (ok, out, err) = batch(&script, &root);
    assert!(ok, "batch failed: {err}");
    let out = out.trim();
    assert!(out.starts_with("(files ") && out.contains(" t matches ") && out.ends_with(" t boot source)"), "{out}\n{err}");
    let files: usize = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!(files > 1000, "{out}");
}
