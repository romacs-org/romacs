//! Variable designation and per-buffer values, owned by the symbol's slot
//! (`Slot::local`), never by its Lisp-visible plist.

#[cfg(test)]
mod uninitialized_image_tests {
    use crate::*;

    #[test]
    fn special_symbol_identity_survives_code_image() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq uv-image-symbol (make-symbol \"uv-image-twin\")) (internal--define-uninitialized-variable uv-image-symbol) (internal--define-uninitialized-variable 'uv-image-interned)").unwrap();
        let entry = vm.compile(&Form::Int(0)).unwrap();
        let store = std::env::temp_dir().join(format!("uv-image-{}", std::process::id()));
        let key = vm.publish_code_image(&store, entry, [0; 32]).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        let (mut loaded, _) = Vm::from_code_image(image, [0; 32]).unwrap();
        let result = loaded.eval_source_raw("(list (special-variable-p uv-image-symbol) (special-variable-p 'uv-image-twin) (special-variable-p 'uv-image-interned))").unwrap();
        assert_eq!(loaded.display(result).unwrap(), "(t nil t)", "code image must preserve declaration identity");
        std::fs::remove_dir_all(store).unwrap();
    }
}

#[cfg(test)]
mod reclamation_tests {
    use super::*;

    #[test]
    fn unrelated_global_reads_do_not_visit_symbols_with_local_bindings() {
        let mut vm = Vm::new();
        for name in ["make-local-variable", "make-variable-buffer-local", "set-buffer", "get-buffer-create"] {
            assert_eq!(vm.eval_source_raw(&format!("(fboundp '{name})")).unwrap(), Value::TRUE, "missing {name}");
        }
        vm.eval_source_raw("(defvar redirect-global 17) (defvar redirect-local 1) (defvar redirect-auto 2)").unwrap();
        assert_eq!(vm.eval_source_raw("(let ((redirect-global 19)) redirect-global)").unwrap(), Value::integer(19).unwrap());
        assert_eq!(vm.eval_source_raw("redirect-global").unwrap(), Value::integer(17).unwrap());
        vm.eval_source_raw("(set-buffer (get-buffer-create \"redirect-A\")) (make-local-variable 'redirect-local) (setq redirect-local 5) (make-variable-buffer-local 'redirect-auto) (setq redirect-auto 6)").unwrap();
        let direct = vm.source_cache_stats().direct;
        assert_eq!(vm.eval_source_raw("(+ redirect-local 0)").unwrap(), Value::integer(5).unwrap(), "direct localized read");
        assert_eq!(vm.source_cache_stats().direct, direct + 1);
        assert_eq!(vm.eval_source_raw("(let ((redirect-local 9)) redirect-local)").unwrap(), Value::integer(9).unwrap());
        assert_eq!(vm.eval_source_raw("redirect-local").unwrap(), Value::integer(5).unwrap());
        vm.eval_source_raw("(set-buffer (get-buffer-create \"redirect-B\"))").unwrap();
        assert_eq!(vm.eval_source_raw("redirect-local").unwrap(), Value::integer(1).unwrap(), "other buffer inherits default");
        assert_eq!(vm.eval_source_raw("(let ((redirect-local 8)) (set-buffer (get-buffer-create \"redirect-A\")) redirect-local)").unwrap(), Value::integer(5).unwrap(), "localized default binding is buffer-qualified");
        assert_eq!(vm.eval_source_raw("redirect-local").unwrap(), Value::integer(5).unwrap());
        vm.eval_source_raw("(set-buffer (get-buffer-create \"redirect-B\"))").unwrap();
        assert_eq!(vm.eval_source_raw("redirect-local").unwrap(), Value::integer(1).unwrap(), "default restored");
        assert_eq!(vm.eval_source_raw("redirect-auto").unwrap(), Value::integer(2).unwrap(), "automatic local leaves the other buffer default alone");
        vm.eval_source_raw("(set-buffer (get-buffer-create \"redirect-A\"))").unwrap();
        assert_eq!(vm.eval_source_raw("redirect-auto").unwrap(), Value::integer(6).unwrap());
        // The per-buffer table is held by the symbol's value redirect, as in
        // GNU (SYMBOL_LOCALIZED), never on its Lisp plist. The plist used to
        // hold it under a name-keyed slot Lisp could overwrite; now no plist
        // operation can reach it: symbol-plist does not show it, and
        // replacing or clearing the plist keeps both the table and the
        // set-only redirect.
        let local = vm.slot("redirect-local");
        let redirect = vm.variable_redirects[local];
        assert_eq!(redirect, Some(VariableRedirect::Localized));
        assert_eq!(vm.eval_source_raw("(symbol-plist 'redirect-local)").unwrap(), Value::NIL, "local table hidden from the plist");
        vm.eval_source_raw("(setplist 'redirect-local (list 'note 1))").unwrap();
        assert_eq!(vm.variable_value(local).unwrap(), Value::integer(5).unwrap(), "plist replacement keeps the local value");
        vm.eval_source_raw("(setplist 'redirect-local nil)").unwrap();
        assert_eq!(vm.variable_value(local).unwrap(), Value::integer(5).unwrap(), "plist clearing keeps the local value");
        assert_eq!(vm.variable_redirects[local], redirect, "clearing a plist cannot clear its redirect");
        // A plist populated before the symbol is ever a variable does not
        // make it localized, and later localization still works.
        vm.eval_source_raw("(put 'redirect-late-owner 'note 1)").unwrap();
        vm.declare_special_raw("redirect-late-owner", Value::integer(4).unwrap());
        let late = vm.slot("redirect-late-owner");
        assert_eq!(vm.variable_value(late).unwrap(), Value::integer(4).unwrap(), "a plist alone is not a local table");
        vm.eval_source_raw("(make-local-variable 'redirect-late-owner) (setq redirect-late-owner 6)").unwrap();
        assert_eq!(vm.variable_value(late).unwrap(), Value::integer(6).unwrap(), "plist populated before localization");
        assert_eq!(vm.eval_source_raw("(get 'redirect-late-owner 'note)").unwrap(), Value::integer(1).unwrap(), "localization keeps the plist");
        let global = vm.slot("redirect-global");
        let mut probes = Vec::new();
        for count in [0, 100, 2_000, 20_000] {
            for i in vm.names.len()..count { vm.slot(&format!("redirect-padding-{i}")); }
            vm.variable_read_probes.set(0);
            for _ in 0..32 {
                assert_eq!(vm.variable_value(global).unwrap(), Value::integer(17).unwrap());
            }
            probes.push((count, vm.variable_read_probes.get()));
        }
        eprintln!("global read symbol visits: {probes:?}");
        assert_eq!(probes.iter().map(|(_, probes)| probes).sum::<usize>(), 0, "unrelated global reads must not inspect symbols at any symbol count: {probes:?}");
    }

