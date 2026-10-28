//! The order in which xdisp.c's load_overlay_strings delivers the overlay
//! strings shown at one buffer position, shared by measurement
//! (prim/leaf/display_elements.rs) and the mini-window's display
//! (mini_window.rs).
use crate::Value;

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
