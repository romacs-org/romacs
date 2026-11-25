;;; process-framing-tests.el --- Framed process output and unsent bytes -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `set-process-framing' over real processes and sockets: what a framed
;; filter is called with, what an accepted connection takes from its
;; server, what turning framing off gives back, and the agent protocol's
;; connection (romacs-agent-rpc.el) on it.  The framer's byte cases
;; (partial frames, limits, UTF-8 and CRLF split across reads) are its
;; Rust tests (crates/romacs-vm/src/framing_tests.rs).

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-agent-rpc)

(defun process-framing-tests--cat (filter &optional coding)
  "A `cat' whose output FILTER gets, read with CODING (default bytes)."
  (make-process :name "process-framing-cat" :command '("cat") :noquery t
                :connection-type 'pipe :coding (or coding 'no-conversion) :filter filter))

(defun process-framing-tests--until (process done)
  "Read every process's output until DONE says t, at most ten seconds:
PROCESS, a listener, has no output of its own to wait for."
  (with-timeout (10 (error "Waited too long for %s" process))
    (while (not (funcall done)) (accept-process-output nil 0.05))))

(ert-deftest process-framing-lines-a-call-a-read ()
  ;; Each call has whole lines only, in order; an over-limit line is `over'
  ;; in its place; a last line with no newline comes at end of file.
  (let* ((calls nil)
         (process (process-framing-tests--cat (lambda (_p frames) (push frames calls)))))
    (set-process-framing process 'lines 8)
    (should (equal (process-framing process) '(lines . 8)))
    (process-send-string process "one\ntwo\n0123456789\nok\nlast")
    (process-send-eof process)
    (process-framing-tests--until process (lambda () (not (process-live-p process))))
    (should (cl-every (lambda (frames) (and (consp frames) (cl-every (lambda (f) (or (stringp f) (eq f 'over))) frames)))
                      calls))
    (should (equal (apply #'append (reverse calls)) '("one" "two" over "ok" "last")))))

(ert-deftest process-framing-content-length-decodes-by-the-coding-system ()
  ;; Bodies counted in bytes, decoded as the process's coding system says.
  (let* ((frames nil)
         (body (encode-coding-string "{\"t\":\"λ😀\"}" 'utf-8))
         (process (process-framing-tests--cat (lambda (_p fs) (setq frames (append frames fs))) 'utf-8)))
    (set-process-framing process 'content-length)
    (process-send-string process (format "Content-Length: %d\r\nContent-Type: x\r\n\r\n%s" (length body) body))
    (process-send-string process "Content-Length: 2\r\n\r\n{}")
    (process-framing-tests--until process (lambda () (= (length frames) 2)))
    (should (equal frames '("{\"t\":\"λ😀\"}" "{}")))
    (should (multibyte-string-p (car frames)))
    (delete-process process)))

(ert-deftest process-framing-off-gives-back-the-partial-line ()
  ;; Framing turned off: the line it held comes first in the next text.
  (let* ((got nil)
         (process (process-framing-tests--cat (lambda (_p x) (push x got)))))
    (set-process-framing process 'lines)
    (process-send-string process "a\nhalf")
    (process-framing-tests--until process (lambda () got))
    (should (equal got '(("a"))))
    (set-process-framing process nil)
    (should-not (process-framing process))
    (process-send-string process "-rest")
    (process-framing-tests--until process (lambda () (stringp (car got))))
    (should (equal (car got) "half-rest"))
    (delete-process process)))

(ert-deftest process-framing-an-accepted-connection-takes-its-servers ()
  (let* ((socket (make-temp-name (expand-file-name "pf-" temporary-file-directory)))
         (frames nil) (accepted nil)
         (server (make-network-process :name "process-framing-server" :family 'local :service socket
                                       :server t :noquery t :coding 'no-conversion
                                       :log (lambda (_server peer _m) (setq accepted peer))
                                       :filter (lambda (_p fs) (setq frames (append frames fs)))))
         client)
    (unwind-protect
        (progn
          (set-process-framing server 'lines 1024)
          (setq client (make-network-process :name "process-framing-client" :family 'local
                                             :service socket :noquery t :coding 'no-conversion))
          (process-send-string client "x\ny\n")
          (process-framing-tests--until server (lambda () (= (length frames) 2)))
          (should (equal frames '("x" "y")))
          (should (equal (process-framing accepted) '(lines . 1024))))
      (when client (delete-process client))
      (delete-process server)
      (ignore-errors (delete-file socket)))))

(ert-deftest process-framing-unsent-bytes-drain-as-the-peer-reads ()
  ;; A `cat' whose output is not read fills its pipes: what it cannot take
  ;; is left unsent, never waited for, and written as it reads, whenever
  ;; Romacs waits.
  (let* ((got 0)
         (process (make-process :name "process-framing-slow" :command '("cat")
                                :noquery t :connection-type 'pipe :coding 'no-conversion
                                :filter (lambda (_p s) (setq got (+ got (length s))))))
         (chunk (make-string 65536 ?z)))
    (unwind-protect
        (progn
          (dotimes (_ 16) (romacs-process-send-nowait process chunk))
          (should (> (romacs-process-unsent-bytes process) 0))
          (process-framing-tests--until process (lambda () (>= got (* 16 65536))))
          (should (= (romacs-process-unsent-bytes process) 0)))
      (delete-process process))))

