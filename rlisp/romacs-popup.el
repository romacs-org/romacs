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
\(WINDOW . POSITION).  COLUMNS and LINES size it, by default to BUFFER's
text; FACE paints its background."
  (let* ((buffer (window-normalize-buffer buffer))
         (frame (romacs-layer--frame buffer)))
    (modify-frame-parameters frame (romacs-popup--parameters kind face))
    (set-window-hscroll (frame-root-window frame) 0)
    (prog1 (romacs-layer-show buffer
                              :position (pcase kind
                                          ('panel 'bottom)
                                          ('palette 'center)
                                          ('cursor `(at ,(car anchor) ,(cdr anchor)))
                                          (_ (error "Unknown menu kind %S" kind)))
                              :columns columns :lines lines
                              ;; A key menu is modal and may be taller than a
                              ;; list: the frame less a few lines of its windows.
                              :max-lines (and (eq kind 'panel)
                                              (max romacs-layer-max-lines
                                                   (- (frame-text-lines (romacs-layer--root-frame)) 4))))
      (when (eq kind 'panel) (run-hooks 'romacs-popup-panel-functions)))))

(defun romacs-popup-hide (buffer)
  "Hide the menu showing BUFFER, which fades; its frame is kept."
  (let ((panel (and (get-buffer buffer) (romacs-popup-panel-p (get-buffer buffer)))))
    (romacs-layer-hide buffer)
    (when panel (run-hooks 'romacs-popup-panel-functions))))

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
