;;; romacs-agent-drawn-tests.el --- A chat's finished turns kept as drawn -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-turns)
  (require 'romacs-agent-ui))

(defconst romacs-agent-drawn-tests--long (make-string 3000 ?x)
  "An output longer than a row keeps.")

(defun romacs-agent-drawn-tests--records (turns)
  "TURNS Claude turns' records: a run note, then a prompt, a call with its
result (the last turn's long), a reply and its end a turn."
  (let ((seq 0) (records nil))
    (cl-flet ((add (key value) (push (romacs-mcp--object key value "seq" (cl-incf seq) "t" (+ 1794600000.0 (* 2 seq))) records)))
      (add "note" (romacs-mcp--object "type" "run"))
      (dotimes (i turns)
        (let ((n (1+ i)))
          (add "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                        "romacsRequest" n "type" "user"))
          (add "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "id" (format "t%d" n)
                                                                       "content" (vector (romacs-mcp--object "type" "tool_use" "id" (format "u%d" n) "name" "Bash"
                                                                                                             "input" (romacs-mcp--object "command" (format "grep %d" n)))))))
          (add "out" (romacs-mcp--object "type" "user" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "content" (vector (romacs-mcp--object "type" "tool_result" "tool_use_id" (format "u%d" n)
                                                                                                             "content" (if (= n turns) romacs-agent-drawn-tests--long "found"))))))
          (add "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "id" (format "m%d" n)
                                                                       "content" (vector (romacs-mcp--object "type" "text" "text" (format "answer %d" n))))))
          (add "out" (romacs-mcp--object "type" "result" "result" (format "answer %d" n))))))
    (nreverse records)))

