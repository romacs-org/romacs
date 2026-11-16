;;; romacs-agent-turns-tests.el --- A session's turns, a row each -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-turns))

(defun romacs-agent-turns-tests--records (turns)
  "TURNS turns' records, each a table: a run note, then a prompt, a tool's
call and result, an answer and a result a turn."
  (let ((seq 0) (records nil))
    (cl-flet ((add (key value) (push (romacs-mcp--object key value "seq" (cl-incf seq) "t" (+ 1794600000.25 seq)) records)))
      (add "note" (romacs-mcp--object "type" "run"))
      (dotimes (i turns)
        (let ((n (1+ i)))
          (add "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                        "romacsRequest" n "type" "user"))
          (add "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "id" (format "t%d" n)
                                                                       "content" (vector (romacs-mcp--object "type" "tool_use" "id" (format "u%d" n) "name" "Bash"
                                                                                                             "input" (romacs-mcp--object "command" (format "grep %d" n)))))))
          (add "out" (romacs-mcp--object "type" "user" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "content" (vector (romacs-mcp--object "type" "tool_result" "tool_use_id" (format "u%d" n)
                                                                                                             "content" "found")))))
          (add "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "id" (format "m%d" n)
                                                                       "content" (vector (romacs-mcp--object "type" "text" "text" (format "answer %d about zebras" n))))))
          (add "out" (romacs-mcp--object "type" "result" "result" (format "answer %d about zebras" n))))))
    (nreverse records)))

