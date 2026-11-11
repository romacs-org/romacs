//! GNU fontset.c: fontsets as `crate::fontsets::Registry` holds them. The
//! rules live there as plain data; what reads them to paint is romacs-font's
//! resolver. `set-fontset-font' and `fontset-info' are Lisp over these
//! leaves (lisp/romacs/romacs-frame.el): a change of a frame's ASCII font goes
//! through its default face.
use super::font;
use super::*;
use romacs_layout_input::{FontDef, FontsetMode, FontsetTarget};
use std::sync::Arc;

const SCRIPT_TABLE: &str = "#:vm2-script-table";
const STAGED_ALIASES: &str = "#:vm2-staged-fontset-aliases";
/// `fontset-alias-alist' as these primitives last left it: while it stands,
/// the registry's name index knows every alias.
const INDEXED_ALIASES: &str = "#:vm2-indexed-fontset-aliases";

/// `fontset-alias-alist' set to ALIASES, which the name index matches.
fn set_aliases(cx: &mut Leaf<'_>, aliases: Value) -> std::result::Result<(), Fault> {
    cx.set_hidden_value(INDEXED_ALIASES, aliases);
    cx.store_variable_named("fontset-alias-alist", aliases)
}

fn window_system(cx: &mut Leaf<'_>) -> std::result::Result<(), Fault> {
    if cx.font_host().is_some() { Ok(()) } else { Err(Fault::Error("Window system is not in use or not initialized".into())) }
}

/// A string's text in lower case (Fdowncase).
fn downcased(cx: &Leaf<'_>, name: Value) -> std::result::Result<String, Fault> {
    Ok(cx.string_text(name)?.to_lowercase())
}

/// fontset_pattern_regexp: PATTERN's regular expression, when it has a
/// wildcard.
fn pattern_regexp(pattern: &str) -> Option<String> {
    if !pattern.contains(['*', '?']) { return None; }
    let star = if pattern.matches('-').count() < 14 { ".*" } else { "[^-]*" };
    let mut regexp = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => regexp.push_str(star),
            '?' => regexp.push('.'),
            '[' | '.' | '\\' | '+' | '^' | '$' => { regexp.push('\\'); regexp.push(c); }
            c => regexp.push(c),
        }
    }
    regexp.push('$');
    Some(regexp)
}

/// How fs_query_fontset takes NAME.
#[derive(Clone, Copy, PartialEq)]
enum Pattern { Wildcard, Regexp, Literal }

/// fs_query_fontset: the base fontset NAME names. A literal name or alias
/// is one lookup; `fontset-alias-alist', which Lisp may have changed, and
/// patterns walk what they must.
fn query(cx: &mut Leaf<'_>, name: Value, how: Pattern) -> std::result::Result<Option<usize>, Fault> {
    let lower = downcased(cx, name)?;
    if how != Pattern::Regexp {
        if let Some(id) = cx.fontsets().named(&lower) { return Ok(Some(id)); }
    }
    let mut full = None;
    let aliases = super::fileio::var(cx, "fontset-alias-alist")?;
    if how != Pattern::Regexp && aliases != cx.hidden_value(INDEXED_ALIASES) {
        let mut tail = aliases;
        while let Some((pair, rest)) = cx.pair(tail) {
            if let Some((car, cdr)) = cx.pair(pair)
                && [car, cdr].into_iter().any(|side| matches!(cx.kind(side), Kind::String | Kind::Bytes) && cx.string_text(side).is_ok_and(|t| t.eq_ignore_ascii_case(&lower)))
                && matches!(cx.kind(car), Kind::String | Kind::Bytes) {
                full = Some(cx.string_text(car)?);
                break;
            }
            tail = rest;
        }
    }
    if let Some(full) = &full {
        return Ok(cx.fontsets().named(full));
    }
    let regexp = match how {
        Pattern::Literal => return Ok(None),
        Pattern::Wildcard => match pattern_regexp(&lower) { Some(regexp) => regexp, None => return Ok(None) },
        Pattern::Regexp => lower,
    };
    let regexp = cx.string(&regexp);
    let names: Vec<(usize, String)> = cx.fontsets().iter().map(|(id, set)| (id, set.name.clone())).collect();
    for (id, name) in names {
        let name = cx.string(&name);
        if cx.file_completion_regexp_match(regexp, name, true)? { return Ok(Some(id)); }
    }
    Ok(None)
}

