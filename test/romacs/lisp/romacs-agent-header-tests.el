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

(defmacro romacs-agent-header-tests--with (&rest body)
  "BODY in the chat of `session', registered and live, its header made;
`written' counts what is written to display from then on: the header's
string, a window's cut of it, an overlay."
  (declare (indent 0))
  `(let* ((romacs-agent-glyphs nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
          (romacs-agent-ui--children-timer nil)
          (session (romacs-agent--make :id "h" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                       :model "claude-opus-5-5" :permission-mode "bypassPermissions"
                                       :options (list :effort "high")
                                       :context-used 440000 :context-window 1000000
                                       :tools (make-hash-table :test #'equal)
                                       :streamed (make-hash-table :test #'equal)))
          ;; No temporary buffer: one of those runs no `kill-buffer-hook'.
          (chat (generate-new-buffer "*romacs-agent-header-tests*"))
          (written 0))
     (puthash "h" session romacs-agent--sessions)
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

;; The host runs window.c's change functions before it redisplays; a batch
;; session has no host, so the test runs them itself.
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
      (should (shows "  50%%  "))
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
      (setf (romacs-agent-state session) 'running)
      (romacs-agent--changed session)
      (should (shows "  Running"))
      (romacs-agent-select session "claude-sonnet-5-5" "low")
      (should (shows "  → claude-sonnet-5-5 · low  "))
      (setf (romacs-agent-state session) 'idle)
      (romacs-agent--settle session)
      (should-not (shows "→"))
      (should (shows " low  "))
      ;; A transcript's facts, where the session has none of its own.
      (setf (romacs-agent-context-used session) nil)
      (require 'romacs-agent-history)
      (romacs-agent-history--keep-facts session '(:context-used 900000))
      (should (shows "  90%%"))
      ;; Its connection gone.
      (setf (romacs-agent-state session) 'detached)
      (romacs-agent--changed session)
      (should (shows "  Detached")))))

(ert-deftest romacs-agent-header-follows-its-workers-and-glyphs ()
  (romacs-agent-header-tests--with
    (let ((child (romacs-agent--make :id "c" :harness 'codex :provider "codex" :title "task" :state 'running
                                     :options '(:parent "h"))))
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
                   (should (string-suffix-p "  🤖1  44%%  🤖" (romacs-agent-header-tests--header))))
          (customize-set-variable 'romacs-agent-glyphs nil))
        (should (string-suffix-p "  1 running  44%%  Running" (romacs-agent-header-tests--header))))
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
      ;; What it reads changed and reads the same: made, not written.
      (setf (romacs-agent-context-used session) 440000)
      (setcar romacs-agent-ui--header nil)
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

(ert-deftest romacs-agent-header-is-cut-at-each-window ()
  ;; Two windows of one chat, one too narrow for its header: each shows it
  ;; cut at its own width, and the cut follows the header, the window's
  ;; width and its buffer.
  (romacs-agent-header-tests--with
    (save-window-excursion
      (delete-other-windows)
      (switch-to-buffer (current-buffer))
      (let* ((wide (selected-window))
             (narrow (split-window wide (- (window-total-width wide) 14) t))
             (whole (romacs-agent-header-tests--header)))
        (cl-flet ((cut (window) (window-parameter window 'header-line-format))
                  (changed () (romacs--run-window-change-functions)))
          (should (< (window-body-width narrow) (string-width whole) (window-body-width wide)))
          (changed)
          ;; The wide one shows the buffer's; the narrow one its own cut.
          (should-not (cut wide))
          (should (equal (substring-no-properties (cut narrow))
                         (romacs-truncate whole (window-body-width narrow))))
          (should (string-suffix-p (truncate-string-ellipsis) (cut narrow)))
          (should (eq (get-text-property 0 'face (cut narrow)) (get-text-property 0 'face header-line-format)))
          ;; The header changes: the cut is of the new one.
          (romacs-agent--raise session (list :type 'mode :permission-mode "acceptEdits"))
          (should-not (cut wide))
          (should (equal (substring-no-properties (cut narrow))
                         (romacs-truncate (romacs-agent-header-tests--header) (window-body-width narrow))))
          ;; A `%' cut into view is still said literally.
          (setf (romacs-agent-provider session) "100%")
          (romacs-agent--changed session)
          (should (string-search "100%%" (cut narrow)))
          ;; Unchanged, a window is not written again.
          (let ((held (cut narrow)))
            (romacs-agent--raise session (list :type 'status :text "requesting"))
            (changed)
            (should (eq (cut narrow) held)))
          ;; The narrow one widened shows the buffer's again.
          (delete-window wide)
          (changed)
          (should-not (cut narrow))
          ;; Narrow again, then showing another buffer: no cut of the chat's is left on it.
          (setq wide (split-window narrow 14 t))
          (changed)
          (should (cut narrow))
          (set-window-buffer narrow (get-buffer-create " *romacs-agent-header-tests*"))
          (changed)
          (should-not (cut narrow))
          ;; A parameter another put there is not this chat's to take.
          (set-window-buffer narrow chat)
          (set-window-parameter narrow 'header-line-format 'none)
          (changed)
          (should (eq (cut narrow) 'none))
          (set-window-parameter narrow 'header-line-format nil)
          (set-window-parameter narrow 'romacs-agent-header nil)
          ;; The chat killed while shown cut: its window keeps nothing.
          (romacs-agent-ui--header-fit narrow)
          (should (cut narrow))
          (kill-buffer chat)
          (should-not (cut narrow)))))))

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

(defconst romacs-agent-header-tests--fields
  '(romacs-agent-state romacs-agent-failed romacs-agent-model romacs-agent-permission-mode
    romacs-agent-options romacs-agent-pending-selection romacs-agent-context-used
    romacs-agent-context-window romacs-agent-children-summary romacs-agent-harness romacs-agent-provider)
  "The fields of a session its chat's header reads.")

(defconst romacs-agent-header-tests--tellers
  '(romacs-agent--changed romacs-agent--raise romacs-agent-ui--header-refresh)
  "What tells a session's views its state changed.")

(defconst romacs-agent-header-tests--untold
  '(;; The event's own changes: its functions hear the event.
    romacs-agent--raise romacs-agent--mode-seen
    ;; A copy no view shows.
    romacs-agent--scratch)
  "The functions that write a header field and tell no one, each with its reason.")

(defun romacs-agent-header-tests--mentions (form symbols)
  "Whether FORM names one of SYMBOLS."
  (cond ((symbolp form) (memq form symbols))
        ((consp form) (or (romacs-agent-header-tests--mentions (car form) symbols)
                          (romacs-agent-header-tests--mentions (cdr form) symbols)))))

(defun romacs-agent-header-tests--sets (form)
  "Whether FORM sets a field of `romacs-agent-header-tests--fields' with `setf'."
  (and (consp form)
       (or (and (eq (car form) 'setf)
                (cl-loop for (place) on (cdr form) by #'cddr
                         thereis (memq (car-safe place) romacs-agent-header-tests--fields)))
           (romacs-agent-header-tests--sets (car form))
           (romacs-agent-header-tests--sets (cdr form)))))

(ert-deftest romacs-agent-header-field-writers-tell ()
  ;; The header is made again only when told: a function that sets a field
  ;; it reads tells the session's views, or is listed with its reason.
  (dolist (library '("romacs-agent" "romacs-agent-ui" "romacs-agent-history" "romacs-agent-claude"
                     "romacs-agent-codex" "romacs-agent-tools" "romacs-agent-rollback" "romacs-agent-handoff"))
    (with-temp-buffer
      (insert-file-contents (locate-library (concat library ".el")))
      (let (form)
        (while (setq form (ignore-errors (read (current-buffer))))
          (when (and (romacs-agent-header-tests--sets form)
                     (not (memq (car-safe (cdr-safe form)) romacs-agent-header-tests--untold)))
            (should (romacs-agent-header-tests--mentions form romacs-agent-header-tests--tellers))))))))

(provide 'romacs-agent-header-tests)
;;; romacs-agent-header-tests.el ends here
