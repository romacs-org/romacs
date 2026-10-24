//! Shared helpers for romacs-face integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use romacs_face::attr::validate;
use romacs_face::{
    Attribute, Attrs, AttrsBuilder, Background, Base, Datum, Deffaces, LispGraph, NodeView, Slot,
    ThemeSpec,
};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/gnu/fixtures")
}

pub fn read_datum(path: &Path) -> Datum {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Datum::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `plist-get` over a `Datum` plist.
pub fn plist_get<'a>(plist: &'a Datum, key: &str) -> Option<&'a Datum> {
    let items = plist.as_list()?;
    items
        .chunks(2)
        .find(|kv| kv[0].as_symbol() == Some(key))
        .and_then(|kv| kv.get(1))
}

pub fn symbol(d: &Datum) -> &str {
    d.as_symbol().unwrap_or_else(|| panic!("not a symbol: {d}"))
}

/// The defface table every GNU run shares: face name -> raw spec.
pub fn deffaces_table() -> Vec<(String, Datum)> {
    let text = std::fs::read_to_string(fixtures().join("deffaces.eld")).expect("deffaces.eld");
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let d = Datum::parse(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        let items = d.as_list().expect("entry");
        out.push((symbol(&items[0]).to_owned(), items[1].clone()));
    }
    assert!(
        out.len() > 400,
        "defface table suspiciously small: {}",
        out.len()
    );
    out
}

/// One theme dump, decoded.
pub struct Dump {
    pub theme: Option<String>,
    pub mode: Background,
    pub base: Datum,
    pub default_row: Datum,
    pub palette: Datum,
    /// (face, theme spec if any, own plist, realized plist minus default row)
    pub faces: Vec<(String, Option<Datum>, Datum, Datum)>,
}

pub fn load_dump(name: &str) -> Dump {
    let d = read_datum(&fixtures().join(format!("{name}.eld")));
    let theme = plist_get(&d, ":theme")
        .and_then(|t| t.as_symbol())
        .filter(|t| *t != "nil")
        .map(str::to_owned);
    let mode = match symbol(plist_get(&d, ":mode").expect(":mode")) {
        "dark" => Background::Dark,
        "light" => Background::Light,
        other => panic!("mode {other}"),
    };
    let faces = plist_get(&d, ":faces")
        .and_then(Datum::as_list)
        .expect(":faces")
        .iter()
        .map(|e| {
            let v = e.as_list().expect("face entry");
            let spec = v[1].as_list().and_then(|l| l.first()).cloned();
            (symbol(&v[0]).to_owned(), spec, v[2].clone(), v[3].clone())
        })
        .collect();
    Dump {
        theme,
        mode,
        base: plist_get(&d, ":base").expect(":base").clone(),
        default_row: plist_get(&d, ":default").expect(":default").clone(),
        palette: plist_get(&d, ":palette").cloned().unwrap_or(Datum::Nil),
        faces,
    }
}

/// Attributes GNU's dump reports (`face-attribute-name-alist`).
pub fn dumped_attributes() -> impl Iterator<Item = Attribute> {
    Attribute::ALL
        .into_iter()
        .filter(|a| *a != Attribute::DistantForeground)
}

pub fn theme_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .expect("fixtures dir")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".eld").map(str::to_owned))
        .filter(|n| n != "deffaces" && n != "nil" && n != "remap")
        .collect();
    names.sort();
    names
}

pub fn base_from(plist: &Datum) -> Base {
    let mut b = AttrsBuilder::default();
    for a in Attribute::ALL {
        // GNU's face-attribute-name-alist has no :distant-foreground.
        if a == Attribute::DistantForeground {
            continue;
        }
        let v =
            plist_get(plist, a.keyword()).unwrap_or_else(|| panic!("base lacks {}", a.keyword()));
        b.set(
            a,
            validate("base", a, &romacs_face::BoundedDatum::new(v).unwrap())
                .expect("valid base value"),
        );
    }
    Base::new(b.build()).expect("absolute base")
}

