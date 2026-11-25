//! A string search of a pattern that reads no syntax or category table
//! (`CompiledPattern::reads_lookup`): it passes the engine's constant
//! `DefaultSyntaxLookup`, so the VM prepares no syntax for it; and the
//! search without match data, which reads its subject where it lies.
use super::{Fault, PatternId, SearchResult, Translation, syntax};
use crate::{Value, Vm, prim};
use romacs_regex as regex;

impl Vm {
    /// PATTERN's cached compilation for POSIX and KEY when it reads no syntax
    /// or category table (`reads_lookup`), found from the string's own bytes:
    /// such a pattern matches alike under any table, so its search needs
    /// neither the current syntax nor the subject's properties, and passes
    /// the constant `DefaultSyntaxLookup`; set for a search of MULTIBYTE
    /// text. None leaves the search to the full path, which compiles and
    /// caches as before.
    #[inline(always)]
    pub(super) fn regexp_plain_hit(&mut self, pattern: Value, posix: bool, key: Translation, multibyte: bool)
        -> Option<PatternId> {
        let view = self.str_view(pattern)?;
        let pattern_multibyte = view.is_multibyte();
        let slot = self.mutator_rest.regexp_cache.find(view.bytes(), |e| e.multibyte == pattern_multibyte
            && e.posix == posix && e.translation == key && e.syntax.is_none() && !e.reads_lookup)?;
        self.mutator_rest.regexp_cache.touch(slot, Some(multibyte))
    }

    /// PATTERN against STRING with the translation KEY names (built as
    /// TRANSLATION), without touching the match data.
    pub(crate) fn string_match_no_data(&mut self, pattern: Value, string: Value, key: Translation,
                                       translation: Option<regex::CaseTranslation>) -> std::result::Result<Option<usize>, prim::Fault> {
        let run = |vm: &mut Self| -> SearchResult<Option<usize>> {
            let multibyte = vm.regexp_subject(string)?.multibyte;
            let (id, source) = if let Some(id) = vm.regexp_plain_hit(pattern, false, key, multibyte) {
                (id, None)
            } else {
                let pattern = vm.regexp_input(pattern)?;
                let source = vm.regexp_syntax(Some(string))?;
                let id = vm.regexp_pattern(&pattern, false, key, move || translation, &source)?;
                vm.regexp_aim(id, multibyte)?;
                (id, Some(source))
            };
            let armed = vm.arm_regexp_quit();
            // The string is read where it lies: nothing moves or changes it
            // while the search borrows the VM.
            let found = {
                let input = vm.regexp_subject(string)?;
                let compiled = vm.regexp_cached(id)?;
                let (bytes, end) = (&input.bytes[..], input.bytes.len().cast_signed());
                let found = if let Some(source) = source {
                    regex::re_search(compiled, bytes, 0, end, &syntax::VmSyntax::new(vm, source), usize::MAX)
                } else {
                    regex::re_search(compiled, bytes, 0, end, &regex::DefaultSyntaxLookup, usize::MAX)
                };
                found.map(|(at, _)| at)
            };
            vm.regexp_quit(armed)?;
            if regex::take_matcher_overflow() {
                return Err(Fault::Text("error", regex::MATCHER_OVERFLOW_MESSAGE.into()));
            }
            Ok(found)
        };
        // A leaf reaches this search, and the matcher answers inspections.
        let found = if crate::host::inspect_enabled() { self.with_site_leaf(run) } else { run(self) };
        found.map_err(|fault| self.regexp_fault(fault))
    }
}
