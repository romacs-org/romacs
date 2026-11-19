;;; romacs-ram-files-tests.el --- Memory-only files stay off the disk -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'warm-restart)
(require 'romacs-ram-files)

(defun romacs-ram-files-tests--strategy (buffer)
  "BUFFER's strategy in the plan a warm restart would make now."
  (plist-get (seq-find (lambda (entry) (eq (plist-get entry :buffer) buffer))
                       (plist-get (warm-restart-plan) :buffers))
             :strategy))

;; Fixtures are made under a directory whose true name is outside
;; /dev/shm, which is always a memory-only directory: a fixture inside it
;; would be kept off the disk whatever $XDG_RUNTIME_DIR says.
(defun romacs-ram-files-tests--outside-shm-p (directory)
  "Whether DIRECTORY's true name is outside /dev/shm."
  (not (string-prefix-p (file-name-as-directory (file-truename "/dev/shm"))
                        (file-name-as-directory (file-truename directory)))))

(defmacro romacs-ram-files-tests--isolated (&rest body)
  "Run BODY with fixtures made outside /dev/shm and its own copy of
`process-environment', so `setenv' changes nothing outside it."
  `(let ((temporary-file-directory
          (seq-find #'romacs-ram-files-tests--outside-shm-p
                    (list temporary-file-directory "/var/tmp/" (expand-file-name "~/"))))
         (process-environment (copy-sequence process-environment)))
     (should temporary-file-directory)
     ,@body))

(ert-deftest romacs-ram-files-tests-a-runtime-directory-file-makes-no-backup-auto-save-or-image ()
  (romacs-ram-files-tests--isolated
   (let* ((runtime (make-temp-file "romacs-ram-files-" t))
          (elsewhere (make-temp-file "romacs-ram-files-disk-" t))
          (process-environment (cons (concat "XDG_RUNTIME_DIR=" runtime) process-environment))
          (auto-save-default t)
          ;; As `romacs-defaults-mode' hooks it.
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (unwind-protect
         (let ((secret (find-file-noselect (expand-file-name "entry.txt" runtime)))
               (plain (find-file-noselect (expand-file-name "notes.txt" elsewhere))))
           (with-current-buffer secret
             (should backup-inhibited)
             (should-not buffer-auto-save-file-name)
             ;; Left out whatever its state, and nothing holds the restart.
             (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
             (insert "password")
             (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
             (save-buffer)
             (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
             ;; A major mode clears the buffer's locals; the protection holds.
             (text-mode)
             (insert "more")
             (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
             (set-buffer-modified-p nil))
           (with-current-buffer plain
             (should (eq (romacs-ram-files-tests--strategy plain) 'file)))
           (kill-buffer secret)
           (kill-buffer plain))
       (delete-directory runtime t)
       (delete-directory elsewhere t)))))

(ert-deftest romacs-ram-files-tests-a-restore-names-unsaved-text-it-did-not-carry ()
  (let ((shown nil))
    (cl-letf (((symbol-function 'message) (lambda (format &rest args) (setq shown (apply #'format format args)))))
      (warm-restart--announce
       (list :buffers 3 :records 3 :seconds 0.1
             :lost (list (list :buffer "entry.txt" :reason "a memory-only file, kept off the disk" :unsaved t)
                         (list :buffer "*Help*" :reason "generated buffer")))))
    (should (string-match-p "unsaved text not carried: entry\\.txt" shown))
    (should-not (string-match-p "\\*Help\\*" shown))
    ;; An incomplete restore warns, and names it too.
    (cl-letf (((symbol-function 'display-warning) (lambda (_type text &rest _) (setq shown text))))
      (warm-restart--announce
       (list :buffers 2 :records 3 :seconds 0.1 :file "/tmp/x.image"
             :lost (list (list :buffer "entry.txt" :reason "a memory-only file, kept off the disk" :unsaved t)))))
    (should (string-match-p "incomplete.*unsaved text not carried: entry\\.txt" shown))))

(ert-deftest romacs-ram-files-tests-an-image-leaves-out-unsaved-ram-text-and-says-so ()
  (romacs-ram-files-tests--isolated
   (let* ((runtime (make-temp-file "romacs-ram-files-" t))
          (process-environment (cons (concat "XDG_RUNTIME_DIR=" runtime) process-environment))
          (image (expand-file-name "session.image" (make-temp-file "romacs-ram-files-image-" t)))
          (secret (find-file-noselect (expand-file-name "entry.txt" runtime))))
     (unwind-protect
         (progn
           (with-current-buffer secret
             (romacs-ram-files-keep-off-disk)
             (insert "secret-text"))
           (warm-restart-write nil image)
           (should (equal (seq-filter (lambda (entry) (equal (plist-get entry :buffer) "entry.txt"))
                                      (plist-get (car (warm-restart--read image)) :lost))
                          '((:buffer "entry.txt" :reason "a memory-only file, kept off the disk" :unsaved t))))
           (with-temp-buffer
             (insert-file-contents-literally image)
             (should-not (search-forward "secret-text" nil t))))
       (with-current-buffer secret (set-buffer-modified-p nil))
       (kill-buffer secret)
       (delete-directory runtime t)
       (delete-directory (file-name-directory image) t)))))

;; The memory-only directories are named by $XDG_RUNTIME_DIR and /dev/shm,
;; and a file under either, through links or not, stays off the disk.  Each
;; case asserts what only the hook sets (`warm-restart-left-out'): in batch,
;; backups and auto-saves are off whether or not it acts.
(ert-deftest romacs-ram-files-tests-a-symlinked-runtime-directory-holds-its-files ()
  (romacs-ram-files-tests--isolated
   (let* ((real (make-temp-file "romacs-ram-files-real-" t))
          (link (concat (make-temp-name (expand-file-name "romacs-ram-files-link-" temporary-file-directory))))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (make-symbolic-link real link)
     (setenv "XDG_RUNTIME_DIR" link)
     (unwind-protect
         (dolist (name (list (expand-file-name "entry.txt" link) (expand-file-name "other.txt" real)))
           (let ((buffer (find-file-noselect name)))
             (with-current-buffer buffer
               ;; What only the hook sets: batch leaves backups and
               ;; auto-saves off whatever it does.
               (should (local-variable-p 'warm-restart-left-out))
               (should backup-inhibited)
               (should-not buffer-auto-save-file-name)
               (should (romacs-ram-files-p name)))
             (kill-buffer buffer)))
       (delete-file link)
       (delete-directory real t)))))

(ert-deftest romacs-ram-files-tests-a-runtime-directory-changed-at-run-time-is-seen ()
  (romacs-ram-files-tests--isolated
   (let* ((first (make-temp-file "romacs-ram-files-a-" t))
          (second (make-temp-file "romacs-ram-files-b-" t))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk))
          (kept (lambda (directory name)
                  (let ((buffer (find-file-noselect (expand-file-name name directory))))
                    (prog1 (buffer-local-value 'warm-restart-left-out buffer)
                      (kill-buffer buffer))))))
     (unwind-protect
         (progn
           (setenv "XDG_RUNTIME_DIR" first)
           (should (romacs-ram-files-p (expand-file-name "x" first)))
           (should-not (romacs-ram-files-p (expand-file-name "x" second)))
           (should (funcall kept first "a.txt"))
           (should-not (funcall kept second "b.txt"))
           (setenv "XDG_RUNTIME_DIR" second)
           (should (romacs-ram-files-p (expand-file-name "x" second)))
           (should-not (romacs-ram-files-p (expand-file-name "x" first)))
           (should (funcall kept second "c.txt"))
           (should-not (funcall kept first "d.txt")))
       (delete-directory first t)
       (delete-directory second t)))))

(ert-deftest romacs-ram-files-tests-a-runtime-directory-created-later-holds-its-files ()
  "XDG_RUNTIME_DIR names a directory that does not exist yet; an
unrelated visit comes first; once the directory exists, a file in it is
kept off the disk."
  (romacs-ram-files-tests--isolated
   (let* ((parent (make-temp-file "romacs-ram-files-later-" t))
          (runtime (expand-file-name "private" parent))
          (elsewhere (make-temp-file "romacs-ram-files-disk-" t))
          (auto-save-default t)
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (setenv "XDG_RUNTIME_DIR" runtime)
     (unwind-protect
         (progn
           (kill-buffer (find-file-noselect (expand-file-name "notes.txt" elsewhere)))
           (make-directory runtime)
           (let ((secret (find-file-noselect (expand-file-name "secret.txt" runtime))))
             (with-current-buffer secret
               (should (local-variable-p 'warm-restart-left-out))
               (should (romacs-ram-files-p buffer-file-name)))
             (kill-buffer secret)))
       (delete-directory parent t)
       (delete-directory elsewhere t)))))

(ert-deftest romacs-ram-files-tests-a-revert-through-a-retargeted-link-is-kept-off-disk ()
  "A file visited through a link to an ordinary directory, the link then
pointed into the runtime directory, and the buffer reverted: the revert
takes the true name again, and the buffer is kept off the disk."
  (romacs-ram-files-tests--isolated
   (let* ((runtime (make-temp-file "romacs-ram-files-run-" t))
          (elsewhere (make-temp-file "romacs-ram-files-disk-" t))
          (link (make-temp-name (expand-file-name "romacs-ram-files-via-" temporary-file-directory)))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (setenv "XDG_RUNTIME_DIR" runtime)
     (dolist (dir (list runtime elsewhere))
       (with-temp-file (expand-file-name "secret.txt" dir) (insert dir)))
     (make-symbolic-link elsewhere link)
     (unwind-protect
         (let ((buffer (find-file-noselect (expand-file-name "secret.txt" link))))
           (with-current-buffer buffer
             (should-not (local-variable-p 'warm-restart-left-out))
             (delete-file link)
             (make-symbolic-link runtime link)
             (revert-buffer t t)
             (should (equal (buffer-string) runtime))
             (should (local-variable-p 'warm-restart-left-out))
             (should-not buffer-auto-save-file-name))
           (kill-buffer buffer))
       (delete-file link)
       (delete-directory runtime t)
       (delete-directory elsewhere t)))))

;; A runtime directory named through a link that is pointed elsewhere
;; mid-session, as a session manager rotates /run/user/UID/session-*.
(ert-deftest romacs-ram-files-tests-a-runtime-link-rotated-to-a-new-directory-is-followed ()
  (romacs-ram-files-tests--isolated
   (let* ((parent (make-temp-file "romacs-ram-files-rot-" t))
          (first (expand-file-name "session-a" parent))
          (second (expand-file-name "session-b" parent))
          (link (expand-file-name "runtime" parent))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (make-directory first) (make-directory second)
     (make-symbolic-link first link)
     (setenv "XDG_RUNTIME_DIR" link)
     (unwind-protect
         (progn
           (should (romacs-ram-files-p (expand-file-name "x" first)))
           (delete-file link)
           (make-symbolic-link second link)
           (let ((secret (find-file-noselect (expand-file-name "secret.txt" second))))
             (with-current-buffer secret
               (should (local-variable-p 'warm-restart-left-out)))
             (kill-buffer secret))
           (should-not (romacs-ram-files-p (expand-file-name "x" first))))
       (delete-directory parent t)))))

;; A visit whose true name `directory-abbrev-alist' abbreviated through a
;; link, then the alist cleared and the mode set again.
(ert-deftest romacs-ram-files-tests-a-mode-change-takes-the-true-name-afresh ()
  (romacs-ram-files-tests--isolated
   (let* ((runtime (make-temp-file "romacs-ram-files-abbrev-" t))
          (home (make-temp-file "romacs-ram-files-home-" t))
          (alias (expand-file-name "secrets" home))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (setenv "XDG_RUNTIME_DIR" runtime)
     (make-symbolic-link runtime alias)
     (unwind-protect
         (let* ((directory-abbrev-alist
                 (list (cons (concat "\\`" (regexp-quote (file-name-as-directory (file-truename runtime))))
                             (file-name-as-directory alias))))
                (secret (find-file-noselect (expand-file-name "secret.txt" runtime))))
           (with-current-buffer secret
             (should (string-prefix-p alias (expand-file-name buffer-file-truename)))
             (should (local-variable-p 'warm-restart-left-out))
             (setq directory-abbrev-alist nil)
             (normal-mode)
             (should (local-variable-p 'warm-restart-left-out)))
           (kill-buffer secret))
       (delete-directory home t)
       (delete-directory runtime t)))))

(ert-deftest romacs-ram-files-tests-a-runtime-directory-renamed-and-relinked-is-followed ()
  "The runtime link's directory renamed (same inode) and the link pointed
at its new name: a file under the new name is kept off the disk."
  (romacs-ram-files-tests--isolated
   (let* ((parent (make-temp-file "romacs-ram-files-mv-" t))
          (first (expand-file-name "session-a" parent))
          (second (expand-file-name "session-b" parent))
          (link (expand-file-name "runtime" parent))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (make-directory first)
     (make-symbolic-link first link)
     (setenv "XDG_RUNTIME_DIR" link)
     (unwind-protect
         (progn
           (should (romacs-ram-files-p (expand-file-name "x" first)))
           (rename-file first second)
           (delete-file link)
           (make-symbolic-link second link)
           (let ((secret (find-file-noselect (expand-file-name "secret.txt" second))))
             (with-current-buffer secret
               (should (local-variable-p 'warm-restart-left-out)))
             (kill-buffer secret)))
       (delete-directory parent t)))))

;; The runtime directory renamed in place with a compatibility link left
;; at its old name, the runtime link unchanged.
(ert-deftest romacs-ram-files-tests-a-renamed-runtime-directory-with-a-compatibility-link-holds-its-files ()
  (romacs-ram-files-tests--isolated
   (let* ((parent (make-temp-file "romacs-ram-files-compat-" t))
          (first (expand-file-name "session-a" parent))
          (second (expand-file-name "session-b" parent))
          (link (expand-file-name "runtime" parent))
          (find-file-hook (list #'romacs-ram-files-keep-off-disk))
          (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
     (make-directory first)
     (make-symbolic-link first link)
     (setenv "XDG_RUNTIME_DIR" link)
     (unwind-protect
         (progn
           (should (romacs-ram-files-p (expand-file-name "x" first)))
           (rename-file first second)
           (make-symbolic-link second first)
           (let ((secret (find-file-noselect (expand-file-name "secret.txt" second))))
             (with-current-buffer secret
               (should (local-variable-p 'warm-restart-left-out)))
             (kill-buffer secret)))
       (delete-directory parent t)))))

;;; romacs-ram-files-tests.el ends here
