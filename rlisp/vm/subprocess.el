;;; subprocess.el --- VM2 T3 tier: GNU process.c and callproc.c paths that call Lisp  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU's subprocess functions that run Lisp -- filters, sentinels, timers,
;; hooks, coding-system lookup and buffer insertion -- transcribed from
;; src/process.c, src/callproc.c and src/keyboard.c over the leaf
;; primitives of prims/process.rs.  Process objects, descriptors, polling
;; and status conversion are native; every Lisp call happens here, so the
;; values a loop keeps across a callback live in traced Lisp frames.
;;
;; Limits (DIVERGENCES.md VM2-PROC1, VM2-PROC2): these waits never read
;; the keyboard; `undecided' output decoding detects only ASCII and UTF-8;
;; network processes are TCP and local (Unix domain) stream sockets,
;; without Internet :local/:remote addresses, datagrams or TLS.

;;; Code:

;;;; Helpers

(defun romacs--check-keyword-args (args)
  "CHECK_KEYWORD_ARGS: ARGS has an even length."
  (if (= (% (length args) 2) 1)
      (signal 'malformed-keyword-arg-list nil)))

(defun romacs--required-string-keyword (contact keyword)
  "get_required_string_keyword_param."
  (let ((arg (plist-member contact keyword)))
    (if (not (and (consp arg) (consp (cdr arg))))
        (error "Missing %s keyword parameter" (symbol-name keyword)))
    (if (not (stringp (car (cdr arg))))
        (error "%s value not a string" (symbol-name keyword)))
    (car (cdr arg))))

(defun romacs--remove-slash-colon (name)
  "remove_slash_colon: strip a leading \"/:\" quoting prefix."
  (if (and (stringp name) (> (length name) 1)
           (eq (aref name 0) ?/) (eq (aref name 1) ?:))
      (substring name 2)
    name))

;; fileio.c Funhandled_file_name_directory.
(defun unhandled-file-name-directory (filename)
  "Return a directly usable directory name somehow associated with FILENAME."
  (let ((handler (find-file-name-handler filename 'unhandled-file-name-directory)))
    (if handler
        (let ((handled (funcall handler 'unhandled-file-name-directory filename)))
          (if (stringp handled) (file-name-as-directory handled) nil))
      (file-name-as-directory filename))))

(defun romacs--current-directory ()
  "callproc.c get_current_directory: the unhandled `default-directory', checked."
  (let* ((curdir default-directory)
         (dir (unhandled-file-name-directory curdir)))
    (if (null dir) (setq dir "~"))
    (setq dir (directory-file-name (expand-file-name dir)))
    (setq dir (romacs--remove-slash-colon dir))
    (romacs--check-directory dir curdir)
    dir))

