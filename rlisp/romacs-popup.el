;;; romacs-popup.el --- Romacs's floating menus: one look, motion and keyboard model  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs's menus are layers (romacs-layer.el) of four kinds, each a
;; placement and a look from one table, `romacs-popup-look':
;;
;; `panel'    a key menu across the frame's bottom, above the status bar
;;            (which-key, keymap-popup);
;; `cursor'   a chip or short menu after a buffer position (eval results);
;; `palette'  a centred prompt and list (romacs-complete's panel);
;; `hints'    the keys of an active transient map.
;;
;; A menu never takes focus or selects a window: keys reach the command
;; loop as in GNU, through the map its caller activated.  Dismissing a
;; menu hides it; deactivating a map is its caller's.  The Lisp APIs
;; packages call stay GNU's; only where and how a menu shows is Romacs's
;; (DIVERGENCES.md VM2-MENU-LAYERS).  `romacs-popup-key-grid' lays out
;; groups of keys for a panel.

;;; Code:

(eval-when-compile (require 'cl-lib))
(require 'seq)
(require 'subr-x)
(require 'romacs-layer)

(defgroup romacs-popup nil
  "Romacs's floating menus."
  :group 'romacs-layer)

(defface romacs-popup '((t :inherit romacs-layer-glass))
  "A menu's text and background.")

(defface romacs-popup-key '((t :inherit romacs-layer-glow))
  "A key in a menu.")

(defface romacs-popup-title '((t :inherit bold))
  "A menu's title and its groups' titles.")

(defface romacs-popup-description '((t))
  "What a key in a menu does.")

(defface romacs-popup-dim '((t :inherit shadow))
  "A menu's secondary text.")

(defconst romacs-popup-look
  '((panel :radius 8 :padding 10 :motion slide :edge bottom)
    (palette :radius 8 :padding 10 :motion slide :edge top)
    (cursor :radius 6 :padding 4 :motion fade :edge top)
    (hints :radius 6 :padding 4 :motion fade :edge bottom))
  "Each menu kind's look: corner radius and padding in pixels, and its
motion: `slide' fades in sliding from its EDGE, `fade' only fades.")

(defvar romacs-popup--panels (make-hash-table :test 'eq)
  "BUFFER -> t for buffers shown as `panel' menus: while one shows, no
hints do.  A panel's hiding or its buffer's death removes it.")

(defvar romacs-popup-panel-functions nil
  "Functions called with no argument after a `panel' menu shows or hides,
so what depends on one being shown (transient-map hints) can follow.")

(defun romacs-popup--parameters (kind face)
  "The frame parameters of a KIND menu in FACE: its glass, or GNU's square
child frame while `romacs-layer-glass' is nil, and its motion."
  (let ((look (alist-get kind romacs-popup-look)))
    `(,@(romacs-layer-glass-parameters romacs-layer-glass face
                                       (plist-get look :padding) (plist-get look :radius))
      (romacs-layer-motion . ,(plist-get look :motion))
      (romacs-popup-kind . ,kind))))

(cl-defun romacs-popup-show (kind buffer &key anchor columns lines (face 'romacs-popup))
  "Show BUFFER as a KIND menu (see `romacs-popup-look') and return its
frame, or nil when it cannot show.  ANCHOR is a `cursor' menu's
\(WINDOW . POSITION), a `hints' menu's window (the selected one).
COLUMNS and LINES size it, by default to BUFFER's text; FACE paints its
background."
  (let* ((buffer (window-normalize-buffer buffer))
         (frame (romacs-layer--frame buffer)))
    (modify-frame-parameters frame (romacs-popup--parameters kind face))
    (set-window-hscroll (frame-root-window frame) 0)
    (prog1 (romacs-layer-show buffer
                              :position (pcase kind
                                          ('panel 'bottom)
                                          ('palette 'center)
                                          ('cursor `(at ,(car anchor) ,(cdr anchor)))
                                          ('hints `(window-bottom ,(or anchor (selected-window))))
                                          (_ (error "Unknown menu kind %S" kind)))
                              :columns columns :lines lines
                              ;; A key menu is modal and may be taller than a
                              ;; list: the frame less a few lines of its windows.
                              :max-lines (and (eq kind 'panel)
                                              (max romacs-layer-max-lines
                                                   (- (frame-text-lines (romacs-layer--root-frame)) 4))))
      (when (eq kind 'panel)
        (unless (gethash buffer romacs-popup--panels)
          (puthash buffer t romacs-popup--panels)
          (with-current-buffer buffer (add-hook 'kill-buffer-hook #'romacs-popup--panel-killed nil t)))
        (run-hooks 'romacs-popup-panel-functions)))))

(defun romacs-popup--panel-killed ()
  "Forget the current buffer's panel, as the buffer is killed, and tell
`romacs-popup-panel-functions' once its layer is gone."
  (remhash (current-buffer) romacs-popup--panels)
  (run-at-time 0 nil #'run-hooks 'romacs-popup-panel-functions))

(put 'romacs-popup--panel-killed 'permanent-local-hook t)

(defun romacs-popup-hide (buffer)
  "Hide the menu showing BUFFER, which fades; its frame is kept."
  (let ((panel (and (get-buffer buffer) (romacs-popup-panel-p (get-buffer buffer)))))
    (romacs-layer-hide buffer)
    (when panel
      (remhash (get-buffer buffer) romacs-popup--panels)
      (run-hooks 'romacs-popup-panel-functions))))

(defun romacs-popup--panel-shown-p ()
  "Non-nil while some `panel' menu shows: the first visible one ends the
walk over the registered ones."
  (catch 'shown
    (maphash (lambda (buffer _) (when (romacs-popup-panel-p buffer) (throw 'shown t))) romacs-popup--panels)))

(defun romacs-popup-panel-p (buffer)
  "Non-nil while BUFFER shows as a visible `panel' menu."
  (let ((frame (romacs-layer--frame-of buffer)))
    (and frame (frame-visible-p frame) (eq (frame-parameter frame 'romacs-popup-kind) 'panel))))

;;;; Key grids

(defun romacs-popup--block (group)
  "GROUP, (TITLE . ((KEY . DESCRIPTION) ...)), as lines in the menu faces.
TITLE nil has no title line."
  (let* ((entries (cdr group))
         (key-width (seq-reduce (lambda (w e) (max w (string-width (car e)))) entries 0)))
    (append (and (car group) (list (propertize (car group) 'face 'romacs-popup-title)))
            (mapcar (lambda (entry)
                      (concat (propertize (car entry) 'face 'romacs-popup-key)
                              (make-string (- (+ 2 key-width) (string-width (car entry))) ?\s)
                              (propertize (cdr entry) 'face 'romacs-popup-description)))
                    entries))))

(defun romacs-popup-key-grid (groups width &optional gap)
  "GROUPS laid out side by side in bands at most WIDTH columns wide, as a
string: each group, (TITLE . ((KEY . DESCRIPTION) ...)), a block of its
title and its keys aligned on its widest key, GAP columns (default 3)
apart; a block that would pass WIDTH starts the next band.  Linear in
the entries."
  (let ((gap (or gap 3)) bands band (used 0))
    (dolist (lines (mapcar #'romacs-popup--block groups))
      (let ((w (seq-reduce (lambda (w line) (max w (string-width line))) lines 1)))
        (when (and band (> (+ used gap w) width))
          (push (nreverse band) bands)
          (setq band nil used 0))
        (setq used (if band (+ used gap w) w))
        ;; Each block's lines as a vector, so a row is one `aref'.
        (push (cons w (vconcat lines)) band)))
    (when band (push (nreverse band) bands))
    (mapconcat
     (lambda (band)
       (let ((height (seq-reduce (lambda (h b) (max h (length (cdr b)))) band 0)) rows)
         (dotimes (i height)
           (push (string-trim-right
                  (mapconcat (lambda (b)
                               (let ((line (if (< i (length (cdr b))) (aref (cdr b) i) "")))
                                 (concat line (make-string (- (car b) (string-width line)) ?\s))))
                             band (make-string gap ?\s)))
                 rows))
         (string-join (nreverse rows) "\n")))
     (nreverse bands) "\n\n")))

;;;; Hints: the keys of an active transient map

(defcustom romacs-popup-hints-delay 0.4
  "Idle seconds before an active transient map's keys show as hints."
  :type 'number)

(defcustom romacs-popup-hints-ignore '(universal-argument-map minibuffer-local-map)
  "Variables whose keymaps never show as hints."
  :type '(repeat symbol))

(defcustom romacs-popup-hints-strip '("windmove-" "text-scale-")
  "Prefixes dropped from a command's name in a hint."
  :type '(repeat string))

(defvar romacs-popup-hints-inhibit nil
  "Non-nil around a `set-transient-map' whose keys another menu shows.")

(defvar romacs-popup--hint-maps (make-hash-table :test 'eq :weakness 'key)
  "MAP -> its `set-transient-map' MESSAGE text, for every map activated.")


(defvar romacs-popup--hints-shown nil "The map whose hints show.")
(defvar romacs-popup--hints-place nil "Where they show: `romacs-popup--hints-place'.")
(defvar romacs-popup--hints-pending nil "The map a timer will show, and the timer.")
(defvar romacs-popup--hints-scheduled nil "A reconcile is scheduled.")

(defun romacs-popup--hints-buffer ()
  (or (get-buffer " *romacs-hints*")
      (with-current-buffer (get-buffer-create " *romacs-hints*")
        (setq-local mode-line-format nil header-line-format nil cursor-type nil truncate-lines t)
        (current-buffer))))

(defun romacs-popup--hint-target ()
  "The innermost registered map of `overriding-terminal-local-map' that
may show hints, or nil: none while a `panel' shows, an ignored map, or a
map activated with hints inhibited.  `internal-push-keymap' composes
\(keymap INNERMOST ... add-keymap-witness . OUTER); another map there
\(isearch's, set directly) has none."
  (when (and (bound-and-true-p romacs-popup-hints-mode)
             (> (hash-table-count romacs-popup--hint-maps) 0)
             (not (romacs-popup--panel-shown-p)))
    (let ((tail (cdr-safe overriding-terminal-local-map)) found)
      (while (and (consp tail) (not found) (keymapp (car tail)))
        (let ((map (car tail)))
          (when (and (gethash map romacs-popup--hint-maps)
                     (not (seq-some (lambda (v) (and (boundp v) (eq (symbol-value v) map))) romacs-popup-hints-ignore)))
            (setq found map)))
        (setq tail (cdr tail)))
      found)))

(defun romacs-popup--hint-label (binding)
  "What a hint says BINDING does, or nil to leave it out."
  (cond ((null binding) nil)
        ((keymapp binding) "+…")
        ((and (consp binding) (eq (car binding) 'menu-item)) (and (stringp (nth 1 binding)) (nth 1 binding)))
        ((symbolp binding)
         (let ((name (symbol-name binding)))
           (dolist (prefix romacs-popup-hints-strip name)
             (when (string-prefix-p prefix name) (setq name (substring name (length prefix)))))))
        ((functionp binding)
         (let ((doc (ignore-errors (documentation binding t))))
           (if (and doc (not (string-empty-p doc))) (car (split-string doc "\n")) "…")))))

(defun romacs-popup--hint-fragments (items)
  "ITEMS, character intervals (INDEX LO HI BINDING) in traversal order, as
the parts of each that no earlier one covers, (INDEX LO HI BINDING) in
ascending LO.  Each interval paints the units between the sorted bounds
that no earlier one painted, a union-find skipping painted units: N
intervals in O(N log N), whatever ranges and singletons mix."
  (let* ((sorted (sort (mapcan (lambda (item) (list (nth 1 item) (1+ (nth 2 item)))) items) #'<))
         (bounds (vconcat (let (unique) (dolist (b sorted (nreverse unique)) (unless (eql b (car unique)) (push b unique))))))
         (m (length bounds))
         (position (make-hash-table :test 'eql :size m))
         ;; NEXT[K]: the first unpainted unit from K; unit K is
         ;; [BOUNDS[K], BOUNDS[K+1]), the last bound only an end.
         (next (make-vector m 0))
         (painted (make-vector m nil))
         (find (lambda (k)
                 (while (/= (aref next k) k)
                   (aset next k (aref next (aref next k)))
                   (setq k (aref next k)))
                 k))
         fragments)
    (dotimes (k m) (puthash (aref bounds k) k position) (aset next k k))
    (dolist (item items)
      (let ((k (funcall find (gethash (nth 1 item) position)))
            (end (gethash (1+ (nth 2 item)) position)))
        (while (< k end)
          (aset painted k item)
          (aset next k (1+ k))
          (setq k (funcall find (1+ k))))))
    (let ((k 0))
      (while (< k (1- m))
        (let ((item (aref painted k)) (start k))
          (while (and (< (1+ k) (1- m)) (eq (aref painted (1+ k)) item)) (setq k (1+ k)))
          (when item (push (list (car item) (aref bounds start) (1- (aref bounds (1+ k))) (nth 3 item)) fragments))
          (setq k (1+ k)))))
    (nreverse fragments)))

(defun romacs-popup--hint-entries (map)
  "MAP's bindings as (KEY . LABEL), in one traversal: what a key is bound
to first (a map's own before its parents') is its effective binding, a
nil one included, for a character range as for a single key, so a range
shows split around the keys bound before it; remaps, defaults, menu and
mouse events are left out.  In traversal order."
  (let ((seen (make-hash-table :test 'eq)) (n 0) characters others)
    (map-keymap
     (lambda (event binding)
       (setq n (1+ n))
       (cond
        ((consp event) (push (list n (car event) (cdr event) binding) characters))
        ((integerp event) (push (list n event event binding) characters))
        ((or (memq event '(remap t menu-bar tool-bar)) (string-match-p "mouse" (symbol-name event))))
        ((gethash event seen))
        (t (puthash event t seen) (push (list n (key-description (vector event)) binding) others))))
     map)
    (let ((keys (nconc (mapcar (pcase-lambda (`(,i ,lo ,hi ,binding))
                                 (list i (if (= lo hi) (key-description (vector lo))
                                           (format "%s..%s" (key-description (vector lo)) (key-description (vector hi))))
                                       binding))
                               (romacs-popup--hint-fragments (nreverse characters)))
                       others)))
      ;; `sort' is stable: a range's parts stay in ascending order.
      (delq nil (mapcar (pcase-lambda (`(,_ ,key ,binding))
                          (when-let* ((label (romacs-popup--hint-label binding))) (cons key label)))
                        (sort keys (lambda (a b) (< (car a) (car b)))))))))

(defun romacs-popup--hints-place ()
  "Where hints show now: the selected window (of the root frame, not the
minibuffer) and its edges."
  (let ((window (romacs-layer--toast-window)))
    (list window (window-edges window t nil t))))

(defun romacs-popup--hints-show (map)
  "Show MAP's keys as hints in the selected window, retiring the toast of
its MESSAGE (shown once, here, while the hints show)."
  (setq romacs-popup--hints-shown map romacs-popup--hints-place (romacs-popup--hints-place))
  (let* ((window (car romacs-popup--hints-place))
         (text (mapconcat (lambda (e) (concat (propertize (car e) 'face 'romacs-popup-key) " "
                                              (propertize (cdr e) 'face 'romacs-popup-description)))
                          (romacs-popup--hint-entries map) "   "))
         (columns (max 10 (- (window-body-width window) 4)))
         (text (if (> (string-width text) columns) (concat (truncate-string-to-width text (1- columns)) "…") text))
         (buffer (romacs-popup--hints-buffer))
         (message (gethash map romacs-popup--hint-maps)))
    (with-current-buffer buffer (let ((inhibit-read-only t)) (erase-buffer) (insert text) (goto-char (point-min))))
    (romacs-popup-show 'hints buffer :anchor window :columns (1+ (string-width text)) :lines 1)
    (romacs-popup--hints-follow-hooks t)
    (when (stringp message)
      (dolist (toast (seq-filter (lambda (toast) (equal (nth 1 toast) message)) romacs-layer--toasts))
        (cancel-timer (nth 2 toast))
        (romacs-layer--toast-expire (car toast))))))

(defun romacs-popup--hints-hide ()
  (setq romacs-popup--hints-shown nil romacs-popup--hints-place nil)
  (romacs-popup--hints-follow-hooks nil)
  (romacs-popup-hide (romacs-popup--hints-buffer)))

(defun romacs-popup--hints-follow (frame)
  "After a change to FRAME's windows (the selection, a size), reconcile
the shown hints when FRAME is the root frame."
  (when (and romacs-popup--hints-shown (eq frame (romacs-layer--root-frame)))
    (romacs-popup--hints-schedule)))

(defun romacs-popup--hints-follow-hooks (on)
  "Follow window changes while hints show (ON), else not."
  (dolist (hook '(window-selection-change-functions window-size-change-functions))
    (if on (add-hook hook #'romacs-popup--hints-follow) (remove-hook hook #'romacs-popup--hints-follow))))

(defun romacs-popup--hints-cancel ()
  "Cancel the timer that would show hints."
  (when (cdr romacs-popup--hints-pending) (cancel-timer (cdr romacs-popup--hints-pending)))
  (setq romacs-popup--hints-pending nil))

(defun romacs-popup--hints-due (map)
  "The pause before MAP's hints is over: show them if MAP is still the
target, else follow what is."
  (setq romacs-popup--hints-pending nil)
  (if (eq (romacs-popup--hint-target) map)
      (romacs-popup--hints-show map)
    (romacs-popup--hints-reconcile)))

(defun romacs-popup--hints-reconcile ()
  "Show, move, hide or schedule hints for what `romacs-popup--hint-target'
is now and where the selected window is."
  (setq romacs-popup--hints-scheduled nil)
  (let ((target (romacs-popup--hint-target)))
    (cond ((and target (eq target romacs-popup--hints-shown))
           (unless (equal romacs-popup--hints-place (romacs-popup--hints-place))
             (romacs-popup--hints-show target)))
          (romacs-popup--hints-shown (romacs-popup--hints-hide)))
    (unless (eq target (car romacs-popup--hints-pending))
      (romacs-popup--hints-cancel))
    (when (and target (not (eq target romacs-popup--hints-shown)) (not romacs-popup--hints-pending))
      ;; A fresh delay from now, also when Emacs is already idle (an idle
      ;; timer whose time has passed fires at once).
      (let ((delay (if-let* ((idle (current-idle-time))) (time-add idle romacs-popup-hints-delay) romacs-popup-hints-delay)))
        (setq romacs-popup--hints-pending
              (cons target (run-with-idle-timer delay nil #'romacs-popup--hints-due target)))))))

(defun romacs-popup--hints-schedule (&rest _)
  "Reconcile the hints once, when the current command or timer is done:
a map popped and pushed again within a command (repeat-mode) shows no
change."
  (unless romacs-popup--hints-scheduled
    (setq romacs-popup--hints-scheduled t)
    (run-at-time 0 nil #'romacs-popup--hints-reconcile)))

(defun romacs-popup--keymap-stack (keymap symbol &rest _)
  "After KEYMAP is pushed on or popped off SYMBOL, follow it with hints."
  (ignore keymap)
  (when (eq symbol 'overriding-terminal-local-map) (romacs-popup--hints-schedule)))

(defun romacs-popup--holds-hint-map (keymap)
  "Non-nil when KEYMAP, composed by `internal-push-keymap', holds a map
registered for hints: a walk over its pushed maps only."
  (and (> (hash-table-count romacs-popup--hint-maps) 0)
       (let ((tail (cdr-safe keymap)) found)
         (while (and (consp tail) (not found) (keymapp (car tail)))
           (setq found (gethash (car tail) romacs-popup--hint-maps) tail (cdr tail)))
         found)))

(defun romacs-popup--keymap-set (_symbol value &rest _)
  "`overriding-terminal-local-map' is about to be VALUE (isearch replaces
and restores it): follow it with hints when that can change them, hints
shown or due, or VALUE holding a registered map.  `read-key' binding it
around each read, with no hints, schedules nothing."
  (when (or romacs-popup--hints-shown romacs-popup--hints-pending (romacs-popup--holds-hint-map value))
    (romacs-popup--hints-schedule)))

(defun romacs-popup--set-transient-map (orig map &rest args)
  "Register MAP for hints and call ORIG, `set-transient-map', with MAP
and ARGS unchanged; its MESSAGE text is what it then shows."
  (unless romacs-popup-hints-inhibit (puthash map t romacs-popup--hint-maps))
  (prog1 (apply orig map args)
    (when (and (nth 2 args) (not romacs-popup-hints-inhibit) (current-message))
      (puthash map (substring-no-properties (current-message)) romacs-popup--hint-maps))))

(defun romacs-popup--hints-claim (text)
  "Claim TEXT, the echo area's new message, when it is the MESSAGE of the
map whose hints show."
  (and romacs-popup--hints-shown
       (equal (gethash romacs-popup--hints-shown romacs-popup--hint-maps) (substring-no-properties text))))

(defvar romacs-mode-line-repeat)
(declare-function romacs-mode-line-repeat-text "romacs-mode-line" (map))

(defun romacs-popup-repeat-echo (map)
  "`repeat-echo-function': the repeat map MAP's keys show once, as hints
while `romacs-popup-hints-mode' is on, else in the status bar's repeat
segment; MAP nil, the repeat over, clears that."
  (let ((text (and map (not (bound-and-true-p romacs-popup-hints-mode)) (fboundp 'romacs-mode-line-repeat-text)
                   (romacs-mode-line-repeat-text map))))
    (unless (equal text (bound-and-true-p romacs-mode-line-repeat))
      (setq romacs-mode-line-repeat text)
      (force-mode-line-update t))))

(defvar repeat-mode)
(defvar repeat-echo-function)
(declare-function repeat-echo-message "repeat" (map))

(defun romacs-popup--repeat-echo-install ()
  "As `repeat-mode' turns on, show its pending repeats through
`romacs-popup-repeat-echo' where `repeat-echo-function' is still GNU's
`repeat-echo-message' (a message, a toast after every repeat); a user's
own choice stays."
  (when (and repeat-mode (eq repeat-echo-function #'repeat-echo-message))
    (setq repeat-echo-function #'romacs-popup-repeat-echo)))

(add-hook 'repeat-mode-hook #'romacs-popup--repeat-echo-install)

(define-minor-mode romacs-popup-hints-mode
  "Show the keys of an active transient map (repeat-mode's and the like)
as hints at the bottom of the selected window, after a pause."
  :global t :group 'romacs-popup
  (if romacs-popup-hints-mode
      (progn (advice-add 'set-transient-map :around #'romacs-popup--set-transient-map)
             (advice-add 'internal-push-keymap :after #'romacs-popup--keymap-stack)
             (advice-add 'internal-pop-keymap :after #'romacs-popup--keymap-stack)
             (add-variable-watcher 'overriding-terminal-local-map #'romacs-popup--keymap-set)
             (add-hook 'romacs-popup-panel-functions #'romacs-popup--hints-reconcile)
             (add-hook 'romacs-layer-echo-claim-functions #'romacs-popup--hints-claim)
             (romacs-popup--hints-reconcile))
    (advice-remove 'set-transient-map #'romacs-popup--set-transient-map)
    (advice-remove 'internal-push-keymap #'romacs-popup--keymap-stack)
    (advice-remove 'internal-pop-keymap #'romacs-popup--keymap-stack)
    (remove-variable-watcher 'overriding-terminal-local-map #'romacs-popup--keymap-set)
    (remove-hook 'romacs-popup-panel-functions #'romacs-popup--hints-reconcile)
    (remove-hook 'romacs-layer-echo-claim-functions #'romacs-popup--hints-claim)
    (romacs-popup--hints-cancel)
    (romacs-popup--hints-hide)))

;;;; which-key's `custom' popup type (which-key.el `which-key-custom-*')

(defvar which-key--buffer)

(defun romacs-popup-which-key-show (dimensions)
  "Show which-key's buffer as a `panel' menu; DIMENSIONS is (LINES . COLUMNS)."
  (romacs-popup-show 'panel which-key--buffer :lines (car dimensions)))

(defun romacs-popup-which-key-hide ()
  "Hide which-key's panel."
  (when (buffer-live-p which-key--buffer)
    (romacs-popup-hide which-key--buffer)))

(defun romacs-popup-panel-columns ()
  "The columns a `panel' menu's text can fill: the root frame's text
width less the panel's padding both sides, in its columns, less the one
a fringe-less window keeps for its continuation glyph."
  (let* ((root (romacs-layer--root-frame))
         (padding (if romacs-layer-glass
                      (plist-get (alist-get 'panel romacs-popup-look) :padding)
                    (alist-get 'child-frame-border-width romacs-layer-frame-parameters))))
    (max 1 (1- (/ (- (frame-text-width root) (* 2 padding)) (frame-char-width root))))))

(defun romacs-popup-which-key-max-dimensions (_selected-window-width)
  "The most which-key's panel shows, as (LINES . COLUMNS)."
  (cons romacs-layer-max-lines (romacs-popup-panel-columns)))

(provide 'romacs-popup)
;;; romacs-popup.el ends here
