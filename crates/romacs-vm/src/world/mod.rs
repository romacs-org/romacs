//! World builds, the VM's side (DESIGN-M1 §5, §6): the recipe runs with
//! `host::Mode::Build`, and the VM marks where it is in the build and keeps
//! the P fence.
//!
//! The fence keeps the builder from computing anything from the values that
//! restore rebuilds (§6.2): the environment lists, `default-directory`, the
//! variables on the delayed list once startup has set them, and the random
//! state. Outside startup, a read or write of a fenced binding refuses the
//! build, naming the root and the site. A `let` is exempt: it moves the
//! fenced value aside and back unread, and a binding it makes after the fence
//! is the `let`'s own; the value it saved is still the root's (F2).
//!
//! Root 6 keeps the startup re-run reading what the first run read (F1):
//! while startup runs, every variable it reads, every property it reads and
//! every function cell it calls through is recorded as an input, and after
//! it a write to an input refuses, whatever value it stores. Reads pass.
//! An in-place mutation has no write site, so the content of each input is
//! digested as startup ends and compared at the end of the recipe.
//!
//! A fenced variable's redirect becomes `Forwarded(Fwd::Fence(slot))`, so
//! every access already takes the forwarded paths, whose `Fence` arms check
//! the fence and act through the redirect the fence keeps. While startup
//! runs every variable is wrapped so, to observe its reads. No path of an
//! unfenced variable changes. Calls are observed through the backtrace's
//! token counter (`Backtrace::observe`).
pub mod arena;
pub mod image;
pub mod store;
pub mod sandbox;

use crate::data::Object;
use crate::forwarded::Fwd;
use crate::{Value, VariableRedirect, Vm, host};
use serde_json::json;

/// A fenced value's root (§6.2). `Observed` is no root: a variable startup
/// may read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Root {
    Observed = 0,
    Environment = 1,
    Directory = 2,
    Delayed = 3,
    Reseed = 4,
    Random = 5,
    Input = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
    /// Of the value a `let` saved.
    SavedRead,
    SavedWrite,
}

/// What startup read (root 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Input {
    Variable(usize),
    /// A symbol's property, by its key symbol's slot.
    Property(usize, usize),
    /// A symbol's whole plist, as `symbol-plist` reads it.
    Plist(usize),
    Function(usize),
}

/// The P fence of a build.
#[derive(Default)]
pub(crate) struct Fence {
    cells: rustc_hash::FxHashMap<usize, Cell>,
    random: bool,
    /// The delayed list as startup began.
    delayed: Vec<usize>,
    /// The delayed list of the last startup run, which a later one computes
    /// again.
    superseded: Vec<usize>,
    /// Startup is running: variables are wrapped and reads are inputs.
    observing: bool,
    /// The inputs in first-read order, each once.
    inputs: Vec<Input>,
    seen: rustc_hash::FxHashSet<Input>,
    /// Each input's content digest as startup ended, or the kind of object
    /// that made it undigestable.
    digests: Vec<Result<String, &'static str>>,
}

/// A fenced variable: its root, the redirect the fence keeps, and the
/// dynamic entries that bound it when it was fenced and are still bound. A
/// `let` of any other entry is newer than the fence.
struct Cell {
    root: Root,
    redirect: Option<VariableRedirect>,
    older: Vec<usize>,
}

impl Vm {
    /// The VM is made: the recipe starts, and per-process host facts may no
    /// longer be read outside startup. In build mode the environment lists,
    /// `default-directory` and the random state are fenced. A no-op outside
    /// build mode.
    pub fn world_begin(&mut self) {
        if host::mode() == host::Mode::Build {
            self.fence = Some(Box::default());
            for name in ["process-environment", "initial-environment", "exec-path"] {
                let slot = self.slot(name);
                self.fence_variable(slot, Root::Environment);
            }
            let slot = self.slot("default-directory");
            self.fence_variable(slot, Root::Directory);
            self.fence_mut().random = true;
        }
        host::begin_recipe();
    }

