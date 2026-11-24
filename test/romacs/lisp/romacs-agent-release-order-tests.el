;;; romacs-agent-release-order-tests.el --- Answers before observers, however many -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-agent--release' says it: each submit's caller gets one answer
;; after its call returns, every answer goes out first, and observers (a
;; session's `changed' event, `romacs-agent-input-functions') run only
;; after the last one; a callback's throw, error or quit touches its own
;; answer alone.  These pin that for many drafts admitted in one turn
;; (each its own admission) and within one hold (admissions inside one
;; another), whatever the callbacks do.  The admission's own cases are
;; romacs-agent-submit-admission-tests.el, whose session fixture this
;; follows.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-claude)
(require 'romacs-agent-test-attached)
(require 'romacs-agent-test-stored)

;;; Sessions, and what they were told in which order

(defun romacs-agent-release-order-tests--with (body &optional others)
  "Call BODY with a session, registered, attached and idle, in a store of
its own, its lines kept nowhere; then with OTHERS (a number) more."
  (romacs-agent-test-with-store
    (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent-event-functions nil)
           (romacs-agent-input-functions nil)
           (sessions
            (cl-loop for n from 0 to (or others 0)
                     collect (let* ((id (romacs-agent--uuid))
                                    (dir (expand-file-name id (romacs-agent--state-root)))
                                    (session (romacs-agent--make :id id :dir dir :harness 'claude :state 'idle
                                                                 :title (format "release %d" n))))
                               (make-directory dir t)
                               (puthash id session romacs-agent--sessions)
                               (romacs-agent-test-attach session t)))))
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent--line) #'ignore))
            (apply body sessions))
        (dolist (session sessions) (delete-process (romacs-agent-connection session)))))))

(defun romacs-agent-release-order-tests--intent (text &optional kind)
  (list :draft-id (romacs-agent--uuid) :kind (or kind 'queue) :text text :attachments nil))

(defun romacs-agent-release-order-tests--observing (log body)
  "Call BODY with every observer's run pushed on LOG's car, (event TYPE) or
\(input), as answers are by `romacs-agent-release-order-tests--submit'."
  (let ((romacs-agent-event-functions
         (list (lambda (_session event) (push (list 'event (plist-get event :type)) (car log)))))
        (romacs-agent-input-functions
         (list (lambda (&rest _) (push (list 'input) (car log))))))
    (funcall body)))

(defun romacs-agent-release-order-tests--submit (session-id i log &optional text then)
  "Submit the I-th draft, TEXT, to SESSION-ID, its answer pushed on LOG's
car as (answer I STATUS); then, as its callback's last act, THEN:
`error' signals, `quit' quits with quitting allowed, as C-g would, and
a function is called."
  (romacs-agent-submit
   session-id (romacs-agent-release-order-tests--intent (or text (format "draft %d" i)))
   (lambda (answer)
     (push (list 'answer i (plist-get answer :status)) (car log))
     (pcase then
       ('error (error "A callback that fails"))
       ('quit (let ((inhibit-quit nil)) (keyboard-quit)))
       ((pred functionp) (funcall then))))))

(defun romacs-agent-release-order-tests--drain (log n)
  "Run timers until LOG holds N answers and a `changed' event, 5 s at most,
then a while more, so a late answer would come too.  A quit out of a
timer is let go, as the command loop does, and timers run on; an error
is reported, as with `debug-on-error' off.  The number of quits seen."
  (let ((deadline (+ (float-time) 5)) (debug-on-error nil) (quits 0)
        (done (lambda ()
                (and (= (cl-count 'answer (car log) :key #'car) n)
                     (member '(event changed) (car log))))))
    (while (and (not (funcall done)) (< (float-time) deadline))
      (condition-case nil (accept-process-output nil 0.01) (quit (cl-incf quits))))
    (dotimes (_ 5)
      (condition-case nil (accept-process-output nil 0.02) (quit (cl-incf quits))))
    quits))

(defun romacs-agent-release-order-tests--order (log)
  "LOG's runs, oldest first, each answer as `answer' and the rest as they are."
  (mapcar (lambda (run) (if (eq (car run) 'answer) 'answer run)) (reverse (car log))))

(defun romacs-agent-release-order-tests--answers (log)
  "The indices of LOG's answers, in the order they came."
  (mapcar #'cadr (seq-filter (lambda (run) (eq (car run) 'answer)) (reverse (car log)))))

(defun romacs-agent-release-order-tests--answers-first (log n)
  "That LOG has its N answers first and observers only after them, one of
them its session's `changed'."
  (let ((order (romacs-agent-release-order-tests--order log)))
    (should (equal (seq-take order n) (make-list n 'answer)))
    (should-not (memq 'answer (nthcdr n order)))
    (should (member '(event changed) (nthcdr n order)))))

;;; Many drafts admitted in one turn, each its own admission

(defun romacs-agent-release-order-tests--one-turn (n)
  "N drafts queued to a running session in one turn: N answers, once each,
in admission order, before any observer."
  (romacs-agent-release-order-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let ((log (list nil)))
       (romacs-agent-release-order-tests--observing
        log (lambda ()
              (dotimes (i n) (romacs-agent-release-order-tests--submit (romacs-agent-id session) i log))
              (should-not (car log))
              (romacs-agent-release-order-tests--drain log n)))
       (should (equal (romacs-agent-release-order-tests--answers log) (number-sequence 0 (1- n))))
       (should (cl-every (lambda (run) (or (not (eq (car run) 'answer)) (eq (nth 2 run) 'accepted))) (car log)))
       (romacs-agent-release-order-tests--answers-first log n)
       (should (= (length (romacs-agent-queue session)) n))))))

(ert-deftest romacs-agent-release-order-two-in-one-turn ()
  (romacs-agent-release-order-tests--one-turn 2))

(ert-deftest romacs-agent-release-order-three-in-one-turn ()
  (romacs-agent-release-order-tests--one-turn 3))

(ert-deftest romacs-agent-release-order-fifty-in-one-turn ()
  (romacs-agent-release-order-tests--one-turn 50))

;;; Many drafts admitted within one hold, one admission inside another

(defun romacs-agent-release-order-tests--one-hold (n &optional thens)
  "A draft sent to an idle session whose line, as it is written, has N-1
more queued to a running one: N answers, once each, the inner ones in
their order, before any observer, the input hook included.  THENS, an
alist (I . THEN), says what the I-th draft's callback does after
recording its answer."
  (romacs-agent-release-order-tests--with
   (lambda (outer inner)
     (setf (romacs-agent-state inner) 'running)
     (let ((log (list nil)) (written nil) (quits 0))
       (romacs-agent-release-order-tests--observing
        log (lambda ()
              (cl-letf (((symbol-function 'romacs-agent--line)
                         (lambda (session &rest _)
                           (when (and (eq session outer) (not written))
                             (setq written t)
                             (cl-loop for i from 1 below n
                                      do (romacs-agent-release-order-tests--submit
                                          (romacs-agent-id inner) i log nil (alist-get i thens)))))))
                (romacs-agent-release-order-tests--submit (romacs-agent-id outer) 0 log "outer" (alist-get 0 thens))
                (should written)
                (should-not (car log))
                (setq quits (romacs-agent-release-order-tests--drain log n)))))
       (let ((answers (romacs-agent-release-order-tests--answers log)))
         (should (equal (sort (copy-sequence answers) #'<) (number-sequence 0 (1- n))))
         (should (equal (remq 0 answers) (number-sequence 1 (1- n)))))
       (should (member '(input) (car log)))
       (romacs-agent-release-order-tests--answers-first log n)
       quits))
   1))

(ert-deftest romacs-agent-release-order-three-in-one-hold ()
  (romacs-agent-release-order-tests--one-hold 3))

(ert-deftest romacs-agent-release-order-fifty-in-one-hold ()
  (romacs-agent-release-order-tests--one-hold 50))

;;; Callbacks that fail: a refusal, an error and a quit among the answers

(ert-deftest romacs-agent-release-order-mixed-turn ()
  ;; Accepted and refused drafts in one turn, one accepted draft's
  ;; callback signalling: every answer still goes out, in order, once,
  ;; and the observers after them (`romacs-agent--run-held' says an
  ;; observer's error, and the rest run; an answer's error touches its
  ;; own answer alone, `romacs-agent--release').
  (romacs-agent-release-order-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let ((log (list nil)) (id (romacs-agent-id session)) (nowhere (romacs-agent--uuid)))
       (romacs-agent-release-order-tests--observing
        log (lambda ()
              (romacs-agent-release-order-tests--submit id 0 log)
              (romacs-agent-release-order-tests--submit nowhere 1 log)
              (romacs-agent-release-order-tests--submit id 2 log nil 'error)
              (romacs-agent-release-order-tests--submit nowhere 3 log)
              (romacs-agent-release-order-tests--submit id 4 log nil 'error)
              (romacs-agent-release-order-tests--submit id 5 log)
              (romacs-agent-release-order-tests--drain log 6)))
       (should (equal (seq-filter (lambda (run) (eq (car run) 'answer)) (reverse (car log)))
                      '((answer 0 accepted) (answer 1 refused) (answer 2 accepted)
                        (answer 3 refused) (answer 4 accepted) (answer 5 accepted))))
       (romacs-agent-release-order-tests--answers-first log 6)
       (should (= (length (romacs-agent-queue session)) 4))))))

(ert-deftest romacs-agent-release-order-a-quit-in-one-turn-loses-no-answer ()
  ;; C-g from an answer's callback, quitting allowed: its own answer is
  ;; cut short, no other (`romacs-agent--release': "a callback's throw,
  ;; error or quit touches its own answer alone"), and the observers
  ;; still run after the last answer.
  (romacs-agent-release-order-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let ((log (list nil)) (id (romacs-agent-id session)))
       (romacs-agent-release-order-tests--observing
        log (lambda ()
              (romacs-agent-release-order-tests--submit id 0 log)
              (romacs-agent-release-order-tests--submit id 1 log nil 'quit)
              (romacs-agent-release-order-tests--submit id 2 log)
              (romacs-agent-release-order-tests--submit id 3 log nil 'quit)
              (romacs-agent-release-order-tests--drain log 4)))
       (should (equal (romacs-agent-release-order-tests--answers log) '(0 1 2 3)))
       (romacs-agent-release-order-tests--answers-first log 4)))))

;; A throw out of a callback leaves its caller's code, through whatever
;; was handing the answers out: the rest still go, after it.
(defun romacs-agent-release-order-tests--drain-thrown (log n tag)
  "Drain LOG of N answers as `romacs-agent-release-order-tests--drain'
does, a throw to TAG caught and the draining gone on with: the throws."
  (let ((thrown 0))
    (while (and (< thrown 5)
                (not (eq (catch tag (romacs-agent-release-order-tests--drain log n) 'drained) 'drained)))
      (cl-incf thrown))
    thrown))

(ert-deftest romacs-agent-release-order-a-throw-in-one-turn-loses-no-answer ()
  ;; A callback that throws out to its caller's catch, as
  ;; romacs-agent-submit-admission-each-answer-goes-alone's does: the
  ;; answers after it still go out, in order, once, and the observers
  ;; after the last (`romacs-agent--release': "a callback's throw, error
  ;; or quit touches its own answer alone").
  (romacs-agent-release-order-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let ((log (list nil)) (id (romacs-agent-id session)) (tag (make-symbol "answer")) (thrown 0))
       (romacs-agent-release-order-tests--observing
        log (lambda ()
              (dotimes (i 4)
                (romacs-agent-release-order-tests--submit id i log nil (and (= i 1) (lambda () (throw tag 'thrown)))))
              (setq thrown (romacs-agent-release-order-tests--drain-thrown log 4 tag))))
       (should (= thrown 1))
       (should (equal (romacs-agent-release-order-tests--answers log) '(0 1 2 3)))
       (romacs-agent-release-order-tests--answers-first log 4)))))

(ert-deftest romacs-agent-release-order-failing-callbacks-in-one-hold ()
  ;; Within one hold: an inner answer's callback fails, and the last
  ;; answer's quits as the hand-out ends; every answer goes out, once,
  ;; and the observers run after all of them.
  (romacs-agent-release-order-tests--one-hold 5 '((2 . error) (0 . quit))))

(provide 'romacs-agent-release-order-tests)
;;; romacs-agent-release-order-tests.el ends here
