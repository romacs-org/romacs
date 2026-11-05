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

(require 'cl-lib)
(require 'project)

(defgroup romacs-mode-line nil
  "Romacs's mode line."
  :group 'mode-line
  :prefix "romacs-mode-line-")

(defface romacs-mode-line-dim '((t :inherit shadow))
  "Secondary text of the mode line: position, encoding.")

(defface romacs-mode-line-bright '((t :inherit mode-line-buffer-id))
  "The buffer's name in the mode line.")

(defface romacs-mode-line-state '((t :inherit bold))
  "A state the mode line announces: a macro recording, overwrite, an
agent, in the accent (`romacs-mode-line-pill-faces').")

(defface romacs-mode-line-state-pill '((t))
  "The status bar's state pill: the accent, with the frame's background
as its text (`romacs-mode-line-pill-faces').")

(defface romacs-mode-line-pill '((t))
  "The background of the status bar's pills: the bar's own blended
toward the frame's (`romacs-mode-line-pill-faces').")

(defvar romacs-mode-line--pill-colors nil
  "The colours `romacs-mode-line-pill-faces' last gave the pill faces.")

(defun romacs-mode-line--mix (a b fraction)
  "Colour A blended FRACTION of the way toward colour B, as #RRGGBB."
  (let ((a (color-values a)) (b (color-values b)))
    (and a b (apply #'format "#%02x%02x%02x"
                    (cl-mapcar (lambda (x y) (round (/ (+ (* (- 1 fraction) x) (* fraction y)) 257.0))) a b)))))

(defun romacs-mode-line-pill-faces (&rest _)
  "Give the pill faces the colours the theme leaves them: a pill the bar's
background blended 40% toward the frame's; the state the accent
\(`romacs-pane-focus'), and its pill the accent with the frame's
background as its text.  A face the theme or the user set keeps its
colours."
  (let* ((bar (face-background 'mode-line-active nil t))
         (frame (face-background 'default nil t))
         (accent (or (and (facep 'romacs-pane-focus) (face-foreground 'romacs-pane-focus nil t))
                     (face-foreground 'default nil t)))
         (colors (list (list 'romacs-mode-line-pill :background (romacs-mode-line--mix bar frame 0.4))
                       (list 'romacs-mode-line-state :foreground accent)
                       (list 'romacs-mode-line-state-pill :background accent :foreground frame))))
    (dolist (entry colors)
      (let ((face (car entry)) (last (assq (car entry) romacs-mode-line--pill-colors)))
        (cl-loop for (attribute value) on (cdr entry) by #'cddr
                 for current = (face-attribute face attribute nil)
                 when (and value (or (eq current 'unspecified) (equal current (plist-get (cdr last) attribute))))
                 do (set-face-attribute face nil attribute value))))
    (setq romacs-mode-line--pill-colors colors)))

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
  :face FACE         the face the segment shows in where it gives none;
  :pill FACE         in the status bar, a rounded pill on FACE's
                     background behind the segment (VM2-STATUS-BAR);
  :pill-text FACE    the segment's text on its pill in FACE's foreground;
  :priority N        where a narrow window's line drops segments to fit,
                     the lower first; a segment without one stays."
  (declare (indent 1) (doc-string 2))
  (let* ((variable (romacs-mode-line--variable name))
         (face (plist-get spec :face))
         (pill (plist-get spec :pill))
         (pill-text (plist-get spec :pill-text))
         (priority (plist-get spec :priority))
         (format (plist-get spec :format))
         (compute (plist-get spec :compute))
         (local (if (plist-member spec :local) (plist-get spec :local) t)))
    (when (and format compute) (error "Segment %s has both :format and :compute" name))
    `(progn
       (defvar ,variable
         ,(and format `'(:propertize ,format ,@(and face `(face ,face)) romacs-segment ,name ,@(and pill `(romacs-pill ,pill))
                                     ,@(and pill-text `(romacs-pill-text ,pill-text))
                                     ,@(and priority `(romacs-priority ,priority))))
         ,doc)
       (put ',variable 'risky-local-variable t)
       ,@(when compute
           `(,@(and local `((make-variable-buffer-local ',variable)))
             (defun ,(intern (format "romacs-mode-line-refresh-%s" name)) ()
               ,(format "Compute the segment `%s' again in the current buffer." name)
               (let* ((text (funcall ,compute))
                      (value (and text ,(if face `(propertize text 'face ',face) 'text))))
                 (unless (equal-including-properties value ,variable)
                   (setq ,variable value)
                   (force-mode-line-update))))))
       (romacs-mode-line--add-segment
        ',name (list :compute ,(and compute t) :hooks ',(plist-get spec :hooks) :local ,local
                     :pill ',pill :pill-text ',pill-text :priority ,priority))
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

;;;; Glyphs

(defvar romacs-mode-line--glyphs
  '((romacs-mode-line--g-record record romacs-mode-line-state)
    (romacs-mode-line--g-overwrite overwrite romacs-mode-line-state)
    (romacs-mode-line--g-agent agent romacs-mode-line-state)
    (romacs-mode-line--g-narrow narrow romacs-mode-line-dim)
    (romacs-mode-line--g-region region romacs-mode-line-dim)
    (romacs-mode-line--g-search search romacs-mode-line-dim)
    (romacs-mode-line--g-keyboard keyboard romacs-mode-line-dim))
  "Each glyph variable the format shows, its icon and face; set by
`romacs-mode-line--refresh-glyphs' where the display can draw them.")

(dolist (glyph romacs-mode-line--glyphs)
  (eval `(defvar ,(car glyph) nil) t)
  (put (car glyph) 'risky-local-variable t))

(defun romacs-mode-line--refresh-glyphs (&rest _)
  "Give each glyph variable its icon as this display draws it."
  (dolist (glyph romacs-mode-line--glyphs)
    (set (car glyph) (or (and (fboundp 'romacs-icon) (romacs-icon (nth 1 glyph) (nth 2 glyph))) ""))))

;;;; Segments
;;
;; What each segment shows and why it earns its place: modeline.md's
;; segment table.

(romacs-mode-line-define-segment front
  "One space at the start, after the accent."
  :format " ")

(romacs-mode-line-define-segment state
  "A keyboard macro recording, overwrite mode, recursive edits, an agent."
  :face romacs-mode-line-state
  :pill romacs-mode-line-state-pill
  :pill-text romacs-mode-line-state-pill
  :format ("" (defining-kbd-macro ("" " " romacs-mode-line--g-record " "))
           (overwrite-mode ("" " " romacs-mode-line--g-overwrite " ")) "%[%]"
           (romacs-mode-line-agent ("" " " romacs-mode-line--g-agent " " romacs-mode-line-agent " "))))

(romacs-mode-line-define-segment buffer-name
  "The buffer's name in `romacs-mode-line-buffer-style'."
  :compute #'romacs-mode-line--buffer-name
  :hooks (find-file-hook after-save-hook after-revert-hook after-set-visited-file-name-hook
          buffer-list-update-hook window-buffer-change-functions))

(romacs-mode-line-define-segment buffer-remote
  "A remote buffer's glyph, its host in the help."
  :compute (lambda () (when-let* ((host (and (stringp default-directory) (file-remote-p default-directory 'host))))
                        (concat " " (propertize (or (romacs-mode-line--glyph 'remote) "@") 'help-echo (format "Remote: %s" host)))))
  :hooks (find-file-hook after-set-visited-file-name-hook))

(romacs-mode-line-define-segment buffer
  "The buffer: its mode's icon, its name, its remote and narrowed
glyphs, and its modified or read-only glyph (none while unmodified)."
  :format (" " romacs-mode-line-mode-icon romacs-mode-line-buffer-name romacs-mode-line-buffer-remote
           (:propertize "%n" romacs-icon ((?N . narrow) (?a . none) (?r . none) (?o . none) (?w . none)))
           (:propertize " %*" romacs-icon ((?* . modified) (?- . none) (?% . read-only)))))

(romacs-mode-line-define-segment position
  "Point's line and column, and how far down the window is unless the
whole buffer shows (GNU's `All' is dropped)."
  :face romacs-mode-line-dim
  :priority 80
  :format (" %l:%c" (:propertize " %p" romacs-icon ((?A . none) (?l . none)))
           (size-indication-mode " %I")))

(defun romacs-mode-line--selection ()
  "The active region's lines and characters."
  (when (region-active-p)
    (let ((start (region-beginning)) (end (region-end)))
      (format "%d×%d" (count-lines start end) (- end start)))))

(romacs-mode-line-define-segment selection
  "The active region's size."
  :face romacs-mode-line-dim
  :priority 75
  :format (mark-active ("" "  " romacs-mode-line--g-region " " (:eval (romacs-mode-line--selection)))))

(defvar isearch-lazy-count-current)
(defvar isearch-lazy-count-total)

(romacs-mode-line-define-segment search
  "The current match and the matches of an incremental search."
  :priority 90
  :compute (lambda () (and (bound-and-true-p isearch-mode)
                           (natnump isearch-lazy-count-current) (natnump isearch-lazy-count-total)
                           (> isearch-lazy-count-total 0)
                           (concat "  " (and (> (length romacs-mode-line--g-search) 0) (concat romacs-mode-line--g-search " "))
                                   (format "%d/%d" isearch-lazy-count-current isearch-lazy-count-total))))
  :hooks (lazy-count-update-hook isearch-mode-end-hook))

(romacs-mode-line-define-segment input-method
  "The input method's title."
  :priority 60
  :format (current-input-method ("" "  " romacs-mode-line--g-keyboard " " current-input-method-title)))

(romacs-mode-line-define-segment right-align
  "The point the segments after it are laid out from the right edge."
  :format " ")

(defvar romacs-mode-line-repeat nil
  "The keys a pending repeat takes, as the status bar shows them, or nil:
set by `romacs-popup-repeat-echo' where hints do not show them.")

(defun romacs-mode-line-repeat-text (map)
  "MAP's keys after the keyboard glyph, as the repeat segment shows them."
  (let (keys)
    (map-keymap (lambda (event binding) (when (and binding (not (consp event))) (push (key-description (vector event)) keys))) map)
    (concat (or (romacs-mode-line--icon 'keyboard) "") (string-join (nreverse keys) " ") " ")))

(romacs-mode-line-define-segment repeat
  "The keys a pending repeat takes, where hints do not show them."
  :priority 60
  :format ("" romacs-mode-line-repeat))

;; What packages add to `mode-line-modes': GNU shows it in every mode
;; line, `[Compiling]' while a compilation runs, erc-track's channel
;; activity, repeat's `repeat-echo-mode-line'; its own elements (the
;; major mode's name, the process, the minor modes' lighters, the
;; narrowing and recursive-edit specs) other segments show or leave out.

(defun romacs-mode-line--gnu-modes-element-p (element)
  "Non-nil for an element of GNU's own `mode-line-modes' (bindings.el),
told by its content: a package's entry may be added before this file
loads (compile.el adds its own as it loads)."
  (pcase element
    ((pred stringp) (member (substring-no-properties element) '("%[" "%]" "%n" "(" ")" " " "")))
    (`(:eval (,(or 'car 'cdr) mode-line-modes-delimiters)) t)
    (`(:propertize ("" mode-name) . ,_) t)
    (`("" ,(or 'mode-line-process 'mode-line-minor-modes 'minor-mode-alist)) t)))

(defun romacs-mode-line--modes-extra (modes)
  "The elements of MODES, a `mode-line-modes' value, packages added."
  (seq-remove #'romacs-mode-line--gnu-modes-element-p (if (listp modes) modes (list modes))))

(defvar romacs-mode-line-modes-extra nil
  "What packages added to `mode-line-modes', as the `modes' segment shows
it: kept by `romacs-mode-line--modes-watch' as `mode-line-modes' is set.")
(put 'romacs-mode-line-modes-extra 'risky-local-variable t)

(defun romacs-mode-line--modes-watch (_symbol value operation where)
  "`mode-line-modes' is about to be VALUE by OPERATION, in buffer WHERE
when that is its local value: keep `romacs-mode-line-modes-extra' to
match.  A local value killed (`makunbound' in WHERE) takes the default's
again."
  (cond ((and (buffer-live-p where) (eq operation 'makunbound))
         (with-current-buffer where (kill-local-variable 'romacs-mode-line-modes-extra)))
        ((buffer-live-p where)
         (with-current-buffer where
           (setq-local romacs-mode-line-modes-extra (romacs-mode-line--modes-extra value))))
        (t (set-default 'romacs-mode-line-modes-extra
                        (and (not (eq operation 'makunbound)) (romacs-mode-line--modes-extra value))))))

(defun romacs-mode-line--modes-reconcile ()
  "Make `romacs-mode-line-modes-extra' match `mode-line-modes' everywhere:
its default, and each buffer's own value where it has one, once, as the
watcher starts."
  (set-default 'romacs-mode-line-modes-extra (romacs-mode-line--modes-extra (default-value 'mode-line-modes)))
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (cond ((local-variable-p 'mode-line-modes)
             (setq-local romacs-mode-line-modes-extra (romacs-mode-line--modes-extra mode-line-modes)))
            ((local-variable-p 'romacs-mode-line-modes-extra)
             (kill-local-variable 'romacs-mode-line-modes-extra))))))

(romacs-mode-line-define-segment modes
  "What packages add to `mode-line-modes': a compilation in progress,
chat activity."
  :priority 45
  :format ("" romacs-mode-line-modes-extra))

(romacs-mode-line-define-segment misc
  "What packages and the user add (`mode-line-misc-info', with
`global-mode-string': the time, a clock, a timer)."
  :priority 40
  :format ("" mode-line-misc-info))

(defface romacs-mode-line-info '((t :inherit success)) "An informational count.")

(defun romacs-mode-line--diagnostics ()
  "Flymake's error, warning and note counts after their glyphs, as
doom-modeline's `check' segment shows them; nil without flymake."
  (when (and (bound-and-true-p flymake-mode) (fboundp 'flymake-diagnostics))
    (let ((counts (list 0 0 0)))
      (dolist (diagnostic (flymake-diagnostics))
        (let ((level (flymake--severity (flymake-diagnostic-type diagnostic))))
          (cl-incf (nth (cond ((>= level (flymake--severity :error)) 0)
                              ((>= level (flymake--severity :warning)) 1)
                              (t 2))
                        counts))))
      (concat " "
              (mapconcat (lambda (entry)
                           (pcase-let ((`(,icon ,face ,count) entry))
                             (concat (or (romacs-mode-line--glyph icon face) "")
                                     (propertize (number-to-string count) 'face face))))
                         (cl-mapcar #'list '(error warning note)
                                    '(romacs-mode-line-error romacs-mode-line-warning romacs-mode-line-info)
                                    counts)
                         "")
              " "))))

(defun romacs-mode-line--after-flymake-report (&rest _)
  "Refresh the diagnostics of the current buffer after flymake reports."
  (when (fboundp 'romacs-mode-line-refresh-diagnostics) (romacs-mode-line-refresh-diagnostics)))

(with-eval-after-load 'flymake
  (advice-add 'flymake--handle-report :after #'romacs-mode-line--after-flymake-report))

(romacs-mode-line-define-segment diagnostics
  "Flymake's error, warning and note counts, each after its glyph."
  :priority 70
  :compute #'romacs-mode-line--diagnostics
  :hooks (flymake-mode-hook))

(romacs-mode-line-define-segment lsp
  "The language server's glyph, its name in the help."
  :priority 30
  :compute (lambda () (when (and (bound-and-true-p eglot--managed-mode) (fboundp 'eglot-current-server))
                        (when-let* ((server (eglot-current-server)))
                          (concat " " (propertize (or (romacs-mode-line--glyph 'lsp) "LSP")
                                                  'help-echo (format "Eglot: %s" (if (fboundp 'eglot-project-nickname)
                                                                                    (eglot-project-nickname server) "eglot")))
                                  " "))))
  :hooks (eglot-managed-mode-hook))

(defun romacs-mode-line--glyph (name &optional face)
  "The icon NAME in FACE where it has one, else nil."
  (and (fboundp 'romacs-icon) (romacs-icon name face)))

(defun romacs-mode-line--vc ()
  "The branch, as doom-modeline shows it: the state's glyph in its colour,
then `vc-mode''s text without the backend's name, with its help and map."
  (when (and (bound-and-true-p vc-mode) (stringp vc-mode))
    (let* ((state (and buffer-file-name (ignore-errors (vc-state buffer-file-name))))
           (glyph (pcase state
                    ((or 'up-to-date 'nil) (romacs-mode-line--glyph 'branch 'romacs-mode-line-ok))
                    ((or 'edited 'added) (romacs-mode-line--glyph 'vc-edited 'romacs-mode-line-warning))
                    (_ (romacs-mode-line--glyph 'vc-conflict 'romacs-mode-line-error))))
           (text (string-trim vc-mode))
           (branch (if (string-match "\\`[A-Za-z]+[-:@!?]" text) (substring text (match-end 0)) text)))
      (concat (and glyph (concat glyph " ")) branch " "))))

(romacs-mode-line-define-segment vc
  "The branch: the version-control state's glyph and the branch's name."
  :priority 60
  :compute #'romacs-mode-line--vc
  :hooks (find-file-hook after-save-hook after-revert-hook vc-checkin-hook vc-post-command-functions))

(defcustom romacs-mode-line-mode-icons
  '((emacs-lisp-mode . lisp) (lisp-mode . lisp) (lisp-data-mode . lisp) (rust-mode . rust) (rust-ts-mode . rust)
    (c-mode . c) (c-ts-mode . c) (c++-mode . c) (c++-ts-mode . c) (python-mode . python) (python-ts-mode . python)
    (js-mode . javascript) (js-ts-mode . javascript) (typescript-ts-mode . typescript) (tsx-ts-mode . typescript)
    (go-mode . go) (go-ts-mode . go) (java-mode . java) (java-ts-mode . java) (nix-mode . nix) (nix-ts-mode . nix)
    (haskell-mode . haskell) (json-mode . json) (json-ts-mode . json) (js-json-mode . json)
    (html-mode . html) (mhtml-mode . html) (css-mode . css) (css-ts-mode . css) (lua-mode . lua) (lua-ts-mode . lua)
    (conf-mode . conf) (yaml-ts-mode . data) (toml-ts-mode . conf) (sql-mode . database)
    (sh-mode . shell) (bash-ts-mode . shell) (org-mode . org) (org-agenda-mode . calendar) (calendar-mode . calendar)
    (markdown-mode . markdown) (gfm-mode . markdown) (dired-mode . dired)
    (eshell-mode . shell) (shell-mode . shell) (term-mode . term) (romacs-term-mode . term) (eww-mode . web)
    (image-mode . image) (help-mode . help) (helpful-mode . help) (Info-mode . info) (Man-mode . info)
    (message-mode . mail) (notmuch-search-mode . mail) (notmuch-show-mode . mail) (notmuch-tree-mode . mail)
    (jabber-chat-mode . chat) (erc-mode . chat) (rcirc-mode . chat)
    (lisp-interaction-mode . scratch) (text-mode . text))
  "Each major mode's icon, found for a mode through its parents.  A mode
with none (fundamental, special and dashboard buffers, a language with
no icon of its own) shows Romacs's R, which says only that this is
Romacs, so its name shows too: an icon earns its place by naming the mode."
  :type '(alist :key-type symbol :value-type symbol))

(defvar-local romacs-mode-line--iconic nil
  "Non-nil when the major mode's icon says what the mode is, so its name
need not show.")

(defcustom romacs-mode-line-telling-icons
  '(rust python go java nix haskell json html css lua javascript typescript markdown org dired)
  "The mode icons that name their mode unambiguously: a mode showing one
of them does not also show its name."
  :type '(repeat symbol))

(defun romacs-mode-line--mode-icon-name ()
  "The icon of the current major mode or its nearest parent."
  (let ((mode major-mode) icon)
    (while (and mode (not (setq icon (alist-get mode romacs-mode-line-mode-icons))))
      (setq mode (get mode 'derived-mode-parent)))
    (or icon 'romacs)))

(romacs-mode-line-define-segment mode-icon
  "The major mode's icon."
  :hooks (window-buffer-change-functions)
  :compute (lambda () (let* ((name (romacs-mode-line--mode-icon-name))
                             ;; Romacs's R in the mark's colour where it shows.
                             (icon (romacs-mode-line--icon name (and (eq name 'romacs) 'romacs-mode-line-mark))))
                        (setq romacs-mode-line--iconic (and icon (memq name romacs-mode-line-telling-icons) t))
                        icon)))

(romacs-mode-line-define-segment major-mode
  "The major mode's name where its icon does not say it (GNU's
`mode-name'), and the minor modes while `romacs-mode-line-minor-modes'."
  :priority 50
  :format ("" (romacs-mode-line--iconic "" (" " mode-name))
           (romacs-mode-line-minor-modes (" " minor-mode-alist)) " "))

(defcustom romacs-mode-line-default-coding 'utf-8-unix
  "The coding system the mode line does not mention: a buffer in any
other shows its coding and end of line."
  :type 'coding-system)

(defun romacs-mode-line--encoding ()
  "The buffer's end of line and coding, as doom-modeline shows them
(`LF UTF-8'), unless they are `romacs-mode-line-default-coding''s or
nothing was decided."
  (let* ((coding buffer-file-coding-system)
         (base (and coding (coding-system-base coding)))
         (eol (and coding (coding-system-eol-type coding))))
    (unless (or (null coding) (memq base '(undecided no-conversion prefer-utf-8))
                (and (eq base (coding-system-base romacs-mode-line-default-coding))
                     (memq eol (list 0 (coding-system-eol-type romacs-mode-line-default-coding)))))
      (concat (pcase eol (1 "CRLF ") (2 "CR ") (_ "LF "))
              (upcase (symbol-name (or (coding-system-get base :mime-charset) base))) " "))))

(romacs-mode-line-define-segment encoding
  "The coding system and end of line, where they are not the default."
  :face romacs-mode-line-dim
  :priority 20
  :compute #'romacs-mode-line--encoding
  :hooks (find-file-hook after-save-hook after-revert-hook after-set-visited-file-name-hook))

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
  '(repeat modes misc lsp encoding major-mode vc diagnostics)
  "The mode line's segments laid out from the right edge."
  :type '(repeat sexp)
  :set #'romacs-mode-line--set-layout)

(defvar romacs-mode-line-format nil
  "The mode-line format `romacs-mode-line-mode' installs: `%e', the left
segments, the `right-align' marker, the right segments.")
(put 'romacs-mode-line-format 'risky-local-variable t)

(defun romacs-mode-line--element (item)
  "ITEM of a segment list as a format element: a segment's variable, a
`:compute' one in the `:propertize' that names it its segment (and pill),
as a `:format' one's own value does."
  (let ((definition (and (symbolp item) (gethash item romacs-mode-line--segments))))
    (cond ((not definition) item)
          ((plist-get definition :compute)
           `(:propertize ("" ,(romacs-mode-line--variable item)) romacs-segment ,item
                         ,@(when-let* ((pill (plist-get definition :pill))) `(romacs-pill ,pill))
                         ,@(when-let* ((text (plist-get definition :pill-text))) `(romacs-pill-text ,text))
                         ,@(when-let* ((priority (plist-get definition :priority))) `(romacs-priority ,priority))))
          (t (romacs-mode-line--variable item)))))

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
        (romacs-mode-line-pill-faces)
        (romacs-mode-line--refresh-glyphs)
        (unless romacs-mode-line-format (romacs-mode-line-rebuild))
        (romacs-mode-line--hook t)
        (romacs-mode-line--refresh-all)
        (romacs-mode-line--modes-reconcile)
        (add-variable-watcher 'mode-line-modes #'romacs-mode-line--modes-watch)
        (if (or (eq current romacs-mode-line--gnu-default)
                (and romacs-mode-line--installed (eq current romacs-mode-line--installed)))
            (progn
              (romacs-mode-line--install current romacs-mode-line-format)
              (romacs-mode-line--install romacs-mode-line--gnu-default romacs-mode-line-format)
              (setq romacs-mode-line--installed romacs-mode-line-format))
          (message "romacs-mode-line-mode: keeping the installed `mode-line-format'")))
    (romacs-mode-line--hook nil)
    (remove-variable-watcher 'mode-line-modes #'romacs-mode-line--modes-watch)
    (when (and romacs-mode-line--installed (eq (default-value 'mode-line-format) romacs-mode-line--installed))
      (romacs-mode-line--install romacs-mode-line--installed romacs-mode-line--gnu-default))))

(romacs-mode-line-rebuild)
(add-hook 'enable-theme-functions #'romacs-mode-line-pill-faces)

(provide 'romacs-mode-line)
;;; romacs-mode-line.el ends here
