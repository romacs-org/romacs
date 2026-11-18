//! A table's colours, allocation, sweeps, generations and cards, tested
//! apart from the table (`super`).
use super::*;

/// The table for a mark, as these tests use it.
pub(super) fn raw_of<T>(t: &mut Table<T>) -> RawTable<T> {
    // SAFETY: each test drops its table only after the last use of this,
    // and whatever else changes the table is what `holds' is to notice.
    unsafe { t.raw(false) }
}
/// A sweep of all RAW sees, on the test's one thread.
pub(super) fn sweep_whole<T>(raw: &RawTable<T>, quarantine: bool, destroy: impl FnMut(usize, &mut T)) -> RangeSwept {
    // SAFETY: the whole table is this thread's, and the range starts at 0.
    unsafe { raw.sweep_range(0..raw.len(), quarantine, destroy) }
}
/// A compaction after the mark standing, of a table with no image entry.
fn compact<T>(t: &mut Table<T>, keep: impl FnMut(&mut T)) {
    let (map, _) = t.compaction_map(0);
    t.collector_compact(&map, keep);
}
fn table(n: u64) -> Table<u64> {
    let mut table = Table::default();
    table.extend(0..n);
    table
}
fn sweep(table: &mut Table<u64>, live: &[usize]) -> usize {
    for &i in live {
        assert!(table.mark(i), "{i} was in use");
    }
    table.sweep(false, |entry| *entry = u64::MAX)
}

#[test]
fn allocation_takes_the_lowest_free_entry_then_the_end() {
    let mut t = table(20);
    assert_eq!(sweep(&mut t, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 19]), 9);
    assert_eq!(t[9], u64::MAX, "a tombstone");
    assert_eq!((t.len(), t.unused(), t.live()), (20, 9, 11));
    assert_eq!(t.alloc(100), 9);
    assert_eq!(t.alloc(101), 10);
    assert_eq!(t.alloc(102), 12);
    assert_eq!((t[9], t[10], t[12]), (100, 101, 102));
    for expected in 13..19 {
        assert_eq!(t.alloc(0), expected);
    }
    assert_eq!(t.unused(), 0);
    assert_eq!(t.alloc(0), 20, "then the end");
}

/// Past a long run in use the scan finds the hole, at any offset.
#[test]
fn a_free_entry_is_found_past_a_long_live_run() {
    for hole in [1, 7, 8, 9, 63, 64, 65, 1000] {
        let mut t = table(1100);
        let live: Vec<usize> = (0..1100).filter(|&i| i != hole).collect();
        assert_eq!(sweep(&mut t, &live), 1);
        assert_eq!(t.alloc(5), hole);
        assert_eq!(t.unused(), 0);
    }
}

#[test]
fn a_dead_tail_is_cut_however_it_falls_in_a_word() {
    for keep in 1..24 {
        let mut t = table(40);
        let live: Vec<usize> = (0..keep).collect();
        sweep(&mut t, &live);
        assert_eq!((t.len(), t.unused()), (keep, 0), "keeping {keep}");
    }
    let mut t = table(40);
    sweep(&mut t, &[]);
    assert_eq!((t.len(), t.unused()), (0, 0), "nothing survives");
}

/// A full sweep leaves every survivor allocated, so the next mark sees
/// the same table: marks do not survive a collection.
#[test]
fn a_table_at_rest_holds_no_marks() {
    let mut t = table(100);
    let all: Vec<usize> = (0..100).collect();
    for _ in 0..3 {
        assert_eq!(sweep(&mut t, &all), 0);
        assert!(t.marks().all(|marked| !marked));
    }
    assert!(t.mark(5));
    assert!(!t.mark(5), "once");
    t.clear_marks();
    assert!(t.mark(5), "an abandoned mark leaves none");
}

#[test]
fn a_stale_reference_marks_nothing() {
    let mut t = table(10);
    sweep(&mut t, &[0, 9]);
    assert!(t.is_free(4));
    assert!(!t.mark(4));
    assert!(!t.marked(4));
    assert_eq!(t.unused(), 8);
}