(defmacro romacs-agent-drawn-tests--with (turns &rest body)
  "BODY with `session', a Claude session of TURNS turns in a state root of
its own, its records in its journal and `records' them all."
  (declare (indent 1))
  `(let* ((root (make-temp-file "romacs-chat-rows-" t))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                       process-environment))
          (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-event-functions nil)
          (id "b1b1b1b1-2222-4333-8444-555566667777")
          (dir (expand-file-name id (romacs-agent--state-root)))
          (records (romacs-agent-drawn-tests--records ,turns)))
     (unwind-protect
         (progn
           (make-directory dir t)
           (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                     (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root "title" "rows"
                                                         "titleAuto" :false "argv" [] "options" (romacs-mcp--object)))
           (let ((coding-system-for-write 'utf-8-unix))
             (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) records "")
                           nil (expand-file-name "journal.jsonl" dir) nil 'silent))
           (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root :title "rows"
                                              :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
             (puthash id session romacs-agent--sessions)
             ,@body))
       (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
       (delete-directory root t))))

(defun romacs-agent-drawn-tests--drawn (session &optional records-only)
  "SESSION's chat transcript as a first show draws it (5 turns), from its
records alone with RECORDS-ONLY, and the buffer."
  (let ((buffer (romacs-agent-ui--make-chat session)))
    (with-current-buffer buffer
      (setq romacs-agent-ui--turns 5)
      (cl-letf (((symbol-function 'romacs-agent-turns-page)
                 (if records-only #'ignore (symbol-function 'romacs-agent-turns-page))))
        (romacs-agent-ui--fill buffer))
      (setf (romacs-agent-buffer session) nil)
      (cons (buffer-substring-no-properties (point-min) romacs-agent-ui--end) buffer))))

(defun romacs-agent-drawn-tests--forms (session n)
  "SESSION's last N turns drawn by `romacs-agent-ui-draw-turn', oldest first."
  (mapcar (lambda (row) (romacs-agent-ui-draw-turn session row))
          (reverse (plist-get (romacs-agent-turns-page session nil n) :rows))))

(ert-deftest romacs-agent-drawn-turns-read-as-the-chat-draws-them ()
  ;; A turn drawn in a worker and inserted reads as the chat draws it from
  ;; its row: its text, its faces, its keys; the form is data.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-drawn-tests--with 12
    (should (= (romacs-agent-turns-fill session) 12))
    (let* ((drawn (romacs-agent-drawn-tests--drawn session t))
           (forms (romacs-agent-drawn-tests--forms session 5))
           (chat (cdr drawn)))
      (dolist (form forms)
        (should-not (string-search "#<" form))
        (should (car (read-from-string form))))
      (with-temp-buffer
        (dolist (form forms) (romacs-agent-drawn-insert form))
        (let ((kept (buffer-string))
              (expected (with-current-buffer chat
                          (buffer-substring (save-excursion (goto-char (point-min)) (forward-line 1) (point))
                                            romacs-agent-ui--end))))
          (should (equal (substring-no-properties kept) (substring-no-properties expected)))
          (dotimes (i (length kept))
            (should (equal (get-text-property i 'face kept) (get-text-property i 'face expected)))
            (should (equal (get-text-property i 'invisible kept) (get-text-property i 'invisible expected)))
            (let ((map (get-text-property i 'keymap kept)))
              (should (eq (if (symbolp map) (symbol-function map) map) (get-text-property i 'keymap expected)))))))
      (kill-buffer chat))))

(ert-deftest romacs-agent-drawn-turn-opens-its-output-through-its-row ()
  ;; A call folded in a kept turn names its output; opened, the output is
  ;; read through its turn's row.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-drawn-tests--with 2
    (should (= (romacs-agent-turns-fill session) 2))
    (let ((form (car (last (romacs-agent-drawn-tests--forms session 1)))))
      (should (string-search ":id \"u2\"" form))
      (with-temp-buffer
        (romacs-agent-chat-mode)
        (setq romacs-agent-ui--session session)
        (romacs-agent-ui--reset)
        (setq romacs-agent-ui--end (point-marker))
        (romacs-agent-ui--insert-drawn form)
        (should (= romacs-agent-ui--end (point-max)))
        (should-not (string-search romacs-agent-drawn-tests--long (buffer-string)))
        (should (romacs-agent-ui--open-payload "u2"))
        (should (string-search romacs-agent-drawn-tests--long (buffer-string)))))))

(ert-deftest romacs-agent-drawn-table-keeps-forms-by-version ()
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((db (sqlite-open)))
    (romacs-agent-drawn-put db "s" 2 "(\"a\" [] [])")
    (romacs-agent-drawn-put db "s" 9 "(\"b\" [] [])")
    (let ((found (romacs-agent-drawn-get db "s" '(2 9 30))))
      (should (equal (gethash 2 found) (cons romacs-agent-drawn-version "(\"a\" [] [])")))
      (should-not (gethash 30 found)))
    (should-not (romacs-agent-drawn-stale db))
    (sqlite-execute db "update turn_drawn set version = 0 where first_seq = 9")
    (should (equal (romacs-agent-drawn-stale db) '(("s" 9))))
    (sqlite-close db)))

(ert-deftest romacs-agent-drawn-gaps-wait-then-swap-in ()
  ;; A chat opened with turns no worker drew yet: a row each, wanted
  ;; first; kept, each takes its row's place, as drawn.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-drawn-tests--with 3
    (should (= (romacs-agent-turns-fill session) 3))
    (let* ((wanted nil)
           (romacs-agent-drawn-wanted-functions (list (lambda (id seqs) (push (cons id seqs) wanted))))
           (rows (plist-get (romacs-agent-turns-page session nil 10) :rows))
           (drawn (romacs-agent-drawn-tests--drawn session)))
      (should (equal (cdar wanted) (sort (mapcar (lambda (row) (plist-get row :first-seq)) rows) #'<)))
      (should (= (cl-count ?⋯ (car drawn)) 3))
      ;; Kept by a worker: they take the rows' places.
      (dolist (row rows)
        (romacs-agent-drawn-put (romacs-agent-turns--db) id (plist-get row :first-seq) (romacs-agent-ui-draw-turn session row)))
      (setf (romacs-agent-buffer session) (cdr drawn))
      (romacs-agent-ui-drawn-ready id (mapcar (lambda (row) (plist-get row :first-seq)) rows))
      (with-current-buffer (cdr drawn)
        (let ((text (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
          (should-not (string-search "⋯" text))
          (should (equal text (car (romacs-agent-drawn-tests--drawn session t))))))
      (kill-buffer (cdr drawn)))))

(ert-deftest romacs-agent-drawn-a-live-turn-is-kept-as-drawn ()
  ;; A turn drawn live is kept as it was drawn at its end, by its prompt's
  ;; seq, and a chat opened later inserts it.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-drawn-tests--with 1
    (should (= (romacs-agent-turns-fill session) 1))
    (let ((buffer (romacs-agent-ui--make-chat session)))
      (with-current-buffer buffer
        (setq romacs-agent-ui--unfilled nil)
        (dolist (event (list (list :type 'input :text "live one" :request 2 :seq 40 :last t)
                             (list :type 'delta :text "said live" :message "m9" :seq 41 :last t)
                             (list :type 'done :duration 1.0 :seq 42 :last t)))
          (romacs-agent-ui--render session event)))
      (let ((kept (gethash 40 (romacs-agent-drawn-get (romacs-agent-turns--db) id '(40)))))
        (should kept)
        (with-temp-buffer
          (romacs-agent-drawn-insert (cdr kept))
          (should (string-prefix-p "> live one\nsaid live\n" (buffer-string)))))
      (kill-buffer buffer))))

;;; romacs-agent-drawn-tests.el ends here
