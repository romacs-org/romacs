//! A VM as a world image (`world::arena`): capture boxes the VM and writes
//! the arena, restore takes the box back from the mapped image, and
//! `world_reinit` rebuilds what a fresh process makes for itself (the
//! design's R fields, §3 and §4): the environment lists, the buffers'
//! default directories, the random state, the time-zone caches, the
//! embedder roots and the process-global counters. The startup steps then
//! run again in Lisp (`romacs-world-restart`).
//!
//! No image is captured, naming the field, unless the VM is at rest (Q) and
//! holds nothing of the process beyond its heap (X): no host attachment,
//! child, job, code image, lowered code, compiled pattern or local
//! time-zone rule, and no object holding a resource (`Object::
//! foreign_resource`). These are the fields and variants the VM's
//! `SnapshotSafe` listing names as at rest (`world::safe`).
use super::arena::{self, ImageFields};
use super::safe::Schema;
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
        let checks: [(&str, bool); 45] = [
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
            // image must then also hold (lisp/romacs/romacs-effects.el).
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
            ("keeper", self.keeper.is_none()),
            ("inotify", self.inotify.is_none()),
            ("local_zone", matches!(self.local_zone, LocalZone::Process)),
            ("tracking", self.tracking.is_none()),
            ("rooted", self.rooted.is_empty()),
            ("execution", self.execution.holds_nothing()),
            ("spare_programs", self.spare_programs.is_empty()),
            ("regexp_cache", self.regexp_cache.is_empty()),
            ("flat_tables", self.flat_tables.is_empty()),
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
            return Err(format!("no world captured: {field} is not at rest"));
        }
        match self.objects.iter().enumerate().find_map(|(at, object)| Some((at, object.foreign_resource()?))) {
            Some((at, what)) => Err(format!("no world captured: object {at} is {what}, a resource of this process")),
            None => Ok(()),
        }
    }

    /// What a fresh process builds for itself, rebuilt in a VM restored
    /// from an image. `Vm::new` builds the same from the same functions.
    pub fn world_reinit(&mut self) {
        // The environment lists, from this process's environment.
        self.environment_snapshot = Some(crate::environment::snapshot());
        // HOST(slot:process-environment): rebuilds the environment lists from this process
        // environment after restore, as Vm::new builds them
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
        self.rooted = crate::handle::RootSet::default();
        self.gc_stress.rearm();
        self.reset_collection_counters();
        self.forget_system_name();
    }
}

/// Capture VM, and CARRY, what the embedder keeps beside it, into FILE as an
/// image keyed KEY: none unless the VM is at rest. The arena is this
/// process's heap; nothing else allocates while the image is written, and
/// the process exits after it. CARRY travels as it is, unencoded: the key
/// binds the embedder, so its restorer knows CARRY's type.
pub fn capture<T: super::safe::SnapshotSafe + super::safe::Schema>(vm: Vm, mut carry: T, file: &std::fs::File, key: [u8; 32]) -> Result<usize, String> {
    let mut vm = vm.at_rest_for_image()?;
    settle(&mut vm.0, &mut carry)?;
    let [serials, writes] = crate::char_table::image_counters();
    let counters = [crate::expand::next_generated(), serials, writes, 0];
    let root = image_root(vm, carry);
    let fields = ImageFields { len: 0, root, counters, executable: arena::executable_base(), key };
    let written = unsafe { arena::write_image(file, fields) }.map_err(|e| format!("world image: {e}"))?;
    // The VM stays allocated: the image owns it, and this process exits.
    whole_in_memory(file, written).map(|()| written)
}

/// tmpfs's `f_type` (linux/magic.h).
const TMPFS_MAGIC: u32 = 0x0102_1994;

