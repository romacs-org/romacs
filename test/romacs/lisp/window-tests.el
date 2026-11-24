;;; window-tests.el --- Window resizing tests -*- lexical-binding: t -*-

(require 'ert)

(ert-deftest romacs-delete-other-windows-preserves-subtree-height ()
  (save-window-excursion
    (delete-other-windows)
    (let* ((window-combination-limit nil)
           (window-combination-resize nil)
           (height (window-pixel-height (frame-root-window)))
           (chat (selected-window))
           (discarded (split-window chat nil 'above))
           (composer (let ((window-combination-limit t))
                       (split-window chat -4 'below)))
           (pair (window-parent chat)))
      (window-make-atom pair)
      (window-preserve-size composer nil t)
      (delete-other-windows chat)
      (should-not (window-live-p discarded))
      (should (eq (frame-root-window) pair))
      (should (= (window-pixel-height pair) height))
      (should (= (window-total-height composer) 4)))))

;;; window-tests.el ends here
