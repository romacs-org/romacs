;;; romacs-oracle-parity.el --- Visual parity oracle scenarios -*- lexical-binding: t -*-

;; Usage:
;;   ROMACS_ORACLE_SCENARIO=hello ROMACS_ORACLE_LABEL=romacs \
;;     ./target/release/romacs -Q -l test/romacs/romacs-oracle-parity.el

;;; Code:

(defconst romacs-oracle-scenario
  (or (getenv "ROMACS_ORACLE_SCENARIO") "hello"))

(defconst romacs-oracle-label
  (or (getenv "ROMACS_ORACLE_LABEL") "romacs"))

(defun romacs-oracle--set-font ()
  "Set a deterministic default font when available."
  (let ((candidates '("Noto Sans Mono-16"
                      "DejaVu Sans Mono-16"
                      "Monospace-16")))
    (catch 'done
      (dolist (font candidates)
        (condition-case nil
            (progn
              (set-frame-font font t t)
              (throw 'done t))
          (error nil))))))

(defun romacs-oracle--setup-common ()
  "Normalize frame state for screenshot comparison."
  (menu-bar-mode -1)
  (tool-bar-mode -1)
  (scroll-bar-mode -1)
  (blink-cursor-mode -1)
  (setq cursor-type 'box)
  (set-frame-size (selected-frame) 132 42)
  (romacs-oracle--set-font)
  (setq frame-title-format
        (format "romacs-oracle-%s-%s"
                romacs-oracle-scenario
                romacs-oracle-label)))

(defun romacs-oracle--scenario-hello ()
  "Open view-hello-file for LANGUAGE/HELLO column alignment checks."
  (view-hello-file)
  (goto-char (point-min))
  (recenter 0))

(defun romacs-oracle--scenario-cjk (index)
  "Create CJK cursor geometry scenario with cursor INDEX (1 or 2)."
  (let ((buf (get-buffer-create "*Romacs Oracle CJK*")))
    (switch-to-buffer buf)
    (setq buffer-read-only nil)
    (erase-buffer)
    (insert "a好好b\n")
    (insert "a好好b\n")
    (insert "\nCursor oracle: compare first/second 好 begin/end.\n")
    (goto-char (point-min))
    (search-forward "a")
    (forward-char (if (> index 1) 1 0))
    (setq buffer-read-only t)))

(defun romacs-oracle--scenario-weights ()
  "Create Noto/Hack bold-vs-extra-bold scenario."
  (let ((buf (get-buffer-create "*Romacs Oracle Weights*")))
    (switch-to-buffer buf)
    (setq buffer-read-only nil)
    (erase-buffer)
    (insert "Noto Sans Mono + Hack weight parity\n\n")
    (insert "Noto h=1.6 w=bold:       ")
    (let ((s (point)))
      (insert "a好好b  ABCXYZ 0123456789  -> <= >=")
      (put-text-property
       s (point) 'face '(:family "Noto Sans Mono" :height 1.6 :weight bold)))
    (insert "\n")
    (insert "Noto h=1.6 w=extra-bold: ")
    (let ((s (point)))
      (insert "a好好b  ABCXYZ 0123456789  -> <= >=")
      (put-text-property
       s (point) 'face '(:family "Noto Sans Mono" :height 1.6 :weight extra-bold)))
    (insert "\n\n")
    (insert "Hack h=1.6 w=bold:       ")
    (let ((s (point)))
      (insert "a好好b  ABCXYZ 0123456789  -> <= >=")
      (put-text-property
       s (point) 'face '(:family "Hack" :height 1.6 :weight bold)))
    (insert "\n")
    (insert "Hack h=1.6 w=extra-bold: ")
    (let ((s (point)))
      (insert "a好好b  ABCXYZ 0123456789  -> <= >=")
      (put-text-property
       s (point) 'face '(:family "Hack" :height 1.6 :weight extra-bold)))
    (insert "\n")
    (goto-char (point-min))
    (setq buffer-read-only t)))

(defun romacs-oracle--run ()
  "Dispatch parity scenario from environment."
  (romacs-oracle--setup-common)
  (pcase romacs-oracle-scenario
    ("hello" (romacs-oracle--scenario-hello))
    ("weights" (romacs-oracle--scenario-weights))
    ("cjk1" (romacs-oracle--scenario-cjk 1))
    ("cjk2" (romacs-oracle--scenario-cjk 2))
    (_ (romacs-oracle--scenario-weights)))
  (redisplay t)
  ;; Safety timeout for automation harness.
  (run-at-time 180 nil (lambda () (kill-emacs 0))))

(add-hook 'emacs-startup-hook #'romacs-oracle--run)

;;; romacs-oracle-parity.el ends here
