;;; romacs-agent-ask-tests.el --- Questions an agent asks, as cards -*- lexical-binding: t; -*-
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
  "BODY in a chat of a session VAR."
  (declare (indent 1))
  `(let ((,var (romacs-agent--make :id "11111111-2222-4333-8444-555555555555" :harness 'claude :state 'running
                                   :title "asker" :tools (make-hash-table :test #'equal))))
     (with-temp-buffer
       (romacs-agent-chat-mode)
       (setq romacs-agent-ui--session ,var
             romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
             romacs-agent-ui--tasks (make-hash-table :test #'equal)
             romacs-agent-ui--messages (make-hash-table :test #'equal)
             romacs-agent-ui--end (point-marker))
       ,@body)))

(defun romacs-agent-ask-tests--draw (session &rest event)
  (romacs-agent-ui--draw session event (current-buffer)))

(ert-deftest romacs-agent-ask-card-rows ()
  (should (equal (substring-no-properties (romacs-agent-ask-card "r1" romacs-agent-ask-tests--questions))
                 (concat "◆ Approach  Which way?\n"
                         "    ○ Tail  the last turns\n"
                         "    ○ Virtual\n"
                         "    ✎ Other…\n")))
  ;; Several questions, or several choices: a Send row.
  (should (string-suffix-p "    ↵ Send\n"
                           (substring-no-properties
                            (romacs-agent-ask-card "r1" '(("A?" :options ("x") :multi t)))))))

(ert-deftest romacs-agent-ask-answers-from-picks ()
  (let ((questions '(("A?" :options ("x" "y") :multi t) ("B?" :options ("z")))))
    (should-not (romacs-agent-ask-answers questions nil))
    (let ((picks (romacs-agent-ask--repick nil 0 "x" t)))
      (setq picks (romacs-agent-ask--repick picks 0 "y" t))
      (should-not (romacs-agent-ask-answers questions picks))
      (setq picks (romacs-agent-ask--repick picks 1 "z" nil))
      (should (equal (romacs-agent-ask-answers questions picks) '(("A?" . "x, y") ("B?" . "z"))))
      ;; Picked again, a choice of several is taken back.
      (setq picks (romacs-agent-ask--repick picks 0 "x" t))
      (should (equal (romacs-agent-ask-answers questions picks) '(("A?" . "y") ("B?" . "z")))))))

(ert-deftest romacs-agent-ask-picking-answers-and-settles ()
  ;; One question of one choice: RET on an option sends it; answered, the
  ;; card keeps the pick and its other rows recede.
  (romacs-agent-ask-tests--with-chat session
    (let ((sent nil))
      (cl-letf (((symbol-function 'romacs-agent-respond)
                 (lambda (_session id allow &rest _) (setq sent (list id allow)))))
        (romacs-agent-ask-tests--draw session :type 'approval :request-id "r1" :tool "AskUserQuestion" :user t
                                      :questions romacs-agent-ask-tests--questions)
        (goto-char (point-min))
        (search-forward "○ Tail")
        ;; n and p step through the options.
        (romacs-agent-ask-next)
        (should (looking-at-p "Virtual"))
        (romacs-agent-ask-previous)
        (should (looking-at-p "Tail"))
        (romacs-agent-ask-pick)
        (should (equal sent '("r1" (:answers (("Which way?" . "Tail"))))))
        (goto-char (point-min))
        (should (search-forward "● Tail" nil t))
        (should (eq (get-text-property (- (point) 1) 'face) 'romacs-agent-ask-picked))
        (romacs-agent-ask-tests--draw session :type 'answered :request-id "r1" :behavior "allow")
        ;; Settled: no option keys, the rest dim, no "answered" row.
        (goto-char (point-min))
        (search-forward "Virtual")
        (should-not (eq (get-text-property (point) 'keymap) romacs-agent-ask-map))
        (should (memq 'romacs-agent-dim (ensure-list (get-text-property (1- (point)) 'face))))
        (should-not (string-search "answered" (buffer-string)))))))

(ert-deftest romacs-agent-ask-own-words ()
  (romacs-agent-ask-tests--with-chat session
    (let ((sent nil))
      (cl-letf (((symbol-function 'romacs-agent-respond)
                 (lambda (_session id allow &rest _) (setq sent (list id allow))))
                ((symbol-function 'read-string) (lambda (&rest _) "Neither")))
        (romacs-agent-ask-tests--draw session :type 'approval :request-id "r2" :user t
                                      :questions romacs-agent-ask-tests--questions)
        (goto-char (point-min))
        (search-forward "Other")
        (romacs-agent-ask-pick)
        (should (equal sent '("r2" (:answers (("Which way?" . "Neither"))))))))))

(ert-deftest romacs-agent-ask-settles-on-what-was-sent ()
  ;; Picked on the card, then answered by y with other words: the card
  ;; says what was sent.
  (romacs-agent-ask-tests--with-chat session
    (cl-letf (((symbol-function 'romacs-agent-respond) #'ignore)
              ((symbol-function 'romacs-agent-ui--read-answers) (lambda (_) '(("A?" . "y")))))
      (romacs-agent-ask-tests--draw session :type 'approval :request-id "r3" :user t
                                    :questions '(("A?" :options ("x" "y") :multi t)))
      (goto-char (point-min))
      (search-forward "☐ x")
      (romacs-agent-ask-pick)
      (romacs-agent-chat-allow)
      (romacs-agent-ask-tests--draw session :type 'answered :request-id "r3" :behavior "allow")
      (let ((text (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
        (should (string-search "☐ x" text))
        (should (string-search "☑ y" text))
        (should-not (string-search "Send" text)))))
  ;; Words of one's own take the Other row.
  (romacs-agent-ask-tests--with-chat session
    (cl-letf (((symbol-function 'romacs-agent-respond) #'ignore)
              ((symbol-function 'read-string) (lambda (&rest _) "Neither")))
      (romacs-agent-ask-tests--draw session :type 'approval :request-id "r4" :user t
                                    :questions romacs-agent-ask-tests--questions)
      (goto-char (point-min))
      (search-forward "Other")
      (romacs-agent-ask-pick)
      (romacs-agent-ask-tests--draw session :type 'answered :request-id "r4" :behavior "allow")
      (should (string-search "    ● Neither\n" (buffer-string)))
      (should-not (string-search "Other" (buffer-string)))))
  ;; Answered where this chat sent nothing (a replay): it says so.
  (romacs-agent-ask-tests--with-chat session
    (romacs-agent-ask-tests--draw session :type 'approval :request-id "r5" :user t
                                  :questions romacs-agent-ask-tests--questions)
    (romacs-agent-ask-tests--draw session :type 'answered :request-id "r5" :behavior "allow")
    (should (string-suffix-p "    ○ Virtual\n    ✓ answered\n" (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
    (should-not (text-property-not-all (point-min) (point-max) 'romacs-agent-ask nil))))

;;; romacs-agent-ask-tests.el ends here
