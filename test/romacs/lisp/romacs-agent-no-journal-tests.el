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
  (require 'romacs-agent-log))
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

(ert-deftest romacs-agent-no-journal-opens-from-the-store-alone ()
  ;; Moved: its chat opens with its journal never read.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-no-journal-tests--with
    (romacs-agent-test-stored session)
    (cl-letf (((symbol-function 'romacs-agent--read-journal-file) (lambda (&rest _) (error "Read the journal")))
              ((symbol-function 'romacs-agent--journal-file-head) (lambda (&rest _) (error "Read the journal")))
              ((symbol-function 'romacs-agent--journal-file-after) (lambda (&rest _) (error "Read the journal"))))
      (let ((buffer (romacs-agent-log-chat session)))
        (with-current-buffer buffer
          (should (string-search "ask 1" (buffer-string)))
          (should (string-search "say 2" (buffer-string))))
        (kill-buffer buffer)))))

(provide 'romacs-agent-no-journal-tests)
;;; romacs-agent-no-journal-tests.el ends here
