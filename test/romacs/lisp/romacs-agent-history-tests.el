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

(defvar romacs-agent-t3-settings)

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
            (let ((noninteractive nil) (romacs-agent-writer-queued nil) (asked 0))
              (cl-letf (((symbol-function 'romacs-agent-history-index-soon) (lambda () (cl-incf asked)))
                        ((symbol-function 'romacs-agent-history--scan) (lambda () (error "Scanned here"))))
                (romacs-agent-history-list))
              (should (= asked 1))
              (should (= (length romacs-agent-history--listing) 1)))
            ;; A session that goes on with it, kept as its row alone (quiet,
            ;; or ended): not listed.
            (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
              (should (romacs-agent-history-items))
              (sqlite-execute (romacs-agent-store--open) "insert into sessions (id, dir, transcript) values ('row', '/r', ?)" (list path))
              (should-not (romacs-agent-history-items))
              (sqlite-execute (romacs-agent-store--open) "delete from sessions where id = 'row'"))
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
        (should (= romacs-agent-history-limit 7)))))
  ;; T3's settings file unset here: not given, the process keeps its default.
  (when (boundp 'romacs-agent-t3-settings)
    (let ((form (let ((romacs-agent-t3-settings "/x.json")) (romacs-agent-history--settings))))
      (should (string-search "\"/x.json\"" form))))
  (let ((form (cl-letf (((symbol-function 'boundp) (lambda (s) (not (eq s 'romacs-agent-t3-settings)))))
                (romacs-agent-history--settings))))
    (should-not (string-search "romacs-agent-t3-settings" form))))

(provide 'romacs-agent-history-tests)
;;; romacs-agent-history-tests.el ends here

(ert-deftest romacs-agent-history-an-import-without-its-transcript-makes-nothing ()
  ;; An import given no transcript, or one not there (a listing read before
  ;; its scan, an item gone since): refused before anything is made.
  (let* ((root (make-temp-file "romacs-import-" t))
         (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment)))
    (unwind-protect
        (progn
          (should-error (romacs-agent-import 'claude "aaaaaaaa-0000-4000-8000-000000000001" nil root "t" "claude" nil)
                        :type 'user-error)
          (should-error (romacs-agent-import 'claude "aaaaaaaa-0000-4000-8000-000000000001"
                                             (expand-file-name "gone.jsonl" root) root "t" "claude" nil)
                        :type 'user-error)
          (should-not (file-exists-p (romacs-agent--state-root))))
      (delete-directory root t))))

(ert-deftest romacs-agent-history-a-harness-defined-in-a-file-is-scanned-from-it ()
  ;; A harness an init or a package defines: its file is kept, and the
  ;; process that writes the transcripts' rows loads it, so its
  ;; transcripts are listed too.
  (let* ((file (make-temp-file "romacs-driver-" nil ".el"
                               "(romacs-agent-define-harness 'romacs-history-test-driver :display \"T\")\n"))
         (romacs-agent--drivers (copy-sequence romacs-agent--drivers))
         (romacs-agent--driver-table (copy-hash-table romacs-agent--driver-table)))
    (unwind-protect
        (progn
          (load file nil t)
          (should (equal (plist-get (romacs-agent--driver-of 'romacs-history-test-driver) :file) file))
          (should (member file (romacs-agent-history--driver-files))))
      (delete-file file))))

(ert-deftest romacs-agent-history-a-scan-loads-harnesses-with-what-they-require ()
  ;; The scan's process loads each harness's file with this editor's
  ;; `load-path', so a library it requires from elsewhere is found; one
  ;; that fails to load leaves the others.
  (let* ((root (make-temp-file "romacs-drivers-" t))
         (helpers (expand-file-name "helpers" root))
         (good (expand-file-name "good-driver.el" root))
         (bad (expand-file-name "bad-driver.el" root))
         (load-path (cons helpers load-path))
         (romacs-agent--drivers (list (list 'bad :file bad) (list 'good :file good))))
    (unwind-protect
        (progn
          (make-directory helpers)
          (write-region "(provide 'romacs-history-test-helper)\n" nil (expand-file-name "romacs-history-test-helper.el" helpers))
          (write-region "(require 'romacs-history-test-helper)\n(provide 'romacs-history-test-good)\n" nil good)
          (write-region "(error \"Broken driver\")\n" nil bad)
          (with-temp-buffer
            (call-process (expand-file-name invocation-name invocation-directory) nil t nil "--batch"
                          "--eval" (romacs-agent-history--drivers-form)
                          "--eval" "(princ (list (featurep 'romacs-history-test-helper) (featurep 'romacs-history-test-good)))")
            (should (string-search "(t t)" (buffer-string)))))
      (delete-directory root t))))
