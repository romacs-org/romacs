//! Session images (lisp/romacs/warm-restart.el) across processes: one batch
//! session builds a session (tests/fixtures/warm-restart-session.el) and
//! writes its image; a fresh process restores it and must observe the same
//! buffers (text, point, mark, mark ring, narrowing, modified flag, mode,
//! minor modes, locals, directory, hard newlines, undo), window layout
//! with points and starts, kill ring, registers, histories and recentf.
//! What is not carried is named in the image; a blocked buffer refuses
//! the image; a damaged image or a buffer that cannot come back keeps the
//! image and never loses its text.
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/warm-restart-session.el");
const ROMACS_LISP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../lisp/romacs");

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romacs-warm-restart-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

fn lisp(path: &Path) -> String {
    format!("{:?}", path.display().to_string())
}

/// A batch session in DIR's private HOME running SOURCE after the fixture;
/// its stdout.
fn batch(dir: &Path, source: &str) -> String {
    let script = dir.join(format!("script-{}.el", source.len()));
    std::fs::write(&script, format!("(setq load-path (cons {} load-path))\n(load {} nil t)\n{source}\n",
                                    lisp(Path::new(ROMACS_LISP)), lisp(Path::new(FIXTURE)))).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "-l"]).arg(&script).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state")).env("ROMACS_WORLD", "off")
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "batch failed: {}\n{stdout}\n{}", output.status, String::from_utf8_lossy(&output.stderr));
    stdout
}

/// The text after TAG on its line of OUT.
fn tagged<'a>(out: &'a str, tag: &str) -> &'a str {
    let prefix = format!("{tag} ");
    out.lines().find_map(|line| line.strip_prefix(&prefix)).unwrap_or_else(|| panic!("no {tag} in {out}"))
}

