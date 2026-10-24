;;; c-variables-oracle.el --- GNU transcripts for tests/c_variables.rs  -*- lexical-binding: t -*-
;; Run once per section, each in a FRESH GNU process (variables are global
;; state; tests/c_variables.rs replays each section in a fresh VM):
;;   for s in object bool int per-buffer; do
;;     HOME=$(mktemp -d) CVO_SECTION=$s /usr/bin/emacs -Q --batch \
;;       -l c-variables-oracle.el > c-variables-oracle-$s.txt; done
;; tests/c_variables.rs reads those transcripts with include_str! and compares
;; every line; no expected value is written by hand.  Forms are evaluated
;; interpreted (`eval FORM t').
(defun cvo (form)
  (princ (format "%S\n  => %S\n" form
                 (condition-case err (list 'ok (eval form t)) (error (list 'signal err))))))
(princ (format ";; %s %s section %s\n" emacs-version emacs-repository-version (getenv "CVO_SECTION")))
(defconst cvo-sections '(
  (object
   ;; Object kind: minibuffer-local-map (keymap.c:3362), the chain frontier.
   (list (boundp 'minibuffer-local-map) (special-variable-p 'minibuffer-local-map) (keymapp minibuffer-local-map) (keymapp (symbol-value 'minibuffer-local-map)) (eq minibuffer-local-map (default-value 'minibuffer-local-map)))
   (let ((map (make-sparse-keymap))) (set-keymap-parent map minibuffer-local-map) (eq (keymap-parent map) minibuffer-local-map))
   (list (condition-case e (makunbound 'minibuffer-local-map) (error e)) (boundp 'minibuffer-local-map) (keymapp minibuffer-local-map))
   (let ((saved minibuffer-local-map)) (list (let ((minibuffer-local-map 5)) (list minibuffer-local-map (symbol-value 'minibuffer-local-map) (default-value 'minibuffer-local-map))) (eq minibuffer-local-map saved)))
   (let ((saved minibuffer-local-map)) (list (condition-case e (let ((minibuffer-local-map 'inner)) (signal 'error (list minibuffer-local-map))) (error e)) (eq minibuffer-local-map saved)))
   (let ((saved minibuffer-local-map)) (list (set 'minibuffer-local-map 'x) minibuffer-local-map (keymapp (set-default 'minibuffer-local-map saved)) (eq minibuffer-local-map saved)))
   (defvar minibuffer-local-map 'ignored)
   (keymapp minibuffer-local-map)
   (defvaralias 'minibuffer-local-map 'cvo-base)
   (progn (defvaralias 'cvo-mlm-alias 'minibuffer-local-map) (list (eq cvo-mlm-alias minibuffer-local-map) (special-variable-p 'cvo-mlm-alias)))
   (list system-type (symbol-value 'system-type)))
  (bool
   ;; Bool kind: enable-recursive-minibuffers (minibuf.c).
   (list enable-recursive-minibuffers (setq enable-recursive-minibuffers 5) enable-recursive-minibuffers (symbol-value 'enable-recursive-minibuffers))
   (list (set 'enable-recursive-minibuffers nil) enable-recursive-minibuffers (set-default 'enable-recursive-minibuffers "s") (default-value 'enable-recursive-minibuffers))
   (progn (setq enable-recursive-minibuffers nil) (list (let ((enable-recursive-minibuffers 'yes)) enable-recursive-minibuffers) enable-recursive-minibuffers))
   (list (condition-case e (makunbound 'enable-recursive-minibuffers) (error e)) enable-recursive-minibuffers))
  (int
   ;; Int kind: undo-limit (undo.c:437).
   (list undo-limit (boundp 'undo-limit) (special-variable-p 'undo-limit))
   (list (condition-case e (setq undo-limit 'x) (error e)) undo-limit)
   (list (condition-case e (set 'undo-limit 1.5) (error e)) undo-limit)
   (list (condition-case e (set-default 'undo-limit "s") (error e)) undo-limit)
   (list (condition-case e (let ((undo-limit nil)) undo-limit) (error e)) undo-limit)
   (list (let ((undo-limit 7)) (list undo-limit (default-value 'undo-limit))) undo-limit)
   (list (condition-case e (let ((undo-limit 7)) (signal 'error (list undo-limit))) (error e)) undo-limit)
   (list (setq undo-limit -3) undo-limit (setq undo-limit 160000))
   (list (condition-case e (makunbound 'undo-limit) (error e)) undo-limit))
  (per-buffer
   ;; Per-buffer kind: fill-column (buffer.c:5259, predicate integerp).
   (list fill-column (default-value 'fill-column) (local-variable-if-set-p 'fill-column))
   (with-temp-buffer (list (condition-case e (setq fill-column 'x) (error e)) fill-column))
   (with-temp-buffer (list (setq fill-column nil) fill-column (default-value 'fill-column)))
   (with-temp-buffer (setq fill-column 33) (list fill-column (default-value 'fill-column) (with-temp-buffer fill-column)))
   (with-temp-buffer (let ((fill-column 44)) (list fill-column (default-value 'fill-column) (with-temp-buffer fill-column))))
   (list (with-temp-buffer (let ((fill-column 44)) (setq fill-column 45) (list fill-column (default-value 'fill-column)))) fill-column)
   (with-temp-buffer (setq fill-column 33) (list (let ((fill-column 44)) (list fill-column (default-value 'fill-column) (with-temp-buffer fill-column))) fill-column))
   (with-temp-buffer (list (condition-case e (let ((fill-column 44)) (signal 'error (list fill-column (default-value 'fill-column)))) (error e)) fill-column (default-value 'fill-column)))
   (with-temp-buffer (setq fill-column 33) (list (condition-case e (let ((fill-column 'bad)) fill-column) (error e)) fill-column))
   (with-temp-buffer (list (let ((fill-column 'bad)) (list fill-column (default-value 'fill-column))) fill-column))
   (list (with-temp-buffer (set-default 'fill-column 'y) (list fill-column (with-temp-buffer fill-column))) (default-value 'fill-column) (set-default 'fill-column 70))
   (with-temp-buffer (setq fill-column 33) (set-default 'fill-column 55) (list fill-column (default-value 'fill-column) (with-temp-buffer fill-column) (set-default 'fill-column 70)))
   (with-temp-buffer (make-local-variable 'fill-column) (list (set-default 'fill-column 56) fill-column (set-default 'fill-column 70)))
   (with-temp-buffer (list (condition-case e (setq fill-column 'x) (error e)) (progn (set-default 'fill-column 57) fill-column) (set-default 'fill-column 70)))
   (list (condition-case e (makunbound 'fill-column) (error e)) fill-column)
   (defvaralias 'fill-column 'cvo-base2)
   (progn (defvaralias 'cvo-fc-alias 'fill-column) (with-temp-buffer (setq cvo-fc-alias 61) (list fill-column cvo-fc-alias (default-value 'fill-column))))
   (eq (make-variable-buffer-local 'fill-column) 'fill-column))))
(mapc #'cvo (cdr (assq (intern (getenv "CVO_SECTION")) cvo-sections)))
