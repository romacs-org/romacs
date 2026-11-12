;;; clipboard-test.el --- Shared kill ring and interprogram clipboard -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(defvar interprogram-paste-function nil)
(defvar interprogram-cut-function nil)

(romacs-deftest clipboard-foreign-text-enters-empty-kill-ring ()
  (let ((kill-ring nil) (kill-ring-yank-pointer nil)
        (interprogram-paste-function (lambda () "Brave text λ\nsecond line"))
        (interprogram-cut-function (lambda (text) (error "Import must not re-export %s" text))))
    (romacs-should (equal (current-kill 0) "Brave text λ\nsecond line"))
    (romacs-should (equal kill-ring '("Brave text λ\nsecond line")))))

(romacs-deftest clipboard-internal-copy-is-exported ()
  (let* ((kill-ring nil) (kill-ring-yank-pointer nil) (exported nil)
         (interprogram-paste-function nil)
         (interprogram-cut-function (lambda (text) (setq exported text))))
    (kill-new "Copied in Romacs λ")
    (romacs-should (equal exported "Copied in Romacs λ"))
    (romacs-should (equal (car kill-ring) exported))))

(romacs-deftest clipboard-yank-pop-rotates-without-reading-desktop ()
  (let ((kill-ring '("new" "old")) (reads 0)
        (kill-ring-yank-pointer nil)
        (interprogram-paste-function (lambda () (setq reads (1+ reads)) "foreign"))
        (interprogram-cut-function nil))
    (setq kill-ring-yank-pointer kill-ring)
    (romacs-should (equal (current-kill 1) "old"))
    (romacs-should (= reads 0))))

(romacs-deftest clipboard-empty-ring-has-a-useful-error ()
  (let ((kill-ring nil) (kill-ring-yank-pointer nil)
        (interprogram-paste-function nil))
    (romacs-should
     (equal (condition-case condition (current-kill 0)
              (error (error-message-string condition)))
            "Kill ring is empty"))))

(romacs-deftest clipboard-failure-preserves-internal-copy-and-yank ()
  (with-temp-buffer
    (let ((read-function (symbol-function 'romacs--clipboard-text))
          (write-function (symbol-function 'romacs--clipboard-set))
          (kill-ring nil) (kill-ring-yank-pointer nil)
          (romacs-clipboard-last-text "previous clipboard")
          (interprogram-cut-function 'romacs-clipboard-select-text)
          (interprogram-paste-function 'romacs-clipboard-selection-value))
      (unwind-protect
          (progn
            (fset 'romacs--clipboard-text (lambda () (error "Fixture clipboard timeout")))
            (fset 'romacs--clipboard-set (lambda (_) (error "Fixture clipboard unavailable")))
            (kill-new "Internal copy λ")
            (romacs-should (equal (car kill-ring) "Internal copy λ"))
            (romacs-should (equal romacs-clipboard-last-text "previous clipboard"))
            (call-interactively 'yank)
            (romacs-should (equal (buffer-string) "Internal copy λ"))
            (romacs-should (equal romacs-clipboard-last-text "previous clipboard")))
        (fset 'romacs--clipboard-text read-function)
        (fset 'romacs--clipboard-set write-function)))))

(provide 'clipboard-test)
