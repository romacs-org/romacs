//! The HTML parser behind xml.c's `libxml-parse-html-region': the document
//! tree libxml2 2.14's htmlReadMemory builds with HTML_PARSE_RECOVER and
//! HTML_PARSE_NOBLANKS, the options GNU parses with. Its tokenizer is
//! HTML5's (character references, comments, raw text); its tree is
//! libxml2's own, not HTML5's: implied html, head and body, the
//! start-tag auto-close table, end-tag priorities and its blank text
//! rule. The tables were read off the pinned GNU (crates/romacs-vm/tests
//! hold its answers). Nothing here calls Lisp.
use crate::html_entities::ENTITIES;

/// One node of the document.
#[derive(Debug, PartialEq)]
pub(crate) enum Node {
    Element { name: String, attrs: Vec<(String, String)>, children: Vec<Node> },
    Text(String),
    Comment(String),
}

/// What a start tag's newcomer closes when one of these is the current
/// element (htmlStartClose): NEW, then the OLD elements it closes.
const START_CLOSE: &[(&str, &[&str])] = &[
    ("a", &["a", "head"]),
    ("abbr", &["head"]), ("acronym", &["head"]), ("bdo", &["head"]), ("big", &["head"]), ("br", &["head"]),
    ("cite", &["head"]), ("code", &["head"]), ("dfn", &["head"]), ("em", &["head"]), ("font", &["head"]),
    ("img", &["head"]), ("kbd", &["head"]), ("map", &["head"]), ("q", &["head"]), ("samp", &["head"]),
    ("span", &["head"]), ("strong", &["head"]), ("sub", &["head"]), ("sup", &["head"]), ("var", &["head"]),
    ("b", &["head"]), ("i", &["head"]), ("s", &["head"]), ("small", &["head"]), ("strike", &["head"]),
    ("tt", &["head"]), ("u", &["head"]), ("iframe", &["head"]),
    ("address", &["p", "ul", "head"]),
    ("blockquote", &["p", "head"]),
    ("body", &["head", "style", "script", "title"]),
    ("caption", &["p"]),
    ("center", &["b", "font", "i", "p", "head"]),
    ("col", &["caption", "p"]),
    ("colgroup", &["caption", "colgroup", "p"]),
    ("dd", &["address", "dir", "dt", "listing", "menu", "p", "pre", "xmp", "head"]),
    ("dir", &["p", "head"]),
    ("div", &["p", "head"]),
    ("dl", &["address", "dir", "dt", "listing", "menu", "p", "pre", "xmp", "head"]),
    ("dt", &["address", "dd", "dir", "listing", "menu", "p", "pre", "xmp", "head"]),
    ("fieldset", &["a", "h1", "h2", "h3", "h4", "h5", "h6", "legend", "listing", "p", "pre", "xmp", "head"]),
    ("form", &["address", "dir", "dl", "form", "h1", "h2", "h3", "h4", "h5", "h6", "listing", "menu", "ol", "p", "pre", "ul", "xmp", "head"]),
    ("frameset", &["head", "style", "script", "title", "p"]),
    ("h1", &["p", "head"]), ("h2", &["p", "head"]), ("h3", &["p", "head"]),
    ("h4", &["p", "head"]), ("h5", &["p", "head"]), ("h6", &["p", "head"]),
    ("head", &["p"]),
    ("hr", &["p", "head"]),
    ("li", &["address", "dl", "h1", "h2", "h3", "h4", "h5", "h6", "li", "listing", "p", "pre", "xmp", "head"]),
    ("listing", &["p", "head"]),
    ("menu", &["p", "ul", "head"]),
    ("noscript", &["script"]),
    ("ol", &["p", "head"]),
    ("optgroup", &["option"]),
    ("option", &["option"]),
    ("p", &["b", "big", "h1", "h2", "h3", "h4", "h5", "h6", "i", "p", "s", "small", "strike", "tt", "u", "head"]),
    ("pre", &["p", "ul", "head"]),
    ("table", &["a", "h1", "h2", "h3", "h4", "h5", "h6", "listing", "p", "pre", "xmp", "head"]),
    ("tbody", &["caption", "colgroup", "p", "tbody", "td", "tfoot", "th", "thead", "tr"]),
    ("td", &["a", "b", "font", "i", "p", "span", "td", "th", "u"]),
    ("tfoot", &["caption", "colgroup", "p", "tbody", "td", "th", "thead", "tr"]),
    ("th", &["a", "b", "font", "i", "p", "span", "td", "th", "u"]),
    ("thead", &["caption", "colgroup"]),
    ("title", &["p"]),
    ("tr", &["caption", "colgroup", "p", "td", "th", "tr"]),
    ("ul", &["address", "dir", "listing", "menu", "p", "pre", "xmp", "head"]),
    ("xmp", &["p", "head"]),
];

