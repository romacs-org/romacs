use super::*;
use crate::prim::leaf::RegionCase;

defsubr! { name = "upcase", gnu = "casefiddle.c:368", effect = Reads,
fn upcase(cx, value: Value) { cx.case_convert(value, false) }}

defsubr! { name = "downcase", gnu = "casefiddle.c:383", effect = Reads,
fn downcase(cx, value: Value) { cx.case_convert(value, true) }}

fn flag(b: bool) -> Value { if b { Value::TRUE } else { Value::NIL } }
fn fixnum(n: i64) -> Value { Value::integer(n).expect("buffer position") }

/// The region commands and the case each applies, by register code.
const REGION_CASES: [(&str, RegionCase); 4] = [("upcase-region", RegionCase::Up), ("downcase-region", RegionCase::Down),
    ("capitalize-region", RegionCase::Capitalize), ("upcase-initials-region", RegionCase::InitialsUp)];

fn region_case(code: Value) -> RegionCase {
    REGION_CASES[code.integer_value().expect("region case code") as usize].1
}

// casify_pnc_region and casify_region (casefiddle.c:529): validate_region,
// modify_text's bracket over the region, each character cased in place,
// and one after-change over what changed. Registers between the steps:
//   [START, END, CASE, INHIBITED]
// CASE is the fixnum index of the command in REGION_CASES.
fn casify_region(cx: &mut Leaf<'_>, case: Value, beg: Value, end: Value, noncontiguous: Value) -> Next {
    if noncontiguous != Value::NIL {
        let function = cx.intern(REGION_CASES[case.integer_value().expect("region case code") as usize].0);
        if super::insdel::half_missing(cx)? {
            return super::insdel::install_half(cx, casify_restart, vec![case, beg, end, noncontiguous]);
        }
        return Ok(Step::Call { f: cx.intern("romacs--casify-noncontiguous"), args: vec![function], then: casified, saved: vec![] });
    }
    let (start, end) = cx.validate_region(beg, end)?;
    if start == end { return Ok(Step::Done(Value::NIL)); }
    let prepared = cx.prepare_change(start, end)?;
    if (prepared.before || cx.after_change_runs(false, false)?) && super::insdel::half_missing(cx)? {
        return super::insdel::install_half(cx, casify_restart, vec![case, fixnum(start), fixnum(end), Value::NIL]);
    }
    let saved = vec![fixnum(start), fixnum(end), case, flag(prepared.inhibited)];
    if !prepared.before { return casify_edit(cx, Value::NIL, &saved); }
    let buffer = cx.current_buffer()?;
    let hooks = prepared.modification_hooks.iter().rev().fold(Value::NIL, |tail, &h| cx.cons(h, tail));
    let args = vec![buffer, fixnum(start), fixnum(end), hooks, flag(prepared.undoable)];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: casify_edit, saved })
}

fn casify_restart(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    casify_region(cx, saved[0], saved[1], saved[2], saved[3])
}

fn casify_edit(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let overlays = super::insdel::found_overlays(cx, before);
    cx.change_deactivates_mark(saved[3] != Value::NIL)?;
    let (start, end) = (saved[0].integer_value().expect("start"), saved[1].integer_value().expect("end"));
    let Some((first, last, added)) = cx.casify_region_edit(start, end, region_case(saved[2]))? else {
        return Ok(Step::Done(Value::NIL));
    };
    if !cx.after_change_runs(overlays != Value::NIL, false)? { return Ok(Step::Done(Value::NIL)); }
    let buffer = cx.current_buffer()?;
    let args = vec![buffer, fixnum(first), fixnum(last), fixnum(last - first - added), Value::NIL, Value::NIL, overlays];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: casified, saved: vec![] })
}

fn casified(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}

defsubr! { name = "upcase-region", gnu = "casefiddle.c:609", effect = Global,
fn upcase_region(cx, beg: Value, end: Value; optional region_noncontiguous_p: Value) -> Step {
    casify_region(cx, fixnum(0), beg, end, region_noncontiguous_p)
}}

