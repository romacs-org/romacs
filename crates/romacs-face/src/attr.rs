//! Face attributes: the immutable record a face is made of.

use std::sync::Arc;

use crate::datum::{BoundedDatum, Datum, float_to_lisp};

/// The sixteen attributes GNU's `face-attribute-name-alist` reports, in its
/// order. `:inherit` is not an attribute here: it is the face graph's edge
/// set, held by [`crate::FaceDef`], and it is resolved away before any
/// record reaches a caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Attribute {
    Family,
    Foundry,
    Width,
    Height,
    Weight,
    Slant,
    Underline,
    Overline,
    Extend,
    StrikeThrough,
    Box,
    InverseVideo,
    Foreground,
    DistantForeground,
    Background,
    Stipple,
}

pub const ATTRIBUTE_COUNT: usize = 16;

impl Attribute {
    pub const ALL: [Attribute; ATTRIBUTE_COUNT] = [
        Attribute::Family,
        Attribute::Foundry,
        Attribute::Width,
        Attribute::Height,
        Attribute::Weight,
        Attribute::Slant,
        Attribute::Underline,
        Attribute::Overline,
        Attribute::Extend,
        Attribute::StrikeThrough,
        Attribute::Box,
        Attribute::InverseVideo,
        Attribute::Foreground,
        Attribute::DistantForeground,
        Attribute::Background,
        Attribute::Stipple,
    ];

    pub fn keyword(self) -> &'static str {
        match self {
            Attribute::Family => ":family",
            Attribute::Foundry => ":foundry",
            Attribute::Width => ":width",
            Attribute::Height => ":height",
            Attribute::Weight => ":weight",
            Attribute::Slant => ":slant",
            Attribute::Underline => ":underline",
            Attribute::Overline => ":overline",
            Attribute::Extend => ":extend",
            Attribute::StrikeThrough => ":strike-through",
            Attribute::Box => ":box",
            Attribute::InverseVideo => ":inverse-video",
            Attribute::Foreground => ":foreground",
            Attribute::DistantForeground => ":distant-foreground",
            Attribute::Background => ":background",
            Attribute::Stipple => ":stipple",
        }
    }

    pub fn from_keyword(kw: &str) -> Option<Attribute> {
        Attribute::ALL.into_iter().find(|a| a.keyword() == kw)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A face height: absolute decipoints, or a scale of whatever it merges onto.
///
/// GNU also accepts a function of the underlying height. A function is code,
/// not data, so it cannot live in an immutable face; see GNU-DIVERGENCES.md.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Height {
    Absolute(i64),
    Relative(f64),
}
romacs_snapshot::snapshot_enum!(Height {
    Height::Absolute(f0) => (f0),
    Height::Relative(f0) => (f0),
});

impl Height {
    /// GNU `merge_face_heights` (xfaces.c): absolute wins; a scale applied to
    /// an absolute height truncates toward zero (the C double-to-integer
    /// conversion); two scales multiply.
    pub fn merge(self, under: Option<Height>) -> Height {
        match (self, under) {
            (Height::Absolute(_), _) | (Height::Relative(_), None) => self,
            (Height::Relative(s), Some(Height::Absolute(h))) => {
                Height::Absolute((s * h as f64) as i64)
            }
            (Height::Relative(s), Some(Height::Relative(t))) => Height::Relative(s * t),
        }
    }
}

/// A specified attribute value. Which variants an attribute admits is
/// enforced by [`validate`]; the record never holds an unvalidated value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A family, foundry or colour name.
    Str(Arc<str>),
    /// A weight, slant or width name from GNU's font.c tables, as given.
    Symbol(Arc<str>),
    Height(Height),
    /// `t` or `nil` for the boolean-ish attributes.
    Bool(bool),
    /// A box line width.
    Int(i64),
    /// A structured underline or box plist, or a stipple bitmap spec.
    Structured(BoundedDatum),
}
romacs_snapshot::snapshot_enum!(Value {
    Value::Str(f0) => (f0),
    Value::Symbol(f0) => (f0),
    Value::Height(f0) => (f0),
    Value::Bool(f0) => (f0),
    Value::Int(f0) => (f0),
    Value::Structured(f0) => (f0),
});

