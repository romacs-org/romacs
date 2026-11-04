;;; romacs-complete.el --- Vertical minibuffer completion in a layer  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Portions Copyright (C) 2021-2026 Free Software Foundation, Inc.,
;; from vertico.el by Daniel Mendler, kept as the GPL requires.

;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Keywords: convenience, files, matching, completion

;; Romacs: derived from vertico.el by Daniel Mendler
;; (https://github.com/minad/vertico), vertico commit c60d802 ("README:
;; Remove old section about Tramp"), version 2.15, with the sorting of
;; extensions/vertico-sort.el and the commands of
;; extensions/vertico-directory.el from the same commit.  What changed
;; is marked "Romacs:" below.

;; This program is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Romacs: vertico's completion UI, shown in a layer over the frame
;; instead of in the minibuffer window's overlays.  With
;; `romacs-complete-mode' on, the frame reserves no minibuffer line: the
;; minibuffer window is hosted by a child frame (`romacs--host-minibuffer'),
;; which shows the prompt and input and, for `completing-read', the
;; candidates below them in a floating glass panel, centred across the
;; frame near its top, its middle or its bottom (`romacs-complete-position'
;; places it and changes nothing else).  The minibuffer
;; stays GNU's buffer and window, with its contents, prompt, point, hooks,
;; history and recursion; only where it is drawn changes.  Messages show
;; as toasts in the frame's top-right corner and stay in *Messages*
;; (DIVERGENCES.md VM2-MINI-COMPLETION-LAYER, VM2-ECHO-LAYER).
;;
;; Candidates come from `completion-all-completions' over the table and
;; predicate the reader was given, with `completion-styles' (orderless
;; where configured, romacs-orderless.el), as vertico computes them.
;; The provided feature is `romacs-complete', so a vertico the user
;; installs is a separate package; enabling it turns this mode off.

;;; Code:

(eval-when-compile
  (require 'cl-lib)
  (require 'subr-x))
(require 'romacs-faces)
(require 'romacs-layer)

(defgroup romacs-complete nil
  "Vertical minibuffer completion in a layer over the frame."
  :group 'minibuffer
  :prefix "romacs-complete-")

(defcustom romacs-complete-count-format (cons "%-6s " "%s/%s")
  "Format string used for the candidate count."
  :type '(choice (const :tag "No candidate count" nil) (cons string string)))

(defcustom romacs-complete-group-format
  (concat #("    " 0 4 (face romacs-complete-group-separator))
          #(" %s " 0 4 (face romacs-complete-group-title))
          #(" " 0 1 (face romacs-complete-group-separator)))
  "Format string used for the group title."
  :type '(choice (const :tag "No group titles" nil) string))

;; Romacs: 12 compact rows by default (vertico: 10).
(defcustom romacs-complete-count 12
  "Maximal number of candidates to show."
  :type 'natnum)

(defcustom romacs-complete-preselect 'directory
  "Configure if the prompt or first candidate is preselected.
- prompt: Always select the prompt.
- first: Select the first candidate, allow prompt selection.
- no-prompt: Like first, but forbid selection of the prompt entirely.
- directory: Like first, but select the prompt if it is a directory."
  :type '(choice (const prompt) (const first) (const no-prompt) (const directory)))

(defcustom romacs-complete-scroll-margin 2
  "Number of lines at the top and bottom when scrolling.
The value should lie between 0 and romacs-complete-count/2."
  :type 'natnum)

(defcustom romacs-complete-cycle nil
  "Enable cycling for `romacs-complete-next' and `romacs-complete-previous'."
  :type 'boolean)

