;;; romacs-keeper-tests.el --- A session and its keeper -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Each test runs its own keeper on its own socket, started detached (no
;; systemd unit), and shuts it down.

(require (quote ert))
(require 'romacs-keeper)
(require 'seq)
(require 'subr-x)

(defmacro romacs-keeper-test--with (&rest body)
  "Run BODY attached to a fresh keeper, `socket' bound to its socket."
  (declare (indent 0))
  `(let* ((dir (make-temp-file "rk" t))
          (socket (expand-file-name "keeper.sock" dir))
          (romacs-keeper-use-systemd nil))
     (unwind-protect
         (progn (romacs-keeper--attach-socket socket) ,@body)
       (ignore-errors (romacs-keeper-shutdown))
       (delete-directory dir t))))

(defun romacs-keeper-test--drain (handle)
  "Everything HANDLE's first descriptor still gives, to end of file."
  (let ((chunks nil) chunk)
    (while (setq chunk (romacs-keeper-read handle 0 65536 10))
      (when (eq chunk t) (error "Nothing more from handle %s" handle))
      (push chunk chunks))
    (apply #'concat (nreverse chunks))))

(defun romacs-keeper-test--row (handle)
  (let (found)
    (dolist (row (romacs-keeper-handles) found)
      (when (eql (plist-get row :handle) handle) (setq found row)))))

(defconst romacs-keeper-test--counter
  "i=0; while [ $i -lt 30000 ]; do echo $i; i=$((i+1)); done; exit 7")

(defconst romacs-keeper-test--first-session
  '(progn
     (require 'romacs-keeper)
     (let ((romacs-keeper-use-systemd nil))
       (romacs-keeper--attach-socket socket)
       (let* ((facts (romacs-keeper-spawn
                      "test" 1 '("/bin/sh" "-c" "umask; pwd; echo \"$GREETING\"; grep SigBlk /proc/self/status; echo $PPID")
                      :environment '("PATH=/usr/bin:/bin" "GREETING=hello keeper") :directory directory
                      :umask #o027 :signal-mask (ash 1 9) :metadata "facts"))
              (counter (romacs-keeper-spawn "test" 1 (list "/bin/sh" "-c" script)
                                            :environment '("PATH=/usr/bin:/bin")))
              (handle (plist-get counter :handle))
              (read ""))
         (while (< (length read) 1000)
           (setq read (concat read (romacs-keeper-read handle 0 (- 1000 (length read)) 10))))
         ;; To a file: nothing buffered survives the kill below.
         (write-region (format "%S" (list (plist-get facts :handle) handle (plist-get counter :pid) read (emacs-pid)))
                       nil (expand-file-name "told" directory) nil 'silent)
         ;; Die as a crash does: no hook runs, nothing is closed.
         (signal-process (emacs-pid) 9)))))

(ert-deftest keeper-child-outlives-a-killed-session-and-a-second-reads-on ()
  (romacs-keeper-test--with
    (romacs-keeper-detach)
    (let* ((form `(let ((socket ,socket) (directory ,dir) (script ,romacs-keeper-test--counter))
                    ,romacs-keeper-test--first-session))
           (told (progn
                   (call-process (expand-file-name invocation-name invocation-directory) nil nil nil
                                 "-Q" "--batch" "--eval" (format "%S" form))
                   (with-temp-buffer
                     (insert-file-contents (expand-file-name "told" dir))
                     (car (read-from-string (buffer-string))))))
           (expected (mapconcat (lambda (i) (format "%d\n" i)) (number-sequence 0 29999) "")))
      (let ((facts (nth 0 told)) (counter (nth 1 told)) (pid (nth 2 told))
            (consumed (nth 3 told)) (first-session (nth 4 told)))
        (should (equal consumed (substring expected 0 1000)))
        (let ((keeper (plist-get (romacs-keeper--attach-socket socket) :pid)))
          (should (eql (romacs-session-pid) keeper))
          (let ((row (romacs-keeper-test--row counter)))
            ;; Blocked on a full pipe: alive, and owned by nobody alive.
            (should (equal (plist-get row :status) '(running)))
            (should (eql (plist-get row :owner) first-session))
            (should-not (plist-get row :owner-alive))
            (should (file-directory-p (format "/proc/%d" pid))))
          (romacs-keeper-claim counter)
          (should (equal (romacs-keeper-test--drain counter) (substring expected 1000)))
          (romacs-keeper-claim facts)
          (should (equal (split-string (romacs-keeper-test--drain facts) "\n" t)
                                (list "0027" dir "hello keeper" "SigBlk:\t0000000000000200" (number-to-string keeper))))
          (should (equal (plist-get (romacs-keeper-test--row facts) :metadata) "facts"))
          (let ((deadline (+ (float-time) 10)) status)
            (while (and (not status) (< (float-time) deadline))
              (dolist (event (romacs-keeper-events))
                (when (eql (plist-get (cdr event) :handle) counter)
                  (setq status (plist-get (cdr event) :status))))
              (unless status (sleep-for 0.01)))
            (should (equal status '(exited 7))))
          (should (equal (plist-get (romacs-keeper-test--row counter) :status) '(exited 7))))))))

(ert-deftest keeper-session-pid-is-the-keepers-while-attached ()
  (should (eql (romacs-session-pid) (emacs-pid)))
  (romacs-keeper-test--with
    (let ((keeper (plist-get (romacs-keeper-info) :pid)))
      (should-not (eql keeper (emacs-pid)))
      (should (eql (romacs-session-pid) keeper))
      (should-not (romacs-keeper-upgrade))
      (romacs-keeper-detach)
      (should (eql (romacs-session-pid) (emacs-pid)))
      (should-error (romacs-keeper-handles))
      (romacs-keeper--attach-socket socket))))

(ert-deftest keeper-handles-signals-and-slots ()
  (romacs-keeper-test--with
    (let* ((row (romacs-keeper-spawn "test" 2 '("/bin/sh" "-c" "cat; echo done")
                                     :environment '("PATH=/usr/bin:/bin")
                                     :descriptors '((0 . reads) (1 . writes) (2 . null)) :kept t))
           (handle (plist-get row :handle)))
      (should (equal (list (plist-get row :kind) (plist-get row :version) (plist-get row :kept) (plist-get row :open))
                            '("test" 2 t (0 1))))
      (should (eql (romacs-keeper-write handle 0 "through\n" 5) 8))
      (romacs-keeper-close-slot handle 0)
      (should (equal (plist-get (romacs-keeper-test--row handle) :open) '(1)))
      (should (equal (let ((chunks "") chunk)
                              (while (setq chunk (romacs-keeper-read handle 1 100 10))
                                (setq chunks (concat chunks chunk)))
                              chunks)
                            "through\ndone\n"))
      (should-error (romacs-keeper-read handle 0 1))
      (let ((sleeper (plist-get (romacs-keeper-spawn "test" 1 '("/bin/sleep" "600")) :handle)))
        (romacs-keeper-signal sleeper 15)
        (let ((deadline (+ (float-time) 10)))
          (while (and (equal (plist-get (romacs-keeper-test--row sleeper) :status) '(running)) (< (float-time) deadline))
            (sleep-for 0.01)))
        (should (equal (plist-get (romacs-keeper-test--row sleeper) :status) '(signaled 15)))
        (should-error (romacs-keeper-signal sleeper 15))
        (romacs-keeper-close sleeper)
        (should-not (romacs-keeper-test--row sleeper))))))

;; A program that does not read takes what its pipe holds and no more:
;; the write returns, short, instead of stopping the session.
(ert-deftest keeper-write-does-not-block-on-a-full-pipe ()
  (romacs-keeper-test--with
    (let* ((handle (plist-get (romacs-keeper-spawn "test" 1 '("/bin/sleep" "600")
                                                   :descriptors '((0 . reads) (1 . null) (2 . null)))
                              :handle))
           (megabyte (make-string (* 1024 1024) ?x))
           (sent (romacs-keeper-write handle 0 megabyte 5)))
      (should (< 0 sent (length megabyte)))
      (should (eql (romacs-keeper-write handle 0 megabyte 0.05) 0))
      (should-error (romacs-keeper--attach-socket socket)))))

;; A quit closes this session's handles; a restart leaves them, under the
;; deadline option when it is set.
(ert-deftest keeper-restart-leaves-handles-and-quit-closes-them ()
  (romacs-keeper-test--with
    (let ((handle (plist-get (romacs-keeper-spawn "test" 1 '("/bin/sleep" "600")) :handle))
          (romacs-keeper-unclaimed-deadline 3600))
      (let ((romacs-keeper-restarting t)) (romacs-keeper--leave))
      (should (romacs-keeper-attached-p))
      (let ((row (romacs-keeper-test--row handle)))
        (should (equal (plist-get row :status) '(running)))
        (should (< 3500 (- (plist-get row :deadline) (float-time)) 3700)))
      (romacs-keeper--leave)
      (should-not (romacs-keeper-attached-p))
      (romacs-keeper--attach-socket socket)
      (let ((deadline (+ (float-time) 10)))
        (while (and (romacs-keeper-test--row handle) (< (float-time) deadline)) (sleep-for 0.01)))
      (should-not (romacs-keeper-test--row handle)))))

;; The unit path, as a session started by systemd runs: the session is a
;; transient user unit holding a pty child through a keeper it started.
;; Killing the session's unit kills everything in its cgroup, and must not
;; reach the keeper or the child.  A late successor still finds the handle,
;; owns it alone, and the keeper leaves once nothing is left.
(defconst romacs-keeper-test--unit-session
  '(progn
     (require 'romacs-keeper)
     (let ((romacs-keeper-idle-seconds 1))
       (romacs-keeper--attach-socket socket)
       (romacs--keeper-call 'configure 1000)
       (let ((row (romacs-keeper-spawn
                   "test" 1 '("/bin/sh" "-c" "i=0; while :; do echo tick $i; i=$((i+1)); sleep 0.05; done")
                   :environment '("PATH=/usr/bin:/bin") :session t
                   :descriptors '((0 . pty) (1 . pty) (2 . pty)))))
         (write-region (format "%S" (list (plist-get row :handle) (plist-get row :pid)
                                          (romacs-keeper-attached-p) (emacs-pid)))
                       nil (expand-file-name "told" directory) nil 'silent)
         (while t (sleep-for 1))))))

(defun romacs-keeper-test--unit-of (pid)
  "The systemd unit PID runs in, from its cgroup."
  (with-temp-buffer
    (insert-file-contents (format "/proc/%d/cgroup" pid))
    (file-name-nondirectory (string-trim (buffer-string)))))

(ert-deftest keeper-outlives-a-killed-session-unit ()
  (skip-unless (and (executable-find "systemd-run")
                    (eql 0 (call-process "systemctl" nil nil nil "--user" "show-environment"))))
  (let* ((dir (make-temp-file "rk" t))
         (socket (expand-file-name "keeper.sock" dir))
         (unit (format "romacs-keeper-test-%d-%d" (emacs-pid) (random 100000)))
         (form `(let ((socket ,socket) (directory ,dir)) ,romacs-keeper-test--unit-session))
         (told-file (expand-file-name "told" dir)))
    (unwind-protect
        (progn
          (should (eql 0 (call-process "systemd-run" nil nil nil "--user" "--quiet" "--collect" "--unit" unit
                                       "--setenv" (concat "HOME=" (getenv "HOME"))
                                       "--setenv" (concat "ROMACS_LISP_DIR=" (or (getenv "ROMACS_LISP_DIR") ""))
                                       "--" (expand-file-name invocation-name invocation-directory)
                                       "-Q" "--batch" "-L" (file-name-directory (locate-library "romacs-keeper"))
                                       "--eval" (format "%S" form))))
          (let ((deadline (+ (float-time) 30)))
            (while (and (not (file-exists-p told-file)) (< (float-time) deadline)) (sleep-for 0.05)))
          (let* ((told (with-temp-buffer (insert-file-contents told-file) (car (read-from-string (buffer-string)))))
                 (handle (nth 0 told)) (child (nth 1 told)) (keeper (nth 2 told)) (session (nth 3 told)))
            ;; The keeper and the child are in a unit of the keeper's own.
            (should (equal (romacs-keeper-test--unit-of session) (concat unit ".service")))
            (should-not (equal (romacs-keeper-test--unit-of keeper) (concat unit ".service")))
            (should (equal (romacs-keeper-test--unit-of child) (romacs-keeper-test--unit-of keeper)))
            (call-process "systemctl" nil nil nil "--user" "kill" "--signal=KILL" (concat unit ".service"))
            (let ((deadline (+ (float-time) 10)))
              (while (and (file-directory-p (format "/proc/%d" session)) (< (float-time) deadline)) (sleep-for 0.05)))
            (should-not (file-directory-p (format "/proc/%d" session)))
            ;; A successor that is slow to come.
            (sleep-for 1.5)
            (should (file-directory-p (format "/proc/%d" keeper)))
            (should (file-directory-p (format "/proc/%d" child)))
            (should (eql (plist-get (romacs-keeper--attach-socket socket) :pid) keeper))
            (let ((row (romacs-keeper-test--row handle)))
              (should (equal (plist-get row :status) '(running)))
              (should-not (plist-get row :owner-alive)))
            (romacs-keeper-claim handle)
            (should (eql (plist-get (romacs-keeper-test--row handle) :owner) (emacs-pid)))
            ;; The shell on the pty kept printing: its ticks are there, in order.
            (let ((text "") (deadline (+ (float-time) 10)))
              (while (and (< (length (split-string text "\n" t)) 5) (< (float-time) deadline))
                (let ((chunk (romacs-keeper-read handle 0 4096 1)))
                  (when (stringp chunk) (setq text (concat text chunk)))))
              (let ((ticks (mapcar (lambda (line) (string-to-number (substring line 5)))
                                   (seq-filter (lambda (line) (string-match-p "\\`tick [0-9]+\r?\\'" line))
                                               (split-string text "\n" t)))))
                (should (>= (length ticks) 3))
                (should (equal ticks (number-sequence (car ticks) (+ (car ticks) (length ticks) -1))))))
            ;; Cleanup happens: a quit closes the handle, the shell gets its
            ;; hangup, and the idle keeper leaves with its unit.
            (romacs-keeper-detach t)
            (let ((deadline (+ (float-time) 15)))
              (while (and (file-directory-p (format "/proc/%d" keeper)) (< (float-time) deadline)) (sleep-for 0.1)))
            (should-not (file-directory-p (format "/proc/%d" child)))
            (should-not (file-directory-p (format "/proc/%d" keeper)))))
      (ignore-errors (call-process "systemctl" nil nil nil "--user" "stop" (concat unit ".service")))
      (ignore-errors (when (romacs-keeper-attached-p) (romacs-keeper-shutdown)))
      (delete-directory dir t))))

(provide (quote romacs-keeper-tests))
;;; romacs-keeper-tests.el ends here
