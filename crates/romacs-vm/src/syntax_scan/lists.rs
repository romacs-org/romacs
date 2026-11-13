//! `Fforward_comment`, `Fbackward_prefix_chars` and `scan_lists`. Forward,
//! each is a loop over [`ScanAt`]'s labels: every update that can stop for
//! Lisp names the label it goes on at, and goes there, so that a resumed
//! scan runs the code an unstopped one does. Backward, a scan stops for
//! propertization at its start only, and after it for `syntax-ppss` in
//! `back_comment`'s decode.
use super::{
    comend_second, comment_style, comstart_first, comstart_second, nested, prefix, CommentHalt, Halt, Pos, Scan, ScanAt,
    Scanner, Stopped, Text, R, SCHARQUOTE, SCLOSE, SCOMMENT, SCOMMENT_FENCE, SENDCOMMENT, SESCAPE, SMATH, SOPEN, SQUOTE,
    SSTRING, SSTRING_FENCE, SSYMBOL, SWHITESPACE, SWORD, ST_COMMENT_STYLE,
};

impl<T: Text> Scanner<'_, T> {
    /// Fforward_comment after its argument checks: final point and value.
    pub(crate) fn forward_comment(&mut self, from: Pos, count: i64) -> Result<(Pos, bool), Stopped<Box<Scan>>> {
        // A stop here has read nothing: the scan starts again.
        self.setup(from.char, count)?;
        self.comments(Scan { from, count, ..Scan::default() })
    }

    /// `forward_comment` from SCAN: its start after the setup, or where a
    /// stop for Lisp left it.
    pub(crate) fn comments(&mut self, scan: Scan) -> Result<(Pos, bool), Stopped<Box<Scan>>> {
        if scan.count > 0 { self.comments_forward(scan) } else { self.comments_backward(scan) }
    }

    fn comments_forward(&mut self, scan: Scan) -> Result<(Pos, bool), Stopped<Box<Scan>>> {
        let Scan { mut at, mut from, mut count, mut c, mut syntax, mut code, mut comstyle, mut comnested, mut forw, .. } = scan;
        let stop = self.zv.char;
        macro_rules! suspend {
            ($halt:expr, $next:expr, $forw:expr) => {
                Stopped {
                    halt: $halt,
                    suspended: Some(Box::new(Scan { at: $next, from, count, c, syntax, code, comstyle, comnested, forw: $forw, ..scan })),
                }
            };
        }
        // UPDATE_SYNTAX_TABLE_FORWARD (from), going on at NEXT after a stop.
        macro_rules! update {
            ($next:expr) => {
                if let Err(halt) = self.update_forward(from.char) { return Err(suspend!(halt, $next, None)); }
            };
        }
        while count > 0 {
            at = match at {
                ScanAt::Top => {
                    if from.char == stop { return Ok((from, false)); }
                    c = self.text.char_at(from);
                    syntax = self.text.syntax(from);
                    code = syntax & 0xff;
                    comnested = nested(syntax);
                    comstyle = comment_style(syntax, 0);
                    from = self.text.inc(from);
                    update!(ScanAt::Stepped);
                    ScanAt::Stepped
                }
                ScanAt::Stepped => {
                    if from.char < stop && comstart_first(syntax) {
                        let other = self.text.syntax(from);
                        if comstart_second(other) {
                            code = SCOMMENT;
                            comstyle = comment_style(other, syntax);
                            comnested |= nested(other);
                            from = self.text.inc(from);
                            update!(ScanAt::Dispatch);
                        }
                    }
                    ScanAt::Dispatch
                }
                ScanAt::Dispatch => {
                    if code == SWHITESPACE || (code == SENDCOMMENT && c == '\n' as u32) {
                        ScanAt::Top
                    } else if code == SCOMMENT_FENCE {
                        comstyle = ST_COMMENT_STYLE;
                        ScanAt::Comment
                    } else if code == SCOMMENT {
                        ScanAt::Comment
                    } else {
                        return Ok((self.text.dec(from), false));
                    }
                }
                _ => {
                    let stepped = if let Some(saved) = forw.take() {
                        self.forw_run(saved, stop, comstyle)
                    } else {
                        self.forw_comment(from, stop, i64::from(comnested), comstyle, 0)
                    };
                    let (found, out, _, _) = match stepped {
                        Ok(stepped) => stepped,
                        Err(CommentHalt { halt, forw: saved }) => return Err(suspend!(halt, ScanAt::Comment, Some(saved))),
                    };
                    from = out;
                    if !found { return Ok((from, false)); }
                    from = self.text.inc(from);
                    // The comment is counted before the update that can stop.
                    count -= 1;
                    update!(ScanAt::Top);
                    ScanAt::Top
                }
            };
        }
        Ok((from, true))
    }

    fn comments_backward(&mut self, scan: Scan) -> Result<(Pos, bool), Stopped<Box<Scan>>> {
        let Scan { mut from, mut count, mut comstyle, mut comnested, mut mark, .. } = scan;
        let stop = self.begv.char;
        // back_comment's decode stopped for `syntax-ppss': it starts again,
        // then the loop goes on as after it.
        let mut decode = scan.at == ScanAt::Decode;
        while count < 0 {
            loop {
                let ended = if std::mem::take(&mut decode) {
                    self.back_decode(from, comnested, comstyle, None)
                } else {
                    if from.char <= stop { return Ok((self.begv, false)); }
                    from = self.text.dec(from);
                    let quoted = self.char_quoted(from)?;
                    mark = from;
                    let syntax = self.text.syntax(from);
                    let mut code = syntax & 0xff;
                    comstyle = 0;
                    comnested = nested(syntax);
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
                    } else if code != SENDCOMMENT {
                        if code != SWHITESPACE || quoted { return Ok((self.text.inc(from), false)); }
                        continue;
                    } else if quoted && self.options.comment_end_can_be_escaped {
                        Ok((false, from))
                    } else {
                        self.back_comment(from, stop, comnested, comstyle)
                    }
                };
                let (found, start) = ended.map_err(|halt| Stopped {
                    suspended: matches!(halt, Halt::Ppss(_))
                        .then(|| Box::new(Scan { at: ScanAt::Decode, from, count, comstyle, comnested, mark, ..scan })),
                    halt,
                })?;
                if found {
                    from = start;
                    break;
                }
                if self.text.char_at(mark) != '\n' as u32 {
                    // A two-character end: back past both.
                    if self.text.syntax(mark) & 0xff != SENDCOMMENT { from = self.text.inc(from); }
                    return Ok((self.text.inc(from), false));
                }
            }
            count += 1;
        }
        Ok((from, true))
    }

    /// Fbackward_prefix_chars: the new point. It stops at its start only.
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
    pub(crate) fn scan_lists(&mut self, from0: i64, count: i64, depth: i64, sexpflag: bool) -> Result<Option<i64>, Stopped<Box<Scan>>> {
        let from = self.text.pos(from0.clamp(self.begv.char, self.zv.char));
        // A stop here has read nothing: the scan starts again.
        self.setup(from.char, count)?;
        let min_depth = if depth > 0 { 0 } else { depth };
        self.lists(Scan { from, count, depth, min_depth, last_good: from0, ..Scan::default() }, sexpflag)
    }

    /// `scan_lists` from SCAN: its start after the setup, or where a stop
    /// for Lisp left it.
    pub(crate) fn lists(&mut self, scan: Scan, sexpflag: bool) -> Result<Option<i64>, Stopped<Box<Scan>>> {
        let Scan {
            mut at, mut from, mut count, mut depth, min_depth, mut mathexit, mut last_good, mut c, mut syntax, mut code,
            mut comstyle, mut comnested, mut mark, mut forw,
        } = scan;
        let stop = if count > 0 { self.zv.char } else { self.begv.char };
        let symbol = sexpflag && self.options.multibyte_syntax_as_symbol;
        let lose = |last_good, from: Pos| Halt::ScanError("Unbalanced parentheses", last_good, from.char);
        let premature = |last_good, from: Pos| Halt::ScanError("Containing expression ends prematurely", last_good, from.char);
        macro_rules! suspend {
            ($halt:expr, $next:expr, $forw:expr) => {
                Stopped {
                    halt: $halt,
                    suspended: Some(Box::new(Scan {
                        at: $next, from, count, depth, min_depth, mathexit, last_good, c, syntax, code, comstyle,
                        comnested, mark, forw: $forw,
                    })),
                }
            };
        }
        // UPDATE_SYNTAX_TABLE_FORWARD (from), going on at NEXT after a stop.
        macro_rules! update {
            ($next:expr) => {
                if let Err(halt) = self.update_forward(from.char) { return Err(suspend!(halt, $next, None)); }
            };
        }
        // done: a sexp or list is passed.
        macro_rules! done {
            () => {{ count -= 1; ScanAt::Top }};
        }
        // close1:
        macro_rules! close {
            () => {{
                depth -= 1;
                if depth == 0 {
                    done!()
                } else if depth < min_depth {
                    return Err(premature(last_good, from).into());
                } else {
                    ScanAt::Top
                }
            }};
        }
        while count > 0 {
            at = match at {
                ScanAt::Top => {
                    if from.char >= stop {
                        if depth != 0 { return Err(lose(last_good, from).into()); }
                        return Ok(None);
                    }
                    update!(ScanAt::Top);
                    c = self.text.char_at(from);
                    syntax = self.text.syntax(from);
                    code = self.class(from, symbol);
                    comnested = nested(syntax);
                    comstyle = comment_style(syntax, 0);
                    if depth == min_depth { last_good = from.char; }
                    from = self.text.inc(from);
                    update!(ScanAt::Stepped);
                    ScanAt::Stepped
                }
                ScanAt::Stepped => {
                    if from.char < stop && comstart_first(syntax) {
                        c = self.text.char_at(from);
                        let other = self.text.syntax(from);
                        if comstart_second(other) && self.options.ignore_comments {
                            code = SCOMMENT;
                            comstyle = comment_style(other, syntax);
                            comnested |= nested(other);
                            from = self.text.inc(from);
                            update!(ScanAt::Dispatch);
                        }
                    }
                    ScanAt::Dispatch
                }
                ScanAt::Dispatch if prefix(syntax) => ScanAt::Top,
                ScanAt::Dispatch => match code {
                    SESCAPE | SCHARQUOTE | SWORD | SSYMBOL => {
                        if code == SESCAPE || code == SCHARQUOTE {
                            if from.char == stop { return Err(lose(last_good, from).into()); }
                            from = self.text.inc(from);
                        }
                        if depth != 0 || !sexpflag { ScanAt::Top } else { ScanAt::Symbol }
                    }
                    SCOMMENT_FENCE | SCOMMENT => {
                        if code == SCOMMENT_FENCE { comstyle = ST_COMMENT_STYLE; }
                        if self.options.ignore_comments { ScanAt::Comment } else { ScanAt::Top }
                    }
                    SMATH if sexpflag => {
                        if from.char != stop && c == self.text.char_at(from) { from = self.text.inc(from); }
                        if mathexit {
                            mathexit = false;
                            close!()
                        } else {
                            mathexit = true;
                            depth += 1;
                            if depth == 0 { done!() } else { ScanAt::Top }
                        }
                    }
                    SOPEN => {
                        depth += 1;
                        if depth == 0 { done!() } else { ScanAt::Top }
                    }
                    SCLOSE => close!(),
                    SSTRING | SSTRING_FENCE => {
                        mark = self.text.dec(from);
                        ScanAt::String
                    }
                    _ => ScanAt::Top,
                },
                ScanAt::Symbol => {
                    // This word counts as a sexp: go on to its end.
                    while from.char < stop {
                        update!(ScanAt::Symbol);
                        match self.class(from, symbol) {
                            SCHARQUOTE | SESCAPE => {
                                from = self.text.inc(from);
                                if from.char == stop { return Err(lose(last_good, from).into()); }
                            }
                            SWORD | SSYMBOL | SQUOTE => {}
                            _ => break,
                        }
                        from = self.text.inc(from);
                    }
                    done!()
                }
                ScanAt::Comment => {
                    let stepped = if let Some(saved) = forw.take() {
                        self.forw_run(saved, stop, comstyle)
                    } else {
                        update!(ScanAt::Comment);
                        self.forw_comment(from, stop, i64::from(comnested), comstyle, 0)
                    };
                    let (found, out, _, _) = match stepped {
                        Ok(stepped) => stepped,
                        Err(CommentHalt { halt, forw: saved }) => return Err(suspend!(halt, ScanAt::Comment, Some(saved))),
                    };
                    from = out;
                    if found {
                        from = self.text.inc(from);
                        update!(ScanAt::Top);
                        ScanAt::Top
                    } else if depth == 0 {
                        done!()
                    } else {
                        return Err(lose(last_good, from).into());
                    }
                }
                ScanAt::String | ScanAt::Decode => {
                    let stringterm = self.text.char_at(mark);
                    loop {
                        if from.char >= stop { return Err(lose(last_good, from).into()); }
                        update!(ScanAt::String);
                        let c = self.text.char_at(from);
                        let c_code = self.class(from, symbol);
                        if if code == SSTRING { c == stringterm && c_code == SSTRING } else { c_code == SSTRING_FENCE } {
                            break;
                        }
                        if c_code == SCHARQUOTE || c_code == SESCAPE { from = self.text.inc(from); }
                        from = self.text.inc(from);
                    }
                    from = self.text.inc(from);
                    if depth == 0 && sexpflag { done!() } else { ScanAt::Top }
                }
            };
        }
        // back_comment, which stops in its decode for `syntax-ppss'.
        macro_rules! commented {
            ($ended:expr, $comnested:expr, $comstyle:expr) => {{
                let ended: R<(bool, Pos)> = $ended;
                let (found, start) = ended.map_err(|halt| Stopped {
                    suspended: matches!(halt, Halt::Ppss(_)).then(|| Box::new(Scan {
                        at: ScanAt::Decode, from, count, depth, min_depth, mathexit, last_good,
                        comnested: $comnested, comstyle: $comstyle, ..Scan::default()
                    })),
                    halt,
                })?;
                if found { from = start; }
            }};
        }
        // A stopped decode starts again; the loop goes on as after it.
        if at == ScanAt::Decode {
            commented!(self.back_decode(from, comnested, comstyle, None), comnested, comstyle);
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
                        commented!(self.back_comment(from, stop, comnested, comstyle), comnested, comstyle);
                    }
                    SCOMMENT_FENCE | SSTRING_FENCE => {
                        loop {
                            if from.char == stop { return Err(lose(last_good, from).into()); }
                            from = self.text.dec(from);
                            if !self.char_quoted(from)? && self.class(from, symbol) == code { break; }
                        }
                        if code == SSTRING_FENCE && depth == 0 && sexpflag { break 'scan true; }
                    }
                    SSTRING => {
                        let stringterm = self.text.char_at(from);
                        loop {
                            if from.char == stop { return Err(lose(last_good, from).into()); }
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
                    if depth < min_depth { return Err(premature(last_good, from).into()); }
                }
            };
            if !done {
                if depth != 0 { return Err(lose(last_good, from).into()); }
                return Ok(None);
            }
            count += 1;
        }
        Ok(Some(from.char))
    }
}
