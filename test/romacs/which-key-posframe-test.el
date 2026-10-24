;;; which-key-posframe-test.el --- Test which-key-posframe with child frames -*- lexical-binding: t -*-

;; Test which-key-posframe rendering on romacs GPU child frame backend.
;; Usage: ./target/release/romacs -Q -l test/romacs/which-key-posframe-test.el
;;
;; Installs which-key, posframe, and which-key-posframe from MELPA,
;; then exercises the keybinding popup UI with various prefix keys.

;;; Code:

;; --- Package bootstrap ---
(require 'package)
(setq package-archives
      '(("melpa" . "https://melpa.org/packages/")
        ("gnu" . "https://elpa.gnu.org/packages/")))
(package-initialize)

(unless (package-installed-p 'which-key)
  (package-refresh-contents)
  (package-install 'which-key))

(unless (package-installed-p 'posframe)
  (unless package-archive-contents (package-refresh-contents))
  (package-install 'posframe))

(unless (package-installed-p 'which-key-posframe)
  (unless package-archive-contents (package-refresh-contents))
  (package-install 'which-key-posframe))

;; --- Enable which-key + posframe ---
(require 'which-key)
(require 'posframe)
(require 'which-key-posframe)

(which-key-mode 1)
(which-key-posframe-mode 1)

;; Make which-key show up quickly for testing
(setq which-key-idle-delay 0.3)
(setq which-key-idle-secondary-delay 0.05)

;; Posframe appearance
(setq which-key-posframe-parameters
      '((left-fringe . 8)
        (right-fringe . 8)
        (child-frame-border-width . 2)
        (internal-border-width . 8)))

(setq which-key-posframe-poshandler #'posframe-poshandler-frame-center)

;; --- Setup buffer ---
(with-current-buffer (get-buffer-create "*which-key-test*")
  (erase-buffer)
  (insert "Which-Key Posframe Test\n")
  (insert "=======================\n\n")
  (insert "Press prefix keys and wait for the which-key popup:\n\n")
  (insert "  C-x     - File/buffer commands\n")
  (insert "  C-c     - Mode-specific commands\n")
  (insert "  C-h     - Help commands\n")
  (insert "  M-g     - Goto commands\n")
  (insert "  M-s     - Search commands\n")
  (insert "  C-x r   - Register/rectangle/bookmark commands\n")
  (insert "  C-x 4   - Other-window commands\n")
  (insert "  C-x 5   - Frame commands\n")
  (insert "  C-x n   - Narrowing commands\n")
  (insert "  C-x a   - Abbrev commands\n\n")
  (insert "The popup should appear as a centered child frame\n")
  (insert "with rounded corners and drop shadow.\n")
  (goto-char (point-min)))

(switch-to-buffer "*which-key-test*")

(message "=== Which-Key Posframe Test ===")
(message "Press C-x, C-h, C-c, M-g, M-s and wait ~0.3s for the posframe popup.")

;;; which-key-posframe-test.el ends here
