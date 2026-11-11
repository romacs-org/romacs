;;; vertico-posframe-test.el --- Test vertico-posframe with child frames -*- lexical-binding: t -*-

;; Test vertico-posframe rendering on romacs GPU child frame backend.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/vertico-posframe-test.el
;;
;; This installs vertico, posframe, and vertico-posframe from MELPA,
;; then exercises the completion UI with various commands.

;;; Code:

;; --- Package bootstrap ---
(require 'package)
(setq package-archives
      '(("melpa" . "https://melpa.org/packages/")
        ("gnu" . "https://elpa.gnu.org/packages/")))
(package-initialize)

(unless (package-installed-p 'vertico)
  (package-refresh-contents)
  (package-install 'vertico))

(unless (package-installed-p 'posframe)
  (unless package-archive-contents (package-refresh-contents))
  (package-install 'posframe))

(unless (package-installed-p 'vertico-posframe)
  (unless package-archive-contents (package-refresh-contents))
  (package-install 'vertico-posframe))

;; --- Enable vertico + posframe ---
(require 'vertico)
(require 'posframe)
(require 'vertico-posframe)

(vertico-mode 1)
(vertico-posframe-mode 1)

;; Configure posframe appearance
(setq vertico-posframe-parameters
      '((left-fringe . 8)
        (right-fringe . 8)
        (child-frame-border-width . 2)
        (internal-border-width . 6)))

(setq vertico-posframe-border-width 2)
(setq vertico-posframe-min-width 60)
(setq vertico-posframe-min-height 10)

;; Use centered posframe
(setq vertico-posframe-poshandler #'posframe-poshandler-frame-center)

;; --- Test content setup ---
(defun vertico-posframe-test--setup ()
  "Create test buffers with content for switching."
  ;; Buffer with code
  (with-current-buffer (get-buffer-create "*test-code*")
    (erase-buffer)
    (emacs-lisp-mode)
    (insert ";; Sample Emacs Lisp code\n")
    (insert "(defun hello-world ()\n")
    (insert "  \"Say hello.\"\n")
    (insert "  (interactive)\n")
    (insert "  (message \"Hello, World!\"))\n\n")
    (insert "(defun fibonacci (n)\n")
    (insert "  \"Compute Nth Fibonacci number.\"\n")
    (insert "  (if (< n 2) n\n")
    (insert "    (+ (fibonacci (- n 1))\n")
    (insert "       (fibonacci (- n 2)))))\n\n")
    (dotimes (i 50)
      (insert (format "(defun test-func-%03d () (interactive) (message \"func %d\"))\n" i i))))

  ;; Buffer with text
  (with-current-buffer (get-buffer-create "*test-text*")
    (erase-buffer)
    (dotimes (i 80)
      (insert (format "Line %03d: The quick brown fox jumps over the lazy dog.\n" (1+ i)))))

  ;; Switch to code buffer
  (switch-to-buffer "*test-code*")
  (goto-char (point-min))
  (message "vertico-posframe test ready — packages loaded and configured"))

;; --- Run setup ---
(vertico-posframe-test--setup)

(message "=== Vertico-Posframe Test ===")
(message "Try: M-x, C-x b, C-x C-f, C-h f, C-h v to test the posframe completion UI")
(message "The completion popup should render as a centered child frame with border and shadow.")

;;; vertico-posframe-test.el ends here
