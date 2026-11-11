;;; romacs-server.el --- Server-first sessions: native clients and emacsclient -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A server session (`romacs --server NAME' starts one) outlives its
;; window: it listens before any window shows, a client asks for one, and
;; closing it keeps the session (DIVERGENCES.md SERVER2).  Two ways in:
;;
;; - The native endpoint (romacs-mcp.el) on
;;   $XDG_RUNTIME_DIR/romacs/NAME/server.sock, with the client methods
;;   `romacs/frame' (show the window and visit files, answering when they
;;   are done), `romacs/eval' and `romacs/stop'.
;; - GNU's server.el on $XDG_RUNTIME_DIR/emacs/NAME for emacsclient.  A
;;   text-terminal frame is refused (SERVER1); `-c' shows the window.
;;
;; A native client waiting on files is not one of GNU's `server-clients':
;; GNU's completion walks every client per finished buffer.  Waiters are
;; kept in two hash tables, buffer to its waiters and waiter to its pending
;; buffers, so finishing a buffer costs that buffer's waiters and a
;; disconnect costs that client's buffers.  `server-edit' (C-x #),
;; `server-done' and killing the buffer finish native waiters through
;; advice, keeping GNU's save question and recursion guard; GNU's own
;; clients of the same buffer are finished by GNU's code as always.

;;; Code:
(require 'cl-lib)
(require 'server)
(require 'romacs-mcp)

(defgroup romacs-server nil
  "Server-first Romacs sessions."
  :group 'external)

(defcustom romacs-server-exit-with-last-frame nil
  "Non-nil means closing the server's window ends the session.
Nil keeps the session, as GNU's daemon keeps its frames' session; the
next `romacs' shows the window again."
  :type 'boolean :group 'romacs-server)

(defcustom romacs-server-gnu-socket t
  "Non-nil means a server also listens for emacsclient through server.el."
  :type 'boolean :group 'romacs-server)

(defvar romacs-server-name nil
  "The name this session serves as, or nil when it is not a server.")

(defvar romacs-server-window-closed-hook nil
  "Hook run after the server's window closed and the session goes on.")

