;;; romacs-agent-glyphs.el --- The marks the agents' views draw -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Every mark a view of agents draws (the chat, its rows, the board and
;; the menus) by the role it plays, in one table a user may
;; change: `romacs-agent-glyph-alist' (`romacs-agent-glyphs' says whether
;; a state is said by its glyph or its word).  The views read a mark as they draw,
;; so a change shows in what is drawn next, no reload.  A row hung
;; under a mark is indented by the mark's width, an emoji's two columns
;; as a symbol's one.

;;; Code:

(defconst romacs-agent-glyphs--roles
  '(;; The chat's transcript
    (thinking "🤔" "Before each paragraph of an agent's thinking")
    (prompt "❯" "Before the user's message")
    (steer "↪" "Before a message sent while the agent works")
    (composer "❯" "Before the composer's line")
    (queue-held "»" "Before a queued message being edited")
    (queue-other "<" "Before a queued message from elsewhere")
    (tool "●" "Before a tool call's row")
    (output "⎿" "Before what a call put out")
    (running "⋮" "Before a running call's last lines")
    (subagent "│" "The rule a subagent's work hangs on")
    (turn-end "✻" "Before a turn's footer")
    (asks "⚠" "Before what waits on the user")
    (warning "⚠" "Before what went wrong in a view")
    (allowed "✔" "Before an approval allowed")
    (denied "✘" "Before an approval denied, or an error")
    (plan "📋" "Before a plan")
    (earlier "↑" "Before the line that shows earlier turns")
    (rolled-back "↶" "Before a rollback's note")
    (image "▣" "Before an image's link")
    (you-bar "▌" "The bar the user's message stands behind")
    (other-bar "┃" "The bar a message from elsewhere is quoted behind")
    (from "←" "Before whom a message from elsewhere is from")
    ;; A call's row
    (message "✉︎" "A message sent to a session")
    (message-bar "┃" "The bar a sent message is quoted behind")
    (delegate "⑂" "A task delegated")
    (romacs "◈" "Romacs's own tools, where its icon cannot be drawn")
    (tasks "◷" "Tasks listed or watched")
    (result "⇒" "Before what an evaluation returned")
    (listing "▤" "Instances or limits listed")
    (restart "↻" "A restart asked for")
    (dev-check "⚑" "A dev-check")
    (queued "⇡" "A branch queued to land")
    (ok "✓" "Done well: committed, merged, landed, enabled")
    (failed "✗" "A task's failure")
    (read "◧" "A file read")
    (edit "✎" "A file written or edited")
    (search "⌕" "A search of files")
    (fetch "↗" "A page fetched")
    (web-search "◍" "A search of the web")
    (todos "☐" "A todo list")
    (eval "λ" "Lisp evaluated")
    (shell "$" "A shell command")
    (task-failed "!" "A task failed, in a list of tasks")
    (task-cancelled "×" "A task cancelled, in a list of tasks")
    (task-idle "·" "A task idle, in a list of tasks")
    (step-doing "→" "A plan's step under way")
    (step-todo "○" "A plan's step to do")
    ;; A harness, where views name it
    (claude "✳" "Claude Code")
    (codex "◎" "Codex")
    ;; A session's state, where views show it
    (approval "✋" "A session that needs an approval")
    (question "❓" "A session that asks the user")
    (failure "❗" "A session failed, or never started")
    (working "🛠️" "A session working")
    (threads "🧵" "Threads live under a session")
    (waiting "⏳" "A session waiting on its workers")
    (completed "✅" "A session that completed")
    (detached "🔌" "A session detached")
    (ended "⏹" "A session ended")
    (history "📜" "A session from history")
    (resume "⏰" "Before when a session resumes")
    (chip-you "◆" "A chip: needs the user")
    (chip-working "●" "A chip: working")
    ;; The board and the answering layer
    (board-threads "▸" "Before a session's thread count on the board")
    (board-settled "✔️" "Before the project's settled thread count on the board")
    (board-history "◇" "Before a session from history on the board")
    (board-open "˅" "A board row whose threads show")
    (board-closed "›" "A board row whose threads are folded")
    (off "·" "Off: an instance disabled, an account not picked")
    (picked "●" "A choice picked, one of several")
    (unpicked "○" "A choice not picked, one of several")
    (checked "☑" "A choice picked, any of several")
    (unchecked "☐" "A choice not picked, any of several")
    (own "✎" "Before the user's own answer")
    (send "↵" "Before the key that sends the answers")
    (attachment "▫" "An attachment, where its icon cannot be drawn"))
  "Each mark's role, its default and what it marks: (ROLE DEFAULT DOC).")

(defvar romacs-agent-glyph-alist-changed-hook nil
  "Run after `romacs-agent-glyph-alist' is set to refresh the agent views.")

(defcustom romacs-agent-glyph-alist
  (mapcar (pcase-lambda (`(,role ,default ,_)) (cons role default)) romacs-agent-glyphs--roles)
  "The marks the views of agents draw, by role.  Change one and what is
drawn next shows it; a role left out draws its default."
  :type `(alist :key-type symbol :value-type string
                :options ,(mapcar (pcase-lambda (`(,role ,_ ,doc)) `(,role (string :tag ,doc)))
                                  romacs-agent-glyphs--roles))
  :group 'romacs-agent
  :initialize #'custom-initialize-default
  :set (lambda (symbol value)
         (set-default symbol value)
         (run-hooks 'romacs-agent-glyph-alist-changed-hook)))

(defun romacs-agent-glyph (role)
  "The mark for ROLE, as `romacs-agent-glyph-alist' says, else its default."
  (or (alist-get role romacs-agent-glyph-alist)
      (pcase (assq role romacs-agent-glyphs--roles)
        (`(,_ ,default ,_) default)
        (_ (error "No mark has the role %S" role)))))

(defun romacs-agent-glyph-padded (role &rest peers)
  "ROLE's mark padded to the widest of it and PEERS', the marks that share
its column, so what follows any of them lines up."
  (let ((mark (romacs-agent-glyph role))
        (width (apply #'max (mapcar (lambda (r) (string-width (romacs-agent-glyph r))) (cons role peers)))))
    (concat mark (make-string (- width (string-width mark)) ?\s))))

(defun romacs-agent-glyph-hang (prefix)
  "Spaces as wide as PREFIX, a mark and what pads it: what a line hung
under it starts with."
  (make-string (string-width prefix) ?\s))

(provide 'romacs-agent-glyphs)
;;; romacs-agent-glyphs.el ends here
