//! The mark twin's stress check.
//!
//! A mark twin is a fork that marks the heap on its own copy while the Lisp
//! process goes on, and hands back what is dead. Before any twin frees
//! anything, this check proves its answer against the collector's. With
//! `ROMACS_GC_TWIN=N`, every Nth collection that leaves entries where they are
//! forks a child from the state it starts from. In the child, after the
//! caches a collection drops are dropped (rule M1):
//! - **the twin's mark:** the collector's own mark (`Vm::mark_only`) with
//!   every weak entry kept and every undo list whole (M2), and, to a fixpoint, every marker and overlay
//!   anchored in a buffer it reached and every private symbol with a local
//!   binding there (M8: what a table walk can hand Lisp is live). Everything
//!   the reference marker reaches must be in it (`check_reachable`, a floor),
//!   and so must every live window, which an id lookup can return. Nothing
//!   else may be: it must equal the closure the collection's own mark finds
//!   from the roots and the M8 extras when every entry of each weak table that
//!   closure reaches is a root too (`bound`);
//! - **the plain mark:** the same mark with the collection's own weak rule.
//!
//! The child sends both, as one bit per table entry, and exits at once. When
//! the parent's collection is done, `check` requires what the plain mark
//! leaves unmarked to be what the collection freed, and the twin's dead to
//! be freed too. What the collector frees and the twin keeps is then an M8
//! extra (`kept_m8`) or reached only through one or a weak entry
//! (`kept_via`). Anything else ends the process with a diagnostic naming the
//! entry. The child's figures say what it cost: its M8 rounds, each a whole
//! mark and walk (`rounds`), the bound's marks, one per level of weak tables
//! reached only through weak tables (`bound_rounds`), and its processor time,
//! all of it (`child_us`) and the bound's (`bound_us`).
//!
//! The check is opt-in, and costs only a process that asks for it. The
//! twin's own mark takes each weak table once, so a chain of them costs its
//! length. The bound, which takes no mark of the twin's on trust, costs one
//! whole mark per level of weak tables reached only through weak tables:
//! 34-146 ms in the sessions measured, which reach one level, and linear in
//! the depth of a deeper nesting (25 levels 1.5 s, 400 levels 10.7 s, on a
//! heap of 437k entries).
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

use crate::data::Object;
use crate::{Value, Vm};

/// The tables a mark colours, in `MarkedSet`'s order.
const TABLES: [&str; 4] = ["conses", "closures", "cells", "objects"];
/// The exit status of a process whose twin disagreed with its collector.
const DIVERGED: i32 = 97;
/// How long a collection waits for its twin's report: a bound for a child
/// stuck on a lock another thread held at the fork, far above any mark.
const DEADLINE: Duration = Duration::from_mins(2);

/// A twin's child on its way: forked as a collection starts. Dropped
/// unreaped (a key press ended the collection, or the report never came),
/// the child is ended and reaped.
pub(crate) struct Pending {
    child: i32,
    report: std::fs::File,
    reaped: bool,
    /// Each table's entries holding an object at the fork.
    allocated: [Vec<bool>; 4],
    collection: u64,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = crate::host::kill(self.child, libc::SIGKILL);
            let _ = crate::subprocess::wait_blocking(self.child);
        }
    }
}

/// Every how many collections a twin is checked: `ROMACS_GC_TWIN`, 0 for never.
fn every() -> u64 {
    static EVERY: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| crate::host::getenv("ROMACS_GC_TWIN").and_then(|v| v.to_str()?.parse().ok()).unwrap_or(0))
}

/// Before a collection: fork the twin's child when one is due. A compacting
/// collection moves the entries the check names by place; a world build
/// refuses the pipe and the fork; and a builder's heap is a shared mapping,
/// which a fork does not copy, so the child would mark the parent's own
/// colours while the parent marks and sweeps them. None of them has a twin.
pub(crate) fn fork_marks(vm: &mut Vm, roots: &[Value]) -> Option<Pending> {
    let n = every();
    let collection = vm.gc_stats.collections;
    if n == 0 || !collection.is_multiple_of(n) || vm.will_compact() || crate::host::mode() == crate::host::Mode::Build
        || crate::world::arena::heap_is_shared() {
        return None;
    }
    spawn(vm, collection, |vm| child_marks(vm, roots))
}