/// Elements with no content (the element table's empty ones).
const VOID: &[&str] = &["area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
                        "isindex", "keygen", "link", "meta", "param", "source", "track", "wbr"];
/// Elements whose content is text up to their end tag.
const RAW: &[&str] = &["iframe", "noembed", "noframes", "script", "style", "xmp"];
/// The same, with character references decoded.
const RCDATA: &[&str] = &["textarea", "title"];
/// Elements a start tag at the top implies a head for (htmlCheckImplied).
const HEAD_CONTENT: &[&str] = &["base", "link", "meta", "script", "style", "title"];
/// Elements whose whitespace text libxml2 keeps (allowPCData).
const ALLOW_PCDATA: &[&str] = &["a", "abbr", "acronym", "address", "applet", "b", "bdo", "big", "blockquote", "body",
    "button", "caption", "center", "cite", "code", "dd", "del", "dfn", "div", "dt", "em", "font", "form", "h1", "h2",
    "h3", "h4", "h5", "h6", "i", "iframe", "ins", "kbd", "label", "legend", "li", "noframes", "noscript", "object", "p", "pre", "q",
    "s", "samp", "small", "span", "strike", "strong", "td", "th", "tt", "u", "var"];
/// Attributes that without a value take their name (htmlBooleanAttrs).
const BOOLEAN: &[&str] = &["checked", "compact", "declare", "defer", "disabled", "ismap", "multiple", "nohref",
                           "noresize", "noshade", "nowrap", "readonly", "selected"];

/// htmlGetEndPriority: an end tag closes past only elements of no higher
/// priority than its own.
fn end_priority(name: &str) -> u8 {
    match name {
        "div" => 150,
        "td" | "th" => 160,
        "tr" => 170,
        "thead" | "tbody" | "tfoot" => 180,
        "table" => 190,
        "head" | "body" => 200,
        "html" => 220,
        _ => 100,
    }
}

fn closes(new: &str, old: &str) -> bool {
    START_CLOSE.iter().any(|(n, olds)| *n == new && olds.contains(&old))
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// HTML5's replacements for numeric references to C1 controls.
fn windows_1252(code: u32) -> u32 {
    const TABLE: [u32; 32] = [
        0x20ac, 0x81, 0x201a, 0x192, 0x201e, 0x2026, 0x2020, 0x2021, 0x2c6, 0x2030, 0x160, 0x2039, 0x152, 0x8d, 0x17d, 0x8f,
        0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x2dc, 0x2122, 0x161, 0x203a, 0x153, 0x9d, 0x17e, 0x178,
    ];
    if (0x80..0xa0).contains(&code) { TABLE[(code - 0x80) as usize] } else { code }
}

/// The character reference at the start of S (just after its `&'): the
/// text it stands for and the bytes it used, or None for a literal `&'.
/// IN_ATTRIBUTE applies HTML5's rule that a legacy name without `;'
/// followed by `=' or an alphanumeric is not a reference there.
fn reference(s: &str, in_attribute: bool) -> Option<(String, usize)> {
    if let Some(rest) = s.strip_prefix('#') {
        let (hex, digits) = match rest.strip_prefix(['x', 'X']) {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let len = digits.bytes().take_while(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }).count();
        if len == 0 { return None; }
        let value = u32::from_str_radix(&digits[..len], if hex { 16 } else { 10 }).unwrap_or(u32::MAX);
        let used = 1 + usize::from(hex) + len + usize::from(digits[len..].starts_with(';'));
        let code = windows_1252(value);
        let text = match code {
            0 => "\u{fffd}".into(),
            _ => char::from_u32(code).map_or("\u{fffd}".into(), String::from),
        };
        return Some((text, used));
    }
    let longest = s.bytes().take(32).take_while(|b| b.is_ascii_alphanumeric() || *b == b';').count();
    for len in (1..=longest).rev() {
        let name = &s[..len];
        if let Ok(i) = ENTITIES.binary_search_by(|(n, _)| (*n).cmp(name)) {
            let after = s.as_bytes().get(len).copied();
            if in_attribute && !name.ends_with(';') && after.is_some_and(|b| b == b'=' || b.is_ascii_alphanumeric()) {
                return None;
            }
            return Some((ENTITIES[i].1.to_owned(), len));
        }
    }
    None
}

/// TEXT with its character references decoded.
fn decode(text: &str, in_attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        match reference(rest, in_attribute) {
            Some((decoded, used)) => { out.push_str(&decoded); rest = &rest[used..]; }
            None => out.push('&'),
        }
    }
    out.push_str(rest);
    out
}