(defcustom romacs-complete-multiline
  (cons #("↲" 0 1 (face romacs-complete-multiline)) #("…" 0 1 (face romacs-complete-multiline)))
  "Replacements for multiline strings."
  :type '(cons (string :tag "Newline") (string :tag "Truncation")))

(defcustom romacs-complete-sort-function #'romacs-complete-sort-history-length-alpha
  "Default sorting function, used if no `display-sort-function' is specified."
  :type '(choice
          (const :tag "No sorting" nil)
          (const :tag "By history, length and alpha" romacs-complete-sort-history-length-alpha)
          (const :tag "By length and alpha" romacs-complete-sort-length-alpha)
          (function :tag "Custom function")))

(defcustom romacs-complete-sort-override-function nil
  "Override sort function which overrides the `display-sort-function'."
  :type '(choice (const nil) function))

;; Romacs: one panel, as Zed's file finder shows one (its proportions:
;; about 11% down, 42% wide); the position moves it and nothing else.
(defcustom romacs-complete-position 'top
  "Where the minibuffer's panel shows, vertically.
`top': near the frame's top.  `center': centred on the frame.
`bottom': near its bottom, above the mode line of the window the
minibuffer was entered from.  The panel is the same in each: centred
across the frame, `romacs-complete-center-width' of it wide, glass
\(`romacs-layer-glass'), its input on top and its candidates below.
Its input stays where the tallest panel would start, so it grows
downward as candidates come and go."
  :type '(choice (const top) (const center) (const bottom)))

(defcustom romacs-complete-center-width 0.42
  "Width of the centred panel, as a fraction of the frame's width."
  :type 'number)

(defgroup romacs-complete-faces nil
  "Faces used by `romacs-complete-mode'."
  :group 'romacs-complete
  :group 'faces)

(defface romacs-complete-multiline '((t :inherit shadow))
  "Face used to highlight multiline replacement characters.")

(defface romacs-complete-group-title '((t :inherit shadow :slant italic))
  "Face used for the title text of the candidate group headlines.")

(defface romacs-complete-group-separator '((t :inherit romacs-complete-group-title :strike-through t))
  "Face used for the separator lines of the candidate groups.")

;; Romacs: the look of a picker's rows, after Zed's file finder (crates/picker,
;; crates/file_finder, crates/ui ListItem and HighlightedLabel at 5a5fbe79;
;; Apache-2.0 or GPL-3.0; ideas and values only, no code): the selected row a
;; subtle tinted band, rounded and inset (the host draws it from the
;; `romacs-layer-selection' parameter), matched characters in the accent
;; colour, a file's directory muted beside its name.  The row, match and
;; prompt faces are romacs-faces.el's; a theme colours them.
(defface romacs-complete-directory '((t :inherit shadow))
  "Face of the directory part of a file candidate, shown after its name;
the name keeps the default face.")

(defvar-keymap romacs-complete-map
  :doc "Minibuffer keymap of `romacs-complete-mode', derived from `minibuffer-local-map'."
  :parent minibuffer-local-map
  "<remap> <beginning-of-buffer>" #'romacs-complete-first
  "<remap> <minibuffer-beginning-of-buffer>" #'romacs-complete-first
  "<remap> <end-of-buffer>" #'romacs-complete-last
  "<remap> <scroll-down-command>" #'romacs-complete-scroll-down
  "<remap> <scroll-up-command>" #'romacs-complete-scroll-up
  "<remap> <next-line>" #'romacs-complete-next
  "<remap> <previous-line>" #'romacs-complete-previous
  "<remap> <next-line-or-history-element>" #'romacs-complete-next
  "<remap> <previous-line-or-history-element>" #'romacs-complete-previous
  "<remap> <backward-paragraph>" #'romacs-complete-previous-group
  "<remap> <forward-paragraph>" #'romacs-complete-next-group
  "<remap> <exit-minibuffer>" #'romacs-complete-directory-enter
  "<remap> <kill-ring-save>" #'romacs-complete-save
  "M-RET" #'romacs-complete-exit-input
  "TAB" #'romacs-complete-insert
  ;; Romacs: vertico-directory's file name editing, on by default.
  "DEL" #'romacs-complete-directory-delete-char
  "M-DEL" #'romacs-complete-directory-delete-word)

(defvar romacs-complete--locals
  '((scroll-margin . 0)
    (completion-auto-help . nil)
    ;; Romacs: the layer grows with the input, never the frame's windows.
    (truncate-lines . nil))
  "Minibuffer local variables of `romacs-complete-mode'.")

(defvar-local romacs-complete--hilit #'identity
  "Lazy candidate highlighting function.")

(defvar-local romacs-complete--count-ov nil
  "Overlay showing the number of candidates.")

(defvar-local romacs-complete--index -1
  "Index of current candidate or negative for prompt selection.")

(defvar-local romacs-complete--scroll 0
  "Scroll position.")

(defvar-local romacs-complete--input nil
  "Cons of last minibuffer contents and point or t.")

(defvar-local romacs-complete--candidates nil
  "List of candidates.")

(defvar-local romacs-complete--metadata nil
  "Completion metadata.")

(defvar-local romacs-complete--base ""
  "Base string, which is concatenated with the candidate.")

(defvar-local romacs-complete--total 0
  "Length of the candidate list `romacs-complete--candidates'.")

(defvar-local romacs-complete--lock-candidate nil
  "Lock-in current candidate.")

(defvar-local romacs-complete--lock-groups nil
  "Lock-in current group order.")

(defvar-local romacs-complete--groups nil
  "List of current group titles.")

(defvar-local romacs-complete--allow-prompt nil
  "Prompt selection is allowed.")

;; Romacs: the rows shown, for the layer's size and for agents and tests.
(defvar-local romacs-complete--current-line nil
  "The row of the shown candidates the current one is on, or nil.")

(defvar-local romacs-complete--lines nil
  "The candidate rows last shown.")

(defun romacs-complete--affixate (cands)
  "Annotate CANDS with annotation function."
  (if-let* ((aff (romacs-complete--metadata-get 'affixation-function)))
      (funcall aff cands)
    (if-let* ((ann (romacs-complete--metadata-get 'annotation-function)))
        (cl-loop for cand in cands collect
                 (let ((suff (or (funcall ann cand) "")))
                   ;; The default completion UI adds the `completions-annotations'
                   ;; face if no other faces are present.
                   (unless (text-property-not-all 0 (length suff) 'face nil suff)
                     (setq suff (propertize suff 'face 'completions-annotations)))
                   (list cand "" suff)))
      (cl-loop for cand in cands collect (list cand "" "")))))

(defun romacs-complete--move-to-front (elem list)
  "Move all ELEM (also duplicates) to front of LIST."
  (if (member elem list)
      (nconc (cl-loop for x in list if (equal x elem) collect x)
             (delete elem list))
    list))

(defun romacs-complete--metadata-get (prop)
  "Return PROP from completion metadata."
  (completion-metadata-get romacs-complete--metadata prop))

(defun romacs-complete--sort-function ()
  "Return the sorting function."
  (or romacs-complete-sort-override-function
      (romacs-complete--metadata-get 'display-sort-function)
      romacs-complete-sort-function))

(defun romacs-complete--compute (input)
  "Compute state given INPUT."
  (pcase-let* ((`(,str . ,pt) input)
               (table minibuffer-completion-table)
               ;; Romacs: the reader's predicate itself, never wrapped, so a
               ;; file table sees `file-exists-p' or `file-directory-p'.
               (pred minibuffer-completion-predicate)
               (before (substring str 0 pt))
               (after (substring str pt))
               ;; bug#47678: `completion-boundaries' fails for `partial-completion'
               ;; if the cursor is moved before the slashes of "~//".
               (bounds (condition-case nil
                           (completion-boundaries before table pred after)
                         (t (cons 0 (length after)))))
               (field (substring str (car bounds) (+ pt (cdr bounds))))
               ;; bug#75910: category instead of `minibuffer-completing-file-name'
               (completing-file (eq 'file (romacs-complete--metadata-get 'category)))
               (`(,all . ,hl)
                (dlet ((completion-lazy-hilit t) (completion-lazy-hilit-fn nil))
                  (cons (completion-all-completions str table pred pt romacs-complete--metadata)
                        completion-lazy-hilit-fn)))
               (base (or (when-let* ((z (last all))) (prog1 (cdr z) (setcdr z nil))) 0))
               (romacs-complete--base (substring str 0 base))
               (def (or (car-safe minibuffer-default) minibuffer-default))
               (rm minibuffer--require-match)
               (groups) (def-missing) (allow-prompt) (lock))
    ;; Filter the ignored file extensions. We cannot use modified predicate for
    ;; this filtering, since this breaks the special casing in the
    ;; `completion-file-name-table' for `file-exists-p' and `file-directory-p'.
    (when completing-file
      (let ((exact (and (not (equal field "")) (member field all))))
        (setq all (romacs-complete--filename-try-filter all))
        (and exact (not (member field all)) (push field all))))
    ;; Sort using the `display-sort-function' or the sort functions here
    (setq all (funcall (or (romacs-complete--sort-function) #'identity) all))
    ;; Move special candidates: "field" appears at the top, before "field/", before default value
    (when (and (stringp def) (not (equal def "")))
      (setq all (romacs-complete--move-to-front def all)))
    (unless (equal field "")
      (when (and completing-file (not (string-suffix-p "/" field)))
        (setq all (romacs-complete--move-to-front (concat field "/") all)))
      (setq all (romacs-complete--move-to-front field all)))
    (setq all (delete-consecutive-dups all))
    (when-let* ((fun (and all (romacs-complete--metadata-get 'group-function))))
      (setq groups (romacs-complete--group-by fun all) all (car groups)))
    (setq def-missing (and def (equal str "") (not (member def all)))
          allow-prompt (and (not (eq romacs-complete-preselect 'no-prompt))
                            (or def-missing (eq romacs-complete-preselect 'prompt)
                                (functionp rm)
                                (memq rm '(nil confirm confirm-after-completion))
                                (test-completion str table pred)))
          lock (and romacs-complete--lock-candidate ;; Locked position of old candidate.
                    (if (< romacs-complete--index 0) (and allow-prompt -1)
                      (seq-position all (nth romacs-complete--index romacs-complete--candidates)))))
    `((romacs-complete--input . ,input)
      (romacs-complete--base . ,romacs-complete--base)
      (romacs-complete--metadata . ,romacs-complete--metadata)
      (romacs-complete--candidates . ,all)
      (romacs-complete--total . ,(length all))
      (romacs-complete--hilit . ,(or hl #'identity))
      (romacs-complete--allow-prompt . ,allow-prompt)
      (romacs-complete--lock-candidate . ,lock)
      (romacs-complete--groups . ,(cdr groups))
      (romacs-complete--index . ,(or lock
                                     (if (or def-missing (eq romacs-complete-preselect 'prompt) (not all)
                                             (and completing-file (eq romacs-complete-preselect 'directory)
                                                  (= (length romacs-complete--base) (length str))
                                                  (test-completion str table pred)))
                                         -1 0))))))

;; Romacs: vertico calls `completion-pcm--filename-try-filter', which
;; builds its regexp with `regexp-opt' and matches every candidate on
;; every key; the candidates of one directory are the same strings from
;; key to key (romacs-orderless.el keeps the listing), so each is matched
;; once per minibuffer.
(defvar-local romacs-complete--ignored nil
  "(EXTENSIONS CASE-FOLD REGEXP MEMO): the regexp of
`completion-pcm--filename-try-filter' for `completion-ignored-extensions'
EXTENSIONS and `case-fold-search' CASE-FOLD, and whether each candidate
string (by identity) matched it.")

(defun romacs-complete--filename-try-filter (all)
  "`completion-pcm--filename-try-filter' of ALL: the candidates that are
not `./', `../' or end in an ignored extension, or ALL if none is left."
  (when all
    (unless (and romacs-complete--ignored
                 (equal (nth 0 romacs-complete--ignored) completion-ignored-extensions)
                 (eq (nth 1 romacs-complete--ignored) case-fold-search))
      (setq romacs-complete--ignored
            (list (copy-sequence completion-ignored-extensions) case-fold-search
                  (concat "\\(?:\\`\\.\\.?/\\|" (regexp-opt completion-ignored-extensions) "\\)\\'")
                  (make-hash-table :test #'eq))))
    (pcase-let ((`(,_ ,_ ,re ,memo) romacs-complete--ignored) (try nil))
      (dolist (f all)
        (unless (let ((known (gethash f memo 'unknown)))
                  (if (eq known 'unknown) (puthash f (string-match-p re f) memo) known))
          (push f try)))
      (or (nreverse try) all))))

(defun romacs-complete--hilit (cand)
  "Highlight CAND string with lazy highlighting."
  ;; bug#77754: Highlight unquoted string.
  (romacs-complete--accent
   (funcall romacs-complete--hilit (substring (or (get-text-property
                                                   0 'completion--unquoted cand) cand)))))

;; Romacs: matched characters in one face, whichever style matched them.
(defun romacs-complete--match-face-p (face)
  "Whether FACE is a completion style's face for matched characters."
  (and (symbolp face)
       (or (memq face '(romacs-fzf-match completions-common-part))
           (string-prefix-p "orderless-match-face-" (symbol-name face)))))

(defun romacs-complete--accent (str)
  "STR with the style's match faces replaced by `romacs-complete-match'."
  (let ((pos 0) (len (length str)))
    (while (< pos len)
      (let* ((next (next-single-property-change pos 'face str len))
             (faces (ensure-list (get-text-property pos 'face str))))
        (when (seq-some #'romacs-complete--match-face-p faces)
          (put-text-property pos next 'face
                             (cons 'romacs-complete-match (seq-remove #'romacs-complete--match-face-p faces))
                             str))
        (setq pos next)))
    str))

;; Romacs: a file candidate shown as a file finder shows it, its name
;; first and its directory muted after it.  Only the display changes: the
;; candidate itself is what RET and TAB take.
(defun romacs-complete--dim-directory (cand)
  "CAND, a file name, shown as its last component (a trailing separator
kept) then its directory in `romacs-complete-directory'."
  (let ((slash (string-match-p "/[^/]+/?\\'" cand)))
    (if (not slash)
        cand
      (let ((dir (substring cand 0 (1+ slash))))
        (add-face-text-property 0 (length dir) 'romacs-complete-directory 'append dir)
        (concat (substring cand (1+ slash)) "  " dir)))))

(defun romacs-complete--cycle (list n)
  "Rotate LIST to position N."
  (nconc (copy-sequence (nthcdr n list)) (seq-take list n)))

(defun romacs-complete--group-by (fun elems)
  "Group ELEMS by FUN."
  (let ((ht (make-hash-table :test #'equal)) titles groups)
    ;; Build hash table of groups
    (cl-loop for elem on elems
             for title = (funcall fun (car elem) nil) do
             (if-let* ((group (gethash title ht)))
                 (setcdr group (setcdr (cdr group) elem)) ;; Append to tail of group
               (puthash title (cons elem elem) ht) ;; New group element (head . tail)
               (push title titles)))
    (setq titles (nreverse titles))
    ;; Cycle groups if `romacs-complete--lock-groups' is set
    (when-let* ((group (seq-find (lambda (group) (gethash group ht))
                                 romacs-complete--lock-groups)))
      (setq titles (romacs-complete--cycle titles (seq-position titles group))))
    ;; Build group list
    (dolist (title titles)
      (push (gethash title ht) groups))
    ;; Unlink last tail
    (setcdr (cdar groups) nil)
    (setq groups (nreverse groups))
    ;; Link groups
    (let ((link groups))
      (while (cdr link)
        (setcdr (cdar link) (caadr link))
        (pop link)))
    (cons (caar groups) titles)))

(defun romacs-complete--remote-p (path)
  "Return t if PATH is a remote path."
  (string-match-p "\\`/[^/|:]+:" (substitute-in-file-name path)))

(defun romacs-complete--update (&optional interruptible)
  "Update state, optionally INTERRUPTIBLE."
  (let* ((pt (max 0 (- (point) (minibuffer-prompt-end))))
         (str (minibuffer-contents-no-properties))
         (input (cons str pt)))
    (unless (or (and interruptible (input-pending-p)) (equal romacs-complete--input input))
      ;; Romacs: no redisplay before the candidates are computed (vertico
      ;; shows the input first, gh:minad/vertico#89): a redisplay here
      ;; paints and waits for the frame, which costs a key more than the
      ;; computation; `while-no-input' still lets typing interrupt it.
      (pcase (let ((romacs-complete--metadata (completion-metadata (substring str 0 pt)
                                                                   minibuffer-completion-table
                                                                   minibuffer-completion-predicate)))
               ;; If Tramp is used, do not compute the candidates in an
               ;; interruptible fashion, since this will break the Tramp
               ;; password and user name prompts (See gh:minad/vertico#23).
               (if (or (not interruptible)
                       (and (eq 'file (romacs-complete--metadata-get 'category))
                            (or (romacs-complete--remote-p str) (romacs-complete--remote-p default-directory))))
                   (romacs-complete--compute input)
                 (let ((non-essential t))
                   (while-no-input (romacs-complete--compute input)))))
        ('nil (abort-recursive-edit))
        ((and state (pred consp))
         (dolist (s state) (set (car s) (cdr s))))))))

(defun romacs-complete--display-string (str)
  "Return display STR without display and invisible properties."
  (let ((end (length str)) (pos 0) chunks)
    (while (< pos end)
      (let ((nextd (next-single-property-change pos 'display str end))
            (disp (get-text-property pos 'display str)))
        (if (stringp disp)
            (let ((face (get-text-property pos 'face str)))
              (when face
                (add-face-text-property
                 0 (length disp) face t (setq disp (concat disp))))
              (setq pos nextd chunks (cons disp chunks)))
          (pcase disp (`(space :align-to . ,_) (push " " chunks)))
          (while (< pos nextd)
            (let ((nexti
                   (next-single-property-change pos 'invisible str nextd)))
              (unless (or (get-text-property pos 'invisible str)
                          (and (= pos 0) (= nexti end))) ;; full=>no allocation
                (push (substring str pos nexti) chunks))
              (setq pos nexti))))))
    (if chunks (apply #'concat (nreverse chunks)) str)))

;; Romacs: the rows are as wide as the layer's candidate window.
(defun romacs-complete--window-width ()
  "The width of the window showing the candidates, in columns."
  (let ((frame (romacs-complete--frame)))
    (window-width (frame-root-window frame))))

(defun romacs-complete--truncate-multiline (str max)
  "Truncate multiline STR to MAX."
  (let ((pos 0) (res ""))
    (while (and (< (length res) (* 2 max)) (string-match "\\(\\S-+\\)\\|\\s-+" str pos))
      (setq res (concat res (if (match-end 1) (match-string 0 str)
                              (if (string-search "\n" (match-string 0 str))
                                  (car romacs-complete-multiline) " ")))
            pos (match-end 0)))
    (truncate-string-to-width (string-trim res) max 0 nil (cdr romacs-complete-multiline))))

(defun romacs-complete--compute-scroll ()
  "Compute new scroll position."
  (let ((off (max (min romacs-complete-scroll-margin (/ romacs-complete-count 2)) 0))
        (corr (if (= romacs-complete-scroll-margin (/ romacs-complete-count 2)) (1- (mod romacs-complete-count 2)) 0)))
    (setq romacs-complete--scroll (min (max 0 (- romacs-complete--total romacs-complete-count))
                                       (max 0 (+ romacs-complete--index off 1 (- romacs-complete-count))
                                            (min (- romacs-complete--index off corr) romacs-complete--scroll))))))

(defun romacs-complete--format-group-title (title cand)
  "Format group TITLE given the current CAND."
  ;; Copy candidate highlighting if title is a prefix of the candidate.
  (when (string-prefix-p title cand)
    (setq title (substring cand 0 (length title)))
    (romacs-complete--remove-face 0 (length title) 'completions-first-difference title))
  (setq title (substring title))
  (add-face-text-property 0 (length title) 'romacs-complete-group-title t title)
  (format (concat romacs-complete-group-format "\n") title))

(defun romacs-complete--format-count ()
  "Format the count string."
  (format (car romacs-complete-count-format)
          (format (cdr romacs-complete-count-format)
                  (cond ((>= romacs-complete--index 0) (1+ romacs-complete--index))
                        (romacs-complete--allow-prompt "*")
                        (t "!"))
                  romacs-complete--total)))

(defun romacs-complete--display-count ()
  "Update count overlay `romacs-complete--count-ov'."
  (move-overlay romacs-complete--count-ov (point-min) (point-min))
  (overlay-put romacs-complete--count-ov 'before-string
               (if romacs-complete-count-format
                   (propertize (romacs-complete--format-count) 'face 'romacs-complete-prompt)
                 "")))

(defun romacs-complete--prompt-selection ()
  "Highlight the prompt if selected."
  (let ((inhibit-modification-hooks t))
    (if (and (< romacs-complete--index 0) romacs-complete--allow-prompt)
        (add-face-text-property (minibuffer-prompt-end) (point-max) 'romacs-complete-current 'append)
      (romacs-complete--remove-face (minibuffer-prompt-end) (point-max) 'romacs-complete-current))))

(defun romacs-complete--remove-face (beg end face &optional obj)
  "Remove FACE between BEG and END from OBJ."
  (while (< beg end)
    (let ((next (next-single-property-change beg 'face obj end)))
      (when-let* ((val (get-text-property beg 'face obj)))
        (put-text-property beg next 'face (remq face (ensure-list val)) obj))
      (setq beg next))))

(defun romacs-complete--debug (&rest _)
  "Debugger used by `romacs-complete--protect'."
  (let ((inhibit-message t))
    (require 'backtrace)
    (declare-function backtrace-to-string "backtrace")
    (message "romacs-complete detected an error:\n%s" (backtrace-to-string)))
  (let (message-log-max)
    (message "%s %s"
             (propertize "romacs-complete detected an error:" 'face 'error)
             (substitute-command-keys "Press \\[view-echo-area-messages] to see the stack trace")))
  nil)

(defun romacs-complete--protect (fun)
  "Protect FUN such that errors are caught.
The stack trace of an error is shown in the *Messages* buffer."
  (ignore-errors
    (handler-bind ((error #'romacs-complete--debug))
      (funcall fun))))

(defun romacs-complete--exhibit ()
  "Exhibit completion UI."
  (romacs-complete--protect
   (lambda ()
     (let ((buffer-undo-list t)) ;; Overlays affect point position and undo list!
       (romacs-complete--update 'interruptible)
       (romacs-complete--prompt-selection)
       (romacs-complete--display-count)
       ;; Romacs: as many rows as the panel has room for, below the
       ;; input as it wraps at the panel's width.
       (romacs--resize-mini-window)
       (let ((romacs-complete-count (romacs-complete--rows)))
         (romacs-complete--display-candidates (romacs-complete--arrange-candidates)))))))

(defun romacs-complete--goto (index)
  "Go to candidate with INDEX."
  (setq romacs-complete--index
        (max (if (or romacs-complete--allow-prompt (= 0 romacs-complete--total)) -1 0)
             (min index (1- romacs-complete--total)))
        romacs-complete--lock-candidate (or (>= romacs-complete--index 0) romacs-complete--allow-prompt)))

(defun romacs-complete--candidate (&optional hl)
  "Return current candidate string with optional highlighting if HL is non-nil."
  (let ((content (or (car-safe romacs-complete--input) (minibuffer-contents-no-properties))))
    (cond
     ((>= romacs-complete--index 0)
      (let ((cand (substring (nth romacs-complete--index romacs-complete--candidates))))
        ;; XXX Drop the completions-common-part face which is added by the
        ;; `completion--twq-all' hack.  This should better be fixed in Emacs
        ;; itself, the corresponding code is already marked as fixme.
        (romacs-complete--remove-face 0 (length cand) 'completions-common-part cand)
        (concat romacs-complete--base (if hl (romacs-complete--hilit cand) cand))))
     ((and (equal content "") (or (car-safe minibuffer-default) minibuffer-default)))
     (t content))))

(defun romacs-complete--match-p (input)
  "Return t if INPUT is a valid match."
  (let ((rm minibuffer--require-match))
    (or (memq rm '(nil confirm-after-completion))
        (equal "" input) ;; Null completion, returns default value
        (if (functionp rm) (funcall rm input) ;; require-match can be a function
          (test-completion input minibuffer-completion-table minibuffer-completion-predicate))
        (if (eq rm 'confirm) (eq (ignore-errors (read-char "Confirm")) 13)
          (minibuffer-message "Match required") nil))))

(defun romacs-complete--format-candidate (cand prefix suffix index _start)
  "Format CAND given PREFIX, SUFFIX and INDEX."
  (when (memq (romacs-complete--metadata-get 'category) '(file project-file))
    (setq cand (romacs-complete--dim-directory (copy-sequence cand))))
  (setq cand (romacs-complete--display-string (concat prefix cand suffix "\n")))
  (when (= index romacs-complete--index)
    (add-face-text-property 0 (length cand) 'romacs-complete-current 'append cand))
  cand)

(defun romacs-complete--arrange-candidates ()
  "Arrange candidates."
  (romacs-complete--compute-scroll)
  (let ((curr-line 0) lines)
    ;; Compute group titles
    (let* (title (index romacs-complete--scroll)
           (group-fun (and romacs-complete-group-format (romacs-complete--metadata-get 'group-function)))
           (candidates
            (romacs-complete--affixate
             (cl-loop repeat romacs-complete-count for c in (nthcdr index romacs-complete--candidates)
                      collect (romacs-complete--hilit c)))))
      (pcase-dolist ((and cand `(,str . ,_)) candidates)
        (when-let* ((new-title (and group-fun (funcall group-fun str nil))))
          (unless (equal title new-title)
            (setq title new-title)
            (push (romacs-complete--format-group-title title str) lines))
          (setcar cand (funcall group-fun str 'transform)))
        (when (= index romacs-complete--index)
          (setq curr-line (length lines)))
        (push (cons index cand) lines)
        (incf index)))
    ;; Drop excess lines
    (setq lines (nreverse lines))
    (cl-loop for count from (length lines) above romacs-complete-count do
             (if (< curr-line (/ count 2))
                 (nbutlast lines)
               (setq curr-line (1- curr-line) lines (cdr lines))))
    ;; Romacs: the row the host draws the selection band on.
    (setq romacs-complete--current-line (and (>= romacs-complete--index 0) curr-line))
    ;; Format candidates
    (let ((max-width (- (romacs-complete--window-width) 4)) start)
      (cl-loop for line on lines do
               (pcase (car line)
                 (`(,index ,cand ,prefix ,suffix)
                  (setq start (or start index))
                  (when (string-search "\n" cand)
                    (setq cand (romacs-complete--truncate-multiline cand max-width)))
                  (setcar line (romacs-complete--format-candidate cand prefix suffix index start))))))
    lines))

;; Romacs: the rows go into the layer's candidate buffer; vertico puts
;; them in an overlay of the minibuffer and grows its window.
(defun romacs-complete--display-candidates (lines)
  "Show LINES in the minibuffer's layer."
  (setq romacs-complete--lines lines)
  (let ((buffer (romacs-complete--candidates-buffer)))
    (with-current-buffer buffer
      (let ((inhibit-read-only t) (inhibit-modification-hooks t))
        (erase-buffer)
        (apply #'insert lines)
        (when lines (delete-char -1))
        (goto-char (point-min))))
    (romacs-complete--show buffer (length lines) romacs-complete--current-line)))

(defun romacs-complete--prepare ()
  "Ensure that the state is prepared before running the next command."
  (when-let* ((cmd (and (symbolp this-command) (symbol-name this-command)))
              ((string-prefix-p "romacs-complete-" cmd))
              ((not (and romacs-complete--metadata (string-prefix-p "romacs-complete-directory-" cmd)))))
    (romacs-complete--update)))

(defun romacs-complete--setup ()
  "Setup completion UI."
  (remove-hook 'minibuffer-setup-hook #'romacs-complete--setup)
  ;; Romacs: icomplete, when the user enables it, owns its minibuffers.
  (unless (bound-and-true-p icomplete-mode)
    (dolist (var romacs-complete--locals)
      (set (make-local-variable (car var)) (cdr var)))
    (setq-local romacs-complete--input t
                romacs-complete--count-ov (make-overlay (point-min) (point-min) nil t t))
    ;; Romacs: the prompt's label recedes; the input is what the panel shows.
    (let ((inhibit-read-only t))
      (add-face-text-property (point-min) (minibuffer-prompt-end) 'romacs-complete-prompt))
    (overlay-put romacs-complete--count-ov 'priority 1) ;; For `minibuffer-depth-indicate-mode'
    (use-local-map romacs-complete-map)
    (add-hook 'pre-command-hook #'romacs-complete--prepare nil 'local)
    (add-hook 'post-command-hook #'romacs-complete--exhibit nil 'local)))

(defun romacs-complete--advice (&rest app)
  "Advice for completion function, apply APP."
  (unwind-protect ;; Do not use `minibuffer-setup-hook' for idempotency.
      (dlet ((completion-eager-display nil))
        (add-hook 'minibuffer-setup-hook #'romacs-complete--setup)
        (apply app))
    (remove-hook 'minibuffer-setup-hook #'romacs-complete--setup)))

(defun romacs-complete-first ()
  "Go to first candidate, or to the prompt when the first candidate is selected."
  (interactive)
  (romacs-complete--goto (if (> romacs-complete--index 0) 0 -1)))

(defun romacs-complete-last ()
  "Go to last candidate."
  (interactive)
  (romacs-complete--goto (1- romacs-complete--total)))

(defun romacs-complete-scroll-down (&optional n)
  "Go back by N pages."
  (interactive "p")
  (romacs-complete--goto (max 0 (- romacs-complete--index (* (or n 1) (romacs-complete--rows))))))

(defun romacs-complete-scroll-up (&optional n)
  "Go forward by N pages."
  (interactive "p")
  (romacs-complete-scroll-down (- (or n 1))))

(defun romacs-complete-next (&optional n)
  "Go forward N candidates."
  (interactive "p")
  (let ((index (+ romacs-complete--index (or n 1))))
    (romacs-complete--goto
     (cond
      ((not romacs-complete-cycle) index)
      ((= romacs-complete--total 0) -1)
      (romacs-complete--allow-prompt (1- (mod (1+ index) (1+ romacs-complete--total))))
      (t (mod index romacs-complete--total))))))

(defun romacs-complete-previous (&optional n)
  "Go backward N candidates."
  (interactive "p")
  (romacs-complete-next (- (or n 1))))

(defun romacs-complete-exit (&optional arg)
  "Exit minibuffer with current candidate or input if prefix ARG is given."
  (interactive "P")
  (when (and (not arg) (>= romacs-complete--index 0))
    (romacs-complete-insert))
  (when (romacs-complete--match-p (minibuffer-contents-no-properties))
    (exit-minibuffer)))

(defun romacs-complete-next-group (&optional n)
  "Cycle N groups forward.
When the prefix argument is 0, the group order is reset."
  (interactive "p")
  (when (cdr romacs-complete--groups)
    (setq romacs-complete--groups (and (not (eq n 0))
                                       (romacs-complete--cycle romacs-complete--groups
                                                               (let ((len (length romacs-complete--groups)))
                                                                 (- len (mod (- (or n 1)) len)))))
          romacs-complete--lock-groups romacs-complete--groups
          romacs-complete--lock-candidate nil
          romacs-complete--input nil)))

(defun romacs-complete-previous-group (&optional n)
  "Cycle N groups backward.
When the prefix argument is 0, the group order is reset."
  (interactive "p")
  (romacs-complete-next-group (- (or n 1))))

(defun romacs-complete-exit-input ()
  "Exit minibuffer with input."
  (interactive)
  (romacs-complete-exit t))

(defun romacs-complete-save ()
  "Save current candidate to kill ring."
  (interactive)
  (if (or (use-region-p) (not transient-mark-mode))
      (call-interactively #'kill-ring-save)
    (kill-new (substring-no-properties (romacs-complete--candidate)))))

(defun romacs-complete-insert ()
  "Insert current candidate in minibuffer."
  (interactive)
  ;; XXX There is a small bug here, depending on interpretation. When completing
  ;; "~/emacs/master/li|/calc" where "|" is the cursor, then the returned
  ;; candidate only includes the prefix "~/emacs/master/lisp/", but not the
  ;; suffix "/calc". Default completion has the same problem when selecting in
  ;; the *Completions* buffer. See bug#48356.
  (when (> romacs-complete--total 0)
    (let ((romacs-complete--index (max 0 romacs-complete--index)))
      (insert (prog1 (romacs-complete--candidate) (delete-minibuffer-contents))))))

;;;; Sorting (vertico-sort.el)

(defvar-local romacs-complete-sort--history nil
  "History hash table and corresponding base string.")

(defcustom romacs-complete-sort-history-duplicate 10
  "History position shift for duplicate history elements.
The more often a duplicate element occurs in the history, the earlier it
appears in the completion list.  The shift decays exponentially with
`romacs-complete-sort-history-decay'.  Note that duplicates occur only if
`history-delete-duplicates' is disabled."
  :type 'number)

(defcustom romacs-complete-sort-history-decay 10
  "Exponential decay for the position shift of duplicate elements.
The shift will decay away after `romacs-complete-sort-history-duplicate' times
`romacs-complete-sort-history-decay' history elements."
  :type 'number)

(defun romacs-complete-sort--history ()
  "Recompute history hash table and return it."
  (or (and (equal (car romacs-complete-sort--history) romacs-complete--base) (cdr romacs-complete-sort--history))
      (let* ((base romacs-complete--base)
             (base-len (length base))
             (hist (and (not (eq minibuffer-history-variable t)) ;; Disabled for `t'.
                        (symbol-value minibuffer-history-variable)))
             (ht (make-hash-table :test #'equal :size (length hist)))
             (file-p (and (> base-len 0) ;; Step-wise completion, unlike `project-find-file'
                          (eq minibuffer-history-variable 'file-name-history)))
             (curr-file (when-let* ((win (and file-p (minibuffer-selected-window)))
                                    (file (buffer-file-name (window-buffer win))))
                          (abbreviate-file-name file)))
             (decay (/ -1.0 (* romacs-complete-sort-history-duplicate romacs-complete-sort-history-decay))))
        (cl-loop for elem in hist for idx from 0 do
                 (when (and (not (equal curr-file elem)) ;; Deprioritize current file
                            (or (= base-len 0)
                                (and (>= (length elem) base-len)
                                     (eq t (compare-strings base 0 base-len elem 0 base-len)))))
                   (let ((file-sep (and file-p (string-search "/" elem base-len))))
                     ;; Drop base string from history elements & special file handling.
                     (when (or (> base-len 0) file-sep)
                       (setq elem (substring elem base-len (and file-sep (1+ file-sep)))))
                     (let ((r (if-let* ((r (gethash elem ht)))
                                  ;; Reduce duplicate rank with exponential decay.
                                  (- r (round (* romacs-complete-sort-history-duplicate
                                                 (exp (* decay idx)))))
                                ;; Never outrank the most recent element.
                                (if (= idx 0) (/ most-negative-fixnum 2) idx))))
                       (puthash elem r ht)))))
        (cdr (setq romacs-complete-sort--history (cons base ht))))))

;; Romacs: vertico's bucket sorts by length then `string<' are
;; `romacs--sort-by-length-alpha', native (romacs-orderless.el's).
(defun romacs-complete-sort--length-alpha (list)
  "LIST sorted by length, then `string<'."
  (if (fboundp 'romacs--sort-by-length-alpha)
      (romacs--sort-by-length-alpha list)
    (sort list (lambda (x y) (or (< (length x) (length y))
                                 (and (= (length x) (length y)) (string< x y)))))))

(defun romacs-complete-sort-history-length-alpha (candidates)
  "Sort CANDIDATES by history, length, alpha."
  (let ((hhash (romacs-complete-sort--history)) hcands rest)
    (dolist (cand candidates)
      (if-let* ((idx (gethash cand hhash))) (push (cons idx cand) hcands) (push cand rest)))
    (setq hcands (sort hcands #'car-less-than-car))
    (cl-loop for item on hcands do (setcar item (cdar item)))
    (nconc hcands (romacs-complete-sort--length-alpha (nreverse rest)))))

(defun romacs-complete-sort-length-alpha (candidates)
  "Sort CANDIDATES by length, alpha."
  (romacs-complete-sort--length-alpha candidates))

;;;; File names (vertico-directory.el)

(defun romacs-complete-directory-enter (&optional arg)
  "Enter directory or exit completion with current candidate.
Exit with current input if prefix ARG is given."
  (interactive "P")
  ;; Romacs: RET runs this in every minibuffer, and `romacs-complete--prepare'
  ;; leaves directory commands to the state the last key left; keys typed
  ;; faster than candidates compute leave that state behind the input.
  (romacs-complete--update)
  (if-let* (((not arg))
            ((>= romacs-complete--index 0))
            ((eq 'file (romacs-complete--metadata-get 'category)))
            ;; Check romacs-complete--base for stepwise file path completion
            ((not (equal romacs-complete--base "")))
            (cand (romacs-complete--candidate))
            ((or (string-suffix-p "/" cand)
                 (and (romacs-complete--remote-p cand)
                      (string-suffix-p ":" cand))))
            ;; Handle /./ and /../ manually instead of via `expand-file-name'
            ;; and `abbreviate-file-name', such that we don't accidentally
            ;; perform unwanted substitutions in the existing completion.
            ((progn
               (setq cand (string-replace "/./" "/" cand))
               (unless (string-suffix-p "/../../" cand)
                 (setq cand (replace-regexp-in-string "/[^/|:]+/\\.\\./\\'" "/" cand)))
               (not (equal (minibuffer-contents-no-properties) cand)))))
      (progn
        (delete-minibuffer-contents)
        (insert cand))
    (romacs-complete-exit arg)))

(defun romacs-complete-directory-up (&optional n)
  "Delete N names before point."
  (interactive "p")
  (when (and (> (point) (minibuffer-prompt-end))
             (eq 'file (romacs-complete--metadata-get 'category)))
    (let ((path (buffer-substring-no-properties (minibuffer-prompt-end) (point)))
          found)
      (when (string-match-p "\\`~[^/]*/\\'" path)
        (delete-minibuffer-contents)
        (insert (expand-file-name path)))
      (dotimes (_ (or n 1) found)
        (save-excursion
          (let ((end (point)))
            (goto-char (1- end))
            (when (search-backward "/" (minibuffer-prompt-end) t)
              (delete-region (1+ (point)) end)
              (setq found t))))))))

(defun romacs-complete-directory-delete-char (n)
  "Delete N directories or chars before point."
  (interactive "p")
  (unless (and (not (and (use-region-p) delete-active-region (= n 1)))
               (eq (char-before) ?/) (romacs-complete-directory-up n))
    (with-no-warnings (delete-backward-char n))))

(defun romacs-complete-directory-delete-word (n)
  "Delete N directories or words before point."
  (interactive "p")
  (unless (and (eq (char-before) ?/) (romacs-complete-directory-up n))
    (delete-region (prog1 (point) (backward-word n)) (point))))

(defun romacs-complete-directory-tidy ()
  "Tidy shadowed file names, see `rfn-eshadow-overlay'."
  (when (and romacs-complete-mode (eq this-command #'self-insert-command))
    (dolist (ov '(tramp-rfn-eshadow-overlay rfn-eshadow-overlay))
      (when (and (boundp ov)
                 (setq ov (symbol-value ov))
                 (overlay-buffer ov)
                 (= (point) (point-max))
                 (> (point) (overlay-end ov)))
        (delete-region (overlay-start ov) (overlay-end ov))))))

;;;; The minibuffer's layer (Romacs)

(defvar romacs-complete--layer nil
  "The child frame hosting the minibuffer window while the mode is on.")

(defun romacs-complete--candidates-buffer ()
  "The buffer showing the candidates of the current minibuffer depth."
  (let ((buffer (get-buffer-create (format " *romacs-complete-%d*" (max 1 (minibuffer-depth))))))
    (with-current-buffer buffer
      (setq-local truncate-lines t mode-line-format nil header-line-format nil
                  cursor-type nil cursor-in-non-selected-windows nil
                  ;; A truncated candidate ends in an ellipsis, not `$'.
                  buffer-display-table (romacs-layer-display-table)))
    buffer))

(defun romacs-complete--frame ()
  "The child frame that hosts the minibuffer window, made once."
  (unless (frame-live-p romacs-complete--layer)
    (let* ((root (romacs-layer--root-frame))
           (after-make-frame-functions nil)
           (frame (make-frame `((parent-frame . ,root)
                                (minibuffer . ,(minibuffer-window root))
                                (visibility . nil)
                                ;; It opens from its top and grows downward.
                                (romacs-layer-edge . top)
                                ,@romacs-layer-frame-parameters)))
           (window (frame-root-window frame)))
      (set-window-buffer window (romacs-complete--candidates-buffer))
      (set-window-parameter window 'no-other-window t)
      (set-window-parameter window 'no-delete-other-windows t)
      (setq romacs-complete--layer frame)))
  (romacs-complete--look romacs-complete--layer)
  (romacs--host-minibuffer romacs-complete--layer)
  romacs-complete--layer)

(defun romacs-complete--panel-width (room column)
  "The floating panel's text width in pixels, with ROOM pixels across the
frame and COLUMN-pixel characters: `romacs-complete-center-width' of the
room, at least 60 columns, never more than the room: a narrow frame or a
large font gets a panel as wide as the frame."
  (min room (max (* 60 column)
                 (min (- room (* 4 column))
                      (round (* romacs-complete-center-width room))))))

(declare-function romacs-chrome-bottom "romacs-chrome" (&optional frame except))
(declare-function romacs-chrome-top "romacs-chrome" (&optional frame))

(defun romacs-complete--mode-line-top (root)
  "The top of the mode line of the window the minibuffer was entered
from, in ROOT's native pixels, where child frames are placed; or where a
layer anchored at ROOT's bottom ends when that window is elsewhere
\(`romacs-chrome-bottom', the panel itself excepted)."
  (let ((window (minibuffer-selected-window)))
    (if (and window (eq (window-frame window) root))
        (- (nth 3 (window-edges window nil nil t)) (window-mode-line-height window)
           (window-bottom-divider-width window))
      (romacs-chrome-bottom root romacs-complete--layer))))

(defun romacs-complete--panel-top (root tallest ch)
  "The panel's outer top in ROOT for `romacs-complete-position', the
panel TALLEST pixels at most, with CH-pixel lines."
  (let ((height (frame-text-height root)) (inset (frame-internal-border-width root)))
    (pcase romacs-complete-position
      ('center (+ inset (/ (- height tallest) 2)))
      ('bottom (- (romacs-complete--mode-line-top root) ch tallest))
      ;; Below a status bar at the top (`romacs-chrome-top') by a line.
      (_ (max (+ (romacs-chrome-top root) ch) (+ inset (max (* 2 ch) (round (* 0.11 height)))))))))

(defun romacs-complete--rows ()
  "Most candidate rows the panel shows: `romacs-complete-count', fewer
at `bottom' where the panel would otherwise reach the mode line it ends
above, but at least one."
  (if (not (eq romacs-complete-position 'bottom))
      romacs-complete-count
    (let* ((frame (romacs-complete--frame)) (root (frame-parent frame))
           (ch (frame-char-height root))
           (room (- (romacs-complete--mode-line-top root) ch
                    (window-pixel-height (minibuffer-window root))
                    (* 2 (or (frame-parameter frame 'child-frame-border-width) 0)))))
      (max 1 (min romacs-complete-count (/ room ch))))))

(defun romacs-complete--placement (frame rows)
  "Where FRAME goes with ROWS candidate rows below the minibuffer
window: (LEFT TOP WIDTH HEIGHT) in pixels, the outer position in the
root frame and the text size.  It is placed at its tallest, so it grows
downward and its input stays put."
  (let* ((root (frame-parent frame))
         (border (or (frame-parameter frame 'child-frame-border-width) 0))
         (ch (frame-char-height root)) (cw (frame-char-width root))
         (mini (window-pixel-height (minibuffer-window root)))
         (room (- (frame-text-width root) (* 2 border)))
         (width (romacs-complete--panel-width room cw))
         (tallest (+ (* (romacs-complete--rows) ch) mini (* 2 border))))
    ;; Centred on the frame: child frames are placed from its native
    ;; origin, outside the internal border round the windows.
    (list (/ (- (frame-native-width root) width (* 2 border)) 2)
          (max 0 (romacs-complete--panel-top root tallest ch))
          width (+ (* rows ch) mini))))

(defvar romacs-complete--looks nil
  "The frame and glass setting the layer's look was last made for.")

(defun romacs-complete--look (frame)
  "Make FRAME glass (`romacs-layer-glass'), else GNU's square border,
when the setting changed."
  (let ((look (list frame romacs-layer-glass)))
    (unless (equal look romacs-complete--looks)
      (setq romacs-complete--looks look)
      (modify-frame-parameters frame (romacs-layer-glass-parameters romacs-layer-glass)))))

(defun romacs-complete--place (frame rows)
  "Move and size FRAME for ROWS candidate rows (`romacs-complete--placement')."
  (pcase-let ((`(,left ,top ,width ,height) (romacs-complete--placement frame rows)))
    (unless (equal (frame-position frame) (cons left top))
      (set-frame-position frame left top))
    (unless (and (= (frame-text-width frame) width) (= (frame-text-height frame) height))
      (set-frame-size frame width height t))))

(defun romacs-complete--show (buffer rows &optional selection)
  "Show the minibuffer's layer with BUFFER's ROWS candidate rows,
SELECTION the row of the current one, if any."
  (let* ((frame (romacs-complete--frame))
         (window (frame-root-window frame)))
    (unless (eq (frame-parameter frame 'romacs-layer-selection) selection)
      (set-frame-parameter frame 'romacs-layer-selection selection))
    (unless (eq (window-buffer window) buffer)
      (set-window-buffer window buffer))
    (romacs-complete--place frame rows)
    ;; The input wraps at the panel's width: place it again for the
    ;; minibuffer window's new height, before redisplay draws it.
    (when (romacs--resize-mini-window) (romacs-complete--place frame rows))
    (set-window-start window (with-current-buffer buffer (point-min)))
    (unless (frame-visible-p frame) (make-frame-visible frame))))

(defun romacs-complete--minibuffer-setup ()
  "Show the minibuffer's layer for a minibuffer without candidates."
  (unless (local-variable-p 'romacs-complete--input)
    (let ((buffer (romacs-complete--candidates-buffer)))
      (with-current-buffer buffer (let ((inhibit-read-only t)) (erase-buffer)))
      (romacs-complete--show buffer 0))))

(defun romacs-complete--minibuffer-exit ()
  "Hide the minibuffer's layer as the outermost minibuffer exits; else
show the enclosing minibuffer's candidates again."
  (when (frame-live-p romacs-complete--layer)
    (if (<= (minibuffer-depth) 1)
        (make-frame-invisible romacs-complete--layer)
      (let ((outer (get-buffer (format " *romacs-complete-%d*" (1- (minibuffer-depth))))))
        (when outer
          (romacs-complete--show outer (with-current-buffer outer
                                         (if (= (buffer-size) 0) 0 (count-lines (point-min) (point-max))))))))))

;;;; Messages (Romacs): toasts (romacs-layer.el)

(defvar romacs-echo-area-functions nil
  "Functions the host calls with the echo area's new message, a string,
or nil once the echo area is cleared.")

(defun romacs-complete--message (text)
  "Show TEXT, the echo area's new message, as a toast.  The echo area's
clearing at the next command leaves a toast to its own timeout: it
covers nothing that needs it gone."
  (when (and text (not (equal text "")))
    (romacs-layer-toast text)))

(defun romacs-complete--hide-message ()
  "Hide every message toast."
  (dolist (toast romacs-layer--toasts)
    (cancel-timer (nth 2 toast))
    (romacs-layer--toast-expire (car toast))))

;;;; The mode

(defun romacs-complete--command-p (_sym buffer)
  "Return non-nil if `romacs-complete-mode' is active in BUFFER."
  (buffer-local-value 'romacs-complete--input buffer))

;;;###autoload
(define-minor-mode romacs-complete-mode
  "Vertical minibuffer completion, with the minibuffer shown in a layer.
The frame then reserves no minibuffer line: the prompt, the input and
the candidates show in a panel, a child frame over the frame's windows,
placed by `romacs-complete-position', and messages show briefly as toasts in the
frame's top-right corner (`romacs-layer-toast').  Turning it on turns `fido-mode' and `fido-vertical-mode' off."
  :global t :group 'romacs-complete
  (dolist (fun '(completing-read-default completing-read-multiple))
    (if romacs-complete-mode
        (advice-add fun :around #'romacs-complete--advice)
      (advice-remove fun #'romacs-complete--advice)))
  (cond
   (romacs-complete-mode
    (dolist (mode '(fido-vertical-mode fido-mode icomplete-vertical-mode icomplete-mode))
      (when (and (boundp mode) (symbol-value mode))
        (funcall mode -1)
        (message "romacs-complete-mode turned %s off" mode)))
    (add-hook 'minibuffer-setup-hook #'romacs-complete--minibuffer-setup)
    (add-hook 'minibuffer-exit-hook #'romacs-complete--minibuffer-exit)
    (add-hook 'romacs-echo-area-functions #'romacs-complete--message)
    (add-hook 'rfn-eshadow-update-overlay-hook #'romacs-complete-directory-tidy)
    (romacs-complete--frame))
   (t
    (remove-hook 'minibuffer-setup-hook #'romacs-complete--minibuffer-setup)
    (remove-hook 'minibuffer-exit-hook #'romacs-complete--minibuffer-exit)
    (remove-hook 'romacs-echo-area-functions #'romacs-complete--message)
    (remove-hook 'rfn-eshadow-update-overlay-hook #'romacs-complete-directory-tidy)
    (romacs-complete--hide-message)
    (romacs--host-minibuffer nil)
    (when (frame-live-p romacs-complete--layer) (delete-frame romacs-complete--layer))
    (setq romacs-complete--layer nil))))

;; Do not show the mode's commands in M-X
(dolist (sym '( romacs-complete-next romacs-complete-next-group romacs-complete-previous
                romacs-complete-previous-group romacs-complete-scroll-down romacs-complete-scroll-up
                romacs-complete-exit romacs-complete-insert romacs-complete-exit-input
                romacs-complete-save romacs-complete-first romacs-complete-last
                romacs-complete-directory-up romacs-complete-directory-enter
                romacs-complete-directory-delete-char romacs-complete-directory-delete-word))
  (put sym 'completion-predicate #'romacs-complete--command-p))

(provide 'romacs-complete)
;;; romacs-complete.el ends here