#[test]
fn quarantined_entries_count_free_and_are_never_reused_or_cut() {
    let mut t = table(10);
    for i in [0, 1] {
        t.mark(i);
    }
    assert_eq!(t.sweep(true, |entry| *entry = u64::MAX), 8);
    assert_eq!((t.len(), t.unused(), t.live()), (10, 8, 2));
    assert_eq!(t.alloc(7), 10, "not reused");
    assert!(t.is_free(9));
    assert!(!t.mark(9));
    for i in [0, 1, 10] {
        t.mark(i);
    }
    assert_eq!(t.sweep(true, |entry| *entry = u64::MAX), 0, "dead once");
}

#[test]
fn compaction_keeps_the_marked_in_order_and_leaves_the_table_dense() {
    let mut t = table(10);
    for i in [2, 5, 7] {
        t.mark(i);
    }
    compact(&mut t, |entry| *entry += 100);
    assert_eq!(t.entries, [102, 105, 107]);
    assert_eq!((t.unused(), t.live()), (0, 3));
    t.collector_prepend(2, || 0);
    assert_eq!(t.entries, [0, 0, 102, 105, 107]);
    assert_eq!(t.alloc(1), 5, "dense: the end");
}

fn conses(n: usize) -> Table<(Value, Value)> {
    let mut table = Table::default();
    table.extend((0..n).map(|_| (Value::NIL, Value::NIL)));
    table
}
/// Every entry survives a sweep: all are old from here.
fn age<T>(table: &mut Table<T>) {
    for i in 0..table.len() { table.mark(i); }
    assert_eq!(table.sweep(false, |_| {}), 0);
}

/// A sweep leaves its survivors old, what is allocated after it is young
/// until the next, and an old entry no mark reaches dies like any other.
/// A minor's sweep of TABLE (`gc_mark::Marking::sweep`): the set cards
/// alone, LIVE claimed first.
fn minor_sweep(t: &mut Table<Value>, live: &[usize]) -> RangeSwept {
    let runs = t.touched();
    // SAFETY: the table is this thread's until the books are kept.
    let raw = unsafe { t.raw(true) };
    for &i in live { assert!(raw.claim(i), "{i} young and unclaimed"); }
    let swept = runs.into_iter().fold(RangeSwept::default(), |all, run| {
        // SAFETY: each run is whole cards, so starts on a multiple of 8.
        all.and(unsafe { raw.sweep_range(run, false, |_, v| *v = Value::NIL) })
    });
    t.swept(swept, false);
    swept
}

/// A minor meets only the cards an entry was handed out or stored into
/// in since the last collection: it reads the old one stored into, frees
/// the young dead, and leaves every card clear and every entry old.
#[test]
fn a_minor_meets_only_the_cards_changed_since_the_last_collection() {
    let mut t: Table<Value> = Table::default();
    t.extend((0..256).map(|_| Value::NIL));
    assert_eq!(t.touched(), [0..256], "every entry was handed out");
    let raw = raw_of(&mut t);
    for i in 0..256 { if i != 130 { assert!(raw.claim(i)); } }
    let swept = tests::sweep_whole(&raw, false, |_, v| *v = Value::NIL);
    t.swept(swept, false);
    assert!(t.settled() && t.touched().is_empty(), "a full leaves no card set");
    t.set(70, Value::TRUE);
    let (young, end) = (t.alloc(Value::NIL), t.alloc(Value::NIL));
    assert_eq!((young, end), (130, 256), "the hole first, then the end");
    assert_eq!(t.touched(), [64..192, 256..257]);
    assert_eq!(t.read_again(), [70], "the old entry stored into, and no other");
    // The young at the end dies: the table ends at the old entry below
    // it, in a card the sweep passed.
    assert_eq!(minor_sweep(&mut t, &[70, young]).freed, 1, "the young dead");
    assert!(t.old(70) && t.old(young) && t.old(255) && t.settled());
    assert_eq!(t.len(), 256, "cut down to the last entry in use");
    assert!(t.touched().is_empty(), "a minor leaves no card set");
}

#[test]
fn a_survivor_is_old_and_a_new_entry_young() {
    let mut t = table(20);
    assert!((0..20).all(|i| !t.old(i)), "never collected");
    assert!(!t.settled());
    sweep(&mut t, &[0, 1, 2, 12]);
    assert!([0, 1, 2, 12].iter().all(|&i| t.old(i)));
    assert!(t.settled());
    let young = t.alloc(7);
    assert!(!t.old(young) && !t.settled());
    assert_eq!(sweep(&mut t, &[0, young]), 3, "old and unreached is dead");
    assert!(t.old(young) && t.settled());
    // A mark's view claims old and young alike, once.
    let fresh = t.alloc(8);
    let raw = raw_of(&mut t);
    assert!(raw.claim(0) && !raw.claim(0) && raw.claim(fresh) && !raw.claim(fresh));
    assert!(!raw.claim(2), "a free entry");
}