impl Value {
    pub fn to_datum(&self) -> Datum {
        match self {
            Value::Str(s) => Datum::Str(s.clone()),
            // Never `nil' or `t': no font table lists either.
            Value::Symbol(s) => Datum::Symbol(s.clone()),
            Value::Height(Height::Absolute(h)) => Datum::Int(*h),
            Value::Height(Height::Relative(f)) => Datum::Float(*f),
            Value::Bool(true) => Datum::T,
            Value::Bool(false) => Datum::Nil,
            Value::Int(i) => Datum::Int(*i),
            Value::Structured(d) => (**d).clone(),
        }
    }

    pub fn to_lisp(&self) -> String {
        match self {
            Value::Height(Height::Relative(f)) => float_to_lisp(*f),
            other => other.to_datum().to_lisp(),
        }
    }
}

/// One attribute slot of a face record.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Slot {
    #[default]
    Unspecified,
    /// GNU's `reset`: take the default face's value.
    Reset,
    Set(Value),
}
romacs_snapshot::snapshot_enum!(Slot {
    Slot::Unspecified => (),
    Slot::Reset => (),
    Slot::Set(f0) => (f0),
});

impl Slot {
    fn is_relative(&self) -> bool {
        matches!(
            self,
            Slot::Unspecified | Slot::Set(Value::Height(Height::Relative(_)))
        )
    }

    /// GNU `merge-face-attribute`: an unspecified or relative value is
    /// completed by the value beneath it.
    pub fn merge(&self, under: &Slot) -> Slot {
        match (self, under) {
            (Slot::Unspecified, u) => u.clone(),
            (Slot::Set(Value::Height(h)), Slot::Set(Value::Height(u))) => {
                Slot::Set(Value::Height(h.merge(Some(*u))))
            }
            (s, _) => s.clone(),
        }
    }

    pub fn to_lisp(&self) -> String {
        match self {
            Slot::Unspecified => "unspecified".into(),
            Slot::Reset => "reset".into(),
            Slot::Set(v) => v.to_lisp(),
        }
    }
}

/// An immutable attribute record. Built only through [`AttrsBuilder`] or by
/// merging, so every slot holds a validated value.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Attrs {
    slots: [Slot; ATTRIBUTE_COUNT],
}
romacs_snapshot::snapshot_struct!(Attrs { slots });

impl Attrs {
    pub fn get(&self, attr: Attribute) -> &Slot {
        &self.slots[attr.index()]
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|s| matches!(s, Slot::Unspecified))
    }

    /// `self` over `under`, slot by slot, with `merge-face-attribute` rules.
    pub fn merge(&self, under: &Attrs) -> Attrs {
        let mut slots: [Slot; ATTRIBUTE_COUNT] = Default::default();
        for (i, slot) in slots.iter_mut().enumerate() {
            *slot = self.slots[i].merge(&under.slots[i]);
        }
        Attrs { slots }
    }

    /// Whether any slot still needs a value from beneath.
    pub fn has_relative(&self) -> bool {
        self.slots.iter().any(Slot::is_relative)
    }

    pub(crate) fn with(&self, attr: Attribute, slot: Slot) -> Attrs {
        let mut next = self.clone();
        next.set(attr, slot);
        next
    }

    pub(crate) fn set(&mut self, attr: Attribute, slot: Slot) {
        self.slots[attr.index()] = slot;
    }
}

/// The only way to put values into a record from outside a merge.
#[derive(Default)]
pub struct AttrsBuilder {
    attrs: Attrs,
}

impl AttrsBuilder {
    pub fn set(&mut self, attr: Attribute, slot: Slot) -> &mut Self {
        self.attrs.slots[attr.index()] = slot;
        self
    }

