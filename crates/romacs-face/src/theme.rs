//! Themes as immutable values.
//!
//! A [`Theme`] is the fully materialised face table for one set of inputs:
//! the `defface` registry, one theme spec, one explicit [`UserOverrides`]
//! value, any runtime `set-face-attribute` edits, background and frame base.
//! Building collapses specs, checks topology and premerges Lisp queries.
//! Redisplay replays the C merge iteratively on a cache miss; relative heights
//! must be applied stepwise, not pre-multiplied before an absolute base.
//! Nothing about a built theme can change: every "mutation" returns a new
//! theme with a new [`Generation`], so a generation can never describe stale
//! data. Switching themes is replacing one `Theme` (an `Arc`) with another.

use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::attr::{Attribute, Attrs, Height, Slot, Value};
use crate::datum::{BoundedDatum, Datum};
use crate::spec::{Background, Collapsed, FaceDef, FaceError};

/// Identity of one immutable face table. Minted only when a [`Theme`] (or a
/// [`crate::Remap`]) is constructed; never reused, never bumped by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Generation(NonZeroU64);
romacs_snapshot::snapshot_tuple!(Generation(f0));

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

impl Generation {
    pub(crate) fn mint() -> Generation {
        let n = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        Generation(NonZeroU64::new(n).expect("generation counter wrapped"))
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// The `defface` registry: which faces exist and their collapsed specs.
#[derive(Clone, Debug, Default)]
pub struct Deffaces {
    aliases: FxHashMap<Arc<str>, Arc<str>>,
    order: Vec<Arc<str>>,
    specs: FxHashMap<Arc<str>, Collapsed>,
}
romacs_snapshot::snapshot_struct!(Deffaces { aliases, order, specs });

impl Deffaces {
    /// Register a face-alias at the boundary, before the immutable theme is built.
    pub fn alias(&mut self, alias: &str, target: &str) -> Result<(), FaceError> {
        let mut path = vec![alias.to_owned()];
        let mut next = target;
        while let Some(parent) = self.aliases.get(next) {
            if path.iter().any(|n| n == next) {
                path.push(next.into());
                return Err(FaceError::InheritanceCycle { cycle: path });
            }
            path.push(next.into());
            next = parent;
        }
        if next == alias {
            path.push(next.into());
            return Err(FaceError::InheritanceCycle { cycle: path });
        }
        self.aliases.insert(alias.into(), target.into());
        Ok(())
    }

    /// `defface` / `face-spec-set` with `face-defface-spec`: collapse SPEC and
    /// register it. Redefining keeps the face's position.
    pub fn defface(&mut self, name: &str, spec: &BoundedDatum) -> Result<(), FaceError> {
        let name = self.canonical_name(name);
        let collapsed = Collapsed::collapse(name, spec)?;
        let key: Arc<str> = name.into();
        if self.specs.insert(key.clone(), collapsed).is_none() {
            self.order.push(key);
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    fn canonical_name<'a>(&'a self, mut name: &'a str) -> &'a str {
        while let Some(target) = self.aliases.get(name) {
            name = target;
        }
        name
    }

    pub fn get(&self, name: &str) -> Option<&Collapsed> {
        self.specs.get(self.canonical_name(name))
    }
}

/// One authored theme's face specs, collapsed. No Custom priority machinery.
#[derive(Clone, Debug)]
pub struct ThemeSpec {
    name: Arc<str>,
    faces: FxHashMap<Arc<str>, Collapsed>,
}
romacs_snapshot::snapshot_struct!(ThemeSpec { name, faces });

impl ThemeSpec {
    pub fn new(name: &str) -> ThemeSpec {
        ThemeSpec {
            name: name.into(),
            faces: FxHashMap::default(),
        }
    }

    pub fn set_face(&mut self, face: &str, spec: &BoundedDatum) -> Result<(), FaceError> {
        self.faces
            .insert(face.into(), Collapsed::collapse(face, spec)?);
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// One explicit user preference value, not an enabled-theme priority stack.
#[derive(Clone, Debug, Default)]
pub struct UserOverrides {
    faces: FxHashMap<Arc<str>, FaceDef>,
}
romacs_snapshot::snapshot_struct!(UserOverrides { faces });

impl UserOverrides {
    /// Read explicit `((FACE . PLIST) ...)` preferences; no Custom forms.
    pub fn from_alist(alist: &BoundedDatum) -> Result<Self, FaceError> {
        let malformed = || FaceError::Spec {
            face: String::new(),
            reason: "override must be ((FACE . PLIST) ...)",
            spec: alist.to_lisp(),
        };
        let entries = alist.as_list().ok_or_else(malformed)?;
        let mut user = Self::default();
        for entry in entries {
            let items = entry.as_list().ok_or_else(malformed)?;
            let name = items
                .first()
                .and_then(Datum::as_symbol)
                .ok_or_else(malformed)?;
            if user.faces.contains_key(name) {
                return Err(malformed());
            }
            // Subtrees and list suffixes of a bounded value remain bounded.
            let plist = BoundedDatum::new(&Datum::list(items[1..].to_vec()))
                .expect("bounded preference suffix");
            user.set_face(name, &plist)?;
        }
        Ok(user)
    }

    pub fn set_face(&mut self, face: &str, plist: &crate::BoundedDatum) -> Result<(), FaceError> {
        let items = plist.as_list().ok_or_else(|| FaceError::Spec {
            face: face.into(),
            reason: "override is not a plist",
            spec: plist.to_lisp(),
        })?;
        self.faces
            .insert(face.into(), FaceDef::from_plist_raw(face, items)?);
        Ok(())
    }
}

/// The frame's base attributes beneath the default face: fully specified and
/// absolute, so every realised face is too. The one exception is
/// `:distant-foreground`, which GNU's default face also leaves unspecified
/// (it is absent from `face-attribute-name-alist`).
#[derive(Clone, Debug, PartialEq)]
pub struct Base(Attrs);
romacs_snapshot::snapshot_tuple!(Base(f0));

impl Base {
    pub fn new(attrs: Attrs) -> Result<Base, FaceError> {
        for a in Attribute::ALL {
            if a == Attribute::DistantForeground {
                continue;
            }
            match attrs.get(a) {
                Slot::Set(Value::Height(Height::Relative(_))) | Slot::Unspecified | Slot::Reset => {
                    return Err(FaceError::Spec {
                        face: "default".into(),
                        reason: "frame base attribute is not absolute",
                        spec: a.keyword().into(),
                    });
                }
                Slot::Set(_) => {}
            }
        }
        Ok(Base(attrs))
    }
}

/// How far `face-attribute` should resolve: GNU's INHERIT argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inherit {
    /// `nil`: the face's own value.
    No,
    /// `t`: merged along `:inherit`.
    Yes,
    /// `default`: merged with the default face too; always absolute.
    Default,
}

#[derive(Clone, Debug)]
pub(crate) struct Face {
    pub(crate) def: Arc<FaceDef>,
    /// The face's own attributes merged along its inheritance, resolved once.
    pub(crate) inherited: Attrs,
}
romacs_snapshot::snapshot_struct!(Face { def, inherited });

#[derive(Clone, Debug)]
struct Inputs {
    deffaces: Arc<Deffaces>,
    spec: Option<Arc<ThemeSpec>>,
    overrides: Arc<FxHashMap<Arc<str>, Vec<Datum>>>,
    override_order: Arc<Vec<Arc<str>>>,
    user: Arc<UserOverrides>,
    background: Background,
    base: Arc<Base>,
}
romacs_snapshot::snapshot_struct!(Inputs { deffaces, spec, overrides, override_order, user, background, base });

#[derive(Debug)]
struct ThemeInner {
    generation: Generation,
    inputs: Inputs,
    order: Vec<Arc<str>>,
    faces: FxHashMap<Arc<str>, Face>,
    default_realized: Attrs,
}
romacs_snapshot::snapshot_struct!(ThemeInner { generation, inputs, order, faces, default_realized });

/// An immutable face table and C merge context. Cheap to clone and switch.
#[derive(Clone, Debug)]
pub struct Theme(Arc<ThemeInner>);
romacs_snapshot::snapshot_tuple!(Theme(f0));

impl Theme {
    /// Build a theme. Fails, before anything is published, if any spec is
    /// invalid or the inheritance graph has a cycle.
    pub fn build(
        deffaces: Arc<Deffaces>,
        spec: Option<Arc<ThemeSpec>>,
        background: Background,
        base: Base,
    ) -> Result<Theme, FaceError> {
        Theme::from_inputs(Inputs {
            deffaces,
            spec,
            overrides: Arc::default(),
            override_order: Arc::default(),
            user: Arc::default(),
            background,
            base: Arc::new(base),
        })
    }

    fn from_inputs(mut inputs: Inputs) -> Result<Theme, FaceError> {
        if let Some(spec) = &inputs.spec {
            inputs.spec = Some(Arc::new(ThemeSpec {
                name: spec.name.clone(),
                faces: canonical_faces(&inputs.deffaces, &spec.faces)?,
            }));
        }
        inputs.user = Arc::new(UserOverrides {
            faces: canonical_faces(&inputs.deffaces, &inputs.user.faces)?,
        });
        let mut order: Vec<Arc<str>> = inputs.deffaces.order.clone();
        let mut extra: Vec<Arc<str>> = inputs.user.faces.keys().cloned().collect();
        extra.sort();
        extra.extend(inputs.override_order.iter().cloned());
        for name in extra {
            if !order.contains(&name) {
                order.push(name);
            }
        }
        let mut defs: FxHashMap<Arc<str>, Arc<FaceDef>> = FxHashMap::default();
        for name in &order {
            let def = select(&inputs, name)?;
            defs.insert(name.clone(), def);
        }
        let (topo, _) = topological(&order, &defs)?;
        let mut faces: FxHashMap<Arc<str>, Face> = FxHashMap::default();
        for name in &topo {
            let def = defs[name].clone();
            let mut inherited = def.attrs.clone();
            for parent in def.parents() {
                if let Some(p) = faces.get(parent) {
                    inherited = inherited.merge(&p.inherited);
                }
            }
            faces.insert(name.clone(), Face { def, inherited });
        }
        let default_realized = match faces.get("default") {
            Some(d) => d.inherited.merge(&inputs.base.0),
            None => inputs.base.0.clone(),
        };
        let default_realized = resolve_reset(&default_realized, &inputs.base.0);
        let mut theme = Theme(Arc::new(ThemeInner {
            generation: Generation::mint(),
            inputs,
            order,
            faces,
            default_realized,
        }));
        let default = crate::remap::render_named(
            &theme,
            &crate::Remap::empty(),
            "default",
            theme.base_attrs(),
        );
        Arc::get_mut(&mut theme.0)
            .expect("unpublished theme")
            .default_realized = default;
        Ok(theme)
    }

    pub fn build_with_overrides(
        deffaces: Arc<Deffaces>,
        spec: Option<Arc<ThemeSpec>>,
        user: Arc<UserOverrides>,
        background: Background,
        base: Base,
    ) -> Result<Theme, FaceError> {
        Self::from_inputs(Inputs {
            deffaces,
            spec,
            user,
            overrides: Arc::default(),
            override_order: Arc::default(),
            background,
            base: Arc::new(base),
        })
    }

    pub fn generation(&self) -> Generation {
        self.0.generation
    }

    pub fn background(&self) -> Background {
        self.0.inputs.background
    }

    pub fn theme_name(&self) -> Option<&str> {
        self.0.inputs.spec.as_deref().map(ThemeSpec::name)
    }

    pub fn face_names(&self) -> impl Iterator<Item = &str> {
        self.0.order.iter().map(|n| &**n)
    }

    /// Resolve aliases once at the named-face boundary. Alias registration is acyclic.
    pub fn canonical_name<'a>(&'a self, face: &'a str) -> &'a str {
        self.0.inputs.deffaces.canonical_name(face)
    }

    pub fn contains(&self, face: &str) -> bool {
        self.0.faces.contains_key(self.canonical_name(face))
    }

    pub fn len(&self) -> usize {
        self.0.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.faces.is_empty()
    }

    /// The face's own definition after spec selection and overrides.
    pub fn definition(&self, face: &str) -> Option<&FaceDef> {
        self.0.faces.get(self.canonical_name(face)).map(|f| &*f.def)
    }

    pub(crate) fn base_attrs(&self) -> &Attrs {
        &self.0.inputs.base.0
    }

    pub(crate) fn default_realized(&self) -> &Attrs {
        &self.0.default_realized
    }

    /// `face-attribute` without remapping (GNU's Lisp `face-attribute` does
    /// not consult `face-remapping-alist` either). `None` if FACE is unknown.
    pub fn face_attribute(&self, face: &str, attr: Attribute, inherit: Inherit) -> Option<Slot> {
        let f = self.0.faces.get(self.canonical_name(face))?;
        Some(match inherit {
            Inherit::No => f.def.attrs.get(attr).clone(),
            Inherit::Yes => f.inherited.get(attr).clone(),
            Inherit::Default => realize_over(&f.inherited, &self.0.default_realized)
                .get(attr)
                .clone(),
        })
    }

    /// The face's attributes made absolute over the default face and base.
    pub fn realize(&self, face: &str) -> Option<Attrs> {
        let face = self.canonical_name(face);
        self.0.faces.get(face)?;
        if face == "default" {
            return Some(self.0.default_realized.clone());
        }
        Some(crate::remap::render_named(
            self,
            &crate::Remap::empty(),
            face,
            &self.0.default_realized,
        ))
    }

    /// `set-face-attribute FACE nil ATTR VALUE`: a new theme with the change.
    /// Rejects an `:inherit` that would close a cycle, with GNU's message.
    pub fn with_face_attribute(
        &self,
        face: &str,
        keyword: &str,
        value: &BoundedDatum,
    ) -> Result<Theme, FaceError> {
        self.with_face_attributes([(face, keyword, value)])
    }

    /// [`Theme::with_face_attribute`] for each of EDITS in order, built once:
    /// the theme the edits make one at a time when every one succeeds.
    pub fn with_face_attributes<'e>(
        &self,
        edits: impl IntoIterator<Item = (&'e str, &'e str, &'e BoundedDatum)>,
    ) -> Result<Theme, FaceError> {
        let mut inputs = self.0.inputs.clone();
        let mut overrides = (*inputs.overrides).clone();
        let mut order = (*inputs.override_order).clone();
        for (face, keyword, value) in edits {
            let key: Arc<str> = self.canonical_name(face).into();
            let entry = overrides.entry(key.clone()).or_default();
            entry.push(Datum::symbol(keyword));
            entry.push((**value).clone());
            if !order.contains(&key) {
                order.push(key);
            }
        }
        inputs.overrides = Arc::new(overrides);
        inputs.override_order = Arc::new(order);
        Theme::from_inputs(inputs)
    }

    /// `defface` after the theme was built (a package loaded): a new theme.
    pub fn with_defface(&self, face: &str, spec: &BoundedDatum) -> Result<Theme, FaceError> {
        let mut inputs = self.0.inputs.clone();
        let mut deffaces = (*inputs.deffaces).clone();
        deffaces.defface(face, spec)?;
        inputs.deffaces = Arc::new(deffaces);
        Theme::from_inputs(inputs)
    }

    /// Compose another theme spec with the same explicit user preferences.
    /// Transient runtime attribute edits are discarded; the user value stays.
    pub fn with_theme_spec(
        &self,
        spec: Option<Arc<ThemeSpec>>,
        background: Background,
    ) -> Result<Theme, FaceError> {
        let mut inputs = self.0.inputs.clone();
        inputs.spec = spec;
        inputs.background = background;
        inputs.overrides = Arc::default();
        inputs.override_order = Arc::default();
        Theme::from_inputs(inputs)
    }
}

/// Merge a face's inherited attributes over the realised default face.
pub(crate) fn realize_over(inherited: &Attrs, default_realized: &Attrs) -> Attrs {
    resolve_reset(&inherited.merge(default_realized), default_realized)
}

/// `reset` means "the default face's value" (xfaces.c merge_named_face).
pub(crate) fn resolve_reset(attrs: &Attrs, default_realized: &Attrs) -> Attrs {
    let mut out = attrs.clone();
    for a in Attribute::ALL {
        if matches!(attrs.get(a), Slot::Reset) {
            out = out.with(a, default_realized.get(a).clone());
        }
    }
    out
}

/// `face-spec-recalc` for one face: the theme spec if it matches this
/// background, else the defface spec; a theme spec that leaves `:extend`
/// unspecified inherits the defface's; then `set-face-attribute` overrides.
fn select(inputs: &Inputs, name: &Arc<str>) -> Result<Arc<FaceDef>, FaceError> {
    let mode = inputs.background;
    let defface = inputs.deffaces.specs.get(name).and_then(|c| c.get(mode));
    let themed = inputs
        .spec
        .as_ref()
        .and_then(|s| s.faces.get(name))
        .and_then(|c| c.get(mode));
    let mut def: Arc<FaceDef> = match (themed, defface) {
        (Some(t), Some(d))
            if matches!(t.attrs.get(Attribute::Extend), Slot::Unspecified)
                && d.extend_mentioned =>
        {
            let mut merged = (**t).clone();
            merged.attrs = merged
                .attrs
                .with(Attribute::Extend, d.attrs.get(Attribute::Extend).clone());
            Arc::new(merged)
        }
        (Some(t), _) => t.clone(),
        (None, Some(d)) => d.clone(),
        (None, None) => Arc::new(FaceDef::default()),
    };
    if name.as_ref() == "default" {
        // face-spec-reset-face on a graphical frame: these start at nil.
        let mut reset = (*def).clone();
        if reset.inherit.is_none() {
            reset.inherit = Some(Vec::new());
        }
        for a in [
            Attribute::Underline,
            Attribute::Overline,
            Attribute::StrikeThrough,
            Attribute::Box,
            Attribute::InverseVideo,
            Attribute::Stipple,
            Attribute::Extend,
        ] {
            if matches!(reset.attrs.get(a), Slot::Unspecified) {
                reset.attrs = reset.attrs.with(a, Slot::Set(Value::Bool(false)));
            }
        }
        def = Arc::new(reset);
    }
    if let Some(user) = inputs.user.faces.get(name) {
        let mut composed = (*def).clone();
        composed.attrs = user.attrs.merge(&composed.attrs);
        if user.inherit.is_some() {
            composed.inherit = user.inherit.clone();
        }
        def = Arc::new(composed);
    }
    if let Some(plist) = inputs.overrides.get(name) {
        let mut over = (*def).clone();
        over.apply_plist(name, plist)?;
        def = Arc::new(over);
    }
    let mut canonical = (*def).clone();
    if let Some(parents) = &mut canonical.inherit {
        for parent in parents {
            while let Some(target) = inputs.deffaces.aliases.get(parent) {
                *parent = target.clone();
            }
        }
    }
    Ok(Arc::new(canonical))
}

fn canonical_faces<T: Clone>(
    registry: &Deffaces,
    faces: &FxHashMap<Arc<str>, T>,
) -> Result<FxHashMap<Arc<str>, T>, FaceError> {
    let mut canonical = FxHashMap::default();
    let mut names: Vec<&Arc<str>> = faces.keys().collect();
    names.sort();
    for name in names {
        let target: Arc<str> = registry.canonical_name(name).into();
        if canonical.insert(target, faces[name].clone()).is_some() {
            return Err(FaceError::Spec {
                face: name.to_string(),
                reason: "duplicate face after alias resolution",
                spec: name.to_string(),
            });
        }
    }
    Ok(canonical)
}

type Topology = (Vec<Arc<str>>, FxHashMap<Arc<str>, Vec<Arc<str>>>);

/// Kahn's algorithm over the `:inherit` graph. Iterative; a face whose
/// parents never all resolve is on, or downstream of, a cycle, which is
/// then extracted and reported. Edges to undefined faces are ignored, as
/// GNU's C merge ignores them.
fn topological(
    order: &[Arc<str>],
    defs: &FxHashMap<Arc<str>, Arc<FaceDef>>,
) -> Result<Topology, FaceError> {
    let mut pending: FxHashMap<Arc<str>, usize> = FxHashMap::default();
    let mut children: FxHashMap<Arc<str>, Vec<Arc<str>>> = FxHashMap::default();
    let mut queue: VecDeque<Arc<str>> = VecDeque::new();
    for name in order {
        let mut seen: FxHashSet<&str> = FxHashSet::default();
        let parents: Vec<&Arc<str>> = defs[name]
            .parents()
            .iter()
            .filter(|p| defs.contains_key(*p) && seen.insert(p))
            .collect();
        for p in &parents {
            children.entry((*p).clone()).or_default().push(name.clone());
        }
        pending.insert(name.clone(), parents.len());
        if parents.is_empty() {
            queue.push_back(name.clone());
        }
    }
    let mut topo = Vec::with_capacity(order.len());
    while let Some(name) = queue.pop_front() {
        for child in children.get(&name).map(Vec::as_slice).unwrap_or(&[]) {
            let n = pending.get_mut(child).expect("every face has a count");
            *n -= 1;
            if *n == 0 {
                queue.push_back(child.clone());
            }
        }
        topo.push(name);
    }
    if topo.len() == order.len() {
        return Ok((topo, children));
    }
    // Some face is stuck: walk unresolved parents until a face repeats.
    let stuck: FxHashSet<&Arc<str>> = pending
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(k, _)| k)
        .collect();
    let start = order
        .iter()
        .find(|n| stuck.contains(n))
        .expect("a stuck face exists");
    let mut path: Vec<&Arc<str>> = vec![start];
    loop {
        let here = *path.last().expect("non-empty");
        let next = defs[here]
            .parents()
            .iter()
            .find(|p| stuck.contains(p))
            .expect("a stuck face has a stuck parent");
        if let Some(i) = path.iter().position(|n| *n == next) {
            let mut cycle: Vec<String> = path[i..].iter().map(|n| n.to_string()).collect();
            cycle.push(next.to_string());
            return Err(FaceError::InheritanceCycle { cycle });
        }
        path.push(next);
    }
}

/// The active theme: switching is replacing a value, O(1).
#[derive(Clone, Debug)]
pub struct ActiveTheme {
    current: Theme,
}

impl ActiveTheme {
    pub fn new(theme: Theme) -> ActiveTheme {
        ActiveTheme { current: theme }
    }

    pub fn get(&self) -> &Theme {
        &self.current
    }

    /// Install NEXT and return the previous theme.
    pub fn switch(&mut self, next: Theme) -> Theme {
        std::mem::replace(&mut self.current, next)
    }
}
