//! The UI a host attaches to a VM: its hosts, held on the host's thread
//! and never in the `Vm`, so a `Vm` stays `Send` while the hosts (trait
//! objects, callbacks) never cross threads (lanes/s2-send S8-DESIGN).
//!
//! A host's thread keeps one entry per VM in [`UIS`], under a key the VM
//! takes at its first attach ([`UiKey`]). The VM keeps only the key: a VM
//! on another thread, or a second VM on this one, finds no entry and runs
//! headless, as batch does. A setter given `None`, dropping its host, is a
//! detach: the last one removes the entry and the key. A key is at rest
//! (`world_quiescent` requires none), so a restored VM never names another
//! VM's entry.
use crate::Vm;
use crate::display_host::{Display, DisplayHost};
use crate::layout_host::LayoutHost;
use crate::session_host::SessionHost;
use crate::terminal_host::TerminalHost;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The hosts a VM's UI owner attached. Not `Send`, by its contents.
#[derive(Default)]
pub(crate) struct Ui {
    pub(crate) layout_host: Option<Box<dyn LayoutHost>>,
    pub(crate) session_host: Option<SessionHost>,
    /// The window system, closed until the session opens it.
    pub(crate) display: Option<Display>,
    pub(crate) terminal_host: Option<Box<dyn TerminalHost>>,
}

impl Ui {
    /// Whether no host is attached.
    fn is_empty(&self) -> bool {
        self.layout_host.is_none() && self.session_host.is_none() && self.display.is_none() && self.terminal_host.is_none()
    }
}

/// An entry, shared between the thread's map and a borrow in progress: the
/// map's borrow ends before an entry's starts, so a host that runs Lisp in
/// another VM reaches that VM's entry, never this one's borrow.
// CELL(Plain): hosts, no Lisp value.
type UiEntry = Rc<RefCell<Ui>>;

thread_local! {
    /// This thread's UIs, by VM key.
    static UIS: RefCell<HashMap<u64, UiEntry>> = RefCell::new(HashMap::new());
}

/// Keys never reused in the process.
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// A VM's key in its UI thread's map. Dropping it, with the VM, drops the
/// entry and its hosts when dropped on that thread; elsewhere the entry
/// goes with its thread.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UiKey(u64);

impl UiKey {
    fn new() -> Self {
        Self(NEXT_KEY.fetch_add(1, Ordering::Relaxed))
    }
}

impl Drop for UiKey {
    fn drop(&mut self) {
        // The entry leaves the map first, and its hosts drop after the map's
        // borrow ends.
        let entry = UIS.try_with(|uis| uis.borrow_mut().remove(&self.0)).ok().flatten();
        drop(entry);
    }
}

/// `Send`, as a `Vm` moving to another thread needs its UI key to be.
const _: fn() = || { fn send<T: Send>() {} send::<UiKey>(); };

impl Vm {
    /// This VM's entry on this thread, if it has one.
    fn ui_lookup(&self) -> Option<UiEntry> {
        let key = self.ui_key.as_ref()?.0;
        UIS.with(|uis| uis.borrow().get(&key).cloned())
    }

    /// `ui_lookup` for a host's use: a key with no entry here is a VM that
    /// moved with its hosts attached.
    fn ui_entry(&self) -> Option<UiEntry> {
        let entry = self.ui_lookup();
        debug_assert!(entry.is_some() || self.ui_key.is_none(), "a VM with hosts attached moved to another thread: detach them first");
        entry
    }

