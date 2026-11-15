;;; romacs-progress-view-tests.el --- Tests for romacs-progress-view.el  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Code:

(require 'ert)
(require 'romacs-progress-view)

(ert-deftest romacs-progress-view-line-shows-a-step-as-it-goes ()
  "A running step: its words, its bar so far and its counts; a done one is
checked and full; a failed one says why, in the failure's face."
  (let ((running (romacs-progress-view--line 'restore '(:status running :done 2 :of 4 :name "init.el")))
        (done (romacs-progress-view--line 'agents '(:status done :done 3 :of 3)))
        (failed (romacs-progress-view--line 'restore '(:status failed :error "a damaged record"))))
    (should (string-match-p "Restoring buffers: init.el" running))
    (should (= (cl-count ?━ running) (/ romacs-progress-view--bar 2)))
    (should (string-suffix-p "2/4" running))
    (should (string-prefix-p "✓ Loading agent sessions" done))
    (should (= (cl-count ?━ done) romacs-progress-view--bar))
    (should (string-prefix-p "✗ " failed))
    (should (string-match-p "a damaged record" failed))
    (should (eq (get-text-property (1- (length failed)) 'face failed) 'romacs-progress-view-failed))))

(ert-deftest romacs-progress-view-takes-the-launch-screen-first ()
  "While the launch screen shows a step, no card is drawn; after it, the card."
  (let ((cards 0) (screen t))
    (cl-letf (((symbol-function 'romacs-startup-progress) (lambda (&rest _) screen))
              ((symbol-function 'romacs-progress-view--card) (lambda () (cl-incf cards)))
              ((symbol-function 'display-graphic-p) (lambda (&rest _) t))
              (noninteractive nil))
      (romacs-progress-view-update 'agents '(:status running :done 1 :of 2))
      (should (= cards 0))
      (setq screen nil)
      (romacs-progress-view-update 'agents '(:status running :done 2 :of 2))
      (should (= cards 1)))))

(provide 'romacs-progress-view-tests)
;;; romacs-progress-view-tests.el ends here