    /// Lift the fence once the build has checked it (§5 step 4): every
    /// fenced variable gets back the redirect the fence kept, so no build
    /// redirect is ever captured.
    pub fn world_lift(&mut self) {
        let Some(fence) = self.fence.take() else { return };
        for (slot, cell) in fence.cells {
            self.variable_redirects[slot] = cell.redirect;
        }
    }

    /// Drop the values restore rebuilds before the capture collection
    /// (§5 step 5): the environment lists, with the strings they share.
    pub fn world_normalize(&mut self) {
        for name in ["process-environment", "initial-environment", "exec-path"] {
            let slot = self.slot(name);
            self.slots[slot].value = Value::NIL;
        }
        self.environment_snapshot = None;
    }

    /// A top-level form of a loaded file ends: a re-seed form's variable is
    /// fenced from here (root 4).
    pub(crate) fn world_form_done(&mut self) {
        if let Some(name) = host::reseed_output().filter(|_| self.fence.is_some()) {
            let slot = self.slot(name);
            self.fence_variable(slot, Root::Reseed);
        }
    }

    /// `romacs--startup-phase`: the startup function's run begins (ON) or
    /// ends. While it runs, its reads are observed. At its end the variables
    /// then on the delayed list are fenced, and writes to its inputs refuse.
    /// A no-op outside build mode.
    pub(crate) fn startup_phase(&mut self, on: bool) {
        if host::mode() != host::Mode::Build || !host::switch_startup(on) {
            return;
        }
        if on {
            // A later run supersedes the earlier: it computes the delayed
            // variables again and reads its own inputs, and restore takes
            // only it (`romacs-startup-function').
            if host::startups() > 1 {
                self.supersede_startup();
            }
            let slot = self.slot("custom-delayed-init-variables");
            let mut tail = self.variable_value(slot).unwrap_or(Value::NIL);
            let mut delayed = Vec::new();
            while let Ok((symbol, rest)) = self.cons_pair(tail) {
                if let Some(slot) = symbol.index_if(7).and_then(|i| i.checked_sub(1)) {
                    delayed.push(self.indirect_variable(slot));
                }
                tail = rest;
            }
            // Once the first run ends the list is t: a later run computes
            // the first run's variables.
            if delayed.is_empty() {
                delayed = self.fence_mut().superseded.clone();
            }
            self.fence_mut().delayed = delayed;
            self.fence_mut().observing = true;
            for slot in 0..self.variable_redirects.len() {
                if self.variable_redirects[slot].is_some() {
                    self.fence_variable(slot, Root::Observed);
                }
            }
            self.backtrace.observe();
        } else {
            self.end_startup();
        }
    }

    /// A later startup run begins: the earlier run's delayed variables and
    /// inputs leave the fence, which the later run fences again as it ends.
    fn supersede_startup(&mut self) {
        let fence = self.fence_mut();
        let earlier: Vec<usize> = fence.cells.iter()
            .filter(|(_, cell)| matches!(cell.root, Root::Delayed | Root::Input)).map(|(&slot, _)| slot).collect();
        fence.inputs.clear();
        fence.seen.clear();
        fence.digests.clear();
        for slot in earlier {
            if let Some(cell) = self.fence_mut().cells.remove(&slot) {
                self.variable_redirects[slot] = cell.redirect;
            }
        }
    }

