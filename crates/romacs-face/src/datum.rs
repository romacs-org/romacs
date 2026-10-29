//! Plain-data input language for face specs.
//!
//! The Lisp API (`defface`, `set-face-attribute`, `face-remapping-alist`)
//! hands this crate Lisp values through [`BoundedDatum::from_graph`]. Every
//! face-data API requires that checked type: it is owned, acyclic, and bounded
//! in depth, cons cells and expanded string/symbol bytes. It holds no Lisp
//! handle, so a moving GC has nothing here to trace or relocate.
//!
//! [`Datum`] itself is deliberately an **untrusted** construction/fixture
//! language. Its public variants do not enforce bounds. Do not clone, format,
//! compare or recursively drop an arbitrarily deep raw tree; its owner must
//! dismantle it iteratively. [`BoundedDatum::new`] borrows and checks a raw tree
//! iteratively *before* cloning it; unchecked data cannot enter a face API.
//!
//! The conversion is where GNU's two pathological inputs are stopped:
//!
//! * a circular `:inherit` plist or circular list of face refs, which GNU's
//!   `merge_face_ref` recurses on in C until SIGSEGV, and
//! * a circular plist *tail*, which GNU's plist walk loops on forever.
//!
//! Both become [`DatumError::Circular`] here, before any face exists. The
//! conversion itself is iterative (an explicit stack), so no input shape can
//! overflow the Rust stack either.

use std::fmt::{self, Write as _};
use std::hash::Hash;
use std::sync::Arc;

use rustc_hash::FxHashSet;

/// Deepest list nesting accepted in a face spec. Real specs nest at most five
/// levels (`spec > entry > display > conjunct > options`, or a box plist inside
/// an attribute plist); 32 leaves a wide margin while keeping every later walk
/// over a `BoundedDatum`, including its `Drop`, trivially bounded.
pub const MAX_DEPTH: usize = 32;

/// Most cons cells one conversion may visit. Shared (DAG) substructure is
/// expanded, so this also bounds exponential sharing.
pub const MAX_CONSES: usize = 1 << 20;

/// Maximum expanded UTF-8 payload per boundary value (not per shared atom).
pub const MAX_BYTES: usize = 16 << 20;

/// Checked boundary value. Its private field cannot be replaced or mutated.
/// Raw `Datum` is an untrusted construction language, never a face API input.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundedDatum(Datum);
romacs_snapshot::snapshot_tuple!(BoundedDatum(f0));

impl BoundedDatum {
    pub fn new(raw: &Datum) -> Result<Self, DatumError> {
        Self::check(raw)?;
        Ok(Self(raw.clone()))
    }