/// Inputs for one dump: the defface registry restricted to the faces GNU
/// had, in GNU's order, and the theme's specs.
pub fn inputs(dump: &Dump, table: &[(String, Datum)]) -> (Arc<Deffaces>, Option<Arc<ThemeSpec>>) {
    let mut deffaces = Deffaces::default();
    for (face, ..) in &dump.faces {
        let spec = table
            .iter()
            .find(|(n, _)| n == face)
            .map(|(_, s)| s.clone())
            .unwrap_or(Datum::Nil);
        deffaces
            .defface(face, &romacs_face::BoundedDatum::new(&spec).unwrap())
            .unwrap_or_else(|e| panic!("defface {face}: {e}"));
    }
    let spec = dump.theme.as_ref().map(|t| {
        let mut ts = ThemeSpec::new(t);
        for (face, spec, ..) in &dump.faces {
            if let Some(s) = spec {
                ts.set_face(face, &romacs_face::BoundedDatum::new(s).unwrap())
                    .unwrap_or_else(|e| panic!("{t}/{face}: {e}"));
            }
        }
        Arc::new(ts)
    });
    (Arc::new(deffaces), spec)
}

/// A slot as the Lisp value GNU's `face-attribute` would return.
pub fn slot_datum(slot: &Slot) -> Datum {
    match slot {
        Slot::Unspecified => Datum::symbol("unspecified"),
        Slot::Reset => Datum::symbol("reset"),
        Slot::Set(v) => v.to_datum(),
    }
}

pub fn attrs_get(attrs: &Attrs, a: Attribute) -> Datum {
    slot_datum(attrs.get(a))
}

/// Equality of Lisp values as `equal` sees them, except that floats compare
/// by value (GNU prints `1.0`; both sides read it back as 1.0).
pub fn lisp_equal(a: &Datum, b: &Datum) -> bool {
    a == b
}

/// A toy Lisp heap of cons cells and atoms with real identity, so tests can
/// build the circular structures GNU crashes or hangs on.
#[derive(Default)]
pub struct Heap {
    pub nodes: Vec<HeapNode>,
}

#[derive(Clone)]
pub enum HeapNode {
    Atom(Datum),
    Cons(usize, usize),
}

impl Heap {
    pub fn atom(&mut self, d: Datum) -> usize {
        self.nodes.push(HeapNode::Atom(d));
        self.nodes.len() - 1
    }
    pub fn cons(&mut self, car: usize, cdr: usize) -> usize {
        self.nodes.push(HeapNode::Cons(car, cdr));
        self.nodes.len() - 1
    }
    pub fn nil(&mut self) -> usize {
        self.atom(Datum::Nil)
    }
    /// A proper list of existing nodes.
    pub fn list(&mut self, items: &[usize]) -> usize {
        let mut tail = self.nil();
        for &i in items.iter().rev() {
            tail = self.cons(i, tail);
        }
        tail
    }
    pub fn set_car(&mut self, cell: usize, car: usize) {
        let HeapNode::Cons(_, cdr) = self.nodes[cell] else {
            panic!("not a cons")
        };
        self.nodes[cell] = HeapNode::Cons(car, cdr);
    }
    pub fn set_cdr(&mut self, cell: usize, cdr: usize) {
        let HeapNode::Cons(car, _) = self.nodes[cell] else {
            panic!("not a cons")
        };
        self.nodes[cell] = HeapNode::Cons(car, cdr);
    }
    pub fn cdr(&self, cell: usize) -> usize {
        let HeapNode::Cons(_, cdr) = self.nodes[cell] else {
            panic!("not a cons")
        };
        cdr
    }
}

impl LispGraph for Heap {
    type Node = usize;
    fn view(&self, node: usize) -> NodeView<'_, usize> {
        match &self.nodes[node] {
            HeapNode::Cons(a, d) => NodeView::Cons(*a, *d),
            HeapNode::Atom(Datum::Nil) => NodeView::Nil,
            HeapNode::Atom(Datum::T) => NodeView::T,
            HeapNode::Atom(Datum::Int(i)) => NodeView::Int(*i),
            HeapNode::Atom(Datum::Float(f)) => NodeView::Float(*f),
            HeapNode::Atom(Datum::Str(s)) => NodeView::Str(s),
            HeapNode::Atom(Datum::Symbol(s)) => NodeView::Symbol(s),
            HeapNode::Atom(_) => NodeView::Other("compound atom"),
        }
    }
}

/// Run F on a thread with a deliberately small stack: anything recursive in
/// the size of its input overflows it and aborts the test binary.
pub fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

pub fn d(text: &str) -> romacs_face::BoundedDatum {
    romacs_face::BoundedDatum::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}
