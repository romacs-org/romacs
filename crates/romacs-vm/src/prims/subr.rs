use super::*;

defsubr! { name = "subr-arity", gnu = "data.c:1038", effect = Pure,
fn subr_arity(cx, subr: Value) {
    let (minimum, maximum) = crate::prim::arity(subr).ok_or(Fault::Wrong("subrp", subr))?;
    // A special form's arity is (MIN . unevalled).
    let special = crate::prim::name(subr).and_then(super::special_forms::special_form_min);
    let minimum = Value::integer(special.unwrap_or(minimum) as i64).unwrap();
    let maximum = match maximum {
        _ if special.is_some() => cx.intern("unevalled"),
        Some(maximum) => Value::integer(maximum as i64).unwrap(),
        None => cx.intern("many"),
    };
    Ok(cx.cons(minimum, maximum))
}}

defsubr! { name = "subr-name", gnu = "data.c:1056", effect = Pure,
fn subr_name(cx, subr: Value) {
    let name = crate::prim::name(subr).ok_or(Fault::Wrong("subrp", subr))?;
    Ok(cx.byte_string(name.as_bytes()))
}}

// Fmarker_last_position: where MARKER is, or was when it left its buffer.
defsubr! { name = "marker-last-position", gnu = "marker.c:458", effect = Reads,
fn marker_last_position(cx, marker: Value) { cx.marker_last_position(marker) }}

// Fgap_size: a buffer's text is a rope, with no gap.
defsubr! { name = "gap-size", gnu = "editfns.c:925", effect = Pure,
fn gap_size(cx) { let _ = cx; Ok(Value::integer(0).unwrap()) }}

// Fsubr_type: a native-compiled subr's declared type; no subr here has one.
defsubr! { name = "subr-type", gnu = "data.c:1089", effect = Pure,
fn subr_type(cx, subr: Value) {
    let _ = cx;
    crate::prim::arity(subr).ok_or(Fault::Wrong("subrp", subr))?;
    Ok(Value::NIL)
}}

// Fsubr_native_lambda_list: t for a subr that is no dynamic native function.
defsubr! { name = "subr-native-lambda-list", gnu = "data.c:1074", effect = Pure,
fn subr_native_lambda_list(cx, subr: Value) {
    let _ = cx;
    crate::prim::arity(subr).ok_or(Fault::Wrong("subrp", subr))?;
    Ok(Value::TRUE)
}}

// Where Rust defines OBJECT, as help-fns.el's `help-C-file-name' finds a
// DEFUN or DEFVAR in GNU's DOC file: for KIND `defvar', OBJECT a symbol
// Rust forwards (its C_VARIABLES entry); else OBJECT a primitive (its
// defsubr, or a legacy one's row in lib.rs's PRIMITIVES).  (FILE LINE TEXT):
// FILE relative to the source tree, LINE the registration's line or nil,
// TEXT the registration's literal text to find there.  nil when Rust
// defines no such thing.
defsubr! { name = "romacs--rust-definition", gnu = "romacs:help", effect = Reads,
fn romacs_rust_definition(cx, object: Value, kind: Value) {
    let defvar = cx.intern("defvar");
    let found: Option<(&str, Option<u32>, String)> = if kind == defvar {
        let name = cx.symbol_name(object)?;
        crate::prim::VARIABLE_SITES.iter().find(|v| v.0 == name).map(|&(_, file, line, text)| (file, Some(line), text.to_owned()))
    } else {
        match object.index_if(6).and_then(crate::prim::descriptor) {
            // A defsubr's file!() is the path its module was included by:
            // from the crate on.
            Some(crate::prim::Descriptor::Generated(d)) => Some((d.file.rfind("crates/romacs-vm/").map_or(d.file, |i| &d.file[i..]), Some(d.line), format!("name = \"{}\"", d.name))),
            Some(crate::prim::Descriptor::Legacy(name, _)) => Some((crate::PRIMITIVES_FILE,
                crate::prim::LEGACY_SITES.iter().find(|s| s.0 == name).map(|s| s.1), format!("(\"{name}\", Primitive::"))),
            None => None,
        }
    };
    let Some((file, line, text)) = found else { return Ok(Value::NIL) };
    let line = match line { Some(n) => Value::integer(i64::from(n)).unwrap(), None => Value::NIL };
    let (file, text) = (cx.make_string(file.as_bytes().to_vec(), true), cx.make_string(text.as_bytes().to_vec(), true));
    let tail = cx.cons(text, Value::NIL);
    let tail = cx.cons(line, tail);
    Ok(cx.cons(file, tail))
}}
