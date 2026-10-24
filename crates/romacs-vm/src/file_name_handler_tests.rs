//! Live GNU 32.0.50 find-file-name-handler regressions.
//! Sources/results: evidence/file-name-handler/{oracle.el,recorded-cases.json}.
//! Wire from lib.rs using #[cfg(test)] mod file_name_handler_tests;
use super::*;

fn show(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source_raw(source) {
        Ok(value) => vm.display(value).unwrap(),
        Err(error) => format!("ERR {error}"),
    }
}

macro_rules! gnu_case {
    ($name:ident, $source:literal, $expected:literal) => {
        #[test]
        fn $name() {
            let mut vm = Vm::new();
            assert_eq!(show(&mut vm, $source), $expected, stringify!($name));
        }
    };
}

gnu_case!(
    nil_alist,
    r###"(condition-case e (let ((file-name-handler-alist nil) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"nil"###
);

gnu_case!(
    matching,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"middle"###
);

gnu_case!(
    nonmatching,
    r###"(condition-case e (let ((file-name-handler-alist '(("z" . absent))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"nil"###
);

gnu_case!(
    greatest_position,
    r###"(condition-case e (let ((file-name-handler-alist '(("a" . start) ("c" . end) ("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"end"###
);

gnu_case!(
    first_tie,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . first) ("bc" . second))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"first"###
);

gnu_case!(
    case_sensitive,
    r###"(condition-case e (let ((file-name-handler-alist '(("abc" . folded))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (let ((case-fold-search t)) (find-file-name-handler "ABC" 'read))) (error e))"###,
    r###"nil"###
);

gnu_case!(
    operation_unrestricted,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . fh-unrestricted))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-unrestricted 'operations nil) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"fh-unrestricted"###
);

gnu_case!(
    operation_allowed,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . fh-allowed))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-allowed 'operations '(write read)) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"fh-allowed"###
);

gnu_case!(
    operation_filtered,
    r###"(condition-case e (let ((file-name-handler-alist '(("a" . fallback) ("c" . fh-filtered))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-filtered 'operations '(write)) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"fallback"###
);

gnu_case!(
    nil_operation,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . fh-nil-op))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-nil-op 'operations '(nil)) (find-file-name-handler "abc" nil)) (error e))"###,
    r###"fh-nil-op"###
);

gnu_case!(
    nonsymbol_handler,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" handler payload))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"(handler payload)"###
);

gnu_case!(
    nil_handler,
    r###"(condition-case e (let ((file-name-handler-alist '(("c" . nil) ("b" . fallback))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"nil"###
);

gnu_case!(
    inhibition_same_operation,
    r###"(condition-case e (let ((file-name-handler-alist '(("c" . end) ("b" . fallback))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (let ((inhibit-file-name-operation 'read) (inhibit-file-name-handlers '(end))) (find-file-name-handler "abc" 'read))) (error e))"###,
    r###"fallback"###
);

gnu_case!(
    inhibition_different_operation,
    r###"(condition-case e (let ((file-name-handler-alist '(("c" . end) ("b" . fallback))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (let ((inhibit-file-name-operation 'write) (inhibit-file-name-handlers '(end))) (find-file-name-handler "abc" 'read))) (error e))"###,
    r###"end"###
);

gnu_case!(
    inhibition_nil_operation,
    r###"(condition-case e (let ((file-name-handler-alist '(("c" . end) ("b" . fallback))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (let ((inhibit-file-name-handlers '(end))) (find-file-name-handler "abc" nil))) (error e))"###,
    r###"fallback"###
);

gnu_case!(
    match_data_preserved,
    r###"(condition-case e (let ((file-name-handler-alist '(("a" . a) ("c" . c))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (progn (string-match "\\(b\\)" "abc") (let ((before (match-data))) (find-file-name-handler "xxabc" 'read) (equal before (match-data))))) (error e))"###,
    r###"t"###
);

gnu_case!(
    match_data_nil_preserved,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (progn (set-match-data nil) (find-file-name-handler "abc" 'read) (match-data))) (error e))"###,
    r###"nil"###
);

gnu_case!(
    invalid_filename_nil,
    r###"(condition-case e (let ((file-name-handler-alist nil) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler nil 'read)) (error e))"###,
    r###"(wrong-type-argument stringp nil)"###
);

gnu_case!(
    invalid_filename_integer,
    r###"(condition-case e (let ((file-name-handler-alist nil) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler 42 'read)) (error e))"###,
    r###"(wrong-type-argument stringp 42)"###
);

gnu_case!(
    invalid_regexp,
    r###"(condition-case e (let ((file-name-handler-alist '(("[" . broken))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"(invalid-regexp "Unmatched [ or [^")"###
);

gnu_case!(
    invalid_regexp_even_filtered,
    r###"(condition-case e (let ((file-name-handler-alist '(("[" . fh-broken-filtered))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-broken-filtered 'operations '(write)) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"(invalid-regexp "Unmatched [ or [^")"###
);

gnu_case!(
    invalid_regexp_match_data_preserved,
    r###"(condition-case e (let ((file-name-handler-alist '(("[" . broken))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (progn (string-match "b" "abc") (let ((before (match-data))) (condition-case nil (find-file-name-handler "abc" 'read) (error nil)) (equal before (match-data))))) (error e))"###,
    r###"t"###
);

gnu_case!(
    nonlist_alist_ignored,
    r###"(condition-case e (let ((file-name-handler-alist 42) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"nil"###
);

gnu_case!(
    dotted_alist_tail_ignored,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . middle) . 42)) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"middle"###
);

gnu_case!(
    malformed_entries_ignored,
    r###"(condition-case e (let ((file-name-handler-alist '(42 nil (7 . ignored) ("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"middle"###
);

gnu_case!(
    invalid_operations_list,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . fh-invalid-ops))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-invalid-ops 'operations 42) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"(wrong-type-argument listp 42)"###
);

gnu_case!(
    dotted_operations_success,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . fh-dotted-ops))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-dotted-ops 'operations '(read . 42)) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"fh-dotted-ops"###
);

gnu_case!(
    dotted_operations_failure,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . fh-dotted-fail))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-dotted-fail 'operations '(write . 42)) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"(wrong-type-argument listp (write . 42))"###
);

gnu_case!(
    invalid_inhibition_list,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (let ((inhibit-file-name-operation 'read) (inhibit-file-name-handlers 42)) (find-file-name-handler "abc" 'read))) (error e))"###,
    r###"(wrong-type-argument listp 42)"###
);

gnu_case!(
    invalid_inhibition_different_operation_ignored,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (let ((inhibit-file-name-operation 'write) (inhibit-file-name-handlers 42)) (find-file-name-handler "abc" 'read))) (error e))"###,
    r###"middle"###
);

gnu_case!(
    filtered_invalid_operations_not_examined_at_earlier_position,
    r###"(condition-case e (let ((file-name-handler-alist '(("c" . end) ("a" . fh-earlier-invalid))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) (put 'fh-earlier-invalid 'operations 42) (find-file-name-handler "abc" 'read)) (error e))"###,
    r###"end"###
);

gnu_case!(
    funcall,
    r###"(condition-case e (let ((file-name-handler-alist '(("b" . middle))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil))  (funcall 'find-file-name-handler "abc" 'read)) (error e))"###,
    r###"middle"###
);

