;;; core-bench.el --- core rows timed against GNU 32 -*- lexical-binding: t; -*-


(require 'cl-lib)
(require 'json)
(require 'seq)

(defvar core-bench--reference nil)
(defvar core-bench--hash-sum nil)
(defvar core-bench--live nil)
(defvar core-bench--churn-last nil)

(when (getenv "CORE_BENCH_REFERENCE")
  (setq core-bench--reference
        (with-temp-buffer
          (insert-file-contents (getenv "CORE_BENCH_REFERENCE"))
          (read (current-buffer)))))

(defvar core-bench--all nil
  "Rows as (NAME . FUNCTION), newest first.")

(defmacro core-bench--def (name &rest body)
  "Define bench row NAME running BODY."
  (declare (indent 1))
  (let ((function (intern (format "core-bench-row-%s" name))))
    `(progn
       (defun ,function () ,@body)
       (push (cons ',name ',function) core-bench--all))))

(defun core-bench--xdisp ()
  "Insert xdisp.c into the current buffer."
  (let ((file (getenv "CORE_BENCH_XDISP")))
    (unless (and file (file-readable-p file))
      (error "CORE_BENCH_XDISP is not a readable file: %S" file))
    (insert-file-contents file)))

;;;; Eval and funcall

(defun core-bench--fib (n)
  (if (< n 2) n (+ (core-bench--fib (- n 1)) (core-bench--fib (- n 2)))))

(core-bench--def eval-fib-interp
  (eval '(progn
           (defun core-bench--fibi (n)
             (if (< n 2) n (+ (core-bench--fibi (- n 1)) (core-bench--fibi (- n 2)))))
           (core-bench--fibi 22))
        t))

(core-bench--def bc-fib-recursive
  (core-bench--fib 27))

(defun core-bench--loop (n)
  (let ((i 0) (s 0)) (while (< i n) (setq s (+ s i) i (1+ i))) s))

(core-bench--def bc-while-loop
  (core-bench--loop 20000000))

(defvar core-bench--special 0)

(defun core-bench--dyn (n)
  (let ((i 0)) (while (< i n) (let ((core-bench--special i)) (setq i (1+ i)))) i))

(core-bench--def bc-specbind-loop
  (core-bench--dyn 5000000))

(defun core-bench--funcall (n)
  (let ((f (lambda (x) (1+ x))) (i 0)) (while (< i n) (setq i (funcall f i))) i))

(core-bench--def bc-funcall-closure
  (core-bench--funcall 10000000))

(core-bench--def bc-mapcar-list
  (let ((l (number-sequence 1 1000)) last)
    (dotimes (_ 2000) (setq last (mapcar #'1+ l))) last))

;;;; V0 VM probes. All are Tier A, including the three anti-gaming rows.
;; Keep the dynamic callee across a named call boundary: neither compiler may
;; replace the argument with a literal closure at this call site.
(defun core-bench--funcall-dynamic (f n)
  (let ((i 0)) (while (< i n) (setq i (funcall f i))) i))

(core-bench--def bc-funcall-dynamic
  (core-bench--funcall-dynamic (lambda (x) (1+ x)) 10000000))

(core-bench--def bc-seq-filter
  (let ((input (number-sequence 1 1000)) last)
    (dotimes (_ 2000) (setq last (seq-filter (lambda (x) (> x 500)) input)))
    last))

(core-bench--def bc-make-closure-1e6
  (let ((i 0) last)
    (while (< i 1000000)
      (let ((capture i)) (setq last (lambda (x) (+ capture x))))
      (setq i (1+ i)))
    (list i (funcall last 1))))

(defun core-bench--handler-step (i)
  (if (= (% i 1000) 0) (signal 'error nil) (1+ i)))

(core-bench--def bc-condition-case-loop
  (let ((i 0) (sum 0))
    (while (< i 1000000)
      (setq sum (+ sum (condition-case nil (core-bench--handler-step i) (error -1)))
            i (1+ i)))
    sum))

(core-bench--def bc-catch-throw-1e5
  (let ((i 0) (sum 0))
    (while (< i 100000)
      (setq sum (+ sum (catch 'core-bench-tag (throw 'core-bench-tag i)))
            i (1+ i)))
    sum))

(defvar core-bench--global 7)
(core-bench--def bc-varref-global
  (let ((i 0) (sum 0))
    (while (< i 10000000)
      (setq sum (+ sum core-bench--global) i (1+ i)))
    sum))

(defun core-bench--optional-rest (a &optional b &rest rest)
  (+ a (or b 0) (if rest (car rest) 0)))

(core-bench--def bc-call-optional-rest
  (let ((i 0) (sum 0))
    (while (< i 1000000)
      (setq sum (+ sum (core-bench--optional-rest i)
                   (core-bench--optional-rest i 2)
                   (core-bench--optional-rest i 2 3 4))
            i (1+ i)))
    sum))

;;;; V0b amendment rows (Tier A, native FLOOR 1.20 like every Tier A row).
;; Amendment 2/3: lexical capture of a MUTATED variable (cconv's cons-cell box,
;; Lua's upvalue). The closure is the RESULT of a call held in a variable, so
;; no compiler can treat it as a literal lambda at the call site.
(defun core-bench--make-counter (n)
  (lambda () (setq n (1+ n))))

(core-bench--def bc-closure-counter
  (let ((counter (core-bench--make-counter 7)) (i 0) (n 0))
    (while (< i 10000000) (setq n (funcall counter) i (1+ i)))
    n))

;; Amendment 4/5: errors are data and arrive with a backtrace ALWAYS. A
;; handler-bind handler captures `backtrace-frames' at signal time, 9 frames
;; of the recursive helper (depth 8 down to 0) per error. The GNU floor for
;; the backtrace-always error path the lazy-reconstruction design must meet.
(define-error 'core-bench-signal "core-bench signal")

(defun core-bench--bt-recur (d)
  (if (= d 0) (signal 'core-bench-signal (list d))
    (1+ (core-bench--bt-recur (1- d)))))

(defun core-bench--bt-count (frames)
  (let ((k 0))
    (dolist (frame frames)
      (when (eq (cadr frame) 'core-bench--bt-recur) (setq k (1+ k))))
    k))

(core-bench--def bc-error-backtrace-1e4
  (let ((i 0) (total 0))
    (while (< i 10000)
      (condition-case nil
          (handler-bind ((core-bench-signal
                          (lambda (_err)
                            (setq total (+ total (core-bench--bt-count (backtrace-frames)))))))
            (core-bench--bt-recur 8))
        (core-bench-signal nil))
      (setq i (1+ i)))
    total))

(core-bench--def bc-hash-puthash-gethash
  (let ((h (make-hash-table :test 'equal)))
    (dotimes (i 300000) (puthash (number-to-string i) i h))
    (let ((sum 0))
      (dotimes (i 300000) (setq sum (+ sum (gethash (number-to-string i) h))))
      (setq core-bench--hash-sum sum))
    h))

(core-bench--def bc-intern-1e5
  (let ((obarray (make-vector 1511 0)))
    (dotimes (i 100000) (intern (format "core-bench-sym-%d" i) obarray))
    obarray))

(defvar core-bench--names nil)
(defvar core-bench--ob nil)

(defun core-bench--prepare (name)
  "Prepare diagnostic inputs outside timing, before the full GC."
  (when (assoc name core-bench--props-rows) (core-bench--props-prepare name))
  (when (string-prefix-p "gnosis-" name)
    (core-bench--gnosis-prepare name))
  (when (member name '("diag-intern-fresh-1e5" "diag-intern-hit-1e5"))
    (unless core-bench--names
      (setq core-bench--names
            (cl-loop for i below 100000 collect (format "core-bench-sym-%d" i))))
    (setq core-bench--ob (obarray-make))
    (when (equal name "diag-intern-hit-1e5")
      (dolist (s core-bench--names) (intern s core-bench--ob)))))

(defun core-bench--check (name result)
  "Check completed work outside the measured region; fail closed for new rows."
  (let ((expected
         (cond
          ((equal name "eval-fib-interp") 17711)
          ((equal name "bc-fib-recursive") 196418)
          ((equal name "bc-while-loop") 199999990000000)
          ((equal name "bc-specbind-loop")
           (unless (= core-bench--special 0) (error "Leaked dynamic binding")) 5000000)
          ((member name '("bc-funcall-closure" "bc-funcall-dynamic")) 10000000)
          ((equal name "bc-seq-filter") (number-sequence 501 1000))
          ((equal name "bc-make-closure-1e6") '(1000000 1000000))
          ((equal name "bc-condition-case-loop") 499500998000)
          ((equal name "bc-catch-throw-1e5") 4999950000)
          ((equal name "bc-varref-global") 70000000)
          ((equal name "bc-call-optional-rest") 1500005500000)
          ((equal name "bc-closure-counter") 10000007)
          ((equal name "bc-error-backtrace-1e4") 90000)
          ((equal name "bc-mapcar-list") (number-sequence 2 1001))
          ((equal name "gc-cons-churn") (list 2000000 (make-list 10 1999999)))
          ((equal name "buf-insert-end-1e6")
           (apply #'concat (make-list 1000 (apply #'concat (make-list 1000 "ab")))))
          ((equal name "regex-string-match-1e5") 100000)
          ((equal name "diag-sqlite-small-select-1e4") 49995000)
          ((equal name "diag-format-1e5") "core-bench-sym-99999")
          (t :special))))
    (cond
     ((not (eq expected :special))
      (unless (equal result expected) (error "Wrong answer for %s" name)))
     ((equal name "bc-hash-puthash-gethash")
      (unless (and (= (hash-table-count result) 300000)
                   (= core-bench--hash-sum 44999850000)
                   (= (gethash "299999" result) 299999)) (error "Wrong hash result")))
     ((member name '("diag-intern-fresh-1e5" "diag-intern-hit-1e5"))
      (unless (and (listp result) (= (length result) 100000)
                   (= (cl-loop for s in core-bench--names for symbol in result
                               count (eq symbol (intern-soft s core-bench--ob))) 100000))
        (error "Wrong intern lookup identities")))
     ((equal name "bc-intern-1e5")
      (let ((n 0))
        (mapatoms (lambda (_) (setq n (1+ n))) result)
        (unless (and (= n 100000) (intern-soft "core-bench-sym-99999" result))
          (error "Wrong intern result"))))
     ((equal name "gc-full-pause-live")
      (unless (and result (= (length (nth 0 core-bench--live)) 1000000)
                   (= (length (nth 1 core-bench--live)) 100000)
                   (= (length (nth 2 core-bench--live)) 10000)
                   (cl-every (lambda (v) (and (vectorp v) (= (length v) 10)))
                             (nth 2 core-bench--live))
                   (= (hash-table-count
                       (let ((h (make-hash-table :test 'eq)))
                         (dolist (v (nth 2 core-bench--live)) (puthash v t h)) h)) 10000))
        (error "Wrong live set")))
     ((equal name "buf-markers-1e3-edits")
      (unless (equal result (cl-loop for i from 1 to 1000
                                    for p = (* i 200)
                                    collect (+ p (min 20000 (1+ (/ (- p 2) 6))))))
        (error "Wrong marker positions")))
     ((assoc name core-bench--props-expected)
      (unless (equal (core-bench--props-answer name)
                     (cdr (assoc name core-bench--props-expected)))
        (error "GNU P0 oracle mismatch for %s: %S expected %S" name
               (core-bench--props-answer name) (cdr (assoc name core-bench--props-expected)))))
     ((assoc name core-bench--reference)
      (unless (equal (core-bench--answer name result) (cdr (assoc name core-bench--reference)))
        (error "Wrong fixture answer for %s" name)))
     (t (error "No answer check for %s" name)))))

(core-bench--def diag-sqlite-small-select-1e4
  (unless (sqlite-available-p) (error "SQLite is unavailable"))
  (let ((db (sqlite-open)) (sum 0))
    (unwind-protect
        (progn
          (sqlite-execute db "create table bench(id integer primary key)")
          (sqlite-execute db "with recursive n(id) as (values(0) union all select id+1 from n where id<9999) insert into bench select id from n")
          (dotimes (id 10000)
            (setq sum (+ sum (caar (sqlite-select db "select id from bench where id = ?" (list id))))))
          sum)
      (sqlite-close db))))

(core-bench--def diag-format-1e5
  (let (last)
    (dotimes (i 100000) (setq last (format "core-bench-sym-%d" i)))
    last))

(core-bench--def diag-intern-fresh-1e5
  (mapcar (lambda (s) (intern s core-bench--ob)) core-bench--names))

(core-bench--def diag-intern-hit-1e5
  (mapcar (lambda (s) (intern s core-bench--ob)) core-bench--names))

;;;; GC

(core-bench--def gc-cons-churn
  (let ((i 0))
    (while (< i 2000000)
      (setq core-bench--churn-last (make-list 10 i) i (1+ i)))
    (list i core-bench--churn-last)))


(core-bench--def gc-full-pause-live
  (unless core-bench--live
    (setq core-bench--live
          (list (make-list 1000000 'x)
                (mapcar #'number-to-string (number-sequence 1 100000))
                (cl-loop repeat 10000 collect (make-vector 10 nil)))))
  (garbage-collect))

;;;; Buffer text

(core-bench--def buf-insert-end-1e6
  (with-temp-buffer (dotimes (_ 1000000) (insert "ab")) (buffer-string)))

(core-bench--def buf-insert-random-1e5
  (with-temp-buffer
    (insert (make-string 1000000 ?x))
    (let ((r 7))
      (dotimes (_ 100000)
        (setq r (% (+ (* r 1103515245) 12345) 1000000))
        (goto-char (1+ r)) (insert "yz") (delete-char -1)))
    (buffer-string)))

(core-bench--def buf-markers-1e3-edits
  (with-temp-buffer
    (insert (make-string 200000 ?x))
    (let ((ms (cl-loop for i from 1 to 1000 collect (copy-marker (* i 200)))))
      (dotimes (i 20000) (goto-char (1+ (* 7 i))) (insert "q"))
      (mapcar #'marker-position ms))))

(core-bench--def buf-textprop-1e5
  (with-temp-buffer
    (insert (make-string 400000 ?x))
    (dotimes (i 100000) (put-text-property (1+ (* 4 i)) (+ 3 (* 4 i)) 'face 'bold))
    (buffer-string)))

(core-bench--def buf-utf8-decode-xdisp
  (with-temp-buffer (core-bench--xdisp) (buffer-string)))

(core-bench--def buf-forward-line-xdisp
  (with-temp-buffer
    (core-bench--xdisp)
    (let (results)
      (dotimes (_ 10)
        (goto-char (point-min))
        (let ((lines 0) (shortage 0))
          (while (= 0 (setq shortage (forward-line 1)))
            (setq lines (1+ lines)))
          (push (list lines shortage (point)) results)))
      (nreverse results))))

(core-bench--def buf-position-xdisp
  (with-temp-buffer
    (core-bench--xdisp)
    (let ((size (buffer-size)) (r 7) (sum 0))
      (dotimes (_ 2000)
        (setq r (% (+ (* r 1103515245) 12345) size))
        (goto-char (1+ r))
        (setq sum (+ sum (line-number-at-pos)))) sum)))

;;;; Search and regex

(core-bench--def search-literal-xdisp
  (with-temp-buffer
    (core-bench--xdisp)
    (let ((n 0))
      (dotimes (_ 20) (goto-char (point-min))
        (while (search-forward "glyph_row" nil t) (setq n (1+ n)))) n)))

(core-bench--def regex-fontlock-xdisp
  (with-temp-buffer
    (core-bench--xdisp)
    (let (counts)
    (dolist (re '("^\\([a-z_]+\\) (" "\\_<\\(?:if\\|while\\|for\\|return\\)\\_>"
                  "\"\\(?:[^\"\\\\]\\|\\\\.\\)*\"" "/\\*\\(?:.\\|\n\\)*?\\*/"
                  "#\\s-*\\(?:define\\|include\\|ifdef\\|endif\\)"
                  "\\b[A-Z_][A-Z0-9_]\\{3,\\}\\b"))
      (goto-char (point-min))
      (let ((n 0))
        (while (re-search-forward re nil t) (setq n (1+ n)))
        (push n counts))) (nreverse counts))))

(core-bench--def regex-string-match-1e5
  (let ((n 0))
    (dotimes (i 100000)
      (when (string-match "\\`\\([a-z]+\\)-\\([0-9]+\\)\\'" (format "abc-%d" i))
        (setq n (1+ n)))) n))

(core-bench--def syntax-ppss-eob-xdisp
  (with-temp-buffer
    (core-bench--xdisp)
    (c-mode)
    (syntax-ppss (point-max))))

;;;; Chat streaming

(core-bench--def chat-stream-20k
  (with-temp-buffer
    (insert "\n> ")
    (let ((m (copy-marker 1 t)) (inhibit-read-only t) searches)
      (dotimes (msg 2000)
        (save-excursion
          (goto-char m)
          (insert (propertize (format "\n* assistant %d\n" msg) 'face 'bold 'read-only t)))
        (dotimes (d 10)
          (save-excursion
            (goto-char m)
            (insert (propertize (format "delta %d of message %d with `code` and words. " d msg)
                                'face 'default 'msg msg)))
          (save-excursion
            (goto-char m)
            (push (when (re-search-backward "`\\([^`]+\\)`" (max 1 (- m 400)) t)
                    (list (match-beginning 0) (match-end 0) (match-string 1)))
                  searches))))
      (list (buffer-string) (nreverse searches)))))

(core-bench--def rss-after-workload
  (require 'org)
  (unless (featurep 'org) (error "Org not loaded"))
  (with-current-buffer (get-buffer-create " *core-bench-workload*")
    (erase-buffer)
    (core-bench--xdisp)
    (c-mode)
    (font-lock-ensure)
    (garbage-collect)
    (list (buffer-size) (not (null (text-property-not-all (point-min) (point-max) 'face nil))))))

;;;; Gnosis dashboard: tabulated-list over SQLite
;;
;; Mirrors the Gnosis themata dashboard (gnosis 49ff111: gnosis-sqlite.el
;; decoding, gnosis-dashboard.el entry formatting/refresh/printer and
;; gnosis-tl.el bulk rendering) over the synthetic database written by
;; bench/gnosis-fixture.py (CORE_BENCH_GNOSIS_DB, 90K themata, ~2.3M tags).
;; Inputs of sub-rows are prepared outside timing; all databases are opened
;; read-only.

(require 'tabulated-list)

(defconst core-bench--gnosis-sql
  "SELECT themata.id, themata.keimenon, themata.hypothesis, themata.answer, (SELECT '(' || GROUP_CONCAT(tag, ' ') || ')' FROM thema_tag WHERE thema_id = themata.id) AS tags, themata.type, scheduler_state.suspended FROM themata JOIN scheduler_state ON themata.id = scheduler_state.thema_id WHERE themata.id IN (%s)")

(defconst core-bench--gnosis-width 180
  "Window width used for the column layout (batch windows are 80 wide).")

(defvar core-bench--gnosis-db nil)
(defvar core-bench--gnosis-max-vars nil)
(defvar core-bench--gnosis-ids nil)
(defvar core-bench--gnosis-raw nil)
(defvar core-bench--gnosis-entries nil)
(defvar core-bench--gnosis-buffer nil)

(defun core-bench--gnosis-open ()
  "Return the read-only fixture handle, opening it once."
  (or core-bench--gnosis-db
      (let ((file (getenv "CORE_BENCH_GNOSIS_DB")))
        (unless (and file (file-readable-p file))
          (error "CORE_BENCH_GNOSIS_DB is not a readable file: %S" file))
        (let ((db (sqlite-open file t)))
          (sqlite-execute db "PRAGMA foreign_keys = ON")
          (sqlite-execute db "PRAGMA busy_timeout = 5000")
          (setq core-bench--gnosis-db db)))))

;; gnosis-sqlite.el
(defun core-bench--gnosis-max-vars (db)
  (or core-bench--gnosis-max-vars
      (setq core-bench--gnosis-max-vars
            (cl-loop for (opt) in (sqlite-select db "PRAGMA compile_options")
                     when (string-match "MAX_VARIABLE_NUMBER=\\([0-9]+\\)" opt)
                     return (string-to-number (match-string 1 opt))
                     finally return 999))))

(defun core-bench--gnosis-decode (value)
  (cond ((null value) nil)
        ((numberp value) value)
        ((and (stringp value) (string-empty-p value)) "")
        ((stringp value)
         (condition-case nil (car (read-from-string value)) (error value)))
        (t value)))

(defun core-bench--gnosis-decode-rows (rows)
  (mapcar (lambda (row) (mapcar #'core-bench--gnosis-decode row)) rows))

(defun core-bench--gnosis-select-raw (db ids)
  "The batched `gnosis-sqlite-select-batch' without decoding."
  (let ((batch (core-bench--gnosis-max-vars db)))
    (cl-loop for chunk in (seq-partition ids batch)
             append (sqlite-select
                     db (format core-bench--gnosis-sql
                                (mapconcat (lambda (_) "?") chunk ", "))
                     chunk))))

(defun core-bench--gnosis-all-ids (db)
  "Unfiltered `gnosis-collect-thema-ids' (all ids, decoded)."
  (mapcar #'car (core-bench--gnosis-decode-rows (sqlite-select db "SELECT id FROM themata"))))

;; gnosis-dashboard.el
(defun core-bench--gnosis-format-entry (row)
  (let* ((fields (cl-loop for item in (cdr row)
                          for formatted =
                          (replace-regexp-in-string
                           "\n" " "
                           (if (listp item)
                               (mapconcat (lambda (x) (format "%s" x)) item ",")
                             (format "%s" item)))
                          collect (if (listp item)
                                      formatted
                                    (replace-regexp-in-string
                                     "\\[\\[id:[^]]+\\]\\[\\(.*?\\)\\]\\]"
                                     "\\1" formatted)))))
    (list (car row)
          (vconcat (append (butlast fields)
                           (list (if (equal (car (last fields)) "1") "Yes" "No")))))))

(defun core-bench--gnosis-entries (ids raw)
  "Decode RAW rows; order formatted entries by IDS.
Mirrors `gnosis-dashboard--output-themata'."
  (let ((entries (make-hash-table :test 'equal)))
    (dolist (row (core-bench--gnosis-decode-rows raw))
      (puthash (car row) (core-bench--gnosis-format-entry row) entries))
    (cl-loop for id in ids for entry = (gethash id entries) when entry collect entry)))

(defun core-bench--gnosis-output-themata (ids)
  (core-bench--gnosis-entries ids (core-bench--gnosis-select-raw (core-bench--gnosis-open) ids)))

(defun core-bench--gnosis-format (width)
  (let ((avail (- width 7)))
    `[("Keimenon"   ,(max 10 (/ (* avail 28) 100)) t)
      ("Hypothesis" ,(max 8  (/ (* avail 16) 100)) t)
      ("Answer"     ,(max 8  (/ (* avail 16) 100)) t)
      ("Tags"       ,(max 8  (/ (* avail 18) 100)) t)
      ("Type"       ,(max 5  (/ (* avail 10) 100)) t)
      ("Suspend"    ,(max 3  (/ (* avail 8) 100)) t)]))

(defvar-local core-bench--gnosis-current-ids nil)
(defvar-local core-bench--gnosis-selected nil)

(defun core-bench--gnosis-print-entry (id columns)
  "`gnosis-dashboard--print-entry': native entry printer plus mark projection."
  (let ((beg (point)))
    (tabulated-list-print-entry id columns)
    (remove-overlays beg (point) 'gnosis-mark t)
    (when (member id core-bench--gnosis-selected)
      (let ((o (make-overlay beg (line-end-position 0))))
        (overlay-put o 'face 'highlight)
        (overlay-put o 'gnosis-mark t)))))

(defun core-bench--gnosis-refresh ()
  "`gnosis-dashboard--refresh-entries' for the themata view."
  (let ((entries (core-bench--gnosis-output-themata core-bench--gnosis-current-ids)))
    (setq tabulated-list-entries entries
          tabulated-list--original-order (make-hash-table :test 'eq))
    (cl-loop for entry in entries for rank from 0
             do (puthash entry rank tabulated-list--original-order))
    (setq core-bench--gnosis-current-ids (mapcar #'car entries))))

(define-derived-mode core-bench-gnosis-mode tabulated-list-mode "CB Gnosis"
  "Themata view set up like `gnosis-dashboard--common-setup'."
  (add-hook 'tabulated-list-revert-hook #'core-bench--gnosis-refresh nil t)
  (setq-local header-line-format nil)
  (setq-local tabulated-list-printer #'core-bench--gnosis-print-entry)
  (setq-local truncate-string-ellipsis "...")
  (setq tabulated-list-padding 2
        tabulated-list-entries nil
        tabulated-list-sort-key nil))

;; gnosis-tl.el
(defun core-bench--gtl-specs (format)
  (let ((len (length format)))
    (cl-loop for i below len
             for col = (aref format i)
             for props = (nthcdr 3 col)
             collect (list :name (nth 0 col) :width (nth 1 col)
                           :pad-right (if (= i (1- len)) 0 (or (plist-get props :pad-right) 1))
                           :right-align (plist-get props :right-align)))))

(defun core-bench--gtl-pad (text width pad-right right-align)
  (let* ((truncated (if (> (string-width text) width)
                        (truncate-string-to-width text width nil nil "...")
                      text))
         (padding (max 0 (- width (string-width truncated)))))
    (if right-align
        (concat (make-string padding ?\s) truncated (make-string pad-right ?\s))
      (concat truncated (make-string (+ padding pad-right) ?\s)))))

(defun core-bench--gtl-render (entries format padding)
  "`gnosis-tl--render-into-buffer'."
  (let* ((specs (core-bench--gtl-specs format))
         (n-cols (length specs))
         (last-idx (1- n-cols))
         (pad-str (when (> padding 0) (make-string padding ?\s)))
         (widths (vconcat (mapcar (lambda (s) (plist-get s :width)) specs)))
         (pad-rights (vconcat (cl-loop for s in specs for i from 0
                                       collect (if (= i last-idx) 0 (plist-get s :pad-right)))))
         (right-aligns (vconcat (mapcar (lambda (s) (plist-get s :right-align)) specs)))
         (fmt-strs (vconcat (cl-loop for i below n-cols
                                     collect (format "%%-%ds" (+ (aref widths i)
                                                                 (aref pad-rights i)))))))
    (dolist (entry entries)
      (let ((id (car entry)) (cols (cadr entry)) (beg (point)) (i 0))
        (when pad-str (insert pad-str))
        (while (< i n-cols)
          (let* ((raw (or (aref cols i) ""))
                 (text (if (stringp raw) raw (format "%s" raw)))
                 (len (length text))
                 (ascii-p (not (string-match-p "[^ -~]" text)))
                 (width (aref widths i))
                 (start (point)))
            (if (= i last-idx)
                (insert (if (and ascii-p (<= len width))
                            text
                          (let ((sw (if ascii-p len (string-width text))))
                            (if (> sw width)
                                (truncate-string-to-width text width nil nil "...")
                              text))))
              (if (and ascii-p (<= len width) (not (aref right-aligns i)))
                  (insert (format (aref fmt-strs i) text))
                (insert (core-bench--gtl-pad text width (aref pad-rights i)
                                             (aref right-aligns i)))))
            (put-text-property start (point) 'tabulated-list-column-name
                               (car (aref format i))))
          (setq i (1+ i)))
        (insert ?\n)
        (add-text-properties beg (point) (list 'tabulated-list-id id
                                               'tabulated-list-entry cols))))))

(defun core-bench--gtl-print ()
  "`gnosis-tl-print' without a sort key or remembered position."
  (let ((inhibit-read-only t) (inhibit-modification-hooks t))
    (erase-buffer)
    (core-bench--gtl-render tabulated-list-entries tabulated-list-format
                            (or tabulated-list-padding 0)))
  (remove-overlays (point-min) (point-max) 'gnosis-mark t)
  (set-buffer-modified-p nil)
  (goto-char (point-min)))

(defun core-bench--gnosis-view (ids entries)
  "Show ENTRIES like `gnosis-dashboard-output-themata', fully settled."
  (with-current-buffer (get-buffer-create "*CB Gnosis*")
    (core-bench-gnosis-mode)
    (setq tabulated-list-format (core-bench--gnosis-format core-bench--gnosis-width))
    (tabulated-list-init-header)
    (setq core-bench--gnosis-current-ids ids
          tabulated-list--original-order (make-hash-table :test 'eq))
    (cl-loop for entry in entries for rank from 0
             do (puthash entry rank tabulated-list--original-order))
    (setq tabulated-list-entries entries)
    (core-bench--gtl-print)
    (current-buffer)))

(defun core-bench--gnosis-prepare (name)
  "Prepare Gnosis inputs for row NAME outside timing."
  (let ((db (core-bench--gnosis-open)))
    (unless core-bench--gnosis-ids
      (setq core-bench--gnosis-ids (core-bench--gnosis-all-ids db)))
    (when (and (member name '("gnosis-dash-entries"))
               (null core-bench--gnosis-raw))
      (setq core-bench--gnosis-raw (core-bench--gnosis-select-raw db core-bench--gnosis-ids)))
    (when (and (member name '("gnosis-dash-print" "gnosis-dash-sort" "gnosis-dash-refresh"
                              "gnosis-dash-nav"))
               (null core-bench--gnosis-entries))
      (setq core-bench--gnosis-entries
            (core-bench--gnosis-output-themata core-bench--gnosis-ids)))
    (when (member name '("gnosis-dash-sort" "gnosis-dash-refresh" "gnosis-dash-nav"))
      ;; A fresh, unsorted, settled view each repetition.
      (setq core-bench--gnosis-buffer
            (core-bench--gnosis-view core-bench--gnosis-ids
                                     (copy-sequence core-bench--gnosis-entries)))
      (with-current-buffer core-bench--gnosis-buffer
        (goto-char (point-min))
        (forward-line 100)))))

(core-bench--def gnosis-dashboard-90k
  ;; Open the themata view: collect ids, query, decode, format, render all rows.
  (let* ((ids (core-bench--gnosis-all-ids (core-bench--gnosis-open)))
         (entries (core-bench--gnosis-output-themata ids)))
    (core-bench--gnosis-view ids entries)))

(core-bench--def gnosis-dash-query
  (core-bench--gnosis-select-raw (core-bench--gnosis-open) core-bench--gnosis-ids))

(core-bench--def gnosis-dash-entries
  (core-bench--gnosis-entries core-bench--gnosis-ids core-bench--gnosis-raw))

(core-bench--def gnosis-dash-print
  (core-bench--gnosis-view core-bench--gnosis-ids core-bench--gnosis-entries))

(core-bench--def gnosis-dash-sort
  ;; Header click / S on Keimenon: native `tabulated-list-print' via the printer.
  (with-current-buffer core-bench--gnosis-buffer
    (tabulated-list-sort 0)
    (current-buffer)))

(core-bench--def gnosis-dash-refresh
  ;; g: refresh hook re-reads every row, then native `tabulated-list-print'.
  (with-current-buffer core-bench--gnosis-buffer
    (revert-buffer)
    (current-buffer)))

(core-bench--def gnosis-dash-nav
  ;; Headless scroll proxy: page through the view reading ids and entries.
  (with-current-buffer core-bench--gnosis-buffer
    (goto-char (point-min))
    (let ((ids nil) (n 0))
      (while (not (eobp))
        (push (tabulated-list-get-id) ids)
        (setq n (+ n (length (tabulated-list-get-entry))))
        (forward-line 45))
      (goto-char (point-max))
      (forward-line -1)
      (list n (length ids) (car ids) (tabulated-list-get-id)
            (line-number-at-pos)))))

(defun core-bench--gnosis-digest (result)
  "Reduce a Gnosis row RESULT to a portable answer."
  (let ((print-length nil) (print-level nil)
        (hash (lambda (o) (secure-hash 'sha1 (encode-coding-string
                                               (if (stringp o) o (prin1-to-string o))
                                               'utf-8-unix)))))
    (if (bufferp result)
        (with-current-buffer result
          (let (ids cols)
            (save-excursion
              (goto-char (point-min))
              (while (not (eobp))
                (push (tabulated-list-get-id) ids)
                (push (list (get-text-property (+ (point) 3) 'tabulated-list-column-name)
                            (get-text-property (max (point-min) (1- (line-end-position)))
                                               'tabulated-list-column-name))
                      cols)
                (forward-line 1)))
            (list (buffer-size) (funcall hash (buffer-substring-no-properties (point-min) (point-max)))
                  (length ids) (funcall hash ids) (funcall hash cols)
                  tabulated-list-sort-key (line-number-at-pos)
                  (aref (tabulated-list-get-entry) 0))))
      (list (length result) (funcall hash result)))))


(defvar core-bench--props-buffer nil)
(defvar core-bench--props-result nil)
(defconst core-bench--props-expected '(("buf-prop-tablist-print" . (1080000 "612a9a53043c"))
    ("buf-prop-tablist-sort" . (1080000 "de4194adc8df"))
    ("buf-prop-insert-propertized" . (200000 "4659a0cb682e"))
    ("buf-prop-insert-then-add" . (100000 "23d2767a3810"))
    ("buf-prop-fontlock-put" . (200000 "93d12258fd38"))
    ("buf-prop-fontlock-refont" . (160001 "9f2c89dc4fd6"))
    ("buf-prop-fontlock-elisp" . (55911 "708463a6e123"))
    ("buf-prop-get-prop-rand" . 500000)
    ("buf-prop-next-single-scan" . 1999990)
    ("buf-prop-props-at-scan" . 200000)
    ("buf-markers-1e4-edits" . 5047966308)
    ("buf-markers-1e3-typing" . 539450000)
    ("buf-markers-make-1e5" . 100000)))
(defconst core-bench--props-rows '(("buf-prop-tablist-print" . "tablist-print")
    ("buf-prop-tablist-sort" . "tablist-sort")
    ("buf-prop-insert-propertized" . "insert-propertized")
    ("buf-prop-insert-then-add" . "insert-then-add")
    ("buf-prop-fontlock-put" . "fontlock-put")
    ("buf-prop-fontlock-refont" . "fontlock-refont")
    ("buf-prop-fontlock-elisp" . "fontlock-elisp")
    ("buf-prop-get-prop-rand" . "get-prop-rand")
    ("buf-prop-next-single-scan" . "next-single-scan")
    ("buf-prop-props-at-scan" . "props-at-scan")
    ("buf-markers-1e4-edits" . "markers-1e4-edits")
    ("buf-markers-1e3-typing" . "markers-1e3-typing")
    ("buf-markers-make-1e5" . "markers-make-1e5")))
(defvar core-bench--props-faces [bold italic font-lock-keyword-face font-lock-string-face font-lock-comment-face])
(defvar core-bench--props-keep nil)

(defun core-bench--props-digest ()
  "Canonical interval digest of current buffer (outside timing)."
  (let ((p (point-min)) acc)
    (while (< p (point-max))
      (let ((e (next-property-change p nil (point-max))))
        (push (list p e (text-properties-at p)) acc)
        (setq p e)))
    (list (length acc) (substring (secure-hash 'sha1 (prin1-to-string (nreverse acc))) 0 12))))

(defun core-bench--props-tablist-buffer (n)
  (let ((b (generate-new-buffer "tl")))
    (with-current-buffer b
      (tabulated-list-mode)
      (setq tabulated-list-format [("Keimenon" 50 t) ("Tags" 30 t) ("Due" 12 t) ("Id" 8 t)])
      (setq tabulated-list-padding 1)
      (setq tabulated-list-sort-key nil)
      (tabulated-list-init-header)
      (setq tabulated-list-entries
            (cl-loop for i below n
                     collect (list i (vector (format "entry %x about thing number %d" (* i 2654435761) i)
                                             (format "tag%d tag%d" (% i 17) (% i 5))
                                             (format "2026-%02d-%02d" (1+ (% i 12)) (1+ (% i 28)))
                                             (number-to-string i))))))
    b))

(defun core-bench--props-prop-buffer ()
  "400k chars with 100k face runs (setup for read rows)."
  (let ((b (generate-new-buffer "pb")))
    (with-current-buffer b
      (insert (make-string 400000 ?x))
      (dotimes (i 100000)
        (put-text-property (1+ (* 4 i)) (+ 3 (* 4 i)) 'face (aref core-bench--props-faces (% i 5)))))
    b))

(defun core-bench--props-prepare-input (row)
  (pcase row
    ((or "tablist-print" "tablist-sort") (core-bench--props-tablist-buffer 90000))
    ((or "get-prop-rand" "next-single-scan" "props-at-scan" "fontlock-refont") (core-bench--props-prop-buffer))
    ("fontlock-elisp"
     (let ((b (generate-new-buffer "fl")))
       (with-current-buffer b
         (dotimes (_ 3)
           (insert-file-contents (or (getenv "CORE_BENCH_SIMPLE")
                                     (error "Set CORE_BENCH_SIMPLE"))))
         (delay-mode-hooks (emacs-lisp-mode))
         (setq-local font-lock-support-mode nil)
         (font-lock-mode 1))
       b))
    ((or "markers-1e4-edits" "markers-1e3-typing")
     (let ((b (generate-new-buffer "mk")))
       (with-current-buffer b (insert (make-string 1000000 ?x)))
       b))
    (_ (generate-new-buffer "e"))))

(defun core-bench--props-body (row b)
  (with-current-buffer b
    (pcase row
      ("tablist-print" (tabulated-list-print) (buffer-size))
      ("tablist-sort" (setq tabulated-list-sort-key (cons "Keimenon" nil))
       (tabulated-list-print) (buffer-size))
      ("insert-propertized"
       (dotimes (i 200000)
         (insert (propertize "word " 'face (aref core-bench--props-faces (% i 5)) 'help-echo "x")))
       (buffer-size))
      ("insert-then-add"
       (dotimes (i 100000)
         (let ((p (point)))
           (insert "characterized line\n")
           (add-text-properties p (point) (list 'tabulated-list-id i 'tabulated-list-entry core-bench--props-faces))))
       (buffer-size))
      ("fontlock-put"
       (insert (make-string 400000 ?x))
       (dotimes (i 100000)
         (put-text-property (1+ (* 4 i)) (+ 3 (* 4 i)) 'face (aref core-bench--props-faces (% i 5))))
       (buffer-size))
      ("fontlock-refont"
       (let ((p 1) (k 0))
         (while (< p (point-max))
           (let ((e (min (point-max) (+ p 1500))))
             (with-silent-modifications
               (remove-list-of-text-properties p e '(face))
               (let ((q p))
                 (while (< (+ q 3) e)
                   (put-text-property (1+ q) (+ q 3) 'face (aref core-bench--props-faces (% (cl-incf k) 5)))
                   (setq q (+ q 5)))))
             (setq p e))))
       (buffer-size))
      ("fontlock-elisp" (font-lock-ensure) (buffer-size))
      ("get-prop-rand"
       (let ((r 7) (n 0) (size (buffer-size)))
         (dotimes (_ 1000000)
           (setq r (% (+ (* r 1103515245) 12345) size))
           (when (get-text-property (1+ r) 'face) (cl-incf n)))
         n))
      ("next-single-scan"
       (let ((n 0))
         (dotimes (_ 10)
           (let ((p (point-min)))
             (while (setq p (next-single-property-change p 'face)) (cl-incf n))))
         n))
      ("props-at-scan"
       (let ((n 0) (p (point-min)))
         (while (< p (point-max))
           (when (text-properties-at p) (cl-incf n))
           (setq p (1+ p)))
         n))
      ("markers-1e4-edits"
       (let ((ms (cl-loop for i from 1 to 10000 collect (copy-marker (* i 100) (cl-oddp i))))
             (r 7))
         (setq core-bench--props-keep ms)
         (dotimes (_ 10000)
           (setq r (% (+ (* r 1103515245) 12345) 999000))
           (goto-char (1+ r)) (insert "qq") (delete-char -1))
         (apply #'+ (mapcar #'marker-position ms))))
      ("markers-1e3-typing"
       (let ((ms (cl-loop for i from 1 to 1000 collect (copy-marker (* i 900)))))
         (setq core-bench--props-keep ms)
         (goto-char 500000)
         (dotimes (_ 200000) (insert "a"))
         (apply #'+ (mapcar #'marker-position ms))))
      ("markers-make-1e5"
       (insert (make-string 100000 ?x))
       (setq core-bench--props-keep (cl-loop for i from 1 to 100000 collect (copy-marker i)))
       (length core-bench--props-keep)))))

(core-bench--def buf-prop-tablist-print
  (setq core-bench--props-result (core-bench--props-body "tablist-print" core-bench--props-buffer)))

(core-bench--def buf-prop-tablist-sort
  (setq core-bench--props-result (core-bench--props-body "tablist-sort" core-bench--props-buffer)))

(core-bench--def buf-prop-insert-propertized
  (setq core-bench--props-result (core-bench--props-body "insert-propertized" core-bench--props-buffer)))

(core-bench--def buf-prop-insert-then-add
  (setq core-bench--props-result (core-bench--props-body "insert-then-add" core-bench--props-buffer)))

(core-bench--def buf-prop-fontlock-put
  (setq core-bench--props-result (core-bench--props-body "fontlock-put" core-bench--props-buffer)))

(core-bench--def buf-prop-fontlock-refont
  (setq core-bench--props-result (core-bench--props-body "fontlock-refont" core-bench--props-buffer)))

(core-bench--def buf-prop-fontlock-elisp
  (setq core-bench--props-result (core-bench--props-body "fontlock-elisp" core-bench--props-buffer)))

(core-bench--def buf-prop-get-prop-rand
  (setq core-bench--props-result (core-bench--props-body "get-prop-rand" core-bench--props-buffer)))

(core-bench--def buf-prop-next-single-scan
  (setq core-bench--props-result (core-bench--props-body "next-single-scan" core-bench--props-buffer)))

(core-bench--def buf-prop-props-at-scan
  (setq core-bench--props-result (core-bench--props-body "props-at-scan" core-bench--props-buffer)))

(core-bench--def buf-markers-1e4-edits
  (setq core-bench--props-result (core-bench--props-body "markers-1e4-edits" core-bench--props-buffer)))

(core-bench--def buf-markers-1e3-typing
  (setq core-bench--props-result (core-bench--props-body "markers-1e3-typing" core-bench--props-buffer)))

(core-bench--def buf-markers-make-1e5
  (setq core-bench--props-result (core-bench--props-body "markers-make-1e5" core-bench--props-buffer)))

(defun core-bench--props-prepare (name)
  "Discard the previous sample and allocate fresh inputs outside timing."
  (when (buffer-live-p core-bench--props-buffer) (kill-buffer core-bench--props-buffer))
  (setq core-bench--props-keep nil
        core-bench--props-buffer
        (core-bench--props-prepare-input (cdr (assoc name core-bench--props-rows)))))

(defun core-bench--props-answer (name)
  "Observe properties outside timing, preserving the original P0 oracle."
  (if (member (cdr (assoc name core-bench--props-rows))
              '("tablist-print" "tablist-sort" "insert-propertized" "insert-then-add"
                "fontlock-put" "fontlock-refont" "fontlock-elisp"))
      (with-current-buffer core-bench--props-buffer (core-bench--props-digest))
    core-bench--props-result))


(defconst core-bench--props-memory-rows
  '("mem-props-runs-1e5" "mem-props-markers-1e5" "mem-props-tablist-90k"
    "mem-props-tablist-90k-noprops" "mem-props-strings-20k"))

(defun core-bench--props-memory (name)
  "Measure fresh-process post-GC RSS delta, without warmup or repetition."
  (let* ((buffer (if (equal name "mem-props-tablist-90k")
                     (core-bench--props-tablist-buffer 90000)
                   (generate-new-buffer " *props-memory*")))
         (core-bench--props-keep nil) before after units)
    (with-current-buffer buffer
      (when (member name '("mem-props-runs-1e5" "mem-props-markers-1e5"))
        (insert (make-string 400000 ?x)))
      (when (equal name "mem-props-tablist-90k") (setq tabulated-list-padding 0))
      (when (equal name "mem-props-strings-20k")
        (insert (propertize "word" 'face 'bold))
        (goto-char (point-min)) (re-search-forward "word"))
      (garbage-collect)
      (setq before (car (core-bench--rss)))
      (pcase name
        ("mem-props-runs-1e5"
         (dotimes (i 100000)
           (put-text-property (1+ (* 4 i)) (+ 3 (* 4 i))
                              'face (aref core-bench--props-faces (% i 5))))
         (setq units 200000))
        ("mem-props-markers-1e5"
         (setq core-bench--props-keep
               (cl-loop for i from 1 to 100000 collect (copy-marker (* 4 i))))
         (setq units 100000))
        ("mem-props-tablist-90k" (tabulated-list-print) (setq units 90000))
        ("mem-props-tablist-90k-noprops"
         (dotimes (_ 90000) (insert (make-string 11 ?x) "\n"))
         (setq units 90000))
        ("mem-props-strings-20k"
         (goto-char (point-min)) (re-search-forward "word")
         (setq core-bench--props-keep (cl-loop repeat 20000 collect (match-string 0)))
         (unless (cl-every (lambda (s) (and (equal s "word")
                                            (eq (get-text-property 0 'face s) 'bold)))
                          core-bench--props-keep)
           (error "Lost match-string properties"))
         (setq units 20000)))
      (garbage-collect)
      (setq after (core-bench--rss))
      (prog1
          `((name . ,name) (rss_before_gc_kib . ,before)
            (rss_after_gc_kib . ,(car after)) (peak_rss_kib . ,(cadr after))
            (anon_huge_pages_kib . ,(nth 2 after)) (pss_kib . ,(nth 3 after))
            (rss_delta_kib . ,(- (car after) before)) (units . ,units)
            (bytes_per_unit . ,(/ (* 1024.0 (- (car after) before)) units))
            (answer . ,(if (equal name "mem-props-markers-1e5")
                           (list (length core-bench--props-keep)
                                 (marker-position (car core-bench--props-keep))
                                 (marker-position (car (last core-bench--props-keep))))
                         (if (equal name "mem-props-strings-20k")
                             (list (length core-bench--props-keep)
                                   (prin1-to-string (object-intervals (car core-bench--props-keep))))
                           (core-bench--props-digest)))))
        (kill-buffer buffer)))))

(core-bench--def mem-props-runs-1e5 (error "Use fresh-process memory protocol"))
(core-bench--def mem-props-markers-1e5 (error "Use fresh-process memory protocol"))
(core-bench--def mem-props-tablist-90k (error "Use fresh-process memory protocol"))
(core-bench--def mem-props-tablist-90k-noprops (error "Use fresh-process memory protocol"))
(core-bench--def mem-props-strings-20k (error "Use fresh-process memory protocol"))

;;;; Driver

(defun core-bench--text-digest (text)
  "Digest text and canonical property intervals outside timing."
  (let ((p 0) (size (length text)) intervals
        (print-length nil) (print-level nil))
    (while (< p size)
      (let* ((end (next-property-change p text size))
             (props (text-properties-at p text))
             (pairs (cl-loop for (key value) on props by #'cddr
                             collect (list key value))))
        (push (list p end (sort pairs (lambda (a b)
                                      (string< (symbol-name (car a)) (symbol-name (car b))))))
              intervals)
        (setq p end)))
    (list (secure-hash 'sha1 (encode-coding-string text 'utf-8-unix))
          (secure-hash 'sha1 (encode-coding-string (prin1-to-string (nreverse intervals)) 'utf-8-unix)))))

(defun core-bench--answer (name result)
  "Reduce actual effects to a portable answer after timing stops."
  (cond
   ((member name '("buf-insert-random-1e5" "buf-textprop-1e5" "buf-utf8-decode-xdisp"))
    (core-bench--text-digest result))
   ((string-prefix-p "gnosis-" name)
    (core-bench--gnosis-digest result))
   ((equal name "chat-stream-20k")
    (let ((print-length nil) (print-level nil))
      (list (core-bench--text-digest (car result))
            (secure-hash 'sha1 (encode-coding-string (prin1-to-string (cadr result)) 'utf-8-unix)))))
   (t result)))

(defun core-bench--fixture-reference ()
  "Derive effect and fixture answers with GNU outside all measured runs."
  (append
   (mapcar (lambda (name)
             (cons name (core-bench--answer name
                          (funcall (cdr (assq (intern name) core-bench--all))))))
           '("buf-insert-random-1e5" "buf-textprop-1e5" "chat-stream-20k"))
   (when (getenv "CORE_BENCH_GNOSIS_DB")
     (mapcar (lambda (name)
               (core-bench--prepare name)
               (cons name (core-bench--answer name
                            (funcall (cdr (assq (intern name) core-bench--all))))))
             '("gnosis-dashboard-90k" "gnosis-dash-query" "gnosis-dash-entries"
               "gnosis-dash-print" "gnosis-dash-sort" "gnosis-dash-refresh" "gnosis-dash-nav")))
   (when (and (getenv "CORE_BENCH_XDISP")
             (file-readable-p (getenv "CORE_BENCH_XDISP")))
    (with-temp-buffer
      (core-bench--xdisp)
      (let* ((text (buffer-string)) (size (length text))
             (r 7) (sum 0) counts)
        (dotimes (_ 2000)
          (setq r (% (+ (* r 1103515245) 12345) size))
          (setq sum (+ sum 1 (cl-count ?\n text :end r))))
        (dolist (re '("^\\([a-z_]+\\) (" "\\_<\\(?:if\\|while\\|for\\|return\\)\\_>"
                      "\"\\(?:[^\"\\\\]\\|\\\\.\\)*\"" "/\\*\\(?:.\\|\n\\)*?\\*/"
                      "#\\s-*\\(?:define\\|include\\|ifdef\\|endif\\)"
                      "\\b[A-Z_][A-Z0-9_]\\{3,\\}\\b"))
          (let ((start 0) (n 0))
            (while (string-match re text start)
              (setq n (1+ n) start (match-end 0)))
            (push n counts)))
        (c-mode)
        `(("buf-utf8-decode-xdisp" . ,(core-bench--text-digest text))
          ("buf-position-xdisp" . ,sum)
          ("buf-forward-line-xdisp" . ,(funcall (cdr (assq 'buf-forward-line-xdisp core-bench--all))))
          ("search-literal-xdisp" . ,(* 20 (1- (length (split-string text "glyph_row")))))
          ("regex-fontlock-xdisp" . ,(nreverse counts))
          ("syntax-ppss-eob-xdisp" . ,(parse-partial-sexp (point-min) (point-max)))
          ("rss-after-workload" . ,(list size t))))))))

(defun core-bench--rss (&optional status-file rollup-file)
  "Read post-GC RSS, sampling-point HWM, anonymous huge pages and PSS KiB."
  (let (rss peak huge pss)
    (with-temp-buffer
      (insert-file-contents (or status-file "/proc/self/status"))
      (goto-char (point-min))
      (re-search-forward "^VmRSS:[[:space:]]+\\([0-9]+\\)")
      (setq rss (string-to-number (match-string 1)))
      (goto-char (point-min))
      (re-search-forward "^VmHWM:[[:space:]]+\\([0-9]+\\)")
      (setq peak (string-to-number (match-string 1))))
    (with-temp-buffer
      (insert-file-contents (or rollup-file "/proc/self/smaps_rollup"))
      (goto-char (point-min))
      (re-search-forward "^AnonHugePages:[[:space:]]+\\([0-9]+\\)")
      (setq huge (string-to-number (match-string 1)))
      (goto-char (point-min))
      (re-search-forward "^Pss:[[:space:]]+\\([0-9]+\\)")
      (setq pss (string-to-number (match-string 1))))
    (unless (and (> rss 0) (>= peak rss) (>= huge 0) (> pss 0))
      (error "Missing memory counters"))
    (list rss peak huge pss)))

(defun core-bench--time (name f)
  "Prepare, time F after a full GC, then check; return (MS GCS GC-MS)."
  (core-bench--prepare name)
  (garbage-collect)
  (let* ((gcs gcs-done) (gce gc-elapsed) (t0 (float-time))
         (result (funcall f))
         (sample (list (* 1000 (- (float-time) t0))
                       (- gcs-done gcs) (* 1000 (- gc-elapsed gce)))))
    (core-bench--check name result)
    (when (and (equal name "gc-full-pause-live") (= (nth 1 sample) 0))
      (error "Live-pause row did not collect"))
    sample))

(defun core-bench--row (name f)
  "Time row NAME (function F) and return its JSON alist."
  (condition-case err
      (if (member name core-bench--props-memory-rows)
          (core-bench--props-memory name)
        (core-bench--time name f)
        (let ((rs (sort (cl-loop repeat (string-to-number (or (getenv "CORE_BENCH_INNER_REPEATS") "5"))
                               collect (core-bench--time name f))
                        (lambda (a b) (< (car a) (car b))))))
          `((name . ,name) (median_ms . ,(car (nth (/ (length rs) 2) rs)))
            (min_ms . ,(car (car rs))) (max_ms . ,(car (car (last rs))))
            (gcs . ,(nth 1 (nth (/ (length rs) 2) rs))) (gc_ms . ,(nth 2 (nth (/ (length rs) 2) rs)))
            ,@(progn (garbage-collect)
                     (let ((rss (core-bench--rss)))
                       `((rss_after_gc_kib . ,(car rss)) (peak_rss_kib . ,(cadr rss))
                         (anon_huge_pages_kib . ,(nth 2 rss)) (pss_kib . ,(nth 3 rss)))))
            ,@(when (assoc name core-bench--props-rows)
                `((answer . ,(core-bench--props-answer name)))))))
    (error `((name . ,name) (error . ,(error-message-string err))))))

(defun core-bench-run ()
  "Run the rows named in `command-line-args-left' (all if none)."
  (let ((names (mapcar #'intern command-line-args-left)))
    (setq command-line-args-left nil)
    (dolist (row (reverse core-bench--all))
      (when (or (null names) (memq (car row) names))
        (princ (json-encode (core-bench--row (symbol-name (car row)) (cdr row))))
        (terpri)))))

(provide 'core-bench)
;;; core-bench.el ends here
