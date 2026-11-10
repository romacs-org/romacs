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

    /// GNU eval.c:766: the oldest SPECPDL_LET_LOCAL of exactly this symbol
    /// in BUFFER, by the symbol each binding recorded and its owner buffer.
    /// An ordinary or localized Boolean variable's local let was made on a
    /// private slot; a per-buffer forwarded one on its own slot without the
    /// LET_DEFAULT flag.
    pub(super) fn local_toplevel_binding(&mut self, slot: usize, buffer: Value) -> Option<usize> {
        let symbol = Value::reference(slot + 1, 7);
        let buffer_forwarded = matches!(self.forwarding(slot), Some(forwarded::Fwd::Buffer(_)));
        self.backtrace.bindings.iter().filter(|(_, bound)| *bound == symbol).map(|(at, _)| *at).find(|&at| {
            let (bound, _, owner, let_default) = self.dynamic[at];
            owner == buffer && if buffer_forwarded { bound == slot && !let_default } else { bound != slot }
        })
    }

    pub(crate) fn allocate_local_binding_slot(&mut self) -> usize {
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
            || (base != slot && (matches!(self.symbol_names[base].as_str(), "nil" | "t") || self.interned_keyword_slot(base)))
    }
    /// Whether SLOT is a keyword GNU makes constant: only a `:' name interned
    /// in the obarray is SYMBOL_NOWRITE (lread.c intern_sym); an uninterned
    /// `:x' is an ordinary variable.
    pub(crate) fn interned_keyword_slot(&self, slot: usize) -> bool {
        let name = &self.symbol_names[slot];
        name.starts_with(':') && self.names.get(name) == Some(&slot)
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
    /// SYMBOL's alias undone (Finternal_delete_indirect_variable): plain and
    /// void.
    pub(super) fn delete_variable_alias(&mut self, slot: usize) {
        self.variable_redirects[slot] = None;
        self.slots[slot].value = Value::UNBOUND;
    }
    // Redirects contain stable slot IDs, never untraced moving-heap Values.
    // Publication is set-only: GNU data.c:2385-2408 removes a local entry
    // without reverting SYMBOL_LOCALIZED. GC owner reclamation must likewise
    // never clear this redirect; future kill-local-variable must preserve it.
    // The local table lives in the slot itself (`Slot::local`), never on the
    // Lisp plist, so no plist operation can install or remove it.
    pub(super) fn initialize_variable_redirect(&mut self, name: &str, slot: usize) {
        self.variable_redirects.resize(self.variable_redirects.len().max(slot + 1), None);
        if self.variable_redirects[slot].is_none() && self.names.get(name) == Some(&slot)
            && let Some(fwd) = self.forwarded.lookup(name) {
            self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(fwd));
        }
        if self.variable_redirects[slot].is_none() {
            let localized = self.slots[slot].local != Value::NIL;
            self.variable_redirects[slot] = Some(if localized { VariableRedirect::Localized } else { VariableRedirect::Plain });
        }
        if self.fence.is_some() {
            self.observe_new_variable(slot);
        }
    }
    /// Publish SLOT's local TABLE, retaining native Boolean forwarding.
    pub(super) fn install_local_table(&mut self, slot: usize, table: Value, explicit: bool) {
        self.slots[slot].local = table;
        self.slots[slot].explicit_local = explicit;
        self.variable_redirects.resize(self.variable_redirects.len().max(slot + 1), None);
        if let Some(VariableRedirect::Forwarded(fwd @ (forwarded::Fwd::Bool(_) | forwarded::Fwd::Int(_) | forwarded::Fwd::Obj(_)))) = self.variable_redirects[slot] {
            if fwd == forwarded::Fwd::Int(self.forwarded.eval_limit_index as u16) { self.forwarded.eval_limit_symbol = slot; }
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
        if !matches!(self.kept_redirect(slot), Some(VariableRedirect::Localized | VariableRedirect::LocalizedForwarded(_))) { return Ok(None); }
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
        if (slot == self.truncate_lines_slot || slot == self.case_symbols_slot) && self.local_table(slot)?.is_none() {
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
        self.slots[local].value = if let Some(fwd) = self.localized_forwarded(slot) { self.forwarded_value(fwd)? } else { self.slots[slot].value };
        self.declaration_hash_put(table, buffer, Value::reference(local+1, 7))?;
        self.note_local_binding(buffer, slot);
        Ok(local)
    }
    /// SLOT's value in BUFFER, as `buffer-local-value' reads it: its
    /// buffer-local value there, else its default.  UNBOUND when void.
    pub(crate) fn value_in_buffer(&mut self, slot: usize, buffer: Value) -> Result<Value> {
        // Always local (PER_BUFFER_IDX -1): BUFFER's representation.
        if slot == self.multibyte_slot {
            return Ok(if matches!(self.object(buffer)?, data::Object::ByteBuffer(_)) { Value::NIL } else { Value::TRUE });
        }
        // A fenced variable's fence is checked here, for BUFFER: its
        // forwarding below is the one the fence keeps, none for a fenced
        // buffer-local variable.
        if matches!(self.access_forwarding(slot), Some(forwarded::Fwd::Fence(_))) {
            self.fence_access(slot, crate::world::Access::Read, buffer);
        }
        if let Some(fwd) = self.forwarding(slot) {
            return match fwd {
                forwarded::Fwd::Buffer(i) => Ok(self.forwarded_buffer_value(buffer.index(0)? - 1, i)),
                forwarded::Fwd::Fence(_) => Ok(self.slots[slot].value),
                _ => self.forwarded_value(fwd),
            };
        }
        if let Some(table) = self.local_table(slot)? {
            if let Some(local) = self.declaration_hash_get(table, buffer)? {
                return Ok(self.slots[local.index(7)? - 1].value);
            }
            if let Some(fwd) = self.localized_forwarded(slot) {
                return self.forwarded_default(fwd);
            }
        }
        Ok(self.slots[slot].value)
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
        let base = self.indirect_variable(slot);
        let mut value = value;
        if self.variable_trapped(base) {
            // The value is rooted across its watchers and read back.
            let frame = self.root_frame(&[value]);
            let notified = self.notify_set(base, &frame);
            [value] = self.release_frame(frame);
            notified?;
        }
        self.assign_variable(slot, value)
    }
    /// `set_variable_value` once the watchers have been told: the store,
    /// which a fenced variable makes through the fence.
    pub(super) fn assign_variable(&mut self, slot: usize, value: Value) -> Result<()> {
        let base = self.indirect_variable(slot);
        if let Some(fwd) = self.native_forwarding(base) {
            if value == Value::UNBOUND {
                return self.set_forwarded(base, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text);
            }
            let local = self.variable_binding_slot(base, true)?;
            if local == base {
                return self.set_forwarded(base, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text);
            }
            self.slots[local].value = Self::localized_value(fwd, value).map_err(forwarded::fault_text)?;
            return Ok(());
        }
        let local = self.variable_binding_slot(slot, true)?;
        self.slots[local].value = value;
        Ok(())
    }
    /// GNU `default_value` (data.c:1960) of the variable at SLOT, the end of
    /// its alias chain: UNBOUND when void.
    pub(super) fn default_value(&mut self, slot: usize) -> Result<Value> {
        match self.native_forwarding(slot) {
            Some(fwd) => self.forwarded_default(fwd),
            None => Ok(self.slots[slot].value),
        }
    }
    /// GNU `set_default_internal` (data.c:2063) at SLOT, after its checks.
    pub(super) fn set_default_value(&mut self, slot: usize, value: Value) -> std::result::Result<(), crate::prim::Fault> {
        match self.native_forwarding(slot) {
            Some(fwd) => self.set_forwarded_default(slot, fwd, value, forwarded::SetFlag::Set),
            None => {
                self.slots[slot].value = value;
                Ok(())
            }
        }
    }
    /// `set_variable_value's notifications of the value in FRAME.
    fn notify_set(&mut self, base: usize, frame: &crate::gc::RootFrame) -> Result<()> {
        if self.rooted(frame, 0) == Value::UNBOUND {
            return self.notify_watchers_at(base, Value::UNBOUND, "makunbound", Some(Value::NIL));
        }
        self.notify_watchers(base, self.rooted(frame, 0), "set")?;
        // set_internal's set_default_internal reports again.
        if let Some(forwarded::Fwd::Buffer(i)) = self.forwarding(base) {
            let buffer = self.buffer_index()?;
            if self.sets_shadowed_default(base, i, buffer) { self.notify_watchers_at(base, self.rooted(frame, 0), "set", Some(Value::NIL))?; }
        }
        Ok(())
    }
    /// Give the forwarded global built-in at SLOT its local-binding table.
    pub(super) fn promote_forwarded_local(&mut self, slot: usize, explicit: bool) -> Result<()> {
        if matches!(self.forwarding(slot), Some(forwarded::Fwd::Bool(_)))
            && self.dynamic.iter().any(|(bound, old, _, _)| *bound == slot && *old != Value::NIL && *old != Value::TRUE) {
            return Err(format!("VM2 localized Boolean default requires nil or t: {}", self.symbol_names[slot]));
        }
        if self.local_table(slot)?.is_none() {
            let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
            self.install_local_table(slot, table, explicit);
        }
        Ok(())
    }
    /// GNU `specbind`: record the binding, then `do_specbind` notifies the
    /// watchers and stores, so unbinding restores the value saved before
    /// any watcher ran, and a watcher's signal leaves the binding to undo.
    /// A per-buffer variable's default binding goes through
    /// set_default_internal, which reports `set'.
    pub(super) fn bind_variable(&mut self, slot: usize, value: Value) -> Result<()> {
        let base = self.indirect_variable(slot);
        // A `let' moves a fenced value aside and back unread; a watcher
        // would see it, so a build refuses that.
        if let Some(forwarded::Fwd::Fence(cell)) = self.native_forwarding(base) {
            self.fence_watched(base);
            return self.unfenced(cell as usize, |vm| vm.bind_variable(slot, value));
        }
        let local = self.variable_binding_slot(base, false)?;
        let forwarded = self.native_forwarding(base).filter(|_| local == base);
        let binding = match forwarded {
            Some(fwd) => self.forwarded_binding(base, fwd).map_err(forwarded::fault_text)?,
            None => (local, self.slots[local].value, self.slots[self.buffer_slot].value, self.local_table(base)?.is_some()),
        };
        if base != self.buffer_slot {
            self.backtrace.bind(self.dynamic.len(), Value::reference(base + 1, 7));
        }
        self.dynamic.push(binding);
        let mut local = local;
        let mut value = value;
        if self.variable_trapped(base) {
            // The value is rooted across its watchers and read back.
            let frame = self.root_frame(&[value]);
            let notified = if matches!(forwarded, Some(forwarded::Fwd::Buffer(_))) && binding.3 {
                self.notify_watchers_at(base, value, "set", Some(Value::NIL))
            } else {
                self.notify_watchers(base, value, "let")
            };
            [value] = self.release_frame(frame);
            notified?;
            // set_internal finds the binding to store into after the watchers.
            local = self.variable_binding_slot(base, false)?;
        }
        if let Some(fwd) = forwarded {
            return self.store_forwarded_binding(base, fwd, value, binding.3).map_err(forwarded::fault_text);
        }
        self.slots[local].value = match self.localized_forwarded(base) {
            Some(fwd) => Self::localized_value(fwd, value).map_err(forwarded::fault_text)?,
            None => value,
        };
        Ok(())
    }
    pub(super) fn make_local_variable(&mut self, slot: usize) -> Result<()> {
        if self.protected_variable_slot(slot) || self.interned_keyword_slot(slot) { return Err("setting constant".into()); }
        // A fenced variable's forwarding is `Fwd::Fence`, whose arm acts
        // through the fence: the kept forwarding would never localize it.
        if let Some(fwd) = self.access_forwarding(slot) { return self.make_forwarded_local(slot, fwd); }
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
            self.note_local_binding(buffer, slot);
        }
        Ok(())
    }
    /// `special-variable-p' of SLOT's symbol: declared special now.
    pub(crate) fn slot_is_special(&self, slot: usize) -> bool {
        self.slots[slot].declared_special
            || (self.names.get(&self.symbol_names[slot]) == Some(&slot) && self.specials.contains(&self.symbol_names[slot]))
    }
    /// What `special_cache' answers hold under: a specials change since
    /// moves it (an insertion that matters grows `specials').
    fn special_key(&self) -> u64 {
        ((self.specials.len() as u64) << 24 ^ self.special_changes) + 1
    }
    /// `slot_is_special', remembered per slot until specials change: the
    /// check each late binding makes as it binds.
    #[inline]
    pub(crate) fn slot_is_special_now(&mut self, slot: usize) -> bool {
        let key = self.special_key();
        if let Some(&cached) = self.special_cache.get(slot) && cached >> 1 == key {
            return cached & 1 == 1;
        }
        let special = self.slot_is_special(slot);
        if self.special_cache.len() <= slot { self.special_cache.resize(slot + 1, 0); }
        self.special_cache[slot] = key << 1 | u64::from(special);
        special
    }
    pub(super) fn variable_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        if n != if matches!(p, DefvarSetDefault | Set) { 2 } else { 1 } { return Err("wrong primitive arity".into()); }
        let symbol = self.stack[start];
        if symbol == Value::NIL || symbol == Value::TRUE {
            return match p {
                SymbolValue => Ok(symbol),
                SpecialVariableP => Ok(Value::TRUE),
                Fboundp | CustomVariableP => Ok(Value::NIL),
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
                if self.protected_variable_slot(original) || self.interned_keyword_slot(slot) { return Err("setting constant".into()); }
                let value = if p == Set { self.stack[start+1] } else { Value::UNBOUND };
                // A write nothing hears stores at once; a heard one waits for
                // its engine's drain (`Vm::write_from_native`).
                if self.deferred_writes.is_empty() && !self.variable_trapped(slot) {
                    self.assign_variable(slot, value)?;
                } else {
                    self.write_from_native(crate::watchers::NativeWrite::set(slot, value)).map_err(crate::watchers::fault_text)?;
                }
                Ok(if p == Set { value } else { symbol })
            }
            MakeLocalVariable => { self.make_local_variable(slot)?; Ok(symbol) }
            SymbolValue => {
                let value = self.variable_value(slot)?;
                if value == Value::UNBOUND { Err("unbound variable".into()) } else { Ok(value) }
            }
            SpecialVariableP => Ok(truth(self.slot_is_special(slot))),
            CustomVariableP => {
                let property = self.intern("standard-value");
                Ok(truth(self.symbol_property(symbol, property)? != Value::NIL))
            }
            MakeVariableBufferLocal if self.access_forwarding(slot).is_some() => {
                match self.access_forwarding(slot) {
                    Some(forwarded::Fwd::Buffer(_)) => Ok(symbol),
                    Some(forwarded::Fwd::Fence(_)) => self.fenced(slot, crate::world::Access::Write, |vm| vm.variable_primitive(p, start, n)),
                    _ => { self.promote_forwarded_local(slot, false)?; Ok(symbol) }
                }
            }
            MakeVariableBufferLocal => {
                self.slots[slot].explicit_local = false;
                if self.local_table(slot)?.is_none() {
                    // data.c Fmake_variable_buffer_local: a plain void
                    // variable's default becomes nil; a localized one keeps it.
                    if self.slots[slot].value == Value::UNBOUND { self.slots[slot].value = Value::NIL; }
                    let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
                    self.install_local_table(slot, table, false);
                }
                Ok(symbol)
            }
            // A built-in variable is always bound (GNU Fboundp), so `defvar'
            // never replaces its value.
            // These read or write the toplevel or saved value, never a let's.
            DefvarBoundp | DefvarSetDefault if matches!(self.native_forwarding(slot), Some(forwarded::Fwd::Fence(_))) => {
                let access = if p == DefvarBoundp { crate::world::Access::SavedRead } else { crate::world::Access::SavedWrite };
                self.fenced(slot, access, |vm| vm.variable_primitive(p, start, n))
            }
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
            // GNU Fdefvar without a value at a loading file's own level: the
            // name is special for the rest of the file, and macroexp.el's
            // `macroexp--dynvars' records it as macroexpand--all-toplevel does.
            DefvarFileLocal => {
                let name = self.symbol_names[original].clone();
                if self.file_specials.insert(name) {
                    let dynvars = self.slot("macroexp--dynvars");
                    let declared = self.variable_value(dynvars)?;
                    let declared = self.keymap_cons(symbol, declared);
                    self.set_variable_value(dynvars, declared)?;
                }
                Ok(symbol)
            }
            // GNU defvar: Fset_default, or a let-shadowed toplevel binding's
            // saved value written in place, without notification.
            DefvarSetDefault => {
                let value = self.stack[start+1];
                if let Some((_,old,_,_)) = self.dynamic.iter_mut().find(|(s,_,_,_)| *s==slot) { *old = value; }
                else {
                    self.notify_watchers_at(slot, value, "set", Some(Value::NIL))?;
                    self.slots[slot].value = value;
                }
                Ok(value)
            }
            _ => Err("invalid variable primitive".into()),
        }
    }
}
