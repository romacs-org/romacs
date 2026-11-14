//! What an image holds, kind by kind (`Schema`): the walk the listing
//! macros derive from the listing that makes a type `SnapshotSafe`.
//!
//! One walk from an image's root meets every value the image holds through
//! a listed field, an element or a pointee. For each type it meets it
//! records how many values there are, the bytes they take where they lie
//! and the bytes of the allocations they own, and the layout the compiler
//! gave the type's fields: each field's name, type, offset and size, per
//! variant for an enum, as the walk first met it (`Walk::census`). A value
//! shared through an `Rc` or `Arc` is walked once.
//!
//! An observer sees every value the walk meets but leaves, unsized values,
//! and cells and locks (whose contents it sees), and may count notes of its own (`Notes`): what this crate cannot know,
//! such as which values name a primitive by its index in the executable's
//! table. The walk counts
//! one note itself: every empty standard hash table, whose control bytes
//! are a static of the executable.
//!
//! At-rest fields are not walked: an image holds them empty
//! (`world_quiescent`).
mod impls;

use rustc_hash::{FxHashMap, FxHashSet};
use std::any::{Any, TypeId, type_name};
use std::collections::BTreeMap;

/// A type whose values a walk visits field by field. The listing macros
/// implement it; the standard library's and other crates' data types are
/// implemented in `impls`.
pub trait Schema: 'static {
    /// Values with nothing to visit and no allocation: numbers and flags. A
    /// container counts its leaves without meeting each.
    const LEAF: bool = false;
    /// Meet each field, element or pointee of this value through W, and
    /// count the bytes of the allocations it owns (`Walk::own`).
    fn walk(&self, w: &mut Walk<'_>);
    /// This value for an observer, or None when it is unsized.
    fn any(&self) -> Option<&dyn Any>;
}

