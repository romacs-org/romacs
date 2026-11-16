;;; romacs-agent-writer-tests.el --- The agents' store written off the editor -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-events)
  (require 'romacs-agent-history))

(defmacro romacs-agent-writer-tests--with (&rest body)
  "BODY as the editor, its writes queued to a writer that runs here: what
it is sent waits until `serve' runs the writer's loop over it, with a
connection of its own, as the writer's process has."
  (declare (indent 0))
  `(let ((sent nil) (romacs-agent-writer-queued t) (romacs-agent-writer--job nil)
         (romacs-agent-writer--pending (make-hash-table)) (romacs-agent-writer--unsent nil)
         (romacs-agent-writer--unsent-tail nil) (romacs-agent-writer--next 0) (romacs-agent-writer--in-flight 0)
         (romacs-agent-writer--failed (make-hash-table)))
     (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (vector 'future)))
               ((symbol-function 'isolate-on-done) #'ignore)
               ((symbol-function 'isolate-future-done-p) #'ignore)
               ((symbol-function 'isolate-future-isolate) (lambda (_) 'writer))
               ((symbol-function 'isolate-send)
                (lambda (to message)
                  (if (eq to 'parent) (romacs-agent-writer--receive 'writer message)
                    (setq sent (append sent (list message)))))))
       (cl-labels ((serve-only ()
                   (let ((inbox sent) (romacs-agent-store--db nil) (romacs-agent-events--made nil)
                         (romacs-agent-turns--made nil) (romacs-agent-store--readonly nil))
                     (setq sent nil)
                     (cl-letf (((symbol-function 'isolate-receive)
                                (lambda (&rest _) (or (pop inbox) '(romacs-agent-writer-stop)))))
                       (unwind-protect (romacs-agent-writer--serve)
                         (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))))))
                 (serve ()
                   (setq sent (append sent (list '(romacs-agent-writer-stop))))
                   (serve-only)))
         ;; Not every test runs the writer.
         (ignore #'serve #'serve-only)
         ,@body))))

(ert-deftest romacs-agent-writer-writes-in-order-and-answers ()
  ;; Queued, a write answers later, in the order written, its value or
  ;; its error; nothing waits for it.
  (let ((romacs-agent-writer-start-functions nil) (answers nil))
    (romacs-agent-writer-tests--with
      (should (eq (romacs-agent-write 'list '(1) (lambda (answer) (push answer answers))) 'queued))
      (romacs-agent-write 'error '("Boom") (lambda (answer) (push (car answer) answers)))
      (should-not answers)
      (should (= (romacs-agent-writer-pending-count) 2))
      (serve)
      (should (equal (nreverse answers) '((ok (1)) error)))
      (should (zerop (romacs-agent-writer-pending-count))))
    ;; Not queued (a batch process, a worker): here, now.
    (let ((romacs-agent-writer-queued nil))
      (should (equal (romacs-agent-write 'list '(2)) '(ok (2)))))))

(ert-deftest romacs-agent-writer-a-writer-that-ends-gets-the-rest-again ()
  ;; The writer gone with writes unanswered: a new one is sent them again,
  ;; in order, and each is answered once.
  (let ((romacs-agent-writer-start-functions nil) (answers nil))
    (romacs-agent-writer-tests--with
      (romacs-agent-write 'list '(1) (lambda (answer) (push answer answers)))
      (romacs-agent-write 'list '(2) (lambda (answer) (push answer answers)))
      (setq sent nil)
      (romacs-agent-writer--ended romacs-agent-writer--job)
      (should (equal (mapcar #'car (cadr (car sent))) '(1 2)))
      (serve)
      (should (equal (nreverse answers) '((ok (1)) (ok (2))))))))

(defvar romacs-agent-writer-tests--made nil "What the writer made, in order.")
(defun romacs-agent-writer-tests--make (x) (push x romacs-agent-writer-tests--made) x)

(ert-deftest romacs-agent-writer-a-drain-makes-every-write ()
  ;; Ending (or handing over to a successor), the editor waits for every
  ;; write it queued, however many, saying so; a writer that died is
  ;; started again with what it did not answer.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil) (said nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'isolate-await) (lambda (&rest _) (serve) t))
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (dotimes (i 500) (romacs-agent-write 'romacs-agent-writer-tests--make (list i)))
        (romacs-agent-writer-drain)
        (should (equal (reverse romacs-agent-writer-tests--made) (number-sequence 0 499)))
        (should (zerop (romacs-agent-writer-pending-count)))
        (should (member "Saving 500 agent records..." said))
        (should (member "Saving 500 agent records...done" said))
        ;; The writer gone with two unanswered: a new one makes them.
        (setq romacs-agent-writer-tests--made nil)
        (romacs-agent-write 'romacs-agent-writer-tests--make '(a))
        (romacs-agent-write 'romacs-agent-writer-tests--make '(b))
        (setq sent nil)
        (let ((dead romacs-agent-writer--job))
          (cl-letf (((symbol-function 'isolate-future-done-p) (lambda (future) (eq future dead))))
            (romacs-agent-writer-drain)))
        (should (equal (reverse romacs-agent-writer-tests--made) '(a b)))
        (should (zerop (romacs-agent-writer-pending-count)))
        ;; A write's answer that queues another: that one is made too,
        ;; before the writer stops.
        (setq romacs-agent-writer-tests--made nil)
        (romacs-agent-write 'romacs-agent-writer-tests--make '(c)
                            (lambda (_) (romacs-agent-write 'romacs-agent-writer-tests--make '(d))))
        ;; The writer makes what it was sent, up to a stop, as it comes.
        (cl-letf (((symbol-function 'isolate-await) (lambda (&rest _) (serve-only) t)))
          (romacs-agent-writer-drain))
        (should (equal (reverse romacs-agent-writer-tests--made) '(c d)))
        (should (zerop (romacs-agent-writer-pending-count)))
        ;; A write queued while the writer stops (a timer's): made by a
        ;; new writer before the drain returns.
        (setq romacs-agent-writer-tests--made nil)
        (romacs-agent-write 'romacs-agent-writer-tests--make '(e))
        (let ((once nil))
          (cl-letf (((symbol-function 'isolate-await)
                     (lambda (&rest _)
                       (let ((stopping (member '(romacs-agent-writer-stop) sent)))
                         (serve-only)
                         (when (and stopping (not once))
                           (setq once t)
                           (romacs-agent-write 'romacs-agent-writer-tests--make '(f))))
                       t)))
            (romacs-agent-writer-drain)))
        (should (equal (reverse romacs-agent-writer-tests--made) '(e f)))
        (should (zerop (romacs-agent-writer-pending-count)))))))

(ert-deftest romacs-agent-writer-a-burst-waits-for-room-never-errs ()
  ;; Thousands of writes at once: never more batches in the writer's
  ;; queue than the window, never an error to the caller, all made, in order.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil) (rounds 0))
    (romacs-agent-writer-tests--with
      (dotimes (i 20000) (should (eq (romacs-agent-write 'romacs-agent-writer-tests--make (list i)) 'queued)))
      (should (<= (length sent) romacs-agent-writer--window))
      (while (and (> (romacs-agent-writer-pending-count) 0) (< (cl-incf rounds) 100))
        (should (<= (length sent) romacs-agent-writer--window))
        (serve-only))
      (should (zerop (romacs-agent-writer-pending-count)))
      (should (equal (reverse romacs-agent-writer-tests--made) (number-sequence 0 19999)))
      ;; Nothing left: the writer only stops.
      (serve))))

(defvar romacs-agent-writer-tests--flaky 0 "Failures left for the flaky write.")
(defun romacs-agent-writer-tests--flaky (x)
  (if (> romacs-agent-writer-tests--flaky 0)
      (progn (cl-decf romacs-agent-writer-tests--flaky) (error "Database is locked"))
    (romacs-agent-writer-tests--make x)))
(defun romacs-agent-writer-tests--broken (_x) (error "Disk full"))

(ert-deftest romacs-agent-writer-no-writer-waits-then-the-end-writes-here ()
  ;; No writer can be started (its world gone stale under the editor):
  ;; the editor commits nothing, the writes wait and a writer is tried
  ;; again; an ending editor makes them itself, each tried again while it
  ;; fails, one that never succeeds kept and said.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil)
        (romacs-agent-writer--retry-timer nil) (romacs-agent-writer-tests--flaky 2) (said nil) (retried nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (signal 'isolate-world-unavailable '("stale"))))
                ((symbol-function 'run-with-timer) (lambda (&rest _) (setq retried t) 'timer))
                ((symbol-function 'timerp) (lambda (x) (eq x 'timer)))
                ((symbol-function 'sleep-for) #'ignore)
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (romacs-agent-write 'romacs-agent-writer-tests--make '(1))
        (romacs-agent-write 'romacs-agent-writer-tests--flaky '(2))
        (romacs-agent-write 'romacs-agent-writer-tests--broken '(3))
        (should-not romacs-agent-writer-tests--made)
        (should (= (romacs-agent-writer-pending-count) 3))
        (should retried)
        (should-not sent)
        (romacs-agent-writer-drain)
        (should (equal (reverse romacs-agent-writer-tests--made) '(1 2)))
        (should (= (romacs-agent-writer-pending-count) 1))
        (should (member "Saving 3 agent records...failed: 1 not saved" said))))))

(ert-deftest romacs-agent-writer-a-drain-outlasts-writers-that-die ()
  ;; Every writer the drain starts dies before it answers: past its tries
  ;; the ending editor makes the writes itself; none is left, none lost.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil) (spawned 0))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (cl-incf spawned) (vector 'doomed)))
                ((symbol-function 'isolate-future-done-p) (lambda (_) t))
                ((symbol-function 'isolate-await) (lambda (&rest _) (signal 'isolate-died nil)))
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) #'ignore))
        (dotimes (i 3) (romacs-agent-write 'romacs-agent-writer-tests--make (list i)))
        (romacs-agent-writer-drain)
        (should (> spawned 1))
        (should (equal (reverse romacs-agent-writer-tests--made) '(0 1 2)))
        (should (zerop (romacs-agent-writer-pending-count)))))))

(ert-deftest romacs-agent-writer-a-failed-commit-is-not-saved ()
  ;; A kept state whose commit fails (a full disk) signals: the drain's
  ;; last resort keeps it and says it was not saved, never "done".
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-turns)
  (let* ((root (make-temp-file "romacs-writer-" t))
         (process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment))
         (romacs-agent-store--db nil) (romacs-agent-turns--made nil) (romacs-agent-events--made nil)
         (romacs-agent-writer-start-functions nil) (said nil))
    (unwind-protect
        (progn
          (make-directory (expand-file-name "romacs/agents" (concat root "/state")) t)
          (let ((execute (symbol-function 'sqlite-execute)))
            (cl-letf (((symbol-function 'sqlite-execute)
                       (lambda (db query &rest args)
                         (if (equal query "commit") (signal 'sqlite-error '("disk I/O error")) (apply execute db query args)))))
              (let ((romacs-agent-writer-queued nil))
                (should-error (romacs-agent-turns-fold-put "s" "(:a 1)")))
              (romacs-agent-writer-tests--with
                (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (signal 'isolate-world-unavailable '("stale"))))
                          ((symbol-function 'run-with-timer) (lambda (&rest _) nil))
                          ((symbol-function 'sleep-for) #'ignore)
                          ((symbol-function 'redisplay) #'ignore)
                          ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
                  (romacs-agent-write 'romacs-agent-turns-fold-put '("s" "(:a 2)"))
                  (romacs-agent-writer-drain)
                  (should (= (romacs-agent-writer-pending-count) 1))
                  (should (member "Saving 1 agent records...failed: 1 not saved" said)))))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(ert-deftest romacs-agent-writer-no-store-is-not-saved ()
  ;; A record whose store cannot be opened signals: the drain keeps it
  ;; and says it was not saved.
  (let ((romacs-agent-writer-start-functions nil) (said nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (signal 'isolate-world-unavailable '("stale"))))
                ((symbol-function 'romacs-agent-events--db) #'ignore)
                ((symbol-function 'run-with-timer) (lambda (&rest _) nil))
                ((symbol-function 'sleep-for) #'ignore)
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (romacs-agent-write 'romacs-agent-events--insert (list (list "s" 1 1.0 "note" "{}")))
        (romacs-agent-writer-drain)
        (should (= (romacs-agent-writer-pending-count) 1))
        (should (member "Saving 1 agent records...failed: 1 not saved" said))))))

(ert-deftest romacs-agent-writer-a-write-the-writer-fails-is-kept ()
  ;; The writer answers a write with an error: it is kept, sent again
  ;; later, and at the end made by the editor or counted not saved.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil)
        (romacs-agent-writer--failed (make-hash-table)) (romacs-agent-writer-tests--flaky 1)
        (later nil) (said nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'run-with-timer) (lambda (_time _repeat f &rest args) (push (cons f args) later) nil))
                ((symbol-function 'sleep-for) #'ignore)
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (romacs-agent-write 'romacs-agent-writer-tests--flaky '(1))
        (romacs-agent-write 'romacs-agent-writer-tests--broken '(2))
        (serve)
        (should (= (romacs-agent-writer-pending-count) 2))
        (should (= (romacs-agent-writer--unanswered) 0))
        (should (= (length later) 2))
        ;; Sent again: the flaky one is made now.
        (dolist (call later) (apply (car call) (cdr call)))
        (serve)
        (should (equal romacs-agent-writer-tests--made '(1)))
        (should (= (romacs-agent-writer-pending-count) 1))
        ;; The end: the broken one tried again, then counted not saved.
        (cl-letf (((symbol-function 'isolate-await) (lambda (&rest _) (serve-only) t)))
          (romacs-agent-writer-drain))
        (should (= (romacs-agent-writer-pending-count) 1))
        (should (member "Saving 1 agent records...failed: 1 not saved" said))))))

(ert-deftest romacs-agent-writer-a-requested-restart-waits-for-the-writes ()
  ;; A requested restart waits while agent records are not yet written
  ;; (`warm-restart-obstacle-functions', the restart's);
  ;; a restart begun by hand drains them, and C-g cancels it, the writes
  ;; still queued.
  (require 'warm-restart)
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) #'ignore))
        (romacs-agent-write 'romacs-agent-writer-tests--make '(1))
        (should (equal (romacs-agent-writer--obstacle) '("1 agent records not yet written")))
        ;; C-g while it drains: the restart stops, nothing lost.
        (cl-letf (((symbol-function 'isolate-await) (lambda (&rest _) (signal 'quit nil))))
          (should (eq (condition-case nil (progn (run-hooks 'warm-restart-before-handover-functions) 'restarted)
                        (quit 'cancelled))
                      'cancelled)))
        (should (= (romacs-agent-writer-pending-count) 1))
        ;; Made meanwhile: no obstacle.
        (serve)
        (should (equal romacs-agent-writer-tests--made '(1)))
        (should-not (romacs-agent-writer--obstacle))))))

(defvar romacs-agent-writer-tests--values nil "What keyed writes put, by key.")
(defun romacs-agent-writer-tests--put (key value) (setf (alist-get key romacs-agent-writer-tests--values nil nil #'equal) value))
(defun romacs-agent-writer-tests--put-fails (_key _value) (error "Disk full"))

(ert-deftest romacs-agent-writer-a-later-write-of-a-key-wins ()
  ;; Two writes of one thing, the first failing: the second's value stays,
  ;; the first is never tried again over it.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--values nil) (later nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'run-with-timer) (lambda (_time _repeat f &rest args) (push (cons f args) later) nil))
                ((symbol-function 'message) #'ignore))
        ;; Both queued before either is answered.
        (romacs-agent-write 'romacs-agent-writer-tests--put-fails '("k" old) nil '(row "k"))
        (romacs-agent-write 'romacs-agent-writer-tests--put '("k" new) nil '(row "k"))
        (serve)
        (should (equal romacs-agent-writer-tests--values '(("k" . new))))
        (should (zerop (romacs-agent-writer-pending-count)))
        (should-not later)
        ;; The first fails alone, kept; a later one of its key supersedes it.
        (romacs-agent-write 'romacs-agent-writer-tests--put-fails '("j" old) nil '(row "j"))
        (serve)
        (should (= (romacs-agent-writer-pending-count) 1))
        (romacs-agent-write 'romacs-agent-writer-tests--put '("j" new) nil '(row "j"))
        (serve)
        (dolist (call later) (apply (car call) (cdr call)))
        (serve)
        (should (equal (alist-get "j" romacs-agent-writer-tests--values nil nil #'equal) 'new))
        (should (zerop (romacs-agent-writer-pending-count)))))))

(ert-deftest romacs-agent-writer-a-last-keep-that-fails-is-not-saved ()
  ;; The last keep of a session fails as the editor ends: it is kept,
  ;; tried by the drain and then here, and counted not saved, never "done".
  (let ((romacs-agent-writer-start-functions nil) (said nil)
        (session (romacs-agent--make :id "k")))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'romacs-agent-turns-fold-put) (lambda (&rest _) (error "Disk full")))
                ((symbol-function 'run-with-timer) (lambda (&rest _) nil))
                ((symbol-function 'sleep-for) #'ignore)
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (should (romacs-agent--kept-put session "(:a 1)"))
        (serve)
        (should (= (romacs-agent-writer-pending-count) 1))
        (cl-letf (((symbol-function 'isolate-await) (lambda (&rest _) (serve-only) t)))
          (romacs-agent-writer-drain))
        (should (= (romacs-agent-writer-pending-count) 1))
        (should (member "Saving 1 agent records...failed: 1 not saved" said))))))

(ert-deftest romacs-agent-writer-a-cancelled-restart-leaves-its-writes-to-the-writer ()
  ;; No writer to be had, a restart drains here, and C-g cancels it in a
  ;; retry's pause: the writes are queued again, and made once a writer
  ;; can be had, in the background.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil)
        (romacs-agent-writer--retry-timer nil) (available nil) (soon nil))
    (romacs-agent-writer-tests--with
      (cl-letf* ((spawn (symbol-function 'isolate-spawn))
                 ((symbol-function 'isolate-spawn)
                  (lambda (&rest args) (if available (apply spawn args) (signal 'isolate-world-unavailable '("stale")))))
                 ((symbol-function 'run-with-timer) (lambda (_time _repeat f &rest args) (push (cons f args) soon) 'timer))
                 ((symbol-function 'timerp) (lambda (x) (eq x 'timer)))
                 ((symbol-function 'romacs-agent-writer-tests--flaky) (lambda (&rest _) (error "Locked")))
                 ((symbol-function 'sleep-for) (lambda (&rest _) (signal 'quit nil)))
                 ((symbol-function 'redisplay) #'ignore)
                 ((symbol-function 'message) #'ignore))
        (romacs-agent-write 'romacs-agent-writer-tests--flaky '(1))
        (romacs-agent-write 'romacs-agent-writer-tests--make '(2))
        (should (eq (condition-case nil (progn (romacs-agent-writer--before-handover) 'restarted) (quit 'cancelled))
                    'cancelled))
        (should (> (romacs-agent-writer-pending-count) 0))
        ;; A writer can be had again: the background sends what is left.
        (setq available t)
        (cl-letf (((symbol-function 'romacs-agent-writer-tests--flaky) #'romacs-agent-writer-tests--make))
          (dolist (call soon) (apply (car call) (cdr call)))
          (serve))
        (should (zerop (romacs-agent-writer-pending-count)))
        (should (equal (sort (copy-sequence romacs-agent-writer-tests--made) #'<) '(1 2)))))))

(ert-deftest romacs-agent-writer-notes-are-made-before-a-launch ()
  ;; After a move, what waits runs in two steps: the notes' writes, then,
  ;; once the writer has answered a write behind them, the launch.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil)
        (romacs-agent-events--waiting (make-hash-table :test #'equal)) (launched nil))
    (romacs-agent-writer-tests--with
      (puthash "s" (list (cons t (lambda () (romacs-agent-write 'romacs-agent-writer-tests--make '(note))))
                         (cons nil (lambda () (setq launched (reverse romacs-agent-writer-tests--made)))))
               romacs-agent-events--waiting)
      (romacs-agent-events--waited "s")
      (should-not launched)
      ;; A launch that comes before the notes are made waits behind them.
      (let ((late nil) (session (romacs-agent--make :id "s" :dir "/nonexistent/s")))
        (cl-letf (((symbol-function 'romacs-agent-events-own) #'ignore)
                  ((symbol-function 'romacs-agent-events-p) (lambda (_) t)))
          (romacs-agent-events-then session (lambda () (setq late (reverse romacs-agent-writer-tests--made)))))
        (should-not late)
        (serve)
        (should (equal launched '(note)))
        (should (equal late '(note))))
      (should-not (gethash "s" romacs-agent-events--waiting))
      ;; Notes alone: the session waits until they are made, then is let go.
      (setq romacs-agent-writer-tests--made nil)
      (puthash "s" (list (cons t (lambda () (romacs-agent-write 'romacs-agent-writer-tests--make '(alone)))))
               romacs-agent-events--waiting)
      (romacs-agent-events--waited "s")
      (should (gethash "s" romacs-agent-events--waiting))
      (serve)
      (should (equal romacs-agent-writer-tests--made '(alone)))
      (should-not (gethash "s" romacs-agent-events--waiting)))))

(ert-deftest romacs-agent-writer-a-flip-made-before-a-writer-ended-is-moved ()
  ;; A flip the writer made before it ended, sent again, answers nil: the
  ;; store says imported, so the editor does what a move does.
  (let ((romacs-agent-events--flip-queue nil) (waited nil) (told nil)
        (romacs-agent--sessions (make-hash-table :test #'equal)))
    (cl-letf (((symbol-function 'romacs-agent-events--db) (lambda () 'db))
              ((symbol-function 'sqlite-select) (lambda (_db query &rest _) (if (string-search "imported" query) '((1)) '((10 3)))))
              ((symbol-function 'romacs-agent-events--back-up-moved) #'ignore)
              ((symbol-function 'romacs-agent-events--waited) (lambda (id) (push id waited)))
              ((symbol-function 'romacs-agent-write) (lambda (_f _a done) (funcall done '(ok nil))))
              ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
      (let ((romacs-agent-events-migrated-functions (list (lambda (ids) (setq told ids)))))
        (romacs-agent-events--flip-soon '("s")))
      (should (equal told '("s")))
      (should (equal waited '("s"))))))

(ert-deftest romacs-agent-writer-the-editor-reads-and-an-import-waits ()
  ;; The editor's connection only reads; a launch on a session whose
  ;; journal is imported by the writer waits for it, and finds it imported.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-writer-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-store--readonly nil) (romacs-agent-store--making nil)
         (romacs-agent-events--made nil) (romacs-agent-turns--made nil)
         (romacs-agent-events--imported (make-hash-table :test #'equal))
         (romacs-agent-events--importing (make-hash-table :test #'equal))
         (romacs-agent-events--waiting (make-hash-table :test #'equal))
         (romacs-agent-events-store t)
         (id "e7e70000-2222-4333-8444-5555666677aa")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (session (romacs-agent--make :id id :dir dir)))
    (unwind-protect
        (romacs-agent-writer-tests--with
          (make-directory dir t)
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region "{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1.0}\n" nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          ;; No store yet: none to read, the writer asked to make it.
          (should-not (romacs-agent-store--open))
          (should (equal (mapcar #'car sent) '(romacs-agent-write-batch)))
          (serve)
          (let ((db (romacs-agent-store--open)))
            (should db)
            (should (romacs-agent-store-readonly-p))
            (should-error (sqlite-execute db "insert into board_folds values ('/p')")))
          ;; Opened to write before writes were queued (early in a start):
          ;; opened again, to read, once they are.
          (let ((romacs-agent-writer-queued nil)) (should (romacs-agent-store--open)) (should-not (romacs-agent-store-readonly-p)))
          (should (romacs-agent-store--open))
          (should (romacs-agent-store-readonly-p))
          ;; A launch waits for the writer's import.
          (let ((ran nil))
            (romacs-agent-events-then session (lambda () (push (romacs-agent-events-p session) ran)))
            (should-not ran)
            (should-not (romacs-agent-events-p session))
            (serve)
            (should (equal ran '(t)))
            (should (equal (caar (sqlite-select (romacs-agent-store--open) "select count(*) from events where session = ?" (list id))) 1))
            ;; Its journal backed up here, once imported.
            (should-not (file-exists-p (expand-file-name "journal.jsonl" dir)))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(provide 'romacs-agent-writer-tests)
;;; romacs-agent-writer-tests.el ends here
