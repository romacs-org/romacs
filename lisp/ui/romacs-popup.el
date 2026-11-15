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
;; `hints'    the keys of an active transient map;
;; `viewer'   a centred picture or file (romacs-attach.el's viewer).
;;
;; A menu never takes focus: keys reach the command loop as in GNU,
;; through the map its caller activated.  Dismissing a menu hides it;
;; deactivating a map is its caller's.  A menu whose buffer has keys of
;; its own takes them while it shows (`romacs-popup-take-keys').  The Lisp APIs
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
    (hints :radius 6 :padding 4 :motion fade :edge bottom)
    (card :radius 8 :padding 12 :motion fade :edge bottom)
    (viewer :radius 10 :padding 12 :motion fade :edge bottom))
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

(cl-defun romacs-popup-show (kind buffer &key anchor columns lines max-lines (face 'romacs-popup))
  "Show BUFFER as a KIND menu (see `romacs-popup-look') and return its
frame, or nil when it cannot show.  ANCHOR is a `cursor' menu's
\(WINDOW . POSITION), a `hints' menu's window (the selected one), a
`card''s top-left (LEFT . TOP) in pixels, a `palette''s when not centred.
COLUMNS and LINES size it, by default to BUFFER's text, at most
MAX-LINES lines (by default the kind's); FACE paints its background."
  (let* ((buffer (window-normalize-buffer buffer))
         (frame (romacs-layer--frame buffer)))
    (modify-frame-parameters frame (romacs-popup--parameters kind face))
    (set-window-hscroll (frame-root-window frame) 0)
    (prog1 (romacs-layer-show buffer
                              :position (pcase kind
                                          ('panel 'bottom)
                                          ('palette (if anchor `(pixel ,(car anchor) ,(cdr anchor)) 'center))
                                          ('viewer 'center)
                                          ('cursor `(at ,(car anchor) ,(cdr anchor)))
                                          ('hints `(window-bottom ,(or anchor (selected-window))))
                                          ('card `(pixel ,(car anchor) ,(cdr anchor)))
                                          (_ (error "Unknown menu kind %S" kind)))
                              :columns columns :lines lines
                              ;; A key menu is modal and may be taller than a
                              ;; list: the frame less a few lines of its windows;
                              ;; a card is laid out to fit already.
                              :max-lines (cond (max-lines)
                                               ((memq kind '(card viewer)) lines)
                                               ((eq kind 'panel)
                                                (max romacs-layer-max-lines
                                                     (- (frame-text-lines (romacs-layer--root-frame)) 4)))))
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
        ((keymapp binding) "+...")
        ((and (consp binding) (eq (car binding) 'menu-item)) (and (stringp (nth 1 binding)) (nth 1 binding)))
        ((symbolp binding)
         (let ((name (symbol-name binding)))
           (dolist (prefix romacs-popup-hints-strip name)
             (when (string-prefix-p prefix name) (setq name (substring name (length prefix)))))))
        ((functionp binding)
         (let ((doc (ignore-errors (documentation binding t))))
           (if (and doc (not (string-empty-p doc))) (car (split-string doc "\n")) "function")))))

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

(defun romacs-popup--map-items (map)
  "MAP's effective bindings with its parents, nil ones kept, in one
traversal: (CHARACTERS . OTHERS), CHARACTERS (INDEX LO HI BINDING)
fragments no earlier binding covers, OTHERS (INDEX EVENT BINDING) for
function keys, the first of each.  Remaps, defaults, menu-bar and mouse
events are left out."
  (let ((seen (make-hash-table :test 'eq)) (n 0) characters others)
    (map-keymap
     (lambda (event binding)
       (setq n (1+ n))
       (cond
        ((consp event) (push (list n (car event) (cdr event) binding) characters))
        ((integerp event) (push (list n event event binding) characters))
        ((or (memq event '(remap t menu-bar tool-bar)) (string-match-p "mouse" (symbol-name event))))
        ((gethash event seen))
        (t (puthash event t seen) (push (list n event binding) others))))
     map)
    (cons (romacs-popup--hint-fragments (nreverse characters)) (nreverse others))))

(defun romacs-popup--hint-entries (map)
  "MAP's bindings as (KEY . LABEL), in one traversal
\(`romacs-popup--map-items'): what a key is bound to first (a map's own
before its parents') is its effective binding, a nil one included, for a
character range as for a single key, so a range shows split around the
keys bound before it.  In traversal order."
  (pcase-let ((`(,characters . ,others) (romacs-popup--map-items map)))
    (let ((keys (nconc (mapcar (pcase-lambda (`(,i ,lo ,hi ,binding))
                                 (list i (if (= lo hi) (key-description (vector lo))
                                           (format "%s..%s" (key-description (vector lo)) (key-description (vector hi))))
                                       binding))
                               characters)
                       (mapcar (pcase-lambda (`(,i ,event ,binding)) (list i (key-description (vector event)) binding))
                               others))))
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
         (dots (truncate-string-ellipsis))
         (text (if (> (romacs-layer--toast-width text) columns)
                   (concat (truncate-string-to-width text (- columns (romacs-layer--toast-width dots))) dots)
                 text))
         (buffer (romacs-popup--hints-buffer))
         (message (gethash map romacs-popup--hint-maps)))
    (with-current-buffer buffer (let ((inhibit-read-only t)) (erase-buffer) (insert text) (goto-char (point-min))))
    (romacs-popup-show 'hints buffer :anchor window :columns (1+ (romacs-layer--toast-width text)) :lines 1)
    (romacs-popup--hints-follow-hooks t)
    (when (stringp message)
      (dolist (toast (seq-filter (lambda (toast) (equal (nth 3 toast) message)) romacs-layer--toasts))
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

;;;; Key hints: a card for a pending prefix (which-key's automatic hints)
;;
;; After a prefix key and which-key's pause, the keys that can follow it
;; and what they do, in a content-sized card at the selected pane's
;; bottom right.  which-key stays the switch, the timer and the source of
;; descriptions (`which-key-add-key-based-replacements',
;; `which-key-replacement-alist'); the bindings, their order and the card
;; are Romacs's.  which-key's explicit commands (`which-key-show-top-level'
;; and the like) keep its pages in the `panel'.  Design: popup-layers.md
;; §19.2.

(defface romacs-popup-group '((t :inherit font-lock-keyword-face))
  "A prefix's name in a key-hint card: `+name'.")

(defcustom romacs-popup-prefix-names
  '((ctl-x-4-map . "other-window") (ctl-x-5-map . "frame") (ctl-x-r-map . "register")
    (narrow-map . "narrow") (tab-prefix-map . "tab") (vc-prefix-map . "vc")
    (project-prefix-map . "project") (ctl-x-x-map . "buffer") (window-prefix-map . "window")
    (kmacro-keymap . "kmacro") (mule-keymap . "coding") (iso-transl-ctl-x-8-map . "char")
    (abbrev-map . "abbrev"))
  "Names of GNU's own prefix maps in a key-hint card, (VARIABLE . NAME).
A prefix key bound to VARIABLE's value is `+NAME' where no which-key
replacement names it; a key bound to anything else never is."
  :type '(alist :key-type symbol :value-type string))

(defvar which-key--pages-obj)
(defvar which-key--buffer)
(defvar which-key--automatic-display)
(defvar which-key-mode)
(declare-function make-which-key--pages "which-key")
(declare-function which-key--maybe-replace "which-key" (key-binding))
(declare-function which-key--popup-showing-p "which-key" ())

(defvar romacs-popup--card nil
  "The card's state, apart from whether it shows: (PREFIX PAGE PAGES
COLUMNS), PREFIX its key vector, PAGES its pages of rows.")

(defvar romacs-popup--card-place nil
  "Where the shown card was placed for: the selected window and its edges.")

(defvar romacs-popup--card-pages nil
  "The `which-key--pages' object the card gave which-key, to tell it from
one of which-key's explicit panels.")

;;; Bindings

(defun romacs-popup--card-winners (prefix)
  "PREFIX's next keys over `(current-active-maps t)', as `key-binding'
resolves them: within a map with its parents a nil hides its parent's
binding, between maps the first non-nil binding wins.  A list of (KEY
BINDING SOURCE ORDER), KEY a vector, SOURCE `global' or `mode', in
precedence order."
  (let ((global (current-global-map)) (seen (make-hash-table :test 'eq)) (i 0) characters others)
    (dolist (map (current-active-maps t))
      (let ((prefix-map (lookup-key map prefix t)))
        (when (keymapp prefix-map)
          (pcase-let ((`(,chars . ,keys) (romacs-popup--map-items prefix-map))
                      (source (if (eq map global) 'global 'mode)))
            (dolist (item chars)
              (when (nth 3 item) (push (list (setq i (1+ i)) (nth 1 item) (nth 2 item) (cons (nth 3 item) source)) characters)))
            (dolist (item keys)
              (when (and (nth 2 item) (not (gethash (nth 1 item) seen)))
                (puthash (nth 1 item) t seen)
                (push (list (setq i (1+ i)) (vector (nth 1 item)) (nth 2 item) source) others)))))))
    (let ((winners (nconc (mapcan (pcase-lambda (`(,order ,lo ,hi (,binding . ,source)))
                                    (if (= lo hi) (list (list order (vector lo) binding source))
                                      ;; A range shows as one entry, its first key and last.
                                      (list (list order (vector lo hi) binding source))))
                                  (romacs-popup--hint-fragments (nreverse characters)))
                          (nreverse others))))
      (mapcar (pcase-lambda (`(,order ,key ,binding ,source)) (list key binding source order))
              (sort winners (lambda (a b) (< (car a) (car b))))))))

(defun romacs-popup--humanise (name)
  "NAME, a command's or a map's, without GNU's naming noise."
  (let ((name (replace-regexp-in-string "\\`ctl-x-" "" name)))
    ;; Stacked suffixes too: `vc-prefix-map' is `vc'.
    (replace-regexp-in-string "\\(?:-\\(?:prefix\\|command\\|map\\)\\)+\\'" "" name)))

(defun romacs-popup--card-entry (prefix winner)
  "WINNER of PREFIX as the card shows it: (KEY LABEL PREFIX-P SOURCE ORDER),
or nil to leave it out (`ignore', `undefined', a which-key replacement
to nil)."
  (pcase-let* ((`(,key ,binding ,source ,order) winner)
               (item (and (eq (car-safe binding) 'menu-item) binding))
               (binding (if item (nth 2 item) binding))
               (prefix-p (keymapp binding))
               (key-desc (if (= (length key) 2)
                             (format "%s..%s" (key-description (vector (aref key 0))) (key-description (vector (aref key 1))))
                           (key-description key)))
               ;; NAME as which-key passes it: a symbol's name (a prefix
               ;; command's too), `prefix' for a keymap with none.
               (name (cond ((and binding (symbolp binding)) (symbol-name binding))
                           (prefix-p "prefix")
                           (t (or (romacs-popup--hint-label binding) "function"))))
               (map (and prefix-p (if (symbolp binding) (indirect-function binding) binding))))
    (unless (memq binding '(ignore undefined))
      (cond
       ((and item (stringp (nth 1 item))) (list key-desc (nth 1 item) prefix-p source order))
       (t
        (let* ((full (string-trim (concat (key-description prefix) " " key-desc)))
               (replaced (if (fboundp 'which-key--maybe-replace) (which-key--maybe-replace (cons full name)) (cons full name))))
          (when replaced
            ;; The replaced key less the prefix's keys, one token each (a
            ;; replacement may respell them: which-key drops `<>').
            (let ((shown-key (let ((tokens (split-string (car replaced) " " t)))
                               (if (> (length tokens) (length prefix))
                                   (string-join (nthcdr (length prefix) tokens) " ")
                                 (car replaced)))))
              (list shown-key
                    (cond ((not (equal (cdr replaced) name)) (cdr replaced))
                          (prefix-p (romacs-popup--prefix-name binding map key full))
                          (t (romacs-popup--humanise name)))
                    prefix-p source order)))))))))

(defcustom romacs-popup-prefix-keys '(("C-x 8" . "char"))
  "Names of GNU's own anonymous prefix maps in a key-hint card, (KEY .
NAME), KEY the whole sequence: used only while KEY holds a keymap no
variable names (a key rebound to a command or a named map is not)."
  :type '(alist :key-type string :value-type string))

(defvar romacs-popup--map-names nil
  "(TICK . INDEX): INDEX maps a keymap to a variable holding it, built from
the obarray when `load-history' was TICK long; built again only when a
load has added to it.")

(defun romacs-popup--map-names-refresh ()
  "Build `romacs-popup--map-names' again when a load has added to
`load-history': once per card, not per prefix."
  (let ((tick (length load-history)))
    (unless (eql (car romacs-popup--map-names) tick)
      (let ((index (make-hash-table :test 'eq :weakness 'key)))
        (mapatoms (lambda (symbol)
                    (when (and (boundp symbol) (not (keywordp symbol)))
                      (let ((value (symbol-value symbol)))
                        (when (and (consp value) (eq (car value) 'keymap) (not (gethash value index)))
                          (puthash value symbol index))))))
        (setq romacs-popup--map-names (cons tick index))))))

(defun romacs-popup--map-variable (map)
  "A variable whose value is MAP, from `romacs-popup--map-names'."
  (and romacs-popup--map-names (gethash map (cdr romacs-popup--map-names))))

(defun romacs-popup--prefix-name (binding map key full)
  "A prefix's name when no which-key replacement gives one: BINDING (a
keymap, or a symbol whose function is MAP) as KEY, the whole sequence
FULL.  GNU's own maps by `romacs-popup-prefix-names'; an ESC map `meta';
MAP's own prompt; the variable holding it, humanised; the prefix
command's name; `romacs-popup-prefix-keys' for an anonymous one; else
`prefix'."
  (let ((variable (and (consp map) (romacs-popup--map-variable map))))
    (or (seq-some (pcase-lambda (`(,var . ,label)) (and (or (eq var binding) (eq var variable)) label))
                  romacs-popup-prefix-names)
        (and (equal key [27]) "meta")
        (keymap-prompt map)
        (and variable (romacs-popup--humanise (symbol-name variable)))
        (and binding (symbolp binding) (romacs-popup--humanise (symbol-name binding)))
        (cdr (assoc full romacs-popup-prefix-keys))
        "prefix")))

(defun romacs-popup--card-entries (prefix)
  "The entries PREFIX's card shows, in its order: prefixes first, then
commands; within each a mode's before global ones, then key order."
  (romacs-popup--map-names-refresh)
  (let ((entries (delq nil (mapcar (lambda (w) (romacs-popup--card-entry prefix w)) (romacs-popup--card-winners prefix)))))
    (sort entries (lambda (a b)
                    (let ((ka (list (if (nth 2 a) 0 1) (if (eq (nth 3 a) 'mode) 0 1)))
                          (kb (list (if (nth 2 b) 0 1) (if (eq (nth 3 b) 'mode) 0 1))))
                      (if (equal ka kb) (< (nth 4 a) (nth 4 b)) (version-list-< ka kb)))))))

;;; Layout

(defun romacs-popup--card-column (entries width)
  "ENTRIES as one card column of lines: keys right-aligned in one key
column, two spaces, the label, cut with `…' past WIDTH columns."
  (let ((key-width (seq-reduce (lambda (w e) (max w (string-width (car e)))) entries 0)))
    (mapcar (lambda (e)
              (let* ((label (if (nth 2 e) (concat "+" (nth 1 e)) (nth 1 e)))
                     (room (- width key-width 2))
                     (label (if (> (string-width label) room) (truncate-string-to-width label (max 1 room) nil nil t) label)))
                (concat (make-string (- key-width (string-width (car e))) ?\s)
                        (propertize (car e) 'face 'romacs-popup-key) "  "
                        (propertize label 'face (if (nth 2 e) 'romacs-popup-group 'romacs-popup-description)))))
            entries)))

(defun romacs-popup--card-natural (entries)
  "The columns ENTRIES take uncut as one card column."
  (let ((key-width (seq-reduce (lambda (w e) (max w (string-width (car e)))) entries 0)))
    (+ key-width 2 (seq-reduce (lambda (w e) (max w (+ (string-width (nth 1 e)) (if (nth 2 e) 1 0)))) entries 0))))

(defun romacs-popup--card-fits (pages rows width)
  "Whether every page of PAGES is at most ROWS rows and every row at most
WIDTH columns, as rendered."
  (seq-every-p (lambda (page) (let ((lines (split-string page "\n")))
                                (and (<= (length lines) rows)
                                     (seq-every-p (lambda (line) (<= (romacs-layer--toast-width line) width)) lines))))
               pages))

(defun romacs-popup--card-paged (entries body columns column-width gap)
  "ENTRIES in pages of COLUMNS columns of BODY rows, at most COLUMN-WIDTH
each, GAP apart, each page but the last ending in `… N more · C-h'."
  (let ((n (length entries)) (shown 0) result)
    (dolist (page (seq-partition entries (* body columns)) (nreverse result))
      (setq shown (+ shown (length page)))
      (let* ((groups (seq-partition page body))
             (widths (mapcar (lambda (g) (min column-width (romacs-popup--card-natural g))) groups))
             (text (romacs-popup--card-join groups widths gap)))
        (push (if (< shown n)
                  (concat text "\n" (propertize (format "... %d more · %s" (- n shown) (key-description (vector help-char)))
                                                'face 'romacs-popup-dim))
                text)
              result)))))

(defun romacs-popup--card-pages (entries rows width)
  "ENTRIES in pages of at most ROWS rows by 1-3 balanced columns, WIDTH
columns wide in all, 3 apart: one page when they fit, else pages whose
last row (inside ROWS) is `… N more · C-h', with the most columns whose
labels all fit uncut (one column with its labels cut where none do).
Nil when no layout fits: a key wider than the room, or no row to spare."
  (let ((gap 3) (n (length entries)))
    (or
     ;; One page: the fewest columns that hold every entry within WIDTH.
     (cl-loop for columns from 1 to 3
              for per = (ceiling n (float columns))
              for groups = (seq-partition entries (max 1 per))
              for widths = (mapcar #'romacs-popup--card-natural groups)
              for page = (and (<= per rows) (<= (+ (apply #'+ widths) (* gap (1- (length groups)))) width)
                              (list (romacs-popup--card-join groups widths gap)))
              when (and page (romacs-popup--card-fits page rows width))
              return page)
     ;; Pages: a row of each kept for the footer.
     (cl-loop with natural = (romacs-popup--card-natural entries)
              with body = (1- rows)
              for columns from 3 downto 1
              for column-width = (/ (- width (* gap (1- columns))) columns)
              for pages = (and (>= body 1) (or (>= column-width natural) (= columns 1))
                               (romacs-popup--card-paged entries body columns column-width gap))
              when (and pages (romacs-popup--card-fits pages rows width))
              return pages))))

(defun romacs-popup--card-join (groups widths gap)
  "GROUPS of entries as columns of WIDTHS side by side, GAP apart."
  (let* ((columns (seq-mapn #'romacs-popup--card-column groups widths))
         (height (seq-reduce (lambda (h c) (max h (length c))) columns 0))
         rows)
    (dotimes (i height)
      (push (string-trim-right
             (string-join (seq-mapn (lambda (column width)
                                       (let ((line (or (nth i column) "")))
                                         (concat line (make-string (max 0 (- width (string-width line))) ?\s))))
                                     columns widths)
                          (make-string gap ?\s)))
            rows))
    (string-join (nreverse rows) "\n")))

;;; Placement

(defun romacs-popup--rect-meets (a b)
  "Whether rectangles A and B, (LEFT TOP RIGHT BOTTOM), overlap."
  (and a b (< (nth 0 a) (nth 2 b)) (< (nth 0 b) (nth 2 a)) (< (nth 1 a) (nth 3 b)) (< (nth 1 b) (nth 3 a))))

(defun romacs-popup--card-bounds (window)
  "Where WINDOW's card may go: its text area less 12 px each side, in the
root frame's pixels, (LEFT TOP RIGHT BOTTOM)."
  (pcase-let ((`(,left ,top ,right ,bottom) (window-edges window t nil t)))
    (list (+ left 12) (+ top 12) (- right 12) (- bottom 12))))

(defun romacs-popup--card-place (window bounds width height)
  "The outer top left of a WIDTH by HEIGHT card in BOUNDS: bottom right,
bottom left, top right or top left, the first over neither point's glyph
in WINDOW nor a shown minibuffer layer; bottom right when all are."
  (pcase-let* ((`(,left ,top ,right ,bottom) bounds)
               (body (window-edges window t nil t))
               (posn (posn-at-point (window-point window) window))
               (xy (and posn (posn-x-y posn)))
               (cw (frame-char-width (romacs-layer--root-frame))) (ch (frame-char-height (romacs-layer--root-frame)))
               ;; Point's glyph where the window draws it: after an hscroll, or
               ;; at a truncated row's last column for a point past it (GNU's
               ;; posn too), a column the card's inset bounds never reach.
               (point (and xy (let ((x (+ (nth 0 body) (car xy))) (y (+ (nth 1 body) (cdr xy))))
                                (list x y (+ x cw) (+ y ch)))))
               (host (and (fboundp 'romacs--minibuffer-host) (romacs--minibuffer-host)))
               (panel (and host (frame-visible-p host)
                           (let ((p (frame-position host))) (list (car p) (cdr p) (+ (car p) (frame-pixel-width host)) (+ (cdr p) (frame-pixel-height host))))))
               (candidates (list (cons (- right width) (- bottom height)) (cons left (- bottom height))
                                 (cons (- right width) top) (cons left top))))
    (or (seq-find (pcase-lambda (`(,x . ,y))
                    (let ((rect (list x y (+ x width) (+ y height))))
                      (not (or (romacs-popup--rect-meets rect point) (romacs-popup--rect-meets rect panel)))))
                  candidates)
        (car candidates))))

;;; Showing and hiding

(defun romacs-popup--card-display-p ()
  "Whether which-key's automatic hints show as Romacs's card."
  (and (bound-and-true-p which-key-mode) (display-graphic-p)
       (eq (bound-and-true-p which-key-popup-type) 'custom)
       (eq (bound-and-true-p which-key-custom-show-popup-function) #'romacs-popup-which-key-show)))

(defun romacs-popup--card-layout (prefix)
  "PREFIX's card: (PAGES WINDOW BOUNDS), or nil when no card fits: the
selected pane's bounds, else the frame's above the status bar."
  (let* ((root (romacs-layer--root-frame))
         (window (romacs-layer--toast-window))
         (cw (frame-char-width root)) (ch (frame-char-height root))
         (padding (plist-get (alist-get 'card romacs-popup-look) :padding))
         (entries (romacs-popup--card-entries prefix))
         (fit (lambda (bounds lines)
                (pcase-let ((`(,left ,top ,right ,bottom) bounds))
                  (let ((rows (min (- (floor (- bottom top (* 2 padding)) ch) 1) lines))
                        (width (- (floor (- right left (* 2 padding)) cw) 1)))
                    (and entries (>= rows 1) (> width 0)
                         (romacs-popup--card-pages entries rows width)))))))
    (let* ((pane (romacs-popup--card-bounds window))
           (pages (funcall fit pane (max 1 (floor (* 0.6 (window-body-height window)))))))
      (if pages (list pages window pane)
        (let* ((frame-bounds (list 12 (+ (romacs-chrome-top root) 12)
                                   (- (frame-native-width root) 12) (- (romacs-chrome-bottom root) 12)))
               (pages (funcall fit frame-bounds (frame-text-lines root))))
          (and pages (list pages window frame-bounds)))))))

(defun romacs-popup--card-render (prefix page)
  "Show PREFIX's card at PAGE (wrapping), its title the prefix; nil when
no card fits.  Records the card's state and which-key's prefix."
  (when-let* ((layout (romacs-popup--card-layout prefix)))
    (pcase-let* ((`(,pages ,window ,bounds) layout)
                 (page (mod page (length pages)))
                 (text (concat (propertize (key-description prefix) 'face 'romacs-popup-dim) "\n" (nth page pages)))
                 (root (romacs-layer--root-frame))
                 (cw (frame-char-width root)) (ch (frame-char-height root))
                 (padding (plist-get (alist-get 'card romacs-popup-look) :padding))
                 (lines (length (split-string text "\n")))
                 ;; One column more than the widest row as painted (the
                 ;; footer's `…' is two cells in Iosevka): a window's last
                 ;; holds its truncation glyph.
                 (columns (1+ (seq-reduce (lambda (w l) (max w (romacs-layer--toast-width l))) (split-string text "\n") 1)))
                 (`(,x . ,y) (romacs-popup--card-place window bounds (+ (* columns cw) (* 2 padding)) (+ (* lines ch) (* 2 padding))))
                 ;; which-key's buffer, made as its own display would make it,
                 ;; so `which-key--popup-showing-p' sees the card.
                 (buffer (if (and (boundp 'which-key--buffer) (buffer-live-p which-key--buffer)) which-key--buffer
                           (setq which-key--buffer (get-buffer-create " *which-key*")))))
      (with-current-buffer buffer
        (let ((inhibit-read-only t))
          (setq-local mode-line-format nil header-line-format nil cursor-type nil truncate-lines t)
          (erase-buffer) (insert text) (goto-char (point-min))))
      (setq romacs-popup--card (list (vconcat prefix) page pages))
      (romacs-popup-show 'card buffer :anchor (cons x y) :columns columns :lines lines)
      (setq romacs-popup--card-place (list window (window-edges window t nil t)))
      (romacs-popup--card-hooks t)
      t)))

(defun romacs-popup--card-hide (&rest _)
  "Hide the card, keeping its state (paging goes on from it)."
  (romacs-popup--card-hooks nil)
  (when-let* ((buffer (and (boundp 'which-key--buffer) which-key--buffer))
              ((buffer-live-p buffer))
              (frame (romacs-layer--frame-of buffer))
              ((eq (frame-parameter frame 'romacs-popup-kind) 'card)))
    (romacs-popup-hide buffer)))

(defun romacs-popup--card-follow (frame)
  "A window change on FRAME: hide the card when FRAME is the root frame
and the card's pane moved (another window selected, its edges changed);
showing the card itself changes nothing it was placed for."
  (when (and (eq frame (romacs-layer--root-frame))
             (not (equal romacs-popup--card-place
                         (let ((window (romacs-layer--toast-window))) (list window (window-edges window t nil t))))))
    (romacs-popup--card-hide)))

(defun romacs-popup--card-hooks (on)
  "Hide the card on the next key, a focus change or a window change while
it shows (ON), else not."
  (if on
      (progn (add-hook 'romacs--key-read-functions #'romacs-popup--card-hide)
             (add-hook 'window-selection-change-functions #'romacs-popup--card-follow)
             (add-hook 'window-size-change-functions #'romacs-popup--card-follow)
             (add-function :after after-focus-change-function #'romacs-popup--card-hide))
    (remove-hook 'romacs--key-read-functions #'romacs-popup--card-hide)
    (remove-hook 'window-selection-change-functions #'romacs-popup--card-follow)
    (remove-hook 'window-size-change-functions #'romacs-popup--card-follow)
    (remove-function after-focus-change-function #'romacs-popup--card-hide)))

(defun romacs-popup--card-record (prefix title)
  "Give which-key a one-page pages object for PREFIX, so its timer sees
the prefix as shown."
  (setq romacs-popup--card-pages
        (make-which-key--pages :pages (list "") :height 1 :widths (list 1) :keys/page (list 0)
                               :page-nums (list 1) :num-pages 1 :total-keys 0
                               :prefix prefix :prefix-title (or title "")))
  (setq which-key--pages-obj romacs-popup--card-pages))

(defun romacs-popup--which-key-create (orig &optional prefix-keys from-keymap filter prefix-title)
  "`which-key--create-buffer-and-show' advice: an automatic hint for a
prefix shows as the card; anything else as which-key does."
  (if (and (bound-and-true-p which-key--automatic-display) (null from-keymap) (null filter)
           (> (length prefix-keys) 0) (romacs-popup--card-display-p))
      (progn (romacs-popup--card-record prefix-keys prefix-title)
             (romacs-popup--card-render prefix-keys 0))
    (funcall orig prefix-keys from-keymap filter prefix-title)))

(defun romacs-popup--card-help (prefix _event)
  "`romacs--prefix-help-functions': PREFIX's help key pages its card (its
first page when none showed), unless the card is not the display, a
macro plays, or one of which-key's explicit panels shows."
  (when (and (romacs-popup--card-display-p) (> (length prefix) 0) (not executing-kbd-macro)
             (not (and (fboundp 'which-key--popup-showing-p) (which-key--popup-showing-p)
                       (not (eq which-key--pages-obj romacs-popup--card-pages)))))
    ;; Taken only when a card shows: where none fits, GNU's fallback runs.
    (let ((page (if (equal (car romacs-popup--card) (vconcat prefix)) (1+ (nth 1 romacs-popup--card)) 0)))
      (when (romacs-popup--card-render prefix page)
        (romacs-popup--card-record prefix nil)
        t))))

(defun romacs-popup--card-mode-hook ()
  "`which-key-mode' off: no card."
  (unless (bound-and-true-p which-key-mode)
    (romacs-popup--card-hide)
    (setq romacs-popup--card nil)))

(with-eval-after-load 'which-key
  (advice-add 'which-key--create-buffer-and-show :around #'romacs-popup--which-key-create)
  (add-hook 'which-key-mode-hook #'romacs-popup--card-mode-hook))
(add-hook 'romacs--prefix-help-functions #'romacs-popup--card-help)

;;;; which-key's `custom' popup type (which-key.el `which-key-custom-*')


(defun romacs-popup-which-key-show (dimensions)
  "Show which-key's buffer as a `panel' menu; DIMENSIONS is (LINES . COLUMNS)."
  (romacs-popup-show 'panel which-key--buffer :lines (car dimensions)))

(defun romacs-popup-which-key-hide ()
  "Hide which-key's panel or card: which-key is done with the prefix."
  (romacs-popup--card-hooks nil)
  (setq romacs-popup--card nil)
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

;;;; Menus that take keys
;;
;; A menu takes no focus, so the keys of its buffer reach it through a
;; transient map while it shows: a key its buffer's local map binds runs
;; there, the menu's window selected, so point moves in the menu as in a
;; window of its own.  Any other key hides the menu and runs as if it had
;; never shown.  One menu takes keys at a time.

(defvar romacs-popup--keys-buffer nil "The buffer whose menu takes keys.")
(defvar romacs-popup--keys-exit nil "What ends its transient map.")
(defvar romacs-popup--keys-then nil "What to call once the key's command returns.")

(defvar romacs-popup-key-hook nil
  "Run in a menu's buffer, its window selected, after each key it takes:
a buffer-local value follows point (a highlighted row).")

(defvar romacs-popup--keys-map
  (let ((map (make-sparse-keymap))) (define-key map [t] #'romacs-popup-menu-key) map)
  "Every key, while a menu takes keys.")

(defun romacs-popup--keys-window ()
  "The window of the menu taking keys, while it shows."
  (when-let* ((buffer romacs-popup--keys-buffer) ((buffer-live-p buffer))
              (window (get-buffer-window buffer t))
              ((frame-visible-p (window-frame window))))
    window))

(defun romacs-popup--function-key (keys)
  "KEYS, one function key event, as the command loop reads it through
`local-function-key-map' (<return> is RET, M-<return> M-RET), or nil."
  (let* ((event (and (= (length keys) 1) (aref keys 0)))
         ;; Its modifiers first: they parse a symbol `event-basic-type' reads.
         (modifiers (and (symbolp event) (event-modifiers event)))
         (plain (and (symbolp event) (lookup-key local-function-key-map (vector (event-basic-type event))))))
    (and (vectorp plain) (= (length plain) 1) (integerp (aref plain 0))
         (vector (seq-reduce (lambda (char modifier)
                               (if-let* ((shift (alist-get modifier '((meta . 27) (control . 26) (shift . 25)
                                                                      (hyper . 24) (super . 23) (alt . 22)))))
                                   (event-apply-modifier char modifier shift "")
                                 char))
                             modifiers (aref plain 0))))))

(defun romacs-popup--menu-command (keys map)
  "The command KEYS run in a menu whose buffer's local map is MAP: MAP's
own, or one MAP remaps (a menu's buffer may remap `self-insert-command'),
for a function key the binding of what it reads as; else nil.  The menu
reads one event: a prefix key is not the menu's."
  (let ((own (and map (lookup-key map keys))))
    (cond ((commandp own) own)
          ((not map) nil)
          ;; The global command MAP remaps, not one the user's buffer
          ;; binds the key to (a chat's own letters).
          ((let ((global (lookup-key (current-global-map) keys)))
             (and (symbolp global) global (command-remapping global nil (list map)))))
          ((when-let* ((plain (romacs-popup--function-key keys)))
             (romacs-popup--menu-command plain map))))))

(defun romacs-popup-take-keys (buffer &optional on-exit)
  "Make the menu showing BUFFER take the keys its local map binds, until
it hides or another key is typed, then call ON-EXIT, if given (the menu's
own layers hide with it); a key read while one of its commands reads the
minibuffer is the minibuffer's."
  (unless (and (eq buffer romacs-popup--keys-buffer) (romacs-popup--keys-window))
    (when romacs-popup--keys-exit (funcall romacs-popup--keys-exit))
    (setq romacs-popup--keys-buffer buffer)
    (let ((depth (minibuffer-depth)) (romacs-popup-hints-inhibit t))
      (setq romacs-popup--keys-exit
            (set-transient-map romacs-popup--keys-map
                               (lambda () (or (> (minibuffer-depth) depth)
                                              (and (eq this-command #'romacs-popup-menu-key)
                                                   (romacs-popup--keys-window))))
                               (lambda ()
                                 (when (eq romacs-popup--keys-buffer buffer)
                                   (setq romacs-popup--keys-buffer nil romacs-popup--keys-exit nil)
                                   (romacs-popup-hide buffer)
                                   (when on-exit (funcall on-exit)))))))))

(defun romacs-popup-then (function)
  "Hide the menu taking keys once this key's command returns, then call
FUNCTION in the window and buffer the user was in: what opens something
for the user (a buffer, a prompt of its own) runs there, not in the menu."
  (setq romacs-popup--keys-then function))

(defun romacs-popup-menu-key ()
  "Run the key just typed in the menu taking keys, or hide the menu and run
the key as if it had never shown."
  (declare (completion ignore))
  (interactive)
  (romacs-popup--menu-key (this-command-keys-vector)))

(defun romacs-popup--menu-key (keys)
  "Run KEYS in the menu taking keys, or hide it and run them as typed anew."
  (let* ((window (romacs-popup--keys-window))
         (command (and window (romacs-popup--menu-command
                               keys (with-current-buffer (window-buffer window) (current-local-map))))))
    (if (not command)
        (progn (when romacs-popup--keys-exit (funcall romacs-popup--keys-exit))
               (setq unread-command-events (append (listify-key-sequence keys) unread-command-events)))
      (setq romacs-popup--keys-then nil)
      ;; The map is the menu's only: not in a minibuffer its command reads.
      (let ((overriding-terminal-local-map nil))
        (with-selected-window window
          (setq this-command command)
          (call-interactively command)
          (run-hooks 'romacs-popup-key-hook)))
      (when-let* ((then romacs-popup--keys-then))
        (setq romacs-popup--keys-then nil)
        (when romacs-popup--keys-exit (funcall romacs-popup--keys-exit))
        (funcall then)))))

(provide 'romacs-popup)
;;; romacs-popup.el ends here
