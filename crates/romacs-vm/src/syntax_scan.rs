//! GNU syntax.c's scanning engine: `forw_comment`, `back_comment`,
//! `Fforward_comment`, `scan_lists` and `scan_sexps_forward`, transcribed
//! over a character view of the accessible buffer and a per-position
//! syntax lookup. Nothing here touches the VM, so a scan cannot collect.
//!
//! GNU calls Lisp from the middle of a scan in two places: just-in-time
//! `internal--syntax-propertize` when `parse-sexp-lookup-properties` is set
//! (`parse_sexp_propertize`), and `syntax-ppss` from `find_defun_start` when
//! `comment-use-syntax-ppss` is set. Here such a point stops the scan with a
//! [`Halt`] naming the call; the primitive performs it and runs the whole
//! (deterministic) scan again with the answer, so the calls happen in GNU's
//! order and with GNU's arguments.
//!
//! The syntax of a position is always looked up at that exact position,
//! which is what gl_state's interval bookkeeping arranges in GNU.
use std::collections::BTreeMap;

pub(crate) const SWHITESPACE: i32 = 0;
pub(crate) const SWORD: i32 = 2;
pub(crate) const SSYMBOL: i32 = 3;
pub(crate) const SOPEN: i32 = 4;
pub(crate) const SCLOSE: i32 = 5;
pub(crate) const SQUOTE: i32 = 6;
pub(crate) const SSTRING: i32 = 7;
pub(crate) const SMATH: i32 = 8;
pub(crate) const SESCAPE: i32 = 9;
pub(crate) const SCHARQUOTE: i32 = 10;
pub(crate) const SCOMMENT: i32 = 11;
pub(crate) const SENDCOMMENT: i32 = 12;
pub(crate) const SCOMMENT_FENCE: i32 = 14;
pub(crate) const SSTRING_FENCE: i32 = 15;
/// `Smax`: no syntax, or a syntax whose two-character power is used up.
pub(crate) const SMAX: i32 = 16;
/// Comment style and string terminator of fence-delimited constructs.
pub(crate) const ST_COMMENT_STYLE: i32 = 256 + 1;
pub(crate) const ST_STRING_STYLE: i32 = 256 + 2;

/// syntax.c syntax_code_spec: the designator character of each class.
pub(crate) const SYNTAX_CODE_SPEC: [u8; 16] = *b" .w_()'\"$\\/<>@!|";

fn comstart_first(s: i32) -> bool { (s >> 16) & 1 != 0 }
fn comstart_second(s: i32) -> bool { (s >> 17) & 1 != 0 }
fn comend_first(s: i32) -> bool { (s >> 18) & 1 != 0 }
fn comend_second(s: i32) -> bool { (s >> 19) & 1 != 0 }
fn comstartend_first(s: i32) -> bool { s & 0x50000 != 0 }
pub(crate) fn prefix(s: i32) -> bool { (s >> 20) & 1 != 0 }
fn nested(s: i32) -> bool { (s >> 22) & 1 != 0 }
/// SYNTAX_FLAGS_COMMENT_STYLE: FLAGS of the main delimiter character.
fn comment_style(flags: i32, other: i32) -> i32 {
    ((flags >> 21) & 1) | ((flags >> 22) & 2) | ((other >> 22) & 2)
}

/// A character position (1-based, like GNU's charpos) and its byte offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pos {
    pub char: i64,
    pub byte: usize,
}

/// The accessible text of a buffer.
pub(crate) trait Text {
    /// FETCH_CHAR_AS_MULTIBYTE; 0 outside the text.
    fn char_at(&self, at: Pos) -> u32;
    /// inc_both and dec_both.
    fn inc(&self, at: Pos) -> Pos;
    fn dec(&self, at: Pos) -> Pos;
    /// CHAR_TO_BYTE, clipped to the buffer.
    fn pos(&self, charpos: i64) -> Pos;
    /// SYNTAX_WITH_FLAGS of the character at AT, honoring its
    /// `syntax-table' property when `parse-sexp-lookup-properties' is set.
    fn syntax(&self, at: Pos) -> i32;
    /// SYNTAX_WITH_FLAGS of character C in the buffer's own syntax table.
    fn table_syntax(&self, c: u32) -> i32;
}

/// The variables a scan reads.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Options {
    pub ignore_comments: bool,
    pub comment_end_can_be_escaped: bool,
    pub multibyte_syntax_as_symbol: bool,
    pub open_paren_in_column_0_is_defun_start: bool,
    pub comment_use_syntax_ppss: bool,
    /// `syntax-propertize--done' when `parse-sexp-lookup-properties' is set.
    pub propertize_done: Option<i64>,
}

/// GNU's find_start_* cache, valid for the buffer being scanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DefunStart {
    pub pos: i64,
    pub value: i64,
    pub begv: i64,
    pub modiff: u64,
}

/// Why a scan stopped short of a result.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Halt {
    /// `(scan-error MESSAGE POS1 POS2)`.
    ScanError(&'static str, i64, i64),
    /// Call `internal--syntax-propertize' with the first position (the
    /// argument of parse_sexp_propertize is the second).
    Propertize(i64, i64),
    /// Call `syntax-ppss' at this position.
    Ppss(i64),
}
pub(crate) type R<T> = Result<T, Halt>;

