//! An XML element read as xml.el's `xml-parse-region' reads it (no DTD, no
//! namespace expansion), for XMPP stanzas: jabber-xml.el's
//! `jabber-xml-parse-next-stanza' reads the first complete element of its
//! connection's buffer with it, and xml.el reads whatever it declines.
//!
//! What xml.el builds, and so what this builds: an element is (NAME ATTRS
//! . CHILDREN), NAME and each attribute name an interned symbol, ATTRS in
//! document order with entity and character references replaced, CHILDREN
//! the sub-elements and every run of text between them, whitespace too,
//! references replaced and then CR LF and lone CR made LF.  A CDATA section
//! is a child string of its own, with the text that follows it up to the
//! next tag.  Declined, so that xml.el reads it (and signals what it
//! signals): comments, processing instructions, DOCTYPE, an entity other
//! than the five predefined ones, a character reference that names no
//! character, a name outside ASCII, attributes not parted by space, and
//! anything not well-formed.

/// A node of the element read.
#[derive(Debug, PartialEq)]
pub(crate) enum XmlNode {
    Element { name: String, attrs: Vec<(String, String)>, children: Vec<XmlNode> },
    Text(String),
}

/// What reading the first element of a text found.
#[derive(Debug, PartialEq)]
pub(crate) enum Read {
    /// No complete element yet: the text ends inside it, or holds no `<`.
    Incomplete,
    /// Something this reader leaves to xml.el.
    Declined,
    /// The element, and the byte offset just after it.
    Element(XmlNode, usize),
}

/// Why a read stopped short of an element.
enum Stop {
    Incomplete,
    Declined,
}

type Step<T> = Result<T, Stop>;

/// The first element of TEXT, skipping what precedes its `<` as xml.el's
/// buffer loop does.
pub(crate) fn read_first_element(text: &str) -> Read {
    let bytes = text.as_bytes();
    let Some(open) = bytes.iter().position(|&b| b == b'<') else { return Read::Incomplete };
    let mut reader = Reader { text, bytes, at: open, depth: 0 };
    match reader.element() {
        Ok(node) => Read::Element(node, reader.at),
        Err(Stop::Incomplete) => Read::Incomplete,
        Err(Stop::Declined) => Read::Declined,
    }
}

struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

/// Elements nested deeper than this are xml.el's (which reaches its own
/// limit), so a hostile stanza cannot exhaust the native stack.
const MAX_DEPTH: usize = 256;

fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b':' || b == b'_'
}

fn name_char(b: u8) -> bool {
    name_start(b) || b.is_ascii_digit() || b == b'.' || b == b'-'
}

