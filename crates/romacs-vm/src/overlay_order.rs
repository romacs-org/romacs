//! GNU's overlay orders: buffer.c sort_overlays/compare_overlays, which
//! decide which overlay's property wins (`overlays-at' SORTED, character
//! properties, faces), and xdisp.c load_overlay_strings, the order of the
//! overlay strings shown at one buffer position, shared by measurement
//! (prim/leaf/display_elements.rs) and the mini-window's display
//! (mini_window.rs).
use crate::Value;

/// buffer.c struct sortvec: an overlay's range and its `priority',
/// (PRIMARY . SECONDARY) or PRIMARY, as make_sortvec_item reads it.
#[derive(Clone, Copy)]
pub(crate) struct SortItem {
    pub overlay: Value,
    pub start: u32,
    pub end: u32,
    pub priority: i64,
    pub secondary: i64,
    /// Overlay::inserted, which orders equal starts.
    pub inserted: u64,
}

/// ITREE_FOREACH's ascending order: by start, and among equal starts the
/// latest inserted first (itree_insert_node goes left on a tie).
fn tree_order<T>(items: &mut [T], key: impl Fn(&T) -> SortItem) {
    items.sort_by_key(|item| { let k = key(item); (k.start, std::cmp::Reverse(k.inserted)) });
}

/// ITEMS in increasing precedence of their KEY, as sort_overlays leaves
/// them. The comparator is not transitive, so its input is GNU's:
/// overlays_in's tree order.
pub(crate) fn sort_overlays<T: Copy>(mut items: Vec<T>, key: impl Fn(&T) -> SortItem) -> Vec<T> {
    tree_order(&mut items, &key);
    merge_sort(items, &|a: &T, b: &T| compare_overlays(&key(a), &key(b)))
}

/// The item of highest precedence by the running comparison that
/// textprop.c get_char_property_and_overlay and editfns.c
/// get_pos_property make over the overlays with the property, in tree
/// order.
pub(crate) fn highest_overlay<T: Copy>(mut items: Vec<T>, key: impl Fn(&T) -> SortItem) -> Option<T> {
    tree_order(&mut items, &key);
    items.into_iter().reduce(|best, this| if compare_overlays(&key(&best), &key(&this)).is_lt() { this } else { best })
}

/// compare_overlays: Greater when A takes precedence over B. At equal
/// priority the overlay not covered by the other wins; where neither
/// covers the other the secondary priority decides. Not a total order.
/// GNU breaks a full tie by object address; Romacs by heap index.
pub(crate) fn compare_overlays(a: &SortItem, b: &SortItem) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let pick = |a_wins: bool| if a_wins { Greater } else { Less };
    if a.priority != b.priority { a.priority.cmp(&b.priority) }
    else if a.start < b.start { pick(a.end < b.end && a.secondary > b.secondary) }
    else if a.start > b.start { pick(!(a.end > b.end && a.secondary < b.secondary)) }
    else if a.end != b.end { pick(b.end > a.end) }
    else if a.secondary != b.secondary { a.secondary.cmp(&b.secondary) }
    else if a.overlay == b.overlay { Equal }
    else { a.overlay.index(0).unwrap_or(0).cmp(&b.overlay.index(0).unwrap_or(0)) }
}

/// An overlay string recorded by load_overlay_strings.
#[derive(Clone, Copy)]
pub(crate) struct OverlayEntry {
    pub overlay: Value,
    pub string: Value,
    pub priority: i64,
    pub after: bool,
    pub start: i64,
}

/// The strings of ENTRIES, given in itree order (decreasing start), in the
/// order they are displayed.
pub(crate) fn display_order(entries: Vec<OverlayEntry>) -> Vec<Value> {
    merge_sort(entries, &compare_overlay_entries).into_iter().map(|e| e.string).collect()
}

/// compare_overlay_entries: after-strings before before-strings, except
/// within one overlay; after-strings by decreasing and before-strings by
/// increasing priority. Not a total order across overlays.
fn compare_overlay_entries(a: &OverlayEntry, b: &OverlayEntry) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Equal, Greater, Less};
    if a.after != b.after {
        let first = if a.overlay == b.overlay { !a.after } else { a.after };
        if first { Less } else { Greater }
    } else if a.priority == b.priority { Equal }
    else if a.after { b.priority.cmp(&a.priority) }
    else { a.priority.cmp(&b.priority) }
}

/// glibc's qsort, a top-down merge sort, so that a comparator which is
/// not a total order yields GNU's sequence.
fn merge_sort<T: Copy>(mut items: Vec<T>, compare: &impl Fn(&T, &T) -> std::cmp::Ordering) -> Vec<T> {
    if items.len() <= 1 { return items; }
    let right = merge_sort(items.split_off(items.len() / 2), compare);
    let left = merge_sort(items, compare);
    let (mut out, mut i, mut j) = (Vec::with_capacity(left.len() + right.len()), 0, 0);
    while i < left.len() && j < right.len() {
        if compare(&left[i], &right[j]).is_le() { out.push(left[i]); i += 1; } else { out.push(right[j]); j += 1; }
    }
    out.extend_from_slice(&left[i..]);
    out.extend_from_slice(&right[j..]);
    out
}
