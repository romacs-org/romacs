//! Face definitions and the one-time collapse of `defface` condition specs.
//!
//! GNU evaluates a spec's display conditions against each frame every time a
//! face is recalculated (`face-spec-choose`, faces.el). Romacs has one render
//! target: a graphical truecolour Wayland display. So `type`, `class`,
//! `min-colors` and `supports` are decided once, here, and the tty,
//! greyscale and monochrome branches are discarded at load.
//!
//! `background` is the one axis that is still live: a theme decides whether
//! the frame is light or dark. A collapsed spec therefore keeps at most two
//! definitions, one per background mode, and usually shares one.

use std::sync::Arc;

use crate::attr::{AttrError, Attribute, Attrs, Slot, validate_raw as validate};
use crate::datum::{BoundedDatum, Datum};

/// The frame background mode, the only display condition decided late.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Background {
    Light,
    Dark,
}

impl Background {
    /// The mode GNU's `frame--current-background-mode` derives from an
    /// exact `#RGB`-style colour: `color-dark-p` (faces.el), sRGB gamma 2.2,
    /// relative luminance below `color-luminance-dark-limit` (0.325).
    /// `None` for colour names, which need a colour database.
    pub fn of_hex_color(spec: &str) -> Option<Background> {
        let hex = spec.strip_prefix('#')?;
        if hex.is_empty()
            || hex.len() % 3 != 0
            || hex.len() > 12
            || !hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        let n = hex.len() / 3;
        let max = (16f64).powi(n as i32) - 1.0;
        let c = |i: usize| -> Option<f64> {
            let v = u64::from_str_radix(&hex[i * n..(i + 1) * n], 16).ok()?;
            Some((v as f64 / max).powf(2.2))
        };
        let y = c(0)? * 0.2126 + c(1)? * 0.7152 + c(2)? * 0.0722;
        Some(if y < 0.325 {
            Background::Dark
        } else {
            Background::Light
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Background::Light => "light",
            Background::Dark => "dark",
        }
    }
}

/// A face's own definition: its attributes and its inheritance edges.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FaceDef {
    pub(crate) attrs: Attrs,
    /// `None` when `:inherit` is unspecified; `Some(vec![])` for `nil`.
    pub(crate) inherit: Option<Vec<Arc<str>>>,
    /// Whether the spec mentioned `:extend`; GNU carries a defface's
    /// `:extend` into a theme spec that leaves it unspecified.
    pub(crate) extend_mentioned: bool,
}

impl FaceDef {
    pub fn attrs(&self) -> &Attrs {
        &self.attrs
    }

    pub fn inherit(&self) -> Option<&[Arc<str>]> {
        self.inherit.as_deref()
    }

    pub(crate) fn parents(&self) -> &[Arc<str>] {
        self.inherit.as_deref().unwrap_or(&[])
    }

    /// Build from an attribute plist with `face-spec-set-2` +
    /// `set-face-attribute` semantics: unknown keywords are dropped, family
    /// and foundry are applied first, the rest in order, a later key
    /// overriding an earlier one.
    pub fn from_plist(face: &str, plist: &BoundedDatum) -> Result<FaceDef, FaceError> {
        let items = plist
            .as_list()
            .ok_or_else(|| spec_error(face, "attributes are not a plist", plist))?;
        Self::from_plist_raw(face, items)
    }

    pub(crate) fn from_plist_raw(face: &str, plist: &[Datum]) -> Result<FaceDef, FaceError> {
        let mut def = FaceDef::default();
        def.apply_plist(face, plist)?;
        Ok(def)
    }

