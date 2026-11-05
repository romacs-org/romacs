//! `face-remapping-alist` as an immutable value, bound to a theme.
//!
//! GNU applies remapping inside its recursive merge, cutting loops with a
//! linked list of "named merge points" on the C stack. Here a remapping is
//! a value with its own [`Generation`]; binding to a [`Theme`] produces an
//! immutable [`View`]. C merge points are replayed iteratively, preserving
//! transitive remapping barriers. [`crate::FaceCache`] memoizes each realized
//! face. Cycles between distinct remapping entries are rejected at binding;
//! inheritance backreferences use GNU's normal/remap hiding rules.

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::attr::{Attribute, Attrs, validate_raw as validate};
use crate::datum::{BoundedDatum, Datum};
use crate::spec::FaceError;
use crate::theme::{Generation, Theme};

/// One face reference, as `merge_face_ref` (xfaces.c) accepts them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layer {
    Named(Arc<str>),
    Anonymous(Arc<AnonymousFace>),
    /// `(:filtered (:window PARAMETER VALUE) REF)`.
    Filtered {
        parameter: Arc<str>,
        value: Datum,
        layers: Vec<Layer>,
    },
}
romacs_snapshot::snapshot_enum!(Layer {
    Layer::Named(f0) => (f0),
    Layer::Anonymous(f0) => (f0),
    Layer::Filtered { parameter, value, layers } => (parameter, value, layers),
});

/// An anonymous face: an attribute plist, possibly inheriting.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnonymousFace {
    pub attrs: Attrs,
    pub inherit: Vec<Layer>,
}
romacs_snapshot::snapshot_struct!(AnonymousFace { attrs, inherit });

/// Parse a face reference into layers, highest precedence first.
///
/// Recursion here follows a [`BoundedDatum`], which is acyclic and at
/// most [`crate::datum::MAX_DEPTH`] deep, so it is bounded by construction.
/// Checked face reference. Only the bounded parser can construct it.
#[derive(Clone, Debug)]
pub struct FaceRef(Vec<Layer>);

pub fn parse_face_ref(face_ref: &BoundedDatum) -> Result<FaceRef, FaceError> {
    parse_face_ref_raw(face_ref).map(FaceRef)
}

fn parse_face_ref_raw(face_ref: &Datum) -> Result<Vec<Layer>, FaceError> {
    let mut out = Vec::new();
    push_face_ref(face_ref, &mut out)?;
    Ok(out)
}

fn push_face_ref(r: &Datum, out: &mut Vec<Layer>) -> Result<(), FaceError> {
    match r {
        Datum::Nil => {}
        Datum::Symbol(s) | Datum::Str(s) => out.push(Layer::Named(s.clone())),
        Datum::T => out.push(Layer::Named("t".into())),
        Datum::Dotted(items, tail) if items.len() == 1 => {
            // (foreground-color . "red") / (background-color . "red")
            let attr = match items[0].as_symbol() {
                Some("foreground-color") => Attribute::Foreground,
                Some("background-color") => Attribute::Background,
                _ => return Err(bad_ref(r)),
            };
            let slot = validate("", attr, tail).map_err(|_| bad_ref(r))?;
            let attrs = Attrs::default().with(attr, slot);
            out.push(Layer::Anonymous(Arc::new(AnonymousFace {
                attrs,
                inherit: Vec::new(),
            })));
        }
        Datum::List(items) if items.first().and_then(Datum::as_symbol) == Some(":filtered") => {
            let [_, Datum::List(filter), inner] = items.as_slice() else {
                return Err(bad_ref(r));
            };
            let [kw, Datum::Symbol(parameter), value] = filter.as_slice() else {
                return Err(bad_ref(r));
            };
            if kw.as_symbol() != Some(":window") {
                return Err(bad_ref(r));
            }
            out.push(Layer::Filtered {
                parameter: parameter.clone(),
                value: value.clone(),
                layers: parse_face_ref_raw(inner)?,
            });
        }
        Datum::List(items) if items.first().is_some_and(Datum::is_keyword) => {
            out.push(Layer::Anonymous(Arc::new(parse_anonymous(items)?)))
        }
        Datum::List(items) => {
            for item in items {
                push_face_ref(item, out)?;
            }
        }
        _ => return Err(bad_ref(r)),
    }
    Ok(())
}