/// Fork a child that runs WORK on its copy of VM and sends what it returns,
/// or why it failed, as its report.
fn spawn(vm: &mut Vm, collection: u64, work: impl FnOnce(&mut Vm) -> Result<Vec<u8>, String>) -> Option<Pending> {
    let allocated = held(vm);
    let (read, write) = crate::host::pipe().ok()?;
    match crate::host::fork_threaded() {
        Ok(0) => {
            drop(read);
            child(vm, write, work)
        }
        Ok(child) => Some(Pending { child, report: read.into(), reaped: false, allocated, collection }),
        Err(_) => None,
    }
}

/// Whether each entry of each table holds an object, in `TABLES`' order.
fn held(vm: &Vm) -> [Vec<bool>; 4] {
    [vm.conses.held(), vm.closures.held(), vm.cells.held(), vm.objects.held()]
}

/// Each table's marks, in `TABLES`' order.
fn marks(vm: &Vm) -> [Vec<bool>; 4] {
    [vm.conses.marks().collect(), vm.closures.marks().collect(), vm.cells.marks().collect(), vm.objects.marks().collect()]
}

/// The twin's child: WORK's report into WRITE, then gone, running nothing of
/// the parent's (no destructor, no atexit handler, no buffered output). A
/// panic runs no inherited hook, which could wait on a lock (a backtrace's,
/// a log's) that a thread the fork left behind holds.
fn child(vm: &mut Vm, write: OwnedFd, work: impl FnOnce(&mut Vm) -> Result<Vec<u8>, String>) -> ! {
    std::panic::set_hook(Box::new(|_| {}));
    let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(vm)))
        .unwrap_or_else(|panic| Err(panic.downcast_ref::<String>().cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_else(|| "a panic".into())));
    let bytes = match report {
        Ok(report) => report,
        Err(text) => [b"NO".as_slice(), text.as_bytes()].concat(),
    };
    let _ = std::fs::File::from(write).write_all(&bytes);
    crate::host::exit_at_once(0)
}

/// The child's two marks and its figures, encoded (`encode`).
fn child_marks(vm: &mut Vm, roots: &[Value]) -> Result<Vec<u8>, String> {
    vm.clear_collection_caches();
    let mut extras: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rounds = 0u64;
    let (set, rooted) = loop {
        rounds += 1;
        vm.clear_marks();
        let rooted: Vec<Value> = roots.iter().chain(&extras).copied().collect();
        let set = vm.mark_only(&rooted, true);
        let more = table_walk_extras(vm, &set.obarray_symbols_live);
        if more.is_empty() {
            break (set, rooted);
        }
        // An extra is a root of the next mark, so the walk never finds it again.
        if let Some(v) = more.iter().find(|v| !seen.insert((v.tag(), v.payload()))) {
            return Err(format!("an M8 extra stays unmarked as a root: tag {} entry {}", v.tag(), v.payload()));
        }
        extras.extend(more);
    };
    vm.check_reachable(&rooted, &set.code_marks, &set.region_marks, &set.obarray_symbols_live);
    let windows = vm.referrers().windows.clone();
    for i in windows {
        if matches!(vm.objects.get(i), Some(Object::Window(w)) if w.live()) && !vm.objects.marked(i) {
            return Err(format!("live window at objects entry {i} is not reached"));
        }
    }
    let twin = marks(vm);
    let spent = || crate::host::live_run_time().map_or(0, |t| u64::try_from(t.as_micros()).unwrap_or(u64::MAX));
    let before = spent();
    let bound_rounds = bound(vm, &rooted, &twin)?;
    let bound_us = spent() - before;
    vm.clear_marks();
    vm.mark_only(roots, false);
    let plain = marks(vm);
    let extra_objects: Vec<u64> = extras.iter().filter(|v| v.tag() == 0).map(|v| v.payload() - 1).collect();
    Ok(encode(&twin, &plain, &extra_objects, [rounds, bound_rounds, bound_us, spent()]))
}