/// struct lisp_parse_state, as `parse-partial-sexp' reads and returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseState {
    pub depth: i64,
    pub instring: i32,
    pub incomment: i64,
    pub comstyle: i32,
    pub quoted: bool,
    pub mindepth: i64,
    pub thislevelstart: i64,
    pub prevlevelstart: i64,
    pub location: i64,
    pub comstr_start: i64,
    /// Fixnum elements of OLDSTATE's open-paren list as Some; on return,
    /// every element is Some.
    pub levelstarts: Vec<Option<i64>>,
    pub prev_syntax: i32,
}
impl Default for ParseState {
    /// internalize_parse_state (Qnil, ...).
    fn default() -> Self {
        Self {
            depth: 0, instring: -1, incomment: 0, comstyle: 0, quoted: false, mindepth: 0,
            thislevelstart: -1, prevlevelstart: -1, location: 0, comstr_start: -1,
            levelstarts: Vec::new(), prev_syntax: SMAX,
        }
    }
}

pub(crate) struct Scanner<'a, T: Text> {
    pub text: &'a T,
    pub begv: Pos,
    pub zv: Pos,
    pub options: Options,
    /// Defun starts of the `syntax-ppss' calls already answered.
    pub ppss: &'a BTreeMap<i64, i64>,
    pub defun_start: Option<DefunStart>,
    pub modiff: u64,
}

/// Labels of scan_sexps_forward.
#[derive(Clone, Copy)]
enum At { Top, AtComment, InComment, Quoted, Symbol, InString, QuotedInString, Stop, EndQuoted, Done }

