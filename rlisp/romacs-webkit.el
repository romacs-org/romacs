;;; romacs-webkit.el --- WebKit browser support for Romacs -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; This package provides WebKit browser embedding support for Romacs.
;; It uses WPE WebKit for headless rendering with GPU acceleration.
;;
;; Basic usage:
;;   (romacs-webkit-browse "https://example.com")
;;
;; API functions:
;;   `romacs-webkit-browse' - Open URL in a new browser view
;;   `romacs-webkit-back' - Go back in history
;;   `romacs-webkit-forward' - Go forward in history
;;   `romacs-webkit-reload' - Reload current page
;;   `romacs-webkit-eval-js' - Execute JavaScript

;;; Code:

(defgroup romacs-webkit nil
  "WebKit browser embedding for Romacs."
  :group 'web
  :prefix "romacs-webkit-")

(defcustom romacs-webkit-default-width 800
  "Default width for WebKit views.
If set to 0 or nil, auto-calculate from window width."
  :type '(choice integer (const nil))
  :group 'romacs-webkit)

(defcustom romacs-webkit-default-height 600
  "Default height for WebKit views.
If set to 0 or nil, auto-calculate from width using aspect ratio."
  :type '(choice integer (const nil))
  :group 'romacs-webkit)

(defcustom romacs-webkit-default-aspect-ratio (/ 16.0 9.0)
  "Default aspect ratio (width/height) for auto-calculated heights."
  :type 'float
  :group 'romacs-webkit)

(defcustom romacs-webkit-auto-resize t
  "If non-nil, automatically resize webkit views when window size changes."
  :type 'boolean
  :group 'romacs-webkit)

(defvar romacs-webkit--views (make-hash-table :test 'eq)
  "Hash table mapping view IDs to their metadata.")

(defvar-local romacs-webkit--buffer-views nil
  "List of (POSITION . VIEW-ID) for webkit views in this buffer.
Used for responsive auto-resize on window size changes.")

(defvar romacs-webkit--initialized nil
  "Whether the WebKit subsystem has been initialized.")

(defcustom romacs-webkit-new-window-action 'new-buffer
  "Action to take when WebKit requests a new window.
Possible values:
  `new-buffer' - Open URL in a new webkit buffer (default)
  `same-buffer' - Load URL in the current webkit view
  `external' - Open URL in external browser
  `ignore' - Ignore the request
  `ask' - Ask user what to do"
  :type '(choice (const :tag "Open in new buffer" new-buffer)
                 (const :tag "Load in same view" same-buffer)
                 (const :tag "Open in external browser" external)
                 (const :tag "Ignore" ignore)
                 (const :tag "Ask user" ask))
  :group 'romacs-webkit)

(defun romacs-webkit--handle-new-window (view-id url frame-name)
  "Handle a new window request from WebKit VIEW-ID.
URL is the requested URL, FRAME-NAME is the target attribute (e.g., \"_blank\")."
  (message "WebKit new window request: view=%d url=%s frame=%s"
           view-id url frame-name)
  (pcase romacs-webkit-new-window-action
    ('new-buffer
     ;; Open URL in a new webkit buffer
     (let ((new-view-id (romacs-webkit-browse url)))
       (when new-view-id
         (message "Opened %s in new webkit view %d" url new-view-id)))
     t)
    ('same-buffer
     ;; Load URL in the same view
     (romacs-webkit-load-uri view-id url)
     (message "Loading %s in view %d" url view-id)
     t)
    ('external
     ;; Open in external browser
     (browse-url url)
     (message "Opened %s in external browser" url)
     t)
    ('ignore
     ;; Ignore the request
     (message "Ignored new window request for %s" url)
     nil)
    ('ask
     ;; Ask user what to do
     (let ((choice (completing-read
                    (format "New window request for %s: " url)
                    '("Open in new buffer" "Load in same view"
                      "Open in external browser" "Ignore")
                    nil t)))
       (pcase choice
         ("Open in new buffer"
          (romacs-webkit-browse url)
          t)
         ("Load in same view"
          (romacs-webkit-load-uri view-id url)
          t)
         ("Open in external browser"
          (browse-url url)
          t)
         (_
          nil))))
    (_
     ;; Default: open in new buffer
     (romacs-webkit-browse url)
     t)))