/// TWIN, the twin's marks, must be exactly the closure the collection's own
/// mark finds from ROOTED (the roots and the M8 extras) when every entry of
/// each weak table the closure reaches is a root too: keeping weak entries
/// (M2) is all that may add to that mark. A buffer's undo list is weak to
/// the collection's own mark in the same way, and is a root here with them.
/// The closure is found from the collection's own marks alone, each adding
/// the entries of the weak tables and the undo lists of the buffers it
/// reached first, so no mark of TWIN's vouches for itself. Leaves the tables
/// marked with the closure; its marks, one per level of weak tables and
/// buffers.
fn bound(vm: &mut Vm, rooted: &[Value], twin: &[Vec<bool>; 4]) -> Result<u64, String> {
    let mut roots = rooted.to_vec();
    let mut promoted = std::collections::HashSet::new();
    let mut rounds = 0;
    let closure = loop {
        rounds += 1;
        vm.clear_marks();
        vm.mark_only(&roots, false);
        let reached: Vec<usize> = vm.objects.iter().enumerate()
            .filter(|&(i, object)| matches!(object, Object::HashTable(t) if t.weakness != crate::hash::Weakness::None)
                && vm.objects.marked(i) && !promoted.contains(&i))
            .map(|(i, _)| i)
            .collect();
        // A buffer's undo list is weak to the collection's own mark as a
        // weak table's entries are, and the twin's keeps it whole.
        let buffers: Vec<usize> = vm.objects.iter().enumerate()
            .filter(|&(i, object)| matches!(object, Object::Buffer(_) | Object::ByteBuffer(_)) && vm.objects.marked(i) && !promoted.contains(&i))
            .map(|(i, _)| i)
            .collect();
        if reached.is_empty() && buffers.is_empty() {
            break marks(vm);
        }
        for i in reached {
            promoted.insert(i);
            if let Object::HashTable(table) = &vm.objects[i] {
                roots.extend(table.entries().flat_map(|(_, key, value)| [key, value]));
            }
        }
        for i in buffers {
            promoted.insert(i);
            roots.push(vm.own_undo_list(i));
        }
    };
    for t in 0..4 {
        if let Some(i) = (0..twin[t].len()).find(|&i| twin[t].get(i) != closure[t].get(i)) {
            let (twin_did, closure_did) = if twin[t][i] { ("marked", "does not reach") } else { ("left unmarked", "reaches") };
            return Err(format!("the twin {twin_did} {} entry {i}, which the mark from its roots, extras and weak entries {closure_did}", TABLES[t]));
        }
    }
    Ok(rounds)
}

/// M8: what a table walk can return from what the mark reached, and the mark
/// did not: each marker and overlay anchored in a reached buffer, and each
/// private symbol with a local binding in one. A buffer of the world image is
/// never freed, so it counts as reached.
fn table_walk_extras<S: std::hash::BuildHasher>(vm: &mut Vm, symbols_live: &std::collections::HashSet<usize, S>) -> Vec<Value> {
    let reached = |vm: &Vm, buffer: Value| buffer.tag() == 0 && buffer != Value::NIL
        && (Vm::image_value(buffer) || buffer.index_if(0).and_then(|i| i.checked_sub(1)).is_some_and(|i| vm.objects.marked(i)));
    let mut extras = Vec::new();
    for (i, object) in vm.objects.iter().enumerate() {
        let buffer = match object {
            Object::Marker(m) => m.buffer,
            Object::Overlay(o) => o.buffer,
            _ => continue,
        };
        if !vm.objects.marked(i) && reached(vm, buffer) {
            extras.push(Value::reference(i + 1, 0));
        }
    }
    for (slot, s) in vm.slots.iter().enumerate() {
        if !vm.obarray_symbol_names.contains_key(&slot) || symbols_live.contains(&slot) {
            continue;
        }
        let Some(Object::HashTable(table)) = vm.object_if(s.local) else { continue };
        if table.entries().any(|(_, buffer, _)| reached(vm, buffer)) {
            extras.push(Value::reference(slot + 1, 7));
        }
    }
    extras
}

/// The child's report: "OK", the twin's marks and the plain marks, each
/// table's as its length and one bit per entry, the count and the objects
/// entries of the M8 extras, then FIGURES (M8 rounds, the bound's rounds, the
/// bound's processor microseconds, all the child's).
/// A child that failed sends "NO" and why.
fn encode(twin: &[Vec<bool>; 4], plain: &[Vec<bool>; 4], extra_objects: &[u64], figures: [u64; 4]) -> Vec<u8> {
    let mut out = b"OK".to_vec();
    for bits in twin.iter().chain(plain) {
        out.extend((bits.len() as u64).to_le_bytes());
        out.extend(bits.chunks(8).map(|byte| byte.iter().enumerate().fold(0u8, |b, (i, &on)| b | (u8::from(on) << i))));
    }
    out.extend((extra_objects.len() as u64).to_le_bytes());
    for word in extra_objects.iter().chain(&figures) {
        out.extend(word.to_le_bytes());
    }
    out
}

