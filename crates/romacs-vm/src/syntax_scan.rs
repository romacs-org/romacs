//! GNU syntax.c's scanning engine: `forw_comment`, `back_comment`,
//! `Fforward_comment`, `scan_lists` and `scan_sexps_forward`, transcribed
//! over a character view of the accessible buffer and a per-position
//! syntax lookup. Nothing here touches the VM, so a scan cannot collect.
//!
//! GNU calls Lisp from the middle of a scan in two places: just-in-time
//! `internal--syntax-propertize` when `parse-sexp-lookup-properties` is set
//! (`parse_sexp_propertize`), and `syntax-ppss` from `find_defun_start` when
//! `comment-use-syntax-ppss` is set. Here such a point stops the scan with a
//! [`Halt`] naming the call and the state the scan stopped in; the primitive
//! performs the call and the scan goes on from that state, so the calls
//! happen in GNU's order and with GNU's arguments, and no position is read
//! twice: what was read before a call keeps the syntax it had then.
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

pub(crate) fn comstart_first(s: i32) -> bool { (s >> 16) & 1 != 0 }
pub(crate) fn comstart_second(s: i32) -> bool { (s >> 17) & 1 != 0 }
fn comend_first(s: i32) -> bool { (s >> 18) & 1 != 0 }
fn comend_second(s: i32) -> bool { (s >> 19) & 1 != 0 }
fn comstartend_first(s: i32) -> bool { s & 0x50000 != 0 }
pub(crate) fn prefix(s: i32) -> bool { (s >> 20) & 1 != 0 }
fn nested(s: i32) -> bool { (s >> 22) & 1 != 0 }
/// SYNTAX_FLAGS_COMMENT_STYLE: FLAGS of the main delimiter character.
fn comment_style(flags: i32, other: i32) -> i32 {
    ((flags >> 21) & 1) | ((flags >> 22) & 2) | ((other >> 22) & 2)
}

/// Whether `forw_comment` only steps over a character of syntax S: no
/// comment delimiter, no first half of a two-character one, no escape.
/// After such a character the next is read with nothing pending.
pub(crate) fn comment_plain(s: i32) -> bool {
    !matches!(s & 0xff, SCOMMENT | SENDCOMMENT | SCOMMENT_FENCE | SESCAPE | SCHARQUOTE) && !comstartend_first(s)
}

/// A character position (1-based, like GNU's charpos) and its byte offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// Pass the characters from AT, before character position LIMIT, whose
    /// syntax `comment_plain` accepts: where the pass ends, and the syntax
    /// of the last character passed, if any.  A text may pass fewer than
    /// there are, or none (as here): `forw_comment` reads the rest one at
    /// a time.
    fn pass_comment_plain(&self, at: Pos, _limit: i64) -> (Pos, Option<i32>) {
        (at, None)
    }
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
romacs_snapshot::snapshot_struct!(DefunStart { pos, value, begv, modiff });

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

/// Where `forw_comment` goes on: the code after each update that can stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ForwAt { Read, EscapeSkipped, Incomment, NestedChecks }

impl ForwAt {
    pub(crate) const ALL: [ForwAt; 4] = [ForwAt::Read, ForwAt::EscapeSkipped, ForwAt::Incomment, ForwAt::NestedChecks];
}

/// `forw_comment`'s whole state between two steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Forw { pub at: ForwAt, pub from: Pos, pub nesting: i64, pub syntax: i32, pub code: i32, pub prev_syntax: i32 }

/// `forw_comment` stopped for Lisp, at FORW.
pub(crate) struct CommentHalt { pub halt: Halt, pub forw: Forw }
impl From<CommentHalt> for Halt {
    fn from(stopped: CommentHalt) -> Self {
        stopped.halt
    }
}

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
    /// The deepest depth the scan reached, as `mindepth` is the shallowest.
    pub maxdepth: i64,
    /// What the list form drops at END. A scan always records it; a scan
    /// started from a state that carries it continues exactly.
    pub continuation: Option<Continuation>,
}
impl Default for ParseState {
    /// internalize_parse_state (Qnil, ...).
    fn default() -> Self {
        Self {
            depth: 0, instring: -1, incomment: 0, comstyle: 0, quoted: false, mindepth: 0,
            thislevelstart: -1, prevlevelstart: -1, location: 0, comstr_start: -1,
            levelstarts: Vec::new(), prev_syntax: SMAX, maxdepth: 0, continuation: None,
        }
    }
}

