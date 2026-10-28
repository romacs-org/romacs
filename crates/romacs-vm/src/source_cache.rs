//! Source-addressed IR. Function IDs are stable; their constants remain GC roots.
use super::*;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
mod persistent;
mod prepared;
mod tier;
pub use prepared::PreparedHandle;
pub use tier::DIRECT_NODE_LIMIT;
const COMPILER: &str = concat!("vm2-source-ir-2:", env!("VM2_COMPILER_ID"));
#[derive(Clone, Copy, Default, Debug)]
pub struct SourceCacheStats {
    pub hits: u64,
    pub compilations: u64,
    pub invalidations: u64,
    pub disk_hits: u64,
    pub direct: u64,
}
#[derive(Clone)]
struct Entry {
    function: usize,
    macros: Vec<(usize, u64, String)>,
    specials: Vec<(String, SpecialKind)>,
    calls: Vec<(usize, bool)>,
}
#[derive(Default)]
pub(super) struct SourceCache {
    entries: NameMap<String, Entry>,
    stats: SourceCacheStats,
    store: Option<PathBuf>,
    prepared: Vec<prepared::Prepared>,
    last_guard: Option<Entry>,
}
fn key(source: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(COMPILER.as_bytes());
    hash.update([0]);
    hash.update(source.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in hash.finalize() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    out
}
impl Vm {
    pub(super) fn cache_primitive(&mut self, p: Primitive, n: usize) -> Result<Value> {
        if n != 0 {
            return Err("wrong primitive arity".into());
        }
        match p {
            Primitive::CacheClear => {
                self.clear_source_cache();
                if let Some(store) = &self.source_cache.store {
                    for file in std::fs::read_dir(store).map_err(|e| e.to_string())? {
                        let file = file.map_err(|e| e.to_string())?;
                        if file
                            .file_name()
                            .to_str()
                            .is_some_and(|s| s.starts_with("source-") && s.len() == 71)
                        {
                            std::fs::remove_file(file.path()).map_err(|e| e.to_string())?;
                        }
                    }
                }
                Ok(Value::NIL)
            }
            Primitive::CacheStats => {
                let s = self.source_cache.stats;
                let values = [
                    s.hits,
                    s.compilations,
                    s.invalidations,
                    s.disk_hits,
                    s.direct,
                    self.source_cache.entries.len() as u64,
                ];
                let values = values
                    .into_iter()
                    .map(|v| Value::integer(v as i64))
                    .collect::<Result<Vec<_>>>()?;
                Ok(self.allocate_object(data::Object::Vector(values)))
            }
            Primitive::CacheEntries => {
                let mut keys: Vec<_> = self.source_cache.entries.keys().cloned().collect();
                keys.sort();
                let values = keys
                    .into_iter()
                    .map(|s| self.literal(&Form::String(s)))
                    .collect::<Result<Vec<_>>>()?;
                Ok(self.allocate_object(data::Object::Vector(values)))
            }
            _ => unreachable!(),
        }
    }
    pub fn source_cache_stats(&self) -> SourceCacheStats {
        self.source_cache.stats
    }
    pub fn clear_source_cache(&mut self) {
        self.source_cache.entries.clear();
        self.source_cache.last_guard = None;
    }
    // This cache already exists; bound its roots rather than letting distinct
    // request texts keep the entire session alive. Prepared handles stay pinned.
    fn trim_source_cache(&mut self) {
        if self.source_cache.entries.len() >= 256 {
            self.source_cache.entries.clear();
        }
    }
    pub(super) fn source_code_roots(&self, roots: &mut Vec<usize>) {
        roots.extend(self.source_cache.entries.values().map(|e| e.function));
        roots.extend(self.source_cache.prepared.iter().map(|p| p.function));
    }
    pub fn set_source_cache_store(&mut self, path: PathBuf) -> Result<()> {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        self.source_cache.store = Some(path);
        Ok(())
    }
    fn cache_valid(&self, e: &Entry) -> bool {
        e.macros.iter().all(|(s, v, stamp)| {
            self.macros.contains(s)
                && self.slots[*s].version == *v
                && self.macro_stamp(*s).as_ref() == Some(stamp)
        }) && e
            .specials
            .iter()
            .all(|(s, v)| self.special_kind(s) == *v)
            && e.calls.iter().all(|(s, v)| self.call_is_macro(*s) == *v)
    }
    pub(super) fn cached_submission(&mut self, source: &str) -> Option<usize> {
        if self.source_cache.entries.is_empty() {
            return None;
        }
        let k = key(source);
        let e = self.source_cache.entries.get(&k)?;
        if self.cache_valid(e) {
            self.source_cache.stats.hits += 1;
            Some(e.function)
        } else {
            self.source_cache.entries.remove(&k);
            self.source_cache.stats.invalidations += 1;
            None
        }
    }
    pub(super) fn cached_source_form(&mut self, source: &str, form: &Form) -> Result<usize> {
        if let Some(f) = self.cached_submission(source) {
            return Ok(f);
        }
        let k = key(source);
        if let Some(f) = self.load_source_ir(&k, form)? {
            return Ok(f);
        }
        self.expanded_macros.clear();
        self.expanded_symbols.clear();
        let (function, compiled) = self.compile_request(form)?;
        self.source_cache.stats.compilations += 1;
        let mut names = NameSet::default();
        collect_symbols(form, &mut names);
        names.extend(self.expanded_symbols.iter().cloned());
        for f in compiled.iter().map(|&id| &self.functions[id]) {
            for op in &f.code {
                if let Op::Global(_, s) | Op::Bind(s, _) | Op::SetGlobal(s, _) = *op {
                    names.insert(self.symbol_names[s].clone());
                }
            }
        }
        let mut specials: Vec<_> = names
            .into_iter()
            .map(|s| {
                let v = self.special_kind(&s);
                (s, v)
            })
            .collect();
        specials.sort();
        let mut calls: Vec<_> = specials
            .iter()
            .map(|(name, _)| {
                let slot = self.slot(name);
                (slot, self.call_is_macro(slot))
            })
            .collect();
        calls.extend(
            compiled
                .iter()
                .map(|&id| &self.functions[id])
                .flat_map(|f| &f.code)
                .filter_map(|op| {
                    if let Op::Direct(_, s, _, _, _) | Op::Prepared(_, _, s, _, _, _) = *op {
                        Some((s, self.call_is_macro(s)))
                    } else {
                        None
                    }
                }),
        );
        calls.sort_unstable();
        calls.dedup();
        let mut macros = self.expanded_macros.clone();
        macros.sort_unstable();
        macros.dedup();
        let macros = macros
            .into_iter()
            .map(|(s, v)| (s, v, self.macro_stamp(s).unwrap_or_default()))
            .collect();
        let e = Entry {
            function,
            macros,
            specials,
            calls,
        };
        self.source_cache.last_guard = Some(e.clone());
        // Arbitrary macro code can read mutable state: only constant expansions
        // are reusable until a complete expansion-effect tracker exists.
        // A lazily held body (R215) is mutable state: never reuse or persist it.
        let immutable = compiled.iter().all(|&id| self.functions[id].lazy.is_none())
            && compiled
            .iter()
            .map(|&id| &self.functions[id])
            .flat_map(|f| &f.code)
            .all(|op| {
                let Op::Const(_, v) = *op else { return true };
                matches!(v.tag(), 1 | 2 | 7)
                    || v == Value::NIL
                    || matches!(self.object_if(v), Some(data::Object::Float(_)))
            });
        if immutable
            && e.macros
                .iter()
                .all(|(s, _, stamp)| !stamp.is_empty() && self.constant_macro(*s))
        {
            // Form::Object owns a root in this VM, even when optimization
            // removed its Op::Const. Never recompile such input in `portable`.
            if self.source_cache.store.is_some() && e.macros.is_empty() && !contains_local_object(form) {
                // Reproduce this compilation's declarations kind for kind: a
                // file-local special must not become a global one.
                let mut portable = Vm::new();
                for (name, kind) in &e.specials {
                    match kind {
                        SpecialKind::Global => portable.declare_special_raw(name, Value::NIL),
                        SpecialKind::FileLocal => {
                            portable.file_specials.insert(name.clone());
                        }
                        SpecialKind::Lexical => {}
                    }
                }
                portable.seed_file_specials = true;
                let function = portable.compile(form)?;
                portable.source_cache.store = self.source_cache.store.clone();
                let disk_entry = Entry {
                    function,
                    macros: vec![],
                    specials: e.specials.clone(),
                    calls: vec![],
                };
                portable.save_source_ir(&k, 0, 0, &disk_entry)?;
            }
            self.trim_source_cache();
            self.source_cache.entries.insert(k, e);
        }
        Ok(function)
    }
    fn macro_stamp(&self, slot: usize) -> Option<String> {
        if !self.constant_macro(slot) {
            return None;
        }
        let i = self.slots[slot].function.index_if(4)?;
        let f = &self.functions[self.closures[i].function];
        let forms = f
            .code
            .iter()
            .filter_map(|op| {
                if let Op::Const(_, v) = *op {
                    Some(self.source_form(v, 0))
                } else {
                    None
                }
            })
            .collect::<Result<Vec<_>>>()
            .ok()?;
        Some(persistent::form_stamp(&forms))
    }
    fn constant_macro(&self, slot: usize) -> bool {
        if !self.generated_slots.is_empty() {
            return false;
        }
        let Some(i) = self.slots[slot].function.index_if(4) else {
            return false;
        };
        let Some(c) = self.closures.get(i) else {
            return false;
        };
        c.captures.is_empty()
            && self.functions[c.function].lazy.is_none()
            && self.functions[c.function]
                .code
                .iter()
                .all(|op| matches!(op, Op::Const(..) | Op::Move(..) | Op::Return(..)))
    }
    pub(super) fn expand_macro(&mut self, form: &Form) -> Result<Option<Form>> {
        let Form::List(items) = form else {
            return Ok(None);
        };
        let Some(Form::Sym(name)) = items.first() else {
            return Ok(None);
        };
        let Some(&head) = self.names.get(name) else {
            return Ok(None);
        };
        // GNU Fmacroexpand (eval.c:1290-1305) follows symbol aliases to the
        // last symbol whose cell is not a symbol, then (eval.c:1326)
        // `autoload_do_load (def, sym, Qmacro)' loads that symbol's file if
        // its cell is a macro autoload; a function's autoload is left alone
        // and the form compiles as a call.
        let slot = self.macro_alias_end(head);
        if !self.macros.contains(&slot) && self.autoload_macro(self.slots[slot].function) {
            let fundef = self.slots[slot].function;
            let threshold = self.gc_threshold;
            self.gc_threshold = usize::MAX;
            let symbol = Value::reference(slot + 1, 7);
            let macro_only = self.intern("macro");
            let loaded = self.autoload_do_load(fundef, symbol, macro_only);
            self.gc_threshold = threshold;
            if let Err(error) = loaded {
                // Compilation fails here with the load's condition as text;
                // its structured escape belongs to no engine and must not
                // outlive this request (it would replace a later signal).
                return Err(match self.propagated_escape.take() {
                    Some(threaded::Escape::Signal(condition, data)) => {
                        let name = self.display(condition).unwrap_or_default();
                        let data = self.display(data).unwrap_or_default();
                        format!("({name}{})", data.strip_prefix('(').and_then(|d| d.strip_suffix(')')).filter(|d| !d.is_empty()).map(|d| format!(" {d}")).unwrap_or_default())
                    }
                    _ => error,
                });
            }
        }
        // The load may have made the symbol an alias (eval.c:1326-1332 uses
        // the definition autoload_do_load returns, the end of that chain).
        let slot = self.macro_alias_end(head);
        if !self.macros.contains(&slot) {
            return Ok(None);
        }
        if slot != head {
            self.expanded_macros.push((head, self.slots[head].version));
        }
        if self.expanding.len() >= 128 || self.expanded_macros.len() >= 4096 {
            return Err("macro expansion limit exceeded".into());
        }
        self.expanded_macros.push((slot, self.slots[slot].version));
        let function = self.slots[slot].function;
        self.expanding.push(slot);
        // Builder constants are not installed GC roots yet. No macro execution
        // may collect while an outer compiler builder contains unrooted Values.
        let threshold = self.gc_threshold;
        self.gc_threshold = usize::MAX;
        let result = (|| {
            let arguments = items[1..].iter().map(|form| self.literal(form)).collect::<Result<Vec<_>>>()?;
            // R215: in GNU's non-eager regime an expansion runs when the form
            // is reached, so the macro's own condition is what the caller
            // sees. Keep it structured instead of flattening it to text.
            // Eager expansion can also run inside `eval`: its Lisp condition
            // must cross back to the calling engine, not become plain text.
            self.propagate_next = true;
            let value = self.call_macro_expander(function, &arguments);
            self.propagate_next = false;
            let value = match value {
                Err(error) => Err(match &self.propagated_escape {
                    Some(threaded::Escape::Signal(condition, data)) => {
                        let (condition, data) = (*condition, *data);
                        let name = self.display(condition).unwrap_or_default();
                        let data = self.display(data).unwrap_or_default();
                        format!("({name}{})", data.strip_prefix('(').and_then(|d| d.strip_suffix(')')).filter(|d| !d.is_empty()).map(|d| format!(" {d}")).unwrap_or_default())
                    }
                    _ => error,
                }),
                value => value,
            }?;
            // Collection is inhibited until the outer compilation has moved
            // any object of the expansion into a traced constant.
            self.expansion_form(value, 0)
        })();
        self.gc_threshold = threshold;
        self.expanding.pop();
        if let Ok(form) = &result {
            collect_symbols(form, &mut self.expanded_symbols);
        }
        result.map(Some)
    }
}
impl Vm {
    /// Whether a call headed by SLOT expands as a macro now: the end of its
    /// alias chain is a macro or a macro autoload. A cached compilation is
    /// valid only while this is unchanged for every symbol it mentions, so a
    /// call through an alias is recompiled when the alias target, or any
    /// alias in between, becomes a macro.
    fn call_is_macro(&self, slot: usize) -> bool {
        let end = self.macro_alias_end(slot);
        self.macros.contains(&end) || self.autoload_macro(self.slots[end].function)
    }
    /// The last symbol of SLOT's alias chain: GNU Fmacroexpand's `sym' after
    /// its `while (SYMBOLP (def))' walk. A cycle stops where it closes.
    pub(super) fn macro_alias_end(&self, slot: usize) -> usize {
        let mut slot = slot;
        for _ in 0..self.slots.len() {
            let f = self.slots[slot].function;
            if f.tag() != 7 || f == Value::NIL || f == Value::UNBOUND || self.macros.contains(&slot) {
                break;
            }
            slot = f.payload() as usize - 1;
        }
        slot
    }
}
fn contains_local_object(form: &Form) -> bool {
    match form {
        Form::Object(_) => true,
        Form::List(items) | Form::Vector(items) => items.iter().any(contains_local_object),
        Form::Dotted(items, tail) => {
            items.iter().any(contains_local_object) || contains_local_object(tail)
        }
        _ => false,
    }
}
fn collect_symbols(f: &Form, out: &mut NameSet<String>) {
    match f {
        Form::Sym(s) => {
            out.insert(s.clone());
        }
        Form::List(a) | Form::Vector(a) => {
            for f in a {
                collect_symbols(f, out);
            }
        }
        Form::Dotted(a, t) => {
            for f in a {
                collect_symbols(f, out);
            }
            collect_symbols(t, out);
        }
        _ => {}
    }
}