fn dirty<T>(table: &Table<T>) -> Vec<usize> { table.dirty_old().collect() }

/// An abandoned mark leaves the generations as they were.
#[test]
fn an_abandoned_mark_keeps_the_generations_and_what_was_written() {
    let mut t = conses(3);
    age(&mut t);
    let young = Value::reference(t.alloc((Value::NIL, Value::NIL)), 3);
    t.set_car(0, young);
    assert!(t.mark(0) && t.mark(1));
    t.clear_marks();
    assert!(t.old(0) && t.old(1) && t.old(2) && !t.old(3));
    assert_eq!(dirty(&t), [0]);
}

/// A store into an old cons leaves it dirty, whatever is stored; a young
/// cons written is not an old one written. A sweep leaves every survivor
/// clean.
#[test]
fn a_store_into_an_old_cons_leaves_it_dirty() {
    let mut t = conses(4);
    t.set_car(0, Value::reference(1, 3));
    assert!(dirty(&t).is_empty(), "a young holder");
    age(&mut t);
    assert!(dirty(&t).is_empty(), "and a sweep cleans what was written before it");
    let young = t.alloc((Value::NIL, Value::NIL));
    let (cons, object) = (Value::reference(young, 3), Value::reference(9, 0));
    t.set_car(0, cons);
    t.set_cdr(0, object);
    t.set_cdr(2, Value::integer(7).unwrap());
    t.set_cdr(young, cons);
    assert_eq!(dirty(&t), [0, 2]);
    assert!(t.logged().is_empty() && !t.settled());
    assert_eq!((t[0], t[2].1, t[young].1), ((cons, object), Value::integer(7).unwrap(), cons), "and the stores happen");
    age(&mut t);
    assert!(dirty(&t).is_empty() && t.settled());
    assert!(t.old(young), "the young cons written survived, old and clean");
}

/// Each of a cons's two stores dirties it.
#[test]
fn either_half_of_a_cons_written_dirties_it() {
    let mut t = conses(2);
    age(&mut t);
    t.set_car(0, Value::TRUE);
    assert_eq!(dirty(&t), [0]);
    t.set_cdr(1, Value::TRUE);
    assert_eq!(dirty(&t), [0, 1]);
}

/// The sweep's word at a time agrees with its entry at a time, whatever
/// the colours beside the mark: a word whose entries all survive leaves
/// each exactly old, and its free and quarantined entries as they were.
#[test]
fn a_word_of_survivors_of_every_colour_is_swept_to_old() {
    let mut t = conses(40);
    // Quarantine 3 and 12, free nothing else.
    for i in (0..40).filter(|i| ![3, 12].contains(i)) { t.mark(i); }
    assert_eq!(t.sweep(true, |_| {}), 2);
    // Old and dirty, old and clean, young and dirty, young and clean.
    let young: Vec<usize> = (0..8).map(|_| t.alloc((Value::NIL, Value::NIL))).collect();
    assert_eq!(young[0], 40, "quarantined entries are not reused");
    for i in [0, 5, 9, 17, 33, young[1], young[6]] { t.set_car(i, Value::TRUE); }
    for i in (0..48).filter(|i| ![3, 12].contains(i)) { assert!(t.mark(i), "{i}"); }
    assert!(!t.mark(3) && !t.mark(12), "nor ever marked");
    assert_eq!(t.sweep(true, |_| panic!("nothing is dead")), 0);
    assert!(t.settled() && dirty(&t).is_empty());
    assert_eq!((t.len(), t.unused(), t.live()), (48, 2, 46));
    assert!((0..48).all(|i| t.old(i) != [3, 12].contains(&i)) && t.is_free(3) && t.is_free(12));
    // And a word with one dead entry among them, by the other path.
    for i in (0..48).filter(|i| ![3, 12, 20].contains(i)) { t.mark(i); }
    assert_eq!(t.sweep(false, |_| {}), 1);
    assert!(t.settled() && t.is_free(20));
    // And a free entry in a word of survivors stays free.
    t.set_cdr(21, Value::TRUE);
    for i in (0..48).filter(|i| ![3, 12, 20].contains(i)) { assert!(t.mark(i)); }
    assert_eq!(t.sweep(false, |_| panic!("nothing is dead")), 0);
    assert!(t.settled() && t.is_free(20) && dirty(&t).is_empty());
    assert_eq!((t.unused(), t.alloc((Value::NIL, Value::NIL))), (3, 20));
}

