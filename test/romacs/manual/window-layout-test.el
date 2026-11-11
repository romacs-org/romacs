;;; window-layout-test.el --- Test window layout and dividers -*- lexical-binding: t -*-

;; Test window splitting, dividers, multi-window rendering, mode-lines,
;; header-lines, tab-lines, and window configuration cycling in the
;; romacs GPU renderer.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/window-layout-test.el

;;; Code:

(defvar wl-test--step 0
  "Current test step for the automated sequence.")

(defvar wl-test--cycle-timer nil
  "Timer for window configuration cycling.")

;; ============================================================================
;; Helper functions
;; ============================================================================

(defun wl-test--reset ()
  "Delete all windows except one and kill test buffers."
  (when (timerp wl-test--cycle-timer)
    (cancel-timer wl-test--cycle-timer)
    (setq wl-test--cycle-timer nil))
  (delete-other-windows)
  (dolist (name '("*wl-alpha*" "*wl-beta*" "*wl-gamma*" "*wl-delta*"
                  "*wl-code*" "*wl-prose*" "*wl-log*" "*wl-data*"
                  "*wl-header*" "*wl-tabline*" "*wl-mini-test*"))
    (when (get-buffer name)
      (kill-buffer name))))

(defun wl-test--make-buffer (name fg content-fn)
  "Create buffer NAME with foreground color FG and populate via CONTENT-FN.
CONTENT-FN is called with the buffer current."
  (let ((buf (get-buffer-create name)))
    (with-current-buffer buf
      (erase-buffer)
      (face-remap-add-relative 'default :foreground fg)
      (funcall content-fn))
    buf))

(defun wl-test--insert-banner (title &optional subtitle)
  "Insert a colored TITLE banner and optional SUBTITLE."
  (let ((start (point)))
    (insert (format " %s \n" title))
    (put-text-property start (point) 'face
                       '(:weight bold :height 1.3 :foreground "gold"
                         :background "#333355")))
  (when subtitle
    (insert (propertize (format "  %s\n" subtitle)
                        'face '(:foreground "gray70" :slant italic))))
  (insert "\n"))

(defun wl-test--fill-lines (prefix count)
  "Insert COUNT numbered lines with PREFIX text."
  (dotimes (i count)
    (insert (format "%s line %03d\n" prefix (1+ i)))))

;; ============================================================================
;; Test 1: Horizontal split (C-x 3) -- side-by-side with vertical divider
;; ============================================================================

(defun wl-test--horizontal-split ()
  "Split window horizontally (side-by-side) with vertical divider."
  (wl-test--reset)
  (let ((buf-left (wl-test--make-buffer "*wl-alpha*" "light cyan"
                    (lambda ()
                      (wl-test--insert-banner "LEFT WINDOW (Alpha)"
                                              "split-window-right / C-x 3")
                      (insert "This window is on the LEFT side.\n")
                      (insert "A vertical divider should appear to the right.\n\n")
                      (wl-test--fill-lines "Left" 40))))
        (buf-right (wl-test--make-buffer "*wl-beta*" "light salmon"
                     (lambda ()
                       (wl-test--insert-banner "RIGHT WINDOW (Beta)"
                                               "Vertical divider to the left")
                       (insert "This window is on the RIGHT side.\n")
                       (insert "The divider separates the two panes.\n\n")
                       (wl-test--fill-lines "Right" 40)))))
    (switch-to-buffer buf-left)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf-right)
    (other-window -1))
  (message "Test 1: Horizontal split -- side-by-side windows with vertical divider"))

;; ============================================================================
;; Test 2: Vertical split (C-x 2) -- stacked with horizontal divider
;; ============================================================================

