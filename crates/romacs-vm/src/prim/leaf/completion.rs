use super::*;
use crate::regexp::Translation;
use romacs_regex_engine::{CaseTable, CaseTranslation};

#[derive(Debug)]
struct CompletionCase {
    ranges: std::sync::Arc<Vec<(u32, u32, i64)>>,
    standard: CaseTranslation,
}

impl CaseTable for CompletionCase {
    fn translate(&self, code: u32) -> u32 {
        let at = self.ranges.partition_point(|&(start, _, _)| start <= code);
        if let Some(&(_, end, mapped)) = at.checked_sub(1).and_then(|i| self.ranges.get(i)) {
            if code <= end {
                return if mapped < 0 { code } else { mapped as u32 };
            }
        }
        self.standard.translate(code)
    }
    fn write_tick(&self) -> u64 {
        0
    }
    fn folds_into_ascii(&self, _: u64, _: bool) -> Option<bool> {
        Some(true)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Leaf<'_> {
    pub(crate) fn completion_obarray_find(
        &self,
        table: Value,
        name: Value,
    ) -> std::result::Result<Option<Value>, Fault> {
        let name = self.string_view(name)?;
        Ok(self.vm.obarray_lookup_value(table, &name.bytes, name.chars))
    }

    pub(crate) fn completion_hash_next(
        &self,
        value: Value,
        from: usize,
    ) -> std::result::Result<Option<(usize, Value, Value)>, Fault> {
        let Some(data::Object::HashTable(table)) = self.vm.object_if(value) else {
            return Err(Fault::Wrong("hash-table-p", value));
        };
        Ok(table
            .next_used(from)
            .and_then(|i| table.entry(i).map(|(key, value)| (i, key, value))))
    }

    pub(crate) fn completion_hash_find(
        &mut self,
        table: Value,
        key: Value,
    ) -> std::result::Result<Option<(Value, Value)>, Fault> {
        self.vm.completion_hash_find(table, key)
    }

    /// How completion folds case when FOLD: as dired.c, with the standard
    /// canonical table (FILE), or as minibuf.c, with the current buffer's
    /// case-canon table; as the pattern cache's key and the translation it
    /// compiles with. WALK names the `all-completions' walk asking, which
    /// walks the canon table once (`walk_case_ranges').
    fn completion_translation(&mut self, fold: bool, file: bool, walk: Option<u64>)
        -> std::result::Result<(Translation, Option<CaseTranslation>), Fault> {
        Ok(match (fold, file) {
            (false, _) => (Translation::None, None),
            (true, true) => (Translation::Standard, Some(CaseTranslation::standard())),
            (true, false) => {
                let canon = self.completion_case_canon()?;
                let case = CompletionCase { ranges: self.walk_case_ranges(canon, walk)?, standard: CaseTranslation::standard() };
                (Translation::Completion(canon), Some(CaseTranslation::from_case_table(std::rc::Rc::new(case))))
            }
        })
    }

    /// A new `all-completions' walk's identity.
    pub(crate) fn next_completion_walk(&mut self) -> u64 {
        self.vm.completion_walks += 1;
        self.vm.completion_walks
    }

    /// The canon table CANON's case ranges, the walk of the whole table
    /// `completion_translation' folds with, made once per `all-completions'
    /// WALK: its predicate runs Lisp between candidates, and the walk
    /// resumes after each call. A predicate that changes the table's
    /// entries in place during the walk is not seen until the next walk.
    fn walk_case_ranges(&mut self, canon: Value, walk: Option<u64>)
        -> std::result::Result<std::sync::Arc<Vec<(u32, u32, i64)>>, Fault> {
        let root = match self.vm.object_if(canon) {
            Some(data::Object::CharTable(table)) => std::sync::Arc::as_ptr(&table.root) as usize,
            _ => 0,
        };
        if let (Some(walk), Some((at, kept, ranges))) = (walk, &self.vm.completion_case_memo)
            && *at == walk && *kept == root && root != 0 {
            return Ok(ranges.clone());
        }
        let ranges = std::sync::Arc::new(self.completion_case_ranges()?);
        if let Some(walk) = walk { self.vm.completion_case_memo = Some((walk, root, ranges.clone())); }
        Ok(ranges)
    }

    pub(crate) fn completion_regexp_match(
        &mut self,
        pattern: Value,
        string: Value,
        fold: bool,
    ) -> std::result::Result<bool, Fault> {
        let (key, translation) = self.completion_translation(fold, false, None)?;
        Ok(self.vm.string_match_no_data(pattern, string, key, translation)?.is_some())
    }

    /// PATTERNS, the `completion-regexp-list' regexps, to test the
    /// candidates of one pass, folding case as `completion_regexp_match'
    /// (FILE: as `file_completion_regexp_match'); None when each candidate's
    /// own `syntax-table' properties matter, so each is tested alone.
    pub(crate) fn completion_matcher(&mut self, patterns: &[Value], fold: bool, file: bool, walk: Option<u64>)
        -> std::result::Result<Option<crate::regexp::StringMatcher>, Fault> {
        let (key, translation) = self.completion_translation(fold, file, walk)?;
        self.vm.string_matcher(patterns, key, translation)
    }

    pub(crate) fn completion_matches(&mut self, matcher: &mut crate::regexp::StringMatcher, name: Value)
        -> std::result::Result<bool, Fault> {
        self.vm.string_matcher_matches(matcher, name)
    }

    /// dired.c file_name_completion matches `completion-regexp-list' with
    /// the standard canonical table when ignoring case, whatever the current
    /// buffer's case table.
    pub(crate) fn file_completion_regexp_match(
        &mut self,
        pattern: Value,
        string: Value,
        fold: bool,
    ) -> std::result::Result<bool, Fault> {
        let (key, translation) = self.completion_translation(fold, true, None)?;
        Ok(self.vm.string_match_no_data(pattern, string, key, translation)?.is_some())
    }
}

/// Orderless components compiled for one pass over candidates, during which
/// no Lisp runs: each component's regexp, and the native matcher
/// (crate::orderless) that answers for it where the native rules hold.
/// Case folds through the current buffer's case-canon table, as the
/// completion translation does; that translation, which walks the whole
/// table, is built only once a regexp has to run.
pub(crate) struct OrderlessPass {
    natives: Vec<Option<crate::orderless::Component>>,
    sources: Vec<Value>,
    regexps: Option<OrderlessRegexps>,
    fold: Option<OrderlessFold>,
    words: [bool; 128],
    original: Vec<u32>,
    folded: Vec<u32>,
}

struct OrderlessFold {
    canon: Value,
    /// Whether CANON is Romacs's own standard table, whose non-ASCII
    /// entries it leaves to the standard folding.
    native: bool,
    standard: CaseTranslation,
    ascii: [u32; 128],
    other: std::collections::HashMap<u32, u32>,
}

struct OrderlessRegexps {
    matcher: Option<crate::regexp::StringMatcher>,
    key: crate::regexp::Translation,
    translation: Option<CaseTranslation>,
}

/// A component as Lisp gives it: its regexp, and its native style and
/// pieces (character codes) when it has one.
pub(crate) struct OrderlessSpec {
    pub regexp: Value,
    pub native: Option<(crate::orderless::Style, Vec<Vec<u32>>)>,
}

impl Leaf<'_> {
    /// SPECS ready to match, ignoring case when FOLD as `all-completions'
    /// does under `completion-ignore-case'.
    pub(crate) fn orderless_pass(&mut self, specs: Vec<OrderlessSpec>, fold: bool) -> std::result::Result<OrderlessPass, Fault> {
        let fold = if fold {
            let canon = self.completion_case_canon()?;
            let native = self.native_case_mapping(canon, 2);
            let mut fold = OrderlessFold { canon, native, standard: CaseTranslation::standard(), ascii: [0; 128],
                                           other: Default::default() };
            for c in 0..128 { fold.ascii[c] = self.orderless_canon(&fold, c as u32)?; }
            Some(fold)
        } else { None };
        let mut pass = OrderlessPass { natives: vec![], sources: specs.iter().map(|spec| spec.regexp).collect(),
                                       regexps: None, fold, words: self.vm.ascii_word_syntax()?,
                                       original: vec![], folded: vec![] };
        for spec in specs {
            let native = match spec.native {
                Some((style, pieces)) => {
                    let mut folded = Vec::with_capacity(pieces.len());
                    for piece in pieces {
                        let mut codes = Vec::with_capacity(piece.len());
                        for c in piece { codes.push(self.orderless_fold(&mut pass, c)?); }
                        folded.push(codes);
                    }
                    // Word syntax a candidate's properties may change is the regexp's.
                    crate::orderless::Component::new(style, folded)
                        .filter(|c| !(c.reads_words() && self.vm.string_syntax_properties()))
                }
                None => None,
            };
            pass.natives.push(native);
        }
        Ok(pass)
    }

