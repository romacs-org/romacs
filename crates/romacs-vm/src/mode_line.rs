//! GNU's C mode-line engine: `display_mode_element` (`src/xdisp.c:28458`)
//! and `decode_mode_spec` (`src/xdisp.c:29429`), for the painted band
//! (`MODE_LINE_DISPLAY`) and for `format-mode-line`'s string
//! (`MODE_LINE_STRING`, `MODE_LINE_NOPROP`). The format is converted to an owned `Form` before
//! expansion. Executable object constants own collector-traced roots across
//! collecting `:eval` forms; window and buffer are re-read from rooted slots.
use crate::*;
use std::ops::Range;

/// GNU's `mode_line_target`: the band a window paints, or the string
/// `format-mode-line' returns, with face properties or without.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target { Display, String, NoProp }

/// One expanded mode line: text plus face-name runs over its bytes.
pub(super) struct Band {
    pub text: String,
    pub runs: Vec<(Range<usize>, String)>,
    pub target: Target,
    /// GNU's `mode_line_elt_no` for a string: the count of elements entered
    /// so far, and the count each piece was stored under, its
    /// `mode-line-elt-no' property. A `format-mode-line' inside an `:eval'
    /// restarts the count, as GNU's global does.
    pub elt: i64,
    pub elts: Vec<(Range<usize>, i64)>,
    /// Columns already built, and the most that can ever be shown. GNU's
    /// display_string stops at the window's last visible column
    /// (`xdisp.c:30105`); text past it is never produced, so no field width
    /// can make the band allocate more than this.
    cols: usize,
    max_cols: usize,
}

/// What `decode_mode_spec` reads from the window and its buffer, taken once
/// when the build starts (char positions are 1-based, as in Lisp).
pub(super) struct SpecState {
    pub buffer_name: String,
    pub start: usize,
    pub end: usize,
    pub begv: usize,
    pub zv: usize,
    pub z: usize,
    pub line: usize,
    pub column: usize,
    pub modified: bool,
    pub frame_cols: usize,
}

/// Field widths and precisions, after GNU's C `int` wrap, are clamped here:
/// anything wider than the band's `4 * frame columns` is invisible anyway,
/// and the clamp keeps every column count in this file far from i64 overflow.
const MAX_FIELD: i64 = 1 << 20;

/// The slot where a `format-mode-line' leaves its final element count.
pub(super) const ELT_RESTARTED: &str = "#:vm2-mode-line-elt-no";

/// GNU `lots_of_dashes` (`xdisp.c:29426`).
const LOTS_OF_DASHES: usize = 140;

/// GNU `char_width` as `c_string_width` uses it (`character.h`
/// `ASCII_CHAR_WIDTH`): TAB is `tab-width` columns, other control characters
/// and DEL 2 (`ctl-arrow` t, VM2's `^X` form), newline 0.
fn char_width(c: char, tab: usize) -> usize {
    match c {
        '\t' => tab,
        '\n' => 0,
        '\u{0}'..='\u{1f}' | '\u{7f}' => 2,
        _ => unicode_width::UnicodeWidthChar::width(c).unwrap_or(1),
    }
}

/// The first `max` characters of `s` (GNU `store_mode_line_string`'s
/// precision, which counts SCHARS).
fn char_prefix(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(at, _)| &s[..at])
}

fn chars(s: &str) -> i64 {
    s.chars().count() as i64
}

/// Longest prefix of `s` no wider than `max` columns (GNU `c_string_width`
/// selecting a format string's literal run).
fn prefix(s: &str, max: usize, tab: usize) -> &str {
    let mut used = 0;
    for (at, c) in s.char_indices() {
        let w = char_width(c, tab);
        if used + w > max {
            return &s[..at];
        }
        used += w;
    }
    s
}

/// GNU `pint2str`: right-justified decimal in WIDTH columns.
fn pint2str(width: i64, d: i64) -> String {
    format!("{:>1$}", d.max(0), width.max(0) as usize)
}