/// What a scan stopped at END knows beyond `parse-partial-sexp''s value.
/// A scan resumed from a state with a continuation reaches what one scan
/// that never stopped would, unless the character before END can start a
/// two-character comment: the scan read it by its own class, not knowing
/// whether the next one completes the comment, and no state undoes that.
/// One resumed from Lisp's list resumes as GNU's does, which differs in
/// three more places:
/// - inside a symbol, GNU starts a new symbol at END, and the list form
///   counts the symbol complete there, which the continuation does not:
///   a symbol a comment start ends is never complete;
/// - GNU resets the current level's `last' and `prev', so element 2 and
///   the innermost start of the levels opened later forget the symbol,
///   string or list that began before END;
/// - after an escape in a comment, with `comment-end-can-be-escaped', GNU
///   reads the escaped character instead of skipping it.
///
/// One state no continuation resumes: a nested comment whose nesting an
/// escaped end used up (forw_comment decrements before it tests the
/// escape, and from 0 never ends the comment). The list reads it as
/// outside any comment; `spent' marks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Continuation {
    pub in_symbol: bool,
    pub top_last: i64,
    pub top_prev: i64,
    pub spent: bool,
}

pub(crate) struct Scanner<'a, T: Text> {
    pub text: &'a T,
    pub begv: Pos,
    pub zv: Pos,
    pub options: Options,
    /// The defun start `syntax-ppss` gave, by the position asked: the
    /// answer to the call the scan stopped for.
    pub ppss: &'a BTreeMap<i64, i64>,
    pub defun_start: Option<DefunStart>,
    pub modiff: u64,
}

/// Labels of `scan_sexps_forward`: GNU's gotos, and the code after each step
/// that can stop for Lisp (`inc_from!` names the label it goes on at).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum At {
    Top, AtComment, InComment, Quoted, Symbol, InString, QuotedInString, Stop, EndQuoted, Done,
    /// The start: a two-character comment starter may span FROM.
    Begin,
    /// The start of an exact continuation inside a comment, an escape pending.
    SkipEscapedInComment,
    /// After the escaped character's step.
    CommentEscaped,
    /// After a two-character comment starter's second step.
    CommentStarted,
    /// After Top's step: the dispatch on the character stepped over.
    TopDispatch,
    /// After the step into a comment starter that ends a symbol.
    SymbolToComment,
    /// After an escape's step inside a symbol.
    SymbolQuoted,
    /// After the step past a string's end.
    StringEnded,
    /// After the step past a comment's end.
    CommentEnded,
}
impl At {
    pub(crate) const ALL: [At; 19] = [
        At::Top, At::AtComment, At::InComment, At::Quoted, At::Symbol, At::InString, At::QuotedInString, At::Stop,
        At::EndQuoted, At::Done, At::Begin, At::SkipEscapedInComment, At::CommentEscaped, At::CommentStarted,
        At::TopDispatch, At::SymbolToComment, At::SymbolQuoted, At::StringEnded, At::CommentEnded,
    ];
}

/// A level of `scan_sexps_forward`: the start of its last sexp, and of the one
/// before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Level { pub last: i64, pub prev: i64 }

/// `scan_sexps_forward`'s locals between two steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Locals {
    pub from: Pos,
    pub prev_from: Pos,
    pub prev_from_syntax: i32,
    pub prev_prev_from_syntax: i32,
    pub depth: i64,
    pub mindepth: i64,
    pub maxdepth: i64,
    pub levels: Vec<Level>,
    pub in_symbol: bool,
    pub symbol_prev: i64,
    pub spent: bool,
}

/// A parse stopped for Lisp, to go on at AT: GNU's scan calls Lisp and goes
/// on, so what it read before is not read again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suspended { pub at: At, pub locals: Locals, pub state: ParseState, pub forw: Option<Forw> }

