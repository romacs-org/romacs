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

(ert-deftest romacs-progress-view-takes-the-launch-screen-first ()
  "While the launch screen shows a step, no card is drawn; after it, the card."
  (let ((cards 0) (screen t))
    (cl-letf (((symbol-function 'romacs-startup-progress) (lambda (&rest _) screen))
              ((symbol-function 'romacs-progress-view--card) (lambda () (cl-incf cards)))
              ((symbol-function 'display-graphic-p) (lambda (&rest _) t))
              (noninteractive nil))
      (let ((romacs-progress-state '((agents :status running :done 1 :of 2)))
            (romacs-progress-view--handoff nil))
        (romacs-progress-view-update 'agents '(:status running :done 1 :of 2))
        (should (= cards 0))
        ;; The screen gave way: the step still going moves to the card.
        (should romacs-progress-view--handoff)
        (romacs-progress-view--handoff)
        (should (= cards 1)))
      (setq cards 0 screen nil)
      (romacs-progress-view-update 'agents '(:status running :done 2 :of 2))
      (should (= cards 1)))))

(provide 'romacs-progress-view-tests)
;;; romacs-progress-view-tests.el ends here