impl Reader<'_> {
    fn peek(&self) -> Step<u8> {
        self.bytes.get(self.at).copied().ok_or(Stop::Incomplete)
    }

    fn starts(&self, prefix: &str) -> bool {
        self.bytes[self.at..].starts_with(prefix.as_bytes())
    }

    /// Whether the rest of the text could still begin with PREFIX: it is a
    /// prefix of PREFIX, so more input decides.
    fn could_start(&self, prefix: &str) -> bool {
        prefix.as_bytes().starts_with(&self.bytes[self.at..])
    }

    fn skip_space(&mut self) {
        while self.bytes.get(self.at).is_some_and(|&b| space(b)) {
            self.at += 1;
        }
    }

    /// A name at point; a name byte beyond ASCII is declined.
    fn name(&mut self) -> Step<String> {
        let start = self.at;
        match self.peek()? {
            b if name_start(b) => self.at += 1,
            b if b >= 0x80 => return Err(Stop::Declined),
            _ => return Err(Stop::Declined),
        }
        loop {
            match self.bytes.get(self.at) {
                Some(&b) if name_char(b) => self.at += 1,
                Some(&b) if b >= 0x80 => return Err(Stop::Declined),
                Some(_) => return Ok(self.text[start..self.at].to_string()),
                None => return Err(Stop::Incomplete),
            }
        }
    }

    /// The element whose `<` is at point.
    fn element(&mut self) -> Step<XmlNode> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Stop::Declined);
        }
        let node = self.element_body();
        self.depth -= 1;
        node
    }

    fn element_body(&mut self) -> Step<XmlNode> {
        // `<` then a name: anything else at a tag (`<?', `<!', `</') is
        // xml.el's, or more input.
        self.at += 1;
        let next = self.peek()?;
        if !name_start(next) {
            return Err(Stop::Declined);
        }
        let name = self.name()?;
        let mut attrs: Vec<(String, String)> = Vec::new();
        // Each name once (xml.el refuses a repeat), without rescanning.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        loop {
            self.skip_space();
            match self.peek()? {
                b'/' => {
                    self.at += 1;
                    return match self.peek()? {
                        b'>' => {
                            self.at += 1;
                            Ok(XmlNode::Element { name, attrs, children: Vec::new() })
                        }
                        _ => Err(Stop::Declined),
                    };
                }
                b'>' => {
                    self.at += 1;
                    let children = self.children(&name)?;
                    return Ok(XmlNode::Element { name, attrs, children });
                }
                _ => {
                    let attr = self.name()?;
                    self.skip_space();
                    if self.peek()? != b'=' {
                        return Err(Stop::Declined);
                    }
                    self.at += 1;
                    self.skip_space();
                    let quote = self.peek()?;
                    if quote != b'"' && quote != b'\'' {
                        return Err(Stop::Declined);
                    }
                    self.at += 1;
                    let start = self.at;
                    let len = self.bytes[start..].iter().position(|&b| b == quote).ok_or(Stop::Incomplete)?;
                    self.at = start + len + 1;
                    if !seen.insert(attr.clone()) {
                        return Err(Stop::Declined);
                    }
                    let value = expand(&self.text[start..start + len])?;
                    attrs.push((attr, value));
                    // Space, or the tag's end, must follow: xml.el would read
                    // `a='1'b='2'', jabber-xml.el's stanza scan does not.
                    match self.peek()? {
                        b if space(b) => {}
                        b'/' | b'>' => {}
                        _ => return Err(Stop::Declined),
                    }
                }
            }
        }
    }

    /// The children of the element NAME, through its end tag.
    fn children(&mut self, name: &str) -> Step<Vec<XmlNode>> {
        let mut children = Vec::new();
        loop {
            if self.peek()? != b'<' {
                let text = self.text_run()?;
                children.push(XmlNode::Text(text));
                continue;
            }
            if self.starts("</") {
                self.at += 2;
                let end = self.name()?;
                self.skip_space();
                if self.peek()? != b'>' {
                    return Err(Stop::Declined);
                }
                self.at += 1;
                return if end == name { Ok(children) } else { Err(Stop::Declined) };
            }
            if self.starts("<![CDATA[") {
                let start = self.at + "<![CDATA[".len();
                let len = self.text[start..].find("]]>").ok_or(Stop::Incomplete)?;
                self.at = start + len + 3;
                let mut text = self.text[start..start + len].to_string();
                if self.peek()? != b'<' {
                    text.push_str(&self.text_run()?);
                }
                children.push(XmlNode::Text(text));
                continue;
            }
            if self.could_start("<![CDATA[") || self.could_start("</") {
                return Err(Stop::Incomplete);
            }
            if self.starts("<!") || self.starts("<?") {
                return Err(Stop::Declined);
            }
            children.push(self.element()?);
        }
    }

    /// The text from point to the next `<`, references replaced, then line
    /// ends made LF (xml.el's `xml-parse-string').
    fn text_run(&mut self) -> Step<String> {
        let start = self.at;
        let len = self.bytes[start..].iter().position(|&b| b == b'<').ok_or(Stop::Incomplete)?;
        self.at = start + len;
        let expanded = expand(&self.text[start..start + len])?;
        Ok(if expanded.contains('\r') { expanded.replace("\r\n", "\n").replace('\r', "\n") } else { expanded })
    }
}