/// check_fontset_name: the fontset NAME and FRAME name.
fn check_name(cx: &mut Leaf<'_>, name: Value, frame: Value) -> std::result::Result<usize, Fault> {
    cx.face_frame(frame, true)?;
    if name == Value::TRUE { return Ok(0); }
    if name == Value::NIL {
        if cx.font_host().is_none() { return Err(Fault::Error("Can\u{2019}t use fontsets in non-GUI frames".into())); }
        return Ok(cx.fontsets().frame);
    }
    if !matches!(cx.kind(name), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", name)); }
    if let Some(id) = query(cx, name, Pattern::Literal)? { return Ok(id); }
    if let Some(id) = query(cx, name, Pattern::Wildcard)? { return Ok(id); }
    Err(Fault::Error(format!("Fontset \u{2018}{}\u{2019} does not exist", cx.string_text(name)?)))
}

/// find_font_encoding's answer for NAME: the encoding charset's id, ascii
/// for no match.
fn font_encoding(cx: &mut Leaf<'_>, name: &str) -> std::result::Result<u32, Fault> {
    let mut tail = super::fileio::var(cx, "font-encoding-alist")?;
    let text = cx.string(name);
    while let Some((element, rest)) = cx.pair(tail) {
        tail = rest;
        let Some((regexp, value)) = cx.pair(element) else { continue };
        if !matches!(cx.kind(regexp), Kind::String | Kind::Bytes) { continue; }
        let encoding = cx.pair(value).map_or(value, |(encoding, _)| encoding);
        if cx.kind(encoding) == Kind::Symbol && cx.is_charset(encoding) && cx.file_completion_regexp_match(regexp, text, true)? {
            return charset_id(cx, encoding);
        }
    }
    let ascii = cx.intern("ascii");
    charset_id(cx, ascii)
}

fn charset_id(cx: &mut Leaf<'_>, charset: Value) -> std::result::Result<u32, Fault> {
    Ok(cx.charset_id_internal(charset)?.integer_value().unwrap_or(0) as u32)
}

/// The encodings found during one call, whose specs often repeat a registry.
type Encodings = std::collections::HashMap<String, u32>;

/// An Fset_fontset_font FONT-SPEC argument as a font-def, None for nil, and
/// the font's name.
fn font_def(cx: &mut Leaf<'_>, spec: Value, encodings: &mut Encodings) -> std::result::Result<Option<FontDef>, Fault> {
    let spec = if spec == Value::NIL { return Ok(None) } else if let Some((family, registry)) = cx.pair(spec) {
        font::spec_from_family_registry(cx, family, registry)?
    } else if matches!(cx.kind(spec), Kind::String | Kind::Bytes) {
        font::spec_from_name(cx, spec)?
    } else if font::is_spec(cx, spec) {
        spec
    } else {
        // Fsignal (Qfont, ...), which is no error symbol.
        let message = cx.string("Invalid error symbol");
        let symbol = cx.intern("font");
        let data = cx.coding_list(&[message, symbol]);
        return Err(Fault::Signal(cx.intern("error"), data));
    };
    let mut def = font::font_def_of(cx, spec)?;
    let name = format!("{}-{}", def.family.as_deref().unwrap_or(""), def.registry.as_deref().unwrap_or(""));
    def.encoding = match encodings.get(&name) {
        Some(&encoding) => encoding,
        None => {
            let encoding = font_encoding(cx, &name)?;
            encodings.insert(name, encoding);
            encoding
        }
    };
    Ok(Some(def))
}

/// `char-script-table''s ranges of SCRIPT: one walk of the table serves
/// every script until the table changes.
fn script_ranges(cx: &mut Leaf<'_>, script: Value) -> std::result::Result<Arc<Vec<(u32, u32)>>, Fault> {
    let table = super::fileio::var(cx, "char-script-table")?;
    if table != cx.hidden_value(SCRIPT_TABLE) {
        cx.set_hidden_value(SCRIPT_TABLE, table);
        cx.fontsets_mut().script(0, u64::MAX);
    }
    let edits = cx.char_table_edits(table)?;
    if let Some(ranges) = cx.fontsets_mut().script(script.0, edits) { return Ok(ranges); }
    let mut scripts: std::collections::HashMap<u64, Vec<(u32, u32)>> = Default::default();
    for (from, to, value) in cx.char_table_runs(table)? { scripts.entry(value.0).or_default().push((from, to)); }
    let ranges = Arc::new(scripts.get(&script.0).cloned().unwrap_or_default());
    cx.fontsets_mut().set_scripts(scripts.into_iter().map(|(key, runs)| (key, Arc::new(runs))).collect());
    Ok(ranges)
}

fn listed(cx: &Leaf<'_>, mut list: Value, item: Value) -> bool {
    while let Some((head, rest)) = cx.pair(list) {
        if head == item { return true; }
        list = rest;
    }
    false
}

fn character(cx: &Leaf<'_>, value: Value) -> std::result::Result<u32, Fault> {
    let _ = cx;
    value.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c)).map(|c| c as u32).ok_or(Fault::Wrong("characterp", value))
}

