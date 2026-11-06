;;; romacs-world-layer.el --- What a world's init layer keeps and restores  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; An init layer (crates/romacs-vm/src/world/layer.rs) is a world that also
;; holds the libraries a user's init loads, and the ones it only declares:
;; the features its `eval-after-load' calls configure (`use-package''s
;; :config, :bind, :hook and :defer, `with-eval-after-load') and the files
;; its `autoload' calls name.  A build loads each declared library as its
;; first use would, after registering the init's forms for it, so they run
;; where GNU runs them; the init, which gives the same forms again at each
;; start, finds them made and skips them.
;;
;; The libraries' definitions that read a per-process fact are evaluated
;; again in each restored process; a declared library's that compute a
;; value only wait for their variable's first access (world/pending.rs).

;;; Code:

(defvar romacs--world-reseed-forms nil
  "Definitions of an init layer's libraries that read a per-process fact.
Each is (KIND SYMBOL FORM CUT DECLARED), innermost first: KIND is
`defcustom', `defvar', `defconst', `defvar-local' or `defimage', FORM the definition
as loaded, CUT non-nil when an effect the build refused cut the definition
short, so SYMBOL was defined as nil, and DECLARED non-nil when a library
the init only declared defined it.  A world build keeps them
\(`world_form_done'); `romacs-world-restart' evaluates each again in this
process, as startup.el's delayed init re-evaluates the dumped options
whose values depend on the process, or leaves it pending
\(`romacs--world-reseed-lazy-p').")

(defvar romacs--world-pending nil
  "The `romacs--world-reseed-forms' entries in load order, as a vector.
A pending variable (`romacs--world-pend') names its entry by index.")

(defun romacs--world-reseed-form (entry)
  "Evaluate ENTRY, a `romacs--world-reseed-forms' element, in this process.
A definition cut short, or a `defimage', is evaluated whole, its symbol
unbound first, so the definition declares as it would have.  Otherwise a `defcustom' takes
its standard (or saved) value again through its :set function, and the
others set their default from the definition's value form."
  (let ((kind (nth 0 entry)) (symbol (nth 1 entry)) (form (nth 2 entry)))
    (cond ((or (nth 3 entry) (eq kind 'defimage)) (makunbound symbol) (eval form t))
          ((eq kind 'defcustom) (custom-reevaluate-setting symbol))
          (t (set-default symbol (eval (nth 2 form) t))))))

(defun romacs--world-reseed-lazy-p (entry)
  "Whether ENTRY, a `romacs--world-reseed-forms' element, may wait.
A declared library's `defvar', `defconst', `defimage' or plain `defcustom' (no :set or
:initialize of its own) computes a value and nothing else, so it is
computed at its variable's first access instead of at every start, which
is when GNU computes it: at the library's first use.  A buffer-local
definition, or a `defcustom' whose setter may act, is evaluated now."
  (let ((kind (nth 0 entry)) (keys (nthcdr 4 (nth 2 entry))))
    (and (nth 4 entry)
         (memq kind '(defvar defconst defcustom defimage))
         (not (and (eq kind 'defcustom)
                   (or (plist-member keys :set) (plist-member keys :initialize)))))))

(defun romacs--world-reseed-all ()
  "Evaluate each `romacs--world-reseed-forms' definition in load order.
One that may wait (`romacs--world-reseed-lazy-p') is left pending until its
variable's first access instead (`romacs--world-pend'), and every such one
is pending again in each process, whatever an earlier one forced."
  (setq romacs--world-pending (vconcat (reverse romacs--world-reseed-forms)))
  (dotimes (index (length romacs--world-pending))
    (let ((entry (aref romacs--world-pending index)))
      (unless (and (romacs--world-reseed-lazy-p entry)
                   (romacs--world-pend (nth 1 entry) index))
        (romacs--world-reseed-form entry)))))

(defun romacs--world-force (index)
  "Evaluate the pending definition at INDEX of `romacs--world-pending'.
Its variable's first access calls this (`romacs--world-pend'); an error is
reported and the variable keeps the value the world held."
  (let ((entry (aref romacs--world-pending index)))
    (condition-case err
        (romacs--world-reseed-form entry)
      (error (message "Re-seeding %s: %s" (nth 1 entry) (error-message-string err))))))

;;;; The init layer's declared libraries (`world::layer').

(defcustom romacs-init-layer-exclude nil
  "Libraries the init layer leaves out: features, or files as named.
A library the init loads or declares (`use-package', an `autoload', an
`eval-after-load') is otherwise held by the init's layer, so its first use
loads nothing; one listed here loads at that use, as in GNU Emacs."
  :type '(repeat (choice symbol string))
  :group 'initialization)

(defvar romacs--layer-baked-after-loads (make-hash-table :test #'equal)
  "The `eval-after-load' calls an init layer made for the init, as keys.
Each key is (LIBRARY . N): the init's Nth such call for LIBRARY, counting
from 0.  The init makes it again at each start, and it already ran: a
layer is restored only for the init content its plan was recorded from.")

(defun romacs--layer-printed (form)
  "FORM as the init gave it to `eval-after-load', with `lexical-binding'."
  (let ((print-length nil) (print-level nil) (print-circle t)
        (print-escape-newlines nil) (print-quoted t))
    (prin1-to-string (cons lexical-binding form))))

(defun romacs--layer-load-declared (file)
  "Load FILE, declared by an init's `autoload', unless it is loaded."
  (unless (load-history-filename-element (load-history-regexp file))
    (load file nil t)))

(defun romacs--layer-load-configured (library forms)
  "Load LIBRARY, a feature or a file the init configured, after FORMS.
Each of FORMS is a printed (LEXICAL . FORM) the init gave
`eval-after-load', in order; each is registered first, so it runs where
GNU runs it, and kept as made by its position (`romacs--layer-baked-after-loads').
A form that does not read back (a closure, which prints as an object, or
one the recording withheld after a secret reader ran) refuses LIBRARY,
which then loads at its first use."
  (dolist (printed forms)
    (let* ((entry (condition-case nil (car (read-from-string printed))
                    (error (error "%s: the init gave a form a layer cannot replay (a closure, or one given after a secret reader ran)"
                                  library))))
           (lexical-binding (car entry)))
      (eval-after-load library (cdr entry))))
  (if (symbolp library) (require library nil t) (load library t t))
  (unless (if (symbolp library) (featurep library)
            (load-history-filename-element (load-history-regexp library)))
    (error "%s is not installed" library))
  (dotimes (n (length forms))
    (puthash (cons library n) t romacs--layer-baked-after-loads)))

(defun romacs--init-eval-after-load (eval-after-load file form)
  "`eval-after-load' of FILE and FORM while the user's init loads.
A call the init's layer already made is not made again; one for a
library not loaded yet is noted for the layer's plan, as is a made one.
One that runs now runs with the init's recording held, so its loads are
noted as its library's, wherever that library was loaded."
  (let* ((ordinal (romacs--init-after-load-ordinal file))
         (made (and ordinal (gethash (cons file ordinal) romacs--layer-baked-after-loads)))
         (loaded (if (stringp file)
                     (load-history-filename-element (load-history-regexp file))
                   (featurep file))))
    (when (and ordinal (or made (not loaded)))
      (romacs--note-init-after-load file (romacs--layer-printed form)))
    (cond (made nil)
          ;; It runs now: what it loads is its library's configuration's.
          (loaded (romacs--init-recording-hold t)
                  (unwind-protect (funcall eval-after-load file form)
                    (romacs--init-recording-hold nil)))
          (t (funcall eval-after-load file form)))))

(defun romacs--init-layer-begin ()
  "The user's init loads from here: see `romacs--init-eval-after-load'."
  (advice-add 'eval-after-load :around #'romacs--init-eval-after-load))

(defun romacs--init-layer-end ()
  "The user's init has loaded."
  (advice-remove 'eval-after-load #'romacs--init-eval-after-load))

(provide 'romacs-world-layer)
;;; romacs-world-layer.el ends here