    fn check(raw: &Datum) -> Result<(), DatumError> {
        let mut stack = vec![(raw, 0usize)];
        let mut conses = 0usize;
        let mut bytes = 0usize;
        while let Some((node, depth)) = stack.pop() {
            match node {
                Datum::List(items) | Datum::Dotted(items, _) => {
                    if depth >= MAX_DEPTH {
                        return Err(DatumError::TooDeep);
                    }
                    conses = conses.saturating_add(items.len());
                    if conses > MAX_CONSES {
                        return Err(DatumError::TooLarge);
                    }
                    stack.extend(items.iter().map(|d| (d, depth + 1)));
                    if let Datum::Dotted(_, tail) = node {
                        stack.push((tail, depth + 1));
                    }
                }
                Datum::Str(s) | Datum::Symbol(s) => {
                    bytes = bytes.saturating_add(s.len());
                    if bytes > MAX_BYTES {
                        return Err(DatumError::TooManyBytes);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn parse(text: &str) -> Result<Self, DatumError> {
        Datum::parse(text).map(Self)
    }

    pub fn from_graph<G: LispGraph>(graph: &G, root: G::Node) -> Result<Self, DatumError> {
        Datum::from_graph(graph, root).map(Self)
    }
}

impl std::ops::Deref for BoundedDatum {
    type Target = Datum;
    fn deref(&self) -> &Datum {
        &self.0
    }
}

impl fmt::Display for BoundedDatum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq<Datum> for BoundedDatum {
    fn eq(&self, other: &Datum) -> bool {
        self.0 == *other
    }
}
impl PartialEq<BoundedDatum> for Datum {
    fn eq(&self, other: &BoundedDatum) -> bool {
        *self == other.0
    }
}

/// Untrusted owned Lisp data. Use `BoundedDatum` at every face boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    Nil,
    T,
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    /// Any symbol other than `nil` and `t`, keywords included (`:foreground`).
    Symbol(Arc<str>),
    /// A non-empty proper list.
    List(Vec<Datum>),
    /// A non-empty improper list `(a b . tail)`; `tail` is never a list.
    Dotted(Vec<Datum>, Box<Datum>),
}
romacs_snapshot::snapshot_enum!(Datum {
    Datum::Nil => (),
    Datum::T => (),
    Datum::Int(f0) => (f0),
    Datum::Float(f0) => (f0),
    Datum::Str(f0) => (f0),
    Datum::Symbol(f0) => (f0),
    Datum::List(f0) => (f0),
    Datum::Dotted(f0, f1) => (f0, f1),
});

/// What a Lisp heap node looks like to the converter.
///
/// `N` is a heap handle. The converter only copies handles and reads through
/// `&G`; it never stores one past conversion's return. A core-side
/// implementation MUST borrow `&Context` for the whole conversion.
/// Tagged `Value` reads alone do not borrow the heap or prevent collection.
/// `view` must never allocate (including native allocation), intern symbols,
/// call Lisp, or enter a GC safe point. Atoms are borrowed; only the converter
/// allocates native owned storage, after checking its budget. The shared
/// Context borrow excludes the `&mut Context` collection entry points.
/// This core-free crate cannot enforce that adapter contract: wiring must
/// supply and test that adapter, not a zero-sized unborrowed reader.
#[derive(Clone, Debug)]
pub enum NodeView<'a, N> {
    Nil,
    T,
    Int(i64),
    Float(f64),
    Str(&'a str),
    /// Interned symbol name. The core reader must reject uninterned symbols
    /// as `Other("uninterned symbol")`, never silently identify them by name.
    Symbol(&'a str),
    Cons(N, N),
    /// Anything a face spec cannot contain (vectors, markers, closures...).
    Other(&'static str),
}

/// A read-only view of a Lisp heap.
pub trait LispGraph {
    type Node: Copy + Eq + Hash;
    fn view(&self, node: Self::Node) -> NodeView<'_, Self::Node>;
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DatumError {
    #[error("circular list structure in face spec ({0})")]
    Circular(&'static str),
    #[error("face spec nests deeper than {MAX_DEPTH} lists")]
    TooDeep,
    #[error("face spec has more than {MAX_CONSES} cons cells")]
    TooLarge,
    #[error("face spec exceeds {MAX_BYTES} expanded string/symbol bytes")]
    TooManyBytes,
    #[error("face spec contains a {0}, which no face attribute accepts")]
    Unsupported(&'static str),
    #[error("malformed printed datum at byte {0}: {1}")]
    Syntax(usize, &'static str),
}

impl Datum {
    pub fn symbol(name: &str) -> Datum {
        match name {
            "nil" => Datum::Nil,
            "t" => Datum::T,
            _ => Datum::Symbol(name.into()),
        }
    }

    pub fn string(s: &str) -> Datum {
        Datum::Str(s.into())
    }

    pub fn list(items: Vec<Datum>) -> Datum {
        if items.is_empty() {
            Datum::Nil
        } else {
            Datum::List(items)
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Datum::Nil)
    }

    /// The symbol name, for `nil`, `t` and other symbols.
    pub fn as_symbol(&self) -> Option<&str> {
        match self {
            Datum::Nil => Some("nil"),
            Datum::T => Some("t"),
            Datum::Symbol(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_keyword(&self) -> bool {
        matches!(self, Datum::Symbol(s) if s.starts_with(':'))
    }

    /// Elements of a proper list; `nil` is the empty list.
    pub fn as_list(&self) -> Option<&[Datum]> {
        match self {
            Datum::Nil => Some(&[]),
            Datum::List(v) => Some(v),
            _ => None,
        }
    }

    /// Convert a Lisp value. Iterative; rejects cycles, over-deep nesting and
    /// oversized structure. See the module docs.
    pub fn from_graph<G: LispGraph>(graph: &G, root: G::Node) -> Result<Datum, DatumError> {
        struct Frame<N> {
            items: Vec<Datum>,
            next: Option<N>,
            /// Cons cells of this list's spine walked so far; they are on the
            /// current path until the frame completes.
            spine: Vec<N>,
        }
        let mut on_path: FxHashSet<G::Node> = FxHashSet::default();
        let mut stack: Vec<Frame<G::Node>> = Vec::new();
        let mut visited = 0usize;
        let mut bytes = 0usize;
        let mut pending = Some(root);
        loop {
            // Convert `pending` (an element or the root) or continue the top frame.
            if let Some(node) = pending.take() {
                match graph.view(node) {
                    NodeView::Cons(..) => {
                        if on_path.contains(&node) {
                            return Err(DatumError::Circular("a list contains itself"));
                        }
                        if stack.len() >= MAX_DEPTH {
                            return Err(DatumError::TooDeep);
                        }
                        stack.push(Frame {
                            items: Vec::new(),
                            next: Some(node),
                            spine: Vec::new(),
                        });
                    }
                    atom => {
                        let d = atom_datum(atom, &mut bytes)?;
                        match stack.last_mut() {
                            Some(top) => top.items.push(d),
                            None => return Ok(d),
                        }
                    }
                }
                continue;
            }
            let Some(top) = stack.last_mut() else {
                unreachable!("an empty stack returns above");
            };
            let cursor = top.next.take().expect("frame without cursor");
            match graph.view(cursor) {
                NodeView::Cons(car, cdr) => {
                    if !on_path.insert(cursor) {
                        return Err(DatumError::Circular("a list's tail returns to itself"));
                    }
                    visited += 1;
                    if visited > MAX_CONSES {
                        return Err(DatumError::TooLarge);
                    }
                    top.spine.push(cursor);
                    top.next = Some(cdr);
                    pending = Some(car);
                }
                tail => {
                    let frame = stack.pop().expect("top exists");
                    for c in &frame.spine {
                        on_path.remove(c);
                    }
                    let d = match tail {
                        NodeView::Nil => Datum::List(frame.items),
                        other => {
                            Datum::Dotted(frame.items, Box::new(atom_datum(other, &mut bytes)?))
                        }
                    };
                    match stack.last_mut() {
                        Some(parent) => parent.items.push(d),
                        None => return Ok(d),
                    }
                }
            }
        }
    }

    /// Print as GNU `prin1` would for the data a face can hold.
    pub fn to_lisp(&self) -> String {
        let mut out = String::new();
        self.write_lisp(&mut out);
        out
    }

    fn write_lisp(&self, out: &mut String) {
        match self {
            Datum::Nil => out.push_str("nil"),
            Datum::T => out.push('t'),
            Datum::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Datum::Float(f) => out.push_str(&float_to_lisp(*f)),
            Datum::Str(s) => write_lisp_string(s, out),
            Datum::Symbol(s) => out.push_str(s),
            Datum::List(items) | Datum::Dotted(items, _) => {
                out.push('(');
                for (i, d) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    d.write_lisp(out);
                }
                if let Datum::Dotted(_, tail) = self {
                    out.push_str(" . ");
                    tail.write_lisp(out);
                }
                out.push(')');
            }
        }
    }

    /// Read printed data (GNU `prin1` output with `print-escape-newlines`).
    ///
    /// This is a fixture reader for oracle dumps, not the Lisp reader: it
    /// handles lists, dotted pairs, strings, symbols, integers, floats,
    /// `'x` and `;` comments, and rejects everything else. Nesting is bounded by
    /// [`MAX_DEPTH`], quotes included, with the same cons/payload limits as
    /// graph conversion. The printed source itself must also fit MAX_BYTES.
    pub fn parse(text: &str) -> Result<Datum, DatumError> {
        if text.len() > MAX_BYTES {
            return Err(DatumError::TooManyBytes);
        }
        let mut r = Reader {
            s: text.as_bytes(),
            i: 0,
        };
        let d = r.read()?;
        r.skip_ws();
        if r.i != r.s.len() {
            return Err(DatumError::Syntax(r.i, "trailing input"));
        }
        BoundedDatum::check(&d)?;
        Ok(d)
    }
}

fn atom_datum<N>(view: NodeView<'_, N>, bytes: &mut usize) -> Result<Datum, DatumError> {
    if let NodeView::Str(s) | NodeView::Symbol(s) = &view {
        *bytes = bytes.saturating_add(s.len());
        if *bytes > MAX_BYTES {
            return Err(DatumError::TooManyBytes);
        }
    }
    Ok(match view {
        NodeView::Nil => Datum::Nil,
        NodeView::T => Datum::T,
        NodeView::Int(i) => Datum::Int(i),
        NodeView::Float(f) => Datum::Float(f),
        NodeView::Str(s) => Datum::Str(s.into()),
        NodeView::Symbol(s) => Datum::symbol(s),
        NodeView::Other(kind) => return Err(DatumError::Unsupported(kind)),
        NodeView::Cons(..) => unreachable!("conses are frames"),
    })
}

/// GNU float printing for the values faces use (`1.0`, `1.2`, `0.8`).
pub fn float_to_lisp(f: f64) -> String {
    if f.is_finite() && f == f.trunc() && f.abs() < 1e16 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn write_lisp_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn skip_ws(&mut self) {
        while let Some(&c) = self.s.get(self.i) {
            if c == b';' {
                while self.s.get(self.i).is_some_and(|&c| c != b'\n') {
                    self.i += 1;
                }
            } else if c.is_ascii_whitespace() {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn read(&mut self) -> Result<Datum, DatumError> {
        // Iterative: each open paren pushes a frame; `'x` pushes a quote frame.
        enum F {
            List(Vec<Datum>, Option<Datum>, bool),
            Quote,
        }
        let mut stack: Vec<F> = Vec::new();
        let mut conses = 0usize;
        loop {
            self.skip_ws();
            let Some(&c) = self.s.get(self.i) else {
                return Err(DatumError::Syntax(self.i, "unexpected end"));
            };
            let mut value = match c {
                b'(' => {
                    self.i += 1;
                    if stack.len() >= MAX_DEPTH {
                        return Err(DatumError::TooDeep);
                    }
                    stack.push(F::List(Vec::new(), None, false));
                    continue;
                }
                b'\'' => {
                    self.i += 1;
                    if stack.len() >= MAX_DEPTH {
                        return Err(DatumError::TooDeep);
                    }
                    stack.push(F::Quote);
                    continue;
                }
                b')' => {
                    self.i += 1;
                    match stack.pop() {
                        Some(F::List(items, tail, dot_pending)) => {
                            if dot_pending {
                                return Err(DatumError::Syntax(self.i, "dot without tail"));
                            }
                            // `(a . (b c))' is `(a b c)', `(a . nil)' is `(a)'.
                            match tail {
                                Some(Datum::List(rest)) => {
                                    Datum::List(items.into_iter().chain(rest).collect())
                                }
                                Some(Datum::Dotted(rest, t)) => {
                                    Datum::Dotted(items.into_iter().chain(rest).collect(), t)
                                }
                                Some(Datum::Nil) | None => Datum::list(items),
                                Some(t) => Datum::Dotted(items, Box::new(t)),
                            }
                        }
                        _ => return Err(DatumError::Syntax(self.i, "unbalanced )")),
                    }
                }
                b'"' => self.read_string()?,
                _ => {
                    let tok = self.read_token()?;
                    if tok == "." {
                        match stack.last_mut() {
                            Some(F::List(items, None, dot)) if !items.is_empty() && !*dot => {
                                *dot = true;
                                continue;
                            }
                            _ => return Err(DatumError::Syntax(self.i, "misplaced dot")),
                        }
                    }
                    token_datum(&tok)
                }
            };
            // Deliver `value` to the enclosing frames.
            loop {
                match stack.last_mut() {
                    None => return Ok(value),
                    Some(F::Quote) => {
                        stack.pop();
                        conses += 2;
                        if conses > MAX_CONSES {
                            return Err(DatumError::TooLarge);
                        }
                        value = Datum::List(vec![Datum::symbol("quote"), value]);
                    }
                    Some(F::List(items, tail, dot)) => {
                        if *dot {
                            if tail.is_some() {
                                return Err(DatumError::Syntax(self.i, "two tails"));
                            }
                            *dot = false;
                            *tail = Some(value);
                        } else if tail.is_some() {
                            return Err(DatumError::Syntax(self.i, "element after tail"));
                        } else {
                            conses += 1;
                            if conses > MAX_CONSES {
                                return Err(DatumError::TooLarge);
                            }
                            items.push(value);
                        }
                        break;
                    }
                }
            }
        }
    }

    fn read_string(&mut self) -> Result<Datum, DatumError> {
        self.i += 1;
        let mut buf = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return Err(DatumError::Syntax(self.i, "unterminated string"));
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return Err(DatumError::Syntax(self.i, "dangling escape"));
                    };
                    self.i += 1;
                    match e {
                        b'n' => buf.push(b'\n'),
                        b't' => buf.push(b'\t'),
                        b'\n' => {}
                        other => buf.push(other),
                    }
                }
                other => buf.push(other),
            }
        }
        String::from_utf8(buf)
            .map(|s| Datum::Str(s.into()))
            .map_err(|_| DatumError::Syntax(self.i, "string is not UTF-8"))
    }

    fn read_token(&mut self) -> Result<String, DatumError> {
        let mut buf = Vec::new();
        while let Some(&c) = self.s.get(self.i) {
            if c.is_ascii_whitespace() || c == b'(' || c == b')' || c == b'"' || c == b'\'' {
                break;
            }
            if c == b'[' || c == b'#' && buf.is_empty() {
                return Err(DatumError::Syntax(self.i, "unsupported syntax"));
            }
            self.i += 1;
            if c == b'\\' {
                if let Some(&e) = self.s.get(self.i) {
                    self.i += 1;
                    buf.push(e);
                }
                continue;
            }
            buf.push(c);
        }
        String::from_utf8(buf).map_err(|_| DatumError::Syntax(self.i, "token is not UTF-8"))
    }
}

fn token_datum(tok: &str) -> Datum {
    if let Ok(i) = tok.parse::<i64>() {
        return Datum::Int(i);
    }
    let numeric = tok.bytes().any(|b| b.is_ascii_digit())
        && tok
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.e".contains(&b));
    if let (true, Ok(f)) = (numeric, tok.parse::<f64>()) {
        return Datum::Float(f);
    }
    Datum::symbol(tok)
}

impl fmt::Display for Datum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_lisp())
    }
}