/// The u64 in IN at AT, moving AT past it.
fn take_u64(input: &[u8], at: &mut usize) -> Option<u64> {
    let word = u64::from_le_bytes(input.get(*at..*at + 8)?.try_into().ok()?);
    *at += 8;
    Some(word)
}

/// One table's bits from IN at AT, moving AT past them.
fn take_bits(input: &[u8], at: &mut usize) -> Option<Vec<bool>> {
    let len = usize::try_from(take_u64(input, at)?).ok()?;
    let bytes = input.get(*at..*at + len.div_ceil(8))?;
    *at += len.div_ceil(8);
    Some((0..len).map(|i| bytes[i / 8] >> (i % 8) & 1 == 1).collect())
}

/// What a twin that agreed found: the entries both kept, the entries both
/// freed, the M8 extras it kept, the other entries it kept, reached only
/// through a weak entry or an extra; and what its child spent.
#[derive(Debug, PartialEq, Eq)]
struct Agreed {
    live: usize,
    dead: usize,
    kept_m8: usize,
    kept_via: usize,
    rounds: u64,
    bound_rounds: u64,
    bound_us: u64,
    child_us: u64,
}

/// REPORT, the child's, against the collection: ALLOCATED holds each
/// table's entries that held an object as it started, SURVIVED those that
/// hold one after it (an entry past a table's end was in its dead tail, cut).
/// KIND names an objects entry. The figures, or what disagreed.
fn verdict(report: &[u8], allocated: &[Vec<bool>; 4], survived: &[Vec<bool>; 4], kind: impl Fn(usize) -> &'static str) -> Result<Agreed, String> {
    if report.is_empty() {
        return Err("no report".into());
    }
    let Some(body) = report.strip_prefix(b"OK") else {
        return Err(String::from_utf8_lossy(report.strip_prefix(b"NO").unwrap_or(report)).into_owned());
    };
    let short = || "a short report".to_owned();
    let mut at = 0;
    let marks: Vec<Vec<bool>> = (0..8).map(|_| take_bits(body, &mut at)).collect::<Option<_>>().ok_or_else(short)?;
    let count = take_u64(body, &mut at).ok_or_else(short)?;
    let extra_objects: std::collections::HashSet<u64> = (0..count).map(|_| take_u64(body, &mut at)).collect::<Option<_>>().ok_or_else(short)?;
    let [rounds, bound_rounds, bound_us, child_us] = [(); 4].map(|()| take_u64(body, &mut at));
    let (Some(rounds), Some(bound_rounds), Some(bound_us), Some(child_us)) = (rounds, bound_rounds, bound_us, child_us) else { return Err(short()) };
    let mut agreed = Agreed { live: 0, dead: 0, kept_m8: 0, kept_via: 0, rounds, bound_rounds, bound_us, child_us };
    for t in 0..4 {
        let (twin, plain) = (&marks[t], &marks[t + 4]);
        for i in (0..allocated[t].len()).filter(|&i| allocated[t][i]) {
            let [twin_live, plain_live, real_live] = [twin, plain, &survived[t]].map(|bits| bits.get(i) == Some(&true));
            let entry = || format!("{} entry {i}{}", TABLES[t], if t == 3 { format!(" ({})", kind(i)) } else { String::new() });
            if plain_live != real_live {
                let (mark, collection) = if plain_live { ("marked", "freed") } else { ("left unmarked", "kept") };
                return Err(format!("the mark alone {mark} what the collection {collection}: {}", entry()));
            }
            match (twin_live, real_live) {
                (false, true) => return Err(format!("the twin counts dead what the collection kept: {}", entry())),
                (false, false) => agreed.dead += 1,
                (true, false) if t == 3 && extra_objects.contains(&(i as u64)) => agreed.kept_m8 += 1,
                (true, false) => agreed.kept_via += 1,
                (true, true) => agreed.live += 1,
            }
        }
    }
    Ok(agreed)
}