/// GNU `pint2hrstr` (`xdisp.c:29234`): 1.7k style, right-justified.
fn pint2hrstr(width: i64, d: i64) -> String {
    const POWER: [&str; 11] = ["", "k", "M", "G", "T", "P", "E", "Z", "Y", "R", "Q"];
    let (mut quotient, mut remainder, mut tenths, mut exponent) = (d.max(0), 0i64, -1i64, 0usize);
    if quotient >= 1000 {
        loop {
            remainder = quotient % 1000;
            quotient /= 1000;
            exponent += 1;
            if quotient < 1000 {
                break;
            }
        }
        if quotient <= 9 {
            tenths = remainder / 100;
            if remainder % 100 >= 50 {
                if tenths < 9 {
                    tenths += 1;
                } else {
                    quotient += 1;
                    tenths = if quotient == 10 { -1 } else { 0 };
                }
            }
        } else if remainder >= 500 {
            if quotient < 999 {
                quotient += 1;
            } else {
                quotient = 1;
                exponent += 1;
                tenths = 0;
            }
        }
    }
    let number = if tenths >= 0 { format!("{quotient}.{tenths}") } else { quotient.to_string() };
    format!("{number:>0$}{1}", width.max(0) as usize, POWER[exponent.min(10)])
}

/// GNU `percent99`.
fn percent99(n: usize, d: usize) -> usize {
    (((d as f64 - 1.0 + 100.0 * n as f64) / d as f64) as usize).min(99)
}

/// FORM's car and cdr, taken apart rather than copied.
fn split(form: Form) -> Option<(Form, Form)> {
    match form {
        Form::List(mut items) if !items.is_empty() => {
            let car = items.remove(0);
            Some((car, if items.is_empty() { Form::Sym("nil".into()) } else { Form::List(items) }))
        }
        Form::Dotted(mut items, tail) if !items.is_empty() => {
            let car = items.remove(0);
            Some((car, if items.is_empty() { *tail } else { Form::Dotted(items, tail) }))
        }
        _ => None,
    }
}

/// How `display_mode_element' treats a list by its car.
enum Head { Eval, Propertize, Symbol, Limit(i64), Each, Other }

fn is_nil(form: &Form) -> bool {
    matches!(form, Form::Sym(s) if s == "nil")
}

fn is_cons(form: &Form) -> bool {
    matches!(form, Form::List(items) | Form::Dotted(items, _) if !items.is_empty())
}

