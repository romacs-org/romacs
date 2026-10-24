;;; romacs-video.el --- Video playback support for Romacs -*- lexical-binding: t -*-

;; Copyright (C) 2024-2026 Free Software Foundation, Inc.

;; Author: Romacs Contributors
;; Keywords: multimedia, video

;; This file is part of GNU Emacs.

;; GNU Emacs is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; GNU Emacs is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with GNU Emacs.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; This package provides cross-platform native video playback for Romacs.
;; 
;; Basic usage:
;;   (romacs-video-play-file "/path/to/video.mp4")
;;
;; API functions:
;;   `romacs-video-load' - Open a video and return an opaque session handle
;;   `romacs-video-play' - Start playback
;;   `romacs-video-pause' - Pause playback
;;   `romacs-video-stop' - Stop playback
;;   `romacs-video-diagnostics' - Inspect observed path and pool telemetry
;;   `romacs-video-insert' - Insert video display at point

;;; Code:

(defgroup romacs-video nil
  "Native video playback in Romacs."
  :group 'multimedia)

(defcustom romacs-video-mode-size '(960 . 540)
  "Display size used when visiting a video file.
The value is a cons cell (WIDTH . HEIGHT) in pixels."
  :type '(cons (integer :tag "Width") (integer :tag "Height"))
  :group 'romacs-video)

(defvar romacs-video--players
  (make-hash-table :test 'eq :weakness 'key)
  "Weak table mapping live video handles to their UI metadata.
The table must not own sessions: display properties and callers do.  When the
last such reference disappears, GC closes the corresponding native session.")

(defvar-local romacs-video--buffer-handle nil
  "Video handle displayed by the current `romacs-video-mode' buffer.")

(defun romacs-video--normalize-source (file)
  "Return FILE as a URI or an absolute local filename."
  (if (string-match-p "\\`[[:alpha:]][[:alnum:]+.-]*://" file)
      file
    (expand-file-name file)))

;;;###autoload
(defun romacs-video-play-file (file)
  "Play video FILE in a new player.
Return the opaque video-session handle."
  (interactive "fVideo file: ")
  (let ((handle (romacs-video-insert file nil nil nil t)))
    (message "Playing video %S: %s" handle file)
    handle))

(defun romacs-video-toggle-pause (handle)
  "Toggle the pause state of video-session HANDLE."
  (interactive (list (or (romacs-video-at-point)
                         (user-error "No video at point"))))
  (let* ((info (gethash handle romacs-video--players))
         (state (plist-get info :state)))
    (if (eq state 'playing)
        (progn
          (romacs-video-pause handle)
          (puthash handle (plist-put info :state 'paused) romacs-video--players)
          (message "Video %S paused" handle))
      (romacs-video-play handle)
      (puthash handle (plist-put info :state 'playing) romacs-video--players)
      (message "Video %S playing" handle))))

(defun romacs-video-stop-all ()
  "Stop all playing videos."
  (interactive)
  (maphash (lambda (id _info)
             (romacs-video-stop id))
           romacs-video--players)
  (clrhash romacs-video--players)
  (message "All videos stopped"))

(defun romacs-video-insert (file &optional width height loop-count autoplay)
  "Insert video FILE at point with optional WIDTH and HEIGHT.
WIDTH and HEIGHT default to 640x360 if not specified.
LOOP-COUNT controls looping: nil or 0 means no loop, -1 means infinite,
positive N means loop N additional times.
AUTOPLAY if non-nil starts playback automatically.
Return the opaque video-session handle.  Point is left after the video."
  (interactive "fVideo file: ")
  (let* ((source (romacs-video--normalize-source file))
         (handle (romacs-video-load source (or loop-count 0) autoplay))
         (w (or width 640))
         (h (or height 360)))
    (puthash handle `(:source ,source
                      :state ,(if autoplay 'playing 'paused)
                      :width ,w :height ,h)
             romacs-video--players)
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display
                         `(video :id ,handle :width ,w :height ,h))
      (put-text-property start (point) 'romacs-video-id handle))
    (message "Inserted video %S" handle)
    handle))

(defun romacs-video--release-buffer-session ()
  "Immediately release the video session owned by the current buffer."
  (when (and romacs-video--buffer-handle
             (romacs-video-p romacs-video--buffer-handle))
    ;; Native destruction is idempotent.  GC remains the fallback if this hook
    ;; is bypassed (for example by an abnormal unwind).
    (ignore-errors (romacs-video-destroy romacs-video--buffer-handle))
    (remhash romacs-video--buffer-handle romacs-video--players)
    (setq romacs-video--buffer-handle nil))
  (let ((inhibit-read-only t)
        (buffer-undo-list t)
        (modified (buffer-modified-p)))
    (remove-list-of-text-properties
     (point-min) (point-max)
     '(display romacs-video-id read-only front-sticky rear-nonsticky))
    (set-buffer-modified-p modified)))

(defun romacs-video--display-visited-file ()
  "Display `buffer-file-name' as one native video session."
  (unless (and buffer-file-name (file-readable-p buffer-file-name))
    (error "Video buffer has no readable local file"))
  (when (zerop (buffer-size))
    (error "Cannot display an empty video file"))
  (romacs-video--release-buffer-session)
  (let* ((width (car romacs-video-mode-size))
         (height (cdr romacs-video-mode-size))
         (handle (romacs-video-load (expand-file-name buffer-file-name) 0 t))
         (inhibit-read-only t)
         (buffer-undo-list t)
         (modified (buffer-modified-p)))
    (setq romacs-video--buffer-handle handle)
    (puthash handle `(:source ,buffer-file-name :state playing
                      :width ,width :height ,height)
             romacs-video--players)
    ;; Follow GNU image-mode's file-buffer model: retain the original bytes so
    ;; saving/copying the buffer remains faithful, and visually replace the
    ;; complete byte range with one display object.
    (add-text-properties
     (point-min) (point-max)
     `(display (video :id ,handle :width ,width :height ,height)
               romacs-video-id ,handle
               read-only t
               front-sticky (read-only)
               rear-nonsticky (display romacs-video-id)))
    (set-buffer-modified-p modified)
    ;; Match image-mode: binary file buffers must not gain a newline or pass
    ;; through text encoding when saved under a different name.
    (when (coding-system-equal (coding-system-base buffer-file-coding-system)
                               'no-conversion)
      (setq-local find-file-literally t))))

(defvar-keymap romacs-video-mode-map
  :doc "Keymap for `romacs-video-mode'."
  "SPC" #'romacs-video-toggle-pause
  "p" #'romacs-video-play-at-point
  "s" #'romacs-video-stop-at-point)

(put 'romacs-video-mode 'mode-class 'special)

;;;###autoload
(define-derived-mode romacs-video-mode special-mode "Video"
  "Major mode for visiting video files with native playback.
`romacs-video--select-visited-video' enters this mode for the extensions in
`romacs-video-file-name-regexp', from `find-file-hook'.  The original file
bytes remain in the buffer underneath a single video display property,
following the lifecycle used by `image-mode'."
  (setq cursor-type nil
        truncate-lines t)
  (romacs-video--display-visited-file)
  (add-hook 'before-revert-hook #'romacs-video--release-buffer-session nil t)
  (add-hook 'after-revert-hook #'romacs-video--display-visited-file nil t)
  (add-hook 'change-major-mode-hook #'romacs-video--release-buffer-session nil t)
  (add-hook 'kill-buffer-hook #'romacs-video--release-buffer-session nil t))

(defvar romacs-video-file-name-regexp
  "\\.\\(?:avi\\|m4v\\|mkv\\|mov\\|mp4\\|mpeg\\|mpg\\|ogv\\|webm\\)\\'"
  "File names Romacs opens in `romacs-video-mode'.

This is the Romacs-only registry for those extensions, consulted by
`romacs-video--select-visited-video'.  They are deliberately NOT in
`auto-mode-alist': that variable is GNU's, and its contents are observable --
the parity suite pins its length and its `assq' results -- so an extension
GNU does not map would report a value GNU never produces.")

;;;###autoload
(defun romacs-video--select-visited-video ()
  "Select `romacs-video-mode' for a video file no other rule claimed.

Installed on `find-file-hook', so it runs once the mode for a visited file has
been chosen.  It claims only a buffer that GNU's own rules left in
`fundamental-mode', which keeps every explicit rule ahead of it: a
`-*- mode: ... -*-' tag, a `.dir-locals.el' entry, an `auto-mode-alist' match,
`magic-mode-alist', and `major-mode-remap-alist' all win.

The hook, rather than `set-auto-mode' advice, is deliberate: `loaddefs.el' is
read at loadup.el:175, long before `nadvice' is loaded at loadup.el:253, so an
autoloaded `advice-add' would be `void-function' during the dump.  `add-hook'
is available that early and never adds a duplicate, so this stays idempotent
however many times `romacs-video' is loaded."
  (when (and buffer-file-name
             (eq major-mode 'fundamental-mode)
             (string-match-p romacs-video-file-name-regexp buffer-file-name))
    (romacs-video-mode)))

;;;###autoload
(add-hook 'find-file-hook #'romacs-video--select-visited-video)

(defun romacs-video-insert-loop (file &optional width height)
  "Insert video FILE with infinite looping and autoplay.
WIDTH and HEIGHT default to 640x360 if not specified."
  (interactive "fVideo file: ")
  (romacs-video-insert file width height -1 t))

(defun romacs-video-at-point ()
  "Return the video-session handle at point, or nil if none."
  (or (get-text-property (point) 'romacs-video-id)
      romacs-video--buffer-handle))

(defun romacs-video-play-at-point ()
  "Start playing the video at point."
  (interactive)
  (let ((handle (romacs-video-at-point)))
    (if handle
        (progn
          (romacs-video-play handle)
          (message "Playing video %S" handle))
      (message "No video at point"))))

(defun romacs-video-pause-at-point ()
  "Pause the video at point."
  (interactive)
  (let ((handle (romacs-video-at-point)))
    (if handle
        (progn
          (romacs-video-pause handle)
          (message "Paused video %S" handle))
      (message "No video at point"))))

(defun romacs-video-stop-at-point ()
  "Stop the video at point."
  (interactive)
  (let ((handle (romacs-video-at-point)))
    (if handle
        (progn
          (romacs-video-stop handle)
          (let ((info (gethash handle romacs-video--players)))
            (when info
              (puthash handle (plist-put info :state 'stopped)
                       romacs-video--players)))
          (message "Stopped video %S" handle))
      (message "No video at point"))))

(defun romacs-video-show-floating (file &optional x y width height)
  "Show video FILE as a floating layer at X, Y with WIDTH, HEIGHT.
Defaults: X=50, Y=50, WIDTH=640, HEIGHT=360.
Return the opaque video-session handle."
  (interactive "fVideo file: ")
  (ignore file x y width height)
  (user-error "Floating video presentation is not implemented; use `romacs-video-insert'"))

(defun romacs-video-hide-floating (handle)
  "Hide the floating video layer for HANDLE and stop playback."
  (interactive (list (or (romacs-video-at-point)
                         (user-error "No video at point"))))
  (romacs-video-stop handle)
  (remhash handle romacs-video--players)
  (message "Video %S stopped" handle))

;;; Loop Control

(defun romacs-video-loop (handle &optional loop-count)
  "Enable loop playback for video-session HANDLE.
LOOP-COUNT can be:
  nil or t - infinite loop
  0 - no looping (disable loop)
  positive integer - loop that many times

Returns t on success."
  (interactive (list (or (romacs-video-at-point)
                         (user-error "No video at point"))
                     current-prefix-arg))
  (let ((count (cond
                ((null loop-count) -1)  ; Default: infinite
                ((eq loop-count t) -1)  ; t means infinite
                ((integerp loop-count) loop-count)
                (t -1))))               ; Fallback: infinite
    (when (romacs-video-set-loop handle count)
      ;; Update metadata
      (let ((info (gethash handle romacs-video--players)))
        (when info
          (puthash handle (plist-put info :loop count) romacs-video--players)))
      (if (= count 0)
          (message "Video %S loop: disabled" handle)
        (message "Video %S loop: %s" handle
                 (if (< count 0) "infinite" (format "%d times" count))))
      t)))

(defun romacs-video-loop-infinite (handle)
  "Enable infinite looping for video-session HANDLE."
  (interactive (list (or (romacs-video-at-point)
                         (user-error "No video at point"))))
  (romacs-video-loop handle t))

(defun romacs-video-loop-disable (handle)
  "Disable looping for video-session HANDLE."
  (interactive (list (or (romacs-video-at-point)
                         (user-error "No video at point"))))
  (romacs-video-loop handle 0))

(defun romacs-video-show-floating-loop (file &optional x y width height)
  "Show video FILE as floating and loop infinitely.
Like `romacs-video-show-floating' but with automatic looping."
  (interactive "fVideo file: ")
  (let ((handle (romacs-video-show-floating file x y width height)))
    (when handle
      (romacs-video-loop handle t))
    handle))

(provide 'romacs-video)
;;; romacs-video.el ends here