/// PENDING's report, read until its child closes its end, and the child
/// reaped: the report and the child's wait status. Past DEADLINE there is no
/// report, and the child is left for `Pending`'s drop to end and reap.
fn collect_report(pending: &mut Pending, deadline: Duration) -> Result<(Vec<u8>, i32), String> {
    let end = crate::host::monotonic() + deadline;
    let mut report = Vec::new();
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        let left = end.saturating_duration_since(crate::host::monotonic());
        if left.is_zero() {
            return Err(format!("no report from the twin's child within {} s", deadline.as_secs_f64()));
        }
        let ready = crate::subprocess::poll(&[(pending.report.as_raw_fd(), libc::POLLIN)], Some(left)).map_err(|e| e.to_string())?;
        if ready[0] == 0 {
            continue;
        }
        match (&pending.report).read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => report.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let status = crate::subprocess::wait_blocking(pending.child).map_err(|e| e.to_string())?;
    pending.reaped = true;
    Ok((report, status))
}

/// PENDING's twin against what the collection left in VM, within DEADLINE.
/// Its child is gone, reaped, whatever the answer.
fn settle(vm: &Vm, mut pending: Pending, deadline: Duration) -> Result<Agreed, String> {
    let (report, status) = collect_report(&mut pending, deadline)?;
    let survived = held(vm);
    verdict(&report, &pending.allocated, &survived, |i| object_kind(vm, i))
        .map_err(|what| if report.is_empty() { format!("{what} from the twin's child (wait status {status})") } else { what })
}

/// After the parent's collection: the twin's answer against what it freed.
/// Ends the process, saying where, when they disagree; prints the figures.
pub(crate) fn check(vm: &mut Vm, pending: Pending) {
    let collection = pending.collection;
    match settle(vm, pending, DEADLINE) {
        Ok(Agreed { live, dead, kept_m8, kept_via, rounds, bound_rounds, bound_us, child_us }) => eprintln!("{}", serde_json::json!({
            "twin": "agreed", "collection": collection, "live": live, "dead": dead, "kept_m8": kept_m8, "kept_via": kept_via,
            "rounds": rounds, "bound_rounds": bound_rounds, "bound_us": bound_us, "child_us": child_us})),
        Err(what) => diverged(collection, &what),
    }
}

/// The process ends: its twin and its collector disagreed about WHAT.
fn diverged(collection: u64, what: &str) -> ! {
    eprintln!("{}", serde_json::json!({"twin": "diverged", "collection": collection, "what": what}));
    crate::host::exit_at_once(DIVERGED)
}

/// The kind of object at objects entry I, for a diagnostic.
fn object_kind(vm: &Vm, i: usize) -> &'static str {
    match vm.objects.get(i) {
        Some(Object::Buffer(_) | Object::ByteBuffer(_)) => "buffer",
        Some(Object::Marker(_)) => "marker",
        Some(Object::Overlay(_)) => "overlay",
        Some(Object::Window(_)) => "window",
        Some(Object::HashTable(_)) => "hash table",
        Some(Object::Free) => "free",
        Some(_) => "object",
        None => "past the table",
    }
}

#[cfg(test)]
mod tests {
    use super::{Agreed, bound, child_marks, collect_report, encode, marks, settle, spawn, verdict};
    use crate::{Value, Vm};
    use std::sync::Mutex;
    use std::time::Duration;

    /// Tables with only objects entries: BITS.
    fn objects(bits: &[bool]) -> [Vec<bool>; 4] {
        [Vec::new(), Vec::new(), Vec::new(), bits.to_vec()]
    }

    fn judge(twin: &[bool], plain: &[bool], extras: &[u64], allocated: &[bool], survived: &[bool]) -> Result<Agreed, String> {
        verdict(&encode(&objects(twin), &objects(plain), extras, [2, 1, 3, 7]), &objects(allocated), &objects(survived), |_| "marker")
    }