    fn end_startup(&mut self) {
        let calls = self.backtrace.observed();
        let fence = self.fence_mut();
        fence.observing = false;
        let observed: Vec<usize> = fence.cells.iter().filter(|(_, cell)| cell.root == Root::Observed).map(|(&slot, _)| slot).collect();
        for slot in observed {
            let cell = self.fence_mut().cells.remove(&slot).expect("an observed cell");
            self.variable_redirects[slot] = cell.redirect;
        }
        for slot in calls {
            self.observe_function(slot);
        }
        let delayed = std::mem::take(&mut self.fence_mut().delayed);
        self.fence_mut().superseded = delayed.clone();
        let names: Vec<&str> = delayed.iter().map(|&slot| self.symbol_names[slot].as_str()).collect();
        host::record("delayed", json!({"symbols": names}));
        for slot in delayed {
            self.fence_variable(slot, Root::Delayed);
        }
        let inputs = self.fence_mut().inputs.clone();
        for &input in &inputs {
            if let Input::Variable(slot) = input {
                self.fence_variable(slot, Root::Input);
            }
        }
        let digests: Vec<_> = inputs.iter().map(|&input| self.input_digest(input)).collect();
        for (&input, digest) in inputs.iter().zip(&digests) {
            let mut item = self.input_json(input);
            match digest {
                Ok(digest) => {
                    item["digest"] = json!(digest);
                    host::record("input", item);
                }
                // Its later changes could not be found.
                Err(kind) => {
                    item["object"] = json!(kind);
                    host::refuse("undigestable", item);
                }
            }
        }
        self.fence_mut().digests = digests;
    }

    fn fence_mut(&mut self) -> &mut Fence {
        self.fence.as_mut().expect("a build's fence")
    }

    /// Put the variable at SLOT behind the fence as ROOT; a fenced one keeps
    /// its root. A buffer-local variable cannot be fenced and refuses the
    /// build; one is not observed.
    pub(crate) fn fence_variable(&mut self, slot: usize, root: Root) {
        self.forwarding(slot);
        let redirect = self.variable_redirects.get(slot).copied().flatten();
        match redirect {
            Some(VariableRedirect::Forwarded(Fwd::Fence(_))) => {}
            Some(VariableRedirect::Plain | VariableRedirect::Forwarded(_)) => {
                let older = (0..self.dynamic.len()).filter(|&at| self.dynamic[at].0 == slot).collect();
                self.fence_mut().cells.insert(slot, Cell { root, redirect, older });
                self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(Fwd::Fence(slot as u32)));
            }
            _ if root == Root::Observed => {}
            _ => host::refuse("unfenceable", json!({"root": root as u8, "symbol": self.symbol_names[slot]})),
        }
    }

    /// While startup runs, a variable whose redirect is first resolved is
    /// observed too.
    pub(crate) fn observe_new_variable(&mut self, slot: usize) {
        if self.fence.as_ref().is_some_and(|fence| fence.observing) {
            self.fence_variable(slot, Root::Observed);
        }
    }
}

fn access_name(access: Access) -> &'static str {
    match access {
        Access::Read => "read",
        Access::Write => "write",
        Access::SavedRead | Access::SavedWrite => "saved",
    }
}

impl Vm {
    fn fence_cell(&self, slot: usize) -> Option<&Cell> {
        self.fence.as_ref()?.cells.get(&slot)
    }

    /// The forwarding the fence keeps for SLOT, for the kind queries.
    pub(crate) fn fenced_forwarding(&self, slot: usize) -> Option<Fwd> {
        match self.fence_cell(slot)?.redirect {
            Some(VariableRedirect::Forwarded(fwd)) => Some(fwd),
            _ => None,
        }
    }

    /// F on the variable at SLOT as its own redirect has it, unchecked. The
    /// redirect F leaves is the one the fence keeps; one F made buffer-local
    /// (`make-local-variable`, already refused) leaves the fence. Any other
    /// variable is F's as it is.
    pub(crate) fn unfenced<T>(&mut self, slot: usize, f: impl FnOnce(&mut Vm) -> T) -> T {
        let Some(own) = self.fence_cell(slot).map(|cell| cell.redirect) else { return f(self) };
        let fenced = std::mem::replace(&mut self.variable_redirects[slot], own);
        let result = f(self);
        let own = self.variable_redirects[slot];
        if matches!(own, Some(VariableRedirect::Localized | VariableRedirect::LocalizedForwarded(_))) {
            self.fence_mut().cells.remove(&slot);
        } else {
            self.variable_redirects[slot] = fenced;
            if let Some(cell) = self.fence.as_mut().and_then(|fence| fence.cells.get_mut(&slot)) {
                cell.redirect = own;
            }
        }
        result
    }

