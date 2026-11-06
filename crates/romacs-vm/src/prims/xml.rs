//! GNU xml.c. `libxml-parse-html-region' parses with Romacs's own HTML
//! parser (crate::html), which builds the tree libxml2 builds for GNU, and
//! `libxml-parse-xml-region' with crate::xml_read's document reader, which
//! answers as libxml2 does; so `libxml-available-p' is t (VM2-LIBXML1).
//! `romacs--xml-read-element' reads an element as xml.el itself does
//! (crate::xml_read too), for XMPP's stanzas.
use super::*;
use crate::html::Node;

defsubr! { name = "libxml-available-p", gnu = "xml.c:321", effect = Pure,
fn libxml_available_p(cx) {
    let _ = cx;
    Ok(Value::TRUE)
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

/// parse_region's text: START..END (default the accessible buffer, in
/// either order).
fn region_text(cx: &mut Leaf<'_>, start: Value, end: Value, base_url: Value) -> std::result::Result<Vec<u8>, Fault> {
    if base_url != Value::NIL && !matches!(cx.kind(base_url), Kind::String | Kind::Bytes) {
        return Err(Fault::Wrong("stringp", base_url));
    }
    let (start, end) = if start == Value::NIL || end == Value::NIL {
        let (_, begv, zv) = cx.point_extent()?;
        let position = |n: i64| Value::integer(n).map_err(Fault::Error);
        (if start == Value::NIL { position(begv)? } else { start }, if end == Value::NIL { position(zv)? } else { end })
    } else { (start, end) };
    if matches!(cx.kind(start), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("integer-or-marker-p", start)); }
    Ok(cx.text_span(start, end, true)?.bytes)
}

// Fparse_region with HTMLP: the region as libxml2 reads it; nil when it
// holds no element. Unless DISCARD-COMMENTS, top-level comments come back
// with the root elements as (top nil NODE...), with the quirk of GNU's
// loop: a lone top-level node that is no element gives nil.
defsubr! { name = "libxml-parse-html-region", gnu = "xml.c:285", effect = Reads,
fn libxml_parse_html_region(cx; optional start: Value, end: Value, base_url: Value, discard_comments: Value) {
    let text = region_text(cx, start, end, base_url)?;
    let nodes = crate::html::parse(&String::from_utf8_lossy(&text));
    if discard_comments == Value::NIL && nodes.len() > 1 {
        let mut items = vec![cx.intern("top"), Value::NIL];
        items.extend(nodes.iter().map(|n| dom(cx, n)));
        return Ok(cx.coding_list(&items));
    }
    Ok(nodes.iter().find(|n| matches!(n, Node::Element { .. })).map_or(Value::NIL, |root| dom(cx, root)))
}}

// Fparse_region without HTMLP: nil when the region is not well-formed.
// Unless DISCARD-COMMENTS, GNU's loop over the top-level nodes gives (top
// nil NODE...) when a node other than the last is not nil: such a node is
// dropped, the last one kept, nil or not. Otherwise, the root element.
defsubr! { name = "libxml-parse-xml-region", gnu = "xml.c:297", effect = Reads,
fn libxml_parse_xml_region(cx; optional start: Value, end: Value, base_url: Value, discard_comments: Value) {
    use crate::xml_read::XmlNode;
    let text = region_text(cx, start, end, base_url)?;
    let Some(nodes) = std::str::from_utf8(&text).ok().and_then(crate::xml_read::parse_document) else { return Ok(Value::NIL) };
    let root = |cx: &mut Leaf<'_>| nodes.iter().find(|n| matches!(n, XmlNode::Element { .. })).map_or(Value::NIL, |n| xml_el(cx, n));
    let Some((last, earlier)) = nodes.split_last() else { return Ok(Value::NIL) };
    if discard_comments != Value::NIL || earlier.iter().all(|n| matches!(n, XmlNode::Other)) {
        return Ok(root(cx));
    }
    let mut items = vec![cx.intern("top"), Value::NIL];
    items.extend(earlier.iter().filter(|n| !matches!(n, XmlNode::Other)).map(|n| xml_el(cx, n)));
    items.push(xml_el(cx, last));
    Ok(cx.coding_list(&items))
}}

/// A node of `crate::xml_read': an element as xml.el and make_dom build it,
/// (NAME ATTRS . CHILDREN), names interned, text a string; a comment and
/// what else make_dom knows.
fn xml_el(cx: &mut Leaf<'_>, node: &crate::xml_read::XmlNode) -> Value {
    use crate::xml_read::XmlNode;
    let string = |cx: &mut Leaf<'_>, s: &str| cx.make_string(s.as_bytes().to_vec(), !s.is_ascii());
    match node {
        XmlNode::Text(text) | XmlNode::Cdata(text) => string(cx, text),
        XmlNode::Comment(text) => {
            let items = [cx.intern("comment"), Value::NIL, string(cx, text)];
            cx.coding_list(&items)
        }
        XmlNode::Other => Value::NIL,
        XmlNode::Element { name, attrs, children } => {
            let attrs: Vec<Value> = attrs.iter().map(|(k, v)| {
                let (k, v) = (cx.intern(k), string(cx, v));
                cx.cons(k, v)
            }).collect();
            let mut items = vec![cx.intern(name), cx.coding_list(&attrs)];
            items.extend(children.iter().map(|c| xml_el(cx, c)));
            cx.coding_list(&items)
        }
    }
}

/// The first 4 KiB of a region read first, then four times as much while
/// the element runs past it: an element is almost always shorter, and a
/// connection's buffer can hold many, so a read copies what it needs and
/// the reads of a buffer stay linear in it.
const XML_READ_WINDOW: i64 = 4 * 1024;

// The first complete element of the current buffer's START..END, as
// xml.el's `xml-parse-region' reads it (romacs/src/xml_read.rs): (ELEMENT .
// POSITION) with POSITION just after it, `incomplete' while the region
// ends inside it or holds no `<', or nil for what it leaves to xml.el.
// jabber-xml.el's `jabber-xml-parse-next-stanza' reads stanzas with it.
defsubr! { name = "romacs--xml-read-element", gnu = "romacs:xml", effect = Reads,
fn romacs_xml_read_element(cx, start: Value, end: Value) {
    let (Some(from), Some(to)) = (start.integer_value(), end.integer_value()) else {
        return Err(Fault::Wrong("integerp", if start.integer_value().is_none() { start } else { end }));
    };
    let mut window = XML_READ_WINDOW;
    let mut limit = to.min(from.saturating_add(window));
    loop {
        let bound = Value::integer(limit).map_err(Fault::Error)?;
        let span = cx.text_span(start, bound, false)?;
        let Ok(text) = std::str::from_utf8(&span.bytes) else { return Ok(Value::NIL) };
        if !span.multibyte && !text.is_ascii() {
            return Ok(Value::NIL);
        }
        match crate::xml_read::read_first_element(text) {
            crate::xml_read::Read::Element(node, after) => {
                let position = from + text[..after].chars().count() as i64;
                let element = xml_el(cx, &node);
                let position = Value::integer(position).map_err(Fault::Error)?;
                return Ok(cx.cons(element, position));
            }
            crate::xml_read::Read::Declined => return Ok(Value::NIL),
            crate::xml_read::Read::Incomplete if limit < to => {
                window = window.saturating_mul(4);
                limit = to.min(from.saturating_add(window));
            }
            crate::xml_read::Read::Incomplete => return Ok(cx.intern("incomplete")),
        }
    }
}}
