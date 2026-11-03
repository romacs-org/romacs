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
    /// `(lambda ...)' lists called as functions compiled (`lambda_list_closure').
    pub lambda_compiles: u64,
}
romacs_snapshot::snapshot_struct!(SourceCacheStats { hits, compilations, invalidations, disk_hits, direct, lambda_compiles });
#[derive(Clone)]
struct Entry {
    function: usize,
    macros: Vec<(usize, u64, String)>,
    specials: Vec<(String, SpecialKind)>,
    calls: Vec<(usize, bool)>,
}
romacs_snapshot::snapshot_struct!(Entry { function, macros, specials, calls });
/// A `(lambda ...)' list's compiled closure (`Vm::lambda_list_closure'):
/// the list as it was compiled (`lambda_snapshot'), and the world epoch it
/// was compiled in.
pub(super) struct LambdaList {
    snapshot: Vec<LambdaNode>,
    closure: Value,
    world: u64,
}
/// One mutable object of a lambda list, in the walk's order, as it was:
/// a cons with its car and cdr, a string with its text, a vector with its
/// elements. Children are held by identity, so replacing one, even with an
/// `equal' copy, is a change, as it is for GNU's interpreter, which
/// evaluates `(quote X)' to that X.
enum LambdaNode {
    Cons(Value, Value, Value),
    Text(Value, Vec<u8>, bool),
    Vector(Value, Vec<Value>),
}
// SAFETY: at rest in a world image (`world_quiescent' requires the map
// empty); its Values are dropped by every collection before they move.
unsafe impl romacs_snapshot::SnapshotSafe for LambdaList {}
/// Conses, strings and vectors a lambda list's snapshot holds at most; a
/// larger (or circular) list is compiled at each call.
const LAMBDA_WALK_LIMIT: usize = 4096;
#[derive(Default)]
pub(super) struct SourceCache {
    entries: NameMap<String, Entry>,
    stats: SourceCacheStats,
    store: Option<PathBuf>,
    prepared: Vec<prepared::Prepared>,
    last_guard: Option<Entry>,
}
romacs_snapshot::snapshot_struct!(SourceCache { entries, stats, store, prepared, last_guard });
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
                    for file in crate::host::read_dir(store).map_err(|e| e.to_string())? {
                        let file = file.map_err(|e| e.to_string())?;
                        if file
                            .file_name()
                            .to_str()
                            .is_some_and(|s| s.starts_with("source-") && s.len() == 71)
                        {
                            crate::host::remove_file(file.path()).map_err(|e| e.to_string())?;
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
        crate::host::create_dir_all(&path).map_err(|e| e.to_string())?;
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
    /// Compile FORM as request code with the guard that says when a cached
    /// copy of it is stale: the macros it expanded, the special-ness of the
    /// symbols it names, and which of its calls are macro calls.
    fn compile_guarded(&mut self, form: &Form) -> Result<(usize, Vec<usize>, Entry)> {
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
                    if let Op::Direct(_, s, _, _, _) = *op {
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
        Ok((function, compiled, e))
    }
    /// eval.c funcall_lambda's reading of a `(lambda ARGS . BODY)' list
    /// called as a function: the closure evaluating LIST makes in GNU's
    /// dynamic dialect (`eval_primitive'). It is reused while LIST is still
    /// the text it was compiled from (`lambda_unchanged') and no function cell
    /// has changed since (`world_version', which every `defun', `defmacro',
    /// `defalias' and `fset' advances): its macros, its inner lambdas' too,
    /// expand once per definition, as GNU expands a closure's, not on every
    /// call as GNU's interpreter does with a list (DIVERGENCES.md
    /// VM2-LAMBDA-LIST-EXPAND). In that dialect every binding is dynamic,
    /// so special declarations change nothing it compiled.
    pub(super) fn lambda_list_closure(&mut self, list: Value) -> Result<Value> {
        if let Some(entry) = self.lambda_lists.get(&list.0)
            && entry.world == self.world_version
            && self.lambda_unchanged(list, &entry.snapshot)
        {
            return Ok(entry.closure);
        }
        // Compiling runs macro expanders, which may collect: LIST is the key
        // and the snapshot's source after it.
        let frame = self.root_frame(&[list]);
        let result = self.eval_primitive(&[list, Value::NIL]);
        let [list] = self.release_frame(frame);
        let closure = result?;
        self.source_cache.stats.lambda_compiles += 1;
        // After the compile: a definition an expander made is part of what
        // this code was compiled against, not a later change.
        let world = self.world_version;
        // Every collection empties the map, so it holds only lists called
        // since the last one.
        if let Some(snapshot) = self.lambda_snapshot(list) {
            self.lambda_lists.insert(list.0, LambdaList { snapshot, closure, world });
        }
        Ok(closure)
    }
    /// Whether LIST is still as SNAPSHOT (`lambda_snapshot') recorded it:
    /// the same walk, compared object by object as it goes.
    fn lambda_unchanged(&self, list: Value, snapshot: &[LambdaNode]) -> bool {
        let mut recorded = snapshot.iter();
        let mut pending = vec![list];
        while let Some(value) = pending.pop() {
            if value == Value::NIL || value == Value::TRUE {
                continue;
            }
            let same = match value.tag() {
                3 => match (self.cons_pair(value), recorded.next()) {
                    (Ok((car, cdr)), Some(LambdaNode::Cons(node, was_car, was_cdr))) => {
                        pending.push(cdr);
                        pending.push(car);
                        (*node, *was_car, *was_cdr) == (value, car, cdr)
                    }
                    _ => false,
                },
                0 => match self.object_if(value) {
                    Some(data::Object::String(text) | data::Object::RawString(text)) => matches!(recorded.next(),
                        Some(LambdaNode::Text(node, bytes, multibyte))
                            if *node == value && bytes.as_slice() == text.bytes() && *multibyte == text.is_multibyte()),
                    Some(data::Object::Vector(items)) => match recorded.next() {
                        Some(LambdaNode::Vector(node, was)) if *node == value && was == items => {
                            pending.extend(items.iter().rev().copied());
                            true
                        }
                        _ => false,
                    },
                    _ => true,
                },
                _ => true,
            };
            if !same {
                return false;
            }
        }
        recorded.next().is_none()
    }
    /// LIST's mutable objects as they are now, depth first: what a later
    /// call compares to decide the list is unchanged. None past
    /// `LAMBDA_WALK_LIMIT' objects, which a circular list reaches.
    fn lambda_snapshot(&self, list: Value) -> Option<Vec<LambdaNode>> {
        let mut nodes = Vec::new();
        let mut pending = vec![list];
        while let Some(value) = pending.pop() {
            if nodes.len() == LAMBDA_WALK_LIMIT {
                return None;
            }
            if value == Value::NIL || value == Value::TRUE {
                continue;
            }
            match value.tag() {
                3 => {
                    let (car, cdr) = self.cons_pair(value).ok()?;
                    nodes.push(LambdaNode::Cons(value, car, cdr));
                    pending.push(cdr);
                    pending.push(car);
                }
                0 => match self.object_if(value) {
                    Some(data::Object::String(text) | data::Object::RawString(text)) => {
                        nodes.push(LambdaNode::Text(value, text.bytes().to_vec(), text.is_multibyte()));
                    }
                    Some(data::Object::Vector(items)) => {
                        nodes.push(LambdaNode::Vector(value, items.clone()));
                        pending.extend(items.iter().rev().copied());
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Some(nodes)
    }
    /// Code for FORM, a datum evaluated again and again (a mode-line
    /// `:eval' form): compiled once and cached by its structure while its
    /// guard holds. GNU evaluates the same object each time, so its
    /// constants are shared across evaluations as GNU's are. A datum
    /// holding an uninterned symbol or an object with no spelling has no
    /// structural key (the stamp writes neither): None.
    pub(super) fn cached_datum(&mut self, form: &Form) -> Result<Option<usize>> {
        if contains_identity(form) {
            return Ok(None);
        }
        // NUL never starts source text, so datum keys stay apart from it.
        let source = format!("\0datum\0{}", persistent::form_stamp(std::slice::from_ref(form)));
        if let Some(f) = self.cached_submission(&source) {
            return Ok(Some(f));
        }
        let (function, _, e) = self.compile_guarded(form)?;
        self.trim_source_cache();
        self.source_cache.entries.insert(key(&source), e);
        Ok(Some(function))
    }
    pub(super) fn cached_source_form(&mut self, source: &str, form: &Form) -> Result<usize> {
        if let Some(f) = self.cached_submission(source) {
            return Ok(f);
        }
        let k = key(source);
        if let Some(f) = self.load_source_ir(&k, form)? {
            return Ok(f);
        }
        let (function, compiled, e) = self.compile_guarded(form)?;
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
            let symbol = Value::reference(slot + 1, 7);
            let macro_only = self.intern("macro");
            let loaded = self.autoload_do_load(fundef, symbol, macro_only);
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
        // Inside a compilation, the open builder holds collection while it
        // keeps unrooted constants. At a load's top level nothing does: ITEMS
        // are forms, whose objects are rooted, and the expander roots
        // FUNCTION and the arguments.
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
            // Converting the expansion runs no Lisp; its objects are rooted.
            self.expansion_form(value, 0)
        })();
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
/// Whether FORM names something by identity rather than by spelling: an
/// uninterned symbol or an object with no source spelling.
fn contains_identity(form: &Form) -> bool {
    match form {
        Form::Object(_) | Form::Generated(_) => true,
        Form::List(items) | Form::Vector(items) => items.iter().any(contains_identity),
        Form::Dotted(items, tail) => items.iter().any(contains_identity) || contains_identity(tail),
        _ => false,
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