    /// An ACCESS to the fenced variable at SLOT, then F through its own
    /// redirect.
    pub(crate) fn fenced<T>(&mut self, slot: usize, access: Access, f: impl FnOnce(&mut Vm) -> T) -> T {
        let buffer = self.slots[self.buffer_slot].value;
        self.fence_access(slot, access, buffer);
        self.unfenced(slot, f)
    }

    /// Refuse ACCESS to the fenced variable at SLOT in BUFFER, unless
    /// startup is running or a `let` newer than the fence holds the binding
    /// (`default-directory`'s in BUFFER). A saved value is always the root's.
    /// An input refuses writes only. While startup runs, a read of an
    /// observed variable makes it an input.
    pub(crate) fn fence_access(&mut self, slot: usize, access: Access, buffer: Value) {
        let Some(cell) = self.fence_cell(slot) else { return };
        let read = matches!(access, Access::Read | Access::SavedRead);
        if host::in_startup() {
            if cell.root == Root::Observed && read {
                self.observe(Input::Variable(slot));
            }
            return;
        }
        if (cell.root == Root::Input && read) || host::reseed_reads(&self.symbol_names[slot]) {
            return;
        }
        let per_buffer = matches!(cell.redirect, Some(VariableRedirect::Forwarded(Fwd::Buffer(_))));
        let saved = matches!(access, Access::SavedRead | Access::SavedWrite);
        let shadowed = !saved && self.dynamic.iter().enumerate().any(|(at, &(bound, _, owner, let_default))|
            bound == slot && !cell.older.contains(&at) && (!per_buffer || let_default || owner == buffer));
        if !shadowed {
            host::refuse("fence", json!({"root": cell.root as u8, "access": access_name(access),
                                         "symbol": self.symbol_names[slot]}));
        }
    }

    /// ACCESS to the value a `let` saved in dynamic entry AT, by
    /// `default-toplevel-value` and its kin. The oldest entry newer than the
    /// fence saved the root's value.
    pub(crate) fn fence_saved(&mut self, slot: usize, at: usize, access: Access) {
        if self.fence_cell(slot).is_some_and(|cell| !cell.older.contains(&at)) {
            self.fence_access(slot, access, Value::NIL);
        }
    }

    /// The fenced variable at SLOT lost its innermost binding, the dynamic
    /// entry past the stack's end.
    pub(crate) fn fence_unbound(&mut self, slot: usize) {
        let at = self.dynamic.len();
        if let Some(cell) = self.fence.as_mut().and_then(|fence| fence.cells.get_mut(&slot)) {
            cell.older.retain(|&older| older != at);
        }
    }

    /// A `let` of the fenced variable at SLOT is made or ends: its watchers
    /// would be handed the root's value, so outside startup a build refuses
    /// a watched root. An input's value may be read.
    pub(crate) fn fence_watched(&mut self, slot: usize) {
        if self.variable_trapped(slot) && !host::in_startup()
            && self.fence_cell(slot).is_some_and(|cell| !matches!(cell.root, Root::Observed | Root::Input))
        {
            host::refuse("fence", json!({"root": self.fence_cell(slot).unwrap().root as u8, "access": "watched",
                                         "symbol": self.symbol_names[slot]}));
        }
    }

    /// A backtrace's read of the binding of SLOT in dynamic entry AT,
    /// OWNER's, whether it is current or a newer `let` saved it. A binding
    /// older than the fence holds the root's value.
    pub(crate) fn fence_binding(&mut self, slot: usize, at: usize, owner: Value) {
        if self.fence_cell(slot).is_some_and(|cell| cell.older.contains(&at)) {
            self.fence_access(slot, Access::SavedRead, owner);
        }
    }

    fn observe(&mut self, input: Input) {
        let fence = self.fence_mut();
        if fence.seen.insert(input) {
            fence.inputs.push(input);
        }
    }

