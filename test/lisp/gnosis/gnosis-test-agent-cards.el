;;; gnosis-test-agent-cards.el --- Creating cards for agents -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;; Author: Thanos Apollo <public@thanosapollo.org>

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;;; Commentary:

;; `gnosis-agent-content-create': batches of every text type, idempotent
;; retries, dry runs, and per-card errors with nothing written.

;;; Code:

(require 'ert)
(require 'gnosis-agent-content)
(require 'gnosis-test-helpers)
(require 'romacs-agent-gnosis)

(defconst gnosis-test-cards--batch
  (vector
   (list :type "basic" :question "Capital of Greece?" :answer ["Athens"] :tags ["geo"]
         :aliases ["Athina"])
   (list :type "double" :question "Hund" :answer ["dog"] :tags ["de"])
   (list :type "mcq" :question "Largest planet?" :choices ["Mars" "Jupiter" "Venus"]
         :answer ["Jupiter"] :parathema "A gas giant.")
   (list :type "cloze" :question "Water boils at 100 degrees at sea level."
         :answer ["100" "sea level"] :tags ["phys" "basics"])
   (list :type "agent-eval" :question "Why does ice float?"
         :answer ["It is less dense than liquid water."]
         :rubric "Mentions density" :key "ice-float")
   (list :type "mc-cloze" :question "The heart pumps blood."
         :choices ["heart" "liver" "lung"] :answer ["heart"])
   (list :type "cloze" :question "{c1:Penicillin} inhibits {c2:cell wall} synthesis."))
  "One card of each type, an inline cloze among them.")

(defun gnosis-test-cards--count ()
  "Return how many themata the database holds."
  (caar (gnosis-sqlite-select gnosis-db "SELECT COUNT(*) FROM themata")))

(defun gnosis-test-cards--statuses (result)
  "Return each card's status in RESULT."
  (mapcar (lambda (card) (plist-get card :status)) (plist-get result :cards)))

(ert-deftest gnosis-test-cards-create-a-batch-of-every-type ()
  "Create every type in one call, with tags and aliases, and report IDs."
  (gnosis-test-with-db
    (let* ((result (gnosis-agent-content-create gnosis-test-cards--batch))
           (cards (plist-get result :cards)))
      (should (eq t (plist-get result :written)))
      (should (eq :false (plist-get result :dry-run)))
      (should (equal (make-list 7 "created") (gnosis-test-cards--statuses result)))
      ;; A double makes two themata, reversed; an inline cloze one per blank.
      (should (equal '(1 2 1 1 1 1 2)
                     (mapcar (lambda (card) (length (plist-get card :ids))) cards)))
      (should (= 9 (gnosis-test-cards--count)))
      (let ((basic (string-to-number (aref (plist-get (aref cards 0) :ids) 0))))
        (should (equal '("Athina") (gnosis-get 'accepted-aliases 'themata `(= id ,basic)))))
      (let ((mcq (string-to-number (aref (plist-get (aref cards 2) :ids) 0))))
        (should (equal '("mcq" "Largest planet?" ("Mars" "Jupiter" "Venus") ("Jupiter"))
                       (car (gnosis-select '[type keimenon hypothesis answer]
                                           'themata `(= id ,mcq))))))
      (let ((cloze (string-to-number (aref (plist-get (aref cards 3) :ids) 0))))
        (should (equal '("basics" "phys")
                       (sort (gnosis-select 'tag 'thema-tag `(= thema-id ,cloze) t)
                             #'string<))))
      (should (equal "ice-float" (plist-get (aref cards 4) :key)))
      (should-not (plist-get (aref cards 0) :key)))))

(ert-deftest gnosis-test-cards-retry-is-idempotent ()
  "A retried call reports the cards it made and makes nothing new."
  (gnosis-test-with-db
    (let* ((first (gnosis-agent-content-create gnosis-test-cards--batch))
           (count (gnosis-test-cards--count))
           (again (gnosis-agent-content-create gnosis-test-cards--batch)))
      (should (equal (make-list 7 "exists") (gnosis-test-cards--statuses again)))
      (should (eq :false (plist-get again :written)))
      (should (= count (gnosis-test-cards--count)))
      (should (equal (mapcar (lambda (card) (plist-get card :ids)) (plist-get first :cards))
                     (mapcar (lambda (card) (plist-get card :ids)) (plist-get again :cards))))
      ;; Without a key, content the learner sees or is graded on tells
      ;; cards apart; tags alone do not.
      (let ((base (list :type "agent-eval" :question "Why is the sky blue?"
                        :answer ["Rayleigh scattering"] :rubric "Names scattering")))
        (should (equal '("created") (gnosis-test-cards--statuses
                                     (gnosis-agent-content-create (vector base)))))
        (dolist (change '((:rubric "Names scattering and wavelength")
                          (:parathema "Short wavelengths scatter more.")))
          (should (equal '("created")
                         (gnosis-test-cards--statuses
                          (gnosis-agent-content-create
                           (vector (append change (copy-sequence base)))))))))
      ;; Aliases are a basic card's; agent-eval takes none.
      (let ((base (list :type "basic" :question "Gas giant?" :answer ["Jupiter"])))
        (dolist (card (list base (append '(:aliases ["Zeus"]) base)))
          (should (equal '("created")
                         (gnosis-test-cards--statuses
                          (gnosis-agent-content-create (vector card)))))))
      (should (equal '("exists")
                     (gnosis-test-cards--statuses
                      (gnosis-agent-content-create
                       (vector (list :type "basic" :question "Capital of Greece?" :answer ["Athens"]
                                     :tags ["places"] :aliases ["Athina"]))))))
      ;; An explicit key holds even when the content changes.
      (let ((changed (gnosis-agent-content-create
                      (vector (list :type "agent-eval" :question "Why does ice float on water?"
                                    :answer ["Lower density."] :rubric "Density"
                                    :key "ice-float")))))
        (should (equal '("exists") (gnosis-test-cards--statuses changed)))))))

(ert-deftest gnosis-test-cards-dry-run-writes-nothing ()
  "A dry run reports what each card would make and keeps nothing."
  (gnosis-test-with-db
    (let ((result (gnosis-agent-content-create gnosis-test-cards--batch :dry-run t)))
      (should (eq t (plist-get result :dry-run)))
      (should (eq :false (plist-get result :written)))
      (should (equal (make-list 7 "would-create") (gnosis-test-cards--statuses result)))
      (should (equal '(1 2 1 1 1 1 2)
                     (mapcar (lambda (card) (plist-get card :themata))
                             (plist-get result :cards))))
      (should (= 0 (gnosis-test-cards--count)))
      ;; The real call afterwards still creates them.
      (should (equal (make-list 7 "created")
                     (gnosis-test-cards--statuses
                      (gnosis-agent-content-create gnosis-test-cards--batch))))
      ;; Not inside another transaction, which could not roll back alone.
      (should-error (gnosis-sqlite-with-transaction gnosis-db
                      (gnosis-agent-content-create gnosis-test-cards--batch :dry-run t))))))

(ert-deftest gnosis-test-cards-errors-name-each-card-and-write-nothing ()
  "Every invalid card says why; with any invalid, nothing is written."
  (gnosis-test-with-db
    (let* ((result (gnosis-agent-content-create
                    (vector
                     (list :type "basic" :question "Fine?" :answer ["Yes"])
                     (list :type "mcq" :question "Pick" :choices ["a" "b"] :answer ["c"])
                     (list :type "basic" :question "Q" :hints ["h"] :choices ["x"] :answer ["A"])
                     (list :type "flashcard" :question "Q" :answer ["A"])
                     (list :type "basic" :question "Q" :answer ["A"] :deck "Bio")
                     (list :type "cloze" :question "No blank here." :answer ["missing"])
                     (list :type "basic" :question "" :answer ["A"])
                     "not an object"
                     (list :type "basic" :question "Twin" :answer ["A"] :key "k")
                     (list :type "basic" :question "Other" :answer ["B"] :key "k")
                     (list :type "basic" :question "Tagged" :answer ["A"] :tags ["spaced-repetition"])
                     (list :type "basic" :question "Keyed" :answer ["A"] :key 5)
                     (list :type "basic" :question "Fine?" :answer ["Yes"])
                     nil
                     (list :type "image-region" :question "Q" :hints ["r"] :answer ["t"])
                     (list :type "basic" :question "Spaced" :answer ["A"] :key "  ")
                     (list :type "basic" :question "Bad key kept" :answer [] :key "mine"))))
           (cards (plist-get result :cards)))
      (should (eq :false (plist-get result :written)))
      (should (= 0 (gnosis-test-cards--count)))
      (should (equal '("would-create" "invalid" "invalid" "invalid" "invalid" "invalid"
                       "invalid" "invalid" "would-create" "invalid" "invalid" "invalid"
                       "invalid" "invalid" "invalid" "invalid" "invalid")
                     (gnosis-test-cards--statuses result)))
      (should (equal "mine" (plist-get (aref cards 16) :key)))
      (should (equal (number-sequence 0 16)
                     (mapcar (lambda (card) (plist-get card :index)) cards)))
      ;; The type's own message, without the editor's line prefix.
      (should (equal "Answer must be a single item, member of hypothesis."
                     (plist-get (aref cards 1) :error)))
      (dolist (case '((2 . "takes hints, not choices") (3 . "type must be one of")
                      (4 . "Unknown fields: :deck") (5 . "occurrence")
                      (6 . "question must be a nonempty string") (7 . "must be an object")
                      (9 . "key repeats card 8") (10 . "Org cannot preserve tag")
                      (11 . "key must be a nonempty string") (12 . "content repeats card 0")
                      (13 . "Card is empty") (14 . "model cards: gnosis_model_card_preview")
                      (15 . "key must be a nonempty string")))
        (should (string-match-p (cdr case) (plist-get (aref cards (car case)) :error)))))))

(ert-deftest gnosis-test-cards-must-survive-the-editor ()
  "A card is made only if the editor can open it and save it unchanged.
Text the editor's draft would split or misread is that card's error,
and the batch is not written; repeated tags are kept once."
  (gnosis-test-with-db
    (let* ((result (gnosis-agent-content-create
                    (vector (list :type "basic" :question "Two points?"
                                  :answer ["First point\n- Second point"])
                            (list :type "basic" :question "Intro\n** First point\nmore"
                                  :answer ["A"])
                            (list :type "basic" :question "Fine" :answer ["A"]))))
           (cards (plist-get result :cards)))
      (should (eq :false (plist-get result :written)))
      (should (= 0 (gnosis-test-cards--count)))
      (should (equal '("invalid" "invalid" "would-create") (gnosis-test-cards--statuses result)))
      (dotimes (i 2)
        (should (string-prefix-p "Its text cannot be kept: " (plist-get (aref cards i) :error)))))
    (let* ((result (gnosis-agent-content-create
                    (vector (list :type "basic" :question "Tags twice" :answer ["A"]
                                  :tags ["bio" "bio"]))))
           (id (string-to-number (aref (plist-get (aref (plist-get result :cards) 0) :ids) 0))))
      (should (equal '("created") (gnosis-test-cards--statuses result)))
      (should (equal '("bio") (gnosis-select 'tag 'thema-tag `(= thema-id ,id) t)))
      ;; What was made opens in the editor and reads back as it was saved.
      (should (= 1 (length (gnosis-agent-content--draft-themata (list id))))))))

(ert-deftest gnosis-test-cards-mixed-batches ()
  "New cards beside existing ones are written; any invalid card stops all."
  (gnosis-test-with-db
    (gnosis-agent-content-create (vector (list :type "basic" :question "Old" :answer ["A"])))
    (let ((saved nil))
      (let ((gnosis-save-hook (list (lambda (id) (push id saved)))))
        (let ((mixed (gnosis-agent-content-create
                      (vector (list :type "basic" :question "Old" :answer ["A"])
                              (list :type "basic" :question "New" :answer ["B"])))))
          (should (eq t (plist-get mixed :written)))
          (should (equal '("exists" "created") (gnosis-test-cards--statuses mixed)))
          ;; Views hear of the new thema after commit, not of the old.
          (should (equal (list (string-to-number
                                (aref (plist-get (aref (plist-get mixed :cards) 1) :ids) 0)))
                         saved))))
      (let ((stopped (gnosis-agent-content-create
                      (vector (list :type "basic" :question "Old" :answer ["A"])
                              (list :type "basic" :question "Newer" :answer ["C"])
                              (list :type "basic" :question "Bad" :answer [])))))
        (should (eq :false (plist-get stopped :written)))
        (should (equal '("exists" "would-create" "invalid") (gnosis-test-cards--statuses stopped)))
        (should (= 2 (gnosis-test-cards--count)))))))

(ert-deftest gnosis-test-cards-refuse-an-empty-or-oversized-call ()
  "A call takes one to the limit of cards."
  (gnosis-test-with-db
    (should-error (gnosis-agent-content-create []) :type 'user-error)
    (should-error (gnosis-agent-content-create '((:type "basic"))) :type 'user-error)
    (should-error (gnosis-agent-content-create
                   (make-vector (1+ gnosis-agent-content--create-limit)
                                (list :type "basic" :question "Q" :answer ["A"])))
                  :type 'user-error)))

(defun gnosis-test-cards--call (json)
  "Call a tool as an agent's JSON-RPC tools/call PARAMS JSON arrives.
Return the reply's decoded payload and whether it is an error."
  (let* ((params (json-parse-string json :object-type 'hash-table :array-type 'array
                                    :null-object :null :false-object :false))
         ;; The server is inside a request while a tool runs.
         (romacs-mcp--active (list :peer nil :message (romacs-mcp--object "id" 1)))
         (reply (romacs-mcp--call params t))
         (text (gethash "text" (aref (gethash "content" reply) 0))))
    (list (eq t (gethash "isError" reply))
          (condition-case nil
              (json-parse-string text :object-type 'plist :array-type 'array
                                 :null-object nil :false-object :false)
            (error text)))))

(ert-deftest gnosis-test-cards-through-the-mcp-tool ()
  "An agent's tools/call creates cards, retries idempotently, and dry-runs."
  (gnosis-test-with-db
    (let ((request "{\"name\": \"gnosis_cards_create\", \"arguments\": {%s\"cards\": [
                     {\"type\": \"basic\", \"question\": \"2 + 2?\", \"answer\": [\"4\"],
                      \"tags\": [\"math\"]},
                     {\"type\": \"mcq\", \"question\": \"Pick the prime\",
                      \"choices\": [\"4\", \"6\", \"7\"], \"answer\": [\"7\"],
                      \"parathema\": null}]}}"))
      (pcase-let ((`(,error ,result) (gnosis-test-cards--call
                                      (format request "\"dry-run\": true, "))))
        (should-not error)
        (should (equal ["would-create" "would-create"]
                       (vconcat (mapcar (lambda (c) (plist-get c :status))
                                        (plist-get result :cards)))))
        (should (= 0 (gnosis-test-cards--count))))
      (pcase-let ((`(,error ,result) (gnosis-test-cards--call (format request ""))))
        (should-not error)
        (should (eq t (plist-get result :written)))
        (should (= 2 (gnosis-test-cards--count)))
        (should (stringp (aref (plist-get (aref (plist-get result :cards) 0) :ids) 0))))
      (pcase-let ((`(,error ,result) (gnosis-test-cards--call (format request ""))))
        (should-not error)
        (should (equal '("exists" "exists") (gnosis-test-cards--statuses result)))
        (should (= 2 (gnosis-test-cards--count))))
      ;; An invalid card is the tool's ordinary result, not an error.
      (pcase-let ((`(,error ,result)
                   (gnosis-test-cards--call
                    "{\"name\": \"gnosis_cards_create\", \"arguments\": {\"cards\": [
                      {\"type\": \"basic\", \"question\": \"Q\", \"answer\": []}]}}")))
        (should-not error)
        (should (eq :false (plist-get result :written)))
        (should (equal '("invalid") (gnosis-test-cards--statuses result))))
      ;; Cards that are not an array are the protocol's invalid-params
      ;; error, raised before the tool runs; an empty array is the tool's.
      (should (equal '(romacs-mcp-protocol-error -32602 "Wrong argument type: cards" nil)
                     (condition-case err
                         (gnosis-test-cards--call
                          "{\"name\": \"gnosis_cards_create\", \"arguments\": {\"cards\": {}}}")
                       (romacs-mcp-protocol-error err))))
      (pcase-let ((`(,error ,result)
                   (gnosis-test-cards--call
                    "{\"name\": \"gnosis_cards_create\", \"arguments\": {\"cards\": []}}")))
        (should error)
        (should (string-match-p "cards must be an array of 1 to 500" result))))))

(provide 'gnosis-test-agent-cards)
;;; gnosis-test-agent-cards.el ends here
