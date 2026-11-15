;;; romacs-agent-turns-tests.el --- A session's turns, a row each -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-turns))

(defun romacs-agent-turns-tests--records (turns)
  "TURNS turns' records, each a table: a run note, then a prompt, a tool's
call and result, an answer and a result a turn."
  (let ((seq 0) (records nil))
    (cl-flet ((add (key value) (push (romacs-mcp--object key value "seq" (cl-incf seq) "t" (+ 1794600000.25 seq)) records)))
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
                                                                                                             "content" "found")))))
          (add "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "id" (format "m%d" n)
                                                                       "content" (vector (romacs-mcp--object "type" "text" "text" (format "answer %d about zebras" n))))))
          (add "out" (romacs-mcp--object "type" "result" "result" (format "answer %d about zebras" n))))))
    (nreverse records)))

(ert-deftest romacs-agent-turns-a-row-a-turn-and-the-tail-from-them ()
  ;; Folded as they come, a session's turns are rows; where its last turns
  ;; begin comes from them, the place the scan of its journal finds; its
  ;; replies are searched.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-turns-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (id "a1a1a1a1-2222-4333-8444-555566667777")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (records (romacs-agent-turns-tests--records 12)))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root "title" "turns"
                                                        "titleAuto" :false "argv" [] "options" (romacs-mcp--object)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) records "")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root
                                             :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
            (puthash id session romacs-agent--sessions)
            ;; As they come: the turns' rows are written.
            (dolist (record records) (romacs-agent--record session record))
            (should (= (romacs-agent-turns-count session) 12))
            (let ((rows (sqlite-select (romacs-agent-turns--db)
                                       "select first_seq, last_seq, prompt, reply, tools from turns where session = ? order by first_seq limit 1"
                                       (list id))))
              (should (equal rows '((2 6 "question 1" "answer 1 about zebras" "Bash · grep 1 · ok")))))
            ;; The tail from the rows: where the journal's scan says.
            (let ((from-rows (romacs-agent--turns-tail session 3))
                  (scanned (romacs-agent--journal-file-tail session 3)))
              (should from-rows)
              (should (equal (car from-rows) (car scanned))))
            ;; Fewer turns than asked, or none earlier: no cut.
            (should-not (romacs-agent--turns-tail session 12))
            (should (equal (sort (mapcar #'cadr (romacs-agent-turns-search "zebras" 50)) #'<) (number-sequence 2 57 5)))
            ;; Turns that came with no editor there (records with no rows):
            ;; caught up from where the rows end, the others not written again.
            (sqlite-execute (romacs-agent-turns--db) "delete from turns where session = ? and first_seq > 42" (list id))
            (should (= (romacs-agent-turns-count session) 9))
            (should (= (romacs-agent-turns-fill session) 3))
            (should (= (romacs-agent-turns-count session) 12))
            (should (= (romacs-agent-turns-fill session) 0))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

;;; romacs-agent-turns-tests.el ends here