    pub(crate) fn apply_plist(&mut self, face: &str, plist: &[Datum]) -> Result<(), FaceError> {
        let pairs: Vec<(&str, &Datum)> = plist
            .chunks(2)
            .filter_map(|kv| {
                let key = kv[0].as_symbol()?;
                Some((key, kv.get(1).unwrap_or(&Datum::Nil)))
            })
            .collect();
        let mut attrs = std::mem::take(&mut self.attrs);
        // set-face-attribute: :family and :foundry first, last occurrence wins.
        for kw in [":family", ":foundry"] {
            if let Some((_, v)) = pairs.iter().rev().find(|(k, _)| *k == kw) {
                let a = Attribute::from_keyword(kw).expect("known");
                attrs = attrs.with(a, validate(face, a, v)?);
            }
        }
        for (key, value) in pairs {
            match key {
                ":family" | ":foundry" => {}
                ":inherit" => self.inherit = parse_inherit(face, value)?,
                // internal-set-lisp-face-attribute: :bold/:italic just set
                // the weight/slant slot, in order (the set-face-attribute
                // docstring's "ignored if :weight is specified" is not what
                // the C does).
                ":bold" | ":italic" => {
                    let (a, on, off) = if key == ":bold" {
                        (Attribute::Weight, "bold", "normal")
                    } else {
                        (Attribute::Slant, "italic", "normal")
                    };
                    let slot = match value.as_symbol() {
                        Some("reset") => Slot::Reset,
                        _ if value.is_nil() => validate(face, a, &Datum::symbol(off))?,
                        _ => validate(face, a, &Datum::symbol(on))?,
                    };
                    attrs = attrs.with(a, slot);
                }
                // A font object cannot be expressed without a font backend;
                // GNU also ignores :font on a non-window frame. Out of scope
                // for this crate; see GNU-DIVERGENCES.md.
                ":font" => {}
                k => {
                    let Some(a) = Attribute::from_keyword(k) else {
                        continue; // face-spec-set-2 drops unknown keywords
                    };
                    if a == Attribute::Extend {
                        self.extend_mentioned = true;
                    }
                    attrs = attrs.with(a, validate(face, a, value)?);
                }
            }
        }
        self.attrs = attrs;
        Ok(())
    }
}