;; A pty ends with EIO, not end of file, and may be reopened: framing one
;; is refused, saying why; nil on it, unframed, changes nothing.
(ert-deftest process-framing-a-pty-is-refused ()
  (let ((process (make-process :name "process-framing-pty" :command '("sleep" "5")
                               :noquery t :connection-type 'pty)))
    (unwind-protect
        (progn
          (should (string-match-p "pty" (cadr (should-error (set-process-framing process 'lines)))))
          (should-not (set-process-framing process nil))
          (should-not (process-framing process)))
      (delete-process process))))

(ert-deftest process-framing-off-in-a-body-gives-back-its-header ()
  (let* ((got nil)
         (process (process-framing-tests--cat (lambda (_p x) (push x got)))))
    (set-process-framing process 'content-length)
    (process-send-string process "Content-Length: 2\r\n\r\n{}Content-Length: 5\r\n\r\nab")
    (process-framing-tests--until process (lambda () got))
    ;; The same kind again keeps the body in progress.
    (set-process-framing process 'content-length 100)
    (set-process-framing process nil)
    (process-send-string process "cde")
    (process-framing-tests--until process (lambda () (stringp (car got))))
    (should (equal (car got) "Content-Length: 5\r\n\r\nabcde"))
    (should (equal (cadr got) '("{}")))
    (delete-process process)))

(ert-deftest process-framing-a-failed-queued-write-closes-only-the-output ()
  ;; The child closes its input and goes on: what the wait could not write
  ;; is dropped and the output closed, as GNU's EPIPE does; nothing signals
  ;; in the wait, the next send does, and the child's last words still come.
  (let* ((said "")
         (process (make-process :name "process-framing-epipe"
                                :command '("sh" "-c" "head -c 1 >/dev/null; exec 0<&-; sleep 1; echo bye")
                                :noquery t :connection-type 'pipe :coding 'no-conversion
                                :filter (lambda (_p s) (setq said (concat said s))))))
    (unwind-protect
        (progn
          (romacs-process-send-nowait process (make-string (* 1024 1024) ?z))
          (process-framing-tests--until process (lambda () (= (romacs-process-unsent-bytes process) 0)))
          (should (process-live-p process))
          (should-error (romacs-process-send-nowait process "more"))
          (process-framing-tests--until process (lambda () (equal said "bye\n"))))
      (delete-process process))))

;; A last line delivered at the end of output is output: a wait for it
;; returns, though the child lives on.
(ert-deftest process-framing-a-last-line-ends-a-wait ()
  (let* ((frames nil)
         (process (make-process :name "process-framing-last" :command '("sh" "-c" "printf last; exec 1>&- 2>&-; sleep 5")
                                :noquery t :connection-type 'pipe :coding 'no-conversion
                                :filter (lambda (_p fs) (setq frames (append frames fs))))))
    (unwind-protect
        (progn
          (set-process-framing process 'lines)
          (with-timeout (3 (error "A wait for the last line never returned"))
            (while (not frames) (accept-process-output process)))
          (should (equal frames '("last")))
          (should (process-live-p process)))
      (delete-process process))))

;; Nil on a process whose channel is gone changes nothing, as on any
;; unframed one.
(ert-deftest process-framing-nil-on-a-deleted-process ()
  (let ((process (process-framing-tests--cat #'ignore)))
    (delete-process process)
    (should-not (set-process-framing process nil))
    (should-not (process-framing process))))

;; At the end, any frame delivered is output: an empty line a change of
;; kind left held ends a wait, though the child lives on.
(ert-deftest process-framing-an-empty-last-frame-ends-a-wait ()
  (let* ((frames nil)
         (process (make-process :name "process-framing-empty" :noquery t :connection-type 'pipe
                                :command '("sh" "-c" "printf '\\n'; read x; exec 1>&- 2>&-; sleep 5")
                                :coding 'no-conversion
                                :filter (lambda (_p fs) (setq frames (append frames (list fs)))))))
    (unwind-protect
        (progn
          (set-process-framing process 'content-length)
          (process-framing-tests--until process (lambda () (> (romacs--proc-get process 'nbytes-read) 0)))
          (set-process-framing process 'lines)
          (process-send-string process "go\n")
          (with-timeout (3 (error "A wait for the last frame never returned"))
            (while (not frames) (accept-process-output process)))
          (should (equal frames '(("")))))
      (delete-process process))))

;; A socket's end, read in the wait that delivers its last line, does not
;; make the wait say no output came.
(ert-deftest process-framing-a-last-line-at-a-sockets-end-is-output ()
  (let* ((socket (make-temp-name (expand-file-name "pf-end-" temporary-file-directory)))
         (peer nil) (frames nil)
         (server (make-network-process :name "process-framing-end" :family 'local :service socket
                                       :server t :noquery t :coding 'no-conversion
                                       :log (lambda (_s p _m) (setq peer p))
                                       :filter (lambda (_p fs) (setq frames (append frames fs)))))
         (client (make-network-process :name "process-framing-end-client" :family 'local
                                       :service socket :noquery t :coding 'no-conversion)))
    (unwind-protect
        (progn
          (set-process-framing server 'lines)
          (process-send-string client "abc")
          (process-framing-tests--until server (lambda () (and peer (> (romacs--proc-get peer 'nbytes-read) 0))))
          (delete-process client)
          (should (accept-process-output peer 2))
          (should (equal frames '("abc"))))
      (delete-process server)
      (ignore-errors (delete-file socket)))))

;;; The agent protocol's connection

(ert-deftest process-framing-rpc-lines-over-a-socket ()
  ;; romacs-agent-rpc's connection frames its lines natively: messages
  ;; whole, a line over the limit closes the peer.
  (let* ((socket (make-temp-name (expand-file-name "pf-rpc-" temporary-file-directory)))
         (received nil) (closed nil)
         (romacs-agent-rpc-message-limit 4096)
         (server (make-network-process
                  :name "process-framing-rpc" :family 'local :service socket :server t :noquery t
                  :coding 'no-conversion
                  :log (lambda (_server peer _m) (process-put peer 'rpc (romacs-agent-rpc-make peer)))
                  :sentinel (lambda (p _e) (unless (process-live-p p) (setq closed t)))
                  :filter (lambda (p frames)
                            (let ((lines (romacs-agent-rpc-lines (process-get p 'rpc) frames)))
                              (if (eq lines 'over)
                                  (romacs-agent-rpc-close (process-get p 'rpc))
                                (dolist (line lines)
                                  (push (gethash "n" (romacs-agent-rpc-parse line)) received)))))))
         (client (make-network-process :name "process-framing-rpc-client" :family 'local :service socket
                                       :noquery t :coding 'no-conversion :filter #'ignore))
         (rpc (romacs-agent-rpc-make client)))
    (unwind-protect
        (progn
          (dotimes (n 200) (romacs-agent-rpc-send rpc (romacs-agent-rpc-object "n" n "pad" (make-string (* n 10) ?p))))
          (process-framing-tests--until server (lambda () (= (length received) 200)))
          (should (equal (nreverse received) (number-sequence 0 199)))
          ;; Over the limit: not sent, signalled.
          (should-error (romacs-agent-rpc-send rpc (romacs-agent-rpc-object "pad" (make-string 5000 ?p))))
          ;; A line over the limit from a peer that does not check closes it.
          (process-send-string client (concat (make-string 5000 ?q) "\n"))
          (process-framing-tests--until server (lambda () closed)))
      (delete-process client)
      (delete-process server)
      (ignore-errors (delete-file socket)))))

(ert-deftest process-framing-rpc-keeps-a-peer-that-read-again ()
  ;; The queue's limit holds what is left once the socket took what it
  ;; could: a peer that has read since is not closed for what it read.
  (let* ((socket (make-temp-name (expand-file-name "pf-rpc-" temporary-file-directory)))
         (peer nil)
         (server (make-network-process :name "process-framing-rpc-reader" :family 'local :service socket
                                       :server t :noquery t :coding 'no-conversion
                                       :log (lambda (_server p _m) (setq peer p))))
         (client (make-network-process :name "process-framing-rpc-writer" :family 'local :service socket
                                       :noquery t :coding 'no-conversion :filter #'ignore))
         (rpc (romacs-agent-rpc-make client)))
    (unwind-protect
        (progn
          (process-framing-tests--until server (lambda () peer))
          ;; Filled with no wait, so nothing is written behind the sends.
          (while (= (romacs-process-unsent-bytes client) 0)
            (romacs-agent-rpc-send rpc (romacs-agent-rpc-object "pad" (make-string 16384 ?p))))
          ;; The peer reads all, outside any wait: the queue is as it was.
          (let ((unsent (romacs-process-unsent-bytes client)))
            (while (> (car (romacs--proc-read peer)) 0))
            (should (= (romacs-process-unsent-bytes client) unsent))
            (let ((romacs-agent-rpc-queue-limit (+ unsent 100)))
              (should (romacs-agent-rpc-send rpc (romacs-agent-rpc-object "pad" (make-string 1024 ?p))))
              (should (process-live-p client)))))
      (delete-process client)
      (delete-process server)
      (ignore-errors (delete-file socket)))))

(ert-deftest process-framing-rpc-closes-a-peer-that-reads-nothing ()
  ;; What a peer leaves unread is bounded: past the queue's limit the
  ;; connection is closed, the sender never waits.
  (let* ((romacs-agent-rpc-queue-limit (* 256 1024))
         (process (make-process :name "process-framing-deaf" :command '("sleep" "30") :noquery t
                                :connection-type 'pipe :coding 'no-conversion))
         (rpc (romacs-agent-rpc-make process))
         (sent 0))
    (unwind-protect
        (progn
          (while (and (romacs-agent-rpc-send rpc (romacs-agent-rpc-object "pad" (make-string 4000 ?p)))
                      (< (cl-incf sent) 1000)))
          (should (< sent 1000))
          (should-not (process-live-p process)))
      (delete-process process))))

(provide 'process-framing-tests)
;;; process-framing-tests.el ends here
