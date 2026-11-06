;;; romacs-option.el --- User options as data, for Lisp and agents  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A thin registry over what GNU's Custom already keeps on symbols: search,
;; describe, check, set and save a user option.  It stores nothing of its
;; own but one cache.  A value is checked by GNU's own `:match', the check
;; `setopt' makes, and set and saved by `customize-set-variable' and
;; `customize-save-variable', so a `:set' function runs as it would in GNU.
;; Unlike those, `romacs-option-set' and `romacs-option-save' refuse a value
;; that does not fit `:type'.  `setopt' and friends keep GNU's leniency.
;;
;; An option here is a variable whose `defcustom' has run.  An autoloaded
;; one that has not is outside search until its library loads; setting it
;; loads it, as GNU does.  Faces are found and described, not changed:
;; `custom-set-faces', `set-face-attribute' and themes do that.
;;
;; The MCP tools are in romacs-agent-tools.el.

;;; Code:

(require 'cl-lib)
(require 'cus-edit)

(defconst romacs-option--value-limit 16384
  "Characters of a printed value a record carries.")

(defvar romacs-option--options nil
  "The declared options, sorted by name, or nil when stale.")

(defvar romacs-option--groups (make-hash-table :test #'eq)
  "Each option to the groups that list it, built with `romacs-option--options'.")

(defvar romacs-option--history nil
  "`load-history' as `romacs-option--options' saw it.")

(defun romacs-option--stale (&rest _)
  "Options changed: the next search rebuilds them."
  (setq romacs-option--options nil))

(add-hook 'custom-define-hook #'romacs-option--stale)
(advice-add 'custom-add-to-group :after #'romacs-option--stale)

(defun romacs-option--rebuild ()
  "Walk the symbols once: the declared options, and who lists them."
  (let ((options nil))
    (clrhash romacs-option--groups)
    (mapatoms (lambda (symbol)
                (when (get symbol 'standard-value) (push symbol options))
                (dolist (member (get symbol 'custom-group))
                  (push symbol (gethash (car member) romacs-option--groups)))))
    (setq romacs-option--history load-history
          romacs-option--options (vconcat (sort options #'string<)))))

(defun romacs-option--current ()
  "The declared options, rebuilt if anything changed."
  (unless (and romacs-option--options (eq romacs-option--history load-history))
    (romacs-option--rebuild))
  romacs-option--options)

(defun romacs-option--print (value)
  "VALUE as `read' takes it, cut at `romacs-option--value-limit'."
  (let* ((print-length nil) (print-level nil) (print-escape-newlines t)
         (text (prin1-to-string value)))
    (if (> (length text) romacs-option--value-limit)
        (concat (substring text 0 romacs-option--value-limit) "...")
      text)))

(defun romacs-option--summary (doc)
  "DOC's first line."
  (and (stringp doc) (substring doc 0 (string-search "\n" doc))))

(defun romacs-option--value (option)
  "OPTION's value as Custom reads it; its second element says it has one."
  (if (default-boundp option)
      (list (funcall (or (get option 'custom-get) #'default-value) option) t)
    (list nil nil)))

(defun romacs-option--describe-variable (option)
  "OPTION's record, OPTION being a variable."
  (romacs-option--current)
  (let ((value (romacs-option--value option)))
    (list :name option :kind 'variable
          :doc (documentation-property option 'variable-documentation t)
          :groups (gethash option romacs-option--groups)
          :type (romacs-option--print (get option 'custom-type))
          :standard (romacs-option--print (car (get option 'standard-value)))
          :value (and (cadr value) (romacs-option--print (car value)))
          :state (custom-variable-state option (car value))
          :themes (mapcar #'car (get option 'theme-value))
          :risky (and (get option 'risky-local-variable) t)
          :safe (and (get option 'safe-local-variable) t)
          :local (and (local-variable-if-set-p option) t)
          :setter (and (get option 'custom-set) t)
          :version (get option 'custom-version)
          :obsolete (and (get option 'byte-obsolete-variable) t)
          :loaded (and (get option 'standard-value) t))))

(defun romacs-option--describe-face (face)
  "FACE's record, aliases followed."
  (let ((face (or (get face 'face-alias) face)))
    (list :name face :kind 'face
          :doc (face-documentation face)
          :standard (romacs-option--print (get face 'face-defface-spec))
          :saved (romacs-option--print (get face 'saved-face))
          :customized (romacs-option--print (get face 'customized-face))
          :state (custom-face-state face)
          :themes (mapcar #'car (get face 'theme-face))
          :loaded t)))

(defun romacs-option-describe (symbol)
  "SYMBOL's record as a plist: an option's, or else a face's.
Values are printed, for `read'.  Signals if SYMBOL is neither."
  (let ((option (indirect-variable symbol)))
    (cond ((get option 'standard-value) (romacs-option--describe-variable option))
          ((facep symbol) (romacs-option--describe-face symbol))
          ((get option 'custom-autoload) (list :name option :kind 'variable :loaded nil))
          (t (user-error "Unknown option: %s" symbol)))))

(defun romacs-option--matches-p (name doc words)
  "Whether each of WORDS, lower case, is in NAME or, given, in DOC."
  (let ((doc (and doc (downcase doc))))
    (cl-every (lambda (word) (or (string-search word name) (and doc (string-search word doc))))
              words)))

(defun romacs-option--candidates (kind)
  "The (SYMBOL KIND . DOC-FUNCTION) candidates of KIND, or of both if nil."
  (let ((variable-doc (lambda (option) (documentation-property option 'variable-documentation t))))
    (nconc (unless (eq kind 'face)
             (cl-loop for option across (romacs-option--current)
                      unless (get option 'byte-obsolete-variable)
                      collect (cons option (cons 'variable variable-doc))))
           (unless (eq kind 'variable)
             (mapcar (lambda (face) (cons face (cons 'face #'face-documentation)))
                     (sort (face-list) #'string<))))))

(cl-defun romacs-option-search (query &key group kind docs (limit 50))
  "Options and faces with every word of QUERY in their name, by name, up to LIMIT.
With DOCS a word may be in the documentation instead.  GROUP keeps that
group's members, KIND `variable' or `face' that kind.  Obsolete options
are left out, and options whose library is not loaded are not known.
A list of (SYMBOL KIND SUMMARY)."
  (let ((words (split-string (downcase query))) (found nil) (count 0)
        (members (and group (get group 'custom-group))))
    (cl-loop for (symbol kind . doc-of) in (romacs-option--candidates kind)
             while (< count limit)
             when (and (or (null group) (assq symbol members))
                       (romacs-option--matches-p (symbol-name symbol)
                                                 (and docs (funcall doc-of symbol)) words))
             do (push (list symbol kind (romacs-option--summary (funcall doc-of symbol))) found)
                (setq count (1+ count)))
    (nreverse found)))

(defun romacs-option-check (symbol value)
  "Whether VALUE fits SYMBOL's `:type', by GNU's `:match', as `setopt' asks.
An option with no `:type' takes anything."
  (let ((type (get (indirect-variable symbol) 'custom-type)))
    (or (null type) (and (widget-apply (widget-convert type) :match value) t))))

(defun romacs-option--writable (symbol value)
  "SYMBOL's base option, its library loaded, if VALUE may be given to it."
  (let ((option (indirect-variable symbol)))
    (custom-load-symbol option)
    (cond ((get option 'standard-value)
           (unless (romacs-option-check option value)
             (user-error "Value %s does not fit %s's :type %s" (romacs-option--print value) option
                         (romacs-option--print (get option 'custom-type))))
           option)
          ((facep symbol) (user-error "Faces are read-only here: %s" symbol))
          (t (user-error "Unknown option: %s" symbol)))))

(defun romacs-option--outcome (option previous)
  "What setting OPTION from PREVIOUS, (VALUE HAD-ONE), came to."
  (let ((value (romacs-option--value option)))
    (list :option option
          :previous (and (cadr previous) (romacs-option--print (car previous)))
          :value (romacs-option--print (car value))
          :state (custom-variable-state option (car value)))))

(defun romacs-option-set (symbol value)
  "Give the option SYMBOL VALUE for this session, through its `:set'.
Refuses a VALUE that does not fit its `:type', changing nothing.
Returns (:option :previous :value :state), values printed."
  (let* ((option (romacs-option--writable symbol value))
         (previous (romacs-option--value option)))
    (customize-set-variable option value)
    (romacs-option--outcome option previous)))

(defun romacs-option-save (symbol value)
  "As `romacs-option-set', and keep VALUE for future sessions.
Refuses before changing anything when there is no file to save to."
  (unless (custom-file t)
    (user-error "Saving is unavailable: no file for saved settings"))
  (let* ((option (romacs-option--writable symbol value))
         (previous (romacs-option--value option)))
    (customize-save-variable option value)
    (romacs-option--outcome option previous)))

(provide 'romacs-option)
;;; romacs-option.el ends here
