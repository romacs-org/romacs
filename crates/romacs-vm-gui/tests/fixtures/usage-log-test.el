;;; usage-log-test.el --- The session's command log  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; ERT tests of rlisp/romacs-usage-log.el, which tests/usage_log.rs runs in
;; Romacs's batch session and in pinned GNU's, with the library loaded first.

(require 'ert)
(require 'seq)
(require 'cl-lib)

(defmacro usage-log-test--logging (file &rest body)
  "Run BODY logging to FILE, a fresh file, then stop; the log's lines, parsed."
  (declare (indent 1))
  `(let ((,file (make-temp-file "usage-log-"))
         (romacs-usage-log--counts (make-hash-table :test 'eq))
         (romacs-usage-log--pending nil)
         (romacs-usage-log--open nil)
         (command-error-function command-error-function)
         (pre-command-hook nil) (post-command-hook nil) (kill-emacs-hook nil) (after-init-hook nil))
     (unwind-protect
         (progn
           (romacs-usage-log--enable ,file)
           ,@body
           (romacs-usage-log-flush)
           (with-temp-buffer
             (insert-file-contents ,file)
             (mapcar (lambda (line) (json-parse-string line :object-type 'plist))
                     (split-string (buffer-string) "\n" t))))
       (romacs-usage-log-stop)
       (delete-file ,file))))

(defun usage-log-test--command (command mode &optional ms)
  "As the command loop runs COMMAND in MODE, taking MS milliseconds;
with MS `open', COMMAND only starts."
  (let ((major-mode mode) (real-this-command command))
    (romacs-usage-log--pre-command)
    (unless (eq ms 'open)
      (when ms (aset (car romacs-usage-log--open) 1 (- (aref (car romacs-usage-log--open) 1) (/ ms 1000.0))))
      (romacs-usage-log--post-command))))

(defun usage-log-test--kind (kind lines)
  (seq-filter (lambda (line) (equal (plist-get line :kind) kind)) lines))

(ert-deftest usage-log-counts-each-command-per-major-mode ()
  (let* ((lines (usage-log-test--logging file
                  (dotimes (_ 3) (usage-log-test--command 'next-line 'text-mode))
                  (usage-log-test--command 'next-line 'org-mode)
                  (usage-log-test--command (lambda () (interactive)) 'org-mode)))
         (counts (seq-filter (lambda (line) (plist-get line :n)) (usage-log-test--kind "command" lines))))
    (should (member '(:kind "command" :command "next-line" :mode "text-mode" :n 3) counts))
    (should (member '(:kind "command" :command "next-line" :mode "org-mode" :n 1) counts))
    (should (member '(:kind "command" :command "anonymous" :mode "org-mode" :n 1) counts))
    (should (= (length counts) 3))))

(ert-deftest usage-log-writes-counts-since-the-last-write ()
  (let ((lines (usage-log-test--logging file
                 (usage-log-test--command 'forward-char 'text-mode)
                 (romacs-usage-log-flush)
                 (usage-log-test--command 'forward-char 'text-mode))))
    (should (equal (mapcar (lambda (line) (plist-get line :n))
                                  (seq-filter (lambda (line) (plist-get line :n)) lines))
                          '(1 1)))))

(ert-deftest usage-log-logs-a-slow-command-with-its-milliseconds ()
  (let* ((lines (usage-log-test--logging file
                  (usage-log-test--command 'fast-command 'text-mode 5)
                  (usage-log-test--command 'slow-command 'text-mode 250)))
         (slow (seq-filter (lambda (line) (plist-get line :ms)) lines)))
    (should (= (length slow) 1))
    (should (equal (plist-get (car slow) :command) "slow-command"))
    (should (equal (plist-get (car slow) :mode) "text-mode"))
    (should (>= (plist-get (car slow) :ms) 250))))

(ert-deftest usage-log-counts-a-command-around-its-minibuffer-read-once
    ()
  ;; OUTER reads a string: the recursive edit's first post-command run at
  ;; depth 1 ends nothing; the read's keys are minibuffer commands; the
  ;; read's exit leaves its own command open at depth 1, and OUTER is timed
  ;; from it, not from before the read.
  (let* ((depth 0)
         (lines (cl-letf (((symbol-function 'recursion-depth) (lambda () depth)))
                  (usage-log-test--logging file
                    (usage-log-test--command 'outer 'text-mode 'open)
                    (aset (car romacs-usage-log--open) 1 (- (float-time) 5))
                    (setq depth 1)
                    (let ((real-this-command 'outer)) (romacs-usage-log--post-command))
                    (usage-log-test--command 'self-insert-command 'minibuffer-mode)
                    (usage-log-test--command 'exit-minibuffer 'minibuffer-mode 'open)
                    (aset (car romacs-usage-log--open) 1 (- (float-time) 0.2))
                    (setq depth 0)
                    (let ((real-this-command 'outer)) (romacs-usage-log--post-command)))))
         (counts (seq-filter (lambda (line) (plist-get line :n)) lines))
         (slow (seq-filter (lambda (line) (plist-get line :ms)) lines)))
    (should (equal (sort (mapcar (lambda (line) (list (plist-get line :command) (plist-get line :mode) (plist-get line :n))) counts)
                         (lambda (a b) (string< (car a) (car b))))
                   '(("outer" "text-mode" 1) ("self-insert-command" "minibuffer-mode" 1))))
    (should (= (length slow) 1))
    (should (equal (plist-get (car slow) :mode) "text-mode"))
    (should (< 150 (plist-get (car slow) :ms) 1000))))

(ert-deftest usage-log-logs-a-command-error-and-keeps-the-reporting ()
  (let* ((reported nil)
         (lines (usage-log-test--logging file
                  (setq command-error-function (lambda (data _context _signal) (setq reported data)))
                  (romacs-usage-log--init-done)
                  (let ((real-this-command 'find-file))
                    (funcall command-error-function
                             '(file-missing "Opening input file" "No such file or directory" "/home/someone/secret.org")
                             "" nil))))
         (errors (usage-log-test--kind "error" lines)))
    (should (equal (car reported) 'file-missing))
    (should (equal errors
                          '((:kind "error" :command "find-file"
                             :error "(file-missing . \"<file>\")"))))
    (should (member "init-done" (mapcar (lambda (line) (plist-get line :event))
                                               (usage-log-test--kind "session" lines))))))

(ert-deftest usage-log-scrubs-file-names-and-long-messages ()
  (dolist (text '("Cannot open ~/notes/a.org: denied" "No such file, /home/me/Private Notes/secret.txt"
                  "Cannot read /home/Bob's file.txt" "notes/secret.txt is missing"
                  "Cannot read My (Private) Notes/secret file.txt, sorry" "Cannot read Clients'/secret.txt"))
    (should (equal (romacs-usage-log--scrub text) "<file>")))
  (should (equal (romacs-usage-log--scrub "Search failed: \"plain words\"") "Search failed: \"plain words\""))
  (should (= (length (romacs-usage-log--scrub (make-string 500 ?x))) 200)))

(ert-deftest usage-log-says-when-the-session-starts-and-ends ()
  (let ((lines (usage-log-test--logging file
                 (run-hooks 'kill-emacs-hook))))
    (should (equal (mapcar (lambda (line) (plist-get line :event)) (usage-log-test--kind "session" lines))
                          '("start" "kill")))
    (should (numberp (plist-get (car lines) :seconds)))))

(ert-deftest usage-log-starts-only-when-the-environment-names-a-file ()
  (let ((process-environment (cons "ROMACS_USAGE_LOG" process-environment))
        (romacs-usage-log-file nil))
    (romacs-usage-log-start)
    (should (null romacs-usage-log-file)))
  ;; A batch session never logs.
  (let ((process-environment (cons "ROMACS_USAGE_LOG=/nonexistent/usage.jsonl" process-environment))
        (romacs-usage-log-file nil)
        (noninteractive t))
    (romacs-usage-log-start)
    (should (null romacs-usage-log-file))))
