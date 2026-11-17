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
;; as its storage is reused.  The desktop's clipboard is not yet told a
;; copy is a password (no `x-kde-passwordManagerHint'), so a clipboard
;; history keeps it.  auth-source keeps what it found for
;; `auth-source-cache-expiry', and the strings it hands its callers are
;; theirs.

;;; Code:

(require 'subr-x)
(require 'epg-config)

(defvar auth-source-pass-filename)
(defvar warm-restart-blocker-functions)
(defvar yank-menu)
(declare-function romacs-pass "romacs-pass-menu" ())
(declare-function romacs-usage-log-note "romacs-usage-log" (kind &rest plist))

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

(defcustom romacs-pass-autotype-delay 3
  "Seconds `romacs-pass-autotype' waits for the target window to be focused.
Typing is refused if a Romacs frame still has the focus then."
  :type 'number)

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

(defvar romacs-pass-slow-decrypt-seconds 1.0
  "Seconds from gpg's start to its answer that put a decryption on record.
The usage log then gets {\"kind\":\"pass-decrypt\",\"ms\":MS,
\"output_ms\":O,\"status\":S,\"t\":T}: MS from the start to the
sentinel that answers, O to gpg's first output (absent when it wrote
none), S \"ok\" or \"failed\", T when it answered, as `float-time'
reads, to join with the log's stall records.  O is gpg's own time,
gpg-agent's and pinentry's included; MS - O is the time its answer
waited for the main VM.  The entry is not named.")

(defun romacs-pass--note-decrypt (start output answered ok)
  "Put a decryption on the usage log's record if it was slow.
START, OUTPUT (gpg's first output, or nil) and ANSWERED are `float-time'
readings; OK whether gpg succeeded.  See `romacs-pass-slow-decrypt-seconds'."
  (when (and (>= (- answered start) romacs-pass-slow-decrypt-seconds)
             (fboundp 'romacs-usage-log-note))
    (apply #'romacs-usage-log-note "pass-decrypt"
           :ms (round (* 1000 (- answered start)))
           (append (and output (list :output_ms (round (* 1000 (- output start)))))
                   (list :status (if ok "ok" "failed") :t answered)))))

(defun romacs-pass--gpg (entry done)
  "Start gpg decrypting ENTRY and return its process.
DONE is called with the text, decoded, and nil when gpg succeeds, or
with nil and gpg's own message when it fails; the text is DONE's own.
gpg asks gpg-agent for the key, which asks for its passphrase through
the configured pinentry.  This is the one place that decrypts an
entry; a slow one goes on the usage log's record
\(`romacs-pass-slow-decrypt-seconds')."
  (let ((file (romacs-pass--file entry))
        (start (float-time))
        (output nil)
        (chunks nil)
        (errors (generate-new-buffer " *romacs-pass gpg*")))
    (unless (file-readable-p file)
      (kill-buffer errors)
      (user-error "No password-store entry %s" entry))
    (make-process
     :name "romacs-pass-gpg"
     :command (list epg-gpg-program "--batch" "--quiet" "--decrypt" "--" file)
     :connection-type 'pipe :coding 'binary :noquery t :buffer nil :stderr errors
     :filter (lambda (_process chunk)
               (unless output (setq output (float-time)))
               (push chunk chunks))
     :sentinel
     (lambda (process _event)
       (unless (process-live-p process)
         (let ((raw (apply #'concat (setq chunks (nreverse chunks))))
               (why (and (buffer-live-p errors)
                         (string-trim (with-current-buffer errors (buffer-string))))))
           (mapc #'clear-string chunks)
           (setq chunks nil)
           (romacs-pass--kill-buffer errors)
           (romacs-pass--note-decrypt start output (float-time) (romacs-pass--succeeded-p process))
           (if (romacs-pass--succeeded-p process)
               (let ((text (decode-coding-string raw 'utf-8)))
                 (clear-string raw)
                 (funcall done text nil))
             (clear-string raw)
             (funcall done nil (format "gpg could not decrypt %s%s" entry
                                       (if (string-empty-p (or why "")) "" (concat ": " why)))))))))))

(defun romacs-pass--succeeded-p (process)
  "Whether PROCESS, ended, exited with status 0."
  (and (eq (process-status process) 'exit) (zerop (process-exit-status process))))

(defun romacs-pass--kill-buffer (buffer)
  "Kill BUFFER and the stderr pipe still reading into it, asking nothing."
  (when (buffer-live-p buffer)
    (when-let* ((process (get-buffer-process buffer)))
      (delete-process process))
    (kill-buffer buffer)))

(defun romacs-pass-with-entry (entry function)
  "Call FUNCTION with ENTRY's decrypted text once gpg has it, then wipe it.
Return at once: gpg runs, and its passphrase prompt waits, without
blocking; the echo area says so meanwhile.  A failure is reported there."
  (message "Decrypting %s..." entry)
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
    ;; A timer or a process filter (jabber reconnecting) runs with
    ;; `inhibit-quit'; C-g still ends this wait.
    (unwind-protect
        (with-local-quit
          (while (not answer)
            (accept-process-output (and (process-live-p process) process) 0.05)))
      (unless answer (delete-process process)))
    (or (car answer) (error "%s" (or (cdr answer) (format "Stopped decrypting %s" entry))))))

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
  (let ((output (generate-new-buffer " *romacs-pass insert*"))
        (process-environment (cons (concat "PASSWORD_STORE_DIR="
                                           (directory-file-name (romacs-pass-directory)))
                                   process-environment))
        (payload (if (string-suffix-p "\n" text) text (concat text "\n"))))
    (unwind-protect
        (let ((process
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
          (process-send-string process payload)
          (process-send-eof process)
          process)
      (unless (eq payload text) (clear-string payload)))))

(defun romacs-pass--wiping (strings function)
  "Call FUNCTION; if it signals, wipe STRINGS first and signal again."
  (condition-case err
      (funcall function)
    (error (mapc #'clear-string (seq-filter #'stringp strings))
           (signal (car err) (cdr err)))))

(defun romacs-pass--write-reported (entry text then)
  "Store TEXT as ENTRY, wipe TEXT, then call THEN with no arguments.
A failure is reported in the echo area instead."
  (romacs-pass--wiping
   (list text)
   (lambda ()
     (romacs-pass--write entry text
                         (lambda (why)
                           (clear-string text)
                           (if why (message "%s" why) (funcall then)))))))

;;;; Copies

(defvar romacs-pass--copies nil
  "The copies in the kill ring and clipboard not yet forgotten, newest first.")

(defun romacs-pass--copy (secret what)
  "Put a copy of SECRET in the kill ring and clipboard for a while.
WHAT names it in the message.  The copy leaves both, and is wiped,
after `romacs-pass-clear-seconds', or when Romacs exits before then."
  (let ((copy (romacs-pass--secret (copy-sequence secret)))
        (kill-do-not-save-duplicates nil))
    (kill-new copy)
    (push copy romacs-pass--copies)
    (add-hook 'kill-emacs-hook #'romacs-pass--forget-all)
    (run-at-time romacs-pass-clear-seconds nil #'romacs-pass--forget copy)
    (message "Copied %s; cleared in %s s" what romacs-pass-clear-seconds)))

(defun romacs-pass--forget-all ()
  "Forget every copy not yet forgotten: Romacs exits, and its timers with it."
  (mapc #'romacs-pass--forget romacs-pass--copies))

(defun romacs-pass--forget (copy)
  "Take COPY out of the clipboard while it holds it, the kill ring and the
Edit menu's yank list.  Then wipe it, and the menu's title for it."
  (setq romacs-pass--copies (delq copy romacs-pass--copies))
  (when (and interprogram-cut-function
             (equal (ignore-errors (gui-get-selection 'CLIPBOARD 'UTF8_STRING)) copy))
    (funcall interprogram-cut-function ""))
  (when (eq (car kill-ring-yank-pointer) copy)
    (setq kill-ring-yank-pointer (cdr kill-ring-yank-pointer)))
  (setq kill-ring (delq copy kill-ring))
  (unless (memq (car kill-ring-yank-pointer) kill-ring)
    (setq kill-ring-yank-pointer kill-ring))
  (when (consp (bound-and-true-p yank-menu))
    (let ((items (seq-filter (lambda (item) (eq (car-safe item) copy)) (cdr yank-menu))))
      (setcdr yank-menu (seq-difference (cdr yank-menu) items #'eq))
      ;; A title is the copy itself when it is short, else a piece of it.
      (dolist (item items)
        (when (stringp (car-safe (cdr item))) (clear-string (cadr item))))))
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
                     (romacs-pass--type-later (copy-sequence user) (copy-sequence secret))))
               (romacs-pass--wipe-data data)))))
  nil)

(defun romacs-pass--focused-p ()
  "Whether a Romacs frame has the focus, or may have it: one no focus
change reached yet counts as focused."
  (seq-some (lambda (frame)
              (let ((update (assq 'last-focus-update (frame-parameters frame))))
                (or (null update) (cdr update))))
            (frame-list)))

(defun romacs-pass--type-later (user secret)
  "Type USER, Tab and SECRET after `romacs-pass-autotype-delay', unless a
Romacs frame has the focus then: what is typed goes to the focused
window, and must not land in a buffer.  Both strings are wiped."
  (message "Typing into the window focused in %s s" romacs-pass-autotype-delay)
  (run-at-time romacs-pass-autotype-delay nil
               (lambda ()
                 (unwind-protect
                     (if (romacs-pass--focused-p)
                         (message "Romacs has the focus: nothing typed")
                       (romacs-pass--type user secret))
                   (clear-string user)
                   (clear-string secret)))))

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
  (let ((length (or length romacs-pass-generate-length))
        (generate (lambda (length) (romacs-pass--generate length (romacs-pass-alphabet)))))
    (when (< length 1) (user-error "A password needs at least one character"))
    (if (not (romacs-pass-entry-p entry))
        (let ((password (funcall generate length)))
          (romacs-pass--store-generated entry password password))
      (unless (yes-or-no-p (format "Replace %s's password? " entry))
        (user-error "Kept %s" entry))
      (romacs-pass-with-entry
       entry (lambda (text)
               (let ((password (funcall generate length)))
                 (romacs-pass--store-generated
                  entry password
                  (concat password (substring text (or (string-search "\n" text) (length text)))))))))
    nil))

(defun romacs-pass--store-generated (entry password text)
  "Store TEXT as ENTRY, then copy its PASSWORD; both are wiped."
  (romacs-pass--wiping
   (list password text)
   (lambda ()
     (romacs-pass--write entry text
                         (lambda (why)
                           (unless (eq text password) (clear-string text))
                           (unwind-protect
                               (if why (message "%s" why)
                                 (romacs-pass--copy password (format "%s's new password" entry)))
                             (clear-string password)))))))

;;;; Editing

(defvar-local romacs-pass--edited nil "The entry the buffer edits.")

(defvar-local romacs-pass--decrypting nil
  "The gpg process decrypting the entry, while the buffer waits for it.")

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
  ;; Text taken out of the buffer (a kill, a copy, a register) is a secret.
  (add-function :filter-return (local 'filter-buffer-substring-function) #'romacs-pass--secret)
  (add-hook 'kill-buffer-hook #'romacs-pass--wipe-buffer nil t)
  (add-hook 'warm-restart-blocker-functions #'romacs-pass--restart-blocker nil t)
  (setq header-line-format (romacs-pass--edit-header)))

(defun romacs-pass--edit-header ()
  "The header line of a buffer editing an entry."
  (substitute-command-keys
   "\\<romacs-pass-edit-mode-map>\\[romacs-pass-edit-save] store  \\[romacs-pass-edit-cancel] cancel"))

(defvar-keymap romacs-pass-decrypting-mode-map
  :doc "Keys of a buffer waiting for its entry's text: C-g stops the wait."
  "<remap> <keyboard-quit>" #'romacs-pass-edit-cancel)

(define-minor-mode romacs-pass-decrypting-mode
  "A buffer that will edit an entry waits for gpg to decrypt it.
It is empty and read-only meanwhile; \\[keyboard-quit] kills it and stops gpg."
  :lighter nil
  (setq buffer-read-only romacs-pass-decrypting-mode))

;;;###autoload
(defun romacs-pass-edit (entry)
  "Edit ENTRY in a buffer, created if the store lacks it.
The buffer shows at once; an existing entry's text fills it when gpg
answers, and until then \\[keyboard-quit] or
\\<romacs-pass-edit-mode-map>\\[romacs-pass-edit-cancel] stops gpg.
A buffer already editing ENTRY, with changes or still waiting, is
shown as it is."
  (interactive (list (romacs-pass-read-entry "Edit entry: " t)))
  (let ((buffer (get-buffer (romacs-pass--edit-name entry))))
    (pop-to-buffer
     (cond ((and buffer (or (buffer-modified-p buffer)
                            (buffer-local-value 'romacs-pass--decrypting buffer)))
            buffer)
           ((romacs-pass-entry-p entry) (romacs-pass--edit-decrypting entry))
           (t (romacs-pass--edit-fill (romacs-pass--edit-buffer entry) ""))))))

(defun romacs-pass--edit-name (entry)
  "The name of the buffer editing ENTRY."
  (format "*pass: %s*" entry))

(defun romacs-pass--edit-buffer (entry)
  "An empty buffer editing ENTRY, unmodified, with no undo history."
  (let ((buffer (get-buffer-create (romacs-pass--edit-name entry))))
    (with-current-buffer buffer
      (romacs-pass-edit-mode)
      (romacs-pass--edit-replace "")
      (setq romacs-pass--edited entry))
    buffer))

(defun romacs-pass--edit-replace (text)
  "Make the buffer's text a copy of TEXT, unmodified, with no undo history:
what it held is not kept in the undo list, where it would outlive it."
  (let ((inhibit-read-only t) (buffer-undo-list t)) (erase-buffer) (insert text))
  (setq buffer-undo-list nil)
  (set-buffer-modified-p nil))

(defun romacs-pass--edit-fill (buffer text)
  "Put a copy of TEXT in BUFFER, editing its entry from the start; BUFFER.
Which window shows it is left as it is."
  (with-current-buffer buffer
    (romacs-pass--edit-replace text)
    (goto-char (point-min)))
  buffer)

(defun romacs-pass--edit-decrypting (entry)
  "A buffer editing ENTRY that waits for gpg, and fills when it answers."
  (let ((buffer (romacs-pass--edit-buffer entry)))
    (with-current-buffer buffer
      (romacs-pass-decrypting-mode 1)
      (setq header-line-format
            (concat "Decrypting " entry "...  "
                    (substitute-command-keys
                     ;; C-g is named from the global map: here it is remapped.
                     "\\<global-map>\\[keyboard-quit] or \\<romacs-pass-edit-mode-map>\\[romacs-pass-edit-cancel] stops")))
      (condition-case err
          (setq romacs-pass--decrypting
                (romacs-pass--gpg entry (lambda (text why) (romacs-pass--edit-decrypted buffer text why))))
        (error (kill-buffer buffer)
               (signal (car err) (cdr err)))))
    buffer))

(defun romacs-pass--edit-decrypted (buffer text why)
  "Fill BUFFER with TEXT, gpg's answer, or kill it and say WHY gpg failed.
A buffer killed meanwhile, its gpg stopped, takes nothing and says
nothing.  TEXT is wiped."
  (unwind-protect
      (when (and (buffer-live-p buffer) (buffer-local-value 'romacs-pass--decrypting buffer))
        (if (null text)
            (progn (kill-buffer buffer) (message "%s" why))
          (with-current-buffer buffer
            (setq romacs-pass--decrypting nil)
            (romacs-pass-decrypting-mode -1)
            (setq header-line-format (romacs-pass--edit-header)))
          (romacs-pass--edit-fill buffer text)))
    (when text (clear-string text))))

(defun romacs-pass-edit-save ()
  "Store the buffer's text as its entry, and kill the buffer.
The buffer is read-only while pass stores it, so what is stored is
what it shows."
  (interactive nil romacs-pass-edit-mode)
  (when romacs-pass--decrypting
    (user-error "%s is still being decrypted" romacs-pass--edited))
  (let ((buffer (current-buffer))
        (entry romacs-pass--edited)
        (text (buffer-substring-no-properties (point-min) (point-max))))
    (romacs-pass--wiping
     (list text)
     (lambda ()
       (romacs-pass--write
        entry text
        (lambda (why)
          (clear-string text)
          (cond ((not why) (when (buffer-live-p buffer) (kill-buffer buffer))
                 (message "Stored %s" entry))
                (t (when (buffer-live-p buffer)
                     (with-current-buffer buffer (setq buffer-read-only nil)))
                   (message "%s" why)))))))
    ;; pass has started: its answer comes after this command.
    (setq buffer-read-only t)))

(defun romacs-pass-edit-cancel ()
  "Kill the buffer, leaving its entry unchanged, and stop a gpg it waits for."
  (interactive nil romacs-pass-edit-mode)
  (kill-buffer))

(defun romacs-pass--wipe-buffer ()
  "Stop the gpg the buffer waits for, if any, then empty the buffer and its
undo list before it is killed."
  ;; The buffer stops waiting first: its gpg's sentinel may run as gpg is
  ;; deleted, and must then find nothing to fill and nothing to report.
  (let ((process romacs-pass--decrypting))
    (setq romacs-pass--decrypting nil)
    (when (process-live-p process) (delete-process process)))
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