    /// While startup runs, `get` of SYMBOL's property KEY (None: its whole
    /// plist) is an input. After it, a write of one refuses.
    pub(crate) fn fence_property(&mut self, symbol: Value, key: Option<Value>, access: Access) {
        let Some(fence) = self.fence.as_ref() else { return };
        let (Some(slot), key) = (symbol.index_if(7).and_then(|i| i.checked_sub(1)), key.and_then(|k| k.index_if(7)).map(|i| i - 1)) else { return };
        if fence.observing {
            if access == Access::Read {
                self.observe(key.map_or(Input::Plist(slot), |key| Input::Property(slot, key)));
            }
            return;
        }
        let written = fence.seen.contains(&Input::Plist(slot)) || match key {
            Some(key) => fence.seen.contains(&Input::Property(slot, key)),
            None => fence.inputs.iter().any(|&input| matches!(input, Input::Property(s, _) if s == slot)),
        };
        if access == Access::Write && written {
            let property = key.map(|key| self.symbol_names[key].clone());
            host::refuse("fence", json!({"root": Root::Input as u8, "access": "write", "symbol": self.symbol_names[slot], "property": property}));
        }
    }

    /// A write of SLOT's function cell: an input's refuses.
    pub(crate) fn fence_function(&mut self, slot: usize) {
        if self.fence.as_ref().is_some_and(|fence| !fence.observing && fence.seen.contains(&Input::Function(slot))) {
            host::refuse("fence", json!({"root": Root::Input as u8, "access": "write", "symbol": self.symbol_names[slot], "function": true}));
        }
    }

    /// A call through SLOT, and every symbol its function cell names in turn.
    fn observe_function(&mut self, mut slot: usize) {
        for _ in 0..self.slots.len() {
            if self.fence_mut().seen.contains(&Input::Function(slot)) {
                return;
            }
            self.observe(Input::Function(slot));
            match self.slots[slot].function.index_if(7).and_then(|i| i.checked_sub(1)) {
                Some(next) => slot = next,
                None => return,
            }
        }
    }

    /// The content check that canonicalize will make: each input whose
    /// content changed since startup ended refuses, naming it. A mutation in
    /// place (`setcar`, `puthash`) has no write site; this finds it.
    pub fn world_check_inputs(&mut self) {
        let Some(fence) = self.fence.as_ref() else { return };
        let checks: Vec<(Input, String)> = fence.inputs.iter().copied().zip(fence.digests.iter().cloned())
            .filter_map(|(input, digest)| Some((input, digest.ok()?))).collect();
        for (input, digest) in checks {
            if self.input_digest(input).as_ref() != Ok(&digest) {
                let mut item = self.input_json(input);
                item["access"] = json!("changed");
                item["root"] = json!(Root::Input as u8);
                host::refuse("fence", item);
            }
        }
    }

    fn input_json(&self, input: Input) -> serde_json::Value {
        let name = |slot: usize| self.symbol_names[slot].clone();
        match input {
            Input::Variable(slot) => json!({"input": "variable", "symbol": name(slot)}),
            Input::Property(slot, key) => json!({"input": "property", "symbol": name(slot), "property": name(key)}),
            Input::Plist(slot) => json!({"input": "plist", "symbol": name(slot)}),
            Input::Function(slot) => json!({"input": "function", "symbol": name(slot)}),
        }
    }