    #[test]
    fn image_intrinsic_plists_acquire_local_redirects() {
        let store = std::env::temp_dir().join(format!("vm2-local-redirect-{}", std::process::id()));
        let mut source = Vm::new();
        for name in ["put", "make-variable-buffer-local", "local-variable-if-set-p"] {
            assert_eq!(source.eval_source_raw(&format!("(fboundp '{name})")).unwrap(), Value::TRUE);
        }
        source.eval_source_raw("(put 'print-circle 'redirect-note 1) (put 'truncate-lines 'redirect-note 1)").unwrap();
        let entry = source.compile(&Form::Int(0)).unwrap();
        let key = source.publish_code_image(&store, entry, [0; 32]).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        let (mut vm, _) = Vm::from_code_image(image, [0; 32]).unwrap();
        vm.eval_source_raw("(make-variable-buffer-local 'print-circle)").unwrap();
        assert_eq!(vm.eval_source_raw("(local-variable-if-set-p 'print-circle)").unwrap(), Value::TRUE, "image intrinsic localization");
        vm.eval_source_raw("(setq truncate-lines t)").unwrap();
        assert_eq!(vm.eval_source_raw("(local-variable-if-set-p 'truncate-lines)").unwrap(), Value::TRUE, "image lazy intrinsic localization");
        std::fs::remove_dir_all(store).unwrap();
    }

