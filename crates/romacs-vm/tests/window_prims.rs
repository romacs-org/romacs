//! Window bookkeeping slots that hold Lisp objects (GNU `w->new_normal`,
//! `w->normal_lines`, `w->normal_cols`, window.h) must keep their objects
//! alive and current across a moving collection. The values are reachable
//! ONLY through a window, so an untraced or unforwarded slot shows up here as
//! wrong contents after one collection, or as objects that are never freed.
use romacs_vm::Vm;

const JUNK: &str = "(let ((i 0)) (while (< i 2000) (copy-sequence \"junk\") (setq i (1+ i))))";

/// A side-by-side split of the selected window, A left of B under parent P.
const HSPLIT: &str = "(let* ((a (selected-window)) (h (window-pixel-width a)) (half (/ h 2)) \
     (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half t 0.5)))) \
     (setq wp-a a wp-b b wp-p (window-parent b) wp-h h wp-half half) nil)";

/// A vertical split of the selected window, A above B under parent P.
const SPLIT: &str = "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) \
     (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))) \
     (setq wp-a a wp-b b wp-p (window-parent b) wp-h h wp-half half) nil)";

fn show(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(&format!("(condition-case e {source} (error (list 'sig e)))"))
        .and_then(|v| v.display())
        .unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

/// Run SETUP (which stores a payload only in a window slot), collect once,
/// read back with READ, then run DROP and require OBJECTS objects / at least
/// BYTES bytes to be released.
fn retention(setup: &str, read: &str, expected: &str, drop: &str, objects: usize, bytes: usize) {
    retention_after(SPLIT, setup, read, expected, drop, objects, bytes);
}

/// `retention` after the split SPLIT instead of the vertical one.
fn retention_after(split: &str, setup: &str, read: &str, expected: &str, drop: &str, objects: usize, bytes: usize) {
    let mut vm = Vm::new();
    vm.eval_source(&format!("(progn {JUNK} {split} {setup} {JUNK} nil)")).unwrap();
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before, "{setup}: no collection ran");
    assert_eq!(show(&mut vm, read), expected, "{setup}: contents after a moving collection");
    vm.collect_garbage();
    let held = vm.heap_stats();
    vm.eval_source(&format!("(progn {drop} nil)")).unwrap();
    vm.collect_garbage();
    vm.collect_garbage();
    let released = vm.heap_stats();
    let freed = (held.live_objects - released.live_objects, held.live_bytes - released.live_bytes);
    eprintln!("{setup}: held {} objects / {} bytes; released {freed:?}", held.live_objects, held.live_bytes);
    assert_eq!(freed.0, objects, "{setup}: objects live only through the window");
    assert!(freed.1 >= bytes, "{setup}: payload bytes live only through the window: {freed:?}");
}

/// `set-window-new-normal` stores any object (window.c:4693); only the
/// window holds this 4000-byte string.
#[test]
fn new_normal_slot_retains_its_object_across_collection() {
    retention(
        "(set-window-new-normal wp-b (make-string 4000 113))",
        "(list (length (window-new-normal wp-b)) (aref (window-new-normal wp-b) 3999) (eq (window-new-normal wp-b) (window-new-normal wp-b)))",
        "(4000 113 t)",
        "(set-window-new-normal wp-b nil)",
        1,
        4000,
    );
}

/// `window-resize-apply` installs a numeric new normal size as the normal
/// size (window.c:4830); after the new-normal slot is cleared, the float is
/// reachable only through `normal_lines`. Dropping installs fixnums, so the
/// two floats become garbage.
#[test]
fn normal_lines_slot_retains_its_float_across_collection() {
    retention(
        "(progn (set-window-new-normal wp-a (/ 1.0 3)) (set-window-new-normal wp-b (/ 2.0 3)) \
         (set-window-new-pixel wp-p wp-h) (set-window-new-pixel wp-a wp-half) (set-window-new-pixel wp-b (- wp-h wp-half)) \
         (window-resize-apply) (set-window-new-normal wp-a nil) (set-window-new-normal wp-b nil))",
        "(list (window-normal-size wp-a) (window-normal-size wp-b) (window-normal-size wp-a t))",
        "(0.3333333333333333 0.6666666666666666 1.0)",
        "(progn (set-window-new-normal wp-a 0) (set-window-new-normal wp-b 1) \
         (set-window-new-pixel wp-p wp-h) (set-window-new-pixel wp-a wp-half) (set-window-new-pixel wp-b (- wp-h wp-half)) \
         (window-resize-apply) (set-window-new-normal wp-a nil) (set-window-new-normal wp-b nil))",
        2,
        1,
    );
}

/// The split parent inherits the split window's normal sizes (window.c:5544)
/// and holds them only in its own slots once the child's are replaced.
#[test]
fn split_parent_keeps_inherited_normal_sizes_across_collection() {
    let mut vm = Vm::new();
    vm.eval_source(&format!("(progn {JUNK} {SPLIT} {JUNK} nil)")).unwrap();
    vm.collect_garbage();
    assert_eq!(
        show(&mut vm, "(list (window-normal-size wp-p) (window-normal-size wp-p t) (window-new-normal wp-p) (window-normal-size wp-a) (window-normal-size wp-b))"),
        "(1.0 1.0 1.0 0 0.5)",
        "parent normal sizes after a moving collection"
    );
}

