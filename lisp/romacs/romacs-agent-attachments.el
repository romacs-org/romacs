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

(defun romacs-agent-attachments--sha (file)
  "FILE's sha256, from `sha256sum' (its bytes never pass through Lisp), or nil."
  (with-temp-buffer
    (and (eql 0 (ignore-errors (call-process "sha256sum" nil t nil "--" file)))
         (progn (goto-char (point-min)) (looking-at "[0-9a-f]\\{64\\}"))
         (match-string 0))))

(defun romacs-agent-attachments--item (path alt)
  "PATH, an absolute local regular file, as an attachment's record with
caption ALT; a `user-error' for anything else."
  (unless (and (stringp path) (file-name-absolute-p path))
    (user-error "Attach a file by its absolute path: %S" path))
  (let ((item (romacs-attach-item path)))
    (romacs-mcp--object "path" (plist-get item :path)
                        "kind" (if (string-prefix-p "image/" (plist-get item :mime)) "image" "file")
                        "alt" (or alt :null) "size" (plist-get item :bytes)
                        "sha" (or (romacs-agent-attachments--sha (plist-get item :path)) :null))))

(defun romacs-agent-attachments--alts (alt count)
  "ALT, a string for one file or an array of one per file, as COUNT captions."
  (cond ((null alt) (make-list count nil))
        ((stringp alt)
         (unless (= count 1) (user-error "alt for several files is an array, one per file"))
         (list alt))
        ((and (vectorp alt) (= (length alt) count) (cl-every (lambda (a) (or (stringp a) (eq a :null))) alt))
         (mapcar (lambda (a) (and (stringp a) a)) (append alt nil)))
        (t (user-error "alt is a string, or an array of one per file"))))

(defun romacs-agent-attachments--tool (arguments)
  "Attach ARGUMENTS' paths to the calling session's reply."
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
    (let ((items (cl-mapcar #'romacs-agent-attachments--item paths
                            (romacs-agent-attachments--alts (gethash "alt" arguments) (length paths)))))
      (romacs-agent--note caller (romacs-mcp--object "type" "attachments" "items" (vconcat items)
                                                     "compare" (if compare (vconcat compare) :null)))
      (romacs-mcp--object "attached" (vconcat items)
                          "next" "They are shown with your reply; nothing more to do."))))

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
