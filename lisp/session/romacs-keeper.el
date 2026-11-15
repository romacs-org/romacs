;;; romacs-keeper.el --- The session's keeper  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A session process is restarted, upgraded, and sometimes crashes.  What
;; must outlive it is held by a small process of its own, the keeper: the
;; descriptors of kept resources, and the children it is the parent of.
;; The session holds duplicates of those descriptors and uses them
;; directly; nothing flows through the keeper.
;;
;; A keeper serves one session name.  `romacs-keeper-attach' finds it, or
;; starts one: in a systemd user unit of its own where there is a user
;; manager (so the end of the session's unit does not take it along), else
;; detached by itself.  It leaves when it holds nothing and no session has
;; been attached for `romacs-keeper-idle-seconds'.
;;
;; A handle names one kept resource.  `romacs-keeper-handles' lists them
;; with their state.  A session owns the handles it spawned or claimed;
;; when it quits they are closed unless marked kept, and when it restarts
;; or crashes they stay for its successor to claim.
;;
;; Batch sessions use no keeper unless they attach to one themselves.

;;; Code:

(defgroup romacs-keeper nil
  "The process that holds what outlives a session."
  :group 'processes)

(defcustom romacs-keeper-enabled (not noninteractive)
  "Whether this session keeps its resources in a keeper.
When nil, nothing outlives the session process."
  :type 'boolean)

(defcustom romacs-keeper-idle-seconds 60
  "Seconds a keeper stays with no handle and no session attached."
  :type 'natnum)

(defcustom romacs-keeper-program nil
  "The keeper's executable, or nil for the one beside this session's."
  :type '(choice (const :tag "Beside the running executable" nil) file))

(defcustom romacs-keeper-use-systemd t
  "Whether a keeper is started in a systemd user unit of its own.
It is, where a user manager answers; elsewhere, and when this is nil,
the keeper detaches by itself."
  :type 'boolean)

(defcustom romacs-keeper-unclaimed-deadline nil
  "Seconds a handle may stay unclaimed after a restart, or nil.
A restarting session sets this deadline on the handles it owns that are
not kept; one its successor has not claimed that long after this
session is gone is closed.  With nil
they are closed only by `romacs-keeper-close' or by the sweep a restored
session makes (`romacs-keeper-close-orphans').  A session that crashes
sets no deadline."
  :type '(choice (const :tag "No deadline" nil) natnum))

(defvar romacs-keeper-restarting nil
  "Non-nil while `kill-emacs-hook' runs for a restart, not a quit.
The session's handles then stay for its successor.")

(defun romacs-keeper-program ()
  "The keeper's executable."
  (or romacs-keeper-program
      (expand-file-name "romacs-keeper" invocation-directory)))

(defun romacs-keeper-attached-p ()
  "The pid of the keeper this session is attached to, or nil."
  (romacs--keeper-call 'attached))

(defun romacs-session-pid ()
  "The pid that names this session across restarts.
The keeper's when the session has one, else `emacs-pid'.  A child told
its parent's pid should be told this one: it does not change when the
session process is replaced."
  (or (romacs-keeper-attached-p) (emacs-pid)))

(defun romacs-keeper-socket (&optional name)
  "Where the keeper of session NAME listens.
NAME defaults to the server's name, else \"default\"."
  (romacs--keeper-call 'socket (or name (bound-and-true-p romacs-server-name) "default")))

(defun romacs-keeper--start (socket)
  "Start a keeper listening at SOCKET."
  (let* ((program (romacs-keeper-program))
         (idle (number-to-string (* 1000 romacs-keeper-idle-seconds)))
         (unit (format "romacs-keeper-%s" (substring (md5 socket) 0 12))))
    (unless (file-executable-p program)
      (error "No keeper executable at %s" program))
    (unless (and romacs-keeper-use-systemd
                 (file-directory-p "/run/systemd/system")
                 (executable-find "systemd-run")
                 (or (eql 0 (call-process "systemd-run" nil nil nil "--user" "--quiet" "--collect"
                                          "--unit" unit "--" program "--socket" socket
                                          "--foreground" "--idle-ms" idle))
                     ;; Refused because the unit exists: another session
                     ;; is starting this keeper.  Starting a detached one
                     ;; here could beat the unit's to the socket.
                     (with-temp-buffer
                       (call-process "systemctl" nil t nil "--user" "show" "--property" "ActiveState"
                                     "--value" (concat unit ".service"))
                       (member (string-trim (buffer-string)) '("active" "activating")))))
      (unless (eql 0 (call-process program nil nil nil "--socket" socket "--idle-ms" idle))
        (error "The keeper at %s did not start" program)))))

(defvar romacs-keeper-attachment nil
  "What `romacs-keeper-attach' returned for the keeper now attached.
Its :incarnation tells this keeper from another one: a handle saved
under one incarnation names nothing in another.")

(defun romacs-keeper--attach-socket (socket)
  "Attach to the keeper at SOCKET, starting one if none answers."
  (setq romacs-keeper-attachment (romacs-keeper--attach-socket-1 socket)))

(defun romacs-keeper--attach-socket-1 (socket)
  "Attach to the keeper at SOCKET, starting one if none answers."
  (when (romacs-keeper-attached-p)
    (error "This session is attached to the keeper with pid %d; detach first"
           (romacs-keeper-attached-p)))
  (or (ignore-errors (romacs--keeper-call 'attach socket))
      (progn
        (romacs-keeper--start socket)
        (let ((tries 200) attached)
          (while (and (not attached) (> tries 0))
            (setq attached (ignore-errors (romacs--keeper-call 'attach socket))
                  tries (1- tries))
            (unless attached (sleep-for 0.025)))
          (or attached (romacs--keeper-call 'attach socket))))))

(define-error 'romacs-keeper-unreachable "Cannot attach to this session's keeper" 'file-error)

(defun romacs-keeper-attach (&optional name)
  "Attach this session to the keeper of session NAME, starting it if needed.
Return a plist with the keeper's :pid and its :incarnation, which is new
in a new keeper and unchanged when a keeper replaces itself in place.
Any failure signals `romacs-keeper-unreachable', naming the socket and
the cause."
  (let ((socket nil))
    (condition-case err
        (progn
          (setq socket (romacs-keeper-socket name))
          (prog1 (romacs-keeper--attach-socket socket)
            (romacs--keeper-call 'configure (* 1000 romacs-keeper-idle-seconds))))
      (error (signal 'romacs-keeper-unreachable
                     (delq nil (list "Cannot attach to this session's keeper" socket
                                     (error-message-string err))))))))

(defun romacs-keeper-detach (&optional quit)
  "Detach from the keeper.
With QUIT, the keeper closes this session's handles that are not kept;
without, they stay for a successor."
  (when (romacs-keeper-attached-p)
    (romacs--keeper-call 'detach quit)))

(defun romacs-keeper-handles ()
  "The keeper's handles, each a plist.
:handle, :kind and :version name the resource; :metadata is its owner's
note; :pid and :status (`running', (exited CODE), (signaled SIGNAL),
(stopped SIGNAL) or `continued', each in a list) describe its child;
:owner and :owner-alive say which session has it; :kept and :deadline say
what happens to it unclaimed; :open lists its descriptor slots."
  (romacs--keeper-call 'list))

(defun romacs-keeper-spawn (kind version argv &rest options)
  "Have the keeper spawn ARGV as a handle of KIND at VERSION; its plist.
ARGV's first element is a file name, looked up in no path.  OPTIONS is a
plist.  The child gets exactly :environment (a list of \"NAME=value\"),
:directory (default \"/\"), :umask (default #o022), :signal-mask (a bit
per blocked signal, bit 0 for signal 1) and :descriptors, a list of
\(NUMBER . END) with END one of `null', `reads', `writes', `socket' and
`pty' (default: output on 1, the rest null); with :session it leads a
session of its own.  The keeper keeps the other end of each pipe, socket
and pty, and this session is handed duplicates, as the handle's slots in
that order.  :metadata is a unibyte note; :kept marks the handle to
survive a quit."
  (romacs--keeper-call 'spawn kind version (or (plist-get options :metadata) "")
                       (plist-get options :kept) argv
                       (plist-get options :environment)
                       (or (plist-get options :directory) "/")
                       (or (plist-get options :umask) #o022)
                       (or (plist-get options :signal-mask) 0)
                       (plist-get options :session)
                       (or (plist-get options :descriptors)
                           '((0 . null) (1 . writes) (2 . null)))))

(defun romacs-keeper-claim (handle &optional supersedes)
  "Become the owner of HANDLE and be handed its descriptors; its plist.
A handle another live session owns is refused, naming that session's
pid, unless SUPERSEDES is that pid: the keeper then kills it first."
  (romacs--keeper-call 'claim handle supersedes))

(defun romacs-keeper-read (handle slot max &optional seconds)
  "Read up to MAX bytes from HANDLE's descriptor SLOT, a unibyte string.
Return nil at end of file, and t when nothing arrives within SECONDS."
  (romacs--keeper-call 'read handle slot max seconds))

(defun romacs-keeper-write (handle slot string &optional seconds)
  "Write STRING to HANDLE's descriptor SLOT; how many bytes went.
That may be fewer than STRING has, and 0 when there is no room within
SECONDS: a program that does not read must not stop this session.  The
caller sends the rest later."
  (romacs--keeper-call 'write handle slot string seconds))

(defun romacs-keeper-close-slot (handle slot)
  "Close descriptor SLOT of HANDLE, in the keeper and in this session.
That is what lets a child reading it see end of file."
  (romacs--keeper-call 'close-slot handle slot))

(defun romacs-keeper-close (handle &optional signal)
  "Close HANDLE: its descriptors, and SIGNAL (a number) to its child."
  (romacs--keeper-call 'close handle signal))

(defun romacs-keeper-signal (handle signal &optional group)
  "Send SIGNAL (a number) to HANDLE's child, or with GROUP to its group.
Signals go through the keeper because it reaps: a pid this session
remembers may name another process by now."
  (romacs--keeper-call 'signal handle signal group))

(defun romacs-keeper-set-kept (handle kept)
  "Mark HANDLE to survive its session's quit, or with nil KEPT not to."
  (romacs--keeper-call 'set-kept handle kept))

(defun romacs-keeper-set-metadata (handle metadata)
  "Set HANDLE's note to the unibyte string METADATA."
  (romacs--keeper-call 'set-metadata handle metadata))

(defun romacs-keeper-set-deadline (handle time)
  "Close HANDLE at TIME (seconds since the epoch) if no session owns it then.
With nil TIME it has no deadline."
  (romacs--keeper-call 'set-deadline handle (and time (float time))))

(defun romacs-keeper-close-orphans ()
  "Close every handle whose session is gone and that is not kept.
What a restored session calls once its adopters have claimed theirs.
Return how many were closed."
  (romacs--keeper-call 'close-orphans))

(defun romacs-keeper-events ()
  "What the keeper said unasked since the last call, oldest first.
Each is (status . PLIST) for a handle whose child changed state, or
\(exec) when the keeper replaced itself."
  (romacs--keeper-call 'events))

(defun romacs-keeper-info ()
  "The attached keeper's :pid, :executable hash, idle time and handle count."
  (romacs--keeper-call 'info))

(defun romacs-keeper-upgrade ()
  "Replace the keeper in place by the installed one, if they differ.
The keeper keeps its pid, its children and its handles.  Return t when
it was replaced."
  (let ((program (romacs-keeper-program)))
    (unless (equal (plist-get (romacs-keeper-info) :executable)
                   (romacs--keeper-call 'executable-hash program))
      (romacs--keeper-call 'exec program)
      t)))

(defun romacs-keeper-shutdown ()
  "Have the keeper close everything and exit."
  (romacs--keeper-call 'shutdown))

(defun romacs-keeper--leave ()
  "At `kill-emacs': a quit closes this session's handles unless kept.
A restart (`romacs-keeper-restarting') leaves them for the successor,
under `romacs-keeper-unclaimed-deadline' when that is set; the end of
this process is then the detach."
  (when (romacs-keeper-attached-p)
    (ignore-errors
      (if (not romacs-keeper-restarting)
          (romacs-keeper-detach t)
        (when romacs-keeper-unclaimed-deadline
          (let ((at (+ (float-time) romacs-keeper-unclaimed-deadline)))
            (dolist (row (romacs-keeper-handles))
              (when (and (eql (plist-get row :owner) (emacs-pid)) (not (plist-get row :kept)))
                (romacs-keeper-set-deadline (plist-get row :handle) at)))))))))

(add-hook 'kill-emacs-hook #'romacs-keeper--leave)

(provide 'romacs-keeper)
;;; romacs-keeper.el ends here
