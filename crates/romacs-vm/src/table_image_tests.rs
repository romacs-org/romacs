//! The image's colours (`PERMANENT`, `REMEMBERED`) through sweeps, stores,
//! settles and compactions.
use super::tests::{raw_of, sweep_whole};
use super::*;

impl crate::Vm {
    /// Whether OPERATION stores into the current buffer's old image entry:
    /// its DIRTY, its card and its remembering, each as WRITES says. The
    /// entry-barrier tests of five files share it.
    pub(crate) fn check_buffer_entry_store(
        &mut self,
        writes: bool,
        operation: impl FnOnce(&mut Self, usize),
    ) {
        self.set_gc_stress(0);
        let card_set = |vm: &Self, index: usize| vm.objects.touched().iter().any(|run| run.contains(&index));
        // A compaction leaves no free entry, so what OPERATION allocates
        // goes at the table's end: padded off the buffer's card, which then
        // marks only what is stored into its entries.
        self.eval_source_raw("(defvar entry-barrier-pad nil)").unwrap();
        let mut index = None;
        for _ in 0..4 {
            let before = self.heap_stats().collections;
            self.collect_compacting();
            assert_eq!(self.heap_stats().collections, before + 1);
            let at = self.buffer_index().unwrap();
            if at / CARD != self.objects.len() / CARD {
                index = Some(at);
                break;
            }
            self.eval_source_raw("(push (make-vector 64 nil) entry-barrier-pad)
              (dotimes (i 64) (aset (car entry-barrier-pad) i (make-string 1 ?x)))").unwrap();
        }
        let index = index.expect("the buffer's card off the table's end");
        let collections = self.heap_stats().collections;
        assert!(self.objects.old(index) && !self.objects.stored_into(index) && !card_set(self, index));

        // Restore remembers all buffers for their shared storage. Model an
        // initially unremembered image entry at the table boundary to check
        // what this operation contributes to the remembered set.
        self.objects.make_permanent(|_| false);
        assert!(self.objects.permanent(index));
        operation(self, index);
        assert_eq!(self.heap_stats().collections, collections);

        // The collection records dirty image entries before clearing DIRTY.
        let (dirty, card) = (self.objects.stored_into(index), card_set(self, index));
        self.objects.remembered();
        assert_eq!(
            (dirty, card, self.objects.is_remembered(index)),
            (writes, writes, writes),
            "(DIRTY, card, remembered) for the old buffer entry"
        );
    }
}

/// A table made the image's: entries 2 and 5 remembered, the rest permanent.
fn image(n: i64) -> Table<Value> {
    let mut t = Table::default();
    t.extend((0..n).map(|i| Value::integer(i).unwrap()));
    t.settle_old();
    t.make_permanent(|v| matches!(v.as_integer(), Ok(2 | 5)));
    t
}

/// A full counts a permanent entry as reached and reads only what it
/// marks; its sweep frees neither kind of image entry it reached, and
/// every survivor keeps its image bit.
#[test]
fn a_full_frees_no_permanent_entry_and_sweeps_keep_the_image() {
    let mut t = image(20);
    let raw = raw_of(&mut t);
    assert!(!raw.claim(3), "a permanent entry counts as reached");
    assert!(raw.claim(2) && raw.claim(5), "a remembered one is read");
    let swept = sweep_whole(&raw, false, |_, _| {});
    t.swept(swept, false);
    assert_eq!(swept.freed, 0);
    assert!((0..20).all(|i| t.old(i)) && t.permanent(3) && t.is_remembered(2) && t.is_remembered(5) && t.settled());
}

/// A store into a permanent entry makes it remembered at the next
/// collection's start, of either kind, or at a settle; before that it
/// is permanent and dirty.
#[test]
fn a_store_into_a_permanent_entry_is_remembered_before_its_dirt_is_cleared() {
    let starts: [fn(&mut Table<Value>); 3] = [|t| { t.remembered(); }, |t| { t.read_again(); }, Table::settle_old];
    for start in starts {
        let mut t = image(10);
        t.set(7, Value::NIL);
        assert!(t.permanent(7) && t.stored_into(7));
        start(&mut t);
        assert!(t.is_remembered(7) && !t.permanent(7), "the store is recorded");
    }
    let mut t = image(10);
    t.set(7, Value::NIL);
    assert_eq!(t.remembered(), vec![2, 5, 7], "a full's roots in the image");
}

/// A compaction keeps every image entry where it is: the image is dense and
/// its entries never die, so the rest slide down after it, and no filler
/// goes in front.
#[test]
fn a_compaction_moves_no_image_entry() {
    let mut t = image(6);
    // Young entries after the image: of them, 8 and 11 live.
    t.extend((6..12).map(|i| Value::integer(i).unwrap()));
    let raw = raw_of(&mut t);
    for i in [2, 5, 8, 11] { raw.claim(i); }
    let (map, pads) = t.compaction_map(3);
    assert_eq!(pads, 0, "no filler goes in front of the image");
    assert_eq!(map[..6], [0, 1, 2, 3, 4, 5], "image entries stay");
    assert_eq!((map[6], map[7], map[8], map[9], map[10], map[11]), (usize::MAX, usize::MAX, 6, usize::MAX, usize::MAX, 7));
    t.collector_compact(&map, |_| {});
    assert_eq!(t.len(), 8);
    assert_eq!(t[3].as_integer().ok(), Some(3));
    assert_eq!((t[6].as_integer().ok(), t[7].as_integer().ok()), (Some(8), Some(11)));
    assert!(t.permanent(3) && t.is_remembered(5) && t.old(7) && !t.permanent(7));
    assert!(t.settled());
}

/// The image's bits live beside the colours: making a table the image's,
/// and a full's sweep, leave every colour value as it was, an untouched image
/// entry's included, while the sweep clears a remembered one's mark. (That no
/// page is written is the census rig's to observe, not a test's.)
#[test]
fn the_image_set_keeps_every_colour_at_the_restore_and_a_full() {
    let mut t = Table::default();
    t.extend((0..40).map(|i| Value::integer(i).unwrap()));
    t.settle_old();
    let before = t.colors();
    t.make_permanent(|v| matches!(v.as_integer(), Ok(9)));
    assert_eq!(t.colors(), before, "the restore changed no colour");
    assert!(t.permanent(3) && t.is_remembered(9) && !t.permanent(9));
    let raw = raw_of(&mut t);
    assert!(raw.claim(9), "a remembered entry is read");
    let swept = sweep_whole(&raw, false, |_, _| {});
    t.swept(swept, false);
    assert_eq!((swept.freed, t.colors()), (0, before), "every colour as it was: the mark cleared");
}

/// An image entry is an index below the image's length: a table made the
/// image's while it holds free entries would take young entries in among
/// them, so it refuses.
#[test]
#[should_panic(expected = "an image's table is dense")]
fn an_image_is_made_of_a_dense_table() {
    let mut t = Table::default();
    t.extend((0..10).map(|i| Value::integer(i).unwrap()));
    let raw = raw_of(&mut t);
    // Entry 1 dies between two that live: a hole below the end.
    for i in [0, 2] { raw.claim(i); }
    let swept = sweep_whole(&raw, false, |_, _| {});
    t.swept(swept, false);
    assert_eq!((t.len(), t.unused()), (3, 1));
    t.make_permanent(|_| false);
}
