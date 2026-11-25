;;; command-loop-tests.el --- GNU command loop parity -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Code:

(require 'ert)
(require 'cl-lib)

(defvar command-loop-tests--log nil)

(defmacro command-loop-tests--with-input (events &rest body)
  "Run BODY in an isolated buffer with EVENTS queued."
  (declare (indent 1) (debug (form body)))
  `(save-window-excursion
     (with-temp-buffer
       (set-window-buffer (selected-window) (current-buffer))
       (let ((unread-command-events ,events)
             (unread-input-method-events nil)
             (unread-post-input-method-events nil)
             (input-method-function nil)
             (keyboard-translate-table nil)
             (input-decode-map (make-sparse-keymap))
             (local-function-key-map (make-sparse-keymap))
             (key-translation-map (make-sparse-keymap))
             (overriding-local-map nil)
             (overriding-terminal-local-map nil)
             (pre-command-hook nil)
             (post-command-hook nil)
             (command-error-function nil)
             (this-command nil)
             (real-this-command nil)
             (this-original-command nil)
             (last-command nil)
             (real-last-command nil)
             (last-repeatable-command nil)
             (last-command-event nil)
             (prefix-arg nil)
             (current-prefix-arg nil)
             (last-prefix-arg nil)
             (deactivate-mark nil)
             (command-loop-tests--log nil))
         (use-local-map (make-sparse-keymap))
         (local-set-key [f12] #'exit-recursive-edit)
         (clear-this-command-keys t)
         (unwind-protect (progn ,@body)
           (clear-this-command-keys t))))))

(defun command-loop-tests--queues ()
  "Return the remaining events in read priority order."
  (list unread-post-input-method-events unread-command-events
        unread-input-method-events))

(defun command-loop-tests--read (reader)
  "Call READER and capture keys, queues and the latest lossage event."
  (let* ((value (funcall reader))
         (lossage (recent-keys)))
    (list value (this-command-keys-vector) (command-loop-tests--queues)
          (and (> (length lossage) 0) (aref lossage (1- (length lossage))))
          last-input-event)))

(defun command-loop-tests--key ()
  "Read a key sequence as a vector."
  (read-key-sequence-vector nil))

(ert-deftest command-loop-tests-unread-wrappers ()
  (command-loop-tests--with-input (list ?a (cons t ?b) (cons 'no-record ?c)
                                        (cons ?d 'disabled))
    (should (equal (command-loop-tests--read #'read-event)
                   '(97 [97] (nil ((t . 98) (no-record . 99) (100 . disabled)) nil) 97 97)))
    (should (equal (command-loop-tests--read #'read-event)
                   '(98 [97 98] (nil ((no-record . 99) (100 . disabled)) nil) 98 98)))
    (should (equal (command-loop-tests--read #'read-event)
                   '(99 [97 98] (nil ((100 . disabled)) nil) 98 99)))
    (should (equal (command-loop-tests--read #'read-event)
                   '(100 [97 98] (nil nil nil) 100 100)))))

(ert-deftest command-loop-tests-source-priority ()
  (command-loop-tests--with-input (list ?b)
    (let ((unread-post-input-method-events (list (list ?a)))
          (unread-input-method-events (list (list ?c)))
          (input-method-function
           (lambda (event)
             (push event command-loop-tests--log)
             (list (+ event 32)))))
      (should (equal (mapcar (lambda (_) (read-event nil t)) '(1 2 3))
                     '(97 130 131)))
      (should (equal (reverse command-loop-tests--log) '(98 99)))
      (should (equal (this-command-keys-vector) [97]))
      (should (equal (command-loop-tests--queues) '(nil nil nil))))))

(ert-deftest command-loop-tests-timed-unread ()
  (command-loop-tests--with-input (list ?a ?b)
    (should (= (read-char nil nil 1) ?a))
    (should (equal (this-command-keys-vector) []))
    (should (equal unread-command-events '(98)))
    (should (= (read-char-exclusive) ?b))
    (should (equal (this-command-keys-vector) [98]))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-unread-bypasses-keyboard-translation ()
  (command-loop-tests--with-input (list ?a)
    (let ((keyboard-translate-table (make-char-table 'keyboard-translate-table)))
      (aset keyboard-translate-table ?a ?z)
      (should (equal (command-loop-tests--read #'read-event)
                     '(97 [97] (nil nil nil) 97 97))))))

(ert-deftest command-loop-tests-translation-keys ()
  (command-loop-tests--with-input (list 'f5)
    (let (seen)
      (define-key key-translation-map [f5]
                  (lambda (_)
                    (setq seen (this-command-keys-vector))
                    [f6]))
      (should (equal (list (command-loop-tests--key) seen
                           (this-command-keys-vector))
                     '([f6] [f5] [f6])))
      (should (equal (command-loop-tests--queues) '(nil nil nil)))
      (should (eq (seq-elt (recent-keys) (1- (length (recent-keys)))) 'f5)))))

(ert-deftest command-loop-tests-shared-menu-item-reader ()
  "GNU calls the shared filter at both occurrences."
  (command-loop-tests--with-input (list 'f5)
    (let* ((hits 0)
           (item (list 'menu-item "x" 'ignore :filter
                       (lambda (_) (and (= (cl-incf hits) 2) 'ignore))))
           (high (list 'keymap (cons 'f5 item)))
           (low (list 'keymap (cons 'f5 item)))
           (overriding-local-map (make-composed-keymap (list high low))))
      (should (equal (list (eq (cdadr high) (cdadr low))
                           (command-loop-tests--key) hits)
                     '(t [f5] 2)))
      (should (equal (this-command-keys-vector) [f5]))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-shared-menu-item-key-binding ()
  "GNU uses the second filter result."
  (command-loop-tests--with-input nil
    (let* ((hits 0)
           (item (list 'menu-item "x" 'ignore :filter
                       (lambda (_) (and (= (cl-incf hits) 2) 'ignore))))
           (high (list 'keymap (cons 'f5 item)))
           (low (list 'keymap (cons 'f5 item)))
           (overriding-local-map (make-composed-keymap (list high low))))
      (should (equal (list (key-binding [f5]) hits) '(ignore 2))))))

(ert-deftest command-loop-tests-textual-key-description ()
  "GNU looks a one-string vector up as the key it describes.
Romacs walks the string as one event (ledger/divergences/VM2-KEYSEQ-TEXT1.md)."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "C-c x") 'command-loop-tests--normalized)
    (should (eq (lookup-key map ["C-c x"]) 'command-loop-tests--normalized))))

(defun command-loop-tests--fallback ()
  (interactive)
  (push 'fallback command-loop-tests--log))

(defun command-loop-tests--replacement ()
  (interactive)
  (push 'replacement command-loop-tests--log))

(defmacro command-loop-tests--mutating-map (&rest body)
  "Run BODY with a menu filter that replaces its own binding."
  (declare (indent 0) (debug t))
  `(let* ((binding (cons 'f5 nil))
          (high (list 'keymap binding (cons 'f12 'exit-recursive-edit)))
          (low (list 'keymap (cons 'f5 'command-loop-tests--fallback)))
          (overriding-local-map (make-composed-keymap (list high low))))
     (setcdr binding
             (list 'menu-item "x" 'ignore :filter
                   (lambda (_)
                     (push 'filter command-loop-tests--log)
                     (setcdr binding 'command-loop-tests--replacement)
                     nil)))
     ,@body))

(ert-deftest command-loop-tests-mutating-menu-item-reader ()
  "GNU runs fallback once before replacement."
  (command-loop-tests--with-input (list 'f5 'f5 'f12)
    (command-loop-tests--mutating-map
      (recursive-edit)
      (should (equal (reverse command-loop-tests--log)
                     '(filter fallback replacement)))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-mutating-menu-item-key-binding ()
  "GNU falls through after the mutation until the next lookup."
  (command-loop-tests--with-input nil
    (command-loop-tests--mutating-map
      (should (eq (key-binding [f5]) 'command-loop-tests--fallback))
      (should (eq (key-binding [f5]) 'command-loop-tests--replacement))
      (should (equal command-loop-tests--log '(filter))))))

(ert-deftest command-loop-tests-recursive-exits ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (cl-letf (((symbol-function 'my-outer)
               (lambda () (interactive)
                 (push (list 'outer-start (recursion-depth)) command-loop-tests--log)
                 (recursive-edit)
                 (push (list 'outer-end (recursion-depth)) command-loop-tests--log)))
              ((symbol-function 'my-inner)
               (lambda () (interactive)
                 (push (list 'inner (recursion-depth)) command-loop-tests--log)
                 (exit-recursive-edit))))
      (local-set-key [f5] 'my-outer)
      (local-set-key [f6] 'my-inner)
      (let ((post-command-hook
             (list (lambda ()
                     (push (list 'post this-command (recursion-depth))
                           command-loop-tests--log)))))
        (recursive-edit))
      (should (equal (reverse command-loop-tests--log)
                     '((post nil 1) (outer-start 1) (post my-outer 2)
                       (inner 2) (outer-end 1) (post my-inner 1))))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-handled-quit ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] #'keyboard-quit)
    (let ((command-error-function
           (lambda (data _context _signal)
             (push (list 'handled data) command-loop-tests--log)))
          (post-command-hook
           (list (lambda () (push (list 'post this-command) command-loop-tests--log)))))
      (recursive-edit))
    (should (equal (reverse command-loop-tests--log)
                   '((post nil) (handled (quit)) (post keyboard-quit))))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-error-order ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (cl-letf (((symbol-function 'my-bad)
               (lambda () (interactive)
                 (push 'bad command-loop-tests--log)
                 (car (identity 1))))
              ((symbol-function 'my-ok)
               (lambda () (interactive) (push 'ok command-loop-tests--log))))
      (local-set-key [f5] 'my-bad)
      (local-set-key [f6] 'my-ok)
      (let ((command-error-function
             (lambda (data _context signal)
               (push (list 'cmd-error (car data) signal this-command)
                     command-loop-tests--log)))
            (post-command-hook
             (list (lambda ()
                     (push (list 'post this-command last-command (recursion-depth))
                           command-loop-tests--log)))))
        (recursive-edit))
      (should (equal (reverse command-loop-tests--log)
                     '((post nil nil 1) bad
                       (cmd-error wrong-type-argument my-bad my-bad)
                       (post my-bad nil 1) ok (post my-ok my-bad 1))))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-entry-resets ()
  (command-loop-tests--with-input (list ?a 'f12)
    (let* ((prefix-arg '(4))
           (last-prefix-arg '(16))
           (deactivate-mark t)
           (state (lambda ()
                    (list prefix-arg last-prefix-arg deactivate-mark
                          (this-command-keys-vector))))
           (post-command-hook (list (lambda () (push (funcall state) command-loop-tests--log)))))
      (should (eq (read-event) ?a))
      (should (equal (funcall state) '((4) (16) t [97])))
      (recursive-edit)
      (should (equal (reverse command-loop-tests--log) '((nil nil nil []))))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-macro-error ()
  (command-loop-tests--with-input nil
    (local-set-key [f5] (lambda () (interactive)
                          (push 'command command-loop-tests--log)
                          (error "audit")))
    (let ((command-error-function (lambda (&rest _) (push 'report command-loop-tests--log)))
          (post-command-hook
           (list (lambda () (push (list 'post this-command) command-loop-tests--log)))))
      (condition-case err (execute-kbd-macro [f5])
        (error (push (list 'caller (car err)) command-loop-tests--log))))
    (should (equal (reverse command-loop-tests--log) '((post nil) command (caller error))))
    (should-not unread-command-events)))

(defun command-loop-tests--watch (symbol value operation locus)
  "Record a watcher notification."
  (push (list symbol value operation locus) command-loop-tests--log))

(ert-deftest command-loop-tests-kboard-watcher ()
  "GNU restores a KBOARD let with set.
Beta's restore_binding emits unlet."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input nil
    (add-variable-watcher 'last-command #'command-loop-tests--watch)
    (unwind-protect
        (progn
          (setq last-command 'audit-set)
          (set-default 'last-command 'audit-default)
          (let ((last-command 'audit-bound))
            (setq last-command 'audit-inner))
          (should (equal (list (reverse command-loop-tests--log)
                               last-command (default-value 'last-command))
                         '(((last-command audit-set set nil)
                            (last-command audit-default set nil)
                            (last-command audit-bound let nil)
                            (last-command audit-inner set nil)
                            (last-command audit-default set nil))
                           audit-default audit-default))))
      (remove-variable-watcher 'last-command #'command-loop-tests--watch))))

(ert-deftest command-loop-tests-kboard-watcher-nonlocal ()
  "GNU restores a KBOARD let with set on throw.
Beta's restore_binding emits unlet."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input nil
    (add-variable-watcher 'last-command #'command-loop-tests--watch)
    (unwind-protect
        (progn
          (setq last-command 'a)
          (catch 'done (let ((last-command 'c)) (throw 'done nil)))
          (should (equal (mapcar #'cdr (reverse command-loop-tests--log))
                         '((a set nil) (c let nil) (a set nil)))))
      (remove-variable-watcher 'last-command #'command-loop-tests--watch))))

(ert-deftest command-loop-tests-global-watcher ()
  "GNU gives two sets on set-default and set then unlet on restore.
Beta's set_default and restore_binding emit one each."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input nil
    (add-variable-watcher 'this-command #'command-loop-tests--watch)
    (unwind-protect
        (progn
          (setq this-command 'a)
          (set-default 'this-command 'b)
          (let ((this-command 'c)) (setq this-command 'd))
          (should (equal (mapcar #'cdr (reverse command-loop-tests--log))
                         '((a set nil) (b set nil) (b set nil) (c let nil)
                           (d set nil) (b set nil) (b unlet nil)))))
      (remove-variable-watcher 'this-command #'command-loop-tests--watch))))

(ert-deftest command-loop-tests-step-watcher ()
  "GNU reports only the Lisp write.
Beta's romacs--command-loop-top and romacs--command-loop-step also call Lisp setters."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] (lambda () (interactive) (setq this-command 'changed)))
    (add-variable-watcher 'this-command #'command-loop-tests--watch)
    (unwind-protect
        (progn
          (recursive-edit)
          (should (equal command-loop-tests--log '((this-command changed set nil)))))
      (remove-variable-watcher 'this-command #'command-loop-tests--watch))))

(defun command-loop-tests--assert-key (keys residue)
  "Assert translated KEYS and the remaining unread RESIDUE."
  (should (equal (command-loop-tests--key) keys))
  (should (equal (this-command-keys-vector) keys))
  (should (equal (command-loop-tests--queues) (list nil residue nil))))

(ert-deftest command-loop-tests-decode-escape ()
  (command-loop-tests--with-input (list 27 ?O ?P 'f12)
    (define-key input-decode-map [27 ?O ?P] [f5])
    (local-set-key [f5] #'ignore)
    (command-loop-tests--assert-key [f5] '(f12))
    (should (equal (this-single-command-raw-keys) [27 ?O ?P]))))

(ert-deftest command-loop-tests-translation-chain ()
  (command-loop-tests--with-input (list 'f2 'f93 'f12)
    (local-set-key [f2] #'ignore)
    (local-set-key [f5] #'ignore)
    (define-key local-function-key-map [f2] [f4])
    (define-key local-function-key-map [f93] [f4])
    (define-key key-translation-map [f4] [f5])
    (command-loop-tests--assert-key [f2] '(f93 f12))
    (command-loop-tests--assert-key [f5] '(f12))
    (define-key key-translation-map [f2] [f5])
    (setq unread-command-events (list 'f2 'f12))
    (command-loop-tests--assert-key [f5] '(f12))))

(ert-deftest command-loop-tests-remap-to-undefined ()
  (command-loop-tests--with-input (list 'f2 'f12)
    (local-set-key [f2] #'forward-char)
    (local-set-key [remap forward-char] #'undefined)
    (define-key local-function-key-map [f2] [f5])
    (command-loop-tests--assert-key [f5] '(f12))))

(ert-deftest command-loop-tests-translation-invalid ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (define-key key-translation-map [f5] (lambda (_) 'invalid))
    (should-error (command-loop-tests--key))
    (should (equal (command-loop-tests--queues) '(nil (f12) nil)))
    (should (equal (this-command-keys-vector) [f5]))))

(ert-deftest command-loop-tests-shift-and-upcase ()
  (command-loop-tests--with-input (list 3 ?A 3 ?A 'S-f5 'f12)
    (local-set-key [3 ?a] #'ignore)
    (local-set-key [f5] #'ignore)
    (let (this-command-keys-shift-translated)
      (command-loop-tests--assert-key [3 ?a] '(3 65 S-f5 f12))
      (should this-command-keys-shift-translated))
    (let (this-command-keys-shift-translated)
      (should (equal (read-key-sequence-vector nil nil t) [3 ?A]))
      (should-not this-command-keys-shift-translated))
    (let (this-command-keys-shift-translated)
      (command-loop-tests--assert-key [f5] '(f12))
      (should this-command-keys-shift-translated))))

(ert-deftest command-loop-tests-shifted-backspace-function-map ()
  (command-loop-tests--with-input (list 'S-backspace 'f12)
    (define-key local-function-key-map [backspace] [127])
    (local-set-key [127] #'ignore)
    (command-loop-tests--assert-key [127] '(f12))
    (should this-command-keys-shift-translated)))

(ert-deftest command-loop-tests-first-unbound-prefix ()
  (command-loop-tests--with-input (list 'f90 'f91 'f92 'f12)
    (define-key key-translation-map [f90 f93] [f5])
    (define-key key-translation-map [f91 f92] [f6])
    (command-loop-tests--assert-key [f6] '(f12))
    (should (equal (this-single-command-raw-keys) [f90 f91 f92]))))

(ert-deftest command-loop-tests-composite-prefix-and-default ()
  (command-loop-tests--with-input (list 3 (cons t ?b) 3 (cons t ?a) 'f8 'f12)
    (let ((minor (make-sparse-keymap))
          (base (make-sparse-keymap)))
      (define-key minor [3 ?a] #'forward-char)
      (define-key base [3 ?a] #'backward-char)
      (define-key base [3 ?b] #'ignore)
      (define-key base [t] #'ignore)
      (let ((overriding-local-map (make-composed-keymap (list minor base))))
        (command-loop-tests--assert-key [3 ?b] '(3 (t . 97) f8 f12))
        (should (eq (key-binding [3 ?b]) 'ignore))
        (command-loop-tests--assert-key [3 ?a] '(f8 f12))
        (should (eq (key-binding [3 ?a]) 'forward-char))
        (command-loop-tests--assert-key [f8] '(f12))
        (should (eq (key-binding [f8] t) 'ignore))))))

(defun command-loop-tests--posn (position)
  "Return a synthetic position in the selected window at POSITION."
  (list (selected-window) position '(0 . 0) 0 nil position
        '(0 . 0) nil '(0 . 0) '(1 . 1)))

(ert-deftest command-loop-tests-mouse-fallbacks ()
  "GNU drops the unbound down event before reading the next key.
Beta's romacs--read-key-sequence returns the down event."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input nil
    (local-set-key [mode-line mouse-1] #'ignore)
    (local-set-key [mouse-9] #'ignore)
    (local-set-key [?a] #'ignore)
    (cl-letf (((get 'mouse-1 'event-kind) 'mouse-click)
              ((get 'drag-mouse-9 'event-kind) 'mouse-click)
              ((get 'down-mouse-9 'event-kind) 'mouse-click))
      (let* ((pos (command-loop-tests--posn 1))
             (click (list 'mouse-1 (command-loop-tests--posn 'mode-line)))
             (down (list 'down-mouse-9 pos))
             (drag (list 'drag-mouse-9 pos pos))
             (unread-command-events (list click down ?a drag 'f12)))
        (command-loop-tests--assert-key (vector 'mode-line click) (list down ?a drag 'f12))
        (command-loop-tests--assert-key [?a] (list drag 'f12))
        (should (equal (command-loop-tests--key) (vector (list 'mouse-9 pos))))
        (should (equal (this-command-keys-vector) (vector drag)))
        (should (equal unread-command-events '(f12)))))))

(ert-deftest command-loop-tests-delayed-switch-frame ()
  (command-loop-tests--with-input nil
    (local-set-key [3 ?a] #'ignore)
    (let* ((event (list 'switch-frame (selected-frame)))
           (unread-command-events (list 3 event (cons t ?a))))
      (command-loop-tests--assert-key [3 ?a] nil)
      (should (equal (command-loop-tests--read (lambda () (read-event nil nil 0.01)))
                     (list event [3 ?a] '(nil nil nil) 97 event))))))

(ert-deftest command-loop-tests-quit-as-key ()
  (command-loop-tests--with-input (list 7 'f12)
    (command-loop-tests--assert-key [7] '(f12))))

(ert-deftest command-loop-tests-command-keys-state ()
  (command-loop-tests--with-input (list ?a 'f6 'f12)
    (should (equal (read-key-sequence nil) "a"))
    (should (equal (this-command-keys) "a"))
    (should (equal (this-single-command-keys) [?a]))
    (should (equal (this-single-command-raw-keys) [?a]))
    (set--this-command-keys "b")
    (should (equal (this-command-keys-vector) [?b]))
    (should (equal (read-key-sequence-vector nil t) [f6]))
    (should (equal (this-command-keys-vector) [?b]))
    (clear-this-command-keys)
    (should (equal (this-command-keys-vector) []))
    (should (equal unread-command-events '(f12)))))

(ert-deftest command-loop-tests-read-char-noncharacter ()
  (command-loop-tests--with-input (list 'f5 ?a 'f12)
    (should-error (read-char) :type 'error)
    (should (equal unread-command-events '(f5)))
    (should (eq (read-event) 'f5))
    (setq unread-command-events (list 'f5 ?a 'f12))
    (should (= (read-char-exclusive) ?a))
    (should (equal unread-command-events '(f12)))))

(ert-deftest command-loop-tests-read-char-timeout ()
  (command-loop-tests--with-input nil
    (should-not (read-char nil nil 0.001))
    (should-not (read-char-exclusive nil nil 0.001))
    (should (equal (this-command-keys-vector) []))
    (should (equal (command-loop-tests--queues) '(nil nil nil)))))

(ert-deftest command-loop-tests-read-char-input-method ()
  (command-loop-tests--with-input (list ?a ?b ?c 'f12)
    (let ((input-method-function
           (lambda (event)
             (push event command-loop-tests--log)
             (list (upcase event)))))
      (should (= (read-char) ?a))
      (should (= (read-char nil t) ?B))
      (should (= (read-char-exclusive nil t) ?C))
      (should (equal (reverse command-loop-tests--log) '(98 99)))
      (should (equal (this-command-keys-vector) [97]))
      (should (equal unread-command-events '(f12))))))

(defun command-loop-tests--note-keys ()
  "Log command keys and raw keys at dispatch."
  (interactive)
  (push (list (this-command-keys-vector) (this-single-command-raw-keys))
        command-loop-tests--log))

(ert-deftest command-loop-tests-macro-string-and-vector ()
  (command-loop-tests--with-input nil
    (local-set-key (kbd "M-a") #'command-loop-tests--note-keys)
    (local-set-key [f5] #'command-loop-tests--note-keys)
    (execute-kbd-macro (unibyte-string #xe1))
    (execute-kbd-macro [f5] 2)
    (should (equal (reverse command-loop-tests--log)
                   (list (list (vector (aref (kbd "M-a") 0))
                               (vector (aref (kbd "M-a") 0)))
                         '([f5] [f5]) '([f5] [f5]))))
    (should (equal (command-loop-tests--queues) '(nil nil nil)))))

(ert-deftest command-loop-tests-macro-loopfunc-and-termination ()
  (command-loop-tests--with-input nil
    (local-set-key [f5] #'command-loop-tests--note-keys)
    (let ((attempts 0)
          (kbd-macro-termination-hook
           (list (lambda () (push 'termination command-loop-tests--log)))))
      (execute-kbd-macro [f5] 0 (lambda () (< (cl-incf attempts) 3)))
      (should (= attempts 3))
      (should (equal (reverse command-loop-tests--log)
                     '(([f5] [f5]) ([f5] [f5]) termination)))
      (should-not executing-kbd-macro))))

(ert-deftest command-loop-tests-macro-reader-exhaustion ()
  (command-loop-tests--with-input nil
    (local-set-key [f5]
                   (lambda ()
                     (interactive)
                     (let ((unread-command-events (list ?a)))
                       (push (list (read-key-sequence nil)
                                   (read-key-sequence nil)
                                   (read-key-sequence-vector nil)
                                   (command-loop-tests--queues))
                             command-loop-tests--log))))
    (execute-kbd-macro [f5])
    (should (equal command-loop-tests--log '(("a" "" [] (nil nil nil)))))))

(ert-deftest command-loop-tests-prefix-arguments ()
  (dolist (case (list (cons '(21) '(4)) (cons '(21 21) '(16))
                      (cons '(21 53) 5) (cons (list (aref (kbd "M-5") 0)) 5)
                      (cons '(21 45) '-) (cons (list (aref (kbd "M--") 0)) '-)
                      (cons '(21 45 21) '(-4))))
    (command-loop-tests--with-input (append (car case) '(f5 f12))
      (let ((state (lambda () (list prefix-arg current-prefix-arg last-prefix-arg
                                    (this-command-keys-vector))))
            (arg (cdr case)))
        (local-set-key [f5] #'command-loop-tests--note-keys)
        (let ((pre-command-hook
               (list (lambda ()
                       (when (eq this-command 'command-loop-tests--note-keys)
                         (push (cons 'pre (funcall state)) command-loop-tests--log)))))
              (post-command-hook
               (list (lambda ()
                       (when (eq this-command 'command-loop-tests--note-keys)
                         (push (cons 'post (funcall state)) command-loop-tests--log))))))
          (recursive-edit))
        (should (equal (reverse command-loop-tests--log)
                       (list (list 'pre arg nil nil [f5])
                             '([f5] [f5]) (list 'post nil arg arg [f5]))))
        (should-not prefix-arg)
        (should-not unread-command-events)))))

(ert-deftest command-loop-tests-command-rotation ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (local-set-key [f5] #'forward-char)
    (local-set-key [remap forward-char] #'command-loop-tests--note-keys)
    (local-set-key [f6] #'ignore)
    (let ((pre-command-hook
           (list (lambda ()
                   (push (list this-original-command this-command real-this-command
                               last-command real-last-command last-repeatable-command)
                         command-loop-tests--log))))
          (post-command-hook
           (list (lambda ()
                   (when (eq this-command 'command-loop-tests--note-keys)
                     (setq this-command 'changed))))))
      (recursive-edit))
    (should (equal (reverse command-loop-tests--log)
                   '((forward-char command-loop-tests--note-keys command-loop-tests--note-keys nil nil nil)
                     ([f5] [f5])
                     (ignore ignore ignore changed command-loop-tests--note-keys command-loop-tests--note-keys)
                     (exit-recursive-edit exit-recursive-edit exit-recursive-edit ignore ignore ignore))))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-deactivate-mark-after-read ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (define-key key-translation-map [f5] (lambda (_) (setq deactivate-mark t) [f6]))
    (local-set-key [f6] #'ignore)
    (let ((pre-command-hook
           (list (lambda () (push (list deactivate-mark (this-command-keys-vector))
                                  command-loop-tests--log)))))
      (recursive-edit))
    (should (equal (reverse command-loop-tests--log) '((nil [f6]) (nil [f12]))))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-nested-translation-and-gc ()
  (command-loop-tests--with-input (list 'f5 ?a 'f12)
    (define-key key-translation-map [f5]
                (lambda (_)
                  (garbage-collect)
                  (push (list (copy-sequence current-key-remap-sequence)
                              (read-key-sequence nil)
                              (this-command-keys-vector))
                        command-loop-tests--log)
                  [f6]))
    (command-loop-tests--assert-key [f6] '(f12))
    (should (equal command-loop-tests--log '(([f5] "a" [97]))))))

(ert-deftest command-loop-tests-throw-from-translation ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (define-key key-translation-map [f5] (lambda (_) (throw 'done 'escape)))
    (should (eq (catch 'done (command-loop-tests--key)) 'escape))
    (command-loop-tests--assert-key [f6] '(f12))))

(ert-deftest command-loop-tests-nested-filter-and-gc ()
  (command-loop-tests--with-input (list 'f5 ?a 'f12)
    (local-set-key [f5]
                   (list 'menu-item "x" 'ignore :filter
                         (lambda (binding)
                           (garbage-collect)
                           (push (read-key-sequence nil) command-loop-tests--log)
                           binding)))
    (should (equal (command-loop-tests--key) [f5]))
    (should (equal (this-command-keys-vector) [?a]))
    (should (equal unread-command-events '(f12)))
    (should (equal command-loop-tests--log '("a")))))

(ert-deftest command-loop-tests-nested-read-continue-echo ()
  (dolist (continue '(nil t))
    (command-loop-tests--with-input (list 'f5 (cons t ?a) 'f12)
      (define-key key-translation-map [f5]
                  (lambda (_)
                    (let ((keys (read-key-sequence nil continue)))
                      (push (list keys (this-command-keys-vector))
                            command-loop-tests--log))
                    [f6]))
      (command-loop-tests--assert-key [f6] '(f12))
      (should (equal command-loop-tests--log
                     (list (list "a" (if continue [f5 ?a] [?a]))))))))

(ert-deftest command-loop-tests-raw-keys-after-mutation ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (define-key key-translation-map [f5]
                (lambda (_)
                  (aset current-key-remap-sequence 0 'f7)
                  [f6]))
    (command-loop-tests--assert-key [f6] '(f12))
    (should (equal (this-single-command-raw-keys) [f5]))))

(ert-deftest command-loop-tests-hook-local-splice-and-failure ()
  (command-loop-tests--with-input (list 'f5 'f5 'f12)
    (local-set-key [f5] #'ignore)
    (let* ((failed (lambda ()
                     (push 'failed command-loop-tests--log)
                     (error "hook failure")))
           (normal (lambda () (push 'default command-loop-tests--log)))
           (pre-command-hook (list failed normal)))
      (setq-local pre-command-hook
                  (list (lambda () (push 'before command-loop-tests--log))
                        t (lambda () (push 'after command-loop-tests--log))))
      (recursive-edit)
      (should (equal (reverse command-loop-tests--log)
                     '(before failed default after before default after before default after)))
      (should (equal (default-value 'pre-command-hook) (list normal)))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-quit-from-hook ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] #'ignore)
    (let ((pre-command-hook (list (lambda () (push 'pre command-loop-tests--log) (signal 'quit nil))))
          (command-error-function
           (lambda (data _context signal)
             (push (list data signal (this-command-keys-vector))
                   command-loop-tests--log))))
      (recursive-edit))
    (should (equal command-loop-tests--log '(pre)))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-throw-from-hook ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] #'ignore)
    (let ((pre-command-hook (list (lambda () (throw 'escape (this-command-keys-vector))))))
      (should (equal (catch 'escape (recursive-edit)) [f5])))
    (should (equal unread-command-events '(f12)))
    (should (zerop (recursion-depth)))))

(defun command-loop-tests--raise-error ()
  (interactive)
  (error "audit"))

(defun command-loop-tests--throw-missing ()
  (interactive)
  (throw 'missing 'value))

(ert-deftest command-loop-tests-error-signal-and-no-catch ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (local-set-key [f5] #'command-loop-tests--raise-error)
    (local-set-key [f6] #'command-loop-tests--throw-missing)
    (let ((command-error-function
           (lambda (data _context signal)
             (push (list data signal (this-command-keys-vector))
                   command-loop-tests--log))))
      (recursive-edit))
    (should (equal (reverse command-loop-tests--log)
                   '(((error "audit") command-loop-tests--raise-error [f5])
                     ((no-catch missing value) command-loop-tests--throw-missing [f6]))))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-nil-this-command ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] #'command-loop-tests--note-keys)
    (let ((pre-command-hook
           (list (lambda ()
                   (when (eq last-command-event 'f5) (setq this-command nil)))))
          (post-command-hook
           (list (lambda ()
                   (when (eq last-command-event 'f5)
                     (push (list this-command real-this-command (this-command-keys-vector))
                           command-loop-tests--log))))))
      (recursive-edit))
    (should (equal command-loop-tests--log '((nil command-loop-tests--note-keys [f5]))))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-kboard-let-recursive ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] #'ignore)
    (setq last-command 'outside)
    (let ((last-command 'inside)
          (post-command-hook
           (list (lambda () (push (list last-command this-command) command-loop-tests--log)))))
      (recursive-edit)
      (should (equal (reverse command-loop-tests--log) '((inside nil) (nil ignore))))
      (should (eq last-command 'ignore)))
    (should (eq last-command 'outside))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-cons-command-repeatability ()
  (command-loop-tests--with-input nil
    (cl-letf (((get 'mouse-1 'event-kind) 'mouse-click))
      (local-set-key [f5] #'ignore)
      (local-set-key [mouse-1] #'command-loop-tests--note-keys)
      (let* ((event (list 'mouse-1 (command-loop-tests--posn 1)))
             (unread-command-events (list 'f5 event 'f12))
             (pre-command-hook
              (list (lambda ()
                      (when (eq this-command 'exit-recursive-edit)
                        (push (list last-command last-repeatable-command)
                              command-loop-tests--log))))))
        (recursive-edit)
        (should (equal command-loop-tests--log
                       (list '(command-loop-tests--note-keys ignore)
                             (list (vector event) (vector event)))))
        (should-not unread-command-events)))))

(ert-deftest command-loop-tests-click-follow-and-remap ()
  (command-loop-tests--with-input nil
    (insert "xy")
    (goto-char 2)
    (let ((map (make-sparse-keymap)))
      (define-key map [mouse-1] #'forward-char)
      (define-key map [remap forward-char] #'command-loop-tests--replacement)
      (put-text-property 1 2 'keymap map)
      (local-set-key [remap forward-char] #'command-loop-tests--note-keys)
      (cl-letf (((get 'mouse-1 'event-kind) 'mouse-click))
        (let* ((event (list 'mouse-1 (command-loop-tests--posn 1)))
               (unread-command-events (list event 'f12)))
          (should (eq (key-binding (vector event) nil t) 'forward-char))
          (let ((pre-command-hook (list (lambda () (push this-command command-loop-tests--log)))))
            (recursive-edit))
          (should (equal command-loop-tests--log
                         '(exit-recursive-edit replacement command-loop-tests--replacement)))
          (should-not unread-command-events))))))

(ert-deftest command-loop-tests-filter-changes-active-map ()
  (command-loop-tests--with-input (list 'f5 'f5 'f12)
    (let ((next (make-sparse-keymap)))
      (define-key next [f5] #'command-loop-tests--replacement)
      (define-key next [f12] #'exit-recursive-edit)
      (local-set-key [f5]
                     (list 'menu-item "x" 'ignore :filter
                           (lambda (_)
                             (push 'filter command-loop-tests--log)
                             (setq overriding-local-map next)
                             'command-loop-tests--fallback)))
      (recursive-edit)
      (should (equal (reverse command-loop-tests--log) '(filter fallback replacement)))
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-minibuffer-command-keys ()
  (command-loop-tests--with-input nil
    (local-set-key
     [f5] (lambda ()
            (interactive)
            (let ((before (this-command-keys-vector))
                  (answer (read-from-minibuffer "Audit: ")))
              (push (list before answer (this-command-keys-vector))
                    command-loop-tests--log))))
    (execute-kbd-macro [f5 ?a 13])
    (should (equal command-loop-tests--log '(([f5] "a" [f5]))))
    (should-not unread-command-events)))

(ert-deftest command-loop-tests-quail-input-method ()
  (command-loop-tests--with-input (list ?' ?e 'f12)
    (activate-input-method "latin-1-prefix")
    (unwind-protect
        (progn
          (should (= (read-char nil t) ?é))
          (should (equal (this-command-keys-vector) [?é]))
          (should (equal unread-command-events '(f12))))
      (deactivate-input-method))))

(ert-deftest command-loop-tests-timer-requeue-during-read ()
  "GNU translates a timed-wait requeue through keyboard-translate-table.
Beta's romacs--read-char retries it through the unread branch."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input nil
    (let ((keyboard-translate-table (make-char-table 'keyboard-translate-table))
          (timer (run-at-time 0.01 nil
                              (lambda () (setq unread-command-events (list (cons t ?a)))))))
      (unwind-protect
          (progn
            (aset keyboard-translate-table ?a ?z)
            (should (equal (command-loop-tests--read (lambda () (read-event nil nil 0.2)))
                           '(122 [] (nil nil nil) 122 122))))
        (cancel-timer timer)))))

(ert-deftest command-loop-tests-prefix-help ()
  "GNU runs prefix-help-command with the prefix and raw help key.
Beta's romacs--read-key-sequence fails to dispatch the help command in this batch fixture."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input (list 'f5 8 'f12)
    (local-set-key [f5 ?a] #'ignore)
    (let ((prefix-help-command #'command-loop-tests--note-keys)
          (help-char 8))
      (recursive-edit)
      (should (equal command-loop-tests--log '(([f5] [f5 8]))))
      (should-not unread-command-events))))

(defun command-loop-tests--batch-top-level ()
  "Drive a top-level throw from batch startup."
  (defalias 'my-top (lambda () (interactive)
                      (push 'my-top command-loop-tests--log)
                      (top-level)))
  (global-set-key [f5] 'my-top)
  (global-set-key [f12] #'exit-recursive-edit)
  (setq command-loop-tests--log nil
        unread-command-events (list 'f5 'f12)
        top-level '(push 'top-level command-loop-tests--log)
        post-command-hook
        (list (lambda ()
                (push (list 'post this-command (recursion-depth)) command-loop-tests--log)))
        kill-emacs-hook
        (list (lambda () (princ (format "KILL %S\n" (reverse command-loop-tests--log))))))
  (recursive-edit)
  (princ "LOAD RESUMED\n"))

(defconst command-loop-tests--file (or load-file-name buffer-file-name))

(ert-deftest command-loop-tests-batch-top-level ()
  "GNU reenters depth zero after the startup throw.
Beta's romacs--top-level reaches batch startup without a catch."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (let ((executable (expand-file-name invocation-name invocation-directory))
        (args (and (fboundp 'romacs--take-user-signal) '("--loadup"))))
    (with-temp-buffer
      (let ((status (apply #'call-process executable nil '(t t) nil
                           (append args (list "-Q" "--batch" "-l" command-loop-tests--file
                                              "-f" "command-loop-tests--batch-top-level")))))
        (should (= status 255))
        (should (equal (split-string (buffer-string) "\n" t)
                       '("No recursive edit is in progress"
                         "KILL ((post nil 1) my-top (post my-top 0))")))))))

(defvar command-loop-tests--minor-active nil)

(ert-deftest command-loop-tests-prefix-wait-keeps-follow-map ()
  "GNU dispatches through the saved prefix map.
Beta's romacs--read-key-sequence refollows the changed map."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input (list 24)
    (local-set-key [24 ?a] #'command-loop-tests--fallback)
    (let* ((map (make-sparse-keymap))
           (command-loop-tests--minor-active nil)
           (minor-mode-map-alist
            (list (cons 'command-loop-tests--minor-active map))))
      (define-key map [24 ?a] #'command-loop-tests--replacement)
      (let ((timer (run-at-time 0.01 nil
                                (lambda ()
                                  (setq command-loop-tests--minor-active t
                                        unread-command-events (list (cons t ?a) 'f12))))))
        (unwind-protect
            (let ((post-command-hook
                   (list (lambda ()
                           (when this-command
                             (push (this-command-keys-vector) command-loop-tests--log))))))
              (recursive-edit)
              (should (equal (reverse command-loop-tests--log) '(fallback [24 97])))
              (should-not unread-command-events))
          (cancel-timer timer))))))

(ert-deftest command-loop-tests-prefix-wait-replays-after-buffer-switch ()
  "GNU handles the signal during the wait and replays the prefix.
Beta's romacs--read-batch-wait resumes on the fallback timer without dispatching the signal."
  :expected-result (if (fboundp 'romacs--take-user-signal) :failed :passed)
  (command-loop-tests--with-input (list 24)
    (local-set-key [24 ?a] #'command-loop-tests--fallback)
    (let* ((other (generate-new-buffer " *command-loop-other*"))
           (special-event-map (make-sparse-keymap))
           (resume (lambda ()
                     (setq unread-command-events (list (cons t ?a) 'f12))))
           (signal-timer (run-at-time 0.01 nil #'signal-process (emacs-pid) 'sigusr1))
           (fallback-timer (run-at-time 0.2 nil resume)))
      (unwind-protect
          (progn
            (define-key special-event-map [sigusr1]
                        (lambda ()
                          (interactive)
                          (push 'signal command-loop-tests--log)
                          (set-buffer other)
                          (funcall resume)))
            (with-current-buffer other
              (use-local-map (make-sparse-keymap))
              (local-set-key [24 ?a] #'command-loop-tests--replacement))
            (define-key key-translation-map [24 ?a]
                        (lambda (_)
                          (push (key-binding [24 ?a]) command-loop-tests--log)
                          [f6]))
            (should (equal (list (command-loop-tests--key) (this-command-keys-vector)
                                 unread-command-events (reverse command-loop-tests--log))
                           '([97] [97] (f12) (signal)))))
        (cancel-timer signal-timer)
        (cancel-timer fallback-timer)
        (kill-buffer other)))))

(ert-deftest command-loop-tests-macro-prefix-finalization ()
  (command-loop-tests--with-input (list 'f5 21 53 'f6 'f7 'f12)
    (local-set-key [f5] #'start-kbd-macro)
    (local-set-key [f6] #'command-loop-tests--note-keys)
    (local-set-key [f7] #'end-kbd-macro)
    (let (last-kbd-macro)
      (recursive-edit)
      (should (equal (vconcat last-kbd-macro) [21 53 f6]))
      (should (equal command-loop-tests--log '(([f6] [f6]))))
      (should-not defining-kbd-macro)
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-macro-minibuffer-quit ()
  (command-loop-tests--with-input nil
    (local-set-key [f5] (lambda () (interactive) (signal 'minibuffer-quit nil)))
    (local-set-key [f6] #'command-loop-tests--note-keys)
    (let ((command-error-function
           (lambda (data _context _signal) (push data command-loop-tests--log))))
      (execute-kbd-macro [f5 f6]))
    (should (equal (reverse command-loop-tests--log)
                   '((minibuffer-quit) ([f6] [f6]))))
    (should-not executing-kbd-macro)))

(ert-deftest command-loop-tests-signal-from-menu-filter ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] (list 'menu-item "x" 'ignore :filter
                              (lambda (_) (error "filter"))))
    (should-not (key-binding [f5]))
    (command-loop-tests--assert-key [f5] '(f12))))

(ert-deftest command-loop-tests-throw-from-menu-filter ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (local-set-key [f5] (list 'menu-item "x" 'ignore :filter
                              (lambda (_) (throw 'escape 'filter))))
    (should (eq (catch 'escape (command-loop-tests--key)) 'filter))
    (command-loop-tests--assert-key [f6] '(f12))))

(ert-deftest command-loop-tests-macro-nested-playback-while-defining ()
  (command-loop-tests--with-input (list 'f5 'f6 'f7 'f12)
    (local-set-key [f5] #'start-kbd-macro)
    (local-set-key [f6]
                   (lambda ()
                     (interactive)
                     (let (unread-command-events)
                       (execute-kbd-macro [f8]))))
    (local-set-key [f7] #'end-kbd-macro)
    (local-set-key [f8] #'command-loop-tests--note-keys)
    (let (last-kbd-macro)
      (recursive-edit)
      (should (equal (vconcat last-kbd-macro) [f6]))
      (should (equal command-loop-tests--log '(([f8] [f8]))))
      (should-not defining-kbd-macro)
      (should-not unread-command-events))))

(ert-deftest command-loop-tests-input-method-retry ()
  (command-loop-tests--with-input (list ?a ?b)
    (let ((input-method-function
           (lambda (event)
             (push event command-loop-tests--log)
             (and (= event ?b) (list ?z ?c)))))
      (should (equal (command-loop-tests--read (lambda () (read-event nil t 0.2)))
                     '(122 [] ((99) nil nil) 122 122)))
      (should (equal (command-loop-tests--read (lambda () (read-event nil t)))
                     '(99 [99] (nil nil nil) 122 99)))
      (should (equal (reverse command-loop-tests--log) '(97 98))))))

(defun command-loop-tests--car-error ()
  (interactive)
  (car (identity 1)))

(ert-deftest command-loop-tests-debugger-command-frames ()
  (command-loop-tests--with-input (list 'f5 'f12)
    (local-set-key [f5] #'command-loop-tests--car-error)
    (let ((debug-on-error t)
          (debugger
           (lambda (&rest _)
             (let ((frames (mapcar #'cadr (backtrace-frames))))
               (throw 'debugged (memq 'car frames))))))
      (let ((frames (catch 'debugged (recursive-edit))))
        (should (equal (seq-take frames 2) '(car command-loop-tests--car-error)))
        (should (memq 'command-execute frames))))
    (should (equal unread-command-events '(f12)))))

(defvar romacs--sequence-end-functions)

(ert-deftest command-loop-tests-reader-cleanup-after-throw ()
  (command-loop-tests--with-input (list 'f5 'f6 'f12)
    (define-key key-translation-map [f5] (lambda (_) (throw 'outer 'body)))
    (let* ((romacs (fboundp 'romacs--take-user-signal))
           (cleanup (lambda ()
                      (push 'cleanup command-loop-tests--log)
                      (throw 'cleanup 'replacement)))
           (romacs--sequence-end-functions (and romacs (list cleanup))))
      (should (eq (catch 'outer
                    (catch 'cleanup
                      (unwind-protect (command-loop-tests--key)
                        (unless romacs (funcall cleanup)))))
                  'replacement)))
    (should (equal command-loop-tests--log '(cleanup)))
    (command-loop-tests--assert-key [f6] '(f12))))

(provide 'command-loop-tests)
;;; command-loop-tests.el ends here