/// A scan's stop: HALT, and where it goes on, unless it stopped before
/// reading anything.
pub(crate) struct Stopped<S = Box<Suspended>> { pub halt: Halt, pub suspended: Option<S> }
impl<S> From<Halt> for Stopped<S> {
    fn from(halt: Halt) -> Self {
        Self { halt, suspended: None }
    }
}

/// Where `scan_lists` and `forward_comment` go on. Forward, the code after
/// each update that can stop: the loop's top, the two-character comment
/// check after its step, the dispatch on the character stepped over, and
/// inside a symbol, a comment or a string. Backward, `back_comment`'s
/// decode, which asks `syntax-ppss`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScanAt { #[default] Top, Stepped, Dispatch, Symbol, Comment, String, Decode }
impl ScanAt {
    pub(crate) const ALL: [ScanAt; 7] = [
        ScanAt::Top, ScanAt::Stepped, ScanAt::Dispatch, ScanAt::Symbol, ScanAt::Comment, ScanAt::String, ScanAt::Decode,
    ];
}

/// The locals of `scan_lists` or `forward_comment` between two steps, the
/// depths only `scan_lists`'s. A scan stopped for Lisp goes on from them:
/// GNU's scan calls Lisp and goes on, so what it read before is not read
/// again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Scan {
    pub at: ScanAt,
    pub from: Pos,
    pub count: i64,
    pub depth: i64,
    pub min_depth: i64,
    pub mathexit: bool,
    pub last_good: i64,
    /// The character the step read, its syntax and class, and the style
    /// and nesting of the comment it starts or ends.
    pub c: u32,
    pub syntax: i32,
    pub code: i32,
    pub comstyle: i32,
    pub comnested: bool,
    /// A string's opener; backward, the last character of a comment end.
    pub mark: Pos,
    /// The comment scan it stopped in.
    pub forw: Option<Forw>,
}

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
    fn forw_comment(&self, from: Pos, stop: i64, nesting: i64, style: i32, prev_syntax: i32)
        -> Result<(bool, Pos, i64, i32), CommentHalt> {
        let nesting = if nesting <= 0 { -1 } else { nesting };
        // GNU's incomment flag: a known syntax before FROM is checked as the
        // first half of an ender or nested starter before FROM is read.
        let at = if prev_syntax != 0 && from.char < stop { ForwAt::Incomment } else { ForwAt::Read };
        self.forw_run(Forw { at, from, nesting, syntax: prev_syntax, code: prev_syntax & 0xff, prev_syntax }, stop, style)
    }

    /// `forw_comment` from FORW, which a stop for Lisp left: each label is the
    /// code after the update that stopped.
    fn forw_run(&self, forw: Forw, stop: i64, style: i32) -> Result<(bool, Pos, i64, i32), CommentHalt> {
        let Forw { mut at, mut from, mut nesting, mut syntax, mut code, mut prev_syntax } = forw;
        macro_rules! update {
            ($next:expr) => {
                if let Err(halt) = self.update_forward(from.char) {
                    return Err(CommentHalt { halt, forw: Forw { at: $next, from, nesting, syntax, code, prev_syntax } });
                }
            };
        }
        loop {
            at = match at {
                ForwAt::Read => {
                    // A run of characters that are only stepped over, at
                    // once: none of them ends the comment, starts a
                    // two-character delimiter, or reaches text that is
                    // still to be propertized (`update_forward` stops
                    // there, so the character before it is read below).
                    let plain_limit = match self.options.propertize_done {
                        Some(done) if done < self.zv.char => stop.min(done - 1),
                        _ => stop,
                    };
                    if let (to, Some(last)) = self.text.pass_comment_plain(from, plain_limit) {
                        syntax = last;
                        code = syntax & 0xff;
                        from = to;
                    }
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
                        update!(ForwAt::EscapeSkipped);
                        ForwAt::EscapeSkipped
                    } else {
                        from = self.text.inc(from);
                        update!(ForwAt::Incomment);
                        ForwAt::Incomment
                    }
                }
                ForwAt::EscapeSkipped => {
                    if from.char == stop {
                        ForwAt::Read
                    } else {
                        syntax = SMAX;
                        code = syntax;
                        from = self.text.inc(from);
                        update!(ForwAt::Incomment);
                        ForwAt::Incomment
                    }
                }
                // forw_incomment:
                ForwAt::Incomment => {
                    if from.char < stop && comend_first(syntax) {
                        let other = self.text.syntax(from);
                        if comend_second(other) && comment_style(syntax, other) == style
                            && (if nested(syntax) || nested(other) { nesting > 0 } else { nesting < 0 }) {
                            syntax = SMAX;
                            nesting -= 1;
                            if nesting <= 0 { break; }
                            from = self.text.inc(from);
                            update!(ForwAt::NestedChecks);
                        }
                    }
                    ForwAt::NestedChecks
                }
                ForwAt::NestedChecks => {
                    if nesting > 0 && from.char < stop && comstart_first(syntax) {
                        let other = self.text.syntax(from);
                        if comment_style(other, syntax) == style && comstart_second(other)
                            && (nested(syntax) || nested(other)) {
                            syntax = SMAX;
                            from = self.text.inc(from);
                            nesting += 1;
                            update!(ForwAt::Read);
                        }
                    }
                    ForwAt::Read
                }
            };
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
        self.back_decode(comment_end, comnested, comstyle, defun_start)
    }

    /// `back_comment`'s lossage: whether a comment ends at `comment_end`
    /// and where it starts, decoded forward from a safe place,
    /// `defun_start` or what `find_defun_start` gives. That stops for
    /// `syntax-ppss` before anything is read here, so a stopped decode
    /// starts again.
    fn back_decode(&mut self, comment_end: Pos, comnested: bool, comstyle: i32, defun_start: Option<Pos>) -> R<(bool, Pos)> {
        let mut from;
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
        // The call the scan stopped for is past the cache it missed.
        let answered = self.ppss.get(&pos.char).copied();
        if let (None, Some(cache)) = (answered, self.defun_start) {
            if pos.char <= cache.pos + 1000 && pos.char >= cache.value
                && cache.begv == self.begv.char && cache.modiff == self.modiff {
                return Ok(self.text.pos(cache.value));
            }
        }
        let value = if let Some(value) = answered {
            value
        } else if self.options.comment_use_syntax_ppss {
            return Err(Halt::Ppss(pos.char));
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
    pub(crate) fn scan_sexps_forward(&mut self, state: &mut ParseState, from: Pos, end: i64,
                                     targetdepth: i64, stopbefore: bool, commentstop: i32) -> R<()> {
        self.parse(state, from, end, targetdepth, stopbefore, commentstop).map_err(|stopped| stopped.halt)
    }

    /// `scan_sexps_forward`, stopping for Lisp so that `resume` goes on.
    pub(crate) fn parse(&mut self, state: &mut ParseState, from: Pos, end: i64,
                        targetdepth: i64, stopbefore: bool, commentstop: i32) -> Result<(), Stopped> {
        const LEVELS: usize = 100;
        let mut prev_from = from;
        if from.char != self.begv.char { prev_from = self.text.dec(prev_from); }
        let start_quoted = state.quoted;
        let mut levels = vec![Level { last: -1, prev: -1 }];
        for start in &state.levelstarts {
            let level = levels.len() - 1;
            if let Some(start) = *start { levels[level].last = start; }
            if levels.len() < LEVELS { levels.push(Level { last: -1, prev: -1 }); }
            let level = levels.len() - 1;
            levels[level] = Level { last: -1, prev: -1 };
        }
        let level = levels.len() - 1;
        let exact = state.continuation;
        levels[level] = exact.map_or(Level { last: -1, prev: -1 }, |c| Level { last: c.top_last, prev: c.top_prev });
        state.quoted = false;
        // A stop here has read nothing: the rerun starts again.
        self.setup(from.char, 1)?;
        let at = if state.incomment != 0 {
            if start_quoted && exact.is_some() && self.options.comment_end_can_be_escaped {
                At::SkipEscapedInComment
            } else {
                At::InComment
            }
        } else if state.instring >= 0 {
            if start_quoted { At::QuotedInString } else { At::InString }
        } else if start_quoted {
            At::Quoted
        } else if exact.is_some_and(|c| c.in_symbol) {
            At::Symbol
        } else {
            At::Begin
        };
        let locals = Locals {
            from, prev_from, prev_from_syntax: state.prev_syntax, prev_prev_from_syntax: SMAX,
            depth: state.depth, mindepth: state.depth, maxdepth: state.depth, levels,
            in_symbol: false, symbol_prev: -1, spent: false,
        };
        self.run(state, at, locals, None, (end, targetdepth, stopbefore, commentstop))
    }

    /// Go on with the parse SUSPENDED stopped, to END as it was asked to:
    /// the state it ends in.
    pub(crate) fn resume(&mut self, suspended: Suspended, end: i64, targetdepth: i64, stopbefore: bool,
                         commentstop: i32) -> Result<ParseState, Stopped> {
        let Suspended { at, locals, mut state, forw } = suspended;
        self.run(&mut state, at, locals, forw, (end, targetdepth, stopbefore, commentstop))?;
        Ok(state)
    }

    /// `scan_sexps_forward`'s loop, from AT with LOCALS (and FORW, a comment
    /// scan it stopped in): every step that can stop for Lisp names the
    /// label it goes on at, and goes there, so that a resumed parse runs the
    /// code an unstopped one does.
    #[allow(unused_assignments)] // INC_FROM's fields are not all read on every path.
    fn run(&mut self, state: &mut ParseState, at: At, locals: Locals, forw: Option<Forw>,
           (end, targetdepth, stopbefore, commentstop): (i64, i64, bool, i32)) -> Result<(), Stopped> {
        const LEVELS: usize = 100;
        let boundary_stop = commentstop == -1;
        let Locals {
            mut from, mut prev_from, mut prev_from_syntax, mut prev_prev_from_syntax, mut depth, mut mindepth,
            mut maxdepth, mut levels, mut in_symbol, mut symbol_prev, mut spent,
        } = locals;
        let mut forw = forw;
        let mut at = at;

        macro_rules! suspend {
            ($halt:expr, $next:expr, $forw:expr) => {
                Stopped {
                    halt: $halt,
                    suspended: Some(Box::new(Suspended {
                        at: $next,
                        locals: Locals {
                            from, prev_from, prev_from_syntax, prev_prev_from_syntax, depth, mindepth, maxdepth,
                            levels: std::mem::take(&mut levels), in_symbol, symbol_prev, spent,
                        },
                        state: state.clone(),
                        forw: $forw,
                    })),
                }
            };
        }
        // INC_FROM, going on at NEXT after a stop.
        macro_rules! inc_from {
            ($next:expr) => {{
                prev_from = from;
                prev_prev_from_syntax = prev_from_syntax;
                prev_from_syntax = self.text.syntax(prev_from);
                from = self.text.inc(from);
                if from.char < end {
                    if let Err(halt) = self.update_forward(from.char) {
                        return Err(suspend!(halt, $next, None));
                    }
                }
            }};
        }
        macro_rules! cur {
            () => { levels.last_mut().expect("a current level") };
        }

        loop {
            at = match at {
                At::Begin => {
                    if from.char < end && self.two_char_comment_start(state, prev_from_syntax, prev_from, from) {
                        inc_from!(At::CommentStarted);
                        At::CommentStarted
                    } else {
                        At::Top
                    }
                }
                At::SkipEscapedInComment => {
                    // forw_comment skips the character an escape quotes.
                    if from.char < end {
                        inc_from!(At::CommentEscaped);
                        At::CommentEscaped
                    } else {
                        At::EndQuoted
                    }
                }
                At::CommentEscaped => {
                    prev_from_syntax = SMAX;
                    At::InComment
                }
                At::CommentStarted => {
                    prev_from_syntax = SMAX;
                    At::AtComment
                }
                At::Top => {
                    if from.char >= end { At::Done } else {
                        inc_from!(At::TopDispatch);
                        At::TopDispatch
                    }
                }
                At::TopDispatch => {
                    if from.char < end && self.two_char_comment_start(state, prev_from_syntax, prev_from, from) {
                        inc_from!(At::CommentStarted);
                        At::CommentStarted
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
                                    maxdepth = maxdepth.max(depth);
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
                At::Quoted => {
                    if from.char == end { At::EndQuoted } else {
                        inc_from!(At::Symbol);
                        At::Symbol
                    }
                }
                At::Symbol => {
                    let mut next = None;
                    let mut ended = false;
                    while from.char < end {
                        if self.two_char_comment_start(state, prev_from_syntax, prev_from, from) {
                            inc_from!(At::SymbolToComment);
                            next = Some(At::SymbolToComment);
                            break;
                        }
                        match self.text.syntax(from) & 0xff {
                            SCHARQUOTE | SESCAPE => {
                                inc_from!(At::SymbolQuoted);
                                next = Some(At::SymbolQuoted);
                                break;
                            }
                            SWORD | SSYMBOL | SQUOTE => {}
                            _ => {
                                ended = true;
                                break;
                            }
                        }
                        inc_from!(At::Symbol);
                    }
                    in_symbol = next.is_none() && !ended;
                    symbol_prev = cur!().prev;
                    next.unwrap_or_else(|| {
                        let level = cur!();
                        level.prev = level.last;
                        At::Top
                    })
                }
                At::SymbolToComment => {
                    in_symbol = false;
                    symbol_prev = cur!().prev;
                    prev_from_syntax = SMAX;
                    At::AtComment
                }
                At::SymbolQuoted => {
                    in_symbol = false;
                    symbol_prev = cur!().prev;
                    if from.char == end { At::EndQuoted } else {
                        inc_from!(At::Symbol);
                        At::Symbol
                    }
                }
                At::AtComment => {
                    if commentstop != 0 || boundary_stop { At::Done } else { At::InComment }
                }
                At::InComment => {
                    let stepped = if let Some(saved) = forw.take() {
                        self.forw_run(saved, end, state.comstyle)
                    } else {
                        let prev = if from.char == self.begv.char { 0 } else { prev_from_syntax };
                        self.forw_comment(from, end, state.incomment, state.comstyle, prev)
                    };
                    let (found, out, incomment, last) = match stepped {
                        Ok(stepped) => stepped,
                        Err(CommentHalt { halt, forw: saved }) => return Err(suspend!(halt, At::InComment, Some(saved))),
                    };
                    from = out;
                    prev_from_syntax = last;
                    if !found {
                        spent = incomment == 0;
                        state.incomment = incomment;
                        let code = prev_from_syntax & 0xff;
                        if code == SESCAPE || code == SCHARQUOTE { At::EndQuoted } else { At::Done }
                    } else {
                        inc_from!(At::CommentEnded);
                        At::CommentEnded
                    }
                }
                At::CommentEnded => {
                    state.incomment = 0;
                    state.comstyle = 0;
                    prev_from_syntax = SMAX;
                    if boundary_stop { At::Done } else { At::Top }
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
                                SCHARQUOTE | SESCAPE => {
                                    // Going on inline is entering QuotedInString.
                                    inc_from!(At::QuotedInString);
                                    quoted = true;
                                }
                                _ => {}
                            }
                        }
                        if quoted {
                            quoted = false;
                            if from.char >= end { next = Some(At::EndQuoted); break; }
                        }
                        inc_from!(At::InString);
                    }
                    match next {
                        Some(next) => next,
                        None => {
                            // string_end:
                            state.instring = -1;
                            let level = cur!();
                            level.prev = level.last;
                            inc_from!(At::StringEnded);
                            At::StringEnded
                        }
                    }
                }
                At::StringEnded => {
                    if boundary_stop { At::Done } else { At::Top }
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
        state.maxdepth = maxdepth;
        let top = levels.last().expect("a current level");
        let top_prev = if in_symbol { symbol_prev } else { top.prev };
        state.continuation = Some(Continuation { in_symbol, top_last: top.last, top_prev, spent });
        Ok(())
    }
}

mod lists;

#[cfg(test)]
#[path = "syntax_scan_tests.rs"]
pub(crate) mod tests;
