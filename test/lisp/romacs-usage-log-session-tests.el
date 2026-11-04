;;; romacs-usage-log-session-tests.el --- A server's session record -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-usage-log)

(defmacro romacs-usage-log-test--isolated (&rest body)
  "BODY with a private state directory and the log's state as new."
  `(let* ((state (make-temp-file "romacs-usage-log-" t))
          (process-environment (append (list (concat "XDG_STATE_HOME=" state) "ROMACS_USAGE_LOG")
                                       process-environment))
          (romacs-usage-log-file nil) (romacs-usage-log--pending nil) (romacs-usage-log--session nil)
          (romacs-usage-log--slow (make-hash-table :test 'equal))
          (romacs-usage-log--traced (make-hash-table :test 'equal))
          (romacs-usage-log--captured (make-hash-table :test 'equal))
          (romacs-usage-log--capture nil) (romacs-usage-log--settling nil))
     (unwind-protect (progn ,@body)
       (romacs-usage-log-stop)
       (delete-directory state t))))

(defun romacs-usage-log-test--lines (file)
  (with-temp-buffer
    (insert-file-contents file)
    (mapcar (lambda (line) (json-parse-string line :object-type 'plist))
            (split-string (buffer-string) "\n" t))))

(ert-deftest romacs-usage-log-server-keeps-its-own-session-record ()
  (romacs-usage-log-test--isolated
   (let* ((root (expand-file-name "romacs/sessions/" state)))
     (make-directory root t)
     ;; Fifty older records: the oldest goes, but a live process's (pid 1,
     ;; another server running) stays however old.
     (dotimes (i 50) (make-directory (expand-file-name (format "20200101T0000%02d-%d" i (+ 1000000 i)) root)))
     (make-directory (expand-file-name "20190101T000000-1" root))
     (let* ((noninteractive nil)
            (directory (romacs-usage-log-server-session "work")))
       (should directory)
       (should (file-exists-p (expand-file-name "session.json" directory)))
       (should (equal romacs-usage-log-file (expand-file-name "usage.jsonl" directory)))
       (should (equal (plist-get (json-parse-string (with-temp-buffer (insert-file-contents (expand-file-name "romacs/server-work.session" state))
                                                                      (buffer-string))
                                                    :object-type 'plist)
                                 :directory)
                      directory))
       (should (= (length (directory-files root nil "\\`[0-9]")) 51))
       (should (file-exists-p (expand-file-name "20190101T000000-1" root)))
       (should-not (file-exists-p (expand-file-name "20200101T000000-1000000" root)))
       (let ((record (json-parse-string (with-temp-buffer (insert-file-contents (expand-file-name "session.json" directory))
                                                          (buffer-string))
                                        :object-type 'plist)))
         (should (equal (plist-get record :server) "work"))
         (should-not (plist-get record :end)))
       (romacs-usage-log--write-session t)
       (let ((record (json-parse-string (with-temp-buffer (insert-file-contents (expand-file-name "session.json" directory))
                                                          (buffer-string))
                                        :object-type 'plist)))
         (should (numberp (plist-get record :end)))
         ;; How it exited is the unit's to say: the hook cannot see it.
         (should-not (plist-member record :status)))))
   ;; A log named by the environment (a launcher's) is the one used.
   (let ((process-environment (cons "ROMACS_USAGE_LOG=/elsewhere" process-environment))
         (romacs-usage-log--session nil) (noninteractive nil) (romacs-usage-log-file nil))
     (should-not (romacs-usage-log-server-session "other")))))

(ert-deftest romacs-usage-log-slow-records-and-first-backtrace ()
  (romacs-usage-log-test--isolated
   (setq romacs-usage-log-file (expand-file-name "usage.jsonl" state))
   ;; Two slow commands of a kind, one fast: one record, n 2, total, max.
   (romacs-usage-log--settle "find-file" "fundamental-mode" (- (float-time) 0.2))
   (romacs-usage-log--settled)
   (romacs-usage-log--settle "find-file" "fundamental-mode" (- (float-time) 0.1))
   (romacs-usage-log--settled)
   (romacs-usage-log--settle "find-file" "fundamental-mode" (float-time))
   (romacs-usage-log--settled)
   ;; An error's first record of its kind has the functions it went through:
   ;; per command, so another command's first has its own, and caught
   ;; signals before it take nothing from it.
   (let ((signal-hook-function #'romacs-usage-log--signal))
     (let ((real-this-command 'romacs-usage-log-test-other))
       (dotimes (_ 3) (ignore-errors (car 2))))
     (let ((real-this-command 'romacs-usage-log-test-command))
       (dotimes (_ 2)
         (condition-case data (car 1)
           (error (romacs-usage-log--command-error data nil nil)))))
     (clrhash romacs-usage-log--captured)
     (let ((real-this-command 'romacs-usage-log-test-other))
       (condition-case data (car 2)
         (error (romacs-usage-log--command-error data nil nil)))))
   (romacs-usage-log-flush)
   (let* ((lines (romacs-usage-log-test--lines romacs-usage-log-file))
          (slow (seq-filter (lambda (l) (equal (plist-get l :kind) "slow")) lines))
          (errors (seq-filter (lambda (l) (equal (plist-get l :kind) "error")) lines)))
     (should (= (length slow) 1))
     (should (equal (plist-get (car slow) :command) "find-file"))
     (should (= (plist-get (car slow) :n) 2))
     (should (>= (plist-get (car slow) :max) 190))
     (should (>= (plist-get (car slow) :ms) 290))
     (should (= (length errors) 3))
     (should (vectorp (plist-get (nth 0 errors) :backtrace)))
     (should (seq-every-p #'stringp (plist-get (nth 0 errors) :backtrace)))
     (should-not (plist-get (nth 1 errors) :backtrace))
     (should (equal (plist-get (nth 2 errors) :command) "romacs-usage-log-test-other"))
     (should (vectorp (plist-get (nth 2 errors) :backtrace))))))

(ert-deftest romacs-usage-log-backtrace-is-its-own-signals ()
  ;; A signal a command caught (`kill-whole-line' at an unterminated last
  ;; line catches end-of-buffer, its data nil) lends its frames neither to
  ;; another kind's record (a quit's, its data nil too) nor to a later
  ;; command's of the same kind.
  (romacs-usage-log-test--isolated
   (setq romacs-usage-log-file (expand-file-name "usage.jsonl" state))
   (let ((signal-hook-function #'romacs-usage-log--signal)
         (real-this-command 'romacs-usage-log-test-kill))
     (romacs-usage-log--pre-command)
     (ignore-errors (signal 'end-of-buffer nil))
     (romacs-usage-log--command-error '(quit) nil nil)
     (romacs-usage-log--post-command)
     ;; Caught, the command ends well: its capture is not the next one's.
     (clrhash romacs-usage-log--captured)
     (romacs-usage-log--pre-command)
     (ignore-errors (signal 'end-of-buffer nil))
     (romacs-usage-log--post-command)
     (romacs-usage-log--pre-command)
     (romacs-usage-log--command-error '(end-of-buffer) nil nil)
     (romacs-usage-log--post-command))
   (romacs-usage-log-flush)
   (let ((errors (seq-filter (lambda (l) (equal (plist-get l :kind) "error"))
                             (romacs-usage-log-test--lines romacs-usage-log-file))))
     (should (= (length errors) 2))
     (should-not (seq-some (lambda (l) (plist-get l :backtrace)) errors)))))

(provide 'romacs-usage-log-session-tests)
;;; romacs-usage-log-session-tests.el ends here