    /// CODE through FOLD's canon table, as CompletionCase translates it.
    fn orderless_canon(&self, fold: &OrderlessFold, code: u32) -> std::result::Result<u32, Fault> {
        let mapped = self.char_table_element(fold.canon, code as usize)?;
        Ok(match mapped.integer_value() {
            Some(n) if n >= 0 => n as u32,
            _ if fold.native && code >= 128 && mapped == Value::NIL => fold.standard.translate(code),
            _ => code,
        })
    }

    fn orderless_fold(&mut self, pass: &mut OrderlessPass, code: u32) -> std::result::Result<u32, Fault> {
        let Some(fold) = pass.fold.as_mut() else { return Ok(code) };
        if let Some(&folded) = fold.ascii.get(code as usize) { return Ok(folded); }
        if let Some(&folded) = fold.other.get(&code) { return Ok(folded); }
        let folded = self.orderless_canon(fold, code)?;
        fold.other.insert(code, folded);
        Ok(folded)
    }

    /// The spans orderless highlights for each of PASS's components in
    /// STRING, in character positions: a component's groups, or its whole
    /// match when no group took part; None for a component that does not
    /// match.  When ALL, stop at the first such.
    pub(crate) fn orderless_find(&mut self, pass: &mut OrderlessPass, string: Value, all: bool)
        -> std::result::Result<Vec<Option<Vec<(usize, usize)>>>, Fault> {
        let (ascii, unibyte_raw) = self.orderless_text(pass, string)?;
        let mut found = Vec::with_capacity(pass.natives.len());
        for i in 0..pass.natives.len() {
            let native = pass.natives[i].as_ref().filter(|c| !unibyte_raw && (ascii || !c.reads_words()));
            let spans = match native {
                Some(component) => {
                    let (original, words) = (&pass.original, &pass.words);
                    let word = |at: usize| words.get(original[at] as usize).copied().unwrap_or(false);
                    component.find(&pass.folded, &word)
                }
                None => self.orderless_regexp(pass, i, string)?,
            };
            let stop = all && spans.is_none();
            found.push(spans);
            if stop { break; }
        }
        Ok(found)
    }

