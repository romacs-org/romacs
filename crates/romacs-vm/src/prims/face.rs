//! GNU xfaces.c's live face registry. The Lisp lface vectors are
//! authoritative; attribute changes on a frame are mirrored into the display
//! theme, which realizes them.
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
    // Faces seen along an alias chain, kept only once there is one.
    let mut seen = std::collections::HashSet::new();
    while face != Value::NIL && cx.kind(face) == Kind::Symbol {
        let next = cx.face_property(face, property)?;
        if next == Value::NIL || cx.kind(next) != Kind::Symbol { break; }
        if !seen.insert(face.0) {
            return if signal_cycle { Err(Fault::Circular(original)) }
                else { Ok(cx.intern("default")) };
        }
        face = next;
    }
    Ok(face)
}

/// Whether the display realizes faces from FRAME's: every frame but a child
/// frame, which the display draws with its root frame's faces
/// (DIVERGENCES.md VM2-CHILD-FRAME). A theme recomputes each face on every
/// frame; mirroring only these keeps the display's work one frame's.
pub(super) fn displayed_faces(cx: &mut Leaf<'_>, frame: Value) -> std::result::Result<bool, Fault> {
    Ok(!cx.windows().is_child_frame(frame)?)
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
fn internal_make_lisp_face(cx, face: Value; optional frame: Value) { make_lisp_face(cx, face, frame) }}

fn make_lisp_face(cx: &mut Leaf<'_>, face: Value, frame: Value) -> Out {
    if cx.kind(face) != Kind::Symbol { return Err(Fault::Wrong("symbolp", face)); }
    // GNU resolves before validating FRAME, including alias-cycle errors.
    let global = lookup(cx, face, Value::NIL)?;
    let frame = cx.face_frame(frame, false)?;
    // A frame showing another's faces keeps showing them while making FACE
    // on it changes nothing it shows: FACE is new there, or still empty.
    if frame != Value::NIL && cx.frame_shares_faces(frame)? {
        let owner = cx.face_owner(frame)?;
        let shown = cx.face_lookup(face, owner)?;
        if shown == Value::NIL || empty_lface(cx, shown)? { return make_lisp_face(cx, face, owner); }
        cx.face_own(frame)?;
    }
    let local = if frame == Value::NIL { Value::NIL } else { lookup(cx, face, frame)? };
    let global = if global == Value::NIL {
        let vector = new_vector(cx);
        cx.face_insert(face, Value::NIL, vector)?
    } else if frame == Value::NIL { reset(cx, global)? } else { global };
    if frame == Value::NIL { return Ok(global); }
    if local != Value::NIL { return reset(cx, local); }
    let vector = new_vector(cx);
    cx.face_insert(face, frame, vector)
}

defsubr! { name = "internal-copy-lisp-face", gnu = "xfaces.c:3145", effect = Global,
fn internal_copy_lisp_face(cx, from: Value, to: Value, frame: Value, new_frame: Value) {
    if cx.kind(from) != Kind::Symbol { return Err(Fault::Wrong("symbolp", from)); }
    if cx.kind(to) != Kind::Symbol { return Err(Fault::Wrong("symbolp", to)); }
    let live = |cx: &mut Leaf<'_>, frame: Value| if frame == Value::NIL { Err(Fault::Wrong("frame-live-p", frame)) } else { cx.face_frame(frame, false) };
    // lface_from_face_name (F, FROM, true): FROM's lface or "Invalid face".
    let (source, target) = if frame == Value::TRUE { (Value::NIL, Value::NIL) } else {
        let new_frame = if new_frame == Value::NIL { frame } else { new_frame };
        (live(cx, frame)?, live(cx, new_frame)?)
    };
    let resolved = resolve(cx, from, true)?;
    let lface = cx.face_lookup(resolved, source)?;
    if lface == Value::NIL { return Err(signal(cx, "Invalid face", from)); }
    // A frame showing another's faces gets its own only when the copy
    // changes what it shows.
    if target != Value::NIL && cx.frame_shares_faces(target)? {
        let shown = cx.face_lookup(to, target)?;
        if shown != Value::NIL && same_lface(cx, shown, lface)? { return Ok(to); }
        cx.face_own(target)?;
    }
    let copy = make_lisp_face(cx, to, target)?;
    for index in 0..LFACE_VECTOR_SIZE {
        let index = Value::integer(index as i64).unwrap();
        let value = cx.vector_get(lface, index)?;
        cx.vector_set(copy, index, value)?;
    }
    // A frame's realized faces follow its named faces (face_change).
    let no_inherit = cx.intern("face-no-inherit");
    if target != Value::NIL && displayed_faces(cx, target)? && cx.face_property(to, no_inherit)? == Value::NIL {
        let name = cx.symbol_name(to)?;
        for (index, keyword) in LFACE_ATTRIBUTES.iter().enumerate() {
            let value = cx.vector_get(copy, Value::integer((index + 1) as i64).unwrap())?;
            let mirrored = theme_value(cx, keyword, value, target)?;
            cx.theme_face_attribute(&name, keyword, mirrored);
        }
    }
    Ok(to)
}}