(defun wl-test--vertical-split ()
  "Split window vertically (stacked) with horizontal divider."
  (wl-test--reset)
  (let ((buf-top (wl-test--make-buffer "*wl-alpha*" "light green"
                   (lambda ()
                     (wl-test--insert-banner "TOP WINDOW (Alpha)"
                                             "split-window-below / C-x 2")
                     (insert "This window is on TOP.\n")
                     (insert "A horizontal divider / mode-line separates\n")
                     (insert "this window from the one below.\n\n")
                     (wl-test--fill-lines "Top" 30))))
        (buf-bottom (wl-test--make-buffer "*wl-beta*" "plum"
                      (lambda ()
                        (wl-test--insert-banner "BOTTOM WINDOW (Beta)"
                                                "Below the horizontal divider")
                        (insert "This window is on the BOTTOM.\n")
                        (insert "Each window has its own mode-line.\n\n")
                        (wl-test--fill-lines "Bottom" 30)))))
    (switch-to-buffer buf-top)
    (split-window-below)
    (other-window 1)
    (switch-to-buffer buf-bottom)
    (other-window -1))
  (message "Test 2: Vertical split -- stacked windows with horizontal divider"))

;; ============================================================================
;; Test 3: Multiple splits -- 4-window grid layout
;; ============================================================================

(defun wl-test--four-window-grid ()
  "Create a 2x2 grid of windows."
  (wl-test--reset)
  (let ((buf-tl (wl-test--make-buffer "*wl-alpha*" "sky blue"
                  (lambda ()
                    (wl-test--insert-banner "TOP-LEFT" "Window 1 of 4")
                    (insert "Grid position: top-left\n\n")
                    (wl-test--fill-lines "TL" 20))))
        (buf-tr (wl-test--make-buffer "*wl-beta*" "sandy brown"
                  (lambda ()
                    (wl-test--insert-banner "TOP-RIGHT" "Window 2 of 4")
                    (insert "Grid position: top-right\n\n")
                    (wl-test--fill-lines "TR" 20))))
        (buf-bl (wl-test--make-buffer "*wl-gamma*" "pale green"
                  (lambda ()
                    (wl-test--insert-banner "BOTTOM-LEFT" "Window 3 of 4")
                    (insert "Grid position: bottom-left\n\n")
                    (wl-test--fill-lines "BL" 20))))
        (buf-br (wl-test--make-buffer "*wl-delta*" "orchid"
                  (lambda ()
                    (wl-test--insert-banner "BOTTOM-RIGHT" "Window 4 of 4")
                    (insert "Grid position: bottom-right\n\n")
                    (wl-test--fill-lines "BR" 20)))))
    ;; Build the grid: split horizontally, then split each side vertically
    (switch-to-buffer buf-tl)
    (split-window-right)              ; left | right
    (split-window-below)              ; top-left / bottom-left
    (other-window 2)                  ; move to right side
    (split-window-below)              ; top-right / bottom-right
    ;; Assign buffers: walk windows in order
    (select-window (frame-first-window))
    (switch-to-buffer buf-tl)
    (other-window 1)
    (switch-to-buffer buf-bl)
    (other-window 1)
    (switch-to-buffer buf-tr)
    (other-window 1)
    (switch-to-buffer buf-br)
    ;; Return focus to top-left
    (select-window (frame-first-window)))
  (message "Test 3: Four-window grid -- 2x2 layout with dividers"))

;; ============================================================================
;; Test 4: Unequal window sizes (window-resize)
;; ============================================================================

(defun wl-test--unequal-sizes ()
  "Create windows with deliberately unequal sizes."
  (wl-test--reset)
  (let ((buf-wide (wl-test--make-buffer "*wl-alpha*" "light goldenrod"
                    (lambda ()
                      (wl-test--insert-banner "WIDE WINDOW"
                                              "~70%% of frame width")
                      (insert "This window should be significantly wider\n")
                      (insert "than the narrow window to its right.\n\n")
                      (wl-test--fill-lines "Wide" 25))))
        (buf-narrow (wl-test--make-buffer "*wl-beta*" "medium aquamarine"
                      (lambda ()
                        (wl-test--insert-banner "NARROW"
                                                "~30%%")
                        (insert "Narrow\nwindow.\n\n")
                        (wl-test--fill-lines "N" 25))))
        (buf-tall (wl-test--make-buffer "*wl-gamma*" "light pink"
                    (lambda ()
                      (wl-test--insert-banner "TALL BOTTOM"
                                              "Full width, short height")
                      (insert "This window spans the full width at the bottom.\n")
                      (insert "It has fewer rows than the windows above.\n\n")
                      (wl-test--fill-lines "Bottom" 10)))))
    ;; Top row: wide | narrow
    (switch-to-buffer buf-wide)
    (split-window-below -10)          ; bottom window gets ~10 lines
    (split-window-right)              ; split top into left | right
    ;; Assign buffers
    (switch-to-buffer buf-wide)
    (other-window 1)
    (switch-to-buffer buf-narrow)
    (other-window 1)
    (switch-to-buffer buf-tall)
    ;; Make left window wider: enlarge by 20 columns
    (select-window (frame-first-window))
    (ignore-errors (window-resize nil 20 t))
    (select-window (frame-first-window)))
  (message "Test 4: Unequal window sizes -- wide/narrow top, full-width bottom"))

