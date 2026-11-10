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
//! child, job, code image, lowered code, compiled pattern or local
//! time-zone rule, and no object holding a resource (`Object::
//! foreign_resource`). These are the fields and variants the VM's
//! `SnapshotSafe` listing names as at rest (`world::safe`).
use super::arena::{self, ImageFields};
use crate::{LocalZone, Value, Vm};
use std::sync::atomic::Ordering;

/// A VM a capture may write. Only `Vm::at_rest_for_image` makes one, and the
/// image's root is built from nothing else, so a capture that skipped it
/// would not compile.
struct AtRest(Vm);

/// The boxed root an image is written for: VM and what the embedder carries.
fn image_root<T>(vm: AtRest, carry: T) -> usize {
    Box::into_raw(Box::new((vm.0, carry))) as usize
}

impl Vm {
    /// All a capture asks of the VM before it writes it: at rest
    /// (`world_quiescent`), and its heap one old generation with no log,
    /// whatever was allocated or stored since the last collection, since to
    /// the process that restores the image all of it is old.
    fn at_rest_for_image(mut self) -> Result<AtRest, String> {
        self.world_quiescent()?;
        self.settle_generations();
        Ok(AtRest(self))
    }
    /// Why this VM cannot be captured, or Ok when it can.
    pub fn world_quiescent(&self) -> Result<(), String> {
        let checks: [(&str, bool); 43] = [
            ("stack", self.stack.is_empty()),
            ("site_leaf", self.site_leaf == (0, 0)),
            ("images", self.images.is_empty()),
            ("dynamic", self.dynamic.is_empty()),
            ("eval_depth", self.eval_depth == 0),
            ("external_roots", self.external_roots.is_empty()),
            ("suspended", self.suspended.is_empty()),
            ("signal_handlers", self.signal_handlers.is_empty()),
            ("propagated_escape", self.propagated_escape.is_none()),
            ("host_input", self.host_input.is_none()),
            ("host_wait", self.host_wait.is_none()),
            ("layout_host", self.layout_host.is_none()),
            ("display", self.display.is_none()),
            // Restored as the renderer's default, which the options in the
            // image must then also hold (rlisp/romacs-effects.el).
            ("visual", *self.visual == romacs_display_protocol::VisualConfig::default()),
            ("session_host", self.session_host.is_none()),
            ("terminal_host", self.terminal_host.is_none()),
            ("terminal_bindings", self.terminal_bindings.is_empty()),
            ("subprocesses", self.subprocesses.is_empty()),
            ("process_jobs", self.process_jobs.is_idle()),
            ("image", self.image.is_none() && self.functions.iter().all(|f| f.mapped_code.is_none())),
            // A frozen region is a mapping or a descriptor of this process.
            ("regions", self.regions_empty()),
            ("isolates", self.isolates.is_empty()),
            ("inotify", self.inotify.is_none()),
            ("local_zone", matches!(self.local_zone, LocalZone::Process)),
            ("fence", self.fence.is_none()),
            ("rooted", self.rooted.is_empty()),
            ("execution", self.execution.holds_nothing()),
            ("spare_programs", self.spare_programs.is_empty()),
            ("regexp_cache", self.regexp_cache.is_empty()),
            ("mode_line_forms", self.mode_line_forms.is_empty()),
            ("lambda_lists", self.lambda_lists.is_empty()),
            ("status_bar_line", self.status_bar_line.is_none()),
            ("idle_since", self.idle_since.is_none()),
            ("profiler", self.profiler.is_idle()),
            // A thread of this process: the running parse of a visit.
            ("treesit_early", crate::treesit::early_idle() && !self.objects.iter().any(|o| matches!(o, crate::data::Object::Buffer(b) if b.treesit.early.is_some()))),
            ("suspended", self.suspended.is_empty()),
            // The collector's reentry state: no engine running or suspended,
            // no frame holding Values out of its sight.
            ("suspended_stacks", self.suspended_stacks.is_empty()),
            ("running_engines", self.running_engines == 0),
            ("gc_holds", self.gc_holds == 0),
            ("unwinding", self.unwinding == 0),
            ("rooted_write", !self.rooted_write),
            ("deferred_writes", self.deferred_writes.is_empty()),
            ("unclaimed_caches", self.request_code.unclaimed_caches_empty()),
        ];
        if let Some((field, _)) = checks.iter().find(|(_, ok)| !ok) {
            return Err(format!("world capture refused: {field} is not at rest"));
        }
        match self.objects.iter().enumerate().find_map(|(at, object)| Some((at, object.foreign_resource()?))) {
            Some((at, what)) => Err(format!("world capture refused: object {at} is {what}, a resource of this process")),
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
        self.gc_stress.rearm();
        self.reset_collection_counters();
    }
}

/// Capture VM, and CARRY, what the embedder keeps beside it, into FILE as an
/// image keyed KEY: refused unless the VM is at rest. The arena is this
/// process's heap; nothing else allocates while the image is written, and
/// the process exits after it. CARRY travels as it is, unencoded: the key
/// binds the embedder, so its restorer knows CARRY's type.
pub fn capture<T: super::safe::SnapshotSafe + 'static>(vm: Vm, carry: T, file: &std::fs::File, key: [u8; 32]) -> Result<usize, String> {
    let vm = vm.at_rest_for_image()?;
    let [serials, writes] = crate::char_table::image_counters();
    let counters = [crate::expand::next_generated(), serials, writes, 0];
    let root = image_root(vm, carry);
    let fields = ImageFields { len: 0, root, counters, executable: arena::executable_base(), key };
    let written = unsafe { arena::write_image(file, fields) }.map_err(|e| format!("world image: {e}"));
    // The VM stays allocated: the image owns it, and this process exits.
    written
}

