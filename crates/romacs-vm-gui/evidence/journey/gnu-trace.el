;;; gnu-trace.el --- which VM2-missing C primitives GNU calls per journey step -*- lexical-binding: t -*-
;; Run INTERACTIVELY (emacs -Q -nw in a pty, see journey-check.sh): the
;; minibuffer, redisplay and jit-lock run as they do for the owner.
;;   emacs -Q -nw -l gnu-trace.el --eval '(journey-trace "FIXTURE" "MISSING" "OUT")'
;;
;; Method and its known blind spot: every C primitive VM2 lacks (MISSING, one
;; name per line) gets a :before advice that records the step being run.
;; Advice only sees calls that go through the symbol's function cell: calls
;; from interpreted or byte-compiled Lisp, and from C via Ffuncall. Calls
;; compiled into native code bypass it, so the journey's own files are
;; reloaded as byte code first (load-no-native) and org is loaded as byte
;; code. What stays native (subr.el and the rest of the dump) is NOT seen:
;; the result is a lower bound per step, never an over-count.
(require 'cl-lib)
(defvar journey--step nil)
(defvar journey--in nil)
(defvar journey--hits (make-hash-table :test #'equal))
(defvar journey--order nil)

(defun journey--note (name)
  (unless journey--in
    (let ((journey--in t))
      (when journey--step
        (let* ((key (cons journey--step name)) (n (gethash key journey--hits)))
          (unless n (push key journey--order))
          (puthash key (1+ (or n 0)) journey--hits))))))

(defconst journey--never
  '(funcall apply eval signal throw error kill-emacs advice-add advice-remove
    gethash puthash symbol-function fset car cdr cons eq not null))

(defun journey--instrument (names)
  (setq native-comp-enable-subr-trampolines nil)
  (let ((n 0))
    (dolist (name names)
      (let ((s (intern-soft name)))
        (when (and s (fboundp s) (not (memq s journey--never))
                   (subr-primitive-p (symbol-function s))
                   (not (special-form-p s)))
          (let ((nm name))
            (advice-add s :before (lambda (&rest _) (journey--note nm)) '((name . journey))))
          (setq n (1+ n)))))
    n))

(defun journey--keys (&rest parts)
  (apply #'vconcat (mapcar (lambda (p) (if (vectorp p) p (kbd p))) parts)))

(defun journey--run (step keys)
  (setq journey--step step)
  (condition-case e (execute-kbd-macro keys)
    (error (push (list step 'error (error-message-string e)) journey--errors)))
  (redisplay t)
  (setq journey--step nil))
(defvar journey--errors nil)

(defun journey-trace (fixture missing out)
  (let* ((names (with-temp-buffer (insert-file-contents missing) (split-string (buffer-string) "\n" t)))
         (copy (make-temp-file "journey-" nil ".org"))
         (path (vconcat copy)))
    (copy-file fixture copy t)
    ;; Byte code, not native code, for the files the journey runs through.
    (let ((load-no-native t))
      (dolist (f '("simple" "files" "window" "minibuffer" "bindings" "font-lock" "jit-lock"
                   "syntax" "progmodes/elisp-mode" "emacs-lisp/lisp-mode" "indent" "newcomment"
                   "isearch" "replace" "buff-menu" "emacs-lisp/tabulated-list" "outline"))
        (load f nil t))
      (require 'org))
    (let ((count (journey--instrument names)))
      (run-at-time
       0.5 nil
       (lambda ()
         (let ((load-no-native t))
           (journey--run "visit" (journey--keys "C-x C-f" path "RET"))
           (journey--run "look" [])
           (journey--run "move-line" (journey--keys "C-n C-n C-n C-p"))
           (journey--run "move-char" (journey--keys "C-f C-f C-b"))
           (journey--run "move-bol-eol" (journey--keys "C-e C-a"))
           (journey--run "move-buffer" (journey--keys "M-> M-<"))
           (journey--run "scroll-page" (journey--keys "C-v M-v"))
           (journey--run "recenter" (journey--keys "C-l"))
           (journey--run "point-visible" (apply #'journey--keys (make-list 130 "C-n")))
           (journey--run "move-buffer" (journey--keys "M-<"))
           (journey--run "move-line" (journey--keys "C-n"))
           (journey--run "move-bol-eol" (journey--keys "C-e"))
           (journey--run "self-insert" (journey--keys "SPC e d i t e d"))
           (journey--run "delete-backward" (journey--keys "DEL DEL"))
           (journey--run "kill-line" (journey--keys "C-k"))
           (journey--run "undo" (journey--keys "C-/"))
           (journey--run "save" (journey--keys "C-x C-s"))
           (journey--run "split-window" (journey--keys "C-x 2 C-x 3"))
           (journey--run "other-window" (journey--keys "C-x o"))
           (journey--run "delete-other-windows" (journey--keys "C-x 1"))
           (journey--run "switch-buffer" (journey--keys "C-x b *scratch* RET"))
           (journey--run "move-buffer" (journey--keys "M->"))
           (journey--run "self-insert" (journey--keys "( + SPC 1 SPC 2 )"))
           (journey--run "eval-last-sexp" (journey--keys "C-x C-e"))
           (journey--run "eval-expression" (journey--keys "M-: ( + SPC 1 SPC 2 ) RET"))
           (journey--run "switch-buffer" (journey--keys "C-x b RET"))
           (journey--run "list-buffers" (journey--keys "C-x C-b"))
           (journey--run "M-x-completion" (journey--keys "M-x forward-ch TAB RET"))
           (journey--run "move-buffer" (journey--keys "M-<"))
           (journey--run "move-line" (journey--keys "C-n"))
           (journey--run "org-cycle" (journey--keys "TAB"))
           (journey--run "org-edit" (journey--keys "M-RET"))
           (journey--run "self-insert" (journey--keys "N e w"))
           (let ((rows (mapcar (lambda (k) (list (car k) (cdr k) (gethash k journey--hits)))
                               (reverse journey--order))))
             (with-temp-file out
               (insert (json-encode
                        `((instrumented . ,count) (missing_given . ,(length names))
                          (saved_md5 . ,(with-temp-buffer
                                          (insert-file-contents-literally copy)
                                          (md5 (current-buffer))))
                          (errors . ,(mapcar (lambda (e) (format "%S" e)) journey--errors))
                          (hits . ,(vconcat (mapcar #'vconcat rows))))))))
           (kill-emacs 0)))))))
