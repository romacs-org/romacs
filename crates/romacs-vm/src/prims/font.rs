//! GNU font.c's font objects over the open display's fonts
//! (display_host.rs, DIVERGENCES.md VM2-FONT1). A font-spec, font-entity
//! or font-object is a record whose type is that symbol, its slots GNU's
//! property vector (font.h FONT_TYPE_INDEX ... FONT_EXTRA_INDEX), a
//! font-object followed by its name, full name, file and metrics. On the
//! initial terminal no font opens, as on GNU's.
use super::*;
use crate::display_host::{FontEntity, FontQuery, OpenedFont};
use crate::prim::leaf::Kind;

// font.h font_property_index, as record slots (the type is slot 0).
const TYPE: usize = 1;
const FOUNDRY: usize = 2;
const FAMILY: usize = 3;
const ADSTYLE: usize = 4;
const REGISTRY: usize = 5;
pub(crate) const WEIGHT: usize = 6;
pub(crate) const SLANT: usize = 7;
pub(crate) const WIDTH: usize = 8;
const SIZE: usize = 9;
const DPI: usize = 10;
const SPACING: usize = 11;
const AVGWIDTH: usize = 12;
const EXTRA: usize = 13;
const SPEC_SLOTS: usize = 14;
const ENTITY_SLOTS: usize = 15;
const NAME: usize = 15;
const FULLNAME: usize = 16;
const FILE: usize = 17;
// A font-object's metrics (GNU's struct font).
const ASCENT: usize = 18;
const DESCENT: usize = 19;
const MAX_WIDTH: usize = 20;
const SPACE_WIDTH: usize = 21;
const AVERAGE_WIDTH: usize = 22;
const CAPABILITY: usize = 23;
const OBJECT_SLOTS: usize = 24;

/// font.c font_property_table's keys in index order: the first twelve are
/// slots, the rest live in the extra alist.
const PROPERTIES: [&str; 15] = [":type", ":foundry", ":family", ":adstyle", ":registry", ":weight", ":slant",
    ":width", ":size", ":dpi", ":spacing", ":avgwidth", ":lang", ":script", ":otf"];
const EXTRA_INDEX: usize = 12;

