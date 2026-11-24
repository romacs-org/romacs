;;; romacs-agent-glyphs-tests.el --- The marks the agents' views draw -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-glyphs)
  (require 'romacs-agent-log)
  (require 'romacs-agent-board))

(ert-deftest romacs-agent-glyphs-every-role-has-its-default ()
  (pcase-dolist (`(,role ,default ,doc) romacs-agent-glyphs--roles)
    (should (equal (romacs-agent-glyph role) default))
    (should (and (stringp doc) (not (string-empty-p default)))))
  ;; Thinking says itself by its emoji; a role left out of the table
  ;; draws its default.
  (should (equal (romacs-agent-glyph 'thinking) "🤔"))
  (let ((romacs-agent-glyph-alist nil))
    (should (equal (romacs-agent-glyph 'tool) "·")))
  (should-error (romacs-agent-glyph 'no-such-role)))

(ert-deftest romacs-agent-glyphs-a-change-draws-at-once ()
  ;; No reload: what is drawn next reads the table, and the views drawn
  ;; ahead are told.
  (let* ((romacs-agent-glyph-alist (copy-alist romacs-agent-glyph-alist))
         (told 0)
         (romacs-agent-glyph-alist-changed-hook (list (lambda () (cl-incf told)))))
    (customize-set-variable 'romacs-agent-glyph-alist
                            (cons '(thinking . "∴") (assq-delete-all 'thinking (copy-alist romacs-agent-glyph-alist))))
    (should (= told 1))
    (should (equal (romacs-agent-log--stream-chunk 'thinking t 0 "a\n\nb") "∴ a\n\n∴ b"))))

(ert-deftest romacs-agent-glyphs-hang-under-a-wide-mark ()
  ;; A line hung under a mark is as wide as it: an emoji takes two columns.
  (should (equal (romacs-agent-glyph-hang "  ⎿ ") "    "))
  (should (equal (romacs-agent-glyph-hang "  🤔 ") "     "))
  (let ((romacs-agent-glyph-alist (cons '(output . "🤔") romacs-agent-glyph-alist)))
    (let ((text (romacs-agent-log--output "one\ntwo" 'romacs-agent-log-output (cons nil "one\ntwo"))))
      (should (string-prefix-p "    🤔 one\n       two" (substring-no-properties text))))))

;; What a mark change meets: a call drawn with the old mark ends.
(ert-deftest romacs-agent-glyphs-a-call-ends-whatever-its-mark-became ()
  (let ((session (romacs-agent--make :id "G" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal)))
        (input (lambda (path) (let ((h (make-hash-table :test #'equal))) (puthash "file_path" path h) h))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (let ((romacs-agent-glyph-alist (copy-alist romacs-agent-glyph-alist)))
        (romacs-agent-log-draw session (list :type 'tool :state 'started :id "a" :name "Agent"
                                             :input (let ((h (make-hash-table :test #'equal))) (puthash "description" "dig" h) h)))
        ;; A wide mark changes while the call runs.
        (setf (alist-get 'read romacs-agent-glyph-alist) "📖")
        (romacs-agent-log-draw session (list :type 'tool :state 'started :id "r" :name "Read" :parent "a"
                                             :input (funcall input "/x/notes.txt")))
        (setf (alist-get 'read romacs-agent-glyph-alist) "R")
        (romacs-agent-log-draw session (list :type 'tool :state 'completed :id "r" :output "a\nb"))
        (let ((text (buffer-substring-no-properties (point-min) romacs-agent-log--end)))
          (should (string-match-p "^│   R notes.txt  /x · 2 lines$" text)))))))

(ert-deftest romacs-agent-glyphs-a-narrowed-chat-keeps-its-narrowing ()
  ;; Narrowed to its transcript's end, a changed mark's separator is
  ;; written outside the narrowing.
  (let ((session (romacs-agent--make :id "N" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (romacs-agent-log--separator t)
      (romacs-agent-log-draw session (list :type 'delta :text "Some words."))
      (romacs-agent-log--separator)
      (narrow-to-region (point-min) romacs-agent-log--end)
      (let ((before (buffer-string)))
        (romacs-agent-log--marks-changed)
        (should (equal (buffer-string) before))))))

(ert-deftest romacs-agent-glyphs-a-composer-takes-a-changed-prompt-mark ()
  ;; The composer's prompt is the table's mark, a changed one at once,
  ;; shown once before its first line.
  (with-temp-buffer
    (romacs-agent-log-mode)
    (let ((composer (romacs-agent-log-composer-buffer (current-buffer)))
          (romacs-agent-glyph-alist (copy-alist romacs-agent-glyph-alist)))
      (setf (alist-get 'composer romacs-agent-glyph-alist) ">>")
      (run-hooks 'romacs-agent-glyph-alist-changed-hook)
      (with-current-buffer composer
        (insert "draft")
        (should (string-suffix-p ">> " (overlay-get romacs-agent-compose--start 'before-string)))
        (should (= (overlay-start romacs-agent-compose--start) (point-min)))
        (should-not (get-char-property (point-min) 'line-prefix))
        (should-not line-prefix)))))

(ert-deftest romacs-agent-glyphs-a-cut-cell-fits-its-column ()
  (dolist (text '("❤️🤔x" "🤔🤔🤔🤔🤔" "abcdefgh"))
    (should (= (string-width (romacs-agent-board--cell text 4)) 4))))

(provide 'romacs-agent-glyphs-tests)
;;; romacs-agent-glyphs-tests.el ends here
