;;; romacs-world-layer-tests.el --- The init layer's Lisp  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-world-layer)

(defmacro romacs-world-layer-tests--preloading (files &rest body)
  "BODY with FILES, (NAME . TEXT) each, as libraries on `load-path', and
`armed', the timers `romacs--layer-preload-step' would start, newest
first, each (KIND FEATURES): none starts."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-preload-" t))
          (load-path (cons dir load-path))
          (armed nil))
     (unwind-protect
         (progn
           (dolist (file ,files)
             (with-temp-file (expand-file-name (concat (car file) ".el") dir) (insert (cdr file))))
           (cl-letf (((symbol-function 'run-at-time)
                      (lambda (_time _repeat _function &rest args) (push (cons 'at args) armed)))
                     ((symbol-function 'run-with-idle-timer)
                      (lambda (_time _repeat _function &rest args) (push (cons 'idle args) armed))))
             ,@body))
       (delete-directory dir t))))

(ert-deftest romacs-world-layer-preload-loads-one-library-a-step ()
  "A step loads the first feature not loaded yet, and goes on with the
rest at once; one already loaded is passed over."
  (romacs-world-layer-tests--preloading
      '(("romacs-wl-tests-b" . "(provide 'romacs-wl-tests-b)\n")
        ("romacs-wl-tests-c" . "(provide 'romacs-wl-tests-c)\n"))
    (provide 'romacs-wl-tests-a)
    (cl-letf (((symbol-function 'input-pending-p) #'ignore))
      (romacs--layer-preload-step '(romacs-wl-tests-a romacs-wl-tests-b romacs-wl-tests-c)))
    (should (featurep 'romacs-wl-tests-b))
    (should-not (featurep 'romacs-wl-tests-c))
    (should (equal armed '((at (romacs-wl-tests-c)))))))

(ert-deftest romacs-world-layer-preload-loads-a-feature-with-the-file-that-provides-it ()
  "A feature no file of its name provides is passed over: the file that
provides it, later in the queue, brings it."
  (romacs-world-layer-tests--preloading
      '(("romacs-wl-tests-g" . "(provide 'romacs-wl-tests-g-plain)\n(provide 'romacs-wl-tests-g)\n"))
    (cl-letf (((symbol-function 'input-pending-p) #'ignore))
      (romacs--layer-preload-step '(romacs-wl-tests-g-plain romacs-wl-tests-g)))
    (should (featurep 'romacs-wl-tests-g-plain))
    (should (featurep 'romacs-wl-tests-g))
    (should (equal armed '((at nil))))))

(ert-deftest romacs-world-layer-preload-waits-while-a-key-waits ()
  "A waiting key loads nothing: the step waits for the next idle moment."
  (romacs-world-layer-tests--preloading
      '(("romacs-wl-tests-d" . "(provide 'romacs-wl-tests-d)\n"))
    (cl-letf (((symbol-function 'input-pending-p) (lambda (&rest _) t)))
      (romacs--layer-preload-step '(romacs-wl-tests-d)))
    (should-not (featurep 'romacs-wl-tests-d))
    (should (equal armed '((idle (romacs-wl-tests-d)))))))

(ert-deftest romacs-world-layer-preload-stops-at-an-error-as-a-warning ()
  "A library that signals stops the loading, as a warning."
  (romacs-world-layer-tests--preloading
      '(("romacs-wl-tests-e" . "(error \"romacs-fixture: this library fails\")\n"))
    (let ((warned nil))
      (cl-letf (((symbol-function 'input-pending-p) #'ignore)
                ((symbol-function 'display-warning) (lambda (type message &rest _) (push (cons type message) warned))))
        (romacs--layer-preload-step '(romacs-wl-tests-e romacs-wl-tests-f)))
      (should (equal (mapcar #'car warned) '(romacs-layer)))
      (should (string-match-p "romacs-wl-tests-e" (cdar warned)))
      (should-not armed))))

;;; romacs-world-layer-tests.el ends here
