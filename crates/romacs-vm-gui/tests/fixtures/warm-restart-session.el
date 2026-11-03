;;; warm-restart-session.el --- a session for tests/warm_restart.rs  -*- lexical-binding: t -*-

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `wrt-make' builds a session in DIR (the test's private directory): file
;; buffers modified, narrowed, renamed and unibyte, an indirect view, a
;; text buffer with hard newlines and a local fill column, undo, a split
;; window layout, registers, the kill ring, histories, recentf, a process
;; and a regenerable view, and a handler-owned buffer that attaches only
;; at activation.  `wrt-observe' prints what a restore must reproduce.

;;; Code:

(require 'cl-lib)
(require 'warm-restart)

;; A handler-owned buffer: its package's data, never its text; the
;; handler defers its attachment while passive, as hermes's does.
(defvar wrt-attached nil "Buffers the test handler attached.")
(define-derived-mode wrt-view-mode special-mode "WRT-View")
(defun wrt-view-save (_dir) (list :topic (buffer-name)))
(defun wrt-view-restore (_file name misc)
  (with-current-buffer (get-buffer-create name)
    (wrt-view-mode)
    (let ((inhibit-read-only t)) (insert "topic " (plist-get misc :topic)))
    (let ((buffer (current-buffer)))
      (if warm-restart-passive
          (add-hook 'warm-restart-activate-functions (lambda () (push buffer wrt-attached)))
        (push buffer wrt-attached)))
    (current-buffer)))
(add-to-list 'desktop-buffer-mode-handlers '(wrt-view-mode . wrt-view-restore))

(defun wrt-file (dir name text &optional coding)
  (let ((file (expand-file-name name dir)))
    (let ((coding-system-for-write (or coding 'utf-8-unix))) (write-region text nil file nil 'silent))
    file))

(defun wrt-make (dir)
  "Build the test session over files in DIR."
  (setq default-directory (file-name-as-directory dir))
  (let ((a (find-file-noselect (wrt-file dir "a.txt" "alpha one\nalpha two\nalpha three\n")))
        (b (find-file-noselect (wrt-file dir "b.txt" (mapconcat (lambda (i) (format "line %d" i)) (number-sequence 1 200) "\n"))))
        (c (find-file-noselect (wrt-file dir "c.txt" "see\n")))
        (u (find-file-noselect (wrt-file dir "u.bin" (unibyte-string 0 200 255 10) 'no-conversion) nil t))
        (notes (get-buffer-create "*notes*")))
    ;; Unsaved edits with undo boundaries, mark and narrowing.
    (with-current-buffer a
      (goto-char (point-max)) (insert "added λ") (undo-boundary)
      (goto-char (point-min)) (insert "first ") (undo-boundary)
      (push-mark 3 t) (push-mark 5 t)
      (set-mark 7) (goto-char 12)
      (narrow-to-region 1 25))
    (with-current-buffer b (goto-char 300) (read-only-mode 1))
    (with-current-buffer c (rename-buffer "renamed-c") (insert "x"))
    (with-current-buffer u (goto-char 3))
    (with-current-buffer (make-indirect-buffer a "a-view")
      (widen) (narrow-to-region 7 18) (goto-char 9))
    (with-current-buffer notes
      (text-mode) (setq default-directory "/tmp/") (setq-local fill-column 50)
      (auto-fill-mode 1) (use-hard-newlines 1 'never)
      (insert "para one") (newline) (insert "para two") (insert (string #x3fff80 #x41))
      (goto-char 4))
    ;; A deleted hard newline, which undo must bring back hard.
    (with-current-buffer (get-buffer-create "*hard*")
      (text-mode) (use-hard-newlines 1 'never)
      (insert "a") (newline) (insert "b") (undo-boundary)
      (delete-region 2 3) (undo-boundary))
    ;; A process, and views that are not carried.
    (with-current-buffer (get-buffer-create "*proc*")
      (start-process "wrt-sleep" (current-buffer) "sleep" "60"))
    (with-current-buffer (get-buffer-create "*regen*") (setq warm-restart-regenerable "test view"))
    (with-current-buffer (get-buffer-create "*wrt-view*")
      (wrt-view-mode) (setq-local desktop-save-buffer #'wrt-view-save))
    ;; Windows: b | (a / notes), notes selected, with explicit starts.
    (delete-other-windows)
    (switch-to-buffer b)
    (set-window-start (selected-window) 100)
    (let ((right (split-window-right)))
      (select-window right)
      (switch-to-buffer a)
      (let ((below (split-window-below)))
        (select-window below)
        (switch-to-buffer notes)
        (set-window-point below 6)))
    ;; Globals.
    (setq kill-ring nil)
    (kill-new "kill one") (kill-new (propertize "kill two" 'face 'bold)) (kill-new "kill three")
    (current-kill 1)
    (set-register ?t "register text")
    (set-register ?n 42)
    (set-register ?p (with-current-buffer b (copy-marker 50)))
    (set-register ?q (with-current-buffer notes (copy-marker 3)))
    (window-configuration-to-register ?w)
    (setq minibuffer-history '("one" "two") file-name-history '("/x/y"))
    (defvar wrt-history nil)
    (setq wrt-history '("h1" "h2"))
    (defvar savehist-additional-variables nil)
    (add-to-list 'savehist-additional-variables 'wrt-history)
    (defvar recentf-list nil)
    (setq recentf-list (list (expand-file-name "a.txt" dir) (expand-file-name "b.txt" dir)))))

(defun wrt-buffer (name)
  (let ((b (get-buffer name)))
    (and b (with-current-buffer b
             (list :name name
                   :text (save-restriction (widen) (buffer-substring-no-properties (point-min) (point-max)))
                   :multibyte enable-multibyte-characters
                   :point (point) :mark (mark t) :mark-ring (mapcar #'marker-position mark-ring)
                   :narrowing (and (buffer-narrowed-p) (list (point-min) (point-max)))
                   :modified (and (buffer-modified-p) t) :mode major-mode :read-only buffer-read-only
                   :file (and buffer-file-name (file-name-nondirectory buffer-file-name))
                   :base (and (buffer-base-buffer) (buffer-name (buffer-base-buffer)))
                   :directory default-directory :fill-column fill-column
                   :auto-fill (and auto-fill-function t)
                   :hard (save-restriction (widen) (cl-loop for pos from (point-min) below (point-max)
                                                            when (get-text-property pos 'hard) collect pos))
                   :undo-entries (if (listp buffer-undo-list) (length (delq nil (copy-sequence buffer-undo-list))) buffer-undo-list))))))

(defun wrt-windows ()
  (cl-loop for w in (window-list nil 'no-minibuf (frame-first-window))
           collect (list (buffer-name (window-buffer w)) (window-point w) (window-start w)
                         (eq w (selected-window)) (window-left-column w) (window-top-line w))))

(defun wrt-observe ()
  "What a restore must reproduce."
  (list :buffers (mapcar #'wrt-buffer '("a.txt" "b.txt" "renamed-c" "u.bin" "a-view" "*notes*"))
        :windows (wrt-windows)
        :kill-ring (mapcar #'substring-no-properties kill-ring) :yank (substring-no-properties (car kill-ring-yank-pointer))
        :registers (list (get-register ?t) (get-register ?n)
                         (let ((p (get-register ?p))) (if (markerp p) (list (buffer-name (marker-buffer p)) (marker-position p)) p))
                         (let ((q (get-register ?q))) (and (markerp q) (list (buffer-name (marker-buffer q)) (marker-position q)))))
        :histories (list minibuffer-history file-name-history (bound-and-true-p wrt-history))
        :recentf (mapcar #'file-name-nondirectory (bound-and-true-p recentf-list))))

(defun wrt-undo (name)
  "Undo NAME's last change group as `undo' does, past the boundary that
ends it; its text, and the positions with a `hard' property."
  (with-current-buffer name
    (save-restriction
      (widen)
      (let ((pending-undo-list (if (car buffer-undo-list) buffer-undo-list (cdr buffer-undo-list))))
        (primitive-undo 1 pending-undo-list))
      (list (buffer-substring-no-properties (point-min) (point-max))
            (cl-loop for pos from (point-min) below (point-max) when (get-text-property pos 'hard) collect pos)))))

(defun wrt-undo-a ()
  "The undone texts of a.txt and *hard*."
  (list (wrt-undo "a.txt") (wrt-undo "*hard*")))

(defun wrt-print (tag value)
  "Print VALUE after TAG on one line."
  (let ((print-escape-newlines t)) (princ (format "%s %S\n" tag value))))

;;; warm-restart-session.el ends here
