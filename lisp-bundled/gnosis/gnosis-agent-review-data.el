;;; gnosis-agent-review-data.el --- Bounded local study values -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions
;; URL: https://thanosapollo.org/projects/gnosis

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Pure validation and tail replacement for session-local text questions.
;; Bounds apply to retained evidence, not just each provider response.  No
;; evidence is truncated to make room.  All external values are JSON plists,
;; vectors, strings, integers, t, :false or nil (JSON null).

;;; Code:

(require 'gnosis)
(require 'json)

(defconst gnosis-agent-review--max-bytes (* 2 1024 1024)
  "Maximum encoded bytes in a local study payload.")

(defun gnosis-agent-review--copy (value)
  "Copy JSON VALUE completely, including mutable strings and vectors."
  (cond ((stringp value) (substring-no-properties value))
        ((vectorp value) (vconcat (mapcar #'gnosis-agent-review--copy value)))
        ((consp value) (mapcar #'gnosis-agent-review--copy value))
        (t value)))

(defun gnosis-agent-review--object (value keys)
  "Require VALUE to contain exactly the unique keyword KEYS."
  (unless (and (proper-list-p value) (= (length value) (* 2 (length keys))))
    (user-error "Invalid agent review object"))
  (let (seen)
    (cl-loop for (key _value) on value by #'cddr do
             (unless (and (memq key keys) (not (memq key seen)))
               (user-error "Unknown or duplicate agent review field: %S" key))
             (push key seen)))
  value)

(defun gnosis-agent-review--text (value &optional empty)
  "Require bounded string VALUE, allowing blank text only with EMPTY."
  (unless (and (stringp value) (<= (string-bytes value) 32768)
               (or empty (not (string-empty-p (string-trim value)))))
    (user-error "Agent review text must be nonempty and at most 32768 bytes"))
  value)

(defun gnosis-agent-review--array (value limit function)
  "Validate vector VALUE of at most LIMIT entries with FUNCTION."
  (unless (and (vectorp value) (<= (length value) limit))
    (user-error "Agent review array exceeds its bound or is not a vector"))
  (mapc function value)
  value)

(defun gnosis-agent-review--json (value &optional depth)
  "Validate bounded JSON VALUE, starting at optional DEPTH."
  (let ((depth (or depth 0)))
    (when (> depth 12) (user-error "Agent context is nested too deeply"))
    (cond
     ((memq value '(nil t :false)) t)
     ((integerp value) t)
     ((stringp value) (gnosis-agent-review--text value t))
     ((vectorp value)
      (gnosis-agent-review--array
       value 512 (lambda (v) (gnosis-agent-review--json v (1+ depth)))))
     ((and (proper-list-p value) (<= (length value) 128)
           (zerop (% (length value) 2)))
      (let (seen)
        (cl-loop for (key v) on value by #'cddr do
                 (unless (and (keywordp key) (not (memq key seen)))
                   (user-error "Context requires unique keyword object fields"))
                 (push key seen)
                 (gnosis-agent-review--json v (1+ depth)))))
     (t (user-error "Not an agent review JSON value")))))

(defun gnosis-agent-review--type (question)
  "Return QUESTION's native type, including legacy semantic questions."
  (if (plist-member question :type) (plist-get question :type) "agent-eval"))

(defun gnosis-agent-review--answers (question)
  "Return QUESTION's answers without changing legacy captured values."
  (or (plist-get question :answer)
      (vector (plist-get question :reference-answer))))

(defun gnosis-agent-review--question (question)
  "Validate and return exact text QUESTION without normalization.
Require all fields, including explicit empty hints/tags vectors and nullable
parathema.  Source is the captured citation or excerpt, never a tool command."
  (gnosis-agent-review--object
   question (if (plist-member question :type)
                '(:id :type :question :answer :choices :rubric :hints :parathema :tags :source)
              '(:id :question :reference-answer :rubric :hints :parathema :tags :source)))
  (dolist (key '(:id :question :source))
    (gnosis-agent-review--text (plist-get question key)))
  (unless (<= (length (plist-get question :id)) 128)
    (user-error "Local question ID is too long"))
  (dolist (key '(:hints :tags))
    (gnosis-agent-review--array (plist-get question key) 32 #'gnosis-agent-review--text))
  (when (plist-get question :parathema)
    (gnosis-agent-review--text (plist-get question :parathema) t))
  (let* ((type (gnosis-agent-review--type question))
         (answers (gnosis-agent-review--answers question))
         (choices (plist-get question :choices)))
    (unless (member type '("basic" "cloze" "mcq" "mc-cloze" "agent-eval"))
      (user-error "Unsupported local question type"))
    (gnosis-agent-review--array answers 32 #'gnosis-agent-review--text)
    (unless (if (equal type "cloze") (> (length answers) 0) (= (length answers) 1))
      (user-error "Invalid number of local answers"))
    (when (plist-member question :type)
      (gnosis-agent-review--array choices 32 #'gnosis-agent-review--text)
      (if (member type '("mcq" "mc-cloze"))
          (unless (and (> (length choices) 1)
                       (= (length choices) (length (delete-dups (append choices nil))))
                       (member (aref answers 0) (append choices nil)))
            (user-error "Choices must be unique and include the answer"))
        (unless (zerop (length choices)) (user-error "This type has no choices"))))
    (when (member type '("cloze" "mc-cloze"))
      (unless (seq-every-p #'identity
                           (gnosis-cloze--spans (plist-get question :question) (append answers nil)))
        (user-error "Every cloze needs a disjoint occurrence in the question")))
    (gnosis--validate-agent-eval-fields
     type (plist-get question :question) (append (plist-get question :hints) nil)
     (append answers nil) (plist-get question :rubric)))
  question)

(defun gnosis-agent-review--questions (questions)
  "Validate unique QUESTIONS, a vector of at most 128 records."
  (gnosis-agent-review--array questions 128 #'gnosis-agent-review--question)
  (let ((ids (mapcar (lambda (q) (plist-get q :id)) (append questions nil))))
    (unless (= (length ids) (length (delete-dups (copy-sequence ids))))
      (user-error "Duplicate local question ID")))
  questions)

(defun gnosis-agent-review--find (data id)
  "Return the captured question with ID from DATA, or signal an error."
  (or (seq-find (lambda (q) (equal id (plist-get q :id))) (plist-get data :questions))
      (user-error "Unknown local question: %s" id)))

(defun gnosis-agent-review--evaluation (result)
  "Validate a provider evaluation RESULT with string verdict."
  (gnosis-agent-review--object result '(:verdict :explanation))
  (unless (member (plist-get result :verdict) '("pass" "fail" "ungradable"))
    (user-error "Invalid agent review verdict"))
  (gnosis-agent-review--text (plist-get result :explanation))
  result)

(defun gnosis-agent-review--plan (data plan)
  "Validate PLAN against immutable question registry DATA.
Return a fresh payload with its replacement tail and adaptation receipt.
A reused ID must have identical content, even if not previously presented."
  (gnosis-agent-review--object plan '(:agent-note :context :remaining :done))
  (gnosis-agent-review--text (plist-get plan :agent-note) t)
  (let ((context (plist-get plan :context)))
    (unless (or (null context) (consp context))
      (user-error "Agent context must be an object or null"))
    (gnosis-agent-review--json context))
  (let* ((questions (gnosis-agent-review--questions (plist-get plan :remaining)))
         (done (plist-get plan :done))
         (registry (append (plist-get data :questions) nil))
         (next (gnosis-agent-review--copy data)))
    (unless (and (memq done '(t :false)) (eq (zerop (length questions)) (eq done t)))
      (user-error "Done must agree with the replacement tail"))
    (mapc (lambda (q)
            (when (and (plist-get data :protocol)
                       (or (equal (plist-get q :id) (plist-get (plist-get data :current) :id))
                           (seq-some (lambda (e)
                                       (equal (plist-get q :id)
                                              (plist-get (plist-get e :question) :id)))
                                     (plist-get data :encounters))))
              (user-error "Adjustment cannot requeue a captured question; use a new ID"))
            (let ((old (seq-find (lambda (v) (equal (plist-get q :id) (plist-get v :id)))
                                 registry)))
              (if old
                  (unless (seq-every-p
                           (lambda (key) (equal (plist-get q key) (plist-get old key)))
                           (seq-filter #'keywordp old))
                    (user-error "Changed question requires a new ID"))
                (setq registry (append registry (list q))))))
          questions)
    (gnosis-agent-review--questions (vconcat registry))
    (setf (plist-get next :questions) (gnosis-agent-review--copy (vconcat registry))
          (plist-get next :context) (gnosis-agent-review--copy (plist-get plan :context))
          (plist-get next :agent-note) (gnosis-agent-review--copy (plist-get plan :agent-note))
          (plist-get next :phase) (if (eq done t) "done" "question")
          (plist-get next :current) nil
          (plist-get next :adaptations)
          (gnosis-agent-review--copy (vconcat (plist-get data :adaptations) (vector plan))))
    next))

(defun gnosis-agent-review--attempt (attempt)
  "Validate retained ungraded or provisional ATTEMPT."
  (gnosis-agent-review--object
   attempt '(:phase :occurrence :request-id :response :evaluation :error :plan :context))
  (unless (member (plist-get attempt :phase) '("evaluate" "adapt"))
    (user-error "Invalid attempt phase"))
  (gnosis-agent-review--text (plist-get attempt :request-id))
  (dolist (key '(:occurrence :response :error))
    (when (plist-get attempt key) (gnosis-agent-review--text (plist-get attempt key) t)))
  (when (plist-get attempt :evaluation)
    (gnosis-agent-review--evaluation (plist-get attempt :evaluation))))

(defun gnosis-agent-review--encounter-value (encounter)
  "Validate an accepted local ENCOUNTER without deriving any new grades."
  (gnosis-agent-review--object
   encounter '(:occurrence :question :response :help :source-viewed :evaluation :context :outcome :override))
  (gnosis-agent-review--text (plist-get encounter :occurrence))
  (gnosis-agent-review--question (plist-get encounter :question))
  (gnosis-agent-review--text (plist-get encounter :response) t)
  (gnosis-agent-review--array (plist-get encounter :help) 32 #'gnosis-agent-review--text)
  (gnosis-agent-review--evaluation (plist-get encounter :evaluation))
  (unless (and (member (plist-get encounter :outcome) '("success" "failure"))
               (member (plist-get (plist-get encounter :evaluation) :verdict) '("pass" "fail"))
               (memq (plist-get encounter :override) '(t :false))
               (memq (plist-get encounter :source-viewed) '(t :false)))
    (user-error "Invalid accepted local outcome")))

(defun gnosis-agent-review--receipt (receipt)
  "Validate an explicit native-save RECEIPT."
  (gnosis-agent-review--object receipt '(:session-id :question-id :thema-id))
  (dolist (key '(:session-id :question-id :thema-id))
    (gnosis-agent-review--text (plist-get receipt key)))
  (unless (string-match-p (rx string-start (in "1-9") (* digit) string-end)
                          (plist-get receipt :thema-id))
    (user-error "Saved thema ID must be a positive decimal string")))

(defun gnosis-agent-review--validate (data remaining)
  "Validate retained DATA and its outer REMAINING local IDs.
Refuse malformed data and total overflow rather than truncating history."
  (gnosis-agent-review--json data)
  (gnosis-agent-review--object
   data (append '(:phase :revision :goal :source :context :questions :current
                  :attempts :encounters :adaptations :saves :agent-note)
                (when (plist-member data :protocol) '(:protocol :proposal))))
  (when (plist-member data :protocol)
    (unless (eq (plist-get data :protocol) 2) (user-error "Unknown tutor protocol"))
    (when (equal (plist-get data :phase) "adapt")
      (user-error "Combined tutoring has no mandatory adaptation phase"))
    (when-let* ((proposal (plist-get data :proposal)))
      (gnosis-agent-review--object proposal '(:occurrence :response :outcome :remaining :plan))
      (dolist (key '(:occurrence :response :outcome))
        (gnosis-agent-review--text (plist-get proposal key) t))
      (gnosis-agent-review--array (plist-get proposal :remaining) 128 #'gnosis-agent-review--text)
      (gnosis-agent-review--plan data (plist-get proposal :plan))))
  (unless (and (member (plist-get data :phase) '("question" "adapt" "done"))
               (natnump (plist-get data :revision)))
    (user-error "Invalid local review phase or revision"))
  (dolist (key '(:goal :source)) (gnosis-agent-review--text (plist-get data key)))
  (gnosis-agent-review--text (plist-get data :agent-note) t)
  (gnosis-agent-review--questions (plist-get data :questions))
  (unless (or (null (plist-get data :context)) (consp (plist-get data :context)))
    (user-error "Agent context must be an object or null"))
  (unless (and (proper-list-p remaining) (<= (length remaining) 128))
    (user-error "Invalid local queue"))
  (unless (= (length remaining) (length (delete-dups (copy-sequence remaining))))
    (user-error "Duplicate local queue ID"))
  (mapc (lambda (id) (gnosis-agent-review--find data id)) remaining)
  (when (or (and (equal (plist-get data :phase) "done") remaining)
            (and (equal (plist-get data :phase) "question") (null remaining)))
    (user-error "Local queue disagrees with phase"))
  (gnosis-agent-review--array (plist-get data :attempts) 256 #'gnosis-agent-review--attempt)
  (gnosis-agent-review--array (plist-get data :encounters) 256 #'gnosis-agent-review--encounter-value)
  (gnosis-agent-review--array (plist-get data :saves) 128 #'gnosis-agent-review--receipt)
  (gnosis-agent-review--array
   (plist-get data :adaptations) 256
   (lambda (plan)
     (gnosis-agent-review--object plan '(:agent-note :context :remaining :done))
     (gnosis-agent-review--text (plist-get plan :agent-note) t)
     (unless (and (or (null (plist-get plan :context)) (consp (plist-get plan :context)))
                  (memq (plist-get plan :done) '(t :false))
                  (eq (eq (plist-get plan :done) t) (zerop (length (plist-get plan :remaining)))))
       (user-error "Invalid retained adaptation"))
     (gnosis-agent-review--questions (plist-get plan :remaining))))
  (mapc (lambda (encounter)
          (let ((question (plist-get encounter :question)))
            (unless (equal question (gnosis-agent-review--find data (plist-get question :id)))
              (user-error "Accepted question differs from its captured registry"))))
        (plist-get data :encounters))
  (when (and (equal (plist-get data :phase) "adapt")
             (zerop (length (plist-get data :encounters))))
    (user-error "Adaptation requires an accepted encounter"))
  (when-let* ((current (plist-get data :current)))
    (gnosis-agent-review--object
     current '(:id :occurrence :response :evaluation :outcome :help :source-viewed))
    (unless (and (equal (plist-get data :phase) "question")
                 (equal (plist-get current :id) (car remaining)))
      (user-error "Presented question no longer owns the queue"))
    (gnosis-agent-review--text (plist-get current :occurrence))
    (gnosis-agent-review--text (plist-get current :response) t)
    (when (plist-get current :evaluation)
      (gnosis-agent-review--evaluation (plist-get current :evaluation)))
    (unless (memq (plist-get current :source-viewed) '(t :false))
      (user-error "Invalid source-viewed marker"))
    (gnosis-agent-review--array (plist-get current :help) 32 #'gnosis-agent-review--text)
    (unless (member (plist-get current :outcome) '(nil "success" "failure"))
      (user-error "Invalid pending outcome")))
  (unless (<= (string-bytes (json-serialize data :false-object :false :null-object nil))
              gnosis-agent-review--max-bytes)
    (user-error "Local study evidence exceeds 2 MiB; pause and start another session"))
  data)

(provide 'gnosis-agent-review-data)
;;; gnosis-agent-review-data.el ends here
