//! Semantic palettes: named colours and mappings resolved once.
//!
//! The `rum` theme is built on modus, whose model is already
//! "generate a palette, override it semantically, apply it as a unit". A
//! palette entry is either a colour, `unspecified`, or the name of another
//! entry (a mapping such as `(comment fg-dim)`); the effective palette is
//! the override layer, then the user layer, then the core layer, first
//! entry winning (modus's `(append overrides user core)` read by
//! `alist-get`).
//!
//! modus resolves mappings by recursion each time a face is built
//! (`modus-themes--retrieve-palette-value`), so a mapping loop is unbounded
//! Lisp recursion. Here every entry is resolved once at construction,
//! iteratively, and a loop is rejected with its members named.

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::datum::{BoundedDatum, Datum};
use crate::theme::Generation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteValue {
    Color(Arc<str>),
    Unspecified,
    /// A mapping to another palette entry.
    Ref(Arc<str>),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaletteError {
    #[error("palette mapping loop: {}", .0.join(" -> "))]
    Cycle(Vec<String>),
    #[error("malformed palette entry: {0}")]
    Malformed(String),
}

/// One layer: `((NAME VALUE) ...)`, as modus writes palettes.
pub fn parse_layer(alist: &BoundedDatum) -> Result<Vec<(Arc<str>, PaletteValue)>, PaletteError> {
    let items = alist
        .as_list()
        .ok_or_else(|| PaletteError::Malformed(alist.to_lisp()))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Datum::List(pair) = item else {
            return Err(PaletteError::Malformed(item.to_lisp()));
        };
        let name = pair
            .first()
            .and_then(Datum::as_symbol)
            .ok_or_else(|| PaletteError::Malformed(item.to_lisp()))?;
        // `(car (alist-get name palette))`: a missing value reads as nil.
        let value = match pair.get(1) {
            Some(Datum::Str(s)) => PaletteValue::Color(s.clone()),
            Some(Datum::Symbol(s)) if &**s == "unspecified" => PaletteValue::Unspecified,
            Some(Datum::Symbol(s)) => PaletteValue::Ref(s.clone()),
            Some(Datum::T) => PaletteValue::Ref("t".into()),
            _ => PaletteValue::Unspecified,
        };
        out.push((name.into(), value));
    }
    Ok(out)
}

/// A resolved palette: every name maps to a colour or `unspecified`.
#[derive(Debug)]
pub struct Palette {
    generation: Generation,
    order: Vec<Arc<str>>,
    resolved: FxHashMap<Arc<str>, Option<Arc<str>>>,
}

impl Palette {
    /// Layers in precedence order (overrides first).
    pub fn layered(layers: &[&[(Arc<str>, PaletteValue)]]) -> Result<Palette, PaletteError> {
        let mut effective: FxHashMap<Arc<str>, PaletteValue> = FxHashMap::default();
        let mut order: Vec<Arc<str>> = Vec::new();
        for layer in layers {
            for (name, value) in layer.iter() {
                if !effective.contains_key(name) {
                    effective.insert(name.clone(), value.clone());
                    order.push(name.clone());
                }
            }
        }
        let mut resolved: FxHashMap<Arc<str>, Option<Arc<str>>> = FxHashMap::default();
        for start in &order {
            if resolved.contains_key(start) {
                continue;
            }
            // Follow the mapping chain iteratively; then fill the whole chain.
            let mut chain: Vec<Arc<str>> = vec![start.clone()];
            let mut on_chain: FxHashSet<Arc<str>> = FxHashSet::from_iter([start.clone()]);
            let value = loop {
                let here = chain.last().expect("non-empty");
                if let Some(done) = resolved.get(here) {
                    break done.clone();
                }
                match &effective[here] {
                    PaletteValue::Color(c) => break Some(c.clone()),
                    PaletteValue::Unspecified => break None,
                    // modus: a symbol that is not a palette key is unspecified.
                    PaletteValue::Ref(next) if !effective.contains_key(next) => break None,
                    PaletteValue::Ref(next) => {
                        if !on_chain.insert(next.clone()) {
                            let i = chain.iter().position(|n| n == next).unwrap_or(0);
                            let mut cycle: Vec<String> =
                                chain[i..].iter().map(|n| n.to_string()).collect();
                            cycle.push(next.to_string());
                            return Err(PaletteError::Cycle(cycle));
                        }
                        chain.push(next.clone());
                    }
                }
            };
            for name in chain {
                resolved.insert(name, value.clone());
            }
        }
        Ok(Palette {
            generation: Generation::mint(),
            order,
            resolved,
        })
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// `Some(None)` for an entry that resolves to `unspecified`.
    pub fn get(&self, name: &str) -> Option<Option<&str>> {
        self.resolved.get(name).map(|v| v.as_deref())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(|n| &**n)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}
