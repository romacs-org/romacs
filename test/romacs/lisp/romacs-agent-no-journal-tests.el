;;; romacs-agent-no-journal-tests.el --- A chat that never reads a journal -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-ui))
(require 'romacs-agent-test-stored)

(defconst romacs-agent-no-journal-tests--id "c0c0c0c0-2222-4333-8444-555566667777")

(defmacro romacs-agent-no-journal-tests--with (&rest body)
  "BODY with `session', a Claude session of two turns, its journal written,
in a store of its own."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
            (romacs-agent-event-functions nil)
            (id romacs-agent-no-journal-tests--id)
            (dir (expand-file-name id (romacs-agent--state-root)))
            (seq 0))
       (make-directory dir t)
       (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                 (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" dir
                                                     "title" "nj" "titleAuto" :false "argv" [] "options" (romacs-mcp--object)))
       (cl-flet ((line (key value) (concat (romacs-agent--json (romacs-mcp--object key value "seq" (cl-incf seq) "t" (float seq))) "\n")))
         (let ((coding-system-for-write 'utf-8-unix))
           (write-region
            (apply #'concat
                   (mapcan (lambda (n)
                             (list (line "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "ask %d" n) "role" "user")
                                                                  "romacsRequest" n "type" "user"))
                                   (line "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                                   "message" (romacs-mcp--object "id" (format "m%d" n) "content"
                                                                                                 (vector (romacs-mcp--object "type" "text" "text" (format "say %d" n))))))
                                   (line "out" (romacs-mcp--object "type" "result" "result" (format "say %d" n)))))
                           '(1 2)))
            nil (expand-file-name "journal.jsonl" dir) nil 'silent)))
       (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd dir :title "nj"
                                          :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
         (puthash id session romacs-agent--sessions)
         (remhash id romacs-agent-events--imported)
         ,@body))))

(defun romacs-agent-no-journal-tests--chat (session)
  (let ((buffer (romacs-agent-ui--make-chat session)))
    (with-current-buffer buffer (romacs-agent-ui--fill buffer))
    buffer))

(ert-deftest romacs-agent-no-journal-waits-for-the-move ()
  ;; Not moved yet: a row says so, the chat waits for the move
  ;; (`romacs-agent-events-then'), its last turns drawn from its journal's
  ;; tail meanwhile, never blank; moved, it is drawn again from the store.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (let ((waits nil))
      (cl-letf (((symbol-function 'romacs-agent-events-then) (lambda (s then) (push (cons (romacs-agent-id s) then) waits))))
        ;; Its move failed: said so, not waited for again.
        (let ((buffer (romacs-agent-no-journal-tests--chat session)))
          (should (equal (mapcar #'car waits) (list id)))
          (should (string-search "move to the store" (with-current-buffer buffer (buffer-string))))
          (should (string-search "say 2" (with-current-buffer buffer (buffer-string))))
          ;; Said to be a preview, above it.
          (should (< (with-current-buffer buffer (string-search "a preview of its last turns" (buffer-string)))
                     (with-current-buffer buffer (string-search "say 2" (buffer-string)))))
          ;; Drawn, and nothing of the session's own state changed by it.
          (should (= (romacs-agent-seq session) 0))
          (funcall (cdr (pop waits)))
          (with-current-buffer buffer
            (should (string-search "could not move" (buffer-string)))
            (should-not (string-search "moving" (buffer-string))))
          (should-not waits)
          (kill-buffer buffer))
        ;; A tail past its budget: the row alone, nothing half drawn.
        (let* ((romacs-agent-ui--preview-seconds -1)
               (buffer (romacs-agent-no-journal-tests--chat session)))
          (should (string-search "move to the store" (with-current-buffer buffer (buffer-string))))
          (should-not (string-search "say" (with-current-buffer buffer (buffer-string))))
          (kill-buffer buffer))
        (pop waits)
        ;; Moved: drawn from the store, its preview gone.
        (let ((buffer (romacs-agent-no-journal-tests--chat session)))
          (romacs-agent-test-stored session)
          (funcall (cdr (pop waits)))
          (with-current-buffer buffer
            (should-not (string-search "move to the store" (buffer-string)))
            (should (= (cl-count-if (lambda (line) (string-search "say 2" line)) (split-string (buffer-string) "\n")) 1)))
          (kill-buffer buffer))))))

(ert-deftest romacs-agent-no-journal-a-preview-cut-short-leaves-nothing ()
  ;; A preview past its budget after it drew a prompt: none of it left,
  ;; nor what it set of the chat's drawing; the row says the records move.
  ;; A preview touches nothing of the session's: not its manifest.  Its
  ;; move failing, the row that says so takes the waiting row's place.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (let* ((manifest (expand-file-name "manifest.json" (romacs-agent-dir session)))
           (before (with-temp-buffer (insert-file-contents manifest) (buffer-string)))
           (waits nil) (renders 0) (time (symbol-function 'float-time))
           (render (symbol-function 'romacs-agent-ui--render)))
      (cl-letf (((symbol-function 'romacs-agent-events-then) (lambda (s then) (push (cons (romacs-agent-id s) then) waits)))
                ((symbol-function 'romacs-agent-ui--render)
                 (lambda (s e) (prog1 (funcall render s e) (cl-incf renders))))
                ;; Its budget spent once it drew its first event.
                ((symbol-function 'float-time) (lambda (&rest args) (if (> renders 0) 1e12 (apply time args)))))
        (let ((buffer (romacs-agent-no-journal-tests--chat session)))
          (should (> renders 0))
          (with-current-buffer buffer
            (should (string-search "move to the store" (buffer-string)))
            (should-not (string-search "ask" (buffer-string)))
            (should-not (string-search "preview" (buffer-string)))
            (should (zerop (or romacs-agent-ui--shown 0))))
          (kill-buffer buffer)))
      (should (equal (with-temp-buffer (insert-file-contents manifest) (buffer-string)) before))
      (setq waits nil)
      (cl-letf (((symbol-function 'romacs-agent-events-then) (lambda (s then) (push (cons (romacs-agent-id s) then) waits))))
        (let ((buffer (romacs-agent-no-journal-tests--chat session)))
          (funcall (cdr (pop waits)))
          (with-current-buffer buffer
            (should (< (string-search "could not move" (buffer-string)) (string-search "say 2" (buffer-string)))))
          (kill-buffer buffer))))))

(ert-deftest romacs-agent-no-journal-a-preview-writes-no-manifest ()
  ;; A record a preview reads would have the session's manifest written (a
  ;; harness naming another conversation): read for the view alone, it
  ;; writes nothing.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (let* ((manifest (expand-file-name "manifest.json" (romacs-agent-dir session)))
           (journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
      (let ((coding-system-for-write 'utf-8-unix))
        (write-region (concat (romacs-agent--json (romacs-mcp--object "out" (romacs-mcp--object "type" "system" "subtype" "init"
                                                                                                "session_id" "another-conversation")
                                                                      "seq" 7 "t" 7.0))
                              "\n")
                      nil journal t 'silent))
      (let ((before (with-temp-buffer (insert-file-contents manifest) (buffer-string))))
        (cl-letf (((symbol-function 'romacs-agent-events-then) #'ignore))
          (let ((buffer (romacs-agent-no-journal-tests--chat session)))
            (should (string-search "a preview of its last turns" (with-current-buffer buffer (buffer-string))))
            (kill-buffer buffer)))
        (should (equal (with-temp-buffer (insert-file-contents manifest) (buffer-string)) before))))))

(ert-deftest romacs-agent-no-journal-a-preview-keeps-to-its-budget-choosing-its-tail ()
  ;; A tail costly to choose (a record of megabytes, then prompts written
  ;; into a turn under way, each looking back over it): the choice is timed
  ;; with the rest, so past the budget there is no preview, not a wait.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (let ((journal (expand-file-name "journal.jsonl" (romacs-agent-dir session))))
      (let ((coding-system-for-write 'utf-8-unix) (seq 6))
        (write-region
         (concat (romacs-agent--json (romacs-mcp--object "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "go" "role" "user")
                                                                              "romacsRequest" 3 "type" "user")
                                                         "seq" (cl-incf seq) "t" 1.0))
                 "\n"
                 (romacs-agent--json (romacs-mcp--object "out" (romacs-mcp--object "type" "user" "message"
                                                                                   (romacs-mcp--object "content" (make-string 2900000 ?x)))
                                                         "seq" (cl-incf seq) "t" 1.0))
                 "\n"
                 (mapconcat (lambda (n)
                              (concat (romacs-agent--json (romacs-mcp--object "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "steer %d" n) "role" "user")
                                                                                                   "romacsRequest" (+ 4 n) "type" "user")
                                                                              "seq" (cl-incf seq) "t" 1.0))
                                      "\n"))
                            (number-sequence 1 64) ""))
         nil journal t 'silent))
      (cl-letf (((symbol-function 'romacs-agent-events-then) #'ignore))
        (let* ((romacs-agent-ui--preview-seconds 0.2) (start (float-time))
               (buffer (romacs-agent-no-journal-tests--chat session)))
          (should (< (- (float-time) start) 0.6))
          (should (string-search "move to the store" (with-current-buffer buffer (buffer-string))))
          (kill-buffer buffer))))))

(ert-deftest romacs-agent-no-journal-loading-session-takes-its-restored-chat ()
  ;; A chat restored from its row is taken over by its session as the
  ;; session loads (a watcher's, not a command's), and drawn again once
  ;; its supervisor's replay is over: what it said meanwhile is not missed.
  (let* ((id "r1r1r1r1-2222-4333-8444-555566667777")
         (row (romacs-agent--make :id id :harness 'claude :title "row"))
         (live (romacs-agent--make :id id :harness 'claude :title "live"))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent-ui--row-chats (make-hash-table :test #'equal))
         (redraws 0))
    (cl-letf (((symbol-function 'romacs-agent-store-row-session) (lambda (_) row))
              ((symbol-function 'romacs-agent-ui--redraw) (lambda () (cl-incf redraws)))
              ;; Its supervisor listens: its replay comes after the load.
              ((symbol-function 'romacs-agent-host-socket) (lambda (_) (or load-file-name buffer-file-name "/"))))
      (let ((buffer (romacs-agent-ui-desktop-restore nil nil (list :session id :draft ""))))
        (unwind-protect
            (progn
              (should (eq (buffer-local-value 'romacs-agent-ui--session buffer) row))
              (with-current-buffer buffer (setq romacs-agent-ui--unfilled nil))
              (run-hook-with-args 'romacs-agent-loaded-functions live (make-hash-table))
              (should (eq (buffer-local-value 'romacs-agent-ui--session buffer) live))
              (should (eq (romacs-agent-buffer live) buffer))
              (should (= redraws 0))
              (run-hook-with-args 'romacs-agent-caught-up-functions live)
              (should (= redraws 1))
              ;; Once.
              (run-hook-with-args 'romacs-agent-caught-up-functions live)
              (should (= redraws 1)))
          (kill-buffer buffer))))))

(ert-deftest romacs-agent-no-journal-opens-from-the-store-alone ()
  ;; Moved: its chat opens with its journal never read.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (romacs-agent-test-stored session)
    (cl-letf (((symbol-function 'romacs-agent--read-journal-file) (lambda (&rest _) (error "Read the journal")))
              ((symbol-function 'romacs-agent--journal-file-head) (lambda (&rest _) (error "Read the journal")))
              ((symbol-function 'romacs-agent--journal-file-after) (lambda (&rest _) (error "Read the journal"))))
      (let ((buffer (romacs-agent-no-journal-tests--chat session)))
        (with-current-buffer buffer
          (should (string-search "ask 1" (buffer-string)))
          (should (string-search "say 2" (buffer-string))))
        (kill-buffer buffer)))))

(ert-deftest romacs-agent-no-journal-restored-chat-loads-when-used ()
  ;; Restored from its row alone, a chat's session loads at the first
  ;; command that uses it.
  (let* ((row (romacs-agent--make :id "r0r0r0r0-2222-4333-8444-555566667777" :harness 'claude :title "row"))
         (live (romacs-agent--make :id "r0r0r0r0-2222-4333-8444-555566667777" :harness 'claude :title "live"))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (loaded nil))
    (with-temp-buffer
      (romacs-agent-chat-mode)
      (insert "transcript\n")
      (setq romacs-agent-ui--session row romacs-agent-ui--end (point-max-marker))
      (cl-letf (((symbol-function 'romacs-agent-load-session)
                 (lambda (id) (setq loaded id) (puthash id live romacs-agent--sessions) live)))
        ;; Moving about the transcript needs nothing of it.
        (goto-char (point-min))
        (let ((this-command 'forward-char)) (romacs-agent-ui--ensure-live))
        (should-not loaded)
        ;; A chat's own command loads it, the chat its live session's.
        (let ((this-command 'romacs-agent-chat-send)) (romacs-agent-ui--ensure-live))
        (should (equal loaded (romacs-agent-id row)))
        (should (eq romacs-agent-ui--session live))
        (should (eq (romacs-agent-buffer live) (current-buffer)))))))

;;; romacs-agent-no-journal-tests.el ends here
