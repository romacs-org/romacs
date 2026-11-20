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
  "BODY with a session VAR that asks."
  (declare (indent 1))
  `(let ((,var (romacs-agent--make :id "11111111-2222-4333-8444-555555555555" :harness 'claude :state 'running
                                   :title "asker" :tools (make-hash-table :test #'equal))))
     (with-temp-buffer ,@body)))

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

(ert-deftest romacs-agent-ask-layers-are-each-sessions ()
  ;; Two sessions may ask under the same request id: a layer each.
  (let ((a (romacs-agent--make :id "aaaaaaaa-0000-4000-8000-000000000001" :harness 'codex :title "a"))
        (b (romacs-agent--make :id "bbbbbbbb-0000-4000-8000-000000000002" :harness 'codex :title "b"))
        (sent nil))
    (cl-letf (((symbol-function 'romacs-agent-respond)
               (lambda (session id allow &rest _) (setq sent (list (romacs-agent-title session) id allow)))))
      (let ((first (romacs-agent-ask-open a "1:0" '(("A?" :options ("x")))))
            (second (romacs-agent-ask-open b "1:0" '(("B?" :options ("y"))))))
        (should-not (eq first second))
        (with-current-buffer second (romacs-agent-ask-pick))
        (should (equal sent '("b" "1:0" (:answers (("B?" . "y"))))))
        (should (buffer-live-p first))
        ;; Its turn over, a session's layers go.
        (romacs-agent-ask-close-all a)
        (should-not (buffer-live-p first))))))

(ert-deftest romacs-agent-ask-size-wraps-long-text ()
  ;; As wide as its longest line, at most the room it has; as tall as
  ;; its lines wrapped there.
  (should (equal (romacs-agent-ask-size "abc\nde\n" 40) '(5 . 2)))
  (should (equal (romacs-agent-ask-size (concat (make-string 90 ?x) "\nshort\n") 40) '(40 . 4)))
  ;; At words: five 41-column words in 80 columns take five rows.
  (should (equal (romacs-agent-ask-size (string-join (make-list 5 (make-string 41 ?w)) " ") 80) '(80 . 5)))
  ;; An option's indent counts, and a row's last column is the wrap's.
  (should (= (romacs-agent-ask--rows-of (concat " 1 ○ " (string-join (make-list 15 "word") " ")) 80) 1))
  (should (= (romacs-agent-ask--rows-of (concat "    " (make-string 76 ?x)) 80) 2))
  (should (= (romacs-agent-ask--rows-of "abcd efgh" 9) 2))
  ;; Two spaces before a note are two columns.
  (should (= (romacs-agent-ask--rows-of (concat " 1 ○ " (make-string 69 ?x) "  note") 80) 2)))

(ert-deftest romacs-agent-ask-band-and-place ()
  ;; The band's row: the wrapped rows above the line.
  (let ((text "title\n\nlong long long\n 1 x\n"))
    (should (= (romacs-agent-ask-row-at text 1 80) 0))
    (should (= (romacs-agent-ask-row-at text 7 80) 1))
    (should (= (romacs-agent-ask-row-at text 23 80) 3))
    ;; Wrapped narrow, the long line takes three rows.
    (should (= (romacs-agent-ask-row-at text 23 7) 5)))
  ;; Below the question's row when it fits, else above; nowhere in
  ;; particular when the row is not in view.
  (should (equal (romacs-agent-ask-place 100 20 200 900) '(140 . 760)))
  (should (equal (romacs-agent-ask-place 800 20 200 900) '(580 . 780)))
  ;; Too tall for either side: the roomier one, no taller than it is.
  (should (equal (romacs-agent-ask-place 300 20 700 900) '(340 . 560)))
  (should (equal (romacs-agent-ask-place 700 20 800 900) '(0 . 680)))
  (should-not (romacs-agent-ask-place nil 20 200 900))
  ;; Sized to the room: as many rows as fit; none, no place of its own.
  (should (equal (romacs-agent-ask-fit 8 20 '(140 . 760)) '(140 . 8)))
  (should (equal (romacs-agent-ask-fit 8 20 '(0 . 84)) '(0 . 3)))
  (should-not (romacs-agent-ask-fit 8 20 (romacs-agent-ask-place 60 20 184 130))))

;;; romacs-agent-ask-tests.el ends here

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