(cl-defstruct (romacs-server--client (:constructor romacs-server--client-make))
  "A native client waiting on files: its held request, its peer, and the
buffers it still waits on."
  handle peer (pending (make-hash-table :test #'eq)))

(defvar romacs-server--waiters (make-hash-table :test #'eq)
  "Buffer to the hash set of native clients waiting on it.")
(defvar romacs-server--peers (make-hash-table :test #'eq)
  "Peer process to the hash set of its waiting native clients.")
(defvar romacs-server--order nil
  "The first of the buffers native clients wait on, in the order they
asked for them: a doubly linked list of [PREVIOUS NEXT BUFFER] nodes.")
(defvar romacs-server--order-last nil)
(defvar romacs-server--nodes (make-hash-table :test #'eq)
  "Buffer to its node in `romacs-server--order': a buffer no one waits on
any more is unlinked at once.")

(defun romacs-server--link (buffer)
  "Put BUFFER last in the order, unless it is in it."
  (unless (gethash buffer romacs-server--nodes)
    (let ((node (vector romacs-server--order-last nil buffer)))
      (if romacs-server--order-last
          (aset romacs-server--order-last 1 node)
        (setq romacs-server--order node))
      (setq romacs-server--order-last node)
      (puthash buffer node romacs-server--nodes))))

(defun romacs-server--unlink (buffer)
  "Take BUFFER out of the order."
  (when-let* ((node (gethash buffer romacs-server--nodes)))
    (remhash buffer romacs-server--nodes)
    (let ((previous (aref node 0)) (next (aref node 1)))
      (if previous (aset previous 1 next) (setq romacs-server--order next))
      (if next (aset next 0 previous) (setq romacs-server--order-last previous)))))

(defun romacs-server-directory (name)
  "The private directory of server NAME's native socket."
  (expand-file-name (concat "romacs/" name "/")
                    (or (getenv "XDG_RUNTIME_DIR")
                        (format "/tmp/romacs%d" (user-uid)))))

;;; Native waiters

(defun romacs-server--finish (client)
  "Answer CLIENT: every buffer it waited on is done."
  (let* ((peer (romacs-server--client-peer client))
         (clients (gethash peer romacs-server--peers)))
    (when clients
      (remhash client clients)
      (when (zerop (hash-table-count clients)) (remhash peer romacs-server--peers))))
  (romacs-mcp-reply (romacs-server--client-handle client)
                    (romacs-mcp--object "done" t)))

(defun romacs-server-native-waiting-p (&optional buffer)
  "Non-nil when a native client waits on BUFFER (default current)."
  (gethash (or buffer (current-buffer)) romacs-server--waiters))

(defun romacs-server--release (buffer)
  "Finish BUFFER for its native waiters; non-nil when it had any.
A waiter whose last buffer this was is answered."
  (when-let* ((waiters (gethash buffer romacs-server--waiters)))
    (remhash buffer romacs-server--waiters)
    (romacs-server--unlink buffer)
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (remove-hook 'kill-buffer-hook #'romacs-server--kill-buffer t)))
    (maphash (lambda (client _)
               (let ((pending (romacs-server--client-pending client)))
                 (remhash buffer pending)
                 (when (zerop (hash-table-count pending))
                   (romacs-server--finish client))))
             waiters)
    t))

(defun romacs-server--wait (handle peer buffers)
  "Make the request HANDLE from PEER wait until BUFFERS are done."
  (let ((client (romacs-server--client-make :handle handle :peer peer)))
    (dolist (buffer buffers)
      (puthash buffer t (romacs-server--client-pending client))
      (let ((waiters (or (gethash buffer romacs-server--waiters)
                         (puthash buffer (make-hash-table :test #'eq) romacs-server--waiters))))
        (puthash client t waiters))
      (romacs-server--link buffer)
      (with-current-buffer buffer
        (add-hook 'kill-buffer-hook #'romacs-server--kill-buffer nil t)))
    (puthash client t (or (gethash peer romacs-server--peers)
                          (puthash peer (make-hash-table :test #'eq) romacs-server--peers)))
    client))

(defun romacs-server--peer-closed (peer)
  "Forget PEER's waiting clients: it went away before its files were done."
  (dolist (client (and (gethash peer romacs-server--peers)
                       (hash-table-keys (gethash peer romacs-server--peers))))
    (maphash (lambda (buffer _)
               (when-let* ((waiters (gethash buffer romacs-server--waiters)))
                 (remhash client waiters)
                 (when (zerop (hash-table-count waiters))
                   (remhash buffer romacs-server--waiters)
                   (romacs-server--unlink buffer)
                   (when (buffer-live-p buffer)
                     (with-current-buffer buffer
                       (remove-hook 'kill-buffer-hook #'romacs-server--kill-buffer t))))))
             (romacs-server--client-pending client)))
  (remhash peer romacs-server--peers))

(defun romacs-server--kill-buffer ()
  "Finish the current buffer's native waiters as it is killed.
Respects GNU's `server-kill-buffer-running' guard."
  (unless server-kill-buffer-running
    (let ((server-kill-buffer-running t))
      (romacs-server--release (current-buffer)))))

(defun romacs-server--buffer-done (buffer &optional _for-killing)
  "Before GNU's `server-buffer-done': finish BUFFER's native waiters too."
  (romacs-server--release buffer))

(defun romacs-server--next-buffer ()
  "The buffer native clients asked for first that is still waited on and
is not the current one, or nil.  Every buffer in the order is waited on."
  (let ((node romacs-server--order))
    (when (and node (eq (aref node 2) (current-buffer))) (setq node (aref node 1)))
    (and node (aref node 2))))

(defun romacs-server--done (orig)
  "Around `server-done': a buffer only native clients wait on is finished
here, with GNU's save question; otherwise GNU's body runs (its
`server-buffer-done' finishes native waiters too)."
  (if (or server-buffer-clients (not (romacs-server-native-waiting-p)))
      (funcall orig)
    (if (server-temp-file-p)
        (let ((version-control nil) (buffer-backed-up nil)) (save-buffer))
      (when (and (buffer-modified-p) buffer-file-name
                 (y-or-n-p (concat "Save file " buffer-file-name "? ")))
        (save-buffer)))
    (let ((buffer (current-buffer)))
      (romacs-server--release buffer)
      (let ((next (romacs-server--next-buffer)))
        (bury-buffer buffer)
        (list next nil)))))

(defun romacs-server--edit (orig &optional arg)
  "Around `server-edit' (C-x #): native-only buffers are done here too."
  (cond
   ((or arg server-clients) (funcall orig arg))
   ((romacs-server-native-waiting-p)
    (pcase-let ((`(,next ,_killed) (server-done)))
      (if (buffer-live-p next)
          (switch-to-buffer next)
        (message "No server editing buffers exist"))))
   ((romacs-server--next-buffer) (switch-to-buffer (romacs-server--next-buffer)))
   (t (funcall orig arg))))

;;; Client methods

(defun romacs-server--files (files cwd)
  "FILES, a vector of JSON objects with path, line and column, as
`server-visit-files' takes them, relative paths against CWD."
  (mapcar (lambda (file)
            (let ((line (gethash "line" file)) (column (gethash "column" file)))
              (cons (expand-file-name (gethash "path" file) cwd)
                    (and (integerp line) (cons line (if (integerp column) column 0))))))
          files))

(defun romacs-server--landing ()
  "A client with nothing to visit lands where the session is empty."
  (when (fboundp 'romacs-landing-new-client) (romacs-landing-new-client)))

(defun romacs-server--frame (peer params)
  "`romacs/frame': show the window, visit the files, and answer with
their buffers' names, or with `wait' once every one is done."
  (unless (romacs--native-window 'show)
    (error "The server could not show its window"))
  (let* ((cwd (file-name-as-directory (or (gethash "cwd" params) default-directory)))
         (files (romacs-server--files (gethash "files" params []) cwd))
         (buffers (let ((default-directory cwd))
                    (nreverse (server-visit-files files nil t)))))
    (if buffers
        (switch-to-buffer (car (last buffers)))
      (romacs-server--landing))
    (if (and buffers (eq (gethash "wait" params) t))
        (progn
          (romacs-server--wait (romacs-mcp-hold) peer buffers)
          (message "%s" (substitute-command-keys "When done with a buffer, type \\[server-edit]"))
          nil)
      (romacs-mcp--object "buffers" (vconcat (mapcar #'buffer-name buffers))))))

(defun romacs-server--eval (_peer params)
  "`romacs/eval': the value of the forms in CODE, printed."
  (romacs-mcp--object "value" (romacs-mcp-eval-string (gethash "code" params))))

(defun romacs-server--modified ()
  "File buffers with unsaved changes."
  (cl-remove-if-not (lambda (buffer) (and (buffer-file-name buffer) (buffer-modified-p buffer)))
                    (buffer-list)))

(defun romacs-server--stop (_peer params)
  "`romacs/stop': end the session after answering.  SAVE `all' saves every
modified file buffer and refuses to end while one stays unsaved; `none'
discards; `ask' shows the window and asks there, as `C-x C-c' would."
  (let ((save (or (gethash "save" params) "ask")))
    (pcase save
      ("all"
       ;; Every one, whatever `save-some-buffers-default-predicate' says.
       (dolist (buffer (romacs-server--modified))
         (with-current-buffer buffer (save-buffer)))
       (when-let* ((left (romacs-server--modified)))
         (error "Not stopping: %s not saved" (mapconcat #'buffer-name left ", ")))
       (run-at-time 0 nil #'kill-emacs)
       (romacs-mcp--object "stopping" t))
      ("none"
       (run-at-time 0 nil #'kill-emacs)
       (romacs-mcp--object "stopping" t))
      ("ask"
       ;; GNU's questions (unsaved files, processes, other clients) need a
       ;; window to be answered in.
       (unless (romacs--native-window 'show)
         (error "No window to ask in; use --stop=all or --stop=none"))
       (run-at-time 0 nil #'save-buffers-kill-emacs)
       (romacs-mcp--object "stopping" "asking in the window"))
      (_ (error "SAVE is ask, all or none")))))

;;; emacsclient

(defun romacs-server--create-frame (_display _nowait proc &rest _)
  "Instead of GNU's `server-create-window-system-frame' (emacsclient -c):
the server's window is the frame.  It is not the client's, so the
client's end does not delete it."
  (unless (romacs--native-window 'show)
    (error "The server could not show its window"))
  (let ((frame (selected-frame)))
    (process-put proc 'frame frame)
    ;; The session's terminal outlives every client.
    (process-put proc 'no-delete-terminal t)
    frame))

(defun romacs-server--execute (orig proc files nowait commands evalexprs dontkill frame tty)
  "Around `server-execute': a client that opened only a frame (`emacsclient
-c') and was shown *scratch* lands (`romacs-server--landing').  Not where
`initial-buffer-choice' chose: its function may clear it."
  (let ((chosen initial-buffer-choice))
    (funcall orig proc files nowait commands evalexprs dontkill frame tty)
    (when (and frame (not chosen) (not (or files commands evalexprs)))
      (romacs-server--landing))))

(defun romacs-server--filter (orig proc string)
  "Around `server-process-filter': emacsclient's -c asks for a window,
which server.el grants only where `x-create-frame' exists.  Romacs's frame
surface has another name (DIVERGENCES.md VM2-WIN-SUBSTRATE), so the name is
bound while a request is read, and the window frame is the server's."
  (if (fboundp 'x-create-frame)
      (funcall orig proc string)
    (cl-letf (((symbol-function 'x-create-frame) #'romacs--create-frame))
      (funcall orig proc string))))

(defun romacs-server--refuse-tty (&rest _)
  "Romacs has no text-terminal frames (SERVER1)."
  (error "Romacs has no text-terminal frames; use -c"))

(defun romacs-server--kill-terminal (orig &optional arg)
  "Around `save-buffers-kill-terminal': in a server, offer to save and
close the window; the session goes on (`romacs --stop' ends it)."
  (if (or (not romacs-server-name) romacs-server-exit-with-last-frame)
      (funcall orig arg)
    (save-some-buffers arg)
    (romacs--native-window 'hide)
    (run-hooks 'romacs-server-window-closed-hook)))

;;; A newer build

;; A server keeps the build it started with: an install only repoints what
;; the next launch runs.  The launcher names that in ROMACS_BUILD_WATCH (its
;; `current' link, or the binary a checkout's launcher builds); once it
;; names another build, the server says so once, at its next idle with its
;; window shown.

(defcustom romacs-server-build-watch (getenv "ROMACS_BUILD_WATCH")
  "The file whose target is the build a new launch runs, or nil."
  :type '(choice (const nil) file) :group 'romacs-server)

(defvar romacs-layer-toast-timeout)
(declare-function romacs-layer-toast "romacs-layer")
(declare-function romacs-agent--route-attached "romacs-agent")

(defvar romacs-server--build nil
  "The watched build's identity when this server started, then when it
last told of a newer one.")

(defun romacs-server--build-identity (file)
  "FILE's target and that target's file identity, or nil."
  (when-let* ((true (and file (file-truename file))) (attributes (file-attributes true)))
    (list true (file-attribute-inode-number attributes) (file-attribute-modification-time attributes))))

(defun romacs-server--build-label (true)
  "The short commit of the build at TRUE (a build directory, or a binary
beside its build.json), or nil."
  (let ((record (expand-file-name "build.json" (if (file-directory-p true) true (file-name-directory true)))))
    (when (file-readable-p record)
      (with-temp-buffer
        (insert-file-contents record)
        (let ((commit (ignore-errors (gethash "commit" (json-parse-buffer)))))
          (and (stringp commit) (substring commit 0 (min 8 (length commit)))))))))

(defun romacs-server-build-notice (true)
  "What the server says when the build at TRUE was installed."
  (let ((label (romacs-server--build-label true))
        (switch (if (fboundp 'romacs-restart)
                    (substitute-command-keys "\\[romacs-restart] to switch")
                  "quit and relaunch to switch")))
    (if label (format "Romacs %s installed; %s" label switch)
      (format "A newer Romacs build is installed; %s" switch))))

(defun romacs-server--check-build ()
  "Tell once of a newer build than this server's, when the window shows."
  (when (and romacs-server--build (romacs--native-window 'query))
    (let ((now (romacs-server--build-identity romacs-server-build-watch)))
      (when (and now (not (equal now romacs-server--build)))
        (setq romacs-server--build now)
        (let ((text (romacs-server-build-notice (car now))))
          (let ((inhibit-message t)) (message "%s" text))
          (if (fboundp 'romacs-layer-toast)
              ;; Long enough to read: it is said once.
              (let ((romacs-layer-toast-timeout 15)) (romacs-layer-toast text))
            (message "%s" text)))))))

(defun romacs-server--watch-build ()
  (when-let* ((identity (romacs-server--build-identity romacs-server-build-watch)))
    (setq romacs-server--build identity)
    ;; Every 2 s, acting only while idle: an idle timer runs once per idle
    ;; spell, and a window shown again by a client starts none.
    (run-at-time 2 2 (lambda () (when (current-idle-time) (romacs-server--check-build))))))

;;; Lifecycle

(defvar warm-restart-directory)

(defun romacs-server--pending-restart (name)
  "Where a restart of server NAME names its image (warm-restart.el's
`warm-restart-pending-file'), without loading warm-restart."
  (expand-file-name (format "pending-%s" name)
                    (or (bound-and-true-p warm-restart-directory)
                        (expand-file-name "romacs/warm-restart/" (or (getenv "XDG_STATE_HOME") "~/.local/state")))))

(defun romacs-server-window-closed ()
  "The server's window was closed: non-nil when the session should end."
  (if romacs-server-exit-with-last-frame
      (progn (romacs-gui-quit) romacs-gui-quit-requested)
    (romacs--native-window 'hide)
    (run-hooks 'romacs-server-window-closed-hook)
    nil))

;;;###autoload
(defun romacs-server-start (name)
  "Serve this session as NAME: the native socket, and GNU's for emacsclient."
  (let ((directory (romacs-server-directory name)))
    (make-directory directory t)
    (set-file-modes directory #o700)
    (setq romacs-server-name name)
    (romacs-mcp-register-method "romacs/frame" #'romacs-server--frame)
    (romacs-mcp-register-method "romacs/eval" #'romacs-server--eval)
    (romacs-mcp-register-method "romacs/stop" #'romacs-server--stop)
    (add-hook 'romacs-mcp-peer-close-functions #'romacs-server--peer-closed)
    (advice-add 'server-buffer-done :before #'romacs-server--buffer-done)
    (advice-add 'server-done :around #'romacs-server--done)
    (advice-add 'server-edit :around #'romacs-server--edit)
    (advice-add 'server-create-window-system-frame :override #'romacs-server--create-frame)
    (advice-add 'server-create-tty-frame :override #'romacs-server--refuse-tty)
    (advice-add 'server-process-filter :around #'romacs-server--filter)
    (advice-add 'server-execute :around #'romacs-server--execute)
    (advice-add 'save-buffers-kill-terminal :around #'romacs-server--kill-terminal)
    ;; An endpoint started before this (agent sessions restored at
    ;; startup) gives way: a server serves its own name's.
    (when romacs-mcp--listener (romacs-mcp-stop))
    (romacs-mcp-start (expand-file-name "server.sock" directory))
    ;; Sessions attached here already lead to the endpoint just replaced.
    (when (fboundp 'romacs-agent--route-attached) (romacs-agent--route-attached))
    (when romacs-server-gnu-socket
      (setq server-name name)
      (server-start))
    (romacs-server--watch-build)
    ;; Its own session record, as the launcher keeps a session's.
    (require 'romacs-usage-log)
    (romacs-usage-log-server-session name)
    ;; Started (by a launcher) while a restart of this server was under
    ;; way: the session it carries is restored here, once.
    (when (file-exists-p (romacs-server--pending-restart name))
      (run-at-time 0 nil (lambda () (require 'warm-restart) (warm-restart-adopt-pending name))))
    ;; Agent sessions outlive servers: attach to the ones still running.
    (when (file-directory-p (expand-file-name "romacs/agents/" (or (getenv "XDG_STATE_HOME") "~/.local/state")))
      (require 'romacs-agent)
      (romacs-agent-restore))
    name))

(provide 'romacs-server)
;;; romacs-server.el ends here