defsubr! { name = "internal-merge-in-global-face", gnu = "xfaces.c:4316", effect = Global,
fn internal_merge_in_global_face(cx, face: Value, frame: Value) {
    // Every attribute the face's new-frame defaults specify overrides
    // FRAME's, which face-set-after-frame-default has just computed from
    // the specs: "global before local", each slot copied as GNU copies it
    // (the frame's own values are not validated again, so a :font or
    // :fontset of `reset' arrives too).  A default of :ignore-defface
    // unspecifies FRAME's.  What changes is mirrored to the display.
    let frame = cx.face_frame(frame, false)?;
    let resolved = resolve(cx, face, true)?;
    let global = cx.face_lookup(resolved, Value::NIL)?;
    if global == Value::NIL { return Err(signal(cx, "Invalid face", face)); }
    let (unspecified, ignore) = (cx.intern("unspecified"), cx.intern(":ignore-defface"));
    let mut wanted = Vec::new();
    for index in 1..LFACE_VECTOR_SIZE {
        let value = cx.vector_get(global, Value::integer(index as i64).unwrap())?;
        if value == ignore { wanted.push((index, unspecified)); }
        else if value != unspecified { wanted.push((index, value)); }
    }
    let mut local = cx.face_lookup(resolved, frame)?;
    if local == Value::NIL { local = make_lisp_face(cx, resolved, frame)?; }
    let mut changes = Vec::new();
    for (index, value) in wanted {
        let old = cx.vector_get(local, Value::integer(index as i64).unwrap())?;
        if !cx.equal(old, value).map_err(|_| Fault::Circular(value))? { changes.push((index, value)); }
    }
    if changes.is_empty() { return Ok(Value::NIL); }
    // A frame showing another's faces gets its own once this changes them.
    if cx.frame_shares_faces(frame)? {
        cx.face_own(frame)?;
        local = cx.face_lookup(resolved, frame)?;
    }
    let no_inherit = cx.intern("face-no-inherit");
    let mirror = displayed_faces(cx, frame)? && cx.face_property(resolved, no_inherit)? == Value::NIL;
    let name = cx.symbol_name(resolved)?;
    for (index, value) in changes {
        cx.vector_set(local, Value::integer(index as i64).unwrap(), value)?;
        if mirror {
            let keyword = LFACE_ATTRIBUTES[index - 1];
            let mirrored = theme_value(cx, keyword, value, frame)?;
            cx.theme_face_attribute(&name, keyword, mirrored);
        }
    }
    Ok(Value::NIL)
}}

defsubr! { name = "face-font", gnu = "xfaces.c:4423", effect = Reads,
fn face_font(cx, face: Value; optional frame: Value, character: Value) {
    if frame == Value::TRUE {
        // The face's defaults for new frames: (bold), (italic) or both.
        let resolved = resolve(cx, face, true)?;
        let vector = cx.face_lookup(resolved, Value::NIL)?;
        if vector == Value::NIL { return Err(signal(cx, "Invalid face", resolved)); }
        let mut result = Value::NIL;
        for (attribute, style) in [(":weight", "bold"), (":slant", "italic")] {
            let index = LFACE_ATTRIBUTES.iter().position(|name| *name == attribute).unwrap() + 1;
            let value = cx.vector_get(vector, Value::integer(index as i64).unwrap())?;
            if value != cx.intern("unspecified") && value != cx.intern("normal") {
                let style = cx.intern(style);
                result = cx.cons(style, result);
            }
        }
        return Ok(result);
    }
    cx.face_frame(frame, true)?;
    let resolved = resolve(cx, face, true)?;
    if cx.face_lookup(resolved, Value::NIL)? == Value::NIL { return Err(signal(cx, "Invalid face", resolved)); }
    // The font the face draws CHARACTER, else ASCII, with on the display.
    // The initial terminal realizes none, but under a graphic display
    // record (a headless GUI) its one font, "tty", is every face's
    // (DIVERGENCES.md VM2-WIN5).
    if cx.font_host().is_none() {
        return Ok(if graphic_record(cx)? { cx.string("tty") } else { Value::NIL });
    }
    let ch = if character == Value::NIL { ' ' } else {
        let code = character.integer_value().filter(|c| (0..=0x3fffff).contains(c)).ok_or(Fault::Wrong("characterp", character))?;
        char::from_u32(code as u32).unwrap_or(char::REPLACEMENT_CHARACTER)
    };
    let name = cx.symbol_name(resolved)?;
    Ok(match super::font::face_font(cx, &name, ch)? { Some(font) => cx.string(&font), None => Value::NIL })
}}

/// Whether romacs-frame.el's display record, `romacs-display', is graphic.
fn graphic_record(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let record = cx.intern("romacs-display");
    let (graphic, mut tail) = (cx.intern(":graphic"), cx.variable_value(record)?.unwrap_or(Value::NIL));
    while let Some((key, rest)) = cx.pair(tail) {
        let Some((value, next)) = cx.pair(rest) else { break };
        if key == graphic { return Ok(value != Value::NIL); }
        tail = next;
    }
    Ok(false)
}

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

// A style font.c's tables (prims/font.rs) know (font_style_to_value).
fn style_name(cx: &mut Leaf<'_>, value: Value, slot: usize, message: &str) -> std::result::Result<(), Fault> {
    if cx.kind(value) != Kind::Symbol { return Err(Fault::Wrong("symbolp", value)); }
    let name = cx.symbol_name(value)?;
    if super::font::style_value(cx, slot, &name, false).is_some() { Ok(()) } else { Err(signal(cx, message, value)) }
}

fn string_nonempty(cx: &Leaf<'_>, value: Value) -> bool {
    matches!(cx.kind(value), Kind::String | Kind::Bytes) && cx.string_view(value).is_ok_and(|v| !v.bytes().is_empty())
}

