;;; romacs-agent-instances-tests.el --- Provider instances and T3 Code's -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-t3))

(defmacro romacs-agent-instances-test--with (settings providers &rest body)
  "BODY with T3's settings file holding SETTINGS (a string, or nil for no
file) and `romacs-agent-providers' PROVIDERS."
  (declare (indent 2))
  `(let* ((dir (make-temp-file "romacs-agent-t3-" t))
          (romacs-agent-t3-settings (expand-file-name "settings.json" dir))
          (romacs-agent-t3--read nil)
          (romacs-agent-providers ,providers))
     (unwind-protect
         (progn (when ,settings (with-temp-file romacs-agent-t3-settings (insert ,settings)))
                ,@body)
       (delete-directory dir t))))

(defun romacs-agent-instances-test--names ()
  (mapcar (lambda (i) (plist-get i :name)) (romacs-agent-instances)))

(defun romacs-agent-instances-test--get (name)
  (cl-find name (romacs-agent-instances) :key (lambda (i) (plist-get i :name)) :test #'equal))

(defun romacs-agent-instances-test--reasons ()
  (mapcar (lambda (c) (plist-get c :reason)) (cdr (romacs-agent-instances t))))

(ert-deftest romacs-agent-t3-split-as-t3-does ()
  (should (equal (romacs-agent-t3-split "--chrome \"a b\" 'c d' e\\ f \"x\\\"y\" '' g")
                 '("--chrome" "a b" "c d" "e f" "x\"y" "" "g")))
  (should (equal (romacs-agent-t3-split "  ") nil))
  (should (equal (romacs-agent-t3-split "\"a\\nb\"") '("a\\nb"))))

(ert-deftest romacs-agent-t3-absent-malformed-and-defaults ()
  ;; No file: only the Lisp entries, and why.
  (romacs-agent-instances-test--with nil '(t3 ("claude" . "~/.claude"))
    (should (equal (romacs-agent-instances-test--names) '("claude")))
    (should (eq (plist-get (romacs-agent-instances-test--get "claude") :source) 'lisp))
    (should (string-prefix-p "T3 settings not found" (car (romacs-agent-instances-test--reasons)))))
  ;; Malformed: the same, said once.
  (romacs-agent-instances-test--with "{not json" '(t3 ("claude" . "~/.claude"))
    (should (equal (romacs-agent-instances-test--names) '("claude")))
    (should (string-prefix-p "T3 settings unreadable" (car (romacs-agent-instances-test--reasons)))))
  ;; Its two defaults, unconfigured: enabled, in their default homes.
  (romacs-agent-instances-test--with "{}" '(t3)
    (should (equal (romacs-agent-instances-test--names) '("claude" "codex")))
    (let ((claude (romacs-agent-instances-test--get "claude")))
      (should (equal (list (plist-get claude :harness) (plist-get claude :t3-id) (plist-get claude :enabled)
                           (plist-get claude :config-dir) (plist-get claude :source))
                     '(claude "claudeAgent" t nil t3))))))

(ert-deftest romacs-agent-t3-a-second-claude-account ()
  ;; As T3 stores a second Claude Code account: driver, display name,
  ;; enabled, config.homePath.  An init naming T3 first, then fallbacks.
  (romacs-agent-instances-test--with
      "{\"providerInstances\":{\"claudeAgent_claude2\":{\"driver\":\"claudeAgent\",\"displayName\":\"Claude2\",\"enabled\":true,\"config\":{\"homePath\":\"~/.config/claude/second\"}}}}"
      '(t3 ("claude" . "~/.claude") ("claude2" . "~/.config/claude/other") ("codex" . "~/.codex"))
    (should (equal (romacs-agent-instances-test--names) '("claude" "codex" "claude2")))
    (let ((second (romacs-agent-instances-test--get "claude2")))
      (should (equal (list (plist-get second :harness) (plist-get second :config-dir) (plist-get second :t3-id)
                           (plist-get second :display) (plist-get second :source))
                     (list 'claude (expand-file-name "~/.config/claude/second") "claudeAgent_claude2" "Claude2" 't3))))
    ;; Its T3 id names it too (a delegation target's alias).
    (should (equal (plist-get (romacs-agent--configured "claudeAgent_claude2") :name) "claude2"))
    (should (equal (romacs-agent--default-provider) "claude")))
  ;; A Lisp entry before `t3' wins its name.
  (romacs-agent-instances-test--with "{}" '(("claude" . "/elsewhere/claude") t3)
    (should (equal (romacs-agent-instances-test--names) '("claude" "codex")))
    (should (eq (plist-get (romacs-agent-instances-test--get "claude") :source) 'lisp))))

(ert-deftest romacs-agent-t3-disabled-unknown-collisions-and-overlays ()
  (romacs-agent-instances-test--with
      (concat "{\"providerInstances\":{"
              "\"claudeAgent\":{\"driver\":\"claudeAgent\",\"enabled\":false},"
              "\"codex\":{\"driver\":\"codex\",\"config\":{\"enabled\":false}},"
              "\"cursor\":{\"driver\":\"cursor\",\"displayName\":\"Cursor\"},"
              "\"claudeAgent_a\":{\"driver\":\"claudeAgent\",\"displayName\":\"Work\",\"config\":{\"homePath\":\"/h/shared\",\"binaryPath\":\"/opt/claude\",\"launchArgs\":\"--chrome --x 'a b'\"}},"
              "\"claudeAgent_b\":{\"driver\":\"claudeAgent\",\"displayName\":\"Work\",\"config\":{\"homePath\":\"/h/shared\"}},"
              "\"codex_shadow\":{\"driver\":\"codex\",\"displayName\":\"Codex Personal\",\"config\":{\"shadowHomePath\":\"~/.codex-t3/p\"}},"
              "\"codex_managed\":{\"driver\":\"codex\",\"displayName\":\"Codex Team\",\"config\":{\"setupMode\":\"managed\"}}"
              "}}")
      '(t3)
    (should (equal (romacs-agent-instances-test--names)
                   '("claude" "codex" "work" "claudeAgent_b" "codex-personal" "codex-team")))
    ;; Disabled by either flag; not offered, still known.
    (should-not (plist-get (romacs-agent-instances-test--get "claude") :enabled))
    (should-not (plist-get (romacs-agent-instances-test--get "codex") :enabled))
    (should-error (romacs-agent-start "/tmp" nil nil "codex") :type 'user-error)
    ;; Two sharing a directory: both kept, one continuation key.
    (let ((a (romacs-agent-instances-test--get "work")) (b (romacs-agent-instances-test--get "claudeAgent_b")))
      (should (equal (plist-get a :key) (plist-get b :key)))
      (should (equal (plist-get a :program) "/opt/claude"))
      (should (equal (plist-get a :arguments) '("--chrome" "--x" "a b")))
      (should (plist-get b :enabled)))
    ;; Account overlays: disabled, with why; a driver Romacs lacks: why only.
    (should-not (plist-get (romacs-agent-instances-test--get "codex-personal") :enabled))
    (should-not (plist-get (romacs-agent-instances-test--get "codex-team") :enabled))
    (let ((reasons (romacs-agent-instances-test--reasons)))
      (should (= (cl-count-if (lambda (r) (string-prefix-p "T3 auth overlay" r)) reasons) 2))
      (should (cl-some (lambda (r) (string-match-p "no Romacs harness for T3 driver cursor" r)) reasons)))
    ;; The default is the first enabled one.
    (should (equal (romacs-agent--default-provider) "work"))))

(ert-deftest romacs-agent-t3-read-once-per-change ()
  (romacs-agent-instances-test--with "{}" '(t3)
    (let ((first (romacs-agent-t3-read romacs-agent-t3-settings (romacs-agent--t3-harnesses) romacs-agent--driver-generation)))
      ;; Unchanged, it is not read again.
      (should (eq first (romacs-agent-t3-read romacs-agent-t3-settings (romacs-agent--t3-harnesses)
                                              romacs-agent--driver-generation)))
      ;; A changed file (size) is read again.
      (with-temp-file romacs-agent-t3-settings
        (insert "{\"providerInstances\":{\"codex\":{\"driver\":\"codex\",\"enabled\":false}}}"))
      (should-not (plist-get (cl-find "codex" (romacs-agent-instances) :key (lambda (i) (plist-get i :name)) :test #'equal)
                             :enabled)))))

(ert-deftest romacs-agent-t3-mistyped-fields-take-nothing ()
  ;; A field of the wrong type is never read as a default (another
  ;; account's home): the file gives no instance, and says why.
  (dolist (settings '("{\"providerInstances\":{\"claudeAgent_x\":{\"driver\":\"claudeAgent\",\"config\":{\"homePath\":42}}}}"
                      "{\"providerInstances\":[]}"
                      "{\"providerInstances\":{\"claudeAgent_x\":{\"driver\":\"claudeAgent\",\"enabled\":\"yes\"}}}"
                      "{\"providerInstances\":{\"claudeAgent_x\":{\"driver\":7}}}"
                      "{\"providerInstances\":{\"claudeAgent_x\":\"claude\"}}"
                      "[1]"))
    (romacs-agent-instances-test--with settings '(t3 ("claude" . "~/.claude"))
      (should (equal (romacs-agent-instances-test--names) '("claude")))
      (should (eq (plist-get (romacs-agent-instances-test--get "claude") :source) 'lisp))
      (should (string-prefix-p "T3 settings unreadable" (car (romacs-agent-instances-test--reasons)))))))

(ert-deftest romacs-agent-t3-ids-name-their-own-instances ()
  ;; One display name another instance's id, one its taken default: each
  ;; is named by its own id, and found by it.
  (romacs-agent-instances-test--with
      (concat "{\"providerInstances\":{"
              "\"codex_a\":{\"driver\":\"codex\",\"displayName\":\"codex_b\",\"config\":{\"homePath\":\"/h/a\"}},"
              "\"codex_b\":{\"driver\":\"codex\",\"displayName\":\"Codex\",\"config\":{\"homePath\":\"/h/b\"}}}}")
      '(t3)
    (should (equal (romacs-agent-instances-test--names) '("claude" "codex" "codex_a" "codex_b")))
    (should (equal (plist-get (romacs-agent--configured "codex_a") :config-dir) "/h/a"))
    (should (equal (plist-get (romacs-agent--configured "codex_b") :config-dir) "/h/b"))))

(ert-deftest romacs-agent-t3-paths-trimmed-and-expanded ()
  (romacs-agent-instances-test--with
      (concat "{\"providerInstances\":{"
              "\"claudeAgent_a\":{\"driver\":\"claudeAgent\",\"displayName\":\"A\",\"config\":{\"binaryPath\":\"  ~/bin/claude \",\"homePath\":\" ~/.claude-a \"}},"
              "\"claudeAgent_b\":{\"driver\":\"claudeAgent\",\"displayName\":\"B\",\"config\":{\"binaryPath\":\"claude-dev\",\"homePath\":\"   \"}}}}")
      '(t3)
    (let ((a (romacs-agent-instances-test--get "a")) (b (romacs-agent-instances-test--get "b")))
      (should (equal (plist-get a :program) (expand-file-name "~/bin/claude")))
      (should (equal (plist-get a :config-dir) (expand-file-name "~/.claude-a")))
      (should (equal (plist-get b :program) "claude-dev"))
      (should-not (plist-get b :config-dir)))))

(ert-deftest romacs-agent-t3-a-driver-defined-again ()
  ;; A driver's T3 name changing, the settings file not: read again.
  (let ((romacs-agent--drivers (copy-alist romacs-agent--drivers))
        (romacs-agent--driver-table (copy-hash-table romacs-agent--driver-table))
        (romacs-agent--driver-generation romacs-agent--driver-generation))
    (romacs-agent-define-harness 'zzrpc :t3-driver "zz" :lifetime 'persistent :config-variable "ZZ_HOME"
                                 :config-default "~/.zz")
    (romacs-agent-instances-test--with "{\"providerInstances\":{\"zz\":{\"driver\":\"zz\",\"displayName\":\"Zz\"}}}" '(t3)
      (should (member "zz" (romacs-agent-instances-test--names)))
      (romacs-agent-define-harness 'zzrpc :lifetime 'persistent :config-variable "ZZ_HOME" :config-default "~/.zz")
      (should-not (member "zz" (romacs-agent-instances-test--names)))
      (should (cl-some (lambda (r) (string-match-p "no Romacs harness for T3 driver zz" r))
                       (romacs-agent-instances-test--reasons))))))

(ert-deftest romacs-agent-default-provider-reads-only-as-far-as-it-needs ()
  (let ((romacs-agent-default-provider nil)
        (romacs-agent-providers '(("claude" :enabled nil) ("claude") ("codex") ("codex-2") ("claude-2")))
        (read nil)
        (provider (symbol-function 'romacs-agent--provider)))
    (cl-letf (((symbol-function 'romacs-agent--provider)
               (lambda (entry) (push (car entry) read) (funcall provider entry))))
      ;; The first entry of a name wins, disabled or not.
      (should (equal (romacs-agent--default-provider) "codex"))
      (should (equal (reverse read) '("claude" "codex"))))
    (let ((romacs-agent-providers '(("claude" :enabled nil))))
      (should-error (romacs-agent--default-provider) :type 'user-error))))

(provide 'romacs-agent-instances-tests)
;;; romacs-agent-instances-tests.el ends here
