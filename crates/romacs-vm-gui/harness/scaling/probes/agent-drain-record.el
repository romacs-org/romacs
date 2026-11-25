;;; agent-drain-record.el --- scaling probe  -*- lexical-binding: t -*-

;; One supervisor record of N MB, sent through `cat' to an attachment
;; framed in lines (romacs-agent.el, `romacs-agent--frame') and handled by
;; its drain (`romacs-agent--drain'): the process layer holds the line as
;; its reads come, each byte scanned once, and the filter gets it whole,
;; so the work is linear in the record.  Measured 2026-11-21 on beta
;; before the Lisp join: 1, 4 and 8 MB took 115M, 739M and 206,033M
;; instructions, and 16 MB never ended, each slice ending before its first
;; line.  The record's handler is stubbed: the probe holds the framing.
;; Its clock is held still, so a slice never ends on time and the count
;; is the framing's alone.

(defun scaling--agent-drain-record (mb)
  "One stream-json record of MB megabytes, its newline included."
  (concat "{\"seq\":1,\"type\":\"user\",\"text\":\"" (make-string (* mb 1048576) ?x) "\"}\n"))

(scaling-probe agent-drain-record :n 1 :k 16
  :setup (progn
           (scaling--agent)
           ;; The session is made once romacs-agent is loaded, so its
           ;; constructor is called, not expanded here.
           (let* ((process (make-process :name "scaling-agent-drain-record" :command '("cat") :noquery t
                                         :connection-type 'pipe :coding 'no-conversion
                                         :filter #'romacs-agent--filter))
                  (session (funcall #'romacs-agent--make :id "dddddddd-0000-4000-8000-000000000000"
                                    :harness 'claude :state 'running :dir "/nonexistent/d"
                                    :host-socket "/nonexistent/d.sock" :connection process)))
             (romacs-agent--frame process)
             (process-put process 'romacs-agent session)
             ;; An attachment already kept: its routing, done once at its
             ;; first line, is not the framing's work.
             (process-put process 'romacs-agent-kept t)
             (list process (scaling--agent-drain-record n))))
  :body (let ((process (car it)) (due nil) (records nil) (turns 0))
          (cl-letf (((symbol-function 'float-time) (lambda (&optional _) 0.0))
                    ((symbol-function 'run-at-time)
                     (lambda (_time _repeat function &rest args)
                       (setq due (append due (list (cons function args)))) 'timer))
                    ((symbol-function 'romacs-agent--record) (lambda (_session record) (push record records)))
                    ((symbol-function 'romacs-agent--raise) #'ignore))
            (process-send-string process (cadr it))
            ;; A drain that makes no progress fails here, not on the CPU limit.
            (while (or due (null records))
              (when (> (cl-incf turns) 100000) (error "agent-drain-record: no progress after %d turns" turns))
              (if due
                  (let ((timer (pop due))) (apply (car timer) (cdr timer)))
                (accept-process-output process 0.05))))
          ;; The whole record, not a line made of its first and last chunks.
          (unless (and (= (length records) 1) (equal (gethash "seq" (car records)) 1)
                       (= (length (gethash "text" (car records))) (* n 1048576)))
            (error "agent-drain-record: %d records, not the one of %d MB" (length records) n))))
