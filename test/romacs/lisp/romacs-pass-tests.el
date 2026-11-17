;;; romacs-pass-tests.el --- The password manager on a throwaway store  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Each test makes a GnuPG home with a throwaway key that has no
;; passphrase, and a password store encrypted to it, both in a temporary
;; directory (`romacs-pass-tests--with-store'); nothing reads the user's
;; own store or keys.  gpg and pass are run for real.

(require 'ert)
(require 'romacs-auth-source-pass)
(require 'romacs-pass-menu)

(defun romacs-pass-tests--run (program &rest arguments)
  "Run PROGRAM with ARGUMENTS, and fail unless it succeeds."
  (with-temp-buffer
    (let ((status (apply #'call-process program nil t nil arguments)))
      (unless (eql status 0)
        (error "%s %S failed (%s): %s" program arguments status (buffer-string))))))

(defun romacs-pass-tests--insert (entry text)
  "Store TEXT as ENTRY with pass itself."
  (with-temp-buffer
    (insert text)
    (let ((status (call-process-region (point-min) (point-max) "pass" nil t nil
                                       "insert" "--multiline" "--force" entry)))
      (unless (eql status 0) (error "pass insert %s failed: %s" entry (buffer-string))))))

(defun romacs-pass-tests--call (entries body)
  "Call BODY with a throwaway key and a store holding ENTRIES, (NAME . TEXT)s."
  (unless (and (executable-find "gpg") (executable-find "gpgconf") (executable-find "pass"))
    (ert-skip "needs gpg, gpgconf and pass"))
  (let* ((root (make-temp-file "romacs-pass-" t))
         (gnupg (expand-file-name "gnupg" root))
         (store (expand-file-name "store" root))
         (process-environment (append (list (concat "GNUPGHOME=" gnupg)
                                            (concat "PASSWORD_STORE_DIR=" store))
                                      process-environment))
         (auth-source-pass-filename store)
         (auth-sources '(password-store))
         (auth-source-do-cache nil))
    (make-directory gnupg)
    (set-file-modes gnupg #o700)
    (unwind-protect
        (progn
          (romacs-pass-tests--run "gpg" "--batch" "--pinentry-mode" "loopback" "--passphrase" ""
                                  "--quick-generate-key" "Romacs Pass Test <pass-test@example.invalid>"
                                  "default" "default" "never")
          (romacs-pass-tests--run "pass" "init" "pass-test@example.invalid")
          (dolist (entry entries) (romacs-pass-tests--insert (car entry) (cdr entry)))
          (funcall body))
      (call-process "gpgconf" nil nil nil "--kill" "gpg-agent")
      (delete-directory root t))))

(defmacro romacs-pass-tests--with-store (entries &rest body)
  "Run BODY with a throwaway key and a store holding ENTRIES, (NAME . TEXT)s."
  (declare (indent 1))
  `(romacs-pass-tests--call ,entries (lambda () ,@body)))

(defun romacs-pass-tests--wait (predicate)
  "Run processes and timers until PREDICATE holds, for up to 30 seconds."
  (let ((deadline (+ (float-time) 30)))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil 0.05)))
  (should (funcall predicate)))

(defun romacs-pass-tests--wiped-p (string)
  "Whether STRING was wiped by `clear-string'."
  (string-match-p "\\`\0*\\'" string))

(defconst romacs-pass-tests--entries
  '(("email/work" . "hunter2\nuser: me@example.org\nurl: https://mail.example.org\n")
    ("xmpp/chat.example.org/thanos" . "jabber-secret\n")
    ("smtp/127.0.0.1/me@example.org" . "smtp-secret\n")
    ("web/otp" . "web-secret\notpauth://totp/Example:me?secret=JBSWY3DPEHPK3PXP&issuer=Example\n"))
  "A store's entries, as (NAME . TEXT).")

;;;; The store

