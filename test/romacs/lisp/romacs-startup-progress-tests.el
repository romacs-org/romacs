;;; romacs-startup-progress-tests.el --- What a start's stages keep -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)

(defvar romacs-progress-state)

(ert-deftest romacs-startup-progress-keeps-each-stages-latest-state ()
  ;; Each stage keeps one entry, its latest label and state, by its symbol
  ;; whether it was named by a symbol or by any string of the same name;
  ;; with no window the display has nothing to show, and the state is kept.
  (let ((romacs-progress-state nil))
    (romacs-startup-progress 'restore "Restoring buffers 1/2" (list :status 'running :done 1 :of 2))
    (romacs-startup-progress 'agents "Agent sessions 3/3" (list :status 'done))
    (romacs-startup-progress (copy-sequence "restore") "Restoring buffers 2/2" (list :status 'running :done 2 :of 2))
    (romacs-startup-progress (concat "rest" "ore") "Session restored" (list :status 'done :done 2 :of 2))
    (should (= (length romacs-progress-state) 2))
    (should (equal (alist-get 'restore romacs-progress-state)
                   '(:label "Session restored" :status done :done 2 :of 2)))
    (should (eq (plist-get (alist-get 'agents romacs-progress-state) :status) 'done))
    ;; No state: the label alone.
    (romacs-startup-progress 'init "Loading your init: init")
    (should (equal (alist-get 'init romacs-progress-state) '(:label "Loading your init: init")))))

;;; romacs-startup-progress-tests.el ends here