fn car_safe(cx: &Leaf<'_>, value: Value) -> Value { cx.pair(value).map_or(Value::NIL, |(car, _)| car) }
fn cdr_safe(cx: &Leaf<'_>, value: Value) -> Value { cx.pair(value).map_or(Value::NIL, |(_, cdr)| cdr) }

fn valid_underline(cx: &mut Leaf<'_>, value: Value) -> bool {
    if value == Value::NIL || value == Value::TRUE || string_nonempty(cx, value) { return true; }
    if cx.pair(value).is_none() { return false; }
    let (position, color, style) = (cx.intern(":position"), cx.intern(":color"), cx.intern(":style"));
    let foreground = cx.intern("foreground-color");
    let styles = ["line", "double-line", "wave", "dots", "dashes"].map(|name| cx.intern(name));
    let mut list = value;
    let mut seen = std::collections::HashSet::new();
    while car_safe(cx, list) != Value::NIL {
        if !seen.insert(list.0) { return true; }
        let key = car_safe(cx, list);
        list = cdr_safe(cx, list);
        let val = car_safe(cx, list);
        list = cdr_safe(cx, list);
        if key == Value::NIL || (val == Value::NIL && key != position) { return false; }
        if key == color && !(val == foreground || string_nonempty(cx, val)) { return false; }
        if key == style && !styles.contains(&val) { return false; }
    }
    true
}

fn valid_box(cx: &mut Leaf<'_>, value: Value) -> bool {
    if value == Value::NIL { return true; }
    if let Some(n) = value.integer_value() { return n != 0; }
    if matches!(cx.kind(value), Kind::String | Kind::Bytes) { return string_nonempty(cx, value); }
    let Some((car, cdr)) = cx.pair(value) else { return false; };
    if car.integer_value().is_some() && cdr.integer_value().is_some() { return true; }
    let (width, color, style) = (cx.intern(":line-width"), cx.intern(":color"), cx.intern(":style"));
    let styles = ["pressed-button", "released-button", "flat-button"].map(|name| cx.intern(name));
    let nonzero = |v: Value| v.integer_value().is_some_and(|n| n != 0);
    let mut tail = value;
    let mut seen = std::collections::HashSet::new();
    while let Some((key, rest)) = cx.pair(tail) {
        if !seen.insert(tail.0) { return false; }
        let Some((v, next)) = cx.pair(rest) else { tail = rest; break; };
        let ok = if key == width {
            cx.pair(v).is_some_and(|(a, b)| nonzero(a) && nonzero(b)) || nonzero(v)
        } else if key == color {
            v == Value::NIL || string_nonempty(cx, v)
        } else if key == style {
            v == Value::NIL || styles.contains(&v)
        } else { false };
        if !ok { return false; }
        tail = next;
    }
    tail == Value::NIL
}

/// bitmap-spec-p (xfaces.c:880): a file name, or (WIDTH HEIGHT DATA) with
/// positive dimensions and DATA a string or bool-vector holding enough bits.
fn bitmap_spec(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<bool, Fault> {
    if matches!(cx.kind(value), Kind::String | Kind::Bytes) { return Ok(true); }
    let Some((width, rest)) = cx.pair(value) else { return Ok(false); };
    let Some((height, rest)) = cx.pair(rest) else { return Ok(false); };
    let Some((data, rest)) = cx.pair(rest) else { return Ok(false); };
    let (Some(w), Some(h)) = (width.integer_value(), height.integer_value()) else { return Ok(false); };
    if rest != Value::NIL || w <= 0 || h <= 0 { return Ok(false); }
    let bits = match cx.kind(data) {
        Kind::String | Kind::Bytes => cx.string_view(data)?.bytes().len() as i64 * 8,
        Kind::BoolVector => cx.bool_vector(data)?.len as i64,
        _ => return Ok(false),
    };
    let bytes_per_row = (w + 7) / 8;
    Ok(bits >= h * bytes_per_row * 8)
}

/// xfaces.c face_inheritance_cycle: FACE's inheritance reaches CHILD (or a
/// face inherits itself). A list stops at its first member with no lface.
/// GNU never lets a cycle form, so its recursion has no bound; ours does.
fn inheritance_cycle(cx: &mut Leaf<'_>, frame: Value, face: Value, child: Value, depth: usize) -> std::result::Result<bool, Fault> {
    if depth > 256 { return Ok(false); }
    let special = |cx: &mut Leaf<'_>, v: Value| v == Value::NIL || v == cx.intern("unspecified")
        || v == cx.intern(":ignore-defface") || v == cx.intern("reset");
    let mut check = |cx: &mut Leaf<'_>, member: Value| -> std::result::Result<Option<bool>, Fault> {
        let resolved = resolve(cx, member, false)?;
        let lface = if cx.kind(resolved) == Kind::Symbol { cx.face_lookup(resolved, frame)? } else { Value::NIL };
        if lface == Value::NIL { return Ok(None); }
        let parent = cx.vector_get(lface, Value::integer(16).unwrap())?;
        if parent == member || parent == child { return Ok(Some(true)); }
        if special(cx, parent) { return Ok(Some(false)); }
        Ok(Some(inheritance_cycle(cx, frame, parent, child, depth + 1)?))
    };
    if cx.pair(face).is_some() {
        let mut tail = face;
        while let Some((member, rest)) = cx.pair(tail) {
            match check(cx, member)? { None => break, Some(true) => return Ok(true), Some(false) => {} }
            tail = rest;
        }
        return Ok(false);
    }
    if cx.kind(face) == Kind::Symbol { return Ok(check(cx, face)?.unwrap_or(false)); }
    Ok(false)
}

/// merge_face_heights (VALUE, 10, nil) for validation.
fn test_height(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<Option<i64>, Fault> {
    if let Some(n) = value.integer_value() { return Ok(Some(n)); }
    if cx.kind(value) == Kind::Float {
        let product = cx.float_value(value)? * 10.0;
        return Ok((product.is_finite() && product.abs() < Value::MAX_FIXNUM as f64).then_some(product as i64));
    }
    function_height(cx, value, 10)
}

/// merge_face_heights (FUNCTION, UNDER, nil) for an absolute UNDER: the
/// function's value when it is an integer (safe_calln), else None.
fn function_height(cx: &mut Leaf<'_>, function: Value, under: i64) -> std::result::Result<Option<i64>, Fault> {
    if !super::vars::function_p(cx, function)? { return Ok(None); }
    Ok(cx.safe_call(function, &[Value::integer(under).unwrap()]).integer_value())
}

/// The theme holds data only (romacs-face): a function :height is mirrored
/// as its value over the default face's height (DIVERGENCES.md VM2-FACEFN1).
fn theme_value(cx: &mut Leaf<'_>, keyword: &str, value: Value, frame: Value) -> std::result::Result<Value, Fault> {
    if keyword != ":height" || value.integer_value().is_some() || cx.kind(value) == Kind::Float { return Ok(value); }
    let default = cx.intern("default");
    let lface = cx.face_lookup(default, frame)?;
    let under = if lface == Value::NIL { None } else { cx.vector_get(lface, Value::integer(4).unwrap())?.integer_value() };
    Ok(match under {
        Some(under) => function_height(cx, value, under)?.map_or(value, |h| Value::integer(h).unwrap()),
        None => value,
    })
}

pub(super) fn set_lisp_face_attribute(cx: &mut Leaf<'_>, face: Value, attr: Value, mut value: Value, frame: Value) -> Out {
    if cx.kind(face) != Kind::Symbol { return Err(Fault::Wrong("symbolp", face)); }
    if cx.kind(attr) != Kind::Symbol { return Err(Fault::Wrong("symbolp", attr)); }
    let face = resolve(cx, face, true)?;
    if frame.integer_value() == Some(0) {
        set_lisp_face_attribute(cx, face, attr, value, Value::TRUE)?;
        let frames = cx.windows().frame_list()?;
        let mut tail = frames;
        while let Some((each, rest)) = cx.pair(tail) {
            // A frame showing another's faces has this change through it.
            if !cx.frame_shares_faces(each)? { set_lisp_face_attribute(cx, face, attr, value, each)?; }
            tail = rest;
        }
        return Ok(face);
    }
    let unspecified = cx.intern("unspecified");
    let ignore = cx.intern(":ignore-defface");
    let reset = cx.intern("reset");
    let global = frame == Value::TRUE;
    let (frame, lface) = if global {
        let lface = cx.face_lookup(face, Value::NIL)?;
        if lface == Value::NIL { return Err(signal(cx, "Invalid face", face)); }
        if value == unspecified { value = ignore; }
        (Value::TRUE, lface)
    } else {
        let frame = cx.face_frame(frame, true)?;
        let mut lface = cx.face_lookup(face, frame)?;
        if lface == Value::NIL { lface = make_lisp_face(cx, face, frame)?; }
        (frame, lface)
    };
    let special = |v: Value| v == unspecified || v == ignore || v == reset;
    let name = cx.symbol_name(attr)?;
    let index = |keyword: &str| Value::integer((LFACE_ATTRIBUTES.iter().position(|k| *k == keyword).unwrap() + 1) as i64).unwrap();
    let nil_is_unspecified = |v: Value| if v == Value::NIL { unspecified } else { v };
    // Each arm validates VALUE and names the lface slot it replaces.
    let slot = match name.as_str() {
        ":family" | ":foundry" => {
            if !special(value) {
                if !matches!(cx.kind(value), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", value)); }
                if !string_nonempty(cx, value) {
                    return Err(signal(cx, if name == ":family" { "Invalid face family" } else { "Invalid face foundry" }, value));
                }
            }
            Some(name.as_str())
        }
        ":height" => {
            if !special(value) {
                if face == cx.intern("default") {
                    if !value.integer_value().is_some_and(|n| n > 0) {
                        return Err(signal(cx, "Default face height not absolute and positive", value));
                    }
                } else if !test_height(cx, value)?.is_some_and(|n| n > 0) {
                    return Err(signal(cx, "Face height does not produce a positive integer", value));
                }
            }
            Some(":height")
        }
        ":weight" => { if !special(value) { style_name(cx, value, super::font::WEIGHT, "Invalid face weight")?; } Some(":weight") }
        ":slant" => { if !special(value) { style_name(cx, value, super::font::SLANT, "Invalid face slant")?; } Some(":slant") }
        ":width" => { if !special(value) { style_name(cx, value, super::font::WIDTH, "Invalid face width")?; } Some(":width") }
        ":underline" => {
            if !special(value) && !valid_underline(cx, value) { return Err(signal(cx, "Invalid face underline", value)); }
            Some(":underline")
        }
        ":overline" | ":strike-through" => {
            let bad = (cx.kind(value) == Kind::Symbol && value != Value::TRUE && value != Value::NIL)
                || (matches!(cx.kind(value), Kind::String | Kind::Bytes) && !string_nonempty(cx, value));
            if !special(value) && bad {
                return Err(signal(cx, if name == ":overline" { "Invalid face overline" } else { "Invalid face strike-through" }, value));
            }
            Some(name.as_str())
        }
        ":box" => {
            if value == Value::TRUE { value = Value::integer(1).unwrap(); }
            if !special(value) && !valid_box(cx, value) { return Err(signal(cx, "Invalid face box", value)); }
            Some(":box")
        }
        ":inverse-video" | ":extend" => {
            if !special(value) {
                if cx.kind(value) != Kind::Symbol { return Err(Fault::Wrong("symbolp", value)); }
                if value != Value::TRUE && value != Value::NIL {
                    return Err(signal(cx, if name == ":extend" { "Invalid extend face attribute value" }
                        else { "Invalid inverse-video face attribute value" }, value));
                }
            }
            Some(name.as_str())
        }
        ":foreground" | ":distant-foreground" | ":background" => {
            value = nil_is_unspecified(value);
            if !special(value) {
                if !matches!(cx.kind(value), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", value)); }
                if !string_nonempty(cx, value) {
                    let message = match name.as_str() {
                        ":foreground" => "Empty foreground color value",
                        ":background" => "Empty background color value",
                        _ => "Empty distant-foreground color value",
                    };
                    return Err(signal(cx, message, value));
                }
            }
            Some(name.as_str())
        }
        ":stipple" => {
            if !special(value) && value != Value::NIL && !bitmap_spec(cx, value)? {
                return Err(signal(cx, "Invalid stipple attribute", value));
            }
            Some(":stipple")
        }
        // A frame without a window system ignores fonts; the default for new
        // frames keeps only the special values (xfaces.c:3627-3680).
        ":font" => {
            if !global { None }
            else if special(value) { Some(":font") }
            else { return Err(super::fileio::refuse("internal-set-lisp-face-attribute :font for new frames")); }
        }
        // query-fontset validates every value but `reset'.
        ":fontset" => {
            if !global { None }
            else if value == reset { Some(":fontset") }
            else { return Err(super::fileio::refuse("internal-set-lisp-face-attribute :fontset for new frames")); }
        }
        ":inherit" => {
            let mut tail = if cx.kind(value) == Kind::Symbol { Value::NIL } else { value };
            let mut seen = std::collections::HashSet::new();
            while let Some((car, cdr)) = cx.pair(tail) {
                if cx.kind(car) != Kind::Symbol || !seen.insert(tail.0) { break; }
                tail = cdr;
            }
            let table = if global { Value::NIL } else { frame };
            if value == face || inheritance_cycle(cx, table, value, face, 0)? {
                return Err(signal(cx, "Face inheritance results in inheritance cycle", value));
            }
            if tail != Value::NIL { return Err(signal(cx, "Invalid face inheritance", value)); }
            Some(":inherit")
        }
        ":bold" => {
            if value != reset { value = if value == Value::NIL { cx.intern("normal") } else { cx.intern("bold") }; }
            Some(":weight")
        }
        ":italic" => {
            if value != reset { value = if value == Value::NIL { cx.intern("normal") } else { cx.intern("italic") }; }
            Some(":slant")
        }
        _ => return Err(signal(cx, "Invalid face attribute name", attr)),
    };
    let Some(keyword) = slot else { return Ok(face); };
    // A frame showing another's faces gets its own only when VALUE changes
    // what it shows.
    let mut lface = lface;
    if !global && cx.frame_shares_faces(frame)? {
        let shown = cx.vector_get(lface, index(keyword))?;
        if cx.equal(shown, value).map_err(|_| Fault::Circular(value))? { return Ok(face); }
        cx.face_own(frame)?;
        lface = cx.face_lookup(face, frame)?;
    }
    let old = cx.vector_get(lface, index(keyword))?;
    cx.vector_set(lface, index(keyword), value)?;
    let changed = !cx.equal(old, value).map_err(|_| Fault::Circular(value))?;
    if !global && changed && displayed_faces(cx, frame)? {
        let no_inherit = cx.intern("face-no-inherit");
        if cx.face_property(face, no_inherit)? == Value::NIL {
            let face_name = cx.symbol_name(face)?;
            let mirrored = theme_value(cx, keyword, value, frame)?;
            cx.theme_face_attribute(&face_name, keyword, mirrored);
        }
    }
    if special(value) || !changed { return Ok(face); }
    // Colors of a few named faces are mirrored into frame parameters.
    let face_name = cx.symbol_name(face)?;
    let parameter = match (face_name.as_str(), keyword) {
        ("default", ":foreground") => Some("foreground-color"),
        ("default", ":background") => Some("background-color"),
        ("scroll-bar", ":foreground") => Some("scroll-bar-foreground"),
        ("scroll-bar", ":background") => Some("scroll-bar-background"),
        ("border", ":background") => Some("border-color"),
        ("cursor", ":background") => Some("cursor-color"),
        ("mouse", ":background") => Some("mouse-color"),
        _ => None,
    };
    if let Some(parameter) = parameter {
        let key = cx.intern(parameter);
        if global {
            // store_in_alist (&Vdefault_frame_alist, PARAM, VALUE).
            let alist_name = cx.intern("default-frame-alist");
            let alist = cx.variable_value(alist_name)?.unwrap_or(Value::NIL);
            let mut tail = alist;
            let mut stored = false;
            while let Some((entry, rest)) = cx.pair(tail) {
                if cx.pair(entry).is_some_and(|(k, _)| k == key) { cx.setcdr(entry, value)?; stored = true; break; }
                tail = rest;
            }
            if !stored {
                let entry = cx.cons(key, value);
                let alist = cx.cons(entry, alist);
                cx.variable_set(alist_name, alist)?;
            }
        } else {
            cx.windows().store_frame_parameter(frame, key, value)?;
            // update_face_from_frame_parameter (xfaces.c:3896): a new
            // background may change the frame's background mode, and with
            // it which defface specs apply.
            if parameter == "background-color" && cx.windows().window_system(frame)? != Value::NIL && displayed_faces(cx, frame)? {
                let function = cx.intern("frame-set-background-mode");
                if cx.fboundp(function) { cx.safe_call(function, &[frame]); }
            }
        }
    }
    Ok(face)
}

defsubr! { name = "internal-set-lisp-face-attribute", gnu = "xfaces.c:3217", effect = Global,
fn internal_set_lisp_face_attribute(cx, face: Value, attr: Value, value: Value; optional frame: Value) {
    set_lisp_face_attribute(cx, face, attr, value, frame)
}}

defsubr! { name = "display-supports-face-attributes-p", gnu = "xfaces.c:5764", effect = Reads,
fn display_supports_face_attributes_p(cx, attributes: Value; optional display: Value) {
    // GNU gives up in batch before examining either argument; without a
    // graphic display record a frame is an initial-terminal frame
    // (VM2-WIN5), which supports no attribute.
    let batch = cx.intern("noninteractive");
    if cx.variable_value(batch)?.is_some_and(|v| v != Value::NIL) || !graphic_record(cx)? {
        return Ok(Value::NIL);
    }
    // Romacs has one display: a DISPLAY that is not a frame names it.
    let frame = cx.face_frame(if cx.kind(display) == Kind::Frame { display } else { Value::NIL }, true)?;
    let unspecified = cx.intern("unspecified");
    let mut attrs = vec![unspecified; LFACE_VECTOR_SIZE];
    merge_face_ref(cx, attributes, frame, &mut attrs, 0)?;
    let default = cx.intern("default");
    let default = cx.face_lookup(default, frame)?;
    Ok(if gui_supports(cx, &attrs, default)? { Value::TRUE } else { Value::NIL })
}}

/// The lface slot of the attribute named NAME.
fn lface_slot(name: &str) -> usize {
    LFACE_ATTRIBUTES.iter().position(|n| *n == name).expect("an lface attribute") + 1
}

/// The lface slot of the attribute KEYWORD.
fn lface_index(cx: &Leaf<'_>, keyword: Value) -> Option<usize> {
    LFACE_ATTRIBUTES.iter().position(|name| cx.symbol_name(keyword).is_ok_and(|n| n == *name)).map(|i| i + 1)
}

/// xfaces.c merge_face_ref into ATTRS: a face name, a property list,
/// (foreground-color . COLOR), (background-color . COLOR), or a list of
/// these whose first takes precedence. Invalid references merge nothing.
fn merge_face_ref(cx: &mut Leaf<'_>, face: Value, frame: Value, attrs: &mut [Value], depth: usize)
    -> std::result::Result<(), Fault> {
    if face == Value::NIL || depth > 64 { return Ok(()); }
    let Some((first, rest)) = cx.pair(face) else { return merge_named_face(cx, face, frame, attrs, depth) };
    let (fg, bg) = (cx.intern("foreground-color"), cx.intern("background-color"));
    if first == fg || first == bg {
        if matches!(cx.kind(rest), Kind::String | Kind::Bytes) {
            attrs[lface_slot(if first == fg { ":foreground" } else { ":background" })] = rest;
        }
        return Ok(());
    }
    if cx.kind(first) == Kind::Symbol && cx.symbol_name(first).is_ok_and(|n| n.starts_with(':')) {
        let inherit = cx.intern(":inherit");
        let mut tail = face;
        while let Some((keyword, rest)) = cx.pair(tail) {
            let Some((value, next)) = cx.pair(rest) else { break };
            if keyword == inherit { merge_face_ref(cx, value, frame, attrs, depth + 1)?; }
            else if let Some(index) = lface_index(cx, keyword) { attrs[index] = value; }
            tail = next;
        }
        return Ok(());
    }
    merge_face_ref(cx, rest, frame, attrs, depth + 1)?;
    merge_face_ref(cx, first, frame, attrs, depth + 1)
}

/// merge_named_face: FACE's attributes on FRAME over those it inherits.
fn merge_named_face(cx: &mut Leaf<'_>, face: Value, frame: Value, attrs: &mut [Value], depth: usize)
    -> std::result::Result<(), Fault> {
    if cx.kind(face) != Kind::Symbol { return Ok(()); }
    let Ok(face) = resolve(cx, face, false) else { return Ok(()) };
    let lface = cx.face_lookup(face, frame)?;
    if lface == Value::NIL { return Ok(()); }
    let unspecified = cx.intern("unspecified");
    let inherit = lface_slot(":inherit");
    let parent = cx.vector_get(lface, Value::integer(inherit as i64).unwrap())?;
    if parent != unspecified { merge_face_ref(cx, parent, frame, attrs, depth + 1)?; }
    for index in 1..LFACE_VECTOR_SIZE {
        let value = cx.vector_get(lface, Value::integer(index as i64).unwrap())?;
        if value != unspecified && index != inherit { attrs[index] = value; }
    }
    Ok(())
}

/// gui_supports_face_attributes_p: each specified attribute differs from
/// the default face's (`reset' meaning the default's own), and one that
/// selects a font selects another font than the default face's.
fn gui_supports(cx: &mut Leaf<'_>, attrs: &[Value], default: Value) -> std::result::Result<bool, Fault> {
    let (unspecified, reset) = (cx.intern("unspecified"), cx.intern("reset"));
    // realize_default_face makes these nil where the default leaves them
    // unspecified.
    let nil_by_default = [":extend", ":underline", ":overline", ":strike-through", ":box", ":inverse-video", ":stipple"]
        .map(lface_slot);
    let default_of = |cx: &mut Leaf<'_>, index: usize| {
        let value = if default == Value::NIL { unspecified } else { cx.vector_get(default, Value::integer(index as i64).unwrap())? };
        Ok::<_, Fault>(if value == unspecified && nil_by_default.contains(&index) { Value::NIL } else { value })
    };
    for keyword in [":underline", ":inverse-video", ":extend", ":foreground", ":distant-foreground",
                    ":background", ":stipple", ":overline", ":strike-through", ":box"] {
        let index = lface_slot(keyword);
        let theirs = default_of(cx, index)?;
        let value = if attrs[index] == reset { theirs } else { attrs[index] };
        if value != unspecified && cx.equal(value, theirs).map_err(|_| Fault::Circular(value))? { return Ok(false); }
    }
    let font_keys = [":family", ":foundry", ":height", ":weight", ":slant", ":width"];
    if font_keys.iter().all(|k| attrs[lface_slot(k)] == unspecified) {
        // Of the underline styles, a window system draws `line' and `wave'.
        let underline = attrs[lface_slot(":underline")];
        let style = cx.pair(underline).filter(|&(key, _)| key == cx.intern(":style"))
            .map(|(_, rest)| cx.pair(rest).map_or(Value::NIL, |(style, _)| style));
        return Ok(style.is_none_or(|s| s == cx.intern("line") || s == cx.intern("wave")));
    }
    let mut merged = Vec::with_capacity(LFACE_VECTOR_SIZE);
    for (index, &value) in attrs.iter().enumerate() {
        merged.push(if value == unspecified || value == reset || index == 0 { default_of(cx, index)? } else { value });
    }
    let defaults = (0..LFACE_VECTOR_SIZE).map(|i| default_of(cx, i)).collect::<std::result::Result<Vec<_>, _>>()?;
    let Ok(crate::FaceFont { family, .. }) = cx.face_font_request("default") else { return Ok(false) };
    let wanted = match cx.string_view(merged[lface_slot(":family")]) {
        Ok(name) => String::from_utf8_lossy(name.bytes()).into_owned(),
        Err(_) => family.clone(),
    };
    // An unknown family falls back to any family, as font_find_for_lface's
    // last alternative does. No font, or the default face's own: unsupported.
    let font = closest_font(cx, Some(&wanted), &merged).or_else(|| closest_font(cx, None, &merged));
    let Some(font) = font else { return Ok(false) };
    if closest_font(cx, Some(&family), &defaults).as_ref() != Some(&font) { return Ok(true); }
    // The same face: a different size is still another font.
    let slot = lface_slot(":height");
    let height = merged[slot];
    let unscaled = cx.kind(height) == Kind::Float && cx.float_value(height).is_ok_and(|f| f == 1.0);
    Ok(attrs[slot] != unspecified && attrs[slot] != reset && !unscaled
        && !cx.equal(height, defaults[slot]).map_err(|_| Fault::Circular(height))?)
}

/// font_find_for_lface over Romacs's installed fonts: FAMILY's face (any
/// family's for nil) nearest LFACE's width, weight and slant, compared in
/// that order, the default `face-font-selection-order'.
fn closest_font(cx: &mut Leaf<'_>, family: Option<&str>, lface: &[Value]) -> Option<crate::display_host::FontEntity> {
    use super::font::{style_value, SLANT, WEIGHT, WIDTH};
    let mut style = |keyword: &str, slot: usize, normal: i64| {
        let name = cx.symbol_name(lface[lface_slot(keyword)]).ok();
        name.and_then(|n| style_value(cx, slot, &n, false)).map_or(normal, |v| v >> 8)
    };
    let (width, weight, slant) = (style(":width", WIDTH, 100), style(":weight", WEIGHT, 80), style(":slant", SLANT, 100));
    let query = crate::display_host::FontQuery { family: family.map(str::to_owned), ..Default::default() };
    let fonts = cx.font_host()?.list_fonts(&query);
    fonts.into_iter().min_by_key(|f| ((f.width - width).abs(), (f.weight - weight).abs(), (f.slant - slant).abs()))
}

defsubr! { name = "frame--face-hash-table", gnu = "xfaces.c:4619", effect = Global,
fn frame_face_hash_table(cx; optional frame: Value) {
    // A live frame always has its face table, as GNU's frames do; a child
    // frame showing another's faces answers with that frame's.
    let frame = cx.face_frame(frame, true)?;
    let owner = cx.face_owner(frame)?;
    cx.face_table(owner, true)
}}

defsubr! { name = "romacs--frame-shares-faces-p", gnu = "romacs:faces", effect = Reads,
fn romacs_frame_shares_faces_p(cx; optional frame: Value) {
    // FRAME shows another frame's faces: a child frame nothing set a face
    // on alone (DIVERGENCES.md VM2-CHILD-FRAME). `face-spec-recalc' has
    // nothing to recompute there.
    let frame = cx.face_frame(frame, true)?;
    Ok(if cx.frame_shares_faces(frame)? { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "internal-lisp-face-equal-p", gnu = "xfaces.c:4382", effect = Reads,
fn internal_lisp_face_equal_p(cx, face1: Value, face2: Value; optional frame: Value, inherit: Value) {
    // FRAME t: the defaults for new frames; nil: the selected frame.
    let frame = if frame == Value::TRUE { Value::NIL } else { cx.face_frame(frame, true)? };
    // lface_from_face_name (F, FACE, true): an alias cycle signals.
    let lface = |cx: &mut Leaf<'_>, face: Value| -> Out {
        let resolved = resolve(cx, face, true)?;
        let found = cx.face_lookup(resolved, frame)?;
        if found == Value::NIL { Err(signal(cx, "Invalid face", face)) } else { Ok(found) }
    };
    let (a, b) = (lface(cx, face1)?, lface(cx, face2)?);
    let mut equal = true;
    for index in 1..LFACE_VECTOR_SIZE {
        let index = Value::integer(index as i64).unwrap();
        let (x, y) = (cx.vector_get(a, index)?, cx.vector_get(b, index)?);
        if !face_attr_equal(cx, x, y)? { equal = false; break; }
    }
    // A face that inherits directly from the other, and whose other
    // attributes are its parent's or unspecified, equals it.
    if !equal && inherit != Value::NIL {
        let slot = Value::integer(lface_slot(":inherit") as i64).unwrap();
        let unspecified = cx.intern("unspecified");
        for (child, parent, other) in [(a, b, face2), (b, a, face1)] {
            if cx.vector_get(child, slot)? != other { continue; }
            equal = true;
            for index in 1..LFACE_VECTOR_SIZE {
                let at = Value::integer(index as i64).unwrap();
                if at == slot { continue; }
                let (x, y) = (cx.vector_get(child, at)?, cx.vector_get(parent, at)?);
                if !(x == unspecified || face_attr_equal(cx, x, y)?) { equal = false; break; }
            }
            break;
        }
    }
    Ok(if equal { Value::TRUE } else { Value::NIL })
}}

/// xfaces.c face_attr_equal_p: lface attributes X and Y of one type,
/// strings by their bytes, numbers and symbols only when `eq', other
/// objects `equal'.
fn face_attr_equal(cx: &mut Leaf<'_>, x: Value, y: Value) -> std::result::Result<bool, Fault> {
    if x == y { return Ok(true); }
    let text = |k: Kind| matches!(k, Kind::String | Kind::Bytes);
    let (kx, ky) = (cx.kind(x), cx.kind(y));
    if kx != ky && !(text(kx) && text(ky)) { return Ok(false); }
    match kx {
        Kind::String | Kind::Bytes => Ok(cx.string_view(x)?.bytes() == cx.string_view(y)?.bytes()),
        Kind::Symbol | Kind::Integer => Ok(false),
        _ => cx.equal(x, y).map_err(|_| Fault::Circular(x)),
    }
}

/// Whether lface vector LFACE has every attribute unspecified.
fn empty_lface(cx: &mut Leaf<'_>, lface: Value) -> std::result::Result<bool, Fault> {
    let unspecified = cx.intern("unspecified");
    for index in 1..LFACE_VECTOR_SIZE {
        if cx.vector_get(lface, Value::integer(index as i64).unwrap())? != unspecified { return Ok(false); }
    }
    Ok(true)
}

/// Whether lface vectors A and B hold `equal' attributes.
fn same_lface(cx: &mut Leaf<'_>, a: Value, b: Value) -> std::result::Result<bool, Fault> {
    for index in 1..LFACE_VECTOR_SIZE {
        let index = Value::integer(index as i64).unwrap();
        let (x, y) = (cx.vector_get(a, index)?, cx.vector_get(b, index)?);
        if !cx.equal(x, y).map_err(|_| Fault::Circular(x))? { return Ok(false); }
    }
    Ok(true)
}

defsubr! { name = "color-values-from-color-spec", gnu = "xfaces.c:1029", effect = Pure,
fn color_values_from_color_spec(cx, spec: Value) {
    let bytes = cx.string_view(spec)?.bytes().to_vec();
    // parse_color_spec reads a C string.
    let bytes = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
    let Some((r, g, b)) = romacs_display_protocol::x11_colors::x11_color_spec_16bit(bytes) else {
        return Ok(Value::NIL);
    };
    let mut list = Value::NIL;
    for c in [b, g, r] { list = cx.cons(Value::integer(i64::from(c)).unwrap(), list); }
    Ok(list)
}}

// xfaces.c Fmerge_face_attribute: VALUE1 unless it is unspecified, then
// VALUE2; for :height, merge_face_heights (VALUE1, VALUE2, VALUE1): a scale
// applies to VALUE2 and a function is called on it.
defsubr! { name = "merge-face-attribute", gnu = "xfaces.c:4214", effect = Global,
fn merge_face_attribute(cx, attribute: Value, value1: Value, value2: Value) -> Step {
    if value1 == cx.intern("unspecified") || value1 == cx.intern(":ignore-defface") { return Ok(Step::Done(value2)); }
    if attribute != cx.intern(":height") || value1.integer_value().is_some() { return Ok(Step::Done(value1)); }
    if cx.kind(value1) == Kind::Float {
        let scale = cx.float_value(value1)?;
        return Ok(Step::Done(if let Some(to) = value2.integer_value() {
            Value::integer((scale * to as f64) as i64).map_err(Fault::Error)?
        } else if cx.kind(value2) == Kind::Float {
            let to = cx.float_value(value2)?;
            cx.float(scale * to)
        } else { value1 }));
    }
    let lambda = cx.pair(value1).is_some_and(|(car, _)| car == cx.intern("lambda") || car == cx.intern("closure"));
    if matches!(cx.kind(value1), Kind::Closure | Kind::Subr) || lambda
        || (cx.kind(value1) == Kind::Symbol && value1 != Value::NIL && cx.fboundp(value1)) {
        return Ok(Step::Call { f: value1, args: vec![value2], then: merged_height, saved: vec![value1, value2] });
    }
    Ok(Step::Done(value1))
}}

/// A function's height, which stays absolute when VALUE2 was.
fn merged_height(_: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(if saved[1].integer_value().is_some() && result.integer_value().is_none() { saved[0] } else { result }))
}

// xfaces.c:794. THOROUGHLY also frees unused fonts in GNU; the display
// host owns opened fonts here, so only the realized faces go.
defsubr! { name = "clear-face-cache", gnu = "xfaces.c:794", effect = Global,
fn clear_face_cache(cx; optional thoroughly: Value) {
    cx.clear_face_cache();
    Ok(Value::NIL)
}}
