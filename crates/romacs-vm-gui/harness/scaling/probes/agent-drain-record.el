;;; agent-drain-record.el --- scaling probe  -*- lexical-binding: t -*-

;; One supervisor record of N MB, read as 64 KB chunks through the
;; attachment's filter and handled by its drain (romacs-agent.el,
;; `romacs-agent--drain'): its chunks are joined once its line's end has
;; come, each byte copied once, and a slice handles a line whatever finding
;; it cost, so the work is linear in the record.  Measured 2026-11-21 on
;; beta before that: 1, 4 and 8 MB took 115M, 739M and 206,033M
;; instructions, and 16 MB never ended, each slice ending before its first
;; line.  The record's handler is stubbed: the probe holds the framing.
;; Its clock is held still, so a slice never ends on time and the count
;; is the framing's alone, the same on any host; the slice's own deadline
;; is the reattach tests' (a slice handles a line however long its search).

(defun scaling--agent-drain-record (mb)
  "One stream-json record of MB megabytes, as a list of 64 KB chunks."
  (let ((line (concat "{\"seq\":1,\"type\":\"user\",\"text\":\"" (make-string (* mb 1048576) ?x) "\"}\n"))
        (chunks nil) (start 0))
    (while (< start (length line))
      (push (substring line start (min (length line) (+ start 65536))) chunks)
      (setq start (+ start 65536)))
    (nreverse chunks)))

(scaling-probe agent-drain-record :n 1 :k 16
  :setup (progn
           (scaling--agent)
           ;; The session is made once romacs-agent is loaded, so its
           ;; constructor is called, not expanded here.
           (let* ((process (make-pipe-process :name "scaling-agent-drain-record" :noquery t))
                  (session (funcall #'romacs-agent--make :id "dddddddd-0000-4000-8000-000000000000"
                                    :harness 'claude :state 'running :dir "/nonexistent/d"
                                    :host-socket "/nonexistent/d.sock" :connection process)))
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
            (dolist (chunk (cadr it)) (romacs-agent--filter process chunk))
            ;; A drain that makes no progress fails here, not on the CPU limit.
            (while due
              (when (> (cl-incf turns) 1000) (error "agent-drain-record: no progress after %d turns" turns))
              (let ((timer (pop due))) (apply (car timer) (cdr timer)))))
          ;; The whole record, not a line made of its first and last chunks.
          (unless (and (= (length records) 1) (equal (gethash "seq" (car records)) 1)
                       (= (length (gethash "text" (car records))) (* n 1048576)))
            (error "agent-drain-record: %d records, not the one of %d MB" (length records) n))))
