;;; romacs-live-tests.el --- Live updates of a session's Lisp  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each test makes a build (a runtime root with build.json and a lisp/
;; tree) and a source tree, a git repository whose first commit is the
;; build's, loads the build's files, edits the source and updates the
;; session from it.  Each trap the update handles has a test that fails
;; with a plain reload of the changed file.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-live)

;; The fixtures' variables, defined as each test loads its files.
(defvar rlt-km-map)
(defvar rlt-km-child-map)
(defvar rlt-uk-map)
(defvar rlt-op-limit)
(defvar rlt-op-set)
(defvar rlt-dv-table)
(defvar rlt-dv-count)
(defvar rlt-dv-width)
(defvar rlt-mva-twice)
(defvar rlt-ad-hook)
(defvar rlt-lk-map)
(defvar romacs-server-build-watch)
(defvar warm-restart-directory)

(defun romacs-live-tests--git (dir &rest args)
  "Git ARGS in DIR; its output."
  (with-temp-buffer
    (let ((default-directory dir))
      (unless (zerop (apply #'call-process "git" nil t nil "-c" "user.name=Romacs test" "-c" "user.email=test@example.invalid"
                            "-c" "commit.gpgsign=false" args))
        (error "git %S failed: %s" args (buffer-string))))
    (string-trim (buffer-string))))

