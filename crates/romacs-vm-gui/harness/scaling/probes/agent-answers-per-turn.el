;;; agent-answers-per-turn.el --- scaling probe  -*- lexical-binding: t -*-

;; N drafts queued to a starting session in one turn, each its own
;; admission (romacs-agent.el, `romacs-agent-submit'): their answers are
;; held, and the counted work is their release, from the timers the turn
;; left until every answer has gone out and the session has been told it
;; changed.  Releasing them is linear in N.  Measured 2026-11-24 on beta
;; before that, with a release timer per admission and a timer per
;; answer, each set into a timer list as long as N: 188M instructions for
;; 100 answers, 1,650M for 300 and 18,090M for 1000, which blocked the
;; main VM for about 2 s.  N is 1000: a linear release of 100 answers does
;; too little work to be measured.
(scaling-probe agent-answers-per-turn :n 1000 :k 3
  :setup (progn
           (scaling--agent)
           (clrhash romacs-agent--sessions)
           (setq romacs-agent-event-functions nil romacs-agent-input-functions nil)
           (let* ((id "aaaaaaaa-0000-4000-8000-000000000000")
                  (session (romacs-agent--make :id id :harness 'claude :state 'starting :title "answers"
                                               :dir (expand-file-name id (romacs-agent--state-root))))
                  (state (list 0 nil)))
             (make-directory (romacs-agent-dir session) t)
             (puthash id session romacs-agent--sessions)
             (add-hook 'romacs-agent-event-functions
                       (lambda (_session event) (when (eq (plist-get event :type) 'changed) (setcar (cdr state) t))))
             (dotimes (i n)
               (romacs-agent-submit id (list :draft-id (romacs-agent--uuid) :kind 'queue
                                             :text (format "draft %d" i) :attachments nil)
                                    (lambda (answer)
                                      (when (eq (plist-get answer :status) 'accepted)
                                        (setcar state (1+ (car state)))))))
             (unless (and (zerop (car state)) (not (cadr state)))
               (error "agent-answers-per-turn: answered or told within the turn"))
             state))
  ;; A release that stops making progress fails here, not on the time limit.
  :body (let ((seen nil) (since (float-time)))
          (while (not (and (= (car it) n) (cadr it)))
            (unless (equal seen it)
              (setq seen (copy-sequence it) since (float-time)))
            (when (> (- (float-time) since) 10)
              (error "agent-answers-per-turn: %d of %d accepted, told %s" (car it) n (cadr it)))
            (accept-process-output nil 0.001))))
