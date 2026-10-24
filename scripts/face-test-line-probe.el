;;; face-test-line-probe.el --- Dump face-test line geometry after GUI startup -*- lexical-binding: t; -*-

(defvar romacs-face-test-probe-line 60
  "1-based source line number to probe in `*Romacs Face Test*'.")

(defvar romacs-face-test-probe-output
  (or (getenv "ROMACS_FACE_TEST_PROBE_OUTPUT")
      "/tmp/romacs-face-line-probe.txt")
  "Output path for the probe report.")

(defvar romacs-face-test-probe-prime-lines nil
  "Optional list of 1-based line numbers to visit before the final probe line.")

(defvar romacs-face-test-probe-delay 0.5
  "Seconds to wait before the first probe attempt.")

(defvar romacs-face-test-probe-poll-interval 0.25
  "Seconds between retries while waiting for the GUI window.")

(defvar romacs-face-test-probe-timeout 15.0
  "Maximum seconds to wait before forcing a probe write.")

(defvar romacs-face-test-probe-expected-frame-width nil
  "Expected `frame-pixel-width' before the probe is considered ready.")

(defvar romacs-face-test-probe-expected-frame-height nil
  "Expected `frame-pixel-height' before the probe is considered ready.")

(defvar romacs-face-test-probe--armed nil
  "Non-nil once the helper has been armed.")

(defvar romacs-face-test-probe--deadline nil
  "Absolute timeout for the current probe attempt.")

(defvar romacs-face-test-probe--timer nil
  "Timer object for the currently scheduled probe attempt.")

(defun romacs-face-test-line-probe--cancel-timer ()
  "Cancel the active probe timer, if any."
  (when (timerp romacs-face-test-probe--timer)
    (cancel-timer romacs-face-test-probe--timer))
  (setq romacs-face-test-probe--timer nil))

(defun romacs-face-test-line-probe--buffer-ready-p ()
  "Return non-nil when the face test buffer is ready for probing."
  (and (display-graphic-p)
       (get-buffer "*Romacs Face Test*")
       (get-buffer-window "*Romacs Face Test*" 0)))

(defun romacs-face-test-line-probe--deadline-expired-p ()
  "Return non-nil when the probe wait deadline has elapsed."
  (and romacs-face-test-probe--deadline
       (not (time-less-p (current-time) romacs-face-test-probe--deadline))))

(defun romacs-face-test-line-probe--geometry-ready-p ()
  "Return non-nil when the selected frame reached the expected pixel geometry."
  (and (or (null romacs-face-test-probe-expected-frame-width)
           (= (frame-pixel-width) romacs-face-test-probe-expected-frame-width))
       (or (null romacs-face-test-probe-expected-frame-height)
           (= (frame-pixel-height) romacs-face-test-probe-expected-frame-height))))

