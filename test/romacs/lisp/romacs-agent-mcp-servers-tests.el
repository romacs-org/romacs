;;; romacs-agent-mcp-servers-tests.el --- Extra MCP servers in each harness's config  -*- lexical-binding: t -*-

;;; Commentary:

;; `romacs-agent-extra-mcp-servers' reaches Claude Code's mcp.json and
;; Codex's -c words beside Romacs's own server, malformed entries left out.

;;; Code:

(require 'ert)
(require 'romacs-agent)
(require 'romacs-agent-claude)
(require 'romacs-agent-codex)

(defmacro romacs-agent-mcp-servers-tests--with (servers &rest body)
  "BODY with `romacs-agent-extra-mcp-servers' SERVERS, the relay's words stubbed."
  (declare (indent 1))
  `(let ((romacs-agent-extra-mcp-servers ,servers))
     (cl-letf (((symbol-function 'romacs-agent--relay-args) (lambda (_session) '("--route" "/r")))
               ((symbol-function 'romacs-agent--program) (lambda (name) (concat "/bin/" name))))
       ,@body)))

(ert-deftest romacs-agent-mcp-servers-only-well-formed-ones-and-never-romacs ()
  (romacs-agent-mcp-servers-tests--with
      '(("forge" :command "/f/forge" :args ("mcp") :env (("FORGE_PROJECT" . "romacs")))
        ("romacs" :command "/evil")
        ("bad name" :command "/x")
        ("noargs" :command "/y")
        ("badenv" :command "/z" :env (("1BAD" . "v")))
        ("nocommand" :args ("a")))
    (should (equal (romacs-agent--extra-mcp-servers)
                   '(("forge" "/f/forge" ("mcp") (("FORGE_PROJECT" . "romacs")))
                     ("noargs" "/y" nil nil))))))

(ert-deftest romacs-agent-mcp-servers-reach-claudes-mcp-json-beside-romacs ()
  (romacs-agent-mcp-servers-tests--with
      '(("forge" :command "/f/forge" :args ("mcp") :env (("FORGE_PROJECT" . "romacs"))))
    (let* ((servers (romacs-agent--claude-mcp-servers 'session))
           (forge (gethash "forge" servers)))
      (should (equal (gethash "command" (gethash "romacs" servers)) "/bin/romacs-mcp"))
      (should (equal (gethash "command" forge) "/f/forge"))
      (should (equal (gethash "args" forge) ["mcp"]))
      (should (equal (gethash "FORGE_PROJECT" (gethash "env" forge)) "romacs")))))

(ert-deftest romacs-agent-mcp-servers-reach-codexs-words-beside-romacs ()
  (romacs-agent-mcp-servers-tests--with
      '(("forge" :command "/f/forge" :args ("mcp") :env (("FORGE_PROJECT" . "romacs"))))
    (let ((words (romacs-agent--codex-mcp-words 'session)))
      (should (member "mcp_servers.romacs.command=\"/bin/romacs-mcp\"" words))
      (should (member "mcp_servers.forge.command=\"/f/forge\"" words))
      (should (member "mcp_servers.forge.args=[\"mcp\"]" words))
      (should (member "mcp_servers.forge.env.FORGE_PROJECT=\"romacs\"" words)))))

(ert-deftest romacs-agent-mcp-servers-none-leaves-romacs-alone ()
  (romacs-agent-mcp-servers-tests--with nil
    (should (equal (hash-table-keys (romacs-agent--claude-mcp-servers 'session)) '("romacs")))
    (should (= (length (romacs-agent--codex-mcp-words 'session)) 6))))

(provide 'romacs-agent-mcp-servers-tests)
;;; romacs-agent-mcp-servers-tests.el ends here
