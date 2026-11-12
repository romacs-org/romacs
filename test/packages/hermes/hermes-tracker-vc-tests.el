;;; hermes-tracker-vc-tests.el --- Tracker VC tests -*- lexical-binding: t; -*-

;;; Code:

(require 'ert)
(require 'hermes-tracker-vc)

(ert-deftest hermes-tracker-vc-parses-configured-scp-remote ()
  "Configured SCP-style remotes resolve to Tracker repository slugs."
  (let ((hermes-tracker-vc-hosts '("git@theodora")))
    (should (equal (hermes-tracker-vc-slug-from-remote
                    "git@theodora:emacs-hermes-tracker.git")
                   "emacs-hermes-tracker"))
    (should-not (hermes-tracker-vc-slug-from-remote
                 "git@elsewhere:emacs-hermes-tracker.git"))))

(ert-deftest hermes-tracker-vc-parses-configured-url-remotes ()
  "HTTPS and SSH URL remotes use the same host allowlist."
  (let ((hermes-tracker-vc-hosts '("https://code.example" "git@theodora")))
    (should (equal (hermes-tracker-vc-slug-from-remote
                    "https://code.example/team/tracker.git")
                   "tracker"))
    (should (equal (hermes-tracker-vc-slug-from-remote
                    "https://code.example/team/tracker.git?transport=1")
                   "tracker"))
    (should (equal (hermes-tracker-vc-slug-from-remote
                    "ssh://git@theodora/team/client.git")
                   "client"))))

(ert-deftest hermes-tracker-vc-rejects-empty-url-hosts ()
  "Malformed configured origins cannot authorize malformed remotes."
  (let ((hermes-tracker-vc-hosts '("https:///missing-host")))
    (should-not (hermes-tracker-vc-slug-from-remote
                 "https:///tracker.git"))))

(ert-deftest hermes-tracker-vc-reads-origin-without-shell ()
  "Repository detection uses argument-safe Git process invocation."
  (let ((hermes-tracker-vc-hosts '("git@theodora")) arguments)
    (cl-letf (((symbol-function 'process-file)
               (lambda (program _in destination _display &rest args)
                 (setq arguments (cons program args))
                 (with-current-buffer destination
                   (insert "git@theodora:tracker.git\n"))
                 0)))
      (should (equal (hermes-tracker-vc-repository "/tmp") "tracker"))
      (should (equal arguments '("git" "config" "--local" "--get"
                                 "remote.origin.url"))))))

(ert-deftest hermes-tracker-vc-failed-origin-read-has-no-context ()
  "Git failure returns nil instead of inventing repository context."
  (let ((hermes-tracker-vc-hosts '("git@theodora")))
    (cl-letf (((symbol-function 'process-file)
               (lambda (&rest _) (signal 'file-missing '("git")))))
      (should-not (hermes-tracker-vc-repository "/tmp")))
    (cl-letf (((symbol-function 'process-file)
               (lambda (&rest _) "killed")))
      (should-not (hermes-tracker-vc-repository "/tmp")))))

(ert-deftest hermes-tracker-vc-global-config-cannot-create-context ()
  "A global origin cannot create Tracker context outside a repository."
  (skip-unless (executable-find "git"))
  (let ((directory (make-temp-file "hermes-tracker-vc-" t))
        (config (make-temp-file "hermes-tracker-gitconfig-"))
        (hermes-tracker-vc-hosts '("git@theodora"))
        (process-environment (copy-sequence process-environment)))
    (unwind-protect
        (progn
          (with-temp-file config
            (insert "[remote \"origin\"]\n"
                    "\turl = git@theodora:injected.git\n"))
          (setenv "GIT_CONFIG_GLOBAL" config)
          (setenv "GIT_CONFIG_NOSYSTEM" "1")
          (should-not (hermes-tracker-vc-repository directory)))
      (delete-directory directory)
      (delete-file config))))

(ert-deftest hermes-tracker-vc-open-renders-cached-repository ()
  "Current-repository command enters normal SQLite-backed TODO rendering."
  (let (rendered required)
    (cl-letf (((symbol-function 'require)
               (lambda (feature &rest _)
                 (setq required feature)))
              ((symbol-function 'hermes-tracker-vc-repository)
               (lambda (&optional _) "tracker"))
              ((symbol-function 'hermes-tracker--render-todos)
               (lambda (slug name &optional owner)
                 (setq rendered (list slug name owner)))))
      (hermes-tracker-open-current-repository)
      (should (eq required 'hermes-tracker))
      (should (equal rendered '("tracker" "tracker" nil))))))

(ert-deftest hermes-tracker-vc-standalone-command-loads-main ()
  "The VC autoload command establishes its main-package boundary."
  (let ((hermes-tracker-vc-hosts nil))
    (should-error (hermes-tracker-open-current-repository "/tmp")
                  :type 'user-error)
    (should (featurep 'hermes-tracker))))

(provide 'hermes-tracker-vc-tests)
;;; hermes-tracker-vc-tests.el ends here
