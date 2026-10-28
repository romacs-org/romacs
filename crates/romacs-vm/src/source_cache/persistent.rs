//! Portable IR images: symbolic slots, relative functions/caches, ordinary
//! literal Forms. No process addresses or private heap indices cross sessions.
use super::*;
mod codec;
pub(super) fn form_stamp(forms: &[Form]) -> String {
    serde_json::Value::Array(forms.iter().map(codec::form_json).collect()).to_string()
}
struct Artifact {
    compiler: String,
    source_key: String,
    symbols: Vec<String>,
    specials: Vec<(String, SpecialKind)>,
    functions: Vec<Function>,
    constants: Vec<(usize, usize, Form)>,
    entry: usize,
    caches: usize,
}
impl Vm {
    pub(super) fn save_source_ir(
        &self,
        k: &str,
        first: usize,
        first_cache: usize,
        e: &Entry,
    ) -> Result<()> {
        let Some(store) = &self.source_cache.store else {
            return Ok(());
        };
        // Compiler identities materialized by macro execution are VM-local, not
        // portable textual slots. Keep this world on source-backed compilation.
        if !e.macros.is_empty() || !self.generated_slots.is_empty() {
            return Ok(());
        }
        let mut functions: Vec<_> = self.functions[first..]
            .iter()
            .map(|f| (**f).clone())
            .collect();
        let mut constants = vec![];
        for (fi, f) in functions.iter_mut().enumerate() {
            for (pc, op) in f.code.iter_mut().enumerate() {
                match op {
                    Op::Const(_, v) | Op::Condition(_, v, _) => {
                        constants.push((fi, pc, self.source_form(*v, 0)?));
                        *v = Value::NIL;
                    }
                    Op::Closure(_, id, _) => {
                        *id = id.checked_sub(first).ok_or("nonlocal IR function")?
                    }
                    Op::Direct(_, _, cache, _, _) | Op::Prepared(_, _, _, cache, _, _) => {
                        *cache = cache.checked_sub(first_cache).ok_or("nonlocal IR cache")?
                    }
                    _ => {}
                }
            }
        }
        let artifact = Artifact {
            compiler: COMPILER.into(),
            source_key: k.into(),
            symbols: self.symbol_names.clone(),
            specials: e.specials.clone(),
            functions,
            constants,
            entry: e.function - first,
            caches: self.caches.len() - first_cache,
        };
        let data = artifact.encode();
        let mut builder = ImageBuilder::default();
        builder.string_raw(&data);
        let digest: [u8; 32] = Sha256::digest(data.as_bytes()).into();
        let hash = builder.publish(store, digest)?;
        let index = store.join(format!("source-{k}"));
        let temporary = store.join(format!(".source-{k}-{}", crate::host::pid()));
        crate::host::write(&temporary, hash)
            .and_then(|_| crate::host::rename(&temporary, index))
            .map_err(|e| e.to_string())
    }
    pub(super) fn load_source_ir(&mut self, k: &str, form: &Form) -> Result<Option<usize>> {
        if !self.generated_slots.is_empty() {
            return Ok(None);
        }
        let Some(store) = &self.source_cache.store else {
            return Ok(None);
        };
        let Ok(hash) = crate::host::read_to_string(store.join(format!("source-{k}"))) else {
            return Ok(None);
        };
        // SAFETY: immutable-inode image store; same deployment contract as --image.
        let Ok(image) = (unsafe { ImageRegion::open(store, &hash) }) else {
            return Ok(None);
        };
        let Ok(v) = image.string_reference_raw(0) else {
            return Ok(None);
        };
        let mut reader = Vm::new();
        reader.attach_image(image)?;
        let Ok(data) = reader.string_value(v) else {
            return Ok(None);
        };
        let Some(mut a) = Artifact::decode(data) else {
            return Ok(None);
        };
        if a.compiler != COMPILER
            || a.source_key != k
            || a.specials
                .iter()
                .any(|(s, v)| self.special_kind(s) != *v)
        {
            return Ok(None);
        }
        let mut names = NameSet::default();
        collect_symbols(form, &mut names);
        if names
            .iter()
            .any(|s| self.names.get(s).is_some_and(|s| self.macros.contains(s)))
        {
            return Ok(None);
        }
        if !valid_artifact(&a) {
            return Ok(None);
        }
        let slots: Vec<_> = a.symbols.iter().map(|s| self.slot(s)).collect();
        // Allocate all literal values before installing any code. Failure cannot
        // leave half-installed executable functions or invalid cache indices.
        for (f, pc, form) in a.constants {
            let v = self.literal(&form)?;
            match &mut a.functions[f].code[pc] {
                Op::Const(_, x) | Op::Condition(_, x, _) => *x = v,
                _ => unreachable!(),
            }
        }
        let (ids, cache_ids) = self.reserve_request_artifact(&a.functions, a.caches);
        for f in &mut a.functions {
            for op in &mut f.code {
                match op {
                    Op::DeclareSpecial(s)
                    | Op::Global(_, s)
                    | Op::ResolveHead(_, s)
                    | Op::SetGlobal(s, _)
                    | Op::Bind(s, _)
                    | Op::Define(s, _)
                    | Op::DefineMacro(s, _) => *s = slots[*s],
                    Op::Direct(_, s, c, _, _) | Op::Prepared(_, _, s, c, _, _) => {
                        *s = slots[*s];
                        *c = cache_ids[*c];
                    }
                    Op::Closure(_, id, _) => *id = ids[*id],
                    _ => {}
                }
            }
        }
        let function = ids[a.entry];
        let mut calls: Vec<_> = a
            .specials
            .iter()
            .map(|(name, _)| (self.slot(name), false))
            .collect();
        calls.extend(a.functions.iter().flat_map(|f| &f.code).filter_map(|op| {
            if let Op::Direct(_, s, _, _, _) | Op::Prepared(_, _, s, _, _, _) = *op {
                Some((s, false))
            } else {
                None
            }
        }));
        calls.sort_unstable();
        calls.dedup();
        for (id, function) in ids.into_iter().zip(a.functions) {
            self.functions[id] = Arc::new(function);
        }
        self.trim_source_cache();
        self.source_cache.entries.insert(
            k.into(),
            Entry {
                function,
                macros: vec![],
                specials: a.specials,
                calls,
            },
        );
        self.source_cache.stats.disk_hits += 1;
        Ok(Some(function))
    }
}
fn valid_artifact(a: &Artifact) -> bool {
    if a.functions.is_empty() || a.entry >= a.functions.len() || a.caches > 1_000_000 {
        return false;
    }
    let mut constants = HashSet::default();
    for (f, pc, _) in &a.constants {
        if *f >= a.functions.len()
            || *pc >= a.functions[*f].code.len()
            || !matches!(a.functions[*f].code[*pc], Op::Const(..) | Op::Condition(..))
            || !constants.insert((*f, *pc))
        {
            return false;
        }
    }
    for (fi, f) in a.functions.iter().enumerate() {
        if f.regs == 0
            || f.regs > 65536
            || f.captures + f.arity + usize::from(f.rest) > f.regs
            || f.required > f.arity
            || !matches!(f.code.last(), Some(Op::Return(_)))
        {
            return false;
        }
        let r = |r| r < f.regs;
        let s = |s| s < a.symbols.len();
        let jump = |t| t < f.code.len();
        for (pc, op) in f.code.iter().enumerate() {
            let valid = match *op {
                Op::Const(d, v) => r(d) && v == Value::NIL && constants.contains(&(fi, pc)),
                Op::Move(d, x) | Op::ReadCell(d, x) | Op::WriteCell(d, x) => r(d) && r(x),
                Op::Cell(d) | Op::Return(d) => r(d),
                Op::Global(d, x) | Op::ResolveHead(d, x) => r(d) && s(x),
                Op::Prepared(d, t, x, c, start, n) => r(d) && r(t) && s(x) && c < a.caches && n <= 4096 && start.checked_add(n).is_some_and(|end| end <= f.regs),
                Op::SetGlobal(x, d) | Op::Bind(x, d) | Op::Define(x, d) | Op::DefineMacro(x, d) => {
                    s(x) && r(d)
                }
                Op::Unbind(n) | Op::PopHandlers(n) => n <= f.code.len(),
                Op::Protect(d, cleanup, end) => r(d) && jump(cleanup) && jump(end),
                Op::LeaveProtect(d) => r(d),
                Op::ResumeExit => true,
                Op::DeclareSpecial(s) => s < a.symbols.len(),
                Op::Jump(t) => jump(t),
                Op::Unless(d, t) => r(d) && jump(t),
                Op::Catch(d, x, t) => r(d) && r(x) && jump(t),
                Op::Condition(d, v, t) => {
                    r(d) && jump(t) && v == Value::NIL && constants.contains(&(fi, pc))
                }
                Op::Closure(d, id, site) => {
                    r(d) && id < a.functions.len()
                        && site < f.sites.len()
                        && f.sites[site].len() == a.functions[id].captures
                        && f.sites[site].iter().all(|x| r(*x))
                }
                Op::Call(d, x, start, n) => {
                    r(d) && r(x)
                        && n <= 4096
                        && start.checked_add(n).is_some_and(|end| end <= f.regs)
                }
                Op::Direct(d, x, c, start, n) => {
                    r(d) && s(x)
                        && c < a.caches
                        && n <= 4096
                        && start.checked_add(n).is_some_and(|end| end <= f.regs)
                }
            };
            if !valid {
                return false;
            }
        }
    }
    let f = &a.functions[a.entry];
    f.captures == 0 && f.arity == 0 && !f.rest
}