(defun romacs-face-test-line-probe--char-info (pos win)
  "Return plist describing POS in WIN."
  (let* ((face-prop (get-text-property pos 'face))
         (posn (posn-at-point pos win))
         (next-posn (and posn (posn-at-point (1+ pos) win)))
         (xy (and posn (posn-x-y posn)))
         (col-row (and posn (posn-col-row posn)))
         (next-xy (and next-posn (posn-x-y next-posn)))
         (x1 (and xy (car xy)))
         (x2 (and next-xy (car next-xy))))
    (list :pos pos
          :char (char-after pos)
          :face-prop face-prop
          :face-head (car-safe face-prop)
          :face-head-keywordp (and (consp face-prop)
                                   (keywordp (car face-prop)))
          :face-head-type (and (consp face-prop)
                               (type-of (car face-prop)))
          :font-lock-face-prop (get-text-property pos 'font-lock-face)
          :char-face-prop (get-char-property pos 'face)
          :text-props (text-properties-at pos)
          :font (when (fboundp 'font-at)
                  (font-at pos win))
          :advance (when (and (numberp x1) (numberp x2))
                     (- x2 x1))
          :x-y xy
          :col-row col-row
          :visible (pos-visible-in-window-p pos win t)
          :posn posn)))

(defun romacs-face-test-line-probe--write ()
  "Write the requested line probe report when the GUI is ready.
Return non-nil when the report was written."
  (when (and (romacs-face-test-line-probe--buffer-ready-p)
             (or (romacs-face-test-line-probe--geometry-ready-p)
                 (romacs-face-test-line-probe--deadline-expired-p))
             (get-buffer "*Romacs Face Test*"))
    (let* ((src (get-buffer "*Romacs Face Test*"))
           (win (or (get-buffer-window src 0)
                    (selected-window)))
           beg end sample chars line-posn probe-data)
      (with-current-buffer src
        (dolist (prime-line romacs-face-test-probe-prime-lines)
          (goto-char (point-min))
          (forward-line (1- prime-line))
          (set-window-point win (line-beginning-position))
          (redisplay t))
        (goto-char (point-min))
        (forward-line (1- romacs-face-test-probe-line))
        (setq beg (line-beginning-position))
        (setq end (line-end-position))
        (setq sample
              (save-excursion
                (goto-char beg)
                (unless (search-forward "a好好b" end t)
                  (error "Line %d does not contain a好好b" romacs-face-test-probe-line))
                (match-beginning 0)))
        (setq probe-data (romacs-face-test--probe-line beg end win))
        (setq line-posn (posn-at-point beg win))
        (setq chars
              (mapcar
               (lambda (pos)
                 (romacs-face-test-line-probe--char-info pos win))
               (number-sequence sample (+ sample 5)))))
      (romacs-face-test-line-probe--cancel-timer)
      (with-temp-file romacs-face-test-probe-output
        (insert (format "line=%S beg=%S end=%S sample=%S ws=%S we=%S point=%S\n"
                        romacs-face-test-probe-line
                        beg
                        end
                        sample
                        (window-start win)
                        (window-end win t)
                        (window-point win)))
        (insert (format "frame=%Sx%S frame-edges=%S\n"
                        (frame-pixel-width)
                        (frame-pixel-height)
                        (frame-edges)))
        (insert (format "selected-frame=%S window-system=%S frame-window-system=%S native=%Sx%S\n"
                        (selected-frame)
                        (window-system)
                        (frame-parameter nil 'window-system)
                        (frame-native-width)
                        (frame-native-height)))
        (insert (format "frame-parameters=%S\n"
                        (frame-parameters)))
        (insert (format "window-pixel-edges=%S inside-pixel-edges=%S body=%Sx%S\n"
                        (window-pixel-edges win)
                        (window-inside-pixel-edges win)
                        (window-body-width win t)
                        (window-body-height win t)))
        (insert (format "line-posn=%S line-text=%S\n"
                        line-posn
                        (with-current-buffer src
                          (buffer-substring-no-properties beg end))))
        (insert (format "probe-summary=%S\n" probe-data))
        (dolist (info chars)
          (insert (format "%S\n" info))))
      t)))

(defun romacs-face-test-line-probe--poll ()
  "Continue polling until the probe can be written."
  (unless (romacs-face-test-line-probe--write)
    (setq romacs-face-test-probe--timer
          (run-with-timer
           romacs-face-test-probe-poll-interval
           nil
           #'romacs-face-test-line-probe--poll))))

(defun romacs-face-test-line-probe-arm ()
  "Arm the line probe helper once."
  (unless romacs-face-test-probe--armed
    (setq romacs-face-test-probe--armed t)
    (setq romacs-face-test-probe--deadline
          (time-add (current-time)
                    (seconds-to-time romacs-face-test-probe-timeout)))
    (unless (romacs-face-test-line-probe--write)
      (add-hook 'emacs-startup-hook #'romacs-face-test-line-probe--poll)
      (add-hook 'window-setup-hook #'romacs-face-test-line-probe--poll)
      (when after-init-time
        (setq romacs-face-test-probe--timer
              (run-with-timer
               romacs-face-test-probe-delay
               nil
               #'romacs-face-test-line-probe--poll))))))

(defun romacs-face-test-line-probe--write-on-size-change (_frame)
  "Write the probe report in response to a frame size change."
  (when (romacs-face-test-line-probe--write)
    (remove-hook 'window-size-change-functions
                 #'romacs-face-test-line-probe--write-on-size-change)))

(defun romacs-face-test-line-probe-install-size-change-hook ()
  "Install a one-shot size-change hook that writes the probe report."
  (add-hook 'window-size-change-functions
            #'romacs-face-test-line-probe--write-on-size-change))

(provide 'face-test-line-probe)

;;; face-test-line-probe.el ends here
