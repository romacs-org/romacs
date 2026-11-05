;;; jabber-test-mam-sync.el --- Tests for the background archive catch-up  -*- lexical-binding: t; -*-

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The worker's side runs here in the test's own VM, against the virtual
;; server (jabberd.el): the job function is ordinary Lisp; only where it
;; runs differs.  The session's side runs with the isolate functions
;; replaced, so no worker starts.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jabber)
(require 'jabberd)
(require 'jabber-mam-sync)

;;; The send gate

(ert-deftest jabber-test-mam-sync-gate ()
  "The sync session sends negotiation, disco#info and archive queries only."
  (dolist (sexp `((auth ((xmlns . "urn:ietf:params:xml:ns:xmpp-sasl")
                         (mechanism . "SCRAM-SHA-1"))
                        "biws")
                  (response ((xmlns . "urn:ietf:params:xml:ns:xmpp-sasl")) "x")
                  (starttls ((xmlns . "urn:ietf:params:xml:ns:xmpp-tls")))
                  (iq ((type . "set")) (bind ((xmlns . "urn:ietf:params:xml:ns:xmpp-bind"))))
                  (iq ((type . "set")) (session ((xmlns . "urn:ietf:params:xml:ns:xmpp-session"))))
                  (iq ((type . "set") (to . "room@muc.example"))
                      (query ((xmlns . ,jabber-mam-xmlns) (queryid . "q"))))
                  (iq ((type . "get") (to . "room@muc.example"))
                      (query ((xmlns . "http://jabber.org/protocol/disco#info"))))
                  (iq ((type . "result") (id . "ping-1")))))
    (should (jabber-mam-sync--allowed-p sexp)))
  (dolist (sexp `((presence ())
                  (presence ((to . "room@muc.example/me"))
                            (x ((xmlns . "http://jabber.org/protocol/muc"))))
                  (message ((to . "juliet@capulet.example")) (body () "hi"))
                  (iq ((type . "get")) (query ((xmlns . "jabber:iq:roster"))))
                  (iq ((type . "set")) (enable ((xmlns . "urn:xmpp:carbons:2"))))
                  (iq ((type . "set")) (prefs ((xmlns . ,jabber-mam-xmlns) (default . "always"))))
                  (iq ((type . "set")) (pubsub ((xmlns . "http://jabber.org/protocol/pubsub"))))
                  (iq ((type . "set")) (query ((xmlns . "jabber:iq:private"))))
                  (iq ((type . "set"))
                      (query ((xmlns . ,jabber-mam-xmlns)))
                      (pubsub ((xmlns . "http://jabber.org/protocol/pubsub"))))
                  (enable ((xmlns . "urn:xmpp:sm:3")))
                  (auth ((xmlns . "jabber:iq:auth")))))
    (should-not (jabber-mam-sync--allowed-p sexp)))
  (dolist (string '("<?xml version='1.0'?><stream:stream to='montague.net' version='1.0' xmlns='jabber:client' xmlns:stream='http://etherx.jabber.org/streams' version='1.0'>\n"
                    "</stream:stream>" " " "\n"))
    (should (jabber-mam-sync--allowed-string-p string)))
  (dolist (string '("<enable xmlns='urn:xmpp:sm:3' resume='true'/>"
                    "<r xmlns='urn:xmpp:sm:3'/>"
                    "<presence/>"
                    "<stream:stream to='x'><presence/>"))
    (should-not (jabber-mam-sync--allowed-string-p string)))
  ;; A refusal is a failed send, never a silent success.
  (let ((jabber-mam-sync--refused nil))
    (should-error (jabber-mam-sync--gate nil '(presence ())) :type 'jabber-mam-sync-refused)
    (should-error (jabber-mam-sync--gate-string nil "<presence/>") :type 'jabber-mam-sync-refused)
    (should (equal '("<presence/>" presence) jabber-mam-sync--refused))))

;;; What the user sees

(ert-deftest jabber-test-mam-sync-toast-text ()
  "One toast: messages and chats, failures by archive, quiet follow-ups."
  (let ((done '(:archives ((nil "chat" success 1 nil) ("room@muc" "groupchat" success 2 nil))
                :peers (("me@x" "a@x" "chat" 1) ("me@x" "room@muc" "groupchat" 2)
                        ("me@x" "b@x" "chat" 0))))
        (nothing '(:archives ((nil "chat" success 0 nil)) :peers nil))
        (partial '(:archives ((nil "chat" success 1 nil) ("room@muc" "groupchat" failed 0 "forbidden"))
                   :peers (("me@x" "a@x" "chat" 1))))
        (failed '(:archives (("room@muc" "groupchat" failed 0 nil) (nil "chat" failed 0 nil))
                  :peers nil))
        (no-archive '(:archives (("bridge@muc" "groupchat" unsupported 0 nil) (nil "chat" success 2 nil))
                      :peers (("me@x" "a@x" "chat" 2 2)))))
    ;; A room without an archive is nothing to read, not a failure.
    (should (equal "Jabber: sync completed (2 messages in 1 chat)"
                   (jabber-mam-sync--toast-text no-archive nil)))
    (should (equal "Jabber: sync completed (3 messages in 2 chats)"
                   (jabber-mam-sync--toast-text done nil)))
    (should (equal "Jabber: sync completed (0 messages in 0 chats)"
                   (jabber-mam-sync--toast-text nothing t)))
    (should-not (jabber-mam-sync--toast-text nothing nil))
    (should (equal "Jabber: sync completed (1 message in 1 chat); failed for room@muc"
                   (jabber-mam-sync--toast-text partial nil)))
    (should (equal "Jabber: sync failed for room@muc, personal archive"
                   (jabber-mam-sync--toast-text failed nil)))
    (should (equal "Jabber: sync completed (1 message in 1 chat); personal archive continues next sync"
                   (jabber-mam-sync--toast-text
                    '(:archives ((nil "chat" incomplete 1 "no progress for 120s")
                                 ("room@muc" "groupchat" success 0 nil))
                      :peers (("me@x" "a@x" "chat" 1 1)))
                    nil)))
    (should (equal "Jabber: sync failed: the sync session did not connect"
                   (jabber-mam-sync--toast-text
                    '(:error "the sync session did not connect" :archives nil) t)))
    ;; A failed archive draws the toast as an error; an archive without
    ;; MAM, or one that continues next sync, does not.
    (should (eq 'error (jabber-mam-sync--toast-kind partial)))
    (should (eq 'error (jabber-mam-sync--toast-kind failed)))
    (should (eq 'error (jabber-mam-sync--toast-kind '(:error "cancelled" :archives nil))))
    (should-not (jabber-mam-sync--toast-kind done))
    (should-not (jabber-mam-sync--toast-kind no-archive))))

(defun jabber-test-mam-sync--connection (&optional resource)
  "A connection whose state names romeo@montague.net/RESOURCE."
  (let ((jc (make-symbol "jc")))
    (put jc :state :session-established)
    (put jc :state-data (list :username "romeo" :server "montague.net"
                              :resource (or resource "Orchard") :password "pw"
                              :connection-type 'network))
    jc))

(defmacro jabber-test-mam-sync--with-session (&rest body)
  "Run BODY with a live connection `jc', no timers, workers or toasts.
`spawned' collects the spawn arguments, `toasts' the toasts, `refreshed'
the merged peers; `done' calls the job's completion callback."
  (declare (indent 0) (debug t))
  `(let* ((jc (jabber-test-mam-sync--connection))
          (jabber-connections (list jc))
          (jabber-mam-sync--accounts (make-hash-table :test #'equal))
          (jabber-mam-sync--due nil)
          (jabber-mam-sync--due-tail nil)
          (jabber-mam-sync--timer nil)
          (jabber-mam-sync--live-jobs 0)
          (jabber-mam-sync--merge-tail nil)
          (jabber-mam-sync--merge-queued (make-hash-table :test #'eq))
          (jabber-mam-sync--merge-queue nil)
          (jabber-mam-sync--merge-timer nil)
          (jabber-mam-sync-complete-functions nil)
          (jabber-mam-sync-finished-functions nil)
          (jabber-mam-background t)
          spawned toasts refreshed done summary)
     (cl-letf (((symbol-function 'jabber-mam-sync-available-p) #'always)
               ((symbol-function 'run-with-timer) (lambda (&rest args) (vector 'timer args)))
               ((symbol-function 'cancel-timer) #'ignore)
               ((symbol-function 'isolate-spawn)
                (lambda (&rest args) (push args spawned) 'future))
               ((symbol-function 'isolate-on-done)
                (lambda (_future callback) (setq done (lambda () (funcall callback 'future)))))
               ((symbol-function 'isolate-await) (lambda (&rest _) summary))
               ((symbol-function 'isolate-future-isolate) (lambda (_) 'worker))
               ((symbol-function 'isolate-live-p) #'ignore)
               ((symbol-function 'thaw) #'identity)
               ((symbol-function 'jabber-muc-nickname)
                (lambda (room _jc) (and (equal room "room@muc.example") "romeo")))
               ((symbol-function 'jabber-mam-sync--toast) (lambda (text &optional _kind) (push text toasts)))
               ((symbol-function 'jabber-mam-sync--refresh)
                (lambda (buffer) (setq refreshed (append refreshed (list buffer))))))
       (unwind-protect (progn ,@body)
         (jabber-db-share-writes nil)))))

(defconst jabber-test-mam-sync--account "romeo@montague.net")

(ert-deftest jabber-test-mam-sync-requests-make-one-job ()
  "A connect, its rooms and a chat make one job on a resource of its own."
  (jabber-test-mam-sync--with-session
    (jabber-mam-maybe-catchup jc)
    (jabber-mam-muc-joined jc "room@muc.example")
    (jabber-mam-muc-joined jc "room@muc.example")
    (jabber-mam-chat-opened jc "juliet@capulet.example")
    (should-not spawned)
    (jabber-mam-sync--start jabber-test-mam-sync--account)
    (should (= 1 (length spawned)))
    (pcase-let ((`(,function ,account ,archives . ,keys) (car spawned)))
      (should (eq function 'jabber-mam-sync-worker-run))
      (should (string-prefix-p "Orchard-sync-" (plist-get account :resource)))
      (should (equal "pw" (plist-get account :password)))
      (should (equal '((:room nil :with nil :nick nil)
                       (:room "room@muc.example" :with nil :nick "romeo")
                       (:room nil :with "juliet@capulet.example" :nick nil))
                     archives))
      (should (equal '(jabber) (plist-get keys :require)))
      (should (plist-get keys :fresh))
      (should (assq 'jabber-db-path (plist-get keys :bindings))))
    ;; Requests while the job runs wait for it.
    (jabber-mam-muc-joined jc "other@muc.example")
    (should (= 1 (length spawned)))))

(ert-deftest jabber-test-mam-sync-completion-toasts-and-merges ()
  "The job's end: one toast, the hook, peers queued, a follow-up job."
  (jabber-test-mam-sync--with-session
    (let (finished)
      (add-hook 'jabber-mam-sync-finished-functions
                (lambda (account s) (push (list account s) finished)))
      (jabber-mam-maybe-catchup jc)
      (jabber-mam-sync--start jabber-test-mam-sync--account)
      (jabber-mam-muc-joined jc "room@muc.example")
      (setq summary '(:archives ((nil "chat" success 2 nil))
                      :peers (("romeo@montague.net" "a@x" "chat" 2 2)
                              ("romeo@montague.net" "b@x" "chat" 0 1))))
      (funcall done)
      (should (equal '("Jabber: sync completed (2 messages in 1 chat)") toasts))
      (should (equal (list (list "romeo@montague.net" summary)) finished))
      ;; The room requested meanwhile is the follow-up job's.
      (should-not (jabber-mam-sync--get jabber-test-mam-sync--account :future))
      (jabber-mam-sync--start jabber-test-mam-sync--account)
      (should (= 2 (length spawned)))
      (should (equal '((:room "room@muc.example" :with nil :nick "romeo"))
                     (nth 2 (car spawned))))
      ;; A follow-up with nothing new says nothing.
      (setq summary '(:archives (("room@muc.example" "groupchat" success 0 nil)) :peers nil))
      (funcall done)
      (should (= 1 (length toasts))))))

(ert-deftest jabber-test-mam-sync-merge-waits-for-idle ()
  "A shown buffer whose history changed is refreshed once the user idles.
Peers changed without new rows (a correction, a reaction) count too;
a hidden buffer is not looked for."
  (jabber-test-mam-sync--with-session
    (let ((idle nil) (pending nil) (jabber-mam-sync--changed (make-hash-table :test #'equal))
          (a (generate-new-buffer " a")) (r (generate-new-buffer " r")))
      (unwind-protect
          (cl-letf (((symbol-function 'current-idle-time) (lambda () idle))
                    ((symbol-function 'input-pending-p) (lambda (&rest _) pending))
                    ((symbol-function 'jabber-chat-buffer--history-key)
                     (lambda (buffer)
                       (cdr (assq buffer (list (cons a '("me@x" "a@x" "chat" nil))
                                               (cons r '("me@x" "r@m" "groupchat" nil))))))))
            (save-window-excursion
              (delete-other-windows)
              (switch-to-buffer a)
              (jabber-mam-sync--merge '(:peers (("me@x" "a@x" "chat" 1 1)
                                                ("me@x" "r@m" "groupchat" 0 2))))
              (jabber-mam-sync--merge '(:peers (("me@x" "a@x" "chat" 1 1))))
              (should (equal (list a) jabber-mam-sync--merge-queue))
              (should (jabber-mam-sync--stale-p r))
              (jabber-mam-sync--merge-next)
              (should-not refreshed)
              (setq idle (seconds-to-time 2) pending t)
              (jabber-mam-sync--merge-next)
              (should-not refreshed)
              (setq pending nil)
              (jabber-mam-sync--merge-next)
              (should (equal (list a) refreshed))
              (should-not jabber-mam-sync--merge-queue)))
        (mapc #'kill-buffer (list a r))))))

(ert-deftest jabber-test-mam-sync-hidden-buffer-refreshes-when-shown ()
  "A hidden buffer whose history changed is refreshed once, when shown."
  (let ((buffer (generate-new-buffer " hidden-chat"))
        (jabber-mam-sync--changed (make-hash-table :test #'equal))
        refreshed)
    (unwind-protect
        (cl-letf (((symbol-function 'jabber-chat-buffer--history-key)
                   (lambda (b) (and (eq b buffer) '("me@x" "a@x" "chat" nil))))
                  ((symbol-function 'jabber-chat-buffer-refresh)
                   (lambda ()
                     (setq jabber-chat--loaded-at (float-time))
                     (push (current-buffer) refreshed))))
          (with-current-buffer buffer (setq jabber-chat--loaded-at (- (float-time) 10)))
          (jabber-mam-sync--merge '(:peers (("me@x" "a@x" "chat" 3 3))))
          (should-not refreshed)
          (should-not jabber-mam-sync--merge-queue)
          (should (jabber-mam-sync--stale-p buffer))
          (save-window-excursion
            (switch-to-buffer buffer)
            (jabber-mam-sync--shown (selected-frame))
            (should (equal (list buffer) refreshed))
            (should-not (jabber-mam-sync--stale-p buffer))
            ;; Shown again: nothing more to do.
            (jabber-mam-sync--shown (selected-frame))
            (should (= 1 (length refreshed)))))
      (kill-buffer buffer))))

(ert-deftest jabber-test-mam-sync-shown-thread-refreshes-before-parent ()
  "A shown thread refreshes, marking itself read, before its shown parent.
And a parent that loaded before the thread was read is stale again."
  (let ((parent (generate-new-buffer " parent"))
        (thread (generate-new-buffer " thread"))
        (jabber-mam-sync--changed (make-hash-table :test #'equal))
        (jabber-mam-sync--merge-queue nil)
        (jabber-mam-sync--merge-tail nil)
        (jabber-mam-sync--merge-queued (make-hash-table :test #'eq))
        (jabber-mam-sync--merge-timer nil)
        order marked)
    (unwind-protect
        (cl-letf (((symbol-function 'jabber-chat-buffer--history-key)
                   (lambda (b) (cond ((eq b parent) '("me@x" "a@x" "chat" nil))
                                     ((eq b thread) '("me@x" "a@x" "chat" t)))))
                  ((symbol-function 'jabber-connection-bare-jid) (lambda (_) "me@x"))
                  ((symbol-function 'jabber-db-mark-message-thread-read)
                   (lambda (&rest args) (push args marked)))
                  ((symbol-function 'run-with-timer) (lambda (&rest _) 'timer))
                  ((symbol-function 'jabber-chat-buffer-refresh)
                   (lambda ()
                     (setq jabber-chat--loaded-at (float-time))
                     (push (current-buffer) order))))
          (with-current-buffer thread
            (setq-local jabber-message-thread-id "t1")
            (setq-local jabber-message-thread-peer "a@x")
            (setq-local jabber-message-thread-type "chat")
            (setq-local jabber-buffer-connection 'jc))
          (dolist (b (list parent thread))
            (with-current-buffer b (setq jabber-chat--loaded-at (- (float-time) 10))))
          (save-window-excursion
            (delete-other-windows)
            (switch-to-buffer parent)
            (set-window-buffer (split-window-below) thread)
            (jabber-mam-sync--merge '(:peers (("me@x" "a@x" "chat" 1 1))))
            (should (equal (list thread parent) jabber-mam-sync--merge-queue))
            (cl-flet ((next () (let ((b (pop jabber-mam-sync--merge-queue)))
                                 (remhash b jabber-mam-sync--merge-queued)
                                 (jabber-mam-sync--refresh b))))
              (next)
              (should marked)
              (next))
            (should (equal (list thread parent) (reverse order)))
            ;; A parent refreshed first is made stale by the thread's read.
            (with-current-buffer parent (setq jabber-chat--loaded-at (float-time)))
            (with-current-buffer thread (setq jabber-chat--loaded-at 0))
            (sleep-for 0.01)
            (jabber-mam-sync--refresh thread)
            (should (jabber-mam-sync--stale-p parent))
            (should (memq parent jabber-mam-sync--merge-queue))))
      (kill-buffer parent)
      (kill-buffer thread))))

(ert-deftest jabber-test-mam-sync-cancelled-with-connection ()
  "A lost connection cancels its job; the cancelled job reports nothing."
  (jabber-test-mam-sync--with-session
    (let (cancelled)
      (cl-letf (((symbol-function 'isolate-future-cancel) (lambda (f) (push f cancelled))))
        (jabber-mam-maybe-catchup jc)
        (jabber-mam-sync--start jabber-test-mam-sync--account)
        (run-hook-with-args 'jabber-lost-connection-hooks jc)
        (should (equal '(future) cancelled))
        ;; Until the cancelled worker answers, a reconnect starts no job.
        (jabber-mam-maybe-catchup jc)
        (jabber-mam-sync--start jabber-test-mam-sync--account)
        (should (= 1 (length spawned)))
        (setq summary '(:archives ((nil "chat" success 5 nil))
                        :peers (("romeo@montague.net" "a@x" "chat" 5 5))))
        (funcall done)
        (should-not toasts)
        (should-not jabber-mam-sync--merge-queue)
        ;; Its answer lets the pending catch-up start.
        (jabber-mam-sync--start jabber-test-mam-sync--account)
        (should (= 2 (length spawned)))))))

(ert-deftest jabber-test-mam-sync-no-worker-no-foreground-catch-up ()
  "A worker that cannot start is a failure toast, not a catch-up in the session."
  (jabber-test-mam-sync--with-session
    (let (queried)
      (cl-letf (((symbol-function 'isolate-spawn)
                 (lambda (&rest _) (signal 'isolate-world-unavailable '("no world"))))
                ((symbol-function 'jabber-mam--query) (lambda (&rest args) (push args queried))))
        (jabber-mam-maybe-catchup jc)
        (jabber-mam-sync--start jabber-test-mam-sync--account)
        (should-not queried)
        (should (= 1 (length toasts)))
        (should (string-prefix-p "Jabber: sync failed: no worker" (car toasts)))))))

(ert-deftest jabber-test-mam-sync-without-password-fails-named ()
  "No stored password: a named failure, never a prompt in a worker."
  (jabber-test-mam-sync--with-session
    (cl-letf (((symbol-function 'auth-source-search) (lambda (&rest _) nil)))
      (put jc :state-data (plist-put (get jc :state-data) :password nil))
      (jabber-mam-maybe-catchup jc)
      (jabber-mam-sync--start jabber-test-mam-sync--account)
      (should-not spawned)
      (should (equal '("Jabber: sync failed: no stored password for the account") toasts)))))

(ert-deftest jabber-test-mam-sync-counts-committed-results-only ()
  "Effects count once their transaction commits: rows added, peers changed."
  (let ((jabber-mam-sync--stored (make-hash-table :test #'equal)))
    (jabber-mam-sync--committed '(("me@x" "a@x" "chat" t) ("me@x" "a@x" "chat" nil)))
    (jabber-mam-sync--committed '(("me@x" "r@m" "groupchat" nil)))
    (should (equal '(1 . 1) (gethash '("me@x" "a@x" "chat") jabber-mam-sync--stored)))
    (should (equal '(0 . 1) (gethash '("me@x" "r@m" "groupchat") jabber-mam-sync--stored)))))

;;; Two writers on one database

(defmacro jabber-test-mam-sync--with-db (&rest body)
  "Run BODY with a fresh database open as `jabber-db--connection'."
  (declare (indent 0) (debug t))
  `(let* ((dir (make-temp-file "jabber-mam-sync-db" t))
          (jabber-db-path (expand-file-name "test.sqlite" dir))
          (jabber-db--connection nil))
     (unwind-protect
         (progn (jabber-db-ensure-open) ,@body)
       (jabber-db-close)
       (delete-directory dir t))))

(ert-deftest jabber-test-mam-sync-intent-marks-lock-acquisitions ()
  "The live session marks the statements that take the write lock, only those."
  (jabber-test-mam-sync--with-db
    (let ((db jabber-db--connection) seen)
      (should (jabber-db--acquires-lock-p db "BEGIN IMMEDIATE"))
      (should (jabber-db--acquires-lock-p db "INSERT INTO mam_progress VALUES (1)"))
      (should-not (jabber-db--acquires-lock-p db "COMMIT"))
      (should-not (jabber-db--acquires-lock-p db " RELEASE jabber_schema"))
      (let ((jabber-db--in-transaction db))
        (should-not (jabber-db--acquires-lock-p db "INSERT INTO x VALUES (1)")))
      (should-not (jabber-db--acquires-lock-p 'another-db "BEGIN IMMEDIATE"))
      (jabber-db--announce-write
       (lambda (&rest _) (push (file-exists-p (jabber-db--intent-file)) seen))
       db "BEGIN IMMEDIATE")
      (jabber-db--announce-write
       (lambda (&rest _) (push (file-exists-p (jabber-db--intent-file)) seen))
       db "COMMIT")
      (should (equal '(nil t) seen))
      (should-not (file-exists-p (jabber-db--intent-file))))))

(ert-deftest jabber-test-mam-sync-worker-defers-to-fresh-intent ()
  "The worker waits while a fresh mark exists, at most its patience; a stale one not."
  (jabber-test-mam-sync--with-db
    (let ((db jabber-db--connection)
          (jabber-db-intent-patience 0.3)
          (file (jabber-db--intent-file)))
      (write-region "" nil file nil 'silent)
      (let ((start (float-time)))
        (jabber-db--defer-to-session #'ignore db "BEGIN IMMEDIATE")
        (should (>= (- (float-time) start) 0.25)))
      ;; Statements inside the worker's own transaction never wait.
      (let ((start (float-time)) (jabber-db--in-transaction db))
        (jabber-db--defer-to-session #'ignore db "INSERT INTO x VALUES (1)")
        (should (< (- (float-time) start) 0.05)))
      (set-file-times file (time-subtract nil 10))
      (let ((start (float-time)))
        (jabber-db--defer-to-session #'ignore db "BEGIN IMMEDIATE")
        (should (< (- (float-time) start) 0.05)))
      (delete-file file))))

(ert-deftest jabber-test-mam-sync-live-write-waits-a-bounded-time ()
  "A write meeting another connection's lock fails after the busy timeout.
It never holds the session longer: the live session's bound."
  (jabber-test-mam-sync--with-db
    (let ((other (sqlite-open jabber-db-path))
          (jabber-db-busy-timeout 250))
      (unwind-protect
          (progn
            (sqlite-select jabber-db--connection
                           (format "PRAGMA busy_timeout=%d" jabber-db-busy-timeout))
            (sqlite-execute other "BEGIN IMMEDIATE")
            (let ((start (float-time)))
              (should-error (jabber-db-store-message
                             "me@x" "a@x" "in" "chat" "held" (floor (float-time))))
              (should (< (- (float-time) start) 1.0)))
            (sqlite-execute other "ROLLBACK")
            ;; Once the other writer is done, the same write succeeds.
            (should (jabber-db-store-message
                     "me@x" "a@x" "in" "chat" "free" (floor (float-time)))))
        (sqlite-close other)))))

(ert-deftest jabber-test-mam-sync-sharing-installs-one-role ()
  "Sharing the database installs one role's advice and removes it after."
  (unwind-protect
      (progn
        (jabber-db-share-writes 'announce)
        (should (advice-member-p #'jabber-db--announce-write 'sqlite-execute))
        (jabber-db-share-writes 'defer)
        (should-not (advice-member-p #'jabber-db--announce-write 'sqlite-execute))
        (should (advice-member-p #'jabber-db--defer-to-session 'sqlite-execute)))
    (jabber-db-share-writes nil))
  (should-not (advice-member-p #'jabber-db--defer-to-session 'sqlite-execute)))

;;; The worker's job, run in this VM against the virtual server

(defun jabber-test-mam-sync--disco (fsm stanza)
  "Answer STANZA's disco#info on FSM: every entity has MAM."
  (let ((to (jabber-xml-get-attribute stanza 'to)))
    (jabberd-send
     fsm `(iq ((type . "result") (id . ,(jabber-xml-get-attribute stanza 'id))
               ,@(when to `((from . ,to))))
              (query ((xmlns . "http://jabber.org/protocol/disco#info"))
                     (feature ((var . ,jabber-mam-xmlns))))))))

(defun jabber-test-mam-sync--archive (fsm stanza)
  "Answer STANZA's archive query on FSM: a room has two messages, the
personal archive one, in a single complete page."
  (let* ((id (jabber-xml-get-attribute stanza 'id))
         (to (jabber-xml-get-attribute stanza 'to))
         (queryid (jabber-xml-get-attribute (car (jabber-xml-get-children stanza 'query))
                                            'queryid))
         (archive (or to "romeo@montague.net"))
         (count (if to 2 1)))
    (dotimes (i count)
      (jabberd-send
       fsm `(message ((from . ,archive) (to . "romeo@montague.net/Orchard"))
                     (result ((xmlns . ,jabber-mam-xmlns) (queryid . ,queryid)
                              (id . ,(format "%s-%d" archive i)))
                             (forwarded ((xmlns . ,jabber-mam-forward-xmlns))
                                        (delay ((xmlns . ,jabber-mam-delay-xmlns)
                                                (stamp . ,(format "2026-11-0%dT10:00:00Z" (1+ i)))))
                                        (message ((type . ,(if to "groupchat" "chat"))
                                                  (from . ,(if to (concat to "/juliet")
                                                             "juliet@capulet.example/balcony"))
                                                  (to . "romeo@montague.net")
                                                  (id . ,(format "m-%s-%d" archive i)))
                                                 (body () ,(format "Hello %d" i))))))))
    (jabberd-send
     fsm `(iq ((type . "result") (id . ,id) ,@(when to `((from . ,to))))
              (fin ((xmlns . ,jabber-mam-xmlns) (complete . "true"))
                   (set ((xmlns . ,jabber-mam-rsm-xmlns))
                        (first () ,(format "%s-0" archive))
                        (last () ,(format "%s-%d" archive (1- count)))))))))

(ert-deftest jabber-test-mam-sync-worker-reads-archives-quietly ()
  "The job connects, reads each archive, sends no presence, and reports."
  (let* ((dir (make-temp-file "jabber-mam-sync" t))
         (jabber-db-path (expand-file-name "test.sqlite" dir))
         (jabber-db--connection nil)
         (jabber-connections nil)
         (jabber-virtual-server-function #'jabberd-handle)
         (jabberd-iq-get-handlers
          (cons (cons "http://jabber.org/protocol/disco#info" #'jabber-test-mam-sync--disco)
                jabberd-iq-get-handlers))
         (jabberd-iq-set-handlers
          (cons (cons jabber-mam-xmlns #'jabber-test-mam-sync--archive)
                jabberd-iq-set-handlers))
         (wire nil)
         (record (lambda (_fsm text) (push text wire))))
    (advice-add 'jabberd-handle :before record)
    (unwind-protect
        (let ((summary (jabber-mam-sync-worker-run
                        '(:username "romeo" :server "montague.net" :resource "Orchard-sync-1"
                          :password "foo" :connection-type virtual)
                        '((:room nil :with nil :nick nil)
                          (:room "room@conference.montague.net" :with nil :nick "romeo")))))
          (should-not (plist-get summary :error))
          (should (equal '((nil "chat" success 1 nil)
                           ("room@conference.montague.net" "groupchat" success 2 nil))
                         (plist-get summary :archives)))
          (should (equal '(("romeo@montague.net" "juliet@capulet.example" "chat" 1 1)
                           ("romeo@montague.net" "room@conference.montague.net" "groupchat" 2 2))
                         (sort (copy-sequence (plist-get summary :peers))
                               (lambda (a b) (string< (nth 1 a) (nth 1 b))))))
          (should-not (plist-get summary :refused))
          (should-not (seq-some (lambda (text) (string-search "<presence" text)) wire))
          (should-not (seq-some (lambda (text) (string-search "<message" text)) wire))
          ;; The virtual server offers Stream Management; the session never enables it.
          (should-not (seq-some (lambda (text) (string-search "urn:xmpp:sm:3" text)) wire))
          ;; The room got a cursor at its newest message.
          (should (equal "room@conference.montague.net-1"
                         (car (jabber-db-mam-progress "romeo@montague.net"
                                                      "room@conference.montague.net" nil))))
          (should-not jabber-connections))
      (advice-remove 'jabberd-handle record)
      (jabber-db-close)
      (delete-directory dir t))))

(provide 'jabber-test-mam-sync)
;;; jabber-test-mam-sync.el ends here