/// The notes an observer counts, by name.
#[derive(Default)]
pub struct Notes(BTreeMap<&'static str, u64>);

impl Notes {
    /// Count N more of NOTE.
    pub fn add(&mut self, note: &'static str, n: u64) {
        *self.0.entry(note).or_default() += n;
    }
}

/// One field as the compiler laid it out.
struct Field {
    name: &'static str,
    ty: &'static str,
    offset: usize,
    size: usize,
}

/// A struct's fields, or one variant's: how often the walk met it, and its
/// fields as described by the first value met.
#[derive(Default)]
struct Layout {
    count: u64,
    fields: Vec<Field>,
    /// Being described by a value the walk is in, or described already.
    claimed: bool,
}

/// One type the walk met.
struct Kind {
    name: &'static str,
    count: u64,
    /// The bytes its values take where they lie.
    inline: u64,
    /// The bytes of the allocations its values own.
    owned: u64,
    /// By variant, "" for a struct; a few, found by their label's address.
    layouts: Vec<(&'static str, Layout)>,
}

impl Kind {
    /// VARIANT's layout, made when first asked for.
    fn layout(&mut self, variant: &'static str) -> &mut Layout {
        let found = self.layouts.iter().position(|(label, _)| std::ptr::eq(*label, variant) || *label == variant);
        let at = found.unwrap_or_else(|| {
            self.layouts.push((variant, Layout::default()));
            self.layouts.len() - 1
        });
        &mut self.layouts[at].1
    }
}

/// The value the walk is in: its kind, where it lies, and, once its first
/// field or its variant is met, whether it describes that layout.
struct Frame {
    kind: usize,
    at: usize,
    layout: Option<(&'static str, bool)>,
}

/// A walk of an image's values: the census it counts (`census`).
pub struct Walk<'o> {
    index: FxHashMap<TypeId, usize>,
    kinds: Vec<Kind>,
    frames: Vec<Frame>,
    shared: FxHashSet<usize>,
    notes: Notes,
    observe: &'o mut dyn FnMut(&dyn Any, &mut Notes),
}

/// The note for a cell or lock the walk found taken: a VM at rest holds none.
pub const TAKEN: &str = "taken: a cell or lock borrowed during the walk";

/// The note counted for each empty standard hash table.
pub const EMPTY_HASH_TABLE: &str = "executable: an empty hash table's static control bytes";

impl<'o> Walk<'o> {
    /// A walk whose observer is OBSERVE.
    pub fn new(observe: &'o mut dyn FnMut(&dyn Any, &mut Notes)) -> Self {
        Self { index: FxHashMap::default(), kinds: Vec::new(), frames: Vec::new(), shared: FxHashSet::default(), notes: Notes::default(), observe }
    }

    /// Meet VALUE: it is counted, and unless it is a leaf the observer sees
    /// it and it walks its own fields.
    pub fn value<T: Schema + ?Sized>(&mut self, value: &T) {
        let kind = self.kind::<T>();
        self.kinds[kind].count += 1;
        self.kinds[kind].inline += std::mem::size_of_val(value) as u64;
        if T::LEAF {
            return;
        }
        if let Some(any) = value.any() {
            (self.observe)(any, &mut self.notes);
        }
        self.frames.push(Frame { kind, at: std::ptr::from_ref(value).cast::<u8>() as usize, layout: None });
        value.walk(self);
        self.frames.pop();
    }

    /// Meet the field NAME of the value the walk is in, which must be
    /// SnapshotSafe: the listing macros' check of every listed field.
    pub fn field<F: crate::SnapshotSafe + Schema>(&mut self, name: &'static str, value: &F) {
        if let Some((kind, at, variant, true)) = self.layout("") {
            let offset = (std::ptr::from_ref(value).cast::<u8>() as usize).wrapping_sub(at);
            let field = Field { name, ty: type_name::<F>(), offset, size: std::mem::size_of::<F>() };
            self.kinds[kind].layout(variant).fields.push(field);
        }
        self.value(value);
    }

    /// The value the walk is in is the enum variant LABEL.
    pub fn variant(&mut self, label: &'static str) {
        self.layout(label);
    }

    /// Meet each of ITEMS; leaves are counted, not met.
    pub fn elements<'a, T: Schema>(&mut self, items: impl ExactSizeIterator<Item = &'a T>) {
        if T::LEAF {
            let kind = self.kind::<T>();
            let n = items.len() as u64;
            self.kinds[kind].count += n;
            self.kinds[kind].inline += n * std::mem::size_of::<T>() as u64;
        } else {
            for item in items {
                self.value(item);
            }
        }
    }

    /// The value the walk is in owns an allocation of BYTES.
    pub fn own(&mut self, bytes: usize) {
        if let Some(frame) = self.frames.last() {
            self.kinds[frame.kind].owned += bytes as u64;
        }
    }

    /// Whether the allocation at ADDRESS, shared, is met here first.
    pub fn first_share(&mut self, address: usize) -> bool {
        self.shared.insert(address)
    }

    /// Count one NOTE.
    pub fn note(&mut self, note: &'static str) {
        self.notes.add(note, 1);
    }

    /// The census: every kind met, the largest first, with its layouts; the
    /// notes; and the totals.
    #[must_use]
    pub fn census(self) -> serde_json::Value {
        let mut kinds = self.kinds;
        kinds.sort_by(|a, b| (b.owned + b.inline).cmp(&(a.owned + a.inline)).then(a.name.cmp(b.name)));
        let total = |f: fn(&Kind) -> u64| kinds.iter().map(f).sum::<u64>();
        let totals = serde_json::json!({"kinds": kinds.len(), "values": total(|k| k.count), "owned_bytes": total(|k| k.owned),
                                        "shared": self.shared.len()});
        let kinds: Vec<_> = kinds.into_iter().map(|kind| {
            let layouts: serde_json::Map<_, _> = kind.layouts.into_iter().map(|(variant, layout)| {
                let fields: Vec<_> = layout.fields.iter().map(|f| serde_json::json!([f.name, f.ty, f.offset, f.size])).collect();
                (variant.to_owned(), serde_json::json!({"count": layout.count, "fields": fields}))
            }).collect();
            serde_json::json!({"type": kind.name, "count": kind.count, "inline_bytes": kind.inline, "owned_bytes": kind.owned, "layouts": layouts})
        }).collect();
        serde_json::json!({"totals": totals, "notes": self.notes.0, "kinds": kinds})
    }

    /// The kind of T, made when first met.
    fn kind<T: Schema + ?Sized>(&mut self) -> usize {
        let kinds = &mut self.kinds;
        *self.index.entry(TypeId::of::<T>()).or_insert_with(|| {
            kinds.push(Kind { name: type_name::<T>(), count: 0, inline: 0, owned: 0, layouts: Vec::new() });
            kinds.len() - 1
        })
    }

    /// The layout of the value the walk is in, as (kind, where it lies,
    /// variant, whether it describes it): VARIANT's, the first time this is
    /// asked. The first value of a layout met describes it, unless a value
    /// it is inside already does.
    fn layout(&mut self, variant: &'static str) -> Option<(usize, usize, &'static str, bool)> {
        let frame = self.frames.last_mut()?;
        let kind = &mut self.kinds[frame.kind];
        let (variant, describing) = *frame.layout.get_or_insert_with(|| {
            let layout = kind.layout(variant);
            layout.count += 1;
            (variant, !std::mem::replace(&mut layout.claimed, true))
        });
        Some((frame.kind, frame.at, variant, describing))
    }
}
