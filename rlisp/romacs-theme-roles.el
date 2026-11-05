;;; romacs-theme-roles.el --- A Romacs palette as every role of Modus's table  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Modus's table (rlisp/romacs-theme-modus.el) reads about 330 names: named
;; colours and semantic mappings.  This file gives a theme all of them from
;; its own palette, with no Modus colour among them: every name is a rule
;; in the language of `romacs-theme-rules', so one reader resolves Romacs's
;; slots and Modus's roles as one namespace, and a palette row or an
;; override of any name reaches everything that reads it.  A name with no
;; rule signals rather than falling back on a core palette.
;;
;; `romacs-theme-roles-rows' ends in data, `romacs-theme-rows''s value: how
;; a theme is applied is not this file's business.

;;; Code:

(require 'romacs-theme)
(require 'romacs-theme-modus)

;;;; Roles

(defconst romacs-theme-roles--hues '(red green yellow blue magenta cyan)
  "The hues every role family is made from.")

(defconst romacs-theme-roles--warm 29
  "The Oklab hue, in degrees, of a warmer colour: red's.")

(defconst romacs-theme-roles--cool 264
  "The Oklab hue, in degrees, of a cooler colour: blue's.")

(defun romacs-theme-roles--name (format hue)
  "FORMAT with HUE, as a symbol."
  (intern (format format hue)))

(defun romacs-theme-roles--hue (hue)
  "The rules of HUE's family."
  (let ((name (lambda (format) (romacs-theme-roles--name format hue))))
    (list (cons (funcall name "%s-intense") (funcall name "bright-%s"))
          (list (funcall name "%s-faint") 'mix hue 'fg 0.25)
          (list (funcall name "%s-warmer") 'rotate hue romacs-theme-roles--warm 12)
          (list (funcall name "%s-cooler") 'rotate hue romacs-theme-roles--cool 12)
          (list (funcall name "bg-%s-intense") 'mix 'bg hue 0.5)
          (list (funcall name "bg-%s-subtle") 'mix 'bg hue 0.3)
          (list (funcall name "bg-%s-nuanced") 'mix 'bg hue 0.15)
          (cons (funcall name "bg-graph-%s-0") hue)
          (list (funcall name "bg-graph-%s-1") 'mix 'bg hue 0.55))))

(defun romacs-theme-roles--diff (kind hue)
  "The rules of diff KIND (`added', `changed', `removed'), shown in HUE."
  (let ((name (lambda (format) (romacs-theme-roles--name format kind)))
        (slot (cdr (assq kind '((added . "add") (changed . "change") (removed . "remove"))))))
    (list (cons (funcall name "bg-%s") (intern (concat "diff-" slot)))
          (list (funcall name "bg-%s-faint") 'mix 'bg hue 0.1)
          (cons (funcall name "bg-%s-refine") (intern (concat "diff-" slot "-refine")))
          (cons (funcall name "bg-%s-fringe") hue)
          (list (funcall name "fg-%s") 'mix 'fg hue 0.6)
          (cons (funcall name "fg-%s-intense") hue))))

(defconst romacs-theme-roles--terminal
  '((black 0 8) (red 1 9) (green 2 10) (yellow 3 11) (blue 4 12) (magenta 5 13) (cyan 6 14) (white 7 15))
  "Each terminal colour's normal and bright ANSI numbers.")

(defun romacs-theme-roles--term (entry)
  "The rules of the terminal colour ENTRY, from the sixteen `ansi-N' slots."
  (let ((normal (intern (format "ansi-%d" (nth 1 entry))))
        (bright (intern (format "ansi-%d" (nth 2 entry)))))
    (list (cons (intern (format "bg-term-%s" (car entry))) normal)
          (cons (intern (format "fg-term-%s" (car entry))) normal)
          (cons (intern (format "bg-term-%s-bright" (car entry))) bright)
          (cons (intern (format "fg-term-%s-bright" (car entry))) bright))))

(defconst romacs-theme-roles
  (append
   '(;; Surfaces and text.
     (bg-main . bg) (fg-main . fg) (bg-dim . bg-dark) (fg-alt . fg-light)
     (bg-active . bg-alt) (bg-inactive mix bg bg-alt 0.6)
     ;; The hues between the six.
     (rust mix red orange 0.5) (gold mix yellow orange 0.5) (olive mix green yellow 0.5)
     (slate mix blue fg-dim 0.5) (indigo mix blue magenta 0.5) (maroon mix red magenta 0.5)
     (pink mix magenta fg 0.35)
     ;; Tinted pairs.
     (bg-clay mix bg red 0.3) (fg-clay mix fg red 0.4)
     (bg-ochre mix bg orange 0.3) (fg-ochre mix fg yellow 0.4)
     (bg-lavender mix bg magenta 0.3) (fg-lavender mix fg magenta 0.4)
     (bg-sage mix bg green 0.3) (fg-sage mix fg green 0.4)
     ;; Interface.
     (bg-completion mix bg accent 0.25) (bg-popup . glass)
     (bg-hover . hover) (bg-hover-secondary mix bg accent-alt 0.25)
     (bg-hl-line . bg-highlight) (bg-paren-match . bg-paren) (bg-paren-expression mix bg magenta 0.2)
     (bg-diff-context . bg-dark)
     (bg-mode-line-active . bg-mode-line) (fg-mode-line-active . fg)
     (border-mode-line-active . border) (border-mode-line-inactive . border)
     (modeline-err . error) (modeline-warning . warning) (modeline-info . info)
     (bg-tab-bar . bg-dark) (bg-tab-current . bg) (bg-tab-other . bg-alt)
     ;; Meanings the palette's slots own.
     (fnname . function) (docstring . doc) (rx-construct . regexp) (rx-backslash . keyword)
     (err . error) (fg-link . link) (underline-link . link) (fg-prompt . prompt)
     ;; Modus's literal colours.
     (fg-table-dim . fg-dim*) (term-dim . ansi-8) (term-light . ansi-7))
   (apply #'append (mapcar #'romacs-theme-roles--hue romacs-theme-roles--hues))
   (romacs-theme-roles--diff 'added 'green)
   (romacs-theme-roles--diff 'changed 'yellow)
   (romacs-theme-roles--diff 'removed 'red)
   (apply #'append (mapcar #'romacs-theme-roles--term romacs-theme-roles--terminal)))
  "How each Modus name is made from a palette's slots, in the language of
`romacs-theme-rules'.  Together with the mappings of
`romacs-theme-modus-vivendi-palette', colour-free, they cover the table.")

(defconst romacs-theme-roles--light
  '((identifier . yellow-cooler) (underline-warning . yellow-intense) (underline-note . cyan-intense)
    (fg-active-argument . yellow-warmer) (fg-active-value . cyan-warmer)
    (accent-0 . blue) (accent-2 . cyan) (accent-3 . red)
    (fg-completion-match-0 . blue) (fg-completion-match-2 . cyan) (fg-completion-match-3 . red)
    (date-holiday . red) (date-scheduled . yellow)
    (mail-cite-0 . blue-faint) (mail-cite-1 . yellow-warmer) (mail-part . cyan)
    (fg-mark-delete . red) (fg-heading-3 . fg-alt) (fg-heading-7 . cyan-warmer))
  "The mappings a light theme has in place of the vivendi core's.")

(defconst romacs-theme-roles--names
  (append (mapcar #'car romacs-theme-modus-vivendi-palette) (mapcar #'car romacs-theme--modus-constants))
  "The names Modus's table reads.")

(defun romacs-theme-roles--mappings ()
  "The vivendi core's semantic mappings as rules: its rows that name
another entry.  Its colours stay out."
  (delq nil (mapcar (lambda (row)
                      (let ((value (romacs-theme--second row)))
                        (and (symbolp value) (cons (car row) value))))
                    romacs-theme-modus-vivendi-palette)))

(defun romacs-theme-roles--index (mode)
  "Every rule for a theme of MODE, by name: Romacs's slots over the roles,
over the light mappings when MODE is `light', over the core's mappings."
  (let ((index (make-hash-table :test 'eq)))
    (dolist (rules (list (romacs-theme-roles--mappings)
                         (and (eq mode 'light) romacs-theme-roles--light)
                         romacs-theme-roles
                         romacs-theme-rules))
      (dolist (rule rules) (puthash (car rule) (cdr rule) index)))
    index))

(defvar romacs-theme-roles--indexes nil
  "`romacs-theme-roles--index''s value per mode, (MODE . INDEX).")

(defun romacs-theme-roles-resolve (palette)
  "PALETTE with every slot and role, as an alist (NAME . VALUE): the one
effective palette a theme's faces and readers share.  Common overrides
are PALETTE's rows too, ahead of its own."
  (let* ((mode (romacs-theme--second (assq 'mode palette)))
         (index (or (cdr (assq mode romacs-theme-roles--indexes))
                    (let ((built (romacs-theme-roles--index mode)))
                      (push (cons mode built) romacs-theme-roles--indexes)
                      built)))
         (rows (append romacs-theme-common-palette-overrides romacs-theme-common-palette-user palette))
         (get (romacs-theme--reader rows (lambda (name) (gethash name index))))
         (names (make-hash-table :test 'eq))
         (resolved nil))
    (dolist (name (append romacs-theme-slots romacs-theme-roles--names (mapcar #'car rows)))
      (unless (gethash name names)
        (puthash name t names)
        (push (cons name (funcall get name)) resolved)))
    (nreverse resolved)))

;;;; Floors

(defconst romacs-theme-roles--dim
  '(shadow line-number mode-line-inactive font-lock-comment-face font-lock-comment-delimiter-face
    romacs-comment completions-annotations org-done)
  "Faces whose text is de-emphasised: floored at 3:1.")

(defconst romacs-theme-roles--dim-slots '(fg-dim fg-dim* fg-muted muted comment fg-mode-line-inactive)
  "The slots whose colour is de-emphasised wherever it shows.")

(defconst romacs-theme-roles--exempt
  '(cursor fringe scroll-bar vertical-border window-divider window-divider-first-pixel window-divider-last-pixel
    show-paren-mismatch romacs-pane-focus romacs-pane-border romacs-layer-glow romacs-landing-mark romacs-landing-word)
  "Faces whose colours are not text on a background.")

(defconst romacs-theme-roles--carrier "\\`\\(?:ansi-color-\\|term-color-\\|eat-term-color-\\|vterm-color-\\)"
  "Faces that carry a terminal's colours, which a program picks.")

(defun romacs-theme-roles--hex (value)
  "VALUE if it is a colour the floors can read."
  (and (stringp value) (= (length value) 7) (eq (aref value 0) ?#) value))

(defun romacs-theme-roles--attributes (spec)
  "The attributes face SPEC gives a graphic display: its first clause for a
display, over its `default' clause."
  (let ((shown (seq-find (lambda (clause) (not (eq (car clause) 'default))) spec))
        (base (assq 'default spec)))
    (append (and shown (romacs-theme--clause-attributes shown))
            (and base (romacs-theme--clause-attributes base)))))

(defun romacs-theme-roles--effective (face index memo)
  "FACE's foreground and background, its own else through `:inherit' among
INDEX's rows, as (FG . BG), nil where neither says.  MEMO holds each face
once."
  (or (gethash face memo)
      (let* ((attributes (gethash face index))
             (fg (romacs-theme-roles--hex (plist-get attributes :foreground)))
             (bg (romacs-theme-roles--hex (plist-get attributes :background)))
             (parents (plist-get attributes :inherit)))
        (puthash face '(nil) memo)
        (dolist (parent (if (listp parents) parents (list parents)))
          (when parent
            (let ((up (romacs-theme-roles--effective parent index memo)))
              (setq fg (or fg (car up)) bg (or bg (cdr up))))))
        (puthash face (cons fg bg) memo)
        (cons fg bg))))

(defun romacs-theme-roles--minimum (face fg dim-colours)
  "The contrast FACE's foreground FG must reach."
  (if (or (memq face romacs-theme-roles--dim) (member fg dim-colours)) 3.0 4.5))

(defun romacs-theme-roles--checked-p (face attributes)
  "Whether FACE, with ATTRIBUTES of its own, is text a floor applies to."
  (and (or (plist-get attributes :foreground) (plist-get attributes :background))
       (not (plist-get attributes :inverse-video))
       (not (memq face romacs-theme-roles--exempt))
       (not (string-match-p romacs-theme-roles--carrier (symbol-name face)))))

(defun romacs-theme-roles-floors (rows resolved)
  "The floor moves ROWS need on RESOLVED's canvas, as (FACE FG MOVED BG
MINIMUM).  Each face's foreground is read against the background it
realises on: its own, through `:inherit', else the theme's.  A face with
no foreground of its own or inherited is left: it tints text whose colour
it does not choose (`region', `hl-line').  Pairs are computed once each."
  (let ((index (make-hash-table :test 'eq)) (memo (make-hash-table :test 'eq))
        (pairs (make-hash-table :test 'equal)) (moves nil)
        (canvas (cdr (assq 'bg resolved)))
        (dim-colours (mapcar (lambda (slot) (cdr (assq slot resolved))) romacs-theme-roles--dim-slots)))
    (dolist (row rows)
      (puthash (car row) (romacs-theme-roles--attributes (romacs-theme--second row)) index))
    (dolist (row rows (nreverse moves))
      (let ((face (car row)) (attributes (gethash (car row) index)))
        (when (romacs-theme-roles--checked-p face attributes)
          (let* ((colours (romacs-theme-roles--effective face index memo))
                 (fg (car colours)) (bg (or (cdr colours) canvas)))
            (when (and fg (not (equal fg bg)))
              (let* ((minimum (romacs-theme-roles--minimum face fg dim-colours))
                     (key (list fg bg minimum))
                     (moved (or (gethash key pairs)
                                (puthash key (if (>= (romacs-color-contrast fg bg) minimum)
                                                 fg
                                               (romacs-color-floor fg bg minimum))
                                         pairs))))
                (unless (equal moved fg)
                  (push (list face fg moved bg minimum) moves))))))))))

;;;; Overrides

(defconst romacs-theme-roles--colour-attributes
  '(:foreground :background :distant-foreground :underline :overline :strike-through :box)
  "The face attributes whose values may name a palette slot.")

(defun romacs-theme-roles--value (value env)
  "VALUE of a colour attribute under ENV, (NAME . VALUE): a slot's name
reads the colour, a `romacs-theme-' form evaluates, a list maps."
  (cond ((and (symbolp value) (stringp (cdr (assq value env)))) (cdr (assq value env)))
        ((and (consp value) (symbolp (car value)) (string-prefix-p "romacs-theme-" (symbol-name (car value))))
         (eval value env))
        ((consp value) (mapcar (lambda (each) (romacs-theme-roles--value each env)) value))
        (t value)))

(defun romacs-theme-roles--override (row mode env)
  "ROW of `def-romacs-theme''s FACES as (FACE . ATTRIBUTES) for a theme of
MODE, or nil where it has no branch for MODE."
  (let ((attributes (if (memq (nth 1 row) '(&dark &light))
                        (plist-get (cdr row) (if (eq mode 'light) '&light '&dark))
                      (cdr row)))
        (out nil))
    (while attributes
      (let ((key (car attributes)) (value (nth 1 attributes)))
        (setq out (append out (list key (if (memq key romacs-theme-roles--colour-attributes)
                                            (romacs-theme-roles--value value env)
                                          value)))
              attributes (cdr (cdr attributes)))))
    (and out (cons (car row) out))))

;;;; The value

(defun romacs-theme-roles-rows (palette faces variables)
  "`romacs-theme-rows''s value for PALETTE, with `def-romacs-theme''s FACES
and VARIABLES over it.  Floors apply to the table and Romacs's rows
merged; FACES come last, on any face, and VARIABLES win."
  (let* ((mode (romacs-theme--second (assq 'mode palette)))
         (resolved (romacs-theme-roles-resolve palette))
         (romacs-theme--reading resolved)
         (bindings (seq-filter (lambda (entry) (memq (car entry) romacs-theme-roles--names)) resolved))
         (settings (romacs-theme--table-settings bindings mode))
         (ours (seq-filter (lambda (row) (romacs-theme--romacs-face-p (car row))) (romacs-theme-faces resolved)))
         (baseline (romacs-theme--merge-rows (car settings) ours))
         (floors (romacs-theme-roles-floors baseline resolved))
         (delta (append (mapcar (lambda (move) (list (car move) :foreground (nth 2 move))) floors)
                        (delq nil (mapcar (lambda (row) (romacs-theme-roles--override row mode resolved)) faces))))
         (own (mapcar (lambda (row) (list (car row) (eval (romacs-theme--second row) resolved))) variables)))
    (list :palette resolved :roles bindings
          :faces (romacs-theme--apply-delta baseline delta)
          :delta delta :floors floors
          :variables (append (seq-remove (lambda (row) (assq (car row) own)) (cdr settings)) own))))

(defun romacs-theme-uncovered (theme)
  "The faces defined now that THEME's rows reach neither directly nor
through `:inherit'."
  (let ((reached (make-hash-table :test 'eq)) (walk nil))
    (dolist (row (plist-get (romacs-theme-rows theme) :faces)) (puthash (car row) t reached))
    (setq walk (lambda (face)
                 (let ((known (gethash face reached 'unknown)))
                   (if (not (eq known 'unknown))
                       known
                     (puthash face nil reached)
                     (let ((parents (face-attribute face :inherit)))
                       (puthash face (seq-some (lambda (parent) (and (facep parent) (funcall walk parent)))
                                               (if (listp parents) parents (list parents)))
                                reached))))))
    (seq-remove walk (face-list))))

(provide 'romacs-theme-roles)
;;; romacs-theme-roles.el ends here
