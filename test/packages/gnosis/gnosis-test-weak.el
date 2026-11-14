;;; gnosis-test-weak.el --- Weak-spot signal tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:

;; Scoring, aggregation, query, buffer indicators and batch launch for
;; `gnosis-weak'.  History comes from the real scheduler writer.

;;; Code:

(require 'ert)
(require 'gnosis-weak)
(require 'gnosis-review)
(require 'gnosis-test-helpers)

(defvar gnosis-review--running)

(defvar gnosis-test-weak--clock 1700000000000000
  "Monotonic synthetic review time in microseconds.")

(defun gnosis-test-weak--day (offset)
  "Return the logical day OFFSET days from today."
  (gnosis--date-to-int (gnosis-date offset)))

(defun gnosis-test-weak--history (id outcomes)
  "Accept OUTCOMES for ID through the scheduler, as (OUTCOME . DAYS-AGO)."
  (pcase-dolist (`(,outcome . ,ago) outcomes)
    (setq gnosis-test-weak--clock (+ gnosis-test-weak--clock 1000000))
    (gnosis-scheduler-accept-review (gnosis-scheduler-event-id) id outcome
                                    gnosis-test-weak--clock
                                    (gnosis-test-weak--day (- ago)))))

(defun gnosis-test-weak--node (id title)
  "Index node ID with TITLE."
  (gnosis--insert-into 'nodes `([,id "notes.org" ,title "1" nil nil nil])))

(defmacro gnosis-test-weak--fixture (&rest body)
  "Run BODY with nodes heart/valves/lungs and scored themata.
Bind `weak', `healthy', `new', `suspended' and `lung' thema IDs."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db
     (gnosis-test-weak--node "heart" "Heart")
     (gnosis-test-weak--node "valves" "Valves")
     (gnosis-test-weak--node "lungs" "Lungs")
     (let ((weak (gnosis-test--add-basic-thema "Which valve fails?" "mitral"))
           (healthy (gnosis-test--add-basic-thema "Chambers?" "four"))
           (new (gnosis-test--add-basic-thema "Unseen?" "x"))
           (suspended (gnosis-test--add-basic-thema "Suspended?" "y" nil nil nil 1))
           (lung (gnosis-test--add-basic-thema "Lobes right lung?" "three")))
       (gnosis-test-weak--history weak '((failure . 40) (failure . 30) (failure . 20)
                                         (success . 19)))
       (gnosis-test-weak--history healthy '((success . 3)))
       (gnosis-test-weak--history lung '((success . 2)))
       (gnosis--insert-into 'thema-links
                            `([,weak "valves"] [,healthy "heart"] [,new "heart"]
                              [,suspended "valves"] [,lung "lungs"]))
       ,@body)))

;;; Pure scoring

(ert-deftest gnosis-weak-thema-score-bounds-and-monotonicity ()
  (should-not (gnosis-weak-thema-score nil nil 0 0))
  (should-not (gnosis-weak-thema-score 0 5 0 0))
  (let* ((fresh (gnosis-weak-thema-score 10.0 5.0 0 0))
         (overdue (gnosis-weak-thema-score 10.0 5.0 60 0))
         (hard (gnosis-weak-thema-score 10.0 9.0 0 0))
         (lapsed (gnosis-weak-thema-score 10.0 5.0 0 3))
         (worst (gnosis-weak-thema-score 0.1 10.0 3650 99)))
    (should (= 1.0 (plist-get fresh :retrievability)))
    ;; Retrievability is the pinned FSRS curve: 90% at elapsed = stability.
    (should (< (abs (- 0.9 (plist-get (gnosis-weak-thema-score 10.0 5.0 10 0)
                                       :retrievability)))
               1e-6))
    (should (< (plist-get fresh :score) gnosis-weak-threshold))
    (dolist (worse (list overdue hard lapsed))
      (should (> (plist-get worse :score) (plist-get fresh :score))))
    (should (<= (plist-get worst :score) 1.0))
    (should (>= (plist-get worst :score) 0.85))
    (should (= 99 (plist-get worst :lapses)))
    ;; Negative elapsed days (clock skew) never raises retrievability above 1.
    (should (= 1.0 (plist-get (gnosis-weak-thema-score 10.0 5.0 -4 0) :retrievability)))))

(ert-deftest gnosis-weak-aggregate-nodes-items-and-input-preserved ()
  (let* ((today 20261012)
         (rows (list (list "a" 1 1.0 9.0 20260901 4 20260910 "Hard\nmultiline" "A")
                     (list "a" 2 30.0 3.0 20261011 0 20261110 "Easy" "A")
                     (list "a" 3 nil nil nil 0 20261012 "New" "A")
                     (list "b" 4 50.0 2.0 20261010 0 20261201 "Fine" "B")
                     (list "a" 5 2.0 8.0 20261001 2 20261003 "Medium" "A")))
         (copy (copy-tree rows))
         (nodes (gnosis-weak-aggregate rows today)))
    (should (equal copy rows))
    (should (equal '("a" "b") (mapcar (lambda (n) (plist-get n :node)) nodes)))
    (let ((a (car nodes)) (b (cadr nodes)))
      (should (equal "A" (plist-get a :title)))
      (should (= 3 (plist-get a :themata)))
      (should (= 1 (plist-get a :unscored)))
      (should (= 2 (plist-get a :weak)))
      (should (equal '(1 5) (mapcar (lambda (i) (plist-get i :thema)) (plist-get a :items))))
      (should (equal "Hard multiline" (plist-get (car (plist-get a :items)) :keimenon)))
      (should (= (plist-get a :score) (plist-get (car (plist-get a :items)) :score)))
      (should (= 0 (plist-get b :weak)))
      (should (= 0.0 (plist-get b :score))))
    ;; A stricter threshold drops the medium item; a lax one admits all scored.
    (should (= 1 (plist-get (car (gnosis-weak-aggregate rows today 0.6)) :weak)))
    (should (= 3 (plist-get (car (gnosis-weak-aggregate rows today 0.0)) :weak)))))

;;; Query

(ert-deftest gnosis-weak-nodes-query-real-history ()
  (gnosis-test-weak--fixture
    (let ((all (gnosis-weak-nodes)))
      (should (equal '("valves") (mapcar (lambda (n) (plist-get n :node)) all)))
      (let ((valves (car all)))
        (should (equal "Valves" (plist-get valves :title)))
        ;; Suspended themata are not counted at all.
        (should (= 1 (plist-get valves :themata)))
        (should (equal (list weak) (mapcar (lambda (i) (plist-get i :thema))
                                           (plist-get valves :items))))
        (should (= 3 (plist-get (car (plist-get valves :items)) :lapses)))))
    (should-not (gnosis-weak-nodes '("heart" "lungs")))
    (should (gnosis-weak-nodes '("valves" "valves")))
    ;; The new thema is reported as unscored, never weak.
    (let ((heart (car (gnosis-weak-aggregate (gnosis-weak--rows '("heart"))
                                             (gnosis--today-int)))))
      (should (= 1 (plist-get heart :unscored)))
      (should (= 1 (plist-get heart :themata))))
    ;; Everything is weak at threshold zero, sorted weakest first.
    (let ((scores (mapcar (lambda (n) (plist-get n :score)) (gnosis-weak-nodes nil 0.0))))
      (should (= 3 (length scores)))
      (should (equal scores (sort (copy-sequence scores) #'>))))))

(ert-deftest gnosis-weak-nodes-reads-without-writing ()
  (gnosis-test-weak--fixture
    (let ((before (mapcar (lambda (table)
                            (gnosis-sqlite-select gnosis-db (format "SELECT * FROM %s ORDER BY 1" table)))
                          '(scheduler_state review_events thema_links nodes)))
          (changes (gnosis-sqlite-select gnosis-db "SELECT total_changes()")))
      (gnosis-weak-nodes)
      (gnosis-weak-nodes '("valves"))
      (should (equal changes (gnosis-sqlite-select gnosis-db "SELECT total_changes()")))
      (should (equal before
                     (mapcar (lambda (table)
                               (gnosis-sqlite-select gnosis-db (format "SELECT * FROM %s ORDER BY 1" table)))
                             '(scheduler_state review_events thema_links nodes)))))))

;;; Buffer indicators

(ert-deftest gnosis-weak-indicator-severity-glyphs ()
  (let ((mild (gnosis-weak--indicator '(:node "a" :weak 2 :themata 3 :score 0.4 :items nil)))
        (severe (gnosis-weak--indicator '(:node "b" :weak 1 :themata 1 :score 0.7 :items nil))))
    (should (string-suffix-p "○ 2" mild))
    (should (eq 'gnosis-weak (get-text-property (1- (length mild)) 'face mild)))
    (should (string-suffix-p "● 1" severe))
    (should (eq 'gnosis-weak-severe (get-text-property (1- (length severe)) 'face severe)))
    (should (equal "b" (get-text-property (1- (length severe)) 'gnosis-weak-node severe)))))

(defconst gnosis-test-weak--note
  ":PROPERTIES:
:ID:       heart
:END:
#+title: Heart

* Chambers
Four of them.
* Valves
:PROPERTIES:
:ID:       valves
:END:
Mitral, tricuspid.
** Example block
#+begin_example
:ID: lungs
#+end_example
* Lungs
:PROPERTIES:
:ID:       lungs
:END:
"
  "Note text with a root node, two heading nodes and a decoy ID line.")

(defun gnosis-test-weak--settle ()
  "Run the current buffer's deferred indicator work now.
Batch Emacs never goes idle, so tests run the idle timer's job directly."
  (when (fboundp 'gnosis-weak--run-pending)
    (gnosis-weak--run-pending (current-buffer))))

(defmacro gnosis-test-weak--with-note (&rest body)
  "Visit a disposable note in `gnosis-nodes-dir' and run BODY.
Bind `file' and `buffer'."
  (declare (indent 0) (debug t))
  `(let* ((gnosis-nodes-dir (expand-file-name "notes" gnosis-test--dir))
          (file (expand-file-name "heart.org" gnosis-nodes-dir))
          (org-id-track-globally nil)
          (buffer nil))
     (make-directory gnosis-nodes-dir t)
     (with-temp-file file (insert gnosis-test-weak--note))
     ;; Replace fabricated rows with the native index of the real file.
     (gnosis-sqlite-execute gnosis-db "DELETE FROM nodes")
     (gnosis-nodes-update-file file)
     (unwind-protect
         (progn (setq buffer (find-file-noselect file))
                (with-current-buffer buffer
                  (gnosis-test-weak--settle)
                  ,@body))
       (when (buffer-live-p buffer)
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer)))))

(defun gnosis-test-weak--indicators ()
  "Return (LINE . TEXT) for each weak indicator in the current buffer."
  (sort (mapcar (lambda (ov)
                  (cons (line-number-at-pos (overlay-start ov))
                        (substring-no-properties (overlay-get ov 'after-string))))
                (seq-filter (lambda (ov) (overlay-get ov 'gnosis-weak))
                            (overlays-in (point-min) (point-max))))
        (lambda (a b) (< (car a) (car b)))))

(ert-deftest gnosis-weak-mode-marks-only-weak-nodes-and-never-writes ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--with-note
      (should gnosis-nodes-mode)
      ;; Enabled automatically through `gnosis-nodes-mode-hook'.
      (should gnosis-weak-mode)
      (let ((text (buffer-string))
            (bytes (with-temp-buffer (insert-file-contents-literally file) (buffer-string))))
        (should (equal '((8 . " ● 1")) (gnosis-test-weak--indicators)))
        (let* ((ov (seq-find (lambda (ov) (overlay-get ov 'gnosis-weak))
                             (overlays-in (point-min) (point-max))))
               (label (overlay-get ov 'after-string))
               (pos (string-search "●" label)))
          (should (eq 'gnosis-weak-severe (get-text-property pos 'face label)))
          (should (string-match-p "1 of 1 reviewed themata weak"
                                  (get-text-property pos 'help-echo label)))
          (should (string-match-p "Which valve fails" (get-text-property pos 'help-echo label)))
          (should (equal "valves" (get-text-property pos 'gnosis-weak-node label)))
          ;; Keyboard access: the heading line itself carries the details.
          (save-excursion
            (goto-char (point-min))
            (forward-line 7)
            (should (string-match-p "Which valve fails" (get-char-property (point) 'help-echo)))))
        ;; Making lungs weak and refreshing adds its marker; the decoy does not.
        (gnosis-test-weak--history lung '((failure . 1) (failure . 0)))
        (gnosis-weak-refresh)
        (should (equal '(8 17) (mapcar #'car (gnosis-test-weak--indicators))))
        ;; Root nodes anchor on the title line.
        (gnosis-test-weak--history healthy '((failure . 1) (failure . 0)))
        (gnosis-weak-refresh)
        (should (equal '(4 8 17) (mapcar #'car (gnosis-test-weak--indicators))))
        (should (equal text (buffer-string)))
        (should-not (buffer-modified-p))
        (should (equal bytes (with-temp-buffer (insert-file-contents-literally file)
                                               (buffer-string))))
        (gnosis-weak-mode -1)
        (should-not (gnosis-test-weak--indicators))
        (should (equal text (buffer-string)))
        (should-not (buffer-modified-p))))))

(ert-deftest gnosis-weak-refresh-uses-one-query-and-respects-narrowing ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--with-note
      (let ((calls 0))
        (cl-letf* ((orig (symbol-function 'sqlite-select))
                   ((symbol-function 'sqlite-select)
                    (lambda (&rest args) (cl-incf calls) (apply orig args))))
          (goto-char (point-min))
          (re-search-forward "^\\* Valves")
          (org-narrow-to-subtree)
          (let ((point (point)) (restriction (cons (point-min) (point-max))))
            (gnosis-weak-refresh)
            (should (= point (point)))
            (should (equal restriction (cons (point-min) (point-max))))))
        ;; One query; the SQLite variable-limit probe may be cached.
        (should (<= calls 2))
        (widen)
        (should (equal '(8) (mapcar #'car (gnosis-test-weak--indicators))))))))

(ert-deftest gnosis-weak-save-refreshes-without-changing-text ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--with-note
      (should (equal '(8) (mapcar #'car (gnosis-test-weak--indicators))))
      (gnosis-test-weak--history healthy '((failure . 1) (failure . 0)))
      ;; Indicators are cached until a refresh trigger such as saving.
      (should (equal '(8) (mapcar #'car (gnosis-test-weak--indicators))))
      (set-buffer-modified-p t)
      (let ((text (buffer-string)))
        (save-buffer)
        (should (equal text (buffer-string))))
      ;; Saving schedules the refresh for idle time instead of blocking.
      (should (equal '(8) (mapcar #'car (gnosis-test-weak--indicators))))
      (gnosis-test-weak--settle)
      (should (equal '(4 8) (mapcar #'car (gnosis-test-weak--indicators))))
      ;; The node index survived the save, so links still resolve.
      (should (equal "Heart:Valves" (gnosis-get 'title 'nodes '(= id "valves")))))))

(defun gnosis-test-weak--anchored ()
  "Return (NODE . HEADING-LINE) for every weak overlay, ignoring narrowing.
Fail unless each overlay spans exactly one whole line."
  (save-restriction
    (widen)
    (sort (mapcar (lambda (ov)
                    (save-excursion
                      (goto-char (overlay-start ov))
                      (should (bolp))
                      (should (= (overlay-end ov) (line-end-position)))
                      (cons (overlay-get ov 'gnosis-weak-node)
                            (buffer-substring-no-properties (point) (line-end-position)))))
                  (seq-filter (lambda (ov) (overlay-get ov 'gnosis-weak))
                              (overlays-in (point-min) (point-max))))
          (lambda (a b) (string< (car a) (car b))))))

(ert-deftest gnosis-weak-narrowed-refresh-and-disable-cover-whole-buffer ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--history lung '((failure . 1) (failure . 0)))
    (gnosis-test-weak--with-note
      (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves"))
                     (gnosis-test-weak--anchored)))
      ;; Valves is inside the restriction, Lungs outside it.
      (goto-char (point-min))
      (re-search-forward "^\\* Valves")
      (org-narrow-to-subtree)
      (dotimes (_ 3) (gnosis-weak-refresh))
      (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves"))
                     (gnosis-test-weak--anchored)))
      (gnosis-weak-mode -1)
      (should-not (gnosis-test-weak--anchored))
      (gnosis-weak-mode 1)
      (gnosis-test-weak--settle)
      (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves"))
                     (gnosis-test-weak--anchored)))
      (gnosis-weak-mode -1)
      (widen)
      (should-not (gnosis-test-weak--indicators)))))

(ert-deftest gnosis-weak-indicators-follow-structural-edits ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--history lung '((failure . 1) (failure . 0)))
    (gnosis-test-weak--with-note
      (let ((calls 0))
        (cl-letf* ((orig (symbol-function 'sqlite-select))
                   ((symbol-function 'sqlite-select)
                    (lambda (&rest args) (cl-incf calls) (apply orig args))))
          ;; A body line typed at the heading end never carries the mark.
          ;; Org no longer sees the property drawer there, so the node has
          ;; no ID until the line is removed again.
          (goto-char (point-min))
          (re-search-forward "^\\* Valves")
          (let ((start (point)))
            (insert "\nNew body text")
            (should (equal '(("lungs" . "* Lungs")) (gnosis-test-weak--anchored)))
            (gnosis-test-weak--settle)
            (should (equal '(("lungs" . "* Lungs")) (gnosis-test-weak--anchored)))
            (delete-region start (point)))
          (gnosis-test-weak--settle)
          (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves"))
                         (gnosis-test-weak--anchored)))
          ;; Body text after the drawer leaves the mark on the heading.
          (re-search-forward "^:END:")
          (insert "\nNew body text")
          (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves"))
                         (gnosis-test-weak--anchored)))
          (gnosis-test-weak--settle)
          (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves"))
                         (gnosis-test-weak--anchored)))
          ;; Extending the heading, at its end and its start.
          (goto-char (point-min))
          (re-search-forward "^\\* Valves")
          (insert " and more")
          (beginning-of-line)
          (insert "*")
          (should (equal '(("lungs" . "* Lungs") ("valves" . "** Valves and more"))
                         (gnosis-test-weak--anchored)))
          (delete-char 1)
          ;; Typing never queries the database.
          (gnosis-test-weak--settle)
          (should (= 0 calls))))
      ;; Moving a subtree keeps each mark on its own heading.
      (goto-char (point-min))
      (re-search-forward "^\\* Lungs")
      (org-move-subtree-up)
      (gnosis-test-weak--settle)
      (should (equal '(("lungs" . "* Lungs") ("valves" . "* Valves and more"))
                     (gnosis-test-weak--anchored)))
      (should (< (progn (goto-char (point-min)) (re-search-forward "^\\* Lungs"))
                 (progn (goto-char (point-min)) (re-search-forward "^\\* Valves"))))
      ;; Deleting a node's ID removes its mark at once.
      (goto-char (point-min))
      (re-search-forward "^:ID: +valves\n")
      (delete-region (match-beginning 0) (match-end 0))
      (should (equal '(("lungs" . "* Lungs")) (gnosis-test-weak--anchored)))
      (gnosis-test-weak--settle)
      (should (equal '(("lungs" . "* Lungs")) (gnosis-test-weak--anchored)))
      ;; Deleting a whole heading removes its mark too.
      (goto-char (point-min))
      (re-search-forward "^\\* Lungs")
      (beginning-of-line)
      (delete-region (point) (save-excursion (org-end-of-subtree t t) (point)))
      (should-not (gnosis-test-weak--anchored))
      (gnosis-test-weak--settle)
      (should-not (gnosis-test-weak--anchored)))))

(defun gnosis-test-weak--settle-all (&rest buffers)
  "Settle the deferred indicator work of each of BUFFERS."
  (dolist (b buffers) (with-current-buffer b (gnosis-test-weak--settle))))

(defun gnosis-test-weak--edit-in (buffer regexp &optional text)
  "In BUFFER, type TEXT after the first match of REGEXP, or delete the match."
  (with-current-buffer buffer
    (save-restriction
      (widen)
      (goto-char (point-min))
      (re-search-forward regexp)
      (if text (insert text) (delete-region (match-beginning 0) (match-end 0))))))

(defun gnosis-test-weak--views (&rest buffers)
  "Return each of BUFFERS' anchored indicators."
  (mapcar (lambda (b) (with-current-buffer b (gnosis-test-weak--anchored))) buffers))

(ert-deftest gnosis-weak-indirect-views-stay-anchored ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--history lung '((failure . 1) (failure . 0)))
    (gnosis-test-weak--with-note
      (let* ((base (current-buffer))
             (both '(("lungs" . "* Lungs") ("valves" . "* Valves")))
             (ind (clone-indirect-buffer "*gnosis-weak-indirect*" nil))
             (calls 0))
        (unwind-protect
            (cl-letf* ((orig (symbol-function 'sqlite-select))
                       ((symbol-function 'sqlite-select)
                        (lambda (&rest args) (cl-incf calls) (apply orig args))))
              (with-current-buffer ind
                (should gnosis-weak-mode)
                ;; The clone owns its pending work, not the base's timer.
                (should-not gnosis-weak--timer))
              ;; Cloning with work pending gives the clone its own timer.
              (with-current-buffer base (gnosis-weak--schedule 'place))
              (let ((clone (clone-indirect-buffer "*gnosis-weak-clone*" nil)))
                (unwind-protect
                    (with-current-buffer clone
                      (should (eq 'place gnosis-weak--pending))
                      (should (equal (list clone) (timer--args gnosis-weak--timer)))
                      (should-not (eq gnosis-weak--timer
                                      (buffer-local-value 'gnosis-weak--timer base))))
                  (kill-buffer clone)))
              (should (memq (buffer-local-value 'gnosis-weak--timer base) timer-idle-list))
              (gnosis-test-weak--settle-all base)
              (should (equal (list both both) (gnosis-test-weak--views base ind)))
              ;; Rename through the indirect view; then through the base.
              (gnosis-test-weak--edit-in ind "^\\* Valves" " renamed")
              (gnosis-test-weak--settle-all base ind)
              (should (equal (make-list 2 '(("lungs" . "* Lungs")
                                            ("valves" . "* Valves renamed")))
                             (gnosis-test-weak--views base ind)))
              (gnosis-test-weak--edit-in base "^\\* Lungs" " renamed")
              (gnosis-test-weak--settle-all base ind)
              (should (equal (make-list 2 '(("lungs" . "* Lungs renamed")
                                            ("valves" . "* Valves renamed")))
                             (gnosis-test-weak--views base ind)))
              ;; Delete an ID through each view: both lose the mark at once.
              (gnosis-test-weak--edit-in ind "^:ID: +valves\n")
              (should (equal (make-list 2 '(("lungs" . "* Lungs renamed")))
                             (gnosis-test-weak--views base ind)))
              (gnosis-test-weak--settle-all base ind)
              (should (equal (make-list 2 '(("lungs" . "* Lungs renamed")))
                             (gnosis-test-weak--views base ind)))
              (gnosis-test-weak--edit-in base "^:ID:       lungs\n")
              (gnosis-test-weak--settle-all base ind)
              (should (equal '(nil nil) (gnosis-test-weak--views base ind)))
              (should (= 0 calls)))
          (with-current-buffer ind (set-buffer-modified-p nil))
          (kill-buffer ind))))))

(defun gnosis-test-weak--disabled-editor (enabled editor)
  "Edit through EDITOR, its indicators off, and check ENABLED's marks.
Assert renaming and deleting an ID stay anchored in ENABLED without SQL."
  (let ((calls 0))
    (cl-letf* ((orig (symbol-function 'sqlite-select))
               ((symbol-function 'sqlite-select)
                (lambda (&rest args) (cl-incf calls) (apply orig args))))
      (gnosis-test-weak--settle-all enabled)
      (with-current-buffer editor (gnosis-weak-mode -1))
      (should (equal '((("valves" . "* Valves"))) (gnosis-test-weak--views enabled)))
      ;; Before idle: the mark already spans the renamed heading line.
      (gnosis-test-weak--edit-in editor "^\\* Valves" " disabled-view")
      (should (equal '((("valves" . "* Valves disabled-view")))
                     (gnosis-test-weak--views enabled)))
      (gnosis-test-weak--settle-all enabled)
      (should (equal '((("valves" . "* Valves disabled-view")))
                     (gnosis-test-weak--views enabled)))
      (gnosis-test-weak--edit-in editor "^:ID: +valves\n")
      (should (equal '(nil) (gnosis-test-weak--views enabled)))
      (gnosis-test-weak--settle-all enabled)
      (should (equal '(nil) (gnosis-test-weak--views enabled)))
      (with-current-buffer editor (should-not (gnosis-weak--overlays)))
      (should (= 0 calls)))))

(ert-deftest gnosis-weak-disabled-view-edits-keep-enabled-views-anchored ()
  (gnosis-test-weak--fixture
    ;; Indicators off in a native clone; the base keeps them.
    (gnosis-test-weak--with-note
      (let ((ind (clone-indirect-buffer "*gnosis-weak-off*" nil)))
        (unwind-protect (gnosis-test-weak--disabled-editor buffer ind)
          (with-current-buffer ind (set-buffer-modified-p nil))
          (kill-buffer ind))))
    ;; Symmetric: indicators off in the base; the clone keeps them.
    (gnosis-test-weak--with-note
      (let ((ind (clone-indirect-buffer "*gnosis-weak-on*" nil)))
        (unwind-protect (gnosis-test-weak--disabled-editor ind buffer)
          (with-current-buffer ind (set-buffer-modified-p nil))
          (kill-buffer ind))))))

(ert-deftest gnosis-weak-change-hook-follows-enabled-views ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--with-note
      (let ((global (lambda () (memq #'gnosis-weak--after-change
                                     (default-value 'after-change-functions))))
            (ind (clone-indirect-buffer "*gnosis-weak-reg*" nil)))
        (should (memq ind gnosis-weak--views))
        (should (memq buffer gnosis-weak--views))
        (should (funcall global))
        (with-current-buffer ind (gnosis-weak-mode -1))
        (should-not (memq ind gnosis-weak--views))
        (should (funcall global))
        (kill-buffer ind)
        (gnosis-weak-mode -1)
        ;; The last enabled view leaves: no global hook remains.
        (should-not gnosis-weak--views)
        (should-not (funcall global))
        (gnosis-weak-mode 1)
        (should (funcall global))
        (set-buffer-modified-p nil)
        (kill-buffer buffer)
        (should-not gnosis-weak--views)
        (should-not (funcall global))))))

(ert-deftest gnosis-weak-narrowed-indirect-subtree-stays-anchored ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--history lung '((failure . 1) (failure . 0)))
    (gnosis-test-weak--with-note
      (let ((base (current-buffer)) ind)
        (goto-char (point-min))
        (re-search-forward "^\\* Valves")
        ;; The native Org subtree view: a cloned, narrowed indirect buffer.
        (org-tree-to-indirect-buffer)
        (setq ind (seq-find (lambda (b) (eq base (buffer-base-buffer b))) (buffer-list)))
        (should ind)
        (unwind-protect
            (let ((restriction (with-current-buffer ind (cons (point-min) (point-max)))))
              (should (with-current-buffer ind (buffer-narrowed-p)))
              (gnosis-test-weak--edit-in ind "^\\* Valves" " renamed")
              (gnosis-test-weak--settle-all base ind)
              (should (equal (make-list 2 '(("lungs" . "* Lungs")
                                            ("valves" . "* Valves renamed")))
                             (gnosis-test-weak--views base ind)))
              ;; A base edit outside the subtree, then an ID deletion inside it.
              (gnosis-test-weak--edit-in base "^\\* Lungs" " renamed")
              (gnosis-test-weak--edit-in ind "^:ID: +valves\n")
              (gnosis-test-weak--settle-all base ind)
              (should (equal (make-list 2 '(("lungs" . "* Lungs renamed")))
                             (gnosis-test-weak--views base ind)))
              ;; Each view keeps its own restriction.
              (should-not (with-current-buffer base (buffer-narrowed-p)))
              (with-current-buffer ind
                (should (buffer-narrowed-p))
                (should (= (car restriction) (point-min)))))
          (with-current-buffer ind (set-buffer-modified-p nil))
          (kill-buffer ind))))))

(ert-deftest gnosis-weak-kill-and-disable-cancel-pending-work ()
  (gnosis-test-weak--fixture
    (gnosis-test-weak--with-note
      (gnosis-weak--schedule 'refresh)
      (let ((timer gnosis-weak--timer) (calls 0))
        (should (memq timer timer-idle-list))
        (gnosis-weak-mode -1)
        (should-not (memq timer timer-idle-list))
        (should-not gnosis-weak--pending)
        (gnosis-weak-mode 1)
        (setq timer gnosis-weak--timer)
        (should (memq timer timer-idle-list))
        (set-buffer-modified-p nil)
        (kill-buffer buffer)
        (should-not (memq timer timer-idle-list))
        ;; Even if the callback still ran, it would read nothing.
        (cl-letf* ((orig (symbol-function 'sqlite-select))
                   ((symbol-function 'sqlite-select)
                    (lambda (&rest args) (cl-incf calls) (apply orig args))))
          (apply (timer--function timer) (timer--args timer)))
        (should (= 0 calls))))))

(defun gnosis-test-weak--populate-scale (themata headings)
  "Link THEMATA reviewed, weak themata across HEADINGS indexed nodes.
Return the matching note text."
  (let ((last (gnosis-test-weak--day -40)))
    (gnosis-sqlite-with-transaction gnosis-db
      (dotimes (i headings)
        (gnosis-test-weak--node (format "node-%d" i) (format "Node %d" i)))
      (dotimes (i themata)
        (let ((id (gnosis-test--add-basic-thema "Question" "Answer" nil nil (1+ i))))
          (gnosis-sqlite-execute
           gnosis-db "UPDATE scheduler_state SET stability = 1.0, difficulty = 9.0,
                      last_review_day = ? WHERE thema_id = ?"
           (list last id))
          (gnosis--insert-into 'thema-links `([,id ,(format "node-%d" (% i headings))])))))
    (mapconcat (lambda (i) (format "* Heading %d\n:PROPERTIES:\n:ID: node-%d\n:END:\nBody\n" i i))
               (number-sequence 0 (1- headings)) "")))

(ert-deftest gnosis-weak-restricted-read-is-driven-by-node-ids ()
  "The restricted read probes links by node, not every thema per node."
  (gnosis-test-with-db
    (let* ((sql (format (gnosis-weak--restricted-sql) "?, ?"))
           (plan (mapcar (lambda (row) (car (last row)))
                         ;; Raw rows: plan details are plain strings.
                         (sqlite-select gnosis-db (concat "EXPLAIN QUERY PLAN " sql)
                                        (gnosis-sqlite--encode-params '("a" "b"))))))
      ;; The outer loop reads thema_links through its dest index.
      (should (string-match-p "\\`SEARCH l USING .*INDEX idx_thema_links_dest (dest=\\?)"
                              (car plan)))
      (should (seq-some (lambda (step) (string-match-p "\\`SEARCH s USING INTEGER PRIMARY KEY" step))
                        plan)))))

(ert-deftest gnosis-weak-refresh-walks-the-buffer-once ()
  "A refresh finds the buffer's nodes once and places from that walk."
  (gnosis-test-weak--fixture
    (gnosis-test-weak--with-note
      (let ((walks 0)
            (walk (symbol-function 'gnosis-weak--buffer-nodes)))
        (cl-letf (((symbol-function 'gnosis-weak--buffer-nodes)
                   (lambda () (cl-incf walks) (funcall walk))))
          (gnosis-weak-refresh))
        (should (= 1 walks))
        (should (equal '((8 . " ● 1")) (gnosis-test-weak--indicators)))
        ;; A note with no node IDs is walked once too.
        (with-temp-buffer
          (org-mode)
          (insert "* No node here\nBody\n")
          (setq walks 0)
          (cl-letf (((symbol-function 'gnosis-weak--buffer-nodes)
                     (lambda () (cl-incf walks) (funcall walk))))
            (gnosis-weak-refresh))
          (should (= 1 walks))
          (should-not (gnosis-test-weak--indicators)))))))

(ert-deftest gnosis-weak-collection-scale-refresh-is-bounded ()
  "5,000 linked themata over 2,000 headings: open defers, refresh is fast."
  ;; Romacs parses Org elements about 5.7 times slower than GNU
  ;; (romacs#112): settling takes about 2 s where GNU takes 0.6 s.
  ;; Expected to fail until that lands; an unexpected pass then says to
  ;; drop this marker.
  :expected-result :failed
  (gnosis-test-with-db
    (let ((text (gnosis-test-weak--populate-scale 5000 2000))
          (calls 0))
      (with-temp-buffer
        (org-mode)
        (insert text)
        (cl-letf* ((orig (symbol-function 'sqlite-select))
                   ((symbol-function 'sqlite-select)
                    (lambda (&rest args) (cl-incf calls) (apply orig args))))
          ;; Enabling (as on note open) does no database work.
          (gnosis-weak-mode 1)
          (should (= 0 calls))
          (should-not (gnosis-test-weak--indicators))
          (let ((start (float-time)))
            (gnosis-test-weak--settle)
            (should (< (- (float-time) start) 1.0)))
          (should (<= calls 2))
          (should (= 2000 (length (gnosis-test-weak--indicators))))
          (setq calls 0)
          (let ((start (float-time)))
            (gnosis-weak-refresh)
            (should (< (- (float-time) start) 1.0)))
          (should (<= calls 2))
          (gnosis-weak-mode -1))))))

(ert-deftest gnosis-weak-mode-only-under-nodes-dir ()
  (gnosis-test-weak--fixture
    (let* ((gnosis-nodes-dir (expand-file-name "notes" gnosis-test--dir))
           (outside (expand-file-name "elsewhere.org" gnosis-test--dir)))
      (make-directory gnosis-nodes-dir t)
      (with-temp-file outside (insert gnosis-test-weak--note))
      (let ((buffer (find-file-noselect outside)))
        (unwind-protect
            (with-current-buffer buffer
              (should-not gnosis-weak-mode)
              (should-not (gnosis-test-weak--indicators)))
          (kill-buffer buffer)))
      (let ((gnosis-weak-indicators nil))
        (gnosis-test-weak--with-note
          (should gnosis-nodes-mode)
          (should-not gnosis-weak-mode))))))

;;; Batches

(ert-deftest gnosis-weak-batch-launch-modes-and-subtree ()
  (gnosis-test-weak--fixture
    ;; Make lungs weak but not due; valves' weak thema is due.
    (gnosis-test-weak--history lung '((failure . 1) (failure . 0)))
    (gnosis-update 'scheduler-state `(= due-day ,(gnosis-test-weak--day 5)) `(= thema-id ,lung))
    (gnosis-update 'scheduler-state `(= due-day ,(gnosis-test-weak--day 0)) `(= thema-id ,weak))
    (gnosis-test-weak--with-note
      (let (launched prompts)
        (cl-letf (((symbol-function 'gnosis-review-loop)
                   (lambda (ids mode target &rest _)
                     (should (consp target))
                     (push (list ids mode) launched)))
                  ((symbol-function 'y-or-n-p)
                   (lambda (prompt) (push prompt prompts) t)))
          (goto-char (point-min))
          (re-search-forward "^Mitral")
          (gnosis-weak-practice)
          (should (equal (list (list weak) 'practice) (car launched)))
          (should (string-match-p "Practise without rescheduling 1 weak thema of Heart:Valves"
                                  (car prompts)))
          ;; Whole-file subtree: root plus all heading nodes, weakest first.
          (goto-char (point-min))
          (gnosis-weak-practice t)
          (should (eq 'practice (cadr (car launched))))
          (should (equal (sort (copy-sequence (car (car launched))) #'<)
                         (sort (list weak lung) #'<)))
          ;; Scheduled review keeps only due weak themata.
          (gnosis-weak-review t)
          (should (equal (list (list weak) 'due) (car launched)))
          ;; Nothing due in lungs: no launch, no prompt.
          (re-search-forward "^\\* Lungs")
          (let ((count (length launched)))
            (gnosis-weak-review)
            (should (= count (length launched)))))))))

(ert-deftest gnosis-weak-launch-refuses-without-node-or-during-review ()
  (gnosis-test-weak--fixture
    (with-temp-buffer
      (should-error (gnosis-weak-practice) :type 'user-error)
      (org-mode)
      (insert "* No ID\n")
      (should-error (gnosis-weak-practice) :type 'user-error))
    (let ((gnosis-review--running t))
      (gnosis-test-weak--with-note
        (goto-char (point-max))
        (should-error (gnosis-weak-practice) :type 'user-error)))))

(provide 'gnosis-test-weak)
;;; gnosis-test-weak.el ends here