/// An element under construction, in the builder's arena.
struct Open {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Item>,
}

enum Item {
    Element(usize),
    Text(String),
    Comment(String),
}

/// libxml2's HTML SAX tree builder.
#[derive(Default)]
struct Builder {
    arena: Vec<Open>,
    top: Vec<Item>,
    stack: Vec<usize>,
    /// ctxt->html: 3 once a head was opened, 10 once a body was.
    seen: u8,
    /// Tags discarded (a second html or body, a misplaced head) whose end
    /// tags are ignored in turn.
    depth: usize,
    /// A DOCTYPE naming HTML 4.01 or 4 strict makes whitespace in body blank.
    html4: bool,
    /// An element would have nested deeper than MAX_DEPTH: libxml2 stops
    /// there, and the rest of the input is not parsed.
    halted: bool,
}

/// libxml2's limit on open elements (xmlParserMaxDepth without HUGE).
const MAX_DEPTH: usize = 256;

impl Builder {
    fn current(&self) -> Option<&str> {
        self.stack.last().map(|&i| self.arena[i].name.as_str())
    }
    fn children(&mut self) -> &mut Vec<Item> {
        match self.stack.last() {
            Some(&i) => &mut self.arena[i].children,
            None => &mut self.top,
        }
    }
    fn push(&mut self, name: &str, attrs: Vec<(String, String)>) {
        if self.stack.len() >= MAX_DEPTH {
            self.halted = true;
            return;
        }
        let id = self.arena.len();
        self.arena.push(Open { name: name.to_owned(), attrs, children: Vec::new() });
        self.children().push(Item::Element(id));
        self.stack.push(id);
        if name == "head" && self.seen < 3 { self.seen = 3; }
        if name == "body" && self.seen < 10 { self.seen = 10; }
    }
    fn auto_close(&mut self, new: &str) {
        while self.current().is_some_and(|old| closes(new, old)) {
            self.stack.pop();
        }
    }
    fn implied(&mut self, new: &str) {
        if new == "html" { return; }
        if self.stack.is_empty() { self.push("html", Vec::new()); }
        if new == "body" || new == "head" { return; }
        if self.stack.len() <= 1 && HEAD_CONTENT.contains(&new) {
            if self.seen < 3 { self.push("head", Vec::new()); }
        } else if !matches!(new, "noframes" | "frame" | "frameset") {
            if self.seen >= 10 { return; }
            if self.stack.iter().any(|&i| matches!(self.arena[i].name.as_str(), "body" | "head")) { return; }
            self.push("body", Vec::new());
        }
    }
    /// htmlParseStartTag; true when the element is open after it.
    fn start(&mut self, name: &str, attrs: Vec<(String, String)>) -> bool {
        self.auto_close(name);
        self.implied(name);
        let discard = (name == "html" && !self.stack.is_empty())
            || (name == "head" && self.stack.len() != 1)
            || (name == "body" && self.stack.iter().any(|&i| self.arena[i].name == "body"));
        if discard {
            self.depth += 1;
            if name == "html" && let Some(&root) = self.stack.first() {
                for (k, v) in attrs {
                    if !self.arena[root].attrs.iter().any(|(n, _)| *n == k) { self.arena[root].attrs.push((k, v)); }
                }
            }
            return false;
        }
        self.push(name, attrs);
        !self.halted
    }
    fn end(&mut self, name: &str) {
        if self.depth > 0 && matches!(name, "html" | "body" | "head") {
            self.depth -= 1;
            return;
        }
        let priority = end_priority(name);
        let Some(at) = self.stack.iter().rposition(|&i| self.arena[i].name == name) else { return };
        if self.stack[at + 1..].iter().any(|&i| end_priority(&self.arena[i].name) > priority) { return; }
        self.stack.truncate(at);
    }
    /// Whitespace TEXT in the current element is blank (areBlanks) when
    /// AT_END or when what precedes it in the element lets libxml2 drop it.
    fn blank(&self, at_end: bool) -> bool {
        if at_end { return true; }
        let Some(&id) = self.stack.last() else { return true };
        let open = &self.arena[id];
        if matches!(open.name.as_str(), "html" | "head") { return true; }
        if open.name == "body" && self.html4 { return true; }
        let last = open.children.iter().rev().find(|c| !matches!(c, Item::Comment(_)));
        match last {
            None => !ALLOW_PCDATA.contains(&open.name.as_str()),
            Some(Item::Text(_)) => false,
            Some(Item::Element(e)) => !ALLOW_PCDATA.contains(&self.arena[*e].name.as_str()),
            Some(Item::Comment(_)) => unreachable!(),
        }
    }
    fn append_text(&mut self, text: &str) {
        if text.is_empty() { return; }
        match self.children().last_mut() {
            Some(Item::Text(t)) => t.push_str(text),
            _ => self.children().push(Item::Text(text.to_owned())),
        }
    }
    /// Character data between markup; AT_END when nothing follows it.
    fn text(&mut self, text: &str, at_end: bool) {
        let mut text = text;
        if matches!(self.current(), None | Some("html" | "head")) {
            text = text.trim_start_matches(is_space);
            if text.is_empty() { return; }
            self.auto_close("p");
            self.implied("p");
        } else if text.chars().all(is_space) && self.blank(at_end) {
            return;
        }
        self.append_text(text);
    }
    /// The content of a raw text element, never blank.
    fn raw_text(&mut self, text: &str) {
        self.append_text(text);
    }
    fn comment(&mut self, text: String) {
        self.children().push(Item::Comment(text));
    }
    fn finish(mut self) -> Vec<Node> {
        let top = std::mem::take(&mut self.top);
        top.into_iter().map(|item| self.node(item)).collect()
    }
    fn node(&mut self, item: Item) -> Node {
        match item {
            Item::Text(t) => Node::Text(t),
            Item::Comment(c) => Node::Comment(c),
            Item::Element(id) => {
                let children = std::mem::take(&mut self.arena[id].children);
                let children = children.into_iter().map(|c| self.node(c)).collect();
                Node::Element { name: std::mem::take(&mut self.arena[id].name), attrs: std::mem::take(&mut self.arena[id].attrs), children }
            }
        }
    }
}

