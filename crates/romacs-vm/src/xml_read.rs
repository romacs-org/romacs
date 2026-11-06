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
//!
//! The same reader, given a [`Doc`], reads a whole document as libxml2 does
//! for `libxml-parse-xml-region' (xml.c: no RECOVER, NOBLANKS, entities
//! left unsubstituted): see [`parse_document`].

use std::collections::{HashMap, HashSet};

/// A node of the element read.
#[derive(Debug, PartialEq)]
pub(crate) enum XmlNode {
    Element { name: String, attrs: Vec<(String, String)>, children: Vec<XmlNode> },
    Text(String),
    /// A document's CDATA section, its comment, and what xml.c's make_dom
    /// answers nil for: a processing instruction, a DOCTYPE or an entity
    /// reference.
    Cdata(String),
    Comment(String),
    Other,
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
    let mut reader = Reader { text, bytes, at: open, depth: 0, doc: None, top: true };
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
    /// What a document read keeps; None reads as xml.el does.
    doc: Option<Doc>,
    /// Whether this is the document itself, not an entity's text.
    top: bool,
}

/// Elements nested deeper than this are xml.el's (which reaches its own
/// limit), so a hostile stanza cannot exhaust the native stack; they end a
/// document read, whose limit is libxml2's own.
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

    /// A name at point; outside a document, a name byte beyond ASCII is
    /// declined.
    fn name(&mut self) -> Step<String> {
        let start = self.at;
        if self.doc.is_some() {
            let mut chars = self.text[start..].chars();
            if !chars.next().is_some_and(xml_name_start) {
                return Err(Stop::Declined);
            }
            let rest = chars.as_str();
            let end = rest.find(|c| !xml_name_char(c)).unwrap_or(rest.len());
            self.at = self.text.len() - rest.len() + end;
            return Ok(self.text[start..self.at].to_string());
        }
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
        if let Some(doc) = &mut self.doc {
            doc.enter_element();
        }
        let node = self.element_body();
        if let Some(doc) = &mut self.doc {
            doc.leave_element();
        }
        self.depth -= 1;
        node
    }

    fn element_body(&mut self) -> Step<XmlNode> {
        // `<` then a name: anything else at a tag (`<?', `<!', `</') is
        // xml.el's, or more input.
        self.at += 1;
        let next = self.peek()?;
        if self.doc.is_none() && !name_start(next) {
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
                            let (name, attrs) = self.resolve(name, attrs);
                            Ok(XmlNode::Element { name, attrs, children: Vec::new() })
                        }
                        _ => Err(Stop::Declined),
                    };
                }
                b'>' => {
                    self.at += 1;
                    let raw = name.clone();
                    let (name, attrs) = self.resolve(name, attrs);
                    let children = if self.doc.is_some() { self.content(Some((&raw, &name)))? } else { self.children(&raw)? };
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
                    if !seen.insert(attr.clone()) {
                        return Err(Stop::Declined);
                    }
                    let value = if self.doc.is_some() {
                        self.attribute_value(&name, &attr, start + len)?
                    } else {
                        expand(&self.text[start..start + len])?
                    };
                    self.at = start + len + 1;
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
                self.end_tag(name)?;
                return Ok(children);
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

    /// The end tag at point, which closes the element NAME.
    fn end_tag(&mut self, name: &str) -> Step<()> {
        self.at += 2;
        let end = self.name()?;
        self.skip_space();
        if self.peek()? != b'>' {
            return Err(Stop::Declined);
        }
        self.at += 1;
        if end == name { Ok(()) } else { Err(Stop::Declined) }
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

/// libxml2's entities nested deeper than this end the document.
const MAX_ENTITY_DEPTH: usize = 19;
/// Content models nested deeper than this end it too.
const MAX_MODEL_DEPTH: usize = 256;
/// parser.c's XML_ENT_FIXED_COST, XML_PARSER_ALLOWED_EXPANSION and
/// XML_MAX_AMPLIFICATION_DEFAULT.
const ENTITY_FIXED_COST: u64 = 20;
const ALLOWED_EXPANSION: u64 = 1_000_000;
const MAX_AMPLIFICATION: u64 = 5;

/// XML 1.0's Char.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// XML 1.0 fifth edition's NameStartChar.
fn xml_name_start(c: char) -> bool {
    matches!(c, ':' | '_' | 'a'..='z' | 'A'..='Z' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
             | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}'
             | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}'
             | '\u{10000}'..='\u{EFFFF}')
}