;; ============================================================================
;; Test 5: Window divider customization
;; ============================================================================

(defun wl-test--divider-customization ()
  "Test window-divider-default-right-width and bottom-width."
  (wl-test--reset)
  ;; Enable window dividers with custom widths
  (setq window-divider-default-right-width 6)
  (setq window-divider-default-bottom-width 4)
  (window-divider-mode 1)
  (let ((buf-a (wl-test--make-buffer "*wl-alpha*" "light steel blue"
                 (lambda ()
                   (wl-test--insert-banner "DIVIDER TEST A"
                                           "right-width=6, bottom-width=4")
                   (insert "Window dividers are enabled.\n")
                   (insert (format "  Right divider width:  %d px\n"
                                   window-divider-default-right-width))
                   (insert (format "  Bottom divider width: %d px\n"
                                   window-divider-default-bottom-width))
                   (insert "\nThe dividers should be clearly visible\n")
                   (insert "and wider than the default 1px.\n\n")
                   (wl-test--fill-lines "DivA" 20))))
        (buf-b (wl-test--make-buffer "*wl-beta*" "dark sea green"
                 (lambda ()
                   (wl-test--insert-banner "DIVIDER TEST B"
                                           "Check right divider")
                   (insert "Look for the thick vertical divider\n")
                   (insert "between this window and the left one.\n\n")
                   (wl-test--fill-lines "DivB" 20))))
        (buf-c (wl-test--make-buffer "*wl-gamma*" "thistle"
                 (lambda ()
                   (wl-test--insert-banner "DIVIDER TEST C"
                                           "Check bottom divider")
                   (insert "Look for the thick horizontal divider\n")
                   (insert "between this window and the ones above.\n\n")
                   (wl-test--fill-lines "DivC" 15)))))
    (switch-to-buffer buf-a)
    (split-window-below -12)
    (split-window-right)
    (switch-to-buffer buf-a)
    (other-window 1)
    (switch-to-buffer buf-b)
    (other-window 1)
    (switch-to-buffer buf-c)
    (select-window (frame-first-window)))
  (message "Test 5: Window divider customization -- right=6px, bottom=4px"))

;; ============================================================================
;; Test 6: Mode-line in each window
;; ============================================================================

(defun wl-test--mode-lines ()
  "Verify per-window mode-line rendering with custom mode-line-format."
  (wl-test--reset)
  (let ((buf-a (wl-test--make-buffer "*wl-alpha*" "pale turquoise"
                 (lambda ()
                   (wl-test--insert-banner "MODE-LINE TEST A"
                                           "Custom mode-line-format")
                   (setq-local mode-line-format
                               '(" " (:propertize "<<< ALPHA >>>"
                                      face (:foreground "cyan" :weight bold))
                                 " | " mode-line-buffer-identification
                                 " | L%l C%c"
                                 " | " mode-name
                                 " %-"))
                   (insert "This window has a custom mode-line.\n")
                   (insert "It should show '<<< ALPHA >>>' on the left.\n\n")
                   (wl-test--fill-lines "ModeA" 20))))
        (buf-b (wl-test--make-buffer "*wl-beta*" "navajo white"
                 (lambda ()
                   (wl-test--insert-banner "MODE-LINE TEST B"
                                           "Different custom mode-line")
                   (setq-local mode-line-format
                               '(" " (:propertize "*** BETA ***"
                                      face (:foreground "orange" :weight bold))
                                 " | %b"
                                 " | " (:propertize "%p" face (:foreground "green"))
                                 " | %m"
                                 " %-"))
                   (insert "This window has a different mode-line.\n")
                   (insert "It should show '*** BETA ***' on the left.\n\n")
                   (wl-test--fill-lines "ModeB" 20))))
        (buf-c (wl-test--make-buffer "*wl-gamma*" "light coral"
                 (lambda ()
                   (wl-test--insert-banner "MODE-LINE TEST C"
                                           "Default mode-line-format")
                   (insert "This window uses the default mode-line.\n")
                   (insert "Compare it with the custom ones above.\n\n")
                   (wl-test--fill-lines "ModeC" 15)))))
    (switch-to-buffer buf-a)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf-b)
    (split-window-below)
    (other-window 1)
    (switch-to-buffer buf-c)
    (select-window (frame-first-window)))
  (message "Test 6: Mode-line -- each window has distinct mode-line content"))