    #[test]
    fn dead_buffer_private_slots_are_reused_after_counted_collection() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let mut counts = Vec::new();
        for _ in 0..3 {
            vm.eval_source_raw("(dotimes (i 1000) (with-temp-buffer (make-local-variable 's124-pool)))").unwrap();
            let slot = vm.slot("s124-pool");
            let redirect = vm.variable_redirects[slot];
            assert_eq!(redirect, Some(VariableRedirect::Localized));
            let before = vm.heap_stats().collections;
            vm.collect(&mut []);
            assert!(vm.heap_stats().collections > before);
            assert!(!vm.free_local_binding_slots.is_empty(), "fixture must reclaim local owners");
            assert_eq!(vm.variable_redirects[slot], redirect, "owner reclamation must retain the symbol redirect");
            assert!(vm.local_table(slot).unwrap().is_some(), "empty localized table remains installed");
            counts.push(vm.local_binding_slots.len());
        }
        assert!(counts[2] <= counts[0] + 2, "dead-buffer private slot metadata grows: {counts:?}");
    }
}
use super::*;
impl Vm {
    /// GNU eval.c:747: search oldest-first for this exact symbol, without
    /// following aliases. Local bindings are excluded; localized ordinary
    /// variables use private slots, while forwarded buffer locals need the
    /// explicit LET_DEFAULT flag. Saved values are already traced by `dynamic`.
    pub(super) fn default_toplevel_binding(&mut self, slot: usize) -> Option<usize> {
        let buffer_forwarded = matches!(self.forwarding(slot), Some(forwarded::Fwd::Buffer(_)));
        self.dynamic.iter().position(|(bound, _, _, let_default)| {
            *bound == slot && (!buffer_forwarded || *let_default)
        })
    }

