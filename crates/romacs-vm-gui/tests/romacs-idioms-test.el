;;; romacs-idioms-test.el --- rlisp's commands and key hints stay idiomatic -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Two lints over rlisp/romacs-*.el, in a `--loadup' session
;; (gnu_completion.rs `romacs_idioms_tests_pass').  Each checks a property
;; it can decide exactly, and says what it does not look at:
;;
;; - A command only a Romacs keymap binds declares where it applies, a
;;   mode or a `completion-predicate', so M-x offers it only there.  A
;;   command no keymap binds, or one whose keymap this session has not
;;   loaded, is not looked at: whether such a command is mode-specific is
;;   a judgement.  An autoloaded command that declares this once loaded
;;   declares it before, or M-x offers it everywhere until something
;;   loads its library.
;;
;; - A key name in the text of a call that shows it (`message', `insert',
;;   a mode line or header line...) comes from a binding, through
;;   `substitute-command-keys'.  Text kept in a variable before it reaches
;;   such a call is not looked at.
;;
;; An exception names its reason here; one no longer needed fails.

(require 'cl-lib)

(defconst romacs-idioms-test--autoloaded
  (let ((commands nil))
    (mapatoms (lambda (symbol)
                (when (and (autoloadp (symbol-function symbol)) (string-prefix-p "romacs-" (symbol-name symbol))
                           (not (or (command-modes symbol) (get symbol 'completion-predicate))))
                  (push symbol commands))))
    commands)
  "The Romacs autoloads that declare nothing yet, read before any library loads.")

(defconst romacs-idioms-test--libraries
  '(romacs-agent-ui romacs-agent-history romacs-attach romacs-video romacs-model
    romacs-shader-playground romacs-shaders romacs-surface romacs-mcp)
  "The libraries with keymaps a session does not load by itself.")

(defconst romacs-idioms-test--anywhere
  '((romacs-agent-switch "Reaches a session from any buffer; a chat binds it too"))
  "Commands a Romacs keymap binds that apply anywhere, each (COMMAND REASON).")

(defconst romacs-idioms-test--literal-keys
  '(("romacs-shader-playground.el" "mouse-2, RET: jump to this line in the playground"
     "GNU's own wording for a button's help-echo, shown in a tooltip, which has no faces"))
  "Key names kept as text, each (FILE TEXT REASON).")

(defconst romacs-idioms-test--showing
  '(message user-error error yes-or-no-p y-or-n-p insert insert-before-markers propertize
    romacs-gui-echo romacs-layer-toast)
  "Functions whose arguments reach the user as text.")

(defconst romacs-idioms-test--not-text
  '(kbd key-parse key-valid-p keymap-set keymap-unset keymap-global-set keymap-local-set
    global-set-key local-set-key define-key defvar-keymap define-keymap keymap-popup-define
    substitute-command-keys romacs-key-hint)
  "Forms whose strings name keys for a keymap, or are `substitute-command-keys' input.")

(defconst romacs-idioms-test--key
  (rx (or bos (any " (\"'`/;,"))
      (or (seq (+ (any "CMSHs") "-") nonl) "RET" "TAB" "SPC" "ESC" "DEL")
      (or eos (any " )\"'/;,.:")))
  "A key's name, as `key-description' spells it, and not a word like C-level.")

;;;; Commands

(defun romacs-idioms-test--commands-in (map seen)
  "The symbols MAP binds, its nested keymaps' too, consed onto SEEN's car.
SEEN is a list of one list, and its cdr the keymaps already walked."
  (unless (memq map (cdr seen))
    (setcdr seen (cons map (cdr seen)))
    (map-keymap
     (lambda (_event binding)
       (let ((binding (if (eq (car-safe binding) 'menu-item) (nth 2 binding) binding)))
         (cond ((keymapp binding) (romacs-idioms-test--commands-in binding seen))
               ((symbolp binding) (setcar seen (cons binding (car seen)))))))
     map))
  seen)

(defun romacs-idioms-test--keymap-commands ()
  "The Romacs commands Romacs's keymaps bind and the global map does not."
  (let ((seen (list nil)))
    (mapatoms (lambda (symbol)
                (when (and (boundp symbol) (string-match-p "\\`romacs-.*-map\\'" (symbol-name symbol))
                           (keymapp (symbol-value symbol)))
                  (romacs-idioms-test--commands-in (symbol-value symbol) seen))))
    (cl-remove-if-not (lambda (command)
                        (and (commandp command) (string-prefix-p "romacs-" (symbol-name command))
                             (not (where-is-internal command (list global-map) t))))
                      (delete-dups (car seen)))))

(defun romacs-idioms-test--declared-p (command)
  "Whether COMMAND says where M-x offers it."
  (or (command-modes command) (get command 'completion-predicate)))

;;;; Key names

(defun romacs-idioms-test--strings (form showing report)
  "Call REPORT with each string of FORM that reaches the user.
SHOWING says FORM is inside a call that shows its arguments."
  (cond
   ((stringp form) (when showing (funcall report form)))
   ((and (consp form) (proper-list-p form))
    (let ((head (car form)))
      (cond
       ((memq head romacs-idioms-test--not-text) nil)
       (t (let ((showing (or showing (memq head romacs-idioms-test--showing)
                             (and (memq head '(setq setq-local))
                                  (or (memq 'mode-line-format form) (memq 'header-line-format form))))))
            (dolist (part form)
              (romacs-idioms-test--strings part showing report)))))))
   ((consp form)
    (romacs-idioms-test--strings (car form) showing report)
    (romacs-idioms-test--strings (cdr form) showing report))))

(defun romacs-idioms-test--file-keys (file)
  "The texts FILE shows with a key's name in them."
  (let ((found nil))
    (with-temp-buffer
      (insert-file-contents file)
      (goto-char (point-min))
      (condition-case nil
          (while t
            (romacs-idioms-test--strings
             (read (current-buffer)) nil
             (lambda (text) (when (string-match-p romacs-idioms-test--key text) (push text found)))))
        (end-of-file nil)))
    found))

(defun romacs-idioms-test--sources ()
  "rlisp/romacs-*.el."
  (directory-files (file-name-directory (locate-library "romacs-term")) t "\\`romacs-.*\\.el\\'"))

;;;; The lints

(romacs-deftest romacs-idioms-command-declares-where-it-applies ()
  "A command only a Romacs keymap binds declares its mode or a predicate."
  (mapc #'require romacs-idioms-test--libraries)
  (let ((commands (romacs-idioms-test--keymap-commands)))
    (romacs-should (> (length commands) 60))
    (romacs-should-not (cl-remove-if (lambda (command) (or (romacs-idioms-test--declared-p command)
                                                           (assq command romacs-idioms-test--anywhere)))
                                     commands))
    ;; An exception still needed.
    (romacs-should-not (cl-remove-if (lambda (entry) (and (memq (car entry) commands)
                                                          (not (romacs-idioms-test--declared-p (car entry)))))
                                     romacs-idioms-test--anywhere))
    ;; Declared once loaded, so declared before it.
    (romacs-should (> (length romacs-idioms-test--autoloaded) 20))
    (romacs-should-not (cl-remove-if-not #'romacs-idioms-test--declared-p romacs-idioms-test--autoloaded))))

(defun romacs-idioms-test--listed-p (entry entries)
  "Whether ENTRIES hold ENTRY's file and text."
  (cl-find-if (lambda (other) (and (equal (car entry) (car other)) (equal (cadr entry) (cadr other)))) entries))

(romacs-deftest romacs-idioms-key-names-come-from-bindings ()
  "Text shown to the user holds no key's name it spelled out itself."
  (let ((sources (romacs-idioms-test--sources)) (found nil))
    (romacs-should (> (length sources) 60))
    (dolist (file sources)
      (dolist (text (romacs-idioms-test--file-keys file))
        (push (list (file-name-nondirectory file) text) found)))
    (romacs-should-not (cl-remove-if (lambda (entry) (romacs-idioms-test--listed-p entry romacs-idioms-test--literal-keys))
                                     found))
    ;; An exception still needed.
    (romacs-should-not (cl-remove-if (lambda (entry) (romacs-idioms-test--listed-p entry found))
                                     romacs-idioms-test--literal-keys))))

(romacs-deftest romacs-idioms-lints-see-what-they-exist-for ()
  "The key lint's walk finds a key in each shape it looks at, and no other."
  (let ((found nil))
    (dolist (form '((message "Press C-c C-c")
                    (insert (format "%s" (propertize "RET open" 'face 'bold)))
                    (setq-local header-line-format '("q" "M-x foo"))
                    (when t (user-error "No, TAB"))))
      (romacs-idioms-test--strings form nil (lambda (text) (when (string-match-p romacs-idioms-test--key text) (push text found)))))
    (romacs-should (equal (nreverse found) '("Press C-c C-c" "RET open" "M-x foo" "No, TAB"))))
  (let ((found nil))
    (dolist (form '((defun f () "Press C-c C-c" (kbd "C-c C-c"))
                    (message "%s" (substitute-command-keys "\\[foo] and C-c"))
                    (define-key map (kbd "RET") 'x)
                    (message "A C-level word, and SPCs, are no keys")))
      (romacs-idioms-test--strings form nil (lambda (text) (when (string-match-p romacs-idioms-test--key text) (push text found)))))
    (romacs-should (equal found nil))))

(provide 'romacs-idioms-test)
;;; romacs-idioms-test.el ends here