(defun romacs--find-program (program)
  "make-process's openp over `exec-path' and `exec-suffixes', for X_OK."
  (let ((found (locate-file-internal program exec-path (default-value 'exec-suffixes) 1)))
    (if (null found)
        (signal 'file-missing (list "Searching for program" "No such file or directory" program)))
    found))

(defun romacs--environment-block (current-dir)
  "callproc.c make_environment_block: the child's environment for CURRENT-DIR."
  (let ((pwd (let ((dir current-dir))
               (while (and (> (length dir) 2) (eq (aref dir (1- (length dir))) ?/))
                 (setq dir (substring dir 0 -1)))
               (concat "PWD=" dir)))
        (display nil)
        (env nil)
        (tem process-environment))
    (while (and (consp tem) (stringp (car tem)))
      (let ((entry (car tem)))
        (if (and (>= (length entry) 7) (string= (substring entry 0 7) "DISPLAY")
                 (or (= (length entry) 7) (eq (aref entry 7) ?=)))
            (setq display t)))
      (setq tem (cdr tem)))
    (if (null display)
        (let ((value (frame-parameter (selected-frame) 'display)))
          (if (and (not (stringp value)) (consp initial-environment))
              (setq value (getenv-internal "DISPLAY" initial-environment)))
          (if (stringp value) (setq display value) (setq display nil))))
    (if (getenv-internal "PWD") (setq env (cons pwd env)))
    (if (stringp display) (setq env (cons (concat "DISPLAY=" display) env)))
    (setq tem process-environment)
    (while (and (consp tem) (stringp (car tem)))
      (setq env (cons (car tem) env))
      (setq tem (cdr tem)))
    (romacs--environment-entries (nreverse env))))

(defun romacs--environment-entries (strings)
  "add_env over STRINGS in order: each name's first entry, with a value.
A lone name is a placeholder that hides the later definitions of its name,
and is left out itself."
  (let ((seen (make-hash-table :test #'equal))
        (result nil))
    (dolist (string strings)
      (let* ((eq (string-search "=" string))
             (name (if eq (substring string 0 eq) string)))
        (unless (gethash name seen)
          (puthash name t seen)
          (if eq (setq result (cons string result))))))
    (nreverse result)))

(defun romacs--encode-file (name)
  "ENCODE_FILE: NAME as bytes for the operating system."
  (if (multibyte-string-p name)
      (encode-coding-string name (or file-name-coding-system default-file-name-coding-system 'utf-8-unix) t)
    name))

(defun romacs--coding-type (coding)
  (plist-get (coding-system-plist coding) :coding-type))

(defun romacs--complement-process-encoding-system (coding)
  "coding.c complement_process_encoding_system."
  (let ((base nil) (eol nil) (i 0) (found nil) (cs coding))
    (while (and (< i 3) (not found))
      (setq cs (cond ((= i 0) coding)
                     ((= i 1) (cdr-safe default-process-coding-system))
                     (t (car (coding-system-priority-list t)))))
      (when (and cs (coding-system-p cs))
        (if (and (null base) (not (eq (romacs--coding-type cs) 'undecided)))
            (setq base (coding-system-base cs)))
        (if (and (null eol) (not (vectorp (coding-system-eol-type cs))))
            (setq eol cs)))
      (if (and base eol) (setq found t) (setq i (1+ i))))
    (if (> i 0)
        ;; coding_inherit_eol_type (BASE, EOL).
        (let ((eols (and base (coding-system-eol-type base)))
              (index (and eol (coding-system-eol-type eol))))
          (if (and (vectorp eols) (integerp index)) (aref eols index) base))
      coding)))

(defun romacs--update-process-mark (proc)
  "update_process_mark: point PROC's mark at its buffer's end."
  (let ((buffer (process-buffer proc))
        (mark (process-mark proc)))
    (if (and (bufferp buffer) (buffer-live-p buffer)
             (not (eq (marker-buffer mark) buffer)))
        (set-marker mark (with-current-buffer buffer (point-max)) buffer))))

(defun romacs--setup-process-coding-systems (proc)
  "setup_process_coding_systems: the coding systems must exist."
  (let ((decode (romacs--proc-get proc 'decode))
        (encode (romacs--proc-get proc 'encode)))
    (if decode (check-coding-system decode))
    (if encode (check-coding-system encode))))

;;;; Creating processes

(defun romacs--pty-from-symbol (symbol)
  "is_pty_from_symbol."
  (cond ((eq symbol 'pty) t)
        ((eq symbol 'pipe) nil)
        ((null symbol) (not (null process-connection-type)))
        (t (signal 'file-error (list "Unknown connection type" symbol)))))

(defun make-process (&rest args)
  "Start a program in a subprocess.  Return the process object for it.
GNU process.c Fmake_process."
  (if (null args)
      nil
    (romacs--check-keyword-args args)
    (let* ((contact args)
           (file-handler (and (plist-get contact :file-handler)
                              (find-file-name-handler default-directory 'make-process))))
      (if file-handler
          (apply file-handler 'make-process contact)
        (romacs--make-process contact)))))

(defun romacs--make-process (contact)
  (let* ((buffer (plist-get contact :buffer))
         (buffer (and buffer (get-buffer-create buffer)))
         (current-dir (romacs--current-directory))
         (name (romacs--required-string-keyword contact :name))
         (command (plist-get contact :command))
         (program (if (consp command) (car command) nil))
         (query-on-exit (null (plist-get contact :noquery)))
         (xstderr (plist-get contact :stderr))
         (stderrproc nil)
         proc)
    (if program (or (stringp program) (signal 'wrong-type-argument (list 'stringp program))))
    (cond ((processp xstderr)
           (if (not (eq (process-type xstderr) 'pipe))
               (error "Process is not a pipe process"))
           (setq stderrproc xstderr))
          (xstderr
           (or (stringp program) (signal 'wrong-type-argument (list 'stringp program)))
           (setq stderrproc (make-pipe-process :name (concat name " stderr")
                                               :buffer (get-buffer-create xstderr)
                                               :noquery (if query-on-exit nil t)))))
    (setq proc (romacs--proc-make name))
    (unwind-protect
        (progn
          (romacs--proc-set proc 'childp t)
          (romacs--proc-set proc 'type 'real)
          (romacs--proc-set proc 'buffer buffer)
          (romacs--proc-set proc 'sentinel (or (plist-get contact :sentinel) 'internal-default-process-sentinel))
          (romacs--proc-set proc 'filter (or (plist-get contact :filter) 'internal-default-process-filter))
          (romacs--proc-set proc 'command (copy-sequence command))
          (if (not query-on-exit) (romacs--proc-set proc 'kill-without-query t))
          (let ((stop (plist-get contact :stop)))
            (if stop (signal 'wrong-type-argument (list 'null stop))))
          (let ((type (plist-get contact :connection-type)))
            (if (consp type)
                (progn (romacs--proc-set proc 'pty-in (romacs--pty-from-symbol (car type)))
                       (romacs--proc-set proc 'pty-out (romacs--pty-from-symbol (cdr type))))
              (let ((pty (romacs--pty-from-symbol type)))
                (romacs--proc-set proc 'pty-in pty)
                (romacs--proc-set proc 'pty-out pty))))
          (if stderrproc (romacs--proc-set proc 'stderrproc stderrproc))
          (romacs--update-process-mark proc)
          (let ((coding-systems t)
                (tem (plist-get contact :coding))
                (val nil))
            (setq val (if tem (if (consp tem) (car tem) tem) coding-system-for-read))
            (if (null val)
                (progn
                  (if program
                      (setq coding-systems
                            (apply #'find-operation-coding-system 'start-process name buffer command)))
                  (cond ((consp coding-systems) (setq val (car coding-systems)))
                        ((consp default-process-coding-system)
                         (setq val (car default-process-coding-system))))))
            (romacs--proc-set proc 'decode val)
            (setq val (if tem (if (consp tem) (cdr tem) tem) coding-system-for-write))
            (if (null val)
                (progn
                  (if (and (eq coding-systems t) program)
                      (setq coding-systems
                            (apply #'find-operation-coding-system 'start-process name buffer command)))
                  (cond ((consp coding-systems) (setq val (cdr coding-systems)))
                        ((consp default-process-coding-system)
                         (setq val (cdr default-process-coding-system))))))
            (romacs--proc-set proc 'encode val))
          (romacs--proc-set proc 'inherit-coding (and buffer inherit-process-coding-system))
          (if program
              (let ((path nil) (argv nil) (arg-encoding nil))
                (if (not (file-name-absolute-p program))
                    (setq path (expand-file-name (romacs--find-program program) default-directory))
                  (if (file-directory-p program)
                      (error "Specified program for new process is a directory"))
                  (setq path program))
                (setq path (romacs--remove-slash-colon path))
                (setq argv (list (romacs--encode-file path)))
                (dolist (arg (cdr command))
                  (or (stringp arg) (signal 'wrong-type-argument (list 'stringp arg)))
                  (if (multibyte-string-p arg)
                      (progn
                        (if (null arg-encoding)
                            (setq arg-encoding (romacs--complement-process-encoding-system
                                                (romacs--proc-get proc 'encode))))
                        (setq arg (let ((last-coding-system-used nil))
                                    (encode-coding-string arg arg-encoding t)))))
                  (setq argv (cons arg argv)))
                (romacs--setup-process-coding-systems proc)
                (romacs--proc-spawn proc (nreverse argv)
                                    (romacs--environment-block current-dir)
                                    (romacs--encode-file current-dir) stderrproc))
            (romacs--setup-process-coding-systems proc)
            (romacs--proc-pty proc)))
      ;; start_process_unwind: a process that did not start is removed.
      (let ((pid (or (process-id proc) 0)))
        (if (and (<= pid 0) (/= pid -2))
            (romacs--proc-remove proc))))
    proc))

(defun make-pipe-process (&rest args)
  "Create and return a bidirectional pipe process.
GNU process.c Fmake_pipe_process."
  (if (null args)
      nil
    (romacs--check-keyword-args args)
    (let* ((contact args)
           (name (romacs--required-string-keyword contact :name))
           (proc (romacs--proc-make name))
           (done nil))
      (unwind-protect
          (let ((buffer (let ((member (plist-member contact :buffer)))
                          (if member (car (cdr member)) name))))
            (romacs--proc-pipe proc)
            (if buffer (setq buffer (get-buffer-create buffer)))
            (romacs--proc-set proc 'buffer buffer)
            (romacs--proc-set proc 'childp contact)
            (romacs--proc-set proc 'plist (copy-sequence (plist-get contact :plist)))
            (romacs--proc-set proc 'type 'pipe)
            (romacs--proc-set proc 'sentinel (or (plist-get contact :sentinel) 'internal-default-process-sentinel))
            (romacs--proc-set proc 'filter (or (plist-get contact :filter) 'internal-default-process-filter))
            (if (plist-get contact :noquery) (romacs--proc-set proc 'kill-without-query t))
            (if (plist-get contact :stop) (romacs--proc-set proc 'command t))
            (if (and (not (eq (process-command proc) t))
                     (not (eq (process-filter proc) t)))
                (romacs--proc-set proc 'reading t))
            (romacs--update-process-mark proc)
            (let ((tem (plist-get contact :coding)) (val nil))
              (setq val (cond (tem (if (consp tem) (car tem) tem))
                              (coding-system-for-read coding-system-for-read)
                              ((if buffer
                                   (not (buffer-local-value 'enable-multibyte-characters buffer))
                                 (not (default-value 'enable-multibyte-characters)))
                               nil)
                              ((consp default-process-coding-system)
                               (car default-process-coding-system))))
              (romacs--proc-set proc 'decode val)
              (setq val (cond (tem (if (consp tem) (cdr tem) tem))
                              (coding-system-for-write coding-system-for-write)
                              ((not enable-multibyte-characters) nil)
                              ((consp default-process-coding-system)
                               (cdr default-process-coding-system))))
              (romacs--proc-set proc 'encode val))
            (romacs--setup-process-coding-systems proc)
            (setq done t))
        (if (not done) (romacs--proc-remove proc)))
      proc)))

(defun romacs--network-service (service)
  "The TCP port SERVICE names: t for any, an integer, or a numeric string."
  (cond ((eq service t) 0)
        ((integerp service) service)
        ((stringp service)
         (if (and (> (length service) 0)
                  (let ((digits t) (i 0))
                    (while (and digits (< i (length service)))
                      (if (not (and (>= (aref service i) ?0) (<= (aref service i) ?9))) (setq digits nil))
                      (setq i (1+ i)))
                    digits))
             (string-to-number service)
           ;; getservbyname: VM2 resolves numeric services only (VM2-PROC1).
           (error "VM2 resolves only numeric network services: %s" service)))
        (t (signal 'wrong-type-argument (list 'stringp service)))))

(defun make-network-process (&rest args)
  "Create and return a network server or client process.
GNU process.c Fmake_network_process, for TCP and local stream sockets."
  (if (null args)
      nil
    (romacs--check-keyword-args args)
    (let* ((contact args)
           (type (plist-get contact :type)))
      (if type (error "Unsupported connection type"))
      (let* ((name (romacs--required-string-keyword contact :name))
             (buffer (plist-get contact :buffer))
             (filter (plist-get contact :filter))
             (sentinel (plist-get contact :sentinel))
             (server (plist-get contact :server))
             (nowait (plist-get contact :nowait))
             (address (plist-get contact (if server :local :remote)))
             (family nil)
             (service nil)
             (host nil)
             (port nil)
             proc)
        (if (and server nowait) (error "`:server' is incompatible with `:nowait'"))
        (or (listp (plist-get contact :tls-parameters))
            (signal 'wrong-type-argument (list 'listp (plist-get contact :tls-parameters))))
        ;; :use-external-socket names the socket a daemon was started
        ;; with; there is none, so it is nil, as in GNU without one.
        (cond
         ;; :local ADDRESS or :remote ADDRESS: a string is a local socket's
         ;; path, and :host and :service are ignored.
         ((stringp address) (setq family 'local))
         (address (error "VM2 network processes take only a local socket's path as :local or :remote"))
         (t
          (setq family (let ((spec (plist-get contact :family)))
                         (cond ((null spec) nil)
                               ((eq spec 'local) 'local)
                               ((eq spec 'ipv4) 'ipv4)
                               ((eq spec 'ipv6) 'ipv6)
                               ((integerp spec)
                                (error "VM2 network processes do not support numeric address families"))
                               (t (error "Unknown address family")))))
          (setq service (plist-get contact :service)
                host (plist-get contact :host))
          (cond ((null host) (if (not (eq family 'local)) (setq host (if (eq family 'ipv6) "::1" "127.0.0.1"))))
                ((eq host 'local) (setq host (if (eq family 'ipv6) "::1" "127.0.0.1")))
                ((not (stringp host)) (signal 'wrong-type-argument (list 'stringp host))))
          (if (eq family 'local)
              (progn
                (if host
                    (progn (message ":family local ignores the :host property")
                           (setq contact (plist-put contact :host nil) host nil)))
                (or (stringp service) (signal 'wrong-type-argument (list 'stringp service)))
                (if (<= 108 (string-bytes service)) (error "Service name too long"))
                (setq address service))
            (setq port (romacs--network-service service))
            (if (not (and (<= 0 port) (< port 65536)))
                (signal 'error (list (format "Unknown service: %s" service)))))))
        (if buffer (setq buffer (get-buffer-create buffer)))
        (setq proc (romacs--proc-make name))
        (let ((done nil))
          (unwind-protect
              (progn
                (romacs--proc-set proc 'childp contact)
                (romacs--proc-set proc 'plist (copy-sequence (plist-get contact :plist)))
                (romacs--proc-set proc 'type 'network)
                (romacs--proc-set proc 'buffer buffer)
                (romacs--proc-set proc 'sentinel (or sentinel 'internal-default-process-sentinel))
                (romacs--proc-set proc 'filter (or filter 'internal-default-process-filter))
                (romacs--proc-set proc 'log (plist-get contact :log))
                (if (plist-get contact :noquery) (romacs--proc-set proc 'kill-without-query t))
                (if (plist-get contact :stop) (romacs--proc-set proc 'command t))
                (romacs--set-network-socket-coding-system proc host service name)
                ;; connect_network_socket.
                (let* ((addresses (romacs--proc-network proc server nowait host (or address port) family contact))
                       (failed (eq (car-safe addresses) 'failed))
                       (local (nth 0 addresses))
                       (remote (nth 1 addresses)))
                  (if (and addresses server (eql port 0))
                      (setq contact (plist-put contact :service (nth 2 addresses))))
                  (if (not failed)
                      (progn
                        (setq contact (plist-put contact (if server :local :remote) (if server local remote)))
                        (if (not server) (setq contact (plist-put contact :local local)))))
                  (romacs--proc-set proc 'childp contact)
                  (if server (romacs--proc-set proc 'status 'listen))
                  (romacs--update-process-mark proc)
                  ;; A :nowait connection is (connect) until the wait
                  ;; sees its nonblocking connect finish; one that failed
                  ;; at once is (failed ERROR), with no sentinel, as GNU's.
                  (cond
                   (failed (romacs--proc-set proc 'status (list 'failed (cdr addresses))))
                   (nowait (romacs--proc-set proc 'status (list 'connect)))
                   (t
                    (romacs--proc-set proc 'reading
                                      (or (and (not (eq (process-filter proc) t))
                                               (not (eq (process-command proc) t)))
                                          (and server (null (process-command proc)))))))
                  (romacs--setup-process-coding-systems proc)
                  (if (and (not failed) (not server) (plist-get contact :tls-parameters))
                      (romacs--boot-tls-parameters proc)))
                (setq done t))
            (if (not done) (romacs--proc-remove proc))))
        proc))))

(defun romacs--set-network-socket-coding-system (proc host service name)
  "set_network_socket_coding_system."
  (let* ((contact (romacs--proc-get proc 'childp))
         (buffer (process-buffer proc))
         (tem (plist-get contact :coding))
         (coding-systems t)
         (val nil))
    (setq val
          (cond (tem (if (consp tem) (car tem) tem))
                (coding-system-for-read coding-system-for-read)
                ((if buffer
                     (not (buffer-local-value 'enable-multibyte-characters buffer))
                   (not (default-value 'enable-multibyte-characters)))
                 nil)
                (t
                 (setq coding-systems
                       (if (or (null host) (null service)) nil
                         (find-operation-coding-system 'open-network-stream name buffer host service)))
                 (cond ((consp coding-systems) (car coding-systems))
                       ((consp default-process-coding-system) (car default-process-coding-system))))))
    (romacs--proc-set proc 'decode val)
    (setq val
          (cond (tem (if (consp tem) (cdr tem) tem))
                (coding-system-for-write coding-system-for-write)
                ((not enable-multibyte-characters) nil)
                (t
                 (if (eq coding-systems t)
                     (setq coding-systems
                           (if (or (null host) (null service)) nil
                             (find-operation-coding-system 'open-network-stream name buffer host service))))
                 (cond ((consp coding-systems) (cdr coding-systems))
                       ((consp default-process-coding-system) (cdr default-process-coding-system))))))
    (romacs--proc-set proc 'encode val)))

(defun format-network-address (address &optional omit-port)
  "Convert network ADDRESS from internal format to a string.
GNU process.c Fformat_network_address: a 4 or 5 element vector is an
IPv4 address, an 8 or 9 element one IPv6, each with a port unless
OMIT-PORT; a string (a local socket's path) is itself; a cons is its
family; nil for anything else, or for an element out of range."
  (cond ((null address) nil)
        ((stringp address) address)
        ((vectorp address)
         (let* ((size (length address))
                (spec (cond ((or (= size 4) (and (= size 5) omit-port)) (cons "%d.%d.%d.%d" 4))
                            ((= size 5) (cons "%d.%d.%d.%d:%d" 5))
                            ((or (= size 8) (and (= size 9) omit-port)) (cons "%x:%x:%x:%x:%x:%x:%x:%x" 8))
                            ((= size 9) (cons "[%x:%x:%x:%x:%x:%x:%x:%x]:%d" 9))))
                (args (and spec (append (substring address 0 (cdr spec)) nil)))
                (valid spec)
                (i 0))
           (dolist (item args)
             (unless (and (fixnump item) (<= 0 item 65535)
                          (or (> (cdr spec) 5) (>= i 4) (<= item 255)))
               (setq valid nil))
             (setq i (1+ i)))
           (and valid (apply #'format (car spec) args))))
        ((consp address) (format "<Family %d>" (car address)))))

(defun romacs--server-accept-connection (server)
  "server_accept_connection: accept one client of SERVER."
  (let ((accepted (romacs--proc-accept server)))
    (cond
     ((null accepted) nil)
     ((eq (car accepted) 'error)
      (if (romacs--proc-get server 'log)
          (funcall (romacs--proc-get server 'log) server nil
                   (concat "accept failed with code" (number-to-string (cdr accepted)) "\n"))))
     (t
      (let* ((connection (nth 0 accepted))
             (host (nth 1 accepted))
             (service (nth 2 accepted))
             (remote (nth 3 accepted))
             (local (nth 4 accepted))
             (suffix (nth 5 accepted))
             (filter (process-filter server))
             (buffer nil)
             (contact nil)
             proc)
        (if (or (eq filter 'internal-default-process-filter) (eq filter t))
            (let ((base (if (process-buffer server)
                            (buffer-name (process-buffer server))
                          (process-name server))))
              (if base (setq buffer (get-buffer-create (concat base suffix))))))
        (setq proc (romacs--proc-make (concat (process-name server) suffix)))
        ;; An accepted client takes its server's lock (process.c:5116).
        (set-process-thread proc (process-thread server))
        (setq contact (copy-sequence (romacs--proc-get server 'childp)))
        (setq contact (plist-put contact :server nil))
        (setq contact (plist-put contact :host host))
        (if service (setq contact (plist-put contact :service service)))
        (setq contact (plist-put contact :remote remote))
        (if local (setq contact (plist-put contact :local local)))
        (romacs--proc-set proc 'childp contact)
        (romacs--proc-set proc 'plist (copy-sequence (process-plist server)))
        (romacs--proc-set proc 'type 'network)
        (romacs--proc-set proc 'buffer buffer)
        (romacs--proc-set proc 'sentinel (process-sentinel server))
        (romacs--proc-set proc 'filter filter)
        (romacs--proc-attach proc connection)
        (romacs--proc-set proc 'status 'run)
        (romacs--proc-set proc 'decode (romacs--proc-get server 'decode))
        (romacs--proc-set proc 'encode (romacs--proc-get server 'encode))
        (romacs--proc-set proc 'inherit-coding
                          (and buffer (romacs--proc-get server 'inherit-coding)))
        (let ((host-string (if (stringp host) host "-")))
          (if (romacs--proc-get server 'log)
              (funcall (romacs--proc-get server 'log) server proc
                       (concat "accept from " host-string "\n")))
          (romacs--exec-sentinel proc (concat "open from " host-string "\n"))))))))

;;;; Process state changes that run Lisp

(defun set-process-buffer (process buffer)
  "Set buffer associated with PROCESS to BUFFER (a buffer, or nil).
GNU process.c Fset_process_buffer."
  (or (processp process) (signal 'wrong-type-argument (list 'processp process)))
  (if buffer (or (bufferp buffer) (signal 'wrong-type-argument (list 'bufferp buffer))))
  (if (not (eq (process-buffer process) buffer))
      (progn
        (romacs--proc-set process 'buffer buffer)
        (romacs--update-process-mark process)))
  (if (memq (process-type process) '(network serial pipe))
      (romacs--proc-set process 'childp
                        (plist-put (romacs--proc-get process 'childp) :buffer buffer)))
  (romacs--setup-process-coding-systems process)
  buffer)

(defun set-process-coding-system (process &optional decoding encoding)
  "Set coding systems of PROCESS to DECODING and ENCODING.
GNU process.c Fset_process_coding_system."
  (or (processp process) (signal 'wrong-type-argument (list 'processp process)))
  (if (not (romacs--proc-get process 'infd))
      (error "Input file descriptor of %s closed" (process-name process)))
  (if (not (romacs--proc-get process 'outfd))
      (error "Output file descriptor of %s closed" (process-name process)))
  (check-coding-system decoding)
  (check-coding-system encoding)
  (romacs--proc-set process 'encode encoding)
  (romacs--proc-set process 'decode decoding)
  (romacs--setup-process-coding-systems process)
  nil)

(defun delete-process (&optional process)
  "Delete PROCESS: kill it and forget about it immediately.
GNU process.c Fdelete_process."
  (interactive (list 'message))
  (let ((mess nil))
    (if (eq process 'message) (setq mess t process nil))
    (setq process (romacs--get-process process))
    (if (memq (process-type process) '(network serial pipe))
        (progn
          (romacs--proc-kill-for-delete process)
          (romacs--proc-set process 'status '(exit 0))
          (romacs--proc-tick process)
          (romacs--status-notify process nil))
      (if (romacs--proc-kill-for-delete process)
          (romacs--status-notify process nil)))
    (romacs--proc-remove process)
    (if mess (message "Deleted process"))
    nil))

(defun romacs--kill-buffer-processes (buffer)
  "process.c kill_buffer_processes: BUFFER's processes, as it is killed."
  (dolist (proc (process-list))
    (if (or (null buffer) (eq (process-buffer proc) buffer))
        (if (memq (process-type proc) '(network serial pipe))
            (delete-process proc)
          (if (romacs--proc-get proc 'infd)
              (romacs--proc-signal proc 1 nil))))))

;;;; Signals

(defun romacs--send-signal (process signo current-group)
  "process_send_signal, then status_notify when SIGCONT revived PROCESS."
  (if (romacs--proc-signal (romacs--get-process process) signo current-group)
      (romacs--status-notify nil nil)))

(defun internal-default-interrupt-process (&optional process current-group)
  "Default function to interrupt process PROCESS."
  (romacs--send-signal process 2 current-group)
  process)

(defun interrupt-process (&optional process current-group)
  "Interrupt process PROCESS.  GNU process.c Finterrupt_process."
  (run-hook-with-args-until-success 'interrupt-process-functions process current-group))

(defun kill-process (&optional process current-group)
  "Kill process PROCESS.  GNU process.c Fkill_process."
  (interactive (list (read-process-name "Kill process")))
  (romacs--send-signal process 9 current-group)
  process)

(defun quit-process (&optional process current-group)
  "Send QUIT signal to process PROCESS.  GNU process.c Fquit_process."
  (romacs--send-signal process 3 current-group)
  process)

(defun stop-process (&optional process current-group)
  "Stop process PROCESS.  GNU process.c Fstop_process."
  (if (and (processp process) (memq (process-type process) '(network serial pipe)))
      (progn
        (if (and (null (process-command process)) (romacs--proc-get process 'infd))
            (romacs--proc-set process 'reading nil))
        (romacs--proc-set process 'command t))
    (romacs--send-signal process 20 current-group))
  process)

(defun continue-process (&optional process current-group)
  "Continue process PROCESS.  GNU process.c Fcontinue_process."
  (if (and (processp process) (memq (process-type process) '(network serial pipe)))
      (progn
        (if (and (eq (process-command process) t)
                 (romacs--proc-get process 'infd)
                 (or (not (eq (process-filter process) t))
                     (eq (romacs--proc-get process 'status) 'listen)))
            (romacs--proc-set process 'reading t))
        (romacs--proc-set process 'command nil))
    (romacs--send-signal process 18 current-group))
  process)

(defun signal-process (process sigcode &optional remote)
  "Send PROCESS the signal with code SIGCODE.  GNU process.c Fsignal_process."
  (run-hook-with-args-until-success 'signal-process-functions process sigcode remote))

;;;; Sending

(defun romacs--connecting-p (proc)
  "connecting_status: PROC's :nowait connection has not completed."
  (eq (car-safe (romacs--proc-get proc 'status)) 'connect))

(defun romacs--wait-while-connecting (proc)
  "wait_while_connecting."
  (while (romacs--connecting-p proc)
    (romacs--wait-reading-process-output 0.02 nil 0)))

(defun romacs--connect-finished (proc errno remote local)
  "The wait's handling of PROC's completed :nowait connection: run and
\"open\" with the REMOTE and LOCAL addresses it connected, or failed
with ERRNO.  A connection with a TLS handshake to go is \"open\" once
that is done (`romacs--tls-finished')."
  (if (/= errno 0)
      (progn
        (romacs--proc-tick proc)
        (romacs--proc-set proc 'status (list 'failed errno))
        (romacs--proc-deactivate proc))
    (if remote
        (romacs--proc-set proc 'childp
                          (plist-put (plist-put (romacs--proc-get proc 'childp) :remote remote)
                                     :local local)))
    (unless (plist-get (romacs--proc-get proc 'childp) :tls-parameters)
      (romacs--proc-set proc 'status 'run)
      (romacs--exec-sentinel proc "open\n"))
    (if (and (romacs--proc-get proc 'infd)
             (not (eq (process-filter proc) t))
             (not (eq (process-command proc) t)))
        (romacs--proc-set proc 'reading t))))

(defun romacs--boot-tls-parameters (proc)
  "connect_network_socket's boot of PROC's :tls-parameters (TYPE . PLIST):
a blocking client's handshake is done here and its connection finished
\(`romacs--finish-tls-connection'); a :nowait one's goes on in the wait."
  (let* ((params (plist-get (romacs--proc-get proc 'childp) :tls-parameters))
         (boot (gnutls-boot proc (car params) (cdr params)))
         (stage (gnutls-get-initstage proc)))
    (cond
     ((= stage 9) (romacs--finish-tls-connection proc))
     ((/= stage 8)
      (romacs--proc-deactivate proc)
      (unless (eq (car-safe (romacs--proc-get proc 'status)) 'failed)
        (romacs--proc-set proc 'status
                          (list 'failed (if boot (gnutls-error-string boot) "TLS negotiation failed"))))))))

(defun romacs--tls-finished (proc result)
  "The wait's handling of PROC's TLS handshake that ended with RESULT:
gnutls_verify_boot, then the connection finished, or failed."
  (cond
   ((not (eq result t)) (romacs--tls-failed proc "TLS negotiation failed"))
   ((eq (romacs--gnutls-verify proc) t) (romacs--finish-tls-connection proc))
   ;; gnutls_verify_boot's boot_error left the reason in the status.
   (t (romacs--tls-failed proc nil))))

(defun romacs--tls-failed (proc reason)
  "PROC's TLS connection failed for REASON (nil: as its status says):
closed, and its sentinel told."
  (romacs--proc-tick proc)
  (if reason (romacs--proc-set proc 'status (list 'failed reason)))
  (romacs--proc-deactivate proc))

(defun romacs--finish-tls-connection (proc)
  "finish_after_tls_connection: the Network Security Manager's verdict on
PROC, then run and \"open\" (its handshake waited for the connection)."
  (let* ((contact (romacs--proc-get proc 'childp))
         (ok (if (fboundp 'nsm-verify-connection)
                 (nsm-verify-connection proc (plist-get contact :host) (plist-get contact :service))
               t)))
    (cond
     ((null ok) (romacs--tls-failed proc "The Network Security Manager stopped the connections"))
     ((not (romacs--proc-get proc 'infd))
      (romacs--proc-tick proc)
      (romacs--proc-set proc 'status 'failed)
      (romacs--proc-deactivate proc))
     (t
      (romacs--proc-set proc 'status 'run)
      (romacs--exec-sentinel proc "open\n")))))

(defun romacs--send-process (proc string)
  "send_process: encode STRING as PROC's encoding requires, and write it,
waiting (and reading output) while the channel is full."
  (if (eq (process-type proc) 'network) (romacs--wait-while-connecting proc))
  (let ((bytes string))
    (if (multibyte-string-p string)
        (let ((coding (romacs--complement-process-encoding-system (romacs--proc-get proc 'encode))))
          (romacs--proc-set proc 'encode coding)
          (setq bytes (encode-coding-string string coding t))))
    (if (not (romacs--proc-send proc bytes))
        (while (not (romacs--proc-send proc nil))
          (romacs--wait-reading-process-output 0.02 nil 0)))))

(defun process-send-string (process string)
  "Send PROCESS the contents of STRING as input.  GNU process.c Fprocess_send_string."
  (or (stringp string) (signal 'wrong-type-argument (list 'stringp string)))
  (romacs--send-process (romacs--get-process process) string)
  nil)

(defun process-send-region (process start end)
  "Send current contents of region as input to PROCESS.  GNU process.c Fprocess_send_region."
  (let ((proc (romacs--get-process process)))
    (if (eq (process-type proc) 'network) (romacs--wait-while-connecting proc))
    (romacs--send-process proc (buffer-substring-no-properties start end)))
  nil)

(defun process-send-eof (&optional process)
  "Make PROCESS see end-of-file in its input.  GNU process.c Fprocess_send_eof."
  (let ((proc (romacs--get-process process)))
    (if (eq (process-type proc) 'network) (romacs--wait-while-connecting proc))
    (if (not (eq (romacs--proc-get proc 'status) 'run))
        (error "Process %s not running: %s" (process-name proc)
               (romacs--proc-status-message proc)))
    (romacs--proc-send-eof proc)
    process))

;;;; Default filter and sentinel

(defun internal-default-process-filter (proc text)
  "Function used as default process filter.  GNU process.c
Finternal_default_process_filter: insert TEXT at PROC's mark in its
buffer, which is left current."
  (or (processp proc) (signal 'wrong-type-argument (list 'processp proc)))
  (or (stringp text) (signal 'wrong-type-argument (list 'stringp text)))
  (let ((buffer (process-buffer proc)))
    (if (and buffer (buffer-live-p buffer))
        (let ((mark (process-mark proc))
              opoint old-read-only old-begv old-zv before)
          (set-buffer buffer)
          (setq opoint (point) old-read-only buffer-read-only
                old-begv (point-min) old-zv (point-max))
          (setq buffer-read-only nil)
          (goto-char (if (marker-buffer mark) mark (point-max)))
          (setq before (point))
          (if (not (eq (not enable-multibyte-characters) (not (multibyte-string-p text))))
              (setq text (if (multibyte-string-p text)
                             (string-as-unibyte text)
                           (string-to-multibyte text))))
          (insert-before-markers text)
          ;; read_process_output_after_insert.
          (if (and (bufferp (process-buffer proc)) (not (eq (process-buffer proc) (current-buffer))))
              (set-marker mark (with-current-buffer (process-buffer proc) (point)) (process-buffer proc))
            (set-marker mark (point) (process-buffer proc)))
          (let ((delta (- (point) before)))
            (if (>= opoint before) (setq opoint (+ opoint delta)))
            (if (> old-begv before) (setq old-begv (+ old-begv delta)))
            (if (>= old-zv before) (setq old-zv (+ old-zv delta))))
          (if (or (/= old-begv (point-min)) (/= old-zv (point-max)))
              (narrow-to-region old-begv old-zv))
          (setq buffer-read-only old-read-only)
          (goto-char opoint))))
  nil)

(defun internal-default-process-sentinel (proc msg)
  "Function used as default sentinel for processes.  GNU process.c
Finternal_default_process_sentinel."
  (or (processp proc) (signal 'wrong-type-argument (list 'processp proc)))
  (let ((buffer (process-buffer proc))
        (symbol (romacs--proc-get proc 'status)))
    (if (consp symbol) (setq symbol (car symbol)))
    (if (and (not (eq symbol 'run)) buffer (buffer-live-p buffer))
        (save-current-buffer
          (set-buffer buffer)
          (let ((mark (process-mark proc))
                (opoint (point))
                before tem)
            (if (not enable-multibyte-characters)
                (setq msg (encode-coding-string msg locale-coding-system t)))
            (if (marker-buffer mark) (goto-char mark) (goto-char (point-max)))
            (setq before (point))
            (setq tem buffer-read-only)
            (setq buffer-read-only nil)
            (insert "\nProcess ")
            (insert (process-name proc))
            (insert " ")
            (insert msg)
            (setq buffer-read-only tem)
            (set-marker mark (point) buffer)
            (if (>= opoint before)
                (goto-char (+ opoint (- (point) before)))
              (goto-char opoint))))))
  nil)

;;;; Reading output, filters and sentinels

(defun romacs--process-error (err context)
  "read_process_output_error_handler: cmd_error_internal on ERR with
CONTEXT through `command-error-function', then the pause."
  (let ((inhibit-quit t))
    (if command-error-function
        (funcall command-error-function err context nil)))
  (if (> process-error-pause-time 0) (sleep-for process-error-pause-time)))

(defun romacs--process-call (function args context)
  "internal_condition_case_1 around FUNCTION: an error is reported, unless
`debug-on-error' lets it through."
  (if debug-on-error
      (apply function args)
    (condition-case err
        (apply function args)
      (error (romacs--process-error (if (consp err) err (cons 'error err)) context)))))

(defun romacs--read-process-output (proc &optional from-wait)
  "read_process_output: read PROC's channel once and give the text to its
filter; the byte count, negative after a failed read.  FROM-WAIT applies
the wait loop's end-of-file and error handling."
  (let* ((result (romacs--proc-read proc))
         (nread (car result))
         (text (cdr result)))
    (if (and (>= nread 0) (stringp text))
        (let ((odeactivate deactivate-mark))
          (save-current-buffer
            (romacs--dispose-process-output proc text))
          (setq deactivate-mark odeactivate)))
    (if from-wait
        (romacs--proc-read-done proc nread (if (< nread 0) text 0)))
    nread))

(defun romacs--dispose-process-output (proc text)
  "read_and_dispose_of_process_output: call PROC's filter on TEXT with the
caller's match data preserved."
  (let ((filter (process-filter proc))
        (match-data (match-data)))
    (let ((inhibit-quit t)
          (last-nonmenu-event t))
      (unwind-protect
          (if (> (string-bytes text) 0)
              (romacs--process-call filter (list proc text) "error in process filter: "))
        (set-match-data match-data t)))))

(defun romacs--exec-sentinel (proc reason)
  "exec_sentinel: call PROC's sentinel with REASON."
  (let ((odeactivate deactivate-mark)
        (match-data (match-data)))
    (save-current-buffer
      (let ((inhibit-quit t)
            (last-nonmenu-event t))
        (unwind-protect
            (romacs--process-call (process-sentinel proc) (list proc reason)
                                  "error in process sentinel: ")
          (set-match-data match-data t))))
    (setq deactivate-mark odeactivate)))

(defun romacs--status-notify (deleting wait-proc)
  "status_notify: run the sentinel of each process whose status changed,
first reading what remains of its output.  GNU's got_some_output."
  (let ((got -1))
    (romacs--proc-ticks-acknowledge)
    (dolist (proc (process-list))
      (if (romacs--proc-changed proc)
          (let ((more t))
            (while (and more
                        (not (eq (process-filter proc) t))
                        (not (romacs--connecting-p proc))
                        (not (eq (romacs--proc-get proc 'status) 'listen))
                        (not (eq (process-command proc) t))
                        (romacs--proc-get proc 'infd)
                        (not (eq proc deleting)))
              (let ((nread (romacs--read-process-output proc)))
                (if (and (or (null wait-proc) (eq wait-proc proc)) (< got nread))
                    (setq got nread))
                (if (<= nread 0) (setq more nil))))
            (romacs--exec-sentinel proc (romacs--proc-status-finish proc)))))
    got))

;;;; Timers

(defvar romacs--timers-run 0 "keyboard.c timers_run: how many timers have run.")

(defun romacs--timer-start-idle ()
  "keyboard.c timer_start_idle: unless already idle, Emacs is idle from now
on, and every idle timer may run once more."
  (unless (current-idle-time)
    (romacs--set-idle t)
    ;; timer.el is GNU's preloaded Lisp; a bare VM has no idle timers.
    (if (fboundp 'internal-timer-start-idle) (internal-timer-start-idle))))

(defun romacs--timer-stop-idle ()
  "keyboard.c timer_stop_idle."
  (romacs--set-idle nil))

(defun romacs--timer-check-2 (timers idle-timers)
  "keyboard.c timer_check_2: of ordinary TIMERS and IDLE-TIMERS, run the
next valid timer if ripe and return 0, else the seconds until it is, or nil
if there is none (`romacs--timer-next' chooses).  An idle timer's time
counts from the start of idleness."
  (let ((chosen (romacs--timer-next timers idle-timers)))
    (if (not (vectorp chosen))
        chosen
      (let ((old-deactivate-mark deactivate-mark))
        ;; Triggered, in case its Lisp fails to reschedule it.
        (aset chosen 0 t)
        (let ((inhibit-quit t))
          (funcall 'timer-event-handler chosen))
        (setq deactivate-mark old-deactivate-mark)
        (setq romacs--timers-run (1+ romacs--timers-run)))
      0)))

(defvar romacs-timer-pass-budget 0.008
  "Seconds of timers one pass runs before it lets pending input go first.
GNU's timer_check runs every ripe timer at once; a burst of them (a
network stream queues one per stanza) then holds the key behind it.  Past
this budget, with input pending, the ripe timers left run on the next pass,
in order, after the key.  Nil runs them all, as GNU does.")

(defun romacs--timer-check ()
  "keyboard.c timer_check: run every ripe timer, idle timers only while
Emacs is idle; the seconds until the next one, or nil.  Copies of the lists
let a timer add itself again without running twice.  Once the pass has run
`romacs-timer-pass-budget' and input is pending, it stops and answers 0: the
ripe timers left stay in the lists for the next pass."
  (let ((timers (copy-sequence timer-list))
        (idle-timers (and (current-idle-time) (copy-sequence timer-idle-list)))
        (start (float-time))
        (next 0))
    (while (and (eql next 0)
                (not (and romacs-timer-pass-budget
                          (> (- (float-time) start) romacs-timer-pass-budget)
                          (input-pending-p))))
      (setq next (romacs--timer-check-2 timers idle-timers)))
    next))

(defun romacs--timer-delay ()
  "The seconds until the next valid timer in the current lists falls due,
0 when one is ripe, or nil if none: what `romacs--timer-check' answers,
without running a timer, so it counts timers added since that check.  Idle
timers count only while Emacs is idle, from the start of idleness."
  (let ((next (romacs--timer-next timer-list (and (current-idle-time) timer-idle-list))))
    (if (vectorp next) 0 next)))

(defun romacs--timer-check-idle ()
  "What keyboard.c read_char does before it blocks for a command key with
no time limit: start idling, then run ripe timers as its wait does.  Return
\=(RAN . DELAY): whether a timer ran, so the caller redisplays, and the
seconds until the next timer, or nil."
  (romacs--timer-start-idle)
  ;; A collection at a request boundary had no engine to finish it in.
  (romacs--run-finalizers)
  (let* ((run romacs--timers-run)
         (delay (romacs--timer-check)))
    (cons (/= run romacs--timers-run) delay)))

;;;; Waiting

(defun romacs--wait-reading-process-output (limit wait-proc just-wait-proc &optional read-kbd)
  "process.c wait_reading_process_output, with no redisplay and no
WAIT_FOR_CELL.  LIMIT is a positive number of seconds, `minimum' or
`forever', or `idle': one round, which reads what is ready once and
returns, as read_char's call returns when keyboard input or a timer is
due, so its caller checks those between reads.  JUST-WAIT-PROC is 0, 1
\(read only WAIT-PROC) or -1 (and run no timers).  READ-KBD non-nil is
GNU's READ_KBD -1 without a terminal: the wait ends once special
events (file notification's among them) or requeued input wait for
read_char, as it does on detect_input_pending, or once a timer ran, so the
reader sees what the timer changed.  Return GNU's got_some_output."
  (let* ((wait (cond ((memq limit '(minimum forever idle)) limit) (t 'timeout)))
         (end-time (if (eq wait 'timeout) (+ (float-time) limit)))
         (got-some-output -1)
         (prev-nbytes (if wait-proc (romacs--proc-get wait-proc 'nbytes-read) 0))
         (got-output-end-time nil)
         (timer-delay nil)
         (timers-run romacs--timers-run)
         (done nil))
    (while (not done)
      (let ((timeout nil))
        (if (eq wait 'timeout)
            (let ((now (float-time)))
              (if (<= end-time now) (setq done t) (setq timeout (- end-time now))))
          (setq timeout (if (memq wait '(minimum idle)) 0 100000)))
        (if (and (not done) (>= just-wait-proc 0))
            (setq timer-delay (romacs--timer-check)))
        ;; A timer may have queued input or ended the last watch: the
        ;; reader checks again (keyboard.el `romacs--read-batch-wait').
        (if (and read-kbd (or unread-command-events romacs--special-events
                              (/= timers-run romacs--timers-run)))
            (setq done t))
        (if (and (not done) (romacs--proc-ticks-pending)
                 (not (romacs--proc-input-pending)))
            (setq got-some-output (romacs--status-notify nil wait-proc)))
        ;; Don't wait for output from a non-running process: read what it has.
        (if (and (not done) wait-proc
                 (not (eq (romacs--proc-get wait-proc 'status) 'run))
                 (not (romacs--connecting-p wait-proc)))
            (progn
              (if (romacs--proc-get wait-proc 'infd)
                  (let ((more t))
                    (while more
                      (let ((nread (romacs--read-process-output wait-proc)))
                        (if (< nread 0)
                            (setq more nil)
                          (if (< got-some-output nread) (setq got-some-output nread))
                          (if (= nread 0) (setq more nil)))))))
              (setq done t)))
        (if (not done)
            (progn
              (if (and (> got-some-output 0) (> timeout 0.01))
                  (setq timeout 0.01))
              (if (and (>= just-wait-proc 0) timer-delay (< timer-delay timeout))
                  (let ((timeout-abs (+ (float-time) timeout)))
                    (if (or (null got-output-end-time) (< timeout-abs got-output-end-time))
                        (setq got-output-end-time timeout-abs))
                    (setq timeout timer-delay))
                (setq got-output-end-time nil))
              (let ((ready (romacs--proc-select wait-proc (/= just-wait-proc 0) timeout read-kbd)))
                (cond
                 ((memq ready '(terminated input)) (setq done t))
                 ((null ready)
                  (if (romacs--proc-ticks-pending)
                      (setq got-some-output (romacs--status-notify nil wait-proc)))
                  (if (or (memq wait '(minimum idle))
                          (and wait-proc
                               (/= (romacs--proc-get wait-proc 'nbytes-read) prev-nbytes)))
                      (setq done t)
                    (let ((cmp (if (eq wait 'timeout) end-time nil)))
                      (if (and (> got-some-output 0) (> timeout 0))
                          (if (null got-output-end-time)
                              (setq done t)
                            (if (or (null cmp) (< got-output-end-time cmp))
                                (setq cmp got-output-end-time))))
                      (if (and (not done) cmp (<= cmp (float-time)))
                          (setq done t)))))
                 ((eq ready t) nil)
                 ((eq (car-safe ready) 'connect)
                  (apply #'romacs--connect-finished (cdr ready)))
                 ((eq (car-safe ready) 'tls)
                  (apply #'romacs--tls-finished (cdr ready)))
                 ((eq (romacs--proc-get ready 'status) 'listen)
                  (romacs--server-accept-connection ready))
                 (t
                  (let ((nread (romacs--read-process-output ready t)))
                    (if (and (or (null wait-proc) (eq wait-proc ready))
                             (< got-some-output nread))
                        (setq got-some-output nread))
                    (if (and (> nread 0) (eq wait-proc ready))
                        (setq wait 'minimum)))))
                (if (eq wait 'idle) (setq done t)))))))
    (if (and wait-proc (/= (romacs--proc-get wait-proc 'nbytes-read) prev-nbytes))
        (setq got-some-output (- (romacs--proc-get wait-proc 'nbytes-read) prev-nbytes)))
    got-some-output))

(defun romacs--process-idle ()
  "What keyboard.c read_char's wait_reading_process_output does for the
processes while the command loop idles for a key: one round that reads
the output, finishes the connections and reports the status changes that
are ready now, without waiting or running timers, so that a process whose
output keeps arriving leaves the keys, timers and redisplay their turn
between rounds.  Non-nil if anything was ready.  With no process there is
nothing to read, and no wait."
  (when (process-list)
    (let ((activity (romacs--proc-activity)))
      (romacs--wait-reading-process-output 'idle nil -1)
      (/= activity (romacs--proc-activity)))))

(defun accept-process-output (&optional process seconds millisec just-this-one)
  "Allow any pending output from subprocesses to be read by Emacs.
GNU process.c Faccept_process_output."
  (if process
      (or (processp process) (signal 'wrong-type-argument (list 'processp process)))
    (setq just-this-one nil))
  (if millisec
      (progn
        (or (integerp millisec) (signal 'wrong-type-argument (list 'fixnump millisec)))
        (if (null seconds)
            (setq seconds (/ millisec 1000.0))
          (or (integerp seconds) (signal 'wrong-type-argument (list 'fixnump seconds)))
          (setq seconds (+ (/ millisec 1000.0) seconds)))))
  (let ((limit 'minimum))
    (cond ((null seconds) (if process (setq limit 'forever)))
          ((integerp seconds) (if (> seconds 0) (setq limit seconds)))
          ((floatp seconds) (if (> seconds 0) (setq limit seconds)))
          (t (signal 'wrong-type-argument (list 'numberp seconds))))
    (if (> (romacs--wait-reading-process-output
            limit process (cond ((null just-this-one) 0)
                                ((integerp just-this-one) -1)
                                (t 1)))
           0)
        t
      nil)))

;;;; Synchronous processes

(defun romacs--signal-text (signal)
  "callproc.c: strsignal (SIGNAL), decoded by `locale-coding-system'."
  (romacs--signal-description signal))

(defun call-process (program &optional infile destination display &rest args)
  "Call PROGRAM synchronously in separate process.
GNU callproc.c Fcall_process."
  (let ((infile (if infile
                    (let ((name (expand-file-name infile (romacs--current-directory))))
                      (or (stringp name) (signal 'wrong-type-argument (list 'stringp name)))
                      name)
                  "/dev/null")))
    (setq infile (romacs--remove-slash-colon infile))
    (if (not (file-readable-p infile))
        ;; emacs_open failing: report_file_error ("Opening process input file").
        (signal (if (file-exists-p infile) 'permission-denied 'file-missing)
                (list "Opening process input file"
                      (if (file-exists-p infile) "Permission denied" "No such file or directory")
                      infile)))
    (romacs--call-process program (romacs--encode-file infile) destination display args
                          (cons program (cons infile (cons destination (cons display args)))))))

(defun romacs--call-process (program infile destination display args all-args)
  "callproc.c call_process: PROGRAM with ARGS reading INFILE.  ALL-ARGS is
what `find-operation-coding-system' sees after the operation."
  (let ((coding-systems t)
        (error-file t)
        (output-file nil)
        (buffer destination)
        (argument-coding nil)
        current-dir path discard)
    (or (stringp program) (signal 'wrong-type-argument (list 'stringp program)))
    ;; Decide the coding system for giving arguments.
    (if args
        (let ((must-encode nil) (val nil))
          (dolist (arg args) (or (stringp arg) (signal 'wrong-type-argument (list 'stringp arg))))
          (dolist (arg args) (if (multibyte-string-p arg) (setq must-encode t)))
          (setq val (cond (coding-system-for-write coding-system-for-write)
                          ((not must-encode) 'raw-text)
                          (t (setq coding-systems
                                   (apply #'find-operation-coding-system 'call-process all-args))
                             (if (consp coding-systems) (cdr coding-systems) nil))))
          (setq val (romacs--complement-process-encoding-system val))
          (check-coding-system val)
          (if (not (plist-get (coding-system-plist val) :ascii-compatible-p))
              (setq val (coding-system-change-text-conversion val 'raw-text)))
          (setq argument-coding val)))
    (if (and (consp buffer) (not (eq (car buffer) :file)))
        (progn
          (if (consp (cdr buffer))
              (let ((stderr-file (car (cdr buffer))))
                (setq error-file (if (or (null stderr-file) (eq stderr-file t))
                                     stderr-file
                                   ;; Fexpand_file_name (STDERR-FILE, Qnil).
                                   (expand-file-name stderr-file default-directory)))))
          (setq buffer (car buffer))))
    (if (and (consp buffer) (eq (car buffer) :file))
        (let ((ofile (cdr buffer)))
          (if (consp ofile) (setq ofile (car ofile)))
          (or (stringp ofile) (signal 'wrong-type-argument (list 'stringp ofile)))
          (setq output-file (expand-file-name ofile default-directory))
          (setq buffer nil)))
    (if (not (or (null buffer) (eq buffer t) (integerp buffer)))
        (let ((spec buffer))
          (setq buffer (get-buffer-create buffer))
          (if (null buffer) (signal 'wrong-type-argument (list 'bufferp spec)))))
    (setq current-dir (romacs--current-directory))
    (if (stringp error-file) (setq error-file (romacs--encode-file (romacs--remove-slash-colon error-file))))
    (if (stringp output-file) (setq output-file (romacs--encode-file (romacs--remove-slash-colon output-file))))
    (setq path (locate-file-internal program exec-path (default-value 'exec-suffixes) 1))
    (if (null path)
        (signal 'file-missing (list "Searching for program" "No such file or directory" program)))
    (setq path (romacs--encode-file (romacs--remove-slash-colon path)))
    (let ((argv (list path)))
      (dolist (arg args)
        (setq argv (cons (if (and argument-coding (multibyte-string-p arg))
                             (encode-coding-string arg argument-coding t)
                           arg)
                         argv)))
      (setq argv (nreverse argv))
      (setq discard (or (integerp buffer) (and (null buffer) (null output-file))))
      (let* ((output (cond (discard nil) ((stringp output-file) output-file) (t 'pipe)))
             (stderr (cond ((stringp error-file) error-file)
                           ((and (null error-file) (not discard)) nil)
                           (t t)))
             (result (romacs--call-process-run argv (romacs--environment-block current-dir)
                                               (romacs--encode-file current-dir) infile
                                               output stderr (not (integerp buffer)))))
        (if (integerp buffer)
            nil
          (save-current-buffer
            (if (bufferp buffer) (set-buffer buffer))
            (if (eq output 'pipe)
                (let ((val (cond (coding-system-for-read coding-system-for-read)
                                 (t (if (eq coding-systems t)
                                        (setq coding-systems
                                              (apply #'find-operation-coding-system 'call-process all-args)))
                                    (cond ((consp coding-systems) (car coding-systems))
                                          ((consp default-process-coding-system)
                                           (car default-process-coding-system)))))))
                  (check-coding-system val)
                  (if (and (not enable-multibyte-characters) val)
                      (setq val (coding-system-change-text-conversion val 'raw-text)))
                  (romacs--call-process-insert (cdr result) val)
                  (if inherit-process-coding-system
                      (funcall 'after-insert-file-set-buffer-file-coding-system
                               (string-bytes (cdr result)))))))
          (let ((status (romacs--wait-status (car result))))
            (if (eq (car status) 'signal)
                (romacs--signal-text (cdr status))
              (cdr status))))))))

(defun romacs--call-process-insert (bytes coding)
  "call_process's decoding insertion of BYTES before point with CODING."
  ;; Vlast_coding_system_used: the coding system decoding settled on.
  (setq last-coding-system-used (or coding 'no-conversion))
  (if (> (length bytes) 0)
      (insert (if (and (not enable-multibyte-characters) (null coding))
                  bytes
                (romacs--decode-output bytes coding)))))

(defun call-process-region (start end program &optional delete buffer display &rest args)
  "Send text from START to END to a synchronous process running PROGRAM.
GNU callproc.c Fcall_process_region."
  (let ((empty-input nil) (infile nil) (tempfile nil) (all-args nil))
    (cond ((stringp start) (setq empty-input (= (length start) 0)))
          ((null start) (setq empty-input (= (point-min) (point-max))))
          (t (let ((s (min start end)) (e (max start end)))
               (if (or (< s (point-min)) (> e (point-max)))
                   (signal 'args-out-of-range (list start end)))
               (setq start s end e empty-input (= s e)))))
    (setq all-args (cons start (cons end (cons program (cons delete (cons buffer (cons display args)))))))
    (unwind-protect
        (progn
          (if (not empty-input)
              (let ((coding (cond (coding-system-for-write coding-system-for-write)
                                  ((not enable-multibyte-characters) 'raw-text)
                                  (t (let ((cs (apply #'find-operation-coding-system
                                                      'call-process-region all-args)))
                                       (if (consp cs) (cdr cs) nil))))))
                (setq coding (romacs--complement-process-encoding-system coding))
                ;; create_temp_file: temp-file-name-pattern in the temporary directory.
                (setq tempfile (make-temp-file-internal
                                (expand-file-name "emacs" (if (stringp temporary-file-directory)
                                                              temporary-file-directory
                                                            (or (getenv "TMPDIR") "/tmp/")))
                                nil "" nil))
                (let ((coding-system-for-write coding)
                      (file-name-handler-alist nil))
                  (write-region start end tempfile nil 'silent))
                (setq infile tempfile)))
          (if delete
              (if (null start)
                  (progn (widen) (delete-region (point-min) (point-max)))
                (delete-region start end)))
          (romacs--call-process program (romacs--encode-file (or infile "/dev/null"))
                                buffer display args
                                (cons program (cons infile (cons buffer (cons display args))))))
      (if tempfile (delete-file tempfile)))))

;;;; Features

;; syms_of_process (process.c:9083-9111): `make-network-process' with the
;; subfeatures this implementation supports, in GNU's order: TCP servers,
;; `:service t', IPv6, IPv4 and local sockets, and `:nowait' connections.
;; Not datagram or seqpacket types or socket options (VM2-PROC1).
(provide 'make-network-process '((:server t) (:service t) (:family ipv6) (:family ipv4) (:family local) (:nowait t)))

;;; subprocess.el ends here
