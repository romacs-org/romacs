;;; face-test-autoreport.el --- Write face probe reports after GUI setup -*- lexical-binding: t; -*-

(defvar romacs-face-test-autoreport-file nil
  "Target file for `romacs-face-test-write-matrix-report'.")

(defvar romacs-face-test-autoreport-delay 0.5
  "Seconds to wait after GUI setup before writing the face probe report.")

(defvar romacs-face-test-autoreport-poll-interval 0.25
  "Seconds between readiness checks while waiting to write the report.")

(defvar romacs-face-test-autoreport-timeout 15.0
  "Maximum seconds to wait for expected GUI geometry before forcing a report.")

(defvar romacs-face-test-autoreport-expected-frame-width nil
  "Expected `frame-pixel-width' before the report is considered ready.")

(defvar romacs-face-test-autoreport-expected-frame-height nil
  "Expected `frame-pixel-height' before the report is considered ready.")

(defvar romacs-face-test-autoreport--armed nil
  "Non-nil once the autoreport helper has been armed.")

(defvar romacs-face-test-autoreport--deadline nil
  "Absolute timeout used by the current autoreport polling cycle.")

(defvar romacs-face-test-autoreport--timer nil
  "Timer object for the currently scheduled autoreport polling cycle.")

(defvar romacs-face-test-autoreport-debug nil
  "Non-nil means log helper scheduling and geometry decisions.")

(defun romacs-face-test-autoreport--log (fmt &rest args)
  "Log a helper debug message when `romacs-face-test-autoreport-debug' is non-nil."
  (when romacs-face-test-autoreport-debug
    (apply #'message (concat "face-test-autoreport: " fmt) args)))

(defun romacs-face-test-autoreport--buffer-ready-p ()
  "Return non-nil when the GUI face test buffer can be probed."
  (and romacs-face-test-autoreport-file
       (display-graphic-p)
       (fboundp 'romacs-face-test-write-matrix-report)
       (get-buffer "*Romacs Face Test*")))

(defun romacs-face-test-autoreport--geometry-ready-p ()
  "Return non-nil when the selected frame reached the expected pixel geometry."
  (and (or (null romacs-face-test-autoreport-expected-frame-width)
           (= (frame-pixel-width) romacs-face-test-autoreport-expected-frame-width))
       (or (null romacs-face-test-autoreport-expected-frame-height)
           (= (frame-pixel-height) romacs-face-test-autoreport-expected-frame-height))))

(defun romacs-face-test-autoreport--deadline-expired-p ()
  "Return non-nil when the current autoreport wait deadline has expired."
  (and romacs-face-test-autoreport--deadline
       (not (time-less-p (current-time) romacs-face-test-autoreport--deadline))))

(defun romacs-face-test-autoreport--cancel-timer ()
  "Cancel the active autoreport timer, if any."
  (when (timerp romacs-face-test-autoreport--timer)
    (romacs-face-test-autoreport--log
     "cancel timer=%S frame=%sx%s"
     romacs-face-test-autoreport--timer
     (frame-pixel-width)
     (frame-pixel-height))
    (cancel-timer romacs-face-test-autoreport--timer))
  (setq romacs-face-test-autoreport--timer nil))

(defun romacs-face-test-autoreport--write ()
  "Write the face probe report when the GUI face test is ready.
Return non-nil when the report was written."
  (when (and (romacs-face-test-autoreport--buffer-ready-p)
             (or (romacs-face-test-autoreport--geometry-ready-p)
                 (romacs-face-test-autoreport--deadline-expired-p)))
    (romacs-face-test-autoreport--log
     "write report ready=%S geometry=%S deadline-expired=%S frame=%sx%s expected=%sx%s"
     (romacs-face-test-autoreport--buffer-ready-p)
     (romacs-face-test-autoreport--geometry-ready-p)
     (romacs-face-test-autoreport--deadline-expired-p)
     (frame-pixel-width)
     (frame-pixel-height)
     (or romacs-face-test-autoreport-expected-frame-width "?")
     (or romacs-face-test-autoreport-expected-frame-height "?"))
    (unless (romacs-face-test-autoreport--geometry-ready-p)
      (message
       "face-test-autoreport: timed out waiting for %sx%s, writing report at %sx%s"
       (or romacs-face-test-autoreport-expected-frame-width "?")
       (or romacs-face-test-autoreport-expected-frame-height "?")
       (frame-pixel-width)
       (frame-pixel-height)))
    (romacs-face-test-autoreport--cancel-timer)
    (romacs-face-test-write-matrix-report romacs-face-test-autoreport-file)
    t))

(defun romacs-face-test-autoreport--poll ()
  "Continue polling until the face probe report can be written."
  (romacs-face-test-autoreport--log
   "poll ready=%S geometry=%S deadline-expired=%S frame=%sx%s expected=%sx%s"
   (romacs-face-test-autoreport--buffer-ready-p)
   (romacs-face-test-autoreport--geometry-ready-p)
   (romacs-face-test-autoreport--deadline-expired-p)
   (frame-pixel-width)
   (frame-pixel-height)
   (or romacs-face-test-autoreport-expected-frame-width "?")
   (or romacs-face-test-autoreport-expected-frame-height "?"))
  (unless (romacs-face-test-autoreport--write)
    (setq romacs-face-test-autoreport--timer
          (run-with-timer
           romacs-face-test-autoreport-poll-interval
           nil
           #'romacs-face-test-autoreport--poll))))

(defun romacs-face-test-autoreport--schedule ()
  "Schedule one deferred face probe report write."
  (when romacs-face-test-autoreport-file
    (setq romacs-face-test-autoreport--deadline
          (time-add (current-time)
                    (seconds-to-time romacs-face-test-autoreport-timeout)))
    (romacs-face-test-autoreport--log
     "schedule frame=%sx%s expected=%sx%s after-init-time=%S startup-file=%S"
     (frame-pixel-width)
     (frame-pixel-height)
     (or romacs-face-test-autoreport-expected-frame-width "?")
     (or romacs-face-test-autoreport-expected-frame-height "?")
     after-init-time
     romacs-face-test-autoreport-file)
    (romacs-face-test-autoreport--cancel-timer)
    (if (romacs-face-test-autoreport--write)
        t
      (setq romacs-face-test-autoreport--timer
            (run-with-timer
             romacs-face-test-autoreport-delay
             nil
             #'romacs-face-test-autoreport--poll)))))

(defun romacs-face-test-autoreport-arm ()
  "Arm the face probe autoreport helper once."
  (unless romacs-face-test-autoreport--armed
    (setq romacs-face-test-autoreport--armed t)
    (romacs-face-test-autoreport--log
     "arm ready=%S after-init-time=%S display=%S file=%S"
     (romacs-face-test-autoreport--buffer-ready-p)
     after-init-time
     (display-graphic-p)
     romacs-face-test-autoreport-file)
    (unless (romacs-face-test-autoreport--write)
      (add-hook 'emacs-startup-hook #'romacs-face-test-autoreport--schedule)
      (add-hook 'window-setup-hook #'romacs-face-test-autoreport--schedule)
      (romacs-face-test-autoreport--log
       "installed startup hooks; after-init-time=%S" after-init-time)
      (when after-init-time
        (romacs-face-test-autoreport--schedule)))))

(provide 'face-test-autoreport)

;;; face-test-autoreport.el ends here