/// An anonymous plist. Unlike GNU, where an attribute *before* `:inherit` in
/// the plist is overridden by the inherited face, the plist's own attributes
/// always win over what it inherits, wherever `:inherit` appears; see
/// GNU-DIVERGENCES.md. Invalid attributes are skipped, as GNU's merge skips
/// them (it logs and carries on).
fn parse_anonymous(plist: &[Datum]) -> Result<AnonymousFace, FaceError> {
    let mut attrs = Attrs::default();
    let mut inherit = Vec::new();
    for kv in plist.chunks(2) {
        let value = kv.get(1).unwrap_or(&Datum::Nil);
        match kv[0].as_symbol() {
            Some(":inherit") => inherit = parse_face_ref_raw(value)?,
            Some(k) => {
                if let Some((a, Ok(slot))) =
                    Attribute::from_keyword(k).map(|a| (a, validate("", a, value)))
                {
                    attrs = attrs.with(a, slot);
                }
            }
            None => {}
        }
    }
    Ok(AnonymousFace { attrs, inherit })
}

fn bad_ref(r: &Datum) -> FaceError {
    FaceError::Spec {
        face: String::new(),
        reason: "invalid face reference",
        spec: r.to_lisp(),
    }
}

/// An immutable `face-remapping-alist`.
#[derive(Debug)]
pub struct Remap {
    generation: Generation,
    entries: FxHashMap<Arc<str>, Vec<Layer>>,
    order: Vec<Arc<str>>,
}
romacs_snapshot::snapshot_struct!(Remap { generation, entries, order });

impl Remap {
    pub fn empty() -> Arc<Remap> {
        Arc::new(Remap {
            generation: Generation::mint(),
            entries: FxHashMap::default(),
            order: Vec::new(),
        })
    }

    /// Parse `((FACE . FACE-REF) ...)`. As with `assq`, the first entry for
    /// a face wins.
    pub fn from_alist(alist: &BoundedDatum) -> Result<Arc<Remap>, FaceError> {
        let items = alist.as_list().ok_or_else(|| bad_ref(alist))?;
        let mut entries: FxHashMap<Arc<str>, Vec<Layer>> = FxHashMap::default();
        let mut order = Vec::new();
        for item in items {
            let (face, rest) = match item {
                Datum::List(v) if !v.is_empty() => (v[0].as_symbol(), Datum::list(v[1..].to_vec())),
                Datum::Dotted(v, tail) if v.len() == 1 => (v[0].as_symbol(), (**tail).clone()),
                _ => return Err(bad_ref(item)),
            };
            let face: Arc<str> = face.ok_or_else(|| bad_ref(item))?.into();
            let layers = parse_face_ref_raw(&rest)?;
            if !entries.contains_key(&face) {
                order.push(face.clone());
            }
            entries.entry(face).or_insert(layers);
        }
        Ok(Arc::new(Remap {
            generation: Generation::mint(),
            entries,
            order,
        }))
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The window parameters its `:filtered` layers test, sorted, once each.
    pub fn filter_parameters(&self) -> Vec<Arc<str>> {
        let mut names = Vec::new();
        for layers in self.entries.values() {
            filter_names(layers, &mut names);
        }
        names.sort_unstable();
        names.dedup();
        names
    }

    /// This remapping as a window shows it (xfaces.c filter_face_ref and
    /// evaluate_face_filter): each `(:filtered (:window PARAMETER VALUE) REF)`
    /// is REF where MATCHES says the window's PARAMETER is VALUE, else nothing.
    pub fn for_window(&self, matches: &dyn Fn(&str, &Datum) -> bool) -> Arc<Remap> {
        Arc::new(Remap {
            generation: Generation::mint(),
            entries: self.entries.iter().map(|(face, layers)| (face.clone(), for_window(layers, matches))).collect(),
            order: self.order.clone(),
        })
    }
}

impl FaceRef {
    /// This reference as a window shows it, as [`Remap::for_window`].
    pub fn for_window(&self, matches: &dyn Fn(&str, &Datum) -> bool) -> FaceRef {
        FaceRef(for_window(&self.0, matches))
    }
}

fn filter_names(layers: &[Layer], out: &mut Vec<Arc<str>>) {
    for layer in layers {
        match layer {
            Layer::Named(_) => {}
            Layer::Anonymous(face) => filter_names(&face.inherit, out),
            Layer::Filtered { parameter, layers, .. } => {
                out.push(parameter.clone());
                filter_names(layers, out);
            }
        }
    }
}

fn for_window(layers: &[Layer], matches: &dyn Fn(&str, &Datum) -> bool) -> Vec<Layer> {
    let mut out = Vec::with_capacity(layers.len());
    for layer in layers {
        match layer {
            Layer::Named(_) => out.push(layer.clone()),
            Layer::Anonymous(face) => out.push(Layer::Anonymous(Arc::new(AnonymousFace {
                attrs: face.attrs.clone(),
                inherit: for_window(&face.inherit, matches),
            }))),
            Layer::Filtered { parameter, value, layers } => {
                if matches(parameter, value) {
                    out.extend(for_window(layers, matches));
                }
            }
        }
    }
    out
}

/// A theme and a buffer-local remapping, resolved as immutable values.
#[derive(Debug)]
pub struct View {
    theme: Theme,
    remap: Arc<Remap>,
    resolved: Arc<Remap>,
    default_realized: Attrs,
}
romacs_snapshot::snapshot_struct!(View { theme, remap, resolved, default_realized });

impl View {
    /// Reject cycles between remapping entries, not valid inheritance backrefs.
    /// Use `bind_or_unremapped` at the buffer boundary to retain a paintable view.
    pub fn bind(theme: &Theme, remap: &Arc<Remap>) -> Result<View, FaceError> {
        let mut entries = FxHashMap::default();
        for name in &remap.order {
            entries
                .entry(Arc::from(theme.canonical_name(name)))
                .or_insert_with(|| remap.entries[name].clone());
        }
        let normalized = Arc::new(Remap {
            generation: remap.generation,
            entries,
            order: remap.order.clone(),
        });
        check_remap_cycles(theme, &normalized)?;
        let default_realized = render_named(theme, &normalized, "default", theme.base_attrs());
        Ok(View {
            theme: theme.clone(),
            remap: remap.clone(),
            resolved: normalized,
            default_realized,
        })
    }

