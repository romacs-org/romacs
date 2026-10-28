//! GNU xfaces.c alternative family and registry setters. Only list spines are copied;
//! interning is deliberately ordered after each complete spine validation.
use super::*;

fn proper_spine(cx: &Leaf<'_>, original: Value) -> std::result::Result<Vec<Value>, Fault> {
    let mut values = Vec::new();
    let mut seen = HashSet::default();
    let mut tail = original;
    while let Some((car, cdr)) = cx.pair(tail) {
        if !seen.insert(tail.0) {
            return Err(Fault::Circular(original));
        }
        values.push(car);
        tail = cdr;
    }
    if tail != Value::NIL {
        return Err(Fault::Wrong("listp", tail));
    }
    Ok(values)
}

fn copied_list(cx: &mut Leaf<'_>, values: Vec<Value>) -> Value {
    values.into_iter().rev().fold(Value::NIL, |tail, car| cx.cons(car, tail))
}

defsubr! { name = "internal-set-alternative-font-family-alist", gnu = "xfaces.c:5921", effect = Global,
fn set_alternative_font_family_alist(cx, alist: Value) {
    // GNU copies the entire outer spine first, then copies each inner spine
    // before converting that row. Earlier successful interns survive failure.
    let rows = proper_spine(cx, alist)?;
    let mut converted = Vec::with_capacity(rows.len());
    let mut names = Vec::with_capacity(rows.len());
    for row in rows {
        let members = proper_spine(cx, row)?;
        let mut symbols = Vec::with_capacity(members.len());
        let mut row_names = Vec::with_capacity(members.len());
        for name in members {
            // CHECK_STRING precedes Fintern's current-obarray resolution.
            cx.string_view(name)?;
            let table = cx.resolve_obarray(Value::NIL)?;
            let symbol = cx.intern_in(table, name)?;
            row_names.push(cx.symbol_name(symbol)?);
            symbols.push(symbol);
        }
        converted.push(copied_list(cx, symbols));
        names.push(row_names);
    }
    let result = copied_list(cx, converted);
    cx.set_font_alternative_families(result, names);
    Ok(result)
}}

defsubr! { name = "internal-set-alternative-font-registry-alist", gnu = "xfaces.c:5940", effect = Global,
fn set_alternative_font_registry_alist(cx, alist: Value) {
    // Copy the outer spine, then each entry's spine, downcasing its members.
    // Romacs font selection does not consult registries yet; the copy is kept
    // traced, as GNU keeps it staticpro'd, and realized faces are discarded.
    let rows = proper_spine(cx, alist)?;
    let mut converted = Vec::with_capacity(rows.len());
    for row in rows {
        let mut members = Vec::new();
        for member in proper_spine(cx, row)? {
            members.push(cx.case_convert(member, true)?);
        }
        converted.push(copied_list(cx, members));
    }
    let result = copied_list(cx, converted);
    cx.set_font_alternative_registries(result);
    Ok(result)
}}
