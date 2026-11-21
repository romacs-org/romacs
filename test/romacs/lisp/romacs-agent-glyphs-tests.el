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
  (require 'romacs-agent-log))

(ert-deftest romacs-agent-glyphs-every-role-has-its-default ()
  (pcase-dolist (`(,role ,default ,doc) romacs-agent-glyphs--roles)
    (should (equal (romacs-agent-glyph role) default))
    (should (and (stringp doc) (not (string-empty-p default)))))
  ;; Thinking says itself by its emoji; a role left out of the table
  ;; draws its default.
  (should (equal (romacs-agent-glyph 'thinking) "🤔"))
  (let ((romacs-agent-glyph-alist nil))
    (should (equal (romacs-agent-glyph 'tool) "●")))
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
      (should (string-prefix-p "  🤔 one\n     two" (substring-no-properties text))))))

;; What a mark change meets: a call drawn with the old mark ends.
(ert-deftest romacs-agent-glyphs-a-call-ends-whatever-its-mark-became ()
  (let ((session (romacs-agent--make :id "G" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal)))
        (input (lambda (path) (let ((h (make-hash-table :test #'equal))) (puthash "file_path" path h) h))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (let ((romacs-agent-glyph-alist (copy-alist romacs-agent-glyph-alist)))
        (romacs-agent-log-draw session (list :type 'tool :state 'started :id "a" :name "Agent"
                                             :input (let ((h (make-hash-table :test #'equal))) (puthash "description" "dig" h) h)))
        ;; Drawn with no mark at all, then the mark changed while it runs:
        ;; its row still says what came of it, its bars whole.
        (setf (alist-get 'tool romacs-agent-glyph-alist) "")
        (romacs-agent-log-draw session (list :type 'tool :state 'started :id "r" :name "Read" :parent "a"
                                             :input (funcall input "/x/notes.txt")))
        (setf (alist-get 'tool romacs-agent-glyph-alist) "●")
        (romacs-agent-log-draw session (list :type 'tool :state 'completed :id "r" :output "a\nb"))
        (let ((text (buffer-substring-no-properties (point-min) romacs-agent-log--end)))
          (should (string-match-p "^│  ◧ notes.txt  /x · 2 lines$" text)))))))

(provide 'romacs-agent-glyphs-tests)
;;; romacs-agent-glyphs-tests.el ends here
