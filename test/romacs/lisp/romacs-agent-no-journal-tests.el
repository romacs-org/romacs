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
  ;; Not moved yet: a row says so, it is asked first, its journal unread;
  ;; moved, the chat draws.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (let ((first nil))
      (cl-letf (((symbol-function 'romacs-agent-events-migrate-first) (lambda (id) (setq first id)))
                ((symbol-function 'romacs-agent--read-journal-file) (lambda (&rest _) (error "Read the journal"))))
        (let ((buffer (romacs-agent-no-journal-tests--chat session)))
          (should (equal first id))
          (should (string-search "move to the store" (with-current-buffer buffer (buffer-string))))
          (should-not (string-search "ask 1" (with-current-buffer buffer (buffer-string))))
          (kill-buffer buffer)))
      (let ((buffer (romacs-agent-no-journal-tests--chat session)))
        (romacs-agent-test-stored session)
        (run-hook-with-args 'romacs-agent-events-migrated-functions (list id))
        (with-current-buffer buffer
          (should-not (string-search "move to the store" (buffer-string)))
          (should (string-search "say 2" (buffer-string))))
        (kill-buffer buffer)))))

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