(defun romacs-live-tests--write (file text)
  (make-directory (file-name-directory file) t)
  (let ((coding-system-for-write 'utf-8-unix)) (write-region text nil file nil 'silent)))

(defun romacs-live-tests--file (name &rest forms)
  "A Lisp file's text providing NAME, made of FORMS (strings)."
  (concat ";;; " name ".el --- a fixture  -*- lexical-binding: t -*-\n"
          (mapconcat #'identity forms "\n") "\n(provide '" name ")\n"))

(defun romacs-live-tests--fixture (files)
  "A source tree whose first commit holds FILES (an alist of path to text)
and a build of that commit: a plist `:top', `:src', `:build', `:commit'."
  (let* ((top (file-name-as-directory (make-temp-file "romacs-live-" t)))
         (src (expand-file-name "src/" top))
         (build (expand-file-name "build/" top)))
    (romacs-live-tests--write (expand-file-name "etc/romacs-runtime-epoch" src) "5\n")
    (pcase-dolist (`(,path . ,text) files)
      (romacs-live-tests--write (expand-file-name path src) text)
      (romacs-live-tests--write (expand-file-name path build) text))
    (romacs-live-tests--git src "init" "-q")
    (romacs-live-tests--git src "add" "-A")
    (romacs-live-tests--git src "commit" "-q" "-m" "base")
    (let ((commit (romacs-live-tests--git src "rev-parse" "HEAD")))
      (romacs-live-tests--write (expand-file-name "build.json" build) (format "{\"commit\":\"%s\"}" commit))
      (list :top top :src src :build build :commit commit))))

(defmacro romacs-live-tests--with (fixture files &rest body)
  "Run BODY with FIXTURE bound to a fixture of FILES, its build's lisp/ on
`load-path', its own ledger, store and mode line."
  (declare (indent 2))
  `(let* ((,fixture (romacs-live-tests--fixture ,files))
          (load-path (cons (expand-file-name "lisp" (plist-get ,fixture :build)) load-path))
          (romacs-live-ledger nil) (romacs-live-last-report nil) (romacs-live--running nil)
          (romacs-live-mode-line nil) (global-mode-string nil)
          (romacs-live-directory (expand-file-name "store" (plist-get ,fixture :top)))
          (romacs-live-keymap-defaults-function nil))
     (cl-letf (((symbol-function 'romacs-live--runtime-root) (lambda () (directory-file-name (plist-get ,fixture :build)))))
       (unwind-protect (progn ,@body)
         (delete-directory (plist-get ,fixture :top) t)))))

(defun romacs-live-tests--edit (fixture path text)
  (romacs-live-tests--write (expand-file-name path (plist-get fixture :src)) text))

(defun romacs-live-tests--update (fixture &rest options)
  "Update the session from FIXTURE's source tree; the report."
  (let ((report :none))
    (apply #'romacs-live-start (list :kind 'worktree :directory (plist-get fixture :src))
           (lambda (r) (setq report r)) options)
    (should-not (eq report :none))
    report))

(defun romacs-live-tests--row (report path)
  (cl-find path (plist-get report :rows) :key (lambda (r) (plist-get r :path)) :test #'equal))

;;;; Keymaps

(ert-deftest romacs-live-a-changed-keymap-reaches-every-holder ()
  ;; A plain reload leaves the map's variable bound, so the new keys
  ;; reach no buffer; unbinding and reloading makes a new map that no
  ;; buffer, derived map or text property holds.  The update changes the
  ;; old map in place.
  (romacs-live-tests--with fx
      `(("lisp/rlt-km.el" . ,(romacs-live-tests--file "rlt-km"
                                 "(defvar rlt-km-map (let ((m (make-sparse-keymap))) (define-key m \"a\" #'ignore) m))"
                                 "(defvar rlt-km-child-map (let ((m (make-sparse-keymap))) (set-keymap-parent m rlt-km-map) m))")))
    (require 'rlt-km)
    (let ((old rlt-km-map) (child rlt-km-child-map))
      (with-temp-buffer
        (use-local-map rlt-km-map)
        (insert (propertize "chat" 'keymap rlt-km-map))
        (romacs-live-tests--edit fx "lisp/rlt-km.el"
                                 (romacs-live-tests--file "rlt-km"
                                     "(defvar rlt-km-map (let ((m (make-sparse-keymap))) (define-key m \"b\" #'forward-char) m))"
                                     "(defvar rlt-km-child-map (let ((m (make-sparse-keymap))) (set-keymap-parent m rlt-km-map) m))"))
        (let ((report (romacs-live-tests--update fx)))
          (should-not (plist-get report :refusals))
          (should (eq rlt-km-map old))
          (should (eq rlt-km-child-map child))
          (should (eq (local-key-binding "b") #'forward-char))
          (should-not (local-key-binding "a"))
          (should (eq (lookup-key (get-text-property 1 'keymap) "b") #'forward-char))
          (should (eq (keymap-parent rlt-km-child-map) old))
          (should (eq (lookup-key rlt-km-child-map "b") #'forward-char)))))))

(ert-deftest romacs-live-a-derived-modes-map-keeps-its-parent ()
  ;; `define-derived-mode' makes the child's map with no parent and gives
  ;; it one when the mode first runs: the new map the load makes has none,
  ;; and the child's buffers would lose every key of the parent mode.
  (romacs-live-tests--with fx
      `(("lisp/rlt-dm.el" . ,(romacs-live-tests--file "rlt-dm"
                                 "(defvar rlt-dm-mode-map (let ((m (make-sparse-keymap))) (define-key m \"a\" #'ignore) m))"
                                 "(define-derived-mode rlt-dm-mode special-mode \"P\")"
                                 "(define-derived-mode rlt-dm-child-mode rlt-dm-mode \"C\")")))
    (require 'rlt-dm)
    (with-temp-buffer
      (funcall 'rlt-dm-child-mode)
      (should (eq (key-binding "a") #'ignore))
      (romacs-live-tests--edit fx "lisp/rlt-dm.el"
                               (romacs-live-tests--file "rlt-dm"
                                   "(defvar rlt-dm-mode-map (let ((m (make-sparse-keymap))) (define-key m \"b\" #'forward-char) m))"
                                   "(define-derived-mode rlt-dm-mode special-mode \"P\")"
                                   "(define-derived-mode rlt-dm-child-mode rlt-dm-mode \"C\")"))
      (romacs-live-tests--update fx)
      (should (eq (key-binding "b") #'forward-char))
      (should (eq (key-binding "q") #'quit-window)))))

(ert-deftest romacs-live-a-replaced-keymap-keeps-the-users-keys ()
  ;; The user bound u in the map after loading it: the library's own
  ;; bindings (what the defaults function says) lack it, so it is the
  ;; user's, and the update binds it again on the replaced map.
  (romacs-live-tests--with fx
      `(("lisp/rlt-uk.el" . ,(romacs-live-tests--file "rlt-uk"
                                 "(defvar rlt-uk-map (let ((m (make-sparse-keymap))) (define-key m \"a\" #'ignore) m))")))
    (require 'rlt-uk)
    (define-key rlt-uk-map "u" #'undo)
    (romacs-live-tests--edit fx "lisp/rlt-uk.el"
                             (romacs-live-tests--file "rlt-uk"
                                 "(defvar rlt-uk-map (let ((m (make-sparse-keymap))) (define-key m \"b\" #'forward-char) m))"))
    (let* ((romacs-live-keymap-defaults-function
            (lambda (_files _library variables callback)
              (funcall callback (mapcar (lambda (var) (cons var (list (cons [?a] 'ignore)))) variables))))
           (report (romacs-live-tests--update fx))
           (row (car (plist-get (romacs-live-tests--row report "lisp/rlt-uk.el") :keymaps))))
      (should (eq (lookup-key rlt-uk-map "u") #'undo))
      (should (eq (lookup-key rlt-uk-map "b") #'forward-char))
      (should (equal (plist-get row :user) (list [?u]))))))

;;;; Options, variables, faces

(ert-deftest romacs-live-an-untouched-option-takes-its-new-default ()
  ;; `defcustom' on a bound variable changes only its standard: the
  ;; option the user never set keeps the old default without the update.
  (romacs-live-tests--with fx
      `(("lisp/rlt-op.el" . ,(romacs-live-tests--file "rlt-op"
                                 "(defcustom rlt-op-limit 10 \"A limit.\" :type 'integer :group 'romacs-live)"
                                 "(defcustom rlt-op-set 1 \"Set by the user.\" :type 'integer :group 'romacs-live)")))
    (require 'rlt-op)
    (setopt rlt-op-set 5)
    (romacs-live-tests--edit fx "lisp/rlt-op.el"
                             (romacs-live-tests--file "rlt-op"
                                 "(defcustom rlt-op-limit 20 \"A limit.\" :type 'integer :group 'romacs-live)"
                                 "(defcustom rlt-op-set 2 \"Set by the user.\" :type 'integer :group 'romacs-live)"))
    (let ((report (romacs-live-tests--update fx)))
      (should (= rlt-op-limit 20))
      (should (= rlt-op-set 5))
      (should (cl-find-if (lambda (v) (and (eq (plist-get v :option) 'rlt-op-set) (plist-get v :kept)))
                          (plist-get (romacs-live-tests--row report "lisp/rlt-op.el") :values))))))

(ert-deftest romacs-live-state-is-kept-and-an-untouched-literal-is-not ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-dv.el" . ,(romacs-live-tests--file "rlt-dv"
                                 "(defvar rlt-dv-table (make-hash-table))"
                                 "(defvar rlt-dv-width 3)"
                                 "(defvar rlt-dv-count 0)")))
    (require 'rlt-dv)
    (puthash 'k 'v rlt-dv-table)
    (setq rlt-dv-count 7)
    (romacs-live-tests--edit fx "lisp/rlt-dv.el"
                             (romacs-live-tests--file "rlt-dv"
                                 "(defvar rlt-dv-table (make-hash-table :test 'equal))"
                                 "(defvar rlt-dv-width 4)"
                                 "(defvar rlt-dv-count 1)"))
    (romacs-live-tests--update fx)
    (should (eq (gethash 'k rlt-dv-table) 'v))
    (should (= rlt-dv-width 4))
    (should (= rlt-dv-count 7))))

(ert-deftest romacs-live-a-moved-variable-is-bound ()
  ;; The 2026-11-06 case: a later version defines a variable in one file
  ;; and uses it in another.  Reloading only the user of it leaves it void;
  ;; the update loads every file that differs, the definer first though
  ;; its name sorts last, and the user reads it as it loads.
  (romacs-live-tests--with fx
      `(("lisp/rlt-mvz.el" . ,(romacs-live-tests--file "rlt-mvz" "(defun rlt-mvz-f () 1)"))
        ("lisp/rlt-mva.el" . ,(romacs-live-tests--file "rlt-mva" "(require 'rlt-mvz)" "(defun rlt-mva-fits (n) (<= n 30))")))
    (require 'rlt-mva)
    (romacs-live-tests--edit fx "lisp/rlt-mvz.el"
                             (romacs-live-tests--file "rlt-mvz" "(defcustom rlt-mvz-max 20 \"Max.\" :type 'integer :group 'romacs-live)"
                                                      "(defun rlt-mvz-f () 1)"))
    (romacs-live-tests--edit fx "lisp/rlt-mva.el"
                             (romacs-live-tests--file "rlt-mva" "(require 'rlt-mvz)" "(defvar rlt-mva-twice (* 2 rlt-mvz-max))"
                                                      "(defun rlt-mva-fits (n) (<= n rlt-mvz-max))"))
    (let ((report (romacs-live-tests--update fx)))
      (should-not (plist-get report :failed))
      (should (equal (mapcar (lambda (r) (plist-get r :path)) (plist-get report :rows))
                     '("lisp/rlt-mvz.el" "lisp/rlt-mva.el")))
      (should (= rlt-mva-twice 40))
      (should (funcall 'rlt-mva-fits 5))
      (should-not (funcall 'rlt-mva-fits 25)))))

(ert-deftest romacs-live-an-uncustomized-face-takes-its-new-spec ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-fc.el" . ,(romacs-live-tests--file "rlt-fc"
                                 "(defface rlt-fc-face '((t :weight bold)) \"A face.\" :group 'romacs-live)")))
    (require 'rlt-fc)
    (romacs-live-tests--edit fx "lisp/rlt-fc.el"
                             (romacs-live-tests--file "rlt-fc"
                                 "(defface rlt-fc-face '((t :slant italic)) \"A face.\" :group 'romacs-live)"))
    (romacs-live-tests--update fx)
    (should (equal (get 'rlt-fc-face 'face-defface-spec) '((t :slant italic))))
    (should (eq (face-attribute 'rlt-fc-face :slant nil t) 'italic))))

;;;; Advice and hooks

(ert-deftest romacs-live-advice-and-the-users-hook-functions-stay ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ad.el" . ,(romacs-live-tests--file "rlt-ad" "(defvar rlt-ad-hook nil)" "(defun rlt-ad-f () 1)")))
    (require 'rlt-ad)
    (advice-add 'rlt-ad-f :around (lambda (f) (* 10 (funcall f))) '((name . rlt-ad-test)))
    (add-hook 'rlt-ad-hook #'ignore)
    (romacs-live-tests--edit fx "lisp/rlt-ad.el"
                             (romacs-live-tests--file "rlt-ad" "(defvar rlt-ad-hook '(rlt-ad-default))" "(defun rlt-ad-f () 2)"))
    (romacs-live-tests--update fx)
    (should (= (funcall 'rlt-ad-f) 20))
    (should (memq #'ignore rlt-ad-hook))))

;;;; Refusals, staging, order, the ledger

(ert-deftest romacs-live-a-struct-layout-change-refuses-the-update ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-st.el" . ,(romacs-live-tests--file "rlt-st" "(cl-defstruct rlt-st-rec a b)" "(defun rlt-st-f () 1)")))
    (require 'rlt-st)
    (romacs-live-tests--edit fx "lisp/rlt-st.el"
                             (romacs-live-tests--file "rlt-st" "(cl-defstruct rlt-st-rec a b c)" "(defun rlt-st-f () 2)"))
    (let ((report (romacs-live-tests--update fx)))
      (should (string-match-p "rlt-st-rec changes its layout (\\+c)" (car (plist-get report :refusals))))
      (should (= (funcall 'rlt-st-f) 1))
      (should-not romacs-live-ledger))))

(ert-deftest romacs-live-a-runtime-change-refuses-the-update ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ep.el" . ,(romacs-live-tests--file "rlt-ep" "(defun rlt-ep-f () 1)")))
    (require 'rlt-ep)
    (romacs-live-tests--edit fx "etc/romacs-runtime-epoch" "6\n")
    (romacs-live-tests--edit fx "lisp/vm/prelude.el" "(progn)\n")
    (romacs-live-tests--edit fx "lisp/rlt-ep.el" (romacs-live-tests--file "rlt-ep" "(defun rlt-ep-f () 2)"))
    (let ((reasons (plist-get (romacs-live-tests--update fx) :refusals)))
      (should (cl-some (lambda (r) (string-match-p "runtime epoch" r)) reasons))
      (should (cl-some (lambda (r) (string-match-p "lisp/vm/prelude.el" r)) reasons))
      (should (= (funcall 'rlt-ep-f) 1)))))

(ert-deftest romacs-live-a-session-without-a-build-takes-files-only ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-nb.el" . ,(romacs-live-tests--file "rlt-nb" "(defun rlt-nb-f () 1)")))
    (delete-file (expand-file-name "build.json" (plist-get fx :build)))
    (require 'rlt-nb)
    (let ((file (expand-file-name "lisp/rlt-nb.el" (plist-get fx :src))) (report nil))
      (romacs-live-tests--edit fx "lisp/rlt-nb.el" (romacs-live-tests--file "rlt-nb" "(defun rlt-nb-f () 2)"))
      (should (string-match-p "build is unknown" (car (plist-get (romacs-live-tests--update fx) :refusals))))
      (romacs-live-start (list :kind 'files :files (list file)) (lambda (r) (setq report r)))
      (should-not (plist-get report :refusals))
      (should (= (funcall 'rlt-nb-f) 2)))))

(ert-deftest romacs-live-a-file-not-loaded-is-staged ()
  ;; The session never loaded it: it is not loaded now, but a later
  ;; `require' gets the source's copy, not the build's.
  (romacs-live-tests--with fx
      `(("lisp/rlt-sg.el" . ,(romacs-live-tests--file "rlt-sg" "(defun rlt-sg-f () 1)")))
    (romacs-live-tests--edit fx "lisp/rlt-sg.el" (romacs-live-tests--file "rlt-sg" "(defun rlt-sg-f () 2)"))
    (let ((report (romacs-live-tests--update fx)))
      (should (eq (plist-get (romacs-live-tests--row report "lisp/rlt-sg.el") :action) 'stage))
      (should-not (featurep 'rlt-sg))
      (require 'rlt-sg)
      (should (= (funcall 'rlt-sg-f) 2)))))

(ert-deftest romacs-live-the-ledger-names-what-runs-and-a-revert-empties-it ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-lg.el" . ,(romacs-live-tests--file "rlt-lg" "(defun rlt-lg-f () 1)")))
    (require 'rlt-lg)
    (romacs-live-tests--edit fx "lisp/rlt-lg.el" (romacs-live-tests--file "rlt-lg" "(defun rlt-lg-f () 2)"))
    (romacs-live-tests--update fx)
    (let ((entry (car (plist-get romacs-live-ledger :entries))))
      (should (equal (plist-get entry :path) "lisp/rlt-lg.el"))
      (should (eq (plist-get entry :state) 'loaded))
      (should (equal (plist-get (plist-get romacs-live-ledger :build) :commit) (plist-get fx :commit)))
      (should (string-prefix-p (expand-file-name romacs-live-directory) (plist-get entry :file)))
      (should (string-match-p "Δ1" romacs-live-mode-line))
      ;; Headed by a string: a list headed by a symbol is a conditional,
      ;; which the mode line shows as *invalid*.
      (should (equal global-mode-string '("" romacs-live-mode-line)))
      ;; One load-history entry says what the session has of the file.
      (should (= 1 (cl-count-if (lambda (e) (and (stringp (car e)) (string-suffix-p "rlt-lg.el" (car e)))) load-history))))
    ;; An update with nothing new loads nothing.
    (should-not (plist-get (romacs-live-tests--update fx) :rows))
    (romacs-live-tests--git (plist-get fx :src) "checkout" "--" "lisp/rlt-lg.el")
    (let ((report (romacs-live-tests--update fx)))
      (should (eq (plist-get (romacs-live-tests--row report "lisp/rlt-lg.el") :action) 'revert))
      (should (= (funcall 'rlt-lg-f) 1))
      (should-not romacs-live-ledger)
      (should-not romacs-live-mode-line))))

(ert-deftest romacs-live-gone-functions-are-listed-and-unbound-on-request ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-gn.el" . ,(romacs-live-tests--file "rlt-gn" "(defun rlt-gn-one () 1)" "(defun rlt-gn-two () 1)"
                                                      "(defun rlt-gn-kept () 1)")))
    (require 'rlt-gn)
    (romacs-live-tests--edit fx "lisp/rlt-gn.el" (romacs-live-tests--file "rlt-gn" "(defun rlt-gn-two () 1)" "(defun rlt-gn-kept () 2)"))
    (let ((report (romacs-live-tests--update fx :plan-only t)))
      (should (plist-get report :plan))
      (should (= (funcall 'rlt-gn-kept) 1)))
    (let ((report (romacs-live-tests--update fx)))
      (should (equal (plist-get report :gone) '(rlt-gn-one)))
      (should (fboundp 'rlt-gn-one)))
    (romacs-live-tests--edit fx "lisp/rlt-gn.el" (romacs-live-tests--file "rlt-gn" "(defun rlt-gn-kept () 3)"))
    (let ((report (romacs-live-tests--update fx :unbind-gone t)))
      (should (equal (plist-get report :unbound) '(rlt-gn-two)))
      (should-not (fboundp 'rlt-gn-two))
      (should (= (funcall 'rlt-gn-kept) 3)))))

(ert-deftest romacs-live-a-macro-change-reloads-its-users ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-mca.el" . ,(romacs-live-tests--file "rlt-mca" "(defmacro rlt-mca-m () 1)"))
        ("lisp/rlt-mcb.el" . ,(romacs-live-tests--file "rlt-mcb" "(require 'rlt-mca)" "(defun rlt-mcb-f () (rlt-mca-m))")))
    (require 'rlt-mcb)
    (romacs-live-tests--edit fx "lisp/rlt-mca.el" (romacs-live-tests--file "rlt-mca" "(defmacro rlt-mca-m () 2)"))
    (let ((report (romacs-live-tests--update fx)))
      (should (eq (plist-get (romacs-live-tests--row report "lisp/rlt-mcb.el") :action) 'dependent))
      (should (= (funcall 'rlt-mcb-f) 2)))))

(ert-deftest romacs-live-the-tool-answers-with-the-report-as-json ()
  (require 'romacs-mcp)
  (romacs-live-tests--with fx
      `(("lisp/rlt-mc.el" . ,(romacs-live-tests--file "rlt-mc" "(defvar rlt-mc-map (make-sparse-keymap))" "(defun rlt-mc-f () 1)")))
    (require 'rlt-mc)
    (romacs-live-tests--edit fx "lisp/rlt-mc.el" (romacs-live-tests--file "rlt-mc" "(defvar rlt-mc-map (make-sparse-keymap))"
                                                                         "(defun rlt-mc-f () 2)"))
    (let* ((arguments (make-hash-table :test #'equal))
           (_ (puthash "source" (plist-get fx :src) arguments))
           (deferred (romacs-live-mcp-update arguments))
           (value (car (funcall (nth 1 deferred))))
           (json (json-parse-string (json-serialize value))))
      (should (string-match-p "1 loaded" (gethash "summary" json)))
      (let ((row (aref (gethash "rows" json) 0)))
        (should (equal (gethash "path" row) "lisp/rlt-mc.el"))
        (should (equal (gethash "action" row) "load"))
        (should (equal (gethash "changed" (gethash "diff" row)) ["function rlt-mc-f"]))
        (should (equal (gethash "variable" (aref (gethash "keymaps" row) 0)) "rlt-mc-map")))
      (should (= (funcall 'rlt-mc-f) 2)))))

(ert-deftest romacs-live-a-file-edited-in-place-reloads-without-a-build ()
  ;; A session running a worktree's own tree: the file it loaded is the
  ;; one edited, so the build's copy says nothing of what it loaded.
  (romacs-live-tests--with fx
      `(("lisp/rlt-ip.el" . ,(romacs-live-tests--file "rlt-ip" "(defun rlt-ip-f () 1)")))
    (let ((load-path (cons (expand-file-name "lisp" (plist-get fx :src)) load-path))
          (file (expand-file-name "lisp/rlt-ip.el" (plist-get fx :src))) (report nil))
      (cl-letf (((symbol-function 'romacs-live--runtime-root) (lambda () (directory-file-name (plist-get fx :src)))))
        (require 'rlt-ip)
        (romacs-live-tests--edit fx "lisp/rlt-ip.el" (romacs-live-tests--file "rlt-ip" "(defun rlt-ip-f () 2)"))
        (romacs-live-start (list :kind 'files :files (list file)) (lambda (r) (setq report r)))
        (should (eq (plist-get (romacs-live-tests--row report "lisp/rlt-ip.el") :action) 'load))
        (should (= (funcall 'rlt-ip-f) 2))))))

(ert-deftest romacs-live-a-revert-after-two-updates-is-the-builds ()
  ;; Each update's copy replaces the last one's directory on `load-path':
  ;; a revert then finds the build's file, not the first update's copy.
  (romacs-live-tests--with fx
      `(("lisp/rlt-rv.el" . ,(romacs-live-tests--file "rlt-rv" "(defun rlt-rv-f () 0)"))
        ("lisp/rlt-rs.el" . ,(romacs-live-tests--file "rlt-rs" "(defun rlt-rs-f () 0)")))
    (require 'rlt-rv)
    (dolist (n '(1 2))
      (romacs-live-tests--edit fx "lisp/rlt-rv.el" (romacs-live-tests--file "rlt-rv" (format "(defun rlt-rv-f () %d)" n)))
      (romacs-live-tests--edit fx "lisp/rlt-rs.el" (romacs-live-tests--file "rlt-rs" (format "(defun rlt-rs-f () %d)" n)))
      (romacs-live-tests--update fx)
      (should (= (funcall 'rlt-rv-f) n)))
    (should (= 2 (length (plist-get romacs-live-ledger :entries))))
    (romacs-live-tests--git (plist-get fx :src) "checkout" "--" "lisp")
    (let ((report (romacs-live-tests--update fx)))
      (should (= (funcall 'rlt-rv-f) 0))
      ;; Staged, never loaded: unstaged, not loaded now.
      (should (eq (plist-get (romacs-live-tests--row report "lisp/rlt-rs.el") :action) 'unstage))
      (should-not (featurep 'rlt-rs))
      (should-not romacs-live-ledger)
      (should-not (cl-some (lambda (dir) (string-prefix-p (expand-file-name romacs-live-directory) (expand-file-name dir)))
                           (cl-remove-if-not #'stringp load-path))))))

(ert-deftest romacs-live-a-qualified-library-keeps-its-name ()
  ;; Loaded as sub/rlt-q, from below a directory on `load-path': the copy
  ;; keeps its subdirectory, so the same name finds it.
  (romacs-live-tests--with fx
      `(("lisp/sub/rlt-q.el" . ,(romacs-live-tests--file "rlt-q" "(defun rlt-q-f () 1)")))
    (load "sub/rlt-q" nil t)
    (romacs-live-tests--edit fx "lisp/sub/rlt-q.el" (romacs-live-tests--file "rlt-q" "(defun rlt-q-f () 2)"))
    (romacs-live-tests--update fx)
    (should (= (funcall 'rlt-q-f) 2))
    (should (string-prefix-p (expand-file-name romacs-live-directory) (locate-library "sub/rlt-q")))))

(ert-deftest romacs-live-the-users-lambda-key-stays-and-a-conflict-is-said ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-lk.el" . ,(romacs-live-tests--file "rlt-lk"
                                 "(defvar rlt-lk-map (let ((m (make-sparse-keymap))) (define-key m \"a\" #'ignore) m))")))
    (require 'rlt-lk)
    (let ((mine (lambda () (interactive) 'mine)))
      (define-key rlt-lk-map "u" mine)
      (define-key rlt-lk-map "pq" #'undo)
      (romacs-live-tests--edit fx "lisp/rlt-lk.el"
                               (romacs-live-tests--file "rlt-lk"
                                   "(defvar rlt-lk-map (let ((m (make-sparse-keymap))) (define-key m \"p\" #'ignore) m))"))
      (let* ((romacs-live-keymap-defaults-function
              (lambda (_files _library variables callback)
                (funcall callback (mapcar (lambda (var) (cons var (list (cons [?a] 'ignore)))) variables))))
             (report (romacs-live-tests--update fx))
             (row (car (plist-get (romacs-live-tests--row report "lisp/rlt-lk.el") :keymaps))))
        (should-not (plist-get report :failed))
        (should (eq (lookup-key rlt-lk-map "u") mine))
        (should (eq (lookup-key rlt-lk-map "p") #'ignore))
        (should (equal (plist-get row :conflicts) (list [?p ?q])))))))

(ert-deftest romacs-live-a-face-takes-each-new-spec ()
  ;; `defface' names a face in load-history only the first time: the
  ;; second update finds it in the old text.
  (romacs-live-tests--with fx
      `(("lisp/rlt-f2.el" . ,(romacs-live-tests--file "rlt-f2" "(defface rlt-f2-face '((t :weight bold)) \"A face.\" :group 'romacs-live)")))
    (require 'rlt-f2)
    (dolist (slant '(italic oblique))
      (romacs-live-tests--edit fx "lisp/rlt-f2.el"
                               (romacs-live-tests--file "rlt-f2" (format "(defface rlt-f2-face '((t :slant %s)) \"A face.\" :group 'romacs-live)" slant)))
      (romacs-live-tests--update fx)
      (should (eq (face-attribute 'rlt-f2-face :slant nil t) slant)))))

(ert-deftest romacs-live-a-failed-load-is-partial-and-loads-again ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-pf.el" . ,(romacs-live-tests--file "rlt-pf" "(defun rlt-pf-f () 1)")))
    (require 'rlt-pf)
    (romacs-live-tests--edit fx "lisp/rlt-pf.el" (romacs-live-tests--file "rlt-pf" "(defun rlt-pf-f () 2)" "(error \"broken\")"))
    (let ((report (romacs-live-tests--update fx)))
      (should (equal (plist-get report :failed) "lisp/rlt-pf.el"))
      (should (eq (plist-get (car (plist-get romacs-live-ledger :entries)) :state) 'partial)))
    ;; The same source again: the partial file is tried again, not skipped.
    (should (equal (plist-get (romacs-live-tests--update fx) :failed) "lisp/rlt-pf.el"))
    (romacs-live-tests--edit fx "lisp/rlt-pf.el" (romacs-live-tests--file "rlt-pf" "(defun rlt-pf-f () 3)"))
    (romacs-live-tests--update fx)
    (should (= (funcall 'rlt-pf-f) 3))
    (should (eq (plist-get (car (plist-get romacs-live-ledger :entries)) :state) 'loaded))))

(ert-deftest romacs-live-a-quit-ends-the-update ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-qt.el" . ,(romacs-live-tests--file "rlt-qt" "(defun rlt-qt-f () 1)")))
    (require 'rlt-qt)
    (romacs-live-tests--edit fx "lisp/rlt-qt.el" (romacs-live-tests--file "rlt-qt" "(defun rlt-qt-f () 2)"))
    (cl-letf (((symbol-function 'romacs-live-apply) (lambda (&rest _) (signal 'quit nil))))
      (let ((report (romacs-live-tests--update fx)))
        ;; Stopped, not refused: it may have loaded files first.
        (should-not (plist-get report :refusals))
        (should (string-match-p "C-g" (plist-get report :stopped)))
        (should-not (string-match-p "nothing loaded" (romacs-live-summary report)))))
    (should-not romacs-live--running)))

(ert-deftest romacs-live-a-moved-function-is-not-gone ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ma.el" . ,(romacs-live-tests--file "rlt-ma" "(defun rlt-ma-moved () 1)"))
        ("lisp/rlt-mb.el" . ,(romacs-live-tests--file "rlt-mb" "(defun rlt-mb-f () 1)")))
    (require 'rlt-ma)
    (require 'rlt-mb)
    (romacs-live-tests--edit fx "lisp/rlt-ma.el" (romacs-live-tests--file "rlt-ma"))
    (romacs-live-tests--edit fx "lisp/rlt-mb.el" (romacs-live-tests--file "rlt-mb" "(defun rlt-mb-f () 1)" "(defun rlt-ma-moved () 2)"))
    (let ((report (romacs-live-tests--update fx :unbind-gone t)))
      (should-not (plist-get report :gone))
      (should (= (funcall 'rlt-ma-moved) 2)))))

(ert-deftest romacs-live-only-lisp-is-a-candidate-and-the-tool-needs-full-access ()
  (require 'romacs-mcp)
  (should (member "romacs_live_update" romacs-mcp--full-access-tools))
  (romacs-live-tests--with fx
      `(("lisp/rlt-ol.el" . ,(romacs-live-tests--file "rlt-ol" "(defun rlt-ol-f () 1)"))
        ("test/rlt-ol-tests.el" . "(message \"a test\")\n"))
    (require 'rlt-ol)
    (romacs-live-tests--edit fx "test/rlt-ol-tests.el" "(message \"a changed test\")\n")
    (should-not (plist-get (romacs-live-tests--update fx) :rows))))

(ert-deftest romacs-live-forms-layout-and-macro-changes ()
  ;; A representation change moves the accessors' offsets; a function that
  ;; becomes a macro changes what its callers compiled.
  (should-not (equal (romacs-live-forms-layout '(cl-defstruct (r (:initial-offset 1)) a))
                     (romacs-live-forms-layout '(cl-defstruct r a))))
  (should-not (equal (romacs-live-forms-layout '(defclass c () ((a :allocation :class))))
                     (romacs-live-forms-layout '(defclass c () ((a))))))
  (should (romacs-live-forms-macro-change-p
           (romacs-live-forms-diff (romacs-live-forms-read "(defun m () 1)") (romacs-live-forms-read "(defmacro m () 1)")))))

(ert-deftest romacs-live-a-staged-file-the-source-drops-is-unstaged ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ns0.el" . ,(romacs-live-tests--file "rlt-ns0" "(defun rlt-ns0-f () 1)")))
    (romacs-live-tests--edit fx "lisp/rlt-ns.el" (romacs-live-tests--file "rlt-ns" "(defun rlt-ns-f () 1)"))
    (romacs-live-tests--update fx)
    (should (locate-library "rlt-ns"))
    (delete-file (expand-file-name "lisp/rlt-ns.el" (plist-get fx :src)))
    (romacs-live-tests--update fx)
    (should-not romacs-live-ledger)
    (should-not (locate-library "rlt-ns"))))

(ert-deftest romacs-live-a-face-may-take-a-nil-spec ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-fn.el" . ,(romacs-live-tests--file "rlt-fn" "(defface rlt-fn-face '((t :weight bold)) \"A face.\" :group 'romacs-live)")))
    (require 'rlt-fn)
    (romacs-live-tests--edit fx "lisp/rlt-fn.el" (romacs-live-tests--file "rlt-fn" "(defface rlt-fn-face nil \"A face.\" :group 'romacs-live)"))
    (romacs-live-tests--update fx)
    (should (equal (get 'rlt-fn-face 'face-defface-spec) nil))))

(ert-deftest romacs-live-a-declared-struct-definer-is-checked ()
  ;; romacs-agent--defstruct wraps `cl-defstruct' and says so: a layout
  ;; change through it refuses the update as one through `cl-defstruct'.
  (romacs-live-tests--with fx
      `(("lisp/rlt-dd.el" . ,(romacs-live-tests--file "rlt-dd"
                                 "(defmacro rlt-dd--defstruct (name &rest slots) `(cl-defstruct ,name ,@slots))"
                                 "(put 'rlt-dd--defstruct 'romacs-live-definer 'cl-defstruct)"
                                 "(rlt-dd--defstruct rlt-dd-rec a b)")))
    (require 'rlt-dd)
    (romacs-live-tests--edit fx "lisp/rlt-dd.el"
                             (romacs-live-tests--file "rlt-dd"
                                 "(defmacro rlt-dd--defstruct (name &rest slots) `(cl-defstruct ,name ,@slots))"
                                 "(put 'rlt-dd--defstruct 'romacs-live-definer 'cl-defstruct)"
                                 "(rlt-dd--defstruct rlt-dd-rec a b c)"))
    (should (string-match-p "rlt-dd-rec changes its layout" (car (plist-get (romacs-live-tests--update fx) :refusals))))))

(ert-deftest romacs-live-a-dependent-reloads-from-where-it-was-loaded ()
  ;; A macro's user loaded by its own path, from no directory on `load-path'.
  (romacs-live-tests--with fx
      `(("lisp/rlt-xa.el" . ,(romacs-live-tests--file "rlt-xa" "(defmacro rlt-xa-m () 1)")))
    (require 'rlt-xa)
    (let ((user (expand-file-name "elsewhere/rlt-xb.el" (plist-get fx :top))))
      (romacs-live-tests--write user (romacs-live-tests--file "rlt-xb" "(require 'rlt-xa)" "(defun rlt-xb-f () (rlt-xa-m))"))
      (load user nil t)
      (romacs-live-tests--edit fx "lisp/rlt-xa.el" (romacs-live-tests--file "rlt-xa" "(defmacro rlt-xa-m () 2)"))
      (should-not (plist-get (romacs-live-tests--update fx) :failed))
      (should (= (funcall 'rlt-xb-f) 2)))))

(ert-deftest romacs-live-a-file-loaded-after-the-survey-is-checked ()
  ;; Staged by the plan, loaded before the apply: its layout is checked then.
  (romacs-live-tests--with fx
      `(("lisp/rlt-lt.el" . ,(romacs-live-tests--file "rlt-lt" "(cl-defstruct rlt-lt-rec a)")))
    (romacs-live-tests--edit fx "lisp/rlt-lt.el" (romacs-live-tests--file "rlt-lt" "(cl-defstruct rlt-lt-rec a b)"))
    (let* ((roots (romacs-live--lisp-roots (plist-get (romacs-live-build) :root)))
           (plan (romacs-live-plan (romacs-live-survey (romacs-live--request (list :kind 'worktree :directory (plist-get fx :src))))
                                   (romacs-live--loaded roots))))
      (should (eq (plist-get (car (plist-get plan :records)) :action) 'stage))
      (require 'rlt-lt)
      (let ((report (romacs-live-apply plan)))
        (should (string-match-p "rlt-lt-rec changes its layout" (car (plist-get report :refusals))))
        (should-not (memq 'b (mapcar #'car (cdr (cl-struct-slot-info 'rlt-lt-rec)))))))))

(ert-deftest romacs-live-a-partial-file-goes-back-to-the-build ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-pb.el" . ,(romacs-live-tests--file "rlt-pb" "(defun rlt-pb-f () 1)")))
    (require 'rlt-pb)
    (romacs-live-tests--edit fx "lisp/rlt-pb.el" (romacs-live-tests--file "rlt-pb" "(defun rlt-pb-f () 2)" "(error \"broken\")"))
    (romacs-live-tests--update fx)
    (should (= (funcall 'rlt-pb-f) 2))
    (romacs-live-tests--git (plist-get fx :src) "checkout" "--" "lisp")
    (romacs-live-tests--update fx)
    (should (= (funcall 'rlt-pb-f) 1))
    (should-not romacs-live-ledger)))

(ert-deftest romacs-live-a-quit-in-a-load-leaves-it-partial ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-qp.el" . ,(romacs-live-tests--file "rlt-qp" "(defun rlt-qp-f () 1)")))
    (require 'rlt-qp)
    (romacs-live-tests--edit fx "lisp/rlt-qp.el" (romacs-live-tests--file "rlt-qp" "(defun rlt-qp-f () 2)" "(signal 'quit nil)"))
    (let ((report (romacs-live-tests--update fx)))
      (should (equal (plist-get report :failed) "lisp/rlt-qp.el"))
      (should (eq (plist-get (car (plist-get romacs-live-ledger :entries)) :state) 'partial))
      (should-not romacs-live--running))))

(ert-deftest romacs-live-a-function-another-file-defines-is-not-gone ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ga.el" . ,(romacs-live-tests--file "rlt-ga" "(defun rlt-ga-shared () 1)" "(defun rlt-ga-f () 1)"))
        ("lisp/rlt-gc.el" . ,(romacs-live-tests--file "rlt-gc" "(defun rlt-ga-shared () 1)")))
    (require 'rlt-ga)
    (require 'rlt-gc)
    (romacs-live-tests--edit fx "lisp/rlt-ga.el" (romacs-live-tests--file "rlt-ga" "(defun rlt-ga-f () 2)"))
    (let ((report (romacs-live-tests--update fx :unbind-gone t)))
      (should-not (plist-get report :gone))
      (should (fboundp 'rlt-ga-shared)))))

(ert-deftest romacs-live-an-installation-relative-file-is-refused ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ir.el" . ,(romacs-live-tests--file "rlt-ir" "(defun rlt-ir-f () 1)")))
    (require 'rlt-ir)
    (romacs-live-tests--edit fx "lisp/rlt-ir.el" (romacs-live-tests--file "rlt-ir" "(defun rlt-ir-f () 2)"))
    (romacs-live-tests--edit fx "lisp/rlt-ir.el" (romacs-live-tests--file "rlt-ir" "(defvar rlt-ir-dir (file-name-directory (or load-file-name buffer-file-name)))" "(defun rlt-ir-f () 2)"))
    (should (string-match-p "beside it in the installation" (car (plist-get (romacs-live-tests--update fx) :refusals))))
    ;; Naming its own file is no lookup beside it.
    (romacs-live-tests--edit fx "lisp/rlt-ir.el" (romacs-live-tests--file "rlt-ir" "(defvar rlt-ir-file load-file-name)" "(defun rlt-ir-f () 3)"))
    (let ((report (romacs-live-tests--update fx)))
      (should-not (plist-get report :refusals))
      (should (plist-get report :warnings)))))

(ert-deftest romacs-live-an-outside-file-in-the-ledger-leaves-a-branch-update-alone ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-of.el" . ,(romacs-live-tests--file "rlt-of" "(defun rlt-of-f () 1)")))
    (let ((outside (expand-file-name "outside/rlt-out.el" (plist-get fx :top))) (report nil))
      (romacs-live-tests--write outside (romacs-live-tests--file "rlt-out" "(defun rlt-out-f () 1)"))
      (romacs-live-start (list :kind 'files :files (list outside)) (lambda (r) (setq report r)))
      (should (plist-get romacs-live-ledger :entries))
      (romacs-live-tests--edit fx "lisp/rlt-of.el" (romacs-live-tests--file "rlt-of" "(defun rlt-of-f () 2)"))
      (romacs-live-tests--git (plist-get fx :src) "commit" "-q" "-am" "branch")
      ;; A branch: its files are asked of `git ls-tree', which takes no
      ;; absolute path.
      (romacs-live-start (list :kind 'rev :rev "HEAD" :repository (plist-get fx :src)) (lambda (r) (setq report r)))
      (should-not (plist-get report :refusals))
      (should (romacs-live-tests--row report "lisp/rlt-of.el")))))

(ert-deftest romacs-live-a-definer-the-source-declares-is-checked ()
  ;; The session has not loaded the declaration: the source's own counts.
  (romacs-live-tests--with fx
      `(("lisp/rlt-sd.el" . ,(romacs-live-tests--file "rlt-sd"
                                 "(defmacro rlt-sd--defstruct (name &rest slots) `(cl-defstruct ,name ,@slots))"
                                 "(rlt-sd--defstruct rlt-sd-rec a b)")))
    (require 'rlt-sd)
    (romacs-live-tests--edit fx "lisp/rlt-sd.el"
                             (romacs-live-tests--file "rlt-sd"
                                 "(defmacro rlt-sd--defstruct (name &rest slots) `(cl-defstruct ,name ,@slots))"
                                 "(put 'rlt-sd--defstruct 'romacs-live-definer 'cl-defstruct)"
                                 "(rlt-sd--defstruct rlt-sd-rec a b c)"))
    (should (string-match-p "rlt-sd-rec changes its layout" (car (plist-get (romacs-live-tests--update fx) :refusals))))))

(ert-deftest romacs-live-an-after-load-load-keeps-the-files-history ()
  ;; A hook that loads another library as the file loads puts its entry
  ;; first: the file's own stays, and a function moved into it is not gone.
  (romacs-live-tests--with fx
      `(("lisp/rlt-ha.el" . ,(romacs-live-tests--file "rlt-ha" "(defun rlt-ha-moved () 1)"))
        ("lisp/rlt-hb.el" . ,(romacs-live-tests--file "rlt-hb" "(defun rlt-hb-f () 1)"))
        ("lisp/rlt-hx.el" . ,(romacs-live-tests--file "rlt-hx" "(defun rlt-hx-f () 1)")))
    (require 'rlt-ha)
    (require 'rlt-hb)
    (with-eval-after-load 'rlt-hb (load "rlt-hx" nil t))
    (romacs-live-tests--edit fx "lisp/rlt-ha.el" (romacs-live-tests--file "rlt-ha"))
    (romacs-live-tests--edit fx "lisp/rlt-hb.el" (romacs-live-tests--file "rlt-hb" "(defun rlt-hb-f () 2)" "(defun rlt-ha-moved () 2)"))
    (let ((report (romacs-live-tests--update fx :unbind-gone t)))
      (should-not (plist-get report :gone))
      (should (= (funcall 'rlt-ha-moved) 2)))))

(ert-deftest romacs-live-forms-a-quoted-lookup-is-no-lookup ()
  ;; package.el writes this form for another file to run: data, no call.
  (should-not (romacs-live-forms-beside-itself-p
               (romacs-live-forms-read "(defun f () '(file-name-directory load-file-name))")))
  (should-not (romacs-live-forms-beside-itself-p
               (romacs-live-forms-read "(defun f () `(add-to-list 'load-path (file-name-directory ,x)) load-file-name)")))
  (should (romacs-live-forms-beside-itself-p
           (romacs-live-forms-read "(load (concat (file-name-directory load-file-name) \"x\"))")))
  ;; A template's unquote runs.
  (should (romacs-live-forms-beside-itself-p
           (romacs-live-forms-read "(defvar d `(dir ,(file-name-directory load-file-name)))")))
  (should (romacs-live-forms-beside-itself-p
           (romacs-live-forms-read "(defvar d (file-name-directory (file-truename (or load-file-name buffer-file-name))))"))))

(defun romacs-live-tests--restart (ledger)
  "Replay LEDGER as a restart's successor does, through its image part's
reader; the report."
  (let ((done nil))
    (cl-letf (((symbol-function 'message) #'ignore))
      (romacs-live-restore-ledger ledger (lambda () (setq done t))))
    (should done)
    romacs-live-last-report))

(ert-deftest romacs-live-a-restart-puts-the-ledger-back ()
  ;; The successor runs the build: the ledger's files load again over it,
  ;; keymap and all, and keep where they came from.
  (romacs-live-tests--with fx
      `(("lisp/rlt-rp.el" . ,(romacs-live-tests--file "rlt-rp" "(defun rlt-rp-f () 1)")))
    (require 'rlt-rp)
    (romacs-live-tests--edit fx "lisp/rlt-rp.el" (romacs-live-tests--file "rlt-rp" "(defun rlt-rp-f () 2)"))
    (romacs-live-tests--update fx)
    (let ((ledger (copy-tree romacs-live-ledger)))
      (should (equal (romacs-live--image-part) (cons 'romacs-live-restore-ledger ledger)))
      ;; As the successor finds it: the build's code, no ledger.
      (load (expand-file-name "lisp/rlt-rp.el" (plist-get fx :build)) nil t)
      (setq romacs-live-ledger nil)
      (should (= (funcall 'rlt-rp-f) 1))
      (let ((report (romacs-live-tests--restart ledger)))
        (should (= (funcall 'rlt-rp-f) 2))
        (should-not (plist-get report :dropped))
        (should (equal (plist-get (car (plist-get romacs-live-ledger :entries)) :source)
                       (plist-get (car (plist-get ledger :entries)) :source)))))))

(ert-deftest romacs-live-a-restart-drops-what-the-build-took-or-changed ()
  (romacs-live-tests--with fx
      `(("lisp/rlt-ld.el" . ,(romacs-live-tests--file "rlt-ld" "(defun rlt-ld-f () 1)"))
        ("lisp/rlt-sp.el" . ,(romacs-live-tests--file "rlt-sp" "(defun rlt-sp-f () 1)")))
    (require 'rlt-ld)
    (require 'rlt-sp)
    (let ((landed (romacs-live-tests--file "rlt-ld" "(defun rlt-ld-f () 2)")))
      (romacs-live-tests--edit fx "lisp/rlt-ld.el" landed)
      (romacs-live-tests--edit fx "lisp/rlt-sp.el" (romacs-live-tests--file "rlt-sp" "(defun rlt-sp-f () 2)"))
      (romacs-live-tests--update fx)
      (let ((ledger (copy-tree romacs-live-ledger)))
        (setq romacs-live-ledger nil)
        ;; The new build has the first file as the ledger had it, and the
        ;; second as neither had it.
        (romacs-live-tests--write (expand-file-name "lisp/rlt-ld.el" (plist-get fx :build)) landed)
        (romacs-live-tests--write (expand-file-name "lisp/rlt-sp.el" (plist-get fx :build))
                                  (romacs-live-tests--file "rlt-sp" "(defun rlt-sp-f () 3)"))
        (let ((report (romacs-live-tests--restart ledger)))
          (should (equal (mapcar (lambda (d) (list (plist-get d :path) (plist-get d :why))) (plist-get report :dropped))
                         '(("lisp/rlt-ld.el" landed) ("lisp/rlt-sp.el" superseded))))
          (should-not romacs-live-ledger))))))

(ert-deftest romacs-live-a-restart-after-two-updates-puts-the-last-back ()
  ;; build -> B -> C, then a restart into the same build: C's base is B,
  ;; but what is compared with the build is the build's own copy.
  (romacs-live-tests--with fx
      `(("lisp/rlt-tu.el" . ,(romacs-live-tests--file "rlt-tu" "(defun rlt-tu-f () 1)")))
    (require 'rlt-tu)
    (dolist (n '(2 3))
      (romacs-live-tests--edit fx "lisp/rlt-tu.el" (romacs-live-tests--file "rlt-tu" (format "(defun rlt-tu-f () %d)" n)))
      (romacs-live-tests--update fx))
    (let ((ledger (copy-tree romacs-live-ledger)))
      (load (expand-file-name "lisp/rlt-tu.el" (plist-get fx :build)) nil t)
      (setq romacs-live-ledger nil)
      (let ((report (romacs-live-tests--restart ledger)))
        (should-not (plist-get report :dropped))
        (should (= (funcall 'rlt-tu-f) 3))))))

(ert-deftest romacs-live-a-session-with-live-files-keeps-them-over-an-image ()
  (let ((romacs-live-ledger '(:entries ((:path "lisp/x.el")))) (called nil))
    (should-error (romacs-live-restore-ledger '(:entries nil) (lambda () (setq called t))) :type 'user-error)
    (should-not called)))

(ert-deftest romacs-live-a-replay-that-puts-nothing-back-says-why ()
  (should (string-match-p "1 landed"
                          (romacs-live-summary '(:label "the ledger" :refusals ("x: a layout changes")
                                                 :dropped ((:path "a" :why landed))))))
  (should (string-match-p "1 landed, 1 superseded"
                          (romacs-live-summary '(:label "the ledger" :rows nil
                                                 :dropped ((:path "a" :why landed) (:path "b" :why superseded)))))))

(ert-deftest romacs-live-a-reset-asks-a-fresh-restart-onto-the-installed-build ()
  (require 'warm-restart)
  (let* ((asked nil)
         (installed (make-temp-file "romacs-live-installed-" t))
         (romacs-server-build-watch installed))
    (unwind-protect
        (cl-letf (((symbol-function 'warm-restart-request) (lambda (&rest args) (setq asked args)))
                  ;; The store's pruning has a test of its own, on a store of its own.
                  ((symbol-function 'romacs-live-prune-store) #'ignore))
          (romacs-session-reset)
          (should (equal (plist-get asked :fresh) 'unsaved))
          (should (equal (plist-get asked :target) installed))
          (romacs-session-reset t)
          (should (equal (plist-get asked :fresh) 'buffers))
          ;; No watched build: this executable, with its runtime, even
          ;; when the runtime root holds another one.
          (let ((romacs-server-build-watch nil))
            (cl-letf (((symbol-function 'romacs-live--runtime-root) (lambda () installed)))
              (with-temp-file (expand-file-name "romacs-vm-gui" installed) (insert "#!/bin/sh\n"))
              (set-file-modes (expand-file-name "romacs-vm-gui" installed) #o755)
              (romacs-session-reset)
              (should (equal (plist-get asked :target) (expand-file-name invocation-name invocation-directory))))))
      (delete-directory installed t))))

(ert-deftest romacs-live-a-prune-keeps-what-a-session-or-image-needs ()
  ;; Four blobs, three of them unused for ten days: the one a restart's
  ;; image names stays, the ledger's stays, the recent one stays, the
  ;; other goes.
  (let* ((top (file-name-as-directory (make-temp-file "romacs-live-prune-" t)))
         (romacs-live-directory (expand-file-name "store/" top))
         (warm-restart-directory (expand-file-name "images/" top))
         (process-environment (cons (concat "XDG_STATE_HOME=" (expand-file-name "state" top)) process-environment))
         (blob (lambda (c) (make-string 40 c)))
         (old (time-subtract nil (days-to-time 10)))
         (romacs-live-ledger (list :entries (list (list :path "lisp/l.el" :new (funcall blob ?d))))))
    (unwind-protect
        (progn
          (dolist (c '(?a ?b ?c ?d))
            (let ((dir (expand-file-name (funcall blob c) romacs-live-directory)))
              (make-directory dir t)
              (write-region "x" nil (expand-file-name "x.el" dir) nil 'silent)
              (unless (eq c ?c) (set-file-times dir old))))
          (make-directory warm-restart-directory t)
          (write-region (format "(:parts ((romacs-live-restore-ledger :entries ((:file %S)))))"
                                (expand-file-name (concat (funcall blob ?b) "/x.el") romacs-live-directory))
                        nil (expand-file-name "pending.image" warm-restart-directory) nil 'silent)
          ;; An image that cannot be read: nothing is known safe to go.
          (make-directory (expand-file-name "unreadable.image" warm-restart-directory))
          (should-not (romacs-live-prune-store))
          (should (file-directory-p (expand-file-name (funcall blob ?a) romacs-live-directory)))
          (delete-directory (expand-file-name "unreadable.image" warm-restart-directory))
          (let ((removed (romacs-live-prune-store)))
            (should (equal (mapcar #'file-name-nondirectory removed) (list (funcall blob ?a))))
            (should (file-directory-p (expand-file-name (funcall blob ?b) romacs-live-directory)))
            (should (file-directory-p (expand-file-name (funcall blob ?c) romacs-live-directory)))
            (should (file-directory-p (expand-file-name (funcall blob ?d) romacs-live-directory)))))
      (delete-directory top t))))

(ert-deftest romacs-live-forms-diff-names-each-definition ()
  (let ((diff (romacs-live-forms-diff
               (romacs-live-forms-read "(defun a () 1) (defvar b 1) (defun c () 1) (message \"x\")")
               (romacs-live-forms-read "(defun a () 2) (defvar b 1) (defmacro d () 1) (message \"y\")"))))
    (should (equal (plist-get diff :changed) '((function a))))
    (should (equal (plist-get diff :added) '((macro d))))
    (should (equal (plist-get diff :removed) '((function c))))
    (should (= (plist-get diff :others) 2))
    (should (romacs-live-forms-macro-change-p (list :changed '((macro d)))))))

(provide 'romacs-live-tests)
;;; romacs-live-tests.el ends here