fn parse_inherit(face: &str, value: &Datum) -> Result<Option<Vec<Arc<str>>>, FaceError> {
    let names: Vec<Arc<str>> = match value {
        Datum::Nil => Vec::new(),
        Datum::Symbol(s) if &**s == "unspecified" => return Ok(None),
        Datum::Symbol(s) => vec![s.clone()],
        Datum::List(items) => {
            let mut v = Vec::with_capacity(items.len());
            for d in items {
                match d {
                    Datum::Symbol(s) => v.push(s.clone()),
                    Datum::Nil => v.push("nil".into()),
                    Datum::T => v.push("t".into()),
                    _ => {
                        return Err(FaceError::InvalidInherit {
                            face: face.into(),
                            value: value.to_lisp(),
                        });
                    }
                }
            }
            v
        }
        _ => {
            return Err(FaceError::InvalidInherit {
                face: face.into(),
                value: value.to_lisp(),
            });
        }
    };
    // A self-reference is a cycle of length one; the theme's topological
    // sort rejects it with every other cycle.
    Ok(Some(names))
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum FaceError {
    #[error(transparent)]
    Attr(#[from] AttrError),
    #[error("invalid face inheritance for `{face}': {value}")]
    InvalidInherit { face: String, value: String },
    /// Raised when a theme, an attribute change or a remapping would close an
    /// inheritance loop. The cycle is listed in order, first face repeated.
    #[error("face inheritance results in inheritance cycle: {}", cycle.join(" -> "))]
    InheritanceCycle { cycle: Vec<String> },
    #[error("invalid face spec for `{face}': {reason}: {spec}")]
    Spec {
        face: String,
        reason: &'static str,
        spec: String,
    },
}

/// A `defface` or theme spec, collapsed for the one render target.
#[derive(Clone, Debug, PartialEq)]
pub struct Collapsed {
    light: Option<Arc<FaceDef>>,
    dark: Option<Arc<FaceDef>>,
}

impl Collapsed {
    /// The definition for `mode`, or `None` when no branch matched (GNU's
    /// `no-match-retval`).
    pub fn get(&self, mode: Background) -> Option<&Arc<FaceDef>> {
        match mode {
            Background::Light => self.light.as_ref(),
            Background::Dark => self.dark.as_ref(),
        }
    }

    /// Whether the two modes share one definition (the common case).
    pub fn is_uniform(&self) -> bool {
        match (&self.light, &self.dark) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    /// Collapse SPEC once. Mirrors `face-spec-choose` and
    /// `face-spec-set-match-display` for a pgtk truecolour frame.
    pub fn collapse(face: &str, spec: &BoundedDatum) -> Result<Collapsed, FaceError> {
        let light = choose(face, spec, Background::Light)?;
        let dark = choose(face, spec, Background::Dark)?;
        let def = |p: Option<Vec<Datum>>| -> Result<Option<Arc<FaceDef>>, FaceError> {
            p.map(|p| FaceDef::from_plist_raw(face, &p).map(Arc::new))
                .transpose()
        };
        if light == dark {
            let shared = def(light)?;
            return Ok(Collapsed {
                dark: shared.clone(),
                light: shared,
            });
        }
        Ok(Collapsed {
            light: def(light)?,
            dark: def(dark)?,
        })
    }

    /// A spec that is already a plain attribute plist for every display.
    pub fn uniform(def: FaceDef) -> Collapsed {
        let d = Arc::new(def);
        Collapsed {
            light: Some(d.clone()),
            dark: Some(d),
        }
    }
}

fn spec_error(face: &str, reason: &'static str, spec: &Datum) -> FaceError {
    FaceError::Spec {
        face: face.into(),
        reason,
        spec: spec.to_lisp(),
    }
}

/// `face-spec-choose` for one background mode: the chosen plist, or `None`.
fn choose(face: &str, spec: &Datum, mode: Background) -> Result<Option<Vec<Datum>>, FaceError> {
    let entries = spec
        .as_list()
        .ok_or_else(|| spec_error(face, "spec is not a list", spec))?;
    let mut defaults: Option<Vec<Datum>> = None;
    let mut result: Option<Vec<Datum>> = None;
    for entry in entries {
        let (display, attrs): (&Datum, Vec<Datum>) = match entry {
            // `(car nil)' is nil, an always-matching display with no attributes.
            Datum::Nil => (&Datum::Nil, Vec::new()),
            Datum::List(items) if !items.is_empty() => (&items[0], items[1..].to_vec()),
            Datum::Dotted(items, _) if items.len() == 1 => (&items[0], Vec::new()),
            _ => return Err(spec_error(face, "spec entry is not a list", entry)),
        };
        // Old-style entry: (DISPLAY ATTR-PLIST).
        let plist = if attrs.len() <= 1 {
            match attrs.first() {
                None | Some(Datum::Nil) => Vec::new(),
                Some(Datum::List(p)) => p.clone(),
                Some(other) => return Err(spec_error(face, "attributes are not a plist", other)),
            }
        } else {
            attrs
        };
        if display.as_symbol() == Some("default") {
            defaults = Some(plist);
        } else if matches_display(face, display, mode)? {
            result = Some(plist);
            break;
        }
    }
    Ok(match (defaults, result) {
        (Some(mut d), r) => {
            d.extend(r.unwrap_or_default());
            Some(d)
        }
        (None, r) => r,
    })
}

/// `face-spec-set-match-display` for the fixed profile: window system
/// `romacs`, treated as GNU's pgtk (with the gtk feature), display class `color`, 2^24 colours, every
/// `supports` query satisfied, and MODE as the background.
fn matches_display(face: &str, display: &Datum, mode: Background) -> Result<bool, FaceError> {
    let conjuncts = match display {
        Datum::T | Datum::Nil => return Ok(true),
        Datum::List(c) => c,
        _ => return Err(spec_error(face, "display is not t or a list", display)),
    };
    for conjunct in conjuncts {
        let (req, options): (&str, &[Datum]) = match conjunct {
            Datum::List(items) if !items.is_empty() => {
                (items[0].as_symbol().unwrap_or(""), &items[1..])
            }
            _ => return Err(spec_error(face, "display conjunct is not a list", conjunct)),
        };
        let has = |name: &str| options.iter().any(|o| o.as_symbol() == Some(name));
        let ok = match req {
            // romacs's own window-system symbol, plus the pgtk build it
            // replaces: specs written for pgtk target exactly this display.
            "type" => has("romacs") || has("pgtk") || has("graphic") || has("gtk"),
            "class" => has("color"),
            "background" => has(mode.name()),
            "min-colors" => match options.first() {
                Some(Datum::Int(n)) => *n <= 1 << 24,
                Some(Datum::Float(n)) => *n <= (1u32 << 24) as f64,
                _ => return Err(spec_error(face, "min-colors needs a number", conjunct)),
            },
            "supports" => true,
            _ => return Err(spec_error(face, "unknown display requirement", conjunct)),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}
