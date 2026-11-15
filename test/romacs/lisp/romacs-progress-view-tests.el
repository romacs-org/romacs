;;; romacs-progress-view-tests.el --- Tests for romacs-progress-view.el  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Code:

(require 'ert)
(require 'romacs-progress-view)

(defvar romacs-progress-state)

(ert-deftest romacs-progress-view-line-shows-a-step-as-it-goes ()
  "A running step: its words, its bar so far and its counts; a done one is
checked and full; a failed one says why, in the failure's face."
  (let ((running (romacs-progress-view--line 'restore '(:status running :done 2 :of 4 :name "init.el") 60))
        (done (romacs-progress-view--line 'agents '(:status done :done 3 :of 3) 60))
        (failed (romacs-progress-view--line 'restore '(:status failed :error "a damaged record") 60)))
    (should (string-match-p "Restoring buffers: init.el" running))
    (should (= (cl-count ?━ running) (/ romacs-progress-view--bar 2)))
    (should (string-suffix-p "2/4" running))
    (should (string-prefix-p "✓ Loading agent sessions" done))
    (should (= (cl-count ?━ done) romacs-progress-view--bar))
    (should (string-prefix-p "✗ " failed))
    (should (string-match-p "a damaged record" failed))
    (should (eq (get-text-property (1- (length failed)) 'face failed) 'romacs-progress-view-failed))))

(ert-deftest romacs-progress-view-line-fits-its-width ()
  "A long name and a long error are cut to the card's width; the bar and
counts stay whole."
  (let* ((name (make-string 200 ?x))
         (running (romacs-progress-view--line 'restore (list :status 'running :done 1 :of 9 :name name) 50))
         (failed (romacs-progress-view--line 'restore (list :status 'failed :error name) 50)))
    (should (<= (string-width running) 50))
    (should (string-suffix-p "1/9" running))
    (should (<= 4 (+ (cl-count ?━ running) (cl-count ?─ running)) romacs-progress-view--bar))
    (should (<= (string-width failed) 50))
    ;; Narrow: the bar gives way first, the counts stay.
    (dolist (width '(24 30 40))
      (let ((line (romacs-progress-view--line 'restore (list :status 'running :done 1 :of 9 :name name) width)))
        (should (<= (string-width line) width))
        (should (string-suffix-p "1/9" line))))
    ;; A failure's long words and long error share the line.
    (dolist (width '(24 50 80))
      (let ((line (romacs-progress-view--line 'restore (list :status 'failed :name name :error name) width)))
        (should (<= (string-width line) width))
        (should (string-match-p "…  x" line))))))

(ert-deftest romacs-progress-view-follows-the-progress-state ()
  "A running step the launch screen shows moves to the card once the
first frame is up; past the screen a running step shows the card, a
failure is said once, and steps all done show no card that was not up."
  (let ((cards 0) (messages nil) (romacs-progress-view--handoff nil) (romacs-progress-view--told nil)
        (romacs-progress-state '((agents :label "Agent sessions 1/2" :status running :done 1 :of 2)
                                 (init :label "Loading your init: org"))))
    (cl-letf (((symbol-function 'romacs-progress-view--card) (lambda () (cl-incf cards)))
              ((symbol-function 'get-buffer-window) #'ignore)
              ((symbol-function 'display-graphic-p) (lambda (&rest _) t))
              ((symbol-function 'message) (lambda (&rest args) (push (apply #'format args) messages)))
              (noninteractive nil))
      (romacs-progress-view-update t)
      (should (= cards 0))
      (should romacs-progress-view--handoff)
      (romacs-progress-view--handoff)
      (should (= cards 1))
      (romacs-progress-view-update)
      (should (= cards 2))
      (setq romacs-progress-state '((restore :label "Session restore failed" :status failed :error "a damaged record")))
      (romacs-progress-view-update)
      (romacs-progress-view-update)
      (should (= (length messages) 1))
      (should (string-match-p "a damaged record" (car messages)))
      (setq romacs-progress-state '((restore :label "Session restored" :status done :done 3 :of 3)) cards 0)
      (romacs-progress-view-update)
      (should (= cards 0)))))

(ert-deftest romacs-progress-view-shows-only-steps-with-a-state ()
  (let ((romacs-progress-state '((agents :label "Agent sessions 1/2" :status running :done 1 :of 2)
                                 (init :label "Loading your init: org"))))
    (should (equal (mapcar #'car (romacs-progress-view--steps)) '(agents)))))

(provide 'romacs-progress-view-tests)
;;; romacs-progress-view-tests.el ends here