;; ============================================================================
;; Test 7: Active vs inactive window indication
;; ============================================================================

(defun wl-test--active-inactive ()
  "Test active vs inactive window cursor style and mode-line face."
  (wl-test--reset)
  (let ((buf-active (wl-test--make-buffer "*wl-alpha*" "white"
                      (lambda ()
                        (wl-test--insert-banner "ACTIVE WINDOW"
                                                "Cursor: filled box, mode-line: active face")
                        (insert "This window should be ACTIVE (selected).\n")
                        (insert "Cursor: filled box (style 0) that blinks.\n")
                        (insert "Mode-line: uses `mode-line' face (brighter).\n\n")
                        (insert "Click in the other window to see:\n")
                        (insert "  - This cursor becomes hollow (style 3)\n")
                        (insert "  - This mode-line becomes dimmer\n")
                        (insert "  - The other window becomes active\n\n")
                        (wl-test--fill-lines "Active" 20))))
        (buf-inactive (wl-test--make-buffer "*wl-beta*" "gray80"
                        (lambda ()
                          (wl-test--insert-banner "INACTIVE WINDOW"
                                                  "Cursor: hollow box, mode-line: inactive face")
                          (insert "This window should be INACTIVE.\n")
                          (insert "Cursor: hollow box (style 3), no blink.\n")
                          (insert "Mode-line: uses `mode-line-inactive' face.\n\n")
                          (insert "Click here to make this window active.\n")
                          (insert "The other window will become inactive.\n\n")
                          (wl-test--fill-lines "Inactive" 20)))))
    (switch-to-buffer buf-active)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf-inactive)
    ;; Move cursor to a visible position in inactive buffer
    (goto-char (point-min))
    (forward-line 5)
    ;; Return to the active window
    (other-window -1)
    (goto-char (point-min))
    (forward-line 5))
  (message "Test 7: Active/inactive -- compare cursor style and mode-line face"))

;; ============================================================================
;; Test 8: Different buffers with different content types
;; ============================================================================