/// The image FILE of LEN bytes, its holes filled where it lies in memory
/// (tmpfs). There a hole is a page the filesystem makes on first touch,
/// and a process that maps the image and touches one while the
/// filesystem is full dies of SIGBUS; filled here, a full filesystem fails
/// this build with ENOSPC instead, as any failed write does. On a disk
/// the holes stay holes.
fn whole_in_memory(file: &std::fs::File, len: usize) -> Result<(), String> {
    // HOST(rustix::file): the type of filesystem of the image file the builder opened through host
    let in_memory = rustix::fs::fstatfs(file).map_err(|e| format!("world image: {e}"))?.f_type;
    if u32::try_from(in_memory).ok() != Some(TMPFS_MAGIC) {
        return Ok(());
    }
    rustix::fs::fallocate(file, rustix::fs::FallocateFlags::empty(), 0, len as u64)
        .map_err(|e| format!("world image in memory (tmpfs): {e}"))
}

/// Make VM and CARRY hold nothing of the executable (`romacs_snapshot::
/// Settle`): every empty hash table they hold gets an allocation of its
/// own, so an image restores wherever this process's executable lay. None,
/// naming the type, when a table out of reach is empty.
fn settle<T: super::safe::Schema>(vm: &mut Vm, carry: &mut T) -> Result<(), String> {
    let mut settle = romacs_snapshot::Settle::default();
    vm.settle(&mut settle);
    carry.settle(&mut settle);
    match settle.unsettled() {
        [] => Ok(()),
        held => Err(format!("no world captured: an empty hash table, out of reach, in {}", held.join(", "))),
    }
}

/// The census of VM and CARRY as an image holds them
/// (`romacs_snapshot::Walk`): every kind with its count, bytes and layout;
/// the values that name something by an ordinal of this build's tables,
/// which another build numbers otherwise; and, in a process that mapped
/// the image, its arena's own counts of the bytes in use and free, against
/// which the walk's owned bytes show how much of the image is the values
/// it holds. The ordinals noted are the types `ordinals` names: a type that
/// comes to hold another is noted when it is added there.
#[must_use]
pub fn census<T: super::safe::Schema>(vm: &Vm, carry: &T) -> serde_json::Value {
    let mut observe = |value: &dyn std::any::Any, notes: &mut romacs_snapshot::Notes| ordinals(value, notes);
    let mut walk = romacs_snapshot::Walk::new(&mut observe);
    walk.value(vm);
    walk.value(carry);
    let mut census = walk.census();
    census["image"] = arena::image_census().map_or(serde_json::Value::Null, |arena| serde_json::json!({
        "used": arena.used, "small_free": arena.small_free, "slab_rest": arena.slab_rest,
        "runs_kept": arena.runs_kept, "runs_released": arena.runs_released,
    }));
    census
}

/// Count VALUE's ordinals of this build's tables into NOTES.
fn ordinals(value: &dyn std::any::Any, notes: &mut romacs_snapshot::Notes) {
    if let Some(value) = value.downcast_ref::<Value>() {
        if value.index_if(6).is_some() {
            notes.add("ordinal: a primitive, by its index in the primitive table", 1);
        }
    } else if value.downcast_ref::<crate::forwarded::Fwd>().is_some() {
        notes.add("ordinal: a forwarded variable, by its place in C_VARIABLES", 1);
    } else if let Some(slots) = value.downcast_ref::<crate::forwarded::PerBuffer>() {
        notes.add("ordinal: a buffer's per-buffer slots, by forwarded place", slots.slots() as u64);
    } else if value.downcast_ref::<crate::properties::Key>().is_some_and(|key| key.is_builtin()) {
        notes.add("ordinal: a built-in property key, at its fixed symbol slot", 1);
    } else if value.downcast_ref::<crate::PendingBody>().is_some() {
        notes.add("ordinal: a pending compiled body, its primitives as raw bits", 1);
    }
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
    let mut root = unsafe { Box::from_raw(fields.root as *mut (Vm, T)) };
    // The at-rest fields are this process's to make, before anything runs.
    // Only the VM, the image's root, has them (`snapshot_fields!`).
    root.0.renew();
    // With the other counters of the process: what the VM stores from now
    // on is stamped at or above the epoch its last collection ended at.
    crate::epoch::raise(root.0.allocation.since);
    Some(*root)
}

