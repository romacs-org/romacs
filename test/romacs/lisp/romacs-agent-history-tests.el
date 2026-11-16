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
