//! treesit.c's primitives against pinned GNU on the same grammars: node
//! traversal and positions, query capture order, predicates and their
//! errors, load errors, printing, `equal' and hashing of nodes, edits,
//! narrowing, ranges, notifiers and searches. The cases run on the system
//! C grammar; given ROMACS_TREESIT_GRAMMARS, a private directory holding
//! copies of the json, python and rust grammars (never a user's
//! directory), the cases on those run too.
use romacs_vm::Vm;
use std::path::Path;

/// A VM on Romacs's loadup chain, as tests/advised_commands.rs boots one.
fn bootstrapped(root: &Path) -> Vm {
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("lisp/romacs/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// One expression per line, on the system C grammar both editors find by
/// GNU's default search (a bare library name), with `user-emacs-directory'
/// bound to a directory that does not exist.
const SYSTEM_CASES: &str = r###"
(with-temp-buffer (insert "int a[] = {1, 22, 333};\nstruct s { int x; };\nchar *p = \"é\";\n") (let* ((p (treesit-parser-create 'c)) (root (treesit-parser-root-node p)) (decl (treesit-node-child root 0)) (init (treesit-node-child-by-field-name (treesit-node-child-by-field-name decl "declarator") "value"))) (list (treesit-node-type root) (treesit-node-start decl) (treesit-node-end decl) (treesit-node-child-count root) (treesit-node-child-count init) (treesit-node-child-count init t) (treesit-node-type (treesit-node-child init -1)) (treesit-node-type (treesit-node-child init 1 t)) (treesit-node-string decl) (prin1-to-string (treesit-node-child init 0)) (prin1-to-string (treesit-node-child init 1 t)) (prin1-to-string p) (type-of p) (type-of root) (treesit-node-check root 'named) (treesit-node-check (treesit-node-child init 0) 'named) (treesit-node-check root 'has-error) (treesit-node-check root 'live) (condition-case e (treesit-node-check root 'has-changes) (error e)) (treesit-node-type (treesit-node-parent init)) (treesit-node-field-name-for-child decl 0) (treesit-node-field-name-for-child decl -2) (treesit-node-type (treesit-node-next-sibling decl t)) (treesit-node-type (treesit-node-prev-sibling (treesit-node-child init 1))) (treesit-node-type (treesit-node-first-child-for-pos init 13 t)) (treesit-node-type (treesit-node-descendant-for-range root 14 15 t)) (treesit-node-eq (treesit-node-child init 1) (treesit-node-child init 1)) (eq (treesit-node-child init 1) (treesit-node-child init 1)) (equal (treesit-node-child init 1) (treesit-node-child init 1)) (equal (treesit-node-child init 1) (treesit-node-child init 2)) (treesit-node-start (treesit-node-child (treesit-node-child root 2) -2)) (treesit-node-end root) (treesit-subtree-stat root))))
(with-temp-buffer (insert "int a[] = {1, 22, 333};") (let* ((p (treesit-parser-create 'c)) (root (treesit-parser-root-node p))) (list (treesit-query-capture root "(number_literal) @n") (treesit-query-capture root '((number_literal) @n) nil nil t) (treesit-query-capture root '((number_literal) @n) 15 17) (treesit-query-capture root '((initializer_list (number_literal) @a (number_literal) @b)) nil nil nil t) (treesit-query-capture root "((number_literal) @n (#match? \"[23]\" @n))" nil nil t) (treesit-query-capture root "((number_literal) @n (#match? @n \"^2\"))" nil nil t) (treesit-query-capture root "((number_literal) @n (#eq? @n \"333\"))" nil nil t) (treesit-query-capture root '(((number_literal) @n) (:equal "1" @n)) nil nil t) (treesit-query-capture 'c "(initializer_list) @a" nil nil t) (treesit-query-capture p "(initializer_list) @a" nil nil t) (treesit-query-capture root "(number_literal) @n" 1 1) (treesit-query-capture root "(number_literal) @n" 1 nil))))
(with-temp-buffer (insert "int a[] = {{1}, 2};") (let ((root (treesit-parser-root-node (treesit-parser-create 'c)))) (treesit-query-capture root "(initializer_list (_) @outer) @n (number_literal) @a" nil nil t)))
(with-temp-buffer (insert "int a[] = {1, 2};") (let ((root (treesit-parser-root-node (treesit-parser-create 'c)))) (list (condition-case e (treesit-query-capture root "((number_literal) @n (#foo? @n))") (error e)) (condition-case e (treesit-query-capture root "((number_literal) @n (#match? @n))") (error e)) (condition-case e (treesit-query-capture root "((number_literal) @n (#match? @n @n))") (error e)) (condition-case e (treesit-query-capture root "((number_literal) @n (#eq? @n))") (error e)) (condition-case e (treesit-query-capture root "((number_literal) @n (#pred? f))") (error e)) (condition-case e (treesit-query-capture root "((number_literal) @n (#eq? @m \"1\"))") (error e)) (condition-case e (treesit-query-capture root "((number_literal) @n (#match? \"\\\\(\" @n))") (error e)) (condition-case e (treesit-query-capture root "(number_literal @n") (error e)) (condition-case e (treesit-query-capture root "(bogus) @n") (error e)) (condition-case e (treesit-query-capture root '((bogus) @n)) (error e)) (condition-case e (treesit-query-capture root 1) (error e)) (condition-case e (treesit-query-capture 1 "(number_literal) @n") (error e)))))
(with-temp-buffer (insert "int a[] = {1, 22, 333};") (let ((root (treesit-parser-root-node (treesit-parser-create 'c)))) (defvar treesit-test-seen nil) (setq treesit-test-seen nil) (defalias 'treesit-test-long (lambda (n) (push (treesit-node-text n) treesit-test-seen) (> (length (treesit-node-text n)) 1))) (defalias 'treesit-test-odd (lambda (n) (= 1 (% (string-to-number (treesit-node-text n)) 2)))) (defalias 'treesit-test-both (lambda (a b) (push (list (treesit-node-text a) (treesit-node-text b)) treesit-test-seen) t)) (defalias 'treesit-test-error (lambda (_) (error "From pred"))) (list (treesit-query-capture root '((((number_literal) @n) (:pred treesit-test-long @n))) nil nil t) (treesit-query-capture root "((number_literal) @n (#pred? treesit-test-odd @n) (#match? \"3\" @n))" nil nil t) (treesit-query-capture root '(((initializer_list (number_literal) @a :anchor (number_literal) @b) (:pred treesit-test-both @a @b))) nil nil nil t) (reverse treesit-test-seen) (condition-case e (treesit-query-capture root '((((number_literal) @n) (:pred treesit-test-error @n)))) (error e)) (with-temp-buffer (treesit-query-capture root '((((number_literal) @n) (:pred treesit-test-odd @n))) nil nil t)) (treesit-query-capture root '(((number_literal) @n) (:pred treesit-test-odd @n))))))
(list (treesit-pattern-expand :anchor) (treesit-pattern-expand :?) (treesit-pattern-expand :equal) (treesit-pattern-expand :match?) (treesit-pattern-expand :pred) (treesit-pattern-expand '(identifier)) (treesit-pattern-expand [(a) (b)]) (treesit-pattern-expand "a\"b\\c\nd") (treesit-pattern-expand 'field:) (treesit-pattern-expand '@cap) (treesit-query-expand '((a) @x (:match "re" @x) [b c] _ (_))))
(let ((q (treesit-query-compile 'c "(number_literal) @n"))) (list (treesit-query-p q) (treesit-compiled-query-p q) (treesit-query-p "x") (treesit-query-p 1) (treesit-query-eagerly-compiled-p q) (treesit-query-language q) (treesit-query-source q) (prin1-to-string q) (type-of q) (treesit-query-eagerly-compiled-p (treesit-query-compile 'c q t)) (condition-case e (treesit-query-compile 'c "(bogus)" t) (error e)) (condition-case e (treesit-query-compile 'c 1) (error e)) (eq q (treesit-query-compile 'c q))))
(list (let ((treesit-extra-load-path '("/nonexistent-romacs-extra"))) (list (treesit-language-available-p 'nosuchlang) (treesit-language-available-p 'nosuchlang t) (treesit-language-abi-version 'nosuchlang) (condition-case e (treesit-parser-create 'nosuchlang) (error (car e))))) (treesit-language-available-p 'c) (treesit-language-available-p 'c t) (treesit-language-abi-version 'c) (treesit-grammar-location 'c) (treesit-library-abi-version) (treesit-library-abi-version t) (condition-case e (treesit-language-available-p "c") (error e)))
(let ((treesit-load-name-override-list '((cx "libtree-sitter-c" "tree_sitter_nosym")))) (treesit-language-available-p 'cx t))
(with-temp-buffer (insert "int a = 1;") (let* ((p (treesit-parser-create 'c)) (n (treesit-node-child (treesit-node-child (treesit-parser-root-node p) 0) 1 t))) (goto-char (point-min)) (insert "  ") (list (treesit-node-start n) (treesit-node-check n 'outdated) (prin1-to-string n) (treesit-parser-changed-regions p) (treesit-node-check n 'outdated) (prin1-to-string n) (condition-case e (treesit-node-start n) (error (car e))) (treesit-node-start (treesit-node-child (treesit-node-child (treesit-parser-root-node p) 0) 1 t)) (progn (goto-char (point-max)) (insert " int b = 2;") (treesit-parser-changed-regions p)) (treesit-parser-changed-regions p) (progn (goto-char 3) (delete-char 3) (insert "long") (treesit-node-string (treesit-parser-root-node p))))))
(with-temp-buffer (insert "int a; int b;") (let ((p (treesit-parser-create 'c))) (narrow-to-region 8 14) (list (treesit-node-string (treesit-parser-root-node p)) (treesit-node-start (treesit-parser-root-node p)) (progn (widen) (treesit-node-string (treesit-parser-root-node p))))))
(with-temp-buffer (insert "int a; int bb; int ccc;") (let ((p (treesit-parser-create 'c))) (list (treesit-parser-set-included-ranges p '((8 . 15))) (treesit-parser-included-ranges p) (treesit-node-string (treesit-parser-root-node p)) (condition-case e (treesit-parser-set-included-ranges p '((5 . 3))) (error e)) (condition-case e (treesit-parser-set-included-ranges p '((1 . 3) (2 . 4))) (error e)) (condition-case e (treesit-parser-set-included-ranges p '((1 . "bad"))) (error e)) (condition-case e (treesit-parser-set-included-ranges p 'x) (error e)) (progn (narrow-to-region 10 20) (treesit-parser-included-ranges p)) (progn (widen) (treesit-parser-set-included-ranges p nil) (treesit-parser-included-ranges p)) (treesit-node-string (treesit-parser-root-node p)))))
(with-temp-buffer (insert "int a;") (let ((p (treesit-parser-create 'c)) (log nil)) (defalias 'treesit-test-notify (lambda (ranges parser) (push (list 'a ranges (eq parser p)) log))) (defalias 'treesit-test-notify-2 (lambda (ranges _) (push (list 'b ranges) log) (error "ignored"))) (treesit-parser-add-notifier p 'treesit-test-notify) (treesit-parser-add-notifier p 'treesit-test-notify-2) (treesit-parser-add-notifier p 'treesit-test-notify) (list (treesit-parser-notifiers p) (progn (treesit-parser-root-node p) (reverse log)) (progn (setq log nil) (goto-char (point-max)) (insert " int b;") (treesit-parser-root-node p) (treesit-parser-root-node p) (reverse log)) (progn (treesit-parser-remove-notifier p 'treesit-test-notify-2) (treesit-parser-notifiers p)) (condition-case e (treesit-parser-add-notifier p (lambda ())) (error (car e))))))
(with-temp-buffer (let* ((p (treesit-parser-create 'c)) (q (treesit-parser-create 'c)) (r (treesit-parser-create 'c nil t)) (s (treesit-parser-create 'c nil nil 'tag))) (list (eq p q) (eq p r) (length (treesit-parser-list)) (length (treesit-parser-list nil nil t)) (length (treesit-parser-list nil 'c 'tag)) (treesit-parser-tag s) (treesit-parser-language p) (eq (treesit-parser-buffer p) (current-buffer)) (treesit-parser-embed-level p) (treesit-parser-set-embed-level p 2) (treesit-parser-embed-level p) (condition-case e (treesit-parser-set-embed-level p -1) (error e)) (condition-case e (treesit-parser-create 'c nil nil t) (error e)) (treesit-parser-delete r) (condition-case e (treesit-parser-root-node r) (error (car e))) (length (treesit-parser-list nil nil t)) (treesit-parser-p p) (treesit-node-p p))))
(with-temp-buffer (insert "int f(int x) { return x + 1; }\nint g = 2;") (let* ((root (treesit-parser-root-node (treesit-parser-create 'c))) (first (treesit-search-subtree root "identifier"))) (list (treesit-node-text first) (treesit-node-text (treesit-search-subtree root "identifier" t)) (treesit-node-text (treesit-search-subtree root "identifier" nil nil 2)) (treesit-node-text (treesit-search-forward first "identifier")) (treesit-node-text (treesit-search-forward first (lambda (n) (equal (treesit-node-type n) "return_statement")))) (treesit-node-text (treesit-search-forward first "(" nil t)) (treesit-node-text (treesit-search-forward first "identifier" t)) (treesit-node-text (treesit-search-subtree root (lambda (n) (equal (treesit-node-text n) "1")))) (mapcar (lambda (n) (if (treesit-node-p n) (treesit-node-text n) n)) (flatten-tree (treesit-induce-sparse-tree root "identifier\\|declaration"))) (treesit-induce-sparse-tree root "number_literal" #'treesit-node-text) (treesit-induce-sparse-tree root "identifier" nil 3) (treesit-node-match-p first "identifier") (treesit-node-match-p first '(or "string" (not "number_literal"))) (treesit-node-match-p first (cons "identifier" (lambda (n) (equal (treesit-node-text n) "f")))) (treesit-node-match-p first 'named) (treesit-node-match-p first (list 'and 'named (rx "ident"))) (condition-case e (treesit-node-match-p first 'nothing) (error e)) (treesit-node-match-p first 'nothing t) (condition-case e (treesit-node-match-p first '(not)) (error e)) (condition-case e (treesit-search-subtree root 1) (error e)) (treesit-search-subtree root 'nothing) (let ((treesit-thing-settings '((c (num "number_literal") (both (or num "identifier")))))) (list (treesit-node-text (treesit-search-subtree root 'num)) (length (flatten-tree (treesit-induce-sparse-tree root 'both))))))))
(with-temp-buffer (insert "int a;") (let ((root (treesit-parser-root-node (treesit-parser-create 'c)))) (list (condition-case e (treesit-search-subtree root (lambda (_) (error "From search"))) (error e)))))
(let ((node (treesit-parse-string "int a;" 'c))) (list (treesit-node-string node) (treesit-parser-list (treesit-parser-buffer (treesit-node-parser node)))))
(with-temp-buffer (insert "int a;") (let ((p (treesit-parser-create 'c))) (treesit-parser-root-node p) (goto-char 1) (delete-char 1) (insert "i") (list (treesit-parser-changed-regions p) (treesit-parser-changed-regions p) (treesit-query-capture p "(identifier) @i" nil nil t))))
(with-temp-buffer (insert "int a;\nint b;\nint c;\n") (let* ((treesit-languages-require-line-column-tracking '(c)) (p (treesit-parser-create 'c))) (treesit-parser-root-node p) (subst-char-in-region 1 (point-max) ?\n ?\s) (goto-char (point-max)) (list (treesit--linecol-at (point-max)) (treesit--linecol-cache) (treesit-parser-tracking-line-column-p p) (treesit-node-string (treesit-parser-root-node p)) (progn (translate-region 1 4 (make-string 256 ?x)) (treesit--linecol-at (point-max))))))
(with-temp-buffer (insert "int a; int b;") (let* ((p (treesit-parser-create 'c)) (q (treesit-parser-create 'c nil nil 'twin)) (log nil) (rp (treesit-parser-root-node p))) (defalias 'treesit-test-notify (lambda (ranges parser) (push (list ranges (eq parser q)) log))) (treesit-parser-add-notifier q 'treesit-test-notify) (let ((rq (treesit-parser-root-node q))) (list (equal (treesit-node-string rp) (treesit-node-string rq)) (treesit-node-eq rp rq) (equal rp rq) (treesit-node-check rq 'live) (treesit-parser-changed-regions q) (reverse log) (progn (setq log nil) (goto-char 5) (insert "xx") (list (treesit-parser-changed-regions p) (treesit-parser-changed-regions q) (treesit-node-string (treesit-parser-root-node p)) (treesit-node-string (treesit-parser-root-node q)) (treesit-node-check rq 'outdated) (reverse log))) (progn (goto-char (point-max)) (insert " int c;") (treesit-parser-delete p) (treesit-node-string (treesit-parser-root-node q)))))))
(with-temp-buffer (insert "int a; int bb; int ccc;") (let ((p (treesit-parser-create 'c)) (q (treesit-parser-create 'c nil nil 'twin)) (r (treesit-parser-create 'c nil nil 'whole)) (s (treesit-parser-create 'c nil nil 'same))) (treesit-parser-set-included-ranges p '((8 . 15))) (treesit-parser-set-included-ranges s '((8 . 15))) (let ((rp (treesit-node-string (treesit-parser-root-node p)))) (treesit-parser-set-included-ranges r (list (cons (point-min) (point-max)))) (list rp (treesit-node-string (treesit-parser-root-node q)) (treesit-node-string (treesit-parser-root-node r)) (treesit-node-string (treesit-parser-root-node s)) (progn (goto-char 8) (insert "long ") (list (treesit-parser-included-ranges s) (treesit-node-string (treesit-parser-root-node s)) (treesit-parser-changed-regions r) (treesit-node-string (treesit-parser-root-node r))))))))
(with-temp-buffer (insert "int a; int b;") (let ((p (treesit-parser-create 'c)) (q (treesit-parser-create 'c nil nil 'narrowed)) (r (treesit-parser-create 'c nil nil 'stale))) (treesit-parser-root-node p) (list (save-restriction (narrow-to-region 8 14) (list (treesit-node-string (treesit-parser-root-node q)) (treesit-node-start (treesit-parser-root-node q)))) (progn (goto-char (point-max)) (insert " int c;") (treesit-node-string (treesit-parser-root-node r))) (treesit-node-string (treesit-parser-root-node p)) (treesit-node-string (treesit-parser-root-node q)))))
(with-temp-buffer (insert ";t f(i/1x)(") (let ((p (treesit-parser-create 'c)) (q (treesit-parser-create 'c nil nil 'twin))) (treesit-parser-root-node p) (goto-char 2) (insert "=") (list (treesit-node-string (treesit-parser-root-node p)) (treesit-node-string (treesit-parser-root-node q)))))
(with-temp-buffer (insert "int a; int b;") (let ((p (treesit-parser-create 'c)) (q (treesit-parser-create 'c nil nil 'twin))) (treesit-parser-root-node p) (let ((n (treesit-node-child (treesit-parser-root-node q) 0))) (goto-char 5) (insert "x") (treesit-parser-root-node p) (garbage-collect) (list (treesit-node-type n) (treesit-node-start n) (treesit-node-check n 'outdated) (treesit-node-type (treesit-node-child n 1 t)) (progn (treesit-parser-root-node q) (treesit-node-check n 'outdated))))))
(with-temp-buffer (insert "int a;\nint b;\n") (let* ((p (treesit-parser-create 'c)) (treesit-languages-require-line-column-tracking '(c)) (q (treesit-parser-create 'c nil nil 'tracking))) (treesit-parser-root-node p) (goto-char 1) (insert "\n") (treesit-parser-root-node p) (list (treesit-parser-tracking-line-column-p p) (treesit-parser-tracking-line-column-p q) (treesit-node-string (treesit-parser-root-node q)) (treesit-node-start (treesit-node-child (treesit-parser-root-node q) 1)))))
(with-temp-buffer (insert "int foo_bar;") (let ((root (treesit-parser-root-node (treesit-parser-create 'c)))) (list (treesit-query-capture root "((identifier) @i (#match? \"\\\\bbar\" @i))" nil nil t) (with-temp-buffer (set-syntax-table (make-syntax-table)) (modify-syntax-entry ?_ "w") (treesit-query-capture root "((identifier) @i (#match? \"\\\\bbar\" @i))" nil nil t)) (progn (set-syntax-table (make-syntax-table)) (modify-syntax-entry ?_ "w") (treesit-query-capture root "((identifier) @i (#match? \"\\\\bbar\" @i))" nil nil t)))))
(with-temp-buffer (insert "int foo_bar;") (let ((root (treesit-parser-root-node (treesit-parser-create 'c))) (q "((identifier) @i (#match? \"\\\\bbar\" @i))")) (put-text-property 8 9 'syntax-table '(2)) (list (treesit-query-capture root q nil nil t) (progn (setq-local parse-sexp-lookup-properties t) (treesit-query-capture root q nil nil t)) (let ((buffer (current-buffer))) (with-temp-buffer (treesit-query-capture root q nil nil t))) (with-temp-buffer (setq-local parse-sexp-lookup-properties nil) (treesit-query-capture root q nil nil t)))))
"###;

/// One expression per line, on the json, python and rust grammars of the
/// private directory ROMACS_TREESIT_GRAMMARS, when it is given.
const CASES: &str = r###"
(with-temp-buffer (insert "[1, {\"a\": true}, \"é\"]") (let* ((p (treesit-parser-create 'json)) (root (treesit-parser-root-node p)) (arr (treesit-node-child root 0))) (list (treesit-node-type root) (treesit-node-start arr) (treesit-node-end arr) (treesit-node-child-count arr) (treesit-node-child-count arr t) (treesit-node-type (treesit-node-child arr -1)) (treesit-node-type (treesit-node-child arr 1 t)) (treesit-node-string arr) (prin1-to-string (treesit-node-child arr 0)) (prin1-to-string (treesit-node-child arr 1 t)) (prin1-to-string p) (type-of p) (type-of root) (treesit-node-check root 'named) (treesit-node-check (treesit-node-child arr 0) 'named) (treesit-node-check root 'has-error) (treesit-node-check root 'live) (treesit-node-type (treesit-node-parent (treesit-node-child arr 1 t))) (treesit-node-field-name-for-child (treesit-node-child (treesit-node-child arr 1 t) 1 t) 0) (treesit-node-type (treesit-node-child-by-field-name (treesit-node-child (treesit-node-child arr 1 t) 1 t) "value")) (treesit-node-type (treesit-node-next-sibling (treesit-node-child arr 1 t) t)) (treesit-node-type (treesit-node-prev-sibling (treesit-node-child arr 1 t))) (treesit-node-type (treesit-node-first-child-for-pos arr 3 t)) (treesit-node-type (treesit-node-descendant-for-range root 5 8 t)) (treesit-node-eq (treesit-node-child arr 0) (treesit-node-child arr 0)) (eq (treesit-node-child arr 0) (treesit-node-child arr 0)) (equal (treesit-node-child arr 0) (treesit-node-child arr 0)) (treesit-node-start (treesit-node-child arr -1 t)) (treesit-subtree-stat root))))
(with-temp-buffer (insert "[1, 2, 3]") (let* ((p (treesit-parser-create 'json)) (root (treesit-parser-root-node p))) (list (treesit-query-capture root "(number) @n") (treesit-query-capture root '((number) @n) nil nil t) (treesit-query-capture root '((number) @n) 4 6) (treesit-query-capture root '((array (number) @a (number) @b)) nil nil nil t) (treesit-query-capture root "((number) @n (#match? \"[23]\" @n))" nil nil t) (treesit-query-capture root "((number) @n (#match? @n \"^2\"))" nil nil t) (treesit-query-capture root "((number) @n (#eq? @n \"3\"))" nil nil t) (treesit-query-capture root '(((number) @n) (:equal "1" @n)) nil nil t) (treesit-query-capture 'json "(array) @a" nil nil t) (treesit-query-capture p "(array) @a" nil nil t) (treesit-query-capture root "(number) @n" 1 1))))
(with-temp-buffer (insert "[[1], 2]") (let ((root (treesit-parser-root-node (treesit-parser-create 'json)))) (treesit-query-capture root "(array (_) @outer) @n (number) @a" nil nil t)))
(with-temp-buffer (insert "[1, 2]") (let ((root (treesit-parser-root-node (treesit-parser-create 'json)))) (list (condition-case e (treesit-query-capture root "((number) @n (#foo? @n))") (error e)) (condition-case e (treesit-query-capture root "((number) @n (#match? @n))") (error e)) (condition-case e (treesit-query-capture root "((number) @n (#match? @n @n))") (error e)) (condition-case e (treesit-query-capture root "((number) @n (#eq? @n))") (error e)) (condition-case e (treesit-query-capture root "((number) @n (#pred? f))") (error e)) (condition-case e (treesit-query-capture root "((number) @n (#eq? @m \"1\"))") (error e)) (condition-case e (treesit-query-capture root "(number @n") (error e)) (condition-case e (treesit-query-capture root "(bogus) @n") (error e)) (condition-case e (treesit-query-capture root '((bogus) @n)) (error e)) (condition-case e (treesit-query-capture root 1) (error e)) (condition-case e (treesit-query-capture 1 "(number) @n") (error e)))))
(with-temp-buffer (insert "[1, 22, 333]") (let ((root (treesit-parser-root-node (treesit-parser-create 'json))) (seen nil)) (list (treesit-query-capture root '(((number) @n) (:pred (lambda (n) (push (treesit-node-text n) seen) (> (length (treesit-node-text n)) 1)) @n)) nil nil t) (progn (defalias 'treesit-test-odd (lambda (n) (= 1 (% (string-to-number (treesit-node-text n)) 2)))) (treesit-query-capture root '(((number) @n) (:pred treesit-test-odd @n)) nil nil t)) seen)))
(list (treesit-pattern-expand :anchor) (treesit-pattern-expand :?) (treesit-pattern-expand :equal) (treesit-pattern-expand :match?) (treesit-pattern-expand :pred) (treesit-pattern-expand '(identifier)) (treesit-pattern-expand [(a) (b)]) (treesit-pattern-expand "a\"b\\c\nd") (treesit-pattern-expand 'field:) (treesit-pattern-expand '@cap) (treesit-query-expand '((a) @x (:match "re" @x) [b c] _ (_))))
(let ((q (treesit-query-compile 'json "(number) @n"))) (list (treesit-query-p q) (treesit-compiled-query-p q) (treesit-query-p "x") (treesit-query-p 1) (treesit-query-eagerly-compiled-p q) (treesit-query-language q) (treesit-query-source q) (prin1-to-string q) (type-of q) (treesit-query-eagerly-compiled-p (treesit-query-compile 'json q t)) (condition-case e (treesit-query-compile 'json "(bogus)" t) (error e)) (condition-case e (treesit-query-compile 'json 1) (error e)) (eq q (treesit-query-compile 'json q))))
(list (let ((user-emacs-directory "/nonexistent-romacs-ued/") (treesit-extra-load-path '("/nonexistent-romacs-extra"))) (list (treesit-language-available-p 'nosuchlang) (treesit-language-available-p 'nosuchlang t) (treesit-language-abi-version 'nosuchlang) (condition-case e (treesit-parser-create 'nosuchlang) (error (car e))))) (treesit-language-available-p 'json) (treesit-language-available-p 'json t) (treesit-language-abi-version 'json) (treesit-library-abi-version) (treesit-library-abi-version t) (condition-case e (treesit-language-available-p "json") (error e)))
(let ((treesit-load-name-override-list '((jsonx "libtree-sitter-json" "tree_sitter_nosym")))) (treesit-language-available-p 'jsonx t))
(with-temp-buffer (insert "[1, 2]") (let* ((p (treesit-parser-create 'json)) (root (treesit-parser-root-node p)) (n (treesit-node-child (treesit-node-child root 0) 1 t))) (goto-char (point-min)) (insert "  ") (list (treesit-node-start n) (treesit-node-check n 'outdated) (prin1-to-string n) (treesit-parser-changed-regions p) (treesit-node-check n 'outdated) (prin1-to-string n) (condition-case e (treesit-node-start n) (error (car e))) (treesit-node-start (treesit-node-child (treesit-node-child (treesit-parser-root-node p) 0) 1 t)) (progn (goto-char (point-max)) (insert " 3]") (treesit-parser-changed-regions p)) (treesit-parser-changed-regions p) (treesit-node-string (treesit-parser-root-node p)))))
(with-temp-buffer (insert "[1, 2] [3]") (let ((p (treesit-parser-create 'json))) (narrow-to-region 8 11) (list (treesit-node-string (treesit-parser-root-node p)) (treesit-node-start (treesit-parser-root-node p)) (progn (widen) (treesit-node-string (treesit-parser-root-node p))))))
(with-temp-buffer (insert "[1] [22] [333]") (let ((p (treesit-parser-create 'json))) (list (treesit-parser-set-included-ranges p '((5 . 9))) (treesit-parser-included-ranges p) (treesit-node-string (treesit-parser-root-node p)) (condition-case e (treesit-parser-set-included-ranges p '((5 . 3))) (error e)) (condition-case e (treesit-parser-set-included-ranges p '((1 . 3) (2 . 4))) (error e)) (progn (narrow-to-region 6 12) (treesit-parser-included-ranges p)) (progn (widen) (treesit-parser-set-included-ranges p nil) (treesit-parser-included-ranges p)) (treesit-node-string (treesit-parser-root-node p)))))
(with-temp-buffer (insert "[1]") (let ((p (treesit-parser-create 'json)) (log nil)) (defalias 'treesit-test-notify (lambda (ranges parser) (push (list 'a ranges (eq parser p)) log))) (defalias 'treesit-test-notify-2 (lambda (ranges _) (push (list 'b ranges) log) (error "ignored"))) (treesit-parser-add-notifier p 'treesit-test-notify) (treesit-parser-add-notifier p 'treesit-test-notify-2) (treesit-parser-add-notifier p 'treesit-test-notify) (list (treesit-parser-notifiers p) (progn (treesit-parser-root-node p) (reverse log)) (progn (setq log nil) (goto-char 2) (insert "2, ") (treesit-parser-root-node p) (treesit-parser-root-node p) (reverse log)) (progn (treesit-parser-remove-notifier p 'treesit-test-notify-2) (treesit-parser-notifiers p)) (condition-case e (treesit-parser-add-notifier p (lambda ())) (error (car e))))))
(with-temp-buffer (let* ((p (treesit-parser-create 'json)) (q (treesit-parser-create 'json)) (r (treesit-parser-create 'json nil t)) (s (treesit-parser-create 'json nil nil 'tag))) (list (eq p q) (eq p r) (length (treesit-parser-list)) (length (treesit-parser-list nil nil t)) (length (treesit-parser-list nil 'json 'tag)) (treesit-parser-tag s) (treesit-parser-language p) (eq (treesit-parser-buffer p) (current-buffer)) (treesit-parser-embed-level p) (treesit-parser-set-embed-level p 2) (treesit-parser-embed-level p) (condition-case e (treesit-parser-set-embed-level p -1) (error e)) (condition-case e (treesit-parser-create 'json nil nil t) (error e)) (treesit-parser-delete r) (condition-case e (treesit-parser-root-node r) (error (car e))) (length (treesit-parser-list nil nil t)) (treesit-parser-p p) (treesit-node-p p))))
(with-temp-buffer (insert "[1, [2, 3], {\"k\": 4}]") (let* ((root (treesit-parser-root-node (treesit-parser-create 'json))) (first (treesit-search-subtree root "number"))) (list (treesit-node-text first) (treesit-node-text (treesit-search-subtree root "number" t)) (treesit-node-text (treesit-search-subtree root "number" nil nil 1)) (treesit-node-text (treesit-search-forward first "number")) (treesit-node-text (treesit-search-forward first (lambda (n) (equal (treesit-node-type n) "pair")))) (treesit-node-text (treesit-search-forward first "," nil t)) (treesit-node-text (treesit-search-subtree root (lambda (n) (equal (treesit-node-text n) "3")))) (mapcar (lambda (n) (if (treesit-node-p n) (treesit-node-text n) n)) (flatten-tree (treesit-induce-sparse-tree root "number\\|array"))) (treesit-induce-sparse-tree root "number" #'treesit-node-text) (treesit-node-match-p first "number") (treesit-node-match-p first '(or "string" (not "array"))) (treesit-node-match-p first '("number" . (lambda (n) (equal (treesit-node-text n) "2")))) (treesit-node-match-p first 'named) (condition-case e (treesit-node-match-p first 'nothing) (error e)) (treesit-node-match-p first 'nothing t) (condition-case e (treesit-node-match-p first '(not)) (error e)) (condition-case e (treesit-search-subtree root 1) (error e)) (treesit-search-subtree root 'nothing))))
(with-temp-buffer (insert "def f(x):\n    return x\n") (let ((root (treesit-parser-root-node (treesit-parser-create 'python)))) (list (treesit-node-string root) (treesit-query-capture root '((function_definition name: (identifier) @name)) nil nil t))))
(with-temp-buffer (insert "fn main() { let x = \"λ\"; }") (let ((root (treesit-parser-root-node (treesit-parser-create 'rust)))) (list (mapcar (lambda (c) (list (car c) (treesit-node-start (cdr c)) (treesit-node-end (cdr c)))) (treesit-query-capture root '((string_literal) @s (identifier) @i))) (treesit-node-type (treesit-node-at 15)))))
(let ((node (treesit-parse-string "[true]" 'json))) (list (treesit-node-string node) (treesit-parser-list (treesit-parser-buffer (treesit-node-parser node)))))
"###;

/// Each of CASES evaluated by pinned GNU (a fresh process) and by VM,
/// after SETUP; the cases whose printed values differ.
fn differences(vm: &mut Vm, setup: &str, cases: &str) -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in cases.lines().filter(|line| !line.is_empty()) {
        let source = format!("(progn {setup} (append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", "(require 'treesit)", "--eval", &format!("(prin1 {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    failures
}

/// Both editors' grammar search sees only the system directories.
const SYSTEM_SETUP: &str = "(setq user-emacs-directory \"/nonexistent-romacs-user-emacs-directory/\" treesit-extra-load-path nil)";

#[test]
fn treesit_matches_pinned_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    vm.eval_source("(require 'treesit)").unwrap();
    let available = vm.eval_source(&format!("(progn {SYSTEM_SETUP} (treesit-language-available-p 'c))")).and_then(|v| v.display());
    assert_eq!(available.as_deref(), Ok("t"), "the system C grammar (libtree-sitter-c) GNU's default search finds");
    let mut failures = differences(&mut vm, SYSTEM_SETUP, SYSTEM_CASES);
    if let Ok(grammars) = std::env::var("ROMACS_TREESIT_GRAMMARS") {
        failures.extend(differences(&mut vm, &format!("(setq treesit-extra-load-path '({grammars:?}))"), CASES));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// An early parse gives the first parser of a buffer what a parse would,
/// through the buffer's changes since, unless it is narrowed or the parser
/// asks otherwise.
#[test]
fn an_early_parse_is_used_only_for_the_text_it_read() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    vm.eval_source("(require 'treesit)").unwrap();
    let source = format!("(progn {SYSTEM_SETUP} \
        (let* ((text \"int a; int b;\") \
               (string (lambda () (treesit-node-string (treesit-parser-root-node (treesit-parser-create 'c))))) \
               (fresh (lambda (text) (with-temp-buffer (insert text) (funcall string))))) \
          (list (with-temp-buffer (insert text) (list (romacs--treesit-parse-early 'c) (romacs--treesit-parse-early 'c) (equal (funcall string) (funcall fresh text)))) \
                (with-temp-buffer (insert text) (romacs--treesit-parse-early 'c) (goto-char 1) (insert \"int z; \") (equal (funcall string) (funcall fresh (buffer-string)))) \
                (with-temp-buffer (insert text) (romacs--treesit-parse-early 'c) (subst-char-in-region 1 14 ?i ?=) (equal (funcall string) (funcall fresh (buffer-string)))) \
                (with-temp-buffer (insert text) (romacs--treesit-parse-early 'c) (narrow-to-region 8 14) (equal (funcall string) (funcall fresh \"int b;\"))) \
                (with-temp-buffer (insert text) (romacs--treesit-parse-early 'c) (romacs--treesit-parse-early nil) (equal (funcall string) (funcall fresh text))) \
                (with-temp-buffer (insert text) (funcall string) (romacs--treesit-parse-early 'c)) \
                (with-temp-buffer (insert text) (romacs--treesit-parse-early 'c) (let ((p (treesit-parser-create 'c))) (treesit-parser-set-included-ranges p '((1 . 7))) (equal (treesit-node-string (treesit-parser-root-node p)) (funcall fresh \"int a;\")))))))");
    let value = vm.eval_source(&source).and_then(|v| v.display());
    assert_eq!(value.as_deref(), Ok("((t nil t) t t t t nil t)"));
}

/// A frame `defer_parse' starts shows a small edit without its parse, and
/// so without `pre-redisplay-functions' or fontification; the next frame
/// runs them, however small its own edit. A larger edit never waits.
#[test]
fn a_small_edit_shows_for_one_frame_before_its_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    // It counts hook calls, and a collection between frames tells every window.
    vm.set_gc_stress(0);
    vm.eval_source("(require 'treesit)").unwrap();
    let setup = format!("(progn {SYSTEM_SETUP} \
        (defvar romacs-treesit-show-before-parse 8) \
        (setq parsed 0 pre 0 fontified 0) \
        (defalias 'treesit-test-parsed (lambda (_ranges _parser) (setq parsed (1+ parsed)))) \
        (set-window-buffer nil (get-buffer-create \"ts\")) (set-buffer \"ts\") (insert \"int a;\\n\") \
        (setq treesit-test-parser (treesit-parser-create 'c)) \
        (treesit-parser-add-notifier treesit-test-parser 'treesit-test-parsed) \
        (add-hook 'pre-redisplay-functions (lambda (_) (setq pre (1+ pre)) (treesit-parser-root-node treesit-test-parser))) \
        (setq fontification-functions (list (lambda (pos) (setq fontified (1+ fontified)) (put-text-property pos (point-max) 'fontified t)))))");
    vm.eval_source(&setup).unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    let counts = |vm: &mut Vm| vm.eval_source("(list parsed pre (> fontified 0))").and_then(|v| v.display()).unwrap();
    assert_eq!(counts(&mut vm), "(1 1 t)");
    vm.eval_source("(progn (setq fontified 0) (goto-char (point-max)) (insert \"x\"))").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(counts(&mut vm), "(1 1 nil)", "a small edit's frame waits for nothing");
    assert!(vm.deferred_parse_done());
    vm.eval_source("(insert \"y\")").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(counts(&mut vm), "(2 2 t)", "the next frame parses, a new edit or not");
    assert!(!vm.deferred_parse_done());
    vm.eval_source("(progn (setq fontified 0) (insert (make-string 100 ?z)))").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(counts(&mut vm), "(3 3 t)", "a large edit waits for its parse");
    assert!(!vm.deferred_parse_done());
    // A parser nothing asks, as an embedded one out of view, holds no one up.
    vm.eval_source("(setq treesit-test-idle (treesit-parser-create 'c nil nil 'idle))").unwrap();
    for _ in 0..3 {
        vm.eval_source("(insert \"abcdef\")").unwrap();
        vm.defer_parse().unwrap();
        vm.frame_snapshot(800, 600).unwrap();
        assert!(vm.deferred_parse_done(), "an idle parser's backlog is not this edit");
        vm.defer_parse().unwrap();
        vm.frame_snapshot(800, 600).unwrap();
        assert!(!vm.deferred_parse_done());
    }
    // Nor does a parser a query ran, which leaves the shown one still to parse.
    vm.eval_source("(progn (insert \"x\") (treesit-parser-root-node treesit-test-idle))").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert!(vm.deferred_parse_done(), "another parser's parse is not the frame's");
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert!(!vm.deferred_parse_done());
    vm.eval_source("(treesit-parser-delete treesit-test-idle)").unwrap();
    // The frame after tells the hooks what changed since they last ran, which
    // the deferred frame has already recorded as shown: here a selection.
    vm.eval_source("(progn (setq treesit-test-seen nil) \
        (add-hook 'pre-redisplay-functions (lambda (w) (push (eq w (selected-window)) treesit-test-seen))) \
        (split-window))").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    vm.eval_source("(progn (setq treesit-test-seen nil) (select-window (next-window)) (insert \"x\"))").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(vm.eval_source("treesit-test-seen").and_then(|v| v.display()).unwrap(), "nil", "no hook ran");
    assert!(vm.deferred_parse_done());
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert!(!vm.deferred_parse_done());
    assert_eq!(vm.eval_source("(and (memq nil treesit-test-seen) (memq t treesit-test-seen) t)").and_then(|v| v.display()).unwrap(), "t",
        "the window selection left and the one it reached both ran");
    vm.eval_source("(progn (setq romacs-treesit-show-before-parse 0) (insert \"x\"))").unwrap();
    vm.defer_parse().unwrap();
    assert!(!vm.deferred_parse_done(), "zero shows nothing early");
}

/// Rows of 25 pixels, cells of 10.
struct Cells;
impl romacs_vm::LayoutHost for Cells {
    fn measure(&mut self, input: &romacs_layout_input::MeasureInput) -> Result<romacs_layout_input::Measured, String> {
        let chars = input.text.utf8.chars().count();
        let heights = vec![25; input.faces.len()];
        Ok(romacs_layout_input::Measured { glyphs: vec![(10, 25); chars], ascents: vec![20; chars], face_ascents: vec![20; heights.len()], face_heights: heights, fallback: vec![false; chars] })
    }
    fn lay_out(&mut self, frame: &romacs_layout_input::FrameSnapshot) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        Ok(frame.windows.iter().map(|w| {
            let len = w.snapshot.text.utf8.len();
            romacs_vm::WindowEnd { id: w.snapshot.id, end: len, visible_end: len, body_height: w.snapshot.height as f32, rows: vec![], point: None }
        }).collect())
    }
}

/// While a visit's first parse runs on its thread (lisp/romacs/romacs-treesit.el),
/// a frame leaves the tree alone: `treesit--pre-redisplay' would wait for the
/// parse, and the window shows its text plain meanwhile. The first frame after
/// the parse lands runs it, and marks the whole text for its faces. The parse
/// in flight is the primitive's answer here, so no thread's timing decides it.
#[test]
fn a_frame_waits_for_no_first_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    vm.set_gc_stress(0);
    vm.eval_source("(progn (require 'treesit) (load \"romacs-treesit\" nil t))").unwrap();
    let setup = format!("(progn {SYSTEM_SETUP} \
        (setq parsed 0 treesit-test-parsing t) \
        (advice-add 'romacs--treesit-parsing-p :override (lambda () treesit-test-parsing)) \
        (set-window-buffer nil (get-buffer-create \"ts\")) (set-buffer \"ts\") (insert \"int a;\\n\") \
        (setq-local treesit-primary-parser (treesit-parser-create 'c)) \
        (defalias 'treesit-test-parsed (lambda (_ranges _parser) (setq parsed (1+ parsed)))) \
        (treesit-parser-add-notifier treesit-primary-parser 'treesit-test-parsed) \
        (add-hook 'pre-redisplay-functions #'treesit--pre-redisplay 0 t) \
        (setq fontification-functions nil))");
    vm.eval_source(&setup).unwrap();
    let state = |vm: &mut Vm| vm.eval_source("(list parsed (with-current-buffer \"ts\" (get-text-property 1 'fontified)))")
        .and_then(|v| v.display()).unwrap();
    vm.eval_source("(put-text-property 1 (point-max) 'fontified t)").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(state(&mut vm), "(0 t)", "no frame asks for the tree while the first parse runs");
    vm.eval_source("(setq treesit-test-parsing nil)").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(state(&mut vm), "(1 nil)", "the frame after it lands parses and marks the text for its faces");
}

/// `redisplay' between a deferred frame and the next tells the hooks what
/// changed: the next frame does not tell them again.
#[test]
fn redisplay_after_a_deferred_frame_settles_the_hooks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    vm.eval_source("(require 'treesit)").unwrap();
    vm.set_layout_host(Some(Box::new(Cells)));
    let setup = format!("(progn {SYSTEM_SETUP} \
        (setq noninteractive nil seen nil) \
        (defvar romacs-treesit-show-before-parse 8) (setq romacs-treesit-show-before-parse 8) \
        (set-window-buffer nil (get-buffer-create \"ts\")) (set-buffer \"ts\") (insert \"int a;\\n\") \
        (treesit-parser-create 'c) \
        (setq fontification-functions (list (lambda (pos) (put-text-property pos (point-max) 'fontified t)))) \
        (with-current-buffer (get-buffer-create \"other\") (insert \"some text\")) \
        (set-window-buffer (split-window) \"other\") \
        (add-hook 'pre-redisplay-functions (lambda (w) (push (buffer-name (window-buffer w)) seen))))");
    vm.eval_source(&setup).unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    vm.eval_source("(progn (insert \"x\") (set-window-point (next-window) 3))").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert!(vm.deferred_parse_done());
    vm.eval_source("(progn (setq seen nil) (redisplay))").unwrap();
    assert_eq!(vm.eval_source("(and (member \"other\" seen) t)").and_then(|v| v.display()).unwrap(), "t", "redisplay names the moved window");
    vm.eval_source("(setq seen nil)").unwrap();
    vm.defer_parse().unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(vm.eval_source("(and (member \"other\" seen) t)").and_then(|v| v.display()).unwrap(), "nil", "and the next frame does not again");
}

/// VM2-TREESIT-HASH: `equal' nodes hash alike, so an `equal' table finds
/// them (GNU hashes the node object's address and misses).
#[test]
fn equal_nodes_hash_alike() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    let source = format!("(progn {SYSTEM_SETUP} \
        (with-temp-buffer (insert \"int a;\") (let* ((root (treesit-parser-root-node (treesit-parser-create 'c))) \
          (a (treesit-node-child root 0)) (b (treesit-node-child root 0)) (table (make-hash-table :test 'equal))) \
          (puthash a 'found table) \
          (list (eq a b) (equal a b) (= (sxhash-equal a) (sxhash-equal b)) (gethash b table)))))");
    let value = vm.eval_source(&source).and_then(|v| v.display());
    assert_eq!(value.as_deref(), Ok("(nil t t found)"));
}

/// A predicate that edits the buffer and forces a reparse: GNU deletes the
/// tree its cursor walks (a use-after-free that can crash the pin), so
/// these are checked against treesit.c's code instead: `#pred?' signals
/// `(treesit-query-error FN)` (`treesit.c:3860`), a search predicate
/// `(treesit-buffer_changed FN)` (`treesit_pred_with_guard`).
#[test]
fn reparsing_predicates_signal_as_treesit_c() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = bootstrapped(&root);
    let source = format!("(progn {SYSTEM_SETUP} \
        (defalias 'treesit-test-reparse (lambda (n) (insert \"x\") (treesit-parser-root-node (treesit-node-parser n)) t)) \
        (with-temp-buffer (insert \"int a[] = {{1, 2}};\") (let ((root (treesit-parser-root-node (treesit-parser-create 'c)))) \
          (list (condition-case e (treesit-query-capture root '((((number_literal) @n) (:pred treesit-test-reparse @n)))) (error e)) \
                (condition-case e (treesit-search-subtree (treesit-parser-root-node (car (treesit-parser-list))) 'treesit-test-reparse) (error e))))))");
    let value = vm.eval_source(&source).and_then(|v| v.display());
    assert_eq!(value.as_deref(), Ok("((treesit-query-error treesit-test-reparse) (treesit-buffer_changed treesit-test-reparse))"));
}
