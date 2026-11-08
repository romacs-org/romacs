;;; romacs-loadup-bootstrap.el --- GNU loadup as source, for package suites
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The files GNU's loadup.el preloads on GNU/Linux, as source, in loadup's
;; order (of its window-system blocks, fringe.el and the PGTK one), then the steps of
;; startup.el's `command-line' a batch session depends on.  Romacs's own display and
;; keymap layers load where GNU's C would already be in place.  Each Romacs
;; adaptation is commented where it happens.
;;
;; This is the regime for package suites (Gnosis, emacs-hermes) and for
;; GNU's own test suite (gnu-tests/run.py, the gnu-ert regime), which
;; assume a dumped GNU Emacs.

;;; Code:

(load "emacs-lisp/debug-early.el" nil t t)
(load "emacs-lisp/byte-run.el" nil t t)
(load "emacs-lisp/backquote.el" nil t t)
;; Every load expands its templates, so the walk is native, and while
;; these two stay as they are here the compiler takes its answer without
;; calling the macro (DIVERGENCES.md VM2-BACKQUOTE).
(defalias 'backquote-process (symbol-function 'romacs--backquote-process))
(put 'backquote 'romacs--standard-macro (cdr (symbol-function 'backquote)))
;; subr.el's defvars own these maps in GNU; the native bare-VM skeleton
;; binds them on first keymap use, so yield them before subr.el runs.  The
;; C-level keymaps (function-key-map and friends) stay native.
(current-global-map)
(dolist (map '(global-map esc-map ctl-x-map)) (makunbound map))
(load "subr.el" nil t t)
(load "keymap.el" nil t t)
(load "version.el" nil t t)
(load "widget.el" nil t t)
(load "custom.el" nil t t)
(load "emacs-lisp/map-ynp.el" nil t t)
(load "international/mule.el" nil t t)
(load "international/mule-conf.el" nil t t)
;; loadup.el:143.
(setq load-source-file-function #'load-with-code-conversion)
(load "env.el" nil t t)
(load "format.el" nil t t)
(load "bindings.el" nil t t)
(load "window.el" nil t t)
;; loadup.el: mini-windows can resize now, so the variable takes its
;; advertised default (xdisp.c starts it as nil).
(setq resize-mini-windows 'grow-only)
(load "files.el" nil t t)
(load "emacs-lisp/macroexp.el" nil t t)
(load "cus-face.el" nil t t)
(load "faces.el" nil t t)
(load "loaddefs.el" nil t t)
(load "button.el" nil t t)
;; loadup.el: add-hook is interpreted here, so gv is required now.
(require 'gv)
(load "emacs-lisp/cl-preloaded.el" nil t t)
;; cl-preloaded.el's `eval-when-compile' requires cl-macs from source
;; before it defines `cl--define-derived-type', so cl-macs's `static-if'
;; left out its derived types (`character', `base-char', `extended-char',
;; `real').  GNU's cl-macs.elc took that branch when it was compiled, so
;; cl-macs loads again now that the function exists.
(load "emacs-lisp/cl-macs.el" nil t t)
(load "emacs-lisp/oclosure.el" nil t t)
(load "obarray.el" nil t t)
(load "abbrev.el" nil t t)
(load "help.el" nil t t)
(load "jka-cmpr-hook.el" nil t t)
(load "epa-hook.el" nil t t)
(load "international/mule-cmds.el" nil t t)
(load "case-table.el" nil t t)
(load "international/charprop.el" t t t)
(load "international/characters.el" nil t t)
(load "composite.el" nil t t)
(load "language/chinese.el" nil t t)
(load "language/cyrillic.el" nil t t)
(load "language/indian.el" nil t t)
(load "language/sinhala.el" nil t t)
(load "language/english.el" nil t t)
(load "language/ethiopic.el" nil t t)
(load "language/european.el" nil t t)
(load "language/czech.el" nil t t)
(load "language/slovak.el" nil t t)
(load "language/romanian.el" nil t t)
(load "language/greek.el" nil t t)
(load "language/hebrew.el" nil t t)
(load "international/cp51932.el" nil t t)
(load "international/eucjp-ms.el" nil t t)
(load "language/japanese.el" nil t t)
(load "language/korean.el" nil t t)
(load "language/lao.el" nil t t)
(load "language/tai-viet.el" nil t t)
(load "language/thai.el" nil t t)
(load "language/tibetan.el" nil t t)
(load "language/vietnamese.el" nil t t)
(load "language/misc-lang.el" nil t t)
(load "language/utf-8-lang.el" nil t t)
(load "language/georgian.el" nil t t)
(load "language/khmer.el" nil t t)
(load "language/burmese.el" nil t t)
(load "language/cham.el" nil t t)
(load "language/philippine.el" nil t t)
(load "language/indonesian.el" nil t t)
(load "indent.el" nil t t)
;; GNU cl-generic, loaded through Romacs's type-query bridge.
(require 'romacs-cl-generic)
;; cl-print's closure method walks body slots VM2 closures do not expose
;; (VM2-CLOSEQ1).
(with-eval-after-load 'cl-print (require 'romacs-cl-print))
;; One walk for every frame's locals (DIVERGENCES DBG-GET-FRAMES).
(with-eval-after-load 'backtrace (require 'romacs-backtrace))
;; SCRAM's key derivation natively (VM2-SCRAM1).
(with-eval-after-load 'sasl-scram-rfc (require 'romacs-sasl-scram))
(load "simple.el" nil t t)
;; The region is drawn natively (`snapshot_region'): no overlay for it.
(setq redisplay-highlight-region-function #'romacs--native-region)
(require 'romacs-function-documentation)
(load "emacs-lisp/seq.el" nil t t)
(load "emacs-lisp/nadvice.el" nil t t)
;; VM2 code is compiled when defined: `byte-compile' of a function value
;; returns it (VM2-BYTECOMP1).
(require 'romacs-bytecomp)
;; GNU cl-print reads an interpreted function's slots, which Romacs's
;; closures keep private (VM2-ERT-BATCH1): whenever cl-print loads (debug.el's
;; *Backtrace*, ERT), they print through the owned method instead.
(with-eval-after-load 'cl-print (require 'romacs-cl-print))
;; One walk for every frame's locals (DIVERGENCES DBG-GET-FRAMES).
(with-eval-after-load 'backtrace (require 'romacs-backtrace))
;; SCRAM's key derivation natively (VM2-SCRAM1).
(with-eval-after-load 'sasl-scram-rfc (require 'romacs-sasl-scram))
;; package.el and package-vc.el install; their long work runs in a worker VM
;; in a session (romacs-package.el).  GNU's own test suite, which
;; `EMACS_TEST_DIRECTORY' marks, tests them as they are.
(unless (getenv "EMACS_TEST_DIRECTORY")
  (with-eval-after-load 'package (require 'romacs-package)))
(load "minibuffer.el" nil t t)
;; minibuffer.el's candidate sorts, natively: through `sort' with a Lisp
;; predicate they cost most of a completion key over thousands of
;; candidates (DIVERGENCES.md VM2-COMPLETION-SORT).
(defalias 'minibuffer--sort-by-length-alpha #'romacs--sort-by-length-alpha)
(defalias 'minibuffer--sort-by-position #'romacs--sort-by-history)
(load "frame.el" nil t t)
(load "startup.el" nil t t)
(load "term/tty-colors.el" nil t t)
;; Romacs owns the display layer of frame.el and faces.el (VM2-DISPLAY1).
(load "romacs-frame.el" nil t t)
;; ... and `enable-theme' and `disable-theme' loop over faces natively.
(load "romacs-custom.el" nil t t)
;; GNU computes the active keymaps in C; Romacs in Lisp.
(load "romacs-keymap.el" nil t t)
;; GNU describes keymaps, char-tables and syntax in C; Romacs in Lisp.
(load "romacs-help.el" nil t t)
(load "font-core.el" nil t t)
(load "emacs-lisp/syntax.el" nil t t)
(load "font-lock.el" nil t t)
(load "jit-lock.el" nil t t)
(load "mouse.el" nil t t)
;; loadup.el:268: a toolkit build (pinned GNU's PGTK) defines scroll bars.
(load "scroll-bar.el" nil t t)
(load "select.el" nil t t)
(load "emacs-lisp/timer.el" nil t t)
(load "emacs-lisp/easymenu.el" nil t t)
(load "isearch.el" nil t t)
(load "rfn-eshadow.el" nil t t)
(load "menu-bar.el" nil t t)
(load "tab-bar.el" nil t t)
(load "emacs-lisp/lisp.el" nil t t)
(load "textmodes/page.el" nil t t)
(load "register.el" nil t t)
(load "textmodes/paragraphs.el" nil t t)
(load "progmodes/prog-mode.el" nil t t)
(load "emacs-lisp/lisp-mode.el" nil t t)
(load "textmodes/text-mode.el" nil t t)
(load "textmodes/fill.el" nil t t)
(load "newcomment.el" nil t t)
(load "replace.el" nil t t)
(load "emacs-lisp/tabulated-list.el" nil t t)
(load "buff-menu.el" nil t t)
;; loadup.el's window-system block (loadup.el:290-298): Romacs is a
;; graphical machine, and pinned GNU, a graphical build, preloads it in
;; batch too.  Then dynamic-setting (loadup.el:300), which GNU's
;; xsettings.c provides in that build.
(load "fringe.el" nil t t)
(load "emacs-lisp/regexp-opt.el" nil t t)
(load "image.el" nil t t)
(load "international/fontset.el" nil t t)
(load "dnd.el" nil t t)
(load "tool-bar.el" nil t t)
(load "dynamic-setting.el" nil t t)
;; loadup.el's PGTK block (loadup.el:357).  GNU's C provides `pgtk' in a
;; pure-GTK build (pgtkterm.c:7550), which pinned GNU is; Romacs's
;; Wayland-native display stands where that window system would.  Its
;; frames are `pgtk' frames once the GUI session opens it (VM2-WIN5), so
;; pgtk-win.el's backend methods run there, over the pgtk primitives
;; romacs-pgtk.el provides; the display queries are Romacs's own
;; (VM2-WINSYS1).
(provide 'pgtk)
(load "pgtk-dnd.el" nil t t)
(load "touch-screen.el" nil t t)
(load "term/common-win.el" nil t t)
(load "term/pgtk-win.el" nil t t)
(load "romacs-pgtk.el" nil t t)
;; menu.c's popup menus and dialogs, as GNU's text menus (VM2-MENU1).
(load "romacs-menu.el" nil t t)
;; loadup.el:363, after the window system's file.
(load "mwheel.el" nil t t)
(load "progmodes/elisp-mode.el" nil t t)
(load "emacs-lisp/float-sup.el" nil t t)
(load "vc/vc-hooks.el" nil t t)
(load "vc/ediff-hook.el" nil t t)
(load "uniquify.el" nil t t)
(load "electric.el" nil t t)
(load "paren.el" nil t t)
(load "emacs-lisp/shorthands.el" nil t t)
(load "emacs-lisp/eldoc.el" nil t t)
(load "emacs-lisp/cconv.el" nil t t)
;; loadup.el preloads cus-start with this flag bound, under `dump-mode',
;; where it does not provide itself: Custom's `(require 'cus-start)'
;; then loads the types and groups the preload skips.
(dlet ((cus-start--preload t)) (load "cus-start.el" nil t t))
(setq features (delq 'cus-start features))
(load "tooltip.el" nil t t)
(load "international/iso-transl.el" nil t t)
(load "emacs-lisp/rmc.el" nil t t)
(load "leim/leim-list.el" nil t t)
;; loadup.el:476: Snarf-documentation (doc.c:500), as loadup runs it when
;; it does not dump: the C variables' docstrings, from DOC, for the help
;; commands.  DOC's functions need nothing here: a primitive finds its own
;; record when asked (`internal-subr-documentation').
(defun Snarf-documentation (filename)
  "Scan the DOC file FILENAME, in `doc-directory', for variable docstrings.
Each variable it documents that is bound, or whose initialization
Custom delayed, gets its record's position as `variable-documentation'."
  (let ((delayed (and (consp custom-delayed-init-variables) custom-delayed-init-variables)))
    (setq internal-doc-file-name filename)
    (dolist (record (romacs--doc-variable-records filename))
      (let ((symbol (intern-soft (car record))))
        (when (and symbol (or (boundp symbol) (memq symbol delayed)))
          (put symbol 'variable-documentation (cdr record)))))))
(condition-case nil
    (Snarf-documentation "DOC")
  (error nil))
;; loadup.el:569: with the preloaded files in, `debug-early' gives way to
;; the full debugger.
(setq debugger #'debug)
;; loadup.el:391.
(setq internal-make-interpreted-closure-function #'cconv-make-interpreted-closure)
;; lread.c Fload keeps the name of each file the dump loads, newest first:
;; GNU's Lisp in this chain, relative to its directory.
(setq preloaded-file-list
      (let ((root (file-name-directory (locate-file "simple" load-path '(".el")))))
        (delq nil (mapcar (lambda (entry)
                            (and (stringp (car entry)) (string-prefix-p root (car entry))
                                 (file-name-sans-extension (substring (car entry) (length root)))))
                          load-history))))

;; startup.el command-line (startup.el:1479): variables whose standard value
;; depends on the runtime (user-mail-address and friends) get it once the
;; preloaded files are in.  Romacs does not run command-line in this
;; bootstrap, so it takes this one step itself, in batch.  A session that
;; starts in another context evaluates the same list again in its own
;; (rlisp/romacs-gui-loadup.el).
(defvar romacs-loadup-delayed-init-variables
  (and (listp custom-delayed-init-variables) (reverse custom-delayed-init-variables))
  "The preloaded variables whose standard value depends on the runtime, in load order.
These are `custom-delayed-init-variables' as the preloaded files left it.")
;; The locale step changes the language and coding state the chain left; a
;; later step, as a VM restored from a world takes in its own process,
;; starts again from that state, as a fresh process starts from its dump's.
(defconst romacs--locale-variables
  '(current-language-environment current-locale-environment current-iso639-language
    system-messages-locale system-time-locale woman-locale locale-coding-system
    default-sendmail-coding-system default-process-coding-system
    default-file-name-coding-system default-terminal-coding-system
    default-keyboard-coding-system keyboard-coding-system default-input-method
    buffer-file-coding-system coding-category-list char-width-table)
  "The variables `set-locale-environment' sets, directly or through
`set-language-environment' and `prefer-coding-system', besides each coding
category's (`coding-category-list').")
(defvar romacs--locale-defaults nil
  "The language and coding state before this session's first locale step
\(`romacs--locale-state'), which each later step starts from.")
(defun romacs--locale-state ()
  "What the locale step changes: each of `romacs--locale-variables' and
of the coding categories by its default value (a bare symbol when void;
a char-table copied), the display table, the coding system and charset
priorities, and the terminal and keyboard coding systems."
  (list (mapcar (lambda (variable)
                  (if (default-boundp variable)
                      (let ((value (default-value variable)))
                        (cons variable (if (char-table-p value) (copy-sequence value) value)))
                    variable))
                (append romacs--locale-variables coding-category-list))
        (and standard-display-table (copy-sequence standard-display-table))
        (coding-system-priority-list)
        (charset-priority-list)
        (terminal-coding-system)
        (keyboard-coding-system)))
(defun romacs--restore-locale-state (state)
  "Give the language and coding state STATE (`romacs--locale-state') back.
A language environment other than STATE's is left first, as
`set-language-environment' leaves it.  Only startup's own replay, before
the user's init, may call this: it discards their coding and language
settings."
  (let ((variables (nth 0 state)))
    (unless (equal current-language-environment
                   (cdr (assq 'current-language-environment variables)))
      (run-hooks 'exit-language-environment-hook)
      (let ((func (get-language-info current-language-environment 'exit-function)))
        (if (functionp func) (funcall func))))
    (apply #'set-coding-system-priority (nth 2 state))
    ;; The whole order, categories with no coding system included.
    (apply #'romacs--set-coding-category-order (cdr (assq 'coding-category-list variables)))
    (apply #'set-charset-priority (nth 3 state))
    (set-terminal-coding-system-internal (nth 4 state))
    (set-keyboard-coding-system-internal (nth 5 state))
    ;; After the priorities, which set the coding categories' variables too.
    (dolist (variable variables)
      (if (consp variable)
          (set-default (car variable) (if (char-table-p (cdr variable)) (copy-sequence (cdr variable)) (cdr variable)))
        (makunbound variable)))
    (setq standard-display-table (and (nth 1 state) (copy-sequence (nth 1 state))))))
;; The per-process steps startup.el takes before the init file, in batch
;; and in an interactive session alike.
(defun romacs--startup-environment ()
  "Take the per-process steps startup.el takes before the init file.
`normal-top-level' (startup.el:639-668) sets Unix's end-of-line
mnemonics, then the language environment and coding systems from this
process's LANG and LC_* (`set-locale-environment'), from the state the
chain left (`romacs--locale-defaults'); `command-line' \(startup.el:1300)
then begins initialization, in the directory the process started in."
  (setq eol-mnemonic-dos "(DOS)"
        eol-mnemonic-mac "(Mac)")
  ;; startup.el command-line (startup.el:1343): where this process's Lisp is.
  (setq lisp-directory (file-name-directory (locate-file "simple" load-path '(".el"))))
  ;; The replay is startup's, before the user's init only.
  (if romacs--locale-defaults
      (romacs--restore-locale-state romacs--locale-defaults)
    (setq romacs--locale-defaults (romacs--locale-state)))
  (set-locale-environment nil)
  (setq before-init-time (current-time) after-init-time nil
        command-line-default-directory default-directory))
;; The steps of startup.el's `command-line' a batch session takes in each
;; process.  `romacs--startup-phase' marks their extent: a world build may
;; read per-process host facts only inside it, and a VM restored from a
;; world runs them again (`romacs-world-restart').
(defun romacs-batch-startup ()
  "Take the per-process steps of startup.el's `command-line', as batch does.
The per-user directory is found as `normal-top-level' finds it, then the
locale and the start of initialization (`romacs--startup-environment'),
the delayed variables get their standard values in this process's context,
and `after-init-time' its clock."
  (unwind-protect
      (progn
        (romacs--startup-phase t)
        ;; startup.el normal-top-level (startup.el:585-592), before
        ;; command-line: the per-user directory, which
        ;; `locate-user-emacs-file' and the delayed standard values build on.
        ;; A GUI session then names its own (rlisp/romacs-gui-loadup.el).
        (setq startup--xdg-config-home-emacs
              (let ((xdg-config-home (getenv-internal "XDG_CONFIG_HOME")))
                (if xdg-config-home
                    (concat xdg-config-home "/emacs/")
                  startup--xdg-config-default)))
        (setq user-emacs-directory
              (startup--xdg-or-homedot startup--xdg-config-home-emacs nil))
        ;; The frame init_display made; a window session has none
        ;; (rlisp/romacs-gui-loadup.el).
        (setq terminal-frame (selected-frame))
        (romacs--startup-environment)
        (let ((user-emacs-directory-warning nil))
          (mapc #'custom-reevaluate-setting romacs-loadup-delayed-init-variables))
        (setq custom-delayed-init-variables t)
        ;; startup.el command-line (startup.el:1717): initialization is over.
        (setq after-init-time (current-time)))
    (romacs--startup-phase nil)))
(defvar romacs-startup-function #'romacs-batch-startup
  "The per-process startup steps this session took last.
A session that takes its own after the chain's sets it to them
\(rlisp/romacs-gui-loadup.el); `romacs-world-restart' takes them again.")
(romacs-batch-startup)
;; startup.el normal-top-level (startup.el:843): the initial frame's faces
;; for its background mode and display type.  It reads no per-process fact
;; in batch, and GNU calls it idempotent, so it is loadup's, not startup's:
;; a world keeps its result.
(frame-set-background-mode (selected-frame))

(defconst romacs--world-reseed
  '((cl-extra "emacs-lisp/cl-extra.el"
     (setq cl--random-state (cl--make-random-state (cl--random-time)))
     "GNU dumps no cl- library but cl-preloaded, so each process seeds the state from its clock when cl-extra loads")
    (project "progmodes/project.el"
     ;; As loadup loads it: before startup names the per-user directory,
     ;; which is still subr.el's nil, so the file is the working directory's.
     (let ((user-emacs-directory nil))
       (custom-reevaluate-setting 'project-list-file))
     "GNU dumps no project.el; loadup loads it (flymake-log's eager expansion), so each process computes the file against its own working directory")
    (find-func "emacs-lisp/find-func.el"
     (setq find-function-C-source-directory
           (let ((dir (expand-file-name "src" source-directory)))
             (if (file-accessible-directory-p dir) dir)))
     "GNU dumps no find-func.el, so each process computes the directory, against its default-directory, when find-func loads")
    (tramp-cache "net/tramp-cache.el"
     ;; tramp-cache.el's load-time read (its last form, from "Read
     ;; persistent connection history"), after the cache the build read is
     ;; cleared, as `tramp-cache-read-persistent-data' guards it.
     (when (and (stringp tramp-persistency-file-name)
                tramp-cache-read-persistent-data)
       (clrhash tramp-cache-data)
       (condition-case nil
           (with-temp-buffer
             (insert-file-contents-literally tramp-persistency-file-name)
             (let ((list (read (current-buffer)))
                   (tramp-verbose 0)
                   element key item)
               (while (setq element (pop list))
                 (setq key (pop element))
                 (when (tramp-file-name-p key)
                   (while (setq item (pop element))
                     (unless (tramp-connection-property-p key (car item))
                       (tramp-set-connection-property key (pop item) (car item)))))))
             (unless (string-equal
                      (tramp-get-connection-property
                       tramp-cache-version "tramp-version" "")
                      tramp-version)
               (signal 'file-error nil))
             (setq tramp-cache-data-changed nil))
         (file-error (clrhash tramp-cache-data))
         (error (clrhash tramp-cache-data))))
     "the persistent connection history is this user's state, which tramp-cache reads when it loads")
    (org-persist "org/org-persist.el"
     ;; org-persist.el's last form: the timer is this process's.
     (unless (and org-persist--disable-when-emacs-Q (not user-init-file))
       (when (timerp org-persist--refresh-gc-lock-timer)
         (cancel-timer org-persist--refresh-gc-lock-timer))
       (setq org-persist--refresh-gc-lock-timer
             (run-at-time nil org-persist-gc-lock-interval #'org-persist--refresh-gc-lock)))
     "the lock-refresh timer org-persist starts when it loads runs in each process"))
  "Load-time forms outside startup that read a per-process fact.
Each entry is (FEATURE FILE FORM REASON): FILE's FORM, which
`romacs-world-restart' evaluates again when FEATURE is loaded.")

(require 'romacs-world-layer)

(defun romacs-world-restart ()
  "Make a VM restored from a world what a fresh process makes at this point.
Take the session's last per-process startup steps again
\(`romacs-startup-function'), then evaluate each `romacs--world-reseed'
form whose feature is loaded, then the init layer's libraries' re-seed
definitions (`romacs--world-reseed-all')."
  (funcall romacs-startup-function)
  (dolist (entry romacs--world-reseed)
    (when (featurep (car entry))
      (eval (nth 2 entry) t)))
  (romacs--world-reseed-all))

;; The stock look is a theme: `default' (etc/themes/default-theme.el),
;; enabled once Custom and the faces are loaded, so every world built from
;; this chain starts in it and `disable-theme' gives GNU's faces.  A colour
;; scheme enabled over it replaces it (romacs-theme.el).
(require 'romacs-theme)
(romacs-theme-start)

;; The default fontsets are data for the window system's initialization
;; (`romacs--setup-fontsets'), made once here so no launch pays for them.
(romacs--make-default-fontsets)
(romacs--stage-fontsets)

;; Saved settings stay out of the user's init, in every launch of this chain.
(require 'romacs-saved-settings)

(provide 'romacs-loadup-bootstrap)
;;; romacs-loadup-bootstrap.el ends here
