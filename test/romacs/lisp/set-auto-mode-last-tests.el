;;; set-auto-mode-last-tests.el --- A remapped mode is kept across files -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; `set-auto-mode' with KEEP-MODE-IF-SAME keeps a buffer's mode when the
;; next file asks for the same one, as xref does for each file it
;; searches.  Under `major-mode-remap-alist' it depends on
;; `set-auto-mode--last', which a minor `mode' from local variables used
;; to overwrite (LISP-SET-AUTO-MODE-LAST).  Each test counts the remapped
;; mode's body runs; pinned GNU's counts are given beside each.

(require 'ert)

(defvar set-auto-mode-last-tests--runs 0
  "How many times `set-auto-mode-last-tests-real-mode' ran.")

(define-derived-mode set-auto-mode-last-tests-asked-mode prog-mode "Asked"
  "The mode files ask for, remapped to `set-auto-mode-last-tests-real-mode'.")

(define-derived-mode set-auto-mode-last-tests-real-mode prog-mode "Real"
  "The mode the remap gives."
  (setq set-auto-mode-last-tests--runs (1+ set-auto-mode-last-tests--runs)))

(defvar set-auto-mode-last-tests--minor-runs 0
  "How many times `set-auto-mode-last-tests-minor-mode' was turned on.")

(define-minor-mode set-auto-mode-last-tests-minor-mode
  "A minor mode local variables turn on."
  :lighter nil
  (when set-auto-mode-last-tests-minor-mode
    (setq set-auto-mode-last-tests--minor-runs (1+ set-auto-mode-last-tests--minor-runs))))

(define-derived-mode set-auto-mode-last-tests-twice-mode prog-mode "Twice"
  "A mode whose setup applies the file's local variables itself, as CC
Mode does for a style (`c-file-style'), before `run-mode-hooks' applies
them again."
  (hack-local-variables))

(defun set-auto-mode-last-tests--files (texts &optional dir-locals)
  "Write each of TEXTS to its own .samlt file in a new directory, with
DIR-LOCALS as its .dir-locals.el when given; return the files."
  (let ((dir (file-name-as-directory (make-temp-file "samlt" t))))
    (when dir-locals
      (with-temp-file (expand-file-name ".dir-locals.el" dir) (prin1 dir-locals (current-buffer))))
    (let ((i 0))
      (mapcar (lambda (text)
                (let ((file (expand-file-name (format "f%d.samlt" (setq i (1+ i))) dir)))
                  (with-temp-file file (insert text))
                  file))
              texts))))

(defmacro set-auto-mode-last-tests--with-remap (&rest body)
  "Run BODY with .samlt files asking for the asked mode, remapped."
  (declare (indent 0))
  `(let ((auto-mode-alist (cons '("\\.samlt\\'" . set-auto-mode-last-tests-asked-mode) auto-mode-alist))
         (major-mode-remap-alist (cons '(set-auto-mode-last-tests-asked-mode . set-auto-mode-last-tests-real-mode)
                                       major-mode-remap-alist))
         (enable-local-variables :all)
         (set-auto-mode-last-tests--runs 0)
         (set-auto-mode-last-tests--minor-runs 0))
     ,@body))

(defun set-auto-mode-last-tests--one-buffer (files)
  "Set each of FILES's mode in one buffer as xref does: its text, then
`set-auto-mode' keeping a mode that is the same.  Return the mode's runs,
the minor mode's runs, and whether the minor mode is on at the end."
  (with-temp-buffer
    (dolist (file files)
      (insert-file-contents file nil nil nil t)
      (let ((buffer-file-name file))
        (set-auto-mode t)))
    (list set-auto-mode-last-tests--runs set-auto-mode-last-tests--minor-runs
          (and set-auto-mode-last-tests-minor-mode t))))

(ert-deftest set-auto-mode-last-tests-a-dir-local-minor-mode-keeps-the-remap ()
  "A .dir-locals.el `mode' that is a minor mode turns it on and leaves the
remapped mode kept for the next file: one run of each for three files
(pinned GNU: 3 of each)."
  (set-auto-mode-last-tests--with-remap
    (let ((files (set-auto-mode-last-tests--files
                  '("a\n" "b\n" "c\n")
                  '((set-auto-mode-last-tests-real-mode . ((mode . set-auto-mode-last-tests-minor)))))))
      (should (equal (set-auto-mode-last-tests--one-buffer files) '(1 1 t))))))

(ert-deftest set-auto-mode-last-tests-a-minor-named-file-local-mode-is-not-applied ()
  "A Local Variables `mode:' whose name ends in -minor is not applied, as
in GNU, and the remapped mode is kept: one run for three files, the
minor mode never on (pinned GNU: the same)."
  (set-auto-mode-last-tests--with-remap
    (let* ((text "x\n;; Local Variables:\n;; mode: set-auto-mode-last-tests-minor\n;; End:\n")
           (files (set-auto-mode-last-tests--files (list text text text))))
      (should (equal (set-auto-mode-last-tests--one-buffer files) '(1 0 nil))))))

(ert-deftest set-auto-mode-last-tests-a-mode-cookie-keeps-the-remap ()
  "A `-*- mode: -*-' cookie naming the asked mode is kept as GNU keeps
it: one run for three files (pinned GNU: 1)."
  (set-auto-mode-last-tests--with-remap
    (let* ((text "-*- mode: set-auto-mode-last-tests-asked -*-\nx\n")
           (files (set-auto-mode-last-tests--files (list text text text))))
      (should (equal (set-auto-mode-last-tests--one-buffer files) '(1 0 nil))))))

(ert-deftest set-auto-mode-last-tests-locals-applied-twice-run-the-minor-mode-once ()
  "A mode whose setup applies the local variables before `run-mode-hooks'
applies them again turns a local minor `mode' on once, as in GNU (pinned
GNU: 1)."
  (set-auto-mode-last-tests--with-remap
    (let* ((auto-mode-alist (cons '("\\.samlt\\'" . set-auto-mode-last-tests-twice-mode) auto-mode-alist))
           (files (set-auto-mode-last-tests--files
                   '("a\n")
                   '((set-auto-mode-last-tests-twice-mode . ((mode . set-auto-mode-last-tests-minor)))))))
      (should (equal (set-auto-mode-last-tests--one-buffer files) '(0 1 t))))))

(ert-deftest set-auto-mode-last-tests-revert-runs-the-mode-again ()
  "Reverting a visited file runs its mode again, as GNU does (pinned GNU:
2 runs for a visit and a revert)."
  (set-auto-mode-last-tests--with-remap
    (let* ((file (car (set-auto-mode-last-tests--files
                       '("a\n") '((set-auto-mode-last-tests-real-mode . ((mode . set-auto-mode-last-tests-minor)))))))
           (buffer (find-file-noselect file)))
      (unwind-protect
          (with-current-buffer buffer
            (revert-buffer t t)
            (should (eq major-mode 'set-auto-mode-last-tests-real-mode))
            (should (= set-auto-mode-last-tests--runs 2)))
        (kill-buffer buffer)))))

;;; set-auto-mode-last-tests.el ends here