(defun wl-test--different-content ()
  "Show different content types in each window."
  (wl-test--reset)
  (let ((buf-code (wl-test--make-buffer "*wl-code*" "light cyan"
                    (lambda ()
                      (emacs-lisp-mode)
                      (wl-test--insert-banner "EMACS LISP CODE")
                      (insert ";; Sample Emacs Lisp\n")
                      (insert "(defun factorial (n)\n")
                      (insert "  \"Compute N factorial.\"\n")
                      (insert "  (if (<= n 1) 1\n")
                      (insert "    (* n (factorial (1- n)))))\n\n")
                      (insert "(defun range (start end)\n")
                      (insert "  \"Generate list from START to END.\"\n")
                      (insert "  (let ((result nil))\n")
                      (insert "    (while (<= start end)\n")
                      (insert "      (push start result)\n")
                      (insert "      (setq start (1+ start)))\n")
                      (insert "    (nreverse result)))\n\n")
                      (insert "(message \"%s\" (mapcar #'factorial (range 1 10)))\n")
                      (font-lock-ensure))))
        (buf-prose (wl-test--make-buffer "*wl-prose*" "wheat"
                     (lambda ()
                       (wl-test--insert-banner "PROSE / TEXT")
                       (insert "Lorem ipsum dolor sit amet, consectetur ")
                       (insert "adipiscing elit. Sed do eiusmod tempor ")
                       (insert "incididunt ut labore et dolore magna aliqua.\n\n")
                       (insert "Ut enim ad minim veniam, quis nostrud ")
                       (insert "exercitation ullamco laboris nisi ut aliquip ")
                       (insert "ex ea commodo consequat.\n\n")
                       (insert (propertize "Bold paragraph: " 'face 'bold))
                       (insert "Duis aute irure dolor in reprehenderit ")
                       (insert "in voluptate velit esse cillum dolore eu ")
                       (insert "fugiat nulla pariatur.\n\n")
                       (insert (propertize "Italic paragraph: " 'face 'italic))
                       (insert "Excepteur sint occaecat cupidatat non ")
                       (insert "proident, sunt in culpa qui officia deserunt ")
                       (insert "mollit anim id est laborum.\n"))))
        (buf-log (wl-test--make-buffer "*wl-log*" "light green"
                   (lambda ()
                     (wl-test--insert-banner "LOG OUTPUT")
                     (dotimes (i 30)
                       (let* ((level (nth (mod i 4) '("INFO" "WARN" "ERROR" "DEBUG")))
                              (face (cond
                                     ((string= level "ERROR")
                                      '(:foreground "red" :weight bold))
                                     ((string= level "WARN")
                                      '(:foreground "orange"))
                                     ((string= level "DEBUG")
                                      '(:foreground "gray60"))
                                     (t '(:foreground "light green")))))
                         (insert (format "2026-02-11 10:%02d:%02d " (/ i 2) (* (mod i 6) 10)))
                         (insert (propertize (format "[%-5s]" level) 'face face))
                         (insert (format " Process %d: operation completed\n" (1+ i))))))))
        (buf-data (wl-test--make-buffer "*wl-data*" "plum"
                    (lambda ()
                      (wl-test--insert-banner "TABULAR DATA")
                      (insert (propertize
                               (format "%-6s %-20s %8s %10s\n"
                                       "ID" "Name" "Score" "Status")
                               'face '(:underline t :weight bold)))
                      (let ((names '("Alice" "Bob" "Charlie" "Diana" "Eve"
                                     "Frank" "Grace" "Hank" "Ivy" "Jack"
                                     "Karen" "Leo" "Mona" "Nick" "Olive"))
                            (statuses '("Active" "Pending" "Closed" "Active" "Error")))
                        (dotimes (i (length names))
                          (let ((status (nth (mod i 5) statuses)))
                            (insert (format "%-6d %-20s %8d "
                                            (1+ i) (nth i names)
                                            (+ 50 (random 50))))
                            (insert (propertize
                                     (format "%10s" status)
                                     'face (cond
                                            ((string= status "Active")
                                             '(:foreground "green"))
                                            ((string= status "Error")
                                             '(:foreground "red" :weight bold))
                                            ((string= status "Pending")
                                             '(:foreground "yellow"))
                                            (t nil))))
                            (insert "\n"))))))))
    ;; Build 4-window layout
    (switch-to-buffer buf-code)
    (split-window-right)
    (split-window-below)
    (other-window 2)
    (split-window-below)
    ;; Assign buffers
    (select-window (frame-first-window))
    (switch-to-buffer buf-code)
    (other-window 1)
    (switch-to-buffer buf-log)
    (other-window 1)
    (switch-to-buffer buf-prose)
    (other-window 1)
    (switch-to-buffer buf-data)
    (select-window (frame-first-window)))
  (message "Test 8: Different content types -- code, prose, log, data"))

;; ============================================================================
;; Test 9: Header-line in one window
;; ============================================================================

(defun wl-test--header-line ()
  "Test header-line-format in a window."
  (wl-test--reset)
  (let ((buf-header (wl-test--make-buffer "*wl-header*" "light sky blue"
                      (lambda ()
                        (setq-local header-line-format
                                    '(" "
                                      (:propertize "HEADER LINE"
                                       face (:foreground "yellow" :weight bold
                                             :background "dark blue"))
                                      " | "
                                      (:propertize "File: window-layout-test.el"
                                       face (:foreground "white"
                                             :background "dark blue"))
                                      " | "
                                      (:propertize "Branch: main"
                                       face (:foreground "light green"
                                             :background "dark blue"))))
                        (wl-test--insert-banner "HEADER-LINE TEST"
                                                "This window has a header-line at the top")
                        (insert "The header-line should appear at the top of\n")
                        (insert "this window, ABOVE the buffer content.\n\n")
                        (insert "It should say: HEADER LINE | File: ... | Branch: ...\n")
                        (insert "with yellow/white/green text on dark blue background.\n\n")
                        (wl-test--fill-lines "HdrContent" 25))))
        (buf-normal (wl-test--make-buffer "*wl-alpha*" "peach puff"
                      (lambda ()
                        (wl-test--insert-banner "NO HEADER-LINE"
                                                "This window has NO header-line")
                        (insert "Compare this window with the left one.\n")
                        (insert "This window starts directly with buffer content,\n")
                        (insert "while the left window has a header-line bar.\n\n")
                        (wl-test--fill-lines "NoHdr" 25)))))
    (switch-to-buffer buf-header)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf-normal)
    (other-window -1))
  (message "Test 9: Header-line -- left window has header-line, right does not"))

;; ============================================================================
;; Test 10: Tab-line in one window
;; ============================================================================

(defun wl-test--tab-line ()
  "Test tab-line-mode in a window."
  (wl-test--reset)
  (let ((buf-tabs (wl-test--make-buffer "*wl-tabline*" "honeydew3"
                    (lambda ()
                      (wl-test--insert-banner "TAB-LINE TEST"
                                              "This window has tab-line-mode enabled")
                      (insert "The tab-line should appear at the very top\n")
                      (insert "of this window, showing buffer tabs.\n\n")
                      (insert "The tab for this buffer should be highlighted\n")
                      (insert "as the currently selected tab.\n\n")
                      (wl-test--fill-lines "TabContent" 20))))
        (buf-normal (wl-test--make-buffer "*wl-alpha*" "misty rose"
                      (lambda ()
                        (wl-test--insert-banner "NO TAB-LINE"
                                                "This window has no tab-line")
                        (insert "Compare: this window has no tab-line,\n")
                        (insert "the left window has tabs visible at the top.\n\n")
                        (wl-test--fill-lines "NoTab" 20)))))
    (switch-to-buffer buf-tabs)
    ;; Enable tab-line-mode only in this buffer
    (tab-line-mode 1)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf-normal)
    (other-window -1))
  (message "Test 10: Tab-line -- left window has tab-line-mode, right does not"))

;; ============================================================================
;; Test 11: Minibuffer interaction with window layout
;; ============================================================================

(defun wl-test--minibuffer-interaction ()
  "Test that the minibuffer works correctly with multi-window layouts."
  (wl-test--reset)
  (let ((buf-a (wl-test--make-buffer "*wl-alpha*" "light blue"
                 (lambda ()
                   (wl-test--insert-banner "MINIBUFFER TEST"
                                           "Multi-window + minibuffer interaction")
                   (insert "This test creates a 3-window layout and then\n")
                   (insert "activates the minibuffer to verify it renders\n")
                   (insert "correctly at the bottom of the frame.\n\n")
                   (insert "The minibuffer should NOT overlap any window.\n")
                   (insert "All windows should remain visible above it.\n\n")
                   (wl-test--fill-lines "MiniA" 15))))
        (buf-b (wl-test--make-buffer "*wl-beta*" "khaki"
                 (lambda ()
                   (wl-test--insert-banner "WINDOW B")
                   (insert "Second window in the layout.\n\n")
                   (wl-test--fill-lines "MiniB" 15))))
        (buf-c (wl-test--make-buffer "*wl-gamma*" "light pink"
                 (lambda ()
                   (wl-test--insert-banner "WINDOW C")
                   (insert "Third window -- bottom pane.\n\n")
                   (wl-test--fill-lines "MiniC" 10)))))
    ;; Build layout: two on top, one on bottom
    (switch-to-buffer buf-a)
    (split-window-below -12)
    (split-window-right)
    (switch-to-buffer buf-a)
    (other-window 1)
    (switch-to-buffer buf-b)
    (other-window 1)
    (switch-to-buffer buf-c)
    (select-window (frame-first-window))
    ;; After a short delay, activate minibuffer with a message
    (run-at-time 1 nil
                 (lambda ()
                   (message "Minibuffer active: all 3 windows + minibuffer visible. Press C-g to dismiss.")
                   ;; Prompt user to exercise the minibuffer
                   (run-at-time 2 nil
                                (lambda ()
                                  (minibuffer-message
                                   " [Test: type in minibuffer, then C-g to exit]"))))))
  (message "Test 11: Minibuffer -- 3-window layout, minibuffer appears in 1 second"))

;; ============================================================================
;; Test 12: Window configuration cycling with timer
;; ============================================================================

(defvar wl-test--cycle-phase 0
  "Current phase in the window cycling animation.")

(defun wl-test--cycle-step ()
  "Execute one step of the window configuration cycling."
  (let ((phase (mod wl-test--cycle-phase 6)))
    (pcase phase
      ;; Phase 0: Single window
      (0
       (delete-other-windows)
       (switch-to-buffer
        (wl-test--make-buffer "*wl-alpha*" "light cyan"
          (lambda ()
            (wl-test--insert-banner "CYCLE: Single Window"
                                    "Phase 1/6 -- one full-frame window")
            (insert "Window configuration cycling demo.\n")
            (insert "This cycles through different layouts\n")
            (insert "to demonstrate split/delete crossfade animation.\n\n")
            (wl-test--fill-lines "Single" 30))))
       (message "Cycle phase 1/6: Single window"))

      ;; Phase 1: Horizontal split
      (1
       (split-window-right)
       (other-window 1)
       (switch-to-buffer
        (wl-test--make-buffer "*wl-beta*" "light salmon"
          (lambda ()
            (wl-test--insert-banner "CYCLE: Right Pane"
                                    "Phase 2/6 -- horizontal split")
            (wl-test--fill-lines "Right" 30))))
       (other-window -1)
       (message "Cycle phase 2/6: Horizontal split (2 windows)"))

      ;; Phase 2: Add vertical split on right side
      (2
       (other-window 1)
       (split-window-below)
       (other-window 1)
       (switch-to-buffer
        (wl-test--make-buffer "*wl-gamma*" "pale green"
          (lambda ()
            (wl-test--insert-banner "CYCLE: Bottom-Right"
                                    "Phase 3/6 -- 3 windows")
            (wl-test--fill-lines "BotRight" 15))))
       (select-window (frame-first-window))
       (message "Cycle phase 3/6: Three windows (L | TR / BR)"))

      ;; Phase 3: Add vertical split on left side -> 4 windows
      (3
       (split-window-below)
       (other-window 1)
       (switch-to-buffer
        (wl-test--make-buffer "*wl-delta*" "orchid"
          (lambda ()
            (wl-test--insert-banner "CYCLE: Bottom-Left"
                                    "Phase 4/6 -- 4-window grid")
            (wl-test--fill-lines "BotLeft" 15))))
       (select-window (frame-first-window))
       (message "Cycle phase 4/6: Four-window grid"))

      ;; Phase 4: Resize -- make left column wider
      (4
       (select-window (frame-first-window))
       (ignore-errors (window-resize nil 15 t))
       (message "Cycle phase 5/6: Resized -- left column wider"))

      ;; Phase 5: Delete windows back to one
      (5
       (delete-other-windows)
       (switch-to-buffer (get-buffer-create "*wl-alpha*"))
       (message "Cycle phase 6/6: Back to single window (crossfade)")))

    (setq wl-test--cycle-phase (1+ wl-test--cycle-phase))))

(defun wl-test--cycle-windows ()
  "Start automatic window configuration cycling."
  (wl-test--reset)
  (setq wl-test--cycle-phase 0)
  ;; Run the first step immediately
  (wl-test--cycle-step)
  ;; Then cycle every 2.5 seconds
  (setq wl-test--cycle-timer
        (run-at-time 2.5 2.5 #'wl-test--cycle-step))
  (message "Test 12: Window cycling -- layouts change every 2.5s, press 'q' to stop"))

;; ============================================================================
;; Test runner
;; ============================================================================

(defvar wl-test--tests
  '(("Horizontal split"       . wl-test--horizontal-split)
    ("Vertical split"         . wl-test--vertical-split)
    ("Four-window grid"       . wl-test--four-window-grid)
    ("Unequal sizes"          . wl-test--unequal-sizes)
    ("Divider customization"  . wl-test--divider-customization)
    ("Mode-lines"             . wl-test--mode-lines)
    ("Active/inactive"        . wl-test--active-inactive)
    ("Different content"      . wl-test--different-content)
    ("Header-line"            . wl-test--header-line)
    ("Tab-line"               . wl-test--tab-line)
    ("Minibuffer interaction" . wl-test--minibuffer-interaction)
    ("Window cycling"         . wl-test--cycle-windows))
  "Alist of (name . function) for window layout tests.")

(defun wl-test-next ()
  "Run the next test in the sequence."
  (interactive)
  (when (>= wl-test--step (length wl-test--tests))
    (setq wl-test--step 0))
  (let ((test (nth wl-test--step wl-test--tests)))
    (message "--- Running test %d/%d: %s ---"
             (1+ wl-test--step)
             (length wl-test--tests)
             (car test))
    (funcall (cdr test))
    (setq wl-test--step (1+ wl-test--step))))

(defun wl-test-run-all ()
  "Run all tests sequentially with pauses."
  (interactive)
  (setq wl-test--step 0)
  (let ((i 0))
    (dolist (test wl-test--tests)
      (run-at-time (* i 4) nil
                   (lambda (test-pair idx)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length wl-test--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    ;; Final message
    (run-at-time (* i 4) nil
                 (lambda ()
                   (message "\n=== All %d window layout tests complete ==="
                            (length wl-test--tests))
                   (message "Press 'q' to reset, 'n' for individual test, 'a' to run all again"))))
  (message "Running all %d tests (4 second intervals)..." (length wl-test--tests)))

;; Keybindings for interactive use
(defvar wl-test-map (make-sparse-keymap)
  "Keymap for window layout test.")

(define-key wl-test-map (kbd "n") #'wl-test-next)
(define-key wl-test-map (kbd "a") #'wl-test-run-all)
(define-key wl-test-map (kbd "q") (lambda () (interactive)
                                    (wl-test--reset)
                                    (message "All test windows cleaned up.")))
(define-key wl-test-map (kbd "1")  (lambda () (interactive) (wl-test--horizontal-split)))
(define-key wl-test-map (kbd "2")  (lambda () (interactive) (wl-test--vertical-split)))
(define-key wl-test-map (kbd "3")  (lambda () (interactive) (wl-test--four-window-grid)))
(define-key wl-test-map (kbd "4")  (lambda () (interactive) (wl-test--unequal-sizes)))
(define-key wl-test-map (kbd "5")  (lambda () (interactive) (wl-test--divider-customization)))
(define-key wl-test-map (kbd "6")  (lambda () (interactive) (wl-test--mode-lines)))
(define-key wl-test-map (kbd "7")  (lambda () (interactive) (wl-test--active-inactive)))
(define-key wl-test-map (kbd "8")  (lambda () (interactive) (wl-test--different-content)))
(define-key wl-test-map (kbd "9")  (lambda () (interactive) (wl-test--header-line)))
(define-key wl-test-map (kbd "0")  (lambda () (interactive) (wl-test--tab-line)))
(define-key wl-test-map (kbd "-")  (lambda () (interactive) (wl-test--minibuffer-interaction)))
(define-key wl-test-map (kbd "=")  (lambda () (interactive) (wl-test--cycle-windows)))

(set-transient-map wl-test-map t)

;; Setup on load
(message "=== Window Layout Test Suite ===")
(message "Keys: n=next  a=run-all  1-9,0,-,==specific test  q=cleanup")
(wl-test--horizontal-split)

;;; window-layout-test.el ends here