/// A start tag's attributes from S (after its name): (ATTRS, SELF_CLOSING,
/// bytes used through `>'), or None when the input ends inside the tag.
fn attributes(s: &str) -> Option<(Vec<(String, String)>, bool, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while i < b.len() && is_space(b[i] as char) { i += 1; }
        match b.get(i)? {
            b'>' => return Some((attrs, self_closing, i + 1)),
            b'/' => {
                i += 1;
                self_closing = b.get(i) == Some(&b'>');
                continue;
            }
            _ => {}
        }
        self_closing = false;
        let start = i;
        i += 1;
        while i < b.len() && !is_space(b[i] as char) && !matches!(b[i], b'/' | b'>' | b'=') { i += 1; }
        let name = s[start..i].to_ascii_lowercase();
        while i < b.len() && is_space(b[i] as char) { i += 1; }
        let value = if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && is_space(b[i] as char) { i += 1; }
            match b.get(i)? {
                q @ (b'"' | b'\'') => {
                    let end = s[i + 1..].find(*q as char)? + i + 1;
                    let v = decode(&s[i + 1..end], true);
                    i = end + 1;
                    Some(v)
                }
                _ => {
                    let start = i;
                    while i < b.len() && !is_space(b[i] as char) && b[i] != b'>' { i += 1; }
                    Some(decode(&s[start..i], true))
                }
            }
        } else { None };
        let value = value.or_else(|| BOOLEAN.contains(&name.as_str()).then(|| name.clone()));
        if let Some(value) = value && !attrs.iter().any(|(n, _)| *n == name) {
            attrs.push((name, value));
        } else if !attrs.iter().any(|(n, _)| *n == name) {
            // A valueless attribute holds a place: a later one of its
            // name is still a duplicate.
            attrs.push((name, String::from("\0")));
        }
    }
}

/// The comment starting after `<!--' in S: its text and the bytes used.
fn comment(s: &str) -> (String, usize) {
    if s.starts_with('>') { return (String::new(), 1); }
    if s.starts_with("->") { return (String::new(), 2); }
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with("-->") { return (s[..i].to_owned(), i + 3); }
        if s[i..].starts_with("--!>") { return (s[..i].to_owned(), i + 4); }
        i += s[i..].chars().next().map_or(1, char::len_utf8);
    }
    (s.to_owned(), s.len())
}

/// Where the raw text of NAME that S starts ends: (content end, after the end tag).
fn raw_end(s: &str, name: &str) -> (usize, usize) {
    let lower = s.to_ascii_lowercase();
    let close = format!("</{name}");
    let mut from = 0;
    while let Some(at) = lower[from..].find(&close).map(|p| p + from) {
        let after = at + close.len();
        match lower.as_bytes().get(after) {
            Some(c) if is_space(*c as char) || *c == b'/' || *c == b'>' => {
                let end = s[after..].find('>').map_or(s.len(), |p| after + p + 1);
                return (at, end);
            }
            None => return (at, s.len()),
            _ => from = after,
        }
    }
    (s.len(), s.len())
}