/// The `face` of a `:propertize` plist: a face symbol, or the first symbol of
/// a face list. Anonymous face specs are not projected (VM2-G1B).
fn face_of(props: &Form) -> Option<String> {
    let Form::List(items) = props else { return None };
    let value = items.chunks(2).find(|pair| matches!(&pair[0], Form::Sym(s) if s == "face"))?.get(1)?;
    match value {
        Form::Sym(s) if s != "nil" => Some(s.clone()),
        Form::List(faces) => faces.iter().find_map(|f| match f {
            Form::Sym(s) if s != "nil" && !s.starts_with(':') => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }
}

impl Band {
    /// `n` columns of padding, never more than can still be shown.
    fn pad(&mut self, n: i64, face: Option<&String>) {
        let n = usize::try_from(n).unwrap_or(0).min(self.max_cols - self.cols);
        self.push(&" ".repeat(n), face);
    }

    /// An element's own padding, which GNU stores with no element number.
    fn pad_element(&mut self, n: i64) {
        let begin = self.text.len();
        self.pad(n, None);
        if self.elts.last().is_some_and(|(range, _)| range.start >= begin) { self.elts.pop(); }
    }

    /// Append text. A painted band clips it at `max_cols` and shows control
    /// characters as GNU's `^X`; a string keeps them all
    /// (`store_mode_line_string`, `store_mode_line_noprop`).
    fn push(&mut self, s: &str, face: Option<&String>) {
        let begin = self.text.len();
        if self.target == Target::Display { self.push_clipped(s) } else { self.text.push_str(s) }
        if self.text.len() == begin || self.target == Target::NoProp { return; }
        if let Some(face) = face { self.runs.push((begin..self.text.len(), face.clone())); }
        if self.target == Target::String { self.elts.push((begin..self.text.len(), self.elt)); }
    }

    /// Append S, the characters from START of a string whose own faces
    /// are OWN (`ml_own_faces'), each run in its own face.
    fn push_own(&mut self, s: &str, own: &[(usize, Option<String>)], start: usize) {
        let mut run = own.partition_point(|(at, _)| *at <= start).saturating_sub(1);
        let mut from = 0;
        for (k, (at, _)) in s.char_indices().enumerate() {
            // Character K of S is the string's character START + K.
            if own.get(run + 1).is_some_and(|(next, _)| start + k >= *next) {
                if at > from { self.push(&s[from..at], own[run].1.as_ref()); }
                from = at;
                while own.get(run + 1).is_some_and(|(next, _)| start + k >= *next) { run += 1; }
            }
        }
        if from < s.len() { self.push(&s[from..], own[run].1.as_ref()); }
    }

    fn push_clipped(&mut self, s: &str) {
        for c in s.chars() {
            let (shown, w) = match c {
                '\u{0}'..='\u{1f}' | '\u{7f}' => ([Some('^'), Some(char::from((c as u8) ^ 0x40))], 2),
                _ => ([Some(c), None], unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)),
            };
            if self.cols + w > self.max_cols {
                self.cols = self.max_cols;
                break;
            }
            self.cols += w;
            self.text.extend(shown.into_iter().flatten());
        }
    }
}

/// The face of character K of a string whose own faces are OWN.
fn own_face(own: &[(usize, Option<String>)], k: usize) -> Option<&String> {
    own[..own.partition_point(|(at, _)| *at <= k)].last().and_then(|(_, face)| face.as_ref())
}

impl Vm {
    /// GNU `SANE_TAB_WIDTH (current_buffer)`: `tab-width` if in 1..=1000, else 8.
    fn ml_tab_width(&mut self) -> Result<usize> {
        Ok(match self.ml_symbol_value("tab-width")? {
            Some(Form::Int(n)) if (1..=1000).contains(&n) => n as usize,
            _ => 8,
        })
    }

    fn ml_form_symbol_value(&mut self, symbol: &Form) -> Result<Option<Form>> {
        let slot = match symbol {
            Form::Sym(name) => return self.ml_symbol_value(name),
            Form::Generated(id) => self.generated_symbol(*id).index(7)? - 1,
            _ => return Ok(None),
        };
        let value = self.variable_value(slot)?;
        Ok((value != Value::UNBOUND).then(|| self.mode_line_form(value)))
    }

    fn ml_symbol_value(&mut self, name: &str) -> Result<Option<Form>> {
        // Constants that never live in an ordinary value cell.
        if name == "nil" || name == "t" || name.starts_with(':') {
            return Ok(Some(Form::Sym(name.into())));
        }
        let slot = self.slot(name);
        let value = self.variable_value(slot)?;
        if value == Value::UNBOUND {
            return Ok(None);
        }
        Ok(Some(self.mode_line_form(value)))
    }

    /// Preserve executable object constants in collector-traced handles.
    pub(super) fn mode_line_form(&self, value: Value) -> Form {
        self.expansion_form(value, 0).unwrap_or(Form::Vector(vec![]))
    }

    /// A string element's text. A string with text properties reaches the
    /// expander as an object (`expansion_form`), as GNU's `propertize`d
    /// mode-line strings do; it expands as its text, and its own faces come
    /// from `ml_own_faces' (other properties are not projected, VM2-G1B).
    fn ml_string(&self, elt: &Form) -> Result<Option<String>> {
        Ok(match elt {
            Form::String(s) => Some(s.clone()),
            Form::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
            Form::Object(object) => {
                let value = object.value(self)?;
                match self.object_if(value) {
                    Some(data::Object::String(_)) => Some(String::from_utf8_lossy(&self.string_bytes(value)?).into_owned()),
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// A propertized string element's own faces where the element's PROPS
    /// give none: GNU merges PROPS onto a string's own properties unless it
    /// is risky (display_mode_element), so each character shows its `face'
    /// (a symbol, or a face list's first one). Runs of one face, each from
    /// its first character; None when no character has one.
    fn ml_own_faces(&mut self, elt: &Form, props: Option<&String>, risky: bool) -> Result<Option<Vec<(usize, Option<String>)>>> {
        if props.is_some() || risky { return Ok(None); }
        let Form::Object(object) = elt else { return Ok(None) };
        let value = object.value(self)?;
        if !matches!(self.object_if(value), Some(data::Object::String(_))) || self.string_bytes(value)?.is_empty() { return Ok(None); }
        let face = self.intern("face");
        let mut runs: Vec<(usize, Option<String>)> = vec![];
        for start in self.property_run_starts(value).map_err(crate::forwarded::fault_text)? {
            let v = self.property_textget(value, start, face).map_err(crate::forwarded::fault_text)?;
            let name = self.ml_face_name(v);
            if runs.last().is_none_or(|(_, last)| *last != name) { runs.push((start as usize, name)); }
        }
        Ok(runs.iter().any(|(_, face)| face.is_some()).then_some(runs))
    }

    /// A `face' property's name: a symbol, or a face list's first symbol.
    fn ml_face_name(&self, value: Value) -> Option<String> {
        let name = |v: Value| (v.tag() == 7 && v != Value::NIL).then(|| v.index(7).ok())
            .flatten().and_then(|i| self.symbol_names.get(i.saturating_sub(1)).cloned())
            .filter(|n| !n.starts_with(':'));
        let mut tail = value;
        if value.tag() != 3 { return name(value); }
        while let Ok((head, rest)) = self.cons_pair(tail) {
            if let Some(found) = name(head) { return Some(found); }
            tail = rest;
        }
        None
    }

    /// GNU's C `Fget (elt, Qrisky_local_variable)`, read in place: compiling
    /// a `get' request for every symbol of every mode line made each redisplay
    /// retire enough request code to force a collection every few keys.
    fn ml_risky_marked(&mut self, symbol: &Form) -> bool {
        let symbol = match symbol {
            Form::Sym(name) => self.intern(name),
            Form::Generated(id) => self.generated_symbol(*id),
            _ => return false,
        };
        let key = self.intern("risky-local-variable");
        self.symbol_property(symbol, key).is_ok_and(|v| v != Value::NIL)
    }

    /// The element count a `format-mode-line' inside an `:eval' left, once
    /// it restarted GNU's global count.
    fn ml_elt_restarted(&mut self) -> Option<i64> {
        let slot = self.slot(ELT_RESTARTED);
        let n = self.slots[slot].value.integer_value();
        self.slots[slot].value = Value::UNBOUND;
        n
    }

    /// The treatment of the list FORM, a cons, by its car.
    fn ml_head(&self, form: &Form) -> Result<Head> {
        let (Form::List(items) | Form::Dotted(items, _)) = form else { return Ok(Head::Other) };
        let car = &items[0];
        let string_car = self.ml_string(car)?.is_some();
        Ok(match car {
            Form::Sym(s) if s == ":eval" => Head::Eval,
            Form::Sym(s) if s == ":propertize" => Head::Propertize,
            Form::Sym(_) | Form::Generated(_) => Head::Symbol,
            Form::Int(lim) => Head::Limit(*lim),
            Form::String(_) | Form::Bytes(_) | Form::List(_) | Form::Dotted(..) | Form::Object(_)
                if string_car || !matches!(car, Form::Object(_)) => Head::Each,
            _ => Head::Other,
        })
    }

    /// Expand a whole mode-line format for the window whose buffer is current.
    pub(super) fn mode_line_band(&mut self, format: &Form, state: &SpecState) -> Result<Band> {
        self.mode_line_expand(format, state, Target::Display)
    }

    pub(super) fn mode_line_expand(&mut self, format: &Form, state: &SpecState, target: Target) -> Result<Band> {
        let max_cols = if target == Target::Display { 4 * state.frame_cols } else { usize::MAX };
        let mut band = Band { text: String::new(), runs: vec![], target, elt: 0, elts: vec![], cols: 0, max_cols };
        self.mode_line_element(&mut band, state, 0, 0, 0, format.clone(), None, false)?;
        Ok(band)
    }

    #[allow(clippy::too_many_arguments)] // GNU's display_mode_element signature
    fn mode_line_element(
        &mut self,
        band: &mut Band,
        state: &SpecState,
        mut depth: usize,
        mut field_width: i64,
        mut precision: i64,
        mut elt: Form,
        mut props: Option<String>,
        mut risky: bool,
    ) -> Result<i64> {
        if band.text.len() > 1 << 16 {
            return Err("mode-line expansion limit".into());
        }
        let mut n = 0i64;
        let mut literal = false;
        loop {
            // `tail_recurse' too counts as entering an element.
            if band.target == Target::String { band.elt += 1; }
            if depth > 100 {
                elt = Form::String("*too-deep*".into());
            }
            depth += 1;
            if let Some(s) = self.ml_string(&elt)? {
                let tab = self.ml_tab_width()?;
                // GNU replaces a risky string's properties with PROPS and
                // otherwise merges PROPS onto the string's own: where PROPS
                // has no face, the string's own faces show.
                let own = self.ml_own_faces(&elt, props.as_ref(), risky)?;
                if literal {
                    // GNU `store_mode_line_string`: precision and N count characters.
                    let shown = if precision - n > 0 { char_prefix(&s, (precision - n) as usize) } else { &s };
                    n += chars(shown);
                    match &own { Some(own) => band.push_own(shown, own, 0), None => band.push(shown, props.as_ref()) }
                    break;
                }
                let mut rest = s.split('\0').next().unwrap_or_default();
                while (precision <= 0 || n < precision) && !rest.is_empty() {
                    if !rest.starts_with('%') {
                        let percent = rest.find('%').unwrap_or(rest.len());
                        let segment = &rest[..percent];
                        // `c_string_width` selects by columns; the selected
                        // substring then counts its characters (`xdisp.c` MODE_LINE_STRING).
                        let shown = if precision > 0 { prefix(segment, (precision - n) as usize, tab) } else { segment };
                        n += chars(shown);
                        match &own {
                            // The segment's first character's index in S.
                            Some(own) => band.push_own(shown, own, chars(&s[..rest.as_ptr() as usize - s.as_ptr() as usize]) as usize),
                            None => band.push(shown, props.as_ref()),
                        }
                        rest = &rest[percent..];
                        continue;
                    }
                    // rest starts with '%'. Its expansion takes the string's
                    // own face there (xdisp.c display_mode_element takes the
                    // text properties at the construct's offset).
                    let face = match &own {
                        Some(own) => own_face(own, chars(&s[..rest.as_ptr() as usize - s.as_ptr() as usize]) as usize),
                        None => props.as_ref(),
                    };
                    rest = &rest[1..];
                    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                    // GNU accumulates the width in a C `int` (`xdisp.c:28663`);
                    // 32.0.50 wraps it modulo 2^32 (`gnu/field-wrap.txt`).
                    let mut field = i64::from(rest[..digits].bytes()
                        .fold(0i32, |f, d| f.wrapping_mul(10).wrapping_add(i32::from(d - b'0'))))
                        .min(MAX_FIELD);
                    rest = &rest[digits..];
                    if field_width - n > 0 && field > field_width - n {
                        field = field_width - n;
                    }
                    let prec = precision - n;
                    let Some(c) = rest.chars().next() else { break };
                    rest = &rest[c.len_utf8()..];
                    if c == 'M' {
                        let value = self.ml_symbol_value("global-mode-string")?.unwrap_or(Form::Sym("nil".into()));
                        n += self.mode_line_element(band, state, depth, field, prec, value, props.clone(), risky)?;
                        continue;
                    }
                    let spec = self.decode_mode_spec(state, c, field, band.target)?;
                    let shown = if prec > 0 { char_prefix(&spec, prec as usize) } else { spec.as_str() };
                    let mut written = chars(shown);
                    band.push(shown, face);
                    if field > written {
                        band.pad(field - written, face);
                        written = field;
                    }
                    n += written;
                }
                break;
            }
            // ELT is taken, not copied: each arm moves its parts on.
            match std::mem::replace(&mut elt, Form::Sym(String::new())) {
                symbol @ (Form::Sym(_) | Form::Generated(_)) => {
                    if !risky && !self.ml_risky_marked(&symbol) {
                        risky = true;
                    }
                    let Some(value) = self.ml_form_symbol_value(&symbol)? else { break };
                    if self.ml_string(&value)?.is_some() {
                        literal = true;
                    }
                    if value.name().ok().is_some_and(|name| symbol.name().ok() == Some(name)) {
                        break;
                    }
                    elt = value;
                }
                form @ (Form::List(_) | Form::Dotted(..)) if is_cons(&form) => {
                    let head = self.ml_head(&form)?;
                    let (car, cdr) = split(form).expect("cons");
                    match head {
                        Head::Eval => {
                            if risky {
                                break;
                            }
                            if let Some((expression, _)) = split(cdr) {
                                // GNU `dsafe_eval`: an error displays as nil.
                                self.ml_elt_restarted();
                                let spec = match self.eval_datum_raw(&expression) {
                                    Ok(value) => self.mode_line_form(value),
                                    Err(error) => {
                                        self.display_state.diagnostics.get_mut().push(format!(":eval: {error}"));
                                        Form::Sym("nil".into())
                                    }
                                };
                                if band.target == Target::String && let Some(n) = self.ml_elt_restarted() { band.elt = n; }
                                n += self.mode_line_element(band, state, depth, field_width - n, precision - n, spec, props.clone(), risky)?;
                            }
                            break;
                        }
                        Head::Propertize => {
                            if risky {
                                break;
                            }
                            if let Some((inner, plist)) = split(cdr) {
                                n += self.mode_line_element(band, state, depth, field_width - n, precision - n, inner, face_of(&plist), risky)?;
                            }
                            break;
                        }
                        Head::Symbol => {
                            let truthy = self.ml_form_symbol_value(&car)?.is_some_and(|v| !is_nil(&v));
                            let Some((then, rest)) = split(cdr) else {
                                elt = Form::String("*invalid*".into());
                                continue;
                            };
                            if truthy {
                                elt = then;
                                continue;
                            }
                            if is_nil(&rest) {
                                break;
                            }
                            let Some((otherwise, _)) = split(rest) else {
                                elt = Form::String("*invalid*".into());
                                continue;
                            };
                            elt = otherwise;
                        }
                        Head::Limit(lim) => {
                            // `int lim = XFIXNUM (car)` (`xdisp.c:28879`): the low 32 bits.
                            let lim = i64::from(lim as i32).clamp(-MAX_FIELD, MAX_FIELD);
                            elt = cdr;
                            if lim < 0 {
                                precision = if precision <= 0 { -lim } else { precision.min(-lim) };
                            } else if lim > 0 {
                                let lim = if precision > 0 { precision.min(lim) } else { lim };
                                field_width = field_width.max(lim);
                            }
                        }
                        Head::Each => {
                            // The elements of the list (a dotted tail is not one).
                            let mut items = vec![car];
                            match cdr {
                                Form::List(rest) | Form::Dotted(rest, _) => items.extend(rest),
                                _ => {}
                            }
                            let count = items.len();
                            for (index, item) in items.into_iter().enumerate() {
                                if 0 < precision && precision <= n {
                                    break;
                                }
                                let last = index + 1 == count;
                                n += self.mode_line_element(band, state, depth, if last { field_width - n } else { 0 }, precision - n, item, props.clone(), risky)?;
                            }
                            break;
                        }
                        Head::Other => break,
                    }
                }
                _ => {
                    elt = Form::String("*invalid*".into());
                }
            }
        }
        if field_width > 0 && n < field_width {
            band.pad_element(field_width - n);
            n = field_width;
        }
        Ok(n)
    }

    /// GNU `decode_mode_spec` for the current (window's) buffer.
    fn decode_mode_spec(&mut self, state: &SpecState, c: char, field: i64, target: Target) -> Result<String> {
        let width = field.min(4 * state.frame_cols as i64);
        let read_only = self.ml_symbol_value("buffer-read-only")?.is_some_and(|v| !is_nil(&v));
        let string_of = |value: Option<Form>| match value {
            Some(Form::String(s)) => s,
            // Unibyte characters are bytes, not UTF-8 sequences. Preserve
            // ASCII and paint high bytes as GNU's default octal glyphs.
            Some(Form::Bytes(bytes)) => bytes.into_iter().map(|b| {
                if b.is_ascii() { char::from(b).to_string() } else { format!("\\{b:03o}") }
            }).collect(),
            _ => String::new(),
        };
        let (top, bottom, begv, zv) = (state.start, state.end, state.begv, state.zv);
        Ok(match c {
            '*' if read_only => "%".into(),
            '*' | '&' => if state.modified { "*" } else { "-" }.into(),
            '+' => if state.modified { "*" } else if read_only { "%" } else { "-" }.into(),
            '%' => "%".into(),
            // command_loop_level: VM2 has no recursive edit.
            '[' | ']' => String::new(),
            // A string's dashes are "infinitely long" as two.
            '-' if target != Target::Display => "--".into(),
            '-' => "-".repeat(if field <= 0 || field as usize > LOTS_OF_DASHES { 4 * state.frame_cols - 1 } else { LOTS_OF_DASHES }),
            'b' => state.buffer_name.clone(),
            'c' => pint2str(width, state.column as i64),
            'C' => pint2str(width, state.column as i64 + 1),
            'e' => String::new(),
            // No frame title or explicit name on a window frame.
            'F' => "Emacs".into(),
            'f' => string_of(self.ml_symbol_value("buffer-file-name")?),
            'i' => pint2str(width, (zv - begv) as i64),
            'I' => pint2hrstr(width, (zv - begv) as i64),
            'l' => pint2str(width, state.line as i64),
            'm' => string_of(self.ml_symbol_value("mode-name")?),
            'n' => if begv > 1 || zv < state.z { " Narrow" } else { "" }.into(),
            'o' => if zv <= bottom {
                if top <= begv { "All" } else { "Bottom" }.into()
            } else if top <= begv {
                "Top".into()
            } else {
                format!("{:2}%", percent99(top - begv, (top - begv) + (zv - bottom)))
            },
            'p' => if bottom >= zv {
                if top <= begv { "All" } else { "Bottom" }.into()
            } else if top <= begv {
                "Top".into()
            } else {
                format!("{:2}%", percent99(top - begv, zv - begv))
            },
            'P' => if zv <= bottom {
                if top <= begv { "All" } else { "Bottom" }.into()
            } else {
                let percent = percent99(bottom - begv, zv - begv);
                if begv < top { format!("{percent:2}%") } else { format!("Top{percent:2}%") }
            },
            'q' => {
                if top <= begv && zv <= bottom {
                    "All   ".into()
                } else {
                    let top = if top <= begv { 0 } else { percent99(top - begv, zv - begv) };
                    let bottom = if zv <= bottom { 100 } else { percent99(bottom - begv, zv - begv) };
                    if top == bottom { format!("{top}%") } else { format!("{top}-{bottom}%") }
                }
            }
            // No processes in VM2.
            's' => "no process".into(),
            '@' => {
                let form = crate::Form::List(vec![
                    Form::Sym("and".into()),
                    Form::List(vec![Form::Sym("stringp".into()), Form::Sym("default-directory".into())]),
                    Form::List(vec![Form::Sym("fboundp".into()), Form::List(vec![Form::Sym("quote".into()), Form::Sym("file-remote-p".into())])]),
                    Form::List(vec![Form::Sym("file-remote-p".into()), Form::Sym("default-directory".into())]),
                ]);
                if self.eval_datum_raw(&form).is_ok_and(|v| v != Value::NIL) { "@" } else { "-" }.into()
            }
            // A window-system frame shows the buffer's coding system
            // (xdisp.c:29840). The initial terminal keeps GNU's "not yet
            // decided" mnemonics (`decode_mode_spec_coding`).
            'z' | 'Z' if self.frame_type() != Value::TRUE => {
                let slot = self.slot("enable-multibyte-characters");
                let multibyte = !matches!(self.variable_value(slot)?, Value::NIL | Value::UNBOUND);
                let slot = self.slot("buffer-file-coding-system");
                let coding = self.variable_value(slot)?;
                let coding = if coding == Value::UNBOUND { Value::NIL } else { coding };
                self.mode_spec_coding(coding, multibyte, c == 'Z')?
            }
            'z' => "-".into(),
            'Z' => {
                let mnemonic = self.ml_symbol_value("eol-mnemonic-undecided")?;
                format!("-{}", if matches!(mnemonic, Some(Form::String(_) | Form::Bytes(_))) {
                    string_of(mnemonic)
                } else {
                    ":".into()
                })
            },
            _ => String::new(),
        })
    }
}
