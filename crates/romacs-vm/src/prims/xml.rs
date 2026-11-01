//! GNU xml.c. `libxml-parse-html-region' parses with Romacs's own HTML
//! parser (crate::html), which builds the tree libxml2 builds for GNU; there
//! is no XML parser, so `libxml-available-p' still answers nil, as a GNU
//! built without HAVE_LIBXML2 does, and its callers (dbus.el, nnatom.el)
//! parse XML with xml.el (VM2-LIBXML1).
use super::*;
use crate::html::Node;

defsubr! { name = "libxml-available-p", gnu = "xml.c:321", effect = Pure,
fn libxml_available_p(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}

/// make_dom: (NAME ATTRS . CHILDREN), a text node's string, or
/// (comment nil TEXT).
fn dom(cx: &mut Leaf<'_>, node: &Node) -> Value {
    let string = |cx: &mut Leaf<'_>, s: &str| cx.make_string(s.as_bytes().to_vec(), !s.is_ascii());
    match node {
        Node::Text(text) => string(cx, text),
        Node::Comment(text) => {
            let items = [cx.intern("comment"), Value::NIL, string(cx, text)];
            cx.coding_list(&items)
        }
        Node::Element { name, attrs, children } => {
            let attrs: Vec<Value> = attrs.iter().map(|(k, v)| {
                let (k, v) = (cx.intern(k), string(cx, v));
                cx.cons(k, v)
            }).collect();
            let mut items = vec![cx.intern(name), cx.coding_list(&attrs)];
            items.extend(children.iter().map(|c| dom(cx, c)));
            cx.coding_list(&items)
        }
    }
}

// Fparse_region with HTMLP: the region START..END (default the accessible
// buffer, in either order) as libxml2 reads it; nil when it holds no
// element. Unless DISCARD-COMMENTS, top-level comments come back with the
// root elements as (top nil NODE...), with the quirk of GNU's loop: a
// lone top-level node that is no element gives nil.
defsubr! { name = "libxml-parse-html-region", gnu = "xml.c:285", effect = Reads,
fn libxml_parse_html_region(cx; optional start: Value, end: Value, base_url: Value, discard_comments: Value) {
    if base_url != Value::NIL && !matches!(cx.kind(base_url), Kind::String | Kind::Bytes) {
        return Err(Fault::Wrong("stringp", base_url));
    }
    let (start, end) = if start == Value::NIL || end == Value::NIL {
        let buffer = cx.current_buffer()?;
        let (begv, zv) = cx.accessible_positions(buffer)?;
        let position = |n: i64| Value::integer(n).map_err(Fault::Error);
        (if start == Value::NIL { position(begv)? } else { start }, if end == Value::NIL { position(zv)? } else { end })
    } else { (start, end) };
    if matches!(cx.kind(start), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("integer-or-marker-p", start)); }
    let text = cx.text_span(start, end, true)?;
    let nodes = crate::html::parse(&String::from_utf8_lossy(&text.bytes));
    if discard_comments == Value::NIL && nodes.len() > 1 {
        let mut items = vec![cx.intern("top"), Value::NIL];
        items.extend(nodes.iter().map(|n| dom(cx, n)));
        return Ok(cx.coding_list(&items));
    }
    Ok(nodes.iter().find(|n| matches!(n, Node::Element { .. })).map_or(Value::NIL, |root| dom(cx, root)))
}}
