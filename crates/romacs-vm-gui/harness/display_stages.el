;;; display_stages.el --- what one redisplay costs, by scenario  -*- lexical-binding: t -*-

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; display_stages_live.py runs each scenario of `display-stages-scenarios'
;; in a fresh session, after scaling/probes.el (its helpers).  A scenario
;; is a buffer shown one way and a step that changes it; each step runs
;; from a timer, then the host redisplays, and the next step starts once
;; `romacs--redisplay-since-mark' saw that redisplay.  At the start of
;; each step, and after the last one's redisplay, the session keeps the
;; evaluator's `romacs-instructions', `romacs--display-stats' and
;; `gcs-done': the differences are what a step cost in all, what its
;; redisplay cost alone, and whether a collection fell in it.  Every
;; reading is taken before anything is serialized.
;;
;; A scenario is (display-stages-scenario NAME [:paints t] :setup FORM
;; :body FORM): :setup's value is `it' in :body, `i' the step; :paints
;; says each step changes what the frame shows, so the host submits a
;; frame for it (a scenario without it must submit none).  All buffers are
;; synthetic.

;;; Code:

(require 'cl-lib)
(require 'json)

(defvar display-stages-scenarios nil
  "The scenarios, newest first: (NAME . PLIST).")

(defmacro display-stages-scenario (name &rest plist)
  "Define scenario NAME from PLIST, its :setup and :body forms made functions."
  (declare (indent 1))
  `(setf (alist-get ',name display-stages-scenarios)
         (list :paints ,(plist-get plist :paints)
               :setup (lambda () ,(plist-get plist :setup))
               :body (lambda (it i) (ignore it i) ,(plist-get plist :body)))))

