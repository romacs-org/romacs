;;; romacs-agent-header-tests.el --- A chat hands display values -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat's header is a string its session's setters refresh, and its
;; composer's overlay is written only where it changes: display is handed
;; values, and a value that reads the same is not handed again.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-ui))

(defconst romacs-agent-header-tests--id "11111111-2222-4333-8444-555555555555")

(defmacro romacs-agent-header-tests--with (&rest body)
  "BODY in the chat of `session', registered and live, its header made;
`written' counts what is written to display from then on: the header's
string, an overlay."
  (declare (indent 0))
  `(let* ((romacs-agent-glyphs nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
          (romacs-agent-ui--children-timer nil)
          (session (romacs-agent--make :id romacs-agent-header-tests--id :harness 'claude :provider "claude" :state 'idle :caught-up t
                                       :model "claude-opus-5-5" :permission-mode "bypassPermissions"
                                       :options (list :effort "high")
                                       :context-used 440000 :context-window 1000000
                                       :tools (make-hash-table :test #'equal)
                                       :streamed (make-hash-table :test #'equal)))
          ;; No temporary buffer: one of those runs no `kill-buffer-hook'.
          (chat (generate-new-buffer "*romacs-agent-header-tests*"))
          (written 0))
     (puthash romacs-agent-header-tests--id session romacs-agent--sessions)
     (cl-letf* ((put (symbol-function 'overlay-put)) (move (symbol-function 'move-overlay))
                (update (symbol-function 'force-mode-line-update))
                ((symbol-function 'overlay-put) (lambda (&rest args) (cl-incf written) (apply put args)))
                ((symbol-function 'move-overlay) (lambda (&rest args) (cl-incf written) (apply move args)))
                ((symbol-function 'force-mode-line-update)
                 (lambda (&rest args) (cl-incf written) (apply update args)))
                ((symbol-function 'romacs-agent--manifest-put) #'ignore)
                ((symbol-function 'romacs-agent--note) #'ignore)
                ((symbol-function 'romacs-agent-ui--dashboard-changed) #'ignore))
       (unwind-protect
           (with-current-buffer chat
             (romacs-agent-chat-mode)
             (setq romacs-agent-ui--session session
                   romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
                   romacs-agent-ui--tasks (make-hash-table :test #'equal)
                   romacs-agent-ui--messages (make-hash-table :test #'equal)
                   romacs-agent-ui--end (point-marker)
                   romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil))
             (setf (romacs-agent-buffer session) chat)
             (romacs-agent-ui--header-refresh session)
             (romacs-agent-ui--composer-update)
             (setq written 0)
             ,@body)
         (when (buffer-live-p chat) (kill-buffer chat))))))

;; What runs before a redisplay: a batch session has no host to run it.
(unless (fboundp 'romacs--run-window-change-functions)
  (load (expand-file-name "vm/window-change.el" (file-name-directory (locate-library "romacs-agent.el"))) nil t))

(defun romacs-agent-header-tests--header ()
  "The chat's header as its buffer hands it to display."
  (should (stringp header-line-format))
  (substring-no-properties header-line-format))

(ert-deftest romacs-agent-header-is-a-string ()
  ;; No form: showing the header runs no Lisp, and its `%' is said literally.
  (romacs-agent-header-tests--with
    (should (equal (romacs-agent-header-tests--header)
                   (concat (romacs-agent-ui--harness-icon 'claude) "  opus-5-5 high  44%%")))
    ;; Its faces are its own text properties.
    (should (eq (get-text-property (string-search "44" header-line-format) 'face header-line-format)
                'romacs-agent-context))
    (should (eq (get-text-property (string-search "opus" header-line-format) 'face header-line-format)
                'romacs-agent-model))))

(ert-deftest romacs-agent-header-follows-each-event ()
  (romacs-agent-header-tests--with
    (cl-flet ((raise (&rest event) (romacs-agent--raise session event))
              (shows (text) (string-search text (romacs-agent-header-tests--header))))
      ;; Its model, and that it runs.
      (raise :type 'session :model "claude-fable-5-2")
      (should (shows "  fable-5-2 high  "))
      (should (shows "  Running"))
      ;; How full its context is.
      (raise :type 'usage :context-used 500000)
      (should (shows "  50%%"))
      ;; Its permission mode.
      (raise :type 'mode :permission-mode "acceptEdits")
      (should (shows "  acceptEdits  "))
      ;; Its state: waiting for the user, then a turn's end, failed.
      (raise :type 'approval :request-id "r1" :tool "Bash" :user t)
      (should (shows "  Approval"))
      (raise :type 'answered :request-id "r1" :behavior "allow")
      (should (shows "  Running"))
      (raise :type 'done :text "over" :error t)
      (should (shows "  Failed"))
      (raise :type 'input :text "again" :time 10.0)
      (should (shows "  Running"))
      (raise :type 'done :text "over")
      (should (string-suffix-p "50%%" (romacs-agent-header-tests--header)))
      ;; The model a selection put in effect.
      (raise :type 'selection :from '(:model "claude-fable-5-2") :to '(:model "claude-sonnet-5-5"))
      (should (shows "  sonnet-5-5 high")))))

(ert-deftest romacs-agent-header-follows-what-no-event-says ()
  (romacs-agent-header-tests--with
    (cl-flet ((shows (text) (string-search text (romacs-agent-header-tests--header))))
      ;; Its options: an effort, plan mode and back.
      (romacs-agent--set-options session '(:effort "max"))
      (should (shows "  opus-5-5 max  "))
      (romacs-agent-set-plan session t)
      (should (shows "  plan  "))
      (romacs-agent-set-plan session nil)
      (should-not (shows "plan"))
      ;; A selection waiting for its turn's end, then in effect.
      (romacs-agent--raise session (list :type 'input :text "go" :time 1.0))
      (romacs-agent-select session "claude-sonnet-5-5" "low")
      (should (shows "  → claude-sonnet-5-5 · low  "))
      (romacs-agent--raise session (list :type 'done :text "over"))
      (romacs-agent--settle session)
      (should-not (shows "→"))
      (should (shows " low  "))
      ;; A transcript's facts, where the session has none of its own.
      (setf (romacs-agent-context-used session) nil)
      (require 'romacs-agent-history)
      (romacs-agent-history--keep-facts session '(:context-used 900000))
      (should (shows "  90%%")))))

(ert-deftest romacs-agent-header-follows-the-harness-coming-and-going ()
  ;; A session's state set where no event says so: a turn launched, the
  ;; startup over, the connection gone, a resume, a restart for a selection.
  (romacs-agent-header-tests--with
    (let ((dir (make-temp-file "romacs-header-" t)) (live nil))
      (setf (romacs-agent-dir session) dir
            ;; No such socket, and a name short enough to be one.
            (romacs-agent-host-socket session) "/nonexistent/romacs-agent.sock")
      ;; A supervisor ran it once: its journal is there.
      (write-region "" nil (expand-file-name "journal.jsonl" dir) nil 'silent)
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent--check-config-dir) #'ignore)
                    ((symbol-function 'romacs-agent--check-companions) #'ignore)
                    ((symbol-function 'romacs-agent--launch) #'ignore)
                    ((symbol-function 'romacs-agent--refork) #'ignore)
                    ((symbol-function 'romacs-agent--drop-relays) #'ignore)
                    ((symbol-function 'romacs-agent--caught-up-p) #'ignore)
                    ((symbol-function 'romacs-agent--stop-then) (lambda (_session then) (funcall then)))
                    ((symbol-function 'process-live-p) (lambda (_) live)))
            (cl-flet ((shows (text) (string-search text (romacs-agent-header-tests--header))))
              ;; A turn launched: starting.
              (should-not (shows "Running"))
              (cl-letf (((symbol-function 'romacs-agent--attach) #'ignore))
                (romacs-agent--turn session "go"))
              (should (shows "  Running"))
              ;; No supervisor to attach to: detached.
              (romacs-agent--attach session 0)
              (should (shows "  Detached"))
              ;; Resumed: starting.
              (cl-letf (((symbol-function 'romacs-agent--attach) #'ignore))
                (romacs-agent-resume session))
              (should (shows "  Running"))
              ;; Its startup over with no turn to take up: idle.
              (setq live t)
              (cl-letf (((symbol-function 'romacs-agent--harness-get)
                         (lambda (_session key) (and (eq key :next) (lambda (_) :ready)))))
                (romacs-agent--step session))
              (should-not (shows "Running"))
              ;; Its connection gone.
              (setq live nil)
              (let ((process (make-pipe-process :name "romacs-agent-header-tests" :noquery t)))
                (delete-process process)
                (process-put process 'romacs-agent session)
                (setf (romacs-agent-connection session) process)
                (romacs-agent--sentinel process "finished\n"))
              (should (shows "  Detached"))
              ;; Stopped here, its supervisor not answering: ended, which
              ;; no event says.
              (write-region "" nil (expand-file-name "stopped" dir) nil 'silent)
              (romacs-agent--attach session 0)
              (should (shows "  Ended"))
              (delete-file (expand-file-name "stopped" dir))
              ;; No journal: its supervisor never started.
              (delete-file (expand-file-name "journal.jsonl" dir))
              (romacs-agent--attach session 0)
              (should (shows "  Never started"))
              ;; A selection its harness is restarted for: waiting, then in
              ;; effect and the harness resuming, then nothing waiting.
              (setf (romacs-agent-pending-selection session)
                    (list :state 'armed :model "claude-sonnet-5-5" :effort "low"))
              (cl-letf (((symbol-function 'romacs-agent-resume) #'ignore))
                (romacs-agent--restart session))
              (should (shows " low  → harness's own"))
              (romacs-agent--settle session)
              (should-not (shows "→"))))
        (delete-directory dir t)))))

(ert-deftest romacs-agent-header-follows-its-workers-and-glyphs ()
  (romacs-agent-header-tests--with
    (let ((child (romacs-agent--make :id "c" :harness 'codex :provider "codex" :title "task" :state 'running
                                     :options (list :parent romacs-agent-header-tests--id))))
      (puthash "c" child romacs-agent--sessions)
      (romacs-agent--index child)
      ;; Its workers' line, made at the end of a batch of their events.
      (romacs-agent-ui--children-changed session)
      (should (timerp romacs-agent-ui--children-timer))
      (cancel-timer romacs-agent-ui--children-timer)
      (romacs-agent-ui--children-flush)
      (should (string-search "  1 running  " (romacs-agent-header-tests--header)))
      ;; Glyphs turned on: the line and the state are said by them.
      (setf (romacs-agent-state session) 'running)
      (cl-letf (((symbol-function 'char-displayable-p) (lambda (_) t))
                ((symbol-function 'romacs-agent-sessions) (lambda () (list session))))
        (unwind-protect
            (progn (customize-set-variable 'romacs-agent-glyphs t)
                   (should (string-suffix-p "  🤖  opus-5-5 high  🤖1  44%%" (romacs-agent-header-tests--header))))
          (customize-set-variable 'romacs-agent-glyphs nil))
        (should (string-suffix-p "  Running  opus-5-5 high  1 running  44%%" (romacs-agent-header-tests--header))))
      ;; A font set that cannot draw the harness's icon: its name.
      (cl-letf (((symbol-function 'char-displayable-p) #'ignore)
                ((symbol-function 'romacs-agent-sessions) (lambda () (list session))))
        (run-hooks 'after-setting-font-hook)
        (should (string-prefix-p "claude  " (romacs-agent-header-tests--header)))))))

(ert-deftest romacs-agent-header-unchanged-is-not-written ()
  (romacs-agent-header-tests--with
    (let ((shown header-line-format))
      ;; Events that change nothing the header reads: a streamed delta, a
      ;; token count inside the same percent, a selection's bookkeeping.
      (romacs-agent--raise session (list :type 'usage :context-used 441000))
      (romacs-agent--raise session (list :type 'status :text "requesting"))
      (romacs-agent--changed session)
      (romacs-agent-ui--header-refresh session)
      (should (eq header-line-format shown))
      (should (= written 0))
      ;; Made again, it reads the same: not written.
      (setq romacs-agent-ui--header nil)
      (romacs-agent-ui--header-refresh session)
      (should (eq header-line-format shown))
      (should (= written 0))
      ;; A copy a journal is replayed into is not the chat's session.
      (let ((scratch (romacs-agent--scratch session)))
        (setf (romacs-agent-state scratch) 'running)
        (romacs-agent--changed scratch)
        (should (eq header-line-format shown)))
      ;; A change is written once.
      (romacs-agent--raise session (list :type 'usage :context-used 600000))
      (should-not (eq header-line-format shown))
      (should (= written 1)))))

(ert-deftest romacs-agent-header-loaded-over-a-running-session ()
  ;; Chats an older version made hold its form, which names functions this
  ;; version has changed; a transcript's view's session is no recorded
  ;; one.  Loading this file makes their headers strings, before any event.
  (romacs-agent-header-tests--with
    (let ((view (generate-new-buffer "*romacs-agent-header-tests-view*"))
          (older '(:eval (romacs-agent-ui--header-line))))
      (unwind-protect
          (progn
            (with-current-buffer view
              (romacs-agent-chat-mode)
              (setq romacs-agent-ui--session (romacs-agent--make :id "view" :harness 'claude :provider "claude"
                                                                :state 'history)))
            (dolist (buffer (list chat view))
              (with-current-buffer buffer (setq header-line-format older romacs-agent-ui--header nil)))
            (load (locate-library "romacs-agent-ui.el") nil t t)
            (should (equal (romacs-agent-header-tests--header)
                           (concat (romacs-agent-ui--harness-icon 'claude) "  opus-5-5 high  44%%")))
            (with-current-buffer view
              (should (string-search "  History  " (romacs-agent-header-tests--header)))))
        (kill-buffer view)))))

(ert-deftest romacs-agent-header-is-one-line-for-every-window ()
  ;; Nothing of the header is a window's: two windows of one chat, one too
  ;; narrow for it, are handed the same string, which display clips where
  ;; each ends.  So what matters most comes first: the state, then the model.
  (romacs-agent-header-tests--with
    (romacs-agent--raise session (list :type 'input :text "go" :time 1.0))
    (romacs-agent--raise session (list :type 'mode :permission-mode "acceptEdits"))
    (should (equal (romacs-agent-header-tests--header)
                   (concat (romacs-agent-ui--harness-icon 'claude) "  Running  opus-5-5 high  acceptEdits  44%%")))
    (save-window-excursion
      (delete-other-windows)
      (switch-to-buffer chat)
      (let* ((wide (selected-window))
             (narrow (split-window wide (- (window-total-width wide) 14) t)))
        (should (< (window-body-width narrow) (string-width header-line-format) (window-body-width wide)))
        ;; The header changes; then what runs before a redisplay runs, in
        ;; the host's order (prepare_menu_bars, then the window change
        ;; functions, which see the split): nothing writes to a window.
        (romacs-agent--raise session (list :type 'mode :permission-mode "default"))
        (redisplay--pre-redisplay-functions nil)
        (romacs--run-window-change-functions)
        (dolist (window (list wide narrow))
          (should-not (seq-some (lambda (parameter) (stringp (cdr parameter))) (window-parameters window)))
          (should (eq (buffer-local-value 'header-line-format (window-buffer window)) header-line-format)))))))

(ert-deftest romacs-agent-header-of-a-transcript-follows-glyphs ()
  ;; A transcript's view has a session no one records: its header is made
  ;; again with the others' when what draws a glyph changes.
  (require 'romacs-agent-history)
  (let ((romacs-agent-glyphs nil)
        (romacs-agent--sessions (make-hash-table :test #'equal))
        (file (make-temp-file "romacs-header-" nil ".jsonl")) (buffer nil))
    (unwind-protect
        (save-window-excursion
          (setq buffer (romacs-agent-history-open
                        (list :path file :title "h" :harness 'claude :provider "claude" :id "h1" :cwd "/")))
          (with-current-buffer buffer
            (should (string-search "  History" (romacs-agent-header-tests--header)))
            (cl-letf (((symbol-function 'char-displayable-p) (lambda (_) t)))
              (unwind-protect
                  (progn (customize-set-variable 'romacs-agent-glyphs t)
                         (should (string-search "  📜" (romacs-agent-header-tests--header))))
                (customize-set-variable 'romacs-agent-glyphs nil)))
            (should (string-search "  History" (romacs-agent-header-tests--header)))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-file file))))

(ert-deftest romacs-agent-composer-writes-what-changed ()
  ;; The boundary and the hint are put on the composer's overlay when they
  ;; read differently, so a tick, a command or a streamed delta that leaves
  ;; them as they were changes nothing display reads.
  (romacs-agent-header-tests--with
    (let ((now 100.0))
      (cl-letf (((symbol-function 'float-time) (lambda (&rest _) now))
                ((symbol-function 'romacs-agent-ui--tick-start) #'ignore))
        (save-window-excursion
          (switch-to-buffer (current-buffer))
          (romacs-agent--raise session (list :type 'input :text "go" :time 100.0 :seq 1 :last t))
          (should (string-search "● Running 0s" (overlay-get romacs-agent-ui--composer 'before-string)))
          (should (equal (overlay-get romacs-agent-ui--composer 'after-string) (romacs-agent-ui--hint)))
          ;; The same second: a tick and a command write nothing.
          (setq written 0)
          (romacs-agent-ui--tick)
          (romacs-agent-ui--composer-update)
          (should (= written 0))
          ;; The clock's second moved: the boundary alone is put.
          (setq now 101.0)
          (romacs-agent-ui--tick)
          (should (= written 1))
          (should (string-search "● Running 1s" (overlay-get romacs-agent-ui--composer 'before-string)))
          ;; A reply streams: its first delta opens a line (the boundary
          ;; gains the newline that ends it), the rest write nothing, and
          ;; the overlay follows the transcript's end with no one moving it.
          (romacs-agent--raise session (list :type 'delta :text "some words " :message "m" :seq 2 :last t))
          (setq written 0)
          (dotimes (i 5)
            (romacs-agent--raise session (list :type 'delta :text "more words " :message "m" :seq (+ 3 i) :last t)))
          (should (= written 0))
          (should (= (overlay-start romacs-agent-ui--composer) (marker-position romacs-agent-ui--end)))
          (should (string-suffix-p "more words " (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
          ;; Text typed in the composer: the hint goes, once.
          (goto-char (point-max))
          (insert "a")
          (romacs-agent-ui--composer-update)
          (should (= written 1))
          (should-not (overlay-get romacs-agent-ui--composer 'after-string))
          (insert "b")
          (romacs-agent-ui--composer-update)
          (should (= written 1)))))))

(provide 'romacs-agent-header-tests)
;;; romacs-agent-header-tests.el ends here