    /// Entry 0 lives, 1 dies, 2 is an M8 extra and 3 is reached only through
    /// it or a weak entry; 4 was free; 5, dead, was in the tail the
    /// collection cut.
    #[test]
    fn a_twin_that_frees_a_subset_of_what_the_collection_frees_agrees() {
        let twin = [true, false, true, true, false, false];
        let plain = [true, false, false, false, false, false];
        let allocated = [true, true, true, true, false, true];
        let survived = [true, false, false, false, false];
        let agreed = Agreed { live: 1, dead: 2, kept_m8: 1, kept_via: 1, rounds: 2, bound_rounds: 1, bound_us: 3, child_us: 7 };
        assert_eq!(judge(&twin, &plain, &[2], &allocated, &survived), Ok(agreed));
    }

    #[test]
    fn a_twin_that_frees_what_the_collection_keeps_is_named() {
        let verdict = judge(&[true, false], &[true, true], &[], &[true, true], &[true, true]);
        assert_eq!(verdict, Err("the twin counts dead what the collection kept: objects entry 1 (marker)".into()));
    }

    #[test]
    fn a_mark_that_is_not_the_collections_is_named() {
        let kept = judge(&[true, true], &[true, false], &[], &[true, true], &[true, true]);
        assert_eq!(kept, Err("the mark alone left unmarked what the collection kept: objects entry 1 (marker)".into()));
        let freed = judge(&[true, true], &[true, true], &[], &[true, true], &[true, false]);
        assert_eq!(freed, Err("the mark alone marked what the collection freed: objects entry 1 (marker)".into()));
    }

    #[test]
    fn a_child_that_failed_or_stopped_short_is_named() {
        let none = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        assert_eq!(verdict(b"NOlive window at objects entry 7 is not reached", &none, &none, |_| ""), Err("live window at objects entry 7 is not reached".into()));
        let report = encode(&none, &none, &[1, 2], [1, 1, 1, 1]);
        assert_eq!(verdict(&report[..report.len() - 1], &none, &none, |_| ""), Err("a short report".into()));
        assert_eq!(verdict(b"", &none, &none, |_| ""), Err("no report".into()));
    }

    /// A VM with nothing collected behind the test's back.
    fn quiet_vm() -> Vm {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        vm
    }

    /// The twin's marks from ROOTS, keeping every weak entry.
    fn twin_marks(vm: &mut Vm, roots: &[Value]) -> [Vec<bool>; 4] {
        vm.clear_marks();
        vm.mark_only(roots, true);
        marks(vm)
    }

    /// The twin's mark is the collection's own from its roots and every weak
    /// entry it reached; a mark beyond that, or short of it, is named.
    #[test]
    fn a_twin_mark_beyond_its_roots_and_weak_entries_is_named() {
        let mut vm = quiet_vm();
        let table = vm.eval_source_raw("(let ((h (make-hash-table :test 'eq :weakness 'key))) (puthash (list 'weak-key) (list 'weak-value) h) h)").unwrap();
        let reached = vm.alloc_cons(Value::integer(1).unwrap(), Value::NIL);
        let unrelated = vm.alloc_cons(Value::integer(2).unwrap(), Value::NIL);
        let roots = [table, reached];
        let mut twin = twin_marks(&mut vm, &roots);
        assert_eq!(bound(&mut vm, &roots, &twin), Ok(2), "keeping weak entries is accounted for");
        let cons = |v: Value| v.index_if(v.tag()).expect("a cons's entry");
        let entry = cons(unrelated);
        twin[0][entry] = true;
        assert_eq!(bound(&mut vm, &roots, &twin),
                   Err(format!("the twin marked conses entry {entry}, which the mark from its roots, extras and weak entries does not reach")));
        twin[0][entry] = false;
        twin[0][cons(reached)] = false;
        assert!(bound(&mut vm, &roots, &twin).is_err_and(|e| e.contains("left unmarked")));
    }

    /// A buffer's undo list past its limits, with a marker nothing else
    /// holds: to the collection's own mark the list is weak, and it decides a
    /// cut. The twin's mark keeps the whole list, as it keeps weak entries,
    /// so the reference walk's floor and the bound both hold for it.
    #[test]
    fn an_undo_list_past_its_limits_is_whole_to_the_twin() {
        let mut vm = quiet_vm();
        vm.eval_source_raw("(progn (set-buffer (get-buffer-create \"undo\")) \
            (setq undo-limit 40 undo-strong-limit 80 undo-outer-limit 100000) \
            (let ((buffer-undo-list t)) (insert \"x\")) \
            (setq buffer-undo-list (list (cons (point-marker) 1) '(1 . 2) nil '(3 . 4) nil '(5 . 6) nil '(7 . 8))) nil)").unwrap();
        let report = child_marks(&mut vm, &[]);
        assert!(report.is_ok(), "{report:?}");
        let twin = twin_marks(&mut vm, &[]);
        vm.clear_marks();
        vm.mark_only(&[], false);
        let plain = marks(&vm);
        let count = |marks: &[Vec<bool>; 4]| marks[0].iter().filter(|&&marked| marked).count();
        assert!(count(&plain) < count(&twin), "the plain mark leaves the cut's conses, and the twin's does not");
    }