/// TEXT with its entity and character references replaced: the five
/// predefined entities and numeric references naming a character.
fn expand(text: &str) -> Step<String> {
    if !text.contains('&') {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = after.find(';').ok_or(Stop::Declined)?;
        let reference = &after[..semi];
        let replacement = match reference {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "apos" => '\'',
            "quot" => '"',
            _ => {
                let (digits, radix) = match reference.strip_prefix('#') {
                    Some(hex) if hex.starts_with('x') => (&hex[1..], 16),
                    Some(dec) => (dec, 10),
                    None => return Err(Stop::Declined),
                };
                let valid = !digits.is_empty()
                    && digits.bytes().all(|b| if radix == 16 { b.is_ascii_hexdigit() } else { b.is_ascii_digit() });
                if !valid {
                    return Err(Stop::Declined);
                }
                let code = u32::from_str_radix(digits, radix).map_err(|_| Stop::Declined)?;
                match char::from_u32(code) {
                    Some(c) if code != 0 => c,
                    _ => return Err(Stop::Declined),
                }
            }
        };
        out.push(replacement);
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(name: &str, attrs: &[(&str, &str)], children: Vec<XmlNode>) -> XmlNode {
        XmlNode::Element {
            name: name.to_string(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            children,
        }
    }

    fn text(s: &str) -> XmlNode {
        XmlNode::Text(s.to_string())
    }

    #[test]
    fn a_stanza_reads_as_xml_el_builds_it() {
        let stanza = "<message from='a@b/c' type=\"chat\"><body>Hi &amp; bye&#33;</body>\r\n<x xmlns='urn:x'/></message>";
        assert_eq!(
            read_first_element(stanza),
            Read::Element(
                el("message", &[("from", "a@b/c"), ("type", "chat")],
                   vec![el("body", &[], vec![text("Hi & bye!")]), text("\n"), el("x", &[("xmlns", "urn:x")], vec![])]),
                stanza.len()
            )
        );
    }

    #[test]
    fn every_prefix_of_a_stanza_is_incomplete() {
        let stanza = "<iq id='1'><query xmlns='q'><item a='&lt;'>t<![CDATA[x<y]]>z</item><item/></query></iq>";
        for end in 1..stanza.len() {
            assert_eq!(read_first_element(&stanza[..end]), Read::Incomplete, "prefix {end}");
        }
        assert!(matches!(read_first_element(stanza), Read::Element(_, n) if n == stanza.len()));
    }

    #[test]
    fn cdata_is_a_child_of_its_own_with_the_text_after_it() {
        assert_eq!(
            read_first_element("<a>one<![CDATA[<two>]]>three<b/></a>"),
            Read::Element(el("a", &[], vec![text("one"), text("<two>three"), el("b", &[], vec![])]), 36)
        );
    }

    #[test]
    fn what_xml_el_reads_otherwise_is_declined() {
        for s in ["<a><!-- c --></a>", "<?xml version='1.0'?><a/>", "<a>&nbsp;</a>", "<a>&#0;</a>", "<a x='1'y='2'/>",
                  "<a x=1/>", "<a></b>", "<a x='1' x='2'/>", "<é/>", "<a>&amp</a>", "</a>", "<!DOCTYPE a><a/>"] {
            assert_eq!(read_first_element(s), Read::Declined, "{s}");
        }
    }

    #[test]
    fn nesting_past_the_limit_is_declined() {
        let deep = format!("{}{}", "<a>".repeat(MAX_DEPTH + 1), "</a>".repeat(MAX_DEPTH + 1));
        assert_eq!(read_first_element(&deep), Read::Declined);
        let fine = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        assert!(matches!(read_first_element(&fine), Read::Element(_, n) if n == fine.len()));
    }

    #[test]
    fn text_before_the_element_is_skipped_and_after_it_left() {
        assert_eq!(read_first_element("  <a/><b/>"), Read::Element(el("a", &[], vec![]), 6));
        assert_eq!(read_first_element("no tag"), Read::Incomplete);
    }
}