/// XML 1.0 fifth edition's NameChar.
fn xml_name_char(c: char) -> bool {
    xml_name_start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

fn predefined(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => return None,
    })
}

/// NAME's prefix and local part when it has the form PREFIX:LOCAL, with no
/// second colon; libxml2 keeps any other name whole.
fn qname(name: &str) -> Option<(&str, &str)> {
    let (prefix, local) = name.split_once(':')?;
    let start = local.chars().next()?;
    (!prefix.is_empty() && xml_name_start(start) && !local.contains(':')).then_some((prefix, local))
}

/// A general entity of the internal subset.
#[derive(Clone)]
enum Entity {
    /// Its replacement text: character references replaced, others kept.
    Internal(String),
    /// SYSTEM or PUBLIC, never read (NONET, and no NOENT).
    External,
    Unparsed,
}

/// xml:space while an element is read, libxml2's `ctxt->space'.
#[derive(Clone, Copy, PartialEq)]
enum Space {
    /// The default: blank runs are judged.
    Unset,
    /// The element has kept text, so its blank runs are all kept.
    Text,
    Preserve,
    Default,
}

/// What a document read keeps beyond one element.
#[derive(Default)]
struct Doc {
    entities: HashMap<String, Entity>,
    parameters: HashMap<String, Option<String>>,
    /// Whether an element's declared content is mixed (xmlIsMixedElement).
    mixed: HashMap<String, bool>,
    /// ELEMENT\0ATTRIBUTE names declared with a type other than CDATA.
    tokenized: HashSet<String>,
    /// An undefined entity is then no error: the external subset or a
    /// parameter entity may define it.
    external: bool,
    standalone: bool,
    /// Entities proven well-formed, in content and in attributes, and those
    /// being checked or expanded, never more than MAX_ENTITY_DEPTH.
    content_ok: HashSet<String>,
    attribute_ok: HashSet<String>,
    checking: Vec<String>,
    /// xmlParserEntityCheck's account: what each checked entity expands to,
    /// what the references so far came to, and the input consumed so far.
    expanded: HashMap<String, u64>,
    cost: u64,
    consumed: u64,
    space: Vec<Space>,
    /// The number of open declarations of each namespace prefix, and the
    /// prefixes each open element declared.
    prefixes: HashMap<String, usize>,
    declared: Vec<Vec<String>>,
}

impl Doc {
    fn enter_element(&mut self) {
        let inherited = match self.space.last() {
            None | Some(Space::Text) => Space::Unset,
            Some(&space) => space,
        };
        self.space.push(inherited);
        self.declared.push(Vec::new());
    }

    fn leave_element(&mut self) {
        self.space.pop();
        for prefix in self.declared.pop().unwrap_or_default() {
            if let Some(count) = self.prefixes.get_mut(&prefix) {
                *count -= 1;
            }
        }
    }

    fn bound(&self, prefix: &str) -> bool {
        prefix == "xml" || self.prefixes.get(prefix).is_some_and(|&count| count > 0)
    }

    /// Charge one reference to text of SIZE bytes to the amplification
    /// account; a document that outgrows it ends, as it does in libxml2.
    fn charge(&mut self, size: u64) -> Step<()> {
        self.cost = self.cost.saturating_add(size + ENTITY_FIXED_COST);
        if self.cost > ALLOWED_EXPANSION && self.cost / MAX_AMPLIFICATION > self.consumed { Err(Stop::Declined) } else { Ok(()) }
    }

    /// An undefined entity is fatal unless something unread may define it.
    fn undefined_is_fatal(&self) -> bool {
        self.standalone || !self.external
    }
}

/// A reference at point: a character, or an entity's name.
enum Reference {
    Char(char),
    Named(String),
}

/// The nodes of the document TEXT at top level, as libxml2 reads it with
/// xml.c's options; None when it is not well-formed.
pub(crate) fn parse_document(text: &str) -> Option<Vec<XmlNode>> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if !text.chars().all(xml_char) {
        return None;
    }
    let mut reader = Reader { text, bytes: text.as_bytes(), at: 0, depth: 0, doc: Some(Doc::default()), top: true };
    reader.document().ok()
}

