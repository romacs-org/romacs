use romacs_vm::Vm;

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let actual = vm.eval_source(source).and_then(|v| v.display()).unwrap();
    assert_eq!(actual, expected, "{source}");
}

#[test]
fn ascii_mail_whitespace_and_signed_motion() {
    check(r#"(progn (insert " \t\nHeader: value") (goto-char 1) (list (skip-chars-forward " \t\n") (point) (skip-chars-backward " \t\n") (point)))"#, "(3 4 -3 1)");
}

#[test]
fn unicode_literal_ranges_and_character_limits() {
    check(r#"(progn (insert "αβγ雪😀!") (goto-char 1) (list (skip-chars-forward "α-γ雪😀" 5) (point) (skip-chars-forward "😀") (point) (skip-chars-backward "α-γ雪😀" 2) (point)))"#, "(4 5 1 6 -4 2)");
}

#[test]
fn narrowing_clamps_limits_without_rebasing_positions() {
    check(r#"(progn (insert "xλλλy") (narrow-to-region 2 5) (goto-char 3) (list (skip-chars-forward "λ" 999) (point) (skip-chars-backward "λ" -999) (point) (point-min) (point-max)))"#, "(2 5 -3 2 2 5)");
}

#[test]
fn limits_behind_motion_do_not_move_point() {
    check(r#"(progn (insert "aaaa") (goto-char 3) (list (skip-chars-forward "a" 1) (point) (skip-chars-backward "a" 5) (point)))"#, "(0 3 0 3)");
}

#[test]
fn bignum_limits_clamp_to_the_accessible_interval() {
    check(r#"(progn (insert "aaa") (goto-char 1) (list (skip-chars-forward "a" #x10000000000000000) (point) (skip-chars-backward "a" #x-10000000000000000) (point)))"#, "(3 4 -3 1)");
}

#[test]
fn marker_limit_uses_character_position() {
    check(r#"(progn (insert "λλλ!") (goto-char 3) (setq skip-limit (copy-marker (point))) (goto-char 1) (list (skip-chars-forward "λ" skip-limit) (point)))"#, "(2 3)");
}

#[test]
fn negation_and_literal_closing_bracket() {
    check(r#"(progn (insert "]]-^abc!") (goto-char 1) (list (skip-chars-forward "]\\-\\^") (point) (skip-chars-forward "^!") (point)))"#, "(4 5 3 8)");
}

#[test]
fn escaping_ranges_reversed_ranges_and_trailing_backslash() {
    check(r#"(progn (insert "abc-\\!") (goto-char 1) (list (skip-chars-forward "z-a") (skip-chars-forward "a-\\c") (skip-chars-forward "-\\\\") (skip-chars-forward "!\\") (point)))"#, "(0 3 2 1 7)");
}

#[test]
fn empty_set_and_complement() {
    check(r#"(progn (insert "λx") (goto-char 1) (list (skip-chars-forward "") (point) (skip-chars-forward "^") (point)))"#, "(0 1 2 3)");
}

#[test]
fn bytebuffer_raw_high_octets_and_narrowing() {
    check(r#"(progn (set-buffer-multibyte nil) (insert 255 254 97 255) (narrow-to-region 2 5) (goto-char 2) (list (skip-chars-forward (unibyte-string 254 97 255)) (point) (skip-chars-backward (unibyte-string 254 97 255) 1) (point)))"#, "(3 5 -3 2)");
}

#[test]
fn multibyte_latin1_is_not_a_raw_byte() {
    check(r#"(progn (set-buffer-multibyte nil) (insert 233) (goto-char 1) (list (skip-chars-forward "é") (skip-chars-forward (unibyte-string 233)) (point)))"#, "(0 1 2)");
}

#[test]
fn motion_and_zero_match_preserve_text_ticks_and_read_only() {
    check(r#"(progn (insert "aaa!") (goto-char 1) (set-buffer-modified-p nil) (setq buffer-read-only t skip-before (buffer-modified-tick) skip-chars-before (buffer-chars-modified-tick)) (list (skip-chars-forward "a") (skip-chars-forward "a") (= skip-before (buffer-modified-tick)) (= skip-chars-before (buffer-chars-modified-tick)) (buffer-modified-p) (buffer-string)))"#, "(3 0 t t nil \"aaa!\")");
}

#[test]
fn invalid_arguments_and_unset_marker_do_not_move_point() {
    check(r#"(progn (insert "aaa") (goto-char 1) (list (condition-case e (skip-chars-forward 42) (error (car e))) (condition-case e (skip-chars-backward "a" 'bad) (error (car e))) (condition-case e (skip-chars-forward "a" (make-marker)) (error (car e))) (point)))"#, "(wrong-type-argument wrong-type-argument error 1)");
}

#[test]
fn foreign_marker_limit_and_motion_belong_to_current_buffer() {
    check(r#"(progn (setq skip-home (current-buffer) skip-other (get-buffer-create "skip-other")) (set-buffer skip-other) (insert "雪雪雪") (goto-char 3) (setq skip-limit (copy-marker (point))) (set-buffer skip-home) (insert "λλλλ") (goto-char 1) (setq skip-distance (skip-chars-forward "λ" skip-limit) skip-point (point)) (set-buffer skip-other) (list skip-distance skip-point (point)))"#, "(2 3 3)");
}

#[test]
fn match_data_is_not_overwritten() {
    check(r#"(progn (insert "aaa") (goto-char 1) (string-match "x" "xx") (setq skip-match (match-data)) (list (skip-chars-forward "a") (equal skip-match (match-data))))"#, "(3 t)");
}

/// Pinned GNU 32.0.50 answers (3 4); skip_chars_classes.rs compares more.
#[test]
fn named_character_classes_combine_with_characters() {
    check(r#"(progn (insert "aaa") (goto-char 1) (list (condition-case e (skip-chars-forward "a[:alpha:]") (error (car (cdr e)))) (point)))"#, "(3 4)");
}
