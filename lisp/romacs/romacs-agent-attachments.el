;;; romacs-agent-attachments.el --- Files an agent attaches to its reply  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An agent attaches files or images to the reply it is writing
;; (romacs_attach): they are referenced where they are, never copied.
;; Each is journaled in the session's journal as an `attachments' note,
;; with its kind, size and sha256, and folds into an `attachments' event
;; in the turn, which the chat draws with the reply.  Two of them can be
;; marked a before-and-after pair.

;;; Code:

(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-agent)
(require 'romacs-attach)

(declare-function romacs-agent-tools--caller "romacs-agent-tools")

(defconst romacs-agent-attachments--limit 16
  "The most files one call attaches.")

(defconst romacs-agent-attachments--hash-limit (* 1024 1024 1024)
  "The largest file hashed: a larger one is attached with no sha256.")

(defconst romacs-agent-attachments--hash-seconds 20
  "How long hashing an attachment may take before it is given up.")

(defun romacs-agent-attachments--hash (file size then)
  "Call THEN with FILE's sha256, or nil when it is larger than
`romacs-agent-attachments--hash-limit' (SIZE its size) or hashing fails
or takes too long.  Off the command loop: `sha256sum', its output
unescaped whatever the file's name."
  (if (> size romacs-agent-attachments--hash-limit) (funcall then nil)
    (let* ((output (generate-new-buffer " *romacs-attach-sha*" t))
           (done nil)
           (settle (lambda (sha)
                     (unless done
                       (setq done t)
                       (when (buffer-live-p output) (kill-buffer output))
                       (funcall then sha))))
           (process (make-process
                     :name "romacs-attach-sha" :buffer output :noquery t :connection-type 'pipe
                     :command (list "sha256sum" "--zero" "--" file)
                     :sentinel (lambda (process _event)
                                 (unless (process-live-p process)
                                   (funcall settle
                                            (and (eql (process-exit-status process) 0) (buffer-live-p output)
                                                 (with-current-buffer output
                                                   (goto-char (point-min))
                                                   (and (looking-at "[0-9a-f]\\{64\\}") (match-string 0))))))))))
      (run-at-time romacs-agent-attachments--hash-seconds nil
                   (lambda () (unless done (delete-process process) (funcall settle nil)))))))

(defun romacs-agent-attachments--check (path)
  "PATH, an attachment's, when it names an absolute local regular file not
under /proc, /sys or /dev; else a `user-error'.  Its metadata is its
target's when it is a symbolic link."
  (unless (and (stringp path) (file-name-absolute-p path))
    (user-error "Attach a file by its absolute path: %S" path))
  (let ((target (file-truename (expand-file-name path))))
    (when (string-match-p "\\`/\\(?:proc\\|sys\\|dev\\)/" target)
      (user-error "Not a file to attach: %s" path))
    (romacs-attach-item target)))

(defun romacs-agent-attachments--alts (alt count)
  "ALT, a string for one file or an array of one per file, as COUNT captions."
  (cond ((null alt) (make-list count nil))
        ((stringp alt)
         (unless (= count 1) (user-error "alt for several files is an array, one per file"))
         (list alt))
        ((and (vectorp alt) (= (length alt) count) (cl-every (lambda (a) (or (stringp a) (eq a :null))) alt))
         (mapcar (lambda (a) (and (stringp a) a)) (append alt nil)))
        (t (user-error "alt is a string, or an array of one per file"))))

(defun romacs-agent-attachments--await (session id then)
  "Call THEN once SESSION's attachments note ID is read back from its journal."
  (letrec ((waiter (lambda (seen event)
                     (when (and (eq seen session) (eq (plist-get event :type) 'attachments)
                                (equal (plist-get event :id) id))
                       (remove-hook 'romacs-agent-event-functions waiter)
                       (funcall then)))))
    (add-hook 'romacs-agent-event-functions waiter)))

(defun romacs-agent-attachments--tool (arguments)
  "Attach ARGUMENTS' paths to the calling session's reply: answered once
every file is hashed and the note is in its journal, its place in the
turn known."
  (romacs-mcp--instance arguments)
  (let* ((caller (or (when-let* ((id (romacs-agent-tools--caller))) (romacs-agent-get id))
                     (user-error "romacs_attach attaches to an agent session's reply: you are none")))
         (paths (append (gethash "paths" arguments) nil))
         (compare (append (gethash "compare" arguments) nil)))
    (unless (and paths (cl-every #'stringp paths)) (user-error "paths is a non-empty array of absolute paths"))
    (when (> (length paths) romacs-agent-attachments--limit)
      (user-error "At most %d files at once" romacs-agent-attachments--limit))
    (when compare
      (unless (and (= (length compare) 2) (cl-every (lambda (p) (member p paths)) compare))
        (user-error "compare is [before, after], two of paths")))
    (let* ((files (mapcar #'romacs-agent-attachments--check paths))
           (items (cl-mapcar (lambda (path file alt)
                               (romacs-mcp--object "path" path
                                                   "kind" (if (string-prefix-p "image/" (plist-get file :mime)) "image" "file")
                                                   "alt" (or alt :null) "size" (plist-get file :bytes) "sha" :null))
                             paths files (romacs-agent-attachments--alts (gethash "alt" arguments) (length paths))))
           (waiting (length items))
           (id (romacs-agent--uuid))
           (cell (list nil)))
      (cl-mapc (lambda (item file)
                 (romacs-agent-attachments--hash
                  (plist-get file :path) (plist-get file :bytes)
                  (lambda (sha)
                    (when sha (puthash "sha" sha item))
                    (when (zerop (cl-decf waiting))
                      (condition-case err
                          (progn (romacs-agent-attachments--await caller id (lambda () (setcar cell (list :done))))
                                 (romacs-agent--note caller (romacs-mcp--object "type" "attachments" "id" id
                                                                                "items" (vconcat items)
                                                                                "compare" (if compare (vconcat compare) :null))))
                        (error (setcar cell (list :error (error-message-string err)))))))))
               items files)
      (romacs-mcp-defer
       (lambda ()
         (pcase (car cell)
           ('nil nil)
           (`(:error ,why) (user-error "%s" why))
           (_ (list (romacs-mcp--object "attached" (vconcat items)
                                        "next" "They are shown with your reply; nothing more to do.")))))))))

(romacs-mcp-register-tool
 "romacs_attach"
 "Attach files or images to the reply you are writing, so the user sees them with it: paths (absolute, local \
files, referenced where they are, not copied), alt (a caption: a string for one file, an array of one per file), \
compare ([before, after], two of paths: shown as a before-and-after pair). Answers each file's kind (image or \
file), size and sha256."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "paths" (romacs-mcp--object "type" "array" "items" (romacs-mcp--object "type" "string"))
                "alt" (romacs-mcp--object "description" "A string for one file, or an array of one per file.")
                "compare" (romacs-mcp--object "type" "array" "items" (romacs-mcp--object "type" "string")))
  "required" ["paths"] "additionalProperties" :false)
 #'romacs-agent-attachments--tool)

(provide 'romacs-agent-attachments)
;;; romacs-agent-attachments.el ends here
