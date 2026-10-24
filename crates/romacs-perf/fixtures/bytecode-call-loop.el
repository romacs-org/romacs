;;; bytecode-call-loop.el --- fixed-work Tier-0 call workload  -*- lexical-binding: t; -*-

(require 'bytecomp)
(require 'json)

(defun romacs-perf-bytecode-call--required-environment (name)
  (or (getenv name)
      (error "required performance environment variable %s is absent" name)))

(defvar romacs-perf-bytecode-call--profile-gate-process nil)
(defvar romacs-perf-bytecode-call--profile-gate-response "")

(defun romacs-perf-bytecode-call--profile-gate-filter (_process output)
  (setq romacs-perf-bytecode-call--profile-gate-response
        (concat romacs-perf-bytecode-call--profile-gate-response output)))

(defun romacs-perf-bytecode-call--profile-gate-connect ()
  (let* ((port-text (getenv "ROMACS_PERF_GATE_PORT"))
         (port (and port-text (string-to-number port-text))))
    (when (and port-text (not (> port 0)))
      (error "invalid bytecode-loop profile gate port %S" port-text))
    (when (and port-text
               (not (process-live-p
                     romacs-perf-bytecode-call--profile-gate-process)))
      (setq romacs-perf-bytecode-call--profile-gate-process
            (make-network-process
             :name "romacs-perf-bytecode-call-gate"
             :family 'ipv4
             :host "127.0.0.1"
             :service port
             :coding 'binary
             :noquery t
             :filter #'romacs-perf-bytecode-call--profile-gate-filter)))
    romacs-perf-bytecode-call--profile-gate-process))

(defun romacs-perf-bytecode-call--sampling-command (command)
  (let ((process (romacs-perf-bytecode-call--profile-gate-connect)))
    (when process
      (setq romacs-perf-bytecode-call--profile-gate-response "")
      (process-send-string process (concat command "\n"))
      (let ((deadline (+ (float-time) 30.0)))
        (while (and
                (not (and
                      (> (length
                          romacs-perf-bytecode-call--profile-gate-response)
                         0)
                      (= (aref
                          romacs-perf-bytecode-call--profile-gate-response
                          (1- (length
                               romacs-perf-bytecode-call--profile-gate-response)))
                         ?\n)))
                (< (float-time) deadline))
          (unless (process-live-p process)
            (error "bytecode-loop profile gate disconnected during %s" command))
          (accept-process-output process 0.05))
        (unless (equal romacs-perf-bytecode-call--profile-gate-response
                       "ack\n")
          (error "bytecode-loop profile gate rejected %s: %S"
                 command
                 romacs-perf-bytecode-call--profile-gate-response))))))

(defun romacs-perf-bytecode-call--close-profile-gate ()
  (when (processp romacs-perf-bytecode-call--profile-gate-process)
    (delete-process romacs-perf-bytecode-call--profile-gate-process)
    (setq romacs-perf-bytecode-call--profile-gate-process nil)))

(defun romacs-perf-bytecode-call--json-boolean (value)
  (if value t :json-false))

(defun romacs-perf-bytecode-call--identity (value)
  value)

(defun romacs-perf-bytecode-call--loop (iterations)
  (let ((last-value nil))
    (while (> iterations 0)
      (setq last-value (romacs-perf-bytecode-call--identity iterations)
            iterations (1- iterations)))
    last-value))

(dolist (function '(romacs-perf-bytecode-call--identity
                    romacs-perf-bytecode-call--loop))
  (byte-compile function))

(defun romacs-perf-bytecode-call--write-result
    (path status iterations elapsed-us bytecode-calls result expected-result
          bytecode-functions-compiled interpreter-requested error-message)
  (with-temp-file path
    (insert
     (json-serialize
      `((schema_version . 1)
        (scenario . "bytecode-call-loop")
        (status . ,status)
        (iterations . ,iterations)
        (elapsed_us . ,elapsed-us)
        (bytecode_calls . ,bytecode-calls)
        (result . ,result)
        (expected_result . ,expected-result)
        (bytecode_functions_compiled
         . ,(romacs-perf-bytecode-call--json-boolean
             bytecode-functions-compiled))
        (interpreter_requested
         . ,(romacs-perf-bytecode-call--json-boolean interpreter-requested))
        (error . ,error-message))
      :false-object :json-false
      :null-object nil))))

(defun romacs-perf-bytecode-call--run ()
  (let* ((result-path
          (romacs-perf-bytecode-call--required-environment
           "ROMACS_PERF_RESULT"))
         (sentinel-path
          (romacs-perf-bytecode-call--required-environment "SENTINEL"))
         (iterations
          (string-to-number
           (romacs-perf-bytecode-call--required-environment
            "ROMACS_PERF_ITERATIONS")))
         (expected-result 1)
         (elapsed-us 0)
         (bytecode-calls 0)
         (result 0)
         (bytecode-functions-compiled nil)
         (interpreter-requested t)
         (status "error")
         (error-message nil)
         (exit-code 2))
    (condition-case error-data
        (progn
          (unless (> iterations 0)
            (error "iterations must be positive"))
          (setq bytecode-functions-compiled
                (and
                 (byte-code-function-p
                  (symbol-function 'romacs-perf-bytecode-call--identity))
                 (byte-code-function-p
                  (symbol-function 'romacs-perf-bytecode-call--loop))))
          (unless bytecode-functions-compiled
            (error "the call workload did not compile to bytecode"))
          ;; Warm instruction/data caches before opening the sampling window.
          (romacs-perf-bytecode-call--loop 1000)
          (garbage-collect)
          (let ((sampling-enabled nil))
            (romacs-perf-bytecode-call--sampling-command "enable")
            (setq sampling-enabled t)
            (unwind-protect
                (let ((started (car (current-cpu-time))))
                  (setq result
                        (romacs-perf-bytecode-call--loop iterations)
                        elapsed-us (- (car (current-cpu-time)) started)
                        bytecode-calls iterations))
              (when sampling-enabled
                (romacs-perf-bytecode-call--sampling-command "disable"))))
          (unless (= result expected-result)
            (error "call loop returned %S, expected %S"
                   result expected-result))
          (setq status "ok"
                exit-code 0))
      (error
       (setq error-message (error-message-string error-data))
       (message "bytecode-call-loop failed: %s" error-message)))
    (romacs-perf-bytecode-call--close-profile-gate)
    (romacs-perf-bytecode-call--write-result
     result-path status iterations elapsed-us bytecode-calls result
     expected-result bytecode-functions-compiled interpreter-requested
     error-message)
    (write-region "done\n" nil sentinel-path nil 'silent)
    (kill-emacs exit-code)))

(if noninteractive
    (romacs-perf-bytecode-call--run)
  (run-at-time 0 nil #'romacs-perf-bytecode-call--run))

;;; bytecode-call-loop.el ends here
