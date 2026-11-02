;;; romacs-mode-line.el --- Romacs's mode line: segments as variables  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs's default `mode-line-format' on a window system,
;; `romacs-mode-line-format': GNU's format engine over a format of
;; symbols, conditionals and `%'-specs, with no `:eval' but the one a
;; region shows and the ones packages put in their own elements.  Every
;; dynamic piece is either a `%'-spec the engine computes or a segment
;; variable a hook refreshed.
;;
;; A segment is a variable, `romacs-mode-line-NAME'
;; (`romacs-mode-line-define-segment'): a `:format' segment holds a
;; mode-line construct installed verbatim, whose `%'-specs and
;; conditionals the engine expands live; a `:compute' segment holds the
;; propertized string its function last made, refreshed by the hooks it
;; names.  `romacs-mode-line-left' and `romacs-mode-line-right' list the
;; segments either side of the `right-align' marker.  Packages keep
;; setting `mode-line-format', `mode-line-process', `global-mode-string'
;; and `vc-mode' as in GNU: those symbols are elements of the format.
;; A package that computes a segment in a process or an isolate sets
;; `romacs-mode-line-NAME' from its sentinel or callback and calls
;; `force-mode-line-update'.
;;
;; `romacs-mode-line-mode' installs the format only over GNU's default or
;; its own last install, and on turning off restores GNU's only over its
;; own: a format an init or a package installed is never replaced.
;; `romacs-classic-chrome' turns it off.  DIVERGENCES.md VM2-MODE-LINE.
;;
;; The idea of segments with a left and a right list, the buffer-name
;; styles of `romacs-mode-line-buffer-style' and refreshing segments from
;; hooks rather than in the format come from doom-modeline
;; (github.com/seagle0128/doom-modeline, GPL-3.0-or-later, revision
;; 4068b067); no code is copied from it.

;;; Code:

(require 'project)

(defgroup romacs-mode-line nil
  "Romacs's mode line."
  :group 'mode-line
  :prefix "romacs-mode-line-")

(defface romacs-mode-line-dim '((t :inherit shadow))
  "Secondary text of the mode line: position, encoding.")

(defface romacs-mode-line-bright '((t :inherit mode-line-buffer-id))
  "The buffer's name in the mode line.")

(defface romacs-mode-line-state '((t :inherit (romacs-pane-focus bold)))
  "A state the mode line announces: a macro recording, overwrite, an agent.")

(defface romacs-mode-line-error '((t :inherit error)) "An error state.")
(defface romacs-mode-line-warning '((t :inherit warning)) "A warning state.")
(defface romacs-mode-line-ok '((t :inherit success)) "A good state.")

(defvar romacs-mode-line--segments (make-hash-table :test 'eq)
  "Each defined segment's name to its definition, a plist of :compute,
:hooks and :local, in definition order.")

(defvar romacs-mode-line--members (make-hash-table :test 'eq)
  "Each hook a `:compute' segment refreshes on, to the set (a hash table)
of those segments' names.")

(defvar romacs-mode-line--functions (make-hash-table :test 'eq)
  "Each hook to the adapter function this file adds to it.")

(defun romacs-mode-line--variable (name)
  "The variable of the segment NAME."
  (intern (format "romacs-mode-line-%s" name)))

(defun romacs-mode-line--adapter (hook)
  "The adapter a segment refreshed on HOOK is run by: by the hook's
arguments, `window-buffer-change-functions' passes a window or a frame,
`vc-post-command-functions' a command and its files, others nothing
the refresh reads."
  (pcase hook
    ('window-buffer-change-functions #'romacs-mode-line--on-window)
    ('vc-post-command-functions #'romacs-mode-line--on-vc-command)
    (_ #'romacs-mode-line--on-buffer)))

(defmacro romacs-mode-line-define-segment (name doc &rest spec)
  "Define the mode-line segment NAME, the variable `romacs-mode-line-NAME'.
DOC documents it.  SPEC is a plist:
  :format CONSTRUCT  a mode-line construct installed verbatim, whose
                     `%'-specs and conditionals the engine expands; or
  :compute FN        a function of no arguments run with a buffer
                     current, returning a string or nil, whose value the
                     variable holds (`romacs-mode-line-refresh-NAME');
  :hooks (HOOK ...)  with :compute, the hooks that make it stale;
  :local BOOL        with :compute, buffer-local (the default) or global;
  :face FACE         the face the segment shows in where it gives none."
  (declare (indent 1) (doc-string 2))
  (let* ((variable (romacs-mode-line--variable name))
         (face (plist-get spec :face))
         (format (plist-get spec :format))
         (compute (plist-get spec :compute))
         (local (if (plist-member spec :local) (plist-get spec :local) t)))
    (when (and format compute) (error "Segment %s has both :format and :compute" name))
    `(progn
       (defvar ,variable
         ,(and format `'(:propertize ,format ,@(and face `(face ,face)) romacs-segment ,name))
         ,doc)
       (put ',variable 'risky-local-variable t)
       ,@(when compute
           `(,@(and local `((make-variable-buffer-local ',variable)))
             (defun ,(intern (format "romacs-mode-line-refresh-%s" name)) ()
               ,(format "Compute the segment `%s' again in the current buffer." name)
               (let* ((text (funcall ,compute))
                      (value (and text (propertize text ,@(and face `('face ',face)) 'romacs-segment ',name))))
                 (unless (equal-including-properties value ,variable)
                   (setq ,variable value)
                   (force-mode-line-update))))))
       (romacs-mode-line--add-segment
        ',name (list :compute ,(and compute t) :hooks ',(plist-get spec :hooks) :local ,local))
       ',name)))

(defun romacs-mode-line--refresh (name)
  "Refresh the `:compute' segment NAME in the current buffer."
  (funcall (intern (format "romacs-mode-line-refresh-%s" name))))

(defun romacs-mode-line--refresh-on (hook)
  "Refresh the segments refreshed on HOOK in the current buffer."
  (when-let* ((names (gethash hook romacs-mode-line--members)))
    (maphash (lambda (name _) (romacs-mode-line--refresh name)) names)))

(defun romacs-mode-line--segment-hooks (definition)
  "The hooks a segment's DEFINITION refreshes it on: its own, and for a
new buffer's values `after-change-major-mode-hook'; none for `:format'."
  (and (plist-get definition :compute)
       (cons 'after-change-major-mode-hook (plist-get definition :hooks))))

(defun romacs-mode-line--refresh-everywhere (name definition)
  "Refresh the `:compute' segment NAME of DEFINITION: in every live buffer
when it is buffer-local, else once."
  (when (plist-get definition :compute)
    (if (plist-get definition :local)
        (dolist (buffer (buffer-list))
          (with-current-buffer buffer (romacs-mode-line--refresh name)))
      (romacs-mode-line--refresh name))))

(defun romacs-mode-line--add-segment (name definition)
  "Record segment NAME's DEFINITION, replacing an earlier one: its hook
memberships, and while `romacs-mode-line-mode' is on, its adapters and
its values."
  (dolist (hook (romacs-mode-line--segment-hooks (gethash name romacs-mode-line--segments)))
    (when-let* ((names (gethash hook romacs-mode-line--members))) (remhash name names)))
  (puthash name definition romacs-mode-line--segments)
  (dolist (hook (romacs-mode-line--segment-hooks definition))
    (puthash name t (or (gethash hook romacs-mode-line--members)
                        (puthash hook (make-hash-table :test 'eq) romacs-mode-line--members))))
  (when (bound-and-true-p romacs-mode-line-mode)
    (dolist (hook (romacs-mode-line--segment-hooks definition))
      (add-hook hook (romacs-mode-line--hook-function hook) 90))
    (romacs-mode-line--refresh-everywhere name definition)))

(defvar romacs-mode-line--current-hook nil
  "The hook whose adapter runs, for the adapter it dispatches from.")

(defun romacs-mode-line--on-buffer (&rest _)
  "Refresh the current buffer's segments of the hook running."
  (romacs-mode-line--refresh-on romacs-mode-line--current-hook))

(defun romacs-mode-line--on-window (window-or-frame)
  "Refresh the segments of `window-buffer-change-functions' in the
buffer of WINDOW-OR-FRAME, or of each window of a frame."
  (dolist (window (if (framep window-or-frame) (window-list window-or-frame 'never) (list window-or-frame)))
    (with-current-buffer (window-buffer window)
      (romacs-mode-line--refresh-on 'window-buffer-change-functions))))

(defun romacs-mode-line--on-vc-command (_command files _flags)
  "Refresh the segments of `vc-post-command-functions' in the live
buffers visiting FILES."
  (dolist (file (ensure-list files))
    (when-let* ((buffer (and (stringp file) (get-file-buffer file))))
      (with-current-buffer buffer
        (romacs-mode-line--refresh-on 'vc-post-command-functions)))))

(defun romacs-mode-line--hook-function (hook)
  "The function added to HOOK: its adapter, told which hook runs it."
  (or (gethash hook romacs-mode-line--functions)
      (let ((adapter (romacs-mode-line--adapter hook)))
        (puthash hook (lambda (&rest args)
                        (let ((romacs-mode-line--current-hook hook))
                          (apply adapter args)))
                 romacs-mode-line--functions))))

;;;; Segments

(defcustom romacs-mode-line-buffer-style 'auto
  "How the mode line names a file's buffer.
`auto' is `truncate-with-project' for a file in a project, else the
buffer's name; `buffer-name' its name; `file-name' its file's name;
`file-name-with-project' that after the project's; `relative-to-project'
and `relative-from-project' its path from the project's root, without
and with the project's name; `truncate-upto-project',
`truncate-from-project', `truncate-with-project' and
`truncate-except-project' its path with the directories before, after,
from or but the project's root cut to their first character;
`truncate-upto-root' and `truncate-all' with every directory but the
last, or all of them, cut; `truncate-nil' the whole path."
  :type '(choice (const auto) (const buffer-name) (const file-name) (const file-name-with-project)
                 (const relative-to-project) (const relative-from-project)
                 (const truncate-upto-project) (const truncate-from-project) (const truncate-with-project)
                 (const truncate-except-project) (const truncate-upto-root) (const truncate-all)
                 (const truncate-nil))
  :set (lambda (option value)
         (set-default-toplevel-value option value)
         (dolist (buffer (buffer-list))
           (with-current-buffer buffer
             (when (fboundp 'romacs-mode-line-refresh-buffer-name) (romacs-mode-line-refresh-buffer-name))))))

(defun romacs-mode-line--truncate (directories)
  "DIRECTORIES, a list of names, each cut to its first character."
  (mapcar (lambda (d) (if (string-empty-p d) d (substring d 0 (if (eq (aref d 0) ?.) 2 1)))) directories))

(defun romacs-mode-line--path (file root style)
  "FILE named by STYLE, ROOT its project's root or nil."
  (let* ((file (abbreviate-file-name file))
         (root (and root (abbreviate-file-name (file-name-as-directory root))))
         (name (file-name-nondirectory file))
         (project (and root (file-name-nondirectory (directory-file-name root))))
         (before (and root (split-string (directory-file-name (file-name-directory (directory-file-name root))) "/")))
         (inside (and root (butlast (split-string (file-relative-name file root) "/"))))
         (all (butlast (split-string file "/")))
         (join (lambda (&rest parts) (string-join (apply #'append parts) "/"))))
    (pcase (if root style (if (memq style '(truncate-upto-root truncate-all truncate-nil)) style 'file-name))
      ('file-name name)
      ('file-name-with-project (concat project ":" name))
      ('relative-to-project (file-relative-name file root))
      ('relative-from-project (concat project "/" (file-relative-name file root)))
      ('truncate-upto-project (funcall join (romacs-mode-line--truncate before) (list project) inside (list name)))
      ('truncate-from-project (funcall join before (list project) (romacs-mode-line--truncate inside) (list name)))
      ('truncate-with-project (funcall join (list project) (romacs-mode-line--truncate inside) (list name)))
      ('truncate-except-project (funcall join (romacs-mode-line--truncate before) (list project)
                                         (romacs-mode-line--truncate inside) (list name)))
      ('truncate-upto-root (funcall join (romacs-mode-line--truncate (butlast all)) (last all) (list name)))
      ('truncate-all (funcall join (romacs-mode-line--truncate all) (list name)))
      (_ file))))

(defun romacs-mode-line--buffer-name ()
  "The current buffer's name as `romacs-mode-line-buffer-style' gives it,
with GNU's buffer-identification face, help and keymap."
  (let* ((file buffer-file-name)
         (style romacs-mode-line-buffer-style)
         (name (if (or (not file) (memq style '(buffer-name)))
                   (buffer-name)
                 (let ((root (and (not (file-remote-p file))
                                  (when-let* ((project (project-current nil (file-name-directory file))))
                                    (project-root project)))))
                   (if (and (eq style 'auto) (not root))
                       (buffer-name)
                     (romacs-mode-line--path file root (if (eq style 'auto) 'truncate-with-project style)))))))
    ;; GNU's identification, so its keymap and help survive; the name is
    ;; a literal: a symbol's string value is never a format.
    (car (propertized-buffer-identification name))))

(defun romacs-mode-line--icon (name &optional face)
  "The icon NAME in FACE and a space, where it has one (`romacs-icon'),
else nil."
  (when-let* ((icon (and (fboundp 'romacs-icon) (romacs-icon name face))))
    (concat icon " ")))

(defvar romacs-mode-line-agent nil
  "Non-nil while an agent works for this buffer: a string to show.")
(put 'romacs-mode-line-agent 'risky-local-variable t)

(defcustom romacs-mode-line-minor-modes nil
  "Whether the mode line lists the minor modes."
  :type 'boolean)

(romacs-mode-line-define-segment front
  "One space at the start, under the status bar's accent."
  :format " ")

(romacs-mode-line-define-segment state
  "A keyboard macro recording, overwrite mode, recursive edits, an agent."
  :face romacs-mode-line-state
  :format ("" (defining-kbd-macro " REC ") (overwrite-mode " OVR ") "%[%]"
           (romacs-mode-line-agent ("" " " romacs-mode-line-agent " "))))

(romacs-mode-line-define-segment buffer-name
  "The buffer's name in `romacs-mode-line-buffer-style'."
  :compute #'romacs-mode-line--buffer-name
  :hooks (find-file-hook after-save-hook after-revert-hook after-set-visited-file-name-hook
          buffer-list-update-hook window-buffer-change-functions))

(romacs-mode-line-define-segment buffer-remote
  "A remote buffer's marker."
  :compute (lambda () (when-let* ((host (and (stringp default-directory) (file-remote-p default-directory 'host))))
                        (propertize " @" 'help-echo (format "Remote: %s" host))))
  :hooks (find-file-hook after-set-visited-file-name-hook))

(romacs-mode-line-define-segment buffer
  "The buffer: its icon, name, modified or read-only state, remote and
narrowed markers."
  :format ("" romacs-mode-line-mode-icon romacs-mode-line-buffer-name " "
           (:propertize "%*" romacs-icon ((?* . modified) (?- . saved) (?% . read-only)))
           romacs-mode-line-buffer-remote "%n"))

(romacs-mode-line-define-segment position
  "Point's line and column and how far the window is down the buffer."
  :face romacs-mode-line-dim
  :format ("  %l:%c  %p" (size-indication-mode " %I")))

(defun romacs-mode-line--selection ()
  "The active region's lines and characters."
  (when (region-active-p)
    (let ((start (region-beginning)) (end (region-end)))
      (format "  %d×%d" (count-lines start end) (- end start)))))

(romacs-mode-line-define-segment selection
  "The active region's size."
  :face romacs-mode-line-dim
  :format (mark-active (:eval (romacs-mode-line--selection))))

(defvar isearch-lazy-count-current)
(defvar isearch-lazy-count-total)

(romacs-mode-line-define-segment search
  "The current match and the matches of an incremental search."
  :compute (lambda () (and (bound-and-true-p isearch-mode)
                           (natnump isearch-lazy-count-current) (natnump isearch-lazy-count-total)
                           (> isearch-lazy-count-total 0)
                           (format "  %d/%d" isearch-lazy-count-current isearch-lazy-count-total)))
  :hooks (lazy-count-update-hook isearch-mode-end-hook))

(romacs-mode-line-define-segment input-method
  "The input method's title."
  :format (current-input-method (" " current-input-method-title)))

(romacs-mode-line-define-segment right-align
  "The point the segments after it are laid out from the right edge."
  :format " ")

(romacs-mode-line-define-segment diagnostics
  "Flymake's counters, GNU's element."
  :format (flymake-mode ("" flymake-mode-line-counters " ")))

(romacs-mode-line-define-segment lsp
  "The language server Eglot manages the buffer with."
  :compute (lambda () (when (and (bound-and-true-p eglot--managed-mode) (fboundp 'eglot-current-server))
                        (when-let* ((server (eglot-current-server)))
                          (format " %s " (if (fboundp 'eglot-project-nickname) (eglot-project-nickname server) "eglot")))))
  :hooks (eglot-managed-mode-hook))

(defun romacs-mode-line--vc-face ()
  "The face of the current buffer's version-control state."
  (pcase (and buffer-file-name (ignore-errors (vc-state buffer-file-name)))
    ((or 'up-to-date 'nil) 'romacs-mode-line-ok)
    ((or 'edited 'added) 'romacs-mode-line-warning)
    (_ 'romacs-mode-line-error)))

(romacs-mode-line-define-segment vc-icon
  "The branch icon in the version-control state's colour."
  :compute (lambda () (and (bound-and-true-p vc-mode) (romacs-mode-line--icon 'branch (romacs-mode-line--vc-face))))
  :hooks (find-file-hook after-save-hook after-revert-hook vc-checkin-hook vc-post-command-functions))

(romacs-mode-line-define-segment vc
  "The branch: its icon, then GNU's `vc-mode' as it is."
  :format (vc-mode ("" romacs-mode-line-vc-icon vc-mode " ")))

(defcustom romacs-mode-line-mode-icons
  '((emacs-lisp-mode . lisp) (lisp-mode . lisp) (lisp-data-mode . lisp) (rust-mode . rust) (rust-ts-mode . rust)
    (c-mode . c) (c-ts-mode . c) (c++-mode . c) (python-mode . python) (python-ts-mode . python)
    (sh-mode . shell) (bash-ts-mode . shell) (org-mode . org) (markdown-mode . markdown) (dired-mode . dired)
    (eshell-mode . shell) (shell-mode . shell) (term-mode . term) (romacs-term-mode . term) (eww-mode . web)
    (image-mode . image) (help-mode . help) (lisp-interaction-mode . scratch) (text-mode . text)
    (prog-mode . default) (special-mode . default))
  "Each major mode's icon, found for a mode through its parents."
  :type '(alist :key-type symbol :value-type symbol))

(defun romacs-mode-line--mode-icon-name ()
  "The icon of the current major mode or its nearest parent."
  (let ((mode major-mode) icon)
    (while (and mode (not (setq icon (alist-get mode romacs-mode-line-mode-icons))))
      (setq mode (get mode 'derived-mode-parent)))
    (or icon 'default)))

(romacs-mode-line-define-segment mode-icon
  "The major mode's icon."
  :compute (lambda () (romacs-mode-line--icon (romacs-mode-line--mode-icon-name))))

(romacs-mode-line-define-segment major-mode
  "The major mode: its icon and GNU's `mode-name', then the minor modes
while `romacs-mode-line-minor-modes'."
  :format ("" romacs-mode-line-mode-icon " " mode-name
           (romacs-mode-line-minor-modes minor-mode-alist) " "))

(romacs-mode-line-define-segment encoding
  "The buffer's coding system."
  :face romacs-mode-line-dim
  :format "%z ")

;;;; The format

(defun romacs-mode-line--set-layout (option value)
  "Set OPTION to VALUE and build the format again."
  (set-default-toplevel-value option value)
  (when (fboundp 'romacs-mode-line-rebuild) (romacs-mode-line-rebuild)))

(defcustom romacs-mode-line-left
  '(front state buffer position selection search input-method mode-line-process)
  "The mode line's segments from the left: segment names and other
mode-line constructs."
  :type '(repeat sexp)
  :set #'romacs-mode-line--set-layout)

(defcustom romacs-mode-line-right
  '(diagnostics lsp vc major-mode encoding display-time-string mode-line-misc-info)
  "The mode line's segments laid out from the right edge."
  :type '(repeat sexp)
  :set #'romacs-mode-line--set-layout)

(defvar romacs-mode-line-format nil
  "The mode-line format `romacs-mode-line-mode' installs: `%e', the
left segments, the `right-align' marker, the right segments.")
(put 'romacs-mode-line-format 'risky-local-variable t)

(defun romacs-mode-line--element (item)
  "ITEM of a segment list as a format element: a segment's variable."
  (if (and (symbolp item) (gethash item romacs-mode-line--segments))
      (romacs-mode-line--variable item)
    item))

(defvar romacs-mode-line--installed nil
  "The format `romacs-mode-line-mode' last installed.")

(defun romacs-mode-line-rebuild ()
  "Build `romacs-mode-line-format' from the segment lists, and install it
where the previous one is installed."
  (let ((old romacs-mode-line-format))
    (setq romacs-mode-line-format
          `("%e" ,@(mapcar #'romacs-mode-line--element romacs-mode-line-left)
            romacs-mode-line-right-align
            ,@(mapcar #'romacs-mode-line--element romacs-mode-line-right)))
    (when (and old (bound-and-true-p romacs-mode-line-mode) (eq old romacs-mode-line--installed))
      (romacs-mode-line--install old romacs-mode-line-format)
      (setq romacs-mode-line--installed romacs-mode-line-format))))

(defvar romacs-mode-line--gnu-default (default-value 'mode-line-format)
  "GNU's default `mode-line-format' (bindings.el), as this file found it.")

(defun romacs-mode-line--install (old new)
  "Make NEW the default `mode-line-format' where it is OLD, and the
value of each buffer whose own value is OLD."
  (when (eq (default-value 'mode-line-format) old)
    (setq-default mode-line-format new))
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (when (and (local-variable-p 'mode-line-format) (eq mode-line-format old))
        (setq mode-line-format new)))))

(defun romacs-mode-line--refresh-all ()
  "Compute every `:compute' segment again: a buffer-local one in every
live buffer, a global one once."
  (maphash #'romacs-mode-line--refresh-everywhere romacs-mode-line--segments))

(defun romacs-mode-line--hook (add)
  "Add each segment hook's adapter at depth 90 when ADD, else remove it."
  (maphash (lambda (hook names)
             (let ((function (romacs-mode-line--hook-function hook)))
               (if (and add (> (hash-table-count names) 0))
                   (add-hook hook function 90)
                 (remove-hook hook function))))
           romacs-mode-line--members))

(define-minor-mode romacs-mode-line-mode
  "Show Romacs's mode line, `romacs-mode-line-format'.
On, it becomes the default `mode-line-format' where that is GNU's or the
one this mode last installed; a format an init or a package installed
stays, and the segments are kept up to date for formats that use them.
Off, GNU's default returns where this mode's is installed."
  :global t
  :group 'romacs-mode-line
  (if romacs-mode-line-mode
      (let ((current (default-value 'mode-line-format)))
        (unless romacs-mode-line-format (romacs-mode-line-rebuild))
        (romacs-mode-line--hook t)
        (romacs-mode-line--refresh-all)
        (if (or (eq current romacs-mode-line--gnu-default)
                (and romacs-mode-line--installed (eq current romacs-mode-line--installed)))
            (progn
              (romacs-mode-line--install current romacs-mode-line-format)
              (romacs-mode-line--install romacs-mode-line--gnu-default romacs-mode-line-format)
              (setq romacs-mode-line--installed romacs-mode-line-format))
          (message "romacs-mode-line-mode: keeping the installed `mode-line-format'")))
    (romacs-mode-line--hook nil)
    (when (and romacs-mode-line--installed (eq (default-value 'mode-line-format) romacs-mode-line--installed))
      (romacs-mode-line--install romacs-mode-line--installed romacs-mode-line--gnu-default))))

(romacs-mode-line-rebuild)

(provide 'romacs-mode-line)
;;; romacs-mode-line.el ends here
