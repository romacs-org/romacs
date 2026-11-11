;;; romacs-commands.el --- M-x over the kept command set  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Copyright (C) 1985-1987, 1993-2026 Free Software Foundation, Inc.
;; Adapted from GNU simple.el: `read-extended-command-1', kept as the GPL
;; requires.

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU's M-x completes over the whole obarray, so each key of the
;; minibuffer calls `commandp' and `read-extended-command-predicate' on
;; every symbol.  Here its table lists the commands the runtime keeps as
;; definitions change (`romacs-command-symbols', crates/romacs-vm/src/
;; command_set.rs), and keeps what its own predicate answered for them
;; until a definition, a command's plist or the buffer's modes change
;; (`romacs-command-epoch').  A predicate that may read anything else runs
;; on each key, over the commands only.  The reader is otherwise GNU's
;; (ledger/divergences/MX-COMMAND-SET.md).

;;; Code:

(defvar romacs-commands-modal-predicates
  '(command-completion-default-include-p command-completion-using-modes-p)
  "Values of `read-extended-command-predicate' whose answers M-x keeps.
Each reads only the command's definition and plist and the buffer's
modes; a command's own `completion-predicate' is asked on each key.
nil, no predicate, is kept too.")

(defun romacs-commands--obsolete-p (symbol)
  "Whether M-x leaves SYMBOL out unless typed in full (simple.el):
obsolete, with no current name or for an older major version."
  (let ((obsolete (get symbol 'byte-obsolete-info)))
    (and obsolete
         (not (and (functionp (car obsolete))
                   (condition-case nil
                       (>= (car (version-to-list (caddr obsolete))) emacs-major-version)
                     (error t)))))))

(defun romacs-commands--predicate (buffer)
  "M-x's predicate in BUFFER, as simple.el's: a command that
`read-extended-command-predicate' accepts."
  (lambda (sym)
    (and (commandp sym)
         (cond ((null read-extended-command-predicate))
               ((functionp read-extended-command-predicate)
                ;; Don't let bugs break M-x completion; interpret
                ;; them as the absence of a predicate.
                (condition-case-unless-debug err
                    (funcall read-extended-command-predicate sym buffer)
                  (error (message "read-extended-command-predicate: %s: %s"
                                  sym (error-message-string err)))))))))

;; One list, for the last predicate and modes M-x completed with: a
;; declared predicate's answers in another buffer of the same modes are
;; the same.
(defvar romacs-commands--kept nil
  "M-x's last candidates (`romacs-commands--list').")

(defun romacs-commands--key (buffer)
  "What decides M-x's candidates in BUFFER, but for the commands asked
on each key."
  ;; Copies: turning a minor mode off `delq's it from these lists in place.
  (list (romacs-command-epoch) read-extended-command-predicate (copy-sequence global-minor-modes)
        (and (buffer-live-p buffer)
             (list (buffer-local-value 'major-mode buffer)
                   (copy-sequence (buffer-local-value 'local-minor-modes buffer))))))

(defun romacs-commands--list (pred buffer)
  "PRED's candidates in BUFFER, as (KEY ASKED ANSWERS NAMES OBSOLETE).
ASKED are the commands PRED is asked about again on each key and
ANSWERS what they answered; NAMES are the candidates' names, and
OBSOLETE the candidates left out unless typed in full.  A predicate
not in `romacs-commands-modal-predicates' is asked about every command."
  (let ((modal (or (null read-extended-command-predicate)
                   (memq read-extended-command-predicate romacs-commands-modal-predicates)))
        (candidates nil) (obsolete nil) (asked nil) (answers nil))
    (dolist (sym (romacs-command-symbols))
      (let ((answer (funcall pred sym)))
        (when (or (not modal) (get sym 'completion-predicate))
          (push sym asked)
          (push answer answers))
        (when answer
          (if (romacs-commands--obsolete-p sym) (push sym obsolete) (push (symbol-name sym) candidates)))))
    ;; The key after the predicate ran: what it caches of itself (a
    ;; mode's parents) is no change.
    (list (romacs-commands--key buffer) asked answers (nreverse candidates) obsolete)))

(defun romacs-commands--candidates (pred buffer)
  "PRED's candidates in BUFFER, kept while they hold."
  (let ((kept romacs-commands--kept))
    (unless (and kept (equal (car kept) (romacs-commands--key buffer))
                 (equal (nth 2 kept) (mapcar pred (nth 1 kept))))
      (setq kept (setq romacs-commands--kept (romacs-commands--list pred buffer))))
    kept))

(defun romacs-commands--table (pred buffer)
  "M-x's completion table, given PRED, M-x's predicate in BUFFER.
Its own PRED's answers are kept (`romacs-commands--candidates'); any
other predicate gets simple.el's table over the obarray."
  (lambda (string table-pred action)
    (cond
     ((and suggest-key-bindings (eq action 'metadata))
      '(metadata
        (affixation-function . read-extended-command--affixation)
        (category . command)))
     ((and (eq table-pred pred) (memq action '(nil t)))
      (let* ((kept (romacs-commands--candidates pred buffer))
             (typed (intern-soft string)))
        (complete-with-action
         action (if (and typed (memq typed (nth 4 kept))) (cons string (nth 3 kept)) (nth 3 kept))
         string nil)))
     (t
      (complete-with-action
       action obarray string
       (if (and table-pred (memq action '(nil t)))
           (lambda (sym)
             (and (funcall table-pred sym)
                  (or (equal string (symbol-name sym))
                      (not (romacs-commands--obsolete-p sym)))))
         table-pred))))))

(defun read-extended-command-1 (prompt initial-input)
  (let* ((buffer (current-buffer))
         (pred (romacs-commands--predicate buffer)))
    (minibuffer-with-setup-hook
        (lambda ()
          (add-hook 'post-self-insert-hook
                    (lambda ()
                      (setq execute-extended-command--last-typed
                            (minibuffer-contents)))
                    nil 'local)
          ;; This is so that we define the `M-X' toggling command.
          (read-extended-command-mode)
          (setq-local minibuffer-default-add-function
	              (lambda ()
	                ;; Get a command name at point in the original buffer
	                ;; to propose it after M-n.
	                (let ((def
                               (with-current-buffer
			           (window-buffer (minibuffer-selected-window))
			         (and (commandp (function-called-at-point))
				      (format
                                       "%S" (function-called-at-point)))))
		              (all (sort (minibuffer-default-add-completions)
                                         #'string<)))
		          (if def
		              (cons def (delete def all))
		            all)))))
      ;; Read a string, completing from and restricting to the set of
      ;; all defined commands.  Save the command read on the
      ;; extended-command history list.
      (completing-read
       (concat (cond
	        ((eq current-prefix-arg '-) "- ")
	        ((and (consp current-prefix-arg)
		      (eq (car current-prefix-arg) 4))
		 "C-u ")
	        ((and (consp current-prefix-arg)
		      (integerp (car current-prefix-arg)))
	         (format "%d " (car current-prefix-arg)))
	        ((integerp current-prefix-arg)
	         (format "%d " current-prefix-arg)))
	       ;; This isn't strictly correct if `execute-extended-command'
	       ;; is bound to anything else (e.g. [menu]).
               (or prompt "M-x "))
       (romacs-commands--table pred buffer)
       pred
       t initial-input 'extended-command-history))))

(provide 'romacs-commands)
;;; romacs-commands.el ends here