#[test]
fn a_session_round_trips_through_an_image_into_a_new_process() {
    let dir = dir("round-trip");
    let image = dir.join("s.image");
    let files = dir.join("files");
    let written = batch(&dir, &format!(
        "(wrt-make {files})
         (wrt-print \"OBS\" (wrt-observe))
         (wrt-print \"IMAGE\" (warm-restart-write nil {image}))
         (wrt-print \"MODE\" (format \"%o\" (file-modes {image})))
         (wrt-print \"UNDO\" (wrt-undo-a))
         (wrt-print \"END\" t)", files = lisp(&files), image = lisp(&image)));
    let restored = batch(&dir, &format!(
        "(setq default-directory {files})
         (let ((report (warm-restart-restore-image {image})))
           (wrt-print \"REPORT\" (list (plist-get report :buffers) (plist-get report :records) (plist-get report :error)
                                      (plist-get report :problems)))
           (wrt-print \"LOST\" (mapcar (lambda (e) (or (plist-get e :process) (plist-get e :buffer) (plist-get e :register)))
                                      (plist-get report :lost))))
         (wrt-print \"OBS\" (wrt-observe))
         (wrt-print \"ATTACHED\" (mapcar #'buffer-name wrt-attached))
         (wrt-print \"KEPT\" (list (file-exists-p {image}) (file-exists-p (expand-file-name \"restored.image\" {dir}))))
         (wrt-print \"UNDO\" (wrt-undo-a))
         (wrt-print \"END\" t)", files = lisp(&files), image = lisp(&image), dir = lisp(&dir)));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tagged(&written, "MODE"), "\"600\"", "an image holds buffer text: owner-only");
    assert_eq!(tagged(&restored, "REPORT"), "(9 9 nil nil)", "every carried buffer restored, no problem");
    assert_eq!(tagged(&restored, "OBS"), tagged(&written, "OBS"), "the restored session observes as the written one");
    assert_eq!(tagged(&written, "UNDO"), r#"(("alpha one\nalpha two\nalpha three\nadded λ" nil) ("a\nb" (2)))"#,
               "undo removes the last insertion, and brings a deleted hard newline back hard");
    assert_eq!(tagged(&restored, "UNDO"), tagged(&written, "UNDO"), "undo in a restored buffer undoes as it did");
    assert_eq!(tagged(&restored, "LOST"), "(\"*proc*\" \"*regen*\" \"wrt-sleep\" 119)",
               "the process buffer, the regenerable view, the process and the window-configuration register are named");
    assert_eq!(tagged(&restored, "ATTACHED"), "(\"*wrt-view*\")", "the handler-owned buffer attached at activation");
    assert_eq!(tagged(&restored, "KEPT"), "(nil t)", "a complete restore keeps the image as restored.image");
}

#[test]
fn a_blocked_buffer_refuses_an_image() {
    let dir = dir("blocked");
    let out = batch(&dir, &format!(
        "(with-current-buffer (get-buffer-create \"*busy*\")
           (add-hook 'warm-restart-blocker-functions (lambda () \"turn in progress\") nil t))
         (wrt-print \"BLOCKED\" (condition-case err (progn (warm-restart-write nil {image}) 'written)
                                (warm-restart-blocked (cdr err))))
         (wrt-print \"WRITTEN\" (file-exists-p {image}))
         (wrt-print \"END\" t)", image = lisp(&dir.join("b.image"))));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tagged(&out, "BLOCKED"), "(\"*busy*: turn in progress\")");
    assert_eq!(tagged(&out, "WRITTEN"), "nil");
}

#[test]
fn a_damaged_image_or_a_failing_mode_keeps_the_image_and_the_text() {
    let dir = dir("damaged");
    let image = dir.join("d.image");
    batch(&dir, &format!(
        "(define-derived-mode wrt-broken-mode text-mode \"Broken\")
         (with-current-buffer (get-buffer-create \"*draft*\") (wrt-broken-mode) (insert \"unsaved draft\"))
         (with-current-buffer (get-buffer-create \"*wrt-view*\")
           (wrt-view-mode) (setq-local desktop-save-buffer #'wrt-view-save))
         (warm-restart-write nil {image})", image = lisp(&image)));
    let whole = std::fs::read(&image).unwrap();
    std::fs::write(dir.join("cut.image"), &whole[..whole.len() - 3]).unwrap();
    let out = batch(&dir, &format!(
        "(let ((report (warm-restart-restore-image {cut})))
           (wrt-print \"CUT\" (list (and (plist-get report :error) t) (file-exists-p {cut}) (get-buffer \"*draft*\"))))
         (define-derived-mode wrt-broken-mode text-mode \"Broken\" (error \"mode refuses\"))
         (defun wrt-view-restore (&rest _) (error \"handler refuses\"))
         (let ((report (warm-restart-restore-image {image})))
           (wrt-print \"BROKEN\" (list (sort (mapcar (lambda (p) (plist-get p :reason)) (plist-get report :problems)) #'string<)
                                      (file-exists-p {image})
                                      (with-current-buffer \"*draft*\" (list major-mode (buffer-string) (buffer-modified-p)))))
           (wrt-print \"HANDLER\" (with-current-buffer \"*wrt-view*\"
                                     (list major-mode (and (string-search \":topic\" (buffer-string)) t)))))
         (wrt-print \"END\" t)", cut = lisp(&dir.join("cut.image")), image = lisp(&image)));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tagged(&out, "CUT"), "(t t nil)", "a truncated image is refused whole and stays");
    assert_eq!(tagged(&out, "BROKEN"),
               "((\"restored as plain text: handler refuses\" \"restored as plain text: mode refuses\") t (fundamental-mode \"unsaved draft\" t))",
               "a buffer whose mode fails keeps its text, and the image stays");
    assert_eq!(tagged(&out, "HANDLER"), "(fundamental-mode t)", "a failing handler leaves what it saved as text");
}

#[test]
fn a_restored_unsaved_file_is_locked_and_another_editors_lock_asks_at_the_next_edit() {
    let dir = dir("locks");
    let image = dir.join("l.image");
    let files = dir.join("files");
    batch(&dir, &format!(
        "(setq default-directory {files})
         (dolist (name '(\"mine.txt\" \"theirs.txt\"))
           (with-temp-file name (insert \"text\\n\"))
           (with-current-buffer (find-file-noselect name) (insert \"edit \")))
         (make-indirect-buffer \"theirs.txt\" \"theirs-view\")
         (warm-restart-write nil {image})
         ;; Exit as `romacs-restart' does: this process's locks go with it.
         (dolist (b (buffer-list)) (with-current-buffer b (when buffer-file-name (unlock-buffer))))",
        files = lisp(&files), image = lisp(&image)));
    // Another editor, alive (pid 1), takes one file while no process holds it.
    std::os::unix::fs::symlink("someone@elsewhere.1:0", files.join(".#theirs.txt")).unwrap();
    let out = batch(&dir, &format!(
        "(setq default-directory {files})
         (warm-restart-restore-image {image})
         (sleep-for 0.1)
         (wrt-print \"LOCKS\" (list (file-locked-p \"mine.txt\") (file-locked-p \"theirs.txt\")
                                    (mapcar (lambda (p) (plist-get p :reason)) (plist-get warm-restart-last-report :locks))))
         ;; The first answer refuses (GNU's `q'), the second steals.
         (let (asked results)
           (cl-letf (((symbol-function 'ask-user-about-lock)
                      (lambda (file who) (push who asked)
                        (if (cdr asked) t (signal 'file-locked (list file who))))))
             (dolist (buffer '(\"theirs-view\" \"theirs.txt\" \"theirs-view\"))
               (push (condition-case nil (with-current-buffer buffer (goto-char (point-min)) (insert \"x\") 'edited)
                       (file-locked 'refused))
                     results)
               ;; A change of mode after the refusal keeps the pending lock.
               (when (equal results '(refused)) (with-current-buffer \"theirs.txt\" (fundamental-mode)))))
           (wrt-print \"ASKED\" (list (nreverse results) (length asked) (file-locked-p \"theirs.txt\")
                                      (with-current-buffer \"theirs.txt\" (buffer-string)))))",
        files = lisp(&files), image = lisp(&image)));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tagged(&out, "LOCKS"), "(t \"someone\" (\"file locked by another editor; asks at the next edit\"))",
               "the new process holds its unsaved file's lock and names the one it could not take");
    assert_eq!(tagged(&out, "ASKED"), "((refused edited edited) 2 t \"xxedit text\\n\")",
               "a refusal in a view refuses that edit; the next edit asks again and, stealing, locks; then none asks");
}

#[test]
fn an_agent_chat_comes_back_on_its_session_with_its_draft() {
    // Agent sessions outlive the editor (lisp/romacs/romacs-agent.el); a restored
    // chat is the session's view, made from its journal, never a copy.
    let dir = dir("agent");
    let session = dir.join("state/romacs/agents/0a1b2c3d-test");
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(session.join("manifest.json"), r#"{"id": "0a1b2c3d-test", "harness": "claude", "cwd": "/tmp",
        "title": "probe chat", "unit": "none", "hostSocket": "/nonexistent/host.sock"}"#).unwrap();
    std::fs::write(session.join("journal.jsonl"), "").unwrap();
    let image = dir.join("a.image");
    let written = batch(&dir, &format!(
        "(require 'romacs-agent)
         (romacs-agent-restore)
         (with-current-buffer (romacs-agent-show (romacs-agent-get \"0a1b2c3d-test\"))
           (goto-char (point-max)) (insert \"half a question\"))
         (wrt-print \"STRATEGY\" (plist-get (seq-find (lambda (e) (equal (plist-get e :name) \"*agent: probe chat*\"))
                                                     (plist-get (warm-restart-plan) :buffers))
                                           :strategy))
         (warm-restart-write nil {image})", image = lisp(&image)));
    let restored = batch(&dir, &format!(
        "(warm-restart-restore-image {image})
         (wrt-print \"CHATS\" (cl-loop for b in (buffer-list)
                                       when (eq (buffer-local-value 'major-mode b) 'romacs-agent-chat-mode)
                                       collect (with-current-buffer b (list (buffer-name) (romacs-agent-id romacs-agent-ui--session)
                                                                            (romacs-agent-ui--composer-text)))))",
        image = lisp(&image)));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tagged(&written, "STRATEGY"), "handler");
    assert_eq!(tagged(&restored, "CHATS"), "((\"*agent: probe chat*\" \"0a1b2c3d-test\" \"half a question\"))");
}

/// A generated widget buffer isn't an edit: its text alone can't work, so
/// it stays behind for its package to rebuild, and typed text still comes.
#[test]
fn a_widget_buffer_is_dropped_and_a_typed_view_is_carried() {
    let dir = dir("widgets");
    let out = batch(&dir,
        "(require 'wid-edit)
         (define-derived-mode wr-view-mode fundamental-mode \"View\")
         (with-current-buffer (get-buffer-create \"*wr-widgets*\")
           (wr-view-mode)
           (widget-create 'push-button \"Go\")
           (widget-setup))
         (with-current-buffer (get-buffer-create \"*wr-typed*\")
           (wr-view-mode)
           (insert \"notes\"))
         (princ (format \"CLASS %S\\n\"
                        (list (with-current-buffer \"*wr-widgets*\" (car (warm-restart--classify)))
                              (with-current-buffer \"*wr-typed*\" (car (warm-restart--classify))))))");
    assert_eq!(tagged(&out, "CLASS"), "(dropped text)");
}

#[test]
fn clones_own_their_ids_and_dired_and_edited_views_come_back() {
    let dir = dir("views");
    let image = dir.join("v.image");
    let files = dir.join("files");
    batch(&dir, &format!(
        "(setq default-directory {files})
         (with-temp-file \"a.txt\" (insert \"text\\n\"))
         (find-file-noselect \"a.txt\")
         (warm-restart-write nil {image})
         (with-current-buffer \"a.txt\" (clone-indirect-buffer \"a-clone\" nil))
         (dired-noselect {files})
         (with-current-buffer (get-buffer-create \"*Help*\") (insert \"notes typed here\"))
         (warm-restart-write nil {image})", files = lisp(&files), image = lisp(&image)));
    let out = batch(&dir, &format!(
        "(let ((report (warm-restart-restore-image {image})))
           (wrt-print \"REPORT\" (list (plist-get report :buffers) (plist-get report :problems))))
         (wrt-print \"CLONE\" (buffer-name (buffer-base-buffer (get-buffer \"a-clone\"))))
         (wrt-print \"DIRED\" (cl-loop for b in (buffer-list) when (eq (buffer-local-value 'major-mode b) 'dired-mode)
                                      collect (buffer-name b)))
         (wrt-print \"HELP\" (with-current-buffer \"*Help*\" (buffer-string)))", image = lisp(&image)));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tagged(&out, "REPORT"), "(5 nil)", "a.txt, its clone, dired, *Help* and *scratch*");
    assert_eq!(tagged(&out, "CLONE"), "\"a.txt\"", "the clone is its own indirect buffer, not its base's id");
    assert_eq!(tagged(&out, "DIRED"), "(\"files\")", "dired comes back through its autoloaded handler");
    assert_eq!(tagged(&out, "HELP"), "\"notes typed here\"", "an edited buffer is carried whatever its name");
}
