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
//! A fenced variable's redirect becomes `Forwarded(Fwd::Fence(slot))`, so
//! every access already takes the forwarded paths, whose `Fence` arms check
//! the fence and act through the redirect the fence keeps. No path of an
//! unfenced variable changes.
use crate::forwarded::Fwd;
use crate::{Value, VariableRedirect, Vm, host};
use serde_json::json;

/// A fenced value's root (§6.2). Root 4, the re-seed variables, arrives with
/// the re-seed table.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Root {
    Environment = 1,
    Directory = 2,
    Delayed = 3,
    Random = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
    /// Of the value a `let` saved.
    Saved,
}

/// The P fence of a build.
#[derive(Default)]
pub(crate) struct Fence {
    cells: rustc_hash::FxHashMap<usize, Cell>,
    random: bool,
    /// The delayed list as startup began.
    delayed: Vec<usize>,
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
    /// ends. At its end the variables then on the delayed list are fenced.
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
        } else {
            for slot in std::mem::take(&mut self.fence_mut().delayed) {
                self.fence_variable(slot, Root::Delayed);
            }
        }
    }

    fn fence_mut(&mut self) -> &mut Fence {
        self.fence.as_mut().expect("a build's fence")
    }

    /// Put the variable at SLOT behind the fence as ROOT. A buffer-local
    /// variable cannot be fenced and refuses the build.
    fn fence_variable(&mut self, slot: usize, root: Root) {
        self.forwarding(slot);
        let redirect = self.variable_redirects.get(slot).copied().flatten();
        match redirect {
            Some(VariableRedirect::Forwarded(Fwd::Fence(_))) => {}
            Some(VariableRedirect::Plain | VariableRedirect::Forwarded(_)) => {
                let depth = self.dynamic.len();
                self.fence_mut().cells.insert(slot, Cell { root, redirect, depth });
                self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(Fwd::Fence(slot as u32)));
            }
            _ => host::refuse("unfenceable", json!({"root": root as u8, "symbol": self.symbol_names[slot]})),
        }
    }

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
    pub(crate) fn fence_access(&self, slot: usize, access: Access, buffer: Value) {
        let Some(cell) = self.fence_cell(slot) else { return };
        if host::in_startup() {
            return;
        }
        let per_buffer = matches!(cell.redirect, Some(VariableRedirect::Forwarded(Fwd::Buffer(_))));
        let shadowed = access != Access::Saved && self.dynamic.iter().skip(cell.depth)
            .any(|&(bound, _, owner, let_default)| bound == slot && (!per_buffer || let_default || owner == buffer));
        if !shadowed {
            host::refuse("fence", json!({"root": cell.root as u8, "access": format!("{access:?}").to_lowercase(),
                                         "symbol": self.symbol_names[slot]}));
        }
    }

    /// The value a `let` saved in dynamic entry AT, read or written by
    /// `default-toplevel-value` and its kin. The oldest entry newer than the
    /// fence saved the root's value.
    pub(crate) fn fence_saved(&self, slot: usize, at: usize) {
        if self.fence_cell(slot).is_some_and(|cell| at >= cell.depth) {
            self.fence_access(slot, Access::Saved, Value::NIL);
        }
    }

    /// A use of the random state (root 5).
    pub(crate) fn fence_random(&self) {
        if self.fence.as_ref().is_some_and(|fence| fence.random) && !host::in_startup() {
            host::refuse("fence", json!({"root": Root::Random as u8, "access": "read", "symbol": "random state"}));
        }
    }
}