#[cfg(test)]
mod tests {
    use crate::{Value, Vm};

    /// A sparse image in memory (tmpfs) comes out whole, every page its
    /// own, so no later touch of it needs the filesystem's room; on another
    /// filesystem its holes stay. Each is tried where one is at hand.
    #[test]
    fn an_image_in_memory_has_no_holes_and_one_elsewhere_keeps_them() {
        use std::os::unix::fs::MetadataExt;
        let len = 16 << 20;
        // (whether it is in memory, the bytes allocated once filled) of a
        // sparse file made in DIR, or None where none can be made.
        let tried = |dir: &std::path::Path| {
            let path = dir.join(format!("romacs-image-holes-{}", std::process::id()));
            let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).ok()?;
            let in_memory = rustix::fs::fstatfs(&file).is_ok_and(|fs| u32::try_from(fs.f_type).ok() == Some(super::TMPFS_MAGIC));
            let filled = file.set_len(len as u64).map_err(|e| e.to_string()).and_then(|()| super::whole_in_memory(&file, len));
            let allocated = std::fs::metadata(&path).map(|m| m.blocks() * 512);
            let _ = std::fs::remove_file(&path);
            filled.unwrap();
            Some((in_memory, allocated.unwrap()))
        };
        let found: Vec<(bool, u64)> = [std::path::PathBuf::from("/dev/shm"), std::env::temp_dir(), std::path::PathBuf::from("/var/tmp")]
            .iter().filter_map(|dir| tried(dir)).collect();
        if let Some((_, allocated)) = found.iter().find(|(in_memory, _)| *in_memory) {
            assert!(*allocated >= len as u64, "an image on tmpfs kept holes: {allocated} of {len} bytes");
        }
        if let Some((_, allocated)) = found.iter().find(|(in_memory, _)| !*in_memory) {
            assert!(*allocated < len as u64, "an image elsewhere lost its holes: {allocated} of {len} bytes");
        }
    }

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
            |vm, old| vm.objects.log_a_put(0, old),
        ];
        for change in unsettle {
            change(&mut vm, roots[0]);
            assert!(!vm.generations_settled());
            vm = vm.at_rest_for_image().expect("a VM at rest").0;
            assert!(vm.generations_settled());
        }
    }

    /// A census meets the VM once and counts what it names by this build's
    /// tables: a fresh VM's function cells name every primitive. (Its
    /// forwarded variables' places are made as variables are first read.)
    #[test]
    fn a_census_counts_the_vm_and_its_ordinals() {
        let census = super::census(&Vm::new(), &());
        let vm = census["kinds"].as_array().unwrap().iter().find(|kind| kind["type"] == "romacs_vm::Vm").unwrap();
        assert_eq!(vm["count"], 1);
        let primitives = census["notes"]["ordinal: a primitive, by its index in the primitive table"].as_u64().unwrap_or(0);
        assert!(primitives >= crate::prim::count() as u64, "{}", census["notes"]);
    }

    #[test]
    fn the_collectors_reentry_state_is_at_rest_or_not_captured() {
        let not_captured = |change: fn(&mut Vm)| {
            let mut vm = Vm::new();
            assert_eq!(vm.world_quiescent(), Ok(()));
            change(&mut vm);
            vm.world_quiescent().unwrap_err()
        };
        assert!(not_captured(|vm| vm.suspended_stacks.push(vec![])).contains("suspended_stacks"));
        assert!(not_captured(|vm| vm.running_engines = 1).contains("running_engines"));
        assert!(not_captured(|vm| vm.gc_holds = 1).contains("gc_holds"));
        assert!(not_captured(|vm| vm.rooted_write = true).contains("rooted_write"));
        assert!(not_captured(|vm| vm.deferred_writes.push((crate::watchers::NativeWrite::set(0, crate::Value::NIL), crate::Value::NIL))).contains("deferred_writes"));
        // A cache slot a builder reserved that no installed code owns.
        assert!(not_captured(|vm| { vm.install_cache(); }).contains("unclaimed_caches"));
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
