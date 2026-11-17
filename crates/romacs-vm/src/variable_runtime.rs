//! Variable designation, and where a variable's value is: its symbol's
//! slot, a built-in's native storage, or the current buffer's own locals
//! (`local_vars.rs`); never its Lisp-visible plist.

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
        let image = ImageRegion::open(&store, &key).unwrap();
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
        let image = ImageRegion::open(&store, &key).unwrap();
        let (mut vm, _) = Vm::from_code_image(image, [0; 32]).unwrap();
        vm.eval_source_raw("(make-variable-buffer-local 'print-circle)").unwrap();
        assert_eq!(vm.eval_source_raw("(local-variable-if-set-p 'print-circle)").unwrap(), Value::TRUE, "image intrinsic localization");
        vm.eval_source_raw("(setq truncate-lines t)").unwrap();
        assert_eq!(vm.eval_source_raw("(local-variable-if-set-p 'truncate-lines)").unwrap(), Value::TRUE, "image lazy intrinsic localization");
        std::fs::remove_dir_all(store).unwrap();
    }

    #[test]
    fn a_killed_buffers_locals_go_with_it() {
        let mut vm = Vm::new();
        let mut counts = Vec::new();
        for _ in 0..3 {
            vm.eval_source_raw("(dotimes (i 1000) (with-temp-buffer (make-local-variable 's124-pool)))").unwrap();
            let slot = vm.slot("s124-pool");
            let redirect = vm.variable_redirects[slot];
            assert_eq!(redirect, Some(VariableRedirect::Localized));
            vm.collect(&mut []);
            assert_eq!(vm.variable_redirects[slot], redirect, "a kill retains the symbol redirect");
            assert!(vm.localized(slot));
            counts.push(vm.slots.len());
        }
        assert_eq!(counts[0], counts[2], "locals take no symbol slots: {counts:?}");
    }
}
use super::*;

#[cfg(test)]
thread_local! {
    /// The variables this thread's VMs have read through `Vm::variable_value`.
    static VARIABLE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// The variables this thread's VMs have read through `Vm::variable_value`.
#[cfg(test)]
pub(crate) fn variable_reads() -> usize {
    VARIABLE_READS.with(std::cell::Cell::get)
}

/// Where the current buffer has a variable's value (`Vm::binding`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    /// The default: the symbol's value cell or a built-in's native storage.
    Default,
    /// The local in the buffer at this object index.
    Local(usize),
}

/// A change to one buffer's local of a variable (`Vm::store_local`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum LocalStore {
    /// Give the buffer a local with this value, unless it has one.
    Make(Value),
    /// Store this value in the local it has.
    Set(Value),
    /// Drop the local it has.
    Kill,
}

/// Where one buffer's value of a variable is read from.
#[derive(Clone, Copy)]
enum Place {
    /// A symbol slot's value cell: the variable's default.
    Slot(usize),
    /// The buffer's local of the variable, this symbol.
    Local(Value),
    /// A built-in's native storage.
    Native(forwarded::Fwd),
    /// Found at each read: an alias, a per-buffer or a tracked built-in.
    Each,
}

/// Variables a primitive reads at every call, and where their values in one
/// buffer are (`Vm::read_places`): GNU reads a C variable, into which
/// set-buffer swapped the buffer's binding. A place holds for its buffer
/// until the buffer's locals change which variables they hold
/// (`LocalVars::version`) or a collection moves the buffer
/// (`Forwarded::collection_epoch`), and while the variable's redirect is the one
/// it was found under (an alias, a tracking and a first localization each
/// change it).
#[derive(Clone, Copy)]
pub(crate) struct Places<const N: usize> {
    symbols: [usize; N],
    buffer: Value,
    /// The buffer's object index and its locals' version when found.
    index: usize,
    version: u32,
    epoch: u64,
    /// Each variable's redirect when found, and where it was found.
    found: [(Option<VariableRedirect>, Place); N],
}
impl<const N: usize> Places<N> {
    /// Of the variables at symbol slots SYMBOLS, none found yet.
    pub(crate) fn new(symbols: [usize; N]) -> Self {
        Self { symbols, buffer: Value::UNBOUND, index: 0, version: 0, epoch: 0, found: [(None, Place::Each); N] }
    }
    /// The symbol slot of variable I.
    pub(crate) fn symbol(&self, i: usize) -> usize {
        self.symbols[i]
    }
}

