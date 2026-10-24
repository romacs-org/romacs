//! GNU xfaces.c's live face registry. The font-realizing setter is not
//! declared here: the immutable display theme is not a Lisp face registry.
use super::*;
use crate::prim::leaf::Kind;
use romacs_face::registry::{new_lface, LFACE_ATTRIBUTES, LFACE_VECTOR_SIZE};

fn resolve(cx: &mut Leaf<'_>, mut face: Value, signal_cycle: bool) -> Out {
    if matches!(cx.kind(face), Kind::String | Kind::Bytes) {
        let table = cx.resolve_obarray(Value::NIL)?;
        face = cx.intern_in(table, face)?;
    }
    let original = face;
    let property = cx.intern("face-alias");
    let mut seen = std::collections::HashSet::new();
    while face != Value::NIL && cx.kind(face) == Kind::Symbol {
        if !seen.insert(face.0) {
            return if signal_cycle { Err(Fault::Circular(original)) }
                else { Ok(cx.intern("default")) };
        }
        let next = cx.face_property(face, property)?;
        if next == Value::NIL || cx.kind(next) != Kind::Symbol { break; }
        face = next;
    }
    Ok(face)
}

fn lookup(cx: &mut Leaf<'_>, face: Value, frame: Value) -> Out {
    let face = resolve(cx, face, false)?;
    cx.face_lookup(face, frame)
}

fn proper_list(cx: &Leaf<'_>, mut value: Value) -> bool {
    let mut seen = std::collections::HashSet::new();
    while value != Value::NIL {
        if !seen.insert(value.0) { return false; }
        let Some((_, next)) = cx.pair(value) else { return false; };
        value = next;
    }
    true
}

fn signal(cx: &mut Leaf<'_>, message: &str, value: Value) -> Fault {
    let message = cx.string(message);
    let tail = if proper_list(cx, value) { value }
        else { cx.cons(value, Value::NIL) };
    let data = cx.cons(message, tail);
    let error = cx.intern("error");
    Fault::Signal(error, data)
}

fn new_vector(cx: &mut Leaf<'_>) -> Value {
    let marker = cx.intern("face");
    let unspecified = cx.intern("unspecified");
    cx.vector_from(new_lface(marker, unspecified))
}

fn reset(cx: &mut Leaf<'_>, vector: Value) -> Out {
    let unspecified = cx.intern("unspecified");
    for index in 1..LFACE_VECTOR_SIZE {
        cx.vector_set(vector, Value::integer(index as i64).unwrap(), unspecified)?;
    }
    Ok(vector)
}

defsubr! { name = "internal-lisp-face-p", gnu = "xfaces.c:3120", effect = Reads,
fn internal_lisp_face_p(cx, face: Value; optional frame: Value) {
    let face = resolve(cx, face, true)?;
    let frame = cx.face_frame(frame, false)?;
    lookup(cx, face, frame)
}}

defsubr! { name = "internal-make-lisp-face", gnu = "xfaces.c:3029", effect = Global,
fn internal_make_lisp_face(cx, face: Value; optional frame: Value) {
    if cx.kind(face) != Kind::Symbol { return Err(Fault::Wrong("symbolp", face)); }
    // GNU resolves before validating FRAME, including alias-cycle errors.
    let global = lookup(cx, face, Value::NIL)?;
    let frame = cx.face_frame(frame, false)?;
    let local = if frame == Value::NIL { Value::NIL } else { lookup(cx, face, frame)? };
    let global = if global == Value::NIL {
        let vector = new_vector(cx);
        cx.face_insert(face, Value::NIL, vector)?
    } else if frame == Value::NIL { reset(cx, global)? } else { global };
    if frame == Value::NIL { return Ok(global); }
    if local != Value::NIL { return reset(cx, local); }
    let vector = new_vector(cx);
    cx.face_insert(face, frame, vector)
}}

defsubr! { name = "internal-get-lisp-face-attribute", gnu = "xfaces.c:4230", effect = Reads,
fn internal_get_lisp_face_attribute(cx, face: Value, attribute: Value; optional frame: Value) {
    let frame = if frame == Value::TRUE { Value::NIL } else { cx.face_frame(frame, true)? };
    let resolved = resolve(cx, face, true)?;
    let vector = cx.face_lookup(resolved, frame)?;
    if vector == Value::NIL { return Err(signal(cx, "Invalid face", resolved)); }
    if cx.kind(face) != Kind::Symbol { return Err(Fault::Wrong("symbolp", face)); }
    if cx.kind(attribute) != Kind::Symbol { return Err(Fault::Wrong("symbolp", attribute)); }
    for (index, name) in LFACE_ATTRIBUTES.iter().enumerate() {
        if attribute == cx.intern(name) {
            let value = cx.vector_get(vector, Value::integer((index + 1) as i64).unwrap())?;
            return Ok(if value == cx.intern(":ignore-defface") { cx.intern("unspecified") } else { value });
        }
    }
    Err(signal(cx, "Invalid face attribute name", attribute))
}}

defsubr! { name = "face-attribute-relative-p", gnu = "xfaces.c:4193", effect = Pure,
fn face_attribute_relative_p(cx, attribute: Value, value: Value) {
    Ok(if value == cx.intern("unspecified") || value == cx.intern(":ignore-defface")
        || (attribute == cx.intern(":height") && value.integer_value().is_none()) {
        Value::TRUE
    } else { Value::NIL })
}}

defsubr! { name = "internal-lisp-face-attribute-values", gnu = "xfaces.c:4295", effect = Pure,
fn internal_lisp_face_attribute_values(cx, attribute: Value) {
    if cx.kind(attribute) != Kind::Symbol { return Err(Fault::Wrong("symbolp", attribute)); }
    for name in [":underline", ":overline", ":strike-through", ":inverse-video", ":extend"] {
        if attribute == cx.intern(name) {
            let tail = cx.cons(Value::NIL, Value::NIL);
            return Ok(cx.cons(Value::TRUE, tail));
        }
    }
    Ok(Value::NIL)
}}
