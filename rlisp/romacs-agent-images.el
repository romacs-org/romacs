;;; romacs-agent-images.el --- Pick an image by sight -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Two cards side by side over a directory's images, newest first: a list
;; of thumbnails and file names, and the selected image large.  Typing
;; filters by file name, each word anywhere in it; C-n and C-p move, C-v
;; and M-v page, RET picks, TAB falls back to `read-file-name', C-g cancels.
;;
;; `romacs-complete' draws text rows, so this is a menu of its own.  A key
;; is a pure step from one state to the next.  Matches are found when the
;; query changes and indexed, and only the rows in view have images made
;; for them, so moving costs the same in any directory.

;;; Code:

(require 'seq)
(require 'romacs-popup)
(require 'romacs-agent)

(defvar romacs-agent-ui--end)
(defconst romacs-agent-images--rows 9 "Rows shown at once.")
(defconst romacs-agent-images--list-columns 52 "The list's width, in columns.")
(defconst romacs-agent-images--thumbnail-height 48 "A thumbnail's height, in pixels.")
(defconst romacs-agent-images--preview-min-width 240 "The narrowest useful preview, in pixels.")
(defconst romacs-agent-images--gap 12 "Pixels between the cards and round them.")

(defun romacs-agent-images--files (dir)
  "The images directly in DIR, newest first."
  (mapcar #'car
          (sort (seq-filter (lambda (entry) (romacs-agent-image-file-p (car entry)))
                            (directory-files-and-attributes dir t nil t))
                (lambda (a b) (time-less-p (file-attribute-modification-time (cdr b))
                                           (file-attribute-modification-time (cdr a)))))))

(defun romacs-agent-images--filter (files query)
  "The vector of FILES whose names hold every word of QUERY."
  (let ((words (split-string (downcase query))))
    (vconcat (seq-filter (lambda (file)
                           (let ((name (downcase (file-name-nondirectory file))))
                             (seq-every-p (lambda (word) (string-search word name)) words)))
                         files))))

(defun romacs-agent-images--state (files &optional query)
  "A menu over the vector FILES searching for QUERY, at its first match."
  (let ((query (or query "")))
    (list :files files :query query :selected 0 :matches (romacs-agent-images--filter files query))))

(defun romacs-agent-images--selection (state)
  "STATE's selected file, or nil when nothing matches."
  (let ((matches (plist-get state :matches)) (selected (plist-get state :selected)))
    (and (< selected (length matches)) (aref matches selected))))

(defun romacs-agent-images--move (state delta)
  "STATE with its selection moved by DELTA, within its matches."
  (let ((last (1- (length (plist-get state :matches)))))
    (plist-put (copy-sequence state) :selected
               (max 0 (min last (+ (plist-get state :selected) delta))))))

(defun romacs-agent-images--step (state key)
  "The state after KEY in STATE.  One with :result ends the menu: a file,
`browse' or `cancel'."
  (let ((query (plist-get state :query)))
    (pcase key
      ((or ?\C-n 'down) (romacs-agent-images--move state 1))
      ((or ?\C-p 'up) (romacs-agent-images--move state -1))
      ((or ?\C-v 'next) (romacs-agent-images--move state romacs-agent-images--rows))
      ((or ?\M-v 'prior) (romacs-agent-images--move state (- romacs-agent-images--rows)))
      ((or ?\C-m 'return)
       (let ((file (romacs-agent-images--selection state)))
         (if file (plist-put (copy-sequence state) :result file) state)))
      ((or ?\C-i 'tab) (plist-put (copy-sequence state) :result 'browse))
      ((or ?\C-g ?\e 'escape) (plist-put (copy-sequence state) :result 'cancel))
      ((or ?\d 'backspace)
       (if (string-empty-p query) state
         (romacs-agent-images--state (plist-get state :files) (substring query 0 -1))))
      ((pred (lambda (key) (and (characterp key) (>= key ?\s))))
       (romacs-agent-images--state (plist-get state :files) (concat query (string key))))
      (_ state))))

(defun romacs-agent-images--first-row (selected count)
  "The first of the rows in view, for SELECTED among COUNT."
  (max 0 (min (- selected (/ romacs-agent-images--rows 2))
              (- count romacs-agent-images--rows))))

(defun romacs-agent-images--age (file)
  "How long ago FILE changed, briefly."
  (let ((seconds (float-time (time-subtract nil (file-attribute-modification-time (file-attributes file))))))
    (cond ((< seconds 3600) (format "%dm" (/ seconds 60)))
          ((< seconds 86400) (format "%dh" (/ seconds 3600)))
          (t (format "%dd" (/ seconds 86400))))))

(defun romacs-agent-images--insert-image (file width height)
  "Insert FILE within WIDTH by HEIGHT pixels, when it loads and the display
shows images."
  (when (display-images-p)
    (let ((image (ignore-errors (create-image file nil nil :max-width width :max-height height))))
      (when image (insert-image image " ")))))

(defun romacs-agent-images--row-text (name age columns)
  "A row's text in COLUMNS: NAME, shortened to leave room for AGE, and AGE."
  (concat "  " (truncate-string-to-width name (- columns 4 (string-width age)) nil nil "…")
          (propertize (concat "  " age) 'face 'shadow)))

(defun romacs-agent-images--insert-row (file selected)
  "Insert the row of FILE, SELECTED or not: one line, always."
  (let ((start (point)) (width (* 2 romacs-agent-images--thumbnail-height)))
    (romacs-agent-images--insert-image file width romacs-agent-images--thumbnail-height)
    (insert (romacs-agent-images--row-text
             (file-name-nondirectory file) (romacs-agent-images--age file)
             ;; Three columns spare: an image takes a little more than its width.
             (- romacs-agent-images--list-columns 3 (ceiling width (frame-char-width))))
            "\n")
    (when selected (add-face-text-property start (point) 'highlight))))

(defun romacs-agent-images--draw (state)
  "Fill the current buffer, the list, from STATE."
  (let* ((inhibit-read-only t)
         (matches (plist-get state :matches))
         (selected (plist-get state :selected))
         (first (romacs-agent-images--first-row selected (length matches))))
    (erase-buffer)
    (insert (propertize "Attach image: " 'face 'minibuffer-prompt) (plist-get state :query)
            (propertize (format "  %d/%d" (length matches) (length (plist-get state :files))) 'face 'shadow)
            "\n")
    (if (zerop (length matches))
        (insert "No match\n")
      (dotimes (row (min romacs-agent-images--rows (- (length matches) first)))
        (romacs-agent-images--insert-row (aref matches (+ first row)) (= (+ first row) selected))))
    (goto-char (point-min))))

(defun romacs-agent-images--draw-preview (file layout)
  "Fill the current buffer, the preview, with FILE, or nothing when nil."
  (let ((inhibit-read-only t))
    (erase-buffer)
    (when file
      ;; A line wider than its window shows nothing of an image.
      (romacs-agent-images--insert-image file (- (plist-get layout :preview-width) (* 2 (frame-char-width)))
                                         (- (plist-get layout :height) (frame-char-height))))
    (goto-char (point-min))))

(defun romacs-agent-images--layout (left top right bottom cw ch)
  "Where the list and the preview go in the area LEFT TOP RIGHT BOTTOM, in
pixels, with characters CW by CH: centred together, of one height, in whole
character cells, as layers are sized.  A plist of :y :lines :height :list-x
:list-columns :preview-x :preview-columns :preview-width, or nil when they
do not fit.  Heights and widths are of the text; a card's padding goes round
it."
  (let* ((padding (* 2 (plist-get (alist-get 'card romacs-popup-look) :padding)))
         (gap romacs-agent-images--gap)
         (lines (ceiling (+ ch (* romacs-agent-images--rows romacs-agent-images--thumbnail-height)) ch))
         (height (* lines ch))
         (columns romacs-agent-images--list-columns)
         (list-width (* columns cw))
         (room (- right left (* 3 gap) list-width (* 2 padding)))
         (preview-columns (/ (min room (/ (* height 16) 9)) cw))
         (preview-width (* preview-columns cw))
         (total (+ list-width padding gap preview-width padding)))
    (when (and (>= preview-width romacs-agent-images--preview-min-width)
               (<= (+ height padding) (- bottom top (* 2 gap))))
      (let ((x (+ left (/ (- right left total) 2))))
        (list :y (+ top (/ (- bottom top height padding) 2)) :lines lines :height height
              :list-x x :list-columns columns
              :preview-x (+ x list-width padding gap)
              :preview-columns preview-columns :preview-width preview-width)))))

(defun romacs-agent-images--frame-layout ()
  "`romacs-agent-images--layout' for the root frame."
  (let ((root (romacs-layer--root-frame)))
    (romacs-agent-images--layout (frame-internal-border-width root) (romacs-chrome-top root)
                                 (- (frame-pixel-width root) (frame-internal-border-width root))
                                 (romacs-chrome-bottom root)
                                 (frame-char-width root) (frame-char-height root))))

(defun romacs-agent-images--show (buffer preview layout)
  "Show the list BUFFER and PREVIEW as cards, where LAYOUT says."
  (let ((y (plist-get layout :y)) (lines (plist-get layout :lines)))
    (romacs-popup-show 'card buffer :anchor (cons (plist-get layout :list-x) y)
                       :columns (plist-get layout :list-columns) :lines lines)
    (romacs-popup-show 'card preview :anchor (cons (plist-get layout :preview-x) y)
                       :columns (plist-get layout :preview-columns) :lines lines)))

(defun romacs-agent-images--checked (state dir)
  "STATE, unless its result is a file gone meanwhile: then the menu again
over DIR as it is now, keeping the query."
  (let ((result (plist-get state :result)))
    (if (or (not (stringp result)) (romacs-agent-image-file-p result)) state
      (message "%s is gone" (file-name-nondirectory result))
      (romacs-agent-images--state (vconcat (romacs-agent-images--files dir)) (plist-get state :query)))))

(defun romacs-agent-images-pick (dir)
  "Choose an image of DIR by sight: its file, `browse' or `cancel'.  A
frame too small for the cards browses, at once or when it shrinks."
  (let ((layout (romacs-agent-images--frame-layout)))
    (if (not layout) 'browse
      (let* ((buffer (get-buffer-create " *romacs-agent-images*"))
             (preview (get-buffer-create " *romacs-agent-images-preview*"))
             (state (romacs-agent-images--state (vconcat (romacs-agent-images--files dir))))
             (shown :none)
             (follow (lambda (frame)
                       (when (eq frame (romacs-layer--root-frame))
                         (let ((new (romacs-agent-images--frame-layout)))
                           (unless (equal new layout)
                             (setq layout new)
                             (if (not new)
                                 ;; TAB, as if asked: the file prompt needs no room.
                                 (setq unread-command-events (append unread-command-events (list ?\C-i)))
                               (romacs-agent-images--show buffer preview new)
                               (unless (eq shown :none)
                                 (with-current-buffer preview (romacs-agent-images--draw-preview shown new))))))))))
        (dolist (each (list buffer preview))
          (with-current-buffer each (setq mode-line-format nil cursor-type nil truncate-lines t)))
        (add-hook 'window-size-change-functions follow)
        (unwind-protect
            (let ((inhibit-quit t))
              (with-current-buffer buffer (romacs-agent-images--draw state))
              (romacs-agent-images--show buffer preview layout)
              (while (not (plist-get state :result))
                (with-current-buffer buffer (romacs-agent-images--draw state))
                ;; Decoding is the slow part: only a new selection pays it.
                (let ((file (romacs-agent-images--selection state)))
                  (unless (equal file shown)
                    (with-current-buffer preview (romacs-agent-images--draw-preview file layout))
                    (setq shown file)))
                (setq state (romacs-agent-images--checked (romacs-agent-images--step state (read-key)) dir)))
              (plist-get state :result))
          (remove-hook 'window-size-change-functions follow)
          (romacs-layer-hide buffer)
          (romacs-layer-hide preview))))))

;;; The tray

(defconst romacs-agent-images--tray-thumbnail 40 "A tray thumbnail's height, in pixels.")

(defun romacs-agent-images--tray-text (names columns)
  "The tray's hint line for NAMES within COLUMNS."
  (truncate-string-to-width
   (format (substitute-command-keys
            "%d image%s · \\<romacs-agent-chat-mode-map>\\[romacs-agent-chat-remove-image] removes the last, N \\[romacs-agent-chat-remove-image] the Nth")
           (length names) (if (cdr names) "s" ""))
   columns nil nil "…"))

(defun romacs-agent-images--tray-draw (files columns)
  "Fill the current buffer, the tray, from FILES within COLUMNS."
  (let ((inhibit-read-only t) (index 0)
        (name-width (max 8 (- (/ columns (max 1 (length files))) 12))))
    (erase-buffer)
    (insert (propertize (romacs-agent-images--tray-text files columns) 'face 'shadow) "\n")
    (dolist (file files)
      (setq index (1+ index))
      (insert (propertize (format "%d " index) 'face 'minibuffer-prompt))
      (romacs-agent-images--insert-image file (* 2 romacs-agent-images--tray-thumbnail)
                                         romacs-agent-images--tray-thumbnail)
      (insert " " (truncate-string-to-width (file-name-nondirectory file) name-width nil nil "…") "  "))
    (goto-char (point-min))))

(defvar romacs-agent-images--tray-shown nil
  "What the tray shows: (FILES COLUMNS ANCHOR), or nil when hidden.")

(defun romacs-agent-images--tray-y (top bottom composer last line height)
  "Where a card of HEIGHT goes in a window from TOP to BOTTOM, whose composer
starts at COMPOSER and whose last row, LINE tall, starts at LAST: below it,
else above it and the rule over it, else nil: it never covers the composer."
  (let ((above (- composer line height romacs-agent-images--gap))
        (below (+ last (* 2 line) romacs-agent-images--gap)))
    (cond ((<= (+ below height) bottom) below)
          ((>= above top) above))))

(defun romacs-agent-images--tray-extent (top bottom start last)
  "The composer's rows as (START-Y . LAST-Y) in a window from TOP to BOTTOM,
START and LAST their seen y, nil when out of view.  A start out of view with
its last row seen is above the window; a last row out of view is below it."
  (cons (or start (if last top bottom)) (or last bottom)))

(defun romacs-agent-images--tray-anchor (window lines)
  "Where a tray of LINES goes by the current buffer's composer in WINDOW;
by its bottom when the composer is out of view; nil when it does not fit."
  (let* ((root (romacs-layer--root-frame))
         (edges (window-edges window t nil t))
         (padding (* 2 (plist-get (alist-get 'card romacs-popup-look) :padding)))
         (height (+ padding (* lines (frame-char-height root))))
         ;; Past the window's end is out of view: no position to measure,
         ;; so a composer far below costs a key nothing.
         (last-shown (window-end window))
         (y-of (lambda (position)
                 (let ((seen (and (<= position last-shown) (pos-visible-in-window-p position window t))))
                   (and seen (+ (nth 1 edges) (cadr seen))))))
         (extent (romacs-agent-images--tray-extent (nth 1 edges) (nth 3 edges)
                                                   (funcall y-of romacs-agent-ui--end)
                                                   (funcall y-of (point-max))))
         (y (romacs-agent-images--tray-y (nth 1 edges) (nth 3 edges) (car extent) (cdr extent)
                                         (default-line-height) height)))
    (and y (cons (+ (nth 0 edges) romacs-agent-images--gap) y))))

(defun romacs-agent-images--tray-files (window)
  "The images of the chat WINDOW shows, if it shows one."
  (let ((buffer (window-buffer window)))
    (and (eq (buffer-local-value 'major-mode buffer) 'romacs-agent-chat-mode)
         (buffer-local-value 'romacs-agent-ui--draft-images buffer))))

(defun romacs-agent-images-tray-sync (&rest _)
  "Make the one tray show the selected chat's images, or hide it.  It reads
the selected window, not its caller, so hooks may run in any order; and what
is shown already costs one comparison, as this runs after every key in a chat."
  (let* ((window (selected-window))
         (files (romacs-agent-images--tray-files window)))
    (when (or files romacs-agent-images--tray-shown)
      (let* ((buffer (get-buffer-create " *romacs-agent-tray*"))
             (columns (- (window-body-width window) 4))
             (lines (ceiling (+ (default-line-height) romacs-agent-images--tray-thumbnail) (frame-char-height)))
             (anchor (and files (with-current-buffer (window-buffer window)
                                  (romacs-agent-images--tray-anchor window lines))))
             (shown romacs-agent-images--tray-shown))
        (cond ((not anchor)
               (romacs-layer-hide buffer)
               (setq romacs-agent-images--tray-shown nil))
              ((not (and shown (eq (nth 0 shown) files) (eql (nth 1 shown) columns) (equal (nth 2 shown) anchor)))
               (with-current-buffer buffer
                 (setq mode-line-format nil cursor-type nil truncate-lines t)
                 (unless (and shown (eq (nth 0 shown) files) (eql (nth 1 shown) columns))
                   (romacs-agent-images--tray-draw files columns)))
               (romacs-popup-show 'card buffer :anchor anchor :columns columns :lines lines)
               (setq romacs-agent-images--tray-shown (list files columns anchor))))))))

;; Selecting, or replacing, a window's buffer moves the keyboard off a chat.
(add-hook 'window-selection-change-functions #'romacs-agent-images-tray-sync)
(add-hook 'window-buffer-change-functions #'romacs-agent-images-tray-sync)

(provide 'romacs-agent-images)

;;; romacs-agent-images.el ends here
