;;; romacs-attach-test.el --- Attachments' Lisp  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Run by attach.rs in a `--loadup' session with a window system open,
;; so `romacs-image-header-size' answers.  `romacs-attach-test-helper' is
;; the workspace's helper binary, or nil when it was not built.

(require 'romacs-attach)

(defvar romacs-attach-test-helper)
;; Org's options, bound with `let' below before org loads.
(defvar org-yank-dnd-method)
(defvar org-yank-image-save-method)

(defconst romacs-attach-test--fixtures
  (expand-file-name "../harness/fixtures/images/" (file-name-directory load-file-name))
  "Image fixtures.")

(defun romacs-attach-test--fixture (name)
  (expand-file-name name romacs-attach-test--fixtures))

(defmacro romacs-attach-test--in-temp (&rest body)
  "Run BODY with `dir' bound to a fresh temporary directory, removed after."
  (declare (indent 0))
  `(let ((dir (make-temp-file "romacs-attach-test" t)))
     (unwind-protect (progn ,@body) (delete-directory dir t))))

(defun romacs-attach-test--text-file (dir name text)
  (let ((file (expand-file-name name dir)))
    (with-temp-file file (insert text))
    file))


(romacs-attach-define-consumer 'romacs-attach-test-claude
  :display "Claude" :max-count 3 :max-total 100000
  :rules '(("\\`image/\\(png\\|jpeg\\|gif\\|webp\\)\\'" image :max-edge 1568 :strip t
            :formats ("png" "jpeg" "gif" "webp"))
           ("\\`application/pdf\\'" document :max-bytes 30000)
           ("\\`text/" text :max-bytes 20)))