/// One word holding every colour a sweep can meet with all in use
/// surviving: free, quarantined, old and young, clean and dirty. Each in
/// use ends exactly old, the other two as they were, nothing is freed.
#[test]
fn one_word_of_every_colour_sweeps_to_exactly_old() {
    let mut t = conses(6);
    for i in [0, 1, 3, 4, 5] { t.mark(i); }
    assert_eq!(t.sweep(true, |_| {}), 1, "2 is quarantined");
    for i in [0, 1, 4, 5] { t.mark(i); }
    assert_eq!(t.sweep(false, |_| {}), 1, "3 is free");
    t.push((Value::NIL, Value::NIL));
    t.push((Value::NIL, Value::NIL));
    for i in [1, 5, 7] { t.set_car(i, Value::TRUE); }
    assert_eq!(t.colors, [OLD | IN_USE, OLD | DIRTY | IN_USE, QUARANTINED, FREE, OLD | IN_USE, OLD | DIRTY | IN_USE, IN_USE, DIRTY | IN_USE]);
    for i in [0, 1, 4, 5, 6, 7] { assert!(t.mark(i)); }
    assert!(!t.mark(2) && !t.mark(3));
    assert_eq!(t.sweep(false, |_| panic!("nothing is dead")), 0);
    let old = OLD | IN_USE;
    assert_eq!(t.colors, [old, old, QUARANTINED, FREE, old, old, old, old]);
    assert_eq!((t.len(), t.unused(), t.live()), (8, 2, 6));
    assert_eq!(t.alloc((Value::NIL, Value::NIL)), 3, "the free entry, never the quarantined one");
}

/// A filler that panics leaves no entry without a colour: a store into
/// what it had put in stays in bounds.
#[test]
fn a_panic_while_prepending_leaves_a_colour_for_every_entry() {
    let mut t = conses(3);
    let mut made = 0;
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        t.collector_prepend(4, || {
            made += 1;
            assert!(made < 3, "a filler's panic");
            (Value::NIL, Value::NIL)
        });
    }));
    assert!(caught.is_err());
    assert!(t.colors.len() >= t.entries.len());
    // Memory safety is all that is left to ask of this table: its
    // colours no longer describe its entries.
    for i in 0..t.entries.len() { t.set_car(i, Value::TRUE); }
}

/// Settling a table a stopped collection left: its marks go, the free
/// entries its sweep never counted are counted, and a view of the table
/// from before no longer holds.
#[test]
fn settling_after_a_stopped_collection_recounts_and_unmarks() {
    let mut t = table(16);
    let raw = raw_of(&mut t);
    for i in (0..16).filter(|i| i % 2 == 0) { assert!(raw.claim(i)); }
    // The sweep, and no `swept': a collection stopped after its sweep.
    assert_eq!(sweep_whole(&raw, false, |_, entry| *entry = u64::MAX).freed, 8);
    assert_eq!(t.unused(), 0, "nothing counted yet");
    t.settle_old();
    assert!(t.settled() && t.marks().all(|marked| !marked));
    assert_eq!((t.unused(), t.live()), (8, 8));
    assert!(!raw.holds(&t));
    assert_eq!(t.alloc(7), 1, "and a counted free entry is reused");
    // Stopped while it marked: the marks alone go.
    let mut t = table(8);
    for i in 0..4 { t.mark(i); }
    let raw = raw_of(&mut t);
    assert!(raw.claim(5));
    t.settle_old();
    assert!(t.settled() && t.marks().all(|marked| !marked) && !raw.holds(&t));
    assert_eq!(t.live(), 8);
}

