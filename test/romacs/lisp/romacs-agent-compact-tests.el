;;; romacs-agent-compact-tests.el --- Compaction and thinking in the transcript -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)
(require 'romacs-test-socket)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-history))

(defconst romacs-agent-compact-tests--summary "This session continues: **Kept** facts.")

(defun romacs-agent-compact-tests--lines ()
  "A /compact's stream, as Claude Code writes it."
  (mapcar
   #'json-parse-string
   (list "{\"type\":\"system\",\"subtype\":\"status\",\"status\":\"compacting\"}"
         "{\"type\":\"system\",\"subtype\":\"status\",\"status\":null,\"compact_result\":\"success\"}"
         "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"compact_metadata\":{\"trigger\":\"manual\"}}"
         (format "{\"type\":\"user\",\"isSynthetic\":true,\"parent_tool_use_id\":null,\"message\":{\"role\":\"user\",\"content\":\"%s\"}}"
                 romacs-agent-compact-tests--summary)
         "{\"type\":\"user\",\"isReplay\":true,\"parent_tool_use_id\":null,\"message\":{\"role\":\"user\",\"content\":\"<local-command-stdout>Compacted </local-command-stdout>\"}}")))

(defun romacs-agent-compact-tests--events (session lines)
  (mapcan (lambda (line) (romacs-agent--claude-events session line)) lines))

(defmacro romacs-agent-compact-tests--with-chat (var &rest body)
  (declare (indent 1))
  `(let ((,var (romacs-agent--make :id "c" :harness 'claude :state 'running :title "t"
                                   :tools (make-hash-table :test #'equal))))
     (with-temp-buffer
       (romacs-agent-chat-mode)
       (setq romacs-agent-ui--session ,var
             romacs-agent-ui--messages (make-hash-table :test #'equal)
             romacs-agent-ui--end (point-marker))
       ,@body)))

(defun romacs-agent-compact-tests--draw (session &rest event)
  (romacs-agent-ui--draw session event (current-buffer)))

(ert-deftest romacs-agent-compaction-is-one-event ()
  (let ((session (romacs-agent--make :harness 'claude)))
    (should (equal (romacs-agent-compact-tests--events session (romacs-agent-compact-tests--lines))
                   `((:type status :text "compacting")
                     (:type compacted :text ,romacs-agent-compact-tests--summary))))
    (should-not (romacs-agent-compacting session))))

(ert-deftest romacs-agent-failed-compaction-is-an-error ()
  (should (equal (romacs-agent--claude-status
                  (json-parse-string "{\"status\":null,\"compact_result\":\"failed\",\"compact_error\":\"too short\"}"))
                 '((:type error :text "Compaction failed: too short")))))

(ert-deftest romacs-agent-local-command-output-loses-its-tags ()
  (let ((session (romacs-agent--make :harness 'claude)))
    (should (equal (romacs-agent--claude-injected "<local-command-stdout>Total cost: $1</local-command-stdout>" nil)
                   '((:type status :text "Total cost: $1" :parent nil))))))

(ert-deftest romacs-agent-automatic-compaction-summary-is-blocks ()
  (let ((session (romacs-agent--make :harness 'claude)))
    (romacs-agent--claude-events session (nth 2 (romacs-agent-compact-tests--lines)))
    (should (equal (romacs-agent--claude-events
                    session
                    (json-parse-string
                     "{\"type\":\"user\",\"isSynthetic\":true,\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Kept.\"}]}}"))
                   '((:type compacted :text "Kept."))))))

(ert-deftest romacs-agent-cut-short-compaction-claims-no-later-line ()
  (let ((session (romacs-agent--make :harness 'claude))
        (lines (romacs-agent-compact-tests--lines)))
    (romacs-agent--claude-events session (nth 2 lines))
    (should (romacs-agent-compacting session))
    (romacs-agent--claude-events session (json-parse-string "{\"type\":\"system\",\"subtype\":\"hook\"}"))
    (should-not (cl-find 'compacted (romacs-agent--claude-events session (nth 3 lines))
                         :key (lambda (event) (plist-get event :type))))))

(ert-deftest romacs-agent-sweep-keeps-a-route-that-answers ()
  (let* ((root (romacs-test-socket-directory "rs")) (routes (expand-file-name "route/" root))
         (live (expand-file-name "live.sock" root)) (server nil)
         (answering (expand-file-name "a.sock" routes)) (dead (expand-file-name "d.sock" routes)))
    (unwind-protect
        (progn
          (make-directory routes)
          (set-file-modes routes #o700)
          (setq server (make-network-process :name "sweep-test" :family 'local :service live :server t :noquery t))
          (make-symbolic-link live answering)
          (make-symbolic-link (expand-file-name "gone.sock" root) dead)
          (cl-letf (((symbol-function 'romacs-agent--runtime-root) (lambda () root))
                    ((symbol-function 'romacs-agent--state-root) (lambda () (expand-file-name "state/" root)))
                    ((symbol-function 'romacs-agent--sessions-base) (lambda () (expand-file-name "none/" root))))
            (romacs-agent--sweep))
          (should (file-symlink-p answering))
          (should-not (file-symlink-p dead)))
      (when server (delete-process server))
      (delete-directory root t))))

(ert-deftest romacs-agent-history-keeps-the-compaction ()
  (let ((events (romacs-agent-history--claude-events
                 (list (json-parse-string
                        (format "{\"type\":\"user\",\"isCompactSummary\":true,\"message\":{\"content\":\"%s\"}}"
                                romacs-agent-compact-tests--summary))))))
    (should (equal events `((:type compacted :text ,romacs-agent-compact-tests--summary))))))

(ert-deftest romacs-agent-claude-asks-for-thinking-text ()
  (cl-letf (((symbol-function 'romacs-agent--write-json) #'ignore))
    (let ((argv (romacs-agent--claude-argv (romacs-agent--make :id "c" :harness 'claude) nil)))
      (should (equal (cadr (member "--thinking-display" argv)) "summarized")))))

(ert-deftest romacs-agent-unstreamed-thinking-is-shown ()
  (let* ((session (romacs-agent--make :harness 'claude :tools (make-hash-table :test #'equal)))
         (line (json-parse-string "{\"type\":\"assistant\",\"parent_tool_use_id\":null,\"message\":{\"id\":\"m\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"Weigh it.\"}]}}")))
    (should (equal (romacs-agent--claude-events session line)
                   '((:type thinking :text "Weigh it." :parent nil))))
    (puthash "m" t (romacs-agent-streamed session))
    (should-not (romacs-agent--claude-events session line))))
