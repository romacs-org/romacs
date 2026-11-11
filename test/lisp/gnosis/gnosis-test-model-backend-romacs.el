;;; gnosis-test-model-backend-romacs.el --- The native backend's highlight layers -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; The native backend paints cues in the renderer's highlight layers, a
;; cue set whole or not at all (`gnosis-model-show-cues'), and keeps a
;; layer for the pick.  Batch has no renderer: the model primitives are
;; replaced by a stand-in that records what the backend asks.

;;; Code:
(require 'ert)
(require 'gnosis-model)
(require 'gnosis-model-backend-romacs)

(defun gnosis-test-backend-romacs--cues (n)
  "N distinct object cue targets."
  (cl-loop for i below n collect (list (cons 'id (format "cue-%d" i)) (cons 'kind 'object) (cons 'mesh i))))

(defmacro gnosis-test-backend-romacs--with (&rest body)
  "BODY with a native backend whose model is a stand-in; `asked' holds the
highlight requests, newest first."
  (declare (indent 0))
  `(let ((asked nil)
         (gnosis-model-backend-romacs--cues nil)
         (gnosis-model-backend-romacs--selection nil)
         (gnosis-model-backend-romacs--selected-id nil)
         (gnosis-model-backend-romacs--pending-pick nil))
     (cl-letf (((symbol-function 'gnosis-model-backend-romacs--owned) (lambda () 'model))
               ((symbol-function 'gnosis-model-backend-romacs--layers-p) (lambda () t))
               ((symbol-function 'gnosis-model-backend-romacs--spec) (lambda (target) (list :cue target)))
               ((symbol-function 'romacs-model-set-highlight) (lambda (_model &rest plist) (push plist asked))))
       ,@body)))

(ert-deftest gnosis-backend-romacs-cues-fill-every-layer-but-the-picks ()
  (gnosis-test-backend-romacs--with
    (let ((room (1- romacs-model-max-highlight-layers)))
      ;; As many cues as leave the pick its layer: all painted.
      (should (gnosis-model-backend-romacs-cue (gnosis-test-backend-romacs--cues room)))
      (should (= (length (plist-get (car asked) :layers)) room))
      ;; One more: declined, and no cue stays painted from before.
      (should-not (gnosis-model-backend-romacs-cue (gnosis-test-backend-romacs--cues (1+ room))))
      (should-not gnosis-model-backend-romacs--cues)
      (should-not (plist-get (car asked) :layers)))))

(ert-deftest gnosis-backend-romacs-layers-keep-cues-whole ()
  (let* ((limit romacs-model-max-highlight-layers)
         (cues (make-list (1- limit) 'cue)))
    ;; Cues and a pick fit: both.
    (should (equal (gnosis-model-backend-romacs--fit-layers cues '(pick)) (append cues '(pick))))
    ;; Answers beside them would not: the answers alone, never part of the cues.
    (should (equal (gnosis-model-backend-romacs--fit-layers cues '(wrong answer)) '(wrong answer)))
    ;; Answers alone over the limit: the last of them.
    (should (= (length (gnosis-model-backend-romacs--fit-layers nil (make-list (1+ limit) 'answer))) limit))))

(provide 'gnosis-test-model-backend-romacs)
;;; gnosis-test-model-backend-romacs.el ends here
