;;; owner_lifecycle_random.el --- Virtual transport for the lifecycle walk -*- lexical-binding: t; -*-
;; The real transition, effect interpreter, dispatch, admission and
;; callback runners see ordinary isolate/future records. Only the
;; transport and new native worker allocation on resubmit are virtual.

(defun rw-reset ()
  (setq romacs--isolates nil romacs--isolate-waiting nil
        rw-futures nil rw-closed nil rw-exited nil rw-requeued nil
        rw-callbacks nil rw-effects nil rw-next-frame nil rw-issues nil
        isolate-world nil romacs--isolate-world-refusal nil)
  (clrhash romacs--isolate-zygotes))

(defun rw-exit (id)
  (unless (memq id rw-exited) (push id rw-exited))
  (when (and (= id 1) (not (memq 3 rw-exited))) (push 3 rw-exited)))

(defun rw-native ()
  ;; A simulated process does not need wall-clock grace before SIGKILL.
  ;; Waiting after the fixture delivered it indicates a broken model.
  (setq isolate-kill-timeout 0)
  (fset 'romacs--isolate-wait (lambda (&rest _) (error "virtual transport cannot wait")))
  (fset 'romacs--isolate-pid (lambda (id) (unless (= id 3) (+ 200 id))))
  (fset 'romacs--isolate-signal
        (lambda (id signal)
          (when (and (= signal 9) (/= id 3)) (rw-exit id))))
  (fset 'romacs--isolate-reap (lambda (id) (and (memq id rw-exited) 0)))
  (fset 'romacs--isolate-signal-children (lambda (_id) nil))
  (fset 'romacs--isolate-post
        (lambda (id kind &rest _args)
          ;; The stopped zygote cannot read its bye. Other idle processes
          ;; accept a graceful end; live children still hold their zygote.
          (when (and (eq kind 'bye) (/= id 8)
                     (or (/= id 1)
                         (not (romacs--isolate-lingering-p
                               rw-main-zygote))))
            (rw-exit id))))
  (fset 'romacs--isolate-close (lambda (id) (push id rw-closed)))
  (fset 'romacs--isolate-take
        (lambda (id)
          (when (eq id (car rw-next-frame))
            (prog1 (cdr rw-next-frame) (setq rw-next-frame nil)))))
  (fset 'romacs--isolate-resubmit-or-fail
        (lambda (f)
          (unless (memq f rw-requeued)
            (push 'resubmit rw-effects)
            (push f rw-requeued))
          ;; The virtual allocator is full. Keep the actual queue so the
          ;; subsequent dispatch or shutdown sees the same pending job.
          (aset f 1 nil)
          (push f romacs--isolate-waiting))))

(defun rw-watch ()
  (setq rw-apply (symbol-function 'romacs--isolate-effect))
  (fset 'romacs--isolate-effect
        (lambda (i event effect out)
          (push (car effect) rw-effects)
          (funcall rw-apply i event effect out)))
  (setq isolate-error-functions
        (list (lambda (&rest _args) (push 'callback rw-effects)))))

(defun rw-add (id role pending parent)
  (let ((i (romacs--isolate-make id nil nil nil)))
    (aset i 10 role)
    (aset i 11 parent)
    (push i romacs--isolates)
    (when pending
      (let ((f (romacs--isolate-future i id nil)))
        (aset i 5 (list (cons id f)))
        (aset i 6 t)
        (push f rw-futures)
        (isolate-on-done f (lambda (done)
                             (push 'callback rw-effects)
                             (push (cons done (aref i 3)) rw-callbacks)))))
    i))

(defun rw-world (variant)
  (rw-reset)
  (let* ((z (rw-add 1 'zygote (= (% variant 3) 0) nil))
         (record (vector 'zygote z nil 'ready nil))
         (stopped (rw-add 8 'zygote nil nil)))
    (setq rw-main-zygote z)
    (romacs--isolate-step stopped '(retire))
    (puthash nil record romacs--isolate-zygotes)
    (vector z (rw-add 2 'job t record) (rw-add 3 'job t record)
            (rw-add 4 'owner t record) (rw-add 5 'pool t nil)
            (rw-add 6 'fresh t nil) (rw-add 7 nil (= (% variant 2) 0) nil) stopped)))

(defun rw-event (i op)
  (let* ((facts (romacs--isolate-facts i))
         (role (plist-get facts :role))
         (phase (plist-get facts :phase))
         (pending (plist-get facts :pending)))
    (cond
     ((eq op 'shutdown) '(shutdown))
     ((not (memq i romacs--isolates)) nil)
     ((eq op 'death) '(death isolate-died nil))
     ((eq op 'hello-refused)
      (and (eq phase 'live) (plist-get facts :forked)
           '(hello-refused (build-mismatch "different build"))))
     ((eq op 'world-refused)
      (and (eq phase 'live) (plist-get facts :forked)
           '(world-refused "cannot adopt world" ("world" . "key"))))
     ((eq op 'memory) (and (eq role 'job) pending '(memory (isolate-memory-exhausted nil nil nil))))
     ((eq op 'answered) (and pending (eq phase 'live) (not (eq role 'zygote))
                              (plist-get facts :forked) '(answered)))
     ((eq op 'orphaned) (and pending (eq role 'job) (plist-get facts :forked) '(orphaned)))
     ((eq op 'refused) (and pending (eq role 'owner) '(refused (owner-unprepared nil nil nil))))
     ((eq op 'refuse) (and (eq role 'zygote) (eq phase 'live) '(refuse missing t)))
     ((eq op 'child-gone) (and (eq role 'zygote) '(child-gone)))
     (t (list op)))))

(defun rw-action (i event)
  (let ((op (car event)))
    (unless (memq op rw-covered) (push op rw-covered))
    (cond
     ((eq op 'shutdown) (isolate-kill-all))
     ((eq op 'answered)
      (let ((id (romacs--isolate-id i)) (f (cdr (car (aref i 5)))))
        (setq rw-next-frame (list id 'result id '(42 nil)))
        (romacs--isolate-dispatch)
        (when (and (memq (aref i 10) '(pool fresh))
                   (eq (cdr (assq f rw-callbacks)) 'live))
          (push 'spent-worker-live-in-callback rw-issues))))
     (t
      (when (eq op 'death) (rw-exit (romacs--isolate-id i)))
      (romacs--isolate-step i event)))))

(defun rw-observe-futures ()
  (dolist (f rw-futures)
    (let ((seen 0) (owner (isolate-future-isolate f)))
      (dolist (item rw-callbacks) (when (eq (car item) f) (setq seen (1+ seen))))
      (if (isolate-future-done-p f)
          (unless (= seen 1) (push 'future-settlement rw-issues))
        (unless (and (= seen 0)
                     (or (memq f romacs--isolate-waiting)
                         (and (memq owner romacs--isolates) (rassq f (aref owner 5)))))
          (push 'pending-future-lost rw-issues))))))

(defun rw-observe-slots (workers)
  (let ((expected 0) (observed 0))
    (dotimes (at (length workers))
      (let* ((i (aref workers at)) (id (romacs--isolate-id i)) (closed 0))
        (dolist (entry rw-closed) (when (eq entry id) (setq closed (1+ closed))))
        (when (or (> closed 1) (and (= closed 1) (not (memq id rw-exited))))
          (push 'child-close-without-one-death rw-issues))
        (when (and (= closed 0) (not (eq (aref i 10) 'owner)))
          (setq expected (1+ expected)))))
    (dolist (i romacs--isolates)
      (when (romacs--isolate-counted-p i) (setq observed (1+ observed))))
    (unless (= observed expected) (push 'unbalanced-pool-slots rw-issues))))

(defun rw-observe-order ()
  (let ((notified nil))
    (dolist (effect (reverse rw-effects))
      (cond ((eq effect 'callback) (setq notified t))
            (notified
             (push 'effect-after-callback rw-issues))))))

(defun rw-shutdown-issues (workers)
  (when (memq 'resubmit rw-effects) (push 'shutdown-resubmitted rw-issues))
  (dotimes (at (length workers))
    (let ((id (romacs--isolate-id (aref workers at))))
      ;; 3 is a fork request without a reported process. A phase change
      ;; or closing a handle supplies no evidence of a child's exit.
      (when (and (/= id 3) (not (memq id rw-exited)))
        (push (list 'shutdown-left-process id) rw-issues))))
  (dolist (f rw-futures)
    (unless (isolate-future-done-p f) (push 'shutdown-left-pending-future rw-issues))))

(defun rw-episode (variant actions)
  (let ((workers (rw-world variant)) (trace nil) (findings nil) (at 0))
    (dolist (action actions)
      (let* ((i (aref workers (car action))) (event (rw-event i (nth 1 action))))
        (when event
          (setq rw-issues nil rw-effects nil)
          (push (list (car action) event) trace)
          (rw-action i event)
          (rw-observe-futures)
          (rw-observe-slots workers)
          (rw-observe-order)
          (when (eq (car event) 'shutdown) (rw-shutdown-issues workers))
          (dolist (issue rw-issues)
            (unless (assoc issue findings)
              (push (list issue at (reverse trace)) findings)))))
      (setq at (1+ at)))
    (dolist (i (isolate-list))
      (when (memq (romacs--isolate-id i) rw-exited)
        (rw-action i '(death isolate-killed nil))))
    (setq rw-issues nil)
    (rw-observe-futures)
    (rw-observe-slots workers)
    (dolist (i (isolate-list))
      (when (memq (aref i 10) '(job owner zygote))
        (push 'child-left-without-death rw-issues)))
    (dolist (issue rw-issues)
      (unless (assoc issue findings) (push (list issue at (reverse trace)) findings)))
    (rw-reset)
    (reverse findings)))

(setq rw-covered nil)
(rw-native)
(rw-watch)
