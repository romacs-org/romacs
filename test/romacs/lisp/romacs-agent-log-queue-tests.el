;;; romacs-agent-log-queue-tests.el --- What waits to be sent, in a chat -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-log)
  (require 'romacs-agent-menu))
(require 'romacs-agent-test-attached)
(require 'romacs-agent-test-stored)

;; These tests start no supervisor or harness.
(advice-add 'romacs-agent--check-companions :override #'ignore)

;; What runs before a redisplay: a batch session has no host to run it.
(unless (fboundp 'romacs--run-window-change-functions)
  (load (expand-file-name "../vm/window-change.el" (file-name-directory (locate-library "romacs-agent.el"))) nil t))

;; A test session's write: each message line caught, from the first.
(defmacro romacs-agent-log-queue-tests--writing (sent &rest body)
  "BODY with the text of each message written pushed onto SENT."
  (declare (indent 1))
  `(cl-letf (((symbol-function 'romacs-agent--line)
              (lambda (_ object &optional _journal) (push (gethash "content" (gethash "message" object)) ,sent)))
             ((symbol-function 'romacs-agent--fold-now) #'ignore))
     ,@body))

(defmacro romacs-agent-log-queue-tests--chat (session &rest body)
  "BODY in SESSION's chat, registered as its live view, separator drawn."
  (declare (indent 1))
  `(with-temp-buffer
     (romacs-agent-log-mode)
     (setq romacs-agent-log--session ,session
           romacs-agent-log--end (point-min-marker)
           romacs-agent-log--input (point-min-marker)
           romacs-agent-log--filled t)
     (set-marker-insertion-type romacs-agent-log--end t)
     ;; Loaded, as a session a chat shows live is.
     (puthash (romacs-agent-id ,session) ,session romacs-agent--sessions)
     (puthash (romacs-agent-id ,session) (current-buffer) romacs-agent-log--chats)
     (unwind-protect
         (progn (romacs-agent-log--separator t) ,@body)
       (remhash (romacs-agent-id ,session) romacs-agent-log--chats)
       (remhash (romacs-agent-id ,session) romacs-agent--sessions))))

(defun romacs-agent-log-queue-tests--rows ()
  "The texts of the queued rows the chat shows, in order."
  (let ((at romacs-agent-log--end) (rows nil))
    (while (setq at (text-property-not-all at romacs-agent-log--input 'romacs-agent-log-queued nil))
      (let ((end (or (next-single-property-change at 'romacs-agent-log-queued nil romacs-agent-log--input) romacs-agent-log--input)))
        (push (replace-regexp-in-string "\\`[ »<]*[0-9]+ +" "" (string-trim (buffer-substring-no-properties at end))) rows)
        (setq at end)))
    (nreverse rows)))

(defun romacs-agent-log-queue-tests--compose (text)
  "Make the composer say TEXT, point at its end."
  (goto-char (point-max))
  (delete-region romacs-agent-log--input (point-max))
  (insert text))

(defun romacs-agent-log-queue-tests--key (map key)
  "Run KEY's command in MAP, as typed."
  (call-interactively (keymap-lookup map key)))

(defun romacs-agent-log-queue-tests--queue (text)
  "Queue TEXT as its user does: in the composer, C-c C-q."
  (romacs-agent-log-queue-tests--compose text)
  (romacs-agent-log-queue-tests--key romacs-agent-log-mode-map "C-c C-q"))

(defun romacs-agent-log-queue-tests--row (n)
  "Point to the Nth queued row, by n from the transcript's end."
  (goto-char romacs-agent-log--end)
  (dotimes (_ n) (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "n")))

(ert-deftest romacs-agent-log-queue-acceptance ()
  ;; Three queued as the user does; the second edited in the composer, the
  ;; first dropped, by their keys: from the first write to the last, the
  ;; agent gets exactly the edited second, then the third, in order, and
  ;; the store keeps what the queue holds, no more.
  (romacs-agent-test-with-store
    (make-directory (file-name-directory (romacs-agent-store--file)) t)
    (should (romacs-agent-store--open))
    (let* ((id "c0c0c0c0-0000-4000-8000-000000000000")
           (session (romacs-agent-test-attach
                     (romacs-agent--make :id id :harness 'claude :state 'running :title "q")
                     t))
           (sent nil))
      (romacs-agent-log-queue-tests--writing sent
        (romacs-agent-log-queue-tests--chat session
          (romacs-agent-log-queue-tests--queue "first")
          (romacs-agent-log-queue-tests--queue "second")
          (romacs-agent-log-queue-tests--queue "third")
          (should (equal (romacs-agent-log-queue-tests--rows) '("first" "second" "third")))
          ;; n twice, e: the second in the composer, held in its place.
          (romacs-agent-log-queue-tests--row 2)
          (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "e")
          (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "second"))
          (should (romacs-agent-item-held (cadr (romacs-agent-queue session))))
          (should (text-property-search-backward 'romacs-agent-log-queued nil nil))
          (should (string-match-p "»2 +second" (buffer-substring-no-properties romacs-agent-log--end romacs-agent-log--input)))
          (romacs-agent-log-queue-tests--compose "second, edited")
          ;; RET: back in its place, said anew, nothing sent.
          (romacs-agent-log-queue-tests--key romacs-agent-log-mode-map "RET")
          (should (string-empty-p (buffer-substring-no-properties romacs-agent-log--input (point-max))))
          (should (equal (romacs-agent-log-queue-tests--rows) '("first" "second, edited" "third")))
          (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("first" "second, edited" "third")))
          ;; d on the first.
          (romacs-agent-log-queue-tests--row 1)
          (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "d")
          (should (equal (romacs-agent-log-queue-tests--rows) '("second, edited" "third")))
          (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("second, edited" "third")))
          (should-not sent)
          ;; The turn ends, then the next: each goes at a turn's end, its
          ;; row gone as it goes.
          (dotimes (_ 3)
            (setf (romacs-agent-state session) 'idle)
            (romacs-agent--dispatch session))
          (should-not (romacs-agent-log-queue-tests--rows))))
      (should (equal (nreverse sent) '("second, edited" "third")))
      (should-not (romacs-agent-queue session))
      (should-not (romacs-agent-store-queued id)))))

(ert-deftest romacs-agent-log-queue-holds-what-is-edited ()
  ;; Held to edit, the head and what waits behind it wait; an empty
  ;; composer puts it back as it was, and it goes.
  (let* ((session (romacs-agent-test-attach
                   (romacs-agent--make :id "c1c1c1c1-0000-4000-8000-000000000000" :harness 'claude :state 'running)
                   t))
         (sent nil))
    (romacs-agent-log-queue-tests--writing sent
      (romacs-agent-log-queue-tests--chat session
        (romacs-agent-log-queue-tests--queue "one")
        (romacs-agent-log-queue-tests--queue "two")
        (romacs-agent-log-queue-tests--row 1)
        (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "RET")
        (setf (romacs-agent-state session) 'idle)
        (romacs-agent--dispatch session)
        (should-not sent)
        (romacs-agent-log-queue-tests--compose "")
        (romacs-agent-log-queue-tests--key romacs-agent-log-mode-map "RET")
        (should (equal (romacs-agent-log-queue-tests--rows) '("one" "two")))
        (romacs-agent--dispatch session)
        (should (equal sent '("one")))))))

(ert-deftest romacs-agent-log-queue-a-steer-never-passes-a-hold ()
  ;; A steer passes what waits for a new turn, not what its user holds.
  (let ((session (romacs-agent-test-attach
                  (romacs-agent--make :id "c3c3c3c3-0000-4000-8000-000000000000" :harness 'claude :state 'running)
                  t)))
    (let ((next (romacs-agent-item "next" :kind 'next :user t))
          (held (romacs-agent-item "held" :kind 'next :user t))
          (steer (romacs-agent-item "steer" :kind 'now)))
      (setf (romacs-agent-item-turn steer) (romacs-agent-turns session))
      (dolist (item (list next held steer)) (romacs-agent--enqueue session item))
      (cl-letf (((symbol-function 'romacs-agent--steerable-p) #'always))
        (should (eq (romacs-agent--next-item session) steer))
        (should (romacs-agent-queue-hold session held t))
        (should-not (romacs-agent--next-item session))))))

(ert-deftest romacs-agent-log-queue-moves-and-keeps-others-messages ()
  ;; M-p and M-n change places, and being put first with them; another
  ;; session's message, or Romacs's own, shows but does not change; a
  ;; draft in the composer is never overwritten; the ends say so.
  (let ((session (romacs-agent-test-attach
                  (romacs-agent--make :id "c2c2c2c2-0000-4000-8000-000000000000" :harness 'claude :state 'running)
                  t)))
    (romacs-agent-log-queue-tests--chat session
      (romacs-agent-log-queue-tests--queue "mine")
      (romacs-agent-send-later session "theirs" nil nil nil (list (list :from "lane x" :text "theirs")))
      (romacs-agent--enqueue session (romacs-agent-item "Romacs's own"))
      (romacs-agent-log-queue-tests--queue "last")
      (setf (romacs-agent-item-front (car (romacs-agent-queue session))) t)
      (romacs-agent-log-queue-tests--row 1)
      (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "M-n")
      (should (equal (romacs-agent-log-queue-tests--rows) '("theirs" "mine" "Romacs's own" "last")))
      (should (equal (romacs-agent-item-text (romacs-agent-log-queue--item-at)) "mine"))
      ;; Put first goes with the place.
      (should (romacs-agent-item-front (car (romacs-agent-queue session))))
      (should-not (romacs-agent-item-front (cadr (romacs-agent-queue session))))
      (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "M-p")
      (should (equal (romacs-agent-log-queue-tests--rows) '("mine" "theirs" "Romacs's own" "last")))
      (should-error (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "M-p") :type 'user-error)
      (romacs-agent-log-queue-tests--row 2)
      (should-error (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "d") :type 'user-error)
      (romacs-agent-log-queue-tests--row 3)
      (should-error (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "e") :type 'user-error)
      (should-not (romacs-agent-queue-drop session (nth 2 (romacs-agent-queue session))))
      (romacs-agent-log-queue-tests--compose "a draft")
      (romacs-agent-log-queue-tests--row 1)
      (should-error (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "e") :type 'user-error)
      (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "a draft")))))

(ert-deftest romacs-agent-log-queue-acts-only-on-the-row-at-point ()
  ;; From the composer the menu's rows are inapt and the commands say
  ;; where to go; on a row they act on it, and at an end a move is inapt.
  (let ((session (romacs-agent-test-attach
                  (romacs-agent--make :id "c4c4c4c4-0000-4000-8000-000000000000" :harness 'claude :state 'running)
                  t)))
    (romacs-agent-log-queue-tests--chat session
      (romacs-agent-log-queue-tests--queue "a")
      (romacs-agent-log-queue-tests--queue "b")
      (goto-char (point-max))
      (should (romacs-agent-log-queue-not-here-p))
      (should-error (romacs-agent-log-queue-drop) :type 'user-error)
      (should-error (romacs-agent-log-queue-sooner) :type 'user-error)
      (should (equal (romacs-agent-log-queue-tests--rows) '("a" "b")))
      (romacs-agent-log-queue-tests--row 1)
      (should-not (romacs-agent-log-queue-not-here-p))
      (should (romacs-agent-log-queue-not-here-p -1))
      (should-not (romacs-agent-log-queue-not-here-p 1))
      ;; Edit inapt with a draft; drop inapt on the one being edited.
      (should-not (romacs-agent-log-queue-cannot-edit-p))
      (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "e")
      (romacs-agent-log-queue-tests--row 1)
      (should (romacs-agent-log-queue-cannot-edit-p))
      (should (romacs-agent-log-queue-cannot-drop-p)))))

(ert-deftest romacs-agent-log-queue-a-held-message-gone-leaves-a-draft ()
  ;; Taken from the queue while held (its records failed it): putting it
  ;; back says so, and the composer keeps its text as a draft.
  (let ((session (romacs-agent-test-attach
                  (romacs-agent--make :id "c5c5c5c5-0000-4000-8000-000000000000" :harness 'claude :state 'running)
                  t)))
    (romacs-agent-log-queue-tests--chat session
      (romacs-agent-log-queue-tests--queue "gone soon")
      (romacs-agent-log-queue-tests--row 1)
      (romacs-agent-log-queue-tests--key romacs-agent-log-queue-map "e")
      (romacs-agent--dequeue session (car (romacs-agent-queue session)))
      (should-error (romacs-agent-log-queue-tests--key romacs-agent-log-mode-map "RET") :type 'user-error)
      (should-not romacs-agent-log-queue--editing)
      (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max)) "gone soon")))))

;;; One write per session, whole

(ert-deftest romacs-agent-log-queue-writes-its-rows-whole ()
  ;; A move is one write under the session's one key, carrying every kept
  ;; row in its new order: a newer write always stands, whole.
  (romacs-agent-test-with-store
    (make-directory (file-name-directory (romacs-agent-store--file)) t)
    (should (romacs-agent-store--open))
    (let* ((id "c6c6c6c6-0000-4000-8000-000000000000")
           (session (romacs-agent-test-attach (romacs-agent--make :id id :harness 'claude :state 'running) t))
           (writes nil))
      (romacs-agent-send-later session "a" nil nil nil nil t)
      (romacs-agent-send-later session "b" nil nil nil nil t)
      (let ((write (symbol-function 'romacs-agent-write)))
        (cl-letf (((symbol-function 'romacs-agent-write)
                   (lambda (function args &optional done key) (push (list function key) writes) (funcall write function args done key))))
          (should (romacs-agent-queue-move session (car (romacs-agent-queue session)) 1))))
      (should (equal writes `((romacs-agent-store--queued-rows (queued ,id)))))
      (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("b" "a"))))))

(ert-deftest romacs-agent-log-queue-a-load-merges-before-it-writes ()
  ;; Sent to while it loads, a session writes no rows: its kept ones are
  ;; not merged yet.  Merged, all are written, kept ones first.
  (romacs-agent-test-with-store
    (make-directory (file-name-directory (romacs-agent-store--file)) t)
    (should (romacs-agent-store--open))
    (let* ((id "c7c7c7c7-0000-4000-8000-000000000000")
           (first (romacs-agent-test-attach (romacs-agent--make :id id :harness 'claude :state 'running :title "k") t)))
      (romacs-agent-send-later first "kept" nil nil nil nil t)
      (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("kept")))
      (let ((again (romacs-agent-test-attach (romacs-agent--make :id id :harness 'claude :state 'running :title "k") t)))
        (puthash id nil romacs-agent--queue-loading)
        (unwind-protect
            (progn
              (romacs-agent-send-later again "meanwhile" nil nil nil nil t)
              (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("kept")))
              (romacs-agent-store--requeue again nil)
              (should-not (gethash id romacs-agent--queue-loading))
              (should (equal (mapcar #'car (romacs-agent-queue again)) '("kept" "meanwhile")))
              (should (equal (mapcar #'car (romacs-agent-store-queued id)) '("kept" "meanwhile"))))
          (remhash id romacs-agent--queue-loading))))))

(ert-deftest romacs-agent-log-queue-keys-run-in-the-chat ()
  ;; Its row's keys and the menu's rows run commands of this mode.
  (dolist (key '("n" "p" "e" "RET" "d" "M-p" "M-n"))
    (let ((command (keymap-lookup romacs-agent-log-queue-map key)))
      (should (commandp command))
      (should (memq 'romacs-agent-log-mode (command-modes command)))))
  (let (commands)
    (map-keymap (lambda (_ binding)
                  ;; A menu row's (menu-item NAME COMMAND . PROPS).
                  (when (eq (car-safe binding) 'menu-item) (setq binding (nth 2 binding)))
                  (when (symbolp binding) (push binding commands)))
                romacs-agent-menu-map)
    (dolist (command '(romacs-agent-log-queue-edit romacs-agent-log-queue-drop
                       romacs-agent-log-queue-sooner romacs-agent-log-queue-later))
      (should (memq command commands)))))

(ert-deftest romacs-agent-log-queue-rows-scale ()
  ;; Many waiting: one row each, drawn in time linear in their number.
  (let* ((items (lambda (n) (mapcar (lambda (i) (romacs-agent-item (format "message %d" i) :kind 'next)) (number-sequence 1 n))))
         (time (lambda (n) (let ((list (funcall items n)) (start (float-time)))
                             (romacs-agent-log-queue-rows list 80)
                             (- (float-time) start)))))
    ;; Its label, then one row each.
    (should (= (length (split-string (romacs-agent-log-queue-rows (funcall items 50) 80) "\n" t)) 51))
    (funcall time 200)
    ;; Ten times the rows, well under a hundred times the time.
    (let ((small (max (funcall time 200) 1e-4)) (large (funcall time 2000)))
      (should (< large (* 40 small))))))

(provide 'romacs-agent-log-queue-tests)
;;; romacs-agent-log-queue-tests.el ends here
