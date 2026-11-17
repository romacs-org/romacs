//! The image's colours (`PERMANENT`, `REMEMBERED`) through sweeps, stores,
//! settles and compactions.
use super::tests::{raw_of, sweep_whole};
use super::*;

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
    let (freed, _) = sweep_whole(&raw, false, |_, _| {});
    t.swept(freed, t.len(), false);
    assert_eq!(freed, 0);
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

/// A compaction keeps every image entry where it is: the rest slide down
/// but never past one, and the holes below one stay free.
#[test]
fn a_compaction_moves_no_image_entry() {
    let mut t = image(6);
    // Young entries after the image, then a sweep of one image-era hole's worth.
    t.extend((6..12).map(|i| Value::integer(i).unwrap()));
    let raw = raw_of(&mut t);
    for i in [2, 5, 8, 11] { raw.claim(i); }
    let (map, pads) = t.compaction_map(3);
    assert_eq!(pads, 0, "no filler goes in front of the image");
    assert_eq!(map[..6], [0, 1, 2, 3, 4, 5], "image entries stay");
    assert_eq!((map[6], map[7], map[8], map[9], map[10], map[11]), (usize::MAX, usize::MAX, 6, usize::MAX, usize::MAX, 7));
    t.collector_compact(&map, |_| {}, || Value::NIL);
    assert_eq!(t.len(), 8);
    assert_eq!(t[3].as_integer().ok(), Some(3));
    assert_eq!((t[6].as_integer().ok(), t[7].as_integer().ok()), (Some(8), Some(11)));
    assert!(t.permanent(3) && t.is_remembered(5) && t.old(7) && !t.permanent(7));
    assert!(t.settled());
}
