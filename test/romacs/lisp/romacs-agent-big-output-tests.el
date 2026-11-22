;;; romacs-agent-big-output-tests.el --- A huge tool output never reaches the chat -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-big-output-tests--with (&rest body)
  "BODY in a chat of `session', its events drawn by `draw'."
  (declare (indent 0))
  `(let ((session (romacs-agent--make :id "B" :harness 'claude :state 'running
                                      :tools (make-hash-table :test #'equal)
                                      :streamed (make-hash-table :test #'equal)))
         (romacs-agent-approval-policy 'ask))
     (with-temp-buffer
       (romacs-agent-log-mode)
       (setq romacs-agent-log--session session
             romacs-agent-log--end (point-min-marker)
             romacs-agent-log--input (point-min-marker))
       (set-marker-insertion-type romacs-agent-log--end t)
       (cl-flet ((draw (&rest event) (romacs-agent-log-draw session event)))
         ,@body))))

(defun romacs-agent-big-output-tests--input (&rest pairs)
  (let ((input (make-hash-table :test #'equal)))
    (while pairs (puthash (pop pairs) (pop pairs) input))
    input))

(defun romacs-agent-big-output-tests--longest ()
  "The longest line of the chat, in characters."
  (save-excursion
    (goto-char (point-min))
    (let ((longest 0))
      (while (not (eobp))
        (setq longest (max longest (- (line-end-position) (point))))
        (forward-line 1))
      longest)))

(ert-deftest romacs-agent-big-output-glimpse-reads-ends ()
  (should (equal (romacs-agent-rows-glimpse "a\nb\nc\n\n  \n" 2 nil " \n") '(("a" "b") . 3)))
  ;; A count trims line breaks only: a blank line still counts.
  (should (= (romacs-agent-rows--lines "a\n \n") 2))
  (should (= (romacs-agent-rows--lines "a\n\n") 2))
  (should (= (romacs-agent-rows--lines "\n") 1))
  (should (= (romacs-agent-rows--lines "a\nb") 2))
  (should (equal (romacs-agent-rows-glimpse "abcdef\ngh" 5 3) '(("abc" "gh") . 2)))
  (should (equal (romacs-agent-rows-glimpse "" 5) '(nil . 0)))
  (should (equal (romacs-agent-rows-glimpse " \n\n" 5 nil " \n") '(nil . 0)))
  ;; Past the bound: its ends only, and no count.
  (let ((romacs-agent-rows-read-limit 10))
    (should (equal (romacs-agent-rows-glimpse "1\n2\n3\n4\n5\n6\n7\n8\n9\n" 2) '(("1" "2"))))
    (should (string-match-p "characters" (romacs-agent-rows-extent "1\n2\n3\n4\n5\n6\n7\n8\n9\n"))))
  (should (= (romacs-agent-rows--lines "one") 1))
  (should (equal (romacs-agent-rows--line "\n\n  first  \nsecond") "first")))

(ert-deftest romacs-agent-big-output-one-huge-line-is-one-row ()
  ;; The owner's case: a 57K-character answer on one line.  Its row says
  ;; what came of it; the answer itself stays out of the buffer, RET away.
  (dolist (call '(("mcp__romacs__romacs_orchestrator_capabilities")
                  ("mcp__other__thing" "q" "x")
                  ("mcp__romacs__romacs_unheard_of" "q" "x")
                  ("Bash" "command" "cat x")))
    (romacs-agent-big-output-tests--with
      (let ((output (if (string-suffix-p "capabilities" (car call))
                        (format "{\"providers\":[%s]}"
                                (mapconcat (lambda (_) (format "{\"providerInstanceId\":\"p\",\"enabled\":true,\"models\":[\"%s\"]}"
                                                               (make-string 14000 ?m)))
                                           (number-sequence 1 4) ","))
                      (make-string 57000 ?y)))
            (before nil))
        (draw :type 'tool :state 'started :id "a" :name (car call)
              :input (apply #'romacs-agent-big-output-tests--input (cdr call)))
        (setq before (buffer-size))
        (draw :type 'tool :state 'completed :id "a" :output output)
        (should (< (- (buffer-size) before) 400))
        (should (<= (romacs-agent-big-output-tests--longest) 240))
        (goto-char (point-min))
        (should (eq (cdr (get-text-property (text-property-not-all (point-min) (point-max) 'romacs-agent-log-full nil)
                                            'romacs-agent-log-full))
                    output))))))

(ert-deftest romacs-agent-big-output-a-long-command-is-its-row ()
  ;; A command that ended well is its row: how many lines it gave, or past
  ;; what the chat reads (millions of characters) its size; RET shows it.
  (dolist (lines '(20000 200000))
    (romacs-agent-big-output-tests--with
      (let ((output (concat (mapconcat (lambda (i) (format "line %d" i)) (number-sequence 1 lines) "\n")
                            "\n\nShell cwd was reset to /x\n")))
        (draw :type 'tool :state 'started :id "a" :name "Bash" :input (romacs-agent-big-output-tests--input "command" "make"))
        (draw :type 'tool :state 'completed :id "a" :output output)
        (let ((text (string-trim (buffer-substring-no-properties (point-min) (point-max)))))
          (should-not (string-search "\n" text))
          (should (string-match-p (if (romacs-agent-rows-readable-p output) "· [0-9]+ lines\\'" "characters\\'") text)))))))

(ert-deftest romacs-agent-big-output-a-failure-past-the-bound-says-its-size ()
  (let ((romacs-agent-rows-read-limit 1000)
        (output (mapconcat #'number-to-string (number-sequence 1 2000) "\n")))
    (romacs-agent-big-output-tests--with
      (draw :type 'tool :state 'started :id "a" :name "Bash" :input (romacs-agent-big-output-tests--input "command" "make"))
      (draw :type 'tool :state 'failed :id "a" :output output)
      (let ((text (buffer-substring-no-properties (point-min) (point-max))))
        ;; The answer's size, not the part read, and no count.
        (should (string-search (concat (romacs-agent-rows-size output) " in all") text))
        (should-not (string-match-p "more lines" text))))))

(ert-deftest romacs-agent-big-output-a-call-never-drawn-is-one-row ()
  ;; A completion whose start this chat never drew: its row, with a huge
  ;; error, is still one capped line.
  (romacs-agent-big-output-tests--with
    (let ((output (format "{\"error\": \"%s\\n%s\"}" (make-string 30000 ?e) (make-string 30000 ?f))))
      (draw :type 'tool :state 'completed :id "gone" :name "mcp__other__thing" :output output)
      (should (< (buffer-size) 600))
      (should (<= (romacs-agent-big-output-tests--longest) 240)))))

(ert-deftest romacs-agent-big-output-an-approval-past-the-bound-is-drawn ()
  ;; A Write asked for with more than the chat reads: its card is drawn,
  ;; saying the input's size.
  (let ((romacs-agent-rows-read-limit 1000))
    (romacs-agent-big-output-tests--with
      (draw :type 'approval :request-id "w" :tool "Write"
            :input (romacs-agent-big-output-tests--input "file_path" "/tmp/x" "content" (make-string 5000 ?x)))
      (should (string-match-p "characters in all" (buffer-substring-no-properties (point-min) (point-max)))))))

(provide 'romacs-agent-big-output-tests)
;;; romacs-agent-big-output-tests.el ends here
