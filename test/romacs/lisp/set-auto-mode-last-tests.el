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

(define-minor-mode set-auto-mode-last-tests-minor-mode
  "A minor mode local variables turn on."
  :lighter nil)

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
         (set-auto-mode-last-tests--runs 0))
     ,@body))

(defun set-auto-mode-last-tests--one-buffer (files)
  "Set each of FILES's mode in one buffer as xref does: its text, then
`set-auto-mode' keeping a mode that is the same.  Return the mode's runs."
  (with-temp-buffer
    (dolist (file files)
      (insert-file-contents file nil nil nil t)
      (let ((buffer-file-name file))
        (set-auto-mode t)))
    set-auto-mode-last-tests--runs))

(ert-deftest set-auto-mode-last-tests-a-dir-local-minor-mode-keeps-the-remap ()
  "A .dir-locals.el `mode' that is a minor mode leaves the remapped mode
kept for the next file: one run for three files (pinned GNU: 3)."
  (set-auto-mode-last-tests--with-remap
    (let ((files (set-auto-mode-last-tests--files
                  '("a\n" "b\n" "c\n")
                  '((set-auto-mode-last-tests-real-mode . ((mode . set-auto-mode-last-tests-minor)))))))
      (should (= (set-auto-mode-last-tests--one-buffer files) 1)))))

(ert-deftest set-auto-mode-last-tests-a-file-local-minor-mode-keeps-the-remap ()
  "A Local Variables `mode:' that is a minor mode keeps the remapped mode,
as in GNU: one run for three files (pinned GNU: 1)."
  (set-auto-mode-last-tests--with-remap
    (let* ((text "x\n;; Local Variables:\n;; mode: set-auto-mode-last-tests-minor\n;; End:\n")
           (files (set-auto-mode-last-tests--files (list text text text))))
      (should (= (set-auto-mode-last-tests--one-buffer files) 1)))))

(ert-deftest set-auto-mode-last-tests-a-mode-cookie-keeps-the-remap ()
  "A `-*- mode: -*-' cookie naming the asked mode is kept as GNU keeps
it: one run for three files (pinned GNU: 1)."
  (set-auto-mode-last-tests--with-remap
    (let* ((text "-*- mode: set-auto-mode-last-tests-asked -*-\nx\n")
           (files (set-auto-mode-last-tests--files (list text text text))))
      (should (= (set-auto-mode-last-tests--one-buffer files) 1)))))

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