impl Vm {
    /// GNU eval.c:747: search oldest-first for this exact symbol, without
    /// following aliases. A buffer's local's lets (SPECPDL_LET_LOCAL) are
    /// excluded. Saved values are already traced by `dynamic`.
    pub(super) fn default_toplevel_binding(&mut self, slot: usize) -> Option<usize> {
        // HOST(dynamic:saved): reads the bound variable, owner or flag, not the saved value
        self.dynamic.iter().position(|&(bound, _, _, kind)| bound == slot && kind != Let::Local)
    }

    /// GNU eval.c:766: the oldest SPECPDL_LET_LOCAL of exactly this symbol
    /// in BUFFER, by the symbol each binding recorded and its owner buffer.
    pub(super) fn local_toplevel_binding(&mut self, slot: usize, buffer: Value) -> Option<usize> {
        let symbol = Value::reference(slot + 1, 7);
        self.backtrace.bindings.iter().filter(|(_, bound)| *bound == symbol).map(|(at, _)| *at).find(|&at| {
            // HOST(dynamic:saved): reads the bound variable, owner or flag, not the saved value
            let (_, _, owner, kind) = self.dynamic[at];
            owner == buffer && kind == Let::Local
        })
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
    // without reverting SYMBOL_LOCALIZED. Killing a buffer must never clear
    // this redirect; nor does kill-local-variable.
    pub(super) fn initialize_variable_redirect(&mut self, name: &str, slot: usize) {
        self.variable_redirects.resize(self.variable_redirects.len().max(slot + 1), None);
        if self.variable_redirects[slot].is_none() && self.names.get(name) == Some(&slot)
            && let Some(fwd) = crate::forwarded::Forwarded::lookup(name) {
            self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(fwd));
        }
        if self.variable_redirects[slot].is_none() {
            let localized = self.slots[slot].localness != Localness::Never;
            self.variable_redirects[slot] = Some(if localized { VariableRedirect::Localized } else { VariableRedirect::Plain });
        }
        if self.tracking.is_some() {
            self.observe_new_variable(slot);
        }
    }
    /// Declare SLOT's variable one with buffer-local values, LOCALNESS
    /// unless it was so declared before, retaining native forwarding: a
    /// write of the symbol's declaration, once a variable.
    pub(super) fn declare_local(&mut self, slot: usize, localness: Localness) {
        if self.slots[slot].localness == Localness::Never {
            self.slots[slot].localness = localness;
        }
        self.variable_redirects.resize(self.variable_redirects.len().max(slot + 1), None);
        if let Some(VariableRedirect::Forwarded(fwd @ (forwarded::Fwd::Bool(_) | forwarded::Fwd::Int(_) | forwarded::Fwd::Obj(_)))) = self.variable_redirects[slot] {
            if fwd == forwarded::Fwd::Int(self.forwarded.eval_limit_index as u16) { self.forwarded.eval_limit_symbol = slot; }
            self.variable_redirects[slot] = Some(VariableRedirect::LocalizedForwarded(fwd));
        } else if !matches!(self.variable_redirects[slot], Some(VariableRedirect::Forwarded(_) | VariableRedirect::LocalizedForwarded(_) | VariableRedirect::Alias(_) | VariableRedirect::Localized)) {
            self.variable_redirects[slot] = Some(VariableRedirect::Localized);
        }
    }
    /// Whether the variable at SLOT (the end of its alias chain) may have
    /// buffer-local values: GNU's SYMBOL_LOCALIZED, as a tracking keeps it.
    pub(super) fn localized(&mut self, slot: usize) -> bool {
        if self.variable_redirects.get(slot).copied().flatten().is_none() {
            // Code images rebuild slots without interning; uninterned slots
            // are never forwarded. Resolve the redirect once.
            let name = self.symbol_names[slot].clone();
            if self.names.get(&name) != Some(&slot) {
                return self.slots[slot].localness != Localness::Never;
            }
            self.initialize_variable_redirect(&name, slot);
        }
        matches!(self.kept_redirect(slot), Some(VariableRedirect::Localized | VariableRedirect::LocalizedForwarded(_)))
    }
    /// Where the variable at SLOT has its value in the current buffer: the
    /// buffer's local, else the default. With CREATE, for a set: an
    /// automatically local variable gets the buffer's local, made with its
    /// default's value, unless a `let' of its default made in this buffer
    /// is in effect (GNU `let_shadows_buffer_binding_p`).
    pub(super) fn binding(&mut self, slot: usize, create: bool) -> Result<Binding> {
        // An alias has no binding of its own: GNU follows SYMBOL_VARALIAS in
        // every reader and writer (data.c:1629, 1719, ...).
        let slot = self.indirect_variable(slot);
        // Built-in automatic locals are declared at their first lookup, so
        // cold VM and code-image construction see none.
        if (slot == self.truncate_lines_slot || slot == self.case_symbols_slot) && self.slots[slot].localness == Localness::Never {
            self.declare_local(slot, Localness::Automatic);
        }
        if !self.localized(slot) { return Ok(Binding::Default); }
        let index = self.buffer_index()?;
        let symbol = Value::reference(slot + 1, 7);
        if self.per_buffer(index).is_some_and(|b| b.locals().contains(symbol)) { return Ok(Binding::Local(index)); }
        if !create || self.slots[slot].localness == Localness::Explicit { return Ok(Binding::Default); }
        // Assignment within a default binding belongs to that binding in
        // its originating buffer, not to a newly created local entry.
        let buffer = self.slots[self.buffer_slot].value;
        // HOST(dynamic:saved): reads the bound variable, owner or flag, not the saved value
        let shadowed = self.dynamic.iter().rev().any(|(bound, _, owner, kind)| {
            *kind == Let::Default && *bound == slot && (*owner == buffer || (*owner == Value::NIL && self.names.get("#:vm2-buffer:*scratch*").is_some_and(|i| self.slots[*i].value == buffer)))
        });
        if shadowed { return Ok(Binding::Default); }
        let value = if let Some(fwd) = self.localized_forwarded(slot) { self.forwarded_value(fwd)? } else { self.slots[slot].value };
        self.store_local(index, slot, LocalStore::Make(value));
        Ok(Binding::Local(index))
    }
    /// The local value of the variable at SLOT in the buffer at object
    /// INDEX, which has one; UNBOUND when void.
    pub(super) fn local_value(&self, index: usize, slot: usize) -> Value {
        self.per_buffer(index).and_then(|b| b.locals().get(Value::reference(slot + 1, 7))).unwrap_or(Value::UNBOUND)
    }
    /// The one way a buffer's Lisp locals change, but for a kill's
    /// wholesale clear (`PerBuffer::retain_locals`) and an indirect clone's
    /// copy (`Vm::clone_indirect_state`): STORE to the local of the
    /// variable at SLOT in the buffer at object INDEX. A store that would
    /// change nothing writes nothing.
    pub(crate) fn store_local(&mut self, index: usize, slot: usize, store: LocalStore) {
        let symbol = Value::reference(slot + 1, 7);
        let Some(has) = self.per_buffer(index).map(|b| b.locals().contains(symbol)) else { return };
        if has == matches!(store, LocalStore::Make(_)) {
            return;
        }
        let locals = self.per_buffer_mut(index).expect("a buffer").locals_mut();
        match store {
            LocalStore::Make(value) => { locals.insert(symbol, value); }
            LocalStore::Set(value) => *locals.get_mut(symbol).expect("its local") = value,
            LocalStore::Kill => { locals.remove(symbol); }
        }
    }
    /// SLOT's value in BUFFER, as `buffer-local-value' reads it: its
    /// buffer-local value there, else its default.  UNBOUND when void.
    pub(crate) fn value_in_buffer(&mut self, slot: usize, buffer: Value) -> Result<Value> {
        // Always local (PER_BUFFER_IDX -1): BUFFER's representation.
        if slot == self.multibyte_slot {
            return Ok(if matches!(self.object(buffer)?, data::Object::ByteBuffer(_)) { Value::NIL } else { Value::TRUE });
        }
        // A tracked variable's tracking is checked here, for BUFFER: its
        // forwarding below is the one the tracking keeps, none for a tracked
        // buffer-local variable.
        if matches!(self.access_forwarding(slot), Some(forwarded::Fwd::Tracked(_))) {
            self.tracked_access(slot, crate::world::Access::Read, buffer);
        }
        if let Some(fwd) = self.forwarding(slot) {
            return match fwd {
                forwarded::Fwd::Buffer(i) => Ok(self.forwarded_buffer_value(buffer.index(0)? - 1, i)),
                forwarded::Fwd::Tracked(_) => Ok(self.slots[slot].value),
                _ => self.forwarded_value(fwd),
            };
        }
        if self.localized(slot) {
            if let Some(value) = self.per_buffer(buffer.index(0)? - 1).and_then(|b| b.locals().get(Value::reference(slot + 1, 7))) {
                return Ok(value);
            }
            if let Some(fwd) = self.localized_forwarded(slot) {
                return self.forwarded_default(fwd);
            }
        }
        Ok(self.slots[slot].value)
    }
    pub(super) fn variable_value(&mut self, slot: usize) -> Result<Value> {
        #[cfg(test)]
        VARIABLE_READS.with(|reads| reads.set(reads.get() + 1));
        let base = self.indirect_variable(slot);
        let binding = self.binding(base, false)?;
        if let Binding::Local(index) = binding {
            return Ok(self.local_value(index, base));
        }
        match self.native_forwarding(base) {
            Some(fwd) => self.forwarded_value(fwd),
            None => Ok(self.slots[base].value),
        }
    }
    /// The values of PLACES' variables in the current buffer, each as
    /// `variable_value` gives it, read where they were found: None when
    /// the places no longer hold, or one of them is found at each read,
    /// for [`Self::find_places`].
    #[inline]
    pub(crate) fn read_places<const N: usize>(&self, places: &Places<N>) -> Option<[Value; N]> {
        if places.buffer != self.slots[self.buffer_slot].value || places.epoch != self.forwarded.collection_epoch {
            return None;
        }
        let locals = self.per_buffer(places.index)?.locals();
        if locals.version() != places.version {
            return None;
        }
        // By index: every call of a search or a scan comes through here.
        let mut values = [Value::NIL; N];
        let mut i = 0;
        while i < N {
            let (redirect, place) = places.found[i];
            if self.variable_redirects.get(places.symbols[i]).copied().flatten() != redirect {
                return None;
            }
            values[i] = match place {
                Place::Slot(slot) => self.slots[slot].value,
                Place::Local(symbol) => locals.get(symbol)?,
                Place::Native(fwd) => self.forwarded.stored(fwd)?,
                Place::Each => return None,
            };
            i += 1;
        }
        Some(values)
    }
    /// The values of PLACES' variables in the current buffer, each read
    /// with `variable_value`, and PLACES made theirs there.
    #[cold]
    pub(crate) fn find_places<const N: usize>(&mut self, places: &mut Places<N>) -> Result<[Value; N]> {
        let mut values = [Value::NIL; N];
        for (i, value) in values.iter_mut().enumerate() {
            let symbol = places.symbols[i];
            *value = self.variable_value(symbol)?;
            // After the read, which resolves a lazy redirect.
            places.found[i] = (self.variable_redirects.get(symbol).copied().flatten(), self.variable_place(symbol)?);
        }
        places.index = self.buffer_index()?;
        places.buffer = self.slots[self.buffer_slot].value;
        places.version = self.per_buffer(places.index).map_or(0, |b| b.locals().version());
        places.epoch = self.forwarded.collection_epoch;
        Ok(values)
    }
    /// Where `variable_value` reads SLOT's value in the current buffer.
    fn variable_place(&mut self, slot: usize) -> Result<Place> {
        use forwarded::Fwd;
        if self.indirect_variable(slot) != slot {
            return Ok(Place::Each);
        }
        let local = matches!(self.binding(slot, false)?, Binding::Local(_)).then(|| Place::Local(Value::reference(slot + 1, 7)));
        Ok(match self.native_forwarding(slot) {
            Some(fwd @ (Fwd::Obj(_) | Fwd::Bool(_) | Fwd::Int(_))) => local.unwrap_or(Place::Native(fwd)),
            Some(Fwd::Buffer(_) | Fwd::Tracked(_)) => Place::Each,
            None => local.unwrap_or(Place::Slot(slot)),
        })
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
    /// which a tracked variable makes through the tracking.
    pub(super) fn assign_variable(&mut self, slot: usize, value: Value) -> Result<()> {
        let base = self.indirect_variable(slot);
        if let Some(fwd) = self.native_forwarding(base) {
            if value == Value::UNBOUND {
                return self.set_forwarded(base, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text);
            }
            let Binding::Local(index) = self.binding(base, true)? else {
                return self.set_forwarded(base, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text);
            };
            let value = Self::localized_value(fwd, value).map_err(forwarded::fault_text)?;
            self.store_local(index, base, LocalStore::Set(value));
            return Ok(());
        }
        match self.binding(base, true)? {
            Binding::Local(index) => self.store_local(index, base, LocalStore::Set(value)),
            Binding::Default => self.slots[base].value = value,
        }
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
    /// Declare the forwarded global built-in at SLOT one with buffer-local
    /// values.
    pub(super) fn promote_forwarded_local(&mut self, slot: usize, explicit: bool) -> Result<()> {
        if matches!(self.forwarding(slot), Some(forwarded::Fwd::Bool(_)))
            // HOST(dynamic:saved): reads saved values of a localized Boolean, which the tracking never
            // holds
            && self.dynamic.iter().any(|(bound, old, _, _)| *bound == slot && *old != Value::NIL && *old != Value::TRUE) {
            return Err(format!("VM2 localized Boolean default requires nil or t: {}", self.symbol_names[slot]));
        }
        if !self.localized(slot) {
            self.declare_local(slot, if explicit { Localness::Explicit } else { Localness::Automatic });
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
        // A `let' moves a tracked value aside and back unread; a watcher
        // would see it, so a build does not capture that.
        if let Some(forwarded::Fwd::Tracked(cell)) = self.native_forwarding(base) {
            self.tracked_watched(base);
            return self.untracked(cell as usize, |vm| vm.bind_variable(slot, value));
        }
        let mut local = self.binding(base, false)?;
        let forwarded = self.native_forwarding(base).filter(|_| local == Binding::Default);
        let binding = match (forwarded, local) {
            (Some(fwd), _) => self.forwarded_binding(base, fwd).map_err(forwarded::fault_text)?,
            (None, Binding::Local(index)) => (base, self.local_value(index, base), self.slots[self.buffer_slot].value, Let::Local),
            (None, Binding::Default) => {
                let kind = if self.localized(base) { Let::Default } else { Let::Plain };
                (base, self.slots[base].value, self.slots[self.buffer_slot].value, kind)
            }
        };
        if base != self.buffer_slot {
            self.backtrace.bind(self.dynamic.len(), Value::reference(base + 1, 7));
        }
        // HOST(dynamic:saved): specbind: pushes a new binding, saving the value it shadows
        self.dynamic.push(binding);
        let mut value = value;
        if self.variable_trapped(base) {
            // The value is rooted across its watchers and read back.
            let frame = self.root_frame(&[value]);
            let notified = if matches!(forwarded, Some(forwarded::Fwd::Buffer(_))) && binding.3 == Let::Default {
                self.notify_watchers_at(base, value, "set", Some(Value::NIL))
            } else {
                self.notify_watchers(base, value, "let")
            };
            [value] = self.release_frame(frame);
            notified?;
            // set_internal finds the binding to store into after the watchers.
            local = self.binding(base, false)?;
        }
        if let Some(fwd) = forwarded {
            return self.store_forwarded_binding(base, fwd, value, binding.3).map_err(forwarded::fault_text);
        }
        let value = match self.localized_forwarded(base) {
            Some(fwd) => Self::localized_value(fwd, value).map_err(forwarded::fault_text)?,
            None => value,
        };
        match local {
            Binding::Local(index) => self.store_local(index, base, LocalStore::Set(value)),
            Binding::Default => self.slots[base].value = value,
        }
        Ok(())
    }
    pub(super) fn make_local_variable(&mut self, slot: usize) -> Result<()> {
        if self.protected_variable_slot(slot) || self.interned_keyword_slot(slot) { return Err("setting constant".into()); }
        // A tracked variable's forwarding is `Fwd::Tracked`, whose arm acts
        // through the tracking: the kept forwarding would never localize it.
        if let Some(fwd) = self.access_forwarding(slot) { return self.make_forwarded_local(slot, fwd); }
        let value = self.variable_value(slot)?;
        if !self.localized(slot) {
            self.declare_local(slot, Localness::Explicit);
        }
        let index = self.buffer_index()?;
        self.store_local(index, slot, LocalStore::Make(value));
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
                    Some(forwarded::Fwd::Tracked(_)) => self.tracked(slot, crate::world::Access::Write, |vm| vm.variable_primitive(p, start, n)),
                    _ => { self.promote_forwarded_local(slot, false)?; Ok(symbol) }
                }
            }
            MakeVariableBufferLocal => {
                // data.c Fmake_variable_buffer_local: a plain void variable's
                // default becomes nil; a localized one keeps it.
                if !self.localized(slot) && self.slots[slot].value == Value::UNBOUND { self.slots[slot].value = Value::NIL; }
                if self.slots[slot].localness != Localness::Automatic { self.slots[slot].localness = Localness::Automatic; }
                self.declare_local(slot, Localness::Automatic);
                Ok(symbol)
            }
            // A built-in variable is always bound (GNU Fboundp), so `defvar'
            // never replaces its value.
            // These read or write the toplevel or saved value, never a let's.
            DefvarBoundp | DefvarSetDefault if matches!(self.native_forwarding(slot), Some(forwarded::Fwd::Tracked(_))) => {
                let access = if p == DefvarBoundp { crate::world::Access::SavedRead } else { crate::world::Access::SavedWrite };
                self.tracked(slot, access, |vm| vm.variable_primitive(p, start, n))
            }
            DefvarBoundp if self.native_forwarding(slot).is_some() => Ok(Value::TRUE),
            DefvarSetDefault if self.native_forwarding(slot).is_some() => {
                let fwd = self.native_forwarding(slot).unwrap();
                let value = self.stack[start+1];
                self.set_forwarded_default(slot, fwd, value, forwarded::SetFlag::Set).map_err(forwarded::fault_text)?;
                Ok(value)
            }
            // GNU Fdefvar (eval.c:944): nothing to set when the current
            // default is bound and no let shadows an unbound toplevel value.
            DefvarBoundp => {
                // HOST(dynamic:saved): saved-binding accessor: defvar's boundp; variable_primitive
                // checks a tracked variable first (F2)
                let shadowed_unbound = self.dynamic.iter().find(|(s,_,_,_)| *s==slot).is_some_and(|(_,v,_,_)| *v == Value::UNBOUND);
                Ok(truth(self.slots[slot].value != Value::UNBOUND && !shadowed_unbound))
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
            // GNU defvar, after the value: Fset_default of a void current
            // binding (a let's too), else a let-shadowed unbound toplevel
            // binding's saved value written in place, without notification.
            // A current binding the value itself bound is set, as GNU sets it
            // where no let shadowed (DefvarBoundp decided before the value ran).
            DefvarSetDefault => {
                let value = self.stack[start+1];
                let shadow = if self.slots[slot].value == Value::UNBOUND { None } else {
                    // HOST(dynamic:saved): saved-binding accessor: defvar's set-default;
                    // variable_primitive checks a tracked variable first (F2)
                    self.dynamic.iter_mut().find(|(s,_,_,_)| *s==slot).filter(|(_,v,_,_)| *v == Value::UNBOUND)
                };
                if let Some((_,old,_,_)) = shadow { *old = value; }
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