impl<T: Text> Scanner<'_, T> {
    fn update_forward(&self, charpos: i64) -> R<()> {
        // UPDATE_SYNTAX_TABLE_FORWARD's just-in-time parse_sexp_propertize.
        match self.options.propertize_done {
            Some(done) if done <= charpos && done < self.zv.char => {
                Err(Halt::Propertize(self.zv.char.min(charpos + 1), charpos))
            }
            _ => Ok(()),
        }
    }
    /// SETUP_SYNTAX_TABLE (FROM, COUNT).
    fn setup(&self, from: i64, count: i64) -> R<()> {
        if count > 0 {
            self.update_forward(from)
        } else if from > self.begv.char {
            self.update_forward(from - 1)
        } else {
            Ok(())
        }
    }
    fn class(&self, at: Pos, multibyte_symbol: bool) -> i32 {
        // syntax_multibyte
        if self.text.char_at(at) < 128 || !multibyte_symbol { self.text.syntax(at) & 0xff } else { SSYMBOL }
    }
    fn char_quoted(&self, at: Pos) -> R<bool> {
        let mut pos = at;
        let mut quoted = false;
        while pos.char > self.begv.char {
            pos = self.text.dec(pos);
            let code = self.text.syntax(pos) & 0xff;
            if !(code == SCHARQUOTE || code == SESCAPE) { break; }
            quoted = !quoted;
        }
        self.update_forward(at.char)?;
        Ok(quoted)
    }
    fn prev_char_comend_first(&self, at: Pos) -> R<bool> {
        let pos = self.text.dec(at);
        let value = comend_first(self.text.syntax(pos));
        self.update_forward(pos.char + 1)?;
        Ok(value)
    }

    /// forw_comment: (found, position, incomment, last syntax).
    fn forw_comment(&self, mut from: Pos, stop: i64, mut nesting: i64, style: i32, mut prev_syntax: i32)
        -> R<(bool, Pos, i64, i32)> {
        if nesting <= 0 { nesting = -1; }
        let mut syntax = prev_syntax;
        let mut code = syntax & 0xff;
        let mut incomment = syntax != 0 && from.char < stop;
        loop {
            if !incomment {
                if from.char == stop {
                    let last = if code == SESCAPE || code == SCHARQUOTE || comend_first(syntax)
                        || (nesting > 0 && comstart_first(syntax)) { syntax } else { SMAX };
                    return Ok((false, from, nesting, last));
                }
                prev_syntax = syntax;
                syntax = self.text.syntax(from);
                code = syntax & 0xff;
                if code == SENDCOMMENT && comment_style(syntax, 0) == style
                    && (if nested(syntax) { nesting > 0 && { nesting -= 1; nesting == 0 } } else { nesting < 0 })
                    && !(self.options.comment_end_can_be_escaped
                         && ((prev_syntax & 0xff) == SESCAPE || (prev_syntax & 0xff) == SCHARQUOTE)) {
                    break;
                }
                if code == SCOMMENT_FENCE && style == ST_COMMENT_STYLE { break; }
                if nesting > 0 && code == SCOMMENT && nested(syntax) && comment_style(syntax, 0) == style {
                    nesting += 1;
                }
                if self.options.comment_end_can_be_escaped && (code == SESCAPE || code == SCHARQUOTE) {
                    from = self.text.inc(from);
                    self.update_forward(from.char)?;
                    if from.char == stop { continue; }
                    syntax = SMAX;
                    code = syntax;
                }
                from = self.text.inc(from);
                self.update_forward(from.char)?;
            }
            incomment = false;
            // forw_incomment:
            if from.char < stop && comend_first(syntax) {
                let other = self.text.syntax(from);
                if comend_second(other) && comment_style(syntax, other) == style
                    && (if nested(syntax) || nested(other) { nesting > 0 } else { nesting < 0 }) {
                    syntax = SMAX;
                    nesting -= 1;
                    if nesting <= 0 { break; }
                    from = self.text.inc(from);
                    self.update_forward(from.char)?;
                }
            }
            if nesting > 0 && from.char < stop && comstart_first(syntax) {
                let other = self.text.syntax(from);
                if comment_style(other, syntax) == style && comstart_second(other)
                    && (nested(syntax) || nested(other)) {
                    syntax = SMAX;
                    from = self.text.inc(from);
                    self.update_forward(from.char)?;
                    nesting += 1;
                }
            }
        }
        Ok((true, from, nesting, SMAX))
    }

    /// back_comment: whether a comment ends at FROM, and where it starts.
    fn back_comment(&mut self, from: Pos, stop: i64, comnested: bool, comstyle: i32) -> R<(bool, Pos)> {
        let comment_end = from;
        let mut from = from;
        let mut string_style = -1;
        let mut string_lossage = false;
        let mut comment_lossage = false;
        let mut comstart_pos: Option<Pos> = None;
        let mut defun_start: Option<Pos> = None;
        let mut nesting = 1;
        let mut syntax = 0;
        let mut lossage = false;
        while from.char != stop {
            from = self.text.dec(from);
            let prev_syntax = syntax;
            let mut c = self.text.char_at(from) as i32;
            syntax = self.text.syntax(from);
            let mut code = syntax & 0xff;
            let com2start = comstart_first(syntax) && comstart_second(prev_syntax)
                && comstyle == comment_style(prev_syntax, syntax)
                && (nested(prev_syntax) || nested(syntax)) == comnested;
            let mut com2end = comend_first(syntax) && comend_second(prev_syntax);
            let comstart = com2start || code == SCOMMENT;
            if from.char > stop && (com2end || comstart) {
                let next = self.text.dec(from);
                let next_syntax = self.text.syntax(next);
                if ((comstart || comnested) && comend_second(syntax) && comend_first(next_syntax))
                    || ((com2end || comnested) && comstart_second(syntax)
                        && comstyle == comment_style(syntax, prev_syntax) && comstart_first(next_syntax)) {
                    lossage = true;
                    break;
                }
            }
            if com2start && comstart_pos.is_none() { com2end = false; }
            if com2end {
                code = SENDCOMMENT;
            } else if com2start {
                code = SCOMMENT;
            } else if code == SCOMMENT && (comstyle != comment_style(syntax, 0) || nested(syntax) != comnested) {
                continue;
            }
            if (self.options.comment_end_can_be_escaped || code != SENDCOMMENT) && self.char_quoted(from)? {
                continue;
            }
            match code {
                SSTRING_FENCE | SCOMMENT_FENCE | SSTRING => {
                    if code == SSTRING_FENCE { c = ST_STRING_STYLE; }
                    if code == SCOMMENT_FENCE { c = ST_COMMENT_STYLE; }
                    if string_style == -1 {
                        string_style = c;
                    } else if string_style == c {
                        string_style = -1;
                    } else {
                        string_lossage = true;
                    }
                }
                SCOMMENT => {
                    if string_style != -1 || comment_lossage || string_lossage {
                        lossage = true;
                        break;
                    }
                    if !comnested {
                        comstart_pos = Some(from);
                    } else {
                        nesting -= 1;
                        if nesting <= 0 { return Ok((from.char != comment_end.char, from)); }
                    }
                }
                SENDCOMMENT => {
                    if comment_style(syntax, 0) == comstyle
                        && ((com2end && nested(prev_syntax)) || nested(syntax)) == comnested {
                        if comnested { nesting += 1; } else { break; }
                    } else if comstart_pos.is_some() || c != '\n' as i32 {
                        comment_lossage = true;
                    }
                }
                SOPEN => {
                    if self.options.open_paren_in_column_0_is_defun_start && !self.options.comment_use_syntax_ppss
                        && (from.char == stop || self.text.char_at(self.text.dec(from)) == '\n' as u32) {
                        defun_start = Some(from);
                        break;
                    }
                }
                _ => {}
            }
        }
        if !lossage {
            from = comstart_pos.unwrap_or(comment_end);
            self.update_forward(if comstart_pos.is_some() { from.char - 1 } else { from.char })?;
            return Ok((from.char != comment_end.char, from));
        }
        // lossage: decode forward from a safe place.
        let mut adjusted = true;
        let mut start = match defun_start {
            Some(start) => start,
            None => {
                let start = self.find_defun_start(comment_end)?;
                adjusted = start.char > self.begv.char;
                start
            }
        };
        loop {
            let mut state = ParseState::default();
            self.scan_sexps_forward(&mut state, start, comment_end.char, i64::MIN, false, 0)?;
            let mut next = comment_end.char;
            if !adjusted {
                adjusted = true;
                if let Some(cache) = &mut self.defun_start {
                    cache.value = match state.levelstarts.first() {
                        Some(first) => first.unwrap_or(-1),
                        None if state.thislevelstart >= 0 => state.thislevelstart,
                        None => cache.value,
                    };
                }
            }
            if state.incomment == if comnested { 1 } else { -1 } && state.comstyle == comstyle {
                from = self.text.pos(state.comstr_start);
            } else {
                from = comment_end;
                if state.incomment != 0 { next = state.comstr_start + 2; }
            }
            if next >= comment_end.char { break; }
            start = self.text.pos(next);
        }
        self.update_forward(from.char - 1)?;
        Ok((from.char != comment_end.char, from))
    }

    /// find_defun_start.
    fn find_defun_start(&mut self, pos: Pos) -> R<Pos> {
        if let Some(cache) = self.defun_start {
            if pos.char <= cache.pos + 1000 && pos.char >= cache.value
                && cache.begv == self.begv.char && cache.modiff == self.modiff {
                return Ok(self.text.pos(cache.value));
            }
        }
        let value = if self.options.comment_use_syntax_ppss {
            *self.ppss.get(&pos.char).ok_or(Halt::Ppss(pos.char))?
        } else if !self.options.open_paren_in_column_0_is_defun_start {
            self.begv.char
        } else {
            // Back to the start of the line, then line by line for an
            // open paren in column 0 in both the buffer's and the
            // property syntax.
            let mut point = self.line_start(pos, 1);
            while point.char > self.begv.char {
                if self.text.table_syntax(self.text.char_at(point)) & 0xff == SOPEN {
                    self.setup(point.char + 1, -1)?;
                    if self.text.syntax(point) & 0xff == SOPEN { break; }
                }
                point = self.line_start(point, 2);
            }
            point.char
        };
        self.defun_start = Some(DefunStart { pos: pos.char, value, begv: self.begv.char, modiff: self.modiff });
        Ok(self.text.pos(value))
    }
    /// scan_newline backward by COUNT newlines: after the COUNTth newline
    /// before POS, or BEGV.
    fn line_start(&self, pos: Pos, count: usize) -> Pos {
        let mut at = pos;
        let mut seen = 0;
        while at.char > self.begv.char {
            let before = self.text.dec(at);
            if self.text.char_at(before) == '\n' as u32 {
                seen += 1;
                if seen == count { return at; }
            }
            at = before;
        }
        at
    }

    /// Fforward_comment after its argument checks: final point and value.
    pub(crate) fn forward_comment(&mut self, from: Pos, count: i64) -> R<(Pos, bool)> {
        let stop = if count > 0 { self.zv.char } else { self.begv.char };
        let mut from = from;
        let mut count = count;
        self.setup(from.char, count)?;
        while count > 0 {
            let (mut code, mut comstyle, mut comnested);
            loop {
                if from.char == stop { return Ok((from, false)); }
                let c = self.text.char_at(from);
                let syntax = self.text.syntax(from);
                code = syntax & 0xff;
                let first = comstart_first(syntax);
                comnested = nested(syntax);
                comstyle = comment_style(syntax, 0);
                from = self.text.inc(from);
                self.update_forward(from.char)?;
                if from.char < stop && first {
                    let other = self.text.syntax(from);
                    if comstart_second(other) {
                        code = SCOMMENT;
                        comstyle = comment_style(other, syntax);
                        comnested |= nested(other);
                        from = self.text.inc(from);
                        self.update_forward(from.char)?;
                    }
                }
                if !(code == SWHITESPACE || (code == SENDCOMMENT && c == '\n' as u32)) { break; }
            }
            if code == SCOMMENT_FENCE {
                comstyle = ST_COMMENT_STYLE;
            } else if code != SCOMMENT {
                return Ok((self.text.dec(from), false));
            }
            let (found, out, _, _) = self.forw_comment(from, stop, i64::from(comnested), comstyle, 0)?;
            from = out;
            if !found { return Ok((from, false)); }
            from = self.text.inc(from);
            self.update_forward(from.char)?;
            count -= 1;
        }
        while count < 0 {
            loop {
                if from.char <= stop { return Ok((self.begv, false)); }
                from = self.text.dec(from);
                let quoted = self.char_quoted(from)?;
                let at = from;
                let c = self.text.char_at(from);
                let syntax = self.text.syntax(from);
                let mut code = syntax & 0xff;
                let mut comstyle = 0;
                let mut comnested = nested(syntax);
                if code == SENDCOMMENT { comstyle = comment_style(syntax, 0); }
                if from.char > stop && comend_second(syntax) && self.prev_char_comend_first(from)?
                    && !self.char_quoted(self.text.dec(from))? {
                    from = self.text.dec(from);
                    code = SENDCOMMENT;
                    let other = self.text.syntax(from);
                    comstyle = comment_style(other, syntax);
                    comnested |= nested(other);
                }
                if code == SCOMMENT_FENCE {
                    let ini = from;
                    let mut fence_found = false;
                    if from.char > stop {
                        loop {
                            from = self.text.dec(from);
                            if self.text.syntax(from) & 0xff == SCOMMENT_FENCE && !self.char_quoted(from)? {
                                fence_found = true;
                                break;
                            } else if from.char == stop {
                                break;
                            }
                        }
                    }
                    if !fence_found { return Ok((self.text.inc(ini), false)); }
                    break;
                } else if code == SENDCOMMENT {
                    let found = if !quoted || !self.options.comment_end_can_be_escaped {
                        let (found, start) = self.back_comment(from, stop, comnested, comstyle)?;
                        if found { from = start; }
                        found
                    } else {
                        false
                    };
                    if found { break; }
                    if c != '\n' as u32 {
                        if self.text.syntax(at) & 0xff != code { from = self.text.inc(from); }
                        return Ok((self.text.inc(from), false));
                    }
                } else if code != SWHITESPACE || quoted {
                    return Ok((self.text.inc(from), false));
                }
            }
            count += 1;
        }
        Ok((from, true))
    }

    /// Fbackward_prefix_chars: the new point.
    pub(crate) fn backward_prefix_chars(&mut self, point: Pos) -> R<Pos> {
        if point.char <= self.begv.char { return Ok(point); }
        self.setup(point.char, -1)?;
        let mut opoint = point;
        let mut pos = self.text.dec(point);
        while !self.char_quoted(pos)? && {
            let syntax = self.text.syntax(pos);
            syntax & 0xff == SQUOTE || prefix(syntax)
        } {
            opoint = pos;
            if pos.char <= self.begv.char { break; }
            pos = self.text.dec(pos);
        }
        Ok(opoint)
    }

    /// scan_lists: the position reached, or None between groupings.
    pub(crate) fn scan_lists(&mut self, from0: i64, count: i64, depth: i64, sexpflag: bool) -> R<Option<i64>> {
        let mut count = count;
        let mut depth = depth;
        let stop = if count > 0 { self.zv.char } else { self.begv.char };
        let mut min_depth = depth;
        let mut mathexit = false;
        let mut last_good = from0;
        let symbol = sexpflag && self.options.multibyte_syntax_as_symbol;
        if depth > 0 { min_depth = 0; }
        let mut from = self.text.pos(from0.clamp(self.begv.char, self.zv.char));
        let lose = |last_good, from: Pos| Halt::ScanError("Unbalanced parentheses", last_good, from.char);
        let premature = |last_good, from: Pos| Halt::ScanError("Containing expression ends prematurely", last_good, from.char);
        self.setup(from.char, count)?;
        while count > 0 {
            let done = 'scan: loop {
                if from.char >= stop { break false; }
                self.update_forward(from.char)?;
                let mut c = self.text.char_at(from);
                let syntax = self.text.syntax(from);
                let mut code = self.class(from, symbol);
                let first = comstart_first(syntax);
                let mut comnested = nested(syntax);
                let mut comstyle = comment_style(syntax, 0);
                if depth == min_depth { last_good = from.char; }
                from = self.text.inc(from);
                self.update_forward(from.char)?;
                if from.char < stop && first {
                    c = self.text.char_at(from);
                    let other = self.text.syntax(from);
                    if comstart_second(other) && self.options.ignore_comments {
                        code = SCOMMENT;
                        comstyle = comment_style(other, syntax);
                        comnested |= nested(other);
                        from = self.text.inc(from);
                        self.update_forward(from.char)?;
                    }
                }
                if prefix(syntax) { continue; }
                let mut close = false;
                match code {
                    SESCAPE | SCHARQUOTE | SWORD | SSYMBOL => {
                        if code == SESCAPE || code == SCHARQUOTE {
                            if from.char == stop { return Err(lose(last_good, from)); }
                            from = self.text.inc(from);
                        }
                        if depth != 0 || !sexpflag { continue; }
                        while from.char < stop {
                            self.update_forward(from.char)?;
                            match self.class(from, symbol) {
                                SCHARQUOTE | SESCAPE => {
                                    from = self.text.inc(from);
                                    if from.char == stop { return Err(lose(last_good, from)); }
                                }
                                SWORD | SSYMBOL | SQUOTE => {}
                                _ => break 'scan true,
                            }
                            from = self.text.inc(from);
                        }
                        break 'scan true;
                    }
                    SCOMMENT_FENCE | SCOMMENT => {
                        if code == SCOMMENT_FENCE { comstyle = ST_COMMENT_STYLE; }
                        if !self.options.ignore_comments { continue; }
                        self.update_forward(from.char)?;
                        let (found, out, _, _) = self.forw_comment(from, stop, i64::from(comnested), comstyle, 0)?;
                        from = out;
                        if !found {
                            if depth == 0 { break 'scan true; }
                            return Err(lose(last_good, from));
                        }
                        from = self.text.inc(from);
                        self.update_forward(from.char)?;
                    }
                    SMATH => {
                        if !sexpflag { continue; }
                        if from.char != stop && c == self.text.char_at(from) { from = self.text.inc(from); }
                        if mathexit {
                            mathexit = false;
                            close = true;
                        } else {
                            mathexit = true;
                            depth += 1;
                            if depth == 0 { break 'scan true; }
                        }
                    }
                    SOPEN => {
                        depth += 1;
                        if depth == 0 { break 'scan true; }
                    }
                    SCLOSE => close = true,
                    SSTRING | SSTRING_FENCE => {
                        let stringterm = self.text.char_at(self.text.dec(from));
                        loop {
                            if from.char >= stop { return Err(lose(last_good, from)); }
                            self.update_forward(from.char)?;
                            let c = self.text.char_at(from);
                            let c_code = self.class(from, symbol);
                            if if code == SSTRING { c == stringterm && c_code == SSTRING } else { c_code == SSTRING_FENCE } {
                                break;
                            }
                            if c_code == SCHARQUOTE || c_code == SESCAPE { from = self.text.inc(from); }
                            from = self.text.inc(from);
                        }
                        from = self.text.inc(from);
                        if depth == 0 && sexpflag { break 'scan true; }
                    }
                    _ => {}
                }
                if close {
                    depth -= 1;
                    if depth == 0 { break 'scan true; }
                    if depth < min_depth { return Err(premature(last_good, from)); }
                }
            };
            if !done {
                if depth != 0 { return Err(lose(last_good, from)); }
                return Ok(None);
            }
            count -= 1;
        }
        while count < 0 {
            let done = 'scan: loop {
                if from.char <= stop { break false; }
                from = self.text.dec(from);
                let c = self.text.char_at(from);
                let syntax = self.text.syntax(from);
                let mut code = self.class(from, symbol);
                if depth == min_depth { last_good = from.char; }
                let mut comstyle = 0;
                let mut comnested = nested(syntax);
                if code == SENDCOMMENT { comstyle = comment_style(syntax, 0); }
                if from.char > stop && comend_second(syntax) && self.prev_char_comend_first(from)?
                    && self.options.ignore_comments {
                    from = self.text.dec(from);
                    code = SENDCOMMENT;
                    let other = self.text.syntax(from);
                    comstyle = comment_style(other, syntax);
                    comnested |= nested(other);
                }
                if code != SENDCOMMENT && self.char_quoted(from)? {
                    from = self.text.dec(from);
                    code = SWORD;
                } else if prefix(syntax) {
                    continue;
                }
                let mut open = false;
                match code {
                    SWORD | SSYMBOL | SESCAPE | SCHARQUOTE => {
                        if depth != 0 || !sexpflag { continue; }
                        while from.char > stop {
                            let mut temp = self.text.dec(from);
                            if self.class(temp, symbol) == SENDCOMMENT { break 'scan true; }
                            let quoted = self.char_quoted(temp)?;
                            if quoted {
                                from = self.text.dec(from);
                                temp = self.text.dec(temp);
                            }
                            if !quoted && !matches!(self.class(temp, symbol), SWORD | SSYMBOL | SQUOTE) {
                                break 'scan true;
                            }
                            from = self.text.dec(from);
                        }
                        break 'scan true;
                    }
                    SMATH => {
                        if !sexpflag { continue; }
                        if from.char > self.begv.char {
                            let temp = self.text.dec(from);
                            if from.char != stop && c == self.text.char_at(temp) { from = self.text.dec(from); }
                        }
                        if mathexit {
                            mathexit = false;
                            open = true;
                        } else {
                            mathexit = true;
                            depth += 1;
                            if depth == 0 { break 'scan true; }
                        }
                    }
                    SCLOSE => {
                        depth += 1;
                        if depth == 0 { break 'scan true; }
                    }
                    SOPEN => open = true,
                    SENDCOMMENT => {
                        if !self.options.ignore_comments { continue; }
                        let (found, start) = self.back_comment(from, stop, comnested, comstyle)?;
                        if found { from = start; }
                    }
                    SCOMMENT_FENCE | SSTRING_FENCE => {
                        loop {
                            if from.char == stop { return Err(lose(last_good, from)); }
                            from = self.text.dec(from);
                            if !self.char_quoted(from)? && self.class(from, symbol) == code { break; }
                        }
                        if code == SSTRING_FENCE && depth == 0 && sexpflag { break 'scan true; }
                    }
                    SSTRING => {
                        let stringterm = self.text.char_at(from);
                        loop {
                            if from.char == stop { return Err(lose(last_good, from)); }
                            from = self.text.dec(from);
                            if !self.char_quoted(from)? && self.text.char_at(from) == stringterm
                                && self.class(from, symbol) == SSTRING {
                                break;
                            }
                        }
                        if depth == 0 && sexpflag { break 'scan true; }
                    }
                    _ => {}
                }
                if open {
                    depth -= 1;
                    if depth == 0 { break 'scan true; }
                    if depth < min_depth { return Err(premature(last_good, from)); }
                }
            };
            if !done {
                if depth != 0 { return Err(lose(last_good, from)); }
                return Ok(None);
            }
            count += 1;
        }
        Ok(Some(from.char))
    }

    /// in_2char_comment_start.
    fn two_char_comment_start(&self, state: &mut ParseState, prev_from_syntax: i32, prev_from: Pos, from: Pos) -> bool {
        if !comstart_first(prev_from_syntax) { return false; }
        let syntax = self.text.syntax(from);
        if !comstart_second(syntax) { return false; }
        state.comstyle = comment_style(syntax, prev_from_syntax);
        state.incomment = if nested(prev_from_syntax) || nested(syntax) { 1 } else { -1 };
        state.comstr_start = prev_from.char;
        true
    }

    /// scan_sexps_forward: parse from FROM to END in STATE. COMMENTSTOP 1
    /// stops at a comment start, -1 also at comment ends and string
    /// boundaries.
    #[allow(unused_assignments)] // INC_FROM's fields are not all read on every path.
    pub(crate) fn scan_sexps_forward(&mut self, state: &mut ParseState, from: Pos, end: i64,
                                     targetdepth: i64, stopbefore: bool, commentstop: i32) -> R<()> {
        #[derive(Clone, Copy)]
        struct Level { last: i64, prev: i64 }
        const LEVELS: usize = 100;
        let boundary_stop = commentstop == -1;
        let mut from = from;
        let mut prev_from = from;
        if from.char != self.begv.char { prev_from = self.text.dec(prev_from); }
        let mut depth = state.depth;
        let start_quoted = state.quoted;
        let mut prev_prev_from_syntax = SMAX;
        let mut prev_from_syntax = state.prev_syntax;
        let mut levels = vec![Level { last: -1, prev: -1 }];
        for start in &state.levelstarts {
            let level = levels.len() - 1;
            if let Some(start) = *start { levels[level].last = start; }
            if levels.len() < LEVELS { levels.push(Level { last: -1, prev: -1 }); }
            let level = levels.len() - 1;
            levels[level] = Level { last: -1, prev: -1 };
        }
        let level = levels.len() - 1;
        levels[level] = Level { last: -1, prev: -1 };
        state.quoted = false;
        let mut mindepth = depth;
        self.setup(from.char, 1)?;

        // INC_FROM.
        macro_rules! inc_from {
            () => {{
                prev_from = from;
                prev_prev_from_syntax = prev_from_syntax;
                prev_from_syntax = self.text.syntax(prev_from);
                from = self.text.inc(from);
                if from.char < end { self.update_forward(from.char)?; }
            }};
        }
        macro_rules! cur {
            () => { levels.last_mut().expect("a current level") };
        }

        let mut at = if state.incomment != 0 {
            At::InComment
        } else if state.instring >= 0 {
            if start_quoted { At::QuotedInString } else { At::InString }
        } else if start_quoted {
            At::Quoted
        } else if from.char < end && self.two_char_comment_start(state, prev_from_syntax, prev_from, from) {
            inc_from!();
            prev_from_syntax = SMAX;
            At::AtComment
        } else {
            At::Top
        };
        loop {
            at = match at {
                At::Top => {
                    if from.char >= end { At::Done } else {
                        inc_from!();
                        if from.char < end && self.two_char_comment_start(state, prev_from_syntax, prev_from, from) {
                            inc_from!();
                            prev_from_syntax = SMAX;
                            At::AtComment
                        } else if prefix(prev_from_syntax) {
                            At::Top
                        } else {
                            let code = prev_from_syntax & 0xff;
                            match code {
                                SESCAPE | SCHARQUOTE | SWORD | SSYMBOL if stopbefore => At::Stop,
                                SESCAPE | SCHARQUOTE => { cur!().last = prev_from.char; At::Quoted }
                                SWORD | SSYMBOL => { cur!().last = prev_from.char; At::Symbol }
                                SCOMMENT_FENCE => {
                                    state.comstyle = ST_COMMENT_STYLE;
                                    state.incomment = -1;
                                    state.comstr_start = prev_from.char;
                                    At::AtComment
                                }
                                SCOMMENT => {
                                    state.comstyle = comment_style(prev_from_syntax, 0);
                                    state.incomment = if nested(prev_from_syntax) { 1 } else { -1 };
                                    state.comstr_start = prev_from.char;
                                    At::AtComment
                                }
                                SOPEN => {
                                    if stopbefore { At::Stop } else {
                                        depth += 1;
                                        cur!().last = prev_from.char;
                                        if levels.len() < LEVELS {
                                            levels.push(Level { last: -1, prev: -1 });
                                        } else {
                                            *cur!() = Level { last: -1, prev: -1 };
                                        }
                                        if targetdepth == depth { At::Done } else { At::Top }
                                    }
                                }
                                SCLOSE => {
                                    depth -= 1;
                                    if depth < mindepth { mindepth = depth; }
                                    if levels.len() > 1 { levels.pop(); }
                                    let level = cur!();
                                    level.prev = level.last;
                                    if targetdepth == depth { At::Done } else { At::Top }
                                }
                                SSTRING | SSTRING_FENCE => {
                                    state.comstr_start = from.char - 1;
                                    if stopbefore { At::Stop } else {
                                        cur!().last = prev_from.char;
                                        state.instring = if code == SSTRING {
                                            self.text.char_at(prev_from) as i32
                                        } else {
                                            ST_STRING_STYLE
                                        };
                                        if boundary_stop { At::Done } else { At::InString }
                                    }
                                }
                                _ => At::Top,
                            }
                        }
                    }
                }
                At::Quoted => {
                    if from.char == end { At::EndQuoted } else { inc_from!(); At::Symbol }
                }
                At::Symbol => {
                    let mut next = None;
                    while from.char < end {
                        if self.two_char_comment_start(state, prev_from_syntax, prev_from, from) {
                            inc_from!();
                            prev_from_syntax = SMAX;
                            next = Some(At::AtComment);
                            break;
                        }
                        match self.text.syntax(from) & 0xff {
                            SCHARQUOTE | SESCAPE => {
                                inc_from!();
                                if from.char == end { next = Some(At::EndQuoted); break; }
                            }
                            SWORD | SSYMBOL | SQUOTE => {}
                            _ => break,
                        }
                        inc_from!();
                    }
                    next.unwrap_or_else(|| {
                        let level = cur!();
                        level.prev = level.last;
                        At::Top
                    })
                }
                At::AtComment => {
                    if commentstop != 0 || boundary_stop { At::Done } else { At::InComment }
                }
                At::InComment => {
                    let prev = if from.char == self.begv.char { 0 } else { prev_from_syntax };
                    let (found, out, incomment, last) =
                        self.forw_comment(from, end, state.incomment, state.comstyle, prev)?;
                    from = out;
                    prev_from_syntax = last;
                    if !found {
                        state.incomment = incomment;
                        let code = prev_from_syntax & 0xff;
                        if code == SESCAPE || code == SCHARQUOTE { At::EndQuoted } else { At::Done }
                    } else {
                        inc_from!();
                        state.incomment = 0;
                        state.comstyle = 0;
                        prev_from_syntax = SMAX;
                        if boundary_stop { At::Done } else { At::Top }
                    }
                }
                At::InString | At::QuotedInString => {
                    let nofence = state.instring != ST_STRING_STYLE;
                    let mut quoted = matches!(at, At::QuotedInString);
                    let mut next = None;
                    loop {
                        if !quoted {
                            if from.char >= end { next = Some(At::Done); break; }
                            let c = self.text.char_at(from);
                            let c_code = self.text.syntax(from) & 0xff;
                            if nofence && c as i32 == state.instring && c_code == SSTRING { break; }
                            match c_code {
                                SSTRING_FENCE if !nofence => break,
                                SCHARQUOTE | SESCAPE => { inc_from!(); quoted = true; }
                                _ => {}
                            }
                        }
                        if quoted {
                            quoted = false;
                            if from.char >= end { next = Some(At::EndQuoted); break; }
                        }
                        inc_from!();
                    }
                    match next {
                        Some(next) => next,
                        None => {
                            // string_end:
                            state.instring = -1;
                            let level = cur!();
                            level.prev = level.last;
                            inc_from!();
                            if boundary_stop { At::Done } else { At::Top }
                        }
                    }
                }
                At::Stop => {
                    from = prev_from;
                    prev_from_syntax = prev_prev_from_syntax;
                    At::Done
                }
                At::EndQuoted => {
                    state.quoted = true;
                    At::Done
                }
                At::Done => break,
            };
        }
        state.depth = depth;
        state.mindepth = mindepth;
        state.thislevelstart = levels.last().expect("a current level").prev;
        state.prevlevelstart = if levels.len() == 1 { -1 } else { levels[levels.len() - 2].last };
        state.location = from.char;
        state.levelstarts = levels[..levels.len() - 1].iter().map(|l| Some(l.last)).collect();
        state.prev_syntax = if comstartend_first(prev_from_syntax) || state.quoted { prev_from_syntax } else { SMAX };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ASCII text with a per-character syntax.
    struct Plain { text: Vec<u8>, table: fn(u8) -> i32 }
    impl Text for Plain {
        fn char_at(&self, at: Pos) -> u32 { self.text.get(at.byte).copied().map_or(0, u32::from) }
        fn inc(&self, at: Pos) -> Pos { Pos { char: at.char + 1, byte: at.byte + 1 } }
        fn dec(&self, at: Pos) -> Pos { Pos { char: at.char - 1, byte: at.byte - 1 } }
        fn pos(&self, charpos: i64) -> Pos { let c = charpos.clamp(1, self.text.len() as i64 + 1); Pos { char: c, byte: c as usize - 1 } }
        fn syntax(&self, at: Pos) -> i32 { self.table_syntax(self.char_at(at)) }
        fn table_syntax(&self, c: u32) -> i32 { (self.table)(c as u8) }
    }
    fn lisp(c: u8) -> i32 {
        match c {
            b'(' => SOPEN, b')' => SCLOSE, b'"' => SSTRING, b'\\' => SESCAPE, b';' => SCOMMENT,
            b'\n' => SENDCOMMENT, b' ' => SWHITESPACE, b'\'' => SQUOTE | 1 << 20, _ => SWORD,
        }
    }
    fn scan<T>(text: &str, f: impl FnOnce(&mut Scanner<'_, Plain>) -> T) -> T {
        let plain = Plain { text: text.as_bytes().to_vec(), table: lisp };
        let ppss = BTreeMap::new();
        let options = Options { ignore_comments: true, open_paren_in_column_0_is_defun_start: true, ..Options::default() };
        let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options, ppss: &ppss, defun_start: None, modiff: 1 };
        f(&mut scanner)
    }

    #[test]
    fn sexps_skip_strings_comments_and_prefixes() {
        assert_eq!(scan("(a \"b)\" ; c)\n d) e", |s| s.scan_lists(1, 1, 0, true)), Ok(Some(17)));
        assert_eq!(scan("'(a) b", |s| s.scan_lists(1, 1, 0, true)), Ok(Some(5)));
        assert_eq!(scan("(a) b", |s| s.scan_lists(6, -2, 0, true)), Ok(Some(1)));
        assert_eq!(scan("(a b", |s| s.scan_lists(1, 1, 0, true)),
                   Err(Halt::ScanError("Unbalanced parentheses", 1, 5)));
        assert_eq!(scan("a) b", |s| s.scan_lists(1, 1, 1, false)), Ok(Some(3)));
        assert_eq!(scan(" ) b", |s| s.scan_lists(1, 1, 0, false)),
                   Err(Halt::ScanError("Containing expression ends prematurely", 2, 3)));
    }

    #[test]
    fn comments_move_both_ways() {
        assert_eq!(scan("  ; a\n ;b\nx", |s| s.forward_comment(s.text.pos(1), 5)).map(|(p, f)| (p.char, f)), Ok((11, false)));
        assert_eq!(scan("x ; a\n ;b\n", |s| s.forward_comment(s.text.pos(11), -2)).map(|(p, f)| (p.char, f)), Ok((3, true)));
    }

    #[test]
    fn partial_parse_reports_depth_strings_and_comments() {
        let state = scan("(a (b \"c", |s| {
            let mut state = ParseState::default();
            s.scan_sexps_forward(&mut state, s.text.pos(1), 9, i64::MIN, false, 0).map(|()| state)
        }).unwrap();
        assert_eq!((state.depth, state.instring, state.comstr_start, state.levelstarts.clone(), state.location),
                   (2, '"' as i32, 7, vec![Some(1), Some(4)], 9));
    }
}
