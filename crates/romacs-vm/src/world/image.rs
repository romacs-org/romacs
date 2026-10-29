//! A VM as a world image (`world::arena`): capture boxes the VM and writes
//! the arena, restore takes the box back from the mapped image, and
//! `world_reinit` rebuilds what a fresh process makes for itself (the
//! design's R fields, §3 and §4): the environment lists, the buffers'
//! default directories, the random state, the time-zone caches, the
//! embedder roots and the process-global counters. The startup steps then
//! run again in Lisp (`romacs-world-restart`).
//!
//! Capture is refused, naming the field, unless the VM is at rest (Q) and
//! holds nothing of the process beyond its heap (X): no host attachment,
//! child, job, code image or local time-zone rule.
use super::arena::{self, ImageFields};
use crate::{LocalZone, Value, Vm};
use std::sync::atomic::Ordering;

impl Vm {
    /// Why this VM cannot be captured, or Ok when it can.
    pub fn world_quiescent(&self) -> Result<(), String> {
        let checks: [(&str, bool); 17] = [
            ("stack", self.stack.is_empty()),
            ("dynamic", self.dynamic.is_empty()),
            ("eval_depth", self.eval_depth == 0),
            ("external_roots", self.external_roots.is_empty()),
            ("suspended_code", self.suspended_code.is_empty()),
            ("signal_handlers", self.signal_handlers.is_empty()),
            ("propagated_escape", self.propagated_escape.is_none()),
            ("host_input", self.host_input.is_none()),
            ("layout_host", self.layout_host.is_none()),
            ("terminal_host", self.terminal_host.is_none()),
            ("terminal_bindings", self.terminal_bindings.is_empty()),
            ("subprocesses", self.subprocesses.is_empty()),
            ("process_jobs", self.process_jobs.is_idle()),
            ("image", self.image.is_none() && self.functions.iter().all(|f| f.mapped_code.is_none())),
            ("local_zone", matches!(self.local_zone, LocalZone::Process)),
            ("fence", self.fence.is_none()),
            ("rooted", self.rooted.is_empty()),
        ];
        match checks.iter().find(|(_, ok)| !ok) {
            Some((field, _)) => Err(format!("world capture refused: {field} is not at rest")),
            None => Ok(()),
        }
    }

    /// What a fresh process builds for itself, rebuilt in a VM restored
    /// from an image. `Vm::new` builds the same from the same functions.
    pub fn world_reinit(&mut self) {
        // The environment lists, from this process's environment.
        self.environment_snapshot = Some(crate::environment::snapshot());
        self.initialize_environment_slot("process-environment");
        // GNU init_buffer: every buffer starts in the startup directory.
        let directory = {
            let current = self.slots[self.buffer_slot].value;
            self.slots[self.buffer_slot].value = Value::NIL;
            let directory = self.inherited_directory();
            self.slots[self.buffer_slot].value = current;
            directory
        };
        for buffer in 0..self.objects.len() {
            if matches!(&self.objects[buffer], crate::data::Object::Buffer(b) if b.state.live) {
                self.set_directory_of(buffer, directory);
            }
        }
        self.random = crate::random::Random::from_entropy();
        self.mktime_offset = 0;
        self.tz_rule_dstoff = 0;
        self.doc_file = None;
        self.idle_since = None;
        self.rooted = crate::handle::RootSet::default();
        self.process_jobs = Default::default();
        self.subprocesses = Default::default();
    }
}

/// Capture VM into FILE as an image keyed KEY: refused unless the VM is at
/// rest. The arena is this process's heap; nothing else allocates while the
/// image is written, and the process exits after it.
pub fn capture(vm: Vm, file: &std::fs::File, key: [u8; 32]) -> Result<usize, String> {
    vm.world_quiescent()?;
    let counters = [crate::expand::next_generated(), 0, 0, 0];
    let root = Box::into_raw(Box::new(vm)) as usize;
    let fields = ImageFields { len: 0, root, counters, layout: arena::layout(), key };
    let written = unsafe { arena::write_image(file, fields) }.map_err(|e| format!("world image: {e}"));
    // The VM stays allocated: the image owns it, and this process exits.
    written
}

/// The VM of the image this process mapped, when it was made for KEY,
/// with the process-global counters it depends on restored. Taken once.
pub fn restore(key: &[u8; 32]) -> Option<Vm> {
    use std::sync::atomic::AtomicBool;
    static TAKEN: AtomicBool = AtomicBool::new(false);
    let fields = arena::mapped_image().filter(|fields| fields.key == *key)?;
    if TAKEN.swap(true, Ordering::AcqRel) {
        return None;
    }
    crate::expand::raise_generated(fields.counters[0]);
    let vm = unsafe { Box::from_raw(fields.root as *mut Vm) };
    Some(*vm)
}
