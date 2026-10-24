;;; romacs-term-interactive-test.el --- Interactive xdotool test for romacs-term -*- lexical-binding: t -*-

;; Usage: ./target/release/romacs -Q -l test/romacs/romacs-term-interactive-test.el
;;
;; This test is driven by xdotool from run-romacs-term-interactive-test.sh.
;; xdotool invokes M-x romacs-term and then sends keystrokes to the terminal.
;; All test state is rooted below the repository-local directory supplied by
;; run-romacs-term-interactive-test.sh.

;;; Code:

(require 'cl-lib)

(defvar romacs-term-itest--results-file
  (or (getenv "NEO_TERM_ITEST_RESULTS")
      (error "NEO_TERM_ITEST_RESULTS is not set"))
  "File to write test results to.")

(defvar romacs-term-itest--marker-dir
  (or (getenv "NEO_TERM_ITEST_MARKER_DIR")
      (error "NEO_TERM_ITEST_MARKER_DIR is not set"))
  "Directory containing phase marker files.")

(defvar romacs-term-itest--terminal-id nil
  "Active terminal ID for testing.")

(defun romacs-term-itest--log (fmt &rest args)
  "Log a message to results file."
  (let ((msg (apply #'format fmt args)))
    (append-to-file (concat msg "\n") nil romacs-term-itest--results-file)
    (message "romacs-term-itest: %s" msg)))

(defun romacs-term-itest--check (name condition)
  "Record a test result."
  (if condition
      (romacs-term-itest--log "PASS: %s" name)
    (romacs-term-itest--log "FAIL: %s" name)))

(defun romacs-term-itest--wait-for-text (terminal-id predicate &optional timeout)
  "Wait until TERMINAL-ID text satisfies PREDICATE, then return the text.
TIMEOUT defaults to five seconds."
  (let ((deadline (+ (float-time) (or timeout 5)))
        text)
    (while (and (< (float-time) deadline)
                (not (and (setq text
                                (ignore-errors
                                  (romacs-terminal-get-text terminal-id)))
                          (stringp text)
                          (funcall predicate text))))
      (sit-for 0.05))
    (and (stringp text) (funcall predicate text) text)))

(defun romacs-term-itest--wait-until-destroyed (terminal-id &optional timeout)
  "Wait until TERMINAL-ID is acknowledged absent by renderer-owned state."
  (let ((deadline (+ (float-time) (or timeout 5)))
        destroyed)
    (while (and (< (float-time) deadline) (not destroyed))
      (setq destroyed
            (condition-case err
                (progn (romacs-terminal-get-text terminal-id) nil)
              (error
               (string-match-p
                "unknown romacs-term terminal id"
                (error-message-string err)))))
      (unless destroyed (sit-for 0.05)))
    destroyed))

(defun romacs-term-itest--marker (name)
  "Return the absolute marker path for NAME."
  (expand-file-name name romacs-term-itest--marker-dir))

(defun romacs-term-itest--run ()
  "Set up observation for an interactively created terminal."
  ;; Clear results file
  (when (file-exists-p romacs-term-itest--results-file)
    (delete-file romacs-term-itest--results-file))

  (romacs-term-itest--log "=== Romacs-Term Interactive Test ===")
  (romacs-term-itest--log "Date: %s" (format-time-string "%Y-%m-%d %H:%M:%S"))

  ;; Check FFI
  (romacs-term-itest--log "Checking FFI availability...")
  (romacs-term-itest--check "FFI available" (fboundp 'romacs-terminal-create))

  (unless (fboundp 'romacs-terminal-create)
    (romacs-term-itest--log "ABORT: FFI not available")
    (romacs-term-itest--log "=== DONE ===")
    (kill-emacs 1))

  ;; Phase 1 is driven through the real M-x command by the shell script.
  (romacs-term-itest--log "")
  (romacs-term-itest--log "--- Phase 1: Waiting for M-x romacs-term ---")

  ;; Set up a timer to check text extraction after xdotool sends input
  ;; The shell script will signal us by creating a marker file
  (run-with-timer 1 1 #'romacs-term-itest--poll-phases))

(defvar romacs-term-itest--phase 0
  "Current test phase.")

(defvar romacs-term-itest--floating-id nil
  "Floating terminal kept live until the screenshot has observed it.")

(defun romacs-term-itest--poll-phases ()
  "Poll for phase progression signals from the shell script."
  (condition-case err
      (cond
       ;; Phase 1: xdotool invoked M-x romacs-term.
       ((= romacs-term-itest--phase 0)
        (let ((terminal-buffer
               (cl-find-if
                (lambda (buffer)
                  (with-current-buffer buffer
                    (eq major-mode 'romacs-term-mode)))
                (buffer-list))))
          (when terminal-buffer
            (setq romacs-term-itest--phase 1)
            (with-current-buffer terminal-buffer
              (setq romacs-term-itest--terminal-id romacs-term--id)
              (romacs-term-itest--check
               "M-x romacs-term created a terminal"
               (and romacs-term-itest--terminal-id
                    (> romacs-term-itest--terminal-id 0)))
              (romacs-term-itest--check
               "Selected buffer is romacs-term-mode"
               (and (eq (current-buffer) (window-buffer (selected-window)))
                    (eq major-mode 'romacs-term-mode)))
              (romacs-term-itest--log "Terminal ID: %s" romacs-term-itest--terminal-id)
              (romacs-term-itest--log "READY_FOR_INPUT")))))

       ;; Phase 2: After xdotool typed "echo hello" + Return
       ((and (= romacs-term-itest--phase 1)
             (file-exists-p (romacs-term-itest--marker "phase2")))
        (setq romacs-term-itest--phase 2)
        (romacs-term-itest--log "")
        (romacs-term-itest--log "--- Phase 2: Text extraction after typing ---")
        (let ((text (romacs-term-itest--wait-for-text
                     romacs-term-itest--terminal-id
                     (lambda (value) (string-match-p "hello" value)))))
          (romacs-term-itest--check "get-text returns string" (stringp text))
          (if (stringp text)
              (progn
                (romacs-term-itest--log "Terminal text (first 200 chars): %s"
                                     (substring text 0 (min 200 (length text))))
                (romacs-term-itest--check "Text contains 'hello'"
                                       (string-match-p "hello" text)))
            (romacs-term-itest--check "Text contains 'hello'" nil))))

       ;; Phase 3: After xdotool typed a command with ANSI colors
       ((and (= romacs-term-itest--phase 2)
             (file-exists-p (romacs-term-itest--marker "phase3")))
        (setq romacs-term-itest--phase 3)
        (romacs-term-itest--log "")
        (romacs-term-itest--log "--- Phase 3: ANSI color output ---")
        (let ((text (romacs-term-itest--wait-for-text
                     romacs-term-itest--terminal-id
                     (lambda (value)
                       (and (string-match-p "RED" value)
                            (string-match-p "GREEN" value)
                            (string-match-p "BLUE" value)
                            (string-match-p "COLOR_TEST" value))))))
          (romacs-term-itest--check "ANSI terminal text returns string" (stringp text))
          (if (stringp text)
              (progn
                (romacs-term-itest--log "Terminal text (first 300 chars): %s"
                                     (substring text 0 (min 300 (length text))))
                (romacs-term-itest--check
                 "Text contains complete color test output"
                 (and (string-match-p "RED" text)
                      (string-match-p "GREEN" text)
                      (string-match-p "BLUE" text)
                      (string-match-p "COLOR_TEST" text))))
            (romacs-term-itest--check "Text contains complete color test output" nil))))

       ;; Phase 4: Resize test
       ((and (= romacs-term-itest--phase 3)
             (file-exists-p (romacs-term-itest--marker "phase4")))
        (setq romacs-term-itest--phase 4)
        (romacs-term-itest--log "")
        (romacs-term-itest--log "--- Phase 4: Terminal resize ---")
        (condition-case err
            (progn
              (romacs-terminal-resize romacs-term-itest--terminal-id 120 40)
              (romacs-term-itest--check "Resize API call succeeded" t)
              ;; Renderer commands are FIFO, so stty observes the PTY only
              ;; after the preceding resize has actually been applied.
              (romacs-terminal-write
               romacs-term-itest--terminal-id
               "printf '__NEO_SIZE__'; stty size\r")
              (let ((text (romacs-term-itest--wait-for-text
                           romacs-term-itest--terminal-id
                           (lambda (value)
                             (string-match-p
                              "__NEO_SIZE__[[:space:]]*40 120"
                              value)))))
                (romacs-term-itest--check "Text extraction works after resize"
                                        (and (stringp text) (> (length text) 0)))
                (romacs-term-itest--check
                 "PTY reports 40 rows by 120 columns"
                 (and (stringp text)
                      (string-match-p "__NEO_SIZE__[[:space:]]*40 120" text)))
                (when (stringp text)
                  (romacs-term-itest--log "After resize text length: %d" (length text)))))
          (error
           (romacs-term-itest--log "ERROR during resize: %s" (error-message-string err))
           (romacs-term-itest--check "Resize API call succeeded" nil))))

       ;; Phase 5: Floating terminal
       ((and (= romacs-term-itest--phase 4)
             (file-exists-p (romacs-term-itest--marker "phase5")))
        (setq romacs-term-itest--phase 5)
        (romacs-term-itest--log "")
        (romacs-term-itest--log "--- Phase 5: Floating terminal ---")
        (condition-case err
            (let ((float-id (romacs-term-floating 150 150 60 20)))
              (setq romacs-term-itest--floating-id float-id)
              (romacs-term-itest--check "Floating terminal created" (and float-id (> float-id 0)))
              (when float-id
                (romacs-term-itest--log "Floating terminal ID: %s" float-id)
                ;; Write to floating terminal
                (romacs-terminal-write
                 float-id
                 "printf '\\033[45mFLOATING_GPU_OK\\033[0m\\n'\r")
                (let ((text (romacs-term-itest--wait-for-text
                             float-id
                             (lambda (value)
                               (string-match-p "FLOATING_GPU_OK" value)))))
                  (romacs-term-itest--check "Floating get-text returns string" (stringp text))
                  (if (stringp text)
                      (progn
                        (romacs-term-itest--log "Floating text: %s"
                                             (substring text 0 (min 200 (length text))))
                        (romacs-term-itest--check "Floating terminal has output"
                                               (string-match-p "FLOATING_GPU_OK" text)))
                    (romacs-term-itest--check "Floating terminal has output" nil)))
                (romacs-term-itest--log "FLOATING_RENDER_READY")))
          (error
           (romacs-term-itest--log "ERROR with floating terminal: %s" (error-message-string err))
           (romacs-term-itest--check "Floating terminal created" nil))))

       ;; Phase 6: Cleanup and done
       ((and (= romacs-term-itest--phase 5)
             (file-exists-p (romacs-term-itest--marker "phase6")))
        (setq romacs-term-itest--phase 6)
        (romacs-term-itest--log "")
        (romacs-term-itest--log "--- Phase 6: Cleanup ---")
        (when romacs-term-itest--floating-id
          (romacs-terminal-destroy romacs-term-itest--floating-id)
          (romacs-term-itest--check
           "Floating terminal removed from renderer state"
           (romacs-term-itest--wait-until-destroyed
            romacs-term-itest--floating-id)))
        ;; Destroy main terminal
        (when romacs-term-itest--terminal-id
          (condition-case err
              (progn
                (romacs-terminal-destroy romacs-term-itest--terminal-id)
                (romacs-term-itest--check
                 "Main terminal removed from renderer state"
                 (romacs-term-itest--wait-until-destroyed
                  romacs-term-itest--terminal-id)))
            (error
             (romacs-term-itest--log "ERROR destroying: %s" (error-message-string err)))))
        (romacs-term-itest--log "")
        (romacs-term-itest--log "=== DONE ===")))
    (error
     (romacs-term-itest--log "POLL ERROR: %s" (error-message-string err)))))

;; Start the test
(romacs-term-itest--run)

;;; romacs-term-interactive-test.el ends here
