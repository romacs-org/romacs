;;; romacs-pass.el --- The password manager, on the password store  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Romacs's password manager keeps its passwords in pass(1)'s store: a
;; directory of gpg-encrypted entries (`auth-source-pass-filename', from
;; PASSWORD_STORE_DIR or ~/.password-store), each a text whose first line
;; is the password and whose other lines are `key: value' fields, an
;; `otpauth://' URI among them for one-time codes.
;;
;; `romacs-pass-copy' copies a password, `romacs-pass-otp' a one-time
;; code, `romacs-pass-autotype' types the user name and password into the
;; focused window, `romacs-pass-edit' edits an entry in a buffer,
;; `romacs-pass-insert' and `romacs-pass-generate' add one, and
;; `romacs-pass-launcher' and the `romacs-pass' list
;; (romacs-pass-menu.el) reach them all.  `romacs-pass-map' holds them
;; on keys, for the init to bind.  The auth-source library finds its
;; secrets here too (romacs-auth-source-pass.el), so the packages that
;; ask it, mail and chat among them, need no configuration.
;;
;; Secrets are read when they are used, never before.  gpg decrypts
;; through gpg-agent, which asks for the key's passphrase with the
;; configured pinentry; the commands wait for it without blocking
;; Romacs.  An entry's plaintext is read once (crate::pass splits it in
;; Rust) and wiped with `clear-string' when its reader is done; every
;; secret string handed out carries the `romacs-secret' text property,
;; which a session image never stores (warm-restart.el).  A copy leaves
;; the kill ring and the clipboard after `romacs-pass-clear-seconds'.
;; Writes go through pass itself (`pass insert'), which encrypts to the
;; store's recipients and commits to its Git repository when it has one;
;; plaintext is given on a pipe and never touches the disk.
;;
;; Wiping is best effort: a collection may have copied a string before
;; it was cleared, and a buffer's text leaves no copy behind only as far
;; as its storage is reused.

;;; Code:

(require 'subr-x)
(require 'epg-config)

(defvar auth-source-pass-filename)
(defvar warm-restart-blocker-functions)

(defgroup romacs-pass nil
  "Passwords in the password store, pass(1)'s."
  :group 'applications
  :prefix "romacs-pass-")

(defcustom romacs-pass-clear-seconds 45
  "Seconds a copied password or code stays in the kill ring and clipboard.
pass(1) clears its own copies after 45."
  :type 'natnum)

(defcustom romacs-pass-generate-length 25
  "Characters in a generated password, as pass(1)'s default."
  :type 'natnum)

(defcustom romacs-pass-generate-symbols t
  "Whether generated passwords use punctuation besides letters and digits."
  :type 'boolean)

(defcustom romacs-pass-program "pass"
  "The pass(1) program, which writes entries."
  :type 'string)

(defcustom romacs-pass-autotype-program "wtype"
  "The program `romacs-pass-autotype' types with: wtype(1) on Wayland."
  :type 'string)

;;;; The store

(defun romacs-pass-directory ()
  "The password store's directory, as a directory name.
It is `auth-source-pass-filename' once that option is defined, else the
value its definition gives it: PASSWORD_STORE_DIR, or ~/.password-store."
  (file-name-as-directory
   (expand-file-name (if (boundp 'auth-source-pass-filename)
                         auth-source-pass-filename
                       (or (getenv "PASSWORD_STORE_DIR") "~/.password-store")))))

(defun romacs-pass-entries ()
  "The store's entry names, sorted; none when it has no directory."
  (let ((directory (romacs-pass-directory)))
    (and (file-directory-p directory) (romacs-pass--entries directory))))

(defun romacs-pass--file (entry)
  "The file holding ENTRY."
  (expand-file-name (concat entry ".gpg") (romacs-pass-directory)))

(defun romacs-pass-entry-p (entry)
  "Whether the store holds ENTRY."
  (file-exists-p (romacs-pass--file entry)))

(defun romacs-pass--secret (string)
  "STRING, marked as a secret with the `romacs-secret' property."
  (put-text-property 0 (length string) 'romacs-secret t string)
  string)

;;;; Decryption

(defun romacs-pass--gpg (entry done)
  "Start gpg decrypting ENTRY and return its process.
DONE is called with the text, decoded, and nil when gpg succeeds, or
with nil and gpg's own message when it fails; the text is DONE's own.
gpg asks gpg-agent for the key, which asks for its passphrase through
the configured pinentry.  This is the one place that decrypts an
entry."
  (let ((file (romacs-pass--file entry))
        (chunks nil)
        (errors (generate-new-buffer " *romacs-pass gpg*")))
    (unless (file-readable-p file)
      (kill-buffer errors)
      (user-error "No password-store entry %s" entry))
    (make-process
     :name "romacs-pass-gpg"
     :command (list epg-gpg-program "--batch" "--quiet" "--decrypt" "--" file)
     :connection-type 'pipe :coding 'binary :noquery t :buffer nil :stderr errors
     :filter (lambda (_process chunk) (push chunk chunks))
     :sentinel
     (lambda (process _event)
       (unless (process-live-p process)
         (let ((raw (apply #'concat (nreverse chunks)))
               (why (and (buffer-live-p errors)
                         (string-trim (with-current-buffer errors (buffer-string))))))
           (mapc #'clear-string chunks)
           (setq chunks nil)
           (romacs-pass--kill-buffer errors)
           (if (and (eq (process-status process) 'exit) (zerop (process-exit-status process)))
               (let ((text (decode-coding-string raw 'utf-8)))
                 (clear-string raw)
                 (funcall done text nil))
             (clear-string raw)
             (funcall done nil (format "gpg could not decrypt %s%s" entry
                                       (if (string-empty-p (or why "")) "" (concat ": " why)))))))))))

(defun romacs-pass--kill-buffer (buffer)
  "Kill BUFFER and the stderr pipe still reading into it, asking nothing."
  (when (buffer-live-p buffer)
    (when-let* ((process (get-buffer-process buffer)))
      (delete-process process))
    (kill-buffer buffer)))

(defun romacs-pass-with-entry (entry function)
  "Call FUNCTION with ENTRY's decrypted text once gpg has it, then wipe it.
Return at once: gpg runs, and its passphrase prompt waits, without
blocking.  A failure is reported in the echo area."
  (romacs-pass--gpg entry (lambda (text why)
                            (if (null text)
                                (message "%s" why)
                              (unwind-protect (funcall function text)
                                (clear-string text))))))

(defun romacs-pass-read (entry)
  "ENTRY's decrypted text, waiting for gpg; the caller wipes it.
For callers that must answer at once, as auth-source's do.  \\[keyboard-quit]
stops the wait and gpg."
  (let* ((answer nil)
         (process (romacs-pass--gpg entry (lambda (text why) (setq answer (cons text why))))))
    (unwind-protect
        (while (not answer)
          (accept-process-output (and (process-live-p process) process) 0.05))
      (unless answer (delete-process process)))
    (or (car answer) (error "%s" (cdr answer)))))

(defun romacs-pass--field (data field)
  "FIELD of DATA, an entry parsed by `romacs-pass--parse'.
FIELD is `secret' or a key; `user' is also found as `username', as
auth-source-pass finds it."
  (or (cdr (assoc field data))
      (and (equal field "user") (cdr (assoc "username" data)))))

(defun romacs-pass--wipe-data (data)
  "Wipe each string of DATA, a parsed entry."
  (dolist (cell data)
    (when (stringp (car cell)) (clear-string (car cell)))
    (when (stringp (cdr cell)) (clear-string (cdr cell)))))

;;;; Writing

(defun romacs-pass--write (entry text done)
  "Store TEXT as ENTRY through `pass insert', replacing what it held.
DONE is called with nil once pass succeeds, or with its message."
  (let* ((output (generate-new-buffer " *romacs-pass insert*"))
         (process-environment (cons (concat "PASSWORD_STORE_DIR="
                                            (directory-file-name (romacs-pass-directory)))
                                    process-environment))
         (process
          (make-process
           :name "romacs-pass-insert"
           :command (list romacs-pass-program "insert" "--multiline" "--force" "--" entry)
           :connection-type 'pipe :coding 'utf-8 :noquery t :buffer output
           :sentinel
           (lambda (process _event)
             (unless (process-live-p process)
               (let ((why (with-current-buffer output (string-trim (buffer-string)))))
                 (kill-buffer output)
                 (funcall done (unless (and (eq (process-status process) 'exit)
                                            (zerop (process-exit-status process)))
                                 (format "pass could not store %s: %s" entry why)))))))))
    (process-send-string process (if (string-suffix-p "\n" text) text (concat text "\n")))
    (process-send-eof process)
    process))

(defun romacs-pass--write-reported (entry text then)
  "Store TEXT as ENTRY, wipe TEXT, then call THEN with no arguments.
A failure is reported in the echo area instead."
  (romacs-pass--write entry text
                      (lambda (why)
                        (clear-string text)
                        (if why (message "%s" why) (funcall then)))))

;;;; Copies

(defun romacs-pass--copy (secret what)
  "Put a copy of SECRET in the kill ring and clipboard for a while.
WHAT names it in the message.  The copy leaves both, and is wiped,
after `romacs-pass-clear-seconds'."
  (let ((copy (romacs-pass--secret (copy-sequence secret)))
        (kill-do-not-save-duplicates nil))
    (kill-new copy)
    (run-at-time romacs-pass-clear-seconds nil #'romacs-pass--forget copy)
    (message "Copied %s; cleared in %s s" what romacs-pass-clear-seconds)))

(defun romacs-pass--forget (copy)
  "Take COPY out of the kill ring, and the clipboard while it holds it.
Then wipe it."
  (when (eq (car kill-ring-yank-pointer) copy)
    (setq kill-ring-yank-pointer (cdr kill-ring-yank-pointer)))
  (setq kill-ring (delq copy kill-ring))
  (unless (memq (car kill-ring-yank-pointer) kill-ring)
    (setq kill-ring-yank-pointer kill-ring))
  (when (and interprogram-cut-function
             (equal (ignore-errors (gui-get-selection 'CLIPBOARD 'UTF8_STRING)) copy))
    (funcall interprogram-cut-function ""))
  (clear-string copy))

;;;; Reading names

(defvar romacs-pass-history nil "Entries read by the password manager.")

(defun romacs-pass-read-entry (prompt &optional new)
  "Read an entry's name with PROMPT, completing over the store's.
Unless NEW, it must name an entry."
  (let ((name (completing-read prompt (romacs-pass-entries) nil (not new) nil 'romacs-pass-history)))
    (if (string-empty-p name) (user-error "No entry named") name)))

;;;; Commands

;;;###autoload
(defun romacs-pass-copy (entry)
  "Copy ENTRY's password for `romacs-pass-clear-seconds'."
  (interactive (list (romacs-pass-read-entry "Copy password: ")))
  (romacs-pass-with-entry
   entry (lambda (text)
           (let ((data (romacs-pass--parse text)))
             (unwind-protect
                 (if-let* ((secret (romacs-pass--field data 'secret)))
                     (romacs-pass--copy secret (format "%s's password" entry))
                   (message "%s has no password" entry))
               (romacs-pass--wipe-data data)))))
  nil)

;;;###autoload
(defun romacs-pass-otp (entry)
  "Copy ENTRY's current one-time code for `romacs-pass-clear-seconds'.
The code is computed from the entry's `otpauth://totp/' URI."
  (interactive (list (romacs-pass-read-entry "Copy one-time code: ")))
  (romacs-pass-with-entry
   entry (lambda (text)
           (pcase (romacs-pass--otp text (floor (float-time)))
             ('nil (message "%s has no otpauth:// URI" entry))
             (`(,code . ,valid)
              (unwind-protect (romacs-pass--copy code (format "%s's code (valid %d s)" entry valid))
                (clear-string code))))))
  nil)

;;;###autoload
(defun romacs-pass-autotype (entry)
  "Type ENTRY's user name, Tab and password into the focused window.
The user name is its `user' (or `username') field, else the entry's last
name component.  The password reaches `romacs-pass-autotype-program'
on its standard input, never its command line."
  (interactive (list (romacs-pass-read-entry "Type: ")))
  (unless (executable-find romacs-pass-autotype-program)
    (user-error "Autotype needs %s" romacs-pass-autotype-program))
  (romacs-pass-with-entry
   entry (lambda (text)
           (let ((data (romacs-pass--parse text)))
             (unwind-protect
                 (let ((user (or (romacs-pass--field data "user") (file-name-nondirectory entry)))
                       (secret (romacs-pass--field data 'secret)))
                   (if (null secret)
                       (message "%s has no password" entry)
                     (romacs-pass--type user secret)))
               (romacs-pass--wipe-data data)))))
  nil)

(defun romacs-pass--type (user secret)
  "Type USER, Tab and SECRET with `romacs-pass-autotype-program'."
  (let ((process (make-process
                  :name "romacs-pass-type" :noquery t :buffer nil :connection-type 'pipe :coding 'utf-8
                  :command (list romacs-pass-autotype-program "-s" "300" user "-k" "Tab" "-")
                  :sentinel (lambda (process _event)
                              (unless (or (process-live-p process) (zerop (process-exit-status process)))
                                (message "%s failed (%s)" romacs-pass-autotype-program
                                         (process-exit-status process)))))))
    (process-send-string process secret)
    (process-send-eof process)))

;;;###autoload
(defun romacs-pass-insert (entry password)
  "Add ENTRY to the store holding PASSWORD, asked for twice."
  (interactive (let ((entry (romacs-pass-read-entry "Insert entry: " t)))
                 (romacs-pass--refuse-existing entry)
                 (list entry (read-passwd (format "Password for %s: " entry) t))))
  (romacs-pass--write-reported entry password (lambda () (message "Stored %s" entry))))

(defun romacs-pass--refuse-existing (entry)
  "Signal unless ENTRY is new, or the user agrees to replace it."
  (when (and (romacs-pass-entry-p entry)
             (not (yes-or-no-p (format "%s exists; replace it? " entry))))
    (user-error "Kept %s" entry)))

(defun romacs-pass-alphabet ()
  "The characters a generated password is drawn from.
Letters and digits, and ASCII punctuation unless
`romacs-pass-generate-symbols' is nil: pass(1)'s [:graph:] and
[:alnum:]."
  (concat "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
          (and romacs-pass-generate-symbols "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~")))

;;;###autoload
(defun romacs-pass-generate (entry &optional length)
  "Generate a password for ENTRY of LENGTH characters, store and copy it.
LENGTH defaults to `romacs-pass-generate-length'; a prefix argument
gives it.  An existing entry keeps its other lines: only its password
is replaced, after confirmation."
  (interactive (list (romacs-pass-read-entry "Generate entry: " t)
                     (and current-prefix-arg (prefix-numeric-value current-prefix-arg))))
  (let ((exists (romacs-pass-entry-p entry))
        (password (romacs-pass--generate (or length romacs-pass-generate-length) (romacs-pass-alphabet))))
    (if (not exists)
        (romacs-pass--store-generated entry password password)
      (unless (yes-or-no-p (format "Replace %s's password? " entry))
        (clear-string password)
        (user-error "Kept %s" entry))
      (romacs-pass-with-entry
       entry (lambda (text)
               (romacs-pass--store-generated
                entry password (concat password (substring text (or (string-search "\n" text) (length text))))))))
    nil))

(defun romacs-pass--store-generated (entry password text)
  "Store TEXT as ENTRY, then copy its PASSWORD and wipe it."
  (romacs-pass--write entry text
                      (lambda (why)
                        (unless (eq text password) (clear-string text))
                        (unwind-protect
                            (if why (message "%s" why)
                              (romacs-pass--copy password (format "%s's new password" entry)))
                          (clear-string password)))))

;;;; Editing

(defvar-local romacs-pass--edited nil "The entry the buffer edits.")

(defvar-keymap romacs-pass-edit-mode-map
  "C-c C-c" #'romacs-pass-edit-save
  "C-c C-k" #'romacs-pass-edit-cancel)

(define-derived-mode romacs-pass-edit-mode nil "Pass-Edit"
  "Editing a password-store entry: its first line is the password.
\\<romacs-pass-edit-mode-map>\\[romacs-pass-edit-save] stores it and \\[romacs-pass-edit-cancel] leaves it unchanged.
The buffer visits no file, makes no auto-save and is never carried
by a session image."
  (setq-local buffer-auto-save-file-name nil
              backup-inhibited t)
  (auto-save-mode -1)
  (add-hook 'kill-buffer-hook #'romacs-pass--wipe-buffer nil t)
  (add-hook 'warm-restart-blocker-functions #'romacs-pass--restart-blocker nil t)
  (setq header-line-format
        (substitute-command-keys
         "\\<romacs-pass-edit-mode-map>\\[romacs-pass-edit-save] store  \\[romacs-pass-edit-cancel] cancel")))

;;;###autoload
(defun romacs-pass-edit (entry)
  "Edit ENTRY in a buffer, created if the store lacks it."
  (interactive (list (romacs-pass-read-entry "Edit entry: " t)))
  (if (romacs-pass-entry-p entry)
      (romacs-pass-with-entry entry (lambda (text) (romacs-pass--edit-buffer entry text)))
    (romacs-pass--edit-buffer entry "")))

(defun romacs-pass--edit-buffer (entry text)
  "Show a buffer editing ENTRY, holding a copy of TEXT."
  (let ((buffer (get-buffer-create (format "*pass: %s*" entry))))
    (with-current-buffer buffer
      (romacs-pass-edit-mode)
      (let ((inhibit-read-only t)) (erase-buffer) (insert text))
      (setq romacs-pass--edited entry
            buffer-undo-list nil)
      (set-buffer-modified-p nil)
      (goto-char (point-min)))
    (pop-to-buffer buffer)))

(defun romacs-pass-edit-save ()
  "Store the buffer's text as its entry, and kill the buffer."
  (interactive nil romacs-pass-edit-mode)
  (let ((buffer (current-buffer))
        (entry romacs-pass--edited))
    (romacs-pass--write-reported entry (buffer-substring-no-properties (point-min) (point-max))
                                 (lambda ()
                                   (when (buffer-live-p buffer) (kill-buffer buffer))
                                   (message "Stored %s" entry)))))

(defun romacs-pass-edit-cancel ()
  "Kill the buffer, leaving its entry unchanged."
  (interactive nil romacs-pass-edit-mode)
  (kill-buffer))

(defun romacs-pass--wipe-buffer ()
  "Empty the buffer and its undo list before it is killed."
  (let ((inhibit-read-only t)) (erase-buffer))
  (setq buffer-undo-list t))

(defun romacs-pass--restart-blocker ()
  "Refuse to hand an edited entry over to a new process."
  (and (buffer-modified-p) "an unsaved password-store entry; store or cancel it first"))

;;;; The launcher

(defun romacs-pass--action (entry)
  "Ask what to do with ENTRY, and do it."
  (pcase (car (read-multiple-choice
               entry '((?c "copy" "Copy the password")
                       (?o "otp" "Copy the one-time code")
                       (?a "type" "Type user name and password")
                       (?e "edit" "Edit the entry"))))
    (?c (romacs-pass-copy entry))
    (?o (romacs-pass-otp entry))
    (?a (romacs-pass-autotype entry))
    (?e (romacs-pass-edit entry))))

;;;###autoload
(defun romacs-pass-launcher ()
  "Choose an entry, then what to do with it; a new name generates one."
  (interactive)
  (let ((entry (romacs-pass-read-entry "Password: " t)))
    (if (romacs-pass-entry-p entry)
        (romacs-pass--action entry)
      (romacs-pass-generate entry))))

;;;###autoload (autoload 'romacs-pass-map "romacs-pass" nil t 'keymap)
(defvar-keymap romacs-pass-map
  :doc "The password manager's commands, for a prefix key."
  "c" #'romacs-pass-copy
  "o" #'romacs-pass-otp
  "a" #'romacs-pass-autotype
  "e" #'romacs-pass-edit
  "g" #'romacs-pass-generate
  "i" #'romacs-pass-insert
  "l" #'romacs-pass-launcher
  "p" #'romacs-pass)
(fset 'romacs-pass-map romacs-pass-map)

(provide 'romacs-pass)
;;; romacs-pass.el ends here
