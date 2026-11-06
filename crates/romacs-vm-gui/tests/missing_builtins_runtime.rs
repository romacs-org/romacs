//! GNU DEFUNs whose Romacs definitions need the whole runtime (Lisp ones
//! among them) or read what it loads, in `romacs-vm-gui -Q --batch' against
//! pinned GNU's `emacs -Q --batch': one form a line, each printed.
use std::process::Command;

const FORMS: &str = r###"
(list (subr-type (symbol-function 'car)) (subr-native-lambda-list (symbol-function 'car)) (subr-type (symbol-function 'if)))
(list (condition-case e (subr-type 3) (error e)) (condition-case e (subr-native-lambda-list 'car) (error e)))
(list (charset-id-internal 'ascii) (charset-id-internal 'unicode) (charset-id-internal 'iso-8859-1) (condition-case e (charset-id-internal) (error e)) (condition-case e (charset-id-internal 'nope) (error e)))
(list (sort-charsets (list 'iso-8859-1 'unicode 'ascii)) (sort-charsets nil) (condition-case e (sort-charsets '(nope)) (error e)) (condition-case e (sort-charsets 3) (error e)) (clear-charset-maps))
(let ((l (list 'latin-iso8859-5 'iso-8859-1 'ascii))) (eq (sort-charsets l) l))
(list (ccl-program-p 'x) (ccl-program-p [1 2]) (ccl-program-p 3) (progn (put 'cclx 'ccl-program-idx 0) (ccl-program-p 'cclx)) (progn (put 'cclx 'ccl-program-idx 100000) (ccl-program-p 'cclx)))
(progn (defvaralias 'va1 'va2 "doc") (setq va2 3) (list (internal-delete-indirect-variable 'va1) (boundp 'va1) (get 'va1 'variable-documentation) va2 (progn (setq va1 7) (list va1 va2))))
(list (condition-case e (internal-delete-indirect-variable 'va9) (error e)) (condition-case e (internal-delete-indirect-variable 3) (error e)))
(let ((print-circle nil)) (print--preprocess '(a)))
(let ((print-circle t) (print-number-table (make-hash-table :test 'eq)) (x (list 1))) (list (print--preprocess (list x x (vector x))) (hash-table-count print-number-table) (gethash x print-number-table)))
(let ((print-circle t) (x (list 1))) (list (print--preprocess (list x x)) print-number-table))
(let ((print-circle t) (print-number-table (make-hash-table :test 'eq)) (s (copy-sequence "abc"))) (print--preprocess (list s s 'u (make-symbol "z"))) (hash-table-count print-number-table))
(let ((print-circle t) (print-gensym t) (print-number-table (make-hash-table :test 'eq)) (g (make-symbol "z"))) (print--preprocess (list g g)) (list (hash-table-count print-number-table) (gethash g print-number-table)))
(list (color-distance "red" "blue") (color-distance "red" "red") (color-distance '(0 0 0) '(65535 65535 65535)) (color-distance "#ff0000" "#00ff00") (color-distance "red" "blue" nil (lambda (a b) (list a b))))
(list (condition-case e (color-distance "nope" "blue") (error e)) (condition-case e (color-distance 3 "blue") (error e)) (condition-case e (color-distance '(1 2) "blue") (error e)) (condition-case e (color-distance "red" "nope") (error e)))
(list (color-supported-p "red") (color-supported-p "red" nil t) (color-supported-p "nope") (condition-case e (color-supported-p 3) (error e)))
(list (natnump (gap-size)) (face-attributes-as-vector '(:foreground "red" :weight bold)) (face-attributes-as-vector nil) (face-attributes-as-vector '(:inherit bold)) (face-attributes-as-vector '(:bogus 1)) (face-attributes-as-vector '(bold)))
(list (face-attributes-as-vector '(:height 1.5 :box t :underline "red" :overline 3 :extend t :weight bogus :slant italic :width condensed)) (face-attributes-as-vector '((:weight bold) (:foreground "x"))) (face-attributes-as-vector '(foreground-color . "c")) (face-attributes-as-vector '(background-color . "c")) (face-attributes-as-vector 'no-such-face) (face-attributes-as-vector '(:inherit (bold italic) :height 2.0)) (face-attributes-as-vector 'italic) (face-attributes-as-vector '(:family unspecified :foreground 3)))
(let ((a '(:weight bold)) (b '(:weight bold :foreground "red"))) (require 'face-remap) (list (face-attrs-more-relative-p a b) (face-attrs-more-relative-p b a) (with-temp-buffer (face-remap-add-relative 'default :height 1.2) (face-remap-add-relative 'default :weight 'bold) face-remapping-alist)))
(let ((m (point-marker))) (list (marker-last-position m) (progn (set-marker m nil) (marker-last-position m)) (marker-position m)))
(list (condition-case e (marker-last-position 3) (error e)) (marker-last-position (make-marker)))
(with-temp-buffer (insert "abc") (let ((m (point-marker))) (list (marker-last-position m) (prog1 (marker-position m) (set-marker m nil)) (marker-last-position m) (marker-position m))))
(let (m) (with-temp-buffer (insert "abc") (setq m (point-marker))) (list (marker-position m) (marker-last-position m)))
(let (m) (with-temp-buffer (insert "abcd") (setq m (copy-marker 3)) (goto-char 1) (insert "xy")) (marker-last-position m))
(list (condition-case e (delete-terminal) (error e)) (condition-case e (delete-terminal (frame-terminal)) (error e)) (condition-case e (delete-terminal (selected-frame)) (error e)) (delete-terminal 3))
(condition-case e (suspend-emacs 3) (error e))
(list (let (m) (with-temp-buffer (insert "abcd") (let ((b (make-indirect-buffer (current-buffer) "ind"))) (with-current-buffer b (setq m (copy-marker 3))) (kill-buffer b) (list (marker-position m) (marker-last-position m))))))
(let (m i) (with-temp-buffer (insert "abcd") (setq i (make-indirect-buffer (current-buffer) "ind2")) (with-current-buffer i (setq m (copy-marker 3)))) (list (buffer-live-p i) (marker-position m) (marker-last-position m)))
(let ((face-remapping-alist '((bold :foreground "red")))) (list (aref (face-attributes-as-vector 'bold) 5) (aref (face-attributes-as-vector 'bold) 9)))
(let ((face-remapping-alist '((bold bold :foreground "red")))) (list (aref (face-attributes-as-vector 'bold) 5) (aref (face-attributes-as-vector 'bold) 9)))
(let ((face-remapping-alist '((bold italic) (italic bold)))) (list (aref (face-attributes-as-vector 'bold) 5) (aref (face-attributes-as-vector 'bold) 6)))
(with-temp-buffer (require 'face-remap) (setq-local face-remapping-alist '((bold :foreground "red"))) (list (face-attrs-more-relative-p 'bold '(:foreground "red" :weight bold))))
(list (aref (face-attributes-as-vector "bold") 5) (aref (face-attributes-as-vector (list :height (lambda (_) 2.0))) 4) (commandp 'suspend-emacs))
(let ((print-circle t) (print-gensym t) (print-continuous-numbering t) (print-number-table (make-hash-table :test 'eq)) (g (make-symbol "z"))) (print--preprocess g) (gethash g print-number-table))
(progn (put 'romacs-test-alias 'face-alias 'bold) (let ((face-remapping-alist '((bold :foreground "red")))) (list (aref (face-attributes-as-vector 'romacs-test-alias) 9))))
(progn (let ((prev 'bold)) (dotimes (i 12) (let ((name (intern (format "romacs-test-alias-%d" i)))) (put name 'face-alias prev) (setq prev name))) (let ((face-remapping-alist '((bold :foreground "red")))) (aref (face-attributes-as-vector prev) 9))))
(let ((b (expt 2 80)) (c (- (expt 3 60)))) (list (truncate b 7) (truncate b c) (truncate c 7) (truncate 7 b) (truncate 7.5 b) (truncate b 2.5) (floor b 7) (floor c 7) (floor c b) (floor 7 b) (floor -7 b) (floor b 2.5) (ceiling b 7) (ceiling c 7) (ceiling 7 b) (ceiling -7 b) (round b 7) (round c 7) (round b 3) (round 7 b) (round (* 3 b) (* 2 b)) (round (* 5 b) (* 2 b)) (round 1.0e300 1.0e-300) (floor 1.0e300 3) (truncate 5.0e-324 1.0e300)))
(let ((b (expt 2 80))) (list (condition-case e (truncate b 0) (error e)) (condition-case e (floor b 0.0) (error e)) (condition-case e (round b 1.0e+INF) (error e)) (condition-case e (ceiling b 0.0e+NaN) (error e)) (condition-case e (floor 'a b) (error e)) (condition-case e (floor b 'a) (error e))))
(let ((b (expt 2 80)) (c (- (expt 3 60)))) (list (% b 7) (% c 7) (% b c) (% 7 b) (% 7 3) (% -7 3) (oddp b) (evenp b) (with-temp-buffer (insert "abcde") (list (% (copy-marker 5) 3) (% 7 (copy-marker 4))))))
(list (condition-case e (% (expt 2 80) 0) (error e)) (condition-case e (% 7 0) (error e)) (condition-case e (% 1.5 2) (error e)) (condition-case e (% 'a 2) (error e)) (condition-case e (% 7 2.5) (error e)) (condition-case e (funcall '% 7 2.5) (error e)) (condition-case e (funcall '% 7 0) (error e)) (condition-case e (% 7 (make-marker)) (error e)))

(let* ((b (expt 2 80)) (n (- b)) (c (list 1 2 3 4 5))) (setcdr (nthcdr 4 c) (nthcdr 2 c)) (list (nth b '(1)) (nthcdr b '(1)) (nth n '(1 2)) (nthcdr n '(1 2)) (nth b nil) (nthcdr b nil) (car (nthcdr b c)) (nth b c) (nth (1+ b) c) (nth (+ b 2) c) (car (nthcdr (+ b 2) c)) (funcall 'nth b c) (condition-case e (nth b '(1 . 2)) (error e)) (condition-case e (nthcdr b '(1 . 2)) (error e)) (condition-case e (nth 1.5 '(1)) (error e)) (condition-case e (nthcdr 1.5 '(1)) (error e)) (let ((r (list 1))) (setcdr r r) (nth b r))))
(let* ((_ (require 'cl-lib)) (keep nil) (d (lambda (f) (let ((a (memory-use-counts))) (setq keep (funcall f)) (mapcar (lambda (x) (> x 0)) (cl-mapcar #'- (memory-use-counts) a)))))) (list (length (memory-use-counts)) (seq-every-p #'natnump (memory-use-counts)) (nth 0 (funcall d (lambda () (cons 1 2)))) (nth 1 (funcall d (lambda () (* 1.5 (float (random 9)))))) (nth 2 (funcall d (lambda () (make-vector 10 0)))) (nth 3 (funcall d (lambda () (make-symbol "x")))) (nth 4 (funcall d (lambda () (make-string 5 ?a)))) (nth 6 (funcall d (lambda () (make-string 5 ?a)))) (let ((a (memory-use-counts))) (garbage-collect) (seq-every-p #'identity (cl-mapcar #'<= a (memory-use-counts))))))
(progn (require 'window-tool-bar) (stringp (window-tool-bar-string)))
(let* ((a (nth 2 (memory-use-counts))) (keep (lambda () 42))) (list (functionp keep) (> (- (nth 2 (memory-use-counts)) a) 0)))
(with-temp-buffer (defvar memcount-probe 1) (let ((a (nth 3 (memory-use-counts)))) (make-local-variable 'memcount-probe) (- (nth 3 (memory-use-counts)) a)))
"###;

#[test]
fn runtime_builtins_match_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-missing-builtins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let forms: Vec<&str> = FORMS.lines().filter(|line| !line.is_empty()).collect();
    let program = forms.iter().map(|form| format!("(princ (concat (prin1-to-string (condition-case e {form} (error (list 'signal e)))) \"\\n\"))"))
        .collect::<Vec<_>>().join("\n");
    let run = |binary: &str| {
        let output = Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(progn {program})")]).current_dir(&dir)
            .env("HOME", &dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
            .output().unwrap();
        assert!(output.status.success(), "{binary}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let (expected, actual) = (run(&gnu), run(env!("CARGO_BIN_EXE_romacs-vm-gui")));
    let failures: Vec<String> = forms.iter().zip(expected.lines().zip(actual.lines())).filter(|(_, (e, a))| e != a)
        .map(|(form, (e, a))| format!("{form}\n  GNU    {e}\n  Romacs {a}")).collect();
    assert_eq!(expected.lines().count(), actual.lines().count(), "{actual}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `suspend-emacs' stops the process group, between its two hooks, as
/// keyboard.c does. No oracle: GNU reads its terminal's size here and has
/// none in batch.
#[test]
fn suspend_emacs_stops_between_its_hooks() {
    use std::os::unix::process::CommandExt;
    let dir = std::env::temp_dir().join(format!("romacs-suspend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = "(progn (add-hook 'suspend-hook (lambda () (princ \"before\\n\")))
                       (add-hook 'suspend-resume-hook (lambda () (princ \"after\\n\")))
                       (prin1 (suspend-emacs)))";
    let run = |binary: &str| {
        let child = Command::new(binary).args(["-Q", "--batch", "--eval", form]).current_dir(&dir).env("HOME", &dir)
            .stdout(std::process::Stdio::piped()).process_group(0).spawn().unwrap();
        let stat = format!("/proc/{}/stat", child.id());
        let stopped = || std::fs::read_to_string(&stat).is_ok_and(|text| text.rsplit(") ").next().unwrap_or("").starts_with('T'));
        let started = std::time::Instant::now();
        while !stopped() {
            assert!(started.elapsed().as_secs() < 30, "{binary} did not stop");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(Command::new("kill").args(["-CONT", &child.id().to_string()]).status().unwrap().success());
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(run(env!("CARGO_BIN_EXE_romacs-vm-gui")), "before\nafter\nnil");
}