    /// A rejected buffer remapping must not blank the buffer. Fall back to
    /// the unremapped theme for this buffer and return the diagnostic for the
    /// caller to report. Other buffers are unchanged.
    pub fn bind_or_unremapped(theme: &Theme, remap: &Arc<Remap>) -> (View, Option<FaceError>) {
        match Self::bind(theme, remap) {
            Ok(view) => (view, None),
            Err(error) => (
                Self::bind(theme, &Remap::empty()).expect("empty remap is acyclic"),
                Some(error),
            ),
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }
    pub fn remap(&self) -> &Arc<Remap> {
        &self.remap
    }

    /// GNU's C `face-attributes-as-vector`, starting from unspecified slots.
    pub fn effective(&self, face: &str) -> Option<Attrs> {
        let name = self.theme.canonical_name(face);
        if !self.theme.contains(name) && !self.resolved.entries.contains_key(name) {
            return None;
        }
        Some(render_named(
            &self.theme,
            &self.resolved,
            name,
            &Attrs::default(),
        ))
    }

    pub fn effective_ref(&self, face_ref: &FaceRef) -> Attrs {
        render(
            &self.theme,
            &self.resolved,
            Task::Layers(&face_ref.0),
            &Attrs::default(),
        )
    }

    /// A text-property face reference made absolute by the C merge.
    pub fn realize_ref(&self, face_ref: &FaceRef) -> Attrs {
        render(
            &self.theme,
            &self.resolved,
            Task::Layers(&face_ref.0),
            &self.default_realized,
        )
    }

    /// Replay the C merge over the absolute default, truncating each step.
    pub fn realize(&self, face: &str) -> Option<Attrs> {
        let name = self.theme.canonical_name(face);
        if !self.theme.contains(name) && !self.resolved.entries.contains_key(name) {
            return None;
        }
        if name == "default" {
            return Some(self.default_realized.clone());
        }
        Some(render_named(
            &self.theme,
            &self.resolved,
            name,
            &self.default_realized,
        ))
    }
}

fn layer_names<'a>(layers: &'a [Layer], out: &mut Vec<&'a Arc<str>>) {
    let mut stack: Vec<&Layer> = layers.iter().collect();
    while let Some(layer) = stack.pop() {
        match layer {
            Layer::Named(n) => out.push(n),
            Layer::Anonymous(a) => stack.extend(a.inherit.iter()),
            Layer::Filtered { .. } => {}
        }
    }
}

fn check_remap_cycles(theme: &Theme, remap: &Remap) -> Result<(), FaceError> {
    let mut done = FxHashSet::default();
    let mut open = FxHashSet::default();
    let mut roots: Vec<&str> = remap.entries.keys().map(|s| &**s).collect();
    roots.sort_unstable();
    for root in roots {
        let mut path = Vec::new();
        let mut work = vec![(root, false)];
        while let Some((name, expanded)) = work.pop() {
            if expanded {
                open.remove(name);
                path.pop();
                done.insert(name);
                continue;
            }
            if done.contains(name) {
                continue;
            }
            if !open.insert(name) {
                let i = path.iter().position(|n| *n == name).unwrap_or(0);
                let mut cycle: Vec<String> =
                    path[i..].iter().map(|n: &&str| n.to_string()).collect();
                cycle.push(name.into());
                return Err(FaceError::InheritanceCycle { cycle });
            }
            path.push(name);
            work.push((name, true));
            let mut names = Vec::new();
            layer_names(&remap.entries[name], &mut names);
            for dep in names {
                let dep = theme.canonical_name(dep);
                if dep != name && remap.entries.contains_key(dep) {
                    work.push((dep, false));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointKind {
    Normal,
    Remap,
}

enum Task<'a> {
    Named(&'a str),
    Definition(&'a crate::FaceDef),
    Layers(&'a [Layer]),
    Attrs(&'a Attrs),
    Pop,
    FinishRemap,
}

pub(crate) fn render_named(theme: &Theme, remap: &Remap, face: &str, under: &Attrs) -> Attrs {
    render(theme, remap, Task::Named(face), under)
}

/// Explicit-stack merge_named_face/get_lface_attributes/merge_face_vectors.
/// A REMAP point hides earlier NORMAL points of the same face. A repeated
/// REMAP falls back to the original definition, including transitively.
fn render<'a>(theme: &'a Theme, remap: &'a Remap, first: Task<'a>, under: &Attrs) -> Attrs {
    let mut value = under.clone();
    let mut points: Vec<(&str, PointKind, Option<PointKind>)> = Vec::new();
    let mut latest: FxHashMap<&str, PointKind> = FxHashMap::default();
    let mut remapping: FxHashSet<&str> = FxHashSet::default();
    let mut saved_values = Vec::new();
    let mut work = vec![first];
    while let Some(task) = work.pop() {
        match task {
            Task::Named(name) => {
                let name = theme.canonical_name(name);
                let last = latest.get(name).copied();
                if last == Some(PointKind::Normal) {
                    continue;
                }
                points.push((name, PointKind::Normal, last));
                latest.insert(name, PointKind::Normal);
                work.push(Task::Pop);
                if let Some(layers) = remap
                    .entries
                    .get(name)
                    .filter(|_| !remapping.contains(name))
                {
                    points.push((name, PointKind::Remap, Some(PointKind::Normal)));
                    latest.insert(name, PointKind::Remap);
                    remapping.insert(name);
                    work.push(Task::Pop);
                    saved_values.push(value);
                    work.push(Task::FinishRemap);
                    value = Attrs::default();
                    work.push(Task::Layers(layers));
                } else if let Some(def) = theme.definition(name) {
                    work.push(Task::Definition(def));
                }
            }
            Task::Definition(def) => {
                work.push(Task::Attrs(def.attrs()));
                for parent in def.parents() {
                    work.push(Task::Named(parent));
                }
            }
            Task::Layers(layers) => {
                for layer in layers {
                    match layer {
                        Layer::Named(name) => work.push(Task::Named(name)),
                        Layer::Anonymous(face) => {
                            work.push(Task::Attrs(&face.attrs));
                            work.push(Task::Layers(&face.inherit));
                        }
                        Layer::Filtered { .. } => {}
                    }
                }
            }
            Task::Attrs(attrs) => {
                value = crate::theme::resolve_reset(attrs, theme.default_realized()).merge(&value)
            }
            Task::Pop => {
                let (name, kind, previous) = points.pop().expect("balanced merge points");
                if kind == PointKind::Remap {
                    remapping.remove(name);
                }
                if let Some(previous) = previous {
                    latest.insert(name, previous);
                } else {
                    latest.remove(name);
                }
            }
            Task::FinishRemap => {
                value = value.merge(&saved_values.pop().expect("balanced remap values"))
            }
        }
    }
    value
}
