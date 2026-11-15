;;; romacs-agent-ask-tests.el --- Questions an agent asks, in a layer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-ui))

(defconst romacs-agent-ask-tests--questions
  '(("Which way?" :header "Approach" :options ("Tail" "Virtual") :notes (("Tail" . "the last turns")) :multi nil)))

(defmacro romacs-agent-ask-tests--with-chat (var &rest body)
  "BODY in a chat of a session VAR, its buffer the session's."
  (declare (indent 1))
  `(let ((,var (romacs-agent--make :id "11111111-2222-4333-8444-555555555555" :harness 'claude :state 'running
                                   :title "asker" :tools (make-hash-table :test #'equal))))
     (with-temp-buffer
       (romacs-agent-chat-mode)
       (setf (romacs-agent-buffer ,var) (current-buffer))
       (setq romacs-agent-ui--session ,var
             romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
             romacs-agent-ui--tasks (make-hash-table :test #'equal)
             romacs-agent-ui--messages (make-hash-table :test #'equal)
             romacs-agent-ui--end (point-marker))
       ,@body)))

(defun romacs-agent-ask-tests--draw (session &rest event)
  (romacs-agent-ui--draw session event (current-buffer)))

(ert-deftest romacs-agent-ask-layer-text ()
  (should (equal (substring-no-properties (romacs-agent-ask-text "asker" romacs-agent-ask-tests--questions nil))
                 (concat "asker\n\n"
                         "Approach  Which way?\n"
                         " 1 ○ Tail  the last turns\n"
                         " 2 ○ Virtual\n"
                         "   ✎ Other…\n")))
  ;; Several choices: boxes, the picked ones checked, a Send row.
  (let ((text (substring-no-properties
               (romacs-agent-ask-text "asker" '(("A?" :options ("x" "y") :multi t)) '((0 "y" "own words"))))))
    (should (string-search " 1 ☐ x\n 2 ☑ y\n   ✎ own words\n" text))
    (should (string-suffix-p "   ↵ Send\n" text))))

(ert-deftest romacs-agent-ask-answers-keep-labels-whole ()
  (let* ((questions '(("A?" :options ("Research, then implement" "B") :multi t) ("C?" :options ("z"))))
         (picks (romacs-agent-ask-repick nil 0 "Research, then implement" t)))
    (should-not (romacs-agent-ask-chosen questions picks))
    (setq picks (romacs-agent-ask-repick picks 0 "B" t))
    (setq picks (romacs-agent-ask-repick picks 1 "z" nil))
    (let ((chosen (romacs-agent-ask-chosen questions picks)))
      ;; Kept as labels; joined only for the harness.
      (should (equal chosen '(("A?" "Research, then implement" "B") ("C?" "z"))))
      (should (equal (romacs-agent-ask-answers chosen) '(("A?" . "Research, then implement, B") ("C?" . "z")))))
    ;; Picked again, a choice of several is taken back.
    (setq picks (romacs-agent-ask-repick picks 0 "B" t))
    (should (equal (cdr (assq 0 picks)) '("Research, then implement")))))

(ert-deftest romacs-agent-ask-record-row ()
  (should (equal (substring-no-properties (romacs-agent-ask-record romacs-agent-ask-tests--questions nil))
                 "? Which way?  · waiting: RET answers"))
  (should (equal (substring-no-properties (romacs-agent-ask-record romacs-agent-ask-tests--questions '(("Which way?" "Tail"))))
                 "? Which way?  → Tail"))
  (should (equal (substring-no-properties (romacs-agent-ask-record '(("A?") ("B?")) t)) "? A? (+1)  · answered")))

(ert-deftest romacs-agent-ask-answered-from-its-layer ()
  ;; The chat keeps one row; RET on it brings the layer; a pick there
  ;; answers the agent, and the row says the answer once it is taken.
  (romacs-agent-ask-tests--with-chat session
    (let ((sent nil) (chat (current-buffer)))
      (cl-letf (((symbol-function 'romacs-agent-respond)
                 (lambda (_session id allow &rest _) (setq sent (list id allow)))))
        (romacs-agent-ask-tests--draw session :type 'approval :request-id "r1" :tool "AskUserQuestion" :user t
                                      :questions romacs-agent-ask-tests--questions)
        (romacs-agent--set-asks session (list "r1"))
        (should (string-search "? Which way?  · waiting" (buffer-string)))
        (goto-char (point-min))
        (search-forward "Which way")
        (let ((layer (romacs-agent-chat-answer)))
          (with-current-buffer layer
            ;; On the first option; n and p step through them.
            (should (equal (romacs-agent-ask--here) '(0 . "Tail")))
            (romacs-agent-ask-next)
            (should (equal (romacs-agent-ask--here) '(0 . "Virtual")))
            (romacs-agent-ask-previous)
            (romacs-agent-ask-pick))
          (should (equal sent '("r1" (:answers (("Which way?" . "Tail"))))))
          (should-not (buffer-live-p layer)))
        (with-current-buffer chat
          (romacs-agent-ask-tests--draw session :type 'answered :request-id "r1" :behavior "allow")
          (should (string-search "? Which way?  → Tail" (buffer-string)))
          (should-not (string-search "waiting" (buffer-string)))
          (should-not (text-property-not-all (point-min) (point-max) 'romacs-agent-asked nil)))))))

(ert-deftest romacs-agent-ask-own-words-and-digits ()
  (romacs-agent-ask-tests--with-chat session
    (let ((sent nil))
      (cl-letf (((symbol-function 'romacs-agent-respond)
                 (lambda (_session id allow &rest _) (setq sent (list id allow))))
                ((symbol-function 'read-string) (lambda (&rest _) "Neither")))
        (with-current-buffer (romacs-agent-ask-open session "r2" romacs-agent-ask-tests--questions)
          (goto-char (point-min))
          (search-forward "Other")
          (romacs-agent-ask-pick))
        (should (equal sent '("r2" (:answers (("Which way?" . "Neither"))))))
        (with-current-buffer (romacs-agent-ask-open session "r3" romacs-agent-ask-tests--questions)
          (cl-letf (((symbol-function 'this-command-keys-vector) (lambda () [?2])))
            (romacs-agent-ask-pick-number)))
        (should (equal sent '("r3" (:answers (("Which way?" . "Virtual"))))))))))

(ert-deftest romacs-agent-ask-answered-elsewhere ()
  ;; Answered where this chat sent nothing (a replay, another client):
  ;; its row says it was answered.
  (romacs-agent-ask-tests--with-chat session
    (romacs-agent-ask-tests--draw session :type 'approval :request-id "r5" :user t
                                  :questions romacs-agent-ask-tests--questions)
    (romacs-agent-ask-tests--draw session :type 'answered :request-id "r5" :behavior "allow")
    (should (string-search "? Which way?  · answered" (buffer-string)))))

;;; romacs-agent-ask-tests.el ends here