/// font.c weight_table, slant_table and width_table: numeric value and
/// names, the first the canonical one.
type StyleTable = &'static [(i64, &'static [&'static str])];
const WEIGHTS: StyleTable = &[(0, &["thin"]), (40, &["ultra-light", "ultralight", "extra-light", "extralight"]),
    (50, &["light"]), (55, &["semi-light", "semilight", "demilight"]), (80, &["regular", "normal", "unspecified", "book"]),
    (100, &["medium"]), (180, &["semi-bold", "semibold", "demibold", "demi-bold", "demi"]), (200, &["bold"]),
    (205, &["extra-bold", "extrabold", "ultra-bold", "ultrabold"]), (210, &["black", "heavy"]),
    (250, &["ultra-heavy", "ultraheavy"])];
const SLANTS: StyleTable = &[(0, &["reverse-oblique", "ro"]), (10, &["reverse-italic", "ri"]),
    (100, &["normal", "r", "unspecified"]), (200, &["italic", "i", "ot"]), (210, &["oblique", "o"])];
const WIDTHS: StyleTable = &[(50, &["ultra-condensed", "ultracondensed"]),
    (63, &["extra-condensed", "extracondensed"]), (75, &["condensed", "compressed", "narrow"]),
    (87, &["semi-condensed", "semicondensed", "demicondensed"]), (100, &["normal", "medium", "regular", "unspecified"]),
    (113, &["semi-expanded", "semiexpanded", "demiexpanded"]), (125, &["expanded"]),
    (150, &["extra-expanded", "extraexpanded"]), (200, &["ultra-expanded", "ultraexpanded", "wide"])];

fn table(slot: usize) -> StyleTable {
    match slot { WEIGHT => WEIGHTS, SLANT => SLANTS, _ => WIDTHS }
}
fn encode(table: StyleTable, row: usize, name: usize) -> i64 { (table[row].0 << 8) | ((row as i64) << 4) | name as i64 }

/// font_style_to_value for a number: exact, else with NOERROR the closest.
fn style_by_number(table: StyleTable, numeric: i64, noerror: bool) -> Option<i64> {
    let mut last: Option<usize> = None;
    for (row, (n, _)) in table.iter().enumerate() {
        if numeric == *n { return Some(encode(table, row, 0)); }
        if numeric < *n {
            if !noerror { return None; }
            return Some(match last {
                Some(prev) if numeric - table[prev].0 <= n - numeric => encode(table, prev, 0),
                _ => encode(table, row, 0),
            });
        }
        last = Some(row);
    }
    if noerror { last.map(|row| encode(table, row, 0)) } else { None }
}
/// The style names font_style_to_value registered past TABLE's own rows,
/// one per row, numeric value 100, as GNU appends them to its table.
const EXTRAS: &str = "#:vm2-font-style-extras";
fn extras(cx: &mut Leaf<'_>, slot: usize) -> Vec<String> {
    let mut list = cx.hidden_value(&format!("{EXTRAS}-{slot}"));
    let mut names = Vec::new();
    while let Some((name, rest)) = cx.pair(list) { names.push(cx.symbol_name(name).unwrap_or_default()); list = rest; }
    names
}
/// font_style_to_value for a symbol: the table's rows, then the names
/// registered past them, exactly and then case-insensitively; with
/// NOERROR an unknown name is registered.
pub(crate) fn style_value(cx: &mut Leaf<'_>, slot: usize, name: &str, noerror: bool) -> Option<i64> {
    let table = table(slot);
    let extra = extras(cx, slot);
    let row_of = |same: &dyn Fn(&str) -> bool| table.iter().enumerate()
        .find_map(|(row, (_, names))| names.iter().position(|n| same(n)).map(|j| encode(table, row, j)))
        .or_else(|| extra.iter().position(|n| same(n)).map(|k| (100 << 8) | (((table.len() + k) as i64) << 4)));
    if let Some(n) = row_of(&|n| n == name).or_else(|| row_of(&|n| n.eq_ignore_ascii_case(name))) { return Some(n); }
    if !noerror { return None; }
    let row = table.len() + extra.len();
    let key = format!("{EXTRAS}-{slot}");
    let mut list = Value::NIL;
    for name in extra.iter().map(String::as_str).chain([name]).collect::<Vec<_>>().into_iter().rev() {
        let symbol = cx.intern(name);
        list = cx.cons(symbol, list);
    }
    cx.set_hidden_value(&key, list);
    Some((100 << 8) | ((row as i64) << 4))
}
/// font_style_symbolic: the name an encoded style value stands for.
// FONT_WEIGHT_NAME_NUMERIC and its kin, as xfaces.c's merge_face_ref tests a
// face's :weight, :slant or :width: whether NAME is a style KIND has.
defsubr! { name = "romacs--font-style-p", gnu = "romacs:font", effect = Reads,
fn romacs_font_style_p(cx, kind: Value, name: Value) {
    let slot = match cx.symbol_name(kind)?.as_str() {
        "weight" => WEIGHT,
        "slant" => SLANT,
        "width" => WIDTH,
        _ => return Err(Fault::Wrong("font-style-kind", kind)),
    };
    if name == Value::NIL || cx.kind(name) != Kind::Symbol { return Ok(Value::NIL); }
    let name = cx.symbol_name(name)?;
    Ok(if style_value(cx, slot, &name, false).is_some() { Value::TRUE } else { Value::NIL })
}}

fn style_name(cx: &mut Leaf<'_>, slot: usize, value: Value) -> Option<String> {
    let n = value.integer_value()?;
    let (row, name) = (((n >> 4) & 0xF) as usize, (n & 0xF) as usize);
    let table = table(slot);
    match table.get(row) {
        Some((_, names)) => names.get(name).map(|name| (*name).to_owned()),
        None => extras(cx, slot).into_iter().nth(row - table.len()).filter(|_| name == 0),
    }
}
fn style_symbol(cx: &mut Leaf<'_>, slot: usize, value: Value) -> Value {
    match style_name(cx, slot, value) { Some(name) => cx.intern(&name), None => Value::NIL }
}
fn style_numeric(value: Value) -> Option<i64> { value.integer_value().map(|n| n >> 8) }

/// signal_error: `error' with MESSAGE and ARG, ARG a one-element list
/// unless it is a proper list.
fn signal_error(cx: &mut Leaf<'_>, message: &str, arg: Value) -> Fault {
    let mut tail = arg;
    let proper = loop {
        match cx.pair(tail) { Some((_, rest)) => tail = rest, None => break tail == Value::NIL }
    };
    let arg = if proper { arg } else { cx.cons(arg, Value::NIL) };
    let message = cx.string(message);
    let data = cx.cons(message, arg);
    Fault::Signal(cx.intern("error"), data)
}
fn error(_cx: &mut Leaf<'_>, message: String) -> Fault { Fault::Error(message) }

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind3 { Spec, Entity, Object }
fn font_kind(cx: &mut Leaf<'_>, value: Value) -> Option<Kind3> {
    let slots = cx.record_slots(value)?;
    let tag = *slots.first()?;
    [(Kind3::Spec, "font-spec", SPEC_SLOTS), (Kind3::Entity, "font-entity", ENTITY_SLOTS), (Kind3::Object, "font-object", OBJECT_SLOTS)]
        .into_iter().find(|(_, name, len)| tag == cx.intern(name) && slots.len() == *len).map(|(kind, _, _)| kind)
}
fn check_font(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<Kind3, Fault> {
    font_kind(cx, value).ok_or(Fault::Wrong("font", value))
}
fn check_spec(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<(), Fault> {
    if font_kind(cx, value) == Some(Kind3::Spec) { Ok(()) } else { Err(Fault::Wrong("font-spec", value)) }
}
fn slot(cx: &Leaf<'_>, font: Value, index: usize) -> Value {
    cx.record_slots(font).and_then(|slots| slots.get(index).copied()).unwrap_or(Value::NIL)
}
fn make_font(cx: &mut Leaf<'_>, tag: &str, len: usize) -> Out {
    let mut slots = vec![Value::NIL; len];
    slots[0] = cx.intern(tag);
    cx.record(slots)
}
fn name_of(cx: &Leaf<'_>, value: Value) -> Option<String> {
    if value == Value::NIL { return None; }
    if cx.kind(value) == Kind::Symbol { return cx.symbol_name(value).ok(); }
    cx.string_view(value).ok().map(|s| String::from_utf8_lossy(s.bytes()).into_owned())
}

/// font.c get_font_prop_index.
fn property_index(cx: &mut Leaf<'_>, key: Value) -> Option<usize> {
    PROPERTIES.iter().position(|name| key == cx.intern(name))
}

/// font_prop_validate: VALUE regularized for property IDX, else
/// `invalid font property'.
fn validate(cx: &mut Leaf<'_>, idx: usize, value: Value) -> Out {
    if value == Value::NIL { return Ok(value); }
    let prop = cx.intern(PROPERTIES[idx]);
    let regular = match idx {
        // font_prop_validate_symbol
        0..=4 | 12 | 13 => {
            let value = if matches!(cx.kind(value), Kind::String | Kind::Bytes) {
                let name = name_of(cx, value).unwrap_or_default();
                cx.intern(&name)
            } else { value };
            if cx.kind(value) != Kind::Symbol { None }
            else if idx == 4 { let name = cx.symbol_name(value)?.to_lowercase(); Some(cx.intern(&name)) }
            else { Some(value) }
        }
        // font_prop_validate_style
        5..=7 => {
            let table = table(idx + 1);
            if let Some(n) = value.integer_value() {
                let (row, name) = (((n >> 4) & 0xF) as usize, (n & 0xF) as usize);
                let extra = extras(cx, idx + 1).len();
                let valid = match table.get(row) {
                    Some((numeric, names)) => name < names.len() && *numeric == n >> 8,
                    None => row < table.len() + extra && name == 0 && n >> 8 == 100,
                };
                valid.then_some(value)
            } else if cx.kind(value) == Kind::Symbol {
                let name = cx.symbol_name(value)?;
                style_value(cx, idx + 1, &name, false).map(|n| Value::integer(n).unwrap())
            } else { None }
        }
        // font_prop_validate_non_neg
        8 | 9 | 11 => {
            let ok = value.integer_value().is_some_and(|n| n >= 0)
                || (cx.kind(value) == Kind::Float && cx.float_value(value)? >= 0.0);
            ok.then_some(value)
        }
        10 => spacing(cx, value),
        // font_prop_validate_otf
        _ => otf_valid(cx, value).then_some(value),
    };
    regular.ok_or_else(|| { let pair = cx.cons(prop, value); signal_error(cx, "invalid font property", pair) })
}
/// font_prop_validate_spacing.
fn spacing(cx: &mut Leaf<'_>, value: Value) -> Option<Value> {
    if value == Value::NIL || value.integer_value().is_some_and(|n| (0..=110).contains(&n)) { return Some(value); }
    if cx.kind(value) != Kind::Symbol { return None; }
    let name = cx.symbol_name(value).ok()?;
    let n = match name.as_bytes() { [b'c' | b'C'] => 110, [b'm' | b'M'] => 100, [b'p' | b'P'] => 0, [b'd' | b'D'] => 90, _ => return None };
    Some(Value::integer(n).unwrap())
}
fn otf_valid(cx: &Leaf<'_>, value: Value) -> bool {
    let Some((script, mut tail)) = cx.pair(value) else { return false };
    if cx.kind(script) != Kind::Symbol { return false; }
    if tail == Value::NIL { return true; }
    let Some((_, rest)) = cx.pair(tail) else { return false };
    tail = rest;
    for _ in 0..2 {
        if tail == Value::NIL { return true; }
        let Some((features, rest)) = cx.pair(tail) else { return false };
        let mut list = features;
        while let Some((feature, next)) = cx.pair(list) {
            if cx.kind(feature) != Kind::Symbol { return false; }
            list = next;
        }
        if list != Value::NIL { return false; }
        tail = rest;
    }
    true
}

/// font_put_extra: replace PROP's entry, else insert it keeping the alist
/// sorted by property name.
fn put_extra(cx: &mut Leaf<'_>, font: Value, prop: Value, value: Value) -> std::result::Result<(), Fault> {
    let extra = slot(cx, font, EXTRA);
    let mut tail = extra;
    while let Some((entry, rest)) = cx.pair(tail) {
        if cx.pair(entry).is_some_and(|(key, _)| key == prop) { cx.setcdr(entry, value)?; return Ok(()); }
        tail = rest;
    }
    let name = cx.symbol_name(prop)?;
    let mut entries = Vec::new();
    let mut tail = extra;
    while let Some((entry, rest)) = cx.pair(tail) { entries.push(entry); tail = rest; }
    let at = entries.iter().position(|&entry| {
        let key = cx.pair(entry).map_or(Value::NIL, |(key, _)| key);
        cx.symbol_name(key).is_ok_and(|key| name < key)
    }).unwrap_or(entries.len());
    let entry = cx.cons(prop, value);
    entries.insert(at, entry);
    let list = entries.into_iter().rev().fold(tail, |rest, entry| cx.cons(entry, rest));
    cx.record_set(font, EXTRA, list)
}

// --- font names (font.c font_parse_name and font_unparse_xlfd) ---------------

/// font_intern_prop: "*" is nil; all digits an integer unless FORCE_SYMBOL.
fn intern_prop(cx: &mut Leaf<'_>, text: &[u8], force_symbol: bool) -> Value {
    if text == b"*" { return Value::NIL; }
    if !force_symbol && !text.is_empty() && text.iter().all(u8::is_ascii_digit)
        && let Some(n) = std::str::from_utf8(text).ok().and_then(|t| t.parse::<i64>().ok())
        && let Ok(n) = Value::integer(n) {
        return n;
    }
    cx.intern(&String::from_utf8_lossy(text))
}
/// FONT_SET_STYLE: NAME's value, registering a name the table lacks.
fn set_style(cx: &mut Leaf<'_>, font: Value, slot_index: usize, name: &str) -> std::result::Result<(), Fault> {
    let encoded = style_value(cx, slot_index, name, true).unwrap();
    cx.record_set(font, slot_index, Value::integer(encoded).unwrap())
}
/// C strtod over the start of TEXT.
fn strtod(text: &[u8]) -> (f64, usize) {
    let end = text.iter().position(|b| !(b.is_ascii_digit() || *b == b'.')).unwrap_or(text.len());
    (std::str::from_utf8(&text[..end]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0), end)
}

// font.c XLFD field masks.
const XLFD_PIXEL: usize = 6;
const XLFD_POINT: usize = 7;
const XLFD_RESX: usize = 8;
const XLFD_RESY: usize = 9;
const XLFD_SPACING: usize = 10;
const XLFD_AVGWIDTH: usize = 11;
const XLFD_REGISTRY: usize = 12;
const XLFD_ENCODING: usize = 13;
const MASK_SYMBOL: u32 = 1 | 2 | 1 << 5 | 1 << 12;
const MASK_NULL: u32 = 1 | 1 << 5;
const MASK_LARGENUM: u32 = 1 << 7 | 1 << 8 | 1 << 9 | 1 << 11;
const MASK_REGENC: u32 = 1 << 12 | 1 << 13;

/// font_expand_wildcards: place the N fields of a wildcarded XLFD among
/// its 14, each where its value can stand. FIELDS returns all 14.
fn expand_wildcards(cx: &mut Leaf<'_>, fields: &mut Vec<Value>) -> bool {
    let n = fields.len();
    if n == 0 || n > 14 { return false; }
    let tmp = fields.clone();
    let mut range = vec![(0i32, 0i32, 0u32); n];
    let mut range_mask: u32 = (1u32 << (15 - n)) - 1;
    let (mut range_from, mut range_to) = (0i32, 14 - n as i32);
    for i in 0..n {
        let val = tmp[i];
        if val == Value::NIL {
            range[i] = (range_from, range_to, range_mask);
        } else {
            let (mut from, mut to, mut mask): (i32, i32, u32) = if let Some(numeric) = val.integer_value() {
                if i + 1 == n { (13, 13, 1 << 13) }
                else if numeric == 0 { (6, 11, 1 << 6 | MASK_LARGENUM) }
                else if numeric <= 48 { (6, 6, 1 << 6) }
                else { (7, 11, MASK_LARGENUM) }
            } else {
                let name = cx.symbol_name(val).unwrap_or_default();
                let numeric = |cx: &mut Leaf<'_>, slot: usize| style_value(cx, slot, &name, false).is_some();
                if name.is_empty() { (0, 5, MASK_NULL) }
                else if i == 0 { (0, 0, 1) }
                else if i + 1 == n { if name.ends_with('*') { (12, 13, MASK_REGENC) } else { (13, 13, 1 << 13) } }
                else if range_from <= 2 && range_to >= 2 && numeric(cx, WEIGHT) { (2, 2, 1 << 2) }
                else if range_from <= 3 && range_to >= 3 && numeric(cx, SLANT) { (3, 3, 1 << 3) }
                else if range_from <= 4 && range_to >= 4 && numeric(cx, WIDTH) { (4, 4, 1 << 4) }
                else if ["c", "m", "p", "d"].contains(&name.as_str()) { (10, 10, 1 << 10) }
                else { (0, 13, MASK_SYMBOL) }
            };
            mask &= range_mask;
            while from < range_from { mask &= !(1 << from); from += 1; }
            while from < 14 && mask & (1 << from) == 0 { from += 1; }
            while to > range_to { mask &= !(1 << to); to -= 1; }
            while to >= 0 && mask & (1 << to) == 0 { to -= 1; }
            if from > to { return false; }
            range[i] = (from, to, mask);
            if from > range_from || to < range_to {
                range_from = from;
                let (mut from, mut to) = (from - 1, to - 1);
                for j in (0..i).rev() {
                    if tmp[j] != Value::NIL && range[j].0 < from {
                        while range[j].0 < from { range[j].2 &= !(1 << range[j].0); range[j].0 += 1; }
                        while from < 14 && range[j].2 & (1 << from) == 0 { from += 1; }
                        range[j].0 = from;
                    } else { from = range[j].0; }
                    if range[j].1 > to {
                        while range[j].1 > to { range[j].2 &= !(1 << range[j].1); range[j].1 -= 1; }
                        while to >= 0 && range[j].2 & (1 << to) == 0 { to -= 1; }
                        range[j].1 = to;
                    } else { to = range[j].1; }
                    if from > to { return false; }
                    from -= 1;
                    to -= 1;
                }
            }
        }
        range_from += 1;
        range_to += 1;
        range_mask <<= 1;
    }
    let mut field = vec![Value::NIL; 14];
    let mut j = 0usize;
    for i in 0..n {
        let from = range[i].0 as usize;
        if j < from {
            if i == 0 || tmp[i - 1] != Value::NIL { return false; }
            j = from;
        }
        if j >= 14 { return false; }
        field[j] = tmp[i];
        j += 1;
    }
    if tmp[n - 1] != Value::NIL && j < XLFD_REGISTRY { return false; }
    if let Some(n) = field[XLFD_ENCODING].integer_value() { field[XLFD_ENCODING] = cx.intern(&n.to_string()); }
    *fields = field;
    true
}

/// font_parse_xlfd_1: a fully specified XLFD (14 fields), else one with
/// wildcards standing for the fields it omits.
fn parse_xlfd_1(cx: &mut Leaf<'_>, name: &[u8], font: Value, mut segments: isize) -> std::result::Result<bool, Fault> {
    if name.is_empty() { return Ok(false); }
    let mut f = [0usize; 15];
    let mut i = 0;
    if name[0] == b'*' && (name.len() == 1 || name[1] == b'-') { f[0] = 0; i = 1; }
    let start = i;
    for (at, &b) in name.iter().enumerate().skip(start) {
        if b == b'-' {
            if segments > 14 && i == 2 { segments -= 1; } else {
                f[i] = at + 1;
                i += 1;
                if i == 14 { break; }
            }
        }
    }
    f[i] = name.len();
    if i != 14 { return parse_partial_xlfd(cx, name, &f[..=i], font); }
    let field = |n: usize| &name[f[n]..f[n + 1].saturating_sub(1).max(f[n])];
    let foundry = intern_prop(cx, field(0), true);
    cx.record_set(font, FOUNDRY, foundry)?;
    let family = intern_prop(cx, field(1), true);
    cx.record_set(font, FAMILY, family)?;
    for (n, slot_index) in [(2, WEIGHT), (3, SLANT), (4, WIDTH)] {
        let text = field(n);
        if text == b"*" { continue; }
        let Some(encoded) = style_value(cx, slot_index, &String::from_utf8_lossy(text), false) else { return Ok(false) };
        cx.record_set(font, slot_index, Value::integer(encoded).unwrap())?;
    }
    let adstyle = intern_prop(cx, field(5), true);
    cx.record_set(font, ADSTYLE, adstyle)?;
    let registry = &name[f[12]..];
    let registry = if registry == b"*-*" { Value::NIL } else { intern_prop(cx, registry, true) };
    cx.record_set(font, REGISTRY, registry)?;
    let pixel = intern_prop(cx, field(6), false);
    if pixel.integer_value().is_some() {
        cx.record_set(font, SIZE, pixel)?;
    } else if font_kind(cx, font) == Some(Kind3::Entity) {
        return Ok(false);
    } else if name[f[7]..].first().is_some_and(u8::is_ascii_digit) {
        let digits = name[f[7]..].iter().take_while(|b| b.is_ascii_digit()).count();
        let points = std::str::from_utf8(&name[f[7]..f[7] + digits]).unwrap().parse::<f64>().unwrap_or(0.0) / 10.0;
        let size = cx.float(points);
        cx.record_set(font, SIZE, size)?;
    }
    let dpi = intern_prop(cx, field(9), false);
    if dpi != Value::NIL && dpi.integer_value().is_none() { return Ok(false); }
    cx.record_set(font, DPI, dpi)?;
    let spacing_value = intern_prop(cx, field(10), false);
    if spacing_value != Value::NIL {
        match spacing(cx, spacing_value) {
            Some(n) if n.integer_value().is_some() => cx.record_set(font, SPACING, n)?,
            _ => return Ok(false),
        }
    }
    let avg = field(11);
    let avg = intern_prop(cx, avg.strip_prefix(b"~").unwrap_or(avg), false);
    if avg != Value::NIL && avg.integer_value().is_none() { return Ok(false); }
    cx.record_set(font, AVGWIDTH, avg)?;
    Ok(true)
}
/// font_parse_xlfd_1's wildcarded branch: the fields F marks, placed by
/// expand_wildcards.
fn parse_partial_xlfd(cx: &mut Leaf<'_>, name: &[u8], f: &[usize], font: Value) -> std::result::Result<bool, Fault> {
    if font_kind(cx, font) == Some(Kind3::Entity) { return Ok(false); }
    let count = f.len() - 1;
    let (mut props, mut wild) = (Vec::with_capacity(count), false);
    for j in 0..count {
        let start = f[j];
        if name.get(start) == Some(&b'*') {
            if name.get(start + 1).is_some_and(|&b| b != b'-') { return Ok(false); }
            props.push(Value::NIL);
            wild = true;
        } else if j + 1 < count {
            props.push(intern_prop(cx, &name[start..f[j + 1].saturating_sub(1).max(start)], false));
        } else {
            props.push(intern_prop(cx, &name[start..f[count]], false));
        }
    }
    if !wild || !expand_wildcards(cx, &mut props) { return Ok(false); }
    cx.record_set(font, FOUNDRY, props[0])?;
    cx.record_set(font, FAMILY, props[1])?;
    for (n, slot_index) in [(2, WEIGHT), (3, SLANT), (4, WIDTH)] {
        let value = props[n];
        if value == Value::NIL { continue; }
        let encoded = match value.integer_value() {
            Some(numeric) => style_by_number(table(slot_index), numeric, true),
            None => { let name = cx.symbol_name(value)?; style_value(cx, slot_index, &name, true) }
        };
        let Some(encoded) = encoded else { return Ok(false) };
        cx.record_set(font, slot_index, Value::integer(encoded).unwrap())?;
    }
    cx.record_set(font, ADSTYLE, props[5])?;
    let text = |cx: &Leaf<'_>, v: Value| name_of(cx, v).unwrap_or_default();
    let registry = match (props[XLFD_REGISTRY], props[XLFD_ENCODING]) {
        (Value::NIL, Value::NIL) => None,
        (Value::NIL, encoding) => Some(format!("*-{}", text(cx, encoding))),
        (registry, Value::NIL) => Some(format!("{}-*", text(cx, registry))),
        (registry, encoding) => Some(format!("{}-{}", text(cx, registry), text(cx, encoding))),
    };
    if let Some(registry) = registry { let symbol = cx.intern(&registry); cx.record_set(font, REGISTRY, symbol)?; }
    if props[XLFD_PIXEL].integer_value().is_some() {
        cx.record_set(font, SIZE, props[XLFD_PIXEL])?;
    } else if let Some(point) = props[XLFD_POINT].integer_value() {
        let size = cx.float(point as f64 / 10.0);
        cx.record_set(font, SIZE, size)?;
    }
    if props[XLFD_RESX].integer_value().is_some() { cx.record_set(font, DPI, props[XLFD_RESY])?; }
    if props[XLFD_SPACING] != Value::NIL {
        match spacing(cx, props[XLFD_SPACING]) {
            Some(n) if n.integer_value().is_some() => cx.record_set(font, SPACING, n)?,
            _ => return Ok(false),
        }
    }
    if props[XLFD_AVGWIDTH].integer_value().is_some() { cx.record_set(font, AVGWIDTH, props[XLFD_AVGWIDTH])?; }
    Ok(true)
}

/// font_parse_xlfd: again with the family taking surplus dashes.
fn parse_xlfd(cx: &mut Leaf<'_>, name: &[u8], font: Value) -> std::result::Result<bool, Fault> {
    if parse_xlfd_1(cx, name, font, -1)? { return Ok(true); }
    let segments = name.iter().filter(|&&b| b == b'-').count() as isize;
    Ok(segments > 14 && parse_xlfd_1(cx, name, font, segments)?)
}

const FC_WEIGHTS: [&str; 14] = ["thin", "ultra-light", "light", "semi-light", "book", "medium", "normal", "semibold",
    "demibold", "bold", "ultra-bold", "black", "heavy", "ultra-heavy"];

/// font_parse_fcname: Fontconfig `FAMILY[-SIZE][:PROP[=VAL]]...' or GTK
/// `FAMILY [STYLE...] [SIZE]'.
fn parse_fcname(cx: &mut Leaf<'_>, name: &[u8], font: Value) -> std::result::Result<bool, Fault> {
    if name.is_empty() { return Ok(false); }
    let len = name.len();
    let (mut size, mut props, mut family_end) = (None::<usize>, None::<usize>, None::<usize>);
    let mut p = 0;
    while p < len {
        if name[p] == b'\\' && p + 1 < len { p += 2; continue; }
        if name[p] == b':' { props = Some(p); family_end = Some(p); break; }
        if name[p] == b'-' {
            let (mut decimal, mut found) = (false, true);
            let mut q = p + 1;
            while q < len && name[q] != b':' {
                if !name[q].is_ascii_digit() {
                    if name[q] != b'.' || decimal { found = false; break; }
                    decimal = true;
                }
                q += 1;
            }
            if found { family_end = Some(p); size = Some(p + 1); break; }
        }
        p += 1;
    }
    if let Some(end) = family_end {
        if end > 0 {
            let family = intern_prop(cx, &name[..end], true);
            cx.record_set(font, FAMILY, family)?;
        }
        if let Some(at) = size {
            let (points, used) = strtod(&name[at..]);
            let value = cx.float(points);
            cx.record_set(font, SIZE, value)?;
            let after = at + used;
            if after < len && name[after] == b':' && after + 1 < len { props = Some(after); }
        }
        if let Some(start) = props {
            let mut p = start;
            while p < len {
                let mut q = p + 1;
                while q < len && name[q] != b'=' && name[q] != b':' { q += 1; }
                if q >= len || name[q] != b'=' {
                    let word = &name[p + 1..q];
                    let text = String::from_utf8_lossy(word).into_owned();
                    if FC_WEIGHTS.contains(&text.as_str()) { set_style(cx, font, WEIGHT, &text)?; }
                    else if ["roman", "italic", "oblique"].contains(&text.as_str()) { set_style(cx, font, SLANT, &text)?; }
                    else if let Some(n) = [("charcell", 110), ("mono", 100), ("proportional", 0)].iter().find(|(w, _)| *w == text).map(|(_, n)| *n) {
                        cx.record_set(font, SPACING, Value::integer(n).unwrap())?;
                    }
                } else {
                    let key_text = &name[p..q];
                    let prop = if key_text == b":pixelsize" { Some(8) } else {
                        let key = intern_prop(cx, key_text, true);
                        property_index(cx, key)
                    };
                    let value_start = q + 1;
                    let mut end = value_start;
                    while end < len && name[end] != b':' { end += 1; }
                    let value = intern_prop(cx, &name[value_start..end], false);
                    if let Some(idx) = prop.filter(|&idx| (1..EXTRA_INDEX).contains(&idx)) {
                        let value = validate(cx, idx, value)?;
                        cx.record_set(font, idx + 1, value)?;
                    }
                    q = end;
                }
                p = q;
            }
        }
        return Ok(true);
    }
    // GTK: a trailing size, then style words, scanning backwards.
    let mut p = len as isize - 1;
    while p >= 0 && name[p as usize].is_ascii_digit() { p -= 1; }
    let mut size = None;
    if p < len as isize - 1 && (p + 1 == 0 || name[p as usize] == b' ') {
        size = Some(strtod(&name[(p + 1) as usize..]).0);
    } else { p = len as isize; }
    let (mut weight, mut slant, mut width) = (None, None, None);
    let mut family_end = None;
    while p > 0 {
        let mut q = p - 1;
        while q >= 0 {
            if q > 0 && name[(q - 1) as usize] == b'\\' { q -= 1; } else if name[q as usize] == b' ' { break; }
            q -= 1;
        }
        let word = &name[(q + 1) as usize..p as usize];
        let save = |slot: &mut Option<&'static str>, value: &'static str| if slot.is_none() { *slot = Some(value) };
        match word {
            b"Ultra-Light" => save(&mut weight, "ultra-light"),
            b"Light" => save(&mut weight, "light"),
            b"Book" => save(&mut weight, "book"),
            b"Medium" => save(&mut weight, "medium"),
            b"Semi-Bold" => save(&mut weight, "semi-bold"),
            b"Bold" => save(&mut weight, "bold"),
            b"Italic" => save(&mut slant, "italic"),
            b"Oblique" => save(&mut slant, "oblique"),
            b"Semi-Condensed" => save(&mut width, "semi-condensed"),
            b"Condensed" => save(&mut width, "condensed"),
            _ => { family_end = Some(p as usize); break; }
        }
        p = q;
    }
    if let Some(end) = family_end {
        let family = intern_prop(cx, &name[..end], true);
        cx.record_set(font, FAMILY, family)?;
    }
    if let Some(points) = size { let value = cx.float(points); cx.record_set(font, SIZE, value)?; }
    for (slot_index, value) in [(WEIGHT, weight), (SLANT, slant), (WIDTH, width)] {
        if let Some(value) = value { set_style(cx, font, slot_index, value)?; }
    }
    Ok(true)
}
/// font_parse_name.
fn parse_name(cx: &mut Leaf<'_>, name: &[u8], font: Value) -> std::result::Result<bool, Fault> {
    if name.first() == Some(&b'-') || name.contains(&b'*') || name.contains(&b'?') { parse_xlfd(cx, name, font) }
    else { parse_fcname(cx, name, font) }
}

/// font_unparse_xlfd, PIXEL_SIZE standing for a size of 0.
fn unparse_xlfd(cx: &mut Leaf<'_>, font: Value, pixel_size: i64) -> String {
    let [weight, slant, width] = [WEIGHT, SLANT, WIDTH].map(|index| {
        let value = slot(cx, font, index);
        style_name(cx, index, value).map_or("*".to_owned(), |name| name.chars().filter(|c| !matches!(c, '-' | '?' | ',' | '"')).collect())
    });
    let field = |index: usize, registry: bool| match name_of(cx, slot(cx, font, index)) {
        None => if registry { "*-*".to_owned() } else { "*".to_owned() },
        Some(name) if registry && !name.contains('-') => format!("{name}{}-*", if name.ends_with('*') { "" } else { "*" }),
        Some(name) => name,
    };
    let size = slot(cx, font, SIZE);
    let pixel = if let Some(n) = size.integer_value() {
        let v = if n > 0 { n } else { pixel_size };
        if v > 0 { format!("{v}-*") } else { "*-*".to_owned() }
    } else if let Ok(points) = cx.float_value(size) { format!("*-{:.0}", points * 10.0) } else { "*-*".to_owned() };
    let dpi = slot(cx, font, DPI).integer_value().map_or("*-*".to_owned(), |v| format!("{v}-{v}"));
    let spacing = slot(cx, font, SPACING).integer_value().map_or("*", |n| match n { ..=0 => "p", ..=90 => "d", ..=100 => "m", _ => "c" });
    let avg = slot(cx, font, AVGWIDTH).integer_value().map_or("*".to_owned(), |n| n.to_string());
    format!("-{}-{}-{}-{}-{}-{}-{}-{}-{}-{}-{}", field(FOUNDRY, false), field(FAMILY, false), weight, slant,
        width, field(ADSTYLE, false), pixel, dpi, spacing, avg, field(REGISTRY, true))
}
/// font_unparse_fcname.
fn unparse_fcname(cx: &mut Leaf<'_>, font: Value, pixel_size: i64) -> String {
    let mut name = name_of(cx, slot(cx, font, FAMILY)).unwrap_or_default();
    let size = slot(cx, font, SIZE);
    match size.integer_value() {
        Some(n) => {
            let pixels = if n != 0 { n } else { pixel_size };
            if pixels > 0 { name.push_str(&format!(":pixelsize={pixels}")); }
        }
        None => if let Ok(points) = cx.float_value(size) && points as i64 > 0 {
            name.push_str(&format!("{}{}", if name.is_empty() { "" } else { "-" }, points as i64));
        },
    }
    if let Some(foundry) = name_of(cx, slot(cx, font, FOUNDRY)) { name.push_str(&format!(":foundry={foundry}")); }
    for (index, key) in [(WEIGHT, "weight"), (SLANT, "slant"), (WIDTH, "width")] {
        let value = slot(cx, font, index);
        if let Some(style) = style_name(cx, index, value) { name.push_str(&format!(":{key}={style}")); }
    }
    if let Some(dpi) = slot(cx, font, DPI).integer_value() { name.push_str(&format!(":dpi={dpi}")); }
    if let Some(spacing) = slot(cx, font, SPACING).integer_value() { name.push_str(&format!(":spacing={spacing}")); }
    if let Some(avg) = slot(cx, font, AVGWIDTH).integer_value() {
        name.push_str(if avg == 0 { ":scalable=true" } else { ":scalable=false" });
    }
    name
}

// --- the display's fonts as entities and objects ------------------------------

/// The backend GNU's pgtk build reports for its fonts (`font-backend').
pub(crate) const BACKEND: &str = "ftcrhb";

fn entity_slots(cx: &mut Leaf<'_>, entity: &FontEntity, slots: &mut [Value]) {
    slots[TYPE] = cx.intern(BACKEND);
    slots[FOUNDRY] = entity.foundry.as_deref().map_or(Value::NIL, |name| cx.intern(name));
    slots[FAMILY] = cx.intern(&entity.family);
    slots[REGISTRY] = cx.intern(&entity.registry);
    for (index, numeric) in [(WEIGHT, entity.weight), (SLANT, entity.slant), (WIDTH, entity.width)] {
        slots[index] = style_by_number(table(index), numeric, true).map_or(Value::NIL, |n| Value::integer(n).unwrap());
    }
    slots[SIZE] = Value::integer(0).unwrap();
    slots[SPACING] = entity.spacing.map_or(Value::NIL, |n| Value::integer(n).unwrap());
    slots[AVGWIDTH] = Value::integer(0).unwrap();
    // ftfont.c keeps the face's file and index as :font-entity.
    let file = cx.string(&entity.file);
    let place = cx.cons(file, Value::integer(i64::from(entity.index)).unwrap());
    let key = cx.intern(":font-entity");
    let entry = cx.cons(key, place);
    slots[EXTRA] = cx.cons(entry, Value::NIL);
}
fn make_entity(cx: &mut Leaf<'_>, entity: &FontEntity) -> Out {
    let mut slots = vec![Value::NIL; ENTITY_SLOTS];
    slots[0] = cx.intern("font-entity");
    entity_slots(cx, entity, &mut slots);
    cx.record(slots)
}
/// The entity behind ENTITY's record, read back from its slots.
fn entity_of(cx: &mut Leaf<'_>, font: Value) -> Option<FontEntity> {
    let key = cx.intern(":font-entity");
    let mut extra = slot(cx, font, EXTRA);
    let mut place = None;
    while let Some((entry, rest)) = cx.pair(extra) {
        if let Some((k, v)) = cx.pair(entry) && k == key { place = cx.pair(v); }
        extra = rest;
    }
    let (file, index) = place?;
    Some(FontEntity {
        foundry: name_of(cx, slot(cx, font, FOUNDRY)),
        family: name_of(cx, slot(cx, font, FAMILY))?,
        weight: style_numeric(slot(cx, font, WEIGHT))?,
        slant: style_numeric(slot(cx, font, SLANT))?,
        width: style_numeric(slot(cx, font, WIDTH))?,
        spacing: slot(cx, font, SPACING).integer_value(),
        registry: name_of(cx, slot(cx, font, REGISTRY)).unwrap_or_default(),
        file: name_of(cx, file)?,
        index: u32::try_from(index.integer_value()?).ok()?,
    })
}
/// font_build_object: the opened font's record, named as GNU names it.
fn make_object(cx: &mut Leaf<'_>, opened: &OpenedFont) -> Out {
    let mut slots = vec![Value::NIL; OBJECT_SLOTS];
    slots[0] = cx.intern("font-object");
    entity_slots(cx, &opened.entity, &mut slots);
    let object = cx.record(slots)?;
    let xlfd = unparse_xlfd(cx, object, opened.pixel_size);
    let full = unparse_fcname(cx, object, opened.pixel_size);
    let integer = |n: i64| Value::integer(n).unwrap();
    let name = cx.string(&xlfd);
    let full = cx.string(&full);
    let file = cx.string(&opened.entity.file);
    // font-info's CAPABILITY, (opentype GSUB . GPOS) as hbfont.c gives it.
    let gsub = otf_list(cx, &opened.otf.0);
    let gpos = otf_list(cx, &opened.otf.1);
    let tables = cx.cons(gsub, gpos);
    let format = cx.intern("opentype");
    let capability = cx.cons(format, tables);
    for (index, value) in [(SIZE, integer(opened.pixel_size)), (NAME, name), (FULLNAME, full), (FILE, file),
        (ASCENT, integer(opened.ascent)), (DESCENT, integer(opened.descent)), (MAX_WIDTH, integer(opened.max_width)),
        (SPACE_WIDTH, integer(opened.space_width)), (AVERAGE_WIDTH, integer(opened.average_width)), (CAPABILITY, capability)] {
        cx.record_set(object, index, value)?;
    }
    Ok(object)
}

/// ((SCRIPT (LANGSYS FEATURE ...) ...) ...), each tag a symbol.
fn otf_list(cx: &mut Leaf<'_>, table: &crate::OtfTable) -> Value {
    let mut scripts = Vec::new();
    for (script, langs) in table {
        let mut entries = Vec::new();
        for (lang, features) in langs {
            let mut list = Value::NIL;
            for feature in features.iter().rev() { let tag = cx.intern(feature); list = cx.cons(tag, list); }
            let lang = lang.as_deref().map_or(Value::NIL, |tag| cx.intern(tag));
            entries.push(cx.cons(lang, list));
        }
        let list = entries.into_iter().rev().fold(Value::NIL, |tail, entry| cx.cons(entry, tail));
        let tag = cx.intern(script);
        scripts.push(cx.cons(tag, list));
    }
    scripts.into_iter().rev().fold(Value::NIL, |tail, script| cx.cons(script, tail))
}

/// The query FONT's properties make (font_list_entities).
fn query_of(cx: &Leaf<'_>, font: Value) -> FontQuery {
    FontQuery {
        family: name_of(cx, slot(cx, font, FAMILY)),
        foundry: name_of(cx, slot(cx, font, FOUNDRY)),
        weight: style_numeric(slot(cx, font, WEIGHT)),
        slant: style_numeric(slot(cx, font, SLANT)),
        width: style_numeric(slot(cx, font, WIDTH)),
        spacing: slot(cx, font, SPACING).integer_value(),
        registry: name_of(cx, slot(cx, font, REGISTRY)),
    }
}
/// font_parse_family_registry over a fresh spec, as Fset_fontset_font takes
/// a (FAMILY . REGISTRY) cons: FAMILY may hold a foundry, REGISTRY an
/// encoding.
pub(super) fn spec_from_family_registry(cx: &mut Leaf<'_>, family: Value, registry: Value) -> Out {
    let spec = make_font(cx, "font-spec", SPEC_SLOTS)?;
    if family != Value::NIL {
        let text = cx.string_text(family).map_err(|_| Fault::Wrong("stringp", family))?;
        let (foundry, name) = match text.split_once('-') { Some((foundry, name)) => (Some(foundry.to_string()), name.to_string()), None => (None, text) };
        if let Some(foundry) = foundry {
            let value = intern_prop(cx, foundry.as_bytes(), true);
            cx.record_set(spec, FOUNDRY, value)?;
        }
        let value = intern_prop(cx, name.as_bytes(), true);
        cx.record_set(spec, FAMILY, value)?;
    }
    if registry != Value::NIL {
        let text = cx.string_text(registry).map_err(|_| Fault::Wrong("stringp", registry))?.to_lowercase();
        let text = if text.contains('-') { text } else if text.ends_with('*') { format!("{text}-*") } else { format!("{text}*-*") };
        let value = intern_prop(cx, text.as_bytes(), true);
        cx.record_set(spec, REGISTRY, value)?;
    }
    Ok(spec)
}

/// `(font-spec :name NAME)'.
pub(super) fn spec_from_name(cx: &mut Leaf<'_>, name: Value) -> Out {
    let spec = make_font(cx, "font-spec", SPEC_SLOTS)?;
    let text = cx.string_text(name)?;
    if !parse_name(cx, text.as_bytes(), spec)? {
        return Err(signal_error(cx, "Invalid font name", name));
    }
    let key = cx.intern(":name");
    put_extra(cx, spec, key, name)?;
    Ok(spec)
}

pub(super) fn is_spec(cx: &mut Leaf<'_>, value: Value) -> bool {
    font_kind(cx, value) == Some(Kind3::Spec)
}

/// A symbol's list, or the one symbol: an `:otf' feature or `:lang' value.
fn symbol_names(cx: &Leaf<'_>, value: Value) -> Vec<String> {
    if cx.kind(value) == Kind::Symbol && value != Value::NIL { return name_of(cx, value).into_iter().collect(); }
    let (mut out, mut tail) = (Vec::new(), value);
    while let Some((item, rest)) = cx.pair(tail) {
        out.extend(name_of(cx, item));
        tail = rest;
    }
    out
}

/// An `:otf' feature list: tags a font must have, then, after a nil, tags
/// it must lack.
fn otf_features(cx: &Leaf<'_>, mut list: Value) -> Vec<(String, bool)> {
    let (mut features, mut wanted) = (Vec::new(), true);
    while let Some((item, rest)) = cx.pair(list) {
        if item == Value::NIL { wanted = false; } else if let Some(tag) = name_of(cx, item) { features.push((tag, wanted)); }
        list = rest;
    }
    features
}

/// `otf_spec': the value of an `:otf' property.
fn otf_spec(cx: &Leaf<'_>, value: Value) -> Option<romacs_layout_input::OtfSpec> {
    let (script, rest) = cx.pair(value)?;
    let script = if script == Value::NIL { "DFLT".to_string() } else { name_of(cx, script)? };
    let (langsys, rest) = cx.pair(rest).unwrap_or((Value::NIL, Value::NIL));
    let (gsub, rest) = cx.pair(rest).unwrap_or((Value::NIL, Value::NIL));
    let (gpos, _) = cx.pair(rest).unwrap_or((Value::NIL, Value::NIL));
    Some(romacs_layout_input::OtfSpec { script, langsys: name_of(cx, langsys).filter(|_| langsys != Value::NIL), gsub: otf_features(cx, gsub), gpos: otf_features(cx, gpos) })
}

/// A font-spec reduced to a font-def, its charsets left for the caller.
pub(super) fn font_def_of(cx: &mut Leaf<'_>, spec: Value) -> std::result::Result<romacs_layout_input::FontDef, Fault> {
    let text = |cx: &Leaf<'_>, index| { let value = slot(cx, spec, index); (value != Value::NIL).then(|| name_of(cx, value)).flatten() };
    let style = |cx: &mut Leaf<'_>, index| { let value = slot(cx, spec, index); (value != Value::NIL).then(|| style_name(cx, index, value)).flatten() };
    let (weight, slant, width) = (style(cx, WEIGHT), style(cx, SLANT), style(cx, WIDTH));
    let mut def = romacs_layout_input::FontDef {
        family: text(cx, FAMILY),
        foundry: text(cx, FOUNDRY),
        registry: text(cx, REGISTRY).map(|registry| registry.to_lowercase()),
        name: unparse_xlfd(cx, spec, 0),
        weight: weight.as_deref().map(romacs_face::attr::css_weight),
        width: width.as_deref().map(romacs_face::attr::width_class),
        slant: slant.as_deref().map(|name| match name { "italic" => 1, "oblique" => 2, _ => 0 }),
        pixel_size: u32::try_from(pixel_size(cx, spec, 0)).ok().filter(|&size| size > 0),
        ..Default::default()
    };
    let (mut extra, script_table) = (slot(cx, spec, EXTRA), super::fileio::var(cx, "script-representative-chars")?);
    while let Some((pair, rest)) = cx.pair(extra) {
        extra = rest;
        let Some((key, value)) = cx.pair(pair) else { continue };
        match name_of(cx, key).as_deref() {
            Some(":lang") => def.lang = symbol_names(cx, value),
            Some(":otf") => def.otf = otf_spec(cx, value),
            Some(":script") => {
                let mut table = script_table;
                while let Some((row, next)) = cx.pair(table) {
                    table = next;
                    let Some((script, mut chars)) = cx.pair(row) else { continue };
                    if script != value { continue; }
                    while let Some((c, more)) = cx.pair(chars) {
                        def.script_chars.extend(c.integer_value().map(|c| c as u32));
                        chars = more;
                    }
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(def)
}

/// A new fontset's NAME, in XLFD format: its registry, `fontset-*', the
/// short alias, and the name with registry iso8859-1, its ASCII font.
pub(super) fn fontset_name_parts(cx: &mut Leaf<'_>, name: &str) -> std::result::Result<(String, String), Fault> {
    let spec = make_font(cx, "font-spec", SPEC_SLOTS)?;
    if !parse_xlfd(cx, name.as_bytes(), spec)? { return Err(error(cx, "Fontset name must be in XLFD format".into())); }
    let short = name_of(cx, slot(cx, spec, REGISTRY)).unwrap_or_default();
    if !short.starts_with("fontset-") || short.len() < 9 {
        return Err(error(cx, "Registry field of fontset name must be \"fontset-*\"".into()));
    }
    let registry = cx.intern("iso8859-1");
    cx.record_set(spec, REGISTRY, registry)?;
    let ascii = unparse_xlfd(cx, spec, 0);
    Ok((short, ascii))
}

/// The XLFD name of the font a def was realized to.
pub(super) fn opened_name(cx: &mut Leaf<'_>, font: &romacs_layout_input::OpenedDefFont) -> std::result::Result<Option<String>, Fault> {
    let Some(opened) = cx.font_host().and_then(|host| host.opened_def_font(font)) else { return Ok(None) };
    let object = make_object(cx, &opened)?;
    Ok(name_of(cx, slot(cx, object, NAME)))
}

/// font_pixel_size: an integer size, else points at the frame's 96 dpi.
fn pixel_size(cx: &Leaf<'_>, font: Value, default: i64) -> i64 {
    let size = slot(cx, font, SIZE);
    if let Some(n) = size.integer_value() { return if n > 0 { n } else { default }; }
    match cx.float_value(size) {
        Ok(points) => {
            let dpi = slot(cx, font, DPI).integer_value().unwrap_or(96) as f64;
            (points * dpi / 72.0 + 0.5) as i64
        }
        Err(_) => default,
    }
}
/// The default face's pixel size, as GNU's frame font's.
fn frame_pixel_size(cx: &Leaf<'_>) -> std::result::Result<i64, Fault> {
    Ok(cx.default_font_request()?.pixel_size.round() as i64)
}
/// font_open_entity and friends: FONT (spec, entity or object) opened on
/// the display, a spec filled from the default face as font_load_for_lface
/// does. None when nothing matches or opens.
fn open(cx: &mut Leaf<'_>, font: Value) -> std::result::Result<Option<Value>, Fault> {
    let kind = check_font(cx, font)?;
    if kind == Kind3::Object { return Ok(Some(font)); }
    let default = frame_pixel_size(cx)?;
    let size = pixel_size(cx, font, default);
    let entity = if kind == Kind3::Entity { entity_of(cx, font) } else {
        let mut query = query_of(cx, font);
        if query.family.is_none() { query.family = Some(cx.default_font_request()?.family); }
        cx.font_host().and_then(|host| host.list_fonts(&query).into_iter().next())
    };
    let Some(entity) = entity else { return Ok(None) };
    match cx.font_host().and_then(|host| host.open_font(&entity, size)) {
        Some(opened) => Ok(Some(make_object(cx, &opened)?)),
        None => Ok(None),
    }
}

/// The frame's font's name while the frame is on a display: the default
/// face's font, the one its ASCII characters draw with.
pub(crate) fn frame_font(cx: &mut Leaf<'_>) -> std::result::Result<Option<String>, Fault> {
    face_font(cx, "default", ' ')
}

/// xfaces.c Fface_font on a display: the name of the font the face FACE
/// draws CH with, as painting resolves it from the face's family and
/// pixel size. None without a display, or for a face painting cannot
/// realize.
pub(crate) fn face_font(cx: &mut Leaf<'_>, face: &str, ch: char) -> std::result::Result<Option<String>, Fault> {
    if cx.font_host().is_none() { return Ok(None); }
    let Ok(request) = cx.face_font_request(face) else { return Ok(None) };
    let key = request.key();
    if ch == ' ' && let Some(name) = cx.cached_face_font(&key) { return Ok(Some(name)); }
    let Some((opened, _)) = cx.font_host().and_then(|host| host.char_font(&request, ch)) else { return Ok(None) };
    // font_build_object's name: the entity's XLFD at the opened size.
    let entity = make_entity(cx, &opened.entity)?;
    let name = unparse_xlfd(cx, entity, opened.pixel_size);
    if ch == ' ' { cx.cache_face_font(key, name.clone()); }
    Ok(Some(name))
}

// --- primitives -----------------------------------------------------------------

defsubr! { name = "fontp", gnu = "font.c:3999", effect = Reads,
fn fontp(cx, object: Value; optional extra_type: Value) {
    let kind = font_kind(cx, object);
    let want = if extra_type == Value::NIL { return Ok(if kind.is_some() { Value::TRUE } else { Value::NIL }) }
        else if extra_type == cx.intern("font-spec") { Kind3::Spec }
        else if extra_type == cx.intern("font-entity") { Kind3::Entity }
        else if extra_type == cx.intern("font-object") { Kind3::Object }
        else { return Err(Fault::Wrong("font-extra-type", extra_type)) };
    Ok(if kind == Some(want) { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "font-spec", gnu = "font.c:4018", effect = Reads,
fn font_spec(cx; rest args) {
    let spec = make_font(cx, "font-spec", SPEC_SLOTS)?;
    let name_key = cx.intern(":name");
    for pair in args.chunks(2) {
        let key = pair[0];
        if cx.kind(key) != Kind::Symbol { return Err(Fault::Wrong("symbolp", key)); }
        let Some(&value) = pair.get(1) else {
            let name = cx.symbol_name(key)?;
            return Err(error(cx, format!("No value for key ‘{name}’")));
        };
        if key == name_key {
            let text = cx.string_view(value)?.bytes().to_vec();
            if !parse_name(cx, &text, spec)? {
                return Err(error(cx, format!("Invalid font name: {}", String::from_utf8_lossy(&text))));
            }
            put_extra(cx, spec, key, value)?;
        } else if let Some(idx) = property_index(cx, key) {
            let value = validate(cx, idx, value)?;
            if idx < EXTRA_INDEX { cx.record_set(spec, idx + 1, value)?; } else { put_extra(cx, spec, key, value)?; }
        } else {
            put_extra(cx, spec, key, value)?;
        }
    }
    Ok(spec)
}}

defsubr! { name = "font-get", gnu = "font.c:4210", effect = Reads,
fn font_get(cx, font: Value, key: Value) {
    let kind = check_font(cx, font)?;
    if cx.kind(key) != Kind::Symbol { return Err(Fault::Wrong("symbolp", key)); }
    match property_index(cx, key) {
        Some(idx @ 5..=7) => { let value = slot(cx, font, idx + 1); return Ok(style_symbol(cx, idx + 1, value)); }
        Some(idx) if idx < EXTRA_INDEX => return Ok(slot(cx, font, idx + 1)),
        _ => {}
    }
    let mut extra = slot(cx, font, EXTRA);
    while let Some((entry, rest)) = cx.pair(extra) {
        if let Some((k, v)) = cx.pair(entry) && k == key { return Ok(v); }
        extra = rest;
    }
    // A font-object answers its OpenType capability, (GSUB . GPOS); Romacs's
    // fonts have no combining capability of their own.
    if kind == Kind3::Object && key == cx.intern(":otf") {
        return Ok(cx.pair(slot(cx, font, CAPABILITY)).map_or(Value::NIL, |(_, tables)| tables));
    }
    Ok(Value::NIL)
}}

defsubr! { name = "font-put", gnu = "font.c:4358", effect = Global,
fn font_put(cx, font: Value, prop: Value, val: Value) {
    match property_index(cx, prop) {
        Some(idx) if idx < EXTRA_INDEX => {
            check_spec(cx, font)?;
            let value = validate(cx, idx, val)?;
            cx.record_set(font, idx + 1, value)?;
        }
        idx => {
            if [":name", ":script", ":lang", ":otf"].iter().any(|name| prop == cx.intern(name)) { check_spec(cx, font)?; }
            else { check_font(cx, font)?; }
            let value = match idx { Some(idx) => validate(cx, idx, val)?, None => val };
            put_extra(cx, font, prop, value)?;
        }
    }
    Ok(val)
}}

defsubr! { name = "list-fonts", gnu = "font.c:4394", effect = Reads,
fn list_fonts(cx, font_spec: Value; optional frame: Value, num: Value, prefer: Value) {
    cx.windows().check_live_frame(frame)?;
    check_spec(cx, font_spec)?;
    let limit = if num == Value::NIL { None } else {
        let n = num.integer_value().ok_or(Fault::Wrong("fixnump", num))?;
        if n <= 0 { return Ok(Value::NIL); }
        Some(n as usize)
    };
    if prefer != Value::NIL { check_spec(cx, prefer)?; }
    let query = query_of(cx, font_spec);
    let entities = cx.font_host().map_or_else(Vec::new, |host| host.list_fonts(&query));
    let mut list = Vec::new();
    for entity in entities.iter().take(limit.unwrap_or(usize::MAX)) { list.push(make_entity(cx, entity)?); }
    Ok(list.into_iter().rev().fold(Value::NIL, |tail, entity| cx.cons(entity, tail)))
}}

defsubr! { name = "find-font", gnu = "font.c:4466", effect = Reads,
fn find_font(cx, font_spec: Value; optional frame: Value) {
    cx.windows().check_live_frame(frame)?;
    check_spec(cx, font_spec)?;
    let query = query_of(cx, font_spec);
    let first = cx.font_host().and_then(|host| host.list_fonts(&query).into_iter().next());
    match first { Some(entity) => make_entity(cx, &entity), None => Ok(Value::NIL) }
}}

defsubr! { name = "font-family-list", gnu = "font.c:4442", effect = Reads,
fn font_family_list(cx; optional frame: Value) {
    cx.windows().check_live_frame(frame)?;
    let families = cx.font_host().map_or_else(Vec::new, |host| host.font_families());
    let mut list = Value::NIL;
    for family in families.iter().rev() { let name = cx.string(family); list = cx.cons(name, list); }
    Ok(list)
}}

defsubr! { name = "font-xlfd-name", gnu = "font.c:4478", effect = Reads,
fn font_xlfd_name(cx, font: Value; optional fold_wildcards: Value, _long_xlfds: Value) {
    let kind = check_font(cx, font)?;
    let mut name = match (kind, name_of(cx, slot(cx, font, NAME))) {
        (Kind3::Object, Some(name)) if name.starts_with('-') => name,
        _ => {
            let pixels = if kind == Kind3::Object { slot(cx, font, SIZE).integer_value().unwrap_or(0) } else { 0 };
            let name = unparse_xlfd(cx, font, pixels);
            if name.len() >= 256 && _long_xlfds == Value::NIL { return Ok(Value::NIL); }
            name
        }
    };
    if fold_wildcards != Value::NIL {
        while let Some(at) = name.find("-*-*") { name.replace_range(at..at + 2, ""); }
    }
    Ok(cx.string(&name))
}}

/// CHECK_FONT_GET_OBJECT.
fn check_object(cx: &mut Leaf<'_>, font: Value) -> std::result::Result<(), Fault> {
    if font_kind(cx, font) == Some(Kind3::Object) { Ok(()) } else { Err(Fault::Wrong("font-object", font)) }
}

/// The OpenedFont a font-object is.
fn opened_of(cx: &mut Leaf<'_>, font: Value) -> Option<OpenedFont> {
    let entity = entity_of(cx, font)?;
    let size = slot(cx, font, SIZE).integer_value()?;
    cx.font_host()?.open_font(&entity, size)
}

defsubr! { name = "query-font", gnu = "font.c:5277", effect = Reads,
fn query_font(cx, font_object: Value) {
    check_object(cx, font_object)?;
    let capability = slot(cx, font_object, CAPABILITY);
    let items: Vec<Value> = [NAME, FILE, SIZE, MAX_WIDTH, ASCENT, DESCENT, SPACE_WIDTH, AVERAGE_WIDTH].iter().map(|&index| slot(cx, font_object, index)).chain([capability]).collect();
    Ok(cx.vector(&items))
}}

defsubr! { name = "font-has-char-p", gnu = "font.c:5335", effect = Reads,
fn font_has_char_p(cx, font: Value, ch: Value; optional frame: Value) {
    check_font(cx, font)?;
    let c = ch.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c)).ok_or(Fault::Wrong("characterp", ch))?;
    if frame != Value::NIL && cx.kind(frame) != Kind::Frame { return Err(Fault::Wrong("framep", frame)); }
    let Some(object) = open(cx, font)? else { return Ok(Value::NIL) };
    let Some(opened) = opened_of(cx, object) else { return Ok(Value::NIL) };
    let found = char::from_u32(c as u32).and_then(|c| cx.font_host()?.glyph_metrics(&opened, c));
    Ok(if found.is_some() { Value::TRUE } else { Value::NIL })
}}

// Fopen_font: at SIZE pixels, points when a float, or the entity's own; 0 is
// 120, as in GNU.
defsubr! { name = "open-font", gnu = "font.c:5242", effect = Reads,
fn open_font(cx, font_entity: Value; optional size: Value, frame: Value) {
    if font_kind(cx, font_entity) != Some(Kind3::Entity) { return Err(Fault::Wrong("font-entity", font_entity)); }
    cx.face_frame(frame, true)?;
    let pixels = if size == Value::NIL {
        slot(cx, font_entity, SIZE).integer_value().unwrap_or(0)
    } else if let Some(n) = size.integer_value() {
        n
    } else if let Ok(points) = cx.float_value(size) {
        (points * 96.0 / 72.27 + 0.5) as i64
    } else {
        return Err(Fault::Wrong("numberp", size));
    };
    if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&pixels) { return Err(Fault::Range(font_entity, size)); }
    let pixels = if pixels == 0 { 120 } else { pixels };
    let Some(entity) = entity_of(cx, font_entity) else { return Ok(Value::NIL) };
    match cx.font_host().and_then(|host| host.open_font(&entity, pixels)) {
        Some(opened) => make_object(cx, &opened),
        None => Ok(Value::NIL),
    }
}}

/// font_match_p's style and size test: FONT differs from SPEC in nothing
/// SPEC names (font_score, zero).
fn matches_spec(cx: &Leaf<'_>, spec: Value, font: Value) -> bool {
    for index in [FOUNDRY, FAMILY, ADSTYLE, REGISTRY] {
        let (want, have) = (slot(cx, spec, index), slot(cx, font, index));
        if want != Value::NIL && have != Value::NIL && want != have { return false; }
    }
    for index in [WEIGHT, SLANT, WIDTH] {
        let (want, have) = (slot(cx, spec, index), slot(cx, font, index));
        if want != Value::NIL && want != have && have.integer_value().is_some()
            && style_numeric(want) != style_numeric(have) { return false; }
    }
    let (want, have) = (pixel_size(cx, spec, 0), slot(cx, font, SIZE).integer_value().unwrap_or(0));
    want <= 0 || have <= 0 || want == have
}

defsubr! { name = "font-match-p", gnu = "font.c:5492", effect = Reads,
fn font_match_p(cx, spec: Value, font: Value) {
    check_spec(cx, spec)?;
    check_font(cx, font)?;
    if !matches_spec(cx, spec, font) { return Ok(Value::NIL); }
    let (mut extra, is_object) = (slot(cx, spec, EXTRA), font_kind(cx, font) == Some(Kind3::Object));
    let script_table = super::fileio::var(cx, "script-representative-chars")?;
    while let Some((pair, rest)) = cx.pair(extra) {
        extra = rest;
        let Some((key, value)) = cx.pair(pair) else { continue };
        match name_of(cx, key).as_deref() {
            Some(":lang") => return Ok(Value::NIL),
            Some(":script") if is_object => {
                let mut table = script_table;
                while let Some((row, next)) = cx.pair(table) {
                    table = next;
                    let Some((script, mut chars)) = cx.pair(row) else { continue };
                    if script != value { continue; }
                    let Some(opened) = opened_of(cx, font) else { return Ok(Value::NIL) };
                    while let Some((c, more)) = cx.pair(chars) {
                        chars = more;
                        let found = c.integer_value().and_then(|c| char::from_u32(c as u32)).and_then(|c| cx.font_host()?.glyph_metrics(&opened, c));
                        if found.is_none() { return Ok(Value::NIL); }
                    }
                    break;
                }
            }
            Some(":otf") if !is_object => return Ok(Value::NIL),
            _ => {}
        }
    }
    Ok(Value::TRUE)
}}

defsubr! { name = "font-face-attributes", gnu = "font.c:4278", effect = Reads,
fn font_face_attributes(cx, font: Value; optional frame: Value) {
    cx.face_frame(frame, true)?;
    let font = if matches!(cx.kind(font), Kind::String | Kind::Bytes) {
        let spec = make_font(cx, "font-spec", SPEC_SLOTS)?;
        let text = cx.string_text(font)?;
        if !parse_name(cx, text.as_bytes(), spec)? { return Err(signal_error(cx, "Invalid font name", font)); }
        spec
    } else if font_kind(cx, font).is_some() { font } else { return Err(signal_error(cx, "Invalid font object", font)); };
    let mut items = Vec::new();
    let family = slot(cx, font, FAMILY);
    if family != Value::NIL {
        items.push(cx.intern(":family"));
        let name = name_of(cx, family).unwrap_or_default();
        items.push(cx.string(&name));
    }
    let size = slot(cx, font, SIZE);
    let height = if let Some(pixels) = size.integer_value() {
        let dpi = slot(cx, font, DPI).integer_value().unwrap_or(96) as f64;
        Some(((pixels * 10) as f64 * 72.27 / dpi + 0.5) as i64)
    } else { cx.float_value(size).ok().map(|points| 10 * points as i64) };
    if let Some(height) = height {
        items.push(cx.intern(":height"));
        items.push(Value::integer(height).map_err(Fault::Error)?);
    }
    for (key, index) in [(":weight", WEIGHT), (":slant", SLANT), (":width", WIDTH)] {
        let value = slot(cx, font, index);
        if value != Value::NIL && let Some(name) = style_name(cx, index, value) {
            items.push(cx.intern(key));
            items.push(cx.intern(&name));
        }
    }
    Ok(cx.coding_list(&items))
}}

defsubr! { name = "font-get-glyphs", gnu = "font.c:5362", effect = Reads,
fn font_get_glyphs(cx, font_object: Value, from: Value, to: Value; optional object: Value) {
    check_object(cx, font_object)?;
    let chars: Vec<i64> = if object == Value::NIL {
        let span = cx.text_span(from, to, true)?;
        String::from_utf8_lossy(&span.bytes).chars().map(|c| i64::from(u32::from(c))).collect()
    } else {
        let elements: Vec<Value> = if let Ok(values) = cx.vector_values(object) {
            values
        } else if let Ok(view) = cx.string_view(object) {
            view.chars().map(|c| Value::integer(i64::from(c)).unwrap()).collect()
        } else {
            return Err(Fault::Wrong("arrayp", object));
        };
        // validate_subarray.
        let size = elements.len() as i64;
        let bound = |v: Value| -> std::result::Result<i64, Fault> {
            let n = v.integer_value().ok_or(Fault::Wrong("integerp", v))?;
            Ok(if n < 0 { n + size } else { n })
        };
        let (start, end) = (bound(from)?, bound(to)?);
        if !(0 <= start && start <= end && end <= size) { return Err(Fault::Range3(object, from, to)); }
        let mut chars = Vec::with_capacity((end - start) as usize);
        for &element in &elements[start as usize..end as usize] {
            chars.push(element.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c)).ok_or(Fault::Wrong("characterp", element))?);
        }
        chars
    };
    if chars.is_empty() { return Ok(Value::NIL); }
    let Some(opened) = opened_of(cx, font_object) else { return Ok(Value::NIL) };
    let integer = |n: i64| Value::integer(n).unwrap();
    let mut glyphs = Vec::with_capacity(chars.len());
    for (index, &code) in chars.iter().enumerate() {
        let metrics = char::from_u32(code as u32).and_then(|c| cx.font_host()?.glyph_metrics(&opened, c));
        glyphs.push(match metrics {
            None => Value::NIL,
            Some(m) => {
                let i = index as i64;
                cx.vector(&[integer(i), integer(i), integer(code), integer(i64::from(m.code)), integer(m.width), integer(m.lbearing), integer(m.rbearing), integer(m.ascent), integer(m.descent), Value::NIL])
            }
        });
    }
    Ok(cx.vector(&glyphs))
}}

// Fclear_font_cache: the display forgets its fonts, and what was resolved
// from them goes, as free_realized_fontsets.
defsubr! { name = "clear-font-cache", gnu = "font.c:4576", effect = Global,
fn clear_font_cache(cx) {
    if let Some(host) = cx.font_host() { host.clear_font_cache(); }
    cx.fontsets_mut().touch();
    cx.note_fontset_change();
    Ok(Value::NIL)
}}

defsubr! { name = "font-info", gnu = "font.c:5594", effect = Reads,
fn font_info(cx, name: Value; optional frame: Value) {
    let is_font = font_kind(cx, name).is_some();
    if !is_font && !matches!(cx.kind(name), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", name)); }
    cx.windows().check_live_frame(frame)?;
    if cx.font_host().is_none() {
        // The initial terminal's one font is "tty" (VM2-WIN5), whose
        // metrics are the frame's canonical character, which the host lays
        // out. Any other font does not open.
        if is_font || !cx.string_view(name).is_ok_and(|n| n.bytes() == b"tty") { return Ok(Value::NIL); }
        let width = cx.windows().frame_char_size(frame, true)?;
        let height = cx.windows().frame_char_size(frame, false)?;
        let zero = Value::integer(0).map_err(Fault::Error)?;
        let format = cx.intern("x");
        let capability = cx.cons(format, Value::NIL);
        return Ok(cx.vector(&[name, name, height, height, zero, zero, zero, width, height, zero, width, width, Value::NIL, capability]));
    }
    let font = if is_font { name } else {
        let spec = make_font(cx, "font-spec", SPEC_SLOTS)?;
        let text = cx.string_view(name)?.bytes().to_vec();
        if !parse_name(cx, &text, spec)? { return Ok(Value::NIL); }
        spec
    };
    let Some(object) = open(cx, font)? else { return Ok(Value::NIL) };
    let get = |cx: &Leaf<'_>, index: usize| slot(cx, object, index);
    let (ascent, descent) = (get(cx, ASCENT).integer_value().unwrap_or(0), get(cx, DESCENT).integer_value().unwrap_or(0));
    let integer = |n: i64| Value::integer(n).unwrap();
    let values = [get(cx, NAME), get(cx, FULLNAME), get(cx, SIZE), integer(ascent + descent), integer(0), integer(0), integer(0),
        get(cx, MAX_WIDTH), integer(ascent), integer(descent), get(cx, SPACE_WIDTH), get(cx, AVERAGE_WIDTH), get(cx, FILE),
        get(cx, CAPABILITY)];
    Ok(cx.vector(&values))
}}

/// The font and glyph code the frame draws CH with: (FONT-OBJECT . CODE).
fn char_font(cx: &mut Leaf<'_>, ch: i64) -> Out {
    let Some(ch) = u32::try_from(ch).ok().and_then(char::from_u32) else { return Ok(Value::NIL) };
    let request = cx.default_font_request()?;
    let Some((opened, code)) = cx.font_host().and_then(|host| host.char_font(&request, ch)) else { return Ok(Value::NIL) };
    let object = make_object(cx, &opened)?;
    Ok(cx.cons(object, Value::integer(i64::from(code)).unwrap()))
}

defsubr! { name = "internal-char-font", gnu = "font.c:4762", effect = Reads,
fn internal_char_font(cx, position: Value; optional ch: Value) -> Step {
    if position == Value::NIL {
        let Some(c) = ch.integer_value().filter(|c| (0..=0x3fffff).contains(c)) else { return Err(Fault::Wrong("characterp", ch)) };
        // Without a display, terminal_glyph_code: the initial terminal has none.
        if cx.font_host().is_none() { return Ok(Step::Done(Value::NIL)); }
        return Ok(Step::Done(char_font(cx, c)?));
    }
    let shape = cx.buffer_shape()?;
    let at = position.integer_value().or_else(|| cx.marker_position(position).flatten())
        .ok_or(Fault::Wrong("integer-or-marker-p", position))?;
    if !(shape.begv <= at && at < shape.zv) {
        return Err(Fault::Range3(position, Value::integer(shape.begv).unwrap(), Value::integer(shape.zv).unwrap()));
    }
    if ch != Value::NIL && !ch.integer_value().is_some_and(|c| c >= 0) { return Err(Fault::Wrong("wholenump", ch)); }
    let buffer = cx.current_buffer()?;
    if cx.windows().get_buffer_window(buffer, Value::NIL)? == Value::NIL || cx.font_host().is_none() {
        return Ok(Step::Done(Value::NIL));
    }
    if ch != Value::NIL { return Ok(Step::Done(char_font(cx, ch.integer_value().unwrap())?)); }
    let f = cx.intern("char-after");
    Ok(Step::Call { f, args: vec![Value::integer(at).unwrap()], then: font_of_char, saved: vec![] })
}}
fn font_of_char(cx: &mut Leaf<'_>, ch: Value, _: &[Value]) -> Next {
    Ok(Step::Done(match ch.integer_value() { Some(c) => char_font(cx, c)?, None => Value::NIL }))
}

defsubr! { name = "font-at", gnu = "font.c:5503", effect = Reads,
fn font_at(cx, position: Value; optional window: Value, string: Value) -> Step {
    // decode_live_window, then the window's buffer.
    let f = cx.intern("window-buffer");
    Ok(Step::Call { f, args: vec![window], then: font_at_window, saved: vec![position, string] })
}}
fn font_at_window(cx: &mut Leaf<'_>, buffer: Value, saved: &[Value]) -> Next {
    let (position, string) = (saved[0], saved[1]);
    if string == Value::NIL {
        if buffer != cx.current_buffer()? { return Err(error(cx, "Specified window is not displaying the current buffer".into())); }
        let shape = cx.buffer_shape()?;
        let at = position.integer_value().or_else(|| cx.marker_position(position).flatten())
            .ok_or(Fault::Wrong("integer-or-marker-p", position))?;
        if !(shape.begv <= at && at < shape.zv) {
            return Err(Fault::Range3(position, Value::integer(shape.begv).unwrap(), Value::integer(shape.zv).unwrap()));
        }
        if cx.font_host().is_none() { return Ok(Step::Done(Value::NIL)); }
        let f = cx.intern("char-after");
        return Ok(Step::Call { f, args: vec![Value::integer(at).unwrap()], then: font_object_of_char, saved: vec![] });
    }
    let at = position.integer_value().ok_or(Fault::Wrong("fixnump", position))?;
    let text = cx.string_view(string)?;
    let ch = usize::try_from(at).ok().and_then(|at| text.chars().nth(at));
    let Some(ch) = ch else { return Err(Fault::Range(string, position)) };
    if cx.font_host().is_none() { return Ok(Step::Done(Value::NIL)); }
    font_object_of_char(cx, Value::integer(i64::from(ch)).unwrap(), &[])
}
fn font_object_of_char(cx: &mut Leaf<'_>, ch: Value, _: &[Value]) -> Next {
    let found = match ch.integer_value() { Some(c) => char_font(cx, c)?, None => Value::NIL };
    Ok(Step::Done(cx.pair(found).map_or(Value::NIL, |(object, _)| object)))
}
