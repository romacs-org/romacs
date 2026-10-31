//! Isolates on the full GNU Lisp chain: a batch session boots from source
//! and spawns workers that boot from source too (`:allow-source-boot t`),
//! so a worker can run `make-process` and the project commands. Covers
//! what the bare-worker tests (romacs-vm `tests/isolates.rs`) cannot: child
//! reporting with pidfds and retirement, the kill reaching a reported
//! child's group, and the project consumer's results being those of the
//! synchronous definitions.
use std::path::{Path, PathBuf};
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

/// A git project of several thousand files for the project commands: the
/// gnu-lisp tree copied into a fresh scratch directory and committed there
/// with a fixed author and date, in a repository of its own. The tests then
/// hold their project wherever the tree they run from lives: the gate tests
/// a frozen copy of the tree that is no git checkout.
fn fixture_project(name: &str) -> (PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("romacs-isolates-gnu-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let project = scratch.join("project");
    copy_tree(&root.join("gnu-lisp"), &project);
    let git = |args: &[&str]| {
        let output = Command::new("git").arg("-C").arg(&project).args(args)
            .env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &scratch).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Fixture Author").env("GIT_AUTHOR_EMAIL", "author@example.org")
            .env("GIT_COMMITTER_NAME", "Fixture Author").env("GIT_COMMITTER_EMAIL", "author@example.org")
            .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00+0000").env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00+0000")
            .output().expect("git");
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "The gnu-lisp tree"]);
    (scratch, project)
}

/// FROM's files and directories copied under TO.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
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
fn a_worker_killed_from_outside_has_its_reported_child_reached() {
    // Sol r1 S7: a death the parent did not cause still signals the groups
    // of the children the worker reported.
    let script = format!("(progn {PREAMBLE}
      (let* ((iso (isolate-create :allow-source-boot t))
             (id (romacs--isolate-id iso))
             (child (isolate-await (isolate-spawn '(lambda () (process-id (make-process :name \"sleeper\" :command '(\"sleep\" \"30\") :connection-type 'pipe))) :isolate iso)))
             (events nil))
        (push (lambda (_iso event data) (push (list event (car (cdr data))) events)) isolate-error-functions)
        (romacs--isolate-pump 0.3)
        (romacs--isolate-signal id 9)
        (let ((end (+ (float-time) 5))) (while (and (null events) (< (float-time) end)) (romacs--isolate-pump 0.1)))
        (prin1 (list 'worker (romacs--isolate-pid id) 'child child 'events events 'live (isolate-list)))))");
    let (ok, out, err) = batch(&script, Path::new("/"));
    assert!(ok, "batch failed: {err}");
    let words: Vec<&str> = out.trim().trim_matches(|c| c == '(' || c == ')').split_whitespace().collect();
    let after = |key: &str| words.iter().position(|w| *w == key).map(|i| words[i + 1].trim_matches(|c| c == '(' || c == ')'));
    let child: i64 = after("child").unwrap().parse().unwrap();
    assert!(out.contains(&format!("events ((isolate-died (({child} . ok))))")), "{out}");
    for _ in 0..100 {
        if !process_exists(child) { return; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("orphan {child} survived the worker's death: {out}");
}

#[test]
fn the_project_commands_give_the_synchronous_results() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let (scratch, project) = fixture_project("sync");
    let script = format!("(progn {PREAMBLE} (require 'project) (load {module:?}) (setq romacs-project-isolate 'source)
      (let* ((pr (project-current nil {dir:?}))
             ;; No worker is ready until one is warmed and has prepared.
             (before (romacs-project--worker pr))
             (worker (romacs-project--warm pr))
             (preparing (romacs-project--worker pr))
             (sync-files (funcall (eval romacs-project--files-job t) pr (list (project-root pr))))
             (iso-files (thaw (isolate-await (romacs-project--spawn pr worker romacs-project--files-job pr (list (project-root pr))))))
             (ready (eq (romacs-project--worker pr) worker))
             (sync-matches (funcall (eval romacs-project--matches-job t) \"defcustom project-\" pr))
             (iso-matches (thaw (isolate-await (romacs-project--spawn pr worker romacs-project--matches-job \"defcustom project-\" pr))))
             (summary (lambda (m) (list (xref-location-group (xref-item-location m)) (xref-location-line (xref-item-location m)) (substring-no-properties (xref-item-summary m))))))
        (prin1 (list 'files (length sync-files) (equal sync-files iso-files)
                     'matches (length sync-matches) (equal (mapcar summary sync-matches) (mapcar summary iso-matches))
                     'ready (list before preparing ready (romacs-project--warm pr))
                     'boot (isolate-await (isolate-spawn '(lambda () isolate-boot) :allow-source-boot t))))
        (isolate-kill-all)))", dir = project.display().to_string() + "/", module = root.join("rlisp/romacs-project.el").display().to_string());
    let (ok, out, err) = batch(&script, &project);
    assert!(ok, "batch failed: {err}");
    let out = out.trim();
    assert!(out.starts_with("(files ") && out.contains(" t matches ") && out.ends_with(" t ready (nil nil t nil) boot source)"), "{out}\n{err}");
    let files: usize = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!(files > 1000, "{out}");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_ready_worker_that_will_not_take_the_job_gives_way_to_the_original() {
    // Sol r6: readiness is checked before submission; a full queue or a
    // process gone since the check runs GNU's definition, not an error.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let (scratch, project) = fixture_project("ready");
    // The commands remember the project; the list stays in the private HOME.
    let script = format!("(progn {PREAMBLE} (require 'project) (load {module:?}) (setq romacs-project-isolate 'source)
      (setq project-list-file (expand-file-name \"projects.eld\" (getenv \"HOME\")))
      (let* ((pr (project-current nil {dir:?}))
             (worker (romacs-project--warm pr))
             (id (romacs--isolate-id worker))
             (original-file (lambda (&optional all) (list 'original-file all)))
             (original-regexp (lambda (regexp) (list 'original-regexp regexp))))
        (isolate-await (isolate-spawn '(lambda () t) :isolate worker))
        (prin1 (list 'ready (eq (romacs-project--worker pr) worker)
                     'full (let ((isolate-queue-limit 0))
                             (list (romacs-project-find-file original-file) (romacs-project-find-regexp original-regexp \"x\")))
                     'still-ready (eq (romacs-project--worker pr) worker)
                     'dead (progn (romacs--isolate-signal id 9)
                                  (while (null (romacs--isolate-reap id)) (romacs--isolate-wait 0.01))
                                  (list (romacs-project-find-file original-file) (romacs-project-find-regexp original-regexp \"x\")))
                     'live (progn (romacs--isolate-pump 0.5) (isolate-list))))
        (isolate-kill-all)))", dir = project.display().to_string() + "/", module = root.join("rlisp/romacs-project.el").display().to_string());
    let (ok, out, err) = batch(&script, &project);
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(ready t full ((original-file nil) (original-regexp \"x\")) still-ready t dead ((original-file nil) (original-regexp \"x\")) live nil)", "{err}");
    let _ = std::fs::remove_dir_all(&scratch);
}
