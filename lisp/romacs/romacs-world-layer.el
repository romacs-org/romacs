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

;;;; What a world build leaves to the session, as code.

;; A world build reads no secret.  A secret reader asked in one
;; (`romacs-world-building-p') does not run: it signals
;; `romacs-secret-deferred', and the code that asked is kept, as code, for
;; the session to evaluate where the source boot would have: a library's
;; top-level form at restore, in load order, among the re-seed forms; one
;; form of the init's configuration for a library at the init's own call
;; for it (`romacs--layer-deferred-after-loads').  The image holds the code
;; that fetches a secret, never the secret.  The condition is no `error',
;; as `quit' is none, so no handler a library or `use-package' wraps around
;; its configuration takes it for a failure; its data holds no secret.
(declare-function romacs--world-secret-asked "world.rs" ())
(declare-function romacs--world-take-secret "world.rs" ())
(declare-function romacs--world-refusals "world.rs" ())
(declare-function romacs--world-forget-refusals "world.rs" (mark))
(declare-function romacs--world-loads "world.rs" ())

(put 'romacs-world-deferred 'error-conditions '(romacs-world-deferred))
(put 'romacs-world-deferred 'error-message "Left to the session")
(put 'romacs-secret-deferred 'error-conditions '(romacs-secret-deferred romacs-world-deferred))
(put 'romacs-secret-deferred 'error-message "Left to the session: a world build reads no secret")

(defun romacs--world-defer-secret (&rest _)
  "A secret reader asked in a world build: it notes that the form running
asked (`romacs--world-secret-asked') and signals `romacs-secret-deferred'."
  (romacs--world-secret-asked)
  (signal 'romacs-secret-deferred nil))

(defun romacs--world-defer-secrets (readers on)
  "Make each of READERS, function symbols, defer in this world build (ON),
or run again.  A reader not defined yet is advised as it is defined."
  (dolist (reader readers)
    (if on (advice-add reader :override #'romacs--world-defer-secret)
      (advice-remove reader #'romacs--world-defer-secret))))

(defvar romacs--world-reseed-forms nil
  "Definitions of an init layer's libraries that read a per-process fact,
and the libraries' top-level forms that asked for a secret.
Each is (KIND SYMBOL FORM CUT DECLARED), innermost first: KIND is
`defcustom', `defvar', `defconst', `defvar-local' or `defimage', or `form'
for another top-level form (SYMBOL nil), FORM the form as loaded, CUT non-nil when an effect the build refused cut the definition
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
unbound first, so the definition declares as it would have; a `form' entry,
a library's top-level form the build left to the session (a secret it
asked), is evaluated whole.  Otherwise a `defcustom' takes
its standard (or saved) value again through its :set function, and the
others set their default from the definition's value form."
  (let ((kind (nth 0 entry)) (symbol (nth 1 entry)) (form (nth 2 entry)))
    (cond ((or (nth 3 entry) (memq kind '(defimage form))) (when symbol (makunbound symbol)) (eval form t))
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

(defvar romacs--layer-entry-timers nil
  "The timers running as a layer build's entry began (`world::layer'):
the ones the entry starts are cancelled at its end, as the forms that
started them run again in each restored process.")

(defvar romacs--layer-deferred-after-loads (make-hash-table :test #'equal)
  "The init's configuration forms a layer build left to the session.
Each key is (LIBRARY . N), as in `romacs--layer-baked-after-loads'; its
value the forms of that call that asked for a secret, each (LEXICAL . FORM)
in order, FORM within the handlers that enclosed it.  The init's call
evaluates them, where GNU evaluates the whole call
\(`romacs--init-eval-after-load').")

(defun romacs--layer-walk (library n lexical form wrap)
  "Evaluate FORM, part of the init's Nth configuration form for LIBRARY,
one smallest whole form at a time: a `progn''s forms in turn, the body of a
`condition-case' or `condition-case-unless-debug' within its handlers, the
branch an `if' takes.  A form that asks for a secret, or meets what the
build cannot hold (`romacs--layer-try'), is kept, WRAP applied (the handlers
around it), for the session to run whole
\(`romacs--layer-deferred-after-loads'), and the next goes on."
  (pcase form
    (`(progn . ,body)
     (dolist (each body) (romacs--layer-walk library n lexical each wrap)))
    (`(,(and head (or 'condition-case 'condition-case-unless-debug)) ,var ,body . ,handlers)
     (let ((inner (lambda (each) (funcall wrap `(,head ,var ,each ,@handlers)))))
       (eval `(,head ,var (romacs--layer-walk ',library ,n ,lexical ',body ',inner) ,@handlers) lexical)))
    (`(if ,test ,then . ,else)
     (let ((tried (romacs--layer-try test lexical)))
       (if tried
           (romacs--layer-walk library n lexical (if (cdr tried) then `(progn ,@else)) wrap)
         ;; The test is the session's to run: the whole `if' with it.
         (romacs--layer-defer library n lexical (funcall wrap form)))))
    (_ (unless (romacs--layer-try form lexical)
         (romacs--layer-defer library n lexical (funcall wrap form))))))

(defun romacs--layer-try (form lexical)
  "Evaluate FORM, LEXICAL, in a layer build: (t . VALUE), or nil when it
is the session's to run.  It is when it asked for a secret, or met, itself
and not in a library it loaded (whose forms answer for theirs), a fact or
an effect the build refused or a timer it started: the build then answers
for nothing it met, and none of its timers runs.  An error after it met
what the build cannot hold (a credential file's read fails there) is the
build's, not the form's."
  (romacs--world-take-secret)
  (let ((refusals (romacs--world-refusals))
        (timers (append timer-list timer-idle-list))
        (loads (romacs--world-loads))
        (deferred nil) (value nil))
    (condition-case err (setq value (eval form lexical))
      (romacs-world-deferred (setq deferred t))
      (error (if (or (romacs--world-take-secret)
                     (and (> (romacs--world-refusals) refusals) (= (romacs--world-loads) loads)))
                 (setq deferred t)
               (signal (car err) (cdr err)))))
    (let ((started (romacs--layer-new-timers timers))
          (own (= (romacs--world-loads) loads)))
      (if (not (or (romacs--world-take-secret) deferred
                   (and own (or started (> (romacs--world-refusals) refusals)))))
          (cons t value)
        (when own
          (romacs--world-forget-refusals refusals)
          (mapc #'cancel-timer started))
        nil))))

(defun romacs--layer-defer (library n lexical form)
  "Keep FORM, LEXICAL, for the init's Nth configuration call for LIBRARY."
  (let ((key (cons library n)))
    (puthash key (append (gethash key romacs--layer-deferred-after-loads) (list (cons lexical form)))
             romacs--layer-deferred-after-loads)))

(defun romacs--layer-new-timers (before)
  "The timers started since BEFORE, the timer lists as they were."
  (let ((new nil))
    (dolist (timer (append timer-list timer-idle-list) new)
      (unless (memq timer before) (push timer new)))))

(defun romacs--layer-load-configured (library forms)
  "Load LIBRARY, a feature or a file the init configured, after FORMS.
Each of FORMS is a printed (LEXICAL . FORM) the init gave
`eval-after-load', in order; each is registered first, so it runs where
GNU runs it, and kept as made by its position (`romacs--layer-baked-after-loads').
A form that does not read back (a closure, which prints as an object, or
one the recording withheld after a secret reader ran) refuses LIBRARY,
which then loads at its first use."
  (let ((n 0))
    (dolist (printed forms)
      (let* ((entry (condition-case nil (car (read-from-string printed))
                      (error (error "%s: the init gave a form a layer cannot replay (a closure, or one given after a secret reader ran)"
                                    library))))
             (lexical (car entry)) (form (cdr entry)) (at n))
        ;; A plan may hold one call twice (merged from sessions whose
        ;; `load-path' differed): it is registered once.  A function the
        ;; init gave is called, as `eval-after-load' calls it.  Each run
        ;; of it (the library loaded again) keeps what it left anew.
        (unless (gethash (cons library at) romacs--layer-baked-after-loads)
          (eval-after-load library
            (lambda ()
              (remhash (cons library at) romacs--layer-deferred-after-loads)
              (romacs--layer-walk library at lexical (if (functionp form) `(funcall ',form) form) #'identity)))))
      (setq n (1+ n))))
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
    (cond (made
           ;; What of it asked for a secret runs now, as the whole call ran
           ;; in GNU; the rest the layer made.
           (let ((deferred (gethash (cons file ordinal) romacs--layer-deferred-after-loads)))
             (when deferred
               (romacs--init-recording-hold t)
               (unwind-protect
                   (dolist (each deferred)
                     (let ((lexical-binding (car each))) (eval (cdr each) (car each))))
                 (romacs--init-recording-hold nil)))))
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