/// The VM and carry of the image this process mapped, when it was made for
/// KEY, with the process-global counters it depends on restored. Taken once.
///
/// T must be the type `capture` carried: KEY binds the embedder that made it.
pub fn restore<T: super::safe::SnapshotSafe + 'static>(key: &[u8; 32]) -> Option<(Vm, T)> {
    use std::sync::atomic::AtomicBool;
    static TAKEN: AtomicBool = AtomicBool::new(false);
    let fields = arena::mapped_image().filter(|fields| fields.key == *key)?;
    if TAKEN.swap(true, Ordering::AcqRel) {
        return None;
    }
    crate::expand::raise_generated(fields.counters[0]);
    crate::char_table::restore_counters([fields.counters[1], fields.counters[2]]);
    let root = unsafe { Box::from_raw(fields.root as *mut (Vm, T)) };
    Some(*root)
}

#[cfg(test)]
mod tests {
    use crate::{Value, Vm};

    /// What a capture writes is one old generation: an entry made and a
    /// store since the last collection are settled before the write. The
    /// image's root takes only what `at_rest_for_image` returns.
    #[test]
    fn a_capture_settles_the_generations_before_it_writes() {
        use crate::hash::{HashTable, Test, Weakness};
        let mut vm = Vm::new();
        let old = vm.alloc_cons(Value::NIL, Value::NIL);
        let table = vm.allocate_object(crate::data::Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::None, 8))));
        let mut roots = [old, table];
        vm.collect(&mut roots);
        assert_eq!(roots[1].index_if(0), Some(1), "the table is the first object");
        assert!(vm.generations_settled());
        // Each alone would leave the heap unsettled: a young entry, a dirty
        // old one, and a log.
        let unsettle: [fn(&mut Vm, Value); 3] = [
            |vm, _| { vm.alloc_cons(Value::NIL, Value::NIL); },
            |vm, old| vm.conses.set_car(old.index_if(3).unwrap(), Value::TRUE),
            |vm, old| {
                let (_, mut stores) = vm.objects.hash_mut(0).expect("the old table");
                stores.stored(old);
            },
        ];
        for change in unsettle {
            change(&mut vm, roots[0]);
            assert!(!vm.generations_settled());
            vm = vm.at_rest_for_image().expect("a VM at rest").0;
            assert!(vm.generations_settled());
        }
    }

    #[test]
    fn the_collectors_reentry_state_is_at_rest_or_refused() {
        let refusal = |change: fn(&mut Vm)| {
            let mut vm = Vm::new();
            assert_eq!(vm.world_quiescent(), Ok(()));
            change(&mut vm);
            vm.world_quiescent().unwrap_err()
        };
        assert!(refusal(|vm| vm.suspended_stacks.push(vec![])).contains("suspended_stacks"));
        assert!(refusal(|vm| vm.running_engines = 1).contains("running_engines"));
        assert!(refusal(|vm| vm.gc_holds = 1).contains("gc_holds"));
        assert!(refusal(|vm| vm.rooted_write = true).contains("rooted_write"));
        assert!(refusal(|vm| vm.deferred_writes.push((crate::watchers::NativeWrite::set(0, crate::Value::NIL), crate::Value::NIL))).contains("deferred_writes"));
        // A cache slot a builder reserved that no installed code owns.
        assert!(refusal(|vm| { vm.install_cache(); }).contains("unclaimed_caches"));
    }

    #[test]
    fn a_collection_reclaims_an_unclaimed_cache_and_the_vm_is_at_rest_again() {
        let mut vm = Vm::new();
        vm.install_cache();
        assert!(vm.world_quiescent().unwrap_err().contains("unclaimed_caches"));
        vm.collect(&mut []);
        assert_eq!(vm.world_quiescent(), Ok(()));
    }
}
