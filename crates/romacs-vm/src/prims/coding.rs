//! Registry calls that may realize coding-system-define-form are T2 calls.
//! The evaluator only runs at Step::Call, never inside a Leaf.
use super::*;

fn list(cx: &mut Leaf<'_>, items: &[Value]) -> Value {
    cx.coding_list(items)
}
fn realize(cx: &mut Leaf<'_>, name: Value, saved: Vec<Value>, then: crate::prim::Then) -> Next {
    let form = cx.coding_define_form(name, true)?;
    if form == Value::NIL {
        return then(cx, Value::NIL, &saved);
    }
    // GNU safe_eval calls (eval FORM t), binds inhibit-redisplay and catches
    // every signaled condition (including quit), but not a nonlocal throw.
    let condition_case = cx.intern("condition-case");
    let handler = list(cx, &[Value::TRUE, Value::NIL]);
    let protected = list(cx, &[condition_case, Value::NIL, form, handler]);
    let inhibit = cx.intern("inhibit-redisplay");
    let binding = list(cx, &[inhibit, Value::TRUE]);
    let bindings = list(cx, &[binding]);
    let let_symbol = cx.intern("let");
    let protected = list(cx, &[let_symbol, bindings, protected]);
    Ok(Step::Call {
        f: cx.intern("eval"),
        args: vec![protected, Value::TRUE],
        then,
        saved,
    })
}
fn operation(cx: &mut Leaf<'_>, code: i64, args: &[Value]) -> Next {
    cx.coding_init()?;
    let mut saved = vec![Value::integer(code).unwrap()];
    saved.extend_from_slice(args);
    let mut name = args[0];
    if (0..=2).contains(&code) && name == Value::NIL {
        name = cx.intern("no-conversion");
    }
    if code == 5 {
        name = args[1];
    }
    if code != 4 && cx.coding_spec(name)? != Value::NIL {
        return resumed(cx, Value::NIL, &saved);
    }
    realize(cx, name, saved, resumed)
}
fn resumed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let code = saved[0].integer_value().unwrap();
    let args = &saved[1..];
    let value = match code {
        0..=2 => cx.coding_access(args[0], code)?,
        4 => cx.coding_check(args[0])?,
        5 => cx.coding_alias(args[0], args[1])?,
        6 => cx.coding_put(args[0], args[1], args[2])?,
        _ => unreachable!(),
    };
    Ok(Step::Done(value))
}

defsubr! { name = "coding-system-p", gnu = "coding.c:8594", effect = Global,
fn coding_system_p(cx, object: Value) { cx.coding_p(object) }}
defsubr! { name = "check-coding-system", gnu = "coding.c:8647", effect = Global,
fn check_coding_system(cx, coding: Value) -> Step { operation(cx,4,&[coding]) }}
defsubr! { name = "coding-system-base", gnu = "coding.c:11614", effect = Global,
fn coding_system_base(cx, coding: Value) -> Step { operation(cx,0,&[coding]) }}
defsubr! { name = "coding-system-plist", gnu = "coding.c:11629", effect = Global,
fn coding_system_plist(cx, coding: Value) -> Step { operation(cx,1,&[coding]) }}
defsubr! { name = "coding-system-aliases", gnu = "coding.c:11644", effect = Global,
fn coding_system_aliases(cx, coding: Value) -> Step { operation(cx,2,&[coding]) }}
defsubr! { name = "coding-system-eol-type", gnu = "coding.c:11657", effect = Global,
fn coding_system_eol_type(cx, coding: Value) { cx.coding_access(coding,3) }}
defsubr! { name = "coding-system-put", gnu = "coding.c:11505", effect = Global,
fn coding_system_put(cx, coding: Value, property: Value, value: Value) -> Step { operation(cx,6,&[coding,property,value]) }}
defsubr! { name = "define-coding-system-alias", gnu = "coding.c:11575", effect = Global,
fn define_coding_system_alias(cx, alias: Value, coding: Value) -> Step {
    // GNU validates ALIAS before forcing the target.
    if cx.kind(alias)!=Kind::Symbol { return Err(Fault::Wrong("symbolp",alias)); }
    operation(cx,5,&[alias,coding])
}}
defsubr! { name = "define-coding-system-internal", gnu = "coding.c:10981", effect = Global,
fn define_coding_system_internal(cx, name: Value, mnemonic: Value, coding_type: Value,
    charset_list: Value, ascii_compatible: Value, decode_translation: Value,
    encode_translation: Value, post_read: Value, pre_write: Value, default_char: Value,
    for_unibyte: Value, plist: Value, eol_type: Value; rest extra) -> Step {
    let mut args = vec![name, mnemonic, coding_type, charset_list, ascii_compatible,
        decode_translation, encode_translation, post_read, pre_write, default_char,
        for_unibyte, plist, eol_type];
    args.extend_from_slice(extra);
    define_next(cx, args, 0)
}}
fn define_next(cx: &mut Leaf<'_>, args: Vec<Value>, next: usize) -> Next {
    // BOM pairs are the only definition fields that can force lazy codings.
    // Validate the ordinary fields in the leaf in GNU order. For an actual
    // pair, realize only missing members without consulting mutable function cells.
    if args.len() >= 14 && (args[2] == cx.intern("utf-8") || args[2] == cx.intern("utf-16")) {
        if let Some((a, b)) = cx.pair(args[13]) {
            if next == 0 {
                cx.coding_validate_prefix(&args)?;
            }
            for index in next..2 {
                let name = if index == 0 { a } else { b };
                if cx.coding_spec(name)? != Value::NIL {
                    continue;
                }
                if name == Value::NIL {
                    return Err(Fault::Wrong("coding-system-p", name));
                }
                let mut saved = vec![Value::integer((index + 1) as i64).unwrap(), name];
                saved.extend_from_slice(&args);
                return realize(cx, name, saved, defined_bom);
            }
        }
    }
    cx.coding_define(&args).map(Step::Done)
}
fn defined_bom(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let next = saved[0].integer_value().unwrap() as usize;
    cx.coding_check(saved[1])?;
    define_next(cx, saved[2..].to_vec(), next)
}