defsubr! { name = "downcase-region", gnu = "casefiddle.c:621", effect = Global,
fn downcase_region(cx, beg: Value, end: Value; optional region_noncontiguous_p: Value) -> Step {
    casify_region(cx, fixnum(1), beg, end, region_noncontiguous_p)
}}

defsubr! { name = "capitalize-region", gnu = "casefiddle.c:632", effect = Global,
fn capitalize_region(cx, beg: Value, end: Value; optional region_noncontiguous_p: Value) -> Step {
    casify_region(cx, fixnum(2), beg, end, region_noncontiguous_p)
}}

defsubr! { name = "upcase-initials-region", gnu = "casefiddle.c:646", effect = Global,
fn upcase_initials_region(cx, beg: Value, end: Value; optional region_noncontiguous_p: Value) -> Step {
    casify_region(cx, fixnum(3), beg, end, region_noncontiguous_p)
}}

#[cfg(test)]
mod tests {
    use crate::Vm;

    fn check(source: &str, expected: &str) {
        let mut vm = Vm::new();
        let value = vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.display(value).unwrap(), expected, "{source}");
    }

    #[test]
    fn native_case_callable_unicode() {
        check(
            "(list (fboundp 'upcase) (functionp #'downcase) (funcall #'upcase \"aßﬃ Ελληνικά русский ქართული 𐐨 中🙂\") (funcall #'downcase \"İ ΟΣ ΟΣΑ РУССКИЙ 𐐀 中🙂\"))",
            "(t t \"ASSFFI ΕΛΛΗΝΙΚΆ РУССКИЙ ᲥᲐᲠᲗᲣᲚᲘ 𐐀 中🙂\" \"i̇ ος οσα русский 𐐨 中🙂\")",
        );
        check(
            "(list (upcase 223) (downcase 304) (upcase 64259) (downcase 931) (upcase 66600) (upcase 20013) (upcase 55296) (downcase 4194303))",
            "(223 105 64259 963 66560 20013 55296 4194303)",
        );
    }

    #[test]
    fn native_case_context_and_no_normalization() {
        check(
            "(list (downcase \"AΣ́ AΣ́A AΣ' AΣ'A\") (upcase \"é é\") (downcase \"ẞ\"))",
            "(\"aσ́ aσ́a aς' aς'a\" \"É É\" \"ß\")",
        );
    }

    #[test]
    fn native_case_input_errors_and_modifiers() {
        check(
            "(list (upcase 134217825) (downcase 134217793) (upcase 4194304) (upcase 1152921504606846975))",
            "(134217793 134217825 4194304 1152921504606846975)",
        );
        check(
            "(list (condition-case e (upcase -1) (wrong-type-argument e)) (condition-case e (downcase nil) (wrong-type-argument e)) (condition-case e (upcase 1.5) (wrong-type-argument e)) (condition-case e (upcase []) (wrong-type-argument e)))",
            "((wrong-type-argument char-or-string-p -1) (wrong-type-argument char-or-string-p nil) (wrong-type-argument char-or-string-p 1.5) (wrong-type-argument char-or-string-p []))",
        );
    }

    #[test]
    fn native_case_buffer_overrides_and_identity() {
        // Special casing precedes the case table, as in GNU. `A', which the
        // table has no entry for, takes the Unicode default; GNU leaves it
        // as it is and gives "Aς".
        check(
            "(let ((d (make-char-table 'case-table)) (u (make-char-table 'case-table))) (aset d 931 120) (aset u 223 223) (aset u 233 81) (set-char-table-extra-slot d 0 u) (set-char-table-extra-slot d 1 (make-char-table 'case-table)) (set-case-table d) (list (upcase 223) (upcase \"ßéﬃ\") (downcase \"AΣ\") (with-temp-buffer (upcase \"ßé\"))))",
            "(223 \"SSQFFI\" \"aς\" \"SSÉ\")",
        );
        check(
            "(let ((d (copy-sequence (standard-case-table)))) (aset d 304 304) (set-case-table d) (list (downcase 304) (downcase \"İA\")))",
            "(304 \"i̇a\")",
        );
    }

    #[test]
    fn native_case_property_conventions() {
        check(
            "(let ((s (propertize (string-to-multibyte \"ßa\") 'p (list 4)))) (list (upcase s) (text-properties-at 0 s) (text-properties-at 0 (upcase s))))",
            "(\"SSA\" (p (4)) nil)",
        );
        check(
            "(let* ((p (list 4)) (s (propertize (unibyte-string 97 255) 'p p)) (u (upcase s))) (list (multibyte-string-p u) (aref u 0) (aref u 1) (eq p (get-text-property 1 'p u)) (eq s u) (aref s 0)))",
            "(nil 65 255 t nil 97)",
        );
    }

    #[test]
    fn native_case_raw_override_is_not_latin1() {
        check(
            "(let ((u (char-table-extra-slot (current-case-table) 0))) (aset u 233 201) (list (upcase 233) (append (upcase (unibyte-string 233)) nil) (with-temp-buffer (set-buffer-multibyte nil) (upcase 233))))",
            "(201 (233) 233)",
        );
        check(
            "(let ((u (char-table-extra-slot (current-case-table) 0))) (aset u 4194281 4194249) (list (append (upcase (unibyte-string 233)) nil) (with-temp-buffer (set-buffer-multibyte nil) (upcase 233)) (upcase 233)))",
            "((201) 201 201)",
        );
    }

    #[test]
    fn native_case_parent_default_and_empty() {
        check(
            "(let* ((p (make-char-table 'case-table)) (d (make-char-table 'case-table))) (aset p 931 931) (set-char-table-parent d p) (set-case-table d) (list (downcase 931) (downcase \"AΣ\") (downcase \"İ\")))",
            "(931 \"aΣ\" \"i̇\")",
        );
        check(
            "(let ((s (string))) (list (eq s (upcase s)) (condition-case e (upcase) (wrong-number-of-arguments (car e))) (condition-case e (downcase 1 2) (wrong-number-of-arguments (car e)))))",
            "(t wrong-number-of-arguments wrong-number-of-arguments)",
        );
    }

    #[test]
    fn native_case_survives_moving_gc() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq waste (make-string 1000 97)) (setq waste nil) (setq saved-prop (list 8)) (setq saved-case-string (propertize (unibyte-string 97 233) 'p saved-prop)) (let ((u (char-table-extra-slot (current-case-table) 0))) (aset u 223 223)) (setq saved-case-result (upcase saved-case-string))").unwrap();
        let before = vm.standard_case_tables;
        vm.collect(&mut []);
        assert_ne!(
            before, vm.standard_case_tables,
            "the collection must actually move the case roots"
        );
        let got = vm.eval_source_raw("(list (upcase \"ßé\") (append (upcase saved-case-string) nil) (eq saved-prop (get-text-property 0 'p saved-case-result)) (eq saved-prop (get-text-property 1 'p (upcase saved-case-string))))").unwrap();
        assert_eq!(vm.display(got).unwrap(), "(\"SSÉ\" (65 233) t t)");
    }

    #[test]
    fn native_case_raw_bytes_and_extended_codes() {
        check(
            "(let ((u (upcase (unibyte-string 97 223 233 255)))) (list (multibyte-string-p u) (append u nil)))",
            "(nil (65 223 233 255))",
        );
        check(
            "(let ((s (string 65 4194176 931 55296 233 2097152))) (append (downcase s) nil))",
            "(97 4194176 963 55296 233 2097152)",
        );
        check(
            "(with-temp-buffer (set-buffer-multibyte nil) (list (upcase 233) (downcase 201) (upcase \"éß\")))",
            "(233 201 \"ÉSS\")",
        );
        check(
            "(let ((u (char-table-extra-slot (current-case-table) 0))) (aset u 97 1000) (append (upcase (unibyte-string 97)) nil))",
            "(65)",
        );
        check(
            "(let ((u (char-table-extra-slot (current-case-table) 0))) (aset u 97 'bad) (upcase \"a\"))",
            "\"a\"",
        );
        check(
            "(let ((u (char-table-extra-slot (current-case-table) 0))) (aset u 97 4194304) (condition-case e (upcase \"a\") (wrong-type-argument e)))",
            "(wrong-type-argument characterp 4194304)",
        );
    }
}
