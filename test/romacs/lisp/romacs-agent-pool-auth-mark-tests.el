;;; romacs-agent-pool-auth-mark-tests.el --- A refused sign-in marks its account -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A task its account refused the sign-in to fails, saying where and why;
;; the account is marked, by the refused attempt's sign-in, and new work
;; draws on the others.

;;; Code:

(require 'ert)
(require 'cl-lib)

(eval-and-compile
  (when-let* ((file (or load-file-name (bound-and-true-p byte-compile-current-file)))
              (load-path (cons (file-name-directory file) load-path)))
    (unless (fboundp 'romacs-agent-pool-auth-tests--call)
      ;; Its fixture, without running its suite again in this file.
      (cl-letf (((symbol-function 'ert-set-test) #'ignore))
        (load "romacs-agent-pool-auth-tests" nil t)))))

(ert-deftest romacs-agent-pool-auth-mark-a-refused-task-fails-saying-where-and-why ()
  ;; Launched on its pool's pick and refused: it fails there, its result
  ;; naming the account and the reason, for its parent to delegate again;
  ;; the account is marked and new work draws on the next.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (romacs-agent-pool-auth-tests--delegate root "claude")
      (drain)
      (let ((task (romacs-agent-pool-auth-tests--task (caar launches))))
        (should (equal (mapcar #'cadr launches) '("claude")))
        (should (equal (gethash "status" task) "failed"))
        (should (equal (gethash "result" task)
                       (concat "Sign-in refused on claude: " romacs-agent-pool-auth-tests--reason)))
        (romacs-agent-pool-auth-tests--unavailable "claude")
        (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))))))

(defun romacs-agent-pool-auth-mark-tests--run (session label &optional provider)
  "Feed SESSION the run note of a launch on PROVIDER whose credentials
stamp was the one named LABEL (`romacs-agent-pool-auth-tests--stamp')."
  (romacs-agent-pool-auth-tests--record
   session "note" (apply #'romacs-mcp--object "type" "run" "credentials" label "stamp" "0"
                         (and provider (list "provider" provider)))))

(defun romacs-agent-pool-auth-mark-tests--other-editor ()
  "A second editor on the same state directory: a function of BODY (a
function) that calls it with that editor's own usage and sign-in caches,
kept from call to call."
  (let ((room (make-hash-table :test #'equal)) (sign-in nil))
    (lambda (body)
      (let ((romacs-agent--room room) (romacs-agent--room-loaded t)
            (romacs-agent--sign-in-cache sign-in))
        (prog1 (funcall body) (setq sign-in romacs-agent--sign-in-cache))))))

(defun romacs-agent-pool-auth-mark-tests--pick (editor)
  "The account EDITOR's next pick of claude's pool draws on."
  (funcall editor (lambda () (plist-get (romacs-agent-pool-pick 'claude) :name))))

(ert-deftest romacs-agent-pool-auth-mark-two-editors-see-it-at-once ()
  ;; One editor sees a login refused, then a turn on it go through: the
  ;; other, its caches warm, skips that account on its very next pick,
  ;; and draws on it again on the one after, with no restart.
  (romacs-agent-pool-auth-tests--with
    (let ((session (romacs-agent-pool-auth-tests--session root "claude"))
          (other (romacs-agent-pool-auth-mark-tests--other-editor)))
      (should (equal (romacs-agent-pool-auth-mark-tests--pick other) "claude"))
      (romacs-agent-pool-auth-tests--fail session)
      (should (equal (romacs-agent-pool-auth-mark-tests--pick other) "claude2"))
      (romacs-agent-pool-auth-tests--succeed session)
      (should (equal (romacs-agent-pool-auth-mark-tests--pick other) "claude")))))

(ert-deftest romacs-agent-pool-auth-mark-a-new-login-holds-everywhere ()
  ;; Its user signs in again: every editor draws on it again, the refusal
  ;; of the old login kept or not.
  (romacs-agent-pool-auth-tests--with
    (let ((other (romacs-agent-pool-auth-mark-tests--other-editor)))
      (should (equal (romacs-agent-pool-auth-mark-tests--pick other) "claude"))
      (romacs-agent-pool-auth-tests--fail (romacs-agent-pool-auth-tests--session root "claude"))
      (should (equal (romacs-agent-pool-auth-mark-tests--pick other) "claude2"))
      (puthash "claude" "after-login" _stamps)
      (should (equal (romacs-agent-pool-auth-mark-tests--pick other) "claude"))
      (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude")))))

(ert-deftest romacs-agent-pool-auth-mark-a-replay-neither-marks-nor-clears ()
  ;; Records read back (not live) say nothing of a login now: a refusal
  ;; among them marks nothing, a turn gone through ends no refusal.
  (romacs-agent-pool-auth-tests--with
    (let ((replayed (romacs-agent-pool-auth-tests--session root "claude"))
          (live (romacs-agent-pool-auth-tests--session root "claude2")))
      (setf (romacs-agent-caught-up replayed) nil)
      (romacs-agent-pool-auth-tests--fail replayed)
      (should (romacs-agent-failed replayed))
      (should (romacs-agent-room "claude"))
      (should-not (romacs-agent-sign-in-refused "claude"))
      (romacs-agent-pool-auth-tests--fail live)
      (romacs-agent-pool-auth-tests--unavailable "claude2")
      (let ((later (romacs-agent-pool-auth-tests--session root "claude2")))
        (setf (romacs-agent-caught-up later) nil)
        (romacs-agent-pool-auth-tests--succeed later))
      (romacs-agent-pool-auth-tests--unavailable "claude2"))))

(ert-deftest romacs-agent-pool-auth-mark-a-quota-window-outlasts-sign-in-events ()
  ;; A refusal and its end leave the account's usage as its reports say:
  ;; a window at its limit reported after the refusal, though of a moment
  ;; before it, still binds once the login holds again.
  (romacs-agent-pool-auth-tests--with
    (let ((session (romacs-agent-pool-auth-tests--session root "claude"))
          (resets (+ (float-time) 3600)))
      (romacs-agent-pool-auth-tests--fail session)
      (romacs-agent-note-room "claude" (list :status 'rejected :used 1.0 :resets resets :window "five_hour"
                                             :time (- (float-time) 5)))
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (romacs-agent-pool-auth-tests--succeed session)
      (let ((room (romacs-agent-room-of "claude")))
        (should (eq (plist-get room :status) 'rejected))
        (should (equal (plist-get room :resets) resets)))
      (should-not (romacs-agent-room "claude"))
      ;; The usage file holds usage alone.
      (should-not (cl-some (lambda (entry) (plist-member (cdr entry) :reason)) (romacs-agent--room-kept))))))

(ert-deftest romacs-agent-pool-auth-mark-the-text-names-the-account-its-run-had ()
  ;; Its run launched on claude (its run note), the session on claude2
  ;; since: the refusal is claude's, in its result and in its mark.
  (romacs-agent-pool-auth-tests--with
    (let ((session (romacs-agent-pool-auth-tests--session root "claude2")))
      (romacs-agent-pool-auth-mark-tests--run session "before-login" "claude")
      (romacs-agent-pool-auth-tests--fail session)
      (should (equal (romacs-agent-reply session)
                     (concat "Sign-in refused on claude: " romacs-agent-pool-auth-tests--reason)))
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (should (romacs-agent-room "claude2")))))

(ert-deftest romacs-agent-pool-auth-mark-a-resume-stamps-the-directory-it-runs ()
  ;; Resumed in the directory it kept, not its instance's configured one:
  ;; that directory's login is the attempt's, and a refusal of it says
  ;; nothing of the instance's own.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (let ((session (romacs-agent-pool-auth-tests--session root "claude")))
        ;; The directory it kept has a login of its own.
        (puthash "claude2" "kept-directory-login" _stamps)
        (setf (romacs-agent-config-dir session) (cdr (assoc "claude2" romacs-agent-providers))
              (romacs-agent-state session) 'ended (romacs-agent-harness-session session) (romacs-agent-id session)
              (romacs-agent-turns session) 1 (romacs-agent-input-seq session) 1
              (romacs-agent-completed-seq session) 2 (romacs-agent-done-seq session) 2
              (romacs-agent-settled-seq session) 2 (romacs-agent-seq session) 2)
        (romacs-agent-tools--send (romacs-mcp--object "taskId" (romacs-agent-id session) "message" "Go on."))
        (drain)
        (should (equal (mapcar #'cadr launches) '("claude")))
        (should (romacs-agent-failed session))
        (should (romacs-agent-room "claude"))
        (should-not (romacs-agent-sign-in-refused "claude"))))))

;;; A write waits for another's, then checks again; stamps are exact and the launched harness's

(defconst romacs-agent-pool-auth-mark-tests--real-stamps
  (list (symbol-function 'romacs-agent--credentials-stamp) (symbol-function 'romacs-agent--home-stamp))
  "The stamp readers as defined, for tests that read real credentials.")

(ert-deftest romacs-agent-pool-auth-mark-a-refusal-waiting-for-the-lock-is-checked-again ()
  ;; Its write waits for another editor's; meanwhile its user signs in again
  ;; and the new login is refused too: the older refusal says nothing of it.
  (romacs-agent-pool-auth-tests--with
    (let* ((older (romacs-agent-pool-auth-tests--session root "claude"))
           (lock (concat (romacs-agent--sign-in-file) ".lock"))
           (released nil)
           (release (lambda ()
                      (unless released
                        (setq released t)
                        (delete-directory lock)
                        (puthash "claude" "after-login" _stamps)
                        (romacs-agent-pool-auth-tests--fail (romacs-agent-pool-auth-tests--session root "claude"))))))
      (make-directory lock t)
      (cl-letf (((symbol-function 'sleep-for) (lambda (&rest _) (funcall release))))
        (romacs-agent-pool-auth-tests--fail older))
      ;; A writer that did not wait: the other editor goes on now.
      (funcall release)
      (accept-process-output nil 0.5)
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2")))))

(ert-deftest romacs-agent-pool-auth-mark-a-refusal-not-written-in-time-is-dropped ()
  ;; Its write gives up on a lock another editor holds; a turn on that
  ;; login then goes through: the login is drawn on, now and after.
  (romacs-agent-pool-auth-tests--with
    (let ((refused (romacs-agent-pool-auth-tests--session root "claude"))
          (passed (romacs-agent-pool-auth-tests--session root "claude"))
          (lock (concat (romacs-agent--sign-in-file) ".lock")))
      (make-directory lock t)
      (romacs-agent-pool-auth-tests--fail refused)
      (delete-directory lock)
      (romacs-agent-pool-auth-tests--succeed passed)
      (accept-process-output nil 0.5)
      (should (romacs-agent-room "claude"))
      (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude")))))

(ert-deftest romacs-agent-pool-auth-mark-a-new-login-a-nanosecond-later-holds ()
  ;; The credentials, a link, rewritten a nanosecond after the refused
  ;; login: a new login, drawn on.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (cl-letf (((symbol-function 'romacs-agent--credentials-stamp) (nth 0 romacs-agent-pool-auth-mark-tests--real-stamps))
                ((symbol-function 'romacs-agent--home-stamp) (nth 1 romacs-agent-pool-auth-mark-tests--real-stamps)))
        (let ((target (expand-file-name "claude-credentials.json" root))
              (link (expand-file-name ".credentials.json" (cdr (assoc "claude" romacs-agent-providers)))))
          (write-region "{}" nil target nil 'silent)
          (set-file-times target '(1700000000123456701 . 1000000000))
          (make-symbolic-link target link)
          (let ((session (romacs-agent-start root "Read the fixture." "Pinned" "claude")))
            (drain)
            (should (romacs-agent-failed session))
            (romacs-agent-pool-auth-tests--unavailable "claude")
            (set-file-times target '(1700000000123456702 . 1000000000))
            (should (romacs-agent-room "claude"))))))))

(ert-deftest romacs-agent-pool-auth-mark-the-launched-harness-stamps-its-login ()
  ;; The instance now names Codex in the same directory; a Claude session
  ;; on it resumes as Claude Code: Claude Code's login is the attempt's,
  ;; and its refusal says nothing of the Codex login the pool draws on.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (cl-letf (((symbol-function 'romacs-agent--credentials-stamp) (nth 0 romacs-agent-pool-auth-mark-tests--real-stamps))
                ((symbol-function 'romacs-agent--home-stamp)
                 (lambda (harness _dir) (romacs-agent-pool-auth-tests--stamp (format "%s-login" harness)))))
        (let* ((dir (cdr (assoc "claude" romacs-agent-providers)))
               (session (romacs-agent-pool-auth-tests--session root "claude")))
          (setf (romacs-agent-state session) 'ended (romacs-agent-harness-session session) (romacs-agent-id session)
                (romacs-agent-turns session) 1 (romacs-agent-input-seq session) 1
                (romacs-agent-completed-seq session) 2 (romacs-agent-done-seq session) 2
                (romacs-agent-settled-seq session) 2 (romacs-agent-seq session) 2)
          (let ((romacs-agent-providers (cons (list "claude" :harness 'codex :config-dir dir)
                                              (cdr romacs-agent-providers))))
            (romacs-agent-tools--send (romacs-mcp--object "taskId" (romacs-agent-id session) "message" "Go on."))
            (drain)
            (should (equal (mapcar #'cadr launches) '("claude")))
            (should (romacs-agent-failed session))
            (should (romacs-agent-room "claude"))
            (should (equal (plist-get (romacs-agent-pool-pick 'codex) :name) "claude"))))))))

;;; A stamp is its file and its time; read back whatever the file holds

(defun romacs-agent-pool-auth-mark-tests--real-credentials (body)
  "Call BODY with the stamp readers as defined: real credentials files."
  (cl-letf (((symbol-function 'romacs-agent--credentials-stamp) (nth 0 romacs-agent-pool-auth-mark-tests--real-stamps))
            ((symbol-function 'romacs-agent--home-stamp) (nth 1 romacs-agent-pool-auth-mark-tests--real-stamps)))
    (funcall body)))

(ert-deftest romacs-agent-pool-auth-mark-two-harnesses-logins-are-two-at-one-time ()
  ;; The instance now names Codex in the directory a Claude session ran
  ;; in, Claude Code's credentials and Codex's written at one moment: the
  ;; refused Claude login is not the Codex one the pool draws on.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (romacs-agent-pool-auth-mark-tests--real-credentials
       (lambda ()
         (let* ((dir (cdr (assoc "claude" romacs-agent-providers)))
                (session (romacs-agent-pool-auth-tests--session root "claude")))
           (dolist (name '(".credentials.json" "auth.json"))
             (write-region "{}" nil (expand-file-name name dir) nil 'silent)
             (set-file-times (expand-file-name name dir) '(1700000000123456701 . 1000000000)))
           (setf (romacs-agent-state session) 'ended (romacs-agent-harness-session session) (romacs-agent-id session)
                 (romacs-agent-turns session) 1 (romacs-agent-input-seq session) 1
                 (romacs-agent-completed-seq session) 2 (romacs-agent-done-seq session) 2
                 (romacs-agent-settled-seq session) 2 (romacs-agent-seq session) 2)
           (let ((romacs-agent-providers (cons (list "claude" :harness 'codex :config-dir dir)
                                               (cdr romacs-agent-providers))))
             (romacs-agent-tools--send (romacs-mcp--object "taskId" (romacs-agent-id session) "message" "Go on."))
             (drain)
             (should (equal (mapcar #'cadr launches) '("claude")))
             (should (romacs-agent-failed session))
             (should (romacs-agent-room "claude"))
             (should (equal (plist-get (romacs-agent-pool-pick 'codex) :name) "claude")))))))))

(ert-deftest romacs-agent-pool-auth-mark-a-login-before-the-epoch-is-refused-too ()
  ;; Credentials dated before 1970, refused: marked like any other, its
  ;; time carried through the run note exactly.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (romacs-agent-pool-auth-mark-tests--real-credentials
       (lambda ()
         (let ((file (expand-file-name ".credentials.json" (cdr (assoc "claude" romacs-agent-providers)))))
           (write-region "{}" nil file nil 'silent)
           (set-file-times file '(-1123456789 . 1000000000))
           (let ((session (romacs-agent-start root "Read the fixture." "Pinned" "claude")))
             (drain)
             (should (romacs-agent-failed session))
             (romacs-agent-pool-auth-tests--unavailable "claude")))))))
  ;; One encoding, exact on either side of the epoch.
  (dolist (time '((-1123456789 . 1000000000) (0 . 1000000000) (1700000000123456701 . 1000000000)))
    (should (equal (romacs-agent--time-of-text (romacs-agent--time-text time)) time))))

(ert-deftest romacs-agent-pool-auth-mark-a-malformed-file-is-no-refusal ()
  ;; Whatever the file holds, a pick reads it without signalling: rows that
  ;; are no refusal (a bad stamp, none) count for nothing, and a file that
  ;; is no list holds none; a live refusal and success go on as ever.
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-mark-tests--real-credentials
     (lambda ()
       (write-region "{}" nil (expand-file-name ".credentials.json" (cdr (assoc "claude" romacs-agent-providers)))
                     nil 'silent)
       (let ((session (romacs-agent-pool-auth-tests--session root "claude")))
         (make-directory (file-name-directory (romacs-agent--sign-in-file)) t)
         (dolist (content (list "((\"claude\" (bad . stamp) . \"bad reason\"))"
                                "((\"claude\" (bad . stamp) . \"bad reason\") (\"claude2\"))"
                                "\"no list at all\""))
           (with-temp-file (romacs-agent--sign-in-file) (insert content))
           (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))
           (should (romacs-agent-room "claude2")))
         (with-temp-file (romacs-agent--sign-in-file)
           (insert "((\"claude\" (bad . stamp) . \"bad reason\"))"))
         (romacs-agent-pool-auth-tests--fail session)
         (romacs-agent-pool-auth-tests--unavailable "claude")
         (romacs-agent-pool-auth-tests--succeed session)
         (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude")))))))

;;; romacs-agent-pool-auth-mark-tests.el ends here