/// The document's top-level nodes: comments and root elements, the DOCTYPE
/// left out (GNU's make_dom gives it nothing).
pub(crate) fn parse(input: &str) -> Vec<Node> {
    let input = input.replace("\r\n", "\n").replace('\r', "\n").replace('\0', "");
    let s = input.as_str();
    let mut b = Builder::default();
    let mut i = 0;
    while i < s.len() && !b.halted {
        let rest = &s[i..];
        let Some(lt) = rest.find('<') else {
            b.text(&decode(rest, false), true);
            break;
        };
        if lt > 0 {
            let text = &rest[..lt];
            b.text(&decode(text, false), false);
            i += lt;
            continue;
        }
        let after = &rest[1..];
        if let Some(body) = after.strip_prefix("!--") {
            let (text, used) = comment(body);
            b.comment(text);
            i += 4 + used;
        } else if after.as_bytes().get(..8).is_some_and(|b| b.eq_ignore_ascii_case(b"!doctype")) {
            let end = after.find('>').map_or(after.len(), |p| p + 1);
            let doctype = after[..end].to_ascii_lowercase();
            b.html4 = doctype.contains("\"-//w3c//dtd html 4.01//en\"") || doctype.contains("\"-//w3c//dtd html 4//en\"");
            i += 1 + end;
        } else if let Some(body) = after.strip_prefix('!') {
            let end = body.find('>').map_or(body.len(), |p| p);
            b.comment(body[..end].to_owned());
            i += 2 + (end + 1).min(body.len());
        } else if after.starts_with('?') {
            let end = after.find('>').map_or(after.len(), |p| p);
            b.comment(after[..end].to_owned());
            i += 1 + (end + 1).min(after.len());
        } else if let Some(body) = after.strip_prefix('/') {
            if body.starts_with('>') {
                i += 3;
            } else if body.starts_with(|c: char| c.is_ascii_alphabetic()) {
                let len = body.find(|c: char| is_space(c) || c == '/' || c == '>').unwrap_or(body.len());
                let end = body.find('>').map_or(body.len(), |p| p + 1);
                if end == body.len() && !body.ends_with('>') { break; }
                b.end(&body[..len].to_ascii_lowercase());
                i += 2 + end;
            } else {
                let end = body.find('>').map_or(body.len(), |p| p);
                b.comment(body[..end].to_owned());
                i += 2 + (end + 1).min(body.len());
            }
        } else if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let len = after.find(|c: char| is_space(c) || c == '/' || c == '>').unwrap_or(after.len());
            let name = after[..len].to_ascii_lowercase();
            let Some((attrs, self_closing, used)) = attributes(&after[len..]) else { break };
            let attrs: Vec<_> = attrs.into_iter().filter(|(_, v)| v != "\0").collect();
            i += 1 + len + used;
            let open = b.start(&name, attrs);
            if !open { continue; }
            if VOID.contains(&name.as_str()) || self_closing {
                b.stack.pop();
            } else if name == "plaintext" {
                b.raw_text(&s[i..]);
                break;
            } else if RAW.contains(&name.as_str()) || RCDATA.contains(&name.as_str()) {
                let (content, end) = raw_end(&s[i..], &name);
                let text = &s[i..i + content];
                if RCDATA.contains(&name.as_str()) { b.raw_text(&decode(text, false)); } else { b.raw_text(text); }
                b.stack.pop();
                i += end;
            }
        } else {
            b.text("<", false);
            i += 1;
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(name: &str, children: Vec<Node>) -> Node {
        Node::Element { name: name.into(), attrs: vec![], children }
    }

    #[test]
    fn references_decode_as_html5() {
        assert_eq!(decode("&amp; &copy &copyx &notit; &bogus; &#x263A; &#128; &#0;", false), "& © ©x ¬it; &bogus; ☺ € \u{fffd}");
        assert_eq!(decode("a?b=1&amp;c=2&copy=3&copy;", true), "a?b=1&c=2&copy=3©");
    }

    #[test]
    fn implied_structure_and_auto_close() {
        assert_eq!(parse("<p>a<p>b"), vec![element("html", vec![element("body", vec![
            element("p", vec![Node::Text("a".into())]), element("p", vec![Node::Text("b".into())])])])]);
        assert_eq!(parse("<title>t</title>"), vec![element("html", vec![element("head", vec![element("title", vec![Node::Text("t".into())])])])]);
    }
}
