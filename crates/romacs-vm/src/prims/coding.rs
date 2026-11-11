//! Registry calls that may realize coding-system-define-form are T2 calls.
//! The evaluator only runs at Step::Call, never inside a Leaf.
use super::*;

fn list(cx: &mut Leaf<'_>, items: &[Value]) -> Value {
    cx.coding_list(items)
}
pub(super) fn realize(cx: &mut Leaf<'_>, name: Value, saved: Vec<Value>, then: crate::prim::Then) -> Next {
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

fn convert_string(cx: &mut Leaf<'_>, encode: bool, string: Value, coding: Value, nocopy: Value, buffer: Value) -> Next {
    cx.string_view(string)?;
    let saved = vec![if encode { Value::TRUE } else { Value::NIL }, string, coding, nocopy, buffer];
    if coding != Value::NIL && cx.kind(coding) == Kind::Symbol && cx.coding_spec(coding)? == Value::NIL {
        return realize(cx, coding, saved, converted_string);
    }
    converted_string(cx, Value::NIL, &saved)
}

fn converted_string(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let buffer = saved[4];
    let result = cx.coding_convert_string(saved[1], saved[2], saved[3], buffer, saved[0] != Value::NIL)?;
    if buffer == Value::NIL || buffer == Value::TRUE {
        return Ok(Step::Done(result));
    }
    // code_convert_string into BUFFER: the text at its point; the length.
    Ok(Step::Call { f: cx.intern("romacs--coding-insert"), args: vec![buffer, result], then: done, saved: vec![] })
}

fn done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }

defsubr! { name = "detect-coding-string", gnu = "coding.c:9040", effect = Reads,
fn detect_coding_string(cx, string: Value; optional highest: Value) { cx.coding_detect_string(string, highest) }}

defsubr! { name = "encode-coding-string", gnu = "coding.c:10509", effect = Global,
fn encode_coding_string(cx, string: Value, coding: Value; optional nocopy: Value, buffer: Value) -> Step {
    convert_string(cx, true, string, coding, nocopy, buffer)
}}

defsubr! { name = "decode-coding-string", gnu = "coding.c:10487", effect = Global,
fn decode_coding_string(cx, string: Value, coding: Value; optional nocopy: Value, buffer: Value) -> Step {
    convert_string(cx, false, string, coding, nocopy, buffer)
}}

// Registers of a region conversion between its steps:
//   [ENCODE, START, END, CODING, DESTINATION]
fn convert_region(cx: &mut Leaf<'_>, encode: bool, start: Value, end: Value, coding: Value, destination: Value) -> Next {
    let saved = vec![if encode { Value::TRUE } else { Value::NIL }, start, end, coding, destination];
    if coding != Value::NIL && cx.kind(coding) == Kind::Symbol && cx.coding_spec(coding)? == Value::NIL {
        return realize(cx, coding, saved, converted_region);
    }
    converted_region(cx, Value::NIL, &saved)
}

/// coding.c `code_convert_region`: START..END of the current buffer through
/// CODING. DESTINATION t returns the text and a buffer receives it at its
/// point. Nil replaces the region as one change (`decode_coding_object`'s
/// `del_range` and insertion: one before-change over the old text and one
/// after-change over the new). Markers at its start stay before the new
/// text and those at its end after it; one inside goes to the start (GNU
/// puts one that advances on insertion after the new text). Point keeps its place
/// outside the old text and goes to its start from inside it. The value is
/// then the new text's length. An empty region that converts to nothing is
/// no change.
fn converted_region(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (encode, coding, destination) = (saved[0] != Value::NIL, saved[3], saved[4]);
    if coding != Value::NIL { cx.coding_check(coding)?; }
    let own = destination == Value::NIL || destination == cx.current_buffer()?;
    if !own && destination != Value::TRUE && !matches!(cx.kind(destination), Kind::Buffer | Kind::ByteBuffer) {
        return Err(Fault::Wrong("bufferp", destination));
    }
    let (from, to) = cx.validate_region(saved[1], saved[2])?;
    // A read-only region refuses before anything is converted or recorded.
    if own { cx.prepare_change(from, to)?; }
    let text = cx.region_plain_text(from, to)?;
    let coding = if coding == Value::NIL { cx.intern("no-conversion") } else { coding };
    let result = cx.coding_convert_string(text, coding, Value::TRUE, Value::NIL, encode)?;
    if destination == Value::TRUE { return Ok(Step::Done(result)); }
    if !own {
        return Ok(Step::Call { f: cx.intern("romacs--coding-insert"), args: vec![destination, result], then: done, saved: vec![] });
    }
    let length = i64::try_from(cx.string_chars(result).expect("converted text")).expect("string length");
    if from == to && length == 0 { return Ok(Step::Done(Value::integer(0).expect("fixnum"))); }
    let (point, _, _) = cx.point_extent()?;
    let point = if point < from { point } else if point < to { from } else { point + length - (to - from) };
    let deactivate = cx.intern("deactivate-mark");
    let kept = cx.variable_value(deactivate)?.unwrap_or(Value::NIL);
    let position = |n: i64| Value::integer(n).expect("buffer position");
    Ok(Step::Call {
        f: cx.intern("romacs--replace-range"),
        args: vec![position(from), position(to), result, Value::NIL],
        then: replaced_region,
        saved: vec![position(point), kept, position(length)],
    })
}

