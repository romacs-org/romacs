;;; parity-probe-tests.el --- The live harnesses' probe answers every exit  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The probe the live harnesses load (crates/romacs-vm-gui/harness/
;; parity_probe.el) once deleted a request before evaluating it and
;; answered only an `error': a `quit' or a `throw' out of the form left
;; neither request nor answer, and the harness waited out its timeout.

;;; Code:

(require 'ert)
(require 'cl-lib)

(defvar parity-probe-tests--dir (make-temp-file "parity-probe-" t))
(add-hook 'kill-emacs-hook (lambda () (delete-directory parity-probe-tests--dir t)))
(let ((process-environment (cons (concat "PARITY_PROBE_DIR=" parity-probe-tests--dir) process-environment)))
  (load (expand-file-name "../../crates/romacs-vm-gui/harness/parity_probe.el"
                          (file-name-directory (or load-file-name buffer-file-name)))
        nil t))
;; The tests call the poll themselves.
(cancel-timer parity-probe--timer)

(defvar parity-probe-tests--evaluations 0)

(defun parity-probe-tests--ask (id form)
  "Write request ID for FORM, poll once, and return (ANSWER . FILES-LEFT):
the answer read, and which of the request and its claim remain."
  (let ((request (parity-probe--file "request.eld"))
        (claimed (parity-probe--file "request.busy"))
        (response (parity-probe--file "response.eld")))
    (when (file-exists-p response) (delete-file response))
    (with-temp-file request (prin1 (cons id form) (current-buffer)))
    (catch 'parity-probe-tests (parity-probe--poll))
    (cons (and (file-exists-p response)
               (with-temp-buffer (insert-file-contents response) (read (current-buffer))))
          (delq nil (list (and (file-exists-p request) 'request) (and (file-exists-p claimed) 'claimed))))))

(ert-deftest parity-probe-tests-answers-a-value-and-an-error ()
  (should (equal (parity-probe-tests--ask 1 '(+ 1 2)) '((1 ok 3))))
  (should (equal (parity-probe-tests--ask 2 '(error "boom")) '((2 error (error "boom"))))))

(ert-deftest parity-probe-tests-answers-a-quit ()
  "A `quit' is no `error': it is answered and named all the same."
  (should (equal (parity-probe-tests--ask 3 '(signal 'quit nil)) '((3 error (quit))))))

(ert-deftest parity-probe-tests-answers-a-throw-and-polls-again ()
  "A `throw' past the form is answered, and the poll is started afresh:
the timer the throw left would never fire again."
  (let ((timer-list nil))
    (should (equal (parity-probe-tests--ask 4 '(throw 'parity-probe-tests t))
                   '((4 error (non-local-exit)))))
    (should (seq-some (lambda (timer) (eq (timer--function timer) #'parity-probe--start)) timer-list))))

(ert-deftest parity-probe-tests-claims-a-request-until-it-is-answered ()
  "While the form runs, its request is claimed, so a later request is
never taken for it; afterwards only the answer is left."
  (let ((request (parity-probe--file "request.eld"))
        (claimed (parity-probe--file "request.busy")))
    (should (equal (parity-probe-tests--ask 5 `(list (file-exists-p ,request) (file-exists-p ,claimed)))
                   '((5 ok (nil t)))))))

(ert-deftest parity-probe-tests-answers-what-it-cannot-print-or-write-once ()
  "A value the answer cannot hold is told as such; and when no answer can
be written at all the form is not evaluated again: its claim stays."
  (let ((write (symbol-function 'parity-probe--write)) (refused nil))
    (cl-letf (((symbol-function 'parity-probe--write)
               (lambda (value) (if refused (funcall write value) (setq refused t) (error "refused")))))
      (should (equal (parity-probe-tests--ask 6 '(+ 1 2)) '((6 error (unprintable (error "refused"))))))))
  (setq parity-probe-tests--evaluations 0)
  (let* ((timer-list nil)
         (timer (run-with-timer 0 0.025 #'parity-probe--poll))
         (request (parity-probe--file "request.eld")))
    (with-temp-file request (prin1 '(7 . (cl-incf parity-probe-tests--evaluations)) (current-buffer)))
    (cl-letf (((symbol-function 'parity-probe--write) (lambda (_) (error "disk full")))
              (debug-on-error nil) (inhibit-message t))
      (timer-event-handler timer)
      (timer-event-handler timer))
    (should (= parity-probe-tests--evaluations 1))
    (should (and (memq timer timer-list) (not (timer--triggered timer))))
    (should (file-exists-p (parity-probe--file "request.busy")))
    ;; A new request takes a stale claim's place.
    (should (equal (parity-probe-tests--ask 8 '(+ 2 2)) '((8 ok 4))))))

(ert-deftest parity-probe-tests-answers-a-request-it-cannot-read ()
  (let ((request (parity-probe--file "request.eld"))
        (response (parity-probe--file "response.eld")))
    (when (file-exists-p response) (delete-file response))
    (with-temp-file request (insert "(9 . (unbalanced"))
    (parity-probe--poll)
    (should (equal (with-temp-buffer (insert-file-contents response) (car (read (current-buffer)))) nil))
    (should-not (file-exists-p request))))

(provide 'parity-probe-tests)
;;; parity-probe-tests.el ends here
