use super::*;
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

    pub(crate) fn completion_regexp_match(
        &mut self,
        pattern: Value,
        string: Value,
        fold: bool,
    ) -> std::result::Result<bool, Fault> {
        let translation = if fold {
            Some(CaseTranslation::from_case_table(std::rc::Rc::new(
                CompletionCase {
                    ranges: self.completion_case_ranges()?,
                    standard: CaseTranslation::standard(),
                },
            )))
        } else {
            None
        };
        Ok(self
            .vm
            .string_match_no_data(pattern, string, translation)?
            .is_some())
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
        let translation = fold.then(CaseTranslation::standard);
        Ok(self.vm.string_match_no_data(pattern, string, translation)?.is_some())
    }
}
