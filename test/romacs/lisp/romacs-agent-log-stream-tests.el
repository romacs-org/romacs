;;; romacs-agent-log-stream-tests.el --- Streamed text in an agent's chat -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))

(ert-deftest romacs-agent-log-stream-splits-off-line-breaks ()
  (should (equal (romacs-agent-log--stream-split "\n\nab\n\ncd\n") '(2 "ab\n\ncd" 1)))
  (should (equal (romacs-agent-log--stream-split "ab") '(0 "ab" 0)))
  (should (equal (romacs-agent-log--stream-split "\n\n\n") '(3 "" 0)))
  (should (equal (romacs-agent-log--stream-split "") '(0 "" 0))))

(ert-deftest romacs-agent-log-stream-cuts-whole-lines ()
  (should (equal (romacs-agent-log--whole-lines "a\nb\nc") '("a\nb" . "c")))
  (should (equal (romacs-agent-log--whole-lines "a\n") '("a" . "")))
  (should (equal (romacs-agent-log--whole-lines "abc") '(nil . "abc"))))

(ert-deftest romacs-agent-log-stream-marks-thinking-paragraphs ()
  (let ((marked (romacs-agent-log--thinking-marks "one\n\ntwo\nmore" t)))
    (should (equal marked "🤔 one\n\n🤔 two\n   more"))
    (should (eq (get-text-property 0 'face marked) 'romacs-agent-log-thinking))
    (should (eq (get-text-property (1- (length marked)) 'face marked) 'romacs-agent-log-thinking)))
  (should (equal (romacs-agent-log--thinking-marks "on" nil) "on")))

(ert-deftest romacs-agent-log-stream-chunk-parts-paragraphs-once ()
  ;; A first text starts the entry, with no line breaks before it.
  (let ((chunk (romacs-agent-log--stream-chunk 'text t 3 "a\n\n\n\nb")))
    (should (equal chunk "a\n\nb"))
    (should (eq (get-text-property 0 'romacs-agent-log-entry chunk) 'text)))
  ;; Later text keeps at most one blank line before it.
  (should (equal (romacs-agent-log--stream-chunk 'text nil 5 "c") "\n\nc"))
  (should (equal (romacs-agent-log--stream-chunk 'text nil 1 "c") "\nc"))
  (should-not (get-text-property 0 'romacs-agent-log-entry (romacs-agent-log--stream-chunk 'text nil 0 "c")))
  ;; Thinking opens each paragraph with its mark; a line run on hangs
  ;; under its words, as wide as the mark.
  (should (equal (romacs-agent-log--stream-chunk 'thinking t 2 "x") "🤔 x"))
  (should (equal (romacs-agent-log--stream-chunk 'thinking nil 2 "y") "\n\n🤔 y"))
  (should (equal (romacs-agent-log--stream-chunk 'thinking nil 1 "y") "\n   y"))
  (should-not (get-text-property 0 'face (romacs-agent-log--stream-chunk 'thinking nil 2 "y"))))

(provide 'romacs-agent-log-stream-tests)
;;; romacs-agent-log-stream-tests.el ends here
