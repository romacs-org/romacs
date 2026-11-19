;;; romacs-agent-live-rows-tests.el --- What a chat's delegates do now -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Above a chat's composer, a row for each delegate that still works:
;; its title, how long its turn has run and what it does now, read from
;; the delegate's turn under way at each tick.  A chat with a delegate at
;; work ticks even while its own session is idle.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-ui))

(defconst romacs-agent-live-rows-tests--parent "11111111-2222-4333-8444-555555555555")

(defun romacs-agent-live-rows-tests--session (id state &optional parent title)
  "A session ID in STATE, delegated by PARENT when given, titled TITLE."
  (romacs-agent--make :id id :harness 'claude :provider "claude" :state state :caught-up t
                      :title title :options (and parent (list :parent parent))
                      :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))

(defmacro romacs-agent-live-rows-tests--with (children &rest body)
  "BODY with `parent', an idle session, and CHILDREN, a list of (ID STATE
TITLE START TOOL) its delegates, registered and loaded: START when its
turn began, TOOL the tool it last started."
  (declare (indent 1))
  `(let* ((romacs-agent-glyphs nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (parent (romacs-agent-live-rows-tests--session romacs-agent-live-rows-tests--parent 'idle)))
     (puthash romacs-agent-live-rows-tests--parent parent romacs-agent--sessions)
     (dolist (spec ,children)
       (pcase-let* ((`(,id ,state ,title ,start ,tool) spec)
                    (child (romacs-agent-live-rows-tests--session id state romacs-agent-live-rows-tests--parent title)))
         (setf (romacs-agent-turn-start child) start (romacs-agent-activity child) tool)
         (puthash id child romacs-agent--sessions)
         (push child (gethash romacs-agent-live-rows-tests--parent romacs-agent--children))))
     (cl-letf (((symbol-function 'romacs-agent-ui--rule-width) (lambda () 100)))
       ,@body)))

(defun romacs-agent-live-rows-tests--child (n)
  (format "2222222%d-2222-4333-8444-555555555555" n))

(ert-deftest romacs-agent-live-rows-show-the-tool-a-delegate-runs ()
  "A working delegate's row: its title, its turn's clock and the tool it runs."
  (romacs-agent-live-rows-tests--with
      (list (list (romacs-agent-live-rows-tests--child 1) 'running "lane compiler" (- (float-time) 75) "Bash"))
    (let ((rows (substring-no-properties (romacs-agent-ui--worker-rows parent))))
      (should (equal rows (format "  ● lane compiler  %s · %s\n" (romacs-agent-ui--clock 75)
                                  (substring-no-properties (romacs-agent-ui--tool-name "Bash"))))))))

(ert-deftest romacs-agent-live-rows-say-thinking-and-waiting ()
  "With no call running a delegate thinks; one asking its user needs approval."
  (romacs-agent-live-rows-tests--with
      (list (list (romacs-agent-live-rows-tests--child 1) 'running "lane motion" (float-time) nil)
            (list (romacs-agent-live-rows-tests--child 2) 'waiting-approval "lane janitor" (float-time) "Bash"))
    (let ((rows (substring-no-properties (romacs-agent-ui--worker-rows parent))))
      (should (string-match-p "● lane motion  [01]s · thinking" rows))
      (should (string-match-p "◆ lane janitor  [01]s · needs approval" rows)))))

(ert-deftest romacs-agent-live-rows-count-what-they-leave-out ()
  "Past the rows a chat shows, the rest working are counted; idle ones are not rows."
  (romacs-agent-live-rows-tests--with
      (append (mapcar (lambda (n) (list (romacs-agent-live-rows-tests--child n) 'running (format "lane %d" n)))
                      (number-sequence 1 7))
              (list (list (romacs-agent-live-rows-tests--child 8) 'idle "lane idle")))
    (let ((rows (substring-no-properties (romacs-agent-ui--worker-rows parent))))
      (should (= (cl-count ?\n rows) (1+ romacs-agent-ui--worker-rows)))
      (should (string-match-p "  \\+2 more working\n\\'" rows))
      (should-not (string-match-p "lane idle" rows)))))

(ert-deftest romacs-agent-live-rows-tick-an-idle-chat-with-workers ()
  "A chat ticks while a delegate works though its own session is idle, and
stops once none does; the boundary carries the rows."
  (romacs-agent-live-rows-tests--with
      (list (list (romacs-agent-live-rows-tests--child 1) 'running "lane compiler" (float-time)))
    (should (romacs-agent-ui--ticks-p parent))
    (with-temp-buffer
      (setq-local romacs-agent-ui--session parent)
      (setq-local romacs-agent-ui--end (point-marker))
      (should (string-match-p "lane compiler" (romacs-agent-ui--boundary))))
    (setf (romacs-agent-state (car (romacs-agent-children parent))) 'idle)
    (should-not (romacs-agent-ui--ticks-p parent))))

(ert-deftest romacs-agent-live-rows-keep-the-clock-and-state-for-a-long-title ()
  "A title too long for the row gives way: the clock and what it does stay."
  (romacs-agent-live-rows-tests--with
      (list (list (romacs-agent-live-rows-tests--child 1) 'waiting-approval (make-string 300 ?x) (float-time)))
    (let ((rows (substring-no-properties (romacs-agent-ui--worker-rows parent))))
      (should (string-match-p "  [01]s · needs approval\n\\'" rows))
      (should (<= (string-width (string-trim-right rows)) 100)))))

(ert-deftest romacs-agent-live-rows-follow-a-delegate-and-stop-ticking ()
  "A delegate's state moving reaches its parent's chat: the rows show and
the ticker starts; a tick in the same second writes nothing; once none
works, the rows go and the next tick stops the ticker."
  (romacs-agent-live-rows-tests--with
      (list (list (romacs-agent-live-rows-tests--child 1) 'running "lane compiler" (float-time) "Bash"))
    (let ((chat (generate-new-buffer "*romacs-agent-live-rows-tests*"))
          (child (car (romacs-agent-children parent)))
          (romacs-agent-ui--ticker nil)
          (written 0))
      (unwind-protect
          (cl-letf* ((put (symbol-function 'overlay-put))
                     ((symbol-function 'overlay-put) (lambda (&rest args) (cl-incf written) (apply put args)))
                     ((symbol-function 'romacs-agent-ui--summarize-children) #'ignore)
                     ((symbol-function 'romacs-agent-ui--header-refresh) #'ignore)
                     ((symbol-function 'romacs-agent-ui--announce) #'ignore))
            (with-current-buffer chat
              (romacs-agent-chat-mode)
              (setq romacs-agent-ui--session parent
                    romacs-agent-ui--end (point-marker)
                    romacs-agent-ui--composer (make-overlay (point) (point))))
            (setf (romacs-agent-buffer parent) chat)
            (set-window-buffer (selected-window) chat)
            (romacs-agent-ui--flush-rows (list parent))
            (should (timerp romacs-agent-ui--ticker))
            (should (string-match-p "lane compiler" (overlay-get (buffer-local-value 'romacs-agent-ui--composer chat) 'before-string)))
            (setq written 0)
            (with-current-buffer chat (romacs-agent-ui--composer-update))
            (should (= written 0))
            (setf (romacs-agent-state child) 'idle)
            ;; Its state moving reaches the parent as it did when it began.
            (romacs-agent-ui--flush-rows (list parent))
            (romacs-agent-ui--tick)
            (should-not romacs-agent-ui--ticker)
            (should-not (string-match-p "lane compiler"
                                        (or (overlay-get (buffer-local-value 'romacs-agent-ui--composer chat) 'before-string) ""))))
        (when (timerp romacs-agent-ui--ticker) (cancel-timer romacs-agent-ui--ticker))
        (kill-buffer chat)))))

(provide 'romacs-agent-live-rows-tests)
;;; romacs-agent-live-rows-tests.el ends here
