;;; cursor-effects-preview-test.el --- Preview Romacs cursor effects -*- lexical-binding: t -*-

;; Run with:
;;   ./target/release/romacs -Q -l test/romacs/cursor-effects-preview-test.el

(require 'cl-lib)
(require 'term/romacs-win nil t)

(defgroup cursor-effects-preview nil
  "Preview Romacs cursor animation effects."
  :group 'romacs)

(defcustom cursor-effects-preview-effect-seconds 2.2
  "Seconds to display each cursor effect before advancing."
  :type 'number
  :group 'cursor-effects-preview)

(defcustom cursor-effects-preview-move-seconds 1.0
  "Seconds between preview cursor moves."
  :type 'number
  :group 'cursor-effects-preview)

(defcustom cursor-effects-preview-min-frame-width 3200
  "Minimum frame pixel width requested for the cursor effect gallery."
  :type 'integer
  :group 'cursor-effects-preview)

(defcustom cursor-effects-preview-min-frame-height 2000
  "Minimum frame pixel height requested for the cursor effect gallery."
  :type 'integer
  :group 'cursor-effects-preview)

(defvar cursor-effects-preview--effect-timer nil)
(defvar cursor-effects-preview--move-timer nil)
(defvar cursor-effects-preview--index 0)
(defvar cursor-effects-preview--positions nil)
(defvar cursor-effects-preview--position-index 0)
(defvar cursor-effects-preview--effect-overlays nil)
(defvar cursor-effects-preview--paused nil)

(defconst cursor-effects-preview--effects
  (cl-loop for effect in (romacs-effect-names 'cursor)
           collect (list :name (string-replace "-" " "
                                               (string-remove-prefix
                                                "cursor-"
                                                (symbol-name effect)))
                         :effect (list effect :enabled t)))
  "Cursor effects discovered from the renderer's typed effect registry.")

(defconst cursor-effects-preview--cursor-types
  '((bar . 8))
  "Cursor shapes to combine with preview effects.")

(defvar cursor-effects-preview--gallery-timer nil)
(defvar-local cursor-effects-preview--move-tick 0)
(defvar-local cursor-effects-preview--visual-lines nil)
(defvar-local romacs-visual-cursors nil)

(defvar cursor-effects-preview-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "q") #'cursor-effects-preview-quit)
    map))

(define-derived-mode cursor-effects-preview-mode special-mode "Cursor-Effects"
  "Major mode for the Romacs cursor effects preview."
  (setq-local cursor-type nil)
  (setq-local truncate-lines t)
  (setq-local mode-line-format nil)
  (setq-local header-line-format nil)
  (setq-local tab-line-format nil))

(defun cursor-effects-preview--maximize-frame ()
  (setq frame-resize-pixelwise t)
  (setq window-resize-pixelwise t)
  (setq window-combination-resize nil)
  (setq window-min-height 1)
  (setq window-min-width 1)
  (set-frame-parameter nil 'fullscreen 'maximized)
  (when (fboundp 'set-frame-size)
    (let ((width (max (frame-pixel-width)
                      cursor-effects-preview-min-frame-width))
          (height (max (frame-pixel-height)
                       cursor-effects-preview-min-frame-height)))
      (set-frame-size (selected-frame) width height t))))

(defun cursor-effects-preview--line-offset (line tick)
  (let* ((width (plist-get line :width))
         (index (plist-get line :index))
         (period (+ 28 (mod (* index 7) 23)))
         (phase (+ tick (* index 11)))
         (step (mod phase (* 2 period))))
    (min (1- width)
         (if (< step period)
             step
           (- (* 2 period) step)))))

(defun cursor-effects-preview--refresh-visual-cursors (&optional buffer)
  (let ((buffer (or buffer (current-buffer))))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (let ((tick cursor-effects-preview--move-tick))
          (setq-local cursor-effects-preview--move-tick
                      (1+ cursor-effects-preview--move-tick))
          (setq-local
           romacs-visual-cursors
           (mapcar
            (lambda (line)
              (let* ((offset (cursor-effects-preview--line-offset line tick))
                     (position (+ (plist-get line :start) offset)))
                (list :position position
                      :cursor-type (plist-get line :cursor-type)
                      :effect (plist-get line :effect)
                      :color (plist-get line :color))))
            cursor-effects-preview--visual-lines)))
        (when (fboundp 'force-window-update)
          (force-window-update buffer))
        (redisplay t)))))

(defun cursor-effects-preview--insert-lines ()
  (let ((lines nil)
        (track "Make Emacs Great Again! Make Emacs Great Again!"))
    (cl-loop for effect in cursor-effects-preview--effects
             for index from 1
             for shape = (car cursor-effects-preview--cursor-types)
             for color = (if (= (mod index 2) 0) "#66CCFF" "#FF9966")
             do
             (insert (format "%02d  %-28s " index (plist-get effect :name)))
             (let ((start (point)))
               (insert track)
               (insert "\n")
               (push (list :index index
                           :start start
                           :width (length track)
                           :cursor-type shape
                           :effect (plist-get effect :effect)
                           :color color)
                     lines)))
    (setq-local cursor-effects-preview--visual-lines (nreverse lines))))

(defun cursor-effects-preview--make-buffer ()
  (let ((buffer (get-buffer-create "*Cursor Effects Preview*")))
    (with-current-buffer buffer
      (cursor-effects-preview-mode)
      (let ((inhibit-read-only t))
        (erase-buffer)
        (cursor-effects-preview--insert-lines)
        (goto-char (point-min)))
      (cursor-effects-preview--refresh-visual-cursors buffer))
    buffer))

(defun cursor-effects-preview--show-buffer (buffer)
  (delete-other-windows)
  (switch-to-buffer buffer)
  (goto-char (point-min)))

(defun cursor-effects-preview-stop ()
  "Stop timers and disable all cursor effects used by the preview."
  (interactive)
  (when (timerp cursor-effects-preview--gallery-timer)
    (cancel-timer cursor-effects-preview--gallery-timer))
  (setq cursor-effects-preview--gallery-timer nil))

(defun cursor-effects-preview-quit ()
  "Quit the cursor effects preview."
  (interactive)
  (cursor-effects-preview-stop)
  (quit-window t))

(defun cursor-effects-preview-start ()
  "Start the Romacs cursor effects preview."
  (interactive)
  (cursor-effects-preview--maximize-frame)
  (cursor-effects-preview-stop)
  (when (fboundp 'blink-cursor-mode)
    (blink-cursor-mode -1))
  (let ((buffer (cursor-effects-preview--make-buffer)))
    (with-current-buffer buffer
      (setq-local cursor-effects-preview--move-tick 0))
    (cursor-effects-preview--show-buffer buffer)
    (setq cursor-effects-preview--gallery-timer
          (run-at-time 0 cursor-effects-preview-move-seconds
                       #'cursor-effects-preview--refresh-visual-cursors
                       buffer))))

(cursor-effects-preview-start)

;;; cursor-effects-preview-test.el ends here