/// TARGETS with overlapping ranges joined, as set_fontset_font folds a
/// script's ranges into a charset's: a character gets each font once.
fn merged_targets(targets: Vec<FontsetTarget>) -> Vec<FontsetTarget> {
    let (mut ranges, mut others) = (Vec::new(), Vec::new());
    for target in targets {
        match target {
            FontsetTarget::Range(from, to) => ranges.push((from.min(to), from.max(to))),
            other => others.push(other),
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (from, to) in ranges {
        match merged.last_mut() {
            Some(last) if from <= last.1.saturating_add(1) => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    others.extend(merged.into_iter().map(|(from, to)| FontsetTarget::Range(from, to)));
    others
}

/// set_fontset_font's work on ID: ADD's mode, what CHARACTERS and SPEC name.
fn add_font(cx: &mut Leaf<'_>, id: usize, characters: Value, spec: Value, add: Value, encodings: &mut Encodings) -> std::result::Result<bool, Fault> {
    let def = font_def(cx, spec, encodings)?;
    let mut ascii_changed = false;
    let mut targets: Vec<FontsetTarget> = Vec::new();
    if characters == Value::NIL {
        targets.push(FontsetTarget::Fallback);
    } else if characters.integer_value().is_some_and(|c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c)) {
        let c = character(cx, characters)?;
        if c < 0x80 { return Err(Fault::Error("Can\u{2019}t set a font for partial ASCII range".into())); }
        targets.push(FontsetTarget::Range(c, c));
    } else if let Some((from, to)) = cx.pair(characters) {
        let (from, to) = (character(cx, from)?, character(cx, to)?);
        if from < 0x80 {
            if from != 0 || to < 0x7F { return Err(Fault::Error("Can\u{2019}t set a font for partial ASCII range".into())); }
            ascii_changed = true;
        }
        targets.push(FontsetTarget::Range(from, to));
    } else if cx.kind(characters) == Kind::Symbol {
        let table = super::fileio::var(cx, "char-script-table")?;
        let zero = Value::integer(0).unwrap();
        let scripts = cx.char_table_extra(table, zero, None)?;
        let script = listed(cx, scripts, characters);
        if script {
            if cx.symbol_name(characters)? == "latin" { ascii_changed = true; }
            targets.extend(script_ranges(cx, characters)?.iter().map(|&(from, to)| FontsetTarget::Range(from, to)));
        }
        if cx.is_charset(characters) {
            if cx.charset_ascii_compatible(characters)? { ascii_changed = true; }
            // Set_fontset_font: the charset's ranges and the script's, with
            // ASCII only when the charset changes the ASCII font.
            let runs = cx.charset_map_calls(characters, Value::NIL, Value::NIL)?;
            targets.extend(runs.into_iter().map(|run| FontsetTarget::Range(run.start as u32, run.end as u32)));
        } else if !script {
            return Err(Fault::Error(format!("Invalid script or charset name: {}", cx.symbol_name(characters)?)));
        }
    } else {
        return Err(Fault::Error("Invalid second argument for setting a font in a fontset".into()));
    }
    let mode = if add == Value::NIL { FontsetMode::Overwrite } else if cx.symbol_name(add).is_ok_and(|name| name == "prepend") { FontsetMode::Prepend } else { FontsetMode::Append };
    if ascii_changed {
        if spec == Value::NIL { return Err(Fault::Error("Can\u{2019}t set ASCII font to nil".into())); }
        let has_ascii = cx.fontsets().get(id).get(0).is_some();
        if has_ascii && mode == FontsetMode::Append { ascii_changed = false; }
    }
    if cx.is_charset(characters) && characters != Value::NIL && !ascii_changed {
        // map_charset_chars's set_fontset_font leaves ASCII alone.
        targets = targets.into_iter().filter_map(|target| match target {
            FontsetTarget::Range(from, to) if from < 0x80 => (to >= 0x80).then_some(FontsetTarget::Range(0x80, to)),
            other => Some(other),
        }).collect();
    }
    let entry = def.clone().map(Arc::new);
    let fontset = cx.fontsets_mut().edit(id);
    for target in merged_targets(targets) { fontset.add(target, entry.clone(), mode); }
    if ascii_changed { fontset.ascii = def.map(|def| def.name); }
    Ok(ascii_changed)
}

// Fset_fontset_font but for the frames: t when the ASCII font of the
// frame's fontset changed, which `set-fontset-font' applies.
defsubr! { name = "romacs--set-fontset-font", gnu = "fontset.c:1461", effect = Global,
fn romacs_set_fontset_font(cx, fontset: Value, characters: Value, font_spec: Value; optional frame: Value, add: Value) {
    let id = check_name(cx, fontset, frame)?;
    let changed = add_font(cx, id, characters, font_spec, add, &mut Encodings::new())?;
    cx.note_fontset_change();
    Ok(if changed && id == cx.fontsets().frame { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "new-fontset", gnu = "fontset.c:1711", effect = Global,
fn new_fontset(cx, name: Value, fontlist: Value) {
    if !matches!(cx.kind(name), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", name)); }
    let lower = downcased(cx, name)?;
    let name = cx.string(&lower);
    let id = match query(cx, name, Pattern::Wildcard)? {
        Some(id) => { cx.fontsets_mut().edit(id).clear_rules(); id }
        None => {
            let (short, ascii) = font::fontset_name_parts(cx, &lower)?;
            let id = cx.fontsets_mut().make(&lower);
            cx.fontsets_mut().alias(&short, id);
            let short = cx.string(&short);
            let pair = cx.cons(name, short);
            let aliases = super::fileio::var(cx, "fontset-alias-alist")?;
            let aliases = cx.cons(pair, aliases);
            set_aliases(cx, aliases)?;
            cx.fontsets_mut().edit(id).ascii = Some(ascii);
            id
        }
    };
    let append = cx.intern("append");
    let mut encodings = Encodings::new();
    let mut tail = fontlist;
    while let Some((element, rest)) = cx.pair(tail) {
        if element != Value::NIL && cx.pair(element).is_none() { return Err(Fault::Wrong("listp", element)); }
        let (script, specs) = cx.pair(element).unwrap_or((Value::NIL, Value::NIL));
        let is_list = cx.pair(specs).is_some_and(|(_, next)| next == Value::NIL || cx.pair(next).is_some());
        if is_list {
            let mut specs = specs;
            while let Some((spec, next)) = cx.pair(specs) {
                add_font(cx, id, script, spec, append, &mut encodings)?;
                specs = next;
            }
        } else {
            add_font(cx, id, script, specs, append, &mut encodings)?;
        }
        tail = rest;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", fontlist)); }
    cx.note_fontset_change();
    Ok(name)
}}

defsubr! { name = "query-fontset", gnu = "fontset.c:1237", effect = Reads,
fn query_fontset(cx, pattern: Value; optional regexpp: Value) {
    window_system(cx)?;
    if !matches!(cx.kind(pattern), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", pattern)); }
    if cx.string_text(pattern)?.is_empty() { return Ok(Value::NIL); }
    let how = if regexpp == Value::NIL { Pattern::Wildcard } else { Pattern::Regexp };
    Ok(match query(cx, pattern, how)? {
        Some(id) => { let name = cx.fontsets().get(id).name.clone(); cx.string(&name) }
        None => Value::NIL,
    })
}}

defsubr! { name = "fontset-list", gnu = "fontset.c:2083", effect = Reads,
fn fontset_list(cx) {
    let names: Vec<String> = cx.fontsets().iter().map(|(_, set)| set.name.clone()).collect();
    let values: Vec<Value> = names.iter().rev().map(|name| cx.string(name)).collect();
    Ok(cx.coding_list(&values))
}}

// Ffontset_font: the first (FAMILY . REGISTRY) for CH, or with ALL each one,
// in the fontset and then the default fontset; a nil entry ends the search
// with nil, and a def whose encoding lacks CH is passed over, which
// fontset.c names repertory but reads from the def's encoding slot.
defsubr! { name = "fontset-font", gnu = "fontset.c:2013", effect = Reads,
fn fontset_font(cx, name: Value, ch: Value; optional all: Value) {
    let id = check_name(cx, name, Value::NIL)?;
    let c = character(cx, ch)?;
    let mut list = Vec::new();
    let mut set = id;
    loop {
        let (own, fallback) = {
            let fontset = cx.fontsets().get(set);
            (fontset.get(c).cloned(), fontset.fallback().cloned())
        };
        let own = own.or_else(|| (set != 0).then(|| cx.fontsets().get(0).get(c).cloned()).flatten());
        for group in [own, fallback].into_iter().flatten() {
            for entry in group.to_vec() {
                let Some(def) = entry else { return Ok(Value::NIL) };
                if !cx.char_in_charset_id(def.encoding as usize, c)? { continue; }
                let text = |cx: &mut Leaf<'_>, text: &Option<String>| text.as_deref().map_or(Value::NIL, |text| cx.string(text));
                let (family, registry) = (text(cx, &def.family), text(cx, &def.registry));
                let pair = cx.cons(family, registry);
                if all == Value::NIL { return Ok(pair); }
                list.push(pair);
            }
        }
        if set == 0 { break; }
        set = 0;
    }
    Ok(cx.coding_list(&list))
}}

/// The names of the fonts defs were realized to, looked up once per call:
/// a def recurs in many runs.
type OpenedNames = std::collections::HashMap<romacs_layout_input::OpenedDefFont, Option<String>>;

/// The alternatives of a group as `fontset-info' lists them: (FONT
/// OPENED-FONT ...), the nils left out.
fn info_alist(cx: &mut Leaf<'_>, group: &romacs_layout_input::Alternatives, names: &mut OpenedNames) -> Out {
    let state = cx.fontset_state();
    let mut items = Vec::new();
    for def in group.to_vec().into_iter().flatten() {
        let mut item = vec![cx.string(&def.name)];
        for opened in state.opened(&def) {
            let name = match names.get(&opened) {
                Some(name) => name.clone(),
                None => {
                    let name = font::opened_name(cx, &opened)?;
                    names.insert(opened, name.clone());
                    name
                }
            };
            if let Some(name) = name { item.push(cx.string(&name)); }
        }
        items.push(cx.coding_list(&item));
    }
    Ok(cx.coding_list(&items))
}

/// (FALLBACK (FROM TO . ALIST) ...) for a fontset's table.
fn info_table(cx: &mut Leaf<'_>, id: usize, names: &mut OpenedNames) -> Out {
    let fontset = cx.fontsets().get(id).clone();
    let fallback = match fontset.fallback() { Some(group) => info_alist(cx, group, names)?, None => Value::NIL };
    let mut runs = Vec::new();
    fontset.runs(|from, to, group| runs.push((from, to, group.clone())));
    let mut items = vec![fallback];
    for (from, to, group) in runs {
        let alist = info_alist(cx, &group, names)?;
        let tail = cx.cons(Value::integer(i64::from(to)).unwrap(), alist);
        items.push(cx.cons(Value::integer(i64::from(from)).unwrap(), tail));
    }
    Ok(cx.coding_list(&items))
}

// What `fontset-info' tabulates: FONTSET's table and, unless it is the
// default fontset, the default's: (OWN . DEFAULT).
defsubr! { name = "romacs--fontset-info", gnu = "fontset.c:1875", effect = Reads,
fn romacs_fontset_info(cx, fontset: Value; optional frame: Value) {
    window_system(cx)?;
    let id = check_name(cx, fontset, frame)?;
    let mut names = OpenedNames::new();
    let own = info_table(cx, id, &mut names)?;
    let shared = if id == 0 { Value::NIL } else { info_table(cx, 0, &mut names)? };
    Ok(cx.cons(own, shared))
}}

// find_font_encoding: what `font-encoding-alist' gives NAME, nil for no match.
defsubr! { name = "romacs--font-encoding", gnu = "font.c:497", effect = Reads,
fn romacs_font_encoding(cx, name: Value) {
    let mut tail = super::fileio::var(cx, "font-encoding-alist")?;
    while let Some((element, rest)) = cx.pair(tail) {
        tail = rest;
        let Some((regexp, value)) = cx.pair(element) else { continue };
        if !matches!(cx.kind(regexp), Kind::String | Kind::Bytes) { continue; }
        if cx.file_completion_regexp_match(regexp, name, true)? { return Ok(value); }
    }
    Ok(Value::NIL)
}}

// fontset_from_font's table work: a fontset NAME with ALIASES, its ASCII font
// FONT-NAME, made the frame's. `romacs--setup-fontsets' fills its rules.
defsubr! { name = "romacs--make-frame-fontset", gnu = "fontset.c:1790", effect = Global,
fn romacs_make_frame_fontset(cx, name: Value, aliases: Value, font_name: Value) {
    let text = cx.string_text(name)?;
    let id = match cx.fontsets().named(&text) { Some(id) => id, None => cx.fontsets_mut().make(&text) };
    let mut alist = super::fileio::var(cx, "fontset-alias-alist")?;
    let mut tail = aliases;
    while let Some((alias, rest)) = cx.pair(tail) {
        let alias_text = cx.string_text(alias)?;
        cx.fontsets_mut().alias(&alias_text, id);
        let pair = cx.cons(name, alias);
        alist = cx.cons(pair, alist);
        tail = rest;
    }
    set_aliases(cx, alist)?;
    let ascii = cx.string_text(font_name)?;
    let registry = cx.fontsets_mut();
    registry.edit(id).ascii = Some(ascii);
    registry.frame = id;
    cx.note_fontset_change();
    Ok(name)
}}

// Put the fontsets made so far aside: the loadup chain makes the default
// rules once, and a batch session starts without them, as GNU's.
defsubr! { name = "romacs--stage-fontsets", gnu = "romacs:fontset", effect = Global,
fn romacs_stage_fontsets(cx) {
    let aliases = super::fileio::var(cx, "fontset-alias-alist")?;
    cx.set_hidden_value(STAGED_ALIASES, aliases);
    cx.fontsets_mut().stage();
    let default = cx.string(crate::fontsets::DEFAULT_NAME);
    let short = cx.string("fontset-default");
    let pair = cx.cons(default, short);
    let list = cx.coding_list(&[pair]);
    set_aliases(cx, list)?;
    Ok(Value::NIL)
}}

// The staged fontsets in place of what was written since: t, or nil when
// none were staged.
defsubr! { name = "romacs--unstage-fontsets", gnu = "romacs:fontset", effect = Global,
fn romacs_unstage_fontsets(cx) {
    if !cx.fontsets_mut().unstage() { return Ok(Value::NIL); }
    let aliases = cx.hidden_value(STAGED_ALIASES);
    set_aliases(cx, aliases)?;
    cx.note_fontset_change();
    Ok(Value::TRUE)
}}