(defun display-stages--elements (name n)
  "A buffer NAME of N lines, each with invisible text and a `display' string."
  (with-current-buffer (get-buffer-create name)
    (erase-buffer)
    (dotimes (i n)
      (insert (format "line %d " i) (propertize "hidden words " 'invisible t)
              (propertize "x" 'display "[shown instead]") " of some ordinary prose text\n"))
    (goto-char (point-min))
    (current-buffer)))

(defun display-stages--show (buffer)
  "Show BUFFER in the selected window from its start; return it."
  (switch-to-buffer buffer)
  (goto-char (point-min))
  buffer)

(defvar romacs-agent-ui--ticker)

(defun display-stages--chat (chars)
  "Show an agent chat of CHARS characters over 100 turns, point at its end;
return its buffer.  Its session says what one in use says, so its header
does: a permission mode, a model, how full its context is, a turn running.
The chat's 1 s ticker is stopped: whether a tick of the wall clock lands in
a step is the host's load, so `chat-tick' runs what it does as its step."
  (let* ((buffer (scaling--big-chat chars))
         (session (buffer-local-value 'romacs-agent-ui--session buffer)))
    (advice-add 'romacs-agent-ui--tick-start :override #'ignore)
    (dolist (event '((:type session :model "claude-opus-5-5" :permission-mode "acceptEdits")
                     (:type usage :context-used 440000)
                     (:type done :context-window 1000000)
                     (:type input :text "a prompt")))
      (romacs-agent--raise session event))
    (when (timerp romacs-agent-ui--ticker)
      (cancel-timer romacs-agent-ui--ticker)
      (setq romacs-agent-ui--ticker nil))
    (switch-to-buffer buffer)
    (goto-char (point-max))
    buffer))

(defun display-stages--turn (buffer seconds)
  "Make the chat BUFFER's session one whose turn began SECONDS ago; return BUFFER."
  (romacs-agent--raise (buffer-local-value 'romacs-agent-ui--session buffer)
                       (list :type 'input :text "a prompt" :time (- (float-time) seconds)))
  buffer)

(defun display-stages--second (buffer)
  "A second passes in the running turn of the chat BUFFER's session, as a
tick finds it, no event raised: its turn began a second earlier.  Return
BUFFER.  (The slot by name: this file loads before the session's type.)"
  (cl-decf (cl-struct-slot-value 'romacs-agent 'turn-start
                                 (buffer-local-value 'romacs-agent-ui--session buffer)))
  buffer)

(defun display-stages--streaming-chat (chars)
  "`display-stages--chat', a reply about to stream at its end."
  (let ((buffer (display-stages--chat chars)))
    (romacs-agent-ui--render (buffer-local-value 'romacs-agent-ui--session buffer)
                             (list :type 'input :text "another prompt" :seq 5001 :last t))
    (goto-char (point-max))
    buffer))

(defun display-stages--delta (buffer i)
  "Stream the I-th 20-character delta of the reply into the chat BUFFER, as
its session raises it: drawn, and every view of the session told."
  (romacs-agent--raise (buffer-local-value 'romacs-agent-ui--session buffer)
                       (list :type 'delta :text "some streamed words " :message "stream"
                             :seq (+ 5002 i) :last t)))

;; A timer ran and nothing changed: the frame is frozen and compared.
(display-stages-scenario plain-idle
  :setup (display-stages--show (scaling--lines-buffer "stages" 20000))
  :body nil)
;; The same in an agent chat of 1,000-character lines.
(display-stages-scenario chat-idle
  :setup (display-stages--chat 100000)
  :body nil)
;; What the chat's ticker does each second of a turn, in a second that
;; leaves its composer's boundary and hint as they read: nothing is written.
(display-stages-scenario chat-tick
  :setup (display-stages--chat 100000)
  :body (with-current-buffer it (romacs-agent-ui--composer-update)))
;; The same in a second the running turn's clock moved in: the boundary's
;; rule says a second more, one overlay string put.
(display-stages-scenario chat-tick-second :paints t
  :setup (display-stages--turn (display-stages--chat 100000) 0)
  :body (with-current-buffer (display-stages--second it)
          (romacs-agent-ui--composer-update)))
;; Point moves in the chat's composer: the frame changes, its header does not.
(display-stages-scenario chat-cursor :paints t
  :setup (with-current-buffer (display-stages--chat 100000)
           (insert "some words typed in the composer")
           (current-buffer))
  :body (if (cl-evenp i) (backward-char 1) (forward-char 1)))
;; A collection, in the same chat: nothing shown changed.
(display-stages-scenario chat-collected
  :setup (display-stages--chat 100000)
  :body (garbage-collect))
;; Point moves along a row the window shows.
(display-stages-scenario cursor-move :paints t
  :setup (display-stages--show (scaling--lines-buffer "stages" 20000))
  :body (forward-char 1))
;; One character typed at point.
(display-stages-scenario typed :paints t
  :setup (display-stages--show (scaling--lines-buffer "stages" 20000))
  :body (insert "a"))
;; Text arrives at the end of a buffer whose window shows its start.
(display-stages-scenario append-unseen
  :setup (display-stages--show (scaling--lines-buffer "stages" 20000))
  :body (with-current-buffer it
          (save-excursion (goto-char (point-max)) (insert "an appended line\n"))))
;; One character typed among invisible and `display' text.
(display-stages-scenario typed-elements :paints t
  :setup (display-stages--show (display-stages--elements "stages" 20000))
  :body (insert "a"))
;; A reply streams into a chat whose lines are 1,000 and 10,000 characters.
(display-stages-scenario chat-stream-1k :paints t
  :setup (display-stages--streaming-chat 100000)
  :body (display-stages--delta it i))
(display-stages-scenario chat-stream-10k :paints t
  :setup (display-stages--streaming-chat 1000000)
  :body (display-stages--delta it i))

(defun display-stages-list ()
  "Print each scenario's name and whether its steps paint, one SCALING line each."
  (dolist (scenario (reverse display-stages-scenarios))
    (scaling--print (list :name (symbol-name (car scenario))
                          :paints (if (plist-get (cdr scenario) :paints) t :json-false)))))

(defun display-stages--reading ()
  "The evaluator's instructions and collections so far, and redisplay's counts."
  `((step-instructions . ,(romacs-instructions)) (collections . ,gcs-done) ,@(romacs--display-stats)))

(defun display-stages-start (name out steps)
  "Run scenario NAME for STEPS steps in this session, then exit.
Once the session is idle, set the scenario up; then run its steps from a
chain of timers, each after the host's redisplay of the step before.  Take
a reading at the start of each step and after the last one's redisplay;
only then stop the profile `scaling--perf' controls, mark the end on
stderr, and write the readings to OUT as JSON."
  (let* ((plist (or (alist-get name display-stages-scenarios) (error "No scenario %s" name)))
         (body (plist-get plist :body)) (gap 0.05) it readings)
    (setq blink-cursor-mode nil)
    (run-with-idle-timer
     0.5 nil
     (lambda ()
       (setq it (funcall (plist-get plist :setup)))
       (garbage-collect)
       (run-with-timer
        1.0 nil
        (lambda ()
          (let ((step nil) (laid-out nil) (i 0))
            (setq laid-out (lambda ()
                             (if (romacs--redisplay-since-mark) (funcall step) (run-with-timer gap nil laid-out))))
            (setq step (lambda ()
                         (push (display-stages--reading) readings)
                         (if (< i steps)
                             (progn (set-buffer (window-buffer (selected-window)))
                                    (funcall body it i) (setq i (1+ i))
                                    (romacs--redisplay-since-mark t)
                                    (run-with-timer gap nil laid-out))
                           ;; The last reading is taken: nothing below is in any count.
                           (scaling--perf "disable")
                           (scaling--mark (list :stages "end"))
                           (with-temp-file out
                             (insert (json-encode
                                      (list :name (symbol-name name) :steps steps
                                            :chars (with-current-buffer (window-buffer (selected-window)) (buffer-size))
                                            :readings (vconcat (nreverse readings))))))
                           (kill-emacs 0))))
            (scaling--mark (list :stages "start"))
            (scaling--perf "enable")
            (funcall step))))))))

(provide 'display-stages)
;;; display_stages.el ends here