    fn allocate_local_binding_slot(&mut self) -> usize {
        if let Some(slot) = self.free_local_binding_slots.pop() {
            return slot;
        }
        // Internal slot IDs must not be interned public symbols: otherwise a
        // Lisp reference could outlive the owner and alias a recycled binding.
        let slot = self.slots.len();
        self.symbol_names.push(format!("#:vm2-local-binding:{slot}"));
        self.slots.push(Slot::new());
        self.local_binding_slots.insert(slot);
        slot
    }
    // Shared with the assignment engine; this is its existing protection set.
    pub(super) fn protected_variable_slot(&self, slot: usize) -> bool {
        let base = self.indirect_variable(slot);
        // An alias inherits its base's constness (GNU copies trapped_write,
        // eval.c:720), including nil, t and keywords as bases.
        base == self.multibyte_slot
            || self.fixnum_slots.contains(&base)
            || (base != slot && (matches!(self.symbol_names[base].as_str(), "nil" | "t") || self.symbol_names[base].starts_with(':')))
    }
    /// Follow GNU's variable-alias chain (`indirect_variable`, data.c:1291) to
    /// the variable that actually holds the value. `defvaralias` refuses
    /// cycles, so the walk ends; the bound is defence, not semantics.
    pub(super) fn indirect_variable(&self, mut slot: usize) -> usize {
        for _ in 0..self.variable_redirects.len() {
            match self.variable_redirects.get(slot) {
                Some(Some(VariableRedirect::Alias(base))) => slot = *base,
                _ => break,
            }
        }
        slot
    }
    /// GNU `Fdefvaralias` (eval.c:631-726) after its checks: both variables
    /// become special and NEW redirects to BASE. Interning BASE's or NEW's
    /// plist later must not demote the alias to a plist redirect.
    pub(super) fn make_variable_alias(&mut self, new: usize, base: usize) {
        for slot in [new, base] {
            let name = self.symbol_names[slot].clone();
            self.specials.insert(name);
        }
        self.variable_redirects.resize(self.variable_redirects.len().max(new.max(base) + 1), None);
        self.variable_redirects[new] = Some(VariableRedirect::Alias(base));
    }
    // Redirects contain stable slot IDs, never untraced moving-heap Values.
    // Publication is set-only: GNU data.c:2385-2408 removes a local entry
    // without reverting SYMBOL_LOCALIZED. GC owner reclamation must likewise
    // never clear this redirect; future kill-local-variable must preserve it.
    // The local table lives in the slot itself (`Slot::local`), never on the
    // Lisp plist, so no plist operation can install or remove it.
    pub(super) fn initialize_variable_redirect(&mut self, name: &str, slot: usize) {
        self.variable_redirects.resize(self.variable_redirects.len().max(slot + 1), None);
        if self.variable_redirects[slot].is_none() && self.names.get(name) == Some(&slot) {
            if let Some(fwd) = self.forwarded.lookup(name) {
                self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(fwd));
            }
        }
        if self.variable_redirects[slot].is_none() {
            let localized = self.slots[slot].local != Value::NIL;
            self.variable_redirects[slot] = Some(if localized { VariableRedirect::Localized } else { VariableRedirect::Plain });
        }
    }
    /// Publish SLOT's local TABLE, retaining native Boolean forwarding.
    pub(super) fn install_local_table(&mut self, slot: usize, table: Value, explicit: bool) {
        self.slots[slot].local = table;
        self.slots[slot].explicit_local = explicit;
        self.variable_redirects.resize(self.variable_redirects.len().max(slot + 1), None);
        if let Some(VariableRedirect::Forwarded(fwd @ forwarded::Fwd::Bool(_))) = self.variable_redirects[slot] {
            self.variable_redirects[slot] = Some(VariableRedirect::LocalizedForwarded(fwd));
        } else if !matches!(self.variable_redirects[slot], Some(VariableRedirect::Forwarded(_) | VariableRedirect::LocalizedForwarded(_) | VariableRedirect::Alias(_))) {
            self.variable_redirects[slot] = Some(VariableRedirect::Localized);
        }
    }
    pub(super) fn local_table(&mut self, slot: usize) -> Result<Option<Value>> {
        if self.variable_redirects.get(slot).copied().flatten().is_none() {
            // Code images rebuild slots without interning; uninterned slots
            // are never forwarded. Resolve the redirect once.
            let name = self.symbol_names[slot].clone();
            if self.names.get(&name) != Some(&slot) {
                let table = self.slots[slot].local;
                return Ok((table != Value::NIL).then_some(table));
            }
            self.initialize_variable_redirect(&name, slot);
        }
        if !matches!(self.variable_redirects[slot], Some(VariableRedirect::Localized | VariableRedirect::LocalizedForwarded(_))) { return Ok(None); }
        let table = self.slots[slot].local;
        Ok((table != Value::NIL).then_some(table))
    }
    // A table maps each buffer to an ordinary private symbol slot. Dynamic
    // bindings save that slot, not the default or another buffer's value.
    // A missing entry inherits the default; UNBOUND in an entry is locally void.
    pub(super) fn variable_binding_slot(&mut self, slot: usize, create: bool) -> Result<usize> {
        // An alias has no binding of its own: GNU follows SYMBOL_VARALIAS in
        // every reader and writer (data.c:1629, 1719, ...).
        let slot = self.indirect_variable(slot);
        // Built-in automatic locals stay heap-free until first lookup, as
        // required by cold VM/code-image construction.
        if (slot == self.truncate_lines_slot || self.names.get("case-symbols-as-words") == Some(&slot)) && self.local_table(slot)?.is_none() {
            let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
            self.install_local_table(slot, table, false);
        }
        let Some(table) = self.local_table(slot)? else { return Ok(slot) };
        self.buffer_index()?;
        let buffer = self.slots[self.buffer_slot].value;
        if let Some(symbol) = self.declaration_hash_get(table, buffer)? { return Ok(symbol.index(7)? - 1); }
        let explicit = self.slots[slot].explicit_local;
        // Assignment within a default binding belongs to that binding in
        // its originating buffer, not to a newly created local entry.
        let shadowed = self.dynamic.iter().rev().any(|(bound, _, owner, localized)| {
            *localized && *bound == slot && (*owner == buffer || (*owner == Value::NIL && self.names.get("#:vm2-buffer:*scratch*").is_some_and(|i| self.slots[*i].value == buffer)))
        });
        if !create || explicit || shadowed { return Ok(slot); }
        let local = self.allocate_local_binding_slot();
        self.slots[local].value = if let Some(fwd) = self.localized_boolean(slot) { self.forwarded_value(fwd)? } else { self.slots[slot].value };
        self.declaration_hash_put(table, buffer, Value::reference(local+1, 7))?;
        Ok(local)
    }
    pub(super) fn variable_value(&mut self, slot: usize) -> Result<Value> {
        let base = self.indirect_variable(slot);
        if let Some(fwd) = self.native_forwarding(base) {
            let local = self.variable_binding_slot(base, false)?;
            return if local == base { self.forwarded_value(fwd) } else { Ok(self.slots[local].value) };
        }
        let local = self.variable_binding_slot(slot, false)?;
        Ok(self.slots[local].value)
    }
    pub(super) fn set_variable_value(&mut self, slot: usize, value: Value) -> Result<()> {
        if !self.trapped.is_empty() {
            if value == Value::UNBOUND { self.notify_watchers_at(slot, value, "makunbound", Some(Value::NIL))?; }
            else { self.notify_watchers(slot, value, "set")?; }
        }
        let base = self.indirect_variable(slot);
        if let Some(fwd) = self.native_forwarding(base) {
            if value == Value::UNBOUND {
                return self.set_forwarded(base, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text);
            }
            let local = self.variable_binding_slot(base, true)?;
            if local == base {
                return self.set_forwarded(base, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text);
            }
            self.slots[local].value = if value == Value::NIL { Value::NIL } else { Value::TRUE };
            return Ok(());
        }
        let local = self.variable_binding_slot(slot, true)?;
        self.slots[local].value = value;
        Ok(())
    }
    pub(super) fn promote_boolean_local(&mut self, slot: usize, explicit: bool) -> Result<()> {
        if self.dynamic.iter().any(|(bound, old, _, _)| *bound == slot && *old != Value::NIL && *old != Value::TRUE) {
            return Err(format!("VM2 localized Boolean default requires nil or t: {}", self.symbol_names[slot]));
        }
        if self.local_table(slot)?.is_none() {
            let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
            self.install_local_table(slot, table, explicit);
        }
        Ok(())
    }
    pub(super) fn bind_variable(&mut self, slot: usize, value: Value) -> Result<()> {
        if !self.trapped.is_empty() { self.notify_watchers(slot, value, "let")?; }
        let base = self.indirect_variable(slot);
        let local = self.variable_binding_slot(base, false)?;
        if let Some(fwd) = self.native_forwarding(base) && local == base {
            return self.bind_forwarded(base, fwd, value).map_err(forwarded::fault_text);
        }
        let localized = self.local_table(base)?.is_some();
        if base != self.buffer_slot {
            self.backtrace.bind(self.dynamic.len(), Value::reference(base + 1, 7));
        }
        self.dynamic.push((local, self.slots[local].value, self.slots[self.buffer_slot].value, localized));
        self.slots[local].value = if self.localized_boolean(base).is_some() { if value == Value::NIL { Value::NIL } else { Value::TRUE } } else { value };
        Ok(())
    }
    pub(super) fn make_local_variable(&mut self, slot: usize) -> Result<()> {
        if self.protected_variable_slot(slot) || self.symbol_names[slot].starts_with(':') { return Err("setting constant".into()); }
        if let Some(fwd) = self.forwarding(slot) { return self.make_forwarded_local(slot, fwd); }
        let value = self.variable_value(slot)?;
        let table = if let Some(table) = self.local_table(slot)? { table } else {
            let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
            self.install_local_table(slot, table, true);
            table
        };
        self.buffer_index()?;
        let buffer = self.slots[self.buffer_slot].value;
        if self.declaration_hash_get(table, buffer)?.is_none() {
            let local = self.allocate_local_binding_slot();
            self.slots[local].value = value;
            self.declaration_hash_put(table, buffer, Value::reference(local+1,7))?;
        }
        Ok(())
    }
    pub(super) fn variable_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        if n != if matches!(p, DefvarSetDefault | Set) { 2 } else { 1 } { return Err("wrong primitive arity".into()); }
        let symbol = self.stack[start];
        if symbol == Value::NIL || symbol == Value::TRUE {
            return match p {
                SymbolValue => Ok(symbol),
                SpecialVariableP => Ok(Value::TRUE),
                Fboundp | LocalVariableIfSetP | CustomVariableP => Ok(Value::NIL),
                _ => Err("setting constant".into())
            };
        }
        let slot = symbol.index(7).map_err(|_| "symbol required")?.checked_sub(1).ok_or("symbol required")?;
        // Every value operation acts on the variable at the end of the alias
        // chain; function-cell and declaration queries stay on SYMBOL itself.
        if p == Makunbound && matches!(self.variable_redirects.get(slot), Some(Some(VariableRedirect::Alias(_)))) {
            // GNU data.c:782-786: makunbound of an alias undoes the alias and
            // leaves the alias itself void; the base keeps its value.
            if self.protected_variable_slot(slot) { return Err("setting constant".into()); }
            self.variable_redirects[slot] = None;
            let name = self.symbol_names[slot].clone();
            self.initialize_variable_redirect(&name, slot);
            self.slots[slot].value = Value::UNBOUND;
            return Ok(symbol);
        }
        let original = slot;
        let slot = if matches!(p, Fboundp | SpecialVariableP | CustomVariableP) { slot } else { self.indirect_variable(slot) };
        let truth = |b| if b {Value::TRUE} else {Value::NIL};
        match p {
            Fboundp => Ok(truth(self.slots[slot].function != Value::UNBOUND && self.slots[slot].function != Value::NIL)),
            Set | Makunbound => {
                // ORIGINAL, not the resolved base: an alias of nil, t or a
                // keyword is itself constant (GNU copies trapped_write).
                if self.protected_variable_slot(original) || self.symbol_names[slot].starts_with(':') { return Err("setting constant".into()); }
                let value = if p == Set { self.stack[start+1] } else { Value::UNBOUND };
                self.set_variable_value(slot, value)?;
                Ok(if p == Set { value } else { symbol })
            }
            MakeLocalVariable => { self.make_local_variable(slot)?; Ok(symbol) }
            SymbolValue => {
                let value = self.variable_value(slot)?;
                if value == Value::UNBOUND { Err("unbound variable".into()) } else { Ok(value) }
            }
            SpecialVariableP => Ok(truth(self.slots[slot].declared_special || (self.names.get(&self.symbol_names[slot]) == Some(&slot) && self.specials.contains(&self.symbol_names[slot])))),
            CustomVariableP => {
                let property = self.intern("standard-value");
                Ok(truth(self.symbol_property(symbol, property)? != Value::NIL))
            }
            MakeVariableBufferLocal if self.forwarding(slot).is_some() => {
                match self.forwarding(slot) {
                    Some(forwarded::Fwd::Buffer(_)) => Ok(symbol),
                    Some(forwarded::Fwd::Bool(_)) => { self.promote_boolean_local(slot, false)?; Ok(symbol) }
                    _ => Err(forwarded::unlocalizable(&self.symbol_names[slot])),
                }
            }
            MakeVariableBufferLocal => {
                self.slots[slot].explicit_local = false;
                if self.local_table(slot)?.is_none() {
                    let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
                    self.install_local_table(slot, table, false);
                }
                Ok(symbol)
            }
            // GNU data.c:2494: all BUFFER_OBJFWD slots become local if set.
            LocalVariableIfSetP if self.forwarding(slot).is_some() => Ok(truth(matches!(self.forwarding(slot), Some(forwarded::Fwd::Buffer(_))))),
            LocalVariableIfSetP => {
                let local = self.variable_binding_slot(slot, false)? != slot;
                let automatic = self.local_table(slot)?.is_some() && !self.slots[slot].explicit_local;
                Ok(truth(automatic || local))
            },
            // A built-in variable is always bound (GNU Fboundp), so `defvar'
            // never replaces its value.
            DefvarBoundp if self.native_forwarding(slot).is_some() => Ok(Value::TRUE),
            DefvarSetDefault if self.native_forwarding(slot).is_some() => {
                let fwd = self.native_forwarding(slot).unwrap();
                let value = self.stack[start+1];
                self.set_forwarded_default(slot, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text)?;
                Ok(value)
            }
            DefvarBoundp => {
                let value = self.dynamic.iter().find(|(s,_,_,_)| *s==slot).map(|(_,v,_,_)| *v).unwrap_or(self.slots[slot].value);
                Ok(truth(value != Value::UNBOUND))
            }
            DefvarSetDefault => {
                let value = self.stack[start+1];
                if let Some((_,old,_,_)) = self.dynamic.iter_mut().find(|(s,_,_,_)| *s==slot) { *old = value; }
                else { self.slots[slot].value = value; }
                Ok(value)
            }
            _ => Err("invalid variable primitive".into()),
        }
    }
}
