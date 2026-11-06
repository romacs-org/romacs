;;; romacs-agent-images.el --- Pick an image by sight -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A centred menu of a directory's images, newest first: the selected
;; image large, below it a thumbnail and file name per row.  Typing filters
;; by file name, each word anywhere in it; C-n and C-p move, C-v and M-v
;; page, RET picks, TAB falls back to `read-file-name', C-g cancels.
;;
;; `romacs-complete' draws text rows, so this is a menu of its own.  A key
;; is a pure step from one state to the next.  Matches are found when the
;; query changes and indexed, and only the rows in view have images made
;; for them, so moving costs the same in any directory.

;;; Code:

(require 'seq)
(require 'romacs-popup)
(require 'romacs-agent)

(defconst romacs-agent-images--rows 5 "Rows shown at once.")
(defconst romacs-agent-images--columns 80 "The menu's width, in columns.")
(defconst romacs-agent-images--preview-height 300 "The preview's height, in pixels.")
(defconst romacs-agent-images--thumbnail-height 48 "A thumbnail's height, in pixels.")

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

(defun romacs-agent-images--insert-image (file height)
  "Insert FILE at HEIGHT pixels, when it loads and the display shows images."
  (when (display-images-p)
    (let ((image (ignore-errors
                   (create-image file nil nil :max-height height
                                 :max-width (* romacs-agent-images--columns (frame-char-width))))))
      (when image (insert-image image " ")))))

(defun romacs-agent-images--insert-row (file selected)
  "Insert the row of FILE, SELECTED or not."
  (let ((start (point)))
    (romacs-agent-images--insert-image file romacs-agent-images--thumbnail-height)
    (insert "  " (file-name-nondirectory file)
            (propertize (concat "  " (romacs-agent-images--age file)) 'face 'shadow) "\n")
    (when selected (add-face-text-property start (point) 'highlight))))

(defun romacs-agent-images--draw (state)
  "Fill the current buffer from STATE."
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
      (romacs-agent-images--insert-image (aref matches selected) romacs-agent-images--preview-height)
      (insert "\n")
      (dotimes (row (min romacs-agent-images--rows (- (length matches) first)))
        (romacs-agent-images--insert-row (aref matches (+ first row)) (= (+ first row) selected))))
    (goto-char (point-min))))

(defun romacs-agent-images--lines ()
  "The menu's height in lines: its text and images, in the frame's lines."
  (+ 2 (ceiling (+ romacs-agent-images--preview-height
                   (* romacs-agent-images--rows romacs-agent-images--thumbnail-height))
                (default-line-height))))

(defun romacs-agent-images--checked (state dir)
  "STATE, unless its result is a file gone meanwhile: then the menu again
over DIR as it is now, keeping the query."
  (let ((result (plist-get state :result)))
    (if (or (not (stringp result)) (romacs-agent-image-file-p result)) state
      (message "%s is gone" (file-name-nondirectory result))
      (romacs-agent-images--state (vconcat (romacs-agent-images--files dir)) (plist-get state :query)))))

(defun romacs-agent-images-pick (dir)
  "Choose an image of DIR by sight: its file, `browse' or `cancel'.  A
frame too short for the menu browses at once."
  (let* ((buffer (get-buffer-create " *romacs-agent-images*"))
         (state (if (> (romacs-agent-images--lines) (- (frame-text-lines) 4))
                    (list :result 'browse)
                  (romacs-agent-images--state (vconcat (romacs-agent-images--files dir)))))
         (lines (romacs-agent-images--lines))
         ;; Images stand taller than the lines a palette is usually held to.
         (romacs-layer-max-lines lines))
    (with-current-buffer buffer (setq mode-line-format nil cursor-type nil))
    (unwind-protect
        (let ((inhibit-quit t))
          (while (not (plist-get state :result))
            (with-current-buffer buffer (romacs-agent-images--draw state))
            (romacs-popup-show 'palette buffer :columns romacs-agent-images--columns :lines lines)
            (setq state (romacs-agent-images--checked (romacs-agent-images--step state (read-key)) dir)))
          (plist-get state :result))
      (romacs-layer-hide buffer))))

(provide 'romacs-agent-images)

;;; romacs-agent-images.el ends here
