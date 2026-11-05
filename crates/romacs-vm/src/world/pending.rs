//! Pending variables: a restored world's re-seed definitions that wait for
//! their variable's first access (`romacs--world-pend`).
//!
//! An init layer holds libraries the init only declared, which GNU loads at
//! their first use. A definition of theirs that reads a per-process fact
//! (`romacs--world-reseed-forms`), such as a `defcustom` asking a program
//! for its version, is what GNU evaluates at that use; evaluating it at
//! every start would make each start pay for a library it may never use.
//! Restore leaves such a definition pending instead
//! (`romacs--world-reseed-lazy-p`), and its variable's first access
//! evaluates it (`romacs--world-force`), then proceeds as that access.
//!
//! A pending variable's redirect is `Forwarded(Fwd::Fence(slot))`, the
//! fence's own: every access of a fenced variable already takes the
//! forwarded paths, and each reaches `fence_access` or `unfenced`, which
//! find no fence cell and force it here. A plain variable's access never
//! looks at this table. Forcing restores the redirect the variable had,
//! so every later access is a plain one.
//!
//! Restore makes every lazy definition pending again, so a process
//! restored from an image saved after a force (a session image, an
//! isolate's world) evaluates it again, in that process, as a fresh process
//! would: no host value is ever restored as current.
use crate::forwarded::Fwd;
use crate::{Value, VariableRedirect, Vm};

/// A pending variable: the redirect it had, and its entry in
/// `romacs--world-pending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pending {
    redirect: Option<VariableRedirect>,
    entry: u32,
}
romacs_snapshot::snapshot_struct!(Pending { redirect, entry });

impl Vm {
    /// Make the variable at SLOT wait for ENTRY's evaluation until its
    /// first access. False, and nothing changes, when it cannot: in a world
    /// build, whose fence owns `Fwd::Fence`, or for a variable that is
    /// buffer-local, forwarded or bound by a `let` now. One pending already
    /// is pending on ENTRY.
    pub(crate) fn pend_variable(&mut self, slot: usize, entry: u32) -> bool {
        if self.fence.is_some() {
            return false;
        }
        self.forwarding(slot);
        let redirect = self.variable_redirects.get(slot).copied().flatten();
        match redirect {
            Some(VariableRedirect::Forwarded(Fwd::Fence(_))) => match self.pending.get_mut(&slot) {
                Some(pending) => { pending.entry = entry; true }
                None => false,
            },
            None | Some(VariableRedirect::Plain) if !self.dynamic.iter().any(|binding| binding.0 == slot) => {
                if self.variable_redirects.len() <= slot {
                    self.variable_redirects.resize(slot + 1, None);
                }
                self.pending.insert(slot, Pending { redirect, entry });
                self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(Fwd::Fence(slot as u32)));
                true
            }
            _ => false,
        }
    }

    /// The variable at SLOT is accessed with no fence cell: when it is
    /// pending, its redirect comes back and its definition is evaluated
    /// (`romacs--world-force`), with collection held, since the access may
    /// come from a native frame holding Values in its locals. An error is
    /// the definition's own to report.
    pub(crate) fn force_pending(&mut self, slot: usize) {
        if self.pending.is_empty() {
            return;
        }
        let Some(pending) = self.pending.remove(&slot) else { return };
        self.variable_redirects[slot] = pending.redirect;
        let function = self.intern("romacs--world-force");
        let Ok(index) = Value::integer(i64::from(pending.entry)) else { return };
        self.gc_holds += 1;
        self.safe_call(function, &[index]);
        self.gc_holds -= 1;
    }

    /// Whether the variable at SLOT is pending.
    pub fn variable_pending(&self, slot: usize) -> bool {
        self.pending.contains_key(&slot)
    }
}
