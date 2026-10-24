;;; scrolling.el --- deterministic page-scrolling workload  -*- lexical-binding: t; -*-

(require 'json)

(defvar romacs-perf--profile-gate-process nil)
(defvar romacs-perf--profile-gate-response "")

(defun romacs-perf--required-environment (name)
  (or (getenv name)
      (error "required performance environment variable %s is absent" name)))

(defun romacs-perf--profile-gate-filter (_process output)
  (setq romacs-perf--profile-gate-response
        (concat romacs-perf--profile-gate-response output)))

(defun romacs-perf--profile-gate-connect ()
  (let* ((port-text (getenv "ROMACS_PERF_GATE_PORT"))
         (port (and port-text (string-to-number port-text))))
    (when (and port-text (not (> port 0)))
      (error "invalid edit-loop profile gate port %S" port-text))
    (when (and port-text
               (not (process-live-p romacs-perf--profile-gate-process)))
      (setq romacs-perf--profile-gate-process
            (make-network-process
             :name "romacs-perf-gate"
             :family 'ipv4
             :host "127.0.0.1"
             :service port
             :coding 'binary
             :noquery t
             :filter #'romacs-perf--profile-gate-filter)))
    romacs-perf--profile-gate-process))

(defun romacs-perf--sampling-command (command)
  (let ((process (romacs-perf--profile-gate-connect)))
    (when process
      (setq romacs-perf--profile-gate-response "")
      (process-send-string process (concat command "\n"))
      (let ((deadline (+ (float-time) 30.0)))
        (while (and (not (and (> (length romacs-perf--profile-gate-response) 0)
                              (= (aref romacs-perf--profile-gate-response
                                       (1- (length romacs-perf--profile-gate-response)))
                                 ?\n)))
                    (< (float-time) deadline))
          (unless (process-live-p process)
            (error "edit-loop profile gate disconnected during %s" command))
          (accept-process-output process 0.05))
        (unless (equal romacs-perf--profile-gate-response "ack\n")
          (error "edit-loop profile gate rejected %s: %S"
                 command romacs-perf--profile-gate-response))))))

(defun romacs-perf--close-profile-gate ()
  (when (processp romacs-perf--profile-gate-process)
    (delete-process romacs-perf--profile-gate-process)
    (setq romacs-perf--profile-gate-process nil)))

(defun romacs-perf--json-boolean (value)
  (if value t :json-false))

;;; Buffer construction.

(defconst romacs-perf-scroll--line-count 2400
  "Buffer size in lines. Large enough that one screenful is a tiny
fraction of the buffer, so scrolling keeps meeting rows the layout
engine has not seen recently instead of re-reading the same dozen.")

(load (expand-file-name "scrolling-content.el" (file-name-directory load-file-name)) nil nil t)

(defun romacs-perf-scroll--insert-buffer ()
  (romacs-scroll-content-insert romacs-perf-scroll--line-count)
  (let ((summary romacs-scroll-content-summary))
    (with-temp-file (concat (getenv "ROMACS_PERF_RESULT") ".content.json")
      (insert (json-serialize summary)))))

;;; Timed work.

(defun romacs-perf-scroll--cpu-us ()
  (car (current-cpu-time)))

(defun romacs-perf-scroll--at-end-p ()
  (>= (point) (point-max)))

(defun romacs-perf-scroll--at-start-p ()
  (<= (point) (point-min)))

(defun romacs-perf-scroll--write-result
    (path status iterations elapsed-us elapsed-wall-us operation-count
          cold-scroll-us warm-scroll-us cold-commands warm-commands
          initial-checksum final-checksum point-restored
          window-start-restored expected-mode actual-mode error-message)
  (with-temp-file path
    (insert
     (json-serialize
      `((schema_version . 1)
        (scenario . "scrolling")
        (status . ,status)
        (iterations . ,iterations)
        (elapsed_us . ,elapsed-us)
        (elapsed_wall_us . ,elapsed-wall-us)
        (operation_count . ,operation-count)
        (cold_scroll_us . ,cold-scroll-us)
        (warm_scroll_us . ,warm-scroll-us)
        (cold_scroll_commands . ,cold-commands)
        (warm_scroll_commands . ,warm-commands)
        (initial_checksum . ,initial-checksum)
        (final_checksum . ,final-checksum)
        (point_restored . ,(romacs-perf--json-boolean point-restored))
        (window_start_restored
         . ,(romacs-perf--json-boolean window-start-restored))
        (expected_major_mode . ,expected-mode)
        (actual_major_mode . ,actual-mode)
        (error . ,error-message))
      :false-object :json-false
      :null-object nil))))

(defun romacs-perf--sampling-command-ack (command)
  "Return t only after the existing sampling gate acknowledges COMMAND."
  (unless (getenv "ROMACS_PERF_GATE_PORT")
    (error "GC window diagnostics require an acknowledged sampling gate"))
  (romacs-perf--sampling-command command)
  (unless (equal romacs-perf--profile-gate-response "ack\n")
    (error "GC window diagnostics lack a %s acknowledgement" command))
  t)

(defun romacs-perf--write-gc-window
    (path scenario iterations before-enable after-enable
          before-disable after-disable enable-ack disable-ack)
  "Write invocation-local diagnostic metadata after sampling has stopped.

The four snapshots bracket both gate handshakes. Derive an exact sampled
GC delta only with both acknowledgements and unchanged handshake counts."
  (with-temp-file path
    (insert
     (json-serialize
      `((schema . "t34-edit-loop-gc-window-v1")
        (scenario . ,scenario)
        (iterations . ,iterations)
        (editor_pid . ,(emacs-pid))
        (gc_window . ((scope . "acknowledged-enable-to-disable")
                      (before_enable . ,before-enable)
                      (after_enable . ,after-enable)
                      (before_disable . ,before-disable)
                      (after_disable . ,after-disable)
                      (enable_ack . ,(if enable-ack t :json-false))
                      (disable_ack . ,(if disable-ack t :json-false)))))
      :false-object :json-false :null-object nil))))

(defun romacs-perf-scroll--run ()
  (let* ((result-path
          (romacs-perf--required-environment "ROMACS_PERF_RESULT"))
         (sentinel-path (romacs-perf--required-environment "SENTINEL"))
         (warm-passes
          (string-to-number
           (romacs-perf--required-environment "ROMACS_PERF_ITERATIONS")))
         (expected-mode "fundamental-mode")
         (initial-point nil)
         (initial-checksum nil)
         (cold-us 0) (warm-us 0)
         (cold-commands 0) (warm-commands 0)
         (started-wall (float-time))
         (status "error")
         (error-message nil)
         (exit-code 2))
    (condition-case error-data
        (progn
          (unless (> warm-passes 0)
            (error "iterations must be positive"))
          (switch-to-buffer (get-buffer-create "*romacs-perf-scrolling*"))
          (erase-buffer)
          (romacs-perf-scroll--insert-buffer)
          (goto-char (point-min))
          (setq initial-point (point)
                initial-checksum
                (secure-hash
                 'sha256
                 (buffer-substring-no-properties (point-min) (point-max))))
          ;; Settle the initial display untimed: the workload times
          ;; SCROLLING, not the first frame.
          (redisplay t)
          (let ((sampling-enabled nil))
            (romacs-perf--sampling-command "enable")
            (setq sampling-enabled t)
            (unwind-protect
                (progn
                  ;; COLD phase: first display of every line, top to
                  ;; bottom. This is where JIT compilation, face
                  ;; realisation, and first-time layout land, which is
                  ;; exactly why it is timed apart from the warm passes.
                  ;; The workload runs `scroll-up-command'/`scroll-down-
                  ;; command' — what C-v/M-v actually execute — with
                  ;; `scroll-error-top-bottom' bound, so the buffer-edge
                  ;; recovery is GNU's own window.el path, and the loops
                  ;; terminate on position rather than on an error.
                  (let ((scroll-error-top-bottom t)
                        (start (romacs-perf-scroll--cpu-us))
                        (safety 2000))
                    (while (and (not (romacs-perf-scroll--at-end-p))
                                (> safety 0))
                      (scroll-up-command nil)
                      (redisplay t)
                      (setq cold-commands (1+ cold-commands)
                            safety (1- safety)))
                    (when (zerop safety)
                      (error "cold pass never reached point-max"))
                    (setq cold-us (- (romacs-perf-scroll--cpu-us) start)))
                  ;; WARM passes: down and back up over rows the cold pass
                  ;; already laid out. One pass is one iteration.
                  (let ((scroll-error-top-bottom t)
                        (start (romacs-perf-scroll--cpu-us)))
                    (dotimes (_ warm-passes)
                      (goto-char (point-min))
                      (set-window-start nil (point-min))
                      (while (not (romacs-perf-scroll--at-end-p))
                        (scroll-up-command nil)
                        (redisplay t)
                        (setq warm-commands (1+ warm-commands)))
                      (while (not (romacs-perf-scroll--at-start-p))
                        (scroll-down-command nil)
                        (redisplay t)
                        (setq warm-commands (1+ warm-commands))))
                    (setq warm-us (- (romacs-perf-scroll--cpu-us) start))))
              (when sampling-enabled
                (romacs-perf--sampling-command "disable"))))
          (setq status "ok"
                exit-code 0))
      (error
       (setq error-message (error-message-string error-data))
       (message "scrolling failed: %s" error-message)))
    (let* ((buffer (get-buffer "*romacs-perf-scrolling*"))
           (live (buffer-live-p buffer))
           (final-checksum
            (and live
                 (with-current-buffer buffer
                   (secure-hash
                    'sha256
                    (buffer-substring-no-properties
                     (point-min) (point-max))))))
           (point-restored
            (and live
                 (with-current-buffer buffer
                   (eql (point) initial-point))))
           (window-start-restored
            (and live
                 (get-buffer-window buffer t)
                 (with-current-buffer buffer
                   (= (window-start (get-buffer-window buffer t)) 1))))
           (actual-mode
            (if live
                (with-current-buffer buffer (symbol-name major-mode))
              ""))
           (elapsed-wall-us
            (round (* 1000000 (- (float-time) started-wall)))))
      (romacs-perf--close-profile-gate)
      (romacs-perf-scroll--write-result
       result-path status warm-passes
       (+ cold-us warm-us)
       elapsed-wall-us
       (+ cold-commands warm-commands)
       cold-us warm-us cold-commands warm-commands
       (or initial-checksum "") (or final-checksum "")
       point-restored window-start-restored
       expected-mode actual-mode error-message))
    (write-region "done\n" nil sentinel-path nil 'silent)
    (kill-emacs exit-code)))

(defun romacs-perf-scroll--run-with-gc-window (gc-window-path)
  "Run the unchanged workload with invocation-local sampling GC snapshots.

All snapshots are lexical state owned by this fixture invocation.  The
existing sampling controller has one owner; no shared mutator cache is
added.  `gcs-done' remains the existing process aggregate, so simultaneous
mutator collections contribute to the observed count."
  (let* ((result-path
          (romacs-perf--required-environment "ROMACS_PERF_RESULT"))
         (sentinel-path (romacs-perf--required-environment "SENTINEL"))
         (warm-passes
          (string-to-number
           (romacs-perf--required-environment "ROMACS_PERF_ITERATIONS")))
         (expected-mode "fundamental-mode")
         (initial-point nil)
         (initial-checksum nil)
         (cold-us 0) (warm-us 0)
         (cold-commands 0) (warm-commands 0)
         (started-wall (float-time))
         (gc-before-enable nil) (gc-after-enable nil)
         (gc-before-disable nil) (gc-after-disable nil)
         (gc-enable-ack nil) (gc-disable-ack nil)
         (status "error")
         (error-message nil)
         (exit-code 2))
    (condition-case error-data
        (progn
          (unless (> warm-passes 0)
            (error "iterations must be positive"))
          (switch-to-buffer (get-buffer-create "*romacs-perf-scrolling*"))
          (erase-buffer)
          (romacs-perf-scroll--insert-buffer)
          (goto-char (point-min))
          (setq initial-point (point)
                initial-checksum
                (secure-hash
                 'sha256
                 (buffer-substring-no-properties (point-min) (point-max))))
          ;; Settle the initial display untimed: the workload times
          ;; SCROLLING, not the first frame.
          (redisplay t)
          (let ((sampling-enabled nil))
            (setq gc-before-enable gcs-done)
            (setq gc-enable-ack
                  (romacs-perf--sampling-command-ack "enable"))
            (setq gc-after-enable gcs-done)
            (setq sampling-enabled t)
            (unwind-protect
                (progn
                  ;; COLD phase: first display of every line, top to
                  ;; bottom. This is where JIT compilation, face
                  ;; realisation, and first-time layout land, which is
                  ;; exactly why it is timed apart from the warm passes.
                  ;; The workload runs `scroll-up-command'/`scroll-down-
                  ;; command' — what C-v/M-v actually execute — with
                  ;; `scroll-error-top-bottom' bound, so the buffer-edge
                  ;; recovery is GNU's own window.el path, and the loops
                  ;; terminate on position rather than on an error.
                  (let ((scroll-error-top-bottom t)
                        (start (romacs-perf-scroll--cpu-us))
                        (safety 2000))
                    (while (and (not (romacs-perf-scroll--at-end-p))
                                (> safety 0))
                      (scroll-up-command nil)
                      (redisplay t)
                      (setq cold-commands (1+ cold-commands)
                            safety (1- safety)))
                    (when (zerop safety)
                      (error "cold pass never reached point-max"))
                    (setq cold-us (- (romacs-perf-scroll--cpu-us) start)))
                  ;; WARM passes: down and back up over rows the cold pass
                  ;; already laid out. One pass is one iteration.
                  (let ((scroll-error-top-bottom t)
                        (start (romacs-perf-scroll--cpu-us)))
                    (dotimes (_ warm-passes)
                      (goto-char (point-min))
                      (set-window-start nil (point-min))
                      (while (not (romacs-perf-scroll--at-end-p))
                        (scroll-up-command nil)
                        (redisplay t)
                        (setq warm-commands (1+ warm-commands)))
                      (while (not (romacs-perf-scroll--at-start-p))
                        (scroll-down-command nil)
                        (redisplay t)
                        (setq warm-commands (1+ warm-commands))))
                    (setq warm-us (- (romacs-perf-scroll--cpu-us) start))))
              (when sampling-enabled
                (setq gc-before-disable gcs-done)
                (setq gc-disable-ack
                      (romacs-perf--sampling-command-ack "disable"))
                (setq gc-after-disable gcs-done))))
          (setq status "ok"
                exit-code 0))
      (error
       (setq error-message (error-message-string error-data))
       (message "scrolling failed: %s" error-message)))
    (let* ((buffer (get-buffer "*romacs-perf-scrolling*"))
           (live (buffer-live-p buffer))
           (final-checksum
            (and live
                 (with-current-buffer buffer
                   (secure-hash
                    'sha256
                    (buffer-substring-no-properties
                     (point-min) (point-max))))))
           (point-restored
            (and live
                 (with-current-buffer buffer
                   (eql (point) initial-point))))
           (window-start-restored
            (and live
                 (get-buffer-window buffer t)
                 (with-current-buffer buffer
                   (= (window-start (get-buffer-window buffer t)) 1))))
           (actual-mode
            (if live
                (with-current-buffer buffer (symbol-name major-mode))
              ""))
           (elapsed-wall-us
            (round (* 1000000 (- (float-time) started-wall)))))
      (romacs-perf--close-profile-gate)
      (romacs-perf-scroll--write-result
       result-path status warm-passes
       (+ cold-us warm-us)
       elapsed-wall-us
       (+ cold-commands warm-commands)
       cold-us warm-us cold-commands warm-commands
       (or initial-checksum "") (or final-checksum "")
       point-restored window-start-restored
       expected-mode actual-mode error-message))
    (romacs-perf--write-gc-window
     gc-window-path "scrolling" warm-passes gc-before-enable gc-after-enable
     gc-before-disable gc-after-disable gc-enable-ack gc-disable-ack)
    (write-region "done\n" nil sentinel-path nil 'silent)
    (kill-emacs exit-code)))

;; Read the diagnostic once per fixture process. With it absent, the
;; ordinary entry form and uninstrumented workload remain literal copies.
(let ((gc-window-path (getenv "ROMACS_PERF_GC_WINDOW_FILE")))
  (if gc-window-path
      (if noninteractive
          (romacs-perf-scroll--run-with-gc-window gc-window-path)
        (run-at-time 0 nil #'romacs-perf-scroll--run-with-gc-window
                     gc-window-path))
    (if noninteractive
    (romacs-perf-scroll--run)
  (run-at-time 0 nil #'romacs-perf-scroll--run))))

;;; scrolling.el ends here
