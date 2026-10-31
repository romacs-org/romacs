;;; romacs_complete_compare.el --- One completion journey for vertico and romacs-complete  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Loaded by pinned GNU with vertico and orderless (fixtures) and by
;; Romacs with `romacs-complete-mode' (romacs_complete_vs_gnu.py).  Each
;; scenario is a reader and the keys typed into it, run as a keyboard
;; macro, so the minibuffer's own commands and hooks run as they do for
;; typed keys.  After every command in the minibuffer the UI's state is
;; recorded (input, selected index, total, scroll, the rows shown), then
;; the reader's value; the two engines' records must be equal.
;; CMP_DIR is the fixture directory, CMP_OUT the record file.

;;; Code:

(defvar cmp-ui (if (featurep 'vertico) "vertico" "romacs-complete"))
(defvar cmp-dir (file-name-as-directory (getenv "CMP_DIR")))
(defvar cmp-out (getenv "CMP_OUT"))
(defvar cmp-records nil)
(defvar cmp-scenario nil)
(defvar cmp-history nil)

(defvar cmp-words
  '("alpha" "beta" "gamma" "delta" "alphabet" "alpine" "gamma-ray" "Alpaca"
    "epsilon" "zeta" "eta" "theta" "iota" "kappa" "lambda" "mu" "nu" "xi"
    "omicron" "pi" "rho" "sigma" "tau" "upsilon" "phi" "chi" "psi" "omega"
    "al-pha" "beta-max" "beta-blocker" "tango" "delta-v" "AL" "alp" "x-ray"))

(defun cmp-grouped (string pred action)
  "A table of fruit and tools with a `group-function'."
  (if (eq action 'metadata)
      `(metadata (group-function . ,(lambda (cand transform)
                                      (if transform cand
                                        (if (member cand '("axe" "saw" "awl" "rasp")) "Tools" "Fruit")))))
    (complete-with-action action '("apple" "axe" "pear" "saw" "plum" "awl" "grape" "rasp" "papaya")
                          string pred)))

(defconst cmp-scenarios
  `((pick-al "al C-n C-n" (completing-read "Pick: " cmp-words))
    (pick-default "" (completing-read "Pick (default beta): " cmp-words nil t nil 'cmp-history "beta"))
    (pick-history "a" (completing-read "Pick: " cmp-words nil t nil 'cmp-history))
    (pick-orderless "a SPC p C-n M->" (completing-read "Pick: " cmp-words))
    (pick-wrap "C-p C-p C-n" (completing-read "Pick: " cmp-words nil nil nil nil "zeta"))
    (pick-scroll "C-v C-v M-v C-n" (completing-read "Pick: " cmp-words))
    (pick-tab "ga TAB" (completing-read "Pick: " cmp-words))
    (pick-group "a C-n C-n C-n" (completing-read "Grouped: " #'cmp-grouped))
    (file-pcm "s/n TAB" (read-file-name "File: " cmp-dir))
    (file-step "su TAB no" (read-file-name "File: " cmp-dir))
    (file-many "file-1 C-n C-n" (read-file-name "File: " cmp-dir))
    (file-del "sub/ DEL DEL" (read-file-name "File: " cmp-dir))
    (file-ignored "file. C-n" (read-file-name "File: " cmp-dir))
    (input-exit "zz M-RET" (completing-read "Pick: " cmp-words)))
  "(NAME KEYS FORM): FORM reads, KEYS are typed, then RET unless KEYS
already exit.")

(defun cmp-state (key)
  "The UI's state after the command KEY ran."
  (let ((var (lambda (name) (symbol-value (intern (format "%s--%s" cmp-ui name))))))
    (list cmp-scenario key (minibuffer-contents-no-properties)
          (funcall var "index") (funcall var "total") (funcall var "scroll")
          (mapcar #'substring-no-properties
                  (seq-take (nthcdr (funcall var "scroll") (funcall var "candidates")) 12)))))

(defun cmp-record ()
  (push (cmp-state (key-description (this-command-keys))) cmp-records))

(defun cmp-setup ()
  (add-hook 'post-command-hook #'cmp-record 90 t))

(defun cmp-read ()
  (interactive)
  (let ((form (nth 2 (assq cmp-scenario cmp-scenarios))))
    (push (list cmp-scenario 'value (condition-case e (let ((value (eval form t)))
                                                         (if (stringp value) (substring-no-properties value) value))
                                       (error (list 'error e))))
          cmp-records)))

(defun cmp-run ()
  "Every scenario, then the records to CMP_OUT, then exit."
  (setq completion-styles '(orderless basic)
        completion-category-overrides '((file (styles orderless partial-completion)))
        completion-ignore-case t read-file-name-completion-ignore-case t
        history-length 100 cmp-history '("gamma" "alpine" "gamma"))
  (keymap-global-set "<f9>" #'cmp-read)
  (add-hook 'minibuffer-setup-hook #'cmp-setup)
  (dolist (scenario cmp-scenarios)
    (setq cmp-scenario (car scenario))
    (let ((keys (nth 1 scenario)))
      (condition-case e
          (execute-kbd-macro (kbd (concat "<f9> " keys (if (string-suffix-p "M-RET" keys) "" " RET"))))
        (error (push (list cmp-scenario 'macro-error e) cmp-records)))))
  (with-temp-file cmp-out
    (dolist (record (nreverse cmp-records))
      (insert (prin1-to-string record) "\n")))
  (kill-emacs 0))

(run-at-time 2 nil #'cmp-run)

;;; romacs_complete_compare.el ends here
