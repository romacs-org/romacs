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
pub mod sandbox;

use crate::data::Object;
use crate::forwarded::Fwd;
use crate::{Value, VariableRedirect, Vm, host};
use serde_json::json;

/// A fenced value's root (§6.2). Root 4, the re-seed variables, arrives with
/// the re-seed table. `Observed` is no root: a variable startup may read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Root {
    Observed = 0,
    Environment = 1,
    Directory = 2,
    Delayed = 3,
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
    /// Startup is running: variables are wrapped and reads are inputs.
    observing: bool,
    /// The inputs in first-read order, each once.
    inputs: Vec<Input>,
    seen: rustc_hash::FxHashSet<Input>,
    /// Each input's content digest as startup ended.
    digests: Vec<String>,
}

/// A fenced variable: its root, the redirect the fence keeps, and the depth
/// of the binding stack when it was fenced, above which a `let` is newer.
struct Cell {
    root: Root,
    redirect: Option<VariableRedirect>,
    depth: usize,
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

    /// `romacs--startup-phase`: the startup function's run begins (ON) or
    /// ends. While it runs, its reads are observed. At its end the variables
    /// then on the delayed list are fenced, and writes to its inputs refuse.
    /// A no-op outside build mode.
    pub(crate) fn startup_phase(&mut self, on: bool) {
        if host::mode() != host::Mode::Build || !host::switch_startup(on) {
            return;
        }
        if on {
            let slot = self.slot("custom-delayed-init-variables");
            let mut tail = self.variable_value(slot).unwrap_or(Value::NIL);
            let mut delayed = Vec::new();
            while let Ok((symbol, rest)) = self.cons_pair(tail) {
                if let Some(slot) = symbol.index_if(7).and_then(|i| i.checked_sub(1)) {
                    delayed.push(self.indirect_variable(slot));
                }
                tail = rest;
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
        let digests: Vec<String> = inputs.iter().map(|&input| self.input_digest(input)).collect();
        for (&input, digest) in inputs.iter().zip(&digests) {
            let mut item = self.input_json(input);
            item["digest"] = json!(digest);
            host::record("input", item);
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
                let depth = self.dynamic.len();
                self.fence_mut().cells.insert(slot, Cell { root, redirect, depth });
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

    /// Whether the variable at SLOT is behind the fence.
    pub(crate) fn fenced_variable(&self, slot: usize) -> bool {
        matches!(self.variable_redirects.get(slot), Some(Some(VariableRedirect::Forwarded(Fwd::Fence(_)))))
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
        if cell.root == Root::Input && read {
            return;
        }
        let per_buffer = matches!(cell.redirect, Some(VariableRedirect::Forwarded(Fwd::Buffer(_))));
        let saved = matches!(access, Access::SavedRead | Access::SavedWrite);
        let shadowed = !saved && self.dynamic.iter().skip(cell.depth)
            .any(|&(bound, _, owner, let_default)| bound == slot && (!per_buffer || let_default || owner == buffer));
        if !shadowed {
            host::refuse("fence", json!({"root": cell.root as u8, "access": access_name(access),
                                         "symbol": self.symbol_names[slot]}));
        }
    }

    /// ACCESS to the value a `let` saved in dynamic entry AT, by
    /// `default-toplevel-value` and its kin. The oldest entry newer than the
    /// fence saved the root's value.
    pub(crate) fn fence_saved(&mut self, slot: usize, at: usize, access: Access) {
        if self.fence_cell(slot).is_some_and(|cell| at >= cell.depth) {
            self.fence_access(slot, access, Value::NIL);
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
        let checks: Vec<(Input, String)> = fence.inputs.iter().copied().zip(fence.digests.iter().cloned()).collect();
        for (input, digest) in checks {
            if self.input_digest(input) != digest {
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

    fn input_digest(&mut self, input: Input) -> String {
        let value = match input {
            Input::Variable(slot) => self.unfenced(slot, |vm| vm.default_value(slot)).unwrap_or(Value::UNBOUND),
            Input::Property(slot, key) => self.plist_lookup(self.slots[slot].plist, Value::reference(key + 1, 7)),
            Input::Plist(slot) => self.slots[slot].plist,
            Input::Function(slot) => self.slots[slot].function,
        };
        self.content_digest(value)
    }

    /// SHA-256 of VALUE's content: the structure and bytes of the conses,
    /// strings, vectors, records, hash tables, closures and cells it reaches,
    /// each once, and the kind of every other object. Nothing depends on an
    /// arena index, which a collection moves.
    fn content_digest(&self, value: Value) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        let mut seen = rustc_hash::FxHashSet::default();
        let mut stack = vec![value];
        while let Some(v) = stack.pop() {
            hasher.update(v.tag().to_le_bytes());
            if matches!(v.tag(), 0 | 3 | 4 | 5) && !seen.insert(v.0) {
                continue;
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
                    hasher.update(self.symbol_names[slot].as_bytes());
                    if self.names.get(&self.symbol_names[slot]) != Some(&slot) {
                        hasher.update(v.0.to_le_bytes());
                    }
                }
                (_, Some(Object::String(_) | Object::RawString(_))) => hasher.update(self.str_view(v).expect("a string").bytes),
                (_, Some(Object::Bytes(bytes))) => hasher.update(bytes),
                (_, Some(Object::Float(f))) => hasher.update(f.to_bits().to_le_bytes()),
                (_, Some(Object::Bignum(n))) => hasher.update(n.to_signed_bytes_le()),
                (_, Some(Object::Vector(items) | Object::Record(items))) => {
                    hasher.update(items.len().to_le_bytes());
                    stack.extend(items.iter().rev());
                }
                (_, Some(Object::HashTable(table))) => {
                    for (_, key, value) in table.entries() {
                        stack.extend([value, key]);
                    }
                }
                (_, Some(object)) => hasher.update(format!("{:?}", std::mem::discriminant(object))),
                _ => hasher.update(v.0.to_le_bytes()),
            }
        }
        hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A use of the random state (root 5).
    pub(crate) fn fence_random(&self) {
        if self.fence.as_ref().is_some_and(|fence| fence.random) && !host::in_startup() {
            host::refuse("fence", json!({"root": Root::Random as u8, "access": "read", "symbol": "random state"}));
        }
    }
}
