;;; romacs-agent-reattach-tests.el --- A session whose attachment ended while its supervisor runs -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude))

(defmacro romacs-agent-reattach-tests--with (&rest body)
  "BODY with `session', detached, its supervisor's socket a file under
`root'; `scheduled' collects each reattach's delay, `attached' each
attach."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-reattach-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "aaaaaaaa-0000-4000-8000-000000000000" :harness 'claude :title "lane"
                                       :state 'detached :dir (expand-file-name "s" root)
                                       :host-socket (expand-file-name "host.sock" root)))
          (scheduled nil) (attached nil))
     (make-directory (romacs-agent-dir session))
     (write-region "" nil (romacs-agent-host-socket session))
     (puthash (romacs-agent-id session) session romacs-agent--sessions)
     (cl-letf (((symbol-function 'run-at-time)
                (lambda (delay _repeat function &rest args)
                  (when (eq function #'romacs-agent--reattach) (push delay scheduled))))
               ((symbol-function 'romacs-agent--attach) (lambda (s &rest _) (push s attached) nil)))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

(ert-deftest romacs-agent-reattach-when-the-attachment-ends ()
  (romacs-agent-reattach-tests--with
    (let ((process (make-process :name "romacs-reattach-fixture" :command '("cat") :noquery t)))
      (process-put process 'romacs-agent session)
      (setf (romacs-agent-state session) 'idle (romacs-agent-connection session) process)
      (delete-process process)
      (romacs-agent--sentinel process "deleted")
      (should (eq (romacs-agent-state session) 'detached))
      (should (equal scheduled '(1)))
      ;; Its time come: attached again.
      (romacs-agent--reattach session)
      (should (equal attached (list session))))))

(ert-deftest romacs-agent-reattach-is-no-longer-detached ()
  (romacs-agent-reattach-tests--with
    (let* ((process (make-process :name "romacs-reattach-fixture" :command '("cat") :noquery t))
           (caught-up nil)
           (romacs-agent-caught-up-functions
            (list (lambda (s)
                    (when (and (eq s session) (romacs-agent-caught-up s))
                      (push (romacs-agent-state s) caught-up))))))
      (unwind-protect
          (progn
            (process-put process 'romacs-agent session)
            (setf (romacs-agent-connection session) process)
            (puthash session 3 romacs-agent--reattaching)
            (cl-letf (((symbol-function 'romacs-agent--kept) #'ignore)
                      ((symbol-function 'romacs-agent--allow-tool-calls) #'ignore)
                      ((symbol-function 'romacs-agent--step) #'ignore))
              (romacs-agent--filter process "{\"attached\":2}\n")
              (should (eq (romacs-agent-state session) 'idle))
              (should (= (gethash session romacs-agent--reattaching 0) 0))
              ;; Its last prompt's turn not done: running.
              (setf (romacs-agent-state session) 'detached (romacs-agent-input-seq session) 9)
              (romacs-agent--filter process "{\"attached\":3}\n")
              (should (eq (romacs-agent-state session) 'running))
              ;; A question it asked still waits for the user.
              (puthash "q1" t (romacs-agent-approvals session))
              (setf (romacs-agent-state session) 'detached)
              (let ((events nil))
                (let ((romacs-agent-event-functions (list (lambda (_s e) (push (plist-get e :text) events)))))
                  (romacs-agent--filter process "{\"attached\":4}\n"))
                (should (eq (romacs-agent-state session) 'waiting-approval))
                ;; Its views and its parent's rows are told.
                (should (member "attached" events))
                (should (equal caught-up '(waiting-approval running idle))))))
        (delete-process process)))))

(ert-deftest romacs-agent-reattach-backs-off-and-stops ()
  (romacs-agent-reattach-tests--with
    (dotimes (_ (1+ (length romacs-agent--reattach-delays)))
      (romacs-agent--reattach-later session))
    (should (equal (reverse scheduled) romacs-agent--reattach-delays))
    ;; Cancelled once its look was scheduled: no attach when it comes.
    (setf (romacs-agent-cancelled session) t)
    (romacs-agent--reattach session)
    (should-not attached)
    (setf (romacs-agent-cancelled session) nil)
    ;; Nothing to look for: no socket, its journal ended, or cancelled.
    (setq scheduled nil)
    (remhash session romacs-agent--reattaching)
    (setf (romacs-agent-state session) 'ended)
    (romacs-agent--reattach-later session)
    (setf (romacs-agent-state session) 'detached (romacs-agent-cancelled session) t)
    (romacs-agent--reattach-later session)
    (setf (romacs-agent-cancelled session) nil)
    (delete-file (romacs-agent-host-socket session))
    (romacs-agent--reattach-later session)
    (should-not scheduled)))

(ert-deftest romacs-agent-reattach-a-record-of-an-earlier-definition ()
  ;; A record an earlier definition made (a live load since), shorter than
  ;; this one's: looking for its supervisor again reads nothing past it.
  (romacs-agent-reattach-tests--with
    (let* ((fields (cl-subseq (vconcat (cl-loop for i below (length session) collect (aref session i))) 0
                              (- (length session) 2)))
           (short (apply #'record (append fields nil))))
      (romacs-agent--reattach-later short)
      (should (= (gethash short romacs-agent--reattaching 0) 1))
      (should (equal scheduled (list (car romacs-agent--reattach-delays)))))))

(ert-deftest romacs-agent-reattach-on-restore ()
  (romacs-agent-reattach-tests--with
    (let ((process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment)))
      (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore)
                ((symbol-function 'romacs-agent--sweep) #'ignore))
        (puthash session 5 romacs-agent--reattaching)
        (romacs-agent-restore)
        (should (equal attached (list session)))
        (should (= (gethash session romacs-agent--reattaching 0) 0))))))

(ert-deftest romacs-agent-reattach-never-a-second-attachment ()
  (let* ((root (make-temp-file "romacs-attach-" t))
         (session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :dir root :host-socket (expand-file-name "host.sock" root)))
         (first (make-process :name "romacs-attach-first" :command '("cat") :noquery t))
         (connected 0))
    (unwind-protect
        (cl-letf (((symbol-function 'make-network-process)
                   (lambda (&rest _) (cl-incf connected)
                     (make-process :name "romacs-attach-second" :command '("cat") :noquery t))))
          (setf (romacs-agent-connection session) first)
          ;; Attached already: the same attachment, no second one to orphan it.
          (should (eq (romacs-agent--attach session 0) first))
          (should (= connected 0))
          ;; Taken over: the first is let go.
          (let ((second (romacs-agent--attach session 0 t)))
            (should (= connected 1))
            (should (eq (romacs-agent-connection session) second))
            (should-not (process-live-p first))
            (delete-process second)))
      (when (process-live-p first) (delete-process first))
      (delete-directory root t))))

(ert-deftest romacs-agent-reattach-a-relay-brings-its-session-back ()
  (romacs-agent-reattach-tests--with
    (let ((peer (make-process :name "romacs-relay-fixture" :command '("cat") :noquery t))
          (romacs-mcp--peers nil))
      (unwind-protect
          (progn
            (push peer romacs-mcp--peers)
            (funcall (gethash "romacs/hello" romacs-mcp-methods) peer
                     (romacs-mcp--object "session" (romacs-agent-id session) "relay" "r1"))
            ;; The session's own peer, and its session attached for its next try.
            (should (process-get peer 'romacs-mcp-owned))
            (should (equal attached (list session))))
        (delete-process peer)))))

(ert-deftest romacs-agent-reattach-a-relay-with-no-supervisor-stays ()
  (romacs-agent-reattach-tests--with
    (let ((peer (make-process :name "romacs-relay-fixture" :command '("cat") :noquery t))
          (romacs-mcp--peers nil) (closing 0))
      (unwind-protect
          (cl-letf (((symbol-function 'run-at-time) (lambda (&rest _) (cl-incf closing))))
            (push peer romacs-mcp--peers)
            ;; No supervisor listens: nothing to attach, and no other editor
            ;; can; the relay is kept, not sent to come back for ever.
            (delete-file (romacs-agent-host-socket session))
            (funcall (gethash "romacs/hello" romacs-mcp-methods) peer
                     (romacs-mcp--object "session" (romacs-agent-id session) "relay" "r1"))
            (should (= closing 0))
            (should-not attached)
            ;; One listens (attach returns its connection): turned away until attached here.
            (write-region "" nil (romacs-agent-host-socket session))
            (cl-letf (((symbol-function 'romacs-agent--attach) (lambda (s &rest _) (push s attached) peer)))
              (funcall (gethash "romacs/hello" romacs-mcp-methods) peer
                       (romacs-mcp--object "session" (romacs-agent-id session) "relay" "r2")))
            (should (= closing 1)))
        (delete-process peer)))))

(ert-deftest romacs-agent-reattach-relays-do-not-fill-the-endpoint ()
  (let* ((romacs-mcp--listener 'listener) (romacs-mcp--peers nil) (closed nil) (fixtures nil))
    (cl-letf (((symbol-function 'romacs-mcp--close) (lambda (p) (push p closed))))
      (unwind-protect
          (progn
            (dotimes (_ romacs-mcp--peer-limit)
              (let ((p (make-process :name "romacs-owned" :command '("cat") :noquery t)))
                (push p fixtures) (romacs-mcp-own-peer p) (push p romacs-mcp--peers)))
            ;; As many relays as the limit: an anonymous client still gets in.
            (let ((client (make-process :name "romacs-client" :command '("cat") :noquery t)))
              (push client fixtures)
              (romacs-mcp--accept 'listener client nil)
              (should (memq client romacs-mcp--peers))
              (should-not closed))
            ;; Anonymous peers alone are bounded.
            (dotimes (_ (1- romacs-mcp--peer-limit))
              (let ((p (make-process :name "romacs-anon" :command '("cat") :noquery t)))
                (push p fixtures) (push p romacs-mcp--peers)))
            (let ((one-more (make-process :name "romacs-anon" :command '("cat") :noquery t)))
              (push one-more fixtures)
              (romacs-mcp--accept 'listener one-more nil)
              (should (equal closed (list one-more)))))
        (mapc #'delete-process fixtures)))))

(provide 'romacs-agent-reattach-tests)
;; 2026-11-21: a restart's replays, hundreds of megabytes, were handled
;; in one go each, inside whatever read their output: 81 s without a key.
(ert-deftest romacs-agent-reattach-a-replay-is-handled-a-turn-at-a-time ()
  ;; A supervisor's records are handled `romacs-agent--drain-turn' seconds
  ;; at a time, the rest from a timer, in the order sent, chunks come in
  ;; between after them, joined a line's chunks at a time; the
  ;; attachment's end waits for them all.
  (let* ((clock 0.0) (handled nil) (due nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000000" :harness 'claude
                                      :state 'running :dir "/nonexistent/b" :host-socket "/nonexistent/b.sock"))
         (process (make-pipe-process :name "romacs-drain-fixture" :noquery t))
         (lines (lambda (from to) (mapconcat (lambda (seq) (format "{\"seq\":%d}\n" seq)) (number-sequence from to) ""))))
    (process-put process 'romacs-agent session)
    (setf (romacs-agent-connection session) process)
    (unwind-protect
        (cl-letf (((symbol-function 'float-time) (lambda (&optional _) clock))
                  ((symbol-function 'run-at-time)
                   (lambda (_time _repeat function &rest args) (setq due (append due (list (cons function args)))) 'timer))
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_session record)
                     ;; Each record costs a hundredth of a second.
                     (setq clock (+ clock 0.01))
                     (push (cons (gethash "seq" record) (romacs-agent-state session)) handled)))
                  ((symbol-function 'romacs-agent--raise) #'ignore)
                  ((symbol-function 'romacs-agent--drop-relays) #'ignore))
          (romacs-agent--filter process (funcall lines 1 10))
          (should (= (length handled) 3))
          (should (= (length due) 1))
          ;; Its turn's slice taken: what comes waits for the timer.
          (romacs-agent--filter process (funcall lines 11 12))
          (should (= (length handled) 3))
          (should (= (length due) 1))
          (delete-process process)
          (romacs-agent--sentinel process "deleted\n")
          (should (eq (romacs-agent-connection session) process))
          (while due (let ((timer (pop due))) (apply (car timer) (cdr timer))))
          (should (equal (mapcar #'car (reverse handled)) (number-sequence 1 12)))
          (should-not (memq 'detached (mapcar #'cdr handled)))
          (should-not (romacs-agent-connection session))
          (should (eq (romacs-agent-state session) 'detached)))
      (delete-process process))))

;; 2026-11-21: a 16 MB record's line end was found in 57 ms, past the
;; turn's 25 ms: each slice ended before its first line, and the drain set
;; itself again, for ever.
(ert-deftest romacs-agent-reattach-a-slice-handles-a-line-however-long-its-search ()
  ;; Finding a line costs more than a turn's slice; the slice still handles
  ;; one, so the replay goes on.
  (let* ((clock 0.0) (handled nil) (due nil) (turns 0)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "cccccccc-0000-4000-8000-000000000000" :harness 'claude
                                      :state 'running :dir "/nonexistent/c" :host-socket "/nonexistent/c.sock"))
         (process (make-pipe-process :name "romacs-drain-slow-search" :noquery t))
         (search (lambda (found) (setq clock (+ clock (* 2 romacs-agent--drain-turn))) found)))
    (process-put process 'romacs-agent session)
    (process-put process 'romacs-agent-kept t)
    (setf (romacs-agent-connection session) process)
    (unwind-protect
        (cl-letf (((symbol-function 'float-time) (lambda (&optional _) clock))
                  ((symbol-function 'run-at-time)
                   (lambda (_time _repeat function &rest args) (setq due (append due (list (cons function args)))) 'timer))
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_session record) (push (gethash "seq" record) handled)))
                  ((symbol-function 'romacs-agent--raise) #'ignore))
          (advice-add 'romacs-agent--line-waits-p :filter-return search)
          (romacs-agent--filter process "{\"seq\":1}\n{\"seq\":2}\n")
          (while (and due (< (cl-incf turns) 10))
            (let ((timer (pop due))) (apply (car timer) (cdr timer))))
          (should (equal (reverse handled) '(1 2))))
      (advice-remove 'romacs-agent--line-waits-p search)
      (delete-process process))))

;; A line in many chunks is joined once its end has come, each byte copied
;; once: joining it a megabyte at a time copied the line so far again each
;; time, quadratic in a long record.
(ert-deftest romacs-agent-reattach-a-long-line-is-joined-once ()
  (let* ((handled nil) (joins 0) (drains 0)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "eeeeeeee-0000-4000-8000-000000000000" :harness 'claude
                                      :state 'running :dir "/nonexistent/e" :host-socket "/nonexistent/e.sock"))
         (process (make-pipe-process :name "romacs-drain-long-line" :noquery t))
         (line (concat "{\"seq\":1,\"text\":\"" (make-string (* 3 1048576) ?x) "\"}\n"))
         (count (lambda (&rest _) (cl-incf joins)))
         (drain (lambda (&rest _) (cl-incf drains))))
    (process-put process 'romacs-agent session)
    (process-put process 'romacs-agent-kept t)
    (setf (romacs-agent-connection session) process)
    (unwind-protect
        (cl-letf (((symbol-function 'float-time) (lambda (&optional _) 0.0))
                  ((symbol-function 'run-at-time) (lambda (&rest _) 'timer))
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_session record) (push (gethash "seq" record) handled)))
                  ((symbol-function 'romacs-agent--raise) #'ignore))
          (advice-add 'romacs-agent--join :before count)
          (advice-add 'romacs-agent--drain :before drain)
          (let ((start 0))
            (while (< start (length line))
              (romacs-agent--filter process (substring line start (min (length line) (+ start 65536))))
              (setq start (+ start 65536))))
          (should (equal handled '(1)))
          (should (= joins 1))
          ;; Chunks with no line's end start no drain.
          (should (= drains 1)))
      (advice-remove 'romacs-agent--join count)
      (advice-remove 'romacs-agent--drain drain)
      (delete-process process))))

(ert-deftest romacs-agent-reattach-chunks-from-before-the-count-are-counted ()
  ;; An attachment whose chunks came under code from before the count
  ;; (loaded into a running editor) has its waiting line ends counted once.
  (let* ((handled nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "ffffffff-0000-4000-8000-000000000000" :harness 'claude
                                      :state 'running :dir "/nonexistent/f" :host-socket "/nonexistent/f.sock"))
         (process (make-pipe-process :name "romacs-drain-legacy" :noquery t)))
    (process-put process 'romacs-agent session)
    (process-put process 'romacs-agent-kept t)
    (setf (romacs-agent-connection session) process)
    (process-put process 'pending (list "{\"seq\":2}\n" "{\"seq\":1}\n"))
    (process-put process 'ready t)
    (unwind-protect
        (cl-letf (((symbol-function 'float-time) (lambda (&optional _) 0.0))
                  ((symbol-function 'run-at-time) (lambda (&rest _) 'timer))
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_session record) (push (gethash "seq" record) handled)))
                  ((symbol-function 'romacs-agent--raise) #'ignore))
          (romacs-agent--drain process)
          (should (equal (reverse handled) '(1 2))))
      (delete-process process))))

(ert-deftest romacs-agent-reattach-a-quit-in-a-record-leaves-the-rest-going ()
  ;; A handler that exits nonlocally (a quit) ends its slice: the records
  ;; after it go on from the timer, in order.
  (let* ((handled nil) (due nil) (quit t)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "cccccccc-0000-4000-8000-000000000000" :harness 'claude
                                      :state 'running :dir "/nonexistent/c" :host-socket "/nonexistent/c.sock"))
         (process (make-pipe-process :name "romacs-drain-quit" :noquery t)))
    (process-put process 'romacs-agent session)
    (setf (romacs-agent-connection session) process)
    (unwind-protect
        (cl-letf (((symbol-function 'run-at-time)
                   (lambda (_time _repeat function &rest args) (setq due (append due (list (cons function args)))) 'timer))
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_session record)
                     (push (gethash "seq" record) handled)
                     (when (and quit (= (gethash "seq" record) 2)) (setq quit nil) (throw 'quit nil)))))
          (catch 'quit (romacs-agent--filter process "{\"seq\":1}\n{\"seq\":2}\n{\"seq\":3}\n"))
          (should (equal handled '(2 1)))
          (should (= (length due) 1))
          (while due (let ((timer (pop due))) (apply (car timer) (cdr timer))))
          (should (equal (reverse handled) '(1 2 3))))
      (delete-process process))))

;;; romacs-agent-reattach-tests.el ends here
