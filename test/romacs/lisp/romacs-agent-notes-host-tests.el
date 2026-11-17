;;; romacs-agent-notes-host-tests.el --- One writer of a session's records -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Against the real `romacs-agent-host': a session's records are numbered
;; by one host at a time, so every note is written once, each record has
;; its own seq, and none is lost, whether the notes go through a
;; notes-only host, a supervisor launched while one runs, or an ending
;; editor's drain.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-writer)
(require 'romacs-test-socket)

(defun romacs-agent-notes-host-tests--host ()
  "The real host beside this binary, when it writes notes alone."
  (when-let* ((host (romacs-agent--found-program "romacs-agent-host")))
    (with-temp-buffer
      (call-process host nil t nil)
      (and (search-backward "--notes-only" nil t) host))))

(defconst romacs-agent-notes-host-tests--patience 60
  "Seconds a wait for a host lasts: a loaded machine starts a process and
hands a lock over in seconds, not the moment it does unloaded.")

(defun romacs-agent-notes-host-tests--until (pred &optional what)
  "Wait, up to `romacs-agent-notes-host-tests--patience' seconds, until
PRED holds; its value.  Not by then: failed, saying WHAT was waited for."
  (let ((end (+ (float-time) romacs-agent-notes-host-tests--patience)))
    (while (and (not (funcall pred)) (< (float-time) end))
      (accept-process-output nil 0.05))
    (or (funcall pred) (ert-fail (list 'waited-for (or what "a host") romacs-agent-notes-host-tests--patience)))))

(defun romacs-agent-notes-host-tests--records (session)
  "SESSION's journal, a record a line."
  (with-temp-buffer
    (insert-file-contents (expand-file-name "journal.jsonl" (romacs-agent-dir session)))
    (mapcar (lambda (line) (json-parse-string line :object-type 'hash-table :null-object :null :false-object :false))
            (split-string (buffer-string) "\n" t))))

(defun romacs-agent-notes-host-tests--check (session types)
  "Each record of SESSION has its own seq, rising; its notes of TYPES
\(a list of strings) are there once each."
  (let* ((records (romacs-agent-notes-host-tests--records session))
         (seqs (mapcar (lambda (r) (gethash "seq" r)) records))
         (notes (delq nil (mapcar (lambda (r) (let ((note (gethash "note" r)))
                                                (and (hash-table-p note) (gethash "type" note))))
                                  records))))
    (should (equal seqs (number-sequence 1 (length seqs))))
    (dolist (type types)
      (should (= (cl-count type notes :test #'equal) 1)))))

(defun romacs-agent-notes-host-tests--with (body)
  "Call BODY with a loaded session that has a run and no supervisor, its
state and runtime directories its own."
  (let* ((root (make-temp-file "romacs-notes-" t))
         (run (romacs-test-socket-directory "notes"))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                            (concat "XDG_RUNTIME_DIR=" run))
                                      process-environment))
         (romacs-agent-events-store nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--launched (make-hash-table :test #'equal))
         (romacs-agent--held-notes (make-hash-table :test #'equal))
         (romacs-agent--note-hosts (make-hash-table :test #'equal))
         (romacs-agent--notes-host #'romacs-agent--launch-notes)
         (id "a0a00000-1111-4222-8333-444455556666")
         (dir (expand-file-name id (romacs-agent--state-root))))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root
                                                        "title" "notes" "titleAuto" :false "argv" []
                                                        "options" (romacs-mcp--object)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (concat (json-serialize (romacs-mcp--object "seq" 1 "t" 1.0 "note" (romacs-mcp--object "type" "run")))
                                  "\n")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (romacs-agent--load dir)
          (funcall body (romacs-agent-get id)))
      (when-let* ((session (gethash id romacs-agent--sessions))
                  (connection (romacs-agent-connection session)))
        (delete-process connection))
      (delete-directory root t)
      (delete-directory run t))))

(defun romacs-agent-notes-host-tests--written (session)
  "Whether SESSION's notes are written and their host gone."
  (and (null (gethash (romacs-agent-id session) romacs-agent--held-notes))
       (null (gethash (romacs-agent-id session) romacs-agent--note-hosts))
       (not (file-exists-p (romacs-agent-host-socket session)))))

(ert-deftest romacs-agent-notes-host-writes-a-session-s-notes ()
  ;; No supervisor: a notes-only host writes the notes, numbered after
  ;; what is there, and ends; the session keeps its state.
  (skip-unless (romacs-agent-notes-host-tests--host))
  (romacs-agent-notes-host-tests--with
   (lambda (session)
     (let ((state (romacs-agent-state session)))
       (romacs-agent--note session (romacs-mcp--object "type" "plan"))
       (romacs-agent--note session (romacs-mcp--object "type" "selection"))
       (should (= (romacs-agent-held-notes-count) 2))
       (should (romacs-agent-notes-host-tests--until (lambda () (romacs-agent-notes-host-tests--written session)) "its notes written and its host gone"))
       (should (eq (romacs-agent-state session) state))
       (should (= (romacs-agent-seq session) 3))
       (romacs-agent-notes-host-tests--check session '("plan" "selection"))
       ;; Again, a new host.
       (romacs-agent--note session (romacs-mcp--object "type" "title"))
       (should (romacs-agent-notes-host-tests--until (lambda () (romacs-agent-notes-host-tests--written session)) "its notes written and its host gone"))
       (romacs-agent-notes-host-tests--check session '("plan" "selection" "title"))))))

(ert-deftest romacs-agent-notes-host-a-launch-waits-for-it ()
  ;; A supervisor launched while a notes-only host writes: it numbers
  ;; after that host's records, a note that host did not echo goes once,
  ;; through it, and what is written to the harness reaches the harness,
  ;; not the notes-only host.
  (skip-unless (romacs-agent-notes-host-tests--host))
  (romacs-agent-notes-host-tests--with
   (lambda (session)
     (let ((real (symbol-function 'call-process)))
       (cl-letf (((symbol-function 'call-process)
                  ;; No user manager: detached, as in a session without one.
                  (lambda (program &rest args) (if (equal program "systemd-run") 1 (apply real program args)))))
         (romacs-agent--note session (romacs-mcp--object "type" "plan"))
         ;; Its harness echoes two lines: the driver's first, then ours.
         (romacs-agent--launch session (list "head" "-n" "2"))
         (romacs-agent--attach session romacs-agent--launch-tries)
         (should (romacs-agent-notes-host-tests--until
                  (lambda () (and (process-live-p (romacs-agent-connection session))
                                  (romacs-agent-caught-up session)
                                  (not (gethash (romacs-agent-id session) romacs-agent--note-hosts))))
                  "the supervisor attached"))
         (process-send-string (romacs-agent-connection session)
                              (concat (json-serialize (romacs-mcp--object "write" "hello")) "\n"))
         (should (romacs-agent-notes-host-tests--until
                  (lambda () (and (null (gethash (romacs-agent-id session) romacs-agent--held-notes))
                                  (cl-some (lambda (r) (gethash "exit" r))
                                           (romacs-agent-notes-host-tests--records session))))
                  "the note written and the harness's exit"))
         (should (cl-some (lambda (r) (equal (gethash "out" r) "hello"))
                          (romacs-agent-notes-host-tests--records session)))
         (romacs-agent-notes-host-tests--check session '("plan")))))))

(ert-deftest romacs-agent-notes-host-an-ending-editor-waits-for-it ()
  ;; Ending: the drain waits for the notes on their way.
  (skip-unless (romacs-agent-notes-host-tests--host))
  (romacs-agent-notes-host-tests--with
   (lambda (session)
     (cl-letf (((symbol-function 'redisplay) #'ignore)
               ((symbol-function 'message) #'ignore))
       (romacs-agent--note session (romacs-mcp--object "type" "plan"))
       (romacs-agent-writer-drain)
       (should (zerop (romacs-agent-held-notes-count)))
       (romacs-agent-notes-host-tests--check session '("plan"))))))

;;; romacs-agent-notes-host-tests.el ends here

(ert-deftest romacs-agent-notes-host-a-batch-process-writes-its-notes-before-ending ()
  ;; A batch process (a turns' fill, a script) noting a session with no
  ;; supervisor: the real notes-only host writes the note before the
  ;; process ends, and no host is left behind.
  (skip-unless (romacs-agent-notes-host-tests--host))
  (romacs-agent-notes-host-tests--with
   (lambda (session)
     (let ((id (romacs-agent-id session)))
       (with-temp-buffer
         (call-process (expand-file-name invocation-name invocation-directory) nil t nil "--batch"
                       "-l" "romacs-agent" "-l" "romacs-agent-writer"
                       "--eval" (format "(progn (setq romacs-agent-events-store nil) (romacs-agent--load %S) (romacs-agent--note (romacs-agent-get %S) (romacs-mcp--object \"type\" \"plan\")))"
                                        (romacs-agent-dir session) id)))
       (romacs-agent-notes-host-tests--check session '("plan"))
       ;; Its host ends a moment after the process that attached to it.
       (should (romacs-agent-notes-host-tests--until
                (lambda () (and (not (file-exists-p (romacs-agent-host-socket session)))
                                (not (string-search (romacs-agent-dir session)
                                                    (shell-command-to-string "ps -eo args")))))
                "the batch process's host gone"))))))

(ert-deftest romacs-agent-notes-host-a-stolen-attachment-writes-the-note-once ()
  ;; Another server steals the notes-only host's attachment before the
  ;; note is written, then lets go: the note goes through the next host,
  ;; once, numbered after what is there.
  (skip-unless (romacs-agent-notes-host-tests--host))
  (romacs-agent-notes-host-tests--with
   (lambda (session)
     (romacs-agent--note session (romacs-mcp--object "type" "plan"))
     (should (romacs-agent-notes-host-tests--until (lambda () (file-exists-p (romacs-agent-host-socket session))) "its notes-only host listening"))
     (let ((thief (make-network-process :name "thief" :family 'local :service (romacs-agent-host-socket session)
                                        :noquery t :coding 'no-conversion)))
       (process-send-string thief "{\"attach\":{\"from\":1,\"steal\":true}}\n")
       (accept-process-output nil 0.5)
       (delete-process thief))
     (should (romacs-agent-notes-host-tests--until (lambda () (romacs-agent-notes-host-tests--written session)) "its notes written and its host gone"))
     (romacs-agent-notes-host-tests--check session '("plan")))))