    /// A weak table reached only through another's entry is a level of its own.
    #[test]
    fn weak_tables_reached_through_weak_tables_are_accounted_for() {
        let mut vm = quiet_vm();
        let outer = vm.eval_source_raw("(let ((inner (make-hash-table :test 'eq :weakness 'key)) (outer (make-hash-table :test 'eq :weakness 'key))) \
            (puthash (list 'dead-inner-key) (list 'inner-value) inner) (puthash (list 'dead-outer-key) inner outer) outer)").unwrap();
        let twin = twin_marks(&mut vm, &[outer]);
        assert_eq!(bound(&mut vm, &[outer], &twin), Ok(3));
    }

    /// Weak tables nothing reaches cannot vouch for themselves: not one whose
    /// entry holds it, nor two whose entries hold each other.
    #[test]
    fn unreachable_weak_tables_holding_themselves_are_named() {
        let mut vm = quiet_vm();
        let lone = vm.eval_source_raw("(let ((h (make-hash-table :test 'eq :weakness 'key))) (puthash h h h) h)").unwrap();
        let pair = vm.eval_source_raw("(let ((a (make-hash-table :test 'eq :weakness 'key)) (b (make-hash-table :test 'eq :weakness 'value))) \
            (puthash a b b) (puthash b a a) (cons a b))").unwrap();
        let (a, b) = vm.cons_pair(pair).unwrap();
        let root = vm.alloc_cons(Value::integer(1).unwrap(), Value::NIL);
        let index = |v: Value| v.index_if(0).and_then(|i| i.checked_sub(1)).expect("an object's entry");
        for vouched in [vec![lone], vec![a, b]] {
            let mut twin = twin_marks(&mut vm, &[root]);
            for &table in &vouched {
                assert!(!twin[3][index(table)], "nothing reaches it");
                twin[3][index(table)] = true;
            }
            let named = bound(&mut vm, &[root], &twin);
            assert!(named.is_err_and(|e| e.starts_with("the twin marked objects entry")), "{vouched:?}");
        }
    }

    /// Taken by the test hook below, so a panic waits while it is held.
    static HOOK_LOCK: Mutex<()> = Mutex::new(());

