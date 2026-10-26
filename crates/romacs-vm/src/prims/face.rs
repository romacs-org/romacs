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
fn internal_make_lisp_face(cx, face: Value; optional frame: Value) { make_lisp_face(cx, face, frame) }}

fn make_lisp_face(cx: &mut Leaf<'_>, face: Value, frame: Value) -> Out {
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

// font.c weight_table, slant_table and width_table: the C defaults of
// font-weight-table, font-slant-table and font-width-table, matched exactly
// and then case-insensitively by font_style_to_value.
const WEIGHTS: &[&str] = &["thin", "ultra-light", "ultralight", "extra-light", "extralight", "light",
    "semi-light", "semilight", "demilight", "regular", "normal", "unspecified", "book", "medium",
    "semi-bold", "semibold", "demibold", "demi-bold", "demi", "bold", "extra-bold", "extrabold",
    "ultra-bold", "ultrabold", "black", "heavy", "ultra-heavy", "ultraheavy"];
const SLANTS: &[&str] = &["reverse-oblique", "ro", "reverse-italic", "ri", "normal", "r", "unspecified",
    "italic", "i", "ot", "oblique", "o"];
const WIDTHS: &[&str] = &["ultra-condensed", "ultracondensed", "extra-condensed", "extracondensed",
    "condensed", "compressed", "narrow", "semi-condensed", "semicondensed", "demicondensed", "normal",
    "medium", "regular", "unspecified", "semi-expanded", "semiexpanded", "demiexpanded", "expanded",
    "extra-expanded", "extraexpanded", "ultra-expanded", "ultraexpanded", "wide"];

fn style_name(cx: &mut Leaf<'_>, value: Value, table: &[&str], message: &str) -> std::result::Result<(), Fault> {
    if cx.kind(value) != Kind::Symbol { return Err(Fault::Wrong("symbolp", value)); }
    let name = cx.symbol_name(value)?;
    if table.iter().any(|entry| entry.eq_ignore_ascii_case(&name)) { Ok(()) } else { Err(signal(cx, message, value)) }
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

/// merge_face_heights (VALUE, 10, nil) for validation; functions are Lisp calls.
fn test_height(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<Option<i64>, Fault> {
    if let Some(n) = value.integer_value() { return Ok(Some(n)); }
    if cx.kind(value) == Kind::Float {
        let product = cx.float_value(value)? * 10.0;
        return Ok((product.is_finite() && product.abs() < Value::MAX_FIXNUM as f64).then_some(product as i64));
    }
    let lambda = cx.pair(value).is_some_and(|(car, _)| car == cx.intern("lambda") || car == cx.intern("closure"));
    if matches!(cx.kind(value), Kind::Closure | Kind::Subr) || lambda
        || (cx.kind(value) == Kind::Symbol && value != Value::NIL && cx.fboundp(value)) {
        return Err(super::fileio::refuse("internal-set-lisp-face-attribute function :height"));
    }
    Ok(None)
}

fn set_lisp_face_attribute(cx: &mut Leaf<'_>, face: Value, attr: Value, mut value: Value, frame: Value) -> Out {
    if cx.kind(face) != Kind::Symbol { return Err(Fault::Wrong("symbolp", face)); }
    if cx.kind(attr) != Kind::Symbol { return Err(Fault::Wrong("symbolp", attr)); }
    let face = resolve(cx, face, true)?;
    if frame.integer_value() == Some(0) {
        set_lisp_face_attribute(cx, face, attr, value, Value::TRUE)?;
        let frames = cx.windows().frame_list()?;
        let mut tail = frames;
        while let Some((each, rest)) = cx.pair(tail) {
            set_lisp_face_attribute(cx, face, attr, value, each)?;
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
        ":weight" => { if !special(value) { style_name(cx, value, WEIGHTS, "Invalid face weight")?; } Some(":weight") }
        ":slant" => { if !special(value) { style_name(cx, value, SLANTS, "Invalid face slant")?; } Some(":slant") }
        ":width" => { if !special(value) { style_name(cx, value, WIDTHS, "Invalid face width")?; } Some(":width") }
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
    let old = cx.vector_get(lface, index(keyword))?;
    cx.vector_set(lface, index(keyword), value)?;
    let changed = !cx.equal(old, value).map_err(|_| Fault::Circular(value))?;
    if !global && changed {
        let no_inherit = cx.intern("face-no-inherit");
        if cx.face_property(face, no_inherit)? == Value::NIL {
            let face_name = cx.symbol_name(face)?;
            cx.theme_face_attribute(&face_name, keyword, value);
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
        }
    }
    Ok(face)
}

defsubr! { name = "internal-set-lisp-face-attribute", gnu = "xfaces.c:3217", effect = Global,
fn internal_set_lisp_face_attribute(cx, face: Value, attr: Value, value: Value; optional frame: Value) {
    set_lisp_face_attribute(cx, face, attr, value, frame)
}}

defsubr! { name = "display-supports-face-attributes-p", gnu = "xfaces.c:5764", effect = Reads,
fn display_supports_face_attributes_p(_cx, _attributes: Value; optional _display: Value) {
    // GNU gives up in batch before examining either argument; VM2 frames
    // are initial-terminal frames (VM2-WIN5), which support no attributes.
    Ok(Value::NIL)
}}