(ert-deftest romacs-pass-lists-and-reads-the-store ()
  "The store's entries are its .gpg files by name, and each decrypts
through gpg to the text pass stored."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (should (equal (romacs-pass-entries) (sort (mapcar #'car romacs-pass-tests--entries) #'string<)))
    (should (equal (romacs-pass-read "email/work") (cdr (assoc "email/work" romacs-pass-tests--entries))))
    (should (equal (romacs-pass--parse (romacs-pass-read "email/work"))
                   '((secret . "hunter2") ("user" . "me@example.org") ("url" . "https://mail.example.org"))))
    (should-error (romacs-pass-read "no/such") :type 'user-error)))

(ert-deftest romacs-pass-without-a-store-has-no-entries ()
  "A store directory that does not exist holds nothing, and auth-source
finds nothing in it, without an error."
  (let ((auth-source-pass-filename (expand-file-name "romacs-pass-absent" temporary-file-directory))
        (auth-sources '(password-store))
        (auth-source-do-cache nil))
    (should-not (romacs-pass-entries))
    (should-not (auth-source-search :host "example.org" :max 1))))

;;;; Copies

(ert-deftest romacs-pass-copy-leaves-after-its-time ()
  "A copied password is a secret in the kill ring, the clipboard and the
Edit menu's yank list, which all lose it after
`romacs-pass-clear-seconds', and it is wiped; the command returns nil,
so its caller never sees the secret."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let* ((romacs-pass-clear-seconds 0.5)
           (kill-ring (list "older"))
           (kill-ring-yank-pointer kill-ring)
           (yank-menu (list "Select Yank"))
           (clipboard nil)
           (interprogram-cut-function (lambda (text) (setq clipboard (copy-sequence text))))
           (interprogram-paste-function #'ignore))
      (cl-letf (((symbol-function 'gui-get-selection) (lambda (&rest _) clipboard)))
        (should-not (romacs-pass-copy "email/work"))
        (romacs-pass-tests--wait (lambda () (equal (car kill-ring) "hunter2")))
        (let ((copy (car kill-ring)))
          (should (get-text-property 0 'romacs-secret copy))
          (should (equal clipboard "hunter2"))
          (when (fboundp 'menu-bar-update-yank-menu)
            (should (seq-some (lambda (item) (eq (car-safe item) copy)) (cdr yank-menu))))
          (romacs-pass-tests--wait (lambda () (equal kill-ring '("older"))))
          (should (equal clipboard ""))
          (should (eq kill-ring-yank-pointer kill-ring))
          (should (romacs-pass-tests--wiped-p copy))
          (should-not (memq copy romacs-pass--copies))
          (should-not (cdr yank-menu)))))))

(ert-deftest romacs-pass-autotype-refuses-a-focused-romacs ()
  "Typing waits for another window to have the focus: with a Romacs frame
focused, nothing is typed and the strings are wiped."
  (let ((romacs-pass-autotype-delay 0)
        (user (copy-sequence "me")) (secret (copy-sequence "hunter2")) (typed nil))
    (cl-letf (((symbol-function 'romacs-pass--focused-p) #'always)
              ((symbol-function 'romacs-pass--type) (lambda (&rest _) (setq typed t))))
      (romacs-pass--type-later user secret)
      (romacs-pass-tests--wait (lambda () (romacs-pass-tests--wiped-p secret))))
    (should-not typed)
    (should (romacs-pass-tests--wiped-p user))))

(ert-deftest romacs-pass-otp-copies-the-current-code ()
  "The one-time code is the entry's TOTP code now, six digits."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let ((romacs-pass-clear-seconds 60) (kill-ring nil) (kill-ring-yank-pointer nil)
          (text (cdr (assoc "web/otp" romacs-pass-tests--entries))))
      (romacs-pass-otp "web/otp")
      (romacs-pass-tests--wait (lambda () (car kill-ring)))
      (let ((now (floor (float-time))))
        (should (string-match-p "\\`[0-9]\\{6\\}\\'" (car kill-ring)))
        (should (member (car kill-ring) (list (car (romacs-pass--otp text now))
                                              (car (romacs-pass--otp text (- now 30)))))))
      (should-not (romacs-pass--otp "pw\nuser: me\n" 0))
      (should-error (romacs-pass--otp "pw\notpauth://hotp/x?secret=JBSWY3DP&counter=1\n" 0)))))

;;;; Writing

(ert-deftest romacs-pass-insert-and-generate-write-through-pass ()
  "An inserted password is stored as given; a generated one is copied and
stored, and a new password for an existing entry keeps its fields."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let ((romacs-pass-clear-seconds 60) (kill-ring nil) (kill-ring-yank-pointer nil))
      (romacs-pass-insert "new/inserted" "typed-secret")
      (romacs-pass-tests--wait (lambda () (romacs-pass-entry-p "new/inserted")))
      (should (equal (romacs-pass-read "new/inserted") "typed-secret\n"))
      (romacs-pass-generate "new/generated")
      (romacs-pass-tests--wait (lambda () (car kill-ring)))
      (let ((generated (car kill-ring)))
        (should (= (length generated) romacs-pass-generate-length))
        (should (equal (romacs-pass-read "new/generated") (concat generated "\n"))))
      (cl-letf (((symbol-function 'yes-or-no-p) #'always))
        (romacs-pass-generate "email/work" 12))
      (romacs-pass-tests--wait (lambda () (= (length (car kill-ring)) 12)))
      (should (equal (romacs-pass-read "email/work")
                     (concat (car kill-ring) "\nuser: me@example.org\nurl: https://mail.example.org\n")))
      (should-error (romacs-pass-generate "new/empty" 0) :type 'user-error))))

(ert-deftest romacs-pass-edit-stores-the-buffer ()
  "An entry is edited in a buffer that makes no auto-save and refuses a
warm restart while modified; storing it writes the text and kills it."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (romacs-pass-edit "email/work")
    (romacs-pass-tests--wait (lambda () (and (get-buffer "*pass: email/work*")
                                             (not (buffer-local-value 'romacs-pass--decrypting
                                                                      (get-buffer "*pass: email/work*"))))))
    (with-current-buffer "*pass: email/work*"
      (should (equal (buffer-string) (cdr (assoc "email/work" romacs-pass-tests--entries))))
      (should-not buffer-file-name)
      (should-not buffer-auto-save-file-name)
      (should-not (romacs-pass--restart-blocker))
      (should (get-text-property 0 'romacs-secret (filter-buffer-substring 1 8)))
      (goto-char (point-max))
      (insert "note: changed\n")
      (should (romacs-pass--restart-blocker))
      ;; Asking again shows the changed buffer as it is.
      (romacs-pass-edit "email/work")
      (should (string-suffix-p "note: changed\n" (buffer-string)))
      (romacs-pass-edit-save)
      (should buffer-read-only))
    (romacs-pass-tests--wait (lambda () (not (get-buffer "*pass: email/work*"))))
    (should (string-suffix-p "note: changed\n" (romacs-pass-read "email/work")))
    (romacs-pass-edit "new/edited")
    (with-current-buffer "*pass: new/edited*"
      (insert "fresh\n")
      (romacs-pass-edit-save))
    (romacs-pass-tests--wait (lambda () (romacs-pass-entry-p "new/edited")))
    (should (equal (romacs-pass-read "new/edited") "fresh\n"))))

;;;; Waiting for gpg

(defun romacs-pass-tests--slow-gpg (seconds)
  "A gpg that waits SECONDS before it runs: a program file in the test's
temporary directory, for `epg-gpg-program'."
  (let ((file (make-temp-file "slow-gpg-" nil ".sh"
                              (format "#!/bin/sh\nsleep %s\nexec %s \"$@\"\n"
                                      seconds (shell-quote-argument (executable-find "gpg"))))))
    (set-file-modes file #o700)
    file))

(ert-deftest romacs-pass-edit-shows-at-once-and-fills-later ()
  "A slow gpg leaves the edit buffer shown at once, empty, read-only and
not storable, C-g stopping it; its text comes when gpg answers, and C-g is
itself again."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let ((epg-gpg-program (romacs-pass-tests--slow-gpg 1)))
      (romacs-pass-edit "email/work")
      (let ((buffer (get-buffer "*pass: email/work*")))
        (should buffer)
        (with-current-buffer buffer
          (should (process-live-p romacs-pass--decrypting))
          (should buffer-read-only)
          (should (equal (buffer-string) ""))
          (should (equal (substring-no-properties header-line-format)
                         "Decrypting email/work...  C-g or C-c C-k stops"))
          (should (eq (key-binding (kbd "C-g")) #'romacs-pass-edit-cancel))
          (should-error (romacs-pass-edit-save) :type 'user-error))
        ;; Asking again while it waits shows the same buffer, still waiting.
        (romacs-pass-edit "email/work")
        (should (buffer-local-value 'romacs-pass--decrypting buffer))
        (romacs-pass-tests--wait (lambda () (not (buffer-local-value 'romacs-pass--decrypting buffer))))
        (with-current-buffer buffer
          (should (equal (buffer-string) (cdr (assoc "email/work" romacs-pass-tests--entries))))
          (should-not buffer-read-only)
          (should-not (buffer-modified-p))
          (should (eq (key-binding (kbd "C-g")) #'keyboard-quit)))
        (kill-buffer buffer)))))

(ert-deftest romacs-pass-edit-cancel-stops-gpg-quietly ()
  "Cancelling a buffer that waits for gpg kills both, reports no failure,
and leaves the entry as it was."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let ((epg-gpg-program (romacs-pass-tests--slow-gpg 5))
          (said nil))
      (romacs-pass-edit "email/work")
      (let* ((buffer (get-buffer "*pass: email/work*"))
             (process (buffer-local-value 'romacs-pass--decrypting buffer)))
        (cl-letf (((symbol-function 'message)
                   (lambda (format &rest args) (push (apply #'format-message format args) said))))
          (with-current-buffer buffer (romacs-pass-edit-cancel))
          (should-not (buffer-live-p buffer))
          (romacs-pass-tests--wait (lambda () (not (process-live-p process))))
          (accept-process-output nil 0.1))
        (should-not (seq-some (lambda (line) (string-search "could not decrypt" line)) said))))
    (should (equal (romacs-pass-read "email/work") (cdr (assoc "email/work" romacs-pass-tests--entries))))))

(ert-deftest romacs-pass-notes-a-slow-decryption-by-its-times ()
  "A decryption over `romacs-pass-slow-decrypt-seconds' goes on the usage
log's record with its times and outcome, never the entry's name; the
commands say at once that gpg is at work."
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let ((romacs-pass-slow-decrypt-seconds 0)
          (romacs-pass-clear-seconds 60) (kill-ring nil) (kill-ring-yank-pointer nil)
          (notes nil) (said nil))
      (cl-letf (((symbol-function 'romacs-usage-log-note)
                 (lambda (kind &rest plist) (push (cons kind plist) notes)))
                ((symbol-function 'message)
                 (lambda (format &rest args) (when format (push (apply #'format-message format args) said)))))
        (romacs-pass-copy "email/work")
        (should (equal (car said) "Decrypting email/work..."))
        (romacs-pass-tests--wait (lambda () (car kill-ring)))
        ;; Written as it is, not through EPA's encrypting handler for .gpg.
        (let ((file-name-handler-alist nil))
          (write-region "not an OpenPGP message" nil
                        (expand-file-name "broken.gpg" auth-source-pass-filename) nil 'silent))
        (should-error (romacs-pass-read "broken")))
      (should (= (length notes) 2))
      (pcase-let ((`(,failed ,ok) notes))
        (should (equal (car ok) "pass-decrypt"))
        (should (equal (plist-get (cdr ok) :status) "ok"))
        (should (natnump (plist-get (cdr ok) :ms)))
        (should (<= (plist-get (cdr ok) :output_ms) (plist-get (cdr ok) :ms)))
        (should (floatp (plist-get (cdr ok) :t)))
        (should (equal (plist-get (cdr failed) :status) "failed")))
      (should-not (seq-some (lambda (note) (string-search "email" (format "%S" note))) notes)))))

;;;; auth-source

(ert-deftest romacs-pass-answers-jabber ()
  "jabber's own password lookup finds its account's entry by
auth-source-pass's naming, with no configuration but the store."
  (require 'jabber-util)
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (should (equal (jabber-read-password "thanos@chat.example.org") "jabber-secret"))
    (should (equal (auth-source-pass-get 'secret "email/work") "hunter2"))
    (should (equal (auth-source-pass-get "user" "email/work") "me@example.org"))
    ;; auth-source keeps what it finds: a one-time code's seed is not part of it.
    (should (equal (auth-source-pass-parse-entry "web/otp") '((secret . "web-secret"))))
    (let ((found (car (auth-source-search :host "chat.example.org" :user "thanos" :port "xmpp"
                                          :max 1 :require '(:secret)))))
      (should (equal (auth-info-password found) "jabber-secret")))))

(defun romacs-pass-tests--smtp-server (heard)
  "A local SMTP server that accepts any mail, and calls HEARD with each
line it reads.  It offers AUTH PLAIN only."
  (make-network-process
   :name "romacs-pass-tests-smtp" :server t :host "127.0.0.1" :service t :family 'ipv4
   :noquery t :coding 'binary
   :sentinel (lambda (process event)
               (when (string-prefix-p "open" event)
                 (process-send-string process "220 localhost ESMTP\r\n")))
   :filter
   (lambda (process chunk)
     (let ((pending (concat (or (process-get process :pending) "") chunk)))
       (while (string-match "\r?\n" pending)
         (let ((line (substring pending 0 (match-beginning 0))))
           (setq pending (substring pending (match-end 0)))
           (funcall heard line)
           (process-send-string
            process
            (cond ((process-get process :data)
                   (if (equal line ".") (progn (process-put process :data nil) "250 queued\r\n") ""))
                  ((string-match-p "\\`EHLO" line) "250-localhost\r\n250 AUTH PLAIN\r\n")
                  ((string-match-p "\\`AUTH PLAIN \\S-" line) "235 accepted\r\n")
                  ((string-match-p "\\`AUTH PLAIN" line) "334 \r\n")
                  ((string-match-p "\\`DATA" line) (process-put process :data t) "354 go on\r\n")
                  ((string-match-p "\\`QUIT" line) "221 bye\r\n")
                  ((process-get process :auth-wanted) "235 accepted\r\n")
                  (t "250 ok\r\n")))
           (process-put process :auth-wanted (string-match-p "\\`AUTH PLAIN\\'" line))))
       (process-put process :pending pending)))))

(ert-deftest romacs-pass-answers-smtpmail ()
  "smtpmail sending to a server that wants a login authenticates with the
store's password for the server and user, found through auth-source."
  (require 'smtpmail)
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (let* ((lines nil)
           (server (romacs-pass-tests--smtp-server (lambda (line) (push line lines)))))
      (unwind-protect
          (with-temp-buffer
            (insert "From: me@example.org\nTo: you@example.org\nSubject: test\n"
                    mail-header-separator "\nbody\n")
            (let ((smtpmail-smtp-server "127.0.0.1")
                  (smtpmail-smtp-service (process-contact server :service))
                  (smtpmail-smtp-user "me@example.org")
                  (smtpmail-stream-type 'plain)
                  (smtpmail-queue-mail nil)
                  (user-mail-address "me@example.org"))
              (smtpmail-send-it)))
        (delete-process server))
      ;; AUTH PLAIN's response is NUL, user, NUL, password, in base64, on
      ;; the AUTH line or the line after it.
      (should (seq-some (lambda (line)
                          (let ((decoded (ignore-errors
                                           (base64-decode-string
                                            (replace-regexp-in-string "\\`AUTH PLAIN " "" line)))))
                            (and decoded (string-suffix-p "\0me@example.org\0smtp-secret" decoded))))
                        lines)))))

;;;; The menu and images

(ert-deftest romacs-pass-menu-folds-folders ()
  "The list shows a closed folder's line only, and its entries once it
is open."
  (should (equal (romacs-pass-menu--rows '("a/b/c" "a/d" "e") nil)
                 '((folder "a/" 0) (entry "e" 0))))
  (should (equal (romacs-pass-menu--rows '("a/b/c" "a/d" "e") '("a/"))
                 '((folder "a/" 0) (folder "a/b/" 1) (entry "a/d" 1) (entry "e" 0))))
  (romacs-pass-tests--with-store romacs-pass-tests--entries
    (romacs-pass)
    (with-current-buffer "*Password Store*"
      (should (equal (romacs-pass-menu--at) '(folder . "email/")))
      (romacs-pass-menu-next 10)
      (should (equal (romacs-pass-menu--at) '(folder . "xmpp/")))
      (romacs-pass-menu-previous 10)
      (romacs-pass-menu-open)
      (should (equal (romacs-pass-menu--at) '(entry . "email/work")))
      (romacs-pass-menu-close)
      (should (equal (romacs-pass-menu--at) '(folder . "email/")))
      (should-not (string-search "work" (buffer-string)))
      (kill-buffer))))

(ert-deftest romacs-pass-secrets-stay-out-of-session-images ()
  "A session image keeps the kill ring's other strings, not its secrets."
  (require 'warm-restart)
  (let* ((kill-ring (list "plain" (romacs-pass--secret (copy-sequence "secret")) "older"))
         (kill-ring-yank-pointer (cddr kill-ring))
         (register-alist (list (cons ?s (romacs-pass--secret (copy-sequence "secret")))
                               ;; A secret appended to ordinary text, as `append-to-register' makes it.
                               (cons ?t (concat "plain " (romacs-pass--secret (copy-sequence "secret"))))))
         (globals (warm-restart--globals)))
    (should (equal (plist-get globals :kill-ring) '("plain" "older")))
    (should (equal (plist-get globals :yank-index) 1))
    (should-not (plist-get globals :registers))))

(provide 'romacs-pass-tests)
;;; romacs-pass-tests.el ends here