(ert-deftest romacs-agent-turns-a-row-a-turn-and-the-tail-from-them ()
  ;; Folded as they come, a session's turns are rows; where its last turns
  ;; begin comes from them, the place the scan of its journal finds; its
  ;; replies are searched.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-turns-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (id "a1a1a1a1-2222-4333-8444-555566667777")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (records (romacs-agent-turns-tests--records 12)))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root "title" "turns"
                                                        "titleAuto" :false "argv" [] "options" (romacs-mcp--object)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) records "")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root
                                             :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
            (puthash id session romacs-agent--sessions)
            ;; Folded as they come, live: a moment after each turn's end its
            ;; session's rows are caught up (here, at once).
            (let ((due nil))
              (cl-letf (((symbol-function 'run-with-timer) (lambda (&rest _) nil))
                        ((symbol-function 'romacs-agent-turns-fill-soon) (lambda (ids) (setq due ids))))
                (dolist (record records) (romacs-agent--record session record))
                (should (equal romacs-agent-turns--due (list id))))
              (ignore due))
            (should (= (romacs-agent-turns-count session) 0))
            (setq romacs-agent-turns--due nil)
            (should (= (romacs-agent-turns-fill session) 12))
            (should (= (romacs-agent-turns-count session) 12))
            ;; Each row at its prompt's time, not the fill's.
            (should (= (caar (sqlite-select (romacs-agent-turns--db) "select t from turns where session = ? and first_seq = 2" (list id)))
                       (+ 1794600000.25 2)))
            (let ((rows (sqlite-select (romacs-agent-turns--db)
                                       "select first_seq, last_seq, prompt, reply, tools from turns where session = ? order by first_seq limit 1"
                                       (list id))))
              (should (equal rows '((2 6 "question 1" "answer 1 about zebras" "Bash · grep 1 · ok")))))
            ;; What a chat draws: the page, newest first, each turn's items.
            (let* ((page (romacs-agent-turns-page session nil 5)) (row (car (plist-get page :rows))))
              (should (= (length (plist-get page :rows)) 5))
              (should (plist-get page :more))
              (should (= (plist-get page :after) 61))
              (should (equal (plist-get row :status) "completed"))
              (should (= (plist-get row :request) 12))
              (should (equal (mapcar (lambda (item) (gethash "kind" item)) (plist-get row :items)) '("tool" "text")))
              (let ((tool (aref (plist-get row :items) 0)) (text (aref (plist-get row :items) 1)))
                (should (equal (gethash "id" tool) "u12"))
                (should (equal (gethash "state" tool) "completed"))
                (should (equal (gethash "command" (gethash "input" tool)) "grep 12"))
                (should (equal (gethash "output" tool) "found"))
                (should (equal (gethash "seq" tool) [58 59]))
                (should (equal (gethash "text" text) "answer 12 about zebras"))
                (should (equal (gethash "message" text) "m12")))
              (should-not (plist-get (romacs-agent-turns-page session 7 5) :more))
              (should (equal (plist-get (romacs-agent-turns-item session "u12") :output) "found"))
              (should (equal (gethash "command" (plist-get (romacs-agent-turns-item session "u12") :input)) "grep 12"))
              (should-not (romacs-agent-turns-item session "nothing")))
            ;; The tail from the rows: where the journal's scan says.
            (let ((from-rows (romacs-agent--turns-tail session 3))
                  (scanned (romacs-agent--journal-file-tail session 3)))
              (should from-rows)
              (should (equal (car from-rows) (car scanned)))
              ;; With no bound, the same place; within too few bytes, none.
              (should (equal (romacs-agent--turns-tail session 3 'none) from-rows))
              (should-not (romacs-agent--turns-tail session 3 10)))
            ;; Fewer turns than asked, or none earlier: no cut.
            (should-not (romacs-agent--turns-tail session 12))
            ;; A journal's own tail is read from its end, its rows not asked.
            (let ((scanned (romacs-agent--journal-file-tail session 3)))
              (cl-letf (((symbol-function 'romacs-agent--turns-tail) (lambda (&rest _) (error "Rows asked"))))
                (should (equal (romacs-agent--journal-tail session 3) scanned))))
            (should (equal (sort (mapcar #'cadr (romacs-agent-turns-search "zebras" 50)) #'<) (number-sequence 2 57 5)))
            ;; Turns that came with no editor there (records with no rows):
            ;; caught up from where the rows end, the others not written again.
            (sqlite-execute (romacs-agent-turns--db) "delete from turns where session = ? and first_seq > 42" (list id))
            (should (= (romacs-agent-turns-count session) 9))
            (should (= (romacs-agent-turns-fill session) 3))
            (should (= (romacs-agent-turns-count session) 12))
            (should (= (romacs-agent-turns-fill session) 0))
            ;; A batch's session loaded with an older seq than its records
            ;; (its supervisor running): its rows are caught up all the same.
            (sqlite-execute (romacs-agent-turns--db) "delete from turns where session = ? and first_seq > 52" (list id))
            (cl-letf (((symbol-function 'romacs-agent--load)
                       (lambda (_dir) (setf (romacs-agent-seq session) 46) (puthash id session romacs-agent--sessions)))
                      ((symbol-function 'princ) #'ignore))
              (let ((command-line-args-left (list id))) (romacs-agent-turns-fill-batch)))
            (should (= (romacs-agent-turns-count session) 12))
            ;; Its records moved to SQLite: filled from there, every row.
            (let ((romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal)))
              (should (romacs-agent-events-import session))
              (sqlite-execute (romacs-agent-turns--db) "delete from turns where session = ?" (list id))
              (should (= (romacs-agent-turns-fill session) 12))
              (should (= (romacs-agent-turns-count session) 12)))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

;; A fill draws the rows it writes where rows are drawn, not in the editor;
;; a redraw draws what has no form of this version.
(ert-deftest romacs-agent-turns-a-fill-draws-its-rows-off-the-editor ()
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (require 'romacs-agent-ui)
  (let* ((root (make-temp-file "romacs-turns-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-turns--made nil) (romacs-agent-drawn--made nil)
         (romacs-agent-events-store nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (id "a1a1a1a1-2222-4333-8444-5555666677dd")
         (dir (expand-file-name id (romacs-agent--state-root))))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root "title" "turns"
                                                        "titleAuto" :false "argv" [] "options" (romacs-mcp--object)))
          (cl-flet ((journal (turns)
                      (let ((coding-system-for-write 'utf-8-unix))
                        (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) (romacs-agent-turns-tests--records turns) "")
                                      nil (expand-file-name "journal.jsonl" dir) nil 'silent)))
                    (session () (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root
                                                    :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))
                    (forms () (sqlite-select (romacs-agent-turns--db) "select first_seq, version from turn_drawn where session = ? order by first_seq" (list id))))
            ;; In the editor: rows, no forms.
            (journal 1)
            (should (= (romacs-agent-turns-fill (session)) 1))
            (should-not (ignore-errors (forms)))
            ;; Where rows are drawn: each row its form, in its transaction.
            (journal 3)
            (let ((romacs-agent-turns-draw-function #'romacs-agent-ui-draw-turn))
              (should (= (romacs-agent-turns-fill (session)) 2))
              (should (equal (forms) `((7 ,romacs-agent-drawn-version) (12 ,romacs-agent-drawn-version))))
              (let ((form (caar (sqlite-select (romacs-agent-turns--db) "select form from turn_drawn where session = ? and first_seq = 7" (list id)))))
                (with-temp-buffer
                  (romacs-agent-drawn-insert form)
                  (should (string-search "answer 2 about zebras" (buffer-string)))))
              ;; The first, drawn by none, and one of an older drawing: drawn again.
              (sqlite-execute (romacs-agent-turns--db) "update turn_drawn set version = 0 where session = ? and first_seq = 12" (list id))
              (should (= (romacs-agent-turns-redraw (session)) 2))
              (should (equal (mapcar #'car (forms)) '(2 7 12)))
              (should (cl-every (lambda (row) (eql (cadr row) romacs-agent-drawn-version)) (forms)))
              (should (= (romacs-agent-turns-redraw (session)) 0)))
            ;; A drawing that fails leaves its row, with no form.
            (sqlite-execute (romacs-agent-turns--db) "delete from turn_drawn")
            (let ((romacs-agent-turns-draw-function (lambda (&rest _) (error "Broken"))))
              (should (= (romacs-agent-turns-redraw (session)) 0))
              (should-not (forms))
              (should (= (romacs-agent-turns-count (session)) 3)))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(ert-deftest romacs-agent-turns-an-item-keeps-a-bounded-input ()
  ;; Long strings cut; an edit's bodies held as their sizes, at any depth.
  (let* ((long (make-string 3000 ?a))
         (input (romacs-mcp--object "file_path" "/x" "old_string" "é" "content" long
                                    "edits" (vector (romacs-mcp--object "new_string" "abc"))))
         (edit (romacs-agent-turns--bounded input t))
         (other (romacs-agent-turns--bounded input nil)))
    (should (equal (gethash "file_path" edit) "/x"))
    (should (= (gethash "old_string" edit) 2))
    (should (= (gethash "content" edit) 3000))
    (should (= (gethash "new_string" (aref (gethash "edits" edit) 0)) 3))
    (should (= (length (gethash "content" other)) (1+ romacs-agent-turns--cut)))
    (should (equal (gethash "old_string" other) "é"))))

(ert-deftest romacs-agent-turns-asks-and-inputs-as-items ()
  ;; A question's ask, as JSON its view reads; a tool's ask with its input;
  ;; an input written into the turn with its images and who said it.
  (let ((turn (list :items nil :by-id (make-hash-table :test #'equal))))
    (romacs-agent-turns--item turn (list :type 'approval :seq 3 :time 1.0 :request-id "r1" :tool "AskUserQuestion" :user t
                                         :questions '(("Which?" :header "Pick" :options ("A" "B") :notes (("A" . "the first")) :multi nil))))
    (romacs-agent-turns--item turn (list :type 'answered :seq 4 :request-id "r1" :behavior "allow"
                                         :answers (romacs-mcp--object "Which?" "A")))
    (romacs-agent-turns--item turn (list :type 'approval :seq 5 :request-id "r2" :tool "Bash" :description "list"
                                         :input (romacs-mcp--object "command" "ls")))
    (romacs-agent-turns--item turn (list :type 'input :seq 6 :text "also" :steer t :request 7 :images '("/a.png")
                                         :messages (list (list :id "m" :from "parent" :kind 'message :text "hi" :wake t))))
    (let* ((items (romacs-agent-turns--ended-items turn))
           (parsed (json-parse-string (romacs-agent--json items) :object-type 'hash-table :null-object nil :false-object nil))
           (ask (aref parsed 0)) (tool (aref parsed 1)) (input (aref parsed 2))
           (question (aref (gethash "questions" ask) 0)))
      (should (equal (gethash "question" question) "Which?"))
      (should (equal (gethash "options" question) ["A" "B"]))
      (should (equal (gethash "A" (gethash "notes" question)) "the first"))
      (should-not (gethash "multi" question))
      (should (eq (gethash "user" ask) t))
      (should (equal (gethash "Which?" (gethash "answers" ask)) "A"))
      (should (equal (gethash "seq" ask) [3 4]))
      (should (equal (gethash "command" (gethash "input" tool)) "ls"))
      (should (equal (gethash "description" tool) "list"))
      (should-not (gethash "plan" tool))
      (should (equal (gethash "images" input) ["/a.png"]))
      (should (equal (gethash "from" (aref (gethash "messages" input) 0)) "parent"))
      (should (equal (gethash "kind" (aref (gethash "messages" input) 0)) "message"))))
  ;; Written and read back: a sender and a wake, plists, as objects.
  (let* ((turn (list :items nil :by-id (make-hash-table :test #'equal)))
         (message (list :id "m" :kind 'message :text "hi"
                        :from (list :session "s1" :name "lead" :address "a@b")
                        :wake (list :when 'end :after 3))))
    (romacs-agent-turns--item turn (list :type 'input :seq 2 :text "x" :messages (list message)))
    (let* ((text (romacs-agent--json (romacs-agent-turns--ended-items turn)))
           (read (aref (gethash "messages" (aref (romacs-agent-turns--parse text) 0)) 0)))
      (should (equal (gethash "name" (gethash "from" read)) "lead"))
      (should (equal (gethash "address" (gethash "from" read)) "a@b"))
      (should (equal (gethash "when" (gethash "wake" read)) "end"))
      (should (= (gethash "after" (gethash "wake" read)) 3)))))

(ert-deftest romacs-agent-turns-a-tool-keeps-its-rows-texts ()
  ;; Its row's texts made from its whole input and output, while the fill
  ;; has them: once as it starts, again as it ends.
  (let ((turn (list :items nil :by-id (make-hash-table :test #'equal) :inputs (make-hash-table :test #'equal)))
        (romacs-agent-turns-tool-texts-function
         (lambda (name input output)
           (list :row (format "%s %s" name (gethash "pattern" input))
                 :ended (and output (format "%d matches" (length (split-string output "\n" t))))))))
    (romacs-agent-turns--item turn (list :type 'tool :state 'started :seq 2 :id "g" :name "Grep"
                                         :input (romacs-mcp--object "pattern" (make-string 3000 ?x))))
    (let ((item (gethash "g" (plist-get turn :by-id))))
      (should (= (length (gethash "row" item)) 3005))
      (should (eq (gethash "ended" item) :null))
      (romacs-agent-turns--item turn (list :type 'tool :state 'completed :seq 3 :id "g" :output "a\nb\nc\n"))
      (should (equal (gethash "ended" item) "3 matches"))
      ;; The input it keeps is bounded all the same.
      (should (= (length (gethash "pattern" (gethash "input" item))) (1+ romacs-agent-turns--cut))))
    ;; A search whose query comes only as it ends: its texts and input from then.
    (romacs-agent-turns--item turn (list :type 'tool :state 'started :seq 4 :id "w" :name "Grep"
                                         :input (romacs-mcp--object "pattern" "")))
    (romacs-agent-turns--item turn (list :type 'tool :state 'completed :seq 5 :id "w" :output "a\n"
                                         :input (romacs-mcp--object "pattern" "zebra")))
    (let ((item (gethash "w" (plist-get turn :by-id))))
      (should (equal (gethash "row" item) "Grep zebra"))
      (should (equal (gethash "pattern" (gethash "input" item)) "zebra")))))

(ert-deftest romacs-agent-turns-codex-answers-and-interrupts ()
  ;; Codex's answers, by question id, kept as Claude's are, by question; a
  ;; turn it interrupted ends interrupted, not failed.
  (require 'romacs-agent-codex)
  (let ((params (romacs-mcp--object "questions" (vector (romacs-mcp--object "id" "q1" "question" "Which?")
                                                        (romacs-mcp--object "id" "q2" "question" "Unanswered?"))))
        (result (romacs-mcp--object "answers" (romacs-mcp--object "q1" (romacs-mcp--object "answers" ["A" "B"])))))
    (let ((answers (romacs-agent--codex-given-answers params result)))
      (should (equal (gethash "Which?" answers) "A, B"))
      (should-not (gethash "Unanswered?" answers)))
    (should-not (romacs-agent--codex-given-answers params (romacs-mcp--object "decision" "accept"))))
  (let ((session (romacs-agent--make :id "c" :harness 'codex :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (should (equal (romacs-agent-turns--status (romacs-agent--codex-done session (romacs-mcp--object "status" "interrupted")))
                   "interrupted"))
    (should (equal (romacs-agent-turns--status (romacs-agent--codex-done session (romacs-mcp--object "status" "failed")))
                   "failed"))))

(ert-deftest romacs-agent-turns-a-long-output-is-read-when-asked ()
  ;; A tool's output over the inline size is not in its row: read from
  ;; its records, by its id.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-turns-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (id "a1a1a1a1-2222-4333-8444-555566669999")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (long (make-string 5000 ?z))
         (records (list (romacs-mcp--object "seq" 1 "t" 10.0 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "go" "role" "user")
                                                                                       "romacsRequest" 1 "type" "user"))
                        (romacs-mcp--object "seq" 2 "t" 11.0 "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                                                        "message" (romacs-mcp--object "id" "m1" "content" (vector (romacs-mcp--object "type" "tool_use" "id" "big" "name" "Bash" "input" (romacs-mcp--object "command" "cat"))))))
                        (romacs-mcp--object "seq" 3 "t" 12.0 "out" (romacs-mcp--object "type" "user" "parent_tool_use_id" :null
                                                                                        "message" (romacs-mcp--object "content" (vector (romacs-mcp--object "type" "tool_result" "tool_use_id" "big" "content" long)))))
                        (romacs-mcp--object "seq" 4 "t" 13.0 "out" (romacs-mcp--object "type" "result" "result" "done")))))
    (unwind-protect
        (progn
          (make-directory dir t)
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) records "")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root
                                             :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
            (puthash id session romacs-agent--sessions)
            (should (= (romacs-agent-turns-fill session) 1))
            (let ((tool (aref (plist-get (car (plist-get (romacs-agent-turns-page session) :rows)) :items) 0)))
              (should (= (gethash "output_bytes" tool) 5000))
              (should-not (gethash "output" tool)))
            (should (= (plist-get (car (plist-get (romacs-agent-turns-page session) :rows)) :duration) 3.0))
            (should (equal (plist-get (romacs-agent-turns-item session "big") :output) long))
            ;; Its id read otherwise in a copy (a harness that numbers ids by
            ;; turn): the only tool its completing record completes.
            (sqlite-execute (romacs-agent-turns--db) "update turns set items = replace(items, '\"big\"', '\"1:big\"') where session = ?" (list id))
            (should (equal (plist-get (romacs-agent-turns-item session "1:big") :output) long))
            ;; Its records moved to SQLite: the same, by seqs.
            (let ((romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal)))
              (should (romacs-agent-events-import session))
              (should (romacs-agent-events-p session))
              (should (equal (plist-get (romacs-agent-turns-item session "1:big") :output) long)))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(ert-deftest romacs-agent-turns-a-turn-cut-and-resumed-is-one-row ()
  ;; A turn its harness's exit cut, then resumed to its end with no new
  ;; prompt: one row, its reply the end's, written once it ended; its
  ;; search text there once.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-turns-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (id "a1a1a1a1-2222-4333-8444-555566668888")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (records (list (romacs-mcp--object "seq" 1 "t" 10.0 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "go" "role" "user")
                                                                                       "romacsRequest" 1 "type" "user"))
                        (romacs-mcp--object "seq" 2 "t" 11.0 "exit" 1)
                        (romacs-mcp--object "seq" 3 "t" 12.0 "note" (romacs-mcp--object "type" "run"))
                        (romacs-mcp--object "seq" 4 "t" 13.0 "out" (romacs-mcp--object "type" "result" "result" "finished after resume")))))
    (unwind-protect
        (progn
          (make-directory dir t)
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) (seq-take records 2) "")
                          nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root
                                             :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
            (puthash id session romacs-agent--sessions)
            ;; Cut: no row yet.
            (should (= (romacs-agent-turns-fill session) 0))
            (let ((coding-system-for-write 'utf-8-unix))
              (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) (nthcdr 2 records) "")
                            nil (expand-file-name "journal.jsonl" dir) t 'silent))
            (should (= (romacs-agent-turns-fill session) 1))
            (should (equal (sqlite-select (romacs-agent-turns--db) "select first_seq, last_seq, reply from turns where session = ?" (list id))
                           '((1 4 "finished after resume"))))
            (should (= (length (romacs-agent-turns-search "resume")) 1))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

;;; romacs-agent-turns-tests.el ends here
