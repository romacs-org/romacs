;;; romacs-agent-store-tests.el --- Agent sessions listed from SQLite -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-store))

(defmacro romacs-agent-store-tests--with (&rest body)
  "BODY with a state root of its own (`root'), a registry of its own, and
`record' recording a session there (ID TITLE &optional RUNNING), its
journal one turn long."
  (declare (indent 0))
  `(let* ((root (file-name-as-directory (make-temp-file "romacs-store-" t)))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-event-functions nil)
          (romacs-agent-store--db nil))
     (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root))
               ((symbol-function 'romacs-agent--host-socket) (lambda (id) (expand-file-name (concat "run/" id ".sock") root))))
       (cl-flet ((record (id title &optional running)
                   (let ((dir (expand-file-name id root)))
                     (make-directory dir t)
                     (write-region (json-serialize (romacs-mcp--object "id" id "harness" "claude" "title" title "cwd" "/p/one/"))
                                   nil (expand-file-name "manifest.json" dir) nil 'silent)
                     (with-temp-file (expand-file-name "journal.jsonl" dir)
                       (insert (json-serialize (romacs-mcp--object "seq" 1 "t" 100.0 "in" (romacs-mcp--object "prompt" "hello"))) "\n"
                               (json-serialize (romacs-mcp--object "seq" 2 "t" 101.0 "ev" (romacs-mcp--object "type" "result" "result" "done it")))
                               "\n"))
                     (when running
                       (make-directory (expand-file-name "run" root) t)
                       (write-region "" nil (romacs-agent--host-socket id) nil 'silent)))))
         (unwind-protect (progn ,@body)
           (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))
           (delete-directory root t))))))

(ert-deftest romacs-agent-store-fills-the-unsaved ()
  ;; Each session with no kept state gets it and a row, its journal folded
  ;; once; one whose supervisor runs is its server's to load; filled, none
  ;; is unsaved, and the rows say what the kept states say.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (record "aaaaaaaa-0000-4000-8000-000000000001" "one")
    (record "aaaaaaaa-0000-4000-8000-000000000002" "two")
    (record "aaaaaaaa-0000-4000-8000-000000000003" "live" t)
    (should (= (length (romacs-agent-store--unsaved)) 2))
    (should (= (romacs-agent-store-fill) 2))
    (should-not (romacs-agent-store--unsaved))
    (dolist (id '("aaaaaaaa-0000-4000-8000-000000000001" "aaaaaaaa-0000-4000-8000-000000000002"))
      (should (file-exists-p (expand-file-name "state.eld" (expand-file-name id root)))))
    ;; Filled sessions are not left loaded: the batch process holds none.
    (should (= (hash-table-count romacs-agent--sessions) 0))
    (let ((rows (sqlite-select (romacs-agent-store--open)
                               "select id, title, cwd, seq from sessions order by id")))
      (should (equal rows '(("aaaaaaaa-0000-4000-8000-000000000001" "one" "/p/one/" 2)
                            ("aaaaaaaa-0000-4000-8000-000000000002" "two" "/p/one/" 2)))))))

(ert-deftest romacs-agent-store-row-follows-the-kept-state ()
  ;; A kept state writes its session's row, a later one replacing it.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-store-tests--with
    (let ((session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000001" :harness 'claude
                                       :title "first" :cwd "/p/two/" :dir (expand-file-name "b" root) :seq 3)))
      (make-directory (romacs-agent-dir session))
      (puthash (romacs-agent-id session) session romacs-agent--sessions)
      (romacs-agent--keep session 'idle)
      (setf (romacs-agent-title session) "second" (romacs-agent-seq session) 4)
      (romacs-agent--keep session 'idle)
      (should (equal (sqlite-select (romacs-agent-store--open) "select title, state, seq from sessions")
                     '(("second" "idle" 4)))))))

;;; romacs-agent-store-tests.el ends here
