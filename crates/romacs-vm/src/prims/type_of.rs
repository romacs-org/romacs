//! GNU data.c Ftype_of preserves `integer` for both integer representations.
use super::*;

defsubr! { name = "type-of", gnu = "data.c:187", effect = Pure,
fn type_of(cx, object: Value) {
    classify(cx, object, false)
}}

defsubr! { name = "cl-type-of", gnu = "data.c:202", effect = Pure,
fn cl_type_of(cx, object: Value) {
    classify(cx, object, true)
}}

fn classify(cx: &mut Leaf<'_>, object: Value, precise: bool) -> Out {
    use crate::prim::leaf::Kind;
    let name = match cx.kind(object) {
        Kind::Integer if precise => "fixnum",
        Kind::Bignum if precise => "bignum",
        Kind::Integer | Kind::Bignum => "integer",
        Kind::Symbol if precise && object == Value::NIL => "null",
        Kind::Symbol if precise && object == Value::TRUE => "boolean",
        Kind::Symbol => "symbol",
        Kind::Float => "float",
        Kind::Cons => "cons",
        Kind::String | Kind::Bytes => "string",
        Kind::Vector => "vector",
        Kind::BoolVector => "bool-vector",
        Kind::Record => return Ok(cx.record_type(object)),
        Kind::Buffer | Kind::ByteBuffer => "buffer",
        Kind::Marker => "marker",
        Kind::Overlay => "overlay",
        Kind::HashTable => "hash-table",
        Kind::Obarray => "obarray",
        Kind::SyntaxTable | Kind::CharTable => "char-table",
        Kind::SubCharTable => "sub-char-table",
        Kind::Window => "window",
        Kind::Frame => "frame",
        Kind::Terminal => "terminal",
        Kind::WindowConfiguration => "window-configuration",
        Kind::Sqlite => "sqlite",
        Kind::Subr if precise => if crate::prim::name(object).and_then(super::special_forms::special_form_min).is_some() { "special-form" } else { "primitive-function" },
        Kind::Subr => "subr",
        Kind::Closure => "interpreted-function",
        Kind::Cell | Kind::Unbound | Kind::Invalid => {
            return Err(Fault::Error("type-of: invalid Lisp object".into()));
        }
    };
    Ok(cx.intern(name))
}
