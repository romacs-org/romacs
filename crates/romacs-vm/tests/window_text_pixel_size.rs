//! window-text-pixel-size, buffer-text-pixel-size and posn-at-point
//! against pinned GNU in batch. GNU measures on its initial terminal
//! frame, 80 columns by 25 lines of 1x1 characters; the VM boots with
//! that geometry and GNU's Lisp as the package ERT runner does. Each case
//! runs in a fresh buffer shown in the selected window, from one Lisp
//! expression per line.
use romacs_vm::Vm;
use std::path::Path;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const SETUP: &str = r#"(progn (let ((ignore-window-parameters t)) (delete-other-windows)) (setq truncate-partial-width-windows 50) (dolist (b (quote ("*w*" "*side*"))) (when (get-buffer b) (kill-buffer b))) (set-buffer (get-buffer-create "*w*")) (set-window-buffer nil (current-buffer)))"#;

const CASES: &str = r###"
(list (window-text-pixel-size) (progn (insert "abc") (window-text-pixel-size)) (progn (insert "\n") (window-text-pixel-size)))
(progn (insert "abc\nde\n\n") (setq mode-line-format "%-") (list (window-text-pixel-size) (window-text-pixel-size nil nil t) (window-text-pixel-size nil nil nil nil nil t) (window-text-pixel-size nil nil nil nil nil nil t) (window-text-pixel-size nil nil nil nil nil 'mode-line) (window-text-pixel-size nil nil nil nil nil 'header-line)))
(mapcar (lambda (n) (erase-buffer) (insert (make-string n ?x)) (list n (window-text-pixel-size) (progn (insert "\n") (window-text-pixel-size)) (progn (insert "y") (window-text-pixel-size)))) '(0 1 78 79 80 81 157 158 159 160 237 238))
(progn (insert (make-string 200 ?x)) (list (window-text-pixel-size nil nil nil t) (window-text-pixel-size nil nil nil 1000) (window-text-pixel-size nil nil nil 3) (window-text-pixel-size nil nil nil 0) (window-text-pixel-size nil nil nil -5) (window-text-pixel-size nil nil nil 'x)))
(progn (insert (make-string 200 ?x)) (setq truncate-lines t) (list (window-text-pixel-size) (window-text-pixel-size nil nil nil 1000) (window-text-pixel-size nil nil nil 50)))
(progn (insert "a\tb\n\C-a\n\177x\n") (list (window-text-pixel-size) (progn (setq ctl-arrow nil) (window-text-pixel-size)) (progn (setq tab-width 4) (window-text-pixel-size))))
(progn (insert "日本語\néé́z") (list (window-text-pixel-size) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 5 8)))
(progn (insert "ab\ncdef\ngh") (list (window-text-pixel-size nil 2 6) (window-text-pixel-size nil 4 5) (window-text-pixel-size nil 5 5) (window-text-pixel-size nil 3 3) (window-text-pixel-size nil '(4 . 0)) (window-text-pixel-size nil '(4 . 1)) (window-text-pixel-size nil '(8 . -1)) (window-text-pixel-size nil '(1 . 5)) (window-text-pixel-size nil '(11 . -20))))
(progn (insert "  \n\n  ab\n cd  \n \n\n") (list (window-text-pixel-size nil t t) (window-text-pixel-size nil t nil) (window-text-pixel-size nil nil t)))
(progn (insert " \n  λx  \nμ \t\n \n") (list (window-text-pixel-size nil t t) (window-text-pixel-size nil t) (window-text-pixel-size nil nil t) (progn (erase-buffer) (insert "λ") (window-text-pixel-size nil t t)) (progn (insert " \t ") (goto-char 1) (insert " \n") (window-text-pixel-size nil t t))))
(progn (insert "ab\ncdef\ngh\nij\nkl") (list (window-text-pixel-size nil nil nil nil 2) (window-text-pixel-size nil nil nil 2 nil) (window-text-pixel-size nil nil nil 2 3 t) (window-text-pixel-size nil nil nil nil 0) (window-text-pixel-size nil nil nil nil 'x) (window-text-pixel-size nil nil nil nil 3 t t)))
(progn (insert "a\C-ab") (list (window-text-pixel-size nil 1 2) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 2 3) (window-text-pixel-size nil 2 4)))
(progn (insert "a\tb") (list (window-text-pixel-size nil 1 2) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 2 3) (window-text-pixel-size nil 3 4)))
(progn (insert "\n\n\n") (list (window-text-pixel-size nil 1 1) (window-text-pixel-size nil 2 2) (window-text-pixel-size nil 2 3) (window-text-pixel-size nil 1 4) (window-text-pixel-size nil 1 4 nil nil nil t) (window-text-pixel-size nil 4 4)))
(progn (insert "ab") (list (window-text-pixel-size nil 1 3 nil nil nil t) (window-text-pixel-size nil 1 2 nil nil nil t) (window-text-pixel-size nil 3 1) (window-text-pixel-size nil 0 99) (window-text-pixel-size nil (copy-marker 2) (point-max-marker))))
(list (condition-case e (window-text-pixel-size nil "x") (error e)) (condition-case e (window-text-pixel-size nil '(1 . x)) (error e)) (condition-case e (window-text-pixel-size nil nil 'x) (error e)) (condition-case e (window-text-pixel-size 'w) (error e)) (condition-case e (window-text-pixel-size nil (make-marker)) (error e)))
(progn (insert "abc" (propertize "hidden" 'invisible t) "de\nfg") (list (window-text-pixel-size) (window-text-pixel-size nil 1 5) (window-text-pixel-size nil 4 10) (window-text-pixel-size nil 5 12)))
(progn (insert "abc" (propertize "hidden" 'invisible 'h) "de") (add-to-invisibility-spec '(h . t)) (list (window-text-pixel-size) (window-text-pixel-size nil 1 5) (window-text-pixel-size nil 1 10) (window-text-pixel-size nil 4 10)))
(progn (insert "one\n" (propertize "two\nthree\n" 'invisible t) "four") (list (window-text-pixel-size) (window-text-pixel-size nil 5 15) (window-text-pixel-size nil 15)))
(progn (insert "ab" (propertize "XYZ" 'display "disp") "cd") (list (window-text-pixel-size) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 1 4) (window-text-pixel-size nil 4 6) (window-text-pixel-size nil 3 6) (window-text-pixel-size nil 6 8)))
(progn (insert "ab" (propertize "X" 'display "line1\nline2") "cd") (list (window-text-pixel-size) (window-text-pixel-size nil 3 4) (window-text-pixel-size nil 4)))
(progn (insert "a" (propertize " " 'display '(space :align-to 10)) "b" (propertize "x" 'display '(space :width 3.7)) (propertize "x" 'display '(space :width (5))) (propertize "y" 'display '(space))) (window-text-pixel-size))
(progn (insert "a" (propertize " " 'display '(space :align-to right)) "b\n" (propertize " " 'display '(space :align-to (- right 3))) "c\n" (propertize " " 'display '(space :width text)) "d") (list (window-text-pixel-size) (window-text-pixel-size nil 1 4) (window-text-pixel-size nil 5 7)))
(progn (insert "ab" (propertize "c" 'display '(height 2.0)) (propertize "d" 'display '(raise 0.5)) (propertize "e" 'display '(image :type png :file "x.png")) (propertize "f" 'display '((margin left-margin) "M")) (propertize "g" 'display '(left-fringe right-arrow))) (list (window-text-pixel-size) (condition-case e nil (error e))))
(progn (insert "abcdef") (let ((o (make-overlay 3 5))) (overlay-put o 'before-string "BEFORE") (overlay-put o 'after-string "AF")) (list (window-text-pixel-size) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 3 4) (window-text-pixel-size nil 3 5) (window-text-pixel-size nil 5 6) (window-text-pixel-size nil 4 5)))
(progn (insert "abcdef") (let ((o (make-overlay 3 5)) (p (make-overlay 3 3))) (overlay-put o 'before-string "B1") (overlay-put p 'before-string "B2\nxx") (overlay-put p 'priority 5) (overlay-put o 'invisible t) (overlay-put o 'after-string "A1")) (list (window-text-pixel-size) (window-text-pixel-size nil 1 4) (window-text-pixel-size nil 4 7)))
(progn (insert "abcdef") (let ((o (make-overlay 2 4))) (overlay-put o 'display "Z") (overlay-put o 'before-string "<<")) (list (window-text-pixel-size) (window-text-pixel-size nil 2 4) (window-text-pixel-size nil 3 5)))
(progn (insert "x" (propertize "ov" 'invisible t) "y") (let ((o (make-overlay 2 4))) (overlay-put o 'before-string "S")) (list (window-text-pixel-size) (window-text-pixel-size nil 2 4)))
(progn (insert (propertize "abc" 'display (propertize "D" 'display "nested")) (propertize "q" 'display '(space :relative-width 3)) (propertize "\t" 'display '(space :relative-width 2))) (window-text-pixel-size))
(progn (insert "abcdef") (let ((o (make-overlay 2 3))) (overlay-put o 'before-string (concat "x" (propertize "hidden" 'invisible t) (propertize "y" 'display "DD") (propertize "z" 'display '(space :width 4))))) (window-text-pixel-size))
(progn (setq word-wrap t) (insert "the quick brown fox jumps over the lazy dog the quick brown fox jumps over the lazy dog and more words here") (list (window-text-pixel-size) (window-text-pixel-size nil 70 90) (window-text-pixel-size nil 1 80) (window-text-pixel-size nil nil nil 20)))
(progn (setq word-wrap t) (insert (make-string 100 ?a) " " (make-string 30 ?b) "\n" (make-string 79 ?c) " d") (list (window-text-pixel-size) (window-text-pixel-size nil nil nil 40)))
(progn (insert (make-string 78 ?a) "日本" (make-string 3 ?b) "\t" (make-string 70 ?c) "\tz") (list (window-text-pixel-size) (window-text-pixel-size nil 78 82) (window-text-pixel-size nil 80 85)))
(progn (setq line-prefix "> ") (insert "abc\n" (make-string 100 ?x) "\n") (list (window-text-pixel-size) (progn (setq wrap-prefix "..") (window-text-pixel-size)) (progn (setq wrap-prefix '(space :width 5)) (window-text-pixel-size))))
(progn (insert (propertize "abc" 'line-prefix "PRE") "\n" (propertize (make-string 90 ?y) 'wrap-prefix (propertize "W" 'invisible t))) (list (window-text-pixel-size) (window-text-pixel-size nil 5)))
(progn (insert "abc d­e​f‑g") (list (window-text-pixel-size) (let ((nobreak-char-display nil)) (window-text-pixel-size)) (let ((nobreak-char-display 'x)) (window-text-pixel-size))))
(progn (insert "a\u0085b\U000E0080c") (list (window-text-pixel-size) (let ((display-raw-bytes-as-hex t)) (window-text-pixel-size))))
(progn (insert "ab\ncd") (setq buffer-display-table (make-display-table)) (aset buffer-display-table ?a [?x ?y ?z]) (aset buffer-display-table ?c []) (set-display-table-slot buffer-display-table 'selective-display [?! ?!]) (list (window-text-pixel-size) (window-text-pixel-size nil 1 2) (progn (insert (propertize "inv" 'invisible 'q)) (add-to-invisibility-spec '(q . t)) (window-text-pixel-size))))
(progn (insert "abc") (narrow-to-region 2 3) (list (window-text-pixel-size) (window-text-pixel-size nil 1 9) (window-text-pixel-size nil t t)))
(progn (insert "abc") (with-temp-buffer (insert "longer text here") (list (window-text-pixel-size) (buffer-text-pixel-size) (buffer-text-pixel-size "*w*") (buffer-text-pixel-size nil nil 4) (buffer-text-pixel-size nil nil nil nil 3 7))))
(list (posn-at-point) (posn-at-point 1) (posn-at-point nil (selected-window)) (posn-at-point 1 (minibuffer-window)) (condition-case e (posn-at-point "x") (error e)) (condition-case e (posn-at-point 1 'w) (error e)))
(progn (insert "abc") (let ((o (make-overlay 1 2))) (overlay-put o 'display "X") (overlay-put o 'invisible t)) (list (window-text-pixel-size) (progn (put-text-property 2 3 'display '(when t . "Q")) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '(when nil . "Q")) (window-text-pixel-size))))
(progn (insert "abcdef") (let ((o (make-overlay 2 4))) (overlay-put o 'before-string "WIN") (overlay-put o 'window (split-window))) (list (window-text-pixel-size) (window-text-pixel-size (next-window))))
(progn (insert (make-string 100 ?a)) (let ((w (split-window-right))) (list (window-body-width) (window-text-pixel-size) (window-text-pixel-size w) (progn (setq truncate-partial-width-windows nil) (list (window-text-pixel-size) (window-text-pixel-size w))))))
(progn (setq word-wrap t) (insert (mapconcat (lambda (n) (make-string n ?w)) '(10 20 30 15 5 40 7 7 7 60 3) " ")) (list (window-text-pixel-size) (window-text-pixel-size nil 50 120) (window-text-pixel-size nil 75 90) (window-text-pixel-size nil nil nil 30) (window-text-pixel-size nil nil nil 30 2) (window-text-pixel-size nil 1 75 nil nil nil t)))
(progn (setq word-wrap t) (insert "aaaa bbbb\tcccc " (make-string 70 ?d) "\n" (make-string 79 ?e) "\n" (make-string 78 ?f) " g") (list (window-text-pixel-size) (window-text-pixel-size nil 1 20) (window-text-pixel-size nil 87 170)))
(progn (insert "abc" (propertize "hidden" 'invisible 'e) "def" (propertize "more" 'invisible 'f) "\n") (setq buffer-invisibility-spec '((e . t) f)) (list (window-text-pixel-size) (window-text-pixel-size nil 1 4) (window-text-pixel-size nil 1 6) (window-text-pixel-size nil 2 10) (window-text-pixel-size nil 5 12) (window-text-pixel-size nil 10 13)))
(progn (insert "ab" (propertize "X" 'display "DISPLAY") "cd" (propertize "Y" 'display '(space :width 4)) "e") (list (window-text-pixel-size nil 3 3) (window-text-pixel-size nil 3 4) (window-text-pixel-size nil 4 5) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 3 6) (window-text-pixel-size nil 6 7) (window-text-pixel-size nil 7 8)))
(progn (insert (propertize "XYZ" 'display "D") "a\n" (propertize "Q" 'display "E") "b") (list (window-text-pixel-size nil 1 2) (window-text-pixel-size nil 2 3) (window-text-pixel-size nil 2 5) (window-text-pixel-size nil 6 7) (window-text-pixel-size nil 1 6 nil nil nil t) (window-text-pixel-size nil 5 6)))
(progn (insert (make-string 79 ?a) "日" "b\n" (make-string 78 ?a) "\t" "c\n" (make-string 77 ?a) "\C-a\C-b" "d") (list (window-text-pixel-size) (window-text-pixel-size nil 1 80) (window-text-pixel-size nil 80 81) (window-text-pixel-size nil 82 162) (window-text-pixel-size nil 162 164) (window-text-pixel-size nil 163 245) (window-text-pixel-size nil 242 244)))
(progn (setq truncate-lines t) (insert (make-string 79 ?a) "日" "b\n" (make-string 78 ?a) "\tc\n\n" (make-string 120 ?z)) (list (window-text-pixel-size) (window-text-pixel-size nil 1 80) (window-text-pixel-size nil 1 81) (window-text-pixel-size nil 80 85) (window-text-pixel-size nil 160 200) (window-text-pixel-size nil nil nil 200)))
(progn (insert "line1\nline2\nline3\nline4\nline5\n") (list (window-text-pixel-size nil '(13 . 0)) (window-text-pixel-size nil '(13 . 2)) (window-text-pixel-size nil '(13 . -2)) (window-text-pixel-size nil '(15 . 100)) (window-text-pixel-size nil '(15 . -100)) (window-text-pixel-size nil '(1 . 3) 20) (window-text-pixel-size nil '(31 . 0))))
(progn (insert (make-string 200 ?x) "\nabc") (list (window-text-pixel-size nil '(100 . 0)) (window-text-pixel-size nil '(100 . -1)) (window-text-pixel-size nil '(100 . 1)) (window-text-pixel-size nil '(202 . -2)) (window-text-pixel-size nil 100) (window-text-pixel-size nil 160)))
(progn (insert "abcdef\n") (let ((a (make-overlay 3 3)) (b (make-overlay 3 5)) (c (make-overlay 1 3))) (overlay-put a 'before-string "1234567890") (overlay-put a 'priority 2) (overlay-put b 'before-string (make-string 75 ?b)) (overlay-put c 'after-string "\t") (overlay-put c 'priority 9)) (list (window-text-pixel-size) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 3 4) (window-text-pixel-size nil 3 8) (window-text-pixel-size nil 1 3 nil nil nil t)))
(progn (insert "ab" (propertize "cd" 'invisible t 'display "!!") "ef") (let ((o (make-overlay 5 6))) (overlay-put o 'invisible t) (overlay-put o 'before-string "<B>") (overlay-put o 'after-string "<A>")) (list (window-text-pixel-size) (window-text-pixel-size nil 1 5) (window-text-pixel-size nil 5 6) (window-text-pixel-size nil 6 7)))
(progn (setq line-prefix (propertize ">>" 'display '(space :width 4)) wrap-prefix "\t") (insert "abc\n" (make-string 150 ?y) "\n\tz") (list (window-text-pixel-size) (window-text-pixel-size nil 5 100) (window-text-pixel-size nil 100 156) (window-text-pixel-size nil 156)))
(progn (setq word-wrap t wrap-prefix "  ") (insert (mapconcat #'identity (make-list 30 "word") " ")) (list (window-text-pixel-size) (window-text-pixel-size nil 80 150)))
(progn (insert "first\n" (propertize "second line\n" 'invisible 'outline) "third") (setq buffer-invisibility-spec '((outline . t))) (list (window-text-pixel-size) (window-text-pixel-size nil 7 19) (window-text-pixel-size nil 7) (window-text-pixel-size nil 19) (window-text-pixel-size nil '(19 . -1))))
(progn (insert (propertize "abc\n" 'invisible t) "def") (list (window-text-pixel-size) (window-text-pixel-size nil 1 3) (window-text-pixel-size nil 5) (window-text-pixel-size nil t t)))
(progn (insert "a" (propertize "b" 'display '(space :align-to 40)) "c" (propertize "d" 'display '(space :align-to (0.5 . text))) "e" (propertize "f" 'display '(space :align-to (+ left 20))) "g" (propertize "h" 'display '(space :align-to center)) "i" (propertize "j" 'display '(space :width (+ 2 3))) "k" (propertize "l" 'display '(space :width tab-width))) (list (window-text-pixel-size) (window-text-pixel-size nil 1 4)))
(progn (insert (propertize "x" 'display '(space :width -3)) (propertize "y" 'display '(space :width 0)) (propertize "z" 'display '(space :width foo)) (propertize "w" 'display '(space :align-to 0)) "q") (window-text-pixel-size))
(progn (setq truncate-lines t) (insert "a" (propertize " " 'display '(space :align-to 200)) "b\n" (propertize " " 'display '(space :width 100)) "c") (list (window-text-pixel-size) (window-text-pixel-size nil nil nil 500)))
(progn (insert "x" (propertize " " 'display '(space :width 100)) "y\nz") (list (window-text-pixel-size) (window-text-pixel-size nil 2 3) (window-text-pixel-size nil 3)))
(progn (insert "a" (propertize "X" 'display '((margin nil) "LONG")) "b") (list (window-text-pixel-size) (progn (put-text-property 2 3 'display '((margin nil) (space :width 5))) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '((margin) "LONG")) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '((margin left-margin) "LONG")) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '((margin right-margin) (space :width 5))) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '((margin foo) "LONG")) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '[((margin nil) . "LONG")]) (window-text-pixel-size)) (progn (put-text-property 2 3 'display '(((margin nil) "LONG") ((margin left-margin) "Q"))) (window-text-pixel-size)) (progn (put-text-property 2 3 'display (propertize "ab" 'display '((margin nil) "LONG"))) (window-text-pixel-size))))
(progn (insert "abc") (let ((o (make-overlay 2 2)) (p (make-overlay 3 4))) (overlay-put o 'before-string (propertize "x" 'display '((margin nil) "WIDE"))) (overlay-put p 'display '((margin nil) (space :width 6)))) (window-text-pixel-size))
(progn (insert "abc\n") (setq mode-line-format nil) (list (window-text-pixel-size nil nil nil nil nil t) (progn (setq mode-line-format "x") (window-text-pixel-size nil nil nil nil nil t)) (progn (set-window-parameter nil 'mode-line-format 'none) (prog1 (window-text-pixel-size nil nil nil nil nil t) (set-window-parameter nil 'mode-line-format nil)))))
"###;

/// window.el's callers, on side windows as keymap-popup and Hermes make them.
const CONSUMERS: &str = r###"
(let ((w (display-buffer-in-side-window (get-buffer-create "*side*") '((side . bottom))))) (with-current-buffer "*side*" (erase-buffer) (insert "a\nb\nc")) (fit-window-to-buffer w) (list (window-total-height w) (window-body-height w) (window-total-height (frame-root-window)) (window-text-pixel-size w nil t nil (frame-pixel-height) t)))
(let ((w (display-buffer-in-side-window (get-buffer-create "*side*") '((side . bottom))))) (with-current-buffer "*side*" (erase-buffer) (dotimes (i 40) (insert (format "line %d\n" i)))) (fit-window-to-buffer w) (list (window-total-height w) (progn (fit-window-to-buffer w 8 3) (window-total-height w)) (progn (with-current-buffer "*side*" (erase-buffer) (insert "x")) (fit-window-to-buffer w) (window-total-height w)) (progn (fit-window-to-buffer w nil 4) (window-total-height w))))
(let ((w (display-buffer-in-side-window (get-buffer-create "*side*") '((side . bottom))))) (with-current-buffer "*side*" (erase-buffer) (insert (make-string 200 ?w) "\nshort\n" (propertize "hidden\n" 'invisible t) "end")) (set-window-parameter w 'mode-line-format 'none) (fit-window-to-buffer w) (list (window-total-height w) (window-body-height w)))
(let ((fit-window-to-buffer-horizontally t) (w (display-buffer-in-side-window (get-buffer-create "*side*") '((side . left))))) (with-current-buffer "*side*" (erase-buffer) (insert "abc\nlonger line here\nx")) (fit-window-to-buffer w) (list (window-total-width w) (window-body-width w) (window-total-height w)))
(let ((w (split-window nil -10))) (set-window-buffer w (get-buffer-create "*side*")) (with-current-buffer "*side*" (erase-buffer) (insert "one\ntwo\n")) (list (shrink-window-if-larger-than-buffer w) (window-total-height w) (progn (with-current-buffer "*side*" (setq word-wrap t) (insert (mapconcat #'identity (make-list 40 "word") " "))) (fit-window-to-buffer w) (window-total-height w))))
"###;

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(progn {SETUP} (princ (prin1-to-string {expression})))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn booted() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el")
}

fn romacs(vm: &mut Vm, expression: &str) -> String {
    let source = format!("(progn {SETUP} (encode-coding-string (condition-case e (prin1-to-string {expression}) (error (format \"signal %S\" e))) 'utf-8))");
    match vm.eval_source(&source).and_then(|v| v.bytes().map(|b| String::from_utf8_lossy(b).into_owned())) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

fn compare(vm: &mut Vm, cases: &str) {
    let mut failures = vec![];
    for expression in cases.lines().filter(|line| !line.is_empty()) {
        let (expected, actual) = (oracle(expression), romacs(vm, expression));
        if expected != actual { failures.push(format!("{expression}\n  GNU:    {expected}\n  Romacs: {actual}")); }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn batch_text_pixel_sizes_match_pinned_gnu() {
    compare(&mut booted(), CASES);
}

#[test]
fn window_el_fits_side_windows_as_pinned_gnu() {
    compare(&mut booted(), CONSUMERS);
}

/// keymap-popup shows its popup in a bottom side window and fits it.
#[test]
fn keymap_popup_window_matches_pinned_gnu() {
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/emacs-lisp/keymap-popup.el");
    let journey = format!(r#"(progn (load {:?} nil t) (eval '(keymap-popup-define wtps-test-map "Test popup." :group "Files" "f" ("Find file" ignore) "s" ("Save" ignore) :group "Other" "x" ("Extra" ignore) "l" ("A longer description of a command" ignore)) t) (keymap-popup wtps-test-map) (prog1 (let ((w (cadr (window-list)))) (list (length (window-list)) (window-total-height w) (window-body-height w) (window-text-pixel-size w nil t nil (frame-pixel-height) t) (with-current-buffer (window-buffer w) (buffer-substring-no-properties (point-min) (point-max))))) (keymap-popup-dismiss)))"#, library.canonicalize().unwrap().display().to_string());
    compare(&mut booted(), &journey);
}


/// What the terminal model cannot answer refuses by name rather than
/// returning numbers GNU would not.
#[test]
fn unsupported_display_features_refuse() {
    let mut vm = booted();
    let refusal = |vm: &mut Vm, body: &str| romacs(vm, &format!("(progn {body} (condition-case e (window-text-pixel-size) (error (cadr e))))"));
    for (body, what) in [
        ("(insert \"a\" (propertize \"b\" 'display '(when (> 2 1) . \"Q\")))", "of a `(when FORM ...)' display specification"),
        ("(insert (propertize \"b\" 'display '(min-width (5))))", "of a `(min-width ...)' display specification"),
        ("(insert (propertize \"b\" 'display '(space :width (1 . in))))", "of a display width in physical units"),
        ("(insert (propertize \"b\" 'composition '(0 1 [?a])))", "of composed text (the `composition' property)"),
        ("(insert \"a\\rb\") (setq selective-display t)", "with `selective-display'"),
        ("(insert \"a\") (setq display-line-numbers t)", "with line numbers"),
        ("(insert \"abc\") (set-window-hscroll nil 1)", "in a horizontally scrolled window"),
        ("(insert \"a b\") (setq word-wrap t word-wrap-by-category t)", "with `word-wrap-by-category'"),
    ] {
        assert_eq!(refusal(&mut vm, body), format!("\"VM2 window-text-pixel-size {what} is unsupported\""), "{body}");
    }
}

/// A host's published layout cannot answer for the terminal frame model;
/// before any layout the frame is like GNU's initial one.
#[test]
fn displayed_window_refuses_after_a_host_layout() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(column * 80, line * 24, line, column).unwrap();
    vm.eval_source(r#"(progn (set-buffer (get-buffer-create "shown")) (insert "abc") (set-window-buffer nil (current-buffer)))"#).unwrap();
    let before = vm.eval_source("(list (window-text-pixel-size) (posn-at-point 1))").unwrap().display().unwrap();
    assert_eq!(before, format!("(({} . {}) nil)", 3 * column, line));
    let snapshot = vm.frame_snapshot(column * 80, line * 24).unwrap();
    let (_, ends) = romacs_layout2::layout_frame(&snapshot, 1).unwrap();
    vm.display_apply_layout(&ends).unwrap();
    for (call, name) in [("(window-text-pixel-size)", "window-text-pixel-size"), ("(buffer-text-pixel-size)", "buffer-text-pixel-size"), ("(posn-at-point 1)", "posn-at-point")] {
        let refused = vm.eval_source(&format!("(condition-case e {call} (error (car (cdr e))))")).unwrap().display().unwrap();
        assert_eq!(refused, format!("\"VM2 {name} in a displayed window is unsupported\""));
    }
    assert_eq!(vm.eval_source("(condition-case e (window-text-pixel-size 'w) (error e))").unwrap().display().unwrap(), "(wrong-type-argument window-live-p w)");
}