    fn input_digest(&mut self, input: Input) -> Result<String, &'static str> {
        let value = match input {
            Input::Variable(slot) => self.unfenced(slot, |vm| vm.default_value(slot)).unwrap_or(Value::UNBOUND),
            Input::Property(slot, key) => self.plist_lookup(self.slots[slot].plist, Value::reference(key + 1, 7)),
            Input::Plist(slot) => self.slots[slot].plist,
            Input::Function(slot) => self.slots[slot].function,
        };
        self.content_digest(value)
    }

    /// SHA-256 of VALUE's content: the structure and bytes of the conses,
    /// strings with their text properties, vectors, records, bool vectors,
    /// char tables, syntax tables, hash tables (with their test, weakness,
    /// size, free list and each entry's stored hash), closures and cells it
    /// reaches, each once; a later reach names the first by its order. A
    /// stored hash that read a heap index digests as only that fact.
    /// Every object leads with its kind and every variable-length field with
    /// its length, so no two contents share a byte stream. Nothing depends
    /// on an arena index, which a collection moves. An object whose
    /// observable state this cannot read, such as a buffer, a marker or a
    /// process, is an error naming its kind.
    fn content_digest(&self, value: Value) -> Result<String, &'static str> {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        let mut seen = rustc_hash::FxHashMap::default();
        let mut stack = vec![value];
        while let Some(v) = stack.pop() {
            hasher.update(v.tag().to_le_bytes());
            if matches!(v.tag(), 0 | 3 | 4 | 5) {
                let order = seen.len();
                let first = *seen.entry(v.0).or_insert(order);
                hasher.update([u8::from(first != order)]);
                if first != order {
                    hasher.update(first.to_le_bytes());
                    continue;
                }
            }
            match (v.tag(), self.object_if(v)) {
                (3, _) => {
                    let (car, cdr) = self.cons_pair(v).expect("a cons");
                    stack.extend([cdr, car]);
                }
                (4, _) => {
                    let closure = &self.closures[v.payload() as usize];
                    hasher.update(closure.function.to_le_bytes());
                    hasher.update([closure.captures.len(), closure.metadata.len()].map(usize::to_le_bytes).concat());
                    stack.push(closure.document);
                    stack.extend(closure.metadata.iter().chain(&closure.captures).rev());
                }
                (5, _) => stack.push(self.read_capture_cell(v.payload() as usize)),
                (7, _) if v != Value::UNBOUND => {
                    let slot = v.payload() as usize - 1;
                    let name = &self.symbol_names[slot];
                    framed(&mut hasher, name.as_bytes());
                    let interned = self.names.get(name) == Some(&slot);
                    hasher.update([u8::from(interned)]);
                    if !interned {
                        hasher.update(v.0.to_le_bytes());
                    }
                }
                (_, Some(Object::String(text) | Object::RawString(text))) => {
                    hasher.update([b's', u8::from(text.is_multibyte())]);
                    framed(&mut hasher, text.bytes());
                    stack.extend(property_runs(&text.properties, text.bytes().len(), &mut hasher));
                }
                (_, Some(Object::Bytes(bytes))) => {
                    hasher.update([b'y']);
                    framed(&mut hasher, bytes);
                }
                (_, Some(Object::Float(f))) => {
                    hasher.update([b'f']);
                    hasher.update(f.to_bits().to_le_bytes());
                }
                (_, Some(Object::Bignum(n))) => {
                    hasher.update([b'n']);
                    framed(&mut hasher, &n.to_signed_bytes_le());
                }
                (_, Some(object @ (Object::Vector(items) | Object::Record(items)))) => {
                    hasher.update([if matches!(object, Object::Record(_)) { b'r' } else { b'v' }]);
                    hasher.update(items.len().to_le_bytes());
                    stack.extend(items.iter().rev());
                }
                (_, Some(Object::BoolVector(bits))) => {
                    hasher.update([b'b']);
                    framed(&mut hasher, bits.printed().as_bytes());
                }
                (_, Some(Object::CharTable(table))) => {
                    hasher.update([b'c']);
                    stack.extend(char_table_content(table, &mut hasher)?);
                }
                (_, Some(Object::SyntaxTable(entries))) => {
                    hasher.update([b't']);
                    hasher.update(entries.len().to_le_bytes());
                    for (&at, &class) in entries {
                        hasher.update(at.to_le_bytes());
                        hasher.update([class as u8]);
                    }
                }
                (_, Some(Object::HashTable(table))) => {
                    hasher.update([b'h', table.test as u8, table.weakness as u8]);
                    hasher.update([table.size(), table.count()].map(usize::to_le_bytes).concat());
                    let free: Vec<u8> = table.free_slots().iter().flat_map(|slot| slot.to_le_bytes()).collect();
                    framed(&mut hasher, &free);
                    for (slot, key, value) in table.entries() {
                        match table.content_hash(slot) {
                            Some(hash) => {
                                hasher.update([b'k']);
                                hasher.update(hash.to_le_bytes());
                            }
                            None => hasher.update([b'i']),
                        }
                        stack.extend([value, key]);
                    }
                }
                (_, Some(object)) => return Err(object_kind(object)),
                _ => {
                    hasher.update([b'_']);
                    hasher.update(v.0.to_le_bytes());
                }
            }
        }
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// A use of the random state (root 5).
    pub(crate) fn fence_random(&self) {
        if self.fence.as_ref().is_some_and(|fence| fence.random) && !host::in_startup() {
            host::refuse("fence", json!({"root": Root::Random as u8, "access": "read", "symbol": "random state"}));
        }
    }
}