    /// A panic hook that waits for `HOOK_LOCK` first, as a hook waits for a
    /// backtrace's or a log's lock, then does what the earlier hook did.
    fn hook_waits_for_its_lock() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let earlier = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                drop(HOOK_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
                earlier(info);
            }));
        });
    }

    /// A child that panics while the lock its inherited hook would wait for
    /// is held at the fork reports the panic: it runs no inherited hook.
    #[test]
    fn a_child_whose_hook_would_wait_on_a_held_lock_still_reports() {
        hook_waits_for_its_lock();
        let mut vm = Vm::new();
        let held = HOOK_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut pending = spawn(&mut vm, 0, |_| panic!("the floor failed")).expect("a fork");
        let report = collect_report(&mut pending, Duration::from_mins(1));
        drop(held);
        assert_eq!(report.map(|(report, _)| report), Ok(b"NOthe floor failed".to_vec()));
    }

    /// A child that never reports is waited for until the deadline; the
    /// collection's check then ends and reaps it before it answers.
    #[test]
    fn a_child_that_never_reports_ends_at_the_deadline() {
        let mut vm = Vm::new();
        let pending = spawn(&mut vm, 0, |_| loop { std::hint::spin_loop() }).expect("a fork");
        let child = pending.child;
        let settled = settle(&vm, pending, Duration::from_millis(300));
        assert!(settled.is_err_and(|e| e.starts_with("no report from the twin's child within 0.3 s")));
        assert!(crate::host::kill(child, 0).is_err(), "the child is gone, reaped");
    }

    /// A process that ends without unwinding in a collection (an isolate job
    /// over its memory budget, `isolate::job_memory_exit`) takes its twin's
    /// child with it: no `Pending` drop runs there to end it. Each wait is
    /// bounded, and each process is watched and signalled through a pidfd, so
    /// a zombie counts as ended and a reused pid is never signalled.
    #[test]
    fn a_twin_dies_with_a_process_that_ends_without_unwinding() {
        use std::io::{Read, Write};
        use std::os::fd::{AsFd, AsRawFd};
        // Ready within ten seconds: a pipe with data or closed, a process ended.
        let ready = |fd: std::os::fd::RawFd| crate::subprocess::poll(&[(fd, libc::POLLIN)], Some(Duration::from_secs(10)))
            .is_ok_and(|ready| ready[0] != 0);
        // Made before the fork: the middle process runs only `spawn' and
        // `_exit', panics nowhere, and takes no lock another test's thread
        // may have held at the fork.
        let mut vm = Vm::new();
        let (pid_read, pid_write) = crate::host::pipe().expect("a pipe");
        let (go_read, go_write) = crate::host::pipe().expect("a pipe");
        let middle = crate::host::fork_threaded().expect("a fork");
        if middle == 0 {
            drop((pid_read, go_write));
            let Some(pending) = spawn(&mut vm, 0, |_| loop { std::hint::spin_loop() }) else { crate::host::exit_at_once(2) };
            let _ = std::fs::File::from(pid_write).write_all(&pending.child.to_le_bytes());
            // Until the test holds the twin's pidfd: its pid stays its own.
            let _ = std::fs::File::from(go_read).read(&mut [0u8; 1]);
            crate::host::exit_at_once(0);
        }
        drop((pid_write, go_read));
        let middle_fd = crate::host::pidfd_open(middle).expect("the middle's pidfd");
        let mut pid_file = std::fs::File::from(pid_read);
        let mut pid = [0u8; 4];
        let reported = ready(pid_file.as_raw_fd()) && pid_file.read_exact(&mut pid).is_ok();
        let twin_fd = if reported { crate::host::pidfd_open(i32::from_le_bytes(pid)) } else { None };
        drop(go_write);
        let middle_ended = ready(middle_fd.as_raw_fd());
        if !middle_ended {
            let _ = crate::host::pidfd_send_signal(middle_fd.as_fd(), libc::SIGKILL, false);
        }
        let _ = crate::subprocess::wait_blocking(middle);
        let twin_ended = twin_fd.as_ref().is_some_and(|fd| ready(fd.as_raw_fd()));
        if let Some(fd) = &twin_fd && !twin_ended {
            let _ = crate::host::pidfd_send_signal(fd.as_fd(), libc::SIGKILL, false);
        }
        assert!(twin_fd.is_some(), "the middle process reported its twin");
        assert!(middle_ended, "the middle process ended");
        assert!(twin_ended, "the twin outlived its process");
    }

    /// What weak tables cost the bound, counted: a chain of DEPTH key-weak
    /// tables, each the value of a dead-keyed entry of the one before, ENTRIES
    /// entries each, in a heap PAD conses larger, costs it one mark per level
    /// and one more, whatever the entries and the heap. (The twin's own mark
    /// takes each table once: gc.rs `a_twins_mark_keeps_a_chain_of_weak_tables`.)
    #[test]
    fn weak_table_chains_cost_the_bound_one_mark_per_level() {
        for (depth, entries, pad) in [(10_u64, 1, 0), (40, 1, 0), (40, 20, 0), (40, 20, 20_000)] {
            let mut vm = quiet_vm();
            for i in 0..pad {
                vm.alloc_cons(Value::integer(i).unwrap(), Value::NIL);
            }
            let first = vm.eval_source_raw(&format!("(let ((next nil)) (dotimes (_ {depth}) (let ((h (make-hash-table :test 'eq :weakness 'key))) \
                (dotimes (_ {}) (puthash (list 'dead) (list 'value) h)) (when next (puthash (list 'dead-link) next h)) (setq next h))) next)", entries - 1)).unwrap();
            let twin = twin_marks(&mut vm, &[first]);
            assert_eq!(bound(&mut vm, &[first], &twin), Ok(depth + 1), "depth {depth}, entries {entries}, pad {pad}");
        }
    }
}
