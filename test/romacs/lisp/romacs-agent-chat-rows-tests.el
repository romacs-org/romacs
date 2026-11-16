;;; romacs-agent-chat-rows-tests.el --- A chat drawn from its turns' rows -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-turns)
  (require 'romacs-agent-ui))

(defconst romacs-agent-chat-rows-tests--long (make-string 3000 ?x)
  "An output longer than a row keeps.")

(defun romacs-agent-chat-rows-tests--records (turns)
  "TURNS Claude turns' records: a run note, then a prompt, a call with its
result (the last turn's long), a reply and its end a turn."
  (let ((seq 0) (records nil))
    (cl-flet ((add (key value) (push (romacs-mcp--object key value "seq" (cl-incf seq) "t" (+ 1794600000.0 (* 2 seq))) records)))
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
                                                                                                             "content" (if (= n turns) romacs-agent-chat-rows-tests--long "found"))))))
          (add "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                         "message" (romacs-mcp--object "id" (format "m%d" n)
                                                                       "content" (vector (romacs-mcp--object "type" "text" "text" (format "answer %d" n))))))
          (add "out" (romacs-mcp--object "type" "result" "result" (format "answer %d" n))))))
    (nreverse records)))

(defmacro romacs-agent-chat-rows-tests--with (turns &rest body)
  "BODY with `session', a Claude session of TURNS turns in a state root of
its own, its records in its journal and `records' them all."
  (declare (indent 1))
  `(let* ((root (make-temp-file "romacs-chat-rows-" t))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                       process-environment))
          (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-event-functions nil)
          (id "b1b1b1b1-2222-4333-8444-555566667777")
          (dir (expand-file-name id (romacs-agent--state-root)))
          (records (romacs-agent-chat-rows-tests--records ,turns)))
     (unwind-protect
         (progn
           (make-directory dir t)
           (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                     (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root "title" "rows"
                                                         "titleAuto" :false "argv" [] "options" (romacs-mcp--object)))
           (let ((coding-system-for-write 'utf-8-unix))
             (write-region (mapconcat (lambda (r) (concat (romacs-agent--json r) "\n")) records "")
                           nil (expand-file-name "journal.jsonl" dir) nil 'silent))
           (let ((session (romacs-agent--make :id id :harness 'claude :provider "claude" :dir dir :cwd root :title "rows"
                                              :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))))
             (puthash id session romacs-agent--sessions)
             ,@body))
       (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
       (delete-directory root t))))

(defun romacs-agent-chat-rows-tests--drawn (session &optional records-only)
  "SESSION's chat transcript as a first show draws it (5 turns), from its
records alone with RECORDS-ONLY, and the buffer."
  (let ((buffer (romacs-agent-ui--make-chat session)))
    (with-current-buffer buffer
      (setq romacs-agent-ui--turns 5)
      (cl-letf (((symbol-function 'romacs-agent-turns-page)
                 (if records-only #'ignore (symbol-function 'romacs-agent-turns-page))))
        (romacs-agent-ui--fill buffer))
      (setf (romacs-agent-buffer session) nil)
      (cons (buffer-substring-no-properties (point-min) romacs-agent-ui--end) buffer))))

(ert-deftest romacs-agent-chat-from-rows-reads-as-from-records ()
  ;; Drawn from its rows, a chat reads as drawn from its records, its
  ;; finished turns' records never read; a long output opens from its
  ;; row's pointer; a turn with no row yet is drawn from its records.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-chat-rows-tests--with 12
    (should (= (romacs-agent-turns-fill session) 12))
    (let* ((from-records (romacs-agent-chat-rows-tests--drawn session t))
           (reads nil)
           (from-rows (cl-letf* ((read (symbol-function 'romacs-agent--read-journal-file))
                                 ((symbol-function 'romacs-agent--read-journal-file)
                                  (lambda (session function from end skip)
                                    (push (cons from end) reads)
                                    (funcall read session function from end skip))))
                        (romacs-agent-chat-rows-tests--drawn session))))
      (should (string-prefix-p "▴ Earlier turns" (car from-rows)))
      (should (equal (car from-rows) (car from-records)))
      (should (string-search "answer 12" (car from-rows)))
      (should-not (string-search "answer 7" (car from-rows)))
      ;; Only what comes after the rows was read: from the journal's end.
      (should (equal reads (list (cons (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" dir))) nil))))
      ;; The long output is read from the row's pointer when opened: its
      ;; call's records alone.
      (with-current-buffer (cdr from-rows)
        (setq reads nil)
        (cl-letf* ((read (symbol-function 'romacs-agent--read-journal-file))
                   ((symbol-function 'romacs-agent--read-journal-file)
                    (lambda (session function from end skip)
                      (push (cons from end) reads)
                      (funcall read session function from end skip))))
          (should (equal (romacs-agent-ui--payload-output (gethash "u12" romacs-agent-ui--payloads))
                         romacs-agent-chat-rows-tests--long)))
        ;; Its start (the whole input) and its end (the output), each a
        ;; bounded read.
        (should (<= 1 (length reads) 2))
        (should (cl-every (lambda (read) (and (> (car read) 0) (integerp (cdr read)))) reads)))
      (kill-buffer (cdr from-rows))
      (kill-buffer (cdr from-records)))
    ;; A turn ended after its row was written: drawn from its records,
    ;; after the rows' turns, which are as many as ever.
    (sqlite-execute (romacs-agent-turns--db) "delete from turns where session = ? and first_seq > 52" (list id))
    (let ((drawn (romacs-agent-chat-rows-tests--drawn session)))
      (should (string-search "> question 7\n" (car drawn)))
      (should-not (string-search "> question 6\n" (car drawn)))
      (should (string-search "answer 11\n" (car drawn)))
      (should (string-suffix-p (substring (car (romacs-agent-chat-rows-tests--drawn session t))
                                          (string-search "> question 12" (car (romacs-agent-chat-rows-tests--drawn session t))))
                               (car drawn)))
      (kill-buffer (cdr drawn)))))

(ert-deftest romacs-agent-chat-from-rows-of-a-short-session ()
  ;; Fewer turns than a chat draws: no earlier row, what came before its
  ;; first turn drawn from its records, the same chat.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-chat-rows-tests--with 3
    (should (= (romacs-agent-turns-fill session) 3))
    (let ((rows (romacs-agent-chat-rows-tests--drawn session))
          (records (romacs-agent-chat-rows-tests--drawn session t)))
      (should-not (string-search "Earlier turns" (car rows)))
      (should (string-search "question 1" (car rows)))
      (should (equal (car rows) (car records)))
      (kill-buffer (cdr rows))
      (kill-buffer (cdr records)))))

(ert-deftest romacs-agent-chat-row-questions-as-asked ()
  ;; A row's questions come back as the layer asks them.
  (let ((question (make-hash-table :test #'equal)) (notes (make-hash-table :test #'equal)))
    (puthash "Tail" "the last turns" notes)
    (puthash "question" "Which way?" question)
    (puthash "header" "Approach" question)
    (puthash "options" ["Tail" "Virtual"] question)
    (puthash "notes" notes question)
    (should (equal (romacs-agent-ui--item-questions (vector question))
                   '(("Which way?" :header "Approach" :options ("Tail" "Virtual") :notes (("Tail" . "the last turns")) :multi nil))))))

(ert-deftest romacs-agent-chat-row-texts-and-no-reads ()
  ;; A row's call draws from the texts its row keeps, never its records:
  ;; an edit's counts and a search's matches as the fill drew them, and an
  ;; output it does not hold only pointed at.  Its prompt keeps its request.
  (let* ((edit (make-hash-table :test #'equal)) (grep (make-hash-table :test #'equal))
         (input (make-hash-table :test #'equal))
         (row (list :first-seq 2 :last-seq 9 :t 100.0 :prompt "go" :request 7 :duration 3.0 :status "completed"
                    :items (vector edit grep))))
    (puthash "file_path" "/p/a.el" input)
    (puthash "old_string" 12 input)
    (dolist (pair `(("kind" . "tool") ("id" . "e1") ("name" . "Edit") ("state" . "completed") ("seq" . [3 4])
                    ("input" . ,input) ("row" . "edit a.el +3 −2") ("output_bytes" . 10) ("output" . "ok")))
      (puthash (car pair) (cdr pair) edit))
    (dolist (pair '(("kind" . "tool") ("id" . "g1") ("name" . "Grep") ("state" . "completed") ("seq" . [5 6])
                    ("row" . "grep zebra") ("ended" . "grep zebra · 400 matches") ("output_bytes" . 90000)))
      (puthash (car pair) (cdr pair) grep))
    (cl-letf (((symbol-function 'romacs-agent-turns-item) (lambda (&rest _) (error "Read a record")))
              ((symbol-function 'romacs-agent--read-journal) (lambda (&rest _) (error "Read a record"))))
      (let ((events (romacs-agent-ui--row-events 'session row)))
        (should (eql (plist-get (car events) :request) 7))
        (should (equal (plist-get (nth 1 events) :row) "edit a.el +3 −2"))
        (let ((ended (seq-find (lambda (e) (and (equal (plist-get e :id) "g1") (eq (plist-get e :state) 'completed))) events)))
          (should (equal (plist-get ended :ended) "grep zebra · 400 matches"))
          (should-not (plist-get ended :output))
          (should (plist-get (plist-get ended :output-ref) :row)))))
    ;; The texts a fill keeps are the renderers' own.
    (let ((full (make-hash-table :test #'equal)))
      (puthash "command" "ls -la" full)
      (should (equal (plist-get (romacs-agent-rows-texts "Bash" full "a\nb") :row)
                     (funcall (plist-get (romacs-agent-rows-get "Bash") :row) full))))))

(ert-deftest romacs-agent-chat-row-messages-images-answers ()
  ;; A row's prompt keeps its files and who said what; an answer the
  ;; harness got comes back as the labels picked, whole.
  (let* ((from (make-hash-table :test #'equal)) (message (make-hash-table :test #'equal))
         (answers (make-hash-table :test #'equal))
         (row (list :first-seq 2 :last-seq 3 :t 1.0 :prompt "see" :images ["/tmp/a.png"]
                    :messages (vector message) :items [])))
    (puthash "session" "s-1" from)
    (puthash "name" "lead" from)
    (dolist (pair `(("id" . "m1") ("from" . ,from) ("kind" . "note") ("text" . "hi")))
      (puthash (car pair) (cdr pair) message))
    (let ((input (car (romacs-agent-ui--row-events 'session row))))
      (should (equal (plist-get input :images) '("/tmp/a.png")))
      (let ((card (car (plist-get input :messages))))
        (should (equal (plist-get card :text) "hi"))
        (should (equal (plist-get (plist-get card :from) :name) "lead"))))
    (puthash "Which?" "Research, then implement, B" answers)
    (should (equal (romacs-agent-ui--answer-labels
                    '(("Which?" :options ("Research, then implement" "B" "C"))) answers)
                   '(("Which?" "Research, then implement" "B"))))
    (puthash "Which?" "my own words" answers)
    (should (equal (romacs-agent-ui--answer-labels '(("Which?" :options ("A"))) answers)
                   '(("Which?" "my own words"))))
    ;; A label inside another is not picked with it; words of one's own
    ;; that name a label are kept as they came.
    (puthash "Which?" "Read all, Test" answers)
    (should (equal (romacs-agent-ui--answer-labels '(("Which?" :options ("Read" "Read all" "Test"))) answers)
                   '(("Which?" "Read all" "Test"))))
    (puthash "Which?" "Please Test only after lunch" answers)
    (should (equal (romacs-agent-ui--answer-labels '(("Which?" :options ("Read" "Test"))) answers)
                   '(("Which?" "Please Test only after lunch"))))
    (puthash "Which?" "Read, Read all" answers)
    (should (equal (romacs-agent-ui--answer-labels '(("Which?" :options ("Read" "Read all"))) answers)
                   '(("Which?" "Read" "Read all"))))))

(ert-deftest romacs-agent-chat-row-call-opens-whole ()
  ;; A row's call opened draws its body from the whole input its turn's
  ;; records hold, not the part its row kept.
  (let* ((kept (make-hash-table :test #'equal)) (whole (make-hash-table :test #'equal)))
    (puthash "old_string" 9 kept)
    (puthash "old_string" "one\ntwo\n" whole)
    (cl-letf (((symbol-function 'romacs-agent-turns-item)
               (lambda (_session id) (and (equal id "e1") (list :input whole :output "done")))))
      (let ((call (romacs-agent-ui--payload-call
                   (list :call (list "Edit" kept) :ref (list :session 'session :id "e1" :row t)))))
        (should (eq (car call) whole))
        (should (equal (cdr call) "done"))))
    ;; Kept live, it is what it was.
    (should (equal (romacs-agent-ui--payload-call (list :call (list "Bash" kept) :output "out")) (cons kept "out")))
    ;; The fill keeps the renderers' texts.
    (should (eq romacs-agent-turns-tool-texts-function #'romacs-agent-rows-texts))))

;;; romacs-agent-chat-rows-tests.el ends here
