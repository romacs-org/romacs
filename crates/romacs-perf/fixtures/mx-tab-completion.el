;;; mx-tab-completion.el --- deterministic M-x TAB workload  -*- lexical-binding: t; -*-

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

(defvar romacs-perf-mx-tab--elapsed-us 0)
(defvar romacs-perf-mx-tab--help-calls 0)
(defvar romacs-perf-mx-tab--warmup-calls 0)
;; When nil, a completion still runs in full but the advice records
;; nothing: the warm-up pass must be indistinguishable from a real one to
;; the JIT and caches, yet contribute zero timed work.
(defvar romacs-perf-mx-tab--timed t)
(defvar romacs-perf-mx-tab--completion-visible t)
(defvar romacs-perf-mx-tab--completion-mode-correct t)
(defvar romacs-perf-mx-tab--known-commands-present t)
(defvar romacs-perf-mx-tab--last-completions nil)
(defvar romacs-perf-mx-tab--candidate-count nil)
(defvar romacs-perf-mx-tab--current-candidate-count 0)
(defvar romacs-perf-mx-tab--candidate-count-stable t)
(defconst romacs-perf-mx-tab--candidate-total 1024)

(defun romacs-perf-mx-tab--controlled-command ()
  (interactive))

(defun romacs-perf-mx-tab--install-controlled-commands ()
  (dotimes (index romacs-perf-mx-tab--candidate-total)
    (fset (intern (format "romacs-perf-command-%04d" index))
          #'romacs-perf-mx-tab--controlled-command)))

(defun romacs-perf-mx-tab--candidate-present-p (name candidates)
  (catch 'present
    (while (consp candidates)
      (when (equal name (car candidates))
        (throw 'present t))
      (setq candidates (cdr candidates)))
    nil))

(defun romacs-perf-mx-tab--capture-completions (original &rest arguments)
  (let ((completions (apply original arguments)))
    ;; A completion style may make nested calls and may use an integer as the
    ;; final cdr for base-size. Retain the largest candidate spine produced by
    ;; this TAB invocation without assuming the result is a proper list.
    (when (and (minibufferp) (consp completions))
      (let ((count (safe-length completions)))
        (when (> count romacs-perf-mx-tab--current-candidate-count)
          (setq romacs-perf-mx-tab--current-candidate-count count
                romacs-perf-mx-tab--last-completions completions))))
    completions))

(defun romacs-perf-mx-tab--around-completion-help (original &rest arguments)
  (if (not romacs-perf-mx-tab--timed)
      ;; Warm-up: identical work, zero recording. Even the redisplay runs,
      ;; so the completion window machinery warms exactly as in a timed
      ;; call.
      (progn
        (apply original arguments)
        (redisplay t)
        (setq romacs-perf-mx-tab--warmup-calls
              (1+ romacs-perf-mx-tab--warmup-calls)))
    (let ((started (car (current-cpu-time))))
      (prog1 (apply original arguments)
      ;; The workload measures the point at which the completion window is
      ;; actually presentable, not merely the end of candidate enumeration.
      (redisplay t)
      (setq romacs-perf-mx-tab--elapsed-us
            (+ romacs-perf-mx-tab--elapsed-us
               (- (car (current-cpu-time)) started))
            romacs-perf-mx-tab--help-calls
            (1+ romacs-perf-mx-tab--help-calls)
            romacs-perf-mx-tab--completion-visible
            (and romacs-perf-mx-tab--completion-visible
                 (get-buffer-window "*Completions*" t))
            romacs-perf-mx-tab--completion-mode-correct
            (and romacs-perf-mx-tab--completion-mode-correct
                 (with-current-buffer "*Completions*"
                   (derived-mode-p 'completion-list-mode))))
      ;; Validate the returned completion data after stopping the timer, so
      ;; the oracle's list walks cannot improve or regress the measurement.
      (setq romacs-perf-mx-tab--candidate-count-stable
            (and romacs-perf-mx-tab--candidate-count-stable
                 (or (null romacs-perf-mx-tab--candidate-count)
                     (= romacs-perf-mx-tab--candidate-count
                        romacs-perf-mx-tab--current-candidate-count)))
            romacs-perf-mx-tab--candidate-count
            romacs-perf-mx-tab--current-candidate-count
            romacs-perf-mx-tab--known-commands-present
            (and romacs-perf-mx-tab--known-commands-present
                 (romacs-perf-mx-tab--candidate-present-p
                  "romacs-perf-command-0000"
                  romacs-perf-mx-tab--last-completions)
                 (romacs-perf-mx-tab--candidate-present-p
                  "romacs-perf-command-1023"
                  romacs-perf-mx-tab--last-completions)))))))

(defun romacs-perf-mx-tab--write-result
    (path scenario-name status iterations elapsed-us completion-help-calls
          warmup-completion-help-calls
          completion-visible completion-mode-correct known-commands-present
          completion-candidate-count candidate-count-stable
          completion-hidden-after-exit minibuffer-depth-restored
          selected-buffer-restored error-message)
  (with-temp-file path
    (insert
     (json-serialize
      `((schema_version . 1)
        (scenario . ,scenario-name)
        (status . ,status)
        (iterations . ,iterations)
        (elapsed_us . ,elapsed-us)
        (completion_help_calls . ,completion-help-calls)
        (warmup_completion_help_calls . ,warmup-completion-help-calls)
        (completion_visible . ,(romacs-perf--json-boolean completion-visible))
        (completion_mode_correct
         . ,(romacs-perf--json-boolean completion-mode-correct))
        (known_commands_present
         . ,(romacs-perf--json-boolean known-commands-present))
        (completion_candidate_count . ,completion-candidate-count)
        (candidate_count_stable
         . ,(romacs-perf--json-boolean candidate-count-stable))
        (completion_hidden_after_exit
         . ,(romacs-perf--json-boolean completion-hidden-after-exit))
        (minibuffer_depth_restored
         . ,(romacs-perf--json-boolean minibuffer-depth-restored))
        (selected_buffer_restored
         . ,(romacs-perf--json-boolean selected-buffer-restored))
        (error . ,error-message))
      :false-object :json-false
      :null-object nil))))

(defun romacs-perf-mx-tab--run ()
  (let* ((result-path
          (romacs-perf--required-environment "ROMACS_PERF_RESULT"))
         (sentinel-path (romacs-perf--required-environment "SENTINEL"))
         (iterations
          (string-to-number
           (romacs-perf--required-environment "ROMACS_PERF_ITERATIONS")))
         (initial-buffer (current-buffer))
         (warmup-completions
          (string-to-number
           (or (getenv "ROMACS_PERF_WARMUP_COMPLETIONS") "0")))
         (scenario-name
          (or (getenv "ROMACS_PERF_SCENARIO_ID") "mx-tab-completion"))
         (completed 0)
         (completion-hidden-after-exit t)
         (minibuffer-depth-restored t)
         (selected-buffer-restored t)
         (status "error")
         (error-message nil)
         (exit-code 2))
    (condition-case error-data
        (progn
          (unless (> iterations 0)
            (error "iterations must be positive"))
          (romacs-perf-mx-tab--install-controlled-commands)
          (advice-add 'minibuffer-completion-help :around
                      #'romacs-perf-mx-tab--around-completion-help)
          (advice-add 'completion-all-completions :around
                      #'romacs-perf-mx-tab--capture-completions)
          ;; Warm-up pass: identical completions, untimed, invariants
          ;; unchecked, and BEFORE the sampling gate so a profile never
          ;; attributes warm-up to the workload. Zero by default, so the
          ;; cold row measures start + warm-up as it always has.
          (let ((romacs-perf-mx-tab--timed nil))
            (dotimes (_ warmup-completions)
              (setq romacs-perf-mx-tab--last-completions nil
                    romacs-perf-mx-tab--current-candidate-count 0)
              (execute-kbd-macro
               (vconcat (kbd "M-x") "romacs-perf-command-"
                        (kbd "TAB") "0000" (kbd "RET")))))
          (let ((sampling-enabled nil))
            (romacs-perf--sampling-command "enable")
            (setq sampling-enabled t)
            (unwind-protect
                (dotimes (_ iterations)
                  ;; Select the no-op command after displaying completions.
                  ;; This keeps the benchmark focused on M-x TAB and gives GNU
                  ;; and Romacs the same normal minibuffer-exit lifecycle.
                  (setq romacs-perf-mx-tab--last-completions nil
                        romacs-perf-mx-tab--current-candidate-count 0)
                  (execute-kbd-macro
                   (vconcat (kbd "M-x") "romacs-perf-command-"
                            (kbd "TAB") "0000" (kbd "RET")))
                  (setq completion-hidden-after-exit
                          (and completion-hidden-after-exit
                               (not (get-buffer-window "*Completions*" t)))
                          minibuffer-depth-restored
                          (and minibuffer-depth-restored
                               (zerop (minibuffer-depth)))
                          selected-buffer-restored
                          (and selected-buffer-restored
                               (eq initial-buffer (current-buffer)))
                          completed (1+ completed)))
              (when sampling-enabled
                (romacs-perf--sampling-command "disable"))))
          (setq status "ok"
                exit-code 0))
      (error
       (setq error-message (error-message-string error-data))
       (message "mx-tab-completion failed: %s" error-message)))
    (advice-remove 'minibuffer-completion-help
                   #'romacs-perf-mx-tab--around-completion-help)
    (advice-remove 'completion-all-completions
                   #'romacs-perf-mx-tab--capture-completions)
    (romacs-perf--close-profile-gate)
    (romacs-perf-mx-tab--write-result
     result-path scenario-name status completed romacs-perf-mx-tab--elapsed-us
     romacs-perf-mx-tab--help-calls
     romacs-perf-mx-tab--warmup-calls
     romacs-perf-mx-tab--completion-visible
     romacs-perf-mx-tab--completion-mode-correct
     romacs-perf-mx-tab--known-commands-present
     (or romacs-perf-mx-tab--candidate-count 0)
     romacs-perf-mx-tab--candidate-count-stable
     completion-hidden-after-exit minibuffer-depth-restored
     selected-buffer-restored error-message)
    (write-region "done\n" nil sentinel-path nil 'silent)
    (kill-emacs exit-code)))

(if noninteractive
    (romacs-perf-mx-tab--run)
  (run-at-time 0 nil #'romacs-perf-mx-tab--run))

;;; mx-tab-completion.el ends here