(defun romacs-webkit--ensure-initialized ()
  "Ensure the WebKit subsystem is initialized."
  (unless romacs-webkit--initialized
    (when (romacs-webkit-init)
      ;; Set up the new window callback
      (romacs-webkit-set-new-window-function #'romacs-webkit--handle-new-window)
      (setq romacs-webkit--initialized t))))

(defun romacs-webkit--calculate-dimensions (width height)
  "Calculate actual dimensions from WIDTH and HEIGHT.
If WIDTH is 0 or nil, use window body width minus margin.
If HEIGHT is 0 or nil, calculate from width using aspect ratio.
Returns (WIDTH . HEIGHT) cons cell."
  (let* ((margin 16)
         (w (if (and width (> width 0))
                width
              (- (window-body-width nil t) margin)))
         (h (if (and height (> height 0))
                height
              (round (/ w romacs-webkit-default-aspect-ratio)))))
    (cons w h)))

(defun romacs-webkit-browse (url &optional width height)
  "Open URL in a new WebKit browser view.
Optional WIDTH and HEIGHT specify view dimensions.
If WIDTH is 0 or nil, auto-calculate from window width.
If HEIGHT is 0 or nil, auto-calculate using aspect ratio.
Returns the view ID on success, nil on failure."
  (interactive "sURL: ")
  (romacs-webkit--ensure-initialized)
  (let* ((dims (romacs-webkit--calculate-dimensions
                (or width romacs-webkit-default-width)
                (or height romacs-webkit-default-height)))
         (w (car dims))
         (h (cdr dims))
         (view-id (romacs-webkit-create w h)))
    (when view-id
      (romacs-webkit-load-uri view-id url)
      (puthash view-id `(:url ,url :width ,w :height ,h) romacs-webkit--views)
      (message "WebKit view %d: %s (%dx%d)" view-id url w h))
    view-id))

(defun romacs-webkit-back (view-id)
  "Go back in history for VIEW-ID."
  (interactive "nView ID: ")
  (romacs-webkit-go-back view-id))

(defun romacs-webkit-forward (view-id)
  "Go forward in history for VIEW-ID."
  (interactive "nView ID: ")
  (romacs-webkit-go-forward view-id))

(defun romacs-webkit-eval-js (view-id script)
  "Execute JavaScript SCRIPT in VIEW-ID."
  (interactive "nView ID: \nsJavaScript: ")
  (romacs-webkit-execute-js view-id script))

(defun romacs-webkit-close (view-id)
  "Close WebKit view VIEW-ID."
  (interactive "nView ID: ")
  (when (romacs-webkit-destroy view-id)
    (remhash view-id romacs-webkit--views)
    (message "WebKit view %d closed" view-id)))

(defun romacs-webkit-resize-view (view-id width height)
  "Resize WebKit view VIEW-ID to WIDTH x HEIGHT pixels.
Returns t on success, nil on failure."
  (interactive "nView ID: \nnWidth: \nnHeight: ")
  (when (romacs-webkit-resize view-id width height)
    ;; Update stored dimensions
    (let ((info (gethash view-id romacs-webkit--views)))
      (when info
        (plist-put info :width width)
        (plist-put info :height height)))
    t))

(defun romacs-webkit--register-view (pos view-id)
  "Register webkit VIEW-ID at buffer position POS for auto-resize tracking."
  (unless romacs-webkit--buffer-views
    (setq romacs-webkit--buffer-views nil))
  (push (cons pos view-id) romacs-webkit--buffer-views))

(defun romacs-webkit--update-display-spec (pos view-id new-width new-height)
  "Update the display spec at POS for VIEW-ID with new dimensions."
  (save-excursion
    (goto-char pos)
    (let ((spec (get-text-property pos 'display)))
      (when (and spec (eq (car spec) 'webkit))
        ;; Update the spec with new dimensions
        (let ((new-spec (list 'webkit
                              :id view-id
                              :width new-width
                              :height new-height)))
          (put-text-property pos (1+ pos) 'display new-spec))))))

(defun romacs-webkit--auto-resize-buffer (window)
  "Resize all webkit views in WINDOW's buffer to fit the new window width."
  (when romacs-webkit-auto-resize
    (with-current-buffer (window-buffer window)
      (when romacs-webkit--buffer-views
        (let* ((margin 16)
               (new-width (- (window-body-width window t) margin))
               (new-height (round (/ new-width romacs-webkit-default-aspect-ratio))))
          (dolist (entry romacs-webkit--buffer-views)
            (let ((pos (car entry))
                  (view-id (cdr entry)))
              ;; Resize the webkit view
              (when (romacs-webkit-resize view-id new-width new-height)
                ;; Update the display spec in the buffer
                (romacs-webkit--update-display-spec pos view-id new-width new-height)
                ;; Update stored metadata
                (let ((info (gethash view-id romacs-webkit--views)))
                  (when info
                    (plist-put info :width new-width)
                    (plist-put info :height new-height)))))))))))

(defun romacs-webkit--window-size-change-hook (frame)
  "Hook function for `window-size-change-functions'.
Resizes webkit views in all windows of FRAME."
  (dolist (window (window-list frame))
    (romacs-webkit--auto-resize-buffer window)))

(defun romacs-webkit-enable-auto-resize ()
  "Enable automatic webkit view resizing on window size changes."
  (interactive)
  (add-hook 'window-size-change-functions #'romacs-webkit--window-size-change-hook)
  (setq romacs-webkit-auto-resize t)
  (message "WebKit auto-resize enabled"))

(defun romacs-webkit-disable-auto-resize ()
  "Disable automatic webkit view resizing on window size changes."
  (interactive)
  (remove-hook 'window-size-change-functions #'romacs-webkit--window-size-change-hook)
  (setq romacs-webkit-auto-resize nil)
  (message "WebKit auto-resize disabled"))

(defun romacs-webkit-close-all ()
  "Close all WebKit views."
  (interactive)
  (maphash (lambda (id _info)
             (romacs-webkit-destroy id))
           romacs-webkit--views)
  (clrhash romacs-webkit--views)
  (message "All WebKit views closed"))

(defun romacs-webkit-show-floating (view-id x y &optional width height)
  "Show VIEW-ID as floating overlay at position (X, Y).
Optional WIDTH and HEIGHT override stored dimensions."
  (interactive "nView ID: \nnX position: \nnY position: ")
  (let* ((info (gethash view-id romacs-webkit--views))
         (w (or width (plist-get info :width) romacs-webkit-default-width))
         (h (or height (plist-get info :height) romacs-webkit-default-height)))
    (romacs-webkit-floating view-id x y w h)))

(defun romacs-webkit-hide-floating (view-id)
  "Hide floating overlay for VIEW-ID."
  (interactive "nView ID: ")
  (romacs-webkit-floating-clear view-id))

;;; Browser buffer mode

(defvar-local romacs-webkit-buffer-view-id nil
  "The WebKit view ID associated with this buffer.")

(defvar romacs-webkit-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map "g" #'romacs-webkit-mode-reload)
    (define-key map "r" #'romacs-webkit-mode-reload)
    (define-key map "B" #'romacs-webkit-mode-back)
    (define-key map "F" #'romacs-webkit-mode-forward)
    (define-key map "q" #'romacs-webkit-mode-quit)
    (define-key map "o" #'romacs-webkit-mode-open)
    (define-key map "i" #'romacs-webkit-mode-toggle-input)  ; Enter input mode
    (define-key map [mouse-1] #'romacs-webkit-mode-mouse-click)
    (define-key map [wheel-up] #'romacs-webkit-mode-scroll-up)
    (define-key map [wheel-down] #'romacs-webkit-mode-scroll-down)
    map)
  "Keymap for `romacs-webkit-mode'.
Press 'i' to enter input mode for typing in web forms.")

(defvar romacs-webkit-mode-line-format
  '(:eval (romacs-webkit--mode-line-string))
  "Mode line format for WebKit buffers.")

(defun romacs-webkit--mode-line-string ()
  "Generate mode line string for WebKit buffer."
  (if romacs-webkit-buffer-view-id
      (let ((title (romacs-webkit-get-title romacs-webkit-buffer-view-id))
            (progress (romacs-webkit-get-progress romacs-webkit-buffer-view-id))
            (loading (romacs-webkit-loading-p romacs-webkit-buffer-view-id)))
        (format " [%s%s]"
                (or title "WebKit")
                (if loading
                    (format " %.0f%%" (* 100 (or progress 0)))
                  "")))
    " [WebKit]"))

(define-derived-mode romacs-webkit-mode special-mode "WebKit"
  "Major mode for browsing the web with WebKit.
\\{romacs-webkit-mode-map}"
  (setq buffer-read-only t)
  (setq mode-line-format
        (list "%e" mode-line-front-space
              romacs-webkit-mode-line-format
              " " mode-line-buffer-identification
              mode-line-end-spaces)))

(defun romacs-webkit-mode-reload ()
  "Reload the current page."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-reload romacs-webkit-buffer-view-id)
    (message "Reloading...")))

(defun romacs-webkit-mode-back ()
  "Go back in history."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-go-back romacs-webkit-buffer-view-id)))

(defun romacs-webkit-mode-forward ()
  "Go forward in history."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-go-forward romacs-webkit-buffer-view-id)))

(defun romacs-webkit-mode-quit ()
  "Close the WebKit browser buffer."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-floating-clear romacs-webkit-buffer-view-id)
    (romacs-webkit-close romacs-webkit-buffer-view-id)
    (setq romacs-webkit-buffer-view-id nil))
  (kill-buffer))

(defun romacs-webkit-mode-open (url)
  "Open URL in the current WebKit view."
  (interactive "sURL: ")
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-load-uri romacs-webkit-buffer-view-id url)
    (message "Loading: %s" url)))

(defvar-local romacs-webkit--floating-position nil
  "Position and size of floating WebKit view (x y width height).")

(defun romacs-webkit-mode-mouse-click (event)
  "Handle mouse click EVENT in WebKit view."
  (interactive "e")
  (when (and romacs-webkit-buffer-view-id romacs-webkit--floating-position)
    (let* ((pos (posn-x-y (event-start event)))
           (click-x (car pos))
           (click-y (cdr pos))
           (float-x (nth 0 romacs-webkit--floating-position))
           (float-y (nth 1 romacs-webkit--floating-position))
           ;; Translate to WebKit view coordinates
           (view-x (- click-x float-x))
           (view-y (- click-y float-y)))
      (romacs-webkit-click romacs-webkit-buffer-view-id view-x view-y 1))))

(defun romacs-webkit-mode-scroll-up (event)
  "Handle scroll up EVENT in WebKit view."
  (interactive "e")
  (when (and romacs-webkit-buffer-view-id romacs-webkit--floating-position)
    (let* ((pos (posn-x-y (event-start event)))
           (float-x (nth 0 romacs-webkit--floating-position))
           (float-y (nth 1 romacs-webkit--floating-position))
           (view-x (- (car pos) float-x))
           (view-y (- (cdr pos) float-y)))
      (romacs-webkit-send-scroll romacs-webkit-buffer-view-id view-x view-y 0 -50))))

(defun romacs-webkit-mode-scroll-down (event)
  "Handle scroll down EVENT in WebKit view."
  (interactive "e")
  (when (and romacs-webkit-buffer-view-id romacs-webkit--floating-position)
    (let* ((pos (posn-x-y (event-start event)))
           (float-x (nth 0 romacs-webkit--floating-position))
           (float-y (nth 1 romacs-webkit--floating-position))
           (view-x (- (car pos) float-x))
           (view-y (- (cdr pos) float-y)))
      (romacs-webkit-send-scroll romacs-webkit-buffer-view-id view-x view-y 0 50))))

;; Keyboard input support for text entry in web forms

(defvar romacs-webkit--input-mode nil
  "Non-nil when input mode is active (for typing in web forms).")

(defun romacs-webkit-mode-toggle-input ()
  "Toggle input mode for typing in web forms.
When enabled, most keys are forwarded to the WebKit view."
  (interactive)
  (setq romacs-webkit--input-mode (not romacs-webkit--input-mode))
  (if romacs-webkit--input-mode
      (progn
        (message "WebKit input mode ON - type to enter text, C-c C-c to exit")
        (romacs-webkit--enable-input-mode))
    (message "WebKit input mode OFF")
    (romacs-webkit--disable-input-mode)))

(defun romacs-webkit--enable-input-mode ()
  "Enable input mode keybindings."
  ;; Use a minor mode map that captures most keys
  (setq-local romacs-webkit--saved-keymap (current-local-map))
  (use-local-map romacs-webkit-input-mode-map))

(defun romacs-webkit--disable-input-mode ()
  "Disable input mode keybindings."
  (when romacs-webkit--saved-keymap
    (use-local-map romacs-webkit--saved-keymap)
    (setq-local romacs-webkit--saved-keymap nil)))

(defvar romacs-webkit--saved-keymap nil
  "Saved keymap before entering input mode.")

(defun romacs-webkit--char-to-keysym (char)
  "Convert CHAR to XKB keysym."
  (cond
   ;; Letters
   ((and (>= char ?a) (<= char ?z)) char)
   ((and (>= char ?A) (<= char ?Z)) char)
   ;; Numbers
   ((and (>= char ?0) (<= char ?9)) char)
   ;; Space
   ((= char ?\s) #x0020)
   ;; Common punctuation
   ((= char ?.) #x002e)
   ((= char ?,) #x002c)
   ((= char ?!) #x0021)
   ((= char ??) #x003f)
   ((= char ?:) #x003a)
   ((= char ?\;) #x003b)
   ((= char ?') #x0027)
   ((= char ?\") #x0022)
   ((= char ?-) #x002d)
   ((= char ?_) #x005f)
   ((= char ?=) #x003d)
   ((= char ?+) #x002b)
   ((= char ?/) #x002f)
   ((= char ?\\) #x005c)
   ((= char ?@) #x0040)
   ((= char ?#) #x0023)
   ((= char ?$) #x0024)
   ((= char ?%) #x0025)
   ((= char ?^) #x005e)
   ((= char ?&) #x0026)
   ((= char ?*) #x002a)
   ((= char ?\() #x0028)
   ((= char ?\)) #x0029)
   ((= char ?\[) #x005b)
   ((= char ?\]) #x005d)
   ((= char ?{) #x007b)
   ((= char ?}) #x007d)
   ((= char ?<) #x003c)
   ((= char ?>) #x003e)
   ((= char ?|) #x007c)
   ((= char ?`) #x0060)
   ((= char ?~) #x007e)
   ;; Default: use char code as keysym
   (t char)))

(defun romacs-webkit-input-self-insert ()
  "Forward current key to WebKit as text input."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (let* ((char last-command-event)
           (keysym (romacs-webkit--char-to-keysym char))
           (modifiers 0))
      ;; Send key down then key up
      (romacs-webkit-send-key romacs-webkit-buffer-view-id keysym 0 t modifiers)
      (romacs-webkit-send-key romacs-webkit-buffer-view-id keysym 0 nil modifiers))))

(defun romacs-webkit-input-return ()
  "Send Return/Enter to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff0d 36 t 0)  ; XK_Return
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff0d 36 nil 0)))

(defun romacs-webkit-input-backspace ()
  "Send Backspace to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff08 22 t 0)  ; XK_BackSpace
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff08 22 nil 0)))

(defun romacs-webkit-input-tab ()
  "Send Tab to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff09 23 t 0)  ; XK_Tab
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff09 23 nil 0)))

(defun romacs-webkit-input-escape ()
  "Send Escape to WebKit and exit input mode."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff1b 9 t 0)  ; XK_Escape
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff1b 9 nil 0))
  (romacs-webkit-mode-toggle-input))

(defun romacs-webkit-input-arrow-up ()
  "Send Up arrow to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff52 111 t 0)
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff52 111 nil 0)))

(defun romacs-webkit-input-arrow-down ()
  "Send Down arrow to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff54 116 t 0)
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff54 116 nil 0)))

(defun romacs-webkit-input-arrow-left ()
  "Send Left arrow to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff51 113 t 0)
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff51 113 nil 0)))

(defun romacs-webkit-input-arrow-right ()
  "Send Right arrow to WebKit."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff53 114 t 0)
    (romacs-webkit-send-key romacs-webkit-buffer-view-id #xff53 114 nil 0)))

(defvar romacs-webkit-input-mode-map
  (let ((map (make-keymap)))
    ;; Self-insert for printable characters
    (dotimes (i 128)
      (when (and (>= i 32) (/= i 127))  ; printable ASCII
        (define-key map (vector i) #'romacs-webkit-input-self-insert)))
    ;; Special keys
    (define-key map (kbd "RET") #'romacs-webkit-input-return)
    (define-key map (kbd "DEL") #'romacs-webkit-input-backspace)
    (define-key map (kbd "<backspace>") #'romacs-webkit-input-backspace)
    (define-key map (kbd "TAB") #'romacs-webkit-input-tab)
    (define-key map (kbd "<up>") #'romacs-webkit-input-arrow-up)
    (define-key map (kbd "<down>") #'romacs-webkit-input-arrow-down)
    (define-key map (kbd "<left>") #'romacs-webkit-input-arrow-left)
    (define-key map (kbd "<right>") #'romacs-webkit-input-arrow-right)
    ;; Exit input mode
    (define-key map (kbd "ESC") #'romacs-webkit-input-escape)
    (define-key map (kbd "C-c C-c") #'romacs-webkit-mode-toggle-input)
    ;; Keep mouse bindings
    (define-key map [mouse-1] #'romacs-webkit-mode-mouse-click)
    (define-key map [wheel-up] #'romacs-webkit-mode-scroll-up)
    (define-key map [wheel-down] #'romacs-webkit-mode-scroll-down)
    map)
  "Keymap for WebKit input mode (typing in web forms).")

(defun romacs-webkit-mode-show-fullscreen ()
  "Show WebKit view as fullscreen floating overlay."
  (interactive)
  (when romacs-webkit-buffer-view-id
    (let* ((frame (selected-frame))
           (width (frame-pixel-width frame))
           (height (frame-pixel-height frame)))
      (setq romacs-webkit--floating-position (list 0 0 width height))
      (romacs-webkit-floating romacs-webkit-buffer-view-id 0 0 width height))))

;;;###autoload
(defun romacs-webkit-open-url (url)
  "Open URL in a new WebKit browser buffer."
  (interactive "sURL: ")
  (romacs-webkit--ensure-initialized)
  (let* ((buffer (generate-new-buffer "*WebKit*"))
         (view-id (romacs-webkit-browse url)))
    (when view-id
      (with-current-buffer buffer
        (romacs-webkit-mode)
        (setq romacs-webkit-buffer-view-id view-id)
        (rename-buffer (format "*WebKit: %s*" url) t))
      (pop-to-buffer buffer)
      (message "WebKit browser opened: %s" url))
    buffer))

;;; Cursor-aware inline WebKit input mode
;;
;; Keyboard input only reaches an inline WebKit view when:
;; 1. The cursor is on the WebKit character (has `romacs-webkit-id' text prop)
;; 2. Input mode is explicitly toggled ON for that view
;;
;; Moving the cursor off the WebKit character auto-deactivates input mode.
;; Clicking on a WebKit view (C sets `romacs-webkit-clicked-view-id') also
;; enters input mode automatically.

(defvar romacs-webkit-debug nil
  "When non-nil, log WebKit input routing events to *Messages*.")

(defun romacs-webkit--log (fmt &rest args)
  "Log FMT with ARGS if `romacs-webkit-debug' is non-nil."
  (when romacs-webkit-debug
    (apply #'message (concat "[webkit-input] " fmt) args)))

(defvar-local romacs-webkit--input-view-id nil
  "View ID of the inline WebKit view currently accepting keyboard input, or nil.")

(defvar romacs-webkit--input-active-buffer nil
  "Buffer where WebKit input mode is currently active, or nil.
Used to deactivate input mode when the user switches to a different
buffer or frame.")

(defvar romacs-webkit--deactivation-timer nil
  "Idle timer that checks for cross-buffer WebKit input deactivation.")

(defun romacs-webkit--check-deactivation ()
  "Idle timer callback: deactivate WebKit input if user left the buffer."
  (when (and romacs-webkit--input-active-buffer
             (not (eq (current-buffer) romacs-webkit--input-active-buffer)))
    (let ((old-buf romacs-webkit--input-active-buffer))
      (romacs-webkit--log "idle: left webkit buffer, deactivating in %s" old-buf)
      (when (buffer-live-p old-buf)
        (with-current-buffer old-buf
          (romacs-webkit--deactivate-input))))))

;;; Hover keymap — active when cursor is on a WebKit character but input is OFF

(defvar romacs-webkit-hover-keymap
  (let ((map (make-sparse-keymap)))
    (define-key map "i" #'romacs-webkit-enter-input-mode)
    (define-key map (kbd "RET") #'romacs-webkit-enter-input-mode)
    map)
  "Keymap active when cursor is on a WebKit character but input mode is off.
Press `i' or RET to enter input mode for this view.")

;;; romacs-webkit-insert — Lisp wrapper that adds text properties

(defun romacs-webkit-insert (url width height &optional load-p)
  "Insert inline WebKit view at point.  Returns view-id or nil.
Calls the C function `romacs-insert-webkit' to create the view,
then inserts a propertized character with `romacs-webkit-id' and
`keymap' text properties for cursor-aware input routing.

URL, WIDTH, HEIGHT, and LOAD-P are passed to `romacs-insert-webkit'."
  (let ((spec (romacs-insert-webkit url width height load-p)))
    (when spec
      (let ((view-id (plist-get (cdr spec) :id)))
        (insert (propertize " "
                  'display spec
                  'romacs-webkit-id view-id
                  'keymap romacs-webkit-hover-keymap))
        view-id))))

;;; Interaction mode keymap — active when input mode is ON

(defvar romacs-webkit-interaction-mode-map
  (let ((map (make-sparse-keymap)))
    ;; Self-insert for printable ASCII
    (dotimes (i 128)
      (when (and (>= i 32) (/= i 127))
        (define-key map (vector i) #'romacs-webkit-interaction-self-insert)))
    ;; Special keys
    (define-key map (kbd "RET") #'romacs-webkit-interaction-return)
    (define-key map (kbd "DEL") #'romacs-webkit-interaction-backspace)
    (define-key map (kbd "<backspace>") #'romacs-webkit-interaction-backspace)
    (define-key map (kbd "TAB") #'romacs-webkit-interaction-tab)
    (define-key map (kbd "<up>") #'romacs-webkit-interaction-arrow-up)
    (define-key map (kbd "<down>") #'romacs-webkit-interaction-arrow-down)
    (define-key map (kbd "<left>") #'romacs-webkit-interaction-arrow-left)
    (define-key map (kbd "<right>") #'romacs-webkit-interaction-arrow-right)
    ;; Exit
    (define-key map (kbd "C-g") #'romacs-webkit-interaction-quit)
    (define-key map (kbd "ESC") #'romacs-webkit-interaction-quit)
    (define-key map (kbd "C-c C-c") #'romacs-webkit-interaction-quit)
    map)
  "Keymap active when typing into an inline WebKit view.")

;;; Minor mode

(define-minor-mode romacs-webkit-interaction-mode
  "Minor mode for keyboard input to inline WebKit views.
Activated when entering input mode on a WebKit character.
Press ESC, C-g, or C-c C-c to exit."
  :lighter " WebKit-Input"
  (if romacs-webkit-interaction-mode
      (progn
        (romacs-webkit--log "input ON for view %s" romacs-webkit--input-view-id)
        (message "WebKit input active (view %s) — ESC to exit"
                 romacs-webkit--input-view-id))
    (romacs-webkit--log "input OFF (was view %s)" romacs-webkit--input-view-id)
    (setq romacs-webkit--input-view-id nil)
    (message "WebKit input off")))

;;; Input activation / deactivation

(defun romacs-webkit--activate-input (view-id)
  "Activate input mode for VIEW-ID."
  (setq romacs-webkit--input-view-id view-id)
  (setq romacs-webkit--input-active-buffer (current-buffer))
  ;; Start idle timer to detect cross-buffer/frame switches
  (unless romacs-webkit--deactivation-timer
    (setq romacs-webkit--deactivation-timer
          (run-with-idle-timer 0.1 t #'romacs-webkit--check-deactivation)))
  (romacs-webkit-interaction-mode 1))

(defun romacs-webkit--deactivate-input ()
  "Deactivate input mode."
  (setq romacs-webkit--input-active-buffer nil)
  ;; Cancel the idle timer
  (when romacs-webkit--deactivation-timer
    (cancel-timer romacs-webkit--deactivation-timer)
    (setq romacs-webkit--deactivation-timer nil))
  (romacs-webkit-interaction-mode -1))

(defun romacs-webkit-enter-input-mode ()
  "Enter input mode for the WebKit view at point.
Reads the `romacs-webkit-id' text property to determine which view."
  (interactive)
  (let ((view-id (get-text-property (point) 'romacs-webkit-id)))
    (if view-id
        (romacs-webkit--activate-input view-id)
      (romacs-webkit--log "enter-input-mode: no webkit-id at point")
      (message "No WebKit view at point"))))

;;; Key forwarding commands

(defun romacs-webkit-interaction-self-insert ()
  "Forward character to the focused inline WebKit view."
  (interactive)
  (when romacs-webkit--input-view-id
    (let* ((char last-command-event)
           (keysym (romacs-webkit--char-to-keysym char)))
      (romacs-webkit-send-key romacs-webkit--input-view-id keysym 0 t 0)
      (romacs-webkit-send-key romacs-webkit--input-view-id keysym 0 nil 0))))

(defun romacs-webkit-interaction--send (keysym keycode)
  "Send KEYSYM with KEYCODE to focused webkit view."
  (when romacs-webkit--input-view-id
    (romacs-webkit-send-key romacs-webkit--input-view-id keysym keycode t 0)
    (romacs-webkit-send-key romacs-webkit--input-view-id keysym keycode nil 0)))

(defun romacs-webkit-interaction-return ()
  "Send Return to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff0d 36))

(defun romacs-webkit-interaction-backspace ()
  "Send Backspace to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff08 22))

(defun romacs-webkit-interaction-tab ()
  "Send Tab to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff09 23))

(defun romacs-webkit-interaction-arrow-up ()
  "Send Up to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff52 111))

(defun romacs-webkit-interaction-arrow-down ()
  "Send Down to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff54 116))

(defun romacs-webkit-interaction-arrow-left ()
  "Send Left to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff51 113))

(defun romacs-webkit-interaction-arrow-right ()
  "Send Right to WebKit." (interactive)
  (romacs-webkit-interaction--send #xff53 114))

(defun romacs-webkit-interaction-quit ()
  "Exit WebKit interaction mode."
  (interactive)
  (romacs-webkit--deactivate-input))

;;; Post-command hook — click-to-activate and auto-deactivate

(defun romacs-webkit--post-command-check ()
  "Post-command hook for WebKit input routing.
Handles three concerns:
1. Click-to-activate: when C sets `romacs-webkit-clicked-view-id',
   auto-enter input mode if point is on that view.
2. Auto-deactivate (same buffer): if input mode is active but cursor
   has moved off the view, deactivate.
3. Auto-deactivate (buffer/frame switch): if user switched to a
   different buffer or frame, deactivate input in the original buffer."
  ;; 1. Click-to-activate
  (when (and (boundp 'romacs-webkit-clicked-view-id)
             romacs-webkit-clicked-view-id
             (integerp romacs-webkit-clicked-view-id)
             (> romacs-webkit-clicked-view-id 0))
    (let ((clicked-id romacs-webkit-clicked-view-id))
      (setq romacs-webkit-clicked-view-id nil)
      (romacs-webkit--log "click detected: view %d" clicked-id)
      ;; Only activate if point is on that view's character
      (when (eq (get-text-property (point) 'romacs-webkit-id) clicked-id)
        (romacs-webkit--activate-input clicked-id))))
  ;; 2. Auto-deactivate (same buffer): cursor left the input-mode view
  (when romacs-webkit--input-view-id
    (unless (eq (get-text-property (point) 'romacs-webkit-id)
                romacs-webkit--input-view-id)
      (romacs-webkit--log "cursor left view %d, deactivating"
                           romacs-webkit--input-view-id)
      (romacs-webkit--deactivate-input))))

(add-hook 'post-command-hook #'romacs-webkit--post-command-check)

(provide 'romacs-webkit)

;;; romacs-webkit.el ends here