/// The horizontal twin of the `normal_lines` test: after a side-by-side
/// split `window-resize-apply` installs the floats as `normal_cols`
/// (window.c:4830), reachable only through that slot.
#[test]
fn normal_cols_slot_retains_its_float_across_collection() {
    retention_after(
        HSPLIT,
        "(progn (set-window-new-normal wp-a (/ 1.0 3)) (set-window-new-normal wp-b (/ 2.0 3)) \
         (set-window-new-pixel wp-p wp-h) (set-window-new-pixel wp-a wp-half) (set-window-new-pixel wp-b (- wp-h wp-half)) \
         (window-resize-apply nil t) (set-window-new-normal wp-a nil) (set-window-new-normal wp-b nil))",
        "(list (window-normal-size wp-a t) (window-normal-size wp-b t) (window-normal-size wp-a))",
        "(0.3333333333333333 0.6666666666666666 1.0)",
        "(progn (set-window-new-normal wp-a 0) (set-window-new-normal wp-b 1) \
         (set-window-new-pixel wp-p wp-h) (set-window-new-pixel wp-a wp-half) (set-window-new-pixel wp-b (- wp-h wp-half)) \
         (window-resize-apply nil t) (set-window-new-normal wp-a nil) (set-window-new-normal wp-b nil))",
        2,
        1,
    );
}

/// replace_window (window.c:2571-2574) gives the new parent the split
/// window's normal sizes and resets the split window's to 1.0, so a later
/// split in the other direction starts from 1.0. Expected values are the
/// pinned GNU's (45b0f7699df, `emacs -Q --batch`) for the same forms.
#[test]
fn split_hands_normal_sizes_to_the_parent_and_resets_the_window() {
    let mut vm = Vm::new();
    let sizes = "(list (window-normal-size a) (window-normal-size a t) (window-normal-size b) (window-normal-size b t) \
         (window-normal-size (window-parent a)) (window-normal-size (window-parent a) t))";
    let vertical = format!("(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) \
         (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))) {sizes})");
    assert_eq!(show(&mut vm, &vertical), "(0 1.0 0.5 1.0 1.0 1.0)", "normal sizes after a vertical split");
    let mut vm = Vm::new();
    let horizontal = format!("(let* ((a (selected-window)) (w (window-pixel-width a)) (half (/ w 2)) \
         (b (progn (set-window-new-pixel a (- w half)) (split-window-internal a half t 0.5)))) {sizes})");
    assert_eq!(show(&mut vm, &horizontal), "(1.0 0 1.0 0.5 1.0 1.0)", "normal sizes after a horizontal split");
    let mut vm = Vm::new();
    let nested = "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) \
         (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5))) \
         (w (window-pixel-width b)) (hw (/ w 2))) \
         (set-window-new-pixel b (- w hw)) (split-window-internal b hw t 0.5) \
         (list (window-normal-size b) (window-normal-size (window-parent b))))";
    assert_eq!(show(&mut vm, nested), "(1.0 0.5)", "split window's normal sizes reset by the second split");
}

/// replace_window (root, window, true) (window.c:3667): the window that
/// takes the root's place inherits the root's normal sizes. Pinned GNU
/// answers (1.0 1.0).
#[test]
fn delete_other_windows_internal_inherits_the_roots_normal_sizes() {
    let mut vm = Vm::new();
    let src = "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) \
         (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))) \
         (delete-other-windows-internal b) (list (window-normal-size b) (window-normal-size b t)))";
    assert_eq!(show(&mut vm, src), "(1.0 1.0)", "normal sizes inherited from the root");
}

/// VM2-WIN4 deliberately pins the missing visual-motion adjustment, not GNU
/// parity. A future implementation must update this test and the divergence.
#[test]
fn vm2_win4_expanding_a_scrolled_lower_window_keeps_its_start() {
    let mut vm = Vm::new();
    let src = format!("(progn {SPLIT} \
        (let ((i 0)) (while (< i 100) (insert \"line\\n\") (setq i (1+ i)))) \
        (set-window-start wp-b 201) \
        (let ((old (window-start wp-b))) \
          (delete-other-windows-internal wp-b) \
          (list old (window-start wp-b) (eq (frame-root-window) wp-b))))");
    assert_eq!(show(&mut vm, &src), "(201 201 t)", "VM2-WIN4: no vmotion scroll preservation; update DIVERGENCES.md when fixed");
}

/// VM2-WIN5 characterizes the hybrid device model, not GNU's terminal
/// invariant. Keep the device type and BOTH backend dimensions observable
/// together, including after the GUI supplies real canonical font metrics.
#[test]
fn vm2_win5_frame_type_and_character_metrics_are_a_documented_hybrid() {
    let mut vm = Vm::new();
    let src = "(list (frame-live-p (selected-frame)) (frame-char-height) (frame-char-width) \
        (= (* (frame-char-height) (window-total-height (minibuffer-window))) \
           (window-pixel-height (minibuffer-window))) \
        (= (* (frame-char-width) (window-total-width)) (window-pixel-width)))";
    assert_eq!(show(&mut vm, src), "(t 20 10 t t)", "VM2-WIN5: default hybrid model; update DIVERGENCES.md when fixed");
    vm.set_frame_geometry(800, 600, 24, 8).unwrap();
    assert_eq!(show(&mut vm, src), "(t 24 8 t t)", "VM2-WIN5: character metrics must follow backend geometry");
}
