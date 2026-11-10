;;; romacs-keeper-tests.el --- A session and its keeper -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Each test runs its own keeper on its own socket, started detached (no
;; systemd unit), and shuts it down.

(require (quote ert))
(require 'romacs-keeper)

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

(provide (quote romacs-keeper-tests))
;;; romacs-keeper-tests.el ends here
