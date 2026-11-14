//! Isolates on the full GNU Lisp chain: a batch session boots from source
//! and spawns workers that boot from source too (`:allow-source-boot t`),
//! so a worker can run `make-process` and the project commands. Covers
//! what the bare-worker tests (romacs-vm `tests/isolates.rs`) cannot: child
//! reporting with pidfds and retirement, the kill reaching a reported
//! child's group, and the project consumer's results being those of the
//! synchronous definitions.
#[path = "support/installed.rs"]
mod installed;
use std::path::{Path, PathBuf};
use std::process::Command;

fn batch(script: &str, cwd: &Path) -> (bool, String, String) {
    let scratch = std::env::temp_dir().join(format!("romacs-isolates-gnu-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let output = Command::new(installed::romacs())
        .args(["--batch", "-Q", "--eval", script])
        .current_dir(cwd)
        .env("HOME", &scratch)
        .env("XDG_CACHE_HOME", scratch.join("cache"))
        .env("XDG_CONFIG_HOME", scratch.join("config"))
        .output()
        .expect("batch session");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

/// A git project of several thousand files for the project commands: the
/// lisp/gnu tree copied into a fresh scratch directory and committed there
/// with a fixed author and date, in a repository of its own. The tests then
/// hold their project wherever the tree they run from lives: the gate tests
/// a frozen copy of the tree that is no git checkout.
fn fixture_project(name: &str) -> (PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("romacs-isolates-gnu-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let project = scratch.join("project");
    copy_tree(&root.join("lisp/gnu"), &project);
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
    git(&["commit", "-q", "-m", "The lisp/gnu tree"]);
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
    // The worker ends on SIGTERM and its own shutdown SIGHUPs the child's
    // group first, so the parent's signal may find it gone; the parent's
    // reach after a SIGKILL is the next test's.
    assert!(out.contains(&format!("outcomes (({child} . ok))")) || out.contains(&format!("outcomes (({child} . gone))")), "{out}");
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
        (isolate-kill-all)))", dir = project.display().to_string() + "/", module = root.join("lisp/romacs/romacs-project.el").display().to_string());
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
      ;; The worker path, whatever the fixture's size.
      (setq romacs-project-inline-threshold 0)
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
        (isolate-kill-all)))", dir = project.display().to_string() + "/", module = root.join("lisp/romacs/romacs-project.el").display().to_string());
    let (ok, out, err) = batch(&script, &project);
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(ready t full ((original-file nil) (original-regexp \"x\")) still-ready t dead ((original-file nil) (original-regexp \"x\")) live nil)", "{err}");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_small_project_enumerates_in_the_session_and_its_time_is_recorded() {
    // Within `romacs-project-inline-threshold' (the estimate: the last
    // enumeration, else the Git index's size at the measured rate) the
    // files are listed in the session: GNU's definition runs, and its
    // `project-files' call is timed for the next decision. (The threshold
    // is set, not let-bound: it is special only once the module loads.)
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let (scratch, project) = fixture_project("inline");
    let script = format!("(progn {PREAMBLE} (require 'project) (load {module:?}) (setq romacs-project-isolate 'source)
      (setq project-list-file (expand-file-name \"projects.eld\" (getenv \"HOME\")))
      (let* ((pr (project-current nil {dir:?}))
             (root (project-root pr))
             (original (lambda (&optional all) (list 'original all (length (project-files (project-current nil {dir:?})))))))
        (prin1 (list 'index (> (romacs-project--index-bytes root) 0)
                     'small (progn (setq romacs-project-inline-threshold 10) (romacs-project--inline-p pr))
                     'none (progn (setq romacs-project-inline-threshold 0) (romacs-project--inline-p pr))
                     'unrecorded (gethash root romacs-project--enumerations)
                     'inline (progn (setq romacs-project-inline-threshold 10)
                                     (car (romacs-project-find-file original)))
                     'recorded (numberp (gethash root romacs-project--enumerations))
                     'slow (progn (puthash root 10.0 romacs-project--enumerations)
                                  (setq romacs-project-inline-threshold 1) (romacs-project--inline-p pr))
                     'live (isolate-list)))
        (isolate-kill-all)))", dir = project.display().to_string() + "/", module = root.join("lisp/romacs/romacs-project.el").display().to_string());
    let (ok, out, err) = batch(&script, &project);
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(index t small t none nil unrecorded nil inline original recorded t slow nil live nil)", "{err}");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_worker_search_leaves_its_xref_window_selected_as_gnus_does() {
    // GNU's synchronous `project-find-regexp' ends in `pop-to-buffer': the
    // xref window is selected and *xref* heads the buffer list, ahead of
    // the buffer searched from, and *xref* is current, so the keys read
    // next are its. The worker's continuation ends the same way.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let (scratch, project) = fixture_project("xref");
    let script = format!("(progn {PREAMBLE} (require 'project) (load {module:?}) (setq romacs-project-isolate 'source)
      (setq project-list-file (expand-file-name \"projects.eld\" (getenv \"HOME\")))
      (let* ((pr (project-current nil {dir:?}))
             (worker (romacs-project--warm pr))
             (origin (find-file-noselect (expand-file-name \"subr.el\" {dir:?})))
             (tries 600))
        (isolate-await (isolate-spawn '(lambda () t) :isolate worker))
        (switch-to-buffer origin)
        (let ((window (selected-window)))
          (prin1 (list 'returned (romacs-project-find-regexp (lambda (_) 'original) \"defcustom project-\")))
          (while (and (not (get-buffer \"*xref*\")) (> tries 0))
            (romacs--isolate-pump 0.1)
            (setq tries (1- tries)))
          (let ((xref (get-buffer \"*xref*\")))
            (prin1 (list 'selected (eq (window-buffer (selected-window)) xref) (eq (current-buffer) xref)
                         'moved (not (eq (selected-window) window))
                         'order (eq (nth 0 (buffer-list)) xref) (eq (nth 1 (buffer-list)) origin)))))
        (isolate-kill-all)))", dir = project.display().to_string() + "/", module = root.join("lisp/romacs/romacs-project.el").display().to_string());
    let (ok, out, err) = batch(&script, &project);
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(returned nil)(selected t t moved t order t t)", "{err}");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_snapshot_carries_the_text_and_its_worker_buffer_refuses_text_edits() {
    // Design §3.6: the accessible text (raw bytes included), its start, point,
    // the mode and the recorded locals; the worker's buffer is read-only as
    // GNU's are (properties change under `inhibit-read-only', as Font Lock
    // changes them), and a text change even then fails the body; staleness
    // follows text, mode, locals and generation, not property changes.
    let script = format!("(progn {PREAMBLE}
      (let (s1 s2 s3 s4 current)
        (with-current-buffer (get-buffer-create \"snap\")
          (insert \"head\\n(defun f () \\\"\\u03bb\\\")\\n\" (unibyte-string 255 128) \"\\ntail\\n\")
          (emacs-lisp-mode)
          (setq-local fill-column 42)
          (narrow-to-region 6 (point-max))
          (goto-char 8)
          (setq s1 (buffer-snapshot nil :locals '(fill-column tab-width)))
          (put-text-property 6 8 'face 'bold)
          (push (buffer-snapshot-current-p s1) current)
          (let ((inhibit-read-only t)) (goto-char (point-max)) (insert \"x\"))
          (push (buffer-snapshot-current-p s1) current)
          (setq s2 (buffer-snapshot))
          (emacs-lisp-mode)
          (push (buffer-snapshot-current-p s2) current)
          (setq-local fill-column 42)
          (setq s3 (buffer-snapshot nil :locals '(fill-column)))
          (setq-local fill-column 50)
          (push (buffer-snapshot-current-p s3) current)
          (setq s4 (buffer-snapshot))
          (buffer-snapshot-invalidate)
          (push (buffer-snapshot-current-p s4) current))
        (let ((got (isolate-await (isolate-spawn '(lambda (s)
                                                     (with-snapshot-buffer s
                                                       (list (buffer-string) major-mode (point) fill-column
                                                             (local-variable-p 'tab-width) buffer-read-only
                                                             (condition-case nil (progn (insert \"x\") 'inserted) (buffer-read-only 'refused))
                                                             (condition-case nil (progn (put-text-property 1 3 'face 'bold) 'put) (buffer-read-only 'refused))
                                                             (let ((inhibit-read-only t)) (put-text-property 1 3 'face 'bold) (get-text-property 1 'face)))))
                                                  s1 :allow-source-boot t)))
              (edited (condition-case err
                          (isolate-await (isolate-spawn '(lambda (s) (with-snapshot-buffer s (let ((inhibit-read-only t)) (insert \"y\")) t)) s1))
                        (error (error-message-string err)))))
          (prin1 (list 'same (equal (car got) (snapshot-string s1))
                       'raw (and (memq #x3fffff (string-to-list (car got))) (memq #x3fff80 (string-to-list (car got))) t)
                       'frozen (frozen-p s1) 'begin (snapshot-begin s1) 'point (snapshot-point s1)
                       'worker (cdr got) 'edited edited 'current (nreverse current))))
        (isolate-kill-all)))");
    let (ok, out, err) = batch(&script, Path::new("/"));
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(same t raw t frozen t begin 6 point 3 worker (emacs-lisp-mode 3 42 nil t refused refused bold) edited \"Snapshot text changed in *snapshot snap*\" current (t nil nil nil nil))", "{err}");
}

/// BUFFER's text properties other than `fontified' and cc-mode's
/// whitespace-scan caches (`c-in-sws', `c-is-sws', which record what the
/// walk happened to scan), as merged (BEG END ALIST) runs with sorted
/// keys: a Lisp form that prints to compare two buffers.
const RUNS: &str = "(lambda (buffer)
  (with-current-buffer buffer
    (let ((pos (point-min)) runs)
      (while (< pos (point-max))
        (let ((next (next-property-change pos nil (point-max))) (props (text-properties-at pos)) kept)
          (while props
            (unless (memq (car props) '(fontified c-in-sws c-is-sws)) (push (cons (car props) (nth 1 props)) kept))
            (setq props (nthcdr 2 props)))
          (setq kept (sort kept (lambda (a b) (string< (car a) (car b)))))
          (if (and kept runs (= (nth 1 (car runs)) pos) (equal (nth 2 (car runs)) kept))
              (setcar runs (list (nth 0 (car runs)) next kept))
            (when kept (push (list pos next kept) runs)))
          (setq pos next)))
      (nreverse runs))))";

#[test]
fn a_workers_fontification_is_the_sessions() {
    // The fontification consumer: the worker's jit-lock walk over a snapshot,
    // received block by block and put on a buffer that was never fontified,
    // gives the properties the session's own jit-lock walk from the same
    // window start gives.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let script = format!("(progn {PREAMBLE} (load {module:?})
      (let ((runs {RUNS}) (results nil))
        (dolist (spec (list (list {el:?} 'emacs-lisp-mode nil 40000) (list {c:?} 'c-mode '(cc-mode) 24000)))
          (dolist (feature (nth 2 spec)) (require feature))
          (let ((reference (generate-new-buffer \"reference\")) (owner (generate-new-buffer \"owner\")))
            (with-current-buffer reference
              (insert-file-contents (nth 0 spec) nil 0 (nth 3 spec)) (funcall (nth 1 spec))
              (let ((noninteractive nil)) (font-lock-mode 1))
              ;; Redisplay showing the buffer from 20000 first, then from its start.
              (dolist (start '(20000 1))
                (let ((pos start))
                  (while (setq pos (text-property-not-all pos (point-max) 'fontified t)) (jit-lock-function pos)))))
            (with-current-buffer owner
              (insert-file-contents (nth 0 spec) nil 0 (nth 3 spec)) (funcall (nth 1 spec))
              (font-lock-set-defaults)
              (let* ((token (setq romacs-fontify--tokens (1+ romacs-fontify--tokens)))
                     (snapshot (buffer-snapshot))
                     (state (romacs-fontify--new-state token snapshot nil)))
                (setq romacs-fontify--state state)
                (puthash token owner romacs-fontify--jobs)
                ;; Start the walk mid-buffer, as a window showing it there would.
                (let ((done (isolate-await (isolate-spawn romacs-fontify--job snapshot token romacs-fontify--block 20000
                                                          :require (nth 2 spec) :allow-source-boot t))))
                  (romacs--isolate-pump 0.2)
                  (let ((blocks (romacs-fontify--covering (point-min) (point-max))))
                    (when blocks (romacs-fontify--apply (point-min) (point-max) blocks))
                    (push (list (nth 1 spec) 'blocks (length blocks) (nth 2 done)
                                'fingerprint (equal (nth 1 done) (aref state 3))
                                'same (equal (funcall runs owner) (funcall runs reference))
                                'runs (length (funcall runs reference)))
                          results)))))))
        (prin1 (nreverse results))
        (isolate-kill-all)))",
        module = root.join("lisp/romacs/romacs-fontify.el").display().to_string(),
        el = root.join("lisp/gnu/subr.el").display().to_string(),
        c = root.join("test/gnu/test/manual/etags/c-src/emacs/src/keyboard.c").display().to_string());
    let (ok, out, err) = batch(&script, Path::new("/"));
    assert!(ok, "batch failed: {err}");
    let out = out.trim();
    assert!(out.starts_with("((emacs-lisp-mode blocks 5 5 fingerprint t same t runs ") && out.contains(") (c-mode blocks 3 3 fingerprint t same t runs "), "{out}\n{err}");
}

#[test]
fn an_idle_worker_ends_on_sigterm_without_waiting_for_sigkill() {
    // The GUI's worker waits on its evaluator thread while the main thread
    // joins it; SIGTERM may land on either, so the handler wakes the wait.
    let script = format!("(progn {PREAMBLE}
      (let* ((iso (isolate-create :allow-source-boot t))
             (isolate-kill-timeout 10))
        (isolate-await (isolate-spawn '(lambda () t) :isolate iso))
        (let ((t0 (float-time)))
          (isolate-kill iso)
          (prin1 (list 'quick (< (- (float-time) t0) 2.0) 'live (isolate-list))))))");
    let (ok, out, err) = batch(&script, Path::new("/"));
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(quick t live nil)", "{err}");
}

#[test]
fn results_wait_for_a_widened_buffer_and_a_changed_setup_refuses_them() {
    // Sol r1 S1-S3: a block delivered again while the owner is narrowed; a
    // syntactic face function the worker lacks changes the fingerprint; keywords
    // added after the results arrived make the buffer fontify in the session.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let script = format!("(progn {PREAMBLE} (load {module:?})
      (with-current-buffer (generate-new-buffer \"owner\")
        (insert-file-contents {el:?} nil 0 20000) (emacs-lisp-mode)
        (font-lock-set-defaults)
        (let* ((token (setq romacs-fontify--tokens (1+ romacs-fontify--tokens)))
               (snapshot (buffer-snapshot))
               (fingerprint (romacs-fontify-fingerprint))
               (mode-function font-lock-syntactic-face-function)
               (other (progn (setq-local font-lock-syntactic-face-function (lambda (_) 'font-lock-warning-face))
                             (prog1 (romacs-fontify-fingerprint) (setq-local font-lock-syntactic-face-function mode-function)))))
          (setq romacs-fontify--state (romacs-fontify--new-state token snapshot nil))
          (puthash token (current-buffer) romacs-fontify--jobs)
          (narrow-to-region 1 100)
          ;; From 9000: the block it began in is delivered twice.
          (let ((done (condition-case err
                          (progn (isolate-await (isolate-spawn romacs-fontify--job snapshot token romacs-fontify--block 9000 :allow-source-boot t))
                                 (romacs--isolate-pump 0.2) 'delivered)
                        (error err))))
            (widen)
            (prin1 (list done 'covered (and (romacs-fontify--covering 1 20000) t)
                         'syntactic-fingerprint-differs (not (equal fingerprint other))
                         ;; Sol r2 S1: a syntax table changed in place, then restored.
                         'syntax-changed (let ((old (aref (syntax-table) ?-)))
                                           (modify-syntax-entry ?- \"w\")
                                           (prog1 (list (romacs-fontify--setup-current-p romacs-fontify--state)
                                                        (progn (aset (syntax-table) ?- old)
                                                               (romacs-fontify--setup-current-p romacs-fontify--state)))))
                         'after-new-keywords (progn (font-lock-add-keywords nil '((\"defun\" 0 'font-lock-warning-face t)))
                                                    (romacs-fontify--covering 1 2000))
                         'refused romacs-fontify--refused)))))
        (isolate-kill-all))",
        module = root.join("lisp/romacs/romacs-fontify.el").display().to_string(),
        el = root.join("lisp/gnu/subr.el").display().to_string());
    let (ok, out, err) = batch(&script, Path::new("/"));
    assert!(ok, "batch failed: {err}");
    assert_eq!(out.trim(), "(delivered covered t syntactic-fingerprint-differs t syntax-changed (nil t) after-new-keywords nil refused t)", "{err}");
}
