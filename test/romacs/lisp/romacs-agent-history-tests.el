;;; romacs-agent-history-tests.el --- The harnesses' transcripts, listed from rows -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-history))

(defconst romacs-agent-history-tests--id "c1c1c1c1-2222-4333-8444-555566667777")

(defun romacs-agent-history-tests--transcript (dir id prompt)
  "Write a Claude Code transcript of session ID under DIR, its first PROMPT."
  (let ((project (expand-file-name "projects/-p-one" dir)))
    (make-directory project t)
    (with-temp-file (expand-file-name (concat id ".jsonl") project)
      (insert (json-serialize (romacs-mcp--object "type" "user" "cwd" "/p/one" "sessionId" id
                                                  "message" (romacs-mcp--object "role" "user" "content" prompt)))
              "\n"))
    (expand-file-name (concat id ".jsonl") project)))

(ert-deftest romacs-agent-history-listed-from-rows-the-scan-aside ()
  ;; The scan writes a row a transcript; a listing reads the rows, as the
  ;; scan listed them, and starts the next scan aside; a transcript gone
  ;; loses its row.  In a batch process the listing is scanned there.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-history-" t))
         (config (expand-file-name "claude" root))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")) process-environment))
         (romacs-agent-store--db nil) (romacs-agent-history--made nil)
         (romacs-agent-history--listing nil) (romacs-agent-history--descriptions (make-hash-table :test #'equal)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-history--places) (lambda () (list (list config 'claude "claude" config)))))
          (make-directory (romacs-agent--state-root) t)
          (let ((path (romacs-agent-history-tests--transcript config romacs-agent-history-tests--id "hello there")))
            (should (= (romacs-agent-history-index) 1))
            (let ((scanned (romacs-agent-history--scan)))
              (romacs-agent-history--from-rows)
              (let ((item (car romacs-agent-history--listing)))
                (should (= (length romacs-agent-history--listing) 1))
                (should (equal (plist-get item :id) romacs-agent-history-tests--id))
                (should (equal (plist-get item :title) "hello there"))
                (should (equal (plist-get item :cwd) "/p/one"))
                (should (eq (plist-get item :harness) 'claude))
                (should (equal (plist-get item :path) path))
                (should (equal (plist-get item :id) (plist-get (car scanned) :id)))))
            ;; In an editor: the rows, the next scan aside.
            (let ((noninteractive nil) (asked 0))
              (cl-letf (((symbol-function 'romacs-agent-history-index-soon) (lambda () (cl-incf asked)))
                        ((symbol-function 'romacs-agent-history--scan) (lambda () (error "Scanned here"))))
                (romacs-agent-history-list))
              (should (= asked 1))
              (should (= (length romacs-agent-history--listing) 1)))
            ;; Gone: its row too.
            (delete-file path)
            (should (= (romacs-agent-history-index) 0))
            (romacs-agent-history--from-rows)
            (should-not romacs-agent-history--listing)
            ;; In a batch process: scanned there.
            (romacs-agent-history-tests--transcript config romacs-agent-history-tests--id "again")
            (romacs-agent-history-list)
            (should (equal (plist-get (car romacs-agent-history--listing) :title) "again"))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(defun romacs-agent-history-tests--line (object)
  (concat (json-serialize object) "\n"))

(defun romacs-agent-history-tests--turn (n &optional tool)
  "Lines of Claude turn N: its prompt, a TOOL call and result, its answer."
  (concat
   (romacs-agent-history-tests--line
    (romacs-mcp--object "type" "user" "cwd" "/p/one" "timestamp" (format "2026-11-16T00:0%d:00Z" n)
                        "message" (romacs-mcp--object "role" "user" "content" (format "question %d" n))))
   (if tool
       (concat
        (romacs-agent-history-tests--line
         (romacs-mcp--object "type" "assistant" "timestamp" (format "2026-11-16T00:0%d:10Z" n)
                             "message" (romacs-mcp--object "id" (format "a%d" n) "model" "claude-x" "role" "assistant"
                                                           "content" (vector (romacs-mcp--object "type" "tool_use" "id" tool "name" "Bash"
                                                                                                 "input" (romacs-mcp--object "command" "ls -la"))))))
        (romacs-agent-history-tests--line
         (romacs-mcp--object "type" "user" "timestamp" (format "2026-11-16T00:0%d:20Z" n)
                             "message" (romacs-mcp--object "role" "user"
                                                           "content" (vector (romacs-mcp--object "type" "tool_result" "tool_use_id" tool
                                                                                                 "content" (make-string 3000 ?o)))))))
     "")
   (romacs-agent-history-tests--line
    (romacs-mcp--object "type" "assistant" "timestamp" (format "2026-11-16T00:0%d:30Z" n)
                        "message" (romacs-mcp--object "id" (format "m%d" n) "model" "claude-x" "role" "assistant"
                                                      "content" (vector (romacs-mcp--object "type" "text" "text" (format "answer %d" n))))))))

(ert-deftest romacs-agent-history-a-transcripts-turns-are-rows ()
  ;; Indexed, a transcript's turns are rows: a page of them, newest first,
  ;; their replies and items, a tool's input and output read from the
  ;; lines its item names.  Grown, only its last turn on is read again.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-turns)
  (let* ((root (make-temp-file "romacs-history-" t))
         (config (expand-file-name "claude" root))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")) process-environment))
         (romacs-agent-store--db nil) (romacs-agent-history--made nil) (romacs-agent-turns--made nil)
         (romacs-agent-history--listing nil) (romacs-agent-history--descriptions (make-hash-table :test #'equal)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-history--places) (lambda () (list (list config 'claude "claude" config)))))
          (make-directory (romacs-agent--state-root) t)
          (let* ((project (expand-file-name "projects/-p-one" config))
                 (path (expand-file-name (concat romacs-agent-history-tests--id ".jsonl") project)))
            (make-directory project t)
            (with-temp-file path (insert (romacs-agent-history-tests--turn 1 "t1") (romacs-agent-history-tests--turn 2)))
            (should (= (romacs-agent-history-index) 1))
            (romacs-agent-history--from-rows)
            (let* ((item (car romacs-agent-history--listing))
                   (page (romacs-agent-history-page item))
                   (rows (plist-get page :rows)))
              (should (= (length rows) 2))
              (should (equal (plist-get (car rows) :prompt) "question 2"))
              (should (equal (plist-get (cadr rows) :reply) "answer 1"))
              (let ((tool (cl-find "tool" (plist-get (cadr rows) :items) :key (lambda (i) (gethash "kind" i)) :test #'equal)))
                (should (equal (gethash "id" tool) "t1"))
                (should (= (gethash "output_bytes" tool) 3000))
                (let ((opened (romacs-agent-history-tool item "t1")))
                  (should (equal (gethash "command" (plist-get opened :input)) "ls -la"))
                  (should (equal (plist-get opened :output) (make-string 3000 ?o)))))
              ;; Grown by a turn: the last read again, the new one added.
              (write-region (romacs-agent-history-tests--turn 3) nil path t 'silent)
              (romacs-agent-history--index-turns item)
              (let ((rows (plist-get (romacs-agent-history-page item) :rows)))
                (should (equal (mapcar (lambda (r) (plist-get r :prompt)) rows) '("question 3" "question 2" "question 1"))))
              ;; Unchanged: nothing read.
              (cl-letf (((symbol-function 'romacs-agent-history--read) (lambda (&rest _) (error "Read again"))))
                (romacs-agent-history--index-turns item)))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(ert-deftest romacs-agent-history-a-scan-gets-the-editors-settings ()
  ;; A batch process reads no init: it is given the providers, T3's
  ;; settings file and the listing's size, readable as they are.
  (let ((romacs-agent-providers '(t3 (:name "x" :harness claude)))
        (romacs-agent-history-limit 7))
    (let ((form (car (read-from-string (romacs-agent-history--settings)))))
      (let ((romacs-agent-providers nil) (romacs-agent-history-limit nil))
        (eval form t)
        (should (equal romacs-agent-providers '(t3 (:name "x" :harness claude))))
        (should (= romacs-agent-history-limit 7))))))

(provide 'romacs-agent-history-tests)
;;; romacs-agent-history-tests.el ends here