    pub fn build(self) -> Attrs {
        self.attrs
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("invalid face attribute {keyword} {value} for face `{face}': {reason}")]
pub struct AttrError {
    pub face: String,
    pub keyword: &'static str,
    pub value: String,
    pub reason: &'static str,
}

/// The CSS weight of `:weight' NAME (one of WEIGHTS), as fontconfig
/// matches it: 400 for `normal' and anything unknown.
pub fn css_weight(name: &str) -> u16 {
    match name {
        "thin" => 100,
        "ultra-light" | "ultralight" | "extra-light" | "extralight" => 200,
        "light" => 300,
        "semi-light" | "semilight" | "demilight" => 350,
        "medium" => 500,
        "semi-bold" | "semibold" | "demibold" | "demi-bold" | "demi" => 600,
        "bold" => 700,
        "extra-bold" | "extrabold" | "ultra-bold" | "ultrabold" => 800,
        "black" | "heavy" | "ultra-heavy" | "ultraheavy" => 900,
        _ => 400,
    }
}

/// The OS/2 width class of `:width' NAME (one of WIDTHS): 5 for `normal'
/// and anything unknown.
pub fn width_class(name: &str) -> u16 {
    match name {
        "ultra-condensed" | "ultracondensed" => 1,
        "extra-condensed" | "extracondensed" => 2,
        "condensed" | "compressed" | "narrow" => 3,
        "semi-condensed" | "semicondensed" | "demicondensed" => 4,
        "semi-expanded" | "semiexpanded" | "demiexpanded" => 6,
        "expanded" => 7,
        "extra-expanded" | "extraexpanded" => 8,
        "ultra-expanded" | "ultraexpanded" | "wide" => 9,
        _ => 5,
    }
}

// GNU font.c weight_table, slant_table, width_table: every accepted name.
const WEIGHTS: &[&str] = &[
    "thin",
    "ultra-light",
    "ultralight",
    "extra-light",
    "extralight",
    "light",
    "semi-light",
    "semilight",
    "demilight",
    "regular",
    "normal",
    "unspecified",
    "book",
    "medium",
    "semi-bold",
    "semibold",
    "demibold",
    "demi-bold",
    "demi",
    "bold",
    "extra-bold",
    "extrabold",
    "ultra-bold",
    "ultrabold",
    "black",
    "heavy",
    "ultra-heavy",
    "ultraheavy",
];
const SLANTS: &[&str] = &[
    "reverse-oblique",
    "ro",
    "reverse-italic",
    "ri",
    "normal",
    "r",
    "unspecified",
    "italic",
    "i",
    "ot",
    "oblique",
    "o",
];
const WIDTHS: &[&str] = &[
    "ultra-condensed",
    "ultracondensed",
    "extra-condensed",
    "extracondensed",
    "condensed",
    "compressed",
    "narrow",
    "semi-condensed",
    "semicondensed",
    "demicondensed",
    "normal",
    "medium",
    "regular",
    "unspecified",
    "semi-expanded",
    "semiexpanded",
    "demiexpanded",
    "expanded",
    "extra-expanded",
    "extraexpanded",
    "ultra-expanded",
    "ultraexpanded",
    "wide",
];

/// VALUE's own name, when it is a symbol TABLE lists.
fn table_name(table: &[&str], value: &Datum) -> Option<Arc<str>> {
    match value {
        Datum::Symbol(name) if table.contains(&&**name) => Some(name.clone()),
        _ => None,
    }
}

/// Validate one attribute value the way GNU's
/// `internal-set-lisp-face-attribute` (xfaces.c) does, returning the slot
/// to store. `face` is needed for the default face's absolute-height rule.
pub fn validate(face: &str, attr: Attribute, value: &BoundedDatum) -> Result<Slot, AttrError> {
    validate_raw(face, attr, value)
}

pub(crate) fn validate_raw(face: &str, attr: Attribute, value: &Datum) -> Result<Slot, AttrError> {
    let err = |reason: &'static str| AttrError {
        face: face.to_owned(),
        keyword: attr.keyword(),
        value: value.to_lisp(),
        reason,
    };
    match value.as_symbol() {
        Some("unspecified") => return Ok(Slot::Unspecified),
        Some("reset") => return Ok(Slot::Reset),
        _ => {}
    }
    let nonempty = |s: &Arc<str>| -> Result<Slot, AttrError> {
        if s.is_empty() {
            Err(err("empty string"))
        } else {
            Ok(Slot::Set(Value::Str(s.clone())))
        }
    };
    use Attribute as A;
    match attr {
        A::Family | A::Foundry => match value {
            Datum::Str(s) => nonempty(s),
            _ => Err(err("not a string")),
        },
        // HANDLE_INVALID_NIL_VALUE: GNU warns and stores `unspecified'.
        A::Foreground | A::DistantForeground | A::Background => match value {
            Datum::Nil => Ok(Slot::Unspecified),
            Datum::Str(s) => nonempty(s),
            _ => Err(err("not a colour string")),
        },
        A::Weight => table_name(WEIGHTS, value)
            .map(|n| Slot::Set(Value::Symbol(n)))
            .ok_or_else(|| err("unknown weight")),
        A::Slant => table_name(SLANTS, value)
            .map(|n| Slot::Set(Value::Symbol(n)))
            .ok_or_else(|| err("unknown slant")),
        A::Width => table_name(WIDTHS, value)
            .map(|n| Slot::Set(Value::Symbol(n)))
            .ok_or_else(|| err("unknown width")),
        A::Height => {
            let h = match value {
                Datum::Int(i) => Height::Absolute(*i),
                Datum::Float(f) => Height::Relative(*f),
                _ => {
                    return Err(err(
                        "not an integer or float (function heights are not data)",
                    ));
                }
            };
            if face == "default" {
                if !matches!(h, Height::Absolute(i) if i > 0) {
                    return Err(err("default face height not absolute and positive"));
                }
            } else if !matches!(h.merge(Some(Height::Absolute(10))), Height::Absolute(i) if i > 0) {
                return Err(err("face height does not produce a positive integer"));
            }
            Ok(Slot::Set(Value::Height(h)))
        }
        A::Underline => match value {
            Datum::Nil => Ok(Slot::Set(Value::Bool(false))),
            Datum::T => Ok(Slot::Set(Value::Bool(true))),
            Datum::Str(s) => nonempty(s),
            Datum::List(_) | Datum::Dotted(..) => Ok(Slot::Set(Value::Structured(
                BoundedDatum::new(value).expect("bounded boundary child"),
            ))),
            _ => Err(err("invalid underline")),
        },
        A::Overline | A::StrikeThrough => match value {
            Datum::Nil => Ok(Slot::Set(Value::Bool(false))),
            Datum::T => Ok(Slot::Set(Value::Bool(true))),
            Datum::Str(s) => nonempty(s),
            // GNU accepts conses here unchecked; keep them as data.
            Datum::List(_) | Datum::Dotted(..) | Datum::Int(_) | Datum::Float(_) => Ok(Slot::Set(
                Value::Structured(BoundedDatum::new(value).expect("bounded boundary child")),
            )),
            _ => Err(err("invalid line value")),
        },
        A::Box => match value {
            Datum::Nil => Ok(Slot::Set(Value::Bool(false))),
            // GNU stores `:box t' as 1.
            Datum::T => Ok(Slot::Set(Value::Int(1))),
            Datum::Int(0) => Err(err("zero box width")),
            Datum::Int(i) => Ok(Slot::Set(Value::Int(*i))),
            Datum::Str(s) => nonempty(s),
            Datum::List(_) | Datum::Dotted(..) => Ok(Slot::Set(Value::Structured(
                BoundedDatum::new(value).expect("bounded boundary child"),
            ))),
            _ => Err(err("invalid box")),
        },
        A::InverseVideo | A::Extend => match value {
            Datum::Nil => Ok(Slot::Set(Value::Bool(false))),
            Datum::T => Ok(Slot::Set(Value::Bool(true))),
            _ => Err(err("not t or nil")),
        },
        A::Stipple => match value {
            Datum::Nil => Ok(Slot::Set(Value::Bool(false))),
            Datum::Str(_) | Datum::List(_) => Ok(Slot::Set(Value::Structured(
                BoundedDatum::new(value).expect("bounded boundary child"),
            ))),
            _ => Err(err("invalid stipple")),
        },
    }
}