impl Reader<'_> {
    fn doc(&mut self) -> &mut Doc {
        self.doc.as_mut().expect("a document read")
    }

    fn at_end(&self) -> bool {
        self.at >= self.bytes.len()
    }

    /// PREFIX at point, passed; a Stop otherwise.
    fn expect(&mut self, prefix: &str) -> Step<()> {
        if !self.starts(prefix) {
            return Err(Stop::Declined);
        }
        self.at += prefix.len();
        Ok(())
    }

    /// Space at point, which must be there.
    fn required_space(&mut self) -> Step<()> {
        let start = self.at;
        self.skip_space();
        if self.at == start { Err(Stop::Declined) } else { Ok(()) }
    }

    /// F run over TEXT, with this read's document.
    fn nested<T>(&mut self, text: &str, f: impl FnOnce(&mut Reader<'_>) -> Step<T>) -> Step<T> {
        self.note_consumed();
        let mut sub = Reader { text, bytes: text.as_bytes(), at: 0, depth: self.depth, doc: self.doc.take(), top: false };
        let result = f(&mut sub);
        self.doc = sub.doc.take();
        result
    }

    fn document(&mut self) -> Step<Vec<XmlNode>> {
        if self.starts("<?xml") && self.bytes.get(5).copied().is_some_and(space) {
            self.xml_declaration()?;
        }
        let (mut nodes, mut doctype, mut root) = (Vec::new(), false, false);
        loop {
            self.skip_space();
            if self.at_end() {
                return if root { Ok(nodes) } else { Err(Stop::Declined) };
            }
            if self.starts("<!--") {
                nodes.push(self.comment()?);
            } else if self.starts("<?") {
                nodes.push(self.processing_instruction()?);
            } else if self.starts("<!DOCTYPE") && !doctype && !root {
                self.doctype()?;
                doctype = true;
                nodes.push(XmlNode::Other);
            } else if self.starts("<") && !self.starts("<!") && !root {
                nodes.push(self.element()?);
                root = true;
            } else {
                return Err(Stop::Declined);
            }
        }
    }

    /// `=' and a quoted value made of what ACCEPT takes.
    fn declared_value(&mut self, accept: impl Fn(u8) -> bool) -> Step<String> {
        self.skip_space();
        self.expect("=")?;
        self.skip_space();
        let quote = self.peek()?;
        if quote != b'"' && quote != b'\'' {
            return Err(Stop::Declined);
        }
        self.at += 1;
        let start = self.at;
        while self.peek()? != quote {
            if !accept(self.peek()?) {
                return Err(Stop::Declined);
            }
            self.at += 1;
        }
        self.at += 1;
        Ok(self.text[start..self.at - 1].to_string())
    }

    fn xml_declaration(&mut self) -> Step<()> {
        self.at += 5;
        self.skip_space();
        self.expect("version")?;
        let version = self.declared_value(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))?;
        if !version.strip_prefix("1.").is_some_and(|minor| minor.bytes().all(|b| b.is_ascii_digit())) {
            return Err(Stop::Declined);
        }
        let mut spaced = self.at;
        self.skip_space();
        spaced = self.at - spaced;
        if self.starts("encoding") {
            if spaced == 0 {
                return Err(Stop::Declined);
            }
            self.at += "encoding".len();
            let name = self.declared_value(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))?;
            if !name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) {
                return Err(Stop::Declined);
            }
            spaced = self.at;
            self.skip_space();
            spaced = self.at - spaced;
        }
        if self.starts("standalone") {
            if spaced == 0 {
                return Err(Stop::Declined);
            }
            self.at += "standalone".len();
            let value = self.declared_value(|b| b.is_ascii_lowercase())?;
            match value.as_str() {
                "yes" => self.doc().standalone = true,
                "no" => {}
                _ => return Err(Stop::Declined),
            }
            self.skip_space();
        }
        self.expect("?>")
    }

    fn comment(&mut self) -> Step<XmlNode> {
        self.at += "<!--".len();
        let len = self.text[self.at..].find("--").ok_or(Stop::Declined)?;
        let body = line_ends(&self.text[self.at..self.at + len]);
        self.at += len;
        self.expect("-->")?;
        Ok(XmlNode::Comment(body))
    }

    fn processing_instruction(&mut self) -> Step<XmlNode> {
        self.at += "<?".len();
        if self.name()?.eq_ignore_ascii_case("xml") {
            return Err(Stop::Declined);
        }
        if !self.starts("?>") {
            self.required_space()?;
            self.at += self.text[self.at..].find("?>").ok_or(Stop::Declined)?;
        }
        self.at += "?>".len();
        Ok(XmlNode::Other)
    }

    /// A quoted literal, its text.
    fn literal(&mut self) -> Step<String> {
        let quote = self.peek()?;
        if quote != b'"' && quote != b'\'' {
            return Err(Stop::Declined);
        }
        let start = self.at + 1;
        let len = self.bytes[start..].iter().position(|&b| b == quote).ok_or(Stop::Declined)?;
        self.at = start + len + 1;
        Ok(self.text[start..start + len].to_string())
    }

    /// SYSTEM or PUBLIC at point, whose system literal is optional when
    /// declaring a notation: whether one was there.
    fn external_id(&mut self, notation: bool) -> Step<bool> {
        if self.starts("SYSTEM") {
            self.at += "SYSTEM".len();
            self.required_space()?;
            self.literal()?;
            return Ok(true);
        }
        if self.starts("PUBLIC") {
            self.at += "PUBLIC".len();
            self.required_space()?;
            let public = self.literal()?;
            let pubid = |b: u8| b.is_ascii_alphanumeric() || b" \r\n-'()+,./:=?;!*#@$_%".contains(&b);
            if !public.bytes().all(pubid) {
                return Err(Stop::Declined);
            }
            let before = self.at;
            self.skip_space();
            if notation && self.starts(">") {
                return Ok(true);
            }
            if self.at == before {
                return Err(Stop::Declined);
            }
            self.literal()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn doctype(&mut self) -> Step<()> {
        self.at += "<!DOCTYPE".len();
        self.required_space()?;
        self.name()?;
        self.skip_space();
        if self.external_id(false)? {
            self.doc().external = true;
        }
        self.skip_space();
        if self.starts("[") {
            self.at += 1;
            self.subset(true)?;
            self.skip_space();
        }
        self.expect(">")
    }

    /// Markup declarations up to `]' (INTERNAL), or to the end of a
    /// parameter entity's text.
    fn subset(&mut self, internal: bool) -> Step<()> {
        loop {
            self.skip_space();
            if self.at_end() {
                return if internal { Err(Stop::Declined) } else { Ok(()) };
            }
            if self.starts("]") && internal {
                self.at += 1;
                return Ok(());
            }
            if self.starts("%") {
                self.at += 1;
                let name = self.name()?;
                self.expect(";")?;
                match self.doc().parameters.get(&name).cloned() {
                    Some(Some(value)) => {
                        let open = format!("%{name}");
                        self.open_entity(&open)?;
                        self.charge(value.len() as u64)?;
                        let expanded = self.nested(&value, |sub| sub.subset(false));
                        self.doc().checking.pop();
                        expanded?;
                    }
                    _ => self.doc().external = true,
                }
            } else if self.starts("<!--") {
                self.comment()?;
            } else if self.starts("<?") {
                self.processing_instruction()?;
            } else if self.starts("<!ENTITY") {
                self.entity_declaration()?;
            } else if self.starts("<!ELEMENT") {
                self.element_declaration()?;
            } else if self.starts("<!ATTLIST") {
                self.attlist_declaration()?;
            } else if self.starts("<!NOTATION") {
                self.at += "<!NOTATION".len();
                self.required_space()?;
                self.name()?;
                self.required_space()?;
                if !self.external_id(true)? {
                    return Err(Stop::Declined);
                }
                self.skip_space();
                self.expect(">")?;
            } else {
                return Err(Stop::Declined);
            }
        }
    }

    fn entity_declaration(&mut self) -> Step<()> {
        self.at += "<!ENTITY".len();
        self.required_space()?;
        let parameter = self.starts("%");
        if parameter {
            self.at += 1;
            self.required_space()?;
        }
        let name = self.name()?;
        self.required_space()?;
        let entity = if self.external_id(false)? {
            let before = self.at;
            self.skip_space();
            if self.starts("NDATA") {
                if self.at == before || parameter {
                    return Err(Stop::Declined);
                }
                self.at += "NDATA".len();
                self.required_space()?;
                self.name()?;
                Entity::Unparsed
            } else {
                Entity::External
            }
        } else {
            let value = self.literal()?;
            Entity::Internal(self.nested(&value, |sub| sub.entity_value())?)
        };
        self.skip_space();
        self.expect(">")?;
        let doc = self.doc();
        if parameter {
            let value = if let Entity::Internal(value) = entity { Some(value) } else { None };
            doc.parameters.entry(name).or_insert(value);
        } else {
            doc.entities.entry(name).or_insert(entity);
        }
        Ok(())
    }

    /// The literal at hand as an entity's replacement text.
    fn entity_value(&mut self) -> Step<String> {
        let mut value = String::new();
        while !self.at_end() {
            match self.peek()? {
                // A parameter entity declared here may not be referred to in
                // the internal subset; an undefined one ends the value.
                b'%' => {
                    self.at += 1;
                    let name = self.name()?;
                    self.expect(";")?;
                    if self.doc().parameters.contains_key(&name) {
                        return Err(Stop::Declined);
                    }
                    self.doc().external = true;
                    return Ok(value);
                }
                b'&' => {
                    let start = self.at;
                    match self.reference()? {
                        Reference::Char(c) => value.push(c),
                        Reference::Named(_) => value.push_str(&self.text[start..self.at]),
                    }
                }
                _ => {
                    let rest = &self.text[self.at..];
                    let len = rest.find(['%', '&']).unwrap_or(rest.len());
                    value.push_str(&rest[..len]);
                    self.at += len;
                }
            }
        }
        Ok(value)
    }

    fn element_declaration(&mut self) -> Step<()> {
        self.at += "<!ELEMENT".len();
        self.required_space()?;
        let name = self.name()?;
        self.required_space()?;
        let mixed = if self.starts("EMPTY") {
            self.at += "EMPTY".len();
            false
        } else if self.starts("ANY") {
            self.at += "ANY".len();
            true
        } else {
            self.content_model(1)?
        };
        self.skip_space();
        self.expect(">")?;
        self.doc().mixed.entry(name).or_insert(mixed);
        Ok(())
    }

    /// A parenthesized content model at DEPTH with its occurrence mark:
    /// whether it is mixed, that is, opens with #PCDATA, which only the
    /// whole model may.
    fn content_model(&mut self, depth: usize) -> Step<bool> {
        if depth > MAX_MODEL_DEPTH {
            return Err(Stop::Declined);
        }
        self.expect("(")?;
        self.skip_space();
        let mixed = self.starts("#PCDATA");
        if mixed {
            if depth > 1 {
                return Err(Stop::Declined);
            }
            self.at += "#PCDATA".len();
        } else {
            self.content_particle(depth)?;
        }
        let (mut separator, mut names) = (None, false);
        loop {
            self.skip_space();
            let next = self.peek()?;
            if next == b')' {
                break;
            }
            if !(next == b'|' || next == b',' && !mixed) || *separator.get_or_insert(next) != next {
                return Err(Stop::Declined);
            }
            self.at += 1;
            self.skip_space();
            if mixed {
                self.name()?;
                names = true;
            } else {
                self.content_particle(depth)?;
            }
        }
        self.at += 1;
        let mark = self.bytes.get(self.at).copied();
        if mixed {
            if mark == Some(b'*') {
                self.at += 1;
            } else if names {
                return Err(Stop::Declined);
            }
        } else if matches!(mark, Some(b'?' | b'*' | b'+')) {
            self.at += 1;
        }
        Ok(mixed)
    }

    fn content_particle(&mut self, depth: usize) -> Step<()> {
        if self.starts("(") {
            return self.content_model(depth + 1).map(|_| ());
        }
        self.name()?;
        if self.bytes.get(self.at).is_some_and(|b| matches!(b, b'?' | b'*' | b'+')) {
            self.at += 1;
        }
        Ok(())
    }

    /// An enumeration's parenthesized alternatives: Names under NOTATION,
    /// else Nmtokens.
    fn enumeration(&mut self, names: bool) -> Step<()> {
        self.expect("(")?;
        loop {
            self.skip_space();
            if names {
                self.name()?;
            } else {
                let rest = &self.text[self.at..];
                let len = rest.find(|c| !xml_name_char(c)).unwrap_or(rest.len());
                if len == 0 {
                    return Err(Stop::Declined);
                }
                self.at += len;
            }
            self.skip_space();
            match self.peek()? {
                b'|' => self.at += 1,
                b')' => {
                    self.at += 1;
                    return Ok(());
                }
                _ => return Err(Stop::Declined),
            }
        }
    }

    /// An attribute default: the references in it must stand, as a value's.
    fn default_value(&mut self) -> Step<()> {
        let value = self.literal()?;
        self.nested(&value, |sub| {
            while !sub.at_end() {
                match sub.peek()? {
                    b'<' => return Err(Stop::Declined),
                    b'&' => {
                        if let Reference::Named(name) = sub.reference()? {
                            sub.attribute_entity(&name)?;
                        }
                    }
                    _ => {
                        let rest = &sub.text[sub.at..];
                        sub.at += rest.find(['<', '&']).unwrap_or(rest.len());
                    }
                }
            }
            Ok(())
        })
    }

    fn attlist_declaration(&mut self) -> Step<()> {
        self.at += "<!ATTLIST".len();
        self.required_space()?;
        let element = self.name()?;
        loop {
            self.skip_space();
            if self.starts(">") {
                self.at += 1;
                return Ok(());
            }
            let attribute = self.name()?;
            self.required_space()?;
            let kind = if self.starts("(") { String::new() } else { self.name()? };
            match kind.as_str() {
                "CDATA" => {}
                "ID" | "IDREF" | "IDREFS" | "ENTITY" | "ENTITIES" | "NMTOKEN" | "NMTOKENS" | "" | "NOTATION" => {
                    if kind == "NOTATION" {
                        self.required_space()?;
                    }
                    if kind.is_empty() || kind == "NOTATION" {
                        self.enumeration(!kind.is_empty())?;
                    }
                    self.doc().tokenized.insert(format!("{element}\0{attribute}"));
                }
                _ => return Err(Stop::Declined),
            }
            self.required_space()?;
            if self.starts("#REQUIRED") {
                self.at += "#REQUIRED".len();
            } else if self.starts("#IMPLIED") {
                self.at += "#IMPLIED".len();
            } else {
                if self.starts("#FIXED") {
                    self.at += "#FIXED".len();
                    self.required_space()?;
                }
                self.default_value()?;
            }
            if !self.starts(">") {
                self.required_space()?;
            }
        }
    }

    /// The reference whose `&' is at point.
    fn reference(&mut self) -> Step<Reference> {
        self.at += 1;
        if self.starts("#") {
            self.at += 1;
            let radix = if self.starts("x") {
                self.at += 1;
                16
            } else {
                10
            };
            let start = self.at;
            while self.bytes.get(self.at).is_some_and(|b| if radix == 16 { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }) {
                self.at += 1;
            }
            let code = u32::from_str_radix(&self.text[start..self.at], radix).map_err(|_| Stop::Declined)?;
            self.expect(";")?;
            return char::from_u32(code).filter(|&c| xml_char(c)).map(Reference::Char).ok_or(Stop::Declined);
        }
        let name = self.name()?;
        self.expect(";")?;
        Ok(Reference::Named(name))
    }

    /// How far the document's own text was read: what amplification is
    /// measured against, so text not reached yet does not count.
    fn note_consumed(&mut self) {
        if self.top {
            let at = self.at as u64;
            self.doc().consumed = at;
        }
    }

    /// Charge a reference to SIZE bytes of text at this point of the input.
    fn charge(&mut self, size: u64) -> Step<()> {
        self.note_consumed();
        self.doc().charge(size)
    }

    /// Enter entity NAME, one level deeper; it must not be open already.
    fn open_entity(&mut self, name: &str) -> Step<()> {
        let doc = self.doc();
        if doc.checking.len() >= MAX_ENTITY_DEPTH || doc.checking.iter().any(|open| open == name) {
            return Err(Stop::Declined);
        }
        doc.checking.push(name.to_string());
        Ok(())
    }

    /// Charge a reference to the checked entity NAME.
    fn charge_entity(&mut self, name: &str) -> Step<()> {
        let size = self.doc().expanded.get(name).copied().unwrap_or(0);
        self.charge(size)
    }

    /// Whether NAME may be referred to from an attribute value: it is an
    /// internal entity that expands to no `<' (`Ok(true)`), or undefined
    /// where that is no error (`Ok(false)`).
    fn attribute_entity(&mut self, name: &str) -> Step<bool> {
        if predefined(name).is_some() {
            return Ok(true);
        }
        if self.doc().attribute_ok.contains(name) {
            self.charge_entity(name)?;
            return Ok(true);
        }
        let value = match self.doc().entities.get(name).cloned() {
            Some(Entity::Internal(value)) => value,
            Some(_) => return Err(Stop::Declined),
            None if self.doc().undefined_is_fatal() => return Err(Stop::Declined),
            None => return Ok(false),
        };
        self.open_entity(name)?;
        let before = self.doc().cost;
        let checked = self.nested(&value, |sub| {
            while !sub.at_end() {
                match sub.peek()? {
                    b'<' => return Err(Stop::Declined),
                    b'&' => {
                        if let Reference::Named(inner) = sub.reference()? {
                            sub.attribute_entity(&inner)?;
                        }
                    }
                    _ => {
                        let rest = &sub.text[sub.at..];
                        sub.at += rest.find(['<', '&']).unwrap_or(rest.len());
                    }
                }
            }
            Ok(())
        });
        self.doc().checking.pop();
        checked?;
        let doc = self.doc();
        let size = (doc.cost - before).saturating_add(value.len() as u64);
        doc.expanded.insert(name.to_string(), size);
        doc.attribute_ok.insert(name.to_string());
        self.charge(value.len() as u64)?;
        Ok(true)
    }

    /// The value of ATTRIBUTE of ELEMENT, from point to END, as make_dom
    /// reads it: the attribute's first node, so the text before its first
    /// entity reference, or that entity's unexpanded replacement text.
    fn attribute_value(&mut self, element: &str, attribute: &str, end: usize) -> Step<String> {
        let mut value = String::new();
        let mut ended = false;
        while self.at < end {
            match self.bytes[self.at] {
                b'<' => return Err(Stop::Declined),
                b'&' => match self.reference()? {
                    Reference::Char(c) if !ended => value.push(c),
                    Reference::Char(_) => {}
                    Reference::Named(name) => {
                        if let Some(c) = predefined(&name) {
                            if !ended {
                                value.push(c);
                            }
                        } else {
                            let defined = self.attribute_entity(&name)?;
                            if !ended && value.is_empty() && defined {
                                if let Some(Entity::Internal(text)) = self.doc().entities.get(&name) {
                                    value.clone_from(text);
                                }
                            }
                            ended = true;
                        }
                    }
                },
                b'\r' | b'\n' | b'\t' => {
                    if self.bytes[self.at] == b'\r' && self.bytes.get(self.at + 1) == Some(&b'\n') {
                        self.at += 1;
                    }
                    self.at += 1;
                    if !ended {
                        value.push(' ');
                    }
                }
                _ => {
                    let rest = &self.text[self.at..end];
                    let len = rest.find(['<', '&', '\r', '\n', '\t']).unwrap_or(rest.len());
                    if !ended {
                        value.push_str(&rest[..len]);
                    }
                    self.at += len;
                }
            }
        }
        if self.doc().tokenized.contains(&format!("{element}\0{attribute}")) {
            value = value.split(' ').filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
        }
        Ok(value)
    }

    /// The name and attributes of the element whose start tag ended, once
    /// its attributes declare what they do: in a document, namespace
    /// declarations go, as does each prefix bound in scope, and xml:space
    /// takes effect.
    fn resolve(&mut self, name: String, attrs: Vec<(String, String)>) -> (String, Vec<(String, String)>) {
        let Some(doc) = &mut self.doc else { return (name, attrs) };
        let declaration = |attr: &str| attr == "xmlns" || attr.starts_with("xmlns:");
        for (attr, value) in &attrs {
            if let Some(prefix) = attr.strip_prefix("xmlns:") {
                let reserved = matches!(value.as_str(), "http://www.w3.org/2000/xmlns/" | "http://www.w3.org/XML/1998/namespace");
                if !value.is_empty() && !reserved && !prefix.contains(':') && prefix != "xml" && prefix != "xmlns" {
                    *doc.prefixes.entry(prefix.to_string()).or_insert(0) += 1;
                    if let Some(open) = doc.declared.last_mut() {
                        open.push(prefix.to_string());
                    }
                }
            } else if attr == "xml:space" {
                let space = match value.as_str() {
                    "preserve" => Some(Space::Preserve),
                    "default" => Some(Space::Default),
                    _ => None,
                };
                if let (Some(space), Some(top)) = (space, doc.space.last_mut()) {
                    *top = space;
                }
            }
        }
        let local = |doc: &Doc, name: String| match qname(&name) {
            Some((prefix, local)) if doc.bound(prefix) => local.to_string(),
            _ => name,
        };
        let attrs = attrs.into_iter().filter(|(attr, _)| !declaration(attr)).map(|(attr, value)| (local(doc, attr), value)).collect();
        (local(doc, name), attrs)
    }

    /// The content of the element ELEMENT (its name as written and as
    /// resolved) through its end tag, or, with None, the whole text of an
    /// entity.
    fn content(&mut self, element: Option<(&str, &str)>) -> Step<Vec<XmlNode>> {
        let mut children = Vec::new();
        loop {
            match self.bytes.get(self.at) {
                None if element.is_none() => return Ok(children),
                None => return Err(Stop::Declined),
                Some(b'<') => {
                    if self.starts("</") {
                        let Some((raw, _)) = element else { return Err(Stop::Declined) };
                        self.end_tag(raw)?;
                        return Ok(children);
                    } else if self.starts("<!--") {
                        children.push(self.comment()?);
                    } else if self.starts("<![CDATA[") {
                        self.at += "<![CDATA[".len();
                        let len = self.text[self.at..].find("]]>").ok_or(Stop::Declined)?;
                        children.push(XmlNode::Cdata(line_ends(&self.text[self.at..self.at + len])));
                        self.at += len + "]]>".len();
                    } else if self.starts("<?") {
                        children.push(self.processing_instruction()?);
                    } else if self.starts("<!") {
                        return Err(Stop::Declined);
                    } else {
                        children.push(self.element()?);
                    }
                }
                Some(b'&') => match self.reference()? {
                    Reference::Char(c) => push_text(&mut children, c.encode_utf8(&mut [0; 4])),
                    Reference::Named(name) => match predefined(&name) {
                        Some(c) => push_text(&mut children, c.encode_utf8(&mut [0; 4])),
                        None => {
                            self.content_entity(&name)?;
                            children.push(XmlNode::Other);
                        }
                    },
                },
                Some(_) => self.run(element.map(|(_, resolved)| resolved), &mut children)?,
            }
        }
    }

    /// Check that NAME, referred to from content, is well-balanced: its
    /// node stays a reference, as without NOENT.
    fn content_entity(&mut self, name: &str) -> Step<()> {
        if self.doc().content_ok.contains(name) {
            return self.charge_entity(name);
        }
        let value = match self.doc().entities.get(name).cloned() {
            Some(Entity::Internal(value)) => value,
            Some(Entity::External) => return Ok(()),
            Some(Entity::Unparsed) => return Err(Stop::Declined),
            None if self.doc().undefined_is_fatal() => return Err(Stop::Declined),
            None => return Ok(()),
        };
        self.open_entity(name)?;
        let before = self.doc().cost;
        let checked = self.nested(&value, |sub| sub.content(None).map(|_| ()));
        self.doc().checking.pop();
        checked?;
        let doc = self.doc();
        let size = (doc.cost - before).saturating_add(value.len() as u64);
        doc.expanded.insert(name.to_string(), size);
        doc.content_ok.insert(name.to_string());
        self.charge(value.len() as u64)
    }

    /// The run of character data at point, in the element named ELEMENT
    /// whose content so far is CHILDREN: kept, unless it is the blank run
    /// NOBLANKS drops (parser.c areBlanks).
    fn run(&mut self, element: Option<&str>, children: &mut Vec<XmlNode>) -> Step<()> {
        let rest = &self.text[self.at..];
        let len = rest.find(['<', '&']).unwrap_or(rest.len());
        let raw = &rest[..len];
        if raw.contains("]]>") {
            return Err(Stop::Declined);
        }
        self.at += len;
        let dropped = element.is_some_and(|name| self.are_blanks(raw, name, children));
        if !dropped {
            push_text(children, &line_ends(raw));
            if let Some(top) = self.doc().space.last_mut().filter(|top| **top == Space::Unset) {
                *top = Space::Text;
            }
        }
        Ok(())
    }

    fn are_blanks(&mut self, raw: &str, element: &str, children: &[XmlNode]) -> bool {
        let is_text = |node: Option<&XmlNode>| matches!(node, Some(XmlNode::Text(_)));
        let next_is_tag = self.starts("<");
        let next_is_end = self.starts("</");
        let doc = self.doc();
        if matches!(doc.space.last(), None | Some(Space::Preserve | Space::Text)) || !raw.bytes().all(space) {
            return false;
        }
        match doc.mixed.get(element) {
            Some(&mixed) => !mixed,
            None => next_is_tag && !(children.is_empty() && next_is_end) && !is_text(children.last()) && !is_text(children.first()),
        }
    }
}

/// TEXT with its line ends LF (XML 1.0 2.11). An entity's literal keeps
/// what it holds, as in libxml2.
fn line_ends(text: &str) -> String {
    if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text.to_string() }
}

/// TEXT added to the content CHILDREN, as a text node goes on growing.
fn push_text(children: &mut Vec<XmlNode>, text: &str) {
    match children.last_mut() {
        Some(XmlNode::Text(last)) => last.push_str(text),
        _ => children.push(XmlNode::Text(text.to_string())),
    }
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