#[cfg(test)]
mod tests {
    fn check(source: &str, expected: &str) {
        for floor in [1, 4096] {
            let mut vm = crate::Vm::new();
            vm.set_collection_floor(floor);
            let value = vm.eval_source(source).unwrap();
            assert_eq!(value.display().unwrap(), expected, "GC floor {floor}");
        }
    }

    #[test]
    fn registry_symbol_identity() {
        check(
            r#"(list
                (condition-case e
                    (define-coding-system-internal 'cod-type 84 (make-symbol "raw-text")
                        nil t nil nil nil nil 0 nil nil 'unix)
                  (error (car e)))
                (progn
                  (define-coding-system-internal 'cod-charset 84 'raw-text
                    (make-symbol "iso-2022") t nil nil nil nil 0 nil nil 'unix)
                  (coding-system-p 'cod-charset))
                (condition-case e
                    (define-coding-system-internal 'cod-eol 84 'raw-text
                        nil t nil nil nil nil 0 nil nil (make-symbol "unix"))
                  (error (car e)))
                (let ((key (make-symbol ":mnemonic")))
                  (define-coding-system-internal 'cod-prop 84 'raw-text
                      nil t nil nil nil nil 0 nil nil 'unix)
                  (coding-system-put 'cod-prop key t)))"#,
            "(error t error t)",
        );
    }

    #[test]
    fn ccl_eof_may_equal_program_length() {
        check(
            "(progn (define-coding-system-internal 'cod-ccl 84 'ccl nil t nil nil nil nil 0 nil nil 'unix [0 3 0] [0 3 0] nil) (coding-system-p 'cod-ccl))",
            "t",
        );
    }

    #[test]
    fn bom_existing_members_are_not_realized_or_dispatched() {
        check(
            "(progn
               (setq cod-forced nil)
               (put 'no-conversion 'coding-system-define-form '(setq cod-forced t))
               (fset 'check-coding-system (lambda (&rest args) (error \"replaced\")))
               (define-coding-system-internal 'cod-bom 84 'utf-8 nil t nil nil nil nil 0 nil nil 'unix '(no-conversion . no-conversion))
               (list (coding-system-p 'cod-bom) cod-forced (not (null (get 'no-conversion 'coding-system-define-form)))))",
            "(t nil t)",
        );
    }

    #[test]
    fn bom_lazy_realization_uses_special_inhibit_redisplay() {
        check(
            "(progn
               (setq inhibit-redisplay nil cod-inhibited nil)
               (fset 'cod-observe (lambda () (setq cod-inhibited inhibit-redisplay)))
               (put 'cod-lazy 'coding-system-define-form
                 '(progn (cod-observe)
                    (define-coding-system-internal 'cod-lazy 84 'raw-text nil t nil nil nil nil 0 nil nil 'unix)))
               (fset 'check-coding-system (lambda (&rest args) (error \"replaced\")))
               (define-coding-system-internal 'cod-lazy-bom 84 'utf-8 nil t nil nil nil nil 0 nil nil 'unix '(cod-lazy . no-conversion))
               (list (coding-system-p 'cod-lazy-bom) cod-inhibited inhibit-redisplay))",
            "(t t nil)",
        );
    }
}