/// A cell and a vector slot are dirtied as a cons is, and a store that
/// fails writes nothing.
#[test]
fn a_cell_and_a_vector_written_are_dirty() {
    let mut cells: Table<Value> = Table::default();
    cells.push(Value::NIL);
    cells.push(Value::NIL);
    age(&mut cells);
    let young = Value::reference(3, 3);
    cells.set(1, young);
    assert_eq!(dirty(&cells), [1]);
    let mut objects: Table<Object> = Table::default();
    objects.push(Object::Vector(vec![Value::NIL; 2]));
    objects.push(Object::Float(1.0));
    objects.push(Object::Record(vec![Value::NIL; 40]));
    objects.push(Object::Vector(vec![Value::NIL; 2]));
    age(&mut objects);
    assert_eq!(objects.update_element(0, 2, young), Err(SlotFault::OutOfRange), "which writes nothing");
    assert_eq!(objects.update_element(1, 0, young), Err(SlotFault::NotSlots));
    assert!(dirty(&objects).is_empty());
    assert_eq!(objects.update_element(0, 1, young), Ok(()));
    assert_eq!(objects.update_element(2, 39, Value::TRUE), Ok(()));
    assert_eq!(dirty(&objects), [0, 2]);
    assert!(objects.logged().is_empty(), "a vector logs no value");
}

/// An old entry handed out whole is dirty; a young one is not an old one.
#[test]
fn an_old_entry_handed_out_whole_is_dirty() {
    let mut t: Table<Object> = Table::default();
    for _ in 0..5 { t.push(Object::Float(0.0)); }
    *t.entry_mut(0) = Object::Float(1.0);
    age(&mut t);
    let young = t.alloc(Object::Float(2.0));
    *t.entry_mut(1) = Object::Float(3.0);
    assert!(t.get_entry_mut(2).is_some() && t.get_entry_mut(9).is_none());
    *t.entry_mut(young) = Object::Float(4.0);
    assert_eq!(dirty(&t), [1, 2]);
    t.swap(0, 3);
    let _ = t.entry_mut_with(4, young);
    assert_eq!(dirty(&t), [0, 1, 2, 3, 4]);
    age(&mut t);
    assert!(dirty(&t).is_empty() && t.settled());
}

/// A strong table's puts log the keys and values; a weak table's entries
/// are no strong edges, so its puts dirty the table.
#[test]
fn a_hash_put_logs_its_values_or_dirties_its_weak_table() {
    use crate::hash::{HashTable, Test, Weakness};
    let mut t: Table<Object> = Table::default();
    t.push(Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::None, 8))));
    t.push(Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::Key, 8))));
    t.push(Object::Float(0.0));
    assert!(t.hash_mut(2).is_none());
    let (key, value) = (Value::reference(5, 3), Value::reference(6, 3));
    t.hash_mut(0).unwrap().1.stored(key);
    assert!(t.logged().is_empty(), "a young table");
    age(&mut t);
    for i in 0..2 {
        let (_, mut stores) = t.hash_mut(i).unwrap();
        stores.stored(key);
        stores.stored(value);
        // Nothing that names no entry, or one no collection frees by parts.
        for inert in [Value::NIL, Value::TRUE, Value::UNBOUND, Value::integer(1).unwrap(), Value::reference(3, 7), Value::reference(crate::image::IMAGE | 3, 3)] {
            stores.stored(inert);
        }
    }
    assert_eq!(t.logged(), [key, value]);
    assert_eq!(dirty(&t), [1]);
    // An abandoned mark keeps the log, a settle and a compaction empty it.
    t.clear_marks();
    assert_eq!(t.logged(), [key, value]);
    t.settle_old();
    assert!(t.logged().is_empty() && t.settled());
    t.hash_mut(0).unwrap().1.stored(key);
    for i in 0..3 { t.mark(i); }
    compact(&mut t, |_| {});
    assert!(t.logged().is_empty() && t.settled());
}

/// A log that only a loop with no poll can fill gives up: it is dropped,
/// records nothing more, and says so until a sweep begins another.
#[test]
fn a_log_past_its_limit_is_dropped_and_says_so() {
    use crate::hash::{HashTable, Test, Weakness};
    let young = Value::reference(3, 3);
    let mut t: Table<Object> = Table::default();
    t.push(Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::None, 8))));
    age(&mut t);
    let put = |t: &mut Table<Object>| t.hash_mut(0).unwrap().1.stored(young);
    for _ in 0..LOG_GIVES_UP - 1 { put(&mut t); }
    assert!(!t.log_lost());
    assert_eq!(t.logged().len(), LOG_GIVES_UP - 1);
    put(&mut t);
    assert!(t.log_lost());
    put(&mut t);
    assert!(t.logged().is_empty(), "nothing more is recorded");
    assert!(!t.settled(), "a lost log is not an empty one");
    t.clear_marks();
    assert!(t.log_lost(), "nor does an abandoned mark find it again");
    age(&mut t);
    assert!(!t.log_lost() && t.settled());
    put(&mut t);
    assert_eq!(t.logged(), [young]);
}