/// BYTES into HASHER, after their length.
fn framed(hasher: &mut sha2::Sha256, bytes: &[u8]) {
    use sha2::Digest;
    hasher.update(bytes.len().to_le_bytes());
    hasher.update(bytes);
}

/// TEXT's property runs as Lisp sees them, their count and each run's
/// bounds and keys into HASHER: the property values, for the caller to
/// digest.
fn property_runs(properties: &crate::properties::Properties, len: usize, hasher: &mut sha2::Sha256) -> Vec<Value> {
    use crate::coordinates::ByteOffset;
    use sha2::Digest;
    let mut runs = Vec::new();
    let mut at = 0;
    while at < len && !properties.is_empty() {
        let end = properties.next_change(ByteOffset(at), ByteOffset(len)).0;
        runs.push((at, end, properties.properties_at(ByteOffset(at))));
        at = end;
    }
    hasher.update(runs.len().to_le_bytes());
    let mut values = Vec::new();
    for (at, end, properties) in runs {
        hasher.update([at, end, properties.len()].map(usize::to_le_bytes).concat());
        for (key, value) in properties {
            framed(hasher, key.as_bytes());
            values.push(value);
        }
    }
    values.reverse();
    values
}

/// TABLE's mapping, as `aref` reads it, ASCII slot included: the count of
/// extras and each run of characters that share a value, into HASHER; the
/// purpose, default, parent, extras and run values, for the caller to
/// digest.
fn char_table_content(table: &crate::char_table::CharTable, hasher: &mut sha2::Sha256) -> Result<Vec<Value>, &'static str> {
    use sha2::Digest;
    let mut values = vec![table.purpose, table.default, table.parent];
    hasher.update(table.extras.len().to_le_bytes());
    values.extend(&table.extras);
    let span = |at| table.span(at).map_err(|_| "char-table");
    let (mut from, (mut value, _, mut to)) = (0, span(0)?);
    while to + 1 < crate::char_table::CharTable::LEN {
        let (next, start, end) = span(to + 1)?;
        if next != value {
            hasher.update([from, to].map(usize::to_le_bytes).concat());
            values.push(value);
            (from, value) = (start, next);
        }
        to = end;
    }
    hasher.update([from, to].map(usize::to_le_bytes).concat());
    values.push(value);
    values.reverse();
    Ok(values)
}

/// The kind of an object whose content `content_digest` cannot read.
fn object_kind(object: &Object) -> &'static str {
    match object {
        Object::Window(_) => "window",
        Object::Frame(_) => "frame",
        Object::Terminal(_) => "terminal",
        Object::WindowConfiguration(_) => "window-configuration",
        Object::Sqlite(_) => "sqlite",
        Object::Process(_) => "process",
        Object::SubCharTable(_) => "sub-char-table",
        Object::MapKeymapCursor(_) => "keymap cursor",
        Object::Buffer(_) => "buffer",
        Object::Marker(_) => "marker",
        Object::Overlay(_) => "overlay",
        Object::ByteBuffer(_) => "byte buffer",
        Object::Obarray(_) => "obarray",
        _ => "object",
    }
}