fn replaced_region(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (_, begv, zv) = cx.point_extent()?;
    cx.goto_char_position(saved[0].integer_value().expect("point").clamp(begv, zv))?;
    let deactivate = cx.intern("deactivate-mark");
    cx.store_variable(deactivate, saved[1])?;
    Ok(Step::Done(saved[2]))
}

defsubr! { name = "decode-coding-region", gnu = "coding.c:9501", effect = Global,
fn decode_coding_region(cx, start: Value, end: Value, coding: Value; optional destination: Value) -> Step {
    convert_region(cx, false, start, end, coding, destination)
}}

defsubr! { name = "encode-coding-region", gnu = "coding.c:9533", effect = Global,
fn encode_coding_region(cx, start: Value, end: Value, coding: Value; optional destination: Value) -> Step {
    convert_region(cx, true, start, end, coding, destination)
}}

defsubr! { name = "set-safe-terminal-coding-system-internal", gnu = "coding.c:10704", effect = Global,
fn set_safe_terminal_coding_system_internal(cx, coding: Value) {
    // Romacs writes no terminal output; the checked coding is kept for
    // whatever later encodes for a terminal (safe_terminal_coding).
    if cx.kind(coding) != leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", coding)); }
    let coding = cx.coding_check(coding)?;
    let slot = cx.intern("#:vm2-safe-terminal-coding");
    cx.store_variable(slot, coding)?;
    Ok(Value::NIL)
}}

defsubr! { name = "set-coding-system-priority", gnu = "coding.c:10875", effect = Global,
fn set_coding_system_priority(cx; rest systems) { cx.set_coding_system_priority(systems) }}

defsubr! { name = "romacs--set-coding-category-order", gnu = "coding.c:10923", effect = Global,
fn set_coding_category_order(cx; rest categories) { cx.set_coding_category_order(categories) }}

defsubr! { name = "coding-system-priority-list", gnu = "coding.c:10933", effect = Reads,
fn coding_system_priority_list(cx; optional highestp: Value) { cx.coding_system_priority_list(highestp) }}

defsubr! { name = "set-terminal-coding-system-internal", gnu = "coding.c:10682", effect = Global,
fn set_terminal_coding_system_internal(cx, coding: Value; optional terminal: Value) {
    let (terminal, _, _) = cx.windows().terminal_codings(terminal)?;
    if cx.kind(coding) != leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", coding)); }
    // setup_coding_system treats nil as undecided.
    let coding = if cx.coding_check(coding)? == Value::NIL { cx.intern("undecided") } else { coding };
    cx.windows().set_terminal_coding(terminal, false, coding)?;
    Ok(Value::NIL)
}}

defsubr! { name = "terminal-coding-system", gnu = "coding.c:10720", effect = Reads,
fn terminal_coding_system(cx; optional terminal: Value) {
    let (_, coding, _) = cx.windows().terminal_codings(terminal)?;
    // For backward compatibility, undecided reads as nil.
    Ok(if coding == cx.intern("undecided") { Value::NIL } else { coding })
}}

defsubr! { name = "set-keyboard-coding-system-internal", gnu = "coding.c:10735", effect = Global,
fn set_keyboard_coding_system_internal(cx, coding: Value; optional terminal: Value) {
    let (terminal, _, _) = cx.windows().terminal_codings(terminal)?;
    if cx.kind(coding) != leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", coding)); }
    let coding = if coding == Value::NIL { cx.intern("no-conversion") } else { cx.coding_check(coding)?; coding };
    cx.windows().set_terminal_coding(terminal, true, coding)?;
    Ok(Value::NIL)
}}

defsubr! { name = "keyboard-coding-system", gnu = "coding.c:10753", effect = Reads,
fn keyboard_coding_system(cx; optional terminal: Value) {
    let (_, _, coding) = cx.windows().terminal_codings(terminal)?;
    Ok(coding)
}}

defsubr! { name = "find-coding-systems-region-internal", gnu = "coding.c:9087", effect = Reads,
fn find_coding_systems_region_internal(cx, start: Value, end: Value; optional exclude: Value) {
    cx.find_coding_systems_region(start, end, exclude)
}}

// Ffind_operation_coding_system: the coding pair the alist for OPERATION
// gives its target, calling a function entry with the whole argument list.
defsubr! { name = "find-operation-coding-system", gnu = "coding.c:10763", effect = Global,
fn find_operation_coding_system(cx, operation: Value; rest arguments) -> Step {
    let args: Vec<Value> = std::iter::once(operation).chain(arguments.iter().copied()).collect();
    match cx.find_operation_coding(&args)? {
        Ok(value) => Ok(Step::Done(value)),
        Err(function) => {
            let list = cx.coding_list(&args);
            Ok(Step::Call { f: function, args: vec![list], then: operation_coding_found, saved: vec![] })
        }
    }
}}
fn operation_coding_found(cx: &mut Leaf<'_>, val: Value, _: &[Value]) -> Next {
    Ok(Step::Done(cx.operation_coding_pair(val)?))
}
