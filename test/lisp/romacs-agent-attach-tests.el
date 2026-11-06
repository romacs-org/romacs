;;; romacs-agent-attach-tests.el --- Images in the composer and transcript -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defmacro romacs-agent-attach-tests--with-chat (var &rest body)
  "BODY in a chat of a session VAR, its directory temporary."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-attach" t))
          (,var (romacs-agent--make :id "11111111-2222-4333-8444-666666666666" :harness 'claude :state 'idle
                                    :title "chat" :dir dir :tools (make-hash-table :test #'equal))))
     (unwind-protect
         (with-temp-buffer
           (romacs-agent-chat-mode)
           (setq romacs-agent-ui--session ,var
                 romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
                 romacs-agent-ui--tasks (make-hash-table :test #'equal)
                 romacs-agent-ui--messages (make-hash-table :test #'equal)
                 romacs-agent-ui--end (point-marker)
                 romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil))
           ,@body)
       (delete-directory dir t))))

(require 'romacs-agent-images)

(defun romacs-agent-attach-tests--state (&rest names)
  (romacs-agent-images--state (vconcat (mapcar (lambda (name) (concat "/shots/" name)) names))))

(defun romacs-agent-attach-tests--keys (state &rest keys)
  (dolist (key keys state) (setq state (romacs-agent-images--step state key))))

(ert-deftest romacs-agent-images-lists-images-newest-first ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let ((old (expand-file-name "old.png" dir)) (new (expand-file-name "new.jpg" dir)))
          (write-region "x" nil old nil 'silent)
          (write-region "x" nil new nil 'silent)
          (write-region "x" nil (expand-file-name "notes.txt" dir) nil 'silent)
          (make-directory (expand-file-name "folder.png" dir))
          (set-file-times old (time-subtract nil 100))
          (should (equal (romacs-agent-images--files dir) (list new old))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-keys-move-filter-and-end ()
  (let ((state (romacs-agent-attach-tests--state "a-romacs.png" "b-term.png" "c-romacs.png")))
    ;; Moving stays within the matches.
    (should (= 2 (plist-get (romacs-agent-attach-tests--keys state ?\C-n ?\C-n ?\C-n) :selected)))
    (should (= 0 (plist-get (romacs-agent-attach-tests--keys state ?\C-p) :selected)))
    (should (= 2 (plist-get (romacs-agent-attach-tests--keys state ?\C-v) :selected)))
    (should (= 0 (plist-get (romacs-agent-attach-tests--keys state ?\C-v ?\M-v) :selected)))
    ;; Typing filters, every word anywhere in the name, case aside, and
    ;; the selection starts over.
    (let ((typed (romacs-agent-attach-tests--keys state ?\C-n ?R ?o ?m ?\s ?c ?-)))
      (should (equal (plist-get typed :query) "Rom c-"))
      (should (= 0 (plist-get typed :selected)))
      (should (equal (plist-get typed :matches) ["/shots/c-romacs.png"]))
      (should (equal (plist-get (romacs-agent-attach-tests--keys typed ?\C-m) :result) "/shots/c-romacs.png"))
      (should (equal (plist-get (romacs-agent-attach-tests--keys typed ?\d ?\d ?\d) :query) "Rom")))
    ;; RET on no match stays; a stray key changes nothing.
    (let ((none (romacs-agent-attach-tests--keys state ?z ?\C-m 'f5)))
      (should-not (plist-get none :result))
      (should (= 0 (plist-get (romacs-agent-attach-tests--keys none ?\C-p ?\C-n) :selected))))
    (should (equal (romacs-agent-attach-tests--keys state ?\d) state))
    (should (eq (plist-get (romacs-agent-attach-tests--keys state ?\C-g) :result) 'cancel))
    (should (eq (plist-get (romacs-agent-attach-tests--keys state ?\C-i) :result) 'browse))
    ;; A step leaves the old state as it was.
    (romacs-agent-attach-tests--keys state ?a ?\C-n)
    (should (equal (plist-get state :query) ""))))

(ert-deftest romacs-agent-images-a-file-gone-at-ret-shows-the-menu-again ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let* ((kept (expand-file-name "kept.png" dir)) (gone (expand-file-name "gone.png" dir))
               (state (romacs-agent-images--state (vector gone kept))))
          (write-region "x" nil kept nil 'silent)
          (let ((chosen (romacs-agent-images--step state ?\C-m)))
            (should (equal (plist-get chosen :result) gone))
            (let ((again (romacs-agent-images--checked chosen dir)))
              (should-not (plist-get again :result))
              (should (equal (plist-get again :matches) (vector kept)))))
          ;; What is there, and what is no file, pass as they are.
          (let ((good (plist-put (copy-sequence state) :result kept)))
            (should (eq (romacs-agent-images--checked good dir) good)))
          (let ((cancel (plist-put (copy-sequence state) :result 'cancel)))
            (should (eq (romacs-agent-images--checked cancel dir) cancel))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-lays-the-cards-side-by-side ()
  (let* ((layout (romacs-agent-images--layout 0 30 2000 1300 10 20))
         (padding (* 2 (plist-get (alist-get 'card romacs-popup-look) :padding)))
         (list-end (+ (plist-get layout :list-x) (* 10 (plist-get layout :list-columns)) padding))
         (end (+ (plist-get layout :preview-x) (plist-get layout :preview-width) padding)))
    ;; The preview starts past the list, and the pair is centred.
    (should (> (plist-get layout :preview-x) list-end))
    (should (<= (abs (- (plist-get layout :list-x) (- 2000 end))) 1))
    (should (= (plist-get layout :height) 460))
    (should (= (plist-get layout :lines) 23))
    (should (= (plist-get layout :preview-width) (* 10 (plist-get layout :preview-columns))))
    (should (> (plist-get layout :y) 30)))
  ;; It fits as it is drawn, in whole cells: 16 lines of 30 are 480 pixels.
  (should-not (romacs-agent-images--layout 0 0 2000 510 15 30))
  (let* ((layout (romacs-agent-images--layout 0 0 2000 560 15 30))
         (padding (* 2 (plist-get (alist-get 'card romacs-popup-look) :padding))))
    (should (<= (+ (plist-get layout :y) (* 30 (plist-get layout :lines)) padding) 560)))
  ;; A narrow frame gives the preview what is left; too little, or too
  ;; short a frame, is no layout.
  (let ((layout (romacs-agent-images--layout 0 0 1000 1300 10 20)))
    (should (< (plist-get layout :preview-width) 400))
    (should (>= (plist-get layout :preview-width) romacs-agent-images--preview-min-width)))
  (should-not (romacs-agent-images--layout 0 0 700 1300 10 20))
  (should-not (romacs-agent-images--layout 0 0 2000 300 10 20)))

(ert-deftest romacs-agent-images-row-stays-one-line-with-its-age ()
  (let* ((name "screenshot-2026-11-06_09-41-20-agent-with-a-very-long-name.png")
         (row (romacs-agent-images--row-text name "12d" 30)))
    (should (<= (string-width row) 30))
    (should (string-match-p "…  12d\\'" row))
    ;; What fits stays whole.
    (should (equal (romacs-agent-images--row-text "a.png" "1h" 30) "  a.png  1h"))))

(defun romacs-agent-attach-tests--pick-through (layouts)
  "Pick with the frame's layout going through LAYOUTS, one per key, the
size hook firing as `read-key' waits: the result and the layouts shown."
  (let* ((dir (make-temp-file "romacs-images" t)) (shown nil) (now (car layouts)) (next (cdr layouts)))
    (unwind-protect
        (progn
          (write-region "x" nil (expand-file-name "a.png" dir) nil 'silent)
          (cl-letf (((symbol-function 'romacs-agent-images--frame-layout) (lambda () now))
                    ((symbol-function 'romacs-agent-images--show) (lambda (_ _p layout) (push layout shown)))
                    ((symbol-function 'romacs-agent-images--draw-preview) #'ignore)
                    ((symbol-function 'romacs-layer-hide) #'ignore)
                    ((symbol-function 'romacs-layer--root-frame) (lambda () 'root))
                    ((symbol-function 'read-key)
                     (lambda (&rest _)
                       (setq now (if next (pop next) now))
                       (run-hook-with-args 'window-size-change-functions 'root)
                       (if unread-command-events (pop unread-command-events) ?\C-g))))
            (list (romacs-agent-images-pick dir) (reverse shown) window-size-change-functions)))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-follows-the-frame-and-browses-when-it-shrinks ()
  (let ((window-size-change-functions nil) (unread-command-events nil))
    ;; A new size moves the cards; the same one does nothing.
    (pcase-let ((`(,result ,shown ,hooks)
                 (romacs-agent-attach-tests--pick-through '((:y 1) (:y 2)))))
      (should (eq result 'cancel))
      (should (equal shown '((:y 1) (:y 2))))
      (should-not hooks))
    ;; No room left: TAB, once, and the file prompt.
    (pcase-let ((`(,result ,shown ,_)
                 (romacs-agent-attach-tests--pick-through '((:y 1) nil))))
      (should (eq result 'browse))
      (should (equal shown '((:y 1))))
      (should-not unread-command-events))))

(ert-deftest romacs-agent-images-short-frame-browses ()
  (let ((shown nil))
    (cl-letf (((symbol-function 'romacs-agent-images--frame-layout) #'ignore)
              ((symbol-function 'romacs-popup-show) (lambda (&rest _) (setq shown t)))
              ((symbol-function 'read-key) (lambda (&rest _) (error "No key is read"))))
      (should (eq (romacs-agent-images-pick temporary-file-directory) 'browse)))
    (should-not shown)))

(ert-deftest romacs-agent-images-keeps-the-selection-in-view ()
  (should (= 0 (romacs-agent-images--first-row 0 100)))
  (should (= 0 (romacs-agent-images--first-row 2 100)))
  (should (= 6 (romacs-agent-images--first-row 10 100)))
  (should (= 91 (romacs-agent-images--first-row 99 100)))
  (should (= 0 (romacs-agent-images--first-row 1 3))))

(ert-deftest romacs-agent-images-draws-only-the-rows-in-view ()
  (with-temp-buffer
    (let ((state (apply #'romacs-agent-attach-tests--state
                        (mapcar (lambda (n) (format "%02d.png" n)) (number-sequence 0 49))))
          (shown nil))
      (cl-letf (((symbol-function 'romacs-agent-images--age) (lambda (_) "1d"))
                ((symbol-function 'frame-char-width) (lambda (&rest _) 10))
                ((symbol-function 'romacs-agent-images--insert-image)
                 (lambda (file &rest _) (push file shown))))
        (romacs-agent-images--draw (plist-put state :selected 10)))
      (should (= (length shown) romacs-agent-images--rows))
      (should (string-match-p "Attach image:   50/50" (buffer-string)))
      (should (string-match-p "10.png  1d" (buffer-string)))
      (should-not (string-match-p "20.png" (buffer-string))))))

(ert-deftest romacs-agent-attach-owns-c-c-c-a ()
  (should (eq (lookup-key romacs-agent-chat-mode-map (kbd "C-c C-a")) #'romacs-agent-chat-attach))
  (should-not (fboundp 'romacs-agent-chat-respond)))

(ert-deftest romacs-agent-attach-keeps-images-out-of-the-text ()
  (romacs-agent-attach-tests--with-chat session
    (romacs-agent-ui--insert "transcript\n")
    (goto-char (point-max))
    (insert "look")
    (romacs-agent-chat-attach "/tmp/shot one.png")
    (romacs-agent-chat-attach "/tmp/b.png")
    (should (equal (romacs-agent-ui--composer-text) "look"))
    (should (equal romacs-agent-ui--draft-images '("/tmp/shot one.png" "/tmp/b.png")))))

(ert-deftest romacs-agent-send-carries-the-images-and-clears-them ()
  (romacs-agent-attach-tests--with-chat session
    (let ((sent nil))
      (romacs-agent-chat-attach "/tmp/a.png")
      ;; An image alone is a message.
      (cl-letf (((symbol-function 'romacs-agent-send)
                 (lambda (_ text _d _v images) (push text sent) (should (equal images '("/tmp/a.png"))))))
        (goto-char (point-max))
        (romacs-agent-chat-return)
        (should (equal sent '("See the attached image.")))
        (should-not romacs-agent-ui--draft-images))
      ;; A send that fails keeps the draft.
      (romacs-agent-chat-attach "/tmp/b.png")
      (insert "text")
      (cl-letf (((symbol-function 'romacs-agent-send) (lambda (&rest _) (user-error "Gone"))))
        (should-error (romacs-agent-chat-return) :type 'user-error))
      (should (equal romacs-agent-ui--draft-images '("/tmp/b.png")))
      (should (equal (romacs-agent-ui--composer-text) "text")))))

(ert-deftest romacs-agent-remove-image-takes-the-last-or-the-nth ()
  (romacs-agent-attach-tests--with-chat session
    (dolist (file '("/a.png" "/b.png" "/c.png")) (romacs-agent-chat-attach file))
    (romacs-agent-chat-remove-image nil)
    (should (equal romacs-agent-ui--draft-images '("/a.png" "/b.png")))
    (romacs-agent-chat-remove-image 1)
    (should (equal romacs-agent-ui--draft-images '("/b.png")))
    (should-error (romacs-agent-chat-remove-image 5) :type 'user-error)
    (romacs-agent-chat-remove-image nil)
    (should-error (romacs-agent-chat-remove-image nil) :type 'user-error)))

(ert-deftest romacs-agent-tray-text-says-how-to-remove ()
  (should (string-match-p "2 images" (romacs-agent-images--tray-text '("a" "b") 80)))
  (should (string-match-p "\\`1 image " (romacs-agent-images--tray-text '("a") 80)))
  (should (<= (string-width (romacs-agent-images--tray-text '("a") 12)) 12)))

(ert-deftest romacs-agent-tray-never-covers-the-composer ()
  ;; Room below: it sits there, past a composer of many rows too.
  (let ((y (romacs-agent-images--tray-y 0 1000 50 50 20 100)))
    (should (>= y 90))
    (should (<= (+ y 100) 1000)))
  (should (>= (romacs-agent-images--tray-y 0 1000 100 260 20 100) 280))
  ;; None below, as with the composer at the window's bottom: above it.
  (should (= (romacs-agent-images--tray-y 0 1000 950 950 20 100) 818))
  ;; A tall draft with room for neither: no tray.
  (should-not (romacs-agent-images--tray-y 0 300 100 260 20 100))
  (should-not (romacs-agent-images--tray-y 0 120 50 50 20 100)))

(ert-deftest romacs-agent-tray-knows-which-side-of-the-composer-is-gone ()
  ;; Its start scrolled above the window, its tail seen: nothing fits, as
  ;; going above would cover the draft.
  (let ((extent (romacs-agent-images--tray-extent 0 300 nil 260)))
    (should (equal extent '(0 . 260)))
    (should-not (romacs-agent-images--tray-y 0 300 (car extent) (cdr extent) 20 100)))
  ;; Its tail below the window: above its start, when that fits.
  (let ((extent (romacs-agent-images--tray-extent 0 300 200 nil)))
    (should (equal extent '(200 . 300)))
    (should (= (romacs-agent-images--tray-y 0 300 (car extent) (cdr extent) 20 100) 68)))
  ;; All out of view, the transcript being read: the window's bottom.
  (should (equal (romacs-agent-images--tray-extent 0 300 nil nil) '(300 . 300))))

(ert-deftest romacs-agent-copy-leaves-out-the-image-rows ()
  (with-temp-buffer
    (insert "see\n"
            (propertize " \n \n" 'romacs-agent-strip t)
            "the reply\n")
    (should (equal (romacs-agent-ui--text-of (point-min) (point-max)) "see\nthe reply\n"))))

(ert-deftest romacs-agent-adjacent-tables-both-refit ()
  (romacs-agent-attach-tests--with-chat session
    (romacs-agent-ui--helpers)
    (let ((table "| A | B |\n|---|---|\n| one | a rather long cell of text to wrap |\n"))
      (let ((width 70))
        (cl-letf (((symbol-function 'romacs-agent-ui--table-width) (lambda () width)))
          (dolist (id '("m1" "m2"))
            (romacs-agent-ui--draw session (list :type 'delta :text table :message id :seq 1) (current-buffer))
            (romacs-agent-ui--draw session (list :type 'message-end :message id :seq 2) (current-buffer)))
          (let ((inhibit-read-only t) (wide (buffer-string)))
            (setq width 24)
            (romacs-agent-ui--refit-tables)
            (let ((narrow (buffer-string)))
              (should-not (equal narrow wide))
              (setq width 70)
              (romacs-agent-ui--refit-tables)
              (should (equal (buffer-string) wide)))))))))

(ert-deftest romacs-agent-recall-brings-text-and-images-back ()
  (romacs-agent-attach-tests--with-chat session
    (setq romacs-agent-ui--history '(("sent" "/a.png" "/b.png")))
    (goto-char (point-max))
    (insert "draft")
    (romacs-agent-chat-attach "/c.png")
    (romacs-agent-chat-history-previous)
    (should (equal (romacs-agent-ui--composer-text) "sent"))
    (should (equal romacs-agent-ui--draft-images '("/a.png" "/b.png")))
    ;; M-n gives the draft back whole.
    (romacs-agent-chat-history-next)
    (should (equal (romacs-agent-ui--composer-text) "draft"))
    (should (equal romacs-agent-ui--draft-images '("/c.png")))))

(ert-deftest romacs-agent-desktop-keeps-the-draft-s-images ()
  (romacs-agent-attach-tests--with-chat session
    (romacs-agent-chat-attach "/a.png")
    (let ((saved (plist-get (romacs-agent-ui--desktop-save nil) :draft)))
      (should (equal saved ""))
      (should (equal (plist-get (romacs-agent-ui--desktop-save nil) :images) '("/a.png"))))))

(ert-deftest romacs-agent-tray-follows-the-selected-chat ()
  ;; It reads the selected window, whatever buffer a hook runs in, and an
  ;; unchanged tray is not shown again.
  (let ((romacs-agent-images--tray-shown nil) (shows 0) (hides 0) (files '("/a.png")) (selected 'chat))
    (cl-letf (((symbol-function 'romacs-agent-images--tray-files) (lambda (_) (and (eq selected 'chat) files)))
              ((symbol-function 'romacs-agent-images--tray-anchor) (lambda (&rest _) '(10 . 20)))
              ((symbol-function 'romacs-agent-images--tray-draw) #'ignore)
              ((symbol-function 'romacs-popup-show) (lambda (&rest _) (cl-incf shows)))
              ((symbol-function 'romacs-layer-hide) (lambda (&rest _) (cl-incf hides))))
      (romacs-agent-images-tray-sync)
      (romacs-agent-images-tray-sync)
      (should (= shows 1))
      (setq selected 'other)
      (romacs-agent-images-tray-sync)
      (should (= hides 1))
      (should-not romacs-agent-images--tray-shown)
      ;; Nothing shown and nothing to show: no work at all.
      (romacs-agent-images-tray-sync)
      (should (= hides 1)))))

(ert-deftest romacs-agent-table-is-a-grid-and-refits ()
  (romacs-agent-attach-tests--with-chat session
    (romacs-agent-ui--helpers)
    (let ((body "Before\n\n| Where | Cost |\n|---|---|\n| notmuch | a long description of what costs the time |\n\nAfter")
          (width 70))
      (cl-letf (((symbol-function 'romacs-agent-ui--table-width) (lambda () width)))
        (romacs-agent-ui--draw session (list :type 'delta :text body :message "m1" :seq 1) (current-buffer))
        (romacs-agent-ui--draw session (list :type 'message-end :message "m1" :seq 2) (current-buffer))
        (let ((text (buffer-string)))
          (should-not (string-match-p "|---" text))
          (should (string-match-p "Where" text))
          (should (string-match-p "Before" text))
          (should (string-match-p "After" text)))
        (should (text-property-any (point-min) (point-max) 'romacs-agent-table
                                   (get-text-property (text-property-not-all (point-min) (point-max) 'romacs-agent-table nil)
                                                      'romacs-agent-table)))
        (let ((wide (buffer-string)) (inhibit-read-only t))
          (setq width 30)
          (romacs-agent-ui--refit-tables)
          (should-not (equal (buffer-string) wide))
          (should (string-match-p "After" (buffer-string)))
          ;; Refitting again at the same width changes nothing.
          (let ((narrow (buffer-string)))
            (romacs-agent-ui--refit-tables)
            (should (equal (buffer-string) narrow))))))))

(ert-deftest romacs-agent-attach-prefers-the-clipboard ()
  (romacs-agent-attach-tests--with-chat session
    (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) (lambda () "image/png"))
              ((symbol-function 'call-process)
               (lambda (program _in destination _display &rest args)
                 (should (equal program "wl-paste"))
                 (should (equal args '("--type" "image/png")))
                 (with-temp-file (cadr destination) (insert "PNG"))
                 0)))
      (call-interactively #'romacs-agent-chat-attach))
    (let ((file (car romacs-agent-ui--draft-images)))
      (should (string-match-p "/attachments/clipboard-.*\\.png\\'" file))
      (should (string-prefix-p (expand-file-name dir) file))
      (should (file-exists-p file)))))

(ert-deftest romacs-agent-attach-prefix-and-no-clipboard-use-the-picker ()
  (romacs-agent-attach-tests--with-chat session
    (write-region "x" nil (expand-file-name "shot.png" dir) nil 'silent)
    (let ((romacs-agent-image-directory dir) (picked 0))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) (lambda () "image/png"))
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) (cl-incf picked) "/tmp/x.png")))
        (let ((current-prefix-arg '(4))) (call-interactively #'romacs-agent-chat-attach)))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) (cl-incf picked) "/tmp/y.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (should (= picked 2))
      (should (equal romacs-agent-ui--draft-images '("/tmp/x.png" "/tmp/y.png"))))))

(ert-deftest romacs-agent-attach-browse-and-empty-directory-read-a-file ()
  (romacs-agent-attach-tests--with-chat session
    (let ((romacs-agent-image-directory dir) (asked 0))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) (error "No images to pick")))
                ((symbol-function 'read-file-name) (lambda (&rest _) (cl-incf asked) "/tmp/z.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (write-region "x" nil (expand-file-name "shot.png" dir) nil 'silent)
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) 'browse))
                ((symbol-function 'read-file-name) (lambda (&rest _) (cl-incf asked) "/tmp/w.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (should (= asked 2))
      (should (equal romacs-agent-ui--draft-images '("/tmp/z.png" "/tmp/w.png"))))))

(ert-deftest romacs-agent-attach-cancel-adds-nothing ()
  (romacs-agent-attach-tests--with-chat session
    (write-region "x" nil (expand-file-name "shot.png" dir) nil 'silent)
    (let ((romacs-agent-image-directory dir))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) 'cancel)))
        (should (eq (condition-case nil (call-interactively #'romacs-agent-chat-attach) (quit 'quit)) 'quit))))
    (should-not romacs-agent-ui--draft-images)))

(ert-deftest romacs-agent-attach-refuses-a-name-no-token-can-hold ()
  (romacs-agent-attach-tests--with-chat session
    (should-error (romacs-agent-chat-attach "/tmp/shot,one.png") :type 'user-error)
    (should-not romacs-agent-ui--draft-images)))

(ert-deftest romacs-agent-attach-leaves-no-file-when-capture-fails ()
  (romacs-agent-attach-tests--with-chat session
    (dolist (failure (list (lambda (&rest _) 1) (lambda (&rest _) (error "No wl-paste"))))
      (cl-letf (((symbol-function 'call-process) failure))
        (should-error (romacs-agent-ui--save-clipboard-image session "image/png")))
      (should-not (directory-files (expand-file-name "attachments" dir) nil "clipboard-")))))

(ert-deftest romacs-agent-an-image-that-will-not-load-says-so ()
  (romacs-agent-attach-tests--with-chat session
    (let ((broken (expand-file-name "b.png" dir)))
      (write-region "x" nil broken nil 'silent)
      (cl-letf (((symbol-function 'display-images-p) (lambda (&rest _) t))
                ((symbol-function 'create-image) (lambda (&rest _) (error "Bad image"))))
        (romacs-agent-ui--draw session (list :type 'input :seq 1 :text "see" :images (list broken "/gone/c.png")) (current-buffer))
        (romacs-agent-ui--draw session (list :type 'input :seq 2 :text "next") (current-buffer)))
      (should (string-match-p "b.png (cannot show)" (buffer-string)))
      (should (string-match-p "c.png (cannot show)" (buffer-string)))
      (should (string-match-p "> next" (buffer-string))))))

(ert-deftest romacs-agent-text-naming-a-file-attaches-nothing ()
  ;; Typed, then recalled and resent, or edited from the transcript: text is
  ;; never read for files.
  (romacs-agent-attach-tests--with-chat session
    (let ((text "look at [image: /private/secret.png]") (sent nil))
      (goto-char (point-max))
      (insert text)
      (cl-letf (((symbol-function 'romacs-agent-send) (lambda (_ m _d _v images) (push (cons m images) sent))))
        (romacs-agent-chat-return)
        (romacs-agent-chat-history-previous)
        (should-not romacs-agent-ui--draft-images)
        (romacs-agent-chat-return))
      (should (equal sent (list (list text) (list text))))
      (romacs-agent-ui--draw session (list :type 'input :seq 1 :text text) (current-buffer))
      (goto-char (point-min))
      (romacs-agent-chat-edit-turn)
      (should-not romacs-agent-ui--draft-images))))

(ert-deftest romacs-agent-a-turn-owns-its-images-whatever-its-text ()
  ;; The same words with and without an image, and two image-only messages.
  (romacs-agent-attach-tests--with-chat session
    (romacs-agent-ui--draw session (list :type 'input :seq 1 :text "same" :images '("/a.png")) (current-buffer))
    (romacs-agent-ui--draw session (list :type 'input :seq 2 :text "same") (current-buffer))
    (goto-char (point-min))
    (romacs-agent-chat-edit-turn)
    (should (equal romacs-agent-ui--draft-images '("/a.png")))
    (romacs-agent-ui--set-draft-parts "" nil)
    (goto-char (point-min))
    (search-forward "> same" nil t 2)
    (romacs-agent-chat-edit-turn)
    (should-not romacs-agent-ui--draft-images)
    (should (equal (romacs-agent-ui--composer-text) "same"))))

(ert-deftest romacs-agent-the-journal-keeps-references-not-bytes ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)) (wire nil) (kept nil))
      (write-region "PNGBYTES" nil image nil 'silent)
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ w j) (setq wire w kept j))))
        (romacs-agent--input session "see" (list image)))
      ;; The harness gets the bytes; the journal the words and the file.
      (should (string-match-p (regexp-quote (base64-encode-string "PNGBYTES" t)) (romacs-agent--json wire)))
      (should-not (string-match-p (regexp-quote (base64-encode-string "PNGBYTES" t)) (romacs-agent--json kept)))
      (let ((event (car (romacs-agent--claude-input-events session kept))))
        (should (equal (plist-get event :text) "see"))
        (should (equal (plist-get event :images) (list image))))
      ;; Without images there is one line, and no images in its event.
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ w &optional j) (setq wire w kept j))))
        (romacs-agent--input session "plain"))
      (should-not kept)
      (should-not (plist-get (car (romacs-agent--claude-input-events session wire)) :images)))))

(ert-deftest romacs-agent-codex-record-carries-its-images ()
  (let ((record (make-hash-table :test #'equal)))
    (puthash "text" "see" record)
    (puthash "images" ["/a.png"] record)
    (let ((event (car (romacs-agent--codex-input-events nil record))))
      (should (equal (plist-get event :text) "see"))
      (should (equal (plist-get event :images) '("/a.png"))))))

(ert-deftest romacs-agent-attach-refuses-an-oversized-image ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "big.png" dir)) (romacs-agent-image-max-bytes 4))
      (write-region "12345" nil image nil 'silent)
      (should-error (romacs-agent-chat-attach image) :type 'user-error)
      (should-not romacs-agent-ui--draft-images))))

(ert-deftest romacs-agent-a-refused-turn-says-why ()
  (let ((result (make-hash-table :test #'equal)))
    (puthash "is_error" t result)
    (puthash "errors" ["Image too large" "Try a smaller one"] result)
    (should (equal (romacs-agent--claude-reason result) "Image too large; Try a smaller one"))
    (remhash "errors" result)
    (puthash "result" "API Error" result)
    (should (equal (romacs-agent--claude-reason result) "API Error"))
    (puthash "is_error" :false result)
    (should-not (romacs-agent--claude-reason result)))
  (romacs-agent-attach-tests--with-chat session
    (cl-letf (((symbol-function 'romacs-agent-ui--footer) #'ignore))
      (romacs-agent-ui--draw session (list :type 'done :seq 1 :error t :reason "Image too large") (current-buffer)))
    (should (string-match-p "! Image too large" (buffer-string)))))

(ert-deftest romacs-agent-claude-is-given-attached-images-as-content ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)))
      (write-region "PNG" nil image nil 'silent)
      ;; Plain text, however it reads, stays a string.
      (let* ((line (romacs-agent--claude-user session (format "see [image: %s]" image)))
             (content (gethash "content" (gethash "message" line))))
        (should (stringp content)))
      (let* ((romacs-agent--message-images (list image))
             (line (romacs-agent--claude-user session "see"))
             (content (gethash "content" (gethash "message" line)))
             (block (aref content 1)))
        (should (equal (gethash "text" (aref content 0)) "see"))
        (should (equal (gethash "type" block) "image"))
        (should (equal (gethash "media_type" (gethash "source" block)) "image/png"))
        (should (equal (gethash "data" (gethash "source" block)) (base64-encode-string "PNG" t)))
        ;; What the journal shows of it is the text alone.
        (should (equal (romacs-agent--text-of content) "see"))))))

(defun romacs-agent-attach-tests--turn-argv (session send)
  "The command line SEND makes SESSION's per-turn harness run."
  (let ((argv nil))
    (cl-letf (((symbol-function 'romacs-agent--instance) (lambda (&rest _) (list :program "codex")))
              ((symbol-function 'romacs-agent--environment-words) #'ignore)
              ((symbol-function 'romacs-agent--codex-mcp-words) #'ignore)
              ((symbol-function 'romacs-agent--check-config-dir) #'ignore)
              ((symbol-function 'romacs-agent--attach) #'ignore)
              ((symbol-function 'romacs-agent--launch) (lambda (_ words &rest _) (setq argv words))))
      (funcall send))
    argv))

(ert-deftest romacs-agent-only-attached-images-reach-codex ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "secret.png" dir))
          (codex (romacs-agent--make :id "22222222-2222-4333-8444-666666666666" :harness 'codex :state 'idle
                                     :title "codex" :dir dir :harness-session "thread")))
      (write-region "x" nil image nil 'silent)
      ;; A child's result quoting a file uploads nothing.
      (let ((argv (romacs-agent-attach-tests--turn-argv
                   codex (lambda () (romacs-agent-send codex (format "Delegated result: [image: %s]" image))))))
        (should argv)
        (should-not (member "--image" argv)))
      (setf (romacs-agent-state codex) 'idle)
      ;; Attached, it goes; before --json, so the prompt stays the last word.
      (let ((argv (romacs-agent-attach-tests--turn-argv
                   codex (lambda () (romacs-agent-send codex "see" nil nil (list image))))))
        (should (equal (seq-take argv 6) (list "codex" "exec" "resume" "thread" "--image" image)))
        (should (equal (nth 6 argv) "--json"))
        (should (equal (car (last argv)) "-")))
      ;; Queued, it waits with its message.
      (setf (romacs-agent-state codex) 'running)
      (romacs-agent-send-later codex "later" nil (list image))
      (setf (romacs-agent-state codex) 'idle)
      (let ((argv (romacs-agent-attach-tests--turn-argv codex (lambda () (romacs-agent--send-next codex)))))
        (should (member image argv))))))

(ert-deftest romacs-agent-attach-loads-the-tray ()
  ;; The clipboard's way in reads no file: attaching alone must bring it.
  (romacs-agent-attach-tests--with-chat session
    (let ((required nil))
      (cl-letf (((symbol-function 'require) (lambda (feature &rest _) (push feature required) t))
                ((symbol-function 'romacs-agent-ui--tray) #'ignore))
        (romacs-agent-chat-attach "/tmp/a.png"))
      (should (memq 'romacs-agent-images required)))))

(ert-deftest romacs-agent-m-n-keeps-the-draft-as-written ()
  (romacs-agent-attach-tests--with-chat session
    (setq romacs-agent-ui--history '(("sent")))
    (goto-char (point-max))
    (insert "   indented draft\n\n")
    (romacs-agent-chat-attach "/c.png")
    (romacs-agent-chat-history-previous)
    (romacs-agent-chat-history-next)
    (should (equal (buffer-substring-no-properties romacs-agent-ui--end (point-max)) "   indented draft\n\n"))
    (should (equal romacs-agent-ui--draft-images '("/c.png")))))

(ert-deftest romacs-agent-png-size-is-read-from-the-header ()
  (let ((file (make-temp-file "romacs-png" nil ".png"))
        (head (unibyte-string 137 ?P ?N ?G 13 10 26 10 0 0 0 13 ?I ?H ?D ?R))
        (end (unibyte-string 0 0 0 0 ?I ?E ?N ?D 174 66 96 130)))
    (cl-flet ((png (&rest parts) (with-temp-file file (set-buffer-multibyte nil) (apply #'insert parts))))
      (unwind-protect
          (progn
            (png head (unibyte-string 0 0 9 228 0 0 5 67) (make-string 9 ?x) end)
            (should (equal (romacs-agent-ui--png-size file) '(2532 . 1347)))
            ;; Cut short: no IEND.
            (png head (unibyte-string 0 0 9 228 0 0 5 67) (make-string 30 ?x))
            (should-not (romacs-agent-ui--png-size file))
            ;; Zero sizes.
            (png head (unibyte-string 0 0 0 0 0 0 5 67) (make-string 9 ?x) end)
            (should-not (romacs-agent-ui--png-size file))
            ;; Too short to be one, and no PNG at all.
            (png head)
            (should-not (romacs-agent-ui--png-size file))
            (png (make-string 60 ?x))
            (should-not (romacs-agent-ui--png-size file)))
        (delete-file file))))
  (should (equal (romacs-agent-ui--fitted-size '(2532 . 1347) 480 270) '(480 . 256)))
  (should (equal (romacs-agent-ui--fitted-size '(100 . 50) 480 270) '(100 . 50)))
  (should (equal (romacs-agent-ui--fitted-size '(1725 . 70) 480 270) '(480 . 20))))

(ert-deftest romacs-agent-transcript-shows-images-as-rows-below-the-prompt ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)))
      (write-region "x" nil image nil 'silent)
      (cl-letf (((symbol-function 'display-images-p) (lambda (&rest _) t))
                ((symbol-function 'create-image) (lambda (file &rest _) (list 'image :file file)))
                ((symbol-function 'romacs-agent-ui--png-size) (lambda (_) '(100 . 50)))
                ((symbol-function 'default-line-height) (lambda () 20)))
        (romacs-agent-ui--draw session (list :type 'input :seq 1 :text "see this" :images (list image)) (current-buffer)))
      (goto-char (point-min))
      (should (looking-at-p "> see this\n"))
      ;; Below the prompt, the image as rows outside the turn's text.
      (forward-line 1)
      (should (= (count-lines (point) romacs-agent-ui--end) 4))
      (let ((display (get-text-property (point) 'display)))
        (should (equal (car display) '(slice 0 0 100 20)))
        (should (equal (cadr display) (list 'image :file image))))
      ;; A strip per line, the last what is left.
      (forward-line 2)
      (should (equal (car (get-text-property (point) 'display)) '(slice 0 40 100 10)))
      (should-not (get-text-property (point) 'romacs-agent-turn))
      ;; Its name below, should the image draw blank.
      (forward-line 1)
      (should (looking-at-p "  ▣ a.png\n")))))