/// What an image holds: every entry old, no log, whatever was young,
/// dirty or logged before.
#[test]
fn settling_makes_every_entry_old_and_the_log_empty() {
    let mut t: Table<Object> = Table::default();
    for _ in 0..4 { t.push(Object::Float(0.0)); }
    sweep_objects(&mut t, &[0, 3]);
    let young = t.alloc(Object::Float(1.0));
    *t.entry_mut(0) = Object::Float(2.0);
    assert!(!t.settled());
    t.settle_old();
    assert!(t.settled() && t.old(young) && t.old(0) && t.is_free(2));
    assert_eq!(t.alloc(Object::Float(3.0)), 2, "and a free entry stays free");
}
fn sweep_objects(table: &mut Table<Object>, live: &[usize]) {
    for &i in live { assert!(table.mark(i)); }
    table.sweep(false, |entry| *entry = Object::Free);
}

/// Compaction leaves survivors: all old.
#[test]
fn compaction_leaves_every_entry_old() {
    let mut t = table(6);
    for i in [1, 4] { t.mark(i); }
    compact(&mut t, |_| {});
    assert!(t.settled() && t.old(0) && t.old(1));
}

/// Growth past the threshold keeps its storage: nothing copies, however
/// long the table gets, and `capacity' says what was touched.
#[test]
fn a_large_table_does_not_move_to_grow() {
    let mut t: Table<u64> = Table::default();
    let bytes = 2 * RESERVE_FROM;
    while t.len() * 8 < bytes {
        t.push(0);
    }
    if !crate::world::arena::may_reserve() { return; }
    assert!(t.reserved, "past RESERVE_FROM");
    let at = t.entries.as_ptr();
    for _ in 0..(4 << 20) {
        t.push(0);
    }
    assert_eq!(t.entries.as_ptr(), at, "no copy");
    assert_eq!(t.capacity(), t.len(), "the reservation is not capacity");
}

/// A mark's view refuses its table once anything moved or changed it:
/// its colours alone, its entries alone, or an entry handed out.
#[test]
fn a_changed_table_is_not_held() {
    let moved = |t: &mut Table<u64>, colors: bool| {
        if colors {
            let old = std::mem::take(&mut t.colors);
            t.colors = old.clone();
        } else {
            let old = std::mem::take(&mut t.entries);
            t.entries = old.clone();
        }
    };
    for colors in [true, false] {
        let mut t = table(100);
        let raw = raw_of(&mut t);
        assert!(raw.holds(&t));
        moved(&mut t, colors);
        assert!(!raw.holds(&t), "moved {}", if colors { "colours" } else { "entries" });
    }
    let mut t = table(100);
    sweep(&mut t, &[0, 1]);
    let raw = raw_of(&mut t);
    t.alloc(7);
    assert!(!raw.holds(&t), "an entry handed out");
    let mut t = table(100);
    let raw = raw_of(&mut t);
    t.clear_marks();
    assert!(!raw.holds(&t), "its marks cleared");
    let mut t = table(100);
    let raw = raw_of(&mut t);
    let _again = raw_of(&mut t);
    assert!(!raw.holds(&t), "a mark begun again");
    // All live, so nothing about its shape changes: its marks still go.
    let mut t = table(100);
    let raw = raw_of(&mut t);
    for i in 0..100 { assert!(raw.claim(i)); }
    compact(&mut t, |_| {});
    assert!(!raw.holds(&t), "compacted with every entry kept");
    let mut t = table(100);
    let raw = raw_of(&mut t);
    for i in 0..100 { assert!(raw.claim(i)); }
    let swept = sweep_whole(&raw, false, |_, _| {});
    t.swept(swept, false);
    assert!(!raw.holds(&t), "swept with every entry kept");
}
