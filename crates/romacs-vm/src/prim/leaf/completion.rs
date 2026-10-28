use super::*;
use crate::regexp::Translation;
use romacs_regex_engine::{CaseTable, CaseTranslation};

#[derive(Debug)]
struct CompletionCase {
    ranges: Vec<(u32, u32, i64)>,
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
    /// compiles with.
    fn completion_translation(&mut self, fold: bool, file: bool)
        -> std::result::Result<(Translation, Option<CaseTranslation>), Fault> {
        Ok(match (fold, file) {
            (false, _) => (Translation::None, None),
            (true, true) => (Translation::Standard, Some(CaseTranslation::standard())),
            (true, false) => {
                let canon = self.completion_case_canon()?;
                let case = CompletionCase { ranges: self.completion_case_ranges()?, standard: CaseTranslation::standard() };
                (Translation::Completion(canon), Some(CaseTranslation::from_case_table(std::rc::Rc::new(case))))
            }
        })
    }

    pub(crate) fn completion_regexp_match(
        &mut self,
        pattern: Value,
        string: Value,
        fold: bool,
    ) -> std::result::Result<bool, Fault> {
        let (key, translation) = self.completion_translation(fold, false)?;
        Ok(self.vm.string_match_no_data(pattern, string, key, translation)?.is_some())
    }

    /// PATTERNS, the `completion-regexp-list' regexps, to test the
    /// candidates of one pass, folding case as `completion_regexp_match'
    /// (FILE: as `file_completion_regexp_match'); None when each candidate's
    /// own `syntax-table' properties matter, so each is tested alone.
    pub(crate) fn completion_matcher(&mut self, patterns: &[Value], fold: bool, file: bool)
        -> std::result::Result<Option<crate::regexp::StringMatcher>, Fault> {
        let (key, translation) = self.completion_translation(fold, file)?;
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
        let (key, translation) = self.completion_translation(fold, true)?;
        Ok(self.vm.string_match_no_data(pattern, string, key, translation)?.is_some())
    }
}
