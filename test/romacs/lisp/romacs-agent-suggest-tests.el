;;; romacs-agent-suggest-tests.el --- The prompt a harness predicts, in the composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(eval-and-compile
  (let ((romacs-mcp-call-function romacs-mcp-call-function)
        (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (require 'romacs-agent)
    (require 'romacs-agent-claude)
    (require 'romacs-agent-ui)
    (require 'completion-preview)))

(defmacro romacs-agent-suggest-tests--registry (&rest body)
  "BODY with the driver registry its own: what it defines goes with it."
  `(let ((romacs-agent--drivers (copy-alist romacs-agent--drivers))
         (romacs-agent--driver-table (make-hash-table :test #'eq)))
     (dolist (cell romacs-agent--drivers) (puthash (car cell) cell romacs-agent--driver-table))
     ,@body))

(defun romacs-agent-suggest-tests--line (object)
  (json-parse-string (json-serialize object) :object-type 'hash-table))

(defun romacs-agent-suggest-tests--feed (session seq key object)
  "Journal record SEQ of SESSION: OBJECT under KEY (\"in\" or \"out\")."
  (let ((record (make-hash-table :test #'equal)))
    (puthash "seq" seq record)
    (puthash key (romacs-agent-suggest-tests--line object) record)
    (romacs-agent--record session record)))

(defun romacs-agent-suggest-tests--suggestion (text)
  (romacs-mcp--object "type" "prompt_suggestion" "suggestion" text "uuid" "u" "session_id" "s"))

(defun romacs-agent-suggest-tests--result ()
  (romacs-mcp--object "type" "result" "subtype" "success" "is_error" :false "result" "done"))

(ert-deftest romacs-agent-suggest-claude-line-is-a-suggestion-event ()
  (let ((session (romacs-agent--make :id "s" :harness 'claude)))
    (should (equal (romacs-agent--claude-events
                    session (romacs-agent-suggest-tests--line (romacs-agent-suggest-tests--suggestion " run it to confirm\n")))
                   '((:type suggestion :text "run it to confirm"))))
    ;; Nothing predicted is no event.
    (dolist (empty '("" "  \n"))
      (should-not (romacs-agent--claude-events
                   session (romacs-agent-suggest-tests--line (romacs-agent-suggest-tests--suggestion empty)))))))

(ert-deftest romacs-agent-suggest-driver-asks-while-on ()
  (romacs-agent-suggest-tests--registry
   (romacs-agent-define-harness 'plaindrv :lifetime 'persistent)
   (let ((claude (romacs-agent--make :id "c" :harness 'claude))
         (plain (romacs-agent--make :id "p" :harness 'plaindrv)))
     (let ((romacs-agent-suggestions t))
       (should (equal (romacs-agent--suggestion-words claude) '("--prompt-suggestions")))
       ;; A driver that cannot ask asks nothing.
       (should-not (romacs-agent--suggestion-words plain)))
     (let ((romacs-agent-suggestions nil))
       (should-not (romacs-agent--suggestion-words claude))))))

(ert-deftest romacs-agent-suggest-belongs-to-the-gap-between-turns ()
  (let ((romacs-agent-event-functions nil)
        (session (romacs-agent--make :id "s" :harness 'claude :state 'running)))
    ;; Predicted mid-turn (the turn before's, late): not offered.
    (romacs-agent-suggest-tests--feed session 1 "out" (romacs-agent-suggest-tests--suggestion "stale"))
    (should-not (romacs-agent-suggestion session))
    (romacs-agent-suggest-tests--feed session 2 "out" (romacs-agent-suggest-tests--result))
    (should (eq (romacs-agent-state session) 'idle))
    (romacs-agent-suggest-tests--feed session 3 "out" (romacs-agent-suggest-tests--suggestion "run it to confirm"))
    (should (equal (romacs-agent-suggestion session) "run it to confirm"))
    ;; The next prompt starts a turn: the prediction is moot.
    (romacs-agent-suggest-tests--feed session 4 "in" (romacs-agent--claude-user session "something else" 1))
    (should (eq (romacs-agent-state session) 'running))
    (should-not (romacs-agent-suggestion session))
    ;; A run's end takes it with it.
    (romacs-agent-suggest-tests--feed session 5 "out" (romacs-agent-suggest-tests--result))
    (romacs-agent-suggest-tests--feed session 6 "out" (romacs-agent-suggest-tests--suggestion "again"))
    (should (equal (romacs-agent-suggestion session) "again"))
    (let ((record (make-hash-table :test #'equal)))
      (puthash "seq" 7 record)
      (puthash "exit" 0 record)
      (romacs-agent--record session record))
    (should-not (romacs-agent-suggestion session))))

(defmacro romacs-agent-suggest-tests--chat (suggestion &rest body)
  "BODY in an empty composer of an idle session whose harness predicted
SUGGESTION (nil, none)."
  (declare (indent 1))
  `(with-temp-buffer
     (romacs-agent-chat-mode)
     (setq romacs-agent-ui--session (romacs-agent--make :id "s" :harness 'claude :state 'idle
                                                         :driver-state (list :suggestion ,suggestion)))
     (let ((inhibit-read-only t)) (insert "> earlier\nreply\n"))
     (setq romacs-agent-ui--end (point-marker))
     (setq romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil))
     (romacs-agent-ui--composer-update)
     ,@body))

(defun romacs-agent-suggest-tests--shown ()
  "What the composer's overlay draws after it, as text."
  (let ((after (overlay-get romacs-agent-ui--composer 'after-string)))
    (and after (substring-no-properties after))))

(ert-deftest romacs-agent-suggest-composer-shows-hides-and-shows-again ()
  (let ((romacs-agent-suggestions t))
    (romacs-agent-suggest-tests--chat "run it to confirm"
      (should (string-prefix-p "run it to confirm" (romacs-agent-suggest-tests--shown)))
      (should (eq (get-text-property 0 'face (overlay-get romacs-agent-ui--composer 'after-string))
                  'romacs-agent-dim))
      ;; Never the buffer's text.
      (should (equal (romacs-agent-ui--composer-text) ""))
      ;; Typing hides it; deleting back to empty shows it again.
      (insert "x")
      (romacs-agent-ui--composer-update)
      (should-not (romacs-agent-suggest-tests--shown))
      (delete-char -1)
      (romacs-agent-ui--composer-update)
      (should (string-prefix-p "run it to confirm" (romacs-agent-suggest-tests--shown)))
      ;; Turned off: the plain hint at once.
      (let ((romacs-agent-suggestions nil))
        (romacs-agent-ui--composer-update)
        (should (equal (romacs-agent-suggest-tests--shown) "Message…"))))))

;;; It is a completion: TAB, M-TAB, C-M-i take it; typed, a preview shows the rest

(ert-deftest romacs-agent-suggest-without-one-shows-the-hint ()
  ;; A harness that predicts nothing (a driver without :suggestions).
  (let ((romacs-agent-suggestions t))
    (romacs-agent-suggest-tests--chat nil
      (should (equal (romacs-agent-suggest-tests--shown) "Message…"))
      (should-not (romacs-agent-ui--suggestion-capf))
      (completion-at-point)
      (should (equal (romacs-agent-ui--composer-text) "")))))

(ert-deftest romacs-agent-suggest-completion-takes-it ()
  (let ((romacs-agent-suggestions t))
    (dolist (key '("TAB" "M-TAB" "C-M-i"))
      (romacs-agent-suggest-tests--chat "run it to confirm"
        (call-interactively (key-binding (kbd key)))
        (should (equal (romacs-agent-ui--composer-text) "run it to confirm"))
        (should (= (point) (point-max)))
        (romacs-agent-ui--composer-update)
        (should-not (romacs-agent-suggest-tests--shown))
        ;; Taken, it is taken once, wherever point is in it.
        (call-interactively (key-binding (kbd key)))
        (goto-char romacs-agent-ui--end)
        (call-interactively (key-binding (kbd key)))
        (should (equal (romacs-agent-ui--composer-text) "run it to confirm"))))
    ;; Typed text that begins it completes to it.
    (romacs-agent-suggest-tests--chat "run it to confirm"
      (insert "run i")
      (completion-at-point)
      (should (equal (romacs-agent-ui--composer-text) "run it to confirm")))))

(ert-deftest romacs-agent-suggest-ret-never-sends-it ()
  (let ((romacs-agent-suggestions t) (sent nil))
    (romacs-agent-suggest-tests--chat "run it to confirm"
      (cl-letf (((symbol-function 'romacs-agent-send) (lambda (&rest args) (push args sent))))
        (romacs-agent-chat-return)
        (should-not sent)
        (should (equal (romacs-agent-ui--composer-text) ""))
        ;; Taken, it is the user's message.
        (completion-at-point)
        (romacs-agent-chat-return)
        (should (equal (nth 1 (car sent)) "run it to confirm"))))))

(ert-deftest romacs-agent-suggest-capf-completes-what-begins-it ()
  (let ((romacs-agent-suggestions t) (suggestion "add tests for reverse_words"))
    (romacs-agent-suggest-tests--chat suggestion
      ;; The chat's own: no other buffer completes to it.
      (should (memq #'romacs-agent-ui--suggestion-capf completion-at-point-functions))
      (should-not (memq #'romacs-agent-ui--suggestion-capf (default-value 'completion-at-point-functions)))
      (let ((start (marker-position romacs-agent-ui--end)))
        (should (equal (romacs-agent-ui--suggestion-capf) (list start start (list suggestion) :exclusive 'no)))
        (insert "add t")
        (should (equal (romacs-agent-ui--suggestion-capf) (list start (point-max) (list suggestion) :exclusive 'no))))
      ;; Never in the transcript.
      (save-excursion (goto-char (point-min)) (should-not (romacs-agent-ui--suggestion-capf)))
      ;; Typed text that does not begin it.
      (insert "x")
      (should-not (romacs-agent-ui--suggestion-capf))
      (delete-char -1)
      ;; None predicted, or offering turned off.
      (let ((romacs-agent-suggestions nil)) (should-not (romacs-agent-ui--suggestion-capf)))
      (setf (romacs-agent-driver-state romacs-agent-ui--session) nil)
      (should-not (romacs-agent-ui--suggestion-capf)))))

(defun romacs-agent-suggest-tests--preview-after (text)
  "The completion preview after TEXT is typed in the empty composer, or nil."
  (completion-preview-active-mode -1)
  (delete-region romacs-agent-ui--end (point-max))
  (insert text)
  (let ((this-command (if (string-empty-p text) 'delete-backward-char 'self-insert-command)))
    (completion-preview--post-command))
  (and completion-preview-active-mode
       (substring-no-properties (overlay-get completion-preview--overlay 'after-string))))

(ert-deftest romacs-agent-suggest-preview-shows-the-rest ()
  (let ((romacs-agent-suggestions t) (suggestion "add tests for reverse_words"))
    (romacs-agent-suggest-tests--chat suggestion
      (completion-preview-mode)
      ;; After any character typed, a space too.
      (dolist (typed '("a" "add " "add tests f"))
        (should (equal (romacs-agent-suggest-tests--preview-after typed) (substring suggestion (length typed)))))
      (completion-preview-insert)
      (should (equal (romacs-agent-ui--composer-text) suggestion))
      ;; Deleted back to empty: the hint shows it, no preview over it.
      (should-not (romacs-agent-suggest-tests--preview-after ""))
      (romacs-agent-ui--composer-update)
      (should (string-prefix-p suggestion (romacs-agent-suggest-tests--shown))))))

(provide 'romacs-agent-suggest-tests)
;;; romacs-agent-suggest-tests.el ends here