    /// Decode STRING into PASS's buffers; whether it is ASCII, and whether
    /// it is unibyte with raw bytes, which only the regexps read right.
    fn orderless_text(&mut self, pass: &mut OrderlessPass, string: Value) -> std::result::Result<(bool, bool), Fault> {
        let view = self.vm.str_view(string).ok_or(Fault::Wrong("stringp", string))?;
        let (ascii, multibyte) = (view.bytes().is_ascii(), view.is_multibyte());
        pass.original.clear();
        pass.original.extend(view.chars());
        let mut original = std::mem::take(&mut pass.original);
        pass.folded.clear();
        for &c in &original {
            let folded = self.orderless_fold(pass, c)?;
            pass.folded.push(folded);
        }
        std::mem::swap(&mut pass.original, &mut original);
        Ok((ascii, !ascii && !multibyte))
    }

    fn orderless_regexp(&mut self, pass: &mut OrderlessPass, i: usize, string: Value)
        -> std::result::Result<Option<Vec<(usize, usize)>>, Fault> {
        if pass.regexps.is_none() {
            let (key, translation) = self.completion_translation(pass.fold.is_some(), false, None)?;
            let matcher = self.vm.string_matcher(&pass.sources, key, translation.clone())?;
            pass.regexps = Some(OrderlessRegexps { matcher, key, translation });
        }
        let regexps = pass.regexps.as_mut().unwrap();
        let groups = match regexps.matcher.as_mut() {
            Some(matcher) => self.vm.string_matcher_groups(matcher, i, string)?,
            None => self.vm.string_match_groups(pass.sources[i], string, regexps.key, regexps.translation.clone())?,
        };
        // orderless--highlight: (or (cddr (match-data)) (match-data)), whose
        // list ends at the last group that took part.
        Ok(groups.map(|groups| match groups.iter().rposition(Option::is_some) {
            Some(last) if last > 0 => groups[1..=last].iter().flatten().copied().collect(),
            _ => groups[0].into_iter().collect(),
        }))
    }
}