(romacs-attach-define-consumer 'romacs-attach-test-codex
  :display "Codex"
  :rules (lambda (context)
           (if (eq context 'no-images) '(("" path))
             '(("\\`image/" image :max-edge 20 :formats ("png" "jpeg")) ("" path)))))

(romacs-deftest romacs-attach-an-item-is-sniffed-without-decoding ()
  "Type from the signature, pixels from the header probe, nil where the
header does not give them."
  (let ((png (romacs-attach-item (romacs-attach-test--fixture "plain.png") :source 'clipboard :owned t))
        (xpm (romacs-attach-item (romacs-attach-test--fixture "dot.xpm"))))
    (romacs-should (equal (plist-get png :mime) "image/png"))
    (romacs-should (equal (plist-get png :pixels) '(40 . 24)))
    (romacs-should (equal (plist-get png :name) "plain.png"))
    (romacs-should (eq (plist-get png :source) 'clipboard))
    (romacs-should (eq (plist-get png :owned) t))
    (romacs-should (equal (plist-get png :bytes)
                          (file-attribute-size (file-attributes (romacs-attach-test--fixture "plain.png")))))
    (romacs-should (equal (plist-get xpm :mime) "image/x-xpixmap"))
    (romacs-should-not (plist-get xpm :pixels))
    (romacs-should-not (romacs-attach-changed-p png))
    (romacs-should-error (romacs-attach-item "/ssh:host:/etc/passwd"))))

(romacs-deftest romacs-attach-plans-name-the-consumer ()
  "Kinds and limits per rule, refusals worded for the consumer, count and
total limits, the conversion badge."
  (romacs-attach-test--in-temp
    (let* ((png (romacs-attach-item (romacs-attach-test--fixture "plain.png")))
           (xpm (romacs-attach-item (romacs-attach-test--fixture "dot.xpm")))
           (short (romacs-attach-item (romacs-attach-test--text-file dir "a.txt" "short")))
           (long (romacs-attach-item (romacs-attach-test--text-file dir "b.txt" (make-string 64 ?x))))
           (plan (romacs-attach-plan 'romacs-attach-test-claude nil (list png xpm short long png png))))
      (romacs-should (equal (cdr (nth 0 plan)) '(:kind image :prep (:max-edge 1568 :strip t :formats ("png" "jpeg" "gif" "webp")))))
      (romacs-should (equal (cdr (nth 1 plan)) '(:refused "Claude takes images, documents and text; this is neither")))
      (romacs-should (eq (plist-get (cdr (nth 2 plan)) :kind) 'text))
      (romacs-should (equal (plist-get (cdr (nth 3 plan)) :refused) "Claude takes files up to 20 B"))
      (romacs-should (eq (plist-get (cdr (nth 4 plan)) :kind) 'image))
      (romacs-should (equal (plist-get (cdr (nth 5 plan)) :refused) "Claude takes at most 3 attachments"))
      (let ((codex (romacs-attach-plan 'romacs-attach-test-codex nil (list png xpm))))
        (romacs-should (equal (plist-get (cdr (nth 0 codex)) :badge) "→ 20 px"))
        (romacs-should (equal (plist-get (cdr (nth 1 codex)) :badge) "→ PNG")))
      (romacs-should (eq (plist-get (cdr (car (romacs-attach-plan 'romacs-attach-test-codex 'no-images (list png)))) :kind)
                         'path)))))

(romacs-deftest romacs-attach-list-operations-return-new-lists ()
  (let* ((items (list '(:id "a") '(:id "b") '(:id "c")))
         (ids (lambda (items) (mapcar (lambda (i) (plist-get i :id)) items))))
    (romacs-should (equal (funcall ids (romacs-attach-move items "a" 1)) '("b" "a" "c")))
    (romacs-should (equal (funcall ids (romacs-attach-move items "a" 9)) '("b" "c" "a")))
    (romacs-should (equal (funcall ids (romacs-attach-move items "c" -1)) '("a" "c" "b")))
    (romacs-should (equal (funcall ids (romacs-attach-remove items "b")) '("a" "c")))
    (romacs-should (equal (plist-get (nth 1 (romacs-attach-caption items "b" "hi")) :caption) "hi"))
    (romacs-should (equal (funcall ids items) '("a" "b" "c")))
    (romacs-should-not (plist-get (nth 1 items) :caption))))

(romacs-deftest romacs-attach-a-long-name-loses-its-middle ()
  (let ((name (romacs-attach--elide "a-very-long-screenshot-name-from-the-desktop.png" 28)))
    (romacs-should (<= (string-width name) 28))
    (romacs-should (string-suffix-p ".png" name))
    (romacs-should (string-prefix-p "a-very" name))
    (romacs-should (equal (romacs-attach--elide "short.png" 28) "short.png"))))

(romacs-deftest romacs-attach-the-tray-string-is-cached-and-batches-plan-once ()
  "nil without items; one plan for a batch of 20; the same string between
changes; a change makes a new one."
  (with-temp-buffer
    (insert "composer")
    (let* ((plans 0)
           (count (lambda (&rest _) (setq plans (1+ plans)))))
      (romacs-attach-tray-setup :consumer 'romacs-attach-test-claude :anchor (copy-marker (point-min)))
      (romacs-should-not (romacs-attach-tray-string))
      (advice-add 'romacs-attach-plan :before count)
      (unwind-protect
          (romacs-attach-files (make-list 20 (romacs-attach-test--fixture "plain.png")))
        (advice-remove 'romacs-attach-plan count))
      (romacs-should (= plans 1))
      (romacs-should (= (length romacs-attach-items) 20))
      (let ((string (romacs-attach-tray-string)))
        (romacs-should (string-suffix-p "\n" string))
        (romacs-should (string-match-p "plain\\.png" string))
        (romacs-should (string-match-p "at most 3 attachments" string))
        (romacs-should (eq string (romacs-attach-tray-string)))
        (romacs-should (equal (get-text-property 0 'romacs-attach-id string) (plist-get (car romacs-attach-items) :id)))
        (romacs-should (eq (overlay-get (plist-get romacs-attach--tray :overlay) 'before-string) string))
        (romacs-attach-set-items (seq-take romacs-attach-items 1))
        (romacs-should-not (eq string (romacs-attach-tray-string))))
      (romacs-attach-set-items nil)
      (romacs-should-not (romacs-attach-tray-string)))))

(romacs-deftest romacs-attach-a-tray-shows-a-bounded-slice ()
  "At most `romacs-attach-tray-max-chips' chips, the rest counted; the
selection is always shown."
  (let ((plan (mapcar (lambda (i) (list (list :id (number-to-string i)) :kind 'path)) (number-sequence 0 99))))
    (pcase-let ((`(,shown ,before ,after) (romacs-attach--shown plan nil 24)))
      (romacs-should (equal (list (length shown) before after) '(24 0 76))))
    (pcase-let ((`(,shown ,before ,after) (romacs-attach--shown plan "50" 24)))
      (romacs-should (equal (list (length shown) before after) '(24 27 49)))
      (romacs-should (equal (plist-get (car (car (last shown))) :id) "50")))
    (pcase-let ((`(,shown ,before ,after) (romacs-attach--shown plan "99" 24)))
      (romacs-should (equal (list (length shown) before after) '(24 76 0))))))

(romacs-deftest romacs-attach-edit-mode-selects-moves-removes-and-captions ()
  (romacs-attach-test--in-temp
    (with-temp-buffer
      (romacs-attach-tray-setup :consumer 'romacs-attach-test-codex :anchor (copy-marker (point-min)))
      (let* ((owned (romacs-attach-test--text-file dir "owned.txt" "x"))
             (files (list (romacs-attach-test--fixture "plain.png") owned (romacs-attach-test--fixture "star.svg")))
             (names (lambda () (mapcar (lambda (i) (plist-get i :name)) romacs-attach-items))))
        (romacs-attach-set-items (mapcar (lambda (f) (romacs-attach-item f :owned (equal f owned))) files))
        (romacs-attach-edit)
        (romacs-should (equal romacs-attach--selected (plist-get (nth 2 romacs-attach-items) :id)))
        (romacs-attach-edit-previous 1)
        (romacs-attach-edit-move-left 1)
        (romacs-should (equal (funcall names) '("owned.txt" "plain.png" "star.svg")))
        (romacs-attach-edit-caption "the log")
        (romacs-should (equal (plist-get (car romacs-attach-items) :caption) "the log"))
        (romacs-should (string-match-p "“the log”" (romacs-attach-tray-string)))
        (romacs-attach-edit-remove)
        (romacs-should (equal (funcall names) '("plain.png" "star.svg")))
        (romacs-should (file-exists-p owned))
        (romacs-attach-edit-undo)
        (romacs-should (equal (funcall names) '("owned.txt" "plain.png" "star.svg")))
        (romacs-attach-edit-remove)
        ;; Leaving the mode deletes the removed owned file.
        (funcall (lambda () (romacs-attach-delete-owned (mapcar #'cdr romacs-attach--removed))))
        (romacs-should-not (file-exists-p owned))))))

(romacs-deftest romacs-attach-take-commits-only-when-told ()
  (romacs-attach-test--in-temp
    (with-temp-buffer
      (romacs-attach-tray-setup :consumer 'romacs-attach-test-codex :anchor (copy-marker (point-min)))
      (let ((owned (romacs-attach-test--text-file dir "shot.txt" "x")))
        (romacs-attach-files (list owned) :owned t)
        (let ((taken (romacs-attach-take)))
          (romacs-should (= (length (car taken)) 1))
          (romacs-should romacs-attach-items)
          (funcall (cdr taken))
          (romacs-should-not romacs-attach-items)
          (romacs-should-not (file-exists-p owned)))))))

(romacs-deftest romacs-attach-reconcile-keeps-named-files-whatever-their-age ()
  (romacs-attach-test--in-temp
    (let* ((kept (romacs-attach-test--text-file dir "kept.png" "k"))
           (orphan (romacs-attach-test--text-file dir "orphan.png" "o"))
           (old (time-subtract nil (* 40 24 3600))))
      (set-file-times kept old)
      (romacs-attach-reconcile dir (list (list :path kept)))
      (romacs-should (file-exists-p kept))
      (romacs-should-not (file-exists-p orphan)))))

(romacs-deftest romacs-attach-the-viewer-renders-an-image-a-text-and-a-card ()
  (romacs-attach-test--in-temp
    (with-current-buffer (romacs-attach-render (romacs-attach-test--fixture "plain.png") 400 300)
      (romacs-should (string-prefix-p "plain.png  40×24" (buffer-string)))
      (romacs-should (eq (car (get-text-property (line-beginning-position 2) 'display)) 'image)))
    (with-current-buffer (romacs-attach-render (romacs-attach-test--text-file dir "n.txt" "hello text") 400 300)
      (romacs-should (string-match-p "hello text" (buffer-string))))
    (with-current-buffer (romacs-attach-render (romacs-attach-test--fixture "dot.xpm") 400 300)
      (romacs-should (string-match-p "image/x-xpixmap" (buffer-string)))
      (romacs-should-not (text-property-not-all (point-min) (point-max) 'display nil)))))

(romacs-deftest romacs-attach-prepare-runs-the-helper ()
  "Every planned copy through `romacs-attach-tool', and the first error."
  (when romacs-attach-test-helper
    (romacs-attach-test--in-temp
      (let* ((romacs-attach-tool-program romacs-attach-test-helper)
             (plan (romacs-attach-plan 'romacs-attach-test-claude nil
                                       (list (romacs-attach-item (romacs-attach-test--fixture "plain.png"))
                                             (romacs-attach-item (romacs-attach-test--fixture "photo.jpg")))))
             (result nil))
        (romacs-attach-prepare plan (expand-file-name "store" dir) (lambda (r) (setq result r)))
        (with-timeout (20 (error "the helper did not finish"))
          (while (null result) (accept-process-output nil 0.05)))
        (romacs-should (= (length result) 2))
        (dolist (entry result)
          (romacs-should (file-exists-p (plist-get (car entry) :sent-path)))
          (romacs-should (= (length (plist-get (car entry) :sha256)) 64)))
        (let* ((max-image-size 10)
               (failed nil))
          (romacs-attach-prepare plan (expand-file-name "store" dir) (lambda (r) (setq failed r)))
          (with-timeout (20 (error "the helper did not finish"))
            (while (null failed) (accept-process-output nil 0.05)))
          (romacs-should (string-match-p "max-image-size" (plist-get failed :error))))))))

;;; romacs-attach-test.el ends here

(romacs-deftest romacs-attach-a-cancelled-empty-preparation-calls-nothing ()
  (let* ((called nil)
         (plan (romacs-attach-plan 'romacs-attach-test-codex 'no-images
                                   (list (romacs-attach-item (romacs-attach-test--fixture "plain.png")))))
         (handle (romacs-attach-prepare plan temporary-file-directory (lambda (_) (setq called t)))))
    (romacs-attach-cancel handle)
    (accept-process-output nil 0.1)
    (sit-for 0.1)
    (romacs-should-not called)))

(romacs-deftest romacs-attach-take-plans-the-files-as-they-are ()
  "A text file grown past the consumer's limit after it was attached is
refused at take."
  (romacs-attach-test--in-temp
    (with-temp-buffer
      (romacs-attach-tray-setup :consumer 'romacs-attach-test-claude :anchor (copy-marker (point-min)))
      (let ((file (romacs-attach-test--text-file dir "grows.txt" "small")))
        (romacs-attach-files (list file))
        (romacs-should (eq (plist-get (cdr (car (plist-get romacs-attach--tray :plan))) :kind) 'text))
        (with-temp-file file (insert (make-string 64 ?x)))
        (romacs-should (plist-get (cdr (car (car (romacs-attach-take)))) :refused))))))

(romacs-deftest romacs-attach-a-wide-name-fits-its-columns ()
  (let ((name (romacs-attach--elide (concat (make-string 30 ?界) ".png") 28)))
    (romacs-should (<= (string-width name) 28))
    (romacs-should (string-suffix-p ".png" name))))

(romacs-deftest romacs-attach-closing-the-viewer-ends-its-keys ()
  (romacs-attach-view (romacs-attach-test--fixture "plain.png"))
  (romacs-should (memq romacs-attach-view-map (if (keymapp overriding-terminal-local-map)
                                                  (cdr overriding-terminal-local-map)
                                                (list overriding-terminal-local-map))))
  (romacs-attach-view-quit)
  (romacs-should-not (and (keymapp overriding-terminal-local-map)
                          (memq romacs-attach-view-map (cdr overriding-terminal-local-map)))))

(romacs-deftest romacs-attach-mail-and-org-take-attachments-inline ()
  "Without a tray, the mode names the consumer: mail inserts its part."
  (with-temp-buffer
    (insert "To: a@example.org\nSubject: s\n--text follows this line--\n")
    (message-mode)
    (goto-char (point-max))
    (romacs-should (equal (romacs-attach--here) '(message)))
    (romacs-attach-file (romacs-attach-test--fixture "plain.png"))
    (romacs-should (string-match-p "<#part type=\"image/png\" filename=\".*plain\\.png\" disposition=inline>"
                                   (buffer-string)))
    ;; A draft asks before it is killed.
    (set-buffer-modified-p nil))
  (with-temp-buffer
    (romacs-should-not (romacs-attach--here))
    (romacs-should-error (romacs-attach-file (romacs-attach-test--fixture "plain.png")))))


(romacs-deftest romacs-attach-an-owned-file-moves-across-filesystems ()
  "Staging on the runtime tmpfs, the save directory on disk: the move
copies and then deletes the staged file."
  (make-directory (romacs-attach-cache-directory) t)
  (let* ((runtime (getenv "XDG_RUNTIME_DIR"))
         (staged (and runtime (make-temp-file (expand-file-name "romacs-attach-move" runtime) nil ".png")))
         (romacs-attach-save-directory (make-temp-file (expand-file-name "romacs-attach-saved" (romacs-attach-cache-directory)) t)))
    (unwind-protect
        (when (and staged (not (equal (file-attribute-device-number (file-attributes staged))
                                      (file-attribute-device-number (file-attributes romacs-attach-save-directory)))))
          (copy-file (romacs-attach-test--fixture "plain.png") staged t)
          (let ((saved (romacs-attach--users (romacs-attach-item staged :owned t))))
            (romacs-should (file-exists-p saved))
            (romacs-should-not (file-exists-p staged))
            (romacs-should (string-prefix-p (file-name-as-directory romacs-attach-save-directory) saved))))
      (when (and staged (file-exists-p staged)) (delete-file staged))
      (delete-directory romacs-attach-save-directory t))))

(romacs-deftest romacs-attach-org-resolves-a-function-image-save-method ()
  (romacs-attach-test--in-temp
    (require 'org)
    (let ((org-yank-dnd-method 'attach)
          (org-yank-image-save-method (lambda () dir)))
      (with-temp-buffer
        (org-mode)
        (romacs-attach--org-insert (romacs-attach-item (romacs-attach-test--fixture "plain.png")) nil)
        (romacs-should (file-exists-p (expand-file-name "plain.png" dir)))
        (romacs-should (string-match-p (regexp-quote (expand-file-name "plain.png" dir)) (buffer-string)))))))