    /// F on this VM's UI, None when it has none here. F has no `Vm`.
    pub(crate) fn ui<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let entry = self.ui_entry()?;
        let mut ui = entry.borrow_mut();
        Some(f(&mut ui))
    }

    /// F on this VM's UI for a read that runs no host code.
    pub(crate) fn ui_ref<R>(&self, f: impl FnOnce(&Ui) -> R) -> Option<R> {
        let entry = self.ui_entry()?;
        let ui = entry.borrow();
        Some(f(&ui))
    }

    /// This VM's entry, entered on this thread at its first attach.
    fn ui_attach(&mut self) -> UiEntry {
        if let Some(entry) = self.ui_entry() {
            return entry;
        }
        let key = UiKey::new();
        let entry = UiEntry::default();
        UIS.with(|uis| uis.borrow_mut().insert(key.0, entry.clone()));
        self.ui_key = Some(key);
        entry
    }

    /// After a detach: the entry and the key go once no host is attached,
    /// and a key with no entry on this thread (a VM moved with its hosts)
    /// goes too.
    pub(crate) fn ui_settle(&mut self) {
        if self.ui_lookup().is_none_or(|entry| entry.borrow().is_empty()) {
            self.ui_key = None;
        }
    }

    /// Put HOST in the slot SLOT names on this VM's UI, attaching the UI
    /// for a host and settling it for none; the host replaced drops after
    /// the entry's borrow ends and the UI has settled, so a host whose drop
    /// panics leaves no empty entry behind.
    pub(crate) fn ui_set<T>(&mut self, host: Option<T>, slot: impl FnOnce(&mut Ui) -> &mut Option<T>) {
        let old = match host {
            Some(host) => slot(&mut self.ui_attach().borrow_mut()).replace(host),
            None => self.ui_lookup().and_then(|entry| slot(&mut entry.borrow_mut()).take()),
        };
        self.ui_settle();
        drop(old);
    }

    /// F on the open display's host, None while the frame is on the
    /// initial terminal.
    pub(crate) fn with_window_system_host<R>(&mut self, f: impl FnOnce(&mut (dyn DisplayHost + 'static)) -> R) -> Option<R> {
        self.ui(|ui| ui.display.as_mut().filter(|display| display.open).map(|display| f(display.host.as_mut()))).flatten()
    }

    /// F on the attached display, open or not.
    pub(crate) fn with_display<R>(&mut self, f: impl FnOnce(&mut Display) -> R) -> Option<R> {
        self.ui(|ui| ui.display.as_mut().map(f)).flatten()
    }

    /// F on the terminal service, or the error a terminal command gives
    /// without one.
    pub(crate) fn with_terminal_host<R>(&mut self, f: impl FnOnce(&mut (dyn TerminalHost + 'static)) -> R) -> Result<R, String> {
        self.ui(|ui| ui.terminal_host.as_mut().map(|host| f(host.as_mut()))).flatten().ok_or_else(|| "No terminal host attached".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_layout_input::{FrameSnapshot, MeasureInput, Measured, WindowEnd};

    struct NoLayout;
    impl LayoutHost for NoLayout {
        fn measure(&mut self, _: &MeasureInput) -> Result<Measured, String> { Err("none".into()) }
        fn lay_out(&mut self, _: &FrameSnapshot) -> Result<Vec<WindowEnd>, String> { Err("none".into()) }
    }

    fn entries() -> usize {
        UIS.with(|uis| uis.borrow().len())
    }

    #[test]
    fn a_host_attaches_on_its_thread_and_detaching_the_last_drops_the_entry() {
        let mut vm = Vm::new();
        let before = entries();
        assert!(vm.ui_key.is_none());
        vm.set_layout_host(Some(Box::new(NoLayout)));
        assert!(vm.has_layout_host() && vm.ui_key.is_some());
        assert_eq!(entries(), before + 1);
        vm.set_session_host(Some(Box::new(|_| -> std::convert::Infallible { unreachable!("not ended") })));
        vm.set_layout_host(None);
        assert!(!vm.has_layout_host() && vm.ui_key.is_some(), "the session host keeps the entry");
        vm.set_session_host(None);
        assert!(vm.ui_key.is_none(), "the last detach removes the key");
        assert_eq!(entries(), before);
    }

    #[test]
    fn two_vms_on_one_thread_keep_their_own_hosts() {
        let (mut a, mut b, c) = (Vm::new(), Vm::new(), Vm::new());
        a.set_layout_host(Some(Box::new(NoLayout)));
        b.set_layout_host(Some(Box::new(NoLayout)));
        b.set_layout_host(None);
        assert!(a.has_layout_host(), "B's detach leaves A's host");
        assert!(!b.has_layout_host() && b.ui_key.is_none());
        assert!(!c.has_layout_host() && c.ui_key.is_none());
    }

    #[test]
    fn dropping_a_vm_drops_its_entry() {
        let before = entries();
        let mut vm = Vm::new();
        vm.set_layout_host(Some(Box::new(NoLayout)));
        assert_eq!(entries(), before + 1);
        drop(vm);
        assert_eq!(entries(), before);
    }

    #[test]
    fn a_host_taken_out_and_put_back_keeps_the_key() {
        let mut vm = Vm::new();
        vm.set_layout_host(Some(Box::new(NoLayout)));
        let host = vm.ui(|ui| ui.layout_host.take()).flatten();
        assert!(host.is_some() && vm.ui_key.is_some(), "a loan is no detach");
        vm.ui(|ui| ui.layout_host = host);
        assert!(vm.has_layout_host());
    }

    #[test]
    fn a_vm_with_a_ui_is_not_at_rest() {
        let mut vm = Vm::new();
        vm.set_layout_host(Some(Box::new(NoLayout)));
        assert!(vm.world_quiescent().is_err_and(|why| why.contains("ui_key")));
        vm.set_layout_host(None);
        assert!(vm.world_quiescent().err().is_none_or(|why| !why.contains("ui_key")));
    }
}
